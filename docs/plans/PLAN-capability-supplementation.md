> 来源：导入自上游 AxAgent（基准 99c9b96f，2026-09-07） ｜ 导入日期：2026-11-04

# 能力补齐链路补全方案

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** 补全认知编排器能力补齐链路的四个缺口，使 CapabilityMissing / GuardRule / ExemptAuthorize 三种补齐场景全链路闭环。

**Architecture:** 在现有 `cognitive_query` 入口的补齐提议通道（T0.3~T0.5）基础上，补全三个阶段：① 工作流模板实际生成与落库；② 安全规则动态注入与回滚；③ 补齐后自动重试；④ 补齐产物的进化闭环反馈。复用已有 `generate_workflow_from_prompt`、`Disposer`、`WorkflowReflector`、`WorkflowEvolver` 基础设施，不引入新框架。

**Tech Stack:** Rust (Tauri commands + harness traits) · TypeScript (React + evolutionStore) · SQLite (SeaORM workflow_templates) · LLM (工作流模板生成)

---

## 现状总结

```
cognitive_query 补齐提议通道 (T0.3~T0.5)
    │
    ├─ CapabilityMissing → build_capability_gap_proposal(None)
    │    └─ await_user_consent → apply_capability_gap_proposal
    │         └─ register_evolution_product()
    │              ├─ 能力护照注册 ✅
    │              ├─ 图谱同步 ✅
    │              └─ 工作流模板创建 ❌ (Gap 1)
    │
    ├─ GuardRule → build_capability_gap_proposal(rejection)
    │    └─ await_user_consent → apply_capability_gap_proposal
    │         └─ 仅记录日志 ❌ (Gap 2)
    │
    └─ ExemptAuthorize → build_capability_gap_proposal(rejection)
         └─ await_user_consent → apply_capability_gap_proposal
              └─ 仅记录日志 ❌ (Gap 2)

补齐后要求用户重发 ❌ (Gap 3)
补齐产物执行后缺少进化反馈 ❌ (Gap 4)
```

---

### Task 1: CapabilityMissing 补齐时生成实际工作流模板

**Gap:** `register_evolution_product` 只注册了能力护照到索引并同步图谱节点，但 `workflow_templates` 表中没有对应模板。下次用户重发请求时，L3 能命中该能力，但 WorkEngine 执行时找不到模板内容。

**Files:**
- Modify: `src-tauri/src/commands/cognitive.rs` (~L1432-1464, `apply_capability_gap_proposal`)
- Create: `src-tauri/src/commands/capability_gap_workflow.rs` (新模块：LLM 生成 + 模板落库)
- Modify: `src-tauri/src/commands/mod.rs` (注册新模块)
- Reference: `src-tauri/src/commands/workflow_ai/generate.rs` (LLM 调用链路)
- Reference: `src-tauri/crates/dao/src/repo/workflow_template.rs` (insert_workflow_template / build_active_model_from_data)

**Step 1: 创建 `capability_gap_workflow.rs` 模块**

新建 `src-tauri/src/commands/capability_gap_workflow.rs`，实现 `generate_gap_workflow_template` 函数：

```rust
//! 能力补齐工作流模板生成器
//!
//! 当认知编排器判定需要补齐能力（CapabilityMissing）且用户同意后，
//! 调用 LLM 基于用户输入和补齐提议生成实际的工作流模板（nodes/edges/variables），
//! 写入 `workflow_templates` 表，使 WorkEngine 可执行该模板。

use crate::AppState;
use crate::commands::error::{CommandError, ErrorCategory};
use axagent_harness::runtime_types::capability_gap::CapabilityGapProposal;
use axagent_harness::workflow_types::WorkflowTemplateData;
use axagent_dao::repo::workflow_template::{build_active_model_from_data, insert_workflow_template};

/// 从用户原始输入 + 补齐提议，生成工作流模板并写入数据库。
pub(crate) async fn generate_gap_workflow_template(
    state: &AppState,
    input: &str,
    proposal: &CapabilityGapProposal,
) -> Result<WorkflowTemplateData, CommandError> {
    // 1. 构造 LLM prompt
    let prompt = format!(
        "用户请求「{}」需要新的工作流能力。补齐描述：{}。\
         请生成一个最小可用工作流模板，包含 Trigger 节点、必要的处理节点和 End 节点。\
         输出 JSON 格式 {{ nodes, edges, variables }}。",
        input, proposal.proposal,
    );

    // 2. 调用 LLM 生成
    // 复用 generate_workflow_from_prompt 的内部逻辑
    let generated = generate_workflow_inner(state, &prompt).await?;

    // 3. 构建 WorkflowTemplateData
    let template = WorkflowTemplateData {
        id: format!("evolution:workflow:{}", proposal.id),
        name: proposal.title.trim().to_string(),
        description: proposal.proposal.clone(),
        nodes: generated.nodes,
        edges: generated.edges,
        variables: generated.variables,
        is_preset: false,
        is_editable: true,
        is_public: false,
        visibility: axagent_harness::capability::Visibility::Public,
        tags: vec!["auto_evolved".to_string(), "capability_gap".to_string()],
        version: 1,
        ..Default::default()
    };

    // 4. 写入 workflow_templates 表
    let active_model = build_active_model_from_data(&template);
    insert_workflow_template(state.harness.db(), active_model).await
        .map_err(|e| CommandError::new(
            axagent_harness::error_codes::cognitive::GAP_PROPOSAL_PENDING,
        )
        .with_category(ErrorCategory::Unrecoverable)
        .with_detail(format!("工作流模板创建失败: {e}")))?;

    Ok(template)
}
```

