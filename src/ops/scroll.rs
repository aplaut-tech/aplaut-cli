//! Обход данных через `GET /scroll/{records_type}` (дизайн §5.3).
//!
//! Стейт сохраняется только после записи страницы и только в точках фиксации формата:
//! он никогда не указывает на данные, которых нет у приёмника (at-least-once).
//!
//! Курсор не идемпотентен (стейджинг, 2026-09-23): повтор тем же курсором отдаёт следующую
//! страницу. Поэтому перед каждым продолжением стейт помечается `in_flight`, а после сбоя с
//! неизвестным исходом слепое продолжение по курсору запрещено — предлагается новый обход
//! с границы последней выданной записи.

use std::collections::HashSet;
use std::path::Path;

use crate::clock::Clock;
use crate::error::CliError;
use crate::filter;
use crate::http::{self, ApiClient, Pace};
use crate::output::{Commit, RecordSink};
use crate::page::Page;
use crate::state::{self, ScrollParams, ScrollState};
use crate::term::Reporter;

/// Страниц подряд без новых записей, после которых обход считается зациклившимся.
pub const MAX_STALE_PAGES: u32 = 3;

pub struct ScrollJob<'a> {
    pub records_type: &'a str,
    pub params: ScrollParams,
    pub state_path: Option<&'a Path>,
    pub max_records: Option<u64>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ScrollOutcome {
    /// Записей, выданных в этом запуске (после дедупа хвоста).
    pub emitted: u64,
    pub pages: u64,
    pub duplicates: u64,
    pub completed: bool,
    pub already_completed: bool,
    pub total_count: Option<u64>,
    /// Всего за обход, включая прошлые запуски по тому же стейту.
    pub emitted_total: u64,
}

