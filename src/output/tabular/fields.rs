//! `--fields`: какие колонки табличного вывода писать и в каком порядке. API не умеет sparse
//! fieldsets (в спеке нет параметра fields), поэтому это проекция на нашей стороне; имена — как
//! в заголовке CSV: `id`, `type`, атрибуты, `<rel>_ref`, `<rel>.<attr>`.

use std::collections::BTreeSet;

use super::projection::{ColumnKey, Row};
use super::schema::{infer_type, Column, Schema};
use crate::error::CliError;

/// Дальше опечатку уже не угадать: подсказка сбивала бы с толку.
const MAX_SUGGESTION_DISTANCE: usize = 2;

/// Локальная проверка списка — до запроса, чтобы ошибка не тратила квоту открытий scroll.
pub fn parse_fields(list: &str, include: &[String]) -> Result<Vec<String>, CliError> {
    let mut fields: Vec<String> = Vec::new();
    for item in list.split(',').map(str::trim) {
        if item.is_empty() {
            return Err(invalid("пустой элемент в --fields".into()));
        }
        if fields.iter().any(|f| f == item) {
            return Err(CliError::usage(
                "duplicate_field",
                format!("колонка «{item}» указана в --fields дважды"),
            )
            .with_field("fields"));
        }
        if let Some((rel, attr)) = item.split_once('.') {
            // В JSON:API точки в именах нет, поэтому `a.b.c` не совпало бы ни с одной колонкой.
            if rel.is_empty() || attr.is_empty() || attr.contains('.') {
                return Err(invalid(format!(
                    "колонка «{item}»: ожидается <связь>.<атрибут>"
                )));
            }
            if !include.iter().any(|i| i == rel) {
                return Err(CliError::usage(
                    "field_needs_include",
                    format!(
                        "колонка «{item}» берётся из связанного объекта {rel}, а он не запрошен"
                    ),
                )
                .with_field("fields")
                .with_hint(format!("добавьте {rel} в --include")));
            }
        }
        fields.push(item.to_string());
    }
    Ok(fields)
}

fn invalid(message: String) -> CliError {
    CliError::usage("invalid_fields", message).with_field("fields")
}

/// Колонки из `--fields` ровно в заданном порядке. Имена сверяются с данными, а не со спекой:
/// сервер отдаёт атрибуты, которых в ней нет. `strict` — ошибка на неизвестное имя; без него
/// (страницу уже не перезапросить) такая колонка просто пустая, а `absent` её назовёт.
pub fn select(fields: &[String], rows: &[Row], strict: bool) -> Result<Schema, CliError> {
    let observed = Observed::new(rows);
    let columns = fields
        .iter()
        .map(|name| {
            let key = match resolve(name, &observed) {
                Ok(key) => key,
                Err(_) if !strict => unchecked_key(name),
                Err(err) => return Err(err),
            };
            let ty = infer_type(rows.iter().filter_map(|row| row.get(&key)));
            Ok(Column {
                key,
                name: name.clone(),
                ty,
            })
        })
        .collect::<Result<_, CliError>>()?;
    Ok(Schema { columns })
}

/// Запрошенные колонки, которых точно нет в этих строках: ключа нет ни в одной, а для
/// `<rel>.<attr>` — при том, что объекты связи в строках есть (иначе судить не о чем).
pub fn absent<'s>(schema: &'s Schema, rows: &[Row]) -> Vec<&'s str> {
    if rows.is_empty() {
        return Vec::new();
    }
    let observed = Observed::new(rows);
    schema
        .columns
        .iter()
        .filter(|column| {
            !observed.keys.contains(&column.key)
                && match &column.key {
                    ColumnKey::Included(rel, _) => observed.objects_seen(rel),
                    _ => true,
                }
        })
        .map(|column| column.name.as_str())
        .collect()
}

/// Ключ для имени, которое не удалось сверить: колонка останется пустой.
fn unchecked_key(name: &str) -> ColumnKey {
    match name.split_once('.') {
        Some((rel, attr)) => ColumnKey::Included(rel.into(), attr.into()),
        None => ColumnKey::Attribute(name.into()),
    }
}

/// Что видно в строках страницы: ключи колонок и связи-списки (их `_ref` — массив id).
struct Observed<'a> {
    keys: BTreeSet<&'a ColumnKey>,
    to_many: BTreeSet<&'a str>,
}

