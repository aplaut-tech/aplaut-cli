//! Разбор дат без внешних зависимостей.
//!
//! Сервер использует три формата: RFC 3339 в данных, формат Ruby `Time#to_s` в
//! `X-RateLimit-Reset` (вопреки спеке, где обещан date-time) и IMF-fixdate в `Date`.

pub type UnixMillis = i64;

pub fn parse_rfc3339(s: &str) -> Option<UnixMillis> {
    let s = s.trim();
    let date = s.get(..10)?;
    let rest = s.get(10..)?;
    let rest = rest.strip_prefix('T').or_else(|| rest.strip_prefix('t'))?;
    let offset_at = rest.find(['Z', 'z', '+', '-'])?;
    let (time, offset) = rest.split_at(offset_at);
    Some(local_millis(date, time)? - offset_seconds(offset)? * 1000)
}

/// `2026-09-23 10:11:10 +0000` — так сервер присылает `X-RateLimit-Reset`.
pub fn parse_ruby_time(s: &str) -> Option<UnixMillis> {
    let mut parts = s.split_whitespace();
    let (date, time, offset) = (parts.next()?, parts.next()?, parts.next()?);
    if parts.next().is_some() {
        return None;
    }
    Some(local_millis(date, time)? - offset_seconds(offset)? * 1000)
}

pub fn parse_rate_limit_reset(s: &str) -> Option<UnixMillis> {
    parse_rfc3339(s).or_else(|| parse_ruby_time(s))
}

/// `Wed, 23 Sep 2026 10:11:09 GMT`.
pub fn parse_http_date(s: &str) -> Option<UnixMillis> {
    let mut parts = s.split_whitespace();
    let _weekday = parts.next()?;
    let day = number(parts.next()?)?;
    let month = match parts.next()? {
        "Jan" => 1,
        "Feb" => 2,
        "Mar" => 3,
        "Apr" => 4,
        "May" => 5,
        "Jun" => 6,
        "Jul" => 7,
        "Aug" => 8,
        "Sep" => 9,
        "Oct" => 10,
        "Nov" => 11,
        "Dec" => 12,
        _ => return None,
    };
    let year = number(parts.next()?)?;
    let time = parts.next()?;
    if parts.next()? != "GMT" {
        return None;
    }
    local_millis(&format!("{year:04}-{month:02}-{day:02}"), time)
}

pub fn format_rfc3339_utc(millis: UnixMillis) -> String {
    let secs = millis.div_euclid(1000);
    let (year, month, day) = civil_from_days(secs.div_euclid(86_400));
    let sod = secs.rem_euclid(86_400);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        sod / 3600,
        sod % 3600 / 60,
        sod % 60
    )
}

pub fn system_now_millis() -> UnixMillis {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    now.as_millis() as UnixMillis
}

fn number(s: &str) -> Option<u32> {
    if s.is_empty() || !s.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    s.parse().ok()
}

/// `YYYY-MM-DD` + `HH:MM:SS[.fff…]` → миллисекунды без учёта смещения.
fn local_millis(date: &str, time: &str) -> Option<UnixMillis> {
    let mut d = date.split('-');
    let (year, month, day) = (number(d.next()?)?, number(d.next()?)?, number(d.next()?)?);
    if d.next().is_some() || !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    let (hms, fraction) = match time.split_once('.') {
        Some((hms, f)) => (hms, Some(f)),
        None => (time, None),
    };
    let mut t = hms.split(':');
    let (hour, minute, second) = (number(t.next()?)?, number(t.next()?)?, number(t.next()?)?);
    if t.next().is_some() || hour > 23 || minute > 59 || second > 60 {
        return None;
    }
    let millis = match fraction {
        None => 0,
        Some(f) => {
            if f.is_empty() || !f.bytes().all(|b| b.is_ascii_digit()) {
                return None;
            }
            let digits = &f[..f.len().min(3)];
            number(digits)? * 10u32.pow(3 - digits.len() as u32)
        }
    };
    let days = days_from_civil(year as i64, month, day);
    let secs = days * 86_400 + hour as i64 * 3600 + minute as i64 * 60 + second as i64;
    Some(secs * 1000 + millis as i64)
}

/// `Z`, `+03:00` или `+0300` → секунды смещения.
fn offset_seconds(s: &str) -> Option<i64> {
    if s == "Z" || s == "z" {
        return Some(0);
    }
    let sign = match s.as_bytes().first()? {
        b'+' => 1,
        b'-' => -1,
        _ => return None,
    };
    let rest = &s[1..];
    let (hh, mm) = match rest.split_once(':') {
        Some(parts) => parts,
        None if rest.len() == 4 => rest.split_at(2),
        None => return None,
    };
    if hh.len() != 2 || mm.len() != 2 {
        return None;
    }
    let (h, m) = (number(hh)?, number(mm)?);
    if h > 23 || m > 59 {
        return None;
    }
    Some(sign * (h as i64 * 3600 + m as i64 * 60))
}

/// Howard Hinnant, days_from_civil: дни от 1970-01-01 по пролептическому григорианскому календарю.
fn days_from_civil(year: i64, month: u32, day: u32) -> i64 {
    let y = if month <= 2 { year - 1 } else { year };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let m = month as i64;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + day as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let year = yoe + era * 400 + if month <= 2 { 1 } else { 0 };
    (year, month, day)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rfc3339_with_offset_and_millis() {
        assert_eq!(
            parse_rfc3339("2021-03-11T12:31:31.561+03:00"),
            Some(1_615_455_091_561)
        );
        assert_eq!(
            parse_rfc3339("2021-03-11T09:31:31.561Z"),
            Some(1_615_455_091_561)
        );
        assert_eq!(
            parse_rfc3339("2014-01-20T00:00:00.000+04:00"),
            Some(1_390_161_600_000)
        );
        assert_eq!(
            parse_rfc3339("2026-09-23T10:11:10+0000"),
            Some(1_790_158_270_000)
        );
    }

    #[test]
    fn rfc3339_rejects_garbage() {
        for bad in [
            "",
            "2021-03-11",
            "2021-13-01T00:00:00Z",
            "2021-03-11T25:00:00Z",
            "2021-03-11T00:00:00",
            "x021-03-11T00:00:00Z",
        ] {
            assert_eq!(parse_rfc3339(bad), None, "{bad}");
        }
    }

    #[test]
    fn ruby_time_as_sent_in_rate_limit_reset() {
        assert_eq!(
            parse_ruby_time("2026-09-23 10:11:10 +0000"),
            Some(1_790_158_270_000)
        );
        assert_eq!(
            parse_rate_limit_reset("2026-09-23 10:11:10 +0000"),
            Some(1_790_158_270_000)
        );
        assert_eq!(
            parse_rate_limit_reset("2026-09-23T10:11:10+00:00"),
            Some(1_790_158_270_000)
        );
    }

    #[test]
    fn http_date_header() {
        assert_eq!(
            parse_http_date("Wed, 23 Sep 2026 10:11:09 GMT"),
            Some(1_790_158_269_000)
        );
        assert_eq!(parse_http_date("Wed, 23 Sep 2026 10:11:09 MSK"), None);
    }

    #[test]
    fn formats_utc_and_handles_leap_and_pre_epoch() {
        assert_eq!(format_rfc3339_utc(0), "1970-01-01T00:00:00Z");
        assert_eq!(format_rfc3339_utc(951_868_799_000), "2000-02-29T23:59:59Z");
        assert_eq!(format_rfc3339_utc(-1_000), "1969-12-31T23:59:59Z");
    }
}
