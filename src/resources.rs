//! Таблица ресурсов: какие глаголы есть у ресурса и какой операции API они соответствуют.
//! Добавить ресурс = строка здесь + вариант в `cli::Command` (дизайн §10.3); тест
//! `spec_drift` проверяет, что операция существует в `spec/api.yaml`.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verb {
    Scroll,
}

impl Verb {
    pub fn name(self) -> &'static str {
        match self {
            Verb::Scroll => "scroll",
        }
    }

    pub fn operation(self) -> (&'static str, &'static str) {
        match self {
            Verb::Scroll => ("GET", "/scroll/{records_type}"),
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
    verbs: &[Verb::Scroll],
};
pub static PRODUCTS: Resource = Resource {
    name: "products",
    records_type: "products",
    verbs: &[Verb::Scroll],
};
pub static QUESTIONS: Resource = Resource {
    name: "questions",
    records_type: "questions",
    verbs: &[Verb::Scroll],
};

pub static ALL: &[&Resource] = &[&REVIEWS, &PRODUCTS, &QUESTIONS];

/// Команды без операции API: работают только с локальными файлами.
pub static LOCAL_COMMANDS: &[&str] = &[
    "auth login",
    "auth logout",
    "profile list",
    "profile get",
    "profile set",
    "profile delete",
    "profile edit",
];

pub fn find(name: &str) -> Option<&'static Resource> {
    ALL.iter().copied().find(|r| r.name == name)
}
