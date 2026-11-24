// SPDX-License-Identifier: AGPL-3.0-only

//! `bot_department_quotas` 表的仓库层（docs/bot-integration/P4/MODEL-P4 §2）。
//!
//! 一个部门至多一行配额（`UNIQUE(department_id)`）；没有行 = 该部门走 P3
//! 默认值（ADR-P4-1）。本层只做数据访问，取值范围的语义校验在命令层
//! （`BOT_QUOTA_INVALID`），数据库 CHECK 是第二道防线。

use sea_orm::*;

use axagent_entities::bot_department_quotas;
use axagent_harness::core_error::{AxAgentError, Result};
use axagent_harness::types::{BotDepartmentQuota, BotDepartmentQuotaInput};
use axagent_harness::util_fns::{gen_id, now_ts};

fn quota_from_entity(m: bot_department_quotas::Model) -> BotDepartmentQuota {
    BotDepartmentQuota {
        department_id: m.department_id,
        max_concurrency: m.max_concurrency,
        total_timeout_secs: m.total_timeout_secs,
        max_steps: m.max_steps,
        max_in_flight_per_depth: m.max_in_flight_per_depth,
        step_timeout_secs: m.step_timeout_secs,
        max_concurrent_runs: m.max_concurrent_runs,
        is_default: false,
        updated_at: Some(m.updated_at),
    }
}

/// 读某部门的配额行。`None` = 该部门没有配额（命令层按
/// [`BotDepartmentQuota::defaults`] 合成 P3 默认返回）。
pub async fn get_by_department(
    db: &DatabaseConnection,
    department_id: &str,
) -> Result<Option<BotDepartmentQuota>> {
    let row = bot_department_quotas::Entity::find()
        .filter(bot_department_quotas::Column::DepartmentId.eq(department_id))
        .one(db)
        .await?;
    Ok(row.map(quota_from_entity))
}

/// 全部配额行（运营面板/诊断用），按 `department_id` 升序稳定排序。
pub async fn list_all(db: &DatabaseConnection) -> Result<Vec<BotDepartmentQuota>> {
    let rows = bot_department_quotas::Entity::find()
        .order_by_asc(bot_department_quotas::Column::DepartmentId)
        .all(db)
        .await?;
    Ok(rows.into_iter().map(quota_from_entity).collect())
}

/// 全量 upsert（IPC-P4 §1 `set_bot_department_quota`）：有行就地更新、无行插入，
/// 返回回读行（`is_default = false`）。
pub async fn upsert(
    db: &DatabaseConnection,
    input: &BotDepartmentQuotaInput,
) -> Result<BotDepartmentQuota> {
    let now = now_ts();
    let existing = bot_department_quotas::Entity::find()
        .filter(bot_department_quotas::Column::DepartmentId.eq(&input.department_id))
        .one(db)
        .await?;
    match existing {
        Some(row) => {
            let mut am: bot_department_quotas::ActiveModel = row.into();
            am.max_concurrency = Set(input.max_concurrency);
            am.total_timeout_secs = Set(input.total_timeout_secs);
            am.max_steps = Set(input.max_steps);
            am.max_in_flight_per_depth = Set(input.max_in_flight_per_depth);
            am.step_timeout_secs = Set(input.step_timeout_secs);
            am.max_concurrent_runs = Set(input.max_concurrent_runs);
            am.updated_at = Set(now);
            am.update(db).await.map_err(unique_conflict)?;
        },
        None => {
            let am = bot_department_quotas::ActiveModel {
                id: Set(gen_id()),
                department_id: Set(input.department_id.clone()),
                max_concurrency: Set(input.max_concurrency),
                total_timeout_secs: Set(input.total_timeout_secs),
                max_steps: Set(input.max_steps),
                max_in_flight_per_depth: Set(input.max_in_flight_per_depth),
                step_timeout_secs: Set(input.step_timeout_secs),
                max_concurrent_runs: Set(input.max_concurrent_runs),
                created_at: Set(now),
                updated_at: Set(now),
            };
            am.insert(db).await.map_err(unique_conflict)?;
        },
    }
    get_by_department(db, &input.department_id).await?.ok_or_else(|| {
        AxAgentError::NotFound(format!("BotDepartmentQuota {}", input.department_id))
    })
}

