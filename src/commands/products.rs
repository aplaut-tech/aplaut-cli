//! Запись у товаров: `create` и `update` (спека products-write). Чтение (`scroll`, `get`) —
//! общее с другими ресурсами, в `records`; общее для записи — в `writes`.

use serde_json::Value;

use super::writes::{check_texts, execute, id_of, prepare, write_operation, Submission};
use super::{check_id, records, Ctx, Outcome};
use crate::cli::{CreateProductArgs, ProductFields, ProductsVerb};
use crate::error::CliError;
use crate::http::Replay;
use crate::ops::write::{self, Flag};
use crate::resources::{self, Verb};
use crate::term::shell_word;

/// `url` сервер требует, хотя в `required` спеки его нет (стейджинг, 2026-09-24; P3).
const CREATE_ALSO_REQUIRED: &str = "url";

pub fn run(verb: ProductsVerb, ctx: &Ctx) -> Result<Outcome, CliError> {
    match verb {
        ProductsVerb::Records(verb) => records::run(&resources::PRODUCTS, verb, ctx),
        ProductsVerb::Create(args) => create(&args, ctx),
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
