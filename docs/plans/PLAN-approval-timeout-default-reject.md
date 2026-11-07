> 来源：导入自上游 AxAgent（基准 99c9b96f，2026-09-07） ｜ 导入日期：2026-11-04

# 改造清单 · 审批超时默认拒绝

> 状态：待评审（未实施）
> 目标：审批超时后，按 `timeout_action` 裁决——默认 **拒绝**（停摆任务），配置 `auto_approve` 才放行——且拒绝必须真正停摆、可追溯、汇入汇报。
> 关联设计稿：`docs/夜间长时自主任务运行-详细设计.md` P1-⑤

---

## 一、事实基线（已核实）

| 环节 | 文件 | 现状 |
|------|------|------|
| timeout_action 定义 | `harness/src/workflow_types.rs` `ApprovalNodeConfig.timeout_action` | ✅ 存在，默认 `auto_reject` |
| 读入请求 | `rt-workflow/.../executors/approval_executor.rs:56` | ✅ 已读入 `ApprovalRequest.timeout_action` |
| 引擎挂起 | `rt-workflow/.../engine/mod.rs:2551-2670` | ✅ Suspend + Paused + 超时自动 cancel |
| 落库 | `engine/mod.rs:2572-2589` + `dao/repo/workflow_approval.rs:save_approval` | ❌ 未落库 `timeout_action` |
| 超时裁决 | `dao/repo/workflow_approval.rs:auto_resolve_timeouts:162-193` | ❌ 仅标 `expired`，不裁决 |
| 审批人操作 | `commands/work_engine.rs:resume_approval:673` / `cancel_approval:734` | ✅ approved→resume / rejected→cancel |

**发现的核心缝隙**：`timeout_action` 在两处产生（定义、读入），却没进持久化、没进裁决，等于"半成品字段"。

---

## 二、改动项清单

### 1. 数据模型：扩 `workflow_approvals` 表（migration）
- `dao/src/repo/workflow_approval.rs` 的 `WorkflowApprovalRecord` 加字段 `timeout_action: String`（默认 `"auto_reject"`）。
- migration：`ALTER TABLE workflow_approvals ADD COLUMN timeout_action TEXT NOT NULL DEFAULT 'auto_reject';`
- 更新 `row_to_record`（加 `try_get`）、`save_approval`（INSERT 加列）。
- 遵守 AGENTS 铁律 #10：表结构变更写 migration，不改 entity 了事。

### 2. 落库：引擎侧写入 `timeout_action`
- `rt-workflow/.../engine/mod.rs:2572-2589` 构造 `WorkflowApprovalRecord` 时：
```rust
timeout_action: approval.timeout_action.clone(),
```
- `approval_executor.rs` 构造 `ApprovalRequest` 时确保 `timeout_action` 已带（已是 `c.timeout_action`，可保留）。

### 3. 裁决逻辑：重写 `auto_resolve_timeouts`
- 目标：把「仅标 expired」改为「按 `timeout_action` 裁决 + 返回待联动列表」。
- 新增统一裁决入口（DAO 层返回决策结果，不直接碰引擎）：
```rust
pub enum TimeoutResolution { Rejected { execution_id, node_id }, Approved { execution_id, node_id }, AlreadyResolved }

pub async fn auto_resolve_timeouts(db, now_ms) -> Result<Vec<TimeoutResolution>, String> {
    // 1) 取超时的 pending 记录
    // 2) 逐条按 timeout_action 分支：
    //    auto_reject(默认) → status='rejected', decision='rejected',
    //                          comment='审批超时，默认拒绝'
    //    auto_approve      → status='approved', decision='approved'
    //    (保留 expired 状态作为中间态记录超时事实，再叠加 decision)
    // 3) 返回 decisions，供命令层联动引擎
}
```
- 保留 `expired` 语义：超时这一事实仍要可追溯（透明化），但决策结果落到 `decision`。

