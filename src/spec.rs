//! Таблицы из `spec/api.yaml`, сгенерированные `build.rs`.

pub struct ScrollSpec {
    pub records_type: &'static str,
    pub filters: &'static [&'static str],
    pub includes: &'static [&'static str],
}

/// Тип атрибута в схеме тела запроса.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AttrType {
    String,
    Number,
    Integer,
    Boolean,
    Array,
    Object,
}

/// Атрибут тела запроса записи: то, что CLI проверяет до сети (спека reviews-write §3).
#[derive(Debug)]
pub struct AttributeSpec {
    pub name: &'static str,
    pub ty: AttrType,
    pub enum_values: &'static [&'static str],
    pub minimum: Option<f64>,
    pub maximum: Option<f64>,
    pub format: Option<&'static str>,
    /// Тип элементов массива.
    pub item_type: Option<AttrType>,
}

/// Схема тела операции записи: `{"data":{"type":…,"attributes":{…}}}`.
#[derive(Debug)]
pub struct WriteSpec {
    pub method: &'static str,
    pub path: &'static str,
    /// `type` документа JSON:API: `reviews`, `comments`.
    pub resource_type: &'static str,
    pub required: &'static [&'static str],
    pub attributes: &'static [AttributeSpec],
}

impl WriteSpec {
    pub fn attribute(&self, name: &str) -> Option<&'static AttributeSpec> {
        self.attributes.iter().find(|a| a.name == name)
    }
}

include!(concat!(env!("OUT_DIR"), "/spec_tables.rs"));

pub fn scroll_spec(records_type: &str) -> Option<&'static ScrollSpec> {
    SCROLL.iter().find(|s| s.records_type == records_type)
}

pub fn has_operation(method: &str, path: &str) -> bool {
    OPERATIONS.iter().any(|(m, p)| *m == method && *p == path)
}

/// Допустимые `include` у `GET /{records_type}/{id}`; `None` — такой операции в спеке нет.
pub fn get_includes(records_type: &str) -> Option<&'static [&'static str]> {
    GET_INCLUDES
        .iter()
        .find(|(t, _)| *t == records_type)
        .map(|(_, includes)| *includes)
}

