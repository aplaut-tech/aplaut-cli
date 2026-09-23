//! Общие помощники интеграционных тестов. Подключаются через `mod support;`.
#![allow(dead_code)]

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT_DIR: AtomicUsize = AtomicUsize::new(0);

/// Временный каталог без крейта `tempfile`: удаляется при drop.
pub struct TempDir(PathBuf);

impl TempDir {
    pub fn new(tag: &str) -> TempDir {
        let path = std::env::temp_dir().join(format!(
            "aplaut-test-{tag}-{}-{}",
            std::process::id(),
            NEXT_DIR.fetch_add(1, Ordering::SeqCst)
        ));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).unwrap();
        TempDir(path)
    }

    pub fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

pub struct Output {
    pub code: i32,
    pub stdout: String,
    pub stderr: String,
}

impl Output {
    /// Последняя строка stderr — JSON-конверт ошибки (без TTY).
    pub fn error_json(&self) -> serde_json::Value {
        let line = self.stderr.lines().last().unwrap_or_default();
        serde_json::from_str(line)
            .unwrap_or_else(|e| panic!("последняя строка stderr не JSON ({e}): {}", self.stderr))
    }
}

/// Запускает бинарь с чистым окружением: HOME и XDG_CONFIG_HOME указывают во временный каталог,
/// stdin/stdout/stderr — пайпы (то есть не TTY).
pub fn aplaut(home: &Path, args: &[&str], env: &[(&str, &str)], stdin: &str) -> Output {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_aplaut"));
    cmd.args(args)
        .env_clear()
        .env("HOME", home)
        .env("XDG_CONFIG_HOME", home.join("config"))
        .env("PATH", std::env::var("PATH").unwrap_or_default());
    for (k, v) in env {
        cmd.env(k, v);
    }
    cmd.stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = cmd.spawn().expect("запуск aplaut");
    if let Some(mut input) = child.stdin.take() {
        let _ = input.write_all(stdin.as_bytes());
    }
    let out = child.wait_with_output().expect("ожидание aplaut");
    Output {
        code: out.status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
    }
}

use std::io::{BufRead, BufReader};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

#[derive(Debug, Clone)]
pub struct RecordedRequest {
    pub method: String,
    pub path: String,
    pub query: Vec<(String, String)>,
    pub headers: Vec<(String, String)>,
}

impl RecordedRequest {
    pub fn header(&self, name: &str) -> Option<&str> {
        let name = name.to_ascii_lowercase();
        self.headers
            .iter()
            .find(|(k, _)| *k == name)
            .map(|(_, v)| v.as_str())
    }

    pub fn query_param(&self, name: &str) -> Option<&str> {
        self.query
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
    }

    pub fn query_keys(&self) -> Vec<&str> {
        self.query.iter().map(|(k, _)| k.as_str()).collect()
    }
}

#[derive(Debug, Clone)]
pub enum Reply {
    Http {
        status: u16,
        headers: Vec<(String, String)>,
        body: Vec<u8>,
    },
    /// Закрыть соединение, не ответив: так выглядит обрыв сети для клиента.
    Hangup,
}

impl Reply {
    pub fn json(status: u16, body: impl Into<String>) -> Reply {
        Reply::Http {
            status,
            headers: vec![(
                "Content-Type".into(),
                "application/vnd.api+json; charset=utf-8".into(),
            )],
            body: body.into().into_bytes(),
        }
    }

    pub fn text(status: u16, body: impl Into<String>) -> Reply {
        Reply::Http {
            status,
            headers: vec![],
            body: body.into().into_bytes(),
        }
    }

    pub fn with_header(self, key: &str, value: &str) -> Reply {
        match self {
            Reply::Http {
                status,
                mut headers,
                body,
            } => {
                headers.push((key.into(), value.into()));
                Reply::Http {
                    status,
                    headers,
                    body,
                }
            }
            Reply::Hangup => Reply::Hangup,
        }
    }
}

