//! MCP-сервер `aplaut mcp` (спека mcp-server): протокол, инструменты и вызовы — через настоящий
//! бинарь, JSON-RPC строками через stdin/stdout.

mod support;

use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

use serde_json::{json, Value};
use support::{aplaut, TempDir};

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
