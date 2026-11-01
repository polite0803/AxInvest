# 审计：`evolution_execution_stats` 历史缺口「回填」可行性 —— 结论：**无对象可回填，且物理上不可回填**

- 日期：2026-09-17
- 触发：用户指令「2、回填」（对应 `AUDIT-db-upsert-dialect-2026-09-17.md` §8 登记的第 2 项：*该表修复前每次 upsert 都失败 ⇒ 历史统计可能有缺口；是否回填属产品语义，需用户定*）
- 结论：**缺口大概率 = 0；且即使不为 0，库内也不存在任何可依的回填源** ⇒ 本轮**未写入任何数据**（写入即伪造）
- 交付：口径正确的可执行回填工具 `scripts/backfill-evolution-stats.mjs`（默认 dry-run）
- 数据库：`postgres://…@localhost:5432/axinvest`（PG 18.4，来自 `~/.axagent/db_config.json`：`db_type=postgres`）

---

## §0 三句话结论

1. **缺口大概率是 0，不是「丢失了多少」的问题**：`generated_tools` 在**两个库中都是 0 行**，而 `evolution_execution_stats` 的唯一写入方只服务「进化产物」⇒ 从未有进化产物存在过，写入方也就不可能被触发过。
2. **即使缺口不为 0，也补不了**：库里**没有任何一张表**记录过 `(conversation_id, 进化产物 tool_id, 成败)` 这个三元组 —— 逐张候选源已排除（§2）。
3. **所以「回填」的正确执行结果是「不写入」**：任何"补数"都只能是编造，会直接污染贝叶斯决策器的「真实执行证据」。

---

## §1 判据设计（先定"什么叫可回填"，再查库）

回填的**充要条件**不是"表是空的"，而是：

> **存在一个独立数据源，它记录了与目标表同一粒度、同一语义的事实。**

因此本轮不是去"想办法凑数"，而是**穷举候选源并逐一证伪**：

| 判据 | 要求 |
|---|---|
| C1 粒度 | 必须是 `(会话, 工具)` 级，而非 workflow 级 / cron 级 / 轨迹步骤级 |
| C2 成败可判 | 必须有可靠的成功/失败信号（布尔或二值 status） |
| C3 会话可归属 | `conversation_id` 必须**非空**且是真会话 id（目标表主键第一列） |
| C4 语义匹配 | 记录的必须是**进化产物**工具，不是内置工具 |

四条全中才可用作源。

---

## §2 取证：目标表 + 全部候选源（真库实测）

### 2.1 目标表

```sql
SELECT count(*) FROM evolution_execution_stats;   -- 0
```

列定义（`information_schema`）：`conversation_id text NOT NULL` / `tool_id text NOT NULL` / `usage_count integer NOT NULL DEFAULT 0` / `successes` / `failures`，复合主键 `(conversation_id, tool_id)`。

### 2.2 候选源逐一证伪

| 表 | 行数 | C1 粒度 | C2 成败 | C3 会话 | C4 语义 | 判定 |
|---|---|---|---|---|---|---|
| `tool_call_logs` | **0** | ✅ | ✅ `success int` | ✅ `conversation_id` | ✅ 任意工具 | **结构合规但空** |
| `tool_executions` | **0** | ✅ | ⚠ 5 值 `status` | ✅ | ✅ | 结构合规但空 |
| `audit_log` | 13 | ✅ | ✅ `success int` | ❌ **全为 null** | ❌ 只有内置工具 | 排除 |
| `trajectory_steps` | 53 | ⚠ 步骤级 | ❌ 无 `success` 字段 | ⚠ 只有 `trajectory_id` | — | 排除 |
| `capability_stats` | 63 | ❌ workflow 级 | ⚠ `success_count` 全 0 | ❌ 无 | — | 排除 |
| `cron_job_history` | 2 | ❌ 任务级 | ✅ | ❌ `task_id` | — | 排除 |
| `trajectory_patterns` | 2010 | ❌ 模式级 | ⚠ `success_rate` 全 0 | ❌ | — | 排除 |
| `trajectory_trajectories` | 15 | ❌ 轨迹级 | ⚠ `outcome` 字符串 | ⚠ `session_id` | — | 排除 |
| `trajectory_rewards` | 63 | ❌ 步骤奖励 | ❌ 连续值 | ❌ | — | 排除 |
| `trajectory_learned_patterns` | **0** | — | ✅ | ❌ | — | 排除 |
| `trajectory_skill_executions` | **0** | — | ✅ | — | — | 排除 |
| `opc_rl_experiences` | **0** | ❌ workflow 级 | ✅ | ❌ | — | 排除 |
| `workflow_execution_stats` | 291 | ❌ 执行级 | ⚠ `status` | ❌ | — | 排除 |
| `workflow_tools` | **0** | ✅ | ❌ 只有滚动 `success_rate`(f64) | ❌ `workflow_id` | ⚠ 工作流工具 | 排除 |

