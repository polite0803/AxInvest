// SPDX-License-Identifier: AGPL-3.0-only

//! schema diff 层：以 SeaORM 实体定义为期望 schema，对照实际库**补缺失列 +
//! 修类型错配**。
//!
//! ## 背景
//!
//! `repair_schema()` 通过无条件重跑迁移实现修复，依赖每条迁移自身幂等。
//! 但 `CREATE TABLE IF NOT EXISTS` 类幂等只保证「表在」，不保证「列全」：
//! 当表由另一条窄 schema 迁移先建、加列迁移又因版本号低于库版本被
//! `run_migrations` 永久跳过时（v136 / opc_invoices.lead_id 即此案例），
//! 重跑迁移永远无法补列——v109 / v124 / v208 / v223 四条自愈迁移全是
//! 同一缺陷的手工补丁。
//!
//! ⚠ 2026-09-16：上面这段是**迁移时代**的成因，迁移清单已清空，
//! `repair_schema()` 也不再重跑迁移（它现在只剩调用本模块这一件事）。保留这段
//! 而不是删掉，是因为它给出的**结论**仍然承重且换了主体：「幂等只保证对象存在，
//! 不保证列全」—— 引擎接管建表后同样如此（它也只做纯新增），所以本层仍是
//! 「有表缺列 / 类型错配」的唯一修复通道。
//!
//! 本层消除这一类：实体（`crates/entities`）是列的权威声明，逐实体
//! 对照实际库列集，缺失即 `ALTER TABLE ADD COLUMN`（类型映射 + 保守
//! 空缺策略）。
//!
//! ## 类型错配自愈（v225 反模式的重做）
//!
//! 历史迁移以 SQLite 方言书写（`REAL`），SQLite 下 REAL 是 8 字节浮点，
//! 与实体 `f64` 兼容；同一 DDL 在 PG 上 `REAL` = `float4`（单精度），
//! sea-orm 解码 `f64` 直接报 mismatched types（2026-09-09 实证：
//! opc_demand_leads.confidence 卡死需求发现全链路）。修这类错配的
//! 正确位置就是本层——实体驱动、零清单维护，而不是再写一条手写
//! 清单迁移。规则保守，只做**无损加宽**：
//! ① 实体期望 DOUBLE PRECISION 而实际为 real → 抬升双精度
//!   （confidence 实证）；② 整数宽度向上加宽（integer→BIGINT，
//!   evaluated_at 实证：SQLite 方言 INTEGER 在 PG = int4，实体 i64
//!   解码 mismatched types）。其余错配类不自动改。
//!
//! ## ⚠ 2026-09-16：类型自愈在 SQLite 上曾经**整段静默失效**（已修）
//!
//! 上面两类错配都是「SQLite 方言的 DDL 跑到 PG 上」才成立的，本层却在**两个**
//! 方言上无差别地跑。之后引擎接管建表，同一份实体声明出现了两个渲染器
//! （引擎用 sea-query：`BigInteger` → `integer`；本层用 `sql_type_of`：
//! `BigInteger` → `BIGINT`），于是 SQLite 上**引擎刚建好的库**被本层判出
//! 「期望 BIGINT / 实际 integer ⇒ 需加宽」，接着发出 SQLite 不支持的
//! `ALTER TABLE … ALTER COLUMN … TYPE` ⇒ 该表列循环在第一列就中断。
//! 实测 183 张实体表里 152 张中招（含 `conversations` / `gateway_keys` /
//! `background_tasks`），`tables_scanned=31`；又因为 `heal_entity` 的失败只被
//! `warn!` 吞掉，`columns_added=[]` 被上上下下读成了「其余表没有缺列」。
//!
//! 三处修正，判据分别落在**根因 / 能力 / 可观测性**上：
//! 1. **[`sql_type_of`] 的 SQLite 整数族逐字对齐 sea-query**（根因）：那份名字表
//!    本身抄错了（`BigInteger` 抄成 `BIGINT`），假阳性是它造成的。
//! 2. [`heal_entity`] 加**方言闸**（能力）：类型自愈只在 PG 上跑 —— SQLite 没有
//!    `ALTER COLUMN` 这条 DDL，且动态类型下本来也不需要加宽。闸与名字表是**两件**
//!    事：名字表错了会误判，误判之后发不发 DDL 由闸决定。
//! 3. [`DiffReport::errors`]（可观测性）：逐实体失败**进返回值**而不是只进日志 ——
//!    「这张表没查完」必须与「这张表没有缺列」可区分，否则修复结果是一份看起来完整、
//!    实则缺页的账。这一条才是让上面两条将来再出错时**能被发现**的那一条。
//!
//! **只补缺失列/修类型，不删已有列，缺表跳过**（建表由声明式引擎
//! `reconcile::apply::bootstrap_schema` 负责），天然幂等。
//!
//! ## 空缺策略（修复语义下的保守选择）
//!
//! - 整数 / 浮点 / 布尔 且实体声明 NOT NULL → `NOT NULL DEFAULT 0`（PG
//!   布尔用 FALSE）
//! - 其余类型一律可空补列：SELECT 不受影响，INSERT 由应用填值；避免
//!   UUID / JSON 等类型给不出合法 DEFAULT 而失败

use std::collections::HashMap;

use sea_orm::{
    ColumnTrait, ConnectionTrait, DbBackend, DbErr, EntityTrait, IdenStatic, Iterable, Statement,
};

// 实体模块注册表由 build.rs 编译期从 crates/entities/src/lib.rs 的 pub mod
// 声明自动生成——新增实体零登记成本。Rust 无运行时反射，无法在运行期
// 枚举类型；但 lib.rs 本身就是磁盘上的声明式清单，build 期读它即可。
include!(concat!(env!("OUT_DIR"), "/entity_registry.rs"));

/// 全库实际列快照：table_name → { column_name → 实际类型 }。
/// SQLite 下类型来自 pragma_table_info.type（视图列可能为空串）。
type ActualColumns = HashMap<String, HashMap<String, String>>;

