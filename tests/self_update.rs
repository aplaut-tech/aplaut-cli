//! `aplaut self update` против мок-GitHub (спека self-update §4): копия бинаря во временном `bin/`,
//! receipt — в `AXOUPDATER_CONFIG_PATH`, API — `APLAUT_CLI_INSTALLER_GHE_BASE_URL` (к нему
//! `axoupdater` добавляет `/api/v3`). Порядок запросов: `…/releases/latest`; при его ошибке `axoupdater`
//! молча идёт в `…/releases`; затем установщик по `browser_download_url`.

mod support;

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::json;
use support::{aplaut_exe, envelope, MockServer, Output, Reply, TempDir};

const CURRENT: &str = env!("CARGO_PKG_VERSION");
const LATEST_PATH: &str = "/api/v3/repos/aplaut-tech/aplaut-cli/releases/latest";
const LIST_PATH: &str = "/api/v3/repos/aplaut-tech/aplaut-cli/releases";

/// Установка «как от установщика»: копия бинаря в `<home>/bin` и receipt на этот каталог.
struct Install {
    home: TempDir,
}

impl Install {
    fn new(tag: &str) -> Install {
        let install = Install {
            home: TempDir::new(tag),
        };
        fs::create_dir_all(install.bin()).unwrap();
        fs::copy(env!("CARGO_BIN_EXE_aplaut"), install.bin().join("aplaut")).unwrap();
        install.write_receipt(&install.bin());
        install
    }

    /// Без receipt: `cargo install`, `APLAUT_CLI_UNMANAGED_INSTALL`, ручная распаковка.
    fn without_receipt(tag: &str) -> Install {
        let install = Install::new(tag);
        fs::remove_dir_all(install.receipt_dir()).unwrap();
        install
    }

    fn bin(&self) -> PathBuf {
        self.home.path().join("bin")
    }

    fn receipt_dir(&self) -> PathBuf {
        self.home.path().join("receipt")
    }

    fn marker(&self) -> PathBuf {
        self.bin().join("installed-marker")
    }

    /// Receipt того вида, что пишет установщик cargo-dist 0.33.
    fn write_receipt(&self, prefix: &Path) {
        let receipt = json!({
            "binaries": ["aplaut"],
            "binary_aliases": {},
            "cdylibs": [],
            "cstaticlibs": [],
            "install_layout": "flat",
            "install_prefix": prefix.to_str().unwrap(),
            "modify_path": false,
            "provider": {"source": "cargo-dist", "version": "0.33.0"},
            "source": {
                "app_name": "aplaut-cli",
                "name": "aplaut-cli",
                "owner": "aplaut-tech",
                "release_type": "github"
            },
            "version": CURRENT,
        });
        fs::create_dir_all(self.receipt_dir()).unwrap();
        fs::write(
            self.receipt_dir().join("aplaut-cli-receipt.json"),
            receipt.to_string(),
        )
        .unwrap();
    }

    fn run(&self, api: &MockServer, args: &[&str], extra_env: &[(&str, &str)]) -> Output {
        let mut full = vec!["self", "update"];
        full.extend_from_slice(args);
        let receipt_dir = self.receipt_dir();
        let origin = api.origin();
        let mut env = vec![
            ("AXOUPDATER_CONFIG_PATH", receipt_dir.to_str().unwrap()),
            ("APLAUT_CLI_INSTALLER_GHE_BASE_URL", origin.as_str()),
        ];
        env.extend_from_slice(extra_env);
        aplaut_exe(
            &self.bin().join("aplaut"),
            self.home.path(),
            &full,
            &env,
            "",
        )
    }
}

/// Поддельный установщик: строка в stdout (её не должно быть в stdout aplaut), строка в stderr,
/// маркер с каталогом установки и запретом правки PATH от `axoupdater`, заданный код выхода.
fn installer(exit_code: i32) -> Reply {
    Reply::text(
        200,
        format!(
            "#!/bin/sh\n\
             echo 'installer stdout noise'\n\
             echo 'installer stderr line' >&2\n\
             echo \"$APLAUT_CLI_INSTALL_DIR $APLAUT_CLI_NO_MODIFY_PATH\" > \"$APLAUT_CLI_INSTALL_DIR/installed-marker\"\n\
             exit {exit_code}\n"
        ),
    )
}

