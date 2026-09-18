# PLAN：OPC 域包配置数据化（DomainPackConfig + 14 模板统一落 YAML）

> 本期主旨：**源从代码 → 文件，行为不变，分步可回退，收尾删硬编码。**
> 上一版被驳回后按「方向对，继续细化」重写。

## Context（为什么做）

OPC 域包静态配置处于**多份载体 + 半迁移**状态，存在同事实两份、能静默漂移的风险：

- `DomainPackConfig`（`domain_pack_config.rs`）14 个硬编码函数是 `automation_rules`/`entity_types`/`kpi_calculations`/`input_fields`/`requirements_approval`/`validations` 的**实际权威**。
- 但同一批数据**已迁入** `config/opc/domain_packs/{id}/runtime.yaml`（14 个域包全部含 `automation_rules`/`validations`/`kpi_sources`/`dashboard_cards`/`kpi_definitions` 段）——只因 schema 没接收（`RuntimeKpiConfig` 只解析 kpi/dashboard，其余段被 serde 静默忽略）而**无人消费**。
- 另有 **14 个 seed 工作流模板**（`seed_domain_pack_*.rs`，`WorkflowTemplateData` 节点/边图）纯硬编码，`config/opc/domain_packs/{id}/` 下**无** `workflows/*.yaml`。

**目标**：让 runtime.yaml 成为静态配置唯一权威、`DomainPackConfig` 由 YAML 组装；14 个模板落到 `workflows/*.yaml` 由 seed 读取。消费方行为零变化。

## 已确认的方向（用户拍板）

- 范围：**静态配置 + 14 模板都 YAML 化**；载体：**复用扩展 runtime.yaml**。
- 红线：**接入即消费**（不接受"段加进 schema 却无人读"）；消费方不受行为破坏。

## 现状关键事实（file:line，已核实）

- 工作流类型已全部 serde 可序列化：`WorkflowNode` 用 `#[serde(tag="type", rename_all="camelCase")]`（`harness/workflow_types.rs:1357-1360`），`WorkflowTemplateData`(:1690)/`EdgeType`(:1599)/`TriggerType`(:177) 已 `Serialize+Deserialize` ⇒ `serde_yaml::from_str::<WorkflowTemplateData>` 零补 derive。
- `AutomationCondition`(`automation.rs:110`)/`AutomationAction`(:130) 为 `#[serde(tag="type")]`，与 runtime.yaml 的 `type:` 对齐 ⇒ `automation_rules` 段可**直接**反序列化为 `DomainPackAutomationRule`。
- ⚠ **validations 段形状不同源**：runtime.yaml 里是 `{entity_type,field,operator,value,message}`（贴近 `domain_pack_validator`），**不是** `DomainPackConfig.validations`（`ValidationDef{field,type,error_message}`，只被 `workflow.rs:83→generate_domain_pack_template_data` 兜底生成消费）。两套校验概念，**不得误合并**。
- `get_config` 消费方（同步无参，command 层直调无 app_dir 上下文）：`opc_domain_pack_runtime.rs:263/302/313/346/407`、`domain_pack_kpi_service.rs:379`、`opc_domain_pack_actions.rs:2590`。
- ⚠ **不在本 scope**：`opc_domain_pack_actions.rs:1964 get_all_domain_pack_configs`（`DomainPackActionConfig`），勿动。`engine_config`(:2522) 是 `domain_pack_config` 别名，仅一套体系。

## 分阶段设计

### 阶段 A：runtime.yaml 段接入 → get_config 改从 YAML 组装（旧函数保留回退）

**A1 扩展 schema**（`analysis-engine/src/opc/domain_pack.rs` 的 `runtime_schema`）
- 给 `RuntimeKpiConfig` 追加（全部 `#[serde(default)]` 防缺段/旧文件崩）：
  - `automation_rules: Vec<DomainPackAutomationRule>`（复用现类型，直接对接）
  - `entity_types`、`kpi_calculations`、`input_fields`、`requires_approval: bool`、`workflow_steps`
  - `validations`：**新增独立段 struct**（`entity_type/field/operator/value/message`，含 serde default），**不复用** `ValidationDef`。
- 目标：这些"现已存在于 YAML 但被忽略"的段全部可被读取，**不再静默丢弃**。

