//! Запись у отзывов: `create` и `comment` (спека reviews-write). Чтение (`scroll`, `get`) —
//! общее с другими ресурсами, в `records`.

use std::io::{self, IsTerminal};
use std::path::Path;

use serde_json::{Map, Value};

use super::{connect, records, Ctx, Outcome, DRY_RUN_PREFIX};
use crate::auth::StdinSource;
use crate::cli::{CreateReviewArgs, ReviewsVerb};
use crate::error::CliError;
use crate::ops::write::{self, Flag, WriteRequest, WriteResult};
use crate::page::record_id;
use crate::resources::{self, Verb};
use crate::spec::{self, WriteSpec};
use crate::term::shell_word;

pub fn run(verb: ReviewsVerb, ctx: &Ctx) -> Result<Outcome, CliError> {
    match verb {
        ReviewsVerb::Records(verb) => records::run(&resources::REVIEWS, verb, ctx),
        ReviewsVerb::Create(args) => create(&args, ctx),
    }
}

fn create(args: &CreateReviewArgs, ctx: &Ctx) -> Result<Outcome, CliError> {
    let (spec, path) = write_operation(Verb::Create)?;
    let flags = create_flags(args);
    let attributes = prepare(spec, args.data.as_deref(), &flags, ctx)?;
    let verify = match attributes.get("external_id").and_then(Value::as_str) {
        Some(id) => format!(
            "проверьте, создан ли отзыв: aplaut reviews get {}",
            shell_word(id)
        ),
        None => "проверьте в личном кабинете, прежде чем повторять; с --external-id это делает aplaut reviews get <external_id>".to_string(),
    };
    let request = write::request(spec, path, attributes);
    execute(request, args.dry.dry_run, &verify, ctx, |created| {
        format!("Отзыв создан: id {}", id_of(created))
    })
}

/// Флаги `create` → атрибуты схемы POST /reviews (§3).
fn create_flags(args: &CreateReviewArgs) -> Vec<Flag<'_>> {
    vec![
        ("rating", args.rating.as_deref()),
        ("body", args.body.as_deref()),
        ("pros", args.pros.as_deref()),
        ("cons", args.cons.as_deref()),
        ("product_id", args.product_id.as_deref()),
        ("external_id", args.external_id.as_deref()),
        ("author_name", args.author_name.as_deref()),
        ("author_email", args.author_email.as_deref()),
        ("state", args.state.as_deref()),
    ]
}

/// Схема тела и шаблон пути — из спеки, по операции, которую объявляет `resources` (W6).
fn write_operation(verb: Verb) -> Result<(&'static WriteSpec, String), CliError> {
    let (method, path) = verb.operation(&resources::REVIEWS);
    let spec = spec::write_spec(method, &path).ok_or_else(|| {
        CliError::general(
            "internal",
            format!("в спеке нет схемы тела {method} {path}"),
        )
    })?;
    Ok((spec, path))
}

/// Всё локальное — до сети: stdin, `--data`, флаги, проверка по схеме (§3).
fn prepare(
    spec: &WriteSpec,
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
    write::validate(spec, &attributes, flags)?;
    Ok(attributes)
}

/// План под `-n` или отправка; `result` одинаковый (W9), токена в нём нет.
fn execute(
    request: WriteRequest,
    dry_run: bool,
    verify: &str,
    ctx: &Ctx,
    done: impl Fn(&Value) -> String,
) -> Result<Outcome, CliError> {
    // Токен проверяется и под -n (agent mode §5): план, который упадёт на no_token, бесполезен.
    let mut api = connect(ctx)?;
    if dry_run {
        ctx.reporter.info(&format!(
            "{DRY_RUN_PREFIX} {} {}",
            request.method, request.path
        ));
        ctx.reporter
            .info(&serde_json::to_string_pretty(&request.body).expect("JSON сериализуется"));
        let result = WriteResult {
            request,
            created: None,
        };
        return Ok(Outcome::stdout(result).with_dry_run(true));
    }
    let created = write::submit(&mut api, &request, verify)?;
    ctx.reporter.info(&done(&created));
    Ok(Outcome::stdout(WriteResult {
        request,
        created: Some(created),
    }))
}

fn id_of(record: &Value) -> &str {
    record_id(record).unwrap_or("?")
}