/// 一次 diff 修复的统计。
#[derive(Debug, Default)]
pub struct DiffReport {
    /// 扫描的实体表数（缺表不计入，**对照失败的也不计入** —— 见 `errors`）
    pub tables_scanned: usize,
    /// 补建的列，格式 `table.column`
    pub columns_added: Vec<String>,
    /// 修复类型的列，格式 `table.column`
    pub types_healed: Vec<String>,
    /// 逐实体对照失败的原因，格式 `table: 原始错误`。正常为空。
    ///
    /// ⚠ 这个字段是**判据的一部分**，不是日志副本。`heal_entity` 的列循环带 `?`，
    /// 任一步失败即中断该表**后续全部列**的对照 ⇒ 它的 `columns_added` 只是
    /// 「检查到出错点为止」的部分账。没有本字段时，「这张表没查完」与「这张表
    /// 真的没有缺列」在返回值上**完全不可区分**（fail-open 静默降级），调用方
    /// 会把不完整的账读成健康结论。
    pub errors: Vec<String>,
}

/// 入口：对照全部已注册实体补缺失列。
///
/// 逐实体容错——单个实体的 diff 失败（如本方言执行不了的 DDL）**不中断其余实体**，
/// 但也不会消失：原因逐条进 [`DiffReport::errors`]。只告警不记录就等于把「这张表
/// 没查完」报告成「这张表没问题」。表必须先存在（建表由引擎的 `bootstrap_schema`
/// 负责，本层不凭空造表）—— 它由 `repair_schema()` 在库已建好之后调用。
///
/// 只有**整体性**失败才返回 `Err`（读不到实际列快照，此时一件事都没做）。
pub async fn heal_all(db: &sea_orm::DatabaseConnection) -> Result<DiffReport, DbErr> {
    let is_pg = db.get_database_backend() == DbBackend::Postgres;
    let actual = load_actual_columns(db, is_pg).await?;
    let mut report = DiffReport::default();

    // 递归宏展开：entity_modules!(cb) → cb!(mod1, mod2, ...) → 逐实体 diff
    macro_rules! heal_entities {
        ($($module:ident),* $(,)?) => {
            $(
                // 报告里用**表名**而不是模块名：两者并不总是同名（`trajectories` 的表是
                // `trajectory_trajectories`，`analyst_feedback` 的是 `analyst_feedbacks`），
                // 而收报告的人要照着它去库里找表。`EntityName` 是 `EntityTrait` 的
                // supertrait，具体类型调用点必须显式导入（同 `expected.rs:43-51` 实测）。
                let table = {
                    use sea_orm::EntityName;
                    axagent_entities::$module::Entity::default().table_name().to_string()
                };
                match heal_entity::<axagent_entities::$module::Entity>(db, &actual).await {
                    Ok((scanned, mut added, mut healed)) => {
                        if scanned {
                            report.tables_scanned += 1;
                        }
                        report.columns_added.append(&mut added);
                        report.types_healed.append(&mut healed);
                    },
                    // 单实体失败只告警、**不中断其余实体**：修复层不阻塞（迁移时代这里
                    // 写的是「不阻塞…与迁移版本记录」，版本记录那一步已随 `repair_schema`
                    // 的重写删除）。
                    //
                    // ⚠ 但必须**同时进返回值**。只 `warn!` 是 fail-open：`heal_entity`
                    // 的列循环带 `?`，出错即中断该表后续全部列的对照 ⇒ `columns_added`
                    // 少掉它的那部分，而「没查完」与「没有缺列」在返回值上不可区分。
                    Err(e) => {
                        tracing::warn!(
                            "[schema_diff] 实体 {}（表 {table}）diff 失败: {}",
                            stringify!($module),
                            e
                        );
                        report.errors.push(format!("{table}: {e}"));
                    },
                }
            )*
        };
    }

    entity_modules!(heal_entities);

    // 失败实体单独报一条，且**先于**下面那句「无缺失列」——否则一张表对照到一半
    // 崩掉、恰好什么都没补时，日志会同时出现「N 张表对照完成，无缺失列」和一条
    // warn，读者极易只看见前者。
    if !report.errors.is_empty() {
        tracing::warn!(
            "[schema_diff] {} 个实体**未能对照完成**（其缺失列/类型错配无从判断，原因见返回值 errors）: {:?}",
            report.errors.len(),
            report.errors,
        );
    }

    if report.columns_added.is_empty() && report.types_healed.is_empty() {
        // ⚠ 措辞刻意限定在「已查部分」：`errors` 非空时，未对照完的那些表**有没有
        // 缺列是不知道的**，说成「无缺失列」就是把一次中断读成了一个结论。
        tracing::info!(
            "[schema_diff] {} 张实体表对照完成，已查部分无缺失列/类型错配",
            report.tables_scanned
        );
    } else {
        if !report.columns_added.is_empty() {
            tracing::warn!(
                "[schema_diff] {} 张实体表对照完成，补列 {} 个: {:?}",
                report.tables_scanned,
                report.columns_added.len(),
                report.columns_added,
            );
        }
        if !report.types_healed.is_empty() {
            tracing::warn!(
                "[schema_diff] 类型错配修复 {} 个: {:?}",
                report.types_healed.len(),
                report.types_healed,
            );
        }
    }
    Ok(report)
}

