# GAP — AxAgent × OpenMausBot Bot 能力差距分析

> 状态：active ｜ 日期：2026-11-22 ｜ 对应基线：`11f32873b` ｜ 来源：本地原创 ｜ 范围：`docs/bot-integration/`

## 0. 结论（一句话）

AxAgent 已经有 Bot 所需的**运行时零件**（agent、会话、记忆、审批、调度、多 Agent），缺的不是引擎，而是**归属层**：Bot 作为「身份 + SOUL + 工作区 + 记忆 + 线程」的宿主对象不存在；现有 SOUL 是全局单例，记忆作用域是项目根，线程只认 `agent_profile` 不认 Bot。P1 因此是一条**接线与收口**的纵向切片，而不是新建 agent 运行时。

## 1. 证据与方法

| 证据 | 位置 |
|---|---|
| 仓库规范与分层铁律 | `AGENTS.md`（禁区 1–14、Harness 六层、文档注册规则） |
| 文档索引 | `docs/INDEX.md`、`docs/architecture/dependency-graph.md` |
| AxAgent 代码 | `src/`、`src-tauri/src/commands/`、`axagent-core/crates/entities/`、`axagent-infra/crates/dao/`、`axagent-ai/crates/agent/` |
| OpenMausBot 文档 | GitHub `milind-soni/OpenMausBot` 的 `docs/memory.md`、`presets.md`、`team-sharing.md`、`routine-schedules.md`、`approval-levels.md`、`runtime-contract.md`、`bot-api.md`（2026-11-22 经 GitHub Contents API 取回） |
| OpenMausBot 运行实例 | 本机数据目录 `~/.openmausbot/bots.json` 实测记录字段；`resources/server/index.js` 的 `APPROVAL_MODES`、`/api/bots` 路由与 `BOT_PROFILE_PATCH_FIELDS` |
| 前端取证 | Bot 前端工程师只读取证（2026-11-22，基线 `11f32873b`） |
| 门禁取证 | Bot 门禁与验证工程师只读取证（同上） |

> 行号会随改动漂移；本文的行号用于**定位**，落点以文件与函数名为准。

## 2. OpenMausBot 的 Bot 模型（权威口径）

### 2.1 Bot 记录

实测 `bots.json` 一条记录的字段：

```text
id, threadId, name, title, description, soul, notifications, color,
unread, resumeCursors, createdAt, section, modelSelection{instanceId,model,effort},
autoApprove, tasks[], soulHash, soulDrift
```

- `soul`：常驻系统指令正文；文件镜像 `bots/<botId>/SOUL.md` 是**镜像**，记录才是真相源，`soulHash` / `soulDrift` 记录两者是否偏离。
- `section`：侧栏分组；`modelSelection`：实例 + 模型 + 推理档；`autoApprove` / 线程级 `approvalMode`：审批级别。
- `tasks[]`：该 Bot 的线程数组，线程记录含 `threadId, title, openedBy{botId,name,threadId,at,kind}, approvalMode, autoApprove, alwaysAllow[], unread, cwd, lastInstanceId, handedMessages, usage, closedBy`。
- 可 PATCH 的 Bot 档案字段（`BOT_PROFILE_PATCH_FIELDS`）：`name, title, description, soul, notifications, avatar*, mascot*, voice, speakReplies, replyStyle`；另有 `modelSelection, hidden, fallback, projects, playbooks, assignedSkills`。

### 2.2 工作区与记忆（`docs/memory.md`）

```text
~/.openmausbot/workspaces/<botId>/
├── MEMORY.md            # 每轮注入「前 200 行或 24 KB，先到者为准」
└── memory/
    ├── <topic>.md       # 按需读取，不自动注入
    ├── archive.md       # 从 MEMORY.md 溢出/过期的条目
    └── log/<date>.md    # 每日日志，不自动注入
```

- 记忆开关按 Bot 生效；关闭后不再注入、不再自动回忆/维护。
- 写入 `0600`、原子写；疑似凭据在落盘前被替换为 `«redacted N chars»`。
- `MEMORY.md` 永不超预算：写入时把最旧的带日期条目移入 `memory/archive.md`，先移归档再写主文件。

### 2.3 审批级别（`docs/approval-levels.md`）

- 级别：`ask < edits < auto < full`，另加 `custom`（仅部分引擎）；个人安装默认 `full`，托管组织默认 `ask`。
- 归属：级别属于 **Bot**，作用于其下一次 provider turn；线程可自带级别。
- 解析：`level = max(own, inherited)`；`custom` 不传递；继承受引擎能力封顶。
- Bot 读取**自己的文件**（SOUL/技能/记忆/会话目录）在任何级别都不询问。

