# PLAN-P5 — P5 分期计划（frozen-for-P5）

> 状态：frozen-for-P5 ｜ 日期：2026-11-25 ｜ 基线：`fedf00c17`（tag `v3.1.0`）+ 本地三提交（`b0a43360e` R1 Android 修复 / `1c7e75b63` R1 reconcile 修复 / `afe255c40` fmt 收口）｜ 分支：`feat/p3-bot-workflow-frontend` ｜ 对应版本：3.1.0（若落地）｜ 来源：本地原创
>
> 前置：`GAP-P5.md`、`ADR-P5.md`、`MODEL-P5.md`、`IPC-P5.md`、`I18N-KEYS-P5.md`、`SCOPE-OPTIONS-P5.md`
>
> **本批范围已冻结**（D1–D6 裁决落值见 `ADR-P5.md` 头部）：**P5 = B（费用/Token 配额）+ C1（每 Bot 技能归属 + preset 导入）两条独立纵向切片**。A（多 Bot 协作运行时）不进本批（ADR-P5-7）；C3（记忆写入）不进本批；G-B13 白名单并入 C1（ADR-P5-6）。

## 0. 来源与事实基线

### 0.1 前置批已收口

- P1（SOUL/MEMORY 注入与查看）、P2-a（例程与审批档）、P2-b（部门与委派）、P3（编排与委派链）、P4（运行配额 + needs_input 待答）均已冻结并收口。
- P4 的「只收紧」配额解析层（`src-tauri/src/bot_quota.rs`）+ task-local 下发（`AssignmentContext`）是本批的**地基**（ADR-P5-1 第 1 条）。

### 0.2 本轮只读实测（决定 P5 不能用「从零造」的方式写）

| 事实 | 证据 | 影响 |
|---|---|---|
| 计价函数已有且**不含缓存 token** | `commands/agent/pricing.rs:186` `estimate_cost_usd` 只算 input/output；`usage_pricing.rs:45-77` 有四类分价 | 账本必须用 `usage_pricing`（ADR-P5-4） |
| 例程轮用量**已在手** | `bot_routine_runner.rs:269-281` `summary.usage` | B2 写入点①就地写 |
| 委派轮用量**不在手上** | `AssignmentTurnExecutor::execute_with_context` 返回 `Result<String, String>` | B2 写入点②需改执行器契约（ADR-P5-8） |
| 编排**不做模型调用** | `bot_workflow_runner.rs:538-665` `orchestrate` 只建 assignment | run 级汇总是派生值，不新表（MODEL-P5 §5） |
| 交互轮费用**已在写** | `commands/agent/mod.rs:2590-2616` | B2 写入点③同处增写账本行 |
| 费用/Token 配额**无表** | `bot_department_quotas` 只有六个运行上限 | 新表 `bot_cost_quotas`（MODEL-P5 §3） |
| 技能库**全全局** | `skill_dirs.rs:6` 六类源目录 + 外部目录；`SkillIndex::rebuild`（`skill/mod.rs:125`）按优先级取同名首个 | 加 botId 作用域过滤（ADR-P5-5） |
| Bot 工作区根已存在 | `BotWorkspace::for_bot`（`bot_workspace.rs:46-52`）= `~/.axagent/bots/<botId>/` | 技能落 `<root>/skills/`，不新建路径约定 |
| Bot 侧栏有页签先例 | `BotDetail.tsx:178-236` Tabs；`BotDepartmentModal.tsx:33` `initialTab` | 用量页签 + 技能页签照此接线 |
| 门禁基线 | 504 码 × 11 语言；`BOT_I18N_KEYS` 185 条 | P5 后期望 508 码 / 217 键 |

### 0.3 本批**不**改的冻结口径（逐字）

