//! Вызов инструмента (спека mcp-server §3–§5): аргументы → argv дочернего процесса того же бинаря,
//! его stdin и куда идёт его stdout.

use std::ffi::OsString;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Seek};
use std::path::{Path, PathBuf};
use std::process::{ExitStatus, Stdio};

use serde_json::{Map, Value};
use tokio::io::AsyncWriteExt;

use super::tools::{Kind, ToolDef, DEFAULT_FORMAT, INLINE_DEFAULT_RECORDS, INLINE_MAX_RECORDS};
use crate::envelope;
use crate::error::CliError;
use crate::spec;
use crate::state;

/// Куда идёт stdout дочернего процесса (§5).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Destination {
    /// Во второй блок ответа — не больше одной страницы.
    Inline,
    File {
        path: PathBuf,
        mode: FileMode,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileMode {
    /// Файла быть не должно — иначе `output_exists`.
    CreateNew,
    Truncate,
    /// Продолжение по стейту — как `>>` в CLI.
    Append,
}

/// Что запустить (§3): argv после бинаря, stdin, куда stdout.
#[derive(Debug, Clone, PartialEq)]
pub struct Plan {
    pub args: Vec<OsString>,
    pub stdin: Option<String>,
    pub destination: Destination,
}

/// Как запускать дочерние вызовы: бинарь, флаги сервера (M6), каталог для относительных путей.
pub struct Runner {
    pub exe: PathBuf,
    pub forwarded: Vec<OsString>,
    pub cwd: PathBuf,
}

impl Runner {
    pub fn new(forwarded: Vec<OsString>) -> Runner {
        Runner {
            exe: self_exe(),
            forwarded,
            cwd: std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
        }
    }
}

/// На Linux — `/proc/self/exe`: после `self update` дочерние вызовы — той же версии, что и схемы
/// инструментов (M14).
fn self_exe() -> PathBuf {
    let proc_exe = Path::new("/proc/self/exe");
    if cfg!(target_os = "linux") && proc_exe.exists() {
        return proc_exe.to_path_buf();
    }
    std::env::current_exe().unwrap_or_else(|_| PathBuf::from("aplaut"))
}

/// Ответ инструмента (§4): конверт, данные инлайн, ошибка ли.
#[derive(Debug, Clone, PartialEq)]
pub struct Reply {
    pub envelope: String,
    pub data: Option<String>,
    pub is_error: bool,
}

/// Вызов целиком: план, дочерний процесс, ответ. Ошибка до запуска — конверт от сервера (§4).
pub async fn call(runner: &Runner, tool: &ToolDef, args: &Map<String, Value>) -> Reply {
    let dry_run = matches!(args.get("dry_run"), Some(Value::Bool(true)));
    let result = match plan(tool, args, &runner.forwarded, &runner.cwd) {
        Ok(plan) => execute(runner, tool.kind, plan).await,
        Err(err) => Err(err),
    };
    result.unwrap_or_else(|err| Reply {
        envelope: envelope::failure(&err, &tool.command_name(), dry_run, &[]),
        data: None,
        is_error: true,
    })
}

async fn execute(runner: &Runner, kind: Kind, plan: Plan) -> Result<Reply, CliError> {
    let Plan {
        args,
        stdin,
        destination,
    } = plan;
    let mut command = tokio::process::Command::new(&runner.exe);
    command
        .args(&args)
        .stdin(if stdin.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stderr(Stdio::piped())
        // Отмена вызова клиентом и выход сервера убивают дочерний процесс (§3).
        .kill_on_drop(true);
    let file = match &destination {
        Destination::Inline => {
            command.stdout(Stdio::piped());
            None
        }
        Destination::File { path, mode } => {
            let file = OutputFile::open(path, *mode)?;
            command.stdout(file.stdout()?);
            Some(file)
        }
    };
    let mut child = command
        .spawn()
        .map_err(|e| CliError::io("запуск дочернего aplaut", &e))?;
    if let (Some(input), Some(mut pipe)) = (stdin, child.stdin.take()) {
        pipe.write_all(input.as_bytes())
            .await
            .map_err(|e| CliError::io("запись в stdin дочернего aplaut", &e))?;
        // `pipe` закрывается здесь: дочерний процесс дочитывает `--data -` до EOF.
    }
    let out = child
        .wait_with_output()
        .await
        .map_err(|e| CliError::io("ожидание дочернего aplaut", &e))?;
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&out.stderr);
    let (line, ok, debug) =
        split_envelope(kind, &stdout, &stderr).ok_or_else(|| no_envelope(out.status, &stderr))?;
    for line in debug {
        eprintln!("{line}");
    }
    if let Some(file) = &file {
        file.settle(ok)?;
    }
    let envelope = match &file {
        Some(file) if ok => add_output_file(&line, &file.path),
        _ => line,
    };
    let data = (kind != Kind::Write && file.is_none() && !stdout.is_empty()).then_some(stdout);
    Ok(Reply {
        envelope,
        data,
        is_error: !ok,
    })
}

/// Файл `output_file` (§5). stdout дочернего процесса — дубликат `handle`: курсор у них общий,
/// поэтому после выхода процесса видно, сколько он записал.
struct OutputFile {
    path: PathBuf,
    mode: FileMode,
    handle: File,
}

impl OutputFile {
    fn open(path: &Path, mode: FileMode) -> Result<OutputFile, CliError> {
        Ok(OutputFile {
            path: path.to_path_buf(),
            mode,
            handle: open(path, mode)?,
        })
    }

    fn stdout(&self) -> Result<Stdio, CliError> {
        self.handle
            .try_clone()
            .map(Stdio::from)
            .map_err(|e| CliError::io("дескриптор output_file", &e))
    }

    /// Файл меняется, только если выгрузка в него писала: неудачный вызов не оставляет пустой файл
    /// (повтор упёрся бы в `output_exists`) и не стирает перезаписываемый. Перезапись отрезает
    /// хвост старого содержимого по тому, сколько записано.
    fn settle(&self, ok: bool) -> Result<(), CliError> {
        let written = (&self.handle)
            .stream_position()
            .map_err(|e| CliError::io("позиция в output_file", &e))?;
        let result = match self.mode {
            FileMode::CreateNew if written == 0 && !ok => fs::remove_file(&self.path),
            FileMode::Truncate if written > 0 || ok => self.handle.set_len(written),
            _ => Ok(()),
        };
        result.map_err(|e| CliError::io(&format!("output_file {}", self.path.display()), &e))
    }
}

/// `CreateNew` атомарно отказывает, если файл уже есть; `Truncate` не обрезает сразу — это делает
/// `OutputFile::settle`, когда видно, писала ли выгрузка.
fn open(path: &Path, mode: FileMode) -> Result<File, CliError> {
    let mut options = OpenOptions::new();
    match mode {
        FileMode::CreateNew => options.write(true).create_new(true),
        FileMode::Truncate => options.write(true).create(true).truncate(false),
        FileMode::Append => options.append(true).create(true),
    };
    options.open(path).map_err(|err| {
        if err.kind() == io::ErrorKind::AlreadyExists {
            CliError::usage("output_exists", format!("файл {} уже есть", path.display()))
                .with_field("output_file")
                .with_hint("укажите другой output_file или overwrite: true; продолжить выгрузку в него — со state")
        } else {
            CliError::io(&format!("открытие {}", path.display()), &err)
        }
    })
}

/// Конверт — последняя строка stderr, если это конверт; у записи успех идёт в stdout (§4).
/// Возвращает строку конверта, `ok` и прочие строки stderr (`debug:`) — для лога сервера.
pub fn split_envelope<'a>(
    kind: Kind,
    stdout: &str,
    stderr: &'a str,
) -> Option<(String, bool, Vec<&'a str>)> {
    let mut lines: Vec<&str> = stderr.lines().filter(|l| !l.trim().is_empty()).collect();
    if let Some(ok) = lines.last().and_then(|line| envelope_ok(line)) {
        let line = lines.pop().expect("последняя строка есть").to_string();
        return Some((line, ok, lines));
    }
    if kind == Kind::Write {
        let line = stdout.lines().rev().find(|l| !l.trim().is_empty())?;
        let ok = envelope_ok(line)?;
        return Some((line.to_string(), ok, lines));
    }
    None
}

