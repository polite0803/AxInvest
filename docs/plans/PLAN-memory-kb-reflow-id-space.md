# PLAN — Memory → Knowledge 回流：**错 id 空间**与静默失败（单独立项）

> 立项日：2026-09-17 ｜ 来源：`docs/plans/PLAN-declarative-schema-sync.md` §六 表格 `:2916` 与 §⑨ 遗留 4（`:2959-2961`）
> 状态：**已实施**（§4.1 选项 A，见下方状态块）｜ 优先级：中（**静默空转**，不影响启动，但功能恒不产出）
> **状态（2026-09-17 16:50）**：✅ **代码已实施**（按 §4.1 **选项 A**）｜ ✅ **静态检查五层全绿** ｜ ✅ **真 PG 侧：机制实证 + 端到端探针 PASS（含「失败可见」实证）** ｜ ✅ **§6 五条判据全部达标**（见 §5e 第 6/7 节）｜
> ✅ **§5d 同型穷举已完成**（51/51 逐点核对：A 31 · B 10 · **C 3 处确证缺陷** · D 7 处同型未发作）｜ ✅ dev-deps 变更后**同源重跑两层**：`clippy -p axagent-dao --all-targets` = **全绿**（`1m 38s` / exit 0 / 0 error 0 warning）· `test -p axagent-dao --lib` = **`303 passed; 0 failed`**（24.53s / exit 0）｜
> ⇒ **本项自身（错 id 空间 + 静默失败）代码 + 验收 + 同型穷举三维均已闭环**。
> ✅ **C-#2（`save_pattern`，唯一有生产实态、且仍在无界增长的一处）已于 2026-09-17 晚按用户授权修复** —— 见 §5d-补
> （内含一条**前置勘察**：为什么**不能**图省事在 repo 层按 `name` upsert）。
> ✅ **C-#1（`upsert_relation`）· C-#3（`save_relationship` 的调用方 `commands/memory.rs`）· causal 787 已于 2026-09-17 按用户授权修复** —— 见 §5d-补2
> （C-#3 的修法判定被现场推翻过一次：缺陷在**调用方**，不在 storage —— 另一个调用方依赖「按原行 id 就地改写」语义）。
> ⏳ **仍未处理，但已全部登记**（`PLAN-declarative-schema-sync.md` 的「必须登记、本轮不擅改」表新增 4 行；其中 C-#1/#2 那两条已改为「已修」）：
> **(c) 待授权**：清 `trajectory_patterns` 存量 2038 行 + 加 `UNIQUE(name)`（删生产数据 + schema 迁移）·
> `knowledge_graph.rs:1144` 批量导入「有存在性检查却仍重复」（**有足迹**：788 行 / 20 组）·
> §5d 附带发现的 CSV 导入 id 命名空间漂移（37397 组）·
> 🆕 **暖启动空转**（`src/lib.rs:477-506`，见 §5d-补 末尾「新发现」）·
> ⚠ harness 一处**既有红**：`DATA_DRIVEN_COLUMN.evidence` 行号腐烂（现值 1748 / 真值 1751，见 §5d-补2 第 5 节）。
> ⇒ 这些属**新缺陷**，需**另行裁决**，不属本项范围。
>
> ⚠ **更新 1**：§4.2 的前置检查**已查实 = ❌ 不通过**（`upsert_entity` 不幂等）⇒ 已采纳「先做 repo 层去重再接线」，见 §4.2 / §5b。
> ⚠ **更新 2**：发现**第二个孪生调用点**（立项时漏登）⇒ 见 §1.1b；两处已**收敛到单一实现**。

---

## §1 缺陷本体（两处，必须一起修）

### 1.1 错 id 空间（语义）

`src-tauri/src/init/services.rs:270-274`：

```rust
let kb_id = if item.namespace_id.is_empty() {
    "memory_default".to_string()
} else {
    item.namespace_id.clone()
};
```

随后 `:277-286` 把它当 `knowledge_entities.knowledge_base_id` 用：

```rust
match axagent_dao::repo::knowledge_graph::upsert_entity(
    harness_state.db(), &kb_id, &name, "memory_item", "[]", confidence, None, None,
)
```

**`memory_namespaces.id` 与 `knowledge_bases.id` 是两个 id 空间**，两者都是 TEXT ⇒ 类型系统拦不住。
判据：**「同名 ≠ 同物」**——两个自由文本主键长得像不等于值域相同。

### 1.1b ⚠ 孪生调用点（**立项时漏登**，2026-09-17 补）

`src-tauri/src/commands/knowledge_graph.rs:475-532` 的 Tauri 命令
`sync_memory_to_knowledge_graph`（UI 上「同步记忆到知识图谱」，`safety = Caution`）
**把同一段逻辑复制了一份**：`:496-500` 的 `"memory_default"` / `namespace_id` 复用与
`:501` 的 `name = content 前 100 字`，与 `services.rs:270-276` **逐字相同**；`:518` 同样只 `debug!`。

⇒ ① 本项缺陷面是**两处**，不是一处（判据 **#256**：同型缺陷按量纲穷举）；
② 这会改变「谁在发作」的判断：`services.rs` 那条在周期任务里（静默空转），
而这一条是**用户可直接点出来的** ⇒ 用户点一次会看到 `{items_read: N, entities_created: 0, failures: N}`
却**看不到失败原因**（它连 `failures` 都只进返回值、不落日志）。
⇒ §5 施工必须**同时**改这两处，且两处应**收敛到同一个函数**，不要再抄第三遍。

### 1.2 错误被 `debug!` 吞掉（可观测）

`:290-296`：`upsert_entity` 失败只走 `tracing::debug!`。生产默认日志级别下等于**不可见**。

⇒ 「失败」与「没有数据」在运维视角**完全不可区分**（判据：ERROR 未必故障 / fail-open 的镜像形态——这里是 fail-silent）。

---

## §2 已实测的后果（非推测）

- 生产 PG 侧 `knowledge_entities.knowledge_base_id` 的**外键早已存在**，而 prod 实测该路径
  **一行都没插进去**（`knowledge_entities` 只有 2 个真实 KB 取值，`''` 为 0）。
- 即：**「Memory → 知识图谱」这条回流在 PG 上早已是静默空转**，不是本轮引入。
- SQLite 侧此前会插进垃圾行（无外键）；声明式引擎接管建表后，SQLite **新库**也带上了该外键
  ⇒ 同一条路径在新库上也开始失效。

引用：`PLAN-declarative-schema-sync.md:2905-2916`、`:2959-2961`。

---

## §3 可对齐的既有协议（**这是本项能干净落地的原因**）

仓库已有一套「**哨兵行**」协议，形态与本项**完全同构**：

| 位置 | 内容 |
|---|---|
| `crates/harness/src/constants.rs:433-451` | `pub mod sentinel`：`TRAJECTORY_KB_ID = "__sys_trajectory__"`、`TRAJECTORY_MEM_NS_ID = "__sys_trajectory_memory__"`、`TRAJECTORY_KB_ENABLED: i32 = 0`；文档写明这些值是**跨 crate 的 DB 协议**，权威定义收敛到 harness，消费点一律 `use` |
| `crates/dao/src/seed.rs:49-53` | `ensure_sentinels()`：幂等（`on_conflict_do_nothing()`，**不靠先查再插**）、**不分方言**（方言由连接决定） |
| `crates/trajectory/src/storage.rs` | 写 `knowledge_entities` / `knowledge_relations` 时**按其 sentinel id 归属** |

⇒ 本项**不需要发明新机制**，只需再增一个 sentinel KB 并让回流路径 `use` 它。

---

## §4 需产品裁决的问题（**开工前必须回答**）

### 4.1 memory namespace ↔ knowledge base 的映射语义

| 选项 | 形态 | 代价 | 何时选 |
|---|---|---|---|
| **A（建议）** | **单一哨兵 KB**（如 `__sys_memory_reflow__`）承载全部回流 | 最简单；与 `__sys_trajectory__` 同构；不新增运行期建行逻辑 | 默认。图谱不要求按命名空间隔离时 |
| B | 每个 memory namespace 对应一个 KB（按需 upsert 该 KB 行） | 多一条「运行期建父行」路径 ⇒ 必须处理并发/幂等；KB 列表会被系统行污染 | 需要按命名空间隔离子图时 |
| C | 映射到某个**已存在**的用户 KB | 需要一张映射表或命名约定 | 有明确业务归属时 |

**问题（需你回答）**：知识图谱里是否要求按 memory 命名空间分图？不要求 ⇒ 选 A。

### 4.2 ⚠ 复活一条死路径的**前置检查** —— **已查实：❌ 不通过**（2026-09-17）

**结论：`upsert_entity` 不是幂等（冲突键打在同一次新生成的随机 id 上），且全仓没有任何
`(kb_id, name)` 唯一约束 ⇒ 直接接线会把「恒空转」换成「数据无界增长」。接线**前**必须先给出
repo 层去重口径。**

#### ① 冲突键 = 同一次新生成的随机 id ⇒ `on_conflict` 永不触发

`crates/dao/src/repo/knowledge_graph.rs:621-622` + `:651-662`：

```rust
let id = format!("ent_{}", gen_id());            // 每次调用都是新 id
knowledge_entities::Entity::insert(am)
    .on_conflict(OnConflict::column(knowledge_entities::Column::Id)   // ← 冲突键就是这个新 id
        .update_columns([Name, LastSeenAt, MentionCount, Confidence, UpdatedAt]))
```

⇒ 语义上**等价于纯 INSERT**（没有两行会共用同一个刚生成的 id）。
⚠ 与本节原文预判的「同类教训」`upsert_relation`（`:709-710` + `:728-736`，同样是
`format!("rel_{}", gen_id())` + `on_conflict(Column::Id)`）**逐字同型** —— 是同一个洞的第 2、3 例。

#### ② 四层都查了，没有任何唯一约束能兜住

| 层 | 位置 | 结果 |
|---|---|---|
| 实体 | `crates/entities/src/knowledge_entities.rs` | 只有 `#[sea_orm(indexed)]`（普通索引），**无 `unique`** |
| L2 声明 | `reconcile/extras.rs` grep `knowledge_entities` | **无匹配**（未声明唯一约束） |
| 迁移 | `crates/dao/src/migrations/` grep `knowledge_entities` | **无匹配**（建表已切声明式引擎） |
| 实测库 | `pragma_index_list('knowledge_entities')` | 只有 `sqlite_autoindex_…_1 (origin=pk)` ⇒ **无 `(kb_id,name)`** |

⇒ 插入重复**既不报错也不合并**，只是新增一行。
⚠ **证据强度分层**（勿过度外推）：前三层是**强证据**（它们正是新库 schema 的来源，两方言同源）；
第四层实测的是本机 `axagent.db`（schema 落后、该表 0 行）⇒ 只作**旁证**。

#### ③ 周期性 × 新 id ⇒ 无界增长

