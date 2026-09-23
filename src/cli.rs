//! Дерево команд. Грамматика: `aplaut <ресурс> <глагол> [аргументы]` (дизайн D1).

use clap::Parser;

pub const VERSION: &str = concat!(
    env!("CARGO_PKG_VERSION"),
    " (Platform API ",
    env!("APLAUT_SPEC_VERSION"),
    ")"
);

#[derive(Debug, Parser)]
#[command(
    name = "aplaut",
    version = VERSION,
    about = "Консольный клиент Aplaut Platform API",
    disable_version_flag = true
)]
pub struct Cli {
    /// Показать версию CLI и спеки API
    #[arg(long, action = clap::ArgAction::Version)]
    pub version: Option<bool>,
}
