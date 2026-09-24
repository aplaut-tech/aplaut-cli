mod support;

use std::rc::Rc;
use std::time::Duration;

use aplaut_cli::api_error::ErrorContext;
use aplaut_cli::clock::FakeClock;
use aplaut_cli::error::Exit;
use aplaut_cli::http::{ApiClient, HttpSettings, Method, Pace, Replay};
use aplaut_cli::secret::Secret;
use aplaut_cli::term::{Reporter, SharedBuf};
use support::{MockServer, Reply};

const TOKEN: &str = "tok-secret-123";
/// 2026-09-23T10:11:09Z — совпадает с `Date` в ответах ниже.
const NOW: i64 = 1_790_158_269_000;

fn client(server: &MockServer, max_retries: u32) -> (ApiClient, Rc<FakeClock>, SharedBuf) {
    client_with_timeout(server, max_retries, Duration::from_secs(5))
}

fn client_with_timeout(
    server: &MockServer,
    max_retries: u32,
    timeout: Duration,
) -> (ApiClient, Rc<FakeClock>, SharedBuf) {
    client_at(server.base_url(), max_retries, timeout)
}

fn client_at(
    base_url: String,
    max_retries: u32,
    timeout: Duration,
) -> (ApiClient, Rc<FakeClock>, SharedBuf) {
    let clock = Rc::new(FakeClock::new(NOW));
    let log = SharedBuf::default();
    let reporter = Rc::new(Reporter::with_writer(
        false,
        true,
        false,
        false,
        Box::new(log.clone()),
    ));
    let settings = HttpSettings {
        base_url: base_url.clone(),
        timeout,
        max_retries,
    };
    let ctx = ErrorContext {
        token_source: "APLAUT_ACCESS_TOKEN".into(),
        base_url,
    };
    (
        ApiClient::new(settings, Secret::new(TOKEN), ctx, clock.clone(), reporter),
        clock,
        log,
    )
}

fn ms(d: &Duration) -> u128 {
    d.as_millis()
}

#[test]
fn sends_auth_accept_user_agent_and_encoded_query() {
    let server = MockServer::start(vec![Reply::json(200, "{}")]);
    let (mut api, _, log) = client(&server, 0);
    let cursor = "eyJ2IjoxfQ==+/x";
    let resp = api
        .get("/scroll/reviews", &[("cursor", cursor)], Pace::Scroll)
        .unwrap();
    assert_eq!(resp.status, 200);
    let req = &server.requests()[0];
    assert_eq!(
        (req.method.as_str(), req.path.as_str()),
        ("GET", "/v4/scroll/reviews")
    );
    assert_eq!(
        req.header("authorization"),
        Some(format!("Bearer {TOKEN}").as_str())
    );
    assert_eq!(req.header("accept"), Some("application/vnd.api+json"));
    assert!(req.header("user-agent").unwrap().starts_with("aplaut-cli/"));
    assert_eq!(req.query_param("cursor"), Some(cursor));
    let log = log.contents();
    assert!(log.contains("Bearer ***") && !log.contains(TOKEN), "{log}");
    assert!(log.contains("cursor=<15 символов>"), "{log}");
}

#[test]
fn retries_429_until_reset_measured_by_server_date() {
    let server = MockServer::start(vec![
        Reply::text(429, "Throttled\n")
            .with_header("Date", "Wed, 23 Sep 2026 10:11:09 GMT")
            .with_header("X-RateLimit-Limit", "1")
            .with_header("X-RateLimit-Period", "2")
            .with_header("X-RateLimit-Reset", "2026-09-23 10:11:10 +0000")
            .with_header("Retry-After", "1"),
        Reply::json(200, "{}"),
    ]);
    let (mut api, clock, _) = client(&server, 3);
    api.get("/scroll/reviews", &[], Pace::Default).unwrap();
    let sleeps = clock.sleeps();
    assert_eq!(sleeps.len(), 1, "{sleeps:?}");
    assert!((1000..=1250).contains(&ms(&sleeps[0])), "{sleeps:?}");
}