pub fn run(
    api: &mut ApiClient,
    sink: &mut dyn RecordSink,
    job: &ScrollJob,
    reporter: &Reporter,
    clock: &dyn Clock,
) -> Result<ScrollOutcome, CliError> {
    let (mut state, loaded) = initial_state(job)?;
    // Последний сохранённый стейт: только его можно помечать `in_flight` — он указывает на
    // данные, уже зафиксированные у приёмника.
    let mut durable: Option<ScrollState> = loaded.then(|| state.clone());
    let mut outcome = ScrollOutcome {
        completed: state.completed,
        already_completed: state.completed,
        total_count: state.total_count,
        emitted_total: state.emitted,
        ..ScrollOutcome::default()
    };
    if state.completed {
        return Ok(outcome);
    }
    let path = format!("/scroll/{}", job.records_type);
    let mut stale_pages = 0;
    loop {
        let opening = state.cursor.is_none();
        let query = request_query(&state);
        let query: Vec<(&str, &str)> = query.iter().map(|(k, v)| (*k, v.as_str())).collect();
        let response = if opening {
            api.get(&path, &query, Pace::Scroll)
        } else {
            // Отметка — до отправки: даже SIGKILL посреди запроса не приведёт к слепому продолжению.
            mark_in_flight(job.state_path, &mut durable, clock)
                .map_err(|e| partial_if(e, outcome.emitted))?;
            api.get_continuation(&path, &query)
        };
        let response = match response {
            Ok(response) => response,
            Err(err) if err.code == http::OUTCOME_UNKNOWN => {
                let at = durable.as_ref().unwrap_or(&state);
                return Err(partial_if(
                    interrupted(err, at, job.state_path),
                    outcome.emitted,
                ));
            }
            Err(err) => {
                // Сервер запрос не обработал (429, 503, 4xx, нет соединения) — курсор годен.
                clear_in_flight(job.state_path, &mut durable, clock)
                    .map_err(|e| partial_if(e, outcome.emitted))?;
                return Err(partial_if(err, outcome.emitted));
            }
        };
        let request_id = response.request_id.clone();
        let page = Page::parse(response.body)
            .map_err(|e| partial_if(e.with_request_id(request_id.clone()), outcome.emitted))?;
        if opening {
            state.total_count = page.meta.total_count;
            state.applied_filter = page.meta.applied_filter.clone();
            if let Some(applied) = &page.meta.applied_filter {
                reporter.warn(&default_filter_warning(job.records_type, applied));
            }
        }
        if page.meta.has_more && page.meta.cursor.is_none() {
            return Err(partial_if(
                CliError::general(
                    "bad_response",
                    "сервер сообщил has_more, но не вернул курсор",
                )
                .with_request_id(request_id),
                outcome.emitted,
            ));
        }
        let seen: HashSet<String> = state.last_page_ids.iter().cloned().collect();
        // Прогресс меряем новыми id, а не записанным: raw пишет и повторы, но зациклившийся
        // сервер должен останавливать обход в любом формате.
        let new_records = page.ids().iter().filter(|id| !seen.contains(*id)).count();
        let report = sink
            .write_page(&page, &seen)
            .map_err(|e| partial_if(e, outcome.emitted))?;
        outcome.pages += 1;
        outcome.emitted += report.written;
        outcome.duplicates += report.duplicates;
        state.emitted += report.written;
        state.last_page_ids = page.ids();
        state.cursor = page.meta.cursor.clone();
        state.completed = !page.meta.has_more;
        if let Some(value) = last_sort_value(&page, &state.params.sort) {
            state.last_sort_value = Some(value);
        }
        if report.commit == Commit::Durable {
            state.in_flight = false;
            save(job.state_path, &mut state, clock).map_err(|e| partial_if(e, outcome.emitted))?;
            durable = Some(state.clone());
        }
        reporter.progress(&progress_line(
            job.records_type,
            state.emitted,
            state.total_count,
        ));
        if state.completed {
            break;
        }
        stale_pages = if new_records == 0 { stale_pages + 1 } else { 0 };
        if stale_pages >= MAX_STALE_PAGES {
            return Err(partial_if(
                CliError::general(
                    "no_progress",
                    format!("{MAX_STALE_PAGES} страницы подряд без новых записей: обход, похоже, зациклился"),
                )
                .with_request_id(request_id)
                .with_hint("стейт сохранён на последней странице; сообщите в поддержку, приложив request id"),
                outcome.emitted,
            ));
        }
        if job.max_records.is_some_and(|max| outcome.emitted >= max) {
            break;
        }
    }
    if sink.finish().map_err(|e| partial_if(e, outcome.emitted))? == Commit::Durable {
        state.in_flight = false;
        save(job.state_path, &mut state, clock).map_err(|e| partial_if(e, outcome.emitted))?;
    }
    reporter.clear_progress();
    outcome.completed = state.completed;
    outcome.total_count = state.total_count;
    outcome.emitted_total = state.emitted;
    Ok(outcome)
}

/// Если часть данных уже ушла в stdout, любая ошибка — «частичный успех» (код 4): приёмник
/// должен знать, что выгрузка не завершена. Выданные записи целые — со `--state` их
/// сохраняют и продолжают, без него выгружают заново.
fn partial_if(err: CliError, emitted: u64) -> CliError {
    if emitted > 0 {
        err.into_partial()
    } else {
        err
    }
}

fn initial_state(job: &ScrollJob) -> Result<(ScrollState, bool), CliError> {
    let fresh = || ScrollState::new(job.records_type, job.params.clone());
    let Some(path) = job.state_path else {
        return Ok((fresh(), false));
    };
    match state::load(path)? {
        Some(saved) => {
            saved.check_matches(job.records_type, &job.params, path)?;
            if saved.in_flight {
                return Err(CliError::usage(
                    "scroll_position_uncertain",
                    format!(
                        "прошлый запуск по стейту {} прервался во время запроса продолжения: позиция обхода на сервере неизвестна",
                        path.display()
                    ),
                )
                .with_hint(resume_hint(&saved, Some(path))));
            }
            Ok((saved, true))
        }
        None => Ok((fresh(), false)),
    }
}

