// SPDX-License-Identifier: AGPL-3.0-only

//! L2 声明式例外清单 —— 「实体语法表达不了的 DDL」的唯一去处。
//!
//! ## 为什么存在这一层
//!
//! 引擎的真相源是 L1（SeaORM 实体）。但 SeaORM 无法表达以下 8 类对象，若不在
//! 这里声明，引擎的孤儿判定会把它们当作「实体未声明的遗留对象」删除：
//!
//! | 类别 | 实体为何表达不了 |
//! |---|---|
//! | FTS5 虚表 | `CREATE VIRTUAL TABLE ... USING fts5(...)` 是虚表 + 专用 tokenizer，非普通表 |
//! | FTS 同步触发器 | 实体只能声明表/列/索引，无触发器概念 |
//! | GIN 索引 | 实体只能声明 B-tree 单列索引，无访问方法（`USING GIN`）开关 |
//! | 多列索引 | `#[sea_orm(indexed)]` 只作用于**单个字段**，无复合键形式 |
//! | UNIQUE 多列 | 同上；实体 `unique` 只作用单列 |
//! | partial 索引 | 实体无 `WHERE` 条件表达 |
//! | CHECK 约束 | 实体无列级/表级约束表达（`sea-orm-macros` 无对应属性） |
//! | 生成列 | 实体无 `GENERATED ALWAYS AS (...)` 表达 |
//! | 自定义函数 | 实体层面不存在函数概念，但生成列依赖它（漏声明 = 生成列建不出来） |
//!
//! ## 口径：本清单 = **生产库实态**，不是「迁移源码里出现过的 DDL」
//!
//! 这是本文件最重要的约定，来源是一次实测冲突（`output/tmp-p2-diff.log`）：
//!
//! | 类别 | 迁移静态扫描 | 生产库实态 | 差异原因 |
//! |---|---|---|---|
//! | tsvector 列 / GIN | 6 / 6 | **9 / 9** | `v227` 用 `format!` 动态生成（`memory_items` + 每张 `vec_*_meta`），静态扫描扫不到 |
//! | 多列索引 | 35 | **36** | 静态扫描只解析 `CREATE INDEX`，漏掉**表级内联 `UNIQUE(...)`** 产生的 2 条；另 1 条已被 `v138` 显式 DROP |
//! | partial 索引 | 4 | **5** | `idx_retrieval_hits_feedback` 的 `ON {}(...)` 模板让静态解析失败 |
//! | CHECK 约束 | 8（DDL 条数） | **3（约束个数）** | 8 条 DDL 实为**同一对约束的历代替换**（v100 内联 → v200 替换 → mod.rs 替换） |
//! | 自定义函数 | 0 | **1** | 静态扫描的 8 个类别里根本没有「函数」这一类 |
//!
//! ⇒ **迁移源码会撒谎（漏、重、废），库里真实存在的结构才是引擎要对齐的目标。**
//! 本清单逐条取自生产库实态，每条在下方注释里标注来源与状态。
//!
//! ## 5 条「静态清单有、库里没有」的条目 —— **刻意不收录**
//!
//! 照搬静态清单会让引擎**重建已被淘汰的对象**，故逐条剔除（归因见行内注释）：
//!
//! | 条目 | 归因 |
//! |---|---|
//! | `trajectory_memories` 表 | `v101:396` 显式 DROP（数据已迁往 `memory_items`） |
//! | `trajectory_memories_fts` 虚表 | `v101:372` 随表一并 DROP |
//! | `trajectory_memories.tsv` 生成列 | 表不存在，无从建列 |
//! | `idx_traj_memories_tsv` | `v101:389` 显式 DROP |
//! | `idx_opc_demand_leads_dedupe` | `v138:51` 显式 DROP（被含 `content_fingerprint` 的新唯一索引取代） |
//!
//! ## 已知缺口（**刻意登记，不在此处修复**）
//!
//! `ax_cjk_ngram`（中文 n-gram 归一化）只被 `v227` 应用到 `notes` / `memory_items` /
//! `vec_*_meta`。`messages` 与 3 张 `trajectory_*` 的 `tsvector` 生成列**仍是旧版**
//! `to_tsvector('simple', ...)` ⇒ 这些表的中文检索仍然失效（连续 CJK 塌缩成单个
//! 词元，子串永不命中；见 `v227_cjk_fts` 模块文档的实测）。本文件按**实态**声明，
//! 不擅自扩大变更面 —— 扩到剩余 4 表是独立改动，见 PLAN。
//!
//! ## 维护纪律
//!
//! 1. **加表/加列后必须复核本文件** —— 漏一条 = 上线后结构缺损（引擎 DROP）。
//! 2. 本文件的条目数由 `#[cfg(test)] mod counts` 锁定；改条目必须同步改断言，
//!    避免「悄悄删了一条」。
//! 3. ✅ `FUNCTIONS[0].render` 的定义已从 `migrations::v227_cjk_fts` 提到
//!    `crate::cjk_ngram`（SQL 模板随迁至 `src/sql/ax_cjk_ngram.sql`）——
//!    本文件因此**不再依赖 `migrations/`**，删迁移不再撞编译错。提出来而不是
//!    就地复制，是因为函数定义（长期契约）与「重建生成列」这个一次性修复动作
//!    生命周期不同，详见 `cjk_ngram` 模块文档。

use super::model::{ColumnModel, IndexModel};

/// SQL 方言。`None` 在声明里表示「双方言通用」。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dialect {
    Sqlite,
    Postgres,
}

/// 索引声明（多列 / UNIQUE 多列 / partial / GIN 四类共用）。
///
/// `cols` 元素允许带排序方向后缀（如 `"created_at DESC"`）—— 生产库实测有 5 条
/// 多列索引带 `DESC`，若声明里丢掉方向，introspect 会判定「索引不一致」并反复重建。
pub struct IndexDecl {
    pub table: &'static str,
    pub name: &'static str,
    pub cols: &'static [&'static str],
    pub unique: bool,
    /// `Some("gin")` = 非默认访问方法；`None` = 默认 B-tree。
    pub method: Option<&'static str>,
    /// partial 索引的谓词（**含**外括号，与 `pg_get_indexdef` 形态一致）。
    pub where_clause: Option<&'static str>,
    /// `None` = 双方言通用。
    pub dialect: Option<Dialect>,
}

/// CHECK 约束声明。
pub struct CheckDecl {
    pub table: &'static str,
    pub name: &'static str,
    /// 约束表达式（不含 `CHECK` 关键字本身）。
    pub expr: &'static str,
    pub dialect: Option<Dialect>,
}

/// 生成列声明。
pub struct GeneratedColDecl {
    pub table: &'static str,
    pub col: &'static str,
    /// 如 `"tsvector"`。
    pub col_type: &'static str,
    pub expr: &'static str,
    pub stored: bool,
    pub dialect: Option<Dialect>,
}

/// SQLite FTS5 虚表声明（`sync_triggers` 引用 [`TRIGGERS`] 里的名字）。
///
/// ## ⚠ 引擎**既不建它、也不删它**（现状口径，2026-09-17 裁定）
///
/// `create_sql` 的作用只有两个：① 进期望侧指纹（`expected::build` ④ 段调
/// `claim_extra("l2.fts5:{名}|{DDL}")`）；② 让迁移 / 测试引用同一份 DDL 原文。
/// 它**不产生 `TableDecl`** ⇒ 引擎不会为虚表输出 `CREATE VIRTUAL TABLE`（建表责任在
/// 应用 / 迁移侧）。反过来引擎也**不得**把它当孤儿删掉 —— 那由 `plan` 侧的
/// `l2_virtual_tables_of`（逐表回调 `expected::l2_virtual_table_reason`）豁免兜住；
/// 缺那一环会产出 `DROP TABLE messages_fts`，而它上面挂着三条 FTS 同步触发器
/// （2026-09-17 实测）。
///
/// ⚠ **若将来要让引擎负责建**，必须**同时**改四处，否则「建」与「豁免」互相抵消：
/// ① 本声明的消费端（`expected` 产出 `TableDecl`）② `render`（渲染 `CREATE VIRTUAL TABLE`）
/// ③ `apply`（建表幂等）④ `plan` 撤掉上面那条豁免 —— 豁免让它在实况里不进孤儿候选，
/// 而期望侧一旦有了它，缺失时又要 `CreateTable`，两处语义会打架。
/// 详见 `facts-audit-2026-09-17.md` §7 第 10 项。
pub struct VirtualTableDecl {
    pub name: &'static str,
    pub create_sql: &'static str,
    pub sync_triggers: &'static [&'static str],
    pub dialect: Option<Dialect>,
}

/// 触发器声明（本项目仅 SQLite 侧有 FTS 同步触发器）。
pub struct TriggerDecl {
    pub name: &'static str,
    pub table: &'static str,
    pub dialect: Option<Dialect>,
    pub sql: &'static str,
}

/// 自定义函数声明。
///
/// `render` 是**函数指针而非字符串**：`ax_cjk_ngram` 的 SQL 含两个字符类占位符
/// （`__CJK__` / `__SEP__`），必须与 Rust 侧 `is_cjk` / `is_separator` 逐范围一致
/// （不一致的后果是索引侧与查询侧**静默失配**，两边都不报错）。用函数指针保证
/// 只有一处渲染逻辑，避免本文件再抄一份。
pub struct FunctionDecl {
    pub name: &'static str,
    pub args: &'static str,
    pub dialect: Option<Dialect>,
    pub render: fn() -> String,
}

/// 按表名模式展开的 tsvector 声明 —— 用于「每集合一张」的向量元数据表。
///
/// `v227` 对 `^vec_.*_meta$` 的**每一张**表都重建 `content_tsv` 生成列 + GIN 索引。
/// 这类表随向量集合创建而增多，无法枚举成静态条目；若不在 L2 里声明，引擎接管后
/// **每新建一个向量集合，其 content_tsv 与 GIN 都会缺失**（混合检索的 BM25 分支
/// 静默退化为全表扫描）。
pub struct PatternTsvDecl {
    /// 匹配**表名**的前缀/后缀（引擎侧用 `starts_with` + `ends_with` 判定）。
    pub table_prefix: &'static str,
    pub table_suffix: &'static str,
    pub column: &'static str,
    /// 参与 `tsv` 的源列（按顺序用 `' '` 连接）。
    pub source_columns: &'static [&'static str],
    /// **完整**的生成列表达式，与 [`GeneratedColDecl::expr`] 同口径（写死字面量）。
    ///
    /// ## 为什么 P4 必须补上它（P3 不需要，所以 P3 掩盖了这个缺口）
    ///
    /// P3 只打印 plan，`pattern_level` 产出 `AddColumn` / `AlterGenerated` 时用得到
    /// 的只有「列名」；`expr` 从未被读过。P4 要把同一条变更渲染成
    ///
    /// ```sql
    /// ALTER TABLE vec_x_meta ADD COLUMN content_tsv tsvector
    ///   GENERATED ALWAYS AS (<本字段>) STORED
    /// ```
    ///
    /// —— **没有本字段就渲染不出来**，只能 fail-closed 跳过，于是这条声明的效果
    /// **永远不会落地**（且是静默的：plan 里明明有一条 `AddColumn`）。
    /// 即「L2 声明必须足以渲染」，与「豁免必须显式化」（判据 **#381**）同族：
    /// 声明里缺的东西，不会在 diff 阶段报错，只会在执行阶段变成空操作。
    ///
    /// ⚠ 与 `v227_cjk_fts::tsv_expression` 是**同一条表达式的两份副本**（v227 按列
    /// 拼串、本字段写死）。P6 删除 `migrations/` 后，本字段即唯一来源；在那之前
    /// `pattern_tsv_expr_covers_source_columns` 锁死「列名集合与 `source_columns` 一致」。
    pub expr: &'static str,
    /// 由表名派生索引名。
    pub index_name: fn(&str) -> String,
    pub dialect: Option<Dialect>,
}

