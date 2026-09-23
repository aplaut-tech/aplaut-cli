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
