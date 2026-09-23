//! Стейт обхода: курсор и всё, что нужно для продолжения после падения.
//!
//! Пишется атомарно и только в точках фиксации формата (дизайн §6.1): стейт никогда не
//! указывает на данные, которых нет у приёмника.

use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::error::{CliError, Exit};
use crate::fsutil;
use crate::time;

pub const STATE_VERSION: u32 = 1;

/// Параметры фиксируются при открытии обхода: сервер отвергает курсор с другими параметрами.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScrollParams {
    pub filter: Option<String>,
    pub sort: String,
    pub include: Vec<String>,
    pub per_page: u32,
    /// `--fields`: в API не уходит, но входит в стейт — продолжение пишет те же колонки, что
    /// проверены на первой странице обхода; иначе склеенный CSV разъехался бы.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fields: Option<Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScrollState {
    pub version: u32,
    pub records_type: String,
    pub params: ScrollParams,
    pub cursor: Option<String>,
    /// Сервер после паузы > 2 мин повторяет хвост последней страницы — по этим id его отсеиваем.
    #[serde(default)]
    pub last_page_ids: Vec<String>,
    pub emitted: u64,
    pub total_count: Option<u64>,
    pub applied_filter: Option<String>,
    pub completed: bool,
    /// Запрос продолжения ушёл, а его страница ещё не зафиксирована. Курсор не идемпотентен
    /// (повтор отдаёт следующую страницу), поэтому после сбоя позиция сервера неизвестна.
    #[serde(default)]
    pub in_flight: bool,
    /// Поле сортировки последней выданной записи: граница для безопасного продолжения
    /// новым обходом, если позиция курсора неизвестна.
    #[serde(default)]
    pub last_sort_value: Option<String>,
    #[serde(default)]
    pub saved_at: String,
}

impl ScrollState {
    pub fn new(records_type: &str, params: ScrollParams) -> Self {
        ScrollState {
            version: STATE_VERSION,
            records_type: records_type.to_string(),
            params,
            cursor: None,
            last_page_ids: Vec::new(),
            emitted: 0,
            total_count: None,
            applied_filter: None,
            completed: false,
            in_flight: false,
            last_sort_value: None,
            saved_at: String::new(),
        }
    }

    pub fn check_matches(
        &self,
        records_type: &str,
        params: &ScrollParams,
        path: &Path,
    ) -> Result<(), CliError> {
        let restart = format!("удалите {}, чтобы начать обход заново", path.display());
        if self.version != STATE_VERSION {
            return Err(CliError::usage(
                "state_version",
                format!(
                    "стейт {} записан версией формата {}, ожидается {STATE_VERSION}",
                    path.display(),
                    self.version
                ),
            )
            .with_hint(restart));
        }
        if self.records_type != records_type {
            return Err(CliError::usage(
                "state_mismatch",
                format!(
                    "стейт {} относится к {}, а не к {records_type}",
                    path.display(),
                    self.records_type
                ),
            )
            .with_hint(restart));
        }
        let mut diffs = Vec::new();
        let show = |v: &Option<String>| v.clone().unwrap_or_else(|| "—".into());
        if self.params.filter != params.filter {
            diffs.push(format!(
                "filter: в стейте «{}», передан «{}»",
                show(&self.params.filter),
                show(&params.filter)
            ));
        }
        if self.params.sort != params.sort {
            diffs.push(format!(
                "sort: в стейте «{}», передан «{}»",
                self.params.sort, params.sort
            ));
        }
        if self.params.include != params.include {
            diffs.push(format!(
                "include: в стейте «{}», передан «{}»",
                self.params.include.join(","),
                params.include.join(",")
            ));
        }
        if self.params.per_page != params.per_page {
            diffs.push(format!(
                "per_page: в стейте {}, передан {}",
                self.params.per_page, params.per_page
            ));
        }
        if self.params.fields != params.fields {
            let show = |v: &Option<Vec<String>>| v.as_ref().map_or("—".into(), |f| f.join(","));
            diffs.push(format!(
                "fields: в стейте «{}», передан «{}»",
                show(&self.params.fields),
                show(&params.fields)
            ));
        }
        if diffs.is_empty() {
            return Ok(());
        }
        Err(CliError::usage(
            "state_mismatch",
            format!(
                "параметры не совпадают со стейтом {}: {}",
                path.display(),
                diffs.join("; ")
            ),
        )
        .with_hint(format!(
            "передайте те же параметры, что при первом запуске, или {restart}"
        )))
    }
}

