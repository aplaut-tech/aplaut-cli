//! Общее для команд записи (reviews create/comment, products create/update, create новых
//! ресурсов): схема из спеки, ввод и проверка до сети, план под `-n` или отправка.

use std::io::{self, IsTerminal};
use std::path::Path;

use clap::CommandFactory;
use serde_json::{Map, Value};

use super::{check_id, connect, Ctx, Outcome, DRY_RUN_PREFIX};
use crate::auth::StdinSource;
use crate::cli::Cli;
use crate::error::CliError;
use crate::http::Replay;
use crate::ops::write::{self, Flag, WriteRequest};
use crate::page::record_id;
use crate::resources::{Resource, Verb};
use crate::spec::{self, WriteSpec};
use crate::term::shell_word;

/// Готовая запись: запрос, политика повтора, как проверить исход и ключ записи в `result`.
pub struct Submission {
    pub request: WriteRequest,
    pub replay: Replay,
    /// Подсказка для `request_outcome_unknown`: как проверить, прежде чем повторять.
    pub verify: String,
    /// Ключ записи в `result`: `created` или `updated`.
    pub outcome: &'static str,
}

/// Схема тела и шаблон пути — из спеки, по операции, которую объявляет `resources` (W6).
pub fn write_operation(
    resource: &Resource,
    verb: Verb,
) -> Result<(&'static WriteSpec, String), CliError> {
    let (method, path) = verb.operation(resource);
    let spec = spec::write_spec(method, &path).ok_or_else(|| {
        CliError::general(
            "internal",
            format!("в спеке нет схемы тела {method} {path}"),
        )
    })?;
    Ok((spec, path))
}

/// Всё локальное — до сети: stdin, `--data`, флаги, проверка по схеме (§3).
pub fn prepare(
    spec: &WriteSpec,
    required: &[&str],
    data: Option<&Path>,
    flags: &[Flag],
    ctx: &Ctx,
) -> Result<Map<String, Value>, CliError> {
    write::check_stdin(
        data,
        ctx.global.token_stdin,
        ctx.global.token_file.as_deref(),
    )?;
    let data = match data {
        Some(path) => {
            let stdin = io::stdin();
            let is_terminal = stdin.is_terminal();
            let mut lock = stdin.lock();
            let mut source = StdinSource {
                is_terminal,
                reader: &mut lock,
            };
            Some(write::read_data(path, &mut source)?)
        }
        None => None,
    };
    let attributes = write::attributes(spec, data, flags);
    write::validate(spec, &attributes, required, flags)?;
    Ok(attributes)
}

/// План под `-n` или отправка; `result` одинаковый (W9), токена в нём нет.
pub fn execute(
    submission: Submission,
    dry_run: bool,
    ctx: &Ctx,
    done: impl Fn(&Value) -> String,
) -> Result<Outcome, CliError> {
    // Токен проверяется и под -n (agent mode §5): план, который упадёт на no_token, бесполезен.
    let mut api = connect(ctx)?;
    let Submission {
        request,
        replay,
        verify,
        outcome,
    } = submission;
    if dry_run {
        report_plan(&request, ctx);
        return Ok(Outcome::stdout(write::result(&request, outcome, None)).with_dry_run(true));
    }
    let record = write::submit(&mut api, &request, replay, &verify)?.record;
    ctx.reporter.info(&done(&record));
    Ok(Outcome::stdout(write::result(
        &request,
        outcome,
        Some(record),
    )))
}

/// Текст плана под `-n`: метод, путь и тело (agent mode §5).
pub fn report_plan(request: &WriteRequest, ctx: &Ctx) {
    ctx.reporter.info(&format!(
        "{DRY_RUN_PREFIX} {} {}",
        request.method, request.path
    ));
    ctx.reporter
        .info(&serde_json::to_string_pretty(&request.body).expect("JSON сериализуется"));
}

