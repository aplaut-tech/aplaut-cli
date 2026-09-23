//! Форматы вывода (дизайн §6). Формат сам сообщает, когда записанное зафиксировано (§6.1):
//! цикл обхода сохраняет стейт только в эти моменты — так at-least-once держится и для
//! потоковых форматов сейчас, и для контейнерных (Parquet) потом.

pub mod csv;
pub mod jsonl;
pub mod raw;
pub mod tabular;

use std::collections::HashSet;
use std::io::{self, Write};
use std::rc::Rc;

use serde_json::Value;

use crate::error::{CliError, Exit};
use crate::page::{record_id, Page};
use crate::term::Reporter;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Raw,
    Jsonl,
    Csv,
}

impl Format {
    pub const NAMES: [&'static str; 3] = ["raw", "jsonl", "csv"];

    /// Табличные форматы строятся на общей проекции (`tabular`) и понимают `--fields`.
    pub fn is_tabular(self) -> bool {
        matches!(self, Format::Csv)
    }

    pub fn from_name(name: &str) -> Option<Format> {
        match name {
            "raw" => Some(Format::Raw),
            "jsonl" => Some(Format::Jsonl),
            "csv" => Some(Format::Csv),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Commit {
    /// Всё записанное уже у приёмника — можно сдвигать стейт.
    Durable,
    /// Данные буферизованы (например, Parquet до футера) — стейт трогать нельзя.
    Pending,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PageReport {
    pub written: u64,
    pub duplicates: u64,
    pub commit: Commit,
}

pub trait RecordSink {
    /// `seen` — id записей предыдущей страницы: после паузы > 2 мин сервер повторяет её хвост.
    fn write_page(&mut self, page: &Page, seen: &HashSet<String>) -> Result<PageReport, CliError>;
    fn finish(&mut self) -> Result<Commit, CliError>;
    /// Обход продолжается по сохранённому стейту: первая страница запуска — продолжение, её уже
    /// не перезапросить (курсор не идемпотентен), поэтому отвергать её формату поздно.
    fn resumed(&mut self) {}
}

pub fn make_sink(
    format: Format,
    out: Box<dyn Write>,
    include: &[String],
    fields: Option<Vec<String>>,
    reporter: Rc<Reporter>,
) -> Box<dyn RecordSink> {
    match format {
        Format::Raw => Box::new(raw::RawSink::new(out)),
        Format::Jsonl => Box::new(jsonl::JsonlSink::new(out)),
        Format::Csv => Box::new(csv::CsvSink::new(out, include.to_vec(), fields, reporter)),
    }
}

/// Закрытый пайп (`| head`) — получатель перестал читать; выгрузка не завершена, поэтому
/// код 4, а стейт не сдвигается дальше отданного.
pub fn write_error(err: io::Error) -> CliError {
    if err.kind() == io::ErrorKind::BrokenPipe {
        CliError::new(
            Exit::Partial,
            "output_closed",
            "получатель закрыл поток вывода",
        )
    } else {
        CliError::io("запись в stdout", &err)
    }
}

pub(crate) fn is_duplicate(record: &Value, seen: &HashSet<String>) -> bool {
    record_id(record).is_some_and(|id| seen.contains(id))
}

#[cfg(test)]
pub(crate) mod test_util {
    use std::collections::HashSet;

    use crate::page::Page;

    pub fn fixture_page() -> Page {
        Page::parse(include_bytes!("../../tests/fixtures/reviews_page_include.json").to_vec())
            .unwrap()
    }

    pub fn none_seen() -> HashSet<String> {
        HashSet::new()
    }
}
