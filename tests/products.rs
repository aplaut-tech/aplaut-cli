//! `products create` и `products update` (спека products-write): тело, повторы, план под `-n`,
//! ошибки до сети.

mod support;

use serde_json::{json, Value};
use support::{envelope, local_error, run, MockServer, Reply};

fn product(status: u16, id: &str) -> Reply {
    Reply::json(
        status,
        json!({"data": {"id": id, "type": "products", "attributes": {"name": "Диск"}}}).to_string(),
    )
}

const CREATE: [&str; 8] = [
    "products",
    "create",
    "--external-id",
    "60757",
    "--name",
    "Диск",
    "--url",
    "https://shop.example/p/60757",
];

fn create_with(extra: &[&'static str]) -> Vec<&'static str> {
    let mut args = CREATE.to_vec();
    args.extend_from_slice(extra);
    args
}

#[test]
fn create_sends_typed_flags_over_data() {
    let server = MockServer::start(vec![product(201, "p1")]);
    let args = create_with(&[
        "--price",
        "5990.5",
        "--available",
        "false",
        "--description",
        "- быстрый\n- тихий",
        "--data",
        "-",
        "--json",
    ]);
    let out = run(
        &server,
        &args,
        r#"{"price": 1, "category_names": ["Электроника", "Диски"]}"#,
    );
    let v = envelope(&out);
    let expected = json!({"data": {"type": "products", "attributes": {
        "external_id": "60757", "name": "Диск", "url": "https://shop.example/p/60757",
        "price": 5990.5, "available": false, "description": "- быстрый\n- тихий",
        "category_names": ["Электроника", "Диски"]
    }}});
    let req = &server.requests()[0];
    assert_eq!(
        (req.method.as_str(), req.path.as_str()),
        ("POST", "/v4/products")
    );
    assert_eq!(req.json(), expected);
    assert_eq!(v["command"], "products.create");
    assert_eq!(
        v["result"]["request"],
        json!({"method": "POST", "path": "/products", "body": expected})
    );
    assert_eq!(v["result"]["created"]["id"], "p1");
}

#[test]
fn create_dry_run_sends_nothing_and_text_mode_names_the_product() {
    let server = MockServer::start(vec![product(201, "p1")]);
    let plan = envelope(&run(&server, &create_with(&["-n", "--json"]), ""));
    assert_eq!(
        (plan["dry_run"].as_bool(), &plan["result"]["created"]),
        (Some(true), &Value::Null)
    );
    assert!(server.requests().is_empty());
    let out = run(&server, &create_with(&[]), "");
    assert_eq!(out.code, 0, "{}", out.stderr);
    assert_eq!(out.stderr, "Товар создан: id p1\n");
}

#[test]
fn create_input_errors_are_caught_before_the_network() {
    let server = MockServer::start(vec![]);
    let no_url = ["products", "create", "--external-id", "1", "--name", "x"];
    local_error(
        &server,
        &run(&server, &no_url, ""),
        "missing_attribute",
        "url",
    );
    let no_id = ["products", "create", "--name", "x", "--url", "https://x"];
    local_error(
        &server,
        &run(&server, &no_id, ""),
        "missing_attribute",
        "external_id",
    );
    let cases: [(&[&'static str], &str, &str); 4] = [
        (&["--available", "yes"], "invalid_attribute", "available"),
        (&["--available", "1"], "invalid_attribute", "available"),
        (&["--price", "10,5"], "invalid_attribute", "price"),
        (&["--description", "-n"], "usage", "description"),
    ];
    for (extra, code, field) in cases {
        local_error(&server, &run(&server, &create_with(extra), ""), code, field);
    }
}

#[test]
fn existing_external_id_points_to_update() {
    let server = MockServer::start(vec![Reply::json(
        422,
        r#"{"errors":{"status":422,"title":"Validation error","details":{"external_id":["is already taken"],"model":"Validation of Product failed."}}}"#,
    )]);
    let out = run(&server, &create_with(&[]), "");
    assert_eq!(out.code, 1, "{}", out.stderr);
    let err = out.error_json();
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
            .contains("aplaut products update 60757"),
        "{err}"
    );
}

#[test]
fn unknown_create_outcome_says_the_retry_is_safe() {
    let server = MockServer::start(vec![Reply::text(500, "oops"), product(201, "p1")]);
    let out = run(&server, &create_with(&[]), "");
    assert_eq!(out.code, 1, "{}", out.stderr);
    let err = out.error_json();
    assert_eq!(err["error"]["code"], "request_outcome_unknown");
    let hint = err["error"]["hint"].as_str().unwrap();
    assert!(
        hint.contains("повтор безопасен") && hint.contains("aplaut products get 60757"),
        "{hint}"
    );
    assert_eq!(server.requests().len(), 1);
}
