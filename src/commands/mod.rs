//! Клей между деревом clap и алгоритмами: сборка зависимостей и сообщения пользователю.

pub mod auth;
pub mod mcp;
pub mod products;
pub mod profile;
pub mod records;
pub mod reviews;
pub mod self_update;
pub mod updates;
pub mod writes;

use std::io::{self, IsTerminal};
use std::path::Path;
use std::rc::Rc;
use std::time::Duration;

use serde::Serialize;

use crate::api_error::ErrorContext;
use crate::auth::{EnvSnapshot, StdinSource, TokenFlags, DEFAULT_BASE_URL};
use crate::cli::{
    AuthVerb, Command, CommentArgs, CreateProductArgs, CreateReviewArgs, GlobalArgs, ProductsVerb,
    ProfileVerb, RecordsVerb, ReviewsVerb, SelfVerb, UpdateInput, UpdateProductArgs,
    UpdateReviewArgs,
};
use crate::clock::Clock;
use crate::config::{self, Paths};
use crate::error::CliError;
use crate::http::{ApiClient, HttpSettings};
use crate::resources;
use crate::term::Reporter;

pub struct Ctx {
    pub global: GlobalArgs,
    pub env: EnvSnapshot,
    pub reporter: Rc<Reporter>,
    pub clock: Rc<dyn Clock>,
}

/// Текстовый итог под `--dry-run` начинается одинаково у всех команд (спека agent mode §5).
pub const DRY_RUN_PREFIX: &str = "Пробный запуск, ничего не изменено:";

/// Куда писать конверт `--json`: у scroll stdout занят данными.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Channel {
    Stdout,
    Stderr,
}

/// Итог команды для `--json` (спека agent mode §2). В текстовом режиме команды печатают сами.
#[derive(Debug, Clone, PartialEq)]
pub struct Outcome {
    pub result: serde_json::Value,
    pub channel: Channel,
    pub dry_run: bool,
}

impl Outcome {
    pub fn stdout(result: impl Serialize) -> Self {
        Self::new(result, Channel::Stdout)
    }

    pub fn stderr(result: impl Serialize) -> Self {
        Self::new(result, Channel::Stderr)
    }

    pub fn with_dry_run(self, dry_run: bool) -> Self {
        Outcome { dry_run, ..self }
    }

    fn new(result: impl Serialize, channel: Channel) -> Self {
        Outcome {
            result: serde_json::to_value(result).expect("итог сериализуется"),
            channel,
            dry_run: false,
        }
    }
}

/// Пути в `result` — абсолютные: агент мог запустить команду из другого каталога.
pub fn path_text(path: &Path) -> String {
    std::path::absolute(path)
        .unwrap_or_else(|_| path.to_path_buf())
        .display()
        .to_string()
}

impl Ctx {
    pub fn json(&self) -> bool {
        self.global.json
    }

    /// Спрашивать человека можно, только если это возможно и разрешено (см. [`can_prompt`]).
    pub fn can_prompt(&self) -> bool {
        can_prompt(
            io::stdin().is_terminal(),
            self.global.no_input,
            self.global.json,
        )
    }
}

/// clig: вопрос — только в терминале и без --no-input; --json тоже запрещает вопросы:
/// агент в псевдотерминале не должен повиснуть на них.
pub fn can_prompt(stdin_tty: bool, no_input: bool, json: bool) -> bool {
    stdin_tty && !no_input && !json
}

pub fn dispatch(command: Command, ctx: &Ctx) -> Result<Outcome, CliError> {
    match command {
        Command::Reviews { verb } => reviews::run(verb, ctx),
        Command::Products { verb } => products::run(verb, ctx),
        Command::Questions { verb } => records::run(&resources::QUESTIONS, verb, ctx),
        Command::Auth { verb } => auth::run(verb, ctx),
        Command::Profile { verb } => profile::run(verb, ctx),
        Command::SelfCmd { verb } => self_update::run(verb, ctx),
        Command::Mcp(args) => mcp::run(&args, ctx),
    }
}

