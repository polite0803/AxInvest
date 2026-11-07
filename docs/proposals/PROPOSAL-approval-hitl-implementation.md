> 来源：导入自上游 AxAgent（基准 99c9b96f，2026-09-07） ｜ 导入日期：2026-11-04

# ApprovalNode 可挂起人工介入（HITL）门 — 具体实现方案

> 生成时间：2026-07-06
> 目标：将 `ApprovalNode` 从占位（`{"status":"pending"}`）改造为**真正挂起工作流、等待外部人工审批/信号后恢复**的节点，对齐 n8n 2.x 的「工具级 human review + 跨渠道路由审批」。
> 核心思路：**复用引擎已有的 `Paused` + `pause_signal` + `resume_*` 机制**，把 Loop 执行器里已验证的 approval 中断模式泛化到顶层 ApprovalNode。

---

## 一、现状与可行性（已回源码核验）

| 事实 | 位置 | 结论 |
|---|---|---|
| `ApprovalExecutor::execute` 仅返回 `{"status":"pending"}`，无挂起/恢复 | `crates/rt-workflow/src/work_engine/executors/approval_executor.rs:28` | 占位，须改造 |
| `ApprovalNodeConfig` 已含 `message/approver/timeout_secs/timeout_action/output_var` | `crates/harness/src/workflow_types.rs:996` | 配置已就绪，执行器没用 |
| `ExecutionStatus::Paused` 枚举存在 | `execution_state.rs:13` | 暂停状态已支持 |
| `pause_signal: Option<Arc<Notify>>` + `resume_breakpoints`/`step_breakpoint` | `execution_state.rs:222`、`engine/mod.rs:352/371` | 顶层挂起/恢复底座已存在 |
| 断点挂起写法：`state.status=Paused` + `exec_pause_signal.notified().await` | `engine/mod.rs:1788-1799` | 直接复用此模式 |
| Loop 里已有 approval 中断：`detect_interrupt` + `LoopCheckpoint.pending_approval_node` + 24h 超时 `sig.notified().await` | `executors/loop_executor.rs:296-389` | **模式已被生产验证**，本方案泛化它 |
| `ExecutionState.callbacks` 已有 `loop_checkpoint` 回调（引擎注入 db 操作，executor 不持 db） | `execution_state.rs:94-95,142-172` | Approval 持久化/派发用同样回调模式 |
| `ExecutionState.credential_manager: Option<Arc<CredentialManager>>` 已存在 | `execution_state.rs:238` | 凭证保险库底座已在 harness，见 §七 |
| `NodeOutput { output, output_var }` 当前无 control 字段 | `node_executor_trait.rs:11` | 需加 `control: Option<NodeControl>` |
| 命令注册两步走：`commands/mod.rs` 声明 + `lib.rs` `generate_handler![]` | AGENTS.md 禁区 #7 | 新增命令必须两步 |

---

## 二、架构总览

```
[ApprovalNode 执行]
   ApprovalExecutor::execute
     → 生成 resume_token + ApprovalRequest(消息/审批人/渠道/超时)
     → 返回 NodeOutput { control: Some(Suspend{...}) }

[引擎 run_workflow 结果收集循环]
   检测到 NodeControl::Suspend
     → callbacks.approval.request(req)        // 持久化 workflow_approvals + 派发到 9 平台网关
     → state.status = Paused; paused_at = node_id
     → persist_execution_state(state)         // SQLite，前端可查"等待审批"
     → exec_pause_signal.notified().await     // 阻塞当前 future（复用断点模式）

[人工在 UI / Slack / 邮件 点击审批]
   resume_approval(execution_id, node_id, decision, approver, comment, token)
     → 校验 token + 状态=pending
     → 更新 workflow_approvals → resolved
     → state.node_outputs[node_id] = decision_json   // 下游 $node["Approval"] 可读
     → self.resume_breakpoints(execution_id)         // notify pause_signal + 置 Running

[引擎从 await 处继续]
   → 下游节点用审批决策作为输入继续执行
```

> **复用而非新造**：不新增独立的挂起 Future 管理体系，直接走 `pause_signal` + `resume_breakpoints`，与断点、Loop 中断同构，降低风险。

---

## 三、数据模型

### 3.1 新增 `NodeControl` 与 `ApprovalRequest`（`node_executor_trait.rs`）

