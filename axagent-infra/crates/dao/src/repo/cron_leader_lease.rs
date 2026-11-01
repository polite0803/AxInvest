// SPDX-License-Identifier: AGPL-3.0-only

//! 调度器 leader 租约（跨进程互斥）。
//!
//! ## 用途
//!
//! 调度器（cron）此前**没有任何跨进程互斥** ⇒ 若同时有两个进程（桌面应用 + 将来的
//! 常驻 daemon，或用户开了两个实例），**两边都会读到同一批到期任务并各自派发一次**
//! ⇒ 同一个 as_of 回放日跑两遍、荐股跑两遍、决策跑两遍 ⇒ 落库冲突 + 样本重复计数。
//!
//! ## 为什么是租约表而不是 PG advisory lock
//!
//! 首选 `pg_try_advisory_lock`（会话级、进程退出自动释放），但本仓的
//! `DatabaseConnection` 是**连接池** ⇒ 会话级锁绑在池中某个连接上，
//! 下一轮 tick 用的可能是另一个连接 ⇒ 锁语义失效。
//! 事务级 `pg_try_advisory_xact_lock` 随事务结束即释放 ⇒ 不构成互斥。
//! ⇒ 改用**带过期时间的租约行**（双方言、自动过期、无需清理逻辑）。
//!
//! ## 过期即释放
//!
//! 进程崩溃 ⇒ 不再续租 ⇒ `expires_at` 到点自动失效 ⇒ 别的进程可接管。
//! **不依赖任何「进程退出时清理」逻辑**（那条路径在 kill -9 / 断电时不成立）。

use axagent_entities::cron_leader_lease;
use sea_orm::{ActiveModelTrait, ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, Set};
use std::time::Duration;

/// 租约名（当前只有一个调度器 ⇒ 一把锁）。
pub const LEASE_SCHEDULER: &str = "scheduler";

/// 租约默认有效期。
///
/// ⚠ 必须**显著大于**续租间隔（心跳周期），否则一次 GC 停顿就会让租约过期
/// ⇒ 两个进程同时当 leader。
/// ⚠ 同时必须**显著小于**「进程崩溃到别人接管」的容忍窗口（分钟级即可）。
pub const LEASE_TTL: Duration = Duration::from_secs(90);

/// 续租间隔（TTL 的 1/3 ⇒ 允许连续 2 次心跳失败仍不丢租约）。
pub const LEASE_HEARTBEAT_INTERVAL: Duration = Duration::from_secs(30);

/// 抢租约的结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LeaseAcquire {
    /// 抢到了（本进程现在持有）。
    Acquired,
    /// 没抢到：已有他人持有且租约仍有效。
    HeldByOther { holder_id: String, expires_at: i64 },
    /// 没抢到，但原因是**租约已过期**且接管失败（并发抢同一把过期租约时，
    /// 别人先抢到了）—— 与 `HeldByOther` 同义，保留独立分支是为了让日志能区分
    /// 「有活跃 leader」与「有过期残留正在被清理」。
    StaleTakenByOther { holder_id: String },
}