impl<'a> Observed<'a> {
    fn new(rows: &'a [Row]) -> Self {
        let keys = rows.iter().flat_map(|row| row.keys()).collect();
        let to_many = rows
            .iter()
            .flat_map(|row| row.iter())
            .filter_map(|(key, value)| match key {
                ColumnKey::Ref(rel) if value.is_array() => Some(rel.as_str()),
                _ => None,
            })
            .collect();
        Observed { keys, to_many }
    }

    fn objects_seen(&self, rel: &str) -> bool {
        self.keys
            .iter()
            .any(|k| matches!(k, ColumnKey::Included(r, _) if r == rel))
    }
}

/// Имя колонки → ключ строки. Без данных (пустая выгрузка) сверять не с чем — нужен только заголовок.
fn resolve(name: &str, observed: &Observed) -> Result<ColumnKey, CliError> {
    let seen = &observed.keys;
    if let Some((rel, attr)) = name.split_once('.') {
        if observed.to_many.contains(rel) {
            return Err(CliError::usage(
                "field_to_many",
                format!("колонка «{name}»: {rel} — список, его объекты в CSV не разворачиваются"),
            )
            .with_field("fields")
            .with_hint(format!(
                "id объектов — в колонке {rel}_ref, сами объекты — в --format jsonl"
            )));
        }
        let key = ColumnKey::Included(rel.into(), attr.into());
        // Связанного объекта может не быть ни у одной записи страницы — тогда колонка просто пустая.
        if observed.objects_seen(rel) && !seen.contains(&key) {
            return Err(unknown(name, seen));
        }
        return Ok(key);
    }
    if let Some(key) = seen.iter().find(|k| k.name() == name) {
        return Ok((*key).clone());
    }
    match name {
        "id" => Ok(ColumnKey::Id),
        "type" => Ok(ColumnKey::Type),
        _ if seen.is_empty() => Ok(ColumnKey::Attribute(name.into())),
        _ => Err(unknown(name, seen)),
    }
}

/// Как `gh --json`: при опечатке — ближайшее имя и все колонки в порядке заголовка CSV.
fn unknown(name: &str, seen: &BTreeSet<&ColumnKey>) -> CliError {
    let names: Vec<String> = seen.iter().map(|k| k.name()).collect();
    let suggestion = names
        .iter()
        .map(|n| (distance(name, n), n))
        .filter(|(d, _)| *d <= MAX_SUGGESTION_DISTANCE)
        .min_by_key(|(d, _)| *d)
        .map(|(_, n)| format!("может, {n}? "))
        .unwrap_or_default();
    CliError::usage("unknown_field", format!("в данных нет колонки «{name}»"))
        .with_field("fields")
        .with_hint(format!(
            "{suggestion}колонки на первой странице: {}",
            names.join(", ")
        ))
}