/// Ответ `GET …/releases/latest`: установщик лежит на отдельном мок-сервере — порт API-сервера до
/// его запуска неизвестен.
fn release(downloads: &MockServer, version: &str) -> Reply {
    Reply::json(
        200,
        json!({
            "tag_name": format!("v{version}"),
            "name": format!("v{version}"),
            "url": "https://api.github.com/repos/aplaut-tech/aplaut-cli/releases/1",
            "prerelease": false,
            "assets": [{
                "name": "aplaut-cli-installer.sh",
                "url": "https://api.github.com/repos/aplaut-tech/aplaut-cli/releases/assets/1",
                "browser_download_url": format!("{}/aplaut-cli-installer.sh", downloads.origin()),
            }],
        })
        .to_string(),
    )
}

fn paths(server: &MockServer) -> Vec<String> {
    server.requests().iter().map(|r| r.path.clone()).collect()
}

#[test]
fn newer_release_is_installed_by_its_installer_into_the_same_dir() {
    let install = Install::new("self-newer");
    let downloads = MockServer::start(vec![installer(0)]);
    let api = MockServer::start(vec![release(&downloads, "9.9.9")]);
    let out = install.run(&api, &["--json"], &[]);
    let v = envelope(&out);
    assert_eq!(v["command"], "self.update");
    assert_eq!(v["dry_run"], false);
    assert_eq!(
        v["result"],
        json!({"current": CURRENT, "latest": "9.9.9", "updated": true})
    );
    assert!(
        out.stderr.contains("installer stderr line"),
        "{}",
        out.stderr
    );
    let marker = fs::read_to_string(install.marker()).expect("установщик запущен");
    assert_eq!(marker.trim(), format!("{} 1", install.bin().display()));
    assert_eq!(paths(&api), [LATEST_PATH], "версия спрошена один раз");
    assert_eq!(downloads.requests().len(), 1);
    assert_eq!(api.requests()[0].header("authorization"), None);
}

#[test]
fn same_version_changes_nothing() {
    let install = Install::new("self-same");
    let downloads = MockServer::start(vec![installer(0)]);
    let api = MockServer::start(vec![release(&downloads, CURRENT)]);
    let v = envelope(&install.run(&api, &["--json"], &[]));
    assert_eq!(
        v["result"],
        json!({"current": CURRENT, "latest": CURRENT, "updated": false})
    );
    assert!(!install.marker().exists());
    assert_eq!(paths(&api), [LATEST_PATH]);
    assert!(downloads.requests().is_empty());
}

/// Review Focus 1: локальная сборка новее релиза — никакого даунгрейда.
#[test]
fn older_release_is_never_installed() {
    let install = Install::new("self-older");
    let downloads = MockServer::start(vec![installer(0)]);
    let api = MockServer::start(vec![release(&downloads, "0.0.0")]);
    let out = install.run(&api, &[], &[]);
    assert_eq!(out.code, 0, "{}", out.stderr);
    assert!(
        out.stderr
            .contains(&format!("aplaut {CURRENT} новее последнего релиза 0.0.0")),
        "{}",
        out.stderr
    );
    assert!(!install.marker().exists());
    assert!(downloads.requests().is_empty());
}

#[test]
fn dry_run_reports_the_newer_release_and_installs_nothing() {
    let install = Install::new("self-dry");
    let downloads = MockServer::start(vec![installer(0)]);
    let api = MockServer::start(vec![release(&downloads, "9.9.9")]);
    let v = envelope(&install.run(&api, &["--dry-run", "--json"], &[]));
    // Agent mode D2: под --dry-run result — будущий результат, `dry_run: true` — что его не было.
    assert_eq!(v["dry_run"], true);
    assert_eq!(
        v["result"],
        json!({"current": CURRENT, "latest": "9.9.9", "updated": true})
    );
    assert!(!install.marker().exists());
    assert!(downloads.requests().is_empty());
}

#[test]
fn text_mode_speaks_on_stderr_and_quiet_silences_it() {
    let install = Install::new("self-text");
    let downloads = MockServer::start(vec![installer(0)]);
    let api = MockServer::start(vec![release(&downloads, "9.9.9")]);
    let out = install.run(&api, &[], &[]);
    assert_eq!(out.code, 0, "{}", out.stderr);
    assert_eq!(out.stdout, "");
    assert!(
        out.stderr
            .contains(&format!("aplaut обновлён: {CURRENT} → 9.9.9")),
        "{}",
        out.stderr
    );

    let install = Install::new("self-dry-text");
    let api = MockServer::start(vec![release(&downloads, "9.9.9")]);
    let out = install.run(&api, &["-n"], &[]);
    assert_eq!(
        out.stderr,
        format!(
            "Пробный запуск, ничего не изменено: доступна версия 9.9.9 (установлена {CURRENT})\n"
        )
    );

    // Review Focus 5: -q — ни нашего текста, ни stdout.
    let install = Install::new("self-quiet");
    let api = MockServer::start(vec![release(&downloads, CURRENT)]);
    let out = install.run(&api, &["-q"], &[]);
    assert_eq!(out.code, 0, "{}", out.stderr);
    assert_eq!((out.stdout.as_str(), out.stderr.as_str()), ("", ""));
}