/// 删配额行（恢复 P3 默认）。返回删除行数；无行时 0（幂等，IPC-P4 §1）。
pub async fn delete_by_department(db: &DatabaseConnection, department_id: &str) -> Result<u64> {
    let res = bot_department_quotas::Entity::delete_many()
        .filter(bot_department_quotas::Column::DepartmentId.eq(department_id))
        .exec(db)
        .await?;
    Ok(res.rows_affected)
}

/// 唯一约束冲突（SQLite 1 与 PG 23505）→ Validation。
fn unique_conflict(err: DbErr) -> AxAgentError {
    let msg = err.to_string();
    if msg.contains("UNIQUE constraint failed") || msg.contains("duplicate key") {
        AxAgentError::Validation("department already has a quota row".to_string())
    } else {
        AxAgentError::Database(err)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn pool() -> crate::db::DbHandle {
        crate::db::create_test_pool().await.expect("test pool")
    }

    fn input(department_id: &str) -> BotDepartmentQuotaInput {
        BotDepartmentQuotaInput {
            department_id: department_id.to_string(),
            max_concurrency: 3,
            total_timeout_secs: 600,
            max_steps: 6,
            max_in_flight_per_depth: 1,
            step_timeout_secs: 120,
            max_concurrent_runs: 4,
        }
    }

    /// 默认路径逐项等于 P3 常量（ADR-P4-1：无行 = P3 行为）。
    #[test]
    fn default_quota_equals_p3_constants() {
        let d = BotDepartmentQuota::defaults("dept-1");
        assert_eq!(d.department_id, "dept-1");
        assert_eq!(d.max_concurrency, 2);
        assert_eq!(d.total_timeout_secs, 900);
        assert_eq!(d.max_steps, 12);
        assert_eq!(d.max_in_flight_per_depth, 2);
        assert_eq!(d.step_timeout_secs, 300);
        assert_eq!(d.max_concurrent_runs, 2);
        assert!(d.is_default, "no row must be is_default=true");
        assert_eq!(d.updated_at, None);
    }

    #[tokio::test]
    async fn upsert_roundtrip_and_update_in_place() {
        let h = pool().await;
        let db = &h.conn;

        assert_eq!(get_by_department(db, "dept-1").await.expect("read"), None);

        let created = upsert(db, &input("dept-1")).await.expect("create");
        assert!(!created.is_default);
        assert_eq!(created.max_concurrency, 3);
        assert!(created.updated_at.is_some());
        let listed = list_all(db).await.expect("list");
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].department_id, "dept-1");

        let mut patch = input("dept-1");
        patch.max_steps = 5;
        let updated = upsert(db, &patch).await.expect("update");
        assert_eq!(updated.max_steps, 5);
        assert_eq!(list_all(db).await.expect("list").len(), 1, "upsert must not add rows");
    }

    /// 两部门配额互不影响（A1 单测口径）。
    #[tokio::test]
    async fn departments_are_independent() {
        let h = pool().await;
        let db = &h.conn;

        let mut a = input("dept-a");
        a.max_concurrency = 4;
        a.max_concurrent_runs = 8;
        upsert(db, &a).await.expect("dept a");
        upsert(db, &input("dept-b")).await.expect("dept b");

        let ra = get_by_department(db, "dept-a").await.expect("read a").expect("a row");
        let rb = get_by_department(db, "dept-b").await.expect("read b").expect("b row");
        assert_eq!(ra.max_concurrency, 4);
        assert_eq!(ra.max_concurrent_runs, 8);
        assert_eq!(rb.max_concurrency, 3, "b unaffected by a");
        assert_eq!(rb.max_concurrent_runs, 4);

        // 删 A 的配额不影响 B。
        assert_eq!(delete_by_department(db, "dept-a").await.expect("delete a"), 1);
        assert_eq!(get_by_department(db, "dept-a").await.expect("read a"), None);
        assert!(get_by_department(db, "dept-b").await.expect("read b").is_some());
    }

    #[tokio::test]
    async fn delete_is_idempotent() {
        let h = pool().await;
        let db = &h.conn;
        assert_eq!(delete_by_department(db, "missing").await.expect("no row"), 0);
    }

    /// 数据库 CHECK 是第二道防线：越界值即使绕过命令层也写不进去。
    #[tokio::test]
    async fn out_of_range_rejected_by_check() {
        let h = pool().await;
        let db = &h.conn;
        let mut bad = input("dept-x");
        bad.max_steps = 13;
        let err = upsert(db, &bad).await.expect_err("max_steps 13 must be rejected");
        assert!(
            matches!(err, AxAgentError::Database(_)),
            "CHECK failure should be Database error: {err:?}"
        );
    }
}
