mod support;

use std::cell::RefCell;
use std::fs;
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Duration;

use aplaut_cli::auth::EnvSnapshot;
use aplaut_cli::commands::profile::{edit_config, EditOutcome, MIN_EDITOR_SESSION};
use aplaut_cli::config::Paths;
use aplaut_cli::term::Reporter;
use support::{aplaut, TempDir};

fn config_text(home: &Path) -> String {
    fs::read_to_string(home.join("config/aplaut/config.toml")).unwrap_or_default()
}

/// Только TOML без заголовка-справки: в комментариях упомянуты все опции.
fn config_body(home: &Path) -> String {
    config_text(home)
        .lines()
        .filter(|l| !l.starts_with('#'))
        .collect::<Vec<_>>()
        .join("\n")
}

fn login(home: &Path, profile: &str, token: &str) {
    let out = aplaut(
        home,
        &["auth", "login", "--token-stdin", "--profile", profile],
        &[],
        token,
    );
    assert_eq!(out.code, 0, "{}", out.stderr);
}

#[test]
fn set_creates_updates_and_resets_base_url() {
    let home = TempDir::new("profile-set");
    let out = aplaut(
        home.path(),
        &[
            "profile",
            "set",
            "staging",
            "--base-url",
            "https://api.staging.example/v4",
        ],
        &[],
        "",
    );
    assert_eq!(out.code, 0, "{}", out.stderr);
    assert!(config_text(home.path()).contains("https://api.staging.example/v4"));
    let reset = aplaut(
        home.path(),
        &["profile", "set", "staging", "--base-url", "none"],
        &[],
        "",
    );
    assert_eq!(reset.code, 0, "{}", reset.stderr);
    let text = config_body(home.path());
    assert!(
        text.contains("[profiles.staging]") && !text.contains("base_url"),
        "{text}"
    );
    let insecure = aplaut(
        home.path(),
        &[
            "profile",
            "set",
            "staging",
            "--base-url",
            "http://example.com/v4",
        ],
        &[],
        "",
    );
    assert_eq!(insecure.code, 2);
    assert_eq!(insecure.error_json()["error"]["code"], "insecure_base_url");
}

#[test]
fn list_is_a_tree_with_descriptions_marks_active_and_never_prints_tokens() {
    let home = TempDir::new("profile-list");
    login(home.path(), "ci", "tok-ci-secret");
    let set = aplaut(
        home.path(),
        &[
            "profile",
            "set",
            "staging",
            "--base-url",
            "https://api.staging.example/v4",
            "--description",
            "Стенд для тестов",
        ],
        &[],
        "",
    );
    assert_eq!(set.code, 0, "{}", set.stderr);
    let out = aplaut(
        home.path(),
        &["profile", "list"],
        &[("APLAUT_PROFILE", "staging")],
        "",
    );
    assert_eq!(out.code, 0, "{}", out.stderr);
    assert_eq!(
        out.stdout,
        "  ci\n    - base_url: https://api.aplaut.io/v4 (по умолчанию)\n    - токен: есть\n\
         * staging — Стенд для тестов\n    - base_url: https://api.staging.example/v4\n    - токен: нет\n"
    );
    let json = aplaut(
        home.path(),
        &["profile", "list", "--format", "json"],
        &[],
        "",
    );
    let v: serde_json::Value = serde_json::from_str(&json.stdout).unwrap();
    assert_eq!(v[0]["name"], "ci");
    assert_eq!(
        (
            v[0]["has_token"].as_bool(),
            v[0]["base_url_default"].as_bool()
        ),
        (Some(true), Some(true))
    );
    assert!(v[0]["description"].is_null());
    assert_eq!(v[1]["description"], "Стенд для тестов");
    assert_eq!(v[1]["base_url"], "https://api.staging.example/v4");
    for o in [&out, &json] {
        assert!(!o.stdout.contains("tok-ci-secret") && !o.stderr.contains("tok-ci-secret"));
    }
}

