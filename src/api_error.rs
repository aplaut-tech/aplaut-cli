//! Ответ API с ошибкой → `CliError`.
//!
//! Реальный формат ошибок отличается от схемы `Error` спеки (дизайн §15): это
//! `{"errors": {"status": <int>, "title", "details": {<param>: [msg]} | {"message"}, "readme"}}`,
//! а 401 приходит вообще без тела — причина только в `WWW-Authenticate`. Разбор терпим к
//! форме из спеки и к JSON:API-массиву `errors`.

use serde_json::Value;

use crate::error::{CliError, Exit};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ResponseHeaders {
    pub request_id: Option<String>,
    pub retry_after: Option<String>,
    pub rate_limit_reset: Option<String>,
    pub date: Option<String>,
    pub www_authenticate: Option<String>,
    pub location: Option<String>,
}

/// Что назвать в подсказке к 401: откуда взят токен и куда шёл запрос.
#[derive(Debug, Clone, Default)]
pub struct ErrorContext {
    pub token_source: String,
    pub base_url: String,
}

pub fn from_response(status: u16, headers: &ResponseHeaders, body: &[u8], ctx: &ErrorContext) -> CliError {
    let parsed = ParsedBody::parse(body);
    let err = match status {
        401 => unauthorized(headers, ctx),
        403 => CliError::new(Exit::Auth, "forbidden", parsed.message_or("доступ запрещён"))
            .with_hint("у токена нет доступа к ресурсу: проверьте scope приложения (Platform API) в ЛК"),
        404 => CliError::new(Exit::NotFound, "not_found", parsed.message_or("объект не найден")),
        400 => bad_request(&parsed),
        422 => CliError::general("validation_failed", parsed.message_or("сервер отклонил параметры запроса")),
        429 => {
            let when = headers
                .rate_limit_reset
                .as_deref()
                .map(|r| format!("лимит сбросится в {r}; "))
                .unwrap_or_default();
            CliError::new(Exit::RateLimited, "rate_limited", "превышен лимит запросов API, повторы исчерпаны")
                .retryable(true)
                .with_hint(format!("{when}повторите позже и не запускайте параллельно несколько выгрузок с одним ключом"))
        }
        500..=599 => CliError::general("server_error", parsed.message_or(format!("сервер вернул {status}"))).retryable(true),
        300..=399 => CliError::general(
            "unexpected_redirect",
            format!(
                "сервер вернул редирект {status}{}",
                headers.location.as_deref().map(|l| format!(" на {l}")).unwrap_or_default()
            ),
        )
        .with_hint("проверьте --base-url: API не перенаправляет запросы"),
        _ => CliError::general(format!("http_{status}"), parsed.message_or(format!("неожиданный ответ {status}"))),
    };
    let err = match (&parsed.field, &err.field) {
        (Some(field), None) => err.with_field(field.clone()),
        _ => err,
    };
    let err = match (&parsed.readme, err.hint.clone()) {
        (Some(link), Some(hint)) => err.with_hint(format!("{hint}; документация: {link}")),
        (Some(link), None) => err.with_hint(format!("документация: {link}")),
        _ => err,
    };
    err.with_request_id(headers.request_id.clone())
}

/// `Bearer realm="Doorkeeper", error="invalid_token", error_description="…"` → (error, error_description).
pub fn parse_www_authenticate(header: &str) -> (Option<String>, Option<String>) {
    let mut error = None;
    let mut description = None;
    for (key, value) in auth_params(header) {
        match key.as_str() {
            "error" => error = Some(value),
            "error_description" => description = Some(value),
            _ => {}
        }
    }
    (error, description)
}

#[derive(Debug, Default)]
struct ParsedBody {
    title: Option<String>,
    messages: Vec<String>,
    field: Option<String>,
    readme: Option<String>,
}

