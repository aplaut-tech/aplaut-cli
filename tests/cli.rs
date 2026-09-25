mod support;

use support::{aplaut, TempDir};

#[test]
fn version_shows_cli_and_spec_versions() {
    let home = TempDir::new("version");
    let out = aplaut(home.path(), &["--version"], &[], "");
    assert_eq!(out.code, 0, "{}", out.stderr);
    assert_eq!(out.stdout, "aplaut 0.4.0 (Platform API 4.1.0)\n");
}

#[test]
fn short_version_flag_is_not_supported() {
    let home = TempDir::new("short-version");
    let out = aplaut(home.path(), &["-V"], &[], "");
    assert_eq!(out.code, 2);
}

use std::fs;
use std::io::{BufRead, BufReader};
use std::os::unix::fs::PermissionsExt;

use support::{first_page_json, page_json, review, MockServer, Reply};

const FILTER: &str = "updated_at:gte:2020-01-01T00:00:00Z";

fn one_page() -> Reply {
    Reply::json(
        200,
        first_page_json(&[review("r1", "t1")], None, false, 1, None),
    )
}

fn scroll<'a>(resource: &'a str, base_url: &'a str, extra: &[&'a str]) -> Vec<&'a str> {
    let mut args = vec![
        resource,
        "scroll",
        "--filter",
        FILTER,
        "--base-url",
        base_url,
    ];
    args.extend_from_slice(extra);
    args
}

#[test]
fn no_args_prints_help_with_examples_and_exits_2() {
    let home = TempDir::new("help");
    let out = aplaut(home.path(), &[], &[], "");
    assert_eq!(out.code, 2);
    assert!(
        out.stderr.contains("Примеры:")
            && out.stderr.contains("reviews")
            && out.stderr.contains("support@aplaut.com"),
        "{}",
        out.stderr
    );
}

#[test]
fn resource_without_verb_prints_help() {
    let home = TempDir::new("help-resource");
    let out = aplaut(home.path(), &["reviews"], &[], "");
    assert_eq!(out.code, 2);
    assert!(out.stderr.contains("scroll"), "{}", out.stderr);
}

#[test]
fn help_subcommand_and_long_help_lead_with_examples() {
    let home = TempDir::new("help-sub");
    let out = aplaut(home.path(), &["help", "reviews", "scroll"], &[], "");
    assert_eq!(out.code, 0, "{}", out.stderr);
    assert!(
        out.stdout.contains("--filter")
            && out.stdout.contains("Примеры:")
            && out.stdout.contains("30 дней")
    );
}

#[test]
fn typo_suggests_and_is_json_without_tty() {
    let home = TempDir::new("typo");
    let out = aplaut(home.path(), &["review", "scroll"], &[], "");
    assert_eq!(out.code, 2);
    let err = out.error_json();
    assert_eq!(err["error"]["code"], "usage");
    assert!(
        err["error"]["message"]
            .as_str()
            .unwrap()
            .contains("reviews"),
        "{err}"
    );
}

#[test]
fn invalid_include_fails_before_any_request() {
    let home = TempDir::new("include");
    let server = MockServer::start(vec![]);
    let url = server.base_url();
    let out = aplaut(
        home.path(),
        &scroll("reviews", &url, &["--include", "nope"]),
        &[("APLAUT_ACCESS_TOKEN", "tok")],
        "",
    );
    assert_eq!(out.code, 2);
    let err = out.error_json();
    assert_eq!(
        (
            err["error"]["code"].as_str(),
            err["error"]["field"].as_str()
        ),
        (Some("invalid_include"), Some("include"))
    );
    assert_eq!(err["command"], "reviews.scroll");
    assert!(server.requests().is_empty());
}

#[test]
fn remote_http_base_url_is_rejected() {
    let home = TempDir::new("http");
    let out = aplaut(
        home.path(),
        &scroll("reviews", "http://example.com/v4", &[]),
        &[("APLAUT_ACCESS_TOKEN", "tok")],
        "",
    );
    assert_eq!(out.code, 2);
    assert_eq!(out.error_json()["error"]["code"], "insecure_base_url");
}

