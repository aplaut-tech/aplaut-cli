//! HTTP-слой: заголовки, троттлинг, ретраи. Один клиент на процесс.
//!
//! Лимиты API жёсткие: 2 запроса/с на IP, а для scroll ещё 1 запрос / 2 с и 5 открытий в
//! минуту на ключ. Троттлинг проактивный: 429 — исключение, а не способ узнать лимит.

use std::rc::Rc;
use std::time::Duration;

use crate::api_error::{self, ErrorContext, ResponseHeaders};
use crate::clock::Clock;
use crate::error::CliError;
use crate::secret::{Secret, MASK};
use crate::term::Reporter;
use crate::time;

const ACCEPT: &str = "application/vnd.api+json";
/// Страница scroll на 100 записей — сотни килобайт; 64 МБ — защита от бесконечного тела.
const MAX_BODY_BYTES: u64 = 64 * 1024 * 1024;
/// 2 запроса в секунду с IP.
const MIN_INTERVAL: Duration = Duration::from_millis(500);
/// Scroll: 1 запрос в 2 с, окна фиксированные; 10% запаса на сетевой джиттер.
const SCROLL_INTERVAL: Duration = Duration::from_millis(2200);
const BACKOFF_BASE: Duration = Duration::from_secs(1);
const BACKOFF_CAP: Duration = Duration::from_secs(60);
const BACKOFF_FLOOR: Duration = Duration::from_millis(100);
/// Заголовки rate limit точны до секунды — добавляем джиттер сверху.
const RATE_LIMIT_JITTER_MS: u64 = 250;
/// Дольше не ждём молча: лучше явная ошибка с временем сброса, чем «зависший» cron.
const MAX_RATE_LIMIT_WAIT: Duration = Duration::from_secs(300);

/// Код ошибки, когда неизвестно, обработал ли сервер запрос (обрыв после отправки, таймаут, 5xx).
pub const OUTCOME_UNKNOWN: &str = "request_outcome_unknown";

/// Можно ли повторить запрос. Курсор scroll не идемпотентен (стейджинг, 2026-09-23): повтор
/// тем же курсором отдаёт *следующую* страницу, поэтому продолжение обхода повторяется только
/// тогда, когда сервер его точно не обработал — иначе страница молча пропадёт.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Replay {
    Safe,
    OnlyIfUnprocessed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pace {
    Default,
    Scroll,
}

#[derive(Debug, Clone)]
pub struct HttpSettings {
    pub base_url: String,
    pub timeout: Duration,
    pub max_retries: u32,
}

#[derive(Debug, Clone)]
pub struct ApiResponse {
    pub status: u16,
    pub body: Vec<u8>,
    pub request_id: Option<String>,
}

struct RawResponse {
    status: u16,
    headers: ResponseHeaders,
    body: Vec<u8>,
}

pub struct ApiClient {
    agent: ureq::Agent,
    settings: HttpSettings,
    token: Secret,
    error_context: ErrorContext,
    clock: Rc<dyn Clock>,
    reporter: Rc<Reporter>,
    last_request: Option<Duration>,
    last_scroll: Option<Duration>,
    jitter: Jitter,
}

impl ApiClient {
    pub fn new(
        settings: HttpSettings,
        token: Secret,
        error_context: ErrorContext,
        clock: Rc<dyn Clock>,
        reporter: Rc<Reporter>,
    ) -> Self {
        let mut config = ureq::Agent::config_builder();
        // http к localhost идёт открытым текстом; прокси из HTTP(S)_PROXY увидел бы токен.
        if crate::auth::url_is_loopback(&settings.base_url) {
            config = config.proxy(None);
        }
        let agent: ureq::Agent = config
            .http_status_as_error(false)
            .timeout_global(Some(settings.timeout))
            // Редирект у API — признак ошибки в base URL; Authorization ureq всё равно не переносит.
            .max_redirects(0)
            .user_agent(format!("aplaut-cli/{}", env!("CARGO_PKG_VERSION")))
            .build()
            .into();
        let seed = (clock.unix_millis() as u64) ^ ((std::process::id() as u64) << 32);
        ApiClient {
            agent,
            settings,
            token,
            error_context,
            clock,
            reporter,
            last_request: None,
            last_scroll: None,
            jitter: Jitter::new(seed),
        }
    }

