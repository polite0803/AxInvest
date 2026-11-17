# 股票分析决策链

> 状态：active ｜ 日期：2026-11-04 ｜ 对应版本：v3.0.0 ｜ 功能点：stock-decision-chain

## 定位

一次股票分析在同一轮工作流里产出超短 / 短 / 中 / 长四档**相互独立**的决策结论，并把分析师卡片的方向分歧合流成一个主档结论。入口是投资工作台的股票分析与后端命令 `run_stock_workflow`。

## 当前行为

### 四周期决策分支

- 四档的唯一枚举是 `axagent-core/crates/harness/src/holding_period.rs:31` 的 `Period`，`Period::ALL` 固定为 `[UltraShort, Short, Mid, Long]`（同文件 `:257`）。
- 默认持有期天数在 `holding_period.rs:294-299`：超短 2、短 5、中 28、长 90（交易日）。该文件头（`:8`）自述这是**全仓唯一天数来源**，禁止另写一份。
- 每档对应的 K 线周期在 `holding_period.rs:350-353`：`hourly` / `daily` / `weekly` / `monthly`；仓位系数在 `:286-289`：0.4 / 0.6 / 0.8 / 1.0。
- 分档模板按 `stock-horizon-<档>` 生成；主模板 `TEMPLATE_VERSION: i32 = 194` 定义在 `src-tauri/src/commands/stock_analysis_setup/seed_stock_analysis.rs:1643`，档模板版本常量 `HORIZON_TIER_TEMPLATE_VERSION` 在 `src-tauri/src/commands/stock_analysis_setup/horizon_tier_template.rs:762` 直接绑定前者（同值，不另立数字）。
- 四档结论以 JSON 落 `stock_analyses.horizon_decisions`；抽取与归一化在 `src-tauri/src/commands/stock_workflow/decision.rs:741`，LLM 输出的 snake_case 旧写法在 `src-tauri/src/commands/stock_workflow/decision.rs:1596` 统一改写为 camelCase。

（对应版本：v3.0.0，当前实现）

### 分析师卡片解析

- 四档各有独立产出（`--ultra_short` / `--short` / `--mid` / `--long`），公共实现收在一份脚本里：`src-tauri/src/commands/analyst-brief.rhai:6-17`。
- 卡片解析入口 `format_analyst` 在 `analyst-brief.rhai:45-87`，读取 `bull_score` / `bear_score` / `bull_points` / `bear_points` 并格式化为简报；字段缺失或解析失败时返回「数据不可用」，不静默补零。
- 前端按档取节点产物时用 `valueSlotKeyOf` 兼容「裸 base」与「带档节点 id」两种命名，避免逐档产物被丢弃：`src/stores/feature/stockAnalysisStore.ts:127-147`；节点标签到四档后缀的映射在 `src/utils/workflowNodeLabel.ts:25-49`（映射到 `stockAnalysis.timeHorizon*`）。

（对应版本：v2.8.6 起（`68bb5fd07`，2026-07-22），当前形态随 v3.0.0 分析师一致性呈现演进（`39b146563`））

### 方向分歧合流

- 分歧归因由 `src-tauri/src/commands/stock_workflow/hooks.rs:146` 的 `divergence_attribution` 生成，随决策快照输出（`:259` 调用、`:297` 落 `divergence` 键）。
- 主方向 `consensusDirection` 作为常量注入脚本作用域：`src-tauri/src/commands/stock_analysis.rs:265-285`；没有显式面板参数时只能从快照 `input_params` 取，取不到也不得伪造（`:278`）。
- `consensus_score` 通过 debate-convergence 的 `f2` 因子（权重 0.25）进入决策，注释见 `src-tauri/src/commands/stock_workflow/hooks.rs:603`。
- 主档的「来源」不自由填写：值域单点声明在 `holding_period.rs:242-248` 的 `HORIZON_SOURCES`（`branch_pick` / `formula_no_branch` / `formula` / `model` / `user`），落库白名单与门禁共用该数组。
- 从决策 JSON 中抽取 `horizonSource` 的实现在 `src-tauri/src/commands/stock_workflow/decision.rs:635`；`branch_pick` 表示主档由四档分支结论选出，`formula_no_branch` 表示四路分支全部未产出、退回后验阈值（必须与前一值可区分）。

（对应版本：v3.0.0，当前实现）

### 决策难度

- 当前实现**没有**名为 `difficulty` 的独立字段：全仓 `difficulty` 仅出现在 RL 课程 / 微调与基准评测的难度分级（`src-tauri/src/commands/evolution_engine.rs:1184`、`src-tauri/src/commands/fine_tune.rs`），与股票决策链无关。
- 现有可替代的「难度」信号是数据质量（`dqi_score` 及 A/B/C/D/F 档）与 LLM 自报 `confidence`；两者都进入决策卡但不叫「难度」（**待确认**：是否有未落库的口径）。

