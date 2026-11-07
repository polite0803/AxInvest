# 审计：UPSERT 裸列名的方言差异缺陷（PG 42702）

> 状态：active ｜ 日期：2026-09-17 ｜ 对应版本：v2.9.7


> 日期：2026-09-17 ｜ 触发：用户贴出运行期报错截图
> 范围：`src-tauri/crates/dao/src/repo/` 下两处 `ON CONFLICT ... DO UPDATE SET`
> 结论：**PG 专有缺陷，SQLite 完全不可见**；已修 + 已加 PG 端到端回归锁（反事实已证区分力）

---

## 0. 三句话结论

1. **报错原文** `Query Error: error returned from database: 字段关联 "communities_json" 是不明确的 at line 932`
   来自 `save_cached_graph` 的 UPSERT：`COALESCE(excluded.communities_json, communities_json)`
   里第二个参数是**裸列名**。PG 的 `DO UPDATE SET` 命名空间**同时**含目标表与 `excluded` 伪关系
   ⇒ 未限定的列名必然歧义（`42702`）。
2. **同批一起坏的还有一处**：`upsert_execution_feedback` 的三处表达式全是裸名
   （`usage_count + 1` / `successes + excluded.successes` / `failures + excluded.failures`）——
   实测**只要列名出现在 DO UPDATE SET 里就会歧义**，与「表达式里有没有 `excluded.`」无关。
3. **两处都改完，并留下一条只能由 PG 观测的回归锁**：修复前它红（且报错字段与用户截图逐字段一致），
   修复后绿。SQLite 内存库**锁不住这个缺陷**（实测 3.53.1 对全部写法宽容）。

`at line 932` **不是你的 SQL 行号**，是 PG 后端 C 源码行号（`parse_relation.c:932` = `colNameToVar`）——
sqlx 0.9 的 `PgDatabaseError` Display 会把它一并打出来（`sqlx-postgres-0.9.0/src/error.rs:157-165`）。

---

## 1. 归因：把错误串拆三段，再用唯一性收口

| 片段 | 出处（已源码取证） | 含义 |
|---|---|---|
| `Query Error:` | SeaORM `DbErr::Query` 的 Display（`sea-orm-2.0.2/src/error.rs`） | 查询类 DbErr。对照：`DbErr::Custom` 打 `Custom Error:`；`AxAgentError::Database` 又是另一种前缀 ⇒ **可用来区分是哪个返回类型抛的** |
| `error returned from database:` | sqlx `Error::Database` 的 `#[error(...)]`（`sqlx-core-0.9.0/src/error.rs:44`） | 确认底层是 sqlx |
| `at line 932` | `impl Display for PgDatabaseError`：`write!(f, " at line {line}")`，`line()` = PG 错误报文里的 `L` 字段 | **PG 后端源码行号**，不是 SQL 行号 |

**唯一性收口**：全仓 grep `communities_json`（排除 `target/`），引用该**列**的 SQL 只有一处 ——
`save_cached_graph` 的 UPSERT。另外两处命中是 `save_cached_communities` 的 `update_many().col_expr(...)`
（普通 UPDATE，命名空间里只有目标表，**不歧义**）与实体声明。

版本（`Cargo.lock` 实读）：`sea-orm 2.0.2` · `sea-query 1.0.2` · `sqlx 0.9.0` · PG 18 本地 `axinvest` 库。

### 实际生成的 SQL（grep 生成器源码得出，非推测）

`sea-query-1.0.2/src/backend/query_builder.rs`：
`prepare_on_conflict_excluded_table()` 产出 `"excluded"."col"`（带引号 + 限定），
`OnConflictUpdate::Expr(col, expr)` 则把**你给的表达式原文**直接写出。于是修复前是：

