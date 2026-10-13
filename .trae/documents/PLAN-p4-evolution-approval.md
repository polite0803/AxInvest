# P4 实施计划：受控自动优化（计划-审批两段式）

> 上游：`docs/plans/PLAN-batch-automation-mover-recall.md` §P4（L182-199）、验收 L245-247。
> P1/P2/P3 已全绿。本阶段完成后跑一次全量三命令门禁。

## Context（背景与目标）

P1 已让 20:00 cron 聚合三类样本并落 `stock_evolution_history` 的 `pending_approval` 行，但：
1. 计划行**没有审批出口**（无接受/拒绝命令，前端无入口），参数永不回写；
2. `axinvest_decorators.rs` 是从未装配的死代码（321 行）；
3. 样本聚合不区分"数据可达性失败"与"业务质量失败"，未来开放结构变异会被脏样本驱动；
4. 进化产物 `best_config`（WeightDecayConfig）**全链路无持久化落点**——消费侧
   （`evolution_drift.rs:229/285`、`weight_decay.rs:172`）全部硬编码 `WeightDecayConfig::default()`，
   进程内存态（`stock_adaptive_engine.current_config`）重启即失；
5. P1 聚合有实证 bug：反思计数过滤 `status="resolved"`，但反思收尾实际写 `"completed"`
   （`commands/stock_workflow/reflection.rs` 状态机 pending→running→completed/failed: …），
   该计数**永远为 0**。

P4 目标：自动优化闭环变为「晚上自动出计划（不碰任何生产配置）→ 人工审批 → 接受才执行并回写
→ 可一键回滚」，且数据可达性失败样本被硬规则隔离。

### 关键勘察结论（已核实）

- **审批通道裁定（偏离 plan 字面，实施层修正）**：`AppState.evolution_consent_senders`
  （`app_state.rs:277`，类型 `Arc<tokio::Mutex<HashMap<String, oneshot::Sender<bool>>>>`，
  消费方 `commands/evolution_hook.rs` + `commands/cognitive.rs:2520`）是**对话内实时**同意通道：
  内存态、emit `evolution-consent-request`、进程重启即失。P4 审批是跨天离线待办，**不复用**
  该通道；审批段走 DB 状态机 + 新 tauri 命令。consent 通道保留给实时弹窗场景。
- 当前生效策略权重 = `strategy_weight_history` 表中每 `(strategy_id, period)` 的最新行
  （`evolution_drift.rs:96-128` `load_current_weights`，`or_insert` 取最新），无独立 current 表，
  无回滚机制。
- 数据失败的现成机械判据：反思行 `mover_label` ∈ `no_market_data` / `rule_unavailable`
  （实体注释 `stock_reflections.rs:52-58`）；`mover_recall.rs:125-129` gain 取不到返回
  `Some("no_market_data")`；`decision_validations` 有 `data_source`、`t_plus_n_price`、
  `final_return_pct`、`hit_outcome` 字段（行情缺失时收益/ outcome 不可算）。
- 实体加列走 `crates/dao/src/reconcile/apply.rs` bootstrap additive 自动 ADD COLUMN
  （P2/P3 同模式，不写 migration）。
- 进化相关现存命令全在 `commands/stock_analysis.rs:6177-6334`
  （get_evolution_drift_dashboard/timeline、get_evolution_history:6211、
  trigger_stock_evolution:6237、manual_recalc_strategy_weights:6296）。
- 前端：`EvolutionDriftPanel.tsx`（731 行）已有 dashboard/history 数据加载与三个操作按钮；
  store actions 在 `stores/feature/stockAnalysisStore.ts:1859-1939`；
  `OverviewTab.tsx` 是 runs-store 驱动的四张健康卡 + 三区块；
  抽屉先例 `pages/TasksCenterPage/jobs/*Drawer.tsx`，`RunDetailDrawer.tsx` 按 source 分发。
- 引擎 `crates/analysis-engine/src/stock_self_evolution.rs`：
  `run_evolution(plan)`（L556）执行进化并 `persist_evolution`（L227，无条件 insert success/failed 行）；
  审批路径必须 UPDATE 原 pending 行而非再 insert。

---

## 实施步骤

### P4-1 删除死代码装饰器

- 删除 `src-tauri/src/init/axinvest_decorators.rs`。
- 删除 `src-tauri/src/init/mod.rs:5` 的 `pub mod axinvest_decorators;`。
- 已 grep 确认：`AxInvestReflectorDecorator` / `AxInvestEvolverDecorator` /
  `axinvest_untrusted_nodes` 全仓仅该文件自引用；`__untrusted` 主链路
  （agent_executor 注入 + rhai 消费 + seed 映射）与装饰器无关，不受影响。

