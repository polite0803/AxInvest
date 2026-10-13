# PLAN：股票批量自动化闭环（选股→分析→反思→优化）+ 妖股反推

> 立项：2026-10-13
> 状态：待评审
> 范围：在保留现有手动选股/分析/反思入口的前提下，由系统完成批量自动化选股、分析、反思、优化；
> 并具备根据四周期（超短/短/中/长）妖股（系统已定义为「窗口涨幅达标」事件）反推过去周期的分析与反思能力。

## 1. 背景与目标

当前用户手动完成选股与分析、反思，系统侧已存在大量自动化零件但闭环不完整。本方案的目标：

1. **前向闭环自动化**：批量选股 → 批量分析 → 到期反思 → 受控优化，全部可定时执行。
2. **后向反推闭环**：事后按四周期阈值识别妖股（窗口涨幅达标事件）→ 对漏检标的补跑 as-of 历史分析 → 生成漏检反思与教训 → 反哺优化。
3. **优化环节保留人工节奏**：进化计划必须经审批闸门接受/拒绝，不允许无人值守的自动变异（沿用业务调优依赖人工专家经验的既定原则，但把「只能手点按钮」升级为「系统自动出计划、人审批」）。

## 2. 现状全景

### 2.1 调度基础设施（已具备，直接复用）

| 能力 | 位置 | 说明 |
|---|---|---|
| 定时任务模型 | `crates/runtime-core/src/cron_job.rs:47` | `CronJob`：cron + task_type + workflow_id + params + priority（含 `Batch` 级），持久化 `cron_jobs` 表 |
| 调度器 | `src/scheduler/mod.rs:3`、`src/scheduler/queue.rs` | 不起新线程，`CronJobStore::list_due` 轮询触发，支持优先级 |
| 执行分发 | `src/init/services.rs:2230` | 各 task_type 分支；内置交易日闸门（非交易日 skip 并 record_run 留痕） |
| 手动执行 | `src/commands/scheduled_task.rs:343` | `trigger_now` 立即跑指定任务；内置任务白名单 `BUILTIN_TASK_TYPES`（`scheduled_task.rs:113`），新增内置 task_type 必须同步登记，否则「立即执行」报不支持 |
| 前端管理 | **任务中心** `src/pages/TasksCenterPage/`（导航「任务中心」） | 三个 tab：概览 / 定时任务 / 全部运行。定时任务 tab 全量列 `cron_jobs`（`list_scheduled_tasks`），分组与「立即执行」可用性由 `taskGrouping.ts` 两张前端白名单判；全部运行 tab 走适配器注册表 `adapters/registry.ts`，现有三源：cron（定时运行记录）、pipeline（投资管道）、timeTravel（时间旅行批次复盘/回放扫描） |

### 2.2 五条内置每日任务（选股/分析/反思已自动化）

种子：`src/init/stock_automation_cron_seed.rs:71`（幂等语义：存在即跳过，用户改时刻不被回滚）。

| 时刻 | 任务 | task_type | 四周期覆盖 | 产物 |
|---|---|---|---|---|
| 00:00 | 智能荐股 | `stock-recommendation` | 四档全跑，top5/档，置信度≥50 | `reco_picks` 四档行 |
| 01:00 | 趋势智选 | `trend-screening`（serenity 工作流） | serenity 风格 | `reco_picks` style=serenity |
| 02:00 | 候选池逐只深度分析 | `pool-scan` | 全池 +「当日未分析」判据，跨日积压自动补 | `stock_analyses` + 反思 pending 行 |
| 07:00 | 决策回测 | `decision-backtest` | T+5/20/60 验证历史 pick | `decision_validations`（rank IC/逐档权重唯一样本源） |
| 18:00 | 到期反思 | `batch-reflection` | `period=None + dueOnly=true`，一条任务覆盖四档 | `stock_reflections` resolved |