`services.rs:253`「步骤 3」位于 `knowledge_consolidation` 内，每轮取 `importance >= 0.7` 至多 100 条
⇒ 接线后**每轮 +100 行**（`name` = content 前 100 字，内容微改即变新 name ⇒ 无自然键可去重）。

#### ⇒ 接线的前置动作（不再是「待查」）

在 repo 层给出 `(kb_id, name)` 的去重口径。可选形态：给表加该唯一约束并**把 `on_conflict`
改打到它上**（注意 `on_conflict(Id)` 对别的唯一约束无效）；或先 `find` 再决定 insert/update。
⚠ 不得用「让断言变绿」的方式回避（§7）。

### 4.3 `importance` 阈值与条数

现为 `importance >= 0.7`、上限 100 条（`:255-259`）。接线后会真的开始写 ⇒ 该口径需确认。

---

## §5 建议实施顺序（**1~5 步均已执行**，记录见 §5b / §5c）

1. ✅ **4.2 的检查已做完（2026-09-17，只读）⇒ 结论：不幂等。** 因此下一步不是接线，而是
   **先在 repo 层给出 `(kb_id, name)` 去重口径**（见 §4.2「前置动作」），**然后才能**动下面的 2~5 步。
2. `harness::constants::sentinel` 增：`MEMORY_REFLOW_KB_ID` / `MEMORY_REFLOW_KB_NAME` / `MEMORY_REFLOW_KB_ENABLED: i32 = 0`
   （⚠ `enabled` 必须是 `i32`：PG 拒绝布尔字面量，历史事故见 `src-tauri/crates/harness/src/constants.rs:443-450`）。
3. `dao/src/seed.rs::ensure_sentinels` 增建该行（幂等、不分方言）。
4. `init/services.rs` 改用该常量；**删除** `"memory_default"` 字面量与 `namespace_id` 复用。
5. 可观测：把逐条失败留在 `debug!`，另在循环结束后按**失败计数**发**一条** `warn!`
   （逐条 `warn!` 会在最坏情况下变成 100 条/次 的噪声，那是把静默换成吵闹，不是修复）。

---

## §5b 实施记录（2026-09-17，按 §4.1 **选项 A**：单一哨兵 KB）

**改动 5 个文件 / 6 处**（`cargo fmt` 已过；**编译与测试尚未验证**，见 §5c）：

| # | 文件 | 改动 |
|---|---|---|
| 1 | `crates/harness/src/constants.rs` | `sentinel` mod 增 `MEMORY_REFLOW_KB_ID = "__sys_memory_reflow__"` / `MEMORY_REFLOW_KB_NAME` / `MEMORY_REFLOW_KB_ENABLED: i32 = 0`，并写明「为什么不能复用 namespace id」（两个 id 空间 + 外键） |
| 2 | `crates/dao/src/seed.rs` | `ensure_sentinels` 增建该 KB 行（照 `ensure_trajectory_knowledge_base` 形态）；**同步改测试**：删除段加删该行、`count_sentinels` 断言 **2→3**、两条测试各加对新行的断言 |
| 3 | `crates/dao/src/repo/knowledge_graph.rs` | `upsert_entity` 改为**按自然键 `(kb_id, name)` 先查后写**；新增 `MemoryReflowStats` / `reflow_memory_to_knowledge` / `REFLOW_IMPORTANCE_THRESHOLD` / `REFLOW_MAX_ITEMS`；新增 `memory_reflow_tests` |
| 4 | `src/init/services.rs` | 「步骤 3」整段改为调用 dao 的单一实现；KB 归属改走哨兵常量 |
| 5 | `src/commands/knowledge_graph.rs` | 同上（Tauri 命令）；默认阈值/上限改取 dao 常量；返回结构 `MemoryToKnowledgeResult` **形状不变**（前端契约不受影响） |

### 两个刻意的取舍（都写进了代码注释，不是省略）

1. **不加 DB 唯一约束**（§4.2 提的「更彻底做法」）。理由：① **存量数据风险** ——
   `UNIQUE(knowledge_base_id, name)` 要求存量无重复行，而生产是 PG、本机无法查证
   （SQLite 侧该表 0 行且 schema 落后，**不能代表 PG**）⇒ 盲加可能让引擎启动即失败；
   ② `crates/entities` 的 SeaORM 实体层没有复合唯一索引的写法。
   **代价**：应用层去重**不抗并发**（两个并发调用可能都 `find` 到 `None`）。
   当前两个调用点都是**串行 for 循环** ⇒ 不触发；
   **新增任何并发调用方之前必须先补唯一约束**（此约束已写在函数文档里）。
2. **阈值/上限保持 0.7 / 100**（§4.3 未裁决，不在实现里顺手改）；只是把这两个数
   从「两处手抄」收敛成 dao 的常量。

### 幂等性怎么被验证（§6 那条验收式的单元化）
`memory_reflow_tests::upsert_entity_is_idempotent_on_natural_key`：连调两次同一
`(kb_id, name)`，断言 `rows == 1`、`mention_count == 2`、`confidence` 被更新；
**并带反向对照**（不同 `name` 必须各占一行 —— 缺这条时，「去重」被误写成
「只留一行」也会通过）。

## §5c 验证状态（如实登记，**静态检查五层全绿 + 真 PG 端到端已 PASS**；过程中的作废/重跑见节末后记）

- ✅ `cargo fmt -p axagent-dao -p axagent-harness -p axagent` = exit 0；`--check` 复验 = exit 0。
  ⚠ 已核对：`cargo fmt` **只改动了本项清单内的 5 个文件**，未波及他人工作。
- ✅ `cargo clippy -p axagent-dao -p axagent-harness --all-targets -- -D warnings` =
  **`Finished dev profile [unoptimized + debuginfo] target(s) in 15m 49s` + `dao_clippy_exit=0`**
  （日志 `output/tmp-lead-dao-v2-clippy.log`，398 B）⇒ 含测试目标（`--all-targets`）在内**零诊断**。
- ✅ `cargo clippy -p axagent-harness --all-targets`（独立复验）= **`Finished … in 2m 18s` + `harness_clippy_exit=0`**
  （日志 `output/tmp-d-v3-harness-clippy.log`，235 B）。
- ✅ `cargo test -p axagent-dao --lib` = **`test result: ok. 303 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 31.88s`** + `dao_test_exit=0`
  （日志 `output/tmp-lead-dao-v2-test.log`，24633 B）。**本项 4 个关键测试全部在列且 `ok`**：
  `repo::knowledge_graph::memory_reflow_tests::upsert_entity_is_idempotent_on_natural_key` ·
  `…::reflow_sentinel_kb_exists` · `seed::tests::ensure_sentinels_restores_deleted_rows` ·
  `seed::tests::create_test_pool_yields_sentinels`
  ⇒ 幂等性（`rows == 1` / `mention_count == 2`）与哨兵「删后补回 3 行」**已有行为级证据**，
  不再是「只有设计推理」。
- 🔴 `cargo clippy -p axagent --all-targets -- -D warnings`（主 crate，含两处收敛）——
  **红了，但红在别人身上**：`app_clippy_exit=101`，唯一错误 =
  `error: this \`if\` statement can be collapsed` @ `crates/runtime-core/src/cron_job.rs:471`
  （`collapsible_if`，被 `-D warnings` 升级；日志 3915 B）。
  **该文件 ∉ 本项改动清单**，且其 mtime = `2026-09-17T06:14:57Z` —— **晚于**本项最后一次改动
  （`06:08:51`）、且正好落在我这次 clippy 开始检查它的前 2 秒 ⇒ 判定为**另一会话正在飞的写入**
  （`runtime-core/src` 内其余文件 mtime 均为 09-05～09-13，仅它今晨 06:14 被动过）。
  ⚠ **因此 `axagent` 主 crate 从未被检查到**：`error: could not compile axagent-runtime-core (lib)`
  ⇒ 依赖链在 runtime-core 断掉，本项两处收敛调用点**仍缺 clippy 证据**。
  ⇒ 按纪律**不代改**他人正在飞的文件（同文件并发 `Edit` 会互相覆盖），改走
  `cargo check -p axagent --all-targets` 补**编译级**验证 → **✅ 已补（见下）**。
- ✅ **后记（同日 16:05）：该 `collapsible_if` 由对方自行修掉，重跑后主 crate clippy 全绿。**
  `cron_job.rs` 的 mtime 第三次变化（`06:56:18Z`，28165 → 28153 B），形态已变为 clippy 建议的
  `if self.persist && let Err(e) = cron_job::Entity::delete_by_id(…).await { … }`
  （`edition 2024` 支持 let-chain）。⇒ 判定得以印证：**它确实是对方的在飞文件**，
  且**我方未代改是正确选择**（若我按旧快照写回，会把对方这处修复覆盖掉）。
  重跑结果：**`Finished dev profile [unoptimized + debuginfo] target(s) in 10m 36s`
  + `app_clippy_exit=0`，`error` 0 条 / `warning` 0 条**（日志 `output/tmp-d-v5-app-clippy.log`，417 B）。
  ⇒ ✅ **本项两处收敛调用点（`init/services.rs` / `commands/knowledge_graph.rs`）的 lint 级证据至此补齐**
  （`--all-targets` 含测试目标）。
- ✅ `cargo check -p axagent --all-targets` = **`Finished dev profile … in 26m 29s` + `app_check_exit=0`**
  （日志 `output/tmp-d-v4-app-check.log`，3402 B；task `e8Laqw`）
  ⇒ 本项收敛的**两处**调用点（`init/services.rs` / `commands/knowledge_graph.rs`）**类型正确、可编译，含测试目标**。
  ⚠ 反向印证：`collapsible_if` 确为 clippy lint 而非编译错误（runtime-core 本身编得过）。
  ⚠ 但这只是**编译级**证据，**不等于 clippy 级通过** —— 主 crate 的 lint 证据仍然缺。
  ⚠ 复核发现：`cron_job.rs` 的 mtime **随后又变了一次**（`06:14:57` → `06:32:32`，28083 → 28165 B），
  而 469–475 那段嵌套 `if` **形态未动** ⇒ 对方**仍在改该文件、且没修这条 lint**
  ⇒ 主 crate clippy 会被**持续**挡住，直到对方（或用户授权）处理它。
- ⚠ **当时未做**（**已被节末「后记 2」覆盖 = 已完成**）：真 PG 侧探针。§6 要求的两条（「全新 PG 库上哨兵行存在」、「连跑两轮不线性增长」）
  **仍未验收** —— 本机 PG 无可用凭证。
  ⇒ 本项现状 =「**代码已实施；fmt · dao+harness clippy · dao 303 测试 · 主 crate clippy
  （含 `--all-targets`）· 主 crate 编译 —— 五层全绿；真 PG 侧机制已实证（§5e）**」。
  ⇒ 当时只剩「真 PG 端到端探针」一项待读数（已排 `cargo run --example memory_reflow_probe`）；
  出读数前**不得记作「已完成」**。
