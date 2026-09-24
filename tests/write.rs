//! `reviews create` и `reviews comment` (спека reviews-write §3–5): тело запроса, повторы,
//! план под `-n`, ошибки до сети.

mod support;

use std::time::Duration;

use serde_json::{json, Value};
use support::{aplaut, MockServer, Output, Reply, TempDir};

const TOKEN: (&str, &str) = ("APLAUT_ACCESS_TOKEN", "tok");

fn created(kind: &str, id: &str) -> Reply {
    Reply::json(
        201,
        json!({"data": {"id": id, "type": kind, "attributes": {"state": "waiting"}}}).to_string(),
    )
}

fn run(server: &MockServer, args: &[&str], stdin: &str) -> Output {
    let home = TempDir::new("write");
    let url = server.base_url();
    let mut full = args.to_vec();
    full.extend(["--base-url", url.as_str()]);
    aplaut(home.path(), &full, &[TOKEN], stdin)
}

/// Успех с `--json`: конверт — единственная строка stdout.
fn envelope(out: &Output) -> Value {
    assert_eq!(out.code, 0, "{}", out.stderr);
    serde_json::from_str(out.stdout.trim_end()).unwrap_or_else(|e| panic!("{e}: {}", out.stdout))
}

/// Ошибка во входных данных: код 2, поле названо, запросов нет.
fn local_error(server: &MockServer, out: &Output, code: &str, field: &str) {
    assert_eq!(out.code, 2, "{}", out.stderr);
    let err = out.error_json();
    assert_eq!(
        (
            err["error"]["code"].as_str(),
            err["error"]["field"].as_str()
        ),
        (Some(code), Some(field)),
        "{err}"
    );
    assert!(server.requests().is_empty(), "ошибка ловится до сети");
}

#[test]
fn create_sends_flags_over_data_and_reports_request_and_created() {
    let home = TempDir::new("write-data");
    let data = home.path().join("review.json");
    std::fs::write(
        &data,
        r#"{"rating": 3, "body": "из файла", "photos": ["https://x/1.jpg"]}"#,
    )
    .unwrap();
    let server = MockServer::start(vec![created("reviews", "r1")]);
    let out = run(
        &server,
        &[
            "reviews",
            "create",
            "--data",
            data.to_str().unwrap(),
            "--rating",
            "5",
            "--pros",
            "- быстро\n- \"вежливо\"",
            "--external-id",
            "crm-1",
            "--json",
        ],
        "",
    );
    let v = envelope(&out);
    let expected = json!({"data": {"type": "reviews", "attributes": {
        "rating": 5, "body": "из файла", "photos": ["https://x/1.jpg"],
        "pros": "- быстро\n- \"вежливо\"", "external_id": "crm-1"
    }}});
    let req = &server.requests()[0];
    assert_eq!(
        (req.method.as_str(), req.path.as_str()),
        ("POST", "/v4/reviews")
    );
    assert_eq!(req.header("content-type"), Some("application/json"));
    assert_eq!(
        req.json(),
        expected,
        "флаги перекрывают --data; текст — байт в байт"
    );
    assert_eq!(
        (v["command"].as_str(), v["dry_run"].as_bool()),
        (Some("reviews.create"), Some(false))
    );
    assert_eq!(
        v["result"]["request"],
        json!({"method": "POST", "path": "/reviews", "body": expected})
    );
    assert_eq!(v["result"]["created"]["id"], "r1");
}

#[test]
fn create_reads_data_from_stdin() {
    let server = MockServer::start(vec![created("reviews", "r1")]);
    let out = run(
        &server,
        &["reviews", "create", "--data", "-", "--json"],
        r#"{"rating": 4, "body": "из stdin", "tags": ["Featured"]}"#,
    );
    envelope(&out);
    assert_eq!(
        server.requests()[0].json()["data"]["attributes"],
        json!({"rating": 4, "body": "из stdin", "tags": ["Featured"]})
    );
}