**A2 get_config 改造**（`domain_pack_config.rs`）
- 签名改 `get_config(id, app_dir: Option<&Path>)` → `resolve_domain_packs_dir(app_dir)` + `load_domain_pack` 组装 `DomainPackConfig`（优先 runtime.yaml，缺失字段回退既有逻辑）。
- 消费方同步：`opc_domain_pack_runtime.rs:263/346` 已有 app_dir 直传；`:302/:313/:407` 加 `app_dir` 参数（命令层 :478/:555/:609 传 `Some(&app_state.app_data_dir)`）；`domain_pack_kpi_service.rs:379` 传 `None`（走仓库根回退）。
- `automation_rules` 经此**天然接入消费**（get_enabled_rules/run_automation_rules 都走 `get_config`），满足"接入即消费"红线。

**A3 validations 段消费（落实红线）**
- 因 validations 段形状 ≠ `ValidationDef`，接入后默认由 `domain_pack_validator` 改读该段（消灭 validator 硬编码 match）；执行时若 validator 改造波及面过大，先保"段被 schema 接收 + get_config 原样透出供兜底"，validator 改读作为同阶段独立小步完成，不放空。

**A 验证**：`opc_get_domain_pack_automation_rules` / `opc_execute_workflow` 兜底 / `opc_list_runtime_industries` 行为与迁移前等价；`run_automation_rules` 命中的规则来自 runtime.yaml。

### 阶段 B：14 个工作流模板落 workflows/*.yaml

**B1 统一加载器**（放 `domain_pack.rs` 就近）：`load_workflow_templates_from_yaml(dir) -> Vec<WorkflowTemplateData>`，扫描 `config/opc/domain_packs/{id}/workflows/*.yaml`，`serde_yaml::from_str`。
**B2 重写 seed 入口**（`opc_workflows/mod.rs::seed_opc_domain_packs_from_seed_files` :143）：遍历 14 域包 `workflows/*.yaml` → `check_template_version` + `upsert_template`。
**B3 生成 14 个 YAML**：用**一次性导出脚本**把现有 seed 硬编码 `WorkflowTemplateData` `serde_yaml` 序列化落盘为 `workflows/{template_id}.yaml`，人工审后提交（避免手工转录节点/边图出错）。
**B4 改造 seed 文件**：14 个 `seed_domain_pack_*.rs` 改为薄 YAML 加载或删除（由 loader 替代）。`workflow.rs::generate_domain_pack_template_data`（被 `opc_execute_workflow:2604` 兜底）**保留**作三级兜底，不冲突。

### 阶段 C：删除硬编码 + 收尾

- A、B 验证通过后：删除 `domain_pack_config.rs` 14 个生产函数；`get_all_configs`/`get_config`/`list_industries` 统一走 YAML。
- 内嵌测试 `domain_pack_config.rs:1132`（`test_opc_record_types_do_not_collide_with_kg_entity_types`）改读 YAML 组装结果。

## 关键文件

- `crates/analysis-engine/src/opc/domain_pack.rs`（runtime_schema / 加载器）
- `crates/analysis-engine/src/opc/domain_pack_config.rs`（14 函数 + get_all/get_config/list）
- `src/commands/opc_domain_pack_runtime.rs`（get_config 消费 + get_enabled_rules/run_automation_rules/get_dashboard）
- `src/commands/opc_domain_pack_logic.rs`（validator）
- `src/commands/opc_workflows/mod.rs` + `seed_domain_pack_*.rs`（seed 入口 + 14 模板）
- `crates/harness/src/workflow_types.rs`（WorkflowTemplateData/WorkflowNode serde）
- `config/opc/domain_packs/{id}/runtime.yaml` + 新增 `workflows/*.yaml`

## 验证（每阶段收口）

- `cargo fmt --check` + `cargo clippy --workspace --all-targets -- -D warnings` + `__TAURI_WORKSPACE__=true cargo test -p axagent`（前端 typecheck 不受影响：runtime_schema 未导出 TS、KPI 走 `KpiDefinition`）。
- `node scripts/check-layer-discipline.mjs`（command 层不得新增直连）。
- 功能等价：迁移前后 `opc_list_runtime_industries` / `opc_get_domain_pack_automation_rules` / 兜底 `opc_execute_workflow` 返回一致；B 后 `workflows/*.yaml` 能 upsert。