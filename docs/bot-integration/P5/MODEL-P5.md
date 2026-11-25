# MODEL-P5 — P5 数据模型变更（结论：新增 3 表，既有表零变更）

> 状态：frozen-for-P5 ｜ 日期：2026-11-25 ｜ 基线：`fedf00c17` + 本地三提交（`b0a43360e` / `1c7e75b63` / `afe255c40`）｜ 前置：`MODEL.md`（P1）、`MODEL-P2.md`、`MODEL-P2B.md`、`MODEL-P3.md`、`MODEL-P4.md`、`ADR-P5.md`
>
> 铁律沿用 P2/P3/P4：`MIGRATIONS = &[]`；只走声明式 reconcile（CreateTable / AddColumn + DEFAULT）；实体加入 `entities/src/lib.rs` 后 `dao/build.rs` 自动注册；`dao/tests/schema_compliance.rs` 的 `SCHEMA` 清单必须同步登记；**单列唯一索引**走 `reconcile/extras.rs` 的 `UNIQUE_INDEXES` 清单（P4 已立此先例，`extras.rs` 的 `UNIQUE_INDEXES` 与 `EXPECT_UNIQUE_INDEXES = 2`），复合唯一索引走 `MULTI_COL_INDEXES`。
>
> **既有表零变更**是本批的硬承诺：`bot_assignments` / `bot_workflow_runs` / `bot_departments` / `bot_department_quotas` / `bots` / `conversations` / `messages` / `agent_sessions` **一列不加**。用量归因走**新账本表**，不靠给既有表加 token/cost 列（ADR-P5-2）。

## 1. 总览

| 变更 | 对象 | 目的 |
|---|---|---|
| 新增表 | `bot_usage_ledger` | append-only 用量账本（每 Bot 轮一行，四类 token + 估算费用） |
| 新增表 | `bot_cost_quotas` | 费用/Token 配额（Bot scope + 部门 scope 各一行），`UNIQUE(scope, scope_id)` |
| 新增表 | `bot_skill_bindings` | 每 Bot 技能归属声明（`UNIQUE(bot_id, skill_name)`） |
| 既有表 | 全部零变更 | 见 §4 排除理由 |
| 既有目录 | `~/.axagent/bots/<botId>/skills/` | 新增子目录（不是表；沿用 `BotWorkspace` 根） |

## 2. `bot_usage_ledger`（ADR-P5-2 / ADR-P5-4）

| 列 | 类型 | 约束 | 语义 |
|---|---|---|---|
| `id` | TEXT | PK | UUID |
| `bot_id` | TEXT | NOT NULL, INDEXED | 花钱的 Bot |
| `department_id` | TEXT | NULL, INDEXED | **写入时快照**的部门归属（Bot 无部门时 NULL；事后换部门不改历史） |
| `conversation_id` | TEXT | NULL, INDEXED | 交互轮的会话 id；例程/委派轮为占位会话 id |
| `execution_id` | TEXT | NULL, INDEXED | 例程/委派轮的临时会话 id（`bot-routine-<uuid>`） |
| `workflow_run_id` | TEXT | NULL, INDEXED | 所属编排 run（D4 下编排步骤经 assignment 上送时写） |
| `assignment_id` | TEXT | NULL, INDEXED | 所属委派 assignment |
| `model_id` | TEXT | NOT NULL | 实际使用的模型 id（估算口径的输入） |
| `origin` | TEXT | NOT NULL, DEFAULT 'routine' | `interactive` / `routine` / `assignment` |
| `input_tokens` | INTEGER | NOT NULL, DEFAULT 0 | 输入 token |
| `output_tokens` | INTEGER | NOT NULL, DEFAULT 0 | 输出 token |
| `cache_creation_tokens` | INTEGER | NOT NULL, DEFAULT 0 | 缓存写入 token |
| `cache_read_tokens` | INTEGER | NOT NULL, DEFAULT 0 | 缓存命中 token |
| `total_tokens` | INTEGER | NOT NULL, DEFAULT 0 | 冗余合计（= 四项之和；写入时固化，便于查询不 join） |
| `estimated_cost_usd` | REAL | NOT NULL, DEFAULT 0 | **估算**费用（四类分价分别计算后求和） |
| `pricing_source` | TEXT | NOT NULL, DEFAULT 'heuristic' | `model` / `config` / `heuristic` / `builtin`（ADR-P5-4 的四档来源） |
| `created_at` | BIGINT | NOT NULL | epoch millis |

**写入语义**：**只 INSERT**。不 UPDATE、不 DELETE（ADR-P5-2）。账本与会话生命周期解耦 —— 例程占位会话被清理后账本仍在。

**费用口径**：`estimated_cost_usd` = `usage_pricing::ModelPricing::cost_for(TokenUsage)` 的 `total_cost_usd()`，四类分价分别计算（`input` / `output` / `cache_creation` / `cache_read` 各用自己的每百万单价）。**不用** `commands/agent/pricing.rs` 的 `estimate_cost_usd` —— 它只算 input/output，忽略缓存 token（`GAP-P5` B-7）。