- P3 运行预算常量 **2 / 900 / 12**（`bot_workflow_runner.rs:31-35`）、**300 / 每层 2**（`bot_assignment_runner.rs:33-35`）—— **不改**；
- P2-b 的 **4 / 8 / 300**（requester 并发 4 / 全局 8 / 超时 300s）—— **不改**；
- P4 的六个运行配额上限与 `min` 只收紧语义 —— **不改**；
- P4 的待答四态与 `BOT_INPUT_*` 三码 —— **不改**；
- P1 的线程归属（`conversations.bot_id`）—— **不加 `opened_by`**（ADR-P5-7）。

## 1. 目标与范围

### 1.1 一句话目标

**让「哪个 Bot / 这个部门花了多少 token 和钱、超了没有」可见且可拦截；同时让「每个 Bot 会哪些技能」可按 Bot 配置与预设导入。**

### 1.2 建议范围：两条独立纵向切片

| 切片 | 一句话 | 预估人日 | 可独立上线 |
|---|---|---|---|
| **P5-b** | 用量账本 + 费用/Token 配额 + 事后熔断 + 用量 UI | **8–10** | 是（一条切片） |
| **P5-c1** | 每 Bot 技能归属 + preset 导入（含白名单）+ 技能页签 | **4–5** | 是（一条切片） |

合计 **12–15 人日**，三人并行 **5–6 个工作日**（对齐 P3/P4 体例）。

### 1.3 明确不做（P5 范围外）

- **A 多 Bot 协作运行时**（事故线程 / ≤2 重试 / `openedBy` / rooms）—— ADR-P5-7；
- **C2 Bot API key**（对外鉴权面）—— 需 key/速率/审计/吊销四件同批，另立；
- **C3 记忆写入与维护** —— 需先裁决审批档（D4），属 P5-b / P6 候选；
- **会话/日级全局预算拦截**（`BudgetConfig.max_cost_per_day_usd` / `max_cost_per_session_usd`）—— 全局口径，与本批正交；
- **精确扣费 / 预扣**、**成本预警通知**、**Bot 技能正文编辑器**、**既有全局技能迁移到 per-bot**；
- **P1–P4 冻结口径的任何变更**。

## 2. 数据模型增量（已冻结 → `MODEL-P5.md`）

| 表 | 用途 | 唯一键 |
|---|---|---|
| `bot_usage_ledger` | append-only 用量账本（四类 token + 估算费用 + 计价来源 + 归属快照） | 无（id 主键） |
| `bot_cost_quotas` | 费用/Token 配额（scope = bot / department） | `UNIQUE(scope, scope_id)` |
| `bot_skill_bindings` | 每 Bot 技能归属声明 + 本 Bot 启停 | `UNIQUE(bot_id, skill_name)` |

既有表**一列不加**。索引声明：`MULTI_COL_INDEXES` +2（`bot_cost_quotas` + `bot_skill_bindings`；`EXPECT_MULTI` 38 → 40），`total_declarations` 83 → 85（用例改名 `l2_total_is_eighty_five`）。B1/C1-1 须同提交落计数。

## 3. IPC 契约增量（已冻结 → `IPC-P5.md`）

新增 **8 条命令**：

| 命令 | 落点 |
|---|---|
| `get_bot_cost_quota` / `set_bot_cost_quota` / `reset_bot_cost_quota` | `src-tauri/src/commands/bot_cost.rs`（新） |
| `list_bot_usage` / `get_bot_usage_summary` | 同上 |
| `list_bot_skills` / `set_bot_skill_enabled` / `import_bot_skill_preset` | `src-tauri/src/commands/bot_skill.rs`（新） |

修改 1 处内部契约：`AssignmentTurnExecutor` 新增 `execute_with_usage`（默认实现兜住旧行为，`execute` / `execute_with_context` 签名不变）。

**不新增事件**（IPC-P5 §3）。

## 4. 分期拆卡（已冻结）

### 4.1 依赖图与并行度

