// SPDX-License-Identifier: AGPL-3.0-only

//! P0: RealtimeMonitor → Tauri 前端 + DB 持久化 + 通知推送 + 插件总线 桥接器
//!
//! 实现 `MonitorEventEmitter` trait，把 monitor.rs 的告警分发到四条通道：
//!   1. Tauri `app.emit(事件名原文, ...)` — 前端实时弹窗，事件名由发射点给（权威枚举成员）
//!   2. 告警类（`IpcEventName::PriceAlertTriggered`）额外写 `price_alerts` 表
//!      `is_triggered=1` + `triggered_at=now` — 持久化触发历史
//!   3. 调用 `NotificationDispatcher::dispatch_alert` — 推送到 PushPlus/ServerChan/Ntfy/Gotify
//!   4. 插件总线 `event.dispatch` —— 价格告警与组合信号各补发一条 `DomainEvent`
//!      （kind 只引用 harness 常量，裁定 N6；派发 fire-and-forget，形态照
//!      `commands/stock_workflow/workflow_run_service.rs` 的 `dispatch_event`）；
//!      其余事件（T+0 重跑请求等）只转发，不落库
//!
//! ⚠ 通道 3 的现状：**分发器在生产里尚未装配任何渠道**（注册点与配置来源归里程碑 M-NOTIF），
//! 渠道数为 0 时本文件会显式 warn「通知不外推」，不把「没渠道」压成「没告警」。
//!
//! ⚠ 通道 4 的前置：总线 `Arc` 由装配点经 `get_capability_registry().get_event_dispatcher()`
//! 取回**同一个对象**（插件订阅者挂的就是它）。取不到时本桥照跑前三条，只在构造点 warn 一次
//! 「插件侧收不到 stock 域事件」—— 另造一个 bus 会「发射成功但插件永远收不到」。
//!
//! 设计要点：
//! - monitor.rs 的 `MonitorEventEmitter::emit(event, payload)` 是同步 trait，
//!   但 DB 写入和通知推送是异步操作。使用 `tokio::spawn` 在后台执行，避免阻塞 monitor 主循环。
//! - v203 后直接用 alert_type 匹配 price_alerts 表（6 类全部可持久化），
//!   替代旧的 6→2 降级映射 + 1% 容差匹配（change/volume 告警历史不再丢失）。
//! - emit 失败（如前端无监听器）不阻塞其他通道。

use axagent_harness::IpcEventName;
use std::sync::Arc;

use axagent_analysis_engine::monitor::MonitorEventEmitter;
use axagent_harness::AlertPayload;
use axagent_harness::AlertSeverity;
use axagent_harness::DispatchMode;
use axagent_harness::DomainEvent;
use axagent_harness::EventCategory;
use axagent_harness::EventDispatchBus;
use axagent_harness::stock_monitor::{STOCK_PORTFOLIO_SIGNAL_KIND, STOCK_PRICE_ALERT_KIND};
use chrono::Utc;
use sea_orm::ActiveModelTrait;
use sea_orm::ColumnTrait;
use sea_orm::DatabaseConnection;
use sea_orm::EntityTrait;
use sea_orm::QueryFilter;
use sea_orm::Set;
use serde_json::Value;
use tauri::Emitter;
use tracing::warn;

use axagent_entities::price_alerts;

/// 桥接 RealtimeMonitor 告警到 Tauri 前端 / DB / 通知推送 / 插件总线
pub struct TauriMonitorEmitter {
    app: tauri::AppHandle,
    db: DatabaseConnection,
    notification_dispatcher: Arc<axagent_notification::NotificationDispatcher>,
    /// `event.dispatch` 内置总线（与插件订阅者同一个 `Arc`）；取不到时第四跳跳过。
    plugin_event_bus: Option<Arc<EventDispatchBus>>,
}

impl TauriMonitorEmitter {
    pub fn new(
        app: tauri::AppHandle,
        db: DatabaseConnection,
        notification_dispatcher: Arc<axagent_notification::NotificationDispatcher>,
        plugin_event_bus: Option<Arc<EventDispatchBus>>,
    ) -> Self {
        Self { app, db, notification_dispatcher, plugin_event_bus }
    }
}

