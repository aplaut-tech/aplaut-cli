//! Схема табличного вывода: колонки из ключей строк, типы — по непустым значениям.
//! CSV использует только имена; типы нужны будущим Avro/Parquet (дизайн §6.1).

use std::collections::BTreeSet;

use serde_json::Value;

use super::projection::{ColumnKey, Row};
use crate::time;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogicalType {
    String,
    Int64,
    Float64,
    Bool,
    Timestamp,
    /// Объекты, массивы, смешанные и неизвестные значения — строкой JSON.
    Json,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Column {
    pub key: ColumnKey,
    pub name: String,
    pub ty: LogicalType,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Schema {
    pub columns: Vec<Column>,
}

impl Schema {
    pub fn infer(rows: &[Row]) -> Schema {
        let mut keys: BTreeSet<ColumnKey> = [ColumnKey::Id, ColumnKey::Type].into_iter().collect();
        for row in rows {
            keys.extend(row.keys().cloned());
        }
        let columns = keys
            .into_iter()
            .map(|key| {
                let ty = infer_type(rows.iter().filter_map(|r| r.get(&key)));
                Column { name: key.name(), key, ty }
            })
            .collect();
        Schema { columns }
    }

    pub fn contains(&self, key: &ColumnKey) -> bool {
        self.columns.iter().any(|c| &c.key == key)
    }
}

fn infer_type<'a>(values: impl Iterator<Item = &'a Value>) -> LogicalType {
    let mut found: Option<LogicalType> = None;
    for value in values {
        let ty = match value {
            Value::Null => continue,
            Value::Bool(_) => LogicalType::Bool,
            Value::Number(n) if n.is_i64() || n.is_u64() => LogicalType::Int64,
            Value::Number(_) => LogicalType::Float64,
            Value::String(s) if time::parse_rfc3339(s).is_some() => LogicalType::Timestamp,
            Value::String(_) => LogicalType::String,
            Value::Array(_) | Value::Object(_) => LogicalType::Json,
        };
        found = Some(match (found, ty) {
            (None, t) => t,
            (Some(a), b) if a == b => a,
            (Some(LogicalType::Int64), LogicalType::Float64) | (Some(LogicalType::Float64), LogicalType::Int64) => {
                LogicalType::Float64
            }
            (Some(LogicalType::Timestamp), LogicalType::String) | (Some(LogicalType::String), LogicalType::Timestamp) => {
                LogicalType::String
            }
            _ => LogicalType::Json,
        });
    }
    found.unwrap_or(LogicalType::Json)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::output::tabular::project;
    use crate::output::test_util::fixture_page;
    use crate::page::IncludedIndex;
    use serde_json::json;

    fn ty(schema: &Schema, name: &str) -> LogicalType {
        schema.columns.iter().find(|c| c.name == name).unwrap_or_else(|| panic!("{name}")).ty
    }

    #[test]
    fn infers_types_from_observed_data() {
        let page = fixture_page();
        let index = IncludedIndex::new(&page.included);
        let rows: Vec<Row> = page.data.iter().map(|r| project(r, &index, &[])).collect();
        let schema = Schema::infer(&rows);
        assert_eq!(schema.columns[0].name, "id");
        assert_eq!(ty(&schema, "rating"), LogicalType::Float64);
        assert_eq!(ty(&schema, "verified"), LogicalType::Bool);
        assert_eq!(ty(&schema, "likes"), LogicalType::Int64);
        assert_eq!(ty(&schema, "created_at"), LogicalType::Timestamp);
        assert_eq!(ty(&schema, "custom_attributes"), LogicalType::Json);
        assert_eq!(ty(&schema, "author_ip"), LogicalType::Json, "только null → json");
    }

    #[test]
    fn mixed_numbers_widen_and_conflicts_fall_back_to_json() {
        let row = |v: Value| Row::from([(ColumnKey::Attribute("x".into()), v)]);
        assert_eq!(ty(&Schema::infer(&[row(json!(1)), row(json!(1.5))]), "x"), LogicalType::Float64);
        assert_eq!(ty(&Schema::infer(&[row(json!("a")), row(json!(true))]), "x"), LogicalType::Json);
        assert_eq!(Schema::infer(&[]).columns.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(), ["id", "type"]);
    }
}