**关键细节（三条易被想当然的地方）**：

- **`trajectory_steps` 有 `tool_calls` / `tool_results` 两个 JSON 列，看着像源，实则不是**：`tool_calls` 仅 1 行非空（`CapabilityView`），`tool_results` **全部为 `null`**（抽样 8 行全 null），且无成败字段 ⇒ 无法判定。
- **`audit_log` 是结构上最像源的**（同时含 `tool_name` + `success` + `conversation_id`），但 13 行中 `conversation_id` **全部为 `NULL`**，工具只有 `OpcDiscoverLeads`(11) 与 `CapabilityBrowse`(2)，`success` 全为 1 ⇒ 既不可归属，也不含进化产物。
- **`capability_stats`（63 行）看似统计表**：`tool_scoped_rows = 0`（`capability_id LIKE 'tool:%'` 零行），全部是 `workflow:<template_id>`；且 `sum(total_calls)=83 / sum(success_count)=0` ⇒ 连 workflow 维度都全零。

### 2.3 旧库交叉核对（排除"数据在另一个库里"）

`C:\Users\polit\.axinvest\axagent.db`（142 MB SQLite，2026-07-13 后停用，只读打开）：

| 表 | 行数 |
|---|---|
| `generated_tools` | **0** |
| `tool_executions` | **0** |
| `evolution_execution_stats` | **表不存在**（该表晚于 SQLite 时代，符合 v122 建表的时间线） |

⇒ 「旧库里有历史数据」这条可能性也排除。

---

## §3 缺口规模判定：为什么说「大概率 = 0」

写入链唯一性（已源码取证）：

```
evolution_engine.rs:729 / :949 / :1458
   └─ .with_feedback_sink(EvolutionFeedbackSinkImpl::new(stats, Some(db)))
        └─ workflow_injections.rs:982  fn record(conversation_id, tool_id, success)
             └─ :1006  upsert_execution_feedback(&db, &conv, &tool_id, success)   ← 唯一写入点
```

调用方唯一性（已源码取证）：

```
tools/src/generated_tool.rs:129   fn report(&self, ctx, success)
        └─ 仅在 call_inner 的「真实执行」分支被调用（:142 等）
generated_tool.rs 的生产实例只来自 evolution_engine 的三处，且位于
        `match creator.create_tool_from_pattern(...) { Ok(tool) => { … } }` 的 Ok 分支内
另一处 work_engine.rs:171 的实例来自 `workflow_tools` 表（**0 行**）⇒ 不产生实例
```

而 `evolution_engine.rs:743-763` 在注册成功后会把工具持久化到 `generated_tools`（`persist_runtime_evolution`），`load_runtime_evolution_tools_impl`（`evolution_engine.rs:899`）启动时也从该表读回 ⇒ **`generated_tools` 是"进化产物是否存在过"的权威判据**。

该表 = **0 行**（PG 与旧 SQLite 皆然）⇒ **从未有进化产物被持久化** ⇒ `record` 未被触发过。

