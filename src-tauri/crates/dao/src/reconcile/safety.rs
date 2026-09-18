// SPDX-License-Identifier: AGPL-3.0-only

//! `reconcile::safety` — apply 的安全网（P4-3）。
//!
//! ## 这个模块存在的前提：引擎从「只读」变成「会写」
//!
//! P0–P3 的引擎只发 `SELECT`，错了最多是「打印错一份计划」。一旦 apply 开始执行
//! DDL，错的代价就是**删数据**。所以安全网不是「附加功能」，而是**开启写入的前置
//! 条件**。这里把七道闸写在一处，且每一道都能在 SQLite 内存库上端到端验证。
//!
//! ## 七道闸
//!
//! | # | 闸 | 触发条件 | 默认 |
//! |---|---|---|---|
//! | 1 | 逃生阀 `dry_run` | `AX_SCHEMA_DRY_RUN` 未设或无法识别 | **开着**（只渲染不执行） |
//! | 2 | 破坏性配额 | 单轮 `destructive` 数 > `max_destructive_per_run` | 5 |
//! | 3 | 基数熔断 | 期望表数 < 上次记录值 | 拒（须 `AX_SCHEMA_ALLOW_BASE_SHRINK=1`） |
//! | 4 | 墓碑证据 | `loses_data` 变更缺导出证据 | 整批中止 |
//! | 5 | 延迟一个周期 | 破坏性变更首次出现 | 只登记，下轮才执行 |
//! | 6 | 人工白名单 | `_ax_schema_orphan_whitelist` 命中且未过期 | 不判孤儿 |
//! | 7 | 审计留痕 | 每一条**无论执行与否**都写 `_ax_schema_audit` | 强制 |
//!
//! ## 元表为什么用「双方言兼容」的 DDL
//!
//! 四张簿记表只用 `TEXT` / `BIGINT`，不用 `jsonb` / `timestamptz`。这不是审美选择：
//! `render` 只做 PG（见 `render.rs` 模块文档），但**簿记不该跟着那个限制走** ——
//! 写成双方言兼容，全部闸判断就能在 `sqlite::memory:` 上端到端跑（CI 没有 PG）。
//! 用 `jsonb` 的代价是「安全网的核心逻辑永远无法在 CI 里被验证」，那等于没有安全网。
//!
//! 时间戳一律存 **epoch 秒**：双方言都是整数比较，不需要格式化依赖，
//! 也不会因时区/解析差异造出「看起来恒为 False」的判据（本仓踩过同类坑）。
//!
//! ⚠⚠ **整型列必须写 `BIGINT`，不能写 `INTEGER`** —— 这是 2026-09-16 真库实测撞到的
//! 缺陷（第一现场：把 2 张非空孤儿表登记进白名单后，`apply::cycle` 在 PG 上直接失败）：
//!
//! ```text
//! 失败: Query Error: error occurred while decoding column "expires_at":
//!       mismatched types; Rust type `core::option::Option<i64>` (as SQL type `INT8`)
//!       is not compatible with SQL type `INT4`
//! ```
//!
//! 三个条件**同时**成立才暴露，所以它在 CI 上永远红不了：
//!
//! 1. PG 的 `INTEGER` 是 **int4**（只有 `BIGINT` 是 int8），而 sqlx 对 `i64` 的解码
//!    **要求** int8 ⇒ 类型不匹配是硬错，不是「值太大才出错」；
//! 2. SQLite 的 `INTEGER` 亲和性本来就是 64 位 ⇒ `i64` 天然匹配，
//!    `sqlite::memory:` 上的测试**全是绿的**；
//! 3. [`load_orphan_whitelist`] 的读取在 `for r in rows` 里 ⇒ **表空时循环体一次都不执行**，
//!    解码检查根本不跑。于是「白名单从来没有内容」这件事一直在盖住它 ——
//!    直到第一次真的登记一条，整条 apply 路径当场不可用。
//!
//! 对照实证，也是**不要按「哪张表报错」定位范围**的理由：`_ax_schema_pending_drops`
//! 当时已有 8 行却从未报错，因为读它的 [`load_pending_drops`] **只取 `table_name`**
//! （`TEXT`）—— 不读整型列就不触发解码。同一个缺陷在不同读取点上表现不同，
//! 唯一可靠的穷举口径是「Rust 侧类型是 `i64`」⇒ 4 张表的整型列**全部**对齐 `BIGINT`。
//!
//! ⚠ 而 `META_DDL` 是 `CREATE TABLE IF NOT EXISTS` ⇒ 改 DDL **改不动已存在的表**。
//! [`PG_INT_WIDENING`] 就是补那一半的（PG 专有的幂等列加宽）。
//!
//! ## 元表对引擎不可见
//!
//! 四张表的表名都以 `_ax_schema_` 开头 ⇒ [`crate::reconcile::introspect::read`]
//! **不读它们**（见 `introspect::ENGINE_META_PREFIX`）。所以它们既不进指纹、也不判
//! 孤儿。`tests::meta_table_names_all_hit_the_prefix` 从**这一头**守住同一条约定 ——
//! 两侧各自锁定，避免「约定只写在一边的注释里」。
//!
//! ## 与 `render` 的分工
//!
//! 本模块**不生成也不执行任何业务 DDL**：DDL 文本由 `render` 产出，本模块只负责
//! 「要不要执行」「执行前必须有什么证据」「执行了什么」这三件事。这样安全判据与
//! 渲染逻辑可以各自单测，互不污染。

use std::collections::{BTreeMap, BTreeSet};

use sea_orm::{ConnectionTrait, DatabaseConnection, DbBackend, DbErr, Statement};

use super::plan::{Change, Plan};

// ═══════════════════════════════════════════════════════════════════════════
// 元表
// ═══════════════════════════════════════════════════════════════════════════

/// 审计表：每一条变更**无论执行与否**都留一行（闸 7）。
pub const META_AUDIT: &str = "_ax_schema_audit";
/// 墓碑表：「导出成功才允许 DROP」的落点（闸 4）。
pub const META_GRAVEYARD: &str = "_ax_schema_graveyard";
/// 待删表：破坏性变更延迟一个周期的状态（闸 5）。
pub const META_PENDING: &str = "_ax_schema_pending_drops";
/// 人工白名单：临时豁免孤儿判定（闸 6）。
pub const META_WHITELIST: &str = "_ax_schema_orphan_whitelist";

/// 四张元表（顺序与 [`META_DDL`] 一一对应）。
pub const META_TABLES: &[&str] = &[META_AUDIT, META_GRAVEYARD, META_PENDING, META_WHITELIST];

/// 建表语句。`IF NOT EXISTS` ⇒ 幂等，可每次 apply 前无条件执行。
///
/// ⚠ 类型只允许 `TEXT` / `BIGINT`，且**不用自增主键**（PG 是 `BIGSERIAL`、SQLite 是
/// `INTEGER PRIMARY KEY AUTOINCREMENT` —— 两套写法）。簿记主键一律是
/// `(run_id, seq)` 复合主键，值由调用方给出，双方言逐字相同。
///
/// ⚠ 整型一律 `BIGINT`，**不要写 `INTEGER`** —— 理由见模块文档那段真库实测：
/// PG 的 `INTEGER` 是 int4，而 Rust 侧这些列一律 `i64`（sqlx 解码要求 int8）。
/// `tests::meta_ddl_has_no_bare_integer_columns` 从**这一头**锁住它（PG 上的解码
/// 错误在 CI 里跑不出来，所以只能在这里用形态断言拦）。
///
/// ⚠ 布尔用 `BIGINT` 存 0/1：SQLite 没有 `BOOLEAN` 类型（只有亲和性），
/// 写 `BOOLEAN` 会在两侧落到不同的亲和性上，读回来还得各自解释。
pub const META_DDL: &[&str] = &[
    "CREATE TABLE IF NOT EXISTS _ax_schema_audit (
    run_id      TEXT   NOT NULL,
    seq         BIGINT NOT NULL,
    kind        TEXT   NOT NULL,
    object      TEXT   NOT NULL,
    destructive BIGINT NOT NULL,
    loses_data  BIGINT NOT NULL,
    executed    BIGINT NOT NULL,
    statements  TEXT   NOT NULL,
    notes       TEXT,
    error       TEXT,
    created_at  BIGINT NOT NULL,
    PRIMARY KEY (run_id, seq)
)",
    "CREATE TABLE IF NOT EXISTS _ax_schema_graveyard (
    run_id        TEXT   NOT NULL,
    seq           BIGINT NOT NULL,
    table_name    TEXT   NOT NULL,
    ddl           TEXT   NOT NULL,
    row_count     BIGINT,
    export_path   TEXT   NOT NULL,
    export_sha256 TEXT   NOT NULL,
    created_at    BIGINT NOT NULL,
    PRIMARY KEY (run_id, seq)
)",
    "CREATE TABLE IF NOT EXISTS _ax_schema_pending_drops (
    table_name    TEXT   NOT NULL PRIMARY KEY,
    first_run_id  TEXT   NOT NULL,
    first_seen_at BIGINT NOT NULL,
    reason        TEXT   NOT NULL
)",
    "CREATE TABLE IF NOT EXISTS _ax_schema_orphan_whitelist (
    table_name TEXT   NOT NULL PRIMARY KEY,
    reason     TEXT   NOT NULL,
    expires_at BIGINT NOT NULL
)",
];

