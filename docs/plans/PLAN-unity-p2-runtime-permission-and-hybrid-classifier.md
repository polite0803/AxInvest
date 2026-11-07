> 来源：导入自上游 AxAgent（基准 99c9b96f，2026-09-07） ｜ 导入日期：2026-11-04

# Unity P2：运行时权限覆盖 + 分类策略实装 + LLM 增强实施计划

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** 让 P1 产出的 `TaskShapeDecision` 真正影响系统行为（从仅展示变为权限/路由实装），并在模糊边界场景引入 LLM 辅助分类。

**Architecture:** 三层最小改动：① agent_query 入口按 `task_shape.isolation_need` 覆盖会话级 PermissionMode → ② cognitive_query 路由阶段按 `task_shape.recommended_strategy` 改写执行路径 → ③ `DefaultTaskShapeClassifier` 引入 Hybrid（规则优先 + LLM 兜底）模式，复用 harness 的 providers 抽象层调用 LLM。

**Tech Stack:** Rust 2024 (orchestrator / harness / commands) · async_trait · tokio · Sea-ORM (agent_profile 表) · reqwest (providers) · TypeScript (前端审批提示 UI 可选)

---

## Task 1: 运行时权限覆盖（task_shape → PermissionMode 映射）

### 1a. 映射函数（harness/task_shape.rs）

**Files:**
- Modify: `src-tauri/crates/harness/src/task_shape.rs`

**Step 1: 写失败测试**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use axagent_harness::runtime_types::permissions::PermissionMode;

    #[test]
    fn test_isolation_to_permission_mapping() {
        // 纯思考 → 不降级（维持原会话级 WorkspaceWrite）
        assert_eq!(
            SecurityIsolationNeed::ReadOnlyThought.to_permission_override(),
            None
        );
        // 工作区写入 → 不降级
        assert_eq!(
            SecurityIsolationNeed::WorkspaceWrite.to_permission_override(),
            None
        );
        // 强制沙箱 → Prompt（用户逐次确认，物理权限墙）
        assert_eq!(
            SecurityIsolationNeed::SandboxRequired.to_permission_override(),
            Some(PermissionMode::Prompt)
        );
        // 审批气隔 → Prompt + 后续审批门拦截（双层保险）
        assert_eq!(
            SecurityIsolationNeed::ApprovalAndAirGap.to_permission_override(),
            Some(PermissionMode::Prompt)
        );
    }

    #[test]
    fn test_resolve_effective_permission() {
        // 会话级 Allow + SandboxRequired → 降级到 Prompt（安全优先）
        assert_eq!(
            resolve_effective_permission(
                PermissionMode::Allow,
                Some(SecurityIsolationNeed::SandboxRequired)
            ),
            PermissionMode::Prompt
        );
        // 会话级 WorkspaceWrite + ReadOnlyThought → 维持 WorkspaceWrite（不升级）
        assert_eq!(
            resolve_effective_permission(
                PermissionMode::WorkspaceWrite,
                Some(SecurityIsolationNeed::ReadOnlyThought)
            ),
            PermissionMode::WorkspaceWrite
        );
        // 会话级 Prompt + 任意 → 维持 Prompt（用户主动最严格配置优先）
        assert_eq!(
            resolve_effective_permission(
                PermissionMode::Prompt,
                Some(SecurityIsolationNeed::ReadOnlyThought)
            ),
            PermissionMode::Prompt
        );
        // None（flag 关闭）→ 维持原样
        assert_eq!(
            resolve_effective_permission(PermissionMode::WorkspaceWrite, None),
            PermissionMode::WorkspaceWrite
        );
    }
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test -p axagent-harness task_shape 2>&1`
Expected: FAIL with `not found in this scope`

**Step 3: Write minimal implementation**

```rust
use crate::runtime_types::permissions::PermissionMode;

impl SecurityIsolationNeed {
    /// 按安全隔离需求推断「权限降级覆盖」。
    ///
    /// 返回 `None` 表示不需要覆盖（沿用会话级权限）；
    /// 返回 `Some(mode)` 表示强制使用该模式。
    ///
    /// ## 设计原则
    ///
    /// - **只降级不升级**：权限覆盖是安全防御性手段，绝不把只读提升为写入。
    ///   用户在会话级设置了 `Prompt`（最严格交互模式）时，必须保持用户配置优先。
    /// - `SandboxRequired` 及以上 → 强制 `Prompt`（逐次人机确认 = 物理权限墙）。
    #[must_use]
    pub fn to_permission_override(self) -> Option<PermissionMode> {
        match self {
            Self::ReadOnlyThought | Self::WorkspaceWrite => None,
            Self::SandboxRequired | Self::ApprovalAndAirGap => Some(PermissionMode::Prompt),
        }
    }
}