/// 对照单个实体的期望列集，为已存在的表补缺失列 + 修类型错配。
///
/// 返回 `(表是否存在, 补建的 "table.column" 列表, 类型修复的 "table.column" 列表)`；
/// 表不存在时跳过（建表由引擎负责，diff 不凭空造表）。
async fn heal_entity<E>(
    db: &sea_orm::DatabaseConnection,
    actual: &ActualColumns,
) -> Result<(bool, Vec<String>, Vec<String>), DbErr>
where
    E: EntityTrait,
{
    let table = E::default().table_name().to_string();
    let Some(actual_cols) = actual.get(&table) else {
        return Ok((false, Vec::new(), Vec::new()));
    };

    let backend = db.get_database_backend();

    // 类型自愈是 **PostgreSQL 专有能力**，判据是「本方言有没有这个 DDL」：
    // 它靠 `ALTER TABLE … ALTER COLUMN … TYPE …`，而 SQLite **没有 `ALTER COLUMN`**
    // ——语句在第二个 `ALTER` 处就解析失败。且 SQLite 也不需要它：动态类型下
    // i64 / f64 / text 都能原样存进 integer / real / text 列，列上声明的类型名
    // 不影响 sea-orm 解码。
    //
    // ⚠ 这条路曾经**静默失效**过（2026-09-16 定因）：`sql_type_of` 把实体 `i64`
    // 写成 `BIGINT`，而 sea-query 的 SQLite 后端把同一个 `BigInteger` 渲染成
    // `integer` ⇒ 引擎刚建好的库上「期望 BIGINT / 实际 integer」被判为需加宽 ⇒
    // 发出 SQLite 不认的 DDL ⇒ 该表列循环在**第一列**就 `?` 中断。实测 183 张
    // 实体表里 152 张中招（含 conversations / gateway_keys / background_tasks），
    // 而错误当时被 `heal_all` 的 `warn!` 吞掉 ⇒ `tables_scanned=31`、
    // `columns_added=[]` 被读成「其余表没有缺列」。
    //
    // 闸放在**这里**（DDL 发出点）而不是 `heal_target_type` 内部：判据是「本方言
    // 支不支持这条 DDL」这一**能力**问题，与「哪些类型组合算错配」那套规则无关。
    // 将来往 `heal_target_type` 加规则时，这个闸不会漏。
    let is_pg = backend == DbBackend::Postgres;

    let mut added = Vec::new();
    let mut healed = Vec::new();
    for col in E::Column::iter() {
        let name = col.as_str();
        let def = col.def();
        let sql_type = sql_type_of(def.get_column_type(), backend);
        match actual_cols.get(name) {
            Some(actual_type) => {
                if !is_pg {
                    // SQLite：无 ALTER COLUMN，且动态类型下无需加宽（见上方闸的说明）
                    continue;
                }
                // 列已存在：检查类型错配（real→DOUBLE PRECISION / 整数加宽）
                if let Some(target) = heal_target_type(sql_type, actual_type) {
                    let ddl = format!("ALTER TABLE {table} ALTER COLUMN {name} TYPE {target}");
                    db.execute_unprepared(&ddl).await?;
                    tracing::warn!(
                        "[schema_diff] {table}.{name} 类型错配（实际 {actual_type}，实体期望 {sql_type}），已抬升为 {target}"
                    );
                    healed.push(format!("{table}.{name}"));
                }
            },
            None => {
                // 实体声明 NOT NULL 且类型能给出合法 DEFAULT 才带约束，
                // 否则保守可空（修复语义：宁可宽不可错）
                let not_null_suffix = if !def.is_null() {
                    not_null_default_suffix(def.get_column_type(), backend)
                } else {
                    None
                };
                let ddl = match &not_null_suffix {
                    Some(suffix) => {
                        format!("ALTER TABLE {table} ADD COLUMN {name} {sql_type}{suffix}")
                    },
                    None => format!("ALTER TABLE {table} ADD COLUMN {name} {sql_type}"),
                };
                db.execute_unprepared(&ddl).await?;
                tracing::warn!("[schema_diff] {table}.{name} 缺失，已补列: {ddl}");
                added.push(format!("{table}.{name}"));
            },
        }
    }
    Ok((true, added, healed))
}

/// 类型错配判定：返回需要自愈的目标类型（None = 不动）。
///
/// 已证实的错配类（均为**无损加宽**，安全）：
/// 1. SQLite 方言迁移在 PG 上产出 `real`(float4 单精度)，而实体是 f64
///    （`DOUBLE PRECISION` 双精度）—— opc_demand_leads.confidence 实证；
/// 2. SQLite 方言迁移 `INTEGER` 在 PG = `integer`(int4)，而实体是 i64
///    （`BIGINT`）—— opc_demand_leads.evaluated_at 实证（sea-orm 解码
///    Option<i64> 直接 mismatched types，整条入库链炸）。
///
/// ## ⚠ 2026-09-16 更正一处**错误的前提**
///
/// 本函数原来写着「SQLite 下类型名与实体期望一致，不会进入本函数」—— 这句话是
/// 错的，而它错的方式正是这类缺陷最难查的地方：`sql_type_of` 把实体 `i64` 映射成
/// `BIGINT`，sea-query 的 SQLite 后端把同一个 `BigInteger` 渲染成 `integer`，
/// 于是「同一份实体声明」在两个渲染器下给出两个名字。**引擎用 sea-query 建表，
/// 本函数用 `sql_type_of` 对照** ⇒ 引擎刚建完的库当场被判出「需加宽」，接着发出
/// SQLite 不支持的 `ALTER COLUMN`（实测 152/183 张表，详见 `heal_entity` 的闸）。
///
/// 现在本函数**只在 PG 上被调用**（由 `heal_entity` 的方言闸保证），所以上面那条
/// 前提不再承重。若将来有人在 SQLite 上放行它，`("BIGINT", "integer")` 这一条会
/// 立刻制造同样的假阳性 —— 加规则前先确认 DDL 在本方言存在。
///
/// 其余错配类暂不自动改（避免误伤），实证出现后再扩。
fn heal_target_type(expected_sql_type: &str, actual_type: &str) -> Option<&'static str> {
    // 浮点：单精度 → 双精度
    if expected_sql_type.eq_ignore_ascii_case("double precision")
        && actual_type.eq_ignore_ascii_case("real")
    {
        return Some("DOUBLE PRECISION");
    }
    // 整数：宽度分级向上加宽（smallint < integer < bigint），只加宽不收窄
    let int_width = |t: &str| match t.to_ascii_lowercase().as_str() {
        "smallint" => Some(1u8),
        "integer" | "int" | "int4" => Some(2),
        "bigint" | "int8" => Some(3),
        _ => None,
    };
    if let (Some(actual_w), Some(expected_w)) =
        (int_width(actual_type), int_width(expected_sql_type))
        && expected_w > actual_w
    {
        return Some(match expected_w {
            1 => "SMALLINT",
            2 => "INTEGER",
            _ => "BIGINT",
        });
    }
    None
}

