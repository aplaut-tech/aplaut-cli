//! Тексты справки: после флагов — примеры, поля JSON (--json), ошибки и коды выхода (agent mode §6).

pub(super) const ROOT_AFTER_HELP: &str = "\
Примеры:
  aplaut auth login
  aplaut reviews get crm-4211 --include comments --format jsonl
  aplaut reviews scroll --filter updated_at:gte:2024-01-01T00:00:00Z --format jsonl > reviews.jsonl
  aplaut products scroll --format csv --max-records 500 | tw

Документация: https://aplaut.com/docs/api-references/platform/
Поддержка: support@aplaut.com";

pub(super) const SCROLL_AFTER_HELP: &str = "\
Примеры:
  aplaut reviews scroll --filter updated_at:gte:2024-01-01T00:00:00Z --format jsonl > reviews.jsonl
  aplaut reviews scroll --filter updated_at:gte:2024-01-01T00:00:00Z --state reviews.state.json

Без --filter сервер отдаёт только записи, изменённые за последние 30 дней.";

pub(super) const SCROLL_AFTER_LONG_HELP: &str = "\
Примеры:
  # Полная выгрузка отзывов в JSONL для ClickHouse или DuckDB:
  aplaut reviews scroll --filter updated_at:gte:2000-01-01T00:00:00Z --format jsonl > reviews.jsonl

  # Просмотр в таблице:
  aplaut products scroll --format csv --max-records 500 | tw

  # Только нужные колонки CSV и в заданном порядке (<связь>.<атрибут> — из --include):
  aplaut reviews scroll --filter updated_at:gte:2024-01-01T00:00:00Z --include product --format csv --fields id,rating,body,product.name

  # Выгрузка с продолжением после сбоя (cron): повторный запуск той же команды продолжит с места остановки.
  aplaut reviews scroll --filter updated_at:gte:2024-01-01T00:00:00Z --state reviews.state.json --format jsonl >> reviews.jsonl

  # Для агента: данные — в файл, итог — одной строкой JSON в stderr.
  aplaut reviews scroll --filter updated_at:gte:2024-01-01T00:00:00Z --state reviews.state.json --format jsonl --json >> reviews.jsonl

Без --filter сервер отдаёт только записи, изменённые за последние 30 дней.

Доставка — «хотя бы один раз»: после сбоя последняя страница может прийти повторно,
убирайте дубли по id (например, ReplacingMergeTree).

