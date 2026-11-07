# AxInvest

**Poste de travail de recherche sur les actions A, avec priorité au local** : une application de bureau basée sur Tauri 2 + Rust qui réunit dans un même espace de travail la collecte des cours et des données fondamentales, la décision par action sur quatre horizons, la sélection d'actions et le retour d'expérience, le portefeuille simulé, le backtest quantitatif, les tâches automatisées et la collaboration avec l'IA. Toutes les données métier sont stockées par défaut dans SQLite en local, les clés API sont chiffrées en AES-256-GCM, et les parcours essentiels ne dépendent d'aucun service cloud tiers.

> **Versions linguistiques** : [English](./README-EN.md) | [繁體中文](./README-ZH-TW.md) | [日本語](./README-JA.md) | [한국어](./README-KO.md) | [Français](./README-FR.md) | [Deutsch](./README-DE.md) | [Español](./README-ES.md) | [Русский](./README-RU.md) | [हिन्दी](./README-HI.md) | [العربية](./README-AR.md)

---

## Positionnement du projet

AxInvest dérive de [`polite0803/AxAgent`](https://github.com/polite0803/AxAgent) : la base d'IA générique hérite de l'upstream, les capacités métier d'investissement évoluent dans ce dépôt. Il vise « une chaîne de recherche d'investissement rejouable », et non un ensemble de fonctionnalités :

- **Une même vision du temps** : `live` et `as-of` traversent toutes les capacités d'investissement, les lots historiques peuvent être rejoués tels quels et les échantillons recalibrés.
- **Une même chaîne de décision** : de la construction du vivier de candidats à la décision indépendante sur quatre horizons, puis à la persistance, à l'attribution et à l'évolution par la réflexion, il n'existe qu'un seul référentiel.
- **Priorité au local** : persistance SQLite par défaut, aucune dépendance cloud, les données et les clés restent sur la machine.

---

## Navigation des fonctionnalités (16 points fonctionnels)

L'entrée de `docs/` est l'index à double vue [docs/INDEX.md](./docs/INDEX.md) : la vue fonctionnelle est organisée par 16 points fonctionnels, la vue d'archivage est classée par proposition / plan / audit / référence, et 265 documents y sont actuellement enregistrés. Chaque point fonctionnel dispose d'un manuel précisant le comportement actuel, les points d'entrée du code, les commutateurs de configuration, les limitations connues et l'historique des changements de version.

**Domaines métier (9)**

| Point fonctionnel                         | slug                   | Documentation                                                      |
| ----------------------------------------- | ---------------------- | ------------------------------------------------------------------ |
| Chaîne de décision sur actions            | `stock-decision-chain` | [stock-decision-chain.md](./docs/features/stock-decision-chain.md) |
| Sélection par tendance                    | `serenity-screening`   | [serenity-screening.md](./docs/features/serenity-screening.md)     |
| Rejeu temporel                            | `time-travel-replay`   | [time-travel-replay.md](./docs/features/time-travel-replay.md)     |
| Cours et sources de données               | `market-data`          | [market-data.md](./docs/features/market-data.md)                   |
| Packs de capacités OPC                    | `opc-capability-packs` | [opc-capability-packs.md](./docs/features/opc-capability-packs.md) |
| Backtest quantitatif                      | `quant-backtest`       | [quant-backtest.md](./docs/features/quant-backtest.md)             |
| Réflexion et évolution                    | `reflection-evolution` | [reflection-evolution.md](./docs/features/reflection-evolution.md) |
| Centre de tâches et ordonnancement        | `task-center`          | [task-center.md](./docs/features/task-center.md)                   |
| Positions simulées et pont de négociation | `paper-portfolio`      | [paper-portfolio.md](./docs/features/paper-portfolio.md)           |

**Domaine plateforme (7)**

| Point fonctionnel                         | slug                 | Documentation                                                  |
| ----------------------------------------- | -------------------- | -------------------------------------------------------------- |
| Système de plugins                        | `plugin-system`      | [plugin-system.md](./docs/features/plugin-system.md)           |
| Qualité des données et portes de contrôle | `data-quality-gates` | [data-quality-gates.md](./docs/features/data-quality-gates.md) |
| Runtime et agents                         | `agent-runtime`      | [agent-runtime.md](./docs/features/agent-runtime.md)           |
| Fournisseurs de modèles et streaming      | `provider-models`    | [provider-models.md](./docs/features/provider-models.md)       |
| Stockage et migration de base de données  | `storage-migration`  | [storage-migration.md](./docs/features/storage-migration.md)   |
| Coque frontend                            | `ui-shell`           | [ui-shell.md](./docs/features/ui-shell.md)                     |
| Build et CI                               | `build-ci`           | [build-ci.md](./docs/features/build-ci.md)                     |

---

## Démarrage rapide

Prérequis : Node.js ≥ 24, Rust ≥ 1.85 (edition 2024), npm ≥ 10. Sous Windows, MSVC Build Tools sont nécessaires ; sous macOS, Xcode Command Line Tools ; sous Linux, `build-essential`, `libwebkit2gtk-4.1-dev` et `libssl-dev`.

```bash
npm install
npm run dev           # Mode navigateur (frontend sur mock localStorage, sans Tauri)
npm run tauri dev     # Application de bureau (HMR Vite + fenêtre Tauri)
```

Build et artefacts :

```bash
npm run build         # Vérification des types + build de production du frontend
npm run tauri:build   # Paquet d'installation de bureau
```

---

## Couches d'architecture (8 racines de workspace)

Le côté Rust est découpé, selon la direction des dépendances, en **8 racines de workspace Cargo** totalisant **42 crates de bibliothèque**, `src-tauri` servant de racine de composition (crate binaire principal + `axagent-mobile`). Règle d'or des dépendances : `composant → harness ← implémentation`.

| Racine de workspace | Rôle                       | Nombre de crates | Contenu principal                                                                                    |
| ------------------- | -------------------------- | ---------------- | ---------------------------------------------------------------------------------------------------- |
| `axagent-core`      | foundation                 | 7                | Traits et DTO `harness`, `entities`, `plugin-proto`, `disk-cache`, `rt-theme`                        |
| `axagent-kit`       | implementor                | 1                | Couche d'implémentation commune `kit`                                                                |
| `axagent-ai`        | consumer + implementor     | 6                | `agent`, `orchestrator`, `providers`, `mcp`, `prompt-guard`, `trajectory`                            |
| `axagent-infra`     | implementor                | 12               | `dao`, `storage`, `migration`, `crypto`, `search`, `telemetry`, `document-parser` etc.               |
| `axagent-quant`     | implementor / consumer     | 5                | `astock-data`, `analysis-engine`, `company-runtime`, `quant`, `market-sim`                           |
| `axagent-plugins`   | implementor + foundation   | 2                | `plugins`, `schema-gen`                                                                              |
| `axagent-runtime`   | wiring + consumer + hybrid | 8                | `runtime`, `runtime-core`, `tools`, `rt-workflow`, `rt-messaging`, `gateway`, `crdt`, `notification` |
| `src-tauri`         | wiring                     | 2                | Commandes Tauri, initialisation et assemblage, `axagent-mobile`                                      |

Le frontend est une application monopage Vite : sous `src/`, il est découpé en `pages / components / stores / hooks / lib / types / i18n`, et les stores Zustand se subdivisent en quatre couches `domain / feature / shared / devtools`.

L'appartenance aux couches, les combinaisons de dépendances autorisées et l'enregistrement du rôle de chaque crate sont décrits dans la section « Rust 后端：Harness 架构准则 » de [AGENTS.md](./AGENTS.md).

---

## Portes de vérification

```bash
npm run format:check                          # Vérification du format dprint et des fins de ligne
npm run lint:eslint                           # oxlint (ESLint ne prend pas en charge TS 7)
npm run typecheck                             # tsc --noEmit
npm run test:run                              # Exécution unique de Vitest
npm run test:e2e                              # Playwright E2E
npm run test:rust                             # Tests Rust (sous Windows via __TAURI_WORKSPACE__=true)
npm run ci:check                              # Miroir local des portes CI
node scripts/cargo-all-workspaces.mjs fmt-check
node scripts/cargo-all-workspaces.mjs clippy
node scripts/cargo-all-workspaces.mjs check
node scripts/cargo-all-workspaces.mjs nextest
```

Un `cargo check` / `cargo test` entièrement vert ne vaut pas franchissement de la porte Clippy : `fmt` doit être lancé avec `--all`, `clippy` avec `--all-targets --all-features -- -D warnings`, et ce workspace par workspace. Avant chaque commit, effectuez une exécution conforme au référentiel CI. La vérification de release est décrite dans [CHECKS.md](./CHECKS.md).

---

## Version et journal des modifications

La version publiée actuelle est **v3.0.0** (2026-10-31). L'unique source des entrées de version est [CHANGELOG.md](./CHANGELOG.md), y compris la section `[unreleased]` non encore publiée ; `package.json`, le `Cargo.toml` de chaque workspace et la « version correspondante » en tête des documents doivent rester alignés sur celui-ci.

---

## Licence et relation avec l'upstream

- Licence : [AGPL-3.0-only](./LICENSE).
- Upstream : ce dépôt dérive de [`polite0803/AxAgent`](https://github.com/polite0803/AxAgent) (git remote `upstream`) ; les capacités de la base d'IA générique sont héritées de l'upstream.
- Adresse de ce dépôt : [`polite0803/AxInvest`](https://github.com/polite0803/AxInvest).