#[test]
fn set_description_can_be_cleared_and_must_be_one_line() {
    let home = TempDir::new("profile-desc");
    assert_eq!(
        aplaut(
            home.path(),
            &[
                "profile",
                "set",
                "prod",
                "--description",
                "Основной аккаунт"
            ],
            &[],
            ""
        )
        .code,
        0
    );
    assert!(config_text(home.path()).contains("description = \"Основной аккаунт\""));
    assert_eq!(
        aplaut(
            home.path(),
            &["profile", "set", "prod", "--description", "none"],
            &[],
            ""
        )
        .code,
        0
    );
    assert!(!config_body(home.path()).contains("description ="));
    let multiline = aplaut(
        home.path(),
        &["profile", "set", "prod", "--description", "a\nb"],
        &[],
        "",
    );
    assert_eq!(multiline.code, 2);
    assert_eq!(multiline.error_json()["error"]["field"], "description");
    let nothing = aplaut(home.path(), &["profile", "set", "prod"], &[], "");
    assert_eq!(nothing.code, 2);
    assert_eq!(nothing.error_json()["error"]["code"], "nothing_to_set");
}

#[test]
fn config_written_by_aplaut_keeps_the_reference_header() {
    let home = TempDir::new("profile-header");
    login(home.path(), "ci", "tok");
    assert_eq!(
        aplaut(
            home.path(),
            &["profile", "set", "ci", "--base-url", "https://x.example/v4"],
            &[],
            ""
        )
        .code,
        0
    );
    let text = config_text(home.path());
    assert!(text.starts_with("# Профили aplaut"), "{text}");
    assert_eq!(
        text.matches("# Профили aplaut").count(),
        1,
        "заголовок не дублируется: {text}"
    );
    assert!(
        text.contains("#   - description") && text.contains("#   - base_url"),
        "{text}"
    );
    assert!(
        text.contains("[profiles.ci]\nbase_url = \"https://x.example/v4\""),
        "{text}"
    );
}

#[test]
fn get_shows_one_profile_and_unknown_is_not_found() {
    let home = TempDir::new("profile-get");
    login(home.path(), "ci", "tok");
    let out = aplaut(
        home.path(),
        &["profile", "get", "ci", "--format", "json"],
        &[],
        "",
    );
    assert_eq!(out.code, 0, "{}", out.stderr);
    let v: serde_json::Value = serde_json::from_str(&out.stdout).unwrap();
    assert_eq!(
        (v["name"].as_str(), v["has_token"].as_bool()),
        (Some("ci"), Some(true))
    );
    let missing = aplaut(home.path(), &["profile", "get", "ghost"], &[], "");
    assert_eq!(missing.code, 5);
    let err = missing.error_json();
    assert_eq!(err["error"]["code"], "profile_not_found");
    assert!(err["error"]["hint"].as_str().unwrap().contains("ci"));
}

#[test]
fn delete_needs_force_without_terminal_and_removes_token_and_config() {
    let home = TempDir::new("profile-delete");
    login(home.path(), "ci", "tok-ci");
    assert_eq!(
        aplaut(
            home.path(),
            &["profile", "set", "ci", "--base-url", "https://x.example/v4"],
            &[],
            ""
        )
        .code,
        0
    );
    let refused = aplaut(home.path(), &["profile", "delete", "ci"], &[], "");
    assert_eq!(refused.code, 8);
    assert_eq!(
        refused.error_json()["error"]["code"],
        "confirmation_required"
    );
    let deleted = aplaut(
        home.path(),
        &["profile", "delete", "ci", "--force"],
        &[],
        "",
    );
    assert_eq!(deleted.code, 0, "{}", deleted.stderr);
    assert!(!config_body(home.path()).contains("ci"));
    let creds = fs::read_to_string(home.path().join("config/aplaut/credentials")).unwrap();
    assert!(!creds.contains("tok-ci"));
    assert_eq!(
        aplaut(home.path(), &["profile", "delete", "ci", "-f"], &[], "").code,
        5
    );
}

#[test]
fn edit_without_terminal_points_to_profile_set() {
    let home = TempDir::new("profile-edit-notty");
    let out = aplaut(home.path(), &["profile", "edit"], &[("EDITOR", "true")], "");
    assert_eq!(out.code, 2);
    let err = out.error_json();
    assert_eq!(err["error"]["code"], "terminal_required");
    assert!(err["error"]["hint"]
        .as_str()
        .unwrap()
        .contains("aplaut profile set"));
}