```sql
INSERT INTO "wiki_graph_cache" ("vault_id", ..., "updated_at") VALUES ($1, ...)
ON CONFLICT ("vault_id") DO UPDATE SET
  "graph_data_json" = "excluded"."graph_data_json",   -- 库自己限定 ✅
  ...
  "communities_json" = COALESCE(excluded.communities_json, communities_json)
                                                          -- ↑ 只有这一项是裸名 ❌
```

---

## 2. 为什么只在 PG 暴露：方言 × 写法 矩阵（真库实测）

PG 18 上用一个 TEMP 表逐写法跑（每格包 `EXCEPTION` 捕获；SQLite 用 `python` 自带 `sqlite3` 逐格新建 `:memory:`）：

| 写法（`SET b = ...`） | PG 18 | SQLite 3.53.1 |
|---|---|---|
| `COALESCE(excluded.b, b)` | ❌ **42702** | ✅ |
| `b + excluded.b` | ❌ **42702** | ✅ |
| `b`（**仅裸名，表达式里没有 `excluded.`**） | ❌ **42702** | ✅ |
| `COALESCE(excluded.b, t.b)` | ✅ | ✅ |
| `"t"."b" + "excluded"."b"`（带引号形态） | ✅ | ✅ |
| **不把该列放进 SET** | ✅ | ✅ |
| `excluded.b`（覆盖写） | ✅ | ✅ |

**这条矩阵里最容易搞错的一格是第 3 行**：判据不是「有没有和 `excluded` 同框」，
而是「**这个列名有没有出现在 DO UPDATE 的 SET 里**」。
⇒ `usage_count + 1` 这种看起来无害的自增，与 `COALESCE(excluded.x, x)` 同病。

**推论（本缺陷的根因性质）**：SQLite 对裸列名**全部宽容** ⇒ 这类缺陷在 SQLite 模式（默认回退路径、
以及任何 SQLite 内存库单测）下**零可观测性**。

---

## 3. 同型穷举：2 文件 4 行（不是"顺手看一眼"）

规则化扫描脚本 `output/verify-graph-2026-09-15/onconflict-bare-column-sweep-0917.mjs`：

1. 遍历 `src-tauri/{src,crates}/**/*.rs`（跳过 `target/`、`node_modules/`）；
2. 对每个 `.on_conflict(` 从其 `(` 起做**括号配平**取块；
3. **全量清单**：块内所有含 `.value(`/`.values(`/`.update_column(s)(`/`.col_expr(`/`Expr::` 的行逐站点打印；
4. **补集法**：块内一行都没被上面正则命中的站点，整块原样打印；
5. 计数自证（任一为 0 即视为脚本自身失效）。

实测计数与结果：

```
scannedFiles=1933  onConflictSites=45  suspects=4  inventoryLines=125
补集（零命中的站点）= 0
```

| 文件 | 行 | 内容 |
|---|---|---|
| `crates/dao/src/repo/wiki_graph_cache.rs` | 108 | `Expr::cust("COALESCE(excluded.communities_json, communities_json)")` |
| `crates/dao/src/repo/evolution_execution_stats.rs` | 67 | `(Column::UsageCount, Expr::cust("usage_count + 1"))` |
| 同上 | 68 | `(Column::Successes, Expr::cust("successes + excluded.successes"))` |
| 同上 | 69 | `(Column::Failures, Expr::cust("failures + excluded.failures"))` |

其余 44 个站点**全部**是 `update_column(s)`（库自己产出 `"excluded"."col"`）或 `Expr::value`——
即**结构性安全**；`ast_index` 的 5 处是 `do_nothing()`。
补集为 0 意味着「没有任何站点是靠正则不匹配而被跳过的」。
另 grep `OnConflict::` 确认构造点都紧挨 `.on_conflict(`，**不存在** `let oc = OnConflict::…` 的间接传递。

> ⚠ 补集法这一步不能省。没有它，「正则没命中」会被读成「该站点安全」——
> 而「日志/命中缺失 ≠ 分支为真」是本项目已被反复教训的判据。

### 同源性

