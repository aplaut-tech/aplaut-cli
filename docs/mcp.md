# MCP-сервер

`aplaut mcp` отдаёт команды aplaut агентам (Claude Code, Claude Desktop, Cursor и другим MCP-клиентам)
инструментами со схемами из спеки API. Сервер говорит по MCP через stdin/stdout; каждый вызов —
команда aplaut с `--json`, поэтому ошибки, коды и `hint` — те же, что в [automation.md](automation.md).

## Подключение

Сначала токен: `aplaut auth login` (или `--profile <имя>`). Через MCP токен не передаётся.

Claude Code:

```bash
claude mcp add aplaut -- aplaut mcp
claude mcp add --scope project aplaut -- aplaut mcp --profile staging   # в .mcp.json проекта
```

Claude Desktop (`claude_desktop_config.json`), Cursor (`.cursor/mcp.json`):

```json
{"mcpServers": {"aplaut": {"command": "aplaut", "args": ["mcp", "--profile", "staging"]}}}
```

Если `aplaut` не в `PATH` клиента — укажите полный путь, например `~/.local/bin/aplaut`.

## Окружение и запись

Профиль, base URL и токен задаются при запуске: `--profile`, `--base-url`, `--token-file`,
переменные `APLAUT_PROFILE`, `APLAUT_ACCESS_TOKEN`, `APLAUT_BASE_URL`. Агент их не меняет. Два
окружения — два сервера:

```bash
claude mcp add aplaut-staging -- aplaut mcp --profile staging --allow-writes
claude mcp add aplaut-prod -- aplaut mcp --profile prod
```

Без `--allow-writes` инструментов записи нет: отзывы и ответы публикуются на сайте магазина. У
инструментов записи есть `dry_run` — проверить и показать запрос, ничего не отправляя.

## Инструменты

| Инструмент | Команда | Режим |
|---|---|---|
| `reviews_scroll`, `products_scroll`, `questions_scroll` | `<ресурс> scroll` | всегда |
| `reviews_get`, `products_get`, `questions_get`, `consumers_get`, `orders_get` | `<ресурс> get` | всегда |
| `reviews_create`, `products_create`, `questions_create`, `consumers_create`, `orders_create` | `<ресурс> create` | `--allow-writes` |
| `reviews_comment` | `reviews comment` | `--allow-writes` |
| `reviews_update`, `products_update`, `questions_update`, `consumers_update`, `orders_update` | `<ресурс> update` | `--allow-writes` |

Параметры повторяют флаги команды (`max_records` ↔ `--max-records`), атрибуты записи — поля схемы
спеки. Формат по умолчанию — `jsonl`: запись со связями из `include` одной строкой.

`update` отзывов (и других ресурсов, где API на `PUT` с неизвестным id создаёт объект) сначала
проверяет, что объект есть: нет — `not_found`, ничего не создано. Параметр `upsert: true` разрешает
создать объект с внешним id из `id`; в `result` — `created: true`.

`consumers_get`, `orders_get` и инструменты записи клиентов и заказов работают с персональными
данными (e-mail, телефон, имя): давайте их агенту, только если это нужно задаче.
`orders_update` заменяет `order_lines` целиком.

Ответ: первый текстовый блок — JSON-конверт, как у `--json`; второй — данные, если они есть. При
ошибке `isError: true`, в конверте — `error.code`, `hint`, `retryable`.

## Выгрузка

- Без `output_file` — данные в ответе, не больше 100 записей за вызов (`max_records`, по умолчанию 20).
  Со `state` повторный вызов отдаёт следующую порцию.
- С `output_file` — любой объём в файл (путь от рабочего каталога сервера), в ответе — итог и
  абсолютный путь. Существующий файл без `state` не перезаписывается (`output_exists`), с
  `overwrite: true` — перезаписывается; со `state` — дописывается, как `>>` в CLI.
- Без `filter` сервер отдаёт только последние 30 дней — предупреждение `default_filter` в `warnings`.
- Размер страницы фиксируется первым вызовом со `state`: следующие порции и выгрузка «остальное в
  файл» с тем же стейтом идут страницами того же размера.

`output_file` и `state` — любые пути, куда может писать пользователь, запустивший сервер, даже без
`--allow-writes`. Текст отзывов приходит от покупателей, поэтому не разрешайте агенту вызовы `scroll`
с `output_file` не глядя: проверяйте путь в запросе на разрешение.

## Как добавить инструмент

- Новый ресурс со знакомыми глаголами (`scroll`, `get`, `create`, `comment`, `update`) — строка в
  `src/resources.rs` и вариант `cli::Command`, как для CLI. Инструменты появятся сами: схемы — из
  `spec/api.yaml`, описания — из справки clap. Списки инструментов в справке `aplaut mcp` и в
  `instructions` сервера строятся из той же таблицы. Добавьте имена в ожидаемые списки `tests/mcp.rs`.
- Новый вид глагола — вариант `Verb`; компилятор потребует ветку в `tool()` в `src/mcp/tools.rs`:
  параметры, argv (`src/mcp/invoke.rs`), аннотации.
- Команда не про ресурс — по умолчанию в `MCP_EXCLUDED` с причиной; тест
  `every_cli_command_is_a_tool_or_excluded` не даст забыть.

Имена инструментов — контракт для агентов: только добавляются, переименование — со сменой
major-версии.
