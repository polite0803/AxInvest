use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

/// 调度器 leader 租约 —— **跨进程互斥的唯一权威**。
///
/// ## 为什么是「租约表」而不是 PG advisory lock
///
/// 首选方案是 `pg_try_advisory_lock`（会话级、进程退出自动释放，语义最干净），
/// 但本仓的 `DatabaseConnection` 是**连接池**（`db.rs` 里 `max_connections` 有上界）
/// ⇒ 会话级 advisory lock 会绑在池中某个连接上，**下一轮 tick 用的可能是另一个连接**
/// ⇒ 锁语义失效（这正是「同名不同义」的跨载体版本）。
///
/// 事务级 `pg_try_advisory_xact_lock` 又会随事务结束立刻释放 ⇒ 变成「一次性抢票」，
/// 不构成互斥。
///
/// ⇒ 改用**租约表**：一行 = 一把带过期时间的锁。
///   · 抢锁 = UPSERT 一行（`holder_id` = 本进程 id，`expires_at` = now + TTL）；
///   · 续租 = 每 TTL/3 更新 `expires_at`；
///   · 释放 = 删除本进程持有的行；
///   · 进程崩溃 ⇒ 租约到期自动失效（**不需要清理逻辑**）。
///
/// ## 方言
///
/// 纯 SQL + SeaORM，**SQLite / PostgreSQL 双方言**（本仓 DB 两者都支持）
/// ⇒ 不用 PG 专有语法，测试可用 SQLite 内存库跑。
///
/// ## 竞态
///
/// 抢锁是「UPSERT + 条件更新」：只有当**没有有效租约**或**租约已过期**时才接管。
/// 两个进程同时抢 ⇒ 靠 DB 的行级串行化收敛到**恰好一个** holder
/// （`UPDATE ... WHERE expires_at < now OR holder_id = me` 的受影响行数判据）。
#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize)]
#[sea_orm(table_name = "cron_leader_lease")]
#[serde(rename_all = "camelCase")]
pub struct Model {
    /// 租约名（当前只有一个：`scheduler`）。主键。
    #[sea_orm(primary_key, auto_increment = false)]
    pub lease_name: String,
    /// 持有者标识（本进程启动时生成，进程内唯一）。
    pub holder_id: String,
    /// 租约到期时刻（UTC ms）。**过期即自动失效** ⇒ 进程崩溃无需清理。
    pub expires_at: i64,
    /// 最后一次续租时刻（UTC ms）—— 供「租约是否在跳」诊断用。
    pub heartbeat_at: i64,
    /// 持有者进程启动时刻（UTC ms）—— 诊断用（谁占了、从什么时候占的）。
    pub acquired_at: i64,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
