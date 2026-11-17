# 时间旅行回放

> 状态：active ｜ 日期：2026-11-04 ｜ 对应版本：v3.0.0 ｜ 功能点：time-travel-replay

## 定位

把分析锚到历史某个交易日 `as_of`，用当时可见的数据重跑选股链并产出对照结论，用来验证策略在历史上是否成立。入口是 `/invest/replay` 的回放工作台（`ReplayWorkbenchPage`）与后端命令 `run_time_travel_batch`。

## 当前行为

### as-of 封闭世界

- 选定的 `as_of` 通过 `AsOfContext` 建立封闭世界：结构体 `axagent-core/crates/harness/src/as_of.rs:57`，构造函数 `:70`，解析入口 `:101`。
- 回放分析的 `analysis_kind` 固定为 `replay`，与 live 只分两态、业务出处由 `source` 承担（`axagent-core/crates/harness/src/analysis_kind.rs:7-8`）；`ab_test` 从未存在（同文件 `:21`）。
- 两条链都在 guard 内回放：荐股链候选落 `reco_picks` 且每行带 `as_of_date`；趋势智选链同口径落库但**不写进程内全局种子**（无代际可打）。两链候选同 code 去重保留置信最高一条后逐只分析（`src-tauri/src/commands/time_travel_batch.rs:12-21`）。
- 防污染手段从「跳过写库」改为「写库 + 打代际标记」（铁律 B2，2026-10-12 改写）：`time_travel_batch.rs:16-17`。

（对应版本：v3.0.0，当前实现）

### 批次编排

- 运行态放进程内 `BATCH_RUNS`（轮询低延迟）；每次状态变更同步 upsert 到 `time_travel_batch_runs` 表，重启后由 `get_time_travel_batch` 从库还原：`time_travel_batch.rs:2-10`。
- 实体 `axagent-core/crates/entities/src/time_travel_batch_runs.rs`，读写收口在 `axagent_dao::repo::time_travel_batch_runs`（同文件注释 `:6-7`）。
- 进程异常退出遗留的 `running` 行在新进程首次访问时收敛为 `interrupted`；as_of 栈是进程级的，**不支持跨进程续跑**（`time_travel_batch.rs:9-10`）；清理陈旧运行态的实现 `src-tauri/src/commands/stock_workflow/reflection.rs:3254`。
- 批次启动主流程 `run_time_travel_batch_inner`（`time_travel_batch.rs:571-725`），内部步骤：解析周期档 / `as_of` → 校验快照覆盖 → 拒绝 live 窗口 → 取全局批次锁 → 创建状态 → 后台执行。

（对应版本：v2.9.9 起（提交 `57c1f86e6`，2026-10-11，定标 v2.9.9），当前实现为 v3.0.0）

### 与 live 互斥

- 时间窗拒绝 `in_live_window` 在 `time_travel_batch.rs:339-358`：北京时间 `23:50-02:10` 与 `17:50-18:10` 两个窗口内拒绝发起批次（as_of 栈是进程级的，会和 live 串扰）。
- 拒绝文案见 `time_travel_batch.rs:654`：「当前处于 live 流水线时间窗（23:50-02:10 / 17:50-18:10）」。
- 第二道互斥是 AppState 上的全局批次锁，`try_lock` 语义：已在跑直接拒绝、不排队（`time_travel_batch.rs:655`）。

（对应版本：v3.0.0，当前实现）

### 准入与可选日期

- `require_pool_snapshot_coverage`（`time_travel_batch.rs:358-466`）校验该 `as_of` 是否有可回放的建池快照；缺全部源时拒绝，缺一个源则放行但显式降级说明。
- `list_replayable_as_of_dates`（`:505-558`）只列出同时满足三条的日期：快照能产出候选股、是交易日、且持有期已到。

（对应版本：v3.0.0，当前实现）

### 四档反思

- 批次**退出 guard 后**才做反思（铁律 B3），否则 guard 内「今天」= `as_of` 会算错到期：`time_travel_batch.rs:23`。
- 到期档（`hindsight ≤ 北京今日`）立即反思；未到期档留 `pending`，由既有 `18:00 batch-reflection` cron 补做：`time_travel_batch.rs:25-26`。
- 四档即四周期持有期：超短 2 / 短 5 / 中 28 / 长 90 交易日（`axagent-core/crates/harness/src/holding_period.rs:294-299`）。
- 反思执行 `run_batch_reflection`（`src-tauri/src/commands/stock_workflow/reflection.rs:1945`）与 `run_batch_reflection_inner`（`:3282`）；批次侧的触发在 `time_travel_batch.rs:1109`。
- 可关闭「guard 外立即反思已到期档」（`false` = 只分析、反思全留给 cron）：参数注释 `time_travel_batch.rs:93`。

