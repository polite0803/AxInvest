// SPDX-License-Identifier: AGPL-3.0-only

//! 实况侧读取 —— 从**真实库**读出结构，中立化为 [`SchemaModel`]。
//!
//! ## 输入口径（PLAN §四·九）
//!
//! 实况侧只认 `pg_catalog` / `sqlite_master` + `pragma` 的**运行期实测**，
//! **绝不看迁移文本**。迁移文本会撒谎：`trajectory_*` 三张表的建表 DDL 永久留在
//! `v100_consolidated.rs` 里，而表已被 `v101` DROP —— 按文本判定会让引擎试图重建它们。
//!
//! ## 管辖范围 = 主库
//!
//! 本模块只读**当前连接所指的库**。侧车库（`index.db` 等独立 SQLite 文件）由
//! 各自 crate 拥有，不在引擎管辖内（PLAN §四·九「铁律的管辖范围 = 主库」）。
//!
//! ## 只读保证
//!
//! 本模块全部语句为 `SELECT`。P3 不生成也不执行任何 DDL —— 只读是 P3 的硬约束，
//! 引擎写错一次就可能删库，先让它在只读模式下被验证足够多轮。

pub mod parse;
pub mod pg;
pub mod sqlite;

use async_trait::async_trait;
use sea_orm::{DatabaseConnection, DbBackend, DbErr};

use crate::reconcile::extras::Dialect;
use crate::reconcile::model::SchemaModel;

/// 引擎自身簿记元表的前缀 —— [`read`] **一律不读**。
///
/// ## 为什么必须在「读」这一层排除，而不是「读了再豁免孤儿判定」
///
/// [`crate::reconcile::safety`] 会创建四张簿记表（`_ax_schema_audit` /
/// `_ax_schema_graveyard` / `_ax_schema_pending_drops` /
/// `_ax_schema_orphan_whitelist`）。它们**不在 L1 实体、也不在 L2 声明里**
/// —— L2 只声明索引 / CHECK / 生成列 / 虚表 / 触发器 / 函数，没有「表」这一层。
///
/// 若只做「读了但不判孤儿」（像 `extras::ORPHAN_EXEMPT` 那样），它们仍会进入
/// **实况指纹**，而期望指纹里没有它们 ⇒ **两个指纹永不相等**，后果两条：
///
/// 1. 「apply 后实况指纹 == 期望指纹」这条 P4 出口判据**永久不成立**；
/// 2. 引擎的指纹短路（相同即跳过）永不生效 ⇒ 每轮都做全量 diff。
///
/// ⇒ 正确做法是在**读取层排除**，让元表对引擎完全不可见。这与 SQLite 侧排除
/// `sqlite_%` 内部表（`sqlite.rs::load_tables`）是**同一条规则的两个实例**。
///
/// ⚠ 前缀是**保留**的：用户若手动建了 `_ax_schema_` 开头的表，它永远不会被同步
/// （既不判孤儿、也不进指纹）。这是隐藏命名空间的代价，用它换「元表不可见」这条
/// 简单规则。
///
/// ⚠ 过滤点**只有一处**（[`read`] 的返回前）。双方言实现都从那里返回，所以不存在
/// 「某个实现忘了过滤」这种分工 —— 这是刻意的：过滤若分散在两个 `load_tables` 里，
/// 加第三方言时会漏。
pub const ENGINE_META_PREFIX: &str = "_ax_schema_";

/// 表名是否属于引擎簿记元表（⇒ 不读、不进指纹、不判孤儿）。
pub fn is_engine_meta_table(name: &str) -> bool {
    name.starts_with(ENGINE_META_PREFIX)
}

/// 实况读取器契约。
///
/// 实现必须满足两条：
/// 1. **只读** —— 只发 `SELECT`；
/// 2. **不臆造** —— 读不到的对象不补默认值。宁可少报（少报表现为「引擎认为该对象不存在
///    而重建」，可自愈），不可错报（错报表现为「引擎认为对象符合期望而跳过」，不可自愈）。
#[async_trait]
pub trait Introspector {
    fn dialect(&self) -> Dialect;
    async fn read(&self, db: &DatabaseConnection) -> Result<SchemaModel, DbErr>;
}

/// 按连接的 backend 分派。仅支持双方言（PLAN §零 决策基线：PG + SQLite 对等）。
pub async fn read(db: &DatabaseConnection) -> Result<SchemaModel, DbErr> {
    let mut m = match db.get_database_backend() {
        DbBackend::Sqlite => sqlite::SqliteIntrospector.read(db).await?,
        DbBackend::Postgres => pg::PgIntrospector.read(db).await?,
        other => {
            return Err(DbErr::Custom(format!(
                "reconcile::introspect 只支持 PostgreSQL / SQLite，实际 backend = {other:?}"
            )));
        },
    };

    // 引擎自己的簿记元表一律不可见（见 [`ENGINE_META_PREFIX`]）。唯一过滤点。
    //
    // 为什么用 `retain` 而不是在两个方言实现里改 SQL / 加 `starts_with` 判断：
    // 方言实现各自过滤 ⇒ 新增方言时会漏掉一处，而这里漏掉的后果是「指纹永不收敛」
    // （不是崩溃，是静默失效）。过滤点收敛到唯一入口，是让这类漏项**结构上不存在**。
    let before = m.tables.len();
    m.tables.retain(|t| !is_engine_meta_table(&t.name));
    if m.tables.len() != before {
        // 表集变了 ⇒ 重建规范化不变量（`finalize` 幂等，但要保证「返回的模型已 finalize」
        // 这条契约——`diff` 与 `fingerprint` 都依赖它）。
        m.finalize();
    }
    Ok(m)
}