**Step 2: 抽取出 `generate_workflow_inner` 内部函数**

在 `src-tauri/src/commands/workflow_ai/generate.rs` 中，将 LLM 调用逻辑抽取为 `pub(crate) async fn generate_workflow_inner` 供复用。

**Step 3: 修改 `apply_capability_gap_proposal`**

在 `cognitive.rs` 的 `CapabilityMissing` 分支中，插入工作流模板生成调用，并添加 `input` 参数到函数签名：

```rust
CapabilityGapType::CapabilityMissing => {
    match generate_gap_workflow_template(state, &input, proposal).await {
        Ok(template) => {
            crate::commands::capability::register_evolution_product(
                state, &template.id, &template.name, &template.description,
            ).await?;
            let _ = app.emit("capability-gap-template-created", serde_json::json!({
                "templateId": template.id,
                "name": template.name,
            }));
        },
        Err(e) => tracing::error!(%e, "工作流模板生成失败"),
    }
}
```

**Step 4: 注册新模块**

在 `src-tauri/src/commands/mod.rs` 中添加 `pub mod capability_gap_workflow;`

**Step 5: 验证**

```bash
__TAURI_WORKSPACE__=true cargo check -p axagent
```

---

### Task 2: GuardRule/ExemptAuthorize 阶段二动态注入实现

**Gap:** `apply_capability_gap_proposal` 的 `GuardRule` / `ExemptAuthorize` 分支仅记录日志，没有实际注入防护规则或有界授权。

**Existing infrastructure:**
- `Disposer` (tools/registry.rs): 可逆副作用记录器，支持 LIFO 回滚
- `UnifiedToolRegistry::push_effect`: 登记扩展副作用
- `UnifiedToolRegistry::cleanup_runtime_effects`: 全量清理
- `PatternPromptGuard` (harness 层): 安全拦截器

**Files:**
- Modify: `src-tauri/src/commands/cognitive.rs` (~L1454-1461)
- Modify: `src-tauri/crates/harness/src/pattern_prompt_guard.rs` (动态规则接口)
- Modify: `src-tauri/src/init/state.rs` (AppState 补充字段)

**Step 1: 扩展 `PatternPromptGuard` 支持动态规则**

在 `harness/src/pattern_prompt_guard.rs` 中添加动态规则管理接口和 `dynamic_rules` 字段，并在 `process_user_input_structured` 中检查动态规则。

**Step 2: 实现 `GuardRule` 动态注入**

```rust
CapabilityGapType::GuardRule => {
    if let Some(category) = &proposal.category {
        let rule = DynamicGuardRule {
            category: *category,
            pattern: proposal.proposal.clone(),
            reason: proposal.reason.clone(),
            created_at: chrono::Utc::now(),
        };
        state.prompt_guard.add_dynamic_rule(&rule).await;
        // 注册 Disposer 可回滚
        if let Ok(mut registry) = state.harness.tool_registry().write() {
            let pattern = proposal.proposal.clone();
            let guard = state.prompt_guard.clone();
            registry.push_effect(Disposer::new(
                "guard_rule",
                format!("动态防护规则: {:?}", category),
                move || {
                    tokio::spawn(async move { guard.remove_dynamic_rule(&pattern).await; });
                },
            ));
        }
    }
}
```

**Step 3: 实现 `ExemptAuthorize` 有界豁免注入**

```rust
CapabilityGapType::ExemptAuthorize => {
    let exemption = ExemptAuthorization {
        pattern: proposal.proposal.clone(),
        scope: "developer_mode".to_string(),
        expires_at: chrono::Utc::now() + chrono::Duration::hours(24),
        created_at: chrono::Utc::now(),
    };
    state.exempt_authorizer.add_exemption(&exemption).await;
    // 注册 Disposer 可回滚
    if let Ok(mut registry) = state.harness.tool_registry().write() {
        let pattern = proposal.proposal.clone();
        let authorizer = state.exempt_authorizer.clone();
        registry.push_effect(Disposer::new(
            "exempt_authorize",
            format!("有界豁免: {}", proposal.proposal),
            move || {
                tokio::spawn(async move { authorizer.remove_exemption(&pattern).await; });
            },
        ));
    }
}
```

