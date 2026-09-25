//! Smoke против живого стенда; в обычный `cargo test` не входит (все тесты `#[ignore]`).
//!
//!   APLAUT_E2E_BASE_URL=https://api.staging-old.aplaut.net/v4 \
//!   APLAUT_ACCESS_TOKEN_FILE=… cargo test --test e2e -- --ignored --test-threads=1
//!
//! Токен — из APLAUT_ACCESS_TOKEN_FILE или APLAUT_ACCESS_TOKEN; адрес стенда в коде не хранится.
//! Круги записи (`review_*`, `product_*`, `question_*`, `consumer_*`, `order_*`) создают и удаляют
//! тестовые объекты — только с APLAUT_E2E_WRITES=1.

mod support;

use support::{aplaut, Output, TempDir};

const FILTER: &str = "updated_at:gte:2020-01-01T00:00:00Z";

fn base_url() -> String {
    std::env::var("APLAUT_E2E_BASE_URL").expect("задайте APLAUT_E2E_BASE_URL")
}

fn run(args: &[&str]) -> Output {
    run_with_stdin(args, "")
}

fn run_with_stdin(args: &[&str], stdin: &str) -> Output {
    let home = TempDir::new("e2e");
    let base = base_url();
    let mut env: Vec<(String, String)> = vec![("APLAUT_BASE_URL".into(), base)];
    for key in ["APLAUT_ACCESS_TOKEN_FILE", "APLAUT_ACCESS_TOKEN"] {
        if let Ok(value) = std::env::var(key) {
            env.push((key.into(), value));
        }
    }
    assert!(
        env.len() > 1,
        "задайте APLAUT_ACCESS_TOKEN_FILE или APLAUT_ACCESS_TOKEN"
    );
    let env: Vec<(&str, &str)> = env.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
    aplaut(home.path(), args, &env, stdin)
}

/// Успешный вызов: последняя строка stdout — конверт (`--json`) или запись (`get`).
fn json_line(out: &Output) -> serde_json::Value {
    assert_eq!(out.code, 0, "{}", out.stderr);
    serde_json::from_str(out.stdout.trim_end()).unwrap_or_else(|e| panic!("{e}: {}", out.stdout))
}

#[test]
#[ignore]
fn reviews_jsonl_with_included_objects() {
    let out = run(&[
        "reviews",
        "scroll",
        "--filter",
        FILTER,
        "--per-page",
        "5",
        "--max-records",
        "10",
        "--include",
        "author,product",
        "--format",
        "jsonl",
    ]);
    assert_eq!(out.code, 0, "{}", out.stderr);
    let records: Vec<serde_json::Value> = out
        .stdout
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert!(!records.is_empty() && records.len() <= 10);
    assert!(records
        .iter()
        .all(|r| r["type"] == "reviews" && r["id"].is_string()));
}

#[test]
#[ignore]
fn products_csv_has_header() {
    let out = run(&[
        "products",
        "scroll",
        "--filter",
        FILTER,
        "--per-page",
        "5",
        "--max-records",
        "5",
        "--format",
        "csv",
    ]);
    assert_eq!(out.code, 0, "{}", out.stderr);
    assert!(out.stdout.starts_with("id,type,"), "{}", out.stdout);
}

#[test]
#[ignore]
fn questions_raw_is_one_page_per_line() {
    let out = run(&[
        "questions",
        "scroll",
        "--filter",
        FILTER,
        "--per-page",
        "3",
        "--max-records",
        "3",
    ]);
    assert_eq!(out.code, 0, "{}", out.stderr);
    for line in out.stdout.lines() {
        let page: serde_json::Value = serde_json::from_str(line).unwrap();
        assert!(page["meta"]["has_more"].is_boolean());
    }
}

