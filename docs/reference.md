# Справочник команд

Команда — `aplaut <ресурс> <действие> [аргументы] [флаги]`. Справка по одной команде — с полями
JSON, ошибками и кодами выхода — `aplaut <ресурс> <действие> --help`. Работа из скриптов и агентов,
коды выхода и ошибок — в [automation.md](automation.md).

| Ресурс | Действия |
|---|---|
| [`auth`](#auth) — токен доступа | [`login`](#auth-login), [`logout`](#auth-logout) |
| [`profile`](#profile) — профили | [`list`](#profile-list), [`get`](#profile-get), [`set`](#profile-set), [`delete`](#profile-delete), [`edit`](#profile-edit) |
| `reviews` — отзывы | [`scroll`](#scroll), [`get`](#get), [`create`](#reviews-create), [`comment`](#reviews-comment) |
| `products` — товары | [`scroll`](#scroll), [`get`](#get), [`create`](#products-create), [`update`](#products-update) |
| `questions` — вопросы | [`scroll`](#scroll), [`get`](#get) |
| [`self`](#self) — сам aplaut | [`update`](#self-update) |
| [`mcp`](#mcp) — MCP-сервер для агентов | — (`aplaut mcp`) |

## Глобальные флаги

Работают у любой команды.

| Флаг | Что делает |
|---|---|
| `--profile NAME` | профиль из `~/.config/aplaut` (по умолчанию — `APLAUT_PROFILE` или `default`) |
| `--base-url URL` | адрес API (по умолчанию `https://api.aplaut.io/v4`) |
| `--token-stdin` | прочитать токен из stdin |
| `--token-file PATH` | прочитать токен из файла (`-` — из stdin) |
| `--timeout SECONDS` | таймаут одного запроса, по умолчанию 30 |
| `--max-retries N` | сколько раз повторять запрос после 429, 5xx и сетевых сбоев, по умолчанию 6 (`create` и `comment` — только если сервер его точно не получил) |
| `-q`, `--quiet` | не печатать служебные сообщения; предупреждения и ошибки остаются |
| `--verbose` | отладочный вывод запросов, токен маскируется |
| `--no-color` | без цвета; то же — `NO_COLOR`, `APLAUT_NO_COLOR` или `TERM=dumb` |
| `--json`, `--no-input` | для скриптов и агентов, см. [automation.md](automation.md) |
| `--version` | версия CLI и спеки API |

## auth

Токен выпускается в ЛК: «Разработчикам» → OAuth-приложение со scope Platform API.

Порядок источников: `--token-stdin` / `--token-file` → `--profile` → `APLAUT_ACCESS_TOKEN_FILE`
→ `APLAUT_ACCESS_TOKEN` → `APLAUT_PROFILE` → профиль `default`. В CI и контейнерах лучше
`APLAUT_ACCESS_TOKEN_FILE`: значение переменной окружения видно в `docker inspect` и
`systemctl show`. Флага `--token` со значением нет — токен в argv виден в `ps`.

Другой стенд — `--base-url` или `APLAUT_BASE_URL`; `http://` допустим только для localhost. При явном
`--profile` берётся `base_url` этого профиля (или прод), а `APLAUT_BASE_URL` игнорируется — чтобы
токен профиля не ушёл на адрес из окружения.

### auth login

```text
aplaut auth login [--token-stdin | --token-file PATH] [--profile NAME] [--base-url URL] [-n]
```

Сохраняет токен в профиль; с `--base-url` — и адрес API. Токен на сервере не проверяется. Без
терминала, с `--no-input` и `--json` — только `--token-stdin` или `--token-file`.

| Флаг | Что делает |
|---|---|
| `-n`, `--dry-run` | всё проверить и показать, что будет сделано, ничего не меняя |

```bash
aplaut auth login                                   # ввод скрыт
aplaut auth login --profile staging --base-url https://api.staging.example/v4
echo "$TOKEN" | aplaut auth login --token-stdin --profile ci
aplaut auth login --token-file token.txt --profile ci --dry-run --json
```

### auth logout

```text
aplaut auth logout [--profile NAME] [-n]
```

Удаляет токен профиля. Токена не было — не ошибка.

| Флаг | Что делает |
|---|---|
| `-n`, `--dry-run` | всё проверить и показать, что будет сделано, ничего не меняя |

```bash
aplaut auth logout --profile ci
```

## profile

Профиль — имя, адрес API и описание; токен к нему задаёт только [`auth login`](#auth-login).
Команды `profile` токены не показывают и не меняют.

Файлы: `~/.config/aplaut/config.toml` (профили, `base_url`) и `~/.config/aplaut/credentials`
(токены), оба с правами 0600. Каталог — `$XDG_CONFIG_HOME/aplaut`, если переменная задана.

### profile list

```text
aplaut profile list
```

Профили, их base URL, есть ли токен; `*` — активный.

```text
$ aplaut profile list
  default — Прод, основной аккаунт
    - base_url: https://api.aplaut.io/v4 (по умолчанию)
    - токен: есть
* staging — Стенд для тестов
    - base_url: https://api.staging.example/v4
    - токен: нет
```

### profile get

```text
aplaut profile get <NAME>
```

Один профиль. Профиля нет — код 5, в подсказке — существующие.

```bash
aplaut profile get staging --json                      # одной строкой JSON
```

### profile set

```text
aplaut profile set <NAME> [--base-url URL|none] [--description TEXT|none] [-n]
```

Создаёт профиль или меняет его. Нужен хотя бы один из `--base-url` и `--description`.

| Флаг | Что делает |
|---|---|
| `--base-url URL\|none` | адрес API профиля; `none` — вернуть прод по умолчанию |
| `--description TEXT\|none` | описание для людей, в одну строку; `none` — убрать |
| `-n`, `--dry-run` | всё проверить и показать, что будет сделано, ничего не меняя |

```bash
aplaut profile set staging --base-url https://api.staging.example/v4 --description "Стенд для тестов"
aplaut profile set staging --base-url none             # вернуть прод по умолчанию
```

### profile delete

```text
aplaut profile delete <NAME> [-y] [-n]
```

Удаляет профиль вместе с его токеном. В терминале спрашивает подтверждение.

| Флаг | Что делает |
|---|---|
| `-y`, `--yes` | не спрашивать подтверждение; без терминала, с `--no-input` и `--json` — обязательно |
| `-n`, `--dry-run` | показать, что было бы удалено; подтверждение не нужно |

```bash
aplaut profile delete staging
aplaut profile delete staging --yes --json
```

### profile edit

```text
aplaut profile edit
```

Открывает `config.toml` в `$VISUAL` / `$EDITOR`.

```bash
aplaut profile edit
EDITOR="code --wait" aplaut profile edit
```

`profile edit` правит копию файла и заменяет оригинал только после проверки: при ошибке
предлагает открыть снова, а если файл изменил другой процесс (например, `auth login`), правки
остаются в копии. Без терминала `edit` не запускается — в скриптах используйте `profile set`.

GUI-редактор должен ждать, пока вы закроете файл. Командам `code`, `code-insiders`, `codium`,
`cursor`, `windsurf`, `zed`/`zeditor` и `subl` aplaut сам добавляет `--wait`, `gvim` — `--nofork`,
`kate` — `--block` (редактор узнаётся по имени программы, обёртки вроде `flatpak run` — нет).
Пока редактор открыт, в терминале видно «Жду, пока вы закроете файл в редакторе…»: в VS Code
закройте вкладку, в gvim и Kate — окно. Если редактор вернул управление сразу, не дав ничего
изменить, `edit` так и скажет и ничего не запишет — тогда задайте флаг ожидания в `VISUAL` сами.

### Файл config.toml

В начале `config.toml` — закомментированная справка со всеми опциями профиля и их значениями
по умолчанию; aplaut восстанавливает её при каждой своей записи файла. Неизвестные ключи —
ошибка: опечатка вроде `base-url` иначе молча отправила бы запросы на прод. Комментарии в файле
пропадают, когда aplaut сам его перезаписывает (`auth login`, `profile set`).

## Чтение: reviews, products, questions

У всех трёх ресурсов есть `scroll` (выгрузка обходом) и `get` (одна запись); флаги у них общие.

### scroll

```text
aplaut <reviews|products|questions> scroll [--filter EXPR] [--sort FIELD:asc] [--include REL,…]
    [--per-page N] [--state PATH] [--max-records N] [--format raw|jsonl|csv] [--fields COL,…]
```

Выгружает записи обходом по курсору (`GET /scroll/{records_type}`).

| Флаг | Что делает |
|---|---|
| `--filter EXPR` | фильтр `параметр:оператор:значение[,…]`, см. [ниже](#фильтр); **без него сервер отдаёт только записи за последние 30 дней** — CLI об этом предупреждает |
| `--sort FIELD:asc` | `updated_at:asc` (по умолчанию) или `created_at:asc` |
| `--include REL,…` | связанные объекты через запятую: у `reviews` — `author`, `product`, `comments`, `state_changes`; у `products` — `reviews_summary_item`; у `questions` — `author`, `product`, `answers` |
| `--per-page N` | записей на страницу, 1–100, по умолчанию 100 |
| `--state PATH` | файл стейта: повторный запуск той же команды продолжит с места остановки |
| `--max-records N` | остановиться, набрав не меньше N записей; граница — страница, поэтому записей может быть чуть больше N (со `--state` можно продолжить) |
| `--format FORMAT` | `raw` (по умолчанию), `jsonl` или `csv`, см. [форматы](#форматы-вывода) |
| `--fields COL,…` | колонки CSV и их порядок, см. [колонки CSV](#колонки-csv) |

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

- Доставка «хотя бы один раз»: после сбоя последняя страница может прийти повторно, поэтому
  убирайте дубли по `id`.
- Лимиты API: 1 запрос в 2 секунды и 5 открытий обхода в минуту на ключ. CLI соблюдает их сам;
  не запускайте параллельно несколько выгрузок с одним ключом.
- Курсор обхода нельзя повторить: сервер на повтор отдаёт уже следующую страницу. Если запрос
  продолжения оборвался и неизвестно, дошёл ли он, CLI не продолжает вслепую (`scroll_interrupted`,
  затем `scroll_position_uncertain`), а печатает команду для нового обхода с границы последней
  выданной записи: `--filter updated_at:gte:<значение> --state <новый файл>`.

#### Фильтр

`--filter` — блок `параметр:оператор:значение`, несколько блоков — через запятую. Параметр и
оператор проверяются до запроса: неверный — код 2, в подсказке — допустимые.

| Оператор | Значение |
|---|---|
| `gt`, `gte`, `lt`, `lte` | больше, не меньше, меньше, не больше |
| `eq`, `neq` | равно, не равно |
| `in` | одно из значений через `\|`, от 1 до 25 |
| `exists` | `true` или `false` |

```bash
aplaut reviews scroll --filter 'updated_at:gte:2024-01-01T00:00:00Z,rating:in:4|5' --format jsonl
```

Параметры по ресурсу (спека API 4.1.0):

| Ресурс | Параметры |
|---|---|
| `reviews` | `created_at`, `updated_at`, `published_at`, `state`, `context_type`, `product_id`, `product_group_id`, `brand_id`, `category_id`, `rating`, `origin`, `syndication_source`, `order_number`, `lang`, `verified`, `featured`, `recommended` |
| `products` | `created_at`, `updated_at`, `last_imported_at`, `summary_updated_at`, `external_id`, `group_id`, `brand_id`, `category_id`, `vendor_code`, `available`, `price`, `rating` |
| `questions` | `created_at`, `updated_at`, `published_at`, `imported_at`, `state`, `context_type`, `product_id`, `product_external_id`, `product_group_id`, `brand_id`, `category_id`, `lang`, `origin`, `has_published_answers` |

### get

```text
aplaut <reviews|products|questions> get <ID> [--include REL,…] [--format raw|jsonl|csv] [--fields COL,…]
```

Одна запись по внутреннему идентификатору или `external_id` (`GET /{records_type}/{id}`).

| Флаг | Что делает |
|---|---|
| `--include REL,…` | связанные объекты: у `reviews` — `author`, `product`, `comments`, `state_changes`; у `products` — `reviews_summary_item`, `reviews`, `questions`, `brand`, `category`; у `questions` — `author`, `product`, `answers` |
| `--format FORMAT` | как у `scroll`: `raw` — тело ответа одной строкой; `jsonl` — запись с подставленными объектами `--include`; `csv` — заголовок и строка |
| `--fields COL,…` | колонки CSV — по правилам `scroll` |

```bash
aplaut reviews get 5f1c2a9e8b7d6c5b4a3f2e1d                              # по внутреннему id
aplaut reviews get crm-4211 --include author,comments --format jsonl    # по внешнему id
aplaut products get 444772 --format csv --fields id,name
```

Id с точкой или `/` CLI не отправляет (код 2): сервер отрезает всё после точки, даже
закодированной, и найдёт другую запись, а `/` не находит вовсе. `--include` проверяется по спеке
до запроса. Записи нет — код 5.

### Форматы вывода

| `--format` | Что в stdout |
|---|---|
| `raw` (по умолчанию) | тело каждого ответа API как есть, одна строка на страницу |
| `jsonl` | запись на строку; связанные объекты из `--include` подставлены в `relationships` |
| `csv` | заголовок и строки; колонки — `id`, `type`, атрибуты, `<связь>_ref`, `<связь>.<атрибут>` |

В `jsonl` ключи идут по алфавиту; точные байты ответа есть только в `raw`. CSV не экранирует
формулы (`=…`): в Excel импортируйте файл как текст.

### Колонки CSV

`--fields id,rating,pros,cons,body` выбирает колонки CSV и их порядок; имена — как в заголовке
(`product.name` требует `--include product`). Заголовок — ровно этот список, даже у пустой
выгрузки: загрузка в DWH не ломается от новых полей API, а лишние персональные данные
(`author_email`, `author_phone`, `author_ip`) не попадают на диск. Колонки отбирает CLI — API
отдаёт все поля, трафик тот же. С `raw` и `jsonl` — код 2.

Имена сверяются с первой страницей обхода: опечатка — код 2 до записи чего-либо, с подсказкой
(«может, rating?») и списком колонок. Продолжение со `--state` берёт колонки из стейта — другой
`--fields` будет ошибкой; если имя проверить не удалось (на первой странице не было, например,
объектов `author`), колонка останется пустой, а CLI один раз предупредит, когда это выяснится.

## Запись: reviews, products

У API нет идемпотентности. CLI повторяет запрос сам, только если сервер точно его не обработал
(429, 503, соединение не установилось). После таймаута, обрыва или 5xx — `request_outcome_unknown`:
проверьте, выполнена ли запись, прежде чем повторять. Исключение — `products update` без смены
`external_id`: правка повторяется, как чтение.

Атрибуты и их типы — из схемы тела запроса в спеке. Неизвестный атрибут, значение не того типа
или не из перечисления, нет обязательного — код 2 до отправки, атрибут назван в `field`. Для
атрибутов без флага — `--data FILE` (или `-` — из stdin) с JSON-объектом; флаги перекрывают его
ключи. `-n` (`--dry-run`) показывает метод, путь и тело запроса и ничего не отправляет.

### reviews create

```text
aplaut reviews create [--rating N] [--body TEXT] [--pros TEXT] [--cons TEXT] [--product-id ID]
    [--external-id ID] [--author-name TEXT] [--author-email EMAIL] [--state STATE] [--data FILE|-] [-n]
```

Создаёт отзыв (`POST /reviews`). Обязательны `rating` и хотя бы одно из `body`, `pros`, `cons`.

| Флаг | Что делает |
|---|---|
| `--rating N` | оценка, 1–5 |
| `--body TEXT` | текст отзыва |
| `--pros TEXT`, `--cons TEXT` | достоинства и недостатки |
| `--product-id ID` | товар (обычно `offer.id` из YML); без него отзыв — о компании |
| `--external-id ID` | id отзыва в вашей системе: по нему `get` проверит, создан ли отзыв после сбоя |
| `--author-name TEXT`, `--author-email EMAIL` | автор |
| `--state STATE` | статус модерации: `published`, `waiting` (по умолчанию), `banned`, `held` |
| `--data FILE\|-` | остальные атрибуты JSON-объектом: `photos`, `tags`, `rating_details`, `custom_attributes`, даты… |
| `-n`, `--dry-run` | показать запрос, не отправляя |

```bash
# Отзыв о компании (без --product-id); флаги — частые атрибуты:
aplaut reviews create --rating 5 --body "Спасибо!" --author-name "Анна" --external-id crm-4211

# Остальные атрибуты — JSON-объектом в --data; флаги перекрывают его ключи:
aplaut reviews create --data review.json --state published
```

Передавайте `--external-id` — тогда после `request_outcome_unknown` проверка —
`aplaut reviews get <external_id>`. Недоступные URL в `photos` сервер молча отбрасывает.

### reviews comment

```text
aplaut reviews comment <REVIEW_ID> [--text TEXT] [--author-name TEXT] [--author-email EMAIL]
    [--state STATE] [--parent-id ID] [--external-id ID] [--data FILE|-] [-n]
```

Ответ магазина на отзыв (`POST /reviews/{id}/relationships/comments`). `REVIEW_ID` — внутренний
идентификатор отзыва или его `external_id`. Обязателен `text`.

| Флаг | Что делает |
|---|---|
| `--text TEXT` | текст комментария |
| `--author-name TEXT`, `--author-email EMAIL` | автор |
| `--state STATE` | статус модерации: `published`, `waiting` (по умолчанию), `banned` |
| `--parent-id ID` | внутренний id родительского комментария — ответ на комментарий |
| `--external-id ID` | id комментария в вашей системе: по нему его проще найти после сбоя |
| `--data FILE\|-` | остальные атрибуты: `files`, `hide_my_data`, `author_external_id`, `external_parent_id`… |
| `-n`, `--dry-run` | показать запрос, не отправляя |

```bash
# Сначала план без отправки, потом отправка:
aplaut reviews comment crm-4211 --text "Спасибо за отзыв!" -n
aplaut reviews comment crm-4211 --text "Спасибо за отзыв!"
```

После `request_outcome_unknown` проверка — `aplaut reviews get <REVIEW_ID> --include comments`.

### products create

```text
aplaut products create --external-id ID --name TEXT --url URL [--price N] [--available true|false]
    [--description TEXT] [--group-id ID] [--category-id ID] [--category-name TEXT] [--brand-name TEXT]
    [--data FILE|-] [-n]
```

Создаёт товар (`POST /products`). Обязательны `external_id`, `name` и `url`.

| Флаг | Что делает |
|---|---|
| `--external-id ID` | id товара в вашей системе (обычно `offer.id` из YML) |
| `--name TEXT` | название |
| `--url URL` | URL карточки товара |
| `--price N` | цена |
| `--available true\|false` | наличие |
| `--description TEXT` | описание |
| `--group-id ID` | внешний идентификатор группы (варианты одного товара) |
| `--category-id ID` | категория по `external_id` из YML |
| `--category-name TEXT` | категория по имени; нет такой — будет создана |
| `--brand-name TEXT` | бренд по имени; нет такого — будет создан |
| `--data FILE\|-` | остальные атрибуты: `category_names`, `picture_urls`, `recommended_product_ids`, `custom_attributes`, даты… |
| `-n`, `--dry-run` | показать запрос, не отправляя |

```bash
aplaut products create --external-id 60757 --name "Transcend StoreJet 1 ТБ" --url https://shop.example/p/60757 --price 5990 --category-id 297
```

Без категории товар попадает в корневую. Второй товар с тем же `external_id` сервер не создаст
(422 `is already taken`), поэтому повтор после сбоя безопасен.

### products update

```text
aplaut products update <ID> [--external-id NEW_ID] [--name TEXT] [--url URL] [--price N] …
    [--data FILE|-] [-n]
```

Меняет товар (`PUT /products/{id}`). `ID` — внутренний идентификатор или `external_id`. Флаги —
как у [`create`](#products-create), плюс `--external-id NEW_ID` переименовывает товар.

```bash
aplaut products update 60757 --price 5490 --available true
echo '{"price":5490,"custom_attributes":{"color":"black","old_key":null}}' | aplaut products update 60757 --data -
```

`update` меняет только переданные атрибуты; `custom_attributes` сливаются, `null` очищает. Правка
повторяется после сбоя, как чтение; со сменой `external_id` — только если сервер точно её не получил.
Вычисляемые `rating`, `reviews_count`, `recommended` сервер не меняет, поэтому они — `unknown_attribute`.

## self

### self update

```text
aplaut self update [-n]
```

Ставит последний релиз тем же установщиком, в тот же каталог. Работает, только если aplaut
поставлен установщиком, — подробности в [README](../README.md#обновление).

| Флаг | Что делает |
|---|---|
| `-n`, `--dry-run` | только проверить, есть ли новая версия |

## Переменные окружения

| Переменная | Что задаёт |
|---|---|
| `APLAUT_PROFILE` | профиль по умолчанию вместо `default` |
| `APLAUT_ACCESS_TOKEN_FILE` | файл с токеном |
| `APLAUT_ACCESS_TOKEN` | сам токен (виден в `docker inspect` — лучше `APLAUT_ACCESS_TOKEN_FILE`) |
| `APLAUT_BASE_URL` | адрес API; при явном `--profile` игнорируется |
| `APLAUT_CLI_GITHUB_TOKEN` | токен GitHub для `self update` — в CI, где 60 запросов в час с IP мало |
| `NO_COLOR`, `APLAUT_NO_COLOR` | без цвета |
| `VISUAL`, `EDITOR` | редактор для `profile edit` |
| `XDG_CONFIG_HOME` | где искать каталог `aplaut` вместо `~/.config` |

## mcp

```text
aplaut mcp [--allow-writes]
```

MCP-сервер через stdin/stdout: команды aplaut — инструментами для агентов. Глобальные флаги
(`--profile`, `--base-url`, `--token-file`, `--timeout`, `--max-retries`, `--verbose`) передаются
каждому вызову; `--token-stdin` и `--token-file -` недоступны — stdin занят протоколом.

| Флаг | Что делает |
|---|---|
| `--allow-writes` | открыть запись: инструменты `create`, `update` и `comment` (список — `aplaut mcp --help`) |

Подключение, инструменты и выгрузка — в [mcp.md](mcp.md).