- ✅ **后记 2（同日 16:40）：真 PG 端到端探针已完成** ⇒ 本节标题里那句「仅真 PG 端到端待探针」
  **已过期**。六项判据 + P5b 见 §5e 第 6/7 节。**本项在「代码 + 验收」两维均已闭环；
  唯一未闭环的是 §5d 同型穷举。**
- ⚠ **后记 3（同日 16:40）：上面那条 dao clippy 读数（`:196`）已作废。** 原因是探针必须接
  `tracing-subscriber` 才能观测日志 ⇒ `crates/dao/Cargo.toml` 的 `[dev-dependencies]` 变化
  ⇒ dao 的 **dev-target 依赖集变了**（连带 `sqlx-*` / `sea-orm` / `reqwest` / `h2` / `hyper` /
  `tower-*` / `tracing-core` / `tokio` / `regex-automata` 全链重编译，实测 2m 15s）。
  ⇒ 重跑命令 `cargo clippy -p axagent-dao --all-targets -- -D warnings` —— **已重跑，读数如下**：
  ✅ **`Finished dev profile [unoptimized + debuginfo] target(s) in 1m 38s` + `dao_clippy2_exit=0`，
  `error` 0 条 / `warning` 0 条**（日志 `output/tmp-d-v5-dao-clippy2.log`，1131 B）
  ⇒ 含 `--all-targets`（lib + tests + **本轮新增的 `examples/memory_reflow_probe.rs`**）在内零诊断。
  ⚠ 注意：这一轮的 `Checking` 列表里**没有** `axagent` 主 crate —— 这正符合预期（主 crate 不是 dao 的 target），
  故主 crate 那条读数（`:219` 后记）**不需要**重跑。
  主 crate `axagent` 的 clippy 读数 **不受影响**（它只依赖 dao 的 normal deps，dev-deps 不进该依赖集）。
- ✅ **后记 4（同日 16:17）：同理，dao 的 `303 passed` 读数也在同一变更之前取得 ⇒ 一并重跑。**
  `cargo test -p axagent-dao --lib` =
  **`test result: ok. 303 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 24.53s`**
  + `dao_test2_exit=0`（日志 `output/tmp-d-v5-dao-test2.log`，24406 B，task `GrZ2So`）。
  本项 4 条关键测试**全部 `ok`**：`repo::knowledge_graph::memory_reflow_tests::upsert_entity_is_idempotent_on_natural_key` ·
  `…::reflow_sentinel_kb_exists` · `seed::tests::ensure_sentinels_restores_deleted_rows` ·
  `seed::tests::create_test_pool_yields_sentinels`。
  ⇒ **口径一致性**：既然以「dev-target 依赖集变了」为由作废 clippy 读数，就**不能对同一变更、同 target 类**的测试读数网开一面 —— 两层都重跑过，五层证据才**同源**。

### §5d 同型穷举（判据 #256）—— ✅ **已完成**（2026-09-17 16:45）

**方法（刻意不用窗口式启发式）**：全仓扫 `.on_conflict` **调用点**（排除 `target/`、排除注释行）得
**51 处 / 29 文件**；逐点**回溯到最近的 `fn` 签名**并取整段函数，按「冲突键**从哪来**」分四类。
凡「键来自结构体字段」的，再按判据 **#10 走两跳**追到该字段的**赋值点**；
并在 **PG 生产库（`axinvest`）** 上做**反事实读数**——不只看码，还看库里真实发生了什么。

| 类 | 含义 | 处数 |
|---|---|---|
| **A** | 冲突键 = 稳定业务键 / 入参 id / 自然键 ⇒ `on_conflict` **真会触发** | 31 |
| **B** | `do_nothing` + **确定性主键**（幂等靠主键本身，不靠 `on_conflict`） | 10 |
| **C** | 🔴 冲突键 = **同一次新生成的随机 id** ⇒ `on_conflict` **永不触发** | **3** |
| **D** | ⚠ 同型（C 型写法）但当前调用路径有**外部兜底**或**从未发作** | 7 |
| | **合计** | **51** |

#### 类 A（31 处）—— 冲突键一览（全部清白）

| 调用点 | 冲突键 | 键的来源 |
|---|---|---|
| `disk-cache/lib.rs:256` | `QueryHash` | 内容哈希 |
| `disk-cache/lib.rs:323` | `SnapshotId` | 入参 |
| `dao/repo/agent_profile.rs:244` | `Id` | 入参 `id: &str` |
| `dao/repo/agent_role.rs:159` | `Id` | 入参 `id: &str` |
| `dao/repo/capability_policy.rs:76` | `Id` | 入参 `id: &str` |
| `dao/repo/capability_domain_override.rs:94` | `Domain` | 自然键 |
| `dao/repo/capability_relationship.rs:95` | `(SourceId,TargetId,RelationshipType)` | 自然键 |
| `src-tauri/crates/dao/src/repo/capability_stats.rs:79` | `CapabilityId` | 自然键 |
| `src-tauri/crates/dao/src/repo/evolution_execution_stats.rs:81` | `(ConversationId,ToolId)` | 自然键（且带 `Expr::col(..).add(1)` 累加，是**真 upsert**） |
| `dao/repo/loop_checkpoint.rs:46` | `(ExecutionId,NodeId)` | 自然键 |
| `dao/repo/program_policy.rs:51` | `ProgramName` | 自然键 |
| `dao/repo/session_state.rs:79` | `StateKey` | 自然键 |
| `src-tauri/crates/dao/src/repo/settings.rs:240` | `Key` | 自然键 |
| `dao/repo/sync_device.rs:91` | `Id` | `device.device_id`（设备自带身份） |
| `src-tauri/crates/dao/src/repo/sync_permission.rs:92` | `DeviceId` | 自然键（PK 用 `Uuid::new_v4()` 但**冲突键不是它**） |
| `src-tauri/crates/dao/src/repo/sync_policy.rs:114` | `Id` | `policy.id` |
| `src-tauri/crates/dao/src/repo/wiki_graph_cache.rs:127` | `VaultId` | 自然键 |
| `dao/repo/workflow_template.rs:181` | `Id` | 三个调用方都传**模板自身 id**（`workflow_template_repository.rs:90 template.id`、`cognitive_router_init` / `workflow_reflection` 同源） |
| `analysis-engine/src/opc/capability_pack.rs:39` | `Id` | `DomainPackManifest.id`（且前一行已 `find_by_id(&m.id)` 保 enabled，是标准 upsert） |
| `trajectory/src/storage.rs:1347` | `Key` | 自然键（PK 用 `pref.id` 但冲突键不是它） |
| `src-tauri/crates/search/src/file_index.rs:204` / `:299` | `Path` | 自然键 |
| `search/ast_index.rs:269` | `(CallerFile,CallerFunction,CalleeName,Line)` | 复合自然键 |
| `semantic_cache/mod.rs:264` | `Id` | `hash`（内容哈希，`id` 与 `prompt_hash` 同值） |
| `src-tauri/crates/runtime-core/src/cron_job.rs:440` | `Id` | 入参 |
| `commands/agency_expert.rs:452` | `Id` | `agency-{dir_name}-{file_stem}` |
| `commands/opc_setup/mod.rs:244` | `Id` | `opc-{id}`（常量表驱动） |
| `commands/opc_setup/domain_agents.rs:54` | `Id` | `opc-{id}` |
| `commands/opc_setup/capability_pack_agents.rs:86` | `Id` | `opc-{id}` |
| `commands/opc_workflows/mod.rs:598` | `Id` | `data.id` |
| `commands/stock_pipeline/seed_stock_pipeline.rs:615` | `Id` | `data.id` |

#### 类 B（10 处）—— `do_nothing` + 确定性主键（清白）

- `search/ast_index.rs:185/208/227/249`（4 处）：**同一事务内先 `delete_many().filter(FilePath.eq(&fp))`**
  再 `insert_many` ⇒ 幂等**由那次删除保证**，`do_nothing` 只是防御。⚠ 这 4 处若单看
  `insert_many + on_conflict(Id).do_nothing()` 会误判成 C 型 —— **必须先看同事务的前置删除**（假阳性来源）。
- `dao/repo/workflow_template.rs:262`：主键 `{t.id}_v{t.version}` 是**确定性**的，
  `do_nothing` 正是「保留既有历史快照」的语义（`:236` 注释写明：裸插会让保存永久失败）。
- `dao/src/seed.rs:76/97/134`（3 处）：哨兵 id 为常量（`TRAJECTORY_KB_ID` / `TRAJECTORY_MEM_NS_ID` /
  `MEMORY_REFLOW_KB_ID`）。
- `dao/examples/memory_reflow_probe.rs:165/169`（2 处）：本项探针自造，id 为常量命名空间 + `{prefix}{i}`。

#### 🔴 类 C（3 处）——**确证缺陷（全部为**码级**确证）**

| # | 位置 | 冲突键 | 键怎么来 | 生产库足迹 |
|---|---|---|---|---|
| 1 | `dao/repo/knowledge_graph.rs:787`<br>`upsert_relation` | `knowledge_relations::Column::Id` | `:768 let id = format!("rel_{}", gen_id())` | 🔘 **零足迹** —— 见下「判别式」。与既有登记「零调用」一致 |
| 2 | `trajectory/src/storage.rs:517`<br>`save_pattern` | `trajectory_patterns::Column::Id` | `harness/trajectory_types.rs:227` `Uuid::new_v4()`（每次 `TrajectoryPattern::new` 都新生成） | 🔴 **2038 行，却只有 3 个不同 `name`**（`tool-CapabilityView`×1019、`multi-step-13`×525、`multi-step-10`×494）；**今天仍在增**<br>✅ **2026-09-17 已修 —— 见 §5d-补** |
| 3 | `trajectory/src/storage.rs:991`<br>`save_relationship` | `knowledge_relations::Column::Id` | `commands/memory.rs:1319` `format!("rel_{}", &uuid[..12])` | 🔘 **零足迹**（库里没有 `rel_{12位十六进制}` 形态的行） |

**三处的证成方式不同，不能混为一谈**：

- **#2 是「仍在发生的无界增长」**（最硬，且**写者唯一**）：
  `trajectory_patterns::Entity::insert` **全仓只有 `storage.rs:496` 一处**（就在 `save_pattern` 内）
  ⇒ 2038 行的归属**没有第二种可能**。
  首行 `2026-09-06T08:49Z`、末行 **`2026-09-17T05:13Z`（今天）**，日均 50–300 行；而逻辑上只有 3 个 pattern。
  调用方是**周期性任务**：`init/services.rs:838`（每 **15 分钟**）与 `:886`（每 **30 分钟**），
  各自对至多 20 / 50 条 trajectory 各产出最多 4 个 pattern。
  ⚠ 根因不止「id 新生成」——`pattern.rs:138` 的查表键是 `tool_seq_{序列}`，而 `:118` 写入
  `learned_patterns` 用的键是 `pattern.name`（`tool-{首个工具}`）⇒ **同一个 `HashMap` 装了两套键名空间，
  查表恒 miss** ⇒ 连「内存内去重」这层也是死的（这是**独立于 `on_conflict` 的第二个缺陷**）。