两处坏点来自**同一次重构（2026-09-16「去方言分支」）**，且两个文件的头注释写着**同一句错判**：

- `wiki_graph_cache.rs`：~~「两方言均支持裸列名引用现有行（PG 的表限定写法在 SQLite 不一定被接受，
  故统一为裸列名）」~~
- `evolution_execution_stats.rs`：~~「原 PG 分支写 `…usage_count + 1`（表限定）……PG 同样接受裸列名，故统一为裸列名」~~

即：**PG 侧原有的表限定被当作冗余删掉了**。事实恰好相反 —— 表限定才是 PG 必需的那半边。
两处头注释已改写为「改造记录 2」/「改造记录 1」并写清真判据。

---

## 4. 修复

### 4.1 `save_cached_graph`（报错点）：按 Some/None 决定该列是否进 SET

```rust
// 以「实际要写入的值」为准判定，与原来的 COALESCE 语义逐例等价：
// 写入 NULL ⇔ 不进 SET ⇔ 旧值保留；写入非 NULL ⇔ 进 SET ⇔ 覆盖。
let write_communities = communities_json.is_some();
...
let mut set_columns = vec![Column::GraphDataJson, Column::NodeCount, Column::EdgeCount,
                           Column::ComputedAt, Column::UpdatedAt];
if write_communities { set_columns.push(Column::CommunitiesJson); }

WikiGraphCache::insert(am)
    .on_conflict(OnConflict::column(Column::VaultId).update_columns(set_columns).to_owned())
```

**语义等价论证**（不是"看起来一样"）：原来 `COALESCE(excluded.c, c)` 的两个分支
分别对应「写入值为 NULL ⇒ 取旧值」与「写入值非 NULL ⇒ 取新值」；
新写法就是这两条按同一个判据（`communities_json.is_some()`）分流。
判定键刻意取**序列化后的实际写入值**而非入参 `communities`，
这样连「`to_string` 失败 ⇒ 写 NULL」这种边角也与原语义逐例一致。

**附带收益**：少一处 raw SQL（`Expr::cust`），少一处方言相关写法。

### 4.2 `upsert_execution_feedback`（同族）：改为表限定的表达式

```rust
use sea_orm::ExprTrait;                       // ⚠ 必须在函数内导入，理由见下
let table = Alias::new(EvolutionExecutionStats.table_name());   // 表名取自实体，不手抄
...
.values([
    (Column::UsageCount, Expr::col((table.clone(), Column::UsageCount)).add(1)),
    (Column::Successes,  Expr::col((table.clone(), Column::Successes)).add(success_inc)),
    (Column::Failures,   Expr::col((table.clone(), Column::Failures)).add(failure_inc)),
])
```

- 渲染为 `"evolution_execution_stats"."usage_count" + 1`（`prepare_column_ref` 对
  `ColumnName(Some(table), col)` 先 `prepare_table_name` 再 `"."` 再列名 —— 已源码确认）；
- 表名走 `Entity.table_name()`（`EntityName` trait），**不手抄字符串** ⇒ 实体改名自动跟随；
- 递增项不再引用 `excluded.*`：`excluded.successes` 的值就是 `Set(success_inc)` 里那个变量，
  与 `ActiveModel` **同源**，少一处方言相关写法。

---

## 5. 回归锁：只能由 PG 观测

新增 `src-tauri/crates/dao/tests/pg_upsert_bare_column.rs`（env-gated，变量名沿用全仓统一的
`AXAGENT_TEST_PG_URL`）。三段断言：

1. 首次写入（`None`）必须成功 —— **修复前就是这一步炸**；
2. 有冲突的更新（`Some`）必须成功，且社区真的落库；
3. **再以 `None` 写入 ⇒ 断言旧社区仍在** —— 锁住「省略该列」与「COALESCE 保留旧值」的等价性
   （只做 ①② 会漏掉「改成了无条件覆盖」这种退步）。