/// PG 专有：把**已存在**元表的 int4 整型列加宽到 int8。
///
/// ## 为什么需要它（`ALTER` 而不是只改 `META_DDL`）
///
/// [`META_DDL`] 是 `CREATE TABLE IF NOT EXISTS` ⇒ 它**改不动已经建出来的表**。
/// 于是「把 DDL 里的 `INTEGER` 改成 `BIGINT`」只对**新建的库**生效，而任何已经跑过
/// `apply` 的库（本地开发库、生产库）里那些列仍是 int4 ⇒ 同一个解码缺陷原封不动。
/// 这一段补的就是那一半 —— 少了它，修复只在「从零开始的环境」里成立，
/// 而真正出问题的那台机器上什么都没变（本仓已记录过这一类「改了却没用」）。
///
/// ## 为什么只在 PG 上跑
///
/// SQLite 不支持 `ALTER COLUMN … TYPE`，而它**也不需要**：SQLite 的 `INTEGER`
/// 亲和性本来就是 64 位，`i64` 从来就匹配。（若哪天误在 SQLite 上执行这段，
/// 会得到语法错误而不是静默成功 —— 这是可接受的失败方向。）
///
/// ## 幂等性
///
/// 目标类型已是 int8 时 PG 判定「无需重写」并跳过（新类型与旧类型相同 ⇒
/// `ATColumnChangeRequiresRewrite` 为假），故每次 `ensure_meta_tables` 无条件跑一遍
/// 是安全的、也不会每次重写表。
///
/// ⚠ 清单必须与 [`META_DDL`] 的整型列**一一对应** ——
/// `tests::pg_int_widening_covers_every_integer_column` 锁住这条对应关系
/// （硬断言条数，将来给元表加一个整型列而忘了加 ALTER 时当场变红）。
const PG_INT_WIDENING: &[&str] = &[
    "ALTER TABLE _ax_schema_audit ALTER COLUMN seq TYPE bigint",
    "ALTER TABLE _ax_schema_audit ALTER COLUMN destructive TYPE bigint",
    "ALTER TABLE _ax_schema_audit ALTER COLUMN loses_data TYPE bigint",
    "ALTER TABLE _ax_schema_audit ALTER COLUMN executed TYPE bigint",
    "ALTER TABLE _ax_schema_audit ALTER COLUMN created_at TYPE bigint",
    "ALTER TABLE _ax_schema_graveyard ALTER COLUMN seq TYPE bigint",
    "ALTER TABLE _ax_schema_graveyard ALTER COLUMN row_count TYPE bigint",
    "ALTER TABLE _ax_schema_graveyard ALTER COLUMN created_at TYPE bigint",
    "ALTER TABLE _ax_schema_pending_drops ALTER COLUMN first_seen_at TYPE bigint",
    "ALTER TABLE _ax_schema_orphan_whitelist ALTER COLUMN expires_at TYPE bigint",
];

/// 该库是否需要跑 [`PG_INT_WIDENING`]（只有 PG 需要）。
///
/// 抽成函数而不是在 `ensure_meta_tables` 里写 `if backend == …`：这样
/// 「SQLite 上不跑」这件事可以被**单测直接断言**，而不是只能靠「跑一遍看会不会报错」。
fn needs_int_widening(backend: DbBackend) -> bool {
    backend == DbBackend::Postgres
}

/// 建出四张元表（幂等）。apply 的**第一步**就是它。
///
/// 失败即返回错误 ⇒ 调用方整批中止：簿记都没建起来时执行 DDL，等于把「这次做了什么」
/// 永久丢失。
///
/// 顺序是**先建表、再列加宽**（[`PG_INT_WIDENING`]）：加宽针对的是已存在的表，
/// 所以它必须排在 `CREATE TABLE IF NOT EXISTS` 之后 —— 反过来在**全新的库**上会
/// `relation does not exist`。这个顺序不是风格问题，是正确性问题。
pub async fn ensure_meta_tables(db: &DatabaseConnection) -> Result<(), DbErr> {
    let backend = db.get_database_backend();
    for ddl in META_DDL {
        db.execute_raw(Statement::from_string(backend, (*ddl).to_string())).await?;
    }
    // `CREATE TABLE IF NOT EXISTS` 改不动旧表 ⇒ 历史 int4 列在这里补齐（见常量文档）。
    if needs_int_widening(backend) {
        for sql in PG_INT_WIDENING {
            db.execute_raw(Statement::from_string(backend, (*sql).to_string())).await?;
        }
    }
    Ok(())
}

// ═══════════════════════════════════════════════════════════════════════════
// 时间与占位符
// ═══════════════════════════════════════════════════════════════════════════

