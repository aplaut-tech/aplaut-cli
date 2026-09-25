//! `consumers get/create/update` (спека writes-and-exports §3): include, тело, e-mail и телефон только
//! при создании, null, подсказки.

mod support;

use serde_json::json;
use support::{envelope, local_error, run, MockServer, Reply};

fn consumer(status: u16, id: &str) -> Reply {
    Reply::json(
        status,
        json!({"data": {"id": id, "type": "consumers", "attributes": {
            "email": "anna@example.com", "name": "Анна", "external_id": "c-1"
        }}})
        .to_string(),
    )
}

fn taken(field: &str) -> Reply {
    Reply::json(
        422,
        format!(
            r#"{{"errors":{{"status":422,"title":"Validation error","details":{{"{field}":["is already taken"],"model":"Validation of Consumer failed."}}}}}}"#
        ),
    )
}

#[test]
fn get_sends_includes_from_the_spec() {
    let server = MockServer::start(vec![consumer(200, "c1")]);
    let out = run(
        &server,
        &[
            "consumers",
            "get",
            "c-1",
            "--include",
            "orders",
            "--format",
            "jsonl",
        ],
        "",
    );
    assert_eq!(out.code, 0, "{}", out.stderr);
    let record: serde_json::Value = serde_json::from_str(out.stdout.trim_end()).unwrap();
    assert_eq!(record["id"], "c1");
    let req = &server.requests()[0];
    assert_eq!(
        (req.path.as_str(), req.query_param("include")),
        ("/v4/consumers/c-1", Some("orders"))
    );
    let bad = run(
        &server,
        &["consumers", "get", "c-1", "--include", "products"],
        "",
    );
    assert_eq!(bad.code, 2, "{}", bad.stderr);
    assert_eq!(bad.error_json()["error"]["code"], "invalid_include");
    assert_eq!(server.requests().len(), 1);
}

#[test]
fn create_sends_flags_and_requires_nothing_else() {
    let server = MockServer::start(vec![consumer(201, "c1")]);
    let v = envelope(&run(
        &server,
        &[
            "consumers",
            "create",
            "--external-id",
            "c-1",
            "--email",
            "anna@example.com",
            "--name",
            "Анна",
            "--unsubscribed",
            "true",
            "--json",
        ],
        "",
    ));
    let expected = json!({"data": {"type": "consumers", "attributes": {
        "external_id": "c-1", "email": "anna@example.com", "name": "Анна", "unsubscribed": true
    }}});
    let req = &server.requests()[0];
    assert_eq!(
        (req.method.as_str(), req.path.as_str()),
        ("POST", "/v4/consumers")
    );
    assert_eq!(req.json(), expected);
    assert_eq!(
        (v["command"].as_str(), v["result"]["created"]["id"].as_str()),
        (Some("consumers.create"), Some("c1"))
    );
    let server = MockServer::start(vec![consumer(201, "c2")]);
    envelope(&run(
        &server,
        &["consumers", "create", "--external-id", "c-2", "--json"],
        "",
    ));
    assert_eq!(
        server.requests()[0].json()["data"]["attributes"],
        json!({"external_id": "c-2"}),
        "сервер не требует email и name (стейджинг, 2026-09-25)"
    );
}

#[test]
fn create_hints_for_retry_and_taken_values() {
    let server = MockServer::start(vec![Reply::text(500, "oops")]);
    let err = run(
        &server,
        &["consumers", "create", "--external-id", "c-1"],
        "",
    )
    .error_json();
    let hint = err["error"]["hint"].as_str().unwrap();
    assert!(
        hint.contains("повтор безопасен") && hint.contains("aplaut consumers get c-1"),
        "{hint}"
    );
    let server = MockServer::start(vec![taken("external_id")]);
    let err = run(
        &server,
        &["consumers", "create", "--external-id", "c-1"],
        "",
    )
    .error_json();
    assert!(
        err["error"]["hint"]
            .as_str()
            .unwrap()
            .contains("aplaut consumers update c-1"),
        "{err}"
    );
    let server = MockServer::start(vec![taken("email")]);
    let err = run(
        &server,
        &["consumers", "create", "--email", "anna@example.com"],
        "",
    )
    .error_json();
    assert_eq!(err["error"]["field"], "email");
    assert!(
        err["error"]["hint"]
            .as_str()
            .unwrap()
            .contains("e-mail уже есть"),
        "{err}"
    );
}

/// E-mail и телефон сервер задаёт только при создании (стейджинг, 2026-09-25).
#[test]
fn email_and_phone_need_upsert() {
    let server = MockServer::start(vec![]);
    local_error(
        &server,
        &run(
            &server,
            &["consumers", "update", "c-1", "--email", "new@example.com"],
            "",
        ),
        "invalid_attribute",
        "email",
    );
    local_error(
        &server,
        &run(
            &server,
            &["consumers", "update", "c-1", "--phone", "79000000000"],
            "",
        ),
        "invalid_attribute",
        "phone",
    );
    let server = MockServer::start(vec![consumer(201, "c9")]);
    let v = envelope(&run(
        &server,
        &[
            "consumers",
            "update",
            "c-9",
            "--email",
            "new@example.com",
            "--upsert",
            "--json",
        ],
        "",
    ));
    assert_eq!(v["result"]["created"], true);
    let req = &server.requests()[0];
    assert_eq!(
        (req.method.as_str(), req.path.as_str()),
        ("PUT", "/v4/consumers/c-9")
    );
    assert_eq!(
        req.json()["data"]["attributes"],
        json!({"email": "new@example.com"})
    );
}

/// null очищает атрибут клиента (стейджинг, 2026-09-25); внешний id не меняется.
#[test]
fn update_clears_with_null_but_cannot_rename() {
    let server = MockServer::start(vec![consumer(200, "c1"), consumer(200, "c1")]);
    envelope(&run(
        &server,
        &["consumers", "update", "c-1", "--data", "-", "--json"],
        r#"{"first_name": null, "custom_attributes": {"old": null}}"#,
    ));
    assert_eq!(
        server.requests()[1].json()["data"]["attributes"],
        json!({"first_name": null, "custom_attributes": {"old": null}})
    );
    let none = MockServer::start(vec![]);
    local_error(
        &none,
        &run(
            &none,
            &["consumers", "update", "c-1", "--data", "-"],
            r#"{"external_id": "c-2"}"#,
        ),
        "invalid_attribute",
        "external_id",
    );
    local_error(
        &none,
        &run(
            &none,
            &["consumers", "update", "c-1", "--unsubscribed", "yes"],
            "",
        ),
        "invalid_attribute",
        "unsubscribed",
    );
}
