//! `aplaut exports` (спека writes-and-exports §5–§7; §9): задача экспорта, ожидание и файл. Модель и
//! ожидание — в `ops::export`, скачивание — в `download`, перевод фильтра — в `export_filter`.

use std::path::Path;
use std::time::Duration;

use serde::Serialize;
use serde_json::{json, Map, Number, Value};

use super::writes::{prepare, report_plan, write_operation};
use super::{check_id, connect, path_text, Ctx, Outcome};
use crate::cli::{CreateExportArgs, ExportWait, ExportsVerb, GetExportArgs};
use crate::download::{self, Timeouts};
use crate::error::CliError;
use crate::export_filter;
use crate::http::{ApiClient, Replay};
use crate::ops::export::{self, ExportTask};
use crate::ops::write::{self, Flag, WriteRequest};
use crate::resources::{self, Verb};
use crate::term::shell_word;

/// Формат по умолчанию: jq к нему не применяется, зато строки — целые записи.
const DEFAULT_FORMAT: &str = "jsonl";
/// Форматы, к которым сервер применяет jq (`export_format`).
const JQ_FORMATS: [&str; 2] = ["csv", "xlsx"];
const SURVEY_RESPONSES: &str = "survey_responses";
/// gzip распаковывается, остальное (xlsx) сохраняется как есть (§9).
const GZIP: &str = "application/gzip";
/// Исход неизвестен: повтор создал бы вторую задачу, а списка задач в API нет.
const VERIFY: &str = "задача, возможно, создана: повтор создаст вторую и займёт минутное окно; проверьте выгрузки в личном кабинете";

pub fn run(verb: ExportsVerb, ctx: &Ctx) -> Result<Outcome, CliError> {
    match verb {
        ExportsVerb::Create(args) => create(&args, ctx),
        ExportsVerb::Get(args) => get(&args, ctx),
    }
}

fn create(args: &CreateExportArgs, ctx: &Ctx) -> Result<Outcome, CliError> {
    let (spec, path) = write_operation(&resources::EXPORTS, Verb::ExportCreate)?;
    let flags: [Flag; 3] = [
        ("records_type", args.records_type.as_deref()),
        ("format", args.format.as_deref()),
        ("export_format", args.jq.as_deref()),
    ];
    let attributes = prepare(spec, &["records_type"], args.data.as_deref(), &flags, ctx)?;
    let attributes = with_search_options(with_format(attributes)?, args)?;
    check_wait(&args.wait)?;
    let request = write::request(spec, path, attributes);
    let mut api = connect(ctx)?;
    if args.dry.dry_run {
        report_plan(&request, ctx);
        return Ok(Outcome::stdout(ExportResult::planned(request)).with_dry_run(true));
    }
    let written = write::submit(&mut api, &request, Replay::OnlyIfUnprocessed, VERIFY)?;
    let task = ExportTask::from_record(&written.record)?;
    finish(&mut api, task, &args.wait, ctx, Some(request))
}

fn get(args: &GetExportArgs, ctx: &Ctx) -> Result<Outcome, CliError> {
    check_id(&args.id, "id")?;
    check_wait(&args.wait)?;
    let mut api = connect(ctx)?;
    let task = export::fetch(&mut api, &args.id)?;
    finish(&mut api, task, &args.wait, ctx, None)
}

/// Формат по умолчанию — явно в теле (план честный); jq — только у csv и xlsx.
fn with_format(mut attributes: Map<String, Value>) -> Result<Map<String, Value>, CliError> {
    let format = attributes
        .entry("format")
        .or_insert_with(|| Value::String(DEFAULT_FORMAT.into()))
        .as_str()
        .unwrap_or(DEFAULT_FORMAT)
        .to_string();
    if attributes.contains_key("export_format") && !JQ_FORMATS.contains(&format.as_str()) {
        return Err(CliError::usage(
            "jq_needs_csv_or_xlsx",
            format!("--jq сервер применяет только к csv и xlsx, а формат — {format}"),
        )
        .with_field("jq")
        .with_hint("добавьте --format csv (или xlsx) либо уберите --jq"));
    }
    Ok(attributes)
}