#[test]
fn install_without_receipt_is_refused_before_the_network() {
    let install = Install::without_receipt("self-no-receipt");
    let api = MockServer::start(vec![]);
    let out = install.run(&api, &["--json"], &[]);
    assert_eq!(out.code, 1, "{}", out.stderr);
    let err = out.error_json();
    assert_eq!(err["command"], "self.update");
    assert_eq!(err["error"]["code"], "update_unavailable");
    assert!(
        err["error"]["hint"].as_str().unwrap().contains("README"),
        "{err}"
    );
    assert!(api.requests().is_empty());
}

#[test]
fn receipt_of_another_install_is_refused_before_the_network() {
    let install = Install::new("self-foreign");
    install.write_receipt(&install.home.path().join("elsewhere").join("bin"));
    let api = MockServer::start(vec![]);
    let out = install.run(&api, &["--json"], &[]);
    assert_eq!(out.code, 1, "{}", out.stderr);
    let err = out.error_json();
    assert_eq!(err["error"]["code"], "update_unavailable");
    let message = err["error"]["message"].as_str().unwrap();
    assert!(message.contains("elsewhere"), "{message}");
    assert!(api.requests().is_empty());
}

#[test]
fn failing_installer_is_update_failed() {
    let install = Install::new("self-install-fail");
    let downloads = MockServer::start(vec![installer(1)]);
    let api = MockServer::start(vec![release(&downloads, "9.9.9")]);
    let out = install.run(&api, &["--json"], &[]);
    assert_eq!(out.code, 1, "{}", out.stderr);
    let err = out.error_json();
    assert_eq!(err["error"]["code"], "update_failed");
    assert!(
        err["error"]["message"]
            .as_str()
            .unwrap()
            .contains("кодом 1"),
        "{err}"
    );
}

#[test]
fn github_errors_map_to_the_cli_catalog() {
    for (status, code, exit, retryable) in [
        (404, "update_unavailable", 1, false),
        (401, "unauthorized", 3, false),
        (403, "rate_limited", 7, true),
        (502, "server_error", 1, true),
    ] {
        let install = Install::new(&format!("self-http-{status}"));
        let api = MockServer::start(vec![Reply::text(status, "{}"), Reply::text(status, "{}")]);
        let out = install.run(&api, &["--json"], &[]);
        assert_eq!(out.code, exit, "{status}: {}", out.stderr);
        let err = out.error_json();
        assert_eq!(err["error"]["code"], code, "{status}");
        assert_eq!(err["error"]["retryable"], retryable, "{status}");
        assert_eq!(paths(&api), [LATEST_PATH, LIST_PATH], "{status}");
    }
}

/// Review Focus 4: мусор вместо JSON — повтор не поможет.
#[test]
fn garbage_from_github_is_update_failed_not_retryable() {
    let install = Install::new("self-garbage");
    let api = MockServer::start(vec![
        Reply::json(200, "not json"),
        Reply::json(200, "not json"),
    ]);
    let out = install.run(&api, &["--json"], &[]);
    assert_eq!(out.code, 1, "{}", out.stderr);
    let err = out.error_json();
    assert_eq!(err["error"]["code"], "update_failed");
    assert_eq!(err["error"]["retryable"], false);
}

#[test]
fn silent_github_is_a_timeout() {
    let install = Install::new("self-timeout");
    let api = MockServer::start(vec![
        Reply::Stall(Duration::from_secs(2)),
        Reply::Stall(Duration::from_secs(2)),
    ]);
    let out = install.run(&api, &["--json", "--timeout", "1"], &[]);
    assert_eq!(out.code, 1, "{}", out.stderr);
    let err = out.error_json();
    assert_eq!(err["error"]["code"], "timeout");
    assert_eq!(err["error"]["retryable"], true);
}

/// Обрыв соединения — сеть, повтор имеет смысл. Ответов с запасом: клиент мог бы переспросить.
#[test]
fn dropped_connection_is_a_retryable_network_error() {
    let install = Install::new("self-hangup");
    let api = MockServer::start(vec![Reply::Hangup; 4]);
    let out = install.run(&api, &["--json"], &[]);
    assert_eq!(out.code, 1, "{}", out.stderr);
    let err = out.error_json();
    assert_eq!(err["error"]["code"], "network_error");
    assert_eq!(err["error"]["retryable"], true);
    assert!(
        err["error"]["hint"]
            .as_str()
            .unwrap()
            .contains("ca-certificates"),
        "{err}"
    );
}

