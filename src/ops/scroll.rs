//! Обход данных через `GET /scroll/{records_type}` (дизайн §5.3).
//!
//! Стейт сохраняется только после записи страницы и только в точках фиксации формата:
//! он никогда не указывает на данные, которых нет у приёмника (at-least-once).

use std::collections::HashSet;
use std::path::Path;

use crate::clock::Clock;
use crate::error::CliError;
use crate::http::{ApiClient, Pace};
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
    let mut state = initial_state(job)?;
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
        let response = api
            .get(&path, &query, Pace::Scroll)
            .map_err(|e| partial_if(e, outcome.emitted))?;
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
                CliError::general("bad_response", "сервер сообщил has_more, но не вернул курсор")
                    .with_request_id(request_id),
                outcome.emitted,
            ));
        }
        let seen: HashSet<String> = state.last_page_ids.iter().cloned().collect();
        let report = sink.write_page(&page, &seen).map_err(|e| partial_if(e, outcome.emitted))?;
        outcome.pages += 1;
        outcome.emitted += report.written;
        outcome.duplicates += report.duplicates;
        state.emitted += report.written;
        state.last_page_ids = page.ids();
        state.cursor = page.meta.cursor.clone();
        state.completed = !page.meta.has_more;
        if report.commit == Commit::Durable {
            save(job.state_path, &mut state, clock).map_err(|e| partial_if(e, outcome.emitted))?;
        }
        reporter.progress(&progress_line(job.records_type, state.emitted, state.total_count));
        if state.completed {
            break;
        }
        stale_pages = if report.written == 0 { stale_pages + 1 } else { 0 };
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
        save(job.state_path, &mut state, clock).map_err(|e| partial_if(e, outcome.emitted))?;
    }
    reporter.clear_progress();
    outcome.completed = state.completed;
    outcome.total_count = state.total_count;
    outcome.emitted_total = state.emitted;
    Ok(outcome)
}

/// Если часть данных уже ушла в stdout, любая ошибка — «частичный успех» (код 4):
/// приёмник должен откатить загрузку, а не решить, что данных не было.
fn partial_if(err: CliError, emitted: u64) -> CliError {
    if emitted > 0 {
        err.into_partial()
    } else {
        err
    }
}

fn initial_state(job: &ScrollJob) -> Result<ScrollState, CliError> {
    let fresh = || ScrollState::new(job.records_type, job.params.clone());
    let Some(path) = job.state_path else {
        return Ok(fresh());
    };
    match state::load(path)? {
        Some(saved) => {
            saved.check_matches(job.records_type, &job.params, path)?;
            Ok(saved)
        }
        None => Ok(fresh()),
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
