# aplaut

Консольный клиент [Aplaut Platform API](https://aplaut.com/docs/api-references/platform/):
выгрузка отзывов, товаров и вопросов для DWH, cron и агентов.

## Установка

Скрипт установки и архивы — на странице релизов GitHub: `aplaut-cli-installer.sh` и
`aplaut-cli-x86_64-unknown-linux-musl.tar.xz` с `.sha256`. Бинарь статический и работает на
любом x86-64 Linux без зависимостей. Проверить происхождение бинаря:
`gh attestation verify <файл> -R aplaut-tech/aplaut-cli`.

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

## Выгрузка

```bash
# Полная выгрузка отзывов в JSONL:
aplaut reviews scroll --filter updated_at:gte:2000-01-01T00:00:00Z --format jsonl > reviews.jsonl

# С продолжением после сбоя (повторный запуск продолжит с места остановки):
aplaut reviews scroll --filter updated_at:gte:2024-01-01T00:00:00Z --state reviews.state.json --format jsonl >> reviews.jsonl

# Просмотр:
aplaut products scroll --format csv --max-records 500 | tw
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

## Форматы

| `--format` | Что в stdout |
|---|---|
| `raw` (по умолчанию) | тело каждого ответа API как есть, одна строка на страницу |
| `jsonl` | запись на строку; связанные объекты из `--include` подставлены в `relationships` |
| `csv` | заголовок и строки; колонки — `id`, `type`, атрибуты, `<связь>_ref`, `<связь>.<атрибут>` |

В `jsonl` ключи идут по алфавиту; точные байты ответа есть только в `raw`. CSV не экранирует
формулы (`=…`): в Excel импортируйте файл как текст.

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

Без терминала последняя строка stderr при ошибке — JSON:
`{"ok":false,"command":"reviews.scroll","error":{"code","message","field","retryable","hint","request_id"}}`.

## Разработка

```bash
mise install            # тулчейн 1.98.1, musl-таргет, cargo-dist
cargo test              # модульные и интеграционные тесты с мок-сервером
cargo build --release --target x86_64-unknown-linux-musl
APLAUT_E2E_BASE_URL=… APLAUT_ACCESS_TOKEN_FILE=… cargo test --test e2e -- --ignored --test-threads=1
dist plan               # что соберёт релиз; релиз — push тега vX.Y.Z
```

Спека API вендорится в `spec/api.yaml` (см. `spec/README.md`); дизайн — в `docs/superpowers/specs/`.
Поддержка: support@aplaut.com.