#[test]
fn unknown_config_key_is_rejected_instead_of_silently_using_prod() {
    let home = TempDir::new("profile-typo");
    let dir = home.path().join("config/aplaut");
    fs::create_dir_all(&dir).unwrap();
    fs::write(
        dir.join("config.toml"),
        "[profiles.staging]\nbase-url = \"https://api.staging.example/v4\"\n",
    )
    .unwrap();
    let out = aplaut(home.path(), &["profile", "list"], &[], "");
    assert_eq!(out.code, 1);
    let err = out.error_json();
    assert_eq!(err["error"]["code"], "config_invalid");
    assert!(
        err["error"]["message"]
            .as_str()
            .unwrap()
            .contains("base-url"),
        "{err}"
    );
}

/// Редактор для тестов: shell-скрипт; `$1` — редактируемый файл, `$0` — сам скрипт.
fn editor_script(dir: &Path, body: &str) -> String {
    named_editor(dir, "editor.sh", body)
}

/// Скрипт-редактор с заданным именем: aplaut узнаёт GUI-редакторы по имени программы.
fn named_editor(dir: &Path, name: &str, body: &str) -> String {
    let script = dir.join(name);
    fs::write(&script, format!("#!/bin/sh\n{body}\n")).unwrap();
    fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
    script.display().to_string()
}

fn append(content: &str) -> String {
    format!("cat >> \"$1\" <<'EOF'\n{content}\nEOF")
}

fn paths(home: &Path) -> Paths {
    Paths::resolve(&EnvSnapshot {
        xdg_config_home: Some(home.join("config")),
        ..EnvSnapshot::default()
    })
    .unwrap()
}

fn leftovers(p: &Paths) -> Vec<String> {
    fs::read_dir(&p.dir)
        .map(|d| {
            d.filter_map(|e| e.ok())
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .filter(|n| n.contains(".edit"))
                .collect()
        })
        .unwrap_or_default()
}

fn never(_: &str) -> bool {
    false
}

fn silent() -> Reporter {
    Reporter::with_writer(false, false, false, false, Box::new(std::io::sink()))
}

/// stderr для тестов: к каждой записи — был ли уже запущен редактор (он создаёт файл-метку).
struct Timeline {
    marker: PathBuf,
    writes: Rc<RefCell<Vec<(String, bool)>>>,
}

