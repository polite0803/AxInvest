// SPDX-License-Identifier: AGPL-3.0-only

//! 中立 schema 模型 —— **期望侧**（L1 实体 + L2 声明）与**实况侧**（introspect）
//! 共用的唯一数据结构。
//!
//! ## 为什么需要中立模型
//!
//! 引擎要做三件事：读库（introspect）、算期望（实体 + L2）、求差（plan）。若三者
//! 各持一套结构，比较就退化成「手写映射」——每一处映射都是一次静默出错的机会。
//! 因此两侧**必须先归一到同一个结构、同一套类型口径**，比较才是结构相等
//! （`==`），而不是「差不多」。
//!
//! ## 规范化（canonical）不变量
//!
//! 指纹是对本模型的 JSON 做 SHA-256，故顺序必须确定。`finalize()` 建立三条不变量：
//!
//! 1. `tables` 按表名升序；表内 `columns` / `indexes` / `checks` / `extras` 按名升序；
//!    `primary_key` 与 `fks` 也排序（复合主键/外键的**列顺序不影响约束语义**）。
//! 2. **`IndexModel::cols` 与 `FkModel::cols` 的列顺序不排序** —— 索引列顺序是语义
//!    （`(a,b)` 与 `(b,a)` 是两条不同索引），排序会让引擎漏判真实差异。
//! 3. `ColumnModel::default` 为空与 `None` 归一（见 [`normalize_default`]）。
//!
//! ## 类型口径：为什么不能逐字比较
//!
//! 实测（`output/tmp-p3-type-probe.log`，生产 PG）：全库 2062 列只有 **9 种**
//! 实际类型（`text` 1333 / `bigint` 354 / `integer` 174 / `double precision` 128 /
//! `boolean` 31 / `jsonb` 20 / `real` 10 / `tsvector` 9 / `vector(1024)` 3），
//! **`varchar` 一列都没有**。而 sea-query 把实体 `String` 渲染成 `varchar`
//! （`sea-query-1.0.2/src/backend/postgres/table.rs:23-30`）。
//!
//! ⇒ 逐字比较会产出 **1333 条**「类型不一致」，而它们**语义上完全等价**
//! （PG 里无长度的 `varchar` 与 `text` 都是无长度限制的可变长字符串）。
//! ⇒ 因此类型一律先过 [`canonical_type`] 归一，再比较。
//!
//! 双方言口径不同，且各有依据：
//!
//! | 方言 | 口径 | 依据 |
//! |---|---|---|
//! | PG | 规范类型名 + **等价类**合并（`text`≡`varchar`、`bool`≡`boolean`、`timestamp`≡`timestamp without time zone`） | PG 类型别名语义 |
//! | SQLite | **类型亲和性**（integer/text/real/blob/numeric）5 类 | SQLite 官方 §3.1 亲和性规则：SQLite 只认亲和性，声明的类型名本身不参与语义 |

use sea_orm::sea_query::ColumnType;
use serde::{Serialize, Serializer};

use crate::reconcile::extras::Dialect;

/// 引擎规则版本。**任何会改变期望结构的行为变更都必须递增它** —— 它在指纹内，
/// 递增即触发全库重跑（§四·一）。
pub const ENGINE_REV: &str = "1";

/// 方言的序列化形态（进指纹）。与 [`Dialect`] 的 `Debug` 表示解耦，避免改名即改指纹。
impl Serialize for Dialect {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(match self {
            Dialect::Sqlite => "sqlite",
            Dialect::Postgres => "postgres",
        })
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// 模型
// ═══════════════════════════════════════════════════════════════════════════

/// 一列。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ColumnModel {
    pub name: String,
    /// **已归一**的方言类型名（见 [`canonical_type`]）。
    pub sql_type: String,
    pub nullable: bool,
    /// 归一后的默认值表达式；`None` = 无默认。
    pub default: Option<String>,
    /// 是否单列主键。复合主键的成员由 [`TableModel::primary_key`] 表达。
    pub primary_key: bool,
    /// 是否**单列** UNIQUE（L1 `#[sea_orm(unique)]` / 库侧 `contype='u'` 且只有一列）。
    /// **多列** UNIQUE 走 [`IndexModel`]（L2 `IndexDecl{unique:true}`）。
    pub unique: bool,
    /// 生成列表达式（`GENERATED ALWAYS AS (expr) STORED`）；`None` = 普通列。
    pub generated: Option<String>,
    /// 改名来源列（L1 `#[sea_orm(renamed_from = "…")]`）。它是「删旧列 vs 改名」的
    /// **唯一**判据（§四·二），缺失即退化为「删旧列 + 建新列」并丢数据。
    pub renamed_from: Option<String>,
    /// 自增列（L1 `#[sea_orm(auto_increment = true)]`）。
    ///
    /// ## 为什么必须有它 —— 它是一类**假差异**的解药（2026-09-16 真库实测）
    ///
    /// PG 里自增列的实现是「一个序列 + 一条 `DEFAULT nextval('…_seq')`」，于是实况侧的
    /// [`Self::default`] **非空**；而 sea-orm 的 `create_table_from_entity` 把自增表达为
    /// `ColumnDef::auto_increment()`，**不写进 `default`**，于是期望侧是 `None`。
    /// 两侧口径不同 ⇒ 差量判据（`plan.rs`「默认值只比有没有」）必然产出
    /// `DROP DEFAULT` —— 而执行它等于**把自增列变成普通列**，之后任何不给 id 的
    /// INSERT 都会因 NOT NULL 失败。
    ///
    /// 实测（`output/tmp-p4-serial.log`）：生产库 10 个 `nextval` 列里，**6 个的实体
    /// 已经正确写了 `auto_increment = true` 却仍被计划成 `DROP DEFAULT`** ⇒ 这不是
    /// 声明缺口，是引擎把「序列绑定」误读成「默认值」。
    ///
    /// ## 为什么不做成一个可执行的变更类别
    ///
    /// 「自增属性不一致」目前**只出 advisory、不改写 DDL**：PG 侧改写自增属性要
    /// `ADD GENERATED` / 建序列 / 改默认值三件事，语义与数据都可能受损，属单独一批
    /// 工作。**宁可显式报出、也不静默当成一致** —— 后者会让这个字段变成装饰。
    pub auto_increment: bool,
}

impl ColumnModel {
    pub fn new(name: impl Into<String>, sql_type: impl Into<String>, nullable: bool) -> Self {
        Self {
            name: name.into(),
            sql_type: sql_type.into(),
            nullable,
            default: None,
            primary_key: false,
            unique: false,
            generated: None,
            renamed_from: None,
            auto_increment: false,
        }
    }

    /// 实况侧的 `default` 是不是「自增列的那条序列绑定」。
    ///
    /// 只看 `nextval(` 前缀与两处 PG 元数据来源（`pg_get_expr` 输出 / `attidentity`），
    /// **不猜序列名**：序列名会被 PG 按 63 字节截断，拼名字必然有长表名的反例。
    pub fn default_is_sequence_binding(&self) -> bool {
        self.default.as_deref().is_some_and(|d| d.starts_with("nextval("))
    }
}

/// 一条索引。
///
/// ⚠ `cols` **保持声明顺序**（不参与排序）—— 见模块文档不变量 2。
/// 元素允许带排序方向后缀（`"created_at DESC"`，对齐 `pg_get_indexdef` 形态）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct IndexModel {
    /// PG 侧恒有名字；**SQLite 侧的表级匿名 UNIQUE 约束没有名字** ⇒ 可能为空串。
    pub name: String,
    pub cols: Vec<String>,
    pub unique: bool,
    /// `Some("gin")` / `Some("hnsw")`；`None` = 默认 B-tree。
    pub method: Option<String>,
    /// partial 索引谓词，**含外括号**（对齐 `pg_get_indexdef`）；`None` = 全表。
    pub where_clause: Option<String>,
}

impl IndexModel {
    pub fn new(name: impl Into<String>, cols: Vec<String>, unique: bool) -> Self {
        Self { name: name.into(), cols, unique, method: None, where_clause: None }
    }

