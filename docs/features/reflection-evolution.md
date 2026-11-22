# 反思与进化

> 状态：active ｜ 日期：2026-11-04 ｜ 对应版本：v3.0.0 ｜ 功能点：reflection-evolution

## 定位

覆盖「决策做完之后回头看」这一层：四档持有期（T+2 / T+5 / T+28 / T+90）的到期反思、`horizon_results_json` 的消费与落库、未到期档进入每日待处理队列、`partial` / 期中观察（observing）的判定语义、归因与命中率统计，以及由此触发的自进化（分析师节点进化、RL 数值进化与技能进化）。

不含：四档持有期常量与 K 线周期本身的定义（归 `stock-decision-chain` 与 harness）、决策当下的产出（归 `stock-decision-chain`）。

## 当前行为

### 四档到期反思（T+2 / 5 / 28 / 90）

- 四档天数权威：`axagent-core/crates/harness/src/holding_period.rs:294-299`（2 / 5 / 28 / 90 交易日），不做第二份拷贝。
- 到期判据按档独立计算，批次入口 `run_reflection_workflow` 在 `src-tauri/src/commands/stock_workflow/reflection.rs:366`；每日档期由 `src-tauri/src/init/state.rs:3049` 的种子登记（「00:00 智能荐股 / 01:00 趋势智选 / 02:00 候选池分析 / 18:00 到期反思」）。
- 反思行的建点（pending）与代际盖章：`build_pending_reflection_rows_for` 在 `src-tauri/src/commands/stock_workflow/reflection.rs:2946`；pending→running 的原子 UPDATE 在 `:428-442`。
- 生成侧参数 `dueOnly`：`src-tauri/src/init/services.rs:3141` 的注释写明 `batch-reflection: { period, dueOnly, maxCount }`，`validate-backfill` 也要求与 `reflection-due-daily` 同口径（`src-tauri/src/bin/axagent-validate-backfill.rs:558`）。

（对应版本：v3.0.0，当前实现）

### `horizon_results_json` 的组装与消费

- 组装：`build_horizon_results_json` 在 `src-tauri/src/commands/stock_workflow/reflection.rs:4782`，模块头 `:4775` 说明它按目标数据契约逐档切片；主档快照在 `:4754`。
- 消费：写入侧在 `:1280`；成熟档按 `action` 投影到真实策略格在 `:1492`；到期反思 → `strategy_performance` 的代际血缘管道在 `:1489`。
- 建点行初始置空：`horizon_results_json: Set(None)`（`:472`、`:3006`），避免提前写入未成熟判定。

（对应版本：v3.0.0，当前实现）

### 未到期档与每日待处理队列

- 未到期档不阻塞批次：`dueOnly` 只处理已到期行（`src-tauri/src/init/services.rs:3141`）；未到期档在后续 18:00 批次中自然到期。
- 批次来自 `batch-reflection` 任务类型：`src-tauri/src/init/services.rs:3136` 注释「`batch-reflection`：设置页可建、前端零调用，真在跑的只有到期链路」。
- 批次级超时：单条 180s（CHANGELOG v3.0.0「反思批次单条超时 180s」；提交 `966e709f9`）。

（对应版本：v3.0.0，当前实现）

### `partial` / 期中观察（observing）语义

- 判定值域：`src-tauri/src/commands/stock_workflow/reflection.rs:41` 只接受 `correct | partial | wrong | incorrect | insufficient`，其余值一律判为不合格结构。
- `partial` 表示「方向对但幅度/时点不达标」，仍进入归因统计；`insufficient` 表示依据不足，不产生策略样本。
- 期中观察：`:1460` 注释「无法判定（无行情 / 中性档 / 期中观察）→ **不写行**」，即未成熟或中性档不落 `strategy_performance`；四态标签在 `:1404`（NULL = 尚未走到收尾）。

（对应版本：v3.0.0，当前实现）

### 归因与命中率统计

