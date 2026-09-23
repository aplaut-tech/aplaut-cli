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

#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConfigFile {
    #[serde(default)]
    pub profiles: BTreeMap<String, ProfileConfig>,
}

#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProfileConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
}

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
    load_toml(&paths.config).map_err(|message| CliError::general("config_invalid", message))
}

pub fn load_credentials(paths: &Paths, reporter: &Reporter) -> Result<CredentialsFile, CliError> {
    if fsutil::is_group_or_world_accessible(&paths.credentials).unwrap_or(false) {
        reporter.warn(&format!(
            "файл {0} доступен другим пользователям; выполните: chmod 600 {0}",
            paths.credentials.display()
        ));
    }
    load_toml(&paths.credentials)
        .map_err(|message| CliError::general("credentials_invalid", message))
}

pub fn save_config(paths: &Paths, config: &ConfigFile) -> Result<(), CliError> {
    save_toml(paths, &paths.config, config)
}

pub fn save_credentials(paths: &Paths, credentials: &CredentialsFile) -> Result<(), CliError> {
    save_toml(paths, &paths.credentials, credentials)
}

/// Текст ошибки — только `message()` без цитаты строки: в `credentials` это был бы токен.
fn load_toml<T: DeserializeOwned + Default>(path: &Path) -> Result<T, String> {
    match fs::read_to_string(path) {
        Ok(text) => toml::from_str(&text).map_err(|e| {
            format!(
                "{}: не удалось разобрать TOML: {}",
                path.display(),
                e.message()
            )
        }),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(T::default()),
        Err(e) => Err(format!("{}: {e}", path.display())),
    }
}

fn save_toml<T: Serialize>(paths: &Paths, path: &Path, value: &T) -> Result<(), CliError> {
    fsutil::ensure_private_dir(&paths.dir)
        .map_err(|e| CliError::io(&format!("создание {}", paths.dir.display()), &e))?;
    let text = toml::to_string_pretty(value).map_err(|e| {
        CliError::general(
            "config_invalid",
            format!("сериализация {}: {e}", path.display()),
        )
    })?;
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
        let p = paths("broken");
        fs::create_dir_all(&p.dir).unwrap();
        fs::write(
            &p.credentials,
            "[profiles.ci]\naccess_token = \"super-secret-token\n",
        )
        .unwrap();
        fs::set_permissions(&p.credentials, fs::Permissions::from_mode(0o600)).unwrap();
        let reporter =
            Reporter::with_writer(false, false, false, false, Box::new(SharedBuf::default()));
        let err = load_credentials(&p, &reporter).unwrap_err();
        assert_eq!(err.code, "credentials_invalid");
        assert!(
            !err.message.contains("super-secret-token"),
            "{}",
            err.message
        );
        fs::remove_dir_all(p.dir.parent().unwrap()).unwrap();
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