/// `ok` конверта, если строка — конверт (`ok` и `command` на месте).
fn envelope_ok(line: &str) -> Option<bool> {
    let value: Value = serde_json::from_str(line).ok()?;
    value.get("command")?;
    value.get("ok")?.as_bool()
}

/// В `result` успешного конверта — абсолютный путь файла (§5).
fn add_output_file(line: &str, path: &Path) -> String {
    let mut value: Value = serde_json::from_str(line).expect("конверт уже разобран");
    if let Some(result) = value.get_mut("result").and_then(Value::as_object_mut) {
        result.insert(
            "output_file".into(),
            Value::String(path.display().to_string()),
        );
    }
    value.to_string()
}

fn no_envelope(status: ExitStatus, stderr: &str) -> CliError {
    let lines: Vec<&str> = stderr.lines().collect();
    let tail = lines[lines.len().saturating_sub(20)..].join("\n");
    CliError::general(
        "internal",
        format!("дочерний aplaut завершился без конверта ({status}): {tail}"),
    )
    .with_hint("сообщите в поддержку: support@aplaut.com")
}

/// Проверка аргументов и argv: `<флаги сервера> <ресурс> <глагол> <флаги> --json --no-input [-- <id>]`.
pub fn plan(
    tool: &ToolDef,
    args: &Map<String, Value>,
    forwarded: &[OsString],
    cwd: &Path,
) -> Result<Plan, CliError> {
    check_known(tool, args)?;
    let mut argv = forwarded.to_vec();
    argv.extend(tool.command.iter().map(OsString::from));
    let (stdin, destination) = match tool.kind {
        Kind::Scroll => (None, scroll_flags(args, cwd, &mut argv)?),
        Kind::Get => {
            get_flags(args, &mut argv)?;
            (None, Destination::Inline)
        }
        Kind::Write => (
            Some(write_flags(tool, args, &mut argv)?),
            Destination::Inline,
        ),
    };
    argv.push("--json".into());
    argv.push("--no-input".into());
    if let Some(name) = &tool.positional {
        let id = string(args, name)?.ok_or_else(|| {
            CliError::usage("usage", format!("нет обязательного параметра {name}")).with_field(name)
        })?;
        argv.push("--".into());
        argv.push(id.into());
    }
    Ok(Plan {
        args: argv,
        stdin,
        destination,
    })
}

