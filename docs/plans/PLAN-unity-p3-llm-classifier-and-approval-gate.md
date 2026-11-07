> 来源：导入自上游 AxAgent（基准 99c9b96f，2026-09-07） ｜ 导入日期：2026-11-04

# P3：LLM 分类器实装 + ApprovalGate 真实阻断 + 端到端串联

## 目标

P2 已完成 trait 定义和 `classify_hybrid` 入口，但三个关键缺口仍在：
1. `TaskShapeLlmClassifier` trait **零实现**——LLM 兜底链路是空壳
2. ApprovalGate 命中后只打 `tracing::warn!` 日志——无真实阻断
3. `cognitive.rs` 仍调用 `classify_input`（同步规则版）——`classify_hybrid` 未被串联

P3 一次性补齐这三块，形成完整的「规则分类 → LLM 兜底 → 权限覆盖 → 审批阻断」端到端链路。

---

## Task 1：LLM 分类器适配器（runtime/wiring crate）

### 新建文件：`crates/runtime/src/task_shape_llm.rs`

**参考模板**：`src-tauri/src/commands/fleet/executor.rs:153-170`（`ProviderFleetIntentLlm`）

```rust
pub struct ProviderTaskShapeLlmClassifier {
    harness: RuntimeHarness,
}

impl ProviderTaskShapeLmClassifier {
    pub fn new(harness: RuntimeHarness) -> Self { Self { harness } }
}

#[async_trait::async_trait]
impl TaskShapeLlmClassifier for ProviderTaskShapeLlmClassifier {
    async fn refine(&self, user_input: &str, rule_result: &TaskShapeDecision)
        -> Result<Option<TaskShapeDecision>, String>
    {
        // 1. 构造 system prompt：输出 JSON { context_cost, isolation_need, evidence, ... }
        // 2. 通过 harness.get_adapter("openai") 取适配器
        // 3. 构造 ChatRequest（非流式、无工具、max_tokens=256）
        // 4. 调用 adapter.chat() → 解析 response.content 为 JSON
        // 5. 反序列化为 TaskShapeDecision → Ok(Some(refined))
        // 6. 任何失败 → Ok(None)（回退规则，绝不阻断）
    }
}
```

### LLM Prompt 设计

System prompt 要求 LLM 输出严格的 JSON：
```json
{
  "context_cost": "ephemeral|resumable|long_running_project|persistent_journey",
  "isolation_need": "read_only_thought|workspace_write|sandbox_required|approval_and_air_gap",
  "evidence": ["reason1", "reason2"],
  "merge_score": 0.0,
  "split_score": 0.0
}
```

LLM 只需判断两轴维度 + 给理由，`recommended_strategy` 由代码按 `decide_strategy()` 重新推导（避免 LLM 不熟悉策略映射）。

### 改动文件

| 文件 | 改动 |
|---|---|
| `crates/runtime/src/task_shape_llm.rs` | **新建**：适配器实现 + 单元测试 |
| `crates/runtime/src/lib.rs` | `pub mod task_shape_llm;` + `pub use task_shape_llm::ProviderTaskShapeLlmClassifier;` |

---

## Task 2：ApprovalGate 真实阻断

### 设计决策：复用 ChannelPermissionPrompter 模式

不新建审批表/DAO/状态机——ApprovalGate 是**会话级即时阻断**（不是工作流节点的持久化审批），复用现有 `ChannelPermissionPrompter` 的 mpsc 通道模式最轻量。

### 实现方案

**新增 AppState 字段**：
```rust
pub task_shape_approval_senders: Arc<Mutex<HashMap<String, oneshot::Sender<bool>>>>,
```
key = `approval_id`（格式 `{conversation_id}-{timestamp}`），value = oneshot Sender<bool>（true=批准, false=拒绝）。