/// Запуск с `--dry-run` (`-n`) — для конверта ошибки: агент должен знать, что упал план, а не
/// настоящий запуск.
pub fn is_dry_run(command: &Command) -> bool {
    match command {
        Command::Profile {
            verb: ProfileVerb::Set { dry, .. } | ProfileVerb::Delete { dry, .. },
        } => dry.dry_run,
        Command::Auth {
            verb: AuthVerb::Login(dry) | AuthVerb::Logout(dry),
        } => dry.dry_run,
        Command::SelfCmd {
            verb: SelfVerb::Update(dry),
        } => dry.dry_run,
        Command::Reviews {
            verb: ReviewsVerb::Create(CreateReviewArgs { dry, .. }),
        }
        | Command::Reviews {
            verb: ReviewsVerb::Comment(CommentArgs { dry, .. }),
        }
        | Command::Reviews {
            verb:
                ReviewsVerb::Update(UpdateReviewArgs {
                    input: UpdateInput { dry, .. },
                    ..
                }),
        }
        | Command::Products {
            verb: ProductsVerb::Create(CreateProductArgs { dry, .. }),
        }
        | Command::Products {
            verb: ProductsVerb::Update(UpdateProductArgs { dry, .. }),
        } => dry.dry_run,
        _ => false,
    }
}

/// Имя для конверта ошибки: `reviews.scroll`, `auth.login`.
pub fn command_name(command: &Command) -> String {
    let (resource, verb) = match command {
        Command::Mcp(_) => return "mcp".to_string(),
        Command::Reviews { verb } => ("reviews", reviews_verb(verb)),
        Command::Products { verb } => ("products", products_verb(verb)),
        Command::Questions { verb } => ("questions", records_verb(verb)),
        Command::Auth { verb } => (
            "auth",
            match verb {
                AuthVerb::Login(_) => "login",
                AuthVerb::Logout(_) => "logout",
            },
        ),
        Command::Profile { verb } => (
            "profile",
            match verb {
                ProfileVerb::List => "list",
                ProfileVerb::Get { .. } => "get",
                ProfileVerb::Set { .. } => "set",
                ProfileVerb::Delete { .. } => "delete",
                ProfileVerb::Edit => "edit",
            },
        ),
        Command::SelfCmd { verb } => (
            "self",
            match verb {
                SelfVerb::Update(_) => "update",
            },
        ),
    };
    format!("{resource}.{verb}")
}

fn products_verb(verb: &ProductsVerb) -> &'static str {
    match verb {
        ProductsVerb::Records(verb) => records_verb(verb),
        ProductsVerb::Create(_) => "create",
        ProductsVerb::Update(_) => "update",
    }
}

fn reviews_verb(verb: &ReviewsVerb) -> &'static str {
    match verb {
        ReviewsVerb::Records(verb) => records_verb(verb),
        ReviewsVerb::Create(_) => "create",
        ReviewsVerb::Comment(_) => "comment",
        ReviewsVerb::Update(_) => "update",
    }
}

fn records_verb(verb: &RecordsVerb) -> &'static str {
    match verb {
        RecordsVerb::Scroll(_) => "scroll",
        RecordsVerb::Get(_) => "get",
    }
}

