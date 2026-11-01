// SPDX-License-Identifier: AGPL-3.0-only

//! 条件单桥接层 —— `conditional_orders` 表 ↔ 内存引擎 ↔ 行情回调
//!
//! ## 三层各自的方向
//!
//! ```text
//! conditional_orders 表（定义唯一权威）
//!   → ConditionalOrderBridge::refresh()          全量读回内存引擎
//!   → RealTimeQuoteWatcher 行情回调（wiring 层）
//!     → handle_quote_event() → 引擎判定
//!       → 触发：计数写回表 + 非「仅通知」动作生成交易意图 + 产出 ConditionTrigger
//! ```
//!
//! ## 这一版换掉的三件事（都是实证过的缺陷，不是风格问题）
//!
//! 1. **定义不再从 `stock_analyses` 的推理文本里嗅探重建**。旧 `load_from_db` 读的是
//!    「已触发的条件单意图行」，用中文关键词猜类别、阈值硬编码成 3%/10%、单日上限硬编码 3、
//!    冷却硬编码 30 分 —— 用户在创建时给的数全部丢失，且触发写下的意图行会在下一趟热加载时
//!    **变成新的定义**（结果产生规则，语义倒置）。
//! 2. **旧 `load_from_db` 用 `*orders = orders_map` 整体覆盖内存**，于是 `create_conditional_order`
//!    只往内存塞、从不落库的一条规则，在任何一次 list（它自己也调 load_from_db）之后就被清空 ⇒
//!    「创建」实际是只返回一个 id 的 no-op。现在写侧统一走 [`ConditionalOrderBridge::put`]。
//! 3. **计数加在引擎那一份上**（`engine.record_trigger`）再写回表，不再回写调用方 clone 的 map ⇒
//!    冷却与单日上限真正参与下一轮判定（旧形态里引擎那份恒 0）。

use crate::conditional_order::{
    ConditionTrigger, ConditionType, ConditionalOrder, ConditionalOrderEngine, OrderAction,
    QuoteSignal, TriggerDecision, change_pct,
};
use crate::trade_intent::TradeIntentService;
use axagent_astock_data::{calendar, realtime_quote::QuoteChangeEvent};
use axagent_entities::conditional_orders;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, QueryOrder, Set,
};
use std::sync::Arc;

/// 读表结果。`rejected` 非空必须被点名 —— 把读不回来的行静默丢掉会被读成「没有规则」。
#[derive(Debug, Clone, Default)]
pub struct LoadReport {
    pub loaded: usize,
    /// `(行 id, 无法读回的原因)`
    pub rejected: Vec<(String, String)>,
}

/// 条件单执行桥接器（无实例状态；状态在一张 static 引擎上）
pub struct ConditionalOrderBridge;

