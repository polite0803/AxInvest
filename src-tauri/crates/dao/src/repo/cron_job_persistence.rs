// SPDX-License-Identifier: AGPL-3.0-only

//! `CronJobPersistence` 端口的 **PostgreSQL / SQLite 双方言实现**。
//!
//! ## 来源（2026-09-18 从 `runtime-core/src/cron_job.rs` 下沉而来）
//!
//! 这段代码原来住在 `axagent-runtime-core` 里，直接 `use axagent_entities::{cron_job, cron_job_history}`。
//! 而分层规则要求 consumer crate 只依赖 `axagent-harness` ⇒ 被 `check:contracts` 的 [D] 项判越界。
//! 现在 DB 访问归位到 DAO（本仓唯一允许碰表实体的层），runtime-core 只持有 trait 对象。
//!
//! ## 为什么建表也在这里，而不是交给版本化迁移
//!
//! 本仓的建表来源已切换到声明式引擎，但 `cron_jobs` / `cron_job_history` 这两张表是
//! **运行时自举**的（`CronJobStore` 在应用启动时确保存在）。保持原行为不变是本次下沉的
//! 硬约束 —— 下沉只改「代码住在哪个 crate」，不改「什么时候建表」。
//!
//! ## 两条被刻意保留下来的既有修复（下沉时逐字保留，不再丢失）
//!
//! 1. **建表语句由实体生成**（`Schema::create_table_from_entity`），不手写列清单 ——
//!    手写列清单与实体一旦漂移不会报错，只会表现为「写入成功但读出的字段是 None」。
//! 2. **`cron_job_history` 的三个时间戳列必须是 BIGINT**：`started_at` 存 epoch 毫秒
//!    （约 1.8e12），PG 的 `INTEGER` 是 int4（上限 2.1e9）⇒ 插入必然
//!    `value out of range for type integer`；而旧库里的表是**旧版 `INTEGER` 建表语句**
//!    建的，`CREATE TABLE IF NOT EXISTS` **不会**改列类型 ⇒ 必须显式迁移，
//!    否则第一次 `record_run` 就报错。SQLite 的 INTEGER 本就是 64 位，跳过。

use async_trait::async_trait;
use axagent_entities::{cron_job, cron_job_history};
use axagent_harness::cron_persistence::{CronHistoryRecord, CronJobPersistence};
use sea_orm::sea_query::OnConflict;
use sea_orm::{
    ColumnTrait, ConnectionTrait, DatabaseConnection, DbBackend, EntityTrait, QueryFilter,
    QueryOrder, QuerySelect, Schema, Set,
};
use std::sync::Arc;
use tracing::error;

pub struct PgCronJobPersistence {
    db: Arc<DatabaseConnection>,
}

impl PgCronJobPersistence {
    pub fn new(db: DatabaseConnection) -> Self {
        Self { db: Arc::new(db) }
    }

    /// 把 `cron_job_history` 的三个时间戳列从 `integer` 迁到 `bigint`（幂等，仅 PG 需要）。
    async fn heal_history_timestamps(&self) {
        if self.db.get_database_backend() != DbBackend::Postgres {
            // SQLite 无静态列类型约束，无需迁移。
            return;
        }
        for col in ["started_at", "completed_at", "duration_ms"] {
            let sql = format!(
                "DO $$ BEGIN \
                   IF EXISTS (SELECT 1 FROM information_schema.columns \
                              WHERE table_schema = 'public' AND table_name = 'cron_job_history' \
                                AND column_name = '{col}' AND data_type = 'integer') THEN \
                     ALTER TABLE cron_job_history ALTER COLUMN {col} TYPE BIGINT; \
                   END IF; \
                 END $$;"
            );
            if let Err(e) = self.db.execute_unprepared(&sql).await {
                error!("[CronJobPersistence] 迁移 cron_job_history.{col} 至 BIGINT 失败: {e}");
            }
        }
    }
}

#[async_trait]
impl CronJobPersistence for PgCronJobPersistence {
    async fn ensure_tables(&self) {
        let mut stmt =
            Schema::new(self.db.get_database_backend()).create_table_from_entity(cron_job::Entity);
        stmt.if_not_exists();
        if let Err(e) = self.db.execute(&stmt).await {
            error!("[CronJobPersistence] 创建 cron_jobs 表失败: {e}");
        }

        let mut stmt = Schema::new(self.db.get_database_backend())
            .create_table_from_entity(cron_job_history::Entity);
        stmt.if_not_exists();
        if let Err(e) = self.db.execute(&stmt).await {
            error!("[CronJobPersistence] 创建 cron_job_history 表失败: {e}");
        }
        self.heal_history_timestamps().await;
    }

    async fn load_jobs(&self) -> Result<Vec<String>, String> {
        let rows = cron_job::Entity::find().all(&*self.db).await.map_err(|e| e.to_string())?;
        Ok(rows.into_iter().map(|m| m.data).collect())
    }

