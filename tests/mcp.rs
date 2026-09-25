//! MCP-сервер `aplaut mcp` (спека mcp-server): протокол, инструменты и вызовы — через настоящий
//! бинарь, JSON-RPC строками через stdin/stdout.

mod support;

use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

use serde_json::{json, Value};
use support::{aplaut, first_page_json, page_json, review, MockServer, Reply, TempDir};

const TOKEN: (&str, &str) = ("APLAUT_ACCESS_TOKEN", "tok");

/// MCP-клиент для тестов: `aplaut mcp` с чистым окружением, рабочий каталог — `home`.
struct Client {
    child: Child,
    stdin: Option<ChildStdin>,
    stdout: BufReader<ChildStdout>,
    next_id: u64,
    init: Value,
}

impl Client {
    fn start(home: &Path, args: &[&str], env: &[(&str, &str)]) -> Client {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_aplaut"));
        cmd.arg("mcp")
            .args(args)
            .env_clear()
            .env("HOME", home)
            .env("XDG_CONFIG_HOME", home.join("config"))
            .env("PATH", std::env::var("PATH").unwrap_or_default())
            .current_dir(home)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit());
        for (k, v) in env {
            cmd.env(k, v);
        }
        let mut child = cmd.spawn().expect("запуск aplaut mcp");
        let stdin = child.stdin.take();
        let stdout = BufReader::new(child.stdout.take().expect("stdout"));
        let mut client = Client {
            child,
            stdin,
            stdout,
            next_id: 0,
            init: Value::Null,
        };
        client.init = client.request(
            "initialize",
            json!({"protocolVersion": "2025-06-18", "capabilities": {},
                   "clientInfo": {"name": "test", "version": "0"}}),
        );
        assert!(client.init.get("result").is_some(), "{}", client.init);
        client.send(json!({"jsonrpc": "2.0", "method": "notifications/initialized"}));
        client
    }

    fn init(&self) -> &Value {
        &self.init["result"]
    }

    fn request(&mut self, method: &str, params: Value) -> Value {
        self.next_id += 1;
        let id = self.next_id;
        self.send(json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}));
        let mut line = String::new();
        self.stdout.read_line(&mut line).expect("ответ сервера");
        let response: Value =
            serde_json::from_str(&line).unwrap_or_else(|e| panic!("не JSON ({e}): {line}"));
        assert_eq!(response["id"], id, "{response}");
        response
    }

    fn call(&mut self, tool: &str, arguments: Value) -> Value {
        self.request("tools/call", json!({"name": tool, "arguments": arguments}))
    }

    fn send(&mut self, message: Value) {
        let stdin = self.stdin.as_mut().expect("stdin открыт");
        writeln!(stdin, "{message}").unwrap();
        stdin.flush().unwrap();
    }

    /// Закрыть stdin и дождаться выхода; код выхода сервера.
    fn finish(mut self) -> i32 {
        drop(self.stdin.take());
        self.child.wait().unwrap().code().unwrap_or(-1)
    }
}

#[test]
fn initialize_names_the_server_its_profile_and_mode() {
    let home = TempDir::new("mcp-init");
    let client = Client::start(
        home.path(),
        &[
            "--profile",
            "staging",
            "--base-url",
            "https://api.example.test/v4",
        ],
        &[TOKEN],
    );
    let init = client.init();
    assert_eq!(init["serverInfo"]["name"], "aplaut");
    assert_eq!(init["serverInfo"]["version"], env!("CARGO_PKG_VERSION"));
    assert!(init["capabilities"]["tools"].is_object(), "{init}");
    let instructions = init["instructions"].as_str().unwrap();
    for needle in [
        "Профиль staging",
        "https://api.example.test/v4",
        "Запись выключена",
        "docs/automation.md",
    ] {
        assert!(
            instructions.contains(needle),
            "нет «{needle}»: {instructions}"
        );
    }
    assert_eq!(client.finish(), 0, "закрытый stdin — штатное завершение");
}

#[test]
fn allow_writes_is_announced_in_instructions() {
    let home = TempDir::new("mcp-init-writes");
    let client = Client::start(home.path(), &["--allow-writes"], &[TOKEN]);
    let instructions = client.init()["instructions"].as_str().unwrap().to_string();
    assert!(instructions.contains("Запись включена"), "{instructions}");
    assert_eq!(client.finish(), 0);
}

