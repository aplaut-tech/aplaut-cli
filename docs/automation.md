# Для агентов и скриптов

Любая команда работает без человека. Команды и их флаги — в [reference.md](reference.md).

MCP-клиентам удобнее `aplaut mcp`: те же команды инструментами со схемами — [mcp.md](mcp.md). Имена
инструментов стабильны так же, как коды ошибок.

## Флаги

| Флаг | Что делает |
|---|---|
| `--json` | итог и ошибка — одной строкой JSON; предупреждения — в `warnings`, а не текстом; включает `--no-input` |
| `--no-input` | ничего не спрашивать: вместо вопроса — ошибка с подсказкой, какой флаг передать |
| `--yes` (`-y`) | подтвердить удаление без вопроса (`profile delete`) |
| `--dry-run` (`-n`) | у `profile set/delete`, `auth login/logout`, `reviews create/comment/update`, `products create/update`, `questions create/update`, `consumers create/update`, `orders create/update`, `self update`: всё проверить и вернуть план в `result`, ничего не меняя |

## JSON-конверт

```json
{"ok":true,"command":"profile.set","cli_version":"0.4.0","dry_run":false,
 "result":{"profile":"staging","created":true,"changes":[{"field":"base_url","from":null,"to":"https://…"}],"config_path":"…"},
 "warnings":[]}
{"ok":false,"command":"reviews.scroll","cli_version":"0.4.0","dry_run":false,
 "error":{"code":"rate_limited","message":"…","field":null,"retryable":true,"retry_after":30,"hint":"…","request_id":"…"},
 "warnings":[{"code":"default_filter","message":"…"}]}
```

- Без терминала или с `--json` ошибка — JSON-конверт.
- Итог — в stdout; у `scroll` и `get` stdout занят данными, поэтому их итог — в stderr. Ошибка — всегда в
  stderr. С `--verbose` строки `debug:` идут до конверта: конверт — всегда последняя строка.
- `retry_after` — сколько секунд ждать по словам сервера (429, 503), иначе `null`.
- Поля `result` каждой команды — в `aplaut <команда> --help`, раздел «JSON (--json)».
- Стабильность: поля и коды только добавляются; переименование или удаление — только со сменой
  major-версии. `cli_version` есть в каждом конверте.

## Коды выхода

| Код | Значение |
|---|---|
| 0 | успех |
| 1 | прочая ошибка |
| 2 | ошибка в параметрах или локальной проверке |
| 3 | нет токена или он отклонён (401/403) |
| 4 | выгрузка прервана; в stdout только целые записи. Со `--state` оставьте полученное и запустите ту же команду снова — она продолжит после последней выданной страницы. Без `--state` выгрузите заново. Повторы убирайте по `id` |
| 5 | не найдено (404) |
| 7 | rate limit, повторы исчерпаны |
| 8 | нужно подтверждение: повторите с `--yes` |

## Коды ошибок