/// 派生向量元数据表的 GIN 索引名：`idx_{表名}_tsv`（与 `v227:216` 逐字一致）。
pub fn vec_meta_index_name(table: &str) -> String {
    format!("idx_{table}_tsv")
}

/// `PATTERN_TSV` → 生成列的 [`ColumnModel`]（P4 渲染 `ADD COLUMN` 用）。
///
/// `nullable: true` 不是随手取的：`v227:153` 的 DDL 里**没有** `NOT NULL`，两侧口径
/// 必须一致 —— 若这里写成 `false`，introspect 每轮都会判出一次可空性差异，
/// 引擎就会反复 `SET NOT NULL`（而生成列不允许改可空性 ⇒ 直接报错）。
pub fn pattern_tsv_column_model(decl: &PatternTsvDecl) -> ColumnModel {
    ColumnModel {
        name: decl.column.to_string(),
        sql_type: "tsvector".to_string(),
        nullable: true,
        default: None,
        primary_key: false,
        unique: false,
        generated: Some(decl.expr.to_string()),
        renamed_from: None,
        auto_increment: false,
    }
}

/// `PATTERN_TSV` → GIN 索引的 [`IndexModel`]（P4 渲染 `CREATE INDEX … USING GIN` 用）。
///
/// 与 `v227:156` 的 `CREATE INDEX … USING GIN ({column})` 对齐：单列、非 unique、
/// 非 partial、访问方法 `gin`。
pub fn pattern_tsv_index_model(decl: &PatternTsvDecl, table: &str) -> IndexModel {
    IndexModel {
        name: (decl.index_name)(table),
        cols: vec![decl.column.to_string()],
        unique: false,
        method: Some("gin".to_string()),
        where_clause: None,
    }
}

/// 孤儿豁免规则的**匹配形态**。
///
/// 两种形态对应**两种完全不同的理由**，故不合并成一个「前缀 + 精确名」的宽结构 ——
/// 混在一起会让「这条规则到底覆盖多大范围」无法从类型上读出来（判据 **#381**：
/// 声称范围必须与落实范围逐对象对账）。
pub enum OrphanExemptScope {
    /// 表**族**：前缀命中、且不以任一排除后缀结尾。
    Prefix {
        /// 匹配**表名**前缀。
        prefix: &'static str,
        /// 命中前缀但**不**豁免的后缀 —— 必须显式列出，禁止改成模糊匹配。
        ///
        /// 理由：`_dup_backup` 这类是**一次性备份残留**，顶着 `vec_` 前缀却不是运行时该族
        /// 成员。若为图省事把规则放宽成「`vec_` 前缀即豁免」，这类垃圾残留会**永久留下**且
        /// 再也不会被发现（判据 **#381**）。
        exclude_suffixes: &'static [&'static str],
    },
    /// 表**个体**：逐字相等的表名。
    ///
    /// 用于**基础设施表** —— 它们由别的子系统拥有（建表、写表、改表都不经过 L1/L2/L3）。
    /// 引擎既不该 `DROP` 它们，也不该照声明去 `ALTER` 它们。
    ///
    /// ⚠ 这里给的是**所有权**，不是「暂时别删」：后者是运行期人工白名单
    /// （`_ax_schema_orphan_whitelist`，带有效期与人工理由）的职责。把永久设施塞进
    /// 带过期的白名单，等于让它某天自动失效。
    Exact(&'static [&'static str]),
}

/// 孤儿豁免规则 —— 命中者**不参与孤儿判定**（不会被 `DROP TABLE`）。
///
/// ## 为什么不能只靠 `PATTERN_TSV` 顺带豁免
///
/// `PATTERN_TSV` 命中 ⇒ 排除出孤儿判定，这条**副作用**（`plan.rs::table_level` ②）
/// 在过去是向量表唯一的保命机制。问题在于：`PATTERN_TSV` 的设计目的是「动态表名的
/// tsvector 生成列」，与「这张表该不该被 DROP」是两件事。把豁免藏在另一条规则的匹配
/// 条件里 ⇒ **收紧 tsv 匹配就会静默删掉向量数据**。
///
/// 更具体的裂缝：`PATTERN_TSV` 的匹配是「前缀 `vec_` **且**后缀 `_meta`」，而向量族的
/// **基表**（`vec_{collection}`，`vector_store` 运行时创建）后缀不是 `_meta` ⇒ 它们从来
/// 没被豁免过。生产库实测 3 张基表共 **49135 行**会被 `DROP`（PLAN §八 风险 1d，
/// 2026-09-16 裁决）。**同一族的两类表保命机制不同，本身就是缺陷。**
///
/// ## 两种形态：族 vs 个体
///
/// `Prefix` 认「运行期按名字族创建的表」（引擎枚举不出，但确实是数据）；
/// `Exact` 认「归别的子系统所有的设施表」（引擎不该管）。
/// 两者的共同点是「不参与孤儿判定」，差别写在 [`OrphanExemptScope`] 上。
pub struct OrphanExemptDecl {
    pub scope: OrphanExemptScope,
    /// 写进 plan advisories 的理由 —— 引擎每次跑都打印，便于逐对象复核。
    ///
    /// [`OrphanExemptScope::Exact`] 形态**必须写明 owner**（哪个模块/子系统拥有它）：
    /// 一张设施表被豁免的唯一依据就是「它不归引擎管」，不写 owner 等于没有依据。
    pub reason: &'static str,
    pub dialect: Option<Dialect>,
}

// ═══════════════════════════════════════════════════════════════════════════
// SQLite 侧：FTS5 虚表（5 条）
// ⚠ 原 6 条里的 `trajectory_memories_fts` 已被 v101:372 随表 DROP ⇒ 不收录。
//   但**存量库里它仍然存在**（2026-09-17 只读实测：原库现存 5 张 `*_fts`，其中就有它）
//   ⇒ 「不收录」**不等于**放它一马：它没有声明，因此仍然照常判孤儿、照常被清。
//   这与下面 5 条的「不判孤儿」**正好相反** —— 别把两者混为一谈（下面有断言锁它）。
// ⚠ 引擎**既不建也不删**虚表（现状口径，2026-09-17 裁定）：见上面 `VirtualTableDecl` 的 doc。
// ═══════════════════════════════════════════════════════════════════════════

pub const VIRTUAL_TABLES: &[VirtualTableDecl] = &[
    VirtualTableDecl {
        name: "messages_fts",
        create_sql: "CREATE VIRTUAL TABLE IF NOT EXISTS messages_fts USING fts5(\
                     content, content=messages, content_rowid=rowid, tokenize='unicode61')",
        sync_triggers: &["messages_ai", "messages_ad", "messages_au"],
        dialect: Some(Dialect::Sqlite),
    },
    VirtualTableDecl {
        name: "trajectories_fts",
        create_sql: "CREATE VIRTUAL TABLE IF NOT EXISTS trajectories_fts USING fts5(\
                     id UNINDEXED, session_id UNINDEXED, topic, summary, content, \
                     outcome UNINDEXED, quality_score UNINDEXED, created_at UNINDEXED, \
                     tokenize='porter unicode61')",
        sync_triggers: &[],
        dialect: Some(Dialect::Sqlite),
    },
    VirtualTableDecl {
        name: "trajectory_skills_fts",
        create_sql: "CREATE VIRTUAL TABLE IF NOT EXISTS trajectory_skills_fts USING fts5(\
                     id UNINDEXED, name, description, content, category UNINDEXED, \
                     tags, created_at UNINDEXED, tokenize='porter unicode61')",
        sync_triggers: &[],
        dialect: Some(Dialect::Sqlite),
    },
    VirtualTableDecl {
        name: "trajectory_messages_fts",
        create_sql: "CREATE VIRTUAL TABLE IF NOT EXISTS trajectory_messages_fts USING fts5(\
                     id UNINDEXED, session_id UNINDEXED, role UNINDEXED, content, \
                     created_at UNINDEXED, tokenize='porter unicode61')",
        sync_triggers: &[],
        dialect: Some(Dialect::Sqlite),
    },
    VirtualTableDecl {
        name: "notes_fts",
        create_sql: "CREATE VIRTUAL TABLE IF NOT EXISTS notes_fts USING fts5(\
                     title, content, content='notes', content_rowid='rowid', \
                     tokenize='porter unicode61')",
        sync_triggers: &["notes_fts_ai", "notes_fts_ad", "notes_fts_au"],
        dialect: Some(Dialect::Sqlite),
    },
];

// ═══════════════════════════════════════════════════════════════════════════
// SQLite 侧：FTS5 同步触发器（6 条）
//   messages_* 来自 v100:1212-1220；notes_fts_* 来自 v104:37-47
// ═══════════════════════════════════════════════════════════════════════════