未设置环境变量时**诚实跳过**并打印「本测试未通过，也未被验证」；表不存在时同样报 SKIP 说明原因。
探针行主键 `__probe_upsert_bare_column__`，末尾删除并断言无残留。

### 5.1 反事实（证明它有区分力，不是"永远绿"的摆设）

把 `save_cached_graph` **临时改回修复前的裸列名写法**再跑同一测试：

```
thread '...' panicked at crates\dao\tests\pg_upsert_bare_column.rs:162:10:
① 首次写入必须成功：裸列名会在此报 42702 column reference is ambiguous:
Query(SqlxError(Database(PgDatabaseError { severity: Error, code: "42702",
  message: "字段关联 \"communities_json\" 是不明确的",
  position: Some(Original(485)), file: Some("parse_relation.c"), line: Some(932),
  routine: Some("colNameToVar") })))
test result: FAILED. 0 passed; 1 failed
```

**与用户截图逐字段一致**：`code 42702` · `message 字段关联 "communities_json" 是不明确的` ·
`file parse_relation.c` · `line 932` · `routine colNameToVar`。
⇒ 归因闭环（同一错误、同一列、同一 PG 源码行），且该测试**确实会红**。
随后从备份还原，三个文件 sha256 逐一比对一致（见 §6）。

---

## 6. 门禁与产物清单

| 门禁 | 结果 |
|---|---|
| `cargo clippy -p axagent-dao --all-targets -- -D warnings` | **EXIT=0**（`Finished dev profile in 49.16s`） |
| `cargo fmt -p axagent-dao -- --check` | **通过**（首轮曾报新测试文件 3 处差异，已按 rustfmt 修） |
| `cargo test -p axagent-dao --test pg_upsert_bare_column` | **1 passed; 0 failed**（真连 PG，非 SKIP） |
| 反事实（裸列名写法） | **1 failed**（42702，字段与截图一致） |

改动文件（未再变，已备份）：

| 文件 | 字节 | sha256[:16] |
|---|---|---|
| `src-tauri/crates/dao/src/repo/wiki_graph_cache.rs` | 6720 | `9857af4d3aea6dc8` |
| `src-tauri/crates/dao/src/repo/evolution_execution_stats.rs` | 5838 | `ffd69ff6fb2a300c` |
| `src-tauri/crates/dao/tests/pg_upsert_bare_column.rs`（新增） | 8253 | `07ba4df737430429` |

备份：`output/backup-2026-09-17-pgfix/`（3 文件 + `CHECKSUMS-pgfix-0917.txt`）。

证据产物（`output/verify-graph-2026-09-15/`）：

| 文件 | 内容 |
|---|---|
| `pg-ambiguity-probe-0917.mjs` / `.json` / `.log` | PG 18 五变体矩阵（V1 裸名 42702 / V2 表限定 ✅ / V3 省略 ✅ …） |
| `sqlite-ambiguity-probe-0917.py` / `.json` | SQLite 未加引号七变体（全 ✅） |
| `sqlite-ambiguity-quoted-0917.py` / `.json` | SQLite 引号形态四变体（`"t"."b"` 亦 ✅ ⇒ 元组限定可移植） |
| `onconflict-bare-column-sweep-0917.{mjs,txt,json}` | 45 站点穷举 + 全量清单 + 补集 |
| `onconflict-zero-hit-0917.txt` | 补集段（0 个站点） |
| `gate-clippy-dao-0917{,b,c,d}.log` | clippy 四轮（含三次失败，见 §7） |
| `gate-pgtest-0917.log` | 修复后 PG 回归绿 |
| `gate-pgtest-counterfactual-0917.log` | 反事实红（42702 逐字段） |
| `final-{fmt,clippy,pgtest}-0917.log` | 还原后最终三连 |

---

## 7. 我自己本轮的错误（编译器与真库逐个打回）