/// 一次性拉取当前 schema 全部表的实际列集与类型。
async fn load_actual_columns(
    db: &sea_orm::DatabaseConnection,
    is_pg: bool,
) -> Result<ActualColumns, DbErr> {
    let mut map: ActualColumns = HashMap::new();
    if is_pg {
        let rows = db
            .query_all_raw(Statement::from_string(
                DbBackend::Postgres,
                "SELECT table_name, column_name, data_type FROM information_schema.columns \
                 WHERE table_schema = current_schema()"
                    .to_string(),
            ))
            .await?;
        for row in rows {
            let table: String = row.try_get("", "table_name")?;
            let column: String = row.try_get("", "column_name")?;
            let data_type: String = row.try_get("", "data_type")?;
            map.entry(table).or_default().insert(column, data_type);
        }
    } else {
        let tables = db
            .query_all_raw(Statement::from_string(
                DbBackend::Sqlite,
                "SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%'"
                    .to_string(),
            ))
            .await?;
        for t in tables {
            let table: String = t.try_get("", "name")?;
            let cols = db
                .query_all_raw(Statement::from_sql_and_values(
                    DbBackend::Sqlite,
                    "SELECT name, type FROM pragma_table_info(?)",
                    [table.clone().into()],
                ))
                .await?;
            let cols_map: HashMap<String, String> = cols
                .iter()
                .filter_map(|r| {
                    let name: String = r.try_get::<String>("", "name").ok()?;
                    let ty: String = r.try_get::<String>("", "type").unwrap_or_default();
                    Some((name, ty))
                })
                .collect();
            map.insert(table, cols_map);
        }
    }
    Ok(map)
}

/// 实体列类型 → 列级 DDL 类型名。修复语义下取「最接近且兼容」的映射：
/// 未精确覆盖的类型一律回退 TEXT（PG/SQLite 均接受任何内容的近亲）。
///
/// ## SQLite：整数族 / 浮点族**逐字**对齐 sea-query（2026-09-16）
///
/// 引擎（`reconcile::apply`）按 sea-query 建表，本函数用来对照与补列 ⇒ 两边必须是
/// **同一张名字表**。出处 `sea-query-1.0.2/src/backend/sqlite/table.rs:108-182`：
/// `TinyInteger|TinyUnsigned → tinyint`、`SmallInteger|SmallUnsigned → smallint`、
/// `Integer|Unsigned|BigInteger|BigUnsigned → integer`、`Float → float`、`Double → double`。
///
/// ⚠ 本仓**另一份**同源镜像在 `reconcile::model::sqlite_declared_type`（`:833-837`），
/// 那份是对的；本函数原来把 `BigInteger` 写成 `BIGINT`，于是同一份实体声明在两个渲染器
/// 下有了两个名字 —— 引擎刚建好的库当场被判出「期望 BIGINT / 实际 integer ⇒ 需加宽」，
/// 进而发出 SQLite 不支持的 `ALTER COLUMN`，实测 152/183 张表的列对照中断（详见
/// [`heal_entity`]）。**光加方言闸只是让它不再发 DDL，那张名字表本身还是错的**，所以这里
/// 逐字改对。判据：`sqlite_type_names_mirror_sea_query_verbatim`（断言字面量）。
///
/// **边界——刻意不整体委托给 `model::sqlite_declared_type`**：那份镜像对字符串 / 时间 /
/// JSON / UUID 等族**自述**「不直接调 sea-query 渲染…只需**亲和性类别**一致」，给出的是
/// `varchar` / `datetime_text` / `jsonb_text` / `vector_blob` / `real_decimal` 这类
/// **编造名**，且兜底臂是 `format!("{other:?}")`（未验证的调试串）。而本函数的返回值是
/// **真 DDL 文本**（`ALTER TABLE … ADD COLUMN {name} {sql_type}`）——把编造名或调试串写进
/// 库不是「对齐」，是引入新的未验证映射。故只对齐有 vendored 行号明证的整数族与浮点族，
/// 其余族沿用「未覆盖一律 TEXT/BLOB」这条保守契约。
///
/// ## PG：已知偏差（**本轮不动**，量纲 0）
///
/// `sea-query-1.0.2/src/backend/postgres/table.rs:32-35` 里 sea-query 对 `TinyInteger|TinyUnsigned|SmallInteger` 渲染
/// `smallint`、对 `Unsigned` 渲染 `bigint`，本函数一律给 `INTEGER`。不改的理由：
/// * 本仓实体与 L2 声明**均不产生**这些 `ColumnType`（实测 0 处）⇒ **偏差不可达**；
/// * `Unsigned → INTEGER` 是**收窄**方向，`heal_target_type` 的 `expected_w > actual_w`
///   在 `2 > 3 = false` 时不触发 ⇒ 即便可达也不会发出 DDL。
///
/// 依据：`output/tmp-lead-type-contract-table.log`（lead 裁定：本轮不动）。
fn sql_type_of(t: &sea_orm::sea_query::ColumnType, backend: DbBackend) -> &'static str {
    use sea_orm::sea_query::ColumnType as C;
    // (类型, 是否 PG) 二元组匹配——避免 match arm 内嵌 if 触发 clippy collapsible_match
    match (t, backend == DbBackend::Postgres) {
        (C::Char(_), _)
        | (C::String(_), _)
        | (C::Text, _)
        | (C::Enum { .. }, _)
        | (C::Custom(_), _)
        | (C::LTree, _)
        | (C::Cidr, _)
        | (C::Inet, _)
        | (C::MacAddr, _)
        | (C::Year, _)
        | (C::Interval(..), _)
        | (C::Vector(_), _)
        | (C::Bit(_), _)
        | (C::VarBit(_), _)
        | (C::Array(_), _) => "TEXT",
        (C::Blob | C::Binary(_) | C::VarBinary(_), true) => "BYTEA",
        (C::Blob | C::Binary(_) | C::VarBinary(_), false) => "BLOB",
        // ── SQLite：整数族 / 浮点族，逐字镜像 sea-query（见函数文档）──
        (C::TinyInteger | C::TinyUnsigned, false) => "tinyint",
        (C::SmallInteger | C::SmallUnsigned, false) => "smallint",
        (C::Integer | C::Unsigned | C::BigInteger | C::BigUnsigned, false) => "integer",
        (C::Float, false) => "float",
        (C::Double, false) => "double",
        // ── PG：本轮不动，已知偏差与不可达依据见函数文档 ──
        (
            C::TinyInteger
            | C::SmallInteger
            | C::Integer
            | C::TinyUnsigned
            | C::SmallUnsigned
            | C::Unsigned,
            true,
        ) => "INTEGER",
        (C::BigInteger | C::BigUnsigned, true) => "BIGINT",
        (C::Float, true) => "REAL",
        (C::Double, true) => "DOUBLE PRECISION",
        (C::Decimal(_), _) => "NUMERIC",
        (C::DateTime | C::Timestamp, true) => "TIMESTAMP",
        (C::DateTime | C::Timestamp, false) => "TEXT",
        (C::TimestampWithTimeZone, true) => "TIMESTAMPTZ",
        (C::TimestampWithTimeZone, false) => "TEXT",
        (C::Time, true) => "TIME",
        (C::Time, false) => "TEXT",
        (C::Date, true) => "DATE",
        (C::Date, false) => "TEXT",
        (C::Boolean, true) => "BOOLEAN",
        (C::Boolean, false) => "INTEGER",
        (C::Money(_), true) => "MONEY",
        (C::Money(_), false) => "REAL",
        (C::Json | C::JsonBinary, true) => "JSONB",
        (C::Json | C::JsonBinary, false) => "TEXT",
        (C::Uuid, true) => "UUID",
        (C::Uuid, false) => "TEXT",
        // sea-query ColumnType 标记 #[non_exhaustive]，未知新变体回退 TEXT
        _ => "TEXT",
    }
}