> **唯一残留窗口（如实标注）**：`register_runtime_tool` 成功后、`persist_runtime_evolution` 失败时，工具只活在内存里，仍可被调用 ⇒ 理论上可产生 `record` 调用而 `generated_tools` 仍为 0 行。
> **但该窗口无法验证也无法回填**：`tracing_subscriber::fmt()`（`lib.rs:174`、`server_main.rs:15`）**没有 `.with_writer(file)`** ⇒ 日志只进 stdout，**不落文件**，因此该路径的 `warn!("进化产物执行反馈落库失败")`（`workflow_injections.rs:1011`）**无处可查**。
> ⇒ 结论只能是「缺口 ≈ 0 但不可证明为严格 0」；**这不影响"不可回填"这个结论**（§4 独立成立）。

---

## §4 「不可回填」的论证（与 §3 无关，独立成立）

即便存在上述残留窗口，**回填依然不可能**，因为回填需要的是"独立记录过同一事实的源"，而 §2 已穷举证伪：**库内不存在任何一张含 `(非空 conversation_id, 工具名, 成败)` 且满足语义匹配的表**。

`tool_call_logs` 是唯一结构合规的候选，但它 **0 行**。根因（**附带发现，见 §6**）：它有完整写入实现（`feedback_data_lake.rs:114 insert_tool_call`）与活调用点（`agent/src/action_executor.rs:89`），却从未写入过 —— 与 `tool_executions`（实现 `repo/tool_execution.rs:43`、调用点 `tools/src/recorder.rs:40` 与 `commands/conversations/streaming.rs:606`）一样都是 0 行。

⇒ **在源为 0 行的前提下，"回填"不存在正确实现；任何写入都是凭空造数。**

---

## §5 交付：把「回填」做成可执行能力（而非编数据）

新增 `scripts/backfill-evolution-stats.mjs`（**默认 dry-run**，`--apply` 才写）：

| 设计点 | 做法 | 理由 |
|---|---|---|
| 源选择 | 仅 `tool_call_logs` | 库内唯一同时满足 C1–C3 的表 |
| **口径收窄** | `WHERE EXISTS (SELECT 1 FROM generated_tools g WHERE g.tool_name = l.tool_name AND g.source_info::jsonb ->> 'source' = 'runtime_evolution')` | 目标表**只服务进化产物**；不加此过滤会把普通工具写进去，污染贝叶斯证据 |
| 聚合 | `GROUP BY conversation_id, tool_name` → `usage_count = count(*)`、`successes = sum(success)`、`failures = count(*) - sum(success)` | 与 `ToolExecutionStats` 字段语义一一对应 |
| 幂等 | 回填用**绝对赋值**（非 `+1` 累加），`ON CONFLICT … DO UPDATE SET usage_count = EXCLUDED.usage_count, …` | 可重复执行；且 SET 内**只用 `EXCLUDED.<col>`**（单独引用 excluded 合法），**规避 2026-09-17 修复的 42702 裸列名坑** |
| 安全 | 源为 0 行时**提前退出且不 BEGIN 事务**；写入包在单个事务里，异常即 `ROLLBACK` | dry-run 与失败路径都不触碰数据 |
| 口令 | 连接串只经 `env.PG_URL`，**不进 argv** | 避免口令泄漏到进程列表 |

**实跑证据（dry-run，2026-09-17）**：

```
=== 源规模 ===
tool_call_logs            0
generated_tools           0
evolution_execution_stats 0

结论：源表 tool_call_logs 为 0 行 ⇒ **无可回填数据**（不会写入任何行）。
这不是「回填失败」，而是「没有对象」：库内不存在记录过这些工具执行的事实源。
```

- 日志：`output/verify-graph-2026-09-15/backfill-dryrun-0917.log`（`BACKFILL_EXIT=0`）
- 复核：跑后 `evolution_execution_stats` 仍为 **0 行**（工具在源为空时提前 `exit(0)`，未发起写入）

---

## §6 附带发现（**只登记，本轮未处置**）

**「记录工具执行事实」的表全线为死**：四张表都是 0 行，且**都有完整写入实现 + 至少一个活调用点**：