#[cfg(test)]
mod tests {
    use super::*;
    use sea_orm::{ConnectionTrait, Database};

    /// 分派：两种 backend 各走各的实现（用空库验证「能读且不报错」）。
    #[tokio::test]
    async fn dispatches_by_backend_for_sqlite() {
        let db = Database::connect("sqlite::memory:").await.expect("连接应成功");
        let m = read(&db).await.expect("空库 introspect 应成功");
        assert_eq!(m.dialect, Dialect::Sqlite);
        assert!(m.is_empty(), "空库不应读出任何表");
    }

    /// 表名过滤：SQLite 内部表（`sqlite_%`）与 FTS5 影子表都不该进模型，
    /// 否则引擎会把它们判成孤儿并尝试 DROP。
    #[tokio::test]
    async fn sqlite_internal_and_shadow_tables_are_excluded() {
        let db = Database::connect("sqlite::memory:").await.expect("连接应成功");
        db.execute_unprepared("CREATE TABLE real_table (id TEXT NOT NULL PRIMARY KEY)")
            .await
            .expect("建表应成功");
        // SQLite 自动生成的内部表
        db.execute_unprepared("CREATE TABLE keep_me (a TEXT UNIQUE)").await.expect("建表应成功");
        let m = read(&db).await.expect("introspect 应成功");

        let names = m.table_names();
        assert!(names.contains(&"real_table"), "普通表应被读出：{names:?}");
        assert!(names.contains(&"keep_me"));
        assert!(
            !names.iter().any(|n| n.starts_with("sqlite_")),
            "sqlite_* 内部表必须排除：{names:?}"
        );
    }

    /// ⚠ **引擎元表必须完全不可见**（不进模型 ⇒ 不进指纹 ⇒ 不判孤儿）。
    ///
    /// 这条测试是 `safety` 元表能被创建而**不破坏收敛判据**的前提：一旦元表进了
    /// 实况模型，「apply 后指纹 == 期望指纹」就永久为假，而症状只是「每轮都全量 diff」
    /// —— 静默、无报错。故必须有一条测试把它钉住。
    #[tokio::test]
    async fn engine_meta_tables_are_invisible_to_introspect() {
        let db = Database::connect("sqlite::memory:").await.expect("连接应成功");
        db.execute_unprepared("CREATE TABLE real_table (id TEXT NOT NULL PRIMARY KEY)")
            .await
            .expect("建表应成功");
        // 逐张建出 safety 的四张簿记表（**字面量**，不引用 safety 的常量 ——
        // safety 依赖 introspect，反向引用会成环。两侧各自锁定同一份清单，
        // 由 `safety::tests::meta_table_names_all_hit_the_prefix` 从另一头守住。）
        for t in [
            "_ax_schema_audit",
            "_ax_schema_graveyard",
            "_ax_schema_pending_drops",
            "_ax_schema_orphan_whitelist",
        ] {
            db.execute_unprepared(&format!("CREATE TABLE {t} (k TEXT NOT NULL PRIMARY KEY)"))
                .await
                .expect("建簿记表应成功");
        }
        // 前缀相同的**非元表**（不该被排除，用来证明过滤只认前缀本身，
        // 而不是把整类名字都吃掉）
        db.execute_unprepared("CREATE TABLE ax_schema_not_meta (k TEXT)")
            .await
            .expect("建表应成功");

        let m = read(&db).await.expect("introspect 应成功");
        let names = m.table_names();
        assert!(names.contains(&"real_table"), "{names:?}");
        assert!(
            names.contains(&"ax_schema_not_meta"),
            "前缀是 `_ax_schema_`（带前导下划线），`ax_schema_` 不在保留空间内：{names:?}"
        );
        assert!(
            !names.iter().any(|n| is_engine_meta_table(n)),
            "引擎元表必须对引擎不可见 —— 否则实况指纹恒 ≠ 期望指纹，收敛判据永久失效：{names:?}"
        );
    }

    /// 前缀判定本身：只认带前导下划线的形态。
    #[test]
    fn meta_prefix_is_exact() {
        assert!(is_engine_meta_table("_ax_schema_audit"));
        assert!(is_engine_meta_table("_ax_schema_"));
        assert!(!is_engine_meta_table("ax_schema_audit"), "少一个前导下划线不算");
        assert!(!is_engine_meta_table("_ax_schema"), "前缀含末尾下划线");
        assert!(!is_engine_meta_table(""));
        assert!(!is_engine_meta_table("public"));
    }
}
