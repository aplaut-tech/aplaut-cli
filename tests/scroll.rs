mod support;

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Duration;

use aplaut_cli::api_error::ErrorContext;
use aplaut_cli::clock::FakeClock;
use aplaut_cli::error::{CliError, Exit};
use aplaut_cli::http::{ApiClient, HttpSettings};
use aplaut_cli::ops::scroll::{self, ScrollJob, ScrollOutcome};
use aplaut_cli::output::{self, Commit, Format, PageReport, RecordSink};
use aplaut_cli::page::Page;
use aplaut_cli::secret::Secret;
use aplaut_cli::state::{self, ScrollParams, ScrollState};
use aplaut_cli::term::{Reporter, SharedBuf};
use support::{first_page_json, page_json, review, MockServer, Reply, TempDir};

struct Harness {
    server: MockServer,
    clock: Rc<FakeClock>,
    log: SharedBuf,
    reporter: Rc<Reporter>,
}

fn harness(replies: Vec<Reply>) -> Harness {
    let log = SharedBuf::default();
    Harness {
        server: MockServer::start(replies),
        clock: Rc::new(FakeClock::new(1_790_158_269_000)),
        reporter: Rc::new(Reporter::with_writer(
            false,
            false,
            false,
            false,
            Box::new(log.clone()),
        )),
        log,
    }
}

fn api(h: &Harness, max_retries: u32) -> ApiClient {
    let settings = HttpSettings {
        base_url: h.server.base_url(),
        timeout: Duration::from_secs(5),
        max_retries,
    };
    ApiClient::new(
        settings,
        Secret::new("tok"),
        ErrorContext::default(),
        h.clock.clone(),
        h.reporter.clone(),
    )
}

fn params(filter: Option<&str>) -> ScrollParams {
    ScrollParams {
        filter: filter.map(str::to_string),
        sort: "updated_at:asc".into(),
        include: vec![],
        per_page: 2,
    }
}

fn run(
    h: &Harness,
    format: Format,
    job: &ScrollJob,
    max_retries: u32,
) -> (Result<ScrollOutcome, CliError>, String) {
    let out = SharedBuf::default();
    let mut sink = output::make_sink(
        format,
        Box::new(out.clone()),
        &job.params.include,
        h.reporter.clone(),
    );
    let mut client = api(h, max_retries);
    let result = scroll::run(
        &mut client,
        sink.as_mut(),
        job,
        &h.reporter,
        h.clock.as_ref(),
    );
    (result, out.contents())
}

fn job<'a>(filter: Option<&str>, state: Option<&'a Path>) -> ScrollJob<'a> {
    ScrollJob {
        records_type: "reviews",
        params: params(filter),
        state_path: state,
        max_records: None,
    }
}

const FILTER: &str = "updated_at:gte:2020-01-01T00:00:00Z";

#[test]
fn walks_all_pages_sending_only_cursor_after_open() {
    let h = harness(vec![
        Reply::json(
            200,
            first_page_json(
                &[review("r1", "t1"), review("r2", "t2")],
                Some("c1"),
                true,
                5,
                None,
            ),
        ),
        Reply::json(
            200,
            page_json(&[review("r3", "t3"), review("r4", "t4")], Some("c2"), true),
        ),
        Reply::json(200, page_json(&[review("r5", "t5")], None, false)),
    ]);
    let (result, out) = run(&h, Format::Jsonl, &job(Some(FILTER), None), 0);
    let outcome = result.unwrap();
    assert_eq!(
        (
            outcome.emitted,
            outcome.pages,
            outcome.completed,
            outcome.total_count
        ),
        (5, 3, true, Some(5))
    );
    assert_eq!(out.lines().count(), 5);
    let reqs = h.server.requests();
    assert_eq!(reqs[0].query_keys(), vec!["filter", "sort", "per_page"]);
    assert_eq!(reqs[0].query_param("filter"), Some(FILTER));
    assert_eq!(reqs[0].query_param("per_page"), Some("2"));
    assert_eq!(reqs[1].query_keys(), vec!["cursor"]);
    assert_eq!(reqs[1].query_param("cursor"), Some("c1"));
    assert_eq!(reqs[2].query_param("cursor"), Some("c2"));
    assert!(
        h.clock
            .sleeps()
            .iter()
            .all(|d| *d == Duration::from_millis(2200)),
        "троттлинг scroll"
    );
}