/// Политика `output_file` (§5); относительный путь — от рабочего каталога сервера.
pub fn destination(
    file: Option<String>,
    has_state: bool,
    overwrite: bool,
    cwd: &Path,
) -> Result<Destination, CliError> {
    let Some(file) = file else {
        if overwrite {
            return Err(
                CliError::usage("usage", "overwrite имеет смысл только с output_file")
                    .with_field("overwrite"),
            );
        }
        return Ok(Destination::Inline);
    };
    if file.trim().is_empty() {
        return Err(CliError::usage("usage", "output_file: пустой путь").with_field("output_file"));
    }
    let mode = match (has_state, overwrite) {
        (true, true) => {
            return Err(CliError::usage(
                "usage",
                "overwrite вместе со state потерял бы уже выгруженное",
            )
            .with_field("overwrite")
            .with_hint("со state файл дописывается; начать заново — новые state и output_file"))
        }
        (true, false) => FileMode::Append,
        (false, true) => FileMode::Truncate,
        (false, false) => FileMode::CreateNew,
    };
    Ok(Destination::File {
        path: cwd.join(file),
        mode,
    })
}

/// `additionalProperties: false` — опечатка в имени не пропускается молча (§2.1).
fn check_known(tool: &ToolDef, args: &Map<String, Value>) -> Result<(), CliError> {
    let properties = tool.properties();
    match args.keys().find(|key| !properties.contains_key(*key)) {
        None => Ok(()),
        Some(key) => {
            let known: Vec<&str> = properties.keys().map(String::as_str).collect();
            Err(CliError::usage(
                "usage",
                format!("{}: неизвестный параметр «{key}»", tool.name),
            )
            .with_field(key)
            .with_hint(format!("допустимые: {}", known.join(", "))))
        }
    }
}

