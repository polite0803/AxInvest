// SPDX-License-Identifier: AGPL-3.0-only

//! `reconcile::render` — 把一条 [`Change`] 渲染成可执行 DDL（P4，**PG 优先**）。
//!
//! ## 契约
//!
//! `render(change, dialect)` 是**纯函数**：输入一条 [`Change`]，输出零到多条语句。
//! 它不读库、不查目录 —— 所有需要的信息都在 [`ChangePayload`] 里（这正是 P4-1 给
//! `Change` 加结构化载荷的原因；从 `object` 字符串反查是做不到的：FK 的 object 是
//! `表.colA,colB`，**自带逗号**，与列级 `表.列` 不是同一套切分）。
//!
//! ## fail-closed
//!
//! 任何一条无法安全渲染的变更都返回 [`RenderError`]，**绝不静默跳过**。理由：跳过
//! 一条 DDL 与「同步成功」在日志上无法区分，而引擎每次启动都会重跑 ⇒ 一条永远渲染
//! 不出来的变更会退化成「每次启动都悄悄不同步」，比当场报错并停下危险得多。
//!
//! ## 方言能力：SQLite 只放行「原生支持的那 6 类」
//!
//! PG 走全量。SQLite 只放行 `CREATE TABLE` / `DROP TABLE` / `ADD COLUMN` /
//! `RENAME COLUMN` / `CREATE INDEX` / `DROP INDEX` —— 这 6 类就是 SQLite
//! `ALTER TABLE` 的原生能力边界（`RENAME TO` / `RENAME COLUMN TO` / `ADD COLUMN` /
//! `DROP COLUMN`，再加上两条不走 `ALTER` 的建删表与建删索引，见
//! [`sqlite_supports_natively`]）。
//!
//! 其余类别（`ALTER COLUMN TYPE` / `SET NOT NULL` / `DROP DEFAULT` / `ADD CONSTRAINT`
//! 全族 / `RENAME INDEX` / `RENAME CONSTRAINT`）在 SQLite 上**没有对应语法**，必须走
//! 「建新表 + 复制 + 改名 + 重建索引」的 12 步重建流程（PLAN §3.1）。那条路径尚未实现，
//! 故此处的处置是返回 [`RenderError::UnsupportedDialect`]，**绝不**退回一条必然失败的 SQL
//! （fail-closed 的理由见上一节）。
//!
//! ⚠ 「SQLite 能建出全部表」**不等于**「SQLite 能收敛全部漂移」，这两句话的适用范围差很多：
//!
//! | 场景 | 需要哪些类别 | 现状 |
//! |---|---|---|
//! | 全新库建表 | 只有 `CREATE TABLE`（+其内联展开的索引/FK/CHECK） | ✅ 本文件已支持 |
//! | 已有库、结构未漂移 | 0 条 | ✅ 同上 |
//! | 已有库、只有列/索引差异 | `ADD COLUMN` / `CREATE INDEX` | ✅ 同上 |
//! | 已有库、有约束/默认值/类型/非空差异 | 重建路径的 6 类外类别 | ❌ `UnsupportedDialect` |
//!
//! 最后一行的处置分两层，**必须分清**（2026-09-16 改判）：
//!
//! * **本文件**仍是 fail-closed —— 返回错误，绝不退回一条必然失败的 SQL（理由见上一节）；
//! * **`apply`** 把 `UnsupportedDialect` 判成「方言能力边界 ⇒ 只跳过本条」，**不再**凑成
//!   整批拒绝。旧行为（整批拒绝 ⇒ 启动中止）会让**每一个迁移建出的 SQLite 库**起不来，
//!   而这 6 类之外的差异在真实库里普遍存在。改判的论证与代价见
//!   `apply::SkipReason::UnsupportedDialect` 与 `apply::bootstrap_schema` 的文档。
//!
//! 即：**「渲染不出」和「因此整批停下」是两件事，别把它们绑在一起看。**
//!
//! ## 两处必须分方言的地方
//!
//! 1. **FK / CHECK 在 SQLite 必须内联进 `CREATE TABLE`** —— SQLite 的 `ALTER TABLE`
//!    没有 `ADD CONSTRAINT`（见 `create_table_statements`）。PG 侧仍是「建表 +
//!    若干 `ALTER TABLE … ADD CONSTRAINT`」，两者语句条数不同，这是刻意的。
//! 2. **索引谓词里的 PG 专有写法在 SQLite 要改写** —— `::` 类型转换与 `!~~` / `~~*`
//!    这类 PG 算子记号都不是 SQLite 语法（见 `create_index_sql` 与 `sqlite_expr`）。
//!    必须复用 `model::strip_casts` + `model::fold_like_operators`，理由是**两边同源**：
//!    `plan::same_index` 用 `normalize_sql_expr`（内含同这两个函数）比较两侧文本，
//!    若渲染侧不改写而比对侧改写，就会出现「渲染出的字符串对不上实况读回来的字符串」
//!    ⇒ 每轮重建索引、永不收敛。
//!
//! ## 三条「必须与 introspect 同源」的硬约束
//!
//! 引擎的收敛判据是「apply 后再 introspect，指纹 == 期望指纹」，所以渲染出的对象
//! **必须被 `introspect` 读成期望的那一种形态**。三处关键：
//!
//! 1. **列级 UNIQUE 用 `UNIQUE` 约束，不用唯一索引。** `dao/src/reconcile/introspect/pg.rs:201-219`
//!    把「`contype='u'` 且单列」分流成**列标志** `ColumnModel.unique` 并 `continue`
//!    （不产生 `IndexModel`）；而唯一索引会落成 `IndexModel{unique:true}`。若
//!    `AddUnique` 建的是唯一索引 ⇒ 列标志永远为 `false` ⇒ **每轮都报同一条差异**，
//!    引擎永不收敛。
//! 2. **多列 / 声明的 UNIQUE 用唯一索引**（`CreateIndex{unique:true}`）。其实况身份
//!    是**索引名**，与 `DROP INDEX` 对称；若建成约束，`DropIndex` 会失败（约束要用
//!    `ALTER TABLE … DROP CONSTRAINT`，`DROP INDEX` 对约束型 UNIQUE 无效）。
//! 3. **FK 动作字符串的词表与 `introspect/pg.rs::fk_action` 逐字一致**（`Cascade` /
//!    `SetNull` / `NoAction` / `Restrict` / `SetDefault`）。认不出的取值一律
//!    [`RenderError::UnknownFkAction`] —— **不能省略 `ON DELETE`**：省略等于把约束
//!    静默放宽成 `NO ACTION`，而计划里明明写着 `CASCADE`。
//!
//! ## `CREATE TABLE` 展开成多条语句
//!
//! `plan.rs` 只为「期望有、实况无」的表发 `CreateTable`，而 `index_level` /
//! `fk_level` / `check_level` **只在两侧都有的表上跑** —— 即新表的索引 / FK / CHECK
//! **不会再单独生成变更**。故 `CREATE TABLE` 必须把它们一并渲染，否则新表只有裸列，
//! 且下一轮引擎会把这些对象全判成孤儿并把索引 `DROP INDEX` 掉。
//!
//! ## 不自作聪明的地方
//!
//! - `DROP TABLE` **不加 `CASCADE`**：有依赖对象时让 PG 报错停下，而不是静默连带删掉
//!   别人（PLAN §八 风险 1d 的同族教训）。
//! - `ALTER COLUMN … TYPE` **总是带 `USING`**：不带时 PG 走赋值转换，遇到
//!   `jsonb → json`、`bigint → integer` 这类会直接拒绝；带 `USING c::t` 则语义明确。
//! - `ALTER COLUMN … TYPE` **前面先垫一条 `DROP DEFAULT`**：光有 `USING` **不够** ——
//!   列上若已有默认值，PG 会把**默认值**也一并转换，而那条转换走的是**赋值转换**规则，
//!   `integer → boolean` 这种只有 explicit cast 的组合会被拒绝。实测报错原文：
//!   `字段 "is_template" 的默认值不能转换成类型 boolean`（见 `AlterColumnType` 分支）。
//! - 生成列**排在建表语句的非生成列之后**：`STORED` 生成列引用同表其它列，把被引用
//!   列放在前面是零成本的防御（PG 本身也允许后向引用，但没必要赌）。

use std::fmt;

use super::extras::Dialect;
use super::model::{
    CheckModel, ColumnModel, FkModel, IndexModel, TableModel, fold_like_operators, strip_casts,
};
use super::plan::{Change, ChangeKind, ChangePayload};

// ═══════════════════════════════════════════════════════════════════════════
// 出口类型
// ═══════════════════════════════════════════════════════════════════════════

/// 一条变更的渲染结果。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Rendered {
    /// 按**执行顺序**排列的语句（`CreateTable` 会展开成 1 + N 条）。
    pub statements: Vec<String>,
    /// 需要留痕的语义（进审计表 / 日志）。不是错误，是「执行方必须知道的事」。
    pub notes: Vec<String>,
}

impl Rendered {
    fn one(stmt: String) -> Self {
        Self { statements: vec![stmt], notes: Vec::new() }
    }

    fn many(stmts: Vec<String>) -> Self {
        Self { statements: stmts, notes: Vec::new() }
    }

    fn with_note(mut self, note: impl Into<String>) -> Self {
        self.notes.push(note.into());
        self
    }
}

/// 渲染失败。**每一种都指得出是哪条变更的哪个字段** —— 否则日志里只有一句
/// 「渲染失败」，无法定位。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RenderError {
    /// 该方言**没有**这类变更的原生 DDL。
    ///
    /// 当前唯一来源是 SQLite，且只覆盖 [`sqlite_supports_natively`] 那 6 类之外的类别 ——
    /// 它们在 SQLite 上必须走「重建表」12 步流程（尚未实现）。PG 走全量，不会产生它。
    UnsupportedDialect { dialect: Dialect, kind: ChangeKind },
    /// `Change.payload` 的变体与 `Change.kind` 不匹配（构造侧缺陷）。
    PayloadMismatch {
        kind: ChangeKind,
        object: String,
        /// 该变更**需要**的载荷类型。
        want: &'static str,
        /// **实际拿到**的载荷类型。
        got: &'static str,
    },
    /// 渲染必需的标识符为空 —— 多半是实况侧匿名对象（SQLite 的匿名 UNIQUE / CHECK）。
    EmptyIdentifier { kind: ChangeKind, object: String, field: &'static str },
    /// 类型名 / 访问方法名含白名单外字符（防注入与防语法错误）。
    UnsafeSqlWord { kind: ChangeKind, object: String, word: String },
    /// FK 动作的取值不在 `introspect::fk_action` 的词表里。
    UnknownFkAction { object: String, action: String },
    /// 结构上无法渲染（缺生成列表达式、生成列同时有 DEFAULT、主键列不在列集里 …）。
    Unrenderable { kind: ChangeKind, object: String, why: String },
}

impl fmt::Display for RenderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedDialect { dialect, kind } => write!(
                f,
                "render: `{}` 在 {dialect:?} 上没有原生 DDL —— SQLite 的 ALTER TABLE 只有 \
                 RENAME TO / RENAME COLUMN TO / ADD COLUMN / DROP COLUMN，这类变更必须走\
                 「建新表 + 复制 + 改名 + 重建索引」的 12 步流程（PLAN §3.1，尚未实现）",
                kind.as_str()
            ),
            Self::PayloadMismatch { kind, object, want, got } => write!(
                f,
                "render: `{object}`（{}）的载荷是 {got}，该变更需要 {want}",
                kind.as_str()
            ),
            Self::EmptyIdentifier { kind, object, field } => write!(
                f,
                "render: `{object}`（{}）的 `{field}` 为空，无法生成 DDL（匿名对象需先具名）",
                kind.as_str()
            ),
            Self::UnsafeSqlWord { kind, object, word } => write!(
                f,
                "render: `{object}`（{}）的类型/方法名 `{word}` 含白名单外字符，拒绝拼接",
                kind.as_str()
            ),
            Self::UnknownFkAction { object, action } => write!(
                f,
                "render: `{object}` 的外键动作 `{action}` 不在词表内，拒绝省略（省略会静默放宽为 NO ACTION）"
            ),
            Self::Unrenderable { kind, object, why } => {
                write!(f, "render: `{object}`（{}）无法渲染：{why}", kind.as_str())
            },
        }
    }
}

impl std::error::Error for RenderError {}

// ═══════════════════════════════════════════════════════════════════════════
// 主入口
// ═══════════════════════════════════════════════════════════════════════════

/// SQLite 有**原生** DDL 可用的变更类别（不需要走 12 步重建表流程）。
///
/// SQLite 的 `ALTER TABLE` 只有四条：`RENAME TO` / `RENAME COLUMN TO` / `ADD COLUMN` /
/// `DROP COLUMN`（3.35+）。加上两条不走 `ALTER` 的（建删表、建删索引），共 6 类。
///
/// ## 为什么是白名单而不是 `!needs_rebuild()`
///
/// 反过来的黑名单要求「每个新加的 [`ChangeKind`] 都被显式判定过一次」—— 而新增类别时
/// 最容易发生的事就是**忘了在两处都加**：忘了加进黑名单 ⇒ 新类别在 SQLite 上被当成
/// 有原生实现 ⇒ 渲染出一条 SQLite 根本不认的 SQL ⇒ 启动路径 fail-stop（还算好）；
/// 更糟的是若那条 SQL 恰好合法但语义不同（如 `ALTER TABLE … ADD COLUMN` 的
/// `NOT NULL` 无默认值），就会静默建出错的表。白名单让「没想到」的默认落到
/// **拒绝**那一侧，与 [`RenderError::UnsupportedDialect`] 的 fail-closed 口径一致。
///
/// 配套守卫：`render.rs` 单测里的 `every_change_kind_is_classified_for_sqlite` ——
/// 它遍历 [`ChangeKind::ALL`] 逐项比对本白名单，新增类别时**必然**红灯（`plan.rs` 的
/// `is_purely_additive` 用的是同一套手法，两处口径相同）。
fn sqlite_supports_natively(kind: ChangeKind) -> bool {
    use ChangeKind::*;
    matches!(kind, CreateTable | DropTable | AddColumn | RenameColumn | CreateIndex | DropIndex)
}

