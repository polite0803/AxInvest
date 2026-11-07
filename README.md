# AxInvest

**本地优先的 A 股投资研究工作台**：基于 Tauri 2 + Rust 的桌面应用，把行情与基本面取数、四周期个股决策、选股与复盘、模拟组合、量化回测、自动化任务和 AI 协作放进同一个工作区。全部业务数据默认保存在本地 SQLite，API Key 经 AES-256-GCM 加密，核心流程不依赖第三方云服务。

> **语言版本**：[English](./README-EN.md) | [繁體中文](./README-ZH-TW.md) | [日本語](./README-JA.md) | [한국어](./README-KO.md) | [Français](./README-FR.md) | [Deutsch](./README-DE.md) | [Español](./README-ES.md) | [Русский](./README-RU.md) | [हिन्दी](./README-HI.md) | [العربية](./README-AR.md)

---

## 项目定位

AxInvest 从 [`polite0803/AxAgent`](https://github.com/polite0803/AxAgent) 派生，通用 AI 底座继承上游，投资业务能力在本仓库演进。它面向的是「一条可复盘的投研链路」，而不是功能集合：

- **同一套时间世界观**：`live` 与 `as-of` 贯穿全部投资能力，历史批次可原样回放，样本可校准。
- **同一条决策链**：从候选池构造、四周期独立决策，到落库、归因与反思进化，口径只有一套。
- **本地优先**：默认 SQLite 落地，无云端依赖，数据与密钥留在本机。

---

## 功能导航（16 个功能点）

`docs/` 的入口是双视图索引 [docs/INDEX.md](./docs/INDEX.md)：功能视图按 16 个功能点组织，归档视图按方案 / 计划 / 审计 / 参考分类，当前共登记 265 篇文档。每个功能点都有一篇说明书，写清当前行为、代码入口、配置开关、已知限制与版本变更记录。

**业务域（9 个）**

| 功能点           | slug                   | 说明书                                                             |
| ---------------- | ---------------------- | ------------------------------------------------------------------ |
| 股票决策链       | `stock-decision-chain` | [stock-decision-chain.md](./docs/features/stock-decision-chain.md) |
| 趋势智选         | `serenity-screening`   | [serenity-screening.md](./docs/features/serenity-screening.md)     |
| 时间旅行回放     | `time-travel-replay`   | [time-travel-replay.md](./docs/features/time-travel-replay.md)     |
| 行情与数据源     | `market-data`          | [market-data.md](./docs/features/market-data.md)                   |
| OPC 能力包       | `opc-capability-packs` | [opc-capability-packs.md](./docs/features/opc-capability-packs.md) |
| 量化回测         | `quant-backtest`       | [quant-backtest.md](./docs/features/quant-backtest.md)             |
| 反思与进化       | `reflection-evolution` | [reflection-evolution.md](./docs/features/reflection-evolution.md) |
| 任务中心与调度   | `task-center`          | [task-center.md](./docs/features/task-center.md)                   |
| 模拟持仓与交易桥 | `paper-portfolio`      | [paper-portfolio.md](./docs/features/paper-portfolio.md)           |

**平台域（7 个）**

| 功能点           | slug                 | 说明书                                                         |
| ---------------- | -------------------- | -------------------------------------------------------------- |
| 插件体系         | `plugin-system`      | [plugin-system.md](./docs/features/plugin-system.md)           |
| 数据质量与门禁   | `data-quality-gates` | [data-quality-gates.md](./docs/features/data-quality-gates.md) |
| 运行时与智能体   | `agent-runtime`      | [agent-runtime.md](./docs/features/agent-runtime.md)           |
| 模型提供商与流式 | `provider-models`    | [provider-models.md](./docs/features/provider-models.md)       |
| 存储与数据库迁移 | `storage-migration`  | [storage-migration.md](./docs/features/storage-migration.md)   |
| 前端外壳         | `ui-shell`           | [ui-shell.md](./docs/features/ui-shell.md)                     |
| 构建与 CI        | `build-ci`           | [build-ci.md](./docs/features/build-ci.md)                     |

---

## 快速开始

环境要求：Node.js ≥ 24、Rust ≥ 1.85（edition 2024）、npm ≥ 10。Windows 需 MSVC Build Tools，macOS 需 Xcode Command Line Tools，Linux 需 `build-essential`、`libwebkit2gtk-4.1-dev`、`libssl-dev`。

```bash
npm install
npm run dev           # 浏览器模式（前端走 localStorage mock，无需 Tauri）
npm run tauri dev     # 桌面应用（Vite HMR + Tauri 窗口）
```

构建与产物：

```bash
npm run build         # 类型检查 + 前端生产构建
npm run tauri:build   # 桌面端安装包
```

---

## 架构分层（8 个 workspace 根）

Rust 侧按依赖方向拆为 **8 个 Cargo workspace 根**，共 **42 个库 crate**，`src-tauri` 作为组合根（主二进制 crate + `axagent-mobile`）。依赖方向铁律：`组件 → harness ← 实现`。

| workspace 根      | 角色                       | crate 数 | 主要内容                                                                                             |
| ----------------- | -------------------------- | -------- | ---------------------------------------------------------------------------------------------------- |
| `axagent-core`    | foundation                 | 7        | `harness` trait 与 DTO、`entities`、`plugin-proto`、`disk-cache`、`rt-theme`                         |
| `axagent-kit`     | implementor                | 1        | 公共实现层 `kit`                                                                                     |
| `axagent-ai`      | consumer + implementor     | 6        | `agent`、`orchestrator`、`providers`、`mcp`、`prompt-guard`、`trajectory`                            |
| `axagent-infra`   | implementor                | 12       | `dao`、`storage`、`migration`、`crypto`、`search`、`telemetry`、`document-parser` 等                 |
| `axagent-quant`   | implementor / consumer     | 5        | `astock-data`、`analysis-engine`、`company-runtime`、`quant`、`market-sim`                           |
| `axagent-plugins` | implementor + foundation   | 2        | `plugins`、`schema-gen`                                                                              |
| `axagent-runtime` | wiring + consumer + hybrid | 8        | `runtime`、`runtime-core`、`tools`、`rt-workflow`、`rt-messaging`、`gateway`、`crdt`、`notification` |
| `src-tauri`       | wiring                     | 2        | Tauri 命令、初始化与装配、`axagent-mobile`                                                           |

前端为 Vite 单页应用：`src/` 下按 `pages / components / stores / hooks / lib / types / i18n` 分层，Zustand store 再分 `domain / feature / shared / devtools` 四层。

分层归属、允许的依赖组合与各 crate 角色登记见 [AGENTS.md](./AGENTS.md) 的「Rust 后端：Harness 架构准则」。

---

## 验证门禁

```bash
npm run format:check                          # dprint 格式与行尾检查
npm run lint:eslint                           # oxlint（ESLint 不支持 TS 7）
npm run typecheck                             # tsc --noEmit
npm run test:run                              # Vitest 单次运行
npm run test:e2e                              # Playwright E2E
npm run test:rust                             # Rust 测试（Windows 走 __TAURI_WORKSPACE__=true）
npm run ci:check                              # 本地 CI 镜像门禁
node scripts/cargo-all-workspaces.mjs fmt-check
node scripts/cargo-all-workspaces.mjs clippy
node scripts/cargo-all-workspaces.mjs check
node scripts/cargo-all-workspaces.mjs nextest
```

`cargo check` / `cargo test` 全绿不等于通过 Clippy 门禁：`fmt` 必须带 `--all`，`clippy` 必须带 `--all-targets --all-features -- -D warnings`，且逐 workspace 执行。提交前按 CI 口径跑一次。发版检查见 [CHECKS.md](./CHECKS.md)。

---

## 版本与更新日志

当前发布版本 **v3.0.0**（2026-10-31）。版本条目以 [CHANGELOG.md](./CHANGELOG.md) 为唯一来源，含未发布的 `[unreleased]` 段；`package.json`、各 workspace 的 `Cargo.toml` 与文档头部「对应版本」须与之一致。

---

## 许可证与上游关系

- 许可证：[AGPL-3.0-only](./LICENSE)。
- 上游：本仓库从 [`polite0803/AxAgent`](https://github.com/polite0803/AxAgent) 派生（git remote `upstream`），通用 AI 底座能力继承自上游。
- 本仓库地址：[`polite0803/AxInvest`](https://github.com/polite0803/AxInvest)。
