//! 手动交易日志引擎 — 用户录入买卖，自动聚合持仓、计算成本、跟踪盈亏。
//!
//! 核心功能:
//! 1. 交易校验（涨跌停、手数、持仓充足性、交易日历）
//! 2. 执行交易（写入 trades 表 + 自动更新 portfolio_holdings）
//! 3. 持仓汇总（加权平均成本 + 实时盈亏）
//! 4. 交易历史查询

use chrono::NaiveDate;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, QueryOrder,
    QuerySelect, Set,
};
use std::sync::Arc;

use axagent_astock_data::AStockClient;
use axagent_astock_data::{detect_market_type, get_st_price_limit_pct};
use axagent_core::entity::{portfolio_holdings, stock_analyses, trades};

/// 手动交易引擎
pub struct TradingEngine {
    db: Arc<DatabaseConnection>,
    astock_client: Arc<AStockClient>,
    /// 交易功能开关（默认 false，需用户主动开启）
    pub enabled: bool,
}

/// 交易校验结果
#[derive(Debug, Clone)]
pub struct TradeValidation {
    pub valid: bool,
    pub errors: Vec<String>,
    pub warnings: Vec<String>,
}

/// 持仓汇总（含实时盈亏）
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PositionSummary {
    pub stock_code: String,
    pub stock_name: String,
    pub total_shares: i32,
    pub avg_cost: f64,
    pub current_price: Option<f64>,
    pub market_value: Option<f64>,
    pub unrealized_pnl: Option<f64>,
    pub unrealized_pnl_pct: Option<f64>,
    pub total_realized_pnl: f64,
}

impl TradingEngine {
    pub fn new(db: Arc<DatabaseConnection>, astock_client: Arc<AStockClient>) -> Self {
        Self {
            db,
            astock_client,
            enabled: false,
        }
    }

    // ── 交易校验 ──

    /// 校验一笔交易是否合法（涨跌停、手数、持仓充足性、交易日历）
    pub async fn validate_trade(
        &self,
        stock_code: &str,
        direction: &str,
        quantity: i32,
        price: f64,
    ) -> TradeValidation {
        let mut errors = Vec::new();
        let mut warnings = Vec::new();

        // 基本合法性
        if quantity <= 0 {
            errors.push("交易数量必须 > 0".to_string());
        }
        if price <= 0.0 {
            errors.push("交易价格必须 > 0".to_string());
        }

        // 数量必须是整手数
        let market = detect_market_type(stock_code);
        let min_lot: i32 = if market == "star" || market == "chinext" {
            200
        } else {
            100
        };
        if quantity % min_lot != 0 {
            errors.push(format!("数量必须是 {} 股的整数倍（{}手）", min_lot, min_lot));
        }

        // 涨跌停校验
        if let Ok(quote) = self.astock_client.get_quote(stock_code).await {
            let limit_pct = get_st_price_limit_pct(quote.is_st, market);
            let reference_price = quote.open;
            let limit_up = reference_price * (1.0 + limit_pct / 100.0);
            let limit_down = reference_price * (1.0 - limit_pct / 100.0);

            if price > limit_up {
                errors.push(format!("买入价 {:.2} 超过涨停价 {:.2}", price, limit_up));
            }
            if price < limit_down {
                errors.push(format!("卖出价 {:.2} 低于跌停价 {:.2}", price, limit_down));
            }
        }

        // 卖出时检查持仓
        if direction == "sell" {
            let holdings = portfolio_holdings::Entity::find()
                .filter(portfolio_holdings::Column::StockCode.eq(stock_code))
                .one(self.db.as_ref())
                .await
                .ok()
                .flatten();

            match holdings {
                Some(h) => {
                    if h.shares < quantity as f64 {
                        errors.push(format!(
                            "持仓不足: 持有 {} 股, 试图卖出 {} 股",
                            h.shares as i32, quantity
                        ));
                    }
                },
                None => {
                    errors.push("没有该股票持仓，无法卖出".to_string());
                },
            }
        }

        // 分析一致性检查（仅买入时）
        if direction == "buy" {
            let last_analysis = stock_analyses::Entity::find()
                .filter(stock_analyses::Column::StockCode.eq(stock_code))
                .filter(stock_analyses::Column::Status.eq("completed"))
                .order_by_desc(stock_analyses::Column::CreatedAt)
                .one(self.db.as_ref())
                .await
                .ok()
                .flatten();

            if let Some(analysis) = last_analysis {
                if let Some(ref decision_json) = analysis.decision_json {
                    if let Ok(decision) =
                        serde_json::from_str::<serde_json::Value>(decision_json)
                    {
                        let suggested_action = decision["action"].as_str().unwrap_or("");
                        let suggested_target = decision["targetPrice"].as_f64();

                        if suggested_action == "卖出" || suggested_action == "减持" {
                            warnings.push(format!(
                                "分析建议「{}」而非买入，请二次确认",
                                suggested_action
                            ));
                        }

                        if let Some(target) = suggested_target {
                            let deviation = ((price - target) / target).abs() * 100.0;
                            if deviation > 5.0 {
                                warnings.push(format!(
                                    "入场价 {:.2} 偏离分析目标价 {:.2} ({:.1}%)",
                                    price, target, deviation
                                ));
                            }
                        }
                    }
                }
            }
        }

        // 交易日历检查（简化版：排除周末）
        let today = chrono::Utc::now().format("%Y-%m-%d").to_string();
        if !is_trading_day(&today) {
            warnings.push("当前非交易日，已记录但仅供参考".to_string());
        }

        TradeValidation {
            valid: errors.is_empty(),
            errors,
            warnings,
        }
    }

