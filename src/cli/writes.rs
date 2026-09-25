//! Аргументы записи у отзывов (`update`), вопросов, клиентов и заказов (спека writes-and-exports);
//! общее у `update` с upsert — `UpdateInput`.

use std::path::PathBuf;

use clap::Args;

use super::DryRun;

/// Общее у `update` ресурсов, где PUT создаёт объект, если его нет (R3): `--data`, `--upsert`, `-n`.
#[derive(Debug, Clone, Args)]
pub struct UpdateInput {
    /// Атрибуты JSON-объектом из файла (`-` — из stdin); флаги перекрывают его ключи
    #[arg(long, value_name = "FILE|-")]
    pub data: Option<PathBuf>,
    /// Нет объекта с таким ID — создать его (внешний id — ID); без флага update меняет только существующий
    #[arg(long)]
    pub upsert: bool,
    #[command(flatten)]
    pub dry: DryRun,
}

/// Флаги `reviews update` — частые атрибуты PUT /reviews/{id}; остальные — через `--data`. Внешнего
/// id нет: сервер его в PUT игнорирует (стейджинг, 2026-09-25; спека writes-and-exports §9).
#[derive(Debug, Clone, Args)]
pub struct UpdateReviewArgs {
    /// Отзыв: внутренний или внешний (external_id) идентификатор
    pub id: String,
    /// Оценка, число от 1 до 5
    #[arg(long, value_name = "N")]
    pub rating: Option<String>,
    /// Текст отзыва
    #[arg(long, value_name = "TEXT", allow_hyphen_values = true)]
    pub body: Option<String>,
    /// Достоинства
    #[arg(long, value_name = "TEXT", allow_hyphen_values = true)]
    pub pros: Option<String>,
    /// Недостатки
    #[arg(long, value_name = "TEXT", allow_hyphen_values = true)]
    pub cons: Option<String>,
    /// Товар (обычно offer.id из YML)
    #[arg(long, value_name = "ID")]
    pub product_id: Option<String>,
    /// Имя автора
    #[arg(long, value_name = "TEXT")]
    pub author_name: Option<String>,
    /// E-mail автора
    #[arg(long, value_name = "EMAIL")]
    pub author_email: Option<String>,
    /// Статус модерации: published, waiting, banned, held
    #[arg(long, value_name = "STATE")]
    pub state: Option<String>,
    #[command(flatten)]
    pub input: UpdateInput,
}