/// 解析「任务级权限」与「会话级权限」的最终生效模式。
///
/// 规则（单调函数，保证用户最严格配置不可被覆盖）：
/// 1. 若 `task_isolation` 为 None → 直接返回 session_mode（flag 关闭或分类失败回退）
/// 2. 调用 `to_permission_override()` 获取任务级建议
/// 3. 取 min(session_mode, suggested_override) — 哪个更严格用哪个（PermissionMode: PartialOrd）
#[must_use]
pub fn resolve_effective_permission(
    session_mode: PermissionMode,
    task_isolation: Option<SecurityIsolationNeed>,
) -> PermissionMode {
    let Some(isolation) = task_isolation else { return session_mode };
    let Some(suggested) = isolation.to_permission_override() else { return session_mode };
    // 严格的那个胜出（PartialOrd: ReadOnly 最小，Allow 最大）
    session_mode.min(suggested)
}
```

**Step 4: Run test to verify it passes**

Run: `cargo test -p axagent-harness task_shape 2>&1`
Expected: PASS 2 new tests

---

### 1b. agent_query 接入权限覆盖

**Files:**
- Modify: `src-tauri/src/commands/agent/mod.rs` (around line 1742-1756)

**Step 1: 定位权限解析段**

现有代码（约 L1742-L1756）：
```rust
let permission_mode_str = db_session
    .as_ref().map(|s| s.permission_mode.clone()).unwrap_or_else(|| "default".to_string());