    // ── 执行交易 ──

    /// 执行一笔交易（写入 trades 表 + 自动更新 portfolio_holdings）
    ///
    /// - 买入：更新持仓（加权平均成本），若已持有则合并且权重平均成本
    /// - 卖出：计算已实现盈亏，减仓或清仓
    #[allow(clippy::too_many_arguments)]
    pub async fn execute_trade(
        &self,
        stock_code: &str,
        stock_name: &str,
        direction: &str,
        price: f64,
        quantity: i32,
        trade_date: &str,
        trade_time: &str,
        notes: Option<&str>,
    ) -> Result<trades::Model, String> {
        // 校验
        let validation = self
            .validate_trade(stock_code, direction, quantity, price)
            .await;
        if !validation.valid {
            return Err(validation.errors.join("; "));
        }

        let now = chrono::Utc::now().timestamp_millis();
        let trade_id = uuid::Uuid::new_v4().to_string();
        let mut realized_pnl = None;

        // 更新持仓
        if direction == "buy" {
            upsert_position(&self.db, stock_code, stock_name, quantity as f64, price).await?;
        } else {
            // sell: 计算实现盈亏
            let holdings = portfolio_holdings::Entity::find()
                .filter(portfolio_holdings::Column::StockCode.eq(stock_code))
                .one(self.db.as_ref())
                .await
                .map_err(|e| e.to_string())?;

            if let Some(h) = holdings {
                let sell_qty = quantity as f64;
                let cost = h.avg_cost * sell_qty;
                let revenue = price * sell_qty;
                realized_pnl = Some(revenue - cost);

                let remaining = h.shares - sell_qty;
                if remaining <= 0.0 {
                    // 清仓：删除持仓记录
                    portfolio_holdings::Entity::delete_by_id(&h.id)
                        .exec(self.db.as_ref())
                        .await
                        .map_err(|e| e.to_string())?;
                } else {
                    // 减仓：更新剩余数量（avg_cost 不变）
                    portfolio_holdings::Entity::update_many()
                        .col_expr(
                            portfolio_holdings::Column::Shares,
                            sea_orm::sea_query::Expr::value(remaining),
                        )
                        .col_expr(
                            portfolio_holdings::Column::UpdatedAt,
                            sea_orm::sea_query::Expr::value(now),
                        )
                        .filter(portfolio_holdings::Column::Id.eq(&h.id))
                        .exec(self.db.as_ref())
                        .await
                        .map_err(|e| e.to_string())?;
                }
            }
        }

        // 写入交易记录
        let trade = trades::ActiveModel {
            id: Set(trade_id),
            stock_code: Set(stock_code.to_string()),
            stock_name: Set(stock_name.to_string()),
            direction: Set(direction.to_string()),
            price: Set(price),
            quantity: Set(quantity),
            trade_date: Set(trade_date.to_string()),
            trade_time: Set(trade_time.to_string()),
            fee: Set(None),
            realized_pnl: Set(realized_pnl),
            notes: Set(notes.map(|s| s.to_string())),
            created_at: Set(now),
        };

        trade
            .insert(self.db.as_ref())
            .await
            .map_err(|e| e.to_string())
    }

    // ── 持仓汇总 ──