```rust
/// 节点对引擎发出的控制指令（可选，默认 None = 正常完成）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum NodeControl {
    /// 挂起整个工作流，等待外部审批/信号后恢复
    Suspend {
        resume_token: String,
        approval: ApprovalRequest,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApprovalRequest {
    pub execution_id: String,
    pub node_id: String,
    pub title: String,
    pub message: String,
    pub approver: Option<String>,     // "role:finance" / "user:alice" / None=任意
    pub channels: Vec<String>,        // "slack","email","im","in_app"
    pub payload: serde_json::Value,   // 透传给审批人查看的上下文
    pub timeout_secs: u64,
    pub timeout_action: String,       // "auto_approve" | "auto_reject" | "keep_waiting"
}

// NodeOutput 增加字段：
pub struct NodeOutput {
    pub output: serde_json::Value,
    pub output_var: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub control: Option<NodeControl>,
}
```

### 3.2 新增实体 `workflow_approvals`（SQLite / Sea-ORM）

```rust
// crates/entities/src/workflow_approval.rs
#[derive(DeriveEntityModel, Serialize, Deserialize)]
#[sea_orm(table_name = "workflow_approvals")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: String,
    pub execution_id: String,
    pub node_id: String,
    pub status: String,            // "pending" | "approved" | "rejected" | "expired"
    pub resume_token: String,
    pub title: String,
    pub message: String,
    pub approver: Option<String>,
    pub channels: String,          // JSON 数组
    pub payload: String,           // Text：透传上下文
    pub decision: Option<String>,  // Text：审批人填的结果(JSON)
    pub approver_actual: Option<String>,
    pub comment: Option<String>,
    pub timeout_secs: i64,
    pub expires_at: i64,
    pub created_at: i64,
    pub resolved_at: Option<i64>,
}
```

- 遵循 AGENTS.md「改表结构必须写 migration」：在 `crates/migration/src` 新增 migration 建表。
- 新增 `crates/entities/src/workflow_approval.rs` + 在 `entities/src/lib.rs` re-export；新增 repo `axagent_core::repo::workflow_approval`（save/get_by_token/list_pending/get_by_execution）。

---

## 四、执行器改造（`approval_executor.rs`）

```rust
async fn execute(&self, node: &WorkflowNode, ctx: &ExecutionState) -> Result<NodeOutput, NodeError> {
    let WorkflowNode::Approval(n) = node else {
        return Err(NodeError::type_mismatch("approval", self.node_type()));
    };
    let c = &n.config;
    let resume_token = uuid::Uuid::new_v4().to_string();
    let channels = resolve_channels(&c.approver); // 由 approver 推断渠道（role:*→im+email，user:*→in_app+email）
    let approval = ApprovalRequest {
        execution_id: ctx.execution_id.clone(),
        node_id: n.base.id.clone(),
        title: n.base.title.clone(),
        message: c.message.clone(),
        approver: c.approver.clone(),
        channels,
        payload: serde_json::json!({ "variables": ctx.variables, "input": ctx.input_params }),
        timeout_secs: c.timeout_secs,
        timeout_action: c.timeout_action.clone(),
    };
    Ok(NodeOutput {
        output: serde_json::json!({ "status": "pending", "node_id": n.base.id }),
        output_var: if c.output_var.is_empty() { None } else { Some(c.output_var.clone()) },
        control: Some(NodeControl::Suspend { resume_token, approval }),
    })
}
```

> 注意：`ApprovalExecutor` 仍然是 **stateless**，不碰 db；持久化/派发一律走引擎注入的 `ApprovalOps` 回调（见 §五）。

---

## 五、引擎接入（`engine/mod.rs`）

### 5.1 新增 `ApprovalOps` 回调（仿 `LoopCheckpointOps`，`execution_state.rs`）

```rust
pub type ApprovalRequestFn = Arc<dyn Fn(ApprovalRequest)
    -> Pin<Box<dyn Future<Output = Result<(), String>> + Send>> + Send + Sync>;
pub type ApprovalResolveFn = Arc<dyn Fn(String, ApprovalDecision)
    -> Pin<Box<dyn Future<Output = Result<(), String>> + Send>> + Send + Sync>;

#[derive(Clone)]
pub struct ApprovalOps { pub request: ApprovalRequestFn, pub resolve: ApprovalResolveFn }

// ExecutionContextCallbacks 增加字段：
pub approval: Option<ApprovalOps>,
```