#[test]
#[ignore]
fn wrong_token_is_exit_3_with_request_id() {
    let home = TempDir::new("e2e-bad");
    let base = base_url();
    let out = aplaut(
        home.path(),
        &[
            "reviews",
            "scroll",
            "--filter",
            FILTER,
            "--max-records",
            "1",
        ],
        &[
            ("APLAUT_BASE_URL", &base),
            ("APLAUT_ACCESS_TOKEN", "definitely-not-a-token"),
        ],
        "",
    );
    assert_eq!(out.code, 3, "{}", out.stderr);
    let err = out.error_json();
    // Два вида 401 на стейджинге: Doorkeeper (`error="invalid_token"`) для токена правильного
    // формата и `Token realm="Aplaut Platform API v4"` без `error` — для строки не того формата.
    let code = err["error"]["code"].as_str().unwrap();
    assert!(code == "invalid_token" || code == "unauthorized", "{err}");
    assert!(err["error"]["request_id"].is_string());
    assert!(err["error"]["hint"]
        .as_str()
        .unwrap()
        .contains("APLAUT_ACCESS_TOKEN"));
}

/// Круг записи (спека reviews-write §6): создать → get по внешнему и внутреннему id →
/// комментарий → удалить. Пишет на стенд, поэтому только с APLAUT_E2E_WRITES=1; удаление — в
/// guard, даже если тест упал.
#[test]
#[ignore]
fn review_write_round_trip() {
    if !writes_enabled() {
        return;
    }
    let external_id = format!("aplaut-cli-test-{}", unix_seconds());
    // Guard — до создания: удаляет по внешнему id, даже если ответ на create не разобрался.
    let _cleanup = Cleanup(format!("reviews/{external_id}"));
    let out = run(&[
        "reviews",
        "create",
        "--rating",
        "5",
        "--body",
        "Тестовый отзыв aplaut-cli, будет удалён",
        "--author-name",
        "aplaut-cli test",
        "--state",
        "waiting",
        "--external-id",
        &external_id,
        "--json",
    ]);
    assert_eq!(out.code, 0, "{}", out.stderr);
    let created: serde_json::Value = serde_json::from_str(out.stdout.trim_end()).unwrap();
    let id = created["result"]["created"]["id"]
        .as_str()
        .expect("id")
        .to_string();
    for key in [&external_id, &id] {
        let out = run(&["reviews", "get", key, "--format", "jsonl"]);
        assert_eq!(out.code, 0, "{key}: {}", out.stderr);
        let record: serde_json::Value = serde_json::from_str(out.stdout.trim_end()).unwrap();
        assert_eq!(
            (
                record["id"].as_str(),
                record["attributes"]["external_id"].as_str()
            ),
            (Some(id.as_str()), Some(external_id.as_str()))
        );
    }
    let out = run(&[
        "reviews",
        "comment",
        &id,
        "--text",
        "Тестовый комментарий aplaut-cli",
        "--author-name",
        "aplaut-cli test",
        "--json",
    ]);
    assert_eq!(out.code, 0, "{}", out.stderr);
    let comment: serde_json::Value = serde_json::from_str(out.stdout.trim_end()).unwrap();
    assert_eq!(comment["result"]["created"]["type"], "comments");
}

/// Круг записи товара (спека products-write §5): создать → изменить цену → get: цена новая,
/// название прежнее (PUT частичный) → удалить в guard. Без категории и бренда: их API не удаляет.
#[test]
#[ignore]
fn product_write_round_trip() {
    if !writes_enabled() {
        return;
    }
    let external_id = format!("aplaut-cli-test-{}", unix_seconds());
    let _cleanup = Cleanup(format!("products/{external_id}"));
    let out = run(&[
        "products",
        "create",
        "--external-id",
        &external_id,
        "--name",
        "aplaut-cli test",
        "--url",
        "https://example.com/aplaut-cli-test",
        "--price",
        "1",
        "--available",
        "false",
        "--json",
    ]);
    assert_eq!(out.code, 0, "{}", out.stderr);
    let out = run(&["products", "update", &external_id, "--price", "2", "--json"]);
    assert_eq!(out.code, 0, "{}", out.stderr);
    let out = run(&["products", "get", &external_id, "--format", "jsonl"]);
    assert_eq!(out.code, 0, "{}", out.stderr);
    let record: serde_json::Value = serde_json::from_str(out.stdout.trim_end()).unwrap();
    assert_eq!(record["attributes"]["price"], 2.0);
    assert_eq!(
        record["attributes"]["name"], "aplaut-cli test",
        "PUT частичный"
    );
}

