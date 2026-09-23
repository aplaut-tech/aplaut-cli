//! Конверт `--json` и ошибок без TTY — публичный контракт (спека agent mode §1): одна строка
//! JSON; ключи своего вида присутствуют всегда, отсутствующее значение — `null`.

use serde::Serialize;
use serde_json::Value;

use crate::error::CliError;
use crate::term::Warning;

#[derive(Serialize)]
struct Success<'a> {
    ok: bool,
    command: &'a str,
    cli_version: &'a str,
    dry_run: bool,
    result: &'a Value,
    warnings: &'a [Warning],
}

#[derive(Serialize)]
struct Failure<'a> {
    ok: bool,
    command: &'a str,
    cli_version: &'a str,
    dry_run: bool,
    error: &'a CliError,
    warnings: &'a [Warning],
}

pub fn success(command: &str, result: &Value, dry_run: bool, warnings: &[Warning]) -> String {
    serde_json::to_string(&Success {
        ok: true,
        command,
        cli_version: env!("CARGO_PKG_VERSION"),
        dry_run,
        result,
        warnings,
    })
    .expect("конверт сериализуется всегда")
}

pub fn failure(err: &CliError, command: &str, dry_run: bool, warnings: &[Warning]) -> String {
    serde_json::to_string(&Failure {
        ok: false,
        command,
        cli_version: env!("CARGO_PKG_VERSION"),
        dry_run,
        error: err,
        warnings,
    })
    .expect("конверт сериализуется всегда")
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::time::Duration;

    use super::*;
    use crate::error::Exit;

    fn keys(v: &Value) -> BTreeSet<&str> {
        v.as_object().unwrap().keys().map(String::as_str).collect()
    }

    #[test]
    fn failure_has_fixed_shape_with_retry_after_and_warnings() {
        let err = CliError::new(Exit::RateLimited, "rate_limited", "превышен лимит")
            .retryable(true)
            .with_retry_after(Some(Duration::from_millis(29_200)));
        let warnings = [Warning {
            code: "default_filter".into(),
            message: "30 дней".into(),
        }];
        let v: Value =
            serde_json::from_str(&failure(&err, "reviews.scroll", false, &warnings)).unwrap();
        assert_eq!(
            keys(&v),
            BTreeSet::from([
                "ok",
                "command",
                "cli_version",
                "dry_run",
                "error",
                "warnings"
            ])
        );
        assert_eq!(
            keys(&v["error"]),
            BTreeSet::from([
                "code",
                "message",
                "field",
                "retryable",
                "retry_after",
                "hint",
                "request_id"
            ])
        );
        assert_eq!(v["ok"], false);
        assert_eq!(v["command"], "reviews.scroll");
        assert_eq!(v["cli_version"], env!("CARGO_PKG_VERSION"));
        assert_eq!(v["error"]["retry_after"], 30, "секунды вверх");
        assert_eq!(v["error"]["field"], Value::Null);
        assert_eq!(v["warnings"][0]["code"], "default_filter");
    }

    #[test]
    fn success_has_fixed_shape() {
        let result = serde_json::json!({"profiles": []});
        let v: Value = serde_json::from_str(&success("profile.list", &result, true, &[])).unwrap();
        assert_eq!(
            keys(&v),
            BTreeSet::from([
                "ok",
                "command",
                "cli_version",
                "dry_run",
                "result",
                "warnings"
            ])
        );
        assert_eq!(
            (v["ok"].as_bool(), v["dry_run"].as_bool()),
            (Some(true), Some(true))
        );
        assert_eq!(v["result"], result);
        assert_eq!(v["warnings"], serde_json::json!([]));
    }
}
