//! `update` ресурсов, у которых PUT создаёт объект, если его нет (upsert): отзывы, вопросы, клиенты,
//! заказы (спека writes-and-exports R3–R6; стейджинг 2026-09-25 — §9). Сначала `GET`: нет объекта —
//! `not_found`, ничего не создано; `--upsert` пропускает проверку и разрешает создать. PUT частичный
//! и идемпотентен — повторяется, как чтение.

use serde_json::{Map, Value};

use super::writes::{capitalized, id_of, prepare, report_plan, write_operation};
use super::{check_id, connect, Ctx, Outcome};
use crate::cli::UpdateInput;
use crate::error::{CliError, Exit};
use crate::http::{self, ApiClient, Pace, Replay};
use crate::ops::write::{self, Flag, WriteRequest};
use crate::page::Page;
use crate::resources::{Noun, Resource, Verb};
use crate::term::shell_word;

/// Что общему `update` нужно от команды ресурса.
pub struct UpdateJob<'a> {
    pub resource: &'static Resource,
    /// Позиционный ID: внутренний идентификатор или внешний.
    pub id: &'a str,
    /// Флаги атрибутов команды.
    pub flags: Vec<Flag<'a>>,
    pub input: &'a UpdateInput,
    /// Атрибуты, которые берутся из текущей записи, если их не передали (у вопроса — `product_id`).
    pub carry: &'static [&'static str],
    /// Проверка тела до сети сверх схемы (у заказа — строки `order_lines`).
    pub check: fn(&Map<String, Value>) -> Result<(), CliError>,
}

/// `UpdateJob::check` для ресурсов без своих проверок.
pub fn no_check(_: &Map<String, Value>) -> Result<(), CliError> {
    Ok(())
}

/// Что известно о записи до PUT.
enum Current {
    /// GET не делали: `--upsert` без `-n` и без подстановки.
    Unchecked,
    Missing,
    Found(Value),
}

pub fn run(job: UpdateJob, ctx: &Ctx) -> Result<Outcome, CliError> {
    check_id(job.id, "id")?;
    let (spec, template) = write_operation(job.resource, Verb::Update)?;
    let attributes = prepare(
        spec,
        spec.required,
        job.input.data.as_deref(),
        &job.flags,
        ctx,
    )?;
    if attributes.is_empty() {
        return Err(nothing_to_update());
    }
    let noun = job.resource.noun;
    let upsert = job.input.upsert;
    let dry_run = job.input.dry.dry_run;
    if !upsert {
        check_create_only(spec.create_only, &attributes, noun)?;
    }
    (job.check)(&attributes)?;
    let mut api = connect(ctx)?;
    let missing_carry = job.carry.iter().any(|name| !attributes.contains_key(*name));
    // GET — проверка R3, честный план под -n и подстановка carry.
    let current = if upsert && !dry_run && !missing_carry {
        Current::Unchecked
    } else {
        fetch(&mut api, job.resource, job.id)?
    };
    let attributes = match &current {
        Current::Missing if !upsert => return Err(not_found(noun, job.id)),
        Current::Found(record) => with_carried(attributes, record, job.carry),
        _ => attributes,
    };
    let path = template.replace("{id}", &http::path_segment(job.id));
    let request = write::request(spec, path, attributes);
    if dry_run {
        let exists = matches!(current, Current::Found(_));
        report_plan(&request, ctx);
        if !exists {
            ctx.reporter.info(&format!(
                "{} {} нет — PUT создаст его (--upsert)",
                capitalized(noun.genitive),
                job.id
            ));
        }
        return Ok(Outcome::stdout(result(&request, None, "exists", exists)).with_dry_run(true));
    }
    let verify = format!(
        "проверьте: aplaut {} get {}",
        job.resource.name,
        shell_word(job.id)
    );
    let written = write::submit(&mut api, &request, Replay::Safe, &verify)?;
    let created = written.status == 201;
    ctx.reporter.info(&format!(
        "{} {}: id {}",
        capitalized(noun.one),
        if created {
            "создан"
        } else {
            "обновлён"
        },
        id_of(&written.record)
    ));
    Ok(Outcome::stdout(result(
        &request,
        Some(written.record),
        "created",
        created,
    )))
}

/// `result`: `{"request", "updated", "created"}`; под `-n` — `{"request", "updated": null, "exists"}`.
fn result(request: &WriteRequest, record: Option<Value>, key: &str, flag: bool) -> Value {
    let mut result = write::result(request, "updated", record);
    result[key] = Value::Bool(flag);
    result
}

/// `GET /{type}/{id}`: запись или `Missing` на 404; прочие ошибки — как у `get`, PUT не уходит.
fn fetch(api: &mut ApiClient, resource: &Resource, id: &str) -> Result<Current, CliError> {
    let path = format!("/{}/{}", resource.records_type, http::path_segment(id));
    match api.get(&path, &[], Pace::Default) {
        Ok(response) => {
            let page = Page::single(response.body)?;
            let record = page
                .data
                .into_iter()
                .next()
                .expect("Page::single — ровно одна запись");
            Ok(Current::Found(record))
        }
        Err(err) if err.exit == Exit::NotFound => Ok(Current::Missing),
        Err(err) => Err(err),
    }
}

/// Недостающие атрибуты из `carry` — из текущей записи, если там есть значение.
fn with_carried(
    attributes: Map<String, Value>,
    record: &Value,
    carry: &[&str],
) -> Map<String, Value> {
    let carried: Vec<(String, Value)> = carry
        .iter()
        .filter(|name| !attributes.contains_key(**name))
        .filter_map(|name| {
            record["attributes"]
                .get(*name)
                .filter(|value| !value.is_null())
                .map(|value| (name.to_string(), value.clone()))
        })
        .collect();
    attributes.into_iter().chain(carried).collect()
}

/// Пустой PUT ничего не меняет (стейджинг, 2026-09-25): скорее всего, атрибут забыт.
fn nothing_to_update() -> CliError {
    CliError::usage(
        "nothing_to_update",
        "нечего менять: не передан ни один атрибут",
    )
    .with_hint("передайте флаг атрибута или ключи в --data (список — в --help)")
}

/// Без `--upsert` объект уже есть, а эти атрибуты сервер применяет только при создании и у
/// существующего молча оставляет прежними (e-mail и телефон клиента; стейджинг, 2026-09-25).
fn check_create_only(
    create_only: &[&str],
    attributes: &Map<String, Value>,
    noun: Noun,
) -> Result<(), CliError> {
    let Some(name) = create_only
        .iter()
        .find(|name| attributes.contains_key(**name))
    else {
        return Ok(());
    };
    Err(CliError::usage(
        "invalid_attribute",
        format!(
            "атрибут «{name}»: сервер задаёт его только при создании {}, у существующего молча оставляет прежнее значение",
            noun.genitive
        ),
    )
    .with_field(*name)
    .with_hint("с --upsert он задаётся, если объекта ещё нет; у существующего его не изменить"))
}

/// R3: PUT по неизвестному ID создал бы новый объект — опечатка в ID не должна его публиковать.
fn not_found(noun: Noun, id: &str) -> CliError {
    CliError::new(
        Exit::NotFound,
        "not_found",
        format!("{} {id} нет — ничего не изменено", capitalized(noun.genitive)),
    )
    .with_field("id")
    .with_hint(format!(
        "проверьте ID (внутренний id или внешний); если объекта действительно нет и его нужно создать с внешним id «{id}» — тот же вызов с --upsert"
    ))
}
