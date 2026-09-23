# PLAN — 股票进化闭环批1（闭合进化：真实 evolver + 真DB参数进化 + 产物落库）

## Context（为什么做）

批1目标：把股票“自我进化闭环”从“部分走模拟 / 产物不生效”修复为“真实进化 → 产物落库可回看”。

前期勘探纠正了一个旧假设：进化其实**已经接了主执行链**——`commands/stock_workflow/core.rs:800` clone `state.stock_adaptive_engine`，`:1033` 在降级/部分失败分支 spawn `trigger_adaptive_cycle`；参数进化 `evolution_optimizer.rs::run_evolution` 也已接真实 DB（`strategy_performance` 复盘胜率做 fitness），策略复盘权重（`weight_decay.rs` / `evolution_drift.rs`）也已落地并有表。

**真正确凿的两处“虚进化/不生效”**：
1. `StockSelfEvolutionEngine.workflow_evolver` 默认 `None`（`stock_self_evolution.rs:178`），流程进化落到 `simulate_workflow_evolution` 占位（`fitness` 硬编码 0.5→0.65、`validation.passed` 硬编码 true、`nodes` 空）——**假进化**。
2. 参数进化存在分叉：`StockSelfEvolutionEngine::evolve_parameters` 走**启发式** fitness（质量分均值×参数合理性，`#279-303`、`#799-831`），而 `evolution_optimizer::run_evolution` 已写真实 DB 的 `weight*win_rate*confidence`（`#82-130`、`#133-190`）——两条线未统一。
3. 产物只在内存（`results_cache` / `adaptation_history`），跨会话丢失——不可回看。

用户本次选定范围：**A 接真实流程进化器 + B 统一参数进化到真DB一条线 + C 产物落库可回看**；真实回测档位 = **现有复盘胜率做 fitness（不接 quant K线回测）**。

约束：AGENTS.md 铁律（禁改命名体系、DB 表结构变更走受控 schema 路径、命令自动注册、camelCase DTO 同步、三条 clippy/fmt/test 门禁）。少造新组件——优先复用既有机制。

## 复用清单（不要重造）

- `axagent_trajectory::WorkflowEvolverImpl::with_defaults()` — 真实流程进化器，`src/init/state.rs:665` 已有构造先例；analysis-engine 已依赖 axagent-trajectory，`StockSelfEvolutionEngine` 已 `use` 之（`stock_self_evolution.rs:30`），**无循环依赖**。
- `analysis-engine::evolution_optimizer::{run_evolution, make_fitness_fn, param_defs}`（`evolution_optimizer.rs`）— 真DB 参数进化 + fitness。
- `analysis-engine::evolution_drift::{load_current_weights, load_performance_window}` — 读策略表现。
- DB 表模板：`crates/entities/src/strategy_weight_history.rs`（既有“复盘→进化留痕”表，new entity 照此形态）。
- Tauri 命令模板：`commands/stock_analysis.rs::get_evolution_drift_dashboard`（约 `:5592`，用 `state.harness.db()`）。

## A — 接真实流程进化器

文件：`src-tauri/crates/analysis-engine/src/stock_self_evolution.rs`
- `new()`：把 `:178` 的 `workflow_evolver: None` 改为 `Some(WorkflowEvolverImpl::with_defaults())`（让流程进化不再走 simulate；`with_workflow_evolver` 保留供覆盖）。
- 新增字段 `db: Option<DatabaseConnection>`（A/B 共用）+ `pub fn with_db(mut self, db: DatabaseConnection) -> Self`。`new()` 签名不变 ⇒ 既有测试（`make_test_engine`）不破。
- 新增 `pub async fn persist_workflow_result(&self, m: &WorkflowModification)`：把 `m.evolved`（`WorkflowGenome`）序列化为 JSON，写入 C 的新表 `stock_evolution_history`（结构落盘 = 闭环下一步）。
- 在 `run_evolution`（`:452-546`）的 `WorkflowEvolution` / `HybridEvolution` 分支，`evolve_workflow` 成功后调用 `persist_workflow_result`。

本次不做 `set_genome_loader` 注入（`initialize` 仍以默认拓扑起步）——属增量，留批2，避免范围膨胀。

## B — 统一参数进化到真 DB

