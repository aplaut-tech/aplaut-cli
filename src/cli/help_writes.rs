//! Справка команд записи из спеки writes-and-exports (agent mode §6): примеры, поля JSON (--json),
//! ошибки и коды выхода.

pub(super) const REVIEWS_UPDATE_AFTER_LONG_HELP: &str = "\
Примеры:
  aplaut reviews update crm-4211 --state banned
  aplaut reviews update crm-4211 --state published -n       # проверить, что отзыв есть, и показать запрос

  # Синхронизация из внешней системы: создать отзыв, если его ещё нет (external_id — ID).
  aplaut reviews update crm-4211 --rating 5 --body \"Спасибо!\" --upsert --json

  # Для агента: частичное обновление JSON-объектом; в custom_attributes null удаляет ключ.
  echo '{\"tags\":[\"Featured\"],\"custom_attributes\":{\"region\":66,\"old\":null}}' \\
    | aplaut reviews update crm-4211 --data - --json

ID — внутренний идентификатор или external_id (без «.» и «/»). Меняются только переданные атрибуты;
custom_attributes сливаются с текущими. Сначала CLI проверяет запросом GET, что отзыв есть: API на
PUT с неизвестным id создаёт новый отзыв, и опечатка в ID опубликовала бы его. Нет отзыва —
not_found (код 5), ничего не создано. --upsert пропускает проверку: нет отзыва — он создаётся с
external_id = ID (нужны rating и текст, как у create). external_id не меняется (сервер его
игнорирует), null атрибут не очищает — оба отвергаются до отправки. С -n (--dry-run) токен и наличие
отзыва проверяются, PUT не отправляется.

Повторы: правка идемпотентна и повторяется после таймаута и 5xx, как чтение.

