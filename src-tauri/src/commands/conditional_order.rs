// SPDX-License-Identifier: AGPL-3.0-only

//! 条件单管理命令 —— 定义写侧的唯一入口
//!
//! 这一层不落 `settings`、也不从任何分析记录反推定义：所有增删改都走
//! [`ConditionalOrderBridge`] 进 `conditional_orders` 表，内存引擎只是它的镜像。
//! 命令层因此不碰实体、不写 SQL —— 行的读回映射只有一份（在桥接层）。

use crate::AppState;
use axagent_agent_macro::agent_command;
use axagent_analysis_engine::conditional_order::{
    ConditionTrigger, ConditionType, ConditionalOrder, OrderAction,
};
use axagent_analysis_engine::conditional_order_bridge::ConditionalOrderBridge;
use serde::{Deserialize, Serialize};
use tauri::State;

/// 创建条件单请求
///
/// **条件阈值与动作参数是两个字段**（旧形态共用一个 `action_value`：一条规则里
/// 「换手率超 10%」和「买入 100 股」抢同一个数，谁后写谁生效）。
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateConditionalOrderRequest {
    pub stock_code: String,
    pub stock_name: String,
    /// `stop_loss` / `take_profit` / `resistance_break` / `support_break` / `change_pct` / `turnover_rate`
    pub condition_type: String,
    /// 阈值：价格类为绝对价（元），比率类为百分比数值（5.0 = 5%）
    pub condition_value: f64,
    /// `buy` / `sell` / `reduce` / `notify`
    pub action_type: String,
    /// 动作参数：`buy` 股数 / `reduce` 比例（0.0–1.0）；`sell`、`notify` 可不给
    pub action_value: Option<f64>,
    pub cool_down_minutes: Option<i32>,
    pub max_triggers_per_day: Option<i32>,
    /// 生效时段（北京时间 `HH:MM`）。任一侧给了就生效，缺的那侧补到当日边界。
    pub active_start: Option<String>,
    /// 生效时段终点（北京时间 `HH:MM`）
    pub active_end: Option<String>,
}

/// 列表项 —— 所有描述都由引擎的 `describe()` 给，不再 `format!("{:?}")` 漏 Rust `Debug`
#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ConditionalOrderItem {
    pub order_id: String,
    pub stock_code: String,
    pub stock_name: String,
    pub condition_type: String,
    pub condition_value: f64,
    pub condition_text: String,
    pub action_type: String,
    pub action_value: Option<f64>,
    pub action_text: String,
    pub enabled: bool,
    pub cool_down_minutes: i32,
    pub max_triggers_per_day: i32,
    pub triggers_today: i32,
    pub trigger_date: Option<String>,
    pub last_triggered_at: Option<i64>,
    pub active_start: Option<String>,
    pub active_end: Option<String>,
}

/// 表里存在、但读不回成规则的该行 —— 必须与「没有规则」分列呈现
#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct UnreadableConditionalOrder {
    pub order_id: String,
    pub reason: String,
}

/// 列表返回体
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConditionalOrderListView {
    pub items: Vec<ConditionalOrderItem>,
    pub unreadable: Vec<UnreadableConditionalOrder>,
    /// 当前正在被引擎评估的条数（= 启用且可读回）—— 与 `items.len()` 是两个计数域，
    /// 并排给但不同名，避免面板把「表里有 5 条」读成「风控在盯 5 条」
    pub evaluating: usize,
}

/// 手动评估的返回体（旧形态返回 `u32`，把「没规则」「判了没中」「判不出」压成同一个 0）
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ManualEvaluateReport {
    pub codes_requested: usize,
    pub codes_quote_failed: usize,
    pub triggered: Vec<ConditionTrigger>,
    /// 本北京日已累计触发的规则数（读自表，不是内存猜的）
    pub triggered_today: usize,
    /// 当前会话阶段（`交易中` / `午休中…` / `已收盘` / `非交易日` …）。
    /// 非交易时段行情是冻结的，这一句让「没触发」不被读成「风控说没问题」。
    pub session: String,
}

