//! 手动交易日志引擎 — 用户录入买卖，自动聚合持仓、计算成本、跟踪盈亏。
//!
//! 核心功能:
//! 1. 交易校验（涨跌停、手数、持仓充足性、交易日历）
//! 2. 执行交易（写入 trades 表 + 自动更新 portfolio_holdings）
//! 3. 持仓汇总（加权平均成本 + 实时盈亏）
//! 4. 交易历史查询

use sea_orm::{
    ActiveModelTrait, ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, QueryOrder,
    QuerySelect, Set,
};
use std::sync::Arc;

use crate::decision_action::{ActionKind, normalize_action};
use axagent_astock_data::AStockClient;
use axagent_astock_data::{detect_market_type, get_st_price_limit_pct};
use axagent_entities::{portfolio_holdings, stock_analyses, trades};

// DTO 权威定义已上收 `axagent-harness`（`harness/src/trading_engine.rs`，接缝
// `trading.engine`）。此处 re-export 保留既有引用路径
// （`crate::trading::PositionSummary` / `axagent_analysis_engine::trading::*`），
// 避免改调用点；**禁止**在本文件重新定义（AGENTS 12：禁止重复定义）。
pub use axagent_harness::{PositionSummary, TradePredictionComparison, TradeValidation};

/// 手动交易引擎
pub struct TradingEngine {
    db: Arc<DatabaseConnection>,
    astock_client: Arc<AStockClient>,
    /// 交易功能开关（默认 false，需用户主动开启）
    pub enabled: bool,
}

/// 目标价偏离阈值的缺省值（百分比）。
///
/// `validate_trade`（命令层）与 `execute_trade_inner`（接缝链路）共用这一个来源：
/// 两处各写一个 `5.0` 字面量，改口径时必漏一处。
const DEFAULT_PRICE_DEVIATION_LIMIT: f64 = 5.0;

/// 接缝 `TradeExecutionOutcome::reason` 的既有取值（字符串而非常量枚举，
/// 见 `harness/src/trading_engine.rs` 的类型文档）。
const REASON_TRADING_DISABLED: &str = "trading_disabled";
const REASON_VALIDATION_FAILED: &str = "validation_failed";
const REASON_WRITE_FAILED: &str = "write_failed";

/// 旧 `execute_trade` 的开关关闭文案 —— 命令层/前端按它做人话提示，**逐字勿改**。
const TRADING_DISABLED_MESSAGE: &str = "交易功能未启用，请先在设置中开启";

/// 交易功能开关的唯一读取口径 —— settings 表的 `trading_enabled`。
///
/// 此前这份判定只内联在 `record_trade` 里，命令层没有读口 ⇒ 前端面板无法回读真实
/// 状态，重进 tab 就显示成本地默认值（界面看着关、后端仍放行）。门禁与前端都必须走本函数。
pub async fn is_trading_enabled(db: &DatabaseConnection) -> bool {
    axagent_dao::repo::settings::get_setting(db, "trading_enabled")
        .await
        .unwrap_or_default()
        .map(|s| s == "true")
        .unwrap_or(false)
}

impl TradingEngine {
    pub fn new(db: Arc<DatabaseConnection>, astock_client: Arc<AStockClient>) -> Self {
        Self { db, astock_client, enabled: false }
    }

    // ── 交易校验 ──

    /// 校验一笔交易是否合法（涨跌停、手数、持仓充足性、交易日历）
    /// 使用默认的 5% 目标价偏离阈值。
    pub async fn validate_trade(
        &self,
        stock_code: &str,
        direction: &str,
        quantity: i32,
        price: f64,
    ) -> TradeValidation {
        self.validate_trade_impl(
            stock_code,
            direction,
            quantity,
            price,
            DEFAULT_PRICE_DEVIATION_LIMIT,
        )
        .await
    }

    /// 带自定义目标价偏离阈值的交易校验。
    /// `price_deviation_limit`: 入场价偏离分析目标价的允许百分比（默认 5.0）
    pub async fn validate_trade_with_config(
        &self,
        stock_code: &str,
        direction: &str,
        quantity: i32,
        price: f64,
        price_deviation_limit: f64,
    ) -> TradeValidation {
        self.validate_trade_impl(stock_code, direction, quantity, price, price_deviation_limit)
            .await
    }