1. **默认 `Expr::col(...).add(1)` 可用** —— 实际 `Expr::col` 返回 `Expr` 枚举，`add` 来自
   `ExprTrait` ⇒ `E0599 no method named add`。是 clippy 第一次跑出来的，不是我读出来的。
2. **把 `use sea_orm::ExprTrait;` 提到模块级** —— 它是对**所有类型**的 blanket impl
   ⇒ 同模块 `row.usage_count.max(0)` 立刻 `E0034 multiple applicable items in scope`
   （`ExprTrait::max` vs `Ord::max`）。改为**函数内导入**并注释原因。
3. **`Entity::default().table_name()`** —— clippy `default_constructed_unit_structs`
   （`-D warnings` 下直接红）⇒ 用单元结构体字面量 `Entity.table_name()`。
4. **首次两轮探针没跑起来**：一轮是 shim bash 的 PATH 整段失效（`dirname: command not found`），
   命令根本没执行；一轮是 `pg-connect.mjs sql` 只支持单条语句（多语句时 node-postgres 返回结果
   **数组**，脚本取 `res.rows` ⇒ 打印 `undefined`）。前者改绝对路径 + 走 PowerShell；后者另写逐条执行
   的脚本，**没有**去改那个共享工具。

> 这 4 条都不是"想错了"，是"以为对但编译器/真库不同意"——正是必须实跑的理由。

---

## 8. 系统性建议与未处置登记

### 8.1 建议（不擅改，交由你裁决）

**任何「统一双方言写法」的重构都必须配一个 PG 端到端验证。**
本次事故的形状很典型：重构把「PG 分支手写的表限定」当成方言冗余删掉，理由是「两方言均支持裸列名」——
而 SQLite 侧的单测**结构上不可能**证伪它（宽容）。这类缺陷的可观测面只有 PG。
仓内 `pg_cjk_fts.rs` / `search/tests/pg_integration.rs` 已是同类先例，但仍是 env-gated（CI 不跑）。

**建议二**：`scripts/pg-connect.mjs` 的 `sql` 子命令建议支持多语句（结果数组时打印每个 Result，
或至少打最后一条）——现在多语句静默打印 `undefined`，会诱导人以为"查询没返回"。
**本轮未改**（属共享工具，改它影响他人用法），只登记。

### 8.2 未处置

- `evolution_execution_stats` 的**历史数据**无需迁移（修的是 SQL 生成，不是列/值）；
  但该 upsert 在修复前**每次调用都失败**（`AxAgentError::Database`），
  ⇒ 若「进化产物执行统计」在界面上出现过异常或长期为 0，可用同一张表交叉核对是否有缺口。
  **本轮未评估该影响面**（涉及产品语义：缺口是否要回填），只登记。
- 用户当前运行的 `npm run tauri dev` 实例仍是**修复前的二进制**，需要一次 Rust 侧重编译才会带上本修复。

---

## 9. 复现与验证步骤

```bash
cd /d/OneManager/AxInvest

# 1) 复现原始缺陷形态（PG 18，TEMP 表，无库内残留）
"C:/Users/polit/.workbuddy/binaries/node/versions/22.22.2-3/node.exe" \
  output/verify-graph-2026-09-15/pg-ambiguity-probe-0917.mjs

# 2) 同型穷举
"C:/Users/polit/.workbuddy/binaries/node/versions/22.22.2-3/node.exe" \
  output/verify-graph-2026-09-15/onconflict-bare-column-sweep-0917.mjs

# 3) 门禁（只限 dao crate，勿跑 workspace 级以免长时间持锁）
cd src-tauri
cargo fmt -p axagent-dao -- --check
cargo clippy -p axagent-dao --all-targets -- -D warnings

# 4) PG 端到端回归（跳过 ≠ 通过，务必看输出里的是 ok 还是 SKIP）
AXAGENT_TEST_PG_URL="$(node ../scripts/pg-connect.mjs url)" \
  cargo test -p axagent-dao --test pg_upsert_bare_column -- --nocapture
```