impl ConditionalOrderBridge {
    fn engine() -> &'static Arc<tokio::sync::RwLock<ConditionalOrderEngine>> {
        static ENGINE: std::sync::OnceLock<Arc<tokio::sync::RwLock<ConditionalOrderEngine>>> =
            std::sync::OnceLock::new();
        ENGINE.get_or_init(|| Arc::new(tokio::sync::RwLock::new(ConditionalOrderEngine::new())))
    }

    /// 全量定义（含已停用）+ 读不回来的行 —— 列表命令与引擎共用这一份映射，
    /// 两处各写一套「行 → 规则」是漂移的起点。
    pub async fn definitions(
        db: &DatabaseConnection,
    ) -> Result<(Vec<ConditionalOrder>, Vec<(String, String)>), String> {
        let rows = conditional_orders::Entity::find()
            .order_by_desc(conditional_orders::Column::UpdatedAt)
            .all(db)
            .await
            .map_err(|e| format!("查询条件单失败: {e}"))?;
        Ok(partition(rows))
    }

    /// 表 → 内存引擎（全量替换）。启动时一次，建单/改单/启停后各一次；
    /// 不再有 5 分钟定时器去覆盖它 —— 那条热加载正是把新建规则清空的那只手。
    pub async fn refresh(db: &DatabaseConnection) -> Result<LoadReport, String> {
        let rows = conditional_orders::Entity::find()
            .filter(conditional_orders::Column::Enabled.eq(true))
            .order_by_desc(conditional_orders::Column::UpdatedAt)
            .all(db)
            .await
            .map_err(|e| format!("加载条件单失败: {e}"))?;

        let (orders, rejected) = partition(rows);
        let loaded = orders.len();
        Self::engine().write().await.set_orders(orders);
        let report = LoadReport { loaded, rejected };
        if !report.rejected.is_empty() {
            tracing::warn!(
                "[conditional] {} 行条件单无法读回为规则（不参与评估）：{}",
                report.rejected.len(),
                report
                    .rejected
                    .iter()
                    .map(|(id, reason)| format!("{id}: {reason}"))
                    .collect::<Vec<_>>()
                    .join(" / ")
            );
        }
        tracing::info!("[conditional] 已加载 {loaded} 条条件单");
        Ok(report)
    }

    /// 写一条定义（插入或按列覆盖定义部分），随后刷新内存。
    ///
    /// 更新只写定义列：触发计数三列属于运行态，覆盖定义不该把「今天已触发几次」抹掉。
    pub async fn put(db: &DatabaseConnection, order: &ConditionalOrder) -> Result<(), String> {
        let existing = conditional_orders::Entity::find_by_id(&order.id)
            .one(db)
            .await
            .map_err(|e| format!("查询条件单失败: {e}"))?;
        match existing {
            Some(row) => {
                let mut active: conditional_orders::ActiveModel = row.into();
                apply_definition(&mut active, order);
                active.update(db).await.map_err(|e| format!("更新条件单失败: {e}"))?;
            },
            None => {
                let mut active = conditional_orders::ActiveModel {
                    id: Set(order.id.clone()),
                    ..Default::default()
                };
                apply_definition(&mut active, order);
                apply_counters(&mut active, order);
                active.insert(db).await.map_err(|e| format!("创建条件单失败: {e}"))?;
            },
        }
        Self::refresh(db).await?;
        Ok(())
    }

    /// 启用 / 停用 —— 改的是本表自己的 `enabled` 列。
    /// 旧实现改的是 `stock_analyses.status`（与分析查询争用同一列，还把整条分析记录标成 `"disabled"`）。
    pub async fn set_enabled(
        db: &DatabaseConnection,
        order_id: &str,
        enabled: bool,
    ) -> Result<ConditionalOrder, String> {
        let row = conditional_orders::Entity::find_by_id(order_id)
            .one(db)
            .await
            .map_err(|e| format!("查询条件单失败: {e}"))?
            .ok_or_else(|| format!("条件单不存在: {order_id}"))?;
        let mut active: conditional_orders::ActiveModel = row.into();
        active.enabled = Set(enabled);
        active.updated_at = Set(chrono::Utc::now().timestamp_millis());
        let saved = active.update(db).await.map_err(|e| format!("变更条件单启停失败: {e}"))?;
        Self::refresh(db).await?;
        domain_from_row(saved)
    }

    /// 摘除一条定义。
    pub async fn remove(db: &DatabaseConnection, order_id: &str) -> Result<(), String> {
        let deleted = conditional_orders::Entity::delete_many()
            .filter(conditional_orders::Column::Id.eq(order_id))
            .exec(db)
            .await
            .map_err(|e| format!("删除条件单失败: {e}"))?;
        if deleted.rows_affected == 0 {
            return Err(format!("条件单不存在: {order_id}"));
        }
        Self::refresh(db).await?;
        Ok(())
    }

    /// 内存快照（定义权威仍是表，这里只是引擎正在用的那一份）。
    pub async fn snapshot() -> Vec<ConditionalOrder> {
        Self::engine().read().await.orders().to_vec()
    }

    /// 行情事件 → 判定 → 触发落地。返回这一轮真正触发的规则（供 wiring 层走同一条告警通路）。
    pub async fn handle_quote_event(
        db: &DatabaseConnection,
        event: &QuoteChangeEvent,
    ) -> Result<Vec<ConditionTrigger>, String> {
        let (hour, minute, today) = calendar::beijing_now();
        let now_ms = chrono::Utc::now().timestamp_millis();
        let beijing_date = today.format("%Y-%m-%d").to_string();
        let signal = QuoteSignal {
            current_price: event.current.price,
            // 前收盘取**这条行情自己**的 pre_close。旧实现读 `event.previous.pre_close`
            // ——那是上一跳记录里的前收盘，午休后第一跳与集合竞价首跳都会拿到错的分母。
            prev_close: event.current.pre_close,
            turnover_rate: turnover_or_none(event.current.turnover_rate),
            now_ms,
            beijing_date: beijing_date.clone(),
            beijing_minutes_of_day: hour * 60 + minute,
        };

        // 引擎读锁必须在这个块里把要带走的东西拷出来，不能把借用带出块外。
        let (triggered, indeterminate) = {
            let engine = Self::engine().read().await;
            let evaluated = engine.evaluate(&event.stock_code, &signal);
            let triggered: Vec<ConditionalOrder> =
                evaluated.iter().filter(|e| e.is_trigger()).map(|e| e.order.clone()).collect();
            let indeterminate: Vec<String> = evaluated
                .iter()
                .filter_map(|e| match &e.decision {
                    TriggerDecision::Indeterminate(reason) => {
                        Some(format!("{}: {reason}", e.order.id))
                    },
                    _ => None,
                })
                .collect();
            (triggered, indeterminate)
        };

        if triggered.is_empty() {
            if !indeterminate.is_empty() {
                // 「这一轮没触发」与「这一轮判不出来」是两件事，后者要有读数
                tracing::debug!(
                    "[conditional] {} 条规则本轮无法判定：{}",
                    indeterminate.len(),
                    indeterminate.join(" / ")
                );
            }
            return Ok(vec![]);
        }

        let pct = change_pct(signal.current_price, signal.prev_close).unwrap_or(0.0);
        let mut out = Vec::with_capacity(triggered.len());
        for candidate in triggered {
            // 1. 计数加在引擎那一份上，再整份写回表
            let persisted = {
                let mut engine = Self::engine().write().await;
                engine.record_trigger(&candidate.id, &beijing_date, now_ms)
            };
            let Some(order) = persisted else {
                tracing::warn!("[conditional] 触发后在引擎里找不到 {candidate:?}，计数未落");
                continue;
            };
            persist_counters(db, &order).await?;

            let trigger = ConditionTrigger {
                order_id: order.id.clone(),
                stock_code: order.stock_code.clone(),
                stock_name: order.stock_name.clone(),
                condition: order.condition.describe(),
                action: order.action.describe(),
                price: signal.current_price,
                change_pct: pct,
                triggered_at_ms: now_ms,
                reason: format!("{}, 现价 {:.2}", order.condition.describe(), signal.current_price),
            };

            // 2. 非「仅通知」的动作生成待审核交易意图 —— 条件单触发不直接下单
            if !order.action.is_notify_only() {
                let reasoning = format!(
                    "条件单触发: {}, 现价 {:.2}, 当日涨跌幅 {pct:.2}%",
                    trigger.condition, signal.current_price
                );
                match TradeIntentService::record_conditional_order_intent(
                    db,
                    &order.stock_code,
                    &order.stock_name,
                    &order.id,
                    &order.action.describe(),
                    &reasoning,
                    None,
                )
                .await
                {
                    Ok(intent_id) => tracing::info!(
                        "[conditional] 触发 → 交易意图已生成: order_id={} intent_id={}",
                        order.id,
                        intent_id
                    ),
                    Err(e) => tracing::error!("[conditional] 触发 → 交易意图生成失败: {e}"),
                }
            }
            out.push(trigger);
        }
        Ok(out)
    }
}