    async fn upsert_job(&self, id: &str, data: &str) -> Result<(), String> {
        // 原实现按 backend 分支：SQLite 用 `INSERT OR REPLACE`、PG 用
        // `ON CONFLICT (id) DO UPDATE`。改用 `OnConflict` 后由 sea-orm 按 backend 生成，
        // 两方言共用一份代码。本表只有 `id` / `data` 两列且 upsert 时都显式赋值，
        // 故 `INSERT OR REPLACE` 的「先删后插」语义与此写法等价。
        let model = cron_job::ActiveModel { id: Set(id.to_string()), data: Set(data.to_string()) };
        cron_job::Entity::insert(model)
            .on_conflict(
                OnConflict::column(cron_job::Column::Id)
                    .update_column(cron_job::Column::Data)
                    .to_owned(),
            )
            .exec(&*self.db)
            .await
            .map(|_| ())
            .map_err(|e| e.to_string())
    }

    async fn delete_job(&self, id: &str) -> Result<(), String> {
        cron_job::Entity::delete_by_id(id.to_string())
            .exec(&*self.db)
            .await
            .map(|_| ())
            .map_err(|e| e.to_string())
    }

    async fn insert_history(&self, record: CronHistoryRecord) -> Result<(), String> {
        let model = cron_job_history::ActiveModel {
            id: Set(record.id),
            task_id: Set(record.task_id),
            started_at: Set(record.started_at),
            completed_at: Set(record.completed_at),
            success: Set(record.success as i32),
            output: Set(record.output),
            error: Set(record.error),
            duration_ms: Set(record.duration_ms),
            partial: Set(record.partial as i32),
            trigger: Set(record.trigger),
        };
        cron_job_history::Entity::insert(model)
            .exec(&*self.db)
            .await
            .map(|_| ())
            .map_err(|e| e.to_string())
    }

    async fn load_history(
        &self,
        task_id: &str,
        limit: i64,
    ) -> Result<Vec<CronHistoryRecord>, String> {
        let rows = cron_job_history::Entity::find()
            .filter(cron_job_history::Column::TaskId.eq(task_id))
            .order_by_desc(cron_job_history::Column::StartedAt)
            .limit(limit as u64)
            .all(&*self.db)
            .await
            .map_err(|e| e.to_string())?;
        Ok(rows
            .into_iter()
            .map(|m| CronHistoryRecord {
                id: m.id,
                task_id: m.task_id,
                started_at: m.started_at,
                completed_at: m.completed_at,
                success: m.success != 0,
                output: m.output,
                error: m.error,
                duration_ms: m.duration_ms,
                partial: m.partial != 0,
                trigger: m.trigger,
            })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn setup() -> PgCronJobPersistence {
        let handle = crate::db::create_test_pool().await.expect("测试库应可建立");
        let p = PgCronJobPersistence::new(handle.conn.clone());
        // `cron_job_history` 由本端口自举（见文件头「为什么建表也在这里」），
        // 而 `partial` 列是实体新增 —— 建表链与 `heal_all` 补列链都要能给出它，
        // 否则这里第一次 insert 就报「列不存在」。
        p.ensure_tables().await;
        p
    }

    fn record(task_id: &str, id: &str, partial: bool, success: bool) -> CronHistoryRecord {
        CronHistoryRecord {
            id: id.to_string(),
            task_id: task_id.to_string(),
            started_at: 1_792_400_000_000,
            completed_at: Some(1_792_400_060_000),
            success: success as i32 != 0,
            output: Some("候选池扫描完成".to_string()),
            error: None,
            duration_ms: 60_000,
            partial,
            trigger: Some("schedule".to_string()),
        }
    }

    /// 「部分完成」必须逐列存活：`success=false` + `partial=true` 落库后回读仍是这组值。
    ///
    /// 这条断言存在的意义是区分两种红 —— 只看 `success` 时，墙钟收尾的轮次与真故障
    /// 在历史表里完全同形，按历史统计的成功率会被系统性压低（池扫描实测 7 轮 3 条 0，
    /// 其中 2 条其实是部分完成）。
    #[tokio::test]
    async fn partial_survives_history_round_trip() {
        let p = setup().await;
        let task = format!("t-partial-{}", uuid::Uuid::new_v4());
        p.insert_history(record(&task, "r1", true, false)).await.expect("部分完成的历史行应可落库");

        let rows = p.load_history(&task, 10).await.expect("读历史应成功");
        assert_eq!(rows.len(), 1, "应恰好读回一行");
        assert!(!rows[0].success, "超墙钟收尾那轮 success 记 false（本轮没跑完规划量）");
        assert!(rows[0].partial, "partial 必须为真 —— 否则该行退化成一枚无法归因的失败");
        assert_eq!(
            rows[0].trigger.as_deref(),
            Some("schedule"),
            "触发来源必须逐列存活（旧行为 NULL 时界面显示未知）"
        );
    }

    /// 负控侧：真故障轮不得被写成「部分完成」。拆掉写侧的 `partial` 赋值时，
    /// 本条与上条不会同时红 —— 上条红在「该真的没真」，本条红在「该假的没假」。
    #[tokio::test]
    async fn hard_failure_is_not_marked_partial() {
        let p = setup().await;
        let task = format!("t-fail-{}", uuid::Uuid::new_v4());
        p.insert_history(record(&task, "r1", false, false)).await.expect("失败历史行应可落库");

        let rows = p.load_history(&task, 10).await.expect("读历史应成功");
        assert_eq!(rows.len(), 1);
        assert!(!rows[0].success);
        assert!(!rows[0].partial, "真故障不得带部分完成标记（否则界面上会被洗成黄色）");
    }
}