#[test]
fn missing_token_exits_3() {
    let home = TempDir::new("notoken");
    let server = MockServer::start(vec![]);
    let url = server.base_url();
    let out = aplaut(home.path(), &scroll("reviews", &url, &[]), &[], "");
    assert_eq!(out.code, 3);
    assert_eq!(out.error_json()["error"]["code"], "no_token");
    assert!(server.requests().is_empty());
}

#[test]
fn scroll_jsonl_end_to_end_with_state() {
    let home = TempDir::new("e2e-mock");
    let server = MockServer::start(vec![
        Reply::json(
            200,
            first_page_json(
                &[review("r1", "t1"), review("r2", "t2")],
                Some("c1"),
                true,
                3,
                None,
            ),
        ),
        Reply::json(200, page_json(&[review("r3", "t3")], None, false)),
    ]);
    let url = server.base_url();
    let state = home.path().join("reviews.state.json");
    let state_arg = state.to_str().unwrap();
    let out = aplaut(
        home.path(),
        &scroll(
            "reviews",
            &url,
            &["--format", "jsonl", "--state", state_arg],
        ),
        &[("APLAUT_ACCESS_TOKEN", "tok")],
        "",
    );
    assert_eq!(out.code, 0, "{}", out.stderr);
    assert_eq!(out.stdout.lines().count(), 3);
    assert!(out.stderr.contains("обход завершён"), "{}", out.stderr);
    let saved: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&state).unwrap()).unwrap();
    assert_eq!(
        (saved["completed"].as_bool(), saved["emitted"].as_u64()),
        (Some(true), Some(3))
    );
    assert_eq!(
        fs::metadata(&state).unwrap().permissions().mode() & 0o777,
        0o600
    );
    assert_eq!(
        server.requests()[0].header("authorization"),
        Some("Bearer tok")
    );
}

#[test]
fn default_format_is_raw_one_line_per_page() {
    let home = TempDir::new("raw");
    let server = MockServer::start(vec![one_page()]);
    let url = server.base_url();
    let out = aplaut(
        home.path(),
        &scroll("products", &url, &[]),
        &[("APLAUT_ACCESS_TOKEN", "tok")],
        "",
    );
    assert_eq!(out.code, 0, "{}", out.stderr);
    assert_eq!(out.stdout.lines().count(), 1);
    let page: serde_json::Value = serde_json::from_str(out.stdout.trim_end()).unwrap();
    assert_eq!(page["meta"]["has_more"], false);
    assert_eq!(server.requests()[0].path, "/v4/scroll/products");
}

#[test]
fn csv_format_writes_header_and_rows() {
    let home = TempDir::new("csv");
    let server = MockServer::start(vec![Reply::json(
        200,
        first_page_json(
            &[review("r1", "t1"), review("r2", "t2")],
            None,
            false,
            2,
            None,
        ),
    )]);
    let url = server.base_url();
    let out = aplaut(
        home.path(),
        &scroll("reviews", &url, &["--format", "csv"]),
        &[("APLAUT_ACCESS_TOKEN", "tok")],
        "",
    );
    assert_eq!(out.code, 0, "{}", out.stderr);
    let lines: Vec<&str> = out.stdout.lines().collect();
    assert_eq!(
        lines[0],
        "id,type,body,rating,updated_at,author_ref,product_ref"
    );
    assert_eq!(lines[1], "r1,reviews,Отзыв r1,5.0,t1,,p-r1");
    assert_eq!(lines.len(), 3);
}

fn two_reviews() -> MockServer {
    MockServer::start(vec![Reply::json(
        200,
        first_page_json(
            &[review("r1", "t1"), review("r2", "t2")],
            None,
            false,
            2,
            None,
        ),
    )])
}

#[test]
fn csv_fields_select_and_order_columns() {
    let home = TempDir::new("csv-fields");
    let server = two_reviews();
    let url = server.base_url();
    let out = aplaut(
        home.path(),
        &scroll(
            "reviews",
            &url,
            &["--format", "csv", "--fields", "rating,id,product_ref"],
        ),
        &[("APLAUT_ACCESS_TOKEN", "tok")],
        "",
    );
    assert_eq!(out.code, 0, "{}", out.stderr);
    assert_eq!(
        out.stdout,
        "rating,id,product_ref\n5.0,r1,p-r1\n5.0,r2,p-r2\n"
    );
    assert!(
        server.requests()[0].query_param("fields").is_none(),
        "параметра fields в API нет"
    );
}