- **#3 是码级确证**：`commands/memory.rs` 的实体循环**有**按 `(name, entity_type)` 的查重（`:1267`），
  但**关系循环没有任何查重**（`:1303-1334` 直接为每条抽出的关系造一个新 uuid 就 `save_relationship`）
  ⇒ 同一对话重复提取一次即多一批重复关系行。**当前零足迹只是因为该命令还没被跑过。**
- **#1 是「已登记未修」**：`PLAN-declarative-schema-sync.md:2917` 已登记（该处判为「零调用」）；
  旁证 `trajectory/src/causal.rs:14,269` 与 `trajectory/tests/causal_integration.rs:80`（后者是条回归测试）。
  本项按 §7 **不顺手改**。

##### ⚠ 本轮我自己的一次**误归属**，以及纠正它用的「判别式」

初稿里我把「`rel_` 前缀 788 行 / 767 键 / 20 个重复组」写成了 **#1 的生产证据**。
**这是错的**，并且与我同节刚写下的「避免误归属」自相矛盾。自查后钉住写者：

- 全仓有**两处**都会产出 `rel_{uuid}` 形态的主键（`gen_id()` = uuid v4）：
  - `knowledge_graph.rs:768`（`upsert_relation`，**无**存在性检查 ← 缺陷所在）
  - `knowledge_graph.rs:1144`（批量导入，`:1131-1143` **有**按 `(kb, source, target, type)` 的存在性检查）
  ⇒ 只看主键形态**分不出写者**。
- **判别式**：两处有一列取值**互斥** —— `upsert_relation` 在 `:772` 写死
  `knowledge_base_id: Set(String::new())`（空串），而 `:1144` 写真实 `kb_id`。
  实测 `SELECT count(*) FROM knowledge_relations WHERE knowledge_base_id = ''` = **0**
  ⇒ **`upsert_relation` 从未写入过任何行**，788 行的写者只能是 `:1144`。
- ⇒ 结论：#1 是**零足迹的码级缺陷**（接线即发作，但不是「已发作」）；
  788 行 / 20 组重复属 `:1144` 那条路径，**另立登记**（见下）。

⇒ **纪律**：同一形态主键有多个写者时，**必须找一个「只有一方会写」的列作为判别式**再归属；
「形态一致」不是归属证据（判据 **#370** 的同族，已固化为 **#475**）。

**两张表都只有 `pkey(id)`，没有任何业务唯一约束**（`pg_indexes` 实测：`trajectory_patterns` 仅
`pkey` + `pattern_type` 普通索引；`knowledge_relations` 仅 `pkey` + 4 条普通索引）
⇒ 与 §4.2 ② 的结论一致：**`on_conflict` 是唯一防线，它在 C 型写法下失效时没有任何东西兜底**。

#### ⚠ 类 D（7 处）—— 同型写法，但当前路径有兜底 / 从未发作（**不是清白，是「靠调用方兜着」**）

| 位置 | 为何未发作 | 风险 |
|---|---|---|
| `dao/repo/knowledge_graph.rs:710 upsert_entity` | **本项已修**：`:646` 先按自然键 `(kb_id,name)` 查，命中即 UPDATE（`mention_count+1`）；`:706` 注释明确「`on_conflict(Id)` 永不触发，保留只为防御 `gen_id()` 撞值」 | 已消除；⚠ 仍不抗并发（`:623` 已登记） |
| `trajectory/src/storage.rs:868 save_entity` | 调用方 `commands/memory.rs:1267` **有**按 `(name, entity_type)` 的查重后才 save | **换调用方即复发** |
| `trajectory/src/storage.rs:166 save_trajectory` | `Trajectory::new` 生成一次 id，存一次 | 同上 |
| `trajectory/src/storage.rs:688 save_skill` | 多数调用方传 `&updated`（保留 id）；`trajectory_skills` 生产 **0 行** ⇒ 无实态证据 | 同上 |
| `trajectory/src/storage.rs:1047 save_session` | 同上 | 同上 |
| `trajectory/src/storage.rs:1245 save_memory` | `memory_providers/service.rs:341` 造 `mem_{now}_{uuid8}`，一条记忆存一次 | 同上 |
| `trajectory/src/storage.rs:1288 save_learning_pattern` | 同上 | 同上 |

⇒ 这一栏的共同形态是：**`upsert_*` 这个名字在骗人**——它实际是「带一条永不命中的
`on_conflict` 的 INSERT」，幂等性**全部外挂在调用方**。判据 **#472** 正是这条。

#### 🆕 附带发现：知识图谱重复的**大头不是本类缺陷**（**避免误归属**，判据 #370）

`knowledge_relations` 全表 **112937 行 / 63191 逻辑键 / 37397 个重复组**，
`knowledge_entities` **72816 行 / 45215 键** —— 数字很大，但**归属不是 `upsert_relation`**：

- 按 `id` 前缀拆分：`other` 49991 · `lemonhu_…` 37383 · `x_employ…` 24775 · **`rel_` 仅 788**。
- 抽一个「重复组」看真实 id，同一 `(source,target,has_concept)` 的 6 行 id 是
  `600631_has_concept_…` / `f9b2b050-…_600631_has_concept_…` / `lemonhu_knowledge_graph_600631_has_concept_…`
  （及 600827 的同类）⇒ **同一批关系在三套 id 命名空间下各存了一份**
  （`lemonhu` CSV 导入在不同时期用了不同的 `knowledge_base_id` + id 前缀约定）。
- ⇒ 这是**导入侧 id 命名空间漂移**，属另一类缺陷（数据/迁移轨），**不得记到本项头上**。

**另有一项要单独登记的**：`rel_` 那 **788 行 / 20 个重复组**（写者 = `knowledge_graph.rs:1144`，
即那条**带存在性检查**的批量导入路径）。它做了 `(kb, source, target, type)` 存在性检查却仍产生重复
⇒ 说明该检查**有漏**（例如 `continue` 之后仍插入、或检查用的 kb/目标列与实际写入不一致），
**这是第三个独立缺陷，且它有生产足迹**。归属证据：全 788 行 `knowledge_base_id` 均为
`f9b2b050-e336-4de8-b305-0b40cbf5740f`（真实 kb），而 `:1144` 正是写真实 `kb_id` 的那一处。
⚠ **本项不动它**（需产品语义决策 + 存量 20 组重复行的清理策略），**只登记**。

#### §5d 出口自查

- 51 / 51 逐点核对完毕，**没有一条停留在「嫌疑」**：每点都落到 A/B/C/D 之一，且 C 型全部给出**具体赋值行**。
- C 型 3 处**全部有生产库反事实或码级调用链**支撑，不是「读码推机制」。
- 明确区分了「清白」（A/B）与「靠调用方兜着」（D）—— D **不算清白**，已在表内标出复发条件。
- 明确拒绝了一次**误归属**（把 CSV 导入的 37397 组重复算成本类缺陷）。
- 仍**未处理**（本项不动）—— 4 项**已全部登记**于 `PLAN-declarative-schema-sync.md`
  的「必须登记、本轮不擅改」表（`:2917` 之后新增 4 行）：C-#1（`upsert_relation` 幂等性为零，
  **零足迹**）· **C-#2（`save_pattern`，唯一有生产实态且在无界增长 —— 优先级最高）** ·
  C-#3（`save_relationship` + `memory.rs` 关系循环无查重）· `:1144` 批量导入「**有检查却仍重复**」。
- ⚠ **本轮我自己发生过一次误归属，并在写完后自查纠正**：初稿把生产库「`rel_` 788 行 / 20 个重复组」
  当成了 C-#1 的证据，与同一节刚写下的「避免误归属」自相矛盾。纠正手段是找**判别列**：
  `upsert_relation` 写死 `knowledge_base_id = ""` 而全表空串行 = **0** ⇒ 它**零足迹**，
  788 行只能属于 `:1144`。⇒ 已固化为判据 **#475**（附带发现里保留了完整过程）。

#### §5d-补 · C-#2 已修复（2026-09-17，用户授权「要」）

**修法取舍（两条路，均已查实）**

| 方案 | 内容 | 判定 |
|---|---|---|
| **(a) 让主键成为自然键的编码** | `TrajectoryPattern::new` 不再 `Uuid::new_v4()`，改为 `stable_id_for_name(name) = "pat_{name}"` ⇒ `ON CONFLICT (Id)` **真的会命中** | ✅ **本轮采用**：不动 schema、不清存量、不碰第二个写者 |
| (b) 在 repo 层按 `name` upsert（或加 `UNIQUE(name)` + `on_conflict(Name)`） | 在 `save_pattern` 里把身份改判为 `name` | ❌ **不可用**，见下 |

**为什么 (b) 不可用 —— 本轮的前置勘察（险些踩中）**

`trajectory_patterns` 有**两个语义不同**的写者：

- 学习器（`pattern.rs` 四条产出路径 → `init/services.rs:838/886`、`commands/agent/mod.rs:2778`）：
  **身份 = `name`**（去重与聚合都按它做）；
- `commands/rl_training.rs:279 save_checkpoint`：**身份 = 调用方给的 `id`** ——
  `load_checkpoint:326` 按 `p.id == checkpoint_id` 命中、`list_checkpoints:359` 按 `ckpt.id` 去重，
  `name` 只是 `rl_checkpoint:{名字}`。

⇒ 在 repo 层按 `name` upsert，会把**同名不同 id 的检查点静默压成一行**（历史检查点从列表与「按 id 加载」里同时消失）。
⇒ 正确位置是「**让产出方的主键去编码它的自然键**」，而不是把某一个写者的语义提升成全表语义。

**改动清单（3 文件 / 5 处）**

| # | 位置 | 改动 |
|---|---|---|
| 1 | `harness/src/trajectory_types.rs:225-259` | 新增 `stable_id_for_name`；`new()` 改用它（附根因注释 + 「需同名多行请自带 `id`」的警示） |
| 2 | `trajectory/src/pattern.rs:139-147` | `extract_tool_sequence` 的查表键 `tool_seq_{seq}` → `pattern.name`（与写入键对齐） |
| 3 | `trajectory/src/pattern.rs:116-123` | 统一 `frequency = trajectory_ids.len()`（四条产出路径共用） |
| 4 | `trajectory/src/pattern.rs:466-471` | `extract_cross_session_patterns` 的自造主键同样派生（同缺陷家族的**第 4 条**路径） |
| 5 | `trajectory/src/storage.rs:518-527` | upsert 更新列补全 `Description / TrajectoryIds / AverageValueScore / RewardProfile` |

**验证（真读数，不是「编译过了」）**