/// 第四跳的登记表：Tauri IPC 事件名 → 插件总线 `DomainEvent.kind`。
///
/// 只登记这两条：`StockMonitorT0RerunRequested` 是宿主内部的工作流重跑提示，契约的 stock 域
/// 只有「价格告警 / 组合信号 / 行情变化」三条 kind（行情变化那一条发射在
/// `commands/stock_workflow/quote_watcher_service.rs` 的 `dispatch_quote_change`，
/// 由 `init/services.rs` 的行情回调调用，不在本桥里）。
/// 比较与产出都用**权威串**：事件名取 `IpcEventName`，kind 取 harness 常量（裁定 N6）
/// ⇒ 本函数不出现任何字面量。
fn plugin_kind_of(event: &str) -> Option<&'static str> {
    if event == IpcEventName::PriceAlertTriggered.as_str() {
        Some(STOCK_PRICE_ALERT_KIND)
    } else if event == IpcEventName::PortfolioSignal.as_str() {
        Some(STOCK_PORTFOLIO_SIGNAL_KIND)
    } else {
        None
    }
}

/// 向插件总线补发一条领域事件（fire-and-forget）。
///
/// 无匹配订阅者时直接返回（`would_dispatch` 短路，不为空总线白 spawn）；
/// `DispatchMode::Emit` 内部是串行 await，**不能**在 `emit` 这个同步 trait 方法里等它 ——
/// 那会把插件 RTT 塞进 monitor 的轮询热路径。
fn dispatch_domain_event(bus: &Arc<EventDispatchBus>, kind: &'static str, payload: Value) {
    let mut event = DomainEvent::new(EventCategory::Stock, kind, payload, "stock.monitor");
    if !bus.would_dispatch(&event) {
        return;
    }
    let bus = Arc::clone(bus);
    tokio::spawn(async move {
        let outcome = bus.dispatch(&mut event, DispatchMode::Emit).await;
        tracing::trace!(
            invoked = outcome.invoked,
            rejected = outcome.rejected,
            "[monitor_emitter] stock 域事件已派发: kind={kind}"
        );
    });
}

impl MonitorEventEmitter for TauriMonitorEmitter {
    fn emit(&self, event: &str, payload: Value) {
        let app = self.app.clone();

        // 即时 emit 到前端（非阻塞，失败仅 warn）
        if let Err(e) = app.emit(event, payload.clone()) {
            warn!("[monitor_emitter] emit {} 失败: {e}", event);
        }

        // 第四跳：同一条告警/组合信号补发给插件订阅者（前端与插件各拿一份，互不替代）
        if let (Some(kind), Some(bus)) = (plugin_kind_of(event), self.plugin_event_bus.as_ref()) {
            dispatch_domain_event(bus, kind, payload.clone());
        }

        // 2026-10-14 收口：告警事件名由唯一权威 `IpcEventName::PriceAlertTriggered` 发起，
        // 发射点（analysis-engine 的 monitor）与监听点（PriceAlertPanel）用的是同一个成员，
        // 因此不再有「兼容别名」这一层 —— 两头各留一个名字正是因为中间没人做主。
        if event == IpcEventName::PriceAlertTriggered.as_str() {
            // 告警事件：异步写 DB + 推送通知
            let db = self.db.clone();
            let dispatcher = self.notification_dispatcher.clone();
            let payload_clone = payload.clone();
            tokio::spawn(async move {
                handle_alert_event(&db, &dispatcher, &payload_clone).await;
            });
        }
    }
}