### P4-2 数据模型：实体加列（bootstrap additive）

`crates/entities/src/stock_evolution_history.rs` 的 Model 新增 5 列：

| 列 | 类型 | 语义 |
|---|---|---|
| `trigger_sample_json` | `Option<String>` | 触发样本：`{reflectionIds:[], validationIds:[], moverRunIds:[], dataBlocked:{reflectionIds:[], moverRunIds:[]}, counts:{reflections,validations,moverMisses,dataBlocked}}` |
| `reviewer` | `Option<String>` | 审批人（命令入参，桌面端缺省 `"local-user"`） |
| `reviewed_at` | `Option<i64>` | 审批时间 ms |
| `reject_reason` | `Option<String>` | 拒绝原因（拒绝时必填）；回滚时写 `"rolled back to evo-xxx"` |
| `rollback_ref` | `Option<String>` | 接受时盖的「上一条 accepted 行 id」（回滚目标链） |

- 实体 `status` 注释更新为：`pending_approval | accepted | rejected`
  （历史 success/failed 行保留不迁移，读侧按新三值过滤）。
- 在 `crates/dao/src/reconcile/apply.rs` 按 P2/P3 先例登记 ADD COLUMN
  （**实施首步先读该文件确认 Option 列/空串列先例写法**，与 `stock_reflections.source`
  加列形态对齐；行尾 LF）。

### P4-3 失败原因分流硬规则（proposal 聚合重写）

文件：`src-tauri/src/commands/evolution_proposal.rs`。

1. **修 status bug**：`"resolved"` → `"completed"`（pending/running/`failed: …` 不计入）。
2. 计数改为**拉行分类**（不再纯 `.count()`），新增集中判定函数：

   ```rust
   /// 数据可达性失败硬规则：这类样本只反映"拿不到数据"，永不驱动任何进化（含未来结构变异）。
   fn is_data_reachability_blocked(
       reflection: Option<&stock_reflections::Model>,
       validation: Option<&decision_validations::Model>,
       mover_miss_reason: Option<&str>,
   ) -> bool
   ```

   机械判据（满足任一即 blocked）：
   - 反思：`mover_label == "no_market_data" | "rule_unavailable"`；
     `blackboard_snapshot` JSON 内含 `__untrusted`/`axinvest_untrusted_nodes` 非空标记
     （**实施时先读一行真实 blackboard_snapshot 确认键路径**，键不存在则仅用 mover_label）；
   - 验证：`final_return_pct.is_none()` 或 `t_plus_n_price` 空（行情不可得，无法形成业务结论）；
   - 漏检：`misses_json` 内 miss 原因为 `no_market_data` 的条目
     （**实施时读 `mover_recall.rs` misses_json 序列化结构确认字段名**）。
3. `ProposalSampleCounts` 增 `data_blocked: u64`；`total()` 只加业务类
   （data_blocked **不**进 total）。只有业务类 total>0 才产计划；全是 blocked 时
   skip message 明示「窗口内 N 条数据可达性失败，仅入数据质量看板，不触发进化」。
4. 落计划行时写 `trigger_sample_json`（业务样本 id 全量 + blocked 样本单独分桶 + 计数）。
5. 计划类型闸口保持 `ParameterEvolution` 并在聚合处加注释硬规则：
   「开放 WorkEvolution 前必须仅使用业务类样本；blocked 样本永远不进结构变异」。
   用单测锁死：blocked 样本存在时 total 不含它、不产计划。
6. 单元测试（tests mod 追加文件末尾）：config 既有 4 测保留；新增纯函数测试
   `is_data_reachability_blocked` 各分支、`ProposalSampleCounts::total` 排除 blocked。
   DB 集成聚合测试若 `create_test_pool` 成本高则只测分类纯函数 + JSON 序列化。

### P4-4 引擎审批执行路径 + 生效配置读取 + 回滚

**引擎侧**（`crates/analysis-engine/src/stock_self_evolution.rs`）：

- 新增 `pub async fn execute_pending_plan(&self, plan: EvolutionPlan) -> Result<StockEvolutionResult>`：
  复用 `run_evolution` 的执行体但**不 insert 新 history 行**（抽内部
  `run_evolution_inner(plan) -> Result<StockEvolutionResult>`，`run_evolution` 改为
  inner + `persist_evolution`；新方法只调 inner）。最小改动、不重写现有逻辑。
