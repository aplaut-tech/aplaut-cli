//! Запись у товаров: `create` и `update` (спека products-write). Чтение (`scroll`, `get`) —
//! общее с другими ресурсами, в `records`; общее для записи — в `writes`.

use serde_json::Value;

use super::writes::{check_texts, execute, id_of, prepare, write_operation, Submission};
use super::{check_id, records, Ctx, Outcome};
use crate::cli::{CreateProductArgs, ProductFields, ProductsVerb, UpdateProductArgs};
use crate::error::CliError;
use crate::http::{self, Replay};
use crate::ops::write::{self, Flag};
use crate::resources::{self, Verb};
use crate::term::shell_word;

/// `url` сервер требует, хотя в `required` спеки его нет (стейджинг, 2026-09-24; P3).
const CREATE_ALSO_REQUIRED: &str = "url";

pub fn run(verb: ProductsVerb, ctx: &Ctx) -> Result<Outcome, CliError> {
    match verb {
        ProductsVerb::Records(verb) => records::run(&resources::PRODUCTS, verb, ctx),
        ProductsVerb::Create(args) => create(&args, ctx),
        ProductsVerb::Update(args) => update(&args, ctx),
    }
}

fn create(args: &CreateProductArgs, ctx: &Ctx) -> Result<Outcome, CliError> {
    check_texts(&["products", "create"], &description(&args.fields))?;
    let (spec, path) = write_operation(&resources::PRODUCTS, Verb::Create)?;
    let flags = product_flags(&args.fields, args.external_id.as_deref());
    let required: Vec<&str> = spec
        .required
        .iter()
        .copied()
        .chain([CREATE_ALSO_REQUIRED])
        .collect();
    let attributes = prepare(spec, &required, args.data.as_deref(), &flags, ctx)?;
    let external_id = attributes
        .get("external_id")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    // Второй товар с тем же external_id сервер не создаст (422 is already taken, P6).
    let verify = match check_id(&external_id, "external_id") {
        Ok(()) => format!(
            "повтор безопасен: второй товар с тем же external_id сервер не создаст; проверить — aplaut products get {}",
            shell_word(&external_id)
        ),
        Err(_) => "повтор безопасен: второй товар с тем же external_id сервер не создаст; проверьте товар в личном кабинете".to_string(),
    };
    let submission = Submission {
        request: write::request(spec, path, attributes),
        replay: Replay::OnlyIfUnprocessed,
        verify,
        outcome: "created",
    };
    execute(submission, args.dry.dry_run, ctx, |created| {
        format!("Товар создан: id {}", id_of(created))
    })
    .map_err(|err| already_taken(err, &external_id))
}

fn update(args: &UpdateProductArgs, ctx: &Ctx) -> Result<Outcome, CliError> {
    check_id(&args.id, "id")?;
    check_texts(&["products", "update"], &description(&args.fields))?;
    let (spec, template) = write_operation(&resources::PRODUCTS, Verb::Update)?;
    let flags = product_flags(&args.fields, args.external_id.as_deref());
    let attributes = prepare(spec, spec.required, args.data.as_deref(), &flags, ctx)?;
    if attributes.is_empty() {
        // Пустой PUT ничего не меняет, но сервер сбрасывает категорию (стейджинг, 2026-09-24; P5).
        return Err(CliError::usage(
            "nothing_to_update",
            "нечего менять: не передан ни один атрибут",
        )
        .with_hint("передайте флаг атрибута (--price, --available, …) или ключи в --data"));
    }
    // Смена external_id: повтор по старому id после потерянного ответа дал бы ложный 404 (P7).
    let renamed = attributes
        .get("external_id")
        .and_then(Value::as_str)
        .filter(|new| *new != args.id);
    let (replay, verify) = match renamed {
        Some(new) if check_id(new, "external_id").is_ok() => (
            Replay::OnlyIfUnprocessed,
            format!(
                "проверьте, переименован ли товар: aplaut products get {}",
                shell_word(new)
            ),
        ),
        Some(_) => (
            Replay::OnlyIfUnprocessed,
            "проверьте товар в личном кабинете, прежде чем повторять".to_string(),
        ),
        None => (Replay::Safe, String::new()),
    };
    let path = template.replace("{id}", &http::path_segment(&args.id));
    let submission = Submission {
        request: write::request(spec, path, attributes),
        replay,
        verify,
        outcome: "updated",
    };
    execute(submission, args.dry.dry_run, ctx, |updated| {
        format!("Товар обновлён: id {}", id_of(updated))
    })
}

/// 422 `external_id is already taken`: товар уже есть — его меняет `update`.
fn already_taken(err: CliError, external_id: &str) -> CliError {
    if err.code != "validation_failed" || err.field.as_deref() != Some("external_id") {
        return err;
    }
    let hint = format!(
        "товар с external_id {external_id} уже есть — изменить его: aplaut products update {} …",
        shell_word(external_id)
    );
    err.with_hint(hint)
}

/// Флаги товара → атрибуты схемы POST /products (§1).
fn product_flags<'a>(fields: &'a ProductFields, external_id: Option<&'a str>) -> Vec<Flag<'a>> {
    vec![
        ("external_id", external_id),
        ("name", fields.name.as_deref()),
        ("url", fields.url.as_deref()),
        ("price", fields.price.as_deref()),
        ("available", fields.available.as_deref()),
        ("description", fields.description.as_deref()),
        ("group_id", fields.group_id.as_deref()),
        ("category_id", fields.category_id.as_deref()),
        ("category_name", fields.category_name.as_deref()),
        ("brand_name", fields.brand_name.as_deref()),
    ]
}

/// Свободный текст товара — только описание (`allow_hyphen_values`).
fn description(fields: &ProductFields) -> [Flag<'_>; 1] {
    [("description", fields.description.as_deref())]
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use super::*;
    use crate::cli::{Cli, Command};

    fn products_verb(args: &[&str]) -> ProductsVerb {
        match Cli::try_parse_from(args).unwrap().command {
            Command::Products { verb } => verb,
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn every_flag_is_an_attribute_of_the_spec_schemas() {
        let ProductsVerb::Create(args) = products_verb(&["aplaut", "products", "create"]) else {
            panic!("create");
        };
        let flags = product_flags(&args.fields, None);
        for verb in [Verb::Create, Verb::Update] {
            let (spec, _) = write_operation(&resources::PRODUCTS, verb).unwrap();
            for (name, _) in &flags {
                assert!(spec.attribute(name).is_some(), "{verb:?} --{name}");
            }
        }
    }
}