**Step 4: 补充 AppState 字段**

确保 `AppState` 中包含 `prompt_guard: Arc<PatternPromptGuard>` 和 `exempt_authorizer: Arc<ExemptAuthorizer>`。

**Step 5: 验证**

```bash
__TAURI_WORKSPACE__=true cargo check -p axagent
```

---

### Task 3: 补齐后自动重试机制

**Gap:** 当前所有补齐场景在用户同意并完成补齐后返回 `GAP_PROPOSAL_APPLIED` 错误，要求用户手动重发。

**Files:**
- Modify: `src-tauri/src/commands/cognitive.rs`

**Step 1: 重构 `cognitive_query` 为内部递归函数**

抽取现有逻辑为 `cognitive_query_impl(app, &state, request, retry_count)`，补齐完成后不再返回 Err，而是递归调用自身：

```rust
const MAX_CAPABILITY_GAP_RETRIES: usize = 2;

async fn cognitive_query_impl(
    app: tauri::AppHandle,
    state: &AppState,
    request: CognitiveQueryRequest,
    retry_count: usize,
) -> Result<CognitiveQueryResponse, CommandError> {
    // ... 现有逻辑 ...
    // 补齐完成后：
    if retry_count < MAX_CAPABILITY_GAP_RETRIES {
        return cognitive_query_impl(app, state, request, retry_count + 1).await;
    }
    // 超限则返回 GAP_PROPOSAL_APPLIED
}

#[tauri::command]
pub async fn cognitive_query(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    request: CognitiveQueryRequest,
) -> Result<CognitiveQueryResponse, CommandError> {
    cognitive_query_impl(app, &state, request, 0).await
}
```

**Step 2: 处理重试策略差异**

| 补齐类型 | 自动重试 | 原因 |
|----------|---------|------|
| CapabilityMissing | ✅ | 模板已生成，重试可命中 |
| GuardRule | ❌ | 输入已被拦截，重试亦然 |
| ExemptAuthorize | ✅ | 输入已放行 |

**Step 3: 前端加载状态**

在 `evolutionStore` 中添加 `applyingGap: boolean` 和 `applyingGapId: string | null`。

**Step 4: 验证**

```bash
__TAURI_WORKSPACE__=true cargo check -p axagent
```

---

### Task 4: 进化闭环反馈接入

**Gap:** 补齐的工作流执行后没有反馈到进化系统，无从优化。

**Existing infrastructure:**
- `WorkflowReflector` / `WorkflowEvolver` / `WorkflowOptimizer` 三层 trait
- `WorkflowReflectorImpl` (trajectory crate)
- `WorkEngine::with_workflow_reflector` / `with_workflow_evolver`

**Files:**
- Modify: `src-tauri/src/commands/capability.rs` (标记 evolvable)
- Modify: `src-tauri/src/commands/cognitive.rs` (记录执行统计)

**Step 1: 标记补齐工作流为可进化**

在 `capability.rs` 的 `register_evolution_product` 中，tags 添加 `"evolvable"` 和 `"capability_gap"`。

**Step 2: 记录补齐工作流命中执行**

在 `cognitive_query` 的工作流执行分支中，检查 `capability_id` 是否以 `evolution:workflow:` 开头，是则记录命中统计。

**Step 3: 确认 WorkEngine 反思钩子已就绪**

检查 wiring 层是否已正确调用 `with_workflow_reflector` 和 `with_workflow_evolver`，确保补齐工作流执行完成后自动触发反思。

**Step 4: 前端展示**

在 `evolutionStore` 中添加 `gapWorkflows: GapWorkflowStatus[]` 类型。

**Step 5: 验证**

```bash
__TAURI_WORKSPACE__=true cargo check -p axagent
```

---

## 执行顺序

```
Task 2 (安全规则注入) ──┐
                       ├── 无依赖，可并行
Task 1 (模板生成) ─────┤
                       │
Task 3 (自动重试) ─────┴── 依赖 Task 1
Task 4 (进化闭环) ─────┴── 依赖 Task 1，可与 Task 3 并行
```

- **Phase 1（并行）**: Task 1 + Task 2
- **Phase 2（并行）**: Task 3 + Task 4（均依赖 Task 1 完成）

## 验证方式

```bash
# 单元测试
__TAURI_WORKSPACE__=true cargo test -p axagent --lib commands::cognitive
__TAURI_WORKSPACE__=true cargo test -p axagent --lib commands::capability_gap_workflow

# 端到端：发送未知请求 → 补齐弹窗 → 同意 → 模板生成 → 自动重试 → 命中执行
# 端到端：发送攻击请求 → 防护拦截 → 补齐弹窗 → 同意 → 规则注入 → 自动重试 → 精确拦截
# 回滚：cleanup_runtime_effects → 动态规则/豁免已移除
```
