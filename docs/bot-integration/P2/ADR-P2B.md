# ADR-P2B — P2-b 架构决策记录

> 状态：accepted（P2-b 范围内）｜ 日期：2026-11-23 ｜ 基线：`ee6690c3a` ｜ 前置：`ADR.md`（P1 001..009）、`ADR-P2.md`（P2-a，编号顺延）

## ADR-P2B-1 — 部门不复用 `fleets`/`fleet_members`，新双表

**决策**：部门用新表 `bot_departments` + `bot_department_members`（MODEL-P2B §2），与办公室舰队并存，互不引用。

**理由**：fleets 的成员主键是 AgentSession、路由键是 agent_slug、执行器带办公室特化取舍（不加载 MCP、首个可用 provider、Phaser room），这些都不是 Bot 组织语义的一部分（GAP-P2B §2 G-B3）。复用要改 fleet_members 主键语义并牵动 executor/水位/前端，改动面远大于两张小表；声明式建表的边际成本近零。

**后果**：两套「团队」概念并存，前端须在文案上区分（「部门」vs「AI 办公室」），进 i18n 键设计；若未来裁决统一组织模型，迁移成本 = join 表数据搬迁（一次 INSERT SELECT）。

## ADR-P2B-2 — 委派执行器与 P2-a 例程分支同构，不新建运行时

**决策**：assignee 轮 = `agent_query` 等价路径的非交互调用，构造方式与 ADR-P2-2 例程分支同构：`resolve_bot_context`（SOUL/MEMORY）+ 临时会话 + `resolve_bot_approval_level` 只取 Bot 档 + prompter=None（ask 类自动 Deny）。差别仅两点：① 上下文里注入**委派任务卡**（task_text + requester 身份 + 「结果将回传给发起方」一句边界说明，不注入 requester 的 SOUL/MEMORY）；② 结果不落 `cron_job_history` 而落 `bot_assignments` + 回执线程。

**理由**：例程分支已验证「非交互 Bot 轮」的全部难点（上下文、审批、事件、历史）；委派与之的差异只在触发源与回执落点，属参数级而非结构级。dispatch 用 `tokio::spawn` 后台执行 + 并行门计数，不新建会话池/调度器（同 ADR-P2-2 的理由）。

**后果**：assignee 轮的流式 chunk 不推送前端（复用例程分支的静默语义）；前端看 Assignment 状态轮询/事件获取进展（IPC-P2B §5 事件）。

## ADR-P2B-3 — 外部不升权：requester 档位不进 assignee 上下文

**决策**：assignee 的运行档 = **assignee 自己的 Bot 档**（经 `resolve_bot_approval_level` 非交互第 2 条），与 requester 的档位、provider、模型完全无关；requester 只能拿到 assignee 愿意输出的文本。consult 结果作为**工具结果文本**回到 requester 轮，不携带 assignee 的内部上下文。

**理由**：OMB 语义「被委派方以自己的权限档运行」（`runtime-contract.md`）；AxAgent 侧 `resolve_effective_permission` 只降不升护栏已保证委派工具不提权，但真正要防的是「高权限 requester 把低权限 assignee 当提权代理」——assignee 侧只认自己的档即天然免疫。

**后果**：`bots.approval_mode` 是唯一安全边界；低档 assignee 被高档 requester 委派时写操作将 Deny 并落 failed（error_code `BOT_DELEGATE_PERMISSION_DENIED`），前端如实展示，不做静默降级重试。

## ADR-P2B-4 — consult 同步、dispatch 异步，两种语义不共用执行路径

**决策**：
- **consult**：requester 轮内的工具调用（tools 层新工具 `delegate_to_bot`，runner 注入模式照抄 `init_delegate_task_runner`），同步等结果，Assignment 记录 status 直接终态（completed/failed），无 queued/running 窗口。
- **dispatch**：Tauri 命令入口（`create_bot_assignment`），落库 queued 后 `tokio::spawn`，立即返回 AssignmentDto；回执 append 发起线程 + Bot 维度通知。

**理由**：OMB 的 consult=「从已有知识回答、不动手」、dispatch=「执行并回报」，超时预算与 UX 完全不同（工具调用受 LLM 轮超时约束，必须同步；派工可后台）。共用路径会把 consult 的超时强加给 LLM 轮或让 dispatch 阻塞 UI，两者都错。

**后果**：consult 的并行门在工具层（requester 在途 consult ≤2，防 LLM 单轮无限扇出）；dispatch 的并行门在命令层（IPC-P2B §3）。 consult 不落回执线程（结果在 requester 轮的对话历史里，自然可见），只落 Assignment 行（审计）。

## ADR-P2B-5 — 委派深度 P2-b 恒一层，字段先落

**决策**：`bot_assignments.root_id/parent_id/depth` 三列随表落地，但 P2-b 校验规则恒 `depth==1`：requester 是 Bot 时，其发起的委派仍 depth=1、parent=NULL（即**不允许 assignee 再委派**，assignee 上下文里不提供 delegate 工具）。

**理由**：可传递委派需要树形状态机 + 循环检测 + 逐层预算，是 P3 的运行时课题；但合同字段是 schema 级决策，先落可避免 P3 加列。一层限制下环检测退化为「assignee 无委派工具」，零成本成立。

**后果**：`delegate_to_bot` 工具只注册给「person/例程/routine 发起的轮次」，assignee 轮的工具注册表不含它；P3 解禁时改工具注册条件 + parent/depth 赋值逻辑即可。

## ADR-P2B-6 — 部门删除级联与 Chief 约束

**决策**：删部门 → 命令层删成员行 + 该部门 Bot 归属清空（Bot 本体不动）；Bot 的 Chief 地位不是 Bot 的属性而是部门的属性，Chief Bot 被删时命令层 `delete_bot` 追加：将以其为 chief 的部门的 `chief_bot_id` 置 NULL（部门保留，Chief 空缺，不级联删部门）。Q4 待用户确认的「是否级联删成员归属」按上句执行（清归属不删部门）。

**理由**：部门是组织常设物，Bot 是可随时删除的个体；反向级联（删 Bot 拆部门）会破坏其他成员的组织关系。与 P2-a「删 Bot 清例程」同款命令层扫描模式（GAP G-10），不建 SQL 外键。

**后果**：Chief 空缺期间部门功能不受限（P2-b Chief 只是标注 + 未来派工路由语义，无运行时特权），前端显示「无 Chief」占位。