引擎在 `run_workflow` 构造 `exec_ctx.callbacks` 处（参照 `loop_checkpoint` 注入点 `engine/mod.rs:1938-1948`）：
```rust
let approval_ops = ApprovalOps {
    request: Arc::new(|req| {
        let db = self.db.clone();
        let gateway = self.gateway.clone(); // 9 平台消息网关
        Box::pin(async move {
            // 1. 写 workflow_approvals (status=pending)
            // 2. 按 req.channels 通过 gateway 派发审批请求（含 deep link / 按钮）
            Ok(())
        })
    }),
    resolve: Arc::new(|execution_id, decision| { /* 更新 workflow_approvals + 写回 */ Box::pin(...) }),
};
// 注入到 callbacks.approval
```

### 5.2 run_workflow 结果收集循环挂起处理（插入点：`engine/mod.rs` ~2598 之后的 `NodeResult` 处理处）

```rust
// 收集到某节点结果后：
if let Some(ctrl) = node_result.dispatch_result.as_ref().ok().and_then(|o| o.control.clone()) {
    if let NodeControl::Suspend { resume_token, approval } = ctrl {
        // 1. 持久化 + 派发
        if let Some(ops) = exec_ctx.callbacks.as_ref().and_then(|cb| cb.approval.clone()) {
            if let Err(e) = (ops.request)(approval.clone()).await {
                tracing::error!("审批请求派发失败: {e}");
            }
        }
        // 2. 标记暂停（复用断点写法）
        {
            let mut executions = self.executions.lock().await;
            if let Some(state) = executions.get_mut(&execution_id) {
                state.status = ExecutionStatus::Paused;
                state.current_node_id = Some(approval.node_id.clone());
                state.paused_at = Some(approval.node_id.clone());
                self.persist_execution_state(state).await; // SQLite，前端可查
            }
        }
        // 3. 阻塞当前 future，等待 resume_approval 通知（同断点模式）
        exec_pause_signal.notified().await;
        // 4. 恢复后 resume_approval 已把决策写入 state.node_outputs[node_id]，继续下游
        continue;
    }
}
```

> **并发安全性**：`run_workflow` 用 `join_set` 并发派发就绪节点。ApprovalNode 作为网关节点，下游依赖其输出，调度器会在其完成前不派发下游。挂起期间 `state.status=Paused`，主循环应在 `status==Paused` 时停止派发新节点（与 §六 终端状态检查同构），只对 `resume_approval` 放行。

---

## 六、新增命令（`src-tauri/src/commands/`）

遵循 AGENTS.md 两步注册：`commands/mod.rs` 声明 + `lib.rs` `generate_handler![]`。

| 命令 | 签名（要点） | 行为 |
|---|---|---|
| `resume_approval` | `(execution_id, node_id, decision: ApprovalDecision, approver, comment, token)` | 校验 token + `workflow_approvals.status==pending`；写决策；`state.node_outputs[node_id]=decision`；`self.resume_breakpoints(execution_id)` |
| `list_pending_approvals` | `(filter?: {execution_id?, approver?})` | 读 `workflow_approvals` 中 `status=pending`；**顺带检查 `expires_at`，超时按 `timeout_action` 自动裁决**（复用 Loop 24h 超时思想） |
| `cancel_approval` | `(execution_id, node_id, token)` | 置 `rejected`(cancelled) + 触发工作流取消 |

`ApprovalDecision`：
```rust
#[derive(Serialize, Deserialize)]
pub enum ApprovalDecision { Approve(serde_json::Value), Reject { reason: String } }
```

> **超时裁决**：在 `resume_approval` / `list_pending_approvals` 入口统一检查 `expires_at`；若超时：
> - `timeout_action == "auto_approve"` → 以空/默认 payload 调用 `Approve`
> - `timeout_action == "auto_reject"` → `Reject{reason:"timeout"}`
> - `timeout_action == "keep_waiting"` → 维持 pending（Loop 用 24h 硬超时防永久挂起，这里也建议加硬上限）

---

## 七、凭证保险库（并行 P0，底座已存在）

