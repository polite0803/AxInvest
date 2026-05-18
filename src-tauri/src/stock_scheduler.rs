//! 股票定时分析调度器
//!
//! 每分钟检查一次 cron 表达式，触发到期的分析任务。
//! 同时检查价格告警条件，触发后通过 Tauri event 通知前端。

use axagent_astock_data::AStockClient;
use axagent_core::entity::{analysis_schedules, price_alerts};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
use std::sync::Arc;
use tauri::Emitter;
use tokio::time::{Duration, interval};

/// 股票分析调度器
#[allow(dead_code)]
pub struct StockScheduler {
    db: Arc<sea_orm::DatabaseConnection>,
    astock_client: Arc<AStockClient>,
    app_handle: tauri::AppHandle,
    running: Arc<tokio::sync::Mutex<std::collections::HashSet<String>>>,
}

#[allow(dead_code)]
impl StockScheduler {
    pub fn new(
        db: Arc<sea_orm::DatabaseConnection>,
        astock_client: Arc<AStockClient>,
        app_handle: tauri::AppHandle,
    ) -> Self {
        Self {
            db,
            astock_client,
            app_handle,
            running: Arc::new(tokio::sync::Mutex::new(std::collections::HashSet::new())),
        }
    }

    /// 启动调度循环（不含价格告警检查，如需告警请用 start_with_alerts）
    #[allow(dead_code)]
    pub async fn start(&self) {
        let mut ticker = interval(Duration::from_secs(60));
        loop {
            ticker.tick().await;
            if let Err(e) = self.check_and_run().await {
                tracing::warn!("StockScheduler check failed: {}", e);
            }
        }
    }