/// NOT NULL 列的 DEFAULT 子句；类型给不出合法 DEFAULT 时返回 None（退化为可空）。
fn not_null_default_suffix(
    t: &sea_orm::sea_query::ColumnType,
    backend: DbBackend,
) -> Option<String> {
    use sea_orm::sea_query::ColumnType as C;
    let is_pg = backend == DbBackend::Postgres;
    match t {
        C::TinyInteger
        | C::SmallInteger
        | C::Integer
        | C::BigInteger
        | C::TinyUnsigned
        | C::SmallUnsigned
        | C::Unsigned
        | C::BigUnsigned
        | C::Float
        | C::Double
        | C::Decimal(..)
        | C::Money(_) => Some(" NOT NULL DEFAULT 0".to_string()),
        C::Boolean => {
            if is_pg {
                Some(" NOT NULL DEFAULT FALSE".to_string())
            } else {
                Some(" NOT NULL DEFAULT 0".to_string())
            }
        },
        // 其余类型（TEXT/UUID/JSON/时间…）给不出跨行都合法的 DEFAULT 时，
        // 保守按可空补列：SELECT 不受影响，INSERT 由应用填值
        _ => None,
    }
}

/// 测试专用：`entity_registry` 里每个实体**自称的表名** —— 即 [`heal_entity`] 的查找键。
///
/// 为什么放在模块作用域而不是 `mod tests` 内：`entity_modules!` 是 `macro_rules!`
/// 文本作用域宏，在 `include!` 之后的**模块作用域**里可见；跨进嵌套模块要靠文本作用域
/// 的边角规则，不值得赌（赌输的症状是编译不过，好认；但也可能是「什么都没收集到」，
/// 那就成了对着空集合下判据）。
#[cfg(test)]
fn entity_registry_table_names() -> Vec<String> {
    // ⚠ `EntityName` 必须显式导入才能解析 `.table_name()`（它是 `EntityTrait` 的
    // supertrait，非泛型调用点不会自动带进来）。同 `expected.rs:43-51` 的实测结论。
    use sea_orm::EntityName;
    let mut names: Vec<String> = Vec::new();
    macro_rules! collect {
        ($($module:ident),* $(,)?) => {
            $(
                names.push(
                    axagent_entities::$module::Entity::default().table_name().to_string(),
                );
            )*
        };
    }
    entity_modules!(collect);
    names
}

#[cfg(test)]
mod tests {
    use super::*;
    use sea_orm::Database;

    /// 类型错配判定：real→DOUBLE PRECISION 与整数加宽需修复；其余组合不动。
    #[test]
    fn type_heal_rule_is_conservative() {
        // 已证实的错配类 1：SQLite 方言 REAL 在 PG 上 vs 实体 f64
        assert_eq!(heal_target_type("DOUBLE PRECISION", "real"), Some("DOUBLE PRECISION"));
        assert_eq!(heal_target_type("double precision", "REAL"), Some("DOUBLE PRECISION"));
        // 已是目标类型：不修
        assert_eq!(heal_target_type("DOUBLE PRECISION", "double precision"), None);
        // SQLite：实体期望 REAL、实际 REAL，相等不动
        assert_eq!(heal_target_type("REAL", "REAL"), None);

        // 已证实的错配类 2：SQLite 方言 INTEGER 在 PG = integer vs 实体 i64 = BIGINT
        assert_eq!(heal_target_type("BIGINT", "integer"), Some("BIGINT"));
        assert_eq!(heal_target_type("BIGINT", "int4"), Some("BIGINT"));
        // 小整数也逐级加宽
        assert_eq!(heal_target_type("BIGINT", "smallint"), Some("BIGINT"));
        assert_eq!(heal_target_type("INTEGER", "smallint"), Some("INTEGER"));
        // 已是目标宽度 / 收窄方向：不动（无损方向才动）
        assert_eq!(heal_target_type("BIGINT", "bigint"), None);
        assert_eq!(heal_target_type("INTEGER", "integer"), None);
        assert_eq!(heal_target_type("INTEGER", "bigint"), None);
        assert_eq!(heal_target_type("SMALLINT", "integer"), None);

        // 其它类型组合一律不动（保守，未实证不改）
        assert_eq!(heal_target_type("TEXT", "real"), None);
        assert_eq!(heal_target_type("INTEGER", "real"), None);
        assert_eq!(heal_target_type("BIGINT", "double precision"), None);
        assert_eq!(heal_target_type("TEXT", "integer"), None);
    }