fn mark_in_flight(
    path: Option<&Path>,
    durable: &mut Option<ScrollState>,
    clock: &dyn Clock,
) -> Result<(), CliError> {
    match (path, durable.as_mut()) {
        (Some(path), Some(saved)) if !saved.in_flight => {
            saved.in_flight = true;
            state::save(path, saved, clock.unix_millis())
        }
        _ => Ok(()),
    }
}

fn clear_in_flight(
    path: Option<&Path>,
    durable: &mut Option<ScrollState>,
    clock: &dyn Clock,
) -> Result<(), CliError> {
    match (path, durable.as_mut()) {
        (Some(path), Some(saved)) if saved.in_flight => {
            saved.in_flight = false;
            state::save(path, saved, clock.unix_millis())
        }
        _ => Ok(()),
    }
}

fn interrupted(err: CliError, at: &ScrollState, path: Option<&Path>) -> CliError {
    CliError::general(
        "scroll_interrupted",
        format!("обход прерван: {}", err.message),
    )
    .with_request_id(err.request_id)
    .with_hint(resume_hint(at, path))
}

/// Повтор курсора пропустил бы страницу, поэтому безопасное продолжение — новый обход
/// с границы последней выданной записи (записи на границе придут повторно).
fn resume_hint(at: &ScrollState, path: Option<&Path>) -> String {
    let Some(value) = &at.last_sort_value else {
        return "сервер мог уже сдвинуть позицию обхода, а повтор курсора пропустил бы страницу: начните выгрузку заново".into();
    };
    let field = sort_field(&at.params.sort);
    let filter = shell_word(&filter::with_lower_bound(
        at.params.filter.as_deref(),
        field,
        value,
    ));
    let new_state = if path.is_some() {
        " --state <новый файл>"
    } else {
        ""
    };
    format!(
        "сервер мог уже сдвинуть позицию обхода, а повтор курсора пропустил бы страницу. \
         Продолжите новым обходом с границы последней выданной записи: --filter {filter}{new_state}; \
         записи на границе придут повторно — уберите дубли по id"
    )
}

fn sort_field(sort: &str) -> &str {
    sort.split(':').next().unwrap_or(sort)
}

fn last_sort_value(page: &Page, sort: &str) -> Option<String> {
    let field = sort_field(sort);
    page.data
        .last()?
        .get("attributes")?
        .get(field)?
        .as_str()
        .map(str::to_string)
}

/// Значение для копирования в шелл: в кавычках, если в нём есть что-то кроме безопасных символов.
fn shell_word(s: &str) -> String {
    if s.chars()
        .all(|c| c.is_ascii_alphanumeric() || "_:.,@%+=/-".contains(c))
    {
        s.to_string()
    } else {
        format!("'{}'", s.replace('\'', "'\\''"))
    }
}

/// Параметры уходят только при открытии; при продолжении — один курсор, иначе сервер вернёт 400.
fn request_query(state: &ScrollState) -> Vec<(&'static str, String)> {
    if let Some(cursor) = &state.cursor {
        return vec![("cursor", cursor.clone())];
    }
    let params = &state.params;
    let mut query = Vec::new();
    if let Some(filter) = &params.filter {
        query.push(("filter", filter.clone()));
    }
    query.push(("sort", params.sort.clone()));
    if !params.include.is_empty() {
        query.push(("include", params.include.join(",")));
    }
    query.push(("per_page", params.per_page.to_string()));
    query
}

fn save(path: Option<&Path>, state: &mut ScrollState, clock: &dyn Clock) -> Result<(), CliError> {
    match path {
        Some(path) => state::save(path, state, clock.unix_millis()),
        None => Ok(()),
    }
}

fn default_filter_warning(records_type: &str, applied: &str) -> String {
    format!(
        "фильтр не задан: сервер ограничил обход {records_type} условием {applied} \
         (записи, изменённые за последние 30 дней). Для полной выгрузки задайте границу явно, \
         например --filter updated_at:gte:2000-01-01T00:00:00Z"
    )
}

fn progress_line(records_type: &str, emitted: u64, total: Option<u64>) -> String {
    match total {
        Some(total) => format!("{records_type}: {emitted} из {total}"),
        None => format!("{records_type}: {emitted}"),
    }
}