- 新增配置读取（同引擎或 `evolution_drift.rs`）：

  ```rust
  /// 生效的权重衰减配置 = 最近一条 accepted 计划行 parameter_result_json.best_config；
  /// 无 accepted 行 / 反序列化失败 → default（与历史行为一致）。
  pub async fn load_active_weight_decay_config(db) -> WeightDecayConfig
  ```

  替换三处硬编码 default：`evolution_drift.rs:229`、`evolution_drift.rs:285`、
  `weight_decay.rs:172`（每处替换后确认函数签名能拿 db；weight_decay.rs 那处
  **实施时先读上下文**，若拿不到 db 则保留 default 并加注释说明）。

**命令侧**（新文件 `src-tauri/src/commands/stock_evolution_approval.rs`，
`commands/mod.rs` 加 `pub mod`；双属性 `#[tauri::command]` + `#[agent_command]`，
错误 `ErrorResponse::new(wf_err::INTERNAL).with_detail(...)`，DTO serde camelCase）：

1. `list_pending_evolution_plans(state) -> Vec<stock_evolution_history::Model>`
   —— status=pending_approval，created_at 升序（先到先审）。
2. `approve_evolution_plan(state, history_id: String, reviewer: Option<String>) -> Model`
   - 取行并校验 status=pending_approval（并发幂等：已审批则报错）；
   - 反序列化 trigger 重建 `EvolutionPlan`（plan_id 用行内值，evolution_type 从行映射，
     反序列化失败报 INTERNAL）；
   - 调 `execute_pending_plan`；失败则 UPDATE 行 status=rejected、reject_reason 写执行错误
     （不留 pending 僵尸行）并返回错误；
   - 成功：UPDATE 原行 → status=accepted、parameter_result_json=序列化 result.parameter_result
     （含 best_config/default_config/improvement/stats，即 diff 与回测概要载体）、
     reviewer、reviewed_at、rollback_ref=此前最近 accepted 行 id（无则 None）；
   - 返回更新后 Model。
3. `reject_evolution_plan(state, history_id, reason: String, reviewer: Option<String>) -> Model`
   - reason 非空校验（空白报参数错误）；UPDATE status=rejected + reviewer/reviewed_at/reject_reason；
   - **反例回灌**：实施时先读 `reflection_lessons`（P3 有 validation_rejected 状态先例）
     与引擎反馈钩子签名；若有现成负反馈记录入口则写一条（来源标 evolution_reject + history_id），
     否则仅在 history 行留痕（不新造机制），代码注释标注后续挂接点。
4. `rollback_evolution(state, history_id: Option<String>) -> Model`
   - 缺省 = 当前生效版本（最近 accepted 行）；目标 = 该行 rollback_ref；
     rollback_ref 为空则报错（无历史版本可回滚）；
   - 复制目标行的 parameter_result_json 新插一条 accepted 行（id `evo-rollback-{uuid}`、
     summary「回滚到 {目标 id}」、rollback_ref 沿用目标行的 rollback_ref 保持链条、
     trigger 复用目标行 trigger）；当前行 UPDATE status=rejected、
     reject_reason=`rolled back to {新行id}`；返回新行。
   - load_active 取"最近 accepted"逻辑不变，回滚后自然读到旧配置；全部留痕可审计。

### P4-5 前端：双入口审批 UI

**类型/store**：
- `src/types/evolution.ts` 增 `EvolutionHistoryRow`（camelCase 对齐实体）、
  `EvolutionTriggerSamples`（triggerSampleJson 解析类型）；从 `@/types` barrel 导出。
- `stores/feature/stockAnalysisStore.ts` 仿现有 fetchEvolutionHistory 增
  `pendingEvolutionPlans` state + 4 actions：
  fetchPendingEvolutionPlans / approveEvolutionPlan / rejectEvolutionPlan / rollbackEvolution
  （invoke 走 `@/lib/invoke`，参数 camelCase）。

**共享抽屉** `src/components/stock-analysis/EvolutionApprovalDrawer.tsx`（命名导出）：
- props：`open / plan: EvolutionHistoryRow | null / onClose / onChanged`；
- 内容：触发摘要 improvement_summary、三类样本计数 + 样本 id 折叠展示
  （triggerSampleJson，blocked 样本单独红色 Tag「数据可达性」）、
  接受后展示 diff（best_config vs default_config 三参数 ewmaAlpha/lookbackDays/sampleSaturation
  前后值）+ improvement_summary 回测概要；
- 操作：接受（Popconfirm 二次确认）/ 拒绝（必填原因 textarea）/ 回滚（仅 accepted 行，
  Popconfirm）；完成后 onChanged 触发重新拉取。
- 浮层 children 若自定义组件透传 ref（禁区 12）。

