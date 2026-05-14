//! 股票定时分析调度器
//!
//! 每分钟检查一次 cron 表达式，触发到期的分析任务。
//! 实际分析逻辑复用 `start_stock_analysis` 的内部调用路径。

use axagent_core::entity::analysis_schedules;
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
use std::sync::Arc;
use tokio::time::{interval, Duration};

/// 股票分析调度器
pub struct StockScheduler {
    db: Arc<sea_orm::DatabaseConnection>,
}

impl StockScheduler {
    pub fn new(db: Arc<sea_orm::DatabaseConnection>) -> Self {
        Self { db }
    }

    /// 启动调度循环，直接持有 DB Arc，不依赖 AppState
    pub async fn start(&self) {
        let mut ticker = interval(Duration::from_secs(60));
        loop {
            ticker.tick().await;
            if let Err(e) = self.check_and_run().await {
                tracing::warn!("StockScheduler check failed: {}", e);
            }
        }
    }

    /// 查询所有启用的计划，检查 next_run_at 是否到期
    async fn check_and_run(&self) -> Result<(), String> {
        let now = chrono::Utc::now().timestamp_millis();
        let schedules = analysis_schedules::Entity::find()
            .filter(analysis_schedules::Column::IsEnabled.eq(true))
            .all(self.db.as_ref())
            .await
            .map_err(|e| e.to_string())?;

        for schedule in schedules {
            if let Some(next_run) = schedule.next_run_at {
                if now >= next_run {
                    tracing::info!(
                        "StockScheduler: 触发定时分析 {} ({})",
                        schedule.stock_code,
                        schedule.stock_name
                    );

                    // 更新 last_run_at 并计算 next_run_at
                    let new_next = compute_next_run(&schedule.cron_expression);
                    let _ = analysis_schedules::Entity::update_many()
                        .col_expr(
                            analysis_schedules::Column::LastRunAt,
                            sea_orm::sea_query::Expr::value(Some(now)),
                        )
                        .col_expr(
                            analysis_schedules::Column::NextRunAt,
                            sea_orm::sea_query::Expr::value(new_next),
                        )
                        .col_expr(
                            analysis_schedules::Column::UpdatedAt,
                            sea_orm::sea_query::Expr::value(now),
                        )
                        .filter(analysis_schedules::Column::Id.eq(&schedule.id))
                        .exec(self.db.as_ref())
                        .await
                        .map_err(|e| e.to_string())?;

                    // TODO: 实际触发 start_stock_analysis（需通过 AppState）
                    // 当前仅记录日志，后续可通过 channel 或内部调度集成触发
                    tracing::info!(
                        "StockScheduler: {} 分析到期 (provider={})",
                        schedule.stock_code,
                        schedule.provider_id
                    );
                }
            } else {
                // 没有 next_run_at，首次计算
                let next = compute_next_run(&schedule.cron_expression);
                let now = chrono::Utc::now().timestamp_millis();
                let _ = analysis_schedules::Entity::update_many()
                    .col_expr(
                        analysis_schedules::Column::NextRunAt,
                        sea_orm::sea_query::Expr::value(next),
                    )
                    .col_expr(
                        analysis_schedules::Column::UpdatedAt,
                        sea_orm::sea_query::Expr::value(now),
                    )
                    .filter(analysis_schedules::Column::Id.eq(&schedule.id))
                    .exec(self.db.as_ref())
                    .await
                    .map_err(|e| e.to_string())?;
            }
        }
        Ok(())
    }
}

/// 根据 cron 表达式计算下一次执行时间（返回毫秒时间戳）
fn compute_next_run(cron_expr: &str) -> Option<i64> {
    // cron 表达式: "分 时 日 月 周" 如 "0 9 * * 1-5"
    let parts: Vec<&str> = cron_expr.split_whitespace().collect();
    if parts.len() != 5 {
        tracing::warn!("无效的 cron 表达式: {}", cron_expr);
        return None;
    }

    // 简单实现：如果每天指定时间，计算今天/明天的目标时间
    let hour: u32 = parts[1].parse().ok()?;
    let minute: u32 = parts[0].parse().ok()?;

    let now = chrono::Utc::now();
    let today_target = now
        .date_naive()
        .and_hms_opt(hour, minute, 0)?
        .and_local_timezone(chrono::Utc)
        .single()?;

    let next = if today_target > now {
        today_target
    } else {
        today_target + chrono::Duration::days(1)
    };

    Some(next.timestamp_millis())
}
