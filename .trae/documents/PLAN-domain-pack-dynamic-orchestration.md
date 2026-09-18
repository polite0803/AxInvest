# 域包管理动态化：OPC 14 域包接入 DomainPackAdapter 动态编排

## Context（为什么做）

用户拍板放弃「域包 *yaml 模板化」（阶段 B/C 作废）。目标改为：**在现有的工作流动态编排基础上实现域包管理**。

当前事实：
- 动态编排底座**已存在**：`axagent-harness` 的 `domain_pack_orchestration` 提供 `DomainPackAdapter` trait、`DomainPackAdapterRegistry`、`DynamicSubGraph::generate()`、`OrchestrationStrategy`（Ordered/FanOut/Pipeline/Race/Debate/Dynamic）。股票是第一个实现者（`StockDomainPackAdapter`）。
- **14 个 OPC 域包没接这套**：`opc_execute_workflow`（`src/commands/opc_domain_pack_actions.rs:2515`）走固定模板 seed + rt-workflow，注释明确「不再运行时动态生成 DAG」。`domain_pack_config.rs` 的 14 个 `xxx_config()` 的 `workflow_steps` 全为空。
- 接线缺口：动态子图 `GeneratedSubGraph::to_workflow()` → `SubGraph`，但**没有**「SubGraph → 落库 workflow_templates → rt-workflow 执行」的现成桥（股票链路 `stock_adaptive_engine.rs` 仅仿真遍历，未真正跑 rt-workflow）。域包这条将是第一个真正接通的链路。

用户拍板方向：
1. **每域包手写一个 Rust `DomainPackAdapter`**（基准照抄 `StockDomainPackAdapter`）。
2. **步骤/工具硬编码进 Rust**（不读 runtime.yaml 的 workflow_steps 段）。
3. **mission 驱动 + preset 兜底**：`decompose_mission` 按任务意图动态选策略生成 DAG；无明确 mission 回退到 adapter 内硬编码默认流程。
4. **本次范围：后端能力 + 执行链路**；前端管理页下一阶段。

## 复用点（不要重复造）

| 能力 | 位置 | 用途 |
|---|---|---|
| `DomainPackAdapter` trait / `DomainPackAdapterRegistry` / `DomainPackAdapterRegistry::get` | `crates/harness/src/domain_pack_orchestration/mod.rs`（经 `axagent_orchestrator` re-export 亦可用） | 每域包实现的接口 + 注册 |
| `DynamicSubGraph::generate(&decomposition_plan)` | `crates/harness/.../subgraph.rs:57` | 从 `SubTask` 构建动态 DAG 子图（含环/孤立校验） |
| `DecompositionPlan` / `SubTask` / `OrchestrationStrategy` / `MissionType` | `crates/harness/.../plan.rs`、`mod.rs` | 分解计划载体 |
| `GeneratedSubGraph::to_workflow() -> SubGraph` | `crates/harness/.../subgraph.rs:38` | 子图 → `SubGraph` |
| `WorkflowTemplateData.sub_graph: Option<SubGraph>` | `crates/harness/src/workflow_types.rs:1691`、`:753` | 子图可直接塞进模板的 `sub_graph` 字段 |
| `upsert_template` | `src/commands/opc_workflows/`（现用于 `opc_domain_pack_actions.rs:2610`） | 模板落库 |
| `run_template_via_engine` | `src/commands/opc_domain_pack_actions.rs:2326` | DB 模板 → rt-workflow 执行 |
| 股票 adapter 全骨架 | `crates/analysis-engine/src/stock_orchestration.rs` | 14 个 adapter 的基准模板 |
| 域包注册链 | `src/init/state.rs:1118` 创建 registry；`register_stock_adapter`（stock_orchestration.rs:444） | 新命令注册 |

## 实现步骤

### 1. 统一骨架（`analysis-engine`，implementor 层，可依赖 harness+entities）
新建 `src-tauri/crates/analysis-engine/src/opc/opc_adapter/skeleton.rs`：
- `pub(crate) fn build_plan(mission: &str, strategy: OrchestrationStrategy, sub_tasks: Vec<SubTask>) -> Result<GeneratedSubGraph, OrchestrationError>`：构造 `DecompositionPlan`（`max_parallel`/`max_replans` 传参）→ `DynamicSubGraph::new().generate(&plan)`。把股票 `build_analysis_pipeline`/`build_debate_strategy` 里重复的 plan+generate 逻辑收敛至此。
- 薄封装默认 `reflection_template` / `evolution_constraints` / `acceptance_criteria`（用 `..Default::default()` 降样板），14 个 adapter 按域业务覆写关键字段。
- `pub(crate) fn to_workflow_template_data(subgraph: &GeneratedSubGraph, domain_pack_id: &str, name: &str) -> WorkflowTemplateData`：`to_workflow()` 得到的 `SubGraph` 塞入 `WorkflowTemplateData { sub_graph: Some(...) }`，`id` 用 `subgraph.id`（天然唯一），填 `trigger_config`(Manual)、`tags=[domain_pack_id,"opc"]`、`variables` 空、`created_at/updated_at`。**不放 harness**（keep foundation 零依赖）。

