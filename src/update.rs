//! Самообновление (спека self-update): перезапуск установщика cargo-dist из последнего релиза через
//! `axoupdater`. Единственный модуль, где видны `axoupdater`, `reqwest` и tokio (исключение из D11, U3).

use std::time::Duration;

use axoupdater::{AxoUpdater, AxoupdateError, Version};
use serde::Serialize;

use crate::error::{CliError, Exit};

/// Имя приложения в receipt и в переменных установщика (`APLAUT_CLI_*`).
pub const APP_NAME: &str = "aplaut-cli";

const REINSTALL_HINT: &str =
    "обновите тем же способом, которым ставили, или переустановите командой из README";
const RELEASES_HINT: &str = "релизы: https://github.com/aplaut-tech/aplaut-cli/releases";
/// Проверка сертификатов здесь — по системному хранилищу, в отличие от остальных команд (U3).
const NETWORK_HINT: &str = "в минимальном контейнере для self update нужен пакет ca-certificates";

pub struct Options<'a> {
    pub dry_run: bool,
    pub timeout: Duration,
    pub github_token: Option<&'a str>,
}

/// `result` команды (спека §2): три ключа, есть всегда.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SelfUpdate {
    pub current: String,
    pub latest: String,
    pub updated: bool,
    /// Последний релиз строго новее текущей версии — для текста людям, не для JSON.
    #[serde(skip)]
    pub update_available: bool,
}

pub fn self_update(opts: &Options) -> Result<SelfUpdate, CliError> {
    let current: Version = env!("CARGO_PKG_VERSION")
        .parse()
        .expect("версия пакета — semver");
    let mut updater = AxoUpdater::new_for(APP_NAME);
    updater.load_receipt().map_err(map_error)?;
    ensure_receipt_is_for_this_binary(&updater)?;
    // Версия из receipt отстаёт после ручной замены бинаря (U6).
    updater
        .set_current_version(current.clone())
        .map_err(map_error)?;
    updater
        .set_client(http_client(opts.timeout)?)
        .disable_installer_stdout()
        .enable_installer_stderr();
    if let Some(token) = opts.github_token {
        updater.set_github_token(token);
    }
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| CliError::io("запуск рантайма для self update", &e))?;
    let latest = runtime
        .block_on(updater.query_new_version())
        .map_err(map_error)?
        .cloned()
        .ok_or_else(no_release)?;
    let update_available = latest > current;
    let result = SelfUpdate {
        current: current.to_string(),
        latest: latest.to_string(),
        updated: false,
        update_available,
    };
    if opts.dry_run || !update_available {
        return Ok(result);
    }
    // Релиз уже получен: без always_update `run()` снова спросил бы GitHub (U11).
    updater.always_update(true);
    runtime.block_on(updater.run()).map_err(map_error)?;
    Ok(SelfUpdate {
        updated: true,
        ..result
    })
}

/// `axoupdater` при чужом receipt молча отвечает «обновление не нужно» (U12) — проверяем сами, до сети.
fn ensure_receipt_is_for_this_binary(updater: &AxoUpdater) -> Result<(), CliError> {
    if updater
        .check_receipt_is_for_this_executable()
        .map_err(map_error)?
    {
        return Ok(());
    }
    let exe = std::env::current_exe()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| "?".into());
    let prefix = updater
        .install_prefix_root()
        .map(|p| p.to_string())
        .unwrap_or_else(|_| "?".into());
    Err(CliError::general(
        "update_unavailable",
        format!(
            "этот aplaut ({exe}) поставлен не тем установщиком, чей файл установки найден: тот ставил в {prefix}"
        ),
    )
    .with_hint(REINSTALL_HINT))
}

/// Клиент `axoupdater` по умолчанию без таймаута: зависшее соединение повесило бы агента (U10).
pub fn http_client(timeout: Duration) -> Result<reqwest::Client, CliError> {
    reqwest::Client::builder()
        .timeout(timeout)
        .build()
        .map_err(|e| CliError::general("update_failed", format!("HTTP-клиент не собрался: {e}")))
}

/// Ошибки `axoupdater` — в каталог CLI (спека §2).
pub fn map_error(err: AxoupdateError) -> CliError {
    match err {
        AxoupdateError::NoReceipt { .. } => CliError::general(
            "update_unavailable",
            "aplaut поставлен не установщиком из README: файла установки (receipt) нет",
        )
        .with_hint(REINSTALL_HINT),
        AxoupdateError::ReceiptLoadFailed { .. } => CliError::general(
            "update_unavailable",
            "файл установки aplaut (receipt) не читается",
        )
        .with_hint("переустановите aplaut командой из README"),
        AxoupdateError::NoStableReleases { .. } | AxoupdateError::ReleaseNotFound { .. } => {
            no_release()
        }
        AxoupdateError::Reqwest(err) => map_http(&err),
        AxoupdateError::InstallFailed { status, .. } => CliError::general(
            "update_failed",
            match status {
                Some(code) => format!("установщик новой версии завершился с кодом {code}"),
                None => "установщик новой версии завершился с ошибкой".to_string(),
            },
        )
        .with_hint("подробности — выше, в выводе установщика"),
        other => CliError::general("update_failed", format!("обновление не удалось: {other}")),
    }
}

