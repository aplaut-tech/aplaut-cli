//! Запись одним запросом — POST или PUT (спеки reviews-write §3–5, products-write §3): атрибуты из
//! `--data` и флагов, проверка по схеме тела из спеки до сети, документ JSON:API. POST у API не
//! идемпотентен, поэтому ошибка во входных данных должна ловиться здесь, а не повтором запроса.

use std::fs;
use std::path::Path;

use serde::Serialize;
use serde_json::{Map, Value};

use crate::auth::StdinSource;
use crate::error::CliError;
use crate::http::{self, ApiClient, Method, Replay};
use crate::spec::{AttrType, AttributeSpec, WriteSpec};
use crate::suggest;
use crate::time;

/// Ключи-обёртки документа JSON:API: `--data` — только объект атрибутов.
const WRAPPER_KEYS: [&str; 2] = ["data", "attributes"];
/// Длиннее значение в сообщении об ошибке не показываем.
const MAX_SHOWN_CHARS: usize = 60;

/// Флаг команды → атрибут: имя атрибута и сырое значение флага, если он передан.
pub type Flag<'a> = (&'static str, Option<&'a str>);

/// План и отчёт команды записи: метод, путь от base URL и тело — без токена (§5).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct WriteRequest {
    pub method: &'static str,
    pub path: String,
    pub body: Value,
}

/// `result` команды записи: `{"request": …, "<outcome>": запись | null}` — одинаковый под `-n` (W9).
pub fn result(request: &WriteRequest, outcome: &str, record: Option<Value>) -> Value {
    let mut result = Map::new();
    result.insert(
        "request".into(),
        serde_json::to_value(request).expect("запрос сериализуется"),
    );
    result.insert(outcome.into(), record.unwrap_or(Value::Null));
    Value::Object(result)
}

/// stdin один: `--data -` и токен из stdin прочитать оба нельзя.
pub fn check_stdin(
    data: Option<&Path>,
    token_stdin: bool,
    token_file: Option<&Path>,
) -> Result<(), CliError> {
    let dash = Path::new("-");
    if data == Some(dash) && (token_stdin || token_file == Some(dash)) {
        return Err(CliError::usage(
            "stdin_conflict",
            "--data - и токен из stdin не могут читать один stdin",
        )
        .with_field("data")
        .with_hint("передайте токен через APLAUT_ACCESS_TOKEN_FILE, --token-file <файл> или профиль, либо атрибуты — файлом: --data review.json"));
    }
    Ok(())
}

/// `--data FILE|-`: JSON-объект атрибутов без обёртки `data`.
pub fn read_data(path: &Path, stdin: &mut StdinSource) -> Result<Map<String, Value>, CliError> {
    let text = if path == Path::new("-") {
        if stdin.is_terminal {
            return Err(CliError::usage(
                "stdin_is_terminal",
                "ожидался JSON в stdin (--data -), но stdin — терминал",
            )
            .with_field("data")
            .with_hint("передайте JSON через пайп: echo '{\"rating\":5,…}' | aplaut … --data -, или файлом: --data review.json"));
        }
        let mut text = String::new();
        stdin
            .reader
            .read_to_string(&mut text)
            .map_err(|e| invalid_data(format!("не удалось прочитать --data из stdin: {e}")))?;
        text
    } else {
        fs::read_to_string(path).map_err(|e| {
            invalid_data(format!(
                "не удалось прочитать --data {}: {e}",
                path.display()
            ))
        })?
    };
    parse_data(&text)
}

pub fn parse_data(text: &str) -> Result<Map<String, Value>, CliError> {
    let value: Value =
        serde_json::from_str(text).map_err(|e| invalid_data(format!("--data: не JSON: {e}")))?;
    let Value::Object(map) = value else {
        return Err(invalid_data(format!(
            "--data: ожидается JSON-объект атрибутов, получено {}",
            shown(&value)
        )));
    };
    if let Some(key) = WRAPPER_KEYS.iter().find(|k| map.contains_key(**k)) {
        return Err(invalid_data(format!(
            "--data: ключ «{key}» — обёртка документа JSON:API, а нужен только объект атрибутов"
        ))
        .with_hint("передайте содержимое attributes: {\"rating\":5,\"body\":\"…\"} — тип и обёртку data CLI добавит сам"));
    }
    Ok(map)
}