    pub fn get(
        &mut self,
        path: &str,
        query: &[(&str, &str)],
        pace: Pace,
    ) -> Result<ApiResponse, CliError> {
        self.request(path, query, pace, Replay::Safe)
    }

    /// Продолжение обхода по курсору: повторяется только после 429, 503 и ошибок соединения,
    /// до которых запрос не дошёл до сервера; иначе — ошибка `OUTCOME_UNKNOWN`.
    pub fn get_continuation(
        &mut self,
        path: &str,
        query: &[(&str, &str)],
    ) -> Result<ApiResponse, CliError> {
        self.request(path, query, Pace::Scroll, Replay::OnlyIfUnprocessed)
    }

    fn request(
        &mut self,
        path: &str,
        query: &[(&str, &str)],
        pace: Pace,
        replay: Replay,
    ) -> Result<ApiResponse, CliError> {
        let url = format!("{}{}", self.settings.base_url, path);
        let mut attempt = 0u32;
        loop {
            self.throttle(pace);
            self.reporter
                .debug(&format!("→ GET {}", describe(&url, query)));
            self.reporter
                .debug(&format!("  Authorization: Bearer {MASK}, Accept: {ACCEPT}"));
            let started = self.clock.elapsed();
            let delay = match self.send(&url, query) {
                Ok(resp) => {
                    self.reporter.debug(&format!(
                        "← {} за {} мс, {} Б{}",
                        resp.status,
                        self.clock.elapsed().saturating_sub(started).as_millis(),
                        resp.body.len(),
                        resp.headers
                            .request_id
                            .as_deref()
                            .map(|id| format!(", request id {id}"))
                            .unwrap_or_default()
                    ));
                    if (200..300).contains(&resp.status) {
                        return Ok(ApiResponse {
                            status: resp.status,
                            body: resp.body,
                            request_id: resp.headers.request_id,
                        });
                    }
                    let unprocessed = matches!(resp.status, 429 | 503);
                    if replay == Replay::OnlyIfUnprocessed && resp.status >= 500 && !unprocessed {
                        return Err(self
                            .outcome_unknown(&format!("сервер ответил {}", resp.status))
                            .with_request_id(resp.headers.request_id.clone()));
                    }
                    match self.retry_delay(&resp, attempt) {
                        Some(delay) if attempt < self.settings.max_retries => {
                            self.note_retry(
                                &format!("сервер ответил {}", resp.status),
                                delay,
                                attempt,
                            );
                            delay
                        }
                        _ => return Err(self.api_error(&resp)),
                    }
                }
                Err(err) => {
                    let error = self.transport_error(&err);
                    let never_sent = matches!(
                        err,
                        ureq::Error::ConnectionFailed | ureq::Error::HostNotFound
                    );
                    if replay == Replay::OnlyIfUnprocessed && !never_sent {
                        return Err(self.outcome_unknown(&error.message));
                    }
                    if !error.retryable || attempt >= self.settings.max_retries {
                        return Err(error);
                    }
                    let delay = backoff(attempt, &mut self.jitter);
                    self.note_retry(&error.message, delay, attempt);
                    delay
                }
            };
            self.clock.sleep(delay);
            attempt += 1;
        }
    }

