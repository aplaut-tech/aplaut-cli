//! Токен — отдельный тип: у него нет `Display`, а `Debug` маскирует значение.
//! Вывод CLI уезжает в логи и чаты с агентами, поэтому случайная печать недопустима.

use std::fmt;

use crate::error::CliError;

pub const MASK: &str = "***";

#[derive(Clone, PartialEq, Eq)]
pub struct Secret(String);

impl Secret {
    pub fn new(value: impl Into<String>) -> Self {
        Secret(value.into())
    }

    /// Явный доступ — только там, где токен уходит в заголовок или в файл credentials.
    pub fn expose(&self) -> &str {
        &self.0
    }

    /// Вычищает токен из произвольного текста (сообщения ureq, эхо сервера).
    pub fn redact(&self, text: &str) -> String {
        if self.0.is_empty() {
            text.to_string()
        } else {
            text.replace(&self.0, MASK)
        }
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Secret(***)")
    }
}

/// Пробел или управляющий символ внутри означает мусор из файла или буфера обмена
/// и сломал бы заголовок `Authorization`; само значение в ошибку не попадает.
pub fn parse_token(raw: &str) -> Result<Secret, CliError> {
    let token = raw.trim();
    if token.is_empty() {
        return Err(CliError::usage("empty_token", "токен пустой"));
    }
    if token.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err(CliError::usage(
            "invalid_token_format",
            "токен содержит пробелы или управляющие символы",
        )
        .with_hint("проверьте, что в файле или stdin только сам токен"));
    }
    Ok(Secret::new(token))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_never_shows_value() {
        let s = Secret::new("tok-123");
        assert_eq!(format!("{s:?}"), "Secret(***)");
    }

    #[test]
    fn redact_replaces_every_occurrence() {
        let s = Secret::new("tok-123");
        assert_eq!(s.redact("a tok-123 b tok-123"), "a *** b ***");
    }

    #[test]
    fn parse_token_trims_newline_and_rejects_inner_whitespace() {
        assert_eq!(parse_token("abc\n").unwrap().expose(), "abc");
        assert_eq!(parse_token("  ").unwrap_err().code, "empty_token");
        let err = parse_token("ab c").unwrap_err();
        assert_eq!(err.code, "invalid_token_format");
        assert!(!err.message.contains("ab c"));
    }
}
