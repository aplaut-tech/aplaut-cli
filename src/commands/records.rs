//! Глаголы ресурсов с записями (reviews, products, questions). Собственный модуль у ресурса
//! появится, когда у него будут свои глаголы.

use std::io::{self, BufWriter, IsTerminal, Write};
use std::path::Path;
use std::time::Duration;

use super::Ctx;
use crate::api_error::ErrorContext;
use crate::auth::{self, StdinSource, TokenFlags, DEFAULT_BASE_URL};
use crate::cli::{RecordsVerb, ScrollArgs};
use crate::config::{self, Paths};
use crate::error::CliError;
use crate::filter;
use crate::http::{ApiClient, HttpSettings};
use crate::ops::scroll::{self, ScrollJob, ScrollOutcome};
use crate::output;
use crate::resources::Resource;
use crate::spec::{self, ScrollSpec};
use crate::state::ScrollParams;

pub fn run(resource: &'static Resource, verb: RecordsVerb, ctx: &Ctx) -> Result<(), CliError> {
    match verb {
        RecordsVerb::Scroll(args) => scroll_records(resource, &args, ctx),
    }
}

fn scroll_records(resource: &Resource, args: &ScrollArgs, ctx: &Ctx) -> Result<(), CliError> {
    let spec = spec::scroll_spec(resource.records_type).ok_or_else(|| {
        CliError::general(
            "internal",
            format!("в спеке нет scroll для {}", resource.records_type),
        )
    })?;
    // Сначала локальные проверки: ошибка в параметрах не должна тратить квоту открытий scroll.
    let params = scroll_params(args, spec)?;
    let mut api = connect(ctx)?;
    let out: Box<dyn Write> = Box::new(BufWriter::new(io::stdout().lock()));
    let mut sink = output::make_sink(args.format, out, &params.include, ctx.reporter.clone());
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
    Ok(())
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
    Ok(ScrollParams {
        filter: args.filter.clone(),
        sort,
        include,
        per_page,
    })
}

fn connect(ctx: &Ctx) -> Result<ApiClient, CliError> {
    // Каталог конфигурации и credentials нужны только для профиля: запуск с токеном из флага
    // или env не должен зависеть ни от HOME, ни от исправности чужих файлов.
    let mut load_credentials =
        || config::load_credentials(&Paths::resolve(&ctx.env)?, &ctx.reporter);
    let flags = TokenFlags {
        token_stdin: ctx.global.token_stdin,
        token_file: ctx.global.token_file.as_deref(),
        profile: ctx.global.profile.as_deref(),
    };
    let stdin = io::stdin();
    let is_terminal = stdin.is_terminal();
    let mut lock = stdin.lock();
    let mut source = StdinSource {
        is_terminal,
        reader: &mut lock,
    };
    let (token, token_source) =
        auth::resolve_token(&flags, &ctx.env, &mut load_credentials, &mut source)?;
    let profile = auth::active_profile(ctx.global.profile.as_deref(), &ctx.env)?;
    let profile_flag = ctx.global.profile.is_some();
    if profile_flag && ctx.global.base_url.is_none() && ctx.env.base_url.is_some() {
        ctx.reporter.warn(&format!(
            "APLAUT_BASE_URL не используется: при явном --profile {profile} берётся base URL профиля"
        ));
    }
    let mut load_profile = || -> Result<_, CliError> {
        let config = config::load_config(&Paths::resolve(&ctx.env)?)?;
        Ok(config.profiles.get(&profile).cloned())
    };
    let base_url = auth::resolve_base_url(
        ctx.global.base_url.as_deref(),
        &ctx.env,
        profile_flag,
        &mut load_profile,
    )?;
    if base_url != DEFAULT_BASE_URL {
        ctx.reporter.debug(&format!("base URL: {base_url}"));
    }
    ctx.reporter
        .debug(&format!("токен из {}", token_source.describe()));
    let settings = HttpSettings {
        base_url: base_url.clone(),
        timeout: Duration::from_secs(ctx.global.timeout),
        max_retries: ctx.global.max_retries,
    };
    let error_context = ErrorContext {
        token_source: token_source.describe(),
        base_url,
    };
    Ok(ApiClient::new(
        settings,
        token,
        error_context,
        ctx.clock.clone(),
        ctx.reporter.clone(),
    ))
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
