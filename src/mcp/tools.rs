//! Каталог инструментов (спека mcp-server §2, §6): ресурс × глагол из `resources::ALL`. Схемы — из
//! таблиц спеки API, описания — из справки clap: один источник с CLI.

use clap::CommandFactory;
use serde_json::{json, Map, Value};

use crate::cli::Cli;
use crate::commands::writes::write_operation;
use crate::filter;
use crate::output::Format;
use crate::resources::{self, Resource, Verb};
use crate::spec::{self, AttrType, AttributeSpec};

/// Инлайн — не больше одной страницы (M7).
pub const INLINE_MAX_RECORDS: u64 = spec::SCROLL_PER_PAGE_MAX as u64;
pub const INLINE_DEFAULT_RECORDS: u64 = 20;
/// Формат по умолчанию у инструментов (M12); у CLI — `raw`.
pub const DEFAULT_FORMAT: &str = "jsonl";

/// Параметры, которых нет у команды CLI: stdout дочернего процесса направляет сам сервер (§5).
pub const MCP_ONLY: [&str; 2] = ["output_file", "overwrite"];

/// Команды CLI, которых нет среди инструментов, и почему (§6).
pub static MCP_EXCLUDED: &[(&str, &str)] = &[
    ("auth login", "токен не должен проходить через модель"),
    ("auth logout", "токены и профили настраивает человек"),
    (
        "profile list",
        "профиль сервера фиксирован при запуске (M6)",
    ),
    ("profile get", "профиль сервера фиксирован при запуске (M6)"),
    ("profile set", "токены и профили настраивает человек"),
    ("profile delete", "токены и профили настраивает человек"),
    ("profile edit", "нужен терминал"),
    ("self update", "обновление бинаря — решение человека"),
    ("mcp", "сам сервер"),
];

const SCROLL_NOTE: &str = "Без output_file данные — во втором блоке ответа, не больше 100 записей \
за вызов (max_records, по умолчанию 20); со state повторный вызов отдаёт следующую порцию. С \
output_file — любой объём в файл, в ответе только итог.";
const GET_NOTE: &str = "Запись — во втором блоке ответа.";
const WRITE_NOTE: &str = "Атрибуты — параметрами; обязательность и значения проверяет aplaut до \
отправки (missing_attribute, invalid_attribute с подсказкой). dry_run: true — проверить и показать \
запрос, ничего не отправляя.";
const FORMAT_HELP: &str = "Формат данных: jsonl (по умолчанию: запись со связями из include одной \
строкой), csv (с fields — самый короткий), raw (страницы JSON:API как есть)";
const MAX_RECORDS_HELP: &str =
    "Остановиться, набрав не меньше N записей (граница — страница). Без \
output_file — 1–100, по умолчанию 20; с output_file — любое, без него — всё";
const OUTPUT_FILE_HELP: &str = "Файл для данных (путь от рабочего каталога сервера): любой объём, \
в ответе — итог и абсолютный путь. Без state существующий файл не перезаписывается (см. overwrite); \
со state — дописывается";
const OVERWRITE_HELP: &str = "Перезаписать существующий output_file (без state)";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Scroll,
    Get,
    Write,
}

/// Аннотации MCP (§2.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Hints {
    pub read_only: bool,
    pub destructive: bool,
    pub idempotent: bool,
}

const READ_ONLY: Hints = Hints {
    read_only: true,
    destructive: false,
    idempotent: true,
};
/// Меняет окружение, ничего не перезаписывая: запись в API или файлы выгрузки.
const ADDITIVE: Hints = Hints {
    read_only: false,
    destructive: false,
    idempotent: false,
};
/// Перезаписывает значения атрибутов; повтор с теми же аргументами ничего не добавляет.
const OVERWRITES: Hints = Hints {
    read_only: false,
    destructive: true,
    idempotent: true,
};