/// Расстояние Левенштейна по символам.
fn distance(a: &str, b: &str) -> usize {
    let b: Vec<char> = b.chars().collect();
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.chars().enumerate() {
        let mut cur = vec![i + 1];
        for (j, cb) in b.iter().enumerate() {
            let replace = prev[j] + usize::from(ca != *cb);
            cur.push(replace.min(prev[j + 1] + 1).min(cur[j] + 1));
        }
        prev = cur;
    }
    prev[b.len()]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::output::tabular::{project, ColumnKey};
    use crate::output::test_util::fixture_page;
    use crate::page::IncludedIndex;
    use serde_json::json;

    fn list(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    fn code(result: Result<Vec<String>, CliError>) -> String {
        result.unwrap_err().code
    }

    /// Строки первой страницы фикстуры; `author` запрошен, `product` — нет.
    fn fixture_rows() -> Vec<Row> {
        let page = fixture_page();
        let index = IncludedIndex::new(&page.included);
        page.data
            .iter()
            .map(|r| project(r, &index, &list(&["author"])))
            .collect()
    }

    fn names(schema: &Schema) -> Vec<&str> {
        schema.columns.iter().map(|c| c.name.as_str()).collect()
    }

    #[test]
    fn parses_trimmed_list_in_given_order() {
        assert_eq!(
            parse_fields(" rating , id,product.name ", &list(&["product"])).unwrap(),
            ["rating", "id", "product.name"]
        );
    }

    #[test]
    fn rejects_empty_duplicate_and_malformed_items() {
        let none: &[String] = &[];
        assert_eq!(code(parse_fields("", none)), "invalid_fields");
        assert_eq!(code(parse_fields("id,,rating", none)), "invalid_fields");
        assert_eq!(code(parse_fields("id,rating,id", none)), "duplicate_field");
        let product = list(&["product"]);
        for bad in [".name", "product.", "product.brand.name"] {
            assert_eq!(code(parse_fields(bad, &product)), "invalid_fields", "{bad}");
        }
    }

    #[test]
    fn related_column_needs_its_include() {
        let err = parse_fields("id,product.name", &list(&["author"])).unwrap_err();
        assert_eq!(err.code, "field_needs_include");
        assert_eq!(err.field.as_deref(), Some("fields"));
        assert!(err.hint.unwrap().contains("--include"));
    }

    #[test]
    fn select_keeps_order_and_resolves_refs_and_includes() {
        let fields = list(&["rating", "id", "author.email", "product_ref"]);
        let schema = select(&fields, &fixture_rows(), true).unwrap();
        assert_eq!(
            names(&schema),
            ["rating", "id", "author.email", "product_ref"]
        );
        let keys: Vec<&ColumnKey> = schema.columns.iter().map(|c| &c.key).collect();
        assert_eq!(
            keys,
            [
                &ColumnKey::Attribute("rating".into()),
                &ColumnKey::Id,
                &ColumnKey::Included("author".into(), "email".into()),
                &ColumnKey::Ref("product".into()),
            ]
        );
    }

    #[test]
    fn select_suggests_the_closest_column_for_a_typo() {
        let err = select(&list(&["id", "raiting"]), &fixture_rows(), true).unwrap_err();
        assert_eq!(err.code, "unknown_field");
        assert_eq!(err.exit, crate::error::Exit::Usage);
        assert!(err.message.contains("«raiting»"), "{}", err.message);
        let hint = err.hint.unwrap();
        assert!(hint.starts_with("может, rating?"), "{hint}");
        assert!(
            hint.contains("body") && hint.contains("author.email"),
            "{hint}"
        );
    }

    #[test]
    fn related_attributes_are_checked_only_when_objects_are_on_the_page() {
        let err = select(&list(&["author.emial"]), &fixture_rows(), true).unwrap_err();
        assert_eq!(err.code, "unknown_field");
        // Ни у одной записи нет объекта product: сверять не с чем, колонка просто пустая.
        let row = Row::from([
            (ColumnKey::Id, json!("r1")),
            (ColumnKey::Ref("product".into()), json!(null)),
        ]);
        assert_eq!(
            names(&select(&list(&["id", "product.name"]), &[row], true).unwrap()),
            ["id", "product.name"]
        );
    }

    /// comments — список: его объекты в CSV не разворачиваются, колонка была бы пустой всегда.
    #[test]
    fn related_column_of_a_to_many_relation_is_rejected() {
        let page = fixture_page();
        let index = IncludedIndex::new(&page.included);
        let rows: Vec<Row> = page
            .data
            .iter()
            .map(|r| project(r, &index, &list(&["comments"])))
            .collect();
        let err = select(&list(&["id", "comments.body"]), &rows, true).unwrap_err();
        assert_eq!(err.code, "field_to_many");
        assert!(err.hint.unwrap().contains("comments_ref"));
    }

    #[test]
    fn without_rows_the_names_are_the_header() {
        assert_eq!(
            names(&select(&list(&["rating", "id"]), &[], true).unwrap()),
            ["rating", "id"]
        );
    }

    /// Продолжение обхода и страницы после пустой первой: перезапросить их нельзя, поэтому
    /// неизвестное имя — не ошибка, а пустая колонка и предупреждение (см. `absent`).
    #[test]
    fn lenient_select_accepts_what_it_cannot_check_and_absent_names_it() {
        let rows = fixture_rows();
        let fields = list(&["id", "raiting", "author.emial", "author.email"]);
        let schema = select(&fields, &rows, false).unwrap();
        assert_eq!(
            names(&schema),
            ["id", "raiting", "author.emial", "author.email"]
        );
        assert_eq!(absent(&schema, &rows), ["raiting", "author.emial"]);
        let no_authors = Row::from([(ColumnKey::Id, json!("r1"))]);
        assert!(
            absent(&schema, &[no_authors])
                .iter()
                .all(|n| !n.starts_with("author.")),
            "объектов author нет — про их атрибуты судить нельзя"
        );
    }
}