#[test]
fn token_from_stdin_is_refused_before_serving() {
    let home = TempDir::new("mcp-token-stdin");
    for (flags, field) in [
        (&["mcp", "--token-stdin"][..], "token_stdin"),
        (&["mcp", "--token-file", "-"][..], "token_file"),
    ] {
        let out = aplaut(home.path(), flags, &[], "tok");
        assert_eq!(out.code, 2, "{flags:?}: {}", out.stderr);
        let err = out.error_json();
        assert_eq!(
            (
                err["error"]["code"].as_str(),
                err["error"]["field"].as_str()
            ),
            (Some("usage"), Some(field)),
            "{err}"
        );
        assert_eq!(out.stdout, "", "stdout принадлежит протоколу");
    }
}

#[test]
fn closed_stdin_before_initialize_is_a_handshake_error() {
    let home = TempDir::new("mcp-handshake");
    let out = aplaut(home.path(), &["mcp"], &[TOKEN], "");
    assert_eq!(out.code, 1, "{}", out.stderr);
    assert_eq!(out.error_json()["error"]["code"], "mcp_handshake_failed");
}

fn tool_names(response: &Value) -> Vec<String> {
    response["result"]["tools"]
        .as_array()
        .unwrap_or_else(|| panic!("нет tools: {response}"))
        .iter()
        .map(|t| t["name"].as_str().unwrap().to_string())
        .collect()
}

fn tool<'a>(response: &'a Value, name: &str) -> &'a Value {
    response["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["name"] == name)
        .unwrap_or_else(|| panic!("нет {name}"))
}

/// Имена инструментов — публичный контракт (M13): список меняется осознанно.
#[test]
fn read_only_server_lists_scroll_and_get() {
    let home = TempDir::new("mcp-list");
    let mut client = Client::start(home.path(), &[], &[TOKEN]);
    let list = client.request("tools/list", json!({}));
    assert_eq!(
        tool_names(&list),
        [
            "reviews_scroll",
            "reviews_get",
            "products_scroll",
            "products_get",
            "questions_scroll",
            "questions_get"
        ]
    );
    assert_eq!(
        tool(&list, "reviews_get")["annotations"],
        json!({"readOnlyHint": true, "openWorldHint": true})
    );
    let scroll = tool(&list, "reviews_scroll");
    assert_eq!(
        scroll["annotations"]["readOnlyHint"], false,
        "пишет output_file и state"
    );
    assert_eq!(scroll["inputSchema"]["additionalProperties"], false);
    assert_eq!(client.finish(), 0);
}

#[test]
fn allow_writes_adds_write_tools() {
    let home = TempDir::new("mcp-list-writes");
    let mut client = Client::start(home.path(), &["--allow-writes"], &[TOKEN]);
    let list = client.request("tools/list", json!({}));
    assert_eq!(
        tool_names(&list),
        [
            "reviews_scroll",
            "reviews_get",
            "reviews_create",
            "reviews_comment",
            "products_scroll",
            "products_get",
            "products_create",
            "products_update",
            "questions_scroll",
            "questions_get"
        ]
    );
    assert_eq!(
        tool(&list, "products_update")["annotations"],
        json!({"readOnlyHint": false, "destructiveHint": true, "idempotentHint": true, "openWorldHint": true})
    );
    assert_eq!(
        tool(&list, "reviews_comment")["inputSchema"]["required"],
        json!(["review_id"])
    );
    assert_eq!(client.finish(), 0);
}

const FILTER: &str = "updated_at:gte:2020-01-01T00:00:00Z";

fn text(response: &Value, index: usize) -> String {
    response["result"]["content"][index]["text"]
        .as_str()
        .unwrap_or_else(|| panic!("нет content[{index}]: {response}"))
        .to_string()
}

fn envelope(response: &Value) -> Value {
    serde_json::from_str(&text(response, 0)).unwrap()
}

fn is_error(response: &Value) -> bool {
    response["result"]["isError"].as_bool().unwrap_or(false)
}

fn served(home: &Path, server: &MockServer, extra: &[&str]) -> Client {
    let url = server.base_url();
    let mut args = vec!["--base-url", url.as_str()];
    args.extend(extra);
    Client::start(home, &args, &[TOKEN])
}

fn created(id: &str) -> Reply {
    Reply::json(
        201,
        json!({"data": {"id": id, "type": "reviews", "attributes": {"state": "waiting"}}})
            .to_string(),
    )
}

#[test]
fn get_returns_envelope_and_record() {
    let home = TempDir::new("mcp-get");
    let server = MockServer::start(vec![Reply::json(
        200,
        json!({"data": review("r1", "2024-01-01T00:00:00Z")}).to_string(),
    )]);
    let mut client = served(home.path(), &server, &[]);
    let response = client.call("reviews_get", json!({"id": "r1"}));
    assert!(!is_error(&response), "{response}");
    let env = envelope(&response);
    assert_eq!(
        (
            env["ok"].as_bool(),
            env["command"].as_str(),
            env["result"]["id"].as_str()
        ),
        (Some(true), Some("reviews.get"), Some("r1"))
    );
    let record: Value = serde_json::from_str(text(&response, 1).trim_end()).unwrap();
    assert_eq!(record["id"], "r1", "jsonl по умолчанию");
    assert_eq!(server.requests()[0].path, "/v4/reviews/r1");
    assert_eq!(client.finish(), 0);
}