    /// 获取持仓汇总（含实时盈亏）
    pub async fn get_positions(&self) -> Result<Vec<PositionSummary>, String> {
        let holdings = portfolio_holdings::Entity::find()
            .all(self.db.as_ref())
            .await
            .map_err(|e| e.to_string())?;

        let mut positions = Vec::new();
        for h in holdings {
            let quote = self.astock_client.get_quote(&h.stock_code).await.ok();
            let current_price = quote.as_ref().map(|q| q.price);
            let market_value = current_price.map(|p| p * h.shares);
            let unrealized_pnl = market_value.map(|mv| mv - h.avg_cost * h.shares);
            let unrealized_pnl_pct = market_value.map(|mv| {
                let cost = h.avg_cost * h.shares;
                if cost > 0.0 {
                    ((mv - cost) / cost) * 100.0
                } else {
                    0.0
                }
            });

            // 累计已实现盈亏
            let realized: f64 = trades::Entity::find()
                .filter(trades::Column::StockCode.eq(&h.stock_code))
                .filter(trades::Column::Direction.eq("sell"))
                .all(self.db.as_ref())
                .await
                .map(|rows| rows.iter().filter_map(|t| t.realized_pnl).sum())
                .unwrap_or(0.0);

            positions.push(PositionSummary {
                stock_code: h.stock_code.clone(),
                stock_name: h.stock_name.clone(),
                total_shares: h.shares as i32,
                avg_cost: h.avg_cost,
                current_price,
                market_value,
                unrealized_pnl,
                unrealized_pnl_pct,
                total_realized_pnl: realized,
            });
        }
        Ok(positions)
    }

    // ── 交易历史 ──

    /// 获取交易历史
    pub async fn get_trades(
        &self,
        stock_code: Option<&str>,
        limit: u32,
    ) -> Result<Vec<trades::Model>, String> {
        let mut query = trades::Entity::find()
            .order_by_desc(trades::Column::CreatedAt)
            .limit(Some(limit as u64));

        if let Some(code) = stock_code {
            query = query.filter(trades::Column::StockCode.eq(code));
        }

        query.all(self.db.as_ref()).await.map_err(|e| e.to_string())
    }
}

// ── 内部 helper ──

/// 更新或创建持仓（买入时调用）
///
/// - 已有持仓：加权平均成本合并
/// - 新持仓：创建记录
async fn upsert_position(
    db: &DatabaseConnection,
    stock_code: &str,
    stock_name: &str,
    shares: f64,
    price: f64,
) -> Result<(), String> {
    let existing = portfolio_holdings::Entity::find()
        .filter(portfolio_holdings::Column::StockCode.eq(stock_code))
        .one(db)
        .await
        .map_err(|e| e.to_string())?;

    let now = chrono::Utc::now().timestamp_millis();

    if let Some(h) = existing {
        // 合并：加权平均成本
        let old_total = h.shares * h.avg_cost;
        let new_total = shares * price;
        let new_shares = h.shares + shares;
        let new_avg_cost = if new_shares > 0.0 {
            (old_total + new_total) / new_shares
        } else {
            price
        };

        portfolio_holdings::Entity::update_many()
            .col_expr(
                portfolio_holdings::Column::Shares,
                sea_orm::sea_query::Expr::value(new_shares),
            )
            .col_expr(
                portfolio_holdings::Column::AvgCost,
                sea_orm::sea_query::Expr::value(new_avg_cost),
            )
            .col_expr(portfolio_holdings::Column::UpdatedAt, sea_orm::sea_query::Expr::value(now))
            .filter(portfolio_holdings::Column::Id.eq(&h.id))
            .exec(db)
            .await
            .map_err(|e| e.to_string())?;
    } else {
        let id = uuid::Uuid::new_v4().to_string();
        let model = portfolio_holdings::ActiveModel {
            id: Set(id),
            stock_code: Set(stock_code.to_string()),
            stock_name: Set(stock_name.to_string()),
            shares: Set(shares),
            avg_cost: Set(price),
            notes: Set(None),
            created_at: Set(now),
            updated_at: Set(now),
        };
        model.insert(db).await.map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// 判断是否为 A 股交易日（简化版：排除周六、周日）
fn is_trading_day(date_str: &str) -> bool {
    if let Ok(d) = NaiveDate::parse_from_str(date_str, "%Y-%m-%d") {
        use chrono::Datelike;
        let weekday = d.weekday();
        weekday != chrono::Weekday::Sat && weekday != chrono::Weekday::Sun
    } else {
        true // 无法解析时默认是交易日
    }
}
