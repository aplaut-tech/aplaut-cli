//! Дерево команд. Грамматика: `aplaut <ресурс> <глагол> [аргументы]` (дизайн D1).
//! Глобальные флаги принимаются в любом месте строки (clig: order-independent).

use std::path::PathBuf;

use clap::builder::{PossibleValuesParser, TypedValueParser};
use clap::{Args, Parser, Subcommand};

use crate::output::Format;

pub const VERSION: &str = concat!(
    env!("CARGO_PKG_VERSION"),
    " (Platform API ",
    env!("APLAUT_SPEC_VERSION"),
    ")"
);

const ROOT_AFTER_HELP: &str = "\
Примеры:
  aplaut auth login
  aplaut reviews scroll --filter updated_at:gte:2024-01-01T00:00:00Z --format jsonl > reviews.jsonl
  aplaut products scroll --format csv --max-records 500 | tw

Документация: https://aplaut.com/docs/api-references/platform/
Поддержка: support@aplaut.com";

const SCROLL_AFTER_HELP: &str = "\
Примеры:
  aplaut reviews scroll --filter updated_at:gte:2024-01-01T00:00:00Z --format jsonl > reviews.jsonl
  aplaut reviews scroll --filter updated_at:gte:2024-01-01T00:00:00Z --state reviews.state.json

Без --filter сервер отдаёт только записи, изменённые за последние 30 дней.";

const SCROLL_AFTER_LONG_HELP: &str = "\
Примеры:
  # Полная выгрузка отзывов в JSONL для ClickHouse или DuckDB:
  aplaut reviews scroll --filter updated_at:gte:2000-01-01T00:00:00Z --format jsonl > reviews.jsonl

  # Просмотр в таблице:
  aplaut products scroll --format csv --max-records 500 | tw

  # Выгрузка с продолжением после сбоя (cron): повторный запуск той же команды продолжит с места остановки.
  aplaut reviews scroll --filter updated_at:gte:2024-01-01T00:00:00Z --state reviews.state.json --format jsonl >> reviews.jsonl

Без --filter сервер отдаёт только записи, изменённые за последние 30 дней.

Доставка — «хотя бы один раз»: после сбоя последняя страница может прийти повторно,
убирайте дубли по id (например, ReplacingMergeTree).

Коды выхода: 0 — успех; 2 — ошибка в параметрах; 3 — нет токена или он отклонён;
4 — часть данных выдана, выгрузка не завершена (откатите загрузку); 7 — исчерпаны повторы
после rate limit; 1 — прочие ошибки.";

#[derive(Debug, Parser)]
#[command(
    name = "aplaut",
    version = VERSION,
    about = "Консольный клиент Aplaut Platform API",
    after_help = ROOT_AFTER_HELP,
    arg_required_else_help = true,
    disable_version_flag = true
)]
pub struct Cli {
    /// Показать версию CLI и спеки API
    #[arg(long, action = clap::ArgAction::Version)]
    pub version: Option<bool>,
    #[command(flatten)]
    pub global: GlobalArgs,
    #[command(subcommand)]
    pub command: Command,
}

/// Свой заголовок в справке: флаги команды идут первыми, общие — ниже (clig).
#[derive(Debug, Clone, Args)]
#[command(next_help_heading = "Глобальные флаги")]
pub struct GlobalArgs {
    /// Профиль из ~/.config/aplaut (по умолчанию — APLAUT_PROFILE или «default»)
    #[arg(long, global = true, value_name = "NAME")]
    pub profile: Option<String>,
    /// Базовый URL API (по умолчанию https://api.aplaut.io/v4)
    #[arg(long, global = true, value_name = "URL")]
    pub base_url: Option<String>,
    /// Прочитать токен из stdin
    #[arg(long, global = true, conflicts_with = "token_file")]
    pub token_stdin: bool,
    /// Прочитать токен из файла (`-` — из stdin)
    #[arg(long, global = true, value_name = "PATH")]
    pub token_file: Option<PathBuf>,
    /// Не печатать служебные сообщения (предупреждения и ошибки остаются)
    #[arg(short, long, global = true)]
    pub quiet: bool,
    /// Отладочный вывод запросов (токен маскируется)
    #[arg(long, global = true)]
    pub verbose: bool,
    /// Никогда не спрашивать ввод
    #[arg(long, global = true)]
    pub no_input: bool,
    /// Без цвета
    #[arg(long, global = true)]
    pub no_color: bool,
    /// Таймаут одного запроса, секунд
    #[arg(long, global = true, value_name = "SECONDS", default_value_t = 30)]
    pub timeout: u64,
    /// Сколько раз повторять запрос после 429, 5xx и сетевых сбоев
    #[arg(long, global = true, value_name = "N", default_value_t = 6)]
    pub max_retries: u32,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Отзывы
    #[command(arg_required_else_help = true)]
    Reviews {
        #[command(subcommand)]
        verb: RecordsVerb,
    },
    /// Товары
    #[command(arg_required_else_help = true)]
    Products {
        #[command(subcommand)]
        verb: RecordsVerb,
    },
    /// Вопросы
    #[command(arg_required_else_help = true)]
    Questions {
        #[command(subcommand)]
        verb: RecordsVerb,
    },
    /// Токен доступа: сохранить или удалить
    #[command(arg_required_else_help = true)]
    Auth {
        #[command(subcommand)]
        verb: AuthVerb,
    },
}

#[derive(Debug, Subcommand)]
pub enum RecordsVerb {
    /// Выгрузить записи обходом по курсору (GET /scroll/{records_type})
    #[command(after_help = SCROLL_AFTER_HELP, after_long_help = SCROLL_AFTER_LONG_HELP)]
    Scroll(ScrollArgs),
}

#[derive(Debug, Clone, Args)]
pub struct ScrollArgs {
    /// Фильтр параметр:оператор:значение[,…]; без него сервер отдаёт только последние 30 дней
    #[arg(long, value_name = "EXPR")]
    pub filter: Option<String>,
    /// Сортировка: updated_at:asc (по умолчанию) или created_at:asc
    #[arg(long, value_name = "FIELD:asc")]
    pub sort: Option<String>,
    /// Связанные объекты через запятую, например author,product
    #[arg(long, value_name = "REL,…")]
    pub include: Option<String>,
    /// Записей на страницу, 1–100 (по умолчанию 100)
    #[arg(long, value_name = "N")]
    pub per_page: Option<u32>,
    /// Файл стейта: продолжить обход после сбоя или остановки
    #[arg(long, value_name = "PATH")]
    pub state: Option<PathBuf>,
    /// Остановиться после N записей (со --state можно продолжить)
    #[arg(long, value_name = "N")]
    pub max_records: Option<u64>,
    /// Формат вывода
    #[arg(long, value_name = "FORMAT", default_value = "raw", value_parser = format_parser())]
    pub format: Format,
}

#[derive(Debug, Subcommand)]
pub enum AuthVerb {
    /// Сохранить токен в профиль (ввод скрыт; в скриптах — --token-stdin или --token-file)
    Login,
    /// Удалить токен профиля
    Logout,
}

/// `output` не зависит от clap: имена форматов проверяет clap, превращение — `Format::from_name`.
fn format_parser() -> impl TypedValueParser<Value = Format> {
    PossibleValuesParser::new(Format::NAMES)
        .map(|name| Format::from_name(&name).expect("значение уже проверено clap"))
}