四周期判据（权威源 `crates/harness/src/holding_period.rs:31`，窗口天数 `holding_period.rs:246`）：
超短 2 交易日 / 短 5 / 中 28 / 长 90。

### 2.3 妖股定义与现有反推能力

- **定义**：妖股在系统内命名为「窗口涨幅达标」事件（刻意不含涨停语义，见
  `crates/analysis-engine/src/mover_recall.rs:9` 命名边界）。阈值（`mover_recall.rs:40`）：
  超短 10% / 短 20% / 中 30% / 长 40%，阈值变量 `mover_gain_*`，窗口复用 `default_holding_days()`。
- **现有能力**：命令 `analyze_mover_recall`（`src/commands/mover_recall.rs:69`）+ 前端
  `src/components/stock-analysis/MoverRecallPanel.tsx:36`。纯机械只读核查：枚举达标事件 →
  对照 `reco_picks` / `seed_pool_json` / 风格淘汰留痕做五层归因
  （同期已推荐 / 他档推荐=档错配 / 候选池降级 / L3 风格打分淘汰 / by-design / unexplained）。
- **边界**：① 只出面板报告，不补跑历史分析；② 不写反思、不生教训；③ 无调度（五条 cron 无它）；
  ④ 归因不驱动任何优化。`Unexplained` 层占比高时禁止据此下优化结论（留痕不足，`mover_recall.rs:15`）。

### 2.4 反思与优化链路现状

- 批量反思：`src/commands/stock_workflow/reflection.rs:2255` `run_batch_reflection_inner`
  消费 pending 队列，落 `stock_reflections`（verdict / lesson_summary / what_went_wrong /
  missed_signals / fix_for_future / mover_label 等）。
- 股票进化引擎：`crates/analysis-engine/src/stock_self_evolution.rs:144` `StockSelfEvolutionEngine`，
  做参数进化 + 流程进化，产物落 `stock_evolution_history`；触发入口仅手动
  （`trigger_stock_evolution`，`src/commands/stock_analysis.rs:6218`，前端 EvolutionDriftPanel 按钮）。
- 其他手动优化入口：`evolve_node_command` / `evolve_analyst_command`
  （`src/commands/analyst_evolution.rs:56`）、`manual_recalc_strategy_weights`、
  `run_lesson_validation_command`。
- 批量反思代码对进化引擎**零调用**：反思 → 优化之间没有自动接线。

## 3. 专项调查：股票模板自动进化为什么被「关」？

> 用户明确表示这不是其设计意图。调查结论：存在**三层错位**，且保护代码当前是死代码。

### 3.1 时间线（git 实证）

| 日期 | commit | 事件 |
|---|---|---|
| 2026-07-20 | `8769b9f1f` | feat(stock)：接通上游反思/进化能力，同时新增 `src/init/axinvest_decorators.rs` 并在 init 接线，对 `stock-*` 模板强制 `should_auto_evolve=false` |
| 2026-08-29 | `992947dfc` | chore：清理 AxInvest 股票业务模块（删除 1092 文件），装饰器接线点被删 |
| 2026-09-02 | `ffbab4f01` | 从删除前版本恢复 6 个股票 crate + 55 个实体；**装饰器文件被恢复，但 init 接线未恢复** |

现状：`AxInvestEvolverDecorator` / `AxInvestReflectorDecorator` 全仓仅被自身单测引用
（`grep AxInvestEvolverDecorator` 仅命中 `axinvest_decorators.rs` 本文件），运行时注册的 evolver 是
`src/init/state.rs:687` 的**裸** `WorkflowEvolverImpl::with_defaults()`，无装饰器包装。

### 3.2 当初为什么要关（理由成立，但针对的是另一条链）

