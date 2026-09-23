//! Генерирует `spec_tables.rs` из вендоренной `spec/api.yaml`.
//!
//! Допустимые фильтры и include для scroll есть в спеке только как markdown-таблицы в
//! `description`, поэтому разбираем их. Не разобралось — падаем: пустая таблица молча
//! отключила бы локальную валидацию, а сервер неизвестный `include` просто игнорирует.

use std::env;
use std::fmt::Write as _;
use std::fs;
use std::path::Path;

use yaml_rust2::{Yaml, YamlLoader};

const SPEC_PATH: &str = "spec/api.yaml";
const HTTP_METHODS: [&str; 5] = ["GET", "POST", "PUT", "PATCH", "DELETE"];

fn main() {
    println!("cargo:rerun-if-changed={SPEC_PATH}");
    let source = fs::read_to_string(SPEC_PATH).unwrap_or_else(|e| panic!("{SPEC_PATH}: {e}"));
    let docs = YamlLoader::load_from_str(&source).unwrap_or_else(|e| panic!("{SPEC_PATH}: {e}"));
    let spec = &docs[0];

    let version = text(&spec["info"]["version"], "info.version");
    println!("cargo:rustc-env=APLAUT_SPEC_VERSION={version}");

    let params = &spec["components"]["parameters"];
    let records_types = enum_values(&params["scrollRecordsType"]["schema"], "scrollRecordsType");
    let filters = markdown_table(
        text(&params["scrollFilter"]["description"], "scrollFilter.description"),
        "scrollFilter",
    );
    let includes = markdown_table(
        text(&params["scrollInclude"]["description"], "scrollInclude.description"),
        "scrollInclude",
    );
    let sort = &params["scrollSort"]["schema"];
    let per_page = &params["scrollPerPage"]["schema"];

    let mut out = String::new();
    writeln!(out, "pub const SPEC_VERSION: &str = {version:?};").unwrap();
    writeln!(out, "pub static SCROLL: &[ScrollSpec] = &[").unwrap();
    for records_type in &records_types {
        let row = |table: &[(String, Vec<String>)], what: &str| {
            table
                .iter()
                .find(|(t, _)| t == records_type)
                .map(|(_, v)| v.clone())
                .unwrap_or_else(|| panic!("{what}: нет строки для {records_type}"))
        };
        writeln!(
            out,
            "    ScrollSpec {{ records_type: {records_type:?}, filters: &{:?}, includes: &{:?} }},",
            row(&filters, "scrollFilter"),
            row(&includes, "scrollInclude"),
        )
        .unwrap();
    }
    writeln!(out, "];").unwrap();
    writeln!(
        out,
        "pub static SCROLL_SORTS: &[&str] = &{:?};",
        enum_values(sort, "scrollSort")
    )
    .unwrap();
    writeln!(
        out,
        "pub const SCROLL_SORT_DEFAULT: &str = {:?};",
        text(&sort["default"], "scrollSort.default")
    )
    .unwrap();
    writeln!(
        out,
        "pub const SCROLL_PER_PAGE_MIN: u32 = {};",
        int(&per_page["minimum"], "scrollPerPage.minimum")
    )
    .unwrap();
    writeln!(
        out,
        "pub const SCROLL_PER_PAGE_MAX: u32 = {};",
        int(&per_page["maximum"], "scrollPerPage.maximum")
    )
    .unwrap();
    writeln!(
        out,
        "pub const SCROLL_PER_PAGE_DEFAULT: u32 = {};",
        int(&per_page["default"], "scrollPerPage.default")
    )
    .unwrap();
    writeln!(
        out,
        "pub static OPERATIONS: &[(&str, &str)] = &{:?};",
        operations(spec)
    )
    .unwrap();

    let dest = Path::new(&env::var("OUT_DIR").unwrap()).join("spec_tables.rs");
    fs::write(dest, out).unwrap();
}

fn text<'a>(node: &'a Yaml, what: &str) -> &'a str {
    node.as_str().unwrap_or_else(|| panic!("spec: нет {what}"))
}

fn int(node: &Yaml, what: &str) -> i64 {
    node.as_i64().unwrap_or_else(|| panic!("spec: нет {what}"))
}

fn enum_values(schema: &Yaml, what: &str) -> Vec<String> {
    schema["enum"]
        .as_vec()
        .unwrap_or_else(|| panic!("spec: {what} без enum"))
        .iter()
        .map(|v| text(v, what).to_string())
        .collect()
}

/// Строки вида `| \`reviews\` | \`a\`, \`b\` |`; заголовок и разделитель без обратных кавычек пропускаются.
fn markdown_table(description: &str, what: &str) -> Vec<(String, Vec<String>)> {
    let rows: Vec<(String, Vec<String>)> = description
        .lines()
        .map(str::trim)
        .filter(|line| line.starts_with('|'))
        .filter_map(|line| {
            let cells: Vec<&str> = line.trim_matches('|').split('|').map(str::trim).collect();
            let key = backticked(cells.first()?);
            (key.len() == 1 && cells.len() >= 2).then(|| (key[0].clone(), backticked(cells[1])))
        })
        .collect();
    if rows.is_empty() {
        panic!("spec: {what}: таблица в description не найдена");
    }
    rows
}

fn backticked(cell: &str) -> Vec<String> {
    cell.split('`')
        .skip(1)
        .step_by(2)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect()
}

fn operations(spec: &Yaml) -> Vec<(String, String)> {
    let paths = spec["paths"].as_hash().expect("spec: нет paths");
    let mut ops = Vec::new();
    for (path, item) in paths {
        let path = text(path, "path");
        for (method, _) in item.as_hash().unwrap_or_else(|| panic!("spec: {path}")) {
            let method = text(method, "method").to_ascii_uppercase();
            if HTTP_METHODS.contains(&method.as_str()) {
                ops.push((method, path.to_string()));
            }
        }
    }
    ops
}