#[test]
fn fields_typo_is_a_usage_error_with_empty_stdout() {
    let home = TempDir::new("csv-fields-typo");
    let server = two_reviews();
    let url = server.base_url();
    let state = home.path().join("reviews.state.json");
    let out = aplaut(
        home.path(),
        &scroll(
            "reviews",
            &url,
            &[
                "--format",
                "csv",
                "--fields",
                "id,raiting",
                "--state",
                state.to_str().unwrap(),
            ],
        ),
        &[("APLAUT_ACCESS_TOKEN", "tok")],
        "",
    );
    assert_eq!(out.code, 2, "{}", out.stderr);
    let err = out.error_json();
    assert_eq!(err["error"]["code"], "unknown_field");
    assert_eq!(err["error"]["field"], "fields");
    assert!(!state.exists(), "ответ на открытие не записан — стейта нет");
    assert!(err["error"]["hint"]
        .as_str()
        .unwrap()
        .starts_with("может, rating?"));
    assert_eq!(out.stdout, "");
}

#[test]
fn fields_need_csv_and_their_includes_before_any_request() {
    let home = TempDir::new("csv-fields-local");
    let server = MockServer::start(vec![]);
    let url = server.base_url();
    for (extra, code) in [
        (&["--fields", "id,rating"][..], "fields_need_tabular_format"),
        (
            &["--format", "jsonl", "--fields", "id"][..],
            "fields_need_tabular_format",
        ),
        (
            &["--format", "csv", "--fields", "id,product.name"][..],
            "field_needs_include",
        ),
        (
            &["--format", "csv", "--fields", "id,,rating"][..],
            "invalid_fields",
        ),
        (
            &["--format", "csv", "--fields", "id,rating,id"][..],
            "duplicate_field",
        ),
    ] {
        let out = aplaut(
            home.path(),
            &scroll("reviews", &url, extra),
            &[("APLAUT_ACCESS_TOKEN", "tok")],
            "",
        );
        assert_eq!(out.code, 2, "{extra:?}: {}", out.stderr);
        assert_eq!(out.error_json()["error"]["code"], code, "{extra:?}");
    }
    assert!(server.requests().is_empty(), "квота открытий не потрачена");
}

/// Колонки проверены на первой странице обхода; другой список при продолжении склеил бы CSV
/// с разной раскладкой.
#[test]
fn resume_with_other_fields_is_rejected_before_any_request() {
    let home = TempDir::new("csv-fields-resume");
    let server = MockServer::start(vec![Reply::json(
        200,
        first_page_json(&[review("r1", "t1")], Some("c1"), true, 2, None),
    )]);
    let url = server.base_url();
    let state = home.path().join("reviews.state.json");
    let state = state.to_str().unwrap();
    let run = |fields: &str| {
        aplaut(
            home.path(),
            &scroll(
                "reviews",
                &url,
                &[
                    "--format",
                    "csv",
                    "--fields",
                    fields,
                    "--state",
                    state,
                    "--max-records",
                    "1",
                ],
            ),
            &[("APLAUT_ACCESS_TOKEN", "tok")],
            "",
        )
    };
    let first = run("id,rating");
    assert_eq!(first.code, 0, "{}", first.stderr);
    let other = run("body,id");
    assert_eq!(other.code, 2, "{}", other.stderr);
    let err = other.error_json();
    assert_eq!(err["error"]["code"], "state_mismatch");
    assert!(
        err["error"]["message"].as_str().unwrap().contains("fields"),
        "{err}"
    );
    assert_eq!(server.requests().len(), 1, "второй запуск не ходил в API");
}