/// 处理 `IpcEventName::PriceAlertTriggered` 告警：
/// 1. 按 payload 的 `stockCode` + `alertType` 更新 price_alerts 表的 is_triggered=1
/// 2. 调用 NotificationDispatcher 推送告警到已配置的渠道（未装配时如实 warn）
async fn handle_alert_event(
    db: &DatabaseConnection,
    dispatcher: &axagent_notification::NotificationDispatcher,
    payload: &Value,
) {
    let stock_code = payload.get("stockCode").and_then(|v| v.as_str()).unwrap_or("");
    let stock_name = payload.get("stockName").and_then(|v| v.as_str()).unwrap_or("");
    let alert_type = payload.get("alertType").and_then(|v| v.as_str()).unwrap_or("");
    let alert_msg = payload.get("alertMessage").and_then(|v| v.as_str()).unwrap_or("");
    let suggested = payload.get("suggestedAction").and_then(|v| v.as_str());

    // ── 1. 更新 price_alerts 表 ──
    // 有 alertId 就精确改这一行。此前按 (stock_code, alert_type) 匹配并把命中的**全部**行置 1，
    // 而现网同一只股票的同一类型有重复行（`(code, type)` 重复组合 16 个，如 601899 有 3 行 stop_loss）
    // ⇒ 一次触发会连带把没触发的兄弟行也标成已触发。
    let alert_id = payload.get("alertId").and_then(|v| v.as_str());
    let now_ms = chrono::Utc::now().timestamp_millis();
    match alert_id {
        Some(id) => mark_triggered_by_id(db, id, now_ms).await,
        None => {
            warn!(
                "[monitor_emitter] 告警 payload 缺 alertId ⇒ 退回按 (stock_code, alert_type) 匹配，\
                 同股同类型的重复行会被一次全部置为已触发: code={stock_code} type={alert_type}"
            );
            mark_triggered_by_code_and_type(db, stock_code, alert_type, now_ms).await;
        },
    }

    // ── 2. 推送通知 ──
    let severity = match alert_type {
        "stop_loss" | "support" => AlertSeverity::Critical,
        "take_profit" => AlertSeverity::Info,
        "resistance" => AlertSeverity::Info,
        "change" | "volume" => AlertSeverity::Warning,
        _ => AlertSeverity::Info,
    };

    let title = format!("告警 · {} ({})", stock_name, stock_code);
    let body = if let Some(s) = suggested {
        format!("{alert_msg}\n{s}")
    } else {
        alert_msg.to_string()
    };

    let alert_payload = AlertPayload {
        severity,
        title,
        body,
        stock_code: Some(stock_code.to_string()),
        generated_at: Utc::now(),
    };

    // 渠道未装配时不静默：如实记录「本条只走前端事件 + 落库，不外推」
    if dispatcher.list_channel_names().await.is_empty() {
        warn!(
            "[monitor_emitter] 通知渠道未装配（已注册 0 个）⇒ 告警不外推，仅前端事件 + price_alerts 落库"
        );
        return;
    }
    let summary = dispatcher.dispatch_alert(&alert_payload, Utc::now()).await;
    if summary.total == 0 || summary.failure_count > 0 {
        warn!(
            "[monitor_emitter] 告警外推未完成：渠道 {}、成功 {}、失败 {}、去重 {}、冷却 {}、级别过滤 {}",
            summary.total,
            summary.success_count,
            summary.failure_count,
            summary.deduped_count,
            summary.cooldown_skipped_count,
            summary.severity_filtered_count
        );
    } else {
        tracing::debug!("[monitor_emitter] 告警已外推：成功 {} 个渠道", summary.success_count);
    }
}

/// 精确把这一行 `price_alerts` 标为已触发（阶段 B 的正门）。
///
/// 抽出函数是为了让判据能拆在这一道守卫上：查不到该 id 时必须点名，
/// 不能把「行已被删」压成「回写成功」。
///
/// `pub(crate)`：命令层 `price_alert_upsert` 的判据要用它把一行置为终态
/// （验证「已触发行不吸收新单」），复用本函数而非另写一份 UPDATE —— 重复定义会分叉。
pub(crate) async fn mark_triggered_by_id(db: &DatabaseConnection, alert_id: &str, now_ms: i64) {
    match price_alerts::Entity::find_by_id(alert_id).one(db).await {
        Ok(Some(alert)) => {
            if alert.is_triggered != 0 {
                tracing::debug!(
                    "[monitor_emitter] 该告警已是触发态，跳过重复回写: id={}",
                    alert.id
                );
                return;
            }
            let mut active: price_alerts::ActiveModel = alert.into();
            active.is_triggered = Set(1);
            active.triggered_at = Set(Some(now_ms));
            active.updated_at = Set(now_ms);
            if let Err(e) = active.update(db).await {
                warn!("[monitor_emitter] 按 id 回写 price_alerts 失败: {e}");
            }
        },
        Ok(None) => {
            warn!("[monitor_emitter] 告警行不存在，回写落空（行被删过或 id 漂移）: id={alert_id}");
        },
        Err(e) => warn!("[monitor_emitter] 按 id 查询 price_alerts 失败: {e}"),
    }
}