#[test]
fn warns_about_server_default_filter() {
    let h = harness(vec![Reply::json(
        200,
        first_page_json(
            &[review("r1", "t1")],
            None,
            false,
            1,
            Some("updated_at:gte:2026-08-24T10:09:35Z"),
        ),
    )]);
    let (result, _) = run(&h, Format::Jsonl, &job(None, None), 0);
    result.unwrap();
    assert!(
        !h.server.requests()[0].query_keys().contains(&"filter"),
        "фильтр не подставляется"
    );
    let log = h.log.contents();
    assert!(
        log.contains("updated_at:gte:2026-08-24T10:09:35Z") && log.contains("30 дней"),
        "{log}"
    );
}

#[test]
fn resumes_from_state_and_drops_repeated_tail() {
    let dir = TempDir::new("resume");
    let path = dir.path().join("state.json");
    let mut saved = ScrollState::new("reviews", params(Some(FILTER)));
    saved.cursor = Some("c1".into());
    saved.last_page_ids = vec!["r2".into()];
    saved.emitted = 2;
    state::save(&path, &mut saved, 0).unwrap();
    let h = harness(vec![Reply::json(
        200,
        page_json(&[review("r2", "t2"), review("r3", "t3")], None, false),
    )]);
    let (result, out) = run(&h, Format::Jsonl, &job(Some(FILTER), Some(&path)), 0);
    let outcome = result.unwrap();
    assert_eq!(
        (outcome.emitted, outcome.duplicates, outcome.emitted_total),
        (1, 1, 3)
    );
    assert!(out.contains("\"r3\"") && !out.contains("\"r2\""));
    assert_eq!(h.server.requests()[0].query_keys(), vec!["cursor"]);
    let after = state::load(&path).unwrap().unwrap();
    assert!(after.completed && after.cursor.is_none());
}

#[test]
fn raw_keeps_repeated_tail() {
    let dir = TempDir::new("raw-dup");
    let path = dir.path().join("state.json");
    let mut saved = ScrollState::new("reviews", params(Some(FILTER)));
    saved.cursor = Some("c1".into());
    saved.last_page_ids = vec!["r2".into()];
    state::save(&path, &mut saved, 0).unwrap();
    let h = harness(vec![Reply::json(
        200,
        page_json(&[review("r2", "t2"), review("r3", "t3")], None, false),
    )]);
    let (result, out) = run(&h, Format::Raw, &job(Some(FILTER), Some(&path)), 0);
    let outcome = result.unwrap();
    assert_eq!((outcome.emitted, outcome.duplicates), (2, 1));
    assert_eq!(out.lines().count(), 1);
    assert!(out.contains("\"r2\""));
}

#[test]
fn completed_state_makes_no_requests() {
    let dir = TempDir::new("done");
    let path = dir.path().join("state.json");
    let mut saved = ScrollState::new("reviews", params(Some(FILTER)));
    saved.completed = true;
    state::save(&path, &mut saved, 0).unwrap();
    let h = harness(vec![]);
    let (result, out) = run(&h, Format::Jsonl, &job(Some(FILTER), Some(&path)), 0);
    assert!(result.unwrap().already_completed);
    assert!(out.is_empty() && h.server.requests().is_empty());
}

#[test]
fn state_mismatch_is_rejected_before_any_request() {
    let dir = TempDir::new("mismatch");
    let path = dir.path().join("state.json");
    let mut saved = ScrollState::new("reviews", params(Some(FILTER)));
    state::save(&path, &mut saved, 0).unwrap();
    let h = harness(vec![]);
    let (result, _) = run(&h, Format::Jsonl, &job(Some("rating:eq:5"), Some(&path)), 0);
    assert_eq!(result.unwrap_err().code, "state_mismatch");
    assert!(h.server.requests().is_empty());
}

#[test]
fn failure_mid_walk_is_partial_and_state_points_to_last_saved_page() {
    let dir = TempDir::new("partial");
    let path = dir.path().join("state.json");
    let h = harness(vec![
        Reply::json(
            200,
            first_page_json(
                &[review("r1", "t1"), review("r2", "t2")],
                Some("c1"),
                true,
                9,
                None,
            ),
        ),
        Reply::Hangup,
    ]);
    let (result, out) = run(&h, Format::Jsonl, &job(Some(FILTER), Some(&path)), 0);
    let err = result.unwrap_err();
    assert_eq!(
        (err.exit, err.code.as_str()),
        (Exit::Partial, "scroll_interrupted")
    );
    assert_eq!(out.lines().count(), 2, "страница до сбоя целиком в stdout");
    let saved = state::load(&path).unwrap().unwrap();
    assert_eq!(
        (saved.cursor.as_deref(), saved.emitted, saved.completed),
        (Some("c1"), 2, false)
    );
}

