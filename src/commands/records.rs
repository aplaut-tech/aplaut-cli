//! Глаголы чтения: scroll (reviews, products, questions) и get (и у клиентов и заказов).

use std::collections::HashSet;
use std::io::{self, BufWriter, Write};
use std::path::Path;

use serde::Serialize;

use super::{check_id, connect, path_text, Ctx, Outcome};
use crate::cli::{GetArgs, RecordsVerb, ScrollArgs};
use crate::error::CliError;
use crate::filter;
use crate::http::{self, Pace};
use crate::ops::scroll::{self, ScrollJob, ScrollOutcome};
use crate::output::{self, tabular, Format};
use crate::page::Page;
use crate::resources::Resource;
use crate::spec::{self, ScrollSpec};
use crate::state::ScrollParams;

pub fn run(resource: &'static Resource, verb: RecordsVerb, ctx: &Ctx) -> Result<Outcome, CliError> {
    match verb {
        RecordsVerb::Scroll(args) => scroll_records(resource, &args, ctx),
        RecordsVerb::Get(args) => get_record(resource, &args, ctx),
    }
}

/// Итог `get` в `result`: внутренний id — даже если запрошен внешний.
#[derive(Serialize)]
struct GetResult<'a> {
    records_type: &'a str,
    id: Option<String>,
}

pub fn get_record(resource: &Resource, args: &GetArgs, ctx: &Ctx) -> Result<Outcome, CliError> {
    let allowed = spec::get_includes(resource.records_type).ok_or_else(|| {
        CliError::general(
            "internal",
            format!("в спеке нет GET /{}/{{id}}", resource.records_type),
        )
    })?;
    check_id(&args.id, "id")?;
    let include = match &args.include {
        Some(list) => filter::parse_include(list, allowed)?,
        None => Vec::new(),
    };
    let fields = output_fields(args.format, args.fields.as_deref(), &include)?;
    let mut api = connect(ctx)?;
    let path = format!(
        "/{}/{}",
        resource.records_type,
        http::path_segment(&args.id)
    );
    let include_list = include.join(",");
    let query: Vec<(&str, &str)> = if include.is_empty() {
        Vec::new()
    } else {
        vec![("include", include_list.as_str())]
    };
    let response = api.get(&path, &query, Pace::Default)?;
    let page = Page::single(response.body)?;
    let out: Box<dyn Write> = Box::new(BufWriter::new(io::stdout().lock()));
    let mut sink = output::make_sink(args.format, out, &include, fields, ctx.reporter.clone());
    sink.write_page(&page, &HashSet::new())?;
    sink.finish()?;
    Ok(Outcome::stderr(GetResult {
        records_type: resource.records_type,
        id: page.ids().into_iter().next(),
    }))
}

/// Итог обхода в `result` (спека agent mode §2).
#[derive(Serialize)]
struct ScrollResult<'a> {
    records_type: &'a str,
    emitted: u64,
    emitted_total: u64,
    pages: u64,
    duplicates: u64,
    completed: bool,
    already_completed: bool,
    total_count: Option<u64>,
    state_path: Option<String>,
}

