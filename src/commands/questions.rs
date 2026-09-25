//! Запись у вопросов: `create` и `update` (спека writes-and-exports §2). Чтение (`scroll`, `get`) —
//! общее, в `records`; общее для записи — в `writes` и `updates`.

use super::updates::{self, UpdateJob};
use super::writes::{check_texts, create_unique, prepare, write_operation};
use super::{records, Ctx, Outcome};
use crate::cli::{CreateQuestionArgs, QuestionFields, QuestionsVerb, UpdateQuestionArgs};
use crate::error::CliError;
use crate::ops::write::{self, Flag};
use crate::resources::{self, Verb};

/// Без `product_id` PUT вопроса о товаре сохраняет правку, но отвечает 404 (стейджинг, 2026-09-25;
/// спека writes-and-exports §9): `update` берёт его из текущей записи (решение владельца).
const CARRY: &[&str] = &["product_id"];

pub fn run(verb: QuestionsVerb, ctx: &Ctx) -> Result<Outcome, CliError> {
    match verb {
        QuestionsVerb::Records(verb) => records::run(&resources::QUESTIONS, verb, ctx),
        QuestionsVerb::Create(args) => create(&args, ctx),
        QuestionsVerb::Update(args) => update(&args, ctx),
    }
}

fn create(args: &CreateQuestionArgs, ctx: &Ctx) -> Result<Outcome, CliError> {
    check_texts(&["questions", "create"], &text(&args.fields))?;
    let (spec, path) = write_operation(&resources::QUESTIONS, Verb::Create)?;
    let flags = create_flags(args);
    let attributes = prepare(spec, spec.required, args.data.as_deref(), &flags, ctx)?;
    create_unique(
        &resources::QUESTIONS,
        "external_id",
        write::request(spec, path, attributes),
        args.dry.dry_run,
        ctx,
    )
    .map_err(text_field)
}

fn update(args: &UpdateQuestionArgs, ctx: &Ctx) -> Result<Outcome, CliError> {
    check_texts(&["questions", "update"], &text(&args.fields))?;
    let job = UpdateJob {
        resource: &resources::QUESTIONS,
        id: &args.id,
        flags: field_flags(&args.fields),
        input: &args.input,
        carry: CARRY,
        check: updates::no_check,
    };
    updates::run(job, ctx).map_err(text_field)
}

/// Сервер называет текст вопроса `body` (422 «body: не может быть пустым»; стейджинг, 2026-09-25),
/// а атрибут и флаг — `text`.
fn text_field(err: CliError) -> CliError {
    if err.code == "validation_failed" && err.field.as_deref() == Some("body") {
        err.with_field("text")
    } else {
        err
    }
}

fn create_flags(args: &CreateQuestionArgs) -> Vec<Flag<'_>> {
    std::iter::once(("external_id", args.external_id.as_deref()))
        .chain(field_flags(&args.fields))
        .collect()
}

/// Флаги вопроса → атрибуты схем POST /questions и PUT /questions/{id}.
fn field_flags(fields: &QuestionFields) -> Vec<Flag<'_>> {
    vec![
        ("text", fields.text.as_deref()),
        ("product_id", fields.product_id.as_deref()),
        ("author_name", fields.author_name.as_deref()),
        ("author_email", fields.author_email.as_deref()),
        ("state", fields.state.as_deref()),
    ]
}

/// Свободный текст вопроса (`allow_hyphen_values`).
fn text(fields: &QuestionFields) -> [Flag<'_>; 1] {
    [("text", fields.text.as_deref())]
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use super::*;
    use crate::cli::{Cli, Command};

    fn questions_verb(args: &[&str]) -> QuestionsVerb {
        match Cli::try_parse_from(args).unwrap().command {
            Command::Questions { verb } => verb,
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn every_flag_is_an_attribute_of_the_spec_schemas() {
        let QuestionsVerb::Create(args) = questions_verb(&["aplaut", "questions", "create"]) else {
            panic!("create");
        };
        let (create_spec, _) = write_operation(&resources::QUESTIONS, Verb::Create).unwrap();
        for (name, _) in create_flags(&args) {
            assert!(create_spec.attribute(name).is_some(), "create --{name}");
        }
        let (update_spec, _) = write_operation(&resources::QUESTIONS, Verb::Update).unwrap();
        for (name, _) in field_flags(&args.fields) {
            assert!(update_spec.attribute(name).is_some(), "update --{name}");
        }
    }
}
