> 来源：导入自上游 AxAgent（基准 99c9b96f，2026-09-07） ｜ 导入日期：2026-11-04

# AxAgent 自进化实施计划

> 依据：[self-evolution-gap-analysis.md](./self-evolution-gap-analysis.md)（含 5.2.5 / 5.3.3 字段修正后的最新版）
> 目标：把自进化从「名不副实」落地为「双通道闭环 + 产物可见」，最高优先级是阶段零。
> 对齐说明：本计划中所有 `CapabilityPassportDto` 示例已按真实结构（`capability_id` / `sub_category` / `tags: Vec<String>` / 无 `path`、`confidence`）编写，与修正后的 gap-analysis.md 完全一致。

---

## 一、目标与范围

| 项 | 内容 |
|----|------|
| 目标 | 打通「能力补齐（通道一）」与「能力偏弱进化改进（通道二）」两条通道，让进化产物经**能力护照 + 工作流图谱**进入认知编排器 L2 混合检索 / L3 RAR 路由，实现「用户下一次输入即可命中进化产物」 |
| 硬前提 | **任何补齐/进化动作必须征得用户显式同意**（铁律，绝无静默自动执行） |
| 不做 | 不照搬 Cordis 插件内核、不做真实 GRPO 训练、不做完整插件生态 |
| 技术基线 | 所有产物遵守 `SYSTEM_ONLY` 隔离 + 自指熔断 + Disposer 可回滚 |

---

## 二、总体路线与阶段依赖

```
阶段零（P0 最高优先级）：双通道闭环 ←──── 本计划先行交付
  ├─ 通道一：能力补齐（不存在 → 提议 → 用户同意 → 生成）
  │   前置依赖：类型定义、结构化拒绝、NO_CANDIDATE/拒绝/Clarify 三触发点
  └─ 通道二：进化改进（偏弱 → 反思 → 提议 → 用户同意 → 更新）
      前置依赖：技能反思钩子、贝叶斯后验、产物护照/图谱注册
        │
        ▼（阶段零的「产物注册/回滚」能力被阶段一复用）
阶段一（P0）：运行时热插拔 + 闭环闭合（AutoToolCreator → ToolRegistry → 护照/图谱）
阶段二（P1）：自指工具 + 可逆副作用（复用 system_* 系统能力通道）
阶段三（P2）：证据驱动进化（决策标签 + Clarify + 执行反馈 → 贝叶斯后验）
阶段四（P3）：组合泛化 + 递归优化（远期，经 L3 图谱动态组合）
```

**依赖规则**：阶段零产出的 `register_evolution_product` / `Disposer` / 同意通道，是阶段一/二的公共底座，必须先落地。阶段一/二/三可并行推进，但均依赖阶段零的护照注册与用户同意机制。

---

## 三、阶段零：双通道闭环（P0 最高优先级）

### 3.1 任务分解总览

| 任务 ID | 任务 | 通道 | 涉及文件 | 依赖 |
|---------|------|------|----------|------|
| T0.1 | `CapabilityGapProposal` / `PromptAttackCategory` 类型定义 | 一 | `harness/src/runtime_types/capability_gap.rs`（新增） | — |
| T0.2 | `process_user_input` 结构化拒绝 `PromptRejection` | 一 | `harness/src/prompt_guard.rs` | T0.1 |
| T0.3 | 拒绝分支改造（前置 1 → 能力补齐提议） | 一 | `commands/cognitive.rs` | T0.1、T0.2 |
| T0.4 | `NO_CANDIDATE` 改造（无产出 → capability_missing 提议） | 一 | `commands/cognitive.rs` | T0.1 |
| T0.5 | Clarify 兜底无候选 → 提议 | 一 | `commands/cognitive.rs` | T0.1 |
| T0.6 | `build_capability_gap_proposal` 归类分析器 | 一 | `commands/cognitive.rs`（或独立 `capability_gap.rs`） | T0.1 |
| T0.7 | `apply_capability_gap_proposal` 补齐执行（护照+图谱，按真实字段） | 一 | `commands/capability.rs`、`commands/cognitive.rs` | T0.1 |
| T0.8 | 错误码 `COGNITIVE_GAP_PROPOSAL_PENDING` + 11 语言翻译 | 一 | `harness/src/error_codes.rs`、`src/i18n/locales/*.json` | T0.1 |
| T0.9 | 技能侧反思钩子（execute_post_tool_call 挂接） | 二 | `runtime-core/src/conversation.rs`、`trajectory/src/skill_evolution.rs` | — |
| T0.10 | 贝叶斯后验 `SkillPosterior`（evidence.rs） | 二 | `trajectory/src/evidence.rs`（新增） | — |
| T0.11 | 进化产物护照/图谱注册（`register_evolution_product`） | 二 | `commands/evolution_engine.rs`、`commands/capability.rs`、`harness/src/workflow_graph.rs` | T0.7 |
| T0.12 | `WorkflowSuggestion` 用户同意后落库 | 二 | `commands/workflow_reflection.rs` | T0.10 |
| T0.13 | 前端同意弹窗 UI（能力补齐/进化改进） | 一+二 | `src/stores/feature/`、`src/components/`、11 语言 i18n | T0.8 |
| T0.14 | 阶段零集成测试 | 一+二 | `src-tauri/crates/trajectory/tests/`、`src-tauri/src/commands/` | 全部 |

