//! Заказы (спека writes-and-exports §4): `get` — общий, в `records`; запись — `create` и `update`.
//! Внешний id заказа — `number`. Данные клиента в заказе персональные.

use serde_json::{Map, Value};

use super::updates::{self, UpdateJob};
use super::writes::{create_unique, prepare, write_operation};
use super::{records, Ctx, Outcome};
use crate::cli::{CreateOrderArgs, OrderFields, OrdersVerb, UpdateOrderArgs};
use crate::error::CliError;
use crate::ops::write::{self, Flag};
use crate::resources::{self, Verb};

/// Сервер требует `number` и хотя бы одно из `consumer_email`, `consumer_phone`; `consumer_name` и
/// `order_lines` из `required` спеки ему не нужны (стейджинг, 2026-09-25; спека writes-and-exports §9).
const CREATE_REQUIRED: [&str; 1] = ["number"];
const CONTACT: [&str; 2] = ["consumer_email", "consumer_phone"];

pub fn run(verb: OrdersVerb, ctx: &Ctx) -> Result<Outcome, CliError> {
    match verb {
        OrdersVerb::Get(args) => records::get_record(&resources::ORDERS, &args, ctx),
        OrdersVerb::Create(args) => create(&args, ctx),
        OrdersVerb::Update(args) => update(&args, ctx),
    }
}

fn create(args: &CreateOrderArgs, ctx: &Ctx) -> Result<Outcome, CliError> {
    let (spec, path) = write_operation(&resources::ORDERS, Verb::Create)?;
    let flags = create_flags(args);
    let attributes = prepare(spec, &CREATE_REQUIRED, args.data.as_deref(), &flags, ctx)?;
    require_contact(&attributes)?;
    check_order_lines(&attributes)?;
    create_unique(
        &resources::ORDERS,
        "number",
        write::request(spec, path, attributes),
        args.dry.dry_run,
        ctx,
    )
}

fn update(args: &UpdateOrderArgs, ctx: &Ctx) -> Result<Outcome, CliError> {
    let job = UpdateJob {
        resource: &resources::ORDERS,
        id: &args.id,
        flags: field_flags(&args.fields),
        input: &args.input,
        carry: &[],
        check: check_order_lines,
    };
    updates::run(job, ctx)
}

fn require_contact(attributes: &Map<String, Value>) -> Result<(), CliError> {
    if CONTACT.iter().any(|name| attributes.contains_key(*name)) {
        return Ok(());
    }
    Err(CliError::usage(
        "missing_attribute",
        "у заказа нет контакта клиента: нужно хотя бы одно из consumer_email, consumer_phone",
    )
    .with_field("consumer_email")
    .with_hint("передайте --consumer-email или --consumer-phone (или ключ в --data)"))
}

/// Строка заказа без `product_id` бесполезна — по ней не попросить отзыв о товаре, — но сервер
/// принимает её молча (стейджинг, 2026-09-25; спека writes-and-exports §9): CLI ловит её до сети.
fn check_order_lines(attributes: &Map<String, Value>) -> Result<(), CliError> {
    let Some(Value::Array(lines)) = attributes.get("order_lines") else {
        return Ok(());
    };
    let bad = lines.iter().position(|line| {
        !line
            .get("product_id")
            .and_then(Value::as_str)
            .is_some_and(|id| !id.trim().is_empty())
    });
    match bad {
        None => Ok(()),
        Some(index) => Err(CliError::usage(
            "invalid_attribute",
            format!("атрибут «order_lines»: у строки {index} нет product_id (строка — id товара)"),
        )
        .with_field("order_lines")
        .with_hint(
            "строка заказа — {\"product_id\":\"…\",\"name\":\"…\",\"price\":…}; product_id — обычно offer.id из YML",
        )),
    }
}

fn create_flags(args: &CreateOrderArgs) -> Vec<Flag<'_>> {
    std::iter::once(("number", args.number.as_deref()))
        .chain(field_flags(&args.fields))
        .collect()
}

/// Флаги заказа → атрибуты схем POST /orders и PUT /orders/{id}.
fn field_flags(fields: &OrderFields) -> Vec<Flag<'_>> {
    vec![
        ("consumer_name", fields.consumer_name.as_deref()),
        ("consumer_email", fields.consumer_email.as_deref()),
        ("consumer_phone", fields.consumer_phone.as_deref()),
    ]
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use super::*;
    use crate::cli::{Cli, Command};

    #[test]
    fn every_flag_is_an_attribute_of_the_spec_schemas() {
        let command = Cli::try_parse_from(["aplaut", "orders", "create"])
            .unwrap()
            .command;
        let Command::Orders {
            verb: OrdersVerb::Create(args),
        } = command
        else {
            panic!("create");
        };
        let (create_spec, _) = write_operation(&resources::ORDERS, Verb::Create).unwrap();
        for (name, _) in create_flags(&args) {
            assert!(create_spec.attribute(name).is_some(), "create --{name}");
        }
        let (update_spec, _) = write_operation(&resources::ORDERS, Verb::Update).unwrap();
        for (name, _) in field_flags(&args.fields) {
            assert!(update_spec.attribute(name).is_some(), "update --{name}");
        }
    }
}