| 门禁 | 命令 | 读数 |
|---|---|---|
| 格式 | `cargo fmt -p axagent-harness -p axagent-trajectory -- --check` | **exit 0**（首轮报我新写的 2 行折行，修后复检日志 0 B） |
| lint | `cargo clippy -p axagent-harness -p axagent-trajectory --all-targets --all-features -- -D warnings` | **exit 0 / 0 error / 0 warning**（`4m 25s`；`Checking` 清单**确含** `axagent-harness` 与 `axagent-trajectory` ⇒ 改动真被检到，不是缓存假绿） |
| 单测 trajectory | `cargo test -p axagent-trajectory --lib` | **425 passed; 0 failed** |
| 单测 harness 契约 | `cargo test -p axagent-harness --tests` | `serialization_contracts` **42 passed; 0 failed** |

新增 4 条回归测试（全部 `ok`）：

| 测试 | 锁什么 |
|---|---|
| `pattern::tests::tool_sequence_pattern_reuses_natural_key_across_runs` | 同一自然键两次学习 ⇒ **同一主键** + frequency 1→2（修前恒 1） |
| `pattern::tests::stable_id_is_derived_from_name_only` | 同名同主键 / 异名异主键（不许把不同模式并成一行） |
| `storage::tests::save_pattern_is_idempotent_on_natural_key` | 连存两次 ⇒ **1 行**，且聚合列同步更新（frequency/success_rate/trajectory_ids） |
| `storage::tests::save_pattern_keeps_distinct_ids_with_same_name_apart` | **反向守卫**：同名异 id 仍是 2 行 ⇒ 证明没把检查点身份压掉 |

**本项没做（如实登记，不许读成「已去重」）**

- **存量 2038 行未清理**（仅 3 个 `name`）⇒ 修法只让**增长停止**，**不等于已去重**。
- **未加 `UNIQUE(name)`**：需先清存量 + 写 PG/SQLite 两方言迁移（属另一类决策，需另授权）。
- **重启后聚合从 0 起算**：`learned_patterns` 是进程内状态，重启后为空 ⇒ 第一次 upsert 会把 DB 行
  覆盖成进程内单次值。**行数不再增长**（同一 `name` 永远命中同一行），但**跨重启的累计值不保**。
  根治依赖下面这条「暖启动空转」。

**🆕 新发现（同文件、不同根因，已登记）**：暖启动**空转** —— `src/lib.rs:472-509` 想把持久化
patterns 回灌 `PatternLearner`，但合成的 `Trajectory` 把 `steps` 置空（`:500`），而四条抽取路径
**都以 `steps` 为前置条件** ⇒ `learn_from_trajectory` 恒返回空 vec ⇒ 重启后 `learned_patterns`
恒为空；而 `:507` 仍打印 `Loaded {pattern_count} persisted patterns`，其中 `pattern_count` 取自
`persisted.len()`（**输入规模**，不是实际装载量）⇒ **日志与效果相反**。判据已固化为 **#477**。

**⚠ 另一处红与本项无关（归属已钉死，勿误记到本项头上）**：`trajectory/tests/causal_integration.rs`
**4 passed / 17 failed**，全部同源于 `code 787 FOREIGN KEY constraint failed` —— 真因是
`trajectory/src/causal.rs:302` 的 `observe_edge` **写死 `knowledge_base_id = ""`**，而声明式引擎
给 SQLite 新库建了 `knowledge_relations → knowledge_bases` 外键（FK 由 sqlx 默认 PRAGMA 打开，
见 `PLAN-declarative-schema-sync.md:1279`）⇒ 无父行即 787。**与本项零交集**：本项只写
`trajectory_patterns`，失败断言读的是 `knowledge_relations`；且未走边的用例（4 条）全绿。
⇒ 属「写死空串 kb id」家族（与 `services.rs:271-275`、`upsert_relation:772` 同型），**另立登记**。

#### §5d-补2 · C-#1 / C-#3 / causal 787 已修复（2026-09-17，用户授权「要」）

**授权范围**：用户对上一轮给出的三选项答「要」⇒ 执行 **(a) 修 C-#1 / C-#3** + **(b) 修 causal 787**。
**(c)（清 `trajectory_patterns` 存量 2038 行 + 加 `UNIQUE(name)`）涉及删除生产数据 + schema 迁移，本轮未做，另请明确授权。**

##### 1. (b) causal 787 —— 先修，用它当后面两项的验证网

| 项 | 内容 |
|---|---|
| 文件 | `trajectory/src/causal.rs` |
| 改动 | 新增 `const CAUSAL_KB_ID`（转发 `harness::constants::sentinel::TRAJECTORY_KB_ID`，值不重复）；`observe_edge` 的 `Set(String::new())` → `Set(CAUSAL_KB_ID.to_string())` |
| 修前读数 | `causal_integration.rs` **4 passed / 17 failed**（全部同源 `code 787`） |
| 修后读数 | **22 passed / 0 failed**（21 原有 + 1 本轮新增） |
| 自证 | 改**一处** `String::new()` ⇒ 17 条红全绿 ⇒ 787 根因判定成立（不是「把红改绿」） |

**为何这不是「测试环境副作用」**（本轮比 §5d-补 时更硬的证据）：`knowledge_relations.knowledge_base_id`
上有指向 `knowledge_bases(id)` 的外键，**而读端早已按哨兵过滤** ——

- `src/commands/proactive.rs:328` / `:568`：`list_causal_edge_stats(state…db(), TRAJECTORY_KB_ID)`
- `trajectory/src/storage.rs:1016`：`get_all_relationships` 也 `.filter(KnowledgeBaseId.eq(Self::TRAJECTORY_KB_ID))`

⇒ 写端写 `""`、读端按哨兵查 ⇒ **即便关掉外键让 `""` 写成功，这些读端也一条读不到**。
本项修的是**写读口径冲突**，不是把测试弄绿。

**新增守卫测试**：`observe_edge_writes_sentinel_kb_visible_to_scoped_readers` —— 断言写端列值 = 哨兵
**且**读端 `list_causal_edge_stats` 能读到；另含反向守卫（用空串 kb 去问必须读不到），
防止将来把两端都改成空串来「修绿」。

##### 2. (a) C-#1 —— `dao::repo::knowledge_graph::upsert_relation`

**先决勘察（决定修法；比 §5d-补 的取舍表更具体）**：

| 勘察项 | 读数 |
|---|---|
| 调用方 | **全 workspace 零调用**（仅定义 + 注释。`capability_relationship.rs` 那个同名函数是同名**不同**表、不同签名的另一个实现，不是它） |
| 同型既有修法 | `upsert_entity`（同文件 `:600-625`，2026-09-17 已修）用的是 **read-then-write**，不是确定性主键 |
| 为何**不**照抄 read-then-write | 该表有**四个写者**，其中 `causal::observe_edge` 已占用 read-then-write；在 repo 层按自然键 upsert 会与它撞 |
| 为何确定性主键对**本**函数可行 | 零调用 ⇒ 表里**没有**「由它写入的存量行」需要被 find 回来 ⇒ 无需数据迁移。反之 `upsert_entity` **有**存量随机 id 行 ⇒ 它只能走 read-then-write（确定性主键会找不到旧行、**反而插重**） |

**改动**：

| 文件 | 改动 |
|---|---|
| `harness/src/knowledge_graph.rs` | 新增 `pub fn stable_relation_id(kb, source, target, rtype)` —— **唯一实现处**（+2 单测） |
| `dao/src/repo/knowledge_graph.rs` | `format!("rel_{}", gen_id())` → `stable_relation_id(...)`；`knowledge_base_id` 空串 → **哨兵**；删已不用的 `gen_id` import；doc 写明边界 |

**为何 `knowledge_base_id` 空串也在本项修（上一轮曾判为「独立缺陷、只登记」）**：该函数 doc 自述
`trajectory-style save`，而该用途下**全部**同族写者与读者都用哨兵（`src-tauri/crates/trajectory/src/storage.rs:963` 写、`:1016` 读、
`causal` 读、`proactive` 读）⇒ 空串不是「另一种产品语义」，就是同型缺陷。改后它一旦被接线**不会立刻 787**。

**明确定界（已写进 doc）**：确定性主键只保证**本函数自己**的幂等；表上**仍无**
`(kb, source, target, rtype)` 唯一约束 ⇒ 与别写者（`rel_causal_{uuid}` 等）**并存两行**的风险仍在。
`relation_type = "causes"` 归 `causal::observe_edge`，不走本函数。

##### 3. (a) C-#3 —— 缺陷**不在** storage，在调用方

**修法判定被现场推翻过一次，记下来**：`storage.rs::save_relationship` 的 `on_conflict(Id)` 有
**两个**调用方，语义**不同**：

| 调用方 | 传的 id | `on_conflict(Id)` 行为 | 判定 |
|---|---|---|---|
| `memory_providers/service.rs:1311-1323`（实体合并重定向） | `rel.clone()` 的**原行 id** | **会命中** ⇒ 就地改写该行 | ✅ 合法用法 |
| `src/commands/memory.rs:1319`（LLM 关系回流） | 每次新造 `rel_{uuid12}` | **永不命中** ⇒ 每次回流插一行 | ❌ 缺陷在这侧 |

⇒ **改的是 `memory.rs`，不是 `storage.rs`**（改后者会破坏实体合并的「就地改写」语义）。
`storage.rs` 本轮只补 doc，把 **id 契约**写死（两种合法用法 + 禁止随机 id + 后果）。

**🆕 本轮第二个高价值发现（已固化为判据 #478）**：自然键必须取**实际落库的那个值**。
`RelationshipType::from` 会把未登记值**折叠**成 `RelatedTo`（`Display` = `related_to`）
⇒ 若用**原始输入** `"Causes"` 派生 id、却把 `related_to` 写进列 ⇒ 「两个键、同一个列值」⇒
去重**照样失效**。修法：先归一，再用 `rel_type.to_string()` 参与派生。

##### 4. 本项没做（勿误读为已完成）

| 未做 | 原因 |
|---|---|
| **(c)** 清 `trajectory_patterns` 存量 2038 行 + 加 `UNIQUE(name)` | 涉及**删除生产数据** + schema 迁移 ⇒ 需单独明确授权 |
| **不追溯存量** | `memory.rs` 的 id 形态改变后，此前回流产出的随机 id 行**找不到** ⇒ 每个逻辑关系最多并存「旧随机行 1 + 新派生行 1」。彻底收敛需清存量 |
| **不给 `causal::observe_edge` 加确定性主键** | 会与 `upsert_relation` 撞 PK；且其 read-then-write 的前提是「表无该维度唯一索引」，本轮不扩大改动面 |
| **`knowledge_relations` 的跨写者去重** | 缺 DB 唯一约束（同属 (c) 类决策） |
| ~~反向实证（变异测试）~~ | ✅ **2026-09-17 已补做** —— 见 §5d-补3 第 1 节（两条变异**实测必红**；且顺带暴露了测试自身的诊断缺陷，见第 2 节） |

##### 5. 门禁真读数（2026-09-17）