（对应版本：v3.0.0，当前实现）

## 关键代码入口

- 后端命令：`src-tauri/src/commands/stock_workflow/core.rs:249` — `run_stock_workflow`（入参含 `stock_code` / `dry_run` / `as_of_date`）。
- 引擎/业务层：`axagent-core/crates/harness/src/holding_period.rs:31`（四档枚举）、`:242`（来源白名单）、`:294`（持有期）、`src-tauri/src/commands/stock_workflow/decision.rs:741`（四档落库）、`src-tauri/src/commands/analyst-brief.rhai:45`（卡片解析）、`src-tauri/src/commands/stock_workflow/hooks.rs:146`（分歧归因）。
- 前端：`src/stores/feature/stockAnalysisStore.ts:127`、`src/utils/workflowNodeLabel.ts:25`、`src/types/stock-analysis.ts:62`（`HorizonPriceMap`，键为 camelCase 四档）。
- 数据表/迁移：`stock_analyses`（实体 `axagent-core/crates/entities/src/stock_analyses.rs`，含 `analysis_kind` 与 `horizon_decisions` 列）；`reco_picks`（`axagent-core/crates/entities/src/reco_picks.rs`）。迁移沿用实体所在的 `axagent-core/crates/migration`。

## 配置与开关

- `TEMPLATE_VERSION: i32 = 194`（`src-tauri/src/commands/stock_analysis_setup/seed_stock_analysis.rs:1643`）；`HORIZON_TIER_TEMPLATE_VERSION` 与之同值（`src-tauri/src/commands/stock_analysis_setup/horizon_tier_template.rs:762`）；`src-tauri/src/commands/stock_analysis.rs:10-13` 只引用模板常量，**禁止复制数字**（复制会与 `axagent-batch-rerun` 的口径分叉）。
- `HORIZON_SOURCES`（`holding_period.rs:242`）是 `horizonSource` 的字面量白名单。
- `Period::default_holding_days` / `kline_interval`（`holding_period.rs:294`、`:350`）是持有期与 K 线周期的唯一定义。
- i18n 前缀：`stockAnalysis.*`（`src/i18n/locales/zh-CN.json:15462` 起），时间档文案为 `stockAnalysis.timeHorizon*`。
- 未发现与该链路相关的环境变量或独立 feature flag（**待确认**）。

## 已知限制与坑

1. **「难度」无独立字段**：只有 `dqi_score` / `confidence` 可作替代，口径未统一（证据：`difficulty` 仅命中 RL 课程与微调，见 `src-tauri/src/commands/evolution_engine.rs:1184`、`src-tauri/src/commands/fine_tune.rs`）。
2. **`horizon_decisions` 有历史命名债**：v124 及更早的存量行是 snake_case（`ultra_short`），读取侧必须兼容两种写法（`src-tauri/src/commands/stock_workflow/decision.rs:699-741`；归一化在 `src-tauri/src/commands/stock_workflow/decision.rs:1596`）。
3. **模板版本只读**：`TEMPLATE_VERSION` 的权威定义在种子文件，命令层只引用（`src-tauri/src/commands/stock_analysis.rs:10-13`）；改图必须走模板迁移而不是改常量。
4. **四档天数禁止就地改**：`holding_period.rs:8` 明确 `default_holding_days` 是全仓唯一来源，另写一份会与回放/反思的 T+N 判据脱钩。
5. **分析师卡片缺失即降级**：`format_analyst` 在缺字段时返回「数据不可用」而非 0 分（`analyst-brief.rhai:45-87`），前端不能把该文案当真实评分。

## 版本变更记录

| 版本 | 日期 | 变更 | 证据 |
|---|---|---|---|
| v2.8.4 | 2026-07-12 | 股票分析引擎、量化回测、市场模拟合入；新增全业务管道编排器 `stock_pipeline` | `CHANGELOG.md` v2.8.4 节 |
| v2.8.5 | 2026-07-15 | 股票工作区 `StockWorkspace` 初版；股票业务双后端适配 | `CHANGELOG.md` v2.8.5 节 |
| v2.8.7 | 2026-08-02 | 股票分析 `as_of` 时点回溯增强；P0–P3 共 18 项股票业务能力强化；行情数据源与估值报告逻辑完善 | `CHANGELOG.md` v2.8.7 节 |
| v2.8.9 | 2026-08-07 | 强化学习引擎集成 + 股票自适应演化；行业学习基础设施 | `CHANGELOG.md` v2.8.9 节 |
| v2.9.1 | 2026-08-09 | 行业工作流重构与股票分析优化；新增条件单桥接、交易意图审计与风险检查 | `CHANGELOG.md` v2.9.1 节 |
| v2.9.9 | 2026-09-23 | 维度目录 / 反思统计 / 依赖指标新增；移除 `executionBridge`，修正 LLM 节点错误码误导 | `CHANGELOG.md` v2.9.9 节；提交 `fd03dc383` |
| v3.0.0 | 2026-10-31 | 四周期独立决策与四 horizon 反思落地；模板升至 v194（超短档个股炸板次数量纲修复） | `CHANGELOG.md` v3.0.0 节；提交 `11aa58772`、`26ffe4240` |

