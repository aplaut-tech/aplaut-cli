//! Модель ошибок и коды выхода.
//!
//! Всё — локальные проверки, сетевые сбои, ответы API — приводится к `CliError`, чтобы
//! агент получал один формат с `retryable` и `hint`: без них LLM повторяет безнадёжный запрос.

use std::fmt;

use serde::Serialize;

/// Коды выхода — публичный контракт (дизайн §8), их используют cron и ETL.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Exit {
    General,
    Usage,
    Auth,
    /// Часть данных уже в stdout, выгрузка не завершена: приёмник должен откатить загрузку.
    Partial,
    NotFound,
    RateLimited,
    Policy,
}

impl Exit {
    pub fn code(self) -> u8 {
        match self {
            Exit::General => 1,
            Exit::Usage => 2,
            Exit::Auth => 3,
            Exit::Partial => 4,
            Exit::NotFound => 5,
            Exit::RateLimited => 7,
            Exit::Policy => 8,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CliError {
    pub code: String,
    pub message: String,
    pub field: Option<String>,
    pub retryable: bool,
    pub hint: Option<String>,
    pub request_id: Option<String>,
    #[serde(skip)]
    pub exit: Exit,
}

impl CliError {
    pub fn new(exit: Exit, code: impl Into<String>, message: impl Into<String>) -> Self {
        CliError {
            code: code.into(),
            message: message.into(),
            field: None,
            retryable: false,
            hint: None,
            request_id: None,
            exit,
        }
    }

    pub fn usage(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self::new(Exit::Usage, code, message)
    }

    pub fn general(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self::new(Exit::General, code, message)
    }

    pub fn io(context: &str, err: &std::io::Error) -> Self {
        Self::general("io_error", format!("{context}: {err}"))
    }

    pub fn with_hint(mut self, hint: impl Into<String>) -> Self {
        self.hint = Some(hint.into());
        self
    }

    pub fn with_field(mut self, field: impl Into<String>) -> Self {
        self.field = Some(field.into());
        self
    }

    pub fn with_request_id(mut self, request_id: Option<String>) -> Self {
        if request_id.is_some() {
            self.request_id = request_id;
        }
        self
    }

    pub fn retryable(mut self, retryable: bool) -> Self {
        self.retryable = retryable;
        self
    }

    /// Причина сохраняется, меняется только код выхода: важно, что данные уже ушли.
    pub fn into_partial(mut self) -> Self {
        self.exit = Exit::Partial;
        self
    }
}

impl fmt::Display for CliError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for CliError {}

#[derive(Serialize)]
struct Envelope<'a> {
    ok: bool,
    command: &'a str,
    cli_version: &'a str,
    error: &'a CliError,
}

/// Одна строка JSON — последняя в stderr; её разбирают агент и обвязки cron.
pub fn render_json(err: &CliError, command: &str) -> String {
    serde_json::to_string(&Envelope {
        ok: false,
        command,
        cli_version: env!("CARGO_PKG_VERSION"),
        error: err,
    })
    .expect("CliError сериализуется всегда")
}

/// Для человека: сообщение, детали, подсказка последней строкой (clig: важное — в конце).
pub fn render_human(err: &CliError, color: bool) -> String {
    let label = if color {
        "\x1b[31merror\x1b[0m"
    } else {
        "error"
    };
    let mut text = format!("{label}: {}\n", err.message);
    if let Some(field) = &err.field {
        text.push_str(&format!("  параметр: {field}\n"));
    }
    if let Some(id) = &err.request_id {
        text.push_str(&format!("  request id: {id}\n"));
    }
    if let Some(hint) = &err.hint {
        text.push_str(&format!("hint: {hint}\n"));
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exit_codes_match_public_contract() {
        let codes: Vec<u8> = [
            Exit::General,
            Exit::Usage,
            Exit::Auth,
            Exit::Partial,
            Exit::NotFound,
            Exit::RateLimited,
            Exit::Policy,
        ]
        .iter()
        .map(|e| e.code())
        .collect();
        assert_eq!(codes, vec![1, 2, 3, 4, 5, 7, 8]);
    }

    #[test]
    fn json_envelope_has_fixed_shape() {
        let err = CliError::usage("invalid_include", "неизвестный include «nope»")
            .with_field("include")
            .with_hint("допустимые значения: author, product");
        let v: serde_json::Value =
            serde_json::from_str(&render_json(&err, "reviews.scroll")).unwrap();
        assert_eq!(v["ok"], false);
        assert_eq!(v["command"], "reviews.scroll");
        assert_eq!(v["cli_version"], env!("CARGO_PKG_VERSION"));
        assert_eq!(v["error"]["code"], "invalid_include");
        assert_eq!(v["error"]["field"], "include");
        assert_eq!(v["error"]["retryable"], false);
        assert_eq!(v["error"]["request_id"], serde_json::Value::Null);
        assert!(v["error"].get("exit").is_none());
    }

    #[test]
    fn human_rendering_puts_hint_last() {
        let err = CliError::general("invalid_cursor", "сервер не принял курсор")
            .with_request_id(Some("req-1".into()))
            .with_hint("удалите файл стейта");
        let text = render_human(&err, false);
        assert!(text.starts_with("error: сервер не принял курсор\n"));
        assert!(text.contains("request id: req-1"));
        assert_eq!(text.lines().last(), Some("hint: удалите файл стейта"));
    }

    #[test]
    fn partial_keeps_original_cause() {
        let err = CliError::general("network_error", "обрыв")
            .retryable(true)
            .into_partial();
        assert_eq!(err.exit, Exit::Partial);
        assert_eq!(err.code, "network_error");
        assert!(err.retryable);
    }
}
