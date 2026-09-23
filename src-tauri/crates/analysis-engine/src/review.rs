use std::collections::HashMap;

use axagent_astock_data::calendar;
use axagent_entities::stock_analyses;
use axagent_harness::market_data::MarketDataProvider;
use sea_orm::{ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, QueryOrder, QuerySelect};

/// 收盘复盘报告
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DailyReview {
    pub date: String,
    /// "交易中" | "已收盘" | "非交易日"
    pub market_status: String,
    pub watchlist_summary: Vec<StockDaySummary>,
    pub generated_at: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StockDaySummary {
    pub stock_code: String,
    pub stock_name: String,
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
    pub change_pct: f64,
    /// 当日量/5日均量
    pub volume_ratio: Option<f64>,
    pub key_events: Vec<String>,
    /// 当日触发的告警描述（来自 price_alerts 表）
    pub alert_triggers: Vec<String>,
    /// 该股上次分析决策对比（新增）
    pub last_decision: Option<DecisionComparison>,
}

/// 上次分析与今日行情的对比
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DecisionComparison {
    pub analysis_date: String,
    /// 决策档位。取 `stock_analyses.decision_action` 的**存储形态** ——
    /// 即中文 6 档 / 中文哨兵（`买入` / `增持` / `持有` / `观望` / `减持` /
    /// `卖出` / `不确定` / `数据缺失`），**不是**英文 token。
    /// 权威值域见 `axagent_harness::decision_action`。
    ///
    /// ⚠️ 本字段是「方向强度」轴，**不含**持仓状态。展示档（持有 vs 观望）由
    ///    消费端用 `(action, position_state, position_pct)` 派生 —— 见 `position_state`。
    pub action: String,
    /// 决策持仓状态轴（与 `action` **正交**，v228 引入）。
    ///
    /// `None` 的语义同 `stock_analyses.decision_position_state`：该记录产生于本字段
    /// 引入之前，**采集时点没有这个信息** —— 消费端应按 `position_pct` 自行派生，
    /// **不得**读成 `EMPTY`。
    ///
    /// 为什么必须下发：此前本结构只带 `action`，前端 `DailyReviewPanel` 直接渲染它，
    /// 于是**同一条决策**在历史卡上显示「观望」、在收盘复盘里显示「持有」——
    /// 与本仓 2026-09-21 修掉的「挂角 vs 结论」矛盾同源（缺的正是这一轴）。
    #[serde(default)]
    pub position_state: Option<String>,
    /// 决策仓位权重（%）。`position_state` 为 `None` 时，消费端用它派生展示档。
    #[serde(default)]
    pub position_pct: Option<f64>,
    pub target_price: Option<f64>,
    pub stop_loss: Option<f64>,
    pub days_since_analysis: u32,
    /// 原始决策时间维度
    pub time_horizon: Option<String>,
    /// 原始期望持有天数
    pub expected_holding_days: Option<i64>,
    /// 收盘价在目标区间内？(仅 BUY)
    pub in_target_zone: bool,
    /// 已触发止损？
    pub stop_loss_hit: bool,
    /// 已达目标价？
    pub target_hit: bool,
}

/// 收盘复盘工作流
pub struct PostCloseReview;

impl PostCloseReview {
    /// 生成每日复盘报告
    ///
    /// `triggered_alerts` 为 stock_code -> alert descriptions 的映射，
    /// `db` 用于查询 `stock_analyses` 以做决策对比。
    pub async fn generate(
        client: &dyn MarketDataProvider,
        watchlist: &[(String, String)],
        triggered_alerts: &HashMap<String, Vec<String>>,
        db: &DatabaseConnection,
    ) -> Result<DailyReview, String> {
        let now = chrono::Utc::now();
        let today = now.format("%Y-%m-%d").to_string();
        // 修复 M-DS-2: 原代码 `NaiveDate::parse_from_str(&today, ...).unwrap_or_default()`
        // 解析失败时返回 1970-01-01（Unix 纪元）。直接从 `now` 取 date_naive()
        // 跳过字符串序列化/反序列化往返，杜绝 1970 错日期。
        let today_date = now.date_naive();

        let market_status = if calendar::is_trading_day(&today_date) {
            if calendar::is_trading_time() {
                "交易中".to_string()
            } else {
                "已收盘".to_string()
            }
        } else {
            "非交易日".to_string()
        };

        let mut summaries = Vec::new();
        for (code, name) in watchlist {
            let quote = match client.get_quote(code).await {
                Ok(q) => q,
                Err(_) => continue,
            };

            let klines = client.get_klines(code, "daily", 6, None).await.ok().unwrap_or_default();
            let vol_ratio = if klines.len() >= 6 {
                let avg_vol_5 = klines.iter().rev().take(5).map(|k| k.volume).sum::<f64>() / 5.0;
                Some(klines.last().map(|k| k.volume).unwrap_or(0.0) / avg_vol_5)
            } else {
                None
            };

            let mut key_events = Vec::new();
            if quote.change_pct.abs() > 5.0 {
                key_events.push(format!("异常波动 {:.2}%", quote.change_pct));
            }
            if let Some(vr) = vol_ratio {
                if vr > 2.0 {
                    key_events.push(format!("放量 {:.1}x", vr));
                }
                if vr < 0.5 {
                    key_events.push("极度缩量".to_string());
                }
            }
            if quote.is_st {
                key_events.push("ST股票".to_string());
            }

            // 合并当日触发的告警
            let stock_alerts = triggered_alerts.get(code).cloned().unwrap_or_default();

            // 查询该股最近一次 completed 分析，做决策对比
            let last_decision = fetch_latest_analysis_decision(code, &quote.price, db).await;

            summaries.push(StockDaySummary {
                stock_code: code.clone(),
                stock_name: name.clone(),
                open: quote.open,
                high: quote.high,
                low: quote.low,
                close: quote.price,
                change_pct: quote.change_pct,
                volume_ratio: vol_ratio,
                key_events,
                alert_triggers: stock_alerts,
                last_decision,
            });
        }

        Ok(DailyReview {
            date: today,
            market_status,
            watchlist_summary: summaries,
            generated_at: chrono::Utc::now().format("%Y-%m-%d %H:%M:%S").to_string(),
        })
    }
}

/// 查询该股最近一次 completed 分析，对比今日收盘生成 DecisionComparison
async fn fetch_latest_analysis_decision(
    stock_code: &str,
    current_price: &f64,
    db: &DatabaseConnection,
) -> Option<DecisionComparison> {
    let row = stock_analyses::Entity::find()
        .filter(stock_analyses::Column::StockCode.eq(stock_code))
        .filter(stock_analyses::Column::Status.eq("completed"))
        .order_by_desc(stock_analyses::Column::CreatedAt)
        .limit(1)
        .one(db)
        .await
        .ok()
        .flatten()?;

    // ⚠️ 兜底一律用权威哨兵 `ACTION_UNAVAILABLE`，**不得**自造 `"uncertain"`：
    //   「决策缺失」与「有决策但无法判断」是两回事（见 `decision_action` 模块头）。
    //   空串也必须走同一分支 —— `filter(!is_empty)` 覆盖历史实现用
    //   `unwrap_or_default()` 留下的空串，那种值会被前端解析成「观望」，
    //   等于把「没有决策」伪装成操作建议。
    let action = row
        .decision_action
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(axagent_harness::decision_action::ACTION_UNAVAILABLE)
        .to_string();
    let target = row.decision_json.as_ref().and_then(|raw| {
        serde_json::from_str::<serde_json::Value>(raw)
            .ok()
            .and_then(|v| v.get("targetPrice").and_then(|p| p.as_f64()))
    });
    let stop_loss = row.decision_json.as_ref().and_then(|raw| {
        serde_json::from_str::<serde_json::Value>(raw)
            .ok()
            .and_then(|v| v.get("stopLoss").and_then(|p| p.as_f64()))
    });

    // 解析分析日期 → 计算已过天数
    let days_since = chrono::NaiveDate::parse_from_str(&row.analysis_date, "%Y-%m-%d")
        .ok()
        .and_then(|d| {
            chrono::Utc::now().date_naive().signed_duration_since(d).num_days().try_into().ok()
        })
        .unwrap_or(0);

    let price = *current_price;
    let in_target_zone = target.is_some_and(|t| (price - t).abs() / t <= 0.05);
    let stop_loss_hit = stop_loss.is_some_and(|s| price <= s);
    let target_hit = target.is_some_and(|t| price >= t);

    // 从 DB 列或 decision_json 提取时间维度
    let time_horizon = row.decision_time_horizon.or_else(|| {
        row.decision_json.as_ref().and_then(|raw| {
            serde_json::from_str::<serde_json::Value>(raw).ok().and_then(|v| {
                v.get("timeHorizon")
                    .or_else(|| v.get("time_horizon"))
                    .and_then(|s| s.as_str().map(|s| s.to_string()))
            })
        })
    });
    let expected_holding_days = row.decision_expected_holding_days.or_else(|| {
        row.decision_json.as_ref().and_then(|raw| {
            serde_json::from_str::<serde_json::Value>(raw).ok().and_then(|v| {
                v.get("expectedHoldingDays")
                    .or_else(|| v.get("expected_holding_days"))
                    .and_then(|n| n.as_u64().map(|n| n as i64))
            })
        })
    });

    // 持仓状态轴：与 `action` 正交，必须一并下发，否则消费端只能拿到方向档
    // （`action`）而看不到持仓状态 ⇒ 「观望 / 持有」在复盘面板里会再次显示错档。
    // 空串同样按 `None` 处理（与 action 同纪律：空值不是有效状态）。
    let position_state = row
        .decision_position_state
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    // ⚠️ 不做 `unwrap_or(0.0)`：`None`（本列引入之前的记录）与「0% 仓位」是两回事，
    //   强行补 0 会把「不知道」变成「空仓」，与 `decision_position_state` 的
    //   NULL 语义约定一致 —— 消费端拿到 `None` 时应保持「信息缺失」。
    let position_pct = row.decision_position_pct;

    Some(DecisionComparison {
        analysis_date: row.analysis_date,
        action,
        position_state,
        position_pct,
        target_price: target,
        stop_loss,
        days_since_analysis: days_since,
        time_horizon,
        expected_holding_days,
        in_target_zone,
        stop_loss_hit,
        target_hit,
    })
}
