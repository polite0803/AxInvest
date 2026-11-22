# ADR — AxAgent Bot 集成的关键取舍

> 状态：active ｜ 日期：2026-11-22 ｜ 对应基线：`11f32873b` ｜ 来源：本地原创 ｜ 范围：`docs/bot-integration/`

本文件只记录**会长期约束实现**的决定。每条格式：背景 → 选项 → 决定 → 后果 → 复评条件。

---

## ADR-001 Bot 记录存在 SQLite 的新表 `bots`，不扩 `agent_profiles`，也不用 JSON

**背景**：`agent_profiles` 是专家/角色模板库，已被 `stock-*` 种子、工作流节点与 cognitive 路由大量引用（`agent_profile_id` 出现在 `src-tauri/src/commands/cognitive.rs`、`stock_analysis_setup/seed_*.rs` 等 40+ 处）。OpenMausBot 的 Bot 记录同时承担身份、SOUL、模型、审批、分组与线程归属。

**选项**

| 选项 | 代价 |
|---|---|
| A. 扩展 `agent_profiles` | 复用现成 CRUD，但 Bot 与「专家模板」语义混住；列表会把 20+ 个 `stock-*` 种子当成 Bot；每加一个 Bot 字段都影响工作流侧消费者 |
| B. 新建 `bots` 表 | 多一层对象；Bot 与 profile 需显式关联；但语义干净、可独立演进 |
| C. JSON 文件（照抄 `bots.json`） | 免建表，但与本仓「SQLite + SeaORM 为唯一持久层」冲突；并发写、迁移、事务、查询都要另造一套 |

**决定**：**B**。`bots` 表是 Bot 的唯一真相源；`bots.agent_profile_id` 为可选外键式引用（不建 FK 约束，与 `conversations.agent_profile_id` 现状一致），用于把「角色/工具模板」挂到 Bot 上。`agent_profiles` 语义不变。

**后果**

- 正面：Bot 列表只查 `bots`，不会漏出专家种子；SOUL/模型/审批字段演进不惊动工作流。
- 负面：一个 Bot 的「工具能力」要看两处（`bots` + 它引用的 profile）；文档与 UI 必须讲清「Bot 是主人，profile 是模板」。
- 兼容：纯新增表，存量库启动时由声明式引擎建表，不触碰任何既有行。

**复评条件**：若 P2 发现 80% 的 Bot 都必须带 profile，且两表 Join 成为热点，再考虑把 profile 的展示字段冗余进 `bots`。

---

## ADR-002 线程归属键是 `conversations.bot_id`（可空 + 索引），不是 `agent_profile_id`

**背景**：`conversations.agent_profile_id` 已存在，前端 `Conversation.agentProfileId` 也在（`src/types/index.ts:188`）。看起来可以直接拿它当 Bot 键。

**选项**

- 复用 `agent_profile_id`：零新增列，但语义漂移——一个 Bot 可以换 profile，一个 profile 可被多个 Bot 复用；且存量会话里 `agent_profile_id` 指向的是专家，不是 Bot。
- 新增 `bot_id`：多一列、多一次迁移式收敛，但归属语义唯一。

**决定**：新增 `conversations.bot_id TEXT NULL`，建索引；**保留** `agent_profile_id` 原义。Bot 线程 = `WHERE bot_id = ?`；未归属会话 `bot_id IS NULL`，行为完全不变。

**后果**

- `list_bot_threads` 是新增命令，不改 `list_conversations` 的既有签名与返回，避免前端大面积改动。
- 删除 Bot 时把其会话 `bot_id` 置 NULL（**不级联删除**），线程与消息完整保留。
- 工作目录仍由 `agent_sessions.cwd` 决定；创建 Bot 线程时若 Bot 配了 `workdir`，则把它写入该会话的 session cwd（ADR-003）。

**复评条件**：若 P2 的委派运行时需要「线程属于某次 assignment」的二级归属，再加 `assignment_id`，不动 `bot_id`。

---

## ADR-003 目录布局：`~/.axagent/bots/<botId>/`，SOUL 以 DB 为真相源、文件为镜像；MEMORY 以文件为真相源

**背景**：AxAgent 现有两套相关目录——`~/.axagent/personalities/<name>/`（全局 SOUL，单例激活）与项目根的 `.axagent/MEMORY.md`（项目记忆）。OpenMausBot 是 `workspaces/<botId>/`。

**决定**

```text
~/.axagent/bots/<botId>/
├── SOUL.md        # 镜像（DB `bots.soul` 为真相源）
├── IDENTITY.md    # 可选，镜像（P2）
├── MEMORY.md      # 真相源（文件）
└── memory/
    ├── <topic>.md
    ├── archive.md
    └── log/<date>.md
```