#[test]
fn fields_without_id_warn_that_duplicates_cannot_be_removed() {
    let home = TempDir::new("csv-fields-no-id");
    let url_of = |server: &MockServer| server.base_url();
    // Без --state повторов между запусками нет — и предупреждать не о чем.
    let plain = two_reviews();
    let out = aplaut(
        home.path(),
        &scroll(
            "reviews",
            &url_of(&plain),
            &["--format", "csv", "--fields", "rating"],
        ),
        &[("APLAUT_ACCESS_TOKEN", "tok")],
        "",
    );
    assert_eq!(out.code, 0, "{}", out.stderr);
    assert!(!out.stderr.contains("нет id"), "{}", out.stderr);
    let server = two_reviews();
    let state = home.path().join("reviews.state.json");
    let state = state.to_str().unwrap();
    let out = aplaut(
        home.path(),
        &scroll(
            "reviews",
            &url_of(&server),
            &["--format", "csv", "--fields", "rating", "--state", state],
        ),
        &[("APLAUT_ACCESS_TOKEN", "tok")],
        "",
    );
    assert_eq!(out.code, 0, "{}", out.stderr);
    assert!(out.stderr.contains("нет id"), "{}", out.stderr);
}

#[test]
fn partial_failure_exits_4_with_flushed_stdout() {
    let home = TempDir::new("partial");
    let server = MockServer::start(vec![
        Reply::json(
            200,
            first_page_json(
                &[review("r1", "t1"), review("r2", "t2")],
                Some("c1"),
                true,
                9,
                None,
            ),
        ),
        Reply::Hangup,
    ]);
    let url = server.base_url();
    let state = home.path().join("s.json");
    let out = aplaut(
        home.path(),
        &scroll(
            "reviews",
            &url,
            &[
                "--format",
                "jsonl",
                "--state",
                state.to_str().unwrap(),
                "--max-retries",
                "0",
            ],
        ),
        &[("APLAUT_ACCESS_TOKEN", "tok")],
        "",
    );
    assert_eq!(out.code, 4, "{}", out.stderr);
    assert_eq!(
        out.stdout.lines().count(),
        2,
        "страница до сбоя должна быть в stdout"
    );
    let err = out.error_json();
    assert_eq!(
        (err["ok"].as_bool(), err["error"]["code"].as_str()),
        (Some(false), Some("scroll_interrupted"))
    );
    let saved: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&state).unwrap()).unwrap();
    assert_eq!(saved["cursor"], "c1");
}

#[test]
fn closed_stdout_stops_with_partial_exit() {
    let home = TempDir::new("epipe");
    let server = MockServer::start(vec![
        Reply::json(
            200,
            first_page_json(&[review("r1", "t1")], Some("c1"), true, 3, None),
        ),
        Reply::json(200, page_json(&[review("r2", "t2")], Some("c2"), true)),
        Reply::json(200, page_json(&[review("r3", "t3")], None, false)),
    ]);
    let url = server.base_url();
    let state = home.path().join("s.json");
    let args = scroll(
        "reviews",
        &url,
        &["--format", "jsonl", "--state", state.to_str().unwrap()],
    );
    let mut child = support::spawn(home.path(), &args, &[("APLAUT_ACCESS_TOKEN", "tok")]);
    let mut stdout = BufReader::new(child.stdout.take().unwrap());
    let mut first = String::new();
    stdout.read_line(&mut first).unwrap();
    drop(stdout);
    let out = child.wait_with_output().unwrap();
    assert_eq!(out.status.code(), Some(4));
    let stderr = String::from_utf8_lossy(&out.stderr);
    let last: serde_json::Value = serde_json::from_str(stderr.lines().last().unwrap()).unwrap();
    assert_eq!(last["error"]["code"], "output_closed");
    let saved: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&state).unwrap()).unwrap();
    assert_eq!(
        saved["cursor"], "c1",
        "стейт не должен уйти дальше отданного"
    );
}

#[test]
fn verbose_never_leaks_token() {
    let home = TempDir::new("verbose");
    let token = "tok-verbose-secret-9f8e";
    let server = MockServer::start(vec![
        one_page(),
        Reply::text(401, "").with_header(
            "WWW-Authenticate",
            r#"Bearer realm="Doorkeeper", error="invalid_token""#,
        ),
    ]);
    let url = server.base_url();
    let ok = aplaut(
        home.path(),
        &scroll("reviews", &url, &["--verbose"]),
        &[("APLAUT_ACCESS_TOKEN", token)],
        "",
    );
    let denied = aplaut(
        home.path(),
        &scroll("reviews", &url, &["--verbose"]),
        &[("APLAUT_ACCESS_TOKEN", token)],
        "",
    );
    assert_eq!((ok.code, denied.code), (0, 3));
    for out in [&ok, &denied] {
        assert!(
            !out.stdout.contains(token) && !out.stderr.contains(token),
            "{}",
            out.stderr
        );
    }
    assert!(ok.stderr.contains("Bearer ***"), "{}", ok.stderr);
    assert_eq!(denied.error_json()["error"]["code"], "invalid_token");
}

