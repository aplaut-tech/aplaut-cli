//! `orders get/create/update` (спека writes-and-exports §4): внешний id — number, контакт клиента,
//! строки заказа, include.

mod support;

use serde_json::json;
use support::{envelope, local_error, run, MockServer, Reply};

fn order(status: u16, id: &str) -> Reply {
    Reply::json(
        status,
        json!({"data": {"id": id, "type": "orders", "attributes": {
            "number": "o-1", "consumer_email": "anna@example.com"
        }}})
        .to_string(),
    )
}

const CREATE: [&str; 6] = [
    "orders",
    "create",
    "--number",
    "o-1",
    "--consumer-email",
    "anna@example.com",
];

#[test]
fn create_sends_number_contact_and_lines() {
    let server = MockServer::start(vec![order(201, "o1")]);
    let mut args = CREATE.to_vec();
    args.extend(["--consumer-name", "Анна", "--data", "-", "--json"]);
    let v = envelope(&run(
        &server,
        &args,
        r#"{"order_lines": [{"product_id": "444772", "name": "Диск", "price": 5990}], "details": {"region": 74}}"#,
    ));
    let expected = json!({"data": {"type": "orders", "attributes": {
        "number": "o-1", "consumer_email": "anna@example.com", "consumer_name": "Анна",
        "order_lines": [{"product_id": "444772", "name": "Диск", "price": 5990}],
        "details": {"region": 74}
    }}});
    let req = &server.requests()[0];
    assert_eq!(
        (req.method.as_str(), req.path.as_str()),
        ("POST", "/v4/orders")
    );
    assert_eq!(req.json(), expected);
    assert_eq!(
        (v["command"].as_str(), v["result"]["created"]["id"].as_str()),
        (Some("orders.create"), Some("o1"))
    );
}

#[test]
fn create_needs_a_number_and_a_contact_before_the_network() {
    let server = MockServer::start(vec![]);
    local_error(
        &server,
        &run(
            &server,
            &["orders", "create", "--consumer-email", "a@example.com"],
            "",
        ),
        "missing_attribute",
        "number",
    );
    local_error(
        &server,
        &run(
            &server,
            &[
                "orders",
                "create",
                "--number",
                "o-1",
                "--consumer-name",
                "Анна",
            ],
            "",
        ),
        "missing_attribute",
        "consumer_email",
    );
    let phone_only = MockServer::start(vec![order(201, "o1")]);
    let out = run(
        &phone_only,
        &[
            "orders",
            "create",
            "--number",
            "o-1",
            "--consumer-phone",
            "79000000000",
        ],
        "",
    );
    assert_eq!(out.code, 0, "{}", out.stderr);
}

/// Review Focus: строку заказа без product_id сервер принимает молча (стейджинг, 2026-09-25) — CLI
/// ловит её до сети.
#[test]
fn order_lines_without_product_id_are_refused() {
    let server = MockServer::start(vec![]);
    let mut args = CREATE.to_vec();
    args.extend(["--data", "-"]);
    local_error(
        &server,
        &run(
            &server,
            &args,
            r#"{"order_lines": [{"product_id": "1"}, {"name": "без товара"}]}"#,
        ),
        "invalid_attribute",
        "order_lines",
    );
    local_error(
        &server,
        &run(
            &server,
            &["orders", "update", "o-1", "--data", "-"],
            r#"{"order_lines": [{"product_id": ""}]}"#,
        ),
        "invalid_attribute",
        "order_lines",
    );
}

/// `product_id` числом, а не строкой (типичная ошибка при переносе значений из YML без кавычек):
/// сообщение должно отличаться от «нет product_id».
#[test]
fn order_lines_numeric_product_id_is_refused() {
    let server = MockServer::start(vec![]);
    let mut args = CREATE.to_vec();
    args.extend(["--data", "-"]);
    let out = run(
        &server,
        &args,
        r#"{"order_lines": [{"product_id": 444772}]}"#,
    );
    local_error(&server, &out, "invalid_attribute", "order_lines");
    assert!(
        out.error_json()["error"]["message"]
            .as_str()
            .unwrap()
            .contains("не строка"),
        "{}",
        out.error_json()
    );
}

#[test]
fn taken_number_points_to_update_and_retry_is_safe() {
    let server = MockServer::start(vec![Reply::json(
        422,
        r#"{"errors":{"status":422,"title":"Validation error","details":{"number":["is already taken"],"model":"Validation of Order failed."}}}"#,
    )]);
    let err = run(&server, &CREATE, "").error_json();
    assert_eq!(err["error"]["field"], "number");
    assert!(
        err["error"]["hint"]
            .as_str()
            .unwrap()
            .contains("aplaut orders update o-1"),
        "{err}"
    );
    let server = MockServer::start(vec![Reply::text(500, "oops")]);
    let err = run(&server, &CREATE, "").error_json();
    let hint = err["error"]["hint"].as_str().unwrap();
    assert!(
        hint.contains("повтор безопасен") && hint.contains("aplaut orders get o-1"),
        "{hint}"
    );
}

/// Стейджинг, 2026-09-25: у GET /orders/{id} сервер принимает только include=consumer.
#[test]
fn get_accepts_only_the_consumer_include() {
    let server = MockServer::start(vec![order(200, "o1")]);
    let out = run(
        &server,
        &["orders", "get", "o-1", "--include", "consumer"],
        "",
    );
    assert_eq!(out.code, 0, "{}", out.stderr);
    assert_eq!(
        server.requests()[0].query_param("include"),
        Some("consumer")
    );
    let bad = run(
        &server,
        &["orders", "get", "o-1", "--include", "product"],
        "",
    );
    assert_eq!(bad.code, 2, "{}", bad.stderr);
    assert_eq!(bad.error_json()["error"]["code"], "invalid_include");
    assert_eq!(server.requests().len(), 1);
}

#[test]
fn update_sends_lines_clears_with_null_and_cannot_renumber() {
    let server = MockServer::start(vec![order(200, "o1"), order(200, "o1")]);
    envelope(&run(
        &server,
        &["orders", "update", "o-1", "--data", "-", "--json"],
        r#"{"order_lines": [{"product_id": "555"}], "consumer_name": null}"#,
    ));
    let requests = server.requests();
    assert_eq!(
        (requests[1].method.as_str(), requests[1].path.as_str()),
        ("PUT", "/v4/orders/o-1")
    );
    assert_eq!(
        requests[1].json()["data"]["attributes"],
        json!({"order_lines": [{"product_id": "555"}], "consumer_name": null})
    );
    let none = MockServer::start(vec![]);
    local_error(
        &none,
        &run(
            &none,
            &["orders", "update", "o-1", "--data", "-"],
            r#"{"number": "o-2"}"#,
        ),
        "invalid_attribute",
        "number",
    );
}