pub const TRIGGERS: &[TriggerDecl] = &[
    TriggerDecl {
        name: "messages_ai",
        table: "messages",
        dialect: Some(Dialect::Sqlite),
        sql: "CREATE TRIGGER IF NOT EXISTS messages_ai AFTER INSERT ON messages BEGIN \
              INSERT INTO messages_fts(rowid, content) VALUES (new.rowid, new.content); END",
    },
    TriggerDecl {
        name: "messages_ad",
        table: "messages",
        dialect: Some(Dialect::Sqlite),
        sql: "CREATE TRIGGER IF NOT EXISTS messages_ad AFTER DELETE ON messages BEGIN \
              INSERT INTO messages_fts(messages_fts, rowid, content) \
              VALUES('delete', old.rowid, old.content); END",
    },
    TriggerDecl {
        name: "messages_au",
        table: "messages",
        dialect: Some(Dialect::Sqlite),
        sql: "CREATE TRIGGER IF NOT EXISTS messages_au AFTER UPDATE OF content ON messages BEGIN \
              INSERT INTO messages_fts(messages_fts, rowid, content) \
              VALUES('delete', old.rowid, old.content); \
              INSERT INTO messages_fts(rowid, content) VALUES (new.rowid, new.content); END",
    },
    TriggerDecl {
        name: "notes_fts_ai",
        table: "notes",
        dialect: Some(Dialect::Sqlite),
        sql: "CREATE TRIGGER IF NOT EXISTS notes_fts_ai AFTER INSERT ON notes BEGIN \
              INSERT INTO notes_fts(rowid, title, content) \
              VALUES (new.rowid, new.title, new.content); END",
    },
    TriggerDecl {
        name: "notes_fts_ad",
        table: "notes",
        dialect: Some(Dialect::Sqlite),
        sql: "CREATE TRIGGER IF NOT EXISTS notes_fts_ad AFTER DELETE ON notes BEGIN \
              INSERT INTO notes_fts(notes_fts, rowid, title, content) \
              VALUES('delete', old.rowid, old.title, old.content); END",
    },
    TriggerDecl {
        name: "notes_fts_au",
        table: "notes",
        dialect: Some(Dialect::Sqlite),
        sql: "CREATE TRIGGER IF NOT EXISTS notes_fts_au AFTER UPDATE OF title, content ON notes BEGIN \
              INSERT INTO notes_fts(notes_fts, rowid, title, content) \
              VALUES('delete', old.rowid, old.title, old.content); \
              INSERT INTO notes_fts(rowid, title, content) \
              VALUES (new.rowid, new.title, new.content); END",
    },
];

// ═══════════════════════════════════════════════════════════════════════════
// PostgreSQL 侧：tsvector 生成列（6 条静态）
//   前 4 条来自 v100:1184-1199；notes / memory_items 来自 v227 的 n-gram 重建
//   ⚠ `trajectory_memories.tsv` 不收录：表已被 v101:396 DROP
//   ⚠ 表达式逐条不同：只有 notes / memory_items 走 ax_cjk_ngram（见模块文档「已知缺口」）
// ═══════════════════════════════════════════════════════════════════════════

pub const GENERATED_COLUMNS: &[GeneratedColDecl] = &[
    GeneratedColDecl {
        table: "messages",
        col: "content_tsv",
        col_type: "tsvector",
        expr: "to_tsvector('simple', COALESCE(content, ''))",
        stored: true,
        dialect: Some(Dialect::Postgres),
    },
    GeneratedColDecl {
        table: "trajectory_trajectories",
        col: "tsv",
        col_type: "tsvector",
        expr: "to_tsvector('simple', COALESCE(topic,'')||' '||COALESCE(summary,'')||' '||COALESCE(outcome,'')||' '||COALESCE(patterns,''))",
        stored: true,
        dialect: Some(Dialect::Postgres),
    },
    GeneratedColDecl {
        table: "trajectory_skills",
        col: "tsv",
        col_type: "tsvector",
        expr: "to_tsvector('simple', COALESCE(name,'')||' '||COALESCE(description,'')||' '||COALESCE(content,'')||' '||COALESCE(category,'')||' '||COALESCE(tags,''))",
        stored: true,
        dialect: Some(Dialect::Postgres),
    },
    GeneratedColDecl {
        table: "trajectory_messages",
        col: "tsv",
        col_type: "tsvector",
        expr: "to_tsvector('simple', COALESCE(content,'')||' '||COALESCE(role,''))",
        stored: true,
        dialect: Some(Dialect::Postgres),
    },
    GeneratedColDecl {
        table: "notes",
        col: "tsv",
        col_type: "tsvector",
        expr: "to_tsvector('simple', ax_cjk_ngram(COALESCE(title, '') || ' ' || COALESCE(content, '')))",
        stored: true,
        dialect: Some(Dialect::Postgres),
    },
    GeneratedColDecl {
        table: "memory_items",
        col: "content_tsv",
        col_type: "tsvector",
        expr: "to_tsvector('simple', ax_cjk_ngram(COALESCE(title, '') || ' ' || COALESCE(content, '') || ' ' || COALESCE(tags, '')))",
        stored: true,
        dialect: Some(Dialect::Postgres),
    },
];

/// 向量元数据表的生成列（动态表名，实测 3 张：`vec_capabilities_meta` + 每集合一张）。
pub const PATTERN_TSV: &[PatternTsvDecl] = &[PatternTsvDecl {
    table_prefix: "vec_",
    table_suffix: "_meta",
    column: "content_tsv",
    source_columns: &["content"],
    // 与 `v227:217` 的 `tsv_expression(&["content"])` 输出**逐字一致**。
    expr: "to_tsvector('simple', ax_cjk_ngram(COALESCE(content, '')))",
    index_name: vec_meta_index_name,
    dialect: Some(Dialect::Postgres),
}];

/// 孤儿豁免清单（**2 条规则**）。
///
/// | # | 形态 | 覆盖 | 理由 |
/// |---|---|---|---|
/// | 1 | `Prefix("vec_")` | 生产库 3 张基表 + 全部 `vec_*_meta` | `vector_store` 运行时按集合名创建（`vector_store.rs:200` / `:235`），L1+L2 **枚举不出**它们的名字 |
/// | 2 | `Exact` | `axagent_schema_version` | 归**迁移子系统**所有，引擎不该管（见下） |
///
/// ## 规则 2 的依据：基础设施表不归 diff 引擎管
///
/// `axagent_schema_version` 是 SeaORM 迁移子系统的元表（`dao/src/migrations/mod.rs:59`
/// `SCHEMA_VERSION_TABLE`），生产库实测 **74 行**。它**不是**「L1 声明缺口」：
///
/// - 建表 / 写表 / 改表全在 `crates/dao/src/migrations` 里，L1/L2/L3 **三层都不表达它**，
///   而且**不该**表达 —— 一旦补一个 L1 实体，引擎就会照声明去 `ALTER` 别人的内部表，
///   迁移子系统改表结构的那天就会与引擎互相打脸；
/// - 它也不适合走运行期白名单 `_ax_schema_orphan_whitelist`：那份带**有效期**，
///   而这是永久设施，到期即被 `DROP TABLE` ⇒ 迁移系统会认为从未迁移过。
///
/// ⚠ **本常量不进 [`total_declarations`]**：它是**判定策略**（「哪些表不参与孤儿判定」），
/// 不产生任何期望对象；`PATTERN_TSV` 则不同 —— 它产生「生成列 + GIN 索引」两个期望对象，
/// 所以算条目。混进条目账会让「L2 声明了多少对象」这个数字失去意义。
///
/// ⚠ 原名 `PATTERN_ORPHAN_EXEMPT`。加入规则 2 后**必须改名** —— 名字里的 `PATTERN`
/// 会变成一句谎（规则 2 与模式无关），而按名字理解范围正是这个文件最容易出错的地方。
pub const ORPHAN_EXEMPT: &[OrphanExemptDecl] = &[
    OrphanExemptDecl {
        scope: OrphanExemptScope::Prefix { prefix: "vec_", exclude_suffixes: &["_dup_backup"] },
        reason: "向量集合表（vec_{collection} / {collection}_meta）由运行时创建，L1+L2 枚举不出",
        dialect: None,
    },
    OrphanExemptDecl {
        scope: OrphanExemptScope::Exact(&["axagent_schema_version"]),
        reason: "owner = crates/dao/src/migrations（SeaORM 迁移子系统的元表，dao/src/migrations/mod.rs:59）：建表/写表/改表都在那边，L1+L2+L3 三层都不表达它、也不该表达它",
        dialect: None,
    },
];

// ═══════════════════════════════════════════════════════════════════════════
// 「非主库实体」声明（2 条 / 8 表）—— 引擎此前缺失的**库归属**维度
//
// ## 这条声明修的是什么（2026-09-16 生产库实测事故）
//
// `crates/entities/src/lib.rs:253-278` 有 8 个**真 `pub mod`** 的侧车库实体：
// `ast_*`(5) + `file_index` + `l2_index_snapshots` + `l2_search_results`。该文件自己
// 在注释里写着「⚠ 它们**不在主库**……在此声明的是 schema，与连接指向哪个库无关」。
//
// 但 `crates/dao/build.rs` 的扫描器是**纯行前缀匹配 `pub mod `、不认注释** ⇒ 这 8 个
// 模块照样进 `entity_registry!` ⇒ `expected.rs` 为它们建 PG `TableDecl` ⇒ 引擎在
// **所连的库**（生产 PG）计划建这 8 张表。实测：P4 首次真库执行建的 12 张表里
// **8 张是侧车库表**（`output/tmp-p4-created.log`），而它们在主库里是**永久空表**
// （8 张全 0 行，`output/tmp-d1-pg.log`）—— 真正的数据住在 `index.db` /
// `l2_cache.db` 两个独立 SQLite 文件里。
//
// 旧引擎天然没这个问题：`migrations::schema_diff::heal_all` **只补列不建表**
// （`heal_entity` 遇表不存在即跳过），所以「实体不在本库」这件事被静默容忍了。
// 新引擎**会建表** ⇒ 必须有一个显式声明把「这张表的家不在这里」说出来。
//
// ## 处置口径：skip（既不建也不删），不是 DROP
//
// 命中者**两个方向都退出**：不进期望集（不再 CREATE / ALTER），也不参与孤儿判定
// （不会被 `DROP TABLE`）。理由与 `vec_` 一族同源 —— **表名与实际拥有者不是同一个
// 对象时，引擎不得对它执行任何 DDL**：
//
// - 若只从期望集移除、让它落进孤儿判定 ⇒ 引擎会 `DROP` 一批**名字属于别的子系统**的
//   表。主库里那 8 张空壳确实该清，但那是**一次性人工清理**（0 行、可验证），不是
//   引擎该在每轮扫描里自己决定的事。判据 **#381**：声称范围必须与落实范围逐对象对账。
// - ⚠ 已知残留（**登记，未擅动**）：主库里那 8 张空壳**仍在**。本声明的作用是
//   「不再建、也不再 maintenance 它们」，清理它们需要单独授权。详见 PLAN §十一·八⑧。
//
// ## 为什么**双方言都**排除（2026-09-17 更正；原为「只排除 PG」）
//
// 原文写的是「引擎没有『库身份』维度，只有 `Dialect`；主库是 PG ⇒ `Some(Postgres)` 就是
// 当前信息量下最准的表述」。⚠ **那句话自己写明了前提**（「主库是 PG」），而本项目的主库
// 方言**由用户在设置中设定**（SQLite / PostgreSQL 二选一）⇒ 该前提并不成立。
//
// 错在把两件事当成一件：
//   * 前提（**成立**）：这 8 张表的**家**是 SQLite 侧车库（`index.db` / `l2_cache.db` 两个独立文件）；
//   * 原结论（**不成立**）：「连接的方言是 SQLite」⇒ 不得把它们排除出期望集。
// 「连接的方言 = SQLite」**不蕴含**「连的就是它们的家」—— 主库选 SQLite 时，主库是**第三个**
// 文件，这 8 张表同样不该在那里。实测后果：`expected::build(Sqlite)` 仍含这 8 张 ⇒ 引擎在
// SQLite 主库里把它们建出来，与 2026-09-16 在 PG 上发生的事故**同形**
// （`PLAN-declarative-schema-sync.md:2144` 标题即「8 张侧车库表被引擎建进了主库」）。
//
// 原文担心的「指向 `index.db` 的那次 reconcile 会把整个侧车库判成孤儿」**不会发生**：
// `dialect: None` 与 `Some(..)` 一样是**两个方向都退出**（既不进期望集，也不进孤儿判定，
// 见 `plan.rs::diff_with` 的 `orphan_exempt.extend(non_main_db_tables_of(..))`）。
// 只有「从期望集移除、却留在孤儿判定里」那种改法才会 DROP —— 那恰是本文件上面第 3 条
// 已经写明要避免的**另一种**改法。且实测全仓**没有任何一处**把侧车库连接交给
// `bootstrap_schema` / `plan::diff`（调用点只有主库 + 测试/示例）⇒ 那是一次尚未存在的调用。
//
// 这 8 张表的家**不依赖本引擎**：三个 owner crate 都自带实体级 DDL，均
// `create_table_from_entity` + `if_not_exists` —— `crates/search/src/ast_index.rs`（5 张
// `ast_*`）、`crates/search/src/file_index.rs`（`file_index`）、
// `crates/disk-cache/src/lib.rs`（2 张 `l2_*`，在 `new()` 里无条件调用）。
//
// ⚠ 出路不是继续拿方言代理「这个库是谁」，而是引入**库身份**维度 ——
// `PLAN-declarative-schema-sync.md:2160` 早已指出「引擎缺一个**『实体归属哪个库』的声明维度**」。
// ═══════════════════════════════════════════════════════════════════════════