### 2.4 例程、自动化、团队、外部 API

| 能力 | OpenMausBot 语义 | 文档 |
|---|---|---|
| 例程 | 五字段 cron + 明确 IANA 时区；调度器到点唤醒，模型不参与判断日期；运行历史与暂停/完成态 | `routine-schedules.md` |
| 自动化 | 应用事件触发（`app-triggers`），非轮询 | 同上 / `proactive-routines.md` |
| 团队/部门 | Bot 可共享为模板；部门有 Chief；共享包**明确不含**聊天、记忆、密钥、模型、审批级别、目录、可见性 | `team-sharing.md` |
| 委派运行时 | 统一 `Assignment` 记录（root/parent/depth、origin、requester、assignee、mode、reply、level、executions、result/evidence），SQLite `runtime.db`，状态机 queued→running→waiting/needs_input→completed/failed/cancelled | `runtime-contract.md` |
| Bot API | 每 Bot 独立开关与 key（只显示一次、服务端只存哈希），`POST /hooks/api/bots/<botId>/messages` | `bot-api.md` |
| 预设 | 白名单：名称/描述/SOUL/外观/playbook/技能/起始记忆；**没有**模型、目录、审批、连接、MCP、同伴字段 ⇒ 预设不能扩权 | `presets.md` |

## 3. AxAgent 现状盘点

### 3.1 后端（Rust）

| 现有能力 | 入口 | 说明 |
|---|---|---|
| Agent 执行 | `src-tauri/src/commands/agent/mod.rs:571` `agent_query` | 组装系统提示词、挂工具、流式事件 |
| 系统提示词 | `src-tauri/src/commands/agent/skill_execution.rs:614` `build_agent_system_prompt` | 已分 slot：persona / agent-profile / user-custom / 默认指令 / workspace / RAG / working-memory / user-profile / skills |
| 全局人格（SOUL） | `axagent-ai/crates/agent/src/personality.rs` | `~/.axagent/personalities/<name>/{SOUL.md,IDENTITY.md,USER.md}`，**全局单例**（`.active`） |
| 项目记忆 | `axagent-ai/crates/agent/src/project_memory.rs` | `.axagent/MEMORY.md` 索引（200 行）+ `memory/{user,feedback,project,reference}/` 主题文件；与 Claude Code 模型对齐 |
| 上下文文件 | `axagent-ai/crates/agent/src/context_files.rs:15` | 自动加载 `AGENTS.md` / `CLAUDE.md` / `.axagent/MEMORY.md` |
| 会话与线程 | `axagent-infra/crates/dao/src/repo/conversation.rs`、`src-tauri/src/commands/conversations/mod.rs:727` `list_conversations` | `conversations.agent_profile_id` 已存在；`agent_sessions.cwd` 决定工作目录 |
| AgentProfile | `axagent-core/crates/entities/src/agent_profiles.rs`、`dao/src/repo/agent_profile.rs` | 专家/角色库（含 `stock-*` 种子）；`list/get/create/upsert/update/delete` 齐全 |
| AgentRole / Expert | `agent_roles.rs`、`agency_experts.rs` | 角色职责、工具、汇报关系（`reports_to`） |
| 多 Agent | `commands/multi_agent.rs`、`orchestrator.rs`、`fleets` / `fleet_members` | 已有委派命令与「舰队」组织模型 |
| 审批 | `commands/approval_rules.rs`、`entities/approval_rules.rs`、`agent_sessions.permission_mode` | 命令级「批准沉淀」；无 Bot 级审批级别 |
| 调度 | `commands/scheduled_task.rs`、`cron_jobs`（整行 JSON） | 已有 cron 持久化与执行历史 |
| 主动建议 | `commands/proactive.rs`、`proactiveStore` | nudge / insight / pattern |
| 记忆命名空间 | `commands/memory.rs`（40+ 命令）、`memory_namespaces/memory_items` | **数据库命名空间 + 向量检索**，与 MEMORY.md 文件模型并行 |
| 事件 | `axagent-core/crates/harness/src/event_names.rs:38` `IpcEventName` | `agent-*` / `chat-stream-*` / title 等已登记 |

### 3.2 前端（React）