---

### 3.2 通道一：能力补齐（对不存在的能力）

#### T0.1 类型定义（`harness/src/runtime_types/capability_gap.rs`）

新增 `CapabilityGapProposal` 与 `PromptAttackCategory`（harness 层纯 DTO，遵循 foundation 零依赖约束）：

```rust
/// 安全拦截命中模式的攻击类别（与 PatternPromptGuard 现有分类对齐）
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PromptAttackCategory {
    RoleOverride,   // 角色覆盖（如 developer mode）
    Delimiter,      // 分隔符注入
    TaskHijack,     // 任务劫持
    PromptLeak,     // 提示词泄露
    Jailbreak,      // 越狱
}

/// 能力缺口类型
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityGapType {
    GuardRule,          // 防护规则缺口：攻击手法不在静态列表 → 提议补规则
    ExemptAuthorize,    // 误伤豁免：本地 IDE 合法诉求 → 提议安全豁免授权
    CapabilityMissing,  // 能力缺口：系统当前无能力安全处理 → 提议生成工作流/技能/工具
}

/// 能力补齐提议（前端弹窗展示 / 用户同意 / 落库）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CapabilityGapProposal {
    pub id: String,
    pub gap_type: CapabilityGapType,
    pub category: Option<PromptAttackCategory>, // 安全拦截场景才有
    pub title: String,                          // 前端弹窗标题
    pub proposal: String,                       // 新增防护规则 / 授权范围 / 补齐工作流描述
    pub reason: String,                         // 命中原因 + 缺口说明
    pub impact: String,                         // 影响范围
    pub rollback: String,                       // 可逆副作用说明（挂 Disposer）
    pub created_at: chrono::DateTime<chrono::Utc>,
}
```

**注意**：该模块放在 `runtime_types/` 下并 re-export，`commands` 层与前端 DTO 保持一致（serde 字段名按项目 camelCase 约定）。

#### T0.2 结构化拒绝（`harness/src/prompt_guard.rs`）

`process_user_input` 的 `Err(String)` 升级为结构化拒绝，保留兼容层避免破坏既有调用方：

```rust
/// 结构化拒绝结果：分类 + 命中模式 + 建议
pub struct PromptRejection {
    pub category: PromptAttackCategory,
    pub pattern: String,
    pub reason: String,
    pub suggestion: String, // 供 build_capability_gap_proposal 归类
}
```

- `process_user_input` 保持返回 `Result<String, PromptRejection>`；既有直接消费 `String` 的调用点用 `rejection.reason` 兼容。
- **必须 grep 全项目**确认 `process_user_input` 的调用方数量与位置，逐处适配。

#### T0.3 拒绝分支改造（`commands/cognitive.rs` 前置 1，L423-436）

将「命中即拒绝」改为「拒绝 + 能力补齐提议」双通道。**安全底线不变**：命中注入/越狱依然**绝不透传给下游执行器**，仅多走一条「分析缺口 → 征求同意」通道：

```rust
let input = match prompt_guard.process_user_input(&input) {
    Ok(processed) => processed,
    Err(rejection) => {
        tracing::error!(%rejection.reason, "🛡️ 安全拦截命中，进入能力补齐提议通道");
        // ① 保留硬阻断：绝不透传（原逻辑，仍写安全日志）
        // ② 拒绝分析：裸 reason 归类为结构化缺口
        let proposal = build_capability_gap_proposal(&rejection, &input).await?;
        // ③ 征求用户同意（复用授权事件通道，前端弹窗）
        if !await_user_consent(&state, &proposal).await? {
            // 用户拒绝 → 保持原拒绝行为 + 拒绝即证据
            persist_decision_to_message(db, &msg_id, &build_rejection_decision(&rejection)).await;
            return Err(CommandError::new(
                axagent_harness::error_codes::cognitive::PROMPT_REJECTED,
            )
            .with_category(ErrorCategory::Unrecoverable)
            .with_detail(rejection.reason));
        }
        // ④ 用户同意 → 执行补齐（防护规则 / 授权 / 工作流，挂 disposer）
        apply_capability_gap_proposal(&state, &proposal).await?;
        // ⑤ 决策标签记录：拒绝原因 + 补齐结果（供阶段三贝叶斯后验消费）
        persist_decision_to_message(db, &msg_id, &build_gap_decision(&proposal)).await;
        // 补齐后的输入按安全化语义继续（或按用户选择直接放行，需显式授权）
        Ok(rejection.safe_continuation(input))
    },
};
```