/// 一组**不在本方言库里**的表 —— 命中者既不参与期望集，也不参与孤儿判定。
///
/// 条目的 `tables` 必须**逐字**是实体声明的 `table_name`：`expected.rs` 的测试会拿
/// 它去 `build(Sqlite)` 里认领（认不到 = 写错名字或实体被改名 ⇒ 测试红），这样
/// 「手抄一份表名」不会静默腐烂。
pub struct NonMainDbDecl {
    pub tables: &'static [&'static str],
    /// 写进 plan advisories 的理由 —— 与 [`OrphanExemptDecl::reason`] 同样**必须写明
    /// owner**（哪个 crate / 哪个文件拥有它）。说不出 owner 就不该排除。
    pub reason: &'static str,
    /// 从哪个方言的**期望集**里排除。`None` = **双方言都不认这些表**（本项目当前用法：
    /// 侧车库的家在独立 SQLite 文件里 ⇒ 无论主库是 PG 还是 SQLite，都不该把它们建进主库）。
    ///
    /// ⚠ `Some(Dialect)` 只在「该表真的归属某一种方言的主库」时才用。拿它代理「这个库是谁」
    /// 会在**主库方言可配**时静默失效 —— 见上面「为什么双方言都排除」。
    pub dialect: Option<Dialect>,
}

pub const NON_MAIN_DB: &[NonMainDbDecl] = &[
    NonMainDbDecl {
        tables: &[
            "ast_call_edges",
            "ast_classes",
            "ast_functions",
            "ast_interfaces",
            "ast_variables",
            "file_index",
        ],
        reason: "owner = src/indexing_triggers.rs:86 `INDEX_DB_FILENAME`（`index.db`，一个独立 \
                 SQLite 文件）。这些实体只描述**那个文件**的 schema（见 entities/src/lib.rs:253-278）",
        // 2026-09-17：原为 `Some(Dialect::Postgres)`。那是以「主库恒为 PG」为前提写的
        // 代理；主库方言由用户可配后它只在一条路径上成立。理由见上方「为什么双方言都排除」。
        dialect: None,
    },
    NonMainDbDecl {
        tables: &["l2_index_snapshots", "l2_search_results"],
        reason: "owner = crates/disk-cache（自持 SQLite 文件 `l2_cache.db`，见该 crate 的 \
                 crates/disk-cache/src/lib.rs:24-26「三张表都是侧车库……不在主库」）",
        dialect: None,
    },
];

/// 命中 `table` 的那条非主库声明。
///
/// ⚠ **方言判定不在这里** —— 与 `PATTERN_TSV` / `ORPHAN_EXEMPT` 同构：本文件只放数据，
/// 「`dialect: None` = 双方言通用」这条语义只有一个实现（`expected.rs::l2_applies`）。
/// 在这里再写一遍（`unwrap_or(dialect)` 之类）就是第二份可能漂移的同义语义。
pub fn non_main_db_decl_for(table: &str) -> Option<&'static NonMainDbDecl> {
    NON_MAIN_DB.iter().find(|d| d.tables.contains(&table))
}

/// 命中 `(table, col)` 的那条表达式默认值声明。
pub fn expr_default_decl_for(table: &str, col: &str) -> Option<&'static ExprDefaultDecl> {
    EXPR_DEFAULTS.iter().find(|d| d.cols.iter().any(|(t, c)| *t == table && *c == col))
}

// ═══════════════════════════════════════════════════════════════════════════
// 「表达式型默认值」声明（1 条 / 8 列）
//
// ## 为什么 L1 表达不了它，而它又不该被删
//
// 生产库有 8 列是 `TEXT NOT NULL DEFAULT (to_char(CURRENT_TIMESTAMP AT TIME ZONE 'UTC',
// 'YYYY-MM-DD HH24:MI:SS'))`（出自 `v100` 迁移的合并基线
// DDL）。实体层**没有任何语法**能表达这个默认值：
//
// - `#[sea_orm(default_value = X)]` 收的是 `Into<Value>`（字面量），写不了函数调用；
// - `#[sea_orm(default_expr = "...")]` 收的是 **Rust 表达式**（`syn::parse_str::
//   <TokenStream>`），只能写成 `Expr::cust("to_char(...)")` 这种 PG 专有 SQL ——
//   而 L1 声明是**方言无关**的，把它写进实体就会让 SQLite 侧的期望集带上一条
//   SQLite 语法里不存在的 `to_char(...)`。
//
// ⇒ 于是这 8 列在期望侧 `default = None`、实况侧非空 ⇒ 判据「默认值只比有没有」产出
// 8 条 `DROP DEFAULT`。**执行它 = 删掉一条 L1 只是「表达不了」的默认值。**
//
// ⚠ 这才是本声明要防的事：**「声明里没有」≠「不该存在」**。引擎若因自身词汇不足而删
// schema，那和「把枚举不出的表判孤儿」是同一类事故（见 [`ORPHAN_EXEMPT`] 的模块文档）。
// 实测该默认值与主流写入 helper **同格式** —— `harness::util_fns::now_datetime_str()`
// 就是 `"%Y-%m-%d %H:%M:%S"`（`artifacts` / `stored_files` / `tool_executions` /
// `conversation_branches` 四个写入点都用它），所以这条默认值是**对的安全网**，不是垃圾。
//
// ## 为什么连 SQL 文本一起声明，而不是只声明「有个默认值」
//
// 判据本身只比「有没有」，只写布尔也够。带上文本是为了让**未来的漂移可见**：
// 文本不一致时 `plan.rs` 会出 `text-diff … default` advisory（不产生 DDL），
// 而不是无声无息。文本**逐字取自 `pg_get_expr` 输出**（含 PG 自己补的 `::text`），
// 所以本声明的 8 列当前逐字相等 —— 真库实测该 8 列 **0 条** advisory（2026-09-16）。
//
// ⚠ **别把这条性质推广到别的列**。`normalize_default` 只在**实况侧**被调用
// （`introspect/pg.rs` 与 `introspect/sqlite.rs`），**期望侧从不调用它**。本声明的
// 8 列能逐字相等，靠的是「文本手工抄自 `pg_get_expr`」这条纪律，**不是**靠「两侧走
// 同一个归一函数」—— 此处原先就是这么写的，与代码事实相反（已改）。
// 全库实测：273 条 advisory 里 266 条正是**别的列**的 default 文本差异
// （`'x'` vs `'x'::text`、`true` vs `TRUE`、`0` vs `0.0`、`'0'` vs `0`）。
// ═══════════════════════════════════════════════════════════════════════════

/// 一组共用**同一个表达式默认值**的列。
///
/// `cols` 而非「一条一列」：这 8 列的默认值来自**同一条 DDL 语句**，逐条抄 8 份表达式
/// 属于「手抄常量多副本」反模式（改一处漏七处）。冻结计数同时锁**规则数**与**列数**。
pub struct ExprDefaultDecl {
    /// `(表, 列)` 逐字 —— 对应实体的 `table_name` 与字段的**列名**。
    pub cols: &'static [(&'static str, &'static str)],
    /// 与 `pg_get_expr` 输出逐字一致的 SQL 表达式。
    pub sql: &'static str,
    /// 写进本文件用，不进日志（列级 advisory 只报文本差异）。
    pub reason: &'static str,
    pub dialect: Option<Dialect>,
}

pub const EXPR_DEFAULTS: &[ExprDefaultDecl] = &[ExprDefaultDecl {
    cols: &[
        ("artifacts", "updated_at"),
        ("backup_manifests", "created_at"),
        ("conversation_branches", "created_at"),
        ("gateway_diagnostics", "created_at"),
        ("import_jobs", "created_at"),
        ("memory_items", "updated_at"),
        ("stored_files", "created_at"),
        ("tool_executions", "created_at"),
    ],
    sql: "to_char((CURRENT_TIMESTAMP AT TIME ZONE 'UTC'::text), 'YYYY-MM-DD HH24:MI:SS'::text)",
    reason: "出自 migrations/v100_consolidated.rs 的合并基线 DDL；实体层无法表达函数型默认值 \
             （见本组的模块文档）",
    dialect: Some(Dialect::Postgres),
}];

/// `(table, col)` 命中的表达式默认值 —— **SQL 文本**，不是「有没有」。
///
/// ⚠ 消费点是 `expected.rs`，且**只在 L1 没给出默认值时才用**：L1 是真相源，L2 只补
/// 它表达不了的东西。反过来（L2 覆盖 L1）会让实体里写的字面量默认值静默失效。
pub fn expr_default_sql_for(table: &str, col: &str) -> Option<&'static str> {
    expr_default_decl_for(table, col).map(|d| d.sql)
}