    /// SQLite 侧类型名必须**逐字**等于 sea-query 写进 DDL 的名字。
    ///
    /// 出处（vendored）：`sea-query-1.0.2/src/backend/sqlite/table.rs:108-182`。
    /// 本仓另一份同源镜像在 `reconcile::model::sqlite_declared_type`（`:833-837`），
    /// 两份**互为对照**。本测试断言的是**字面量**，不是「两份相等」——后者恒真、零区分力。
    ///
    /// ⚠ 大小写是**判据的一部分**：SQLite 分支返回的是「sea-query 的原文」，故按 `==`
    /// 比字面量，不做 `eq_ignore_ascii_case`。
    ///
    /// ⚠ 修前必红的是整数族那 5 条：原实现是 `BigInteger|BigUnsigned => "BIGINT"` 与
    /// 打包的 `… => "INTEGER"`。2026-09-16 实测 152/183 张表对照中断的直接成因就是
    /// `BigInteger => "BIGINT"`（引擎按 sea-query 建出的是 `integer`）。
    #[test]
    fn sqlite_type_names_mirror_sea_query_verbatim() {
        use sea_orm::sea_query::ColumnType as C;

        // 整数族（sea-query-1.0.2/src/backend/sqlite/table.rs:108-182）
        assert_eq!(sql_type_of(&C::TinyInteger, DbBackend::Sqlite), "tinyint");
        assert_eq!(sql_type_of(&C::TinyUnsigned, DbBackend::Sqlite), "tinyint");
        assert_eq!(sql_type_of(&C::SmallInteger, DbBackend::Sqlite), "smallint");
        assert_eq!(sql_type_of(&C::SmallUnsigned, DbBackend::Sqlite), "smallint");
        assert_eq!(sql_type_of(&C::Integer, DbBackend::Sqlite), "integer");
        assert_eq!(sql_type_of(&C::Unsigned, DbBackend::Sqlite), "integer");
        // ★ 本次缺陷的成因条目
        assert_eq!(sql_type_of(&C::BigInteger, DbBackend::Sqlite), "integer");
        assert_eq!(sql_type_of(&C::BigUnsigned, DbBackend::Sqlite), "integer");

        // 浮点族
        assert_eq!(sql_type_of(&C::Float, DbBackend::Sqlite), "float");
        assert_eq!(sql_type_of(&C::Double, DbBackend::Sqlite), "double");

        // 区分力：三个整数**宽度必须给出三个不同的名字**。把 tinyint/smallint 也并成
        // `integer` 是对这张表最自然的「抄错」形态（`model.rs` 那份把 Integer 及以上
        // 合并，但保留 tinyint/smallint 之别），上面对照不出来。
        let widths = [
            sql_type_of(&C::TinyInteger, DbBackend::Sqlite),
            sql_type_of(&C::SmallInteger, DbBackend::Sqlite),
            sql_type_of(&C::Integer, DbBackend::Sqlite),
        ];
        let mut uniq = widths.to_vec();
        uniq.sort_unstable();
        uniq.dedup();
        assert_eq!(uniq.len(), 3, "三个整数宽度必须给出三个不同的 SQLite 名字，实际 {widths:?}");

        // PG 侧本轮不动：只钉住「拆臂时别把 PG 一起改了」这三条（PG 的已知偏差见函数文档）
        assert_eq!(sql_type_of(&C::BigInteger, DbBackend::Postgres), "BIGINT");
        assert_eq!(sql_type_of(&C::Float, DbBackend::Postgres), "REAL");
        assert_eq!(sql_type_of(&C::Double, DbBackend::Postgres), "DOUBLE PRECISION");
    }

    /// ★ 方言闸回归：SQLite 上**不得**发出 `ALTER COLUMN`。
    ///
    /// 夹具构造出**真实**的误判：把实体一个 i64 列声明成比实体期望**更窄**的 SQLite
    /// 类型（`smallint`）—— 旧库上确实有这种历史迁移手写的窄列。此时
    /// `heal_target_type` 会**正确地**判出「需加宽」，但加宽那条 DDL
    /// （`ALTER TABLE … ALTER COLUMN … TYPE …`）**在 SQLite 根本不存在**：语句在第二个
    /// `ALTER` 处解析失败，该表后续全部列的对照随之被 `?` 中断（2026-09-16 实测
    /// 152/183 张表正是这个形态，只是当年的触发词是那张抄错的名字表）。
    ///
    /// ⚠ 夹具**刻意不用 `integer`**：那是引擎给 i64 列写的名字，与 [`sql_type_of`]
    /// 对齐之后它不再触发加宽规则 ⇒ 用 `integer` 会让本测试退化成真空真（绿，但零信息）。
    /// `smallint` 才这条闸真正要挡的场景。
    ///
    /// 前提**自证**：下方 `heal_target_type(...) == Some("INTEGER")` 一条钉住
    /// 「这个夹具确实会让规则开火」——否则本测试无法区分「闸生效」与「规则没触发」。
    #[tokio::test]
    async fn sqlite_never_emits_alter_column() {
        use axagent_entities::notes::{Column as NotesColumn, Entity as NotesEntity};
        use sea_orm::EntityName;
        use sea_orm::sea_query::ColumnType as C;

        // 找一个实体声明为 BigInteger(i64) 的列。不硬编码列名：实体改名/换类型时这条
        // 测试跟着走，而不是变成一条永远为真的历史记录。按 **ColumnType** 选而不是按
        // `sql_type_of` 的输出选 —— 后者是被测对象之一，用它选列会让夹具跟着它漂。
        let wide = NotesColumn::iter()
            .find(|c| matches!(c.def().get_column_type(), C::BigInteger))
            .expect("notes 实体应有 i64 列，夹具的前提才成立");

        // 前提自证（见函数文档末段）
        let expected_type = sql_type_of(&C::BigInteger, DbBackend::Sqlite);
        assert_eq!(
            heal_target_type(expected_type, "smallint"),
            Some("INTEGER"),
            "夹具失效：`smallint` 不再被判成「比 {expected_type} 窄」⇒ 本测试无法区分\
             「闸生效」与「规则压根没触发」"
        );

        let db = Database::connect("sqlite::memory:").await.expect("连接应成功");
        // `Entity` 是单元结构体，直接用它本身而不是 `Entity::default()`
        // （clippy `default_constructed_unit_structs`；宏展开里的同类写法不会被查，
        // 手写的会被查 —— 所以这里必须写成单元值）
        let table = NotesEntity.table_name().to_string();
        db.execute_unprepared(&format!(
            "CREATE TABLE {table} (id TEXT NOT NULL PRIMARY KEY, {} smallint)",
            wide.as_str()
        ))
        .await
        .expect("建夹具表应成功");

        let report = heal_all(&db).await.expect("heal_all 应成功");

        // 反空真：表在，`heal_entity` 才会真的走进列循环（否则下面两条是真空真）
        assert_eq!(
            report.tables_scanned, 1,
            "夹具表 {table} 应被对照到且不报错；实际 tables_scanned={} errors={:?}",
            report.tables_scanned, report.errors
        );
        assert!(
            report.errors.is_empty(),
            "SQLite 上不该有任何实体对照失败 —— `ALTER COLUMN` 是本方言没有的 DDL: {:?}",
            report.errors
        );
        let target = format!("{table}.{}", wide.as_str());
        assert!(
            !report.types_healed.contains(&target),
            "SQLite 上不该做类型加宽：它是动态类型，列声明的类型名不影响 sea-orm 解码。\
             若要加宽 {target}，DDL 在本方言根本无法执行。实际: {:?}",
            report.types_healed
        );
    }