**关键点**：
- 新增 `build_capability_gap_proposal`（T0.6）与 `apply_capability_gap_proposal`（T0.7）。
- 新增 `await_user_consent`（T0.13 配套），复用既有授权事件通道（前端已有 `permissionRequest` 类 i18n，11 语言齐备）。
- `build_rejection_decision` / `build_gap_decision` 产出结构化决策标签，复用 `persist_decision_to_message`（L390）。

#### T0.4 NO_CANDIDATE 改造（`commands/cognitive.rs` L728-732）

主 DAG 无产出时不再只返回 Retryable 错误，改为生成 `capability_missing` 提议：

```rust
let l3_value = match workflow.output {
    Some(v) => v,
    None => {
        // 无候选 → 不直接报错，进入能力补齐提议通道
        let proposal = build_capability_gap_proposal_for_missing(&input).await?;
        if await_user_consent(&state, &proposal).await? {
            apply_capability_gap_proposal(&state, &proposal).await?;
            return Err(CommandError::new(
                axagent_harness::error_codes::cognitive::GAP_PROPOSAL_APPLIED,
            )
            .with_category(ErrorCategory::Recoverable)
            .with_detail("已按用户同意补齐能力，请重新发送请求".to_string()));
        }
        return Err(CommandError::new(
            axagent_harness::error_codes::cognitive::NO_CANDIDATE,
        )
        .with_category(ErrorCategory::Retryable)
        .with_detail("认知编排器主 DAG 未产出路由决策".to_string()));
    }
};
```

#### T0.5 Clarify 兜底无候选 → 提议

Clarify 分支（置信度 0.60~0.90）候选为空且无 `forcedCapabilityId` 时，进入同一提议通道（不重复建逻辑，复用 T0.3 的 `await_user_consent` + `apply_capability_gap_proposal`）。

#### T0.6 build_capability_gap_proposal（归类分析器）

输入：`PromptRejection` 或「无候选」信号 → 输出 `CapabilityGapProposal`：

| 场景 | gap_type | 提议内容 |
|------|----------|----------|
| 攻击手法不在静态防护列表 | GuardRule | 新增防护正则（挂 Disposer 可回滚） |
| 本地 IDE 合法诉求误伤（如 developer mode） | ExemptAuthorize | 按命中的具体模式+作用域授权（有界豁免，不开放全量） |
| 用户请求系统当前无能力安全处理 | CapabilityMissing | 生成补齐工作流/技能/工具（见 T0.7） |
| 主 DAG 无候选 / Clarify 兜底无命中 | CapabilityMissing | 生成补齐工作流模板 + 护照 + 图谱 |

#### T0.7 apply_capability_gap_proposal（补齐执行）

> **✅ 已对齐（gap-analysis.md 5.2.5 已修正）**：`CapabilityPassportDto` 真实字段为 `capability_id`（**不是** `id`），且**无 `path` / `confidence` 字段**（见 `harness/src/capability.rs` L533-573，无 `Default` 派生，需显式构造全部必填字段）。`route_path` 概念不落在护照字段上：L3 路径由 `WorkflowGraphSync::sync_workflow` 决定，`confidence` 留在决策标签而非护照。下方示例即修正后的权威写法，与 gap-analysis.md 5.2.5 / 5.3.3 一致。

```rust
/// 简单能力 → 注册普通工具护照
fn build_tool_passport(name: &str, description: &str) -> CapabilityPassportDto {
    CapabilityPassportDto {
        capability_id: format!("evolution:tool:{name}"),
        name: name.to_string(),
        description: description.to_string(),
        kind: CapabilityKind::Tool,
        domain: CapabilityDomain::General,
        sub_category: "auto_generated".to_string(),       // L2 簇
        visibility: Visibility::Discoverable,             // 进化产物默认对外可见
        caller_permissions: CallerPermissions::default(), // 权限按需收紧
        tags: vec!["auto_evolved".to_string(),
                   format!("route:/evolution/auto_generated/tool/{name}")],
        security_level: SecurityLevel::default(),
        planning_complexity: PlanningComplexity::default(),
        enabled: true,
        // 其余字段：input_schema/negative_scenarios/modality_support/
        // output_capabilities/model_iq_requirement/stats 等按默认构造
        .. 全部字段显式补齐
    }
}

/// 复杂能力 → 生成工作流模板 + 护照 + 图谱同步
async fn register_workflow_product(
    indexer: &Arc<dyn CapabilityIndexer>,
    graph: &mut WorkflowGraph,
    workflow_id: &str,
    display_name: &str,
    description: &str,
) -> Result<(), String> {
    // 1. 同步进工作流图谱（L3 system_workflow_graph_router 可见）
    WorkflowGraphSync::sync_workflow(graph, "general", "auto_generated", workflow_id, display_name);
    // 2. 注册能力护照（L2 混合检索可见）
    let passport = CapabilityPassportDto {
        capability_id: format!("evolution:workflow:{workflow_id}"),
        name: display_name.to_string(),
        description: description.to_string(),
        kind: CapabilityKind::Workflow,
        domain: CapabilityDomain::General,
        sub_category: "auto_generated".to_string(),
        visibility: Visibility::Discoverable,
        tags: vec!["auto_evolved".to_string(),
                   format!("route:/evolution/auto_generated/workflow/{workflow_id}")],
        enabled: true,
        // 其余字段同 build_tool_passport 补齐
    };
    indexer.index_passport(&passport).await?;
    Ok(())
}
```

