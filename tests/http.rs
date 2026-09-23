mod support;

use std::rc::Rc;
use std::time::Duration;

use aplaut_cli::api_error::ErrorContext;
use aplaut_cli::clock::FakeClock;
use aplaut_cli::error::Exit;
use aplaut_cli::http::{ApiClient, HttpSettings, Pace};
use aplaut_cli::secret::Secret;
use aplaut_cli::term::{Reporter, SharedBuf};
use support::{MockServer, Reply};

const TOKEN: &str = "tok-secret-123";
/// 2026-09-23T10:11:09Z — совпадает с `Date` в ответах ниже.
const NOW: i64 = 1_790_158_269_000;

fn client(server: &MockServer, max_retries: u32) -> (ApiClient, Rc<FakeClock>, SharedBuf) {
    let clock = Rc::new(FakeClock::new(NOW));
    let log = SharedBuf::default();
    let reporter = Rc::new(Reporter::with_writer(false, true, false, false, Box::new(log.clone())));
    let settings = HttpSettings { base_url: server.base_url(), timeout: Duration::from_secs(5), max_retries };
    let ctx = ErrorContext { token_source: "APLAUT_ACCESS_TOKEN".into(), base_url: server.base_url() };
    (ApiClient::new(settings, Secret::new(TOKEN), ctx, clock.clone(), reporter), clock, log)
}

fn ms(d: &Duration) -> u128 {
    d.as_millis()
}

#[test]
fn sends_auth_accept_user_agent_and_encoded_query() {
    let server = MockServer::start(vec![Reply::json(200, "{}")]);
    let (mut api, _, log) = client(&server, 0);
    let cursor = "eyJ2IjoxfQ==+/x";
    let resp = api.get("/scroll/reviews", &[("cursor", cursor)], Pace::Scroll).unwrap();
    assert_eq!(resp.status, 200);
    let req = &server.requests()[0];
    assert_eq!((req.method.as_str(), req.path.as_str()), ("GET", "/v4/scroll/reviews"));
    assert_eq!(req.header("authorization"), Some(format!("Bearer {TOKEN}").as_str()));
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
    let server = MockServer::start(vec![
        Reply::text(429, "").with_header("Retry-After", "3600"),
    ]);
    let (mut api, clock, _) = client(&server, 3);
    let err = api.get("/x", &[], Pace::Default).unwrap_err();
    assert_eq!((err.code.as_str(), err.exit), ("rate_limited", Exit::RateLimited));
    assert!(clock.sleeps().is_empty());
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
    assert!(sleeps.len() >= 3 && sleeps.iter().all(|d| ms(d) <= 4000), "{sleeps:?}");
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
    assert_eq!((err.code.as_str(), err.exit, err.retryable), ("rate_limited", Exit::RateLimited, true));
    assert_eq!(server.requests().len(), 3);
}

#[test]
fn does_not_retry_4xx_and_redacts_token_from_server_text() {
    let body = format!(r#"{{"errors":{{"status":422,"title":"Invalid query params","details":{{"filter":["bad {TOKEN}"]}}}}}}"#);
    let server = MockServer::start(vec![Reply::json(422, body)]);
    let (mut api, clock, _) = client(&server, 6);
    let err = api.get("/x", &[], Pace::Default).unwrap_err();
    assert_eq!(err.code, "validation_failed");
    assert!(!err.message.contains(TOKEN) && err.message.contains("***"), "{}", err.message);
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
    assert_eq!((err.code.as_str(), err.exit, err.retryable), ("network_error", Exit::General, true));
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
    let server = MockServer::start(vec![Reply::text(302, "").with_header("Location", "https://example.com/login")]);
    let (mut api, _, _) = client(&server, 3);
    let err = api.get("/x", &[], Pace::Default).unwrap_err();
    assert_eq!(err.code, "unexpected_redirect");
    assert_eq!(server.requests().len(), 1);
}