/// Review Focus 2: API ответил, а скачивание установщика зависло — тот же --timeout.
#[test]
fn stalled_installer_download_is_a_timeout() {
    let install = Install::new("self-download-timeout");
    let downloads = MockServer::start(vec![Reply::Stall(Duration::from_secs(2))]);
    let api = MockServer::start(vec![release(&downloads, "9.9.9")]);
    let out = install.run(&api, &["--json", "--timeout", "1"], &[]);
    assert_eq!(out.code, 1, "{}", out.stderr);
    assert_eq!(out.error_json()["error"]["code"], "timeout");
    assert!(!install.marker().exists());
}

#[test]
fn github_token_goes_to_the_api_as_bearer() {
    let install = Install::new("self-token");
    let downloads = MockServer::start(vec![]);
    let api = MockServer::start(vec![release(&downloads, CURRENT)]);
    envelope(&install.run(
        &api,
        &["--json"],
        &[("APLAUT_CLI_GITHUB_TOKEN", "gh-test-token")],
    ));
    assert_eq!(
        api.requests()[0].header("authorization"),
        Some("Bearer gh-test-token")
    );
}

/// Review Focus 3: в CI не задан секрет — переменная пустая; `Bearer ` без токена дал бы 401.
#[test]
fn empty_github_token_sends_no_authorization() {
    let install = Install::new("self-empty-token");
    let downloads = MockServer::start(vec![]);
    let api = MockServer::start(vec![release(&downloads, CURRENT)]);
    envelope(&install.run(&api, &["--json"], &[("APLAUT_CLI_GITHUB_TOKEN", "")]));
    assert_eq!(api.requests()[0].header("authorization"), None);
}

/// Задача 1 плана: без системных корней сертификатов `reqwest` не собирает клиент, а
/// `AxoUpdater::new_for` на этом паникует — нужна ошибка с подсказкой, а не паника. Только Linux:
/// на macOS корни берутся из Keychain, `SSL_CERT_FILE`/`SSL_CERT_DIR` на них не влияют.
#[cfg(target_os = "linux")]
#[test]
fn missing_system_certificates_is_a_clear_error_not_a_panic() {
    let install = Install::new("self-no-ca");
    let api = MockServer::start(vec![]);
    let out = install.run(
        &api,
        &["--json"],
        &[
            ("SSL_CERT_FILE", "/nonexistent"),
            ("SSL_CERT_DIR", "/nonexistent"),
        ],
    );
    assert_eq!(out.code, 1, "{}", out.stderr);
    let err = out.error_json();
    assert_eq!(err["error"]["code"], "update_failed");
    assert!(
        err["error"]["hint"]
            .as_str()
            .unwrap_or_default()
            .contains("ca-certificates"),
        "{err}"
    );
    assert!(api.requests().is_empty());
}

/// Финальное ревью: таймаут во время чтения тела reqwest отдаёт как `decode(TimedOut)` — это
/// всё равно таймаут, а не «ответ не разобрался».
#[test]
fn installer_body_stalling_after_headers_is_a_timeout() {
    let install = Install::new("self-body-stall");
    let downloads = MockServer::start(vec![Reply::Truncated(Duration::from_secs(2))]);
    let api = MockServer::start(vec![release(&downloads, "9.9.9")]);
    let out = install.run(&api, &["--json", "--timeout", "1"], &[]);
    assert_eq!(out.code, 1, "{}", out.stderr);
    let err = out.error_json();
    assert_eq!(err["error"]["code"], "timeout", "{err}");
    assert_eq!(err["error"]["retryable"], true);
    assert!(!install.marker().exists());
}

/// Обрыв посреди тела — сеть, повтор имеет смысл; «мусор» — только если не разобрался JSON.
#[test]
fn connection_dropped_mid_body_is_a_retryable_network_error() {
    let install = Install::new("self-body-drop");
    let api = MockServer::start(vec![Reply::Truncated(Duration::ZERO); 2]);
    let out = install.run(&api, &["--json"], &[]);
    assert_eq!(out.code, 1, "{}", out.stderr);
    let err = out.error_json();
    assert_eq!(err["error"]["code"], "network_error", "{err}");
    assert_eq!(err["error"]["retryable"], true);
}
