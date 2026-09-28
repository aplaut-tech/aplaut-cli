//! Скачивание архива экспорта (спека writes-and-exports R9; §9). Ссылка хранилища публичная, поэтому
//! `Authorization` туда не уходит: у клиента скачивания токена нет вовсе. gzip распаковывается потоком,
//! файл появляется атомарно: временный файл рядом с `PATH` (имя — `fsutil::tmp_path`, с pid процесса,
//! `create_new`) → `fsync` файла → `rename` в `PATH` → `fsync` каталога; при сбое прежний `PATH` не
//! тронут, временный файл удаляется. Pid в имени, а не фиксированный `PATH.aplaut-tmp` (было раньше,
//! ruling I1 финальной ревизии, 2026-09-28): два параллельных запуска с одним `--output` (например,
//! перекрывающиеся вызовы из cron) иначе делили бы один inode — один процесс мог переименовать
//! недописанный файл другого, оставляя дыры из нулей или ещё дозаписываемые данные под именем `PATH`.

use std::fs::{self, OpenOptions};
use std::io::{self, BufWriter, ErrorKind, Read, Write};
use std::path::Path;
use std::time::Duration;

use flate2::read::MultiGzDecoder;

use crate::auth::url_is_loopback;
use crate::error::CliError;
use crate::fsutil;

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
    let tmp = fsutil::tmp_path(dest);
    // Остаток от упавшего процесса с тем же pid (маловероятно, но безобиднее убрать) мешал бы
    // create_new; чужой параллельный процесс — с другим pid, у него другое имя.
    let _ = fs::remove_file(&tmp);
    let result = (|| {
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&tmp)
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
        fsutil::sync_parent_dir(dest)
            .map_err(|e| CliError::io(&format!("fsync каталога для {}", dest.display()), &e))?;
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

/// Ruling M4 финальной ревизии (2026-09-28): обрезанный, пустой или с мусором в хвосте gzip
/// заканчивается у `flate2` `UnexpectedEof` — тем же `ErrorKind`, которым обрывается и настоящий разрыв
/// соединения раньше заявленного `Content-Length`. Различаем по источнику (`is_from_ureq`): настоящий
/// обрыв — от `ureq`, битый архив — ошибка, которую сам `flate2` строит из нехватки байт в уже
/// полностью полученном теле. Только `UnexpectedEof` не от `ureq` и только при `gzip: true` (иначе
/// `xlsx` без распаковки никогда не читает поток настолько, чтобы поймать такую ошибку) — это битый
/// архив, не сеть.
fn read_error(host: &str, gzip: bool, err: &io::Error) -> CliError {
    match err.kind() {
        ErrorKind::InvalidData | ErrorKind::InvalidInput if gzip => CliError::general(
            "bad_response",
            format!("архив экспорта с {host} повреждён: {err}"),
        ),
        ErrorKind::UnexpectedEof if gzip && !is_from_ureq(err) => CliError::general(
            "bad_response",
            format!("архив экспорта с {host} повреждён (обрезан): {err}"),
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

fn ureq_error(err: &io::Error) -> Option<&ureq::Error> {
    err.get_ref().and_then(|e| e.downcast_ref::<ureq::Error>())
}

/// Обрыв тела раньше заявленного `Content-Length` — тоже от `ureq`, но НЕ виден через downcast:
/// `ureq::Error::disconnected` (ureq 3.4.2, `src/error.rs`) заворачивает его как `Error::Io(io::Error)`,
/// а `Error::into_io()` для варианта `Io` отдаёт этот `io::Error` как есть, без обёртки `ureq::Error`
/// (в отличие от `Error::Timeout`, который `into_io()` заворачивает в `io::Error::other(self)` — см.
/// `is_body_timeout`). Единственный устойчивый признак — фиксированный текст ureq для этого случая,
/// "Peer disconnected"; проверено на стенде теста (`MockServer::Truncated`): битый gzip (`flate2`)
/// говорит другими словами ("incomplete deflate stream" и т.п.), с сетью не пересекается.
fn is_from_ureq(err: &io::Error) -> bool {
    ureq_error(err).is_some() || err.to_string().contains("Peer disconnected")
}

/// ureq 3.4 заворачивает обрыв тела по `--wait-timeout` не в `ErrorKind::TimedOut`, а в
/// `io::Error::other(ureq::Error::Timeout(_))` (`Error::into_io`, у чтения тела всегда так —
/// `Error::Io` тут нет): код `ErrorKind` при этом `Other`. Распаковываем исходную ошибку явно;
/// `ErrorKind::TimedOut` оставлен как запасной вариант на случай других источников чтения.
fn is_body_timeout(err: &io::Error) -> bool {
    err.kind() == ErrorKind::TimedOut
        || ureq_error(err).is_some_and(|e| matches!(e, ureq::Error::Timeout(_)))
}

fn transport(host: &str, err: &ureq::Error) -> CliError {
    let code = match err {
        ureq::Error::Timeout(_) => "timeout",
        _ => "network_error",
    };
    CliError::general(code, format!("архив экспорта не скачан с {host}: {err}")).retryable(true)
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
