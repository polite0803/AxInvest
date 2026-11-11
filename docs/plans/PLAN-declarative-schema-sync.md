# PLAN-declarative-schema-sync

> 状态：done ｜ 日期：2026-09-16 ｜ 对应版本：v2.9.7


> **目标态（已裁决，不再讨论取舍）**：删除全部 74 个 `v*.rs` 迁移文件与版本号机制。
> 库中存指纹 A，运行时从 SeaORM 实体元数据算出指纹 B；`A ≠ B` 即自动把库改造成 B 的形状——
> **含破坏性变更、双方言对等、无人工确认**。种子数据剥离出引擎。
>
> **本文档性质：架构设计**（非可行性评估）。判据是「每一种未来的 schema 变更，都能映射到引擎的哪条规则」。
> 映射不出来的地方 = 架构缺陷，必须先补设计再实施。§三 是这张映射表本身。
>
> 设计日期：2026-09-16 ｜ 取证方式：源码级 + 量化统计（未运行构建、未连生产库）
> 修订：**v4** — 由评估转为设计。前两版有两处论证被源码否证，见 §一。

---

## 零、决策基线

| 维度 | 档位 | 对设计的硬约束 |
|---|---|---|
| 验收标准 | **彻底删除** | 目标态 = 仓库搜不到任何 `v*.rs`。引擎必须覆盖 100% 历史语义，没有"留几个文件兜底"的退路 |
| 破坏性变更 | **引擎全权** | 自动执行删列/删表/改名/收紧。**必须自带自动安全网**（§四·七），不能靠人工确认兜底 |
| 方言 | **PG + SQLite 对等** | 必须实现 SQLite 的表重建路径（无 `ALTER COLUMN` 时的唯一手段） |
| 种子数据 | **无此需求** | 引擎不管默认数据；`v101`/`v131` 的默认行随迁移一起消失 |

---

## 一、被源码否证的两处前版论断（先纠正，再设计）

### 1.1 ❌「diff 视角下改名不存在」→ **错**

SeaORM 2.0.1 **原生支持列级改名声明**，且官方 sync 就在消费它。

`sea-orm-macros-2.0.1/src/derives/entity_model.rs:321-329`：

```rust
} else if meta.path.is_ident("renamed_from") {
    let lit = meta.value()?.parse()?;
    if let Lit::Str(litstr) = lit {
        renamed_from = Some(litstr.value());
    } else { return Err(meta.error(format!("Invalid renamed_from {lit:?}"))); }
}
```

`sea-orm-2.0.1/src/schema/builder.rs:429-453`（官方 sync 的实际分流）：

```rust
let mut renamed_from = "";
if let Some(comment) = &column_def.get_column_spec().comment
    && let Some((_, suffix)) = comment.rsplit_once("renamed_from \"")
    && let Some((prefix, _)) = suffix.split_once('"')
{ renamed_from = prefix; }

if renamed_from.is_empty() {
    db.execute(TableAlterStatement::new().table(..).add_column(column_def)).await?;      // 加列
} else {
    db.execute(TableAlterStatement::new().table(..)
        .rename_column(renamed_from.to_owned(), column_def.get_column_name())).await?;   // 改名
}
```

所以「改名 vs 删旧建新」的语义歧义**有官方解法**：实体上写 `#[sea_orm(renamed_from = "industry_id")]`。
前两版把它判为"信息缺失、无解"，是因为只看了 `SchemaBuilder` 的公开文档（那份文档只承诺"加"，没提改名），没读实现。

**代价要记清**：这个语义是通过 **column comment 字符串**传递的（`sea-orm-2.0.2/src/schema/entity.rs:270-278` 把 `renamed_from` 拼进 comment，`sea-orm-2.0.2/src/schema/builder.rs:430` 再 `rsplit_once` 解析回来）。是字符串契约，脆弱但可用；自研引擎照抄这条通道，并额外提供直接声明入口（§四·二）。

### 1.2 ❌「索引真相源不在实体里，所以索引不能自动化」→ **不成立**

实测 179 条 `CREATE INDEX`：**162 条是单列非唯一索引**（90.5%），`#[sea_orm(indexed)]` 可直接表达。
"168 个实体只有 1 个 `indexed` 注解"说明的是**待补注解的工作量**，不是**表达能力不足**。两件事前版混为一谈了。

---

## 二、架构总览

### 2.1 三阶段

```
启动
 └─ reconcile::run(db)
     ├─ ① INTROSPECT  读实际库结构 → SchemaModel（中立模型）
     ├─ ② FINGERPRINT 算 B(实体) 与 A(库中记录)
     │    └─ A == B → 直接返回（性能短路，绝大多数启动走这条）
     └─ ③ PLAN+APPLY  差异 → Vec<Change> → 拓扑排序 → 分组 → 双方言渲染 → 事务执行
          ├─ 写审计（DDL 全文 + 前后指纹）
          ├─ 破坏性操作前落墓碑（§四·七）
          └─ 回写 A = B
```

### 2.2 三层真相源（这是"零迁移文件"的载体）

| 层 | 形态 | 承担 | 是否"手写" |
|---|---|---|---|
| **L1 实体声明** | SeaORM 实体上的属性 | 表/列/类型/可空/默认/单列索引/唯一/FK/列改名 | 只改实体，不是新文件 |
| **L2 例外声明** | `reconcile/extras.rs` 的 `&'static [Decl]` | 实体表达不了的 DDL：CHECK、生成列、GIN/partial/多列索引、触发器、虚表、函数、表级改名 | 是，但**无版本号、幂等、状态驱动** |
| **L3 程序式规则** | `reconcile/rules/*.rs` 的 `fn(&DatabaseConnection)` | 需要 DML 的：值域改名、数据搬迁、去重、复杂回填 | 是，同样无版本号、幂等 |

**L2/L3 不是"迁移文件"**：它们不参与版本序、不带 `up()` 时序、没有"跑过就不跑"的状态；每次启动都按库的**当前状态**决定是否动作。漏执行不会永久错——下次启动自动补。

### 2.3 与现状的机制对应

| 能力 | 现状 | 新架构 |
|---|---|---|
| 全量重跑 | `repair_schema` 无条件遍历 `MIGRATIONS`（`migrations/mod.rs:783`） | `reconcile::run` 每次启动 |
| 幂等 | 项目硬约定（`mod.rs:549-552`）+ 43/74 有测试 | 仍要求幂等，但由引擎保证（见 §四·四） |
| 版本号 | `axagent_schema_version` 表 + `applied_version` 比对 | **降级为性能缓存**：指纹相等即跳过 |
| 建表 | 手写 DDL（`v100`/`v200`，177 条 `CREATE TABLE`） | L1 实体派生 |
| 加列 | 手写 `ALTER TABLE ADD COLUMN` | L1 自动 |
| 改名 | 手写 `v232`/`v233` | L1 `renamed_from`（列）+ L2（表） |
| 破坏性 | 手写 `DROP ... IF EXISTS` | 引擎自动（孤儿判定） |

---

## 三、完整性映射：全部变更类型 × 引擎规则

**这张表就是"能不能解决所有问题"的答案**。左侧穷举 schema 变更的全部形态，右侧给出引擎实现路径。

| # | 变更类型 | 真相源 | PostgreSQL | SQLite |
|---|---|---|---|---|
| 1 | 新增表 | L1 `table_name` | `CREATE TABLE`（FK 拓扑排序） | 同 |
| 2 | 删除表 | 孤儿判定 §四·九 | `DROP TABLE` | 同 |
| 3 | **表改名** | L2 `TableRenameDecl` | `ALTER TABLE .. RENAME TO` | 同 |
| 4 | 新增列 | L1 字段 | `ADD COLUMN` | 同 |
| 5 | **列改名** | L1 `renamed_from` | `RENAME COLUMN` | `RENAME COLUMN`（3.25+） |
| 6 | 删除列 | 孤儿判定 | `DROP COLUMN` | 重建表 |
| 7 | 类型变更 | L1 `column_type` / Rust 类型 | `ALTER COLUMN TYPE`（+`USING`） | 重建表 |
| 8 | 加 NOT NULL | L1 非 `Option<T>` | `SET NOT NULL` | 重建表 |
| 9 | 去 NOT NULL | L1 `Option<T>` | `DROP NOT NULL` | 重建表 |
| 10 | 加/改 DEFAULT | L1 `default_value` / `default_expr` | `SET DEFAULT` | 重建表 |
| 11 | 删 DEFAULT | L1 无 default | `DROP DEFAULT` | 重建表 |
| 12 | 单列索引 增/删 | L1 `indexed` | `CREATE/DROP INDEX` | 同 |
| 13 | 多列索引 增/删 | L2 `IndexDecl` | `CREATE/DROP INDEX` | 同 |
| 14 | partial 索引 | L2 `IndexDecl{where}` | `CREATE INDEX .. WHERE` | 同 |
| 15 | GIN / 表达式索引 | L2 `IndexDecl{method}` | `CREATE INDEX .. USING gin` | 声明 skip + 告警 |
| 16 | 唯一约束 增/删 | L1 `unique` / `unique_key` | `ADD/DROP CONSTRAINT` | 重建表 |
| 17 | 外键 增/删 | L1 `belongs_to` + `on_delete` | `ADD/DROP CONSTRAINT` | 重建表 |
| 18 | CHECK 约束 增/删 | L2 `CheckDecl` | `ADD/DROP CONSTRAINT` | 重建表 |
| 19 | 生成列 | L2 `GeneratedColDecl` | 重建表 | 重建表 |
| 20 | 触发器 | L2 `TriggerDecl` | `CREATE OR REPLACE` | `DROP` + `CREATE` |
| 21 | 虚表（FTS5） | L2 `VirtualTableDecl` | n/a（PG 侧走 `tsvector` + GIN） | `CREATE VIRTUAL TABLE` |
| 22 | 自定义函数 | L2 `FunctionDecl` | `CREATE OR REPLACE FUNCTION` | n/a |
| 23 | 数据回填 / 搬迁 | L3 规则 | 任意 DML | 同 |
| 24 | 值域改名（枚举值） | L3 规则 | `UPDATE .. WHERE 旧值` | 同 |
| 25 | 列拆分 / 合并 | L1（新列）+ L3（搬数据） | DML | 同 |

**覆盖结论：25/25 有实现路径，0 项无解。** 其中 L1 覆盖 1–2、4–12、16–17（共 14 项），L2 覆盖 3、13–15、18–22（共 10 项），L3 覆盖 23–25（3 项，L1/L2 配合）。

### 3.1 为什么 6 类 SQLite 动作必须走"重建表"

SQLite 只支持 `ADD COLUMN` / `RENAME COLUMN` / `RENAME TO`，**其余全部**要重建：

```
PRAGMA foreign_keys=OFF;            -- 必须在事务外设置（事务内无效）
BEGIN;
  CREATE TABLE t_new (...新定义...);
  INSERT INTO t_new SELECT <列映射> FROM t_old;   -- 列改名/删除在此映射
  DROP TABLE t_old;
  ALTER TABLE t_new RENAME TO t_old;
  -- 重建该表全部索引/触发器
  PRAGMA foreign_key_check;         -- 失败则 ROLLBACK
COMMIT;
PRAGMA foreign_keys=ON;
```

三个必须处理的坑：
1. `PRAGMA foreign_keys` **在事务内是 no-op**，必须事务外先关。
2. `legacy_alter_table` 行为差异：重建期间若该表被其他表引用，需临时 `PRAGMA legacy_alter_table=ON`。
3. `PRAGMA foreign_key_check` 要在 `COMMIT` **之前**，否则约束已落地无法回滚。

---

## 四、关键机制设计

### 四·一 指纹 A/B

**B（期望）** = SHA-256（排序后的规范化 JSON）：

```json
{
  "engine_rev": "1",
  "dialect": "postgres|sqlite",
  "tables": [{
    "name": "...", "pk": ["id"],
    "columns": [{"name":"...","type":"...","nullable":false,"default":"...","renamed_from":"..."}],
    "indexes": [{"name":"idx_x_y","cols":["x","y"],"unique":false,"where":null,"method":null}],
    "fks":  [{"cols":["a"],"ref_table":"b","ref_cols":["id"],"on_delete":"Cascade"}],
    "checks": ["..."], "extras": ["mqtt_trigger_v1"]
  }]
}
```

**三个要点**：
- `engine_rev` 必须进指纹 —— 引擎规则本身变了（比如新增一类 DDL 支持）也要触发重跑。
- `dialect` 必须进指纹 —— PG 与 SQLite 的期望结构本就不同（GIN 索引 vs FTS5 虚表），共用一个 A 会互相踩。
- `extras` 进指纹 —— L2 声明改了也要重跑。
- **必须运行时算，不能放 build.rs**：`EntityTrait` 是运行时 trait，build.rs 拿不到；只能在源文本层解析宏属性，必然漂移。

**存储**：

```sql
CREATE TABLE _ax_schema_state (
  id INTEGER PRIMARY KEY CHECK (id = 1),   -- 单行表
  fingerprint TEXT NOT NULL,
  engine_rev TEXT NOT NULL,
  applied_at BIGINT NOT NULL
);
CREATE TABLE _ax_schema_audit (          -- 只增不改，审计用
  id INTEGER PRIMARY KEY,
  applied_at BIGINT NOT NULL,
  from_fp TEXT NOT NULL, to_fp TEXT NOT NULL,
  ddl_json TEXT NOT NULL,                -- 本次执行的全部语句
  dropped_json TEXT                       -- 被删除对象的墓碑索引
);
```

### 四·二 改名语义（两条通道）

| 对象 | 通道 | 形态 |
|---|---|---|
| 列 | **L1 实体属性** | `#[sea_orm(column_name = "domain_pack_id", renamed_from = "industry_id")]` |
| 表 | **L2 声明** | `TableRenameDecl { from: "opc_industry", to: "opc_domain_pack" }` |
| 值域 | **L3 规则** | `UPDATE t SET c = 'new' WHERE c = 'old'`（幂等：二次命中 0 行） |
| 索引 | 无需声明 | 引擎按"旧名消失 + 新名出现 + 列集相同"自动识别为改名，走 `ALTER INDEX RENAME` |

**列改名在引擎里的判定顺序**（必须严格按此顺序，否则会退化成"删旧列+建新列"）：

```
1. 实体列声明了 renamed_from，且库中存在该旧列  →  RENAME COLUMN（数据保留）
2. 实体列无 renamed_from，库中也不存在          →  ADD COLUMN（新列）
3. 实体列无 renamed_from，但库中存在同列名      →  类型/约束对账
4. 库中有列、实体无对应列且无人声明 renamed_from →  判为孤儿 ⇒ DROP（引擎全权）
```

第 4 条是**全架构最危险的一步**，缓解见 §四·七 与 §五。

### 四·三 变更排序与分组原子性

`Vec<Change>` 必须排序后才执行：

1. **建表**按 FK 拓扑序（`sea-orm-2.0.2/src/schema/builder.rs:284` 官方也这么做）
2. **删表**按逆拓扑序
3. **列变更**（增/改/删）先于该表上的索引与约束
4. **约束删除**先于依赖它的数据变更；**约束重建**在其后
5. **索引/触发器**最后重建

**分组（`ChangeGroup`）是必须的**，源自一个真实事故：`v232:189-197` 记录的死锁——旧 CHECK 允许 `opc-industry`、拒绝 `opc_domain_pack`，而代码侧正好相反，于是"改数据"被旧约束拒、"重建约束"又被旧数据拒，**单步执行永远解不开**。

```rust
ChangeGroup { label: "opc_domain_pack_value_rename",
    steps: [ DropCheck(..), RunSql("UPDATE.."), AddCheck(..) ] }   // 一个事务内按序执行
```

### 四·四 幂等性由引擎保证，而非靠约定

现状靠"人手写幂等"（`mod.rs:549-552` 硬约定 + 43/74 有测试）。新架构下**引擎自己保证**：

| 引擎行为 | 幂等方式 |
|---|---|
| 建表 | 先 `introspect` 判存在性，不存在才建（不依赖 `IF NOT EXISTS`） |
| 加列/删列/改类型 | 同上，全走"先比对后动作"，天然幂等 |
| L2 声明 | 引擎比对声明与实况，差异为空则不执行 |
| L3 规则 | **契约要求**：`fn(&DatabaseConnection) -> Result<bool>`，返回"是否发生变更"；规则内部必须状态驱动 |

对 L3 的硬要求：**规则的返回值决定是否需要重跑**。引擎在首次执行后把指纹写入 A；若某次规则返回 `true`（发生变更）但指纹已落盘，说明规则的状态判据有 bug —— 引擎记录 `WARN` 并计入审计。这是对"规则漏判状态"的自动检测。

### 四·五 方言类型映射

SeaORM 的 `ColumnType` 已内置方言分支（`ColumnType::Json` → PG `jsonb` / SQLite `text`），**优先用它**。

**禁令**：L1 里禁止出现 `column_type = "Custom(...)"` 形式的硬编码 SQL 类型 —— 那会同时写死两种方言。需要方言差异时，走 L2 的 `ColumnTypeOverride { dialect, sql }`。

自研引擎的渲染器只做一件事：把 `ColumnType` + 列属性渲染成双方言 DDL，并处理差异：

| 语义 | PG | SQLite |
|---|---|---|
| 自增主键 | `BIGSERIAL` / `GENERATED .. AS IDENTITY` | `INTEGER PRIMARY KEY AUTOINCREMENT` |
| 布尔 | `BOOLEAN` | `INTEGER`（0/1） |
| JSON | `JSONB` | `TEXT` |
| 时间戳 | `TIMESTAMPTZ` | `INTEGER`（epoch ms）或 `TEXT` |
| 二进制 | `BYTEA` | `BLOB` |

### 四·六 并发互斥

多个实例（桌面 App 多开 / 服务 + CLI）可能同时启动：

- **PG**：`SELECT pg_advisory_lock(<const>)` 包住整个 apply，结束 `pg_advisory_unlock`
- **SQLite**：`BEGIN IMMEDIATE` 抢写锁（SQLite 单写者，天然互斥）

拿不到锁的实例**不等待、不重试**，直接跳过 reconcile（另一次启动会完成）。

### 四·七 自动安全网（"引擎全权"的必要配套）

用户选择"无人工确认"，所以安全必须自动：

| 机制 | 作用 |
|---|---|
| **墓碑** `_ax_schema_graveyard` | 任何 `DROP TABLE / DROP COLUMN / 重建表` 前，把受影响对象的 DDL + 全量数据导出为 JSON 存入。**数据可人工找回**，不依赖备份 |
| **审计全文** `_ax_schema_audit.ddl_json` | 每次执行的全部语句落库，事后可精确复现 |
| **启动前快照** | 若本次变更含破坏性操作，先 `pg_dump` / 复制 SQLite 文件到 `backups/`（保留 N 份） |
| **逃生阀** `AX_SCHEMA_DRY_RUN=1` | 只打印 plan 不执行，默认关闭（不影响"全自动"） |
| **变更上限熔断** | 单次 apply 若涉及 >X 张表或 >Y 条语句，先落审计再执行；异常值（如全库表都被判孤儿）**拒绝执行并告警**，防止实体加载失败导致全库被清 |

最后一条是关键保险：**"实体集合为空"这类加载失败，在引擎全权下等价于"所有表都是孤儿"**。引擎必须校验「注册实体数 > 0 且 ≥ 上次记录值」，否则熔断。

### 四·七·补 正面判定：会不会丢数据

**结论：在"引擎全权 + 无延迟执行"下，会丢。四条路径，逐条给封堵。**

| # | 丢数据路径 | 触发条件 | 后果 | 封堵 | 封堵后 |
|---|---|---|---|---|---|
| 1 | **实体漏写** | 新增表未加实体（或实体文件未注册进 `lib.rs`） | 整表 + 全量数据 `DROP` | 墓碑（**导出成功才允许 DROP**）+ 基数熔断 + `OrphanWhitelist` | ⚠️ 降为"**可找回**"，**不等于"不丢"** ⇒ 必须叠加**延迟执行** |
| 2 | **`renamed_from` 漏写** | 列改名未在实体声明 | 旧列无实体认领 → 判孤儿 → `DROP COLUMN`，数据随列消失 | 引擎发现「库里旧列消失 + 实体新增列 + 无 `renamed_from`/`renamed_table` 归因」时 **拒绝执行该条**，其余照常 | ✅ **fail-closed ⇒ 不丢** |
| 3 | **类型收窄** | 实体把 `VARCHAR(255)` 改成 `VARCHAR(50)`、`BIGINT`→`INTEGER` | PG 侧 `ALTER TYPE` 直接报错（安全）；**SQLite 重建路径可能静默截断** | 重建路径前做长度/精度预检（`SELECT max(length(col))`），超限拒绝执行 | ✅ 可消除 |
| 4 | **SQLite 重建中断** | 「建新表 → 搬数据 → 换名」任一步失败 | 数据半途，旧表可能已不在 | 单事务 + `PRAGMA foreign_keys` 在事务**外**设 + `foreign_key_check` 在 `COMMIT` **前**（§3.1） | ✅ 可消除（事务原子） |

#### 「不丢数据」的三个必要条件（缺一不可）

**① 导出成功 → 才是删除的前置条件**，不是"异步备份"。
墓碑写入失败 ⇒ 整个 apply **中止并回滚**。「先删了，备份失败就算了」这种顺序一律不成立。

**② 破坏性操作延迟一个周期执行**（这一条是唯一能覆盖路径 1 的机制）。

```
本次启动：A ≠ B → 算出 plan → 把破坏性条目写入 _ax_schema_pending_drops(表名, 原因, 首次发现时间, cancelled)
下次启动（或距首次发现 > N 分钟）：复查这些对象仍未被认领 → 落墓碑 → 执行 DROP
```

为什么必须有它：**墓碑能找回"被删掉的数据"，但找不回"删除发生到人发现问题"之间的那个窗口。** 若引擎在启动瞬间删表、而人一周后才发现，这一周里所有指向该表的新写入都会静默失败或改道 —— 这部分不可逆。
写成 `cancelled = true` 即可取消，无需改代码。
**这不违反"引擎全权"** —— 全程无人干预，只是加了时间缓冲，代价是一次 schema 变更需两次启动生效。

**③ 引擎在"无法归因的删除"上必须 fail-closed**。三条熔断判据：

| 判据 | 触发 | 动作 |
|---|---|---|
| 注册实体数 < 上次记录值 | 实体文件漏注册 / `lib.rs` 改错 | 全部拒绝执行，只告警 |
| 单次 plan 的 DROP 对象数 > 库对象总数 × X% | 真相源加载失败被误解为"全库无主" | 拒绝执行该 plan |
| 库里"上次存在、本次无任何认领"的表 ≥ 1 且无归因 | 实体漏写 / 改名未声明 | **该条**降为 pending，其余照常 |

⇒ **加上三条之后，架构在数学上不丢数据**：残余仅剩「墓碑与快照同时写失败」这一场景，而该场景下引擎 **fail-closed 拒绝执行**（因为①要求导出成功是前置条件）。**代价是"绝对不丢"不来自任何单点机制，而来自"失败即停"的组合**。

#### 切换到新架构这件事本身会不会丢

> ⚠️ **本节结论已于 2026-09-16 二次修正**：① 初版写「除 5 张零访问表外不会」是错的；② 中间版据此改判为「17 张」—— **同样是错的**，它把「运行时自建的表」一律当成在主库，**漏了「追连接来源」这一步**。三次取证后的正确结论：**真实风险面是 7 张**（5 张零访问 + 2 张主库活跃表）。

**会丢 —— 若不处理这 7 张。**

| 批次 | 来源 | 张数 | 在主库？ | 数据重要性 | 正确处置 |
|---|---|---|---|---|---|
| **A 批** | §五·一：迁移建表但无实体 | 5 | ✅ 是（表存在） | 低（代码零读写） | 不补实体，交引擎清理（`opc_demand_lead` 已确认无数据） |
| **B 批** | §五·四：运行时自建**且在主库** | **2** | ✅ 是（`cron_jobs` / `cron_job_history`） | 🔴 **高（定时任务，活跃）** | **必须补 2 个实体 + 改造 `cron_job.rs`** |
| C 批 | §五·四：运行时自建**但不在主库** | 10 | ❌ **否** | — | **引擎看不见，无需处理**（6 张在 `index.db` 侧车库 / 4 张宿主零实例化 ⇒ 表不存在） |

⇒ **原判的 `RuntimeOwnedTable` 声明通道不再需要**：真正落在主库、需要纳入真相源的只有 2 张表，补实体即可，**不引入新的声明类**（完整论证见 §五·四）。

⇒ **P0b 必须做**：A 批 4 张表的 `SELECT count(*)`（`opc_demand_lead` 已确认无数据故免）；B 批 2 张在主库**必然有数据**（定时任务在跑），重点转为**核对实体列定义与 `cron_jobs` 现有 DDL 是否一致**。

⇒ 数值口径：已被实体覆盖的表 **169 张**；加 §五·一 那批**在库中的** 5 张、§五·四 的 2 张 ⇒ 库里实际对象总数 ≈ **176 张**（另有 6 张 FTS 虚表由 L2 认领）。**「169」不是全量** —— 这个数差正是孤儿判定的误判面。

### 四·八 性能

- **指纹短路**：绝大多数启动 `A == B`，只做一次 introspect + hash，不做 diff。
- **introspect 成本**：PG 用 `pg_catalog` 一次查全部（避免 N+1）；SQLite 用 `pragma` 逐表（表数 177，可接受）。
- **重活隔离**：SQLite 表重建会全表拷贝（`notes` 实测 48590 行），PG 生成列重建同理。这类操作**异步执行 + 进度落审计**，不阻塞主窗口启动。

### 四·九 孤儿对象判定（引擎全权的核心风险点）

原则：**库里存在、实体与 L2 声明都没认领的对象 → 判为孤儿 → DROP。**

实测数据（本轮量化，见 §九）：

| 类别 | 数量 | 处置 |
|---|---|---|
| 有 DDL 也有实体 | 165 → **169** | 正常对账 |
| **有 DDL 无实体（实表）** | **12** → **5** | ⚠️ **会被 DROP** —— 12 张已逐张裁决（§五·一）：4 张补实体、3 张库里本就不存在、**5 张判定为不补实体 ⇒ 引擎上线后自动清理** |
| **有 DDL 无实体（FTS 虚表）** | **6** | 由 L2 `VirtualTableDecl` 认领 |
| 有实体无 DDL | 4 | 结构靠实体派生（正常） |

**必须提供白名单**：`OrphanWhitelist`（表名 + 过期时间 + 原因），给"暂时不想建实体但要保住表"的场景留活口，否则每次启动都在删同一张表。

#### ⚠️⚠️ 孤儿判定的最大盲区：**运行时自建表**（2026-09-16 实测发现）

**建表在本仓有 4 条独立路径，实体只挂在其中一条上：**

| # | 路径 | 位置 | 实测数量 | 与实体的关系 |
|---|---|---|---|---|
| ① | 手写迁移 | `dao/src/migrations/` | 177 条 `CREATE TABLE` | 弱（写完就忘） |
| ② | **业务代码运行时自建** | `runtime-core/src/cron_job.rs:38`、`disk-cache/src/lib.rs:88`、`search/src/ast_index.rs:85`、`tools/src/audit.rs:93`、`dao/src/task_ledger.rs:240` 等 | **27 处 / 15 个文件** | **无任何关联** |
| ③ | 动态表名自建 | `search/src/vector_store.rs:235/246/260` | 3 处（`{name}`） | 无任何关联 |
| ④ | SeaORM 实体派生 | 能力存在（`SchemaBuilder`），**`schema-sync` feature 未开 ⇒ 从未启用** | 0 | 唯一正相关 |

⇒ **走 ②③ 建的表有没有实体，纯取决于作者顺手写没写。**

**初步统计：12 张活跃表无实体**（脚本 `output/tmp-ct-audit.mjs`）—— `cron_jobs` / `cron_job_history`（定时任务）、`l2_*`（L2 缓存）、`ast_*` + `file_index`（索引）、`audit_log`（审计）。

#### ⚠️ 但这 12 张不能一起判 —— 必须再走一步「追连接来源」

初版本节由此断言「引擎上线会把这 12 张全当孤儿 DROP，每次启动清空一次」。**该论断有误**，它漏了决定性的一步：**追这些表建在哪个数据库连接上**。

实测（详见 §五·四 的逐表判定表 + 脚本 `output/tmp-wiring-audit.mjs` → `tmp-wiring-audit.log`）：

| 宿主类型 | 数量 | 连接 | 宿主的实例化情况 | 是否在主库 |
|---|---|---|---|---|
| **sea-orm `DatabaseConnection`** | **2** | 主库 | ✅ `src/init/state.rs:523`（启动时真实构造） | ✅ **是** |
| **rusqlite 指向 `index_dir/index.db`** | 6 | 独立文件（`indexing_triggers.rs:109`） | ✅ `indexing_triggers.rs:122/161` | ❌ 否 |
| **rusqlite，宿主类型零实例化** | 4 | — | ❌ 无生产构造点（`l2_*` 只在自身测试；`audit_log` 的 `audit_db_path` 全仓恒 `None`；`divergence_logs` 只在自身测试） | ❌ 否（**表从未建成**） |

⇒ **真正会被引擎误判的只有 2 张**（`cron_jobs` / `cron_job_history`）。其余 10 张**不在主库 schema 内**，引擎根本看不到。

```
引擎启动 → 只对「经主库连接可达的对象」做对账
         ├─ cron_jobs / cron_job_history  → 会判孤儿 ⚠️ 必须补实体
         └─ index.db 的 6 张 / 未实例化的 4 张 → 不可达，不参与判定 ✅
```

**为什么这些模块选择自建表（不是偷懒，是有意为之）**：
`cron_job.rs:27-33` 是活证据 —— 它记录了 2026-09-13 的修复：

> 建表语句本身 PG 也能跑（`TEXT` 通用），但**此前后续所有读写**都硬编码 `DbBackend::Sqlite` + SQLite 方言 SQL（`INSERT OR REPLACE` / `json_extract` / `?` 占位符），在 PostgreSQL 上必然报错；而这些错误又全被 `let _ =` 吞掉 —— 于是 `cron_jobs` 长期 0 行、用户在设置里建的定时任务只活在内存里、重启即消失。

⇒ 对模块作者而言，「我自己保证我的表存在」比「指望跨 crate 的迁移体系记得建我的表」更可靠。

#### 架构结论：铁律的管辖范围 = 主库

**「实体 = 唯一建表与访问形式」这条铁律，在「主库」范围内成立且可实现**；对**侧车库**（`index.db` 等独立 SQLite 文件）则应显式排除，理由是**技术不能 + 架构不该**：

1. **技术不能**：实体（`DeriveEntityModel`）绑定主库的 `DatabaseConnection`；侧车库是另一条 `rusqlite::Connection` 指向的**另一个物理文件**——实体机制在语法上就无法管辖它（不是"不想"，是"不能"）。
2. **架构不该**：侧车库装的是**可重建的本地索引**（FTS/代码索引），主库装的是**不可重建的业务真相**。把索引搬进 PG 会牺牲性能且无收益。
3. **判据**：**「多个入口写同一个库」才是问题，「一个入口写自己的库」不是问题。** 侧车库现状（一个 crate 拥有一个库 + 一个 `ensure_table()` 入口 + 全部访问经该 crate API）**已合规**。

> **侧车库合规三条件**（必须写成明文约束，否则边界就是模糊的）：
> ① 该库由**唯一一个** crate 拥有；② 建表只经**该 crate 的 `ensure_table()` 单一入口**；③ 其它 crate **不得**越过该 crate 的 API 直接用裸 SQL 访问它的表。

而 `cron_jobs` 的现状恰恰是**违规**的：它在**主库**里，却用裸 SQL `execute_unprepared` 建表（`cron_job.rs:37`）+ `DbBackend` 分支拼 SQL 访问，**完全绕过实体体系** —— 这正是铁律要消灭的形态（处置见 §五·四）。

⚠ **孤儿判定的输入必须是「真实库 introspect 结果」，且仅限主库可达对象。**

实证（§五·一 A 组）：`trajectory_entities` / `trajectory_memories` / `trajectory_relationships` 由 v100 建、被 v101 PHASE 9 DROP，**库中已不存在**，但**迁移文件里的建表 DDL 文本永远在**。若按文本判定，引擎会把它们当成"期望侧要求存在的表"而**试图重建** —— 直接错。

⇒ **判据**：期望侧取**实体元数据 + L2 声明**（编译期已知）；实际侧取 **`information_schema` / `pg_catalog`**（运行期实测）。**两边的输入都不是"迁移文本"** —— 迁移文本在目标架构里只是一个待删除的历史包袱，不参与任何判定。

---

## 五、⚠️ 生死线：三个前置条件（做不到就不能开工）

### 五·一 实体覆盖率补齐：12 张表会被自动删除 → **✅ 已定案并落地（2026-09-16）**

对全仓 **1985 个 `.rs` 文件**逐个扫描这 12 张表的读写路径，逐张定性。**结论：5 张删表、4 张补实体、3 张早已被淘汰。**

| 表 | 建表迁移 | 程序访问方式（实测） | 处置 | 实体覆盖状态 |
|---|---|---|---|---|
| `chat_runs` | v207 | **零访问** | **不补实体** | ⬜ 待引擎清理（或运维手工 DROP） |
| `chat_run_events` | v207 | **零访问** | **不补实体** | ⬜ 待引擎清理 |
| `rl_policies` | v100 | **零访问** | **不补实体** | ⬜ 待引擎清理 |
| `opc_demand_lead`（单数） | v221 | **零访问**（B 链路已废弃） | **不补实体** | ⬜ 待引擎清理 / 运维手工 DROP |
| `opc_market_platform` | v221 | **零访问**（B 链路已废弃） | **不补实体** | ⬜ 待引擎清理 |
| `loop_checkpoints` | v100 | 原生 SQL（`dao/repo/loop_checkpoint.rs`，双方言双分支） | **补实体 + 改造** | ✅ **已完成** |
| `evolution_execution_stats` | v122 | 原生 SQL（`dao/repo/evolution_execution_stats.rs`） | **补实体 + 改造** | ✅ **已完成** |
| `wiki_graph_cache` | v103 | 原生 SQL（`dao/repo/wiki_graph_cache.rs`） | **补实体 + 改造** | ✅ **已完成** |
| `semantic_cache` | v100 + 运行时自建 | 原生 SQL（`src/semantic_cache/mod.rs`） | **补实体 + 改造** | ✅ **已完成** |
| `trajectory_entities` | v100 | —— 已被 `v101` PHASE 9 DROP | 无需动作 | ✅ 库里已不存在 |
| `trajectory_memories` | v100 | —— 已被 `v101` PHASE 9 DROP | 无需动作 | ✅ 库里已不存在 |
| `trajectory_relationships` | v100 | —— 已被 `v101` PHASE 9 DROP | 无需动作 | ✅ 库里已不存在 |

> **⚠ 为什么前 5 张表「不补实体」就等于「被删」，且不需要写 `v234` 之类的删表迁移**
>
> 在目标架构下，「续用什么、废弃什么」的唯一表达方式就是**实体是否存在**：
> 补实体 ⇒ 表受保护；不补 ⇒ 引擎判为孤儿并清理。**这件事不需要人手写任何东西。**
>
> 本轮一度新增了一条 `v234_drop_orphan_tables` 迁移来删这 5 张表，随后**已撤回**（文件与注册均已回滚）。理由：
> 1. **它与本次改造的目标直接相悖** —— 目标是「不再新增手写迁移文件」，而删表恰恰是目标引擎**必然能自动完成**的动作（§四·九）。用新写的一条迁移去删表，等于给待淘汰的机制续命。
> 2. **它违背项目自己已写下的约定** —— `commands/demand_discovery.rs:21-22` 原文：「线上孤儿表由**运维侧手动 DROP，不走迁移**」。本轮的做法正好相反。
> 3. 这 5 张表要么零写入（`chat_runs` / `chat_run_events` / `rl_policies`），要么已废弃且无数据（`opc_demand_lead` / `opc_market_platform`），**留着不产生任何功能影响**，不需要抢在引擎上线前删除。
>
> ⇒ 正确的落地方式是二选一：**(a)** 等引擎上线后由孤儿清理自动处理（顺带成为引擎的第一个真实验收用例）；**(b)** 需要立刻清爽的话，走运维手工 `DROP TABLE IF EXISTS`（一次性动作，不进代码库）。
>
> 这条也反过来证明了一件事：**P0a 的产出不是"代码改动"，而是"实体侧的决定"** —— 12 张表的处置里，真正需要落代码的只有 4 张补实体；另外 5 张的"删"是**什么都不做**。

#### ① 零访问 → 不补实体，交由引擎清理（5 张）

- **`chat_runs` / `chat_run_events`**：`v207_chat_run.rs:46-48` 承诺「开启 `persist_runs: true` 后 RunStore 会同时写入内存和数据库（双写）」「历史查询通过 `list_persisted_runs` / `get_persisted_run_events` 从数据库读取」，但全仓 grep 这三个名字（`persist_runs` / `list_persisted_runs` / `get_persisted_run_events`）**仅命中该注释自身，零实现**。实际实现（`crates/gateway/src/handlers/runs.rs:103-109`）是 `RunStore { runs: Mutex<HashMap<String, ChatRun>>, event_txs: ... }` —— 纯进程内存储，结构体里连 DB 句柄都没有。
  ⇒ **持久化从未实现，这两张表自创建起零写入。** ⚠ 若将来要做多实例网关的 run 持久化，需**重新建表 + 同步补实体**，不能只建表（否则又回到本轮修掉的"孤儿表"状态）。
- **`rl_policies`**：全仓仅 v100 的建表语句 + 2 个索引，**零读写**。
  ⚠ **易误判点**：`crates/orchestrator/src/capability_pack_learning.rs:169` 的 `RLPolicyUpdate` 是同名不同物的**纯 DTO**（不触碰该表），按名字搜会误判成"有访问"。
- **`opc_demand_lead`（单数） / `opc_market_platform`**：`v221_demand_discovery.rs` 引入的 B 链路。`commands/demand_discovery.rs:7-25` 头部注释写明已收敛为 A 链路薄壳：「已删除的 B 链路命令：…（连同废弃实体 `opc_demand_lead` / `opc_market_platform`；**线上孤儿表由运维侧手动 DROP，不走迁移**）」。
  ⇒ 本轮的处置**遵循这条既有约定**：不写删表迁移。用户已确认单数表内无数据。
  ⚠ 与 A 链路复数表 `opc_demand_leads` 仅差一个字母，误删会损伤真实需求数据 —— `opc_demand_leads`（复数）才是活跃表，**有**实体（`entities/src/opc_demand_leads.rs`）。

#### ② 有真实访问但走原生 SQL → 补实体 + 改造（4 张）

四张表的共同形态：**持久化模块用 `Statement::from_sql_and_values` 手写 SQLite(`?N`) / PostgreSQL(`$N`) 双分支**（PG 会把 `?1` 解析成 `?` 操作符 + integer，报 "operator does not exist: ? integer"）。补实体后方言由 sea-query 处理，双分支全部消失。

改造中处理的方言/语义差异（逐条已核）：

| 表 | 差异 | 处置 |
|---|---|---|
| `loop_checkpoints` | SQLite 用 `INSERT OR REPLACE` / PG 用 `ON CONFLICT DO UPDATE` | 统一 UPSERT；**语义相同**（都覆盖全部业务列），等价 |
| `evolution_execution_stats` | PG 写表限定名 `evolution_execution_stats.usage_count + 1`，SQLite 写裸列名 | 统一裸列名（PG 在 `DO UPDATE` 的 SET 表达式中同样接受裸列名指向现有行），等价 |
| `wiki_graph_cache` | PG 用 `ON CONFLICT`；**SQLite 用「先 UPDATE，`rows_affected()==0` 再 INSERT」两步** | 统一为单条 UPSERT。**顺带修掉一处竞态**：原 SQLite 两步在并发下会主键冲突 |
| `semantic_cache` | **建表 DDL 一表两定义**（v100 写 `INTEGER NOT NULL DEFAULT 0`；运行时自建写 `BIGINT` 且无 NOT NULL） | 统一为 v100 那一份（应用总是先跑迁移 ⇒ 生产库实际形状一直是它） |
| `semantic_cache` | SQLite 用 `INSERT OR REPLACE`（整行替换 ⇒ **清零 `hit_count`**）；PG 用 `ON CONFLICT DO UPDATE`（保留） | ⚠ **行为变更**：统一为保留 `hit_count`。判定依据见下 |
| `semantic_cache` | `model_id IS NOT DISTINCT FROM $2`（要求 SQLite ≥ 3.39） | 按 `model_id` 有无分支为 `= ?` / `IS NULL`，等价且更可移植 |

**⚠ 唯一的行为变更是 `semantic_cache` 的 UPSERT（仅影响 SQLite）**：原 `INSERT OR REPLACE` 是整行替换，会因"条目内容被重写"而把命中统计 `hit_count` 清零；PG 分支不重置。判据：`hit_count` 是缓存命中统计，不因内容刷新而清零才合理 ⇒ 取 PG 语义。该表无外键引用，`INSERT OR REPLACE` 的级联删除副作用在此不存在。已补测试 `upsert_preserves_hit_count` 锁定。

#### ③ 早已淘汰 → 无需动作（3 张）

`trajectory_entities` / `trajectory_memories` / `trajectory_relationships`：v100 创建，**v101 PHASE 9 已 DROP 并迁走数据**到 `knowledge_entities` / `knowledge_relations` / `memory_items`。`dao/tests/pg_migrations.rs:239-241` 有断言锁定其不存在（⚠ 该测试文件已于 2026-09-16 随 74 个迁移一起退休，且**当时未备份**；下方引用的断言原文，因此是它此刻**唯一的存档** —— 见 §十一·十一⑨ 的流程失误登记）：

```rust
for t in ["trajectory_entities", "trajectory_relationships", "trajectory_memories"] {
    assert!(!table_exists(&db, t).await, "table {t} should have been DROPPED by v101 PHASE 9");
}
```

#### 剩余 8 张的完整处置（12 − 4 张补实体）

| 组 | 表 | 库里是否存在 | 处置 |
|---|---|---|---|
| **A（3 张）** | `trajectory_entities` / `trajectory_memories` / `trajectory_relationships` | ❌ **不存在** | **无需任何动作** |
| **B（5 张）** | `chat_runs` / `chat_run_events` / `rl_policies` / `opc_demand_lead` / `opc_market_platform` | ✅ 存在 | **不补实体**（该决定本身即处置） |

**A 组是「静态统计的假阳性」**：它们被算进「12 张无实体表」，只因统计脚本扫的是**迁移文件的 DDL 文本** —— v100 建了、v101 PHASE 9 又 DROP 了，而**迁移文件不删、建表文本就永远在**。库里实际没有这 3 张表（原 `dao/tests/pg_migrations.rs:239-241` 有断言锁定；该文件已于 2026-09-16 退休且**未备份** ⇒ 断言原文只留在本节上方 :508-512 的引用里）。
⇒ 引擎 introspect 真实库时看不到它们，**无需处理**。
⇒ **由此得出一条设计约束（已并入 §四·九）**：孤儿判定必须 introspect 真实库，**不能扫迁移文本**，否则会试图"重建"这 3 张表。

**B 组的「处置」就是「不补实体」，该决定已完成。** 目标架构下「续用 / 废弃」的唯一表达方式就是**实体是否存在**：补 ⇒ 受保护；不补 ⇒ 引擎判孤儿 ⇒ 自动 DROP。**所以正确动作是「什么都不做」** —— 不写迁移、不写手工 SQL。
引擎上线前它们的影响：`chat_runs` / `chat_run_events` / `rl_policies` 零写入；`opc_demand_lead` / `opc_market_platform` 已废弃且无数据 ⇒ **无功能影响**。

**⚠ 两件必须做的事，都落在「引擎侧」而非「删表侧」**：

1. **孤儿清理必须有豁免机制**：`chat_runs` / `chat_run_events` 是**未来功能预留**（v207 注释写明用途是「多实例网关共享 run 状态」）。将来真做该功能时需**重新建表 + 补实体**。豁免机制用于表达「暂时无实体但有意保留」这一状态 —— 本轮判定为删，故这 5 张**不进豁免名单**，但**机制本身必须在**：否则「故意不写实体」与「忘了写实体」在引擎眼里完全一样（这正是 §八 风险 1「实体漏写 = 静默删表」的另一面）。
2. **这 5 张 = 引擎孤儿清理的第一批验收对象**：一个真实存在、答案已知的场景，用于验证引擎不会乱删。

**⚠ 过渡期现象（非问题，但需知情）**：引擎上线后、P6 删除 `migrations/` 之前，**新装库**会经历「v207/v221 建表 → 引擎判孤儿 → DROP」的"建了又删"。成本 = 两个空表的 CREATE + DROP，P6 后消失。

#### 落地清单（本轮实际改动）

| 文件 | 动作 |
|---|---|
| `src-tauri/crates/entities/src/{loop_checkpoints,evolution_execution_stats,wiki_graph_cache,semantic_cache}.rs` | **新增 4 个实体** |
| `src-tauri/crates/entities/src/lib.rs` | 注册 4 个 `pub mod` |
| `src-tauri/crates/dao/src/repo/loop_checkpoint.rs` | 改造为 Entity API（消除双方言分支） |
| `src-tauri/crates/dao/src/repo/evolution_execution_stats.rs` | 同上 |
| `src-tauri/crates/dao/src/repo/wiki_graph_cache.rs` | 同上 + 修竞态 |
| `src-tauri/src/semantic_cache/mod.rs` | 同上 + 统一建表 DDL + 补 3 个回归测试 |
| ~~`src-tauri/crates/dao/src/migrations/v234_drop_orphan_tables.rs`~~ | ❌ **已撤回**（见上「为什么不需要写删表迁移」） |
| ~~`src-tauri/crates/dao/src/migrations/mod.rs`~~ | ❌ **已回滚**（`CURRENT_VERSION` 保持 `233`，迁移数组未追加） |

⇒ **本轮没有新增任何迁移文件，`CURRENT_VERSION` 保持 233。** 5 张零访问表按「不补实体 ⇒ 引擎判孤儿 ⇒ 自动清理」的路径处理（§四·九），这也是引擎上线后的第一批真实验收对象。

**验证状态（2026-09-16）**：

| 项 | 方法 | 结果 |
|---|---|---|
| 列核对 | `output/tmp-verify-entities.mjs`：实体 `Model` 字段 ↔ DDL 列（名称/顺序/可空性/主键/`auto_increment=false` 数量） | ✅ 4/4 全通过 |
| import 核对 | `output/tmp-verify-imports2.mjs`：逐符号普查 + trait 通过方法调用隐式使用的判定 | ✅ 无 unused import（9 个 trait 逐个给出方法调用依据） |
| API 签名核对 | 逐条对照 `sea-query-1.0.2` / `sea-orm-2.0.2` 源码：`OnConflict::{columns,values,value,update_columns}`、`PaginatorTrait::count`、`ColumnTrait::in_subquery`、`ExprTrait::{add,gt,sum}`、`SimpleExpr = Expr` 别名 | ✅ 全部匹配 |
| **编译** | `cargo check -p axagent-entities -p axagent-dao -p axagent --all-targets` | ⏸ **未闭环** —— 全程 `Blocking waiting for file lock`（另有 3 组并发 cargo，持锁者非本会话，按纪律不抢锁）⇒ **本轮改动尚未经编译器确认** |

⇒ **§五·一 这个前置条件的「实体侧决定」部分已全部完成。** 另两个前置条件均已落地：§五·二 索引注解（**实测 198 个已落地**）、§五·三 L2 例外清单（**已冻结并落地为代码，`extras.rs` 68 条** —— 原「静态实测 72 项」是另一口径，见 §五·三）。

### 五·二 索引注解补齐：**✅ 已落地 198 个（2026-09-16）**

**本轮实测（可复现）** —— 脚本 `output/tmp-p1-index-inventory2.mjs` → `output/tmp-p1-inv2.json`：

| 指标 | 实测值 |
|---|---|
| 扫描迁移文件 | 74（`v100`–`v233`） |
| 解析到的 `CREATE INDEX` 字面量 | 269（去重后 **265**） |
| 单列非唯一 → 需 `#[sea_orm(indexed)]` | 204（按 `(表, 列)` 去重后 **198**） |
| 多列 / partial / unique → 进 L2 | 39 |
| 建在**无实体表**上的索引 | 26（见下） |
| 解析失败（已逐条定性） | 10（6 条 `USING GIN` + 4 条动态表名 `{}`） |

**已落地**：198 个 `#[sea_orm(indexed)]`，涉及 **103 个实体文件**（187 处新增属性行 + 11 处追加到已有 `#[sea_orm(...)]`）。取证：

| 核对项 | 结果 |
|---|---|
| 落地前实体侧 `indexed` 注解总数 | **0**（全 crate 仅 3 处无关命中：`default_value = "indexed"` 字符串 / 列名 `last_indexed_at` / 注释） |
| 落地后独立 grep 复扫 | 198 处，涉及 103 文件 |
| 随机抽 6 条回迁移原文核对 | **6/6 命中**，表名与列名逐字一致 |
| `cargo fmt -p axagent-entities --check` | `EXIT=0`，零 diff |

> ⚠ **`cargo fmt --check` 通过不能证明语法正确** —— 本轮实证：13 处 `to = "..."` 缺尾逗号的**语法错误**在 `fmt --check` 下报 `EXIT=0` 零 diff（rustfmt 对解析失败的文件静默跳过）。最终确认靠 `cargo check -p axagent-entities --all-targets`。
>
> ⇒ 本项已落为判据（见 `MEMORY-RULES.md` 同批次号）。

**⚠ 与本节旧值（162 个 / 179 条）的差异未归因** —— 旧值无法用当前脚本复现，且已排除「按 `idx_` 前缀统计」这一可能（实测 264/265 都是 `idx_` 前缀）。旧值当时的统计口径未记录在案，**不编归因**；以本轮可复现脚本为准。

**26 条建在无实体表上的索引**（逐表计数）：`opc_demand_lead`(8)、`trajectory_memories`(5)、`chat_runs`(3)、`trajectory_entities`(2)、`trajectory_relationships`(2)、`rl_policies`(2)、`gateway_message_queue`(2)、`chat_run_events`(1)、`opc_market_platform`(1)。这批正是 P0b 待定性的孤儿表，其索引**不进 L1 注解**（无实体可挂），处置随这些表本身一并裁决。

### 五·二·补 FK / `on_delete` 核对：**✅ 已落地 27 条（2026-09-16）**

**判决依据（三方对账，缺一不可）**：迁移声明 × 实体注解 × **真实 PG 库 `pg_constraint`**。只做前两方对账会得出错误的处置方向。

| 指标 | 实测值 |
|---|---|
| 迁移 DDL 的 FK 声明 | **30** ← 修正，见下方假阴性 |
| **真实 PG 库 FK 约束数** | **30**（29 张表，`retrieval_hits` 占 2） |
| 两者关系 | **逐条吻合，零 DROP**（迁移说的 FK 全部真在库里） |
| 实体 `belongs_to` 总数（原） | 29 |
| 两侧一致（已有 `on_delete` 且值相同） | 3 |
| **实体缺 `on_delete`、迁移明确声明** | **13** → 已补 `on_delete = "Cascade"` |
| 两侧值冲突 | **0** |
| **迁移有 FK、实体无 `belongs_to`** | **14 → 已全部补进实体**（含 2 条 `SetNull`） |
| 实体 `belongs_to` 总数（现） | **43** = 29 + 14 |
| **实体有 `belongs_to`、迁移与 PG 都没有** | **14（已裁决「接受」，P4 将新增 FK，见下）** |

**① 只补实体侧，迁移侧一个字不动** —— 迁移是即将删除的对象，往它上面补是反方向。14 条「迁移有、实体无」的处置：直接在实体补 `belongs_to` + `on_delete`。因 PG 实测这 14 条**全部真实存在**，补完后实体派生的 FK 与库现状**逐条等价** ⇒ P4 替换时零 DDL 变更，纯增益。

已补清单（13 文件 / 14 条）：`search_citations` `tool_executions` `retrieval_hits`×2 `artifacts` `context_sources` `conversation_branches` `stored_files`(**SetNull**) `agent_sessions` `wiki_sources` `wiki_pages` `wiki_operations` `wiki_sync_queue` → `conversations`/`wikis`/`knowledge_bases`；`agent_profiles` → `agency_experts`(**SetNull**)。

**② 余 14 条「实体有、迁移与 PG 都没有」= 已裁决「接受」（2026-09-16 用户裁决）**

清单（14 条 / **12 表** —— `knowledge_attributes` 与 `note_links` 各占 2 条）：`gateway_link_activities.link_id` `gateway_link_policies.link_id`（→`gateway_links`，**无 on_delete**）；`knowledge_attributes.knowledge_base_id` `knowledge_attributes.entity_id` `knowledge_entities.knowledge_base_id` `knowledge_flows.knowledge_base_id` `knowledge_interfaces.knowledge_base_id` `knowledge_relations.knowledge_base_id`（→`knowledge_bases`/`knowledge_entities`）；`note_backlinks.target_note_id` `note_links.source_note_id` `note_links.target_note_id` `wiki_pages.note_id`（→`notes`，**4 条**）；`reading_list_items.reading_list_id`（→`reading_lists`）；`workflow_marketplace_reviews.marketplace_id`（→`workflow_marketplace`，**无 on_delete**）。

裁决：**接受** —— 视为「实体真相源纠正了迁移时代的遗漏」，P4 时按实体建出 FK；**不加 `skip_fk`**。

> ⚠ 影响面实测（P0b 精确 `COUNT(*)`）：其中 **4 条指向 `notes`**，**3 条所在表有真实数据** —— `note_backlinks` **74791 行**、`note_links` **74791 行**（两条 FK 均在其上）、`notes` 本身 **48590 行** ⇒ 新增 FK 会带来写入时校验开销 + 首次 `VALIDATE` 全表扫描。第 4 条 `wiki_pages.note_id` 所在表 **0 行**，无影响面。

#### 「接受」的可执行性实测（`output/tmp-p1-fk-orphan.mjs`）

接受的语义是「P4 会把这 14 条建出来」⇒ 必须先回答「**PG 存量数据建得出来吗**」。判据：`ADD CONSTRAINT` 只对**非 NULL** 值要求引用完整性 ⇒ 孤儿率 = 非 NULL 且父行不存在的行数 / 非 NULL 行数。

| 结论 | 条数 | 说明 |
|---|---|---|
| **可建（孤儿 = 0）** | **13** | **9 条表为空**（0/0）；**4 条有真数据**且引用完整（`knowledge_relations` 112937、`note_backlinks` 74791、`note_links` 74791×2，全命中） |
| **⚠ 不可建（孤儿 > 0）** | **1** | `knowledge_entities.knowledge_base_id` —— **51277 / 72816 = 70.4% 孤儿** |

⚠ 比较一律 `::text` 投射：迁移时代无 FK，列类型两代可能不一致，直接 `c.col = p.id` 会报 `operator does not exist` 并中断整批检查。实测 14 条 child/parent 类型**全部一致**（text→text），未触发该分支。

#### 第 14 条的真根因：`merge_entities` 把**哨兵值**写进了外键列

`crates/dao/src/repo/knowledge_graph.rs:1209-1214`（实体合并作业的软删除）：

```rust
// 标记被合并实体为已合并（或将其 kb_id 改为主实体的 kb_id）
// 这里采用软删除策略：将 kb_id 设为空字符串或特殊标记
let mut del_am: knowledge_entities::ActiveModel = (*entity).clone().into();
del_am.knowledge_base_id = Set(String::from("__merged__"));
del_am.updated_at = Set(now);
del_am.update(&txn).await?;
```

实测证据（`output/tmp-p1-kb-orphan-drill.mjs`）：

- 父表 `knowledge_bases` **只有 3 行**：`__sys_trajectory__` / `f9b2b050-…` / `lemonhu_knowledge_graph`
- 子表 `knowledge_entities` 72816 行，`knowledge_base_id` 只有 **3 个取值**：`__merged__` **51277**、`lemonhu_knowledge_graph` 21535、`f9b2b050-…` 4 ⇒ **`__merged__` 不是任何知识库的 id，是哨兵值**
- 同族对照：`knowledge_relations` 的 `knowledge_base_id` **无 `__merged__`**（distinct 2、孤儿 0）⇒ 污染**只发生在 entities**

⇒ 三层问题（**均未擅改，待裁决**）：

1. **哨兵值塞进外键列** —— 该列一旦建 FK 立即违反引用完整性。「已合并」是**行状态**，不是「属于哪个库」；表里**已有 `lifecycle` jsonb 列且当前全为 null**，语义位置本就在那里。
2. **软删除在读端零过滤** —— 全仓 `__merged__` 字面量**只有 1 处命中**（即上面这个写入端）。所有读路径（`knowledge_graph.rs:259/492/565/581`、`trajectory/src/storage.rs:859`、`analysis-engine/src/knowledge_loader.rs:65`）都**不排除**它 ⇒ **已合并实体仍出现在列表/搜索/图查询里**，而 `mention_count` 已累加到主实体 ⇒ **重复计数**（采样实证：同名「赵健」3 行、「恒基达鑫」2 行）。
3. **同表两列时间单位不一致** —— 采样行 `created_at = "1786277831750"`（**毫秒**）而 `updated_at = 1789421929`（**秒**）。

**已实施 A（2026-09-16 用户裁决）** —— 数据修复 + 代码修复均已落地，验收全绿。

#### 勘察（5 轮只读；`output/tmp-p1-merged-repair-survey{,2,3,4}.mjs`、`tmp-p1-merge-repair-dryrun.mjs`）

方案的每个输入都必须实测，否则一条 UPDATE 会写坏 5 万行：

| 问题 | 实测结论 |
|---|---|
| 组形态 | 主流 **3 行/组**（1 活行 + 2 副本），共 **22612 组** |
| 副本 id 形态 | `lemonhu_knowledge_graph_<hash>` 24271 / `f9b2b050-…_<hash>` 24270 / **裸 hash 2736** |
| 裸 hash 属哪个库 | 活行里裸 hash **100% 属 `lemonhu_knowledge_graph`**（21535 行，另 3 行属 `f9b2b050`）；`source_path` 全为 lemonhu 的 CSV（`raw/executive.csv` 等）⇒ 判定归 lemonhu |
| **1073 组整组皆被标 merged** | 主实体丢失 ⇒ 简单按 kb 过滤会让这些概念**从库里消失**（「张杰」「王伟」「李刚」等 person，最多 33 行/组）⇒ 必须**复活**每组唯一幸存者 |
| 4 行「形态异常」 | id 是 **A 股代码**（`600827` 百联股份 / `600631` 百联股份 / `600637` 东方明珠 / `600832` 东方明珠），非 hash 也非前缀 |
| 唯一约束 | 只有 `pk(id)`；`name` / `entity_type` **无 NULL** ⇒ 分组与 join 安全 |
| `lifecycle` 列 | `jsonb`、可空、无默认 —— 本就是为行状态准备的，此前**全为 null** |
| 关系端点 | **5971 条源端 + 66 条目标端**指向被合并行 ⇒ 不重定向就会变**断边** |
| 关系表 kb 列 | **无 `__merged__`**（112937 行全在 bases 内）⇒ 该列无需处理（但 40% 与源端实体 kb 不一致，属独立既存问题） |
| **PG 侧权威** | `knowledge_entities` / `knowledge_relations` 上**没有任何 FK 约束**（`pg_constraint` 查询为空） |

#### 数据修复（`output/tmp-p1-merge-repair-apply.mjs`：先备份 → 事务 → 复查）

备份放**独立 schema**（`ax_backup`），避免污染「public 无实体表」盘点：
`ax_backup.knowledge_entities_20260916`（72816）/ `ax_backup.knowledge_relations_20260916`（112937）。

| 步骤 | 影响行数 |
|---|---|
| 实体 UPDATE（kb 还原 + 写 `lifecycle={merged_into, merged_at_ms, group_mention_sum}` + 复活 1073 幸存者） | **51277** |
| 关系 source 端重定向 | 6019 |
| 关系 target 端重定向 | 66 |
| 补丁：边端改以 `lifecycle->>'merged_into'` 为真相源 | 48 —— 根因见判据 **#339** |

**验收（全绿）**：活跃行 **22612**（= 组数）· 已合并 **50204** · kb 孤儿 **0** · 每组活跃行恒为 1 · 关系断边 **0** · 指向已合并实体 **0** · 自环 **0** · 补丁幂等自证再跑匹配 **0 行** · kb 分布 48542 / 24274 与 dry-run 预测**完全一致**。

#### 代码修复（`output/tmp-p1-lifecycle-code-edit.mjs`：10 处，逐处断言命中 1 次）

`crates/dao/src/repo/knowledge_graph.rs` 8 处 + `crates/trajectory/src/storage.rs` 2 处：

- **merger**：读端加 `lifecycle IS NULL`（幂等）；`mention`/`aliases`/`description` 改**累加器**（原实现每轮都用循环外快照 ⇒ 后轮覆盖前轮，实测活跃行 mention 常为 1 而组内总和 11，`confidence` 由 mention 推导 ⇒ 一并算错）；软删除从「改写外键列」改为「写 `lifecycle`」
- **读端过滤**：`list_entities` / 评分搜索 / 按 kb 列实体 / 按名搜索 / 邻居扩展 / **写入端同名探测**（最后这条最要紧 —— 见判据 **#340**，漏了它会静默丢数据）
- **有意不改**：`find_by_id`（按 id 精确取，管理用途）；`analysis-engine/src/knowledge_loader.rs:65`（其 id 源 = 关系表端点，已收敛到活跃实体；再加过滤会把「未来关系表又指向 merged」这类回归**静默变成少节点**，反而不如让它显式暴露）

> 本次修复把「实体有 `belongs_to`、库无 FK」的 14 条中最后 1 条的**可执行性**补齐：`knowledge_entities.knowledge_base_id` 的孤儿率 **70.4% → 0** ⇒ P4 这 14 条全部可建。

> **源码事实**（`sea-orm-2.0.2` 实读，决定「补 belongs_to 是否等于补 FK」）：
> - `sea-orm-2.0.2/src/schema/entity.rs:227-233` —— `create_table_from_entity` **遍历 Relation 生成 FK**，`if relation.is_owner || relation.skip_fk { continue }`
> - `sea-orm-2.0.2/src/entity/active_model.rs:690/717/738/762/800/816` —— `is_owner == true` 时报 *"is not belongs_to"* ⇒ **`belongs_to` 的 `is_owner == false`** ⇒ 它**不被跳过**，即 **`belongs_to` 是实体派生 FK 的唯一来源**
> - `sea-orm-2.0.2/src/entity/relation.rs:418-419` —— `skip_fk` 的定义原文：*"Don't emit a `FOREIGN KEY` constraint when this relation is used to generate schema DDL"* ⇒ **只影响 DDL，不影响 ORM 关联查询**（`find_also_related` 等照常工作）
> - `sea-orm-2.0.2/src/entity/relation.rs:101-103/420-423` —— `skip_fk: bool` 字段 + `skip_fk()` 建筑器；宏侧 `sea-orm-macros-2.0.2/src/derives/attributes.rs:40` 为 `relation_attr` 的**无值 flag**
> - 官方用例 `sea-orm-2.0.2/src/tests_cfg/cake_filling_price.rs:26-32` 给出确切写法：`to = "…",` **有逗号**，`skip_fk` **单独一行且其后无逗号**

## 三个踩过的坑（都会静默给错答案）

1. **⚠ 匹配口径：`belongs_to` 里是「模块名」，不是 `table_name`** —— `belongs_to = "super::sync_device::Entity"` 的模块是**单数** `sync_device`，而表名是**复数** `sync_devices`。不先建「模块名 → table_name」映射，同一条 FK 会**同时**被报成 ENTITY_ONLY 与 MIGRATION_ONLY（首轮各 17 条即此假象，修正后各 14 条）。同族：`workflow_marketplace_review.rs` 文件名单数、`table_name` 复数。
2. **⚠ 扫描器假阴性**（本轮实测）：只写单行 `REFERENCES … ON DELETE` 的正则会**漏掉**写在多行 `CREATE TABLE` 内的 FK —— `v107_paper_reading_list.rs:135/154` 的 `FOREIGN KEY (reading_list_id) REFERENCES reading_lists(id) ON DELETE CASCADE` 就被漏抓，导致 `reading_list_items` 被误判为「实体独有」。**假阴性靠「迁移声明数 vs PG 实测数」的差额才暴露**（29 vs 30 ⇒ 差 1 ⇒ 查出）。⇒ 任何「A 有 B 无」类清单，必须再找一个**独立口径**校总数。
3. **⚠ `Column` enum 大多是宏生成的，文本 grep 不到** —— `conversations.rs` 等用 `#[derive(DeriveEntityModel)]`，`enum Column` 由宏产出。写「目标实体有 `Column::Id` 吗」这类断言**不能用 grep**，真判据是「存在 `#[sea_orm(primary_key…)]` 修饰的 `pub id:` 字段」。

### 五·二·补二 ⚠ **索引命名不兼容：198 个 `indexed` 落地后的直接后果**

**源码事实**（`sea-orm-2.0.2/src/schema/entity.rs:156-162`，本轮实读）：

```rust
if column_def.indexed && !column_def.unique {
    let stmt = Index::create()
        .name(format!("idx-{}-{}", entity.to_string(), column.to_string()))
```

| 事实 | 值 | 依据 |
|---|---|---|
| `indexed` 生成的索引名 | `idx-{表名}-{列名}`，**连字符** | 同上 |
| `entity.to_string()` | **表名**（非结构体名） | 同文件测试期望 `idx-app_user-email`、`idx-indexes-index1_attr` |
| `column.to_string()` | **列名**（snake_case） | 同上 |
| `unique` 字段 | **不生成** indexed 索引（条件含 `!column_def.unique`）；走另一条 unique key 路径 `:170-172` | 同上 :156 |

⇒ 现有 **264** 条 `idx_{表}_{列}`（下划线）与引擎默认产出的 `idx-{表}-{列}`**全量不匹配**。

**风险面实测（已收敛）**：全仓 grep `DROP INDEX` / `REINDEX` / `index_exists` —— **生产代码零引用**，命中全部落在迁移文件内（`v101:380-389`、`v138:51`、`v227:150`、`v232:112-174`），而迁移正是 P6 要删除的对象 ⇒ **无生产代码风险**。

**⟹ 判决（2026-09-16）：采用引擎默认命名，不自研命名层。**

**先补一个我上一版漏掉的前提**：命名问题**不只是索引**。FK 名同样面临同一抉择 —— 库里 30 条 FK 用的是 PG 默认名 `{表}_{列}_fkey`，而引擎产出的是 `fk-{表}-{列}`（`sea-orm-2.0.2/src/entity/relation.rs:509`：`format!("fk-{}-{}", from_tbl, from_cols.join("-"))`）。**只对索引"保留历史命名"、对 FK 用新命名，会制造两套并存的约定 —— 比全改更乱。** 因此二者必须一起决策，且只有两个自洽选项：全默认，或全自定义。

| 选项 | 索引 | FK | 自定义代码 | 一次性成本 |
|---|---|---|---|---|
| **全默认（选定）** | `idx-{表}-{列}` | `fk-{表}-{列}` | **0** | 264 索引 + 30 FK 改名 |
| 全自定义 | 保留 `idx_{表}_{列}` | 保留 `{表}_{列}_fkey` | 永久持有「命名生成 + 名字映射 diff」两层 | 0 |

选**全默认**的三条依据：

1. **索引名在 `create_table_from_entity` 里是硬编码的**（`sea-orm-2.0.2/src/schema/entity.rs:158`），实体注解**没有** `index_name` 可覆盖 ⇒ 「保留 `idx_`」无法只靠官方 API 实现，必须永久自研命名生成 **+ 自研索引 diff**（因为比对时实体侧期望名与库中实名不一致）。而 FK 侧虽有 `fk_name` 可自定义，但那只解决一半。
2. **一次性成本 vs 永久成本**：改名是**一次性、可测、可错峰**的（PG 可用 `CREATE INDEX CONCURRENTLY` 避免写锁；FK 可 `NOT VALID` 后补 `VALIDATE`），而自定义命名层是**每次 SeaORM 升级都要复核**的长期负债。
3. **"未来只有实体" 的纯粹性**：真相源 = 实体 + 库既定约定。保留历史命名等于把「迁移时代的命名」也供成真相的一部分 —— 于是迁移即使被删，它的命名规则仍活着。

**执行约束（P4 必须遵守）**：首次 apply 的删旧建新**不得夹在启动路径里同步跑**，须做成显式、可观测、可回滚的一次性动作（带耗时统计与逐条日志），否则一次启动就会卡住十几分钟且失败无痕。

**残余风险（无法用代码证据排除）**：我只能证明**生产代码**零按名引用（全仓 grep `DROP INDEX`/`REINDEX`/`index_exists` 命中全部落在迁移文件内，而迁移正是要删的对象）。**外部工具**（备份脚本 / 监控 / 性能诊断 / 手工 SQL）按名引用索引或 FK 的可能无法从代码侧排除 ⇒ P4 前应人工确认一次。

**另**：孤儿判定（§四·九）应明确**索引是否纳入 DROP 范围** —— 若纳入，须同时把 §五·三 的 **68 项** L2 例外（FTS5 虚表/触发器/GIN/多列/partial/CHECK/生成列）**排除在孤儿判定之外**，否则它们会被当成「实体未声明的索引」误删。

### 五·三 L2 例外清单冻结：**⚠ 原「43 项」口径已作废 → 实测 72 项（2026-09-16 重算）**

> **为什么重算**：P2 起手按「冻结」要求复核旧表，发现旧值与源码实测多处不符，且**旧口径
> 无法用任何脚本复现**（旧数字怎么来的已无据可查）。按纪律「不可复现的统计量不得作为施工
> 依据」，重新实测。复核脚本 `output/tmp-p2-l2-freeze2.mjs`，输出逐条带 `文件:行号`，可逐行核对。
>
> **差异不是学术问题**：旧值把「多列索引」少算 26 条 ⇒ 若照旧清单实现 L2，引擎接管后会
> **静默缺建 26 个多列索引 + 2 个 partial 索引 + 1 个 CHECK**，正是本表自己警告的
> 「少一条 = 上线后结构缺损」。**该警告反过来命中了旧表自身。**

| 类别 | 旧值 | **实测** | 差异 | 细分 / 说明 |
|---|---|---|---|---|
| FTS5 虚表 | 6 | **6** ✅ | 0 | v100 五个（`messages_fts`/`trajectories_fts`/`trajectory_memories_fts`/`trajectory_skills_fts`/`trajectory_messages_fts`）+ v104 `notes_fts` |
| FTS 同步触发器 | 6 | **6** ✅ | 0 | v100 三个（`messages_ai`/`_ad`/`_au`）+ v104 三个（`notes_fts_ai`/`_ad`/`_au`） |
| GIN 索引 | 6 | **6** ✅ | 0 | messages / trajectory_trajectories / trajectory_memories / trajectory_skills / trajectory_messages / notes 各一 |
| 多列索引 | 9 | **35** ⚠ | **+26** | 普通 31 + **UNIQUE 多列 4**（`uq_fleet_messages_convo_seq` 等） |
| partial 索引 | 2 | **4** ⚠ | **+2** | `idx_messages_branch` / `idx_workflow_templates_mission_hash` / `idx_tool_call_logs_success` / `idx_workflow_executions_paused` |
| CHECK 约束 | 7 | **8** ⚠ | +1 | 静态 4（v100×2、v200×2）+ **模板 4**（`mod.rs` 的 `{categories}`×2、v230 的 `{PERIOD_REGEX}`×2，后者含 `NOT VALID`） |
| 生成列 | 7 | **7** ✅ | 0 | 静态 6（v100×5、v104×1）+ 模板 1（`v227:153` 的 `ALTER TABLE {t} ADD COLUMN {column} ...`） |
| **合计** | 43 | **72** | **+29** | |

**为什么这些必须走 L2（实体语法表达不了）**

| 类别 | 实体为何表达不了 |
|---|---|
| FTS5 虚表 | `CREATE VIRTUAL TABLE ... USING fts5(...)` 是虚表 + 专用 tokenizer 参数，非普通表 |
| FTS 触发器 | 实体只能声明表/列/索引，无触发器概念 |
| GIN 索引 | 实体只能声明 B-tree 单列索引，无访问方法（`USING GIN`）开关 |
| 多列索引 | `#[sea_orm(indexed)]` 只作用于**单个字段**，无复合键形式 |
| UNIQUE 多列 | 同上；且实体 `unique` 只作用单列 |
| partial 索引 | 实体无 `WHERE` 条件表达 |
| CHECK 约束 | 实体无列级/表级约束表达（源码 `sea-orm-macros` 无对应属性） |
| 生成列 | 实体无 `GENERATED ALWAYS AS (...)` 表达 |

**实现形态分两类（决定 `extras.rs` 的接口）**

| 形态 | 数量 | 逐字比对可用？ |
|---|---|---|
| **静态 DDL**（可整条搬进 `extras.rs` 常量） | 6 + 6 + 6 + 35 + 4 + 4 + 6 = **67** | ✅ 可与迁移原文逐字比对 |
| **模板 DDL**（含 `{占位符}`，`format!` 拼出来的） | 4（CHECK）+ 1（生成列）= **5** | ❌ 须**连拼装逻辑一起搬**，不能只搬一份文本 |

> ⚠ 另有 **4 条索引解析失败**（`v111` 的 3 条 `ON {}(...)`、`v227` 的 `CREATE INDEX {i} ON {t} USING GIN ({column})`）
> —— 它们属于上表「模板」形态的索引，已计入对应类别（GIN / 单列），不重复计数。

> ⚠ **方法教训（v1 脚本为何造假）**：首版按「Rust 字符串字面量流」解析全部 7 类，报出
> **触发器 0 / 生成列 0 / FTS5 2** —— 而 grep 实证真值为 6 / 7 / 6。原因是这些 DDL **跨字面量拼接**
> （`"CREATE VIRTUAL TABLE ... fts5(\" + "content, ...`、`for sql in &[...]` 数组里多条语句各占一个字面量），
> 字面量流把它们切断。⇒ **统计量掉到 0 或异常低时先怀疑工具**（纪律 #8 的原型，本会话第二次踩）。
> v2 改为「索引类用字面量解析（P1 已验证 269 条）+ 虚表/触发器/生成列/CHECK 用**按行扫描**」。
> 另修掉一处误收：`mod.rs:493` 的 `description: "... period 格式 CHECK —— ..."` 是说明文字而非 DDL。

---

#### 五·三·补 ⚠ **双口径对账（2026-09-16，`extras.rs` 施工前实测）：静态清单在 4 类上都不准**

> **为什么必须再对一次**：上表的「实测 72 项」仍是**从迁移源码静态扫**出来的条数。
> 而 §五·五 发现 2 的「6 条 tsvector 列」是**连生产库实测**的。两者已经在同一件事上
> 给出不同答案（静态 6 / 实测含 `memory_items` 共 6 条，但静态版有 `trajectory_memories.tsv`
> 而实测版有 `memory_items.content_tsv`）⇒ 按 §五·三 自己的判据「少一条 = 上线后结构缺损」，
> **必须先确定哪个口径是权威**。
>
> **结论：库实态是权威。** 引擎的职责是「让库与声明一致」⇒ 声明必须取自**库的真实结构**，
> 而不是「迁移源码里写过的 DDL」（后者含已删除项、重叠项、以及静态扫描扫不到的项）。
> 脚本 `output/tmp-p2-extras-survey.mjs`（库侧）+ `tmp-p2-diff.mjs`（双向 diff），
> 逐条输出到 `tmp-p2-extras-survey.log` / `tmp-p2-diff.log`。

**四类差异全表（每一格的差都已归因）**

| 类别 | 静态扫描 | **库实态** | 差异归因 |
|---|---|---|---|
| FTS5 虚表（SQLite） | 6 | **5** | **−1**：`trajectory_memories_fts` 随表被 `v101:372` DROP |
| FTS 同步触发器（SQLite） | 6 | **6** | 0 ✅ |
| tsvector 生成列（PG） | 6 静态 + 1 模板 | **9** | **+3**：`v227` 用 `format!` 动态生成 `memory_items.content_tsv` + 3 张 `vec_*_meta.content_tsv`（静态扫不到）；**−1**：`trajectory_memories.tsv` 表已删 |
| GIN 索引（PG） | 6 | **9** | 同上（`idx_memory_items_tsv` + 3 条 `idx_vec_*_meta_tsv`） |
| 多列索引 | 35 | **36** | **−1**：`idx_opc_demand_leads_dedupe` 被 `v138:51` 显式 DROP；**+2**：`news_archive_source_article_code_key` / `workflow_tools_workflow_id_tool_name_key`（**表级内联 `UNIQUE(...)`**，扫描器只解析 `CREATE INDEX` ⇒ 扫不到） |
| partial 索引 | 4 | **5** | **+1**：`idx_retrieval_hits_feedback` —— `v111` 的 `ON {}(...)` 模板让静态解析失败（4 条解析失败之一） |
| CHECK 约束 | 8（**DDL 条数**） | **3**（**约束个数**） | **−5**：8 条 DDL 实为**同一对约束的历代替换** —— `v100:460/470`（建表内联，11 值）→ `v200:409/425`（`ALTER` 替换，14 值含 `opc-industry`）→ `mod.rs:905/921`（`ALTER` 替换，14 值含 `opc-domain_pack`）；另 `v230:116/129` 是**同一约束的 `NOT VALID` 互斥分支** |
| 自定义函数 | **0** | **1** | **+1**：`ax_cjk_ngram(text)` —— 静态扫描的 8 个类别里**根本没有「函数」这一类**，而 `v227` 的生成列表达式引用它 ⇒ 漏声明 = 生成列建不出来（**有报错，非静默**） |

**静态扫描的三个类别缺口（结构性，不是脚本 bug）**

| # | 缺口 | 证据 |
|---|---|---|
| 1 | **函数** | 8 类里无此类；`ax_cjk_ngram` 只从 `v227` 模块常量与 `include_str!` 的 `.sql` 文件里出现 |
| 2 | **表级内联 `UNIQUE(...)` 约束** | 扫描器只解析 `CREATE INDEX`；`news_archive` / `workflow_tools` 的两条多列唯一约束走 `UNIQUE(a, b)` 表约束 ⇒ 静态 35 条不含它们 |
| 3 | **`format!` 模板索引** | `v111` 的 3 条（`ON {}(...)`）+ `v227` 的 1 条（`{i} ON {t} USING GIN ({column})`）解析失败；其中 `idx_retrieval_hits_feedback` 是**带 WHERE 的 partial**，直接影响 L2 覆盖 |

**5 条「静态清单有、库里没有」—— 刻意不收录（收录 = 让引擎重建淘汰对象）**

| 条目 | 归因 |
|---|---|
| `trajectory_memories` 表 | `v101:396` 显式 DROP（数据已迁往 `memory_items`） |
| `trajectory_memories_fts` 虚表 | `v101:372` 随表 DROP |
| `trajectory_memories.tsv` 生成列 | 表不存在，无从建列 |
| `idx_traj_memories_tsv` | `v101:389` 显式 DROP |
| `idx_opc_demand_leads_dedupe` | `v138:51` 显式 DROP（被含 `content_fingerprint` 的新唯一索引取代） |

**⟹ L2 声明的最终条目账（= `reconcile/extras.rs` 的冻结值，68 条）**

| 声明载体 | 条数 | 说明 |
|---|---|---|
| `VIRTUAL_TABLES` | 5 | FTS5 虚表（SQLite） |
| `TRIGGERS` | 6 | FTS 同步触发器（SQLite） |
| `GENERATED_COLUMNS` | 6 | tsvector 生成列（PG，静态表名） |
| `PATTERN_TSV` | 1 | `^vec_.*_meta$` 动态表名的生成列 + GIN（覆盖 3 张现存表，且**未来新建集合自动覆盖**） |
| `FUNCTIONS` | 1 | `ax_cjk_ngram(text)` |
| `CHECK_CONSTRAINTS` | 3 | 去重后的真实约束数 |
| `GIN_INDEXES` | 6 | 静态表名 |
| `MULTI_COL_INDEXES` | 35 | 含 2 条表级内联 UNIQUE；剔除 1 条已 DROP + **1 条无实体表**（见下） |
| `PARTIAL_INDEXES` | 5 | 含 1 条静态漏计 |
| **合计** | **68** | |

> ⚠ **不要再把「72」当施工依据** —— 72 是「迁移源码里非实体 DDL 的静态条数」，
> 与「引擎必须声明以保证不丢结构的条目数」**口径不同**（前者含 5 条已删除项、
> 含 3 条重叠的历代替换、少 3 类对象）。`extras.rs` 按 **68** 冻结，并由
> `#[cfg(test)] mod counts` 逐类锁定条目数 ⇒ 改条目必须显式改断言。

**⚠ P3 删掉 1 条（69 → 68）：`idx_chat_run_events_run_id`（2026-09-16）**

| 项 | 内容 |
|---|---|
| 条目 | `MULTI_COL_INDEXES` 里的 `{ table: "chat_run_events", name: "idx_chat_run_events_run_id", cols: ["run_id","seq"] }`（源自 `v207_chat_run.rs`） |
| 为什么必须删 | `chat_run_events` **没有实体**，按 §五·一 B 组的裁决该表整体交孤儿判定处置（引擎上线后 DROP）。L2 再认领它的索引 ⇒ 同一轮 plan 里会同时出现 `DROP TABLE chat_run_events` 与 `CREATE INDEX idx_chat_run_events_run_id`，后者**必然执行失败** |
| 怎么被抓出来的 | 不是静态扫描，而是 P3 的 `expected::build` **硬错误**：L2 静态声明认不到表即 `Err`（`L2 索引 … 指向实体未覆盖的表 …`）。**这正是「声明与实体集必须自洽」这条规则的价值实证** |
| 由此确立的规则 | **无实体表上的索引一律不进 L2** —— 处置随表本身（§五·一 已裁决的 26 条同理）。回归锁：`extras::counts::indexes_on_entity_less_tables_are_not_declared` + `expected::tests::l2_static_decls_all_resolve_to_entity_tables` |
| 复核 | `output/tmp-p3-l2-orphan.mjs`：把 `GIN/MULTI/PARTIAL/CHECK/GENERATED` 五处 `table:` 字面量与实体 `table_name` 全集（183 张）求差 ⇒ 修前 1 条、修后 **0 条** |

**同轮新登记的两条缺口（按纪律不擅改，仅登记）**

| # | 内容 | 性质 |
|---|---|---|
| 1 | `idx_retrieval_hits_created`（`v111` 建的单列索引）**实体未声明** `#[sea_orm(indexed)]` on `created_at` | P1 的 198 个 `indexed` 落地时的遗漏。按 §五·二·补二「全默认命名」裁决，补注解后旧名索引会被判孤儿 ⇒ 属 P4 的一次性改名成本；**不进 L2**（单列索引实体能表达） |
| 2 | `ax_cjk_ngram` 只覆盖 `notes` / `memory_items` / `vec_*_meta`；`messages` 与 3 张 `trajectory_*` 的 `tsvector` **仍是旧版** `to_tsvector('simple', ...)` | 这些表的中文检索仍然失效（连续 CJK 塌缩成单词语元）。`extras.rs` 按**实态**声明（不擅自扩大变更面）；扩到剩余 4 表是独立改动 |

---

### 五·四 运行时自建表（**2026-09-16 二次修正：真正在主库的只有 2 张**）

> **修正说明（同日二次）**：本节初版据「迁移目录之外的 `CREATE TABLE` 有 27 处」断言「12 张活跃表会被引擎当孤儿 DROP，每次启动清空一次」。**该论断有误**——它漏了决定性的一步：**追这些表建在哪个数据库连接上**。
>
> 实测结论：12 张里 **10 张不在主库**（4 张的宿主**从未被实例化** ⇒ 表在库里根本不存在；6 张在**独立索引文件** `index_dir/index.db` 里）。**只有 `cron_jobs` / `cron_job_history` 在主库。**
>
> 教训：**「有 `CREATE TABLE` 语句」≠「表在主库里」**。判定一个对象是否受 schema 引擎管辖，唯一可靠的判据是**追它的连接来源**（见判据 #287）。

**逐表判定**（全部基于代码事实，非推断）——⚠ **各行的「生产实例化点 / 宿主库 / 引擎可见」三列均为 P0a 时点（2026-09-16 之前）的判定快照，不是现值**；现值以紧接本表的「**2026-09-16 实测更正**」表为准（`audit_log` **✅ 已接通** · `divergence_logs` **✅ 整链删除** · `l2_search_results` / `l2_index_snapshots` **✅ 已真接线** · `l2_summaries` **✅ 整链删除**）。本表的价值是**记录当时的推断及其错处**（P0a 把 `DiskCache::new` 当判据，锚点选错致结论反向，见判据 #357），**不是当前真值**；改动本表行号 = 篡改历史陈述（判据 #542）：

| 表 | 宿主 crate | 连接类型 | 生产实例化点 | 宿主库 | 主库？ | 引擎可见 |
|---|---|---|---|---|---|---|
| `cron_jobs` | `runtime-core` | **sea-orm `DatabaseConnection`** | ✅ `src/init/state.rs:523` `CronJobStore::new(Arc::new(sea_db.clone()))` | 主库 | ✅ **是** | ⚠️ **会被判孤儿** |
| `cron_job_history` | `runtime-core` | 同上 | ✅ 同上（`src-tauri/crates/runtime-core/src/cron_job.rs:441`） | 主库 | ✅ **是** | ⚠️ **会被判孤儿** |
| `file_index` | `search` | rusqlite | ✅ `src/indexing_triggers.rs:122` | **`index_dir/index.db`**（`indexing_triggers.rs:109`，常量 `INDEX_DB_FILENAME = "index.db"`） | ❌ | 否 |
| `ast_functions` / `ast_classes` / `ast_interfaces` / `ast_variables` / `ast_call_edges` | `search` | rusqlite | ✅ `src/indexing_triggers.rs:161` | 同上（同一 `index.db`） | ❌ | 否 |
| `l2_search_results` / `l2_index_snapshots` | `disk-cache` | ~~rusqlite~~ **SeaORM**（2026-09-16 改造） | ✅ `src/init/state.rs:546` `init_l2(app_dir/l2_cache.db)` —— **启动路径上无条件调用** | **`{app_dir}/l2_cache.db`**（侧车库） | ❌ | 否（侧车库，不进注册清单；实体在册仅供建表派生） |
| ~~`l2_summaries`~~ | ~~`disk-cache`~~ | — | — | — | — | **2026-09-16 整链删除**（零调用 + 与主库 `conversation_summaries` 功能重复 + 侧车库无 FK ⇒ 必产孤儿；`l2_cache.db` 从未被创建 ⇒ **零 DROP**） |
| `audit_log` | `tools` | rusqlite | **〔P0a 时点〕** ❌ **零实例化**——`ToolAuditor::new` 零命中；唯一生产构造是 `tools/src/registry.rs:685` 的 `ToolAuditor::default()`，而 `default()` 置 `audit_db_path: None`（`audit.rs:71`）⇒ `and_then`（`audit.rs:89`）短路，**从不开库** | 无（`None`） | ❌ | 否（表不存在）**〔P0a 时点；现值 ✅ 已接通，见下更正表〕** |
| `divergence_logs` | ~~主 crate~~ **已删除** | ~~rusqlite~~ | **〔P0a 时点〕** ❌ 零实例化——`DivergenceLogDao::init` 仅在 `src/divergence-log.rs:281/311` 自身测试中调用（该文件亦已删除） | 无 | ❌ | 否（表不存在）**⇒ 2026-09-16 裁决：整链删除**（见 §五·四 ③ 删除记录） |

**由此得出三个结论**：

1. **真正需要补实体的只有 2 张**（`cron_jobs` / `cron_job_history`），工作量远小于初版判断。
2. **`cron_jobs` 的现状恰恰是「违规」的典型**：它在**主库**里，却用裸 SQL `execute_unprepared`（`cron_job.rs:37`）建表、用 `DbBackend` 分支拼 SQL 访问，**完全绕过实体体系** —— 这正是「实体 = 唯一建表与访问形式」这条铁律要消灭的形态。
3. ~~**`l2_*` / `audit_log` / `divergence_logs` 三组不是「活表」，是死代码**~~ —— **P0a 时点判定，2026-09-16 已逐条更正（见下）**。原判据是「写入代码存在，但宿主从未被实例化 ⇒ 表从未建成」，它对 `divergence_logs` 成立（整链已删，实体与实现同归于死），对另两组**不成立**。

**2026-09-16 实测更正（双轨：上方 P0a 原值保留，本段为现值）**

| 组 | P0a 判定 | 2026-09-16 实测 | 根因 |
|---|---|---|---|
| `audit_log` | 零实例化 | **✅ 已接通**（`src/init/state.rs:539` `init_audit_db(sea_db.clone())`） | 批 2 修好（`audit_db_path` 恒 `None` ⇒ 只写内存） |
| `divergence_logs` | 零实例化 | **✅ 整链删除** | 用户裁决「没用就删了」 |
| `l2_search_results` | 零实例化 | **✅ 已真接线**：读 `search/src/search.rs:519`（`get_search_results`）/ 写 `:594`（`store_search_results`） | `DiskCache::new` 是**测试专用构造**，生产走 `DiskCache::open` ← `init_l2`（`init/state.rs:546`）。P0a 用 `DiskCache::new` 做判据 ⇒ **锚点选错，结论反向** |
| `l2_index_snapshots` | 零实例化 | **✅ 已真接线**：读 `indexing_triggers.rs:258`（`get_snapshot`）/ 写 `:268`（`record_snapshot`，真 upsert） | 同上 |
| `l2_summaries` | 零实例化 | **❌ 确认零外部调用** —— 3 个方法（`store_summary`/`get_summaries`/`evict_old_summaries`）全仓只有 crate 自测调用 ⇒ **✅ 2026-09-16 整链删除**（用户裁决「按建议处理」） | 唯一判对的；且与主库 `conversation_summaries` 功能重复 |

⇒ **教训（判据 #357）**：判「宿主是否被实例化」时，**不能用某个具体构造方法的命中数当判据** —— 一个类型可以有多个构造入口（`new` 仅测试、`open`/`load_or_default` 走生产）。正确判据是**追进程级单例的注册点**（本例 `init_l2`），再看该注册点是否在启动路径上。

⚠ **同时修正 §五·四 ③ 的措辞**：原文把 `l2_*` 三张当一整组说「零消费者」是**错的** —— 它们内部差异极大（2 活 1 死）。**同名前缀不构成同一处置组**。

**待处置清单（按 2026-09-16 用户原则修正）**：

| 组 | 对象 | 处置 | 理由 |
|---|---|---|---|
| **① 主库 · 活跃** | `cron_jobs` / `cron_job_history` | **补 2 个实体 + 改造 `cron_job.rs`** | 在主库、活跃、绕过了实体体系 |
| **② 侧车库 · 活跃** | `file_index` + `ast_*`（6 张，`index.db`） | **补 6 个实体** + 该库连接改用 `DatabaseConnection` | 按用户原则「访问须走实体」；**原判「保留现状」作废** |
| **③ 宿主未实例化**（标题已过时见 §五·四 后注） | `l2_*`（**须拆成 2 活 1 死**）/ `audit_log` / `divergence_logs` | **✅ 全组处置完毕（2026-09-16）**：`audit_log` ✅ 已接通（批 2，顺带修 2 个真 bug）；`divergence_logs` ✅ 整链删除（裁决「没用就删了」）；**`l2_search_results` / `l2_index_snapshots` ✅ 实测已真接线**，前者另修一个真缺陷（见下）；`l2_summaries` ✅ **整链删除**（裁决「按建议处理」） | 见各表判定行与下方两段记录 |
| **④ 向量表 · 豁免** | `vector_store` 的 `{name}`（3 处） | **保留原生 SQL**（用户明确批准） | 动态表名，实体语法无法表达 |

#### ② 的边界裁决：侧车库**必须纳入**（**2026-09-16 用户裁决，我原判作废**）

> ⚠️ **我的原判是错的，此处留痕。** 我先前写：「实体（`DeriveEntityModel`）绑定的是**主库的** `DatabaseConnection`……**实体机制在语法上就无法管辖侧车库**——不是「不想」，是「不能」」，并据此建议**把侧车库排除在铁律范围外**。
>
> **错在哪**：我把「实体绑定主库」当成了**语言层事实**。实际 SeaORM 的 `DatabaseConnection` 可以连接**任意 SQLite 文件**（`Database::connect("sqlite:/path/index.db?mode=rwc")`），而实体是一个**类型**，可用于任何**兼容的连接** ⇒ **实体绑定的是「连接」，不是「某个特定的库」**。「主库」只是「当前唯一那条连接」的偶然事实，不是实体机制的限制。

**用户裁决的原则（2026-09-16）**：

> **除向量操作可使用 SQL 原生语句外，其它数据库访问必须通过 SeaORM 实体进行。**

由此，管辖范围**不按「主库 / 侧车库」分界**，而按「**该存储是否属于向量操作**」分界：

| 存储 | 纳入实体管辖 | 说明 |
|---|---|---|
| 主库（PG / SQLite） | ✅ **必须** | 原本如此 |
| 侧车库 `index.db`（`file_index` + `ast_*`，6 张） | ✅ **必须**（原判「排除」作废） | 另建一条 `DatabaseConnection` 指向该文件即可 |
| 审计库（`audit_log`） | ✅ **必须** | 同上 |
| ~~分歧日志（`divergence_logs`）~~ | ❌ **不适用（已删除）** | 2026-09-16 裁决：零写入端 + 零读取端 + 迁移链无 `CREATE TABLE` + 库中亦无该表 ⇒ 整链移除，**不再有需要管辖的对象** |
| L2 缓存库（`l2_*`，3 张） | ✅ **必须** | 同上（`disk-cache` 是唯一需新增 sea-orm + tokio 的 crate） |
| **向量表**（`vector_store` 的 `{name}` 动态表名） | ❌ **豁免**（用户明确批准） | 表名运行时拼接，一实体一固定表名 ⇒ **实体语法无法表达** |
| FTS5 虚表查询（`MATCH`） | ⚠️ 折中方案 | sea-query **无 MATCH 构造器**（只有 PG 的 `to_tsquery` / `plainto_tsquery` 族，非 SQLite FTS5 的 `MATCH`）；但 `Expr::cust`（`sea-query-1.0.2/src/expr/enum.rs:368`）可把 `notes_fts MATCH ?` 作为**查询条件**嵌进 `Entity::find().filter(...)` ⇒ **查询主体仍是实体**，只是条件片段是裸 SQL |

⇒ **引擎的孤儿判定输入随之扩展**：不再只 introspect 主库连接，而是**对每一条「由本仓拥有的 `DatabaseConnection`」各做一次 introspect**（新增 `ConnectionRegistry`：连接 → 该连接上期望存在的实体集合）。这样 `index.db` 的 6 张表也进入真相源，而不是靠「假装看不见」来规避误判。

**成本分布**（实测各 crate 的 `Cargo.toml`）：

| crate | sea-orm | tokio | entities | 结论 |
|---|---|---|---|---|
| `search` | ✅ | ✅ | ✅ | 只需改代码 |
| `trajectory` | ✅ | ✅ | ✅ | 只需改代码 |
| `tools` | ✅ | ✅ | ❌ | **+ `axagent-entities`** |
| `runtime-core` | ✅ | ✅ | ❌ | **+ `axagent-entities`** |
| **`disk-cache`** | ❌ | ❌ | ❌ | **唯一需新增 sea-orm + tokio**，且其 API 是同步 `fn` ⇒ 须异步化 |

⇒ **主要成本是「`rusqlite::Connection` → `DatabaseConnection` + 同步 `fn` → `async fn`」的改造面**，不是「加依赖」。
⇒ 依赖方向无风险：`entities → harness`（harness 零 axagent 依赖）⇒ `entities` 是第二低位 crate，**任何 crate 加它都不会产生循环依赖**。

#### ③ 死代码组的处置建议

三组的共同形态是「**有完整实现 + 有实体/有 DDL，但宿主零实例化**」——即「**最后一公里未接线**」。它们与 §五·一 那 5 张零访问表（表存在但无人读写）不同，是**更早一步的死**：连表都没建成。

其中 `divergence_logs` 值得单列：它有**防篡改哈希链**设计（`row_hash` / `prev_hash`，且 DAO 层显式禁 DELETE，`divergence-log.rs:200-205`）——是一个**设计完整的审计功能，从未被接上生产**。是否启用属产品决策。

**⇒ 2026-09-16 用户裁决：「没用就删了」。整链删除，不留残骸。**

删除前复核三项前置判据，全部成立：

| 判据 | 实测 |
|---|---|
| 零写入端 / 零读取端 | 全仓 `divergence_logs::` 路径引用 **0**；`compute_hash` / `verify_chain` 的外部命中**全是别处同名的私有函数**（`storage/file_store.rs`、`runtime-core/prompt_cache.rs`、`commands/wiki.rs`）—— 判据 K 的撞名形态，不是消费者 |
| 迁移链内无 `CREATE TABLE` | `migrations/` 全目录 grep `divergence` = **0 命中** ⇒ 该表**从未有迁移建表路径** |
| 库中亦无该表 | 本机 PG 任意 schema 查 `pg_class` 的 `%divergence%` = **0 行** ⇒ 删除**不需任何 DROP**，零 DB 变更 |

**删除清单（4 项）**：

| 对象 | 处理 |
|---|---|
| `crates/entities/src/divergence_logs.rs` | 删（13713 B） |
| `crates/entities/src/lib.rs` 的 `pub mod divergence_logs;` | 删注册行 |
| `src-tauri/config/divergence-log-schema.json` | 删（4672 B；设计稿，全仓零加载，且字段口径与实体不一致） |
| `crates/entities/Cargo.toml` 的 `hex` / `sha2` | 删（删实体后**全 crate 无使用者**，grep 已证仅该文件引用） |

**备份**：`output/backup-2026-09-16-divlog/`（被删的 4 个文件 + 改前的 `src/lib.rs`）。`src-tauri/src/lib.rs` 处的注释就地更新为闭环记录 —— 原注释指向实体的「收敛记录」段，删实体后即成**悬空指针**。

**随之消失、在此存档的两项**（否则重建时只能从零猜）：

1. **`record_divergence_from_trail` 的规则→维度映射**（无生产调用点，且**本身是推测性的**）：`R-200 → risk_assessment`、`R-201 → trader_consensus`、`R-401|R-402|R-403|R-404|R-405 → technicals`、其余 → `trader_consensus`。⚠ `seed_stock_analysis.rs` 已实证 **R-403/R-404 在本仓零生产者** ⇒ 要重建写入端，应按 `decision-explainer` 的现行规则表（公式决策层 / 组合风控门 / 技术面否决）重写，**不要照搬此表**。
2. **实体侧的 `compute_hash` / `verify_chain` 是一份正确实现**（真重算 + 升序，曾被 `test_verify_tampered_chain` 锁住）—— 与手工版的「假验证」（只比 `prev_hash` 指针、顺序还反了）恰成对照。若将来重建，**取备份里的版本**，不要重新发明。

⇒ **连带效果**：§五·五 的实体侧计数同步 −1（实体 184→**183**、交集 169→**168**、B 侧 15→**14**、全仓注解 185→**184** 而越界仍 **0**）。

#### ③ 组第二批处置：`l2_summaries` 删除 + `l2_search_results` 缺陷修复（2026-09-16，用户裁决「按建议处理」）

**先更正我上一轮的一处误判**：我曾把 `l2_*` 三张当**一整组**说「零消费者」。**错的** —— 实测是 **2 活 1 死**：`l2_search_results`（读 `src-tauri/crates/search/src/search.rs:519` / 写 `:594`）与 `l2_index_snapshots`（读 `indexing_triggers.rs:258` / 写 `:268`）**两列都非空 = 已真接线**；只有 `l2_summaries` 两列皆空。根因是判据选错：我用 `DiskCache::new` 的命中数当「是否实例化」的依据，而它**只是测试专用构造**，生产走 `DiskCache::open`（← `init_l2` ← `init/state.rs:546`，**在启动路径上无条件调用**）⇒ **锚点选错，结论反向**（判据 #357）。

**A. `l2_summaries` 整链删除**（三条理由，任一条单独成立）

| 理由 | 实测 |
|---|---|
| 零调用 | 3 个方法（`store_summary` / `get_summaries` / `evict_old_summaries`）全仓只有 crate 自测调用 |
| 与主库功能重复 | 主库 `conversation_summaries` 由 `dao::repo::conversation::upsert_summary` 写入，字段更全（多 `compressed_until_message_id` / `token_count` / `model_used`）且是权威真相源（`commands/conversations/compress.rs:253`） |
| **必然产生孤儿** | 侧车库与主库**无外键** ⇒ 会话删除后本表摘要行不会被级联清理，也无主体去清它 ⇒ 只增不减 |

**零损失论证**：`l2_cache.db` 位于 `~/.axagent/`，实测**该文件从未被创建**（接线是 09-16 做的，应用最后启动是 09-15）⇒ 表从未建出 ⇒ **零 DROP、零迁移、零 DB 变更**。

删除清单（6 处）：实体文件 `entities/src/l2_summaries.rs`（912 B）· `entities/src/lib.rs` 的 `pub mod` 行 · crate 文档的接线表行与 `l2_summaries` 段 · `CachedSummary` 结构体 · 三个方法及其 `// ── Summary caching` 分节 · `ensure_tables` 的建表调用 · `DiskCacheConfig` 的 `max_summaries` / `summary_ttl_days` 两个字段（含 `Default`）· 自测 `test_summary_store_and_load`。

备份：`output/backup-2026-09-16-l2/`（5 文件）。

⚠ **同时更正一处空护栏声明**：crate 文档原写「同一份摘要再写进侧车库会被 `scripts/check-single-source-facts.mjs` 拦下」。实测该脚本（712 行）只扫两类对象 —— rustdoc 的 `文件:行` 引用、reranker 模型文件名；**表级重复不在扫描面内**（脚本内 grep `l2_summaries` 零命中）⇒ **项目里并不存在拦这类重复的单源门禁**。删表是因为上表三条实质理由，不是因为会被拦。

**B. `l2_search_results` 的真缺陷修复**（`query_hash` 无唯一约束）

原实现 `INSERT OR REPLACE` 对**无唯一约束**的列**从不触发替换** ⇒ 每次 store 都新增一行；重复行被 `get` 的「取首行」掩盖，表现为 `hit_count` 永远从 1 重来 + 表无限膨胀。**演进史三步，每步的修法自身都留过缺陷**：

| # | 修法 | 留下的缺陷 |
|---|---|---|
| 1 | `INSERT OR REPLACE` | 缺唯一约束 ⇒ 从不替换，只累积 |
| 2 | 「先按 hash `DELETE`、再 `INSERT`」 | 累积止住，但**两条非原子语句** ⇒ 中途失败留「已删未插」空洞，而调用方看到成功 |
| 3 | 实体加 `#[sea_orm(unique)]` + `ON CONFLICT (query_hash) DO UPDATE` | **单条原子语句**，唯一性由数据库保证 |

与第 2 步的一处**有意差异**：upsert 保留原行 `id`（`DELETE`+`INSERT` 会换新 id），使 `CachedSearchResult.id` 在覆盖前后稳定；`hit_count` 在覆盖时重置为 1（缓存内容已变，旧命中次数不再可加）。

**关键前置（本轮实测澄清，别再踩）**：sea-orm 对唯一性**分两条路**（`sea-orm-2.0.2/src/schema/entity.rs`）—— `#[sea_orm(unique)]` 由 `create_table_from_entity` 落进建表语句（`:258-260`）；`#[sea_orm(unique_key = "...")]` 则要 `create_index_from_entity`（`:143-183`）**另发一条 `CREATE UNIQUE INDEX`**，而 `disk-cache` **没调它**。⇒ 属性从 `unique` 误改成 `unique_key`，约束会**静默消失**、upsert 退化成「每次 INSERT 都成功」。已加回归锁 `test_query_hash_unique_constraint_exists` 专拦这一种改法。

⚠ **我踩的一个错锚点**：起初想拿「PG 里现有 unique 约束的形态」反推 `create_table_from_entity` 的行为 —— 但库里那些约束**全是迁移手写的**（引擎 P4 才接管）⇒ **测的不是被测对象**。唯一可靠判据是自己跑（实跑测试），即判据纪律「反向断言须锚定被测对象自身形态」。

**验收**：`cargo test -p axagent-disk-cache --lib` → `TEST_EXIT=0`、**8 passed / 0 failed**、零 warning；其中 `test_query_hash_unique_constraint_exists` 与 `test_store_overwrites_in_place` 为本轮新增的两条回归锁。日志 `output/tmp-l2-test.log`。

**已知边界（登记，未处理）**：`ensure_tables` 用 `if_not_exists()` ⇒ 若将来出现「已存在但缺唯一约束」的 `l2_cache.db`，`ON CONFLICT` 会报错。当前无此库（实测不存在），故不引入额外迁移；届时错误信息本身已明确指向该问题，且 `search.rs` 对写入失败是 `warn!` 静默降级（不影响检索本身）。

**连带计数**（§五·五 / §九 已同步为**完整账**）：实体 `table_name` 声明 **183**（P0b 时点 184 → P0c 迁入 `gateway_message_queue` **+1** = 185 → 删 `divergence_logs` **−1** = 184 → 删 `l2_summaries` **−1** = **183**）；`entity_modules!` 注册 **183** = `pub mod` **183**（读 `build.rs` 产物 `entity_registry.rs` 实证，零漏注册零幽灵声明）；越界仍 **0**；「实体有·库中无」15 → **13**。

⇒ **P0c 的 ③ 组至此全部处置完毕**（`audit_log` ✅ 接通 / `divergence_logs` ✅ 删 / `l2_*` ✅ 2 留 1 删）。

⇒ **两个口径必须分开报（本节最终结论）**：

| 口径 | 范围 | 含义 |
|---|---|---|
| **引擎误判面**（不处理会丢数据） | **2 张**（`cron_jobs` / `cron_job_history`） | 只有这 2 张在**主库**且活跃，引擎上线会判孤儿 DROP |
| **架构合规改造面**（按用户原则须改实体） | **12 张**（上表 ①②③ 三组） | 「除向量操作外访问须走实体」⇒ 全部须改造；其中 10 张属「不改造也不会丢数据」 |

⇒ **原判的 `RuntimeOwnedTable` 声明通道仍然不需要** —— 真相源就是实体本身，不是新增的声明类。而「给侧车库开豁免」这类**声明式绕行恰是用户原则所拒绝的形态**（原判建议的「侧车库合规三条件」已随之作废）。

---

### 五·五 生产库实态全量 diff（P0b）：**✅ 已完成（2026-09-16）**

脚本：`output/tmp-p0b-schema-diff.mjs` → `output/tmp-p0b-diff.log` + `output/tmp-p0b-report.json`（主）
辅助：`tmp-p0b-outside-entities.mjs`（引擎盲区）· `tmp-p0b-gap-check2.mjs`（tsvector 列 + 孤儿访问面）· `tmp-p0b-tail-check.mjs`（`Json`/`text` 差异 + 零迁移实体）· `tmp-p0b-version-chain.mjs`（版本链）

**库侧实测（`schema='public'`）**：**184 张表 / 0 视图 / 2062 列 / 477 索引 / 30 FK**

#### ① 四项原定目标的实测结果

| # | 目标 | 实测 |
|---|---|---|
| ① | A 批 5 张零访问表的实际行数 | **全部 0 行** ⇒ 「不补实体、交引擎清理」**零数据损失**（判定获实证支持） |
| ② | B 批运行时自建活跃表的行数 | `cron_jobs` **2 行**（活跃定时任务）· `cron_job_history` 0；补实体 4 张中仅 `wiki_graph_cache` 有数据（2 行） |
| ③ | 补实体表的列类型是否被 `schema_diff` 无损加宽过 | **危险错配 = 0**。6 条「实体 `Json` / 库 `text`」差异的列**全部 `nonNULL = 0`**（零数据）⇒ 读写均无实际暴露 ⇒ 定性**无害** |
| ④ | 库里对象总数与实体对账 | 库 **184** ｜ 实体 **184** ｜ 交集 **169** ｜ 两侧各缺 **15**（全量归因见下） |

#### ② 表级对账全量归因（P0b 出口判据：差距清单可逐条归因 ⇒ **100% 归因**）

**A 侧：库有表 · 无实体（15）**

> ⚠ **本节是 P0b 时点的口径**（`migrations/schema_diff.rs` 扫描 ⇒ 15 张），与 P3 实测（引擎自己的 `introspect` ⇒ **14 张**）不同。差的那 1 张是 `gateway_message_queue` —— 它已在 **§八 风险 1c** 里**迁入实体**（✅ 2026-09-16 批 10），故不再属「仅实况」。向量表 7 张与 `opc_capability` 两处口径**完全一致**。**权威口径以 §七·补二 ③ 的 P3 实测 14 张为准**（逐张打印、可复现），本节保留用于追溯「这 15 张当初是怎么归因的」。
>
> 同因：本行上方的「库 **184**｜实体 **184**」也是 P0b 时点的数（P3 实测为「实况 **185**｜期望 **183**」）—— 期望侧减少源于实体归并，实况侧增加源于库中新增表。

| 分类 | 数量 | 明细 |
|---|---|---|
| 向量表 | 7 | `vec_capabilities` `vec_capabilities_meta` `vec_kb_*`×3 `vec_wiki_*`×2。⚠ **P3 实测（2026-09-16）**：这 7 张**声称「已豁免」，当时引擎只豁免了其中 3 张 `_meta`** —— 豁免通道 `PATTERN_TSV` 要求后缀**恰好** `_meta`，基表（`vec_capabilities` / `vec_kb_f9b2b050_…` / `vec_wiki_dc8efbd8_…`）**仍会被判孤儿 DROP**。✅ **已裁决并落地**：新增显式规则 `PATTERN_ORPHAN_EXEMPT`（`vec_` 前缀 + 排除 `_dup_backup`）⇒ 全族豁免；实测 `DROP TABLE 11 → 8`（详见 **§七·补二 ③‑补** 与 **§八 风险 1d**） |
| 引擎版本表 | 1 | `axagent_schema_version`（**74 行** = 74 个迁移；P6 随迁移机制一并删除） |
| A 批：P0a 判定零访问不补实体 | 5 | `chat_runs` `chat_run_events` `rl_policies` `opc_demand_lead` `opc_market_platform`。⚠ **P3 实测订正（2026-09-16）**：前版此处写「上列 5 张里有 3 张在库中已不存在」，**该说法对错了对象** —— 真实情况是：① 这 **5 张全部存在**，且 **全部 0 行**（`COUNT(*)` 实测，见 §9.5 E15）⇒ 「判 DROP 零数据损失」成立；② 库中**确实不存在**的是另外 3 张 `opc_capabilities` / `opc_domain_pack` / `opc_industry`（P0a 旧清单里的候选人，改名后消失，**不属于**本行的 5 张）；③ 容易混淆的名字是 `opc_demand_leads`（**复数**，**594 行**，**有实体**、不在「仅实况」内）与 `opc_capability`（**单数**，**324 行**，**无实体**、在「仅实况」内）。**权威口径以 §七·补二 ③ 的 14 张表为准** |
| 🟡 **P0a 处置表未收录**（本轮补登） | 1 | `opc_capability`（**324 行**，`COUNT(*)` 复测同值）⇒ ✅ **已裁决（2026-09-16）：先导出再 DROP**（导出 324/324 行已校验，§9.6 E19） |
| 🔴 **引擎盲区**（见发现 1） | 1 | `gateway_message_queue` |

**B 侧：实体有 · 库无表（15）**

| 分类 | 数量 | 明细与归因 |
|---|---|---|
| 侧车库 `index.db` | 6 | `ast_*`×5 + `file_index`（P0a 已归因；引擎不管辖该库） |
| 宿主零实例化（表从未建成） | **4** | `l2_*`×3 + `audit_log`（P0a 已归因）。⚠ P0b 当时记为 **5**，第 5 项 `divergence_logs` 的实体已于 2026-09-16 删除（见 §五·四 ③）⇒ 现值 −1 |
| 迁移有建表 DDL 但未应用 | 1 | `capability_domain_overrides` —— **库版本停在 `v230`**，v231/232/233 **从未在本机库执行**（正常开发状态，非缺陷） |
| 🔴 **零迁移建表**（见发现 3） | 3 | `analyst_feedbacks` `business_roles` `capability_clusters` |

#### ③ 三项本轮新发现（PLAN 前版没有）

**发现 1 · 🔴 引擎盲区：`gateway_message_queue`** — 与 B 批 `cron_jobs` 同型的高危项

- `crates/runtime/src/persistent_queue.rs:7` 有**完整**的 `DeriveEntityModel` 实体（`Model` + `Relation` + `ActiveModelBehavior`），但它**不在 `entities` crate**。
- 而注册权威是 `crates/dao/build.rs` 读取的 **`entities/src/lib.rs` 的 `pub mod` 清单**（build.rs 从磁盘读、生成 `entity_modules!` 宏）⇒ 该实体**不进引擎的遍历集合** ⇒ `gateway_message_queue` 被判孤儿表 **DROP**，而 `PersistentQueue` 仍在读写它。
- 全仓 `#[sea_orm(table_name=...)]` 注解共 **185** 处 ⇒ **仅此 1 处越界**（边界清晰，不是普遍问题）。
- **处置（与 B 批同级，P4 强制前置，非可选）**：把该实体迁入 `crates/entities/src/` 并在 `lib.rs` 声明 `pub mod`，runtime 侧改为引用。

**发现 2 · 🔴 6 条 `tsvector` 列实体未声明 ⇒ 引擎会 DROP 掉 CJK 全文检索**

| 表.列 | 表行数 | 非 NULL | GIN 索引 |
|---|---|---|---|
| `notes.tsv` | **48590** | **48590** | `idx_notes_tsv` |
| `trajectory_trajectories.tsv` | 15 | 15 | `idx_traj_trajectories_tsv` |
| `memory_items.content_tsv` | 2 | 2 | `idx_memory_items_tsv` |
| `messages.content_tsv` | 2 | 2 | `idx_messages_content_tsv` |
| `trajectory_messages.tsv` | 0 | 0 | `idx_traj_messages_tsv` |
| `trajectory_skills.tsv` | 0 | 0 | `idx_traj_skills_tsv` |

- 来源：`v100_consolidated.rs:1184-1200`（`ALTER TABLE … ADD COLUMN tsvector` + GIN）、`v104_notes_fts.rs:64-67`、`v227_cjk_fts.rs`（中文 n-gram 生成列重建）。
- **消费面广泛**：`search/src/hybrid_search.rs` · `search/src/vector_store.rs` · `search/src/text_ngram.rs` · `src/commands/conversations_search.rs` · `src/commands/wiki.rs` · `tools/src/tools/agent_memory.rs` · `dao/src/repo/conversation.rs`。
- ⇒ **实体语法无法表达 `tsvector` + `USING GIN`（含 `v227` 的生成列表达式）⇒ 属「实体表达不了的 DDL」⇒ 必须进 L2 例外清单**（`reconcile/extras.rs`），P2 冻结时**不可遗漏**（清单最终 **68 项**，见 §五·三）。这是「实体漏写 = 静默删列」在**主库**上的第一个确凿实例（`notes` 一条就涉及 48590 行）。
- 同类还有 3 条 `narrative_structures.arcs` / `confluences` / `foreshadows`：v220 自述为**旧三列 schema 的残留草稿**（真正的 schema 权威是 v126 的 `structure` 单列），该表 **0 行** ⇒ 引擎 DROP 属**正确清理**，无需 L2。

**发现 3 · 🟡 3 个实体零迁移建表（引擎会给它们新建表）**

| 实体 | 主库有表 | 消费面（实测） | 性质 |
|---|---|---|---|
| `analyst_feedbacks` | ❌ | **`src/commands/analyst_feedback.rs`(9 处) + `analysis-engine/node_evolution.rs`(4) + 已注册进 `register_commands.rs`(6)** | **有完整功能，只缺表** ⇒ 引擎建表 = **修复「命令存在但表不存在」的断链**（纯收益） |
| `business_roles` | ❌ | **零**（仅 `lib.rs` 的 `pub mod` 声明） | 死实体 ⇒ 引擎会建 1 张空表 |
| `capability_clusters` | ❌ | **零**（9 处同名引用**全部**是 harness 自己的静态模块 `crate::capability_clusters`，与 entities 实体无关 —— 撞名掩盖） | 死实体 ⇒ 引擎会建 1 张空表 |

⇒ `business_roles` / `capability_clusters` 属 P0c「宿主未实例化」组的同类：**二选一（补 DAO 接线把功能接通 / 删该死链）**，按纪律不擅改，此处仅登记。

#### ④ 同时确认的三项正面证据

1. **`entity_modules!` 注册完整性 100%**（P0b 时点值）：`entities/src/lib.rs` 的 `pub mod` = **184**，目录 `.rs` 文件（除 lib）= **184**，其中有 `table_name` 的 = **184** ⇒ **零漏注册、零幽灵声明、零文件缺失**。（删 `divergence_logs` 后三数同步为 **183**，等式不变。）
2. **FK 30 条**与 P1 三方对账完全一致（迁移 30 = PG 30）。
3. `index_jobs` **98339 行**（第 2 大表）在实体覆盖范围内（交集 169 之一）。

#### ⑤ 本节自证踩到的两个口径坑（都会静默给错答案）

| 坑 | 实证 | 后果 |
|---|---|---|
| **`pg_stat_user_tables.n_live_tup` 是估算值，且在本库严重失准** | 报 0 行的有 ~146 张，而 `COUNT(*)` 后实际空表仅 **129** 张 ⇒ **`notes`(48590) / `note_backlinks`(74791) / `note_links`(74791) 被误报为空表** | 会得出「notes 表是空的、FK 影响面不存在」的**反向结论** |
| **`pg_stat_user_tables` / `pg_indexes` 不区分 schema** | `pg_stat_user_tables` 会把 `ax_backup` 的备份表混进「全库最大表」；`knowledge_entities_20260916` 冒充主表出现在榜首 | 「库里对象总数」与「public 表数」对不上 |

⇒ 一切行数与对象数判据**必须显式过滤 `table_schema='public'` + 用 `COUNT(*)`**。

#### ⑥ 结论

- **P0b 出口判据（差距清单可逐条归因）达成**：表级 15+15 条全部有归属，零「待归因」。
- **新增 2 项 P4 强制前置**（与 B 批同级、非可选）：① `gateway_message_queue` 实体迁入 entities crate → ✅ **已完成（2026-09-16 批 10）**；② 6 条 `tsvector` 列进 L2 例外清单 → 待 P2 写入 `extras.rs`。
- **P0a 处置表补 1 条漏项**：`opc_capability`（324 行存量；全仓仅 2 处引用，**均为注释**且内容正是「该表已移除」⇒ 零代码访问，交引擎清理）。
- **P0c 待办组新增 2 项**：`business_roles` / `capability_clusters` 两个零消费实体的二选一处置。

---

## 六、代码结构

```
src-tauri/crates/dao/src/
├── migrations/                    # ← 目标：整个目录删除
└── reconcile/                     # 新架构
    ├── mod.rs                     # run(db)：编排 + 锁 + 熔断（P4）
    ├── model.rs                   # 中立模型 SchemaModel/Table/Column/Index/Fk/Check
    ├── expected.rs                # ⭐ L1 实体 + L2 声明 → 期望 SchemaModel（P3 新增）
    ├── fingerprint.rs             # B 计算（含 engine_rev / dialect / extras）
    ├── introspect/
    │   ├── mod.rs                 # trait Introspector { fn read(&self) -> SchemaModel }
    │   ├── parse.rs               # ⭐ DDL 微解析（配平括号 + 引号状态机；PG/SQLite 共用）
    │   ├── pg.rs                  # pg_catalog / information_schema
    │   └── sqlite.rs              # pragma table_xinfo / index_list / index_xinfo / foreign_key_list
    ├── plan.rs                    # diff → Vec<Change> + 拓扑排序 + 分组（P3 已落地核心）
    ├── render/                    # ← 仅 P4 存在，P3 刻意没有它（= 结构上无法执行）
    │   ├── mod.rs                 # trait Renderer
    │   ├── pg.rs
    │   └── sqlite.rs              # 含 rebuild_table()（§三·一）
    ├── extras.rs                  # L2 声明式例外清单（68 项，见 §五·三）
    ├── rules/
    │   ├── mod.rs                 # Rule 注册表
    │   └── *.rs                   # L3 程序式规则（值域改名/数据搬迁/去重）
    └── safety.rs                  # 墓碑 / 审计 / 快照 / 熔断 / 逃生阀
```

**P3 的只读保证是结构性的**：`render/` 目录在 P3 **不存在**，
`reconcile` 里没有任何能把 `Change` 变成 DDL 文本的代码路径。
出口判据由 `crates/dao/examples/p3_plan_probe.rs` 回答（三种模式，见 §九·5）。

**L2 声明形态**（编译期可校验，拼错表名/列名靠 `Entity::Column` 关联）：

> ⚠ **本节形态已按 P2 实测更正**（2026-09-16）。草案原版有 3 处与实际需求不符，
> 每一处都是实测逼出来的，不是设计偏好：① 原 `cols: &[&str]` **表达不了排序方向** ——
> 库里有 5 条多列索引带 `DESC`（如 `idx_index_jobs_status (status, priority DESC, created_at)`），
> 丢掉方向会让 introspect 判「索引不一致」并反复重建；② 原类型集**没有函数** ——
> `ax_cjk_ngram` 是 `v227` 生成列表达式的前置依赖，无声明通道；③ 原类型集**没有动态表名** ——
> `vec_*_meta` 每集合一张，无法枚举成静态条目（详见 §五·三·补）。
> 最终形态与逐条内容以 `src-tauri/crates/dao/src/reconcile/extras.rs` 为准。

```rust
pub enum Dialect { Sqlite, Postgres }

pub struct IndexDecl {
    pub table: &'static str, pub name: &'static str,
    pub cols: &'static [&'static str],          // 允许 "col DESC"（实测 5 条带方向）
    pub unique: bool,
    pub where_clause: Option<&'static str>,     // partial；含外括号，对齐 pg_get_indexdef
    pub method: Option<&'static str>,           // Some("gin")；None = 默认 B-tree
    pub dialect: Option<Dialect>,               // None = 双方言
}
pub struct CheckDecl { pub table: &'static str, pub name: &'static str, pub expr: &'static str, pub dialect: Option<Dialect> }
pub struct GeneratedColDecl { pub table: &'static str, pub col: &'static str, pub col_type: &'static str, pub expr: &'static str, pub stored: bool, pub dialect: Option<Dialect> }
pub struct TriggerDecl { pub name: &'static str, pub table: &'static str, pub dialect: Option<Dialect>, pub sql: &'static str }
pub struct VirtualTableDecl { pub name: &'static str, pub create_sql: &'static str, pub sync_triggers: &'static [&'static str], pub dialect: Option<Dialect> }
/// 自定义函数：`render` 是**函数指针**而非字符串 —— SQL 含 __CJK__ / __SEP__ 占位符，
/// 必须与 search::text_ngram 的 is_cjk / is_separator 逐范围一致（不一致 = 静默失配），
/// 故只允许一处渲染逻辑，不允许在 L2 里再抄一份。
pub struct FunctionDecl { pub name: &'static str, pub args: &'static str, pub dialect: Option<Dialect>, pub render: fn() -> String }
/// 动态表名的 tsvector 声明（`^vec_.*_meta$`）。
pub struct PatternTsvDecl { pub table_prefix: &'static str, pub table_suffix: &'static str, pub column: &'static str, pub source_columns: &'static [&'static str], pub index_name: fn(&str) -> String, pub dialect: Option<Dialect> }
pub struct TableRenameDecl { pub from: &'static str, pub to: &'static str }
pub struct OrphanWhitelist { pub table: &'static str, pub reason: &'static str }
```

**L2 清单的守卫**（`extras.rs` 内 `#[cfg(test)] mod counts`）：逐类锁定条目数
（5/6/6/1/1/3/6/**35**/5 = **68**）+ 形态断言（GIN 必带 `method`、多列必 ≥2 列、
partial 必带 WHERE 且 `cols` 不含谓词）+ **反回流断言**（已 DROP 的
`idx_traj_memories_tsv` / `idx_opc_demand_leads_dedupe` / `trajectory_memories_fts`
不得再次出现，**无实体表上的索引也不得进 L2** —— 见 §五·三 的
`idx_chat_run_events_run_id`）。**这条守卫不是可选测试** —— L2 失效是静默的：少一条 ⇒ 引擎 DROP
该对象（`notes.tsv` 一条涉 48590 行，且 CJK 检索整体失效而无任何报错）。

**L3 规则形态**：

```rust
pub struct Rule {
    pub id: &'static str,
    pub applies: fn(&SchemaModel) -> bool,                                  // 状态判据
    pub run: fn(DatabaseConnection) -> BoxFuture<'static, Result<bool, DbErr>>,  // 返回是否发生变更
}
```

---

## 七、落地路径

| 阶段 | 内容 | 出口判据 |
|---|---|---|
| **P0a** ✅ **已完成**（2026-09-16） | **离线静态审计**：全仓 1985 个 `.rs` 扫这 12 张孤儿表的读写路径，逐张定性为「零访问 → 不补实体」/「原生 SQL → 补实体 + 改造」/「早已淘汰 → 不动」 | ✅ **12 张全部有裁决结论**（见 §五·一）；4 张已补实体并改造，**本轮零新增迁移文件**，5 张零访问表不补实体 |
| **P0b** ✅ **已完成**（2026-09-16） | 连**生产 PG** 拉 `information_schema` + `pg_catalog`，与实体元数据全量 diff（见 **§五·五**）。四项原定目标全部有实测结论：① A 批 5 张**全 0 行** ⇒ 不补实体零损失；② `cron_jobs` **2 行**（活跃）/ 补实体 4 张仅 `wiki_graph_cache` 有数据；③ **危险类型错配 = 0**（6 条 `Json`/`text` 差异的列全 `nonNULL=0`）；④ 库 **184** / 实体 **184** / 交集 **169** / 两侧各缺 15 | ✅ 差距清单 **100% 逐条归因**（表级 15+15 全有归属，零待归因）；另新增 3 项发现（盲区 1 / tsvector 列 6 / 零迁移实体 3） |
| **P0c** ✅ **已完成**（2026-09-16，①②③④ 全组处置完毕） | **按用户原则改造全部非实体访问**（§五·四 待处置清单 4 组）：① **主库 2 张**（`cron_jobs` / `cron_job_history`）→ ✅ **已完成**（批 1：实体 + `create_table_from_entity` + 删除 5 条按 backend 分支的手写 SQL）；**①补 `gateway_message_queue`** → ✅ **已完成（2026-09-16，本轮）**：实体原定义在 `crates/runtime/src/persistent_queue.rs`（**在 entities crate 之外** ⇒ 不进 `entity_modules!` ⇒ 引擎看不见），已迁入 `axagent-entities::gateway_message_queue` 并补 2 个 `indexed`（对齐 `v200` 的 2 个索引），错位模块整体删除（其 `///` 注释实为 `persistent_runner` 的、系注释错位）；② **侧车库 6 张**（`index.db` 的 `file_index` + `ast_*`）→ ✅ **已完成**（实体就位 + `open_index_db` 走 `Database::connect` + `FileIndex::new`/`AstIndex::new` 全 async）；③ **宿主未实例化**（**全组处置完毕**）：`audit_log` ✅ 已接通（批 2，顺带修 2 个真 bug）；`divergence_logs` ✅ **整链删除**（裁决「没用就删了」）；`l2_*`(3) ✅ **拆成 2 留 1 删** —— `l2_search_results` / `l2_index_snapshots` **实测已真接线**（原判「零消费者」是我锚点选错，已更正），前者另修一个真缺陷（补 `#[sea_orm(unique)]` + `store_search_results` 改真 upsert）；`l2_summaries` ✅ **整链删除**（裁决「按建议处理」）；④ **向量表豁免** ✅ 已裁决。⚠ **`RuntimeOwnedTable` 声明通道已作废** | 引擎干跑（dry-run）在真实库副本上，孤儿清单**为空集**或仅含已裁决对象。**已达成的前置判据**：`table_name` 注解在 entities crate 内 **183** / 外 **0**（原 1 处越界已归零；删 `divergence_logs` 前 185、删 `l2_summaries` 前 184）；`entity_modules!` 注册清单 **183 = `pub mod` 183 = 有 `table_name` 的实体 183**（读 build.rs 产物实证，零漏注册零幽灵声明），且 `gateway_message_queue` 在册 |
| **P1** ✅ **已完成**（2026-09-16） | 补齐 L1：实体加 `indexed`（**198 个已落地**，103 文件，见 §五·二）；FK 核对（**三方对账**：迁移声明 30 = 真实 PG `pg_constraint` 30；**补 `on_delete` 13 条 + 补 `belongs_to` 14 条**，`belongs_to` 29→43，见 §五·二·补）；补 12 张表的实体（**4 张已于 P0a 完成，5 张判定为不补**） | 实体元数据 == 迁移 DDL：**索引侧脚本比对通过**；**FK 侧 30/30 逐条等价**（P4 替换零 DDL 变更）；余 14 条「实体有、迁移与 PG 都无」已**裁决「接受」**（定性为 P4 预期新增 FK，非缺口，不加 `skip_fk`）；⚠ 但其中 `knowledge_entities.knowledge_base_id` 实测 **51277/72816 = 70.4% 孤儿**（`merge_entities` 把哨兵值 `__merged__` 写进了外键列）⇒ **该条 P4 建不出来，已另案待裁决**（见 §五·二·补 ②） |
| **P2** ✅ **已完成**（2026-09-16） | 冻结 L2：⭐ **口径已从「迁移静态扫描」改为「生产库实态」**，最终 **68 条声明**写进 `src-tauri/crates/dao/src/reconcile/extras.rs`（+ `mod.rs`；未接线，纯声明 + 自检）。⚠ 原「43 项」与「72 项」**两个口径均已作废** —— 前者不可复现（漏 29 项），后者是「静态 DDL 条数」（含 5 条已删项、含 3 条重叠的历代替换、少 3 类对象）。明细+逐格归因见 **§五·三·补** | 三项出口判据全绿（`output/tmp-p2-exit-criteria.log`）：① **47 条索引与库实态逐字段吻合**（表名/列序/**DESC 方向**/unique/method/WHERE）；② 库侧 7 条非声明项**全部归因**（4 复合主键 ⇒ 实体表达 + 3 条 `idx_vec_*_meta_tsv` ⇒ `PATTERN_TSV` 动态覆盖）；③ 生成列 6 / CHECK 3 全绿。**并配运行期守卫** `#[cfg(test)] mod counts`（8 条测试全过）：逐类锁条目数（5+6+6+1+1+3+6+**35**+5 = **68**）+ 形态断言（GIN 必带 method、多列必 ≥2 列、partial 的 cols 不得混入谓词）+ **反回流断言**（`idx_traj_memories_tsv`/`idx_opc_demand_leads_dedupe`/`trajectory_memories_fts` 不得再出现；**无实体表上的索引也不得进 L2** —— P3 删掉的 `idx_chat_run_events_run_id`） |
| **P3** ✅ **已完成**（2026-09-16） | 实现引擎（introspect / model / fingerprint / plan）**只读**：能算出 plan 并打印，不执行。🔑 **起手前置已清（2026-09-16）**：`cargo test -p axagent-dao --lib` 曾长期 `EXIT=101`（3 条 `v229_*` 红），使该命令**不能当判据用**；更关键的是那三条里有一条是**唯一索引的唯一行为守卫**，而 P4–P6 的引擎恰恰要按声明重建索引。已修，P3 起手时基线 **203 passed / 0 failed / EXIT=0**（见下方前置说明） | ✅ **已完成（2026-09-16）**：两条空库判据通过 + 生产库 715 项变更 **100% 逐条归因**；出口暴露的 2 项 P4 阻断项**同轮裁决并落地** ⇒ 修后 **712 项**（§七·补二 ③‑补、§9.6）。空 SQLite 实测 `期望 183 / CREATE 183 / 孤儿 0 / 破坏性 0`（真连 `sqlite::memory:`）；空 PG 同值；生产库 `实况 185｜期望 183｜交集 171｜仅实况 14｜仅期望 12`、诊断 **32（修后 35）**。收尾基线 **265 passed / 0 failed**（fmt 后重跑）。**只读是结构性的**：`reconcile/` 下**没有** `render` 模块，生产代码里对库的调用仅 2 处且**都是 `query_all_raw`** ⇒ 代码路径上无法把 `Change` 变成 DDL。⚠ 本轮自查出 **2 条假判据**（「fmt 零差异」与「诊断 62 条」）—— 见 §9.7 |
| **P4** ⏳ **进行中**（2026-09-16 起） | 实现 render + apply（PG 优先）：`AX_SCHEMA_DRY_RUN` 默认开。**P4-1 ✅ 已完成**（2026-09-16）：render 的**输入契约** `ChangePayload`（25 处调用点）+ **声明完整性** `PatternTsvDecl.expr`（原缺口：生成列表达式未声明 ⇒ 渲染不出来），实测「载荷零影响」（生产库 plan 输出**逐字节一致**）⇒ 详见 **§十一**。✅ **P3 暴露的 2 项硬前置已清（2026-09-16）**：① 向量表豁免升格为**显式 L2 规则** `PATTERN_ORPHAN_EXEMPT`（`vec_` 前缀 + `exclude_suffixes: ["_dup_backup"]`），实测 `DROP TABLE 11 → 8`；② `opc_capability`（324 行）与 `vec_kb_…_meta_dup_backup`（24518 行）已**先导出并校验**（`output/backup-2026-09-16-schema-orphans/`）。⚠ **P4 自己仍必须做的**：**墓碑（导出成功才允许 DROP）+ 破坏性操作延迟一个周期执行** —— 这两条才是「不丢」的真正来源（§四·七·补），且 `OrphanWhitelist` 至今**仍是零实现**。详见 **§9.6** 与 **§八 风险 1d** | 生产库副本上跑 apply，结构与迁移结果逐项等价 |
| **P5** | 实现 SQLite 重建路径；双方言 CI 双跑 | 同一份实体在两种方言下产出的结构经 introspect 后指纹一致（除方言差异项） |
| **P6** | 切换：删除 `migrations/`、`axagent_schema_version` 表、`repair_schema`、`db_config` 的"修复 Schema"按钮 | 仓库 `glob **/v[0-9]*.rs` 命中 0 |

**P3 之前不允许改任何生产代码** —— 引擎写错一次就可能删库，先让它在只读模式下被验证足够多轮。

### 七·补 🔑 P3 起手前置：DAO 测试门禁恢复绿（2026-09-16）

**处置**：`cargo test -p axagent-dao --lib` 的 3 条 `v229_*` 红**已修**，现基线 **203 passed / 0 failed / `TEST_EXIT=0`**、`cargo fmt -p axagent-dao -- --check` 零差异、日志零 warning。

**为什么从「登记不代改」改判为「修」**（这是本节的核心，不是顺手的清洁工作）：

| 面 | 事实 |
|---|---|
| 门禁信噪比 | 该命令**恒定** `EXIT=101` ⇒ 不能再把 `EXIT=0` 当验收判据；每次必须记住「3 条已知红」。同一条失败已跨**三轮**各做一次归属取证（约 10h / 7h / 本轮），成本是重复发生的 |
| **守卫面真空（决定性）** | 三条中 `v229_unique_index_rejects_duplicate_seq_in_same_conversation` 是唯一索引 `uq_fleet_messages_convo_seq` 的**唯一行为守卫**（`reconcile/extras.rs` 只锁该索引的**声明形态**，验不了数据库是否真在执行它）。而 v229 文档称该索引是并发兜底的「**真正的兜底**」—— `seq` 由 `MAX(seq)+1` 分配，读与写**不在同一事务**，PG READ COMMITTED 下两个请求可读到同一 MAX。**P4–P6 的引擎恰恰要按声明重建索引** ⇒ 守卫失效期间，列序改错或整条删掉都**不会有任何测试红** |

**根因**（两跳回溯，非「搜项目里哪写着 PRAGMA」）：`v229` 测试的 `fresh_db()` 只跑 `v102`（建父**表**）而**从不 `INSERT` 父行**，三条用例却以 `fleet_id='f1'` / `'f2'` 插子行 ⇒ SQLite `code 787`。而 **SQLite 自身 FK 默认是关的**，本项目测试环境的 FK 由**第三方依赖默认值**提供：`sqlx-sqlite-0.9.0/src/options/mod.rs:185-187` 的 `SqliteConnectOptions::new()` 内 `pragmas.insert("foreign_keys", Some("ON"))`。

**改动面（收押在测试模块）**：`src-tauri/crates/dao/src/migrations/v229_create_fleet_messages.rs` 的 `#[cfg(test)] mod tests` —— ① `fresh_db()` 补播种 `fleets` 父行（f1/f2，覆盖「同舰队同会话 / 跨舰队 / 同舰队不同会话」三种断言形态）；② 新增 `v229_fk_enforcement_rejects_orphan_child_row`，把「FK 强制开启」这个**隐含前提变成可执行断言**（断言行为而非读 PRAGMA 值），专拦「关掉 FK 来修绿」这类**假修复** —— 那样以后再无任何用例能证明 `REFERENCES fleets(id)` 真的在生效。**DDL / `up()` / 生产代码一字未动**。

**并发归属与披露**：该文件属 **Fleet 群聊工作流**（另一会话），mtime `2026-09-15T11:35:23.150Z`（静止 12.6h）。依纪律「不代改他人文件」的**例外条款**（静止 + 纯 bug + 改动面收押）执行；为防对方整写覆盖，补丁另落 `output/fleet-v229-test-fk-patch.md`，改动前原文备份 `output/backup-2026-09-16-fleet/`（7680 B，sha256 `eea42e94…c9327`）。

⚠ **与 `PLAN-domain-single-source.md` §④ 的口径差异（该表那条 FK 归因不完整）**：它把 FK 来源写成「`dao/src/db.rs:65/559` 显式 `PRAGMA`」——**对生产库成立，对本测试不成立**（测试走 `Database::connect("sqlite::memory:")`，不经 `db.rs`）。两者修法互不通用：**只改 `db.rs` 修不好这三条测试**。我未改他人 PLAN，仅在此更正（判据 **#370**）。

> **为什么 P0a 可以先行**：它只动「已在实体体系之外的孤儿表」，与引擎本身无关，且**不依赖引擎存在**。补实体让这 4 张表进入真相源覆盖范围（这是引擎的前置条件）；不补实体的那 5 张则本来就会被引擎判为孤儿。**本轮全部改动在旧架构下同样正确，且不含任何 schema 变更** —— 未新增迁移、未改 `CURRENT_VERSION`（保持 233）、未 DROP 任何对象。唯一新增的运行时行为是 4 处持久化模块从原生 SQL 改为实体 API（逐条等价性见 §五·一 表格；其中 `semantic_cache` 的 `hit_count` 行为变更已单列标注）。

### 七·补二 ✅ P3 已完成（2026-09-16）：只读引擎落地 + 三条出口判据实跑

**交付物（全部为 P3 新增，未触碰任何既有生产代码）**：

| 层 | 文件 | 职责 |
|---|---|---|
| 模型 | `crates/dao/src/reconcile/model.rs` | `SchemaModel`/`TableModel`/`ColumnModel`/`IndexModel`/`FkModel`/`CheckModel` + `finalize()`（**唯一排序点**）+ 方言类型等价类 |
| 声明侧 | `crates/dao/src/reconcile/expected.rs` | 读 `entity_modules!` 遍历全部实体 → 用 `sea_orm::schema::Schema` 渲染出**期望模型**（`create_table_from_entity` / `create_index_from_entity` / `get_column_def`）+ 并入 L2 |
| 声明侧 | `crates/dao/src/reconcile/extras.rs` | L2 静态例外（**68 条**，本轮由 69 删 1） |
| 实况侧 | `crates/dao/src/reconcile/introspect/{mod,pg,sqlite,parse}.rs` | 真库 → 实况模型。PG 走 `pg_catalog`/`information_schema`，SQLite 走 `sqlite_master` + `pragma_*`，`parse.rs` 补 `sqlite_master.sql` 原文解析 |
| 判据 | `crates/dao/src/reconcile/fingerprint.rs` | 结构指纹（A/B 短路用） |
| 计划 | `crates/dao/src/reconcile/plan.rs` | `diff()` → `Plan{changes, advisories, n_destructive, n_safe}` + `render()`（**纯打印**） |
| 探针 | `crates/dao/examples/p3_plan_probe.rs` | 三模式只读出口判据：`<pg-url>` / `--empty-pg` / `--empty-sqlite` |

#### ① 「只读」如何自证（不是承诺，是取证）

| 面 | 取证 |
|---|---|
| **结构性** | `reconcile/` 下**不存在** `render` 模块（P4 才做）⇒ 没有把 `Change` 转成 DDL 的**代码路径** |
| **调用面** | `reconcile/` 生产代码里对库的调用**恰好 2 处**：`introspect/sqlite.rs:74`、`introspect/pg.rs:56`，**两处都是 `query_all_raw`**。全目录 `execute_unprepared` 命中 7 处，**全部落在 `#[cfg(test)] mod tests` 内**（`introspect/sqlite.rs` 测试自 462 行起、`introspect/mod.rs` 自 54 行起） |
| **文本面** | 非测试代码里的 `"CREATE/DROP/ALTER …"` 字面量只有两类：`extras.rs` 的 L2 **声明数据**（`create_sql` / `sql` 字段，是「期望侧」值、不经执行）与 `plan.rs` 的 `ChangeKind::as_str()` **显示名** |
| **探针面** | `p3_plan_probe` 只调 `introspect::read` / `expected::build` / `plan::diff` / `Plan::render` |

#### ② 两条空库判据：**通过**

```
== P3 plan · 空 SQLite（真实空库） ==        == P3 plan · 空 PG（模型等价式） ==
-- 前提实证：真连 sqlite::memory: 返回 0 张表 --
期望表数 = 183 / CREATE 183 / 孤儿 0 / 破坏性 0    期望表数 = 183 / CREATE 183 / 孤儿 0 / 破坏性 0
PROBE_EXIT=0                                     PROBE_EXIT=0
```

- 空 SQLite 是**实测**（真连一张空库、断言 `introspect` 返回 0 张表）；空 PG 是**由实测撑起的等价式**（`introspect` 在空 schema 上返回的正是空 `SchemaModel`）。
- 判据 `creates == want.tables.len() && orphans == 0 && p.n_destructive == 0` —— 三者同时成立才 `PROBE_EXIT=0`。

#### ③ 生产库只读：715 项变更**逐条归因完毕**（出口判据本体）

> **修后（2026-09-16 裁决落地后）为 712 项** —— 差异只有 `DROP TABLE 11 → 8`（豁免 3 张向量基表），其余分桶逐项不变。下表给的是**修前**全量归因，保留它是因为「712」的每一分项都建立在它之上。

```
实况指纹 : ded1ce7305b498db      期望指纹 : 551c97cfe1072b69
变更合计 : 715（破坏性 471 / 非破坏性 244）      ← 修前；修后 712（468 / 244）
  段0 : 194   段1 : 28   段2 : 12   段3 : 455   段4 : 3   段5 : 23
实况 185｜期望 183｜交集 171｜仅实况 14｜仅期望 12      诊断 32 条
```

> ⚠ **「诊断 62 条」是本行曾出现的错误数字，已订正为 32（修前）/ 35（修后）**。核实方法：`output/tmp-p3-probe-prod2.log` 里 `\b62\b` **零命中**；按标题行切段（`-- 诊断（只报告，不产生变更） --` L762 → `-- 表级对账 --` L796）实测 **32 条** = `pattern 3 + text-diff 29`。修后 `output/tmp-p3v2-probe-prod.log`（L756–792）= **35 条** = `pattern 3 + text-diff 29 + exempt 3`，差 3 正是新增的豁免留痕（自洽）。
>
> 这条订正值得单独记一笔：**报告中的每个数字都必须能从日志里复算出来**。「62」既不在探针输出里、也不在任何分桶求和里 —— 它是凭印象写下的。判据见 §9.5 的 **#386**。

**分桶合计 = 715（与探针恒等，零待归因）**：

| 条数 | kind | 破坏性 | 归因 |
|---|---|---|---|
| **409** | `DROP DEFAULT` | 是（不丢数据） | **L1 声明缺口**：实体未写 `default_value`，而迁移 DDL 写了。占比 **57%**，是最大噪声源。属可用「实体补 `default`」批量消掉的一类 |
| **194** | `RENAME INDEX` | 否 | **P1「全默认命名」裁决的一次性成本**：198 个 `indexed` 落地后引擎按 `idx-{表}-{列}`（连字符）命名，与库中历史 `idx_{表}_{列}`（下划线）不同名。列集/unique/method 逐项一致 ⇒ 引擎正确识别为**改名而非删建**。⚠ 见 §五·二·补二 的命名不兼容裁决 |
| **31** | `ALTER TYPE` | **是（丢数据）** | `knowledge_attributes.{constraints,metadata,validation_rules}` 等列 `jsonb → json`。**实体声明 `Json` 而库是 `jsonb`**，属声明与实际的分歧，须裁决「改实体还是改库」 |
| **17** | `DROP INDEX` | 是（不丢数据） | 实况有 `idx_*_updated` / `idx_market_mainlines_strength` 这类 `DESC` 序索引，L1+L2 均无声明 ⇒ 引擎判孤儿。与后文 5 条 `CREATE INDEX` 是**同一批索引的命名/方向分歧**（`idx-conversations-updated_at` vs `idx_conversations_updated`） |
| **13** | `ADD FK` | 否 | P1 补的 `on_delete`/`belongs_to` 落地后的**预期新增**。其中 `knowledge_attributes.entity_id` 属 §五·二·补 ② 已登记的「P4 建不出来，待裁决」 |
| **12** | `CREATE TABLE` | 否 | **= 「仅期望」12 张**（恒等 ✓）：侧车库 6（`ast_*`×5 + `file_index`）+ 宿主零实例化 3（`l2_*`）+ 零迁移建表 3（`analyst_feedbacks`/`business_roles`/`capability_clusters`）。全部已在 §五·一/§五·四 裁决 |
| **11 → 8** | `DROP TABLE` | **是（丢数据）** | 修前 **= 「仅实况」14 − 3 张 `PATTERN_TSV` 运行时表**（恒等 ✓）；修后 **= 14 − 3（tsv）− 3（豁免的向量基表）**（恒等 ✓）。见下表与 §七·补二 ③‑补 |
| **7** | `DROP NOT NULL` | 否 | `opc_invoices.*` 实体未标 `NOT NULL` 而库有 ⇒ 同属 L1 声明缺口 |
| **5** | `DROP UNIQUE` | 否 | `opc_blog_posts.slug` / `opc_landing_pages.slug` / `sync_devices.unique_id` —— 实体缺 `#[sea_orm(unique)]`，同属 L1 缺口 |
| **5** | `CREATE INDEX` | 否 | 与上面 17 条 `DROP INDEX` 成对（命名归一后的新名） |
| **4** | `SET NOT NULL` | 否 | `memory_items.applicability_tags` 等实体标 `NOT NULL` 而库可空 ⇒ **反向**的 L1 缺口 |
| **4** | `SET DEFAULT` | 否 | 实体写了 default 而库无（`models.input_price_per_mtok` 等） |
| **3** | `DROP COLUMN` | **是（丢数据）** | `narrative_structures.{arcs,confluences,foreshadows}` —— 实体已无此三列 |

| 合计校验 | 值 |
|---|---|
| 修前 `471 + 244` | **715** ✓ |
| 修后 `468 + 244` | **712** ✓（豁免 3 张向量基表 ⇒ `DROP TABLE 11 → 8`、破坏性 `471 → 468`） |
| `loses_data 45` = `DROP TABLE 11 + DROP COLUMN 3 + ALTER TYPE 31` | **45** ✓（= 渲染区 `!!` 行数实测） |
| `destructive 471` = `45 + DROP INDEX 17 + DROP DEFAULT 409` | **471** ✓ |
| `CREATE TABLE 12` vs 「仅期望 12」· `DROP TABLE 11` vs 「仅实况 14 − 3 运行时表」 | 双侧恒等 ✓ |

**⚠ 叙事纠正（这是本节最该看的一行）**：`破坏性 471` 这个头条数字**不代表危险度** —— 其中 **409（87%）是 `DROP DEFAULT`**。按 `plan.rs` 的分级，真丢数据的只有 **45 项**（`!!` 标记），其余 426 项是「丢结构但不丢数据」。P4 的墓碑只需覆盖那 45 项。⇒ **报数时必须分开报「丢数据 N1 / 丢结构 N2」**（判据 **#380**；本轮自己先按 `!!` 读错了一次，量出 45 而实际是 471，差 10 倍）。

**「仅实况（将被 DROP）」14 张 · 行数已用真 `COUNT(*)` 钉死**：

| 表 | 列数 | **行数** | 归因 | 引擎是否判 DROP | 裁决状态 |
|---|---|---|---|---|---|
| `axagent_schema_version` | 3 | **74** | 迁移机制自己的版本表（74 行 = 74 个迁移，与 §9.3 恒等 ✓） | 是 | P6 随迁移机制删除（§五·一 已定案） |
| `chat_runs` | 12 | **0** | A 批：P0a 判零访问、不补实体 | 是 | ✅ 已裁决 |
| `chat_run_events` | 6 | **0** | 同上 | 是 | ✅ 已裁决 |
| `opc_demand_lead` | 31 | **0** | 同上 | 是 | ✅ 已裁决 |
| `opc_market_platform` | 10 | **0** | 同上 | 是 | ✅ 已裁决 |
| `rl_policies` | 11 | **0** | 同上 | 是 | ✅ 已裁决 |
| `opc_capability` | 13 | **324** | **有数据、无实体**（§五·五 已发现，本轮复测同值） | 是 | 🔴 **P4 前必须先处置**（324 行会丢） |
| `vec_kb_f9b2b050_…`（基表） | 2 | **0** | `vector_store` 动态表 `vec_{collection}` | 是 | 🔴 **豁免缺口**（见下） |
| `vec_wiki_dc8efbd8_…`（基表） | 2 | **49072** | 同上 | 是 | 🔴 **豁免缺口（49072 行）** |
| `vec_kb_…_meta_dup_backup` | 1 | **24518** | 某次备份/去重的残留，后缀不在任何模式内 | 是 | 🔴 **须补裁决（24518 行）** |
| `vec_capabilities` | 2 | **63** | 基表 `vec_{collection}`（`collection_id = capabilities`） | **是** | 🔴 **豁免缺口（63 行）** |
| `vec_capabilities_meta` | 6 | 2280 | 命中 `PATTERN_TSV` | **否**（豁免） | ✅ 已裁决 |
| `vec_kb_f9b2b050_…_meta` | 6 | 24516 | 命中 `PATTERN_TSV` | **否**（豁免） | ✅ 已裁决 |
| `vec_wiki_dc8efbd8_…_meta` | 6 | 49072 | 命中 `PATTERN_TSV` | **否**（豁免） | ✅ 已裁决 |

> **3 张 `PATTERN_TSV` 运行时表的区别**：`vec_capabilities_meta` / `vec_kb_…_meta` / `vec_wiki_…_meta` 出现在「仅实况」清单里，但被 `PATTERN_TSV` 动态覆盖 ⇒ **不判孤儿、不产生 `DROP TABLE`**（这正是 `DROP TABLE 11 = 14 − 3` 的来源，也解释了为什么表级对账的 14 与变更序列的 11 不相等）。

#### ③‑补 🔴 P3 出口暴露的 **P4 阻断项**：`vec_` 基表与 `_dup_backup` 残留**不在豁免内** — ✅ **已裁决并落地（2026-09-16）**，见本小节末

**声称 vs 代码事实**：§五·四 ④ 与 §五·四 表已把「向量表」裁决为**豁免**（保留原生 SQL，理由「表名运行时拼接，实体语法无法表达」）。但该豁免在**引擎侧的实现**只有一处 —— `extras.rs` 的 `PATTERN_TSV`，其匹配条件是 `table_prefix = "vec_"` **且** `table_suffix = "_meta"`。而 `expected.rs:1011-1019` 的测试**明确断言**基表与 `_dup_backup` **不匹配**：

```rust
assert!(!pattern_tsv_applies(d, "vec_kb_abc_meta_dup_backup", Dialect::Postgres));
assert!(!pattern_tsv_applies(d, "vec_collections", Dialect::Postgres));
```

⇒ **裁决的豁免范围（全部向量表）比引擎实际落实的范围（只有 `vec_*_meta`）小得多**。且 `PATTERN_TSV` 的**设计目的**本来是「动态表名的 tsvector 生成列」，**不是**「孤儿豁免」—— 它顺带产生的豁免是副作用。

**后果（修前：P4 apply 会真的删掉 73653 行向量数据）**：

| 将被 DROP 的表 | 行数 | 为什么没被豁免 |
|---|---|---|
| `vec_wiki_dc8efbd8_3a28_4fdf_9c96_9d26e2b2dac9` | **49072** | 基表，后缀不是 `_meta` |
| `vec_kb_f9b2b050_…_meta_dup_backup` | **24518** | 后缀是 `_meta_dup_backup`，模式要求**恰好** `_meta` 结尾 |
| `vec_capabilities` | 63 | 基表 |
| `vec_kb_f9b2b050_…` | 0 | 基表 |
| **合计** | **73653** | |

**并且当前没有任何别的机制能兜住**：`OrphanWhitelist` 在 §八 里被列为缓解措施，但**全仓 grep 零命中**（`crates/` 下 `whitelist` 的命中全是 bash sandbox / env 白名单，与 schema 无关）⇒ P4 一经 apply，这 4 张表无保护。

#### 裁决与落实（2026-09-16）

| # | 事项 | 裁决 | 落实点（fmt 后行号） |
|---|---|---|---|
| ① | **向量族豁免缺口** —— 3 张基表共 **49135 行**（`vec_wiki_…` 49072 + `vec_capabilities` 63 + `vec_kb_…` 0） | 采纳「首选」：把豁免从 `PATTERN_TSV` 的**副作用**升格为**显式 L2 规则** | `extras.rs:497` `ORPHAN_EXEMPT`（+ `:283` `OrphanExemptDecl`）· `expected.rs:130` `orphan_exempt_applies` · `plan.rs:345` `orphan_exempt_tables_of` + `plan.rs:396` `table_level` ②′ |
| ② | **`_dup_backup` 残留**（24518 行） | 「一次性 DROP + **先导出**」—— 不放宽模式保它命，它继续判孤儿 | `exclude_suffixes: &["_dup_backup"]` |
| ③ | **`opc_capability`**（324 行，同批无实体） | 同上：**先导出再 DROP**（不补实体） | 导出见下 |

⚠ **裁决 ① 与 ② 是同一条规则的两面**：豁免写宽（「`vec_` 前缀即豁免」）⇒ 垃圾残留被永久掩盖；写窄 ⇒ 向量数据被删。所以规则**必须**带显式 `exclude_suffixes`，且 `extras.rs::counts::orphan_exempt_rules_keep_explicit_exclusions` 会**拒绝空排除列表**（判据 #381 的机械化）。

> ⚠ **原「处置建议 1」里的「无后缀条件」是错的** —— 若照此实现，`_dup_backup` 会被宽模式一起豁免、永久留下。这是实施时才暴露的建议缺陷：**放宽匹配换省事，代价是把垃圾残留一起保下来**。

**导出证据（「导出成功才允许 DROP」的前提，不是事后补救）**：

| 表 | 库内 `COUNT(*)` | 导出文件行数 | 字节 | sha256(前 16) |
|---|---|---|---|---|
| `opc_capability` | 324 | **324 ✓** | 175320 | `bb7868d452bade5b` |
| `vec_kb_…_meta_dup_backup` | 24518 | **24518 ✓** | 735543 | `8396524dc854acc7` |

落盘 `output/backup-2026-09-16-schema-orphans/`；脚本 `output/tmp-p3v2-export.mjs`（只读：`SELECT` + `count(*)`）；报告 `output/tmp-p3v2-export.log`。校验方式是**读回文件 → `JSON.parse` → 比对条数**，不是「写成功即算数」。

**豁免的精确边界（修后，逐对象可对账）**：

- 豁免 = 名字以 `vec_` 开头 **且 不** 以 `_dup_backup` 结尾；
- 覆盖实况里的 **3 张基表**（`vec_capabilities` / `vec_kb_…` / `vec_wiki_…`）+ **3 张 `vec_*_meta`**（后者本就由 `PATTERN_TSV` 覆盖）；
- `_dup_backup` 与其余 7 张继续判孤儿 ⇒ **`DROP TABLE 8`**。

**修后实测（`output/tmp-p3v2-probe-prod.log`，`PROBE_EXIT=0`）**：

```
实况指纹 : ded1ce7305b498db      期望指纹 : 551c97cfe1072b69      ← 与修前逐位相同
变更合计 : 712（破坏性 468 / 非破坏性 244）
  段0 : 194   段1 : 25   段2 : 12   段3 : 455   段4 : 3   段5 : 23
实况 185｜期望 183｜交集 171｜仅实况 14｜仅期望 12      诊断 35 条
```

| 指标 | 修前 | 修后 | 恒等校验 |
|---|---|---|---|
| 变更合计 | 715 | **712** | −3 = 豁免的 3 张基表 ✓ |
| 破坏性 | 471 | **468** | −3 ✓ |
| `DROP TABLE` | 11 | **8** | `14 − 3(tsv) − 3(豁免)` ✓ |
| 段 1 条数 | 28 | **25** | −3 ✓ |
| 诊断 | 32 | **35** | +3 条 `exempt:` 留痕 ✓ |
| 两侧指纹 | — | **逐位不变** | 期望侧未动（只加了**判定策略**，不改期望对象） ✓ |

**豁免必须留痕**：被豁免的表在 plan 里**不产生任何变更**，若不打 advisory，下游无法从产出本身判断「它为什么没被 DROP」。故 `table_level` ②′ 对每条豁免打印 `exempt: 表 X 命中 L2 孤儿豁免（理由）`，实测 **3 条**（正好 3 张基表）。

**新增 4 条回归锁**（`cargo test -p axagent-dao --lib` 261 → 265）：`orphan_exempt_rules_keep_explicit_exclusions`（拒绝空排除）· `orphan_exempt_covers_vector_family_but_not_backup_residue`（族内豁免 + 残留不豁免）· `orphan_exempt_covers_every_pattern_tsv_match`（豁免范围必须覆盖 `PATTERN_TSV` 范围，否则 plan 自相矛盾）· `vector_family_is_orphan_exempt_but_backup_residue_is_not`（`plan.rs` 行为锁：只该有 2 个 `DROP`）。

> ✅ 这一条**不是 P3 的缺陷**：P3 的职责就是「算出 plan 并打印」，它**正确地把这件事暴露出来了** —— 这正是「引擎只读先跑」的价值。裁决后 **P4 前置已清**。

**诊断 32 条（修后 35 条）全部为设计内**：3 条 `pattern:` 声明 + **29** 条 `text-diff`（`CHECK` 的 `ARRAY[...]` 空格、`default` 的 `'x'::text` vs `'x'`、生成列的 `((a || ' ') || b)` vs `a || ' ' || b` 括号结合形态）。修后新增 3 条 `exempt:` ⇒ 35。全部符合 `plan.rs` 模块文档「**表达式文本一律不参与结构等价，只进 advisories**」的设计。

> ⚠ **本节曾写「62 条 = 3 条 `pattern:` + 59 条 `text-diff`」** —— 两个加数都是错的（实测 `3 + 29`），但**加和恰好自洽**，所以它读起来毫无破绽。这是判据 **#386** 的实例：**「加和自洽」只能排除算术错误，排除不了凭空取值**。凡汇总数字，必须给出「从哪条日志的哪一段数出来的」。

#### ④ 本轮修复的 15 条失败测试（P3 落地过程中显形，全部为**引擎自身的真缺陷**）

| 源 | 条数 | 根因 | 性质 |
|---|---|---|---|
| `expected::tests` | 4 | L2 声明 `idx_chat_run_events_run_id` 指向 **无实体的表** `chat_run_events` ⇒ `expected::build` 抛硬错误 | ✅ **规则正确工作的实证**（不是测试错）。处置：删该 L2 声明（69 → 68），交 §五·一 B 组孤儿判定，并加反回流断言 |
| `introspect::parse` | 5 | `Group` 下标基准错（整体左移 `on_pos + 3` 字节 ⇒ 产出 `tus ON public.index_jobs USING bt` 这种不可能列名）+ 无 `USING` 时表名区域未在列组处截断 + 锚点用 `find("INDEX ")` 而非**前缀匹配** | 真缺陷，静默给错答案 |
| sqlite introspect | 3 | `pragma_index_xinfo` 返回 `key=0` / `cid=-1` / `name=NULL` 的非键辅助行，未过滤 ⇒ 每个复合主键/UNIQUE 都误报「含表达式列」 | 真缺陷 |
| `model::tests` | 1 | SQLite 亲和性口径：`boolean` 被官方规则归 NUMERIC，而实体渲染 `BOOLEAN`、迁移写 `INTEGER` ⇒ 两边不同类 | 真缺陷（引入 `sqlite_class` 收拢） |
| `plan::tests` | 2 | ① 全局排序压掉了 `CreateTable` 的 **FK 拓扑序**；② 改名列同轮又补 `AddColumn` | 真缺陷（①判据 **#374**） |

**本轮判据编号（`mem.mjs add`，水位 373 → 388，共 15 条）**：#374 `sort_by` 稳定排序与拓扑序 · #375 声明域必须 == 实体覆盖域 · #376 `pragma_index_xinfo` 的 `key=0` 辅助行 · #377 `reltuples = -1` 不是 0 行 · #378 方言等价类须逐对显式 + `assert_ne!` · #379 `Group` 下标基准 · #380 破坏性两级与渲染标记 · #381 「声称的豁免范围」vs「代码落实的范围」 · #382 PG `"char"` 须 `::text` · **#383 判据时效性审计要机械执行 + 基准锚定**（本轮自查抓到 3 条判据作废，见 E17）· **#384 一条规则不得兼两项职责（副作用型豁免必须升格为显式声明）** · **#385 豁免必须带显式排除项，禁「前缀即豁免」** · **#386 门禁命令核退出码 + 汇总数字须注明来源段**（本轮两条假判据的直接产物，见 §9.7）· **#387 工具参数不得经 shell 变量中转** · **#388 被规则静默跳过的对象必须留痕**。

**收尾基线**：`cargo test -p axagent-dao --lib` = **265 passed / 0 failed / `TEST_EXIT=0`**（P3 起手 203 → 引擎收尾 261 → fmt 后 **265**），`cargo fmt -p axagent-dao -- --check` **零差异（`FMT_EXIT=0`，修后实测）**。

> ⚠ **本行原文曾写「261 passed … `cargo fmt --check` 零差异」，两句都不对**：261 是**早于 fmt** 的基线（fmt 后重跑为 265）；「fmt 零差异」是**假判据** —— 修前实测 `FMT_EXIT=1`、**9 个文件 / 47 处** `Diff in`。订正过程与根因见 **§9.7**（那 9 个文件全在 `reconcile/` 内，mtime `00:44–01:25` ⇒ 均系 P3 期间我自己产生的，无并发会话介入）。

> ⚠ **收尾基线为何是 261 而不是 260**：260 那一版是**在 `pg.rs` 的 `contype::text` 修复之前**跑的（`output/tmp-p3-test2.log`，mtime 早于 `pg.rs` 改动 **160 s**）。按纪律 **#175「日志 mtime 早于改动即作废」**，该判据连同两个空库探针一起作废，已全量重跑 ⇒ `output/tmp-p3-test3.log` / `tmp-p3-probe-empty-{sqlite,pg}2.log`。多出的那 1 条正是 `pg.rs` 新增的守卫 `char_typed_pg_columns_are_always_cast_in_select_list`。**这次是纪律 1 次生效、保住 1 条假判据的实例** —— 生产库那条 715 项归因（`tmp-p3-probe-prod2.log`，晚于 `pg.rs` 62 s）**本来就是有效的**，无需重跑。
>
> **3 个作废日志已按惯例清理**（`tmp-p3-test2.log` / `tmp-p3-probe-empty-sqlite.log` / `tmp-p3-probe-empty-pg.log`）—— 留着一个名叫 `test2` 的「260 passed」日志在旁边，正是下一次会话最可能误当基线的陷阱；作废事实与数值记录在本节与 E17。

#### ⑤ 真库才暴露的缺陷（**单测无 PG 连接 ⇒ 结构上测不出来**）

`contype` / `confdeltype` / `confupdtype` / `relkind` / `attgenerated` / `typtype` 等 `pg_catalog` 列在 PG 里是内部类型 `"char"`。`SELECT con.contype` 会被 sea-orm 解码为 `mismatched types; Rust type Option<String> (as SQL type TEXT) is not compatible with SQL type "CHAR"` ⇒ 生产库探针首跑 `PROBE_EXIT=2`。**全部改为 `::text` 投射**，并新增常驻守卫 `char_typed_pg_columns_are_always_cast_in_select_list`（扫 `pg.rs` 自身源码，含「守卫自身命中数 ≥ 3」的失效检测，防守卫本身烂掉）。⇒ 判据 **#382**。**这条缺陷的价值在于它证明了 P3 出口判据设计的必要性**：单测里没有任何 PG 连接，所以无论写多少条 `#[cfg(test)]` 都不可能测出它 —— **只有「拿真库跑一次」这一条路**。

---

## 八、剩余风险（诚实清单）

| # | 风险 | 性质 | 缓解 |
|---|---|---|---|
| **0** | 🔴 **主库内绕过实体体系的裸 SQL 建表**（§五·四）—— **2 张**（`cron_jobs` / `cron_job_history`，`runtime-core/src/cron_job.rs:37/54` 的 `execute_unprepared`）。它们在**主库**且活跃，引擎上线即判孤儿 DROP，业务代码再 `IF NOT EXISTS` 建回空表 ⇒ **每次启动静默清空定时任务** | **架构固有**（建表有 4 条路径，实体只管 1 条） | **补 2 个实体 + 改造 `cron_job.rs`**（强制前置，非可选）。⚠ 初版把这批误判为 12 张 —— 侧车库（`index.db`）与未实例化宿主**不在主库**，不在管辖范围 |
| 1 | **实体漏写 = 静默删表** | 信息缺失，无法从代码侧消除 | 墓碑（**导出成功才允许 DROP**）+ **延迟一个周期执行**（§四·七·补 条件②）+ 基数熔断 + `OrphanWhitelist` ⇒ 降为"可找回" |
| **1b** | 🔴 **`tsvector` 列实体未声明 = 静默删列**（§五·五 发现 2）—— 实测 **6 条**，其中 `notes.tsv` 一条涉 **48590 行**；引擎 DROP 列会**连带删 6 条 GIN 索引** ⇒ **CJK 全文检索整体失效**（`v227_cjk_fts` 的成果归零） | **实体语法固有**（`tsvector` + `USING GIN` + 生成列表达式均表达不了） | 6 条进 **L2 例外清单**（`reconcile/extras.rs`）—— L2 是「实体表达不了的 DDL」的唯一去处，P2 冻结时**不得遗漏**（清单最终 **68 项**，见 §五·三） |
| **1c** | 🔴 **实体定义在 entities crate 之外 ⇒ 引擎盲区**（§五·五 发现 1）—— `gateway_message_queue` 的实体在 `runtime/src/persistent_queue.rs:7` | 注册权威是 `entities/src/lib.rs` 的 `pub mod`（`dao/build.rs` 读它生成 `entity_modules!`）⇒ 注解写在别处即**不进引擎遍历集合** | 实体迁入 `entities/src/` + `lib.rs` 声明 → ✅ **已完成（2026-09-16 批 10）**：迁入 `axagent-entities::gateway_message_queue`（补 2 个 `indexed` 对齐 `v200`）、`runtime` 侧错位模块删除；验收 `table_name` 注解 **内 185 / 外 0** ✅。**建议加门禁：`table_name` 注解出现在 `entities/src` 之外即构建失败** |
| **1d** | ~~🔴 「向量表豁免」只落实了一半 ⇒ P4 会删掉 73653 行向量数据~~ ✅ **已裁决并落地（2026-09-16）**（**P3 出口新发现**，见 §七·补二 ③‑补）—— 发现时的事实：§五·四 已裁决「全部向量表豁免」，但引擎侧唯一的豁免通道 `PATTERN_TSV` 要求 `vec_` 前缀 **且** `_meta` 后缀，`expected.rs` 还**明确断言**基表与 `_dup_backup` 不匹配。修前将被 DROP：`vec_wiki_dc8efbd8_…`（**49072 行**）· `vec_kb_…_meta_dup_backup`（**24518 行**）· `vec_capabilities`（63 行）· `vec_kb_f9b2b050_…`（0 行）。且 `OrphanWhitelist`（本表第 1 行提到的缓解措施）**全仓 grep 零命中**，当时无任何机制兜住 | **「声称的豁免范围」比「代码落实的范围」大** —— 且 `PATTERN_TSV` 的设计目的是「动态表名的 tsvector 生成列」，**孤儿豁免只是它的副作用**，所以它的边界从来没按「孤儿判定」的需求审过 | ✅ **已落地**：① 豁免从副作用升格为**显式 L2 规则** `ORPHAN_EXEMPT`（`extras.rs:497`，含 `counts` 自检）；② **不采纳**初版建议的「无后缀条件」—— 改用 `exclude_suffixes: &["_dup_backup"]`，残留**继续判孤儿**（放宽模式会把真垃圾残留永久留下）；③ `_dup_backup`（24518）与 `opc_capability`（324）已**先导出并校验**（E19）。**修后实测 712 项 / `DROP TABLE 8`**、两侧指纹逐位不变（E20） |
| 2 | **`renamed_from` 漏写 = 旧列判孤儿删除** | 同上，但**可 fail-closed** | 引擎发现「库中旧列消失 + 实体新增列 + 无改名归因」时**拒绝执行该条**（§四·七·补 路径 2）⇒ **不丢** |
| 3 | `renamed_from` 走 column comment 字符串契约（`"{comment}; renamed_from \"{old}\""`） | 脆弱（官方实现如此：`ColumnDef.renamed_from` 字段 `pub(crate)`、**无 getter**，只有这一条通道） | **P3 已实现并逐字锁定**：`reconcile/expected.rs:renamed_from_of` 两条折法各有断言（`renamed_from_is_recovered_from_column_comment`）⇒ sea-orm 改折法时测试立刻红，而不是引擎静默退化成「DROP 旧列 + ADD 新列」。**仍建议**在 L1 之外提供直接声明入口（P4 决定是否引入 `ColumnRenameDecl`） |
| 4 | 大表重建耗时/锁（`notes` 48590 行） | 性能 | 指纹短路 + 异步 + 维护窗口 |
| 5 | 双方言语义漂移 | 结构性 | CI 双跑 + 指纹含 dialect |
| 6 | **引擎 bug 的爆炸半径远大于单个迁移** | 架构固有 | P3 只读验证期 + dry-run 逃生阀 + 审计全文可复现 |
| 7 | SQLite 无 `DROP COLUMN` 前的版本兼容 | 环境 | 明确最低 SQLite 版本（3.35+ 支持 `DROP COLUMN`，否则一律重建） |

**残留成本说明**：这不是"零维护"。L2 的 68 项声明、L3 的规则仍由人手写，性能上依赖指纹短路。真正的收益是：**新增字段/表/索引从"写迁移 + 升版本号"降级为"改实体"**，以及**漏执行不再永久错**（现状 `mod.rs:625-626` 自认这是"新装/升级不一致"类缺陷的根源）。

---

## 九、证据索引

### 9.1 本轮量化（对象覆盖率审计）

脚本：`output/tmp-reconcile-audit.mjs` → `output/tmp-reconcile-audit.log`（对象覆盖率）
　　　`output/tmp-orphan-access-audit.mjs` → `output/tmp-orphan-access.log`（12 张孤儿表的访问路径，P0a）
　　　**P2 双口径对账**：`output/tmp-p2-extras-survey.mjs` → `tmp-p2-extras-survey.{json,log}`（生产库侧：tsvector 列 / GIN / 生成列 / CHECK / 多列 / partial 全集）
　　　　　　　　　　　`output/tmp-p2-diff.mjs` → `tmp-p2-diff.log`（静态清单 ⟷ 库实态**双向** diff，逐条归因）
　　　　　　　　　　　`output/tmp-p2-exit-criteria.mjs` → `tmp-p2-exit-criteria.log`（**出口判据**：47 条索引逐字段吻合 + 库侧 7 条全归因 + 生成列/CHECK 对账）
　　　　　　　　　　　`output/tmp-p2-gen-idx.mjs` + `tmp-p2-assemble.mjs`（从 `pg_get_indexdef` 生成 47 条 `IndexDecl` 并拼装 `extras.rs`；手抄禁）
　　　`output/tmp-p0b-schema-diff.mjs` → `output/tmp-p0b-diff.log` + `output/tmp-p0b-report.json`（**P0b：库 × 实体全量 diff**）
　　　**P3 出口判据**：`src-tauri/crates/dao/examples/p3_plan_probe.rs`（三模式只读探针，判据本体）· `output/tmp-p3-prod-attr.mjs`（715 项分桶归因）· `output/tmp-p3-destr-count.mjs`（`!!`/`~` 两级标记实测）· `output/tmp-p3-l2-orphan.mjs`（L2 越界声明穷举）· `output/tmp-p3-cleanup.mjs`（临时产物清理 + 证据在位复核）—— 逐项结论见 **§九 9.5**

| 指标 | 审计时 | P0a 后 | **P0b 实测（2026-09-16，权威）** |
|---|---|---|---|
| 实体 `table_name` 声明 | 169 | 173 | **183**（**完整账**：P0b 时点 **184** → P0c 迁入 `gateway_message_queue` **+1** = 185 → 删 `divergence_logs` **−1** = 184 → 删 `l2_summaries` **−1** = **183**）＝ `lib.rs` `pub mod` **183** **交叉一致**（读 `build.rs` 产物 `entity_registry.rs` 实证） ⚠ 173→184 差额 11 无留存口径，不编归因。⚠ **本节上一版此格误写 183**（漏计 `gateway_message_queue` 的 +1，当时实测应为 184）—— 与本轮正确值 183 数值巧合，故特别标注以免掩盖该错误 |
| 库内 `public` 实表 | — | — | **184** |
| 实体 ∩ 库 | — | — | **168**（P0b 时点 169；−1 同上） |
| 库有表 · 实体无 | — | （静态文本层 8） | **15**：向量豁免 7 + 引擎版本表 1 + A 批 5 + `opc_capability` 1 + 盲区 1 |
| 实体有 · 库中无 | — | — | **13**：侧车库 6 + 原「宿主零实例化」组 **3**（P0b 时点 5 —— `divergence_logs`、`l2_summaries` 各 −1；余 3 张为 `audit_log` ✅ 已接通 + `l2_search_results` / `l2_index_snapshots` ✅ 已真接线，**该组名已失义**，见 §五·四 后注）+ 迁移未应用 1 + 零迁移实体 3 |
| DDL `CREATE TABLE`（**静态文本统计**） | 177 | 177 | 177（**迁移文件未删，其 DDL 文本永久存在** ⇒ 文本口径恒为 177，与库实态**必然脱节**） |
| DDL 虚表 | 6 | 6 | 6 |
| 库内列总数 | — | — | **2062**（实体未声明的列：**9**） |
| 库内索引总数 | — | — | **477**（跨 183 张表；另 1 张表零索引） |
| 库内 FK 约束 | — | — | **30**（跨 29 张表，与 P1 三方对账一致） |
| `CREATE INDEX`（迁移文本） | 179（plain 162 / multi 9 / partial 2 / GIN 6） | 179 | 179 |
| `CREATE TRIGGER` | 6 | 6 | 6 |
| 函数 / 视图 | 0 / 0 | 0 / 0 | 0 / 0（库内 `public` 视图实测 **0**） |
| 动态拼接建表 | 2（`{SCHEMA_VERSION_TABLE}`） | 2 | 2 |
| **危险类型错配**（实体 ↔ 库实际） | — | — | **0** |
| **引擎盲区**（`table_name` 注解在 entities crate 外） | — | — | **0**（`gateway_message_queue` 已于 P0c 迁入 entities）。P0b 时点为 **1**、全仓注解 185 = 184 + 1；删 `divergence_logs` 后 184 = 184 + 0；**删 `l2_summaries` 后现为 183 = 183 + 0** |

> ⚠ **「静态文本层 8」≠「库里还有 8 张孤儿表」**，两个口径必须分开报：
> - **3 张 `trajectory_*`** —— `v101` PHASE 9 已 DROP，**库里确实没有**（原 `dao/tests/pg_migrations.rs:239-241` 有断言；该文件已于 2026-09-16 退休，断言原文存于本文件 §五·一③ :508-512）。
> - **5 张零访问表**（`chat_runs` / `chat_run_events` / `rl_policies` / `opc_demand_lead` / `opc_market_platform`）—— 本轮判定为**不补实体**，**库里仍然存在**，待引擎孤儿清理或运维手工 DROP。
>
> 而**迁移文件不删、其建表 DDL 文本就永远在**，所以任何「扫迁移文本统计孤儿表」的脚本都会把这 8 张全算成孤儿。这正是 §四·九「孤儿判定必须由引擎 introspect 真实库、而非扫迁移文本」的实证 —— 按文本判会多删 3 张已不存在的表（无害），也会把「已人为接管」与「从未处理」混为一谈（有害）。

### 9.2 源码坐标

> **版本说明**：本节坐标已按项目**实际锁定版本**（`Cargo.lock`: `sea-orm 2.0.2` / `sea-orm-macros 2.0.2` / `sea-query 1.0.2`）逐条复核 —— 文档早前几轮的取证读的是 registry 里的 `2.0.1` 快照，2026-09-16 确认 2.0.2 下**全部特性结论不变**，且 `builder.rs` 中 `drop_column` / `drop_table` / `alter_column` 出现次数仍为 **0**（官方 sync 依旧不做破坏性变更）。

| 事实 | 位置 |
|---|---|
| `renamed_from` 属性解析 | `sea-orm-macros-2.0.2/src/derives/entity_model.rs:321-324` |
| `renamed_from` → `ColumnDef` | 同上 `:492-494` |
| `renamed_from` 编码进 comment | `sea-orm-2.0.2/src/schema/entity.rs:270-272` |
| 官方 sync 消费改名 | `sea-orm-2.0.2/src/schema/builder.rs:447`（`rename_column`） |
| 官方索引命名 `idx-{entity}-{col}` | `sea-orm-2.0.2/src/schema/entity.rs:158`、`:172`（连字符，与现有 179 个 `idx_` 不兼容） |
| `default_value` / `default_expr` 支持 | `sea-orm-macros-2.0.2/src/derives/entity_model.rs:253` |
| FK `on_delete` 支持 | `sea-orm-macros-2.0.2/src/derives/relation.rs:153-155` |
| `repair_schema` 无条件全量重跑 | `crates/dao/src/migrations/mod.rs:783` |
| `SimpleExpr = Expr` 类型别名（UPSERT 表达式可用） | `sea-query-1.0.2/src/expr/mod.rs:19` |
| `ExprTrait`（`add`/`gt`/`sum`）**不在** sea-orm prelude | `sea-query-1.0.2/src/expr/trait.rs:22`；须 `use sea_orm::sea_query::ExprTrait` |
| `ColumnTrait` 必须显式 import（`eq`/`is_null`/`in_subquery`） | `sea-orm-2.0.2/src/entity/column/mod.rs`（`bind_subquery_func!`） |
| `OnConflict::{value, values, expr}` | `sea-query-1.0.2/src/query/on_conflict.rs:456`、`:434`、`:148` |
| `PaginatorTrait::count` | `sea-orm-2.0.2/src/executor/paginator.rs:290` |
| 幂等硬约定 | 同上 `:549-552` |
| 中间版本号被永久静默跳过的缺陷自述 | 同上 `:625-626` |
| 约束死锁实录（必须分组执行） | `v232_opc_domain_pack_rename.rs:189-197` |
| 现有自愈补丁（待消灭） | `v109` / `v124` / `v125` / `v208` / `v223` |
| CREATE TABLE 时 FK 拓扑排序（官方做法参照） | `sea-orm-2.0.1/src/schema/builder.rs:284` |

### 9.3 迁移侧基线数据

| 指标 | 数值 |
|---|---|
| 迁移文件 | 74（`v100`–`v233`） |
| 生产代码行数（去掉 `#[cfg(test)]`） | 8613 |
| 带 `vXXX_is_self_idempotent` 测试 | 43 / 74 |
| 无测试但有幂等声明 | 16 |
| 既无测试也无声明 | 15 |
| `DEFAULT` 出现次数 | 515 |
| `REFERENCES`（FK） | 31 |
| `CHECK (...)` | 7 |
| `GENERATED ALWAYS` | 7 |

### 9.4 P3 起手前置的证据链（v229 门禁修复，2026-09-16）

| 编号 | 结论 | 证据（可复现） |
|---|---|---|
| E1 | 失败清单 = 3 条，同根因 `SQLite code 787` | `output/tmp-p2-test.log`：`v229_creates_table_and_unique_index` / `v229_unique_index_rejects_duplicate_seq_in_same_conversation` / `v229_same_seq_allowed_across_fleets_and_conversations`；`test result: FAILED. 199 passed; 3 failed`；panic 行号 `:114` / `:136` / `:161`。`v229_is_self_idempotent` 不插数据 ⇒ 幸免 |
| E2 | 根因 = `fresh_db()` 只建父**表**不播种父**行** | `v229:95-100`（只跑 `v102`）+ `v102_create_fleets.rs:35-47`（`fleets` 建表、**零 `INSERT`**）+ `v229:55` 的 `REFERENCES fleets(id)` |
| E3 | FK 在测试路径上是**开的**，来源是 sqlx 而非本项目 | `sqlx-sqlite-0.9.0/src/options/mod.rs:185-187`（`new()` 内 `pragmas.insert("foreign_keys", Some("ON"))`，注释「SQLx 选择默认开启，尽管 SQLite 通常为向后兼容而关闭」）。⚠ 测试走 `Database::connect` **不经 `dao/src/db.rs`** ⇒ 与 `PLAN-domain-single-source.md` §④ 的归因不同，修法也不同 |
| E4 | 同型现场全仓仅 2 处，另一处**未暴露** | 全 dao 迁移里带 `REFERENCES` 的表只有 `v204`（`paper_positions → paper_portfolios`）与 `v229`；`v204` 的测试只查 `sqlite_master`、不插数据 ⇒ 同一坑侥幸未暴露 |
| E5 | 修复后门禁恢复 | `output/tmp-fix-v229-test2.log`：`test result: ok. 203 passed; 0 failed`，5 条 `v229_*` 全 `ok`（含新增守卫 `v229_fk_enforcement_rejects_orphan_child_row`）；`TEST_EXIT=0`、`FMT_EXIT=0`、warning/error 行数 0 |
| E6 | 改动面收押 + 可回滚 | 备份 `output/backup-2026-09-16-fleet/v229_create_fleet_messages.rs`（7680 B / sha256 `eea42e94…c9327`）；补丁 `output/fleet-v229-test-fk-patch.md`；判据 #370–#373 |

**边界（未做）**：只跑了 `--lib` 目标；`dao/tests/schema_compliance.rs`（集成目标）**未跑**（可能依赖真库）。DDL / `up()` / 任一生产代码未动，`uq_fleet_messages_convo_seq` 的声明（`reconcile/extras.rs:501-507`）未动。

### 9.5 P3 出口判据的证据链（2026-09-16）

| 编号 | 结论 | 证据（可复现） |
|---|---|---|
| E7 | 只读引擎落地，编译与测试全绿 | **当前基线见 E18 / E21**（`output/tmp-p3v2-test2.log`：**265 passed / 0 failed**，fmt 后重跑）。本行原引 `output/tmp-p3-test3.log`（261 passed）**已作废并清理** —— 它早于本轮 fmt 改动；更早一代的 `tmp-p3-test2.log`（260 passed）因早于 `pg.rs` 改动而作废。基线演进：**203（起手）→ 261（引擎收尾）→ 265（fmt 后）**。作废事实见 E17 与 §9.7 第 3 行 |
| E8 | 首轮 15 条红全为**引擎真缺陷**（不是测试错） | `output/tmp-p3-test1.log`；逐条根因见 §七·补二 ④。判据：修的是 `parse.rs` / `sqlite.rs` / `model.rs` / `plan.rs`，**不是**改测试期望 |
| E9 | 空 SQLite 判据通过（**真连**空库） | `output/tmp-p3v2-probe-empty-sqlite.log`（**fmt 后重跑**，mtime 01:40）：日志内自带前提实证 `-- 前提实证：真连 sqlite::memory: introspect 返回 0 张表 --`；`期望表数 183 / CREATE 183 / 孤儿 0 / 破坏性 0`；`PROBE_EXIT=0`。⚠ 上一轮产物 `tmp-p3-probe-empty-sqlite2.log` **已作废**（早于 fmt 改动），见 §9.7 第 3 行 |
| E10 | 空 PG 判据通过（模型等价式） | `output/tmp-p3v2-probe-empty-pg.log`（**fmt 后重跑**，mtime 01:40）：`183 / 183 / 0 / 0`；`PROBE_EXIT=0`。⚠ 同上 |
| E11 | 生产库只读跑通，715 项逐条归因 | `output/tmp-p3-probe-prod2.log` + `.head`（`PROBE_EXIT=0`）；分桶脚本 `output/tmp-p3-prod-attr.mjs` → `.log` / `.head`（**分桶合计 = 715**，恒等） |
| E12 | `!!` 与 `~` 是两个不同量级（**纠正一处口径误读**） | `output/tmp-p3-destr-count.mjs` → `.log`：渲染区 `!!` 实测 **45 行** = `loses_data`（`DropTable 11 + DropColumn 3 + AlterColumnType 31`），而 `n_destructive 471` 还含 `~` 标记的 `DropIndex 17 + DropDefault 409`。`45 + 426 = 471` ✓、`471 + 244 = 715` ✓ |
| E13 | L2 越界声明复核：修前 1 条 → 修后 **0 条** | `output/tmp-p3-l2-orphan.mjs` → `.log` / `.out` |
| E14 | SQLite 后端渲染实态（`Decimal → real_decimal`、`Boolean → boolean`） | `output/tmp-p3-sqlite-render2.log`（**取证对象是依赖本体，不是本项目源码**）：`sea-query-1.0.2/src/backend/sqlite/table.rs:145`（`None => sql.write_str("real_decimal")`）· `:167`（`ColumnType::Boolean => sql.write_str("boolean")`）· `:197`（`fn integer`）。⚠ 初版日志锚点写错（按 `model.rs` 的 mtime 比）—— **依赖源码类事实的判据基准是依赖本体**（同族 #362：反向断言须锚定被测对象自身形态） |
| E15 | 🔴 **豁免缺口实测：14 张「仅实况」表逐张行数** | `scripts/pg-connect.mjs sql`（只读）→ `output/tmp-p3-drop-rows.log` + `tmp-p3-drop-rows2.log` + `tmp-p3-opc-exist.log`。关键值：`vec_wiki_dc8efbd8_…` **49072**、`vec_kb_…_meta_dup_backup` **24518**、`opc_capability` **324**、`axagent_schema_version` **74**、A 批 5 张全 **0** |
| E16 | 豁免边界的源码锚点（**行号已按 fmt 后实测订正**） | **修前**：`extras.rs:356-362`（`PATTERN_TSV` 的 `table_prefix: "vec_"` + `table_suffix: "_meta"`）· `expected.rs:541-542`（断言基表与 `_dup_backup` **不匹配**）· `vector_store.rs:200`（基表 `vec_{collection}`）· `OrphanWhitelist` 全仓 grep 零命中<br>**修后新增**：`extras.rs:150`（`OrphanExemptDecl`）· `extras.rs:374`（`PATTERN_ORPHAN_EXEMPT`）· `expected.rs:130`（`orphan_exempt_applies`）· `plan.rs:345`（`orphan_exempt_tables_of`）· `plan.rs:396`（`table_level` ②′）<br>⚠ **路径与行号两处订正**：`vector_store.rs` 实为 `src-tauri/crates/search/src/vector_store.rs`；`{name}_meta` 命名在 **`:344`**（旧记 `:235` 是错的）。`extras.rs:329-335` / `expected.rs:527-528` 也是旧值 —— 新代码插在前面导致整体后移 |

| E17 | **判据时效性审计**（纪律 #175 的机械执行） | `output/tmp-p3-mtime-check.mjs` → `.log`（源码 mtime vs 各判据日志 mtime，**一律 epoch 毫秒比较**）。结论：`prod2` / `prod-attr` / `l2-orphan` 三条**有效**；`test2` / `empty-sqlite` / `empty-pg` 三条**作废并已重跑**（见 E7/E9/E10）。⚠ 同批暴露**我自己的判据基准锚错**：`tmp-p3-sqlite-render.log` 的取证对象是 **sea-query 依赖源码**，却拿 `model.rs` 的 mtime 去比 ⇒ 已改用依赖本体重新取证（E14） |

### 9.6 裁决落地与收尾自查（2026-09-16，同轮）

| 编号 | 结论 | 证据（可复现） |
|---|---|---|
| E18 | 裁决① 落地：向量族豁免显式化，**4 条回归锁**（261 → **265** passed） | `output/tmp-p3v2-test2.log`（**fmt 后重跑**：`265 passed / 0 failed / TEST_EXIT=0`）；代码位置见 E16。四条测试：`orphan_exempt_rules_keep_explicit_exclusions` · `orphan_exempt_covers_vector_family_but_not_backup_residue` · `orphan_exempt_covers_every_pattern_tsv_match` · `vector_family_is_orphan_exempt_but_backup_residue_is_not`。⚠ 同轮产物 `tmp-p3v2-test1.log`（同为 265 passed，但**早于 fmt**）已作废清理 |
| E19 | 裁决②③ 落地：「导出成功才允许 DROP」的前提**已满足** | `output/tmp-p3v2-export.mjs` → `.log`：`opc_capability` **324/324**（175320 B，sha256 `bb7868d452bade5b`）· `vec_kb_…_meta_dup_backup` **24518/24518**（735543 B，sha256 `8396524dc854acc7`）。产物 `output/backup-2026-09-16-schema-orphans/`。校验 = 读回 + `JSON.parse` + 条数比对 |
| E20 | 生产库**修后**实测：豁免生效且未过宽 | `output/tmp-p3v2-prod-probe.mjs` → `output/tmp-p3v2-probe-prod.log`（`PROBE_EXIT=0`）：`715 → 712`、破坏性 `471 → 468`、`DROP TABLE 11 → 8`、段 1 `28 → 25`、诊断 `32 → 35`、**两侧指纹逐位不变**。分桶求和 `409+194+31+17+13+12+8+7+5+5+4+4+3 = 712` **恒等** ✓；`exempt:` 恰好 3 条（3 张基表）✓；`_dup_backup` **仍在** `DROP TABLE` 名单 ✓（裁决②的行为证据） |
| E21 | 🔴 **fmt 门禁的「零差异」曾是假判据** —— 已跑并修复 | `output/tmp-p3v2-fmt.log`（修前 `FMT_EXIT=1`，**9 个文件 / 47 处** `Diff in`）→ `output/tmp-p3v2-fmt-apply.log`（修后 `--check` **残留 Diff = 0**）。9 个文件**全部在 `reconcile/` 内**（+ 其 example），mtime `00:44–01:25` ⇒ 均系 P3 期间产生，**无并发会话介入**（行尾符普查亦为全 LF）。备份 `output/backup-2026-09-16-fmt/`（10 文件，可回滚）；语义未变的判据是 **fmt 后 `265 passed / 0 failed` 重跑**（`output/tmp-p3v2-test2.log`） |
| E22 | 报告数字订正：**「诊断 62 条」无据** | 真实 = 修前 **32**（`pattern 3 + text-diff 29`，`tmp-p3-probe-prod2.log` L762–795）/ 修后 **35**（+`exempt 3`，`tmp-p3v2-probe-prod.log` L756–792）。旧日志 `\b62\b` **零命中**；原文「3 + 59」两个加数皆错但加和自洽 ⇒ **加和自洽排除不了凭空取值**（判据 **#386**） |

### 9.7 本轮的自查（两条假判据 + 一条校验方法缺陷）

| # | 我声称过什么 | 事实 | 为什么会发生 | 机械化的防空手段 |
|---|---|---|---|---|
| 1 | P3 收尾「`cargo fmt -p axagent-dao -- --check` **零差异**」 | 修前实为 **9 文件 / 47 处差异**（`FMT_EXIT=1`） | `cargo fmt --check` 报差异时**既不打印 `error` 也不写 stderr**，只在 stdout 列 `Diff in …` 并以退出码 1 表达 ⇒ 「grep 输出里有没有 error」这种读法**结构性看不见它** | **门禁命令一律核退出码**，不看输出措辞；`FMT_EXIT` / `TEST_EXIT` 与结果同行落盘（判据 **#386** 同族） |
| 2 | 「生产库 advisories **62 条**」 | 修前 **32** / 修后 **35** | 数字是凭印象写的；「3 条 pattern + 59 条 text-diff」凑出了自洽的 62 ⇒ 自身**没有**暴露破绽 | 汇总数字必须注明「从哪条日志的哪一段数出来的」；写完后**复算一遍**（判据 **#386**） |

| 3 | （**不是**假判据，是**差点被我自己的校验脚本误杀**）`tmp-p3v2-fmt.log`（`FMT_EXIT=1`，修前证据）与 `tmp-p3-probe-empty-*-2.log`（早于 fmt 的空库探针） | 前者**必须**早于修复才有意义（它就是「修前有问题」的记录）；后者**必须**晚于修复才有效（已重跑，E9/E10） | **时效判据的「方向」取决于日志性质**：#383 只说了「日志要晚于改动」，但**诊断性日志的方位恰好相反** —— 一律按「晚于改动」判，会把有效的问题证据误判作废（第一版校验脚本确实报了 `✖ 作废 tmp-p3v2-fmt.log`） | 校验脚本按日志**性质**分两栏：**结果型**（测试/探针 ⇒ 须晚于改动）与**诊断型**（问题证据 ⇒ 须早于改动） |

> 三条的共同点：**判据的读取方式比判据本身更容易出错**。「零差异」是我读错了输出形态（漏看退出码），「62」是我没有读，第三条是我把「方位」写死成单一方向。故 #386 的要求是：**先固定「这个数字从哪个文件的哪一段来」，再确定它该早于还是晚于改动，最后才去读它。**

**边界（未做）**：未跑 `--workspace`（按纪律只跑改动 crate + 直接下游）；`dao/tests/schema_compliance.rs` 集成目标未跑；**未执行任何 DDL**，对生产库只有 SELECT。

---

---

## 十、三个问题的正面回答（本节的唯一目的）

| 你问的 | 回答 |
|---|---|
| **架构能不能实现** | **能**。§三 的 25 类 schema 变更 **0 项无解**。但"零手写迁移文件"≠"零手写代码"：L2 的 68 条例外声明（`extras.rs`）与 L3 的数据规则（`rules/*.rs`）仍需人手写 —— 它们**不是迁移文件**（无版本号、无 `up()` 时序、不参与"跑过就不跑"），但确实是手写代码。**若你的预期是"以后一行都不用写"，那个预期不成立。** |
| **会不会丢数据** | **会 —— 且比前一版判定更严重。** 三条独立成因：<br>① **结构层**：建表有 **4 条路径**（迁移 / 运行时自建 27 处 / 动态表名 3 处 / 实体派生 0 处），实体只管 1 条 ⇒ 实测 **17 张表无实体**（5 张零访问 + **12 张活跃表**）会被判孤儿。**其中 12 张活跃表会陷入"引擎删 → 业务建回空表 → 再删"的静默循环**（§四·九）<br>② **信息层**：路径 1（实体漏写）只能降级为"可找回"，**必须叠加"破坏性操作延迟一个周期执行"**<br>③ 路径 2/3/4 可由 fail-closed + 事务原子性**彻底消除**<br>⇒ **"不丢"不来自单点机制，来自"失败即停 + 期望侧完整"的组合。** |
| **如何实现** | §七 的 P0→P6。**P3 之前不碰生产代码**（引擎在只读模式被验证足够多轮）。当前的真正阻塞项只有两个：§五·二（索引注解，198 个已落地）、§五·三（L2 声明，**68 条已冻结并落地**），都是**体力活，无技术未知**。 |

### 需要你裁决的一处设计选择

§四·七·补 的**必要条件 ②「破坏性操作延迟一个周期执行」** — 它与"引擎全权"不冲突（全程无人干预），但有一个副作用：

> **一次 schema 变更需两次启动生效**（第一次标记 pending，第二次执行 DROP）。

| 选项 | 行为 | 不丢数据？ |
|---|---|---|
| A. 加延迟执行 | DROP 类操作跨两次启动生效 | ✅ 真不丢 |
| B. 不加，仅靠墓碑 | DROP 立即执行，数据进墓碑可找回 | ⚠️ 降为"可找回"，有不可逆窗口 |

**建议 A** —— 因为它覆盖的恰恰是"最危险的静默丢失"（实体漏写 = 整表消失），而代价只是删除类变更晚一次启动生效（新增/加列类**不受影响，仍然即时生效**）。

### 仍必须由人确认的最后一个事实 —— ✅ **已关闭（2026-09-16，P3 实测）**

§五·一 已定案（4 补实体 / 5 不补 / 3 不存在），无需再裁决。

**P0b 的四个数字已实测到手**（本节前版写「我未在生产库上执行任何查询」，现已补上 —— 走 `scripts/pg-connect.mjs sql` 只读查询，未执行任何 DDL）：

| 表 | 存量行数 | 结论 |
|---|---|---|
| `chat_runs` | **0** | ✅ 判 DROP 零数据损失 |
| `chat_run_events` | **0** | ✅ 同上 |
| `rl_policies` | **0** | ✅ 同上 |
| `opc_market_platform` | **0** | ✅ 同上 |
| `opc_demand_lead` | **0** | ✅ 同上（同批第 5 张） |
| `opc_capability` | **324** | 🔴 **不在这批四个数字里，但同样无实体 ⇒ 有真实数据会被 DROP**，须先处置（见 §八 风险 1d 同批） |

⇒ 「架构切换会不会丢数据」对 A 批 5 张的答案是 **不会（全 0 行）**；唯一有存量的是 `opc_capability`（324 行），以及 §七·补二 ③‑补 里那 73653 行向量数据。

**⚠ 顺带纠正一个测量口径**：`pg_class.reltuples` 对本库多张表返回 **`-1`**，含义是「**从未 ANALYZE**」而**不是「0 行」**。上面所有行数均由 `COUNT(*)` 实测，非 `reltuples`。

**两项裁决 —— ✅ 已关闭（2026-09-16）**（落实与证据见 §七·补二 ③‑补 / §9.6）：

| 曾待裁决 | 裁决 | 状态 |
|---|---|---|
| `opc_capability`（324 行）—— 补实体 / 先导出再 DROP / 保留？ | **先导出再 DROP**（不补实体）：它是 v100 期的能力目录，现由 `opc_capabilities` 族承载 | ✅ 导出 324/324 行已校验（E19），DROP 前提满足 |
| 向量表豁免范围 + `vec_kb_…_meta_dup_backup`（24518 行） | 豁免**显式化**为 `PATTERN_ORPHAN_EXEMPT`（`vec_` 前缀，排除 `_dup_backup`）；残留按「先导出再 DROP」 | ✅ `712` 实测（E20），`_dup_backup` 仍在 `DROP` 名单 ✓ |

---

## 十一、P4（**代码完成 · 真库已执行 1 次 · 声明侧已补齐 · 仍未接线**）：render + apply

> 进度：`十一·五` run3 真库执行 → `十一·六` 修 2 项 → `十一·八`（2026-09-16）落地两项裁决 +
> 修 3 处判据缺陷 + 补 392 条 L1 声明 + 第三轮只读复验（`DROP DEFAULT` 409→8、`DROP TABLE` 8→7）
> → **同日第二轮：6 项剩余裁决按「先分析建议值、再落地」全部完成**（见 `十一·八 ⑧`，第四轮只读复验
> `output/tmp-d9-pgrender.log`）：`DROP DEFAULT` **8→0**、`SET DEFAULT` **4→0**、期望表数 **183→175**、
> `DROP TABLE` **7 不变**、`CREATE TABLE` **0 不变**、渲染失败 **0**。
> 本轮**新登记 2 项**（`§⑧.6`：基数熔断缺「上一轮期望基数」的持久化端 ⇒ 该闸实际不生效；
> 8 张侧车库空壳仍在主库，待一次性清理）。

### 十一·一 ✅ 已完成（2026-09-16）：render 的**输入契约**与**声明完整性**

**为什么 P4 不能直接从「写 render」开始** —— 有两块前置在 P3 阶段是**结构上不可见**的，
上手才发现。它们的共同点：**只在「要执行」时才暴露，只在「只打印」时无害**。

| # | 缺口 | 为什么 P3 看不见 | 若照 P3 的状态直接写 render |
|---|---|---|---|
| 1 | `Change` **没有结构化参数** | P3 只打印：`object`（`表.列` / 索引名）+ `detail`（人读文本）就够 | DDL 要的是**定义本身**（类型/可空/默认/生成表达式/列集/访问方法/谓词）。从 `object` 反查有两处硬伤：① FK 的 object 是 `表.colA,colB`，**自带逗号**，与列级 `表.列` 不是同一套切分规则；② 反查要同时持有期望模型与实况模型 ⇒ `render` 不再是纯函数，无法单测 |
| 2 | `PatternTsvDecl` **未声明生成列表达式** | 同上 —— 该字段从未被读过 | `ADD COLUMN … GENERATED ALWAYS AS (<expr>)` **渲染不出来** ⇒ 只能 fail-closed 跳过 ⇒ 这条 L2 声明的效果**永远不会落地**，而 plan 里明明有一条 `AddColumn`（**静默空操作**） |

#### 处置 1：`ChangePayload` —— 载荷在产出点 clone（`plan.rs`，25 处调用点）

```rust
pub enum ChangePayload {
    Bare,                                       // DROP TABLE（object 已含全部信息）
    Rename { table: String, from: String, to: String },   // 列 / 索引 / FK 改名
    Column { table: String, col: ColumnModel }, // 建列取**期望侧**、删列取**实况侧**
    Index  { table: String, def: IndexModel },
    Fk     { table: String, def: FkModel },
    Check  { table: String, def: CheckModel },
    Table(TableModel),                          // CREATE TABLE
}
```

两条设计要点：

1. 载荷在**产出变更的那一刻**从模型 clone（那时信息就在手边）⇒ `render` 退化成
   `&[Change] -> Vec<Statement>`，可单测；
2. `Differ::push` 加第 4 个参数 ⇒ **漏给载荷 = 编译错误**。这是**借编译器穷举 25 处**，
   而不是靠人记住「还有哪几处没补」（判据 **#390**）。

> ⚠ `Rename.from` 允许是空串（实况侧匿名对象，SQLite 的匿名 UNIQUE / CHECK 就是这种）——
> 渲染器须据此改走「按定义重建」而非 `RENAME`，见 §十一·三等 render 落地时补。

#### 处置 2：`PatternTsvDecl.expr` —— 声明必须足以渲染（`extras.rs`）

补法与既有声明习惯**保持一致**（`GeneratedColDecl:349` 就是把表达式写死为字面量，
不引入函数指针）：

```rust
expr: "to_tsvector('simple', ax_cjk_ngram(COALESCE(content, '')))",   // 与 v227:217 输出逐字一致
```

并新增两个**渲染入口的构造器**——这就是「声明足以渲染」的**可执行判据**：

```rust
pub fn pattern_tsv_column_model(decl) -> ColumnModel       // tsvector + generated=Some(expr)
pub fn pattern_tsv_index_model(decl, table) -> IndexModel  // method="gin" + cols=[生成列]
```

⚠ 两处口径细节（都写成断言）：

| 字段 | 取值 | 为什么不能随手取 |
|---|---|---|
| `nullable` | **`true`** | `v227:153` 的 DDL **没写** `NOT NULL`。写成 `false` 会让 introspect 每轮判出一次可空性差异 ⇒ 引擎反复 `SET NOT NULL`，而**生成列不允许改可空性** ⇒ 直接报错 |
| 索引列 | **必须 == 生成列名** | 否则 GIN 建在一个不存在的列上 |

**新增 2 条测试**：`pattern_tsv_expr_covers_source_columns`（锁 `expr` 与 `source_columns`
的派生同步 —— 挡「加了源列忘改 expr」，后果是索引只覆盖一半内容且**不报错**）·
`pattern_tsv_models_are_renderable`（两个模型真能构造出来 + 形态与 v227 DDL 对齐）。

### 十一·二 出口判据（本轮实跑，2026-09-16）

| 判据 | 结果 | 证据 |
|---|---|---|
| 单测基线 | **267 passed / 0 failed**（265 → **+2**） | `output/tmp-p4-test1.log`（`TEST_EXIT=0`） |
| **载荷零影响**（反向断言） | 生产库 plan 输出**逐字节一致**：87427 字符 == 87427 字符 | P3 证据 `output/tmp-p3v2-probe-prod.log` **vs** 本次 `output/tmp-p4-probe-prod.log`（`PROBE_EXIT=0`）；比对脚本 `output/tmp-p4-verify-payload.mjs` · 探针脚本 `output/tmp-p4-probe-prod.mjs` · 载荷补齐脚本 `output/tmp-p4-payload.mjs` |
| 关键数字未变 | 指纹 `ded1ce7305b498db` / `551c97cfe1072b69` · **712**（破坏性 468 / 非破坏性 244）· 段 `194/25/12/455/3/23` · `exempt:` **3** 条 | 同上 |

> **为什么这条反向断言必须实测**：「载荷只加在 `Change` 上、不进 `diff` 与指纹」在推理上
> 显然，但**推理不算判据**（判据纪律：反向断言须锚定被测对象自身形态）。实测口径取
> **整份 plan 输出逐字节比**，远强于「比较若干计数」——后者对「某一段悄悄重排」是盲的。
>
> **比对时要剥掉 cargo 噪声行**：首轮比对报「差异 1 行」，实为 `Finished dev profile …in
> 1m 03s` vs `1m 01s`（编译耗时）。**同名判据若把编译耗时算进去，就永远无法通过**。

### 十一·三 剩余工作（**三块全部落地**）

| 批次 | 内容 | 状态 |
|---|---|---|
| P4-1 | render 的输入契约（`ChangePayload`）+ 声明完整性（`expr`） | ✅ **已完成**（见本节） |
| P4-2 | `render.rs`：21 个 `ChangeKind` → DDL（PG 优先，SQLite 留 P5）。含 `AlterGenerated` 展开为「DROP + ADD」（PG 不支持给已有列加 `GENERATED`）、`Rename` 空 `from` 的重建分支 | ✅ **已完成**（见十一·四） |
| P4-3 | `safety.rs`：墓碑（**导出成功才允许 DROP**）/ 审计 / 熔断 / 逃生阀 / `_ax_schema_pending_drops`（延迟一个周期）/ `OrphanWhitelist`（原「至今零实现」→ 读写两端齐） | ✅ **已完成**（见十一·四） |
| P4-4 | `apply.rs` 编排 + `p4_apply_probe.rs` 四模式；**不夹在启动路径里同步跑**（PLAN §五·三 执行约束），并加守卫测试钉死「未接线」 | ✅ **已完成**（见十一·四） |

### 十一·四 P4-2/3/4 落地与证据链（2026-09-16）

#### 新增文件

| 文件 | 规模 | 职责 |
|---|---|---|
| `reconcile/render.rs` | 21 类 → DDL 文本 | **纯函数**：`fn(&Change, Dialect) -> Result<Rendered, RenderError>`，不碰数据库 |
| `reconcile/safety.rs` | ~1150 行 / 21 测试 | 七道闸 + 四张元表簿记 |
| `reconcile/apply.rs` | ~1100 行 / 24 测试 | 编排（`run` / `cycle`）+ 结果模型 |
| `examples/p4_apply_probe.rs` | 四模式 | 空 SQLite / 空模型 / 真库只读 / 真库 apply |

#### 出口判据（实跑，全部可从日志复算）

| 判据 | 结果 | 证据 |
|---|---|---|
| 单测基线 | **347 passed / 0 failed**（346 → **+1**，新增 `plan::tests::change_kind_all_is_complete_and_unique`；差集已逐条核对，无消失用例） | `output/tmp-p4-test-v2.log`（`TEST_EXIT=0`） |
| `reconcile` 测试分布 | **151 条**：apply 24 · expected 9 · extras 11 · fingerprint 7 · introspect 25 · model 11 · plan 12 · render 27 · safety 21 · mod::tests 4 | 同上（Node 统计） |
| 探针构建 | `BUILD_EXIT=0`（47.20s，无 warning） | `output/tmp-p4-build-v2.log` |
| 空 SQLite 冒烟 | **8/8 判据 ✓**，`EXIT=0`：渲染零失败 · `executed=0` · `audit_rows=183 == 变更 183` · 四张元表已建出 · introspect 可见表 `[]` · 两遍 `cycle` 逐字段一致 · `converged=None` | `output/tmp-p4-probe-empty-sqlite.log` |
| 空模型渲染覆盖 | `EXIT=0`：`CREATE TABLE 183 / 0 失败`；**只触及 1 / 21 类**（已显式打印未触及的 20 类） | `output/tmp-p4-probe-render-cov.log` |
| **真库只读渲染**（零写入） | `EXIT=0`：**185 张实况表** · 指纹 `ded1ce7305b498db` / `551c97cfe1072b69` · **712 变更**（破坏性 468 / 非破坏性 244）· **渲染 0 失败** · 期望表数 183 · 孤儿候选 28 · 丢数据 42 · `DROP TABLE` 8 | `output/tmp-p4-probe-pg-render.log`（822 行） |
| 驱动总退出码 | `[0,0,0]` → `DRIVER_EXIT=0` | `output/tmp-p4-run-probes.log` |

#### ⚠ 「21 类都验证过」是**假的** —— 只到 13 类

把三份 plan 的类别并起来（空模型 1 类 ⊂ 真库 13 类）：

```
真库产出：ADD FK · ALTER TYPE · CREATE INDEX · CREATE TABLE · DROP COLUMN · DROP DEFAULT
         DROP INDEX · DROP NOT NULL · DROP TABLE · DROP UNIQUE · RENAME INDEX
         SET DEFAULT · SET NOT NULL            = 13 类
未触及  ：ADD CHECK · ADD COLUMN · ADD UNIQUE · ALTER GENERATED
         DROP CHECK · DROP FK · RENAME COLUMN · RENAME FK   =  8 类
                                              13 + 8 = 21 ✓
```

**这 8 类目前只有单测穷举用例（构造的 `Change`）覆盖，没有任何真实 plan 产出过它们。**
且方向一致 —— 未触及的全是「**声明比实况更严**」那一侧（ADD COLUMN / ADD UNIQUE / ADD CHECK /
ADD FK 之外的新增类），而真库产出的是「**声明比实况更弱**」那一侧（409 条 DROP DEFAULT）。
两边互相印证，不是巧合。**上线的第一批真实变更必然落在这 8 类里**，那才是它们的首次验证。

其余三个由实测发现的要点：

| 现象 | 判定 |
|---|---|
| 409 条 `DROP DEFAULT` 的右端是 **`-`**（期望无 default），非文本形态差异 | 与 §九 1343 行「L1 声明缺口」归因**一致**；曾怀疑是 `'active'::text` vs `'active'` 的归一化误判，做交集核查后**证伪**：22 条 default 形态差异全在 `text-diff` advisory 里、**与 DROP DEFAULT 交集 = 0**，即顾问输出与计划判定**没有互相矛盾** |
| 194 条 `RENAME INDEX`（`idx_x_y` → `idx-x-y`） | 命名法规格化的**真实差异**，需产品语义决策（接受改名 or 补豁免），非引擎缺陷 |
| 渲染 0 失败 / 712 条全渲染成功 | 渲染层在真实规模下可用 |

#### 五条顺序决策（写进 `apply.rs` 模块文档，此处指路）

① `ensure_meta_tables` 最先 · ② 白名单在 diff **之前**读（否则豁免不生效） ·
③ **全部渲染完**再执行（否则半同步库）· ④ 墓碑在 `DROP` **之前**写 ·
⑤ 审计在**执行之后**写。

审计与墓碑的**分工**是这里唯一不显然的地方：审计在后写 ⇒ 存在「DDL 已生效而审计缺行」的窗口，
但对**不可逆那一类窗口是关闭的**（`DROP TABLE` 前必落墓碑）。所以修法不是「把审计提前」，
而是「**把墓碑提前**」。

两条实测踩出来的语义（都已加测试钉住）：

- **dry-run 不消费「延迟一周期」令牌** —— 否则「先 dry-run 看一眼再真跑」会让真跑当场执行破坏性
  变更，**一次观察行动把延迟闸吃掉**。dry-run 只**报告**「若真跑，本条会被延迟」。
- **不包整轮事务**，每条语句各自提交 —— 墓碑必须能被后续独立查询看到；放进同一回滚单元会让
  `DROP` 失败时墓碑一并消失。正确性改由 **审计 + 墓碑 + fail-stop** 承担。

#### 上线前置（**阻断项，不是 P4 的缺陷**）

`--execute` **一次都没跑过**，四张元表**尚未在生产库创建**。原因不是闸不够，而是：
真跑会执行 **468 条破坏性变更**，其中 **409 条**源于「L1 实体未写 `default_value`」——
引擎会仅因为**声明里没读到**就把库里 409 个列的默认值删掉。

⇒ **顺序必须是「先补 L1 声明，再看计划」，不是「先跑引擎，再看它删了什么」。**
这与 §九 的结论是同一条，此处只是有了执行器后的量化确认。

### 十一·五 首次真库执行（2026-09-16，用户授权）

> ⚠ 上一节（十一·四）的「上线前置」写于**真跑之前**。用户随后授权在生产库跑一次，
> 故此处**更正**：`--execute` 已跑过一次，四张元表已在生产库创建。下面全部是实测。

#### 跑了三次，前两次被闸拦住（这正是闸存在的意义）

| # | 配置 | 结果 | 证据 |
|---|---|---|---|
| 1 | `--execute`，默认配额 | `[circuit] 破坏性变更 468 条，超过单轮配额 5 条` ⇒ **执行 0** | `output/tmp-p4-exec-run1.log` |
| 2 | 同上 + `AX_SCHEMA_MAX_DESTRUCTIVE=100000` | `[evidence] 42 条会丢数据的变更缺少导出证据` ⇒ **执行 0** | `output/tmp-p4-exec-run2.log` |
| 3 | 同上 + `--export-evidence` | **执行 242｜延迟 8｜失败 1｜中止 461** | `output/tmp-p4-exec-run3.log` |

第 2 跑暴露的那个缺口是**真缺口**，不是配置问题：
**「导出成功才允许 DROP」当时只有强制端、没有产出端** —— 全项目没有任何工具能产出
`GraveEvidence`，引擎**永远无法执行任何丢数据的变更**。修法见十一·六第 1 条。

#### 第 3 跑的执行结果（净效果 = 零数据丢失）

```
已执行 242 = RENAME INDEX 194 · DROP INDEX 17 · CREATE TABLE 12 · ALTER TYPE 19
延迟     8 = DROP TABLE 8 张        ← 闸 5 生效，一张都没删
失败     1 = narrative_structures.is_template 的 ALTER TYPE
中止   461 = 后续全部（含 409 条 DROP DEFAULT、3 条 DROP COLUMN —— 都没执行）
```

**没有任何丢数据动作被执行过**：8 张表靠闸 5 延迟、409 条 DROP DEFAULT 与 3 条 DROP COLUMN
因为排在失败点之后被 fail-stop 拦住。**fail-stop 在这里是净收益**。

#### 收敛证据（+242 / −242 逐项对齐）

| 判据 | 执行前 | 执行后 |
|---|---|---|
| 实况表数 | 185 | **197**（+12 = CREATE TABLE 12） |
| 变更合计 | 712 | **470**（−242） |
| 破坏性 / 非破坏性 | 468 / 244 | 432 / **38**（−36 / −206） |
| RENAME INDEX · DROP INDEX · CREATE TABLE | 194 · 17 · 12 | **全部 0** |
| ALTER TYPE | 31 | **12**（−19 = 那 19 条真的成功了） |
| 段0 · 段2 | 194 · 12 | **整段消失** |

证据：`output/tmp-p4-probe-pg-render.log`（执行前）vs `tmp-p4-probe-pg-render-after.log`（执行后）。

> 顺带核对一处**看着像不自洽但实际自洽**的数字：执行后实况 197、期望 183、`CREATE TABLE 0`、
> `DROP TABLE 8`，四者差 6。差的那 6 张是**被静态规则豁免的表**（`PATTERN_ORPHAN_EXEMPT`）——
> 它们既不在期望里、也不进孤儿候选，所以两个方向都不出现。
> 这类「差一笔」必须**当场算清**，否则它会在下一次被当成引擎漏判。

#### 备份与回滚路径（真跑前已就位）

| 产物 | 大小 | 用途 |
|---|---|---|
| `output/backup-2026-09-16/axinvest-before-p4-apply.dump` | 352.8 MB | `pg_dump -Fc` 全库，选择性恢复 |
| `output/backup-2026-09-16/metadata-rollback.sql` | 328 KB | 475 DEFAULT + 1499 NOT NULL + 482 索引 + 1702 约束的**可重放**反向语句 |
| `output/backup-2026-09-16/tables/*.json` + `_manifest.json` | 8.7 MB | 42 个待删对象的全量数据 + sha256 |

### 十一·六 首次真库执行暴露的问题与修复（3 条）

#### 1. ✅ 真缺口：「导出成功才允许 DROP」缺**产出端** → 新增 `reconcile/evidence.rs`

`ApplyOptions::evidence` 一直是空的，因为没有工具能产出它。新增模块（~460 行 / 16 测试）：

| API | 职责 |
|---|---|
| `target_of(&Change)` | **纯函数**：`DropTable` ⇒ 整表 / `DropColumn`、`AlterColumnType` ⇒ 该列；其余 18 类 ⇒ `NotDataLosing` |
| `export_sql` / `count_sql` | PG 专有（`to_jsonb(...)::text`，输出 JSON Lines）；SQLite ⇒ `UnsupportedDialect`，与 `render` 同源 |
| `evidence_path` | ⚠ **路径穿越拦截点**：object 来自数据库标识符，PG 允许 `"a/b"`、`".."` ⇒ 只留 `[A-Za-z0-9_-]` |
| `verify_export` | **回读文件字节**算 sha256 + 数行数 + 与 `count(*)` 比对，不等即报错 |

关键判据 `target_of_agrees_with_loses_data_on_all_kinds` 遍历 `ChangeKind::ALL` 断言
`target_of(k).is_ok() == k.loses_data()` —— 将来新增一个 `loses_data` 类别而忘了教导出器时
**当场变红**，而不是等真跑那天被 `missing_evidence` 报「缺证据」让人以为「只是没导出」。

实测：42 个对象 / **687824 行 / 8.7 MB**，全部通过完整性校验。

#### 2. ✅ 实测缺陷：`ALTER COLUMN … TYPE` 前必须先 `DROP DEFAULT`

**报错原文**（fail-stop 了后面 461 条）：

```text
ALTER TABLE "narrative_structures" ALTER COLUMN "is_template" TYPE boolean
  USING "is_template"::boolean
失败：字段 "is_template" 的默认值不能转换成类型 boolean
```

**根因**：光有 `USING` 不够 —— 列上若已有默认值，PG 会**把默认值也一并转换**，而那条转换走
**赋值转换**（assignment cast）规则；`integer → boolean` 只有 explicit cast ⇒ 被拒。
实测该列 `DEFAULT 0`（实况来自 `v220_narrative_structure.rs` 的 `INTEGER NOT NULL DEFAULT 0`），
而 L1 实体声明 `pub is_template: bool`。

计划里**确实有**该列的 `DropDefault`，但它按 `ChangeKind` 的枚举序排在 `AlterColumnType`
**之后** ⇒ 永远执行不到。

**修法**：把 `DROP DEFAULT` **并进 `AlterColumnType` 的渲染**（三步：`DROP DEFAULT` → `TYPE … USING`
→ `SET DEFAULT`（仅当期望侧有）），**而不是**把 `DropDefault` 排到前面 —— 后者修的是
「同列两条变更的相对顺序」这个**全局**性质，任何一次排序调整都可能重新破坏它；
并进本条后该类别**自包含**，排在哪里都对。无默认值的列上 `DROP DEFAULT` 是合法 no-op，故可无条件加。

新增 3 条测试，其中 `alter_type_drops_default_before_changing_type` 断言的是**相对顺序**
（只断言「存在 DROP DEFAULT」会被「排在 TYPE 之后」蒙混，而正是那个顺序出的错）。

#### 3. ✅ 验证：`BEGIN … ROLLBACK` 在真库试跑全部 31 条 ⇒ **31 / 31 通过**

只看 DDL 文本不足以证明 PG 会接受它（是否被接受取决于**列上的默认值和实际数据**）。
PG 的 DDL 是事务性的 ⇒ 用 `BEGIN; DROP DEFAULT; TYPE USING; ROLLBACK;` 逐条试跑，零副作用：

```
=== 试跑成功 31 / 失败 0 ===   （含修复前失败的那条 narrative_structures.is_template integer → boolean）
```

证据：`output/tmp-p4-alter-trial.log` · 脚本 `output/tmp-p4-alter-trial.mjs`。

> 这个手法值得独立记住：**「这条 DDL 会不会被接受」是可以在生产库上被安全回答的**，
> 前提是 PG 的可事务化 DDL + 一个明确回滚。它比「造一个副本库」便宜，也比「先跑了再说」安全。

### 十一·七 剩余 470 条的预判（**未执行，需先裁决**）

| 段 | 内容 | 预判 |
|---|---|---|
| 1 | `DROP TABLE` 8 张 | 闸 5 首见只登记 ⇒ **不会执行**（需第二跑才删） |
| 3 | `DROP DEFAULT` **409** | 会执行。⚠ **本行原归因（「全部源于声明缺口」）已作废**：其中 **9 条是引擎判据缺陷**（把「序列绑定」误读成「默认值」），另有 8 条属类型冲突而非声明缺口 ⇒ 真实需补声明的是 **392** 条。见 §十一·八 |
| 3 | `ALTER TYPE` 12 | ✅ **已逐条试跑通过** |
| 3 | `SET NOT NULL` 4 · `SET DEFAULT` 4 · `DROP NOT NULL` 7 | 未验证；`SET NOT NULL` 遇 NULL 值会失败 |
| 4 | `DROP COLUMN` 3 | 表总行数 0 ⇒ 无害 |
| 5 | `ADD FK` **13** | ~~⚠ **大概率失败**：§八 已登记 `knowledge_entities.knowledge_base_id` 有 **70.4% 孤儿** ⇒ 先清孤儿~~ → **2026-09-16 已作废**：该孤儿本文件 `:697` 已记 `70.4% → 0`，本轮真库实测 13 条**孤儿全为 0**（子表 0～112937 行）⇒ **可建** |
| 5 | `DROP UNIQUE` 5 · `CREATE INDEX` 5 | 未验证；`DROP UNIQUE` 按**派生名**删，名字不符则 fail-closed 报错 |

⚠ **`axagent_schema_version`（74 行）仍在 `DROP TABLE` 候选里** —— 它是 SeaORM 的**迁移元表**，
删掉会让迁移系统认为从未迁移过。这一条**必须由人裁决**（补 L1 声明 / 登记白名单），
不是引擎的缺陷：引擎只是如实报告「这张表没有任何一层声明它」。

> ✅ **2026-09-16 已裁决并落地**：登记为**基础设施表**豁免（`ORPHAN_EXEMPT` 规则 2，
> `OrphanExemptScope::Exact`）——**不**补 L1 实体。理由与不适用运行期白名单的原因见 §十一·八 ②。
> 实测 `DROP TABLE` 候选 8 → **7**。

> 也因此**没有自动重跑第二次**：剩余 470 条里最重的两块（409 条 DROP DEFAULT、8 张 DROP TABLE）
> 都是声明侧要处理的事，而不是执行侧能修的。**先把声明补齐，再跑第二遍。**

---

### 十一·八 ✅ 裁决执行（2026-09-16，用户指令「需要拍板的事情按照建议执行」）

上一节列了**两项待裁决**，本节记录它们的落地。执行过程中**新发现 3 件事**，其中 1 件推翻了
上一节自己给出的归因（§十一·七 表格第 3 行已就地更正）。

> **同日第二轮**（2026-09-16，用户指令「分析裁决项的建议值，然后按照建议值进行」）：
> 本节 ⑧ 原列的 6 项已逐条复核并**全部落地**。这一轮的新意是**先分析建议值本身** ——
> 复核推翻了其中 **3 条的前提**（项2 的「实体声明 `i64`」与「接受 `DROP DEFAULT`」、
> 项4 的「70.4% 孤儿」），依据见 ⑧.1；另附两处顺手修（⑧.3 4 处 `default_value = "NULL"`
> 字面量误用；⑧.5 单一真相源门禁 14 条 BROKEN → 0）与 ⑧.6 两条新登记。

#### ① 🔴 409 条 `DROP DEFAULT` 里 **9 条不是声明缺口，是引擎判据缺陷**

**根因**：PG 的 `SERIAL` 实现是「一个序列 + 一条 `DEFAULT nextval('…_seq')`」⇒ 实况侧 `default` **非空**；
而 sea-orm 的 `create_table_from_entity` 把自增表达为 `ColumnDef::auto_increment()`，**不写进 `default`**
⇒ 期望侧 `None`。`plan.rs` 的判据是「默认值只比有没有」⇒ 必然产出 `DROP DEFAULT`。
**执行它的后果是把自增列降级成普通列**，此后任何不给 id 的 INSERT 都因 NOT NULL 失败。

**关键取证**：`sea-query::ColumnSpec` 本来就有 `pub auto_increment: bool`，sea-orm 一直在填，
只是本仓 `expected.rs` **从未读它**。桥在 `sea-orm-2.0.2/src/schema/entity.rs:279-287`：

```rust
if column.as_str() == primary_key.into_column().as_str() {
    if E::PrimaryKey::auto_increment() { column_def.auto_increment(); }
```

⇒ **`spec.auto_increment` 一直有值**，这是个「字段存在但没人消费」的缺陷。

**⚠ 本节同时更正上一轮的一个错数字。** 上一轮报「实体侧已正确声明自增的 6 个 / 疑似漏声明的 4 个」——
那是用 `/auto_increment\s*=\s*true/` 这一**只看字面量**的判据测的，漏掉了宏的**隐式形态**
（`sea-orm-macros-2.0.2/src/derives/entity_model.rs:461,532`）：

```rust
if is_primary_key && auto_increment.is_none() {
    for suffix in NOT_AUTO_INCRE_TYPE_SUFFIX {   // = ["String", "Uuid"]
        if field_type.ends_with(suffix) { auto_increment = Some(false); break }
    }
}
let auto_increment = match auto_increment {
    Some(v) => v && primary_keys.len() == 1,
    None    => primary_keys.len() == 1,          // ← 裸 primary_key + 单主键 ⇒ TRUE
};
```

⇒ **裸 `#[sea_orm(primary_key)] pub id: i64` 的 `auto_increment()` 就是 `true`**。重测（判据 = 宏语义复算）：

| 组 | 旧（判据错） | **新** | 内容 |
|---|---|---|---|
| A 双侧都自增 ⇒ 仍被判 DROP DEFAULT | 6 | **9** | 6 显式 + 3 隐式（`gateway_usage` / `trajectory_steps` / `session_events`） |
| B 期望侧不自增 | 4 | **1** | 仅 `chat_run_events.id`（无实体，本是 DROP TABLE 候选） |
| C 期望侧自增但 DB 非自增 | 0 | **2** | `audit_log.id` / `l2_search_results.id`（见 ④） |

⇒ 9 条全部是引擎缺陷（不是「3 条声明缺口 + 6 条缺陷」）。**原计划「给 3 张表补 `auto_increment`」的活不存在。**

**修法（7 个文件）**：

| 文件 | 改动 |
|---|---|
| `reconcile/model.rs` | `ColumnModel` 加 `auto_increment: bool` + `default_is_sequence_binding()`（只看 `nextval(` 前缀，**不拼序列名** —— PG 按 63 字节截断，拼名字在长表名必错） |
| `reconcile/expected.rs` | 读 `spec.auto_increment`（**必须在 `c.default` 之前**） |
| `reconcile/plan.rs` | 任一侧自增 ⇒ **不比对 default**；两侧自增属性不一致 ⇒ **advisory 而非 DDL** |
| `reconcile/introspect/pg.rs` | 加 `a.attidentity::text`；`auto_increment = !ident.is_empty() \|\| default_is_sequence_binding()`（PG 两种实现：SERIAL / IDENTITY） |
| `reconcile/introspect/sqlite.rs` | rowid 别名判据：**单列主键且声明类型逐字是 `INTEGER`**。必须用**声明类型原文** —— `sqlite_class` 把 `BOOLEAN` 也归成 `"integer"` |
| `reconcile/render.rs` | 新增 `column_sql_type()`：`smallint/integer/bigint → smallserial/serial/bigserial`，其余 **fail-closed**；自增与 DEFAULT 共存即报错 |
| `reconcile/extras.rs` | `pattern_tsv_column_model` 补 `auto_increment: false` |

> ⚠ 自增的**两条渲染路径语义不同**：`CREATE TABLE` / `ADD COLUMN` 用**伪类型** `serial` 族，
> 而 `ALTER COLUMN … TYPE bigserial` 是**语法错误** ⇒ 映射必须放在调用侧（`column_sql_type`），
> **不能**改写 `ColumnModel.sql_type`。
> ⚠ 修改前的 `render.rs` 完全不看 `auto_increment` ⇒ 会**静默建出一张没有自增的表**（这正是 ④ 的成因）。

新增 8 条测试，其中 `auto_increment_mismatch_is_advisory_not_ddl` 断言的是**不得产出 DropDefault**
（只断言「有 advisory」会被「既 advisory 又 DDL」蒙混）。

#### ② ✅ `axagent_schema_version` 裁决：登记为**基础设施表**豁免（不补 L1 实体）

**理由不是「暂时别删」，是「所有权」**：该表的建表 / 写表 / 改表全在 `crates/dao/src/migrations`
（`SCHEMA_VERSION_TABLE`，`migrations/mod.rs:114`），L1 / L2 / L3 三层都**不表达它、也不该表达它** ——
补 L1 实体等于让 diff 引擎去 `ALTER` 别人的内部表。

**为什么不进运行期白名单**（`_ax_schema_orphan_whitelist`）：那份白名单**带有效期与人工理由**，
是「暂时别删」的职责。把永久设施塞进去，等于让它某天自动失效，然后迁移系统认为从未迁移过。

**实现**：`OrphanExemptDecl` 的匹配形态从「只有前缀」扩为两种 ——
`OrphanExemptScope::{Prefix { prefix, exclude_suffixes }, Exact(&[&str])}`。
两种形态对应两种完全不同的理由，**不合并成一个「前缀 + 精确名」的宽结构**（否则「这条规则覆盖多大范围」
无法从类型上读出来，判据 **#381**）。常量随之改名 `PATTERN_ORPHAN_EXEMPT` → **`ORPHAN_EXEMPT`**
（名字里的 `PATTERN` 在加入规则 2 后会变成一句谎）。`Exact` 形态的 `reason` **强制写明 owner**。

⚠ **顺带修掉一条恒真断言**：`plan.rs` / `expected.rs` 原先拿 `axagent_schema_version` 当「非向量族、
故不该被豁免」的样本；加入规则 2 后它命中豁免 ⇒ 该断言**恒真**（判据 **#174**：样本必须真落在规则范围之外）。
样本已换成 `notes` / `axagent_schema_versions`（近邻但不等于），并新增
`exact_orphan_exempt_covers_only_listed_infrastructure_tables` 守「规则 2 的范围」——
一旦有人图省事写成前缀 `axagent_`，整个迁移子系统的表都会静默脱离孤儿判定。

#### ③ ✅ 补 L1 声明：**392** 条（不是 400）

口径（三个数各自有实测依据）：

```
409  只读渲染里 DROP DEFAULT 的条数（引擎权威口径）
 − 9  自增列（实况 default 是序列绑定，归 auto_increment 管）          → §①
 − 8  `to_char(...)` 表达式默认值的列 —— 实况 text / 声明 String      → 见下方残留
 = 392
```

**独立两路测量闭合**（这对账是本节最值得记的部分）：

| 形态 | DB 侧（`tmp-p4-dd2.log`） | 写入侧（逐行差分） | 差 |
|---|---|---|---|
| 字符串字面量 | 191 | 191 | 0 |
| 数字 | 172 | int 145 + float 25 = 170 | −2 |
| 布尔 | 29 | 31 | +2 |
| 表达式 | 17（9 `nextval` + 8 `to_char`） | 不写 | — |

那 ±2 正是 `narrative_structures.is_template` 与 `opc_rl_experiences.success`
（实况 `integer default 0`，声明 `bool`）⇒ 按**声明类型**取字面量。两侧合计都是 392。

**写入前证伪的三件事**（都在动 116 个文件之前）：

1. **`#[sea_orm(default_value = X)]` 是否只影响 DDL、不改运行期 insert 行为？**
   ⇒ 是。宏侧**只有一个落点**（`sea-orm-macros-2.0.2/src/derives/entity_model.rs:495`），产出 `ColumnDef`；
   `default_value()` 只写 `self.default`（`sea-orm-2.0.2/src/entity/column_def.rs:87`）；唯一消费点是
   `sea-orm-2.0.2/src/schema/entity.rs:262` 的建表。`ActiveModelTrait::default_values()` 是**另一个机制**：
   契约明写「has a **`Default` impl**」，宏生成 `#(default.#fields = …)*` **无条件对每列**、
   **无按属性过滤的分支**；`Insert::one` 只发 `Set`/`Unchanged`（`sea-orm-2.0.2/src/query/insert.rs:107-117`）。
2. **`bool` 字面量能否编译？** 全仓既有先例只有 `0` / `0.0` / `"str"`，**bool 无先例**。
   ⇒ 查 `sea-query-1.0.2/src/value.rs:1138` `type_to_value!(bool, Bool, Boolean)` ✓。
3. **解析范围是否收窄到 `pub struct Model { … }`？** 原先扫整个文件的 `pub name: Type` ——
   若文件里还有别的 struct 且字段名撞上列名，`dbColumn` 匹配会**通过**，声明被插到错误的结构体里，
   而写在非 `Model` 结构体上**既不报错也不生效**（宏只认 `DeriveEntityModel` 的那个）⇒ **静默无效**。
   收窄后四项计数一字不变 ⇒ 原先没有误插。

**写入后核对**：116 文件 / 恰好 **392** 条新增 / **无任何其他行被改动**（逐行贪心归并差分，
任何非法改动即报错）。备份 `output/backup-2026-09-16-entities-defaults`（184 文件逐字节一致）。

**验证**：`cargo test -p axagent-dao --lib` ⇒ **373 passed / 0 failed，零 warning**；
`cargo build -p axagent-dao --example p4_apply_probe` ⇒ 0 error。
冻结计数测试（`l2_item_counts_are_frozen` / `entity_derivation_buckets_match_introspect` / fingerprint 族）全绿。

#### ④ 🔴 新发现（**未修，需裁决**）：8 张**侧车库表**被引擎建进了主库

`entities/src/lib.rs:253-278` 的 8 张表（`ast_call_edges` / `ast_classes` / `ast_functions` /
`ast_interfaces` / `ast_variables` / `file_index` / `l2_index_snapshots` / `l2_search_results`）
是**真 `pub mod`**，而 `dao/build.rs:25-37` 的扫描器是**纯前缀匹配、完全不认注释** ⇒ 全部进
`entity_registry` ⇒ `expected.rs` 为它们建 PG `TableDecl` ⇒ reconcile 计划在**所连的库**建表。

而该文件自己写着：

> ⚠ 它们**不在主库**：`file_index` / `ast_*` 落在 `indexing_triggers.rs` 的 `INDEX_DB_FILENAME`；
> `l2_*` 由 disk-cache 自持一个 SQLite 文件。
> 在此声明的是 schema，**与连接指向哪个库无关**。

⇒ 后果：**run3 的 12 张 `CREATE TABLE` 里有 8 张是侧车库表**。它们在主库是**永久空表**
（写入端在 SQLite 侧），且引擎会认为「它们必须存在」。

⇒ 引擎缺一个**「实体归属哪个库」的声明维度**。建议：新增与孤儿豁免**同构**的「非主库实体」声明
（**skip 而非 exempt**）—— 这些实体既不参与主库 `CREATE`，也不参与主库 `DROP` / 列 diff。
**不要**给 `l2_search_results.id` 补序列 —— 那张表本不该在主库（它现在只出一条 advisory，是诚实的）。

#### ⑤ 新发现（**已修**）：`audit_log` 缺自增序列 —— ④ 的既成损害

`audit_log` 是**正当的主库表**（`tools/src/audit.rs` 的审计流水，补实体就是为了让它真落库），
却因为修改前的 `render.rs` 忽略 `auto_increment`，被 run3 建成 `id integer NOT NULL` **无序列**。
（上一轮报「实测无既成损害」同样是错判据得出的 —— 当时只核对了 6 张**显式**自增表，漏了 2 张隐式的。）

**已按「纯 bug 可顺手修」处理**：纯增量 DDL（与 PG 给 `serial` 的展开逐字一致），
表 0 行、无依赖对象、可回滚：

```sql
CREATE SEQUENCE IF NOT EXISTS audit_log_id_seq AS integer OWNED BY audit_log.id;
ALTER TABLE audit_log ALTER COLUMN id SET DEFAULT nextval('audit_log_id_seq'::regclass);
```

**功能验证**：不指定 id 的 INSERT 返回 `id = 1`；探针行已清理，表回到 0 行。
`l2_search_results` **不动**（理由见 ④）。

> ⚠ 工具坑（值得记）：`node scripts/pg-connect.mjs sql "<多语句>"` 时 node-postgres 返回的是
> **结果数组**，`res.rows` 为 `undefined` ⇒ 脚本打印**字面量** `"undefined"` ⇒ `JSON.parse` 崩。
> **而语句其实已经执行了**（崩在序列化环节）。⇒ 逐条执行；且凡在真库跑 DDL 的脚本，按**幂等**写。

#### ⑥ 本轮自纠的错误判据（**都在本文件里就地更正**）

| # | 说过的错话 | 错在哪 | 真值 |
|---|---|---|---|
| 1 | 「409 条全部源于 L1 实体未写 `default_value` 的声明缺口」 | 未区分「序列绑定」与「默认值」两种东西 | 9 条是引擎缺陷、8 条是类型冲突、**392** 条才是声明缺口 |
| 2 | 「实体只声明了 6 处自增（A=6 / B=4）」 | 判据只 grep 字面量 `auto_increment = true`，漏宏的隐式形态 | A=**9** / B=**1** / C=**2** |
| 3 | 「实测无既成损害」（指 render 忽略自增） | 只核对了 6 张显式自增表 | `audit_log` / `l2_search_results` **确实中招**（已修 1、留 1 待裁决） |

共同点：**三次都是「判据比被测对象窄」** —— 用一个只覆盖部分情形的谓词去测一个更宽的对象。
与判据 **#174（样本必须落在规则范围之外）**、**#381（声称范围必须与落实范围逐对象对账）** 同族。

#### ⑦ 第三轮真库只读复验（`output/tmp-p4-render3.log`，零写入）

| 判据 | §十一·七 记录 | **本轮** | 说明 |
|---|---|---|---|
| `DROP DEFAULT` | 409 | **8** | 401 = 392 声明 + 9 自增，**算术精确闭合**；剩 8 = `to_char` 族 |
| `DROP TABLE` 候选 | 8 | **7** | `axagent_schema_version` 已豁免 |
| 孤儿候选 | 10 | **10** | 7 表 + 3 列（`narrative_structures.{arcs,confluences,foreshadows}`） |
| `ALTER TYPE` | 12 | 12 / 0 | 已逐条试跑通过 |
| `ADD FK` | 13 | 13 / 0 | ⚠ ~~真跑时大概率失败（§八：`knowledge_entities.knowledge_base_id` 70.4% 孤儿）~~ → **2026-09-16 已作废**：真库实测 13 条**孤儿全为 0** ⇒ 可建 |
| `DROP COLUMN` | 3 | 3 / 0 | 表 0 行 |
| `SET NOT NULL` / `DROP NOT NULL` / `SET DEFAULT` | 4 / 7 / 4 | 4 / 7 / 4 | 未试跑 |
| `DROP UNIQUE` / `CREATE INDEX` | 5 / 5 | 5 / 5 | 未试跑 |
| `CREATE TABLE` | 12（run3 已执行） | **0** | 期望表数 183，全部已存在 |
| 丢数据变更 | — | **22** | 7 `DROP TABLE` + 3 `DROP COLUMN` + 12 `ALTER TYPE`，每条都要导出证据 |
| 渲染失败 | — | **0** | |
| 自增 advisory（新） | — | **1** | 仅 `l2_search_results`（见 ④） |

期望指纹 `80474fe7405a34dc`（L1 改动前后**一致**）；实况指纹因 `audit_log` 被真改而变
（`3d385acfdf05009b` → `1d93fd60d92f6854`）—— 这是**预期**：实况确实变了。

#### ⑧ 剩余 6 项裁决：**先分析建议值，再按裁决落地**（2026-09-16 全部落地）

⚠ 原表（「剩余待裁决（本轮只登记，未擅改）」）的 6 条**建议值本身**在动手前被逐条复核，
其中 **3 条的前提失实 / 过时**、**1 条不完整**（见 ⑧.1）。下表是复核后的最终裁决。

**⑧.1 对「建议值」本身的分析（先证伪，再动手）**

| # | 建议值原话 | 复核结论 | 证据 |
|---|---|---|---|
| 2 | 「库里是 `text`、实体声明 **`i64`**」 | **❌ 前提失实**：那 8 列的实体声明**全是 `String`**（`entities/src/artifacts.rs:24` `pub updated_at: String` 等）⇒ **本无类型冲突**，建议值里「改声明类型」这一支根本不成立 | `output/tmp-d1-entity.log` |
| 2 | 「…或保留 DB 表达式默认值并**接受 `DROP DEFAULT`**」 | **❌ 不该接受**：这 8 条是 `to_char(...)` 生成的时间戳，与主流写入 helper `now_datetime_str()`（`harness/src/util_fns.rs:30-32`）**逐字同格式** —— 该函数自己的文档就写着「与 PG `to_char(CURRENT_TIMESTAMP AT TIME ZONE 'UTC', 'YYYY-MM-DD HH24:MI:SS')` 默认值格式完全一致」⇒ 它是**安全网**，不是垃圾 | 同上；`output/tmp-d1-writers.log`（8 个写入点全部显式 `Set(...)`，即该默认值只在绕开 helper 时兜底） |
| 4 | 「先清 `knowledge_entities.knowledge_base_id` 孤儿（**70.4%**）」 | **❌ 归因过时**：该孤儿**早已清零**（本文件 `:697` 原文「孤儿率 **70.4% → 0** ⇒ P4 这 14 条全部可建」）。本轮真库实测 13 条 `ADD FK` 的**孤儿数全部为 0**（子表 0～112937 行） | `output/tmp-d1-pg.log` |
| 3 | 「…但需确认是否要补实体」 | **✅ 确认无需补**：`dao/src/repo.rs:38-41` 原文「依赖 `entities::narrative_structure`（**单数**，`arcs/confluences/foreshadows` 三字段**旧 schema**）。现行链是 `narrative.rs` + `entities::narrative_structures`（复数，`structure` 单字段，同表已收敛）」 | Grep：全仓无该 3 列的 DB 列引用 |
| 5 | 「…⇒ 需导出证据（`--export-evidence`）」 | **⚠ 不完整**：引擎闸要的是**它自己格式**的证据（`.jsonl` + 回读 sha256 + 行数比对，`evidence.rs:212-272`）⇒ 手工导出**不算数**，必须走 `--export-evidence`，且目录可用 `AX_SCHEMA_EVIDENCE_DIR` 覆盖 | `evidence.rs` |

**⑧.2 逐项裁决与落地**

| # | 裁决 | 落地位置 | 实测复验 |
|---|---|---|---|
| 1 | 加「非主库实体」声明并 **skip** —— 且 skip 必须**双向**（既排除期望集、**也**不参与孤儿判定）；**按方言**限定 `Postgres` | `reconcile/extras.rs` `NON_MAIN_DB`（2 规则 / 8 表）；`expected.rs::build` 过滤 + `non_main_db_applies/non_main_db_reason`；`expected.rs::expected_table_count(dialect)` 改签名；`plan.rs::non_main_db_tables_of` 并入孤儿豁免 | 期望表数 **183 → 175**（−8，正好 8 张）·`CREATE TABLE` **0**·`DROP TABLE` **7**（8 张**既不建也不删**） |
| 2 | 给引擎加 **L2「表达式默认值」**支持（建议值三支里的**第三支**，前两支已证伪） | `reconcile/extras.rs` `EXPR_DEFAULTS`（1 规则 / 8 列）；`expected.rs::table_from_entity` 在 `c.default.is_none()` 时注入（**只在 L1 没给值时补**，L1 仍是真相源） | `DROP DEFAULT` **8 → 0**（该类**整类消失**，落进「未触及的 13 类」） |
| 3 | 那 3 列**随表处置**，**不补** L1 字段 | 无代码改动（宿主表 0 行，零风险） | 仍在 `DROP COLUMN` 3 条里，按 ⑧ 未执行 |
| 4 | 13 条 `ADD FK` **可建**（前置早已满足，建议值基于过时事实） | 无代码改动 | 渲染 **13 / 0** |
| 5 | 维持「首见只登记、第二跑才删」；第二跑**必须** `--export-evidence` | 引擎既有行为，无需改动 | 本份 plan `DROP TABLE = 7`（真跑时这 7 条先被延迟） |
| 6 | 单独一批工作（PG 侧 `ADD GENERATED` / 建序列 / 改默认值三件事） | **未动**（不在本轮范围） | 仍为 `ALTER GENERATED` 未触及类 |

**⑧.3 顺手修：4 处 `default_value = "NULL"` 是字面量误用**

`#[sea_orm(default_value = X)]` 收的是 `Into<Value>` 的**字面量**，字符串 `"NULL"` 会渲染成
`DEFAULT 'NULL'`。实测两类后果都坏：`double precision` 列上 `'NULL'::double precision`
**报「无效的类型 double precision 输入语法」** ⇒ `ALTER … SET DEFAULT` **DDL 直接失败**；
`text` 列上则写入字面量字符串 `"NULL"`（`stock_analyses.llm_decision_json` 会被按 JSON 解析失败，
`parent_analysis_id` 变成指向不存在行的**悬挂引用**）。
⇒ 删掉这 4 处属性（`entities/src/models.rs` 2 处、`entities/src/stock_analyses.rs` 2 处）——
`Option<T>` 列的「默认 NULL」本来就等于**不写 DEFAULT 子句**。复验 `SET DEFAULT` **4 → 0**。

**⑧.4 第四轮真库只读复验（`output/tmp-d9-pgrender.log`，零写入）**

| 判据 | §⑦（`tmp-p4-render3.log`） | **本轮** | 说明 |
|---|---|---|---|
| 期望表数 | 183 | **175** | **−8**，正好等于 8 张侧车库表 ⇒ 声明生效 |
| `DROP DEFAULT` | 8 | **0** | 该类整类消失 ⇒ `EXPR_DEFAULTS` 生效 |
| `SET DEFAULT` | 4 | **0** | 该类整类消失 ⇒ 4 处字面量误用已清 |
| `DROP TABLE` | 7 | **7** | 8 张侧车库表**没有**被追加进孤儿 ⇒ skip 是双向的 |
| `CREATE TABLE` | 0 | **0** | 8 张不再进期望集 ⇒ 不会重演 run3 的建表 |
| 孤儿候选 | 10 | **10** | 7 表 + 3 列 |
| 自增 advisory | 1（`l2_search_results`） | **0** | 该表本不该在主库 ⇒ 随排除一并消失（**遗留项已消**） |
| 渲染失败 | 0 | **0** | |
| 变更合计 | 56 | **56**（破坏性 22 / 非破坏性 34） | 逐类：`ADD FK 13`·`ALTER TYPE 12`·`CREATE INDEX 5`·`DROP COLUMN 3`·`DROP NOT NULL 7`·`DROP TABLE 7`·`DROP UNIQUE 5`·`SET NOT NULL 4` = 56 ✔ |

**分账恒等（可复算）**：实况 **197** 张表 = 期望 **175** + 孤儿候选 **7**（表）+ 豁免/模式覆盖 **15**
（`ast_*` 5 + `file_index` + `l2_index_snapshots` + `l2_search_results` + `axagent_schema_version` +
3 张 `vec_*` + 3 张 `vec_*_meta` 走 `PATTERN_TSV`）⇒ `175 + 7 + 15 = 197` ✔
另：实况表数 185 → 197（**+12**）正是 §十一·七 记录的「run3 已执行的 `CREATE TABLE` 12 条」。

**⑧.5 顺手修：单一真相源门禁 14 条 BROKEN → 0**

动手前先证伪了「这 14 条是不是本轮引入的」：其中 **11 条在我未编辑过的文件里**
（`reconcile/model.rs` 7 条 · `harness/{knowledge_graph,page_type}.rs` 4 条），
另 3 条在 `expected.rs` 的 FK 注释块（非本轮所写）⇒ **非本轮回归**；且棘轮基线
`scripts/single-source-facts-allowlist.json` 是 `count: 0 / known: []`（**建立时该门禁是绿的**），
而 `--update-baseline` 在有新增失败时**拒绝写**（`:628-635`）⇒ 只能真修，灌不进去。

| 类 | 条数 | 根因 | 修法 |
|---|---|---|---|
| 引用仓外第三方源码被误判 BROKEN | **10**（`model.rs` 7 · `expected.rs` 3） | `locate()` 先跑（`:294-309`，仓外文件必然 `kind === "B"`），**只有 kind B 才查** 块级 `EXTERNAL_MARK`（`:200` = `/[A-Za-z0-9_.-]+-\d+\.\d+\.\d+[/"`]/`）⇒ 块里**没写** `crate-x.y.z/` 前缀的引用全被判「文件不存在」。两版本并存（`sea-orm-2.0.1/2.0.2`、`sea-query-1.0.1/1.0.2`）正是必须写全版本目录的理由 | 补全为 `sea-query-1.0.2/src/…` / `sea-orm-2.0.2/src/…`（与同文件 `model.rs:29`·`expected.rs:21` 已有写法一致）。⚠ **先逐条核对行号再补**（对照 `C:\Users\polit\.cargo\registry\…` 真源），否则就是把**可见的错**变成**不可见的错** |
| 引用指向空行（`wiki.rs` 行号漂移） | **4**（`knowledge_graph.rs` → `mapping` 边；`page_type.rs` → `daily`/`knowledge`/`synced`） | `src/commands/wiki.rs` 后续改动导致行号漂移 | **只改引用处的行号**（真源是代码）：`1132 → 1149`（`"mapping"` 字面量）、`1406 → 1413`、`1660 → 1667`、`1906 → 1913`（三条**同为 +7**，同一处插入所致） |

复验（`output/tmp-d9-ssf.log`）：`BROKEN/越界/空行/控制流 14 → 0`；`EXTERNAL 12 → 22`（**+10**，正是补全的那 10 条）；
`LOCATED 272 → 276`（**+4**，正是修正行号的那 4 条）。三分法合计 `276 + 117 + 22 + 0 = 415`
= 去重后引用总数 ✔（**没有任何引用被静默丢弃**）。`SSF_EXIT=0`。

**⑧.6 本轮新登记（只登记，未擅改）**

| # | 项 | 影响 | 建议 |
|---|---|---|---|
| N1 | `ApplyOptions::last_expected_table_count` **在真库路径从未被赋值**：`apply.rs:95` 默认 `None`，`apply.rs:634` 只传**本轮**的 `want.tables.len()`（`apply.rs:377` 文档亦如此写明），全仓只有测试 `apply.rs:1076` 设过 `Some(200)` ⇒ `safety.rs:334` 的 `if let Some(last)` 分支**恒不进入** | 「期望集塌缩」（`BaseShrank`）这条熔断**目前实际不生效** —— 而它是 P0a 那类事故（期望集崩成空 ⇒ 整库判孤儿）的最后一道闸 | 需要一个「上一轮期望基数」的**持久化端**（元表存 + 下轮读回）。在那之前，该闸只剩 `expected_table_count == 0` 这一种兜底 |
| N2 | 8 张侧车库空壳**仍在主库**（本轮只做到「不再建、不再删」） | 8 张永久空表占位 | 一次性人工清理需**单独授权**（本轮未动） |

**⑧.7 顺手修：本 PLAN 自有文件的 `cargo fmt` 偏差（93 → 1）**

`package.json:210` 的 `pre-commit` 就是 `cd src-tauri && cargo fmt --check`（整个 workspace），
而本 PLAN 的 P4 文件**写完后从未跑过 fmt**。实测（`tmp-d9-fmtws.log` → `tmp-d9-fmtws3.log`）：

| | 偏差处数 | 文件数 | 明细 |
|---|---|---|---|
| 修前 | **93** | 10 | `render.rs` 31 · `apply.rs` 17 · `examples/p4_apply_probe.rs` 15 · `evidence.rs` 11 · `safety.rs` 6 · `extras.rs` 5 · `plan.rs` 3 · `expected.rs` 2 · `mod.rs` 2 · **`disk-cache/src/lib.rs` 1（非本 PLAN）** |
| 修后 | **1** | 1 | 仅 `crates/disk-cache/src/lib.rs` —— **未动**（非本 PLAN 的代码，只登记） |

做法沿用 §9.7 的既有口径：`rustfmt --config-path rustfmt.toml --edition 2024 <本 PLAN 的文件>`。
⚠ **配置路径必须传 Windows 形式** —— 传 `/d/OneManager/.../rustfmt.toml` 会报
`unable to find a config file for the given path` 并**退 1 且不写任何文件**（实测踩到一次；
与「Node 路径必须传 Windows 形式」是同一条纪律）。

**只格式化了本 PLAN 自有的文件**（`reconcile/*.rs` + `examples/p4_apply_probe.rs`）。
fmt 后按纪律**重跑全部判据**：`378 passed / 0 failed`（`tmp-d9-test3.log`）、example 重编
`BUILD3_EXIT=0`（`tmp-d9-build3.log`）、**真库只读渲染逐字节相同**（`tmp-d9-pgrender.log` 仍是
31782 chars，四项判据一个没变）⇒ 格式化未改变任何行为。

#### ⑨ 证据索引

| 文件 | 内容 |
|---|---|
| `output/tmp-p4-ai-remeasure.log` | 按宏语义复算的 A/B/C 分组（**取代** `tmp-p4-serial.log`，后者判据错） |
| `output/tmp-p4-dv3.stdout` | `default_value` 的宏侧落点 + `or_default_values` / `column_def` / `value.rs` 三处源码 |
| `output/tmp-p4-sq3.log` | `type_to_value!(bool, Bool, Boolean)` 证明 bool 字面量可编译 |
| `output/tmp-p4-l1-dryrun3.log` | 收窄解析范围后的 dry-run（392 / 9 / 8 / 3 四项对账） |
| `output/tmp-p4-l1-apply.log` · `tmp-p4-verify-write.log` | 写入与逐行差分核对（116 文件 / 392 条 / 无其他改动） |
| `output/tmp-p4-t2-test.log` · `tmp-p4-t2-build.log` | 373 passed / 0 warning / 0 error |
| `output/tmp-p4-render3.log` | 第三轮真库只读复验（零写入） |
| `output/tmp-p4-seqfix.log` | `audit_log` 序列修复的前后快照 + 功能验证 |
| `output/tmp-p4-sidecar.log` | 侧车库发现（`lib.rs` 声明 + `build.rs` 扫描器） |
| `output/backup-2026-09-16-entities-defaults/` | 184 个实体文件的逐字节备份（回滚用） |
| `output/tmp-d9-fmtws.log` · `tmp-d9-fmtws3.log` · `tmp-d9-fmtagg.log` | fmt 偏差归因与补齐：**93 处 / 10 文件 → 1 处 / 1 文件**（残留仅 `disk-cache/src/lib.rs`） |
| `output/tmp-d9-test3.log` · `tmp-d9-build3.log` | **fmt 后重跑**：`378 passed / 0 failed` · `BUILD3_EXIT=0` |
| `output/tmp-d9-fmtfix2.log` · `tmp-d9-fmtfix3.log` | rustfmt 写入与复核（`RF_WRITE_EXIT=0` / `RF3_EXIT=0` / `RF_ALL_EXIT=0`） |
| `output/tmp-d9-mem.log` · `tmp-d9-memadd.log` | 记忆三层体检（注入层 **7095 / 7168 B = 99%**，余量 73 B）+ 新增铁律 #408/#409（组 A）、#410（组 I） |
| `output/backup-2026-09-16/PLAN-declarative-schema-sync.md` | 本轮 PLAN 改动前的完整副本（回滚用） |
| `output/tmp-d8-test.log` | 本轮 `cargo test -p axagent-dao --lib`：**378 passed / 0 failed / 0 warning**（新增 6 条：`extras` 2 · `expected` 2 新增 + 1 改写 · `plan` 1） |
| `output/tmp-d8-build.log` | 本轮 `cargo build -p axagent-dao --example p4_apply_probe`：`Finished` / `BUILD_EXIT=0` |
| `output/tmp-d9-pgrender.log` | **第四轮**真库只读复验（零写入）：期望 175 / `DROP DEFAULT` 0 / `SET DEFAULT` 0 / `DROP TABLE` 7 / `CREATE TABLE` 0 / 渲染失败 0 |
| `output/tmp-d9-ssf.log` | 单一真相源门禁：**BROKEN 14 → 0**、`EXTERNAL` 12 → 22、`LOCATED` 272 → 276、`SSF_EXIT=0` |
| `output/tmp-d8-seasrc.log` | 第三方真源定位（`sea-orm-2.0.2` / `sea-query-1.0.2` 在 `~/.cargo/registry`），用于**逐条核对**被门禁判 BROKEN 的行号 |
| `output/tmp-d8-ssfix-harness.log` · `tmp-d8-ssfix-dao.log` | 引用行号/路径修正的逐条计数 + 「行数不变」断言 |

---

### 十一·九 第二次真库执行：56 条未实施项落地（2026-09-16，用户授权）

> 承 §十一·八 结尾那三件「待授权」。用户指令「**授权进行未实施项**」—— 本轮落点是第 1 件：
> 在生产库 `localhost:5432/axinvest` 上**真正执行**那批 DDL。
>
> ⚠ 授权是对**结果**的授权，不是对方法的授权。所以本轮的第一动作不是执行，而是
> **把「授权时双方都不知道的事实」挖出来**：凡涉及不可逆数据损失的，一律**先阻断再回报**，
> 没有替用户拍板（见下第 1 条，两张非空孤儿表因此被拦住）。

#### ① 执行前的数据侦察（只读）—— 挖出 2 张非空孤儿表，按引擎设计用**闸 6**阻断

计划里 7 条 `DROP TABLE` 逐张查了行数。**5 张 0 行，2 张非空**：

| 表 | 行数 | 判定 |
|---|---|---|
| `chat_run_events` · `chat_runs` · `opc_demand_lead` · `opc_market_platform` · `rl_policies` | 各 **0** | 删表零损失 |
| `opc_capability` | **324**（`tool:AddKnowledgeDocument` 等工具能力清单，中文描述 + `scanned_at`） | ⚠ 有真实业务数据；写入端已移除（`v221_demand_discovery.rs` 已随 `992947dfc` 删除，其原文注记「已移除」/ `demand_discovery.rs:39`「不再落库」） |
| `vec_kb_…_meta_dup_backup` | **24518**（仅 `rowid` 一列） | ⚠ 名自陈 `dup_backup`；活表 `vec_kb_…_meta` 现 **24516** 行 ⇒ 差 2 行，即被去重掉的项 ⇒ 它是**去重前快照** |

顺带排掉一对**同名族陷阱**：`opc_demand_lead`（单数，31 列，DROP 候选）与 `opc_demand_leads`
（复数，23 列，活跃）。若单数版有数据，性质就是「改名」而不是「废弃」。实测 **0 行** ⇒ 无风险。

两张非空表**没有删**，按引擎自身设计写进**闸 6 白名单**（`_ax_schema_orphan_whitelist`，
90 天有效至 epoch `1797310821`）：

```sql
INSERT INTO _ax_schema_orphan_whitelist (table_name, reason, expires_at) VALUES
  ('opc_capability',            '324 行真实业务数据（工具能力扫描清单）…无 L1 声明属「有写无声明」缺陷 ⇒ 删除须先裁决…', 1797310821),
  ('vec_kb_…_meta_dup_backup',  '24518 行、仅 rowid 一列，名自陈 dup_backup ⇒ 人工去重前快照…删备份须人确认…',        1797310821)
ON CONFLICT (table_name) DO UPDATE SET reason = excluded.reason, expires_at = excluded.expires_at;
```

⇒ `DROP TABLE` **7 → 5**，变更合计 **56 → 54**（`tmp-k1-dryrun2.log`）。

#### ② 缺陷 D1（真库第一现场）：元表整型列声明成 `INTEGER` ⇒ 整条 apply 路径在 PG 上不可用

第一次带白名单的 dry-run 就炸：

```text
失败: Query Error: error occurred while decoding column "expires_at":
      mismatched types; Rust type `core::option::Option<i64>` (as SQL type `INT8`)
      is not compatible with SQL type `INT4`
```

根因：`safety.rs::META_DDL` 把 8 个整型列声明成 `INTEGER`（PG = **int4**），
而 Rust 侧这些列一律 `i64`（sqlx 解码**要求 int8**）。

**CI 永远测不到的三个掩盖条件**（缺任何一个都会当场红）：

1. PG 的 `INTEGER` 是 **int4**，`BIGINT` 才是 int8 —— 两者不同名同义；
2. SQLite 的 `INTEGER` 亲和性本就是 **64 位** ⇒ `sqlite::memory:` 上 `i64` 天然匹配，恒绿；
3. `load_orphan_whitelist` 的解码在 `for r in rows` 里 ⇒ **表空时循环体一次都不执行**，
   解码检查根本不跑。本次的白名单登记是这张表**第一次有内容** ⇒ 第一次引爆。

⇒ 结论：**整条 apply 路径在 PG 上此前不可用**，而所有自动化判据都是绿的。

修法（`safety.rs`）：8 个整型列 `INTEGER → BIGINT`；新增 `PG_INT_WIDENING`（10 条幂等
`ALTER TABLE … TYPE bigint`）+ `needs_int_widening(backend)`（仅 PG 执行）。

⚠ **顺序是正确性问题，不是风格问题**：`CREATE TABLE IF NOT EXISTS` **改不动已存在的表** ⇒
必须**先建表、后加宽**；反过来在全新库上会 `relation does not exist`。

新增 4 条测试（378 → **382**）。其中两条是 CI 里**唯一**能拦住它的地方：

- `meta_ddl_has_no_bare_integer_columns` —— 断言 `META_DDL` 里不出现 `INTEGER`；
- `pg_int_widening_covers_every_integer_column` —— 硬断言 `len() == 10` + 形态 + **只允许动
  `_ax_schema_*` 元表**（这段是**自动执行**的 DDL，写错表名就是去改业务表）。

#### ③ 缺陷 D2（真库第一现场）：闸 5 的「延迟一周期」在这里实装成了「加速」

`_ax_schema_pending_drops` 已有 **8 行**（上一轮 `run-1789526796-000` 登记），
其中 7 张正是本次要删的表 ⇒ `defer_decision` 返回 `Execute`，**下一轮会直接 DROP**
（闸 5 的「首见延迟」缓冲已经用掉了，这是它的正常语义，不是缺陷）。

**缺陷在另一半**：白名单「豁免」只让表不进 `plan`，**不清 pending 行** ⇒
豁免**到期/撤销**后该表重新进 plan，而 pending 里的陈旧登记让它**跳过首见延迟、立刻被删**。
「暂缓」实现在这里变成了「加速」—— 恰好把闸 6 的意图反转。

修法：`put_orphan_whitelist` 末尾补 `clear_pending_drop(db, table)`。
**不对称是刻意的** —— `remove_orphan_whitelist` **不**恢复 pending 行（理由写在函数文档里）。
加 1 条测试 `whitelisting_a_table_clears_its_pending_drop`。

顺带清掉 3 行陈旧 pending（**先全表备份**到 `output/backup-2026-09-16-pending-drops.json`，8 行含
`first_seen_at`）：`axagent_schema_version`（已被 `ORPHAN_EXEMPT` 规则 2 豁免）+ 白名单里的 2 张。
它们的 pending 行已是**死数据**。

#### ④ 执行前的逐条试跑（事务内跑 + 强制 ROLLBACK）

54 条先在 `BEGIN … ROLLBACK` 里**逐条试跑**（`SET LOCAL lock_timeout = '5s'`，`try/finally` 保证回滚）：
**54 / 54 全部通过、0 失败**，并复核 `public` 表数不变 ⇒ **回滚干净、库未改动**。

`ADD FK` 那 13 条试跑通过尤其重要 —— PG 在 `ADD FK` 时会**真实校验现有数据**，
等于独立确认了「孤儿为 0」这件事，而不是靠前置查询推断。

#### ⑤ 执行结果（run_id `run-1789535504-000`）

```text
变更合计 : 54｜执行 54｜延迟 0｜未执行 0
整批拒绝 : 无      执行中止 : 无      墓碑 : 5 条
审计     : 54 行   白名单 : 生效 2 条｜已过期 0 条
```

| 项 | 执行前 | 执行后 | 对账 |
|---|---|---|---|
| `public` 表数 | 201（197 可见 + 4 元表） | **196**（192 可见 + 4 元表） | 201 − 5 ✔ |
| `_ax_schema_graveyard` | — | **5** 行 | = DROP TABLE 5 ✔ |
| `_ax_schema_pending_drops` | 8 | **0** | 5（删成功时清）+ 3（手工清）✔ |
| `_ax_schema_orphan_whitelist` | 2 | **2** | 未过期 ✔ |
| 5 张待删表残留 | 5 | **0** | ✔ |
| 2 张受保护表残留 | 2 | **2** | ✔ |

证据导出：**20 个对象 / 145770 行 / 1.1 MB**（`notes.tags` · `notes.related_pages` ·
`notes.source_refs` 各 48590 行；其余 17 个对象均 0 行，sha256 统一为
`e3b0c44298fc1c14…` = 空文件）。

> 陈旧证据混杂的处置：`output/schema-evidence/` 里有上一轮的 **42 个文件 / 9.14 MB**，
> 而本次 plan 只需 22 个对象 ⇒ 全量导出只会覆盖同名文件、目录新旧混杂、分不出哪些是本次的。
> 故整目录改名为 `output/schema-evidence-prev-run-1789526796/` 后再导出。

#### ⑥ ✅ 归因 `converged = false`：**17 张差集，机械闭合**（结论：不是执行失败）

执行后重跑 dry-run（`tmp-n1-dryrun3.log`）：**变更合计 0，逐条空集（结构与声明一致）**
⇒ 54 条 DDL 全部生效，列 / 约束层面**已完全对齐**。但 `converged = Some(false)`：

```text
期望表数 175 ｜ 期望指纹 862750c6d78a104a ｜ 应用后指纹 aabbc10d39084312
```

**「逐条空集 + 指纹不等」这个组合本身就排除了结构差异**：`diff` 报 0 条 ⇒ 同名对象全一致、
无缺失、无未豁免孤儿。差异只能来自「不进 diff 比对范围」的对象 —— 即**被豁免的实况多余表**
（`diff` 不会为它们产生变更，但它们仍在实况模型里、仍在指纹里）。

机械取证（两份名单做差集，**不是推断**）：

| 实况可见表 | 期望表 | 差集 |
|---|---|---|
| **192** | **175** | **17 张「实况有、期望无」**；反向 **0 张缺表** |

| 张数 | 表 | 豁免来源（逐张对上） |
|---|---|---|
| 6 | `ast_call_edges` · `ast_classes` · `ast_functions` · `ast_interfaces` · `ast_variables` · `file_index` | `NON_MAIN_DB` 规则①（`index.db` 族） |
| 2 | `l2_index_snapshots` · `l2_search_results` | `NON_MAIN_DB` 规则②（`l2_cache.db` 族） |
| 1 | `axagent_schema_version` | `ORPHAN_EXEMPT` 规则 2（`Exact`） |
| 6 | `vec_capabilities` · `vec_capabilities_meta` · `vec_kb_…` · `vec_kb_…_meta` · `vec_wiki_…` · `vec_wiki_…_meta` | `ORPHAN_EXEMPT` 规则 1（`Prefix("vec_")`） |
| 2 | `opc_capability` · `vec_kb_…_meta_dup_backup` | **人工白名单**（本轮写入） |

8 + 1 + 6 + 2 = **17** ✔ 精确闭合。旁证两条：`8` 与
`extras.rs EXPECT_NON_MAIN_DB_TABLES = 8` 逐字吻合；`vec_…_dup_backup` 虽以 `vec_` 开头，
却命中规则 1 的 `exclude_suffixes` 而**未被**豁免、落到人工白名单
⇒ 排除后缀机制确实在工作（不是一条死规则）。

期望名单的取法（**零改动**）：`--render-coverage` 用空模型 diff 期望模型 ⇒ 175 条 `CreateTable`，
`Plan::render` 的变更序列逐条含表名。实况名单直接查 `pg_class`（`relkind` 分布
`r:196 / i:458 / S:10`，**无视图 / 物化视图 / 分区表**，干净）。

**接上本文件旧记录**：§十一·五 结尾那段——

> 顺带核对一处**看着像不自洽但实际自洽**的数字：执行后实况 197、期望 183、`CREATE TABLE 0`、
> `DROP TABLE 8`，四者差 6。差的那 6 张是**被静态规则豁免的表**…

当时已观察到这个现象，但只把它当成「差一笔要当场算清」。现在可以确认：**它就是 `converged = false`
的根因**。当时那 6 张（向量族）现在仍在名单里；另 11 张是后来新增的豁免
（`NON_MAIN_DB` 8 张 + 白名单 2 张 + `axagent_schema_version` 1 张）。
期望表数 **183 → 175** 也正是 `NON_MAIN_DB` 排除那 8 张造成的。

#### ⑦ 🔴 登记 D3（**未修，需裁决**）：P4 收敛判据在生产库上**结构性恒假**

引擎自己的文档**预言过**这个缺陷（`introspect/mod.rs:40-44`）：

> 若只做「读了但不判孤儿」（像 `extras::ORPHAN_EXEMPT` 那样），它们仍会进入**实况指纹**，
> 而期望指纹里没有它们 ⇒ **两个指纹永不相等**，后果两条：
> 1. 「apply 后实况指纹 == 期望指纹」这条 P4 出口判据**永久不成立**；
> 2. 引擎的指纹短路（相同即跳过）永不生效 ⇒ 每轮都做全量 diff。

当时的处置只把**元表**纳入「读取层排除」（`ENGINE_META_PREFIX` + `introspect::read` 唯一 `retain` 点）。
而 `ORPHAN_EXEMPT` / `NON_MAIN_DB` / 人工白名单这三类**只做了「不判孤儿」那半边**，
另半边（不进指纹）没做 ⇒ **只要库里存在任一命中豁免的表，`converged` 就永不为真**。

后果分三层，严重性递增：

1. 不丢数据 —— `diff` 0 条是硬结论，结构与声明**确已对齐**；
2. P4 出口判据**无法用作「同步完成」的验收信号**（它永远红）；
3. 指纹短路永不生效 ⇒ **每轮都做全量 introspect + diff**（噪声与开销），
   且闸 3 的基数基准（`ApplyOptions::last_expected_table_count`，`apply.rs:81-82`）
   在本仓库**没有任何生产调用方给它赋过值** ⇒ 该闸在真库上恒为 `None`、**从不比对**。

三个候选修法（**各有取舍，属产品语义决策，本轮未动**）：

| # | 做法 | 代价 |
|---|---|---|
| **(a)** | 实况模型在 `fingerprint` 前 `retain` 掉「期望集没有 **且** 命中豁免规则」的表 —— 与元表**同一个规则**、同处**唯一入口**（`introspect::read:87-98` 已实现过这个模式，只需把条件从「前缀」扩为「前缀 ∪ 豁免规则」） | 指纹不再覆盖这些表的结构变化；但它们本来就不归引擎管 ⇒ 可接受 |
| (b) | 把豁免表加进期望模型（「已认领但不约束」的占位） | 期望集不再是 L1+L2 的忠实投影 |
| (c) | 拆判据：`converged` 改用「diff 为空」判，指纹相等只做短路用 | 改 P4 出口判据的定义 |

⚠ **必须避免的错解法**：为了让 P4 变绿而去删这 17 张表。其中 6 张是向量集合**真数据**、
8 张是侧车库的**家**（不该在主库、但**也不该由主库引擎删**，理由见 `plan.rs:497-500`）、
1 张是迁移子系统元表。把判据扭曲去迁就实现，是本 PLAN 反复记录过的那类错误。

#### ⑧ 本轮改动清单与验收

| 文件 | 改动 |
|---|---|
| `crates/dao/src/reconcile/safety.rs` | 模块文档（`TEXT`/`INTEGER` → `TEXT`/`BIGINT` + 三个掩盖条件 + 为何不能按报错表定位范围）· `META_DDL` 8 列加宽 · 新增 `PG_INT_WIDENING` + `needs_int_widening` · `ensure_meta_tables` 先建后加宽 · `put_orphan_whitelist` 补 `clear_pending_drop` · 新增 4 条测试 |
| `_ax_schema_orphan_whitelist`（生产库**数据**） | 新增 2 行（非空孤儿表阻断，90 天有效） |
| `_ax_schema_pending_drops`（生产库**数据**） | 清 3 行陈旧登记（先备份 8 行） |

验收（**代码本轮未再改动**，故为复核）：

| 判据 | 结果 | 证据 |
|---|---|---|
| `cargo test -p axagent-dao --lib` | **382 passed / 0 failed / 0 ignored / 0 warning** | `output/tmp-o8-test.log` |
| `cargo fmt -p axagent-dao -- --check` | **全绿**（0 字节输出） | `output/tmp-o6-fmt.log` |
| 执行后 dry-run | 变更 **0** 条、逐条空集 | `output/tmp-n1-dryrun3.log` |
| 分账闭合 | `201 − 5 = 196` ✔ | `output/tmp-n2-postcheck.log` |

#### ⑨ 证据索引（本轮）

| 文件 | 内容 |
|---|---|
| `output/tmp-k1-dryrun2.log` | 白名单生效后的 dry-run：变更 **56 → 54**、`DROP TABLE 7 → 5`、解码缺陷已修 |
| `output/tmp-l1-ddltrial.log` | 54 条 DDL 事务内逐条试跑 + 强制 ROLLBACK：**54 / 54 通过** |
| `output/tmp-m1-execute.log` | 真库执行 `run-1789535504-000`：**执行 54 / 中止 0 / 整批拒绝 0 / 墓碑 5** |
| `output/tmp-n2-postcheck.log` | 执行后机械核对六项分账（`196 / 5 / 0 / 2 / 0 / 2`） |
| `output/tmp-n1-dryrun3.log` | 执行后重跑 dry-run：**变更 0、逐条空集**（结构收敛，但指纹不等） |
| `output/tmp-o1-expect-dump.log` · `tmp-o3-expect-names.json` | 期望侧 **175** 张表名清单（`--render-coverage` 导出的真实声明集） |
| `output/tmp-o4-actual-tables.json` | 实况 `public` 全部对象（`relkind` 分布 `r:196 / i:458 / S:10`，无视图/物化视图/分区表） |
| `output/tmp-o5-diffset.json` | 差集：**17 张「实况有、期望无」+ 0 张缺表** |
| `output/tmp-o6-fmt.log` · `tmp-o8-test.log` | `fmt` 全绿（0 B）· **382 passed / 0 failed / 0 warning** |
| `output/schema-evidence/` | 本轮导出证据 **20 对象 / 145770 行 / 1.1 MB** |
| `output/schema-evidence-prev-run-1789526796/` | 上一轮 42 个文件的保全（避免新旧同名文件混杂） |
| `output/backup-2026-09-16-pending-drops.json` | pending 表现状备份（8 行含 `first_seen_at`），清 3 行前的安全网 |
| `output/backup-2026-09-16/PLAN-declarative-schema-sync.before-s11-9.md` | 本 PLAN 改动前完整副本（222619 B，sha256 `a9a4f32f0374f755`，回滚用） |

#### ⑩ 遗留（**未擅自推进**，逐条给出阻断理由）

1. 🔴 **D3**（上第 ⑦ 条）：P4 收敛判据在生产库结构性恒假 —— 修法需裁决（三选一）。
2. `opc_capability`（324 行）与 `vec_…_meta_dup_backup`（24518 行）的**最终去留** ——
   白名单 90 天有效（至 epoch `1797310821`），**到期后它们会重新进入孤儿判定**。
3. 8 张侧车库空壳仍在主库：`NON_MAIN_DB` 已让引擎不再管它们，但**物理清理**未做
   （§五·一 :444-473 已实测裁决「零访问 ⇒ 不补实体」，并记载曾新增
   `v234_drop_orphan_tables` 迁移来删它们、随后**已撤回**）。
4. `ApplyOptions::last_expected_table_count` 的持久化端仍未接 ⇒ **闸 3 在生产路径上从不比对**
   （见 ⑦ 第 3 层后果）。

## 十一·十 第二轮生产运行：全模式矩阵 + 日志排查（2026-09-16）

> 触发：上一轮真库执行（`run-1789535504-000`，54/54）之后，指令「运行生产环境，根据日志排查问题」。
> 做法：把探针的**四个模式 + 一个冲突用例**全跑一遍（不止生产那一路），逐份日志排查。
> 结果：**修 3 处**（T1 安全 / T2 口径矛盾 / T3 信息丢失），**登记 1 处**（T4，需裁决），
> 并把 §十一·九 ⑦ 的收敛判据根因**推进到更深一层**（原方案已被证明不充分）。

### ① 缺陷 T1（**安全级**）：明文口令经 argv 落进日志

| 项 | 内容 |
|---|---|
| 现象 | `output/tmp-p1-dryrun-prod.log:26` 出现 `Running \`...p4_apply_probe.exe 'postgres://postgres:<口令>@localhost:5432/axinvest'\`` |
| root cause | 探针**代码**的脱敏做得很足（`pg_target_label` 只打 `host:port/db`，`require_pg_url` 只回显 scheme），但**调用方式不归它管**：连接串是从 **argv** 进的库，而 `cargo run` 会把整条 argv 原样回显 ⇒ 脱敏写在代码里、泄漏发生在传输层 |
| scope | 驱动脚本 `output/tmp-h1-apply.mjs` 的**文件头注释**写「不进命令行、不进日志」，而第 42 行 `args` 里塞的正是裸 `url` ⇒ **注释断言与代码事实相反**（自证注释污染，本仓已记录过一类） |
| fix | ①探针新增 `ENV_PG_URL = AX_SCHEMA_PG_URL`（`resolve_pg_url`）；位置参数仍可用但会**显式告警**；**两个来源同时给出 ⇒ 报错退出**（连接目标歧义在 `--execute` 下最不可容忍，fail-closed）②`pg_render` 同步支持 ③驱动脚本改走 env 并改正注释 ④`usage()` 的示例行原本就是泄漏形式，已换 |
| verification | `output/tmp-p3-conflict.log`：`exit=2` + 歧义报错（用**假串** `postgres://u:p@h:1/d` 跑，避免「为测泄漏而制造泄漏」）· 矩阵脚本内置逐份日志的**泄漏自检**，5 份日志 0 告警 · `output/tmp-p3-matrix2-outer.log` 全程无口令 |
| 泄漏面量化 | `output/tmp-p8-leakscan.mjs`（只输出文件名/行号，绝不打印口令）：**11 个文件 / 14 处**。`output/` 与 `.workbuddy/` 均被 `.gitignore` 覆盖（`:134` / `:52`）⇒ **纯本地泄漏，未进版本库** |
| ⚠ 同族未修 | **同型调用还有 3 个 example**（`wiki_edge_relation_probe.rs:19`、`port_axiom_gate_probe.rs:17`、`p3_plan_probe.rs:76`）与 `harness/examples/port_axiom_audit.rs:19` 的 `psql "$(...url)"` 文档式。<br>不擅改的理由：跨 crate（`dao` / `harness`）×4 份**同一段解析逻辑**，正确做法是**收成一处共享实现**（避免 4 份手抄各自腐烂），选址属设计决策 ⇒ 登记待裁。 |
| ⚠ 特别提示 | `output/backup-2026-09-13/tmp_decrypt_pg.py:3` 仍是**明文口令写死**——正是 `scripts/pg-connect.mjs` 文档里声明「**已删除**」的那个脚本，它活在备份目录里（备份目录不进版本库，故仅本地风险）。 |

### ② 缺陷 T2：`--pg-render` 不读白名单 ⇒ **两条路径对同一批表给出矛盾结论**

同一天、同一份 plan、同一个库（白名单已含 2 条人工豁免）：

| 路径 | 孤儿候选 | DROP TABLE | 丢数据变更 | 对 `opc_capability`（324 行真数据）的结论 |
|---|---|---|---|---|
| `<pg-url>` 生产 dry-run | — | 0 | 0 | 不判孤儿（白名单生效） |
| `--pg-render`（修前） | 2 | 2 | 2 | **要删**，且打印「真跑时每条都要导出证据，缺一条即整批被拒」 |

后者的那两句是**假的**：真跑时这 2 条根本不在 plan 里，既不导证据也不延迟。
只读模式的用途是「在真库数据上验证渲染器」，读的人会把它当成真实计划 ⇒ **必须同口径**。

- root cause：`pg_render` 调三参数 `plan::diff(&want, &actual, dialect)`，白名单只在
  `apply::cycle` 与探针的 `--export-evidence` 分支里被读 ⇒ 与 ⑨ 同族
  （**机制存在、第二个消费端没接**）。
- fix：新增只读 `read_only_whitelist()`（先 `to_regclass` 探存在性 —— **不能**用
  `ensure_meta_tables`，那会建 4 张表，破坏本模式「生产库连一张表都不会多」的承诺），
  改走 `plan::diff_with(..., PlanOptions { orphan_whitelist })`，并打印白名单状态。
- verification：**孤儿候选 / DROP TABLE / 丢数据变更 2 / 2 / 2 → 0 / 0 / 0**，
  且日志新增 `-- 白名单：生效 2 条：opc_capability、vec_…_meta_dup_backup`。

### ③ 缺陷 T3：`advisory` 在生产日志里**丢失 290 条**

- root cause：`ApplyOutcome` **没有** advisory 字段、`report()` 也从不打印 ⇒ 同一份 plan、同一次 diff，
  `--pg-render` 日志里 273 条 `text-diff`，生产路径 **0 条**。
  而 advisory 机制自己的立项理由就是「**而不是无声无息**」（`extras.rs:608`）——
  生产路径把它的唯一价值丢掉了。
- fix：`ApplyOutcome.advisories`（自 `plan.advisories` 克隆）+ `report()` 新增「声明漂移」段。
- verification：生产 dry-run 日志 **25 行 → 317 行**，`声明漂移 290 条` 全量可见。
- **加总机械闭合**（这是本轮最有价值的一条副产物）：

  | 类 | 条数 | 含义 |
  |---|---|---|
  | `!! text-diff …` | 273 | PG 对表达式文本的重写（见 ⑤） |
  | `!! exempt: 表 …不判孤儿` | 14 | 豁免规则逐表说明 |
  | `!! pattern: 表 …动态覆盖` | 3 | `PATTERN_TSV` 运行时表（`vec_*_meta`） |
  | **合计** | **290** | 14 + 3 = **17**，与 §十一·九 归因出的 17 张差集表**逐一对应** |

  且**比 §十一·九 的归因更细**：当时把 6 张 `vec_*` 一并归给 `ORPHAN_EXEMPT` 规则 1，
  实测是**两条不同规则** —— 3 张 `vec_*` 主表走 `exempt`、3 张 `vec_*_meta` 走 `pattern`。

### ④ 缺陷 T4（**登记，需裁决**）：收敛判据结构性恒假 —— 根因比 §十一·九 ⑦ 更深一层

§十一·九 把 `converged = false` 归给「17 张豁免表在实况不在期望 ⇒ 指纹天然不等」。
本轮把 `--pg-render` 与生产路径的 `text-diff` 精确取证后，发现**还有第二重、且更致命**：

> `plan.rs:1587-1590` 的测试注释原话：
> 「表达式文本差异**不得**产出变更（PG 会重写 default/generated 文本）。
> 这是 P3 出口判据可满足的前提：库侧 1333 列 `text` 与 PG 补的 `::text` 转换属于同一类噪声，
> **若判成差异，plan 永远清不空**。」

实测 273 条**全部**是这一类噪声，无一为语义漂移：

| 后缀 | 条数 | 样例 |
|---|---|---|
| `default` | 266 | 实况 `'general'::text` ｜ 期望 `'general'`；`true` vs `TRUE`；`0.0` vs `0`；`'0'` vs `0` |
| `generated` | 5 | PG 给 `COALESCE(...)` 补括号 |
| `check` | 2 | `= ANY (ARRAY[…])` 的渲染差异 |

⇒ **同一个噪声，两个判据，只堵了一个**：`diff` 已把它正确降级为 advisory（专门写了测试），
而 `fingerprint` = `SHA-256(canonical_json())` = **整个模型逐字段序列化**，表达式文本全在里面
⇒ `post_fingerprint == expected_fingerprint` 在 PG 上**永不成立**。
（`fingerprint` 在引擎里只有两个消费端：`plan.rs:427-428` 写进 Plan 供诊断、`apply.rs:776` 判收敛；
`_ax_schema_state` 那张 A/B 快路由表**尚不存在**。）

- **修法建议（已修订）**：§十一·九 ⑦ 的候选 (a)「在 `fingerprint` 前 `retain` 掉豁免表」
  **已被证明不充分**（只解决 17，不解决 273）。建议改为 **(c)**：
  `converged` 改判「**apply 后再 diff 一次，是否为空**」—— 即与 P3 出口判据**同一个等价关系**，
  指纹相等降级为短路/诊断。这样两重问题一次解决，且豁免表、白名单、延迟项三者的语义
  **自动正确**（豁免表不进 plan ⇒ 不产变更 ⇒ 收敛；延迟的 DropTable 仍在 ⇒ 不收敛，与原文档一致）。
- **未擅改的理由**：这改的是 PLAN 明确拥有的 **P4 出口判据定义**，不是实现细节。
  §十一·九 ⑦ 已把「判据 vs 实现」的取舍标为待裁决，本轮只把证据补强、不动判据。
- 顺带修的**诊断文案**（不改判据值）：`report()` 原写「指纹不等，需逐条归因」，把一个**已知口径差**
  说成了**未知故障** —— 实测代价就是读日志的人去逐条查一个不存在的结构差异。
  现改为：plan 为空时直接说明「结构层面已一致 + 已知口径差是什么 + 结构收敛请看逐条是否为空」。

### ⑤ 附带：与代码事实相反的注释（已修）

| 位置 | 原话 | 事实 |
|---|---|---|
| `extras.rs:610` | 「两侧走**同一个** `normalize_default`（折空白 + 剥配平括号），故当前会逐字相等」 | `normalize_default` **只在实况侧被调用**（`introspect/pg.rs:103`、`introspect/sqlite.rs:211`），期望侧从不调用。该 8 列能逐字相等靠的是「文本手工抄自 `pg_get_expr`」这条纪律 —— 实测这 8 列 **0 条** advisory（结论对、理由错）。已改正并补上「全库 266 条其它列差异」这一事实。 |
| `output/tmp-h1-apply.mjs`（文件头） | 「连接串…**不进命令行、不进日志**」 | 同文件第 42 行把裸 url 塞进 `args` ⇒ 见 T1。已改正并把连接串改走 env。 |

### ⑥ 证据索引（本轮）

| 文件 | 内容 |
|---|---|
| `output/tmp-p1-dryrun-prod.log` | 泄漏第一现场（`:26` cargo 回显 argv） |
| `output/tmp-p2-check.log` · `tmp-p5-check.log` | 分阶段 `cargo check -q -p axagent-dao [--all-targets]` ⇒ `CHECK_EXIT=0` |
| `output/tmp-p3-{sqlite,coverage,pgrender,dryrun,conflict}.log` | **修复后**的全模式矩阵（5 例；`conflict` 期望 exit 2 且已达成） |
| `output/tmp-p9-matrix2-outer.log` · `tmp-p3-summary.txt` | 矩阵驱动输出 + 逐例退出码，含**逐份日志的泄漏自检** |
| `output/tmp-p8-leakscan.mjs` · `tmp-p8-leakscan.log` · `tmp-p8-leakscan.json` | 泄漏面扫描：**11 文件 / 14 处**（只输出文件名与行号） |
| `output/tmp-p6-fmt.log` | `cargo fmt -p axagent-dao` ⇒ `FMT_EXIT=0` |
| `output/tmp-p7-test.log` | `cargo test -p axagent-dao --lib` ⇒ **383 passed / 0 failed / 0 warning**（382 → 383，+1 守卫） |
| `output/backup-2026-09-16/PLAN-declarative-schema-sync.before-s11-9.md` | §十一·九 回填前的完整副本（回滚用） |

### ⑦ 遗留（**本轮新增/变更**；其余见 ⑩）

1. 🔴 **T4 收敛判据**：根因已定位到二层（17 表 + 273 文本），建议改判「apply 后 diff 为空」（§④）。
2. 🔴 **凭据同族**：3 个 example + 1 处 `psql` 文档式仍在 argv 里传连接串（T1「同族未修」）。
   建议先**轮换 PG 口令**，再决定是否收成共享 `resolve_pg_url`。
3. ⚠ `output/backup-2026-09-13/tmp_decrypt_pg.py` 明文口令仍在（`.gitignore` 覆盖，纯本地）。
4. ⑩ 的 1–4 项**仍未推进**（`opc_capability` 去留 / 8 张侧车库空壳 / 闸 3 持久化端）。

## 十一·十一 P6 收尾：迁移清单清空（74 → 0）与 74 个迁移文件删除（2026-09-16）

> 本轮把「建表来源」从**版本化迁移**整体切到**声明式引擎**，删掉全部 74 个
> `migrations/vNNN_*.rs`。目标状态（`MIGRATIONS = &[]` + 引擎独占建表）**已达成并全量验证**。
> 本节只登记**代价**与**移交不清的东西**，不重复 §十一 系列已登记的 P4/P5 内容。

### ① 本轮做了什么（可核对的数字）

| 项 | 前 | 后 | 判据 |
|---|---|---|---|
| `migrations/vNNN_*.rs` | 74 | **0** | 目录内 `.rs` 仅剩 `mod.rs` / `pg_ddl.rs` / `schema_diff.rs` |
| `MIGRATIONS` 注册表 | 74 条 | **`&[]`** | `migrations/mod.rs:93`，含「不可跟着删的三项」文档 |
| `migrations/mod.rs` | 1330 行 | 704 行 | 删 74 条模块声明 + 15 条已退役用例，新增 3 条改判用例 |
| `dao` 全量测试 | — | **308 passed / 0 failed** | `cargo test -p axagent-dao`：lib 292 + 集成 16（2+7+1+5+1） |

**删除前完整备份**：`output/backup-2026-09-16/dao-src-migrations/`（77 文件，逐文件 sha256 与删除前一致）
＋ `mod.rs` / `extras.rs` / `expected.rs` / `lib.rs` 四个文件的改动前副本。

### ② 为什么能删：先解掉一个编译硬阻断（否则删文件 = 编译失败）

`reconcile/extras.rs:675` 的 `FUNCTIONS[0].render` 指向
`crate::migrations::v227_cjk_fts::render_function_sql` ⇒ 删 `v227` 直接编译不过（E0432）。

修法不是「就地复制一份」（那是两份定义各自腐烂），而是**提取 + 转出**：

| 件 | 内容 |
|---|---|
| `dao/src/cjk_ngram.rs`（新建） | `CJK_CLASS` / `SEP_CLASS` / `FUNCTION_SQL_TEMPLATE` / `render_function_sql()` 的**唯一定义处**，附 2 条判据 |
| `dao/src/sql/ax_cjk_ngram.sql`（从迁移目录移来） | 63 行 SQL 模板，**逐字节相同**（sha256 核验），仅 1 行注释里的出处文件名改动 |
| `migrations/v227_cjk_fts.rs` | 删掉 53 行常量块，改为 `pub use crate::cjk_ngram::{…};`（既有引用路径仍可用，且**不产生第二份副本**） |
| `scripts/check-ngram-consistency.mjs` | 两个硬编码路径改指新位置（脚本实跑 **39 项检查全通过**） |

模块文档写明了**为什么函数定义与数据修复迁移的生命周期不同**：函数是长期契约（索引与查询两侧必须逐字节一致，否则静默失配），重建生成列是一次性数据修复。

### ③ 覆盖缺口审计：唯一性语义 **14 个载体**逐条核对（本节经两次纠错，前两版都错）

> ⚠ **本节是第 3 版结论。把前两版留在这里，因为「错在哪」比正确结论更值钱。**
>
> | 版本 | 谁得出 | 结论 | 判定 |
> |---|---|---|---|
> | v1 | 我，删迁移当日 | 「只有 **1** 处真功能损失（`keyword`），其余 8 条是误报」 | ❌ **错**。我把 `idx_opc_..._slug` 一类归为「唯一索引而非旧索引名 ⇒ 误报」，而实体侧**根本没承接** ⇒ 它们是真损失 |
> | v2 | 独立验证者 `Verify-1` 对抗复核 | 「另有 **5** 列 UNIQUE 语义无承接」并列出实体 file:line | ⚠ **对，但不完整**：只覆盖它抽查到的 5 条，没有穷举 |
> | v3 | 穷举脚本（本节） | **14 个载体 = 8 单列 + 6 复合；修复前 5 处损失，修复后 0** | ✅ 可复现 |
>
> **两次错的根因不同，都要记住**：
> - v1 错在**没查消费者就下结论** —— 只看「索引名字像不像」，没看实体有没有 `unique` 承接。
> - v3 自己的第 1 稿也错了一次：它把**复合**唯一键**逐列展开**，再用「单列列标志」**一套**判据去量
>   ⇒ 凭空报出 **13 条假损失**（`fleet_messages` 3 列、`news_archive` 2 列…）。
>   真相是**唯一性有两条承接通道，判据必须按载体形态分流**（见下表右列）。
>   同族纪律：「统计量恰好整齐 ⇒ 先怀疑测量工具」「反向断言须锚定被测对象自身形态」。

**可复现方法**：`node output/tmp-p3-unique-inventory.mjs`（只读）。它做三件事 ——
① 从备份迁移里按**载体形态**抽出全部唯一性载体（`CREATE UNIQUE INDEX` / 建表列级 `UNIQUE` / 建表表级 `UNIQUE (…)`）；
② 从 `entities/src/*.rs` 抽 `#[sea_orm(unique)]` 列标志；③ 从 `reconcile/extras.rs` 抽 L2 `IndexDecl`。
另加一维：载体若被**后续迁移显式 DROP**（`v138:51` 干掉 `v133` 的 dedupe 索引）则记 `SUPERSEDED`，不计入损失。

| # | 载体 | 形态 | 迁移出处 | 承接通道 | 状态 |
|---|---|---|---|---|---|
| 1 | `opc_demand_subscriptions(keyword)` | 单列 | v135:45 `CREATE UNIQUE INDEX` | 实体 `#[sea_orm(unique)]` | ✅ 已修（v1 版本修的） |
| 2 | `sync_devices(unique_id)` | 单列 | v116:41 内联 `UNIQUE` | 实体列标志 | ✅ **本轮修** |
| 3 | `sync_permissions(device_id)` | 单列 | v116:182 `… ON DELETE CASCADE UNIQUE` | 实体列标志 | ✅ **本轮修** |
| 4 | `opc_landing_pages(slug)` | 单列 | v210:12 内联 `UNIQUE` | 实体列标志 | ✅ **本轮修**（v1 曾误判为「误报」） |
| 5 | `opc_blog_posts(slug)` | 单列 | v210:25 内联 `UNIQUE` | 实体列标志 | ✅ **本轮修**（同上） |
| 6 | `trajectory_preferences(key)` | 单列 | v100:738 内联 `UNIQUE` | 实体列标志 | ✅ **本轮修** |
| 7 | `gateway_keys(key_hash)` | 单列 | v100 内联 `UNIQUE` | 实体列标志 | ✅ 本来就有 |
| 8 | `program_policies(program_name)` | 单列 | v100 内联 `UNIQUE` | 实体列标志 | ✅ 本来就有 |
| 9 | `fleet_messages(fleet_id, conversation_id, seq)` | **复合** | v229 `uq_fleet_messages_convo_seq` | L2 `extras.rs:869` `unique: true` | ✅ 本来就有 |
| 10 | `news_archive(source, article_code)` | **复合** | v200 表级内联 `UNIQUE` | L2 `extras.rs:950` | ✅ 本来就有 |
| 11 | `opc_demand_leads(platform, content_fingerprint)` | **复合** | v138 `idx_…_fingerprint` | L2 `extras.rs:1004` | ✅ 本来就有 |
| 12 | `session_events(session_id, seq)` | **复合** | v139 `idx_…_session_seq` | L2 `extras.rs:1040` | ✅ 本来就有 |
| 13 | `workflow_tools(workflow_id, tool_name)` | **复合** | v123 表级内联 `UNIQUE` | L2 `extras.rs:1139` | ✅ 本来就有 |
| 14 | `opc_demand_leads(platform, source_url)` | 复合 | v133 `idx_opc_demand_leads_dedupe` | —— | **SUPERSEDED**：v138:51 已显式 DROP，被 #11 取代 ⇒ 本就不该有承接 |

**结论：修复前 5 处真损失（#2–#6），修复后 0 处。** 穷举脚本对修复后的源码给出
`OK_ENTITY 8 / OK_L2 5 / SUPERSEDED 1`，**正例 14 条全中、反例（SUPERSEDED）1 条正确识别** ⇒ 判据有区分力。

**本轮修复动作**：5 个实体列各加 `#[sea_orm(unique)]` 并**移除同列的 `#[sea_orm(indexed)]`**
（`sea-orm-2.0.2/src/schema/entity.rs:156` 的条件是 `indexed && !unique` ⇒ unique 列不派生普通索引，
留 `indexed` 只是「声明了但没产出」的死标志）；注释里写明迁移出处，并注明
「靠**列标志**保住语义，不是靠那个索引名字」。

**顺带补的三处判据**（都属「同型缺陷按量纲穷举」）：
1. `migration_unique_semantics_survive_in_column_flag` 从「只查 `keyword` 一条」扩成
   **6 条表驱动**（每条带「同表反例列必须不 unique」的区分力断言 × 双方言）。
2. 新增 `composite_unique_decls_still_unique`：**5 条复合唯一键**（`fleet_messages` /
   `news_archive` / `opc_demand_leads` / `session_events` / `workflow_tools`）按
   「列集（**有序**）匹配」锚定 L2 声明的 `unique: true` ——
   实体列标志**表达不了复合键**，这是它们唯一的承接判据。
3. `l2_static_decls_all_resolve_to_entity_tables` 加 `assert_eq!(hit.unique, d.unique, …)`。
   ⚠ **必须标明它的能力边界**：`overlay_l2` 是 `t.upsert_index(IndexModel { unique: d.unique, .. })`
   —— **覆盖式**写入 ⇒ 这句只能挡「`overlay_l2` 不再透传 unique」这类**实现回归**，
   **挡不住**「有人把 `extras.rs` 里某条声明改成 `unique: false`」（模型会忠实跟着变，恒绿）。
   真正挡后者的是上面第 2 条。
   —— 这个边界是我写的时候差点搞错的：一开始以为加了这句就守住了复合唯一性，
   读了 `overlay_l2` 才发现它**是自证化的**。教训：**判据要锚事实，不要锚镜像**。

**存量库实证（`output/tmp-p2-dup-check.log`，只读探针）**：本机 `axinvest` 库上这 6 张表
**全部 `rows=0`** ⇒ 既有数据无重复、无 NULL 冲突 ⇒ 「补 `ADD UNIQUE`」**零风险**。
但**唯一性在存量库上确实已丢**：6 条目标列在 `pg_indexes` 里只剩**非唯一**的
`CREATE INDEX`（其中 `opc_demand_subscriptions.keyword` 连普通索引都没有）；
`pg_constraint` 里除 `_pkey` 与 `sync_permissions_device_id_fkey` 外无任何唯一约束。
**是否给存量库补 `ADD UNIQUE` 是独立的产品/运维决策**（新库已由实体属性保住），
本轮**只登记不动手**，见 ⑨ 遗留。

> ⚠ 探针自己踩过一次「判据读错对象」：首版用 `/UNIQUE/i.test(indexdef)` 判唯一，结果
> `idx-sync_devices-unique_id` 因**索引名里含 "unique"** 被误标为 UNIQUE。改为锚定
> `^CREATE UNIQUE INDEX` 前缀后修正（正例 `*_pkey` 全中、反例目标索引全不中）。

**普通索引类（非唯一）归因 —— v1 这部分是对的，保留**：
`opc_demand_leads` 的普通索引 / `price_alerts.alert_type` / `opc_kpi_records.name`·`period` /
`opc_revenue_records.category` / `opc_follow_up_tasks.status` —— 都已有 `#[sea_orm(indexed)]`；
`opc_demand_subs_due` 是纯性能索引。（穷举脚本未把 `price_alerts` / `opc_kpi_records` /
`opc_revenue_records` / `opc_follow_up_tasks` 列为**唯一性**载体 ⇒ 它们确实不是唯一性损失。）

**三张无实体死表**（`rl_policies` / `chat_runs` + `chat_run_events` / 废弃的 `opc_demand_lead` +
`opc_market_platform`）**不需要改代码**：无实体 ⇒ 孤儿 ⇒ DROP 候选，是 P3 已定的设计；
`chat_runs` 的消费方（gateway handlers）用的是**进程内 `HashMap`**（`RunStore`），不是库表。
⚠ **不能**用 `ORPHAN_EXEMPT` 豁免它们 —— 那会把垃圾永久藏起来。

### ④ 数据操作迁移的移植状态（复核后**修正口径**）

我在上一轮口头给过「含数据操作 23 条」，复核后**该数字作废**。可复现的口径是：

| 口径 | 条数 | 明细 |
|---|---|---|
| `up()` 内含**行级 DML** | **10** | v101 / v104 / v119 / v131 / v137 / v203 / v219 / v225 / v232 / v233 |
| 数据**依赖**型 DDL（无 DML，但改写/依赖存量行） | **+3** | v227（生成列全表重算）/ v228（加列保持 NULL）/ v230（带 DEFAULT 加列） |
| 合计（宽口径） | **13** | 我此前的「12」落在 10 与 13 之间，差异是**口径**而非漏项 |

复核方法（可重跑）：按 `mod tests` 切分，只在**非测试区、非注释行**、大小写不敏感匹配
`INSERT [OR IGNORE] INTO | UPDATE <t> SET | DELETE FROM | REPLACE INTO`，人工排除
`ON DELETE CASCADE/SET NULL`（FK 子句）与 `CREATE TRIGGER … AFTER INSERT/UPDATE/DELETE`（触发器定义）；
v119 无 SQL 关键字，另按 SeaORM `ActiveModel.update()` 单独识别。
**建议采信「行级 DML 10 + 数据依赖型 DDL 3」。**

- **已移植 1 条**：v101 的两个哨兵行 → `dao/src/seed.rs::ensure_sentinels`
  （挂在 `db::initialize_schema` 链上 = 生产与测试共用；`on_conflict_do_nothing()` + `exec_without_returning`，
  **无方言分支**）。归属边界写在模块文档里：✅「全新库从零建起也必须存在的行」；
  ❌「存量库的一次性数据修复（历史事件）」。
- **未移植 12 条**：价值判断见 ⑤。

### ⑤ ⚠ 随文件消失的用例：106~109 条，其中 **9 条是真损失**

删 74 个文件同时删掉了它们内部的 `#[cfg(test)]` 用例。**这是本轮最大的隐性代价。**

| 口径 | 数量 |
|---|---|
| 按属性行精确数（独立复核） | **106**（`#[tokio::test]` 102 + `#[test]` 4；分布在 49 个文件，25 个文件无测试） |
| 按用例总数差分反推 | **109**（410 − 15 退休 + 6 新增 − 292 实测） |
| 差 | **3，未消**（已登记）。不影响结论：106~109 同量级、方向一致 |

按被测对象分类：

| 类 | 数量 | 内容 | 替代物 |
|---|---|---|---|
| (A) DDL 形态断言 | **97** | 表/列/索引/约束存在、DDL 语法、**幂等重跑 42**、往返写入 3 | 引擎 `apply.rs` / `expected.rs` / `bootstrap_schema` 已覆盖 ⇒ **可接受损失** |
| (B) 数据搬迁/回填语义 | **9** | 见下表 | **无替代物 ⇒ 真损失** |
| (C) 版本号/版本表耦合 | **0** | —— | `CURRENT_VERSION` / `schema_migrations` 在测试区**零命中** |

**(B) 类 9 条逐条**（出处均在备份目录）：

| # | 出处 | 用例 | 断言的数据语义 |
|---|---|---|---|
| 1 | v101:469 | `sentinel_kb_enabled_is_integer_zero` | 哨兵 KB 的 `enabled` 必须是**整数 0**（不是布尔 false/1） |
| 2 | v101:481 | `sentinel_ids_are_non_empty` | 哨兵 id/name 非空（空值会插出无归属行） |
| 3 | v101:493 | `v101_is_self_idempotent_and_creates_sentinels` | 幂等建出两哨兵**行** + 三张 `trajectory_*` 旧表被 DROP |
| 4 | v131:54 | `v131_backfills_wiki_entities_only` | 回填 `source_type/source_id` 只命中 wiki 实体，**真实 KB 实体不受影响** |
| 5 | v137:125 | `v137_disables_only_tokenless_rows_sqlite` | 只治理「无/空 token」行，「已配 token」行保留 `enabled=1`（不误伤） |
| 6 | v203:117 | `v203_backfills_legacy_above_below_correctly` | `above→take_profit` / `below→stop_loss` / `change_up→change` 的映射 |
| 7 | v225:113 | `v225_columns_and_backfill` | `executing` 行回填授权三列；`reviewing` 行**必须保持未授权** |
| 8 | v228:147 | `v228_does_not_backfill_history` | **反向断言**：历史行必须保持 NULL，不得用 `decision_action` 事后推导值冒充记录值 |
| 9 | v230:257 | `v230_is_safe_on_table_with_existing_rows` | 带 DEFAULT 加列后存量行填 `''`（语义=「未标注」），且不因 NOT NULL 失败 |

其中 #1/#2 是常量级（不连库），#3–#9 是真实行级（造存量行再断言）⇒ **行级真损失 = 7 条 / 6 文件**；
而 **#1–#3 已由 `seed.rs` 的两条测试承接**（`ensure_sentinels_restores_deleted_rows` 就是「先删再补」的版本）
⇒ **实际未承接的行级真损失 = 6 条**（v131 / v137 / v203 / v225 / v228 / v230）。

**⚠ 两条最重的数据迁移本来就没有任何测试**：
`v232_opc_domain_pack_rename.rs`（存量改名 89 行，含**主键改名 + 子表外键回填 + `REPLACE`**）与
`v233_opc_consulting_expert_ids.rs`（FK 双向互锁的「先解引用 → 改名 → 回填」三步）
在 74 个文件里**测试数为 0**。也就是说：10 个数据操作迁移里只有 5 个留有 (B) 类测试；
**v104 / v119 / v219 / v232 / v233 在删除前就是零覆盖**。

**⚠ v104 的覆盖藏在 `mod.rs`、不在自己文件里**：`mod.rs` 那 15 条里有 `v104_notes_fts_objects_exist` /
`v104_is_self_idempotent`，而 v104 是**真正做了 FTS 数据回填**的迁移（`INSERT INTO notes_fts … SELECT`）。
这 15 条已退休 ⇒ v104 现在**彻底没有测试**（它的对象存在性其实由
`migrations::tests::l2_declared_indices_reach_ddl_on_engine_built_db` 与 `schema_compliance.rs` 部分代偿）。

### ⑥ ⚠ 跨方言一致化把 SQLite 的外键「变真了」（**本轮引入的行为变化**）

删掉的不只是建表语句，还有**约束的宽严**。这是本轮最容易被忽略的一类后果。

| | 迁移时代（SQLite） | 引擎时代（SQLite **新库**） |
|---|---|---|
| `knowledge_entities.knowledge_base_id` 外键 | **不存在**（`v100_consolidated.rs:588-592` 的 SQLite 分支只有 `knowledge_base_id TEXT NOT NULL`）；实体侧 `belongs_to` 是 2026-09-16 P1 才补的，而 SQLite 无 `ADD FK` 原生形态 ⇒ **补不上** | **内联在 `CREATE TABLE` 里 + `PRAGMA foreign_keys=ON` ⇒ 真的生效** |

实测证据（非推测）：`dao/tests/knowledge_graph_search.rs` 用 `kb_x` / `kb_main` / `kb_other` 三个
**不存在的**知识库 id 插实体，迁移时代一直通过；引擎接管后当场 `code 787 FOREIGN KEY constraint failed`（2 条用例）。

- **已修**：该文件夹具补父行（`seed_kb`），理由写进注释 —— 它测的是**关系过滤**，与外键无关，
  正确修法是补父行，而不是关掉外键把缺陷盖回去。
- **必须登记、本轮不擅改**（需产品语义决策）：

| 位置 | 情况 | 判定 |
|---|---|---|
| `src/init/services.rs:271-275` | 把 **memory namespace id**（或 `"memory_default"`）当 `knowledge_entities.knowledge_base_id` 用 | 外键一生效必然 787。生产 PG 侧同一个外键**早已存在**，而 prod 实测该路径**一行都没插进去**（`knowledge_entities` 只有 2 个真实 KB 取值、`''` 为 0）⇒ **该路径在 PG 上早已静默失效**，错误被 `tracing::debug!` 吞掉。⚠ 不是本轮引入的，但本轮让它在 SQLite 新库上也失效（此前 SQLite 会插进垃圾行） |
| `dao/src/repo/knowledge_graph.rs:745 upsert_relation`<br>（行号漂移史：`:688` → `:787` → 现 `:745`；来源：`PLAN-memory-kb-reflow-id-space.md` §5d 类 C-#1） | **两个缺陷叠加**：① **零调用**（全 workspace 仅定义 + 注释 ⇒ 从未写入过任何行；判别式：全表 `knowledge_base_id = ''` 行 = **0**。⚠ `capability_relationship.rs` 那个同名 `upsert_relation` 是同名**不同**表、不同签名的另一实现，不是它）；② `on_conflict(Id)` 打在 `:768` 刚 `gen_id()` 出来的 id 上 ⇒ **永不触发** ⇒ 幂等性为零；③（同族第三处）写死 `knowledge_base_id = ""` ⇒ 一旦接线即 787 | ✅ **2026-09-17 已修**（用户授权）。三处一起修：**主键改为由自然键派生**（新增契约层 `harness::knowledge_graph::stable_relation_id`，**唯一实现处**，`dao` 与 `src/commands/memory.rs` 共用）· `knowledge_base_id` 空串 → **哨兵**（该函数 doc 自述 `trajectory-style save`，而同用途下全部同族写读端都用哨兵：`src-tauri/crates/trajectory/src/storage.rs:963` 写、`:1016` 读、`causal` 读、`proactive` 读 ⇒ 空串不是「另一种产品语义」）· 删已不用的 `gen_id` import。**为何用确定性主键而不照抄 `upsert_entity` 的 read-then-write**：本函数**零调用** ⇒ 表里没有「由它写入的存量行」需要被 `find` 回来；而 `upsert_entity` **有**存量随机 id 行，改确定性主键会找不到旧行、**反而插重**。**仍未做**：表上仍无 `(kb, source, target, rtype)` 唯一约束 ⇒ 跨写者去重仍缺；`relation_type = "causes"` 归 `causal::observe_edge`。完整取舍见 `PLAN-memory-kb-reflow-id-space.md` §5d-补2 |
| `crates/trajectory/src/storage.rs:517 save_pattern`<br>（来源：`PLAN-memory-kb-reflow-id-space.md` §5d 类 C-#2） | `on_conflict(Id)` 打在 `TrajectoryPattern::new`（`harness/trajectory_types.rs:227`）每次新生成的 `Uuid::new_v4()` 上 ⇒ 永不触发。**生产实态：2038 行 / 仅 3 个不同 `name`，且今天（`2026-09-17T05:13Z`）仍在增**（写者唯一 = `storage.rs:496`）。调用方是周期任务：`src/init/services.rs:838`（15 分钟）· `:886`（30 分钟）。**另有第二个独立缺陷**：`trajectory/src/pattern.rs:118` 写 `learned_patterns` 用的键是 `pattern.name`，而 `:138` 查表用的键是 `tool_seq_{序列}` ⇒ **同一 `HashMap` 两套键名空间，查表恒 miss** ⇒ 内存内去重这层也是死的 | ✅ **2026-09-17 已修**（用户授权）。修法**不是**在 repo 层按 `name` upsert，而是**让主键成为自然键的编码**：`TrajectoryPattern::new` 改用 `stable_id_for_name(name) = "pat_{name}"`（`harness/src/trajectory_types.rs:240`）⇒ `ON CONFLICT (Id)` 真的会命中；并修掉 `trajectory/src/pattern.rs` 里 `learned_patterns` 的两套键名空间、统一 `frequency` 口径、补全 upsert 更新列。**为何不能在 repo 层按 `name` upsert**：本表还有第二个写者 `src/commands/rl_training.rs:279`（检查点，**身份 = 调用方给的 `id`**，`load_checkpoint:326` 按 id 命中、`list_checkpoints:359` 按 `ckpt.id` 去重）⇒ 按 `name` upsert 会把**同名不同 id 的检查点静默压成一行**。完整取舍与验证见 `PLAN-memory-kb-reflow-id-space.md` §5d-补。⚠ **存量 2038 行 / 3 个 `name` 未清理、`UNIQUE(name)` 未加**（两者都需另授权） |
| `crates/trajectory/src/storage.rs:991 save_relationship`<br>+ `src/commands/memory.rs:1319`<br>（来源：`PLAN-memory-kb-reflow-id-space.md` §5d 类 C-#3） | `on_conflict(Id)` 打在调用方每条关系新造的 `format!("rel_{}", &uuid[..12])` 上 ⇒ 永不触发。关系循环**无任何查重**（对比：**同一函数**的实体循环有按 `(name, entity_type)` 的前置查重）。码级确证，**当前零足迹**（库中无 `rel_{12 位十六进制}` 形态行 ⇒ 该命令尚未被跑过） | ✅ **2026-09-17 已修**（用户授权）—— ⚠ **修法判定被现场推翻过一次**：该 storage 函数有**两个**调用方，`trajectory/src/memory_providers/service.rs:1311-1323`（实体合并把关系重定向）传的是 `rel.clone()` 的**原行 id** ⇒ `on_conflict(Id)` **会命中并就地改写那一行**（合法用法）⇒ **缺陷不在 storage，在 `memory.rs` 这侧**。故只改 `memory.rs`（id 改为由自然键派生，走 `harness::knowledge_graph::stable_relation_id`），`storage.rs` 仅补 doc 把 **id 契约**写死（两种合法用法 + 禁止随机 id）。⚠ **不追溯存量**（旧随机 id 行找不到 ⇒ 每逻辑关系最多并存「旧随机行 1 + 新派生行 1」）。⚠ 附带发现（已固化判据 **#478**）：自然键必须取**实际落库的值** —— `RelationshipType::from` 会把未登记值折叠成 `RelatedTo`（Display = `related_to`），用原始输入 `"Causes"` 派生会得「两个键、同一个列值」⇒ 去重照样失效。完整记录见 `PLAN-memory-kb-reflow-id-space.md` §5d-补2 |
| `crates/dao/src/repo/knowledge_graph.rs:1144` 批量导入 | **有**存在性检查（`:1131-1143` 按 `(kb, source, target, type)`）却仍产生重复 ⇒ 检查**有漏**。生产实态：`rel_{uuid}` **788 行 / 767 个逻辑键 ⇒ 20 个重复组**（全部 `knowledge_base_id = f9b2b050-e336-4de8-b305-0b40cbf5740f`） | ⚠ 归属已钉死：788 行**不属** `upsert_relation`（它零足迹）⇒ 属**此处**。需查「检查为何漏」（`continue` 后仍插 / 检查列与实际写入列不一致）+ 决定存量 20 组重复的清理策略 ⇒ 只登记不擅改 |
| `src/lib.rs:472-509` 暖启动**空转**<br>（来源：`PLAN-memory-kb-reflow-id-space.md` §5d-补「新发现」） | 开机把持久化 patterns 回灌 `PatternLearner` 的循环**零产出**：`:478-505` 合成的 `Trajectory` 把 `steps` 置为 `vec![]`，而四条抽取路径**都以 `steps` 为前置条件**（`tool_names.len() < 2` / `reasoning_steps < 2` / `error_indices` 空 / `tool_call_count < 3`）⇒ `learn_from_trajectory` 恒返回空 vec ⇒ 重启后 `learned_patterns` **恒为空**；而 `:507` 仍打印 `Loaded {pattern_count} persisted patterns into PatternLearner`，其中 `pattern_count` 取自 `persisted.len()`（**输入规模**，不是实际装载量）⇒ **日志与效果相反** | ⚠ 需产品语义决策：正解应是「新增一个吃 `TrajectoryPattern` 的注入入口，把持久化行直接灌进 `learned_patterns`」——这也会让 DB 聚合**跨重启累积**（当前重启后 in-memory 从 0 起算，upsert 会把 DB 行覆盖成进程内单次值）⇒ **只登记不擅改** |
| `trajectory/src/causal.rs:302`（`observe_edge` 的 INSERT）<br>+ `tests/causal_integration.rs` 整套变红<br>（来源：`PLAN-memory-kb-reflow-id-space.md` §5d-补 末段） | 第三个「**写死 `knowledge_base_id = ""`**」的写者（另两处：`src/init/services.rs:271-275`、`knowledge_graph.rs` 的 `upsert_relation`）。声明式引擎给 SQLite 新库建了 `knowledge_relations → knowledge_bases` 外键（实体名 `knowledge_relations.rs:35 belongs_to`；FK 由 sqlx 默认 PRAGMA 打开，见本文件 `:1279`）⇒ 无父行即 `code 787`。**修前实测：`cargo test -p axagent-trajectory --test causal_integration` = 4 passed / 17 failed，17 条全部同源于该 787**（未走边的 4 条全绿 ⇒ 失败签名一致）。生产侧**零足迹**：`causal_enabled` 默认关（测试 `storage_causal_disabled_by_default` 为证）⇒ `observe_edge` 从不执行，与「`knowledge_relations` 空串 kb 行 = **0**」的读数一致 | ✅ **2026-09-17 已修**（用户授权）。**修法 = 不是补 `knowledge_bases` 父行、也不是让它接受 kb 参数，而是改用已存在的 sentinel KB**（新增 `const CAUSAL_KB_ID` 转发 `harness::constants::sentinel::TRAJECTORY_KB_ID`，值不重复）。**为何哨兵才是正解（本轮比登记时更硬的证据）**：读端**早已**按哨兵过滤 —— `src/commands/proactive.rs:328/:568` 调 `list_causal_edge_stats(db, TRAJECTORY_KB_ID)`、`storage.rs:1016` 的 `get_all_relationships` 也 `.filter(KnowledgeBaseId.eq(Self::TRAJECTORY_KB_ID))` ⇒ 写端写 `""`、读端按哨兵查 ⇒ **即便关掉外键让 `""` 写成功，这些读端也一条读不到**。本项修的是**写读口径冲突**，不是把测试弄绿。**验证：改一处 `String::new()` ⇒ 17 条红全绿**（修后 **22 passed / 0 failed**，含新增守卫测试 `observe_edge_writes_sentinel_kb_visible_to_scoped_readers`）。⚠ 原本判定的「这张回归网在 C-#1 接线前是哑的」已随之解除。完整记录见 `PLAN-memory-kb-reflow-id-space.md` §5d-补2 |

**规模对照**：实体侧 `belongs_to` 共 **43** 条（39 个文件）；迁移非测试区 `REFERENCES` 共 **33** 处
（6 个文件，其中 v100 的 25 处走 `pg_ddl::exec_ddl` 转换路径）。
两者**不是同一口径**（文本出现 ≠ 一个约束），但方向明确：**引擎在 SQLite 上建出的外键 ≥ 迁移时代**，
差值集中在 P1 补的那 14 条 `belongs_to`。

### ⑦ 其余新老库形态分歧（本轮确认，均**无法**用列标志承接）

| 分歧 | 事实 | 后果 |
|---|---|---|
| **DESC 索引方向** | `idx_conversations_updated`（实况 `cols[updated_at DESC]`）被引擎 **DROP**（不是改名）：列集/unique 相同但**方向**不同，而实体声明里没有「方向」这一维 | `ORDER BY updated_at DESC` 失去有序索引。同族 4 条：`idx_dynamic_ui_schemas_updated` / `idx_market_mainlines_strength` / `idx_screenshot_diagnoses_created` / `idx_vec_collections_updated` |
| **改名匹配成功的一类**（对照） | `idx_provider_keys_provider` → **RENAME** 成 `idx-provider_keys-provider_id` | 无语义损失 ⇒ 证明上面的 DROP **不是命名差异**造成的 |
| **引擎不重建已有表** | 存量库上形态不同的对象（含生成列表达式、缺失的外键）**不会被改写**；`bootstrap_schema` 只做纯新增 | 存量 SQLite 库**永远拿不到** ⑥ 里那条外键 ⇒ **新库/老库长期分歧**，且分歧只体现在约束宽严上（不报错、极难发现） |
| **`repair_schema` 的 `fixed`** | 注册表清空后恒为 **0** | 它的语义是「重跑了几个迁移」，不是「修复了几个字段」。前端按钮文案仍按后者解释 ⇒ 恒显「已补 0 个字段」（遗留） |

### ⑧ 证据索引（本节）

| 文件 | 内容 |
|---|---|
| `output/backup-2026-09-16/dao-src-migrations/` | 删除前 74 个 v 文件的完整备份（77 文件，逐文件 sha256 一致） |
| `output/tmp-p6-clean-mod.mjs` | 带断言的迁移手术脚本（dry-run / `P6_OUT` 只改副本 / `--apply` 落盘并删文件） |
| `output/tmp-p3-probe-prod2.log` | 真库 plan 探针原文：`DROP INDEX idx_conversations_updated … cols[updated_at DESC]`、`RENAME INDEX idx_provider_keys_provider → idx-provider_keys-provider_id` |
| `output/tmp-d9-pgrender.log:76` | 渲染侧原文 `CREATE INDEX idx-conversations-updated_at cols[updated_at]` —— 判据里那个「连字符+下划线混写」名字的实证（sea-orm 硬编码 `idx-{表}-{列名原文}`，`sea-orm-2.0.2/src/schema/entity.rs:158`） |
| `output/tmp-p1-index-inventory.log` | 迁移索引全量清单：298 条（plain 239 / multi 28 / partial 12 / parse_failed 14 / unique 5）、FK 31 |
| `output/tmp-p3-unique-inventory.mjs` / `.log` | **唯一性载体穷举**（③）：按载体形态抽 `CREATE UNIQUE INDEX` / 建表列级 `UNIQUE` / 建表表级 `UNIQUE (…)`，逐条对实体列标志与 L2 声明；实测 **14 个载体 = 8 单列 + 6 复合**，`OK_ENTITY 8 / OK_L2 5 / SUPERSEDED 1` |
| `output/tmp-p2-dup-check.log` | **存量库重复值与现状索引**（③）：本机 `axinvest` 这 6 张表 `rows=0`；6 条目标列在 `pg_indexes` 里只剩**非唯一** `CREATE INDEX`（`opc_demand_subscriptions.keyword` 连普通索引都没有） |
| `output/tmp-p4-orphan-probe.log` | **43 条 `belongs_to` 孤儿行计数**（⑨·11）：全部 0 孤儿；区分力自证把 CLEAN 二分为「12 条非平凡 / 31 条表空平凡」 |
| `node scripts/pg-connect.mjs sql …` | prod 实测：`knowledge_bases` 只有 3 个 id（**无 `''`**）；`knowledge_relations` 112937 行且 `knowledge_base_id=''` 为 **0**；`knowledge_entities` 按 kb 分组只有 2 个真实 KB |
| `output/tmp-e2e-mutation-inject.log` | **判据区分力自证（变异注入）**：把 `fleet_messages` 的 CASE 列序颠倒成 `["seq","conversation_id","fleet_id"]` 后跑 `composite_unique_decls_still_unique` ⇒ **`FAILED`**，`:13` 报出实际索引 `[("uq_fleet_messages_convo_seq", ["fleet_id","conversation_id","seq"])]`。证明该判据抓得住「L2 声明被误改**列序**」——正是 `assert_eq!(hit.unique, d.unique)`（覆盖式 upsert ⇒ 恒真）抓不到的那一类 |
| `output/tmp-e2e-dao-lib-293.log` | 变异**还原**后全量 `cargo test -p axagent-dao --lib`：**293 passed / 0 failed**（`V8_EXIT=0`，重编 `dao` 1m19s ⇒ 格式改动与还原确实进了编译） |
| `output/tmp-e2e-fmt-mtime.mjs` · `tmp-e2e-fmt-guard.mjs` | 格式化前的**归属判定**探针：逐文件核 mtime，确认 21 处 fmt 违规所在文件全部「早于本轮改动、无 5 分钟内写入」 ⇒ 才动手；避免把并发会话的中间态格式化掉 |

### ⑨ 遗留（**本轮新增/变更**；其余见 ⑦ 与 ⑩）

1. 🔴 **6 条行级数据语义无承接**（v131 / v137 / v203 / v225 / v228 / v230，见 ⑤）。
   建议**逐条改写为「引擎建表后的等价判据」**，而不是重建迁移；v228 的反向断言尤其值得保
   （它防的是「用推导值冒充记录值」）。
2. 🔴 **v232 / v233 的改名语义删除前就零覆盖**，现在连迁移本体都没了 ⇒ 将来若要回放这两条改名，
   **无参照物**。建议把语义写进 PLAN 或 `docs/` 运维手册，不要只留在备份里。
3. ⚠ **SQLite 外键宽严的新老分歧**（⑥⑦）：是否为此实现「重建表以补齐约束」，与 §3.1 的
   6 类 SQLite 动作是同一件事。
4. ⚠ `init/services.rs:271` 的**错 id 空间**（memory namespace id 当 KB id 用）+ 错误被 `debug!` 吞掉
   ⇒ 该「Memory → 实体」回流功能在 PG 上早已是静默空转。**不是本轮引入的**，但与本轮同族
   （外键有效性），建议单独立项（含「该失败必须能被看见」）。
5. ✅ `knowledge_graph.rs` 的 `upsert_relation`（**零调用** + 写死空串 KB + `on_conflict(Id)` 永不触发）**已于 2026-09-17 修**（见上表：主键改为由自然键派生 + KB 改哨兵）。
   ✅ **同日补完 C-#2 的两件收尾**（用户指令「清 `trajectory_patterns` 存量 2038 行 + 加 `UNIQUE(name)`」）：
   - **存量清零**：PG `axinvest` 实测 **2038 行 / 3 个 `name`**（`tool-CapabilityView` 1019 · `multi-step-13` 525 · `multi-step-10` 494）
     ⇒ 先备份（`output/backup-trajectory-patterns-2026-09-17/`：`rows.json` 1.03 MB + `restore.sql` 2038 条 `INSERT`），
     再删 **2038 → 0**，恒等核对 ✔（删前 = 实际删除 = 备份行数）。**不丢不可重算的数据** ——
     `PatternLearner.learned_patterns` 初始为空、不从 DB 装载（`trajectory/src/pattern.rs:79` / `:90`）⇒ 这批行是纯派生输出。
   - **约束落地**：⚠ 原要求的**裸** `UNIQUE(name)` **不可行**。`rl_checkpoint:` 是三方一致登记的同名多行例外 ——
     ① `harness/src/trajectory_types.rs::stable_id_for_name` 的 doc（「需要同名多行的场景不要走 `new`」）；
     ② `commands/rl_training.rs` 用结构体字面量自定 `id`、`name` 才是 `format!("rl_checkpoint:{}", …)`；
     ③ `trajectory/src/storage.rs::save_pattern_keeps_distinct_ids_with_same_name_apart` 刻意锁住的反向守卫。
     ⇒ 改落**排除式 partial unique**：`uq_trajectory_patterns_name ON trajectory_patterns (name) WHERE (name !~~ 'rl_checkpoint:%'::text)`
     （PG 把 `NOT LIKE` 归一成 `!~~` + `::text`，实测即此形态）。已建入生产库并回读对账
     （`indisunique=t` / `is_partial=t` / `indisvalid=t`；where 段与 `extras.rs` 声明**归一后逐字一致** ⇒ 引擎下一轮 plan 对该索引为 no-op）；
     正负对照 4/4：非 checkpoint 同名第二行**被拒**，`rl_checkpoint:%` 同名第 2、3 行**均放行**。
   - ⚠ **刻意只声明 PG 侧**（`dialect: Some(Postgres)`，`extras.rs::PARTIAL_INDEXES` 第 6 条）：`render.rs` 对 SQLite 只做
     `strip_casts`，会留下 SQLite 不认的 `!~~` ⇒ 声明写成 `None` 会让 **38 处测试**用的 SQLite 文件库渲染失败。
     SQLite 侧要表达同语义须另写 `(name NOT LIKE 'rl_checkpoint:%')`，**本轮未做**（见下一条）。
   - ⚠ **SQLite 侧另有独立一份存量**（`C:\Users\polit\.axinvest\axagent.db`：**2748 行 / 6 个 `name`**，各 458 行；
     mtime 停在 2026-07-13 ⇒ 已非当前配置后端）—— **不在本轮授权范围内，未动**。
     ⚠ **订正（同日复核源码后）**：本处原先写「若切回 SQLite 会因存量同名行**建索引失败**」是**错的** ——
     `dialect: Some(Postgres)` 经 `expected.rs:427-433::l2_applies` 过滤 ⇒ 引擎在 SQLite 上**根本不会建这条索引**
     ⇒ 不会失败；但**收益也不存在**：SQLite 后端下 C-#2 的「同名覆盖」缺陷**依旧在**，那 2748 行也不受影响。
     ⇒ 真正待裁决的是「**是否要为 SQLite 侧也表达该约束**」，而**单条声明做不到**，三条路都不通：
     ① `None` + `NOT LIKE` ⇒ PG 侧比对恒不等（`normalize_sql_expr` 只剥 `::type`、**不折叠算子** `!~~`↔`NOT LIKE`）
     ⇒ 复现 `model.rs:484-498` 记录的 `ASC` 事故**同一形态**（plan 拆成同名两条 ⇒ `additive_only` 只建不删 ⇒ 撞「已存在」fail-stop）；
     ② `None` + `!~~` ⇒ SQLite 侧渲染出非法 SQL（`render.rs` 只 `strip_casts`）；
     ③ 两条声明分方言 ⇒ 同一对象被两方言各声明一次，需先确认 `all_indexes()` 的去重/同名判据容得下。
     ⇒ **要双语表达须给比对侧加算子折叠**，属独立改造项。
   - ⚠ **边界登记**：`save_pattern` 的冲突分支 `update_columns`（`src-tauri/crates/trajectory/src/storage.rs:518-527`）**会写 `name`** ⇒
     若将来出现「把检查点的 `name` 改写成学习器模式名」的路径，本排除式约束**不会**拦（它只约束非 `rl_checkpoint:%`）。
     本轮实测 `rl_checkpoint:` 行数为 **0**，无可达路径；登记为待观察项。
6. ⚠ 106 vs 109 的 **3 条差未消**（⑤）。
7. 🔴 **删 `tests/pg_migrations.rs` 时未备份 ⇒ 源文件不可恢复**。
   **这是本轮我自己的流程失误**，必须记清楚：备份做了两件（`migrations/` 整目录 77 文件
   sha256 一致 + `mod.rs`/`extras.rs`/`expected.rs`/`lib.rs` 四个文件的改动前副本），
   **却漏了「被删掉的那个测试文件」** —— 因为备份清单是照「主目录」列的，而它不在主目录里。
   （同一次手术里其余被删的都是 `migrations/` 内文件 ⇒ 只有这一个踩到。）
   该文件是 `AXAGENT_TEST_PG_URL` 门控、本轮**无法运行** ⇒ 我选择退休而非改判（不写无法验证的测试），
   但**「退休」不等于「可以不备份」**——这两件事我混了。
   - **可恢复粒度**（从 `output/fix5-verify-report.md`、`output/p2-verify-all.txt`、`output/ngram-test.log`
     的残留记录反查）：它只有 **1 个测试** `pg_migrations_apply_and_search_works`，做的事是
     「全新 PG 库 → 跑完全量迁移 → `missing_versions` 为空 + 6 条 v010–v013 类型断言 +
     `session_events` UNIQUE + FTS 端到端」。
     ⚠ **它被孤儿化过两次**：那 6 条类型断言原本属于 `pg_runtime_test.rs`，上一轮（`fix5`）
     以「并入 `pg_migrations.rs` 后删除源文件」的方式处置 ⇒ `pg_migrations.rs` 是它们**唯一**的
     承接处；本轮把它删掉，等于把上一轮的收敛结果**又打散一次**。这正是「收敛到唯一承接处」
     这个动作的固有风险：承接处一删，被收敛的东西跟着消失（与 `#412` 同族）。
   - **不可恢复**：断言的具体文本（唯一残留是 §五·一③ :508-512 里被引用的那 3 行）。
   - ⇒ 教训已写进 `schema-source-switch-playbook` skill：**备份范围必须按「将被删的全部文件」列，
     而不是「主目录」**，并在删除前用「清单 ↔ 备份清单」做一次差集核对。
8. ⚠ `pg_cjk_fts.rs` 改判后**失去**「重建生成列」这一步（引擎不重写存量生成列表达式，
   `AlterColumnType` 不在纯新增白名单里）—— 已在该文件注释里登记。
9. ✅ **CI 断链（已修）** —— 独立验证者 `Verify-1` 发现：`.github/workflows/ci.yml:389`
   仍跑 `cargo test -p axagent-dao --test pg_migrations`，而该 test target 已被本轮删除；
   cargo 对不存在的 target 返回非 0，叠上该 `run` 块开头的 `set -euo pipefail`（`:388`）
   ⇒ **CI heavy job 必然失败**。已移除该行，并同步 `:305`（历史依据段）与 `:382`（顺序依赖段）
   两处注释 —— 它们讲的排序理由（「它是唯一会重建库的测试」）随文件删除而失效。
   **由此产生的覆盖缺口（已登记，未修）**：删掉 `pg_migrations` 后，真 PG 上**没有任何断言**
   覆盖「声明式引擎建出的**全量** schema（表集合 / 列类型 / 约束）」。
   注意缺口**不是**「引擎在真 PG 上没被跑过」—— `pg_cjk_fts.rs:111` 就会调
   `reconcile::apply::bootstrap_schema` 真跑一轮收敛，只是它断言面只覆盖 CJK 子集。
   补法：新写一个 PG 侧引擎建库测试（**尚未做**）。
   ⚠ **系统性教训**：「删 N 个文件」的手术必须同步扫**引用这些文件路径的非源码位置** ——
   CI workflow、脚本注释、skill 文档都不在编译器的射程内（编译器只抓 Rust 引用）。
10. ⚠ **存量库的唯一性约束未补**（详见 ③）：6 条列在 prod 上只剩**非唯一**索引
   （`output/tmp-p2-dup-check.log` 实证；`opc_demand_subscriptions.keyword` 连普通索引都没有）。
   实体属性只能救**全新库** —— 引擎 `bootstrap_schema` 只做纯新增，**不会**给已有表补 `ADD UNIQUE`。
   本机 `axinvest` 库这 6 张表 **`rows=0`** ⇒ 在此库上补约束**零风险**；
   但「是否补」属**产品/运维决策**（其它部署的库可能非空，`ADD UNIQUE` 失败会 fail-closed
   卡住 reconcile）⇒ 本轮**只登记不动手**。
11. ✅ **FK「引擎比迁移更严」的冷启动风险已实测：本机存量库 0 孤儿**
   （`output/tmp-p4-orphan-probe.log`，只读探针；`Verify-1` §7 提出清单时**明确未查孤儿行**）。
   §7 的清单是 13 列（43 条 `belongs_to` 中 30 条迁移侧有 `REFERENCES`，这 13 条没有）；
   若引擎对这些列发 `ADD CONSTRAINT`，存量库上的孤儿行会让 ALTER 失败、fail-closed 卡住 reconcile。
   现已补测：**43 条 `belongs_to` 全部 0 孤儿**。
   ⚠ 但「43 条全 0」是**过于整齐**的结果，故做了**区分力自证** —— 按「该列是否有非 NULL 数据」二分，
   只有 **12 条是非平凡 CLEAN**（真证据），其余 31 条是**子表为空导致的平凡成立**（不能当证据）。
   非平凡的 12 条**含 §7 清单里的 5 条**，且都是大表：
   `knowledge_relations.knowledge_base_id`（**112937** 行）/ `knowledge_entities.knowledge_base_id`（72816）/
   `note_links.source_note_id` · `note_links.target_note_id` · `note_backlinks.target_note_id`（各 74791）/
   `knowledge_documents.knowledge_base_id`（48542）/ `agent_profiles.expert_id`（576）/
   `models.provider_id`（52）/ `provider_keys.provider_id`（7）/ `messages.conversation_id`（2）/
   `memory_items.namespace_id`（2）/ `agent_sessions.conversation_id`（1）。
   ⇒ **在本机 `axinvest` 库上，引擎加 FK 不会 fail-closed**。
   ⚠ 残留未知：其它部署的库未测（故此项**不构成**「可以放心批量加 FK」的通行证）。
   ⚠ **探针自身两次踩坑，都值得记**：
   ① **窗口式正则系统性漏项** —— v1 用「`belongs_to` 之后 220 字符内找 `}`」，而
      `knowledge_entities.rs` / `note_links.rs` / `wiki_pages.rs` 里这些声明后面紧跟
      `has_many` 或第二个 `belongs_to`，到最近的 `}` 远超 220 ⇒ **静默漏 5 条**，
      报出「38 条全 CLEAN」的假象。教训：**窗口式正则在「同类声明长度不一」时系统性偏向短的那种**。
      改按完整 `#[sea_orm(…)]` 属性块切分后得 **43 条**，与 `Verify-1` 独立数出的 43 吻合
      （并加 5 条自证断言锁定，防再次退化）。
   ② **含 `${…}` 的 JS 不能经 shell `-e` 传递** —— bash 会把 JS 模板字符串当参数扩展**执行**，
      把脚本写坏（与纪律「值走 `$(...)` / argv 即绕过全部代码防线」同族）。临时脚本一律用文件写入。
12. ✅ **今早这条改动线的 21 处 `cargo fmt` 违规已清**（`cargo fmt --check -p axagent-dao -p axagent-entities`
   现为 `EXIT=0` / **0 diff**；此前 21 处 diff 横跨 11 个文件）。
   ⚠ 处置顺序是「**先归因、再动手**」，不是看到 diff 就 `cargo fmt`：
   - 先落盘完整 `--check` 输出并抽出 `Diff in` 清单 → 21 处 / 11 文件。
   - 逐文件核 mtime（`output/tmp-e2e-fmt-mtime.mjs`）：本轮我改的 3 个
     （`examples/p5_engine_takeover.rs` / `migrations/mod.rs` / `reconcile/expected.rs`）
     与今早这条线的 7 个（`cjk_ngram.rs` / `reconcile/{apply,model,plan}.rs` / `seed.rs` /
     `tests/{pg_cjk_fts,schema_compliance}.rs`）**全部早于本轮、且无 5 分钟内写入**
     （`output/tmp-e2e-fmt-guard.mjs`）⇒ 才判定为「本线自己漏跑的 fmt」，而非并发会话中间态。
   - 再用 `rustfmt --edition 2024 --config-path src-tauri/rustfmt.toml --emit files <单文件>`
     **逐文件**格式化，**刻意不用 `cargo fmt`** —— 后者会波及同 crate 内其它文件的并发中间态。
   - ⚠ 排查中曾出现一个**假警报**：`expected.rs` 的 mtime 比「我开始改动的时间」晚 10 分钟，
     一度怀疑有外部会话在写它。核实 diff 内容（`opc_blog_posts` CASE / `find` 闭包）后确认
     就是我自己的编辑落盘 —— **mtime 只能证明「某时刻被写过」，不能证明「谁写的」**；
     归属必须靠内容核对，与 `#242`（外键先查 DB）同族：**先查事实，再下结论**。
   - 教训：**`cargo fmt --check` 报出的文件不一定都是本轮引入的**；反过来，本项目里
     「中文按双宽计 + `use_small_heuristics = "Max"`」会让**看起来没超 100 的行**实际超限，
     所以「我写的时候觉得没超」不构成不跑 fmt 的理由。
13. ✅ **闭合「实体标志 → DDL」端到端缺口**（③ 的新增判据；本轮唯一新增测试代码）。
    **发现**：`reconcile/apply.rs::bootstrap_schema_builds_every_table_on_sqlite` **只断言「表存在」，
    对约束一字未提**；而 ② 修的 5 条唯一性走的是「实体 `#[sea_orm(unique)]` → 模型列标志 →
    `render` 内联 `UNIQUE (col)` → 真建进 SQLite → `introspect` 读回列标志」整条链 ——
    **两端各自都有断言，却没有一条把它整条走完**（模型侧由
    `expected.rs::migration_unique_semantics_survive_in_column_flag` 守，渲染侧由
    `render.rs:2164-2190` 的**合成 payload** 守）：中间若断了（标志有了但 DDL 没出，或 DDL 出了但
    introspect 没读回）两端照样绿 ⇒ **无法区分「真修复」与「只改模型标志的假修复」**。
    **补法**：在同一测试内加表驱动 5 条（`sync_devices.unique_id` / `sync_permissions.device_id` /
    `opc_landing_pages.slug` / `opc_blog_posts.slug` / `trajectory_preferences.key`，逐条带迁移出处）
    + **3 条区分力反例**（同表非唯一列必须为 `false`：`opc_landing_pages.title` /
    `opc_blog_posts.title` / `trajectory_preferences.value`）—— 后者专门防「introspect 退化成
    给每列都标 `unique`」时前者集体假绿。
    **实测**：`SQLite bootstrap：期望 183 张，实况 183 张；5 条单列唯一性端到端已核对`，`V10_EXIT=0`
    （同批 `--test schema_compliance` 的 `verify_all_entity_columns_exist_in_db` 亦 ok）。
    ⚠ **形态提示**（已写进断言注释）：`render.rs:1100` 生成的是**表级** `UNIQUE ("col")`
    （不是列级 `col TEXT UNIQUE`）；SQLite 对两者都产生 `origin='u'` 的单列自动索引 ⇒
    `introspect/sqlite.rs:286-292` 分流成列标志。若哪天渲染改用唯一索引，该分流会把它落成
    `IndexModel` 而非列标志、本条即红 —— 那正是要拦的回归。
14. ⚠ **clippy 门禁：12 处 error（本轮修 4 处我引入的 + 8 处本线遗留）**。
    `cargo clippy -p axagent-dao -p axagent-entities --all-targets -- -D warnings` 实测
    `V11_EXIT=101`。归因分两批（**先归因再动手**，不因为是一份日志就整批照改）：
    - **本轮我引入的 4 处**（全在 `reconcile/expected.rs`）：`explicit_auto_deref` ——
      表驱动 CASE 解构出的 `table` / `col` / `counter` 是 `&&str`，我手写了 `*`；
      auto-deref 本来就会处理 ⇒ 去掉 `*`。
    - **本线遗留 8 处**：`introspect/parse.rs`（`type_complexity`）、
      `introspect/sqlite.rs` ×2（`type_complexity`）、`reconcile/model.rs` ×2
      （`unnecessary_sort_by`）、`reconcile/plan.rs`（`redundant_closure`）、
      `reconcile/extras.rs`（`single_element_loop`）、`reconcile/safety.rs`
      （`cloned_ref_to_slice_refs`）。**归属判据**：这些文件本轮只被格式化、逻辑未动，
      且注释直接指向本线阶段（`extras.rs` 那条引「PLAN §五·一 B 组」、`model.rs` 那条在
      `finalize()` 里）⇒ 是 **P2–P4 阶段的产物**，那段时间**没跑过 clippy 全量**，
      所以从未暴露（`cargo check` / `cargo test` 都不执行这类 lint）。
    - **修法全部是机械等价改写**（零产品语义决策）：去 `*`；`sort_by(|a,b| a.key().cmp(&b.key()))`
      → `sort_by_key(|a| a.key())`（两者都**稳定**排序，且 `key()` 无副作用 ⇒ 等价）；
      闭包 → 函数项（并注明 `norm_cols` 那处**不能**同样改，因为 `c` 是 `&String`，
      需要闭包提供 deref coercion）；单元素循环 → 直陈；`&[row.clone()]` →
      `std::slice::from_ref(&row)`（`row` 到下一行才 move，借用不冲突）；
      两处抽 `type` 别名（`parse::CreateTableBody` 为 `pub` —— 类型等价、调用方无需改；
      `sqlite.rs` 的 `FkAcc` 定义在**函数内**，因为它只是局部聚合，不是对外抽象）。
    - ⚠ **过程教训（重要；我第一版判断写错了，此处更正留痕）**：
      起初看到 V11 一次报出 12 个 error，我写下「这 12 个是一次全报的，不是 AGENTS.md 警告的短路」。
      **这个判断是错的** —— 修完那 12 处后重跑，**又冒出 3 个全新的**
      （`examples/p5_engine_takeover.rs:64/76/90`，`needless_borrows_for_generic_args`：
      `exit2(&format!(…))` → `exit2(format!(…))`）。
      真实机制：`--all-targets` 下 lib / lib test / example / integration test 是**并行 job**，
      一旦有 job 失败，cargo 打 `warning: build failed, waiting for other jobs to finish...`
      并**打断其余 job** ⇒ **每一轮只暴露「先失败的那批 target」**，其余 target **从未被 lint**。
      （V11 只看到 lib 6 + lib test 6；V12 只看到 example 3 —— 两批互斥即是证据。）
      ⇒ **正确做法**：修完后**反复跑直到全绿**，或按 target 分批、一轮看全：
      `clippy --lib` / `--examples` / `--tests` / `--benches` 各自独立跑、各自记退出码。
      **绝不能**由「本轮只有 N 个 error」推定其余 target 干净 —— 这正是 AGENTS.md 那条
      「遇首个 error 即短路」在 `--all-targets` 下的**真实形态**（我差点又栽在这里）。
    - ⚠ **第四个坑：分批跑的第一版结果是假绿**。分批 `--lib` / `--tests` / `--benches` 三批
      都是 `EXIT=0`，却只耗时 0.85–1.41s 且日志里**没有 `Checking` 行** ⇒ cargo 判定产物新鲜、
      **直接跳过了 clippy-driver**：「0 error」实为「**没有 lint**」（同族于「0 命中 ≠ 没问题」）。
      用 `touch crates/dao/src/lib.rs`（**只改 mtime、不动内容**）强制重编后重跑，三批**都出现
      `Checking`** 且零 error：

      | 批 | 退出码 | 耗时 |
      |---|---|---|
      | `--lib` | **0** | 25.99s |
      | `--tests` | **0** | 10m12s |
      | `--examples` | **0** | 2.99s |

      （`--benches` 无 target。`dao` 实际有 **5 个 examples** + **5 个 integration tests** + 0 benches
      —— 而 V12 只报了 `p5_engine_takeover` 一个 example，**另 4 个当时正被短路**。）
      ⇒ `axagent-dao` + `axagent-entities` 的 clippy 门禁**已实证全绿**（15 处全清）。
      ⚠ **仍欠 `--workspace --all-targets` 全量**（AGENTS.md 的完整口径）：本轮只定点这两个 crate，
      因为单批 `--tests` 就耗时 10m12s，全量在并发持锁下耗时不可控 —— **列为遗留**。
15. ✅ **`sql_type_of` 与 `model::sqlite_declared_type` 口径不一致 ⇒ 152/183 张表的类型对照长期失效**
    （2026-09-16 定因并已修；P6 收尾期由 `impl-dao` 侧发现并闭环）。
    **现象**：设置页「修复 Schema」报 `tables_scanned=31`，而本方言期望表数 183。
    **四个要点**：
    - **口径不一致（根因）**：同一张 sea-query SQLite 类型表在本仓有**两份手抄**。
      `reconcile/model.rs:833-837` 那份是对的，且注释写死了 vendored 行号
      （`BigInteger → integer`）；`migrations/schema_diff.rs::sql_type_of` 那份是错的
      （`BigInteger → BIGINT`）。引擎按 sea-query 建表（建出 `integer`），而 healer 用
      `sql_type_of` 对照 ⇒ **同一份实体声明在两个渲染器下有了两个名字**。
    - **152 张失效**：`heal_target_type("BIGINT", "integer")` 判为「需加宽」⇒ 发
      `ALTER TABLE … ALTER COLUMN … TYPE BIGINT`，而 **SQLite 没有 `ALTER COLUMN`**
      ⇒ 语句在第二个 `ALTER` 处解析失败。`heal_entity` 的列循环带 `?` ⇒ 该表**后续全部列**
      的对照在第一列就中断。实测 183 张实体表里 **152 张**中招（含 `conversations` /
      `gateway_keys` / `background_tasks`），命中表只剩 **31** 张。
    - **fail-open 吞掉**：`heal_all` 的 per-entity `Err` 只 `warn!`、不进返回值，而中断的表
      的 `columns_added` **恰好也是空的** ⇒ `columns_added=[]` 被上上下下读成「其余表没有
      缺列」。**这一条才是让前两条活了这么久的机制**：错误进了日志，结论没进数据。
    - **证据日志**：`output/tmp-schema-ui-judge3.log:22`（定因探针，修前
      `命中表=31 ／ 表缺失=0 ／ 报错=152`，错误串 `near "ALTER": syntax error`）、
      `output/tmp-schema-ui-judge2.log:7` / `judge3.log:37`（修前 `tables_scanned=31
      columns_added=[]`）、      `output/tmp-schema-ui-test2.log`（修后 `[repair_schema]
      tables_scanned=183 … errors=[]` + 探针 `命中表=183 ／ 报错=0`，`298 passed / 0 failed`）、
      `output/tmp-schema-ui-clippy4.log` · `tmp-schema-ui-fmt4.log` · `tmp-schema-ui-check-app3.log`。
      **最终一轮（含名字表对齐 + 断言升级，`impl-dao`）**：`output/tmp-schema-ui-test4.log`
      —— `299 passed / 0 failed`、`TEST_EXIT=0`、`[repair_schema] tables_scanned=183
      columns_added=[] types_healed=[] errors=[]`、探针 `命中表=183 ／ 报错=0`、
      `[probe/bootstrap 一致性] dialect=sqlite tables_expected=183 tables_actual=184
      pending_apply=0`；`output/tmp-schema-ui-clippy6.log`（`CLIPPY_EXIT=0`，含
      `Checking axagent-dao`）· `tmp-schema-ui-fmt6.log`（`FMT_ALL_EXIT=0`）。
      ⚠ **判据不是「探针打印了 0」**（打印无判别力），而是新增的三条可红判据：
      `schema_diff::tests::sqlite_type_names_mirror_sea_query_verbatim`（断言**字面量**，
      `BigInteger => "integer"` 与 `Integer|Unsigned|BigUnsigned => "integer"` 修前必红）、
      `schema_diff::tests::sqlite_never_emits_alter_column`（夹具为前提自证的 `smallint` 窄列，
      去闸必红）、`migrations::tests::repair_schema_on_converged_db_reports_no_heal`
      （`tables_scanned == expected_table_count(Sqlite)`**实测两侧同为 183** + `errors` 为空）。
      ⚠ 断言**不写死 183**：期望值是派生函数 `expected::expected_table_count` 的返回值，
      手抄数会随实体集腐烂（见 `impl-dao` 对旧 `assert!(tables_scanned > 0)` 的自我否定：
      `31 > 0` 为真 ⇒ 那条断言拦不住本次缺陷）。
    **修法三件**（分开做，因为三者判据不同）：
    1. **名字表对齐**（根因）：`sql_type_of` 的 SQLite 整数族/浮点族逐字对齐 sea-query
       （`tinyint` / `smallint` / `integer` / `float` / `double`）。
       ⚠ 刻意**不**整体委托给 `model::sqlite_declared_type`：那份镜像对字符串/时间/JSON/UUID
       族**自述**「不直接调 sea-query 渲染…只需**亲和性类别**一致」，给出的是 `varchar` /
       `datetime_text` / `jsonb_text` / `vector_blob` / `real_decimal` 这类**编造名**，
       兜底臂还是 `format!("{other:?}")`（未验证的调试串）。而 `sql_type_of` 的返回值是
       **真 DDL 文本**（`ALTER TABLE … ADD COLUMN {name} {sql_type}`）⇒ 把编造名或调试串
       写进库不是「对齐」，而是引入新的未验证映射。故只对齐有 vendored 行号明证的两族。
    2. **方言闸**（能力）：闸放在 `heal_entity` 的 **DDL 发出点**（`if !is_pg { continue; }`），
       不放进 `heal_target_type` 内部 —— 判据是「本方言有没有这条 DDL」这一**能力**问题，
       与「哪些类型组合算错配」那套规则无关（将来往规则里加条目也不会漏掉这个闸）。
       SQLite 既做不了（无 `ALTER COLUMN`）也不需要（动态类型下，列上声明的类型名不影响
       sea-orm 解码）。**闸与名字表是两件事**：名字表错了会误判，误判之后发不发 DDL 由闸决定。
    3. **`errors` 上抛**（可观测性）：`schema_diff::DiffReport.errors` +
       `migrations::SchemaRepairReport.errors` + `init/database.rs` 启动自愈的告警分支。
       失败分两层：**整体失败 ⇒ `Err`**（连列快照都读不到）；**部分未完成 ⇒ `Ok` + `errors`
       非空**（其余表确实修了，升级成 `Err` 会把「修好 180 张、2 张没查完」显示成「修复失败」；
       压回 `warn!` 就是 fail-open）。
       ⚠ 这一条同时解掉一处**前端硬崩**：`DatabaseSettings.tsx:93/95` 已在 #10 消费
       `report.errors`（`report.errors.length`），而后端当时没有这个字段 ⇒ 点按钮必抛
       `Cannot read properties of undefined (reading 'length')`。
    **PG 已知偏差（本轮不动，量纲 0）**：`sea-query-1.0.2/src/backend/postgres/table.rs:32-35` 里 sea-query 对
    `TinyInteger|TinyUnsigned|SmallInteger` 渲染 `smallint`、对 `Unsigned` 渲染 `bigint`，
    而 `sql_type_of` 一律给 `INTEGER`。**已由 lead 判定：本轮不动，量纲 0，依据
    `output/tmp-lead-type-contract-table.log`** —— 本仓实体与 L2 声明**均不产生**这些
    `ColumnType`（实测 0 处）⇒ 偏差**不可达**；且 `Unsigned → INTEGER` 是**收窄**方向
    （`2 > 3 = false`）不触发 `heal_target_type`。该偏差已写进 `sql_type_of` 文档注释（含出处）。
    **核查既有条目（有无被推翻或重复）**：第 **12** 条（fmt）—— 本轮 `cargo fmt --all` =
    `EXIT=0`，与之一致，未被推翻；第 **13** 条（实体标志 → DDL 端到端）—— 不受影响，本缺陷在
    `migrations/` 侧、不在 `apply.rs::bootstrap_schema` 的断言链上；第 **14** 条（clippy）——
    本轮 `cargo clippy -p axagent-dao --all-targets -- -D warnings` = `EXIT=0`（日志含
    `Checking axagent-dao`），**无新增 error**，其「仍欠 `--workspace --all-targets` 全量」的
    遗留**仍然成立**。**三条均未被推翻、与本条无重复**（本缺陷此前不在 ⑨ 任何条目里）。
    ⚠ **方法论教训**：缺陷形态是「**同一张表手抄两份，而负责跑的那份是错的**」。两份都能编译、
    都有各自的测试、都对各自的局部口径自洽 —— 只有把两条通道的输出**放在一起比**（引擎建出的
    `integer` vs 本函数期望的 `BIGINT`）才会显形。这与 §五·三·补「双口径对账」同族：
    **同一语义存在两份声明时，必须有一条判据把它们对起来**，否则两份会各自演化。
    - ⚠ **我自己的回归测试第一版栽在同一处**（值得记）：`sqlite_never_emits_alter_column`
      的第一版**按 `sql_type_of` 的输出选夹具列**（`… == "BIGINT"`）⇒ 名字表一改对，这个夹具
      就**选不到列**，报出 `notes 实体应有 i64 列` 这种**指错方向**的 panic
      （`output/tmp-schema-ui-test3.log`：`TEST_EXIT=101` / `1 failed`；修后
      `tmp-schema-ui-test4.log`：`299 passed / 0 failed`）。
      教训：**夹具不得用「被测对象的输出」来挑选** —— 判据要建立在被测对象**之外**的事实上
      （现改为按 `ColumnType::BigInteger` 选列）。与「自证式断言」同族：自证的是**恒真**，
      这一种是**夹具随被测对象漂移**。
    - ⚠ 该测试的夹具**不能用 `integer`**：与 `sql_type_of` 对齐之后 `integer` 不再触发加宽规则
      ⇒ 会退化成真空真（绿，但零信息）。改用旧库上真实存在的**窄列** `smallint`，并**自证前提**：
      `assert_eq!(heal_target_type(sql_type_of(&C::BigInteger, Sqlite), "smallint"), Some("INTEGER"))`
      —— 没有这一条，本测试分不清「闸生效」与「规则压根没触发」。

16. ✅ **`NON_MAIN_DB` 的方言作用域已改（2026-09-17，用户裁决「需要裁决的内容按建议实施」）** ——
    `extras.rs:584` / `:590` 的 `dialect` 由 `Some(Dialect::Postgres)` 改为 `None`（**双方言都不认**）。
    **为什么这是缺陷而不是口味**：原声明在 `:524-527` 的注释里**自己写明了前提**「主库是 PG」，
    而用户硬约束是「主库方言由用户在设置中设定（sqlite / postgresql）」⇒ 该前提不成立。
    逻辑跳步的精确位置是测试断言文案（原 `extras.rs:1489`）：「这 8 张表的**家**是 SQLite 侧车库」
    （成立）⇒「**连接的方言是 SQLite** 时不得排除」（不成立）—— 主库选 SQLite 时，主库是**第三个**文件。
    **实测后果**（由两条既有断言相乘反推，非推测）：`db.rs:141-143` 建表来源 = `expected::build(dialect)`；
    `schema_diff.rs:88` 定义 `tables_scanned`「**缺表不计入**」；`migrations/mod.rs:842-849` 断言
    全新测试库 `tables_scanned == expected_table_count(Sqlite)`（旧值 183）⇒ 该 SQLite 库里
    **物理存在这 183 张表，含 8 张侧车库表** ⇒ 与本节 ④（`:2144`）在 PG 上的事故**同形**。
    **反向论据不成立**：`:526-527` 担心的「指向 `index.db` 的那次 reconcile 会把侧车库判成孤儿」
    —— ① `None` 与 `Some(..)` 一样是**两个方向都退出**（`plan.rs:570` + `:651-656`），不会 DROP；
    ② 全仓**没有一处**把 `index.db` / `l2_cache.db` 的连接交给 `bootstrap_schema` / `plan::diff`；
    ③ 那 8 张表的 schema 由 owner crate **自建**（`search/src/ast_index.rs` / `search/src/file_index.rs` /
    `disk-cache/src/lib.rs` 的 `ensure_table(s)`，均 `create_table_from_entity` + `if_not_exists`）。
    **改动面 6 处 / 3 文件**：`extras.rs`（2 处 `dialect` + 注释块重写 + 字段文档 + 形态约束测试改名与断言翻面）、
    `expected.rs`（文档 + 判定式由 `Sqlite − Postgres == 8` 改为**两侧相等**；第二测试改名 + 断言翻面 +
    **新增「声明必须认领到真实实体」检查** —— 原检查靠 `lite.table(t).is_some()`，翻面后对「抄错表名」
    会**恒真通过**，故改为直查 `entity_modules!` 的未过滤注册表）、`plan.rs`
    （`non_main_db_table_is_neither_created_nor_dropped` 由只跑 PG 改为**两个方言都跑**）。
    ⚠ **读数随之变化（文档需同步，不是缺陷）**：期望表数 **PG 175 / SQLite 183 → 两侧同为 175**；
    `schema_diff.rs:791` 的「应打印 `命中表=183 ／ 表缺失=0 ／ 报错=0`」→ **`175 ／ 8 ／ 0`**
    （该测试池里那 8 张不再存在），该处注释已同步。本文件 `:2256` / `:2505-2506` / `:1102` / `:1531`
    等处「183 → 175」应读作**历史读数**，现值以本条为准。
    判据层新增 **#468**（D 组）· 证据层 **DETAIL §88**。**验收**：`cargo fmt --all -- --check` 已绿
    （0 字节输出）；`cargo test -p axagent-dao --lib` 在锁释放后自动执行（见 §⑨ 20）。

17. 📋 **`init/services.rs:271-275` 错 id 空间 —— 已单独立项（2026-09-17）** →
    `docs/plans/PLAN-memory-kb-reflow-id-space.md`。要点：语义与可观测**必须一起做**；
    可对齐的既有「哨兵行」协议（`harness::constants::sentinel` + `dao/src/seed.rs::ensure_sentinels`）；
    ⚠ **复活死路径的前置检查** —— 该路径在 PG 上**从未真正产出过数据** ⇒ `upsert_entity` 的幂等性
    从未被生产验证，直接接线有可能把「恒空转」换成「数据无界增长」。
18. 🔬 **D 项前置检查已查实（2026-09-17）—— 结论：❌ 不通过；且发现漏登的孪生点。**
    ① **`upsert_entity` 不幂等**：`crates/dao/src/repo/knowledge_graph.rs:621-622` 先
    `let id = format!("ent_{}", gen_id())`，`:653` 的 `on_conflict(Column::Id)` **就打在这个刚生成的
    随机 id 上** ⇒ 永不冲突 ⇒ **语义等价于纯 INSERT**。与本节 16 提到的另一处同型缺陷
    `upsert_relation`（`:709-710` + `:730`，同为 `gen_id()` + `on_conflict(Id)`）**逐字同型**。
    ② **全仓无 `(kb_id, name)` 唯一约束**（四层查证：实体层只有 `#[sea_orm(indexed)]` 无 `unique` ·
    L2 声明 `extras.rs` grep 无匹配 · 迁移 grep 无匹配（建表已切声明式）· 实测 `pragma_index_list`
    只有 PK 自动索引）⇒ 重复插入**既不报错也不合并**。
    ③ ⇒ 接线后 `knowledge_consolidation`（周期任务，每轮 ≤100 条）**每轮 +100 行** ⇒
    **「恒空转」直接变「数据无界增长」**。**接线前必须先给 repo 层去重口径**。
    ④ ⚠ **立项时漏登的孪生点**：`src-tauri/src/commands/knowledge_graph.rs:475-532` 的 Tauri 命令
    `sync_memory_to_knowledge_graph`（UI 可点）**逐字复制了同一段逻辑**（`:496-500` 复用的
    `"memory_default"` / `namespace_id` + `:501` name=前100字 + `:518` 同样只 `debug!`）。
    ⇒ 缺陷面是**两处**；且这一处是**用户可直接触发**的（用户点一次会看到 `entities_created: 0,
    failures: N` 却看不到原因）。施工须**同时**改两处并**收敛到同一函数**（判据 **#256**）。
    详见 `PLAN-memory-kb-reflow-id-space.md` §1.1b 与 §4.2（已就地更新，7141 → 10300 B）。
    **待裁决**：§4.1 的映射语义（A 单一哨兵 KB / B 每 namespace 一 KB / C 映射既有 KB）——
    建议 **A**。**未获此项裁决前不开工。**

19. 📋 **本节 9 的「补 PG 侧引擎建库测试」方向已改（2026-09-17 裁决）：不写 env-gated 测试。**
    理由：本节 7 已就同一条权衡做过一次选择（`pg_migrations.rs` 因 `AXAGENT_TEST_PG_URL` 门控、
    CI 不设 ⇒ 长期 skip 而 runner 报通过，故**退休**）；再写一条同类门控测试，等于把刚退休的
    东西换个名字请回来。**替代形态**：真 PG 覆盖走 `output/` 下的只读探针（现有
    `tmp-p3-probe-prod2.log` / `tmp-p4-*.log` 一族）。若将来要在 CI 里覆盖真 PG ⇒ 前提是
    **CI 侧同时提供可用的 PG 服务**，否则仍是假绿。

20. ⏳ **本轮验证状态（截至 2026-09-17 14:0x，未完成，如实登记；含一次本会话操作事故）**：
    - ✅ `cargo fmt --all -- --check` = **exit 0 / 0 字节**（全 workspace，复跑 3 次）。
    - ❌❌ **`cargo test -p axagent-dao --lib` 与 `cargo clippy -p axagent --all-targets` 双双无效
      —— 不是「红了」，是「没有产物」**。日志实证：
      `tmp-lead-dao-test.log` 仅 **72 B** = `Blocking waiting for file lock on build directory`
      + **`dao_test_exit=127`**，**无任何 `test result:` 行**；
      `tmp-lead-clippy-axagent.log` **3311 B / 0 error / 0 warning**，但末尾停在
      `Checking axagent-quant`，**无 `Finished` 行**。
      ⇒ 二者均在 `TaskStop('D5jS6T')` 之后被终结，而该调用返回的是 **`Cleanup completed`**
      （清理范围显然超出单个任务）⇒ **判定为本会话操作事故，不是被测代码的问题**。
      ⚠ 纪律：**「没有产物」不得读作「通过」，也不得读作「失败」**。
    - 🔻 `cargo clippy --workspace --all-targets -- -D warnings` —— **已由本会话主动撤下**
      （13:47 发起 / 13:54 撤下，Runtime 6m33s）。**撤销理由**：F 项获授权时尚未掌握全局队列；
      实测该机同时有**非本会话**的 cargo 在跑，且用户的 `cargo run`（`tauri dev`，PID 4896，13:46 起）
      **正排在本条之后** ⇒ 本条一旦拿到锁会**持锁 1~2 小时**（已知实测值），把用户自己的 dev 饿死。
      **替代覆盖**：本轮改动**只碰 `crates/dao` 3 个文件**，`NON_MAIN_DB` 的消费点已 grep 穷举
      （只有 `expected.rs` / `plan.rs`，全在 dao 内）⇒ `clippy -p axagent-dao --all-targets` 覆盖面等价。
      ⚠ **workspace 级仍未跑**，这是**未完成项**。
    - 🔁 **重跑已发起（14:0x，task `TOuNpd`）**：一次拿锁串跑
      `clippy -p axagent-dao --all-targets -- -D warnings` → `test -p axagent-dao --lib`
      （日志 `tmp-lead-dao-v2-clippy.log` / `tmp-lead-dao-v2-test.log`，用 `;` 分隔以免前者失败短路后者；
      **不再调用 `TaskStop`**）。
    - ⚠⚠ **归属判据更正（本节先前写错过，此处更正）**：本节曾按 `CommandLine` **形态**判归属，
      结论「`clippy -p axagent`(PID 35280) 非本会话」——**错的**，它正是本会话的 `EIdfyF`
      （本会话也会用 `--manifest-path` 绝对路径形态，形态**不能**作判据）。
      **唯一致命正确判据 = 后台任务记录（`task_id` → `command`）**；形态仅可作辅助，且必须正反都验。
      另据 `output/tmp-lead-procs4.txt`（`TaskStop` 后**立即**拍）⇒ 用户的 `cargo run`（PID 4896）
      **仍活着** ⇒ **未误伤用户进程**；`procs5.txt` 中它已自行退出（时间点在两次快照之间，非本会话所致）。
      仍在跑的**非本会话**进程：`test -p axagent --lib`（PID 2592@13:39 → **9428@13:54:49**，
      带引号形态，**正在编 `axagent_dao`**）⇒ 该会话在反复重试。
    - ✅ **锁是活的，不是死锁**（按记忆里的**唯一正判据**，且已修正观察面）：间隔 60 秒测
      `target/debug/deps` + `target/debug/build/*/out` + `incremental` **取最大** ⇒
      最新 mtime 由 `05:43:31` → `05:53:21`（`axagent_lib-*.d`，增量 **590488 ms**，`deps` 文件数 +16）。
      ⚠ 只测 `build/*/out` 会**误判成卡死**（该处只放 build script 产物，rustc 阶段的产物在 `deps`）。
    - ✅ **同型缺陷穷举已完成（判据 #256）**：`extras.rs` 九个声明块逐块判定 ⇒ **`NON_MAIN_DB` 是本文件
      唯一一条「用方言代理库身份」的声明**。其余 `Some(..)` 全是**真方言差异**：
      `VIRTUAL_TABLES`/`TRIGGERS`（`CREATE VIRTUAL TABLE … fts5` = SQLite 专有）、
      `GENERATED_COLUMNS`/`PATTERN_TSV`（`tsvector`+`to_tsvector`+`ax_cjk_ngram` = PG 专有）、
      `CHECK_CONSTRAINTS`（`category = ANY (ARRAY[…])` / `period ~ '^…$'` = PG 专有）、
      `FUNCTIONS`（`ax_cjk_ngram`）、`GIN_INDEXES`（`method: gin`）；`ORPHAN_EXEMPT` 为 `None`
      （判定策略，与方言无关）。**无遗漏孪生点。**
    - ⚠ 本条 14 记录的「`axagent-dao` + `axagent-entities` clippy 曾实证全绿（15 处全清）」**不能顺延**
      —— 那是**改动前**的读数，本轮改了 dao 3 个文件 ⇒ 必须重跑才能引用。
21. 🔬 **A 项的影响面已实测（2026-09-17）—— 结论与本轮改动前的一个推测**相反**，如实登记**：

    **测法**（只读，不动用户库）：`node:sqlite` 的 `DatabaseSync('C:/Users/polit/.axinvest/axagent.db', {readOnly:true})`。
    选只读是因为 `axagent.db-shm` 的 mtime（`2026-09-17T03:41:45Z`）表明**用户的 `tauri dev` 正持有该库连接**。

    **实测读数**：
    - 主库 `C:\Users\polit\.axinvest\axagent.db` 共 **140** 张表；
    - 8 张侧车库表 **全部缺席（命中 = 0）**；
    - 库内**不存在任何** `l2_*` / `ast_*` / `file_index` 形态的表（按名模式扫描 = 空集）；
    - `.axinvest` 目录下**没有 `index.db` / `l2_cache.db`** ⇒ 侧车库文件从未被创建。
    - 该库 mtime = `2026-07-13`，`expected_table_count` 现值为 **175** ⇒ 该库落后 **35** 张表。

    **由此得到三条结论（全部改变了本项的定性）**：

    a. **PG 侧行为逐字不变 ⇒ 对当前生产库零风险。**
       `l2_applies(Some(Postgres), Postgres) == l2_applies(None, Postgres) == true`
       （`expected.rs:427-433`，`None => true`）⇒ 改动前后 PG 期望集都排除这 8 张、孤儿豁免都收进它们。
       用户在用的 `axinvest`(PG) 库**不会因本次改动产生任何 DDL 差异**。这一条是**纯代码等价性证明**，
       不依赖实测，但它解释了为什么 PG 那 8 张既有空壳（`extras.rs:519-520` 已登记）**不受影响**。

    b. ⚠ **撤销上一轮的一个担心**：我先前的推理是「改动前 SQLite 侧每轮都会 `CREATE` 这 8 张 ⇒
       改动后它们从『每轮重建』变成『永久留存』⇒ 需要一次性人工清理」。**实测证明该担心不成立**：
       生产 SQLite 主库里 0 张残留 ⇒ **不存在需要清理的对象**，本条**不产生**清理授权项。

    c. **缺陷本身仍然成立，但准确的定性是「堵一条尚未启用的路径」，不是「修一个正在发作的故障」。**
       缺陷的**实证**在 §⑨ 16 已给出且依然有效 —— `migrations/mod.rs:842-849` 断言
       全新 SQLite 库 `tables_scanned == expected_table_count(Sqlite)`（旧值 183），
       而 `schema_diff.rs:88` 定义 `tables_scanned`「缺表不计入」⇒ **测试里新建的 SQLite 库确实被建出
       183 张（含 8 张侧车库表）** ⇒ 逻辑路径是真的会走。
       但生产 SQLite 主库停在 140 张 / mtime 2026-07-13 ⇒ **该路径在本机从未真正启用**。
       ⇒ 本项的价值是：**用户一旦在设置里把主库切成 SQLite，改前会静默多建 8 张永久空表、改后不会**。
       这正是用户硬约束（「不能硬编码为 sqlite」= 方言必须可配）下的**必然要求** —— 可配的另一侧必须成立。

    **附带确认**：`index.db` / `l2_cache.db` 未被创建，与本节 16 的**反向论据②**（全仓没有一处把侧车库
    连接交给 `bootstrap_schema` / `plan::diff`）互相印证 —— 那两个文件在 SQLite 主库模式下也不存在。
22. ✅ **两处既有红已修（2026-09-17，用户授权「继续未作的内容」）+ 一次自我更正。**
    详细版 → `PLAN-memory-kb-reflow-id-space.md` §5d-补3。

    | 红 | 归属取证 | 修法 | 读数 |
    |---|---|---|---|
    | `cargo fmt --check`：`crates/dao/examples/memory_reflow_probe.rs` **10 处** | mtime `2026-09-17T08:08:41Z`，**早于**本轮 09:01–09:03 的 dao 改动 **53 分钟** | `cargo fmt -p axagent-dao`（**限包**：只格式化该包文件） | 只有该文件 mtime 变 `09:34:04`；`fmt --check` **EXIT=0**（原为 1） |
    | `test`：harness `knowledge_graph::type_vocabulary_tests::test_csv_import_relations_are_declared` | 见下更正 | `harness/src/knowledge_graph.rs:577` 的 `DATA_DRIVEN_COLUMN.evidence` 行号 **`1748 → 1751`** | harness lib **705/1 → 706/0** |

    **⚠ 自我更正 —— 我此前对这条红的归属说错了，且两处都错。**
    我先前在会话里写过「同样的红在 09-15 就已存在（`output/verify-2026-09-15c/test-harness.log:697`，当时真值 1749）」：
    ① 该**路径不存在**（真实文件是 `output/test-harness-2026-09-15c.log`，57144 B）；
    ② 该日志里这条测试是 **`ok`**（`:494`）且 `test result: ok. 684 passed; 0 failed`（`:899`）—— 与我的说法**相反**。

    **正确归属（可复现）**：09-15 02:25 它是**绿**的 → `src/commands/knowledge.rs` mtime `09-15T08:15:47Z`
    把 `let rtype = fields[2]` 推到 **1751**（`:1748` 现为 `}`）而 `DATA_DRIVEN_COLUMN.evidence` 未同步
    ⇒ **跨文件行号引用腐烂由此产生** → `output/test-c2.log`（`2026-09-17T08:48:25Z`）**首现该红**，
    panic 落在 `knowledge_graph.rs:1139` ⇒ **早于**我 09:01–09:03 的编辑所致位移（`:1139 → :1184`，
    **+45 行 = 新增 `stable_relation_id` 块 `:854-898` 的 45 行**，两条新测试在其后不参与位移）。

    **收尾全量读数**（`cargo test -p axagent-harness -p axagent-dao -p axagent-trajectory --no-fail-fast`，
    `output/test-final.log`，4m22s）：`test result:` **14 条全 `ok`**、`FAILED` **0 条**、`error:` **0 条** ——
    dao lib **305/0** · dao 集成 6 个 **2/7/1/1/5/1** · harness lib **706/0** · `serialization_contracts` **42/0** ·
    trajectory lib **425/0** · `causal_integration` **22/0** · doctests **6/0**（9 ignored）⇒ **三包首次全绿**。

    判据层新增 **#480**（A 组：断言行级唯一性时 `count` 必须排在「读单行」之前 —— `.one()` 多行时静默取首行，
    会把「有 2 行」伪装成「字段没更新」）· **#481**（L 组：引用历史证据前必须 `stat` + 读正文）。

    **同日顺带清掉证据门禁硬红 4 处**（`check-single-source-facts.mjs --ci` **EXIT 1 → 0**；
    `check-decl-evidence.mjs` **EXIT 1 → 3**，即结构腐烂 **2 → 0**）。要点（明细 →
    `PLAN-memory-kb-reflow-id-space.md` §5d-补3 第 6 节）：
    ① **其中 1 处是我造成的** —— 在 `trajectory/src/causal.rs` 插 `CAUSAL_KB_ID` 块（`:36-49`）
    把该文件行号下推 ⇒ `harness` 里 `causal.rs:305` 那处引用指到空行（真值 `:322`）；
    判据靠**早于本批的门禁日志**：`decl-evidence-after-fix.log`（09-15 01:59）显示该引用当时**有效**、
    `tmp-d9-ssf.log`（09-16 03:44）硬错 **0**。
    ② 另 3 处非本批（`src/commands/knowledge_graph.rs` mtime 06:08 · `schema_diff.rs` mtime 05:46）。
    ③ `sqlite/table.rs:108/182` 的根因**不是路径错**：vendored `sea-query 1.0.2` 实存，
    是 `schema_diff.rs:586` 把路径**省略**成 `.../sqlite/table.rs` ⇒ 门禁的第三方识别（按**注释块**
    匹配 `name-x.y.z/`）失效 ⇒ 被当本仓路径；修法是写全版本化路径。
    ④ **上一轮我漏修了同族的另一半**：`DATA_DRIVEN_COLUMN` 的**文档行**仍写 `:1748`（只改了 `evidence` 字段）
    ⇒ 半修；连同 `has_concept :1777→:1780`、`in_industry :1792→:1795`、`employ_ :1808→:1811`
    一并清掉（同为 **+3** 位移）。本轮共改 **10 处行号引用**（harness 9 + `schema_diff.rs` 1）。
    ⑤ **教训**：「改完任何 `.rs` 就跑证据门禁」不是可选项 —— 本轮我第一遍**忘了跑**，
    回写文档时才补跑并立刻见红。