/// 换手率的「拿不到」判定。
///
/// `StockQuote.turnover_rate` 是裸 `f64`，不给这一路数据的 vendor 交回来是 `0.0`。
/// 直接 `Some(0.0)` 去比阈值会把「数据源没给换手率」判成「换手率没到」——
/// 本轮要消的正是这类把缺席读成结论的形态，故 ≤0 与非有限值都算拿不到。
fn turnover_or_none(value: f64) -> Option<f64> {
    (value > 0.0 && value.is_finite()).then_some(value)
}

/// 行集合 → `(可用规则, [(行 id, 无法读回的原因)])`。读不回来的行不静默丢。
fn partition(
    rows: Vec<conditional_orders::Model>,
) -> (Vec<ConditionalOrder>, Vec<(String, String)>) {
    let mut orders = Vec::with_capacity(rows.len());
    let mut rejected = Vec::new();
    for row in rows {
        let id = row.id.clone();
        match domain_from_row(row) {
            Ok(order) => orders.push(order),
            Err(reason) => rejected.push((id, reason)),
        }
    }
    (orders, rejected)
}

/// 表行 → 领域规则。`Err` 带的是原因（行 id 由调用方拿着），未知类别不兜底成默认值。
fn domain_from_row(row: conditional_orders::Model) -> Result<ConditionalOrder, String> {
    let condition = ConditionType::from_stored(&row.condition_type, row.condition_value)
        .ok_or_else(|| {
            format!(
                "条件类别或阈值无法读回（type={:?}, value={:?}）",
                row.condition_type, row.condition_value
            )
        })?;
    let action = match row.action_type.as_str() {
        "buy" => OrderAction::Buy { quantity: row.action_value.unwrap_or(100.0) as i32 },
        "sell" => OrderAction::Sell { quantity: row.action_value.map(|q| q as i32) },
        "reduce" => OrderAction::Reduce { ratio: row.action_value.unwrap_or(50.0) / 100.0 },
        "notify" => OrderAction::Notify,
        other => return Err(format!("动作类别无法读回（action_type={other:?}）")),
    };
    Ok(ConditionalOrder {
        id: row.id,
        stock_code: row.stock_code,
        stock_name: row.stock_name,
        condition,
        action,
        enabled: row.enabled,
        active_start: row.active_start,
        active_end: row.active_end,
        max_triggers_per_day: row.max_triggers_per_day.max(0) as u32,
        cool_down_minutes: row.cool_down_minutes.max(0) as u32,
        today_trigger_count: row.triggers_today.max(0) as u32,
        trigger_date: row.trigger_date,
        last_triggered_at: row.last_triggered_at,
        created_at: row.created_at,
        updated_at: row.updated_at,
    })
}

