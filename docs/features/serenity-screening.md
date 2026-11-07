# 趋势智选 / Serenity 筛选

> 状态：active ｜ 日期：2026-11-04 ｜ 对应版本：v3.0.0 ｜ 功能点：serenity-screening

## 定位

从趋势 / 瓶颈候选池出发，用确定性评分加轻量链路筛出当日可分析的标的，并把结果写回候选池供工作流与分析使用。入口是投资工作台的选股页签（`ScreenerPage` / `SerenityScreeningPanel`）与后端命令 `run_serenity_screening`、`run_pool_scan`。

## 当前行为

### 候选池构造

- 池子从 `reco_picks` 读取，**不排除** `style='serenity'`，即智能荐股与趋势智选两类产物同池竞争：`src-tauri/src/commands/pool_scan.rs:232-343`。
- 过滤条件：`min_confidence`、`lookback_days`（`0` ⇒ 全池、不限进池日期）、当日已分析集合、`max_stocks`（用户配额）；排序为 `confidence` 降序：`pool_scan.rs:21-27`、`:55-59`。
- 单轮硬上限 `DEFAULT_MAX_PER_RUN = 20`（`pool_scan.rs:80`），与用户配额取小（`effective_max_per_run`，`:93-98`）；`run_deadline_seconds = 0` 时改用内置默认（`:103-107`）。
- 扫描入口 `run_pool_scan` 在 `pool_scan.rs:425`，批量分析入口 `run_analysis_for_candidates` 在 `:471`。

（对应版本：v3.0.0，当前实现）

### 去重

- `select_candidates`（`pool_scan.rs:168`）与 `dedup_explicit_candidates`（`:213`）按 `stock_code` 去重，**保留 confidence 最高的一条**，用于智能荐股与趋势智选撞车的场景。
- 同一规则复用于时间旅行批次：两条链候选合并后「同 code 留置信最高一条」再逐只回放（`src-tauri/src/commands/time_travel_batch.rs:19-20`）。

（对应版本：v3.0.0，当前实现）

### 快速链

- 趋势智选有两条模板链：`SERENITY_TEMPLATE_IDS: [&str; 2]` 在 `src-tauri/src/commands/stock_workflow/serenity.rs:23-24`，`run_serenity_screening` 只接受白名单内的 id（`:940-961`，非白名单直接拒绝）。
- 快速链模板由主模板派生：`FAST_TEMPLATE_ID` 定义在 `src-tauri/src/commands/stock_analysis_setup/seed_stock_analysis.rs:8543`，派生与种子化日志在 `src-tauri/src/commands/stock_analysis_setup/seed_stock_analysis.rs:10511`、`:10605`。
- 快速链与原链的差异写在种子文件顶部：段 A（取数 / 算法 / 聚合 / 简报）与段 E（落库契约）逐字相同（`src-tauri/src/commands/stock_analysis_setup/seed_stock_analysis.rs:8509`）；快速链**自建**同 id 取数节点、`FAST_REQUIRED_NODE_IDS` 摘掉四个评分节点（`:1011-1060`）。
- 前端类型层把「快速链没有 10 个分析师节点」判为设计行为而非失败：`feedback.skipped === "fast_chain"`（`src/types/stock-analysis.ts:1379-1394`）。

（对应版本：v3.0.0，当前实现）

### 来源优先级

- 模板侧的候选来源优先级：`candidate_pool`（确定性池）→ `chain_nodes` 中的代表性公司 → `top_pick`，见 `src-tauri/src/commands/stock_analysis_setup/seed_serenity.rs:718` 与 `seed_serenity_fast.rs:495`。
- 打分输入优先级：`serenity_score`（确定性评分链产物，快速链由 `strategy-scorer` 算出 `bottleneck_composite`）> `confidence`（LLM 自报）> 缺省中性 `50`，统一 clamp 到 `0..100`：`src-tauri/src/commands/stock_workflow/serenity.rs:721-731`。
- 趋势召回补充 `mover_watchlist_digest`，观察池上限 `MOVER_WATCHLIST_LIMIT = 20`：`src-tauri/src/commands/stock_workflow/serenity.rs:1009`、`:1077`。

（对应版本：v3.0.0，当前实现）

### 定时筛选

- 定时任务只认 `serenity-screening` 与 `serenity-screening-fast` 两条链（白名单校验 `workflow_id`），产物写候选池：`src-tauri/src/commands/recommendation_cron.rs:217-242`。
- 增删改查命令 `create/list/toggle/delete_trend_screening_cron` 在 `src-tauri/src/commands/recommendation_cron.rs:247-332`；池扫描侧的 `create/list/toggle/delete_pool_scan_cron` 在 `src-tauri/src/commands/pool_scan.rs:607-697`。

（对应版本：v3.0.0，当前实现）

## 关键代码入口