fn invalid_data(message: String) -> CliError {
    CliError::usage("invalid_data", message).with_field("data")
}

/// `--data` + флаги; флаги перекрывают одноимённые ключи `--data` (W5).
pub fn attributes(
    spec: &WriteSpec,
    data: Option<Map<String, Value>>,
    flags: &[Flag],
) -> Map<String, Value> {
    let from_flags = flags.iter().filter_map(|(name, raw)| {
        raw.map(|raw| (name.to_string(), flag_value(spec.attribute(name), raw)))
    });
    data.unwrap_or_default()
        .into_iter()
        .chain(from_flags)
        .collect()
}

/// Строковый атрибут — как есть; иначе — JSON-литерал (`--rating 5` — число, а не «5»). Не
/// разобралось — строка: тогда `validate` назовёт ожидаемый тип.
fn flag_value(attribute: Option<&AttributeSpec>, raw: &str) -> Value {
    match attribute {
        Some(a) if a.ty != AttrType::String => {
            serde_json::from_str(raw.trim()).unwrap_or_else(|_| Value::String(raw.to_string()))
        }
        _ => Value::String(raw.to_string()),
    }
}

/// Проверка до сети (§3): неизвестные атрибуты, типы, enum, границы, date-time, обязательные.
/// Глубже (поля объектов в массивах) проверяет сервер: 422 → `validation_failed`. `required` —
/// обычно `spec.required`; команда сужает его, если сервер строже схемы не требует (§15).
pub fn validate(
    spec: &WriteSpec,
    attributes: &Map<String, Value>,
    required: &[&str],
    flags: &[Flag],
) -> Result<(), CliError> {
    for (name, value) in attributes {
        let attribute = spec.attribute(name).ok_or_else(|| unknown(spec, name))?;
        if value.is_null() && spec.nullable {
            continue;
        }
        check(attribute, value).map_err(|problem| invalid(attribute, problem))?;
    }
    match required
        .iter()
        .find(|name| !attributes.contains_key(**name))
    {
        Some(name) => Err(missing(name, flags)),
        None => Ok(()),
    }
}

/// Документ JSON:API для тела запроса: `{"data":{"type":…,"attributes":{…}}}`.
pub fn request(spec: &WriteSpec, path: String, attributes: Map<String, Value>) -> WriteRequest {
    WriteRequest {
        method: spec.method,
        path,
        body: serde_json::json!({"data": {"type": spec.resource_type, "attributes": attributes}}),
    }
}

/// Отправка (W7). 2xx — запись создана или изменена: `data` ответа. `replay` — когда повтор
/// безопасен; исход неизвестен — в подсказке `verify`: как проверить, прежде чем повторять.
pub fn submit(
    api: &mut ApiClient,
    request: &WriteRequest,
    replay: Replay,
    verify: &str,
) -> Result<Value, CliError> {
    let method = Method::from_name(request.method).ok_or_else(|| {
        CliError::general(
            "internal",
            format!("метод записи {} не поддерживается", request.method),
        )
    })?;
    let response = api
        .write(method, &request.path, &request.body, replay)
        .map_err(|err| match err.code.as_str() {
            http::OUTCOME_UNKNOWN => err.with_hint(verify),
            _ => err,
        })?;
    serde_json::from_slice::<Value>(&response.body)
        .ok()
        .and_then(|mut doc| doc.get_mut("data").map(Value::take))
        .filter(Value::is_object)
        .ok_or_else(|| {
            // Запрос выполнен (2xx): повтор POST создал бы дубль, повтор идемпотентного PUT — нет.
            CliError::general(
                "bad_response",
                format!("сервер ответил {}, но в теле нет записи", response.status),
            )
            .retryable(replay == Replay::Safe)
            .with_request_id(response.request_id.clone())
            .with_hint(match replay {
                Replay::Safe => {
                    format!("запрос, скорее всего, выполнен; повторить его безопасно — {verify}")
                }
                Replay::OnlyIfUnprocessed => {
                    format!("запрос, скорее всего, выполнен — не повторяйте вслепую: {verify}")
                }
            })
        })
}

/// Что не так со значением: для сообщения и подсказки.
struct Problem {
    detail: String,
    hint: Option<String>,
}

impl Problem {
    fn expected(what: impl Into<String>, value: &Value) -> Problem {
        Problem {
            detail: format!("ожидается {}, получено {}", what.into(), shown(value)),
            hint: None,
        }
    }
}

