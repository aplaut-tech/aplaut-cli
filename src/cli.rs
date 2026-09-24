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
  aplaut reviews get crm-4211 --include comments --format jsonl
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

  # Только нужные колонки CSV и в заданном порядке (<связь>.<атрибут> — из --include):
  aplaut reviews scroll --filter updated_at:gte:2024-01-01T00:00:00Z --include product --format csv --fields id,rating,body,product.name

  # Выгрузка с продолжением после сбоя (cron): повторный запуск той же команды продолжит с места остановки.
  aplaut reviews scroll --filter updated_at:gte:2024-01-01T00:00:00Z --state reviews.state.json --format jsonl >> reviews.jsonl

  # Для агента: данные — в файл, итог — одной строкой JSON в stderr.
  aplaut reviews scroll --filter updated_at:gte:2024-01-01T00:00:00Z --state reviews.state.json --format jsonl --json >> reviews.jsonl

Без --filter сервер отдаёт только записи, изменённые за последние 30 дней.

Доставка — «хотя бы один раз»: после сбоя последняя страница может прийти повторно,
убирайте дубли по id (например, ReplacingMergeTree).

JSON (--json): данные — в stdout в --format; итог — в stderr одной строкой:
  {\"ok\":true,\"command\":\"reviews.scroll\",\"cli_version\":…,\"dry_run\":false,
   \"result\":{\"records_type\",\"emitted\",\"emitted_total\",\"pages\",\"duplicates\",\"completed\",
   \"already_completed\",\"total_count\",\"state_path\"},\"warnings\":[{\"code\",\"message\"}]}
  Предупреждения: default_filter, base_url_env_ignored, fields_without_id, field_absent,
  csv_to_many, csv_new_columns.

Ошибки: invalid_filter, invalid_include, invalid_sort, invalid_per_page, invalid_max_records,
invalid_fields, duplicate_field, field_needs_include, fields_need_tabular_format, unknown_field,
field_to_many, state_mismatch, state_invalid, state_version, scroll_position_uncertain,
scroll_interrupted, no_progress, output_closed, no_token, invalid_token, unauthorized, forbidden,
rate_limited (retry_after — сколько секунд ждать), server_error, network_error.

Коды выхода: 0 — успех; 2 — ошибка в параметрах; 3 — нет токена или он отклонён;
7 — исчерпаны повторы после rate limit; 1 — прочие ошибки;
4 — выгрузка прервана, но в stdout только целые записи: со --state оставьте полученное
и запустите ту же команду снова — она продолжит после последней выданной страницы;
без --state выгрузите заново (повторы в обоих случаях убирайте по id).";

const ROOT_AFTER_LONG_HELP: &str = "\
Примеры:
  aplaut auth login
  aplaut reviews get crm-4211 --include comments --format jsonl
  aplaut reviews scroll --filter updated_at:gte:2024-01-01T00:00:00Z --format jsonl > reviews.jsonl
  aplaut products scroll --format csv --max-records 500 | tw

Для агентов и скриптов:
  --json      итог и ошибка — одной строкой JSON:
                {\"ok\":true,\"command\":…,\"cli_version\":…,\"dry_run\":false,\"result\":…,\"warnings\":[…]}
                {\"ok\":false,…,\"error\":{\"code\",\"message\",\"field\",\"retryable\",\"retry_after\",
                 \"hint\",\"request_id\"},\"warnings\":[…]}
              итог — в stdout (у scroll — в stderr: stdout занят данными), ошибка — в stderr;
              предупреждения — в warnings с кодами, а не текстом; включает --no-input
  --no-input  ничего не спрашивать: вместо вопроса — ошибка с подсказкой, какой флаг передать
  --yes       подтвердить удаление без вопроса (profile delete)
  --dry-run   (-n) у команд с изменениями: всё проверить и вернуть план в result, ничего не меняя
  retry_after в ошибке — сколько секунд ждать по словам сервера (429, 503).
  Без терминала ошибка приходит JSON-конвертом и без --json.
  Поля result — в разделе «JSON (--json)» справки команды: aplaut <команда> --help.
  Каталог кодов ошибок и предупреждений — README, раздел «Для агентов и скриптов».

Коды выхода: 0 — успех; 1 — прочие ошибки; 2 — ошибка в параметрах; 3 — нет токена или он
отклонён; 4 — выгрузка прервана, в stdout только целые записи; 5 — не найдено;
7 — rate limit, повторы исчерпаны; 8 — нужно подтверждение (--yes).

Документация: https://aplaut.com/docs/api-references/platform/
Поддержка: support@aplaut.com";

const GET_AFTER_LONG_HELP: &str = "\
Примеры:
  aplaut reviews get 5f1c2a9e8b7d6c5b4a3f2e1d                              # по внутреннему id
  aplaut reviews get crm-4211 --include author,comments --format jsonl    # по внешнему id
  aplaut products get 444772 --format csv --fields id,name

  # Для агента: запись — в stdout, итог — одной строкой JSON в stderr.
  aplaut reviews get crm-4211 --include comments --format jsonl --json

ID — внутренний идентификатор или external_id. --include по ресурсу (проверяется до запроса):
  reviews    author, product, comments, state_changes
  products   reviews_summary_item, reviews, questions, brand, category
  questions  author, product, answers

Форматы — как у scroll, для одной записи: raw — тело ответа одной строкой; jsonl — запись
с подставленными объектами --include; csv — заголовок и строка (--fields — по правилам scroll).

JSON (--json): запись — в stdout в --format; итог — в stderr одной строкой:
  {\"ok\":true,\"command\":\"reviews.get\",\"cli_version\":…,\"dry_run\":false,
   \"result\":{\"records_type\":\"reviews\",\"id\":\"<внутренний id>\"},\"warnings\":[]}

Ошибки: invalid_id, invalid_include, invalid_fields, duplicate_field, field_needs_include,
fields_need_tabular_format, unknown_field, field_to_many, not_found, bad_response, no_token,
invalid_token, unauthorized, forbidden, rate_limited (retry_after — сколько секунд ждать),
server_error, network_error, timeout.

Коды выхода: 0 — успех; 2 — ошибка в параметрах; 3 — нет токена или он отклонён; 5 — записи нет;
7 — rate limit, повторы исчерпаны; 1 — прочие ошибки.";

const REVIEWS_CREATE_AFTER_LONG_HELP: &str = "\
Примеры:
  aplaut reviews create --rating 5 --body \"Отличный магазин\" --author-name \"Анна\" --external-id crm-4211
  aplaut reviews create --rating 4 --body \"Хорошо\" --product-id 444772 -n   # показать запрос, не отправляя

  # Для агента: атрибуты — JSON-объектом из stdin, итог — одной строкой JSON в stdout.
  echo '{\"rating\":5,\"body\":\"Спасибо!\",\"photos\":[\"https://…/1.jpg\"],\"external_id\":\"crm-4211\"}' \\
    | aplaut reviews create --data - --json

Атрибуты — из схемы тела POST /reviews в спеке; флаги перекрывают одноимённые ключи --data.
Через --data передаётся всё, для чего нет флага: photos, tags, rating_details, dimensions,
custom_attributes, даты, hide_my_data и т. д. Без product_id отзыв создаётся о компании.
С -n (--dry-run) токен проверяется, но запросов нет.

Повторы: CLI повторяет запрос сам, только если сервер его точно не обработал (429, 503,
соединение не установилось). Иначе — request_outcome_unknown: проверьте, создан ли отзыв,
прежде чем повторять. С --external-id проверка — aplaut reviews get <external_id>.

JSON (--json):
  {\"ok\":true,\"command\":\"reviews.create\",\"cli_version\":…,\"dry_run\":false,
   \"result\":{\"request\":{\"method\":\"POST\",\"path\":\"/reviews\",
   \"body\":{\"data\":{\"type\":\"reviews\",\"attributes\":{…}}}},
   \"created\":{\"id\",\"type\":\"reviews\",\"attributes\":{…}}},\"warnings\":[]}
  С -n — тот же request, \"created\":null и \"dry_run\":true.

Ошибки: unknown_attribute (в hint — допустимые), invalid_attribute, missing_attribute,
invalid_data, stdin_conflict, stdin_is_terminal, validation_failed (422, в field — атрибут),
request_outcome_unknown, bad_response, no_token, invalid_token, unauthorized, forbidden,
rate_limited (retry_after — сколько секунд ждать), server_error, network_error.

Коды выхода: 0 — создан (с -n — план); 2 — ошибка во входных данных, до сети; 3 — нет токена
или он отклонён; 7 — rate limit, повторы исчерпаны; 1 — прочие ошибки, в том числе
request_outcome_unknown (исход неизвестен — проверьте, прежде чем повторять).";

/// Короткая справка (`-h`) листовых команд отсылает к длинной.
const LEAF_AFTER_HELP: &str = "Примеры, поля JSON (--json) и коды выхода: --help";

const AUTH_LOGIN_AFTER_LONG_HELP: &str = "\
Примеры:
  aplaut auth login                                     # ввод скрыт
  aplaut auth login --profile staging --base-url https://api.staging.example/v4
  printf '%s' \"$TOKEN\" | aplaut auth login --token-stdin --profile ci --json
  aplaut auth login --token-file token.txt --profile ci --dry-run --json

Токен сохраняется в ~/.config/aplaut/credentials (0600) и на сервере не проверяется.
Без терминала, с --no-input и с --json — только --token-stdin или --token-file.

JSON (--json):
  {\"ok\":true,\"command\":\"auth.login\",\"dry_run\":false,\"result\":{\"profile\":\"ci\",\"token\":\"***\",
   \"token_source\":\"prompt|stdin|file\",\"replaced\":false,\"credentials_path\":\"…\"},\"warnings\":[]}
  Сам токен не выводится никогда. С --dry-run — тот же result и \"dry_run\":true, файлы не меняются.

Ошибки: token_required (передайте --token-stdin или --token-file), empty_token,
invalid_token_format, stdin_is_terminal, token_file_unreadable, invalid_profile,
invalid_base_url, insecure_base_url, credentials_invalid, config_invalid, io_error.

Коды выхода: 0 — сохранено (с --dry-run — план); 2 — ошибка в параметрах;
3 — не прочитать файл токена; 1 — прочие ошибки.";

const AUTH_LOGOUT_AFTER_LONG_HELP: &str = "\
Примеры:
  aplaut auth logout --profile ci
  aplaut auth logout --profile ci --dry-run --json

JSON (--json):
  {\"ok\":true,\"command\":\"auth.logout\",\"dry_run\":false,\"result\":{\"profile\":\"ci\",\"removed\":true},
   \"warnings\":[]}
  removed: false — токена в профиле не было (это не ошибка).

Ошибки: invalid_profile, credentials_invalid, io_error.

Коды выхода: 0 — готово (с --dry-run — план); 2 — ошибка в параметрах; 1 — прочие ошибки.";

const PROFILE_LIST_AFTER_LONG_HELP: &str = "\
Примеры:
  aplaut profile list
  aplaut profile list --json

JSON (--json):
  {\"ok\":true,\"command\":\"profile.list\",\"dry_run\":false,\"result\":{\"profiles\":[{\"name\",
   \"description\",\"base_url\",\"base_url_default\",\"has_token\",\"active\"}]},\"warnings\":[]}
  Токены не выводятся: has_token говорит только, есть ли он.

Ошибки: config_invalid, credentials_invalid.

Коды выхода: 0 — успех; 1 — файлы профилей не читаются.";

const PROFILE_GET_AFTER_LONG_HELP: &str = "\
Примеры:
  aplaut profile get staging
  aplaut profile get staging --json

JSON (--json):
  {\"ok\":true,\"command\":\"profile.get\",\"dry_run\":false,\"result\":{\"name\",\"description\",
   \"base_url\",\"base_url_default\",\"has_token\",\"active\"},\"warnings\":[]}

Ошибки: profile_not_found (в hint — существующие профили), config_invalid, credentials_invalid.

Коды выхода: 0 — успех; 5 — профиля нет; 1 — прочие ошибки.";

const PROFILE_SET_AFTER_LONG_HELP: &str = "\
Примеры:
  aplaut profile set staging --base-url https://api.staging.example/v4 --description \"Стенд для тестов\"
  aplaut profile set staging --base-url none               # вернуть прод по умолчанию
  aplaut profile set staging --description none --dry-run --json

JSON (--json):
  {\"ok\":true,\"command\":\"profile.set\",\"dry_run\":false,\"result\":{\"profile\":\"staging\",
   \"created\":true,\"changes\":[{\"field\":\"base_url\",\"from\":null,\"to\":\"https://…\"}],
   \"config_path\":\"…\"},\"warnings\":[]}
  С --dry-run — тот же result и \"dry_run\":true, файл не меняется.

Ошибки: nothing_to_set, invalid_profile, invalid_base_url, insecure_base_url,
invalid_description, config_invalid, io_error.

Коды выхода: 0 — сохранено (с --dry-run — план); 2 — ошибка в параметрах; 1 — прочие ошибки.";

const PROFILE_DELETE_AFTER_LONG_HELP: &str = "\
Примеры:
  aplaut profile delete staging                  # в терминале спросит подтверждение
  aplaut profile delete staging --yes --json
  aplaut profile delete staging --dry-run --json

JSON (--json):
  {\"ok\":true,\"command\":\"profile.delete\",\"dry_run\":false,\"result\":{\"profile\":\"staging\",
   \"removed_config\":true,\"removed_token\":false},\"warnings\":[]}
  С --dry-run — что было бы удалено; подтверждение не нужно.

Ошибки: confirmation_required (без терминала, с --no-input и --json нужен --yes),
profile_not_found, invalid_profile, config_invalid, credentials_invalid, io_error.

Коды выхода: 0 — удалено (с --dry-run — план); 2 — ошибка в параметрах; 5 — профиля нет;
8 — нужен --yes; 1 — прочие ошибки.";

const PROFILE_EDIT_AFTER_LONG_HELP: &str = "\
Примеры:
  aplaut profile edit
  EDITOR=\"code --wait\" aplaut profile edit

Правится копия config.toml: оригинал заменяется только после проверки. VS Code, Zed,
Sublime Text, gvim и Kate получают флаг ожидания сами.

JSON (--json): не поддерживается — редактору нужен человек. С --json, --no-input и без
терминала команда отказывает (terminal_required); агентам и скриптам — aplaut profile set.

Ошибки: terminal_required, editor_failed, editor_returned_immediately, config_invalid,
config_changed_during_edit, io_error.

Коды выхода: 0 — сохранено или без изменений; 2 — нет терминала; 1 — прочие ошибки.";

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
        verb: ReviewsVerb,
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
    /// Профили: base URL и наличие токена (сам токен задаёт `auth login`)
    #[command(arg_required_else_help = true)]
    Profile {
        #[command(subcommand)]
        verb: ProfileVerb,
    },
}

/// У отзывов, кроме общих глаголов чтения, есть запись (спека reviews-write W1).
#[derive(Debug, Subcommand)]
pub enum ReviewsVerb {
    #[command(flatten)]
    Records(RecordsVerb),
    /// Создать отзыв (POST /reviews): атрибуты флагами или JSON-объектом в --data
    #[command(after_help = LEAF_AFTER_HELP, after_long_help = REVIEWS_CREATE_AFTER_LONG_HELP)]
    Create(CreateReviewArgs),
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