/// 当前租约持有者（用于诊断与 UI 回显）。
#[derive(Debug, Clone)]
pub struct LeaseInfo {
    pub lease_name: String,
    pub holder_id: String,
    pub expires_at: i64,
    pub heartbeat_at: i64,
    pub acquired_at: i64,
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

/// 尝试抢租约。
///
/// 语义：**只有在「没有有效租约」或「租约已过期」时**才接管 ⇒ 两个进程同时抢，
/// DB 的行级串行化保证**恰好一个**成功（另一个看到有效租约即返回 `HeldByOther`）。
pub async fn try_acquire(
    db: &DatabaseConnection,
    lease_name: &str,
    holder_id: &str,
    ttl: Duration,
) -> Result<LeaseAcquire, String> {
    let now = now_ms();
    let expires_at = now + ttl.as_millis() as i64;

    // 1) 先读现状（用于分支判定与诊断信息）。
    let existing = cron_leader_lease::Entity::find_by_id(lease_name.to_string())
        .one(db)
        .await
        .map_err(|e| format!("读租约 {} 失败: {e}", lease_name))?;

    if let Some(row) = existing {
        if row.expires_at > now && row.holder_id != holder_id {
            // 有效租约且不是自己的 ⇒ 不抢。
            return Ok(LeaseAcquire::HeldByOther {
                holder_id: row.holder_id,
                expires_at: row.expires_at,
            });
        }
        // 有效且是自己的 ⇒ 续租（幂等重入）。
        if row.expires_at > now {
            let res = cron_leader_lease::Entity::update_many()
                .col_expr(
                    cron_leader_lease::Column::ExpiresAt,
                    sea_orm::sea_query::Expr::value(expires_at),
                )
                .col_expr(
                    cron_leader_lease::Column::HeartbeatAt,
                    sea_orm::sea_query::Expr::value(now),
                )
                .filter(cron_leader_lease::Column::LeaseName.eq(lease_name))
                .filter(cron_leader_lease::Column::HolderId.eq(holder_id))
                .exec(db)
                .await
                .map_err(|e| format!("续租 {} 失败: {e}", lease_name))?;
            return Ok(if res.rows_affected > 0 {
                LeaseAcquire::Acquired
            } else {
                // 极窄竞态：行在读与写之间被换人接管。
                LeaseAcquire::StaleTakenByOther { holder_id: holder_id.to_string() }
            });
        }
        // 已过期 ⇒ 接管（把 holder 换成自己并延长到期时间）。
        let res = cron_leader_lease::Entity::update_many()
            .col_expr(
                cron_leader_lease::Column::HolderId,
                sea_orm::sea_query::Expr::value(holder_id.to_string()),
            )
            .col_expr(
                cron_leader_lease::Column::ExpiresAt,
                sea_orm::sea_query::Expr::value(expires_at),
            )
            .col_expr(cron_leader_lease::Column::HeartbeatAt, sea_orm::sea_query::Expr::value(now))
            .col_expr(cron_leader_lease::Column::AcquiredAt, sea_orm::sea_query::Expr::value(now))
            .filter(cron_leader_lease::Column::LeaseName.eq(lease_name))
            .filter(cron_leader_lease::Column::ExpiresAt.lte(now))
            .exec(db)
            .await
            .map_err(|e| format!("接管过期租约 {} 失败: {e}", lease_name))?;
        return Ok(if res.rows_affected > 0 {
            LeaseAcquire::Acquired
        } else {
            // 并发：别人先接管了。
            LeaseAcquire::StaleTakenByOther { holder_id: holder_id.to_string() }
        });
    }

    // 2) 没有行 ⇒ 插入（主键冲突时对方已抢先 ⇒ 读回来判定）。
    let model = cron_leader_lease::ActiveModel {
        lease_name: Set(lease_name.to_string()),
        holder_id: Set(holder_id.to_string()),
        expires_at: Set(expires_at),
        heartbeat_at: Set(now),
        acquired_at: Set(now),
    };
    match model.insert(db).await {
        Ok(_) => Ok(LeaseAcquire::Acquired),
        Err(e) => {
            // 主键冲突 ⇒ 对方已抢先插入。读回来看它是否有效。
            let row = cron_leader_lease::Entity::find_by_id(lease_name.to_string())
                .one(db)
                .await
                .map_err(|e| format!("读回租约 {} 失败: {e}", lease_name))?;
            match row {
                Some(r) if r.expires_at > now => Ok(LeaseAcquire::HeldByOther {
                    holder_id: r.holder_id,
                    expires_at: r.expires_at,
                }),
                Some(r) => Ok(LeaseAcquire::StaleTakenByOther { holder_id: r.holder_id }),
                None => Err(format!("插入租约 {lease_name} 失败且读回不到行: {e}")),
            }
        },
    }
}

/// 续租（心跳）。返回是否仍持有（false = 已被别人接管 ⇒ 本进程应停止派发）。
pub async fn heartbeat(
    db: &DatabaseConnection,
    lease_name: &str,
    holder_id: &str,
    ttl: Duration,
) -> Result<bool, String> {
    let now = now_ms();
    let expires_at = now + ttl.as_millis() as i64;
    let res = cron_leader_lease::Entity::update_many()
        .col_expr(cron_leader_lease::Column::ExpiresAt, sea_orm::sea_query::Expr::value(expires_at))
        .col_expr(cron_leader_lease::Column::HeartbeatAt, sea_orm::sea_query::Expr::value(now))
        .filter(cron_leader_lease::Column::LeaseName.eq(lease_name))
        .filter(cron_leader_lease::Column::HolderId.eq(holder_id))
        .exec(db)
        .await
        .map_err(|e| format!("续租 {} 失败: {e}", lease_name))?;
    Ok(res.rows_affected > 0)
}

/// 释放租约（进程正常退出时调用；进程崩溃则靠过期自动失效）。
pub async fn release(
    db: &DatabaseConnection,
    lease_name: &str,
    holder_id: &str,
) -> Result<(), String> {
    cron_leader_lease::Entity::delete_many()
        .filter(cron_leader_lease::Column::LeaseName.eq(lease_name))
        .filter(cron_leader_lease::Column::HolderId.eq(holder_id))
        .exec(db)
        .await
        .map_err(|e| format!("释放租约 {} 失败: {e}", lease_name))?;
    Ok(())
}

/// 读当前租约（诊断用；过期租约也返回，由调用方判有效期）。
pub async fn current(
    db: &DatabaseConnection,
    lease_name: &str,
) -> Result<Option<LeaseInfo>, String> {
    let row = cron_leader_lease::Entity::find_by_id(lease_name.to_string())
        .one(db)
        .await
        .map_err(|e| format!("读租约 {} 失败: {e}", lease_name))?;
    Ok(row.map(|r| LeaseInfo {
        lease_name: r.lease_name,
        holder_id: r.holder_id,
        expires_at: r.expires_at,
        heartbeat_at: r.heartbeat_at,
        acquired_at: r.acquired_at,
    }))
}

/// 租约是否仍然有效（`expires_at > now`）。
pub fn is_valid(info: &LeaseInfo) -> bool {
    info.expires_at > now_ms()
}