| 门禁 | 读数 |
|---|---|
| `cargo fmt -p axagent -p axagent-harness -p axagent-dao -p axagent-trajectory -- --check` | 本轮 5 个文件 **0 diff**；日志只剩 `crates/dao/examples/memory_reflow_probe.rs`（10 处，mtime **08:08:41Z** ⇒ 早于本轮 09:01–09:03 的改动，**非本轮引入**）⇒ ✅ **已修**，见 §5d-补3 第 3 节 |
| `cargo test -p axagent-harness -p axagent-dao -p axagent-trajectory --no-fail-fast` | trajectory lib **425 passed / 0 failed**；`causal_integration` **22 passed / 0 failed**；dao lib **305 passed / 0 failed**（含本轮新增 2 条） |
| harness lib | **705 passed / 1 failed** —— 唯一红是 `test_csv_import_relations_are_declared`（**既有红**，见下）⇒ ✅ **已修**，见 §5d-补3 第 3 节 |

**harness 那 1 条红的归属（对象级非重叠）**：panic 正文要求核对 `let rtype = fields[2]` 的行号，
该行在 **`src/commands/knowledge.rs:1751`** —— 而该文件 mtime = 09-15，**本轮从未改动**；
且 `output/verify-2026-09-15c/test-harness.log:697` 记录**同样的红在 09-15 就已存在**
（当时真值 1749，现已漂到 1751）。⇒ 属**跨文件行号引用腐烂**（`DATA_DRIVEN_COLUMN.evidence` 现值 1748）。
**一行修法**：把 `DATA_DRIVEN_COLUMN.evidence` 的行号改成 1751 ⇒ ✅ **已改**（见 §5d-补3 第 3 节，含 `:1751` 确为 `let rtype = fields[2]` 的核对）。

#### §5d-补3 · 变异实证 + 两处既有红已修（2026-09-17 续，用户授权「继续未作的内容」）

##### 1. 变异实证（mutation testing）—— 把「逻辑上必红」变成「实测必红」

上一轮我在 §5d-补2 第 4 节登记的 caveat（「4 条新测试**未做**变异实证 ⇒ 不接受把『逻辑上必红』说成『已验证必红』」）
本轮**已补做**。

**纪律**：变异前 `cp` 字节级备份 3 文件 → `output/backup-2026-09-17-mutation/`
（`dao-knowledge_graph.rs` **75393 B** · `causal.rs` **29766 B** · `causal_integration.rs` **22512 B**）；
还原后**逐项断言**（`restored bytes=` 一致 / 新代码回来了 / 变异标记消失）+ **重跑确认恢复绿**。

| # | 变异（把生产代码**改回缺陷形态**） | 实测结果 |
|---|---|---|
| 1 | `dao/src/repo/knowledge_graph.rs::upsert_relation` 主键改回 `format!("rel_{}", axagent_harness::util_fns::gen_id())` | ✅ **FAILED**（`output/mutation1-dao.log`）：`upsert_relation_is_idempotent_on_natural_key` 红；同批 `upsert_relation_writes_sentinel_kb` **仍 ok** ⇒ 变异只打中一个面（区分力合格） |
| 2 | `trajectory/src/causal.rs::observe_edge` 的 kb 改回 `Set(String::new())` | ✅ **FAILED**（`output/mutation2-causal.log`）：报 `Query Error: error returned from database: (code: 787) FOREIGN KEY constraint failed` —— 错误码**直指根因**；同时 `warning: constant CAUSAL_KB_ID is never used`（符合预期） |

还原后：变异 1 ⇒ `2 passed; 0 failed`（`303 filtered out`，`output/mutation1-restore.log`）；变异 2 ⇒ 语义断言全过。
⇒ 结论：这两处修复**不是**「逻辑上应该能抓住」，而是**实测能被抓住**。

##### 2. ⭐ 变异实证顺带暴露了**我自己测试的诊断缺陷**（已固化为判据 #480）

变异 1 的 panic **没有**红在「行数」断言上，而是红在 weight 断言上：

```
panicked at crates\dao\src\repo\src-tauri/crates/dao/src/repo/knowledge_graph.rs:1736:9:
二次 upsert 应更新 weight（实测 0.5）
```

**根因** = 断言顺序写成了「先 `row_of(...)`（内部 `.one()`）→ 再 `count`」，
而 sea-orm 的 **`.one()` 在多行时静默取第一行** ⇒ 真根因「表里其实有 2 行」被掩盖成「weight 没更新」，
**失败信息把排查方向指反了**。

**修法**：把 `count` 断言**提前**到 `row_of` 之前，并在测试里留注释解释「为什么顺序不能换」。
⇒ 立为判据 **#480**：凡断言行级唯一性的测试，`count` 必须排在「读单行」之前。

> 这条的价值不在 dao，而在**变异实证会测出测试自身的观测缺陷**：若不做变异，
> 将来 `on_conflict` 真的失效时，CI 给出的线索会**指向 weight 而不是行数**。

##### 3. 两处既有红已修 —— ⚠ **含一次对我自己前面结论的更正**

| 红 | 归属取证 | 修法 | 结果 |
|---|---|---|---|
| fmt：`crates/dao/examples/memory_reflow_probe.rs` **10 处** | mtime **`2026-09-17T08:08:41Z`**，早于本轮 09:01–09:03 的改动 **53 分钟** | `cargo fmt -p axagent-dao`（**限包**：只格式化该包文件，不碰其它包） | 取证：改动后只有该文件 mtime 变 `09:34:04`；`fmt --check` **EXIT=0**（上一轮为 1） |
| test：harness `knowledge_graph::type_vocabulary_tests::test_csv_import_relations_are_declared` | 见下「更正」 | `harness/src/knowledge_graph.rs:577` 的 `DATA_DRIVEN_COLUMN.evidence` 行号 **`1748 → 1751`** | 核对：`src/commands/knowledge.rs:1751` 确为 `let rtype = fields[2].trim_matches('"').to_string();`，`:1748` 是 `}` ⇒ 改行号即**正解**（该测试刻意**不锁死行号**，而是按 evidence 给的行号**去磁盘读那一行再断言**，见 `:1160-1190`） |

**⚠ 更正：本轮我犯过一次「凭记忆引用证据」的错，必须记下。**
我在上一轮的答复里写过「**同样的红在 09-15 就已存在**（`output/verify-2026-09-15c/test-harness.log:697`，当时真值 1749）」。
本轮核验发现**两处都是假的**：

| 我此前的说法 | 核验结果（单文件 `stat` + 读正文） |
|---|---|
| 证据文件 `output/verify-2026-09-15c/test-harness.log` | **不存在** —— `output/` 下没有 `verify-2026-09-15c/` 这个目录。真实文件是 **`output/test-harness-2026-09-15c.log`**（mtime `2026-09-15T02:25:21Z`，57144 B / 968 行） |
| 该日志里「同样的红已存在」 | **事实相反**：该日志 `:494` 是 `... test_csv_import_relations_are_declared ... **ok**`，`:899` 是 `test result: ok. **684 passed; 0 failed**` |

**修正后的归属（全部有文件级证据，可复现）**：

1. **09-15 02:25** —— 该测试**绿**（`output/test-harness-2026-09-15c.log`）。
2. **09-15 08:15:47** —— `src/commands/knowledge.rs` 被改动（mtime），**晚于**该日志 **6 小时**
   ⇒ 是这次改动把 `let rtype = fields[2]` 推到了 **1751**，而 `DATA_DRIVEN_COLUMN.evidence` 没跟着挪
   ⇒ **红由此产生**（跨文件行号引用腐烂）。
3. **2026-09-17T08:48:25** —— `output/test-c2.log` 里该测试已 **FAILED**，panic 落在 `knowledge_graph.rs:**1139**:9`。
   而我在 **09:01–09:03** 的编辑使该断言迁到 **`:1184`**（`output/test-c1c3b.log`，mtime `09:09:25`，panic `:1184:9`）。
   位移 = **+45 行**，已核对**恰好等于**我新增的 `stable_relation_id` 块（现 `:854-898`，含 doc，**45 行**）；
   两条新测试（`:1332`/`:1352`）位于该测试**之后**，不参与位移。
   ⇒ 证明 **08:48 那次运行时该测试体还在 `:1139`**，即**红早于本轮 C-#1/C-#3 改动**。
4. ⇒ 性质 = **跨文件行号引用腐烂**，**由 09-15 08:15 的 `knowledge.rs` 改动引入**，
   最早观测于 **2026-09-17 08:48**；**不是**本轮改动引入。

**这条更正的通用价值** ⇒ 立为判据 **#481**：引用历史证据前必须 **`stat` 该文件 + 读正文**；
记忆或上一轮答复里的**路径与结论都可能整条是错的**（本次实测：路径不存在 **且** 结论与事实相反）。

##### 4. 门禁终读数（2026-09-17 收尾）

**命令**：`cargo test -p axagent-harness -p axagent-dao -p axagent-trajectory --no-fail-fast`
（日志 `output/test-final.log`，110420 B，耗时 **4m22s**）

| 包 / 目标 | 读数 |
|---|---|
| `axagent-dao` lib | **305 passed / 0 failed**（36.47s） |
| `axagent-dao` 集成测试 6 个（`knowledge_graph_search` / `orphan_memory_index` / `pg_cjk_fts` / `pg_upsert_bare_column` / `repo_crud` / `schema_compliance`） | **2 / 7 / 1 / 1 / 5 / 1 passed，全 0 failed** |
| `axagent-harness` lib | ✅ **706 passed / 0 failed**（上一轮 **705 / 1** ⇒ 那 1 条红已修，总数守恒 705+1=706） |
| `axagent-harness` `serialization_contracts` | **42 passed / 0 failed** |
| `axagent-trajectory` lib | **425 passed / 0 failed**（3.61s） |
| `axagent-trajectory` `causal_integration` | **22 passed / 0 failed**（27.44s，含本轮新增哨兵守卫） |
| doctests（3 包） | 6 passed / 0 failed（9 ignored） |

**汇总**：`test result:` 行 **14 条全部 `ok`**；`FAILED` 行 **0 条**；`error:` 行 **0 条**
⇒ 本轮**首次**做到三包全绿（此前 harness 恒有 1 条跨文件行号腐烂红）。

其余门禁：

| 门禁 | 读数 |
|---|---|
| `cargo fmt -p axagent -p axagent-harness -p axagent-dao -p axagent-trajectory -- --check` | **EXIT=0**（本轮 5 个改动文件 0 diff；加上修掉的 `memory_reflow_probe.rs`）。改证据引用后复检 `-p axagent-harness -p axagent-dao -- --check` 亦 **EXIT=0 / 0 字节输出** |
| `cargo clippy -p axagent-harness -p axagent-dao -p axagent-trajectory --all-targets --all-features -- -D warnings` | **0 warning / 0 error**，`Finished dev profile … in 2m 09s`（EXIT=0） |
| `check-single-source-facts.mjs --selftest` | **EXIT=0**（14 项自证全过，「两类判据都能红」） |
| `check-single-source-facts.mjs --ci` | **EXIT=0** —— 硬红 **4 → 0**（详见第 6 节） |
| `check-decl-evidence.mjs` | **EXIT=3** = 它的**软判据**出口；**结构腐烂 2 → 0** |
| `mem.mjs budget` | `7082 / 7168 B`（余量 **86 B**，EXIT=0） |
| 判据层 `MEMORY-RULES.md` | `276582 B / 478 条 / 最大编号 **481**`（本轮新增 #480 / #481） |

