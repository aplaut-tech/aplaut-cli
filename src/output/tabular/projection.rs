//! Запись JSON:API → плоская строка: `id`, `type`, атрибуты, `<rel>_ref`, `<rel>.<attr>`.

use std::collections::BTreeMap;

use serde_json::Value;

use crate::page::IncludedIndex;

/// Порядок вариантов задаёт порядок колонок: id, type, атрибуты, ссылки, включения;
/// внутри группы — по алфавиту. Так заголовок стабилен между запусками и версиями сервера.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ColumnKey {
    Id,
    Type,
    Attribute(String),
    Ref(String),
    Included(String, String),
}

impl ColumnKey {
    pub fn name(&self) -> String {
        match self {
            ColumnKey::Id => "id".into(),
            ColumnKey::Type => "type".into(),
            ColumnKey::Attribute(a) => a.clone(),
            ColumnKey::Ref(rel) => format!("{rel}_ref"),
            ColumnKey::Included(rel, attr) => format!("{rel}.{attr}"),
        }
    }
}

pub type Row = BTreeMap<ColumnKey, Value>;

/// `include` — связи, запрошенные через `--include`: только их to-one объекты разворачиваются в колонки.
pub fn project(record: &Value, index: &IncludedIndex, include: &[String]) -> Row {
    let mut row = Row::new();
    row.insert(ColumnKey::Id, record.get("id").cloned().unwrap_or(Value::Null));
    row.insert(ColumnKey::Type, record.get("type").cloned().unwrap_or(Value::Null));
    if let Some(attributes) = record.get("attributes").and_then(Value::as_object) {
        for (name, value) in attributes {
            row.insert(ColumnKey::Attribute(name.clone()), value.clone());
        }
    }
    let Some(relationships) = record.get("relationships").and_then(Value::as_object) else {
        return row;
    };
    for (name, relationship) in relationships {
        let data = relationship.get("data").unwrap_or(&Value::Null);
        let reference = match data {
            Value::Object(_) => data.get("id").cloned().unwrap_or(Value::Null),
            Value::Array(items) => Value::Array(items.iter().filter_map(|i| i.get("id").cloned()).collect()),
            _ => Value::Null,
        };
        row.insert(ColumnKey::Ref(name.clone()), reference);
        if !include.iter().any(|i| i == name) {
            continue;
        }
        let included = match (data.get("type").and_then(Value::as_str), data.get("id").and_then(Value::as_str)) {
            (Some(kind), Some(id)) => index.get(kind, id),
            _ => None,
        };
        if let Some(attributes) = included.and_then(|o| o.get("attributes")).and_then(Value::as_object) {
            for (attr, value) in attributes {
                row.insert(ColumnKey::Included(name.clone(), attr.clone()), value.clone());
            }
        }
    }
    row
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::output::test_util::fixture_page;

    #[test]
    fn flattens_attributes_refs_and_requested_includes() {
        let page = fixture_page();
        let index = IncludedIndex::new(&page.included);
        let row = project(&page.data[0], &index, &["author".to_string()]);
        let names: Vec<String> = row.keys().map(ColumnKey::name).collect();
        assert_eq!(&names[..2], ["id", "type"]);
        assert!(names.contains(&"product_name".to_string()));
        assert_eq!(row[&ColumnKey::Ref("author".into())], "0000000000000000aa000002");
        assert_eq!(row[&ColumnKey::Included("author".into(), "email".into())], "author1@example.com");
        assert!(!row.contains_key(&ColumnKey::Included("product".into(), "name".into())), "product не запрошен");
        let second = project(&page.data[1], &index, &["author".to_string()]);
        assert!(second[&ColumnKey::Ref("author".into())].is_null());
        assert!(second[&ColumnKey::Ref("comments".into())].is_array());
        let first_ref = names.iter().position(|n| n.ends_with("_ref")).unwrap();
        let first_inc = names.iter().position(|n| n.contains('.')).unwrap();
        assert!(names.iter().position(|n| n == "verified").unwrap() < first_ref && first_ref < first_inc);
    }
}