上游 `WorkflowEvolverImpl` 的自动触发规则（`crates/trajectory/src/workflow_adapters/evolver.rs:542`）：
最近 20 次运行中 `Failed + PartiallyCompleted` 占比 ≥ 50%（`auto_trigger_success_threshold=0.5`、
`min_usages=3`，`crates/trajectory/src/skill_evolution.rs:88`），且当次质量分 ≤ 5
（调用点 `crates/rt-workflow/src/work_engine/engine/mod.rs:5707`），即 LLM 自动变异模板
（改节点/边/变量）。

股票工作流的高频失败原因是**数据可达性**（vendor 限流、行情缺失、`__untrusted` 标记），
不是工作流结构问题——同一提交里的 ReflectorDecorator 恰恰在扫 `__untrusted` 节点可证。
失败率阈值会把「取数失败」误诊为「模板该变异」，LLM 据此乱改专家节点/prompt，破坏业务节奏。
故当时的保护意图是合理的：**阻断「失败率驱动的无人值守模板结构变异」**，
而不是阻断「基于业务反思的参数/策略优化」（后者另有 `StockSelfEvolutionEngine` 手动链，从未被阻断）。

### 3.3 当前真实行为：判定开着、保护没挂、变异空转

1. `record_reflection` 每次工作流执行后都在喂数据（`engine/mod.rs:5701`），stock 模板失败率统计持续累积；
2. 一旦满足阈值，`engine/mod.rs:5718` 直接调裸 evolver `run()`——**装饰器保护实际不存在**；
3. 但 `run()` 只返回 `WorkflowModification`，调用处 `if let Err(e)` 丢弃了 `Ok` 结果，
   且 `run()` 内部无 apply/save/落库动作（`evolver.rs:452` 起的实现）——
   **变异计划无人应用，自动进化名义开着、实际空转**，真正生效的进化只有手动 `trigger_stock_evolution`。

### 3.4 处置建议（纳入本方案 P4）

- 不要简单恢复装饰器：那会继续让系统停留在「只能手点」的状态，与本次目标冲突。
- 也不要放任裸链：一旦未来 engine 开始 apply modification，stock 模板会被失败率误触发的变异直接改写，无审批无回滚。
- 正解：删除或停用装饰器，在 `StockSelfEvolutionEngine` 上建立**反思/漏检驱动 → 出进化计划 → 审批闸门 → 接受才落库**
  的受控自动链（复用 `AppState.evolution_consent_senders` 既有审批通道），并显式按失败原因分流：
  数据可达性类失败（`__untrusted` / vendor 错误）永不触发模板结构变异，只计入数据质量监控。

## 4. 差距分析

| 目标能力 | 现状 | 缺口 |
|---|---|---|
| 批量自动选股 | 已有（00:00/01:00） | 无 |
| 批量自动分析 | 已有（02:00 池扫描） | 无 |
| 批量自动反思 | 已有（18:00 到期反思） | 无 |
| 批量自动优化 | 仅手动；裸自动链空转；保护代码是死代码 | 受控自动链 + 审批闸门 + 失败原因分流 |
| 妖股定时核查 | 仅手动面板 | 第 6 条内置 cron |
| 妖股漏检 → 补历史分析 | 无 | as-of 时点重放分析，synthetic 标记 |
| 妖股漏检 → 补反思/教训 | 无 | 漏检事件转反思样本，独立 source 分类 |
| 漏检教训 → 优化 | 无 | 与到期反思、decision_validations 聚合后驱动进化计划 |

## 5. 方案设计（四阶段，逐阶段可验收）

### P1 调度化：妖股核查 + 优化聚合任务

1. 新增内置 cron `mover-recall-weekly`（建议周六 09:00；长档窗口 90 天，周频足够），
   task_type `mover-recall`，在 `src/init/services.rs` 增执行分支调用
   `analyze_mover_recall` 的内部逻辑（抽出可复用 inner 函数，命令与 cron 共用），
   产物：漏检事件快照落新表 `mover_recall_runs`（见 §6）+ 桌面通知摘要。
