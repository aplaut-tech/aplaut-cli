//! Запись у отзывов: `create` и `comment` (спека reviews-write). Чтение (`scroll`, `get`) —
//! общее с другими ресурсами, в `records`; общее для записи — в `writes`.

use serde_json::{Map, Value};

use super::writes::{check_texts, execute, id_of, prepare, write_operation, Submission};
use super::{check_id, records, Ctx, Outcome};
use crate::cli::{CommentArgs, CreateReviewArgs, ReviewsVerb};
use crate::error::CliError;
use crate::http::{self, Replay};
use crate::ops::write::{self, Flag};
use crate::resources::{self, Verb};
use crate::term::shell_word;

pub fn run(verb: ReviewsVerb, ctx: &Ctx) -> Result<Outcome, CliError> {
    match verb {
        ReviewsVerb::Records(verb) => records::run(&resources::REVIEWS, verb, ctx),
        ReviewsVerb::Create(args) => create(&args, ctx),
        ReviewsVerb::Comment(args) => comment(&args, ctx),
    }
}

/// Сервер принимает отзыв с любым из текстов (стейджинг, 2026-09-24; §15 дизайна среза 1):
/// схема требует body, описание — хотя бы одно из pros, cons, body.
const REVIEW_TEXT: [&str; 3] = ["body", "pros", "cons"];

fn create(args: &CreateReviewArgs, ctx: &Ctx) -> Result<Outcome, CliError> {
    check_texts(
        &["reviews", "create"],
        &[
            ("body", args.body.as_deref()),
            ("pros", args.pros.as_deref()),
            ("cons", args.cons.as_deref()),
        ],
    )?;
    let (spec, path) = write_operation(&resources::REVIEWS, Verb::Create)?;
    let flags = create_flags(args);
    let required: Vec<&str> = spec
        .required
        .iter()
        .copied()
        .filter(|name| !REVIEW_TEXT.contains(name))
        .collect();
    let attributes = prepare(spec, &required, args.data.as_deref(), &flags, ctx)?;
    require_review_text(&attributes)?;
    let verify = match attributes.get("external_id").and_then(Value::as_str) {
        // Id, который get не примет (пустой, с «.» или «/»), проверкой предлагать нельзя.
        Some(id) if check_id(id, "external_id").is_ok() => format!(
            "проверьте, создан ли отзыв: aplaut reviews get {}",
            shell_word(id)
        ),
        _ => "проверьте в личном кабинете, прежде чем повторять; с --external-id это делает aplaut reviews get <external_id>".to_string(),
    };
    let submission = Submission {
        request: write::request(spec, path, attributes),
        replay: Replay::OnlyIfUnprocessed,
        verify,
        outcome: "created",
    };
    execute(submission, args.dry.dry_run, ctx, |created| {
        format!("Отзыв создан: id {}", id_of(created))
    })
}

fn require_review_text(attributes: &Map<String, Value>) -> Result<(), CliError> {
    if REVIEW_TEXT
        .iter()
        .any(|name| attributes.contains_key(*name))
    {
        return Ok(());
    }
    Err(CliError::usage(
        "missing_attribute",
        "у отзыва нет текста: нужно хотя бы одно из body, pros, cons",
    )
    .with_field("body")
    .with_hint("передайте --body, --pros или --cons (или ключ в --data)"))
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

fn comment(args: &CommentArgs, ctx: &Ctx) -> Result<Outcome, CliError> {
    check_id(&args.review_id, "review_id")?;
    check_texts(&["reviews", "comment"], &[("text", args.text.as_deref())])?;
    let (spec, template) = write_operation(&resources::REVIEWS, Verb::Comment)?;
    let flags = comment_flags(args);
    let attributes = prepare(spec, spec.required, args.data.as_deref(), &flags, ctx)?;
    let review = &args.review_id;
    let lookup = format!(
        "aplaut reviews get {} --include comments --format jsonl",
        shell_word(review)
    );
    let verify = match attributes.get("external_id").and_then(Value::as_str) {
        Some(id) => format!("проверьте, добавлен ли комментарий с external_id {id}: {lookup}"),
        None => format!("проверьте, прежде чем повторять: {lookup}"),
    };
    let path = template.replace("{id}", &http::path_segment(review));
    let submission = Submission {
        request: write::request(spec, path, attributes),
        replay: Replay::OnlyIfUnprocessed,
        verify,
        outcome: "created",
    };
    execute(submission, args.dry.dry_run, ctx, |created| {
        format!(
            "Комментарий добавлен к отзыву {review}: id {}",
            id_of(created)
        )
    })
}

/// Флаги `comment` → атрибуты схемы POST /reviews/{id}/relationships/comments (§3).
fn comment_flags(args: &CommentArgs) -> Vec<Flag<'_>> {
    vec![
        ("text", args.text.as_deref()),
        ("author_name", args.author_name.as_deref()),
        ("author_email", args.author_email.as_deref()),
        ("state", args.state.as_deref()),
        ("parent_id", args.parent_id.as_deref()),
        ("external_id", args.external_id.as_deref()),
    ]
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use super::*;
    use crate::cli::{Cli, Command};

    fn reviews_verb(args: &[&str]) -> ReviewsVerb {
        match Cli::try_parse_from(args).unwrap().command {
            Command::Reviews { verb } => verb,
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn every_flag_is_an_attribute_of_the_spec_schema() {
        let (create_spec, _) = write_operation(&resources::REVIEWS, Verb::Create).unwrap();
        let ReviewsVerb::Create(args) = reviews_verb(&["aplaut", "reviews", "create"]) else {
            panic!("create");
        };
        for (name, _) in create_flags(&args) {
            assert!(create_spec.attribute(name).is_some(), "create --{name}");
        }
        let (comment_spec, _) = write_operation(&resources::REVIEWS, Verb::Comment).unwrap();
        let ReviewsVerb::Comment(args) = reviews_verb(&["aplaut", "reviews", "comment", "r1"])
        else {
            panic!("comment");
        };
        for (name, _) in comment_flags(&args) {
            assert!(comment_spec.attribute(name).is_some(), "comment --{name}");
        }
    }
}
