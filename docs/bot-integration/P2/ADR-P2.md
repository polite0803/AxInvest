# ADR-P2 — P2-a 架构决策记录

> 状态：accepted（P2-a 范围内）｜ 日期：2026-11-23 ｜ 前置：`ADR.md`（P1 ADR-001..009，编号顺延）

## ADR-P2-1 — Bot 审批级别的解析位置与合并规则

**决策**：在命令层 `agent_query` 的现有三档映射点（`src-tauri/src/commands/agent/mod.rs:2109-2137`）上游插入一个纯函数 `resolve_bot_approval_level(bot: &Bot, session_mode: Option<&str>) -> PermissionMode`，落点新建 `src-tauri/src/bot_approval.rs`。规则按 OpenMausBot 原义（非 P1 GAP §2.3 的 max 转述）：

1. **线程自设直接生效**：会话级 `permission_mode`（三档）非 `default` 时覆盖 Bot 级别（specific 覆盖 general，双向）；`default`/未设时用 Bot 级别。
2. **非交互源（例程）只取 Bot 级别**，不读会话档位——例程会话不复用交互会话，无合并对象。
3. **`custom` 不参与映射**：`resolve_bot_approval_level` 对 custom 落 `Prompt`（最安全解释），并由 `level_supported(provider, "custom") == false` 在保存/执行两侧拒绝。
4. **只降不升护栏保留**：函数结果仍流经 `resolve_effective_permission`（task_shape），delegation 工具等强制上限不变。

**理由**：AxAgent 的会话档位只有三档（`default/accept_edits/full_access`），与 Bot 五档不在同一枚举；把映射收敛为一个可单测纯函数，避免散落在 `agent_query` 与 cron executor 两处。`bots.approval_mode` 仍只存不解释（与 P1 ADR-007 一致），解释权集中在这一处。

**后果**：`agent_query` 对「Bot 会话 + 会话档位 default」的轮次行为会变化（此前三档映射 Prompt，现在按 Bot 级别）——这是本 ADR 的目的而非回归；普通（无 botId）会话零变化。

## ADR-P2-2 — cron 到点如何构造 Bot 执行上下文

**决策**：Bot 例程不走独立进程/独立会话池。`init/services.rs` executor handler 在 `workflow_id` 兜底之前（`:4433` 前）插入 `bot_id` 分支：`store.get(bot)` 校验存在 → 复用 `resolve_bot_context`（P1，`src-tauri/src/bot_context.rs`）组装 SOUL/MEMORY → 以**每轮新建的临时会话**跑 `agent_query` 等价路径（与 `agent_turn_adapter.rs` 的 headless 先例同构：prompter = None，ask 类裁决走 `PermissionPolicy` 自动 Deny；Bot 级别按 ADR-P2-1 第 2 条仅取 Bot 档）。运行结果经 `record_run` 落 `cron_job_history`，一次性任务跑完 `set_status(Disabled)`。

**理由**：调度器 30s tick 单派发、在途去重已是现成并发护栏；新建会话池会引入与 leader 租约、`dispatched` 表的第二套状态机，违反「不重建引擎」约束。

**后果**：例程轮的上下文构造在 Tauri 进程内同步完成，失败即 history.error；无重试（与现有 workflow 兜底分支一致，重试用任务中心手动触发）。

## ADR-P2-3 — 例程结果如何落通知未读

**决策**：**零新增管线**。例程轮复用 `agent_query`，其 `AgentDone/AgentError` 事件被 agentStore 现有监听（`src/stores/feature/agentStore.ts:475,555`）转成 `pushNotification`；P2-a 仅做两件事：① `Notification` 接口加 `botId?`（写入时从事件 payload 的 conversationId → `Conversation.botId` 解析）；② NotificationBell 与 BotsPage 增加按 Bot 过滤/角标。离线窗口（前端未开时事件丢失）**接受为已知限制**（GAP G-26），不引入后端补发队列。

**理由**：通知是纯前端 localStorage 模型，后端建表/事件即重建引擎；桌面应用常驻是主场景，窗口损失可量化后再在 P2-b 评估补发。

**后果**：Bot 例程完成通知依赖前端开启；文档化于 ACCEPTANCE-P2 已知限制（若单写，落 PLAN T6' DoD）。

## ADR-P2-4 — Bot 五档 → 运行档映射表（含引擎能力上限）

**决策**：`src-tauri/src/bot_approval.rs` 内两张静态表：

| OMB 档 | PermissionMode | 非交互（例程）下的效果 |
|---|---|---|
| `ask` | `Prompt` | prompter=None ⇒ 一切 ask 类操作 Deny，纯读可跑 |
| `edits` | `WorkspaceWrite` | 工作区内写自动，区外 ask ⇒ Deny |
| `auto` | `WorkspaceWrite`（**建议值，待裁决 Q1**） | 同 edits；「auto=放手」的更强解释（`DangerFullAccess`）留待裁决 |
| `full` | `Allow` | 全部自动 |
| `custom` | `Prompt`（永不保存成功，见 ADR-P2-1 第 3 条） | — |

`level_supported(provider: ProviderType, mode) -> bool`：`custom` 对所有 provider false；其余档对所有 AxAgent provider true（GAP G-18：API 引擎每轮读 PermissionPolicy，无引擎档位限制；OpenClaw 等 CLI 型如有例外由 T1' 实测补表）。

**理由**：`DangerFullAccess` 在 AxAgent 语义里是「绕过工作区边界」，与 OMB `auto`（自动执行常规操作但仍守 Bot 边界）不完全等价；先取保守值，裁决后一行可改。