// ═══════════════════════════════════════════════════════════════════════════
// PostgreSQL 侧：自定义函数（1 条）
//   ⚠ 静态扫描的 8 个类别覆盖不到「函数」，若漏声明：生成列表达式引用不存在的
//     函数 ⇒ 建列直接报错（不会静默）
// ═══════════════════════════════════════════════════════════════════════════

/// `ax_cjk_ngram(text)` —— 索引侧与查询侧共用的唯一分词器。
///
/// `render` 复用 `crate::cjk_ngram::render_function_sql()`：该函数把 `CJK_CLASS` /
/// `SEP_CLASS` 两个范围片段注入 SQL 模板的 `__CJK__` / `__SEP__` 占位符，而这些
/// 常量必须与 `crates/search/src/text_ngram.rs` 的 `is_cjk` / `is_separator`
/// 逐范围一致。**定义只有一个处所（`crate::cjk_ngram`），任何地方不得另抄一份**
/// —— 副本会各自腐烂，而腐烂的后果是索引侧与查询侧静默失配。
pub const FUNCTIONS: &[FunctionDecl] = &[FunctionDecl {
    name: "ax_cjk_ngram",
    args: "text",
    dialect: Some(Dialect::Postgres),
    render: crate::cjk_ngram::render_function_sql,
}];

// ═══════════════════════════════════════════════════════════════════════════
// CHECK 约束（3 条）
//
// 口径说明：迁移里与之相关的 DDL 共 8 条，但它们是**同 3 个约束的历代替换**：
//   v100:460/470（建表内联，11 值）
//     → v200:409/425（ALTER 替换，14 值，含 opc-industry）
//     → crates/dao/src/migrations/mod.rs:582/598（ALTER 替换，14 值，含 opc-domain_pack）
// 生产库实测正是 3 个约束（`agency_experts_category_check` /
// `agent_profiles_category_check` / `ck_opc_kpi_period_format`），与去重结果一致。
// 若照 8 条 DDL 录入，会产生 3 条重复声明 + 1 条同名互斥分支（v230 的 NOT VALID 版）。
//
// ⚠ 白名单必须与 `migrations::ensure_category_check_constraints`（crates/dao/src/migrations/mod.rs:546-548）
//   和 `opc_setup` 种子写值保持同步。历史上该列表被上游合并覆盖回 9 值版，
//   导致存量 opc-* 行把 ADD CONSTRAINT 顶回，且 DROP 先行 ⇒ 表约束整个缺失
//   （crates/dao/src/migrations/mod.rs:533-536 有记录）。本处为 14 值版。
// ═══════════════════════════════════════════════════════════════════════════

pub const CHECK_CONSTRAINTS: &[CheckDecl] = &[
    CheckDecl {
        table: "agency_experts",
        name: "agency_experts_category_check",
        expr: "category = ANY (ARRAY['general','development','security','data','finance','devops','design','writing','business','opc-company','opc-experts','opc-domain_pack','opc-domain','stock-analysis'])",
        dialect: Some(Dialect::Postgres),
    },
    CheckDecl {
        table: "agent_profiles",
        name: "agent_profiles_category_check",
        expr: "category = ANY (ARRAY['general','development','security','data','finance','devops','design','writing','business','opc-company','opc-experts','opc-domain_pack','opc-domain','stock-analysis'])",
        dialect: Some(Dialect::Postgres),
    },
    CheckDecl {
        table: "opc_kpi_records",
        name: "ck_opc_kpi_period_format",
        expr: "period ~ '^\\d{4}-\\d{2}$'",
        dialect: Some(Dialect::Postgres),
    },
];

// ═══════════════ GIN 索引（tsvector 全文检索，实体无访问方法开关） ═══════════════
const GIN_INDEXES: &[IndexDecl] = &[
    IndexDecl {
        table: "memory_items",
        name: "idx_memory_items_tsv",
        cols: &["content_tsv"],
        unique: false,
        method: Some("gin"),
        where_clause: None,
        dialect: Some(Dialect::Postgres), // ← ?
    },
    IndexDecl {
        table: "messages",
        name: "idx_messages_content_tsv",
        cols: &["content_tsv"],
        unique: false,
        method: Some("gin"),
        where_clause: None,
        dialect: Some(Dialect::Postgres), // ← v100_consolidated.rs
    },
    IndexDecl {
        table: "notes",
        name: "idx_notes_tsv",
        cols: &["tsv"],
        unique: false,
        method: Some("gin"),
        where_clause: None,
        dialect: Some(Dialect::Postgres), // ← v104_notes_fts.rs
    },
    IndexDecl {
        table: "trajectory_messages",
        name: "idx_traj_messages_tsv",
        cols: &["tsv"],
        unique: false,
        method: Some("gin"),
        where_clause: None,
        dialect: Some(Dialect::Postgres), // ← v100_consolidated.rs
    },
    IndexDecl {
        table: "trajectory_skills",
        name: "idx_traj_skills_tsv",
        cols: &["tsv"],
        unique: false,
        method: Some("gin"),
        where_clause: None,
        dialect: Some(Dialect::Postgres), // ← v100_consolidated.rs
    },
    IndexDecl {
        table: "trajectory_trajectories",
        name: "idx_traj_trajectories_tsv",
        cols: &["tsv"],
        unique: false,
        method: Some("gin"),
        where_clause: None,
        dialect: Some(Dialect::Postgres), // ← v100_consolidated.rs
    },
];

// ═══════════════ 多列索引（实体 indexed 只作用单字段，无复合键形式） ═══════════════
const MULTI_COL_INDEXES: &[IndexDecl] = &[
    IndexDecl {
        table: "agent_sessions",
        name: "idx_sessions_user",
        cols: &["conversation_id", "total_tokens DESC"],
        unique: false,
        method: None,
        where_clause: None,
        dialect: None, // ← v100_consolidated.rs
    },
    // ⚠ `idx_chat_run_events_run_id`（v207）曾在此 —— **P3 删除**，理由见 `counts`
    //    模块的 `indexes_on_entity_less_tables_are_not_declared`：`chat_run_events`
    //    没有实体，按 PLAN §五·一 B 组的裁决它整张表交孤儿判定处置（引擎上线后
    //    DROP），L2 再认领它的索引就是自相矛盾（`DROP TABLE` 与 `CREATE INDEX`
    //    同轮出现）。**无实体表上的索引一律不进 L2**，处置随表本身。
    IndexDecl {
        table: "dynamic_ui_pins",
        name: "idx_dynamic_ui_pins_group",
        cols: &["group_name", "position"],
        unique: false,
        method: None,
        where_clause: None,
        dialect: None, // ← v100_consolidated.rs
    },
    IndexDecl {
        table: "dynamic_ui_schema_versions",
        name: "idx_dyn_ui_schema_versions_created",
        cols: &["schema_id", "created_at DESC"],
        unique: false,
        method: None,
        where_clause: None,
        dialect: None, // ← v100_consolidated.rs
    },
    IndexDecl {
        table: "financial_snapshots",
        name: "idx_financial_snapshots_stock_date",
        cols: &["stock_code", "snapshot_date"],
        unique: false,
        method: None,
        where_clause: None,
        dialect: None, // ← v200_axinvest_stock_tables.rs
    },
    IndexDecl {
        table: "fleet_messages",
        name: "uq_fleet_messages_convo_seq",
        cols: &["fleet_id", "conversation_id", "seq"],
        unique: true,
        method: None,
        where_clause: None,
        dialect: None, // ← v229_create_fleet_messages.rs
    },
    IndexDecl {
        table: "gateway_usage",
        name: "idx_gateway_usage_key",
        cols: &["key_id", "created_at DESC"],
        unique: false,
        method: None,
        where_clause: None,
        dialect: None, // ← v100_consolidated.rs
    },
    IndexDecl {
        table: "index_jobs",
        name: "idx_index_jobs_container",
        cols: &["container_type", "container_id"],
        unique: false,
        method: None,
        where_clause: None,
        dialect: None, // ← v100_consolidated.rs
    },
    IndexDecl {
        table: "index_jobs",
        name: "idx_index_jobs_item",
        cols: &["container_type", "item_id"],
        unique: false,
        method: None,
        where_clause: None,
        dialect: None, // ← v100_consolidated.rs
    },
    IndexDecl {
        table: "index_jobs",
        name: "idx_index_jobs_status",
        cols: &["status", "priority DESC", "created_at"],
        unique: false,
        method: None,
        where_clause: None,
        dialect: None, // ← v100_consolidated.rs
    },
    IndexDecl {
        table: "lesson_applications",
        name: "idx_lesson_applications_lesson_outcome",
        cols: &["lesson_id", "outcome_at_validation"],
        unique: false,
        method: None,
        where_clause: None,
        dialect: None, // ← v201_lesson_application_tracking.rs
    },
    IndexDecl {
        table: "market_mainlines",
        name: "idx_market_mainlines_date_theme",
        cols: &["mainline_date", "theme"],
        unique: false,
        method: None,
        where_clause: None,
        dialect: None, // ← v205_market_mainline.rs
    },
    IndexDecl {
        table: "memory_items",
        name: "idx_memory_items_ns_tier",
        cols: &["namespace_id", "tier"],
        unique: false,
        method: None,
        where_clause: None,
        dialect: None, // ← v132_memory_access_indexes.rs
    },
    IndexDecl {
        table: "messages",
        name: "idx_messages_conv_created",
        cols: &["conversation_id", "created_at DESC"],
        unique: false,
        method: None,
        where_clause: None,
        dialect: None, // ← v100_consolidated.rs
    },
    IndexDecl {
        table: "news_archive",
        name: "news_archive_source_article_code_key",
        cols: &["source", "article_code"],
        unique: true,
        method: None,
        where_clause: None,
        dialect: None, // ← （静态扫描未覆盖：表级内联 UNIQUE 约束）
    },
    IndexDecl {
        table: "note_backlinks",
        name: "idx_note_backlinks_vault_source",
        cols: &["vault_id", "source_note_id"],
        unique: false,
        method: None,
        where_clause: None,
        dialect: None, // ← v103_wiki_graph_perf.rs
    },
    IndexDecl {
        table: "note_backlinks",
        name: "idx_note_backlinks_vault_target",
        cols: &["vault_id", "target_note_id"],
        unique: false,
        method: None,
        where_clause: None,
        dialect: None, // ← v103_wiki_graph_perf.rs
    },
    IndexDecl {
        table: "note_links",
        name: "idx_note_links_vault_source",
        cols: &["vault_id", "source_note_id"],
        unique: false,
        method: None,
        where_clause: None,
        dialect: None, // ← v103_wiki_graph_perf.rs
    },
    IndexDecl {
        table: "note_links",
        name: "idx_note_links_vault_target",
        cols: &["vault_id", "target_note_id"],
        unique: false,
        method: None,
        where_clause: None,
        dialect: None, // ← v103_wiki_graph_perf.rs
    },
    IndexDecl {
        table: "notes",
        name: "idx_notes_vault_deleted",
        cols: &["vault_id", "is_deleted"],
        unique: false,
        method: None,
        where_clause: None,
        dialect: None, // ← v103_wiki_graph_perf.rs
    },
    IndexDecl {
        table: "opc_demand_leads",
        name: "idx_opc_demand_leads_fingerprint",
        cols: &["platform", "content_fingerprint"],
        unique: true,
        method: None,
        where_clause: None,
        dialect: None, // ← v138_demand_lead_dedupe_fingerprint.rs
    },
    IndexDecl {
        table: "opc_demand_subscriptions",
        name: "idx_opc_demand_subs_due",
        cols: &["enabled", "last_scanned_at"],
        unique: false,
        method: None,
        where_clause: None,
        dialect: None, // ← v135_demand_subscriptions.rs
    },
    IndexDecl {
        table: "opc_publish_schedules",
        name: "idx_opc_publish_schedules_content",
        cols: &["content_ref_type", "content_ref_id"],
        unique: false,
        method: None,
        where_clause: None,
        dialect: None, // ← v217_opc_publish_schedules.rs
    },
    IndexDecl {
        table: "paper_positions",
        name: "idx_paper_positions_portfolio_status",
        cols: &["portfolio_id", "status"],
        unique: false,
        method: None,
        where_clause: None,
        dialect: None, // ← v204_paper_portfolio.rs
    },
    IndexDecl {
        table: "session_events",
        name: "idx_session_events_session_seq",
        cols: &["session_id", "seq"],
        unique: true,
        method: None,
        where_clause: None,
        dialect: None, // ← v139_create_session_events.rs
    },
    IndexDecl {
        table: "session_events",
        name: "idx_session_events_session_type",
        cols: &["session_id", "event_type"],
        unique: false,
        method: None,
        where_clause: None,
        dialect: None, // ← v139_create_session_events.rs
    },
    IndexDecl {
        table: "stock_analyses",
        name: "idx_stock_analyses_code_date",
        cols: &["stock_code", "analysis_date"],
        unique: false,
        method: None,
        where_clause: None,
        dialect: None, // ← v200_axinvest_stock_tables.rs
    },
    IndexDecl {
        table: "stock_reflections",
        name: "idx_stock_reflections_code_asof",
        cols: &["stock_code", "as_of_date"],
        unique: false,
        method: None,
        where_clause: None,
        dialect: None, // ← v200_axinvest_stock_tables.rs
    },
    IndexDecl {
        table: "strategy_performance",
        name: "idx_strategy_performance_strategy_period",
        cols: &["strategy_id", "period"],
        unique: false,
        method: None,
        where_clause: None,
        dialect: None, // ← v200_axinvest_stock_tables.rs
    },
    IndexDecl {
        table: "task_events",
        name: "idx_task_events_source_created",
        cols: &["source", "created_at"],
        unique: false,
        method: None,
        where_clause: None,
        dialect: None, // ← v226_task_ledger.rs
    },
    IndexDecl {
        table: "task_events",
        name: "idx_task_events_status_created",
        cols: &["to_status", "created_at"],
        unique: false,
        method: None,
        where_clause: None,
        dialect: None, // ← v226_task_ledger.rs
    },
    IndexDecl {
        table: "task_events",
        name: "idx_task_events_task_created",
        cols: &["task_id", "created_at"],
        unique: false,
        method: None,
        where_clause: None,
        dialect: None, // ← v226_task_ledger.rs
    },
    IndexDecl {
        table: "trajectory_skill_executions",
        name: "idx_traj_skill_exec",
        cols: &["skill_id", "created_at"],
        unique: false,
        method: None,
        where_clause: None,
        dialect: None, // ← v100_consolidated.rs
    },
    IndexDecl {
        table: "trajectory_steps",
        name: "idx_traj_steps_traj",
        cols: &["trajectory_id", "step_index"],
        unique: false,
        method: None,
        where_clause: None,
        dialect: None, // ← v100_consolidated.rs
    },
    IndexDecl {
        table: "workflow_tools",
        name: "idx_workflow_tools_workflow",
        cols: &["workflow_id", "status"],
        unique: false,
        method: None,
        where_clause: None,
        dialect: None, // ← v123_workflow_tools.rs
    },
    IndexDecl {
        table: "workflow_tools",
        name: "workflow_tools_workflow_id_tool_name_key",
        cols: &["workflow_id", "tool_name"],
        unique: true,
        method: None,
        where_clause: None,
        dialect: None, // ← （静态扫描未覆盖：表级内联 UNIQUE 约束）
    },
];

