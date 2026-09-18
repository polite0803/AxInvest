// SPDX-License-Identifier: AGPL-3.0-only

//! **期望侧**：L1（SeaORM 实体）+ L2（`extras.rs` 声明） → [`SchemaModel`]。
//!
//! 与 [`super::introspect`]（实况侧）严格对偶：`expected::build(dialect)` 产出 B，
//! `introspect::read(db)` 产出实际结构，两者归一后之差就是 plan 的输入。
//!
//! ## 唯一入口是「委托 sea-orm 自己算」，不扫源文本
//!
//! sea-orm 的 `ColumnDef`（`sea-orm-2.0.2/src/entity/column_def.rs:16`）**全部字段
//! 都是 `pub(crate)`，且只有 4 个 getter**（`get_column_type` / `get_column_default` /
//! `is_null` / `is_unique`）。决定索引名的 `indexed` / `unique_key` 与决定改名的
//! `renamed_from` **没有 getter**，从 crate 外读不到。
//!
//! ⇒ 这里**不**去扫实体源文本重新解析宏属性（PLAN §四·一 明文禁止：`EntityTrait`
//! 是运行时 trait，源文本解析必然漂移），而是调用 sea-orm 自己的公开实现：
//!
//! | 要取什么 | 调用谁 | 为什么这是安全的 |
//! |---|---|---|
//! | 列集 / 类型 / NOT NULL / UNIQUE / FK | `Schema::create_table_from_entity` | 与 P4 渲染用的是**同一个函数** ⇒ 期望值与被渲染的 DDL 天然一致 |
//! | 索引名 / 列集 / unique | `Schema::create_index_from_entity` | 索引名硬编码于 `sea-orm-2.0.2/src/schema/entity.rs:158`，复用才不漂移 |
//! | nullable / default / unique / primary_key / generated | `sea_query::ColumnSpec` | `ColumnSpec` 字段**全 pub**（`sea-query-1.0.2/src/table/column.rs:195`）⇒ 唯一公开可读通道 |
//! | `renamed_from` | 列注释回捞（sea-orm 把 `renamed_from` 折进 comment） | 无 getter；折法由本文件测试逐字锁死 |
//!
//! ## L2 叠加的硬要求：**认不到表就报错，不许静默跳过**
//!
//! L2 静态声明（索引 / CHECK / 生成列）指向一张实体未覆盖的表时，说明「引擎看不见
//! 这张表」—— 那正是 P0a 事故的形态（该表会被判孤儿并 DROP）。故这些叠加全部走
//! `ok_or_else`：声明与实体集不一致时 [`build`] 直接失败，而不是悄悄少一条。
//!
//! 例外是 [`extras::PatternTsvDecl`]：它按**表名模式**匹配，覆盖的正是「运行时按
//! 集合创建」的 `vec_*_meta` —— 那些表**不在实体集里**（实测 3 张，见
//! `output/tmp-p3-tsv-probe.log`），且它们**只在 `introspect` 之后才被看见**。
//! 故模式声明导出给 [`super::plan`] 在 plan 阶段对「实况表名」应用（见
//! [`pattern_tsv_decls`]），期望侧这里只把声明本身记进指纹。//!
//! ## 实测口径（`output/tmp-p3-tsv-probe.log`，2026-09-16）
//!
//! PG 全库 9 条 `tsvector` 列**全是生成列且全可空**（`attnotnull=false`、
//! `attgenerated='s'`）：6 条来自 `GENERATED_COLUMNS`，3 条来自 `PATTERN_TSV`。
//! 且**没有任何一张表的实体声明了该列** ⇒ L2 必须能**新建**列，而不只是修改已有列。

use sea_orm::{
    // ⚠ `EntityName` 必须显式导入，尽管 `table_from_entity` 里 `entity.table_name()` 看起来
    // 不需要它（2026-09-16 实测踩到，代价 183 条 `E0599`）：泛型参数 `E: EntityTrait` 上
    // 的方法解析会**顺带查 supertrait**，而 `EntityName` 正是 `EntityTrait` 的 supertrait；
    // 但 [`expected_table_count`] 的宏是在**具体类型**上调用的，那一路径**只认作用域内的
    // trait** ⇒ 不导入就整片报 `no method named table_name`。两者定义在同一文件
    // （`sea-orm-2.0.2/src/entity/base_entity.rs`）⇒ 导入路径与 `EntityTrait` 完全同源。
    DbBackend,
    EntityName,
    EntityTrait,
    IdenStatic,
    Iterable,
    PrimaryKeyToColumn,
    Schema,
    sea_query::{
        Alias, ColumnDef, ColumnType, Expr, PostgresQueryBuilder, SqliteQueryBuilder,
        TableCreateStatement, TableRef,
    },
};

use super::{
    extras::{self, Dialect},
    model::{CheckModel, ColumnModel, FkModel, IndexModel, SchemaModel, TableModel},
};

// 实体清单的**唯一权威**是 `crates/entities/src/lib.rs` 的 `pub mod` 声明，
// 由 `crates/dao/build.rs` 在编译期读盘生成 —— 新增实体零登记成本。
include!(concat!(env!("OUT_DIR"), "/entity_registry.rs"));

/// sea-orm 把 `renamed_from` 折进列注释时的前缀（`sea-orm-2.0.2/src/schema/entity.rs:270-278`）：
///
/// ```text
/// (Some(renamed_from), Some(comment)) => "{comment}; renamed_from \"{old}\""
/// (Some(renamed_from), None)          => "renamed_from \"{old}\""
/// ```
///
/// 这是**唯一**能拿到 `renamed_from` 的通道。两条形态都由
/// `renamed_from_is_recovered_from_column_comment` 测试逐字锁定 —— sea-orm 改了折法
/// 测试立刻红，而不是引擎静默退化成「DROP 旧列 + ADD 新列」。
const RENAMED_FROM_PREFIX: &str = "renamed_from \"";

/// 期望侧入口：L1 全实体 + L2 全声明 → 已 `finalize()` 的 [`SchemaModel`]。
///
/// 纯计算、无 IO、无数据库连接 —— 便于单测直接对账。
pub fn build(dialect: Dialect) -> Result<SchemaModel, sea_orm::DbErr> {
    let schema = Schema::new(backend_of(dialect));
    let mut model = SchemaModel::new(dialect);

    macro_rules! add_entities {
        ($($module:ident),* $(,)?) => {
            $(
                let t = table_from_entity::<axagent_entities::$module::Entity>(&schema, dialect);
                // 「非主库实体」不进本方言的期望集 —— 见 `extras::NON_MAIN_DB`。
                //
                // ⚠ 为什么必须在这里排除而不是让它落进孤儿判定：`entity_registry!` 由
                // `build.rs` **按行前缀扫描 `pub mod`** 生成、不认注释，而实体文件里明明
                // 写着「不在主库」。若只从期望集移除、不做别的，引擎就会把一批**名字属于
                // 别的子系统**的表判成孤儿并 `DROP` —— 那不是「清理」，是越权。
                if !non_main_db_applies(&t.name, dialect) {
                    model.upsert_table(t);
                }
            )*
        };
    }
    entity_modules!(add_entities);

    overlay_l2(&mut model, dialect)?;

    model.finalize();
    Ok(model)
}