| 表 | 写入实现 | 已知调用点 | 实测行数 |
|---|---|---|---|
| `tool_call_logs` | `crates/dao/src/repo/feedback_data_lake.rs:114` | `crates/agent/src/action_executor.rs:89` | 0 |
| `tool_executions` | `crates/dao/src/repo/tool_execution.rs:43` | `crates/tools/src/recorder.rs:40`、`src/commands/conversations/streaming.rs:606` | 0 |
| `evolution_execution_stats` | 已修（`repo/evolution_execution_stats.rs:49`） | `src/init/workflow_injections.rs:1006` | 0 |
| `workflow_tools` | `crates/dao/src/repo/workflow_tool.rs:122` | `src/commands/work_engine.rs:111` | 0 |

⇒ 这是**独立于本轮的缺陷族**（"有实现、有调用点、运行期零写入"），需要单独一轮「写入链为何不触发」的取证（形状类似既有技能 `dead-data-source-repair`）。**本轮不扩大范围**。

**为什么它值得优先于"回填"**：只要 `tool_call_logs` 一天不写，`evolution_execution_stats` 就一天没有可回填源；且 `feedback_data_lake`（统一反馈数据湖）整体也拿不到任何工具调用数据。

---

## §7 处置纪律说明

| 项 | 处置 |
|---|---|
| 目标表写入 | **未执行**（无对象；写入即伪造） |
| 共享工具 `scripts/pg-connect.mjs` | **未改**（其 `sql` 子命令多语句静默 `undefined` 的缺陷仍在 §8.1 of `AUDIT-db-upsert-dialect-2026-09-17.md` 登记） |
| 产品语义改动（`record` 落库失败静默丢弃 / tracing 不落文件） | **未改，仅登记**（§3 残留窗口、§6） |
| 新增文件 | `scripts/backfill-evolution-stats.mjs`（新增，不改动任何既有文件） |
| 只读性 | 全部探针只读：PG 走 `SELECT` / `information_schema`；SQLite 走 `mode=ro`；`db_config.json` 仅输出脱敏摘要（口令字段只报长度与前 3 字符） |
| 日志 | 未依赖运行日志作证据（`tracing` 不落文件，见 §3） |

---

## §8 复现步骤

```powershell
$node = "C:\Users\polit\.workbuddy\binaries\node\versions\22.22.2-3\node.exe"
Set-Location "D:\OneManager\AxInvest"
$env:PG_URL = (& $node "scripts\pg-connect.mjs" url 2>$null | Select-Object -First 1)

# 1. 目标表与候选源规模
& $node "output\verify-graph-2026-09-15\probe-eestats-sources-0917.mjs"

# 2. 旧 SQLite 交叉核对（只读）
& "C:\Users\polit\.workbuddy\binaries\python\versions\3.13.12\python.exe" "output\verify-graph-2026-09-15\probe-sqlite-0917.py"

# 3. 排除 audit_log / trajectory_* 等候选源
& $node "output\verify-graph-2026-09-15\probe-auditlog-0917.mjs"
& $node "output\verify-graph-2026-09-15\probe-trajectory-sources-0917.mjs"

# 4. 回填工具（dry-run，确认无对象）
& $node "scripts\backfill-evolution-stats.mjs"

$env:PG_URL = $null
```

**探针产物**：`probe-eestats-sources-0917.json` · `probe-pg-activity-0917.json` · `probe-sqlite-0917.json` · `probe-trajectory-sources-0917.json` · `probe-auditlog-0917.json` · `backfill-dryrun-0917.log`（均在 `output/verify-graph-2026-09-15/`）

---

## §9 与前一版审计的关系

- 前一版 `AUDIT-db-upsert-dialect-2026-09-17.md`：修的是**写入方的 SQL 方言缺陷**（PG 42702，已修 + 反事实验证 + 回归锁）。
- 本文：判的是**修好之后要不要补历史数据**。结论是**不补**（无对象、无源）。
- 两者共同结论：**该表在修复后从"写必失败"变成"写会成功，但当前无内容"**；一旦进化产物出现（`generated_tools` 有行），写入链即自动生效，无需人工回填。
