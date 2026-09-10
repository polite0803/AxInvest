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
//! **只补缺失列/修类型，不删已有列，缺表跳过**（建表仍是 migration
//! 的职责），天然幂等。
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
    /// 扫描的实体表数（缺表不计入）
    pub tables_scanned: usize,
    /// 补建的列，格式 `table.column`
    pub columns_added: Vec<String>,
    /// 修复类型的列，格式 `table.column`
    pub types_healed: Vec<String>,
}

/// 入口：对照全部已注册实体补缺失列。
///
/// 逐实体容错——单个实体的 diff 失败（如类型映射外的 DDL 错误）只告警
/// 不中断，其余实体继续。在 `repair_schema()` 迁移重跑**之后**调用：
/// 先让迁移把表建出来，diff 再补列。
pub async fn heal_all(db: &sea_orm::DatabaseConnection) -> Result<DiffReport, DbErr> {
    let is_pg = db.get_database_backend() == DbBackend::Postgres;
    let actual = load_actual_columns(db, is_pg).await?;
    let mut report = DiffReport::default();

    // 递归宏展开：entity_modules!(cb) → cb!(mod1, mod2, ...) → 逐实体 diff
    macro_rules! heal_entities {
        ($($module:ident),* $(,)?) => {
            $(
                match heal_entity::<axagent_entities::$module::Entity>(db, &actual).await {
                    Ok((scanned, mut added, mut healed)) => {
                        if scanned {
                            report.tables_scanned += 1;
                        }
                        report.columns_added.append(&mut added);
                        report.types_healed.append(&mut healed);
                    },
                    // 单实体失败只告警：修复层不阻塞其余实体与迁移版本记录
                    Err(e) => {
                        tracing::warn!("[schema_diff] 实体 {} diff 失败: {}", stringify!($module), e);
                    },
                }
            )*
        };
    }

    entity_modules!(heal_entities);

    if report.columns_added.is_empty() && report.types_healed.is_empty() {
        tracing::info!(
            "[schema_diff] {} 张实体表对照完成，无缺失列/类型错配",
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
/// 表不存在时跳过（建表是 migration 的职责，diff 不凭空造表）。
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

    let mut added = Vec::new();
    let mut healed = Vec::new();
    for col in E::Column::iter() {
        let name = col.as_str();
        let def = col.def();
        let sql_type = sql_type_of(def.get_column_type(), db.get_database_backend());
        match actual_cols.get(name) {
            Some(actual_type) => {
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
                    not_null_default_suffix(def.get_column_type(), db.get_database_backend())
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
/// SQLite 下类型名与实体期望一致，不会进入本函数。其余错配类暂不自动改
/// （避免误伤），实证出现后再扩。
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
        (
            C::TinyInteger
            | C::SmallInteger
            | C::Integer
            | C::TinyUnsigned
            | C::SmallUnsigned
            | C::Unsigned,
            _,
        ) => "INTEGER",
        (C::BigInteger | C::BigUnsigned, _) => "BIGINT",
        (C::Float, _) => "REAL",
        (C::Double, true) => "DOUBLE PRECISION",
        (C::Double, false) => "REAL",
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
}