    /// 查询 SQLite 表中是否存在指定列。
    async fn column_exists(db: &sea_orm::DatabaseConnection, table: &str, column: &str) -> bool {
        let rows = db
            .query_all_raw(Statement::from_sql_and_values(
                DbBackend::Sqlite,
                "SELECT name FROM pragma_table_info(?)",
                [table.into()],
            ))
            .await
            .expect("查询应成功");
        rows.iter().any(|r| r.try_get_by::<String, _>("name").unwrap_or_default() == column)
    }

    /// 核心回归：模拟「表先以窄 schema 存在、实体声明了更多列」的存量库。
    /// 夹具完全实体驱动——只建 notes 表的 id 一列，期望列集从
    /// axagent_entities::notes::Entity 动态提取（不硬编码任何列名），
    /// 上游/下游仓库通用。
    #[tokio::test]
    async fn heals_missing_columns_from_entity_definitions() {
        let db = Database::connect("sqlite::memory:").await.expect("连接应成功");
        db.execute_unprepared("CREATE TABLE notes (id TEXT NOT NULL PRIMARY KEY)")
            .await
            .expect("建夹具表应成功");

        // 期望列集 = 实体声明的全部列
        let expected: Vec<String> =
            axagent_entities::notes::Column::iter().map(|c| c.as_str().to_string()).collect();
        assert!(expected.len() > 2, "notes 实体应有多个列，夹具才有意义");

        let report = heal_all(&db).await.expect("heal_all 应成功");

        // heal 后：实体的每一列都应实际存在
        for col in &expected {
            assert!(column_exists(&db, "notes", col).await, "实体列 {col} 应被补齐");
        }
        // id 本来就存在，不应出现在补列报告里
        assert!(
            !report.columns_added.iter().any(|c| c == "notes.id"),
            "已有列不应重复补，实际: {:?}",
            report.columns_added
        );
        assert!(
            report.columns_added.iter().any(|c| c.starts_with("notes.")),
            "报告应包含 notes 的补列记录，实际: {:?}",
            report.columns_added
        );
    }

    /// 幂等：补过列后再跑，报告应为空且不报错。
    #[tokio::test]
    async fn is_idempotent() {
        let db = Database::connect("sqlite::memory:").await.expect("连接应成功");
        db.execute_unprepared("CREATE TABLE notes (id TEXT NOT NULL PRIMARY KEY)")
            .await
            .expect("建夹具表应成功");
        heal_all(&db).await.expect("首次应成功");
        let report = heal_all(&db).await.expect("二次应成功");
        assert!(
            report.columns_added.is_empty(),
            "二次不应再补列，实际: {:?}",
            report.columns_added
        );
    }

    /// 空库：所有实体表缺失时应全部跳过（diff 不凭空造表），不报错。
    #[tokio::test]
    async fn skips_missing_tables() {
        let db = Database::connect("sqlite::memory:").await.expect("连接应成功");
        let report = heal_all(&db).await.expect("空库上应安全通过");
        assert!(
            report.columns_added.is_empty() && report.tables_scanned == 0,
            "空库不应补任何列，实际: {:?}",
            report
        );
    }