#[test]
fn dry_run_shows_the_exact_request_and_sends_nothing() {
    let server = MockServer::start(vec![]);
    let args = [
        "reviews",
        "create",
        "--rating",
        "5",
        "--body",
        "Спасибо!",
        "-n",
    ];
    let mut with_json = args.to_vec();
    with_json.push("--json");
    let v = envelope(&run(&server, &with_json, ""));
    assert_eq!(v["dry_run"], true);
    assert_eq!(
        v["result"],
        json!({"request": {"method": "POST", "path": "/reviews", "body": {"data": {
            "type": "reviews", "attributes": {"rating": 5, "body": "Спасибо!"}}}},
            "created": null})
    );
    let text = run(&server, &args, "");
    assert_eq!(text.code, 0, "{}", text.stderr);
    assert!(
        text.stderr
            .starts_with("Пробный запуск, ничего не изменено: POST /reviews\n{\n"),
        "{}",
        text.stderr
    );
    assert!(
        text.stderr.contains("\"body\": \"Спасибо!\""),
        "{}",
        text.stderr
    );
    assert!(server.requests().is_empty());
}

#[test]
fn text_mode_reports_the_new_id() {
    let server = MockServer::start(vec![created("reviews", "r1")]);
    let out = run(
        &server,
        &["reviews", "create", "--rating", "5", "--body", "ok"],
        "",
    );
    assert_eq!(out.code, 0, "{}", out.stderr);
    assert_eq!(out.stderr, "Отзыв создан: id r1\n");
    assert_eq!(out.stdout, "");
}

#[test]
fn create_is_replayed_after_503() {
    let server = MockServer::start(vec![
        Reply::text(503, "").with_header("Retry-After", "0"),
        created("reviews", "r1"),
    ]);
    let out = run(
        &server,
        &[
            "reviews", "create", "--rating", "5", "--body", "ok", "--json",
        ],
        "",
    );
    assert_eq!(envelope(&out)["result"]["created"]["id"], "r1");
    assert_eq!(server.requests().len(), 2);
}

#[test]
fn unknown_outcome_is_not_replayed_and_says_how_to_check() {
    let server = MockServer::start(vec![Reply::text(500, "oops"), created("reviews", "r1")]);
    let out = run(
        &server,
        &[
            "reviews",
            "create",
            "--rating",
            "5",
            "--body",
            "ok",
            "--external-id",
            "crm-1",
        ],
        "",
    );
    assert_eq!(out.code, 1, "{}", out.stderr);
    let err = out.error_json();
    assert_eq!(
        (
            err["error"]["code"].as_str(),
            err["error"]["retryable"].as_bool()
        ),
        (Some("request_outcome_unknown"), Some(false))
    );
    assert!(
        err["error"]["hint"]
            .as_str()
            .unwrap()
            .contains("aplaut reviews get crm-1"),
        "{err}"
    );
    assert_eq!(server.requests().len(), 1);
}

#[test]
fn timeout_without_external_id_suggests_passing_one() {
    let server = MockServer::start(vec![Reply::Stall(Duration::from_secs(2))]);
    let out = run(
        &server,
        &[
            "reviews",
            "create",
            "--rating",
            "5",
            "--body",
            "ok",
            "--timeout",
            "1",
        ],
        "",
    );
    assert_eq!(out.code, 1, "{}", out.stderr);
    let err = out.error_json();
    assert_eq!(err["error"]["code"], "request_outcome_unknown");
    assert!(
        err["error"]["hint"]
            .as_str()
            .unwrap()
            .contains("--external-id"),
        "{err}"
    );
    assert_eq!(server.requests().len(), 1);
}

