//! Ожидание задачи экспорта и скачивание архива (спека writes-and-exports §5, R8, R9; §9): в процессе, с
//! фальшивыми часами — без реального сна.

mod support;

use std::io::Write;
use std::rc::Rc;
use std::time::Duration;

use aplaut_cli::api_error::ErrorContext;
use aplaut_cli::clock::FakeClock;
use aplaut_cli::download::{self, Timeouts};
use aplaut_cli::http::{ApiClient, HttpSettings};
use aplaut_cli::ops::export::{self, ExportTask};
use aplaut_cli::secret::Secret;
use aplaut_cli::term::{Reporter, SharedBuf};
use serde_json::json;
use support::{MockServer, Reply, TempDir};

const NOW: i64 = 1_790_578_000_000;
const RESUME: &str = "aplaut exports get e1 --wait";

struct Harness {
    api: ApiClient,
    clock: Rc<FakeClock>,
    reporter: Rc<Reporter>,
}

fn harness(server: &MockServer, max_retries: u32) -> Harness {
    let clock = Rc::new(FakeClock::new(NOW));
    let reporter = Rc::new(Reporter::with_writer(
        false,
        false,
        false,
        false,
        Box::new(SharedBuf::default()),
    ));
    let base_url = server.base_url();
    let api = ApiClient::new(
        HttpSettings {
            base_url: base_url.clone(),
            timeout: Duration::from_secs(5),
            max_retries,
        },
        Secret::new("tok"),
        ErrorContext {
            token_source: "APLAUT_ACCESS_TOKEN".into(),
            base_url,
        },
        clock.clone(),
        reporter.clone(),
    );
    Harness {
        api,
        clock,
        reporter,
    }
}

fn task(state: &str, extra: serde_json::Value) -> serde_json::Value {
    let mut attributes = json!({"records_type": "reviews", "format": "jsonl", "state": state,
        "created_at": "2026-09-28T09:52:30.321+03:00"});
    attributes
        .as_object_mut()
        .unwrap()
        .extend(extra.as_object().unwrap().clone());
    json!({"id": "e1", "type": "export_tasks", "attributes": attributes})
}

fn reply(state: &str, extra: serde_json::Value) -> Reply {
    Reply::json(200, json!({"data": task(state, extra)}).to_string())
}

fn waiting() -> ExportTask {
    ExportTask::from_record(&task("waiting", json!({}))).unwrap()
}

fn gzip(text: &str) -> Vec<u8> {
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    encoder.write_all(text.as_bytes()).unwrap();
    encoder.finish().unwrap()
}

fn raw(status: u16, body: Vec<u8>) -> Reply {
    Reply::Http {
        status,
        headers: vec![],
        body,
    }
}

const TIMEOUTS: Timeouts = Timeouts {
    response: Duration::from_secs(5),
    body: Duration::from_secs(5),
};

#[test]
fn task_is_read_in_the_server_shape() {
    let parsed = ExportTask::from_record(&task(
        "completed",
        json!({"archive_size": 825435, "archive_url": "https://x/a.jsonl.gz",
               "archive_content_type": "application/gzip", "finished_at": "2026-09-28T09:52:37.401+03:00",
               "search_options": {"filter": null, "sort": null}, "unknown_field": 1}),
    ))
    .unwrap();
    assert_eq!(parsed.id, "e1");
    assert_eq!(parsed.state.as_deref(), Some("completed"));
    assert_eq!(
        parsed.archive_size.as_ref().map(ToString::to_string),
        Some("825435".into())
    );
    assert!(parsed.is_completed());
    assert!(
        ExportTask::from_record(&json!({"type": "export_tasks"})).is_err(),
        "нет id"
    );
}

#[test]
fn wait_polls_with_a_growing_interval_until_completed() {
    let server = MockServer::start(vec![
        reply("processing", json!({})),
        reply("processing", json!({})),
        reply("processing", json!({})),
        reply("completed", json!({"finished_at": "x"})),
    ]);
    let mut h = harness(&server, 0);
    let done = export::wait(
        &mut h.api,
        h.clock.as_ref(),
        &h.reporter,
        waiting(),
        Duration::from_secs(1800),
        RESUME,
    )
    .unwrap();
    assert!(done.is_completed());
    let secs: Vec<f64> = h.clock.sleeps().iter().map(Duration::as_secs_f64).collect();
    assert_eq!(secs, [2.0, 3.0, 4.5, 6.75]);
    assert!(server
        .requests()
        .iter()
        .all(|r| r.path == "/v4/export_tasks/e1"));
}