#[test]
fn quiet_hides_summary() {
    let home = TempDir::new("quiet");
    let server = MockServer::start(vec![one_page()]);
    let url = server.base_url();
    let out = aplaut(
        home.path(),
        &scroll("reviews", &url, &["-q"]),
        &[("APLAUT_ACCESS_TOKEN", "tok")],
        "",
    );
    assert_eq!(out.code, 0);
    assert_eq!(out.stderr, "");
}

#[test]
fn login_from_stdin_then_scroll_uses_profile_token_and_base_url() {
    let home = TempDir::new("login");
    let server = MockServer::start(vec![one_page()]);
    let url = server.base_url();
    let login = aplaut(
        home.path(),
        &[
            "auth",
            "login",
            "--token-stdin",
            "--profile",
            "ci",
            "--base-url",
            &url,
        ],
        &[],
        "tok-ci\n",
    );
    assert_eq!(login.code, 0, "{}", login.stderr);
    let creds = home.path().join("config/aplaut/credentials");
    assert_eq!(
        fs::metadata(&creds).unwrap().permissions().mode() & 0o777,
        0o600
    );
    assert!(
        fs::read_to_string(home.path().join("config/aplaut/config.toml"))
            .unwrap()
            .contains(&url)
    );
    assert!(!login.stderr.contains("tok-ci"));
    let out = aplaut(
        home.path(),
        &["reviews", "scroll", "--filter", FILTER, "--profile", "ci"],
        &[],
        "",
    );
    assert_eq!(out.code, 0, "{}", out.stderr);
    assert_eq!(
        server.requests()[0].header("authorization"),
        Some("Bearer tok-ci")
    );
}

#[test]
fn profile_flag_beats_env_token_and_token_file_env_beats_token_env() {
    let home = TempDir::new("precedence");
    let server = MockServer::start(vec![one_page(), one_page()]);
    let url = server.base_url();
    assert_eq!(
        aplaut(
            home.path(),
            &["auth", "login", "--token-stdin", "--profile", "ci"],
            &[],
            "tok-ci"
        )
        .code,
        0
    );
    let file = home.path().join("token");
    fs::write(&file, "file-tok\n").unwrap();
    let a = aplaut(
        home.path(),
        &scroll("reviews", &url, &["--profile", "ci"]),
        &[("APLAUT_ACCESS_TOKEN", "env-tok")],
        "",
    );
    let b = aplaut(
        home.path(),
        &scroll("reviews", &url, &[]),
        &[
            ("APLAUT_ACCESS_TOKEN", "env-tok"),
            ("APLAUT_ACCESS_TOKEN_FILE", file.to_str().unwrap()),
        ],
        "",
    );
    assert_eq!((a.code, b.code), (0, 0), "{} {}", a.stderr, b.stderr);
    let reqs = server.requests();
    assert_eq!(reqs[0].header("authorization"), Some("Bearer tok-ci"));
    assert_eq!(reqs[1].header("authorization"), Some("Bearer file-tok"));
}

#[test]
fn logout_removes_token_and_is_idempotent() {
    let home = TempDir::new("logout");
    assert_eq!(
        aplaut(home.path(), &["auth", "login", "--token-stdin"], &[], "tok").code,
        0
    );
    let first = aplaut(home.path(), &["auth", "logout"], &[], "");
    let second = aplaut(home.path(), &["auth", "logout"], &[], "");
    assert_eq!((first.code, second.code), (0, 0));
    assert!(
        !fs::read_to_string(home.path().join("config/aplaut/credentials"))
            .unwrap()
            .contains("tok")
    );
    assert!(second.stderr.contains("нечего"), "{}", second.stderr);
}