fn scroll_records(resource: &Resource, args: &ScrollArgs, ctx: &Ctx) -> Result<Outcome, CliError> {
    let spec = spec::scroll_spec(resource.records_type).ok_or_else(|| {
        CliError::general(
            "internal",
            format!("в спеке нет scroll для {}", resource.records_type),
        )
    })?;
    // Сначала локальные проверки: ошибка в параметрах не должна тратить квоту открытий scroll.
    let params = scroll_params(args, spec)?;
    if args.state.is_some()
        && params
            .fields
            .as_ref()
            .is_some_and(|f| !f.iter().any(|c| c == "id"))
    {
        ctx.reporter.warn(
            "fields_without_id",
            "в --fields нет id: после сбоя записи на границе страницы придут повторно, а без id их не убрать",
        );
    }
    let mut api = connect(ctx)?;
    let out: Box<dyn Write> = Box::new(BufWriter::new(io::stdout().lock()));
    let mut sink = output::make_sink(
        args.format,
        out,
        &params.include,
        params.fields.clone(),
        ctx.reporter.clone(),
    );
    let job = ScrollJob {
        records_type: resource.records_type,
        params,
        state_path: args.state.as_deref(),
        max_records: args.max_records,
    };
    let outcome = scroll::run(
        &mut api,
        sink.as_mut(),
        &job,
        &ctx.reporter,
        ctx.clock.as_ref(),
    )?;
    summarize(resource.name, &outcome, args.state.as_deref(), ctx);
    Ok(Outcome::stderr(ScrollResult {
        records_type: resource.records_type,
        emitted: outcome.emitted,
        emitted_total: outcome.emitted_total,
        pages: outcome.pages,
        duplicates: outcome.duplicates,
        completed: outcome.completed,
        already_completed: outcome.already_completed,
        total_count: outcome.total_count,
        state_path: args.state.as_deref().map(path_text),
    }))
}

pub fn scroll_params(args: &ScrollArgs, spec: &ScrollSpec) -> Result<ScrollParams, CliError> {
    if let Some(expr) = &args.filter {
        filter::check_filter(expr, spec.filters)?;
    }
    let include = match &args.include {
        Some(list) => filter::parse_include(list, spec.includes)?,
        None => Vec::new(),
    };
    let sort = args
        .sort
        .clone()
        .unwrap_or_else(|| spec::SCROLL_SORT_DEFAULT.to_string());
    filter::check_sort(&sort, spec::SCROLL_SORTS)?;
    let per_page = args.per_page.unwrap_or(spec::SCROLL_PER_PAGE_DEFAULT);
    filter::check_per_page(
        per_page,
        spec::SCROLL_PER_PAGE_MIN,
        spec::SCROLL_PER_PAGE_MAX,
    )?;
    if args.max_records == Some(0) {
        return Err(
            CliError::usage("invalid_max_records", "--max-records должно быть больше 0")
                .with_field("max_records"),
        );
    }
    let fields = output_fields(args.format, args.fields.as_deref(), &include)?;
    Ok(ScrollParams {
        filter: args.filter.clone(),
        sort,
        include,
        per_page,
        fields,
    })
}

/// `--fields` — только для табличных форматов: `raw` отдаёт тело ответа как есть, `jsonl` — документ.
fn output_fields(
    format: Format,
    fields: Option<&str>,
    include: &[String],
) -> Result<Option<Vec<String>>, CliError> {
    let Some(list) = fields else {
        return Ok(None);
    };
    if !format.is_tabular() {
        return Err(CliError::usage(
            "fields_need_tabular_format",
            "--fields выбирает колонки табличного вывода и работает только с --format csv",
        )
        .with_field("fields")
        .with_hint("добавьте --format csv или уберите --fields"));
    }
    tabular::parse_fields(list, include).map(Some)
}

/// clig: после работы — коротко, что произошло и что делать дальше.
fn summarize(name: &str, outcome: &ScrollOutcome, state: Option<&Path>, ctx: &Ctx) {
    if outcome.already_completed {
        let path = state.map(|p| p.display().to_string()).unwrap_or_default();
        ctx.reporter.info(&format!(
            "{name}: обход по стейту {path} уже завершён, запросов не было; удалите файл, чтобы начать заново"
        ));
        return;
    }
    let mut line = format!(
        "{name}: записей {}, страниц {}",
        outcome.emitted, outcome.pages
    );
    if outcome.duplicates > 0 {
        line.push_str(&format!(
            ", повторов с прошлой страницы {}",
            outcome.duplicates
        ));
    }
    line.push_str(match (outcome.completed, state) {
        (true, _) => "; обход завершён",
        (false, Some(_)) => "; обход не завершён — запустите ту же команду, чтобы продолжить",
        (false, None) => "; обход не завершён (без --state продолжить нельзя)",
    });
    ctx.reporter.info(&line);
}