- **领域能力**：一并生成 `agent_profile:*` 护照（`sub_category = "agent_profile"`），`agent_profile_id` 指向其关联的 AgentProfile。
- **可逆**：注册动作返回/登记 `Disposer`（见阶段二 T2 副作用栈），用户可随时撤销。
- **安全**：`route:/evolution/` 前缀纳入自指熔断 `protected_keywords`，进化产物仅供业务工作流调用，不参与编排器路由回路。

#### T0.8 错误码 + 11 语言翻译

`harness/src/error_codes.rs`（`mod cognitive`）新增：

```rust
pub const GAP_PROPOSAL_PENDING: &str = "COGNITIVE_GAP_PROPOSAL_PENDING";
pub const GAP_PROPOSAL_APPLIED: &str = "COGNITIVE_GAP_PROPOSAL_APPLIED";
pub const GAP_PROPOSAL_REJECTED: &str = "COGNITIVE_GAP_PROPOSAL_REJECTED";
```

**强制**：同步在 `src/i18n/locales/` 全部 11 种语言（zh-CN 为源）的顶层 `error` 段添加 `error.${CODE}` 翻译，缺译会导致该语言显示原始码串。前端错误解析按现有 `JSON.parse(e.message)` → `t("error.${code}", params)` 模式消费。

---

### 3.3 通道二：进化改进（对已有能力偏弱）

复用现状（不重造）：`WorkflowEvolverImpl::should_auto_evolve` / `run`、rt-workflow `post_execution_reflect`、`WorkflowSuggestion`（`harness/src/workflow_optimization.rs`，字段 `id/category/priority/target_node_id/description/...`）。补四个缺口：

#### T0.9 技能侧反思钩子（`runtime-core/src/conversation.rs`）

在工具调用完成后的 `execute_post_tool_call` 钩子（L1133）挂接技能反思：技能执行完成后，若失败或质量低 → 调用 `SkillEvolutionEngine::should_auto_evolve`，命中则生成 `SkillEvolutionProposal` 进入用户同意通道（复用 T0.13 弹窗）。

```rust
// execute_post_tool_call 内部追加：
if let Some(skill) = skill_of_tool(tool_name) {
    record_skill_outcome(&skill, &tool_result).await;  // 更新 consecutive_failures/total_usages/success_rate
    if evolution_engine.should_auto_evolve(&skill).await {
        let proposal = evolution_engine.propose_improvement(&skill).await?;
        // 进入用户同意通道（T0.13），同意后执行进化 + T0.11 注册
        emit_evolution_consent(&proposal).await?;
    }
}
```

- `record_skill_outcome` 复用 `Skill` 既有字段（`consecutive_failures` / `total_usages` / `success_rate`）。
- 先 grep 确认 `Skill` 结构与字段名，避免臆造。

#### T0.10 贝叶斯后验（`trajectory/src/evidence.rs` 新增）

```rust
pub struct SkillPosterior {
    alpha: f64,       // 成功计数 + 1
    beta: f64,        // 失败计数 + 1
    prior_alpha: f64, // 先验参数（可用既有一致性先验）
    prior_beta: f64,
}

impl SkillPosterior {
    /// 双证据更新：决策置信度（认知编排器）+ 执行结果
    pub fn update(&mut self, confidence: f64, success: bool) { /* 按置信度加权 */ }
    /// P(success) = alpha / (alpha + beta)
    pub fn p_success(&self) -> f64 { self.alpha / (self.alpha + self.beta) }
}
```

决策规则（替换 2.5 的失败率/成功率启发式）：
- `P(success) < 低阈值` → 触发进化（进入用户同意通道）
- `P(success) > 高阈值` 且样本量足够 → 标记稳定
- 样本量不足但有失败 → 标记待观察

#### T0.11 进化产物护照/图谱注册（`register_evolution_product`）

工作流侧 `evolver.run` 产物、技能侧进化产物统一走 T0.7 的 `register_workflow_product` / `build_tool_passport`（含 `auto_evolved` 标签 + `/evolution/` 前缀），消除 2.5.2 缺口 3「产物未注册护照/图谱」。

#### T0.12 WorkflowSuggestion 用户同意后落库（`commands/workflow_reflection.rs`）

`workflow_optimize_suggest` 已生成 `WorkflowSuggestion`（注释仅「供前端展示/人工审核」）。补齐落地：