```
        ┌────────────── B1 ──────────────┐
        │  实体/DAO/SCHEMA/索引声明       │
        └───────┬────────────────┬───────┘
                │                │
        ┌───────▼──────┐  ┌──────▼────────┐
        │  B2 写入点   │  │ B3 配额+命令   │
        │  +执行器回传  │  │ +错误码+DTO   │
        └───────┬──────┘  └──────┬────────┘
                │                │
                └───────┬────────┘
                        │
              ┌─────────▼─────────┐
              │ B4 前端页签+store │
              └─────────┬─────────┘
                        │
              ┌─────────▼─────────┐
              │ B5 测试+门禁     │
              └───────────────────┘

  ┌────────────── C1-1 ──────────────┐
  │ 归属表/DAO/SCHEMA + skills 目录  │
  └───────┬────────────────┬─────────┘
          │                │
  ┌───────▼──────┐  ┌──────▼──────────┐
  │ C1-2 索引过滤 │  │ C1-3 preset 导入 │
  │ +提示词注入   │  │ +白名单+原子落盘 │
  └───────┬──────┘  └──────┬──────────┘
          │                │
          └───────┬────────┘
                  │
        ┌─────────▼─────────┐
        │ C1-4 页签+测试+门禁│
        └───────────────────┘
```

**并行度**：B 线 3 人（后端/前端/门禁）、C1 线 2 人（后端/前端），门禁工程师跨两条线。**关键路径 = B1 → B2/B3 → B4 → B5**（约 5–6 个工作日）。

### 4.2 拆卡总表

| 卡 | 内容 | 依赖 | 预估人日 | 归属 |
|---|---|---|---|---|
| B1 | `bot_usage_ledger` / `bot_cost_quotas` 实体 + DAO + SCHEMA + 索引声明 + `reconcile` 计数改写 | — | 1.5 | 后端 |
| B2 | 三个写入点（例程轮 / 委派轮 / 交互轮）+ `execute_with_usage` 回传 | B1 | 2 | 后端 |
| B3 | 费用配额解析（只收紧）+ 事后熔断 + 5 命令 + 4 码 + TS DTO | B1 | 2 | 后端 |
| B4 | Bot 详情页「用量」页签 + 部门弹窗「用量」页签 + store + i18n 32 键中的 17 键 | B3 | 1.5 | 前端 |
| B5 | Rust 测试 + e2e + 契约审计扩展 + 门禁 | B2,B3,B4 | 1.5 | 门禁 |
| C1-1 | `bot_skill_bindings` 实体 + DAO + SCHEMA + `BotWorkspace::skills_root()` | — | 1 | 后端 |
| C1-2 | `SkillIndex` botId 作用域过滤（Bot 内优先）+ `<bot-skills>` 提示词段 | C1-1 | 1.5 | 后端 |
| C1-3 | `bot-skill-preset.v1` 解析 + 白名单过滤 + 原子落盘 + 3 命令 | C1-1 | 1.5 | 后端 |
| C1-4 | Bot 详情页「技能」页签 + i18n 15 键 + Rust 测试 + e2e + 门禁 | C1-2,C1-3 | 1.5 | 前端 + 门禁 |

### 4.3 P5-b 逐卡

#### B1 — 用量账本与费用配额表（实体 / DAO / SCHEMA / 索引）

**落点**：
- 新增 `axagent-core/crates/entities/src/bot_usage_ledger.rs`、`bot_cost_quotas.rs`；加入 `entities/src/lib.rs`；
- 新增 `axagent-infra/crates/dao/src/repo/bot_usage_ledger.rs`、`bot_cost_quota.rs`；`repo/mod.rs` 导出；
- `axagent-infra/crates/dao/tests/schema_compliance.rs` 登记两表全列；
- `axagent-infra/crates/dao/src/reconcile/extras.rs`：`MULTI_COL_INDEXES` 加 `bot_cost_quotas` 的 `UNIQUE(scope, scope_id)`；`EXPECT_MULTI` 38 → 40；`total_declarations` 83 → 85；用例 `l2_total_is_eighty_three` → `l2_total_is_eighty_five`，断言 85。

