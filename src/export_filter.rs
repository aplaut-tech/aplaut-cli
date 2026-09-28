//! `--filter` у `exports create` (спека writes-and-exports §9, фаза 2). Синтаксис и параметры — как у
//! `scroll` того же типа, но сервер экспорта понимает только хэш `{"поле": {"оператор": значение}}` по полям
//! индекса. Строку он разбирает лишь у отзывов, у товаров молча отбрасывает, у вопросов падает, а клиенты и
//! заказы фильтр не читают вовсе (код сервера, 2026-09-28). Поэтому CLI сам переводит выражение в хэш — с
//! правилами каждого типа.

use serde_json::{Map, Value};

use crate::error::CliError;
use crate::filter::{self, Clause, Op};
use crate::spec;

/// Операторы хэша у сервера (код сервера, 2026-09-28): отзывы — полный набор без `exists`, вопросы — без
/// `ne`, товары — только сравнения и `eq`.
const REVIEWS_OPS: &[Op] = &[Op::Eq, Op::Neq, Op::In, Op::Gt, Op::Gte, Op::Lt, Op::Lte];
const QUESTIONS_OPS: &[Op] = &[Op::Eq, Op::In, Op::Gt, Op::Gte, Op::Lt, Op::Lte];
const PRODUCTS_OPS: &[Op] = &[Op::Eq, Op::Gt, Op::Gte, Op::Lt, Op::Lte];
/// Поле индекса, если оно не совпадает с параметром `scroll`: товар принадлежит ветке категорий.
const PRODUCTS_FIELDS: &[(&str, &str)] = &[("category_id", "category_ids")];
/// `rating` в индексе нормализован: звёзды ÷ 5 (`scroll` переводит сам, экспорт — нет).
const RATING_SCALE: f64 = 5.0;

/// `--filter` → `search_options.filter` задачи экспорта.
pub fn search_filter(records_type: &str, expr: &str) -> Result<Value, CliError> {
    let (ops, fields) = match records_type {
        "reviews" => (REVIEWS_OPS, &[][..]),
        "questions" => (QUESTIONS_OPS, &[][..]),
        "products" => (PRODUCTS_OPS, PRODUCTS_FIELDS),
        other => return Err(unsupported_type(other)),
    };
    let allowed = spec::scroll_spec(records_type)
        .expect("у reviews, products, questions есть scroll")
        .filters;
    filter::check_filter(expr, allowed)?;
    let mut out = Map::new();
    for clause in filter::parse_filter(expr)? {
        if !ops.contains(&clause.op) {
            return Err(unsupported_op(records_type, &clause, ops));
        }
        let field = fields
            .iter()
            .find(|(param, _)| *param == clause.param)
            .map_or(clause.param.as_str(), |(_, index)| *index);
        let operator = match clause.op {
            Op::Neq => "ne",
            other => other.name(),
        };
        let value = operand(&clause)?;
        let entry = out
            .entry(field.to_string())
            .or_insert_with(|| Value::Object(Map::new()));
        let operators = entry.as_object_mut().expect("поле фильтра — объект");
        if operators.contains_key(operator) {
            return Err(invalid(format!(
                "оператор «{}» у «{}» указан дважды",
                clause.op.name(),
                clause.param
            ))
            .with_hint("объедините условия: один оператор на поле, диапазон — gte и lt"));
        }
        operators.insert(operator.to_string(), value);
    }
    Ok(Value::Object(out))
}

/// Значение для сервера: `in` — строкой через `|`, `rating` — долей от 5. У `in`-списка `rating`
/// джойнится через `Display` `f64` (`1.0` → `"1"`), а не через JSON-число (`serde_json` печатает
/// `"1.0"`) — сервер сравнивает строки, и лишний `.0` не совпал бы со звёздами.
fn operand(clause: &Clause) -> Result<Value, CliError> {
    if clause.param == "rating" {
        let stars: Vec<f64> = clause
            .values
            .iter()
            .map(|stars| scaled_rating(stars))
            .collect::<Result<_, CliError>>()?;
        if clause.op == Op::In {
            let joined = stars
                .iter()
                .map(f64::to_string)
                .collect::<Vec<_>>()
                .join("|");
            return Ok(Value::String(joined));
        }
        let stars = *stars.first().expect("у оператора есть значение");
        return Ok(Value::from(stars));
    }
    if clause.op == Op::In {
        return Ok(Value::String(clause.values.join("|")));
    }
    let value = clause.values.first().expect("у оператора есть значение");
    Ok(Value::String(value.clone()))
}

fn scaled_rating(stars: &str) -> Result<f64, CliError> {
    stars
        .trim()
        .parse::<f64>()
        .map(|stars| stars / RATING_SCALE)
        .map_err(|_| {
            invalid(format!("rating: «{stars}» — не число звёзд"))
                .with_hint("например rating:gte:4")
        })
}