/// Круг `reviews update` (спека writes-and-exports §10): `--upsert` создаёт → смена статуса → get:
/// статус новый, текст прежний → update несуществующего без `--upsert` — код 5 → удалить в guard.
#[test]
#[ignore]
fn review_update_round_trip() {
    if !writes_enabled() {
        return;
    }
    let external_id = format!("aplaut-cli-test-{}-ru", unix_seconds());
    let _cleanup = Cleanup(format!("reviews/{external_id}"));
    let body = "Тестовый отзыв aplaut-cli, будет удалён";
    let created = json_line(&run(&[
        "reviews",
        "update",
        &external_id,
        "--rating",
        "5",
        "--body",
        body,
        "--author-name",
        "aplaut-cli test",
        "--state",
        "waiting",
        "--upsert",
        "--json",
    ]));
    assert_eq!(created["result"]["created"], true);
    let updated = json_line(&run(&[
        "reviews",
        "update",
        &external_id,
        "--state",
        "banned",
        "--json",
    ]));
    assert_eq!(updated["result"]["created"], false);
    let record = json_line(&run(&["reviews", "get", &external_id, "--format", "jsonl"]));
    assert_eq!(record["attributes"]["state"], "banned");
    assert_eq!(record["attributes"]["body"], body, "PUT частичный");
    let missing = format!("{external_id}-missing");
    // Если R3 сломан, PUT создал бы отзыв — guard уберёт и его.
    let _missing = Cleanup(format!("reviews/{missing}"));
    let out = run(&["reviews", "update", &missing, "--state", "banned"]);
    assert_eq!(out.code, 5, "{}", out.stderr);
}

/// Круг вопросов: вопрос о тестовом товаре → смена статуса без --product-id (CLI подставляет его:
/// без него сервер отвечает 404, стейджинг 2026-09-25) → get → удалить вопрос и товар.
#[test]
#[ignore]
fn question_write_round_trip() {
    if !writes_enabled() {
        return;
    }
    let stamp = unix_seconds();
    let product = format!("aplaut-cli-test-{stamp}-p");
    let question = format!("aplaut-cli-test-{stamp}-q");
    // Guard'ы удаляются в обратном порядке: сначала вопрос, потом товар.
    let _product = Cleanup(format!("products/{product}"));
    let _question = Cleanup(format!("questions/{question}"));
    json_line(&run(&[
        "products",
        "create",
        "--external-id",
        &product,
        "--name",
        "aplaut-cli test",
        "--url",
        "https://example.com/aplaut-cli-test",
        "--available",
        "false",
        "--json",
    ]));
    json_line(&run(&[
        "questions",
        "create",
        "--external-id",
        &question,
        "--text",
        "Тестовый вопрос aplaut-cli, будет удалён?",
        "--author-name",
        "aplaut-cli test",
        "--state",
        "waiting",
        "--product-id",
        &product,
        "--json",
    ]));
    let updated = json_line(&run(&[
        "questions",
        "update",
        &question,
        "--state",
        "banned",
        "--json",
    ]));
    assert_eq!(
        updated["result"]["request"]["body"]["data"]["attributes"]["product_id"],
        product.as_str()
    );
    let record = json_line(&run(&["questions", "get", &question, "--format", "jsonl"]));
    assert_eq!(
        (
            record["attributes"]["state"].as_str(),
            record["attributes"]["product_id"].as_str()
        ),
        (Some("banned"), Some(product.as_str()))
    );
}