| 现有能力 | 入口 | 说明 |
|---|---|---|
| 侧栏导航 | `src/lib/navRegistry.tsx:46-162`、`src/lib/navGroups.ts:38-94`、`Sidebar.tsx:282-312` | `NavItem{key,icon,labelKey,path}`；分组骨架在 `navGroups` |
| 路由 | `src/lib/pageRegistry.ts:31` `BUILTIN_PAGE_PATH`、`ContentArea.tsx:174+` | 新增内置页要同批改 pageRegistry / navRegistry / navGroups / Route |
| 会话列表 | `src/stores/domain/conversationStore.ts:152`、`ChatSidebar.tsx:745-945` | 先父子线程、再按 `workspaceDir` 分组，其余按 pinned/日期 |
| AgentProfile 选择 | `src/components/chat/AgentProfileSelect.tsx:8-35` | 直接 `invoke("list_agent_profiles")`，全仓零引用（待接线） |
| Agent 面板 | `src/components/agent/AgentPanel*.tsx` | 可作 Bot 详情抽屉骨架 |
| 记忆页 | `src/pages/MemoryPage.tsx`、`MemorySettings.tsx`、`MemoryApprovalPanel.tsx` | 管的是 DB 命名空间与写审批，**没有** MEMORY.md 文件视图 |
| 人格页 | `src/pages/PersonaPage.tsx:36` | 列表 + 详情 + SOUL 文本编辑，可作 SOUL 编辑参照 |
| 多 Agent 卡片 | `src/components/multi-agent/MultiAgentDashboard.tsx:73-112` `RoleCard` | 最接近 Bot 卡片样式 |
| 状态 | `src/stores/feature/agentStore.ts:116-224` | 已有 profiles 切片与 CRUD；无 `botStore` |
| i18n | `src/i18n/locales/*.json`（11 语言） | 无 `bots.*` 命名空间；无批量补键脚本 |

### 3.3 数据模型与建表机制（决定 P1 能否加表）

- **不能用 migration**：`axagent-infra/crates/dao/src/migrations/mod.rs:112` 为 `MIGRATIONS = &[]`，并有护栏测试 `missing_versions_is_empty_after_migration_purge`（同文件 `:859`）断言其必须为空。
- 表结构由**声明式引擎**收敛：实体在 `axagent-core/crates/entities/src/lib.rs` 声明 `pub mod`，`crates/dao/build.rs` 编译期生成实体注册表（`reconcile/expected.rs:69` `include!`），支持 `CreateTable` / `AddColumn`（`reconcile/plan.rs:126-128`）。
- 新增表/列必须同步登记 `axagent-infra/crates/dao/tests/schema_compliance.rs:15` 的 `SCHEMA` 清单，否则是**静默覆盖缺口**。
- 引擎会处置「实体未覆盖的孤儿表」（P0a 事故形态）⇒ 新表只能通过实体声明产生。

## 4. 逐项差距表

判定：**A=已有可复用**、**B=需改造**、**C=缺失**。

