//! Дерево команд. Грамматика: `aplaut <ресурс> <глагол> [аргументы]` (дизайн D1).
//! Глобальные флаги принимаются в любом месте строки (clig: order-independent).

use std::path::PathBuf;

use clap::builder::{PossibleValuesParser, TypedValueParser};
use clap::{Args, Parser, Subcommand};

use crate::output::Format;

mod help;
mod help_writes;
mod writes;

use help::*;
use help_writes::*;
pub use writes::*;

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
    after_help = ROOT_AFTER_HELP,
    after_long_help = ROOT_AFTER_LONG_HELP,
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
    /// Итог и ошибки — одной строкой JSON (форма — в --help команды); включает --no-input
    #[arg(long, global = true)]
    pub json: bool,
    /// Без цвета
    #[arg(long, global = true)]
    pub no_color: bool,
    /// Таймаут одного запроса, секунд
    #[arg(long, global = true, value_name = "SECONDS", default_value_t = 30)]
    pub timeout: u64,
    /// Сколько раз повторять запрос после 429, 5xx и сетевых сбоев (create и comment — только если сервер его точно не получил)
    #[arg(long, global = true, value_name = "N", default_value_t = 6)]
    pub max_retries: u32,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Отзывы
    #[command(arg_required_else_help = true)]
    Reviews {
        #[command(subcommand)]
        verb: ReviewsVerb,
    },
    /// Товары
    #[command(arg_required_else_help = true)]
    Products {
        #[command(subcommand)]
        verb: ProductsVerb,
    },
    /// Вопросы
    #[command(arg_required_else_help = true)]
    Questions {
        #[command(subcommand)]
        verb: QuestionsVerb,
    },
    /// Токен доступа: сохранить или удалить
    #[command(arg_required_else_help = true)]
    Auth {
        #[command(subcommand)]
        verb: AuthVerb,
    },
    /// Профили: base URL и наличие токена (сам токен задаёт `auth login`)
    #[command(arg_required_else_help = true)]
    Profile {
        #[command(subcommand)]
        verb: ProfileVerb,
    },
    /// Сам aplaut: обновление до последнего релиза
    #[command(name = "self", arg_required_else_help = true)]
    SelfCmd {
        #[command(subcommand)]
        verb: SelfVerb,
    },
    /// MCP-сервер для агентов: команды aplaut — инструментами (stdio)
    #[command(after_help = LEAF_AFTER_HELP, after_long_help = help::mcp_after_long_help())]
    Mcp(McpArgs),
}

/// У товаров, кроме общих глаголов чтения, есть запись (спека products-write P1).
#[derive(Debug, Subcommand)]
pub enum ProductsVerb {
    #[command(flatten)]
    Records(RecordsVerb),
    /// Создать товар (POST /products): атрибуты флагами или JSON-объектом в --data
    #[command(after_help = LEAF_AFTER_HELP, after_long_help = PRODUCTS_CREATE_AFTER_LONG_HELP)]
    Create(CreateProductArgs),
    /// Изменить товар (PUT /products/{id}): меняются только переданные атрибуты
    #[command(after_help = LEAF_AFTER_HELP, after_long_help = PRODUCTS_UPDATE_AFTER_LONG_HELP)]
    Update(UpdateProductArgs),
}

/// Атрибуты товара, общие для create и update (спека products-write §1). Описание может
/// начинаться с `-` (список), поэтому у него `allow_hyphen_values`.
#[derive(Debug, Clone, Args)]
pub struct ProductFields {
    /// Название
    #[arg(long, value_name = "TEXT")]
    pub name: Option<String>,
    /// URL карточки товара
    #[arg(long, value_name = "URL")]
    pub url: Option<String>,
    /// Цена, число
    #[arg(long, value_name = "N")]
    pub price: Option<String>,
    /// Наличие: true или false
    #[arg(long, value_name = "true|false")]
    pub available: Option<String>,
    /// Описание
    #[arg(long, value_name = "TEXT", allow_hyphen_values = true)]
    pub description: Option<String>,
    /// Внешний идентификатор группы (варианты одного товара)
    #[arg(long, value_name = "ID")]
    pub group_id: Option<String>,
    /// Категория по external_id (из YML)
    #[arg(long, value_name = "ID")]
    pub category_id: Option<String>,
    /// Категория по имени; нет такой — будет создана
    #[arg(long, value_name = "TEXT")]
    pub category_name: Option<String>,
    /// Бренд по имени; нет такого — будет создан
    #[arg(long, value_name = "TEXT")]
    pub brand_name: Option<String>,
}

#[derive(Debug, Clone, Args)]
pub struct CreateProductArgs {
    /// Id товара в вашей системе (обычно offer.id из YML); обязателен
    #[arg(long, value_name = "ID")]
    pub external_id: Option<String>,
    #[command(flatten)]
    pub fields: ProductFields,
    /// Атрибуты JSON-объектом из файла (`-` — из stdin); флаги перекрывают его ключи
    #[arg(long, value_name = "FILE|-")]
    pub data: Option<PathBuf>,
    #[command(flatten)]
    pub dry: DryRun,
}