/// 注册表中的实体数 —— **按方言**，与 [`build`] 产出的表数逐一对齐。
///
/// 单独暴露是为了让上层做「期望侧为空」的运行期守卫：空期望 + 非空库 = 整库判孤儿。
///
/// ⚠ 为什么必须带 `dialect` 参数（2026-09-16）：加入「非主库实体」声明后，注册表里
/// 的实体数与本方言期望表数**不再相等**（PG 少 8 张侧车库表）。若这个函数仍返回
/// 注册表原始条数，它会同时错在两个地方：① 与 [`build`] 对不上（测试会立刻红）；
/// ② 更危险的是基数熔断 —— `apply` 拿它当「本轮期望基数」，一个偏大的值会让
/// 「期望集塌陷」这条闸**永远差 8 张才报**。
pub fn expected_table_count(dialect: Dialect) -> usize {
    let mut n = 0usize;
    macro_rules! count_entities {
        ($($module:ident),* $(,)?) => {
            $(
                if !non_main_db_applies(
                    axagent_entities::$module::Entity::default().table_name(),
                    dialect,
                ) {
                    n += 1;
                }
            )*
        };
    }
    entity_modules!(count_entities);
    n
}

/// L2 `PATTERN_TSV` 的只读出口 —— 供 [`super::plan`] 对**实况表名**应用。
///
/// 为什么不在本模块应用：模式匹配的是运行时按向量集合创建的表，它们**不在实体集里**，
/// 只有 `introspect` 之后才知道名字。期望侧凭空枚举不出来。
pub fn pattern_tsv_decls() -> impl Iterator<Item = &'static extras::PatternTsvDecl> {
    extras::PATTERN_TSV.iter()
}

/// 表名是否匹配某个 `PATTERN_TSV` 声明（引擎与 introspect 两侧共用同一判定）。
pub fn pattern_tsv_applies(d: &extras::PatternTsvDecl, table: &str, dialect: Dialect) -> bool {
    l2_applies(d.dialect, dialect)
        && table.starts_with(d.table_prefix)
        && table.ends_with(d.table_suffix)
}

/// L2 `ORPHAN_EXEMPT` 的只读出口 —— 与 [`pattern_tsv_decls`] 同构。
///
/// 分开暴露的原因见 [`extras::OrphanExemptDecl`]：豁免是**判定策略**，不是期望对象。
pub fn orphan_exempt_decls() -> impl Iterator<Item = &'static extras::OrphanExemptDecl> {
    extras::ORPHAN_EXEMPT.iter()
}

/// 表名是否命中某条孤儿豁免规则（命中 ⇒ 不参与孤儿判定，不会被 `DROP TABLE`）。
///
/// 两种形态的匹配分开写在 [`extras::OrphanExemptScope`] 上 —— 不在这里做「前缀为空即按
/// 精确名比」之类的塌缩：那会让一条写错前缀的规则静默退化成「什么都不命中」，而豁免
/// 静默失效的症状是**数据被删**。
pub fn orphan_exempt_applies(d: &extras::OrphanExemptDecl, table: &str, dialect: Dialect) -> bool {
    if !l2_applies(d.dialect, dialect) {
        return false;
    }
    match &d.scope {
        extras::OrphanExemptScope::Prefix { prefix, exclude_suffixes } => {
            table.starts_with(prefix) && !exclude_suffixes.iter().any(|s| table.ends_with(s))
        },
        extras::OrphanExemptScope::Exact(names) => names.contains(&table),
    }
}

/// 表是否命中「非主库实体」声明（命中 ⇒ 本方言下**既不进期望集，也不判孤儿**）。
///
/// 两个消费点共用这一条判据：本模块的 [`build`]（排除出期望集）与
/// `plan.rs::diff_with`（并入 `orphan_exempt`）。**故意只留一个入口** —— 若两边各自
/// 实现一遍，一边改了另一边没改的症状是「不建也不删」退化成「不建但删」，即引擎
/// 反过来把别的子系统的表清掉（判据 **#381**：声称范围必须与落实范围逐对象对账）。
pub fn non_main_db_applies(table: &str, dialect: Dialect) -> bool {
    non_main_db_reason(table, dialect).is_some()
}

/// 命中「非主库实体」声明时的**理由**（`None` = 没命中）。
///
/// 与 [`non_main_db_applies`] 是同一个判据的两种视图：`plan.rs` 要理由串写 advisory，
/// `build` 只要布尔。理由里必须带 owner，否则「这些表为什么被排除」无从追溯。
pub fn non_main_db_reason(table: &str, dialect: Dialect) -> Option<&'static str> {
    let d = extras::non_main_db_decl_for(table)?;
    l2_applies(d.dialect, dialect).then_some(d.reason)
}

/// 表名是否命中 L2 声明的 **FTS5 虚表**（命中 ⇒ 不判孤儿）；命中时返回写进 advisory 的理由。
///
/// ## 为什么需要第三个「不判孤儿」来源
///
/// L2 的 [`extras::VIRTUAL_TABLES`] 声明在 [`build`] 的 ④ 段里**只进指纹**
/// （`claim_extra("l2.fts5:…")`），它**不产生 `TableDecl`** ⇒ 期望集里没有这些名字。
/// 而实况侧相反：`introspect` 的 `read_table` 对虚表**明确建表**
/// （`introspect/sqlite.rs` 的 `m.table_or_insert(&raw.name)` + `claim_extra("virtual_table:…")`）。
///
/// 两侧不对称的后果是 `plan` 的表级判定认为「实况有、期望无」= 孤儿：
///
/// ```text
/// !! 段1 DROP TABLE  messages_fts  实况有表、L1+L2 均无声明；列 0 个
/// ```
///
/// 实测（2026-09-17，`p3_plan_probe` 对生产库副本）：4 张**在 L2 里声明过、且在库中
/// 存在**的虚表全部落进「仅实况 ⇒ 将被 DROP」；其中 `messages_fts` 上挂着三条同步
/// 触发器 —— 计划一旦执行，该库的全文检索当场失效。
///
/// ## ⚠ 判据必须是「声明名精确匹配」，不能是后缀 `_fts` 通配
///
/// [`extras::VIRTUAL_TABLES`] **刻意没有**收录 `trajectory_memories_fts`
/// （`v101:372` 已随表把它 DROP），而它在存量库里**仍然存在** —— 它是**真残留、
/// 必须继续判孤儿**。后缀通配会把它一并豁免 ⇒ 残留永久驻留。
/// 判据 K 组：豁免范围必须与声明**逐对象**对账，而不是按名字形态外推。
pub fn l2_virtual_table_reason(name: &str, dialect: Dialect) -> Option<&'static str> {
    extras::VIRTUAL_TABLES.iter().find(|d| d.name == name && l2_applies(d.dialect, dialect)).map(
        |_| {
            "extras::VIRTUAL_TABLES 声明（SQLite FTS5 全文索引虚表；建表原文与同步触发器都在 L2，\
             期望侧只进指纹不建表 ⇒ 引擎既不建它、也不得删它）"
        },
    )
}

/// `(table, col)` 在本方言下的**表达式型默认值**（L2 声明）。
///
/// 只在 L1 没给出默认值时由 [`table_from_entity`] 取用 —— L1 是真相源。
pub fn expr_default_sql(table: &str, col: &str, dialect: Dialect) -> Option<&'static str> {
    let d = extras::expr_default_decl_for(table, col)?;
    l2_applies(d.dialect, dialect).then_some(d.sql)
}