### 4. 命令层联动：超时后真正停摆/放行
- `commands/work_engine.rs` 现有 `list_pending_approvals:629` 已调 `auto_resolve_timeouts`。
- 增加统一的「裁决后联动」函数（复用 `resume_approval` 的引擎动作）：
```rust
async fn apply_timeout_resolution(state, resolutions) {
    for r in resolutions {
        match r {
            Rejected { execution_id, .. } => engine.cancel(execution_id).await,  // 停摆
            Approved  { execution_id, .. } => engine.resume_breakpoints(execution_id).await, // 放行
        }
    }
}
```
- 在 `list_pending_approvals`、以及任何轮询审批的入口调用它，保证"超时即停摆"落地。

### 5. 竞态处理：引擎 timeout 与 DAO swipe 统一入口
- 现状有两条超时路径可能同时触发：
  - 引擎内 `tokio::time::timeout`（`mod.rs:2621`）→ 当前 `break` 退出、标 expired。
  - `auto_resolve_timeouts`（轮询触发）→ 标 expired。
- 改造：引擎超时分支（`mod.rs:2630-2667`）改为**调用同一个裁决函数**（或至少读 `timeout_action` 决定 cancel/resume），并保证 `resolve_approval` 的 `WHERE status='pending'` 天然幂等——谁先到谁生效，后者改 0 行即视为已处理。
- 引擎侧补 `cancel` 而非裸 `break`：拒绝时必须真正停摆（写 Cancelled + 收口 background_tasks）。

### 6. 一致性：`resume_approval` / `cancel_approval` 无需改逻辑
- 已正确：approved→resume、rejected→cancel、cancelled→记录。超时裁决复用其引擎动作即可，避免重复。

### 7. 前端（camelCase 对齐）
- `types/approval.ts` 的 `ApprovalRequest` 加 `timeoutAction`（消费 camelCase）。
- 审批面板（`approvalStore.ts` / 相关组件）展示超时策略与超时时间；本地化文案 11 语言同步（AGENTS 铁律#2）。

### 8. 汇报并入（呼应⑦）
- 长任务报告中从 `workflow_approvals` 读 `decision='rejected'`（含超时拒）记录，汇总"超时拒绝"项。

---

## 三、命令注册（AGENTS 铁律#7：两步）

- 若新增独立命令（如 `resolve_all_timeouts` 以主动触发裁决）：
  - `src-tauri/src/commands/mod.rs`：声明模块/命令。
  - `src-tauri/src/lib.rs` 或 `register_commands.rs`：`generate_handler![...]` 注册。
- 若仅改 `list_pending_approvals` 内部联动，则无新命令，仅改内部实现（推荐，避免接口膨胀）。

---

## 四、测试要点
- DAO 单测：`auto_resolve_timeouts` 对 `auto_reject`/`auto_approve`/已过期时间三种输入，断言状态、decision、comment。
- 集成：审批节点挂起 → 不操作 → 超时 → 断言工作流 Cancelled 或 Approved 后继续。
- 幂等：双路径并发触发，断言只处理一次（rows_affected=0 分支）。
- 兼容：无 `timeout_action` 的旧记录读取不 panic（默认 `auto_reject`）。

---

## 五、风险 / 待确认
1. **默认值安全取向**：`auto_reject` 是安全取向，符合"超时默认拒绝"。确认是否允许某类低危审批显式配置 `auto_approve`。
2. **`expired` 状态**：作为中间态保留（记录超时事实）+ `decision` 承载裁决，避免丢追溯信息。
3. **引擎 `break` 改 `cancel`** 的连带影响：需验证 `cancel` 是否清理 loop_checkpoint / background_tasks（`mod.rs:3584` 已含 loop_checkpoint 清理）。
4. **调用点**：`list_pending_approvals` 是唯一轮询入口触进度扫描，需确认是否还需在定时任务里主动触发。

---

## 六、落地顺序
1. migration + `WorkflowApprovalRecord` 字段 + `save_approval`/`row_to_record`（P0）
2. 引擎落库 `timeout_action`（P0）
3. 重写 `auto_resolve_timeouts` 裁决 + 命令层联动（P1）
4. 引擎超时分支统一（P1）
5. 前端 + 汇报并入（P2）
6. 测试
