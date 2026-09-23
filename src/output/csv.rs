//! `csv`: кавычки и экранирование по RFC 4180, UTF-8 без BOM, строки через `\n`.
//! Сценарий — `aplaut … --format csv | tw` и загрузка в таблицы.
//! Формулы (`=…`) не экранируются: это исказило бы данные для DWH; для Excel — импорт как текст.

use std::collections::HashSet;
use std::io::Write;
use std::rc::Rc;

use serde_json::Value;

use super::tabular::{project, Schema};
use super::{is_duplicate, write_error, Commit, PageReport, RecordSink};
use crate::error::CliError;
use crate::page::{IncludedIndex, Page};
use crate::term::Reporter;

pub struct CsvSink {
    out: Box<dyn Write>,
    include: Vec<String>,
    reporter: Rc<Reporter>,
    /// Фиксируется по первой странице: заголовок CSV нельзя поменять на ходу.
    schema: Option<Schema>,
    warned_new_columns: bool,
    warned_to_many: bool,
}

impl CsvSink {
    pub fn new(out: Box<dyn Write>, include: Vec<String>, reporter: Rc<Reporter>) -> Self {
        CsvSink {
            out,
            include,
            reporter,
            schema: None,
            warned_new_columns: false,
            warned_to_many: false,
        }
    }

    fn warn_to_many(&mut self, record: &Value) {
        if self.warned_to_many {
            return;
        }
        let lists: Vec<&str> = self
            .include
            .iter()
            .filter(|rel| {
                record
                    .pointer(&format!("/relationships/{rel}/data"))
                    .is_some_and(Value::is_array)
            })
            .map(String::as_str)
            .collect();
        if !lists.is_empty() {
            self.warned_to_many = true;
            self.reporter.warn(&format!(
                "связи «{}» — списки; в CSV они не разворачиваются (объекты есть в --format jsonl)",
                lists.join(", ")
            ));
        }
    }
}

impl RecordSink for CsvSink {
    fn write_page(&mut self, page: &Page, seen: &HashSet<String>) -> Result<PageReport, CliError> {
        let index = IncludedIndex::new(&page.included);
        let mut rows = Vec::new();
        let mut duplicates = 0;
        for record in &page.data {
            if is_duplicate(record, seen) {
                duplicates += 1;
                continue;
            }
            self.warn_to_many(record);
            rows.push(project(record, &index, &self.include));
        }
        if self.schema.is_none() && !rows.is_empty() {
            let schema = Schema::infer(&rows);
            write_record(
                self.out.as_mut(),
                schema.columns.iter().map(|c| c.name.clone()),
            )?;
            self.schema = Some(schema);
        }
        if let Some(schema) = &self.schema {
            for row in &rows {
                if !self.warned_new_columns && row.keys().any(|k| !schema.contains(k)) {
                    self.warned_new_columns = true;
                    self.reporter.warn(
                        "в данных появились поля, которых не было на первой странице; в CSV они не попадут (есть в --format jsonl)",
                    );
                }
                let cells = schema
                    .columns
                    .iter()
                    .map(|c| row.get(&c.key).map(cell_text).unwrap_or_default());
                write_record(self.out.as_mut(), cells)?;
            }
        }
        self.out.flush().map_err(write_error)?;
        Ok(PageReport {
            written: rows.len() as u64,
            duplicates,
            commit: Commit::Durable,
        })
    }

    fn finish(&mut self) -> Result<Commit, CliError> {
        if self.schema.is_none() {
            let schema = Schema::infer(&[]);
            write_record(
                self.out.as_mut(),
                schema.columns.iter().map(|c| c.name.clone()),
            )?;
            self.schema = Some(schema);
        }
        self.out.flush().map_err(write_error)?;
        Ok(Commit::Durable)
    }
}

fn write_record<I, S>(out: &mut dyn Write, fields: I) -> Result<(), CliError>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let mut line = String::new();
    for (i, field) in fields.into_iter().enumerate() {
        if i > 0 {
            line.push(',');
        }
        push_field(&mut line, field.as_ref());
    }
    line.push('\n');
    out.write_all(line.as_bytes()).map_err(write_error)
}

