//! `auth login` / `auth logout`. На сервере токен проверит будущий `auth status`.

use std::io::{self, IsTerminal};
use std::path::Path;

use serde::Serialize;

use super::{path_text, Ctx, Outcome};
use crate::auth::{self as resolve, StdinSource};
use crate::cli::AuthVerb;
use crate::config::{self, Paths, ProfileCredentials};
use crate::error::CliError;
use crate::secret::{parse_token, Secret};

pub fn run(verb: AuthVerb, ctx: &Ctx) -> Result<Outcome, CliError> {
    match verb {
        AuthVerb::Login => login(ctx),
        AuthVerb::Logout => logout(ctx),
    }
}

/// Откуда взят токен — в `result` вместо самого токена.
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "lowercase")]
enum TokenSource {
    Prompt,
    Stdin,
    File,
}

#[derive(Serialize)]
struct LoginResult {
    profile: String,
    /// Токен не выводится никогда (спека agent mode §2).
    token: &'static str,
    token_source: TokenSource,
    replaced: bool,
    credentials_path: String,
}

fn login(ctx: &Ctx) -> Result<Outcome, CliError> {
    let profile = resolve::active_profile(ctx.global.profile.as_deref(), &ctx.env)?;
    let base_url = ctx
        .global
        .base_url
        .as_deref()
        .map(resolve::validate_base_url)
        .transpose()?;
    let (token, token_source) = read_token(ctx)?;
    let paths = Paths::resolve(&ctx.env)?;
    let mut credentials = config::load_credentials(&paths, &ctx.reporter)?;
    let replaced = credentials.profiles.contains_key(&profile);
    credentials.profiles.insert(
        profile.clone(),
        ProfileCredentials {
            access_token: token.expose().to_string(),
        },
    );
    let mut cfg = config::load_config(&paths)?;
    let entry = cfg.profiles.entry(profile.clone()).or_default();
    if base_url.is_some() {
        entry.base_url = base_url.clone();
    }
    config::save_credentials(&paths, &credentials)?;
    config::save_config(&paths, &cfg)?;
    ctx.reporter.info(&format!(
        "Токен сохранён в профиль «{profile}» ({}).",
        paths.credentials.display()
    ));
    if let Some(url) = base_url {
        ctx.reporter.info(&format!("Base URL профиля: {url}"));
    }
    ctx.reporter
        .info("Токен не проверялся на сервере: проверка появится вместе с `aplaut auth status`.");
    Ok(Outcome::stdout(LoginResult {
        profile,
        token: "***",
        token_source,
        replaced,
        credentials_path: path_text(&paths.credentials),
    }))
}

fn read_token(ctx: &Ctx) -> Result<(Secret, TokenSource), CliError> {
    let stdin = io::stdin();
    let is_terminal = stdin.is_terminal();
    let mut lock = stdin.lock();
    let mut source = StdinSource {
        is_terminal,
        reader: &mut lock,
    };
    if ctx.global.token_stdin {
        return resolve::read_stdin_token(&mut source).map(|t| (t, TokenSource::Stdin));
    }
    if let Some(path) = ctx.global.token_file.as_deref() {
        return if path == Path::new("-") {
            resolve::read_stdin_token(&mut source).map(|t| (t, TokenSource::Stdin))
        } else {
            resolve::read_token_file(path).map(|t| (t, TokenSource::File))
        };
    }
    // Никогда не требуем промпт (clig): без TTY, с --no-input или --json — только флаги.
    if !ctx.can_prompt() {
        return Err(CliError::usage("token_required", "нужен токен").with_hint(
            "передайте его через --token-stdin или --token-file; ввод с клавиатуры — только в терминале без --no-input и --json",
        ));
    }
    let raw = rpassword::prompt_password("Токен Platform API (ввод скрыт): ")
        .map_err(|e| CliError::io("чтение токена", &e))?;
    parse_token(&raw).map(|t| (t, TokenSource::Prompt))
}

fn logout(ctx: &Ctx) -> Result<Outcome, CliError> {
    let profile = resolve::active_profile(ctx.global.profile.as_deref(), &ctx.env)?;
    let paths = Paths::resolve(&ctx.env)?;
    let mut credentials = config::load_credentials(&paths, &ctx.reporter)?;
    if credentials.profiles.remove(&profile).is_none() {
        ctx.reporter.info(&format!(
            "В профиле «{profile}» нет сохранённого токена — удалять нечего."
        ));
        return Ok(Outcome::stdout(
            serde_json::json!({"profile": profile, "removed": false}),
        ));
    }
    config::save_credentials(&paths, &credentials)?;
    ctx.reporter.info(&format!(
        "Токен профиля «{profile}» удалён из {}.",
        paths.credentials.display()
    ));
    Ok(Outcome::stdout(
        serde_json::json!({"profile": profile, "removed": true}),
    ))
}