fn check(attribute: &AttributeSpec, value: &Value) -> Result<(), Problem> {
    if !has_type(attribute.ty, value) {
        return Err(Problem::expected(type_name(attribute.ty), value));
    }
    if !attribute.enum_values.is_empty()
        && !value
            .as_str()
            .is_some_and(|s| attribute.enum_values.contains(&s))
    {
        return Err(Problem::expected(
            format!("одно из: {}", attribute.enum_values.join(", ")),
            value,
        ));
    }
    if let Some(number) = value.as_f64() {
        let below = attribute.minimum.is_some_and(|min| number < min);
        let above = attribute.maximum.is_some_and(|max| number > max);
        if below || above {
            return Err(Problem::expected(range(attribute), value));
        }
    }
    if attribute.format == Some("date-time")
        && !value
            .as_str()
            .is_some_and(|s| time::parse_rfc3339(s).is_some())
    {
        return Err(Problem {
            hint: Some("например 2026-09-24T12:00:00+03:00".into()),
            ..Problem::expected("дата и время RFC 3339", value)
        });
    }
    if let (Some(item_type), Some(items)) = (attribute.item_type, value.as_array()) {
        if let Some(index) = items.iter().position(|item| !has_type(item_type, item)) {
            return Err(Problem {
                detail: format!(
                    "ожидается массив, где каждый элемент — {}; элемент {index} — {}",
                    type_name(item_type),
                    shown(&items[index])
                ),
                hint: None,
            });
        }
    }
    Ok(())
}

fn has_type(ty: AttrType, value: &Value) -> bool {
    match ty {
        AttrType::String => value.is_string(),
        AttrType::Number => value.is_number(),
        // JSON Schema: целое — число без дробной части, `5.0` тоже подходит.
        AttrType::Integer => value.as_f64().is_some_and(|n| n.fract() == 0.0),
        AttrType::Boolean => value.is_boolean(),
        AttrType::Array => value.is_array(),
        AttrType::Object => value.is_object(),
    }
}

fn type_name(ty: AttrType) -> &'static str {
    match ty {
        AttrType::String => "строка",
        AttrType::Number => "число",
        AttrType::Integer => "целое число",
        AttrType::Boolean => "true или false",
        AttrType::Array => "массив",
        AttrType::Object => "объект",
    }
}

fn range(attribute: &AttributeSpec) -> String {
    match (attribute.minimum, attribute.maximum) {
        (Some(min), Some(max)) => format!("число от {min} до {max}"),
        (Some(min), None) => format!("число не меньше {min}"),
        (None, Some(max)) => format!("число не больше {max}"),
        (None, None) => "число".into(),
    }
}

/// Значение в сообщении — компактным JSON, длинное — обрезанным.
fn shown(value: &Value) -> String {
    let text = value.to_string();
    if text.chars().count() <= MAX_SHOWN_CHARS {
        return text;
    }
    let cut: String = text.chars().take(MAX_SHOWN_CHARS).collect();
    format!("{cut}…")
}

fn unknown(spec: &WriteSpec, name: &str) -> CliError {
    let names = || spec.attributes.iter().map(|a| a.name);
    let suggestion = suggest::closest(name, names())
        .map(|n| format!("может, {n}? "))
        .unwrap_or_default();
    CliError::usage(
        "unknown_attribute",
        format!(
            "атрибута «{name}» нет в схеме {} {}",
            spec.method, spec.path
        ),
    )
    .with_field(name)
    .with_hint(format!(
        "{suggestion}допустимые: {}",
        names().collect::<Vec<_>>().join(", ")
    ))
}

fn invalid(attribute: &AttributeSpec, problem: Problem) -> CliError {
    let err = CliError::usage(
        "invalid_attribute",
        format!("атрибут «{}»: {}", attribute.name, problem.detail),
    )
    .with_field(attribute.name);
    match problem.hint {
        Some(hint) => err.with_hint(hint),
        None => err,
    }
}