#[test]
fn server_validation_error_names_the_field() {
    let server = MockServer::start(vec![Reply::json(
        422,
        r#"{"errors":{"status":422,"title":"Validation failed","details":{"product_id":["not found"]}}}"#,
    )]);
    let out = run(
        &server,
        &[
            "reviews",
            "create",
            "--rating",
            "5",
            "--body",
            "ok",
            "--product-id",
            "nope",
        ],
        "",
    );
    assert_eq!(out.code, 1, "{}", out.stderr);
    let err = out.error_json();
    assert_eq!(
        (
            err["error"]["code"].as_str(),
            err["error"]["field"].as_str()
        ),
        (Some("validation_failed"), Some("product_id"))
    );
}

/// 2xx без записи в теле: запрос выполнен, поэтому не «повторите», а «проверьте».
#[test]
fn created_without_a_record_is_not_reported_as_retryable() {
    let server = MockServer::start(vec![Reply::text(201, "")]);
    let out = run(
        &server,
        &[
            "reviews",
            "create",
            "--rating",
            "5",
            "--body",
            "ok",
            "--external-id",
            "crm-1",
        ],
        "",
    );
    assert_eq!(out.code, 1, "{}", out.stderr);
    let err = out.error_json();
    assert_eq!(
        (
            err["error"]["code"].as_str(),
            err["error"]["retryable"].as_bool()
        ),
        (Some("bad_response"), Some(false))
    );
    assert!(
        err["error"]["hint"]
            .as_str()
            .unwrap()
            .contains("aplaut reviews get crm-1"),
        "{err}"
    );
}

#[test]
fn input_errors_are_caught_before_the_network() {
    let server = MockServer::start(vec![]);
    let cases: [(&[&str], &str, &str, &str); 8] = [
        (
            &["--rating", "7", "--body", "ok"],
            "",
            "invalid_attribute",
            "rating",
        ),
        (
            &["--rating", "five", "--body", "ok"],
            "",
            "invalid_attribute",
            "rating",
        ),
        (
            &["--rating", "5", "--body", "ok", "--state", "live"],
            "",
            "invalid_attribute",
            "state",
        ),
        (&["--rating", "5"], "", "missing_attribute", "body"),
        (
            &["--data", "-"],
            r#"{"raiting": 5, "body": "ok"}"#,
            "unknown_attribute",
            "raiting",
        ),
        (
            &["--data", "-"],
            r#"{"data": {"type": "reviews", "attributes": {}}}"#,
            "invalid_data",
            "data",
        ),
        (&["--data", "-"], "rating=5", "invalid_data", "data"),
        (
            &["--data", "-", "--token-stdin"],
            "tok",
            "stdin_conflict",
            "data",
        ),
    ];
    for (extra, stdin, code, field) in cases {
        let mut args = vec!["reviews", "create"];
        args.extend_from_slice(extra);
        local_error(&server, &run(&server, &args, stdin), code, field);
    }
}

