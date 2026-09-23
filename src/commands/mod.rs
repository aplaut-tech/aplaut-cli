//! Клей между деревом clap и алгоритмами: сборка зависимостей и сообщения пользователю.

pub mod auth;
pub mod records;

use std::rc::Rc;

use crate::auth::EnvSnapshot;
use crate::cli::{AuthVerb, Command, GlobalArgs, RecordsVerb};
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

pub fn dispatch(command: Command, ctx: &Ctx) -> Result<(), CliError> {
    match command {
        Command::Reviews { verb } => records::run(&resources::REVIEWS, verb, ctx),
        Command::Products { verb } => records::run(&resources::PRODUCTS, verb, ctx),
        Command::Questions { verb } => records::run(&resources::QUESTIONS, verb, ctx),
        Command::Auth { verb } => auth::run(verb, ctx),
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
                AuthVerb::Login => "login",
                AuthVerb::Logout => "logout",
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