**依赖 crate**：`axagent-entities`、`axagent-dao`、`axagent-harness`（DTO 类型）。

**测试**：`cargo test -p axagent-dao --lib bot_` —— 账本 append-only 语义、额度 upsert/幂等删除、两 scope 互不影响。

#### B2 — 三个写入点 + 执行器回传用量

**落点**：
- `src-tauri/src/bot_routine_runner.rs`：在 `run_bot_non_interactive_turn_with_context` 的 `summary.usage` 处（现 :269-281）回传用量（ADR-P5-8 要求新增带回传的变体或改返回值为结构体）；
- `src-tauri/src/bot_assignment_runner.rs`：`AssignmentTurnExecutor` 新增 `execute_with_usage` + 默认实现；`RoutineTurnExecutor` 覆写；`ConsultRunner::consult`（:359+）同走新方法；
- `src-tauri/src/commands/agent/mod.rs`：在既有 `estimate_cost_usd` 调用处（:2590）**增写一条账本行**（带 `botId` / `conversationId`，`origin='interactive'`）；
- 账本写入用 `usage_pricing::ModelPricing::cost_for(TokenUsage).total_cost_usd()`（**四类分价**，ADR-P5-4），并落 `pricing_source`。

**关键约束**：`execute` / `execute_with_context` 签名**逐字节不变**（P2-b Fake 零改动）；用量 `None` ⇒ **不写行**（不写 0 行）。

**测试**：`cargo test --manifest-path src-tauri/Cargo.toml --test bot_assignment --test bot_delegation_chain` 零新增红 + 新增「委派轮写账本」「交互轮写账本」用例。

#### B3 — 费用配额解析 + 事后熔断 + 命令 + 错误码

**落点**：
- `src-tauri/src/bot_cost.rs`（新文件，与 `bot_quota.rs` **平行**，不改造它）：解析窗口内用量 → 判定是否 `breached`；
- `src-tauri/src/commands/bot_cost.rs`（新文件）：5 条命令；
- `src-tauri/src/commands/error_code.rs` `pub mod bot`：4 个新码；
- `axagent-core/crates/harness/src/types/bot.rs` + `src/types/bot.ts`：`BotCostQuota` / `BotCostQuotaInput` / `BotUsageEntry` / `BotUsageFilter` / `BotUsageSummary`；
- 熔断接入点：新建编排 run 前（`bot_workflow_runner.rs` `start_run`，与 P4 `check_run_allowed` **并列**，不合并）与新建例程轮 / 委派轮前。

**关键约束**：熔断**只拦新建**，已启动的 run 跑完（ADR-P5-3）；交互轮**不拦**（只记账）。

**测试**：Rust 单测覆盖「未超限放行 / 恰好等于上限被拒 / 超限被拒 / 无配额行不拦 / 部门 scope 汇总全部 Bot / Bot scope 只算自己」。

#### B4 — 前端「用量」页签 + store + i18n

**落点**：
- `src/stores/feature/botStore.ts`：新增 `botCostQuota` / `botUsageSummary` / `botUsageEntries` 状态与 action；
- `src/components/bot/BotUsagePanel.tsx`（新）：Bot 维度用量 + 配额编辑（复用 `BotQuotaPanel.tsx` 的表单形态）；
- `src/pages/BotsPage.tsx`：`BotDetail` 注入 `usagePanel`；部门弹窗加「用量」页签（`BotDepartmentModal.tsx` 的 `departmentModalTab` 扩一档 `"usage"`）；
- `src/i18n/locales/*.json`（11 语言）：`bot.usage.*` 17 键；
- `scripts/check-bot-i18n-keys.mjs`：`BOT_I18N_KEYS` 加 17 条。

**前端路由**：**无新路由**（在 `/bots` 页内的页签），不新增 `pageRegistry` 条目。