impl Write for Timeline {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let text = String::from_utf8_lossy(buf).into_owned();
        self.writes.borrow_mut().push((text, self.marker.exists()));
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// Как git: GUI-редактор с ожиданием держит терминал, пока открыта вкладка, — без строки
/// «жду» кажется, что aplaut завис. Строка появляется до запуска редактора и стирается после.
#[test]
fn edit_says_it_waits_for_the_editor_and_then_clears_the_line() {
    let home = TempDir::new("edit-hint");
    let p = paths(home.path());
    let marker = home.path().join("editor-ran");
    let writes = Rc::new(RefCell::new(Vec::new()));
    let stderr = Timeline {
        marker: marker.clone(),
        writes: writes.clone(),
    };
    let tty = Reporter::with_writer(false, false, true, false, Box::new(stderr));
    let body = format!("touch '{}'\n{}", marker.display(), append("[profiles.a]"));
    let editor = editor_script(home.path(), &body);
    edit_config(&p, &editor, &tty, &mut never).unwrap();
    assert_eq!(
        *writes.borrow(),
        vec![
            (
                "\rЖду, пока вы закроете файл в редакторе…\x1b[K".to_string(),
                false
            ),
            ("\r\x1b[K".to_string(), true),
        ]
    );
}

#[test]
fn edit_without_changes_writes_nothing() {
    let home = TempDir::new("edit-unchanged");
    let p = paths(home.path());
    // Человек посмотрел и закрыл — дольше, чем возвращается GUI-редактор без ожидания.
    let looked = MIN_EDITOR_SESSION + Duration::from_millis(200);
    let editor = editor_script(home.path(), &format!("sleep {}", looked.as_secs_f64()));
    assert_eq!(
        edit_config(&p, &editor, &silent(), &mut never).unwrap(),
        EditOutcome::Unchanged
    );
    assert!(!p.config.exists(), "шаблон без правок не сохраняется");
    assert!(leftovers(&p).is_empty());
}

#[test]
fn edit_reports_editor_that_returned_immediately_without_changes() {
    let home = TempDir::new("edit-instant");
    let p = paths(home.path());
    let err = edit_config(&p, "true", &silent(), &mut never).unwrap_err();
    assert_eq!(err.code, "editor_returned_immediately");
    assert!(err.message.contains("«true»"), "{}", err.message);
    let hint = err.hint.unwrap();
    // VISUAL главнее EDITOR: совет «EDITOR=vi» не помог бы при заданном VISUAL.
    assert!(
        hint.contains("--wait") && hint.contains("VISUAL=vi aplaut profile edit"),
        "{hint}"
    );
    assert!(!p.config.exists());
    assert!(leftovers(&p).is_empty());
}

/// Редактор сохранил файл не в UTF-8 (например, в CP1251): прочесть правки нельзя, но и удалять
/// их нельзя — копия остаётся, путь к ней в подсказке.
#[test]
fn edit_keeps_the_copy_with_edits_it_cannot_read_back() {
    let home = TempDir::new("edit-unreadable");
    let p = paths(home.path());
    let editor = editor_script(home.path(), "printf '\\317\\360\\356\\344' >> \"$1\"");
    let err = edit_config(&p, &editor, &silent(), &mut never).unwrap_err();
    assert_eq!(err.code, "io_error");
    let copy = leftovers(&p);
    assert_eq!(copy.len(), 1, "копия с правками осталась");
    let hint = err.hint.unwrap_or_default();
    assert!(hint.contains(&copy[0]), "{hint}");
}

#[test]
fn edit_reopened_editor_that_returns_immediately_is_reported_not_asked_again() {
    let home = TempDir::new("edit-instant-reopen");
    let p = paths(home.path());
    // Первый сеанс ломает файл, повторный возвращается сразу, ничего не тронув.
    let body = format!(
        "[ -f \"$0.done\" ] && exit 0\ntouch \"$0.done\"\n{}",
        append("[profiles.x")
    );
    let editor = editor_script(home.path(), &body);
    let mut asked = 0;
    let mut ask = |_: &str| {
        asked += 1;
        asked < 2
    };
    let err = edit_config(&p, &editor, &silent(), &mut ask).unwrap_err();
    assert_eq!(err.code, "editor_returned_immediately");
    assert_eq!(asked, 1, "после мгновенного возврата вопрос не повторяется");
    assert!(leftovers(&p).is_empty());
}

#[test]
fn edit_saves_valid_changes_with_summary() {
    let home = TempDir::new("edit-save");
    let p = paths(home.path());
    let editor = editor_script(
        home.path(),
        &append("[profiles.staging]\nbase_url = \"https://api.staging.example/v4\""),
    );
    let outcome = edit_config(&p, &editor, &silent(), &mut never).unwrap();
    assert_eq!(
        outcome,
        EditOutcome::Saved {
            profiles: 1,
            changes: vec!["staging: добавлен, base_url https://api.staging.example/v4".into()]
        }
    );
    let text = fs::read_to_string(&p.config).unwrap();
    assert!(
        text.contains("# Профили aplaut") && text.contains("[profiles.staging]"),
        "{text}"
    );
    assert_eq!(
        fs::metadata(&p.config).unwrap().permissions().mode() & 0o777,
        0o600
    );
    assert!(leftovers(&p).is_empty());
}

#[test]
fn edit_invalid_file_offers_reopen_and_keeps_original_when_declined() {
    let home = TempDir::new("edit-invalid");
    let p = paths(home.path());
    fs::create_dir_all(&p.dir).unwrap();
    let original = "[profiles.prod]\nbase_url = \"https://api.aplaut.io/v4\"\n";
    fs::write(&p.config, original).unwrap();
    let editor = editor_script(
        home.path(),
        &format!(
            "echo run >> \"$0.runs\"\n{}",
            append("[profiles.x\nbase_url = 1")
        ),
    );
    let mut asked = Vec::new();
    let mut ask = |q: &str| {
        asked.push(q.to_string());
        asked.len() < 2
    };
    let err = edit_config(&p, &editor, &silent(), &mut ask).unwrap_err();
    assert_eq!(err.code, "config_invalid");
    assert!(err.message.contains("строка"), "{}", err.message);
    assert_eq!(asked.len(), 2, "после каждой неудачной проверки — вопрос");
    assert_eq!(
        fs::read_to_string(format!("{editor}.runs"))
            .unwrap()
            .lines()
            .count(),
        2,
        "редактор открыт повторно"
    );
    assert_eq!(
        fs::read_to_string(&p.config).unwrap(),
        original,
        "оригинал не тронут"
    );
    assert!(leftovers(&p).is_empty());
}

#[test]
fn edit_reopen_lets_user_fix_the_mistake() {
    let home = TempDir::new("edit-fix");
    let p = paths(home.path());
    // Первый запуск ломает файл, второй — чинит.
    let body = format!(
        "if [ -f \"$0.done\" ]; then\n  printf '[profiles.ci]\\n' > \"$1\"\nelse\n  touch \"$0.done\"\n  {}\nfi",
        append("[profiles.ci]\nbase_url = \"http://example.com/v4\"")
    );
    let editor = editor_script(home.path(), &body);
    let mut always = |_: &str| true;
    let outcome = edit_config(&p, &editor, &silent(), &mut always).unwrap();
    assert_eq!(
        outcome,
        EditOutcome::Saved {
            profiles: 1,
            changes: vec!["ci: добавлен, base_url прод по умолчанию".into()]
        }
    );
}

#[test]
fn edit_does_not_overwrite_a_concurrent_change_and_keeps_the_copy() {
    let home = TempDir::new("edit-race");
    let p = paths(home.path());
    fs::create_dir_all(&p.dir).unwrap();
    fs::write(&p.config, "[profiles.a]\n").unwrap();
    let concurrent = p.config.display().to_string();
    let body = format!(
        "printf '[profiles.b]\\n' > '{concurrent}'\n{}",
        append("[profiles.c]")
    );
    let editor = editor_script(home.path(), &body);
    let err = edit_config(&p, &editor, &silent(), &mut never).unwrap_err();
    assert_eq!(err.code, "config_changed_during_edit");
    assert_eq!(
        fs::read_to_string(&p.config).unwrap(),
        "[profiles.b]\n",
        "параллельная запись сохранена"
    );
    let hint = err.hint.unwrap();
    let copy = leftovers(&p);
    assert_eq!(copy.len(), 1, "копия с правками осталась");
    assert!(hint.contains(&copy[0]), "{hint}");
}

#[test]
fn edit_summary_reports_changed_and_removed_profiles() {
    let home = TempDir::new("edit-summary");
    let p = paths(home.path());
    fs::create_dir_all(&p.dir).unwrap();
    fs::write(
        &p.config,
        "[profiles.a]\nbase_url = \"https://a.example/v4\"\n[profiles.b]\n",
    )
    .unwrap();
    let body = "printf '[profiles.a]\\nbase_url = \"https://a2.example/v4\"\\n' > \"$1\"";
    let editor = editor_script(home.path(), body);
    let outcome = edit_config(&p, &editor, &silent(), &mut never).unwrap();
    assert_eq!(
        outcome,
        EditOutcome::Saved {
            profiles: 1,
            changes: vec![
                "a: base_url https://a.example/v4 → https://a2.example/v4".into(),
                "b: удалён из config.toml (токен, если был, остался — aplaut profile delete b)"
                    .into(),
            ]
        }
    );
}

/// Как VS Code: без `--wait` передаёт файл окну и сразу выходит — aplaut принял бы это за выход
/// без правок и удалил копию, а окно открыло бы уже пустой файл. С `--wait` ждёт, пока человек
/// поправит файл и закроет вкладку.
#[test]
fn edit_makes_gui_editor_wait_for_the_file_to_close() {
    let home = TempDir::new("edit-gui");
    let p = paths(home.path());
    let code = named_editor(
        home.path(),
        "code",
        &format!(
            "[ \"$1\" = --wait ] || exit 0\nshift\n{}",
            append("[profiles.gui]")
        ),
    );
    let outcome = edit_config(&p, &code, &silent(), &mut never).unwrap();
    assert_eq!(
        outcome,
        EditOutcome::Saved {
            profiles: 1,
            changes: vec!["gui: добавлен, base_url прод по умолчанию".into()]
        }
    );
}

#[test]
fn edit_propagates_editor_failure_and_cleans_up() {
    let home = TempDir::new("edit-fail");
    let p = paths(home.path());
    let err = edit_config(&p, "false", &silent(), &mut never).unwrap_err();
    assert_eq!(err.code, "editor_failed");
    assert!(leftovers(&p).is_empty());
}