/// 当前 epoch 秒 —— 本模块**唯一**的时间来源。
///
/// 为什么不用格式化时间：见模块文档（双方言整数比较、无时区解析风险）。
/// 系统时钟早于 `UNIX_EPOCH`（几乎不可能）时返回 0，而不是 panic。
pub fn now_epoch() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// 生成 `n` 个参数占位符：PG 是 `$1, $2, …`，SQLite 是 `?, ?, …`。
///
/// ⚠ **必须按方言生成**：`Statement::from_sql_and_values` 拿到的 SQL 是原样的，
/// 它不做占位符转换。写死一种 ⇒ 另一种方言上整条语句语法错误。
fn placeholders(backend: DbBackend, n: usize) -> String {
    (1..=n)
        .map(|i| {
            if backend == DbBackend::Postgres {
                format!("${i}")
            } else {
                "?".to_string()
            }
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// `run_id` —— 一次 apply 运行的标识（审计与墓碑靠它成组）。
///
/// 形如 `run-<epoch 秒>-<序号>`：同一秒内的多次运行靠调用方给的序号区分。
/// **故意不引入随机数依赖** —— run_id 的唯一性要求只是「同一台机器上不撞」，
/// 秒 + 序号已足够，而多加一个依赖就多一处供应链面。
pub fn make_run_id(now_epoch: i64, ordinal: u32) -> String {
    format!("run-{now_epoch}-{ordinal:03}")
}

/// 取一个**当前未被占用**的 `run_id`。
///
/// ⚠ 为什么不能直接用 `make_run_id(now, ordinal)`：`(run_id, seq)` 是审计表的**主键**，
/// 而 `run_id` 只由「epoch 秒 + 序号」构成。同一秒内跑两次（冒烟探针就是这么做的，
/// 任何重试也是）且序号相同 ⇒ 主键冲突。实测症状：
/// `UNIQUE constraint failed: _ax_schema_audit.run_id, _ax_schema_audit.seq`。
///
/// 让引擎自己找空位，比要求调用方保证唯一更可靠 —— **默认参数恰恰是最容易撞的那个**
/// （`ordinal` 默认 0）。而「同秒重跑」不是异常路径，是最常见的调试路径。
///
/// 并发场景下两个进程仍可能同时选中同一个空位 —— 那时由审计表的主键约束**报错**，
/// 这是正确的失败方式（响的），不是需要在这里靠锁去消掉的（静默的）。
pub async fn allocate_run_id(
    db: &DatabaseConnection,
    now_epoch: i64,
    first_ordinal: u32,
) -> Result<String, DbErr> {
    ensure_meta_tables(db).await?;
    let backend = db.get_database_backend();
    for o in first_ordinal..=first_ordinal.saturating_add(1000) {
        let candidate = make_run_id(now_epoch, o);
        let sql = format!(
            "SELECT run_id FROM {META_AUDIT} WHERE run_id = {} LIMIT 1",
            placeholders(backend, 1)
        );
        let taken = !db
            .query_all_raw(Statement::from_sql_and_values(backend, sql, [candidate.clone().into()]))
            .await?
            .is_empty();
        if !taken {
            return Ok(candidate);
        }
    }
    Err(DbErr::Custom(format!(
        "无法为 epoch {now_epoch} 分配 run_id：从序号 {first_ordinal} 起连续 1000 个都被占用"
    )))
}

// ═══════════════════════════════════════════════════════════════════════════
// 配置（闸 1–3 / 5）
// ═══════════════════════════════════════════════════════════════════════════

pub const ENV_DRY_RUN: &str = "AX_SCHEMA_DRY_RUN";
pub const ENV_MAX_DESTRUCTIVE: &str = "AX_SCHEMA_MAX_DESTRUCTIVE";
pub const ENV_DEFER_DESTRUCTIVE: &str = "AX_SCHEMA_DEFER_DESTRUCTIVE";
pub const ENV_ALLOW_BASE_SHRINK: &str = "AX_SCHEMA_ALLOW_BASE_SHRINK";

/// 安全网的运行期配置。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SafetyConfig {
    /// 逃生阀：`true` = 只渲染与写审计，**不执行任何 DDL**。
    pub dry_run: bool,
    /// 单轮允许的破坏性变更上限。
    pub max_destructive_per_run: usize,
    /// 破坏性操作是否延迟一个周期（首次见到只登记）。
    pub defer_destructive: bool,
    /// 是否允许「期望表数」相对上次下降（下降通常意味着声明被误删）。
    pub allow_base_shrink: bool,
}

impl Default for SafetyConfig {
    /// **全部取安全方向**：逃生阀开、配额 5、延迟执行、不允许基数下降。
    fn default() -> Self {
        Self {
            dry_run: true,
            max_destructive_per_run: 5,
            defer_destructive: true,
            allow_base_shrink: false,
        }
    }
}

impl SafetyConfig {
    /// 从进程环境读（生产路径）。
    pub fn from_env() -> Self {
        Self::from_lookup(|k| std::env::var(k).ok())
    }

    /// 从注入的查询函数读 —— **测试用**：不碰进程环境，避免测试间互相污染
    /// （`std::env::set_var` 是进程级全局，并行测试下会让判据互相打架）。
    pub fn from_lookup(lookup: impl Fn(&str) -> Option<String>) -> Self {
        let d = Self::default();
        Self {
            // 逃生阀：**默认 ON**，只有明确写成关才关（见 `switch_default_on`）
            dry_run: switch_default_on(lookup(ENV_DRY_RUN).as_deref()),
            // 配额：解析失败 ⇒ 用默认（默认比「无上限」保守）
            max_destructive_per_run: parse_limit(
                lookup(ENV_MAX_DESTRUCTIVE).as_deref(),
                d.max_destructive_per_run,
            ),
            defer_destructive: switch_default_on(lookup(ENV_DEFER_DESTRUCTIVE).as_deref()),
            allow_base_shrink: switch_default_off(lookup(ENV_ALLOW_BASE_SHRINK).as_deref()),
        }
    }
}

/// 解析一个「**默认为开**」的开关。
///
/// 只有明确写成「关」才关；**未设置、空串、写错（`treu` / `1 ` 之外的一切）都算开**。
///
/// 为什么方向是这个：它是**逃生阀**，误判的代价不对称 —— 「本该执行却被拦住」只浪费
/// 一次运行，「本该拦住却执行了」会删数据。任何「解析失败就当作关」的写法，都会让一个
/// 手误的环境变量直接把门打开。
fn switch_default_on(raw: Option<&str>) -> bool {
    !matches!(
        raw.map(str::trim).map(str::to_ascii_lowercase).as_deref(),
        Some("0") | Some("false") | Some("off") | Some("no")
    )
}

/// 解析一个「**默认为关**」的开关：只有明确写成开才开。
fn switch_default_off(raw: Option<&str>) -> bool {
    matches!(
        raw.map(str::trim).map(str::to_ascii_lowercase).as_deref(),
        Some("1") | Some("true") | Some("on") | Some("yes")
    )
}

/// 解析数值上限：**解析失败 ⇒ 用默认值**，不退化成「无上限」。
///
/// 写错 `AX_SCHEMA_MAX_DESTRUCTIVE=lots` 的结果是「仍然只允许 5 条」，而不是放行一切。
fn parse_limit(raw: Option<&str>, default: usize) -> usize {
    raw.and_then(|s| s.trim().parse::<usize>().ok()).unwrap_or(default)
}

// ═══════════════════════════════════════════════════════════════════════════
// 闸 2 + 3：熔断
// ═══════════════════════════════════════════════════════════════════════════

/// 熔断原因（`None` = 放行）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    /// 破坏性变更数超配额。
    TooManyDestructive { n: usize, limit: usize },
    /// 期望表数相对上次**下降**。
    BaseShrank { now: usize, last: usize },
}

impl Refusal {
    /// 人读的一行说明（进日志与审计）。
    pub fn reason(&self) -> String {
        match self {
            Self::TooManyDestructive { n, limit } => format!(
                "破坏性变更 {n} 条，超过单轮配额 {limit} 条（调 `{ENV_MAX_DESTRUCTIVE}` 显式放宽）"
            ),
            Self::BaseShrank { now, last } => format!(
                "期望表数从 {last} 降到 {now} —— 声明被误删时会这样。\
                 确认要删表则设 `{ENV_ALLOW_BASE_SHRINK}=1`"
            ),
        }
    }
}

/// 熔断判据（闸 2 + 闸 3）。两道判据都要跑，返回**第一条**命中的。
///
/// ⚠ 「期望表数下降」为什么也要拒：`expected` 是从 L1 实体 + L2 声明算出来的。
/// 若某次改动误删了一批实体（或 `expected::build` 出错漏读），期望侧就会塌陷成
/// 几十张表 —— 而 plan 看到的是「实况有 200 张、期望只有 30 张」⇒ **170 条 DROP TABLE**。
/// 这类事故在只读阶段只是一份错误报告，在 apply 阶段是全库删除。
/// 「基数下降必须显式放行」是这条事故的专用闸。
pub fn check_circuit_breakers(
    plan: &Plan,
    expected_table_count: usize,
    last_expected_table_count: Option<usize>,
    cfg: &SafetyConfig,
) -> Option<Refusal> {
    if plan.n_destructive > cfg.max_destructive_per_run {
        return Some(Refusal::TooManyDestructive {
            n: plan.n_destructive,
            limit: cfg.max_destructive_per_run,
        });
    }
    if let Some(last) = last_expected_table_count
        && expected_table_count < last
        && !cfg.allow_base_shrink
    {
        return Some(Refusal::BaseShrank { now: expected_table_count, last });
    }
    None
}

// ═══════════════════════════════════════════════════════════════════════════
// 闸 4：墓碑证据
// ═══════════════════════════════════════════════════════════════════════════

/// 一条 `loses_data` 变更的**导出证据**。
///
/// 「导出成功才允许 DROP」这条契约里，safety **不自己去导出** —— 它不知道要导什么
/// （整表？某一列？），也不该猜。它只**强制证据存在**：没有证据就整批中止，
/// 让人先把数据拿出来。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GraveEvidence {
    /// 导出文件路径（相对或绝对都可，只要求非空）。
    pub export_path: String,
    /// 导出内容的 sha256（前若干位即可，只要求非空）。
    pub export_sha256: String,
    /// 导出时的行数。`None` = 未统计（**不是 0 行**）。
    pub row_count: Option<i64>,
}

impl GraveEvidence {
    /// 证据是否**完整**。空 path 或空 sha256 都算不完整。
    ///
    /// 为什么空 sha256 不接受：它正是「写了文件但没校验」的形态 —— 导出脚本中途失败
    /// 会留下一个半截文件，而只有回读 + 指纹比对才能区分「导成功」与「文件存在」。
    pub fn is_complete(&self) -> bool {
        !self.export_path.trim().is_empty() && !self.export_sha256.trim().is_empty()
    }
}

/// 找出**缺少导出证据**的 `loses_data` 变更（按 `Change::object` 索引证据）。
///
/// 返回非空 ⇒ 调用方**必须整批中止**。不是「跳过这几条」—— 跳过会留下一个半同步的
/// 库（一半改了、一半没改），而引擎下次启动时无法区分「未同步」与「被外部改动」。
pub fn missing_evidence<'a>(
    plan: &'a Plan,
    provided: &BTreeMap<String, GraveEvidence>,
) -> Vec<&'a Change> {
    plan.changes
        .iter()
        .filter(|c| c.loses_data)
        .filter(|c| provided.get(&c.object).map(|e| !e.is_complete()).unwrap_or(true))
        .collect()
}

