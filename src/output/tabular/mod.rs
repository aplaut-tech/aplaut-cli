//! Общая часть табличных форматов: CSV сейчас, Avro/Parquet позже (дизайн §6.1).

mod projection;
mod schema;

pub use projection::{project, ColumnKey, Row};
pub use schema::{Column, LogicalType, Schema};