impl ParsedBody {
    fn parse(body: &[u8]) -> ParsedBody {
        let Ok(doc) = serde_json::from_slice::<Value>(body) else {
            return ParsedBody::default();
        };
        let error = match doc.get("errors") {
            Some(Value::Array(items)) => items.first().cloned().unwrap_or(Value::Null),
            Some(obj @ Value::Object(_)) => obj.clone(),
            _ => doc.clone(),
        };
        let mut parsed = ParsedBody {
            title: error.get("title").and_then(Value::as_str).map(str::to_string),
            readme: error.pointer("/readme/link").and_then(Value::as_str).map(str::to_string),
            ..ParsedBody::default()
        };
        match error.get("details").or_else(|| error.get("detail")) {
            Some(Value::Object(map)) => {
                for (key, value) in map {
                    let texts: Vec<String> = match value {
                        Value::Array(items) => items.iter().filter_map(|x| x.as_str().map(str::to_string)).collect(),
                        Value::String(s) => vec![s.clone()],
                        _ => Vec::new(),
                    };
                    if key != "message" && parsed.field.is_none() {
                        parsed.field = Some(key.clone());
                    }
                    for text in texts {
                        parsed.messages.push(if key == "message" { text } else { format!("{key}: {text}") });
                    }
                }
            }
            Some(Value::Array(items)) => {
                parsed.messages.extend(items.iter().filter_map(|x| x.as_str().map(str::to_string)))
            }
            Some(Value::String(s)) => parsed.messages.push(s.clone()),
            _ => {}
        }
        parsed
    }

    fn message_or(&self, fallback: impl Into<String>) -> String {
        match (&self.title, self.messages.is_empty()) {
            (Some(title), false) => format!("{title}: {}", self.messages.join("; ")),
            (None, false) => self.messages.join("; "),
            (Some(title), true) => title.clone(),
            (None, true) => fallback.into(),
        }
    }
}

fn bad_request(parsed: &ParsedBody) -> CliError {
    let text = parsed.messages.join("; ");
    if text.contains("Invalid cursor") {
        CliError::general("invalid_cursor", format!("сервер не принял курсор обхода: {text}"))
            .with_hint("курсор устарел или повреждён: удалите файл стейта и начните обход заново")
    } else if text.contains("Cursor was issued for a different query") {
        CliError::general("cursor_mismatch", format!("курсор выдан для другого запроса: {text}"))
            .with_hint("параметры обхода не совпадают с курсором: удалите файл стейта или передайте исходные параметры")
    } else {
        CliError::general("bad_request", parsed.message_or("сервер отклонил запрос"))
    }
}

fn unauthorized(headers: &ResponseHeaders, ctx: &ErrorContext) -> CliError {
    let (code, description) = headers
        .www_authenticate
        .as_deref()
        .map(parse_www_authenticate)
        .unwrap_or((None, None));
    let message = description
        .map(|d| format!("токен отклонён: {d}"))
        .unwrap_or_else(|| "токен отклонён сервером (401)".to_string());
    CliError::new(Exit::Auth, code.unwrap_or_else(|| "unauthorized".into()), message).with_hint(format!(
        "токен из {}, base URL {}: токен отозван, истёк или выпущен для другого окружения",
        ctx.token_source, ctx.base_url
    ))
}

/// Разбивает параметры по запятым вне кавычек; схема (`Bearer`) отбрасывается из первого ключа.
fn auth_params(header: &str) -> Vec<(String, String)> {
    let mut params = Vec::new();
    let mut current = String::new();
    let mut in_quotes = false;
    for c in header.chars() {
        match c {
            '"' => {
                in_quotes = !in_quotes;
                current.push(c);
            }
            ',' if !in_quotes => {
                push_param(&mut params, &current);
                current.clear();
            }
            _ => current.push(c),
        }
    }
    push_param(&mut params, &current);
    params
}

