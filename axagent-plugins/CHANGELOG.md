# Changelog — AxAgent Plugins（插件层 workspace）

本 workspace 的版本演进记录。版本号唯一权威：`Cargo.toml` 的 `[workspace.package].version`；
全部成员 crate（`plugin-opc` / `plugin-stock` / `plugins` / `schema-gen` / `stock-core`）
以 `version.workspace = true` 继承该值，**不得在成员 crate 单独写 version**。

## 版本演进约定

- 主仓应用的 `npm run bump`（`scripts/bump-version.mjs`）只覆盖 `package.json` /
  `src-tauri/tauri.conf.json` / `src-tauri/Cargo.toml(+lock)`，**不触及本 workspace** ——
  插件 SDK 版本号手动演进。
- 演进步骤：改 `Cargo.toml` 的 `[workspace.package].version` → 在本 workspace 根
  `cargo update -w`（同步 Cargo.lock）→ 在本文件追加条目。
- 独立仓前置三判据（依赖面收敛 / SDK 版本化 / CI 独立跑绿）见主仓
  `docs/plans/PLAN-shell-plus-plugins.md` §157。

## [3.0.0]

初版（当前态快照，2026-11-13）：

- `plugins`：宿主侧插件运行时 —— 清单发现、启停、长驻 worker 管理、T2 命令/接缝分派。
- `plugin-opc`：OPC（OpenOPC）域 worker —— 130 条命令；业务命令全部走 T2 命令帧 +
  `opc.store` 接缝回宿主，依赖面收敛到 `axagent-plugin-proto`（协议 crate）。
- `plugin-stock`：股票域 worker —— 41 条命令；仍依赖 `axagent-analysis-engine`（path
  依赖，独立仓判据① 未闭合，stock 分析逻辑权威实现仍在其中）。
- `stock-core`：foundation —— 股票域纯函数（意图解析等），零 `axagent-*` 依赖。
- `schema-gen`：构建期 IPC 类型生成器，无运行时消费方。
- 协议 crate `axagent-plugin-proto` 在 `axagent-core` workspace；2026-11-13 内联化改造后
  （version/edition/thiserror/lints 不再继承 workspace 根），其版本权威为自身 `Cargo.toml`
  的 `version = "3.0.0"` —— 内联原因：SDK 独立仓验证形态把它随行拷出，目录里没有
  `axagent-core` workspace 根，继承字段会让 manifest 解析直接失败。宿主与插件 worker
  两侧同源消费，不得各写一套帧形状。

[3.0.0]: 与主仓应用同值起步，无更早条目。