#[test]
fn the_interval_stops_growing_at_thirty_seconds() {
    let mut replies: Vec<Reply> = (0..10).map(|_| reply("processing", json!({}))).collect();
    replies.push(reply("completed", json!({"finished_at": "x"})));
    let server = MockServer::start(replies);
    let mut h = harness(&server, 0);
    export::wait(
        &mut h.api,
        h.clock.as_ref(),
        &h.reporter,
        waiting(),
        Duration::from_secs(1800),
        RESUME,
    )
    .unwrap();
    let sleeps = h.clock.sleeps();
    assert_eq!(sleeps.iter().max(), Some(&Duration::from_secs(30)));
    assert_eq!(sleeps.last(), Some(&Duration::from_secs(30)));
}

#[test]
fn an_already_finished_task_needs_no_requests() {
    let server = MockServer::start(vec![]);
    let mut h = harness(&server, 0);
    let completed =
        ExportTask::from_record(&task("completed", json!({"finished_at": "x"}))).unwrap();
    export::wait(
        &mut h.api,
        h.clock.as_ref(),
        &h.reporter,
        completed,
        Duration::from_secs(10),
        RESUME,
    )
    .unwrap();
    assert!(server.requests().is_empty());
    assert!(h.clock.sleeps().is_empty());
}

#[test]
fn wait_timeout_is_retryable_and_says_how_to_resume() {
    let server = MockServer::start(vec![
        reply("processing", json!({})),
        reply("processing", json!({})),
    ]);
    let mut h = harness(&server, 0);
    let err = export::wait(
        &mut h.api,
        h.clock.as_ref(),
        &h.reporter,
        waiting(),
        Duration::from_secs(5),
        RESUME,
    )
    .unwrap_err();
    assert_eq!(
        (err.code.as_str(), err.retryable),
        ("export_wait_timeout", true)
    );
    assert!(err.message.contains("e1"), "{}", err.message);
    assert!(err.hint.unwrap().contains(RESUME));
    let total: Duration = h.clock.sleeps().iter().sum();
    assert_eq!(total, Duration::from_secs(5), "не дольше --wait-timeout");
}

#[test]
fn rejected_task_reports_the_server_message_and_jq_hint() {
    let server = MockServer::start(vec![reply(
        "rejected",
        json!({"format": "csv", "export_format": "{id: .id}",
               "error_message": "undefined method '[]' for nil", "finished_at": "x"}),
    )]);
    let mut h = harness(&server, 0);
    let err = export::wait(
        &mut h.api,
        h.clock.as_ref(),
        &h.reporter,
        waiting(),
        Duration::from_secs(60),
        RESUME,
    )
    .unwrap_err();
    assert_eq!(err.code, "export_rejected");
    assert!(err.message.contains("undefined method"), "{}", err.message);
    assert!(err.hint.unwrap().contains("массив"));
}

#[test]
fn refused_task_explains_the_quota_and_other_states_count_as_rejected() {
    let server = MockServer::start(vec![reply(
        "refused",
        json!({"refuse_reason": "export_forbidden_in_company_settings", "finished_at": "x"}),
    )]);
    let mut h = harness(&server, 0);
    let err = export::wait(
        &mut h.api,
        h.clock.as_ref(),
        &h.reporter,
        waiting(),
        Duration::from_secs(60),
        RESUME,
    )
    .unwrap_err();
    assert_eq!(err.code, "export_refused");
    assert!(err.message.contains("квота"), "{}", err.message);
    let server = MockServer::start(vec![reply("archived", json!({"finished_at": "x"}))]);
    let mut h = harness(&server, 0);
    let err = export::wait(
        &mut h.api,
        h.clock.as_ref(),
        &h.reporter,
        waiting(),
        Duration::from_secs(60),
        RESUME,
    )
    .unwrap_err();
    assert_eq!(err.code, "export_rejected");
    assert!(err.message.contains("archived"), "{}", err.message);
}