fn parse_condition(kind: &str, value: f64) -> Result<ConditionType, String> {
    if matches!(kind, "ma_cross" | "custom") {
        return Err(if kind == "ma_cross" {
            "均线穿越（maCross）本版本未实现：引擎拿不到日 K 序列，不接受创建该类别".to_string()
        } else {
            "自定义表达式（custom）本版本未实现：Rhai 脚本未接入条件单评估，不接受创建该类别"
                .to_string()
        });
    }
    if !value.is_finite() || value <= 0.0 {
        return Err(format!("条件阈值必须是正数（价格类为元、比率类为百分比），实测 {value}"));
    }
    ConditionType::from_stored(kind, Some(value)).ok_or_else(|| format!("未知条件类型: {kind}"))
}

fn parse_action(kind: &str, value: Option<f64>) -> Result<OrderAction, String> {
    match kind {
        "buy" => {
            let quantity = value.ok_or_else(|| "买入动作需要股数（actionValue）".to_string())?;
            if !quantity.is_finite() || quantity <= 0.0 {
                return Err(format!("买入股数必须是正数，实测 {quantity}"));
            }
            Ok(OrderAction::Buy { quantity: quantity as i32 })
        },
        "sell" => Ok(OrderAction::Sell { quantity: value.map(|q| q as i32) }),
        "reduce" => {
            let ratio =
                value.ok_or_else(|| "减仓动作需要比例（actionValue，0.0–1.0）".to_string())?;
            if !(0.0..=1.0).contains(&ratio) || ratio <= 0.0 {
                return Err(format!("减仓比例必须落在 (0, 1]，实测 {ratio}"));
            }
            Ok(OrderAction::Reduce { ratio })
        },
        "notify" => Ok(OrderAction::Notify),
        other => Err(format!("未知动作类型: {other}")),
    }
}

fn to_item(order: ConditionalOrder) -> ConditionalOrderItem {
    let value = order.condition.threshold_value();
    ConditionalOrderItem {
        order_id: order.id,
        stock_code: order.stock_code,
        stock_name: order.stock_name,
        condition_type: order.condition.as_str().to_string(),
        condition_value: value,
        condition_text: order.condition.describe(),
        action_type: match order.action {
            OrderAction::Buy { .. } => "buy",
            OrderAction::Sell { .. } => "sell",
            OrderAction::Reduce { .. } => "reduce",
            OrderAction::Notify => "notify",
        }
        .to_string(),
        action_value: match order.action {
            OrderAction::Buy { quantity } => Some(quantity as f64),
            OrderAction::Sell { quantity } => quantity.map(|q| q as f64),
            OrderAction::Reduce { ratio } => Some(ratio),
            OrderAction::Notify => None,
        },
        action_text: order.action.describe(),
        enabled: order.enabled,
        cool_down_minutes: order.cool_down_minutes as i32,
        max_triggers_per_day: order.max_triggers_per_day as i32,
        triggers_today: order.today_trigger_count as i32,
        trigger_date: order.trigger_date,
        last_triggered_at: order.last_triggered_at,
        active_start: order.active_start,
        active_end: order.active_end,
    }
}

/// 创建条件单（落 `conditional_orders` 表）
#[agent_command(domain = "finance", safety = Safe, call_mode = StateInput, description = "创建条件单：按用户给的阈值登记一条 if-then 规则并立即参与行情评估")]
#[tauri::command]
pub async fn create_conditional_order(
    state: State<'_, AppState>,
    req: CreateConditionalOrderRequest,
) -> Result<String, String> {
    let condition = parse_condition(&req.condition_type, req.condition_value)?;
    let action = parse_action(&req.action_type, req.action_value)?;

    let now = chrono::Utc::now().timestamp_millis();
    let order = ConditionalOrder {
        id: format!("co_{}", uuid::Uuid::new_v4()),
        stock_code: req.stock_code,
        stock_name: req.stock_name,
        condition,
        action,
        enabled: true,
        active_start: req.active_start,
        active_end: req.active_end,
        max_triggers_per_day: req.max_triggers_per_day.unwrap_or(3).max(0) as u32,
        cool_down_minutes: req.cool_down_minutes.unwrap_or(30).max(0) as u32,
        today_trigger_count: 0,
        trigger_date: None,
        last_triggered_at: None,
        created_at: now,
        updated_at: now,
    };
    let id = order.id.clone();
    ConditionalOrderBridge::put(state.harness.db(), &order).await?;
    Ok(id)
}