/// 定义列（启停/新建时写这些；运行态计数列不在其中）
fn apply_definition(active: &mut conditional_orders::ActiveModel, order: &ConditionalOrder) {
    active.stock_code = Set(order.stock_code.clone());
    active.stock_name = Set(order.stock_name.clone());
    active.condition_type = Set(order.condition.as_str().to_string());
    active.condition_value = Set(Some(order.condition.threshold_value()));
    active.action_type = Set(action_type_str(&order.action).to_string());
    active.action_value = Set(action_value_of(&order.action));
    active.enabled = Set(order.enabled);
    active.active_start = Set(order.active_start.clone());
    active.active_end = Set(order.active_end.clone());
    active.cool_down_minutes = Set(order.cool_down_minutes as i32);
    active.max_triggers_per_day = Set(order.max_triggers_per_day as i32);
    active.created_at = Set(order.created_at);
    active.updated_at = Set(order.updated_at);
}

/// 运行态列 —— 只在插入时写（更新定义不该把「今天已触发几次」抹掉）
fn apply_counters(active: &mut conditional_orders::ActiveModel, order: &ConditionalOrder) {
    active.triggers_today = Set(order.today_trigger_count as i32);
    active.trigger_date = Set(order.trigger_date.clone());
    active.last_triggered_at = Set(order.last_triggered_at);
    active.last_trigger_reason = Set(None);
}

fn action_type_str(action: &OrderAction) -> &'static str {
    match action {
        OrderAction::Buy { .. } => "buy",
        OrderAction::Sell { .. } => "sell",
        OrderAction::Reduce { .. } => "reduce",
        OrderAction::Notify => "notify",
    }
}

fn action_value_of(action: &OrderAction) -> Option<f64> {
    match action {
        OrderAction::Buy { quantity } => Some(*quantity as f64),
        OrderAction::Sell { quantity } => quantity.map(|q| q as f64),
        OrderAction::Reduce { ratio } => Some(*ratio),
        OrderAction::Notify => None,
    }
}