`ExecutionState.credential_manager: Option<Arc<axagent_harness::credential::CredentialManager>>` 已在 harness 存在。缺口是**UI + 节点接线**：
1. 新增 `credentials` 表 + `CredentialVault` crate（apiKey/oauth2/basic，AES 加密）。
2. `HttpRequestNode` / `EmailNode` 配置增加 `credential_id`，执行时经 `ctx.credential_manager.get(credential_id)` 懒加载解密。
3. 前端 `Settings` 增加「凭证」页，一处配置多处复用。
4. （企业扩展）预留外部密钥提供者接口（对齐 n8n 2.x 的 1Password/项目级外部密钥）。

> 与 HITL 解耦，可独立排期；但两者同为 P0，建议同期落地。

---

## 八、前端改动

| 模块 | 改动 |
|---|---|
| `src/stores/feature/approvalStore.ts`（新建） | `listPendingApprovals()`、`resumeApproval()`、`cancelApproval()`，轮询或订阅 |
| `src/components/workflow/ApprovalPanel.tsx`（新建） | 列出待审批项：消息 + 上下文 + 通过/拒绝按钮（调 `resume_approval`） |
| `ExecutionHistoryPanel` | 状态显示 `Paused` + 「等待审批」徽标，点击跳转到 ApprovalPanel |
| 编辑器属性面板 `ApprovalNode` | 暴露 `approver`/`timeout_secs`/`timeout_action`/`channels`（已存在于 config，仅前端未接） |
| 9 平台网关消息体 | 审批消息带 deep link / 回调按钮，点击即 `resume_approval` |

---

## 九、实施步骤（建议顺序）

1. **数据层**：`workflow_approvals` 实体 + repo + migration（建表）。
2. **控制流**：`node_executor_trait.rs` 加 `NodeControl`/`ApprovalRequest`/`NodeOutput.control`。
3. **回调**：`execution_state.rs` 加 `ApprovalOps`；引擎在 `run_workflow` 注入（仿 `loop_checkpoint`）。
4. **执行器**：`approval_executor.rs` 返回 `Suspend`。
5. **引擎挂起**：`run_workflow` 结果收集循环加 Suspend 分支（§5.2）。
6. **命令**：`resume_approval` / `list_pending_approvals` / `cancel_approval` + 两步注册。
7. **超时裁决**：`list_pending_approvals` 入口统一过期处理。
8. **前端**：`approvalStore` + `ApprovalPanel` + 历史面板徽标 + 网关消息体。
9. **测试**：新增 `approval_executor_integration.rs`（挂起→注入决策→恢复→下游读到决策）；并扩 `loop_executor_integration.rs` 现有 approval 中断测试，验证两种路径一致。

---

## 十、风险与对策

| 风险 | 对策 |
|---|---|
| 并发派发下挂起导致下游误执行 | 挂起时置 `Paused` 并停止 `join_set` 新派发；下游依赖 ApprovalNode 输出，调度器本就不前置派发 |
| 进程重启丢失 `pause_signal` future | 本地优先桌面场景与现有断点行为一致（future 在内存）；`workflow_approvals` + `persist_execution_state` 已落库，可做重启 rehydrate（增强项，先记录 TODO） |
| 审批人失联永久挂起 | `timeout_action` + 硬上限（参考 Loop 24h）双保险 |
| token 泄露被冒审批 | `resume_token` 用 UUID/`Uuid::new_v4`，存库比对；可选加 approver 身份校验 |
| 与 Loop 内 approval 中断重复 | 顶层 `ApprovalNode` 走引擎级 Suspend；Loop 内 approval 仍走 `detect_interrupt`；两者在 `workflow_approvals` 表统一记录，避免双套存储 |

---

## 十一、验证清单

- [ ] `ApprovalNode` 执行后 `workflow_executions.status == "paused"`，`workflow_approvals` 出现 pending 行
- [ ] UI/网关收到审批请求，点击通过后下游节点用决策继续，最终 `status == "completed"`
- [ ] 拒绝后走错误/补偿分支（配合 §五 后续可接逐节点 error port）
- [ ] `timeout_action=auto_reject` 且超时，`list_pending_approvals` 自动置 rejected 并恢复/终止
- [ ] `cancel_approval` 正确取消工作流
- [ ] `cargo fmt` + `cargo clippy -D warnings` 通过；`approval_executor_integration` 测试通过
```