/// `--filter` и `--survey-id` перекрывают `filter` из `search_options` в `--data` (W5).
fn with_search_options(
    mut attributes: Map<String, Value>,
    args: &CreateExportArgs,
) -> Result<Map<String, Value>, CliError> {
    let records_type = attributes
        .get("records_type")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let mut options = match attributes.remove("search_options") {
        Some(Value::Object(options)) => options,
        _ => Map::new(),
    };
    if let Some(expr) = &args.filter {
        options.insert(
            "filter".into(),
            export_filter::search_filter(&records_type, expr)?,
        );
    }
    if let Some(survey) = &args.survey_id {
        if records_type != SURVEY_RESPONSES {
            return Err(CliError::usage(
                "usage",
                format!("--survey-id — только для survey_responses, а тип — {records_type}"),
            )
            .with_field("survey_id"));
        }
        let filter = options
            .entry("filter")
            .or_insert_with(|| Value::Object(Map::new()));
        if !filter.is_object() {
            *filter = Value::Object(Map::new());
        }
        filter
            .as_object_mut()
            .expect("filter — объект")
            .insert("survey_id".into(), json!({"eq": survey}));
    }
    let has_survey = options
        .get("filter")
        .and_then(|f| f.get("survey_id"))
        .and_then(|s| s.get("eq"))
        .is_some();
    if records_type == SURVEY_RESPONSES && !has_survey {
        // Без survey_id сервер ответит 422, а он тратит минутное окно (стейджинг, 2026-09-28).
        return Err(CliError::usage(
            "missing_attribute",
            "для survey_responses нужен опрос: survey_id",
        )
        .with_field("survey_id")
        .with_hint("передайте --survey-id ID"));
    }
    if !options.is_empty() {
        attributes.insert("search_options".into(), Value::Object(options));
    }
    Ok(attributes)
}

/// Всё, что сорвало бы ожидание или файл уже после создания задачи, — до сети.
fn check_wait(wait: &ExportWait) -> Result<(), CliError> {
    if wait.wait_timeout == 0 {
        return Err(
            CliError::usage("usage", "--wait-timeout должно быть больше 0")
                .with_field("wait_timeout"),
        );
    }
    let Some(output) = &wait.output else {
        return Ok(());
    };
    if output.is_dir() {
        return Err(CliError::usage(
            "usage",
            format!(
                "--output {}: это каталог, а нужен путь к файлу",
                output.display()
            ),
        )
        .with_field("output"));
    }
    let parent = match output.parent() {
        Some(p) if !p.as_os_str().is_empty() => p,
        _ => Path::new("."),
    };
    if !parent.is_dir() {
        return Err(CliError::usage(
            "usage",
            format!(
                "--output {}: каталога {} нет",
                output.display(),
                parent.display()
            ),
        )
        .with_field("output")
        .with_hint("создайте каталог заранее: файл кладётся атомарно рядом с ним"));
    }
    Ok(())
}

fn finish(
    api: &mut ApiClient,
    task: ExportTask,
    wait: &ExportWait,
    ctx: &Ctx,
    request: Option<WriteRequest>,
) -> Result<Outcome, CliError> {
    let resume = resume_command(&task.id, wait.output.as_deref());
    let task = if wait.wait || wait.output.is_some() {
        export::wait(
            api,
            ctx.clock.as_ref(),
            &ctx.reporter,
            task,
            Duration::from_secs(wait.wait_timeout),
            &resume,
        )?
    } else {
        task
    };
    let output_path = match &wait.output {
        Some(path) => Some(save(&task, path, wait, ctx, &resume)?),
        None => None,
    };
    report(&task, output_path.as_deref(), request.is_some(), ctx);
    Ok(Outcome::stdout(ExportResult::of(
        request,
        &task,
        output_path,
    )))
}