    /// diff 与排序用的**稳定身份**。
    ///
    /// SQLite 的**匿名**表级 `UNIQUE (a, b)` 不保留约束名（`sqlite_master.sql` 里就是裸
    /// `UNIQUE (a, b)`，`pragma_index_list` 只给 `sqlite_autoindex_{表}_{N}` 这种**位置**名，
    /// 会随 DDL 顺序漂移）。无名时退化为按「唯一性 + 列集」识别，否则同一约束在
    /// 两个方言间无法对账，且每次 DDL 顺序微调都会被判为「删一条 + 加一条」。
    pub fn key(&self) -> String {
        if self.name.is_empty() {
            format!("{}:{}", if self.unique { "uq" } else { "idx" }, self.cols.join(","))
        } else {
            self.name.clone()
        }
    }
}

/// 一条外键。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FkModel {
    pub name: Option<String>,
    pub cols: Vec<String>,
    pub ref_table: String,
    pub ref_cols: Vec<String>,
    /// `"Cascade"` / `"SetNull"` / `"Restrict"` / `"NoAction"` / `"SetDefault"`。
    pub on_delete: Option<String>,
    pub on_update: Option<String>,
}

/// 一条 CHECK 约束。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CheckModel {
    /// PG 侧恒有名字；**SQLite 侧通常没有**（匿名 CHECK）⇒ 可能为空串。
    pub name: String,
    pub expr: String,
}

impl CheckModel {
    pub fn new(name: impl Into<String>, expr: impl Into<String>) -> Self {
        Self { name: name.into(), expr: expr.into() }
    }

    /// diff 与排序用的**稳定身份**。
    ///
    /// SQLite 不给 CHECK 命名（`sqlite_master.sql` 里往往就是裸 `CHECK (...)`），
    /// 故无名时退化为按表达式识别 —— 否则两个方言的 CHECK 无法对账，
    /// 且同一约束在改名后会被判为「删一条 + 加一条」。
    pub fn key(&self) -> String {
        if self.name.is_empty() {
            format!("expr:{}", self.expr)
        } else {
            self.name.clone()
        }
    }
}

/// 一张表。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TableModel {
    pub name: String,
    pub columns: Vec<ColumnModel>,
    /// 主键列（单列主键也会在这里出现一次）。
    pub primary_key: Vec<String>,
    pub indexes: Vec<IndexModel>,
    pub fks: Vec<FkModel>,
    pub checks: Vec<CheckModel>,
    /// L2 认领标记：让引擎知道本表还有哪些**实体表达不了**的对象挂在身上
    /// （FTS5 虚表 / 触发器 / 生成列 / 动态 tsvector）。缺一项 ⇒ 该对象被判孤儿。
    pub extras: Vec<String>,
}

impl TableModel {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            columns: Vec::new(),
            primary_key: Vec::new(),
            indexes: Vec::new(),
            fks: Vec::new(),
            checks: Vec::new(),
            extras: Vec::new(),
        }
    }

    pub fn column(&self, name: &str) -> Option<&ColumnModel> {
        self.columns.iter().find(|c| c.name == name)
    }

    pub fn index(&self, name: &str) -> Option<&IndexModel> {
        self.indexes.iter().find(|i| i.key() == name)
    }

    /// 按名插入或替换一列（L1 先写、L2 后补同名列时用）。
    pub fn upsert_column(&mut self, col: ColumnModel) {
        match self.columns.iter_mut().find(|c| c.name == col.name) {
            Some(slot) => *slot = col,
            None => self.columns.push(col),
        }
    }

    /// 按名插入或替换一条索引。
    pub fn upsert_index(&mut self, idx: IndexModel) {
        match self.indexes.iter_mut().find(|i| i.key() == idx.key()) {
            Some(slot) => *slot = idx,
            None => self.indexes.push(idx),
        }
    }

    /// 按名插入或替换一条 CHECK。
    pub fn upsert_check(&mut self, ck: CheckModel) {
        match self.checks.iter_mut().find(|c| c.key() == ck.key()) {
            Some(slot) => *slot = ck,
            None => self.checks.push(ck),
        }
    }

    /// 追加一个 L2 认领标记（去重，保持有序由 `finalize` 保证）。
    pub fn claim_extra(&mut self, tag: impl Into<String>) {
        let tag = tag.into();
        if !self.extras.contains(&tag) {
            self.extras.push(tag);
        }
    }
}

/// 全库 schema 模型。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SchemaModel {
    /// 引擎规则版本（进指纹）。
    pub engine_rev: &'static str,
    pub dialect: Dialect,
    pub tables: Vec<TableModel>,
    /// **全库级** L2 认领标记（`fn:ax_cjk_ngram`、`fts5:messages_fts` …）。
    ///
    /// 为什么不能塞进 `TableModel.extras`：这些对象**不挂在任何一张表上**
    /// （函数、触发器、虚表本体）。它们必须同样进指纹 —— 否则 L2 里改了一条
    /// 函数声明，指纹不变 ⇒ 引擎短路跳过 ⇒ 声明永不生效（静默）。
    pub extras: Vec<String>,
}

impl SchemaModel {
    pub fn new(dialect: Dialect) -> Self {
        Self { engine_rev: ENGINE_REV, dialect, tables: Vec::new(), extras: Vec::new() }
    }

    /// 追加一个全库级 L2 认领标记（去重；有序由 `finalize` 保证）。
    pub fn claim_extra(&mut self, tag: impl Into<String>) {
        let tag = tag.into();
        if !self.extras.contains(&tag) {
            self.extras.push(tag);
        }
    }

    pub fn table(&self, name: &str) -> Option<&TableModel> {
        self.tables.iter().find(|t| t.name == name)
    }

    pub fn table_mut(&mut self, name: &str) -> Option<&mut TableModel> {
        self.tables.iter_mut().find(|t| t.name == name)
    }

    /// 按名插入或替换一张表。
    pub fn upsert_table(&mut self, t: TableModel) {
        match self.tables.iter_mut().find(|x| x.name == t.name) {
            Some(slot) => *slot = t,
            None => self.tables.push(t),
        }
    }

    /// 取表，不存在则建（L1 与 L2 往同一张表上叠声明时用）。
    pub fn table_or_insert(&mut self, name: &str) -> &mut TableModel {
        if self.table(name).is_none() {
            self.tables.push(TableModel::new(name));
        }
        self.table_mut(name).expect("刚刚插入")
    }

    pub fn table_names(&self) -> Vec<&str> {
        self.tables.iter().map(|t| t.name.as_str()).collect()
    }

    pub fn is_empty(&self) -> bool {
        self.tables.is_empty()
    }

    /// 建立规范化不变量（模块文档 1–2）。**幂等**：重复调用结果不变。
    ///
    /// 指纹与 diff 都要求两侧已 `finalize()`；`plan()` 入口会断言这一点。
    pub fn finalize(&mut self) {
        for t in &mut self.tables {
            t.columns.sort_by(|a, b| a.name.cmp(&b.name));
            // cols 不排序（语义）；indexes 列表本身按 key 排序
            // 用 `sort_by_key` 而非 `sort_by(|a,b| a.key().cmp(&b.key()))`：clippy
            // `unnecessary_sort_by`（`-D warnings` 下是硬门禁）。两者都是**稳定**排序，
            // 且 `key()` 无副作用 ⇒ 语义等价（只是调用次数 O(n log n) → O(n)）。
            t.indexes.sort_by_key(|a| a.key());
            t.primary_key.sort();
            // fks：先按列集，再按引用表（列顺序保留，故用 cols 原序比较）
            t.fks.sort_by(|a, b| {
                a.cols
                    .cmp(&b.cols)
                    .then_with(|| a.ref_table.cmp(&b.ref_table))
                    .then_with(|| a.ref_cols.cmp(&b.ref_cols))
            });
            t.checks.sort_by_key(|a| a.key());
            t.extras.sort();
            t.extras.dedup();
        }
        self.tables.sort_by(|a, b| a.name.cmp(&b.name));
        self.extras.sort();
        self.extras.dedup();
    }

    /// 规范化 JSON（指纹输入）。**调用方须先 `finalize()`**。
    pub fn canonical_json(&self) -> String {
        serde_json::to_string(self).expect("SchemaModel 序列化不应失败")
    }