// ═══════════════ partial 索引（实体无 WHERE 条件表达） ═══════════════
const PARTIAL_INDEXES: &[IndexDecl] = &[
    IndexDecl {
        table: "messages",
        name: "idx_messages_branch",
        cols: &["branch_id"],
        unique: false,
        method: None,
        where_clause: Some("(branch_id IS NOT NULL)"),
        dialect: None, // ← v100_consolidated.rs
    },
    IndexDecl {
        table: "retrieval_hits",
        name: "idx_retrieval_hits_feedback",
        cols: &["feedback"],
        unique: false,
        method: None,
        where_clause: Some("(feedback IS NOT NULL)"),
        dialect: Some(Dialect::Postgres), // ← v111_retrieval_hits_feedback.rs（模板）：PG 分支带 WHERE，SQLite 分支为普通索引
    },
    IndexDecl {
        table: "tool_call_logs",
        name: "idx_tool_call_logs_success",
        cols: &["success"],
        unique: false,
        method: None,
        where_clause: Some("(success = 0)"),
        dialect: None, // ← v112_feedback_data_lake.rs
    },
    IndexDecl {
        table: "workflow_executions",
        name: "idx_workflow_executions_paused",
        cols: &["status"],
        unique: false,
        method: None,
        where_clause: Some("(status = 'paused'::text)"),
        dialect: None, // ← v117_workflow_execution_resume.rs
    },
    IndexDecl {
        table: "workflow_templates",
        name: "idx_workflow_templates_mission_hash",
        cols: &["mission_hash"],
        unique: false,
        method: None,
        where_clause: Some("(mission_hash IS NOT NULL)"),
        dialect: None, // ← v100_consolidated.rs
    },
    // ⚠ 这是本清单**唯一一条 unique 的 partial 索引**，且它**必须**排除 `rl_checkpoint:%`。
    // 裸 `UNIQUE (name)` 是不可行的 —— 三方一致地登记了「检查点允许同名多行」：
    // ① `harness/src/trajectory_types.rs::stable_id_for_name` 的 doc（「需要同名多行的
    //    场景不要走 `new`：典型是 `rl_checkpoint:`，身份是调用方给的主键」）；
    // ② `commands/rl_training.rs` 用**结构体字面量**自定 `id`，`name` 才是
    //    `format!("rl_checkpoint:{}", ckpt.name)`；
    // ③ `trajectory/src/storage.rs::save_pattern_keeps_distinct_ids_with_same_name_apart`
    //    —— 刻意锁住的反向守卫（同名异 id 必须仍是两行）。
    // 裸唯一键会直接打红 ③，并在线上把「同名不同 epoch 的检查点」压成一行。
    //
    // ⚠ **双方言表达**（`dialect: None`，2026-09-17 由 `Some(Postgres)` 改过来）——
    // 谓词写成 **SQL 标准形态 `NOT LIKE`**，因为它是两家都能发的交集写法：
    // · PG 侧：`pg_get_indexdef` 会把 `NOT LIKE` **回读**成 `!~~ 'x'::text`。比对侧
    //   `normalize_sql_expr` 里的 `fold_like_operators` 把两种写法折成同一个串；
    //   **不折的后果不是「多做一件事」而是 fail-stop** —— `plan::same_index` 恒判不等 ⇒
    //   plan 把同名索引拆成「期望 CreateIndex + 实况 DropIndex」⇒ 启动路径 `additive_only`
    //   只建不删 ⇒ 执行撞「已存在」（同 `model.rs::strip_default_sort_dir` 记录的 ASC 事故）。
    // · SQLite 侧：`render::sqlite_expr` 同样跑 `strip_casts` + `fold_like_operators` ⇒
    //   本声明在 SQLite 上也渲染得出合法 DDL（原先留 `Some(Postgres)` ⇒ SQLite 期望集
    //   整条不含它 ⇒ 该后端下 C-#2 的同名覆盖缺陷**静默保留**）。
    //
    // ⚠ **边界登记（实测，勿当理论）**：SQLite 的 `LIKE` 默认**不区分 ASCII 大小写**，
    // 而 PG 的 `LIKE` 区分 ⇒ 对**手写的**大写前缀（`RL_CHECKPOINT:x`）两侧宽严不同
    // （`output/tmp-sqlite-like-partial-probe.log`），且 SQLite 侧还会随
    // `PRAGMA case_sensitive_like` 变（`output/tmp-sqlite-csl-probe.log`）。
    // 本表的前缀一律由 `format!("rl_checkpoint:{}", …)` 生成（恒小写）⇒ 该差异**不可达**。
    // 刻意不改用 `substr(...)` 之类「大小写严格」的写法：那会失去「与 PG 回读形态可对账」
    // 这条唯一可靠的验证手段（改成 `substr` 就无法确认 PG 会把表达式重写成什么）。
    IndexDecl {
        table: "trajectory_patterns",
        name: "uq_trajectory_patterns_name",
        cols: &["name"],
        unique: true,
        method: None,
        // 写 SQL 标准形态：同时满足「PG 回读可对齐」与「SQLite 可直接发出去」。
        where_clause: Some("(name NOT LIKE 'rl_checkpoint:%')"),
        dialect: None, // ← 声明式新增（无迁移出处），缺陷编号 C-#2
    },
];

// 静态条目数自证：GIN 6 / MULTI 35 / PARTIAL 6

// ═══════════════════════════════════════════════════════════════════════════
// 汇总视图 —— 引擎（P3+）消费入口
// ═══════════════════════════════════════════════════════════════════════════

/// 全部索引声明的迭代器（GIN + 多列 + partial 三来源合并）。
///
/// 合并而非让调用方自己拼，是为了让「新增一类索引」只需改本文件一处。
pub fn all_indexes() -> impl Iterator<Item = &'static IndexDecl> {
    GIN_INDEXES.iter().chain(MULTI_COL_INDEXES).chain(PARTIAL_INDEXES)
}

/// L2 声明的条目总数（用于漂移告警与 `#[cfg(test)]` 锁定）。
pub fn total_declarations() -> usize {
    VIRTUAL_TABLES.len()
        + TRIGGERS.len()
        + GENERATED_COLUMNS.len()
        + PATTERN_TSV.len()
        + FUNCTIONS.len()
        + CHECK_CONSTRAINTS.len()
        + GIN_INDEXES.len()
        + MULTI_COL_INDEXES.len()
        + PARTIAL_INDEXES.len()
}