/// Однопоточный HTTP/1.1-сервер со сценарием ответов: без внешних крейтов и без tokio.
pub struct MockServer {
    addr: SocketAddr,
    requests: Arc<Mutex<Vec<RecordedRequest>>>,
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl MockServer {
    pub fn start(replies: Vec<Reply>) -> MockServer {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let (log, stop_flag) = (requests.clone(), stop.clone());
        let handle = std::thread::spawn(move || {
            let mut replies = replies.into_iter();
            for stream in listener.incoming() {
                if stop_flag.load(Ordering::SeqCst) {
                    break;
                }
                let Ok(mut stream) = stream else { continue };
                let Some(request) = read_request(&mut stream) else {
                    continue;
                };
                log.lock().unwrap().push(request);
                match replies.next() {
                    Some(Reply::Http {
                        status,
                        headers,
                        body,
                    }) => write_response(&mut stream, status, &headers, &body),
                    Some(Reply::Hangup) => {}
                    None => write_response(&mut stream, 599, &[], b"mock: no more replies"),
                }
            }
        });
        MockServer {
            addr,
            requests,
            stop,
            handle: Some(handle),
        }
    }

    pub fn base_url(&self) -> String {
        format!("http://{}/v4", self.addr)
    }

    /// `http://127.0.0.1:PORT` — например, чтобы выдать сервер за прокси.
    pub fn origin(&self) -> String {
        format!("http://{}", self.addr)
    }

    pub fn requests(&self) -> Vec<RecordedRequest> {
        self.requests.lock().unwrap().clone()
    }
}

impl Drop for MockServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        let _ = TcpStream::connect(self.addr);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

fn read_request(stream: &mut TcpStream) -> Option<RecordedRequest> {
    stream.set_read_timeout(Some(Duration::from_secs(5))).ok();
    let mut reader = BufReader::new(stream.try_clone().ok()?);
    let mut line = String::new();
    reader.read_line(&mut line).ok()?;
    let mut parts = line.split_whitespace();
    let method = parts.next()?.to_string();
    let target = parts.next()?.to_string();
    let mut headers = Vec::new();
    loop {
        let mut header = String::new();
        if reader.read_line(&mut header).ok()? == 0 {
            break;
        }
        let header = header.trim_end();
        if header.is_empty() {
            break;
        }
        if let Some((k, v)) = header.split_once(':') {
            headers.push((k.trim().to_ascii_lowercase(), v.trim().to_string()));
        }
    }
    let (path, query) = match target.split_once('?') {
        Some((p, q)) => (p.to_string(), parse_query(q)),
        None => (target, Vec::new()),
    };
    Some(RecordedRequest {
        method,
        path,
        query,
        headers,
    })
}

fn write_response(stream: &mut TcpStream, status: u16, headers: &[(String, String)], body: &[u8]) {
    let mut head = format!(
        "HTTP/1.1 {status} {}\r\nContent-Length: {}\r\nConnection: close\r\n",
        reason(status),
        body.len()
    );
    for (k, v) in headers {
        head.push_str(&format!("{k}: {v}\r\n"));
    }
    head.push_str("\r\n");
    let _ = stream.write_all(head.as_bytes());
    let _ = stream.write_all(body);
    let _ = stream.flush();
}

fn reason(status: u16) -> &'static str {
    match status {
        200 => "OK",
        302 => "Found",
        400 => "Bad Request",
        401 => "Unauthorized",
        404 => "Not Found",
        422 => "Unprocessable Entity",
        429 => "Too Many Requests",
        500 => "Internal Server Error",
        502 => "Bad Gateway",
        503 => "Service Unavailable",
        _ => "Status",
    }
}

fn parse_query(query: &str) -> Vec<(String, String)> {
    query
        .split('&')
        .filter(|s| !s.is_empty())
        .map(|pair| {
            let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
            (percent_decode(k), percent_decode(v))
        })
        .collect()
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 3 <= bytes.len() {
            if let Ok(b) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                out.push(b);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Отзыв в форме, которую отдаёт `/scroll/reviews` (связи — JSON:API `{data: …}`).
pub fn review(id: &str, updated_at: &str) -> serde_json::Value {
    serde_json::json!({
        "id": id,
        "type": "reviews",
        "attributes": {"rating": 5.0, "body": format!("Отзыв {id}"), "updated_at": updated_at},
        "relationships": {
            "author": {"data": null},
            "product": {"data": {"id": format!("p-{id}"), "type": "products"}}
        }
    })
}

pub fn page_json(records: &[serde_json::Value], cursor: Option<&str>, has_more: bool) -> String {
    let mut meta = serde_json::json!({"has_more": has_more, "count": records.len()});
    if let Some(c) = cursor {
        meta["cursor"] = c.into();
    }
    serde_json::json!({"data": records, "meta": meta}).to_string()
}

pub fn first_page_json(
    records: &[serde_json::Value],
    cursor: Option<&str>,
    has_more: bool,
    total: u64,
    applied_filter: Option<&str>,
) -> String {
    let mut page: serde_json::Value =
        serde_json::from_str(&page_json(records, cursor, has_more)).unwrap();
    page["meta"]["total_count"] = total.into();
    if let Some(f) = applied_filter {
        page["meta"]["applied_filter"] = f.into();
    }
    page.to_string()
}

/// Как `aplaut`, но без ожидания: тест сам читает stdout (например, закрывает его раньше времени).
pub fn spawn(home: &Path, args: &[&str], env: &[(&str, &str)]) -> std::process::Child {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_aplaut"));
    cmd.args(args)
        .env_clear()
        .env("HOME", home)
        .env("XDG_CONFIG_HOME", home.join("config"))
        .env("PATH", std::env::var("PATH").unwrap_or_default());
    for (k, v) in env {
        cmd.env(k, v);
    }
    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    cmd.spawn().expect("запуск aplaut")
}