> ⚠ 本轮全量测试命令**未把退出码落进日志**（`echo EXIT=$?` 在重定向之外）⇒
> 退出码判据改用「14/14 `test result: ok` + 0 `FAILED` + 0 `error:`」，非 `$?` 直读。

**第二次全量（修完证据门禁之后重跑，`output/test-after-evidence.log`，4m29s）**：
读数**与上表逐字一致**（14/14 `ok` · 0 `FAILED` · 0 `error:` · harness `706/0` · dao `305/0` · trajectory `425/0` ·
`causal_integration` `22/0`）⇒ 第 6 节的引用修正**未破坏任何测试**（改动仅 `evidence` 字符串与 `//` 注释）。

##### 5. 仍未做（勿误读为已完成）

| 未做 | 原因 |
|---|---|
| **(c)** 清 `trajectory_patterns` 存量 2038 行 + 加 `UNIQUE(name)` | 删**生产数据** + schema 迁移 ⇒ 仍待**单独明确授权** |
| `knowledge_relations` 跨写者去重（4 个写者） | 缺 DB 唯一约束，同属 (c) 类决策 |
| 追溯存量随机 id 行 | 同上（`memory.rs` 的 id 形态改变后，旧随机行「找不到」⇒ 每个逻辑关系最多并存「旧随机行 1 + 新派生行 1」） |

##### 6. 顺带清掉：证据门禁（`--ci`）**硬红 4 处，其中 1 处是我自己造成的**

本项由「改完任何 `.rs` 就跑 `check-single-source-facts.mjs`」这条既有纪律触发（本轮我**第一遍忘了跑**，
是在回写文档时才补跑，**立刻**发现硬红）。**教训：这不是可选项。**

**修前读数**：`check-single-source-facts.mjs --ci` **EXIT=1**，`✖ 新增客观错 4 处`；
`check-decl-evidence.mjs` **EXIT=1**，`腐烂 2 条`。

| # | 硬红 | 归属（有文件级证据） | 修法 |
|---|---|---|---|
| 1 | `harness/knowledge_graph.rs → trajectory/src/causal.rs:305` 指到**空行** | ⚠ **我造成的**：`causal.rs` mtime `09-17T09:02:42Z`（我插 `CAUSAL_KB_ID` 块 `:36-49`）；且 `output/decl-evidence-after-fix.log`（**09-15 01:59**）显示当时该引用**有效**（`·`），`output/tmp-d9-ssf.log`（**09-16 03:44**）硬错 **0** ⇒ 该红是本轮引入 | 真值 = **`:322`**（`knowledge_base_id: Set(CAUSAL_KB_ID.to_string())`），改引用 |
| 2 | `harness/knowledge_graph.rs → src/commands/knowledge_graph.rs:563` 指到**空行** | 非本批：该文件 mtime `09-17T06:08:05Z`（**较早的会话轮次**）；同样满足「09-16 03:44 硬错 0」 | 真值 = **`:541`**（fixture 输入 `{"source": "Tauri", …, "relation": "uses"}`）；**同串里的 `:575` 也是烂的**（真值 `:553` = `assert_eq!(relations[0].relation_type, "uses");`），只因 `:575` 恰好落在**非空行**才没被硬拦 ⇒ 一并改 |
| 3–4 | `dao/migrations/schema_diff.rs → sqlite/table.rs:108` / `:182`「引用了不存在的文件」 | 非本批：`schema_diff.rs` mtime `09-17T05:46:24Z` | 根因不是路径错 —— vendored 版本 **`sea-query 1.0.2` 实存**（`Cargo.lock` = 1.0.2；`~/.cargo/registry/src/.../sea-query-1.0.2` 在盘）。是 `:586` 把路径**省略成** `` `.../sqlite/table.rs:108-182` `` ⇒ 门禁的第三方识别（`EXTERNAL_MARK = /[A-Za-z0-9_.-]+-\d+\.\d+\.\d+[/"`]/`，按**注释块**上下文匹配）**失效** ⇒ 被当成本仓路径。修法：该行写全 `` sea-query-1.0.2/src/backend/sqlite/table.rs:108-182 `` |

**顺带清掉另外 3 条同类软红（同一 +3 位移，全在同一文件）**：`src/commands/knowledge.rs` 的
`has_concept` 写入在 **`:1780`**（原引用 `:1777`）、`in_industry` 在 **`:1795`**（原 `:1792`，含文档 2 处）、
`employ_{position}` 在 **`:1811`**（原 `:1808`，含文档 1 处）；`DATA_DRIVEN_COLUMN` 文档行
`1748 → 1751`（**上一轮我只改了 `evidence` 字段、漏了同行文档，属半修**）。
⇒ 全部与 `let rtype` 的 `1748 → 1751` 是**同一个 +3 位移**（09-15 对该文件的那次改动）。

**修后读数**：`check-single-source-facts.mjs --ci` **EXIT=0**（硬红 **4 → 0**；软判据 **5 → 3**）；
`check-decl-evidence.mjs` **EXIT=3**（**结构腐烂 2 → 0**；`EXIT=3` 是它的软判据出口，按设计）。
本轮共改 **10 处行号引用**（harness 9 + schema_diff 1）。

**剩余软判据（**存量、非本轮、按设计容忍**，需人判，本轮未改）**：
`reconcile/apply.rs → render.rs:1100`（附近已无引号值 `"col"`）·
`harness/page_type.rs → init/services.rs:375`（指向收尾括号）·
`stock_workflow/core.rs → harness/workflow_types.rs:2565`（指向 `},`）·
`decl-evidence` 侧 5 条（`wiki.rs:1149`、`GraphView.tsx:116` ×3 等）。
⇒ 归因：`--ci` 对软判据**按设计容忍**，所以它们会一直存在直到有人判；**「0 命中 ≠ 没问题」也适用：NONLOC 137 条根本没被检查**。

### §5e 真 PG 侧实证（2026-09-17，**已做**）

**凭证路径**：`scripts/pg-connect.mjs`（口令 AES-256-GCM 存于 `~/.axagent/db_config.json`，
密钥在同目录 `master.key`，解密逻辑在仓库里）⇒ 走它的 **`sql` 子命令**：口令由脚本自己解密，
**不进 argv、不进日志**，只回 `rows` 的 JSON。目标：`axinvest`（生产，**只读**）与
`axagent_pg_migtest`（独立测试库）。

#### 1. 缺陷机制 —— **从推理变为实证**（对照实验）

同一列集 `(id, knowledge_base_id, name, entity_type, source_path, properties, created_at, updated_at)`，
**唯一变量是 `knowledge_base_id`**；两次都在 `BEGIN … ROLLBACK` 内，净写入 0：

| 测试 | `knowledge_base_id` | 结果 |
|---|---|---|
| **C1** | `'memory_default'`（幽灵） | **exit=1** · PG 原生报错：`插入或更新表 "knowledge_entities" 违反外键约束 "fk-knowledge_entities-knowledge_base_id"` |
| **C2** | `'__sys_trajectory__'`（真实存在） | exit=0，无错误 |
| **D** | 总数复核 | `knowledge_entities` 72816 → **72816**（零持久写入） |

外键定义（`pg_get_constraintdef` 原文）：
`FOREIGN KEY (knowledge_base_id) REFERENCES knowledge_bases(id) ON DELETE CASCADE`

#### 2. ⚠ 比原诊断更狠：**两个分支都必然失败**

`memory_namespaces` 在 `axinvest` 上共 **3 行**：`__sys_trajectory_memory__`(system) ·
`29fbb7e8-774a-4e75-b14a-96dc3ce24504`(Reflector Insights) · `d9ff67cf-aa76-4b45-988b-8507f4d10467`(opc-business)

- **没有 `memory_default`** ⇒ 兜底分支写的那个字面量**在任何表里都不存在**；
- 且实测 `memory_items` 里 `namespace_id` 为空的 **0 条** ⇒ **兜底分支从未被触发**；
- 实际走的是**非空分支**，`kb_id = 29fbb7e8-…`（记忆命名空间的 UUID）——
  它也**不在 `knowledge_bases` 里** ⇒ **同样外键违反**。

⇒ 结论：原实现**无论走哪个分支，写 `knowledge_entities` 都必然被 DB 拒绝**。
「静默空转」是**必然**而非**偶然**。

两个 id 空间互不相交（判据 **#468** 的真库印证）：
KB 的 UUID 是 `f9b2b050-e336-4de8-b305-0b40cbf5740f`，与两个 memory namespace 的 UUID **完全不同**。

#### 3. 存量数据状态

| 读数 | 值 | 含义 |
|---|---|---|
| `knowledge_entities` 中「`knowledge_base_id` 不在 `knowledge_bases` 里」的行 | **`[]`** | 幽灵写入**从未成功过**（与 §1 的机制一致） |
| `knowledge_entities` 按 KB 分布 | `lemonhu_knowledge_graph` 48542 · `f9b2b050-…` 24274 | 无哨兵 KB 下的行 |
| `knowledge_bases`（生产 3 行） | `__sys_trajectory__`(enabled=0) · `f9b2b050-…` · `lemonhu_knowledge_graph` | **`__sys_memory_reflow__` 不在** |
| `memory_items` | 共 **2 条**，`importance >= 0.7` 的 **0 条** | ⇒ **§6 第 3/4 条在存量库不可判**（须造数据） |
| `knowledge_bases` 的 NOT NULL 列 DEFAULT | `enabled=1` · `sort_order=0` · `kind='indexed'` | 我的 `..Default::default()` 策略在 PG 上安全 |
| `[A]` 我这条哨兵 INSERT（`BEGIN…ROLLBACK`） | **无报错** | 该 INSERT 被 PG **接受**（`[A2]` 回滚后仍 3 行 ⇒ 零写入） |

#### 4. 为什么 `__sys_memory_reflow__` 现在两库都不存在（**如实说明**）

它不是缺陷，而是**时间线**：该行由 `ensure_sentinels` 在**启动时**播种，而
`axinvest` 与 `axagent_pg_migtest` 都被**本次改动之前的二进制**访问过
⇒ 我的新增代码**尚未被任何启动路径加载**。
`__sys_trajectory__`（同形态、同函数）在两库里都存在 ⇒ 该机制在 PG 上**确已工作**。

#### 5. 当时仍缺的验收：需要真跑 Rust（**探针已备好，且随后已运行 —— 读数见第 6 节**）

§6 的 5 条里有 4 条（回流写入 / 两轮不增长 / `mention_count` / 失败可见）**必须跑生产函数**，
而存量库 `importance >= 0.7` 为 0 ⇒ 必须在独立库造数据。已新增探针

`src-tauri/crates/dao/examples/memory_reflow_probe.rs`

- **默认库 = `axagent_pg_migtest`**（安全侧：不显式传 `--db=` 就不会碰 `axinvest`）；
- **连接串不进 argv** —— 探针在进程内调 `pg-connect.mjs` 并读它的 stdout。
  ⚠ 这是与既有探针（`wiki_edge_relation_probe` / `p4_apply_probe` 走
  `"$(node scripts/pg-connect.mjs url)"`）的**关键区别**：那种形态会把口令落在 **argv**，
  并被 `Running … <exe> 'postgres://user:<口令>@…'` **回显** ——
  正是 `PLAN-declarative-schema-sync.md:2598` 记录过的那次泄漏。
- 六项判据：① 哨兵 KB 建出且 `enabled=0` ② `ensure_sentinels` 幂等 ③ 第 1 轮回流写 3 行
  ④ **第 2 轮行数仍为 3（区分力所在：旧写法会变 6 行）** ⑤ `mention_count == 2`（证明走 UPDATE）
  ⑥ 清理自造数据；
- ⚠ **当前状态**：探针已写成并**已运行**（读数见下）—— 见第 6 节。

#### 6. 端到端探针读数（`axagent_pg_migtest`，**PASS / exit=0**）

```
[P1] ensure_sentinels 之前/之后 knowledge_bases = ["__sys_memory_reflow__", "__sys_trajectory__"]
[P1] 回流 KB: name="System Memory Reflow" enabled=0
             description=Some("Auto-reflowed high-importance memory items (system-internal)")