| Код | Выход | Когда |
|---|---|---|
| `usage` | 2 | неверные аргументы: неизвестный флаг, нет обязательного |
| `invalid_filter` | 2 | `--filter` не по спеке |
| `invalid_include` | 2 | неизвестная связь в `--include` |
| `invalid_id` | 2 | пустой идентификатор или с `.`/`/`: API такой в пути обрезает до другой записи или не находит |
| `invalid_sort` | 2 | сортировка не из допустимых |
| `invalid_per_page` | 2 | `--per-page` вне 1–100 |
| `invalid_max_records` | 2 | `--max-records 0` |
| `invalid_fields` | 2 | пустой или кривой элемент `--fields` |
| `duplicate_field` | 2 | колонка в `--fields` дважды |
| `field_needs_include` | 2 | `<связь>.<атрибут>` без `--include` этой связи |
| `fields_need_tabular_format` | 2 | `--fields` без `--format csv` |
| `unknown_field` | 2 | колонки из `--fields` нет на первой странице обхода (в `hint` — ближайшее имя и все колонки) |
| `field_to_many` | 2 | `<связь>.<атрибут>` у связи-списка |
| `unknown_attribute` | 2 | атрибута нет в схеме тела запроса; в `hint` — ближайшее имя и допустимые |
| `invalid_attribute` | 2 | значение атрибута не того типа, не из перечисления, вне границ или не RFC 3339; или атрибут, который сервер в этой операции молча игнорирует (внешний id в `update`), e-mail и телефон клиента в `update` без `--upsert`, строка `order_lines` без `product_id` |
| `missing_attribute` | 2 | не задан обязательный атрибут |
| `invalid_data` | 2 | `--data` не читается, не JSON-объект или обёрнут в `data` |
| `stdin_conflict` | 2 | `--data -` вместе с токеном из stdin |
| `nothing_to_update` | 2 | `update` без единого атрибута |
| `invalid_profile` | 2 | недопустимое имя профиля |
| `invalid_base_url` | 2 | base URL не разбирается |
| `insecure_base_url` | 2 | `http://` не для localhost |
| `invalid_description` | 2 | описание профиля не в одну строку |
| `nothing_to_set` | 2 | `profile set` без `--base-url` и `--description` |
| `empty_token` | 2 | токен пустой |
| `invalid_token_format` | 2 | в токене пробелы или управляющие символы |
| `stdin_is_terminal` | 2 | `--token-stdin` или `--data -`, а stdin — терминал |
| `token_required` | 2 | `auth login` без `--token-stdin`/`--token-file` там, где спросить нельзя |
| `terminal_required` | 2 | `profile edit` без терминала, с `--no-input` или `--json` |
| `state_invalid` | 2 | файл стейта повреждён |
| `state_version` | 2 | стейт записан другой версией формата |
| `state_mismatch` | 2 | параметры (в том числе `--fields`) не совпадают со стейтом |
| `scroll_position_uncertain` | 2 | прошлый запуск оборвался посреди продолжения; в `hint` — команда для нового обхода |
| `output_exists` | 2 | `aplaut mcp`: `output_file` уже есть, а `state` и `overwrite` не заданы |
| `no_token` | 3 | токен не найден ни в одном источнике |
| `token_file_unreadable` | 3 | файл токена не читается |
| `unauthorized`, `invalid_token` | 3 | 401: токен отклонён (код — из `WWW-Authenticate`, если сервер его прислал); у `self update` — GitHub отклонил `APLAUT_CLI_GITHUB_TOKEN` |
| `forbidden` | 3 | 403: у токена нет прав |
| `output_closed` | 4 | получатель закрыл stdout (например, `\| head`) |
| `not_found` | 5 | 404; у `update` с проверкой наличия (без `--upsert`) — объекта нет, ничего не создано |
| `profile_not_found` | 5 | профиля нет; в `hint` — существующие |
| `rate_limited` | 7 | 429: повторы исчерпаны или ждать дольше 5 минут; `retry_after` — сколько. У `self update` — 403/429 GitHub API, `retry_after` — `null` |
| `confirmation_required` | 8 | удаление без `--yes` там, где спросить нельзя |
| `bad_request` | 1 | 400 |
| `validation_failed` | 1 | 422: сервер отклонил параметры |
| `invalid_cursor` | 1 | сервер не принял курсор |
| `cursor_mismatch` | 1 | курсор выдан для другого запроса |
| `server_error` | 1 | 5xx; у 503 — `retry_after`. У `self update` — 5xx GitHub API, `retry_after` — `null` |
| `request_outcome_unknown` | 1 | запись (POST или PUT со сменой `external_id`) оборвалась после отправки — таймаут, обрыв, 5xx кроме 503: неизвестно, выполнена ли она; CLI не повторяет, в `hint` — как проверить |
| `unexpected_redirect` | 1 | 3xx: редиректы не выполняются |
| `http_<status>` | 1 | прочие ответы HTTP без своего кода, например `http_409` |
| `timeout` | 1 | нет ответа за `--timeout` после повторов |
| `network_error` | 1 | сеть или TLS после повторов |
| `response_too_large` | 1 | ответ больше допустимого |
| `bad_response` | 1 | ответ не по контракту: не JSON, `has_more` без курсора, 2xx записи без `data` (запись, скорее всего, выполнена; `retryable: false` у POST, `true` у идемпотентного PUT) |
| `scroll_interrupted` | 1 | продолжение оборвалось, неизвестно, обработал ли его сервер; в `hint` — как продолжить |
| `no_progress` | 1 | страницы без новых записей: обход зациклился |
| `update_unavailable` | 1 | `self update`: aplaut поставлен не установщиком (нет receipt или он от другого бинаря) или на GitHub нет релиза с установщиком; ничего не изменено |
| `update_failed` | 1 | `self update`: установщик новой версии завершился с ошибкой (подробности — выше, в его выводе) или GitHub ответил непонятно |
| `mcp_handshake_failed` | 1 | `aplaut mcp`: клиент закрыл соединение или прислал не MCP до `initialize` |
| `config_invalid` | 1 | `config.toml` не разбирается или в нём неизвестные ключи |
| `credentials_invalid` | 1 | `credentials` не разбирается |
| `config_changed_during_edit` | 1 | `config.toml` изменился, пока был открыт редактор; правки — в копии |
| `editor_failed` | 1 | редактор завершился с ошибкой |
| `editor_returned_immediately` | 1 | GUI-редактор вернул управление, не дождавшись правок |
| `no_config_dir` | 1 | не заданы `XDG_CONFIG_HOME` и `HOME` |
| `io_error` | 1 | ошибка чтения или записи файла |
| `internal` | 1 | внутренняя ошибка CLI — сообщите в поддержку |

Любая ошибка после того, как часть записей уже выдана, — код выхода 4 с исходным `code`.

## Предупреждения

| Код | Когда |
|---|---|
| `default_filter` | без `--filter` сервер ограничил обход последними 30 днями |
| `base_url_env_ignored` | явный `--profile`, а `APLAUT_BASE_URL` задан — используется base URL профиля |
| `credentials_permissions` | файл `credentials` доступен другим пользователям |
| `fields_without_id` | `--fields` без `id` при `--state`: повторы после сбоя не убрать |
| `field_absent` | колонки из `--fields` нет в данных — в CSV она пустая |
| `csv_to_many` | связь-список в CSV не разворачивается |
| `csv_new_columns` | в данных появились поля, которых не было на первой странице |
