//! Страница ответа scroll (или одна запись `get`): тело как есть плюс разобранные `data`,
//! `included`, `meta`.

use std::collections::HashMap;

use serde::Deserialize;
use serde_json::Value;

use crate::error::CliError;

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct PageMeta {
    pub has_more: bool,
    #[serde(default)]
    pub count: Option<u64>,
    #[serde(default)]
    pub cursor: Option<String>,
    #[serde(default)]
    pub total_count: Option<u64>,
    #[serde(default)]
    pub applied_filter: Option<String>,
}

#[derive(Debug, Clone)]
pub struct Page {
    pub body: Vec<u8>,
    pub data: Vec<Value>,
    pub included: Vec<Value>,
    pub meta: PageMeta,
}

#[derive(Deserialize)]
struct Wire {
    data: Vec<Value>,
    #[serde(default)]
    included: Vec<Value>,
    meta: PageMeta,
}

#[derive(Deserialize)]
struct SingleWire {
    data: Value,
    #[serde(default)]
    included: Vec<Value>,
}

impl Page {
    pub fn parse(body: Vec<u8>) -> Result<Page, CliError> {
        let wire: Wire = serde_json::from_slice(&body)
            .map_err(|e| bad_response(format!("ответ сервера не похож на страницу обхода: {e}")))?;
        Ok(Page {
            body,
            data: wire.data,
            included: wire.included,
            meta: wire.meta,
        })
    }

    /// Ответ `GET /{type}/{id}` — страница из одной записи: её выводят те же форматы, что scroll.
    pub fn single(body: Vec<u8>) -> Result<Page, CliError> {
        let wire: SingleWire = serde_json::from_slice(&body)
            .map_err(|e| bad_response(format!("ответ сервера не похож на запись: {e}")))?;
        if !wire.data.is_object() {
            return Err(bad_response(
                "ответ сервера не похож на запись: data — не объект".into(),
            ));
        }
        Ok(Page {
            body,
            data: vec![wire.data],
            included: wire.included,
            meta: PageMeta {
                has_more: false,
                count: Some(1),
                cursor: None,
                total_count: None,
                applied_filter: None,
            },
        })
    }

    pub fn ids(&self) -> Vec<String> {
        self.data
            .iter()
            .filter_map(record_id)
            .map(str::to_string)
            .collect()
    }
}

fn bad_response(message: String) -> CliError {
    CliError::general("bad_response", message)
        .retryable(true)
        .with_hint(
            "между CLI и API может стоять прокси, или идут технические работы; повторите позже",
        )
}

pub fn record_id(record: &Value) -> Option<&str> {
    record.get("id").and_then(Value::as_str)
}

/// `included` по (type, id) — чтобы подставлять связанные объекты за O(1).
pub struct IncludedIndex<'a> {
    by_type: HashMap<&'a str, HashMap<&'a str, &'a Value>>,
}

impl<'a> IncludedIndex<'a> {
    pub fn new(included: &'a [Value]) -> Self {
        let mut by_type: HashMap<&'a str, HashMap<&'a str, &'a Value>> = HashMap::new();
        for object in included {
            if let (Some(kind), Some(id)) = (
                object.get("type").and_then(Value::as_str),
                record_id(object),
            ) {
                by_type.entry(kind).or_default().insert(id, object);
            }
        }
        IncludedIndex { by_type }
    }

    pub fn get(&self, kind: &str, id: &str) -> Option<&'a Value> {
        self.by_type.get(kind)?.get(id).copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(crate) const FIXTURE: &str = include_str!("../tests/fixtures/reviews_page_include.json");

    #[test]
    fn parses_observed_page_shape() {
        let page = Page::parse(FIXTURE.as_bytes().to_vec()).unwrap();
        assert_eq!(
            page.ids(),
            vec!["0000000000000000aa000001", "0000000000000000aa000004"]
        );
        assert_eq!(page.meta.cursor.as_deref(), Some("fixture-cursor-1"));
        assert_eq!(
            (page.meta.has_more, page.meta.total_count),
            (true, Some(744))
        );
        let index = IncludedIndex::new(&page.included);
        assert!(index.get("consumers", "0000000000000000aa000002").is_some());
        assert!(index.get("products", "0000000000000000aa000002").is_none());
    }

    #[test]
    fn single_record_is_a_one_record_page() {
        let body = br#"{"data":{"id":"r1","type":"reviews","relationships":{"author":{"data":{"id":"c1","type":"consumers"}}}},"included":[{"id":"c1","type":"consumers","attributes":{"name":"Anna"}}]}"#;
        let page = Page::single(body.to_vec()).unwrap();
        assert_eq!(page.ids(), vec!["r1"]);
        assert_eq!(page.included.len(), 1);
        assert!(!page.meta.has_more);
        for bad in [&br#"{"data":[]}"#[..], b"<html>", br#"{"data":null}"#] {
            assert_eq!(Page::single(bad.to_vec()).unwrap_err().code, "bad_response");
        }
    }

    #[test]
    fn non_json_page_is_bad_response() {
        for body in [&b"<html>maintenance</html>"[..], br#"{"data": []}"#] {
            let err = Page::parse(body.to_vec()).unwrap_err();
            assert_eq!(err.code, "bad_response");
        }
    }
}