JSON (--json):
  {\"ok\":true,\"command\":\"reviews.update\",\"cli_version\":…,\"dry_run\":false,
   \"result\":{\"request\":{\"method\":\"PUT\",\"path\":\"/reviews/<id>\",
   \"body\":{\"data\":{\"type\":\"reviews\",\"attributes\":{…}}}},
   \"updated\":{\"id\",\"type\":\"reviews\",\"attributes\":{…}},\"created\":false},\"warnings\":[]}
  created: true — отзыв создан этим вызовом (--upsert, ответ 201; после повтора — false).
  С -n — тот же request, \"updated\":null, \"exists\":true|false и \"dry_run\":true.

Ошибки: invalid_id, nothing_to_update, unknown_attribute, invalid_attribute, invalid_data,
stdin_conflict, stdin_is_terminal, usage (флаг вместо текста), not_found (отзыва нет, без --upsert),
validation_failed (422), bad_response, no_token, invalid_token, unauthorized, forbidden,
rate_limited (retry_after — сколько секунд ждать), server_error, network_error, timeout.

Коды выхода: 0 — обновлён или создан (с -n — план); 2 — ошибка во входных данных, до сети;
3 — нет токена или он отклонён; 5 — отзыва нет (без --upsert); 7 — rate limit, повторы исчерпаны;
1 — прочие ошибки.";

pub(super) const QUESTIONS_CREATE_AFTER_LONG_HELP: &str = "\
Примеры:
  aplaut questions create --text \"Есть размер M?\" --product-id 444772 --author-name \"Анна\" --external-id crm-q-17
  aplaut questions create --text \"Есть доставка в Казань?\" -n   # показать запрос, не отправляя

  # Для агента: атрибуты — JSON-объектом из stdin, итог — одной строкой JSON в stdout.
  echo '{\"text\":\"Есть размер M?\",\"product_id\":\"444772\",\"tags\":[\"размер\"],\"external_id\":\"crm-q-17\"}' \\
    | aplaut questions create --data - --json

Атрибуты — из схемы тела POST /questions в спеке; флаги перекрывают одноимённые ключи --data.
Обязателен text. Без product_id вопрос создаётся о компании. tags, files, hide_my_data, даты — через
--data. С -n (--dry-run) токен проверяется, но запросов нет.

Повторы: CLI повторяет запрос сам, только если сервер его точно не обработал. После
request_outcome_unknown с --external-id повтор безопасен: второй вопрос с тем же external_id сервер
не создаст (422, is already taken); проверить — aplaut questions get <external_id>. Без --external-id
проверьте в личном кабинете, прежде чем повторять.

JSON (--json):
  {\"ok\":true,\"command\":\"questions.create\",\"cli_version\":…,\"dry_run\":false,
   \"result\":{\"request\":{\"method\":\"POST\",\"path\":\"/questions\",
   \"body\":{\"data\":{\"type\":\"questions\",\"attributes\":{…}}}},
   \"created\":{\"id\",\"type\":\"questions\",\"attributes\":{…}}},\"warnings\":[]}
  С -n — тот же request, \"created\":null и \"dry_run\":true.

Ошибки: unknown_attribute (в hint — допустимые), invalid_attribute, missing_attribute,
invalid_data, stdin_conflict, stdin_is_terminal, usage (флаг вместо текста),
validation_failed (422; external_id is already taken — вопрос уже есть, в hint — questions update),
request_outcome_unknown, bad_response, no_token, invalid_token, unauthorized, forbidden,
rate_limited (retry_after — сколько секунд ждать), server_error, network_error.

Коды выхода: 0 — создан (с -n — план); 2 — ошибка во входных данных, до сети; 3 — нет токена
или он отклонён; 7 — rate limit, повторы исчерпаны; 1 — прочие ошибки, в том числе
request_outcome_unknown.";

pub(super) const QUESTIONS_UPDATE_AFTER_LONG_HELP: &str = "\
Примеры:
  aplaut questions update crm-q-17 --state published
  aplaut questions update crm-q-17 --text \"Есть размер M и L?\" -n    # проверить, что вопрос есть, и показать запрос

  # Синхронизация: создать вопрос, если его ещё нет (external_id — ID).
  aplaut questions update crm-q-17 --text \"Есть размер M?\" --product-id 444772 --upsert --json

ID — внутренний идентификатор или external_id (без «.» и «/»). Меняются только переданные атрибуты.
Сначала CLI проверяет запросом GET, что вопрос есть: API на PUT с неизвестным id создаёт новый
вопрос. Нет вопроса — not_found (код 5), ничего не создано; --upsert пропускает проверку и создаёт
вопрос с external_id = ID (нужен text).
У вопроса о товаре CLI сам добавляет в тело product_id из текущей записи, если --product-id не
передан: без него сервер сохраняет правку, но отвечает 404. Поэтому с --upsert без --product-id
GET тоже делается. Подставленный product_id виден в result.request.
external_id не меняется (сервер его игнорирует), null атрибут не очищает — оба отвергаются до
отправки. С -n (--dry-run) токен и наличие вопроса проверяются, PUT не отправляется.

Повторы: правка идемпотентна и повторяется после таймаута и 5xx, как чтение.

JSON (--json):
  {\"ok\":true,\"command\":\"questions.update\",\"cli_version\":…,\"dry_run\":false,
   \"result\":{\"request\":{\"method\":\"PUT\",\"path\":\"/questions/<id>\",
   \"body\":{\"data\":{\"type\":\"questions\",\"attributes\":{…}}}},
   \"updated\":{\"id\",\"type\":\"questions\",\"attributes\":{…}},\"created\":false},\"warnings\":[]}
  created: true — вопрос создан этим вызовом (--upsert, ответ 201; после повтора — false).
  С -n — тот же request, \"updated\":null, \"exists\":true|false и \"dry_run\":true.

Ошибки: invalid_id, nothing_to_update, unknown_attribute, invalid_attribute, invalid_data,
stdin_conflict, stdin_is_terminal, usage (флаг вместо текста), not_found (вопроса нет, без --upsert),
validation_failed (422), bad_response, no_token, invalid_token, unauthorized, forbidden,
rate_limited (retry_after — сколько секунд ждать), server_error, network_error, timeout.

Коды выхода: 0 — обновлён или создан (с -n — план); 2 — ошибка во входных данных, до сети;
3 — нет токена или он отклонён; 5 — вопроса нет (без --upsert); 7 — rate limit, повторы
исчерпаны; 1 — прочие ошибки.";

pub(super) const CONSUMERS_CREATE_AFTER_LONG_HELP: &str = "\
Примеры:
  aplaut consumers create --external-id crm-c-42 --email anna@example.com --name \"Анна Петрова\"
  aplaut consumers create --external-id crm-c-42 --phone \"+7 900 000-00-00\" -n   # показать запрос, не отправляя

  # Для агента: атрибуты — JSON-объектом из stdin, итог — одной строкой JSON в stdout.
  echo '{\"external_id\":\"crm-c-42\",\"email\":\"anna@example.com\",\"custom_attributes\":{\"segment\":\"vip\"}}' \\
    | aplaut consumers create --data - --json

Данные клиентов — персональные: e-mail, телефон, имя.
Атрибуты — из схемы тела POST /consumers в спеке; флаги перекрывают одноимённые ключи --data.
Нужен хотя бы один из external_id, email, phone: сервер создал бы клиента и без них (спека к тому
же требует name), но такого клиента потом не найти и не отличить от дубля — CLI отказывает до
сети. Сервер приводит name к виду «Анна Петрова» и сбрасывает first_name, из телефона оставляет
цифры. custom_attributes и даты — через --data. С -n (--dry-run) токен проверяется, но запросов нет.

Повторы: CLI повторяет запрос сам, только если сервер его точно не обработал. После
request_outcome_unknown с --external-id повтор безопасен: второго клиента с тем же external_id
(и с тем же e-mail) сервер не создаст; проверить — aplaut consumers get <external_id>.

JSON (--json):
  {\"ok\":true,\"command\":\"consumers.create\",\"cli_version\":…,\"dry_run\":false,
   \"result\":{\"request\":{\"method\":\"POST\",\"path\":\"/consumers\",
   \"body\":{\"data\":{\"type\":\"consumers\",\"attributes\":{…}}}},
   \"created\":{\"id\",\"type\":\"consumers\",\"attributes\":{…}}},\"warnings\":[]}
  С -n — тот же request, \"created\":null и \"dry_run\":true.

Ошибки: unknown_attribute (в hint — допустимые), missing_attribute (нет ни external_id, ни email,
ни phone), invalid_attribute, invalid_data, stdin_conflict, stdin_is_terminal, validation_failed
(422; external_id или email is already taken — клиент уже есть, в hint — что делать),
request_outcome_unknown, bad_response, no_token, invalid_token, unauthorized, forbidden,
rate_limited (retry_after — сколько секунд ждать), server_error, network_error.

Коды выхода: 0 — создан (с -n — план); 2 — ошибка во входных данных, до сети; 3 — нет токена
или он отклонён; 7 — rate limit, повторы исчерпаны; 1 — прочие ошибки, в том числе
request_outcome_unknown.";

pub(super) const CONSUMERS_UPDATE_AFTER_LONG_HELP: &str = "\
Примеры:
  aplaut consumers update crm-c-42 --unsubscribed true
  aplaut consumers update crm-c-42 --first-name \"Анна\" -n          # проверить, что клиент есть, и показать запрос

  # Синхронизация из CRM: создать клиента, если его ещё нет (external_id — ID); e-mail и телефон
  # применяются только при создании.
  aplaut consumers update crm-c-42 --email anna@example.com --name \"Анна Петрова\" --upsert --json

  # Для агента: null очищает атрибут, в custom_attributes — удаляет ключ.
  echo '{\"first_name\":null,\"custom_attributes\":{\"segment\":\"vip\",\"old\":null}}' \\
    | aplaut consumers update crm-c-42 --data - --json

Данные клиентов — персональные: e-mail, телефон, имя.
ID — внутренний идентификатор или external_id (без «.» и «/»). Меняются только переданные атрибуты,
custom_attributes сливаются с текущими. Сначала CLI проверяет запросом GET, что клиент есть: API на
PUT с неизвестным id создаёт нового клиента. Нет клиента — not_found (код 5), ничего не создано;
--upsert пропускает проверку и создаёт клиента с external_id = ID.
E-mail и телефон сервер задаёт только при создании, а у существующего клиента молча оставляет
прежними: без --upsert --email и --phone — ошибка до отправки. name сервер приводит к виду
«Анна Петрова» и сбрасывает first_name. external_id не меняется. С -n (--dry-run) токен и наличие
клиента проверяются, PUT не отправляется.

Повторы: правка идемпотентна и повторяется после таймаута и 5xx, как чтение.

JSON (--json):
  {\"ok\":true,\"command\":\"consumers.update\",\"cli_version\":…,\"dry_run\":false,
   \"result\":{\"request\":{\"method\":\"PUT\",\"path\":\"/consumers/<id>\",
   \"body\":{\"data\":{\"type\":\"consumers\",\"attributes\":{…}}}},
   \"updated\":{\"id\",\"type\":\"consumers\",\"attributes\":{…}},\"created\":false},\"warnings\":[]}
  created: true — клиент создан этим вызовом (--upsert, ответ 201; после повтора — false).
  С -n — тот же request, \"updated\":null, \"exists\":true|false и \"dry_run\":true.

Ошибки: invalid_id, nothing_to_update, unknown_attribute, invalid_attribute (в том числе email и
phone без --upsert), invalid_data, stdin_conflict, stdin_is_terminal, not_found (клиента нет, без
--upsert), validation_failed (422), bad_response, no_token, invalid_token, unauthorized, forbidden,
rate_limited (retry_after — сколько секунд ждать), server_error, network_error, timeout.

Коды выхода: 0 — обновлён или создан (с -n — план); 2 — ошибка во входных данных, до сети;
3 — нет токена или он отклонён; 5 — клиента нет (без --upsert); 7 — rate limit, повторы
исчерпаны; 1 — прочие ошибки.";

pub(super) const ORDERS_CREATE_AFTER_LONG_HELP: &str = "\
Примеры:
  aplaut orders create --number 31337 --consumer-email anna@example.com --consumer-name \"Анна\" -n

  # Строки заказа — только через --data: массив объектов удобнее JSON, чем флагами.
  echo '{\"order_lines\":[{\"product_id\":\"444772\",\"name\":\"Диск\",\"price\":5990}],\"details\":{\"region\":74}}' \\
    | aplaut orders create --number 31337 --consumer-email anna@example.com --data - --json

Данные клиента в заказе — персональные: e-mail, телефон, имя.
Атрибуты — из схемы тела POST /orders в спеке; флаги перекрывают одноимённые ключи --data.
Обязательны number (внешний id заказа) и хотя бы одно из consumer_email, consumer_phone (так
проверяет сервер; спека требует ещё consumer_name и order_lines). Строка заказа —
{\"product_id\",\"name\",\"price\"}, product_id обязателен: сервер строку без него принимает молча, CLI —
нет. С новым e-mail или телефоном сервер создаёт и клиента. details — через --data.
С -n (--dry-run) токен проверяется, но запросов нет.

Повторы: CLI повторяет запрос сам, только если сервер его точно не обработал. После
request_outcome_unknown повтор безопасен: второй заказ с тем же number сервер не создаст (422, is
already taken); проверить — aplaut orders get <number>.

JSON (--json):
  {\"ok\":true,\"command\":\"orders.create\",\"cli_version\":…,\"dry_run\":false,
   \"result\":{\"request\":{\"method\":\"POST\",\"path\":\"/orders\",
   \"body\":{\"data\":{\"type\":\"orders\",\"attributes\":{…}}}},
   \"created\":{\"id\",\"type\":\"orders\",\"attributes\":{…}}},\"warnings\":[]}
  С -n — тот же request, \"created\":null и \"dry_run\":true.

Ошибки: unknown_attribute (в hint — допустимые), invalid_attribute (в том числе строка order_lines
без product_id), missing_attribute, invalid_data, stdin_conflict, stdin_is_terminal,
validation_failed (422; number is already taken — заказ уже есть, в hint — orders update),
request_outcome_unknown, bad_response, no_token, invalid_token, unauthorized, forbidden,
rate_limited (retry_after — сколько секунд ждать), server_error, network_error.

Коды выхода: 0 — создан (с -n — план); 2 — ошибка во входных данных, до сети; 3 — нет токена
или он отклонён; 7 — rate limit, повторы исчерпаны; 1 — прочие ошибки, в том числе
request_outcome_unknown (повтор безопасен).";

pub(super) const ORDERS_UPDATE_AFTER_LONG_HELP: &str = "\
Примеры:
  aplaut orders update 31337 --consumer-name \"Анна Петрова\"
  aplaut orders update 31337 --consumer-phone \"+7 900 000-00-00\" -n    # проверить, что заказ есть, и показать запрос

  # Корзина заменяется целиком: передайте все строки; details сливаются с текущими.
  echo '{\"order_lines\":[{\"product_id\":\"444772\",\"name\":\"Диск\",\"price\":5490}],\"details\":{\"payment_status\":\"paid\"}}' \\
    | aplaut orders update 31337 --data - --json

Данные клиента в заказе — персональные: e-mail, телефон, имя.
ID — внутренний идентификатор или номер заказа (number; без «.» и «/»). Меняются только переданные
атрибуты: order_lines сервер заменяет целиком, details сливает, null очищает атрибут. Сначала CLI
проверяет запросом GET, что заказ есть: API на PUT с неизвестным id создаёт новый заказ. Нет заказа —
not_found (код 5), ничего не создано; --upsert пропускает проверку и создаёт заказ с number = ID
(нужен e-mail или телефон клиента). number не меняется (сервер его игнорирует) — отвергается до
отправки. Смена consumer_email меняет атрибут заказа, связанный клиент остаётся прежним.
С -n (--dry-run) токен и наличие заказа проверяются, PUT не отправляется.

Повторы: правка идемпотентна и повторяется после таймаута и 5xx, как чтение.

JSON (--json):
  {\"ok\":true,\"command\":\"orders.update\",\"cli_version\":…,\"dry_run\":false,
   \"result\":{\"request\":{\"method\":\"PUT\",\"path\":\"/orders/<id>\",
   \"body\":{\"data\":{\"type\":\"orders\",\"attributes\":{…}}}},
   \"updated\":{\"id\",\"type\":\"orders\",\"attributes\":{…}},\"created\":false},\"warnings\":[]}
  created: true — заказ создан этим вызовом (--upsert, ответ 201; после повтора — false).
  С -n — тот же request, \"updated\":null, \"exists\":true|false и \"dry_run\":true.

Ошибки: invalid_id, nothing_to_update, unknown_attribute, invalid_attribute (в том числе number и
строка order_lines без product_id), invalid_data, stdin_conflict, stdin_is_terminal, not_found
(заказа нет, без --upsert), validation_failed (422), bad_response, no_token, invalid_token,
unauthorized, forbidden, rate_limited (retry_after — сколько секунд ждать), server_error,
network_error, timeout.

Коды выхода: 0 — обновлён или создан (с -n — план); 2 — ошибка во входных данных, до сети;
3 — нет токена или он отклонён; 5 — заказа нет (без --upsert); 7 — rate limit, повторы
исчерпаны; 1 — прочие ошибки.";