#[test]
fn reset_without_date_uses_local_clock_and_larger_retry_after_wins() {
    let server = MockServer::start(vec![
        Reply::text(429, "").with_header("X-RateLimit-Reset", "2026-09-23T10:11:12+00:00"),
        Reply::text(429, "")
            .with_header("Date", "Wed, 23 Sep 2026 10:11:12 GMT")
            .with_header("X-RateLimit-Reset", "2026-09-23 10:11:13 +0000")
            .with_header("Retry-After", "5"),
        Reply::json(200, "{}"),
    ]);
    let (mut api, clock, _) = client(&server, 3);
    api.get("/x", &[], Pace::Default).unwrap();
    let sleeps = clock.sleeps();
    assert!((3000..=3250).contains(&ms(&sleeps[0])), "{sleeps:?}");
    assert!((5000..=5250).contains(&ms(&sleeps[1])), "{sleeps:?}");
}

#[test]
fn too_long_rate_limit_wait_fails_fast() {
    let server = MockServer::start(vec![Reply::text(429, "").with_header("Retry-After", "3600")]);
    let (mut api, clock, _) = client(&server, 3);
    let err = api.get("/x", &[], Pace::Default).unwrap_err();
    assert_eq!(
        (err.code.as_str(), err.exit),
        ("rate_limited", Exit::RateLimited)
    );
    assert!(clock.sleeps().is_empty());
}

/// Агенту — сколько ждать по словам сервера, чтобы не повторять раньше времени.
#[test]
fn rate_limit_errors_say_how_long_to_wait() {
    let server = MockServer::start(vec![Reply::text(429, "").with_header("Retry-After", "3600")]);
    let (mut api, _, _) = client(&server, 3);
    let fast = api.get("/x", &[], Pace::Default).unwrap_err();
    assert_eq!(fast.retry_after, Some(3600), "отказ сразу");
    let server = MockServer::start(vec![Reply::text(429, "").with_header("Retry-After", "30")]);
    let (mut api, _, _) = client(&server, 0);
    let exhausted = api.get("/x", &[], Pace::Default).unwrap_err();
    assert_eq!(exhausted.retry_after, Some(30), "повторы исчерпаны");
    let server = MockServer::start(vec![Reply::text(503, "").with_header("Retry-After", "600")]);
    let (mut api, _, _) = client(&server, 3);
    let err = api.get("/x", &[], Pace::Default).unwrap_err();
    assert_eq!(
        (err.code.as_str(), err.retry_after),
        ("server_error", Some(600))
    );
    let server = MockServer::start(vec![Reply::text(404, "")]);
    let (mut api, _, _) = client(&server, 0);
    assert_eq!(
        api.get("/x", &[], Pace::Default).unwrap_err().retry_after,
        None
    );
}

#[test]
fn retries_503_with_retry_after_and_5xx_with_backoff() {
    let server = MockServer::start(vec![
        Reply::text(503, "").with_header("Retry-After", "2"),
        Reply::text(500, "oops"),
        Reply::text(502, "oops"),
        Reply::json(200, "{}"),
    ]);
    let (mut api, clock, _) = client(&server, 6);
    api.get("/x", &[], Pace::Default).unwrap();
    // Первая пауза — ровно Retry-After; дальше backoff (≤ 2 с и ≤ 4 с) и, если backoff
    // короче 500 мс, ещё пауза троттлинга — поэтому проверяем границы, а не позиции.
    let sleeps = clock.sleeps();
    assert_eq!(ms(&sleeps[0]), 2000);
    assert!(
        sleeps.len() >= 3 && sleeps.iter().all(|d| ms(d) <= 4000),
        "{sleeps:?}"
    );
    assert_eq!(server.requests().len(), 4);
}

#[test]
fn gives_up_after_max_retries() {
    let server = MockServer::start(vec![
        Reply::text(429, "").with_header("Retry-After", "1"),
        Reply::text(429, "").with_header("Retry-After", "1"),
        Reply::text(429, "").with_header("Retry-After", "1"),
    ]);
    let (mut api, _, _) = client(&server, 2);
    let err = api.get("/x", &[], Pace::Default).unwrap_err();
    assert_eq!(
        (err.code.as_str(), err.exit, err.retryable),
        ("rate_limited", Exit::RateLimited, true)
    );
    assert_eq!(server.requests().len(), 3);
}