fn no_release() -> CliError {
    CliError::general(
        "update_unavailable",
        "на GitHub нет релиза aplaut с установщиком",
    )
    .with_hint(RELEASES_HINT)
}

/// Ошибка HTTP от `axoupdater` приходит без заголовков: 403 не отличить от прочих — у GitHub API без
/// токена это лимит (спека §2).
fn map_http(err: &reqwest::Error) -> CliError {
    if err.is_decode() {
        return CliError::general(
            "update_failed",
            format!("ответ GitHub не разобрался: {err}"),
        );
    }
    match err.status().map(|s| s.as_u16()) {
        Some(404) => no_release(),
        Some(401) => CliError::new(
            Exit::Auth,
            "unauthorized",
            "GitHub отклонил APLAUT_CLI_GITHUB_TOKEN",
        )
        .with_hint("проверьте токен или уберите переменную: для публичного репо он не нужен"),
        Some(403 | 429) => CliError::new(
            Exit::RateLimited,
            "rate_limited",
            "GitHub API: лимит запросов исчерпан",
        )
        .retryable(true)
        .with_hint("задайте APLAUT_CLI_GITHUB_TOKEN — лимит станет выше"),
        Some(status) => CliError::general("update_failed", format!("GitHub API ответил {status}")),
        None if err.is_timeout() => {
            CliError::general("timeout", "GitHub не ответил за --timeout").retryable(true)
        }
        None => CliError::general("network_error", format!("нет связи с GitHub: {err}"))
            .retryable(true)
            .with_hint(NETWORK_HINT),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `ureq` тянет в rustls провайдер ring, `reqwest` — aws-lc-rs: сборка TLS-конфига не должна
    /// паниковать из-за двух провайдеров (спека §6, риск 2). `new_for` тоже собирает клиент.
    #[test]
    fn http_client_builds_with_both_rustls_providers_linked() {
        assert!(http_client(Duration::from_secs(1)).is_ok());
        let _ = axoupdater::AxoUpdater::new_for(APP_NAME);
    }

    fn codes(err: CliError) -> (String, Exit, bool) {
        (err.code, err.exit, err.retryable)
    }

    #[test]
    fn installs_not_made_by_the_installer_are_unavailable_with_a_reinstall_hint() {
        for err in [
            AxoupdateError::NoReceipt {
                app_name: APP_NAME.into(),
            },
            AxoupdateError::ReceiptLoadFailed {
                app_name: APP_NAME.into(),
            },
        ] {
            let mapped = map_error(err);
            assert!(
                mapped.hint.as_deref().unwrap().contains("README"),
                "{mapped:?}"
            );
            assert_eq!(
                codes(mapped),
                ("update_unavailable".into(), Exit::General, false)
            );
        }
    }

    #[test]
    fn missing_release_points_to_the_releases_page() {
        for err in [
            AxoupdateError::NoStableReleases {
                app_name: APP_NAME.into(),
            },
            AxoupdateError::ReleaseNotFound {
                name: APP_NAME.into(),
                app_name: APP_NAME.into(),
            },
        ] {
            let mapped = map_error(err);
            assert_eq!(
                mapped.hint.as_deref(),
                Some("релизы: https://github.com/aplaut-tech/aplaut-cli/releases")
            );
            assert_eq!(
                codes(mapped),
                ("update_unavailable".into(), Exit::General, false)
            );
        }
    }

    #[test]
    fn failed_installer_reports_its_exit_code() {
        let mapped = map_error(AxoupdateError::InstallFailed {
            status: Some(1),
            stdout: None,
            stderr: None,
        });
        assert!(mapped.message.contains("кодом 1"), "{}", mapped.message);
        assert_eq!(
            codes(mapped),
            ("update_failed".into(), Exit::General, false)
        );
    }

    #[test]
    fn other_axoupdater_errors_are_update_failed_with_their_text() {
        let mapped = map_error(AxoupdateError::NoInstallerForPackage {});
        assert!(
            mapped.message.starts_with("обновление не удалось: "),
            "{}",
            mapped.message
        );
        assert_eq!(
            codes(mapped),
            ("update_failed".into(), Exit::General, false)
        );
    }

    #[test]
    fn result_has_exactly_the_three_documented_keys() {
        let result = SelfUpdate {
            current: "0.2.0".into(),
            latest: "0.3.0".into(),
            updated: true,
            update_available: true,
        };
        assert_eq!(
            serde_json::to_value(&result).unwrap(),
            serde_json::json!({"current": "0.2.0", "latest": "0.3.0", "updated": true})
        );
    }
}