2. 新增内置 cron `evolution-proposal-daily`（建议 20:00，在 18:00 反思之后）：
   聚合当日新增的三类样本（到期反思 / decision_validations / mover 漏检），调用
   `StockSelfEvolutionEngine` 的**计划生成**接口产出进化建议（参数/权重/风格阈值/horizon_prior），
   状态为 `pending_approval`，推送审批通知；**不直接接受**。
3. 种子文件沿用「存在即跳过」幂等语义，固定 job id 写入
   `src/init/stock_automation_cron_seed.rs`，同步补交易日/周末闸门判定（核查依赖收盘快照，
   周五数据周六跑；节假周末同样 skip 留痕）。
4. **任务中心接线（强制，否则种子任务虽自动出现但分组错/不能手动跑）**：
   - 后端 `src/commands/scheduled_task.rs:113` `BUILTIN_TASK_TYPES` 登记
     `mover-recall`、`evolution-proposal`（该表注释已立规：与 services.rs 执行分支一一对应）；
   - 前端 `src/pages/TasksCenterPage/taskGrouping.ts` 两处同步：`INVEST_TASK_TYPES`
     （归入「投资」组而非「自定义」组）与 `BUILTIN_EXECUTABLE_TYPES`（点亮「立即执行」火箭按钮）；
   - 两个新类型属系统内置，**不进**「新建任务」菜单（`CreateJobsMenu.tsx` /
     `forms/InvestTaskFields.tsx` 不加选项），与现有五条内置任务同口径——只能由种子创建、
     可在任务中心启停/改时刻/立即执行/删除；
   - 运行记录经现有 `record_run` 机制自动进入任务中心「全部运行」tab 的 cron 源，无需新适配器；
   - 若新增任务类型专属文案（如任务中心侧栏/通知模板），按禁区 2 补齐 11 语言；任务名本身
     用种子 `name` 字段（与现有内置任务一致），不新增前端硬编码。

### P2 妖股漏检 → as-of 历史补分析

1. 消费 P1 落库的漏检事件（`counts_as_miss()` 为真的 `MissRow`），排除
   `PoolDegraded` / `ByDesign` / `Unexplained` 高占比批次（数据与留痕不足，不进样本）。
2. 以窗口起点（`window_start`）为 as-of 日期，按 `crates/analysis-engine/src/backtest.rs:219`
   `backtest_history` 的既有 HistoricalAnalysis 回放口径，用当时可得行情重放 stock-analysis
   工作流（禁止引用 as-of 之后的数据——vendor 侧已有历史 K 线能力，需审计每个节点的取数时点）。
3. 落 `stock_analyses` 时打标记：`source='mover_recall'`、`synthetic=1`、
   `anchor_event_id` 关联漏检事件。决策回测的 `exclude_synthetic` 口径已存在
   （`backtest_validation.rs` 的 RunDecisionBacktestRequest），确保补分析不污染真实推荐命中率分母。
4. 命令 `run_mover_recall_analysis`（手动/定时两用）+ 配额参数（单批上限、最小间隔、失败重试一次）。
5. **批次运行进任务中心**：补分析是长批量 as-of 回放作业，与「全部运行」tab 既有
   timeTravel 源（时间旅行批次复盘/回放扫描，`adapters/timeTravel.adapter.ts`）同族。
   复用其后端批次登记表与进度事件（不新造第四套 run 模型）；若回放作业的 run 结构与
   timeTravel 现有 source 无法共用，则在 `adapters/registry.ts` 注册新源（如 `mover-recall`），
   不允许批次进度只活在 MoverRecallPanel 里。批次明细抽屉展示逐票状态（待补/分析中/已反思/失败），
   支持从任务中心直接发起手动批（即第 4 条命令的 UI 入口放在任务中心，而非股票分析页内部）。

### P3 漏检 → 结构化反思与教训