    async fn validate_trade_impl(
        &self,
        stock_code: &str,
        direction: &str,
        quantity: i32,
        price: f64,
        price_deviation_limit: f64,
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
        let min_lot: i32 = 100;
        if quantity % min_lot != 0 {
            errors.push(format!("数量必须是 {} 股的整数倍（{}手）", min_lot, min_lot));
        }

        // 涨跌停校验（优先使用 vendor 预计算的 limit_up/limit_down，回退到 pre_close 计算）
        if let Ok(quote) = self.astock_client.get_quote(stock_code).await {
            let limit_up = quote.limit_up.unwrap_or_else(|| {
                let limit_pct = get_st_price_limit_pct(quote.is_st, market);
                let ref_price = if quote.pre_close > 0.0 {
                    quote.pre_close
                } else {
                    quote.open
                };
                ref_price * (1.0 + limit_pct / 100.0)
            });
            let limit_down = quote.limit_down.unwrap_or_else(|| {
                let limit_pct = get_st_price_limit_pct(quote.is_st, market);
                let ref_price = if quote.pre_close > 0.0 {
                    quote.pre_close
                } else {
                    quote.open
                };
                ref_price * (1.0 - limit_pct / 100.0)
            });

            if direction == "buy" && price > limit_up {
                errors.push(format!("买入价 {:.2} 超过涨停价 {:.2}", price, limit_up));
            }
            if direction == "buy" && price < limit_down {
                warnings.push(format!(
                    "买入价 {:.2} 低于跌停价 {:.2}，可能难以成交",
                    price, limit_down
                ));
            }
            if direction == "sell" && price < limit_down {
                errors.push(format!("卖出价 {:.2} 低于跌停价 {:.2}", price, limit_down));
            }
            if direction == "sell" && price > limit_up {
                warnings
                    .push(format!("卖出价 {:.2} 超过涨停价 {:.2}，可能难以成交", price, limit_up));
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

            if let Some(analysis) = last_analysis
                && let Some(ref decision_json) = analysis.decision_json
                && let Ok(decision) = serde_json::from_str::<serde_json::Value>(decision_json)
            {
                let suggested_action = decision["action"].as_str().unwrap_or("");
                let suggested_target = decision["targetPrice"].as_f64();

                // P1-6(2026-09-14): 原先只认 2 个中文字面量 —— 英文 SELL
                // 不触发「分析建议卖出而非买入」二次确认（漏报）。
                if matches!(
                    normalize_action(suggested_action),
                    Some(ActionKind::Reduce | ActionKind::Sell)
                ) {
                    warnings.push(format!("分析建议「{}」而非买入，请二次确认", suggested_action));
                }

                if let Some(target) = suggested_target {
                    let deviation = ((price - target) / target).abs() * 100.0;
                    if deviation > price_deviation_limit {
                        warnings.push(format!(
                            "入场价 {:.2} 偏离分析目标价 {:.2} ({:.1}%)",
                            price, target, deviation
                        ));
                    }
                }
            }
        }

        // 仓位上限检查（买入时）
        if direction == "buy" {
            // 仓位三条走面板（`pos_max_*`，回落 20/10/40 == 接线前的 `Default`）。
            let limits = crate::position_limits::PositionLimits::panel_effective();
            let positions = self.get_positions().await.unwrap_or_default();
            let current_count = positions.len();
            let total_mv: f64 = positions.iter().map(|p| p.market_value.unwrap_or(0.0)).sum();
            let new_position_value = price * quantity as f64;
            if let Err(e) =
                limits.check_new_position(new_position_value, total_mv, current_count, None, &[])
            {
                warnings.push(format!("仓位限制: {}", e));
            }
        }

        // 交易日历检查
        let today = chrono::Utc::now().format("%Y-%m-%d").to_string();
        if !is_trading_day(&today) {
            warnings.push("当前非交易日，已记录但仅供参考".to_string());
        }

        TradeValidation { valid: errors.is_empty(), errors, warnings }
    }

    // ── 执行交易 ──

    /// 执行一笔交易（写入 trades 表 + 自动更新 portfolio_holdings）
    ///
    /// - 买入：更新持仓（加权平均成本），若已持有则合并且权重平均成本
    /// - 卖出：计算已实现盈亏，减仓或清仓
    /// - 需要 settings 的 `trading_enabled = true` 且通过校验才执行
    ///   （`self.enabled` 是死字段、恒 false，实际只由 DB 决定）
    ///
    /// ⚠ 本方法**保留给命令层**（`record_trade`）：失败一律折成 `Err(String)`。
    /// 接缝 `trading.engine` 走 [`Self::execute_trade_outcome`]，把业务拒绝做成数据。
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
        analysis_id: Option<&str>,
    ) -> Result<trades::Model, String> {
        let request = axagent_harness::TradeExecutionRequest {
            stock_code: stock_code.to_string(),
            stock_name: stock_name.to_string(),
            direction: direction.to_string(),
            price,
            quantity,
            trade_date: trade_date.to_string(),
            trade_time: trade_time.to_string(),
            notes: notes.map(str::to_string),
            analysis_id: analysis_id.map(str::to_string),
        };
        match self.execute_trade_inner(&request, DEFAULT_PRICE_DEVIATION_LIMIT).await {
            Ok((model, _)) => Ok(model),
            // 逐字保留旧文案（命令层 / 前端按它做人话提示，改文案等于改语义）
            Err(ExecuteFailure::Disabled) => Err(TRADING_DISABLED_MESSAGE.to_string()),
            // 旧行为：errors 用 "; " 拼成一条串，warnings 丢弃 —— 勿改
            Err(ExecuteFailure::Validation(validation)) => Err(validation.errors.join("; ")),
            Err(ExecuteFailure::Write(e)) => Err(e),
        }
    }