/// 旧口径回退：只在 payload 缺 `alertId` 时走，命中一行还是全行由数据决定，**会扇出**。
async fn mark_triggered_by_code_and_type(
    db: &DatabaseConnection,
    stock_code: &str,
    alert_type: &str,
    now_ms: i64,
) {
    match price_alerts::Entity::find()
        .filter(price_alerts::Column::StockCode.eq(stock_code))
        .filter(price_alerts::Column::AlertType.eq(alert_type))
        .filter(price_alerts::Column::IsTriggered.eq(0))
        .all(db)
        .await
    {
        Ok(alerts) => {
            for alert in alerts {
                let mut active: price_alerts::ActiveModel = alert.into();
                active.is_triggered = Set(1);
                active.triggered_at = Set(Some(now_ms));
                active.updated_at = Set(now_ms);
                if let Err(e) = active.update(db).await {
                    warn!("[monitor_emitter] 回退口径更新 price_alerts 失败: {e}");
                }
            }
        },
        Err(e) => warn!("[monitor_emitter] 回退口径查询 price_alerts 失败: {e}"),
    }
}

#[cfg(test)]
mod tests {
    //! 阶段 B 的回写形态锁：触发状态必须**只**改这一行。
    //!
    //! 回归对象：旧回写按 `(stock_code, alert_type, is_triggered=0)` 命中并把匹配到的全部行置 1，
    //! 而现网同一只股票的同一类型有重复行（`(code, type)` 重复组合 16 个，如 601899 有 3 行
    //! stop_loss）⇒ 一次触发连带把没触发的兄弟行也标成已触发，「已触发」Tab 从此读不出是哪条。
    //! 测试走 `create_test_pool`（临时 SQLite 文件库，与生产同一 `initialize_schema`）。

    use super::*;
    use axagent_dao::db::create_test_pool;

    fn row(id: &str, code: &str, alert_type: &str, threshold: f64) -> price_alerts::ActiveModel {
        let now = chrono::Utc::now().timestamp_millis();
        price_alerts::ActiveModel {
            id: Set(id.to_string()),
            stock_code: Set(code.to_string()),
            stock_name: Set("测试股".to_string()),
            condition: Set(if alert_type == "stop_loss" {
                "below"
            } else {
                "above"
            }
            .to_string()),
            target_price: Set(threshold),
            alert_type: Set(Some(alert_type.to_string())),
            condition_type: Set(Some("price".to_string())),
            threshold: Set(Some(threshold)),
            is_triggered: Set(0),
            triggered_at: Set(None),
            created_at: Set(now),
            updated_at: Set(now),
        }
    }

    async fn insert_two_same_code_same_type(db: &DatabaseConnection) -> (String, String) {
        let id_a = "11111111-1111-4111-8111-111111111111".to_string();
        let id_b = "22222222-2222-4222-8222-222222222222".to_string();
        // 同 (stock_code, alert_type) 两行 —— 就是现网那副形态（601899 有 3 行 stop_loss）
        for (id, threshold) in [(id_a.clone(), 13.0), (id_b.clone(), 14.0)] {
            row(&id, "601899", "stop_loss", threshold)
                .insert(db)
                .await
                .expect("插入测试告警行应成功");
        }
        (id_a, id_b)
    }