    /// 结构化指纹输入，便于诊断（`plan --explain` 用）。
    pub fn canonical_value(&self) -> serde_json::Value {
        serde_json::to_value(self).expect("SchemaModel 序列化不应失败")
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// 类型归一
// ═══════════════════════════════════════════════════════════════════════════

/// 统一入口：把「期望侧 `ColumnType`」或「实况侧类型名」归一到同一条口径。
///
/// 两侧都从这里出，比较才是结构相等。
pub fn canonical_type_expected(t: &ColumnType, dialect: Dialect) -> String {
    match dialect {
        Dialect::Postgres => pg_canonical_expected(t),
        Dialect::Sqlite => sqlite_class(&sqlite_declared_type(t)).to_string(),
    }
}

/// 实况侧入口。
pub fn canonical_type_actual(raw: &str, dialect: Dialect) -> String {
    match dialect {
        Dialect::Postgres => pg_canonical_actual(raw),
        Dialect::Sqlite => sqlite_class(raw).to_string(),
    }
}

/// 归一默认值表达式：去空白、剥外层括号、统一 `NULL` 表示。
///
/// 实况侧来自 `pg_get_expr`/`column_default`，期望侧来自 `sea_query::Expr` 的
/// 渲染，两边的空白与括号风格必有差异 ⇒ 只比「去掉装饰后的语义文本」。
pub fn normalize_default(raw: &str) -> String {
    let mut s = raw.trim().to_string();
    // 逐层剥**配平**的外层括号：`('x'::text)` → `'x'::text`
    loop {
        if !(s.starts_with('(') && s.ends_with(')')) {
            break;
        }
        if !parens_balanced(&s[1..s.len() - 1]) {
            break;
        }
        s = s[1..s.len() - 1].trim().to_string();
    }
    // 折叠空白（PG 会输出 `'a'::character varying` 之类，空白数量不稳定）
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// `raw` 的括号是否配平（用于判断能否安全剥掉最外层）。
fn parens_balanced(raw: &str) -> bool {
    let mut depth: i32 = 0;
    let mut in_str = false;
    let mut prev = '\0';
    for ch in raw.chars() {
        if in_str {
            if ch == '\'' && prev != '\\' {
                in_str = false;
            }
        } else {
            match ch {
                '\'' => in_str = true,
                '(' => depth += 1,
                ')' => {
                    depth -= 1;
                    if depth < 0 {
                        return false;
                    }
                },
                _ => {},
            }
        }
        prev = ch;
    }
    depth == 0
}

/// 归一 SQL **表达式**文本，用于比较「生成列表达式」这类会被 PG 重写的文本。
///
/// ## 为什么需要它（实测证据，`output/tmp-p3-constraint-probe.log`）
///
/// PG 会在 `pg_get_expr` / `pg_get_constraintdef` 里**重写**表达式：补显式类型转换、
/// 补冗余括号。实测：
///
/// ```text
/// 手写声明：period ~ '^\d{4}-\d{2}$'
/// PG 报回：CHECK ((period ~ '^\d{4}-\d{2}$'::text))
///
/// 手写声明：to_tsvector('simple', COALESCE(content, ''))
/// PG 报回：to_tsvector('simple'::regconfig, COALESCE(content, ''::text))
/// ```
///
/// ⇒ 逐字比较必然不等。归一化做五件事：**剥 `::类型` 转换**、**折空白**、
/// **把 PG 专有的 LIKE 类算子折成 SQL 标准写法**（见 [`fold_like_operators`]）、
/// **剥末尾显式的默认排序方向 `ASC`**（见 [`strip_default_sort_dir`]）、
/// **剥配平的外层括号**（见 [`normalize_default`]）。
///
/// 五件事的共同点是**它们全是恒等变换**：变换前后两条表达式在 SQL 语义上完全等价。
/// 这一点是硬要求 —— 本函数用在 `plan::same_index` 这类**判等**位置上，凡是会抹掉
/// 真实差异的变换都是缺陷（那会让引擎看不见应该做的事）。
///
/// ⚠ **本函数是近似，不是等价判定**。它只用于「差异报告里少一点噪声」；
/// 凡是依赖它的比较，都必须在文档里写明「表达式文本比较不可靠」，
/// 并在 `plan.rs` 里按**名字**做身份判定。
pub fn normalize_sql_expr(raw: &str) -> String {
    let collapsed = raw.split_whitespace().collect::<Vec<_>>().join(" ");
    let stripped = strip_casts(&collapsed);
    let folded = fold_like_operators(&stripped);
    let dir_free = strip_default_sort_dir(&folded);
    normalize_default(&dir_free)
}

/// 把 PG 专有的 LIKE 类**算子记号**折成 SQL 标准关键字写法：
/// `!~~*` → `NOT ILIKE`、`~~*` → `ILIKE`、`!~~` → `NOT LIKE`、`~~` → `LIKE`。
///
/// ## 为什么必须有这一步（实测缺陷，2026-09-17 第一现场）
///
/// 给引擎加一条**双方言**的部分唯一索引时发现：声明写成 SQL 标准的
/// `(name NOT LIKE 'rl_checkpoint:%')`，而 PG 的 `pg_get_indexdef` **回读**成
/// `(name !~~ 'rl_checkpoint:%'::text)`。归一化链原先只剥 `::type`、**不换算子**
/// ⇒ 两侧永远差一个算子的写法 ⇒ `plan::same_index` **恒判不等** ⇒ 每次启动都
/// 「重建」这条已经存在的索引。而这条链上的既有事故（`ASC` 那一次，见
/// [`strip_default_sort_dir`]）证明它的后果不是「多做一件事」，而是
/// **`additive_only` 只建不删同名的 ⇒ 执行必撞「已存在」⇒ fail-stop**。
///
/// ## 为什么它也是恒等变换
///
/// PG 把 `~~` / `!~~` / `~~*` / `!~~*` 定义为 `LIKE` / `NOT LIKE` / `ILIKE` / `NOT ILIKE`
/// 的**同一批算子的内部名**（`\do` 可见，语义逐字对应）⇒ 换写法不动语义。
///
/// ## 边界（刻意不做的部分）
///
/// * `~` / `!~`（正则）**不折** —— SQLite 没有等价算子，折成 `REGEXP` 会引入一个
///   双方言语义并不相同的对应（PG 的 `~` 是 POSIX 正则，SQLite 的 `REGEXP` 默认未实现）；
/// * **只折算子记号，不动关键字大小写** —— 写 `not like` 仍与 `!~~` 不等（同
///   [`strip_default_sort_dir`]「不做大小写归一」的分工）；
/// * **只折引号外的记号** —— `'a~~b'` 这类字面量必须原样保留（状态机保证）；
/// * 记号必须是**连续字节**：`!~~` 中间插空格（`! ~~`）不是合法 PG 写法，不归本函数管。
///
/// ## 为什么是 `pub(crate)` 而不是私有
///
/// 与 [`strip_casts`] 同一条理由，**两个调用方必须用同一把尺子**：比对侧
/// [`normalize_sql_expr`] 与渲染侧 `render::sqlite_expr`。渲染侧不折 ⇒ PG 写法的声明在
/// SQLite 上发出 `!~~` ⇒ 建索引当场失败；只剥 cast 不折算子的组合已经被实测证伪。
pub(crate) fn fold_like_operators(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = String::with_capacity(s.len());
    let mut quote: Option<u8> = None;
    let mut i = 0usize;
    while i < bytes.len() {
        let c = bytes[i];
        match quote {
            Some(q) => {
                let len = push_char(s, i, &mut out);
                if len == 1 && c == q {
                    if bytes.get(i + 1) == Some(&q) {
                        out.push(q as char);
                        i += 2;
                        continue;
                    }
                    quote = None;
                }
                i += len;
            },
            None => {
                if c == b'\'' || c == b'"' {
                    quote = Some(c);
                    i += push_char(s, i, &mut out);
                    continue;
                }
                // ⚠ 最长匹配优先：`!~~*` 必须先于 `!~~`、`~~*` 先于 `~~` 试，
                // 否则 `!~~*` 会被吃成 `NOT LIKE` + 一个落单的 `*`。
                let rest = &bytes[i..];
                if let Some((text, len)) = match_like_operator(rest) {
                    out.push_str(text);
                    i += len;
                } else {
                    i += push_char(s, i, &mut out);
                }
            },
        }
    }
    out
}

/// LIKE 类算子记号的**最长匹配**表：返回（标准写法, 消耗字节数）。
fn match_like_operator(rest: &[u8]) -> Option<(&'static str, usize)> {
    // 顺序即优先级（长的在前）。
    if rest.starts_with(b"!~~*") {
        Some(("NOT ILIKE", 4))
    } else if rest.starts_with(b"~~*") {
        Some(("ILIKE", 3))
    } else if rest.starts_with(b"!~~") {
        Some(("NOT LIKE", 3))
    } else if rest.starts_with(b"~~") {
        Some(("LIKE", 2))
    } else {
        None
    }
}

/// 剥掉末尾**显式的默认排序方向** `ASC`。
///
/// ## 为什么它是恒等变换
///
/// SQL 标准里索引列与 `ORDER BY` 项的默认方向就是 `ASC` ⇒ `(created_at ASC)` 与
/// `(created_at)` **语义完全相同**。声明作者写不写它纯属风格差异，不该被当成结构差异。
///
/// ## 为什么必须剥（实测缺陷，第一现场 2026-09-16）
///
/// `p5_engine_takeover --legacy-sqlite`（一个由迁移建出的 SQLite 库）跑 bootstrap 时
/// **首条就 fail-stop 中止**：`CREATE INDEX "idx_index_jobs_status" … already exists`。
/// 两侧原文一对照，差的就是一个 `ASC`：
///
/// | 侧 | `idx_index_jobs_status` 的列 |
/// |---|---|
/// | L2 声明（`extras.rs:903-904`） | `["status", "priority DESC", "created_at"]` |
/// | 实况（存量库由已删的 `v100` 合并基线建出） | `(status, priority DESC, created_at ASC)` |
///
/// 传导链：`plan::same_index` 判「不等」⇒ 同一条索引在 plan 里被拆成**同名**两条
/// （期望侧 `CreateIndex`、实况侧 `DropIndex`，见 `crates/dao/src/reconcile/plan.rs:1090-1108`）⇒ 启动路径的
/// `additive_only` 滤掉 `DropIndex`、只留 `CreateIndex` ⇒ **「只建不删同名的」** ⇒
/// 执行必撞「已存在」。所以它不是一个「少做一件事」的小问题，而是**把一次本该无声的
/// 结构等价判成了启动中止**。
///
/// ## 边界：只剥 `ASC`，且只剥末尾那一个
///
/// * `DESC` **不剥** —— 它有语义；
/// * `NULLS FIRST` / `NULLS LAST` **不剥** —— 也有语义，而且 PG（`ASC` ⇒ NULLS LAST）
///   与 SQLite（`ASC` ⇒ NULLS FIRST）的默认值**并不一致**，抹平它会是真实缺陷；
/// * 引号内的 `"asc"` / `'asc'` **不剥** —— 那是标识符或字面量（长度 3 的列名并不罕见）；
/// * 只剥**末尾**一个：`a asc, b` 这种中间位置不归本函数管（`cols` 是逐元素传入的，
///   每个元素自带方向后缀，所以末尾判据恰好够用）。
///
/// 本函数**不**做大小写归一 —— 那是 [`normalize_default`] 那边的事，两处各管一段。
fn strip_default_sort_dir(s: &str) -> String {
    let b = s.as_bytes();
    // 记录「最后一个位于引号外的空白」。ASCII 空格：入参已由 `split_whitespace` 折过，
    // 故不必考虑 `\t` / 连续空白。
    let mut last_gap: Option<usize> = None;
    let mut quote: Option<u8> = None;
    let mut i = 0usize;
    while i < b.len() {
        let c = b[i];
        match quote {
            Some(q) => {
                if c == q {
                    // SQL 里连写两个引号是「转义一个引号」，不结束引用。
                    if b.get(i + 1) == Some(&q) {
                        i += 2;
                        continue;
                    }
                    quote = None;
                }
            },
            None => {
                if c == b'\'' || c == b'"' {
                    quote = Some(c);
                } else if c == b' ' {
                    last_gap = Some(i);
                }
            },
        }
        i += 1;
    }

    let Some(g) = last_gap else { return s.to_string() };
    // `g` 是 ASCII 空格的字节位置 ⇒ `g + 1` 与 `g` 都在 char 边界上（不会有 UTF-8 截断）。
    let tail = &s[g + 1..];
    // 判据锚在「末尾这个裸词**就是** ASC」上：`"asc"`（带引号）与 `asc_x`（另一列名）
    // 都不满足 `eq_ignore_ascii_case`，故不会被误剥。
    if tail.eq_ignore_ascii_case("asc") {
        s[..g].to_string()
    } else {
        s.to_string()
    }
}

/// 从 `s[i..]` 取**一个完整字符**追加到 `out`，返回它占的字节数。
///
/// 本文件的扫描器都逐**字节**推进以识别 ASCII 记号，但**搬运**必须整字符 ——
/// `bytes[i] as char` 会把多字节字符拆坏（见 [`strip_casts`] 的文档）。
fn push_char(s: &str, i: usize, out: &mut String) -> usize {
    let ch = s[i..].chars().next().expect("i 必须落在字符边界上");
    out.push(ch);
    ch.len_utf8()
}

/// 剥掉 `::类型名`（含 `::double precision`、`::character varying`、`::text[]`）。
///
/// 状态机保证**字符串字面量内的 `::` 不动**（如 `'a::b'`）。
///
/// ## 为什么是 `pub(crate)` 而不是私有
///
/// 有两个调用方，且**必须用同一把尺子**：
///
/// 1. [`normalize_sql_expr`] —— **比对侧**。`plan::same_index` 用它比「声明文本」与
///    「实况读回的文本」。
/// 2. `render::create_index_sql` —— **渲染侧**，仅在 SQLite 分支。`::` 不是 SQLite 语法，
///    不剥就发不出去；但更根本的理由是**两侧同源**：实况侧是 SQLite 从 `sqlite_master.sql`
///    里读出的原始 DDL 文本，里面不可能有 `::type`（写了就建不出来）。若渲染侧不剥，
///    两侧文本永远差一段 `::text` ⇒ 每轮判成「索引不同」⇒ 反复 DROP + CREATE，**永不收敛**。
///
/// 两处各写一份实现迟早会漂移，而漂移的后果恰好就是上面那条 ⇒ 只留一份。
///
/// ⚠ **按字符搬运，不按字节**（2026-09-17 修）：本函数原先对每个字节写
/// `out.push(bytes[i] as char)` —— 那对非 ASCII 是**破坏性的**：一个 3 字节的汉字会被拆成
/// 3 个 `U+00XX` 字符，输出内容与长度全变。而 `render::create_index_sql` 的 SQLite 分支
/// 直接拿本函数的返回值拼 DDL ⇒ 含中文的谓词（如 `status = '暂停'`）会渲染出**乱码 DDL**，
/// 且不报任何错。实测当前 `extras.rs` 的 63 处非 ASCII 全在 `reason` / 断言消息里、
/// 没有进 `where_clause` ⇒ 这是**潜在**缺陷而非已发生；但它静默，故直接修掉。
/// 修它**不改变任何判等结果**：声明侧与实况侧都过本函数，两侧的搬运方式（错的或对的）
/// 始终一致 ⇒ 等价关系不变；变的是**渲染侧产物的正确性**。
pub(crate) fn strip_casts(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = String::with_capacity(s.len());
    let mut i = 0usize;
    let mut quote: Option<u8> = None;
    while i < bytes.len() {
        let c = bytes[i];
        match quote {
            Some(q) => {
                let len = push_char(s, i, &mut out);
                // 结束引用的判据只在 ASCII 引号上成立（`len == 1` 把多字节字符排除在外）。
                if len == 1 && c == q {
                    if bytes.get(i + 1) == Some(&q) {
                        out.push(q as char);
                        i += 2;
                        continue;
                    }
                    quote = None;
                }
                i += len;
            },
            None => {
                if c == b'\'' || c == b'"' {
                    quote = Some(c);
                    i += push_char(s, i, &mut out);
                    continue;
                }
                if c == b':' && bytes.get(i + 1) == Some(&b':') {
                    // 跳过 `::` 与其后的类型名（允许空格分隔的多词类型）
                    i += 2;
                    while i < bytes.len() && (bytes[i] as char).is_ascii_whitespace() {
                        i += 1;
                    }
                    while i < bytes.len()
                        && (bytes[i].is_ascii_alphanumeric()
                            || bytes[i] == b'_'
                            || bytes[i] == b'.'
                            || bytes[i] == b'['
                            || bytes[i] == b']')
                    {
                        i += 1;
                    }
                    // 多词类型（double precision / character varying）：仅当后面还跟词且
                    // 该词以字母开头、再后面是 `)` 或 `,` 时才吞（避免吃掉真实的别名）
                    loop {
                        let save = i;
                        let mut j = i;
                        while j < bytes.len() && (bytes[j] as char).is_ascii_whitespace() {
                            j += 1;
                        }
                        let start = j;
                        while j < bytes.len()
                            && (bytes[j].is_ascii_alphabetic() || bytes[j] == b'_')
                        {
                            j += 1;
                        }
                        let word = &s[start..j];
                        let is_multiword = matches!(
                            word,
                            "precision" | "varying" | "with" | "without" | "time" | "zone"
                        );
                        if is_multiword && j > start {
                            i = j;
                        } else {
                            i = save;
                            break;
                        }
                    }
                    continue;
                }
                i += push_char(s, i, &mut out);
            },
        }
    }
    out
}

// ─────────────────────────── PG ───────────────────────────

/// PG 实况类型名 → 规范名。
///
/// 输入形态：`format_type(atttypid, atttypmod)` 的输出（`character varying(255)`、
/// `timestamp without time zone`、`vector(1024)` …）。
///
/// 归并的等价类（PG 里是同一类型的别名，逐字比较会误报）：
/// - 无长度 `varchar` / `character varying` ≡ `text`（两者都是无长度限制的可变长串）
/// - `bool` ≡ `boolean`
/// - `timestamp without time zone` ≡ `timestamp`
/// - `timestamp with time zone` ≡ `timestamptz`
/// - `int2`/`int4`/`int8`/`float4`/`float8` 的数组式内部名 → 标准名
pub fn pg_canonical_actual(raw: &str) -> String {
    let s = raw.trim().to_ascii_lowercase();
    let s = s.split_whitespace().collect::<Vec<_>>().join(" ");
    // 带长度/精度的先拆出参数：参数部分**保留**（长度是约束，不能并）
    if let Some((base, args)) = split_type_args(&s) {
        let base = base.trim();
        let name = match base {
            // 无参形态才折成 text；带参时长度是约束 ⇒ 保留
            "varchar" | "character varying" => "varchar",
            "char" | "character" | "bpchar" => "char",
            other => fold_pg_base(other).unwrap_or(other),
        };
        return format!("{name}({args})");
    }
    // 无参形态：字符串族（无长度限制）一律折为 text
    match s.as_str() {
        "text" | "varchar" | "character varying" | "citext" => "text".to_string(),
        other => fold_pg_base(other).map(str::to_string).unwrap_or_else(|| other.to_string()),
    }
}

/// PG 基类型名折叠（**不含参数**的形态）。返回 `None` = 无需折叠，调用方保留原名。
///
/// 用 `Option` 而非哨兵串：哨兵会让未知类型丢掉原名（`OTHER`），
/// 而「未知类型」必须保持可辨识 —— 否则两个不同的未知类型会被判相等。
fn fold_pg_base(base: &str) -> Option<&'static str> {
    Some(match base {
        "char" | "character" | "bpchar" => "char",
        "bool" | "boolean" => "boolean",
        "timestamp" | "timestamp without time zone" => "timestamp",
        "timestamptz" | "timestamp with time zone" => "timestamptz",
        "time" | "time without time zone" => "time",
        "timetz" | "time with time zone" => "timetz",
        "date" => "date",
        "interval" => "interval",
        "int2" | "smallint" => "smallint",
        "int4" | "integer" => "integer",
        "int8" | "bigint" => "bigint",
        "float4" | "real" => "real",
        "float8" | "double precision" => "double precision",
        "numeric" | "decimal" => "numeric",
        "bytea" => "bytea",
        "money" => "money",
        _ => return None,
    })
}

/// PG `ColumnType` → 规范名。折叠规则与 [`pg_canonical_actual`] 严格镜像。
///
/// ⚠ 关键一格：`String(None)` → **`text`** 而非 `varchar`。sea-query 渲染成
/// `varchar`（`sea-query-1.0.2/src/backend/postgres/table.rs:29`），实测库侧 1333 列全是 `text`；
/// 两者在 PG 中语义等价，故归一到库侧实际使用的 `text`。带长度时保留（长度是约束）。
fn pg_canonical_expected(t: &ColumnType) -> String {
    use sea_orm::sea_query::ColumnType as C;
    use sea_orm::sea_query::StringLen;
    match t {
        // PG 的 `character` 无长度时等价于 `character(1)`（`format_type` 就输出 `character(1)`）
        C::Char(Some(n)) => format!("char({n})"),
        C::Char(None) => "char(1)".to_string(),
        C::String(StringLen::N(n)) => format!("varchar({n})"),
        C::String(_) => "text".to_string(),
        C::Text => "text".to_string(),
        // ⚠ 逐条镜像 sea-query PG 渲染（`sea-query-1.0.2/src/backend/postgres/table.rs:32-37`），**不按名字猜**：
        // TinyInteger/TinyUnsigned/SmallInteger → smallint；
        // **SmallUnsigned → integer**；**Unsigned → bigint**；其余整数 → bigint。
        // 猜错在这里的代价是「永远修不好的假差异」（P4 会反复 ALTER 同一列）。
        C::TinyInteger | C::TinyUnsigned | C::SmallInteger => "smallint".to_string(),
        C::Integer | C::SmallUnsigned => "integer".to_string(),
        C::BigInteger | C::Unsigned | C::BigUnsigned => "bigint".to_string(),
        C::Float => "real".to_string(),
        C::Double => "double precision".to_string(),
        C::Decimal(Some((p, s))) => format!("numeric({p},{s})"),
        C::Decimal(None) => "numeric".to_string(),
        C::DateTime | C::Timestamp => "timestamp".to_string(),
        C::TimestampWithTimeZone => "timestamptz".to_string(),
        C::Time => "time".to_string(),
        C::Date => "date".to_string(),
        C::Interval(..) => "interval".to_string(),
        C::Binary(_) | C::VarBinary(_) | C::Blob => "bytea".to_string(),
        C::Boolean => "boolean".to_string(),
        // PG 的 `money` 是固定 8 字节带标度类型，无精度参数
        C::Money(_) => "money".to_string(),
        C::Json => "json".to_string(),
        C::JsonBinary => "jsonb".to_string(),
        C::Uuid => "uuid".to_string(),
        C::Vector(Some(n)) => format!("vector({n})"),
        C::Vector(None) => "vector".to_string(),
        C::Array(inner) => format!("{}[]", pg_canonical_expected(inner)),
        C::Cidr => "cidr".to_string(),
        C::Inet => "inet".to_string(),
        C::MacAddr => "macaddr".to_string(),
        C::LTree => "ltree".to_string(),
        C::Bit(Some(n)) => format!("bit({n})"),
        C::Bit(None) => "bit".to_string(),
        C::VarBit(n) => format!("varbit({n})"),
        C::Year => "year".to_string(),
        C::Enum { name, .. } => name.inner().to_ascii_lowercase(),
        C::Custom(iden) => iden.inner().to_ascii_lowercase(),
        // sea-query `ColumnType` 标记 `#[non_exhaustive]`：未知新变体退化为
        // Debug 表示的规范化（小写去空格），保证「未知 ≠ 相等」——宁可多报差异
        // 也不静默判等。新增变体时这里会产出与库侧不同的串，从而暴露出来。
        other => format!("{other:?}").to_ascii_lowercase().replace(' ', ""),
    }
}

/// 拆 `base(args)`；无括号返回 `None`。
fn split_type_args(s: &str) -> Option<(&str, String)> {
    let open = s.find('(')?;
    if !s.ends_with(')') {
        return None;
    }
    Some((&s[..open], s[open + 1..s.len() - 1].replace(' ', "")))
}

// ─────────────────────────── SQLite ───────────────────────────

/// SQLite 声明的类型名 → **类型亲和性**（官方 §3.1，规则按序短路）。
///
/// SQLite 只认亲和性：`varchar(255)` 与 `text` 完全等价，`boolean` 落到 NUMERIC。
/// 逐字比较在这里同样是误报源，故一律折叠到 5 类。
pub fn sqlite_affinity(declared: &str) -> &'static str {
    let t = declared.trim().to_ascii_uppercase();
    if t.contains("INT") {
        return "integer";
    }
    if t.contains("CHAR") || t.contains("CLOB") || t.contains("TEXT") {
        return "text";
    }
    if t.is_empty() || t.contains("BLOB") {
        return "blob";
    }
    if t.contains("REAL") || t.contains("FLOA") || t.contains("DOUB") {
        return "real";
    }
    "numeric"
}

/// SQLite 亲和性 → **比较用的等价类**（在官方亲和性之上只做一处收拢）。
///
/// 为什么不能直接拿 [`sqlite_affinity`] 当比较口径：官方规则把 `boolean` 归入
/// NUMERIC 亲和，而 sea-orm 渲染 Rust `bool` 为 `BOOLEAN`（`backend/sqlite/table.rs`），
/// 本仓手写迁移却一律写 `INTEGER` ⇒ 逐亲和性比较会把两者判成差异，**每次启动都
/// 重建一次含 bool 列的表**（P4 的破坏性路径，且收敛不了）。
///
/// 收拢的正当性：SQLite 无独立布尔存储类，`bool` 列取值恒为 0/1，两种声明在
/// 该列上的行为完全一致；差异只体现在「非整数文本的转换」上，而走实体层写入的
/// bool 列不可能出现那种值。
///
/// ⚠ **只收拢这一对**：`numeric`（`DECIMAL` / `DECIMAL(10,5)`）与 `integer` 仍严格
/// 区分 —— 那两者确实不同（前者允许小数），合并会让真实的精度变更被静默漏掉。
/// 见 `sqlite_equivalence_class_is_exactly_one_pair`。
fn sqlite_class(declared: &str) -> &'static str {
    if declared.trim().to_ascii_uppercase().contains("BOOL") {
        return "integer";
    }
    sqlite_affinity(declared)
}

