//! Задача экспорта одним файлом (спека writes-and-exports §5, R8; стейджинг и код сервера 2026-09-28 —
//! §9): разбор ответа в форме сервера, ожидание конечного состояния с нарастающим интервалом, ошибки
//! конечных состояний. Время — через `Clock`: тесты не спят.

use std::time::Duration;

use serde::Deserialize;
use serde_json::{Number, Value};

use crate::clock::Clock;
use crate::error::CliError;
use crate::http::{self, ApiClient, Pace};
use crate::term::Reporter;

/// Первый интервал опроса, рост и потолок (R8).
const FIRST_POLL: Duration = Duration::from_secs(2);
const POLL_GROWTH: f64 = 1.5;
const MAX_POLL: Duration = Duration::from_secs(30);

/// Задача в том виде, в каком её отдаёт сервер: `finished_at`, `error_message`, `refuse_reason` вместо полей
/// спеки API (§9). Лишние поля игнорируются, отсутствующие — `None`.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct ExportTask {
    #[serde(skip)]
    pub id: String,
    pub records_type: Option<String>,
    pub format: Option<String>,
    pub export_format: Option<String>,
    pub state: Option<String>,
    pub archive_url: Option<String>,
    pub archive_size: Option<Number>,
    pub archive_content_type: Option<String>,
    pub created_at: Option<String>,
    pub started_at: Option<String>,
    pub finished_at: Option<String>,
    pub error_message: Option<String>,
    pub refuse_reason: Option<String>,
}

impl ExportTask {
    /// `data` ответа JSON:API: `{"id", "type", "attributes": {…}}`.
    pub fn from_record(record: &Value) -> Result<ExportTask, CliError> {
        let id = record
            .get("id")
            .and_then(Value::as_str)
            .ok_or_else(|| bad_response("в ответе нет id задачи экспорта"))?;
        let attributes = record.get("attributes").cloned().unwrap_or(Value::Null);
        let attributes = if attributes.is_null() {
            Value::Object(Default::default())
        } else {
            attributes
        };
        let task: ExportTask = serde_json::from_value(attributes).map_err(|e| {
            // Ruling M5 финальной ревизии (2026-09-28): id уже прочитан из ответа сервера — задача,
            // скорее всего, создана (POST) или точно существует (GET); не терять его в ошибке разбора
            // остальных полей, иначе агент не узнает, чем в итоге кончилась выгрузка.
            bad_response(&format!("задача экспорта не разобралась: {e}")).with_hint(format!(
                "id задачи — {id}; проверить её — aplaut exports get {}",
                crate::term::shell_word(id)
            ))
        })?;
        Ok(ExportTask {
            id: id.to_string(),
            ..task
        })
    }

    pub fn is_completed(&self) -> bool {
        self.state.as_deref() == Some("completed")
    }
}

/// `GET /export_tasks/{id}`.
pub fn fetch(api: &mut ApiClient, id: &str) -> Result<ExportTask, CliError> {
    let path = format!("/export_tasks/{}", http::path_segment(id));
    let response = api.get(&path, &[], Pace::Default)?;
    let record = serde_json::from_slice::<Value>(&response.body)
        .ok()
        .and_then(|mut doc| doc.get_mut("data").map(Value::take))
        .filter(Value::is_object)
        .ok_or_else(|| {
            bad_response("ответ сервера не похож на задачу экспорта")
                .with_request_id(response.request_id.clone())
        })?;
    ExportTask::from_record(&record)
}

/// Что известно о задаче: идёт, готова или кончилась ошибкой.
enum Status {
    Pending,
    Completed,
    Failed(CliError),
}

fn status(task: &ExportTask) -> Status {
    match task.state.as_deref() {
        Some("completed") => Status::Completed,
        Some("rejected") => Status::Failed(rejected(task)),
        Some("refused") => Status::Failed(refused(task)),
        // Состояние, которого мы не знаем, но задача закончилась — не ждать вечно.
        _ if task.finished_at.is_some() => Status::Failed(rejected(task)),
        _ => Status::Pending,
    }
}

/// Опрос до конечного состояния (R8): 2 с, ×1,5, не больше 30 с; не дольше `timeout`. `resume` — команда,
/// которой агент продолжит ожидание, не создавая задачу заново.
pub fn wait(
    api: &mut ApiClient,
    clock: &dyn Clock,
    reporter: &Reporter,
    task: ExportTask,
    timeout: Duration,
    resume: &str,
) -> Result<ExportTask, CliError> {
    let started = clock.elapsed();
    let mut interval = FIRST_POLL;
    let mut task = task;
    loop {
        match status(&task) {
            Status::Completed => {
                reporter.clear_progress();
                return Ok(task);
            }
            Status::Failed(err) => {
                reporter.clear_progress();
                return Err(err);
            }
            Status::Pending => {}
        }
        let waited = clock.elapsed().saturating_sub(started);
        if waited >= timeout {
            reporter.clear_progress();
            return Err(wait_timeout(&task, timeout, resume));
        }
        reporter.progress(&format!(
            "Экспорт {}: {}, {}",
            task.id,
            task.state.as_deref().unwrap_or("?"),
            human(waited)
        ));
        clock.sleep(interval.min(timeout - waited));
        interval = interval.mul_f64(POLL_GROWTH).min(MAX_POLL);
        task = fetch(api, &task.id).map_err(|err| {
            reporter.clear_progress();
            resumable(err, &task.id, resume)
        })?;
    }
}