- 新增命令 `workflow_suggestion_confirm`：前端弹窗同意后调用，把建议写入模板 + 注册护照/图谱（复用 T0.11）。
- 新增命令 `workflow_suggestion_reject`：拒绝即丢弃 + 记决策标签（拒绝即证据）。
- 现有 `workflow_optimize_apply` 保持向后兼容（可作为程序化 apply 通道，但生产路径必须经用户同意）。

---

### 3.4 前端同意弹窗 UI（T0.13）

**文件**：`src/stores/feature/`（新增 `evolutionStore` 或复用现有授权 store）+ `src/components/`（新增 `EvolutionConsentModal`）。

- **事件通道**：复用既有授权事件通道（`permissionRequest` 类），新增事件名如 `evolution-consent-request`；后端 `await_user_consent` 通过该事件下发 `CapabilityGapProposal` / `WorkflowSuggestion`，前端弹窗确认后回传。
- **弹窗内容**：提议标题 / gap_type（补齐 或 改进）/ 具体改动 / 影响范围 / 回滚方式 / 风险提示；按钮：同意、拒绝、查看详情。
- **i18n**：全部文案走 `t()`，11 种语言同步补 key。
- **组件规范**：命名导出 + 透传 ref（若被 Popover/Modal 等浮层包裹）；Modal 用 `destroyOnHidden`（禁 `destroyOnClose`）。
- **决策标签展示**：同意/拒绝结果随消息决策标签展示（复用 `persist_decision_to_message` 链路）。

---

### 3.5 阶段零验收标准

| 编号 | 验收项 | 验证方式 |
|------|--------|----------|
| A0.1 | 安全拦截命中后弹窗提议补齐，用户拒绝仍返回 `PROMPT_REJECTED` 且不透传 | Rust 单测 + 手动验证 |
| A0.2 | 用户同意后执行补齐，产物护照注册 + 图谱同步成功，`capability_list_passports` 可见 `evolution:*` 护照 | 命令单测 |
| A0.3 | 主 DAG 无产出时进入能力补齐提议（不再静默报错） | Rust 单测 |
| A0.4 | 技能执行失败达到阈值后触发反思钩子，生成进化提议（技能侧 `should_auto_evolve` 不再死代码） | conversation 单测 |
| A0.5 | 进化/补齐产物遵守 `SYSTEM_ONLY` 隔离 + 自指熔断 + Disposer 可回滚 | 安全单测 |
| A0.6 | 新错误码 11 语言翻译齐全（`error.${CODE}`） | `npm run typecheck` + grep |
| A0.7 | `cargo fmt` / `cargo clippy -D warnings` / `npm run format` / `npm run typecheck` 全绿 | CI |

---

## 四、阶段一：运行时热插拔 + 闭环闭合（P0）

| 任务 ID | 任务 | 涉及文件 | 依赖 |
|---------|------|----------|------|
| T1.1 | `UnifiedToolRegistry` 新增 `register_runtime_tool` / `unregister_runtime_tool` / `runtime_tool_sources` 字段 | `tools/src/registry.rs` | T0.7 |
| T1.2 | `AutoToolCreator` 输出经回调接入 ToolRegistry（或 `evolution_engine.rs` 桥接） | `trajectory/src/auto_tool.rs`、`commands/evolution_engine.rs` | T1.1 |
| T1.3 | 持久化标记 `source = 'runtime_evolution'`，重启自动加载 | `runtime/src/tool_generator/persistence.rs` | T1.1 |
| T1.4 | `get_chat_tools()` 包含运行时注册工具，会话可见 | `runtime-core`（确认现有实现） | T1.2 |
| T1.5 | 错误码 `TOOL_REGISTRATION_DUPLICATE` 等 + 11 语言 | `commands/error_code.rs`、i18n | T1.1 |

**验收**：进化工具注册后，同会话 Agent 立即可调用；卸载后立即不可见；重启后已持久化工具自动加载。

**安全**：`unregister_runtime_tool` 仅允许来自 `runtime_tool_sources` 的来源（防止误卸内置工具）；命名冲突返回 `TOOL_REGISTRATION_DUPLICATE`。

---

## 五、阶段二：自指工具 + 可逆副作用（P1）

| 任务 ID | 任务 | 涉及文件 | 依赖 |
|---------|------|----------|------|
| T2.1 | `runtime_mutation.rs` 定义自指工具（inspect/define/deploy/undeploy） | `tools/src/runtime_mutation.rs`（新增） | T1.1 |
| T2.2 | `Disposer` 类型 + `effects` 副作用栈（后进先出清理） | `tools/src/registry.rs` | T1.1 |
| T2.3 | `system_evolution_*` 系统能力回调，复用 `system_*` 通道 + SYSTEM_ONLY 护照 | `harness/src/cognitive_router.rs`、`commands/capability.rs` | T0.7 |
| T2.4 | 自指熔断配置：`/evolution/` 前缀入 `protected_keywords` | `harness`（SelfReferenceProtection） | T2.3 |
| T2.5 | 前端权限控制 UI | `src/stores/feature/`、`src/components/` | T0.13 |

