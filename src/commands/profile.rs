//! Локальные профили: base URL и наличие токена. Сам токен задаёт только `auth login`
//! (скрытый ввод или stdin) — здесь он никогда не печатается и не редактируется.

use std::collections::BTreeSet;
use std::io::{self, BufRead, IsTerminal, Write};
use std::process::Command;
use std::time::{Duration, Instant};

use serde::Serialize;

use super::Ctx;
use crate::auth::{self, DEFAULT_BASE_URL};
use crate::cli::ProfileVerb;
use crate::config::{self, ConfigFile, CredentialsFile, Paths, ProfileConfig};
use crate::error::{CliError, Exit};
use crate::fsutil;
use crate::term::Reporter;

#[derive(Debug, Serialize)]
struct ProfileView {
    name: String,
    description: Option<String>,
    base_url: String,
    base_url_default: bool,
    has_token: bool,
    active: bool,
}

pub fn run(verb: ProfileVerb, ctx: &Ctx) -> Result<(), CliError> {
    match verb {
        ProfileVerb::List(args) => list(&args.format, ctx),
        ProfileVerb::Get { name, format } => get(&name, &format.format, ctx),
        ProfileVerb::Set { name, description } => set(&name, description.as_deref(), ctx),
        ProfileVerb::Delete { name, force } => delete(&name, force, ctx),
        ProfileVerb::Edit => edit(ctx),
    }
}

/// Быстрее человек файл не поправит: возврат без правок раньше этого срока — признак
/// GUI-редактора, который передал файл окну и не ждёт.
pub const MIN_EDITOR_SESSION: Duration = Duration::from_secs(1);

/// Итог `profile edit`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EditOutcome {
    Unchanged,
    Saved {
        profiles: usize,
        changes: Vec<String>,
    },
}

/// Правка `config.toml` через копию: оригинал заменяется атомарно и только после проверки,
/// а правка параллельного процесса (например, `auth login`) не затирается. `ask` — вопрос
/// «открыть снова?» после неудачной проверки; `false` — отказ, оригинал остаётся как был.
pub fn edit_config(
    paths: &Paths,
    editor: &str,
    reporter: &Reporter,
    ask: &mut dyn FnMut(&str) -> bool,
) -> Result<EditOutcome, CliError> {
    let io_err = |what: &str, e: &std::io::Error| {
        CliError::io(&format!("{what} {}", paths.config.display()), e)
    };
    let editor = &with_wait_flag(editor);
    fsutil::ensure_private_dir(&paths.dir).map_err(|e| io_err("каталог для", &e))?;
    let original = read_optional(&paths.config).map_err(|e| io_err("чтение", &e))?;
    let old_config = match &original {
        Some(text) => config::parse_config(text, &paths.config)?,
        None => ConfigFile::default(),
    };
    let start_text = original
        .clone()
        .unwrap_or_else(|| config::CONFIG_HEADER.to_string());
    // Расширение .toml — чтобы редактор включил подсветку.
    let copy = paths
        .dir
        .join(format!(".config.edit-{}.toml", std::process::id()));
    fsutil::write_atomic(&copy, start_text.as_bytes()).map_err(|e| io_err("копия", &e))?;
    let cleanup = |copy: &std::path::Path| {
        let _ = std::fs::remove_file(copy);
    };
    let mut before = start_text.clone();
    loop {
        let elapsed = match editor_session(editor, &copy, reporter) {
            Ok(elapsed) => elapsed,
            Err(err) => {
                cleanup(&copy);
                return Err(err);
            }
        };
        // Нечитаемую копию (например, сохранённую не в UTF-8) не удаляем: в ней правки человека.
        let edited = std::fs::read_to_string(&copy).map_err(|e| {
            CliError::io(&format!("чтение правок {}", copy.display()), &e).with_hint(format!(
                "config.toml не изменён; правки остались в {}: перенесите их и удалите копию",
                copy.display()
            ))
        })?;
        if edited == before && elapsed < MIN_EDITOR_SESSION {
            cleanup(&copy);
            return Err(returned_immediately(editor, elapsed));
        }
        if edited == start_text {
            cleanup(&copy);
            return Ok(EditOutcome::Unchanged);
        }
        let new_config = match validate(&edited, paths) {
            Ok(config) => config,
            Err(err) => {
                if ask(&format!("{}\nОткрыть снова? [Y/n] ", err.message)) {
                    before = edited;
                    continue;
                }
                cleanup(&copy);
                return Err(err.with_hint("файл не изменён; исправить: aplaut profile edit"));
            }
        };
        let current = read_optional(&paths.config).map_err(|e| io_err("чтение", &e))?;
        if current != original {
            return Err(CliError::general(
                "config_changed_during_edit",
                format!(
                    "{} изменился, пока был открыт редактор",
                    paths.config.display()
                ),
            )
            .with_hint(format!(
                "ваши правки сохранены в {}: перенесите их и удалите копию",
                copy.display()
            )));
        }
        fsutil::write_atomic(&paths.config, edited.as_bytes()).map_err(|e| io_err("запись", &e))?;
        cleanup(&copy);
        return Ok(EditOutcome::Saved {
            profiles: new_config.profiles.len(),
            changes: changes(&old_config, &new_config),
        });
    }
}

