//! Скачивание архива экспорта (спека writes-and-exports R9; §9). Ссылка хранилища публичная, поэтому
//! `Authorization` туда не уходит: у клиента скачивания токена нет вовсе. gzip распаковывается потоком,
//! файл появляется атомарно: `PATH.aplaut-tmp` → `fsync` → `rename`; при сбое прежний `PATH` не тронут.

use std::ffi::OsString;
use std::fs::{self, File};
use std::io::{self, BufWriter, ErrorKind, Read, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use flate2::read::MultiGzDecoder;

use crate::auth::url_is_loopback;
use crate::error::CliError;

/// Хранилище может перенаправлять (http → https); больше — признак ошибки.
const MAX_REDIRECTS: u32 = 5;
const BUFFER: usize = 64 * 1024;

#[derive(Debug, Clone, Copy)]
pub struct Timeouts {
    /// Соединение и заголовки ответа — `--timeout`.
    pub response: Duration,
    /// Всё тело — `--wait-timeout`: архивы бывают по сотне мегабайт.
    pub body: Duration,
}

pub fn download(url: &str, gzip: bool, dest: &Path, timeouts: Timeouts) -> Result<u64, CliError> {
    let host = host(url);
    let mut config = ureq::Agent::config_builder()
        .http_status_as_error(false)
        .max_redirects(MAX_REDIRECTS)
        .timeout_connect(Some(timeouts.response))
        .timeout_recv_response(Some(timeouts.response))
        .timeout_recv_body(Some(timeouts.body))
        .user_agent(format!("aplaut-cli/{}", env!("CARGO_PKG_VERSION")));
    if url_is_loopback(url) {
        config = config.proxy(None);
    }
    let agent: ureq::Agent = config.build().into();
    let mut response = agent.get(url).call().map_err(|e| transport(&host, &e))?;
    let status = response.status().as_u16();
    if !(200..300).contains(&status) {
        return Err(CliError::general(
            format!("http_{status}"),
            format!("архив экспорта не скачан с {host}: ответ {status}"),
        )
        .retryable(status >= 500));
    }
    let body = response.body_mut().as_reader();
    let reader: Box<dyn Read> = if gzip {
        // `MultiGzDecoder`, а не `GzDecoder`: несколько gzip-членов подряд в одном архиве
        // `GzDecoder` молча обрежет на первом, `MultiGzDecoder` распакует все.
        Box::new(MultiGzDecoder::new(body))
    } else {
        Box::new(body)
    };
    write_atomically(reader, dest, &host, gzip)
}

fn write_atomically(
    reader: Box<dyn Read + '_>,
    dest: &Path,
    host: &str,
    gzip: bool,
) -> Result<u64, CliError> {
    let tmp = tmp_path(dest);
    let result = (|| {
        let file = File::create(&tmp)
            .map_err(|e| CliError::io(&format!("создание {}", tmp.display()), &e))?;
        let mut out = BufWriter::new(file);
        let written = copy(reader, &mut out, host, gzip, &tmp)?;
        let file = out
            .into_inner()
            .map_err(|e| CliError::io(&format!("запись {}", tmp.display()), e.error()))?;
        file.sync_all()
            .map_err(|e| CliError::io(&format!("запись {}", tmp.display()), &e))?;
        fs::rename(&tmp, dest)
            .map_err(|e| CliError::io(&format!("переименование в {}", dest.display()), &e))?;
        Ok(written)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result
}

/// Ошибки чтения и записи различаются: обрыв сети повторим, битый gzip — нет, диск — `io_error`.
fn copy(
    mut reader: Box<dyn Read + '_>,
    out: &mut impl Write,
    host: &str,
    gzip: bool,
    tmp: &Path,
) -> Result<u64, CliError> {
    let mut buffer = vec![0u8; BUFFER];
    let mut written = 0u64;
    loop {
        let n = match reader.read(&mut buffer) {
            Ok(0) => return Ok(written),
            Ok(n) => n,
            Err(e) if e.kind() == ErrorKind::Interrupted => continue,
            Err(e) => return Err(read_error(host, gzip, &e)),
        };
        out.write_all(&buffer[..n])
            .map_err(|e| CliError::io(&format!("запись {}", tmp.display()), &e))?;
        written += n as u64;
    }
}

fn read_error(host: &str, gzip: bool, err: &io::Error) -> CliError {
    match err.kind() {
        ErrorKind::InvalidData | ErrorKind::InvalidInput if gzip => CliError::general(
            "bad_response",
            format!("архив экспорта с {host} повреждён: {err}"),
        ),
        _ if is_body_timeout(err) => CliError::general(
            "timeout",
            format!("архив экспорта с {host} не докачан за отведённое время: {err}"),
        )
        .retryable(true),
        _ => CliError::general(
            "network_error",
            format!("скачивание архива с {host} оборвалось: {err}"),
        )
        .retryable(true),
    }
}

/// ureq 3.4 заворачивает обрыв тела по `--wait-timeout` не в `ErrorKind::TimedOut`, а в
/// `io::Error::other(ureq::Error::Timeout(_))` (`Error::into_io`, у чтения тела всегда так —
/// `Error::Io` тут нет): код `ErrorKind` при этом `Other`. Распаковываем исходную ошибку явно;
/// `ErrorKind::TimedOut` оставлен как запасной вариант на случай других источников чтения.
fn is_body_timeout(err: &io::Error) -> bool {
    err.kind() == ErrorKind::TimedOut
        || err
            .get_ref()
            .and_then(|e| e.downcast_ref::<ureq::Error>())
            .is_some_and(|e| matches!(e, ureq::Error::Timeout(_)))
}

fn transport(host: &str, err: &ureq::Error) -> CliError {
    let code = match err {
        ureq::Error::Timeout(_) => "timeout",
        _ => "network_error",
    };
    CliError::general(code, format!("архив экспорта не скачан с {host}: {err}")).retryable(true)
}

fn tmp_path(dest: &Path) -> PathBuf {
    let mut name = OsString::from(dest.as_os_str());
    name.push(".aplaut-tmp");
    PathBuf::from(name)
}

/// Хост для сообщений: путь и query ссылки не печатаем — ссылка сама по себе даёт доступ к данным.
fn host(url: &str) -> String {
    url.split_once("://")
        .map_or(url, |(_, rest)| rest)
        .split(['/', '?', '#'])
        .next()
        .unwrap_or("")
        .to_string()
}
