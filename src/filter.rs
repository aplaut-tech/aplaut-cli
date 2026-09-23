//! Разбор и локальная проверка `--filter`, `--include`, `--sort`, `--per-page`.
//!
//! Проверять до запроса важно: неизвестный `include` сервер молча игнорирует, а каждое
//! отклонённое открытие scroll съедает одно из 5 открытий в минуту (проверено на стейджинге).

use crate::error::CliError;

pub const MAX_IN_VALUES: usize = 25;
const OPERATORS: &str = "gt, gte, lt, lte, eq, neq, in, exists";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Op {
    Gt,
    Gte,
    Lt,
    Lte,
    Eq,
    Neq,
    In,
    Exists,
}

impl Op {
    fn parse(s: &str) -> Option<Op> {
        Some(match s {
            "gt" => Op::Gt,
            "gte" => Op::Gte,
            "lt" => Op::Lt,
            "lte" => Op::Lte,
            "eq" => Op::Eq,
            "neq" => Op::Neq,
            "in" => Op::In,
            "exists" => Op::Exists,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Clause {
    pub param: String,
    pub op: Op,
    pub values: Vec<String>,
}

pub fn parse_filter(expr: &str) -> Result<Vec<Clause>, CliError> {
    let expr = expr.trim();
    if expr.is_empty() {
        return Err(invalid_filter("пустой фильтр"));
    }
    expr.split(',').map(parse_clause).collect()
}

pub fn check_filter(expr: &str, allowed: &[&str]) -> Result<(), CliError> {
    for clause in parse_filter(expr)? {
        if !allowed.contains(&clause.param.as_str()) {
            return Err(invalid_filter(&format!(
                "параметр фильтра «{}» не поддерживается",
                clause.param
            ))
            .with_hint(format!("допустимые параметры: {}", allowed.join(", "))));
        }
    }
    Ok(())
}

pub fn parse_include(list: &str, allowed: &[&str]) -> Result<Vec<String>, CliError> {
    let mut out: Vec<String> = Vec::new();
    for item in list.split(',').map(str::trim) {
        if item.is_empty() {
            return Err(
                CliError::usage("invalid_include", "пустой элемент в --include").with_field("include")
            );
        }
        if !allowed.contains(&item) {
            return Err(CliError::usage(
                "invalid_include",
                format!("неизвестное значение include «{item}»"),
            )
            .with_field("include")
            .with_hint(format!("допустимые значения: {}", allowed.join(", "))));
        }
        if !out.iter().any(|x| x == item) {
            out.push(item.to_string());
        }
    }
    Ok(out)
}

pub fn check_sort(sort: &str, allowed: &[&str]) -> Result<(), CliError> {
    if allowed.contains(&sort) {
        return Ok(());
    }
    Err(
        CliError::usage("invalid_sort", format!("сортировка «{sort}» не поддерживается"))
            .with_field("sort")
            .with_hint(format!("допустимые значения: {}", allowed.join(", "))),
    )
}

pub fn check_per_page(per_page: u32, min: u32, max: u32) -> Result<(), CliError> {
    if (min..=max).contains(&per_page) {
        return Ok(());
    }
    Err(CliError::usage(
        "invalid_per_page",
        format!("--per-page должно быть от {min} до {max}"),
    )
    .with_field("per_page"))
}

fn parse_clause(block: &str) -> Result<Clause, CliError> {
    let block = block.trim();
    // Значение — всё после второго двоеточия: в датах своих двоеточий хватает.
    let mut parts = block.splitn(3, ':');
    let (param, op, value) = match (parts.next(), parts.next(), parts.next()) {
        (Some(p), Some(o), Some(v)) if !p.is_empty() && !v.is_empty() => (p, o, v),
        _ => {
            return Err(invalid_filter(&format!(
                "блок «{block}» не в формате параметр:оператор:значение"
            )))
        }
    };
    let op = Op::parse(op).ok_or_else(|| {
        invalid_filter(&format!("неизвестный оператор «{op}» в «{block}»"))
            .with_hint(format!("операторы: {OPERATORS}"))
    })?;
    let values: Vec<String> = match op {
        Op::In => value.split('|').map(str::to_string).collect(),
        _ => vec![value.to_string()],
    };
    if op == Op::In && (values.len() > MAX_IN_VALUES || values.iter().any(String::is_empty)) {
        return Err(invalid_filter(&format!(
            "в «{block}»: оператор in принимает от 1 до {MAX_IN_VALUES} непустых значений через |"
        )));
    }
    if op == Op::Exists && value != "true" && value != "false" {
        return Err(invalid_filter(&format!(
            "в «{block}»: exists принимает true или false"
        )));
    }
    Ok(Clause {
        param: param.to_string(),
        op,
        values,
    })
}

fn invalid_filter(message: &str) -> CliError {
    CliError::usage("invalid_filter", message).with_field("filter")
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALLOWED: &[&str] = &["updated_at", "rating", "state"];

    #[test]
    fn filter_value_keeps_colons() {
        let clauses = parse_filter("updated_at:gte:2020-01-01T00:00:00Z").unwrap();
        assert_eq!(
            clauses,
            vec![Clause { param: "updated_at".into(), op: Op::Gte, values: vec!["2020-01-01T00:00:00Z".into()] }]
        );
    }

    #[test]
    fn parses_several_blocks_and_in_lists() {
        let clauses = parse_filter("rating:in:4|5,state:eq:published").unwrap();
        assert_eq!(clauses.len(), 2);
        assert_eq!(clauses[0].values, vec!["4", "5"]);
        assert_eq!(clauses[1].op, Op::Eq);
    }

    #[test]
    fn rejects_malformed_blocks() {
        for bad in ["", "rating", "rating:gt", "rating:gt:", ":gt:3", "rating:like:3", "state:exists:yes"] {
            let err = parse_filter(bad).unwrap_err();
            assert_eq!(err.code, "invalid_filter", "{bad}");
            assert_eq!(err.field.as_deref(), Some("filter"));
        }
    }

    #[test]
    fn in_list_is_limited_to_25_values() {
        let many = (0..26).map(|i| i.to_string()).collect::<Vec<_>>().join("|");
        assert!(parse_filter(&format!("rating:in:{many}")).is_err());
    }

    #[test]
    fn unknown_param_lists_allowed() {
        let err = check_filter("foo:eq:1", ALLOWED).unwrap_err();
        assert_eq!(err.code, "invalid_filter");
        assert!(err.hint.unwrap().contains("updated_at, rating, state"));
        assert!(check_filter("rating:gte:4,state:neq:banned", ALLOWED).is_ok());
    }

    #[test]
    fn include_is_validated_and_deduplicated() {
        let allowed = &["author", "product"];
        assert_eq!(parse_include("author, product,author", allowed).unwrap(), vec!["author", "product"]);
        let err = parse_include("author,nope", allowed).unwrap_err();
        assert_eq!(err.code, "invalid_include");
        assert_eq!(err.field.as_deref(), Some("include"));
        assert!(parse_include("author,,product", allowed).is_err());
    }

    #[test]
    fn sort_and_per_page_ranges() {
        assert!(check_sort("created_at:asc", &["updated_at:asc", "created_at:asc"]).is_ok());
        assert_eq!(check_sort("rating:desc", &["updated_at:asc"]).unwrap_err().code, "invalid_sort");
        assert!(check_per_page(100, 1, 100).is_ok());
        assert_eq!(check_per_page(101, 1, 100).unwrap_err().field.as_deref(), Some("per_page"));
        assert!(check_per_page(0, 1, 100).is_err());
    }
}
