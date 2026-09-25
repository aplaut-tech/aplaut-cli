# Aplaut CLI

Консольный клиент [Aplaut Platform API](https://aplaut.com/docs/api-references/platform/):
выгрузка отзывов, товаров и вопросов для DWH, cron и агентов.

## Установка

```bash
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/aplaut-tech/aplaut-cli/releases/latest/download/aplaut-cli-installer.sh | sh
```

Работает на Linux (x86-64, ARM64) и macOS. Установщик кладёт `aplaut` в `~/.local/bin`; если
этого каталога нет в `PATH`, пропишет его в `~/.profile` и `~/.zshrc` — перезапустите шелл.

Дальше — токен: `aplaut auth login`, см. [справочник](docs/reference.md#auth).

### Конкретная версия

Тот же скрипт из нужного релиза:

```bash
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/aplaut-tech/aplaut-cli/releases/download/v0.3.0/aplaut-cli-installer.sh | sh
```

### Серверы, контейнеры и CI

Без правки профиля шелла:

```bash
# только бинарь в указанный каталог, без ~/.profile и служебных файлов
curl … | APLAUT_CLI_UNMANAGED_INSTALL=/opt/aplaut/bin sh
# обычная установка, но PATH не трогать
curl … | APLAUT_CLI_NO_MODIFY_PATH=1 sh
```

## Обновление

```bash
aplaut self update        # последний релиз — тем же установщиком, в тот же каталог
aplaut self update -n     # только проверить, есть ли новая версия
```

Работает, если `aplaut` поставлен установщиком выше: тот оставляет файл установки
`~/.config/aplaut-cli/aplaut-cli-receipt.json`. Поставленный иначе (`cargo install`,
`APLAUT_CLI_UNMANAGED_INSTALL`) обновляйте тем же способом, которым ставили.
Версия 0.1.0 команды ещё не знает — один раз переставьте её установщиком.

Без токена GitHub даёт 60 запросов в час с одного IP; в CI задайте `APLAUT_CLI_GITHUB_TOKEN`.
В минимальном Linux-контейнере для `self update` нужен пакет `ca-certificates`: в отличие от остальных
команд, она проверяет сертификаты по системному хранилищу.

## Удаление

Удалите `~/.local/bin/aplaut`, каталог `~/.config/aplaut-cli/`, файл
`~/.config/fish/conf.d/aplaut-cli.env.fish` и строку с `aplaut-cli/env.sh` в `~/.profile` /
`~/.zshrc`. Токены и профили лежат отдельно, в `~/.config/aplaut/`.

## Документация

- [Справочник команд](docs/reference.md) — ресурсы, действия, флаги и примеры.
- [Для агентов и скриптов](docs/automation.md) — `--json`, коды выхода, каталог ошибок и предупреждений.
- [MCP-сервер](docs/mcp.md) — `aplaut mcp`: работа через агента (Claude Code, Claude Desktop, Cursor).
- `aplaut <ресурс> <действие> --help` — справка по одной команде, с полями JSON.
- [Разработка](CONTRIBUTING.md).

Поддержка: support@aplaut.com.