fn backend_of(dialect: Dialect) -> DbBackend {
    match dialect {
        Dialect::Postgres => DbBackend::Postgres,
        Dialect::Sqlite => DbBackend::Sqlite,
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// L1：单个实体 → TableModel
// ═══════════════════════════════════════════════════════════════════════════

/// 单个实体的期望结构。
///
/// **分流逻辑必须与 `introspect/pg.rs` / `introspect/sqlite.rs` 严格镜像**，
/// 否则同一份库结构在两侧会落进不同的桶，产出永远修不好的假差异：
///
/// | 对象 | 期望侧（本函数） | 实况侧 |
/// |---|---|---|
/// | 单列主键 | 列标志 `primary_key` | 列在 `primary_key` 集合里 |
/// | 复合主键 | `E::PrimaryKey`，且**丢弃** `pk-{表}` 索引 | pk 索引/约束**跳过** |
/// | 列级 UNIQUE | `spec.unique` → 列标志 | `contype='u'` 单列 → 列标志 |
/// | 多列 UNIQUE | `create_index_from_entity` 的 `idx-{表}-{key}` | `contype='u'` 多列 → 索引 |
/// | 普通索引 | `idx-{表}-{列}` | 非 pk/非 unique 的索引 |
/// | FK | `belongs_to`（`!is_owner && !skip_fk`） | `pg_constraint` contype='f' |
fn table_from_entity<E>(schema: &Schema, dialect: Dialect) -> TableModel
where
    E: EntityTrait,
{
    let entity = E::default();
    let mut t = TableModel::new(entity.table_name());

    // 复合主键只在这里出现一次；`create_table_from_entity` 会为 ARITY>1 另生成
    // `pk-{表}` 索引，那是主键的另一种渲染而非独立索引 ⇒ 下面丢弃。
    t.primary_key = E::PrimaryKey::iter().map(|pk| pk.into_column().as_str().to_string()).collect();

    let table_stmt = schema.create_table_from_entity(entity);

    for col in table_stmt.get_columns() {
        let spec = col.get_column_spec();
        let raw_type = col.get_column_type().expect("create_table_from_entity 必给出列类型");
        let mut c = ColumnModel::new(
            col.get_column_name(),
            super::model::canonical_type_expected(raw_type, dialect),
            // `nullable: None` ⇒ 实体没写 NOT NULL ⇒ 可空
            spec.nullable.unwrap_or(true),
        );
        c.primary_key = spec.primary_key;
        c.unique = spec.unique;
        // ⚠ `auto_increment` 必须在 `default` **之前或同时**取：`create_table_from_entity`
        // 把自增表达为 `ColumnDef::auto_increment()` 而**不写 `spec.default`**，所以自增列
        // 在期望侧天生 `default = None`；若不单独记下这个事实，实况侧的
        // `DEFAULT nextval('…_seq')` 会被判成「多出来的默认值」⇒ `DROP DEFAULT`（实测 9 条）。
        // 详见 `ColumnModel::auto_increment` 的字段文档。
        c.auto_increment = spec.auto_increment;
        c.default = spec.default.as_ref().map(|e| render_expr(e, dialect));
        // L2 补「表达式型默认值」—— 见 `extras::EXPR_DEFAULTS`。
        //
        // L1 在**语法上**表达不了函数调用型默认值：`default_value` 只收 `Into<Value>`
        // 的字面量，`default_expr` 收的是 Rust 表达式（写 `Expr::cust("to_char(...)")`
        // 等于把 PG 专有 SQL 塞进方言无关的实体声明）。而「声明里没有」≠「不该存在」——
        // 不补这一手，引擎就会因为自己词汇不足而给 8 列发 `DROP DEFAULT`，把一条与
        // `now_datetime_str()` 同格式的安全网删掉。
        //
        // **只在 L1 没给值时补**：L1 是真相源，L2 只兜它够不着的部分。
        if c.default.is_none() {
            c.default = expr_default_sql(entity.table_name(), &c.name, dialect).map(str::to_string);
        }
        c.generated = spec
            .generated
            .as_ref()
            .map(|g| super::model::normalize_sql_expr(&render_expr(&g.expr, dialect)));
        c.renamed_from = renamed_from_of(spec.comment.as_deref());
        t.upsert_column(c);
    }

    // `create_table_from_entity` 里的索引只有复合主键那一条（`sea-orm-2.0.2/src/schema/entity.rs:219-225`）。
    // 断言而非静默丢弃：哪天它开始顺带产出真实索引，这里要立刻发现。
    for idx in table_stmt.get_indexes() {
        debug_assert!(
            idx.is_primary_key(),
            "create_table_from_entity 只应产出复合主键索引，出现 {:?} 说明 sea-orm 行为变了",
            idx.get_index_spec().get_name()
        );
    }

    // 真实索引走 `create_index_from_entity` —— 与 P4 渲染同一函数，名/列集不会漂移。
    for idx in schema.create_index_from_entity(entity) {
        if idx.is_primary_key() {
            continue;
        }
        let spec = idx.get_index_spec();
        t.upsert_index(IndexModel {
            name: spec.get_name().unwrap_or_default().to_string(),
            cols: spec.get_column_names(),
            unique: idx.is_unique_key(),
            method: None,
            where_clause: None,
        });
    }

    // FK：`create_table_from_entity` 已按 `if relation.is_owner || relation.skip_fk
    // { continue }` 过滤（`sea-orm-2.0.2/src/schema/entity.rs:227-233`），即**只取 `belongs_to`**。
    // ⚠ 方向极易搞反：`EntityTrait::belongs_to` 构造时传 `is_owner = false`
    // （`sea-orm-2.0.2/src/entity/base_entity.rs:105`），`has_one` / `has_many` 才是 `true`
    // （`sea-orm-2.0.2/src/entity/base_entity.rs:115/125`）。FK 挂在 `belongs_to` 一侧。
    for fk_stmt in table_stmt.get_foreign_key_create_stmts() {
        let fk = fk_stmt.get_foreign_key();
        t.fks.push(FkModel {
            name: fk.get_name().map(str::to_string),
            cols: fk.get_columns(),
            ref_table: fk.get_ref_table().map(table_ref_name).unwrap_or_default(),
            ref_cols: fk.get_ref_columns(),
            on_delete: fk.get_on_delete().map(|a| a.variant_name().to_string()),
            on_update: fk.get_on_update().map(|a| a.variant_name().to_string()),
        });
    }

    t
}

/// `TableRef` → 裸表名（不含 schema 限定）。
fn table_ref_name(r: &TableRef) -> String {
    match r {
        TableRef::Table(tbl, _) => {
            debug_assert!(
                tbl.0.is_none(),
                "本项目实体不使用 schema 限定名；出现限定名需重估索引/FK 命名口径"
            );
            tbl.1.to_string()
        },
        other => format!("{other:?}"),
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// 默认值 / 生成列表达式 / renamed_from 的取值通道
// ═══════════════════════════════════════════════════════════════════════════

/// 把 `Expr` 渲染成 SQL 文本。
///
/// **为什么不能直接 `expr.to_string(qb)`**：sea-query 1.0.2 没给 `Expr` 提供公开的
/// 文本渲染入口（`src/expr/enum.rs` 的 `impl Expr` 里没有 `to_string`）。
///
/// 故借一条**最小语句**渲染 —— `CREATE TABLE "_ax_probe" ( "_ax_c" text DEFAULT <expr> )`
/// —— 再切掉第一个 ` DEFAULT ` 之前的部分。取**第一个**：列自己的 DEFAULT 必在最前，
/// 所以表达式内部即使含字符串 `" DEFAULT "` 也不会切错。
///
/// ⚠ 返回的是 **sea-query 的渲染文本**，不是 PG 会报回的文本。PG 会补 `::类型` 与
/// 多余括号 ⇒ 两边文本**必然**不同，故 `default` / `generated` 只作诊断字段：
/// 结构等价只看「有没有」，不看「写了什么」。见 `model.rs` 对应字段文档。
fn render_expr(expr: &Expr, dialect: Dialect) -> String {
    let mut stmt = TableCreateStatement::new();
    stmt.table(Alias::new("_ax_probe"))
        .col(ColumnDef::new_with_type(Alias::new("_ax_c"), ColumnType::Text).default(expr.clone()));
    let sql = match dialect {
        Dialect::Postgres => stmt.to_string(PostgresQueryBuilder),
        Dialect::Sqlite => stmt.to_string(SqliteQueryBuilder),
    };
    match sql.find(" DEFAULT ") {
        Some(i) => {
            let tail = sql[i + " DEFAULT ".len()..].trim_end();
            // 只放了一列 ⇒ 末尾那个 `)` 是列定义的收尾
            tail.strip_suffix(')').unwrap_or(tail).trim().to_string()
        },
        // 没渲染出 DEFAULT ⇒ sea-query 行为变了。返回空串而非 panic：
        // 该字段是诊断用，不参与结构等价判定。
        None => String::new(),
    }
}

/// 从列注释里回捞 `renamed_from`（折法见 [`RENAMED_FROM_PREFIX`] 文档）。
///
/// 注释里出现 `renamed_from "` 只可能来自 sea-orm 的折叠 —— 本项目实体从不写
/// `#[sea_orm(comment = ...)]`（`grep -rn 'comment =' crates/entities/src` 可验）。
fn renamed_from_of(comment: Option<&str>) -> Option<String> {
    let c = comment?;
    let i = c.find(RENAMED_FROM_PREFIX)?;
    let rest = &c[i + RENAMED_FROM_PREFIX.len()..];
    let end = rest.find('"')?;
    Some(rest[..end].to_string())
}

// ═══════════════════════════════════════════════════════════════════════════
// L2：声明叠加
// ═══════════════════════════════════════════════════════════════════════════

fn l2_applies(d: Option<Dialect>, dialect: Dialect) -> bool {
    match d {
        None => true,
        Some(Dialect::Postgres) => dialect == Dialect::Postgres,
        Some(Dialect::Sqlite) => dialect == Dialect::Sqlite,
    }
}

fn err(msg: String) -> sea_orm::DbErr {
    sea_orm::DbErr::Custom(format!("reconcile::expected：{msg}"))
}

fn overlay_l2(model: &mut SchemaModel, dialect: Dialect) -> Result<(), sea_orm::DbErr> {
    // ① 索引（GIN + 多列 + partial 三来源合并）
    for d in extras::all_indexes() {
        if !l2_applies(d.dialect, dialect) {
            continue;
        }
        let t = model
            .table_mut(d.table)
            .ok_or_else(|| err(format!("L2 索引 `{}` 指向实体未覆盖的表 `{}`", d.name, d.table)))?;
        t.upsert_index(IndexModel {
            name: d.name.to_string(),
            cols: d.cols.iter().map(|c| (*c).to_string()).collect(),
            unique: d.unique,
            method: d.method.map(str::to_string),
            where_clause: d.where_clause.map(str::to_string),
        });
        // 认领标记带上**全部声明内容**：L2 改了 name/cols/method 任一项，指纹必须变，
        // 否则「改了 L2 但引擎短路跳过」—— 声明永不生效且无任何报错。
        t.claim_extra(format!(
            "l2.index:{}|{}|{}|{}|{}",
            d.name,
            d.cols.join(","),
            d.unique,
            d.method.unwrap_or(""),
            d.where_clause.unwrap_or("")
        ));
    }

    // ② CHECK 约束
    for d in extras::CHECK_CONSTRAINTS {
        if !l2_applies(d.dialect, dialect) {
            continue;
        }
        let t = model.table_mut(d.table).ok_or_else(|| {
            err(format!("L2 CHECK `{}` 指向实体未覆盖的表 `{}`", d.name, d.table))
        })?;
        t.upsert_check(CheckModel::new(d.name, super::model::normalize_sql_expr(d.expr)));
        t.claim_extra(format!("l2.check:{}|{}", d.name, d.expr));
    }

    // ③ 静态生成列（6 条，实测全是 tsvector 且全可空）
    for d in extras::GENERATED_COLUMNS {
        if !l2_applies(d.dialect, dialect) {
            continue;
        }
        let t = model
            .table_mut(d.table)
            .ok_or_else(|| err(format!("L2 生成列 `{}.{}` 指向实体未覆盖的表", d.table, d.col)))?;
        // 列的实体声明可能不存在（实测：6 张表里一张都没有）⇒ 以 L2 的 col_type 新建。
        // L2 给的是**裸 SQL 类型名**（`"tsvector"`），故走「实况侧归一」入口。
        let mut c = t.column(d.col).cloned().unwrap_or_else(|| {
            ColumnModel::new(d.col, super::model::canonical_type_actual(d.col_type, dialect), true)
        });
        c.sql_type = super::model::canonical_type_actual(d.col_type, dialect);
        c.generated = Some(super::model::normalize_sql_expr(d.expr));
        t.upsert_column(c);
        t.claim_extra(format!("l2.generated:{}|{}|{}", d.col, d.col_type, d.expr));
    }

    // ④ 函数 / 虚表 / 触发器：不挂表，但必须进指纹（改了要触发重跑）。
    //    声明内容一并进 tag —— 只记名字的话，改 SQL 正文不会改指纹。
    for d in extras::FUNCTIONS {
        if l2_applies(d.dialect, dialect) {
            model.claim_extra(format!("l2.fn:{}|{}|{}", d.name, d.args, (d.render)()));
        }
    }
    for d in extras::VIRTUAL_TABLES {
        if l2_applies(d.dialect, dialect) {
            model.claim_extra(format!("l2.fts5:{}|{}", d.name, d.create_sql));
        }
    }
    for d in extras::TRIGGERS {
        if l2_applies(d.dialect, dialect) {
            model.claim_extra(format!("l2.trigger:{}|{}", d.name, d.sql));
        }
    }

    // ⑤ 模式 tsvector：期望侧只记声明本身；对**实况表名**的应用在 plan 阶段
    //    （见 `pub_pattern_tsv` 文档 —— 运行时表的列集只有 introspect 之后才知道）。
    for d in extras::PATTERN_TSV {
        if l2_applies(d.dialect, dialect) {
            model.claim_extra(format!(
                "l2.pattern_tsv:{}|{}|{}|{}",
                d.table_prefix,
                d.table_suffix,
                d.column,
                d.source_columns.join(",")
            ));
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 每个注册实体的去向必须**二选一且恰好一个**：进本方言的期望集，或被
    /// 「非主库实体」声明排除。`expected_table_count` 与 `build()` 产出的表数逐方言一致。
    ///
    /// 不一致 = 有实体被静默丢弃（或两张实体声明了同一个 `table_name`，后者会
    /// `upsert_table` 覆盖前者 ⇒ 少一张表 ⇒ 那个表会被判孤儿）。这是必须暴露的缺陷。
    ///
    /// ⚠ 为什么**两个方言都**断言：非主库声明现在是**双方言通用**（`dialect: None`）。
    /// 但这不等于「随便测一个就行」—— 恰恰相反：只要哪天有人把某条声明的 `dialect` 改回
    /// `Some(..)`，**另一条**方言路径就会把 8 张侧车库表重新建进主库（2026-09-17 那次的
    /// 成因：主库方言由用户可配，`Some(Postgres)` 只在主库 = PG 时成立）。只测一个方言
    /// 看不见这个错。
    #[test]
    fn every_registered_entity_becomes_a_table() {
        for dialect in [Dialect::Postgres, Dialect::Sqlite] {
            let n = expected_table_count(dialect);
            assert!(n > 150, "实体注册表应有 180+ 项，实际 {n}");

            let m = build(dialect).expect("期望侧构建应成功");
            assert_eq!(
                m.tables.len(),
                n,
                "[{dialect:?}] 期望表数 {} ≠ 该方言实体数 {n} ⇒ 存在同名 `table_name` 覆盖，\
                 或「非主库实体」声明的方言作用域不对",
                m.tables.len()
            );
        }

        // 两方言的期望表数必须**相等** —— 这是「双方言都排除」的判定式：那 8 张侧车库表
        // 的家在独立 SQLite 文件里，**不属于任何一条方言的主库** ⇒ 两侧都不该含它们。
        // 一旦某条声明被写回 `Some(..)`，差值就会变成 8，本行红。
        assert_eq!(
            expected_table_count(Dialect::Sqlite),
            expected_table_count(Dialect::Postgres),
            "两方言的期望表数不等（差 {}）⇒ 有「非主库」声明的 `dialect` 不是 `None`，\
             那条路径上引擎会把侧车库表建进主库",
            expected_table_count(Dialect::Sqlite) as i64
                - expected_table_count(Dialect::Postgres) as i64
        );
    }

    /// 「非主库实体」声明的**双向作用**：**两个方言**的期望集都不含它们（不建、不改），
    /// 且两侧的孤儿判定也都放过它们（不删）。
    ///
    /// ⚠ 关键是「**不删**」那一半：`dialect: None` 若只做到「从期望集移除」而漏了孤儿豁免，
    /// 引擎就会去 DROP 一批**名字属于别的子系统**的表 —— 那不是清理，是越权
    /// （判据 **#381**：声称范围必须与落实范围逐对象对账）。两个消费点共用
    /// [`non_main_db_applies`] 这一个入口，本测试把它的两侧取值都钉住。
    ///
    /// 顺带测「声明必须认领到真实实体」—— ⚠ **这一条不能靠 `build()` 测**：被排除的名字
    /// 本来就不在期望集里，所以「抄错表名」在期望集上**看不出任何异常**（排除一个不存在的
    /// 名字与排除一个存在的名字，结果都是 `table(t) == None`）。故这里直接问实体注册表。
    #[test]
    fn non_main_db_entities_are_excluded_from_both_dialects() {
        let pg = build(Dialect::Postgres).expect("PG 期望侧应构建成功");
        let lite = build(Dialect::Sqlite).expect("SQLite 期望侧应构建成功");

        // 注册表的**未过滤**全量 `table_name` 集 —— `expected_table_count` 与 `build` 都
        // 回答不了「这个被排除的名字在不在注册表里」，只有这里能问。
        let mut registered: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
        macro_rules! collect_registered {
            ($($module:ident),* $(,)?) => {
                $(
                    registered.insert(
                        axagent_entities::$module::Entity::default().table_name().to_string(),
                    );
                )*
            };
        }
        entity_modules!(collect_registered);

        for d in extras::NON_MAIN_DB {
            // `for &t in` 而不是 `for t in`（后者 `t: &&str`，写 `*t` 会触发
            // clippy `explicit_auto_deref` —— 本项目启用了该 lint，见 PLAN §⑨ 14）。
            for &t in d.tables {
                assert!(
                    registered.contains(t),
                    "非主库声明里的 `{t}` 不是任何已注册实体的 table_name —— 抄错了名字，\
                     或实体被改名后声明没跟着改 ⇒ 这条声明**静默失效**（判据 #381）"
                );
                assert!(non_main_db_applies(t, Dialect::Postgres), "{t} 在 PG 侧必须被排除");
                assert!(non_main_db_applies(t, Dialect::Sqlite), "{t} 在 SQLite 侧也必须被排除");
                assert!(pg.table(t).is_none(), "PG 期望集不该含侧车库表 {t}");
                assert!(
                    lite.table(t).is_none(),
                    "SQLite 期望集不该含侧车库表 {t} —— 它的家是独立 SQLite 文件，\
                     与主库是不是 SQLite 无关；含它 ⇒ 引擎会在 SQLite 主库里建出一批空壳"
                );
            }
        }

        // 反向边界：普通主库表**不得**被这条声明波及（豁免/排除类规则一旦过宽，
        // 症状是静默的 —— 该建的没建）。这里用一张最普通的业务表做样本，两个方言都查。
        assert!(pg.table("audit_log").is_some(), "普通主库表不得被非主库声明波及");
        assert!(lite.table("audit_log").is_some(), "普通主库表不得被非主库声明波及");
    }

    /// L2 表达式默认值：**PG 侧注入、SQLite 侧不注入**，且每条声明都认领到真实列。
    ///
    /// ⚠ 这条测试同时是「声明与实体对账」的守卫：`pg.table(t)?.column(c)?` 任一步为 `None`
    /// 即说明声明指向了一个不存在的表/列（改列名后最容易腐烂的一环）。
    #[test]
    fn l2_expr_defaults_fill_pg_columns_only() {
        let pg = build(Dialect::Postgres).expect("PG 期望侧应构建成功");
        let lite = build(Dialect::Sqlite).expect("SQLite 期望侧应构建成功");

        for d in extras::EXPR_DEFAULTS {
            for (t, c) in d.cols {
                let got = pg
                    .table(t)
                    .unwrap_or_else(|| panic!("声明里的表 `{t}` 不在 PG 期望集里"))
                    .column(c)
                    .unwrap_or_else(|| panic!("声明里的列 `{t}.{c}` 不在 PG 期望集里"))
                    .default
                    .clone();
                assert_eq!(
                    got.as_deref(),
                    Some(d.sql),
                    "`{t}.{c}` 的期望默认值不是 L2 声明的那一条 —— 没注入，或被别的东西覆盖了"
                );

                let lite_default =
                    lite.table(t).and_then(|x| x.column(c)).and_then(|x| x.default.clone());
                assert!(
                    lite_default.is_none(),
                    "`{t}.{c}` 是 PG 专有表达式默认值，SQLite 期望侧不得带它（实际 {:?}）",
                    lite_default
                );
            }
        }
    }

    /// 期望侧必须自洽：表按名升序、列按名升序（`finalize` 的不变量）。
    #[test]
    fn build_output_is_finalized() {
        let m = build(Dialect::Postgres).expect("构建应成功");
        for w in m.tables.windows(2) {
            assert!(w[0].name < w[1].name, "表未按名排序：{} / {}", w[0].name, w[1].name);
        }
        for t in &m.tables {
            for w in t.columns.windows(2) {
                assert!(w[0].name < w[1].name, "{}.{} 列未排序", t.name, w[1].name);
            }
        }
    }

    /// L2 静态声明全部落在实体覆盖的表上（认不到 ⇒ `build` 报错，不是静默少一条）。
    ///
    /// 这条测试是 `overlay_l2` 里那批 `ok_or_else` 的正面证据：只要它绿，说明
    /// **69 条 L2 声明里所有「指向表」的条目都找到了归属**。
    #[test]
    fn l2_static_decls_all_resolve_to_entity_tables() {
        let m = build(Dialect::Postgres).expect("L2 声明与实体集不一致时构建必须失败");

        for d in extras::all_indexes().filter(|d| l2_applies(d.dialect, Dialect::Postgres)) {
            let t = m.table(d.table).expect("索引声明应已解析到表");
            let hit = t.indexes.iter().find(|i| i.name == d.name).unwrap_or_else(|| {
                panic!(
                    "{} 上应已叠加索引 {}（该表实际索引：{:?}）",
                    d.table,
                    d.name,
                    t.indexes.iter().map(|i| &i.name).collect::<Vec<_>>()
                )
            });
            // ⚠ 核 `unique` 标志时必须看清这条断言的**能力边界**（2026-09-16 补）：
            // `overlay_l2` 是 `t.upsert_index(IndexModel { unique: d.unique, .. })` ——
            // **覆盖式**写入 ⇒ 本行实际只能挡住「`overlay_l2` 改成不覆盖 unique」这类
            // **实现回归**，挡不住「`extras.rs` 里某条声明被误改成 `unique: false`」
            // （那种情况下模型会忠实地跟着变，本行照样绿）。
            // 防「声明被误改」的判据是**另一条**：`composite_unique_decls_still_unique`
            // —— 它把迁移原文的 5 个复合唯一键硬编码成清单来锚定。
            assert_eq!(
                hit.unique, d.unique,
                "{} 的索引 {} 的 unique 与声明不一致（声明 {} / 模型 {}）—— \
                 `overlay_l2` 应原样透传 unique 标志",
                d.table, d.name, d.unique, hit.unique
            );
        }
        for d in
            extras::CHECK_CONSTRAINTS.iter().filter(|d| l2_applies(d.dialect, Dialect::Postgres))
        {
            let t = m.table(d.table).expect("CHECK 声明应已解析到表");
            assert!(t.checks.iter().any(|c| c.name == d.name), "应已叠加 CHECK {}", d.name);
        }
        for d in
            extras::GENERATED_COLUMNS.iter().filter(|d| l2_applies(d.dialect, Dialect::Postgres))
        {
            let t = m.table(d.table).expect("生成列声明应已解析到表");
            let c = t.column(d.col).expect("应已叠加生成列");
            assert!(c.generated.is_some(), "{}.{} 应被标为生成列", d.table, d.col);
            assert_eq!(c.sql_type, "tsvector", "{}.{} 类型应为 tsvector", d.table, d.col);
            assert!(c.nullable, "实测这批生成列全可空（tmp-p3-tsv-probe.log）");
        }
    }

    /// 实体派生的分流：主键 / 唯一 / 索引必须落在与 introspect 相同的桶里。
    #[test]
    fn entity_derivation_buckets_match_introspect() {
        let m = build(Dialect::Postgres).expect("构建应成功");

        // 主键列必然同时出现在 primary_key 集合里
        let t = m.table("reco_picks").expect("reco_picks 实体存在");
        assert_eq!(t.primary_key, vec!["id".to_string()]);
        assert!(t.column("id").is_some_and(|c| c.primary_key));

        // `#[sea_orm(indexed)]` → 索引名恒为 `idx-{表}-{列}`（sea-orm 硬编码）
        let t = m.table("stock_analyses").expect("stock_analyses 实体存在");
        assert!(
            t.indexes.iter().any(|i| i.name == "idx-stock_analyses-status"),
            "indexed 属性应派生 idx-{{表}}-{{列}}，实际 {:?}",
            t.indexes.iter().map(|i| &i.name).collect::<Vec<_>>()
        );
        // 且**不**应产出 pk 索引（复合主键才有，stock_analyses 是单列主键）
        assert!(!t.indexes.iter().any(|i| i.name == "pk-stock_analyses"));
    }

    /// 迁移时代靠 `CREATE UNIQUE INDEX` / **建表内联 `UNIQUE`** 表达的**唯一性语义**，
    /// 必须由列级 `#[sea_orm(unique)]` 承接。
    ///
    /// 为什么必须有这条判据（P6 删 74 个迁移的直接后果）：唯一性在迁移里有两处载体 ——
    /// ① 独立的 `CREATE UNIQUE INDEX`（如 `v135:45` 的 `idx_opc_demand_subs_keyword`）；
    /// ② 建表语句里的列级 `UNIQUE`。两者的共同点是**都不经过实体**，
    /// 所以删掉迁移文件后，这条语义只剩「实体列标志」一条路。
    /// 若该属性丢失，全新库会建出一张**允许重复值**的表 —— 不报错、不告警：
    /// 去重 / 对外路由键 / KV 语义静默消失（`repo` 侧「先查后插」随之失去兜底）。
    ///
    /// ⚠ 索引名**不**是这条语义的载体：实体派生名是 `idx-{表}-{列}`，与迁移里的
    /// `idx_{表}_{列}` 不同名，靠名字比对永远对不上 —— 所以判据锚在**列标志**上。
    ///
    /// ⚠ 2026-09-16 补：本判据原先只覆盖 `keyword` **一条**，漏掉了下面 5 条同型损失。
    /// 由独立验证者对抗复核发现 —— 且生产库上这 5 条的唯一约束**已被引擎 DROP 且未补**
    /// （实证 `output/tmp-*-execute.log` 的 `[执行] 段5 DROP UNIQUE`）。
    /// 教训：「同型缺陷必须按量纲穷举」，只查第一条就会把「未覆盖」误报成「已通过」。
    #[test]
    fn migration_unique_semantics_survive_in_column_flag() {
        // (表, 唯一列, 同表反例列（必须**不** unique）, 迁移出处)
        //
        // 反例列的用途：防「整表被误标 unique」这类假绿 —— 若判据读错了对象
        // （例如把列级标志读成表级），反例也会变 true ⇒ 立刻红。
        const CASES: &[(&str, &str, &str, &str)] = &[
            // 载体 ①：独立 CREATE UNIQUE INDEX
            (
                "opc_demand_subscriptions",
                "keyword",
                "enabled",
                "v135 `CREATE UNIQUE INDEX idx_opc_demand_subs_keyword`",
            ),
            // 载体 ②：建表语句内联 UNIQUE（下面 5 条同型，2026-09-16 才补进本判据）
            ("sync_devices", "unique_id", "name", "v116 `unique_id TEXT NOT NULL UNIQUE`"),
            (
                "sync_permissions",
                "device_id",
                "trust_level",
                "v116 `device_id … ON DELETE CASCADE UNIQUE`",
            ),
            ("opc_landing_pages", "slug", "title", "v210 `slug TEXT NOT NULL UNIQUE`"),
            ("opc_blog_posts", "slug", "title", "v210 `slug TEXT NOT NULL UNIQUE`"),
            ("trajectory_preferences", "key", "value", "v100 `key TEXT NOT NULL UNIQUE`"),
        ];

        // 两种方言都要有（唯一性是方言无关的语义，SQLite 侧同样不能丢）。
        for (dialect, dname) in [(Dialect::Postgres, "Postgres"), (Dialect::Sqlite, "Sqlite")] {
            let m = build(dialect).expect("构建应成功");
            for (table, col, counter, src) in CASES {
                // `table` / `col` / `counter` 解构出来是 `&&str` —— 直接传、交给 auto-deref；
                // 手写 `*` 会被 clippy 判 `explicit_auto_deref`（`-D warnings` ⇒ 门禁红）。
                let t = m.table(table).unwrap_or_else(|| panic!("{table} 实体应存在"));
                assert!(
                    t.column(col).is_some_and(|c| c.unique),
                    "{dname} 侧 {table}.{col} 丢了列级 unique ⇒ 全新库允许重复值。\
                     该唯一性在 P6 删迁移前由 `{src}` 承载，现在只能靠 `#[sea_orm(unique)]`"
                );
                assert!(
                    !t.column(counter).is_some_and(|c| c.unique),
                    "{dname} 侧 {table}.{counter} 不该是 unique —— 若它也 true，\
                     说明本判据读错了对象（区分力断言）"
                );
            }
        }
    }

    /// 迁移里那批**复合**唯一键，必须在 L2 里仍然声明为 `unique: true`。
    ///
    /// 为什么单列一条判据、复合**再一条**：唯一性有两条承接通道（见上一条判据的文档），
    /// 而实体列标志**表达不了复合键** —— 实测 6 个复合载体**全部**只靠 `extras.rs` 的
    /// `IndexDecl { unique: true }` 撑着，实体侧一点都沾不上。
    ///
    /// ⚠ 为什么不能只靠 `l2_static_decls_all_resolve_to_entity_tables` 里那句
    /// `assert_eq!(hit.unique, d.unique)`：`overlay_l2` 是 `upsert_index` **覆盖式**写入
    /// ⇒ 那句只能挡「实现不再透传 unique」这类回归，**挡不住**「有人把声明改成
    /// `unique: false`」（模型会忠实跟着变，那句照样绿）。本判据把**迁移原文**的语义
    /// 硬编码成清单来锚定，改错即红 —— 这是唯一能挡住它的形态（同 §6「判据锚在事实而非镜像」）。
    ///
    /// 出处逐条：`v229` 独立 `CREATE UNIQUE INDEX`；`v200` / `v123` 建表**表级**内联 `UNIQUE (…)`；
    /// `v138` / `v139` 独立唯一索引。另注：`v133` 的 `idx_opc_demand_leads_dedupe`
    /// 被 `v138:51` 显式 DROP ⇒ **不在此列**（它本就不该有承接，`extras.rs:47` 有记载）。
    #[test]
    fn composite_unique_decls_still_unique() {
        // (表, 列序（**有序** —— 复合键的列序是语义的一部分）, 迁移出处)
        const CASES: &[(&str, &[&str], &str)] = &[
            (
                "fleet_messages",
                &["fleet_id", "conversation_id", "seq"],
                "v229_create_fleet_messages.rs `CREATE UNIQUE INDEX uq_fleet_messages_convo_seq`",
            ),
            (
                "news_archive",
                &["source", "article_code"],
                "v200_axinvest_stock_tables.rs 表级 `UNIQUE (source, article_code)`",
            ),
            (
                "opc_demand_leads",
                &["platform", "content_fingerprint"],
                "v138_demand_lead_dedupe_fingerprint.rs `CREATE UNIQUE INDEX idx_opc_demand_leads_fingerprint`",
            ),
            (
                "session_events",
                &["session_id", "seq"],
                "v139_create_session_events.rs `CREATE UNIQUE INDEX idx_session_events_session_seq`",
            ),
            (
                "workflow_tools",
                &["workflow_id", "tool_name"],
                "v123_workflow_tools.rs 表级 `UNIQUE (workflow_id, tool_name)`",
            ),
        ];

        for (dialect, dname) in [(Dialect::Postgres, "Postgres"), (Dialect::Sqlite, "Sqlite")] {
            let m = build(dialect).expect("构建应成功");
            for (table, cols, src) in CASES {
                // `table` 是 `&&str`，交给 auto-deref（clippy `explicit_auto_deref`）。
                let t = m.table(table).unwrap_or_else(|| panic!("{table} 实体应存在"));
                let hit = t
                    .indexes
                    .iter()
                    .find(|i| {
                        i.cols.len() == cols.len()
                            && i.cols.iter().zip(cols.iter()).all(|(a, b)| a.as_str() == *b)
                    })
                    .unwrap_or_else(|| {
                        panic!(
                            "{dname} 侧 {table}({}) 上没有列集匹配的索引 —— \
                             该复合唯一键的 L2 声明被删或列序被改（表上实际索引：{:?}）",
                            cols.join(","),
                            t.indexes.iter().map(|i| (&i.name, &i.cols)).collect::<Vec<_>>()
                        )
                    });
                assert!(
                    hit.unique,
                    "{dname} 侧 {table}({}) 的唯一性被降级成普通索引。该语义在 P6 删迁移前由 \
                     `{src}` 承载；实体列标志**表达不了复合键**，只能靠 extras.rs 的 \
                     `IndexDecl {{ unique: true }}`",
                    cols.join(",")
                );
            }
        }
    }

    /// **排除式部分唯一索引**这条载体必须在**双方言**的期望集里都成立。
    ///
    /// 唯一性在删迁前有三类载体，本文件原先只锁住了前两类：
    /// ① 实体列级 `#[sea_orm(unique)]`（`migration_unique_semantics_survive_in_column_flag`）；
    /// ② L2 的**复合** `IndexDecl { unique: true }`（`composite_unique_decls_still_unique`）；
    /// ③ **单列 + WHERE** 的 `unique: true` —— 就是本条。
    ///
    /// ①② 都靠「迁移原文」锚定，而 `uq_trajectory_patterns_name` **没有迁移出处**
    /// （声明式新增，出处写的是缺陷编号 C-#2）⇒ 那套手法在它身上无对象，
    /// 这正是它当初被漏掉的成因（「同型缺陷按量纲穷举」少算了这一维）。
    ///
    /// ⚠ 为什么必须**双方言**都断言：本表的 `name` 列**没有** `#[sea_orm(unique)]`
    /// （见 `crates/entities/src/trajectory_patterns.rs`）⇒ 唯一性**只剩这一条载体**；
    /// 而 `IndexDecl.dialect` 一旦被写成 `Some(<另一方言>)`，**本方言**的期望集里整条消失、
    /// 引擎于是根本不建它，且**无任何报错** —— 全新库静默允许重名。
    /// 2026-09-17 修掉的 C-#2 正是这个形态（原为 `Some(Dialect::Postgres)`）。
    ///
    /// ⚠ 为什么锚**谓词文本**而不是「存在一个 unique 索引」：这条索引的全部语义是
    /// 「`rl_checkpoint:%` **之外**的同名唯一」。谓词丢了就退化成**整表唯一** ——
    /// 那比缺索引更糟：检查点**需要**同名多行（`src/commands/rl_training.rs:281` 的 name 是
    /// `format!("rl_checkpoint:{}", ckpt.name)`，`crates/harness/src/trajectory_types.rs:237`
    /// 有「需要同名多行的场景不要走 `new`」的告诫）⇒ 退化后的表现是**插入失败**，
    /// 而不是「约束没生效」，归因方向正好相反。
    #[test]
    fn partial_unique_predicate_survives_for_both_dialects() {
        const NAME: &str = "uq_trajectory_patterns_name";
        // 谓词逐字硬编码 —— 它就是「排除检查点」这条产品语义的载体，改错必须红。
        const PREDICATE: &str = "(name NOT LIKE 'rl_checkpoint:%')";
        // 反例：清单里唯一一条 `dialect: Some(Postgres)` 的 partial（见 `extras.rs` 的
        // `EXPECT_PARTIAL` 文档）—— 它**必须**在 SQLite 侧的期望集里缺席。
        const PG_ONLY: &str = "idx_retrieval_hits_feedback";

        // 区分力基线：方言过滤本身得是活的。若它恒真/恒假，下面「SQLite 侧存在」就只是
        // 「过滤失效」的副产物 —— 本判据会退化成恒绿（或恒红），两种都失去信号。
        assert!(
            !l2_applies(Some(Dialect::Postgres), Dialect::Sqlite)
                && l2_applies(Some(Dialect::Postgres), Dialect::Postgres)
                && l2_applies(None, Dialect::Sqlite)
                && l2_applies(None, Dialect::Postgres),
            "`l2_applies` 的方言判定自身失效了 ⇒ 本判据的区分力随之归零"
        );

        for (dialect, dname) in [(Dialect::Postgres, "Postgres"), (Dialect::Sqlite, "Sqlite")] {
            let m = build(dialect).expect("构建应成功");
            let t = m.table("trajectory_patterns").expect("trajectory_patterns 实体应存在");
            let hit = t.indexes.iter().find(|i| i.name == NAME).unwrap_or_else(|| {
                panic!(
                    "{dname} 侧期望集里没有 {NAME} —— 该声明的 `dialect` 被写成 \
                     `Some(<另一方言>)` 时本方言整条消失（C-#2），而本表 `name` 列没有 \
                     `#[sea_orm(unique)]`，唯一性只剩这一条载体 ⇒ 全新库静默允许重名。\
                     表上实际索引：{:?}",
                    t.indexes.iter().map(|i| &i.name).collect::<Vec<_>>()
                )
            });
            assert!(hit.unique, "{dname} 侧 {NAME} 的唯一性被降级成普通索引");
            assert_eq!(hit.cols, vec!["name".to_string()], "{dname} 侧 {NAME} 的列集变了");
            assert!(hit.method.is_none(), "{dname} 侧 {NAME} 不该指定访问方法（B-tree 即默认）");
            assert_eq!(
                hit.where_clause.as_deref(),
                Some(PREDICATE),
                "{dname} 侧 {NAME} 的排除谓词被改或丢了 —— 丢了就退化成**整表唯一**，\
                 连 `rl_checkpoint:%` 的同名多行也会被挡（那是插入失败，比缺索引更难归因）"
            );

            // 反例：PG-only 的那条在本方言的期望集里必须缺席（SQLite 侧）。
            let pg_only_present = m
                .table("retrieval_hits")
                .expect("retrieval_hits 实体应存在（L2 有它的索引声明）")
                .indexes
                .iter()
                .any(|i| i.name == PG_ONLY);
            assert_eq!(
                pg_only_present,
                dialect == Dialect::Postgres,
                "{PG_ONLY} 的方言归属错了（{dname} 侧 present={pg_only_present}）—— \
                 若两侧都 true，说明 `dialect: Some(..)` 被整个忽略，那么上面那条 \
                 「{NAME} 在本方言存在」的断言就失去了区分力"
            );
        }
    }

    /// `renamed_from` 的两种折法都要能回捞（形态逐字对齐 `sea-orm-2.0.2/src/schema/entity.rs:270-278`）。
    ///
    /// 回捞失败 = 引擎把「改名」退化成「DROP 旧列 + ADD 新列」= 静默丢数据。
    #[test]
    fn renamed_from_is_recovered_from_column_comment() {
        // 无原注释形态
        assert_eq!(
            renamed_from_of(Some("renamed_from \"industry_id\"")),
            Some("industry_id".to_string())
        );
        // 有原注释形态（`{comment}; renamed_from "{old}"`）
        assert_eq!(
            renamed_from_of(Some("业务域; renamed_from \"industry_id\"")),
            Some("industry_id".to_string())
        );
        // 没写过 rename ⇒ 不该凭空造一个
        assert_eq!(renamed_from_of(Some("普通注释")), None);
        assert_eq!(renamed_from_of(None), None);
    }

    /// `Expr` 渲染通道可用：`render_expr` 必须抠出**裸表达式**，不含列名与类型。
    #[test]
    fn render_expr_strips_column_and_type() {
        let e = Expr::val("live");
        let s = render_expr(&e, Dialect::Postgres);
        assert_eq!(s, "'live'");
        assert!(!s.contains("_ax_c"), "不应残留探测列名：{s}");
        assert!(!s.contains("DEFAULT"), "不应残留 DEFAULT 关键字：{s}");

        // 数值默认值
        assert_eq!(render_expr(&Expr::val(0i32), Dialect::Postgres), "0");
    }

    /// 模式匹配只认 `vec_*_meta`，且 `_meta_dup_backup` 这类残留**不匹配**
    /// （它必须被暴露成孤儿候选，而不是被模式悄悄认领）。
    #[test]
    fn pattern_tsv_matches_only_meta_tables() {
        let d = &extras::PATTERN_TSV[0];
        assert!(pattern_tsv_applies(d, "vec_capabilities_meta", Dialect::Postgres));
        assert!(pattern_tsv_applies(d, "vec_kb_abc_meta", Dialect::Postgres));
        assert!(!pattern_tsv_applies(d, "vec_kb_abc_meta_dup_backup", Dialect::Postgres));
        assert!(!pattern_tsv_applies(d, "vec_collections", Dialect::Postgres));
        assert!(!pattern_tsv_applies(d, "notes", Dialect::Postgres));
        // 方言不符 ⇒ 不适用
        assert!(!pattern_tsv_applies(d, "vec_capabilities_meta", Dialect::Sqlite));
    }

    /// 孤儿豁免认**整个向量族**（基表 + 元数据表），但**不**认备份残留。
    ///
    /// 与 `pattern_tsv_matches_only_meta_tables` 是两条独立判据：基表**不是**靠豁免保命的
    /// 前身 —— `PATTERN_TSV` 认领不了它（后缀不是 `_meta`），这正是本次裁决要补的裂缝
    /// （生产库 3 张基表共 49135 行）。
    #[test]
    fn orphan_exempt_covers_vector_family_but_not_backup_residue() {
        let d = &extras::ORPHAN_EXEMPT[0];
        assert!(
            matches!(d.scope, extras::OrphanExemptScope::Prefix { .. }),
            "`ORPHAN_EXEMPT[0]` 必须是向量族那条前缀规则（顺序变了要同步本测试）"
        );

        // 基表：过去必然被判孤儿 DROP
        let real_base = "vec_wiki_dc8efbd8_3a28_4fdf_9c96_9d26e2b2dac9";
        assert!(orphan_exempt_applies(d, "vec_capabilities", Dialect::Postgres));
        assert!(orphan_exempt_applies(d, real_base, Dialect::Postgres));
        // 元数据表：过去靠 `PATTERN_TSV` 的副作用保命
        assert!(orphan_exempt_applies(d, "vec_kb_abc_meta", Dialect::Postgres));
        // 备份残留：必须继续判孤儿（裁决②：先导出再 DROP）
        assert!(!orphan_exempt_applies(d, "vec_kb_abc_meta_dup_backup", Dialect::Postgres));
        // 非向量族不受影响 —— ⚠ 样本**不能**用 `axagent_schema_version`：
        // 它已被规则 2（`Exact`）豁免，拿它当「不受影响」的样本会变成一条**恒真**的
        // 断言（判据 #174：样本必须真落在规则范围之外）。
        assert!(!orphan_exempt_applies(d, "notes", Dialect::Postgres));
        assert!(!orphan_exempt_applies(d, "axagent_schema_versions", Dialect::Postgres));
        // 豁免是判定策略（`dialect: None`）⇒ 两侧都适用：`vector_store` 两种方言都建表
        assert!(orphan_exempt_applies(d, "vec_capabilities", Dialect::Sqlite));
    }

    /// 规则 2：`Exact` 形态只认列出来的那几张表，**前缀族不得顺带命中**。
    ///
    /// 这条守的是「规则 2 的**范围**」：它是为基础设施表开的口子，一旦写成前缀匹配
    /// （比如图省事写 `axagent_`），整个迁移子系统的表都会静默脱离孤儿判定。
    #[test]
    fn exact_orphan_exempt_covers_only_listed_infrastructure_tables() {
        let d = extras::ORPHAN_EXEMPT
            .iter()
            .find(|d| matches!(d.scope, extras::OrphanExemptScope::Exact(_)))
            .expect("必须有一条 Exact 规则（基础设施表）");

        // 被豁免的那张
        assert!(orphan_exempt_applies(d, "axagent_schema_version", Dialect::Postgres));
        assert!(orphan_exempt_applies(d, "axagent_schema_version", Dialect::Sqlite));
        // 近邻不得被顺带豁免（判据 #381：声称范围与落实范围逐对象对账）
        for near in ["axagent_schema_versions", "axagent_schema_version_backup", "schema_version"] {
            assert!(
                !orphan_exempt_applies(d, near, Dialect::Postgres),
                "`{near}` 不在精确名单里，不该被豁免"
            );
        }
        // owner 必须写在理由里 —— 豁免设施表的唯一依据就是「不归引擎管」
        assert!(d.reason.contains("owner"), "Exact 规则的理由必须写明 owner：{}", d.reason);
    }

    /// 豁免范围必须**覆盖** `PATTERN_TSV` 的匹配范围。
    ///
    /// 否则同一张表会被「建 `content_tsv` 列」（`plan.rs::pattern_level`）与「判孤儿
    /// `DROP TABLE`」（`plan.rs::table_level` ②）同时命中 ⇒ plan 自相矛盾。
    #[test]
    fn orphan_exempt_covers_every_pattern_tsv_match() {
        let tsv = &extras::PATTERN_TSV[0];
        // 用 `PATTERN_TSV` 自身的匹配条件**构造**一个必然命中的表名，再断言它也豁免。
        let sample = format!("{}kb_abc{}", tsv.table_prefix, tsv.table_suffix);
        assert!(
            pattern_tsv_applies(tsv, &sample, Dialect::Postgres),
            "构造样本必须命中 PATTERN_TSV"
        );
        assert!(
            orphan_exempt_decls().any(|d| orphan_exempt_applies(d, &sample, Dialect::Postgres)),
            "表 `{sample}` 会被 tsv 判据覆盖却不豁免孤儿 ⇒ plan 会自相矛盾"
        );
    }
}