1. P2 产物自动进入反思 pending 队列（复用 `run_reflection_workflow`，
   `src/commands/stock_workflow/reflection.rs:168`），反思模板对漏检样本增加固定三问：
   启动前可见信号有哪些（missed_signals）、卡在哪一层（pool/L3/档错配）、未来判据如何修（fix_for_future）。
2. `stock_reflections` 增 `source` 列（`due` 到期反思 / `mover_recall` 漏检反思），
   命中率统计按 source 分桶，避免两类样本混在一个分母。
3. 教训入模前过 `reflection_lesson_validator`（既有），校验失败的教训只留档不生效。
4. 前端 MoverRecallPanel 的漏检明细行支持展开查看补分析报告与反思结论。

### P4 受控自动优化（替代装饰器的最终形态）

1. 停用并删除 `axinvest_decorators.rs`（死代码；如保留 ReflectorDecorator 的 `__untrusted`
   扫描价值则只删 Evolver 部分并重新接线 Reflector 部分——实施时先确认该 metadata 是否有消费方）。
2. 进化触发改为**计划-审批两段式**：
   - 计划段（自动）：三类样本聚合 → `StockSelfEvolutionEngine.trigger_manual_evolution`
     同款执行链产出 `StockEvolutionResult`，落 `stock_evolution_history` 时状态记 `pending_approval`；
   - 审批段（人工）：前端 EvolutionDriftPanel 增待审批列表，接受才回写参数/权重/horizon_prior，
     拒绝则记原因并回灌为反例。审批通道复用 `AppState.evolution_consent_senders`。
3. 失败原因分流硬规则：反思记录中失败归因为数据可达性（`__untrusted` / vendor 限流 /
   行情缺失）的样本只进数据质量看板，永不触发模板结构（节点/边）变异；
   仅业务质量类（方向错、漏信号、风格阈值不当）样本驱动进化。
4. 可观测：每次自动计划记录触发样本 id 列表、改动 diff、回测概要（已有 persist_evolution 基础），
   支持一键回滚到上一个已接受版本。
5. **审批待办进任务中心概览**：`pending_approval` 计划除 EvolutionDriftPanel 外，
   在任务中心概览 tab（`OverviewTab.tsx`）展示待审批计数与入口（卡片/角标），
   点击进入审批列表；审批动作（接受/拒绝/查看 diff/回滚）在统一抽屉完成。
   即「系统自动出计划」的产物在任务中心可见可处理，不依赖用户记得打开股票分析页。

### 5.5 任务中心集成矩阵（横切验收口径）

所有新自动化能力的统一归宿是任务中心，不允许出现「只在股票分析页内可见」的后台作业。

| 新能力 | 定时任务 tab | 立即执行 | 全部运行 tab | 概览 tab | 手动发起入口 |
|---|---|---|---|---|---|
| 妖股周核查 `mover-recall` | 投资组（P1 接线） | 后端+前端双白名单 | cron 源（record_run 自动） | 上次核查摘要/漏检数（可选） | 任务中心火箭按钮 |
| 进化计划聚合 `evolution-proposal` | 投资组（P1 接线） | 同上 | cron 源（record_run 自动） | **待审批计数+入口（P4）** | 任务中心火箭按钮 |
| 漏检补分析批次 | 不单独建任务（可由核查任务内联触发或挂周频任务） | — | timeTravel 源复用或注册新源（P2） | 进行中批次进度 | **任务中心发起批处理**（P2） |
| 漏检反思 | 复用既有 18:00 反思任务（source 分桶，无需新 cron） | 既有 | cron 源 | — | 既有 ReflectionPanel |
| 进化审批动作 | — | — | — | 待办卡片+审批抽屉（P4） | 概览 / EvolutionDriftPanel 双入口 |

前置事实（实现时照此对齐，勿另立机制）：
- 定时任务列表是 `list_scheduled_tasks` 全量返回，**新种子任务不经任何前端注册即自动出现**；
  要修的是分组归属与按钮可用性两张白名单（前端 `taskGrouping.ts`，后端 `scheduled_task.rs:113`）。