/// Review Focus: сбой сети посреди ожидания — id задачи остаётся в подсказке.
#[test]
fn network_failure_while_waiting_keeps_the_task_id() {
    let server = MockServer::start(vec![Reply::text(500, "oops")]);
    let mut h = harness(&server, 0);
    let err = export::wait(
        &mut h.api,
        h.clock.as_ref(),
        &h.reporter,
        waiting(),
        Duration::from_secs(60),
        RESUME,
    )
    .unwrap_err();
    assert_eq!(err.code, "server_error");
    assert!(err.hint.unwrap().contains(RESUME));
}

#[test]
fn gzip_archive_is_unpacked_into_the_destination() {
    let server = MockServer::start(vec![raw(200, gzip("{\"id\":\"r1\"}\n{\"id\":\"r2\"}\n"))]);
    let dir = TempDir::new("dl-gzip");
    let dest = dir.path().join("reviews.jsonl");
    let url = format!("{}/export_data/a.jsonl.gz?1790578357", server.origin());
    let written = download::download(&url, true, &dest, TIMEOUTS).unwrap();
    assert_eq!(
        std::fs::read_to_string(&dest).unwrap(),
        "{\"id\":\"r1\"}\n{\"id\":\"r2\"}\n"
    );
    assert_eq!(written, 24);
    assert_eq!(
        std::fs::read_dir(dir.path()).unwrap().count(),
        1,
        "tmp не остался"
    );
}

#[test]
fn non_gzip_archive_is_saved_as_is() {
    let body = b"PK\x03\x04 xlsx bytes".to_vec();
    let server = MockServer::start(vec![raw(200, body.clone())]);
    let dir = TempDir::new("dl-raw");
    let dest = dir.path().join("products.xlsx");
    let url = format!("{}/export_data/p.xlsx", server.origin());
    download::download(&url, false, &dest, TIMEOUTS).unwrap();
    assert_eq!(std::fs::read(&dest).unwrap(), body);
}

/// Review Focus: токен не уходит на хост хранилища — и после редиректа.
#[test]
fn archive_request_carries_no_authorization_even_after_a_redirect() {
    let server = MockServer::start(vec![
        Reply::text(302, "").with_header("Location", "/export_data/b.jsonl.gz"),
        raw(200, gzip("x\n")),
    ]);
    let dir = TempDir::new("dl-redirect");
    let dest = dir.path().join("r.jsonl");
    let url = format!("{}/export_data/a.jsonl.gz", server.origin());
    download::download(&url, true, &dest, TIMEOUTS).unwrap();
    let requests = server.requests();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[1].path, "/export_data/b.jsonl.gz");
    assert!(requests.iter().all(|r| r.header("authorization").is_none()));
}

/// Review Focus: оборванное скачивание не трогает прежний файл и не оставляет tmp.
#[test]
fn interrupted_download_keeps_the_old_file_and_leaves_no_tmp() {
    let server = MockServer::start(vec![Reply::Truncated(Duration::ZERO)]);
    let dir = TempDir::new("dl-cut");
    let dest = dir.path().join("reviews.jsonl");
    std::fs::write(&dest, "old").unwrap();
    let url = format!("{}/export_data/a.jsonl", server.origin());
    let err = download::download(&url, false, &dest, TIMEOUTS).unwrap_err();
    assert!(
        ["network_error", "timeout"].contains(&err.code.as_str()),
        "{}",
        err.code
    );
    assert!(err.retryable);
    assert_eq!(std::fs::read_to_string(&dest).unwrap(), "old");
    assert_eq!(
        std::fs::read_dir(dir.path()).unwrap().count(),
        1,
        "tmp удалён"
    );
}