fn read_optional(path: &std::path::Path) -> std::io::Result<Option<String>> {
    match std::fs::read_to_string(path) {
        Ok(text) => Ok(Some(text)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}

/// GUI-редакторы, которые без флага передают файл окну и сразу возвращают управление: правка
/// ещё не началась, а aplaut уже прочёл бы копию. Поля: программы, флаг ожидания, его короткая форма.
const WAIT_FLAGS: &[(&[&str], &str, &str)] = &[
    (
        &[
            "code",
            "code-insiders",
            "codium",
            "cursor",
            "windsurf",
            "zed",
            "zeditor",
            "subl",
        ],
        "--wait",
        "-w",
    ),
    (&["gvim"], "--nofork", "-f"),
    (&["kate"], "--block", "-b"),
];

/// Команда редактора, которая ждёт закрытия файла: знакомым GUI-редакторам — флаг ожидания,
/// если его нет. Остальные команды — как есть.
fn with_wait_flag(editor: &str) -> String {
    let mut words = editor.split_whitespace();
    let program = words.next().unwrap_or_default();
    let name = program.rsplit('/').next().unwrap_or(program);
    match WAIT_FLAGS
        .iter()
        .find(|(names, _, _)| names.contains(&name))
    {
        Some((_, long, short)) if !words.any(|w| w == *long || w == *short) => {
            format!("{editor} {long}")
        }
        _ => editor.to_string(),
    }
}

/// Один сеанс редактора; возвращает его длительность. Как git: GUI-редактор с ожиданием держит
/// терминал, пока открыта вкладка, — без строки «жду» кажется, что aplaut завис.
fn editor_session(
    editor: &str,
    copy: &std::path::Path,
    reporter: &Reporter,
) -> Result<Duration, CliError> {
    reporter.progress("Жду, пока вы закроете файл в редакторе…");
    let started = Instant::now();
    let status = run_editor(editor, copy);
    let elapsed = started.elapsed();
    reporter.clear_progress();
    status.map(|()| elapsed)
}

/// Мгновенный возврат без правок — не решение человека: копию сейчас удалят, и окно
/// GUI-редактора откроет пустой файл.
fn returned_immediately(editor: &str, elapsed: Duration) -> CliError {
    CliError::general(
        "editor_returned_immediately",
        format!(
            "редактор «{editor}» вернул управление через {} мс, и файл не изменился. Если он \
             открыл файл в отдельном окне, то не ждёт, пока вы закончите: правки в том окне \
             в config.toml не попадут",
            elapsed.as_millis()
        ),
    )
    .with_hint(
        "config.toml не изменён. Задайте в VISUAL (он главнее EDITOR) команду, которая ждёт \
         закрытия файла — у GUI-редакторов обычно флаг вроде --wait, — или терминальный \
         редактор: VISUAL=vi aplaut profile edit",
    )
}

/// Через `sh -c`, как git: редактор может быть с аргументами — EDITOR="code --wait".
fn run_editor(editor: &str, file: &std::path::Path) -> Result<(), CliError> {
    let status = Command::new("sh")
        .arg("-c")
        .arg(format!("{editor} \"$1\""))
        .arg("sh")
        .arg(file)
        .status()
        .map_err(|e| CliError::io("запуск редактора", &e))?;
    if status.success() {
        return Ok(());
    }
    Err(CliError::general(
        "editor_failed",
        format!("редактор «{editor}» завершился с ошибкой ({status})"),
    )
    .with_hint("проверьте $VISUAL / $EDITOR"))
}

fn validate(text: &str, paths: &Paths) -> Result<ConfigFile, CliError> {
    let config = config::parse_config(text, &paths.config)?;
    for (name, profile) in &config.profiles {
        auth::validate_profile_name(name)?;
        if let Some(description) = &profile.description {
            validate_description(description)?;
        }
        if let Some(url) = &profile.base_url {
            auth::validate_base_url(url)?;
        }
    }
    Ok(config)
}

/// Короткая сводка для человека: что добавлено, изменено, удалено.
fn changes(old: &ConfigFile, new: &ConfigFile) -> Vec<String> {
    let url = |p: &ProfileConfig| {
        p.base_url
            .clone()
            .unwrap_or_else(|| "прод по умолчанию".to_string())
    };
    let desc = |p: &ProfileConfig| {
        p.description
            .as_deref()
            .map(|d| format!("«{d}»"))
            .unwrap_or_else(|| "нет".to_string())
    };
    let names: BTreeSet<&String> = old.profiles.keys().chain(new.profiles.keys()).collect();
    let mut out = Vec::new();
    for name in names {
        match (old.profiles.get(name), new.profiles.get(name)) {
            (None, Some(p)) => {
                let mut line = format!("{name}: добавлен, base_url {}", url(p));
                if p.description.is_some() {
                    line.push_str(&format!(", description {}", desc(p)));
                }
                out.push(line);
            }
            (Some(_), None) => out.push(format!(
                "{name}: удалён из config.toml (токен, если был, остался — aplaut profile delete {name})"
            )),
            (Some(a), Some(b)) => {
                if a.base_url != b.base_url {
                    out.push(format!("{name}: base_url {} → {}", url(a), url(b)));
                }
                if a.description != b.description {
                    out.push(format!("{name}: description {} → {}", desc(a), desc(b)));
                }
            }
            (None, None) => {}
        }
    }
    out
}

fn list(format: &str, ctx: &Ctx) -> Result<(), CliError> {
    let (_, cfg, creds) = load(ctx)?;
    let views = views(&cfg, &creds, &active(ctx)?);
    if format == "json" {
        return print_json(&views);
    }
    if views.is_empty() {
        ctx.reporter.info(
            "Профилей нет: aplaut auth login --profile NAME или aplaut profile set NAME --base-url URL",
        );
        return Ok(());
    }
    let text: String = views.iter().map(render).collect();
    io::stdout()
        .write_all(text.as_bytes())
        .map_err(crate::output::write_error)
}

fn get(name: &str, format: &str, ctx: &Ctx) -> Result<(), CliError> {
    let (_, cfg, creds) = load(ctx)?;
    let views = views(&cfg, &creds, &active(ctx)?);
    let Some(view) = views.iter().find(|v| v.name == name) else {
        return Err(not_found(name, &views));
    };
    if format == "json" {
        return print_json(view);
    }
    let text = render(view);
    io::stdout()
        .write_all(text.as_bytes())
        .map_err(crate::output::write_error)
}

/// Upsert, как `set` во всей грамматике: несуществующий профиль создаётся.
/// `none` убирает значение (clig: специальное слово вместо пустой строки).
fn set(name: &str, description: Option<&str>, ctx: &Ctx) -> Result<(), CliError> {
    auth::validate_profile_name(name)?;
    let base_url = ctx.global.base_url.as_deref();
    if base_url.is_none() && description.is_none() {
        return Err(CliError::usage(
            "nothing_to_set",
            "укажите, что изменить: --base-url URL|none и/или --description TEXT|none",
        ));
    }
    let base_url = match base_url {
        None => None,
        Some("none") => Some(None),
        Some(url) => Some(Some(auth::validate_base_url(url)?)),
    };
    let description = match description {
        None => None,
        Some("none") => Some(None),
        Some(text) => Some(Some(validate_description(text)?)),
    };
    let paths = Paths::resolve(&ctx.env)?;
    let mut cfg = config::load_config(&paths)?;
    let entry = cfg.profiles.entry(name.to_string()).or_default();
    if let Some(url) = base_url {
        entry.base_url = url;
    }
    if let Some(text) = description {
        entry.description = text;
    }
    let summary = format!(
        "Профиль «{name}»: base_url — {}, описание — {}",
        entry.base_url.as_deref().unwrap_or("прод по умолчанию"),
        entry.description.as_deref().unwrap_or("нет")
    );
    config::save_config(&paths, &cfg)?;
    ctx.reporter.info(&summary);
    Ok(())
}

/// Описание — одна строка: оно выводится в `profile list` после имени.
fn validate_description(text: &str) -> Result<String, CliError> {
    let text = text.trim();
    if text.chars().any(char::is_control) {
        return Err(CliError::usage(
            "invalid_description",
            "описание профиля должно быть одной строкой без управляющих символов",
        )
        .with_field("description"));
    }
    Ok(text.to_string())
}

fn delete(name: &str, force: bool, ctx: &Ctx) -> Result<(), CliError> {
    auth::validate_profile_name(name)?;
    let (paths, mut cfg, mut creds) = load(ctx)?;
    if !cfg.profiles.contains_key(name) && !creds.profiles.contains_key(name) {
        return Err(not_found(name, &views(&cfg, &creds, "")));
    }
    let with_token = creds.profiles.contains_key(name);
    if !force {
        let question = format!(
            "Удалить профиль «{name}»{}? [y/N] ",
            if with_token {
                " вместе с токеном (восстановить его нельзя, только выпустить новый в ЛК)"
            } else {
                ""
            }
        );
        if !confirm(ctx, &question)? {
            ctx.reporter.info("Удаление отменено.");
            return Ok(());
        }
    }
    cfg.profiles.remove(name);
    creds.profiles.remove(name);
    config::save_config(&paths, &cfg)?;
    config::save_credentials(&paths, &creds)?;
    ctx.reporter.info(&format!("Профиль «{name}» удалён."));
    Ok(())
}

fn edit(ctx: &Ctx) -> Result<(), CliError> {
    if ctx.global.no_input || !io::stdin().is_terminal() {
        return Err(CliError::usage(
            "terminal_required",
            "profile edit открывает редактор и работает только в терминале",
        )
        .with_hint("в скриптах используйте aplaut profile set NAME --base-url URL"));
    }
    let paths = Paths::resolve(&ctx.env)?;
    let editor = ctx.env.editor.clone().unwrap_or_else(|| "vi".to_string());
    let mut ask = |question: &str| ask_yes(question, true);
    match edit_config(&paths, &editor, &ctx.reporter, &mut ask)? {
        EditOutcome::Unchanged => ctx.reporter.info("Без изменений."),
        EditOutcome::Saved { profiles, changes } => {
            let summary = if changes.is_empty() {
                "изменены только комментарии или форматирование".to_string()
            } else {
                changes.join("; ")
            };
            ctx.reporter.info(&format!(
                "{} сохранён, профилей {profiles}: {summary}",
                paths.config.display()
            ));
        }
    }
    Ok(())
}

/// clig: в терминале — вопрос, без терминала — только явный `--force` (у агента нет TTY).
fn confirm(ctx: &Ctx, question: &str) -> Result<bool, CliError> {
    let stdin = io::stdin();
    if ctx.global.no_input || !stdin.is_terminal() {
        return Err(CliError::new(
            Exit::Policy,
            "confirmation_required",
            "удаление профиля требует подтверждения",
        )
        .with_hint("повторите с --force"));
    }
    Ok(ask_yes(question, false))
}

/// Вопрос да/нет в терминале; пустой ответ — `default`. Ошибка чтения — «нет».
fn ask_yes(question: &str, default: bool) -> bool {
    eprint!("{question}");
    let mut answer = String::new();
    if io::stdin().lock().read_line(&mut answer).is_err() {
        return false;
    }
    match answer.trim().to_lowercase().as_str() {
        "" => default,
        "y" | "yes" | "д" | "да" => true,
        _ => false,
    }
}

fn load(ctx: &Ctx) -> Result<(Paths, ConfigFile, CredentialsFile), CliError> {
    let paths = Paths::resolve(&ctx.env)?;
    let cfg = config::load_config(&paths)?;
    let creds = config::load_credentials(&paths, &ctx.reporter)?;
    Ok((paths, cfg, creds))
}

fn active(ctx: &Ctx) -> Result<String, CliError> {
    auth::active_profile(ctx.global.profile.as_deref(), &ctx.env)
}

/// Профиль может быть только в config.toml (base URL без токена) или только в credentials.
fn views(cfg: &ConfigFile, creds: &CredentialsFile, active: &str) -> Vec<ProfileView> {
    let names: BTreeSet<&String> = cfg.profiles.keys().chain(creds.profiles.keys()).collect();
    names
        .into_iter()
        .map(|name| {
            let base_url = cfg.profiles.get(name).and_then(|p| p.base_url.clone());
            ProfileView {
                name: name.clone(),
                description: cfg.profiles.get(name).and_then(|p| p.description.clone()),
                base_url_default: base_url.is_none(),
                base_url: base_url.unwrap_or_else(|| DEFAULT_BASE_URL.to_string()),
                has_token: creds.profiles.contains_key(name),
                active: name == active,
            }
        })
        .collect()
}

/// `* имя — описание`, под ним опции списком; `*` — активный профиль.
fn render(view: &ProfileView) -> String {
    let mut text = format!("{} {}", if view.active { "*" } else { " " }, view.name);
    if let Some(description) = &view.description {
        text.push_str(&format!(" — {description}"));
    }
    text.push_str(&format!("\n    - base_url: {}\n", base_url_text(view)));
    text.push_str(&format!(
        "    - токен: {}\n",
        if view.has_token { "есть" } else { "нет" }
    ));
    text
}

fn base_url_text(view: &ProfileView) -> String {
    if view.base_url_default {
        format!("{} (по умолчанию)", view.base_url)
    } else {
        view.base_url.clone()
    }
}

fn not_found(name: &str, views: &[ProfileView]) -> CliError {
    let known: Vec<&str> = views.iter().map(|v| v.name.as_str()).collect();
    let hint = if known.is_empty() {
        "профилей нет: aplaut auth login --profile NAME".to_string()
    } else {
        format!("есть профили: {}", known.join(", "))
    };
    CliError::new(
        Exit::NotFound,
        "profile_not_found",
        format!("профиль «{name}» не найден"),
    )
    .with_hint(hint)
}

fn print_json<T: Serialize + ?Sized>(value: &T) -> Result<(), CliError> {
    let mut out = io::stdout().lock();
    serde_json::to_writer(&mut out, value).map_err(|e| crate::output::write_error(e.into()))?;
    writeln!(out).map_err(crate::output::write_error)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gui_editors_get_their_wait_flag() {
        assert_eq!(with_wait_flag("code"), "code --wait");
        assert_eq!(
            with_wait_flag("/usr/bin/code -n"),
            "/usr/bin/code -n --wait"
        );
        assert_eq!(with_wait_flag("subl"), "subl --wait");
        assert_eq!(with_wait_flag("zeditor"), "zeditor --wait");
        assert_eq!(with_wait_flag("gvim"), "gvim --nofork");
        assert_eq!(with_wait_flag("kate"), "kate --block");
    }

    #[test]
    fn editor_that_already_waits_or_is_unknown_is_left_alone() {
        for editor in [
            "code --wait",
            "code -w",
            "gvim -f",
            "kate -b",
            "vim",
            "nano -w",
            "codex",
            "",
        ] {
            assert_eq!(with_wait_flag(editor), editor);
        }
    }
}