**索引**：`bot_id` / `department_id` / `conversation_id` / `execution_id` / `workflow_run_id` / `assignment_id` 为单列索引（查询过滤组合按 `bot_id` + `created_at` 最常用）。**不建复合索引**：账本是纯 append，查询走单列 + 时间范围即可；复合索引会让每次 INSERT 多一处维护成本。

**DAO**（`axagent-infra/crates/dao/src/repo/bot_usage_ledger.rs`）：
- `append(&BotUsageLedgerEntry) -> Result<()>`（单条 INSERT）
- `list(filter: &BotUsageFilter) -> Result<Vec<BotUsageLedgerEntry>>`（`bot_id` / `department_id` / `origin` / `workflow_run_id` / `assignment_id` / `since` / `until`，全部可选；按 `created_at DESC`）
- `sum_by_bot(db, bot_id, since, until) -> Result<BotUsageTotals>`（汇总：四类 token 分别求和 + 费用求和）
- `sum_by_department(db, department_id, since, until) -> Result<BotUsageTotals>`
- `sum_by_workflow_run(db, workflow_run_id) -> Result<BotUsageTotals>`（run 级汇总，ADR-P5-2 的派生值，不新表）

`BotUsageTotals { input_tokens, output_tokens, cache_creation_tokens, cache_read_tokens, total_tokens, estimated_cost_usd, entry_count }`。

## 3. `bot_cost_quotas`（ADR-P5-3）

| 列 | 类型 | 约束 | 语义 |
|---|---|---|---|
| `id` | TEXT | PK | UUID |
| `scope` | TEXT | NOT NULL | `bot` / `department`（两个 scope；不引入第三种） |
| `scope_id` | TEXT | NOT NULL | scope 对应的 id（bot id 或 department id） |
| `max_total_tokens` | INTEGER | NOT NULL, DEFAULT 0 | 窗口内 token 上限；**0 = 不限**（与 `BudgetConfig` 的 0 语义一致） |
| `max_cost_usd` | REAL | NOT NULL, DEFAULT 0 | 窗口内费用上限（估算 USD）；**0 = 不限** |
| `window_hours` | INTEGER | NOT NULL, DEFAULT 168 | 计量窗口（小时）。默认 168 = 7 天 |
| `is_default` | — | — | **不入库**：无行 = 不限，由命令层合成 |
| `created_at` | BIGINT | NOT NULL | epoch millis |
| `updated_at` | BIGINT | NOT NULL | epoch millis |

**唯一约束**：`UNIQUE(scope, scope_id)` —— 一个 scope 对象至多一行配额。**复合唯一索引**（2 列）⇒ 走 `MULTI_COL_INDEXES` 清单（P2-b `bot_department_members` 同款先例，`UNIQUE(department_id, bot_id)`）。

**「0 = 不限」的语义**：与 `commands/agent/pricing.rs` 的 `BudgetConfig`（`max_cost_per_day_usd: 0` 表示不限，`check_token_budget` 的 `max_tokens == 0` 短路）逐字一致，避免两套「0 的含义」。

**只收紧**：费用配额是**上限**，不存在「配额放大默认值」的路径；生效判断是「当前窗口用量 ≥ 上限 ⇒ 拒新建」（ADR-P5-3）。与 P4 的 `min(定义值, 配额)` 不同 —— P4 收紧的是**定义值**，P5 收紧的是**是否允许新建**，两者都遵守「不放大」原则。

**DAO**（`axagent-infra/crates/dao/src/repo/bot_cost_quota.rs`）：
- `get_by_scope(db, scope, scope_id) -> Result<Option<BotCostQuota>>`
- `upsert(&BotCostQuotaInput) -> Result<BotCostQuota>`（按 `(scope, scope_id)` 判存在）
- `delete_by_scope(db, scope, scope_id) -> Result<u64>`（幂等）
- `list_all(db) -> Result<Vec<BotCostQuota>>`（诊断用，不在热路径）

**删除级联**：删 Bot ⇒ 同事务删该 Bot 的 `scope='bot'` 配额行 + `bot_skill_bindings` 行；删部门 ⇒ 同事务删该部门的 `scope='department'` 配额行。**不删账本**（账本是历史事实）。

## 4. `bot_skill_bindings`（ADR-P5-5 / ADR-P5-6）

| 列 | 类型 | 约束 | 语义 |
|---|---|---|---|
| `id` | TEXT | PK | UUID |
| `bot_id` | TEXT | NOT NULL, INDEXED | 归属 Bot |
| `skill_name` | TEXT | NOT NULL | 技能名（与目录名一致） |
| `source` | TEXT | NOT NULL, DEFAULT 'workspace' | `workspace`（用户手放） / `preset`（preset 导入） |
| `preset_id` | TEXT | NULL | 来源 preset 的标识（`source='preset'` 时非空） |
| `enabled` | INTEGER | NOT NULL, DEFAULT 1 | **本 Bot 停用标记**（与全局 `skill_states` 分层，ADR-P5-5） |
| `created_at` | BIGINT | NOT NULL | epoch millis |
| `updated_at` | BIGINT | NOT NULL | epoch millis |