pub fn write_spec(method: &str, path: &str) -> Option<&'static WriteSpec> {
    WRITES.iter().find(|w| w.method == method && w.path == path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scroll_tables_come_from_spec() {
        let reviews = scroll_spec("reviews").expect("reviews");
        // Сервер (стейджинг, 2026-09-23) возвращает для reviews ровно этот набор из 17 полей.
        let mut filters = reviews.filters.to_vec();
        filters.sort_unstable();
        assert_eq!(
            filters,
            vec![
                "brand_id",
                "category_id",
                "context_type",
                "created_at",
                "featured",
                "lang",
                "order_number",
                "origin",
                "product_group_id",
                "product_id",
                "published_at",
                "rating",
                "recommended",
                "state",
                "syndication_source",
                "updated_at",
                "verified",
            ]
        );
        assert_eq!(
            reviews.includes,
            &["author", "product", "comments", "state_changes"]
        );
        assert_eq!(
            scroll_spec("products").unwrap().includes,
            &["reviews_summary_item"]
        );
        assert!(scroll_spec("questions")
            .unwrap()
            .filters
            .contains(&"has_published_answers"));
        assert!(scroll_spec("orders").is_none());
    }

    #[test]
    fn scroll_parameters_and_operations() {
        assert_eq!(SPEC_VERSION, "4.1.0");
        assert_eq!(SCROLL_SORTS, &["updated_at:asc", "created_at:asc"]);
        assert_eq!(SCROLL_SORT_DEFAULT, "updated_at:asc");
        assert_eq!(
            (
                SCROLL_PER_PAGE_MIN,
                SCROLL_PER_PAGE_MAX,
                SCROLL_PER_PAGE_DEFAULT
            ),
            (1, 100, 100)
        );
        assert!(has_operation("GET", "/scroll/{records_type}"));
        assert!(has_operation("PUT", "/reviews/{id}"));
        assert!(!has_operation("GET", "/webhooks"));
    }

    #[test]
    fn get_includes_come_from_spec() {
        assert_eq!(
            get_includes("reviews"),
            Some(&["author", "product", "comments", "state_changes"][..])
        );
        assert_eq!(
            get_includes("products"),
            Some(
                &[
                    "reviews_summary_item",
                    "reviews",
                    "questions",
                    "brand",
                    "category"
                ][..]
            )
        );
        assert_eq!(
            get_includes("questions"),
            Some(&["author", "product", "answers"][..])
        );
        assert_eq!(
            get_includes("users"),
            Some(&[][..]),
            "GET есть, include нет"
        );
        assert_eq!(get_includes("scroll"), None);
    }

    #[test]
    fn write_schemas_come_from_spec() {
        let review = write_spec("POST", "/reviews").expect("POST /reviews");
        assert_eq!(review.resource_type, "reviews");
        let mut required = review.required.to_vec();
        required.sort_unstable();
        assert_eq!(required, ["body", "rating"]);
        let rating = review.attribute("rating").unwrap();
        assert_eq!(
            (rating.ty, rating.minimum, rating.maximum),
            (AttrType::Number, Some(1.0), Some(5.0))
        );
        assert_eq!(
            review.attribute("state").unwrap().enum_values,
            &["published", "waiting", "banned", "held"]
        );
        assert_eq!(
            review.attribute("photos").unwrap().item_type,
            Some(AttrType::String)
        );
        assert_eq!(
            review.attribute("rating_details").unwrap().item_type,
            Some(AttrType::Object)
        );
        assert_eq!(
            review.attribute("created_at").unwrap().format,
            Some("date-time")
        );
        assert_eq!(
            review.attribute("hide_my_data").unwrap().ty,
            AttrType::Boolean
        );
        assert_eq!(review.attribute("likes").unwrap().ty, AttrType::Integer);
        assert_eq!(
            review.attribute("custom_attributes").unwrap().ty,
            AttrType::Object
        );
        assert!(
            review.attribute("context_type").is_none(),
            "только в ответе"
        );
        let comment = write_spec("POST", "/reviews/{id}/relationships/comments").expect("comments");
        assert_eq!(
            (comment.resource_type, comment.required),
            ("comments", &["text"][..])
        );
        assert_eq!(
            comment.attribute("state").unwrap().enum_values,
            &["published", "waiting", "banned"]
        );
        assert_eq!(
            comment.attribute("files").unwrap().item_type,
            Some(AttrType::Object)
        );
        assert!(write_spec("PUT", "/reviews/{id}").is_none());
    }

    /// Стейджинг, 2026-09-24 (спека products-write §6): PUT частичный и принимает атрибуты создания.
    #[test]
    fn product_write_schemas_follow_staging() {
        let create = write_spec("POST", "/products").expect("POST /products");
        assert_eq!(create.resource_type, "products");
        let mut required = create.required.to_vec();
        required.sort_unstable();
        assert_eq!(
            required,
            ["external_id", "name"],
            "url добавляет команда (P3)"
        );
        assert_eq!(create.attribute("available").unwrap().ty, AttrType::Boolean);
        assert_eq!(create.attribute("price").unwrap().ty, AttrType::Number);
        assert_eq!(
            create.attribute("category_names").unwrap().item_type,
            Some(AttrType::String)
        );
        let update = write_spec("PUT", "/products/{id}").expect("PUT /products/{id}");
        assert_eq!(
            (update.resource_type, update.required),
            ("products", &[][..])
        );
        assert!(update.attribute("category_name").is_some());
        assert!(update.attribute("brand_name").is_some());
        assert!(
            update.attribute("rating").is_none() && update.attribute("reviews_count").is_none()
        );
    }
}