// ═══════════════════════════════════════════════════════════════════════════
// 自校验 —— 这不是「可选测试」，是 L2 清单的唯一守卫
//
// 为什么必须有：本清单的失效方式是**静默的** —— 少一条 ⇒ 引擎把该对象判为孤儿
// 并 DROP ⇒ `notes.tsv` 一条就涉及 48590 行，且 CJK 全文检索整体失效而无任何报错。
// 计数字面量刻意写成常量（不是 `.len()` 自比），改条目必须显式改断言。
// ═══════════════════════════════════════════════════════════════════════════

#[cfg(test)]
mod counts {
    use super::*;

    // 冻结值 —— 取自生产库实测（output/tmp-p2-extras-survey.json + tmp-p2-diff.log）
    const EXPECT_VIRTUAL_TABLES: usize = 5;
    const EXPECT_TRIGGERS: usize = 6;
    const EXPECT_GENERATED_COLUMNS: usize = 6;
    const EXPECT_PATTERN_TSV: usize = 1;
    /// 孤儿豁免规则的条数 —— **不计入** `total_declarations`（它是判定策略，非期望对象）。
    ///
    /// `2` = ① 向量族前缀规则 ② `axagent_schema_version` 精确规则（基础设施表）。
    const EXPECT_ORPHAN_EXEMPT_RULES: usize = 2;
    /// 「非主库实体」声明条数 —— 同样**不计入** `total_declarations`（判定策略，不产生期望对象）。
    ///
    /// `2` = ① `index.db` 族（`ast_*` + `file_index`）② `l2_cache.db` 族（`l2_*`）。
    const EXPECT_NON_MAIN_DB_RULES: usize = 2;
    /// `NON_MAIN_DB` 覆盖的表数（细则见 `crates/entities/src/lib.rs:253-278`）。
    const EXPECT_NON_MAIN_DB_TABLES: usize = 8;
    /// 表达式默认值声明 —— 不计入 `total_declarations`（它是**既有列的一个属性**，
    /// 不新增任何对象）。
    const EXPECT_EXPR_DEFAULT_RULES: usize = 1;
    /// 表达式默认值覆盖的列数（8 = artifacts.updated_at 等，全部是 `*_at` 文本时间列）。
    const EXPECT_EXPR_DEFAULT_COLS: usize = 8;
    const EXPECT_FUNCTIONS: usize = 1;
    const EXPECT_CHECKS: usize = 3;
    const EXPECT_GIN: usize = 6;
    const EXPECT_MULTI: usize = 35;
    /// partial 索引条目数。
    ///
    /// ⚠ 6 条里**前 5 条全部有迁移出处**（v100 / v111 / v112 / v117）；第 6 条
    /// `uq_trajectory_patterns_name` 是**声明式新增**（无迁移），出处写本项的缺陷编号 C-#2。
    /// 它是本清单**唯一一条 `unique: true`** 的 partial；`dialect` 分布是
    /// **5 条 `None`（双方言）+ 1 条 `Some(Postgres)`** —— 唯一的 PG-only 是
    /// `idx_retrieval_hits_feedback`（`v111` 的模板给 PG 分支加了 WHERE、SQLite 分支却是
    /// **普通索引**，两侧不同形，故一条声明表达不了）。第 6 条刻意写 `None`，理由见该条注释。
    const EXPECT_PARTIAL: usize = 6;

    #[test]
    fn l2_item_counts_are_frozen() {
        assert_eq!(VIRTUAL_TABLES.len(), EXPECT_VIRTUAL_TABLES, "FTS5 虚表条目数变了");
        assert_eq!(TRIGGERS.len(), EXPECT_TRIGGERS, "FTS 触发器条目数变了");
        assert_eq!(GENERATED_COLUMNS.len(), EXPECT_GENERATED_COLUMNS, "生成列条目数变了");
        assert_eq!(PATTERN_TSV.len(), EXPECT_PATTERN_TSV, "动态 tsvector 模式条目数变了");
        assert_eq!(
            ORPHAN_EXEMPT.len(),
            EXPECT_ORPHAN_EXEMPT_RULES,
            "孤儿豁免规则条数变了（增删规则必须显式改本断言）"
        );
        assert_eq!(
            NON_MAIN_DB.len(),
            EXPECT_NON_MAIN_DB_RULES,
            "非主库声明条数变了（增删规则必须显式改本断言）"
        );
        assert_eq!(
            NON_MAIN_DB.iter().map(|d| d.tables.len()).sum::<usize>(),
            EXPECT_NON_MAIN_DB_TABLES,
            "非主库声明覆盖的表数变了 —— 少一张即那张表会被引擎在主库建出来"
        );
        assert_eq!(
            EXPR_DEFAULTS.iter().map(|d| d.cols.len()).sum::<usize>(),
            EXPECT_EXPR_DEFAULT_COLS,
            "表达式默认值覆盖的列数变了 —— 少一列即那一列会被 DROP DEFAULT"
        );
        assert_eq!(EXPR_DEFAULTS.len(), EXPECT_EXPR_DEFAULT_RULES, "表达式默认值声明条数变了");
        assert_eq!(FUNCTIONS.len(), EXPECT_FUNCTIONS, "函数条目数变了");
        assert_eq!(CHECK_CONSTRAINTS.len(), EXPECT_CHECKS, "CHECK 约束条目数变了");
        assert_eq!(GIN_INDEXES.len(), EXPECT_GIN, "GIN 索引条目数变了");
        assert_eq!(MULTI_COL_INDEXES.len(), EXPECT_MULTI, "多列索引条目数变了");
        assert_eq!(PARTIAL_INDEXES.len(), EXPECT_PARTIAL, "partial 索引条目数变了");
    }

    #[test]
    fn l2_total_is_sixty_nine() {
        let expect = EXPECT_VIRTUAL_TABLES
            + EXPECT_TRIGGERS
            + EXPECT_GENERATED_COLUMNS
            + EXPECT_PATTERN_TSV
            + EXPECT_FUNCTIONS
            + EXPECT_CHECKS
            + EXPECT_GIN
            + EXPECT_MULTI
            + EXPECT_PARTIAL;
        assert_eq!(expect, 69, "冻结总数常量自相矛盾");
        assert_eq!(total_declarations(), expect, "total_declarations() 与各类求和不符");
    }

    /// 孤儿豁免规则的**形态约束** —— 防止「条目数对但内容错」。
    ///
    /// 两种形态各有各的最危险退化：
    ///
    /// - `Prefix`：**清空 `exclude_suffixes`** ⇒ 等于把「`vec_` 前缀即豁免」写实，
    ///   `_dup_backup` 之类的一次性残留会被永久留下且再也不会被发现（判据 **#381**）。
    /// - `Exact`：**名单为空**或**理由没写 owner** ⇒ 前者是条死规则（恒不命中，看着
    ///   像配了豁免其实没有），后者让「为什么这张表不被删」失去唯一依据。
    ///
    /// 豁免是**必须逐条对账**的机制，不允许用模糊匹配换省事。
    #[test]
    fn orphan_exempt_rules_keep_explicit_exclusions() {
        for d in ORPHAN_EXEMPT {
            assert!(d.reason.len() >= 10, "豁免理由必须写清楚 —— 它会进 plan advisories");
            match &d.scope {
                OrphanExemptScope::Prefix { prefix, exclude_suffixes } => {
                    assert!(!prefix.is_empty(), "豁免规则的前缀不能为空（等于豁免全库）");
                    assert!(
                        !exclude_suffixes.is_empty(),
                        "前缀 `{prefix}` 的豁免规则清空了排除后缀 —— 垃圾残留会被永久豁免"
                    );
                    assert!(
                        !exclude_suffixes.iter().any(|s| s.is_empty()),
                        "前缀 `{prefix}` 的排除后缀含空串（`ends_with(\"\")` 恒真 ⇒ 该规则恒不命中，豁免静默失效）"
                    );
                },
                OrphanExemptScope::Exact(names) => {
                    assert!(!names.is_empty(), "精确豁免名单不能为空（那是一条死规则）");
                    assert!(
                        !names.iter().any(|n| n.is_empty()),
                        "精确豁免名单含空表名（`contains(&\"\")` 恒不命中，规则静默失效）"
                    );
                    assert!(
                        d.reason.contains("owner"),
                        "精确豁免必须写明 owner —— 豁免设施表的唯一依据是「它不归引擎管」：{}",
                        d.reason
                    );
                },
            }
        }
    }

    /// **无实体表上的索引不进 L2**（处置随表本身，见 PLAN §五·一 B 组）。
    ///
    /// 这条是 P3 抓出来的实例：`idx_chat_run_events_run_id` 在 P2 冻结时照搬了迁移
    /// 文本，但 `chat_run_events` 没有实体 ⇒ 它整张表要交孤儿判定 DROP。保留声明会让
    /// 同一轮 plan 里同时出现 `DROP TABLE chat_run_events` 与
    /// `CREATE INDEX idx_chat_run_events_run_id`（后者必然执行失败）。
    ///
    /// 抓出它的是 [`crate::reconcile::expected::build`] 的硬错误（L2 静态声明认不到
    /// 表即报错），而不是任何静态扫描 —— 故本测试只锁**已裁掉的这一条**，通用规则由
    /// `expected.rs::tests::l2_static_decls_all_resolve_to_entity_tables` 兜住。
    #[test]
    fn indexes_on_entity_less_tables_are_not_declared() {
        let all: Vec<&str> = all_indexes().map(|d| d.name).collect();
        // chat_run_events 无实体（v207 建表，PLAN §五·一 B 组）
        //
        // ⚠ 原先是 `for dead in [ … ]` 的表驱动形态，但清单只有一条 ⇒ clippy
        // `single_element_loop` 报红（`-D warnings` 下是硬门禁）。将来若有多条同类，
        // **改回表驱动循环**，别继续堆平铺断言。
        let dead = "idx_chat_run_events_run_id";
        assert!(!all.contains(&dead), "{dead} 建在无实体表上，该表由孤儿判定处置，其索引不得进 L2");
    }

    /// 索引类的**形态约束** —— 防止「条目数对但内容错」。
    #[test]
    fn index_decls_have_matching_shape() {
        for d in GIN_INDEXES {
            assert_eq!(d.method, Some("gin"), "{} 在 GIN 清单里却没有 gin 访问方法", d.name);
            assert_eq!(d.dialect, Some(Dialect::Postgres), "{} GIN 仅 PG 支持", d.name);
            assert!(d.where_clause.is_none(), "{} 是 GIN 清单项，不应带 WHERE", d.name);
        }
        for d in MULTI_COL_INDEXES {
            assert!(
                d.cols.len() >= 2,
                "{} 落在多列清单却只有 {} 列（解析锚点可能取错）",
                d.name,
                d.cols.len()
            );
            assert!(d.method.is_none(), "{} 多列索引不应指定访问方法", d.name);
            assert!(d.where_clause.is_none(), "{} 多列索引不应带 WHERE", d.name);
        }
        for d in PARTIAL_INDEXES {
            assert!(d.where_clause.is_some(), "{} 落在 partial 清单却没有 WHERE", d.name);
            assert_eq!(
                d.cols.iter().filter(|c| c.contains(' ')).count(),
                0,
                "{} 的 cols 混入了谓词（解析锚点取错：应取 USING <method> ( 后的括号对）",
                d.name
            );
        }
    }