**唯一约束**：`UNIQUE(bot_id, skill_name)` —— 复合 2 列 ⇒ 走 `MULTI_COL_INDEXES`。

**目录即真相源**：目录 `~/.axagent/bots/<botId>/skills/<skillName>/SKILL.md` 存在 ⇒ 技能在；归属表只作**归属声明与审计**（回答「哪些是显式装的」）。两者不一致时**以目录为准**，UI 标「缺失」。

**与全局 `skill_states` 的分层**：`skill_states`（`name` / `enabled` / `updated_at`）是**全局**启停，语义不变；本表的 `enabled` 只作用于该 Bot 自己的技能。两者**不联动**：全局停用一个技能 ⇒ 所有 Bot 都看不到它；本 Bot 停用 ⇒ 只影响该 Bot。

**DAO**（`axagent-infra/crates/dao/src/repo/bot_skill_binding.rs`）：
- `list_by_bot(db, bot_id) -> Result<Vec<BotSkillBinding>>`
- `upsert(&BotSkillBindingInput) -> Result<BotSkillBinding>`
- `delete(db, bot_id, skill_name) -> Result<u64>`
- `delete_by_bot(db, bot_id) -> Result<u64>`（删 Bot 级联）
- `set_enabled(db, bot_id, skill_name, enabled) -> Result<bool>`

## 5. 既有表排除理由

| 表 | 为何零变更 |
|---|---|
| `bot_assignments` | 用量归因走账本表；给 assignment 加 token/cost 列会要求所有写入路径同步维护，且 P3 的五态与断言零变更承诺（ADR-P5-7）不能破 |
| `bot_workflow_runs` | run 级汇总是**派生值**（`sum_by_workflow_run`），不落冗余列；避免两处真相不一致 |
| `bot_departments` | 费用配额独立生命周期与校验规则；塞 `metadata_json` 失去类型校验与索引（同 P4 `MODEL-P4` §4） |
| `bot_department_quotas` | 那是**运行**配额（六个上限）；费用配额是**用量**维度，语义、窗口、校验规则都不同，混表会让「只收紧」原则被两个口径污染 |
| `bots` | 技能归属走独立表；`bots` 不承担技能关系 |
| `conversations` | 账本已固化归属；`conversations.bot_id` 是 P1 冻结口径，不加 `opened_by`（ADR-P5-7） |
| `messages` / `agent_sessions` | 既有 token/cost 列是**会话级**的；Bot 维度归因必须写入时固化（会话会被清理），不能靠 join |
| `cron_jobs` | 熔断发生在创建 run 前，不需要在任务表上留字段 |

## 6. SCHEMA 与 reconcile 登记清单（实现线必须全做）

1. **新实体**（3 个）：
   - `axagent-core/crates/entities/src/bot_usage_ledger.rs`
   - `axagent-core/crates/entities/src/bot_cost_quotas.rs`
   - `axagent-core/crates/entities/src/bot_skill_bindings.rs`
   三文件加入 `axagent-core/crates/entities/src/lib.rs`。
2. **索引声明**（`axagent-infra/crates/dao/src/reconcile/extras.rs`）：
   - `MULTI_COL_INDEXES` 加 **1 条**：`bot_cost_quotas` 的 `UNIQUE(scope, scope_id)`（2 列）；
   - `MULTI_COL_INDEXES` 加 **1 条**：`bot_skill_bindings` 的 `UNIQUE(bot_id, skill_name)`（2 列）；
   - ⇒ `EXPECT_MULTI` 由 38 改为 **40**；
   - `bot_usage_ledger` **无唯一索引**（append-only，id 主键足够）⇒ `UNIQUE_INDEXES` 仍是 2 条，`EXPECT_UNIQUE_INDEXES` 不变；
   - `total_declarations()` 的冻结总数由 83 改为 **85**；对应用例 `l2_total_is_eighty_three` **改名为 `l2_total_is_eighty_five`**，断言值 85。
3. **`SCHEMA` 清单**（`axagent-infra/crates/dao/tests/schema_compliance.rs`）：登记三表全列（列序与 §2/§3/§4 一致）。
4. **`repo/mod.rs`**：导出三个新 repo。
5. **旧库升级**：reconcile 自动建空表；**无数据迁移、无回填**（账本从升级后开始记；既有历史用量不可追回 —— 如实记录为边界）。

## 7. 回滚

三表均为纯增量：回滚 = 删实体注册 + 删 SCHEMA 行 + 删索引声明（表留在库中不影响旧代码，reconcile 不会 drop）。账本数据在回滚后**留在库里不丢失**（append-only 的另一面）。`~/.axagent/bots/<botId>/skills/` 目录是纯文件，回滚不删目录（用户数据）。
