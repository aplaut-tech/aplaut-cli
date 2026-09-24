//! Генерирует `spec_tables.rs` из вендоренной `spec/api.yaml`.
//!
//! Допустимые фильтры и include для scroll есть в спеке только как markdown-таблицы в
//! `description`, поэтому разбираем их. Не разобралось — падаем: пустая таблица молча
//! отключила бы локальную валидацию, а сервер неизвестный `include` просто игнорирует.
//!
//! Для `get` и записи — `include` из параметров `GET /{type}/{id}` и схемы тел
//! `WRITE_OPERATIONS` (спека reviews-write §7).

use std::env;
use std::fmt::Write as _;
use std::fs;
use std::path::Path;

use yaml_rust2::{Yaml, YamlLoader};

const SPEC_PATH: &str = "spec/api.yaml";
const HTTP_METHODS: [&str; 5] = ["GET", "POST", "PUT", "PATCH", "DELETE"];
/// Операция записи, для которой генерируется схема тела (спека reviews-write §7).
struct WriteOp {
    method: &'static str,
    path: &'static str,
    /// Операция, из тела которой берутся атрибуты.
    schema: (&'static str, &'static str),
    /// Частичное обновление: обязательных атрибутов нет.
    partial: bool,
}

const WRITE_OPERATIONS: [WriteOp; 4] = [
    WriteOp {
        method: "POST",
        path: "/reviews",
        schema: ("POST", "/reviews"),
        partial: false,
    },
    WriteOp {
        method: "POST",
        path: "/reviews/{id}/relationships/comments",
        schema: ("POST", "/reviews/{id}/relationships/comments"),
        partial: false,
    },
    WriteOp {
        method: "POST",
        path: "/products",
        schema: ("POST", "/products"),
        partial: false,
    },
    // Стейджинг, 2026-09-24 (спека products-write §6): PUT частичный и принимает атрибуты создания
    // (категорию, бренд); схема PUT в спеке — `ProductAttributes` с вычисляемыми полями.
    WriteOp {
        method: "PUT",
        path: "/products/{id}",
        schema: ("POST", "/products"),
        partial: true,
    },
];

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
        text(
            &params["scrollFilter"]["description"],
            "scrollFilter.description",
        ),
        "scrollFilter",
    );
    let includes = markdown_table(
        text(
            &params["scrollInclude"]["description"],
            "scrollInclude.description",
        ),
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
    writeln!(out, "pub static GET_INCLUDES: &[(&str, &[&str])] = &[").unwrap();
    for (records_type, includes) in get_includes(spec) {
        writeln!(out, "    ({records_type:?}, &{includes:?}),").unwrap();
    }
    writeln!(out, "];").unwrap();
    writeln!(out, "pub static WRITES: &[WriteSpec] = &[").unwrap();
    for op in &WRITE_OPERATIONS {
        write_spec(&mut out, spec, op);
    }
    writeln!(out, "];").unwrap();

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

/// `$ref` вида `#/components/…` → узел, на который он указывает (цепочки тоже).
fn resolve<'a>(spec: &'a Yaml, node: &'a Yaml, what: &str) -> &'a Yaml {
    let Some(reference) = node["$ref"].as_str() else {
        return node;
    };
    let pointer = reference
        .strip_prefix("#/")
        .unwrap_or_else(|| panic!("spec: {what}: внешний $ref {reference}"));
    let target = pointer.split('/').fold(spec, |node, key| &node[key]);
    if target.is_badvalue() {
        panic!("spec: {what}: $ref {reference} не найден");
    }
    resolve(spec, target, what)
}

/// `GET /{type}/{id}` → перечисление `include` этой операции (пусто, если параметра нет).
fn get_includes(spec: &Yaml) -> Vec<(String, Vec<String>)> {
    let paths = spec["paths"].as_hash().expect("spec: нет paths");
    let mut out = Vec::new();
    for (path, item) in paths {
        let path = text(path, "path");
        let Some(records_type) = path.strip_prefix('/').and_then(|p| p.strip_suffix("/{id}"))
        else {
            continue;
        };
        let get = &item["get"];
        if records_type.contains('/') || get.is_badvalue() {
            continue;
        }
        let includes = get["parameters"]
            .as_vec()
            .into_iter()
            .flatten()
            .map(|p| resolve(spec, p, path))
            .find(|p| p["name"].as_str() == Some("include") && p["in"].as_str() == Some("query"))
            .map(|p| enum_values(&p["schema"], path))
            .unwrap_or_default();
        out.push((records_type.to_string(), includes));
    }
    out
}

/// Строка `WriteSpec { … }`: тип документа, обязательные и таблица атрибутов из схемы тела.
fn write_spec(out: &mut String, spec: &Yaml, op: &WriteOp) {
    let (method, path) = (op.method, op.path);
    let what = format!("{method} {path}");
    if spec["paths"][path][method.to_ascii_lowercase().as_str()].is_badvalue() {
        panic!("spec: {what}: нет операции");
    }
    let (schema_method, schema_path) = op.schema;
    let operation = &spec["paths"][schema_path][schema_method.to_ascii_lowercase().as_str()];
    let data = resolve(
        spec,
        &operation["requestBody"]["content"]["application/json"]["schema"]["properties"]["data"],
        &what,
    );
    let resource_type = enum_values(&data["properties"]["type"], &what)
        .into_iter()
        .next()
        .unwrap_or_else(|| panic!("spec: {what}: пустой enum type"));
    let attributes = resolve(spec, &data["properties"]["attributes"], &what);
    let properties = attributes["properties"]
        .as_hash()
        .unwrap_or_else(|| panic!("spec: {what}: нет properties атрибутов"));
    writeln!(
        out,
        "    WriteSpec {{ method: {method:?}, path: {path:?}, resource_type: {resource_type:?}, required: &{:?}, attributes: &[",
        if op.partial {
            Vec::new()
        } else {
            strings(&attributes["required"])
        },
    )
    .unwrap();
    for (name, schema) in properties {
        let name = text(name, &what);
        let at = format!("{what}: {name}");
        let schema = resolve(spec, schema, &at);
        let ty = attr_type(&schema["type"], &at);
        let item_type = match ty {
            "Array" => format!(
                "Some(AttrType::{})",
                attr_type(&resolve(spec, &schema["items"], &at)["type"], &at)
            ),
            _ => "None".to_string(),
        };
        writeln!(
            out,
            "        AttributeSpec {{ name: {name:?}, ty: AttrType::{ty}, enum_values: &{:?}, minimum: {:?}, maximum: {:?}, format: {:?}, item_type: {item_type} }},",
            strings(&schema["enum"]),
            number(&schema["minimum"]),
            number(&schema["maximum"]),
            schema["format"].as_str(),
        )
        .unwrap();
    }
    writeln!(out, "    ], nullable: {} }},", op.partial).unwrap();
}

fn attr_type(node: &Yaml, what: &str) -> &'static str {
    match text(node, what) {
        "string" => "String",
        "number" => "Number",
        "integer" => "Integer",
        "boolean" => "Boolean",
        "array" => "Array",
        "object" => "Object",
        other => panic!("spec: {what}: неизвестный type {other}"),
    }
}

fn strings(node: &Yaml) -> Vec<String> {
    node.as_vec()
        .map(|items| {
            items
                .iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

fn number(node: &Yaml) -> Option<f64> {
    node.as_f64().or_else(|| node.as_i64().map(|v| v as f64))
}
