//! Таблица ресурсов: какие глаголы есть у ресурса и какой операции API они соответствуют.
//! Добавить ресурс = строка здесь + вариант в `cli::Command` (дизайн §10.3); тест
//! `spec_drift` проверяет, что операция существует в `spec/api.yaml`.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verb {
    Scroll,
    Get,
    Create,
    Comment,
    Update,
}

impl Verb {
    pub fn name(self) -> &'static str {
        match self {
            Verb::Scroll => "scroll",
            Verb::Get => "get",
            Verb::Create => "create",
            Verb::Comment => "comment",
            Verb::Update => "update",
        }
    }

    /// Метод и шаблон пути операции в `spec/api.yaml`.
    pub fn operation(self, resource: &Resource) -> (&'static str, String) {
        match self {
            Verb::Scroll => ("GET", "/scroll/{records_type}".to_string()),
            Verb::Get => ("GET", format!("/{}/{{id}}", resource.records_type)),
            Verb::Create => ("POST", format!("/{}", resource.records_type)),
            Verb::Comment => (
                "POST",
                format!("/{}/{{id}}/relationships/comments", resource.records_type),
            ),
            Verb::Update => ("PUT", format!("/{}/{{id}}", resource.records_type)),
        }
    }
}

#[derive(Debug)]
pub struct Resource {
    pub name: &'static str,
    pub records_type: &'static str,
    pub verbs: &'static [Verb],
}

pub static REVIEWS: Resource = Resource {
    name: "reviews",
    records_type: "reviews",
    verbs: &[Verb::Scroll, Verb::Get, Verb::Create, Verb::Comment],
};
pub static PRODUCTS: Resource = Resource {
    name: "products",
    records_type: "products",
    verbs: &[Verb::Scroll, Verb::Get, Verb::Create, Verb::Update],
};
pub static QUESTIONS: Resource = Resource {
    name: "questions",
    records_type: "questions",
    verbs: &[Verb::Scroll, Verb::Get],
};

pub static ALL: &[&Resource] = &[&REVIEWS, &PRODUCTS, &QUESTIONS];

/// Команды без операции Platform API: локальные файлы и самообновление (`self update` ходит в
/// GitHub Releases).
pub static LOCAL_COMMANDS: &[&str] = &[
    "auth login",
    "auth logout",
    "profile list",
    "profile get",
    "profile set",
    "profile delete",
    "profile edit",
    "self update",
];

pub fn find(name: &str) -> Option<&'static Resource> {
    ALL.iter().copied().find(|r| r.name == name)
}