#[derive(Debug, Clone, Args)]
pub struct UpdateProductArgs {
    /// Товар: внутренний или внешний (external_id) идентификатор
    pub id: String,
    /// Новый external_id — переименовать товар
    #[arg(long, value_name = "NEW_ID")]
    pub external_id: Option<String>,
    #[command(flatten)]
    pub fields: ProductFields,
    /// Атрибуты JSON-объектом из файла (`-` — из stdin); флаги перекрывают его ключи
    #[arg(long, value_name = "FILE|-")]
    pub data: Option<PathBuf>,
    #[command(flatten)]
    pub dry: DryRun,
}

/// У отзывов, кроме общих глаголов чтения, есть запись (спека reviews-write W1).
#[derive(Debug, Subcommand)]
pub enum ReviewsVerb {
    #[command(flatten)]
    Records(RecordsVerb),
    /// Создать отзыв (POST /reviews): атрибуты флагами или JSON-объектом в --data
    #[command(after_help = LEAF_AFTER_HELP, after_long_help = REVIEWS_CREATE_AFTER_LONG_HELP)]
    Create(CreateReviewArgs),
    /// Ответить на отзыв (POST /reviews/{id}/relationships/comments)
    #[command(after_help = LEAF_AFTER_HELP, after_long_help = REVIEWS_COMMENT_AFTER_LONG_HELP)]
    Comment(CommentArgs),
    /// Изменить отзыв (PUT /reviews/{id}): меняются только переданные атрибуты; нет отзыва — ошибка, с --upsert — создать
    #[command(after_help = LEAF_AFTER_HELP, after_long_help = REVIEWS_UPDATE_AFTER_LONG_HELP)]
    Update(UpdateReviewArgs),
}

#[derive(Debug, Clone, Args)]
pub struct CommentArgs {
    /// Отзыв: внутренний или внешний (external_id) идентификатор
    pub review_id: String,
    /// Текст комментария
    #[arg(long, value_name = "TEXT", allow_hyphen_values = true)]
    pub text: Option<String>,
    /// Имя автора
    #[arg(long, value_name = "TEXT")]
    pub author_name: Option<String>,
    /// E-mail автора
    #[arg(long, value_name = "EMAIL")]
    pub author_email: Option<String>,
    /// Статус модерации: published, waiting (по умолчанию), banned
    #[arg(long, value_name = "STATE")]
    pub state: Option<String>,
    /// Внутренний id родительского комментария — ответ на комментарий
    #[arg(long, value_name = "ID")]
    pub parent_id: Option<String>,
    /// Id комментария в вашей системе: по нему его проще найти после сбоя
    #[arg(long, value_name = "ID")]
    pub external_id: Option<String>,
    /// Атрибуты JSON-объектом из файла (`-` — из stdin); флаги перекрывают его ключи
    #[arg(long, value_name = "FILE|-")]
    pub data: Option<PathBuf>,
    #[command(flatten)]
    pub dry: DryRun,
}

/// Частые атрибуты — флагами, остальные — через `--data` (W5). Свободный текст может начинаться
/// с `-` (список в достоинствах), поэтому у текстовых флагов `allow_hyphen_values`.
#[derive(Debug, Clone, Args)]
pub struct CreateReviewArgs {
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
    /// Товар (обычно offer.id из YML); без него отзыв — о компании
    #[arg(long, value_name = "ID")]
    pub product_id: Option<String>,
    /// Id отзыва в вашей системе: по нему get проверит, создан ли отзыв после сбоя
    #[arg(long, value_name = "ID")]
    pub external_id: Option<String>,
    /// Имя автора
    #[arg(long, value_name = "TEXT")]
    pub author_name: Option<String>,
    /// E-mail автора
    #[arg(long, value_name = "EMAIL")]
    pub author_email: Option<String>,
    /// Статус модерации: published, waiting (по умолчанию), banned, held
    #[arg(long, value_name = "STATE")]
    pub state: Option<String>,
    /// Атрибуты JSON-объектом из файла (`-` — из stdin); флаги перекрывают его ключи
    #[arg(long, value_name = "FILE|-")]
    pub data: Option<PathBuf>,
    #[command(flatten)]
    pub dry: DryRun,
}

#[derive(Debug, Subcommand)]
pub enum RecordsVerb {
    /// Выгрузить записи обходом по курсору (GET /scroll/{records_type})
    #[command(after_help = SCROLL_AFTER_HELP, after_long_help = SCROLL_AFTER_LONG_HELP)]
    Scroll(ScrollArgs),
    /// Одна запись по внутреннему или внешнему id (GET /{records_type}/{id})
    #[command(after_help = LEAF_AFTER_HELP, after_long_help = GET_AFTER_LONG_HELP)]
    Get(GetArgs),
}

