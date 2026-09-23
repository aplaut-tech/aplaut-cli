//! Контракт для агентов (спека agent mode): конверт `--json`, `result` каждой команды,
//! предупреждения в конверте, `--dry-run`.

mod support;

use std::path::Path;

use serde_json::{json, Value};
use support::{aplaut, first_page_json, review, MockServer, Output, Reply, TempDir};

const TOKEN: (&str, &str) = ("APLAUT_ACCESS_TOKEN", "tok");
const FILTER: &str = "updated_at:gte:2020-01-01T00:00:00Z";

fn envelope(line: &str) -> Value {
    serde_json::from_str(line).unwrap_or_else(|e| panic!("не JSON ({e}): {line}"))
}

/// Команда без данных: в stdout ровно одна строка-конверт, stderr пуст.
fn only_stdout(out: &Output) -> Value {
    assert_eq!(out.code, 0, "{}", out.stderr);
    assert_eq!(out.stderr, "", "с --json в stderr ничего лишнего");
    assert_eq!(out.stdout.lines().count(), 1, "{}", out.stdout);
    envelope(out.stdout.trim_end())
}

fn login(home: &Path, profile: &str) -> Value {
    only_stdout(&aplaut(
        home,
        &[
            "auth",
            "login",
            "--token-stdin",
            "--profile",
            profile,
            "--json",
        ],
        &[],
        "tok-secret",
    ))
}

#[test]
fn auth_login_and_logout_report_results_but_never_the_token() {
    let home = TempDir::new("agent-auth");
    let first = login(home.path(), "ci");
    assert_eq!(
        (first["ok"].as_bool(), first["command"].as_str()),
        (Some(true), Some("auth.login"))
    );
    let result = &first["result"];
    assert_eq!(
        (result["profile"].as_str(), result["token"].as_str()),
        (Some("ci"), Some("***"))
    );
    assert_eq!(
        (
            result["token_source"].as_str(),
            result["replaced"].as_bool()
        ),
        (Some("stdin"), Some(false))
    );
    assert!(result["credentials_path"]
        .as_str()
        .unwrap()
        .ends_with("credentials"));
    assert!(!first.to_string().contains("tok-secret"));
    assert_eq!(login(home.path(), "ci")["result"]["replaced"], true);
    let logout = ["auth", "logout", "--profile", "ci", "--json"];
    let out = aplaut(home.path(), &logout, &[], "");
    assert_eq!(
        only_stdout(&out)["result"],
        json!({"profile": "ci", "removed": true})
    );
    let again = aplaut(home.path(), &logout, &[], "");
    assert_eq!(only_stdout(&again)["result"]["removed"], false);
}

#[test]
fn profile_commands_return_documented_results() {
    let home = TempDir::new("agent-profile");
    let set = aplaut(
        home.path(),
        &[
            "profile",
            "set",
            "staging",
            "--base-url",
            "https://api.staging.example/v4",
            "--json",
        ],
        &[],
        "",
    );
    let v = only_stdout(&set);
    assert_eq!(v["command"], "profile.set");
    assert_eq!(v["result"]["created"], true);
    assert_eq!(
        v["result"]["changes"],
        json!([{"field": "base_url", "from": null, "to": "https://api.staging.example/v4"}])
    );
    assert!(v["result"]["config_path"]
        .as_str()
        .unwrap()
        .ends_with("config.toml"));
    let desc = only_stdout(&aplaut(
        home.path(),
        &[
            "profile",
            "set",
            "staging",
            "--description",
            "Стенд",
            "--json",
        ],
        &[],
        "",
    ));
    assert_eq!(desc["result"]["created"], false);
    assert_eq!(
        desc["result"]["changes"],
        json!([{"field": "description", "from": null, "to": "Стенд"}])
    );
    let list = only_stdout(&aplaut(
        home.path(),
        &["profile", "list", "--json"],
        &[],
        "",
    ));
    assert_eq!(list["result"]["profiles"][0]["name"], "staging");
    let get = only_stdout(&aplaut(
        home.path(),
        &["profile", "get", "staging", "--json"],
        &[],
        "",
    ));
    assert_eq!(get["result"]["description"], "Стенд");
    let delete = only_stdout(&aplaut(
        home.path(),
        &["profile", "delete", "staging", "--yes", "--json"],
        &[],
        "",
    ));
    assert_eq!(
        delete["result"],
        json!({"profile": "staging", "removed_config": true, "removed_token": false})
    );
}

#[test]
fn scroll_result_goes_to_stderr_and_data_stays_on_stdout() {
    let home = TempDir::new("agent-scroll");
    let server = MockServer::start(vec![Reply::json(
        200,
        first_page_json(&[review("r1", "t1")], None, false, 1, None),
    )]);
    let url = server.base_url();
    let out = aplaut(
        home.path(),
        &[
            "reviews",
            "scroll",
            "--filter",
            FILTER,
            "--base-url",
            &url,
            "--format",
            "jsonl",
            "--json",
        ],
        &[TOKEN],
        "",
    );
    assert_eq!(out.code, 0, "{}", out.stderr);
    assert_eq!(out.stdout.lines().count(), 1, "данные — в stdout");
    assert_eq!(
        out.stderr.lines().count(),
        1,
        "в stderr — только конверт: {}",
        out.stderr
    );
    let v = envelope(out.stderr.trim_end());
    assert_eq!(v["command"], "reviews.scroll");
    let r = &v["result"];
    assert_eq!(
        (
            r["records_type"].as_str(),
            r["emitted"].as_u64(),
            r["completed"].as_bool()
        ),
        (Some("reviews"), Some(1), Some(true))
    );
    assert_eq!(r["state_path"], Value::Null);
}