### 2. 14 个域包 adapter（每域一文件，步骤硬编码）
新建 `src-tauri/crates/analysis-engine/src/opc/opc_adapter/{accounting,ai_research,consulting,content_media,design,ecommerce,education,finance_invest,game_dev,geospatial,project_management,sales_growth,security,software_dev}_adapter.rs`，每份实现 `DomainPackAdapter`：
- `new()`：填 `domain_pack_id`（下划线）、`domain_pack_name`、硬编码的 `reflection_template`/`evolution_constraints`/`acceptance_criteria`/`learning_config`。
- `select_strategy(mission)`：关键词路由到 `Debate`/`Pipeline`/`FanOut`/`Ordered`。
- `decompose_mission`：构造硬编码 `SubTask` 列表（含 role、dependencies、tools）→ 按策略走 `build_plan`。
- `detect_mission_type`：域专属关键词 → `MissionType`。
- `preset_steps()`：返回硬编码默认流程（无 mission 兜底时用）。
- `create_xxx_adapter() -> Arc<dyn DomainPackAdapter>` + 测试（对齐 `stock_orchestration.rs:450`）。

> 步聚原料：各域 `config/opc/domain_packs/{id}/runtime.yaml` 的 `workflow_steps`（如 software_dev 的 18 步 SDLC + 18 步重构）转为硬编码 `SubTask`，避免从零设计业务流程。

### 3. 注册与启动接线
- `state.rs:1118` 处 registry 注册链：新增 `register_opc_domain_pack_adapters(registry)`（放 `analysis-engine`，仿 `register_stock_adapter`），静态把 14 个 `create_xxx_adapter()` 全注册（幂等稳定，不按存在动态裁剪）。在 `load_domain_pack_adapters`（`src/commands/opc_workflows/mod.rs` 附近）追加以保证启动注入。

### 4. 新执行命令 + 桥（wiring 层）
`src/commands/opc_domain_pack_actions.rs` 新增 `#[tauri::command]`：
- `opc_execute_mission(app_state, domain_pack_id, mission, user_input) -> Result<Value,String>`：
  1. `registry.get(domain_pack_id)`（注意连字符/下划线归一，与现有 `opc_execute_workflow` 一致）取 adapter；
  2. `adapter.decompose_mission(&mission, &DomainPackContext::default()).await`；
  3. `to_workflow_template_data(...)` 组模板；
  4. `upsert_template(db, data)` 落库（`id=subgraph.id`）；
  5. `run_template_via_engine(db, &engine, &domain_pack_id, &subgraph_id, days, user_input)` 执行。
- 命令参数由 `#[tauri::command]` 自动 camelCase（前端传 `domainPackId`），**勿手改 register_commands.rs**。
- **保留** `opc_execute_workflow` 固定模板路径作兜底（现 DAG 是「用户可编辑模板」权威，动态子图每次运行时生成，两者互补）。不替换、不破坏前端工作表编辑器。

### 5. 一致性约束（AGENTS.md 硬约束）
- `WorkflowTemplateData`/`SubTask` 用 harness 类型，禁自造节点类型（复用 `workflow_types` 标准节点）。
- 分层：adapter 放 `analysis-engine`（implementor，依赖 harness+entities）；新命令放 `src/commands`（wiring）。**不引 orchestrator/agent/runtime-core 等 consumer**。
- tools：command 层复用现有 `opc_tool_defs/stock_tool_defs/local_tool_defs` 组合（`opc_domain_pack_actions.rs:2595`）。

## 扩展性权衡（D）
用户方向（每域硬编码 adapter）与「新增维度不改核心代码」存在张力：新增分析维度需改对应域 `decompose_mission`/`SubTask` 源码。折中：保留 `adapter.learning_config()` 读取能力，作为未来「新维度经 learning/预设步骤注入」的扩展点；本次不做 LLM 动态拆解（harness `decompose_mission` 为纯规则，无法在适配器内凭空引入 LLM）。此矛盾在 plan 评审时向用户明示。

## 测试与验证
- 每域 adapter 单测（对齐 `stock_orchestration.rs:450`）：`adapter_has_correct_id`、`detects_<domain>_mission`、`selects_default_strategy`、`decomposes_into_pipeline`（nodes.len≥阈值）、`has_acceptance_criteria`、`has_evolution_constraints`；`skeleton` 加 `to_workflow_template_data_carries_subgraph`。
- 门禁（提交前缺一不可，`src-tauri/` 下）：
  - `cargo fmt --check`
  - `cargo clippy --workspace --all-targets -- -D warnings`
  - `__TAURI_WORKSPACE__=true cargo test --workspace`
- 手测：`npm run tauri dev`，前端调 `opc_execute_mission({domainPackId:"software_dev", mission:"分析一个大型项目重构任务"})`，观察 `workflow_templates` 新增子图模板 + 前端会话进度面板出现动态 DAG 步骤。

## 风险与边界
- runtime.yaml `workflow_steps` 段在手写 adapter 接入后不再被工作流编排消费（仅 preset 阶段被 base 读）。保持 `workflow.rs` 现有原模板生成不受影响（两套模板 ID：固定 `{id}_harness_workflow` vs 动态 `subgraph.id`，不互踩）。
- clippy 短路：修首 error 后必须全量重跑 clippy。
- `upsert_template` 路径已存在于 `opc_workflows`（见 `opc_execute_workflow:2610` 调用实例），直接复用签名，无需新造。