**关键**：自指工具**不**硬塞 Agent 预设，走 `system_*` 系统能力回调通道（主 DAG 引擎内部回调执行），天然遵守 SYSTEM_ONLY 隔离 + 自指熔断。自指护照：

```rust
let passport = CapabilityPassportDto {
    capability_id: format!("system:self_evolution:{name}"),
    kind: CapabilityKind::Tool,
    domain: CapabilityDomain::System,      // System 域，业务检索不可达
    sub_category: "self_evolution".to_string(),
    visibility: Visibility::SystemOnly,    // 仅系统内部可见
    tags: vec!["SYSTEM_ONLY".to_string(), "META".to_string()],
    enabled: true,
    // 其余字段按 T0.7 模式补齐
};
```

---

## 六、阶段三：证据驱动进化（P2）

| 任务 ID | 任务 | 涉及文件 | 依赖 |
|---------|------|----------|------|
| T3.1 | `TrajectoryStorage` append-only 化（可标记失效不可删除） | `trajectory/src/storage.rs` | — |
| T3.2 | 贝叶斯后验接入进化决策（消费决策标签流 + 执行反馈） | `trajectory/src/evidence.rs`（复用 T0.10） | T0.10 |
| T3.3 | 决策标签作为证据源（execution_mode / confidence / route_path / 选中能力） | `commands/cognitive.rs` | T0.3 |
| T3.4 | 进化策略 if-else → LLM 结构化 diff + 沙箱验证 | `trajectory/src/skill_evolution.rs` | T3.2 |

**证据源汇总**（复用认知编排器已有观测，不另起炉灶）：
```
证据源 = 决策标签流（每条消息已持久化 execution_mode / confidence / route_path / 选中能力）
      + 执行反馈（Workflow 成败 / Plan 拒绝 / Clarify 澄清率）
      + 安全拦截拒绝（阶段零产物，见 T0.3）
        ↓
贝叶斯后验（Beta 分布）→ 低于阈值触发进化（经用户同意通道）
        ↓
进化产物 → 护照 / 图谱注册（T0.11）→ 下一轮用户输入可命中
```

---

## 七、阶段四：组合泛化 + 分层执行（P3）

### 7.1 目标与现状依据

**目标**：把进化产物从「可发现的模板定义」升级为「可真正执行的产物」，并按产物类型分层执行，复用项目已验证的两套引擎（Rhai + rt-workflow），**不新造执行器**。

**现状依据（查证结论）**：
- `GeneratedTool.code` 目前是伪 JS 模板（`DefaultLlmToolProvider::build_code_from_template`），`GeneratedToolAdapter::call()` 只返回定义不执行（阶段一中间态）。
- `RhaiEngineAdapter` / `RhaiEngine`（`axagent-tools::rhai_engine`）已完整实现：编译缓存、30s 超时、`RhaiToolFn`（脚本内可调用注入工具）。
- `RhaiToolDef { tool_name, description, code }` 已存在，工作流模板 `tool_defs` 字段可直接把脚本注册为可调用工具。
- `WorkflowGenome { nodes, edges, variables, changed_node_ids, ... }` 已存在，`WorkflowEvolverImpl` 已实现遗传进化（LLM 语义变异 + 启发式调整），但 `sandbox=None` 占位跳过验证。

**分层执行决策**（经用户确认「两者结合」）：
- **计算型产物** → Rhai 脚本节点：纯计算 / 数据处理 / 内部工具编排，复用 `RhaiEngineAdapter`。
- **编排型产物** → 工作流 DAG：涉及内部能力的调用序列，映射为 `WorkflowGenome`，复用 rt-workflow 引擎执行。

### 7.2 任务分解总览

| 任务 ID | 任务 | 涉及文件 | 依赖 |
|---------|------|----------|------|
| T4.1 | 进化产物分类定型（`EvolutionArtifactKind`: RhaiScript / WorkflowDag），LLM 进化输出类型标注 | `harness/src/trajectory_types.rs`、`trajectory/src/auto_tool.rs` | — |
| T4.2 | 计算型产物执行链路：`GeneratedToolAdapter::call()` 接入 Rhai 引擎真正执行 | `tools/src/generated_tool.rs`、`tools/src/rhai_engine.rs`、`harness/src/rhai_engine.rs` | T4.1 |
| T4.3 | 编排型产物执行链路：进化产物 → `WorkflowGenome` → rt-workflow 执行，护照/图谱注册 | `trajectory/src/workflow_adapters/evolver.rs`、`commands/evolution_engine.rs`、`commands/capability.rs` | T4.1 |
| T4.4 | 沙箱真实验证：wiring 注入真实 `WorkflowSandbox`（结构性 + 试运行）；Rhai 产物执行前沙箱验证 | `src/init/workflow_injections.rs`、`commands/evolution_engine.rs` | T4.2、T4.3 |
| T4.5 | 阶段四集成测试 + 构建验证 | `src-tauri/crates/trajectory/tests/`、`src-tauri/src/commands/` | 全部 |

