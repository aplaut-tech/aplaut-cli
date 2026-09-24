//! `<ресурс> get <ID>` (спека reviews-write §2): одна запись теми же форматами, что scroll.

mod support;

use serde_json::{json, Value};
use support::{aplaut, MockServer, Output, Reply, TempDir};

const TOKEN: (&str, &str) = ("APLAUT_ACCESS_TOKEN", "tok");

/// Ответ `GET /reviews/{id}?include=author`, с отступами — как отдаёт сервер.
fn record_json() -> String {
    serde_json::to_string_pretty(&json!({
        "data": {
            "id": "r1",
            "type": "reviews",
            "attributes": {"rating": 5.0, "body": "Отлично, \"спасибо\"", "external_id": "crm/42 а"},
            "relationships": {"author": {"data": {"id": "c1", "type": "consumers"}}}
        },
        "included": [{"id": "c1", "type": "consumers", "attributes": {"email": "anna@example.com"}}]
    }))
    .unwrap()
}

fn get(server: &MockServer, args: &[&str]) -> Output {
    let home = TempDir::new("get");
    let url = server.base_url();
    let mut full = args.to_vec();
    full.extend(["--base-url", url.as_str()]);
    aplaut(home.path(), &full, &[TOKEN], "")
}

#[test]
fn raw_is_the_response_body_on_one_line() {
    let server = MockServer::start(vec![Reply::json(200, record_json())]);
    let out = get(&server, &["reviews", "get", "r1"]);
    assert_eq!(out.code, 0, "{}", out.stderr);
    assert_eq!(out.stdout.lines().count(), 1, "{}", out.stdout);
    let body: Value = serde_json::from_str(&out.stdout).unwrap();
    assert_eq!(body, serde_json::from_str::<Value>(&record_json()).unwrap());
    let req = &server.requests()[0];
    assert_eq!(
        (req.method.as_str(), req.path.as_str()),
        ("GET", "/v4/reviews/r1")
    );
    assert!(req.query.is_empty(), "{:?}", req.query);
}

#[test]
fn jsonl_inlines_included_objects() {
    let server = MockServer::start(vec![Reply::json(200, record_json())]);
    let out = get(
        &server,
        &[
            "reviews",
            "get",
            "r1",
            "--include",
            "author",
            "--format",
            "jsonl",
        ],
    );
    assert_eq!(out.code, 0, "{}", out.stderr);
    let record: Value = serde_json::from_str(out.stdout.trim_end()).unwrap();
    assert_eq!(
        record["relationships"]["author"]["data"]["attributes"]["email"],
        "anna@example.com"
    );
    assert_eq!(server.requests()[0].query_param("include"), Some("author"));
}

#[test]
fn csv_is_header_and_one_row_with_fields() {
    let server = MockServer::start(vec![Reply::json(200, record_json())]);
    let out = get(
        &server,
        &[
            "reviews",
            "get",
            "r1",
            "--include",
            "author",
            "--format",
            "csv",
            "--fields",
            "id,rating,author.email",
        ],
    );
    assert_eq!(out.code, 0, "{}", out.stderr);
    assert_eq!(
        out.stdout,
        "id,rating,author.email\nr1,5.0,anna@example.com\n"
    );
}

#[test]
fn unknown_field_fails_before_writing_anything() {
    let server = MockServer::start(vec![Reply::json(200, record_json())]);
    let out = get(
        &server,
        &[
            "reviews",
            "get",
            "r1",
            "--format",
            "csv",
            "--fields",
            "id,raiting",
        ],
    );
    assert_eq!(out.code, 2, "{}", out.stderr);
    assert_eq!(out.stdout, "");
    let err = out.error_json();
    assert_eq!(err["error"]["code"], "unknown_field");
    assert!(
        err["error"]["hint"].as_str().unwrap().contains("rating"),
        "{err}"
    );
}

#[test]
fn include_outside_the_spec_enum_fails_before_any_request() {
    let server = MockServer::start(vec![]);
    let out = get(&server, &["products", "get", "p1", "--include", "author"]);
    assert_eq!(out.code, 2);
    let err = out.error_json();
    assert_eq!(
        (err["command"].as_str(), err["error"]["code"].as_str()),
        (Some("products.get"), Some("invalid_include"))
    );
    assert!(
        err["error"]["hint"]
            .as_str()
            .unwrap()
            .contains("reviews_summary_item"),
        "{err}"
    );
    assert!(server.requests().is_empty());
}

#[test]
fn not_found_is_exit_5() {
    let server = MockServer::start(vec![Reply::json(
        404,
        r#"{"errors":{"status":404,"title":"Not found"}}"#,
    )]);
    let out = get(&server, &["questions", "get", "q404"]);
    assert_eq!(out.code, 5, "{}", out.stderr);
    let err = out.error_json();
    assert_eq!(
        (err["command"].as_str(), err["error"]["code"].as_str()),
        (Some("questions.get"), Some("not_found"))
    );
}

#[test]
fn json_summary_goes_to_stderr_with_the_internal_id() {
    let server = MockServer::start(vec![Reply::json(200, record_json())]);
    let out = get(
        &server,
        &["reviews", "get", "crm-42", "--format", "jsonl", "--json"],
    );
    assert_eq!(out.code, 0, "{}", out.stderr);
    let record: Value = serde_json::from_str(out.stdout.trim_end()).unwrap();
    assert_eq!(record["id"], "r1");
    let envelope: Value = serde_json::from_str(out.stderr.trim_end()).unwrap();
    assert_eq!(envelope["command"], "reviews.get");
    assert_eq!(
        envelope["result"],
        json!({"records_type": "reviews", "id": "r1"})
    );
}

/// Внешний id из системы пользователя — один сегмент пути, какие бы символы в нём ни были.
#[test]
fn external_id_is_sent_as_one_encoded_path_segment() {
    let server = MockServer::start(vec![Reply::json(200, record_json())]);
    let out = get(&server, &["reviews", "get", "crm 42 а?x#y"]);
    assert_eq!(out.code, 0, "{}", out.stderr);
    assert_eq!(
        server.requests()[0].path,
        "/v4/reviews/crm%2042%20%D0%B0%3Fx%23y"
    );
}

/// Пустой id из незаданной переменной шелла не должен стать запросом списка `GET /reviews/`.
#[test]
fn empty_id_is_rejected_before_any_request() {
    let server = MockServer::start(vec![]);
    for id in ["", " ", ".."] {
        let out = get(&server, &["reviews", "get", id]);
        assert_eq!(out.code, 2, "{id:?}: {}", out.stderr);
        let err = out.error_json();
        assert_eq!(
            (
                err["error"]["code"].as_str(),
                err["error"]["field"].as_str()
            ),
            (Some("invalid_id"), Some("id")),
            "{id:?}"
        );
    }
    assert!(server.requests().is_empty());
}

/// Стейджинг, 2026-09-24: `GET /reviews/x.json` и `/reviews/x%2Ejson` ищут запись «x», а `%2F`
/// не маршрутизируется — такой id вернул бы чужую запись или ложное «не найдено».
#[test]
fn ids_the_server_cannot_route_are_refused_before_any_request() {
    let server = MockServer::start(vec![]);
    for id in ["crm-1.5", "report.json", "a/b"] {
        let out = get(&server, &["reviews", "get", id]);
        assert_eq!(out.code, 2, "{id}: {}", out.stderr);
        let err = out.error_json();
        assert_eq!(err["error"]["code"], "invalid_id", "{id}");
    }
    assert!(server.requests().is_empty());
}
