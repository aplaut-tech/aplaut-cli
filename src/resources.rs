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

    /// Глагол меняет данные в API: инструмент MCP — только с `--allow-writes` (M5).
    pub fn writes(self) -> bool {
        match self {
            Verb::Scroll | Verb::Get => false,
            Verb::Create | Verb::Comment | Verb::Update => true,
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

/// Как назвать объект ресурса в сообщениях. Все — мужского рода: «создан», «обновлён».
#[derive(Debug, Clone, Copy)]
pub struct Noun {
    /// «отзыв»
    pub one: &'static str,
    /// «отзыва»: «Отзыва crm-4211 нет»
    pub genitive: &'static str,
}

#[derive(Debug)]
pub struct Resource {
    pub name: &'static str,
    pub records_type: &'static str,
    pub verbs: &'static [Verb],
    pub noun: Noun,
}

pub static REVIEWS: Resource = Resource {
    name: "reviews",
    records_type: "reviews",
    verbs: &[
        Verb::Scroll,
        Verb::Get,
        Verb::Create,
        Verb::Comment,
        Verb::Update,
    ],
    noun: Noun {
        one: "отзыв",
        genitive: "отзыва",
    },
};
pub static PRODUCTS: Resource = Resource {
    name: "products",
    records_type: "products",
    verbs: &[Verb::Scroll, Verb::Get, Verb::Create, Verb::Update],
    noun: Noun {
        one: "товар",
        genitive: "товара",
    },
};
pub static QUESTIONS: Resource = Resource {
    name: "questions",
    records_type: "questions",
    verbs: &[Verb::Scroll, Verb::Get, Verb::Create, Verb::Update],
    noun: Noun {
        one: "вопрос",
        genitive: "вопроса",
    },
};

/// Клиенты: scroll API не поддерживает (спека writes-and-exports, «Вне объёма»).
pub static CONSUMERS: Resource = Resource {
    name: "consumers",
    records_type: "consumers",
    verbs: &[Verb::Get, Verb::Create, Verb::Update],
    noun: Noun {
        one: "клиент",
        genitive: "клиента",
    },
};

/// Заказы: scroll API не поддерживает; внешний id — `number` (спека writes-and-exports §4).
pub static ORDERS: Resource = Resource {
    name: "orders",
    records_type: "orders",
    verbs: &[Verb::Get, Verb::Create, Verb::Update],
    noun: Noun {
        one: "заказ",
        genitive: "заказа",
    },
};

pub static ALL: &[&Resource] = &[&REVIEWS, &PRODUCTS, &QUESTIONS, &CONSUMERS, &ORDERS];

/// Команды без операции Platform API: локальные файлы, самообновление (`self update` ходит в
/// GitHub Releases) и MCP-сервер (его инструменты вызывают остальные команды).
pub static LOCAL_COMMANDS: &[&str] = &[
    "auth login",
    "auth logout",
    "profile list",
    "profile get",
    "profile set",
    "profile delete",
    "profile edit",
    "self update",
    "mcp",
];

pub fn find(name: &str) -> Option<&'static Resource> {
    ALL.iter().copied().find(|r| r.name == name)
}