### 7.3 T4.2 计算型产物执行（复用 Rhai 引擎）

- `GeneratedTool` 增加产物类型字段（T4.1），`call()` 内分支：
  - `RhaiScript`：将 `code` 交给 `RhaiEngineAdapter::execute_script` 真正执行；脚本内可通过 `RhaiToolFn` 注入已注册工具（复用 `tool_fns` 机制），使计算型工具也能编排内部能力。
  - 保持 `is_read_only` 只读语义仅用于纯计算产物；涉及写操作的产物标注可写。
- wiring 层注入 `RhaiEngineAdapter` 到进化工具执行路径（`GeneratedToolAdapter` 通过 trait 接口访问，不直接依赖 `axagent-tools::rhai_engine`）。
- 超时复用 `RhaiEngine` 内置 30s；执行失败返回结构化错误码（复用 `error_codes`，需同步 11 语言）。

### 7.4 T4.3 编排型产物执行（复用 rt-workflow）

- 新增转换函数：`GeneratedTool` / 技能进化产物 → `WorkflowGenome`（`nodes` / `edges` / `variables` 由产物步骤映射）。
- 复用 `WorkflowEvolverImpl` 做拓扑级进化；执行走 rt-workflow 引擎（节点热插拔、权限、审计、Disposer 回滚天然可用）。
- 产物经 `register_evolution_product` 注册护照 + 图谱，`route:/evolution/` 前缀保留在自指熔断 `protected_keywords` 内。
- `variables` 复用现有 `Variable` 定义（`workflow_types.rs`），不新建变量类型。

### 7.5 T4.4 沙箱真实验证

- wiring 层注入真实 `WorkflowSandbox`（替换 `WorkflowEvolverImpl` 默认 `None` 占位）：结构性校验（node id 唯一 / edge 引用有效 / variable name 唯一）+ 试运行。
- 计算型 Rhai 产物执行前先过沙箱（危险模式检测 / 命令长度限制，复用阶段三 `SkillSandboxExecutor` 的模式）。
- 进化产物验证不过 → 不落地；`changed_node_ids` mask 保证 LLM 只改声明节点，不误伤健康节点。

### 7.6 约束

- 复用 `RhaiEngineAdapter` / `WorkflowGenome` / `WorkflowEvolverImpl` 现有类型与引擎，**禁止新建并行执行器或重复类型**。
- 新增命令 / 错误码须两步注册 + 11 语言 i18n（见 §8.2）。
- 进化产物一律先沙箱验证 + 用户同意通道，再落库注册（沿用阶段零铁律）。
- 递归优化器（优化器自身进化）仍为远期评估项，不在本阶段落地。

---

## 八、关键实现约束与接口修正（必须遵守）

### 8.1 CapabilityPassportDto 权威字段清单（已与修正后的 gap-analysis.md 对齐）

gap-analysis.md 5.2.5 / 5.3.3 已按真实结构修正，本清单为实施时的权威依据（`harness/src/capability.rs` L533-573）：

| 字段 | 类型 | 实施要点 |
|------|------|----------|
| `capability_id` | `String` | 主键，`evolution:{kind}:{name}` / `system:self_evolution:{name}`（**不是** `id`） |
| `name` / `description` | `String` | 必填 |
| `kind` | `CapabilityKind` | Tool / Workflow / Skill / Agent / KnowledgeBase |
| `domain` | `CapabilityDomain` | 进化产物 `General`；自指工具 `System` |
| `sub_category` | `String` | L2 簇：`auto_generated` / `self_evolution` / `agent_profile` |
| `visibility` | `Visibility` | 进化产物 `Discoverable`；自指工具 `SystemOnly` |
| `tags` | `Vec<String>` | `auto_evolved` + `route:/evolution/...`（**非 Option**） |
| `security_level` / `planning_complexity` / `enabled` | 枚举 / bool | 必填，显式构造 |

**无 `path` / `confidence` 字段**：L3 路径由 `WorkflowGraphSync::sync_workflow` 决定，置信度留在决策标签。

**`CapabilityPassportDto` 未派生 `Default`**，不能写 `..Default::default()`。所有必填字段必须显式构造；仅各字段自身的 `#[serde(default)]` 可用于反序列化缺省。

### 8.2 错误码 i18n 同步（强制）

- 新增后端错误码（T0.8、T1.5）时，**必须同步补齐 11 种语言**的 `error.${CODE}` 翻译（zh-CN 为源），缺译会导致该语言下显示原始码串。
- 前端消费走 `JSON.parse(e.message)` → `t("error.${code}", params)`，禁止直接 `message.error(String(e))`。

### 8.3 架构分层铁律

