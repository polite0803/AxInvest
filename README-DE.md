# AxInvest

**Lokal-first-Arbeitsplatz für Investmentresearch zu A-Aktien**: eine Desktop-Anwendung auf Basis von Tauri 2 + Rust, die Kurs- und Fundamentaldatenbeschaffung, Einzelaktienentscheidungen über vier Horizonte, Aktienauswahl und Review, simuliertes Portfolio, quantitatives Backtesting, automatisierte Aufgaben und KI-Zusammenarbeit in einem Arbeitsbereich vereint. Alle Geschäftsdaten werden standardmäßig lokal in SQLite gespeichert, API-Keys sind mit AES-256-GCM verschlüsselt, und die Kernabläufe hängen von keinem Cloud-Drittanbieterdienst ab.

> **Sprachversionen**: [English](./README-EN.md) | [繁體中文](./README-ZH-TW.md) | [日本語](./README-JA.md) | [한국어](./README-KO.md) | [Français](./README-FR.md) | [Deutsch](./README-DE.md) | [Español](./README-ES.md) | [Русский](./README-RU.md) | [हिन्दी](./README-HI.md) | [العربية](./README-AR.md)

---

## Projektpositionierung

AxInvest leitet sich von [`polite0803/AxAgent`](https://github.com/polite0803/AxAgent) ab: Die allgemeine KI-Basis erbt vom Upstream, die Investment-Fachfähigkeiten entwickeln sich in diesem Repository weiter. Ziel ist „eine nachvollziehbare Investment-Research-Kette“, nicht eine Sammlung von Funktionen:

- **Eine gemeinsame Zeitauffassung**: `live` und `as-of` durchziehen alle Investment-Fähigkeiten, historische Batches lassen sich unverändert wiedergeben und Stichproben kalibrieren.
- **Eine gemeinsame Entscheidungskette**: von der Konstruktion des Kandidatenpools über unabhängige Entscheidungen auf vier Horizonten bis hin zu Persistenz, Attribution und evolutionärer Reflexion gibt es nur ein Regelwerk.
- **Lokal zuerst**: standardmäßige SQLite-Persistenz, keine Cloud-Abhängigkeit, Daten und Schlüssel bleiben auf dem Rechner.

---

## Funktionsnavigation (16 Funktionspunkte)

Der Einstieg in `docs/` ist der Zwei-Ansichten-Index [docs/INDEX.md](./docs/INDEX.md): Die Funktionsansicht ist nach 16 Funktionspunkten gegliedert, die Archivansicht nach Konzept / Plan / Audit / Referenz klassifiziert; derzeit sind 265 Dokumente registriert. Zu jedem Funktionspunkt gibt es ein Handbuch mit aktuellem Verhalten, Code-Einstiegspunkten, Konfigurationsschaltern, bekannten Einschränkungen und Versionshistorie.

**Fachdomänen (9)**

| Funktionspunkt                           | slug                   | Handbuch                                                           |
| ---------------------------------------- | ---------------------- | ------------------------------------------------------------------ |
| Aktienentscheidungskette                 | `stock-decision-chain` | [stock-decision-chain.md](./docs/features/stock-decision-chain.md) |
| Trendauswahl                             | `serenity-screening`   | [serenity-screening.md](./docs/features/serenity-screening.md)     |
| Zeitreise-Replay                         | `time-travel-replay`   | [time-travel-replay.md](./docs/features/time-travel-replay.md)     |
| Kursdaten und Datenquellen               | `market-data`          | [market-data.md](./docs/features/market-data.md)                   |
| OPC-Kompetenzpakete                      | `opc-capability-packs` | [opc-capability-packs.md](./docs/features/opc-capability-packs.md) |
| Quantitatives Backtesting                | `quant-backtest`       | [quant-backtest.md](./docs/features/quant-backtest.md)             |
| Reflexion und Evolution                  | `reflection-evolution` | [reflection-evolution.md](./docs/features/reflection-evolution.md) |
| Aufgabencenter und Scheduling            | `task-center`          | [task-center.md](./docs/features/task-center.md)                   |
| Simulierte Positionen und Trading-Bridge | `paper-portfolio`      | [paper-portfolio.md](./docs/features/paper-portfolio.md)           |

**Plattformdomäne (7)**

| Funktionspunkt                  | slug                 | Handbuch                                                       |
| ------------------------------- | -------------------- | -------------------------------------------------------------- |
| Plugin-System                   | `plugin-system`      | [plugin-system.md](./docs/features/plugin-system.md)           |
| Datenqualität und Gates         | `data-quality-gates` | [data-quality-gates.md](./docs/features/data-quality-gates.md) |
| Runtime und Agenten             | `agent-runtime`      | [agent-runtime.md](./docs/features/agent-runtime.md)           |
| Modellanbieter und Streaming    | `provider-models`    | [provider-models.md](./docs/features/provider-models.md)       |
| Speicher und Datenbankmigration | `storage-migration`  | [storage-migration.md](./docs/features/storage-migration.md)   |
| Frontend-Shell                  | `ui-shell`           | [ui-shell.md](./docs/features/ui-shell.md)                     |
| Build und CI                    | `build-ci`           | [build-ci.md](./docs/features/build-ci.md)                     |

---

## Schnellstart

Voraussetzungen: Node.js ≥ 24, Rust ≥ 1.85 (edition 2024), npm ≥ 10. Unter Windows sind MSVC Build Tools erforderlich, unter macOS Xcode Command Line Tools, unter Linux `build-essential`, `libwebkit2gtk-4.1-dev`, `libssl-dev`.

```bash
npm install
npm run dev           # Browser-Modus (Frontend mit localStorage-Mock, ohne Tauri)
npm run tauri dev     # Desktop-Anwendung (Vite HMR + Tauri-Fenster)
```

Build und Artefakte:

```bash
npm run build         # Typprüfung + Produktionsbuild des Frontends
npm run tauri:build   # Desktop-Installationspaket
```

---

## Architekturschichten (8 Workspace-Wurzeln)

Die Rust-Seite ist entlang der Abhängigkeitsrichtung in **8 Cargo-Workspace-Wurzeln** aufgeteilt, mit insgesamt **42 Library-Crates**; `src-tauri` dient als Kompositionswurzel (Haupt-Binary-Crate + `axagent-mobile`). Eiserne Regel der Abhängigkeiten: `Komponente → harness ← Implementierung`.

| Workspace-Wurzel  | Rolle                      | Anzahl Crates | Hauptinhalt                                                                                          |
| ----------------- | -------------------------- | ------------- | ---------------------------------------------------------------------------------------------------- |
| `axagent-core`    | foundation                 | 7             | `harness`-Traits und -DTOs, `entities`, `plugin-proto`, `disk-cache`, `rt-theme`                     |
| `axagent-kit`     | implementor                | 1             | Gemeinsame Implementierungsschicht `kit`                                                             |
| `axagent-ai`      | consumer + implementor     | 6             | `agent`, `orchestrator`, `providers`, `mcp`, `prompt-guard`, `trajectory`                            |
| `axagent-infra`   | implementor                | 12            | `dao`, `storage`, `migration`, `crypto`, `search`, `telemetry`, `document-parser` usw.               |
| `axagent-quant`   | implementor / consumer     | 5             | `astock-data`, `analysis-engine`, `company-runtime`, `quant`, `market-sim`                           |
| `axagent-plugins` | implementor + foundation   | 2             | `plugins`, `schema-gen`                                                                              |
| `axagent-runtime` | wiring + consumer + hybrid | 8             | `runtime`, `runtime-core`, `tools`, `rt-workflow`, `rt-messaging`, `gateway`, `crdt`, `notification` |
| `src-tauri`       | wiring                     | 2             | Tauri-Befehle, Initialisierung und Zusammenbau, `axagent-mobile`                                     |

Das Frontend ist eine Vite-Single-Page-Anwendung: Unter `src/` ist es nach `pages / components / stores / hooks / lib / types / i18n` geschichtet, die Zustand-Stores wiederum in vier Ebenen `domain / feature / shared / devtools`.

Schichtzugehörigkeit, erlaubte Abhängigkeitskombinationen und die Rollenregistrierung der einzelnen Crates sind im Abschnitt „Rust 后端：Harness 架构准则“ von [AGENTS.md](./AGENTS.md) beschrieben.

---

## Prüf-Gates

```bash
npm run format:check                          # dprint-Format- und Zeilenendenprüfung
npm run lint:eslint                           # oxlint (ESLint unterstützt TS 7 nicht)
npm run typecheck                             # tsc --noEmit
npm run test:run                              # Einmaliger Vitest-Lauf
npm run test:e2e                              # Playwright E2E
npm run test:rust                             # Rust-Tests (unter Windows mit __TAURI_WORKSPACE__=true)
npm run ci:check                              # Lokaler CI-Spiegel
node scripts/cargo-all-workspaces.mjs fmt-check
node scripts/cargo-all-workspaces.mjs clippy
node scripts/cargo-all-workspaces.mjs check
node scripts/cargo-all-workspaces.mjs nextest
```

Ein durchweg grünes `cargo check` / `cargo test` bedeutet nicht, dass das Clippy-Gate bestanden ist: `fmt` muss mit `--all` laufen, `clippy` mit `--all-targets --all-features -- -D warnings`, und zwar Workspace für Workspace. Vor dem Commit einmal gemäß CI-Referenz ausführen. Die Release-Prüfung steht in [CHECKS.md](./CHECKS.md).

---

## Version und Changelog

Die aktuell veröffentlichte Version ist **v3.0.0** (2026-10-31). Einzige Quelle für Versioneinträge ist [CHANGELOG.md](./CHANGELOG.md), einschließlich des noch unveröffentlichten Abschnitts `[unreleased]`; `package.json`, die `Cargo.toml` der einzelnen Workspaces und die „entsprechende Version“ in den Dokumentköpfen müssen dazu konsistent bleiben.

---

## Lizenz und Upstream-Beziehung

- Lizenz: [AGPL-3.0-only](./LICENSE).
- Upstream: Dieses Repository leitet sich von [`polite0803/AxAgent`](https://github.com/polite0803/AxAgent) ab (git remote `upstream`); die Fähigkeiten der allgemeinen KI-Basis erben vom Upstream.
- Adresse dieses Repositorys: [`polite0803/AxInvest`](https://github.com/polite0803/AxInvest).
