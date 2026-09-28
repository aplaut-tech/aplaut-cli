//! `exports create` и `exports get` (спека writes-and-exports §5; §9): тело задачи, перевод фильтра, проверки
//! до сети, план под `-n`, скачивание, конечные состояния. Ответы сразу конечные — реального ожидания нет.

mod support;

use std::io::Write;

use serde_json::{json, Value};
use support::{envelope, local_error, run, MockServer, Reply, TempDir};

fn task(state: &str, extra: Value) -> Value {
    let mut attributes = json!({"records_type": "reviews", "format": "jsonl", "state": state,
        "search_options": {"filter": null, "sort": null}, "created_at": "2026-09-28T09:52:30.321+03:00"});
    attributes
        .as_object_mut()
        .unwrap()
        .extend(extra.as_object().unwrap().clone());
    json!({"data": {"id": "e1", "type": "export_tasks", "attributes": attributes}})
}

fn reply(status: u16, state: &str, extra: Value) -> Reply {
    Reply::json(status, task(state, extra).to_string())
}

fn gzip(text: &str) -> Vec<u8> {
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    encoder.write_all(text.as_bytes()).unwrap();
    encoder.finish().unwrap()
}

#[test]
fn create_sends_the_task_with_jsonl_by_default() {
    let server = MockServer::start(vec![reply(201, "waiting", json!({}))]);
    let v = envelope(&run(
        &server,
        &["exports", "create", "--records-type", "reviews", "--json"],
        "",
    ));
    let expected = json!({"data": {"type": "export_tasks", "attributes": {"records_type": "reviews", "format": "jsonl"}}});
    let requests = server.requests();
    assert_eq!(requests.len(), 1, "без --wait опроса нет");
    assert_eq!(
        (requests[0].method.as_str(), requests[0].path.as_str()),
        ("POST", "/v4/export_tasks")
    );
    assert_eq!(requests[0].json(), expected);
    assert_eq!(v["command"], "exports.create");
    assert_eq!(
        v["result"]["request"],
        json!({"method": "POST", "path": "/export_tasks", "body": expected})
    );
    assert_eq!(
        (v["result"]["id"].as_str(), v["result"]["state"].as_str()),
        (Some("e1"), Some("waiting"))
    );
    assert_eq!(v["result"]["output_path"], Value::Null);
}

#[test]
fn filter_and_survey_id_become_search_options() {
    let server = MockServer::start(vec![reply(201, "waiting", json!({}))]);
    envelope(&run(
        &server,
        &[
            "exports",
            "create",
            "--records-type",
            "reviews",
            "--filter",
            "updated_at:gte:2026-09-01T00:00:00Z,rating:gte:4,state:neq:published",
            "--json",
        ],
        "",
    ));
    assert_eq!(
        server.requests()[0].json()["data"]["attributes"]["search_options"],
        json!({"filter": {"updated_at": {"gte": "2026-09-01T00:00:00Z"}, "rating": {"gte": 0.8},
                          "state": {"ne": "published"}}})
    );
    let server = MockServer::start(vec![reply(201, "waiting", json!({}))]);
    envelope(&run(
        &server,
        &[
            "exports",
            "create",
            "--records-type",
            "survey_responses",
            "--survey-id",
            "s1",
            "--data",
            "-",
            "--json",
        ],
        r#"{"search_options": {"sort": {"created_at": "asc"}}}"#,
    ));
    assert_eq!(
        server.requests()[0].json()["data"]["attributes"]["search_options"],
        json!({"sort": {"created_at": "asc"}, "filter": {"survey_id": {"eq": "s1"}}})
    );
}

#[test]
fn input_errors_are_caught_before_the_network() {
    let server = MockServer::start(vec![]);
    let dir = TempDir::new("exports-local");
    let missing_dir = dir.path().join("no-such-dir").join("r.jsonl");
    let missing = missing_dir.to_str().unwrap();
    let cases: [(&[&str], &str, &str); 9] = [
        (&["exports", "create"], "missing_attribute", "records_type"),
        (
            &["exports", "create", "--records-type", "bogus"],
            "invalid_attribute",
            "records_type",
        ),
        (
            &[
                "exports",
                "create",
                "--records-type",
                "reviews",
                "--jq",
                "[.id]",
            ],
            "jq_needs_csv_or_xlsx",
            "jq",
        ),
        (
            &[
                "exports",
                "create",
                "--records-type",
                "consumers",
                "--filter",
                "created_at:gte:2026-01-01",
            ],
            "invalid_filter",
            "filter",
        ),
        (
            &[
                "exports",
                "create",
                "--records-type",
                "products",
                "--filter",
                "price:in:1|2",
            ],
            "invalid_filter",
            "filter",
        ),
        (
            &["exports", "create", "--records-type", "survey_responses"],
            "missing_attribute",
            "survey_id",
        ),
        (
            &[
                "exports",
                "create",
                "--records-type",
                "reviews",
                "--survey-id",
                "s1",
            ],
            "usage",
            "survey_id",
        ),
        (
            &[
                "exports",
                "create",
                "--records-type",
                "reviews",
                "--output",
                missing,
            ],
            "usage",
            "output",
        ),
        (
            &[
                "exports",
                "create",
                "--records-type",
                "reviews",
                "--wait-timeout",
                "0",
            ],
            "usage",
            "wait_timeout",
        ),
    ];
    for (args, code, field) in cases {
        local_error(&server, &run(&server, args, ""), code, field);
    }
    let unknown = run(
        &server,
        &[
            "exports",
            "create",
            "--records-type",
            "reviews",
            "--data",
            "-",
        ],
        r#"{"filter": "x"}"#,
    );
    local_error(&server, &unknown, "unknown_attribute", "filter");
}