- `stock_self_evolution.rs::evolve_parameters`（`:279-303`）：`self.db.is_some()` 时改为调 `crate::evolution_optimizer::run_evolution(db, None)`（Live 模式）；`db.is_none()` 保留旧启发式分支兜底（保单测）。
- `evolution_optimizer.rs::run_evolution` 可能 `Err("无表现数据")`（`:140-142`，`strategy_performance` 无数据时）：调用方捕获后置 `success=false` 降级，勿让 `run_adaptive_cycle` 崩溃。（回测档位=复盘胜率，不接 quant。）
- `stock_adaptive_engine.rs`：新增 `pub fn with_db(mut self, db: DatabaseConnection) -> Self`，内部 `self.evolution_engine = <engine>.with_db(db.clone()).with_workflow_evolver(WorkflowEvolverImpl::with_defaults()).into()` —— 一次接线 A+B。
- `src/init/state.rs:1584-1589`：构造 `StockAdaptiveEngine::new()` 改为 `StockAdaptiveEngine::with_db(sea_db)`（`sea_db` 该处已存在）。
- `run_adaptive_cycle`（`:288-407`）主体不变；`core.rs` spawn 链无需改。

## C — 产物落库 + 可回看（后端先行）

- 新建 entity `src-tauri/crates/entities/src/stock_evolution_history.rs`（形态照抄 `strategy_weight_history.rs`）：
  表 `stock_evolution_history`，字段：`id`(String, PK, auto=false)、`plan_id`、`trigger`(进化触发原因 JSON)、`evolution_type`(parameter/workflow/hybrid)、`stock_code`、`quality_before`(u8)、`parameter_result_json`(Option)、`workflow_result_json`(Option，含 genome)、`status`(success/failed)、`improvement_summary`、`created_at`(i64)，`#[serde(rename_all="camelCase")]`。
- `entities/src/lib.rs`：在股票段（`strategy_weight_history` 附近）加 `pub mod stock_evolution_history;`。
- **表结构落地走项目既有声明式 schema 同步**：执行前先读 `src-tauri/crates/dao/src/migrations/schema_diff.rs` 确认新增 entity 如何被检测/建表（本项目 entity 即真相 + dao reconcile/schema_diff；按 `strategy_weight_history` 落地方式一致执行）。切勿绕过既有建表路径。
- 写入点：A 的 `persist_workflow_result` + `run_evolution` 结束统一 insert 该表（含 B 的 `EvolutionResult`）。
- Tauri 命令 `get_evolution_history(State, limit: Option<usize>, stock_code: Option<String>) -> Result<Vec<..., String>>`：放 `commands/stock_analysis.rs`，仿 `get_evolution_drift_dashboard`。命令注册由 `build.rs` 依 `mod.rs` + `#[tauri::command]` 自动生成，勿手改清单。
- 前端：本期只通后端（命令 + `@/types` camelCase 类型补一个最小消费即可，方便用 `@/lib/invoke` 拉通）；完整“可回看面板”UI 不在本期，避免范围膨胀。

## 关键文件

| 文件 | 动作 |
|---|---|
| `src-tauri/crates/analysis-engine/src/stock_self_evolution.rs` | A+B 主改：接 evolver、`with_db`、`persist_workflow_result`、`evolve_parameters` 走真DB |
| `src-tauri/crates/analysis-engine/src/stock_adaptive_engine.rs` | 加 `with_db` 接线（A+B） |
| `src-tauri/src/init/state.rs` | `StockAdaptiveEngine::with_db(sea_db)` |
| `src-tauri/crates/entities/src/stock_evolution_history.rs` | 新建 entity（C） |
| `src-tauri/crates/entities/src/lib.rs` | 加 `pub mod` |
| `src-tauri/src/commands/stock_analysis.rs` | 加 `get_evolution_history` 命令 |

## 验证（三条门禁必须过，与 CI 口径一致）

```
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
__TAURI_WORKSPACE__=true cargo test --workspace
```
- 新增单元测试：
  - `StockSelfEvolutionEngine::new()` 后 `workflow_evolver.is_some()`（不再 simulate）。
  - `with_db` 后 `evolve_parameters` 走到真DB 分支（db.is_some）；`db.is_none()` 仍走旧启发式且成功。
  - `persist_workflow_result` 将 `WorkflowModification` 写入 `stock_evolution_history` 并回读一致。
  - `run_evolution` 当 `strategy_performance` 为空时返回 `success=false` 而非崩溃。
- `cargo run`（或已有命令）触发一次股票分析，验证进化历史能经 `get_evolution_history` 读出。

## 不做（本期边界）
- 不接 quant K线回测（用户选定复盘胜率档位）。
- 不注入 `genome_loader` / 不改造 `simulate_workflow_evolution`（占位保留为 `db=None` 兜底）。
- 不做前端“可回看”UI 面板（仅后端命令 + 最小消费类型）。