#### B5 — Rust 测试 + e2e + 契约审计 + 门禁

**落点**：
- `src-tauri/tests/bot_cost.rs`（新）：Rust 集成测试；
- `e2e/bot-cost.spec.ts`（新）：UI e2e（用量面板显示「估算」标识、超限徽标、配额保存回显）；
- `scripts/lib/bot-ipc-audit.mjs`：新增 `P5_COMMANDS` 段（8 命令 + DTO + 4 码 + `execute_with_usage` + 三表 SCHEMA + 四类分价引用）；
- `scripts/check-contracts.mjs`：sources 加 `p5:` 块；`--selftest=bot` 加 4 条单点破坏自检。

### 4.4 P5-c1 逐卡

#### C1-1 — 归属表 + `BotWorkspace::skills_root()`

**落点**：`entities/src/bot_skill_bindings.rs` + `entities/src/lib.rs`；`dao/src/repo/bot_skill_binding.rs` + `repo/mod.rs`；`schema_compliance.rs` 登记；`reconcile/extras.rs` `MULTI_COL_INDEXES` 加 `UNIQUE(bot_id, skill_name)`（⇒ `EXPECT_MULTI` 39 → 40，**与 B1 的计数叠加**）；`axagent-ai/crates/agent/src/bot_workspace.rs` 增 `skills_root()`（`<root>/skills`）。

> **计数合并提示**：B1 把 `EXPECT_MULTI` 38→39、总数 83→84；C1-1 再把 39→40、总数 84→85。**两张卡必须在同一次提交里改计数**，否则中间态 CI 会红。最终值：`EXPECT_MULTI = 40`、`total_declarations = 85`、用例名 `l2_total_is_eighty_five`。

#### C1-2 — `SkillIndex` 作用域过滤 + `<bot-skills>` 提示词段

**落点**：
- `axagent-runtime/crates/tools/src/tools/skill/mod.rs`：`SkillIndex::rebuild` 增扫 `~/.axagent/bots/<botId>/skills/`，条目带 `scope`（`global` / `bot`）与 `owner_bot_id`；**同名时 Bot 内优先**（ADR-P5-5）；
- `src-tauri/src/bot_routine_runner.rs` 的 `build_bot_system_prompt`（:42-45）：追加 `<bot-skills>` 段，**只列名字与一行描述，不注入全文**；
- `src-tauri/src/bot_context.rs`：交互轮的 Bot 上下文同样追加 `<bot-skills>` 段（保持交互/非交互一致）。

**关键约束**：全局技能对所有 Bot **永远可见**，行为逐字节等于今天；`skill_states` 全局启停语义不变。

#### C1-3 — preset 导入（解析 + 白名单 + 原子落盘）+ 3 命令

**落点**：
- `src-tauri/src/commands/bot_skill.rs`（新）：`list_bot_skills` / `set_bot_skill_enabled` / `import_bot_skill_preset`；
- preset 解析与落盘：`axagent-ai/crates/agent/src/bot_workspace.rs` 或 `src-tauri/src/bot_skill.rs`（**原子写沿用 `write_soul_atomic` 的 tmp+rename 手法**）；
- 白名单来源：`~/.axagent/config.yaml` 的 `skills.preset_whitelist`（**默认空 = 不过滤**，不改变现状行为）。

**关键约束**：整包校验通过才落盘；白名单外的条目**跳过并回报**（不整包失败）；不污染全局库。

#### C1-4 — 前端「技能」页签 + i18n + 测试 + 门禁

**落点**：
- `src/components/bot/BotSkillPanel.tsx`（新）：技能列表（来源标签 / 遮蔽标记 / 缺失标记 / 本 Bot 启停）+ preset 导入；
- `src/pages/BotsPage.tsx`：`BotDetail` 注入 `skillsPanel`（页签接线仿 `BotDetail.tsx:231-234` 的注入面板模式）；
- `src/i18n/locales/*.json`（11 语言）：`bot.skills.*` 15 键；`BOT_I18N_KEYS` 加 15 条（185+32 = 217）；
- `e2e/bot-skills.spec.ts`（新）；`src-tauri/tests/bot_skill.rs`（新）。