/// 渲染一条变更为 DDL 语句序列。
///
/// `dialect` 由调用方给出（`Change` 里没有它 —— 它是连接的属性，不是变更的属性）。
///
/// PG 全量支持；SQLite 只支持 [`sqlite_supports_natively`] 放行的 6 类，其余返回
/// [`RenderError::UnsupportedDialect`]（方言能力边界的完整说明见模块文档）。
pub fn render(change: &Change, dialect: Dialect) -> Result<Rendered, RenderError> {
    if dialect != Dialect::Postgres && !sqlite_supports_natively(change.kind) {
        return Err(RenderError::UnsupportedDialect { dialect, kind: change.kind });
    }

    use ChangeKind::*;
    match change.kind {
        // ── 段 0：改名 ──
        RenameColumn => {
            let (table, from, to) = rename_of(change)?;
            require(table, change, "table")?;
            require(from, change, "from（旧列名）")?;
            require(to, change, "to（新列名）")?;
            Ok(Rendered::one(format!(
                "ALTER TABLE {} RENAME COLUMN {} TO {}",
                quote_ident(table),
                quote_ident(from),
                quote_ident(to)
            )))
        },
        RenameIndex => {
            let (_table, from, to) = rename_of(change)?;
            // 索引改名不经过表名（`ALTER INDEX` 直接作用于索引）。
            require(from, change, "from（旧索引名）")?;
            require(to, change, "to（新索引名）")?;
            Ok(Rendered::one(format!(
                "ALTER INDEX {} RENAME TO {}",
                quote_ident(from),
                quote_ident(to)
            )))
        },
        RenameForeignKey => {
            let (table, from, to) = rename_of(change)?;
            require(table, change, "table")?;
            require(from, change, "from（旧约束名）")?;
            require(to, change, "to（新约束名）")?;
            Ok(Rendered::one(format!(
                "ALTER TABLE {} RENAME CONSTRAINT {} TO {}",
                quote_ident(table),
                quote_ident(from),
                quote_ident(to)
            )))
        },

        // ── 段 1：撤依赖 ──
        DropCheck => {
            let (table, def) = check_of(change)?;
            require(table, change, "table")?;
            require(&def.name, change, "name（CHECK 约束名）")?;
            Ok(Rendered::one(format!(
                "ALTER TABLE {} DROP CONSTRAINT {}",
                quote_ident(table),
                quote_ident(&def.name)
            )))
        },
        DropForeignKey => {
            let (table, def) = fk_of(change)?;
            require(table, change, "table")?;
            let name = fk_constraint_name(table, def);
            let note = format!(
                "DROP FK `{table}`：按派生名 `{name}` 删除（实况侧无名字时 PG 的默认命名规则）"
            );
            Ok(Rendered::one(format!(
                "ALTER TABLE {} DROP CONSTRAINT {}",
                quote_ident(table),
                quote_ident(&name)
            ))
            .with_note(note))
        },
        DropIndex => {
            let (_table, def) = index_of(change)?;
            require(&def.name, change, "name（索引名）")?;
            Ok(Rendered::one(format!("DROP INDEX {}", quote_ident(&def.name))).with_note(
                "DROP INDEX 不带表名限定：索引不在 search_path 的 schema 内会失败（fail-closed，不静默）"
                    .to_string(),
            ))
        },
        DropTable => {
            // `object` 就是表名（载荷为 `Bare`）。**不加 CASCADE**。
            //
            // ⚠ `Bare` 必须**显式校验**：本分支不读载荷的任何字段，若只取 `object`
            // 就当表名用，那么一条被误配上 `Column` / `Index` 载荷的 `DropTable` 会
            // **静默渲染成 `DROP TABLE`** —— 「载荷与 kind 必须匹配」这条不变量就多了
            // 一个例外（P4-2 首轮由测试 `payload_mismatch_is_rejected` 戳出）。
            if !matches!(change.payload, ChangePayload::Bare) {
                return Err(mismatch(change, "Bare"));
            }
            let table = &change.object;
            require(table, change, "object（表名）")?;
            Ok(Rendered::one(format!("DROP TABLE {}", quote_ident(table))).with_note(
                "DROP TABLE 不带 CASCADE：有依赖对象时让 PG 报错停下，避免连带删掉未在计划内的对象"
                    .to_string(),
            ))
        },

        // ── 段 2：建表 ──
        CreateTable => {
            let ChangePayload::Table(t) = &change.payload else {
                return Err(mismatch(change, "Table"));
            };
            let mut r = Rendered::many(create_table_statements(t, change, dialect)?);
            r.notes.push(format!(
                "CREATE TABLE `{}` 展开成 {} 条：索引 / FK / CHECK 不会再单独生成变更（plan 只对两侧都有的表做对象级对账）",
                t.name,
                r.statements.len()
            ));
            Ok(r)
        },

        // ── 段 3：列变更 ──
        AddColumn => {
            let (table, col) = col_of(change)?;
            require(table, change, "table")?;
            Ok(Rendered::one(format!(
                "ALTER TABLE {} ADD COLUMN {}",
                quote_ident(table),
                column_def(col, change, dialect)?
            )))
        },
        AlterColumnType => {
            let (table, col) = col_of(change)?;
            require(table, change, "table")?;
            require(&col.name, change, "col.name")?;
            check_sql_word(&col.sql_type, change)?;
            let t = quote_ident(table);
            let c = quote_ident(&col.name);
            let ty = &col.sql_type;
            // 三步：**先 DROP DEFAULT** → `TYPE … USING` → `SET DEFAULT`（仅当期望侧有）。
            //
            // ⚠⚠ 为什么必须先 DROP DEFAULT（P4 首次真库执行实测，2026-09-16）：
            // 只写 `USING c::t` **不够**。列上若有默认值，PG 在 `ALTER COLUMN TYPE` 时会把
            // **默认值也一并转换**，而那条转换走**赋值转换**（assignment cast）规则 ——
            // `integer → boolean` 这类只有 explicit cast 的组合会被拒。实测原文：
            //
            // ```text
            // ALTER TABLE "narrative_structures" ALTER COLUMN "is_template" TYPE boolean
            //   USING "is_template"::boolean
            // 失败：字段 "is_template" 的默认值不能转换成类型 boolean
            // ```
            //
            // 当时计划里确实有该列的 `DropDefault`，但它按 `ChangeKind` 的枚举序排在本条
            // **之后** ⇒ 永远执行不到，整条 ALTER TYPE 卡死并 fail-stop 掉后面 461 条。
            //
            // 修法选「把 DROP DEFAULT 并进本条渲染」，**不**选「把 `DropDefault` 排到
            // `AlterColumnType` 前面」：后者修的是「同列两条变更的相对顺序」这个**全局**
            // 性质，任何一次排序调整（或将来多出一条同列变更）都可能把它重新破坏；而
            // 并进本条之后，`AlterColumnType` **自包含**，排在哪里都对。
            //
            // 无默认值的列上 `DROP DEFAULT` 是合法 no-op（PG 不报错），故可无条件加。
            let mut stmts = vec![
                format!("ALTER TABLE {t} ALTER COLUMN {c} DROP DEFAULT"),
                format!("ALTER TABLE {t} ALTER COLUMN {c} TYPE {ty} USING {c}::{ty}"),
            ];
            // 期望侧有默认值 ⇒ 用**期望侧**的值补回（它已经是新类型的值，无需转换）。
            // 这里刻意不做 `check_sql_word`：默认值本就是表达式（`'active'::text`、
            // `now()`），白名单会误杀 —— 与 `SetDefault` 分支口径一致。
            if let Some(d) = col.default.as_deref() {
                stmts.push(format!("ALTER TABLE {t} ALTER COLUMN {c} SET DEFAULT {d}"));
            }
            Ok(Rendered::many(stmts))
        },
        SetNotNull | DropNotNull | DropDefault => {
            let (table, col) = col_of(change)?;
            require(table, change, "table")?;
            require(&col.name, change, "col.name")?;
            let verb = match change.kind {
                SetNotNull => "SET NOT NULL",
                DropNotNull => "DROP NOT NULL",
                _ => "DROP DEFAULT",
            };
            Ok(Rendered::one(format!(
                "ALTER TABLE {} ALTER COLUMN {} {verb}",
                quote_ident(table),
                quote_ident(&col.name)
            )))
        },
        SetDefault => {
            let (table, col) = col_of(change)?;
            require(table, change, "table")?;
            require(&col.name, change, "col.name")?;
            let Some(default) = col.default.as_deref() else {
                return Err(RenderError::Unrenderable {
                    kind: change.kind,
                    object: change.object.clone(),
                    why: "`SetDefault` 但期望列的 `default` 为 None（建模矛盾）".to_string(),
                });
            };
            Ok(Rendered::one(format!(
                "ALTER TABLE {} ALTER COLUMN {} SET DEFAULT {default}",
                quote_ident(table),
                quote_ident(&col.name)
            )))
        },
        AlterGenerated => {
            let (table, col) = col_of(change)?;
            require(table, change, "table")?;
            require(&col.name, change, "col.name")?;
            if col.generated.is_none() {
                return Err(RenderError::Unrenderable {
                    kind: change.kind,
                    object: change.object.clone(),
                    why: "`AlterGenerated` 但期望列没有生成表达式".to_string(),
                });
            }
            // PG **不支持**给已有列补 `GENERATED`（既不能 `ALTER COLUMN … ADD GENERATED`，
            // 也不能改表达式）⇒ 只能 DROP + ADD。STORED 生成列的值是派生的，重建即重算，
            // **不丢数据**（`loses_data()` 之所以把它排除在外正是这个理由）。
            Ok(Rendered::many(vec![
                format!(
                    "ALTER TABLE {} DROP COLUMN {}",
                    quote_ident(table),
                    quote_ident(&col.name)
                ),
                format!(
                    "ALTER TABLE {} ADD COLUMN {}",
                    quote_ident(table),
                    column_def(col, change, dialect)?
                ),
            ])
            .with_note(
                "生成列改为「DROP COLUMN + ADD COLUMN」：PG 不支持给已有列补 GENERATED。\
                 值由表达式重算，不丢数据；但**引用该列的索引会被 DROP COLUMN 级联删除**，\
                 若该索引两侧一致（不产生 CreateIndex）则本轮不会重建 —— 下一轮启动时收敛。"
                    .to_string(),
            ))
        },

        // ── 段 4：删列 ──
        DropColumn => {
            let (table, col) = col_of(change)?;
            require(table, change, "table")?;
            require(&col.name, change, "col.name")?;
            // PG 的 `DROP COLUMN` **不带** CASCADE ⇒ 被视图/索引依赖时报错停下。
            Ok(Rendered::one(format!(
                "ALTER TABLE {} DROP COLUMN {}",
                quote_ident(table),
                quote_ident(&col.name)
            )))
        },

        // ── 段 5：建依赖 ──
        AddUnique => {
            let (table, col) = col_of(change)?;
            require(table, change, "table")?;
            require(&col.name, change, "col.name")?;
            let name = unique_constraint_name(table, &col.name);
            Ok(Rendered::one(format!(
                "ALTER TABLE {} ADD CONSTRAINT {} UNIQUE ({})",
                quote_ident(table),
                quote_ident(&name),
                quote_ident(&col.name)
            ))
            .with_note(format!(
                "列级 UNIQUE 建成**约束**（不是唯一索引）：introspect 只把 `contype='u'` 单列读成列标志，建成索引会永不收敛。名字 `{name}` 与 PG 自动命名规则一致"
            )))
        },
        DropUnique => {
            let (table, col) = col_of(change)?;
            require(table, change, "table")?;
            require(&col.name, change, "col.name")?;
            let name = unique_constraint_name(table, &col.name);
            Ok(Rendered::one(format!(
                "ALTER TABLE {} DROP CONSTRAINT {}",
                quote_ident(table),
                quote_ident(&name)
            ))
            .with_note(format!(
                "DROP UNIQUE 按**派生名** `{name}` 删除；若该约束当初用了别的名字，PG 会报错停下（fail-closed，不会静默漏删）"
            )))
        },
        AddForeignKey => {
            let (table, def) = fk_of(change)?;
            require(table, change, "table")?;
            Ok(Rendered::one(format!(
                "ALTER TABLE {} ADD CONSTRAINT {} {}",
                quote_ident(table),
                quote_ident(&fk_constraint_name(table, def)),
                fk_clause(def, change)?
            )))
        },
        AddCheck => {
            let (table, def) = check_of(change)?;
            require(table, change, "table")?;
            require(&def.name, change, "name（CHECK 约束名）")?;
            if def.expr.trim().is_empty() {
                return Err(RenderError::Unrenderable {
                    kind: change.kind,
                    object: change.object.clone(),
                    why: "CHECK 表达式为空".to_string(),
                });
            }
            Ok(Rendered::one(format!(
                "ALTER TABLE {} ADD CONSTRAINT {} CHECK ({})",
                quote_ident(table),
                quote_ident(&def.name),
                def.expr
            )))
        },
        CreateIndex => {
            let (table, def) = index_of(change)?;
            require(table, change, "table")?;
            Ok(Rendered::one(create_index_sql(table, def, change, dialect)?))
        },
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// 载荷解构（变体不符 ⇒ PayloadMismatch，而不是 panic）
// ═══════════════════════════════════════════════════════════════════════════

fn col_of(change: &Change) -> Result<(&String, &ColumnModel), RenderError> {
    match &change.payload {
        ChangePayload::Column { table, col } => Ok((table, col)),
        _ => Err(mismatch(change, "Column")),
    }
}

fn index_of(change: &Change) -> Result<(&String, &IndexModel), RenderError> {
    match &change.payload {
        ChangePayload::Index { table, def } => Ok((table, def)),
        _ => Err(mismatch(change, "Index")),
    }
}

fn fk_of(change: &Change) -> Result<(&String, &FkModel), RenderError> {
    match &change.payload {
        ChangePayload::Fk { table, def } => Ok((table, def)),
        _ => Err(mismatch(change, "Fk")),
    }
}

fn check_of(change: &Change) -> Result<(&String, &CheckModel), RenderError> {
    match &change.payload {
        ChangePayload::Check { table, def } => Ok((table, def)),
        _ => Err(mismatch(change, "Check")),
    }
}

fn rename_of(change: &Change) -> Result<(&String, &String, &String), RenderError> {
    match &change.payload {
        ChangePayload::Rename { table, from, to } => Ok((table, from, to)),
        _ => Err(mismatch(change, "Rename")),
    }
}

fn mismatch(change: &Change, want: &'static str) -> RenderError {
    RenderError::PayloadMismatch {
        kind: change.kind,
        object: change.object.clone(),
        want,
        got: payload_name(&change.payload),
    }
}

/// 载荷变体名（只用于错误消息）。
fn payload_name(p: &ChangePayload) -> &'static str {
    match p {
        ChangePayload::Bare => "Bare",
        ChangePayload::Rename { .. } => "Rename",
        ChangePayload::Column { .. } => "Column",
        ChangePayload::Index { .. } => "Index",
        ChangePayload::Fk { .. } => "Fk",
        ChangePayload::Check { .. } => "Check",
        ChangePayload::Table(_) => "Table",
    }
}

/// 标识符非空断言（fail-closed 的统一入口）。
fn require(value: &str, change: &Change, field: &'static str) -> Result<(), RenderError> {
    if value.trim().is_empty() {
        Err(RenderError::EmptyIdentifier {
            kind: change.kind,
            object: change.object.clone(),
            field,
        })
    } else {
        Ok(())
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// 标识符 / 词元
// ═══════════════════════════════════════════════════════════════════════════

/// 引用一个标识符：`"name"`，内部 `"` 双写（PG 规则）。
///
/// **一律加引号**，不做「小写裸名可以不加」的优化：那会让大小写敏感的名字（生产库里
/// 有 `vec_kb_F9B2…` 这类混写名）在某一侧静默变成小写，而且 `DROP TABLE Foo` 与
/// `DROP TABLE "Foo"` 是不同的对象。
///
/// `pub(crate)` 而非私有：`reconcile::evidence` 也要把标识符拼进 SQL。**标识符引用
/// 必须只有一份实现** —— 第二份的典型腐坏形态是忘了双写内部 `"`（那样带引号的表名
/// 会拼出语法错误的 SQL，而两边看起来都「像是在引用标识符」）。
pub(crate) fn quote_ident(s: &str) -> String {
    format!("\"{}\"", s.replace('"', "\"\""))
}

/// 类型名 / 访问方法名等**不可加引号**的词元安全校验。
///
/// 为什么必须有：类型名来自「实况内省」（`format_type(...)` 经归一）与「L2 声明」
/// （`col_type` 字面量），**不经转义**就要拼进 DDL。白名单外的一切字符都拒 ——
/// 尤其是 `;`（语句分隔）、`'` / `"`（造字面量或提前闭合标识符）、`-`（`--` 注释）。
///
/// 合法样本（生产库实测类型名的全体）：`text` / `bigint` / `double precision` /
/// `numeric(10,2)` / `vector(1024)` / `text[]` / `timestamp without time zone`。
fn sql_word_is_safe(word: &str) -> bool {
    !word.trim().is_empty()
        && word.chars().all(|c| {
            c.is_ascii_alphanumeric() || matches!(c, '_' | ' ' | '(' | ')' | ',' | '[' | ']' | '.')
        })
}

fn check_sql_word(word: &str, change: &Change) -> Result<(), RenderError> {
    if sql_word_is_safe(word) {
        Ok(())
    } else {
        Err(RenderError::UnsafeSqlWord {
            kind: change.kind,
            object: change.object.clone(),
            word: word.to_string(),
        })
    }
}

/// PG 的标识符长度上限（`NAMEDATALEN - 1`）。超出时 PG 自己会截断 —— 显式先截断，
/// 让「建」与「删」用同一个名字，否则 `DROP CONSTRAINT` 找不到对象。
fn truncate_ident(s: &str) -> String {
    const MAX: usize = 63;
    if s.len() <= MAX {
        return s.to_string();
    }
    let mut end = MAX;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    s[..end].to_string()
}

/// 列级 UNIQUE 约束名 —— 与 PG 的自动命名规则 `{表}_{列}_key` 一致。
///
/// 为什么必须自己派生：`AddUnique` / `DropUnique` 的载荷里**只有列**，没有约束名
/// （`ColumnModel.unique` 是个 bool）。用默认命名规则派生，就能让「建」与「删」
/// 以及「`CREATE TABLE` 内联的 `UNIQUE (c)`」三处指向同一个对象。
fn unique_constraint_name(table: &str, col: &str) -> String {
    truncate_ident(&format!("{table}_{col}_key"))
}

/// FK 约束名：载荷给了就用，没给则按 PG 默认规则 `{表}_{列集}_fkey` 派生。
fn fk_constraint_name(table: &str, def: &FkModel) -> String {
    match def.name.as_deref() {
        Some(n) if !n.trim().is_empty() => n.to_string(),
        _ => truncate_ident(&format!("{table}_{}_fkey", def.cols.join("_"))),
    }
}

/// `introspect/pg.rs::fk_action` 的**反向**映射（稳定串 → SQL 关键字）。
///
/// 词表必须逐字镜像：`introspect` 侧产出 `NoAction` / `Restrict` / `Cascade` /
/// `SetNull` / `SetDefault`（以及未知编码的 `UNKNOWN(x)`），期望侧 `expected.rs:353-354`
/// 用的是 `sea_orm::sea_query::ReferentialAction::variant_name()`，取值集合相同。
/// 认不出的取值返回 `None` ⇒ 上层报错，**绝不省略**。
fn fk_action_sql(action: &str) -> Option<&'static str> {
    Some(match action {
        "NoAction" => "NO ACTION",
        "Restrict" => "RESTRICT",
        "Cascade" => "CASCADE",
        "SetNull" => "SET NULL",
        "SetDefault" => "SET DEFAULT",
        _ => return None,
    })
}

// ═══════════════════════════════════════════════════════════════════════════
// 对象级渲染
// ═══════════════════════════════════════════════════════════════════════════

/// 列的实际 DDL 类型名 —— **只有「建列」路径用这个映射**。
///
/// ## 为什么映射写在这里、而不是改写 `ColumnModel.sql_type`
///
/// 自增在 PG 里的写法是**伪类型** `smallserial` / `serial` / `bigserial`，它只在
/// `CREATE TABLE` / `ADD COLUMN` 里合法；`ALTER COLUMN … TYPE bigserial` 是**语法错误**
/// （`ALTER COLUMN … TYPE` 见 [`render`] 的 `AlterColumnType` 分支，那里直接用
/// `col.sql_type` 原文）。**同一个字段在两条渲染路径上语义不同** ⇒ 把映射放在调用侧，
/// 让「什么时候可以是伪类型」这件事只由使用点决定。
///
/// ## 为什么要 fail-closed 而不是退回原类型
///
/// 实况侧 `format_type()` 对 `bigserial` 列报回的是 `bigint`（serial 不是真类型，
/// 只是「整型 + 序列 + 默认值」的语法糖）⇒ 期望/实况两边的 `sql_type` 本就都是 `bigint`。
/// 若这里认不出类型就直接写 `bigint`，会建出一张**没有自增**的表 —— 而且它是**静默**的：
/// 表建成了、列也在、只是不给 id 的 INSERT 会失败。宁可整条变更渲染失败（`apply` 会
/// 整批拒绝并说明原因），也不建一张错的表。
///
/// ## SQLite 分支：**不能**映射成 `serial`，必须写回 `INTEGER`
///
/// SQLite 没有自增伪类型，「自增」的同义语是 **rowid 别名**。它的成立条件是
/// 「单列主键 + 声明类型**逐字是 `INTEGER`**」（`dao/src/reconcile/introspect/sqlite.rs:221-238` 用的
/// 就是这条判据）。两个反例说明为什么必须显式映射：
///
/// - 写成 `serial`：`serial` 里没有 `INT` 子串 ⇒ 亲和性是 **NUMERIC** ⇒ 既不是
///   rowid 别名，连 INTEGER 亲和性都拿不到。建表**照样成功**，但 INSERT 立刻坏。
/// - 写成 `bigint`：含 `INT` ⇒ 亲和性是 INTEGER，但**不是逐字 `INTEGER`** ⇒
///   仍然**不是** rowid 别名。
///
/// 两条都属于「建表成功、运行期才炸」，且 sqlite 对任意类型名一律接受 ⇒ 语法探针
/// **看不见**它们。故此处不依赖探针，直接按 SQLite 的判据写死。
///
/// 映射成 `INTEGER` 与期望侧的 `sql_type` 是否可比？可比 —— 两侧的 `sql_type` 都是
/// **亲和性类别**（`model::canonical_type_expected` / `canonical_type_actual` 都过
/// `sqlite_class`），`INTEGER` 与 `integer` 归到同一个类，且大小写不敏感。
fn column_sql_type(
    col: &ColumnModel,
    change: &Change,
    dialect: Dialect,
) -> Result<String, RenderError> {
    if !col.auto_increment {
        return Ok(col.sql_type.clone());
    }
    if dialect == Dialect::Sqlite {
        // 期望侧 `sql_type` 是亲和性类别，自增列只可能落在 `integer` 这一类上
        // （`model::sqlite_declared_type` 把 Integer / BigInteger 全归成 `integer`）。
        // 认不出 ⇒ 报错而不是硬写 `INTEGER`：把 text 列改成 INTEGER 是静默改语义。
        if col.sql_type.eq_ignore_ascii_case("integer") {
            return Ok("INTEGER".to_string());
        }
        return Err(RenderError::Unrenderable {
            kind: change.kind,
            object: change.object.clone(),
            why: format!(
                "列 `{}` 在 SQLite 上声明为自增（rowid 别名），但类型类别是 `{}` —— \
                 rowid 别名要求 INTEGER；把非整型列建成 INTEGER 是静默改语义，故拒绝",
                col.name, col.sql_type
            ),
        });
    }
    match col.sql_type.as_str() {
        "smallint" => Ok("smallserial".to_string()),
        "integer" => Ok("serial".to_string()),
        "bigint" => Ok("bigserial".to_string()),
        other => Err(RenderError::Unrenderable {
            kind: change.kind,
            object: change.object.clone(),
            why: format!(
                "列 `{}` 声明为自增，但类型 `{other}` 没有对应的 PG 自增伪类型\
                 （只支持 smallint / integer / bigint ⇒ smallserial / serial / bigserial）",
                col.name
            ),
        }),
    }
}

/// 一列的定义片段：`"name" type [GENERATED ALWAYS AS (expr) STORED] [DEFAULT x] [NOT NULL]`。
///
/// **不写** `PRIMARY KEY` / `UNIQUE`：单列与复合两种情况统一由表级子句表达，避免
/// 「列内写一次、表级再写一次」的重复来源（`CreateTable` 的路径见
/// [`create_table_statements`]；`UNIQUE` 走 [`unique_constraint_name`] 同一套命名）。
fn column_def(col: &ColumnModel, change: &Change, dialect: Dialect) -> Result<String, RenderError> {
    require(&col.name, change, "col.name")?;
    let ty = column_sql_type(col, change, dialect)?;
    check_sql_word(&ty, change)?;
    // 自增伪类型（`serial` 族）**自带**一条 `DEFAULT nextval(…)`，再写一个 DEFAULT 会被 PG
    // 拒（`multiple default values specified`）。在渲染期拦住，比让执行侧去解析 PG 的
    // 报错原文更精确。
    if col.auto_increment && col.default.is_some() {
        return Err(RenderError::Unrenderable {
            kind: change.kind,
            object: change.object.clone(),
            why: format!(
                "列 `{}` 同时声明为自增与有默认值 —— 自增伪类型自带默认值，PG 不接受第二个",
                col.name
            ),
        });
    }

    let mut s = format!("{} {}", quote_ident(&col.name), ty);
    match col.generated.as_deref() {
        Some(expr) => {
            if expr.trim().is_empty() {
                return Err(RenderError::Unrenderable {
                    kind: change.kind,
                    object: change.object.clone(),
                    why: format!("列 `{}` 声明为生成列但表达式为空", col.name),
                });
            }
            if col.default.is_some() {
                // PG 明确禁止：`ERROR: both default and generation expression specified`
                return Err(RenderError::Unrenderable {
                    kind: change.kind,
                    object: change.object.clone(),
                    why: format!("列 `{}` 同时有生成表达式与 DEFAULT（PG 不允许）", col.name),
                });
            }
            s.push_str(&format!(" GENERATED ALWAYS AS ({expr}) STORED"));
            if !col.nullable {
                s.push_str(" NOT NULL");
            }
        },
        None => {
            if let Some(d) = col.default.as_deref() {
                s.push_str(&format!(" DEFAULT {d}"));
            }
            if !col.nullable {
                s.push_str(" NOT NULL");
            }
        },
    }
    Ok(s)
}

/// SQLite 侧的表达式预处理：**必须与比对侧同源**（两步，顺序与
/// `model::normalize_sql_expr` 一致）。
///
/// 1. [`strip_casts`] —— `::type` 不是 SQLite 语法；
/// 2. [`fold_like_operators`] —— PG 的 `!~~` / `~~*` 等**算子记号** SQLite 不认，
///    而 SQLite 的 `LIKE` 就是同一语义（`grp1` 与 `!~~` 在 PG 里本就是同一个算子
///    的两个名字，见该函数文档）。
///
/// ⚠ 只做第 1 步是不够的（2026-09-17 实测）：一条用 PG 写法声明、且 `dialect: None`
/// （**双方言**）的索引会让 SQLite 侧渲染出 `WHERE (name !~~ 'x:%'::text)` ——
/// 剥掉 cast 后仍是 `!~~`，SQLite 解析不了 ⇒ 建索引当场失败。而「声明写哪一家的写法」
/// 由声明作者决定，渲染侧有义务把两家写法都收下来。
fn sqlite_expr(raw: &str) -> String {
    fold_like_operators(&strip_casts(raw))
}

/// 一条索引的 `CREATE [UNIQUE] INDEX` 语句。
///
/// ## 分方言处：SQLite 上的谓词与表达式列要剥 PG 类型转换、折算子记号
///
/// `::type` 不是 SQLite 语法（SQLite 只有 `CAST(x AS type)`）⇒ 原样发过去必然语法错误。
/// 但**不能只当成「让 SQLite 能跑」** —— 真正的理由是**与比对侧同源**：
/// `plan::same_index` 用 `model::normalize_sql_expr`（内含同一个 [`strip_casts`] 与
/// [`fold_like_operators`]）比较「声明文本」与「实况读回的文本」。实况侧是 SQLite 从
/// `sqlite_master.sql` 里读出来的**原始 DDL 文本**，里面不可能有 `::type`，也不可能有
/// `!~~`（写了就建不出来）；若渲染侧不改写，两侧文本永远差那一段 ⇒ 每轮都判成
/// 「索引不同」⇒ 反复 DROP + CREATE，**永不收敛**。
/// 所以这不是「适配」而是「两侧用同一把尺子」，与 [`render`] 模块文档约束 1/2/3 同类。
///
/// 复用一个函数而不是另写一份：两份实现迟早漂移，而漂移的后果恰好就是上面这条。
///
/// ## `USING <method>` 在 SQLite 上要拒（而不是照发）
///
/// SQLite 的 `CREATE INDEX` 语法里没有 `USING` 子句 ⇒ 照发是语法错误，fail-stop。
/// 之所以仍显式拦一道，是为了让报错说**为什么**（「这条声明是 PG 专有的，SQLite
/// 期望集里本不该出现它」），而不是让 SQLite 回一句 `near "gin": syntax error`。
/// 现状：SQLite 期望集里零条非 btree 索引，故本分支目前不可达 —— 它守的是将来。
fn create_index_sql(
    table: &str,
    def: &IndexModel,
    change: &Change,
    dialect: Dialect,
) -> Result<String, RenderError> {
    require(table, change, "table")?;
    require(&def.name, change, "name（索引名）")?;
    if def.cols.is_empty() {
        return Err(RenderError::Unrenderable {
            kind: change.kind,
            object: change.object.clone(),
            why: format!("索引 `{}` 没有任何列", def.name),
        });
    }

    let mut s = String::from("CREATE ");
    if def.unique {
        s.push_str("UNIQUE ");
    }
    s.push_str(&format!("INDEX {} ON {}", quote_ident(&def.name), quote_ident(table)));

    // `method` 为 `None` 或 `btree` 时不写：btree 是 PG 默认值，写不写语义相同，
    // 且 introspect 侧对 btree 做了 `filter(|m| m != "btree")` 归一
    // （`dao/src/reconcile/introspect/pg.rs:233`）⇒ 此处也归一到「不写」，两边才一致。
    if let Some(m) = def.method.as_deref()
        && m != "btree"
    {
        if dialect == Dialect::Sqlite {
            return Err(RenderError::Unrenderable {
                kind: change.kind,
                object: change.object.clone(),
                why: format!(
                    "索引 `{}` 声明了 `USING {m}` —— SQLite 的 CREATE INDEX 没有 USING 子句，\
                     这类声明是 PG 专有的（应挂在 dialect: Some(Postgres) 上，见 extras.rs）",
                    def.name
                ),
            });
        }
        check_sql_word(m, change)?;
        s.push_str(&format!(" USING {m}"));
    }

    let cols: Vec<String> = def
        .cols
        .iter()
        .map(|c| {
            let sql = index_col_sql(c);
            if dialect == Dialect::Sqlite {
                sqlite_expr(&sql)
            } else {
                sql
            }
        })
        .collect();
    s.push_str(&format!(" ({})", cols.join(", ")));

    if let Some(w) = def.where_clause.as_deref()
        && !w.trim().is_empty()
    {
        // 见本函数文档「分方言处」：改写的不是「PG 专有语法」本身，而是**比对侧已经改写过的那一段**
        // —— 两侧必须用同一把尺子，否则「渲染出的文本」永远对不上「实况读回来的文本」。
        let predicate = if dialect == Dialect::Sqlite {
            sqlite_expr(w)
        } else {
            w.to_string()
        };
        s.push_str(&format!(" WHERE {predicate}"));
    }
    Ok(s)
}

/// 单个索引列项：`created_at DESC` → `"created_at" DESC`；`lower(name)` 原样。
///
/// 为什么不能无脑加引号：表达式列（`lower(name)`、`(a + b)`）加引号就成了**列名**，
/// PG 会报「列不存在」。判据是「整项是否为裸标识符 + 可选方向后缀」。
fn index_col_sql(raw: &str) -> String {
    let (head, suffix) = split_direction(raw);
    if is_bare_ident(head) {
        format!("{}{suffix}", quote_ident(head))
    } else {
        raw.to_string()
    }
}

/// 拆出排序方向后缀（长后缀优先，避免 ` DESC` 先命中吃掉 ` NULLS LAST`）。
fn split_direction(raw: &str) -> (&str, &str) {
    const SUFFIXES: &[&str] = &[
        " DESC NULLS FIRST",
        " DESC NULLS LAST",
        " ASC NULLS FIRST",
        " ASC NULLS LAST",
        " DESC",
        " ASC",
    ];
    let up = raw.to_ascii_uppercase();
    for kw in SUFFIXES {
        if up.ends_with(kw) {
            let i = raw.len() - kw.len();
            // `i > 0`：`kw` 以空格开头，切出空 head 说明整个 raw 就是后缀 ⇒ 不动
            if i > 0 {
                return (&raw[..i], &raw[i..]);
            }
        }
    }
    (raw, "")
}

/// 是否形如裸标识符（`[A-Za-z_][A-Za-z0-9_]*`）。
fn is_bare_ident(s: &str) -> bool {
    let mut chars = s.chars();
    match chars.next() {
        Some(c) if c.is_ascii_alphabetic() || c == '_' => {},
        _ => return false,
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// 一条 FK 的引用子句：`FOREIGN KEY (a) REFERENCES "rt" (x) ON DELETE CASCADE …`。
fn fk_clause(def: &FkModel, change: &Change) -> Result<String, RenderError> {
    if def.cols.is_empty() {
        return Err(RenderError::Unrenderable {
            kind: change.kind,
            object: change.object.clone(),
            why: "FK 没有任何列".to_string(),
        });
    }
    require(&def.ref_table, change, "ref_table")?;
    if def.ref_cols.is_empty() {
        return Err(RenderError::Unrenderable {
            kind: change.kind,
            object: change.object.clone(),
            why: format!("FK 指向 `{}` 但没给引用列", def.ref_table),
        });
    }
    if def.cols.len() != def.ref_cols.len() {
        return Err(RenderError::Unrenderable {
            kind: change.kind,
            object: change.object.clone(),
            why: format!(
                "FK 两侧列数不等（{} vs {}）—— PG 会拒绝该 DDL，此处提前判",
                def.cols.len(),
                def.ref_cols.len()
            ),
        });
    }

    let cols: Vec<String> = def.cols.iter().map(|c| quote_ident(c)).collect();
    let ref_cols: Vec<String> = def.ref_cols.iter().map(|c| quote_ident(c)).collect();
    let mut s = format!(
        "FOREIGN KEY ({}) REFERENCES {} ({})",
        cols.join(", "),
        quote_ident(&def.ref_table),
        ref_cols.join(", ")
    );
    for (slot, label) in [(&def.on_delete, "ON DELETE"), (&def.on_update, "ON UPDATE")] {
        if let Some(action) = slot.as_deref() {
            // 词表外的取值（含 introspect 产出的 `UNKNOWN(x)`）**一律报错**：
            // 省略一个动作等于把约束静默放宽成 NO ACTION。
            let sql = fk_action_sql(action).ok_or_else(|| RenderError::UnknownFkAction {
                object: change.object.clone(),
                action: action.to_string(),
            })?;
            s.push_str(&format!(" {label} {sql}"));
        }
    }
    Ok(s)
}

/// 建表的完整语句序列：`CREATE TABLE` + 索引 + 多列 UNIQUE + FK + CHECK。
///
/// 展开顺序即执行顺序：先表、再索引、再约束。索引与约束都作用在这张刚建好的表上，
/// 彼此无依赖。
///
/// ## 分方言处：FK / CHECK 在 SQLite 必须**内联**，不能靠 `ALTER TABLE`
///
/// SQLite 的 `ALTER TABLE` 只有四条（`RENAME TO` / `RENAME COLUMN TO` / `ADD COLUMN` /
/// `DROP COLUMN`）—— **没有 `ADD CONSTRAINT`**。所以同一份声明在两种方言下产出的语句
/// **条数不同**，这是刻意的：
///
/// | 方言 | FK / CHECK 的落点 | 语句数 |
/// |---|---|---|
/// | PG | `ALTER TABLE … ADD CONSTRAINT`（跟在建表之后） | 1 + 索引数 + FK 数 + CHECK 数 |
/// | SQLite | 内联进 `CREATE TABLE ( … )` 的项列表 | 1 + 索引数 |
///
/// ⚠ 两侧的**语句顺序**不能互推：PG 侧 FK 在索引之后、CHECK 在 FK 之后（改动前就是这个
/// 顺序，保持不变）；SQLite 侧它们全在建表语句**内部**。别把某一侧的条数当契约去断言另一侧。
///
/// 约束名两侧都显式给出（`CONSTRAINT <名>`）。SQLite 上名字不是语法必需，但保留它有三个
/// 理由：① 与 PG 侧产出同形，`introspect` 的解析口径只有一处；② 缺名时 `require` 当场报错，
/// 而不是建出一条无名约束（fail-closed）；③ 将来做 SQLite 重建表的 12 步流程时要按名字恢复。
fn create_table_statements(
    t: &TableModel,
    change: &Change,
    dialect: Dialect,
) -> Result<Vec<String>, RenderError> {
    require(&t.name, change, "table.name")?;
    if t.columns.is_empty() {
        return Err(RenderError::Unrenderable {
            kind: change.kind,
            object: change.object.clone(),
            why: format!("表 `{}` 没有任何列", t.name),
        });
    }

    // 主键列的权威来源是 `TableModel.primary_key`（`finalize` 后已排序，顺序不影响
    // 约束语义）。`ColumnModel.primary_key` 只作回退 —— 两者不一致时以表级为准，
    // 因为 `Plan` 的主键对账（`read_primary_keys`）读的也是表级集合。
    let pk: Vec<String> = if t.primary_key.is_empty() {
        t.columns.iter().filter(|c| c.primary_key).map(|c| c.name.clone()).collect()
    } else {
        t.primary_key.clone()
    };
    for col in &pk {
        if t.column(col).is_none() {
            return Err(RenderError::Unrenderable {
                kind: change.kind,
                object: change.object.clone(),
                why: format!("表 `{}` 的主键列 `{col}` 不在列集里", t.name),
            });
        }
    }

    // 生成列排后：STORED 生成列引用同表其它列，被引用列先出现是零成本防御。
    // 用**稳定**排序（`sort_by_key` 是稳定的），故同组内仍保持 `finalize` 的名字序。
    let mut ordered: Vec<&ColumnModel> = t.columns.iter().collect();
    ordered.sort_by_key(|c| c.generated.is_some());

    let mut items: Vec<String> = Vec::new();
    for c in &ordered {
        items.push(column_def(c, change, dialect)?);
    }
    if !pk.is_empty() {
        let cols: Vec<String> = pk.iter().map(|c| quote_ident(c)).collect();
        items.push(format!("PRIMARY KEY ({})", cols.join(", ")));
    }
    // 单列 UNIQUE 内联为**表级 UNIQUE 约束**（不是唯一索引）—— 见模块文档约束 1。
    // 名字省略 ⇒ PG 自动命名 `{表}_{列}_key`，与 `unique_constraint_name` 同源。
    for c in &ordered {
        if c.unique {
            items.push(format!("UNIQUE ({})", quote_ident(&c.name)));
        }
    }
    // FK / CHECK：SQLite 内联进 `items`，PG 走 `ALTER TABLE`。理由见本函数文档。
    //
    // 单列 UNIQUE 上面已内联过，两个方言一样 —— 因为 `ALTER TABLE … ADD CONSTRAINT UNIQUE`
    // 在 SQLite 上同样不存在，内联是唯一选择；而 PG 侧原本就是内联（模块文档约束 1）。
    let mut trailing: Vec<String> = Vec::new();
    let sqlite_inline = dialect == Dialect::Sqlite;
    for fk in &t.fks {
        let name = fk_constraint_name(&t.name, fk);
        let clause = fk_clause(fk, change)?;
        if sqlite_inline {
            items.push(format!("CONSTRAINT {} {clause}", quote_ident(&name)));
        } else {
            trailing.push(format!(
                "ALTER TABLE {} ADD CONSTRAINT {} {clause}",
                quote_ident(&t.name),
                quote_ident(&name)
            ));
        }
    }
    for ck in &t.checks {
        require(&ck.name, change, "check.name（建表内联 CHECK 需要名字）")?;
        if sqlite_inline {
            items.push(format!("CONSTRAINT {} CHECK ({})", quote_ident(&ck.name), ck.expr));
        } else {
            trailing.push(format!(
                "ALTER TABLE {} ADD CONSTRAINT {} CHECK ({})",
                quote_ident(&t.name),
                quote_ident(&ck.name),
                ck.expr
            ));
        }
    }

    let mut stmts =
        vec![format!("CREATE TABLE {} (\n    {}\n)", quote_ident(&t.name), items.join(",\n    "))];

    // `t.indexes` 里的条目：普通索引、partial、GIN、**多列 UNIQUE**。
    // 后者的实况身份是索引名/约束名，统一用 `CREATE [UNIQUE] INDEX` 建，
    // 与 `DropIndex`（`DROP INDEX`）对称。
    for idx in &t.indexes {
        stmts.push(create_index_sql(&t.name, idx, change, dialect)?);
    }
    stmts.extend(trailing);
    Ok(stmts)
}

// ═══════════════════════════════════════════════════════════════════════════
// 测试
// ═══════════════════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reconcile::model::TableModel;

    /// **守卫**：`expected::build(Sqlite)` 的**全部**表都要能真建出来。
    ///
    /// 这是「让引擎接管 SQLite 建表」的出口判据。前身是一条**只打印不断言**的探针
    /// （产物是给决策用的数字）；SQLite 适配做完后升级成守卫 —— 现在有明确期望值可断。
    ///
    /// ⚠ 采样集不能当边过滤（纪律 #250）：这里用的是 `expected::build` 的**全量**表，
    /// 不是挑几张代表性表 —— 采样会漏掉只有个别表才有的形态。
    ///
    /// ## 四条断言各自守什么
    ///
    /// 1. **渲染零失败 + 执行零失败**，张数 == 期望表数。
    /// 2. **没有一条 `ALTER TABLE … ADD CONSTRAINT`**：正向证明 FK / CHECK 真的**内联**了。
    ///    只断「执行成功」不够 —— 若将来 SQLite 支持了那条语法，断言 1 会静默放行，
    ///    而「声明 → 语句」的对应关系已经变了。形态本身要单独钉住。
    /// 3. **没有一条语句含 `::`**：正向证明 PG 类型转换被剥掉了（`::` 不是 SQLite 语法）。
    /// 4. **自增列必须是 rowid 别名**：按 SQLite 自己的判据（单列主键 + 声明类型恰为
    ///    `INTEGER`）逐列核，另加一条真 INSERT 的行为验证（见另一个用例）。
    ///    ⚠ 这一条**语法探针看不见** —— 写成 `serial`（NUMERIC 亲和性）或 `bigint`
    ///    （不是逐字 INTEGER）时**建表照样成功**，坏在运行期：不给 id 的 INSERT 失败。
    ///
    /// 反向断言（防「假绿」）：自增列数、内联 FK/CHECK 数都必须 > 0，否则断言 2/4 是空跑。
    #[tokio::test]
    async fn sqlite_can_create_every_expected_table() {
        use crate::reconcile::expected;
        use sea_orm::{ConnectionTrait, Database, DbBackend, Statement};

        let want = expected::build(Dialect::Sqlite).expect("期望模型应可构建");
        let db = Database::connect("sqlite::memory:").await.expect("内存库应可连");

        let mut rendered_ok = 0usize;
        let mut render_fail: Vec<(String, String)> = Vec::new();
        let mut exec_ok = 0usize;
        let mut exec_fail: Vec<(String, String, String)> = Vec::new();
        let mut exec_fail_kind: std::collections::BTreeMap<String, usize> =
            std::collections::BTreeMap::new();
        let mut stmts_total = 0usize;
        let mut stmts_with_add_constraint: Vec<String> = Vec::new();
        let mut stmts_with_cast: Vec<String> = Vec::new();
        let mut auto_inc_cols = 0usize;
        let mut bad_rowid_alias: Vec<String> = Vec::new();

        for t in &want.tables {
            let ch = Change {
                kind: ChangeKind::CreateTable,
                object: t.name.clone(),
                destructive: false,
                loses_data: false,
                detail: String::new(),
                payload: ChangePayload::Table(t.clone()),
            };
            let stmts = match create_table_statements(t, &ch, Dialect::Sqlite) {
                Ok(s) => {
                    rendered_ok += 1;
                    s
                },
                Err(e) => {
                    render_fail.push((t.name.clone(), e.to_string()));
                    continue;
                },
            };
            stmts_total += stmts.len();
            for s in &stmts {
                if s.to_ascii_uppercase().contains("ADD CONSTRAINT") {
                    stmts_with_add_constraint.push(s.clone());
                }
                if s.contains("::") {
                    stmts_with_cast.push(s.clone());
                }
            }
            // 逐条执行（而不是 `join(";\n")` 一次提交）：一次提交失败时只知道「这张表的
            // 语句集里有问题」，不知道是哪一条、更看不到原文。
            // ⚠ SQLite 允许前向引用（FK 未开启时是惰性的），故建表顺序不影响本用例。
            let mut bad: Option<(usize, String, String)> = None;
            for (i, s) in stmts.iter().enumerate() {
                if let Err(e) = db.execute_unprepared(s).await {
                    bad = Some((i, s.clone(), e.to_string()));
                    break;
                }
            }
            match bad {
                None => exec_ok += 1,
                Some((_i, sql, e)) => {
                    let msg = e.to_string();
                    let key = msg.lines().next().unwrap_or("(空错误)").trim().to_string();
                    *exec_fail_kind.entry(key).or_insert(0) += 1;
                    exec_fail.push((t.name.clone(), sql, msg));
                },
            }

            // 断言 4：逐列核 rowid 别名。
            let expected_auto: Vec<&ColumnModel> =
                t.columns.iter().filter(|c| c.auto_increment).collect();
            if expected_auto.is_empty() {
                continue;
            }
            auto_inc_cols += expected_auto.len();
            let info = db
                .query_all_raw(Statement::from_string(
                    DbBackend::Sqlite,
                    format!("PRAGMA table_info({})", quote_ident(&t.name)),
                ))
                .await
                .expect("PRAGMA table_info 应可读");
            let mut pk_count = 0usize;
            for row in &info {
                let name: String = row.try_get("", "name").expect("PRAGMA 有 name 列");
                let decl: String = row.try_get("", "type").unwrap_or_default();
                let pk: i64 = row.try_get("", "pk").unwrap_or(0);
                if pk > 0 {
                    pk_count += 1;
                }
                if expected_auto.iter().any(|c| c.name == name) {
                    // SQLite 的判据（`dao/src/reconcile/introspect/sqlite.rs:234-236` 用的是同一条）：
                    // 声明类型**逐字是 INTEGER**（大小写不敏感）+ 单列主键。
                    if !decl.trim().eq_ignore_ascii_case("INTEGER") || pk != 1 {
                        bad_rowid_alias
                            .push(format!("{}.{} 声明类型 `{decl}` pk={pk}", t.name, name));
                    }
                }
            }
            if pk_count != 1 {
                bad_rowid_alias
                    .push(format!("{} 的主键列数 = {pk_count}（rowid 别名要求单列）", t.name));
            }
        }

        // ── 诊断输出（断言失败时要能一眼看出是哪张表哪条语句） ──
        println!("=== SQLite 建表守卫 ===");
        println!(
            "期望表数   = {}｜渲染成功 = {rendered_ok}｜渲染失败 = {}",
            want.tables.len(),
            render_fail.len()
        );
        println!("执行成功   = {exec_ok}｜执行失败 = {}", exec_fail.len());
        println!("语句总数   = {stmts_total}");
        println!("自增列数   = {auto_inc_cols}｜非 rowid 别名 = {}", bad_rowid_alias.len());
        println!("含 ADD CONSTRAINT 的语句 = {}", stmts_with_add_constraint.len());
        println!("含 `::` 的语句 = {}", stmts_with_cast.len());
        // 失败**语句原文**才是能定位的东西 —— 只给错误消息等于让人去猜语法。
        // 按错误归类各取**一条代表**：同类错误只占一行，别的种类才看得见
        // （此前 39 条 `near "CONSTRAINT"` 会把唯一那条 `unrecognized token ":"` 挤掉）。
        let mut seen: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
        for (n, sql, r) in &exec_fail {
            let key = r.lines().next().unwrap_or("").trim().to_string();
            if seen.insert(key.clone()) {
                println!(
                    "  ◆ 执行失败 {n}\n      语句: {}\n      错误: {key}",
                    sql.replace('\n', " ")
                );
            }
        }
        for (n, r) in render_fail.iter().take(10) {
            println!("  ◆ 渲染失败 {n} :: {r}");
        }
        for s in stmts_with_add_constraint.iter().take(3) {
            println!("  ◆ 不该出现的 ADD CONSTRAINT: {}", s.replace('\n', " "));
        }
        for s in stmts_with_cast.iter().take(3) {
            println!("  ◆ 不该残留的 `::`: {}", s.replace('\n', " "));
        }
        for b in bad_rowid_alias.iter().take(10) {
            println!("  ◆ 非 rowid 别名: {b}");
        }

        assert!(render_fail.is_empty(), "{} 张表渲染失败，见上", render_fail.len());
        assert!(exec_fail.is_empty(), "{} 张表执行失败，见上", exec_fail.len());
        assert_eq!(rendered_ok, want.tables.len(), "渲染成功数 ≠ 期望表数");
        assert_eq!(exec_ok, want.tables.len(), "执行成功数 ≠ 期望表数");
        assert!(
            stmts_with_add_constraint.is_empty(),
            "SQLite 侧出现了 {} 条 `ADD CONSTRAINT` —— FK/CHECK 必须内联进 CREATE TABLE",
            stmts_with_add_constraint.len()
        );
        assert!(
            stmts_with_cast.is_empty(),
            "SQLite 侧残留 {} 条 `::` 类型转换",
            stmts_with_cast.len()
        );
        assert!(bad_rowid_alias.is_empty(), "{} 个自增列不是 rowid 别名", bad_rowid_alias.len());
        // 反向断言：两个「可能空跑」的判据必须真的被行使过（否则本用例给的是假绿）。
        assert!(auto_inc_cols > 0, "期望模型里没有自增列 —— 断言 4 是空跑");
        assert!(stmts_total > want.tables.len(), "语句数 == 表数 ⇒ 索引/FK/CHECK 一条都没展开");
    }

    /// `SQLite` 的自增列必须**真能自动赋值** —— 用真 INSERT 验，不靠 DDL 文本推断。
    ///
    /// 与 `sqlite_can_create_every_expected_table` 的断言 4 互补：那条核的是 SQLite
    /// **文档里的判据**（声明类型逐字 INTEGER + 单列主键），这条核的是**行为**。
    /// 两者都要 —— 只核文本等于用「引擎自己的实况侧判据」去验「引擎自己的渲染」，
    /// 而实况侧那个判据本身也可能理解错（纪律：判据与被测对象不能同源到只剩一处）。
    ///
    /// 手工搭一张最小表而不是从 183 张里挑一张：期望模型里的自增表普遍还有别的 NOT NULL
    /// 列，`INSERT` 省不掉它们，于是失败原因会混进「缺列」而不是「没自增」。
    #[tokio::test]
    async fn sqlite_autoincrement_column_really_auto_assigns() {
        use sea_orm::{ConnectionTrait, Database, DbBackend, Statement};

        let mut t = TableModel::new("autoinc_probe");
        let mut id = ColumnModel::new("id", "integer", false);
        id.auto_increment = true;
        id.primary_key = true;
        t.columns.push(id);
        t.columns.push(ColumnModel::new("label", "text", true));
        t.primary_key = vec!["id".into()];

        let change = ch(ChangeKind::CreateTable, "autoinc_probe", ChangePayload::Table(t.clone()));
        let stmts = create_table_statements(&t, &change, Dialect::Sqlite).expect("应可渲染");
        let db = Database::connect("sqlite::memory:").await.expect("内存库应可连");
        for s in &stmts {
            db.execute_unprepared(s).await.unwrap_or_else(|e| panic!("执行失败：{s}\n{e}"));
        }
        // 两行都不给 id：第一行拿 rowid 1，第二行拿 rowid 2 ⇒ 证明真的自增。
        for label in ["a", "b"] {
            db.execute_unprepared(&format!(
                "INSERT INTO \"autoinc_probe\" (\"label\") VALUES ('{label}')"
            ))
            .await
            .expect("不给 id 的 INSERT 必须成功（否则该列不是 rowid 别名）");
        }
        let rows = db
            .query_all_raw(Statement::from_string(
                DbBackend::Sqlite,
                "SELECT id FROM \"autoinc_probe\" ORDER BY id".to_string(),
            ))
            .await
            .expect("查询应成功");
        let ids: Vec<i64> = rows.iter().map(|r| r.try_get("", "id").expect("id 列")).collect();
        assert_eq!(ids, vec![1, 2], "自增列没有按 rowid 递增赋值");
    }

    /// SQLite 的自增列**不许**被映射成 `serial` / `bigint` —— 两者都不是 rowid 别名。
    ///
    /// 这是上面那个行为用例的**单位级**对应物：行为用例证明「现在是好的」，本用例把
    /// 两个已知的错误写法钉死，使「把 `INTEGER` 改回 `col.sql_type`（= `integer`）」
    /// 这类改动不至于悄悄溜过（`integer` 其实也合格，但 `serial` 不合格）。
    #[test]
    fn sqlite_autoincrement_maps_to_exactly_integer_not_serial() {
        let mut c = ColumnModel::new("id", "integer", false);
        c.auto_increment = true;
        // PG：映射成伪类型 serial。
        assert_eq!(
            column_sql_type(
                &c,
                &ch(ChangeKind::AddColumn, "t.id", col_payload("t", c.clone())),
                Dialect::Postgres
            )
            .expect("PG 应可渲染"),
            "serial"
        );
        // SQLite：必须逐字 INTEGER（大小写不敏感地等于 "INTEGER"）。
        let lite = column_sql_type(
            &c,
            &ch(ChangeKind::AddColumn, "t.id", col_payload("t", c.clone())),
            Dialect::Sqlite,
        )
        .expect("SQLite 应可渲染");
        assert!(
            lite.eq_ignore_ascii_case("INTEGER"),
            "SQLite 自增列类型应为 INTEGER，实际 `{lite}`"
        );
        assert!(!lite.eq_ignore_ascii_case("serial"));

        // 非整型类别声明的自增列 ⇒ 报错（fail-closed，不硬写 INTEGER 静默改语义）。
        let mut bad = ColumnModel::new("id", "text", false);
        bad.auto_increment = true;
        let e = column_sql_type(
            &bad,
            &ch(ChangeKind::AddColumn, "t.id", col_payload("t", bad.clone())),
            Dialect::Sqlite,
        )
        .expect_err("text 自增列应被拒");
        assert!(matches!(e, RenderError::Unrenderable { .. }), "{e:?}");
    }

    // ── SQLite 方言能力边界 ──

    /// **穷举守卫**：每个 [`ChangeKind`] 都必须被显式分到「SQLite 原生支持」或「不支持」
    /// 两堆里的一堆。
    ///
    /// 为什么是穷举而不是几个抽样：`sqlite_supports_natively` 是**白名单** —— 新增
    /// [`ChangeKind`] 时若没人动它，新类别默认落到「拒绝」侧（安全方向）。但「默认安全」
    /// 会掩盖另一件事：**它可能本来该支持**。所以这里逐项对照一份独立的期望清单，
    /// 新增类别时必然红灯，逼人做一次二元决策。
    ///
    /// 判据形态与 `plan.rs::every_change_kind_is_classified_for_bootstrap` 一致
    /// （同一套手法用在两处，避免「一处穷举、一处抽样」的口径漂移）。
    #[test]
    fn every_change_kind_is_classified_for_sqlite() {
        const SQLITE_NATIVE: [ChangeKind; 6] = [
            ChangeKind::CreateTable,
            ChangeKind::DropTable,
            ChangeKind::AddColumn,
            ChangeKind::RenameColumn,
            ChangeKind::CreateIndex,
            ChangeKind::DropIndex,
        ];
        assert_eq!(
            SQLITE_NATIVE.len() + 15,
            ChangeKind::ALL.len(),
            "两堆之和必须覆盖 ALL（21 类）"
        );

        for k in ChangeKind::ALL {
            let expected = SQLITE_NATIVE.contains(&k);
            assert_eq!(
                sqlite_supports_natively(k),
                expected,
                "`{}` 的 SQLite 归类与本用例的期望不一致 —— 新增/改动 ChangeKind 时\
                 必须在此做一次二元决策：它有没有 SQLite 原生 DDL？没有就该走重建表 12 步流程",
                k.as_str()
            );
        }
    }

    /// 白名单里的 6 类在 SQLite 上**真的**能渲染出来（不抛 `UnsupportedDialect`）。
    ///
    /// 与穷举守卫互补：那条只比集合成员，这条走一遍 `render()` 真路径 —— 集合对了但
    /// 分支里写错分支（比如某类落进了 PG-only 的 `USING` / `ADD CONSTRAINT` 模板）时，
    /// 只有本用例会红。
    #[test]
    fn sqlite_native_kinds_actually_render() {
        let mut t = TableModel::new("t");
        t.columns.push(ColumnModel::new("id", "integer", false));
        t.primary_key = vec!["id".into()];

        let cases: Vec<(ChangeKind, ChangePayload)> = vec![
            (ChangeKind::CreateTable, ChangePayload::Table(t)),
            (ChangeKind::DropTable, ChangePayload::Bare),
            (ChangeKind::AddColumn, col_payload("t", col("c", "text"))),
            (
                ChangeKind::RenameColumn,
                ChangePayload::Rename { table: "t".into(), from: "a".into(), to: "b".into() },
            ),
            (
                ChangeKind::CreateIndex,
                ChangePayload::Index {
                    table: "t".into(),
                    def: IndexModel::new("idx_t_c", vec!["c".into()], false),
                },
            ),
            (
                ChangeKind::DropIndex,
                ChangePayload::Index {
                    table: "t".into(),
                    def: IndexModel::new("idx_t_c", vec!["c".into()], false),
                },
            ),
        ];
        assert_eq!(cases.len(), 6, "本用例应覆盖白名单里的全部 6 类");
        for (kind, payload) in cases {
            let r = render(&ch(kind, "t", payload), Dialect::Sqlite);
            assert!(r.is_ok(), "`{}` 在 SQLite 上应可渲染，实际：{:?}", kind.as_str(), r.err());
        }
    }

    /// 想**保留**的行为：SQLite 不支持的类别必须**报错**，而且错误信息要说清
    /// 「不支持」这件事本身（`UnsupportedDialect`），不是抛别的错或静默产出。
    ///
    /// 前身是 `sqlite_is_explicitly_unsupported`，它拿 `AddColumn` 当样本 —— SQLite 适配后
    /// `AddColumn` 变成**支持**的，那条用例因此红灯。这是**正确的红灯**（被测行为变了），
    /// 处置是换成仍然不支持的类别。
    ///
    /// ⚠ 这里**不再**断言错误文案里含「P4 只做 PostgreSQL」：那句口径已过期（P4 早过了，
    /// 现在支持 6 类）。把过期文案留在断言里，等于给下一个改文案的人设一个必须撒谎的门槛。
    /// 真正该守的是**类别 → 错误类型**的映射，文案由 `UnsupportedDialect` 的 `Display` 统一给。
    #[test]
    fn sqlite_unsupported_kinds_are_refused_not_silently_rendered() {
        // 三类分别代表三种「PG 有、SQLite 没有」的形态：
        // 改列类型 / 改非空 / 加 CHECK —— 在 SQLite 上都要走重建表 12 步流程。
        let cases: Vec<(ChangeKind, ChangePayload)> = vec![
            (ChangeKind::AlterColumnType, col_payload("t", col("c", "text"))),
            (ChangeKind::SetNotNull, col_payload("t", col("c", "text"))),
            (
                ChangeKind::AddCheck,
                ChangePayload::Check {
                    table: "t".into(),
                    def: CheckModel::new("ck_t_c", "c <> ''"),
                },
            ),
        ];
        for (kind, payload) in cases {
            let e = render(&ch(kind, "t", payload), Dialect::Sqlite).expect_err(&format!(
                "`{}` 在 SQLite 上没有原生 DDL，必须返回 Err 而不是渲染出一条跑不通的 SQL",
                kind.as_str()
            ));
            assert!(
                matches!(e, RenderError::UnsupportedDialect { dialect: Dialect::Sqlite, .. }),
                "`{}` 的错误类型应为 UnsupportedDialect，实际 {e:?}",
                kind.as_str()
            );
        }
    }

    fn ch(kind: ChangeKind, object: &str, payload: ChangePayload) -> Change {
        Change {
            kind,
            object: object.to_string(),
            destructive: kind.destructive(),
            loses_data: kind.loses_data(),
            detail: String::new(),
            payload,
        }
    }

    fn col(name: &str, ty: &str) -> ColumnModel {
        ColumnModel::new(name, ty, true)
    }

    fn col_payload(table: &str, c: ColumnModel) -> ChangePayload {
        ChangePayload::Column { table: table.to_string(), col: c }
    }

    fn one(kind: ChangeKind, object: &str, payload: ChangePayload) -> String {
        let r = render(&ch(kind, object, payload), Dialect::Postgres).expect("应渲染成功");
        assert_eq!(r.statements.len(), 1, "本用例期望单条语句：{r:?}");
        r.statements.into_iter().next().expect("非空")
    }

    /// 期望**多条**语句的用例（`CreateTable` / `AlterGenerated` / `AlterColumnType`）。
    ///
    /// 与 `one` 分开而不是把 `one` 改成返回 `Vec`：`one` 里那句 `len() == 1` **本身就是
    /// 判据** —— 它守住「本来就该单条的类别不会悄悄变成多条」。
    fn all(kind: ChangeKind, object: &str, payload: ChangePayload) -> Vec<String> {
        render(&ch(kind, object, payload), Dialect::Postgres).expect("应渲染成功").statements
    }

    fn err(kind: ChangeKind, object: &str, payload: ChangePayload) -> RenderError {
        render(&ch(kind, object, payload), Dialect::Postgres).expect_err("应渲染失败")
    }

    // ── 分方言落点（SQLite vs PG） ──

    /// 同一份表定义，两个方言的 FK / CHECK **落点必须不同**：SQLite 内联、PG 走 `ALTER TABLE`。
    ///
    /// 守的是 `create_table_statements` 的分方言分支。只验「两边都能建」是不够的：
    /// 若有人把 PG 也改成内联，PG 侧**照样建得出来** —— 但 `DropForeignKey` / `DropCheck`
    /// （走 `ALTER TABLE … DROP CONSTRAINT`）与建表形态不再对称，退步是静默的。
    /// 反过来把 SQLite 改回统一 `ALTER TABLE` 会被全量守卫拦下（SQLite 报语法错）。
    #[test]
    fn fk_and_check_are_inlined_only_on_sqlite() {
        let t = sample_table();
        let pg = render(
            &ch(ChangeKind::CreateTable, "child", ChangePayload::Table(t.clone())),
            Dialect::Postgres,
        )
        .expect("PG 应渲染");
        let lite =
            render(&ch(ChangeKind::CreateTable, "child", ChangePayload::Table(t)), Dialect::Sqlite)
                .expect("SQLite 应渲染");

        // PG：建表里没有 FK/CHECK，它们各占一条 `ALTER TABLE … ADD CONSTRAINT`。
        assert!(!pg.statements[0].contains("FOREIGN KEY"), "{}", pg.statements[0]);
        assert!(!pg.statements[0].contains("CHECK ("), "{}", pg.statements[0]);
        assert!(
            pg.statements.iter().filter(|s| s.contains("ADD CONSTRAINT")).count() == 2,
            "PG 侧 FK + CHECK 应各占一条：{:#?}",
            pg.statements
        );
        // SQLite：FK/CHECK 内联进建表语句，之后只剩索引语句。
        assert!(lite.statements[0].contains("FOREIGN KEY"), "{}", lite.statements[0]);
        assert!(lite.statements[0].contains("CHECK ("), "{}", lite.statements[0]);
        assert!(
            !lite.statements.iter().any(|s| s.contains("ADD CONSTRAINT")),
            "SQLite 上不该出现 ALTER TABLE … ADD CONSTRAINT：{:#?}",
            lite.statements
        );
        // 条数差 == FK 数 + CHECK 数（样本表各 1）—— 用差值而不是写死 4/2，
        // 这样样本表增删对象时本断言仍然指向「差值」这个真正的性质。
        assert_eq!(
            pg.statements.len() - lite.statements.len(),
            2,
            "PG {:#?}\nSQLite {:#?}",
            pg.statements,
            lite.statements
        );
    }

    /// SQLite 上索引谓词里的 PG 类型转换必须被剥掉 —— 剥的正是**比对侧已经剥过的那一段**。
    ///
    /// 只断「SQLite 语法能过」不够。真正的风险是**不收敛**：实况侧从 `sqlite_master.sql`
    /// 读回的文本不可能含 `::text`（写了就建不出来），而 `plan::same_index` 两侧都过
    /// `normalize_sql_expr`（内含 `strip_casts`）⇒ 声明侧若不剥，归一后仍差一段 `::text`
    /// ⇒ 每轮判「索引不同」⇒ 反复 DROP + CREATE。
    ///
    /// 同时断言 **PG 侧保留 `::`**：防止有人图省事把「剥 cast」提到方言判断之外 ——
    /// 那会让 PG 侧渲染文本与 `pg_get_indexdef` 的文本不一致，同样是不收敛。
    #[test]
    fn index_predicate_casts_are_stripped_only_on_sqlite() {
        let mut def = IndexModel::new("idx_t_c", vec!["c".into()], false);
        def.where_clause = Some("(c = 'paused'::text)".to_string());
        let payload = || ChangePayload::Index { table: "t".into(), def: def.clone() };

        let pg = one(ChangeKind::CreateIndex, "idx_t_c", payload());
        assert!(pg.contains("'paused'::text"), "PG 侧应原样保留 cast：{pg}");

        let lite = render(&ch(ChangeKind::CreateIndex, "idx_t_c", payload()), Dialect::Sqlite)
            .expect("SQLite 应渲染");
        let sql = &lite.statements[0];
        assert!(!sql.contains("::"), "SQLite 侧不该残留 `::`：{sql}");
        assert!(sql.contains("(c = 'paused')"), "剥掉 cast 后应剩原字面量：{sql}");
    }

    /// SQLite 上 `USING <method>` 要**报错**而不是照发。
    ///
    /// SQLite 的 `CREATE INDEX` 语法里没有 `USING` 子句 ⇒ 照发必然语法错。显式拦一道
    /// 是为了让报错说清**为什么**（这条声明是 PG 专有的），而不是让 SQLite 回一句
    /// `near "gin": syntax error`。
    ///
    /// 现状：SQLite 期望集里零条非 btree 索引 ⇒ 本分支当前不可达，它守的是将来
    /// （有人给某条索引漏了 `dialect: Some(Postgres)`）。
    #[test]
    fn sqlite_rejects_index_access_methods() {
        let mut def = IndexModel::new("idx_t_tsv", vec!["content_tsv".into()], false);
        def.method = Some("gin".to_string());
        let e = render(
            &ch(
                ChangeKind::CreateIndex,
                "idx_t_tsv",
                ChangePayload::Index { table: "t".into(), def },
            ),
            Dialect::Sqlite,
        )
        .expect_err("SQLite 不该接受 USING gin");
        assert!(matches!(e, RenderError::Unrenderable { .. }), "{e:?}");
        assert!(e.to_string().contains("USING"), "报错要点出是 USING 的问题：{e}");
    }

    // ── 标识符 / 词元 ──

    #[test]
    fn identifier_quoting_doubles_inner_quotes() {
        assert_eq!(quote_ident("plain"), "\"plain\"");
        assert_eq!(quote_ident("Mixed_Case"), "\"Mixed_Case\"");
        assert_eq!(quote_ident("we\"ird"), "\"we\"\"ird\"");
    }

    /// 白名单必须**恰好**放过生产库实测的类型名，并挡住注入字符。
    #[test]
    fn sql_word_whitelist_is_exact() {
        for ok in [
            "text",
            "bigint",
            "double precision",
            "numeric(10,2)",
            "vector(1024)",
            "text[]",
            "timestamp without time zone",
            "tsvector",
            "gin",
        ] {
            assert!(sql_word_is_safe(ok), "{ok} 应放行");
        }
        for bad in
            ["text; DROP TABLE users", "text'", "te\"xt", "text--c", "", "   ", "text\n", "text\\"]
        {
            assert!(!sql_word_is_safe(bad), "{bad:?} 应拒绝");
        }
    }

    /// 类型名带非法字符 ⇒ 报错（不静默拼接）。
    #[test]
    fn unsafe_type_name_is_rejected() {
        let e = err(ChangeKind::AddColumn, "t.c", col_payload("t", col("c", "text; DROP TABLE t")));
        assert!(matches!(e, RenderError::UnsafeSqlWord { .. }), "{e:?}");
    }

    #[test]
    fn ident_truncation_keeps_utf8_boundary_and_63_bytes() {
        assert_eq!(truncate_ident("short"), "short");
        let long = "a".repeat(100);
        assert_eq!(truncate_ident(&long).len(), 63);
        // 多字节字符不能被切坏
        let cjk = "表".repeat(40); // 每个 3 字节
        let cut = truncate_ident(&cjk);
        assert!(cut.len() <= 63);
        assert_eq!(cut.len() % 3, 0, "必须落在字符边界上");
    }

    /// 派生名与 PG 默认命名规则一致，且建/删两处同源。
    #[test]
    fn unique_constraint_name_is_the_pg_default() {
        assert_eq!(unique_constraint_name("users", "email"), "users_email_key");
        // 超长时先截断，保证「建」与「删」拼同一个名字
        let long_table = "t".repeat(60);
        let n = unique_constraint_name(&long_table, "col");
        assert_eq!(n.len(), 63);
    }

    /// FK 动作词表必须与 `introspect::fk_action` 的产出逐字对齐。
    #[test]
    fn fk_action_word_table_mirrors_introspect() {
        // introspect 侧可能产出的全部取值（`introspect/pg.rs::fk_action`）
        assert_eq!(fk_action_sql("NoAction"), Some("NO ACTION"));
        assert_eq!(fk_action_sql("Restrict"), Some("RESTRICT"));
        assert_eq!(fk_action_sql("Cascade"), Some("CASCADE"));
        assert_eq!(fk_action_sql("SetNull"), Some("SET NULL"));
        assert_eq!(fk_action_sql("SetDefault"), Some("SET DEFAULT"));
        // 未知编码与未知取值都必须拒绝（不能省略动作）
        assert_eq!(fk_action_sql("UNKNOWN(x)"), None);
        assert_eq!(fk_action_sql("cascade"), None, "大小写不同即不认，避免猜");
        assert_eq!(fk_action_sql(""), None);
    }

    /// 索引列项：裸标识符加引号并保留方向；表达式原样。
    #[test]
    fn index_columns_quote_idents_but_keep_expressions() {
        assert_eq!(index_col_sql("created_at"), "\"created_at\"");
        assert_eq!(index_col_sql("created_at DESC"), "\"created_at\" DESC");
        assert_eq!(index_col_sql("a ASC NULLS LAST"), "\"a\" ASC NULLS LAST");
        assert_eq!(index_col_sql("priority DESC NULLS FIRST"), "\"priority\" DESC NULLS FIRST");
        // 表达式不能加引号（加了会被当成不存在的列名）
        assert_eq!(index_col_sql("lower(name)"), "lower(name)");
        assert_eq!(index_col_sql("(a + b)"), "(a + b)");
        // 多词但不是方向后缀 ⇒ 不动（防把 `a b` 误拆）
        assert_eq!(index_col_sql("some thing"), "some thing");
    }

    // ── 逐类渲染 ──

    #[test]
    fn renames_render_per_object_type() {
        assert_eq!(
            one(
                ChangeKind::RenameColumn,
                "t.new",
                ChangePayload::Rename { table: "t".into(), from: "old".into(), to: "new".into() }
            ),
            "ALTER TABLE \"t\" RENAME COLUMN \"old\" TO \"new\""
        );
        assert_eq!(
            one(
                ChangeKind::RenameIndex,
                "i2",
                ChangePayload::Rename { table: "t".into(), from: "i1".into(), to: "i2".into() }
            ),
            "ALTER INDEX \"i1\" RENAME TO \"i2\""
        );
        assert_eq!(
            one(
                ChangeKind::RenameForeignKey,
                "t.col",
                ChangePayload::Rename { table: "t".into(), from: "fk_a".into(), to: "fk_b".into() }
            ),
            "ALTER TABLE \"t\" RENAME CONSTRAINT \"fk_a\" TO \"fk_b\""
        );
    }

    /// 实况侧匿名对象（空名）**不能**渲染成 `RENAME TO ""` —— 必须报错。
    #[test]
    fn rename_with_empty_from_is_rejected() {
        let e = err(
            ChangeKind::RenameColumn,
            "t.c",
            ChangePayload::Rename { table: "t".into(), from: String::new(), to: "c".into() },
        );
        assert!(matches!(e, RenderError::EmptyIdentifier { field: "from（旧列名）", .. }), "{e:?}");
    }

    #[test]
    fn drops_render_and_drop_table_never_cascades() {
        assert_eq!(
            one(
                ChangeKind::DropCheck,
                "t.ck",
                ChangePayload::Check {
                    table: "t".into(),
                    def: CheckModel::new("ck_t_n", "n >= 0")
                }
            ),
            "ALTER TABLE \"t\" DROP CONSTRAINT \"ck_t_n\""
        );
        assert_eq!(
            one(
                ChangeKind::DropIndex,
                "idx_a",
                ChangePayload::Index {
                    table: "t".into(),
                    def: IndexModel::new("idx_a", vec!["a".into()], false)
                }
            ),
            "DROP INDEX \"idx_a\""
        );

        let t =
            render(&ch(ChangeKind::DropTable, "orphan", ChangePayload::Bare), Dialect::Postgres)
                .expect("应渲染");
        assert_eq!(t.statements, vec!["DROP TABLE \"orphan\""]);
        assert!(
            !t.statements[0].contains("CASCADE"),
            "DROP TABLE 绝不能带 CASCADE：会把计划外的依赖对象一起删掉"
        );
    }

    /// 列级变更的四种形态。
    #[test]
    fn column_level_statements() {
        assert_eq!(
            one(ChangeKind::AddColumn, "t.c", col_payload("t", col("c", "bigint"))),
            "ALTER TABLE \"t\" ADD COLUMN \"c\" bigint"
        );
        let mut nn = col("c", "text");
        nn.nullable = false;
        nn.default = Some("'x'".into());
        assert_eq!(
            one(ChangeKind::AddColumn, "t.c", col_payload("t", nn.clone())),
            "ALTER TABLE \"t\" ADD COLUMN \"c\" text DEFAULT 'x' NOT NULL"
        );
        // ⚠ `AlterColumnType` 展开成**两条**（DROP DEFAULT + TYPE USING）——
        // 见该分支里那段实测记录：列上若有默认值，PG 会拒绝对它做赋值转换。
        assert_eq!(
            all(ChangeKind::AlterColumnType, "t.c", col_payload("t", col("c", "json"))),
            vec![
                "ALTER TABLE \"t\" ALTER COLUMN \"c\" DROP DEFAULT".to_string(),
                "ALTER TABLE \"t\" ALTER COLUMN \"c\" TYPE json USING \"c\"::json".to_string(),
            ]
        );
        assert_eq!(
            one(ChangeKind::SetNotNull, "t.c", col_payload("t", col("c", "text"))),
            "ALTER TABLE \"t\" ALTER COLUMN \"c\" SET NOT NULL"
        );
        assert_eq!(
            one(ChangeKind::DropNotNull, "t.c", col_payload("t", col("c", "text"))),
            "ALTER TABLE \"t\" ALTER COLUMN \"c\" DROP NOT NULL"
        );
        assert_eq!(
            one(ChangeKind::DropDefault, "t.c", col_payload("t", col("c", "text"))),
            "ALTER TABLE \"t\" ALTER COLUMN \"c\" DROP DEFAULT"
        );
        assert_eq!(
            one(ChangeKind::DropColumn, "t.c", col_payload("t", col("c", "text"))),
            "ALTER TABLE \"t\" DROP COLUMN \"c\""
        );
    }

    /// ⚠ `AlterColumnType` **必须**带 `USING`：不带时 PG 对 `jsonb → json` 这类会拒绝。
    #[test]
    fn alter_type_always_uses_explicit_cast() {
        let stmts = all(ChangeKind::AlterColumnType, "t.c", col_payload("t", col("c", "bigint")));
        assert!(stmts.iter().any(|s| s.contains("USING \"c\"::bigint")), "{stmts:?}");
    }

    /// ⚠⚠ P4 **首次真库执行**实测缺陷的守卫（`narrative_structures.is_template`）。
    ///
    /// 只断言「序列里存在一条 DROP DEFAULT」不够 —— 那也能被「排在 TYPE 之后」蒙混，
    /// 而正是那个顺序导致了 PG 报 `字段 "is_template" 的默认值不能转换成类型 boolean`。
    /// 所以这里断言的是**相对顺序**。
    #[test]
    fn alter_type_drops_default_before_changing_type() {
        let stmts = all(ChangeKind::AlterColumnType, "t.c", col_payload("t", col("c", "boolean")));
        let i_drop = stmts.iter().position(|s| s.ends_with("DROP DEFAULT"));
        let i_type = stmts.iter().position(|s| s.contains(" TYPE "));
        let i_drop = i_drop.unwrap_or_else(|| panic!("没有 DROP DEFAULT：{stmts:?}"));
        let i_type = i_type.unwrap_or_else(|| panic!("没有 TYPE 语句：{stmts:?}"));
        assert!(
            i_drop < i_type,
            "DROP DEFAULT 必须排在 TYPE 之前（实测就是这个顺序问题让 461 条被 fail-stop）：\
             {i_drop} vs {i_type}，{stmts:?}"
        );
    }

    /// 期望侧**有**默认值时，转换完用**期望侧**的值补回 —— 它已是新类型的值，无需转换。
    #[test]
    fn alter_type_restores_the_expected_default() {
        let mut c = col("c", "boolean");
        c.default = Some("false".into());
        let stmts = all(ChangeKind::AlterColumnType, "t.c", col_payload("t", c));
        assert_eq!(stmts.len(), 3, "{stmts:?}");
        assert_eq!(
            stmts[2], "ALTER TABLE \"t\" ALTER COLUMN \"c\" SET DEFAULT false",
            "补回的默认值必须原样用期望侧的值：{stmts:?}"
        );
    }

    /// 反向：期望侧**无**默认值时**不许**自作主张补一个 `SET DEFAULT`。
    #[test]
    fn alter_type_without_expected_default_adds_nothing() {
        let stmts = all(ChangeKind::AlterColumnType, "t.c", col_payload("t", col("c", "boolean")));
        assert_eq!(stmts.len(), 2, "{stmts:?}");
        assert!(!stmts.iter().any(|s| s.contains("SET DEFAULT")), "{stmts:?}");
    }

    /// `SetDefault` 但列为 None ⇒ 报错（建模矛盾不该产出 `SET DEFAULT ` 空尾）。
    #[test]
    fn set_default_without_value_is_rejected() {
        let e = err(ChangeKind::SetDefault, "t.c", col_payload("t", col("c", "text")));
        assert!(matches!(e, RenderError::Unrenderable { .. }), "{e:?}");
    }

    /// 生成列：`ADD COLUMN` 带 `GENERATED ALWAYS AS … STORED`；`AlterGenerated` 展开两条。
    #[test]
    fn generated_column_statements() {
        let mut g = col("content_tsv", "tsvector");
        g.generated = Some("to_tsvector('simple', COALESCE(content, ''))".into());
        assert_eq!(
            one(ChangeKind::AddColumn, "t.content_tsv", col_payload("t", g.clone())),
            "ALTER TABLE \"t\" ADD COLUMN \"content_tsv\" tsvector \
             GENERATED ALWAYS AS (to_tsvector('simple', COALESCE(content, ''))) STORED"
        );

        let r = render(
            &ch(ChangeKind::AlterGenerated, "t.content_tsv", col_payload("t", g)),
            Dialect::Postgres,
        )
        .expect("应渲染");
        assert_eq!(r.statements.len(), 2, "必须展开成 DROP + ADD");
        assert!(r.statements[0].starts_with("ALTER TABLE \"t\" DROP COLUMN \"content_tsv\""));
        assert!(r.statements[1].contains("GENERATED ALWAYS AS"));
        assert!(r.notes.iter().any(|n| n.contains("级联删除")), "{:?}", r.notes);
    }

    /// 生成列 + DEFAULT 同时存在 ⇒ 报错（PG 会拒绝该 DDL）。
    #[test]
    fn generated_with_default_is_rejected() {
        let mut g = col("tsv", "tsvector");
        g.generated = Some("to_tsvector(a)".into());
        g.default = Some("''".into());
        let e = err(ChangeKind::AddColumn, "t.tsv", col_payload("t", g));
        assert!(matches!(e, RenderError::Unrenderable { .. }), "{e:?}");
    }

    /// ⚠ 同源点 1：`AddUnique` 建**约束**，不是唯一索引。
    #[test]
    fn add_unique_builds_a_constraint_not_an_index() {
        let mut c = col("email", "text");
        c.unique = true;
        let sql = one(ChangeKind::AddUnique, "t.email", col_payload("t", c));
        assert_eq!(sql, "ALTER TABLE \"t\" ADD CONSTRAINT \"t_email_key\" UNIQUE (\"email\")");
        assert!(!sql.contains("INDEX"), "建成唯一索引会让列标志永远为 false：{sql}");
        // 删也必须走 DROP CONSTRAINT（DROP INDEX 对约束型 UNIQUE 无效）
        let drop = one(ChangeKind::DropUnique, "t.email", col_payload("t", col("email", "text")));
        assert_eq!(drop, "ALTER TABLE \"t\" DROP CONSTRAINT \"t_email_key\"");
    }

    /// 索引：`USING gin`、partial 谓词、unique、btree 不写。
    #[test]
    fn create_index_variants() {
        assert_eq!(
            one(
                ChangeKind::CreateIndex,
                "idx_t_a",
                ChangePayload::Index {
                    table: "t".into(),
                    def: IndexModel::new("idx_t_a", vec!["a".into()], false)
                }
            ),
            "CREATE INDEX \"idx_t_a\" ON \"t\" (\"a\")"
        );
        let mut gin = IndexModel::new("idx_t_tsv", vec!["content_tsv".into()], false);
        gin.method = Some("gin".into());
        assert_eq!(
            one(
                ChangeKind::CreateIndex,
                "idx_t_tsv",
                ChangePayload::Index { table: "t".into(), def: gin }
            ),
            "CREATE INDEX \"idx_t_tsv\" ON \"t\" USING gin (\"content_tsv\")"
        );
        let mut part = IndexModel::new("idx_t_p", vec!["a".into()], false);
        part.where_clause = Some("(a IS NOT NULL)".into());
        assert_eq!(
            one(
                ChangeKind::CreateIndex,
                "idx_t_p",
                ChangePayload::Index { table: "t".into(), def: part }
            ),
            "CREATE INDEX \"idx_t_p\" ON \"t\" (\"a\") WHERE (a IS NOT NULL)"
        );
        assert_eq!(
            one(
                ChangeKind::CreateIndex,
                "uq_t_ab",
                ChangePayload::Index {
                    table: "t".into(),
                    def: IndexModel::new("uq_t_ab", vec!["a".into(), "b DESC".into()], true)
                }
            ),
            "CREATE UNIQUE INDEX \"uq_t_ab\" ON \"t\" (\"a\", \"b\" DESC)"
        );
        // 显式 btree 与 None 归一（introspect 侧也对 btree 做了同样的归一）
        let mut bt = IndexModel::new("idx_t_b", vec!["a".into()], false);
        bt.method = Some("btree".into());
        assert_eq!(
            one(
                ChangeKind::CreateIndex,
                "idx_t_b",
                ChangePayload::Index { table: "t".into(), def: bt }
            ),
            "CREATE INDEX \"idx_t_b\" ON \"t\" (\"a\")",
            "btree 是默认值，写出来会与 introspect 的归一口径不一致"
        );
    }

    #[test]
    fn index_without_columns_is_rejected() {
        let e = err(
            ChangeKind::CreateIndex,
            "idx_t_empty",
            ChangePayload::Index {
                table: "t".into(),
                def: IndexModel::new("idx_t_empty", vec![], false),
            },
        );
        assert!(matches!(e, RenderError::Unrenderable { .. }), "{e:?}");
    }

    /// FK：名派生、动作渲染、列数不等报错、未知动作报错。
    #[test]
    fn foreign_key_variants() {
        let fk = FkModel {
            name: Some("fk_child_parent".into()),
            cols: vec!["parent_id".into()],
            ref_table: "parent".into(),
            ref_cols: vec!["id".into()],
            on_delete: Some("Cascade".into()),
            on_update: None,
        };
        assert_eq!(
            one(
                ChangeKind::AddForeignKey,
                "child.parent_id",
                ChangePayload::Fk { table: "child".into(), def: fk.clone() }
            ),
            "ALTER TABLE \"child\" ADD CONSTRAINT \"fk_child_parent\" \
             FOREIGN KEY (\"parent_id\") REFERENCES \"parent\" (\"id\") ON DELETE CASCADE"
        );

        // 无名 ⇒ 按 PG 默认规则派生
        let mut anon = fk.clone();
        anon.name = None;
        let sql = one(
            ChangeKind::AddForeignKey,
            "child.parent_id",
            ChangePayload::Fk { table: "child".into(), def: anon },
        );
        assert!(sql.contains("ADD CONSTRAINT \"child_parent_id_fkey\""), "{sql}");

        // 未知动作 ⇒ 报错，**不能**省略
        let mut weird = fk.clone();
        weird.on_delete = Some("UNKNOWN(x)".into());
        let e = err(
            ChangeKind::AddForeignKey,
            "child.parent_id",
            ChangePayload::Fk { table: "child".into(), def: weird },
        );
        assert!(
            matches!(e, RenderError::UnknownFkAction { ref action, .. } if action == "UNKNOWN(x)"),
            "{e:?}"
        );

        // 两侧列数不等 ⇒ 提前判（PG 会拒绝该 DDL）
        let mut uneven = fk.clone();
        uneven.ref_cols = vec!["id".into(), "x".into()];
        let e = err(
            ChangeKind::AddForeignKey,
            "child.parent_id",
            ChangePayload::Fk { table: "child".into(), def: uneven },
        );
        assert!(matches!(e, RenderError::Unrenderable { .. }), "{e:?}");
    }

    #[test]
    fn check_variants() {
        assert_eq!(
            one(
                ChangeKind::AddCheck,
                "t.ck_t_n",
                ChangePayload::Check {
                    table: "t".into(),
                    def: CheckModel::new("ck_t_n", "n >= 0")
                }
            ),
            "ALTER TABLE \"t\" ADD CONSTRAINT \"ck_t_n\" CHECK (n >= 0)"
        );
        // PG 的 CHECK 恒有名字；空名说明载荷来自 SQLite 侧内省 ⇒ 报错而不是造匿名约束
        let e = err(
            ChangeKind::AddCheck,
            "t.<expr>",
            ChangePayload::Check { table: "t".into(), def: CheckModel::new("", "n >= 0") },
        );
        assert!(matches!(e, RenderError::EmptyIdentifier { .. }), "{e:?}");
    }

    // ── CreateTable 展开 ──

    fn sample_table() -> TableModel {
        let mut t = TableModel::new("child");
        t.columns.push(ColumnModel::new("id", "bigint", false));
        t.columns.push(ColumnModel::new("parent_id", "bigint", false));
        let mut code = ColumnModel::new("code", "text", true);
        code.unique = true;
        t.columns.push(code);
        let mut tsv = ColumnModel::new("content_tsv", "tsvector", true);
        tsv.generated = Some("to_tsvector('simple', COALESCE(code, ''))".into());
        t.columns.push(tsv);
        t.primary_key = vec!["id".into()];
        t.indexes.push(IndexModel::new(
            "uq_child_parent_code",
            vec!["parent_id".into(), "code".into()],
            true,
        ));
        t.fks.push(FkModel {
            name: Some("fk_child_parent".into()),
            cols: vec!["parent_id".into()],
            ref_table: "parent".into(),
            ref_cols: vec!["id".into()],
            on_delete: Some("Cascade".into()),
            on_update: None,
        });
        t.checks.push(CheckModel::new("ck_child_code", "length(code) > 0"));
        t
    }

    /// `CreateTable` 必须把索引 / FK / CHECK 一并展开 —— 因为 plan 不会为新表再发这些变更。
    #[test]
    fn create_table_expands_every_owned_object() {
        let t = sample_table();
        let r = render(
            &ch(ChangeKind::CreateTable, "child", ChangePayload::Table(t)),
            Dialect::Postgres,
        )
        .expect("应渲染");

        // 1 建表 + 1 索引 + 1 FK + 1 CHECK
        assert_eq!(r.statements.len(), 4, "{:#?}", r.statements);
        let create = &r.statements[0];
        assert!(create.starts_with("CREATE TABLE \"child\" ("), "{create}");
        assert!(create.contains("\"id\" bigint NOT NULL"));
        assert!(create.contains("PRIMARY KEY (\"id\")"));
        assert!(create.contains("UNIQUE (\"code\")"), "单列 UNIQUE 内联为表级约束：{create}");
        assert!(
            create
                .contains("GENERATED ALWAYS AS (to_tsvector('simple', COALESCE(code, ''))) STORED")
        );
        assert_eq!(
            r.statements[1],
            "CREATE UNIQUE INDEX \"uq_child_parent_code\" ON \"child\" (\"parent_id\", \"code\")"
        );
        assert!(r.statements[2].contains("ADD CONSTRAINT \"fk_child_parent\""));
        assert!(
            r.statements[3].contains("ADD CONSTRAINT \"ck_child_code\" CHECK (length(code) > 0)")
        );
        assert!(r.notes.iter().any(|n| n.contains("展开成 4 条")), "{:?}", r.notes);
    }

    /// 生成列必须排在被引用列之后（零成本防御 PG 的前向引用限制）。
    #[test]
    fn generated_columns_come_after_plain_ones() {
        let mut t = TableModel::new("t");
        // 故意让生成列在名字序上排前：`a_tsv` < `z_src`
        let mut g = ColumnModel::new("a_tsv", "tsvector", true);
        g.generated = Some("to_tsvector('simple', z_src)".into());
        t.columns.push(g);
        t.columns.push(ColumnModel::new("z_src", "text", true));
        let r =
            render(&ch(ChangeKind::CreateTable, "t", ChangePayload::Table(t)), Dialect::Postgres)
                .expect("应渲染");
        let create = &r.statements[0];
        let pos_src = create.find("\"z_src\"").expect("源列");
        let pos_gen = create.find("\"a_tsv\"").expect("生成列");
        assert!(pos_src < pos_gen, "被引用的列必须先定义：{create}");
    }

    /// 自增列建表必须渲染成 PG 的自增**伪类型**。
    ///
    /// 实况侧 `format_type()` 对 `bigserial` 列报回的是 `bigint`（serial 不是真类型，只是
    /// 「整型 + 序列 + 默认值」的语法糖）⇒ 两侧 `sql_type` 本就都是 `bigint`。若建表时照写
    /// `bigint`，会建出一张**没有自增**的表，而且它是**静默**的：表建成了、列也在，只是
    /// 不给 id 的 INSERT 会失败。
    #[test]
    fn create_table_renders_auto_increment_as_pseudo_type() {
        let mut t = TableModel::new("t");
        let mut id = ColumnModel::new("id", "bigint", false);
        id.auto_increment = true;
        t.columns.push(id);
        t.primary_key = vec!["id".into()];

        let r =
            render(&ch(ChangeKind::CreateTable, "t", ChangePayload::Table(t)), Dialect::Postgres)
                .expect("应渲染");
        assert!(r.statements[0].contains("\"id\" bigserial"), "{}", r.statements[0]);
    }

    /// 三种整数宽度各对应一个伪类型；认不出的类型**报错**而不是退回原类型。
    #[test]
    fn auto_increment_pseudo_types_cover_three_integer_widths() {
        for (ty, want) in
            [("smallint", "smallserial"), ("integer", "serial"), ("bigint", "bigserial")]
        {
            let mut t = TableModel::new("t");
            let mut id = ColumnModel::new("id", ty, false);
            id.auto_increment = true;
            t.columns.push(id);
            t.primary_key = vec!["id".into()];
            let r = render(
                &ch(ChangeKind::CreateTable, "t", ChangePayload::Table(t)),
                Dialect::Postgres,
            )
            .expect("应渲染");
            assert!(
                r.statements[0].contains(&format!("\"id\" {want}")),
                "{ty} 应映射成 {want}：{}",
                r.statements[0]
            );
        }

        // `text` 没有自增伪类型 ⇒ fail-closed：宁可整条变更渲染失败（apply 会整批拒绝并
        // 说明原因），也不建一张「看着像自增、其实没有自增」的表。
        let mut t = TableModel::new("t");
        let mut id = ColumnModel::new("id", "text", false);
        id.auto_increment = true;
        t.columns.push(id);
        t.primary_key = vec!["id".into()];
        let e = err(ChangeKind::CreateTable, "t", ChangePayload::Table(t));
        assert!(matches!(e, RenderError::Unrenderable { .. }), "{e:?}");
    }

    /// 自增 + DEFAULT 同时出现 ⇒ **渲染期**就报错。
    ///
    /// `serial` 族自带一条 `DEFAULT nextval(…)`，PG 拒绝第二个默认值
    /// （`multiple default values specified`）。在渲染期拦住比让执行侧去解析 PG 报错原文精确。
    #[test]
    fn auto_increment_with_default_is_rejected() {
        let mut c = ColumnModel::new("id", "bigint", false);
        c.auto_increment = true;
        c.default = Some("0".into());
        let e =
            err(ChangeKind::AddColumn, "t.id", ChangePayload::Column { table: "t".into(), col: c });
        assert!(matches!(e, RenderError::Unrenderable { .. }), "{e:?}");
    }

    #[test]
    fn create_table_rejects_missing_pk_column() {
        let mut t = TableModel::new("t");
        t.columns.push(ColumnModel::new("a", "bigint", false));
        t.primary_key = vec!["nope".into()];
        let e = err(ChangeKind::CreateTable, "t", ChangePayload::Table(t));
        assert!(matches!(e, RenderError::Unrenderable { .. }), "{e:?}");
    }

    #[test]
    fn create_table_without_columns_is_rejected() {
        let e = err(ChangeKind::CreateTable, "t", ChangePayload::Table(TableModel::new("t")));
        assert!(matches!(e, RenderError::Unrenderable { .. }), "{e:?}");
    }

    // ── fail-closed 面 ──

    #[test]
    fn sqlite_render_failure_names_the_dialect_and_the_kind() {
        // 这条守的是**错误载荷**而不是错误文案：`UnsupportedDialect` 必须带上足够的信息，
        // 让人不查源代码就知道「哪个方言、哪一类」不支持（apply 的整批拒绝报告靠它）。
        let e = render(
            &ch(ChangeKind::AlterColumnType, "t.c", col_payload("t", col("c", "text"))),
            Dialect::Sqlite,
        )
        .expect_err("SQLite 不支持 ALTER COLUMN TYPE");
        let msg = e.to_string();
        assert!(msg.contains("sqlite") || msg.contains("Sqlite"), "错误文案未指明方言：{msg}");
        // ⚠ 断言的是 `as_str()` 而**不是** Rust 的变体名：`AlterColumnType` 的 `as_str()` 是
        // `"ALTER TYPE"`。同一份文案要被人读，也要被 apply 的整批拒绝报告拼进去 —— 那里用的
        // 就是 `as_str()`，所以判据必须锚在同一个串上，否则「改个变体名」就会静默放过它。
        assert!(
            msg.contains(ChangeKind::AlterColumnType.as_str()),
            "错误文案未指明变更类别（期望含 `{}`）：{msg}",
            ChangeKind::AlterColumnType.as_str()
        );
    }

    #[test]
    fn payload_mismatch_is_rejected() {
        let e = err(ChangeKind::AddColumn, "t.c", ChangePayload::Bare);
        assert!(matches!(e, RenderError::PayloadMismatch { .. }), "{e:?}");
        let e = err(ChangeKind::DropTable, "t", col_payload("t", col("c", "text")));
        assert!(matches!(e, RenderError::PayloadMismatch { .. }), "{e:?}");
    }

    /// 21 个 `ChangeKind` 逐个喂进去：要么 `Ok`，要么 `Err`，**绝不 panic**。
    ///
    /// 这条测试的价值不在「渲染对了」，而在于：新增 `ChangeKind` 变体时若忘了在
    /// `render` 里处理，`match` 会先编译不过；若处理了却 panic（`unwrap`/索引越界），
    /// 这里会红。故它同时守住「穷举」与「不 panic」两件事。
    #[test]
    fn every_change_kind_renders_or_errors_without_panicking() {
        use ChangeKind::*;
        let table = sample_table();
        let cases: Vec<(ChangeKind, ChangePayload)> = vec![
            (
                RenameColumn,
                ChangePayload::Rename { table: "t".into(), from: "a".into(), to: "b".into() },
            ),
            (
                RenameIndex,
                ChangePayload::Rename { table: "t".into(), from: "a".into(), to: "b".into() },
            ),
            (
                RenameForeignKey,
                ChangePayload::Rename { table: "t".into(), from: "a".into(), to: "b".into() },
            ),
            (
                DropCheck,
                ChangePayload::Check { table: "t".into(), def: CheckModel::new("ck", "n>=0") },
            ),
            (
                DropForeignKey,
                ChangePayload::Fk {
                    table: "t".into(),
                    def: FkModel {
                        name: Some("fk".into()),
                        cols: vec!["a".into()],
                        ref_table: "r".into(),
                        ref_cols: vec!["id".into()],
                        on_delete: None,
                        on_update: None,
                    },
                },
            ),
            (
                DropIndex,
                ChangePayload::Index {
                    table: "t".into(),
                    def: IndexModel::new("i", vec!["a".into()], false),
                },
            ),
            (DropTable, ChangePayload::Bare),
            (CreateTable, ChangePayload::Table(table)),
            (AddColumn, col_payload("t", col("c", "text"))),
            (AlterColumnType, col_payload("t", col("c", "text"))),
            (SetNotNull, col_payload("t", col("c", "text"))),
            (DropNotNull, col_payload("t", col("c", "text"))),
            (SetDefault, col_payload("t", col("c", "text"))),
            (DropDefault, col_payload("t", col("c", "text"))),
            (AlterGenerated, col_payload("t", col("c", "text"))),
            (DropColumn, col_payload("t", col("c", "text"))),
            (AddUnique, col_payload("t", col("c", "text"))),
            (DropUnique, col_payload("t", col("c", "text"))),
            (
                AddForeignKey,
                ChangePayload::Fk {
                    table: "t".into(),
                    def: FkModel {
                        name: None,
                        cols: vec!["a".into()],
                        ref_table: "r".into(),
                        ref_cols: vec!["id".into()],
                        on_delete: Some("Cascade".into()),
                        on_update: None,
                    },
                },
            ),
            (
                AddCheck,
                ChangePayload::Check { table: "t".into(), def: CheckModel::new("ck", "n>=0") },
            ),
            (
                CreateIndex,
                ChangePayload::Index {
                    table: "t".into(),
                    def: IndexModel::new("i", vec!["a".into()], false),
                },
            ),
        ];
        // ⚠ 这里比对的是**集合**，不是数量。数量断言（`cases.len() == 21`）有个洞：
        // 加一类的同时删一类，数量不变、少测的那一类无人发现。集合比对堵住它 ——
        // 「21 类全覆盖」这句话因此变成机器守着的判据，而不是一句自我评价。
        let mut got: Vec<&str> = cases.iter().map(|(k, _)| k.as_str()).collect();
        got.sort_unstable();
        got.dedup();
        let mut want: Vec<&str> = ChangeKind::ALL.iter().map(|k| k.as_str()).collect();
        want.sort_unstable();
        assert_eq!(want.len(), 21, "`ChangeKind::ALL` 的长度变了：同步 plan.rs 与本测试");
        assert_eq!(
            got, want,
            "渲染穷举用例的类别集合与 `ChangeKind::ALL` 不一致 —— \
             `want` 里有而 `got` 里没有的那些类别**没有被测到**"
        );

        for (kind, payload) in cases {
            match render(&ch(kind, "obj", payload), Dialect::Postgres) {
                Ok(r) => assert!(!r.statements.is_empty(), "{} 渲染出 0 条语句", kind.as_str()),
                Err(e) => {
                    // 失败必须给出**非空**原因，不能是空消息
                    assert!(!format!("{e}").is_empty(), "{} 的错误消息为空", kind.as_str());
                },
            }
        }
    }
}