#[test]
fn non_json_page_is_bad_response() {
    let h = harness(vec![
        Reply::json(
            200,
            first_page_json(&[review("r1", "t1")], Some("c1"), true, 9, None),
        ),
        Reply::text(200, "<html>maintenance</html>"),
    ]);
    let (result, _) = run(&h, Format::Jsonl, &job(Some(FILTER), None), 0);
    let err = result.unwrap_err();
    assert_eq!(
        (err.exit, err.code.as_str()),
        (Exit::Partial, "bad_response")
    );
}

#[test]
fn has_more_without_cursor_is_bad_response() {
    let h = harness(vec![Reply::json(
        200,
        page_json(&[review("r1", "t1")], None, true),
    )]);
    let (result, _) = run(&h, Format::Jsonl, &job(Some(FILTER), None), 0);
    assert_eq!(result.unwrap_err().code, "bad_response");
}

#[test]
fn max_records_stops_early_and_keeps_cursor() {
    let dir = TempDir::new("max");
    let path = dir.path().join("state.json");
    let h = harness(vec![
        Reply::json(
            200,
            first_page_json(
                &[review("r1", "t1"), review("r2", "t2")],
                Some("c1"),
                true,
                6,
                None,
            ),
        ),
        Reply::json(
            200,
            page_json(&[review("r3", "t3"), review("r4", "t4")], Some("c2"), true),
        ),
    ]);
    let job = ScrollJob {
        max_records: Some(3),
        ..job(Some(FILTER), Some(&path))
    };
    let (result, out) = run(&h, Format::Jsonl, &job, 0);
    let outcome = result.unwrap();
    assert_eq!((outcome.emitted, outcome.completed), (4, false));
    assert_eq!(out.lines().count(), 4);
    assert_eq!(
        state::load(&path).unwrap().unwrap().cursor.as_deref(),
        Some("c2")
    );
    assert_eq!(h.server.requests().len(), 2);
}

#[test]
fn stale_pages_stop_the_walk() {
    let dir = TempDir::new("stale");
    let path = dir.path().join("state.json");
    let mut saved = ScrollState::new("reviews", params(Some(FILTER)));
    saved.cursor = Some("c0".into());
    saved.last_page_ids = vec!["r1".into()];
    state::save(&path, &mut saved, 0).unwrap();
    let h = harness(vec![
        Reply::json(200, page_json(&[review("r1", "t1")], Some("c1"), true)),
        Reply::json(200, page_json(&[review("r1", "t1")], Some("c2"), true)),
        Reply::json(200, page_json(&[review("r1", "t1")], Some("c3"), true)),
    ]);
    let (result, _) = run(&h, Format::Jsonl, &job(Some(FILTER), Some(&path)), 0);
    assert_eq!(result.unwrap_err().code, "no_progress");
}

/// Формат, который фиксирует данные только в `finish` (как будущий Parquet).
struct PendingSink {
    state_path: PathBuf,
    state_existed_during_write: Vec<bool>,
}

impl RecordSink for PendingSink {
    fn write_page(&mut self, page: &Page, _seen: &HashSet<String>) -> Result<PageReport, CliError> {
        self.state_existed_during_write
            .push(self.state_path.exists());
        Ok(PageReport {
            written: page.data.len() as u64,
            duplicates: 0,
            commit: Commit::Pending,
        })
    }

    fn finish(&mut self) -> Result<Commit, CliError> {
        Ok(Commit::Durable)
    }
}

#[test]
fn pending_sink_defers_state_until_finish() {
    let dir = TempDir::new("pending");
    let path = dir.path().join("state.json");
    let h = harness(vec![
        Reply::json(
            200,
            first_page_json(&[review("r1", "t1")], Some("c1"), true, 2, None),
        ),
        Reply::json(200, page_json(&[review("r2", "t2")], None, false)),
    ]);
    let mut sink = PendingSink {
        state_path: path.clone(),
        state_existed_during_write: vec![],
    };
    let mut client = api(&h, 0);
    let job = job(Some(FILTER), Some(&path));
    scroll::run(&mut client, &mut sink, &job, &h.reporter, h.clock.as_ref()).unwrap();
    assert_eq!(sink.state_existed_during_write, vec![false, false]);
    assert!(state::load(&path).unwrap().unwrap().completed);
}

