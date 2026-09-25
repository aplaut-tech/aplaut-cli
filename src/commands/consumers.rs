//! Клиенты (спека writes-and-exports §3): `get` — общий, в `records`; запись — `create` и `update`.
//! Данные клиентов персональные: e-mail, телефон, имя.

use super::updates::{self, UpdateJob};
use super::writes::{create_unique, prepare, write_operation};
use super::{records, Ctx, Outcome};
use crate::cli::{ConsumerFields, ConsumersVerb, CreateConsumerArgs, UpdateConsumerArgs};
use crate::error::CliError;
use crate::ops::write::{self, Flag};
use crate::resources::{self, Verb};

pub fn run(verb: ConsumersVerb, ctx: &Ctx) -> Result<Outcome, CliError> {
    match verb {
        ConsumersVerb::Get(args) => records::get_record(&resources::CONSUMERS, &args, ctx),
        ConsumersVerb::Create(args) => create(&args, ctx),
        ConsumersVerb::Update(args) => update(&args, ctx),
    }
}

fn create(args: &CreateConsumerArgs, ctx: &Ctx) -> Result<Outcome, CliError> {
    let (spec, path) = write_operation(&resources::CONSUMERS, Verb::Create)?;
    let flags = create_flags(args);
    // Сервер создаёт клиента без обязательных атрибутов, хотя спека требует email и name
    // (стейджинг, 2026-09-25; спека writes-and-exports §9).
    let attributes = prepare(spec, &[], args.data.as_deref(), &flags, ctx)?;
    create_unique(
        &resources::CONSUMERS,
        "external_id",
        write::request(spec, path, attributes),
        args.dry.dry_run,
        ctx,
    )
    .map_err(email_taken)
}

fn update(args: &UpdateConsumerArgs, ctx: &Ctx) -> Result<Outcome, CliError> {
    let job = UpdateJob {
        resource: &resources::CONSUMERS,
        id: &args.id,
        flags: field_flags(&args.fields),
        input: &args.input,
        carry: &[],
        check: updates::no_check,
    };
    updates::run(job, ctx)
}

/// 422 `email is already taken`: второго клиента с тем же e-mail сервер не создаёт (стейджинг,
/// 2026-09-25).
fn email_taken(err: CliError) -> CliError {
    let taken = err.code == "validation_failed"
        && err.field.as_deref() == Some("email")
        && err.message.contains("is already taken");
    if !taken {
        return err;
    }
    err.with_hint(
        "клиент с этим e-mail уже есть; изменить его — aplaut consumers update <id или external_id> …",
    )
}

fn create_flags(args: &CreateConsumerArgs) -> Vec<Flag<'_>> {
    std::iter::once(("external_id", args.external_id.as_deref()))
        .chain(field_flags(&args.fields))
        .collect()
}

/// Флаги клиента → атрибуты схем POST /consumers и PUT /consumers/{id}.
fn field_flags(fields: &ConsumerFields) -> Vec<Flag<'_>> {
    vec![
        ("email", fields.email.as_deref()),
        ("name", fields.name.as_deref()),
        ("first_name", fields.first_name.as_deref()),
        ("phone", fields.phone.as_deref()),
        ("unsubscribed", fields.unsubscribed.as_deref()),
    ]
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use super::*;
    use crate::cli::{Cli, Command};

    #[test]
    fn every_flag_is_an_attribute_of_the_spec_schemas() {
        let command = Cli::try_parse_from(["aplaut", "consumers", "create"])
            .unwrap()
            .command;
        let Command::Consumers {
            verb: ConsumersVerb::Create(args),
        } = command
        else {
            panic!("create");
        };
        let (create_spec, _) = write_operation(&resources::CONSUMERS, Verb::Create).unwrap();
        for (name, _) in create_flags(&args) {
            assert!(create_spec.attribute(name).is_some(), "create --{name}");
        }
        let (update_spec, _) = write_operation(&resources::CONSUMERS, Verb::Update).unwrap();
        for (name, _) in field_flags(&args.fields) {
            assert!(update_spec.attribute(name).is_some(), "update --{name}");
        }
    }
}