/// `scroll` (§2.1, §5): инлайн — одна страница, иначе — файл.
fn scroll_flags(
    args: &Map<String, Value>,
    cwd: &Path,
    argv: &mut Vec<OsString>,
) -> Result<Destination, CliError> {
    if let Some(filter) = string(args, "filter")? {
        push_flag(argv, "filter", &filter);
    }
    if let Some(sort) = string(args, "sort")? {
        push_flag(argv, "sort", &sort);
    }
    if let Some(include) = list(args, "include")? {
        push_flag(argv, "include", &include);
    }
    format_flags(args, argv)?;
    let state = string(args, "state")?;
    if let Some(state) = &state {
        push_flag(argv, "state", state);
    }
    let destination = destination(
        string(args, "output_file")?,
        state.is_some(),
        boolean(args, "overwrite")?,
        cwd,
    )?;
    let max_records = max_records(count(args, "max_records")?, &destination)?;
    if let Some(n) = max_records {
        push_flag(argv, "max-records", &n.to_string());
    }
    // Размер страницы фиксируется при открытии обхода, стейт требует тот же: иначе порция инлайн,
    // а затем остальное в файл упали бы на `state_mismatch` по невидимому агенту `per_page`.
    let per_page = state
        .as_deref()
        .and_then(|state| saved_per_page(&cwd.join(state)))
        .or(max_records.map(|n| n.min(u64::from(spec::SCROLL_PER_PAGE_MAX))));
    if let Some(per_page) = per_page {
        push_flag(argv, "per-page", &per_page.to_string());
    }
    Ok(destination)
}

/// `per_page` из стейта; нет файла или он повреждён — решит дочерний CLI (`state_invalid`).
fn saved_per_page(path: &Path) -> Option<u64> {
    state::load(path)
        .ok()
        .flatten()
        .map(|saved| u64::from(saved.params.per_page))
}

/// Инлайн — 1–100 записей, по умолчанию 20 (M7); в файл — сколько угодно, без лимита — всё.
fn max_records(requested: Option<u64>, destination: &Destination) -> Result<Option<u64>, CliError> {
    let n = match (requested, destination) {
        (None, Destination::Inline) => return Ok(Some(INLINE_DEFAULT_RECORDS)),
        (None, Destination::File { .. }) => return Ok(None),
        (Some(n), _) => n,
    };
    if n == 0 {
        return Err(
            CliError::usage("invalid_max_records", "max_records должно быть больше 0")
                .with_field("max_records"),
        );
    }
    if n > INLINE_MAX_RECORDS && *destination == Destination::Inline {
        return Err(CliError::usage(
            "invalid_max_records",
            format!(
                "max_records {n}: без output_file — не больше {INLINE_MAX_RECORDS} записей за вызов"
            ),
        )
        .with_field("max_records")
        .with_hint("больше — с output_file; или листайте порциями со state"));
    }
    Ok(Some(n))
}

fn get_flags(args: &Map<String, Value>, argv: &mut Vec<OsString>) -> Result<(), CliError> {
    if let Some(include) = list(args, "include")? {
        push_flag(argv, "include", &include);
    }
    format_flags(args, argv)
}

fn format_flags(args: &Map<String, Value>, argv: &mut Vec<OsString>) -> Result<(), CliError> {
    let format = string(args, "format")?.unwrap_or_else(|| DEFAULT_FORMAT.to_string());
    push_flag(argv, "format", &format);
    if let Some(fields) = list(args, "fields")? {
        push_flag(argv, "fields", &fields);
    }
    Ok(())
}

/// Атрибуты — JSON-объектом в stdin (`--data -`, M10); проверяет их дочерний CLI (M11).
fn write_flags(
    tool: &ToolDef,
    args: &Map<String, Value>,
    argv: &mut Vec<OsString>,
) -> Result<String, CliError> {
    let data: Map<String, Value> = args
        .iter()
        .filter(|(key, _)| tool.attributes.contains(&key.as_str()))
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect();
    push_flag(argv, "data", "-");
    if boolean(args, "dry_run")? {
        argv.push("--dry-run".into());
    }
    Ok(Value::Object(data).to_string())
}