fn push_param(params: &mut Vec<(String, String)>, part: &str) {
    if let Some((key, value)) = part.split_once('=') {
        let key = key.trim().rsplit(' ').next().unwrap_or_default().to_string();
        params.push((key, value.trim().trim_matches('"').to_string()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx() -> ErrorContext {
        ErrorContext { token_source: "APLAUT_ACCESS_TOKEN".into(), base_url: "https://api.staging.example/v4".into() }
    }

    fn headers() -> ResponseHeaders {
        ResponseHeaders { request_id: Some("req-42".into()), ..ResponseHeaders::default() }
    }

    #[test]
    fn observed_422_gives_field_and_readme_hint() {
        let body = br#"{"errors":{"status":422,"title":"Invalid query params","details":{"filter":["unknown field 'foo', allowed: brand_id, category_id"]},"readme":{"link":"https://aplaut.com/docs/api-references/platform/"}}}"#;
        let err = from_response(422, &headers(), body, &ctx());
        assert_eq!((err.code.as_str(), err.exit), ("validation_failed", Exit::General));
        assert_eq!(err.field.as_deref(), Some("filter"));
        assert!(err.message.contains("unknown field 'foo'"), "{}", err.message);
        assert!(err.hint.unwrap().contains("https://aplaut.com/docs/api-references/platform/"));
        assert_eq!(err.request_id.as_deref(), Some("req-42"));
        assert!(!err.retryable);
    }

    #[test]
    fn observed_400_cursor_errors_have_own_codes() {
        let invalid = br#"{"errors":{"status":400,"title":"Bad request","details":{"message":"Invalid cursor"}}}"#;
        let mismatch = br#"{"errors":{"status":400,"title":"Bad request","details":{"message":"Cursor was issued for a different query"}}}"#;
        assert_eq!(from_response(400, &headers(), invalid, &ctx()).code, "invalid_cursor");
        let err = from_response(400, &headers(), mismatch, &ctx());
        assert_eq!(err.code, "cursor_mismatch");
        assert!(err.hint.unwrap().contains("стейт"));
    }

    #[test]
    fn spec_shape_is_also_understood() {
        let body = br#"{"status":"422","title":"Validation failed","details":["rating must be 1..5"]}"#;
        let err = from_response(422, &headers(), body, &ctx());
        assert!(err.message.contains("rating must be 1..5"));
        let array = br#"{"errors":[{"status":"404","title":"Not found"}]}"#;
        let err = from_response(404, &headers(), array, &ctx());
        assert_eq!((err.code.as_str(), err.exit), ("not_found", Exit::NotFound));
    }

    #[test]
    fn observed_401_uses_www_authenticate_and_names_token_source() {
        let h = ResponseHeaders {
            www_authenticate: Some(r#"Bearer realm="Doorkeeper", error="invalid_token", error_description="The access token is invalid""#.into()),
            ..headers()
        };
        let err = from_response(401, &h, b"", &ctx());
        assert_eq!((err.code.as_str(), err.exit), ("invalid_token", Exit::Auth));
        assert!(err.message.contains("The access token is invalid"));
        let hint = err.hint.unwrap();
        assert!(hint.contains("APLAUT_ACCESS_TOKEN") && hint.contains("https://api.staging.example/v4"));
    }

    #[test]
    fn observed_401_without_error_param_is_still_auth_error() {
        // Так стейджинг отвечает на строку, не похожую на токен (2026-09-23).
        let h = ResponseHeaders {
            www_authenticate: Some(r#"Token realm="Aplaut Platform API v4""#.into()),
            ..headers()
        };
        let err = from_response(401, &h, b"HTTP Token: Access denied.\n", &ctx());
        assert_eq!((err.code.as_str(), err.exit), ("unauthorized", Exit::Auth));
        assert_eq!(err.request_id.as_deref(), Some("req-42"));
    }

    #[test]
    fn www_authenticate_quoted_commas() {
        let (e, d) = parse_www_authenticate(r#"Bearer realm="x", error="invalid_token", error_description="expired, sorry""#);
        assert_eq!((e.as_deref(), d.as_deref()), (Some("invalid_token"), Some("expired, sorry")));
    }

    #[test]
    fn non_json_and_status_classes() {
        assert_eq!(from_response(503, &headers(), b"<html>", &ctx()).code, "server_error");
        assert!(from_response(503, &headers(), b"", &ctx()).retryable);
        let err = from_response(429, &ResponseHeaders { rate_limit_reset: Some("2026-09-23 10:12:00 +0000".into()), ..headers() }, b"Throttled\n", &ctx());
        assert_eq!((err.code.as_str(), err.exit), ("rate_limited", Exit::RateLimited));
        assert!(err.hint.unwrap().contains("2026-09-23 10:12:00 +0000"));
        let redirect = from_response(302, &ResponseHeaders { location: Some("https://x/login".into()), ..headers() }, b"", &ctx());
        assert_eq!(redirect.code, "unexpected_redirect");
        assert_eq!(from_response(403, &headers(), b"", &ctx()).exit, Exit::Auth);
    }
}