    /// 结构化下单（接缝 `trading.engine` 的 `execute_trade` 用）。
    ///
    /// 「开关关闭 / 校验不过 / 落库失败」都是**数据**（`accepted = false` + `reason`），
    /// 不借用错误码；与旧 [`Self::execute_trade`] 共用同一条内层链路
    /// （[`Self::execute_trade_inner`]），差异只有三处：
    ///
    /// 1. **只校验一次** —— 内层直接调 `validate_trade_impl`，不再「先 validate 再 execute」
    ///    各取一遍行情（`validate_trade_impl` 本身含取行情 + `get_positions` 的 N+1，
    ///    调两次等于把成本翻倍）；
    /// 2. 校验失败时把 `errors` / `warnings` **原样带回**，而不是 `join("; ")` 成一条串；
    /// 3. 落库失败返回 `accepted = false` + `reason = "write_failed"` + 错误明细，
    ///    既不 `Err` 也不 panic。
    pub async fn execute_trade_outcome(
        &self,
        request: axagent_harness::TradeExecutionRequest,
    ) -> axagent_harness::TradeExecutionOutcome {
        match self.execute_trade_inner(&request, DEFAULT_PRICE_DEVIATION_LIMIT).await {
            Ok((model, validation)) => axagent_harness::TradeExecutionOutcome {
                accepted: true,
                reason: None,
                trade: Some(trade_view(&model)),
                // 受理时 errors 恒空（valid ⇒ errors 为空）
                errors: Vec::new(),
                warnings: validation.warnings,
            },
            // 开关关闭时引擎**不跑校验** ⇒ errors / warnings 都为空
            Err(ExecuteFailure::Disabled) => {
                rejection(REASON_TRADING_DISABLED, Vec::new(), Vec::new())
            },
            Err(ExecuteFailure::Validation(validation)) => axagent_harness::TradeExecutionOutcome {
                accepted: false,
                reason: Some(REASON_VALIDATION_FAILED.to_string()),
                trade: None,
                errors: validation.errors,
                warnings: validation.warnings,
            },
            Err(ExecuteFailure::Write(e)) => rejection(REASON_WRITE_FAILED, vec![e], Vec::new()),
        }
    }

    /// 开关 → 校验 → 落库的唯一链路（旧/新两个公开入口共用，避免逻辑复制）。
    ///
    /// 返回 `(落库后的记录, 校验结果)`：旧入口只取模型，接缝入口还要 `warnings`。
    async fn execute_trade_inner(
        &self,
        request: &axagent_harness::TradeExecutionRequest,
        price_deviation_limit: f64,
    ) -> Result<(trades::Model, TradeValidation), ExecuteFailure> {
        // ① 开关闸：settings 的 `trading_enabled` 是唯一权威（`self.enabled` 是死字段）；
        //    保留 `!self.enabled &&` 只为与旧实现逐字同义（OR 语义）。
        let enabled = is_trading_enabled(self.db.as_ref()).await;
        if !self.enabled && !enabled {
            return Err(ExecuteFailure::Disabled);
        }

        // ② 校验**只做一次**
        let validation = self
            .validate_trade_impl(
                &request.stock_code,
                &request.direction,
                request.quantity,
                request.price,
                price_deviation_limit,
            )
            .await;
        if !validation.valid {
            return Err(ExecuteFailure::Validation(validation));
        }

        // ③ 落库（trades 先写、持仓后改，与既有 R1 取舍一致）
        let model = self.persist_trade(request).await.map_err(ExecuteFailure::Write)?;
        Ok((model, validation))
    }