- **T0.1/T0.2 放 harness（foundation）**：纯 DTO / 纯逻辑，零 axagent-* 依赖。
- **T0.9 放 runtime-core（consumer）**：仅依赖 harness trait；`SkillEvolutionEngine` 通过 trait 注入，不直接依赖 trajectory 实现层。
- **T0.7/T0.11 放 commands（wiring）**：通过 harness trait（`CapabilityIndexer` / `WorkflowGraphSync`）传能力，不直接构造实现层。
- 新增任何 `Cargo.toml` 的 `axagent-*` 依赖前，对照 AGENTS.md 的 crate 角色表检查依赖方向。

### 8.4 通用编码规范

- 全程中文注释；前端命名导出 + `@/types` 导入 + `t()` i18n。
- 新增 Tauri 命令必须两步注册（`commands/mod.rs` + `lib.rs`）。
- 删除/修改既有接口前 grep 确认调用方，避免破坏调用链。
- 浮层触发组件必须透传 ref（React 19 无 findDOMNode）。

---

## 九、风险与回滚

| 风险 | 等级 | 缓解 / 回滚 |
|------|------|-------------|
| `process_user_input` 返回值类型变更破坏既有调用方 | 中 | 先 grep 全项目调用点；保留兼容层（`rejection.reason` 作为 String 出口） |
| 能力补齐误生成低质量工具/工作流污染能力库 | 高 | 用户同意铁律 + `auto_evolved` 标签可过滤 + Disposer 可回滚 + 沙箱验证 |
| 护照注册引入自引用路由循环 | 高 | `route:/evolution/` 入自指熔断 `protected_keywords`；`SYSTEM_ONLY` 隔离 |
| 动态注册工具与静态工具命名冲突 | 低 | `register_runtime_tool` 重复名校验返回 `TOOL_REGISTRATION_DUPLICATE` |
| 反思钩子误触发（质量分阈值过严/过松） | 中 | 阈值参数化 + 用户同意通道兜底；贝叶斯后验逐步替代启发式 |
| 前端弹窗在低置信度场景高频打扰 | 中 | 弹窗按 gap_type 分组 + 统一队列；`NO_CANDIDATE` 与拒绝分支分别提示 |
| `CapabilityPassportDto` 字段构造遗漏（无 Default） | 中 | 编译期强制补齐必填字段；对照 8.1 修正清单逐项核对 |

---

## 十、测试策略

| 层 | 内容 | 命令 |
|----|------|------|
| Rust 单测（阶段零核心） | `build_capability_gap_proposal` 归类、`apply_capability_gap_proposal` 护照注册、贝叶斯 `SkillPosterior`、`should_auto_evolve` 触发 | `__TAURI_WORKSPACE__=true cargo test -p axagent --lib commands::cognitive`、`cargo test -p axagent-trajectory` |
| Rust 单测（通道二） | 技能反思钩子、`WorkflowSuggestion` 同意/拒绝落库 | `cargo test -p axagent-runtime-core` |
| 前端单测 | EvolutionConsentModal 渲染 / 事件收发 / i18n 键 | `npm run test:run` |
| 集成测试 | 拒绝→同意→补齐→护照可见 全链路 | `cargo test -p axagent --lib`（含 commands 集成） |
| 构建验证 | 类型 / 格式 / lint | `npm run typecheck`、`npm run format`、`cargo fmt`、`cargo clippy -D warnings` |

---

## 十一、里程碑与交付物

| 里程碑 | 内容 | 交付物 |
|--------|------|--------|
| M0（阶段零） | 双通道闭环 | 类型定义、拒绝/NO_CANDIDATE/Clarify 三触发点改造、贝叶斯后验、产物护照/图谱注册、同意弹窗、错误码 11 语言 |
| M1（阶段一） | 运行时热插拔 | `register_runtime_tool` / `unregister_runtime_tool`、AutoToolCreator 桥接、持久化 |
| M2（阶段二） | 自指工具 + 可逆副作用 | `runtime_mutation.rs`、Disposer 副作用栈、`system_evolution_*` 系统能力回调 |
| M3（阶段三） | 证据驱动进化 | append-only 轨迹、贝叶斯决策接入、LLM 结构化 diff 进化 |
| M4（阶段四） | 组合泛化 | 工作流节点动态组合、递归优化器评估（远期） |

> 交付节奏：M0 是硬前置，先行完成并验收（A0.1~A0.7）；M1/M2/M3 在 M0 落地后可并行；M4 视证据积累情况启动。

---

## 十二、依赖清单（外部/基建）

- 无新增第三方依赖：贝叶斯后验用内置 `f64` 数学即可，LLM 语义变异复用现有 `LlmEvolutionProvider`（`harness/src/trajectory_types.rs`）。
- 需要确认可用的基建：`CapabilityIndexer`（app_state 已持初始化实例）、`WorkflowGraphSync`、`WorkflowSuggestion`（`harness/src/workflow_optimization.rs`）、`FilePermissionRequestEvent / AgentPermissionRequest` 事件通道。
- 若发现既有工具/事件通道命名与文档描述有出入，以实际代码为准并同步修正本计划与 gap-analysis.md。