fn missing(name: &str, flags: &[Flag]) -> CliError {
    let how = if flags.iter().any(|(flag, _)| *flag == name) {
        format!(
            "передайте --{} или ключ «{name}» в --data",
            name.replace('_', "-")
        )
    } else {
        format!("передайте ключ «{name}» в --data")
    };
    CliError::usage(
        "missing_attribute",
        format!("не задан обязательный атрибут «{name}»"),
    )
    .with_field(name)
    .with_hint(how)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::error::Exit;
    use crate::spec;

    fn reviews() -> &'static WriteSpec {
        spec::write_spec("POST", "/reviews").unwrap()
    }

    fn object(value: Value) -> Map<String, Value> {
        value.as_object().unwrap().clone()
    }

    const REVIEW_FLAGS: [Flag<'static>; 2] = [("rating", None), ("body", None)];

    fn check_review(value: Value) -> Result<(), CliError> {
        validate(reviews(), &object(value), reviews().required, &REVIEW_FLAGS)
    }

    #[test]
    fn flags_override_data_and_take_the_attribute_type() {
        let data = object(json!({"rating": 3, "body": "из файла", "photos": ["https://x/1.jpg"]}));
        let flags = [
            ("rating", Some("4.5")),
            ("body", None),
            ("external_id", Some("42")),
        ];
        let merged = attributes(reviews(), Some(data), &flags);
        assert_eq!(
            Value::Object(merged),
            json!({"rating": 4.5, "body": "из файла", "photos": ["https://x/1.jpg"], "external_id": "42"})
        );
        let whole = attributes(reviews(), None, &[("rating", Some("5"))]);
        assert_eq!(whole["rating"], json!(5), "целое — без .0");
        let text = attributes(reviews(), None, &[("rating", Some("five"))]);
        assert_eq!(
            text["rating"], "five",
            "не число — строкой, validate назовёт тип"
        );
    }

    #[test]
    fn valid_values_pass() {
        check_review(json!({
            "rating": 5, "body": "ok", "photos": ["a"], "hide_my_data": true,
            "created_at": "2026-09-24T12:00:00+03:00", "rating_details": [{"name": "x"}],
            "custom_attributes": {"region": 66}, "likes": 3, "state": "published"
        }))
        .unwrap();
    }

    #[test]
    fn values_are_checked_against_the_schema() {
        for (value, field) in [
            (json!({"rating": "abc", "body": "ok"}), "rating"),
            (json!({"rating": 0, "body": "ok"}), "rating"),
            (json!({"rating": 5.5, "body": "ok"}), "rating"),
            (json!({"rating": 5, "body": 5}), "body"),
            (json!({"rating": 5, "body": null}), "body"),
            (json!({"rating": 5, "body": "ok", "state": "live"}), "state"),
            (
                json!({"rating": 5, "body": "ok", "photos": ["a", 1]}),
                "photos",
            ),
            (
                json!({"rating": 5, "body": "ok", "rating_details": [1]}),
                "rating_details",
            ),
            (
                json!({"rating": 5, "body": "ok", "hide_my_data": "yes"}),
                "hide_my_data",
            ),
            (
                json!({"rating": 5, "body": "ok", "custom_attributes": []}),
                "custom_attributes",
            ),
            (
                json!({"rating": 5, "body": "ok", "created_at": "вчера"}),
                "created_at",
            ),
            (json!({"rating": 5, "body": "ok", "likes": 1.5}), "likes"),
        ] {
            let err = check_review(value.clone()).unwrap_err();
            assert_eq!(
                (err.code.as_str(), err.field.as_deref(), err.exit),
                ("invalid_attribute", Some(field), Exit::Usage),
                "{value}"
            );
        }
    }

    #[test]
    fn invalid_values_say_what_is_expected() {
        let err = check_review(json!({"rating": 5, "body": "ok", "state": "live"})).unwrap_err();
        assert!(
            err.message.contains("published, waiting, banned, held"),
            "{}",
            err.message
        );
        let err =
            check_review(json!({"rating": 5, "body": "ok", "created_at": "вчера"})).unwrap_err();
        assert!(err.hint.unwrap().contains("2026-09-24T12:00:00+03:00"));
        let err = check_review(json!({"rating": 9, "body": "ok"})).unwrap_err();
        assert!(
            err.message.contains("от 1 до 5") && err.message.contains('9'),
            "{}",
            err.message
        );
        let err = check_review(json!({"rating": 5, "body": "ok", "photos": ["a", 1]})).unwrap_err();
        assert!(err.message.contains("элемент 1"), "{}", err.message);
    }

    #[test]
    fn unknown_attribute_suggests_the_closest_and_lists_all() {
        let err = check_review(json!({"raiting": 5, "rating": 5, "body": "ok"})).unwrap_err();
        assert_eq!(
            (err.code.as_str(), err.field.as_deref(), err.exit),
            ("unknown_attribute", Some("raiting"), Exit::Usage)
        );
        let hint = err.hint.unwrap();
        assert!(
            hint.starts_with("может, rating?") && hint.contains("photos"),
            "{hint}"
        );
    }

    #[test]
    fn missing_required_attribute_names_the_flag_when_there_is_one() {
        let err = check_review(json!({"rating": 5})).unwrap_err();
        assert_eq!(
            (err.code.as_str(), err.field.as_deref(), err.exit),
            ("missing_attribute", Some("body"), Exit::Usage)
        );
        assert!(err.hint.unwrap().contains("--body"));
        let err = validate(
            reviews(),
            &object(json!({"body": "ok"})),
            reviews().required,
            &[],
        )
        .unwrap_err();
        assert_eq!(err.field.as_deref(), Some("rating"));
        assert!(
            !err.hint.unwrap().contains("--rating"),
            "флага нет — только --data"
        );
    }

    #[test]
    fn data_must_be_a_bare_attributes_object() {
        assert_eq!(parse_data(r#"{"rating": 5}"#).unwrap()["rating"], 5);
        for text in [
            r#"{"data":{"type":"reviews","attributes":{"rating":5}}}"#,
            r#"{"attributes":{"rating":5}}"#,
            "[1]",
            "rating=5",
            "",
        ] {
            let err = parse_data(text).unwrap_err();
            assert_eq!(
                (err.code.as_str(), err.field.as_deref(), err.exit),
                ("invalid_data", Some("data"), Exit::Usage),
                "{text:?}"
            );
        }
        assert!(parse_data(r#"{"data":{}}"#)
            .unwrap_err()
            .hint
            .unwrap()
            .contains("attributes"));
    }

    #[test]
    fn data_comes_from_stdin_or_a_file_but_not_from_a_terminal() {
        let mut input = r#"{"body": "из stdin"}"#.as_bytes();
        let mut piped = StdinSource {
            is_terminal: false,
            reader: &mut input,
        };
        assert_eq!(
            read_data(Path::new("-"), &mut piped).unwrap()["body"],
            "из stdin"
        );
        let mut empty = "".as_bytes();
        let mut tty = StdinSource {
            is_terminal: true,
            reader: &mut empty,
        };
        let err = read_data(Path::new("-"), &mut tty).unwrap_err();
        assert_eq!(
            (err.code.as_str(), err.field.as_deref()),
            ("stdin_is_terminal", Some("data")),
            "агент в pty не должен повиснуть"
        );
        let err = read_data(Path::new("/nonexistent/review.json"), &mut tty).unwrap_err();
        assert_eq!(err.code, "invalid_data");
    }

    #[test]
    fn data_from_stdin_conflicts_with_a_token_from_stdin() {
        let dash = Path::new("-");
        for (token_stdin, token_file) in [(true, None), (false, Some(dash))] {
            let err = check_stdin(Some(dash), token_stdin, token_file).unwrap_err();
            assert_eq!(
                (err.code.as_str(), err.field.as_deref(), err.exit),
                ("stdin_conflict", Some("data"), Exit::Usage)
            );
        }
        assert!(check_stdin(Some(dash), false, Some(Path::new("token.txt"))).is_ok());
        assert!(check_stdin(Some(Path::new("review.json")), true, None).is_ok());
        assert!(check_stdin(None, true, None).is_ok());
    }

    #[test]
    fn result_has_the_request_and_the_record_under_its_key() {
        let plan = request(reviews(), "/reviews".into(), object(json!({"rating": 5})));
        let planned = result(&plan, "created", None);
        assert_eq!(planned["created"], Value::Null);
        assert_eq!(planned["request"]["path"], "/reviews");
        let done = result(&plan, "updated", Some(json!({"id": "p1"})));
        assert_eq!(done["updated"]["id"], "p1");
        assert!(done.get("created").is_none());
    }

    #[test]
    fn request_is_a_json_api_document_without_the_token() {
        let plan = request(reviews(), "/reviews".into(), object(json!({"rating": 5})));
        assert_eq!(
            serde_json::to_value(&plan).unwrap(),
            json!({"method": "POST", "path": "/reviews",
                   "body": {"data": {"type": "reviews", "attributes": {"rating": 5}}}})
        );
    }
}
