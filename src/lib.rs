//! aplaut — консольный клиент Aplaut Platform API.

// `CliError` намеренно плоский (поля читают рендер и тесты): ошибка — редкий путь CLI,
// лишнее копирование ничтожно рядом с сетевым I/O, а `Box` усложнил бы доступ к полям.
#![allow(clippy::result_large_err)]

pub mod api_error;
pub mod auth;
pub mod cli;
pub mod clock;
pub mod commands;
pub mod config;
pub mod envelope;
pub mod error;
pub mod filter;
pub mod fsutil;
pub mod http;
pub mod ops;
pub mod output;
pub mod page;
pub mod resources;
pub mod secret;
pub mod spec;
pub mod state;
pub mod term;
pub mod time;

use std::ffi::OsString;
use std::io::{self, IsTerminal, Write};
use std::rc::Rc;

use clap::{CommandFactory, FromArgMatches};

use crate::error::{CliError, Exit};

/// Точка входа бинаря; возвращает код выхода.
pub fn run() -> u8 {
    let env = auth::EnvSnapshot::capture();
    let args: Vec<OsString> = std::env::args_os().collect();
    let stderr_tty = io::stderr().is_terminal();
    // `--no-color` нужен до разбора: от него зависит, раскрасит ли clap справку и ошибки.
    let no_color_flag = args.iter().any(|a| a == "--no-color");
    let color = term::color_enabled(stderr_tty, no_color_flag, &env.term);
    let mut command = cli::Cli::command();
    if !color {
        command = command.color(clap::ColorChoice::Never);
    }
    let parsed = command
        .try_get_matches_from(args)
        .and_then(|matches| cli::Cli::from_arg_matches(&matches));
    let cli = match parsed {
        Ok(cli) => cli,
        Err(err) => return clap_error(err, stderr_tty),
    };
    let name = commands::command_name(&cli.command);
    let reporter = Rc::new(term::Reporter::new(
        cli.global.quiet,
        cli.global.verbose,
        stderr_tty,
        color,
    ));
    let ctx = commands::Ctx {
        global: cli.global,
        env,
        reporter: reporter.clone(),
        clock: Rc::new(clock::SystemClock::new()),
    };
    match commands::dispatch(cli.command, &ctx) {
        Ok(()) => 0,
        Err(err) => {
            reporter.clear_progress();
            let text = if stderr_tty {
                error::render_human(&err, color)
            } else {
                format!("{}\n", envelope::failure(&err, &name, false, &[]))
            };
            let _ = io::stderr().write_all(text.as_bytes());
            err.exit.code()
        }
    }
}

/// Справка и версия — с кодом clap; ошибки разбора — код 2, без TTY — JSON-конвертом.
fn clap_error(err: clap::Error, stderr_tty: bool) -> u8 {
    use clap::error::ErrorKind;
    match err.kind() {
        ErrorKind::DisplayHelp
        | ErrorKind::DisplayVersion
        | ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand => {
            let _ = err.print();
            err.exit_code() as u8
        }
        _ if stderr_tty => {
            let _ = err.print();
            Exit::Usage.code()
        }
        _ => {
            let rendered = err.render().to_string();
            let message = rendered.trim().trim_start_matches("error: ").to_string();
            let usage = CliError::usage("usage", message).with_hint("aplaut --help");
            let _ = writeln!(
                io::stderr(),
                "{}",
                envelope::failure(&usage, "aplaut", false, &[])
            );
            Exit::Usage.code()
        }
    }
}
