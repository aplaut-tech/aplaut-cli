//! Аргументы `aplaut exports` (спека writes-and-exports §5): задача экспорта одним файлом.

use std::path::PathBuf;

use clap::{Args, Subcommand};

use super::help::LEAF_AFTER_HELP;
use super::help_writes::{EXPORTS_CREATE_AFTER_LONG_HELP, EXPORTS_GET_AFTER_LONG_HELP};
use super::DryRun;

#[derive(Debug, Subcommand)]
pub enum ExportsVerb {
    /// Создать задачу экспорта (POST /export_tasks); с --wait дождаться, с --output — скачать файл
    #[command(after_help = LEAF_AFTER_HELP, after_long_help = EXPORTS_CREATE_AFTER_LONG_HELP)]
    Create(CreateExportArgs),
    /// Задача экспорта по id (GET /export_tasks/{id}); с --wait дождаться, с --output — скачать файл
    #[command(after_help = LEAF_AFTER_HELP, after_long_help = EXPORTS_GET_AFTER_LONG_HELP)]
    Get(GetExportArgs),
}

/// Ожидание и скачивание — общее у create и get (R8, R9).
#[derive(Debug, Clone, Args)]
pub struct ExportWait {
    /// Дождаться конца задачи (опрос: сначала через 2 с, интервал растёт до 30 с)
    #[arg(long)]
    pub wait: bool,
    /// Сколько ждать задачу и скачивание, секунд
    #[arg(long, value_name = "SECONDS", default_value_t = 1800)]
    pub wait_timeout: u64,
    /// Скачать готовый файл сюда (включает --wait): gzip распаковывается, xlsx — как есть; файл
    /// перезаписывается; "-" не значит stdout — экспорт пишет только файл
    #[arg(long, value_name = "PATH")]
    pub output: Option<PathBuf>,
}

#[derive(Debug, Clone, Args)]
pub struct CreateExportArgs {
    /// Что выгрузить: reviews, products, questions, consumers, orders, survey_responses
    #[arg(long, value_name = "TYPE")]
    pub records_type: Option<String>,
    /// Формат файла: jsonl (по умолчанию), csv, xlsx
    #[arg(long, value_name = "FORMAT")]
    pub format: Option<String>,
    /// jq-фильтр строк для csv и xlsx; должен вернуть массив, например [.id, .rating]
    #[arg(long, value_name = "FILTER", allow_hyphen_values = true)]
    pub jq: Option<String>,
    /// Фильтр как у scroll: параметр:оператор:значение[,…]; только reviews, products, questions
    #[arg(long, value_name = "EXPR")]
    pub filter: Option<String>,
    /// Опрос, ответы которого выгрузить; для survey_responses обязателен
    #[arg(long, value_name = "ID")]
    pub survey_id: Option<String>,
    /// Атрибуты задачи JSON-объектом из файла (`-` — из stdin); флаги перекрывают его ключи
    #[arg(long, value_name = "FILE|-")]
    pub data: Option<PathBuf>,
    #[command(flatten)]
    pub wait: ExportWait,
    #[command(flatten)]
    pub dry: DryRun,
}

#[derive(Debug, Clone, Args)]
pub struct GetExportArgs {
    /// Id задачи экспорта (поле id из exports create)
    pub id: String,
    #[command(flatten)]
    pub wait: ExportWait,
}