#[derive(Debug, Clone)]
pub struct ToolDef {
    /// `reviews_scroll` — публичный контракт (M13).
    pub name: String,
    /// Путь команды CLI: `["reviews", "scroll"]`.
    pub command: [&'static str; 2],
    pub kind: Kind,
    pub title: String,
    pub description: String,
    /// JSON Schema объекта аргументов.
    pub schema: Map<String, Value>,
    pub hints: Hints,
    /// Позиционный аргумент команды (`id`, `review_id`) — в argv после `--` (M9).
    pub positional: Option<String>,
    /// Атрибуты тела записи — JSON-объектом в `--data -` (M10).
    pub attributes: Vec<&'static str>,
}

impl ToolDef {
    pub fn writes(&self) -> bool {
        self.kind == Kind::Write
    }

    /// Имя команды в конверте: `reviews.scroll`.
    pub fn command_name(&self) -> String {
        self.command.join(".")
    }

    pub fn properties(&self) -> &Map<String, Value> {
        self.schema["properties"]
            .as_object()
            .expect("у схемы инструмента есть properties")
    }
}

/// Все инструменты, включая запись.
pub fn catalog() -> Vec<ToolDef> {
    let mut root = Cli::command();
    root.build();
    let mut tools = Vec::new();
    for resource in resources::ALL {
        let group = root
            .find_subcommand(resource.name)
            .expect("ресурс есть в дереве clap (spec_drift)");
        for verb in resource.verbs {
            let leaf = group
                .find_subcommand(verb.name())
                .expect("глагол ресурса есть в дереве clap (spec_drift)");
            tools.push(tool(resource, *verb, group, leaf));
        }
    }
    tools
}

/// Инструменты сервера: запись — только с `--allow-writes` (M5).
pub fn enabled(allow_writes: bool) -> Vec<ToolDef> {
    catalog()
        .into_iter()
        .filter(|t| allow_writes || !t.writes())
        .collect()
}

/// Имена инструментов (`reviews_create`) по таблице ресурсов, без дерева clap: список нужен и
/// справке `aplaut mcp` (она сама часть дерева), и `instructions` сервера (спека writes-and-exports
/// R12).
pub fn names(writes: bool) -> Vec<String> {
    resources::ALL
        .iter()
        .flat_map(|resource| {
            resource
                .verbs
                .iter()
                .filter(move |verb| verb.writes() == writes)
                .map(move |verb| format!("{}_{}", resource.name, verb.name()))
        })
        .collect()
}

/// Вид глагола → схема, аннотации (M8): новый `Verb` не соберётся без ветки здесь.
fn tool(
    resource: &'static Resource,
    verb: Verb,
    group: &clap::Command,
    leaf: &clap::Command,
) -> ToolDef {
    let (kind, hints) = match verb {
        Verb::Scroll => (Kind::Scroll, ADDITIVE),
        Verb::Get => (Kind::Get, READ_ONLY),
        Verb::Create | Verb::Comment => (Kind::Write, ADDITIVE),
        Verb::Update => (Kind::Write, OVERWRITES),
    };
    let (mut properties, attributes, note) = match kind {
        Kind::Scroll => (scroll_properties(resource, leaf), Vec::new(), SCROLL_NOTE),
        Kind::Get => (get_properties(resource, leaf), Vec::new(), GET_NOTE),
        Kind::Write => {
            let (properties, attributes) = write_properties(resource, verb, leaf);
            (properties, attributes, WRITE_NOTE)
        }
    };
    let positional = leaf
        .get_arguments()
        .find(|a| a.is_positional())
        .map(|a| a.get_id().as_str().to_string());
    let mut schema = Map::new();
    schema.insert("type".into(), json!("object"));
    if let Some(name) = &positional {
        properties.insert(
            name.clone(),
            json!({"type": "string", "description": help(leaf, name)}),
        );
        schema.insert("required".into(), json!([name]));
    }
    schema.insert("properties".into(), Value::Object(properties));
    schema.insert("additionalProperties".into(), json!(false));
    let about = text(leaf.get_about()).replace("{records_type}", resource.records_type);
    ToolDef {
        name: format!("{}_{}", resource.name, verb.name()),
        command: [resource.name, verb.name()],
        kind,
        title: format!("{}: {}", text(group.get_about()), verb.name()),
        description: format!("{about}.\n\n{note}"),
        schema,
        hints,
        positional,
        attributes,
    }
}

fn scroll_properties(resource: &Resource, leaf: &clap::Command) -> Map<String, Value> {
    let spec = spec::scroll_spec(resource.records_type).expect("scroll есть в спеке (spec_drift)");
    let mut p = Map::new();
    let filter = format!(
        "{}. Параметры: {}. Операторы: {}.",
        help(leaf, "filter"),
        spec.filters.join(", "),
        filter::OPERATORS
    );
    p.insert(
        "filter".into(),
        json!({"type": "string", "description": filter}),
    );
    p.insert(
        "sort".into(),
        json!({"type": "string", "enum": spec::SCROLL_SORTS, "description": help(leaf, "sort")}),
    );
    p.insert(
        "include".into(),
        string_list(spec.includes, &help(leaf, "include")),
    );
    p.extend(format_properties(leaf));
    p.insert(
        "max_records".into(),
        json!({"type": "integer", "minimum": 1, "description": MAX_RECORDS_HELP}),
    );
    p.insert(
        "state".into(),
        json!({"type": "string", "description": help(leaf, "state")}),
    );
    p.insert(
        "output_file".into(),
        json!({"type": "string", "description": OUTPUT_FILE_HELP}),
    );
    p.insert(
        "overwrite".into(),
        json!({"type": "boolean", "description": OVERWRITE_HELP}),
    );
    p
}

fn get_properties(resource: &Resource, leaf: &clap::Command) -> Map<String, Value> {
    let includes =
        spec::get_includes(resource.records_type).expect("get есть в спеке (spec_drift)");
    let mut p = Map::new();
    p.insert(
        "include".into(),
        string_list(includes, &help(leaf, "include")),
    );
    p.extend(format_properties(leaf));
    p
}

fn format_properties(leaf: &clap::Command) -> Map<String, Value> {
    let mut p = Map::new();
    p.insert(
        "format".into(),
        json!({"type": "string", "enum": Format::NAMES, "default": DEFAULT_FORMAT, "description": FORMAT_HELP}),
    );
    p.insert(
        "fields".into(),
        json!({"type": "array", "items": {"type": "string"}, "description": help(leaf, "fields")}),
    );
    p
}

/// Атрибуты тела из схемы спеки (§2.1) и `dry_run`; имена атрибутов — для `--data -`.
fn write_properties(
    resource: &Resource,
    verb: Verb,
    leaf: &clap::Command,
) -> (Map<String, Value>, Vec<&'static str>) {
    let (spec, _) = write_operation(resource, verb).expect("схема тела есть в спеке (spec_drift)");
    let mut p = Map::new();
    for attribute in spec.attributes {
        p.insert(
            attribute.name.into(),
            attribute_schema(attribute, spec.nullable, &help(leaf, attribute.name)),
        );
    }
    p.insert(
        "dry_run".into(),
        json!({"type": "boolean", "description": help(leaf, "dry_run")}),
    );
    (p, spec.attributes.iter().map(|a| a.name).collect())
}

fn attribute_schema(attribute: &AttributeSpec, nullable: bool, help: &str) -> Value {
    let ty = json_type(attribute.ty);
    let mut schema = Map::new();
    schema.insert(
        "type".into(),
        if nullable {
            json!([ty, "null"])
        } else {
            json!(ty)
        },
    );
    if !attribute.enum_values.is_empty() {
        let mut values: Vec<Value> = attribute.enum_values.iter().map(|v| json!(v)).collect();
        if nullable {
            values.push(Value::Null);
        }
        schema.insert("enum".into(), Value::Array(values));
    }
    if let Some(min) = attribute.minimum {
        schema.insert("minimum".into(), json!(min));
    }
    if let Some(max) = attribute.maximum {
        schema.insert("maximum".into(), json!(max));
    }
    if let Some(format) = attribute.format {
        schema.insert("format".into(), json!(format));
    }
    if let Some(item) = attribute.item_type {
        schema.insert("items".into(), json!({"type": json_type(item)}));
    }
    if !help.is_empty() {
        schema.insert("description".into(), json!(help));
    }
    Value::Object(schema)
}

fn json_type(ty: AttrType) -> &'static str {
    match ty {
        AttrType::String => "string",
        AttrType::Number => "number",
        AttrType::Integer => "integer",
        AttrType::Boolean => "boolean",
        AttrType::Array => "array",
        AttrType::Object => "object",
    }
}