    /// 迁移链上**已被显式 DROP** 的对象不得回流。
    ///
    /// 每条都曾出现在「迁移静态扫描」的清单里，照搬会让引擎重建淘汰对象。
    #[test]
    fn dropped_objects_do_not_reappear() {
        let all: Vec<&str> = all_indexes().map(|d| d.name).collect();
        for dead in [
            "idx_traj_memories_tsv",       // v101:389 DROP
            "idx_opc_demand_leads_dedupe", // v138:51 DROP
        ] {
            assert!(!all.contains(&dead), "{} 已被后续迁移 DROP，不应出现在 L2 清单", dead);
        }
        assert!(
            !VIRTUAL_TABLES.iter().any(|v| v.name == "trajectory_memories_fts"),
            "trajectory_memories_fts 已随表 DROP（v101:372）"
        );
        assert!(
            !GENERATED_COLUMNS.iter().any(|c| c.table == "trajectory_memories"),
            "trajectory_memories 表已被 v101:396 DROP，其生成列无从建立"
        );
    }

    /// 两处最容易漏的高危对象必须在内 —— 它们是「漏一条 = 上线后结构缺损」的实例。
    #[test]
    fn high_risk_items_are_present() {
        // notes.tsv: 48590 行；漏了会连带 DROP idx_notes_tsv ⇒ CJK 检索整体失效
        assert!(
            GENERATED_COLUMNS.iter().any(|c| c.table == "notes" && c.col == "tsv"),
            "notes.tsv 是 CJK 全文检索的载体，必须在 L2 清单里"
        );
        // v111 的 partial 索引：迁移静态扫描因 `ON {}(...)` 模板解析失败而漏计
        assert!(
            PARTIAL_INDEXES.iter().any(|d| d.name == "idx_retrieval_hits_feedback"),
            "idx_retrieval_hits_feedback 带 WHERE，实体表达不了，必须在 L2 清单里"
        );
        // v227 动态表名：不在 L2 里 ⇒ 每新建向量集合都缺 content_tsv
        assert!(
            PATTERN_TSV.iter().any(|p| p.table_suffix == "_meta"),
            "vec_*_meta 的 content_tsv 是动态表名，必须有模式声明"
        );
    }

    /// `ax_cjk_ngram` 必须可渲染（否则生成列表达式引用不到函数，建列直接报错）。
    ///
    /// 只断言「建函数语句 + 函数名 + 两个占位符都已替换 + 基本区范围片段在」，
    /// **不逐范围精确比对**：字符类定义会随 Unicode 范围调整而变，逐范围比对会让
    /// 本测试退化成「改范围就要改测试」的负担，进而被后来者顺手删掉。
    /// 真正需要「两侧逐范围一致」的约束由 `crates/search` 的
    /// `tests/ngram_consistency.rs` 与 `scripts/check-ngram-consistency.mjs` 承担。
    #[test]
    fn cjk_ngram_renderer_is_wired() {
        let sql = (FUNCTIONS[0].render)();
        assert!(sql.contains("CREATE OR REPLACE FUNCTION"), "渲染结果不是建函数语句");
        assert!(sql.contains("ax_cjk_ngram"), "渲染结果不含函数名");
        assert!(!sql.contains("__CJK__"), "占位符 __CJK__ 未被替换");
        assert!(!sql.contains("__SEP__"), "占位符 __SEP__ 未被替换");
        assert!(sql.contains("\\u4E00-\\u9FFF"), "渲染结果缺 CJK 基本区范围片段");
    }

    /// 派生索引名与 `v227:216` 逐字一致（`idx_{表名}_tsv`）。
    #[test]
    fn vec_meta_index_name_matches_v227() {
        assert_eq!(vec_meta_index_name("vec_capabilities_meta"), "idx_vec_capabilities_meta_tsv");
        assert_eq!(
            vec_meta_index_name("vec_kb_f9b2b050_e336_4de8_b305_0b40cbf5740f_meta"),
            "idx_vec_kb_f9b2b050_e336_4de8_b305_0b40cbf5740f_meta_tsv"
        );
    }

    /// `expr` 与 `source_columns` 是**派生关系**（改列名就要改表达式），此处锁死同步。
    ///
    /// 这条挡的是「加了源列但忘了改 expr」—— 后果是索引只覆盖一半内容，
    /// 且**不报错**（BM25 分支静默漏召回）。
    #[test]
    fn pattern_tsv_expr_covers_source_columns() {
        for d in PATTERN_TSV {
            assert!(!d.source_columns.is_empty(), "抽词源列不能为空（表达式会退化成恒空串）");
            for c in d.source_columns {
                assert!(
                    d.expr.contains(&format!("COALESCE({c}, '')")),
                    "`{}` 的 expr 未覆盖源列 `{c}`：{}",
                    d.column,
                    d.expr
                );
            }
            // 与 `GeneratedColDecl.expr` 同腔调：`to_tsvector('simple', ax_cjk_ngram(…))`
            assert!(d.expr.starts_with("to_tsvector('simple', ax_cjk_ngram("), "{}", d.expr);
            assert!(d.expr.ends_with("))"), "{}", d.expr);
        }
    }

    /// P4 的渲染入口：两个模型必须真能构造出来，且形态与 `v227` 的 DDL 对齐。
    ///
    /// 这是「声明足以渲染」的可执行判据 —— P3 期间该缺口不可见（只打印不渲染）。
    #[test]
    fn pattern_tsv_models_are_renderable() {
        let d = &PATTERN_TSV[0];
        let col = pattern_tsv_column_model(d);
        assert_eq!(col.name, "content_tsv");
        assert_eq!(col.sql_type, "tsvector");
        assert!(col.nullable, "v227:153 的 DDL 未写 NOT NULL，两侧口径必须一致");
        assert_eq!(col.generated.as_deref(), Some(d.expr), "生成列表达式必须是声明的那一条");
        assert!(
            col.default.is_none() && !col.primary_key && !col.unique && col.renamed_from.is_none()
        );

        let idx = pattern_tsv_index_model(d, "vec_capabilities_meta");
        assert_eq!(idx.name, "idx_vec_capabilities_meta_tsv");
        assert_eq!(idx.method.as_deref(), Some("gin"));
        assert!(!idx.unique && idx.where_clause.is_none());
        assert_eq!(idx.cols, vec![col.name.clone()], "GIN 必须建在那一个生成列上");
    }

    /// 非主库声明的**形态约束** —— 与 [`orphan_exempt_rules_keep_explicit_exclusions`] 同腔调。
    ///
    /// 最危险的三种退化：
    ///
    /// 1. **名单为空 / 含空串** ⇒ 死规则：看着配了「这 8 张表不在主库」，实际一张也不命中，
    ///    引擎照旧在主库把它们建出来（本轮修的就是这个症状，见 [`NON_MAIN_DB`] 文档）；
    /// 2. **不写 owner** ⇒ 「为什么这些表被排除」失去唯一依据，下一个人只能猜；
    /// 3. **`dialect` 写成 `Some(..)`** ⇒ 只有**一条**方言路径被判排除（2026-09-17 之前正是
    ///    `Some(Dialect::Postgres)`）。而主库方言**由用户在设置中设定** ⇒ 另一条路径上引擎
    ///    会把这 8 张侧车库表建进主库 —— 与 2026-09-16 在 PG 上发生的事故**同形**，且**静默**
    ///    （无报错、只是一批永久空表）。⇒ 必须**双方言都**排除（`None` = 既不建也不删）。
    ///    近邻名字必须逐字相等这条边界，在下面的第二段锁住。
    #[test]
    fn non_main_db_rules_name_their_owner_and_are_dialect_agnostic() {
        for d in NON_MAIN_DB {
            assert!(!d.tables.is_empty(), "非主库声明不能是空名单（那是一条死规则）");
            assert!(
                !d.tables.iter().any(|t| t.is_empty()),
                "非主库名单含空表名（`contains(&\"\")` 恒不命中，规则静默失效）"
            );
            assert!(d.reason.contains("owner"), "非主库声明必须写明 owner：{}", d.reason);
            assert!(
                d.dialect.is_none(),
                "非主库声明必须**双方言都**排除，实际 dialect={:?} —— 主库方言由用户可配，\
                 写成 Some(..) 会让另一条路径上引擎把这些侧车库表建进主库（它们的家在 \
                 `index.db` / `l2_cache.db` 两个独立 SQLite 文件里，且那两个库的 schema \
                 由 owner crate 自建，不需要本引擎代劳）",
                d.dialect
            );
        }
        // 近邻名字不得误命中：豁免/排除类规则一旦过宽，症状是**静默**的（该建的不建、
        // 或该管的不管），所以逐字比而不是前缀比，并在这里把边界钉住。
        for near in [
            "file_index_backup",
            "file_indexes",
            "ast_class",        // 少一个 s
            "ast_classes_old",  // 多一个后缀
            "l2_search_result", // 少一个 s
            "l2_index_snapshot",
        ] {
            assert!(
                non_main_db_decl_for(near).is_none(),
                "`{near}` 是近邻名字，不该命中非主库声明（判据必须是逐字相等）"
            );
        }
    }

    /// 表达式默认值声明的**形态约束**。
    #[test]
    fn expr_default_rules_are_well_formed() {
        for d in EXPR_DEFAULTS {
            assert!(!d.cols.is_empty(), "表达式默认值声明不能是空列表（那是一条死规则）");
            assert!(
                !d.cols.iter().any(|(t, c)| t.is_empty() || c.is_empty()),
                "表达式默认值声明含空表名/列名（恒不命中，规则静默失效）"
            );
            assert_eq!(
                d.sql,
                d.sql.trim(),
                "SQL 文本首尾不得有空白 —— 它会与 `pg_get_expr` 输出逐字比对"
            );
            assert!(!d.sql.is_empty(), "SQL 文本不能为空（`SET DEFAULT ` 空尾是建模矛盾）");
            assert!(d.reason.len() >= 10, "表达式默认值声明必须写明理由");
            assert_eq!(
                d.dialect,
                Some(Dialect::Postgres),
                "该表达式是 PG 专有的 `to_char(...)`；写成双方言通用会让 SQLite 期望集带上一条它没有的默认值"
            );
        }
    }
}