[P2] 第二次 ensure_sentinels 之后 = 同上                ⇒ 幂等
[P3] 第 1 轮: items_read=3 entities_created=3 failures=0 ⇒ 该 KB 下实体行数 = 3
[P4] 第 2 轮: items_read=3 entities_created=3 failures=0 ⇒ 该 KB 下实体行数 = **3**（不增长）
[P5] 3 行，mention_count 全为 2，confidence = 0.8 / 0.85 / 0.9，entity_type=memory_item，
     且「全部来自探针自造条目? **true**」
[P6] 清理完成：实体残留 = 0，探针命名空间残留 = 0
=== 结论 === PASS
```

（两跑均 PASS —— 第二次是删掉哨兵行后的复跑，见下。）

**P4 是本项的核心证据**：旧写法（新生成 id + `on_conflict(Id)`）在这里会看到 **6 行**；
而 ①②③⑤ 四条在旧写法下**都会通过** ⇒ **只有 P4 有区分力**。

##### ⚠ P1 的证据完整性：改用「删除-重建对照」而非 before/after 差

探针**第一次**跑时 `before` 就显示该行已存在 ⇒ 一度怀疑「是别人建的」。
实测澄清：`create_pool` → `initialize_schema` 会在探针读 `before` **之前**就把哨兵行播种出来
⇒ **before/after 差这条判据本身没有区分力**。改用删除-重建对照：

| 步骤 | 读数 |
|---|---|
| ① SQL `DELETE FROM knowledge_bases WHERE id='__sys_memory_reflow__'` | 删除前该 KB 下实体 = **0**（无 CASCADE 损失） |
| ② SQL 复核 | `knowledge_bases` = **只剩 `__sys_trajectory__`** |
| ③ 复跑探针 | `before` **又是 2 行** —— 它在 `ensure_sentinels` 调用前就被建出 |
| ④ SQL 复核 | `knowledge_bases` = **2 行** ⇒ 该行已重建 |
| ⑤ 且 P1~P6 **再一次全部 PASS** | 可重复，非一次侥幸 |

⇒ **证明**：启动链 `create_pool` → `initialize_schema` → `ensure_sentinels` 在真 PG 上
**确实会建出哨兵行**，且**建连即跑**（不是等启动流程走到末端）。

##### §6 判据达标情况（真 PG 侧）

| §6 判据 | 状态 |
|---|---|
| 全新库上哨兵行存在 | ✅ 删除-重建对照（PG）· ✅ `create_test_pool_yields_sentinels`（SQLite） |
| 存量库启动一次后同样存在（幂等） | ✅ P1/P2（PG）· ✅ `ensure_sentinels_restores_deleted_rows`（SQLite，断言 2→3） |
| 回流后该 KB 下行数 > 0，且**无 787** | ✅ P3：3 行、`failures = 0`。⚠「787」是 **SQLite 的 `FOREIGN KEY constraint failed`**（见 `PLAN-declarative-schema-sync.md:1279`），**不是行数** ⇒ `failures = 0` 就是它的读数，「无 787」= 无外键失败 |
| 连跑两轮不线性增长 | ✅ P4（3 → 3） |
| 失败可见（计数型 warn） | ✅ P5b 三项断言 + **真打出一条计数型 warn**（见第 7 节后记） |

#### 7. 「失败可见」（§6 第 5 条）—— 分段闭合，一处**发现探针自身缺陷**

探针加了 `--inject-failure`：用一个**不存在**的 kb_id 调一次 `reflow_memory_to_knowledge`。

- ✅ **P5b 三项断言全过**：`items_read = 3`（场景非空）· `entities_created = 0`（外键全拒）·
  `failures = 3`（== `items_read`，逐条失败而非整体 `Err`）· 该假 KB 下实体行数 = 0。
- ⚠ **第一次跑时 stderr 里一条 warn 都没有** —— 原因不是生产代码，而是
  **探针没有初始化 tracing subscriber** ⇒ `tracing::warn!` **无处落地**。
  ⇒ 这是**探针自身的缺陷**：不接 subscriber 就无法观测日志，「失败可见」只能靠 stats 字段**推断**而验不成。
- 修法：给 `crates/dao/Cargo.toml` 的 `[dev-dependencies]` 加
  `tracing-subscriber = { workspace = true, features = ["env-filter"] }`（工作区已有该依赖），
  并在探针 `main` 开头 `try_init()` + 默认过滤 `axagent_dao=warn`。
  ⚠ 这使 dao 的 **dev-target 依赖集发生变化** ⇒ `clippy -p axagent-dao --all-targets` 需**重跑**才有效。
- ✅ **后记（当日 16:11）：修完重跑，warn 如期打出。**
  日志 `output/tmp-d-pg-probe3.log`（task `fKWeww`，3123 B）—— 编译
  `Finished dev profile [unoptimized + debuginfo] target(s) in 2m 15s`，随后运行
  `target\debug\examples\memory_reflow_probe.exe '--node=…' --inject-failure`；
  stderr 里出现**恰好一条**计数型 warn：

  > `2026-09-17T08:11:04.003443Z  WARN axagent_dao::repo::knowledge_graph: [memory_reflow] kb=__probe_nonexistent_kb__ 回流部分失败：读取 3 条，成功 0，失败 3；首个错误：Database error: Query Error: error returned from database: 插入或更新表 "knowledge_entities" 违反外键约束 "fk-knowledge_entities-knowledge_base_id" at line 2783`

  ⇒ 这一条同时坐实三件事：① 日志**确实**落地了（`debug!` → `warn!` 的改动在**真 PG** 上生效，
  不是只有推理）；② 形态是**一条计数型**（3 条失败只出 1 行，不是每条一行的噪声）；
  ③ 报错原文就是**外键约束**（`fk-knowledge_entities-knowledge_base_id`），与第 1、2 节的
  对照实验完全一致。`probe3_exit=0`，六项判据仍全 PASS。
- ⚠ **这次修复的代价，如实登记**：`[dev-dependencies]` 加 `tracing-subscriber` 会改变 dao 的
  dev-target 依赖集 ⇒ 触发全链重编译（实测 2m 15s，因多数 `.rlib` 已在），
  并且 **§5c 里那条 `clippy -p axagent-dao --all-targets` 的旧读数作废、必须重跑**。
  ✅ **已重跑：`1m 38s` + exit 0 + 0 error / 0 warning**（`output/tmp-d-v5-dao-clippy2.log`）。
  ✅ **测试层同样重跑**：`test -p axagent-dao --lib` = `303 passed; 0 failed`（24.53s / exit 0，见 §5c 后记 4）。

## §6 验收判据

- [x] 全新 PG 库与全新 SQLite 库上：哨兵行存在（`knowledge_bases.id = <sentinel>`）。
      ⇒ PG：删除-重建对照（§5e 第 6 节）｜ SQLite：`seed::tests::create_test_pool_yields_sentinels`。
- [x] 存量库：启动一次后同样存在（`ensure_sentinels` 幂等）。
      ⇒ PG：P1 / P2 ｜ SQLite：`seed::tests::ensure_sentinels_restores_deleted_rows`（2 → 3）。
- [x] 回流写入后 `knowledge_entities` 中该 KB 下的行数 **> 0**，且**无 787**。
      ⇒ PG P3：3 行、`failures = 0`。⚠「787」是 **SQLite 的 `FOREIGN KEY constraint failed`**
      （`PLAN-declarative-schema-sync.md:1279`），**不是行数** ⇒ `failures = 0` 就是它的读数。
- [x] 连跑两轮 consolidation，行数**不因重复回流而线性增长**（4.2 幂等性的验收式）。
      ⇒ PG P4：**3 → 3**（旧写法在此处会看到 **6 行** ⇒ 五条判据里**只有这条有区分力**）。
- [x] 失败可见：注入一次人为失败，能在日志里看到**计数型** warn（不是每条一行的噪声）。
      ⇒ PG P5b：`items_read=3 / entities_created=0 / failures=3`，且 stderr 里**恰好一条** warn
      （`output/tmp-d-pg-probe3.log`，详见 §5e 第 7 节后记）。

### 测试形态的纪律（勿再踩）

`crates/dao/tests/pg_migrations.rs` 已因 `AXAGENT_TEST_PG_URL` 门控、CI 不设 ⇒ **长期 skip 而 runner 报通过**
被退休，且退休时未备份（`PLAN-declarative-schema-sync.md:2964` 登记为执行者失误）。
⇒ 本项的 PG 侧验收**优先走探针**（`output/` 下的实跑日志），**不要**再写一条跑不到的 env-gated 测试。
SQLite 侧可用 `dao::db::create_test_pool()`（与生产同一条 `initialize_schema`）落成常规单测。

---

## §7 明确**不做**的事

- ❌ 不在本项顺手删/改 `knowledge_graph.rs:688 upsert_relation`（它有自己的登记：`PLAN…:2917`，接线前必修）。
- ❌ 不改 `importance` 阈值与条数上限（4.3 未裁决）。
- ❌ 不为了让断言变绿而关外键（历史已明确拒绝过这种「把缺陷盖回去」的修法）。