/// 条件单列表（读表，含已停用与读不回来的行）
#[agent_command(domain = "finance", safety = Safe, call_mode = StateOnly, description = "查询条件单列表（含已停用项与无法读回的行）")]
#[tauri::command]
pub async fn list_conditional_orders(
    state: State<'_, AppState>,
) -> Result<ConditionalOrderListView, String> {
    let (orders, unreadable) = ConditionalOrderBridge::definitions(state.harness.db()).await?;
    let evaluating = orders.iter().filter(|o| o.enabled).count();
    Ok(ConditionalOrderListView {
        items: orders.into_iter().map(to_item).collect(),
        unreadable: unreadable
            .into_iter()
            .map(|(order_id, reason)| UnreadableConditionalOrder { order_id, reason })
            .collect(),
        evaluating,
    })
}

/// 启用 / 停用一条条件单（改本表自己的 `enabled` 列）
#[agent_command(domain = "finance", safety = Safe, call_mode = StateInput, description = "启用或停用指定条件单")]
#[tauri::command]
pub async fn set_conditional_order_enabled(
    state: State<'_, AppState>,
    order_id: String,
    enabled: bool,
) -> Result<ConditionalOrderItem, String> {
    let order = ConditionalOrderBridge::set_enabled(state.harness.db(), &order_id, enabled).await?;
    Ok(to_item(order))
}

/// 摘除一条条件单
#[agent_command(domain = "finance", safety = Caution, call_mode = StateInput, description = "删除指定条件单（连同其触发计数）")]
#[tauri::command]
pub async fn delete_conditional_order(
    state: State<'_, AppState>,
    order_id: String,
) -> Result<(), String> {
    ConditionalOrderBridge::remove(state.harness.db(), &order_id).await
}

/// 手动评估（调试用）。默认评估「有启用条件单的那些股票」而不是持仓（持仓现网 0 行，
/// 旧写法因此每次都直接返回 0）。时段闸门已撤：判不出要说判不出，不能静默返空。
#[agent_command(domain = "finance", safety = Caution, call_mode = Manual, description = "立即用最新行情评估条件单，返回触发明细与无法判定的计数")]
#[tauri::command]
pub async fn manual_evaluate_conditions(
    state: State<'_, AppState>,
    stock_code: Option<String>,
) -> Result<ManualEvaluateReport, String> {
    let db = state.harness.db().clone();
    let client = state.astock_client.clone();

    // 先对齐表与内存引擎：调试命令判的必须是「表里现在这份定义」，
    // 否则外部改过表之后它会拿旧的内存镜像得出一个看起来正常的 0。
    ConditionalOrderBridge::refresh(&db).await?;

    let codes: Vec<String> = match stock_code {
        Some(code) => vec![code],
        None => {
            let (orders, _) = ConditionalOrderBridge::definitions(&db).await?;
            let mut codes: Vec<String> =
                orders.into_iter().filter(|o| o.enabled).map(|o| o.stock_code).collect();
            codes.sort();
            codes.dedup();
            codes
        },
    };

    let mut triggered = Vec::new();
    let mut codes_quote_failed = 0usize;
    for code in &codes {
        let quote = match client.get_quote(code).await {
            Ok(q) => q,
            Err(e) => {
                tracing::warn!("[manual_evaluate] 股票 {code} 行情获取失败: {e}");
                codes_quote_failed += 1;
                continue;
            },
        };
        let event = axagent_astock_data::realtime_quote::QuoteChangeEvent {
            stock_code: quote.code.clone(),
            previous: None,
            current: quote,
            change_pct: 0.0,
            trigger: "manual".to_string(),
        };
        triggered.extend(ConditionalOrderBridge::handle_quote_event(&db, &event).await?);
    }

    let (orders, _) = ConditionalOrderBridge::definitions(&db).await?;
    let today = axagent_astock_data::calendar::beijing_today().format("%Y-%m-%d").to_string();
    Ok(ManualEvaluateReport {
        codes_requested: codes.len(),
        codes_quote_failed,
        triggered,
        triggered_today: orders
            .iter()
            .filter(|o| o.trigger_date.as_deref() == Some(today.as_str()))
            .filter(|o| o.today_trigger_count > 0)
            .count(),
        session: axagent_astock_data::calendar::current_trading_session()
            .status_label()
            .to_string(),
    })
}