fn push_flag(argv: &mut Vec<OsString>, flag: &str, value: &str) {
    argv.push(format!("--{flag}={value}").into());
}

fn string(args: &Map<String, Value>, name: &str) -> Result<Option<String>, CliError> {
    match args.get(name) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => Ok(Some(s.clone())),
        Some(other) => Err(wrong_type(name, "строка", other)),
    }
}

fn boolean(args: &Map<String, Value>, name: &str) -> Result<bool, CliError> {
    match args.get(name) {
        None | Some(Value::Null) => Ok(false),
        Some(Value::Bool(b)) => Ok(*b),
        Some(other) => Err(wrong_type(name, "true или false", other)),
    }
}

fn count(args: &Map<String, Value>, name: &str) -> Result<Option<u64>, CliError> {
    match args.get(name) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => value
            .as_u64()
            .map(Some)
            .ok_or_else(|| wrong_type(name, "целое число", value)),
    }
}

/// Массив строк или строка через запятую — как пишут в CLI (модели шлют и так, и так).
fn list(args: &Map<String, Value>, name: &str) -> Result<Option<String>, CliError> {
    match args.get(name) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => Ok(Some(s.clone())),
        Some(Value::Array(items)) => items
            .iter()
            .map(|item| {
                item.as_str()
                    .ok_or_else(|| wrong_type(name, "массив строк", item))
            })
            .collect::<Result<Vec<_>, _>>()
            .map(|items| Some(items.join(","))),
        Some(other) => Err(wrong_type(name, "массив строк", other)),
    }
}