- 后端命令：`src-tauri/src/commands/stock_workflow/serenity.rs:947` — `run_serenity_screening`；`src-tauri/src/commands/pool_scan.rs:425` — `run_pool_scan`；`src-tauri/src/commands/pool_scan.rs:471` — `run_analysis_for_candidates`。
- 引擎/业务层：`pool_scan.rs:168`（选池）、`:213`（去重）、`src-tauri/src/commands/stock_workflow/serenity.rs:725`（评分优先级）、`src-tauri/src/commands/stock_analysis_setup/seed_stock_analysis.rs:8543`（快速链模板 id）。
- 前端：`src/components/stock-analysis/ScreenerPage.tsx`、`SerenityScreeningPanel.tsx`、`SerenityCandidateCard.tsx`、`SerenityValidationPanel.tsx`、`StockScreenerPanel.tsx`。
- 数据表/迁移：`reco_picks`（实体 `axagent-core/crates/entities/src/reco_picks.rs`，含 `as_of_date` 与 `style`）；分析结果落 `stock_analyses`（`axagent-core/crates/entities/src/stock_analyses.rs`）。

## 配置与开关

- `SERENITY_TEMPLATE_IDS`（`src-tauri/src/commands/stock_workflow/serenity.rs:23-24`）：唯一允许的模板 id 集合。
- `DEFAULT_MAX_PER_RUN = 20`（`pool_scan.rs:80`）；`max_stocks` / `max_per_run` / `min_confidence` / `lookback_days` / `run_deadline_seconds` 由 `PoolScanConfig` 透传（`pool_scan.rs:55-74`）。
- `MOVER_WATCHLIST_LIMIT = 20`（`src-tauri/src/commands/stock_workflow/serenity.rs:1009`）。
- i18n 前缀：`screener.*`（`src/i18n/locales/zh-CN.json:15079` 起）与 `stockAnalysis.*`（`:15462` 起）。
- 未发现该链路的独立环境变量（**待确认**）。

## 已知限制与坑

1. **快速链缺 10 个分析师节点是设计**：前端必须把 `skipped === "fast_chain"` 当正常态，否则会把快速筛选误报成失败（`src/types/stock-analysis.ts:1379-1394`）。
2. **模板 id 不接受任意值**：`run_serenity_screening` 会拒绝白名单外的 id 并在报错里列出允许值（`src-tauri/src/commands/stock_workflow/serenity.rs:940-961`）。
3. **空候选时 summary 可能是唯一解释**：上游趋势 / 瓶颈数据缺失时，LLM 通常把原因写在 `arguments.summary`，前端必须展示，否则用户只看到空列表（`src-tauri/src/commands/stock_workflow/serenity.rs:733-738`）。
4. **池扫描单轮有硬上限**：`max_per_run` 与用户 `max_stocks` 取小（`pool_scan.rs:57-59`、`:93-98`），大池会分轮次而不是一次跑完。
5. **两条 cron 命令族容易混用**：`*_trend_screening_cron` 走 serenity 链，`*_pool_scan_cron` 走池扫描（`recommendation_cron.rs:244-332`、`pool_scan.rs:606-697`）。

## 版本变更记录

| 版本 | 日期 | 变更 | 证据 |
|---|---|---|---|
| v2.8.4 | 2026-07-12 | 股票分析引擎、量化回测、市场模拟合入，筛选 / 候选池随股票业务回归 | `CHANGELOG.md` v2.8.4 节 |
| v2.8.7 | 2026-08-02 | P0–P3 共 18 项股票业务能力强化；行情数据源与估值报告完善 | `CHANGELOG.md` v2.8.7 节 |
| v2.9.1 | 2026-08-09 | 行业工作流重构与股票分析优化；条件单桥接与风险检查 | `CHANGELOG.md` v2.9.1 节 |
| v2.9.9 | 2026-09-23 | 模板列表按领域分组与领域筛选器；筛选链模板随领域工作流演进 | `CHANGELOG.md` v2.9.9 节 |
| v3.0.0 | 2026-10-31 | 荐股信号质量收口、全市场发现、`as_of` 取数修复；候选池批次留痕；荐股反思闭环 A1–A3 | `CHANGELOG.md` v3.0.0 节；提交 `733289d2e`、`f57c76c22` |

## 相关文档

> 与 `docs/INDEX.md` 功能视图「趋势智选」一节同源，按日期倒序；状态与对应版本均取自文档头部元信息。

- `PLAN-reco-systemic-rebuild.md` — active — v3.0.0+（未定版，待确认）
- `PLAN-serenity-full-coverage-predictive.md` — active — v3.0.0+（未定版，待确认）
- `PLAN-a2-backfill-validation.md` — active — v2.9.9
- `AUDIT-mover-miss-closure-2026-10-23.md` — active — v2.9.9
- `PLAN-mover-recall-defect-closure.md` — active — v2.9.9
- `PLAN-reco-signal-quality.md` — active — v2.9.9
- `PLAN-batch-automation-mover-recall.md` — active — v2.9.9
- `PLAN-reco-asof-generation.md` — active — v2.9.9
- `AUDIT-mover-universe-feasibility-2026-09-30.md` — active — v2.9.9
- `PLAN-mover-recall-attribution.md` — done — v2.9.9
- `PLAN-serenity-horizon-adaptation.md` — active — v2.9.9
- `PLAN-reco-horizon-science-alignment.md` — active — v2.9.9
- `PLAN-serenity-fast-workflow.md` — active — v2.9.9

