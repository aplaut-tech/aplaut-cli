//! Smoke против живого стенда; в обычный `cargo test` не входит (все тесты `#[ignore]`).
//!
//!   APLAUT_E2E_BASE_URL=https://api.staging-old.aplaut.net/v4 \
//!   APLAUT_ACCESS_TOKEN_FILE=… cargo test --test e2e -- --ignored --test-threads=1
//!
//! Токен — из APLAUT_ACCESS_TOKEN_FILE или APLAUT_ACCESS_TOKEN; адрес стенда в коде не хранится.

mod support;

use support::{aplaut, Output, TempDir};

const FILTER: &str = "updated_at:gte:2020-01-01T00:00:00Z";

fn base_url() -> String {
    std::env::var("APLAUT_E2E_BASE_URL").expect("задайте APLAUT_E2E_BASE_URL")
}

fn run(args: &[&str]) -> Output {
    let home = TempDir::new("e2e");
    let base = base_url();
    let mut env: Vec<(String, String)> = vec![("APLAUT_BASE_URL".into(), base)];
    for key in ["APLAUT_ACCESS_TOKEN_FILE", "APLAUT_ACCESS_TOKEN"] {
        if let Ok(value) = std::env::var(key) {
            env.push((key.into(), value));
        }
    }
    assert!(env.len() > 1, "задайте APLAUT_ACCESS_TOKEN_FILE или APLAUT_ACCESS_TOKEN");
    let env: Vec<(&str, &str)> = env.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
    aplaut(home.path(), args, &env, "")
}

#[test]
#[ignore]
fn reviews_jsonl_with_included_objects() {
    let out = run(&["reviews", "scroll", "--filter", FILTER, "--per-page", "5", "--max-records", "10", "--include", "author,product", "--format", "jsonl"]);
    assert_eq!(out.code, 0, "{}", out.stderr);
    let records: Vec<serde_json::Value> = out.stdout.lines().map(|l| serde_json::from_str(l).unwrap()).collect();
    assert!(!records.is_empty() && records.len() <= 10);
    assert!(records.iter().all(|r| r["type"] == "reviews" && r["id"].is_string()));
}

#[test]
#[ignore]
fn products_csv_has_header() {
    let out = run(&["products", "scroll", "--filter", FILTER, "--per-page", "5", "--max-records", "5", "--format", "csv"]);
    assert_eq!(out.code, 0, "{}", out.stderr);
    assert!(out.stdout.starts_with("id,type,"), "{}", out.stdout);
}

#[test]
#[ignore]
fn questions_raw_is_one_page_per_line() {
    let out = run(&["questions", "scroll", "--filter", FILTER, "--per-page", "3", "--max-records", "3"]);
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
    let out = aplaut(home.path(), &["reviews", "scroll", "--filter", FILTER, "--max-records", "1"], &[("APLAUT_BASE_URL", &base), ("APLAUT_ACCESS_TOKEN", "definitely-not-a-token")], "");
    assert_eq!(out.code, 3, "{}", out.stderr);
    let err = out.error_json();
    // Два вида 401 на стейджинге: Doorkeeper (`error="invalid_token"`) для токена правильного
    // формата и `Token realm="Aplaut Platform API v4"` без `error` — для строки не того формата.
    let code = err["error"]["code"].as_str().unwrap();
    assert!(code == "invalid_token" || code == "unauthorized", "{err}");
    assert!(err["error"]["request_id"].is_string());
    assert!(err["error"]["hint"].as_str().unwrap().contains("APLAUT_ACCESS_TOKEN"));
}
