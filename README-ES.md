# AxInvest

**Estación de trabajo de investigación de acciones A con prioridad local**: aplicación de escritorio basada en Tauri 2 + Rust que reúne en un mismo espacio de trabajo la obtención de datos de mercado y fundamentales, las decisiones de acciones en cuatro horizontes, el screening y la revisión, las carteras simuladas, el backtesting cuantitativo, las tareas automatizadas y la colaboración con IA. Todos los datos de negocio se guardan por defecto en SQLite local, las claves API se cifran con AES-256-GCM y los flujos principales no dependen de servicios en la nube de terceros.

> **Versiones de idioma**: [English](./README-EN.md) | [繁體中文](./README-ZH-TW.md) | [日本語](./README-JA.md) | [한국어](./README-KO.md) | [Français](./README-FR.md) | [Deutsch](./README-DE.md) | [Español](./README-ES.md) | [Русский](./README-RU.md) | [हिन्दी](./README-HI.md) | [العربية](./README-AR.md)

---

## Posicionamiento del proyecto

AxInvest deriva de [`polite0803/AxAgent`](https://github.com/polite0803/AxAgent); la base general de IA se hereda del upstream y las capacidades de negocio de inversión evolucionan en este repositorio. Está orientado a «una cadena de investigación de inversión que se puede reproducir», no a un conjunto de funcionalidades:

- **Una misma concepción del tiempo**: `live` y `as-of` atraviesan todas las capacidades de inversión; los lotes históricos se pueden reproducir tal cual y las muestras se pueden calibrar.
- **Una misma cadena de decisión**: desde la construcción del grupo de candidatos y la decisión independiente en cuatro horizontes, hasta el almacenamiento, la atribución y la evolución por reflexión, con un único criterio.
- **Prioridad local**: por defecto se persiste en SQLite, sin dependencia de la nube; los datos y las claves permanecen en la máquina local.

---

## Navegación funcional (16 funcionalidades)

El punto de entrada de `docs/` es el índice de doble vista [docs/INDEX.md](./docs/INDEX.md): la vista funcional se organiza por 16 funcionalidades, la vista de archivo se clasifica por propuestas / planes / auditorías / referencias, y actualmente hay 265 documentos registrados. Cada funcionalidad cuenta con una guía que describe el comportamiento actual, los puntos de entrada del código, los interruptores de configuración, las limitaciones conocidas y el registro de cambios de versión.

**Dominio de negocio (9)**

| Funcionalidad                        | slug                   | Guía                                                               |
| ------------------------------------ | ---------------------- | ------------------------------------------------------------------ |
| Cadena de decisión de acciones       | `stock-decision-chain` | [stock-decision-chain.md](./docs/features/stock-decision-chain.md) |
| Selección inteligente de tendencias  | `serenity-screening`   | [serenity-screening.md](./docs/features/serenity-screening.md)     |
| Reproducción de viaje en el tiempo   | `time-travel-replay`   | [time-travel-replay.md](./docs/features/time-travel-replay.md)     |
| Datos de mercado y fuentes de datos  | `market-data`          | [market-data.md](./docs/features/market-data.md)                   |
| Paquetes de capacidades OPC          | `opc-capability-packs` | [opc-capability-packs.md](./docs/features/opc-capability-packs.md) |
| Backtesting cuantitativo             | `quant-backtest`       | [quant-backtest.md](./docs/features/quant-backtest.md)             |
| Reflexión y evolución                | `reflection-evolution` | [reflection-evolution.md](./docs/features/reflection-evolution.md) |
| Centro de tareas y programación      | `task-center`          | [task-center.md](./docs/features/task-center.md)                   |
| Cartera simulada y puente de trading | `paper-portfolio`      | [paper-portfolio.md](./docs/features/paper-portfolio.md)           |

**Dominio de plataforma (7)**

| Funcionalidad                                 | slug                 | Guía                                                           |
| --------------------------------------------- | -------------------- | -------------------------------------------------------------- |
| Sistema de plugins                            | `plugin-system`      | [plugin-system.md](./docs/features/plugin-system.md)           |
| Calidad de datos y controles                  | `data-quality-gates` | [data-quality-gates.md](./docs/features/data-quality-gates.md) |
| Runtime y agentes                             | `agent-runtime`      | [agent-runtime.md](./docs/features/agent-runtime.md)           |
| Proveedores de modelos y streaming            | `provider-models`    | [provider-models.md](./docs/features/provider-models.md)       |
| Almacenamiento y migraciones de base de datos | `storage-migration`  | [storage-migration.md](./docs/features/storage-migration.md)   |
| Shell del frontend                            | `ui-shell`           | [ui-shell.md](./docs/features/ui-shell.md)                     |
| Compilación y CI                              | `build-ci`           | [build-ci.md](./docs/features/build-ci.md)                     |

---

## Inicio rápido

Requisitos de entorno: Node.js ≥ 24, Rust ≥ 1.85 (edition 2024), npm ≥ 10. En Windows se requiere MSVC Build Tools, en macOS Xcode Command Line Tools y en Linux `build-essential`, `libwebkit2gtk-4.1-dev`, `libssl-dev`.

```bash
npm install
npm run dev           # Modo navegador (el frontend usa localStorage mock, sin Tauri)
npm run tauri dev     # Aplicación de escritorio (Vite HMR + ventana Tauri)
```

Construcción y artefactos:

```bash
npm run build         # Comprobación de tipos + compilación de producción del frontend
npm run tauri:build   # Instalador de escritorio
```

---

## Capas de arquitectura (8 raíces de workspace)

El lado Rust se divide según la dirección de dependencias en **8 raíces de Cargo workspace**, con un total de **42 crates de biblioteca**; `src-tauri` actúa como raíz de composición (crate del binario principal + `axagent-mobile`). Regla de hierro de la dirección de dependencias: `componentes → harness ← implementaciones`.

| Raíz de workspace | Rol                        | Nº de crates | Contenido principal                                                                                  |
| ----------------- | -------------------------- | ------------ | ---------------------------------------------------------------------------------------------------- |
| `axagent-core`    | foundation                 | 7            | traits y DTO de `harness`, `entities`, `plugin-proto`, `disk-cache`, `rt-theme`                      |
| `axagent-kit`     | implementor                | 1            | Capa de implementación común `kit`                                                                   |
| `axagent-ai`      | consumer + implementor     | 6            | `agent`, `orchestrator`, `providers`, `mcp`, `prompt-guard`, `trajectory`                            |
| `axagent-infra`   | implementor                | 12           | `dao`, `storage`, `migration`, `crypto`, `search`, `telemetry`, `document-parser`, etc.              |
| `axagent-quant`   | implementor / consumer     | 5            | `astock-data`, `analysis-engine`, `company-runtime`, `quant`, `market-sim`                           |
| `axagent-plugins` | implementor + foundation   | 2            | `plugins`, `schema-gen`                                                                              |
| `axagent-runtime` | wiring + consumer + hybrid | 8            | `runtime`, `runtime-core`, `tools`, `rt-workflow`, `rt-messaging`, `gateway`, `crdt`, `notification` |
| `src-tauri`       | wiring                     | 2            | Comandos Tauri, inicialización y ensamblado, `axagent-mobile`                                        |

El frontend es una aplicación de una sola página con Vite: `src/` se divide en `pages / components / stores / hooks / lib / types / i18n`, y los stores de Zustand se dividen a su vez en cuatro capas: `domain / feature / shared / devtools`.

La asignación de capas, las combinaciones de dependencias permitidas y el registro de roles de cada crate se encuentran en los principios de arquitectura Harness del backend Rust de [AGENTS.md](./AGENTS.md).

---

## Controles de verificación

```bash
npm run format:check                          # Comprobación de formato y fin de línea de dprint
npm run lint:eslint                           # oxlint (ESLint no admite TS 7)
npm run typecheck                             # tsc --noEmit
npm run test:run                              # Ejecución única de Vitest
npm run test:e2e                              # Playwright E2E
npm run test:rust                             # Pruebas Rust (en Windows se usa __TAURI_WORKSPACE__=true)
npm run ci:check                              # Control espejo local de CI
node scripts/cargo-all-workspaces.mjs fmt-check
node scripts/cargo-all-workspaces.mjs clippy
node scripts/cargo-all-workspaces.mjs check
node scripts/cargo-all-workspaces.mjs nextest
```

Un `cargo check` / `cargo test` en verde no equivale a superar el control de Clippy: `fmt` debe llevarse con `--all`, `clippy` con `--all-targets --all-features -- -D warnings`, y ejecutarse workspace por workspace. Antes de enviar, ejecútalo una vez según el criterio de CI. La lista de comprobación de publicación está en [CHECKS.md](./CHECKS.md).

---

## Versión y registro de cambios

La versión publicada actual es **v3.0.0** (2026-10-31). La única fuente de las entradas de versión es [CHANGELOG.md](./CHANGELOG.md), incluida la sección `[unreleased]` sin publicar; `package.json`, los `Cargo.toml` de cada workspace y la «versión correspondiente» en la cabecera de la documentación deben coincidir con ella.

---

## Licencia y relación con upstream

- Licencia: [AGPL-3.0-only](./LICENSE).
- Upstream: este repositorio deriva de [`polite0803/AxAgent`](https://github.com/polite0803/AxAgent) (git remote `upstream`); las capacidades de base general de IA se heredan del upstream.
- Dirección de este repositorio: [`polite0803/AxInvest`](https://github.com/polite0803/AxInvest).