**EvolutionDriftPanel.tsx**：在操作区下方新增「待审批进化计划」Card
（Badge 计数 + 列表，点击行打开 EvolutionApprovalDrawer）；useEffect 合并拉 pending；
accepted 历史行的「查看」也打开同一抽屉（只读 diff + 回滚按钮）。

**任务中心**：
- `OverviewTab.tsx` 健康卡行新增一张「待审批进化」卡（独立本地 state +
  list_pending_evolution_plans，不走 runs store，避免改动 UnifiedRun/source 白名单）：
  显示计数，点击展开本页内嵌 Drawer（复用 EvolutionApprovalDrawer，列表态在卡/Popover 内选择）；
  交互形态：卡片点击切换一个受控 state，在 OverviewTab 底部挂载抽屉 + 内部待审批小列表，
  选中行进入计划详情。**实施时若发现任务中心有全局抽屉容器则挂载到该容器**。
- browser/localStorage mock：实施时定位现有 evolution mock（grep
  `get_evolution_history` 的 mock 注册点）补 4 个新命令 mock。

**i18n（11 语言，强制）**：
- 新增 key 挂 `stockAnalysis.evolutionApproval.*`（标题/样本/接受/拒绝/回滚/数据可达性 Tag/
  成功失败提示等）+ `tasksCenter.health.pendingEvolution`；
- Python 脚本注入（仿 `.workbuddy/tmp/add_p3_review_i18n.py`）：
  `.workbuddy/tmp/add_p4_approval_i18n.py`，`json.dumps(ensure_ascii=False, indent=2)`，
  LF 落盘，11 文件：zh-CN（源）/zh-TW/en-US/ja/ko/de/fr/es/ru/ar/hi。

**测试**：
- EvolutionApprovalDrawer 组件测试（接受/拒绝校验/blocked Tag 渲染/diff 展示）；
- OverviewTab 待审批卡测试（计数、点击）；
- mock 形态照 P3 教训：`vi.mock("@/lib/invoke", ...)`、I18nextProvider 包裹、
  混合文本用容器 textContent 断言、act 内 await setTimeout。

### P4-6 合并验证（全部改完后一次性跑）

Rust（src-tauri 下 PowerShell，改代码阶段不编译）：

```powershell
cargo check
$env:__TAURI_WORKSPACE__='true'; cargo clippy -p axagent -p axagent-entities -p axagent-dao -p axagent-analysis-engine --all-targets -- -D warnings
$env:__TAURI_WORKSPACE__='true'; cargo test -p axagent --lib -- evolution
$env:__TAURI_WORKSPACE__='true'; cargo test -p axagent-analysis-engine --lib
cargo fmt -p axagent -p axagent-entities -p axagent-dao -p axagent-analysis-engine
```

前端（仓库根）：`npm run typecheck`；定点 `npx vitest run` EvolutionDrift/Overview/
EvolutionApproval 相关；`npx dprint fmt --incremental=false <改动文件>`；oxlint；
i18n 三校验：`node scripts/check-i18n-key-parity.mjs`、
`node scripts/check-i18n-placeholder-usage.mjs`、`python scripts/check_i18n.py --strict`。

**P1-P4 全部完成后**（由用户触发时点）跑全量门禁：
`cargo fmt --all --check` /
`cargo clippy --workspace --all-targets --all-features -- -D warnings` /
`$env:__TAURI_WORKSPACE__='true'; cargo test --workspace`
+ 前端 `npm run typecheck`、`npm run format`。

---

## 不做什么（防范围漂移）

- 不开放 WorkflowEvolution（节点/边结构变异）——P4 只立分流硬规则与闸口注释。
- 不新建数据质量看板页面——blocked 样本在审批抽屉与 cron 输出里可见即满足验收；
  不新建 KV 配置表（accepted history 行即配置来源）。
- 不改 cron 投递/种子逻辑（P1 已完成）；不动 `evolution_consent_senders` 实时通道。
- 不迁移历史 success/failed 行；不碰 P1-P3 在途无关文件。

## 端到端验证路径

1. `__TAURI_WORKSPACE__=true cargo test` 定点全绿（聚合分类、审批状态机、回滚链）。
2. `npm run tauri dev` 手测：SQL/命令构造一条 pending 行（或等 cron/手动调聚合 inner）
   → EvolutionDriftPanel 出现待审批卡 → 接受后行变 accepted、diff 可见、
   触发 `manual_recalc_strategy_weights` 后权重计算读到新 lookbackDays →
   回滚后再重算恢复旧配置；拒绝必须填原因，拒绝后行留 rejected。
3. 任务中心概览出现待审批计数，点击同一抽屉可完成审批。