    /// 启动含价格告警检查的调度循环
    pub async fn start_with_alerts(&self) {
        let mut ticker = interval(Duration::from_secs(60));
        loop {
            ticker.tick().await;
            if let Err(e) = self.check_and_run().await {
                tracing::warn!("StockScheduler check failed: {}", e);
            }
            if let Err(e) = self
                .check_price_alerts(self.astock_client.as_ref(), &self.app_handle)
                .await
            {
                tracing::warn!("StockScheduler price alert check failed: {}", e);
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

                    // 并发保护：同一计划不重复触发
                    {
                        let mut running = self.running.lock().await;
                        if !running.insert(schedule.id.clone()) {
                            tracing::warn!("StockScheduler: 跳过重复触发 {}", schedule.id);
                            continue;
                        }
                    }
                    let schedule_id = schedule.id.clone();

                    // 触发股票分析
                    let stock_code = schedule.stock_code.clone();
                    let stock_name = schedule.stock_name.clone();
                    let provider_id = schedule.provider_id.clone();
                    let app = self.app_handle.clone();
                    let running_set = self.running.clone();

                    tokio::spawn(async move {
                        tracing::info!(
                            "StockScheduler: 开始执行分析 {} ({})",
                            stock_code,
                            stock_name
                        );
                        let date = chrono::Utc::now().format("%Y-%m-%d").to_string();
                        let result = crate::commands::stock_analysis::run_scheduled_analysis(
                            &app,
                            &stock_code,
                            &stock_name,
                            &date,
                            &provider_id,
                        )
                        .await;
                        // 完成后从 running 集合移除
                        {
                            {
                                let mut r = running_set.lock().await;
                                r.remove(&schedule_id);
                            }
                        }
                        if let Err(e) = result {
                            tracing::error!("StockScheduler: 分析失败 {}: {}", stock_code, e);
                        }
                    });
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

    /// 检查所有未触发的价格告警
    async fn check_price_alerts(
        &self,
        data_client: &AStockClient,
        app: &tauri::AppHandle,
    ) -> Result<(), String> {
        let alerts = price_alerts::Entity::find()
            .filter(price_alerts::Column::IsTriggered.eq(false))
            .all(self.db.as_ref())
            .await
            .map_err(|e| e.to_string())?;

        let now = chrono::Utc::now().timestamp_millis();

        for alert in alerts {
            if let Ok(quote) = data_client.get_quote(&alert.stock_code).await {
                let triggered = match alert.condition.as_str() {
                    "above" => quote.price >= alert.target_price,
                    "below" => quote.price <= alert.target_price,
                    _ => false,
                };

                if triggered {
                    // 更新告警为已触发
                    let _ = price_alerts::Entity::update_many()
                        .col_expr(
                            price_alerts::Column::IsTriggered,
                            sea_orm::sea_query::Expr::value(true),
                        )
                        .col_expr(
                            price_alerts::Column::TriggeredAt,
                            sea_orm::sea_query::Expr::value(Some(now)),
                        )
                        .col_expr(
                            price_alerts::Column::UpdatedAt,
                            sea_orm::sea_query::Expr::value(now),
                        )
                        .filter(price_alerts::Column::Id.eq(&alert.id))
                        .exec(self.db.as_ref())
                        .await
                        .map_err(|e| e.to_string())?;

                    // 向前端发送事件通知
                    let _ = app.emit(
                        "price-alert-triggered",
                        serde_json::json!({
                            "id": alert.id,
                            "stockCode": alert.stock_code,
                            "stockName": alert.stock_name,
                            "currentPrice": quote.price,
                            "targetPrice": alert.target_price,
                            "condition": alert.condition,
                        }),
                    );

                    tracing::info!(
                        "PriceAlert 触发: {} 价格 {} 已{} {}",
                        alert.stock_code,
                        quote.price,
                        if alert.condition == "above" {
                            "突破"
                        } else {
                            "跌破"
                        },
                        alert.target_price
                    );
                }
            }
        }
        Ok(())
    }
}

/// 检查日期是否匹配 cron 日/周/月字段
fn matches_day(date: chrono::NaiveDate, dom: &str, dow: &str) -> bool {
    let day = date.format("%d").to_string();
    let dom_ok = dom == "*" || dom.split(',').any(|p| p == day);
    let dow_ok = dow == "*"
        || dow.split(',').any(|p| {
            // chrono: Mon=1..Sun=7
            let chrono_dow = date.format("%u").to_string().parse::<u32>().unwrap_or(0);
            if p.contains('-') {
                let parts: Vec<&str> = p.split('-').collect();
                let start: u32 = parts[0].parse().unwrap_or(0);
                let end: u32 = parts.get(1).and_then(|s| s.parse().ok()).unwrap_or(0);
                chrono_dow >= start && chrono_dow <= end
            } else {
                let target: u32 = p.parse().unwrap_or(99);
                let target = if target == 0 { 7 } else { target };
                target == chrono_dow
            }
        });
    // Unix cron 语义：日和周都不为 * 时使用 OR，否则 AND（因为 * 恒真）
    if dom != "*" && dow != "*" {
        dom_ok || dow_ok
    } else {
        dom_ok && dow_ok
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

    let hour: u32 = parts[1].parse().ok()?;
    let minute: u32 = parts[0].parse().ok()?;
    let dom: &str = parts[2];
    let dow: &str = parts[4];

    let now = chrono::Utc::now();
    let today = now.date_naive();
    let today_target = today
        .and_hms_opt(hour, minute, 0)?
        .and_local_timezone(chrono::Utc)
        .single()?;

    // 检查今天是否满足日/周约束
    if matches_day(today, dom, dow) && today_target > now {
        return Some(today_target.timestamp_millis());
    }

    // 查找下一个满足约束的日期（最多查 366 天，覆盖跨月/跨年场景）
    for offset in 1..=366 {
        let candidate = today + chrono::Duration::days(offset);
        if matches_day(candidate, dom, dow) {
            let target = candidate
                .and_hms_opt(hour, minute, 0)?
                .and_local_timezone(chrono::Utc)
                .single()?;
            return Some(target.timestamp_millis());
        }
    }
    None // 不应该到达}
}
