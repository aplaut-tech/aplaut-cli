//! Откуда берутся токен и base URL (дизайн §4). Явный флаг всегда сильнее env (clig).
//! Флага `--token` со значением нет намеренно: токен в argv виден в `ps` и истории шелла.

use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

use crate::config::{CredentialsFile, ProfileConfig};
use crate::error::{CliError, Exit};
use crate::secret::{parse_token, Secret};
use crate::term::TermEnv;

pub const DEFAULT_PROFILE: &str = "default";
pub const DEFAULT_BASE_URL: &str = "https://api.aplaut.io/v4";

/// Окружение читается один раз при старте: дальше код не трогает `std::env`, и его легко тестировать.
#[derive(Debug, Clone, Default)]
pub struct EnvSnapshot {
    pub access_token: Option<String>,
    pub access_token_file: Option<PathBuf>,
    pub profile: Option<String>,
    pub base_url: Option<String>,
    pub xdg_config_home: Option<PathBuf>,
    pub home: Option<PathBuf>,
    pub term: TermEnv,
}

impl EnvSnapshot {
    pub fn capture() -> Self {
        let var = |k: &str| std::env::var(k).ok().filter(|v| !v.is_empty());
        EnvSnapshot {
            access_token: var("APLAUT_ACCESS_TOKEN"),
            access_token_file: var("APLAUT_ACCESS_TOKEN_FILE").map(PathBuf::from),
            profile: var("APLAUT_PROFILE"),
            base_url: var("APLAUT_BASE_URL"),
            xdg_config_home: var("XDG_CONFIG_HOME").map(PathBuf::from),
            home: var("HOME").map(PathBuf::from),
            term: TermEnv::capture(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TokenSource {
    Stdin,
    File(PathBuf),
    Profile(String),
    EnvFile(PathBuf),
    Env,
    EnvProfile(String),
    DefaultProfile,
}

impl TokenSource {
    /// Для подсказки к 401: откуда взят токен — самая частая причина отказа.
    pub fn describe(&self) -> String {
        match self {
            TokenSource::Stdin => "stdin (--token-stdin)".into(),
            TokenSource::File(p) => format!("файла {} (--token-file)", p.display()),
            TokenSource::Profile(n) => format!("профиля «{n}» (--profile)"),
            TokenSource::EnvFile(p) => format!("APLAUT_ACCESS_TOKEN_FILE ({})", p.display()),
            TokenSource::Env => "APLAUT_ACCESS_TOKEN".into(),
            TokenSource::EnvProfile(n) => format!("профиля «{n}» (APLAUT_PROFILE)"),
            TokenSource::DefaultProfile => format!("профиля «{DEFAULT_PROFILE}»"),
        }
    }
}

pub struct TokenFlags<'a> {
    pub token_stdin: bool,
    pub token_file: Option<&'a Path>,
    pub profile: Option<&'a str>,
}

/// stdin как зависимость: в тестах токен — строка, а «stdin — терминал» задаётся флагом.
pub struct StdinSource<'a> {
    pub is_terminal: bool,
    pub reader: &'a mut dyn Read,
}

pub fn resolve_token(
    flags: &TokenFlags,
    env: &EnvSnapshot,
    creds: &CredentialsFile,
    stdin: &mut StdinSource,
) -> Result<(Secret, TokenSource), CliError> {
    if flags.token_stdin {
        return Ok((read_stdin_token(stdin)?, TokenSource::Stdin));
    }
    if let Some(path) = flags.token_file {
        let token = if path == Path::new("-") {
            read_stdin_token(stdin)?
        } else {
            read_token_file(path)?
        };
        return Ok((token, TokenSource::File(path.to_path_buf())));
    }
    if let Some(name) = flags.profile {
        return Ok((
            profile_token(creds, name)?,
            TokenSource::Profile(name.to_string()),
        ));
    }
    if let Some(path) = &env.access_token_file {
        return Ok((read_token_file(path)?, TokenSource::EnvFile(path.clone())));
    }
    if let Some(token) = &env.access_token {
        return Ok((parse_token(token)?, TokenSource::Env));
    }
    if let Some(name) = &env.profile {
        return Ok((
            profile_token(creds, name)?,
            TokenSource::EnvProfile(name.clone()),
        ));
    }
    match creds.profiles.get(DEFAULT_PROFILE) {
        Some(entry) => Ok((parse_token(&entry.access_token)?, TokenSource::DefaultProfile)),
        None => Err(CliError::new(Exit::Auth, "no_token", "токен не найден").with_hint(
            "выполните aplaut auth login или передайте токен через --token-file / APLAUT_ACCESS_TOKEN_FILE",
        )),
    }
}

pub fn read_stdin_token(stdin: &mut StdinSource) -> Result<Secret, CliError> {
    if stdin.is_terminal {
        return Err(CliError::usage(
            "stdin_is_terminal",
            "ожидался токен в stdin, но stdin — терминал",
        )
        .with_hint("передайте токен через пайп: echo \"$TOKEN\" | aplaut … --token-stdin"));
    }
    let mut text = String::new();
    stdin
        .reader
        .read_to_string(&mut text)
        .map_err(|e| CliError::io("чтение токена из stdin", &e))?;
    parse_token(&text)
}

pub fn read_token_file(path: &Path) -> Result<Secret, CliError> {
    let text = fs::read_to_string(path).map_err(|e| {
        CliError::new(
            Exit::Auth,
            "token_file_unreadable",
            format!("не удалось прочитать файл токена {}: {e}", path.display()),
        )
    })?;
    parse_token(&text)
}

pub fn active_profile(flag: Option<&str>, env: &EnvSnapshot) -> Result<String, CliError> {
    let name = flag
        .map(str::to_string)
        .or_else(|| env.profile.clone())
        .unwrap_or_else(|| DEFAULT_PROFILE.to_string());
    validate_profile_name(&name)?;
    Ok(name)
}

/// Имя профиля становится ключом TOML и частью сообщений — только безопасные символы.
pub fn validate_profile_name(name: &str) -> Result<(), CliError> {
    if !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Ok(());
    }
    Err(
        CliError::usage("invalid_profile", format!("недопустимое имя профиля «{name}»"))
            .with_field("profile")
            .with_hint("используйте латиницу, цифры, - и _"),
    )
}

pub fn resolve_base_url(
    flag: Option<&str>,
    env: &EnvSnapshot,
    profile: Option<&ProfileConfig>,
) -> Result<String, CliError> {
    let raw = flag
        .map(str::to_string)
        .or_else(|| env.base_url.clone())
        .or_else(|| profile.and_then(|p| p.base_url.clone()))
        .unwrap_or_else(|| DEFAULT_BASE_URL.to_string());
    validate_base_url(&raw)
}

/// Не-https разрешён только для loopback: иначе токен ушёл бы по сети открытым текстом.
pub fn validate_base_url(raw: &str) -> Result<String, CliError> {
    let url = raw.trim().trim_end_matches('/');
    let bad = || {
        CliError::usage("invalid_base_url", format!("некорректный base URL «{url}»"))
            .with_field("base_url")
            .with_hint("пример: https://api.aplaut.io/v4")
    };
    let (scheme, rest) = url.split_once("://").ok_or_else(bad)?;
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    let host = host_of(authority).ok_or_else(bad)?;
    match scheme {
        "https" => Ok(url.to_string()),
        "http" if is_loopback(host) => Ok(url.to_string()),
        "http" => Err(CliError::usage(
            "insecure_base_url",
            format!("base URL {url} использует http: токен ушёл бы по сети открытым текстом"),
        )
        .with_field("base_url")
        .with_hint("используйте https://; http допустим только для localhost")),
        _ => Err(bad()),
    }
}

pub fn is_loopback(host: &str) -> bool {
    let host = host.to_ascii_lowercase();
    host == "localhost"
        || host.ends_with(".localhost")
        || host == "::1"
        || host
            .parse::<std::net::Ipv4Addr>()
            .is_ok_and(|ip| ip.is_loopback())
}

fn host_of(authority: &str) -> Option<&str> {
    let authority = authority.rsplit('@').next()?;
    if let Some(rest) = authority.strip_prefix('[') {
        return rest.split(']').next().filter(|h| !h.is_empty());
    }
    let host = authority.split(':').next()?;
    (!host.is_empty()).then_some(host)
}

fn profile_token(creds: &CredentialsFile, name: &str) -> Result<Secret, CliError> {
    match creds.profiles.get(name) {
        Some(entry) => parse_token(&entry.access_token),
        None => Err(
            CliError::new(Exit::Auth, "no_token", format!("в профиле «{name}» нет токена"))
                .with_hint(format!("выполните aplaut auth login --profile {name}")),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ProfileCredentials;

    fn creds(entries: &[(&str, &str)]) -> CredentialsFile {
        let mut c = CredentialsFile::default();
        for (name, token) in entries {
            c.profiles.insert(name.to_string(), ProfileCredentials { access_token: token.to_string() });
        }
        c
    }

    fn resolve(flags: &TokenFlags, env: &EnvSnapshot, creds: &CredentialsFile, stdin: &str, tty: bool) -> Result<(String, TokenSource), CliError> {
        let mut reader = stdin.as_bytes();
        let mut src = StdinSource { is_terminal: tty, reader: &mut reader };
        resolve_token(flags, env, creds, &mut src).map(|(s, src)| (s.expose().to_string(), src))
    }

    fn no_flags() -> TokenFlags<'static> {
        TokenFlags { token_stdin: false, token_file: None, profile: None }
    }

    #[test]
    fn explicit_profile_beats_env_token() {
        let env = EnvSnapshot { access_token: Some("env-tok".into()), ..EnvSnapshot::default() };
        let flags = TokenFlags { profile: Some("ci"), ..no_flags() };
        let (tok, src) = resolve(&flags, &env, &creds(&[("ci", "ci-tok")]), "", false).unwrap();
        assert_eq!((tok.as_str(), src), ("ci-tok", TokenSource::Profile("ci".into())));
    }

    #[test]
    fn stdin_flag_beats_everything() {
        let env = EnvSnapshot { access_token: Some("env-tok".into()), ..EnvSnapshot::default() };
        let flags = TokenFlags { token_stdin: true, profile: Some("ci"), ..no_flags() };
        let (tok, src) = resolve(&flags, &env, &creds(&[("ci", "ci-tok")]), "stdin-tok\n", false).unwrap();
        assert_eq!((tok.as_str(), src), ("stdin-tok", TokenSource::Stdin));
    }

    #[test]
    fn stdin_terminal_is_rejected_instead_of_hanging() {
        let flags = TokenFlags { token_stdin: true, ..no_flags() };
        let err = resolve(&flags, &EnvSnapshot::default(), &creds(&[]), "", true).unwrap_err();
        assert_eq!((err.code.as_str(), err.exit), ("stdin_is_terminal", Exit::Usage));
    }

    #[test]
    fn env_order_file_then_value_then_env_profile_then_default() {
        let dir = std::env::temp_dir().join(format!("aplaut-auth-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("token");
        std::fs::write(&file, "file-tok\n").unwrap();
        let all = EnvSnapshot {
            access_token_file: Some(file.clone()),
            access_token: Some("env-tok".into()),
            profile: Some("prod".into()),
            ..EnvSnapshot::default()
        };
        let c = creds(&[("prod", "prod-tok"), ("default", "def-tok")]);
        assert_eq!(resolve(&no_flags(), &all, &c, "", false).unwrap().0, "file-tok");
        let no_file = EnvSnapshot { access_token_file: None, ..all.clone() };
        assert_eq!(resolve(&no_flags(), &no_file, &c, "", false).unwrap().0, "env-tok");
        let only_profile = EnvSnapshot { access_token: None, ..no_file.clone() };
        assert_eq!(resolve(&no_flags(), &only_profile, &c, "", false).unwrap().0, "prod-tok");
        assert_eq!(resolve(&no_flags(), &EnvSnapshot::default(), &c, "", false).unwrap().1, TokenSource::DefaultProfile);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn missing_token_is_auth_error_with_hint() {
        let err = resolve(&no_flags(), &EnvSnapshot::default(), &creds(&[]), "", false).unwrap_err();
        assert_eq!((err.code.as_str(), err.exit), ("no_token", Exit::Auth));
        assert!(err.hint.unwrap().contains("aplaut auth login"));
        let flags = TokenFlags { profile: Some("ghost"), ..no_flags() };
        let err = resolve(&flags, &EnvSnapshot::default(), &creds(&[]), "", false).unwrap_err();
        assert!(err.hint.unwrap().contains("--profile ghost"));
    }

    #[test]
    fn base_url_precedence_and_https_rule() {
        let env = EnvSnapshot { base_url: Some("https://env.example/v4".into()), ..EnvSnapshot::default() };
        let profile = ProfileConfig { base_url: Some("https://profile.example/v4".into()) };
        assert_eq!(resolve_base_url(Some("https://flag.example/v4/"), &env, Some(&profile)).unwrap(), "https://flag.example/v4");
        assert_eq!(resolve_base_url(None, &env, Some(&profile)).unwrap(), "https://env.example/v4");
        assert_eq!(resolve_base_url(None, &EnvSnapshot::default(), Some(&profile)).unwrap(), "https://profile.example/v4");
        assert_eq!(resolve_base_url(None, &EnvSnapshot::default(), None).unwrap(), DEFAULT_BASE_URL);
        for ok in ["http://localhost:3000/v4", "http://127.0.0.1:8080/v4", "http://[::1]:9000/v4", "http://api.localhost/v4"] {
            assert!(validate_base_url(ok).is_ok(), "{ok}");
        }
        let err = validate_base_url("http://api.aplaut.io/v4").unwrap_err();
        assert_eq!((err.code.as_str(), err.field.as_deref()), ("insecure_base_url", Some("base_url")));
        assert!(validate_base_url("ftp://x/v4").is_err());
        assert!(validate_base_url("api.aplaut.io/v4").is_err());
    }

    #[test]
    fn profile_names_are_restricted() {
        assert!(validate_profile_name("prod-1_a").is_ok());
        assert!(validate_profile_name("../etc").is_err());
        assert!(validate_profile_name("").is_err());
    }
}
