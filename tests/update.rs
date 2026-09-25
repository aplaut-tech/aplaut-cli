//! Общий `update` с проверкой наличия (спека writes-and-exports R3–R6) на примере `reviews update`:
//! GET перед PUT, `--upsert`, `created`, план под `-n`, повторы, ошибки до сети.

mod support;

use serde_json::{json, Value};
use support::{envelope, local_error, run, MockServer, Reply};

fn review(status: u16, id: &str) -> Reply {
    Reply::json(
        status,
        json!({"data": {"id": id, "type": "reviews", "attributes": {"state": "banned", "external_id": "crm-1"}}})
            .to_string(),
    )
}

fn missing() -> Reply {
    Reply::json(
        404,
        r#"{"errors":{"status":404,"title":"Document not found","details":{"message":"Document(s) not found for class Review with id(s) crm-1."}}}"#,
    )
}

const UPDATE: [&str; 5] = ["reviews", "update", "crm-1", "--state", "banned"];

fn update_with(extra: &[&'static str]) -> Vec<&'static str> {
    let mut args = UPDATE.to_vec();
    args.extend_from_slice(extra);
    args
}

#[test]
fn update_checks_the_review_exists_then_sends_only_the_given_attributes() {
    let server = MockServer::start(vec![review(200, "r1"), review(200, "r1")]);
    let v = envelope(&run(&server, &update_with(&["--json"]), ""));
    let requests = server.requests();
    let calls: Vec<(&str, &str)> = requests
        .iter()
        .map(|r| (r.method.as_str(), r.path.as_str()))
        .collect();
    assert_eq!(
        calls,
        [("GET", "/v4/reviews/crm-1"), ("PUT", "/v4/reviews/crm-1")]
    );
    let expected = json!({"data": {"type": "reviews", "attributes": {"state": "banned"}}});
    assert_eq!(requests[1].json(), expected);
    assert_eq!(v["command"], "reviews.update");
    assert_eq!(
        v["result"]["request"],
        json!({"method": "PUT", "path": "/reviews/crm-1", "body": expected})
    );
    assert_eq!(
        (
            v["result"]["updated"]["id"].as_str(),
            v["result"]["created"].as_bool()
        ),
        (Some("r1"), Some(false))
    );
    assert!(v["result"].get("exists").is_none());
}

#[test]
fn text_mode_names_the_updated_review() {
    let server = MockServer::start(vec![review(200, "r1"), review(200, "r1")]);
    let out = run(&server, &UPDATE, "");
    assert_eq!(out.code, 0, "{}", out.stderr);
    assert_eq!(out.stderr, "Отзыв обновлён: id r1\n");
}

#[test]
fn missing_review_is_not_created_without_upsert() {
    let server = MockServer::start(vec![missing()]);
    let out = run(&server, &UPDATE, "");
    assert_eq!(out.code, 5, "{}", out.stderr);
    let err = out.error_json();
    assert_eq!(
        (
            err["error"]["code"].as_str(),
            err["error"]["field"].as_str()
        ),
        (Some("not_found"), Some("id"))
    );
    assert!(
        err["error"]["hint"].as_str().unwrap().contains("--upsert"),
        "{err}"
    );
    assert_eq!(server.requests().len(), 1, "PUT не ушёл");
}

#[test]
fn upsert_skips_the_check_and_reports_creation() {
    let server = MockServer::start(vec![review(201, "r9")]);
    let out = run(&server, &update_with(&["--upsert"]), "");
    assert_eq!(out.code, 0, "{}", out.stderr);
    assert_eq!(out.stderr, "Отзыв создан: id r9\n");
    let requests = server.requests();
    assert_eq!((requests.len(), requests[0].method.as_str()), (1, "PUT"));
    let server = MockServer::start(vec![review(201, "r9")]);
    let v = envelope(&run(&server, &update_with(&["--upsert", "--json"]), ""));
    assert_eq!(v["result"]["created"], true);
}

#[test]
fn dry_run_reads_the_review_and_sends_no_put() {
    let server = MockServer::start(vec![review(200, "r1")]);
    let plan = envelope(&run(&server, &update_with(&["-n", "--json"]), ""));
    assert_eq!(
        (
            plan["dry_run"].as_bool(),
            &plan["result"]["updated"],
            plan["result"]["exists"].as_bool()
        ),
        (Some(true), &Value::Null, Some(true))
    );
    assert_eq!(plan["result"]["request"]["method"], "PUT");
    let requests = server.requests();
    assert_eq!((requests.len(), requests[0].method.as_str()), (1, "GET"));

    let server = MockServer::start(vec![missing()]);
    let plan = envelope(&run(
        &server,
        &update_with(&["--upsert", "-n", "--json"]),
        "",
    ));
    assert_eq!(
        plan["result"]["exists"], false,
        "с --upsert PUT создаст отзыв"
    );
    assert_eq!(server.requests().len(), 1);

    let server = MockServer::start(vec![missing()]);
    let out = run(&server, &update_with(&["-n", "--json"]), "");
    assert_eq!(out.code, 5, "{}", out.stderr);
    let err = out.error_json();
    assert_eq!(
        (err["error"]["code"].as_str(), err["dry_run"].as_bool()),
        (Some("not_found"), Some(true))
    );
}

#[test]
fn update_is_replayed_after_500() {
    let server = MockServer::start(vec![
        review(200, "r1"),
        Reply::text(500, "oops"),
        review(200, "r1"),
    ]);
    let out = run(&server, &UPDATE, "");
    assert_eq!(out.code, 0, "{}", out.stderr);
    assert!(
        out.stderr.ends_with("Отзыв обновлён: id r1\n"),
        "{}",
        out.stderr
    );
    assert_eq!(server.requests().len(), 3);
}

/// Review Focus: ответ на создающий PUT потерялся, повтор нашёл отзыв — `created: false`, как обещает
/// справка.
#[test]
fn upsert_replayed_after_a_lost_answer_reports_an_update() {
    let server = MockServer::start(vec![Reply::text(500, "oops"), review(200, "r9")]);
    let v = envelope(&run(&server, &update_with(&["--upsert", "--json"]), ""));
    assert_eq!(v["result"]["created"], false);
    assert_eq!(server.requests().len(), 2);
}

/// Review Focus: внешний id с пробелом и кириллицей — GET и PUT адресуют один и тот же сегмент.
#[test]
fn get_and_put_address_the_same_encoded_id() {
    let server = MockServer::start(vec![review(200, "r1"), review(200, "r1")]);
    let out = run(
        &server,
        &["reviews", "update", "crm 42 а", "--state", "banned"],
        "",
    );
    assert_eq!(out.code, 0, "{}", out.stderr);
    let paths: Vec<String> = server.requests().iter().map(|r| r.path.clone()).collect();
    assert_eq!(
        paths,
        [
            "/v4/reviews/crm%2042%20%D0%B0",
            "/v4/reviews/crm%2042%20%D0%B0"
        ]
    );
}

/// Review Focus: проверка наличия упала — ошибка та же, что у get, PUT не уходит.
#[test]
fn failed_existence_check_sends_no_put() {
    let server = MockServer::start(vec![Reply::json(
        403,
        r#"{"errors":{"status":403,"title":"Forbidden"}}"#,
    )]);
    let out = run(&server, &UPDATE, "");
    assert_eq!(out.code, 3, "{}", out.stderr);
    assert_eq!(out.error_json()["error"]["code"], "forbidden");
    assert_eq!(server.requests().len(), 1);
}

#[test]
fn input_errors_are_caught_before_the_network() {
    let server = MockServer::start(vec![]);
    let data = |body: &str| {
        run(
            &server,
            &["reviews", "update", "crm-1", "--data", "-"],
            body,
        )
    };
    local_error(
        &server,
        &data(r#"{"external_id": "crm-2"}"#),
        "invalid_attribute",
        "external_id",
    );
    local_error(
        &server,
        &data(r#"{"pros": null}"#),
        "invalid_attribute",
        "pros",
    );
    local_error(
        &server,
        &data(r#"{"context_type": "product"}"#),
        "unknown_attribute",
        "context_type",
    );
    let flags: [(&[&str], &str, &str); 3] = [
        (
            &["reviews", "update", "crm-1", "--rating", "7"],
            "invalid_attribute",
            "rating",
        ),
        (
            &["reviews", "update", "crm-1", "--body", "-n"],
            "usage",
            "body",
        ),
        (
            &["reviews", "update", "crm.1", "--state", "banned"],
            "invalid_id",
            "id",
        ),
    ];
    for (args, code, field) in flags {
        local_error(&server, &run(&server, args, ""), code, field);
    }
    let out = run(&server, &["reviews", "update", "crm-1"], "");
    assert_eq!(
        (out.code, out.error_json()["error"]["code"].as_str()),
        (2, Some("nothing_to_update"))
    );
    assert!(server.requests().is_empty());
}