fn string_list(values: &[&str], description: &str) -> Value {
    json!({"type": "array", "items": {"type": "string", "enum": values}, "uniqueItems": true, "description": description})
}

/// Справка флага `--<name>` (подчёркивания → дефисы) или позиционного аргумента `<name>`.
fn help(leaf: &clap::Command, name: &str) -> String {
    let long = name.replace('_', "-");
    leaf.get_arguments()
        .find(|a| {
            a.get_long() == Some(long.as_str())
                || (a.is_positional() && a.get_id().as_str() == name)
        })
        .map(|a| text(a.get_help()))
        .unwrap_or_default()
}

fn text(styled: Option<&clap::builder::StyledStr>) -> String {
    styled.map(|s| s.to_string()).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn find<'a>(tools: &'a [ToolDef], name: &str) -> &'a ToolDef {
        tools
            .iter()
            .find(|t| t.name == name)
            .unwrap_or_else(|| panic!("нет {name}"))
    }

    fn leaves(cmd: &clap::Command, prefix: &str, out: &mut Vec<String>) {
        for sub in cmd.get_subcommands().filter(|s| s.get_name() != "help") {
            let path = if prefix.is_empty() {
                sub.get_name().to_string()
            } else {
                format!("{prefix} {}", sub.get_name())
            };
            if sub.has_subcommands() {
                leaves(sub, &path, out);
            } else {
                out.push(path);
            }
        }
    }

    /// Списки для справки `aplaut mcp` и `instructions` — те же, что каталог (R12).
    #[test]
    fn names_by_mode_match_the_catalog() {
        let catalog = catalog();
        for writes in [false, true] {
            let expected: Vec<String> = catalog
                .iter()
                .filter(|t| t.writes() == writes)
                .map(|t| t.name.clone())
                .collect();
            assert_eq!(names(writes), expected, "writes: {writes}");
        }
    }

    #[test]
    fn names_are_resource_and_verb() {
        let names: Vec<String> = catalog().into_iter().map(|t| t.name).collect();
        assert_eq!(
            names,
            [
                "reviews_scroll",
                "reviews_get",
                "reviews_create",
                "reviews_comment",
                "reviews_update",
                "products_scroll",
                "products_get",
                "products_create",
                "products_update",
                "questions_scroll",
                "questions_get"
            ]
        );
        assert!(enabled(false).iter().all(|t| !t.writes()));
        assert_eq!(enabled(true).len(), catalog().len());
    }

    /// Страж §6: новая команда CLI без решения по MCP роняет тест.
    #[test]
    fn every_cli_command_is_a_tool_or_excluded() {
        let tools: Vec<String> = catalog().iter().map(|t| t.command.join(" ")).collect();
        let mut all = Vec::new();
        leaves(&Cli::command(), "", &mut all);
        for path in &all {
            let excluded = MCP_EXCLUDED.iter().any(|(c, _)| c == path);
            assert!(
                tools.contains(path) != excluded,
                "«{path}»: должна быть либо инструментом, либо в MCP_EXCLUDED с причиной"
            );
        }
        for (command, _) in MCP_EXCLUDED {
            assert!(
                all.iter().any(|p| p == command),
                "«{command}» в MCP_EXCLUDED, но такой команды нет"
            );
        }
    }

    /// Страж §6: параметр — флаг или позиционный аргумент команды; переименовали флаг — тест упал.
    #[test]
    fn parameters_are_flags_of_their_command() {
        let mut root = Cli::command();
        root.build();
        for tool in catalog() {
            let leaf = root
                .find_subcommand(tool.command[0])
                .and_then(|c| c.find_subcommand(tool.command[1]))
                .unwrap();
            for name in tool.properties().keys() {
                if tool.attributes.contains(&name.as_str()) || MCP_ONLY.contains(&name.as_str()) {
                    continue;
                }
                let long = name.replace('_', "-");
                let known = leaf.get_arguments().any(|a| {
                    a.get_long() == Some(long.as_str())
                        || (a.is_positional() && a.get_id().as_str() == name)
                });
                assert!(
                    known,
                    "{}: параметр {name} — не флаг `aplaut {}`",
                    tool.name,
                    tool.command.join(" ")
                );
            }
        }
    }

    #[test]
    fn every_write_attribute_is_a_typed_parameter() {
        for tool in catalog().iter().filter(|t| t.writes()) {
            let resource = resources::find(tool.command[0]).unwrap();
            let verb = *resource
                .verbs
                .iter()
                .find(|v| v.name() == tool.command[1])
                .unwrap();
            let (spec, _) = write_operation(resource, verb).unwrap();
            for attribute in spec.attributes {
                let schema = &tool.properties()[attribute.name];
                assert!(
                    schema.get("type").is_some(),
                    "{}.{}",
                    tool.name,
                    attribute.name
                );
                if !attribute.enum_values.is_empty() {
                    assert!(
                        schema.get("enum").is_some(),
                        "{}.{}",
                        tool.name,
                        attribute.name
                    );
                }
            }
            assert!(tool.properties().contains_key("dry_run"), "{}", tool.name);
        }
        let catalog = catalog();
        let update = find(&catalog, "products_update");
        assert_eq!(
            update.properties()["name"]["type"],
            json!(["string", "null"]),
            "null очищает атрибут"
        );
        let create = find(&catalog, "reviews_create");
        assert_eq!(create.properties()["rating"]["type"], "number");
        assert_eq!(
            create.properties()["rating"]["description"],
            "Оценка, число от 1 до 5"
        );
    }

    #[test]
    fn attributes_do_not_collide_with_tool_parameters() {
        for tool in catalog() {
            for attribute in &tool.attributes {
                assert!(
                    !["id", "review_id", "dry_run"].contains(attribute),
                    "{}: атрибут {attribute} совпал с параметром инструмента",
                    tool.name
                );
            }
        }
    }

    #[test]
    fn required_is_only_the_positional_id() {
        let catalog = catalog();
        for (name, required) in [
            ("reviews_scroll", None),
            ("reviews_get", Some(json!(["id"]))),
            ("reviews_create", None),
            ("reviews_comment", Some(json!(["review_id"]))),
            ("reviews_update", Some(json!(["id"]))),
            ("products_update", Some(json!(["id"]))),
        ] {
            let tool = find(&catalog, name);
            assert_eq!(tool.schema.get("required").cloned(), required, "{name}");
            assert_eq!(tool.schema["additionalProperties"], false, "{name}");
        }
    }

    #[test]
    fn descriptions_come_from_cli_help_and_spec() {
        let catalog = catalog();
        let scroll = find(&catalog, "reviews_scroll");
        assert!(
            scroll.description.contains("/scroll/reviews"),
            "{}",
            scroll.description
        );
        assert!(
            scroll.description.contains("output_file"),
            "{}",
            scroll.description
        );
        let filter = scroll.properties()["filter"]["description"]
            .as_str()
            .unwrap();
        for name in spec::scroll_spec("reviews").unwrap().filters {
            assert!(filter.contains(name), "нет фильтра {name}: {filter}");
        }
        assert!(filter.contains("gte"), "{filter}");
        assert_eq!(scroll.properties()["format"]["default"], "jsonl");
        let includes = &scroll.properties()["include"]["items"]["enum"];
        assert_eq!(
            includes,
            &json!(spec::scroll_spec("reviews").unwrap().includes)
        );
        assert_eq!(scroll.title, "Отзывы: scroll");
    }
}
