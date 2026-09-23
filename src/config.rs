//! `config.toml` (профили без секретов) и `credentials` (токены) в `$XDG_CONFIG_HOME/aplaut/`.
//!
//! Секреты лежат отдельно, чтобы конфиг можно было показать в issue.

use std::collections::BTreeMap;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::auth::EnvSnapshot;
use crate::error::CliError;
use crate::fsutil;
use crate::term::Reporter;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Paths {
    pub dir: PathBuf,
    pub config: PathBuf,
    pub credentials: PathBuf,
}

impl Paths {
    pub fn resolve(env: &EnvSnapshot) -> Result<Paths, CliError> {
        let base = env
            .xdg_config_home
            .clone()
            .or_else(|| env.home.as_ref().map(|h| h.join(".config")))
            .ok_or_else(|| {
                CliError::general(
                    "no_config_dir",
                    "не удалось определить каталог конфигурации: не заданы XDG_CONFIG_HOME и HOME",
                )
            })?;
        let dir = base.join("aplaut");
        Ok(Paths {
            config: dir.join("config.toml"),
            credentials: dir.join("credentials"),
            dir,
        })
    }
}

/// Неизвестные ключи — ошибка: опечатка вроде `base-url` иначе молча отправила бы запросы на прод.
#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConfigFile {
    #[serde(default)]
    pub profiles: BTreeMap<String, ProfileConfig>,
}

/// Все поля перечислены в `CONFIG_HEADER` — тест не даст справке разойтись с кодом.
#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProfileConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
}

/// Справка в начале `config.toml`. aplaut пишет её при каждой своей записи файла, поэтому
/// список опций виден всегда, хотя сериализатор TOML не сохраняет комментарии.
pub const CONFIG_HEADER: &str = "\
# Профили aplaut CLI. Токенов здесь нет — они в ./credentials (aplaut auth login --profile NAME),
# поэтому этот файл можно показывать, например в issue.
# Профиль выбирается так: --profile NAME → APLAUT_PROFILE → \"default\".
# Править: aplaut profile edit (с проверкой) или aplaut profile set NAME ….
# Этот заголовок aplaut восстанавливает сам; другие комментарии пропадут при его записи.
#
# [profiles.NAME] — профиль: настройки одного аккаунта или стенда
#   - description — описание для людей, видно в aplaut profile list; по умолчанию нет
#   - base_url    — адрес Platform API; по умолчанию https://api.aplaut.io/v4.
#                   Удалённый хост — только https, http — только для localhost.
#                   Главнее него флаг --base-url и APLAUT_BASE_URL (если нет явного --profile).
#
# Пример:
# [profiles.staging]
# description = \"Стенд для тестов\"
# base_url = \"https://api.staging.example/v4\"
";

#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
pub struct CredentialsFile {
    #[serde(default)]
    pub profiles: BTreeMap<String, ProfileCredentials>,
}

#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct ProfileCredentials {
    pub access_token: String,
}

impl fmt::Debug for ProfileCredentials {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ProfileCredentials { access_token: *** }")
    }
}

pub fn load_config(paths: &Paths) -> Result<ConfigFile, CliError> {
    load_toml(&paths.config, false).map_err(|message| CliError::general("config_invalid", message))
}

/// Разбор текста конфига теми же правилами, что `load_config` (для проверки правок до записи).
/// `path` — только для сообщений об ошибке.
pub fn parse_config(text: &str, path: &Path) -> Result<ConfigFile, CliError> {
    parse_toml(text, path, false).map_err(|message| CliError::general("config_invalid", message))
}

pub fn load_credentials(paths: &Paths, reporter: &Reporter) -> Result<CredentialsFile, CliError> {
    if fsutil::is_group_or_world_accessible(&paths.credentials).unwrap_or(false) {
        reporter.warn(&format!(
            "файл {0} доступен другим пользователям; выполните: chmod 600 {0}",
            paths.credentials.display()
        ));
    }
    load_toml(&paths.credentials, true)
        .map_err(|message| CliError::general("credentials_invalid", message))
}

