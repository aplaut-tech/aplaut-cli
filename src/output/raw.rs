//! `raw`: тело каждого ответа как есть, одна строка JSON на страницу.

use std::collections::HashSet;
use std::io::Write;

use super::{is_duplicate, write_error, Commit, PageReport, RecordSink};
use crate::error::CliError;
use crate::page::Page;

pub struct RawSink {
    out: Box<dyn Write>,
}

impl RawSink {
    pub fn new(out: Box<dyn Write>) -> Self {
        RawSink { out }
    }
}

impl RecordSink for RawSink {
    fn write_page(&mut self, page: &Page, seen: &HashSet<String>) -> Result<PageReport, CliError> {
        // Перевод строки внутри JSON-строк всегда экранирован, поэтому сырой \n — только
        // форматирование ответа: тогда сжимаем, чтобы сохранить «одна страница — одна строка».
        let body = page.body.trim_ascii();
        if body.contains(&b'\n') {
            let value: serde_json::Value = serde_json::from_slice(body)
                .map_err(|e| CliError::general("bad_response", e.to_string()))?;
            serde_json::to_writer(&mut self.out, &value).map_err(|e| write_error(e.into()))?;
        } else {
            self.out.write_all(body).map_err(write_error)?;
        }
        self.out.write_all(b"\n").map_err(write_error)?;
        self.out.flush().map_err(write_error)?;
        // Дубли в raw не отбрасываются — тело не меняем; только считаем для сообщения.
        let duplicates = page.data.iter().filter(|r| is_duplicate(r, seen)).count() as u64;
        Ok(PageReport { written: page.data.len() as u64, duplicates, commit: Commit::Durable })
    }

    fn finish(&mut self) -> Result<Commit, CliError> {
        self.out.flush().map_err(write_error)?;
        Ok(Commit::Durable)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::output::test_util::{fixture_page, none_seen};
    use crate::term::SharedBuf;

    #[test]
    fn pretty_body_becomes_one_line_with_same_content() {
        let buf = SharedBuf::default();
        let mut sink = RawSink::new(Box::new(buf.clone()));
        let page = fixture_page();
        let report = sink.write_page(&page, &none_seen()).unwrap();
        let out = buf.contents();
        assert_eq!(out.matches('\n').count(), 1);
        let reparsed: serde_json::Value = serde_json::from_str(out.trim_end()).unwrap();
        assert_eq!(reparsed, serde_json::from_slice::<serde_json::Value>(&page.body).unwrap());
        assert_eq!(report, PageReport { written: 2, duplicates: 0, commit: Commit::Durable });
    }

    #[test]
    fn compact_body_is_written_byte_for_byte_and_duplicates_are_kept() {
        let body = br#"{"data":[{"id":"a","type":"reviews"},{"id":"b","type":"reviews"}],"meta":{"has_more":false}}"#;
        let page = Page::parse(body.to_vec()).unwrap();
        let buf = SharedBuf::default();
        let mut sink = RawSink::new(Box::new(buf.clone()));
        let seen = HashSet::from(["a".to_string()]);
        let report = sink.write_page(&page, &seen).unwrap();
        assert_eq!(buf.contents().as_bytes(), [&body[..], b"\n"].concat());
        assert_eq!((report.written, report.duplicates), (2, 1));
    }
}