/// Стейджинг, 2026-09-24: схема требует body, а сервер — rating и любой из текстов.
#[test]
fn any_of_the_texts_is_enough() {
    let server = MockServer::start(vec![created("reviews", "r1"), created("reviews", "r2")]);
    for text in ["--pros", "--cons"] {
        let out = run(
            &server,
            &["reviews", "create", "--rating", "4", text, "Тест", "--json"],
            "",
        );
        envelope(&out);
    }
    assert_eq!(server.requests().len(), 2);
    let none = MockServer::start(vec![]);
    let out = run(&none, &["reviews", "create", "--rating", "4"], "");
    local_error(&none, &out, "missing_attribute", "body");
    let hint = out.error_json()["error"]["hint"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(hint.contains("--pros") && hint.contains("--cons"), "{hint}");
}

#[test]
fn failed_plan_is_marked_dry_run() {
    let server = MockServer::start(vec![]);
    let out = run(&server, &["reviews", "create", "--rating", "5", "-n"], "");
    local_error(&server, &out, "missing_attribute", "body");
    assert_eq!(out.error_json()["dry_run"], true);
}

#[test]
fn comment_posts_to_the_encoded_review_path() {
    let server = MockServer::start(vec![created("comments", "c1")]);
    let out = run(
        &server,
        &[
            "reviews",
            "comment",
            "crm/42 а",
            "--text",
            "Спасибо за отзыв!",
            "--state",
            "published",
            "--json",
        ],
        "",
    );
    let v = envelope(&out);
    let req = &server.requests()[0];
    assert_eq!(
        (req.method.as_str(), req.path.as_str()),
        (
            "POST",
            "/v4/reviews/crm%2F42%20%D0%B0/relationships/comments"
        )
    );
    assert_eq!(
        req.json(),
        json!({"data": {"type": "comments", "attributes": {"text": "Спасибо за отзыв!", "state": "published"}}})
    );
    assert_eq!(v["command"], "reviews.comment");
    assert_eq!(
        v["result"]["request"]["path"],
        "/reviews/crm%2F42%20%D0%B0/relationships/comments"
    );
    assert_eq!(v["result"]["created"]["id"], "c1");
}

#[test]
fn comment_on_a_missing_review_is_exit_5() {
    let server = MockServer::start(vec![Reply::json(
        404,
        r#"{"errors":{"status":404,"title":"Not found"}}"#,
    )]);
    let out = run(
        &server,
        &["reviews", "comment", "nope", "--text", "Спасибо!"],
        "",
    );
    assert_eq!(out.code, 5, "{}", out.stderr);
    let err = out.error_json();
    assert_eq!(
        (err["command"].as_str(), err["error"]["code"].as_str()),
        (Some("reviews.comment"), Some("not_found"))
    );
}

#[test]
fn comment_input_is_checked_before_the_network() {
    let server = MockServer::start(vec![]);
    let missing = run(&server, &["reviews", "comment", "r1"], "");
    local_error(&server, &missing, "missing_attribute", "text");
    let held = run(
        &server,
        &["reviews", "comment", "r1", "--text", "x", "--state", "held"],
        "",
    );
    local_error(&server, &held, "invalid_attribute", "state");
    let empty = run(&server, &["reviews", "comment", "", "--text", "x"], "");
    local_error(&server, &empty, "invalid_id", "review_id");
    let plan = run(&server, &["reviews", "comment", "r1", "-n"], "");
    local_error(&server, &plan, "missing_attribute", "text");
    assert_eq!(plan.error_json()["dry_run"], true);
}

#[test]
fn comment_dry_run_then_text_mode() {
    let server = MockServer::start(vec![created("comments", "c1")]);
    let plan = run(
        &server,
        &["reviews", "comment", "r1", "--text", "Спасибо!", "-n"],
        "",
    );
    assert_eq!(plan.code, 0, "{}", plan.stderr);
    assert!(
        plan.stderr.starts_with(
            "Пробный запуск, ничего не изменено: POST /reviews/r1/relationships/comments\n"
        ),
        "{}",
        plan.stderr
    );
    assert!(server.requests().is_empty());
    let out = run(
        &server,
        &["reviews", "comment", "r1", "--text", "Спасибо!"],
        "",
    );
    assert_eq!(out.code, 0, "{}", out.stderr);
    assert_eq!(out.stderr, "Комментарий добавлен к отзыву r1: id c1\n");
}

#[test]
fn comment_unknown_outcome_points_to_the_review_comments() {
    let server = MockServer::start(vec![Reply::Hangup]);
    let out = run(
        &server,
        &[
            "reviews",
            "comment",
            "r1",
            "--text",
            "Спасибо!",
            "--external-id",
            "reply-1",
        ],
        "",
    );
    assert_eq!(out.code, 1, "{}", out.stderr);
    let err = out.error_json();
    assert_eq!(err["error"]["code"], "request_outcome_unknown");
    let hint = err["error"]["hint"].as_str().unwrap();
    assert!(
        hint.contains("aplaut reviews get r1 --include comments") && hint.contains("reply-1"),
        "{hint}"
    );
    assert_eq!(server.requests().len(), 1);
}