    fn send(&self, url: &str, query: &[(&str, &str)]) -> Result<RawResponse, ureq::Error> {
        let mut request = self
            .agent
            .get(url)
            .header("Authorization", format!("Bearer {}", self.token.expose()))
            .header("Accept", ACCEPT);
        for (key, value) in query {
            request = request.query(*key, *value);
        }
        let mut response = request.call()?;
        let status = response.status().as_u16();
        let headers = {
            let h = response.headers();
            let get = |name: &str| {
                h.get(name)
                    .and_then(|v| v.to_str().ok())
                    .map(str::to_string)
            };
            ResponseHeaders {
                request_id: get("x-request-id"),
                retry_after: get("retry-after"),
                rate_limit_reset: get("x-ratelimit-reset"),
                date: get("date"),
                www_authenticate: get("www-authenticate"),
                location: get("location"),
            }
        };
        let body = response
            .body_mut()
            .with_config()
            .limit(MAX_BODY_BYTES)
            .read_to_vec()?;
        Ok(RawResponse {
            status,
            headers,
            body,
        })
    }

    fn throttle(&mut self, pace: Pace) {
        let now = self.clock.elapsed();
        let mut wait = Duration::ZERO;
        if let Some(last) = self.last_request {
            wait = wait.max((last + MIN_INTERVAL).saturating_sub(now));
        }
        if pace == Pace::Scroll {
            if let Some(last) = self.last_scroll {
                wait = wait.max((last + SCROLL_INTERVAL).saturating_sub(now));
            }
        }
        if !wait.is_zero() {
            self.clock.sleep(wait);
        }
        let at = self.clock.elapsed();
        self.last_request = Some(at);
        if pace == Pace::Scroll {
            self.last_scroll = Some(at);
        }
    }

    fn retry_delay(&mut self, resp: &RawResponse, attempt: u32) -> Option<Duration> {
        match resp.status {
            429 => {
                let delay = match rate_limit_delay(&resp.headers, self.clock.unix_millis()) {
                    Some(d) => {
                        d + Duration::from_millis(self.jitter.below(RATE_LIMIT_JITTER_MS + 1))
                    }
                    None => backoff(attempt, &mut self.jitter),
                };
                (delay <= MAX_RATE_LIMIT_WAIT).then_some(delay)
            }
            503 => {
                let delay = resp
                    .headers
                    .retry_after
                    .as_deref()
                    .and_then(parse_retry_after)
                    .unwrap_or_else(|| backoff(attempt, &mut self.jitter));
                // Тот же потолок, что для 429: страница техработ с Retry-After на часы не должна
                // превращать cron-задачу в молча висящий процесс.
                (delay <= MAX_RATE_LIMIT_WAIT).then_some(delay)
            }
            500..=599 => Some(backoff(attempt, &mut self.jitter)),
            _ => None,
        }
    }

    fn note_retry(&self, reason: &str, delay: Duration, attempt: u32) {
        self.reporter.info(&format!(
            "{reason}; повтор {}/{} через {:.1} с",
            attempt + 1,
            self.settings.max_retries,
            delay.as_secs_f64()
        ));
    }

    fn api_error(&self, resp: &RawResponse) -> CliError {
        let mut err =
            api_error::from_response(resp.status, &resp.headers, &resp.body, &self.error_context);
        err.message = self.token.redact(&err.message);
        err.hint = err.hint.map(|h| self.token.redact(&h));
        // Сколько ждать по словам сервера — агенту, чтобы не повторять раньше времени.
        let wait = match resp.status {
            429 => rate_limit_delay(&resp.headers, self.clock.unix_millis()),
            503 => resp
                .headers
                .retry_after
                .as_deref()
                .and_then(parse_retry_after),
            _ => None,
        };
        err.with_retry_after(wait)
    }

    fn outcome_unknown(&self, reason: &str) -> CliError {
        CliError::general(
            OUTCOME_UNKNOWN,
            format!("{reason}; неизвестно, обработал ли сервер запрос"),
        )
    }