**前端路由**：**无新路由**（`/bots` 页内页签）。

## 5. 冻结约束（实现线不得越界）

1. **不改 P1–P4 冻结口径**：P3 的 2/900/12、P2-b 的 4/8/300、P4 六个运行上限与 `min` 只收紧语义、P4 待答四态与三码、P1 线程归属 —— 一律不改。
2. **既有表零变更**：三张新表之外**一列不加**（MODEL-P5 §5）。
3. **执行器契约向后兼容**：`execute` / `execute_with_context` 签名不变，新增方法走默认实现（ADR-P5-8）。
4. **账本 append-only**：只 INSERT，不 UPDATE 不 DELETE（ADR-P5-2）。
5. **熔断只拦新建**：已启动的 run 跑完；交互轮不拦（ADR-P5-3）。
6. **费用是估算**：UI 必须带「估算」标识；不做对账承诺（ADR-P5-4）。
7. **全局技能兜底不变**：全局技能对所有 Bot 可见；Bot 内同名优先；`skill_states` 全局启停不动（ADR-P5-5）。
8. **白名单并入 C1**：preset 导入的过滤规则，与导入同一条路径（ADR-P5-6）。
9. **不做 A**：多 Bot 协作运行时不在本批（ADR-P5-7）。
10. **i18n 落盘顺序**：先 locale → 再 `error_code.rs` → 再 `BOT_I18N_KEYS` → 最后组件引键。

## 6. 开放项清单（无需用户裁决，已按默认冻结）

| # | 事项 | 冻结值 | 依据 |
|---|---|---|---|
| O-1 | 配额 scope 集合 | bot + department 两个 | ADR-P5-3；部门是现有唯一组织维度 |
| O-2 | 计量窗口默认值 | 168 小时（7 天） | ADR-P5-3；`window_hours` 可配 |
| O-3 | 「0 = 不限」语义 | 与 `BudgetConfig` 逐字一致 | ADR-P5-3 |
| O-4 | 两个上限都为 0 时 | 归一为「不限」（删行） | IPC-P5 §1 |
| O-5 | 交互轮是否被熔断 | **不拦**（只记账） | ADR-P5-3 |
| O-6 | 熔断状态的存储 | **不存**，由用量查询派生 | ADR-P5-3（无状态一致性问题） |
| O-7 | 账本的 `department_id` | 写入时快照 | ADR-P5-2 |
| O-8 | preset 格式的真源 | 本仓 `SKILL.md` frontmatter 约定（OMB 原文未取回） | GAP-P5 F-1 |
| O-9 | 白名单默认内容 | 空（不过滤） | ADR-P5-6 |
| O-10 | 技能正文注入 | 只列名字与一行描述 | ADR-P5-6 |

**无阻塞项**：本批不存在「无法默认」的产品取舍。若后续取回 OMB `presets.md` 原文导致格式不兼容，只做**格式对齐**（O-8），不改语义。

## 7. 验收条件（收口七条 + 门禁原文）

### 7.1 收口七条

