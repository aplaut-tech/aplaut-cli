//! `jsonl`: по строке на запись; ссылки в `relationships` заменены объектами из `included`.
//! Ключи выходят в алфавитном порядке (serde_json без `preserve_order`); точные байты — `raw`.

use std::collections::HashSet;
use std::io::Write;

use serde_json::Value;

use super::{is_duplicate, write_error, Commit, PageReport, RecordSink};
use crate::error::CliError;
use crate::page::{IncludedIndex, Page};

pub struct JsonlSink {
    out: Box<dyn Write>,
}

impl JsonlSink {
    pub fn new(out: Box<dyn Write>) -> Self {
        JsonlSink { out }
    }
}

impl RecordSink for JsonlSink {
    fn write_page(&mut self, page: &Page, seen: &HashSet<String>) -> Result<PageReport, CliError> {
        let index = IncludedIndex::new(&page.included);
        let (mut written, mut duplicates) = (0, 0);
        for record in &page.data {
            if is_duplicate(record, seen) {
                duplicates += 1;
                continue;
            }
            serde_json::to_writer(&mut self.out, &resolve_relationships(record, &index))
                .map_err(|e| write_error(e.into()))?;
            self.out.write_all(b"\n").map_err(write_error)?;
            written += 1;
        }
        self.out.flush().map_err(write_error)?;
        Ok(PageReport { written, duplicates, commit: Commit::Durable })
    }

    fn finish(&mut self) -> Result<Commit, CliError> {
        self.out.flush().map_err(write_error)?;
        Ok(Commit::Durable)
    }
}

/// Ссылка `{type, id}` заменяется объектом из `included`; без объекта остаётся ссылкой.
pub fn resolve_relationships(record: &Value, index: &IncludedIndex) -> Value {
    let mut out = record.clone();
    if let Some(relationships) = out.get_mut("relationships").and_then(Value::as_object_mut) {
        for relationship in relationships.values_mut() {
            match relationship.get_mut("data") {
                Some(Value::Array(items)) => items.iter_mut().for_each(|item| resolve_ref(item, index)),
                Some(item @ Value::Object(_)) => resolve_ref(item, index),
                _ => {}
            }
        }
    }
    out
}

fn resolve_ref(reference: &mut Value, index: &IncludedIndex) {
    let found = match (
        reference.get("type").and_then(Value::as_str),
        reference.get("id").and_then(Value::as_str),
    ) {
        (Some(kind), Some(id)) => index.get(kind, id).cloned(),
        _ => None,
    };
    if let Some(object) = found {
        *reference = object;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::output::test_util::{fixture_page, none_seen};
    use crate::term::SharedBuf;

    fn lines(buf: &SharedBuf) -> Vec<Value> {
        buf.contents().lines().map(|l| serde_json::from_str(l).unwrap()).collect()
    }

    #[test]
    fn inlines_included_objects_and_keeps_unresolved_refs() {
        let buf = SharedBuf::default();
        let mut sink = JsonlSink::new(Box::new(buf.clone()));
        sink.write_page(&fixture_page(), &none_seen()).unwrap();
        let records = lines(&buf);
        assert_eq!(records.len(), 2);
        let author = &records[0]["relationships"]["author"]["data"];
        assert_eq!(author["type"], "consumers");
        assert_eq!(author["attributes"]["email"], "author1@example.com");
        assert_eq!(records[0]["relationships"]["product"]["data"]["attributes"]["name"], "Товар 1");
        assert!(records[1]["relationships"]["author"]["data"].is_null());
        let comment = &records[1]["relationships"]["comments"]["data"][0];
        assert!(comment.get("attributes").is_none() && comment["type"] == "comments");
        assert_eq!(records[0]["attributes"]["rating"], 5.0);
    }

    #[test]
    fn drops_repeated_tail_of_previous_page() {
        let buf = SharedBuf::default();
        let mut sink = JsonlSink::new(Box::new(buf.clone()));
        let seen = HashSet::from(["0000000000000000aa000001".to_string()]);
        let report = sink.write_page(&fixture_page(), &seen).unwrap();
        assert_eq!((report.written, report.duplicates), (1, 1));
        assert_eq!(lines(&buf)[0]["id"], "0000000000000000aa000004");
    }
}