#[test]
fn scroll_inline_fetches_exactly_one_page() {
    let home = TempDir::new("mcp-scroll");
    let page = first_page_json(
        &[
            review("r1", "2024-01-01T00:00:00Z"),
            review("r2", "2024-01-02T00:00:00Z"),
        ],
        Some("c1"),
        true,
        10,
        None,
    );
    let server = MockServer::start(vec![Reply::json(200, page)]);
    let mut client = served(home.path(), &server, &[]);
    let response = client.call(
        "reviews_scroll",
        json!({"filter": FILTER, "max_records": 2}),
    );
    assert!(!is_error(&response), "{response}");
    assert_eq!(text(&response, 1).lines().count(), 2);
    let env = envelope(&response);
    assert_eq!(
        (
            env["result"]["emitted"].as_u64(),
            env["result"]["completed"].as_bool()
        ),
        (Some(2), Some(false))
    );
    let requests = server.requests();
    assert_eq!(requests.len(), 1, "лимит набран — продолжения нет");
    assert_eq!(requests[0].query_param("per_page"), Some("2"));
    assert_eq!(client.finish(), 0);
}

#[test]
fn scroll_to_file_writes_records_and_reports_absolute_path() {
    let home = TempDir::new("mcp-file");
    let page = first_page_json(
        &[review("r1", "2024-01-01T00:00:00Z")],
        None,
        false,
        1,
        None,
    );
    let server = MockServer::start(vec![Reply::json(200, page)]);
    let mut client = served(home.path(), &server, &[]);
    let response = client.call(
        "reviews_scroll",
        json!({"filter": FILTER, "output_file": "out.jsonl"}),
    );
    assert!(!is_error(&response), "{response}");
    assert!(
        response["result"]["content"][1].is_null(),
        "данные — в файле, не в ответе"
    );
    let reported = envelope(&response)["result"]["output_file"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(Path::new(&reported).is_absolute(), "{reported}");
    assert_eq!(
        Path::new(&reported).canonicalize().unwrap(),
        home.path().join("out.jsonl").canonicalize().unwrap(),
        "относительный путь — от рабочего каталога сервера"
    );
    assert_eq!(
        std::fs::read_to_string(&reported).unwrap().lines().count(),
        1
    );
    assert_eq!(client.finish(), 0);
}

#[test]
fn existing_output_file_is_not_overwritten_without_consent() {
    let home = TempDir::new("mcp-exists");
    let target = home.path().join("out.jsonl");
    std::fs::write(&target, "старое\n").unwrap();
    let page = first_page_json(
        &[review("r1", "2024-01-01T00:00:00Z")],
        None,
        false,
        1,
        None,
    );
    let server = MockServer::start(vec![Reply::json(200, page)]);
    let mut client = served(home.path(), &server, &[]);
    let refused = client.call(
        "reviews_scroll",
        json!({"filter": FILTER, "output_file": "out.jsonl"}),
    );
    assert!(is_error(&refused), "{refused}");
    let err = &envelope(&refused)["error"];
    assert_eq!(
        (err["code"].as_str(), err["field"].as_str()),
        (Some("output_exists"), Some("output_file"))
    );
    assert_eq!(std::fs::read_to_string(&target).unwrap(), "старое\n");
    assert!(server.requests().is_empty(), "до сети");
    let replaced = client.call(
        "reviews_scroll",
        json!({"filter": FILTER, "output_file": "out.jsonl", "overwrite": true}),
    );
    assert!(!is_error(&replaced), "{replaced}");
    assert!(!std::fs::read_to_string(&target).unwrap().contains("старое"));
    assert_eq!(client.finish(), 0);
}

#[test]
fn state_appends_to_output_file() {
    let home = TempDir::new("mcp-append");
    let first = first_page_json(
        &[
            review("r1", "2024-01-01T00:00:00Z"),
            review("r2", "2024-01-02T00:00:00Z"),
        ],
        Some("c1"),
        true,
        3,
        None,
    );
    let second = page_json(&[review("r3", "2024-01-03T00:00:00Z")], None, false);
    let server = MockServer::start(vec![Reply::json(200, first), Reply::json(200, second)]);
    let mut client = served(home.path(), &server, &[]);
    let args =
        json!({"filter": FILTER, "state": "s.json", "output_file": "out.jsonl", "max_records": 2});
    let one = client.call("reviews_scroll", args.clone());
    assert!(!is_error(&one), "{one}");
    let two = client.call("reviews_scroll", args);
    assert!(!is_error(&two), "{two}");
    assert_eq!(envelope(&two)["result"]["completed"], true);
    let lines = std::fs::read_to_string(home.path().join("out.jsonl")).unwrap();
    assert_eq!(lines.lines().count(), 3, "{lines}");
    assert_eq!(client.finish(), 0);
}

#[test]
fn write_dry_run_sends_nothing() {
    let home = TempDir::new("mcp-dry");
    let server = MockServer::start(vec![]);
    let mut client = served(home.path(), &server, &["--allow-writes"]);
    let response = client.call(
        "reviews_create",
        json!({"rating": 5, "body": "Отлично", "dry_run": true}),
    );
    assert!(!is_error(&response), "{response}");
    let env = envelope(&response);
    assert_eq!(
        (env["command"].as_str(), env["dry_run"].as_bool()),
        (Some("reviews.create"), Some(true))
    );
    assert_eq!(
        env["result"]["request"]["body"]["data"]["attributes"]["body"],
        "Отлично"
    );
    assert!(server.requests().is_empty());
    assert_eq!(client.finish(), 0);
}

#[test]
fn create_sends_attributes_unchanged() {
    let home = TempDir::new("mcp-create");
    let server = MockServer::start(vec![created("r9")]);
    let mut client = served(home.path(), &server, &["--allow-writes"]);
    let body = "- Отлично, \"спасибо\" 👍";
    let response = client.call(
        "reviews_create",
        json!({"rating": 5, "body": body, "author_name": "Анна"}),
    );
    assert!(!is_error(&response), "{response}");
    assert_eq!(envelope(&response)["result"]["created"]["id"], "r9");
    let request = &server.requests()[0];
    assert_eq!(
        (request.method.as_str(), request.path.as_str()),
        ("POST", "/v4/reviews")
    );
    let attributes = &request.json()["data"]["attributes"];
    assert_eq!(
        (
            attributes["body"].as_str(),
            attributes["rating"].as_i64(),
            attributes["author_name"].as_str()
        ),
        (Some(body), Some(5), Some("Анна"))
    );
    assert_eq!(client.finish(), 0);
}

#[test]
fn write_tools_are_absent_without_allow_writes() {
    let home = TempDir::new("mcp-no-writes");
    let server = MockServer::start(vec![]);
    let mut client = served(home.path(), &server, &[]);
    let response = client.call("reviews_create", json!({"rating": 5, "body": "x"}));
    assert_eq!(response["error"]["code"], -32602, "{response}");
    assert!(server.requests().is_empty());
    assert_eq!(client.finish(), 0);
}

#[test]
fn missing_token_is_a_tool_error_with_hint() {
    let home = TempDir::new("mcp-no-token");
    let mut client = Client::start(home.path(), &[], &[]);
    let response = client.call("reviews_get", json!({"id": "r1"}));
    assert!(is_error(&response), "{response}");
    let err = &envelope(&response)["error"];
    assert_eq!(err["code"], "no_token");
    assert!(
        !err["hint"].as_str().unwrap_or_default().is_empty(),
        "{err}"
    );
    assert_eq!(client.finish(), 0);
}

#[test]
fn unknown_parameter_is_a_usage_error() {
    let home = TempDir::new("mcp-unknown");
    let server = MockServer::start(vec![]);
    let mut client = served(home.path(), &server, &[]);
    let response = client.call("reviews_scroll", json!({"filtr": FILTER}));
    assert!(is_error(&response), "{response}");
    let env = envelope(&response);
    assert_eq!(
        (
            env["error"]["code"].as_str(),
            env["error"]["field"].as_str(),
            env["command"].as_str()
        ),
        (Some("usage"), Some("filtr"), Some("reviews.scroll"))
    );
    assert!(server.requests().is_empty());
    assert_eq!(client.finish(), 0);
}

#[test]
fn child_validation_errors_pass_through() {
    let home = TempDir::new("mcp-child-error");
    let server = MockServer::start(vec![]);
    let mut client = served(home.path(), &server, &[]);
    let response = client.call(
        "reviews_scroll",
        json!({"filter": FILTER, "fields": ["id"]}),
    );
    assert!(is_error(&response), "{response}");
    assert_eq!(
        envelope(&response)["error"]["code"],
        "fields_need_tabular_format"
    );
    assert!(server.requests().is_empty());
    assert_eq!(client.finish(), 0);
}