| # | 条件 | 判定证据 | 签字 |
|---|---|---|---|
| ① | D1–D6 已落值；冻结包六份文档落盘并两视图登记 INDEX；状态已改 `frozen-for-P5` | 本文头部 + `docs/INDEX.md` 两视图各一行 | 架构 ✅（2026-11-25） |
| ② | B1–B5 + C1-1…C1-4 全绿且提交链完整；新表 reconcile 幂等；旧库升级零破坏 | `git log --oneline`；`cargo test ... schema_compliance`；reconcile 重跑零 diff | 待签 |
| ③ | 用量账本 append-only：只 INSERT；会话清理后账本仍在；`department_id` 是写入时快照 | `bot_cost` Rust 测试断言「追加后行数只增不减」「删会话后账本仍在」 | 待签 |
| ④ | 熔断语义正确：未超限放行 / 恰等于上限被拒 / 超限被拒 / 已启动的 run 跑完 / 交互轮不拦；两个 scope 互不串 | `bot_cost` Rust 测试 + `e2e/bot-cost.spec.ts` | 待签 |
| ⑤ | 技能归属正确：全局技能对所有 Bot 可见；Bot 内同名优先；Bot 之间互不可见；preset 整包原子导入；白名单外跳过并回报 | `bot_skill` Rust 测试 + `e2e/bot-skills.spec.ts` | 待签 |
| ⑥ | P1–P4 既有回归零新增红；执行器旧签名未变；i18n/契约/门禁全绿 | §7.2 命令逐条 exit 与摘要 | 待签 |
| ⑦ | 未改 P1–P4 冻结口径；费用列带「估算」；未 push | `git diff -- docs/bot-integration/P1 P2 P3 P4` 无未记录改动；`git status` 无待推送 | 待签 |

### 7.2 门禁命令原文（WSL ext4）

见 `IPC-P5.md` §7（同源，此处不重复粘贴，避免两处漂移）。执行顺序：数据层 → P5 新增 Rust 测试 → P1–P4 回归 → 契约与 i18n → 格式 → 前端单测与 e2e。

**额外必跑**（本批特有）：
```bash
# reconcile 计数（B1 + C1-1 的合并值）
cargo test --manifest-path axagent-infra/Cargo.toml -p axagent-dao --lib reconcile::extras::counts

# 8 个 workspace 逐个 fmt（repo 根无 Cargo.toml，必须逐 workspace 跑）
for w in axagent-core axagent-infra axagent-quant axagent-ai axagent-runtime axagent-kit axagent-plugins src-tauri; do
  cargo fmt --manifest-path "$w/Cargo.toml" --all -- --check || echo "FMT FAIL: $w"
done
```

### 7.3 收口判定

P5 收口需同时满足 ①–⑦。**分节签字**：B 线（C1 线）任一节 FAIL 不影响另一节 PASS，但**必须在 `ACCEPTANCE-P5.md` 写明哪一节未完成**，不得整体宣称「P5 完成」。

## 8. 风险与反转成本

| 风险 | 说明 | 缓解 |
|---|---|---|
| 事后熔断有滞后性 | 用户可能在超限当轮仍跑完一轮 | UI 明写「只拦截新建」（`bot.usage.quotaHint`）；ADR-P5-3 已冻结该语义 |
| 费用口径不准 | 跨 provider 计价不统一 | 标「估算」+ `pricing_source` 落库；不做对账承诺 |
| 缓存 token 漏算 | 旧 `estimate_cost_usd` 只算 input/output | 账本强制用 `usage_pricing` 四类分价（契约审计第 6 条钉住） |
| 执行器契约污染 P2-b | 改 `execute` 签名会波及 P2-b Fake 与全部 P3 测试 | ADR-P5-8：新增方法 + 默认实现，旧签名逐字节不变 |
| 技能索引改动波及全局 | `SkillIndex` 是所有 Bot 与通用聊天的公共路径 | 全局条目路径**零改动**，只加过滤；Bot 条目是新来源 |
| 归属表与目录不一致 | 手删文件但归属行还在 | 目录为准，UI 标「缺失」（ADR-P5-5） |
| preset 格式与 OMB 原文不符 | 本轮未取回 OMB 原文 | O-8：格式对齐留待取回后；语义不变 |
| 账本体积增长 | 每轮一行 | 本批不做归档（留 P6）；append-only 换审计价值 |

## 9. 变更记录

| 日期 | 变更 |
|---|---|
| 2026-11-25 | 冻结初稿：六份文档落盘，D1–D6 按用户裁决落值；两视图登记 INDEX |
