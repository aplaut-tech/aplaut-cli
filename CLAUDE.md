# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Проект

`aplaut` — консольный клиент Aplaut Platform API на Rust (бинарь `aplaut`, библиотека `aplaut_cli`):
выгрузка и запись отзывов, товаров и вопросов для DWH, cron и агентов. Справка CLI, сообщения,
документация и комментарии в коде — на русском; сообщения коммитов — на английском, conventional commits
(`feat:`, `fix:`, `chore:`, `docs:`, `ci:`, `refactor:`, `test:`).

Репозиторий публичный. `docs/superpowers/` (внутренние спеки и планы) в `.gitignore` — не коммитить.
Адрес стейджинга и токены — только в `.envrc` (тоже в `.gitignore`), не в код и не в коммиты.

## Команды

```bash
mise install                                   # тулчейн 1.98.1, musl-таргет, cargo-dist
cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test   # то же, что CI (Linux и macOS)
cargo test --test scroll                       # один файл интеграционных тестов
cargo test --test agent auth_login             # тесты по подстроке имени
cargo test --lib output::csv                   # модульные тесты одного модуля
mise run build                                 # статический musl-бинарь как в релизе (нужен musl-gcc)
mise run install                               # то же + в ~/.local/bin
APLAUT_E2E_BASE_URL=… APLAUT_ACCESS_TOKEN_FILE=… cargo test --test e2e -- --ignored --test-threads=1
APLAUT_E2E_WRITES=1 …                          # e2e плюс круги записи (создают и удаляют тестовые объекты)
dist plan                                      # что соберёт релиз
```

Релиз: версия в `Cargo.toml` (и `Cargo.lock`), ссылка на установщик в `README.md`, версия в
`tests/cli.rs` → коммит `chore: release X.Y.Z` → push тега `vX.Y.Z`; cargo-dist собирает 4 таргета
(`dist-workspace.toml`).

## Архитектура

Путь команды: `lib::run` → clap (`cli`) → `commands::dispatch` → команда проверяет всё локальное,
затем `commands::connect` (токен и base URL: флаги → env → профиль) → `http::ApiClient` → `ops`
(`scroll`, `write`) → `output` (sink формата). Команда возвращает `Outcome` (`result` и канал), а
`lib::run` печатает конверт `--json` или рендерит `CliError`. Любая ошибка — `CliError` с `code`,
`hint`, `retryable`, `retry_after`; код выхода — `error::Exit`.

- **Спека API → код.** `spec/api.yaml` (вендоренная, см. `spec/README.md`) → `build.rs` →
  `spec_tables.rs`, доступ через `spec.rs`: фильтры и include scroll, include `get`, схемы тел записи.
  `resources.rs` — таблица «ресурс × глагол → операция API». Добавить ресурс = строка в `resources.rs`
  + вариант в `cli::Command`; `tests/spec_drift.rs` проверяет соответствие команд и операций спеки.
- **HTTP** (`http.rs`): синхронный `ureq`, один клиент на процесс. Троттлинг проактивный (2 запроса/с;
  scroll — 1 запрос в 2 с и 5 открытий в минуту на ключ). Повторы по политике `Replay`: POST
  повторяется, только если сервер точно не получил запрос, иначе `request_outcome_unknown`.
- **Scroll** (`ops/scroll.rs`, `state.rs`): курсор не идемпотентен — перед продолжением стейт
  помечается `in_flight`. Стейт пишется атомарно и только в точках фиксации формата
  (`RecordSink::write_page` → `Commit::Durable`): at-least-once, хвост прошлой страницы отсекается по id.
- **Форматы** (`output`): `raw` — тела страниц как есть, `jsonl` — запись со связями из `included`,
  `csv` — проекция `output/tabular` с `--fields`.
- **Время** — через трейт `Clock` (`clock.rs`): тесты ретраев и троттлинга без реального ожидания.
- **Async** — только `update.rs` (`self update` через `axoupdater`) и `src/mcp` (tokio current-thread).
  Остальной CLI синхронный и crash-only, без обработчиков сигналов.
- **MCP** (`src/mcp`, `aplaut mcp`): сервер на `rmcp` (stdio). Каталог инструментов выводится из
  `resources::ALL` × `Verb` (`mcp/tools.rs`), схемы — из спеки, описания — из справки clap. Каждый
  вызов — дочерний процесс того же бинаря с `--json --no-input` (`mcp/invoke.rs`); значения — одним
  токеном `--flag=value`, id — после `--`. Новая команда CLI должна стать инструментом или попасть в
  `MCP_EXCLUDED` — иначе падает страж в `mcp/tools.rs`. Как добавить инструмент — `docs/mcp.md`.

## Контракты, которые нельзя сломать

- stdout — данные или итог, всё для человека — в stderr через `term::Reporter`. С `--json` stderr — только
  конверт; у `scroll` и `get` конверт идёт в stderr, потому что stdout занят данными.
- Конверт (`envelope.rs`), коды ошибок, предупреждений и выхода — публичный контракт для агентов: только
  добавляются. Каждый новый код — строка в каталоге `docs/automation.md`, иначе падает
  `tests/catalog.rs`. Поля `result` каждой команды описаны в её `--help` (`cli/help.rs`).
- Локальные проверки — до сети: ошибка в параметрах не должна тратить квоту открытий scroll.
- CLI следует clig.dev.
- Комментарии вида «стейджинг, 2026-09-24» и «спека reviews-write W5» фиксируют наблюдённое поведение
  сервера и решения из внутренних спек — не удалять без причины.

## Тесты

Интеграционные тесты запускают настоящий бинарь через `tests/support`: `aplaut()` с чистыми `HOME` и
`XDG_CONFIG_HOME` и пайпами вместо TTY, `MockServer` — HTTP-мок, `TempDir` — без крейта `tempfile`.
`tests/e2e.rs` — только `#[ignore]`, против живого стенда; адрес стенда берётся из env.