// ═══════════════════════════════════════════════════════════════════════════
// 闸 5：延迟一个周期
// ═══════════════════════════════════════════════════════════════════════════

/// 破坏性变更的延迟判定。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeferDecision {
    /// 首次出现 ⇒ 只登记进 `_ax_schema_pending_drops`，**本轮不执行**。
    DeferFirstSight,
    /// 已在待删表里出现过 ⇒ 放行执行。
    Execute,
}

/// 判定一条待删表该「登记」还是「执行」。
///
/// 「一个周期」= 一次 apply 运行，判据是**表名是否是待删表的既有行**，
/// 而不是时间间隔 —— 两次启动可能相隔一秒，也可能相隔一周，时间间隔不构成「周期」。
///
/// 配置关掉延迟（`AX_SCHEMA_DEFER_DESTRUCTIVE=0`）⇒ 恒 `Execute`。
pub fn defer_decision(
    table: &str,
    pending: &BTreeSet<String>,
    cfg: &SafetyConfig,
) -> DeferDecision {
    if !cfg.defer_destructive {
        return DeferDecision::Execute;
    }
    if pending.contains(table) {
        DeferDecision::Execute
    } else {
        DeferDecision::DeferFirstSight
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// 闸 7：审计
// ═══════════════════════════════════════════════════════════════════════════

/// 一条审计行。**每一条变更都写**，包括被逃生阀拦下、被延迟、被白名单豁免的。
///
/// 为什么「未执行」也要写：日志里只有 `Compiling`-式的成功行，无法区分「没这条变更」
/// 与「有但被拦了」。审计是「这次运行到底决定了什么」的唯一记录。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuditRow {
    pub seq: usize,
    pub kind: &'static str,
    pub object: String,
    pub destructive: bool,
    pub loses_data: bool,
    /// 是否**真的**执行了（`false` = 被 dry-run / 延迟 / 熔断拦下）。
    pub executed: bool,
    pub statements: Vec<String>,
    pub notes: Vec<String>,
    pub error: Option<String>,
}

/// 把一批审计行写入 `_ax_schema_audit`。
///
/// `statements` / `notes` 以 JSON 数组存文本：它们的长度与条数都不定，拆成列会让
/// 表结构随渲染器演进而改（而元表一旦建出来就不能随版本改结构）。
pub async fn write_audit(
    db: &DatabaseConnection,
    run_id: &str,
    rows: &[AuditRow],
    now: i64,
) -> Result<(), DbErr> {
    if rows.is_empty() {
        return Ok(());
    }
    let backend = db.get_database_backend();
    for r in rows {
        let sql = format!(
            "INSERT INTO {META_AUDIT}
    (run_id, seq, kind, object, destructive, loses_data, executed, statements, notes, error, created_at)
VALUES ({})",
            placeholders(backend, 11)
        );
        let statements = serde_json::to_string(&r.statements).unwrap_or_else(|_| "[]".to_string());
        let notes = match r.notes.is_empty() {
            true => None,
            false => Some(serde_json::to_string(&r.notes).unwrap_or_else(|_| "[]".to_string())),
        };
        db.execute_raw(Statement::from_sql_and_values(
            backend,
            sql,
            [
                run_id.into(),
                (r.seq as i64).into(),
                r.kind.into(),
                r.object.clone().into(),
                (r.destructive as i64).into(),
                (r.loses_data as i64).into(),
                (r.executed as i64).into(),
                statements.into(),
                notes.into(),
                r.error.clone().into(),
                now.into(),
            ],
        ))
        .await?;
    }
    Ok(())
}

/// 写一条墓碑。**必须在 `DROP` 之前**，且失败要中止整批（由调用方 `?` 保证）。
///
/// `ddl` 存的是将被执行的 DDL 原文 —— 墓碑的作用是「事后能知道当初删了什么」，
/// 只记表名不够（表名相同的两张表在不同时间可能结构不同）。
pub async fn write_graveyard(
    db: &DatabaseConnection,
    run_id: &str,
    seq: usize,
    table_name: &str,
    ddl: &str,
    ev: &GraveEvidence,
    now: i64,
) -> Result<(), DbErr> {
    // 契约在这里再判一次（不只在调用方判）：墓碑是最后一道，它自己必须拒绝无证据的写入，
    // 否则「导出成功才允许 DROP」就只靠调用方的自觉。
    if !ev.is_complete() {
        return Err(DbErr::Custom(format!(
            "墓碑拒绝写入：`{table_name}` 的导出证据不完整（path/sha256 不能为空）"
        )));
    }
    let backend = db.get_database_backend();
    let sql = format!(
        "INSERT INTO {META_GRAVEYARD}
    (run_id, seq, table_name, ddl, row_count, export_path, export_sha256, created_at)
VALUES ({})",
        placeholders(backend, 8)
    );
    db.execute_raw(Statement::from_sql_and_values(
        backend,
        sql,
        [
            run_id.into(),
            (seq as i64).into(),
            table_name.into(),
            ddl.into(),
            ev.row_count.into(),
            ev.export_path.clone().into(),
            ev.export_sha256.clone().into(),
            now.into(),
        ],
    ))
    .await?;
    Ok(())
}

// ═══════════════════════════════════════════════════════════════════════════
// 闸 5（状态）+ 闸 6：待删表与白名单的读写
// ═══════════════════════════════════════════════════════════════════════════

/// 读待删表里的全部表名（闸 5 的状态）。
pub async fn load_pending_drops(db: &DatabaseConnection) -> Result<BTreeSet<String>, DbErr> {
    let backend = db.get_database_backend();
    let rows = db
        .query_all_raw(Statement::from_string(
            backend,
            format!("SELECT table_name FROM {META_PENDING}"),
        ))
        .await?;
    let mut out = BTreeSet::new();
    for r in rows {
        out.insert(r.try_get::<String>("", "table_name")?);
    }
    Ok(out)
}

/// 登记一条待删表（闸 5：首次见到时调用）。已存在则**不覆盖** `first_seen_at`。
pub async fn record_pending_drop(
    db: &DatabaseConnection,
    table: &str,
    run_id: &str,
    reason: &str,
    now: i64,
) -> Result<(), DbErr> {
    let backend = db.get_database_backend();
    let sql = format!(
        "SELECT table_name FROM {META_PENDING} WHERE table_name = {}",
        placeholders(backend, 1)
    );
    let exists = !db
        .query_all_raw(Statement::from_sql_and_values(backend, sql, [table.into()]))
        .await?
        .is_empty();
    if exists {
        return Ok(());
    }
    let sql = format!(
        "INSERT INTO {META_PENDING} (table_name, first_run_id, first_seen_at, reason) VALUES ({})",
        placeholders(backend, 4)
    );
    db.execute_raw(Statement::from_sql_and_values(
        backend,
        sql,
        [table.into(), run_id.into(), now.into(), reason.into()],
    ))
    .await?;
    Ok(())
}

/// 移除一条待删表登记。
///
/// 两处调用，理由不同但结论相同 —— 那条登记**都已作废**：
/// 1. `apply` 里**真的执行完 `DROP`** 之后（表没了，登记自然失效）；
/// 2. [`put_orphan_whitelist`] 里 —— 表被豁免后**不再进 `plan`**，
///    于是「下一轮删它」这个决定同样作废（详细推理见该函数的注释）。
pub async fn clear_pending_drop(db: &DatabaseConnection, table: &str) -> Result<(), DbErr> {
    let backend = db.get_database_backend();
    let sql = format!("DELETE FROM {META_PENDING} WHERE table_name = {}", placeholders(backend, 1));
    db.execute_raw(Statement::from_sql_and_values(backend, sql, [table.into()])).await?;
    Ok(())
}

/// 登记一条人工孤儿豁免（闸 6 的**写入口**）。
///
/// ⚠ 为什么必须有它：只提供 [`load_orphan_whitelist`]（读）而不提供写入口时，调用方
/// 只能自己拼 `INSERT` —— 而自己拼就一定会有人漏掉 `ensure_meta_tables`，症状是
/// `no such table: _ax_schema_orphan_whitelist`（**从读那一侧完全看不出来**，
/// 因为读的地方自动带建表）。这正是本项目已知的「有读无写的死表」那一类缺陷。
///
/// 幂等：同表重复登记则覆盖 `reason` 与 `expires_at`（理由会过期，必须能改）。
pub async fn put_orphan_whitelist(
    db: &DatabaseConnection,
    table: &str,
    reason: &str,
    expires_at: i64,
) -> Result<(), DbErr> {
    ensure_meta_tables(db).await?;
    let backend = db.get_database_backend();
    // `ON CONFLICT (…) DO UPDATE` 在 PG 与 SQLite 上同形（PG 9.5+ / SQLite 3.24+），
    // 所以这里不需要按方言分支 —— 元表双方言兼容的收益在这里兑现。
    let sql = format!(
        "INSERT INTO {META_WHITELIST} (table_name, reason, expires_at) VALUES ({})
ON CONFLICT (table_name) DO UPDATE SET reason = excluded.reason, expires_at = excluded.expires_at",
        placeholders(backend, 3)
    );
    db.execute_raw(Statement::from_sql_and_values(
        backend,
        sql,
        [table.into(), reason.into(), expires_at.into()],
    ))
    .await?;

    // ⚠ 顺带清掉该表的「待删登记」—— 这是 2026-09-16 发现的**语义反转**缺陷。
    //
    // 推理：表进白名单 ⇒ `diff` 不再把它算孤儿 ⇒ 它**不会出现在任何一轮
    // `plan.changes` 里** ⇒ 「下一轮删它」这个决定已经作废，`_ax_schema_pending_drops`
    // 里剩下的那行是**死数据**。
    //
    // 留死数据的后果是反向的：白名单**到期或撤销**后该表重新进 plan，
    // `defer_decision` 读到那行陈旧记录 ⇒ 判 `Execute` ⇒ **当轮直接 DROP**，
    // 跳过了「首见只登记」的缓冲。也就是说「暂缓」变成了「加速」——与豁免的语义
    // 正好相反。真库上确实出现过这个叠加状态（2 张非空孤儿表同时在
    // pending 与 whitelist 里，见 PLAN §十一·九）。
    //
    // ⚠ 不对称是**刻意**的：[`remove_orphan_whitelist`] **不**恢复 pending 行。
    // 撤销豁免后该表重新进 plan，会以「首见」身份重新登记 ⇒ 天然再享一轮延迟；
    // 恢复旧行反而会把「刚撤销」变成「立刻删」，那是同一个缺陷的另一面。
    clear_pending_drop(db, table).await?;
    Ok(())
}

/// 撤销一条人工孤儿豁免。
///
/// ⚠ 撤销必须**真的删行**，不能靠「把 `expires_at` 改到过去」：过期项会在
/// [`load_orphan_whitelist`] 的 `expired` 列表里继续留痕（这是刻意的），于是撤销会
/// 变成一条永远存在的噪声。撤销与过期是两件事。
pub async fn remove_orphan_whitelist(db: &DatabaseConnection, table: &str) -> Result<(), DbErr> {
    ensure_meta_tables(db).await?;
    let backend = db.get_database_backend();
    let sql =
        format!("DELETE FROM {META_WHITELIST} WHERE table_name = {}", placeholders(backend, 1));
    db.execute_raw(Statement::from_sql_and_values(backend, sql, [table.into()])).await?;
    Ok(())
}

/// 白名单快照（闸 6）。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WhitelistSnapshot {
    /// 未过期项 → `PlanOptions::orphan_whitelist` 的形状（理由自带来源与到期日）。
    pub active: BTreeMap<String, String>,
    /// 已过期项的表名。**必须留痕**（调用方应写进 advisory），否则「我登记过它、
    /// 但它还是被删了」无从解释。
    pub expired: Vec<String>,
}