fn wrong_type(name: &str, expected: &str, got: &Value) -> CliError {
    CliError::usage(
        "usage",
        format!("параметр {name}: ожидается {expected}, передано {got}"),
    )
    .with_field(name)
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use serde_json::json;

    use super::*;
    use crate::mcp::tools::catalog;

    fn tool(name: &str) -> ToolDef {
        catalog().into_iter().find(|t| t.name == name).unwrap()
    }

    fn args(value: Value) -> Map<String, Value> {
        value.as_object().unwrap().clone()
    }

    fn planned(name: &str, value: Value) -> Result<Plan, CliError> {
        plan(&tool(name), &args(value), &[], Path::new("/work"))
    }

    fn argv(plan: &Plan) -> Vec<String> {
        plan.args
            .iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect()
    }

    fn error(result: Result<Plan, CliError>) -> (String, Option<String>) {
        let err = result.expect_err("ожидалась ошибка");
        (err.code, err.field)
    }

    #[test]
    fn scroll_inline_is_one_page_of_twenty_jsonl() {
        let plan = planned("reviews_scroll", json!({})).unwrap();
        assert_eq!(
            argv(&plan),
            [
                "reviews",
                "scroll",
                "--format=jsonl",
                "--max-records=20",
                "--per-page=20",
                "--json",
                "--no-input"
            ]
        );
        assert_eq!((plan.stdin, plan.destination), (None, Destination::Inline));
    }

    #[test]
    fn server_flags_come_first() {
        let forwarded = [
            OsString::from("--profile=staging"),
            OsString::from("--timeout=30"),
        ];
        let plan = plan(
            &tool("reviews_get"),
            &args(json!({"id": "r1"})),
            &forwarded,
            Path::new("/"),
        )
        .unwrap();
        assert_eq!(
            &argv(&plan)[..3],
            ["--profile=staging", "--timeout=30", "reviews"]
        );
    }

    /// M9: значение не становится флагом, id — после `--`.
    #[test]
    fn values_stay_single_tokens() {
        let plan = planned(
            "reviews_scroll",
            json!({"filter": "--profile=prod", "state": "-n"}),
        )
        .unwrap();
        let args = argv(&plan);
        assert!(
            args.contains(&"--filter=--profile=prod".to_string()),
            "{args:?}"
        );
        assert!(args.contains(&"--state=-n".to_string()), "{args:?}");
        assert!(!args.contains(&"--profile=prod".to_string()), "{args:?}");
        let get = argv(&planned("reviews_get", json!({"id": "--base-url=http://evil"})).unwrap());
        assert_eq!(&get[get.len() - 2..], ["--", "--base-url=http://evil"]);
    }

    #[test]
    fn lists_accept_arrays_and_comma_strings() {
        for include in [json!(["author", "product"]), json!("author,product")] {
            let plan = planned("reviews_scroll", json!({"include": include})).unwrap();
            assert!(argv(&plan).contains(&"--include=author,product".to_string()));
        }
        let csv = planned(
            "reviews_scroll",
            json!({"format": "csv", "fields": ["id", "rating"]}),
        )
        .unwrap();
        let args = argv(&csv);
        assert!(
            args.contains(&"--format=csv".to_string())
                && args.contains(&"--fields=id,rating".to_string()),
            "{args:?}"
        );
    }

    #[test]
    fn wrong_types_name_the_parameter() {
        for (value, field) in [
            (json!({"max_records": "50"}), "max_records"),
            (json!({"filter": 5}), "filter"),
            (json!({"include": [1]}), "include"),
            (json!({"overwrite": "yes"}), "overwrite"),
        ] {
            assert_eq!(
                error(planned("reviews_scroll", value)),
                ("usage".to_string(), Some(field.to_string()))
            );
        }
    }

    #[test]
    fn unknown_parameter_lists_known_ones() {
        let err = planned("reviews_scroll", json!({"filtr": "x"})).unwrap_err();
        assert_eq!(
            (err.code.as_str(), err.field.as_deref()),
            ("usage", Some("filtr"))
        );
        assert!(err.hint.unwrap().contains("filter"));
    }

    #[test]
    fn inline_is_at_most_one_page() {
        assert_eq!(
            error(planned("reviews_scroll", json!({"max_records": 101}))),
            (
                "invalid_max_records".to_string(),
                Some("max_records".to_string())
            )
        );
        assert_eq!(
            error(planned("reviews_scroll", json!({"max_records": 0}))).0,
            "invalid_max_records"
        );
        let big = planned(
            "reviews_scroll",
            json!({"max_records": 250, "output_file": "out.jsonl"}),
        )
        .unwrap();
        let args = argv(&big);
        assert!(
            args.contains(&"--max-records=250".to_string())
                && args.contains(&"--per-page=100".to_string()),
            "{args:?}"
        );
        let all = planned("reviews_scroll", json!({"output_file": "out.jsonl"})).unwrap();
        assert!(
            !argv(&all).iter().any(|a| a.starts_with("--max-records")),
            "в файл без лимита — всё"
        );
    }

    #[test]
    fn get_requires_id() {
        assert_eq!(
            error(planned("reviews_get", json!({}))),
            ("usage".to_string(), Some("id".to_string()))
        );
    }

    #[test]
    fn write_attributes_go_to_stdin() {
        let plan = planned(
            "reviews_create",
            json!({"rating": 5, "body": "- Отлично", "dry_run": true}),
        )
        .unwrap();
        assert_eq!(
            argv(&plan),
            [
                "reviews",
                "create",
                "--data=-",
                "--dry-run",
                "--json",
                "--no-input"
            ]
        );
        let data: Value = serde_json::from_str(plan.stdin.as_deref().unwrap()).unwrap();
        assert_eq!(
            data,
            json!({"rating": 5, "body": "- Отлично"}),
            "dry_run — флаг, не атрибут"
        );
    }

    #[test]
    fn update_keeps_explicit_null_and_puts_id_last() {
        let plan = planned("products_update", json!({"id": "p1", "price": null})).unwrap();
        let args = argv(&plan);
        assert_eq!(&args[args.len() - 2..], ["--", "p1"]);
        let data: Value = serde_json::from_str(plan.stdin.as_deref().unwrap()).unwrap();
        assert_eq!(data, json!({"price": null}), "null очищает атрибут");
    }

    #[test]
    fn output_file_policy() {
        let cwd = Path::new("/work");
        let file = |mode| -> Result<Destination, CliError> {
            Ok(Destination::File {
                path: PathBuf::from("/work/out.jsonl"),
                mode,
            })
        };
        let out = || Some("out.jsonl".to_string());
        assert_eq!(
            destination(None, false, false, cwd),
            Ok(Destination::Inline)
        );
        assert_eq!(
            destination(out(), false, false, cwd),
            file(FileMode::CreateNew)
        );
        assert_eq!(
            destination(out(), false, true, cwd),
            file(FileMode::Truncate)
        );
        assert_eq!(destination(out(), true, false, cwd), file(FileMode::Append));
        assert_eq!(
            destination(Some("/abs/out.jsonl".into()), false, false, cwd),
            Ok(Destination::File {
                path: PathBuf::from("/abs/out.jsonl"),
                mode: FileMode::CreateNew
            })
        );
        for (file, state, overwrite, field) in [
            (out(), true, true, "overwrite"),
            (None, false, true, "overwrite"),
            (Some(" ".to_string()), false, false, "output_file"),
        ] {
            let err = destination(file, state, overwrite, cwd).unwrap_err();
            assert_eq!(
                (err.code.as_str(), err.field.as_deref()),
                ("usage", Some(field))
            );
        }
    }

    #[test]
    fn envelope_is_the_last_stderr_line_or_stdout_for_writes() {
        let env = r#"{"ok":true,"command":"reviews.scroll","cli_version":"0","dry_run":false,"result":{},"warnings":[]}"#;
        let stderr = format!("debug: GET /scroll/reviews\n{env}\n");
        let (line, ok, debug) = split_envelope(Kind::Scroll, "{\"id\":\"r1\"}\n", &stderr).unwrap();
        assert_eq!(
            (line.as_str(), ok, debug),
            (env, true, vec!["debug: GET /scroll/reviews"])
        );
        let write = r#"{"ok":true,"command":"reviews.create","cli_version":"0","dry_run":true,"result":{},"warnings":[]}"#;
        let (line, ok, debug) =
            split_envelope(Kind::Write, &format!("{write}\n"), "debug: x\n").unwrap();
        assert_eq!((line.as_str(), ok, debug), (write, true, vec!["debug: x"]));
        let failed = r#"{"ok":false,"command":"reviews.get","cli_version":"0","dry_run":false,"error":{},"warnings":[]}"#;
        let (_, ok, _) = split_envelope(Kind::Get, "", failed).unwrap();
        assert!(!ok);
        assert!(
            split_envelope(Kind::Scroll, "{\"ok\":true,\"command\":\"x\"}", "паника").is_none(),
            "у scroll stdout — данные"
        );
        assert!(split_envelope(Kind::Write, "", "").is_none());
    }

    #[test]
    fn output_file_is_added_to_result() {
        let env = r#"{"ok":true,"command":"reviews.scroll","result":{"emitted":1}}"#;
        let added: Value =
            serde_json::from_str(&add_output_file(env, Path::new("/work/out.jsonl"))).unwrap();
        assert_eq!(
            added["result"],
            json!({"emitted": 1, "output_file": "/work/out.jsonl"})
        );
    }

    #[test]
    fn create_new_refuses_existing_file() {
        let dir = std::env::temp_dir().join(format!("aplaut-mcp-open-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("out.jsonl");
        std::fs::write(&path, "old\n").unwrap();
        let err = open(&path, FileMode::CreateNew).unwrap_err();
        assert_eq!(
            (err.code.as_str(), err.field.as_deref()),
            ("output_exists", Some("output_file"))
        );
        use std::io::Write as _;
        open(&path, FileMode::Append)
            .unwrap()
            .write_all(b"new\n")
            .unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "old\nnew\n");
        // Перезапись и новый файл меняются, только если выгрузка в них писала (§5).
        let failed = OutputFile::open(&path, FileMode::Truncate).unwrap();
        failed.settle(false).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "old\nnew\n");
        let replaced = OutputFile::open(&path, FileMode::Truncate).unwrap();
        // Дочерний процесс пишет в дубликат дескриптора — курсор общий.
        (&replaced.handle).write_all(b"x\n").unwrap();
        replaced.settle(true).unwrap();
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "x\n",
            "хвост старого отрезан"
        );
        let fresh = dir.join("new.jsonl");
        OutputFile::open(&fresh, FileMode::CreateNew)
            .unwrap()
            .settle(false)
            .unwrap();
        assert!(!fresh.exists(), "пустой файл после ошибки удалён");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