#[derive(Debug, Clone, Args)]
pub struct GetArgs {
    /// Внутренний или внешний (external_id) идентификатор
    pub id: String,
    /// Связанные объекты через запятую (допустимые по ресурсу — в --help)
    #[arg(long, value_name = "REL,…")]
    pub include: Option<String>,
    /// Формат вывода
    #[arg(long, value_name = "FORMAT", default_value = "raw", value_parser = format_parser())]
    pub format: Format,
    /// Колонки CSV (только --format csv) через запятую, в заданном порядке — как у scroll
    #[arg(long, value_name = "COL,…")]
    pub fields: Option<String>,
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
    /// Остановиться, набрав не меньше N записей: граница — страница, её не режем (со --state можно продолжить)
    #[arg(long, value_name = "N")]
    pub max_records: Option<u64>,
    /// Формат вывода
    #[arg(long, value_name = "FORMAT", default_value = "raw", value_parser = format_parser())]
    pub format: Format,
    /// Колонки CSV (только --format csv) через запятую, в заданном порядке: id,rating,body; <связь>_ref — id связи, <связь>.<атрибут> — поле объекта из --include
    #[arg(long, value_name = "COL,…")]
    pub fields: Option<String>,
}

#[derive(Debug, Subcommand)]
pub enum ProfileVerb {
    /// Показать профили: base URL, есть ли токен, какой активен
    #[command(after_help = LEAF_AFTER_HELP, after_long_help = PROFILE_LIST_AFTER_LONG_HELP)]
    List,
    /// Показать один профиль
    #[command(after_help = LEAF_AFTER_HELP, after_long_help = PROFILE_GET_AFTER_LONG_HELP)]
    Get {
        /// Имя профиля
        name: String,
    },
    /// Создать или изменить профиль: `--base-url URL|none`, `--description TEXT|none`
    #[command(after_help = LEAF_AFTER_HELP, after_long_help = PROFILE_SET_AFTER_LONG_HELP)]
    Set {
        /// Имя профиля
        name: String,
        /// Описание профиля для людей (`none` — убрать)
        #[arg(long, value_name = "TEXT|none")]
        description: Option<String>,
        #[command(flatten)]
        dry: DryRun,
    },
    /// Удалить профиль вместе с его токеном
    #[command(after_help = LEAF_AFTER_HELP, after_long_help = PROFILE_DELETE_AFTER_LONG_HELP)]
    Delete {
        /// Имя профиля
        name: String,
        /// Не спрашивать подтверждение (без терминала, с --no-input и --json — обязательно)
        #[arg(short = 'y', long)]
        yes: bool,
        #[command(flatten)]
        dry: DryRun,
    },
    /// Открыть файл профилей (config.toml) в $VISUAL / $EDITOR
    #[command(after_help = LEAF_AFTER_HELP, after_long_help = PROFILE_EDIT_AFTER_LONG_HELP)]
    Edit,
}

#[derive(Debug, Subcommand)]
pub enum AuthVerb {
    /// Сохранить токен в профиль (ввод скрыт; в скриптах — --token-stdin или --token-file)
    #[command(after_help = LEAF_AFTER_HELP, after_long_help = AUTH_LOGIN_AFTER_LONG_HELP)]
    Login(DryRun),
    /// Удалить токен профиля
    #[command(after_help = LEAF_AFTER_HELP, after_long_help = AUTH_LOGOUT_AFTER_LONG_HELP)]
    Logout(DryRun),
}

/// Глаголы `aplaut self` (спека self-update U1); `self` — ключевое слово Rust, отсюда `SelfCmd`.
#[derive(Debug, Subcommand)]
pub enum SelfVerb {
    /// Поставить последний релиз тем же установщиком, которым поставлен этот aplaut
    #[command(after_help = LEAF_AFTER_HELP, after_long_help = SELF_UPDATE_AFTER_LONG_HELP)]
    Update(DryRun),
}

/// `aplaut mcp` (спека mcp-server §1): флаги сервера; глобальные флаги он передаёт каждому вызову.
#[derive(Debug, Clone, Args)]
pub struct McpArgs {
    /// Открыть агенту запись: инструменты create, update и comment (список — в --help)
    #[arg(long)]
    pub allow_writes: bool,
}

/// `--dry-run` у команд, которые что-то меняют (спека agent mode §5).
#[derive(Debug, Clone, Args)]
pub struct DryRun {
    /// Всё проверить и показать, что будет сделано, ничего не меняя
    #[arg(short = 'n', long)]
    pub dry_run: bool,
}

/// `output` не зависит от clap: имена форматов проверяет clap, превращение — `Format::from_name`.
fn format_parser() -> impl TypedValueParser<Value = Format> {
    PossibleValuesParser::new(Format::NAMES)
        .map(|name| Format::from_name(&name).expect("значение уже проверено clap"))
}