- 内置任务在 UI 上不可编辑参数（`isBuiltinTask` 判定，仅自定义/工作流任务显示编辑按钮），
  可启停/删/立即执行——新任务沿用此约束，参数调整走种子配置版本化而非编辑弹窗。
- 删除内置任务后重启不会自动重建（幂等语义为「存在即跳过」而非「缺失即补建」），
  删除确认文案已有 builtinWarning，无需新增。

## 6. 数据模型变更（均需 migration，禁区 10）

1. **新表 `mover_recall_runs`**：run_id、run_at、覆盖区间 from/to、rules_json、rates_json、
   layers_json、事件总数、漏检数、状态、created_at。
2. **新表 `mover_recall_events`**：事件 id、run_id、stock_code、period、anchor_date、
   window_start/end、cum_gain_pct、miss_layer、picked_any_period、scored_out_styles_json、
   补分析 analysis_id（可空）、反思 id（可空）、处理状态。
3. **`stock_analyses` 加列**：`source`（默认 `live`）、`synthetic`（默认 0）、
   `anchor_event_id`（可空）。
4. **`stock_reflections` 加列**：`source`（默认 `due`）。
5. **`stock_evolution_history` 加列**：`status` 已有（success/failed），扩展取值
   `pending_approval/accepted/rejected`；加 `trigger_sample_json`、`reviewer`、`reviewed_at`、
   `reject_reason`、`rollback_ref`。

## 7. 验收标准

- **P1**：重启后 `cron_jobs` 出现 2 条新任务且幂等（再重启不重复）；周六核查自动落
  `mover_recall_runs`；20:00 进化建议进待审批；非交易日 skip 留痕；**任务中心定时任务 tab
  两条新任务均归「投资」组、火箭按钮可点、运行记录出现在「全部运行」cron 源、新建菜单无这两项**。
- **P2**：对一组合成漏检事件，补分析报告的 as-of 取数审计通过（无未来数据）；
  `exclude_synthetic=true` 的决策回测样本数不含补分析行；`false` 时显式分桶可见；
  **补分析批次在任务中心「全部运行」可见进度与逐票明细，可从任务中心手动发起**。
- **P3**：漏检反思行在 `stock_reflections` 按 source=mover_recall 可查；
  教训校验拒绝样例不生效；前端漏检行可展开报告。
- **P4**：① 仓内不存在未接线的装饰器死代码；② 数据可达性失败样本不产生结构变异计划；
  ③ 未审批的进化不回写任何参数；接受/拒绝全链路有记录；④ 接受后下一周期荐股/分析读到新参数；
  ⑤ 可一键回滚；⑥ **任务中心概览显示待审批计数并可完成接受/拒绝操作**。
- 全量门禁（所有阶段完成后一次）：`cargo fmt --all --check`、
  `cargo clippy --workspace --all-targets --all-features -- -D warnings`、
  `__TAURI_WORKSPACE__=true cargo test --workspace`、`npm run typecheck`、`npm run format`。

## 8. 风险与边界

- **as-of 重放的数据穿越**：补分析只能用窗口起点前可得数据；每个分析节点取数需带 as-of 审计，
  无法保证时点的维度（如事后修复的概念分类）显式标注或跳过。
- **样本污染**：synthetic 标记必须贯穿 reco/回测/命中率统计全链路，漏标一处闭环分母即失真。
- **Unexplained 不驱动优化**：留痕不足的漏检只报告不进化（`mover_recall.rs:15` 既定边界）。
- **进化节奏**：审批闸门是硬约束；自动计划频率每日至多一次，参数类变更带冷却期，
  避免连续小样本抖动（weight_decay 已有衰减机制可复用）。
- **增量原则**：全部新能力以新 task_type / 新命令 / 加列方式接入，不改五条既有 cron 的语义，
  手动入口全部保留。
