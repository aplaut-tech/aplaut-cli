//! `aplaut mcp` (спека mcp-server §1): проверка глобальных флагов, флаги для дочерних вызовов и
//! `instructions` для модели; сам сервер — в `crate::mcp`.

use std::ffi::OsString;
use std::path::Path;

use serde_json::json;

use super::{base_url_for, Ctx, Outcome};
use crate::auth;
use crate::cli::{GlobalArgs, McpArgs};
use crate::error::CliError;
use crate::mcp::{self, ServerSetup};

const INSTRUCTIONS_HEAD: &str = "Aplaut Platform API: отзывы, товары, вопросы.";

const INSTRUCTIONS_BODY: &str = "\
Ответ инструмента: первый текстовый блок — JSON-конверт (ok, command, result или error, warnings), \
второй — данные, если они есть (jsonl: запись со связями из include одной строкой). При ok=false \
смотрите error.code, error.hint и error.retryable; retry_after — сколько секунд ждать. Каталог кодов: \
https://github.com/aplaut-tech/aplaut-cli/blob/main/docs/automation.md
Без filter сервер отдаёт только последние 30 дней (предупреждение default_filter). Выгрузка без \
output_file — не больше 100 записей за вызов (max_records, по умолчанию 20); со state повторный вызов \
отдаёт следующую порцию. Больше — в output_file.";

pub fn run(args: &McpArgs, ctx: &Ctx) -> Result<Outcome, CliError> {
    check_stdin_free(&ctx.global)?;
    let setup = ServerSetup {
        allow_writes: args.allow_writes,
        forwarded: forwarded_flags(&ctx.global),
        instructions: instructions(args.allow_writes, target(ctx)),
    };
    mcp::serve(setup)?;
    // stdout принадлежал протоколу — итог `--json` идёт в stderr, как у scroll.
    Ok(Outcome::stderr(
        json!({ "allow_writes": args.allow_writes }),
    ))
}

/// stdin — канал протокола: токен оттуда не прочитать ни серверу, ни дочерним вызовам.
fn check_stdin_free(global: &GlobalArgs) -> Result<(), CliError> {
    let token_file_stdin = global.token_file.as_deref() == Some(Path::new("-"));
    if !global.token_stdin && !token_file_stdin {
        return Ok(());
    }
    let field = if global.token_stdin {
        "token_stdin"
    } else {
        "token_file"
    };
    Err(CliError::usage(
        "usage",
        "aplaut mcp: stdin занят протоколом MCP, токен из stdin не прочитать",
    )
    .with_field(field)
    .with_hint(
        "сохраните токен: aplaut auth login; или --token-file PATH, или APLAUT_ACCESS_TOKEN",
    ))
}

/// Глобальные флаги сервера — каждому дочернему вызову, одним токеном (M6, M9): агент их не меняет.
pub fn forwarded_flags(global: &GlobalArgs) -> Vec<OsString> {
    let mut flags = Vec::new();
    if let Some(profile) = &global.profile {
        flags.push(OsString::from(format!("--profile={profile}")));
    }
    if let Some(url) = &global.base_url {
        flags.push(OsString::from(format!("--base-url={url}")));
    }
    if let Some(path) = &global.token_file {
        let mut flag = OsString::from("--token-file=");
        flag.push(path);
        flags.push(flag);
    }
    flags.push(OsString::from(format!("--timeout={}", global.timeout)));
    flags.push(OsString::from(format!(
        "--max-retries={}",
        global.max_retries
    )));
    if global.verbose {
        flags.push(OsString::from("--verbose"));
    }
    flags
}

/// Профиль и base URL — как их увидит дочерний вызов.
fn target(ctx: &Ctx) -> Result<(String, String), CliError> {
    let profile = auth::active_profile(ctx.global.profile.as_deref(), &ctx.env)?;
    let base_url = base_url_for(ctx, &profile)?;
    Ok((profile, base_url))
}

/// `instructions` из ответа на `initialize` — их видит модель (§1).
fn instructions(allow_writes: bool, target: Result<(String, String), CliError>) -> String {
    let target = match target {
        Ok((profile, base_url)) => format!("Профиль {profile}, base URL {base_url}."),
        Err(err) => format!(
            "Профиль не определён: {} — вызовы вернут ошибку с подсказкой.",
            err.message
        ),
    };
    let mode = if allow_writes {
        "Запись включена: reviews_create, reviews_comment, products_create, products_update; \
         проверяйте запрос параметром dry_run, прежде чем отправлять."
    } else {
        "Запись выключена (сервер запущен без --allow-writes): только чтение и выгрузка."
    };
    format!("{INSTRUCTIONS_HEAD} {target} {mode}\n{INSTRUCTIONS_BODY}")
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use super::*;
    use crate::cli::Cli;

    fn global(args: &[&str]) -> GlobalArgs {
        let mut full = vec!["aplaut", "mcp"];
        full.extend(args);
        Cli::try_parse_from(full).unwrap().global
    }

    fn texts(flags: &[OsString]) -> Vec<String> {
        flags
            .iter()
            .map(|f| f.to_string_lossy().into_owned())
            .collect()
    }

    #[test]
    fn forwards_server_flags_as_single_tokens() {
        let flags = forwarded_flags(&global(&[
            "--profile",
            "prod",
            "--base-url",
            "https://api.example.test/v4",
            "--token-file",
            "tok.txt",
            "--timeout",
            "5",
            "--verbose",
            "--quiet",
            "--no-color",
        ]));
        assert_eq!(
            texts(&flags),
            [
                "--profile=prod",
                "--base-url=https://api.example.test/v4",
                "--token-file=tok.txt",
                "--timeout=5",
                "--max-retries=6",
                "--verbose",
            ],
            "--quiet и --no-color к протоколу отношения не имеют"
        );
    }

    #[test]
    fn instructions_name_target_and_mode() {
        let ok = instructions(true, Ok(("staging".into(), "https://x.test/v4".into())));
        assert!(
            ok.contains("Профиль staging, base URL https://x.test/v4."),
            "{ok}"
        );
        assert!(ok.contains("Запись включена"), "{ok}");
        let err = CliError::usage("invalid_profile", "недопустимое имя профиля «a b»");
        let text = instructions(false, Err(err));
        assert!(
            text.contains("Профиль не определён: недопустимое имя профиля «a b»"),
            "{text}"
        );
        assert!(text.contains("Запись выключена"), "{text}");
    }
}