/// Review Focus: каталога для файла нет — ошибка до создания задачи (и до траты минутного окна).
#[test]
fn output_into_a_missing_directory_is_refused_before_the_network() {
    let server = MockServer::start(vec![]);
    let dir = TempDir::new("exports-out");
    let target = dir.path().join("missing").join("reviews.jsonl");
    let out = run(
        &server,
        &[
            "exports",
            "create",
            "--records-type",
            "reviews",
            "--output",
            target.to_str().unwrap(),
        ],
        "",
    );
    local_error(&server, &out, "usage", "output");
    let out = run(
        &server,
        &[
            "exports",
            "get",
            "e1",
            "--output",
            dir.path().to_str().unwrap(),
        ],
        "",
    );
    local_error(&server, &out, "usage", "output");
}

#[test]
fn dry_run_plans_the_task_and_sends_nothing() {
    let server = MockServer::start(vec![]);
    let plan = envelope(&run(
        &server,
        &[
            "exports",
            "create",
            "--records-type",
            "reviews",
            "--format",
            "csv",
            "--jq",
            "[.id, .rating]",
            "-n",
            "--json",
        ],
        "",
    ));
    assert_eq!(plan["dry_run"], true);
    assert_eq!(
        plan["result"]["request"]["body"]["data"]["attributes"],
        json!({"records_type": "reviews", "format": "csv", "export_format": "[.id, .rating]"})
    );
    assert_eq!(
        (&plan["result"]["id"], &plan["result"]["state"]),
        (&Value::Null, &Value::Null)
    );
    assert!(server.requests().is_empty());
}

/// Review Focus: архив скачивается без токена; gzip распакован, путь в result — абсолютный.
#[test]
fn create_with_output_downloads_without_the_token() {
    // Один сервер отдаёт и API, и архив: ссылка в ответе указывает на него же.
    let server = MockServer::start_with(|origin| {
        vec![
            reply(
                201,
                "completed",
                json!({"archive_url": format!("{origin}/export_data/a.jsonl.gz?1790578357"),
                       "archive_content_type": "application/gzip", "archive_size": 42,
                       "finished_at": "2026-09-28T09:52:37.401+03:00"}),
            ),
            Reply::Http {
                status: 200,
                headers: vec![],
                body: gzip("{\"id\":\"r1\"}\n"),
            },
        ]
    });
    let dir = TempDir::new("exports-dl");
    let dest = dir.path().join("reviews.jsonl");
    let v = envelope(&run(
        &server,
        &[
            "exports",
            "create",
            "--records-type",
            "reviews",
            "--output",
            dest.to_str().unwrap(),
            "--json",
        ],
        "",
    ));
    assert_eq!(std::fs::read_to_string(&dest).unwrap(), "{\"id\":\"r1\"}\n");
    assert_eq!(v["result"]["output_path"], dest.to_str().unwrap());
    let requests = server.requests();
    assert_eq!(requests[1].path, "/export_data/a.jsonl.gz");
    assert!(requests[1].header("authorization").is_none());
    assert!(requests[0].header("authorization").is_some());
}

#[test]
fn get_reports_the_state_and_wait_turns_failures_into_errors() {
    let rejected = || {
        reply(
            200,
            "rejected",
            json!({"format": "csv", "export_format": "{id: .id}",
                   "error_message": "undefined method '[]' for nil", "finished_at": "x"}),
        )
    };
    let server = MockServer::start(vec![rejected(), rejected()]);
    let v = envelope(&run(&server, &["exports", "get", "e1", "--json"], ""));
    assert_eq!(
        (v["command"].as_str(), v["result"]["state"].as_str()),
        (Some("exports.get"), Some("rejected"))
    );
    assert_eq!(
        v["result"]["error_message"],
        "undefined method '[]' for nil"
    );
    assert_eq!(server.requests()[0].path, "/v4/export_tasks/e1");
    let out = run(&server, &["exports", "get", "e1", "--wait", "--json"], "");
    assert_eq!(out.code, 1, "{}", out.stderr);
    let err = out.error_json();
    assert_eq!(err["error"]["code"], "export_rejected");
    assert!(err["error"]["hint"].as_str().unwrap().contains("массив"));
    let none = MockServer::start(vec![]);
    local_error(
        &none,
        &run(&none, &["exports", "get", "e.1"], ""),
        "invalid_id",
        "id",
    );
}

/// Review Focus: вторая задача в ту же минуту — 429 с Retry-After, CLI ждёт окно и повторяет.
#[test]
fn second_create_in_a_minute_waits_for_the_window() {
    let server = MockServer::start(vec![
        Reply::text(429, "Throttled").with_header("Retry-After", "1"),
        reply(201, "waiting", json!({})),
    ]);
    let v = envelope(&run(
        &server,
        &["exports", "create", "--records-type", "orders", "--json"],
        "",
    ));
    assert_eq!(v["result"]["id"], "e1");
    assert_eq!(server.requests().len(), 2);
}

#[test]
fn text_mode_says_how_to_wait_for_the_file() {
    let server = MockServer::start(vec![reply(201, "waiting", json!({}))]);
    let out = run(
        &server,
        &["exports", "create", "--records-type", "reviews"],
        "",
    );
    assert_eq!(out.code, 0, "{}", out.stderr);
    assert!(
        out.stdout.is_empty(),
        "данных у команды нет: {}",
        out.stdout
    );
    assert!(
        out.stderr.contains("e1") && out.stderr.contains("aplaut exports get e1 --output"),
        "{}",
        out.stderr
    );
}