/// Круг клиентов: создать (e-mail на example.com) → сменить имя и отписку → get → e-mail без
/// --upsert — код 2 → удалить.
#[test]
#[ignore]
fn consumer_write_round_trip() {
    if !writes_enabled() {
        return;
    }
    let stamp = unix_seconds();
    let external_id = format!("aplaut-cli-test-{stamp}-c");
    let email = format!("aplaut-cli-test+{stamp}@example.com");
    let _cleanup = Cleanup(format!("consumers/{external_id}"));
    json_line(&run(&[
        "consumers",
        "create",
        "--external-id",
        &external_id,
        "--email",
        &email,
        "--name",
        "aplaut-cli test",
        "--json",
    ]));
    json_line(&run(&[
        "consumers",
        "update",
        &external_id,
        "--first-name",
        "Тест",
        "--unsubscribed",
        "true",
        "--json",
    ]));
    let record = json_line(&run(&[
        "consumers",
        "get",
        &external_id,
        "--format",
        "jsonl",
    ]));
    assert_eq!(
        (
            record["attributes"]["email"].as_str(),
            record["attributes"]["first_name"].as_str(),
            record["attributes"]["unsubscribed"].as_bool()
        ),
        (Some(email.as_str()), Some("Тест"), Some(true))
    );
    let out = run(&[
        "consumers",
        "update",
        &external_id,
        "--email",
        "other@example.com",
    ]);
    assert_eq!(out.code, 2, "{}", out.stderr);
}

/// Круг заказов: создать со строкой → get с include=consumer (заказ создаёт клиента — удалить и
/// его) → сменить имя → get: имя новое, строки прежние → удалить заказ и клиента.
#[test]
#[ignore]
fn order_write_round_trip() {
    if !writes_enabled() {
        return;
    }
    let stamp = unix_seconds();
    let number = format!("aplaut-cli-test-{stamp}-o");
    let line_product = format!("aplaut-cli-test-{stamp}-p");
    let _order = Cleanup(format!("orders/{number}"));
    let lines = serde_json::json!({"order_lines": [
        {"product_id": line_product, "name": "aplaut-cli test", "price": 1}
    ]})
    .to_string();
    let email = format!("aplaut-cli-test+{stamp}-o@example.com");
    json_line(&run_with_stdin(
        &[
            "orders",
            "create",
            "--number",
            &number,
            "--consumer-email",
            &email,
            "--consumer-name",
            "aplaut-cli test",
            "--data",
            "-",
            "--json",
        ],
        &lines,
    ));
    let raw = json_line(&run(&["orders", "get", &number, "--include", "consumer"]));
    let consumer = raw["data"]["relationships"]["consumer"]["data"]["id"]
        .as_str()
        .expect("клиент заказа")
        .to_string();
    let _consumer = Cleanup(format!("consumers/{consumer}"));
    json_line(&run(&[
        "orders",
        "update",
        &number,
        "--consumer-name",
        "aplaut-cli test 2",
        "--json",
    ]));
    let record = json_line(&run(&["orders", "get", &number, "--format", "jsonl"]));
    assert_eq!(record["attributes"]["consumer_name"], "aplaut-cli test 2");
    assert_eq!(
        record["attributes"]["order_lines"][0]["product_id"],
        line_product.as_str(),
        "PUT частичный"
    );
}

/// Запись на стенд — только с APLAUT_E2E_WRITES=1.
fn writes_enabled() -> bool {
    let enabled = std::env::var("APLAUT_E2E_WRITES").as_deref() == Ok("1");
    if !enabled {
        eprintln!("пропущено: запись на стенд — только с APLAUT_E2E_WRITES=1");
    }
    enabled
}

fn unix_seconds() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

/// Удаляет тестовый объект по пути от base URL (`reviews/<external_id>`). Токен — из окружения
/// теста, не из argv.
struct Cleanup(String);

impl Drop for Cleanup {
    fn drop(&mut self) {
        let url = format!("{}/{}", base_url(), self.0);
        let result = ureq::delete(&url)
            .header("Authorization", format!("Bearer {}", token()))
            .header("Accept", "application/vnd.api+json")
            .call();
        match result {
            Ok(_) | Err(ureq::Error::StatusCode(404)) => {}
            Err(err) => eprintln!(
                "НЕ УДАЛЁН тестовый объект {}: {err} — удалите вручную",
                self.0
            ),
        }
    }
}

fn token() -> String {
    match std::env::var("APLAUT_ACCESS_TOKEN_FILE") {
        Ok(path) => std::fs::read_to_string(path).unwrap().trim().to_string(),
        Err(_) => std::env::var("APLAUT_ACCESS_TOKEN")
            .expect("задайте APLAUT_ACCESS_TOKEN_FILE или APLAUT_ACCESS_TOKEN"),
    }
}
