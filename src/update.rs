//! Самообновление (спека self-update): перезапуск установщика cargo-dist из последнего релиза через
//! `axoupdater`. Единственный модуль, где видны `axoupdater`, `reqwest` и tokio (исключение из D11, U3).

use std::time::Duration;

use crate::error::CliError;

/// Имя приложения в receipt и в переменных установщика (`APLAUT_CLI_*`).
pub const APP_NAME: &str = "aplaut-cli";

/// Клиент `axoupdater` по умолчанию без таймаута: зависшее соединение повесило бы агента (U10).
pub fn http_client(timeout: Duration) -> Result<reqwest::Client, CliError> {
    reqwest::Client::builder()
        .timeout(timeout)
        .build()
        .map_err(|e| CliError::general("update_failed", format!("HTTP-клиент не собрался: {e}")))
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
}