pub fn connect(ctx: &Ctx) -> Result<ApiClient, CliError> {
    // Каталог конфигурации и credentials нужны только для профиля: запуск с токеном из флага
    // или env не должен зависеть ни от HOME, ни от исправности чужих файлов.
    let mut load_credentials =
        || config::load_credentials(&Paths::resolve(&ctx.env)?, &ctx.reporter);
    let flags = TokenFlags {
        token_stdin: ctx.global.token_stdin,
        token_file: ctx.global.token_file.as_deref(),
        profile: ctx.global.profile.as_deref(),
    };
    let stdin = io::stdin();
    let is_terminal = stdin.is_terminal();
    let mut lock = stdin.lock();
    let mut source = StdinSource {
        is_terminal,
        reader: &mut lock,
    };
    let (token, token_source) =
        crate::auth::resolve_token(&flags, &ctx.env, &mut load_credentials, &mut source)?;
    let profile = crate::auth::active_profile(ctx.global.profile.as_deref(), &ctx.env)?;
    let profile_flag = ctx.global.profile.is_some();
    if profile_flag && ctx.global.base_url.is_none() && ctx.env.base_url.is_some() {
        ctx.reporter.warn(
            "base_url_env_ignored",
            &format!(
                "APLAUT_BASE_URL не используется: при явном --profile {profile} берётся base URL профиля"
            ),
        );
    }
    let base_url = base_url_for(ctx, &profile)?;
    if base_url != DEFAULT_BASE_URL {
        ctx.reporter.debug(&format!("base URL: {base_url}"));
    }
    ctx.reporter
        .debug(&format!("токен из {}", token_source.describe()));
    let settings = HttpSettings {
        base_url: base_url.clone(),
        timeout: Duration::from_secs(ctx.global.timeout),
        max_retries: ctx.global.max_retries,
    };
    let error_context = ErrorContext {
        token_source: token_source.describe(),
        base_url,
    };
    Ok(ApiClient::new(
        settings,
        token,
        error_context,
        ctx.clock.clone(),
        ctx.reporter.clone(),
    ))
}

/// Base URL — как у `connect`: `--base-url`, `APLAUT_BASE_URL` (без явного `--profile`), профиль,
/// по умолчанию. Нужен и серверу MCP — для `instructions`.
pub fn base_url_for(ctx: &Ctx, profile: &str) -> Result<String, CliError> {
    let mut load_profile = || -> Result<_, CliError> {
        let config = config::load_config(&Paths::resolve(&ctx.env)?)?;
        Ok(config.profiles.get(profile).cloned())
    };
    crate::auth::resolve_base_url(
        ctx.global.base_url.as_deref(),
        &ctx.env,
        ctx.global.profile.is_some(),
        &mut load_profile,
    )
}

/// Идентификатор уходит сегментом пути. Пустой (незаданная переменная шелла) превратил бы
/// `GET /reviews/{id}` в запрос списка. Точку сервер считает началом формата и отрезает всё после
/// неё, даже закодированную, а `/` (и `%2F`) не маршрутизирует (стейджинг, 2026-09-24; §15
/// дизайна среза 1): запрос ушёл бы к чужой записи или никуда.
pub fn check_id(id: &str, field: &str) -> Result<(), CliError> {
    let (problem, hint) = if id.trim().is_empty() {
        (
            "пустой идентификатор",
            "передайте внутренний id или external_id записи",
        )
    } else if id.contains('.') {
        (
            "API отрезает от идентификатора всё после точки — запрос ушёл бы к другой записи",
            "передайте внутренний id записи (поле id в выгрузке scroll)",
        )
    } else if id.contains('/') {
        (
            "API не находит записи с «/» в идентификаторе",
            "передайте внутренний id записи (поле id в выгрузке scroll)",
        )
    } else {
        return Ok(());
    };
    Err(
        CliError::usage("invalid_id", format!("идентификатор «{id}»: {problem}"))
            .with_field(field)
            .with_hint(hint),
    )
}

#[cfg(test)]
mod tests {
    use super::can_prompt;

    #[test]
    fn prompts_only_in_a_terminal_without_no_input_and_json() {
        assert!(can_prompt(true, false, false));
        for (tty, no_input, json) in [
            (false, false, false),
            (true, true, false),
            (true, false, true),
        ] {
            assert!(!can_prompt(tty, no_input, json), "{tty} {no_input} {json}");
        }
    }
}