/// `ColumnType` → SQLite 会写进 DDL 的声明类型名。
///
/// **逐条镜像** `sea-query-1.0.2/src/backend/sqlite/table.rs:108-182`。不直接调
/// sea-query 渲染（它只给整条 `CREATE TABLE` 文本，取单列类型要解析 DDL）；
/// 但**只用于算亲和性**，故不需要逐字一致，只需**亲和性类别**一致 —— 见下方测试。
fn sqlite_declared_type(t: &ColumnType) -> String {
    use sea_orm::sea_query::ColumnType as C;
    use sea_orm::sea_query::StringLen;
    match t {
        C::Char(Some(n)) => format!("char({n})"),
        C::Char(None) => "char".to_string(),
        C::String(StringLen::N(n)) => format!("varchar({n})"),
        C::String(_) => "varchar".to_string(),
        C::Text => "text".to_string(),
        C::TinyInteger | C::TinyUnsigned => "tinyint".to_string(),
        C::SmallInteger | C::SmallUnsigned => "smallint".to_string(),
        C::Integer | C::Unsigned | C::BigInteger | C::BigUnsigned => "integer".to_string(),
        C::Float => "float".to_string(),
        C::Double => "double".to_string(),
        C::Decimal(Some((p, s))) => format!("real({p}, {s})"),
        C::Decimal(None) => "real_decimal".to_string(),
        C::DateTime => "datetime_text".to_string(),
        C::Timestamp => "timestamp_text".to_string(),
        C::TimestampWithTimeZone => "timestamp_with_timezone_text".to_string(),
        C::Time => "time_text".to_string(),
        C::Date => "date_text".to_string(),
        C::Binary(n) => format!("blob({n})"),
        C::VarBinary(_) | C::Blob => "blob".to_string(),
        C::Boolean => "boolean".to_string(),
        C::Money(_) => "real_money".to_string(),
        C::Json => "json_text".to_string(),
        C::JsonBinary => "jsonb_text".to_string(),
        C::Uuid => "uuid_text".to_string(),
        C::Enum { .. } => "enum_text".to_string(),
        // `Custom` 在 sea-query 里**原样输出标识符**（`sea-query-1.0.2/src/backend/sqlite/table.rs:181`），
        // 不追加任何后缀 —— 故这里也原样保留，否则 `tsvector` 之类会被改成
        // `tsvector_text`，与库侧 DDL 原文不一致。
        C::Custom(iden) => iden.inner().to_string(),
        // ⚠ 以下变体在 SQLite 后端是 `unimplemented!()`（`sea-query-1.0.2/src/backend/sqlite/table.rs:152-191`），
        // 即它们**永远不可能**出现在 SQLite 库里（渲染就 panic）。这里给出占位名只是
        // 为了让亲和性计算不至于编不出来；真实路径上 P4 渲染 DDL 会立刻 panic 暴露。
        C::Vector(_) => "vector_blob".to_string(),
        C::Array(_) => "array_text".to_string(),
        C::Cidr => "cidr_text".to_string(),
        C::Inet => "inet_text".to_string(),
        C::MacAddr => "macaddr_text".to_string(),
        C::LTree => "ltree_text".to_string(),
        C::Bit(_) => "bit".to_string(),
        C::VarBit(_) => "varbit".to_string(),
        C::Interval(..) => "interval_text".to_string(),
        C::Year => "year_text".to_string(),
        other => format!("{other:?}").to_ascii_lowercase().replace(' ', ""),
    }
}
// ═══════════════════════════════════════════════════════════════════════════
// 测试
// ═══════════════════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;

    fn col(name: &str) -> ColumnModel {
        ColumnModel::new(name, "text", true)
    }

    /// 规范化不变量：表和表内集合排序，但索引 `cols` 与 `finalize` 都不动它。
    #[test]
    fn finalize_sorts_but_preserves_semantic_column_order() {
        let mut m = SchemaModel::new(Dialect::Postgres);
        let mut t = TableModel::new("zeta");
        t.upsert_column(col("b_col"));
        t.upsert_column(col("a_col"));
        t.primary_key = vec!["z".into(), "a".into()];
        t.upsert_index(IndexModel {
            name: "idx_zeta_b_a".into(),
            cols: vec!["b_col".into(), "a_col DESC".into()],
            unique: false,
            method: None,
            where_clause: None,
        });
        m.upsert_table(t);
        let mut t2 = TableModel::new("alpha");
        t2.upsert_column(col("id"));
        m.upsert_table(t2);

        m.finalize();

        assert_eq!(m.table_names(), vec!["alpha", "zeta"], "表按名升序");
        let z = m.table("zeta").unwrap();
        assert_eq!(
            z.columns.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(),
            ["a_col", "b_col"]
        );
        assert_eq!(z.primary_key, vec!["a", "z"], "主键列顺序不影响约束语义，排序");
        assert_eq!(
            z.index("idx_zeta_b_a").unwrap().cols,
            vec!["b_col", "a_col DESC"],
            "索引列顺序是语义（含 DESC 方向），必须原样保留"
        );
    }

    /// `finalize` 幂等：调两次与调一次结果一致（指纹稳定的前提）。
    #[test]
    fn finalize_is_idempotent() {
        let mut m = SchemaModel::new(Dialect::Postgres);
        let mut t = TableModel::new("t");
        t.upsert_column(col("b"));
        t.upsert_column(col("a"));
        m.upsert_table(t);
        m.finalize();
        let once = m.canonical_json();
        m.finalize();
        assert_eq!(once, m.canonical_json(), "二次 finalize 不应改变序列化结果");
    }

    /// PG 等价类：无长度 varchar ≡ text；bool ≡ boolean；timestamp 别名。
    /// 这三条若漏，生产库会产生 1333+31 条假差异。
    #[test]
    fn pg_shrinks_equivalent_types() {
        for raw in ["text", "varchar", "character varying", "TEXT", " character  varying "] {
            assert_eq!(pg_canonical_actual(raw), "text", "raw={raw}");
        }
        assert_eq!(pg_canonical_actual("bool"), "boolean");
        assert_eq!(pg_canonical_actual("boolean"), "boolean");
        assert_eq!(pg_canonical_actual("timestamp without time zone"), "timestamp");
        assert_eq!(pg_canonical_actual("timestamp"), "timestamp");
        assert_eq!(pg_canonical_actual("timestamp with time zone"), "timestamptz");
        assert_eq!(pg_canonical_actual("double precision"), "double precision");
        assert_eq!(pg_canonical_actual("int8"), "bigint");
    }

    /// ⚠ 带长度的 varchar **不**与 text 合并：长度是约束，合并会让引擎漏判收窄。
    #[test]
    fn pg_keeps_length_qualified_types_distinct() {
        assert_eq!(pg_canonical_actual("varchar(50)"), "varchar(50)");
        assert_eq!(pg_canonical_actual("character varying(50)"), "varchar(50)");
        assert_ne!(pg_canonical_actual("varchar(50)"), pg_canonical_actual("text"));
        assert_eq!(pg_canonical_actual("numeric(10,2)"), "numeric(10,2)");
        assert_eq!(pg_canonical_actual("vector(1024)"), "vector(1024)");
    }

    /// 期望侧与实况侧必须落在同一条口径上 —— 这是「比较=结构相等」的前提。
    #[test]
    fn expected_and_actual_agree_on_canonical_form() {
        use sea_orm::sea_query::ColumnType as C;
        use sea_orm::sea_query::StringLen;
        let pairs: &[(C, &str)] = &[
            (C::String(StringLen::None), "text"),
            (C::String(StringLen::None), "character varying"),
            (C::Text, "text"),
            (C::String(StringLen::N(50)), "character varying(50)"),
            (C::String(StringLen::N(50)), "varchar(50)"),
            (C::BigInteger, "BIGINT"),
            (C::Integer, "integer"),
            (C::Double, "double precision"),
            (C::Float, "real"),
            (C::Boolean, "bool"),
            (C::Boolean, "boolean"),
            (C::JsonBinary, "jsonb"),
            (C::DateTime, "timestamp without time zone"),
            (C::Decimal(Some((10, 2))), "numeric(10,2)"),
            (C::Uuid, "uuid"),
        ];
        for (ct, actual) in pairs {
            let e = canonical_type_expected(ct, Dialect::Postgres);
            let a = canonical_type_actual(actual, Dialect::Postgres);
            assert_eq!(e, a, "期望侧 {ct:?} 与实况侧 {actual:?} 归一后应相等");
        }
    }

    /// SQLite 亲和性规则（官方 §3.1 五条，按序短路）。关键是 INT 先于 REAL/DOUB。
    #[test]
    fn sqlite_affinity_follows_official_rules() {
        for (declared, want) in [
            ("INT", "integer"),
            ("integer", "integer"),
            ("BIGINT", "integer"),
            ("tinyint", "integer"),
            ("CHARACTER(20)", "text"),
            ("VARCHAR(255)", "text"),
            ("varying character(255)", "text"),
            ("NCHAR(55)", "text"),
            ("native character(70)", "text"),
            ("nvarchar(100)", "text"),
            ("CLOB", "text"),
            ("text", "text"),
            ("BLOB", "blob"),
            ("", "blob"),
            ("REAL", "real"),
            ("DOUBLE", "real"),
            ("DOUBLE PRECISION", "real"),
            ("FLOAT", "real"),
            ("NUMERIC", "numeric"),
            ("DECIMAL(10,5)", "numeric"),
            ("BOOLEAN", "numeric"),
            ("DATE", "numeric"),
            ("DATETIME", "numeric"),
            ("string", "numeric"),
        ] {
            assert_eq!(sqlite_affinity(declared), want, "declared={declared:?}");
        }
    }

    /// ⚠ 亲和性是 SQLite 侧比较的**唯一**口径：声明名不同但同类 ⇒ 不算差异。
    /// 反之若混入 VARCHAR 这类未匹配到 INT/CHAR/TEXT 的字面名（实际会匹配 CHAR），
    /// 状态会漂到 numeric ⇒ 本测试锁死「分类只看亲和性类别」。
    #[test]
    fn sqlite_expected_matches_actual_across_equivalent_declarations() {
        use sea_orm::sea_query::ColumnType as C;
        use sea_orm::sea_query::StringLen;
        for (ct, actual) in [
            (C::String(StringLen::None), "text"),
            (C::String(StringLen::None), "varchar"),
            (C::Text, "TEXT"),
            (C::BigInteger, "INTEGER"),
            (C::Integer, "integer"),
            (C::Boolean, "INTEGER"),
            (C::Boolean, "BOOLEAN"),
            (C::Boolean, "bool"),
            (C::JsonBinary, "TEXT"),
            (C::Uuid, "TEXT"),
            (C::Blob, "BLOB"),
            (C::Double, "REAL"),
            // sea-query 对 `Decimal(None)` 的字面渲染就是 `real_decimal`
            // （`sea-query-1.0.2/src/backend/sqlite/table.rs:145`），不是笔误 —— 见下方口径分歧登记。
            (C::Decimal(None), "real_decimal"),
        ] {
            let e = canonical_type_expected(&ct, Dialect::Sqlite);
            let a = canonical_type_actual(actual, Dialect::Sqlite);
            assert_eq!(e, a, "SQLite：{ct:?} vs {actual:?} 亲和性应一致");
        }
    }

    /// 等价类**必须精确** —— 收拢过头会把真实差异静默吞掉（比误报差异更危险）。
    #[test]
    fn sqlite_equivalence_class_is_exactly_one_pair() {
        use sea_orm::sea_query::ColumnType as C;
        use sea_orm::sea_query::StringLen;
        // 收拢的那一对：Boolean ⇄ integer/boolean 双向相等
        assert_eq!(
            canonical_type_expected(&C::Boolean, Dialect::Sqlite),
            canonical_type_actual("INTEGER", Dialect::Sqlite)
        );
        // 不能顺手把 DECIMAL 也并进来（允许小数 vs 不允许）
        assert_ne!(
            canonical_type_expected(&C::Decimal(None), Dialect::Sqlite),
            canonical_type_actual("INTEGER", Dialect::Sqlite),
            "numeric 与 integer 是不同亲和性，合并会漏掉真实精度变更"
        );
        // ⚠ **已知口径分歧（P3 登记，未擅改）**：实体 `Decimal` 经 sea-query 渲染成
        // `real_decimal` / `real(p, s)`（→ REAL 亲和），而迁移时代的
        // `migrations/schema_diff.rs:309` 把 `Decimal` 写成 `NUMERIC`（→ NUMERIC 亲和）。
        // 两者**刻意不判等**：收拢它等于把「P4 首轮要不要为这些列重建表」这个语义
        // 决策藏进一个相等判断里。P3 如实报差异，处置留待裁决（已登记进 PLAN）。
        assert_ne!(
            canonical_type_expected(&C::Decimal(None), Dialect::Sqlite),
            canonical_type_actual("NUMERIC", Dialect::Sqlite),
            "Decimal 的两种声明口径不同，必须如实暴露"
        );
        assert_ne!(
            canonical_type_expected(&C::Boolean, Dialect::Sqlite),
            canonical_type_actual("TEXT", Dialect::Sqlite)
        );
        // 无长度字符串仍与 BLOB 区分（合并会让 TEXT ⇄ BLOB 互相判等）
        assert_ne!(
            canonical_type_expected(&C::String(StringLen::None), Dialect::Sqlite),
            canonical_type_actual("BLOB", Dialect::Sqlite)
        );
        // 纯函数层面不动官方规则：`boolean` 仍是 NUMERIC 亲和
        assert_eq!(sqlite_affinity("BOOLEAN"), "numeric");
    }

    /// partial 谓词的两侧形态**必须**判等：L2 声明写成 `(x IS NOT NULL)`（PG 风格），
    /// SQLite 侧是 DDL 原文 `x IS NOT NULL`（无外括号）。这条一旦失守，SQLite 每个
    /// partial 索引都会每轮被判「定义不一致」而反复重建。
    #[test]
    fn sqlite_partial_predicate_forms_are_equivalent() {
        assert_eq!(
            normalize_sql_expr("(branch_id IS NOT NULL)"),
            normalize_sql_expr("branch_id IS NOT NULL"),
            "外括号必须在归一后被抹平"
        );
        assert_eq!(
            normalize_sql_expr("(status = 'paused'::text)"),
            normalize_sql_expr("status = 'paused'"),
            "`::text` 也必须被抹平（PG 会补、SQLite 不会）"
        );
    }

    /// ⚠ **实测缺陷的回归判据**：末尾显式 `ASC` 是**默认方向** ⇒ 归一后必须消失。
    ///
    /// 第一现场（2026-09-16，`p5_engine_takeover --legacy-sqlite`）：声明侧
    /// `created_at`、实况侧 `created_at ASC` ⇒ 裸 `cols` 比较判不等 ⇒ 同名索引被拆成
    /// `DropIndex` + `CreateIndex` ⇒ `additive_only` 只留后者 ⇒ 撞「already exists」⇒
    /// **启动中止**。
    ///
    /// 区分力（下半段）：`DESC` / `NULLS LAST` / 带引号的 `"asc"` 都**必须保留** ——
    /// 否则「一律把尾巴剪掉」也能让上面那条通过，而那是把真差异一起抹了。
    #[test]
    fn trailing_default_asc_is_normalized_away_but_real_differences_are_not() {
        // ── 主题：ASC / asc 都是默认方向 ──
        assert_eq!(normalize_sql_expr("created_at ASC"), "created_at");
        assert_eq!(normalize_sql_expr("created_at asc"), "created_at");
        assert_eq!(normalize_sql_expr("created_at"), "created_at");
        // 与「双侧原文对照」一致：迁移建出的写法 == 声明写法
        assert_eq!(
            normalize_sql_expr("created_at ASC"),
            normalize_sql_expr("created_at"),
            "两侧必须判等，否则每轮 DROP+CREATE 且启动路径必中止"
        );
        // 空格先被折，故多空格 / 制表符都能命中
        assert_eq!(normalize_sql_expr("created_at   ASC"), "created_at");
        // 只剥末尾那一个（`cols` 是逐元素传入的，每个元素自带方向后缀）
        assert_eq!(normalize_sql_expr("a ASC"), "a");

        // ── 区分力：这些都不能被剥 ──
        assert_eq!(normalize_sql_expr("priority DESC"), "priority DESC", "DESC 有语义");
        assert_eq!(normalize_sql_expr("a ASC NULLS LAST"), "a ASC NULLS LAST", "NULLS 有语义");
        assert_eq!(normalize_sql_expr("\"asc\""), "\"asc\"", "引号内是标识符，不是关键字");
        assert_eq!(normalize_sql_expr("asc_x"), "asc_x", "另一列名不该被误剥");
        assert_eq!(normalize_sql_expr("floor(a)"), "floor(a)");
        // 单元素且以 ASC 结尾但属于表达式的一部分 ⇒ `asc` 前面没有空白 ⇒ 不剥
        assert_eq!(normalize_sql_expr("asc"), "asc", "整个串就是 asc 时不剥（没有可剥的空白）");
    }

    /// ★ 本轮缺陷的**回归测试**：声明用 SQL 标准写法、PG 回读用算子记号，
    /// 归一后必须相等 —— 否则 `plan::same_index` 恒判不等，启动路径撞「已存在」fail-stop。
    ///
    /// 两个串都是**实测形态**：左侧是 `pg_get_indexdef` 对生产库里
    /// `uq_trajectory_patterns_name` 的回读原文（`output/tmp-final-pg-state.log`）。
    #[test]
    fn like_operator_spellings_normalize_equal() {
        assert_eq!(
            normalize_sql_expr("(name !~~ 'rl_checkpoint:%'::text)"),
            normalize_sql_expr("(name NOT LIKE 'rl_checkpoint:%')"),
            "PG 算子记号与 SQL 标准写法必须归一成同一个串"
        );
        // 四个算子逐个对照（含最长匹配的那一对）。
        assert_eq!(normalize_sql_expr("a ~~ 'x'"), "a LIKE 'x'");
        assert_eq!(normalize_sql_expr("a !~~ 'x'"), "a NOT LIKE 'x'");
        assert_eq!(normalize_sql_expr("a ~~* 'x'"), "a ILIKE 'x'");
        assert_eq!(normalize_sql_expr("a !~~* 'x'"), "a NOT ILIKE 'x'");
    }

    /// 折叠的**区分力**：它会折「同一算子的两种写法」，但绝不能折掉真实差异。
    ///
    /// 这一条比上一条更重要 —— 判等位置上的归一化，失效方向有两个：折少了（引擎
    /// 反复重建）与折多了（引擎看不见该做的事）。上一条挡后者，本条挡前者。
    #[test]
    fn like_folding_keeps_real_differences() {
        // ① 谓词里的字面量不同 ⇒ 仍必须不等
        assert_ne!(
            normalize_sql_expr("(name !~~ 'rl_checkpoint:%')"),
            normalize_sql_expr("(name !~~ 'rl_checkpointX%')"),
        );
        // ② 算子本身就不同（LIKE vs ILIKE）⇒ 不等，别把 ILIKE 也折成 LIKE
        assert_ne!(normalize_sql_expr("a ~~ 'x'"), normalize_sql_expr("a ~~* 'x'"));
        // ③ 否定与非否定 ⇒ 不等
        assert_ne!(normalize_sql_expr("a ~~ 'x'"), normalize_sql_expr("a !~~ 'x'"));
        // ④ 引号内的 `~~` 是字面量，不是算子 ⇒ 原样保留，且两串不等
        assert_eq!(normalize_sql_expr("'a~~b'"), "'a~~b'");
        assert_ne!(normalize_sql_expr("a = 'x~~y'"), normalize_sql_expr("a = 'xLIKEy'"));
        // ⑤ 正则算子 `~` / `!~` 不折（SQLite 无等价算子，见函数文档的边界）
        assert_eq!(normalize_sql_expr("period ~ '^\\d{4}$'"), "period ~ '^\\d{4}$'");
        assert_eq!(normalize_sql_expr("a !~ 'x'"), "a !~ 'x'");
    }

    /// `strip_casts` 必须**按字符**搬运：非 ASCII 不能被逐字节拆坏。
    ///
    /// 实测形态：含中文的谓词经 SQLite 渲染会写出乱码 DDL（原实现 `bytes[i] as char`）。
    /// 本测试钉住「剥 `::text` 的同时中文原样保留」。
    #[test]
    fn strip_casts_preserves_multibyte_text() {
        assert_eq!(strip_casts("status = '暂停'::text"), "status = '暂停'");
        // 引号外也要保真：中文标识符 + 转换
        assert_eq!(strip_casts("名称 = '值'::text"), "名称 = '值'");
        // 纯 ASCII 行为不变（回归）
        assert_eq!(strip_casts("'paused'::text"), "'paused'");
        assert_eq!(strip_casts("'a::b'::text"), "'a::b'");
        // 长度自证：3 字节的汉字被拆坏会让长度变成 3 个字符各占 2 字节 ⇒ 长度不同
        assert_eq!(strip_casts("'暂停'").chars().count(), 4);
        assert_eq!(strip_casts("'暂停'").len(), 2 + 6);
    }

    /// 默认值归一：剥配平外括号 + 折空白；不配平则不动（防切坏 `('(a)')`）。
    #[test]
    fn default_normalization_is_conservative() {
        assert_eq!(normalize_default("  'x'::text "), "'x'::text");
        assert_eq!(normalize_default("('x'::text)"), "'x'::text");
        assert_eq!(normalize_default("((0))"), "0");
        assert_eq!(normalize_default("'a'::character  varying"), "'a'::character varying");
        // 不配平：`(a))` 去掉首尾后剩 `a)` ⇒ 括号不配平 ⇒ 保留原样
        assert_eq!(normalize_default("(a))"), "(a))");
        // 单引号内的括号不参与计数
        assert_eq!(normalize_default("('(')"), "'('");
    }

    /// `upsert_*` 按名替换而非叠加（L2 覆盖 L1 同名列时不能出现两列）。
    #[test]
    fn upsert_replaces_by_name() {
        let mut t = TableModel::new("t");
        t.upsert_column(ColumnModel::new("a", "text", true));
        let mut c = ColumnModel::new("a", "bigint", false);
        c.primary_key = true;
        t.upsert_column(c);
        assert_eq!(t.columns.len(), 1, "同名应替换而不是追加");
        assert_eq!(t.column("a").unwrap().sql_type, "bigint");

        t.claim_extra("fts5:notes_fts");
        t.claim_extra("fts5:notes_fts");
        assert_eq!(t.extras.len(), 1, "extras 去重");
    }
}