**cognitive.rs Agent 分支前插入 barrier**：
```rust
if let Some(ts) = &task_shape_decision {
    if matches!(ts.recommended_strategy, ExecutionStrategy::ApprovalGate) {
        let approval_id = format!("{}-{}", conversation_id, chrono::Utc::now().timestamp_millis());
        let (tx, rx) = tokio::sync::oneshot::channel();
        state.task_shape_approval_senders.lock().await.insert(approval_id.clone(), tx);
        app.emit("task-shape-approval-request", json!({
            "approval_id": approval_id,
            "conversation_id": conversation_id,
            "input_preview": &input.chars().take(80).collect::<String>(),
            "evidence": &ts.evidence,
            "merge_score": ts.merge_score,
            "split_score": ts.split_score,
        }));
        let approved = rx.await.unwrap_or(false);
        state.task_shape_approval_senders.lock().await.remove(&approval_id);
        if !approved {
            return Ok(CognitiveQueryResponse { /* rejected status */ });
        }
    }
}
```

**新增 Tauri 命令**：`respond_task_shape_approval(approval_id, approved) → Result<(), String>`
- 从 `task_shape_approval_senders` 取 Sender，发送 bool

### 改动文件

| 文件 | 改动 |
|---|---|
| `src-tauri/src/app_state.rs` | 新增 `task_shape_approval_senders` 字段 |
| `src-tauri/src/init/state.rs` | 初始化 `task_shape_approval_senders: Arc::new(Mutex::new(HashMap::new()))` |
| `src-tauri/src/commands/cognitive.rs` | Agent 分支前插入审批 barrier |
| `src-tauri/src/commands/cognitive.rs` | 新增 `respond_task_shape_approval` 命令 |
| `src-tauri/src/commands/mod.rs` | 声明命令（若 cognitive 不在 mod.rs） |
| `src-tauri/src/lib.rs` | `generate_handler![]` 注册命令 |

---

## Task 3：AppState 注入 + cognitive_query 串联 classify_hybrid

### 改动文件

| 文件 | 改动 |
|---|---|
| `src-tauri/src/app_state.rs` | 新增 `task_shape_llm_classifier: Arc<dyn TaskShapeLlmClassifier>` 字段 |
| `src-tauri/src/init/state.rs` | 实例化 `ProviderTaskShapeLlmClassifier::new(harness.clone())` 注入 |
| `src-tauri/src/commands/cognitive.rs` | `classify_task_shape` 改为 async + 调用 `classify_hybrid` |

### cognitive.rs 关键改动

```rust
// 从同步改为异步
async fn classify_task_shape(
    state: &AppState,
    input: &str,
    options: Option<&AgentOptions>,
) -> Option<TaskShapeDecision> {
    if !UNITY_P0_TASK_SHAPE_ENABLED { return None; }
    let permission = /* 从 options 取 */;
    let decision = axagent_orchestrator::classify_hybrid(
        input,
        permission,
        Some(&*state.task_shape_llm_classifier),
    ).await.ok()?;
    Some(decision)
}
```

调用点改为 `.await`。

---

## Task 4：验证矩阵

- `cargo test -p axagent-runtime --lib task_shape_llm`（适配器单测）
- `cargo test -p axagent-harness -p axagent-orchestrator --lib task_shape`（回归）
- `cargo check -p axagent-runtime -p axagent`
- `cargo fmt --all --check`
- `npm run typecheck`
- `npm run format`

---

## Feature Flag

- `UNITY_P0_TASK_SHAPE_ENABLED` 保持 `false`（P3 不开启，P4 验证后开启）
- 审批 barrier 仅在 flag 开启时生效（内层 if 守卫）
- LLM 分类器即使注入 AppState，flag 关闭时也不会被调用

---

## AGENTS.md 铁律合规

| 铁律 | 合规 |
|---|---|
| 4 不允许重复类型体系 | ✅ 实现 trait 而非重定义 |
| 6 新增 crate 声明归属 | ✅ 不新增 crate，runtime 已是 wiring |
| 8 异步锁 tokio::sync | ✅ oneshot/mpsc 都是 tokio |
| 12 禁止重复定义 | ✅ 复用 harness/orchestrator 既有定义 |
| 13 DTO camelCase | ✅ emit 事件 JSON 用 camelCase |
