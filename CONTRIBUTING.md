# Разработка

```bash
mise install            # тулчейн 1.98.1, musl-таргет, cargo-dist; для musl-сборки нужен пакет musl (musl-gcc)
cargo test              # модульные и интеграционные тесты с мок-сервером
mise run build          # статический бинарь как в релизе: target/x86_64-unknown-linux-musl/dist/aplaut
mise run install        # то же и поставить в ~/.local/bin (другой каталог — APLAUT_CLI_INSTALL_DIR)
APLAUT_E2E_BASE_URL=… APLAUT_ACCESS_TOKEN_FILE=… cargo test --test e2e -- --ignored --test-threads=1
APLAUT_E2E_WRITES=1 …  # то же плюс круги записи: создают и удаляют тестовые отзыв и товар
dist plan               # что соберёт релиз; релиз — push тега vX.Y.Z
```

Спека API вендорится в `spec/api.yaml` (см. [spec/README.md](spec/README.md)).

Каталог кодов ошибок и предупреждений в [docs/automation.md](docs/automation.md) — контракт для
агентов: тест `tests/catalog.rs` проверяет, что там описан каждый код из `src/`.
