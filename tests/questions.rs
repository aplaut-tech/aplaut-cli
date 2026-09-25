//! `questions create` и `questions update` (спека writes-and-exports §2): тело, повторы, подстановка
//! `product_id` у вопроса о товаре, ошибки до сети.

mod support;

use serde_json::json;
use support::{envelope, local_error, run, MockServer, Reply};

fn question(status: u16, id: &str, product_id: Option<&str>) -> Reply {
    Reply::json(
        status,
        json!({"data": {"id": id, "type": "questions", "attributes": {
            "text": "Есть размер M?", "product_id": product_id, "state": "waiting"
        }}})
        .to_string(),
    )
}

fn missing() -> Reply {
    Reply::json(
        404,
        r#"{"errors":{"status":404,"title":"Document not found"}}"#,
    )
}

const CREATE: [&str; 6] = [
    "questions",
    "create",
    "--external-id",
    "q-1",
    "--text",
    "Есть размер M?",
];

#[test]
fn create_sends_flags_over_data() {
    let server = MockServer::start(vec![question(201, "q1", Some("444772"))]);
    let mut args = CREATE.to_vec();
    args.extend([
        "--product-id",
        "444772",
        "--state",
        "waiting",
        "--data",
        "-",
        "--json",
    ]);
    let v = envelope(&run(
        &server,
        &args,
        r#"{"text": "из файла", "tags": ["размер"]}"#,
    ));
    let expected = json!({"data": {"type": "questions", "attributes": {
        "external_id": "q-1", "text": "Есть размер M?", "product_id": "444772",
        "state": "waiting", "tags": ["размер"]
    }}});
    let req = &server.requests()[0];
    assert_eq!(
        (req.method.as_str(), req.path.as_str()),
        ("POST", "/v4/questions")
    );
    assert_eq!(req.json(), expected);
    assert_eq!(v["command"], "questions.create");
    assert_eq!(v["result"]["created"]["id"], "q1");
}

#[test]
fn create_text_mode_names_the_question() {
    let server = MockServer::start(vec![question(201, "q1", None)]);
    let out = run(&server, &CREATE, "");
    assert_eq!(out.code, 0, "{}", out.stderr);
    assert_eq!(out.stderr, "Вопрос создан: id q1\n");
}

#[test]
fn create_input_errors_are_caught_before_the_network() {
    let server = MockServer::start(vec![]);
    let cases: [(&[&str], &str, &str); 3] = [
        (
            &["questions", "create", "--external-id", "q-1"],
            "missing_attribute",
            "text",
        ),
        (
            &["questions", "create", "--text", "?", "--state", "held"],
            "invalid_attribute",
            "state",
        ),
        (
            &["questions", "create", "--text", "--json"],
            "usage",
            "text",
        ),
    ];
    for (args, code, field) in cases {
        local_error(&server, &run(&server, args, ""), code, field);
    }
}

#[test]
fn unknown_create_outcome_is_safe_to_retry_only_with_an_external_id() {
    let server = MockServer::start(vec![Reply::text(500, "oops")]);
    let out = run(&server, &CREATE, "");
    assert_eq!(out.code, 1, "{}", out.stderr);
    let err = out.error_json();
    assert_eq!(err["error"]["code"], "request_outcome_unknown");
    let hint = err["error"]["hint"].as_str().unwrap();
    assert!(
        hint.contains("повтор безопасен") && hint.contains("aplaut questions get q-1"),
        "{hint}"
    );
    assert_eq!(server.requests().len(), 1);
    let server = MockServer::start(vec![Reply::text(500, "oops")]);
    let err = run(&server, &["questions", "create", "--text", "?"], "").error_json();
    let hint = err["error"]["hint"].as_str().unwrap();
    assert!(hint.contains("прежде чем повторять"), "{hint}");
}

#[test]
fn taken_external_id_points_to_update() {
    let server = MockServer::start(vec![Reply::json(
        422,
        r#"{"errors":{"status":422,"title":"Validation error","details":{"external_id":["is already taken"],"model":"Validation of Question failed."}}}"#,
    )]);
    let err = run(&server, &CREATE, "").error_json();
    assert_eq!(
        (
            err["error"]["code"].as_str(),
            err["error"]["field"].as_str()
        ),
        (Some("validation_failed"), Some("external_id"))
    );
    assert!(
        err["error"]["hint"]
            .as_str()
            .unwrap()
            .contains("aplaut questions update q-1"),
        "{err}"
    );
}

/// Сервер называет текст вопроса `body` (стейджинг, 2026-09-25) — в ошибке поле `text`.
#[test]
fn server_error_about_body_names_the_text() {
    let server = MockServer::start(vec![Reply::json(
        422,
        r#"{"errors":{"status":422,"title":"Validation error","details":{"body":["не может быть пустым"],"model":"Validation of Question failed."}}}"#,
    )]);
    let err = run(&server, &["questions", "create", "--text", ""], "").error_json();
    assert_eq!(
        (
            err["error"]["code"].as_str(),
            err["error"]["field"].as_str()
        ),
        (Some("validation_failed"), Some("text"))
    );
}