#[test]
fn pending_sink_failure_leaves_no_state() {
    let dir = TempDir::new("pending-fail");
    let path = dir.path().join("state.json");
    let h = harness(vec![
        Reply::json(
            200,
            first_page_json(&[review("r1", "t1")], Some("c1"), true, 2, None),
        ),
        Reply::Hangup,
    ]);
    let mut sink = PendingSink {
        state_path: path.clone(),
        state_existed_during_write: vec![],
    };
    let mut client = api(&h, 0);
    let job = job(Some(FILTER), Some(&path));
    assert!(scroll::run(&mut client, &mut sink, &job, &h.reporter, h.clock.as_ref()).is_err());
    assert!(!path.exists());
}

#[test]
fn stale_pages_stop_the_walk_in_raw_too() {
    // raw не отбрасывает повторы, но и зациклившийся обход должен остановиться (новых id нет).
    let dir = TempDir::new("stale-raw");
    let path = dir.path().join("state.json");
    let mut saved = ScrollState::new("reviews", params(Some(FILTER)));
    saved.cursor = Some("c0".into());
    saved.last_page_ids = vec!["r1".into()];
    state::save(&path, &mut saved, 0).unwrap();
    let pages: Vec<Reply> = (1..=9)
        .map(|i| {
            Reply::json(
                200,
                page_json(&[review("r1", "t1")], Some(&format!("c{i}")), true),
            )
        })
        .collect();
    let h = harness(pages);
    let (result, _) = run(&h, Format::Raw, &job(Some(FILTER), Some(&path)), 0);
    assert_eq!(result.unwrap_err().code, "no_progress");
    assert_eq!(h.server.requests().len(), 3);
}

#[test]
fn interrupted_continuation_marks_state_and_next_run_refuses_blind_resume() {
    let dir = TempDir::new("inflight");
    let path = dir.path().join("state.json");
    let h = harness(vec![
        Reply::json(
            200,
            first_page_json(
                &[
                    review("r1", "2021-01-01T00:00:00Z"),
                    review("r2", "2021-02-02T00:00:00Z"),
                ],
                Some("c1"),
                true,
                9,
                None,
            ),
        ),
        Reply::Hangup,
    ]);
    let (result, out) = run(&h, Format::Jsonl, &job(Some(FILTER), Some(&path)), 6);
    let err = result.unwrap_err();
    assert_eq!(
        (err.exit, err.code.as_str()),
        (Exit::Partial, "scroll_interrupted")
    );
    assert_eq!(
        h.server.requests().len(),
        2,
        "продолжение не повторяется вслепую"
    );
    assert_eq!(out.lines().count(), 2);
    let hint = err.hint.unwrap();
    assert!(
        hint.contains("updated_at:gte:2021-02-02T00:00:00Z") && !hint.contains("2020-01-01"),
        "{hint}"
    );
    let saved = state::load(&path).unwrap().unwrap();
    assert!(saved.in_flight, "стейт помнит, что запрос был в полёте");
    assert_eq!(
        saved.last_sort_value.as_deref(),
        Some("2021-02-02T00:00:00Z")
    );

    let again = harness(vec![]);
    let (result, _) = run(&again, Format::Jsonl, &job(Some(FILTER), Some(&path)), 6);
    let err = result.unwrap_err();
    assert_eq!(
        (err.exit, err.code.as_str()),
        (Exit::Usage, "scroll_position_uncertain")
    );
    assert!(err
        .hint
        .unwrap()
        .contains("--filter updated_at:gte:2021-02-02T00:00:00Z"));
    assert!(
        again.server.requests().is_empty(),
        "по неизвестной позиции не ходим"
    );
}

#[test]
fn rate_limited_continuation_keeps_cursor_resumable() {
    let dir = TempDir::new("inflight-429");
    let path = dir.path().join("state.json");
    let h = harness(vec![
        Reply::json(
            200,
            first_page_json(&[review("r1", "t1")], Some("c1"), true, 9, None),
        ),
        Reply::text(429, "Throttled\n").with_header("Retry-After", "1"),
    ]);
    let (result, _) = run(&h, Format::Jsonl, &job(Some(FILTER), Some(&path)), 0);
    assert_eq!(result.unwrap_err().code, "rate_limited");
    let saved = state::load(&path).unwrap().unwrap();
    assert!(
        !saved.in_flight,
        "429 сервер не обработал — курсор по-прежнему годен"
    );
    let again = harness(vec![Reply::json(
        200,
        page_json(&[review("r2", "t2")], None, false),
    )]);
    let (result, _) = run(&again, Format::Jsonl, &job(Some(FILTER), Some(&path)), 0);
    result.unwrap();
    assert_eq!(again.server.requests()[0].query_param("cursor"), Some("c1"));
}