- `strategy_performance` 是命中率与胜率的唯一落点：写入在 `src-tauri/src/commands/stock_workflow/reflection.rs:689-710`（确定性短路路径）与 `:1483-1489`（正常路径）。
- 代际血缘：`:1492` 注释「`horizon_results_json` 中每一个成熟可判定档按 action→真实策略格」。
- 统计下沉：CHANGELOG v3.0.0「cron 种子模块归位 init；命中率统计下沉 engine；reco 查询下沉 dao」。
- 历史缺陷：`:1389-1394` 记录「只写入 `strategy_performance`、从未回写 `stock_reflections` 自身 ⇒ 三列实测 11/11 全 NULL」，回归判据为两处一致。

（对应版本：v3.0.0，当前实现）

### 自进化触发阈值

阈值全部在分析师节点进化评估器 `axagent-quant/crates/analysis-engine/src/node_evolution.rs`：

- 总均分低于 70 才考虑进化：`:103` 的 `avg_quality < 70.0`。
- 单维度低于 0.7 计入短板：`:113`（`score` 维度）与 `:122`（`direction` 维度），即用户口径的「score / direction 一致性 <70%」。
- 同一问题高频：`:180` 的 `count > total / 3`，即「高频问题 > 1/3」才升级为进化建议。
- 生成建议的入口在 `node_evolution.rs:161` 之后的构建段；执行侧由 `src-tauri/src/commands/evolution_engine.rs:517` 的 `run_skill_evolution_generation` 落命令。

（对应版本：v3.0.0，当前实现）

### RL 与技能进化（`crates/trajectory/`）

- 数值进化：`axagent-ai/crates/trajectory/src/numeric_evolution.rs:161` 的 `evolve_generation`。
- 技能进化：`axagent-ai/crates/trajectory/src/skill_evolution.rs:498` 的 `SkillEvolutionEngine`、同文件 `:588` 的 `evolve_generation_v2`。
- 引擎开关与状态：`src-tauri/src/commands/evolution_engine.rs:317`（`get_all_engine_status`）、`:355`（`start_engine`）、`:427`（`stop_engine`）、`:851`（`load_evolution_execution_stats_impl`）。
- 反思→进化的契约层早于业务层：v2.8.6 已落地「工作流反思/进化/优化三层 trait 契约」与「反思历史持久化到 trajectory_storage」（CHANGELOG v2.8.6 节）。

（对应版本：v3.0.0，当前实现；契约层自 v2.8.6 起）

## 关键代码入口

- 后端命令：`src-tauri/src/commands/stock_workflow/reflection.rs:366`（`run_reflection_workflow`）、`src-tauri/src/commands/stock_analysis.rs:6387`（`create_batch_reflection_cron`）。
- 引擎/业务层：`src-tauri/src/commands/stock_workflow/reflection.rs:2946`（建点）、`:4782`（`horizon_results_json`）、`:1489`（代际血缘）、`src-tauri/src/commands/evolution_engine.rs:517`、`axagent-quant/crates/analysis-engine/src/node_evolution.rs:103`。
- 前端：`src/components/stock-analysis/`（反思看板与执行层反思分栏）；具体组件按工作流页挂载。
- 数据表/迁移：`stock_reflections`、`strategy_performance`（DAO 调用在 `reflection.rs`，实体与迁移在 `axagent-core/crates/entities/` + `axagent-infra/crates/migration/`）。

## 配置与开关

- 四档天数：`axagent-core/crates/harness/src/holding_period.rs:294-299`。
- 反思判定值域白名单：`src-tauri/src/commands/stock_workflow/reflection.rs:41`。
- 批次参数：`{ period, dueOnly, maxCount }`（`src-tauri/src/init/services.rs:3141`）；单条超时 180s（CHANGELOG v3.0.0）。
- 自进化阈值：均分 <70（`node_evolution.rs:103`）、维度 <0.7（`:113`、`:122`）、高频 >1/3（`:180`）。
- 每日档期：`src-tauri/src/init/state.rs:3049`（18:00 到期反思）。
- i18n 前缀：`reflection.*`（`src/i18n/locales/zh-CN.json:4701`）。
- 未发现独立 feature flag；引擎启停走 `start_engine` / `stop_engine` 命令（**待确认**：是否有环境变量级开关）。

## 已知限制与坑