fn unsupported_type(records_type: &str) -> CliError {
    let (message, hint) = match records_type {
        "survey_responses" => (
            "у survey_responses нет --filter: выборку задаёт --survey-id".to_string(),
            "свой фильтр — хэшем в search_options через --data".to_string(),
        ),
        other => (
            format!("сервер не применяет фильтр к экспорту {other}: выгрузились бы все записи"),
            "уберите --filter или выгрузите reviews, products, questions".to_string(),
        ),
    };
    invalid(message).with_hint(hint)
}

fn unsupported_op(records_type: &str, clause: &Clause, ops: &[Op]) -> CliError {
    let names: Vec<&str> = ops.iter().map(|op| op.name()).collect();
    invalid(format!(
        "оператор «{}» в «{}»: экспорт {records_type} его не поддерживает",
        clause.op.name(),
        clause.param
    ))
    .with_hint(format!(
        "операторы экспорта {records_type}: {}",
        names.join(", ")
    ))
}

fn invalid(message: impl Into<String>) -> CliError {
    CliError::usage("invalid_filter", message).with_field("filter")
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn code_and_field(err: CliError) -> (String, Option<String>) {
        (err.code, err.field)
    }

    #[test]
    fn reviews_filter_becomes_the_export_hash() {
        let filter = search_filter(
            "reviews",
            "updated_at:gte:2026-09-01T00:00:00Z,rating:gte:4,state:neq:published",
        )
        .unwrap();
        assert_eq!(
            filter,
            json!({
                "updated_at": {"gte": "2026-09-01T00:00:00Z"},
                "rating": {"gte": 0.8},
                "state": {"ne": "published"}
            })
        );
    }

    #[test]
    fn in_lists_are_joined_with_a_bar_and_rating_is_scaled() {
        assert_eq!(
            search_filter("reviews", "state:in:banned|waiting").unwrap(),
            json!({"state": {"in": "banned|waiting"}})
        );
        assert_eq!(
            search_filter("reviews", "rating:in:4|5").unwrap(),
            json!({"rating": {"in": "0.8|1"}})
        );
        assert_eq!(
            search_filter("reviews", "rating:eq:1").unwrap(),
            json!({"rating": {"eq": 0.2}})
        );
    }

    #[test]
    fn several_operators_of_one_field_share_an_object_but_a_repeat_is_refused() {
        assert_eq!(
            search_filter(
                "questions",
                "created_at:gte:2026-01-01T00:00:00Z,created_at:lt:2027-01-01T00:00:00Z"
            )
            .unwrap(),
            json!({"created_at": {"gte": "2026-01-01T00:00:00Z", "lt": "2027-01-01T00:00:00Z"}})
        );
        let err = search_filter(
            "reviews",
            "created_at:gte:2026-01-01,created_at:gte:2026-02-01",
        )
        .unwrap_err();
        assert_eq!(
            code_and_field(err),
            ("invalid_filter".into(), Some("filter".into()))
        );
    }

    #[test]
    fn products_use_index_names_and_only_their_operators() {
        assert_eq!(
            search_filter("products", "category_id:eq:297,price:lte:1000").unwrap(),
            json!({"category_ids": {"eq": "297"}, "price": {"lte": "1000"}})
        );
        for expr in ["price:in:1|2", "available:neq:true"] {
            let err = search_filter("products", expr).unwrap_err();
            assert_eq!(err.code, "invalid_filter", "{expr}");
            assert!(err.hint.unwrap().contains("eq, gt, gte, lt, lte"), "{expr}");
        }
    }

    #[test]
    fn unsupported_operators_and_params_are_refused() {
        for (records_type, expr) in [
            ("questions", "state:neq:published"),
            ("reviews", "photos:exists:true"),
            ("reviews", "bogus_param:eq:1"),
            ("reviews", "rating:eq:five"),
        ] {
            let err = search_filter(records_type, expr).unwrap_err();
            assert_eq!(
                (err.code.as_str(), err.field.as_deref()),
                ("invalid_filter", Some("filter")),
                "{records_type} {expr}"
            );
        }
    }

    #[test]
    fn types_whose_export_ignores_the_filter_are_refused() {
        for records_type in ["consumers", "orders", "survey_responses"] {
            let err = search_filter(records_type, "created_at:gte:2026-01-01").unwrap_err();
            assert_eq!(err.code, "invalid_filter", "{records_type}");
            assert!(err.hint.is_some(), "{records_type}");
        }
        let err = search_filter("consumers", "created_at:gte:2026-01-01").unwrap_err();
        assert!(err.message.contains("не применяет"), "{}", err.message);
    }
}
