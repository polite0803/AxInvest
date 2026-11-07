# AxInvest

**Локально-ориентированное рабочее место для исследования акций A**: настольное приложение на Tauri 2 + Rust, объединяющее в одном рабочем пространстве получение рыночных и фундаментальных данных, решения по акциям на четырёх горизонтах, скрининг и разбор результатов, бумажные портфели, количественный бэктест, автоматизированные задачи и работу с ИИ. Все бизнес-данные по умолчанию хранятся в локальной SQLite, ключи API шифруются AES-256-GCM, а основные сценарии не зависят от сторонних облачных сервисов.

> **Языковые версии**: [English](./README-EN.md) | [繁體中文](./README-ZH-TW.md) | [日本語](./README-JA.md) | [한국어](./README-KO.md) | [Français](./README-FR.md) | [Deutsch](./README-DE.md) | [Español](./README-ES.md) | [Русский](./README-RU.md) | [हिन्दी](./README-HI.md) | [العربية](./README-AR.md)

---

## Позиционирование проекта

AxInvest создан на основе [`polite0803/AxAgent`](https://github.com/polite0803/AxAgent); общая ИИ-основа наследуется от upstream, а инвестиционные бизнес-возможности развиваются в этом репозитории. Проект ориентирован на «воспроизводимую цепочку инвестиционного исследования», а не на набор функций:

- **Единая модель времени**: `live` и `as-of` проходят через все инвестиционные возможности; исторические пакеты можно воспроизводить как есть, а выборки — калибровать.
- **Единая цепочка решений**: от построения пула кандидатов и независимых решений на четырёх горизонтах до сохранения в БД, атрибуции и эволюции через рефлексию — критерий один.
- **Локальный приоритет**: по умолчанию всё сохраняется в SQLite, без облачной зависимости; данные и ключи остаются на локальной машине.

---

## Навигация по функциям (16 функциональных точек)

Точка входа в `docs/` — индекс с двумя представлениями [docs/INDEX.md](./docs/INDEX.md): функциональное представление организовано по 16 функциональным точкам, архивное — по категориям предложения / планы / аудиты / справочные материалы; сейчас зарегистрировано 265 документов. Для каждой функциональной точки есть руководство, в котором описаны текущее поведение, точки входа в коде, переключатели конфигурации, известные ограничения и история изменений версий.

**Бизнес-домен (9)**

| Функциональная точка                     | slug                   | Руководство                                                        |
| ---------------------------------------- | ---------------------- | ------------------------------------------------------------------ |
| Цепочка решений по акциям                | `stock-decision-chain` | [stock-decision-chain.md](./docs/features/stock-decision-chain.md) |
| Интеллектуальный отбор трендов           | `serenity-screening`   | [serenity-screening.md](./docs/features/serenity-screening.md)     |
| Воспроизведение «путешествия во времени» | `time-travel-replay`   | [time-travel-replay.md](./docs/features/time-travel-replay.md)     |
| Рыночные данные и источники данных       | `market-data`          | [market-data.md](./docs/features/market-data.md)                   |
| Пакеты возможностей OPC                  | `opc-capability-packs` | [opc-capability-packs.md](./docs/features/opc-capability-packs.md) |
| Количественный бэктест                   | `quant-backtest`       | [quant-backtest.md](./docs/features/quant-backtest.md)             |
| Рефлексия и эволюция                     | `reflection-evolution` | [reflection-evolution.md](./docs/features/reflection-evolution.md) |
| Центр задач и планировщик                | `task-center`          | [task-center.md](./docs/features/task-center.md)                   |
| Бумажные позиции и торговый мост         | `paper-portfolio`      | [paper-portfolio.md](./docs/features/paper-portfolio.md)           |

**Платформенный домен (7)**

| Функциональная точка             | slug                 | Руководство                                                    |
| -------------------------------- | -------------------- | -------------------------------------------------------------- |
| Система плагинов                 | `plugin-system`      | [plugin-system.md](./docs/features/plugin-system.md)           |
| Качество данных и шлюзы          | `data-quality-gates` | [data-quality-gates.md](./docs/features/data-quality-gates.md) |
| Runtime и агенты                 | `agent-runtime`      | [agent-runtime.md](./docs/features/agent-runtime.md)           |
| Провайдеры моделей и стриминг    | `provider-models`    | [provider-models.md](./docs/features/provider-models.md)       |
| Хранилище и миграции базы данных | `storage-migration`  | [storage-migration.md](./docs/features/storage-migration.md)   |
| Оболочка frontend                | `ui-shell`           | [ui-shell.md](./docs/features/ui-shell.md)                     |
| Сборка и CI                      | `build-ci`           | [build-ci.md](./docs/features/build-ci.md)                     |

---

## Быстрый старт

Требования к окружению: Node.js ≥ 24, Rust ≥ 1.85 (edition 2024), npm ≥ 10. В Windows требуются MSVC Build Tools, в macOS — Xcode Command Line Tools, в Linux — `build-essential`, `libwebkit2gtk-4.1-dev`, `libssl-dev`.

```bash
npm install
npm run dev           # Браузерный режим (frontend использует localStorage mock, Tauri не нужен)
npm run tauri dev     # Настольное приложение (Vite HMR + окно Tauri)
```

Сборка и артефакты:

```bash
npm run build         # Проверка типов + production-сборка frontend
npm run tauri:build   # Установочный пакет для настольного клиента
```

---

## Слои архитектуры (8 корней workspace)

Rust-часть разделена по направлению зависимостей на **8 корней Cargo workspace**, всего **42 библиотечных crate**; `src-tauri` выступает корнем композиции (crate основного бинарника + `axagent-mobile`). Железное правило направления зависимостей: `компоненты → harness ← реализации`.

| Корень workspace  | Роль                       | Число crate | Основное содержимое                                                                                  |
| ----------------- | -------------------------- | ----------- | ---------------------------------------------------------------------------------------------------- |
| `axagent-core`    | foundation                 | 7           | trait и DTO `harness`, `entities`, `plugin-proto`, `disk-cache`, `rt-theme`                          |
| `axagent-kit`     | implementor                | 1           | Общий слой реализации `kit`                                                                          |
| `axagent-ai`      | consumer + implementor     | 6           | `agent`, `orchestrator`, `providers`, `mcp`, `prompt-guard`, `trajectory`                            |
| `axagent-infra`   | implementor                | 12          | `dao`, `storage`, `migration`, `crypto`, `search`, `telemetry`, `document-parser` и др.              |
| `axagent-quant`   | implementor / consumer     | 5           | `astock-data`, `analysis-engine`, `company-runtime`, `quant`, `market-sim`                           |
| `axagent-plugins` | implementor + foundation   | 2           | `plugins`, `schema-gen`                                                                              |
| `axagent-runtime` | wiring + consumer + hybrid | 8           | `runtime`, `runtime-core`, `tools`, `rt-workflow`, `rt-messaging`, `gateway`, `crdt`, `notification` |
| `src-tauri`       | wiring                     | 2           | Команды Tauri, инициализация и сборка, `axagent-mobile`                                              |

Frontend — одностраничное приложение на Vite: `src/` разделён на `pages / components / stores / hooks / lib / types / i18n`, а хранилища Zustand — на четыре слоя: `domain / feature / shared / devtools`.

Принадлежность к слоям, разрешённые комбинации зависимостей и регистрация ролей crate описаны в разделе о принципах архитектуры Harness для Rust-бэкенда в [AGENTS.md](./AGENTS.md).

---

## Проверочные шлюзы

```bash
npm run format:check                          # Проверка форматирования и переводов строк dprint
npm run lint:eslint                           # oxlint (ESLint не поддерживает TS 7)
npm run typecheck                             # tsc --noEmit
npm run test:run                              # Однократный запуск Vitest
npm run test:e2e                              # Playwright E2E
npm run test:rust                             # Rust-тесты (в Windows используется __TAURI_WORKSPACE__=true)
npm run ci:check                              # Локальное зеркало CI-шлюза
node scripts/cargo-all-workspaces.mjs fmt-check
node scripts/cargo-all-workspaces.mjs clippy
node scripts/cargo-all-workspaces.mjs check
node scripts/cargo-all-workspaces.mjs nextest
```

«Зелёные» `cargo check` / `cargo test` не означают прохождения шлюза Clippy: `fmt` должен запускаться с `--all`, `clippy` — с `--all-targets --all-features -- -D warnings`, и по каждому workspace отдельно. Перед коммитом запустите один раз по критериям CI. Чек-лист релиза — в [CHECKS.md](./CHECKS.md).

---

## Версия и журнал изменений

Текущая опубликованная версия — **v3.0.0** (2026-10-31). Единственный источник записей о версиях — [CHANGELOG.md](./CHANGELOG.md), включая неопубликованный раздел `[unreleased]`; `package.json`, `Cargo.toml` каждого workspace и «соответствующая версия» в шапке документации должны совпадать с ним.

---

## Лицензия и связь с upstream

- Лицензия: [AGPL-3.0-only](./LICENSE).
- Upstream: репозиторий создан на основе [`polite0803/AxAgent`](https://github.com/polite0803/AxAgent) (git remote `upstream`); возможности общей ИИ-основы наследуются от upstream.
- Адрес репозитория: [`polite0803/AxInvest`](https://github.com/polite0803/AxInvest).