/// 读白名单，按 `now` 分成「有效」与「已过期」。
///
/// 过期判定在这里做（不在 SQL 里）：`expires_at` 是调用方给的时间基准，
/// 放进 SQL 会让「用哪个时间」散在两处（一处 `now()`、一处 Rust），
/// 而测试需要注入固定时间。
pub async fn load_orphan_whitelist(
    db: &DatabaseConnection,
    now: i64,
) -> Result<WhitelistSnapshot, DbErr> {
    let backend = db.get_database_backend();
    let rows = db
        .query_all_raw(Statement::from_string(
            backend,
            format!(
                "SELECT table_name, reason, expires_at FROM {META_WHITELIST} ORDER BY table_name"
            ),
        ))
        .await?;

    let mut snap = WhitelistSnapshot::default();
    for r in rows {
        let name: String = r.try_get("", "table_name")?;
        let reason: String = r.try_get("", "reason")?;
        let expires_at: i64 = r.try_get("", "expires_at")?;
        if expires_at > now {
            snap.active.insert(name, format!("whitelist: {reason}（至 epoch {expires_at}）"));
        } else {
            snap.expired.push(name);
        }
    }
    Ok(snap)
}

// ═══════════════════════════════════════════════════════════════════════════
// 测试
// ═══════════════════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reconcile::extras::Dialect;
    use crate::reconcile::model::SchemaModel;
    use crate::reconcile::plan::{ChangeKind, ChangePayload};
    use sea_orm::Database;

    async fn mem() -> DatabaseConnection {
        let db = Database::connect("sqlite::memory:").await.expect("连接内存库应成功");
        ensure_meta_tables(&db).await.expect("建元表应成功");
        db
    }

    fn cfg(dry_run: bool) -> SafetyConfig {
        SafetyConfig { dry_run, ..SafetyConfig::default() }
    }

    /// 造一条变更（`loses_data` 由 `kind` 决定，与 `plan.rs` 一致）。
    fn change(kind: ChangeKind, object: &str) -> Change {
        Change {
            kind,
            object: object.to_string(),
            destructive: kind.destructive(),
            loses_data: kind.loses_data(),
            detail: String::new(),
            payload: ChangePayload::Bare,
        }
    }

    fn plan_with(changes: Vec<Change>) -> Plan {
        let n_destructive = changes.iter().filter(|c| c.destructive).count();
        let n_safe = changes.len() - n_destructive;
        Plan { changes, n_destructive, n_safe, ..Plan::default() }
    }

    // ── 元表 ──

    /// ⚠ 元表清单必须**全部**落在 `introspect::ENGINE_META_PREFIX` 之下。
    ///
    /// 这是「元表对引擎不可见」这条约定的**另一头**：`introspect` 侧按前缀排除，
    /// 本侧按常量建表。若哪天加了一张不以 `_ax_schema_` 开头的簿记表，它会进实况
    /// 指纹 ⇒ 收敛判据永久失效，而症状只是「每轮都全量 diff」（静默）。
    #[test]
    fn meta_table_names_all_hit_the_prefix() {
        let prefix = crate::reconcile::introspect::ENGINE_META_PREFIX;
        for t in META_TABLES {
            assert!(
                t.starts_with(prefix),
                "元表 `{t}` 不在保留前缀 `{prefix}` 之下 —— 它会进实况指纹，让「apply 后\
                 指纹 == 期望指纹」永久为假"
            );
        }
        assert_eq!(META_TABLES.len(), META_DDL.len(), "清单与 DDL 必须一一对应");
        // DDL 里出现的表名与常量一致（防「改了常量忘了改 DDL」）
        for (name, ddl) in META_TABLES.iter().zip(META_DDL) {
            assert!(ddl.contains(name), "DDL 里找不到表名 `{name}`：{ddl}");
        }
    }

    /// 建表幂等：连建两次不报错（apply 每次启动都会执行）。
    #[tokio::test]
    async fn meta_ddl_is_idempotent() {
        let db = mem().await;
        ensure_meta_tables(&db).await.expect("第二次 ensure 也应成功");
        for t in META_TABLES {
            let rows = db
                .query_all_raw(Statement::from_string(
                    DbBackend::Sqlite,
                    format!("SELECT name FROM sqlite_master WHERE type='table' AND name='{t}'"),
                ))
                .await
                .expect("查表应成功");
            assert_eq!(rows.len(), 1, "元表 `{t}` 应存在");
        }
    }

    /// ⚠ 元表的整型列**必须是 `BIGINT`**，不能是 `INTEGER`。
    ///
    /// PG 的 `INTEGER` 是 int4，而 Rust 侧这些列一律 `i64`（sqlx 解码要求 int8）⇒
    /// 在 PG 上 `try_get::<i64>` 报「not compatible with SQL type INT4」，整条 apply
    /// 路径不可用（真库实测，根因与三个掩盖条件见模块文档）。
    ///
    /// **这条断言是 CI 里唯一能拦住它的地方**：SQLite 的 `INTEGER` 亲和性本来就是 64 位，
    /// 所以「在 SQLite 上跑一遍」永远是绿的 —— 不能指望 `sqlite::memory:` 的测试。
    #[test]
    fn meta_ddl_has_no_bare_integer_columns() {
        for ddl in META_DDL {
            assert!(
                !ddl.contains("INTEGER"),
                "元表 DDL 里不允许出现 `INTEGER`（PG 上是 int4，与 Rust 侧 i64 不匹配）—— \
                 应写 `BIGINT`：{ddl}"
            );
        }
    }

    /// [`PG_INT_WIDENING`] 必须覆盖元表的**每一个**整型列，且只针对元表。
    ///
    /// 硬断言条数（而不是「大于 0」）：将来给元表加一个整型列而忘了加 ALTER 时，
    /// 这条当场变红。理由不是形式主义 —— **只改 `META_DDL` 而不加 ALTER，修复就只对
    /// 从零新建的库生效**，而出问题的恰恰是「已经建过表」的那台机器。
    #[test]
    fn pg_int_widening_covers_every_integer_column() {
        assert_eq!(
            PG_INT_WIDENING.len(),
            10,
            "元表整型列清单变了？同步 `PG_INT_WIDENING`（当前清单：{PG_INT_WIDENING:#?}）"
        );
        for sql in PG_INT_WIDENING {
            assert!(sql.starts_with("ALTER TABLE "), "形态不对：{sql}");
            assert!(sql.ends_with(" TYPE bigint"), "目标类型必须是 bigint：{sql}");
            // ⚠ 只允许动元表 —— 这段是**自动执行**的 DDL，写错表名就是去改业务表。
            assert!(
                META_TABLES.iter().any(|t| sql.contains(*t)),
                "`{sql}` 指向的不是元表：{META_TABLES:?}"
            );
        }
    }

    /// `PG_INT_WIDENING` **只在 PG 上跑**。
    ///
    /// 把方言判断抽成 `needs_int_widening` 就是为了能直接断言它 —— 否则「SQLite 上不跑」
    /// 只能靠「跑一遍看会不会报语法错误」间接观察，而间接观察在这里不可靠
    /// （SQLite 接受一部分 `ALTER TABLE` 形态，可能给出假绿）。
    #[test]
    fn int_widening_is_postgres_only() {
        assert!(needs_int_widening(DbBackend::Postgres));
        assert!(!needs_int_widening(DbBackend::Sqlite));
    }

    /// 元表建出来之后，`introspect` 仍然看不见它们 —— 这是「收敛判据成立」的实证。
    ///
    /// 用**真实的 `META_DDL`**（不是硬编码字面量），所以它验证的是「这四张表」而不是
    /// 「四张名字相似的表」。
    #[tokio::test]
    async fn meta_tables_are_invisible_to_introspect_after_real_ddl() {
        let db = mem().await;
        db.execute_unprepared("CREATE TABLE real_table (id TEXT NOT NULL PRIMARY KEY)")
            .await
            .expect("建表应成功");
        let m = crate::reconcile::introspect::read(&db).await.expect("introspect 应成功");
        let names = m.table_names();
        assert_eq!(names, vec!["real_table"], "元表必须不可见，否则指纹永不收敛：{names:?}");
    }

    // ── 配置（闸 1） ──

    /// 逃生阀的方向：**默认开**，且**只有明确写关才关**。
    ///
    /// 「乱写也算开」是刻意的：误判代价不对称（拦住了只浪费一次运行，没拦住会删数据）。
    #[test]
    fn escape_hatch_defaults_on_and_fails_safe() {
        let none: Option<&str> = None;
        assert!(switch_default_on(none), "未设置 ⇒ 开");
        for off in ["0", "false", "FALSE", " off ", "No"] {
            assert!(!switch_default_on(Some(off)), "{off:?} 应判为关");
        }
        for on in ["1", "true", "yes", "on", "", "treu", "2", "  "] {
            assert!(switch_default_on(Some(on)), "{on:?} 应判为开（含写错的）");
        }
    }

    #[test]
    fn config_from_lookup_takes_the_safe_direction() {
        // 完全没有环境变量 ⇒ 全默认（最保守）
        let d = SafetyConfig::from_lookup(|_| None);
        assert_eq!(d, SafetyConfig::default());
        assert!(d.dry_run, "默认必须只渲染不执行");
        assert!(d.defer_destructive, "默认必须延迟");
        assert!(!d.allow_base_shrink, "默认不允许基数下降");
        assert_eq!(d.max_destructive_per_run, 5);

        // 显式设成「危险」才危险
        let risky = SafetyConfig::from_lookup(|k| match k {
            ENV_DRY_RUN => Some("0".into()),
            ENV_MAX_DESTRUCTIVE => Some("500".into()),
            ENV_DEFER_DESTRUCTIVE => Some("0".into()),
            ENV_ALLOW_BASE_SHRINK => Some("1".into()),
            _ => None,
        });
        assert!(!risky.dry_run);
        assert_eq!(risky.max_destructive_per_run, 500);
        assert!(!risky.defer_destructive);
        assert!(risky.allow_base_shrink);
    }

    /// 配额解析失败 ⇒ 退回默认（**不是**无上限）。
    #[test]
    fn limit_parsing_falls_back_to_default_not_infinity() {
        assert_eq!(parse_limit(Some("12"), 5), 12);
        assert_eq!(parse_limit(Some(" 12 "), 5), 12);
        for bad in ["", "lots", "-1", "1e9", "12.5", "  "] {
            assert_eq!(parse_limit(Some(bad), 5), 5, "{bad:?} 应退回默认而不是放行一切");
        }
    }

    // ── 熔断（闸 2 + 3） ──

    #[test]
    fn destructive_quota_blocks_before_execution() {
        let many: Vec<Change> =
            (0..6).map(|i| change(ChangeKind::DropTable, &format!("t{i}"))).collect();
        let p = plan_with(many);
        let r = check_circuit_breakers(&p, 100, None, &cfg(false)).expect("6 > 5 应熔断");
        assert!(matches!(r, Refusal::TooManyDestructive { n: 6, limit: 5 }), "{r:?}");
        assert!(r.reason().contains(ENV_MAX_DESTRUCTIVE), "理由要指出怎么放宽：{}", r.reason());

        // 恰好等于配额 ⇒ 放行（配额是闭区间上限）
        let five: Vec<Change> =
            (0..5).map(|i| change(ChangeKind::DropTable, &format!("t{i}"))).collect();
        assert!(check_circuit_breakers(&plan_with(five), 100, None, &cfg(false)).is_none());
    }

    /// 基数熔断：期望表数下降必须显式放行 —— 这是「声明被误删 ⇒ 全库 DROP」的专用闸。
    #[test]
    fn base_shrink_requires_explicit_opt_in() {
        let p = plan_with(vec![change(ChangeKind::CreateTable, "t")]);
        // 无历史记录 ⇒ 不比（首轮没有基准）
        assert!(check_circuit_breakers(&p, 30, None, &cfg(false)).is_none());
        // 持平或增长 ⇒ 放行
        assert!(check_circuit_breakers(&p, 200, Some(200), &cfg(false)).is_none());
        assert!(check_circuit_breakers(&p, 201, Some(200), &cfg(false)).is_none());
        // 下降 ⇒ 拒
        let r = check_circuit_breakers(&p, 30, Some(200), &cfg(false)).expect("下降应熔断");
        assert!(matches!(r, Refusal::BaseShrank { now: 30, last: 200 }), "{r:?}");
        assert!(r.reason().contains(ENV_ALLOW_BASE_SHRINK));
        // 显式放行 ⇒ 过
        let allow = SafetyConfig { allow_base_shrink: true, ..cfg(false) };
        assert!(check_circuit_breakers(&p, 30, Some(200), &allow).is_none());
    }

    /// 配额闸先于基数闸（返回第一条命中，顺序稳定）。
    #[test]
    fn refusal_order_is_stable() {
        let many: Vec<Change> =
            (0..9).map(|i| change(ChangeKind::DropTable, &format!("t{i}"))).collect();
        let r = check_circuit_breakers(&plan_with(many), 10, Some(999), &cfg(false));
        assert!(matches!(r, Some(Refusal::TooManyDestructive { .. })), "{r:?}");
    }

    // ── 墓碑证据（闸 4） ──

    /// `loses_data` 的每一条都必须在证据表里有**完整**证据；空 sha256 不算。
    #[test]
    fn every_data_losing_change_demands_complete_evidence() {
        let p = plan_with(vec![
            change(ChangeKind::DropTable, "orphan_a"),
            change(ChangeKind::DropColumn, "t.col"),
            change(ChangeKind::AlterColumnType, "t.c2"),
            change(ChangeKind::CreateTable, "new_t"), // 不丢数据 ⇒ 不要证据
            change(ChangeKind::DropIndex, "idx_x"),   // 不丢数据 ⇒ 不要证据
        ]);

        let none: BTreeMap<String, GraveEvidence> = BTreeMap::new();
        let missing = missing_evidence(&p, &none);
        assert_eq!(
            missing.iter().map(|c| c.object.as_str()).collect::<Vec<_>>(),
            vec!["orphan_a", "t.col", "t.c2"],
            "只有 loses_data 的三条要证据"
        );

        // 空 sha256 / 空 path 都不算完整
        let mut partial: BTreeMap<String, GraveEvidence> = BTreeMap::new();
        partial.insert(
            "orphan_a".into(),
            GraveEvidence {
                export_path: "output/a.json".into(),
                export_sha256: String::new(), // ⚠ 半截证据
                row_count: Some(10),
            },
        );
        partial.insert(
            "t.col".into(),
            GraveEvidence {
                export_path: "  ".into(),
                export_sha256: "abc123".into(),
                row_count: None,
            },
        );
        let missing = missing_evidence(&p, &partial);
        assert_eq!(
            missing.iter().map(|c| c.object.as_str()).collect::<Vec<_>>(),
            vec!["orphan_a", "t.col", "t.c2"],
            "路径或指纹为空都视为缺证据"
        );

        // 补齐后清零
        for obj in ["orphan_a", "t.col", "t.c2"] {
            partial.insert(
                obj.into(),
                GraveEvidence {
                    export_path: format!("output/{obj}.json"),
                    export_sha256: "deadbeef".into(),
                    row_count: None,
                },
            );
        }
        assert!(missing_evidence(&p, &partial).is_empty());
    }

    /// 墓碑写入**自己**也拒绝不完整证据（不只靠调用方判）。
    #[tokio::test]
    async fn graveyard_refuses_incomplete_evidence() {
        let db = mem().await;
        let bad = GraveEvidence {
            export_path: "output/x.json".into(),
            export_sha256: String::new(),
            row_count: None,
        };
        let e = write_graveyard(&db, "run-1", 0, "t", "DROP TABLE \"t\"", &bad, 1)
            .await
            .expect_err("空 sha256 应被拒");
        assert!(e.to_string().contains("证据不完整"), "{e}");

        let good = GraveEvidence {
            export_path: "output/x.json".into(),
            export_sha256: "abc".into(),
            row_count: Some(7),
        };
        write_graveyard(&db, "run-1", 0, "t", "DROP TABLE \"t\"", &good, 1)
            .await
            .expect("完整证据应可写");

        let rows = db
            .query_all_raw(Statement::from_string(
                DbBackend::Sqlite,
                format!("SELECT table_name, row_count, export_path FROM {META_GRAVEYARD}"),
            ))
            .await
            .expect("读墓碑应成功");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].try_get::<String>("", "table_name").unwrap(), "t");
        assert_eq!(rows[0].try_get::<i64>("", "row_count").unwrap(), 7);
    }

    /// ⚠ `row_count = None` 必须与 `0` 可区分（「没统计」≠「零行」）。
    #[tokio::test]
    async fn null_row_count_is_not_zero() {
        let db = mem().await;
        let ev = GraveEvidence {
            export_path: "output/e.json".into(),
            export_sha256: "abc".into(),
            row_count: None,
        };
        write_graveyard(&db, "run-1", 0, "t", "DROP TABLE \"t\"", &ev, 1).await.unwrap();
        let rows = db
            .query_all_raw(Statement::from_string(
                DbBackend::Sqlite,
                format!("SELECT row_count FROM {META_GRAVEYARD}"),
            ))
            .await
            .unwrap();
        let got: Option<i64> = rows[0].try_get("", "row_count").unwrap();
        assert_eq!(got, None, "未统计必须读回 NULL，而不是被写成 0");
    }

    // ── 延迟执行（闸 5） ──

    /// 首次见到 ⇒ 只登记；再次见到 ⇒ 执行。配置关掉 ⇒ 恒执行。
    #[test]
    fn destructive_changes_are_deferred_by_one_cycle() {
        let pending: BTreeSet<String> = BTreeSet::new();
        assert_eq!(
            defer_decision("orphan", &pending, &cfg(false)),
            DeferDecision::DeferFirstSight,
            "首见必须只登记"
        );

        let pending = BTreeSet::from(["orphan".to_string()]);
        assert_eq!(defer_decision("orphan", &pending, &cfg(false)), DeferDecision::Execute);
        assert_eq!(
            defer_decision("other", &pending, &cfg(false)),
            DeferDecision::DeferFirstSight,
            "别的表不受影响"
        );

        let no_defer = SafetyConfig { defer_destructive: false, ..cfg(false) };
        assert_eq!(
            defer_decision("fresh", &BTreeSet::new(), &no_defer),
            DeferDecision::Execute,
            "关掉延迟后首见即执行"
        );
    }

    /// 待删表的登记/读取/清除（真读写 SQLite）。已登记的不刷新首见时间。
    #[tokio::test]
    async fn pending_drops_round_trip() {
        let db = mem().await;
        assert!(load_pending_drops(&db).await.unwrap().is_empty());

        record_pending_drop(&db, "a", "run-1", "孤儿", 100).await.unwrap();
        record_pending_drop(&db, "b", "run-1", "孤儿", 100).await.unwrap();
        // 重复登记（不同 run / 不同时间）不得覆盖首见时间
        record_pending_drop(&db, "a", "run-2", "孤儿", 999).await.unwrap();

        let got = load_pending_drops(&db).await.unwrap();
        assert_eq!(got, BTreeSet::from(["a".to_string(), "b".to_string()]));

        let rows = db
            .query_all_raw(Statement::from_string(
                DbBackend::Sqlite,
                "SELECT first_seen_at, first_run_id FROM _ax_schema_pending_drops WHERE table_name='a'"
                    .to_string(),
            ))
            .await
            .unwrap();
        assert_eq!(rows[0].try_get::<i64>("", "first_seen_at").unwrap(), 100, "首见时间不得被刷新");
        assert_eq!(rows[0].try_get::<String>("", "first_run_id").unwrap(), "run-1");

        clear_pending_drop(&db, "a").await.unwrap();
        assert_eq!(load_pending_drops(&db).await.unwrap(), BTreeSet::from(["b".to_string()]));
    }

    // ── 白名单（闸 6） ──

    /// 白名单按 `now` 分「有效」与「过期」，且过期项**必须留痕**。
    #[tokio::test]
    async fn whitelist_splits_active_and_expired() {
        let db = mem().await;
        for (name, reason, exp) in
            [("alive", "迁移中", 2000i64), ("dead", "早已过期", 500i64), ("edge", "临界", 1000i64)]
        {
            let sql = format!(
                "INSERT INTO {META_WHITELIST} (table_name, reason, expires_at) VALUES ({})",
                placeholders(DbBackend::Sqlite, 3)
            );
            db.execute_raw(Statement::from_sql_and_values(
                DbBackend::Sqlite,
                sql,
                [name.into(), reason.into(), exp.into()],
            ))
            .await
            .unwrap();
        }

        let snap = load_orphan_whitelist(&db, 1000).await.unwrap();
        assert!(snap.active.contains_key("alive"), "{snap:?}");
        // ⚠ `edge` 的 expires_at == now ⇒ **过期**（判据是 `expires_at > now`）。
        // 这条边界必须显式：写成 `>=` 会让「登记到某日」在那一天之后仍然生效一天。
        assert_eq!(
            snap.expired,
            vec!["dead".to_string(), "edge".to_string()],
            "恰好到期算过期，且按表名排序：{snap:?}"
        );
        assert!(!snap.active.contains_key("edge"), "{snap:?}");

        // 理由必须自带来源与到期时间（否则「为什么没删」无从追）
        let reason = snap.active.get("alive").expect("alive 应有效");
        assert!(reason.contains("whitelist:"), "{reason}");
        assert!(reason.contains("2000"), "{reason}");
    }

    /// ⚠ 豁免一张表**必须**同时清掉它的待删登记 —— 否则「暂缓」会变成「加速」。
    ///
    /// 真库实测场景（2026-09-16）：2 张非空孤儿表**同时**在
    /// `_ax_schema_pending_drops`（上一轮 apply 登记的「下轮删」）与
    /// `_ax_schema_orphan_whitelist`（本轮为保护数据登记）里。白名单生效期内没事，
    /// 但白名单**到期/撤销**后该表重新进 plan ⇒ `defer_decision` 读到那行陈旧登记
    /// ⇒ `Execute` ⇒ 当轮直接 `DROP`，**跳过了首见延迟**。豁免的语义是「暂缓」，
    /// 实现在这里却把它变成了「加速」。
    ///
    /// 反向也断言：撤销豁免**不**恢复登记（那会把「刚撤销」变成「立刻删」，
    /// 是同一个缺陷的另一面）。
    #[tokio::test]
    async fn whitelisting_a_table_clears_its_pending_drop() {
        let db = mem().await;

        // 先造出「已被登记为待删」的状态（模拟上一轮 apply 的产物），再豁免它。
        record_pending_drop(&db, "t1", "run-1", "孤儿表：闸 5 首次见到", 100).await.unwrap();
        assert_eq!(load_pending_drops(&db).await.unwrap().len(), 1, "前置条件：应已登记");

        put_orphan_whitelist(&db, "t1", "有数据，暂缓", 5_000).await.unwrap();
        assert!(
            load_pending_drops(&db).await.unwrap().is_empty(),
            "豁免后待删登记必须消失，否则豁免一到期就会跳过首见延迟直接删"
        );

        remove_orphan_whitelist(&db, "t1").await.unwrap();
        assert!(
            load_pending_drops(&db).await.unwrap().is_empty(),
            "撤销豁免不该恢复旧登记（那会把「刚撤销」变成「立刻删」）"
        );
    }

    /// 白名单的**写入口**：幂等覆盖 + 撤销真的删行。
    ///
    /// 这里刻意用一张**没有元表**的新库：写入口必须自带建表，否则调用方自己拼
    /// `INSERT` 时会漏掉那一步，症状是 `no such table` —— 而读那一侧看不出来。
    #[tokio::test]
    async fn whitelist_writer_upserts_and_remover_clears() {
        let db = Database::connect("sqlite::memory:").await.expect("连内存库应成功");

        put_orphan_whitelist(&db, "t1", "迁移中", 5_000).await.expect("写入口必须自带建表");
        let snap = load_orphan_whitelist(&db, 1_000).await.unwrap();
        assert_eq!(snap.active.len(), 1);
        assert!(snap.active["t1"].contains("迁移中"), "{:?}", snap.active);

        // 重复登记 ⇒ 覆盖。理由会过期，必须能改；不能插出第二行（主键在守，但也要显式验）
        put_orphan_whitelist(&db, "t1", "改期", 9_000).await.unwrap();
        let snap = load_orphan_whitelist(&db, 1_000).await.unwrap();
        assert_eq!(snap.active.len(), 1, "重复登记不得插出第二行：{:?}", snap.active);
        assert!(snap.active["t1"].contains("改期"), "{:?}", snap.active);
        assert!(snap.active["t1"].contains("9000"), "到期时间也要被覆盖：{:?}", snap.active);

        // 撤销 ⇒ 真删行。留一条「已过期」的痕迹会把撤销变成永久噪声。
        remove_orphan_whitelist(&db, "t1").await.unwrap();
        let snap = load_orphan_whitelist(&db, 1_000).await.unwrap();
        assert!(snap.active.is_empty() && snap.expired.is_empty(), "{snap:?}");
    }

    /// ⚠ `(run_id, seq)` 是审计表**主键** ⇒ 同秒内重复运行必须自动换号。
    ///
    /// 这条守的是「同秒重跑」这个最常见调试路径：把 `run_id` 交给调用方保证唯一，
    /// 默认参数（序号 0）就必撞 —— 实测症状是
    /// `UNIQUE constraint failed: _ax_schema_audit.run_id, _ax_schema_audit.seq`。
    #[tokio::test]
    async fn run_id_allocation_avoids_audit_primary_key_collisions() {
        let db = mem().await;
        let row = AuditRow {
            seq: 0,
            kind: "CREATE TABLE",
            object: "t".into(),
            destructive: false,
            loses_data: false,
            executed: true,
            statements: vec!["SELECT 1".into()],
            notes: vec![],
            error: None,
        };

        // 空表 ⇒ 直接用起始序号
        let a = allocate_run_id(&db, 7_000, 0).await.unwrap();
        assert_eq!(a, make_run_id(7_000, 0));

        // 占用它之后再分配 ⇒ 必须换号
        // 借用而非 `clone`：`row` 直到下面 `&[row]`（move）才再被用到，两者不冲突
        // （clippy `cloned_ref_to_slice_refs`）。少一次 clone，语义不变。
        write_audit(&db, &a, std::slice::from_ref(&row), 7_000).await.unwrap();
        let b = allocate_run_id(&db, 7_000, 0).await.unwrap();
        assert_ne!(a, b, "同一秒必须能拿到不同的 run_id");
        write_audit(&db, &b, &[row], 7_000).await.expect("换号后写审计不应撞主键");

        // 换个 epoch ⇒ 回到起始序号（不是全局单调）
        assert_eq!(allocate_run_id(&db, 7_001, 0).await.unwrap(), make_run_id(7_001, 0));
    }

    // ── 审计（闸 7） ──

    /// 审计必须把「执行了」与「没执行」都记下来 —— 只记成功行无法区分
    /// 「没这条变更」与「有但被拦了」。
    #[tokio::test]
    async fn audit_records_both_executed_and_blocked_rows() {
        let db = mem().await;
        let rows = vec![
            AuditRow {
                seq: 0,
                kind: "CREATE TABLE",
                object: "new_t".into(),
                destructive: false,
                loses_data: false,
                executed: true,
                statements: vec!["CREATE TABLE \"new_t\" (\"id\" bigint NOT NULL)".into()],
                notes: vec![],
                error: None,
            },
            AuditRow {
                seq: 1,
                kind: "DROP TABLE",
                object: "orphan".into(),
                destructive: true,
                loses_data: true,
                executed: false, // 被 dry-run 拦下
                statements: vec!["DROP TABLE \"orphan\"".into()],
                notes: vec!["dry-run：未执行".into()],
                error: None,
            },
        ];
        write_audit(&db, "run-1", &rows, 1234).await.expect("写审计应成功");

        let got = db
            .query_all_raw(Statement::from_string(
                DbBackend::Sqlite,
                format!(
                    "SELECT seq, object, executed, statements, notes FROM {META_AUDIT} ORDER BY seq"
                ),
            ))
            .await
            .unwrap();
        assert_eq!(got.len(), 2);
        assert_eq!(got[0].try_get::<i64>("", "executed").unwrap(), 1);
        assert_eq!(got[1].try_get::<i64>("", "executed").unwrap(), 0);

        // `statements` / `notes` 是 JSON 文本，必须能解析回来（否则审计不可读）
        let stmts: Vec<String> =
            serde_json::from_str(&got[0].try_get::<String>("", "statements").unwrap()).unwrap();
        assert_eq!(stmts, rows[0].statements);
        let notes: Vec<String> =
            serde_json::from_str(&got[1].try_get::<String>("", "notes").unwrap()).unwrap();
        assert_eq!(notes, rows[1].notes);

        // 空批次不写任何行（也不报错）
        write_audit(&db, "run-2", &[], 1234).await.unwrap();
        let n = db
            .query_all_raw(Statement::from_string(
                DbBackend::Sqlite,
                format!("SELECT count(*) AS n FROM {META_AUDIT}"),
            ))
            .await
            .unwrap();
        assert_eq!(n[0].try_get::<i64>("", "n").unwrap(), 2);
    }

    /// `run_id` 的形状：秒 + 序号，同秒内靠序号区分。
    #[test]
    fn run_id_is_readable_and_ordered() {
        assert_eq!(make_run_id(1_700_000_000, 1), "run-1700000000-001");
        assert_ne!(make_run_id(1_700_000_000, 1), make_run_id(1_700_000_000, 2));
        assert!(make_run_id(1_700_000_001, 1) > make_run_id(1_700_000_000, 999));
    }

    /// 时间源必须是 epoch 秒（形如 1.7e9），不是毫秒也不会是 0。
    #[test]
    fn now_epoch_is_seconds() {
        let t = now_epoch();
        // 2020-01-01 ~ 2100-01-01 的 epoch 秒区间
        assert!((1_577_836_800..4_102_444_800).contains(&t), "now_epoch 看起来不是秒：{t}");
    }

    /// 模型侧的一个小验证：`Plan` 的空值可直接用于构造（测试夹具依赖它）。
    #[test]
    fn empty_plan_is_a_valid_baseline() {
        let p = Plan::default();
        assert!(p.is_empty());
        assert_eq!(p.n_destructive, 0);
        assert!(check_circuit_breakers(&p, 0, Some(1), &cfg(false)).is_some(), "下降仍要拒");
        let _ = SchemaModel::new(Dialect::Postgres); // 保持 imports 有效
    }
}