let runtime_permission_mode = match permission_mode_str.as_str() { ... };
```

**Step 2: 在 runtime_permission_mode 赋值后，追加 task_shape 覆盖逻辑**

```rust
// P2: 按任务形态决策的「安全隔离需求」覆盖会话级权限模式（只降级不升级）。
// 当 UNITY_P0_TASK_SHAPE flag 关闭或分类失败时 task_shape 为 None，本块无操作。
if let Some(ts) = &request.task_shape {
    use axagent_harness::task_shape::resolve_effective_permission;
    let effective = resolve_effective_permission(
        runtime_permission_mode,
        Some(ts.isolation_need)
    );
    if effective != runtime_permission_mode {
        info!(
            "[agent_query] Permission overridden by task_shape: {:?} → {:?} (isolation={:?})",
            runtime_permission_mode, effective, ts.isolation_need
        );
        runtime_permission_mode = effective;
    }
}
```

**Step 3: 验证编译**

Run: `cargo check -p axagent --lib`（临时移除 android 字段 workaround）
Expected: PASS

---

## Task 2: 策略实装：按 ExecutionStrategy 分流（cognitive_query）

### 2a. ApprovalGate → 审计日志 + 审批标记

**Files:**
- Modify: `src-tauri/src/commands/cognitive.rs` (主流程 Agent 分支入口之前)

在执行 Agent 或 Workflow 前，检查 task_shape 命中 ApprovalGate 时写审计日志（实际阻断 P3 接入）。最小实现：日志留痕 + 前端 DecisionCard 已显示「审批门」标签。

**Step 1: 在 Agent 分支前追加 safety gate 日志**

```rust
// P2: 审批门策略命中 → 写审计日志（实际阻断 P3 接入 approval-hitl）
if let Some(ts) = &task_shape_decision {
    if matches!(ts.recommended_strategy, ExecutionStrategy::ApprovalGate) {
        warn!(
            conversation_id = %conversation_id,
            input_preview = %&input[..input.len().min(40)],
            evidence = ?ts.evidence,
            "🚨 ApprovalGate 命中：高危任务已打标，后续需接入实际审批阻断"
        );
    }
}
```

**Step 2: 验证**

Run: `cargo check -p axagent --lib`
Expected: PASS

---

### 2b. DelegateSingleExpert → 自动覆盖 expert_id

**Files:**
- Modify: `src-tauri/src/commands/cognitive.rs` (主流程 agent_profile 赋值)

**Step 1: 在 expert_id 确定后追加 task_shape 覆盖逻辑**

在构造 `agent_request` 前约 L1080-L1090 处插入：

```rust
// P2: 任务形态建议 DelegateSingleExpert + 用户未显式指定 expert → 用分类器建议覆盖
let effective_expert_id = if request.agent_profile_id.is_none() && request.expert_id.is_none() {
    task_shape_decision.as_ref().and_then(|ts| {
        if let ExecutionStrategy::DelegateSingleExpert { expert_id: expert_from_shape }
            = &ts.recommended_strategy
        {
            info!(
                "[cognitive_query] task_shape suggests expert: {}",
                expert_from_shape
            );
            Some(expert_from_shape.clone())
        } else { None }
    }).or(expert_id)
} else { expert_id };
```

**Step 2: 构造 agent_request 时用 effective_expert_id 替换 expert_id**

**Step 3: 验证编译**

Run: `cargo check -p axagent --lib`
Expected: PASS

---

## Task 3: LLM 增强分类器（Hybrid 模式：规则 → LLM 兜底）

### 3a. harness 中定义 LLM 分类器 trait（consumer-only）

**Files:**
- Modify: `src-tauri/crates/harness/src/task_shape.rs`
- Modify: `src-tauri/crates/harness/src/lib.rs`（re-export）

新增 trait：

```rust
/// LLM 任务形态分类器 — 用于规则置信度不足时的兜底判断。
///
/// 由 wiring 层（runtime / commands）实现，调用 providers 抽象层发起轻量 LLM 请求。
/// 保持在 harness 定义不引任何 implementor 依赖（consumer-only, foundation tier）。
///
/// ## 调用契约
///
/// 输入：用户原始输入 + 已匹配证据（规则层产出）
/// 输出：若能做出更精确判断则 `Some(TaskShapeDecision)`，否则 `None` 回退到规则层结论。
#[async_trait::async_trait]
pub trait TaskShapeLlmClassifier: Send + Sync {
    async fn refine(
        &self,
        user_input: &str,
        rule_result: &TaskShapeDecision,
    ) -> Result<Option<TaskShapeDecision>, String>;
}
```

### 3b. orchestrator 中扩展 classify_hybrid (async wrapper)

**Files:**
- Modify: `src-tauri/crates/orchestrator/src/task_shape_classifier.rs`
- Modify: `src-tauri/crates/orchestrator/src/lib.rs`（re-export）

新增：
```rust
/// 混合分类入口：规则优先 + 规则置信度低于阈值时调用 LLM 分类器（若提供）。
///
/// 置信度阈值判定（任一满足即标记为「模糊」，触发 LLM 兜底）：
/// - `merge_score` 和 `split_score` 差值 < 0.2（两种倾向接近）
/// - 关键词证据 < 2 条（证据稀疏）
pub async fn classify_hybrid(
    user_input: &str,
    active_permission: PermissionMode,
    llm_classifier: Option<&dyn TaskShapeLlmClassifier>,
) -> Result<TaskShapeDecision, String> {
    let rule_result = classify_input(user_input, active_permission);
    let is_ambiguous = {
        let diff = (rule_result.merge_score - rule_result.split_score).abs();
        diff < 0.2 || rule_result.evidence.len() < 2
    };
    if !is_ambiguous { return Ok(rule_result); }
    let Some(llm) = llm_classifier else { return Ok(rule_result) };
    match llm.refine(user_input, &rule_result).await {
        Ok(Some(refined)) => Ok(refined),
        Ok(None) => Ok(rule_result),
        Err(e) => {
            tracing::warn!(error = %e, "LLM 分类失败，回退规则结论");
            Ok(rule_result)
        }
    }
}
```

**设计约束**：`TaskShapeLlmClassifier` 实现在 P3（wiring 层 runtime crate），P2 不引入 providers → orchestrator 反向依赖（违反架构铁律）。

---

## Task 4: 验证矩阵

| 验证项 | 命令 | 预期 |
|---|---|---|
| harness task_shape 单测 | `cargo test -p axagent-harness task_shape 2>&1` | 新增 2 个通过 |
| orchestrator 单测 | `cargo test -p axagent-orchestrator task_shape 2>&1` | 9+ 通过 |
| workspace check | `cargo check --workspace --exclude axagent --exclude axagent-mobile 2>&1` | PASS |
| axagent 主 check | `cargo check -p axagent --lib 2>&1`（临时移除 android 字段） | PASS |
| TS typecheck | `npm run typecheck 2>&1` | PASS |
| 格式化 | `cargo fmt -- --check` + `npm run format` | PASS |
| 错误码对齐 | `node scripts/check-errorcode-alignment.mjs` | PASS |