fn push_field(line: &mut String, text: &str) {
    if text.contains([',', '"', '\n', '\r']) {
        line.push('"');
        line.push_str(&text.replace('"', "\"\""));
        line.push('"');
    } else {
        line.push_str(text);
    }
}

/// null → пустая ячейка; объекты и массивы — компактным JSON.
fn cell_text(value: &Value) -> String {
    match value {
        Value::Null => String::new(),
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::output::test_util::{fixture_page, none_seen};
    use crate::term::SharedBuf;

    fn sink(include: &[&str]) -> (CsvSink, SharedBuf, SharedBuf) {
        let (out, log) = (SharedBuf::default(), SharedBuf::default());
        let reporter = Rc::new(Reporter::with_writer(
            false,
            false,
            false,
            false,
            Box::new(log.clone()),
        ));
        let include = include.iter().map(|s| s.to_string()).collect();
        (
            CsvSink::new(Box::new(out.clone()), include, reporter),
            out,
            log,
        )
    }

    /// Минимальный разбор RFC 4180 для проверки: кавычки, "" и переводы строк внутри поля.
    fn parse_csv(text: &str) -> Vec<Vec<String>> {
        let (mut rows, mut row, mut field, mut quoted) =
            (Vec::new(), Vec::new(), String::new(), false);
        let mut chars = text.chars().peekable();
        while let Some(c) = chars.next() {
            match (c, quoted) {
                ('"', true) if chars.peek() == Some(&'"') => {
                    field.push('"');
                    chars.next();
                }
                ('"', _) => quoted = !quoted,
                (',', false) => row.push(std::mem::take(&mut field)),
                ('\n', false) => {
                    row.push(std::mem::take(&mut field));
                    rows.push(std::mem::take(&mut row));
                }
                (c, _) => field.push(c),
            }
        }
        rows
    }

    #[test]
    fn header_from_first_page_and_rfc4180_escaping() {
        let (mut s, out, log) = sink(&["author", "comments"]);
        let report = s.write_page(&fixture_page(), &none_seen()).unwrap();
        s.finish().unwrap();
        let rows = parse_csv(&out.contents());
        assert_eq!(rows.len(), 3, "заголовок + 2 записи");
        let header = &rows[0];
        assert_eq!(&header[..2], ["id", "type"]);
        let col = |name: &str| {
            header
                .iter()
                .position(|h| h == name)
                .unwrap_or_else(|| panic!("{name}"))
        };
        assert_eq!(rows[1][col("author.email")], "author1@example.com");
        assert_eq!(
            rows[2][col("author.email")],
            "",
            "у второго отзыва автора нет"
        );
        assert_eq!(
            rows[1][col("body")],
            "Текст body 1\nвторая строка, с \"кавычками\""
        );
        assert!(rows.iter().all(|r| r.len() == header.len()));
        assert_eq!(report.written, 2);
        assert!(
            log.contents().contains("comments"),
            "предупреждение про список: {}",
            log.contents()
        );
    }

    #[test]
    fn empty_export_is_just_header() {
        let (mut s, out, _) = sink(&[]);
        s.finish().unwrap();
        assert_eq!(out.contents(), "id,type\n");
    }

    #[test]
    fn new_keys_after_first_page_warn_once() {
        let (mut s, out, log) = sink(&[]);
        let first = br#"{"data":[{"id":"a","type":"reviews","attributes":{"x":1}}],"meta":{"has_more":true,"cursor":"c"}}"#;
        let second = br#"{"data":[{"id":"b","type":"reviews","attributes":{"x":2,"y":3}},{"id":"c","type":"reviews","attributes":{"z":1}}],"meta":{"has_more":false}}"#;
        s.write_page(&Page::parse(first.to_vec()).unwrap(), &none_seen())
            .unwrap();
        s.write_page(&Page::parse(second.to_vec()).unwrap(), &none_seen())
            .unwrap();
        assert_eq!(
            out.contents(),
            "id,type,x\na,reviews,1\nb,reviews,2\nc,reviews,\n"
        );
        assert_eq!(log.contents().matches("появились поля").count(), 1);
    }
}