#[test]
fn update_of_a_product_question_carries_its_product_id() {
    let server = MockServer::start(vec![
        question(200, "q1", Some("444772")),
        question(200, "q1", Some("444772")),
    ]);
    let v = envelope(&run(
        &server,
        &["questions", "update", "q-1", "--state", "banned", "--json"],
        "",
    ));
    let requests = server.requests();
    assert_eq!(
        (requests[0].method.as_str(), requests[1].method.as_str()),
        ("GET", "PUT")
    );
    let expected = json!({"data": {"type": "questions", "attributes": {"state": "banned", "product_id": "444772"}}});
    assert_eq!(requests[1].json(), expected);
    assert_eq!(
        v["result"]["request"]["body"], expected,
        "подстановка видна агенту"
    );
}

#[test]
fn company_question_and_explicit_product_id_are_sent_as_given() {
    let server = MockServer::start(vec![question(200, "q1", None), question(200, "q1", None)]);
    envelope(&run(
        &server,
        &["questions", "update", "q-1", "--state", "banned", "--json"],
        "",
    ));
    assert_eq!(
        server.requests()[1].json()["data"]["attributes"],
        json!({"state": "banned"})
    );
    let server = MockServer::start(vec![
        question(200, "q1", Some("444772")),
        question(200, "q1", Some("555")),
    ]);
    envelope(&run(
        &server,
        &[
            "questions",
            "update",
            "q-1",
            "--product-id",
            "555",
            "--json",
        ],
        "",
    ));
    assert_eq!(
        server.requests()[1].json()["data"]["attributes"],
        json!({"product_id": "555"})
    );
}

/// Под --upsert вопрос читается, если product_id не передан: иначе правка вопроса о товаре ушла бы с
/// ответом 404 (стейджинг, 2026-09-25).
#[test]
fn upsert_reads_the_question_unless_product_id_is_given() {
    let server = MockServer::start(vec![
        question(200, "q1", Some("444772")),
        question(200, "q1", Some("444772")),
    ]);
    let v = envelope(&run(
        &server,
        &[
            "questions",
            "update",
            "q-1",
            "--state",
            "banned",
            "--upsert",
            "--json",
        ],
        "",
    ));
    assert_eq!(server.requests().len(), 2);
    assert_eq!(
        v["result"]["request"]["body"]["data"]["attributes"]["product_id"],
        "444772"
    );

    let server = MockServer::start(vec![question(201, "q9", Some("555"))]);
    let v = envelope(&run(
        &server,
        &[
            "questions",
            "update",
            "q-9",
            "--text",
            "?",
            "--product-id",
            "555",
            "--upsert",
            "--json",
        ],
        "",
    ));
    assert_eq!(
        server.requests().len(),
        1,
        "product_id передан — GET не нужен"
    );
    assert_eq!(v["result"]["created"], true);

    let server = MockServer::start(vec![missing(), question(201, "q9", None)]);
    let v = envelope(&run(
        &server,
        &[
            "questions",
            "update",
            "q-9",
            "--text",
            "?",
            "--upsert",
            "--json",
        ],
        "",
    ));
    assert_eq!(
        server.requests()[1].json()["data"]["attributes"],
        json!({"text": "?"})
    );
    assert_eq!(v["result"]["created"], true);
}

#[test]
fn dry_run_plan_shows_the_carried_product_id() {
    let server = MockServer::start(vec![question(200, "q1", Some("444772"))]);
    let plan = envelope(&run(
        &server,
        &[
            "questions",
            "update",
            "q-1",
            "--state",
            "banned",
            "-n",
            "--json",
        ],
        "",
    ));
    assert_eq!(
        plan["result"]["request"]["body"]["data"]["attributes"],
        json!({"state": "banned", "product_id": "444772"})
    );
    assert_eq!(
        (
            plan["result"]["exists"].as_bool(),
            plan["dry_run"].as_bool()
        ),
        (Some(true), Some(true))
    );
    assert_eq!(server.requests().len(), 1);
}

#[test]
fn update_input_errors_are_caught_before_the_network() {
    let server = MockServer::start(vec![]);
    let data = |body: &str| {
        run(
            &server,
            &["questions", "update", "q-1", "--data", "-"],
            body,
        )
    };
    local_error(
        &server,
        &data(r#"{"external_id": "q-2"}"#),
        "invalid_attribute",
        "external_id",
    );
    local_error(
        &server,
        &data(r#"{"author_name": null}"#),
        "invalid_attribute",
        "author_name",
    );
    local_error(
        &server,
        &run(&server, &["questions", "update", "q-1", "--text", "-n"], ""),
        "usage",
        "text",
    );
}