- **SOUL**：DB 是真相源（与 OpenMausBot 一致）。保存时写 DB 并原子重写镜像，计算 `soul_hash`。读取时若镜像的 hash 与 DB 不符 ⇒ `soul_drift = 1`，UI 提示「文件被外部修改」，提供「以文件为准」按钮。
- **MEMORY**：文件是真相源，不复制进 DB。`ProjectMemory` 直接以 `bot_root` 构造，复用其解析、渲染、200 行预算、原子写与分类目录。
- 目录权限 `0700`，文件 `0600`（Unix）；写入走临时文件 + rename。
- 不迁移、不删除旧的 `~/.axagent/personalities/`：P1 只提供「从人格导入」的一次性动作（P2 做）。

**后果**：Bot 的文件工具天然以 Bot 目录为家；用户可脱离 App 编辑 SOUL/MEMORY；凭据脱敏沿用 `ProjectMemory` 现有能力（P2 补齐记忆写入路径的脱敏）。

**复评条件**：若 Windows 上 `dirs::home_dir()` 与 Tauri 的 app data 目录需要统一（便携版/多实例），改为从 `AppState` 注入根路径，`BotWorkspace` 只接受根路径参数。

---

## ADR-004 SOUL 进 system prompt：新增 `<bot-soul>` slot，且 Bot 会话**不再注入全局 persona**

**背景**：`build_agent_system_prompt` 已有 slot 0 `<persona>`（全局人格）、slot 1 `<agent-profile>`、slot 2 `<user-custom-prompt>`。若只追加而不处理优先级，会出现「全局人格 + Bot SOUL」双注入，行为不可预测。

**决定**

1. 函数签名加 `bot_soul: Option<&str>` 与 `bot_memory: Option<&str>`（见 `IPC.md` / `PLAN.md` T5）。
2. 顺序固定为：`<bot-soul>` → `<agent-profile>` → `<user-custom-prompt>` → 默认指令 → workspace → RAG → `<bot-memory>` → working-memory → user-profile → skills。
3. **互斥**：当会话有 `bot_id` 且 Bot 的 `soul` 非空时，传 `persona_prompt = None`；全局 persona 只服务于「无 Bot 归属」的会话。
4. 两个调用点都要改：`src-tauri/src/commands/agent/mod.rs:1971` 与 `src-tauri/src/commands/agent/skill_execution.rs:874`。
5. `<bot-soul>` 用独立边界标签，内容按原文注入，不解析、不执行其中指令（与 `<user-custom-prompt>` 同一防注入纪律）。

**为什么不是「Bot SOUL 追加在 persona 后」**：全局 persona 是用户在设置里给「整个 App」的人格；Bot SOUL 是「这个 Bot 的岗位说明书」。两者同时生效会让模型收到互相冲突的自我描述。互斥是最小惊讶原则。

**后果**：Bot 会话的行为完全由 Bot SOUL + profile 决定；切换会话即切换人格。无 Bot 的旧会话行为零变化。

**复评条件**：若用户明确要求「全局人格作为所有 Bot 的基底」，改成可配置的 `inheritGlobalPersona`（默认 false），而不是默认叠加。

---

## ADR-005 MEMORY 注入：复用 `ProjectMemory`，只把根换成 Bot 目录，注入索引不注入主题文件

**背景**：`project_memory.rs` 已经是「MEMORY.md 索引（200 行硬限）+ 分类主题文件 + 按需检索」的完整实现，与 OpenMausBot 的 `docs/memory.md` 同构。

**决定**

- `BotWorkspace::memory()` 返回 `ProjectMemory::new(bot_root)`。
- `agent_query` 在 `conversation.bot_id` 非空时：`let idx = bot_memory.load_index().await;` 把 `idx.render()` 的结果作为 `bot_memory` 传入提示词。
- 主题文件**不自动注入**；模型用现有文件工具按需读取（`context_files` 的 `scan_relevant_files` 保持原样）。
- 200 行 / 24 KB 预算由 `ProjectMemory` 的 render 逻辑保证；P1 不在调用点二次截断。

**后果**：零新增记忆引擎；Bot 记忆的写入/归档/脱敏能力随 `ProjectMemory` 演进。风险是「项目根记忆」与「Bot 记忆」两套同时存在——P1 明确：Bot 会话只注入 Bot 记忆，不注入项目根 `.axagent/MEMORY.md`；非 Bot 会话维持原样。

**复评条件**：P2 若要求 Bot 同时读项目记忆，改成「Bot 索引 + 项目索引」两段，各自限预算，而不是拼接后截断。