pub fn save_config(paths: &Paths, config: &ConfigFile) -> Result<(), CliError> {
    save_toml(paths, &paths.config, CONFIG_HEADER, config)
}

pub fn save_credentials(paths: &Paths, credentials: &CredentialsFile) -> Result<(), CliError> {
    save_toml(paths, &paths.credentials, "", credentials)
}

/// Сообщения парсера toml об ошибках типов цитируют значение (`invalid type: string "…"`),
/// а в `credentials` значение — это токен. Поэтому для секретного файла выводим только
/// номер строки, а текст парсера — лишь для `config.toml`, где секретов нет.
fn load_toml<T: DeserializeOwned + Default>(path: &Path, secret: bool) -> Result<T, String> {
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(T::default()),
        Err(e) => return Err(format!("{}: {e}", path.display())),
    };
    parse_toml(&text, path, secret)
}

fn parse_toml<T: DeserializeOwned>(text: &str, path: &Path, secret: bool) -> Result<T, String> {
    toml::from_str(text).map_err(|e| {
        let line = e
            .span()
            .map(|span| format!(" (строка {})", text[..span.start].matches('\n').count() + 1))
            .unwrap_or_default();
        if secret {
            format!("{}: не удалось разобрать TOML{line}", path.display())
        } else {
            format!(
                "{}: не удалось разобрать TOML{line}: {}",
                path.display(),
                e.message()
            )
        }
    })
}