JSON (--json): данные — в stdout в --format; итог — в stderr одной строкой:
  {\"ok\":true,\"command\":\"reviews.scroll\",\"cli_version\":…,\"dry_run\":false,
   \"result\":{\"records_type\",\"emitted\",\"emitted_total\",\"pages\",\"duplicates\",\"completed\",
   \"already_completed\",\"total_count\",\"state_path\"},\"warnings\":[{\"code\",\"message\"}]}
  Предупреждения: default_filter, base_url_env_ignored, fields_without_id, field_absent,
  csv_to_many, csv_new_columns.

Ошибки: invalid_filter, invalid_include, invalid_sort, invalid_per_page, invalid_max_records,
invalid_fields, duplicate_field, field_needs_include, fields_need_tabular_format, unknown_field,
field_to_many, state_mismatch, state_invalid, state_version, scroll_position_uncertain,
scroll_interrupted, no_progress, output_closed, no_token, invalid_token, unauthorized, forbidden,
rate_limited (retry_after — сколько секунд ждать), server_error, network_error.

Коды выхода: 0 — успех; 2 — ошибка в параметрах; 3 — нет токена или он отклонён;
7 — исчерпаны повторы после rate limit; 1 — прочие ошибки;
4 — выгрузка прервана, но в stdout только целые записи: со --state оставьте полученное
и запустите ту же команду снова — она продолжит после последней выданной страницы;
без --state выгрузите заново (повторы в обоих случаях убирайте по id).";

// Ссылка на каталог — в тег этой версии: коды в `main` могут уйти вперёд установленного бинаря.
pub(super) const ROOT_AFTER_LONG_HELP: &str = concat!(
    "\
Примеры:
  aplaut auth login
  aplaut reviews get crm-4211 --include comments --format jsonl
  aplaut reviews scroll --filter updated_at:gte:2024-01-01T00:00:00Z --format jsonl > reviews.jsonl
  aplaut products scroll --format csv --max-records 500 | tw

Для агентов и скриптов:
  --json      итог и ошибка — одной строкой JSON:
                {\"ok\":true,\"command\":…,\"cli_version\":…,\"dry_run\":false,\"result\":…,\"warnings\":[…]}
                {\"ok\":false,…,\"error\":{\"code\",\"message\",\"field\",\"retryable\",\"retry_after\",
                 \"hint\",\"request_id\"},\"warnings\":[…]}
              итог — в stdout (у scroll и get — в stderr: stdout занят данными), ошибка — в stderr;
              предупреждения — в warnings с кодами, а не текстом; включает --no-input
  --no-input  ничего не спрашивать: вместо вопроса — ошибка с подсказкой, какой флаг передать
  --yes       подтвердить удаление без вопроса (profile delete)
  --dry-run   (-n) у команд с изменениями: всё проверить и вернуть план в result, ничего не меняя
  retry_after в ошибке — сколько секунд ждать по словам сервера (429, 503).
  Без терминала ошибка приходит JSON-конвертом и без --json.
  Поля result — в разделе «JSON (--json)» справки команды: aplaut <команда> --help.
  Каталог кодов ошибок и предупреждений:
  https://github.com/aplaut-tech/aplaut-cli/blob/v",
    env!("CARGO_PKG_VERSION"),
    "/docs/automation.md

Коды выхода: 0 — успех; 1 — прочие ошибки; 2 — ошибка в параметрах; 3 — нет токена или он
отклонён; 4 — выгрузка прервана, в stdout только целые записи; 5 — не найдено;
7 — rate limit, повторы исчерпаны; 8 — нужно подтверждение (--yes).

Документация: https://aplaut.com/docs/api-references/platform/
Поддержка: support@aplaut.com"
);

pub(super) const GET_AFTER_LONG_HELP: &str = "\
Примеры:
  aplaut reviews get 5f1c2a9e8b7d6c5b4a3f2e1d                              # по внутреннему id
  aplaut reviews get crm-4211 --include author,comments --format jsonl    # по внешнему id
  aplaut products get 444772 --format csv --fields id,name

  # Для агента: запись — в stdout, итог — одной строкой JSON в stderr.
  aplaut reviews get crm-4211 --include comments --format jsonl --json

ID — внутренний идентификатор или external_id (без «.» и «/»: API такие id в пути обрезает
или не находит). --include по ресурсу (проверяется до запроса):
  reviews    author, product, comments, state_changes
  products   reviews_summary_item, reviews, questions, brand, category
  questions  author, product, answers
  consumers  reviews, questions, orders

Форматы — как у scroll, для одной записи: raw — тело ответа одной строкой; jsonl — запись
с подставленными объектами --include; csv — заголовок и строка (--fields — по правилам scroll).

JSON (--json): запись — в stdout в --format; итог — в stderr одной строкой:
  {\"ok\":true,\"command\":\"reviews.get\",\"cli_version\":…,\"dry_run\":false,
   \"result\":{\"records_type\":\"reviews\",\"id\":\"<внутренний id>\"},\"warnings\":[]}

Ошибки: invalid_id, invalid_include, invalid_fields, duplicate_field, field_needs_include,
fields_need_tabular_format, unknown_field, field_to_many, not_found, bad_response, no_token,
invalid_token, unauthorized, forbidden, rate_limited (retry_after — сколько секунд ждать),
server_error, network_error, timeout.

Коды выхода: 0 — успех; 2 — ошибка в параметрах; 3 — нет токена или он отклонён; 5 — записи нет;
7 — rate limit, повторы исчерпаны; 1 — прочие ошибки.";

pub(super) const REVIEWS_CREATE_AFTER_LONG_HELP: &str = "\
Примеры:
  aplaut reviews create --rating 5 --body \"Отличный магазин\" --author-name \"Анна\" --external-id crm-4211
  aplaut reviews create --rating 4 --body \"Хорошо\" --product-id 444772 -n   # показать запрос, не отправляя

  # Для агента: атрибуты — JSON-объектом из stdin, итог — одной строкой JSON в stdout.
  echo '{\"rating\":5,\"body\":\"Спасибо!\",\"photos\":[\"https://…/1.jpg\"],\"external_id\":\"crm-4211\"}' \\
    | aplaut reviews create --data - --json

Атрибуты — из схемы тела POST /reviews в спеке; флаги перекрывают одноимённые ключи --data.
Обязательны rating и хотя бы одно из body, pros, cons (так проверяет сервер; схема спеки
требует body). Через --data передаётся всё, для чего нет флага: photos, tags, rating_details,
dimensions, custom_attributes, даты, hide_my_data и т. д. Недоступные URL в photos сервер молча
отбрасывает. Без product_id отзыв создаётся о компании.
С -n (--dry-run) токен проверяется, но запросов нет.

Повторы: CLI повторяет запрос сам, только если сервер его точно не обработал (429, 503,
соединение не установилось). Иначе — request_outcome_unknown: проверьте, создан ли отзыв,
прежде чем повторять. С --external-id (без «.» и «/») проверка — aplaut reviews get <external_id>.

JSON (--json):
  {\"ok\":true,\"command\":\"reviews.create\",\"cli_version\":…,\"dry_run\":false,
   \"result\":{\"request\":{\"method\":\"POST\",\"path\":\"/reviews\",
   \"body\":{\"data\":{\"type\":\"reviews\",\"attributes\":{…}}}},
   \"created\":{\"id\",\"type\":\"reviews\",\"attributes\":{…}}},\"warnings\":[]}
  С -n — тот же request, \"created\":null и \"dry_run\":true.

Ошибки: unknown_attribute (в hint — допустимые), invalid_attribute, missing_attribute,
invalid_data, stdin_conflict, stdin_is_terminal, validation_failed (422, в field — атрибут),
request_outcome_unknown, bad_response, no_token, invalid_token, unauthorized, forbidden,
rate_limited (retry_after — сколько секунд ждать), server_error, network_error.

Коды выхода: 0 — создан (с -n — план); 2 — ошибка во входных данных, до сети; 3 — нет токена
или он отклонён; 7 — rate limit, повторы исчерпаны; 1 — прочие ошибки, в том числе
request_outcome_unknown (исход неизвестен — проверьте, прежде чем повторять).";

pub(super) const REVIEWS_COMMENT_AFTER_LONG_HELP: &str = "\
Примеры:
  aplaut reviews comment 5f1c2a9e8b7d6c5b4a3f2e1d --text \"Спасибо за отзыв!\" --author-name \"Магазин\"
  aplaut reviews comment crm-4211 --text \"Спасибо!\" -n        # показать запрос, не отправляя

  # Для агента: сначала план — показать человеку result.request.body, после согласия — отправка.
  aplaut reviews comment crm-4211 --text \"Спасибо за отзыв!\" --external-id reply-crm-4211 -n --json
  aplaut reviews comment crm-4211 --text \"Спасибо за отзыв!\" --external-id reply-crm-4211 --json

REVIEW_ID — внутренний идентификатор отзыва или его external_id (без «.» и «/»). Атрибуты — из схемы тела
POST /reviews/{id}/relationships/comments; через --data — остальные (files, hide_my_data,
author_external_id, external_parent_id, …); флаги перекрывают одноимённые ключи --data.
С -n (--dry-run) токен проверяется, но запросов нет.

Повторы — как у create: только если сервер точно не обработал запрос. Иначе
request_outcome_unknown, а в hint — как проверить: aplaut reviews get <REVIEW_ID> --include comments.

JSON (--json):
  {\"ok\":true,\"command\":\"reviews.comment\",\"cli_version\":…,\"dry_run\":false,
   \"result\":{\"request\":{\"method\":\"POST\",\"path\":\"/reviews/<id>/relationships/comments\",
   \"body\":{\"data\":{\"type\":\"comments\",\"attributes\":{\"text\":…}}}},
   \"created\":{\"id\",\"type\":\"comments\",\"attributes\":{…}}},\"warnings\":[]}
  С -n — тот же request, \"created\":null и \"dry_run\":true.

Ошибки: invalid_id, unknown_attribute, invalid_attribute, missing_attribute, invalid_data,
stdin_conflict, stdin_is_terminal, not_found (отзыва нет), validation_failed,
request_outcome_unknown, bad_response, no_token, invalid_token, unauthorized, forbidden,
rate_limited (retry_after — сколько секунд ждать), server_error, network_error.

Коды выхода: 0 — добавлен (с -n — план); 2 — ошибка во входных данных, до сети; 3 — нет токена
или он отклонён; 5 — отзыва нет; 7 — rate limit, повторы исчерпаны; 1 — прочие ошибки, в том
числе request_outcome_unknown (исход неизвестен — проверьте, прежде чем повторять).";

/// Короткая справка (`-h`) листовых команд отсылает к длинной.
pub(super) const LEAF_AFTER_HELP: &str = "Примеры, поля JSON (--json) и коды выхода: --help";

pub(super) const AUTH_LOGIN_AFTER_LONG_HELP: &str = "\
Примеры:
  aplaut auth login                                     # ввод скрыт
  aplaut auth login --profile staging --base-url https://api.staging.example/v4
  printf '%s' \"$TOKEN\" | aplaut auth login --token-stdin --profile ci --json
  aplaut auth login --token-file token.txt --profile ci --dry-run --json

Токен сохраняется в ~/.config/aplaut/credentials (0600) и на сервере не проверяется.
Без терминала, с --no-input и с --json — только --token-stdin или --token-file.

JSON (--json):
  {\"ok\":true,\"command\":\"auth.login\",\"dry_run\":false,\"result\":{\"profile\":\"ci\",\"token\":\"***\",
   \"token_source\":\"prompt|stdin|file\",\"replaced\":false,\"credentials_path\":\"…\"},\"warnings\":[]}
  Сам токен не выводится никогда. С --dry-run — тот же result и \"dry_run\":true, файлы не меняются.

Ошибки: token_required (передайте --token-stdin или --token-file), empty_token,
invalid_token_format, stdin_is_terminal, token_file_unreadable, invalid_profile,
invalid_base_url, insecure_base_url, credentials_invalid, config_invalid, io_error.

Коды выхода: 0 — сохранено (с --dry-run — план); 2 — ошибка в параметрах;
3 — не прочитать файл токена; 1 — прочие ошибки.";

pub(super) const AUTH_LOGOUT_AFTER_LONG_HELP: &str = "\
Примеры:
  aplaut auth logout --profile ci
  aplaut auth logout --profile ci --dry-run --json

JSON (--json):
  {\"ok\":true,\"command\":\"auth.logout\",\"dry_run\":false,\"result\":{\"profile\":\"ci\",\"removed\":true},
   \"warnings\":[]}
  removed: false — токена в профиле не было (это не ошибка).

Ошибки: invalid_profile, credentials_invalid, io_error.

Коды выхода: 0 — готово (с --dry-run — план); 2 — ошибка в параметрах; 1 — прочие ошибки.";

pub(super) const PROFILE_LIST_AFTER_LONG_HELP: &str = "\
Примеры:
  aplaut profile list
  aplaut profile list --json

JSON (--json):
  {\"ok\":true,\"command\":\"profile.list\",\"dry_run\":false,\"result\":{\"profiles\":[{\"name\",
   \"description\",\"base_url\",\"base_url_default\",\"has_token\",\"active\"}]},\"warnings\":[]}
  Токены не выводятся: has_token говорит только, есть ли он.

Ошибки: config_invalid, credentials_invalid.

Коды выхода: 0 — успех; 1 — файлы профилей не читаются.";

pub(super) const PROFILE_GET_AFTER_LONG_HELP: &str = "\
Примеры:
  aplaut profile get staging
  aplaut profile get staging --json

JSON (--json):
  {\"ok\":true,\"command\":\"profile.get\",\"dry_run\":false,\"result\":{\"name\",\"description\",
   \"base_url\",\"base_url_default\",\"has_token\",\"active\"},\"warnings\":[]}

Ошибки: profile_not_found (в hint — существующие профили), config_invalid, credentials_invalid.

Коды выхода: 0 — успех; 5 — профиля нет; 1 — прочие ошибки.";

pub(super) const PROFILE_SET_AFTER_LONG_HELP: &str = "\
Примеры:
  aplaut profile set staging --base-url https://api.staging.example/v4 --description \"Стенд для тестов\"
  aplaut profile set staging --base-url none               # вернуть прод по умолчанию
  aplaut profile set staging --description none --dry-run --json

JSON (--json):
  {\"ok\":true,\"command\":\"profile.set\",\"dry_run\":false,\"result\":{\"profile\":\"staging\",
   \"created\":true,\"changes\":[{\"field\":\"base_url\",\"from\":null,\"to\":\"https://…\"}],
   \"config_path\":\"…\"},\"warnings\":[]}
  С --dry-run — тот же result и \"dry_run\":true, файл не меняется.

Ошибки: nothing_to_set, invalid_profile, invalid_base_url, insecure_base_url,
invalid_description, config_invalid, io_error.

Коды выхода: 0 — сохранено (с --dry-run — план); 2 — ошибка в параметрах; 1 — прочие ошибки.";

pub(super) const PROFILE_DELETE_AFTER_LONG_HELP: &str = "\
Примеры:
  aplaut profile delete staging                  # в терминале спросит подтверждение
  aplaut profile delete staging --yes --json
  aplaut profile delete staging --dry-run --json

JSON (--json):
  {\"ok\":true,\"command\":\"profile.delete\",\"dry_run\":false,\"result\":{\"profile\":\"staging\",
   \"removed_config\":true,\"removed_token\":false},\"warnings\":[]}
  С --dry-run — что было бы удалено; подтверждение не нужно.

Ошибки: confirmation_required (без терминала, с --no-input и --json нужен --yes),
profile_not_found, invalid_profile, config_invalid, credentials_invalid, io_error.

Коды выхода: 0 — удалено (с --dry-run — план); 2 — ошибка в параметрах; 5 — профиля нет;
8 — нужен --yes; 1 — прочие ошибки.";

pub(super) const PROFILE_EDIT_AFTER_LONG_HELP: &str = "\
Примеры:
  aplaut profile edit
  EDITOR=\"code --wait\" aplaut profile edit

Правится копия config.toml: оригинал заменяется только после проверки. VS Code, Zed,
Sublime Text, gvim и Kate получают флаг ожидания сами.

JSON (--json): не поддерживается — редактору нужен человек. С --json, --no-input и без
терминала команда отказывает (terminal_required); агентам и скриптам — aplaut profile set.

Ошибки: terminal_required, editor_failed, editor_returned_immediately, config_invalid,
config_changed_during_edit, io_error.

Коды выхода: 0 — сохранено или без изменений; 2 — нет терминала; 1 — прочие ошибки.";

pub(super) const SELF_UPDATE_AFTER_LONG_HELP: &str = "\
Примеры:
  aplaut self update             # последний релиз — тем же установщиком, в тот же каталог
  aplaut self update --dry-run   # только проверить, есть ли новая версия
  aplaut self update --json

Работает, если aplaut поставлен установщиком из README: тот оставляет файл установки (receipt) в
~/.config/aplaut-cli/. Поставленный иначе обновляйте тем же способом, которым ставили.
Без токена GitHub даёт 60 запросов в час; в CI задайте APLAUT_CLI_GITHUB_TOKEN.

JSON (--json):
  {\"ok\":true,\"command\":\"self.update\",\"dry_run\":false,\"result\":{\"current\":\"0.2.0\",
   \"latest\":\"0.3.0\",\"updated\":true},\"warnings\":[]}
  updated: false — новее нет; с --dry-run — обновил бы или нет (ничего не поставлено);
  latest — последний релиз.

Ошибки: update_unavailable (поставлен не установщиком или релиза нет), update_failed (установщик
завершился с ошибкой), unauthorized (GitHub отклонил APLAUT_CLI_GITHUB_TOKEN), rate_limited, timeout,
network_error.

Коды выхода: 0 — готово (с --dry-run — проверка); 3 — GitHub отклонил токен; 7 — лимит GitHub API;
1 — прочие ошибки.";

pub(super) const PRODUCTS_CREATE_AFTER_LONG_HELP: &str = "\
Примеры:
  aplaut products create --external-id 60757 --name \"Transcend StoreJet 1 ТБ\" --url https://shop.example/p/60757 --price 5990 --available true --category-id 297
  aplaut products create --external-id 60757 --name \"Диск\" --url https://shop.example/p/60757 -n   # показать запрос, не отправляя

  # Для агента: атрибуты — JSON-объектом из stdin, итог — одной строкой JSON в stdout.
  echo '{\"external_id\":\"60757\",\"name\":\"Диск\",\"url\":\"https://shop.example/p/60757\",\"category_names\":[\"Электроника\",\"Диски\"]}' \\
    | aplaut products create --data - --json

Атрибуты — из схемы тела POST /products в спеке; флаги перекрывают одноимённые ключи --data.
Обязательны external_id, name и url (url требует сервер, хотя в спеке он не обязателен).
Категория — category_names (цепочка имён, через --data), --category-id (external_id категории)
или --category-name; без них товар попадает в корневую категорию. category_names и
--category-name создают категорию, если её нет, --brand-name — бренд. picture_urls,
recommended_product_ids, custom_attributes и даты — через --data.
С -n (--dry-run) токен проверяется, но запросов нет.

Повторы: CLI повторяет запрос сам, только если сервер его точно не обработал. После
request_outcome_unknown повтор безопасен: второй товар с тем же external_id сервер не создаст
(422, external_id is already taken). Проверить — aplaut products get <external_id>.

JSON (--json):
  {\"ok\":true,\"command\":\"products.create\",\"cli_version\":…,\"dry_run\":false,
   \"result\":{\"request\":{\"method\":\"POST\",\"path\":\"/products\",
   \"body\":{\"data\":{\"type\":\"products\",\"attributes\":{…}}}},
   \"created\":{\"id\",\"type\":\"products\",\"attributes\":{…}}},\"warnings\":[]}
  С -n — тот же request, \"created\":null и \"dry_run\":true.

Ошибки: unknown_attribute (в hint — допустимые), invalid_attribute, missing_attribute,
invalid_data, stdin_conflict, stdin_is_terminal, usage (флаг вместо значения --description),
validation_failed (422; external_id is already taken — товар уже есть, в hint — products update),
request_outcome_unknown, bad_response, no_token, invalid_token, unauthorized, forbidden,
rate_limited (retry_after — сколько секунд ждать), server_error, network_error.

Коды выхода: 0 — создан (с -n — план); 2 — ошибка во входных данных, до сети; 3 — нет токена
или он отклонён; 7 — rate limit, повторы исчерпаны; 1 — прочие ошибки, в том числе
request_outcome_unknown (повтор безопасен).";

pub(super) const PRODUCTS_UPDATE_AFTER_LONG_HELP: &str = "\
Примеры:
  aplaut products update 60757 --price 5490 --available true
  aplaut products update 60757 --description \"Новое описание\" -n        # показать запрос, не отправляя

  # Для агента: частичное обновление JSON-объектом; null очищает атрибут, в custom_attributes — удаляет ключ.
  echo '{\"price\":5490,\"custom_attributes\":{\"color\":\"black\",\"old_key\":null}}' | aplaut products update 60757 --data - --json

ID — внутренний идентификатор или external_id (без «.» и «/»). Меняются только переданные
атрибуты, custom_attributes сливаются с текущими. Атрибуты — те же, что у create: вычисляемые
rating, reviews_count, recommended сервер не меняет, поэтому они — unknown_attribute.
--external-id NEW_ID переименовывает товар. С -n (--dry-run) токен проверяется, но запросов нет.

Повторы: правка идемпотентна и повторяется после таймаута и 5xx, как чтение. Со сменой
external_id — только если сервер точно не обработал запрос, иначе request_outcome_unknown.

JSON (--json):
  {\"ok\":true,\"command\":\"products.update\",\"cli_version\":…,\"dry_run\":false,
   \"result\":{\"request\":{\"method\":\"PUT\",\"path\":\"/products/<id>\",
   \"body\":{\"data\":{\"type\":\"products\",\"attributes\":{…}}}},
   \"updated\":{\"id\",\"type\":\"products\",\"attributes\":{…}}},\"warnings\":[]}
  С -n — тот же request, \"updated\":null и \"dry_run\":true.

Ошибки: invalid_id, nothing_to_update, unknown_attribute, invalid_attribute, invalid_data,
stdin_conflict, stdin_is_terminal, usage, not_found (товара нет), validation_failed (422),
request_outcome_unknown, bad_response, no_token, invalid_token, unauthorized, forbidden,
rate_limited (retry_after — сколько секунд ждать), server_error, network_error, timeout.

Коды выхода: 0 — обновлён (с -n — план); 2 — ошибка во входных данных, до сети; 3 — нет токена
или он отклонён; 5 — товара нет; 7 — rate limit, повторы исчерпаны; 1 — прочие ошибки.";

/// Списки инструментов — из таблицы ресурсов (спека writes-and-exports R12).
pub(super) fn mcp_after_long_help() -> String {
    let read = crate::mcp::tools::names(false).join(", ");
    let write = crate::mcp::tools::names(true).join(", ");
    format!(
        "\
Примеры:
  claude mcp add aplaut -- aplaut mcp                                   # Claude Code, профиль по умолчанию
  claude mcp add aplaut-staging -- aplaut mcp --profile staging
  claude mcp add aplaut-prod -- aplaut mcp --profile prod --allow-writes

Сервер говорит по MCP через stdin и stdout, пока клиент не закроет stdin. Каждый вызов инструмента —
команда aplaut с --json и глобальными флагами сервера (--profile, --base-url, --token-file, --timeout,
--max-retries, --verbose): агент их не меняет.
Инструменты: {read}.
С --allow-writes ещё: {write}.
Подробнее — docs/mcp.md.

JSON (--json):
  {{\"ok\":true,\"command\":\"mcp\",\"dry_run\":false,\"result\":{{\"allow_writes\":false}},\"warnings\":[]}}
  конверт — в stderr после завершения сервера: stdout занят протоколом.

Ошибки: usage (--token-stdin или --token-file -: stdin занят протоколом), mcp_handshake_failed (клиент
закрыл соединение или прислал не MCP до initialize).

Коды выхода: 0 — клиент закрыл соединение; 2 — ошибка в параметрах; 1 — прочие ошибки."
    )
}