1. **期中观察/中性档不写样本** —— 无行情或未成熟时不写 `strategy_performance`（`reflection.rs:1460`），命中率分母因此偏小。
2. **`stock_reflections` 与 `strategy_performance` 曾不同步** —— 三列实测 11/11 全 NULL，判据是两处必须一致（`reflection.rs:1389-1394`）。
3. **pending 行盖章优先于分析行现值** —— 代际取 `pending_generation.or(analysis.template_version)`（`reflection.rs:553-555`），重跑改写分析行不会污染已建点代际。
4. **反思 sub-analysis 会撑爆模型窗口** —— 已加三层上下文预算（CHANGELOG v3.0.0；提交 `b89f30355` 一族）。
5. **教训退役环依赖每日触发** —— 缺省不触发会导致教训只增不退（CHANGELOG v3.0.0「教训退役环补每日触发」）。
6. **cron 档期归零/自愈** —— 反思输出三段编码改为递归取值后，cron 档期归零具备自愈（提交 `9037acf47`）。

## 版本变更记录

| 版本 | 日期 | 变更 | 证据 |
|---|---|---|---|
| v2.8.6 | 2026-07-20 | 工作流反思/进化/优化三层 trait 契约落地；WorkEngine 注入三层 trait 与触发钩子；反思历史持久化到 `trajectory_storage` | `CHANGELOG.md` v2.8.6 节「工作流反思/进化/优化三层 trait 契约落地」「WorkEngine 注入反思/进化/优化三层 trait 与触发钩子 (阶段 3)」「反思历史持久化到 trajectory_storage（优化 3）」 |
| v2.8.7 | 2026-08-02 | 接通上游反思/进化能力 | `CHANGELOG.md` v2.8.7 节「接通上游反思/进化能力 + 修复股票工作流执行路径（P1-3/P0-1/P0-2/P1-4/P1-5/P1-6）」 |
| v2.9.9 | 2026-09-23 | 股票分析业务迭代：反思统计与依赖指标 | `CHANGELOG.md` v2.9.9 节「股票分析业务迭代 — 维度目录/反思统计/依赖指标新增」 |
| v3.0.0 | 2026-10-31 | 股票反思四 horizon；到期反思→`strategy_performance` 代际血缘；反思参数应用台账（换代 + 到期两代命中率 + 人工回滚）；反思模板 v4 | `CHANGELOG.md` v3.0.0 节「股票反思四 horizon」「打通到期反思→strategy_performance 代际血缘管道」「反思参数应用台账」「反思模板 v4 的 mover_recall_context 登记为运行时注入」 |

## 相关文档

| 文档 | 状态 | 版本 | 关联 |
|---|---|---|---|
| `PLAN-missed-mover-reflection-closure.md` | active | v2.9.9 | 漏检回灌反思闭环 |
| `PLAN-chain-closure-remediation.md` | active | v2.9.9 | 链路闭环整改 |
| `PLAN-mover-coverage-and-reflection-exit.md` | active | v2.9.9 | 漏检覆盖与反思出口 |
| `PLAN-reflection-optimization-closure.md` | active | v2.9.9 | 反思优化收口 |
| `PLAN-reflection-per-horizon-row.md` | active | v2.9.9 | 逐档反思行 |
| `PLAN-reco-reflection-closure.md` | active | v2.9.9 | 荐股反思闭环 |
| `PLAN-stock-reflection-four-horizon.md` | active | v2.9.9 | 四档反思方案 |
| `AUDIT-stock-reflection-four-horizon-baseline-2026-09-23.md` | active | v2.9.9 | 四档反思基线 |
| `AUDIT-evolution-stats-backfill-2026-09-17.md` | active | v2.9.7 | 进化统计回填 |
| `PLAN-evoflow-borrowings.md` | active | v2.9.7 | EvoFlow 借鉴 |
| `AUDIT-evoflow-vs-axinvest-2026-09-12.md` | active | v2.9.7 | 与 EvoFlow 对比 |
| `PLAN-self-evolution-implementation.md` | 未标注 | v2.9.5 | 自进化实现 |
| `AUDIT-self-evolution-gap-2026-08-13.md` | 未标注 | v2.9.3 | 自进化缺口审计 |
