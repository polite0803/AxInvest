# AxInvest

**本地優先的 A 股投資研究工作台**：以 Tauri 2 + Rust 打造的桌面應用程式，把行情與基本面取數、四週期個股決策、選股與復盤、模擬投資組合、量化回測、自動化任務與 AI 協作放進同一個工作區。全部業務資料預設儲存在本地 SQLite，API Key 經 AES-256-GCM 加密，核心流程不依賴第三方雲端服務。

> **語言版本**：[English](./README-EN.md) | [繁體中文](./README-ZH-TW.md) | [日本語](./README-JA.md) | [한국어](./README-KO.md) | [Français](./README-FR.md) | [Deutsch](./README-DE.md) | [Español](./README-ES.md) | [Русский](./README-RU.md) | [हिन्दी](./README-HI.md) | [العربية](./README-AR.md)

---

## 專案定位

AxInvest 從 [`polite0803/AxAgent`](https://github.com/polite0803/AxAgent) 派生，通用 AI 底座繼承上游，投資業務能力在本倉庫演進。它面向的是「一條可復盤的投研流程」，而不是功能集合：

- **同一套時間世界觀**：`live` 與 `as-of` 貫穿全部投資能力，歷史批次可原樣回放，樣本可校準。
- **同一條決策鏈**：從候選池建構、四週期獨立決策，到落庫、歸因與反思進化，口徑只有一套。
- **本地優先**：預設 SQLite 落地，無雲端依賴，資料與金鑰留在本機。

---

## 功能導覽（16 個功能點）

`docs/` 的入口是雙視圖索引 [docs/INDEX.md](./docs/INDEX.md)：功能視圖按 16 個功能點組織，歸檔視圖按方案 / 計畫 / 稽核 / 參考分類，目前共登記 265 篇文件。每個功能點都有一篇說明書，寫清目前行為、程式碼入口、設定開關、已知限制與版本變更記錄。

**業務領域（9 個）**

| 功能點           | slug                   | 說明書                                                             |
| ---------------- | ---------------------- | ------------------------------------------------------------------ |
| 股票決策鏈       | `stock-decision-chain` | [stock-decision-chain.md](./docs/features/stock-decision-chain.md) |
| 趨勢智選         | `serenity-screening`   | [serenity-screening.md](./docs/features/serenity-screening.md)     |
| 時間旅行回放     | `time-travel-replay`   | [time-travel-replay.md](./docs/features/time-travel-replay.md)     |
| 行情與資料源     | `market-data`          | [market-data.md](./docs/features/market-data.md)                   |
| OPC 能力包       | `opc-capability-packs` | [opc-capability-packs.md](./docs/features/opc-capability-packs.md) |
| 量化回測         | `quant-backtest`       | [quant-backtest.md](./docs/features/quant-backtest.md)             |
| 反思與進化       | `reflection-evolution` | [reflection-evolution.md](./docs/features/reflection-evolution.md) |
| 任務中心與排程   | `task-center`          | [task-center.md](./docs/features/task-center.md)                   |
| 模擬持倉與交易橋 | `paper-portfolio`      | [paper-portfolio.md](./docs/features/paper-portfolio.md)           |

**平台領域（7 個）**

| 功能點             | slug                 | 說明書                                                         |
| ------------------ | -------------------- | -------------------------------------------------------------- |
| 外掛體系           | `plugin-system`      | [plugin-system.md](./docs/features/plugin-system.md)           |
| 資料品質與門禁     | `data-quality-gates` | [data-quality-gates.md](./docs/features/data-quality-gates.md) |
| 執行時期與智慧代理 | `agent-runtime`      | [agent-runtime.md](./docs/features/agent-runtime.md)           |
| 模型供應商與串流   | `provider-models`    | [provider-models.md](./docs/features/provider-models.md)       |
| 儲存與資料庫遷移   | `storage-migration`  | [storage-migration.md](./docs/features/storage-migration.md)   |
| 前端外殼           | `ui-shell`           | [ui-shell.md](./docs/features/ui-shell.md)                     |
| 建置與 CI          | `build-ci`           | [build-ci.md](./docs/features/build-ci.md)                     |

---

## 快速開始

環境需求：Node.js ≥ 24、Rust ≥ 1.85（edition 2024）、npm ≥ 10。Windows 需 MSVC Build Tools，macOS 需 Xcode Command Line Tools，Linux 需 `build-essential`、`libwebkit2gtk-4.1-dev`、`libssl-dev`。

```bash
npm install
npm run dev           # 瀏覽器模式（前端走 localStorage mock，無需 Tauri）
npm run tauri dev     # 桌面應用程式（Vite HMR + Tauri 視窗）
```

建置與產物：

```bash
npm run build         # 型別檢查 + 前端生產建置
npm run tauri:build   # 桌面端安裝包
```

---

## 架構分層（8 個 workspace 根）

Rust 側按依賴方向拆分為 **8 個 Cargo workspace 根**，共 **42 個函式庫 crate**，`src-tauri` 作為組合根（主二進位 crate + `axagent-mobile`）。依賴方向鐵律：`元件 → harness ← 實作`。

| workspace 根      | 角色                       | crate 數 | 主要內容                                                                                             |
| ----------------- | -------------------------- | -------- | ---------------------------------------------------------------------------------------------------- |
| `axagent-core`    | foundation                 | 7        | `harness` trait 與 DTO、`entities`、`plugin-proto`、`disk-cache`、`rt-theme`                         |
| `axagent-kit`     | implementor                | 1        | 公共實作層 `kit`                                                                                     |
| `axagent-ai`      | consumer + implementor     | 6        | `agent`、`orchestrator`、`providers`、`mcp`、`prompt-guard`、`trajectory`                            |
| `axagent-infra`   | implementor                | 12       | `dao`、`storage`、`migration`、`crypto`、`search`、`telemetry`、`document-parser` 等                 |
| `axagent-quant`   | implementor / consumer     | 5        | `astock-data`、`analysis-engine`、`company-runtime`、`quant`、`market-sim`                           |
| `axagent-plugins` | implementor + foundation   | 2        | `plugins`、`schema-gen`                                                                              |
| `axagent-runtime` | wiring + consumer + hybrid | 8        | `runtime`、`runtime-core`、`tools`、`rt-workflow`、`rt-messaging`、`gateway`、`crdt`、`notification` |
| `src-tauri`       | wiring                     | 2        | Tauri 命令、初始化與裝配、`axagent-mobile`                                                           |

前端為 Vite 單頁應用程式：`src/` 下按 `pages / components / stores / hooks / lib / types / i18n` 分層，Zustand store 再分 `domain / feature / shared / devtools` 四層。

分層歸屬、允許的依賴組合與各 crate 角色登記見 [AGENTS.md](./AGENTS.md) 的「Rust 後端：Harness 架構準則」。

---

## 驗證門禁

```bash
npm run format:check                          # dprint 格式與行尾檢查
npm run lint:eslint                           # oxlint（ESLint 不支援 TS 7）
npm run typecheck                             # tsc --noEmit
npm run test:run                              # Vitest 單次執行
npm run test:e2e                              # Playwright E2E
npm run test:rust                             # Rust 測試（Windows 走 __TAURI_WORKSPACE__=true）
npm run ci:check                              # 本地 CI 鏡像門禁
node scripts/cargo-all-workspaces.mjs fmt-check
node scripts/cargo-all-workspaces.mjs clippy
node scripts/cargo-all-workspaces.mjs check
node scripts/cargo-all-workspaces.mjs nextest
```

`cargo check` / `cargo test` 全綠不等於通過 Clippy 門禁：`fmt` 必須帶 `--all`，`clippy` 必須帶 `--all-targets --all-features -- -D warnings`，且逐 workspace 執行。提交前按 CI 口徑跑一次。發版檢查見 [CHECKS.md](./CHECKS.md)。

---

## 版本與更新日誌

目前發佈版本 **v3.0.0**（2026-10-31）。版本條目以 [CHANGELOG.md](./CHANGELOG.md) 為唯一來源，含未發佈的 `[unreleased]` 段；`package.json`、各 workspace 的 `Cargo.toml` 與文件開頭「對應版本」須與之一致。

---

## 授權條款與上游關係

- 授權條款：[AGPL-3.0-only](./LICENSE)。
- 上游：本倉庫從 [`polite0803/AxAgent`](https://github.com/polite0803/AxAgent) 派生（git remote `upstream`），通用 AI 底座能力繼承自上游。
- 本倉庫位址：[`polite0803/AxInvest`](https://github.com/polite0803/AxInvest)。
