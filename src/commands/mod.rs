//! Клей между деревом clap и алгоритмами: сборка зависимостей и сообщения пользователю.

pub mod auth;
pub mod profile;
pub mod records;

use std::io::{self, IsTerminal};
use std::path::Path;
use std::rc::Rc;

use serde::Serialize;

use crate::auth::EnvSnapshot;
use crate::cli::{AuthVerb, Command, GlobalArgs, ProfileVerb, RecordsVerb};
use crate::clock::Clock;
use crate::error::CliError;
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
        Command::Reviews { verb } => records::run(&resources::REVIEWS, verb, ctx),
        Command::Products { verb } => records::run(&resources::PRODUCTS, verb, ctx),
        Command::Questions { verb } => records::run(&resources::QUESTIONS, verb, ctx),
        Command::Auth { verb } => auth::run(verb, ctx),
        Command::Profile { verb } => profile::run(verb, ctx),
    }
}

/// Имя для конверта ошибки: `reviews.scroll`, `auth.login`.
pub fn command_name(command: &Command) -> String {
    let (resource, verb) = match command {
        Command::Reviews { verb } => ("reviews", records_verb(verb)),
        Command::Products { verb } => ("products", records_verb(verb)),
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
    };
    format!("{resource}.{verb}")
}

fn records_verb(verb: &RecordsVerb) -> &'static str {
    match verb {
        RecordsVerb::Scroll(_) => "scroll",
    }
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