fn save_toml<T: Serialize>(
    paths: &Paths,
    path: &Path,
    header: &str,
    value: &T,
) -> Result<(), CliError> {
    fsutil::ensure_private_dir(&paths.dir)
        .map_err(|e| CliError::io(&format!("создание {}", paths.dir.display()), &e))?;
    let text = toml::to_string_pretty(value).map_err(|e| {
        CliError::general(
            "config_invalid",
            format!("сериализация {}: {e}", path.display()),
        )
    })?;
    let text = if header.is_empty() {
        text
    } else {
        format!("{header}\n{text}")
    };
    fsutil::write_atomic(path, text.as_bytes())
        .map_err(|e| CliError::io(&format!("запись {}", path.display()), &e))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::term::{Reporter, SharedBuf};
    use std::os::unix::fs::PermissionsExt;

    fn paths(tag: &str) -> Paths {
        let base = std::env::temp_dir().join(format!("aplaut-config-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        let env = EnvSnapshot {
            xdg_config_home: Some(base),
            ..EnvSnapshot::default()
        };
        Paths::resolve(&env).unwrap()
    }

    #[test]
    fn reference_header_mentions_every_profile_option_and_is_only_comments() {
        let every = ProfileConfig {
            description: Some("x".into()),
            base_url: Some("https://x/v4".into()),
        };
        let keys = serde_json::to_value(&every).unwrap();
        for key in keys.as_object().unwrap().keys() {
            assert!(
                CONFIG_HEADER.contains(&format!("#   - {key}")),
                "опция {key} не описана в заголовке"
            );
        }
        assert!(CONFIG_HEADER
            .lines()
            .all(|l| l.is_empty() || l.starts_with('#')));
        assert_eq!(
            toml::from_str::<ConfigFile>(CONFIG_HEADER).unwrap(),
            ConfigFile::default()
        );
    }

    #[test]
    fn xdg_then_home() {
        let env = EnvSnapshot {
            home: Some("/home/u".into()),
            ..EnvSnapshot::default()
        };
        assert_eq!(
            Paths::resolve(&env).unwrap().config,
            PathBuf::from("/home/u/.config/aplaut/config.toml")
        );
        let env = EnvSnapshot {
            xdg_config_home: Some("/x".into()),
            home: Some("/home/u".into()),
            ..EnvSnapshot::default()
        };
        assert_eq!(
            Paths::resolve(&env).unwrap().credentials,
            PathBuf::from("/x/aplaut/credentials")
        );
        assert!(Paths::resolve(&EnvSnapshot::default()).is_err());
    }

    #[test]
    fn roundtrip_with_private_permissions() {
        let p = paths("roundtrip");
        let mut creds = CredentialsFile::default();
        creds.profiles.insert(
            "ci".into(),
            ProfileCredentials {
                access_token: "tok".into(),
            },
        );
        save_credentials(&p, &creds).unwrap();
        let mut cfg = ConfigFile::default();
        cfg.profiles.insert(
            "ci".into(),
            ProfileConfig {
                base_url: Some("https://x/v4".into()),
                description: Some("Тестовый профиль".into()),
            },
        );
        save_config(&p, &cfg).unwrap();
        let reporter =
            Reporter::with_writer(false, false, false, false, Box::new(SharedBuf::default()));
        assert_eq!(load_credentials(&p, &reporter).unwrap(), creds);
        assert_eq!(load_config(&p).unwrap(), cfg);
        assert_eq!(
            fs::metadata(&p.credentials).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(
            fs::metadata(&p.dir).unwrap().permissions().mode() & 0o777,
            0o700
        );
        assert!(
            !format!("{creds:?}").contains("tok\""),
            "Debug не должен показывать токен"
        );
        fs::remove_dir_all(p.dir.parent().unwrap()).unwrap();
    }

    #[test]
    fn missing_files_are_empty() {
        let p = paths("missing");
        let reporter =
            Reporter::with_writer(false, false, false, false, Box::new(SharedBuf::default()));
        assert_eq!(
            load_credentials(&p, &reporter).unwrap(),
            CredentialsFile::default()
        );
        assert_eq!(load_config(&p).unwrap(), ConfigFile::default());
    }

    #[test]
    fn broken_credentials_error_does_not_echo_token() {
        // Синтаксическая ошибка и ошибки типов: сообщения toml для последних цитируют значение.
        let shapes = [
            "[profiles.ci]\naccess_token = \"super-secret-token\n",
            "[profiles]\ndefault = \"super-secret-token\"\n",
            "profiles = \"super-secret-token\"\n",
            "[profiles.ci]\naccess_token = 12345678901234\n",
        ];
        for (i, text) in shapes.iter().enumerate() {
            let p = paths(&format!("broken{i}"));
            fs::create_dir_all(&p.dir).unwrap();
            fs::write(&p.credentials, text).unwrap();
            fs::set_permissions(&p.credentials, fs::Permissions::from_mode(0o600)).unwrap();
            let reporter =
                Reporter::with_writer(false, false, false, false, Box::new(SharedBuf::default()));
            let err = load_credentials(&p, &reporter).unwrap_err();
            assert_eq!(err.code, "credentials_invalid");
            for secret in ["super-secret-token", "12345678901234"] {
                assert!(!err.message.contains(secret), "{i}: {}", err.message);
            }
            assert!(
                err.message.contains("строка"),
                "{i}: номер строки помогает найти ошибку: {}",
                err.message
            );
            fs::remove_dir_all(p.dir.parent().unwrap()).unwrap();
        }
    }

    #[test]
    fn warns_when_credentials_are_readable_by_others() {
        let p = paths("perms");
        fs::create_dir_all(&p.dir).unwrap();
        fs::write(&p.credentials, "").unwrap();
        fs::set_permissions(&p.credentials, fs::Permissions::from_mode(0o644)).unwrap();
        let buf = SharedBuf::default();
        let reporter = Reporter::with_writer(false, false, false, false, Box::new(buf.clone()));
        load_credentials(&p, &reporter).unwrap();
        assert!(buf.contents().contains("chmod 600"));
        fs::remove_dir_all(p.dir.parent().unwrap()).unwrap();
    }
}
