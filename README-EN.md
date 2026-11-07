# AxInvest

**A local-first A-share investment research workspace**: a desktop application built on Tauri 2 + Rust that brings market and fundamental data retrieval, four-horizon per-stock decisions, screening and review, paper portfolios, quantitative backtesting, automated tasks, and AI collaboration into a single workspace. All business data is stored in local SQLite by default, API keys are encrypted with AES-256-GCM, and core workflows do not depend on third-party cloud services.

> **Languages**: [English](./README-EN.md) | [繁體中文](./README-ZH-TW.md) | [日本語](./README-JA.md) | [한국어](./README-KO.md) | [Français](./README-FR.md) | [Deutsch](./README-DE.md) | [Español](./README-ES.md) | [Русский](./README-RU.md) | [हिन्दी](./README-HI.md) | [العربية](./README-AR.md)

---

## Project Positioning

AxInvest is derived from [`polite0803/AxAgent`](https://github.com/polite0803/AxAgent): the general-purpose AI foundation is inherited from upstream, while the investment business capabilities evolve in this repository. It targets "one reviewable investment research pipeline", not a collection of features:

- **One shared notion of time**: `live` and `as-of` run through every investment capability, so historical batches can be replayed as-is and samples can be calibrated.
- **One shared decision chain**: from candidate pool construction and independent four-horizon decisions to persistence, attribution, and reflective evolution, there is only one set of conventions.
- **Local first**: SQLite by default, no cloud dependency, with data and keys kept on the local machine.

---

## Feature Map (16 Feature Areas)

The entry point of `docs/` is the dual-view index [docs/INDEX.md](./docs/INDEX.md): the feature view is organized by 16 feature areas, and the archive view is classified by proposal / plan / audit / reference, with 265 documents registered in total. Each feature area has one handbook that documents current behavior, code entry points, configuration switches, known limitations, and version change records.

**Business domain (9)**

| Feature area                       | slug                   | Handbook                                                           |
| ---------------------------------- | ---------------------- | ------------------------------------------------------------------ |
| Stock decision chain               | `stock-decision-chain` | [stock-decision-chain.md](./docs/features/stock-decision-chain.md) |
| Serenity screening                 | `serenity-screening`   | [serenity-screening.md](./docs/features/serenity-screening.md)     |
| Time-travel replay                 | `time-travel-replay`   | [time-travel-replay.md](./docs/features/time-travel-replay.md)     |
| Market data and data sources       | `market-data`          | [market-data.md](./docs/features/market-data.md)                   |
| OPC capability packs               | `opc-capability-packs` | [opc-capability-packs.md](./docs/features/opc-capability-packs.md) |
| Quantitative backtesting           | `quant-backtest`       | [quant-backtest.md](./docs/features/quant-backtest.md)             |
| Reflection and evolution           | `reflection-evolution` | [reflection-evolution.md](./docs/features/reflection-evolution.md) |
| Task center and scheduling         | `task-center`          | [task-center.md](./docs/features/task-center.md)                   |
| Paper positions and trading bridge | `paper-portfolio`      | [paper-portfolio.md](./docs/features/paper-portfolio.md)           |

**Platform domain (7)**

| Feature area                   | slug                 | Handbook                                                       |
| ------------------------------ | -------------------- | -------------------------------------------------------------- |
| Plugin system                  | `plugin-system`      | [plugin-system.md](./docs/features/plugin-system.md)           |
| Data quality and gates         | `data-quality-gates` | [data-quality-gates.md](./docs/features/data-quality-gates.md) |
| Runtime and agents             | `agent-runtime`      | [agent-runtime.md](./docs/features/agent-runtime.md)           |
| Model providers and streaming  | `provider-models`    | [provider-models.md](./docs/features/provider-models.md)       |
| Storage and database migration | `storage-migration`  | [storage-migration.md](./docs/features/storage-migration.md)   |
| Frontend shell                 | `ui-shell`           | [ui-shell.md](./docs/features/ui-shell.md)                     |
| Build and CI                   | `build-ci`           | [build-ci.md](./docs/features/build-ci.md)                     |

---

## Quick Start

Requirements: Node.js ≥ 24, Rust ≥ 1.85 (edition 2024), npm ≥ 10. Windows requires MSVC Build Tools, macOS requires Xcode Command Line Tools, and Linux requires `build-essential`, `libwebkit2gtk-4.1-dev`, and `libssl-dev`.

```bash
npm install
npm run dev           # Browser mode (frontend uses the localStorage mock, no Tauri required)
npm run tauri dev     # Desktop app (Vite HMR + Tauri window)
```

Build and artifacts:

```bash
npm run build         # Type checking + frontend production build
npm run tauri:build   # Desktop installer package
```

---

## Architecture Layers (8 Workspace Roots)

On the Rust side, the code is split by dependency direction into **8 Cargo workspace roots**, with **42 library crates** in total; `src-tauri` acts as the composition root (main binary crate + `axagent-mobile`). The iron rule for dependency direction: `component → harness ← implementation`.

| Workspace root    | Role                       | crates | Main contents                                                                                        |
| ----------------- | -------------------------- | ------ | ---------------------------------------------------------------------------------------------------- |
| `axagent-core`    | foundation                 | 7      | `harness` traits and DTOs, `entities`, `plugin-proto`, `disk-cache`, `rt-theme`                      |
| `axagent-kit`     | implementor                | 1      | Shared implementation layer `kit`                                                                    |
| `axagent-ai`      | consumer + implementor     | 6      | `agent`, `orchestrator`, `providers`, `mcp`, `prompt-guard`, `trajectory`                            |
| `axagent-infra`   | implementor                | 12     | `dao`, `storage`, `migration`, `crypto`, `search`, `telemetry`, `document-parser`, etc.              |
| `axagent-quant`   | implementor / consumer     | 5      | `astock-data`, `analysis-engine`, `company-runtime`, `quant`, `market-sim`                           |
| `axagent-plugins` | implementor + foundation   | 2      | `plugins`, `schema-gen`                                                                              |
| `axagent-runtime` | wiring + consumer + hybrid | 8      | `runtime`, `runtime-core`, `tools`, `rt-workflow`, `rt-messaging`, `gateway`, `crdt`, `notification` |
| `src-tauri`       | wiring                     | 2      | Tauri commands, initialization and wiring, `axagent-mobile`                                          |

The frontend is a Vite single-page application: `src/` is layered into `pages / components / stores / hooks / lib / types / i18n`, and Zustand stores are further divided into four layers: `domain / feature / shared / devtools`.

For layer ownership, allowed dependency combinations, and the role registration of each crate, see "Rust backend: Harness architecture guidelines" in [AGENTS.md](./AGENTS.md).

---

## Verification Gates

```bash
npm run format:check                          # dprint formatting and line-ending check
npm run lint:eslint                           # oxlint (ESLint does not support TS 7)
npm run typecheck                             # tsc --noEmit
npm run test:run                              # Single Vitest run
npm run test:e2e                              # Playwright E2E
npm run test:rust                             # Rust tests (on Windows use __TAURI_WORKSPACE__=true)
npm run ci:check                              # Local CI mirror gates
node scripts/cargo-all-workspaces.mjs fmt-check
node scripts/cargo-all-workspaces.mjs clippy
node scripts/cargo-all-workspaces.mjs check
node scripts/cargo-all-workspaces.mjs nextest
```

`cargo check` / `cargo test` being all green does not mean the Clippy gate is passed: `fmt` must be run with `--all`, `clippy` must be run with `--all-targets --all-features -- -D warnings`, and both must be executed per workspace. Run them once with the CI criteria before committing. For release checks see [CHECKS.md](./CHECKS.md).

---

## Version and Changelog

The current release is **v3.0.0** (2026-10-31). [CHANGELOG.md](./CHANGELOG.md) is the single source of truth for version entries, including the unreleased `[unreleased]` section; `package.json`, the `Cargo.toml` of each workspace, and the "corresponding version" header in the docs must stay consistent with it.

---

## License and Upstream Relationship

- License: [AGPL-3.0-only](./LICENSE).
- Upstream: this repository is derived from [`polite0803/AxAgent`](https://github.com/polite0803/AxAgent) (git remote `upstream`); the general-purpose AI foundation capabilities are inherited from upstream.
- This repository: [`polite0803/AxInvest`](https://github.com/polite0803/AxInvest).