/// 只回写计数三列 + `updated_at`，不动定义。
async fn persist_counters(db: &DatabaseConnection, order: &ConditionalOrder) -> Result<(), String> {
    let row = conditional_orders::Entity::find_by_id(&order.id)
        .one(db)
        .await
        .map_err(|e| format!("回写触发计数失败（查询）: {e}"))?;
    let Some(row) = row else {
        // 表里没有这一行 ⇒ 内存与表已经不一致，必须点名而不是假装计数成功
        tracing::warn!(
            "[conditional] 触发计数无处可写：表里没有条件单 {}（内存引擎与表已不一致）",
            order.id
        );
        return Ok(());
    };
    let mut active: conditional_orders::ActiveModel = row.into();
    apply_counters(&mut active, order);
    active.updated_at = Set(chrono::Utc::now().timestamp_millis());
    active.update(db).await.map_err(|e| format!("回写触发计数失败: {e}"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use axagent_entities::conditional_orders::{ActiveModel, Entity};
    use axagent_harness::market_data::StockQuote;
    use sea_orm::{EntityTrait, Set};

    fn quote(code: &str, price: f64) -> StockQuote {
        StockQuote {
            code: code.to_string(),
            name: "测试股".to_string(),
            price,
            pre_close: 12.0,
            open: 12.1,
            high: 12.5,
            low: 11.9,
            volume: 1_000_000.0,
            amount: 12_340_000.0,
            change_pct: -8.33,
            turnover_rate: 1.5,
            pe: Some(10.0),
            pb: Some(1.2),
            total_mv: Some(1.2e10),
            circulating_mv: Some(1.0e10),
            limit_up: Some(13.2),
            limit_down: Some(10.8),
            is_st: false,
            timestamp: "2026-10-15 10:00:00".to_string(),
        }
    }

    fn event(code: &str, price: f64) -> QuoteChangeEvent {
        QuoteChangeEvent {
            stock_code: code.to_string(),
            previous: None,
            current: quote(code, price),
            change_pct: -8.33,
            trigger: "test".to_string(),
        }
    }

    fn order(id: &str, code: &str, price: f64, action: OrderAction) -> ConditionalOrder {
        ConditionalOrder {
            id: id.to_string(),
            stock_code: code.to_string(),
            stock_name: "测试股".to_string(),
            condition: ConditionType::StopLoss { price },
            action,
            enabled: true,
            active_start: None,
            active_end: None,
            max_triggers_per_day: 3,
            cool_down_minutes: 30,
            today_trigger_count: 0,
            trigger_date: None,
            last_triggered_at: None,
            created_at: 1_760_000_000_000,
            updated_at: 1_760_000_000_000,
        }
    }

    /// 落库层端到端（阶段 F-1 的第二层门：参数决定结构的批必须「纯函数 + 真落库」各有一条）。
    ///
    /// ⚠ 为什么几条断言合并在**一个**测试里顺序跑：桥接层的引擎是进程级 static，
    /// 两个 `#[tokio::test]` 并发时会互相 `refresh()` 覆盖对方的规则集 ⇒ 断言变成随机。
    /// 要把它们拆成独立测试，得先把引擎从 static 挪到 `AppState`（像 `stock_monitor` 那样），
    /// 那是另一批的事，此处先如实记下这个测试形状约束。
    #[tokio::test]
    async fn definition_and_trigger_state_round_trip_through_the_table() {
        let h = axagent_dao::db::create_test_pool().await.expect("测试库应可创建");
        let db = &h.conn;

        // ① 写侧落库 + 读回：用户阈值必须逐字回来（旧形态在这里是硬编码 3%/10%）
        ConditionalOrderBridge::put(db, &order("co_rt", "601899", 12.34, OrderAction::Notify))
            .await
            .expect("新表应可建并可写（声明式对账正门）");
        let (defs, rejected) = ConditionalOrderBridge::definitions(db).await.unwrap();
        assert!(rejected.is_empty(), "可读回的行不该进缺席集合：{rejected:?}");
        assert_eq!(defs.len(), 1);
        assert_eq!(
            defs[0].condition,
            ConditionType::StopLoss { price: 12.34 },
            "阈值必须是用户给的那个价"
        );
        assert_eq!(defs[0].cool_down_minutes, 30);
        assert_eq!(defs[0].max_triggers_per_day, 3);

        // ② 读不回来的行必须点名，而不是静默少一条（否则「表里有 3 条」被读成「风控在盯 2 条」）
        Entity::insert(ActiveModel {
            id: Set("co_bad".to_string()),
            stock_code: Set("000001".to_string()),
            stock_name: Set("测试股".to_string()),
            condition_type: Set("ma_cross".to_string()),
            condition_value: Set(Some(3.0)),
            action_type: Set("notify".to_string()),
            action_value: Set(None),
            enabled: Set(true),
            active_start: Set(None),
            active_end: Set(None),
            cool_down_minutes: Set(30),
            max_triggers_per_day: Set(3),
            triggers_today: Set(0),
            trigger_date: Set(None),
            last_triggered_at: Set(None),
            last_trigger_reason: Set(None),
            created_at: Set(1_760_000_000_000),
            updated_at: Set(1_760_000_000_000),
        })
        .exec(db)
        .await
        .expect("坏样本应能写入");
        let report = ConditionalOrderBridge::refresh(db).await.unwrap();
        assert_eq!(report.loaded, 1, "只有一行能读回为规则");
        assert_eq!(report.rejected.len(), 1);
        assert_eq!(report.rejected[0].0, "co_bad");
        assert!(
            report.rejected[0].1.contains("ma_cross"),
            "要指名是哪个类别读不回来：{}",
            report.rejected[0].1
        );

        // ③ 触发：计数落表 + 落进引擎那一份（旧形态回写的是调用方 clone，引擎恒 0）
        let triggers = ConditionalOrderBridge::handle_quote_event(db, &event("601899", 11.0))
            .await
            .expect("行情回调评估不该失败");
        assert_eq!(triggers.len(), 1, "现价 11.0 已跌破 12.34 ⇒ 必须触发");
        assert_eq!(triggers[0].condition, "跌破止损价 12.34", "触发产物要带人话描述");
        let row = Entity::find_by_id("co_rt").one(db).await.unwrap().expect("①写入的行应在");
        assert_eq!(row.triggers_today, 1, "计数必须写回表");
        assert_eq!(
            row.trigger_date.as_deref(),
            Some(
                axagent_astock_data::calendar::beijing_today()
                    .format("%Y-%m-%d")
                    .to_string()
                    .as_str()
            ),
            "计数要挂在北京日上，否则跨日不滚动"
        );
        assert!(row.last_triggered_at.is_some(), "冷却窗口读的是这个时刻");

        // ④ 同一条规则紧接着再来一次同一行情 ⇒ 被冷却拦住（这条红就是旧缺陷 (c) 复发）
        let again =
            ConditionalOrderBridge::handle_quote_event(db, &event("601899", 11.0)).await.unwrap();
        assert!(again.is_empty(), "30 分钟冷却窗口内不得重复触发");

        // ⑤ 停用的是本表自己的 enabled 列，且引擎随即不再评估它
        let disabled = ConditionalOrderBridge::set_enabled(db, "co_rt", false).await.unwrap();
        assert!(!disabled.enabled);
        let raw = Entity::find_by_id("co_rt").one(db).await.unwrap().unwrap();
        assert!(!raw.enabled, "停用要落库，不能只改内存");
        assert_eq!(raw.triggers_today, 1, "改 enabled 不得抹掉运行态计数（定义列与运行列分开写）");
        let third =
            ConditionalOrderBridge::handle_quote_event(db, &event("601899", 11.0)).await.unwrap();
        assert!(third.is_empty(), "停用后该规则不参与评估");
    }

    /// 映射层：坏数据不许被兜成「看起来正常」的默认值。
    #[test]
    fn mapping_refuses_to_invent_values_for_threshold_action_or_turnover() {
        let base = conditional_orders::Model {
            id: "co_x".to_string(),
            stock_code: "600519".to_string(),
            stock_name: "测试".to_string(),
            condition_type: "stop_loss".to_string(),
            condition_value: Some(9.9),
            action_type: "notify".to_string(),
            action_value: None,
            enabled: true,
            active_start: None,
            active_end: None,
            cool_down_minutes: 30,
            max_triggers_per_day: 3,
            triggers_today: 0,
            trigger_date: None,
            last_triggered_at: None,
            last_trigger_reason: None,
            created_at: 0,
            updated_at: 0,
        };
        assert!(domain_from_row(base.clone()).is_ok(), "齐备形态必须读得回来");

        let mut no_threshold = base.clone();
        no_threshold.condition_value = None;
        assert!(
            domain_from_row(no_threshold).is_err(),
            "缺阈值的价格类条件不能默认成某个价（那会把「没设过」判成「设了」）"
        );

        let mut unknown_action = base.clone();
        unknown_action.action_type = "short_selling".to_string();
        assert!(domain_from_row(unknown_action).is_err(), "未知动作类别不得兜成 Notify");

        let negative_counters = conditional_orders::Model {
            triggers_today: -7,
            cool_down_minutes: -5,
            max_triggers_per_day: -1,
            ..base
        };
        let mapped = domain_from_row(negative_counters).expect("负数计数应能读回并夹到 0");
        assert_eq!(
            (mapped.today_trigger_count, mapped.cool_down_minutes, mapped.max_triggers_per_day),
            (0, 0, 0)
        );

        // 换手率哨兵：`StockQuote.turnover_rate` 是裸 f64，不给这一路数据的 vendor 交回 0.0。
        // 把 0.0 当 0% 去比阈值，就是「数据源没给」被读成「换手率没到」。
        assert_eq!(turnover_or_none(0.0), None, "0.0 必须判成拿不到，不是 0%");
        assert_eq!(turnover_or_none(f64::NAN), None, "NaN 同样不是 0%");
        assert_eq!(turnover_or_none(8.5), Some(8.5), "正常值原样交回");
    }
}