    fn transport_error(&self, err: &ureq::Error) -> CliError {
        use ureq::Error as E;
        let (code, retryable) = match err {
            E::Timeout(_) => ("timeout", true),
            E::Io(_) | E::ConnectionFailed | E::HostNotFound | E::Protocol(_) | E::BodyStalled => {
                ("network_error", true)
            }
            E::BodyExceedsLimit(_) => ("response_too_large", false),
            _ => ("network_error", false),
        };
        let text = self.token.redact(&err.to_string());
        CliError::general(
            code,
            format!("запрос к {} не выполнен: {text}", self.settings.base_url),
        )
        .retryable(retryable)
    }
}

/// max(Retry-After, Reset − Date). Сравниваем Reset с часами сервера из `Date`, а не с
/// локальными: расхождение часов не должно превращаться в лишние 429 или долгий сон.
pub fn rate_limit_delay(headers: &ResponseHeaders, local_now_ms: i64) -> Option<Duration> {
    let retry_after = headers.retry_after.as_deref().and_then(parse_retry_after);
    let server_now = headers
        .date
        .as_deref()
        .and_then(time::parse_http_date)
        .unwrap_or(local_now_ms);
    let until_reset = headers
        .rate_limit_reset
        .as_deref()
        .and_then(time::parse_rate_limit_reset)
        .map(|reset| Duration::from_millis((reset - server_now).max(0) as u64));
    match (retry_after, until_reset) {
        (Some(a), Some(b)) => Some(a.max(b)),
        (a, b) => a.or(b),
    }
}

fn parse_retry_after(value: &str) -> Option<Duration> {
    value.trim().parse::<u64>().ok().map(Duration::from_secs)
}

/// Полный джиттер (AWS): равномерно в [0, min(потолок, база·2^n)], но не меньше 100 мс.
fn backoff(attempt: u32, jitter: &mut Jitter) -> Duration {
    let ceiling = BACKOFF_BASE
        .saturating_mul(1u32 << attempt.min(16))
        .min(BACKOFF_CAP);
    Duration::from_millis(jitter.below(ceiling.as_millis() as u64 + 1)).max(BACKOFF_FLOOR)
}

/// Для `--verbose`: курсор в ~1 КБ только засоряет вывод.
fn describe(url: &str, query: &[(&str, &str)]) -> String {
    if query.is_empty() {
        return url.to_string();
    }
    let parts: Vec<String> = query
        .iter()
        .map(|(k, v)| {
            if *k == "cursor" {
                format!("{k}=<{} символов>", v.chars().count())
            } else {
                format!("{k}={v}")
            }
        })
        .collect();
    format!("{url}?{}", parts.join("&"))
}

/// xorshift64*: джиттеру нужна только разница между процессами, криптостойкость не нужна.
struct Jitter(u64);

impl Jitter {
    fn new(seed: u64) -> Self {
        Jitter(seed | 1)
    }

    /// Равномерно в [0, bound).
    fn below(&mut self, bound: u64) -> u64 {
        if bound == 0 {
            return 0;
        }
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D) % bound
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delay_is_relative_to_server_date() {
        let h = ResponseHeaders {
            date: Some("Wed, 23 Sep 2026 10:11:09 GMT".into()),
            rate_limit_reset: Some("2026-09-23 10:11:10 +0000".into()),
            ..ResponseHeaders::default()
        };
        // Локальные часы убежали на час вперёд — на результат это не влияет.
        assert_eq!(
            rate_limit_delay(&h, 1_790_158_269_000 + 3_600_000),
            Some(Duration::from_secs(1))
        );
        assert_eq!(rate_limit_delay(&ResponseHeaders::default(), 0), None);
    }

    #[test]
    fn backoff_stays_within_bounds() {
        let mut j = Jitter::new(42);
        for attempt in 0..20 {
            let d = backoff(attempt, &mut j);
            assert!(d >= BACKOFF_FLOOR && d <= BACKOFF_CAP, "{attempt}: {d:?}");
        }
        assert!(backoff(0, &mut j) <= Duration::from_secs(1));
    }
}