    #[tokio::test]
    async fn mark_triggered_by_id_only_touches_that_row() {
        let handle = create_test_pool().await.expect("测试：创建连接池应成功");
        let db = &handle.conn;
        let (id_a, id_b) = insert_two_same_code_same_type(db).await;

        let stamp = 1_700_000_000_000_i64;
        mark_triggered_by_id(db, &id_a, stamp).await;

        let a = price_alerts::Entity::find_by_id(&id_a).one(db).await.unwrap().expect("a 行应在");
        let b = price_alerts::Entity::find_by_id(&id_b).one(db).await.unwrap().expect("b 行应在");
        assert_eq!(a.is_triggered, 1, "被触发的这一行必须置 1");
        assert_eq!(a.triggered_at, Some(stamp), "触发时刻要落到这一行");
        assert_eq!(b.is_triggered, 0, "同股同类型的兄弟行不得被连带改写");
        assert_eq!(b.triggered_at, None, "同上");
    }

    /// 这条钉的是**事实**不是期望：payload 缺 `alertId` 时走的回退口径会一次扇出全部匹配行。
    /// 保留它是因为回退路径还在代码里；哪天回退被撤，这条要跟着撤而不是改成常绿。
    #[tokio::test]
    async fn fallback_path_without_alert_id_still_marks_every_matching_row() {
        let handle = create_test_pool().await.expect("测试：创建连接池应成功");
        let db = &handle.conn;
        insert_two_same_code_same_type(db).await;

        mark_triggered_by_code_and_type(db, "601899", "stop_loss", 1_700_000_000_000).await;

        let triggered = price_alerts::Entity::find()
            .filter(price_alerts::Column::IsTriggered.eq(1))
            .all(db)
            .await
            .unwrap();
        assert_eq!(
            triggered.len(),
            2,
            "回退口径的扇出行为必须被记录在案（这正是 id 路径要存在的原因）"
        );
    }

    #[tokio::test]
    async fn mark_triggered_by_id_on_missing_row_changes_nothing_and_does_not_panic() {
        let handle = create_test_pool().await.expect("测试：创建连接池应成功");
        let db = &handle.conn;
        let (_, id_b) = insert_two_same_code_same_type(db).await;

        mark_triggered_by_id(db, "00000000-0000-4000-8000-000000000000", 1_700_000_000_000).await;

        assert_eq!(
            price_alerts::Entity::find_by_id(&id_b).one(db).await.unwrap().unwrap().is_triggered,
            0,
            "查不到目标 id 时不得顺手改到别的行"
        );
    }

    /// 第四跳的登记表：两条要过线、T+0 重跑提示不要过线。
    ///
    /// 这条钉的是「桥接面」而不是总线本身：漏登记 = 插件静默收不到（正是本仓反复出现的
    /// 「写了但没人装配」形态），多登记 = 把宿主内部提示卖给插件。
    #[test]
    fn only_alert_and_portfolio_events_cross_into_the_plugin_bus() {
        assert_eq!(
            plugin_kind_of(IpcEventName::PriceAlertTriggered.as_str()),
            Some(STOCK_PRICE_ALERT_KIND)
        );
        assert_eq!(
            plugin_kind_of(IpcEventName::PortfolioSignal.as_str()),
            Some(STOCK_PORTFOLIO_SIGNAL_KIND)
        );
        // 未登记的（T+0 重跑请求、行情推送）一律 None ⇒ 不冒充 stock 域事件
        assert_eq!(plugin_kind_of(IpcEventName::StockMonitorT0RerunRequested.as_str()), None);
        assert_eq!(plugin_kind_of(IpcEventName::StockQuoteUpdate.as_str()), None);
        assert_eq!(plugin_kind_of(""), None);

        // kind 前缀必须等于 `EventCategory::Stock`（裁定 N6 的「不发散新前缀」在这一格）
        let prefix = format!("{}/", EventCategory::Stock.as_str());
        for kind in [STOCK_PRICE_ALERT_KIND, STOCK_PORTFOLIO_SIGNAL_KIND] {
            assert!(kind.starts_with(&prefix), "kind {kind} 的前缀不是 {prefix}");
        }
    }
}