（对应版本：v3.0.0，当前实现）

## 关键代码入口

- 后端命令：`src-tauri/src/commands/time_travel_batch.rs:730-740` — `run_time_travel_batch`；`:754` — `get_time_travel_batch`；`:746` — `list_time_travel_batches`。
- 引擎/业务层：`axagent-core/crates/harness/src/as_of.rs:57`（封闭世界）、`src-tauri/src/commands/stock_workflow/reflection.rs:1945`（批次反思）、`:3282`（反思主体）。
- 前端：`src/components/time-travel/PageTimeAnchor.tsx:45-67`（实时 / 历史切换）、`ReplayBadge.tsx:8-45`（回放标识）、`ReplayWorkbench.tsx`、`TimeTravelBatchCard.tsx`、`AsOfDatePicker.tsx`；路由见 `src/components/layout/ContentArea.tsx:309-311`。
- 数据表/迁移：`time_travel_batch_runs`（实体 `axagent-core/crates/entities/src/time_travel_batch_runs.rs`）；候选落 `reco_picks`、分析落 `stock_analyses`（`analysis_kind='replay'`）。

## 配置与开关

- live 互斥窗口常量写在 `time_travel_batch.rs:339-358`（没有独立命名常量，改动需同步注释与文案）。
- 立即反思开关：`time_travel_batch.rs:93`（`false` ⇒ 全部交给 `18:00 batch-reflection` cron）。
- 反思档位天数来自 `Period::default_holding_days`（`holding_period.rs:294`），不在回放侧另写一份。
- i18n 前缀：`replayWorkbench.*`（`src/i18n/locales/zh-CN.json:14990` 起）。
- 未发现环境变量或 feature flag（**待确认**）。

## 已知限制与坑

1. **不能跨进程续跑**：`running` 行在重启后变 `interrupted`，因为 as_of 栈是进程级的（`time_travel_batch.rs:9-10`）。
2. **快照缺源只降级不阻断**：缺一个源会放行并在结果里标注降级，比较结论时容易把降级当成策略失效（`time_travel_batch.rs:358-466`）。
3. **回放依赖当前代码与模板版本**：`as_of` 只封闭数据，不封闭代码；代码漂移会污染历史结论（`time_travel_batch.rs:16-17` 的代际标记正是为此）。
4. **反思有 pending 窗口**：未到期的档不会立刻有结论，需要等 cron 补做（`:25-26`）。
5. **批次原计划文档不在 `docs/`**：实现依据 `.trae/documents/time-travel-batch_plan.md`（工具目录，不纳入 `docs/INDEX.md`）；是否归档或迁入待确认。

## 版本变更记录

| 版本 | 日期 | 变更 | 证据 |
|---|---|---|---|
| v2.9.9 | 2026-09-23 | 时间旅行批次复盘上线（批次状态、`as_of` 回放与四档反思骨架）；批次历史读写下沉 dao | 提交 `57c1f86e6`（2026-10-11，定标 v2.9.9）、`7ff23b09a`、`05988e6f4` |
| v3.0.0 | 2026-10-31 | 回放日更与回填验证闭环；调度器无头化 + 跨进程 leader 租约 + 回放/验证双门禁；落库批次键收成 `BatchKey` 锁住四条落库链 | `CHANGELOG.md` v3.0.0 节；提交 `39b146563`、`d787ebf37`、`929cfd2f9` |

## 相关文档

> 与 `docs/INDEX.md` 功能视图「时间旅行回放」一节同源，按日期倒序；状态与对应版本均取自文档头部元信息。

- `PLAN-r2-daily-replay.md` — active — v3.0.0+（未定版，待确认）
- `AUDIT-replay-sample-feasibility-2026-10-25.md` — active — v2.9.9
- `PLAN-asof-replay-quality-attribution.md` — active — v2.9.9
- `PLAN-asof-gaps.md` — active — v2.9.9

