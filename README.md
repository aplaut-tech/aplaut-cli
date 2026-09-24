# aplaut

Консольный клиент [Aplaut Platform API](https://aplaut.com/docs/api-references/platform/):
выгрузка отзывов, товаров и вопросов для DWH, cron и агентов.

## Установка

```bash
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/aplaut-tech/aplaut-cli/releases/latest/download/aplaut-cli-installer.sh | sh
```

Установщик скачивает статический бинарь для x86-64 Linux (работает на любом дистрибутиве, без
зависимостей), сверяет его sha256 и кладёт `aplaut` в `~/.local/bin`. Если этого каталога нет в
`PATH`, установщик добавит строку `. "$HOME/.config/aplaut-cli/env.sh"` в `~/.profile` и
`~/.zshrc` — перезапустите шелл или выполните её вручную. Скрипт на POSIX sh, `| bash` тоже подходит.

Конкретная версия — тот же скрипт из нужного релиза:

```bash
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/aplaut-tech/aplaut-cli/releases/download/v0.1.0/aplaut-cli-installer.sh | sh
```

Серверы, контейнеры и CI — без правки профиля шелла:

```bash
# только бинарь в указанный каталог, без ~/.profile и служебных файлов
curl … | APLAUT_CLI_UNMANAGED_INSTALL=/opt/aplaut/bin sh
# обычная установка, но PATH не трогать
curl … | APLAUT_CLI_NO_MODIFY_PATH=1 sh
```

Без `curl | sh`: скачайте со страницы релизов `aplaut-cli-x86_64-unknown-linux-musl.tar.xz` и
`.sha256`, проверьте `sha256sum -c` и распакуйте. Проверка sha256 защищает от битой загрузки,
происхождение бинаря подтверждает attestation:
`gh attestation verify aplaut-cli-x86_64-unknown-linux-musl.tar.xz -R aplaut-tech/aplaut-cli`.

Удаление: `rm ~/.local/bin/aplaut`, каталог `~/.config/aplaut-cli/`, файл
`~/.config/fish/conf.d/aplaut-cli.env.fish` и строку с `aplaut-cli/env.sh` в `~/.profile` /
`~/.zshrc`. Токены и профили лежат отдельно, в `~/.config/aplaut/`.

## Токен

Токен выпускается в ЛК: «Разработчикам» → OAuth-приложение со scope Platform API.

```bash
aplaut auth login                                   # ввод скрыт
echo "$TOKEN" | aplaut auth login --token-stdin --profile ci
aplaut auth logout --profile ci
```

Порядок источников: `--token-stdin` / `--token-file` → `--profile` → `APLAUT_ACCESS_TOKEN_FILE`
→ `APLAUT_ACCESS_TOKEN` → `APLAUT_PROFILE` → профиль `default`. В CI и контейнерах лучше
`APLAUT_ACCESS_TOKEN_FILE`: значение переменной окружения видно в `docker inspect` и
`systemctl show`. Флага `--token` со значением нет — токен в argv виден в `ps`.

Файлы: `~/.config/aplaut/config.toml` (профили, `base_url`) и `~/.config/aplaut/credentials`
(токены), оба с правами 0600. Другой стенд — `--base-url` или `APLAUT_BASE_URL`; `http://`
допустим только для localhost. При явном `--profile` берётся `base_url` этого профиля (или прод),
а `APLAUT_BASE_URL` игнорируется — чтобы токен профиля не ушёл на адрес из окружения.

## Профили

```bash
aplaut profile list                                    # профили, base URL, есть ли токен; * — активный
aplaut profile get staging --json                      # один профиль; --json — одной строкой JSON
aplaut profile set staging --base-url https://api.staging.example/v4 --description "Стенд для тестов"
aplaut profile set staging --base-url none             # вернуть прод по умолчанию (--description none — убрать описание)
aplaut profile delete staging                          # вместе с токеном; без терминала — --yes
aplaut profile edit                                    # config.toml в $VISUAL / $EDITOR
```

```text
$ aplaut profile list
  default — Прод, основной аккаунт
    - base_url: https://api.aplaut.io/v4 (по умолчанию)
    - токен: есть
* staging — Стенд для тестов
    - base_url: https://api.staging.example/v4
    - токен: нет
```

Токены эти команды не показывают и не меняют: токен задаёт только `aplaut auth login`.
В начале `config.toml` — закомментированная справка со всеми опциями профиля и их значениями
по умолчанию; aplaut восстанавливает её при каждой своей записи файла.
`profile edit` правит копию файла и заменяет оригинал только после проверки: при ошибке
предлагает открыть снова, а если файл изменил другой процесс (например, `auth login`), правки
остаются в копии. Без терминала `edit` не запускается — в скриптах используйте `profile set`.
GUI-редактор должен ждать, пока вы закроете файл. Командам `code`, `code-insiders`, `codium`,
`cursor`, `windsurf`, `zed`/`zeditor` и `subl` aplaut сам добавляет `--wait`, `gvim` — `--nofork`,
`kate` — `--block` (редактор узнаётся по имени программы, обёртки вроде `flatpak run` — нет).
Пока редактор открыт, в терминале видно «Жду, пока вы закроете файл в редакторе…»: в VS Code
закройте вкладку, в gvim и Kate — окно. Если редактор вернул управление сразу, не дав ничего
изменить, `edit` так и скажет и ничего не запишет — тогда задайте флаг ожидания в `VISUAL` сами.
Неизвестные ключи в `config.toml` — ошибка: опечатка вроде `base-url` иначе молча отправила
бы запросы на прод. Комментарии в файле пропадают, когда aplaut сам его перезаписывает
(`auth login`, `profile set`).

## Выгрузка

```bash
# Полная выгрузка отзывов в JSONL:
aplaut reviews scroll --filter updated_at:gte:2000-01-01T00:00:00Z --format jsonl > reviews.jsonl

# С продолжением после сбоя (повторный запуск продолжит с места остановки):
aplaut reviews scroll --filter updated_at:gte:2024-01-01T00:00:00Z --state reviews.state.json --format jsonl >> reviews.jsonl

# Просмотр:
aplaut products scroll --format csv --max-records 500 | tw

# Только нужные колонки и в нужном порядке:
aplaut reviews scroll --filter updated_at:gte:2024-01-01T00:00:00Z --include product --format csv --fields id,rating,pros,cons,body,product.name > reviews.csv
```

- **Без `--filter` сервер отдаёт только записи за последние 30 дней** — CLI об этом предупреждает.
- Доставка «хотя бы один раз»: после сбоя последняя страница может прийти повторно, поэтому
  убирайте дубли по `id`.
- Лимиты API: 1 запрос в 2 секунды и 5 открытий обхода в минуту на ключ. CLI соблюдает их сам;
  не запускайте параллельно несколько выгрузок с одним ключом.
- `--max-records N` останавливается на границе страницы: записей может быть чуть больше N.
- Курсор обхода нельзя повторить: сервер на повтор отдаёт уже следующую страницу. Если запрос
  продолжения оборвался и неизвестно, дошёл ли он, CLI не продолжает вслепую (`scroll_interrupted`,
  затем `scroll_position_uncertain`), а печатает команду для нового обхода с границы последней
  выданной записи: `--filter updated_at:gte:<значение> --state <новый файл>`.

## Одна запись

```bash
aplaut reviews get 5f1c2a9e8b7d6c5b4a3f2e1d                              # по внутреннему id
aplaut reviews get crm-4211 --include author,comments --format jsonl    # по внешнему id
aplaut products get 444772 --format csv --fields id,name
```

`get` есть у `reviews`, `products` и `questions`; `ID` — внутренний идентификатор или
`external_id`. Id с точкой или `/` CLI не отправляет (код 2): сервер отрезает всё после точки,
даже закодированной, и найдёт другую запись, а `/` не находит вовсе. Форматы и `--fields` — как у выгрузки. `--include` проверяется по спеке до
запроса (допустимые — в `aplaut <ресурс> get --help`). Записи нет — код 5.

## Создание отзыва и ответ на отзыв

```bash
# Отзыв о компании (без --product-id); флаги — частые атрибуты:
aplaut reviews create --rating 5 --body "Спасибо!" --author-name "Анна" --external-id crm-4211

# Остальные атрибуты (photos, tags, rating_details, даты…) — JSON-объектом в --data; флаги перекрывают его ключи:
aplaut reviews create --data review.json --state published

# Ответ магазина на отзыв: сначала план без отправки, потом отправка
aplaut reviews comment crm-4211 --text "Спасибо за отзыв!" -n
aplaut reviews comment crm-4211 --text "Спасибо за отзыв!"
```

- Атрибуты и их типы — из схемы тела запроса в спеке. Неизвестный атрибут, значение не того типа
  или не из перечисления, нет обязательного — код 2 до отправки, атрибут назван в `field`.
- У отзыва обязательны `rating` и хотя бы одно из `body`, `pros`, `cons`; у комментария — `text`.
- `-n` (`--dry-run`) показывает метод, путь и тело запроса и ничего не отправляет.
- У API нет идемпотентности. CLI повторяет запрос сам, только если сервер точно его не обработал
  (429, 503, соединение не установилось). После таймаута, обрыва или 5xx —
  `request_outcome_unknown`: проверьте, создан ли отзыв, прежде чем повторять. Передавайте
  `--external-id` — тогда проверка — `aplaut reviews get <external_id>`.

## Создание и изменение товаров

```bash
aplaut products create --external-id 60757 --name "Transcend StoreJet 1 ТБ" --url https://shop.example/p/60757 --price 5990 --category-id 297
aplaut products update 60757 --price 5490 --available true
```

- У `create` обязательны `external_id`, `name` и `url`. Второй товар с тем же `external_id` сервер не
  создаст (422 `is already taken`), поэтому повтор после сбоя безопасен.
- `update` меняет только переданные атрибуты; `custom_attributes` сливаются, `null` очищает. Правка
  повторяется после сбоя, как чтение; со сменой `external_id` — только если сервер точно её не получил.
- `-n` показывает метод, путь и тело запроса и ничего не отправляет.

## Форматы

| `--format` | Что в stdout |
|---|---|
| `raw` (по умолчанию) | тело каждого ответа API как есть, одна строка на страницу |
| `jsonl` | запись на строку; связанные объекты из `--include` подставлены в `relationships` |
| `csv` | заголовок и строки; колонки — `id`, `type`, атрибуты, `<связь>_ref`, `<связь>.<атрибут>` |

В `jsonl` ключи идут по алфавиту; точные байты ответа есть только в `raw`. CSV не экранирует
формулы (`=…`): в Excel импортируйте файл как текст.

`--fields id,rating,pros,cons,body` выбирает колонки CSV и их порядок; имена — как в заголовке
(`product.name` требует `--include product`). Заголовок — ровно этот список, даже у пустой
выгрузки: загрузка в DWH не ломается от новых полей API, а лишние персональные данные
(`author_email`, `author_phone`, `author_ip`) не попадают на диск. Колонки отбирает CLI — API
отдаёт все поля, трафик тот же. С `raw` и `jsonl` — код 2.

Имена сверяются с первой страницей обхода: опечатка — код 2 до записи чего-либо, с подсказкой
(«может, rating?») и списком колонок. Продолжение со `--state` берёт колонки из стейта — другой
`--fields` будет ошибкой; если имя проверить не удалось (на первой странице не было, например,
объектов `author`), колонка останется пустой, а CLI один раз предупредит, когда это выяснится.

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

Без терминала или с `--json` ошибка — JSON-конверт, см. «Для агентов и скриптов».

## Для агентов и скриптов

Любая команда работает без человека:

| Флаг | Что делает |
|---|---|
| `--json` | итог и ошибка — одной строкой JSON; предупреждения — в `warnings`, а не текстом; включает `--no-input` |
| `--no-input` | ничего не спрашивать: вместо вопроса — ошибка с подсказкой, какой флаг передать |
| `--yes` (`-y`) | подтвердить удаление без вопроса (`profile delete`) |
| `--dry-run` (`-n`) | у `profile set/delete`, `auth login/logout`, `reviews create/comment`, `products create/update`: всё проверить и вернуть план в `result`, ничего не меняя |

```json
{"ok":true,"command":"profile.set","cli_version":"0.1.0","dry_run":false,
 "result":{"profile":"staging","created":true,"changes":[{"field":"base_url","from":null,"to":"https://…"}],"config_path":"…"},
 "warnings":[]}
{"ok":false,"command":"reviews.scroll","cli_version":"0.1.0","dry_run":false,
 "error":{"code":"rate_limited","message":"…","field":null,"retryable":true,"retry_after":30,"hint":"…","request_id":"…"},
 "warnings":[{"code":"default_filter","message":"…"}]}
```

- Итог — в stdout; у `scroll` и `get` stdout занят данными, поэтому их итог — в stderr. Ошибка — всегда в
  stderr. С `--verbose` строки `debug:` идут до конверта: конверт — всегда последняя строка.
- `retry_after` — сколько секунд ждать по словам сервера (429, 503), иначе `null`.
- Поля `result` каждой команды — в `aplaut <команда> --help`, раздел «JSON (--json)».
- Стабильность: поля и коды только добавляются; переименование или удаление — только со сменой
  major-версии. `cli_version` есть в каждом конверте.

### Коды ошибок

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
| `invalid_attribute` | 2 | значение атрибута не того типа, не из перечисления, вне границ или не RFC 3339 |
| `missing_attribute` | 2 | не задан обязательный атрибут |
| `invalid_data` | 2 | `--data` не читается, не JSON-объект или обёрнут в `data` |
| `stdin_conflict` | 2 | `--data -` вместе с токеном из stdin |
| `nothing_to_update` | 2 | `products update` без единого атрибута |
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
| `no_token` | 3 | токен не найден ни в одном источнике |
| `token_file_unreadable` | 3 | файл токена не читается |
| `unauthorized`, `invalid_token` | 3 | 401: токен отклонён (код — из `WWW-Authenticate`, если сервер его прислал) |
| `forbidden` | 3 | 403: у токена нет прав |
| `output_closed` | 4 | получатель закрыл stdout (например, `\| head`) |
| `not_found` | 5 | 404 |
| `profile_not_found` | 5 | профиля нет; в `hint` — существующие |
| `rate_limited` | 7 | 429: повторы исчерпаны или ждать дольше 5 минут; `retry_after` — сколько |
| `confirmation_required` | 8 | удаление без `--yes` там, где спросить нельзя |
| `bad_request` | 1 | 400 |
| `validation_failed` | 1 | 422: сервер отклонил параметры |
| `invalid_cursor` | 1 | сервер не принял курсор |
| `cursor_mismatch` | 1 | курсор выдан для другого запроса |
| `server_error` | 1 | 5xx; у 503 — `retry_after` |
| `request_outcome_unknown` | 1 | запись (POST или PUT со сменой `external_id`) оборвалась после отправки — таймаут, обрыв, 5xx кроме 503: неизвестно, выполнена ли она; CLI не повторяет, в `hint` — как проверить |
| `unexpected_redirect` | 1 | 3xx: редиректы не выполняются |
| `http_<status>` | 1 | прочие ответы HTTP без своего кода, например `http_409` |
| `timeout` | 1 | нет ответа за `--timeout` после повторов |
| `network_error` | 1 | сеть или TLS после повторов |
| `response_too_large` | 1 | ответ больше допустимого |
| `bad_response` | 1 | ответ не по контракту: не JSON, `has_more` без курсора, 2xx записи без `data` (запись, скорее всего, выполнена; `retryable: false` у POST, `true` у идемпотентного PUT) |
| `scroll_interrupted` | 1 | продолжение оборвалось, неизвестно, обработал ли его сервер; в `hint` — как продолжить |
| `no_progress` | 1 | страницы без новых записей: обход зациклился |
| `config_invalid` | 1 | `config.toml` не разбирается или в нём неизвестные ключи |
| `credentials_invalid` | 1 | `credentials` не разбирается |
| `config_changed_during_edit` | 1 | `config.toml` изменился, пока был открыт редактор; правки — в копии |
| `editor_failed` | 1 | редактор завершился с ошибкой |
| `editor_returned_immediately` | 1 | GUI-редактор вернул управление, не дождавшись правок |
| `no_config_dir` | 1 | не заданы `XDG_CONFIG_HOME` и `HOME` |
| `io_error` | 1 | ошибка чтения или записи файла |
| `internal` | 1 | внутренняя ошибка CLI — сообщите в поддержку |

Любая ошибка после того, как часть записей уже выдана, — код выхода 4 с исходным `code`.

### Предупреждения

| Код | Когда |
|---|---|
| `default_filter` | без `--filter` сервер ограничил обход последними 30 днями |
| `base_url_env_ignored` | явный `--profile`, а `APLAUT_BASE_URL` задан — используется base URL профиля |
| `credentials_permissions` | файл `credentials` доступен другим пользователям |
| `fields_without_id` | `--fields` без `id` при `--state`: повторы после сбоя не убрать |
| `field_absent` | колонки из `--fields` нет в данных — в CSV она пустая |
| `csv_to_many` | связь-список в CSV не разворачивается |
| `csv_new_columns` | в данных появились поля, которых не было на первой странице |

## Разработка

```bash
mise install            # тулчейн 1.98.1, musl-таргет, cargo-dist
cargo test              # модульные и интеграционные тесты с мок-сервером
cargo build --release --target x86_64-unknown-linux-musl
APLAUT_E2E_BASE_URL=… APLAUT_ACCESS_TOKEN_FILE=… cargo test --test e2e -- --ignored --test-threads=1
APLAUT_E2E_WRITES=1 …  # то же плюс круги записи: создают и удаляют тестовые отзыв и товар
dist plan               # что соберёт релиз; релиз — push тега vX.Y.Z
```

Спека API вендорится в `spec/api.yaml` (см. `spec/README.md`); дизайн — в `docs/superpowers/specs/`.
Поддержка: support@aplaut.com.