    /// ★ **判决性对照实验**：`repair_schema` 报的 `tables_scanned=31`（而期望表数 183）
    /// 到底是哪一种？三种假设会给出**不同的数**，故一次打印即可分开：
    ///
    /// | 观察 | A：实体表名与实况 key 不匹配 | B：`load_actual_columns` 取数漏表 | C：单实体 `Err` 被 `warn!` 吞掉 |
    /// |---|---|---|---|
    /// | `load_actual_columns` 的 key 数 | ≈ 实况表数 | ≈ 31 | ≈ 实况表数（+ 不过滤的元表） |
    /// | 实体键命中率 | 低 | 低 | **满**（表都在，问题在列/DDL 那一层） |
    /// | 逐实体复算的 `Err` 数 | 0 | 0 | **非 0** |
    ///
    /// 关键：`heal_all` 的 `tables_scanned` 只统计 `Ok((true, ..))` ——
    /// `Ok((false, ..))`（表缺失）与 **`Err(..)`（`heal_all` 只 `warn!`、不计数）**
    /// 都不进这个数。所以「31」也可能根本与「表在不在」无关。
    ///
    /// ## 结论（2026-09-16 实跑，非推测）
    ///
    /// `entity_modules` 条数 = 去重后 = **183** = `tables_expected`（无重名表）；
    /// 实体键命中 **183/183**、落空 0；`load_actual_columns` 的 key 数 188
    /// （多出的 4 张是 `_ax_schema_*` 引擎元表，正常）；「只在 introspect 里」为空集。
    /// ⇒ **A（表名不匹配）与 B（取数漏表）都被否**，成立的是 C：逐实体复算得到
    /// `命中表=31 ／ 表缺失=0 ／ 报错=152`，错误串 `near "ALTER": syntax error`
    /// —— 即 `heal_entity` 发出 SQLite 不支持的 `ALTER COLUMN` 后中断，
    /// 失败被 `heal_all` 的 `warn!` 吞掉。
    ///
    /// 修法已落地（见 `heal_entity` 的方言闸与 [`DiffReport::errors`]）。
    /// **修后本测试第 2 步应打印 `命中表=175 ／ 表缺失=8 ／ 报错=0`**；
    /// ⚠ 这三个数在 **2026-09-17** 变了，不是缺陷：`NON_MAIN_DB` 的 `dialect` 由
    /// `Some(Postgres)` 改为 `None` ⇒ 8 张侧车库表在**两个方言**上都不再进期望集、
    /// 也不再被建进主库，于是本测试的测试池里它们**不存在** ⇒ 记为「表缺失 8」。
    /// （原读数 `183 / 0 / 0` 对应「SQLite 侧仍认这 8 张」的旧语义。）
    /// 若又出现非 0，就是新的真发现，不要只当成噪音。
    ///
    /// ⚠ 本测试**刻意无结论性断言**（只留一条「探针确实读到了东西」的反空真断言）：
    /// 它的产出是数字，判据在 `sqlite_never_emits_alter_column`（方言闸）与
    /// `migrations::tests::repair_schema_on_converged_db_reports_no_heal`（`errors` 为空）里。
    #[tokio::test]
    async fn probe_heal_all_coverage_against_entity_registry() {
        let handle = crate::db::create_test_pool().await.expect("测试库应可建立");
        let db = &handle.conn;

        let actual = load_actual_columns(db, false).await.expect("列快照应可读");
        let intro = crate::reconcile::introspect::read(db).await.expect("introspect 应成功");
        let entities = entity_registry_table_names();

        let actual_keys: std::collections::BTreeSet<&str> =
            actual.keys().map(String::as_str).collect();
        let intro_names: std::collections::BTreeSet<&str> =
            intro.tables.iter().map(|t| t.name.as_str()).collect();

        let mut uniq = entities.clone();
        uniq.sort();
        uniq.dedup();

        let missing: Vec<&String> =
            entities.iter().filter(|t| !actual.contains_key(t.as_str())).collect();
        let hit = entities.len() - missing.len();

        println!("[判决] entity_modules 条数={} 去重后={}", entities.len(), uniq.len());
        println!(
            "[判决] introspect::read 表数={} ／ load_actual_columns key 数={}",
            intro.tables.len(),
            actual.len()
        );
        println!("[判决] 实体键命中 {hit} ／ 落空 {}", missing.len());
        println!("[判决] 落空实体名（前 20）: {:?}", missing.iter().take(20).collect::<Vec<_>>());
        println!(
            "[判决] 只在 introspect 里（前 20）: {:?}",
            intro_names.difference(&actual_keys).take(20).collect::<Vec<_>>()
        );
        println!(
            "[判决] 只在 load_actual_columns 里（前 20）: {:?}",
            actual_keys.difference(&intro_names).take(20).collect::<Vec<_>>()
        );
        println!(
            "[判决] actual.keys（前 20，升序）: {:?}",
            actual_keys.iter().take(20).collect::<Vec<_>>()
        );
        println!("[判决] 实体名（前 20，升序）: {:?}", uniq.iter().take(20).collect::<Vec<_>>());
        // 形态线索：落空的名字是否只是大小写不同（或带了引号/模式前缀）
        let lower: std::collections::HashMap<String, &String> =
            actual.keys().map(|k| (k.to_lowercase(), k)).collect();
        for m in missing.iter().take(10) {
            println!("[判决]   落空 {m} → 大小写不敏感命中: {:?}", lower.get(&m.to_lowercase()));
        }

        // ── 第二步：逐实体复算 `heal_entity` 的**三种**结局 ──
        //
        // 上一步只证明了「表名都能查到」。但 `heal_all` 的 `tables_scanned` 只统计
        // `Ok((true, ..))` —— `Ok((false, ..))`（表缺失）与 **`Err(..)`（单实体 diff
        // 失败，heal_all 只 `warn!` 不计数）** 都不进那个数。所以 `31` 还可能是
        // 「152 个实体抛错被静默降级」而不是「152 张表找不到」。
        //
        // ⚠ 本段**会执行 DDL**（`heal_entity` 在判定缺列/类型错配时自己 ALTER），与
        // `heal_all` 等价 —— 这正是复算它的账所必需的，且只发生在测试库上。
        let mut ok = 0usize;
        let mut absent = 0usize;
        let mut errs: Vec<(String, String)> = Vec::new();
        macro_rules! census {
            ($($module:ident),*) => {
                $(
                    match heal_entity::<axagent_entities::$module::Entity>(db, &actual).await {
                        Ok((true, _, _)) => ok += 1,
                        Ok((false, _, _)) => absent += 1,
                        Err(e) => errs.push((stringify!($module).to_string(), e.to_string())),
                    }
                )*
            };
        }
        entity_modules!(census);
        println!(
            "[判决] heal_entity 逐实体复算：命中表={ok} ／ 表缺失={absent} ／ 报错={}",
            errs.len()
        );
        for (m, e) in errs.iter().take(8) {
            println!("[判决]   报错实体 {m}: {e}");
        }

        // 反空真：探针必须**真的读到了东西**，否则上面那些 0 ／ 空集合没有意义。
        assert!(
            !intro.tables.is_empty() && !entities.is_empty(),
            "探针没读到任何表（introspect={} ／ 实体={}），上面的数没有意义",
            intro.tables.len(),
            entities.len()
        );
    }
}