/// Ошибка чтения задачи посреди ожидания: задача на сервере жива — агенту нужен её id. Подсказка сервера
/// (например, ссылка на документацию) сохраняется, команда продолжения дописывается.
fn resumable(err: CliError, id: &str, resume: &str) -> CliError {
    let resume = format!("задача {id} создана; продолжить — {resume}");
    let hint = match &err.hint {
        Some(hint) => format!("{hint}; {resume}"),
        None => resume,
    };
    err.with_hint(hint)
}

fn rejected(task: &ExportTask) -> CliError {
    let reason = task
        .error_message
        .as_deref()
        .unwrap_or("причина не указана");
    let message = match task.state.as_deref() {
        Some("rejected") | None => format!("экспорт {} завершился с ошибкой: {reason}", task.id),
        Some(other) => format!(
            "экспорт {} завершился в состоянии «{other}»: {reason}",
            task.id
        ),
    };
    let tabular = matches!(task.format.as_deref(), Some("csv" | "xlsx"));
    // Хард-лимиты записей (код сервера, 2026-09-28, app/models/exports/export_*_task.rb): reviews,
    // questions, consumers, orders — 500 000, products — 800 000; сверх лимита сервер сам заканчивает
    // задачу `rejected` с текстом «Can't export N records, the limit is 500000.» — сузить выборку
    // может только `--filter`.
    let generic = format!(
        "сузьте выгрузку --filter (лимит записей: 500 000, у товаров — 800 000) или создайте задачу \
заново позже; если ошибка повторяется — сообщите в поддержку (support@aplaut.com), id задачи {}",
        task.id
    );
    let hint = if tabular && task.export_format.is_some() {
        format!(
            "jq-фильтр (--jq) у csv и xlsx должен вернуть массив, например [.id, .rating]; объект \
сервер отклоняет; {generic}"
        )
    } else {
        generic
    };
    CliError::general("export_rejected", message).with_hint(hint)
}

/// `refused` (в спеке API — `suspended`): экспорт выключен в настройках компании
/// (`export_forbidden_in_company_settings`) или задача не стартовала за сутки
/// (`maximum_time_to_start_exceeded`) (код сервера, 2026-09-28,
/// `app/interactors/exports/create_and_start_export_task.rb:54-81`). Превышение 60-минутной дневной
/// квоты экспорта компании само по себе задачу не отклоняет — она лишь уходит в низкоприоритетную
/// очередь `exports_lp` и выполняется медленнее; это не `refused`.
fn refused(task: &ExportTask) -> CliError {
    let (reason, hint) = match task.refuse_reason.as_deref() {
        Some("export_forbidden_in_company_settings") => (
            "экспорт запрещён в настройках компании".to_string(),
            "включите экспорт в настройках компании или напишите в поддержку (support@aplaut.com)"
                .to_string(),
        ),
        Some("maximum_time_to_start_exceeded") => (
            "задача ждала запуска дольше суток".to_string(),
            "создайте задачу заново".to_string(),
        ),
        Some(other) => (
            format!("причина: {other}"),
            "создайте задачу заново позже".to_string(),
        ),
        None => (
            "причина не указана".to_string(),
            "создайте задачу заново позже".to_string(),
        ),
    };
    CliError::general(
        "export_refused",
        format!("сервер отклонил экспорт {}: {reason}", task.id),
    )
    .with_hint(hint)
}

fn wait_timeout(task: &ExportTask, timeout: Duration, resume: &str) -> CliError {
    CliError::general(
        "export_wait_timeout",
        format!(
            "экспорт {} не завершился за {} с (состояние {})",
            task.id,
            timeout.as_secs(),
            task.state.as_deref().unwrap_or("?")
        ),
    )
    .retryable(true)
    .with_hint(format!(
        "задача продолжается на сервере; дождаться — {resume}"
    ))
}

fn human(waited: Duration) -> String {
    let secs = waited.as_secs();
    if secs < 60 {
        format!("{secs} с")
    } else {
        format!("{} мин", secs / 60)
    }
}

fn bad_response(message: &str) -> CliError {
    CliError::general("bad_response", message)
}