fn save(
    task: &ExportTask,
    path: &Path,
    wait: &ExportWait,
    ctx: &Ctx,
    resume: &str,
) -> Result<String, CliError> {
    let url = task.archive_url.as_deref().ok_or_else(|| {
        CliError::general(
            "bad_response",
            format!("экспорт {} готов, но ссылки на архив нет", task.id),
        )
    })?;
    let gzip = task.archive_content_type.as_deref() == Some(GZIP);
    let timeouts = Timeouts {
        response: Duration::from_secs(ctx.global.timeout),
        body: Duration::from_secs(wait.wait_timeout),
    };
    download::download(url, gzip, path, timeouts).map_err(|err| {
        let again = format!("задача {} готова; скачать снова — {resume}", task.id);
        let hint = match &err.hint {
            Some(hint) => format!("{hint}; {again}"),
            None => again,
        };
        err.with_hint(hint)
    })?;
    Ok(path_text(path))
}

/// Команда продолжения для подсказок: та же задача, без повторного создания.
fn resume_command(id: &str, output: Option<&Path>) -> String {
    match output {
        Some(path) => format!(
            "aplaut exports get {} --output {}",
            shell_word(id),
            shell_word(&path.display().to_string())
        ),
        None => format!("aplaut exports get {} --wait", shell_word(id)),
    }
}

fn report(task: &ExportTask, output_path: Option<&str>, created: bool, ctx: &Ctx) {
    let state = task.state.as_deref().unwrap_or("?");
    let line = match (output_path, task.is_completed()) {
        (Some(path), _) => format!("Экспорт {} сохранён: {path}", task.id),
        (None, true) => format!(
            "Экспорт {} готов: {}",
            task.id,
            task.archive_url.as_deref().unwrap_or("ссылки нет")
        ),
        (None, false) if created => format!(
            "Задача экспорта {} создана ({state}); дождаться и скачать — aplaut exports get {} --output FILE",
            task.id,
            shell_word(&task.id)
        ),
        (None, false) => {
            let detail = task
                .error_message
                .as_deref()
                .or(task.refuse_reason.as_deref())
                .map(|d| format!(": {d}"))
                .unwrap_or_default();
            format!("Задача экспорта {}: {state}{detail}", task.id)
        }
    };
    ctx.reporter.info(&line);
}

/// `result` обеих команд (§9): поля сервера и `output_path`; у `create` — ещё `request`.
#[derive(Serialize)]
struct ExportResult {
    #[serde(skip_serializing_if = "Option::is_none")]
    request: Option<WriteRequest>,
    id: Option<String>,
    records_type: Option<String>,
    format: Option<String>,
    state: Option<String>,
    archive_url: Option<String>,
    archive_size: Option<Number>,
    archive_content_type: Option<String>,
    created_at: Option<String>,
    started_at: Option<String>,
    finished_at: Option<String>,
    error_message: Option<String>,
    refuse_reason: Option<String>,
    output_path: Option<String>,
}

impl ExportResult {
    fn planned(request: WriteRequest) -> ExportResult {
        ExportResult {
            request: Some(request),
            id: None,
            records_type: None,
            format: None,
            state: None,
            archive_url: None,
            archive_size: None,
            archive_content_type: None,
            created_at: None,
            started_at: None,
            finished_at: None,
            error_message: None,
            refuse_reason: None,
            output_path: None,
        }
    }

    fn of(
        request: Option<WriteRequest>,
        task: &ExportTask,
        output_path: Option<String>,
    ) -> ExportResult {
        let task = task.clone();
        ExportResult {
            request,
            id: Some(task.id),
            records_type: task.records_type,
            format: task.format,
            state: task.state,
            archive_url: task.archive_url,
            archive_size: task.archive_size,
            archive_content_type: task.archive_content_type,
            created_at: task.created_at,
            started_at: task.started_at,
            finished_at: task.finished_at,
            error_message: task.error_message,
            refuse_reason: task.refuse_reason,
            output_path,
        }
    }
}