pub fn load(path: &Path) -> Result<Option<ScrollState>, CliError> {
    match fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes).map(Some).map_err(|e| {
            CliError::usage(
                "state_invalid",
                format!("файл стейта {} повреждён: {e}", path.display()),
            )
            .with_hint("удалите файл, чтобы начать обход заново")
        }),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(CliError::new(
            Exit::General,
            "io_error",
            format!("чтение стейта {}: {e}", path.display()),
        )),
    }
}

pub fn save(path: &Path, state: &mut ScrollState, now_ms: i64) -> Result<(), CliError> {
    state.saved_at = time::format_rfc3339_utc(now_ms);
    let mut bytes = serde_json::to_vec_pretty(state).expect("ScrollState сериализуется всегда");
    bytes.push(b'\n');
    fsutil::write_atomic(path, &bytes)
        .map_err(|e| CliError::io(&format!("запись стейта {}", path.display()), &e))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    use std::path::PathBuf;

    fn params(filter: Option<&str>) -> ScrollParams {
        ScrollParams {
            filter: filter.map(str::to_string),
            sort: "updated_at:asc".into(),
            include: vec!["author".into()],
            per_page: 100,
            fields: None,
        }
    }

    fn path(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("aplaut-state-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir.join("state.json")
    }

    #[test]
    fn roundtrip_is_private_and_stamped() {
        let p = path("roundtrip");
        let mut state = ScrollState::new(
            "reviews",
            params(Some("updated_at:gte:2020-01-01T00:00:00Z")),
        );
        state.cursor = Some("c1".into());
        state.last_page_ids = vec!["a".into(), "b".into()];
        state.emitted = 2;
        save(&p, &mut state, 1_790_158_270_000).unwrap();
        assert_eq!(state.saved_at, "2026-09-23T10:11:10Z");
        assert_eq!(load(&p).unwrap(), Some(state));
        assert_eq!(
            fs::metadata(&p).unwrap().permissions().mode() & 0o777,
            0o600
        );
        fs::remove_dir_all(p.parent().unwrap()).unwrap();
    }

    #[test]
    fn missing_state_is_none() {
        assert_eq!(load(&path("missing")).unwrap(), None);
    }

    #[test]
    fn corrupted_state_is_usage_error() {
        let p = path("corrupt");
        fs::write(&p, "{not json").unwrap();
        let err = load(&p).unwrap_err();
        assert_eq!(
            (err.code.as_str(), err.exit),
            ("state_invalid", Exit::Usage)
        );
        assert!(err.hint.unwrap().contains("удалите"));
        fs::remove_dir_all(p.parent().unwrap()).unwrap();
    }

    #[test]
    fn mismatch_names_the_difference() {
        let p = Path::new("/tmp/s.json");
        let state = ScrollState::new(
            "reviews",
            params(Some("updated_at:gte:2020-01-01T00:00:00Z")),
        );
        assert!(state
            .check_matches(
                "reviews",
                &params(Some("updated_at:gte:2020-01-01T00:00:00Z")),
                p
            )
            .is_ok());
        let err = state
            .check_matches("reviews", &params(Some("rating:eq:5")), p)
            .unwrap_err();
        assert_eq!(
            (err.code.as_str(), err.exit),
            ("state_mismatch", Exit::Usage)
        );
        assert!(
            err.message.contains("filter") && err.message.contains("rating:eq:5"),
            "{}",
            err.message
        );
        assert_eq!(
            state
                .check_matches("products", &state.params, p)
                .unwrap_err()
                .code,
            "state_mismatch"
        );
        let mut old = state.clone();
        old.version = 99;
        assert_eq!(
            old.check_matches("reviews", &state.params, p)
                .unwrap_err()
                .code,
            "state_version"
        );
    }

    #[test]
    fn fields_are_part_of_the_state() {
        let mut saved = ScrollState::new("reviews", params(None));
        saved.params.fields = Some(vec!["id".into(), "rating".into()]);
        let other = ScrollParams {
            fields: Some(vec!["body".into(), "id".into()]),
            ..params(None)
        };
        let err = saved
            .check_matches("reviews", &other, Path::new("s.json"))
            .unwrap_err();
        assert_eq!(err.code, "state_mismatch");
        assert!(
            err.message
                .contains("fields: в стейте «id,rating», передан «body,id»"),
            "{}",
            err.message
        );
        let none = saved
            .check_matches("reviews", &params(None), Path::new("s.json"))
            .unwrap_err();
        assert!(none.message.contains("передан «—»"), "{}", none.message);
    }

    #[test]
    fn state_without_fields_keeps_its_format() {
        let json = serde_json::to_string(&ScrollState::new("reviews", params(None))).unwrap();
        assert!(!json.contains("fields"), "{json}");
        let back: ScrollState = serde_json::from_str(&json).unwrap();
        assert_eq!(back.params.fields, None);
    }
}
