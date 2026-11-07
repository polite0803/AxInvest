# AxInvest

**ローカルファーストの A 株投資リサーチ・ワークステーション**：Tauri 2 + Rust によるデスクトップアプリで、相場・ファンダメンタルデータの取得、4 周期の個別株判断、銘柄選定と振り返り、模擬ポートフォリオ、クオンツ・バックテスト、自動化タスク、AI コラボレーションを 1 つのワークスペースに統合している。すべての業務データは既定でローカル SQLite に保存され、API Key は AES-256-GCM で暗号化され、主要フローはサードパーティのクラウドサービスに依存しない。

> **言語版**：[English](./README-EN.md) | [繁體中文](./README-ZH-TW.md) | [日本語](./README-JA.md) | [한국어](./README-KO.md) | [Français](./README-FR.md) | [Deutsch](./README-DE.md) | [Español](./README-ES.md) | [Русский](./README-RU.md) | [हिन्दी](./README-HI.md) | [العربية](./README-AR.md)

---

## プロジェクトの位置づけ

AxInvest は [`polite0803/AxAgent`](https://github.com/polite0803/AxAgent) から派生しており、汎用 AI 基盤は上流から継承し、投資業務の能力は本リポジトリで発展させる。目指すのは「振り返り可能な投資リサーチの一連の流れ」であり、機能の集合体ではない：

- **同一の時間世界観**：`live` と `as-of` がすべての投資能力を貫き、過去のバッチをそのまま再生でき、サンプルを校正できる。
- **同一の意思決定チェーン**：候補プールの構築から 4 周期それぞれの独立判断、データベースへの記録、要因分析と反省・進化まで、基準はただ 1 つ。
- **ローカルファースト**：既定で SQLite に保存し、クラウド依存はなく、データと鍵は手元に残る。

---

## 機能ナビゲーション（16 の機能ポイント）

`docs/` の入口はデュアルビューの索引 [docs/INDEX.md](./docs/INDEX.md)：機能ビューは 16 の機能ポイントで構成され、アーカイブビューは案 / 計画 / 監査 / 参考で分類され、現在 265 件のドキュメントを登録している。各機能ポイントには 1 件の説明書があり、現在の挙動、コード入口、設定スイッチ、既知の制限、バージョン変更履歴を明記している。

**業務ドメイン（9 個）**

| 機能ポイント                           | slug                   | 説明書                                                             |
| -------------------------------------- | ---------------------- | ------------------------------------------------------------------ |
| 株式意思決定チェーン                   | `stock-decision-chain` | [stock-decision-chain.md](./docs/features/stock-decision-chain.md) |
| トレンド・スマート選別                 | `serenity-screening`   | [serenity-screening.md](./docs/features/serenity-screening.md)     |
| タイムトラベル・リプレイ               | `time-travel-replay`   | [time-travel-replay.md](./docs/features/time-travel-replay.md)     |
| 相場とデータソース                     | `market-data`          | [market-data.md](./docs/features/market-data.md)                   |
| OPC ケイパビリティパック               | `opc-capability-packs` | [opc-capability-packs.md](./docs/features/opc-capability-packs.md) |
| クオンツ・バックテスト                 | `quant-backtest`       | [quant-backtest.md](./docs/features/quant-backtest.md)             |
| 反省と進化                             | `reflection-evolution` | [reflection-evolution.md](./docs/features/reflection-evolution.md) |
| タスクセンターとスケジューリング       | `task-center`          | [task-center.md](./docs/features/task-center.md)                   |
| 模擬ポジションとトレーディングブリッジ | `paper-portfolio`      | [paper-portfolio.md](./docs/features/paper-portfolio.md)           |

**プラットフォームドメイン（7 個）**

| 機能ポイント                             | slug                 | 説明書                                                         |
| ---------------------------------------- | -------------------- | -------------------------------------------------------------- |
| プラグイン体系                           | `plugin-system`      | [plugin-system.md](./docs/features/plugin-system.md)           |
| データ品質とゲート                       | `data-quality-gates` | [data-quality-gates.md](./docs/features/data-quality-gates.md) |
| ランタイムとエージェント                 | `agent-runtime`      | [agent-runtime.md](./docs/features/agent-runtime.md)           |
| モデルプロバイダーとストリーミング       | `provider-models`    | [provider-models.md](./docs/features/provider-models.md)       |
| ストレージとデータベースマイグレーション | `storage-migration`  | [storage-migration.md](./docs/features/storage-migration.md)   |
| フロントエンドシェル                     | `ui-shell`           | [ui-shell.md](./docs/features/ui-shell.md)                     |
| ビルドと CI                              | `build-ci`           | [build-ci.md](./docs/features/build-ci.md)                     |

---

## クイックスタート

環境要件：Node.js ≥ 24、Rust ≥ 1.85（edition 2024）、npm ≥ 10。Windows は MSVC Build Tools、macOS は Xcode Command Line Tools、Linux は `build-essential`、`libwebkit2gtk-4.1-dev`、`libssl-dev` が必要。

```bash
npm install
npm run dev           # ブラウザモード（フロントエンドは localStorage mock、Tauri 不要）
npm run tauri dev     # デスクトップアプリ（Vite HMR + Tauri ウィンドウ）
```

ビルドと成果物：

```bash
npm run build         # 型チェック + フロントエンド本番ビルド
npm run tauri:build   # デスクトップインストーラー
```

---

## アーキテクチャ階層（8 つの workspace ルート）

Rust 側は依存方向に従って **8 つの Cargo workspace ルート**に分割され、合計 **42 個のライブラリ crate**、`src-tauri` が構成ルート（メインのバイナリ crate + `axagent-mobile`）。依存方向の鉄則：`コンポーネント → harness ← 実装`。

| workspace ルート  | 役割                       | crate 数 | 主な内容                                                                                             |
| ----------------- | -------------------------- | -------- | ---------------------------------------------------------------------------------------------------- |
| `axagent-core`    | foundation                 | 7        | `harness` trait と DTO、`entities`、`plugin-proto`、`disk-cache`、`rt-theme`                         |
| `axagent-kit`     | implementor                | 1        | 共通実装層 `kit`                                                                                     |
| `axagent-ai`      | consumer + implementor     | 6        | `agent`、`orchestrator`、`providers`、`mcp`、`prompt-guard`、`trajectory`                            |
| `axagent-infra`   | implementor                | 12       | `dao`、`storage`、`migration`、`crypto`、`search`、`telemetry`、`document-parser` など               |
| `axagent-quant`   | implementor / consumer     | 5        | `astock-data`、`analysis-engine`、`company-runtime`、`quant`、`market-sim`                           |
| `axagent-plugins` | implementor + foundation   | 2        | `plugins`、`schema-gen`                                                                              |
| `axagent-runtime` | wiring + consumer + hybrid | 8        | `runtime`、`runtime-core`、`tools`、`rt-workflow`、`rt-messaging`、`gateway`、`crdt`、`notification` |
| `src-tauri`       | wiring                     | 2        | Tauri コマンド、初期化と組み立て、`axagent-mobile`                                                   |

フロントエンドは Vite のシングルページアプリ：`src/` 配下を `pages / components / stores / hooks / lib / types / i18n` で階層化し、Zustand store はさらに `domain / feature / shared / devtools` の 4 層に分かれる。

階層の帰属、許容される依存の組み合わせ、各 crate の役割登録は [AGENTS.md](./AGENTS.md) の「Rust バックエンド：Harness アーキテクチャ原則」を参照。

---

## 検証ゲート

```bash
npm run format:check                          # dprint のフォーマットと行末のチェック
npm run lint:eslint                           # oxlint（ESLint は TS 7 非対応）
npm run typecheck                             # tsc --noEmit
npm run test:run                              # Vitest を 1 回実行
npm run test:e2e                              # Playwright E2E
npm run test:rust                             # Rust テスト（Windows では __TAURI_WORKSPACE__=true）
npm run ci:check                              # ローカル CI ミラーゲート
node scripts/cargo-all-workspaces.mjs fmt-check
node scripts/cargo-all-workspaces.mjs clippy
node scripts/cargo-all-workspaces.mjs check
node scripts/cargo-all-workspaces.mjs nextest
```

`cargo check` / `cargo test` がすべてグリーンでも Clippy ゲートの通過を意味しない：`fmt` は `--all` を、`clippy` は `--all-targets --all-features -- -D warnings` を付けて、workspace ごとに実行すること。提出前に CI と同じ基準で 1 回実行する。リリース前の確認は [CHECKS.md](./CHECKS.md) を参照。

---

## バージョンと更新履歴

現在のリリースバージョンは **v3.0.0**（2026-10-31）。バージョン項目の唯一のソースは [CHANGELOG.md](./CHANGELOG.md) で、未リリースの `[unreleased]` セクションを含む。`package.json`、各 workspace の `Cargo.toml`、ドキュメント冒頭の「対応バージョン」はこれと一致させること。

---

## ライセンスと上流の関係

- ライセンス：[AGPL-3.0-only](./LICENSE)。
- 上流：本リポジトリは [`polite0803/AxAgent`](https://github.com/polite0803/AxAgent) から派生（git remote `upstream`）し、汎用 AI 基盤の能力は上流から継承する。
- 本リポジトリのアドレス：[`polite0803/AxInvest`](https://github.com/polite0803/AxInvest)。