#[test]
fn does_not_retry_4xx_and_redacts_token_from_server_text() {
    let body = format!(
        r#"{{"errors":{{"status":422,"title":"Invalid query params","details":{{"filter":["bad {TOKEN}"]}}}}}}"#
    );
    let server = MockServer::start(vec![Reply::json(422, body)]);
    let (mut api, clock, _) = client(&server, 6);
    let err = api.get("/x", &[], Pace::Default).unwrap_err();
    assert_eq!(err.code, "validation_failed");
    assert!(
        !err.message.contains(TOKEN) && err.message.contains("***"),
        "{}",
        err.message
    );
    assert_eq!(server.requests().len(), 1);
    assert!(clock.sleeps().is_empty());
}

#[test]
fn maps_observed_401() {
    let server = MockServer::start(vec![Reply::text(401, "")
        .with_header("WWW-Authenticate", r#"Bearer realm="Doorkeeper", error="invalid_token", error_description="The access token is invalid""#)
        .with_header("x-request-id", "fbfe30c0")]);
    let (mut api, _, _) = client(&server, 6);
    let err = api.get("/x", &[], Pace::Default).unwrap_err();
    assert_eq!((err.code.as_str(), err.exit), ("invalid_token", Exit::Auth));
    assert_eq!(err.request_id.as_deref(), Some("fbfe30c0"));
}

#[test]
fn retries_dropped_connection() {
    let server = MockServer::start(vec![Reply::Hangup, Reply::json(200, "{}")]);
    let (mut api, clock, _) = client(&server, 2);
    api.get("/x", &[], Pace::Default).unwrap();
    assert!(!clock.sleeps().is_empty());
    assert_eq!(server.requests().len(), 2);
}

#[test]
fn network_failure_after_retries_is_retryable_error() {
    let server = MockServer::start(vec![Reply::Hangup, Reply::Hangup]);
    let (mut api, _, _) = client(&server, 1);
    let err = api.get("/x", &[], Pace::Default).unwrap_err();
    assert_eq!(
        (err.code.as_str(), err.exit, err.retryable),
        ("network_error", Exit::General, true)
    );
    assert!(!err.message.contains(TOKEN));
}

#[test]
fn paces_scroll_and_default_requests() {
    let server = MockServer::start(vec![
        Reply::json(200, "{}"),
        Reply::json(200, "{}"),
        Reply::json(200, "{}"),
        Reply::json(200, "{}"),
    ]);
    let (mut api, clock, _) = client(&server, 0);
    api.get("/scroll/reviews", &[], Pace::Scroll).unwrap();
    api.get("/scroll/reviews", &[], Pace::Scroll).unwrap();
    api.get("/surveys", &[], Pace::Default).unwrap();
    api.get("/scroll/reviews", &[], Pace::Scroll).unwrap();
    let sleeps: Vec<u128> = clock.sleeps().iter().map(ms).collect();
    assert_eq!(sleeps, vec![2200, 500, 1700]);
}

#[test]
fn redirect_is_not_followed() {
    let server = MockServer::start(vec![
        Reply::text(302, "").with_header("Location", "https://example.com/login")
    ]);
    let (mut api, _, _) = client(&server, 3);
    let err = api.get("/x", &[], Pace::Default).unwrap_err();
    assert_eq!(err.code, "unexpected_redirect");
    assert_eq!(server.requests().len(), 1);
}

#[test]
fn long_503_retry_after_fails_fast_instead_of_sleeping_for_hours() {
    let server = MockServer::start(vec![
        Reply::text(503, "maintenance").with_header("Retry-After", "7200")
    ]);
    let (mut api, clock, _) = client(&server, 6);
    let err = api.get("/x", &[], Pace::Default).unwrap_err();
    assert_eq!(
        (err.code.as_str(), err.exit),
        ("server_error", Exit::General)
    );
    assert!(err.retryable);
    assert!(
        err.hint.as_deref().unwrap_or_default().contains("7200"),
        "{err:?}"
    );
    assert!(clock.sleeps().is_empty(), "{:?}", clock.sleeps());
    assert_eq!(server.requests().len(), 1);
}

// Курсор scroll не идемпотентен (стейджинг, 2026-09-23): повтор тем же курсором отдаёт
// следующую страницу. Продолжение можно повторять, только если сервер его точно не обработал.

#[test]
fn continuation_is_not_replayed_after_dropped_connection() {
    let server = MockServer::start(vec![Reply::Hangup, Reply::json(200, "{}")]);
    let (mut api, _, _) = client(&server, 6);
    let err = api
        .get_continuation("/scroll/reviews", &[("cursor", "c1")])
        .unwrap_err();
    assert_eq!(err.code, aplaut_cli::http::OUTCOME_UNKNOWN);
    assert!(!err.retryable);
    assert_eq!(server.requests().len(), 1);
}

#[test]
fn continuation_is_not_replayed_after_500() {
    let server = MockServer::start(vec![Reply::text(500, "oops"), Reply::json(200, "{}")]);
    let (mut api, _, _) = client(&server, 6);
    let err = api
        .get_continuation("/scroll/reviews", &[("cursor", "c1")])
        .unwrap_err();
    assert_eq!(err.code, aplaut_cli::http::OUTCOME_UNKNOWN);
    assert_eq!(server.requests().len(), 1);
}

#[test]
fn continuation_is_replayed_after_429_and_503() {
    let server = MockServer::start(vec![
        Reply::text(429, "Throttled\n").with_header("Retry-After", "1"),
        Reply::text(503, "").with_header("Retry-After", "1"),
        Reply::json(200, "{}"),
    ]);
    let (mut api, _, _) = client(&server, 6);
    api.get_continuation("/scroll/reviews", &[("cursor", "c1")])
        .unwrap();
    assert_eq!(server.requests().len(), 3);
    assert!(server
        .requests()
        .iter()
        .all(|r| r.query_param("cursor") == Some("c1")));
}

// Запись не идемпотентна: повтор POST, который сервер мог обработать, создал бы дубль.

fn review_doc() -> serde_json::Value {
    serde_json::json!({"data": {"type": "reviews", "attributes": {"rating": 5}}})
}

#[test]
fn post_sends_json_body_with_content_type_and_logs_it_masked() {
    let server = MockServer::start(vec![Reply::json(
        201,
        r#"{"data":{"id":"c1","type":"comments"}}"#,
    )]);
    let (mut api, _, log) = client(&server, 0);
    let body = serde_json::json!({"data": {"type": "comments", "attributes": {"text": "Спасибо, \"друг\"\nи до встречи"}}});
    let resp = api
        .post("/reviews/r1/relationships/comments", &body)
        .unwrap();
    assert_eq!(resp.status, 201);
    let req = &server.requests()[0];
    assert_eq!(
        (req.method.as_str(), req.path.as_str()),
        ("POST", "/v4/reviews/r1/relationships/comments")
    );
    assert_eq!(req.header("content-type"), Some("application/json"));
    assert_eq!(req.header("accept"), Some("application/vnd.api+json"));
    assert_eq!(
        req.header("authorization"),
        Some(format!("Bearer {TOKEN}").as_str())
    );
    assert_eq!(
        req.json(),
        body,
        "тело — ровно этот документ, UTF-8 без искажений"
    );
    let log = log.contents();
    assert!(
        log.contains("→ POST") && log.contains("Content-Type: application/json"),
        "{log}"
    );
    assert!(log.contains("body: {") && log.contains("Спасибо"), "{log}");
    assert!(!log.contains(TOKEN), "{log}");
}

#[test]
fn post_is_replayed_only_after_429_and_503() {
    let server = MockServer::start(vec![
        Reply::text(429, "Throttled\n").with_header("Retry-After", "1"),
        Reply::text(503, "").with_header("Retry-After", "1"),
        Reply::json(201, r#"{"data":{"id":"r1","type":"reviews"}}"#),
    ]);
    let (mut api, _, _) = client(&server, 6);
    api.post("/reviews", &review_doc()).unwrap();
    let requests = server.requests();
    assert_eq!(requests.len(), 3);
    assert!(requests.iter().all(|r| r.json() == review_doc()));
}

#[test]
fn post_is_not_replayed_when_the_server_may_have_processed_it() {
    for (reply, what) in [
        (Reply::text(500, "oops"), "500"),
        (Reply::text(502, "bad gateway"), "502"),
        (Reply::Hangup, "обрыв после отправки"),
    ] {
        let server = MockServer::start(vec![reply, Reply::json(201, "{}")]);
        let (mut api, _, _) = client(&server, 6);
        let err = api.post("/reviews", &review_doc()).unwrap_err();
        assert_eq!(err.code, aplaut_cli::http::OUTCOME_UNKNOWN, "{what}");
        assert!(!err.retryable, "{what}");
        assert_eq!(server.requests().len(), 1, "{what}");
    }
}

#[test]
fn post_timeout_is_outcome_unknown_without_replay() {
    let server = MockServer::start(vec![
        Reply::Stall(Duration::from_millis(800)),
        Reply::json(201, "{}"),
    ]);
    let (mut api, _, _) = client_with_timeout(&server, 6, Duration::from_millis(200));
    let err = api.post("/reviews", &review_doc()).unwrap_err();
    assert_eq!(err.code, aplaut_cli::http::OUTCOME_UNKNOWN);
    assert!(err.message.contains("неизвестно"), "{}", err.message);
    assert_eq!(server.requests().len(), 1);
}

#[test]
fn post_422_is_validation_failed_with_field() {
    let server = MockServer::start(vec![Reply::json(
        422,
        r#"{"errors":{"status":422,"title":"Validation failed","details":{"rating":["must be less than or equal to 5"]}}}"#,
    )]);
    let (mut api, _, _) = client(&server, 6);
    let err = api.post("/reviews", &review_doc()).unwrap_err();
    assert_eq!(
        (err.code.as_str(), err.field.as_deref(), err.exit),
        ("validation_failed", Some("rating"), Exit::General)
    );
    assert_eq!(server.requests().len(), 1);
}

/// Отказ в соединении — запрос точно не ушёл (W7): POST и продолжение обхода повторяются, а
/// итог — `network_error`, а не «исход неизвестен».
#[test]
fn refused_connection_is_never_sent_so_it_is_retried() {
    let port = {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.local_addr().unwrap().port()
    };
    let base_url = format!("http://127.0.0.1:{port}/v4");
    let (mut api, clock, _) = client_at(base_url.clone(), 2, Duration::from_secs(5));
    let err = api.post("/reviews", &review_doc()).unwrap_err();
    assert_eq!(
        (err.code.as_str(), err.retryable),
        ("network_error", true),
        "{err:?}"
    );
    assert!(!clock.sleeps().is_empty(), "были повторы");
    let (mut api, _, _) = client_at(base_url, 1, Duration::from_secs(5));
    let err = api
        .get_continuation("/scroll/reviews", &[("cursor", "c1")])
        .unwrap_err();
    assert_eq!(err.code, "network_error", "{err:?}");
}

/// PUT идемпотентен: повтор того же тела даёт то же состояние (спека products-write P7).
#[test]
fn put_sends_the_body_and_is_replayed_like_get() {
    let server = MockServer::start(vec![
        Reply::text(500, "oops"),
        Reply::json(200, r#"{"data":{"id":"p1","type":"products"}}"#),
    ]);
    let (mut api, _, _) = client(&server, 6);
    let body = serde_json::json!({"data": {"type": "products", "attributes": {"price": 20}}});
    api.write(Method::Put, "/products/p1", &body, Replay::Safe)
        .unwrap();
    let requests = server.requests();
    assert_eq!(requests.len(), 2, "повтор после 500");
    assert!(requests
        .iter()
        .all(|r| r.method == "PUT" && r.path == "/v4/products/p1" && r.json() == body));
    assert_eq!(requests[0].header("content-type"), Some("application/json"));
}

#[test]
fn put_that_may_address_another_object_is_not_replayed() {
    let server = MockServer::start(vec![Reply::text(500, "oops"), Reply::json(200, "{}")]);
    let (mut api, _, _) = client(&server, 6);
    let err = api
        .write(
            Method::Put,
            "/products/old",
            &review_doc(),
            Replay::OnlyIfUnprocessed,
        )
        .unwrap_err();
    assert_eq!(err.code, aplaut_cli::http::OUTCOME_UNKNOWN);
    assert_eq!(server.requests().len(), 1);
}