/// Предупреждение до сбоя — в конверте ошибки; `--verbose` оставляет отладку, конверт последний.
#[test]
fn warnings_go_into_the_envelope_even_when_the_run_fails() {
    let home = TempDir::new("agent-warn");
    let server = MockServer::start(vec![
        Reply::json(
            200,
            first_page_json(
                &[review("r1", "t1")],
                Some("c1"),
                true,
                2,
                Some("updated_at:gte:2026-08-24T10:09:35Z"),
            ),
        ),
        Reply::json(401, r#"{"errors":{"status":401,"title":"Unauthorized"}}"#),
    ]);
    let url = server.base_url();
    let out = aplaut(
        home.path(),
        &[
            "reviews",
            "scroll",
            "--base-url",
            &url,
            "--format",
            "jsonl",
            "--json",
            "--verbose",
        ],
        &[TOKEN],
        "",
    );
    assert_eq!(out.code, 4, "часть выдана: {}", out.stderr);
    assert_eq!(out.stdout.lines().count(), 1, "выданная запись цела");
    assert!(out.stderr.contains("debug:"), "--verbose оставляет отладку");
    let v = envelope(out.stderr.lines().last().unwrap());
    assert_eq!(v["ok"], false);
    assert_eq!(v["warnings"][0]["code"], "default_filter");
    assert!(
        !out.stderr
            .lines()
            .any(|l| l.contains("30 дней") && !l.starts_with('{')),
        "предупреждение не текстом: {}",
        out.stderr
    );
}

#[test]
fn parse_error_with_json_is_an_envelope() {
    let home = TempDir::new("agent-clap");
    let out = aplaut(
        home.path(),
        &["reviews", "scroll", "--json", "--bogus"],
        &[],
        "",
    );
    assert_eq!(out.code, 2);
    let v = envelope(out.stderr.lines().last().unwrap());
    assert_eq!(
        (v["command"].as_str(), v["error"]["code"].as_str()),
        (Some("aplaut"), Some("usage"))
    );
}

/// Все файлы каталога конфигурации: имя → байты.
fn snapshot(home: &Path) -> Vec<(String, Vec<u8>)> {
    let dir = home.join("config/aplaut");
    let mut files: Vec<(String, Vec<u8>)> = std::fs::read_dir(&dir)
        .map(|d| {
            d.filter_map(|e| e.ok())
                .map(|e| {
                    (
                        e.file_name().to_string_lossy().into_owned(),
                        std::fs::read(e.path()).unwrap(),
                    )
                })
                .collect()
        })
        .unwrap_or_default();
    files.sort();
    files
}

#[test]
fn dry_run_returns_the_plan_and_changes_nothing() {
    let home = TempDir::new("agent-dry");
    login(home.path(), "ci");
    let before = snapshot(home.path());
    for (args, command, stdin) in [
        (
            vec!["profile", "set", "ci", "--base-url", "https://x.example/v4"],
            "profile.set",
            "",
        ),
        (vec!["profile", "delete", "ci"], "profile.delete", ""),
        (
            vec!["auth", "login", "--token-stdin", "--profile", "ci"],
            "auth.login",
            "tok-new",
        ),
        (vec!["auth", "logout", "--profile", "ci"], "auth.logout", ""),
    ] {
        let mut full = args.clone();
        full.extend(["--dry-run", "--json"]);
        let v = only_stdout(&aplaut(home.path(), &full, &[], stdin));
        assert_eq!(
            (v["command"].as_str(), v["dry_run"].as_bool()),
            (Some(command), Some(true))
        );
        assert!(!v["result"].is_null(), "{command}: план — это result");
        assert_eq!(snapshot(home.path()), before, "{command} ничего не изменил");
    }
}

#[test]
fn dry_run_in_an_empty_home_creates_nothing() {
    let home = TempDir::new("agent-dry-empty");
    let text = aplaut(
        home.path(),
        &[
            "profile",
            "set",
            "x",
            "--base-url",
            "https://x.example/v4",
            "--dry-run",
        ],
        &[],
        "",
    );
    assert_eq!(text.code, 0, "{}", text.stderr);
    assert!(
        text.stderr
            .starts_with("Пробный запуск, ничего не изменено:"),
        "{}",
        text.stderr
    );
    let login = aplaut(
        home.path(),
        &[
            "auth",
            "login",
            "--token-stdin",
            "--profile",
            "x",
            "--dry-run",
            "--json",
        ],
        &[],
        "tok",
    );
    assert_eq!(only_stdout(&login)["result"]["token"], "***");
    assert!(
        !home.path().join("config").exists(),
        "каталог конфигурации не создан"
    );
}

/// План или настоящий запуск — агент должен знать и тогда, когда команда упала; `-n` — то же,
/// что `--dry-run`.
#[test]
fn failure_envelope_reports_dry_run_for_both_spellings() {
    let home = TempDir::new("agent-dry-fail");
    for (flag, expected) in [("-n", true), ("--dry-run", true), ("--yes", false)] {
        let out = aplaut(
            home.path(),
            &["profile", "delete", "ghost", flag, "--json"],
            &[],
            "",
        );
        assert_eq!(out.code, 5, "{flag}: {}", out.stderr);
        let v = envelope(out.stderr.lines().last().unwrap());
        assert_eq!(v["dry_run"], expected, "{flag}");
    }
}