/// Слово с заглавной буквы — начало сообщения: «Отзыв обновлён».
pub fn capitalized(word: &str) -> String {
    let mut chars = word.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

pub fn id_of(record: &Value) -> &str {
    record_id(record).unwrap_or("?")
}

/// Свободный текст можно начинать с `-` (`allow_hyphen_values`), но значение, равное флагу
/// команды, значит, что сам текст пропущен (`--text $EMPTY -n`): иначе `-n` ушёл бы в отзыв
/// текстом, а пробный запуск стал бы настоящей записью.
pub fn check_texts(path: &[&str], texts: &[Flag]) -> Result<(), CliError> {
    let mut root = Cli::command();
    root.build();
    let leaf = path.iter().fold(&root, |command, name| {
        command
            .find_subcommand(name)
            .expect("подкоманда есть в дереве")
    });
    let flags: Vec<String> = leaf
        .get_arguments()
        .flat_map(|arg| {
            let long = arg.get_long().map(|l| format!("--{l}"));
            let short = arg.get_short().map(|s| format!("-{s}"));
            long.into_iter().chain(short)
        })
        .collect();
    for (name, value) in texts {
        if let Some(value) = value.filter(|v| flags.iter().any(|f| f == v)) {
            return Err(CliError::usage(
                "usage",
                format!("--{name}: значение «{value}» — это флаг; похоже, текст пропущен"),
            )
            .with_field(*name)
            .with_hint(
                "передайте текст в кавычках; текст, совпадающий с флагом, — ключом в --data",
            ));
        }
    }
    Ok(())
}

/// `create`, повтор которого безопасен, если задан внешний id (`key`): второй объект с тем же
/// значением сервер не создаст, а ответит 422 `is already taken` (товары — P6; вопросы, клиенты,
/// заказы — стейджинг, 2026-09-25, спека writes-and-exports §9).
pub fn create_unique(
    resource: &'static Resource,
    key: &'static str,
    request: WriteRequest,
    dry_run: bool,
    ctx: &Ctx,
) -> Result<Outcome, CliError> {
    let noun = resource.noun;
    let external_id = request.body["data"]["attributes"][key]
        .as_str()
        .map(str::to_string);
    let submission = Submission {
        verify: retry_hint(resource, key, external_id.as_deref()),
        request,
        replay: Replay::OnlyIfUnprocessed,
        outcome: "created",
    };
    execute(submission, dry_run, ctx, |created| {
        format!("{} создан: id {}", capitalized(noun.one), id_of(created))
    })
    .map_err(|err| already_taken(err, resource, key, external_id.as_deref()))
}

/// Как проверить исход `request_outcome_unknown`: по внешнему id, если `get` его примет.
fn retry_hint(resource: &Resource, key: &str, external_id: Option<&str>) -> String {
    let one = resource.noun.one;
    match external_id {
        Some(id) if check_id(id, key).is_ok() => format!(
            "повтор безопасен: второй {one} с тем же {key} сервер не создаст; проверить — aplaut {} get {}",
            resource.name,
            shell_word(id)
        ),
        Some(_) => format!(
            "повтор безопасен: второй {one} с тем же {key} сервер не создаст; проверьте в личном кабинете"
        ),
        None => format!(
            "проверьте в личном кабинете, прежде чем повторять: без {key} повтор создаст второй {one}"
        ),
    }
}

/// 422 `<key> is already taken`: объект уже есть — его меняет `update`. Другие ошибки того же
/// атрибута (например, пустой) — не про существующий объект.
fn already_taken(
    err: CliError,
    resource: &Resource,
    key: &str,
    external_id: Option<&str>,
) -> CliError {
    let taken = err.code == "validation_failed"
        && err.field.as_deref() == Some(key)
        && err.message.contains("is already taken");
    let Some(id) = external_id.filter(|_| taken) else {
        return err;
    };
    let noun = capitalized(resource.noun.one);
    let hint = match check_id(id, key) {
        Ok(()) => format!(
            "{noun} с {key} {id} уже есть — изменить его: aplaut {} update {} …",
            resource.name,
            shell_word(id)
        ),
        Err(_) => format!(
            "{noun} с {key} {id} уже есть; такой id update не адресует — измените его по внутреннему id (поле id записи)"
        ),
    };
    err.with_hint(hint)
}