    /// 落库：写 `trades` + 更新 `portfolio_holdings`（无事务，与既有实现逐字同序）。
    ///
    /// `request.analysis_id` 仍然**被丢弃** —— `trades` 实体没有该列，接缝照实透传但不承诺回读。
    async fn persist_trade(
        &self,
        request: &axagent_harness::TradeExecutionRequest,
    ) -> Result<trades::Model, String> {
        let now = chrono::Utc::now().timestamp_millis();
        let trade_id = uuid::Uuid::new_v4().to_string();
        let mut realized_pnl = None;

        // R1-修复: 调整操作顺序为"先写交易记录，再更新持仓"。
        //   原顺序（先持仓后交易）在 trades 写入失败时会导致持仓已变更但无审计记录。
        //   新顺序确保交易审计数据优先保存，持仓更新失败可通过对账修复。
        // 卖出时先读取持仓计算 realized_pnl（不修改），供交易记录使用。
        if request.direction == "sell" {
            let holdings = portfolio_holdings::Entity::find()
                .filter(portfolio_holdings::Column::StockCode.eq(&request.stock_code))
                .one(self.db.as_ref())
                .await
                .map_err(|e| e.to_string())?;

            if let Some(h) = holdings {
                let sell_qty = request.quantity as f64;
                let cost = h.avg_cost * sell_qty;
                let revenue = request.price * sell_qty;
                realized_pnl = Some(revenue - cost);
            }
        }

        // 写入交易记录（审计优先）
        let trade = trades::ActiveModel {
            id: Set(trade_id),
            stock_code: Set(request.stock_code.clone()),
            stock_name: Set(request.stock_name.clone()),
            direction: Set(request.direction.clone()),
            price: Set(request.price),
            quantity: Set(request.quantity),
            trade_date: Set(request.trade_date.clone()),
            trade_time: Set(request.trade_time.clone()),
            fee: Set(None),
            realized_pnl: Set(realized_pnl),
            // R2-遗留修复: trades entity 有 strategy 字段,之前漏写导致 cargo check E0063。
            // 用户手动 record_trade 走的是 stock_analysis command,不带 strategy 上下文,
            // 与 trade_import.rs:412 的 Set(None) 保持一致(手动录入不属于任何自动策略)。
            strategy: Set(None),
            notes: Set(request.notes.clone()),
            // analysis_id 当前未持久化（trades entity 暂未挂该字段，留作未来扩展）
            created_at: Set(now),
        };

        let trade_model = trade.insert(self.db.as_ref()).await.map_err(|e| e.to_string())?;

        // 交易记录写入成功后再更新持仓（持仓更新失败不丢失审计数据）
        if request.direction == "buy" {
            upsert_position(
                &self.db,
                &request.stock_code,
                &request.stock_name,
                request.quantity as f64,
                request.price,
            )
            .await?;
        } else {
            // sell: 减仓或清仓
            let holdings = portfolio_holdings::Entity::find()
                .filter(portfolio_holdings::Column::StockCode.eq(&request.stock_code))
                .one(self.db.as_ref())
                .await
                .map_err(|e| e.to_string())?;

            if let Some(h) = holdings {
                let remaining = h.shares - request.quantity as f64;
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

        Ok(trade_model)
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

            // 查询行业分类
            let sector_name = self
                .astock_client
                .get_sector_info(&h.stock_code)
                .await
                .ok()
                .flatten()
                .map(|s| s.sector_name)
                .filter(|n| !n.is_empty());

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
                sector_name,
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

    // ── 出场价 vs 分析预测对比 ──

    /// 对比实际交易出场价与最近分析预测价位。
    ///
    /// ⚠ 「交易前没有已完成分析」在本方法里是 `Err`（既有命令层语义，**勿改**）；
    /// 接缝走 [`Self::compare_trade_vs_prediction_opt`]，同一情形返回 `Ok(None)`。
    pub async fn compare_trade_vs_prediction(
        &self,
        trade: &trades::Model,
    ) -> Result<TradePredictionComparison, String> {
        self.compare_trade_impl(trade)
            .await?
            .ok_or_else(|| "没有找到在交易前的分析记录".to_string())
    }

    /// 按 `trade_id` 对比（接缝 `trading.engine` 的 `compare_trade` 用）。
    ///
    /// 「交易查无」与「交易前没有已完成分析」都是 `Ok(None)`
    /// （与 `jobs.store` / `workflow.run` 的「查无即 None」同口径，不借错误码）。
    pub async fn compare_trade_vs_prediction_opt(
        &self,
        trade_id: &str,
    ) -> Result<Option<TradePredictionComparison>, String> {
        let trade = trades::Entity::find_by_id(trade_id.to_string())
            .one(self.db.as_ref())
            .await
            .map_err(|e| e.to_string())?;
        let Some(trade) = trade else {
            return Ok(None);
        };
        self.compare_trade_impl(&trade).await
    }

    /// 对比的唯一计算体：查交易前最近一条已完成分析并算出偏离度。
    ///
    /// 「查无分析」在这里是 `Ok(None)` —— 两个公开方法各自决定折成 `Err` 还是 `None`。
    async fn compare_trade_impl(
        &self,
        trade: &trades::Model,
    ) -> Result<Option<TradePredictionComparison>, String> {
        let mut comparison = TradePredictionComparison {
            analysis_action: String::new(),
            analysis_target: None,
            analysis_stop: None,
            actual_price: trade.price,
            target_deviation_pct: 0.0,
        };

        let last_analysis = stock_analyses::Entity::find()
            .filter(stock_analyses::Column::StockCode.eq(&trade.stock_code))
            .filter(stock_analyses::Column::Status.eq("completed"))
            .filter(stock_analyses::Column::CreatedAt.lt(trade.created_at))
            .order_by_desc(stock_analyses::Column::CreatedAt)
            .one(self.db.as_ref())
            .await
            .map_err(|e| e.to_string())?;

        // 交易前没有已完成分析 ⇒ 无可比对对象（业务上的「无」，不是故障）
        let Some(last_analysis) = last_analysis else {
            return Ok(None);
        };

        if let Some(ref decision_json) = last_analysis.decision_json
            && let Ok(decision) = serde_json::from_str::<serde_json::Value>(decision_json)
        {
            comparison.analysis_action = decision["action"].as_str().unwrap_or("").to_string();
            comparison.analysis_target = decision["targetPrice"].as_f64();
            comparison.analysis_stop = decision["stopLoss"].as_f64();

            if let Some(target) = comparison.analysis_target
                && target != 0.0
            {
                comparison.target_deviation_pct = ((trade.price - target) / target) * 100.0;
            }
        }

        Ok(Some(comparison))
    }
}

// ── 内部 helper ──

/// 下单内层失败类别 —— 两个公开入口各自折成自己的形态：
/// 旧入口 → `Err(String)`；接缝入口 → `accepted = false` + `reason`。
enum ExecuteFailure {
    /// settings 的 `trading_enabled` 为 false（此时**未跑**校验）。
    Disabled,
    /// 校验不过（`errors` 非空；`warnings` 可能也有）。
    Validation(TradeValidation),
    /// 落库失败（trades 插入 / 持仓更新），值为引擎原始错误串。
    Write(String),
}

/// 组装一个「无 trade、无 warnings」的拒绝结果（三类拒绝里有两类是纯 reason）。
fn rejection(
    reason: &str,
    errors: Vec<String>,
    warnings: Vec<String>,
) -> axagent_harness::TradeExecutionOutcome {
    axagent_harness::TradeExecutionOutcome {
        accepted: false,
        reason: Some(reason.to_string()),
        trade: None,
        errors,
        warnings,
    }
}

/// `trades::Model` → 接缝 `TradeView` —— ORM 类型不过线的**唯一**转换点。
///
/// 不用 `From`/`Into`：`TradeView`（harness）与 `trades::Model`（entities）对本 crate
/// 都是 foreign 类型，`impl From<…> for …` 违反孤儿规则。宿主 provider 的 `get_trades`
/// 复用本函数，避免第二份字段映射漂移。
pub fn trade_view(model: &trades::Model) -> axagent_harness::TradeView {
    axagent_harness::TradeView {
        id: model.id.clone(),
        stock_code: model.stock_code.clone(),
        stock_name: model.stock_name.clone(),
        direction: model.direction.clone(),
        price: model.price,
        quantity: model.quantity,
        trade_date: model.trade_date.clone(),
        trade_time: model.trade_time.clone(),
        fee: model.fee,
        realized_pnl: model.realized_pnl,
        strategy: model.strategy.clone(),
        notes: model.notes.clone(),
        created_at: model.created_at,
    }
}

/// 更新或创建持仓（买入时调用）
///
/// - 已有持仓：加权平均成本合并
/// - 新持仓：创建记录
/// - 注意：SQLite 单写入序列化已提供基础并发保护；桌面单用户场景下竞态概率极低
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
        let model = portfolio_holdings::ActiveModel {
            id: Set(uuid::Uuid::new_v4().to_string()),
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

/// 判断是否为 A 股交易日（委托给 astock-data 的交易日历，含节假日和调休）
fn is_trading_day(date_str: &str) -> bool {
    chrono::NaiveDate::parse_from_str(date_str, "%Y-%m-%d")
        .map(|d| axagent_astock_data::calendar::is_trading_day(&d))
        .unwrap_or(true)
}

#[cfg(test)]
mod tests {
    //! 本模块只测**纯函数**（不碰 DB / 网络）：`trade_view` 的字段映射与拒绝结果构造。
    //!
    //! `execute_trade_outcome` / `compare_trade_vs_prediction_opt` 都强依赖
    //! `DatabaseConnection`（前者还要读 settings + 取行情），本 crate 无测试库基建，
    //! 造一个 `TradingEngine` 需要真 sea-orm 连接 + `AStockClient`，成本远高于本批，
    //! 故不硬造 —— 见 `.tmp/res99-report.md` 的「未加测试」说明。

    use super::{REASON_TRADING_DISABLED, rejection, trade_view};
    use axagent_entities::trades;

    fn sample_model() -> trades::Model {
        trades::Model {
            id: "trade-1".into(),
            stock_code: "600519".into(),
            stock_name: "贵州茅台".into(),
            direction: "buy".into(),
            price: 1500.5,
            quantity: 100,
            trade_date: "2026-11-02".into(),
            trade_time: "14:30".into(),
            fee: Some(5.0),
            realized_pnl: None,
            strategy: Some("trend".into()),
            notes: Some("测试".into()),
            created_at: 1_700_000_000_000,
        }
    }

    #[test]
    fn trade_view_mirrors_all_thirteen_columns() {
        let model = sample_model();
        let view = trade_view(&model);

        // 逐字段等值（漏一列就会在这里红 —— 这正是本转换函数存在的理由）
        assert_eq!(view.id, model.id);
        assert_eq!(view.stock_code, model.stock_code);
        assert_eq!(view.stock_name, model.stock_name);
        assert_eq!(view.direction, model.direction);
        assert_eq!(view.price, model.price);
        assert_eq!(view.quantity, model.quantity);
        assert_eq!(view.trade_date, model.trade_date);
        assert_eq!(view.trade_time, model.trade_time);
        assert_eq!(view.fee, model.fee);
        assert_eq!(view.realized_pnl, model.realized_pnl);
        assert_eq!(view.strategy, model.strategy);
        assert_eq!(view.notes, model.notes);
        assert_eq!(view.created_at, model.created_at);

        // 过线形状必须是 camelCase（插件按 JS 键名读）
        let val = serde_json::to_value(&view).expect("TradeView 应可序列化");
        assert_eq!(val["stockCode"], serde_json::json!("600519"));
        assert_eq!(val["tradeDate"], serde_json::json!("2026-11-02"));
        assert_eq!(val["tradeTime"], serde_json::json!("14:30"));
        assert_eq!(val["createdAt"], serde_json::json!(1_700_000_000_000_i64));
        // 实体没有 analysis_id ⇒ 视图也不得凭空造
        assert!(val.get("analysisId").is_none());
    }

    #[test]
    fn rejection_is_data_not_error() {
        let out = rejection(REASON_TRADING_DISABLED, Vec::new(), Vec::new());
        assert!(!out.accepted);
        assert_eq!(out.reason.as_deref(), Some("trading_disabled"));
        assert!(out.trade.is_none());
        assert!(out.errors.is_empty());
        assert!(out.warnings.is_empty());
    }
}
