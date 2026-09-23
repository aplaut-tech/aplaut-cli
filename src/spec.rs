//! Таблицы из `spec/api.yaml`, сгенерированные `build.rs`.

pub struct ScrollSpec {
    pub records_type: &'static str,
    pub filters: &'static [&'static str],
    pub includes: &'static [&'static str],
}

include!(concat!(env!("OUT_DIR"), "/spec_tables.rs"));

pub fn scroll_spec(records_type: &str) -> Option<&'static ScrollSpec> {
    SCROLL.iter().find(|s| s.records_type == records_type)
}

pub fn has_operation(method: &str, path: &str) -> bool {
    OPERATIONS.iter().any(|(m, p)| *m == method && *p == path)
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
                "brand_id", "category_id", "context_type", "created_at", "featured", "lang",
                "order_number", "origin", "product_group_id", "product_id", "published_at",
                "rating", "recommended", "state", "syndication_source", "updated_at", "verified",
            ]
        );
        assert_eq!(reviews.includes, &["author", "product", "comments", "state_changes"]);
        assert_eq!(scroll_spec("products").unwrap().includes, &["reviews_summary_item"]);
        assert!(scroll_spec("questions").unwrap().filters.contains(&"has_published_answers"));
        assert!(scroll_spec("orders").is_none());
    }

    #[test]
    fn scroll_parameters_and_operations() {
        assert_eq!(SPEC_VERSION, "4.1.0");
        assert_eq!(SCROLL_SORTS, &["updated_at:asc", "created_at:asc"]);
        assert_eq!(SCROLL_SORT_DEFAULT, "updated_at:asc");
        assert_eq!((SCROLL_PER_PAGE_MIN, SCROLL_PER_PAGE_MAX, SCROLL_PER_PAGE_DEFAULT), (1, 100, 100));
        assert!(has_operation("GET", "/scroll/{records_type}"));
        assert!(has_operation("PUT", "/reviews/{id}"));
        assert!(!has_operation("GET", "/webhooks"));
    }
}