| # | 能力 | OpenMausBot 语义 | AxAgent 现状 | 判定 | 关键证据 | 阶段 |
|---|---|---|---|---|---|---|
| 1 | Bot 记录 | 独立记录：name/title/description/soul/model/approval/section/外观 | 只有 `agent_profiles`（专家库）与 `agent_roles` | **B**（新增 `bots` 表，不复用 profile 当 Bot） | `entities/agent_profiles.rs`、`bots.json` 字段 | P1 |
| 2 | Bot 列表/创建/编辑/删除 | 侧栏列表 + 新建即建 Bot | `agentStore` 的 profiles CRUD，但语义是「专家模板」 | **B** | `agentStore.ts:167-224` | P1 |
| 3 | SOUL 持久化与镜像 | `soul` 字段 + `SOUL.md` 镜像 + hash/drift | 全局单例 `~/.axagent/personalities/<name>/SOUL.md`，无 hash 校验 | **B** | `personality.rs:70-110` | P1 |
| 4 | SOUL 注入 | 每 Bot 常驻指令进 system prompt | 只有全局 `<persona>` slot（`agent/mod.rs:647`） | **B**（加 `<bot-soul>` slot + 优先级） | `skill_execution.rs:614-640` | P1 |
| 5 | 工作区 | Bot 私有目录即文件工具 cwd | `agent_sessions.cwd` 每会话可设 | **A**（复用，仅改默认来源） | `agent/mod.rs:1942-1964` | P1 |
| 6 | 记忆文件模型 | MEMORY.md 200 行/24 KB + topics + archive + log + 脱敏 | `project_memory.rs` 同构，但作用域是「项目根」 | **B**（作用域改 Bot） | `project_memory.rs:197-231` | P1 |
| 7 | 记忆注入 | 每轮注入 MEMORY.md 索引 | `context_files.rs` 只认项目根 | **B**（Bot 根加入发现/注入） | `context_files.rs:15` | P1 |
| 8 | 记忆查看 UI | 预算 gauge + 编辑器 + 变更日志 | 只有 DB 命名空间管理；无 MEMORY.md 视图 | **B**（P1 只读视图） | `pages/MemoryPage.tsx` | P1 |
| 9 | 线程归属 | Bot→tasks[]，线程含 openedBy/cwd/approval | `conversations.agent_profile_id` 可绑 profile，**无 Bot 归属** | **B**（`conversations.bot_id`） | `conversations.rs`、`ChatSidebar.tsx:745` | P1 |
| 10 | 线程级审批级别 | `approvalMode` + `max(own,inherited)` | `agent_sessions.permission_mode` 已有，未与 Bot 联动 | **B** | `agent_sessions.rs` | P2 |
| 11 | Bot 级审批级别 | ask/edits/auto/full/custom | 仅 `agent_profiles.recommend_permission_mode` 建议字段 | **B**（P1 存字段，P2 执行） | `agent_profiles.rs:31` | P1/P2 |
| 12 | 例程 | 五字段 cron + IANA 时区 + 运行历史 | `cron_jobs` + `scheduled_task` 已具备 | **A** | `scheduled_task.rs:281-668` | P2 |
| 13 | 应用事件自动化 | app-triggers，事件驱动 | 无应用事件触发源；有 proactive/后台任务 | **B** | `proactive.rs` | P2 |
| 14 | 团队/部门 | 部门、Chief、共享模板、事故 | `fleets`/`fleet_members`/`agent_roles.reports_to` + multi_agent | **A**（模型已在，接线未做） | `fleets.rs`、`multi_agent.rs:197` | P2 |
| 15 | Bot 间委派运行时 | 统一 Assignment 合同 + 状态机 + 预算 | `delegate_task` + orchestrator，语义分散 | **B** | `multi_agent.rs`、`orchestrator.rs` | P2 |
| 16 | Bot 外部 API | 每 Bot key + `/messages` | API 网关（Axum，OpenAI 兼容）已有，无按 Bot 的 key/入口 | **B** | `gateway` crate、`gateway_keys` | P2 |
| 17 | 预设/模板 | 白名单预设，技能默认关 | 无 preset；有内置 profile 种子 | **C** | `stock_analysis_setup/seed_*` | P2 |
| 18 | 每 Bot 技能归属 | Bot 私有 `skills/` + 安装戳 | 技能是全局库 + 会话级 `enabled_skill_ids` | **B** | `skillStore`、`skill-state` | P2 |
| 19 | 通知/未读 | notifications + unread | `NotificationBell` + nudge | **A** | `layout/NotificationBell.tsx` | P2 |
| 20 | 记忆写审批 | 写入前人工确认 | `MemoryApprovalPanel` + `get_pending_memory_writes` | **A** | `components/memory/MemoryApprovalPanel.tsx:22` | P2 |
| 21 | 事件与流式 | 统一流式/生命周期事件 | `IpcEventName` 已覆盖 agent 生命周期 | **A** | `event_names.rs:38-65` | P1 |

## 5. 明确不重建的清单（P1 硬约束）

1. **不新建 agent 运行时**：`agent_query`、工具系统、流式事件沿用。
2. **不新建提示词框架**：只在 `build_agent_system_prompt` 现有 slot 体系里加两段。
3. **不新建记忆引擎**：`ProjectMemory` 的解析/渲染/预算/原子写全部复用，只换根目录。
4. **不新建调度器**：P2 把 `cron_jobs` 的 `botId` 接上即可。
5. **不新建组织模型**：`fleets` / `fleet_members` 已覆盖「部门/成员」。
6. **不改 `agent_profiles` 语义**：它仍是专家/角色模板库，Bot 通过可选 `agent_profile_id` 引用它。

## 6. P2 范围（只列不展开）

- 例程与自动化：`cron_jobs` 加 Bot 归属、routine run 记录、暂停/完成态、应用事件触发。
- Bot 级与线程级审批：级别解析 `max(own, inherited)`、引擎能力封顶、`agent_sessions.permission_mode` 接线。
- 团队/部门：`fleets` ↔ Bot、Chief、共享模板（白名单校验，禁止带出记忆/密钥/审批）。
- 多 Bot 委派运行时：`Assignment` 合同、树/深度/预算/去重、waiting/needs_input 状态机。
- 每 Bot 技能、预设导入、Bot API key、通知与未读。

## 7. 未验证项

- OpenMausBot 的 `memory/log` 自动日志与「dream」维护未在本仓落地评估；P1 只做 MEMORY.md 注入与查看。
- 审批级别到各引擎（Claude/Codex/Gemini 等）的映射细节未在 AxAgent 侧验证；P2 再对齐。
- `cron_jobs` 的 `botId` 字段是否存在需 P2 开工前复核（本次仅确认它是整行 JSON，新增字段无需建表）。
