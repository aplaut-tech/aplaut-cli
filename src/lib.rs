//! aplaut — консольный клиент Aplaut Platform API.

pub mod cli;
pub mod error;
pub mod term;

use clap::Parser;

/// Точка входа бинаря; возвращает код выхода.
pub fn run() -> u8 {
    match cli::Cli::try_parse() {
        Ok(_) => 0,
        Err(e) => {
            let _ = e.print();
            e.exit_code() as u8
        }
    }
}