#[test]
fn broken_gzip_is_a_bad_response_and_http_errors_keep_their_status() {
    let server = MockServer::start(vec![raw(200, b"not gzip at all".to_vec())]);
    let dir = TempDir::new("dl-bad");
    let dest = dir.path().join("reviews.jsonl");
    let url = format!("{}/export_data/a.jsonl.gz", server.origin());
    let err = download::download(&url, true, &dest, TIMEOUTS).unwrap_err();
    assert_eq!(err.code, "bad_response");
    assert!(!dest.exists());
    let server = MockServer::start(vec![Reply::text(404, "NoSuchKey")]);
    let url = format!("{}/export_data/gone.jsonl.gz", server.origin());
    let err = download::download(&url, true, &dest, TIMEOUTS).unwrap_err();
    assert_eq!(err.code, "http_404");
    assert!(!dest.exists());
}

/// Review Focus (fix round 1, finding 1): ureq 3.4 заворачивает обрыв тела по таймауту в
/// `io::Error::other(ureq::Error::Timeout(_))`, а не в `ErrorKind::TimedOut` — без явной
/// распаковки исходной ошибки такой обрыв выглядел бы как `network_error`, и агент не понял бы,
/// что средство — увеличить `--wait-timeout`, а не повторить как есть.
#[test]
fn body_timeout_is_reported_as_timeout_not_network_error() {
    // Заголовки и первые байты тела приходят сразу, дальше сервер 300 мс молчит — дольше,
    // чем настроенный таймаут тела (50 мс), но не настолько, чтобы тест был медленным.
    let server = MockServer::start(vec![Reply::Truncated(Duration::from_millis(300))]);
    let dir = TempDir::new("dl-body-timeout");
    let dest = dir.path().join("reviews.jsonl");
    std::fs::write(&dest, "old").unwrap();
    let url = format!("{}/export_data/a.jsonl", server.origin());
    let timeouts = Timeouts {
        response: Duration::from_secs(5),
        body: Duration::from_millis(50),
    };
    let err = download::download(&url, false, &dest, timeouts).unwrap_err();
    assert_eq!(err.code, "timeout");
    assert!(err.retryable);
    assert_eq!(std::fs::read_to_string(&dest).unwrap(), "old");
    assert_eq!(
        std::fs::read_dir(dir.path()).unwrap().count(),
        1,
        "tmp удалён"
    );
}

/// Review Focus (fix round 1, finding 2): хранилище может отдать архив с транспортным
/// `Content-Encoding: gzip` (частый приём для статики — реальные байты объекта и есть тело
/// ответа, только сам HTTP-клиент должен их распаковать). `ureq` собран без фичи `gzip`
/// (`Cargo.toml`), поэтому он не трогает тело сам и не снимает заголовок — распаковывает
/// код `download`, ровно один раз, по `archive_content_type` сервера, а не по заголовку
/// транспорта. Без этой фичи валидный архив не превращается в `bad_response` от двойной
/// распаковки.
#[test]
fn transport_content_encoding_gzip_does_not_cause_double_decoding() {
    let server = MockServer::start(vec![
        raw(200, gzip("plain text payload\n")).with_header("Content-Encoding", "gzip")
    ]);
    let dir = TempDir::new("dl-content-encoding");
    let dest = dir.path().join("reviews.jsonl");
    let url = format!("{}/export_data/a.jsonl.gz", server.origin());
    download::download(&url, true, &dest, TIMEOUTS).unwrap();
    assert_eq!(
        std::fs::read_to_string(&dest).unwrap(),
        "plain text payload\n"
    );
}

/// Review Focus (fix round 1, finding 3): `flate2::read::GzDecoder` останавливается после
/// первого gzip-члена и молча отбрасывает хвост — `MultiGzDecoder` распаковывает все члены
/// подряд.
#[test]
fn multi_member_gzip_archive_is_fully_unpacked() {
    let mut body = gzip("{\"id\":\"r1\"}\n");
    body.extend(gzip("{\"id\":\"r2\"}\n"));
    let server = MockServer::start(vec![raw(200, body)]);
    let dir = TempDir::new("dl-multi-gzip");
    let dest = dir.path().join("reviews.jsonl");
    let url = format!("{}/export_data/a.jsonl.gz", server.origin());
    download::download(&url, true, &dest, TIMEOUTS).unwrap();
    assert_eq!(
        std::fs::read_to_string(&dest).unwrap(),
        "{\"id\":\"r1\"}\n{\"id\":\"r2\"}\n"
    );
}
