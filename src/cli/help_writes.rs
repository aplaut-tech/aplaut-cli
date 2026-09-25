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