## 相关文档

> 与 `docs/INDEX.md` 功能视图「股票决策链」一节同源，按日期倒序；状态与对应版本均取自文档头部元信息。

- `PLAN-four-horizon-workflow-alignment.md` — active — v3.0.0+（未定版，待确认）
- `PLAN-r3h-long-sample-calibration.md` — active — v3.0.0+（未定版，待确认）
- `AUDIT-p0-p1-implementation-2026-10-26.md` — done — v2.9.9
- `AUDIT-stock-business-profitability-2026-10-26-1440.md` — active — v2.9.9
- `AUDIT-stock-business-profitability-2026-10-26.md` — active — v2.9.9
- `AUDIT-factor-evidence-2026-10-25.md` — active — v2.9.9
- `AUDIT-limitup-entry-2026-10-25.md` — active — v2.9.9
- `AUDIT-signal-efficacy-2026-10-25.md` — active — v2.9.9
- `AUDIT-stock-business-profitability-2026-10-25.md` — superseded — v2.9.9
- `AUDIT-stock-profitability-verdict-2026-10-25.md` — active — v2.9.9
- `PLAN-stock-analysis-latency.md` — active — v2.9.9
- `PLAN-validation-fill-model.md` — active — v2.9.9
- `AUDIT-stock-accuracy-2026-10-24.md` — active — v2.9.9
- `PLAN-hitrate-cost-basis.md` — done — v2.9.9
- `AUDIT-verdict-shape-normalization-2026-10-23.md` — active — v2.9.9
- `PLAN-decision-action-single-source.md` — active — v2.9.9
- `PLAN-llm-horizon-decision-and-parity.md` — active — v2.9.9
- `PLAN-capital-long-evidence.md` — active — v2.9.9
- `PLAN-horizon-scale-alignment.md` — active — v2.9.9
- `AUDIT-horizon-naming-2026-10-16.md` — active — v2.9.9
- `PLAN-evidence-factor-audit.md` — active — v2.9.9
- `PLAN-stock-decision-reliability.md` — active — v2.9.9
- `PLAN-horizon-decision-repair.md` — active — v2.9.9
- `PLAN-dcf-growth-single-point.md` — active — v2.9.9
- `AUDIT-analyst-low-confidence-2026-10-01.md` — active — v2.9.9
- `PLAN-multi-horizon-decision-science.md` — active — v2.9.9
- `PLAN-dcf-inapplicable-panel-gate.md` — active — v2.9.9
- `PLAN-horizon-four-cycle-closure.md` — active — v2.9.9
- `PLAN-dcf-fallback-anchor-applicability.md` — active — v2.9.9
- `AUDIT-300642-valuation-leg-cancellation-2026-09-23.md` — active — v2.9.9
- `AUDIT-decision-chain-structural-bias-2026-09-23.md` — active — v2.9.9
- `AUDIT-valuation-model-scientific-soundness-2026-09-23.md` — active — v2.9.9
- `PLAN-decision-action-convergence.md` — done — v2.9.9
- `PLAN-stock-analysis-fast-workflow.md` — active — v2.9.9
- `PLAN-stock-decision-hitrate-validation.md` — done — v2.9.9
- `PROPOSAL-stock-decision-four-horizon.md` — done — v2.9.9
- `AUDIT-300642-run-variance-2026-09-22.md` — active — v2.9.7
- `PLAN-stock-audit-fixes.md` — active — v2.9.7
- `AUDIT-analyst-low-confidence-2026-09-21.md` — active — v2.9.7
- `AUDIT-dcf-cashflow-anchor-2026-09-21.md` — active — v2.9.7
- `AUDIT-decision-action-consistency-2026-09-21.md` — active — v2.9.7
- `AUDIT-pledge-attribution-2026-09-21.md` — done — v2.9.7
- `AUDIT-valuation-param-db-override-2026-09-21.md` — active — v2.9.7
- `AUDIT-jev-adoption-stock-workflow-2026-09-20.md` — active — v2.9.7
- `AUDIT-601166-chain-break-2026-09-14.md` — active — v2.9.7
- `AUDIT-601166-valuation-decision-2026-09-14.md` — active — v2.9.7
- `AUDIT-sany-600031-decision-2026-09-13.md` — active — v2.9.7
- `AUDIT-xingye-601166-decision-2026-09-13.md` — active — v2.9.7
- `AUDIT-stock-business-2026-09-12.md` — active — v2.9.7
- `PLAN-VALUATION-CONFIG-WIRING.md` — active — v2.9.7