---

## ADR-006 团队/部门不进入 P1

**背景**：本仓已有 `fleets` / `fleet_members` / `agent_roles.reports_to`，OpenMausBot 另有统一的委派运行时与共享模板。

**决定**：P1 不引入团队、部门、Chief、共享模板、委派运行时。理由：这些能力依赖「Bot 已存在且线程可归属」这一前置；在没有 Bot 记录与归属键之前做团队，会把两套未稳定的模型焊死。

**后果**：P1 的 Bot 是「单人助手」，不能互相委派；P2 再把 `fleets` 的成员指向 `bots.id`，并引入 Assignment 合同。

**复评条件**：P2 立项时先做「Bot 一对一委派」最小闭环，再扩部门。

---

## ADR-007 审批级别在 P1 只存字段、不执行

**背景**：现有审批是「命令级批准沉淀」（`approval_rules`）+ 会话级 `agent_sessions.permission_mode`；OpenMausBot 的级别是 Bot/线程属性并参与 `max(own, inherited)` 解析。

**决定**：`bots.approval_mode TEXT NOT NULL DEFAULT 'full'`，取值集合严格为 `ask|edits|auto|full|custom`（与 `APPROVAL_MODES` 对齐）；P1 的创建/编辑表单可编辑该字段，但**不改变运行时行为**。P2 做解析器与 `permission_mode` 接线。

**后果**：避免 P1 出现「UI 显示 Ask，实际仍全权」的静默欺骗——P1 在 UI 上明确标注「P2 生效」。若产品不接受该标注，则 P1 干脆隐藏该字段（二选一，默认标注）。

**复评条件**：P2 开工时以 `agent_sessions.permission_mode` 为落点，新增解析函数并加不变量测试（记录级别 = 实际运行级别）。

---

## ADR-008 不新增 migration，走声明式引擎 + `schema_compliance` 登记

**背景**：`MIGRATIONS = &[]` 且有测试强制为空；引擎按实体自动建表/加列。

**决定**

1. 新实体 `axagent-core/crates/entities/src/bots.rs`，在 `entities/src/lib.rs` 加 `pub mod bots;`（build.rs 自动登记）。
2. `conversations.rs` 加 `bot_id` 列。
3. 同步更新 `axagent-infra/crates/dao/tests/schema_compliance.rs` 的 `SCHEMA`：新增 `bots` 全列，`conversations` 加 `bot_id`。
4. 不写任何 `m*_*.rs`；不改 `CURRENT_VERSION`（保持 233）。

**后果**：存量库启动时自动收敛（CreateTable / AddColumn），无需数据搬迁。风险是「引擎静默」——必须靠 `schema_compliance` 与启动日志双保险。

**复评条件**：若未来需要数据回填（如把旧 `agent_profile` 转成 Bot），那属于数据搬迁，必须单独立项并恢复受控迁移通道，不能塞进本次声明式收敛。

---

## ADR-009 删除 Bot 的语义：删记录、保留线程与记忆目录

**背景**：OpenMausBot 删除 Bot 时线程保留。

**决定**

- `delete_bot`：删除 `bots` 行；`UPDATE conversations SET bot_id = NULL WHERE bot_id = ?`；**不删** `~/.axagent/bots/<id>/`（SOUL/MEMORY 保留，便于恢复或人工取用）。
- 前端确认文案必须写明「线程与记忆会保留」（i18n `bots.deleteConfirm`）。
- 不提供 P1 的「彻底删除记忆」入口；P2 再加。

**后果**：误删可恢复（重建同 id 不可，但记忆目录仍在）；磁盘可能残留孤儿目录，P2 提供清理。

**复评条件**：若用户反馈残留目录困扰，P2 加「删除时一并删除记忆」的显式勾选。

---

## 决策汇总

| ADR | 决定 | 影响面 |
|---|---|---|
| 001 | 新表 `bots` | 存储、DAO、UI 列表 |
| 002 | `conversations.bot_id` | 会话模型、列表命令 |
| 003 | `~/.axagent/bots/<id>/` | 文件系统、记忆、SOUL 镜像 |
| 004 | `<bot-soul>` slot + 与 persona 互斥 | 提示词组装（两处调用点） |
| 005 | 复用 `ProjectMemory`，只换根 | 记忆注入 |
| 006 | 团队不进 P1 | 范围控制 |
| 007 | 审批只存不执行 | UI 标注、P2 接线 |
| 008 | 无 migration，声明式 + SCHEMA 登记 | 建表、门禁 |
| 009 | 删除保留线程与记忆 | 删除命令、文案 |