#[test]
fn login_without_terminal_or_token_flags_is_usage_error() {
    let home = TempDir::new("login-notty");
    let out = aplaut(home.path(), &["auth", "login"], &[], "");
    assert_eq!(out.code, 2);
    assert_eq!(out.error_json()["error"]["code"], "token_required");
}

#[test]
fn state_file_for_other_resource_is_usage_error() {
    let home = TempDir::new("state-other");
    let server = MockServer::start(vec![one_page()]);
    let url = server.base_url();
    let state = home.path().join("s.json");
    let s = state.to_str().unwrap();
    assert_eq!(
        aplaut(
            home.path(),
            &scroll("reviews", &url, &["--state", s]),
            &[("APLAUT_ACCESS_TOKEN", "tok")],
            ""
        )
        .code,
        0
    );
    let out = aplaut(
        home.path(),
        &scroll("products", &url, &["--state", s]),
        &[("APLAUT_ACCESS_TOKEN", "tok")],
        "",
    );
    assert_eq!(out.code, 2);
    assert_eq!(out.error_json()["error"]["code"], "state_mismatch");
}

#[test]
fn env_token_works_despite_broken_credentials_file() {
    let home = TempDir::new("broken-creds");
    let dir = home.path().join("config/aplaut");
    fs::create_dir_all(&dir).unwrap();
    let creds = dir.join("credentials");
    fs::write(&creds, "[profiles]\ndefault = \"stored-secret-token\"\n").unwrap();
    fs::set_permissions(&creds, fs::Permissions::from_mode(0o600)).unwrap();
    let server = MockServer::start(vec![one_page()]);
    let url = server.base_url();
    let out = aplaut(
        home.path(),
        &scroll("reviews", &url, &[]),
        &[("APLAUT_ACCESS_TOKEN", "tok")],
        "",
    );
    assert_eq!(out.code, 0, "{}", out.stderr);
    assert!(
        !out.stderr.contains("stored-secret-token"),
        "{}",
        out.stderr
    );
    assert_eq!(
        server.requests()[0].header("authorization"),
        Some("Bearer tok")
    );
}

#[test]
fn exit_4_guidance_does_not_tell_state_users_to_roll_back() {
    // Со --state стейт уже сдвинут за выданные страницы: откат + повторный запуск = дыра в данных.
    let home = TempDir::new("help-exit4");
    let out = aplaut(home.path(), &["reviews", "scroll", "--help"], &[], "");
    assert_eq!(out.code, 0);
    let help = out.stdout.replace('\n', " ");
    assert!(!help.contains("откатите загрузку)"), "{}", out.stdout);
    assert!(
        help.contains("со --state оставьте полученное"),
        "{}",
        out.stdout
    );
    assert!(help.contains("без --state"), "{}", out.stdout);
}

#[test]
fn loopback_base_url_bypasses_proxy_from_env() {
    // http к localhost идёт открытым текстом: через прокси это отдало бы ему токен.
    let home = TempDir::new("proxy");
    let proxy = MockServer::start(vec![]);
    let server = MockServer::start(vec![one_page()]);
    let url = server.base_url();
    let proxy_url = proxy.origin();
    let env = [
        ("APLAUT_ACCESS_TOKEN", "tok-proxy"),
        ("HTTP_PROXY", proxy_url.as_str()),
        ("HTTPS_PROXY", proxy_url.as_str()),
        ("ALL_PROXY", proxy_url.as_str()),
    ];
    let out = aplaut(
        home.path(),
        &scroll("reviews", &url, &["--max-retries", "0"]),
        &env,
        "",
    );
    assert!(
        proxy.requests().is_empty(),
        "прокси получил запрос: {:?}",
        proxy.requests()
    );
    assert_eq!(out.code, 0, "{}", out.stderr);
    assert_eq!(server.requests().len(), 1);
}

#[test]
fn max_records_help_says_the_boundary_is_a_page() {
    // Обрезать страницу нельзя: остаток пришлось бы добирать повтором курсора, а он
    // не идемпотентен — страница бы потерялась.
    let home = TempDir::new("help-max");
    let out = aplaut(home.path(), &["reviews", "scroll", "--help"], &[], "");
    assert!(
        out.stdout.replace('\n', " ").contains("граница — страница"),
        "{}",
        out.stdout
    );
}
