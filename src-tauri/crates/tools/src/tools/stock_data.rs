use crate::{Tool, ToolCategory, ToolContext, ToolError, ToolResult};
use async_trait::async_trait;
use axagent_astock_data::AStockClient;
use chrono::Local;
use serde_json::{Value, json};
use std::sync::Arc;

fn te(msg: String) -> ToolError {
    ToolError::execution_failed(msg)
}

// ── 1. StockQuoteTool ──
pub struct StockQuoteTool {
    pub client: Arc<AStockClient>,
}
impl StockQuoteTool {
    pub fn new(c: Arc<AStockClient>) -> Self {
        Self { client: c }
    }
}
#[async_trait]
impl Tool for StockQuoteTool {
    fn name(&self) -> &str {
        "get_stock_quote"
    }
    fn description(&self) -> &str {
        "获取A股实时行情：现价、涨跌幅、PE、PB、市值"
    }
    fn input_schema(&self) -> Value {
        json!({"type":"object","properties":{"stock_code":{"type":"string","description":"6位股票代码"}},"required":["stock_code"]})
    }
    fn category(&self) -> ToolCategory {
        ToolCategory::Finance
    }
    async fn call(&self, input: Value, _ctx: &ToolContext) -> Result<ToolResult, ToolError> {
        let code = input["stock_code"]
            .as_str()
            .ok_or_else(|| te("stock_code不能为空".into()))?;
        let r = self
            .client
            .get_quote(code)
            .await
            .map_err(|e| te(e.to_string()))?;
        Ok(ToolResult::success(serde_json::to_string(&r).unwrap_or_default()))
    }
}

// ── 2. StockKlineTool ──
pub struct StockKlineTool {
    pub client: Arc<AStockClient>,
}
impl StockKlineTool {
    pub fn new(c: Arc<AStockClient>) -> Self {
        Self { client: c }
    }
}
#[async_trait]
impl Tool for StockKlineTool {
    fn name(&self) -> &str {
        "get_stock_kline"
    }
    fn description(&self) -> &str {
        "获取A股历史K线"
    }
    fn input_schema(&self) -> Value {
        json!({"type":"object","properties":{"stock_code":{"type":"string"},"period":{"type":"string","description":"daily/weekly/monthly"},"limit":{"type":"integer","description":"数量","default":120}},"required":["stock_code"]})
    }
    fn category(&self) -> ToolCategory {
        ToolCategory::Finance
    }
    async fn call(&self, input: Value, _ctx: &ToolContext) -> Result<ToolResult, ToolError> {
        let code = input["stock_code"]
            .as_str()
            .ok_or_else(|| te("stock_code不能为空".into()))?;
        let period = input["period"].as_str().unwrap_or("daily");
        let limit = input["limit"].as_u64().unwrap_or(120) as u32;
        let r = self
            .client
            .get_klines(code, period, limit)
            .await
            .map_err(|e| te(e.to_string()))?;
        Ok(ToolResult::success(serde_json::to_string(&r).unwrap_or_default()))
    }
}

// ── 3. StockFinancialsTool ──
pub struct StockFinancialsTool {
    pub client: Arc<AStockClient>,
}
impl StockFinancialsTool {
    pub fn new(c: Arc<AStockClient>) -> Self {
        Self { client: c }
    }
}
#[async_trait]
impl Tool for StockFinancialsTool {
    fn name(&self) -> &str {
        "get_stock_financials"
    }
    fn description(&self) -> &str {
        "获取A股财务报表"
    }
    fn input_schema(&self) -> Value {
        json!({"type":"object","properties":{"stock_code":{"type":"string"}},"required":["stock_code"]})
    }
    fn category(&self) -> ToolCategory {
        ToolCategory::Finance
    }
    async fn call(&self, input: Value, _ctx: &ToolContext) -> Result<ToolResult, ToolError> {
        let code = input["stock_code"]
            .as_str()
            .ok_or_else(|| te("stock_code不能为空".into()))?;
        let r = self
            .client
            .get_financials(code)
            .await
            .map_err(|e| te(e.to_string()))?;
        Ok(ToolResult::success(serde_json::to_string(&r).unwrap_or_default()))
    }
}

// ── 4. StockNewsTool ──
pub struct StockNewsTool {
    pub client: Arc<AStockClient>,
}
impl StockNewsTool {
    pub fn new(c: Arc<AStockClient>) -> Self {
        Self { client: c }
    }
}
#[async_trait]
impl Tool for StockNewsTool {
    fn name(&self) -> &str {
        "get_stock_news"
    }
    fn description(&self) -> &str {
        "获取A股新闻公告"
    }
    fn input_schema(&self) -> Value {
        json!({"type":"object","properties":{"stock_code":{"type":"string"},"limit":{"type":"integer","default":30}},"required":["stock_code"]})
    }
    fn category(&self) -> ToolCategory {
        ToolCategory::Finance
    }
    async fn call(&self, input: Value, _ctx: &ToolContext) -> Result<ToolResult, ToolError> {
        let code = input["stock_code"]
            .as_str()
            .ok_or_else(|| te("stock_code不能为空".into()))?;
        let limit = input["limit"].as_u64().unwrap_or(30) as u32;
        let r = self
            .client
            .get_news(code, limit)
            .await
            .map_err(|e| te(e.to_string()))?;
        Ok(ToolResult::success(serde_json::to_string(&r).unwrap_or_default()))
    }
}

// ── 5. StockMoneyFlowTool ──
pub struct StockMoneyFlowTool {
    pub client: Arc<AStockClient>,
}
impl StockMoneyFlowTool {
    pub fn new(c: Arc<AStockClient>) -> Self {
        Self { client: c }
    }
}
#[async_trait]
impl Tool for StockMoneyFlowTool {
    fn name(&self) -> &str {
        "get_stock_money_flow"
    }
    fn description(&self) -> &str {
        "获取A股资金流向"
    }
    fn input_schema(&self) -> Value {
        json!({"type":"object","properties":{"stock_code":{"type":"string"}},"required":["stock_code"]})
    }
    fn category(&self) -> ToolCategory {
        ToolCategory::Finance
    }
    async fn call(&self, input: Value, _ctx: &ToolContext) -> Result<ToolResult, ToolError> {
        let code = input["stock_code"]
            .as_str()
            .ok_or_else(|| te("stock_code不能为空".into()))?;
        let r = self
            .client
            .get_money_flow(code)
            .await
            .map_err(|e| te(e.to_string()))?;
        Ok(ToolResult::success(serde_json::to_string(&r).unwrap_or_default()))
    }
}

// ── 6. StockHotStocksTool ──
pub struct StockHotStocksTool {
    pub client: Arc<AStockClient>,
}
impl StockHotStocksTool {
    pub fn new(c: Arc<AStockClient>) -> Self {
        Self { client: c }
    }
}
#[async_trait]
impl Tool for StockHotStocksTool {
    fn name(&self) -> &str {
        "get_hot_stocks"
    }
    fn description(&self) -> &str {
        "获取当前热门A股榜单"
    }
    fn input_schema(&self) -> Value {
        json!({"type":"object","properties":{}})
    }
    fn category(&self) -> ToolCategory {
        ToolCategory::Finance
    }
    async fn call(&self, _input: Value, _ctx: &ToolContext) -> Result<ToolResult, ToolError> {
        let r = self
            .client
            .get_hot_stocks()
            .await
            .map_err(|e| te(e.to_string()))?;
        Ok(ToolResult::success(serde_json::to_string(&r).unwrap_or_default()))
    }
}

// ── 7. StockIndustryRankTool ──
pub struct StockIndustryRankTool {
    pub client: Arc<AStockClient>,
}
impl StockIndustryRankTool {
    pub fn new(c: Arc<AStockClient>) -> Self {
        Self { client: c }
    }
}
#[async_trait]
impl Tool for StockIndustryRankTool {
    fn name(&self) -> &str {
        "get_industry_ranking"
    }
    fn description(&self) -> &str {
        "获取申万行业板块涨跌排名"
    }
    fn input_schema(&self) -> Value {
        json!({"type":"object","properties":{}})
    }
    fn category(&self) -> ToolCategory {
        ToolCategory::Finance
    }
    async fn call(&self, _input: Value, _ctx: &ToolContext) -> Result<ToolResult, ToolError> {
        let r = self
            .client
            .get_industry_ranking()
            .await
            .map_err(|e| te(e.to_string()))?;
        Ok(ToolResult::success(serde_json::to_string(&r).unwrap_or_default()))
    }
}

// ── 8. StockAnnouncementsTool ──
pub struct StockAnnouncementsTool {
    pub client: Arc<AStockClient>,
}
impl StockAnnouncementsTool {
    pub fn new(c: Arc<AStockClient>) -> Self {
        Self { client: c }
    }
}
#[async_trait]
impl Tool for StockAnnouncementsTool {
    fn name(&self) -> &str {
        "get_announcements"
    }
    fn description(&self) -> &str {
        "获取A股公司公告/披露文件"
    }
    fn input_schema(&self) -> Value {
        json!({"type":"object","properties":{"stock_code":{"type":"string"}},"required":["stock_code"]})
    }
    fn category(&self) -> ToolCategory {
        ToolCategory::Finance
    }
    async fn call(&self, input: Value, _ctx: &ToolContext) -> Result<ToolResult, ToolError> {
        let code = input["stock_code"]
            .as_str()
            .ok_or_else(|| te("stock_code不能为空".into()))?;
        let r = self
            .client
            .get_announcements(code)
            .await
            .map_err(|e| te(e.to_string()))?;
        Ok(ToolResult::success(serde_json::to_string(&r).unwrap_or_default()))
    }
}

// ── 9. StockConsensusEPSTool ──
pub struct StockConsensusEPSTool {
    pub client: Arc<AStockClient>,
}
impl StockConsensusEPSTool {
    pub fn new(c: Arc<AStockClient>) -> Self {
        Self { client: c }
    }
}
#[async_trait]
impl Tool for StockConsensusEPSTool {
    fn name(&self) -> &str {
        "get_consensus_eps"
    }
    fn description(&self) -> &str {
        "获取分析师一致预期EPS预测"
    }
    fn input_schema(&self) -> Value {
        json!({"type":"object","properties":{"stock_code":{"type":"string"}},"required":["stock_code"]})
    }
    fn category(&self) -> ToolCategory {
        ToolCategory::Finance
    }
    async fn call(&self, input: Value, _ctx: &ToolContext) -> Result<ToolResult, ToolError> {
        let code = input["stock_code"]
            .as_str()
            .ok_or_else(|| te("stock_code不能为空".into()))?;
        let r = self
            .client
            .get_consensus_eps(code)
            .await
            .map_err(|e| te(e.to_string()))?;
        Ok(ToolResult::success(serde_json::to_string(&r).unwrap_or_default()))
    }
}

// ── 10. SearchStockTool ──
pub struct SearchStockTool {
    pub client: Arc<AStockClient>,
}
impl SearchStockTool {
    pub fn new(c: Arc<AStockClient>) -> Self {
        Self { client: c }
    }
}
#[async_trait]
impl Tool for SearchStockTool {
    fn name(&self) -> &str {
        "search_stock"
    }
    fn description(&self) -> &str {
        "按代码或名称模糊搜索A股"
    }
    fn input_schema(&self) -> Value {
        json!({"type":"object","properties":{"keyword":{"type":"string","description":"股票代码或名称"}},"required":["keyword"]})
    }
    fn category(&self) -> ToolCategory {
        ToolCategory::Finance
    }
    async fn call(&self, input: Value, _ctx: &ToolContext) -> Result<ToolResult, ToolError> {
        let kw = input["keyword"]
            .as_str()
            .ok_or_else(|| te("keyword不能为空".into()))?;
        let r = self
            .client
            .search_stock(kw)
            .await
            .map_err(|e| te(e.to_string()))?;
        Ok(ToolResult::success(serde_json::to_string(&r).unwrap_or_default()))
    }
}

pub struct ComputeScoringTool {
    pub client: Arc<AStockClient>,
}
impl ComputeScoringTool {
    pub fn new(c: Arc<AStockClient>) -> Self {
        Self { client: c }
    }
}
#[async_trait]
impl Tool for ComputeScoringTool {
    fn name(&self) -> &str {
        "compute_scoring"
    }
    fn description(&self) -> &str {
        "六维度技术评分：趋势/乖离/MACD/量能/RSI/支撑+价值修正"
    }
    fn input_schema(&self) -> Value {
        json!({"type":"object","properties":{"stock_code":{"type":"string","description":"6位股票代码"}},"required":["stock_code"]})
    }
    fn category(&self) -> ToolCategory {
        ToolCategory::Finance
    }
    async fn call(&self, input: Value, _ctx: &ToolContext) -> Result<ToolResult, ToolError> {
        let code = input["stock_code"]
            .as_str()
            .ok_or_else(|| te("stock_code不能为空".into()))?;
        let (klines_r, quote_r) =
            tokio::join!(self.client.get_klines(code, "daily", 120), self.client.get_quote(code));
        let klines = klines_r.map_err(|e| te(e.to_string()))?;
        let quote = quote_r.map_err(|e| te(e.to_string()))?;
        let indicators = axagent_astock_data::indicators::compute_indicators(code, &klines);

        let trend_score = match indicators.ma_alignment.as_str() {
            "多头排列" => 90.0,
            "弱多头" => 70.0,
            "缠绕/交叉" => 50.0,
            "空头排列" => 30.0,
            _ => 50.0,
        };

        let bias_avg = (indicators.bias_ma5.abs() + indicators.bias_ma20.abs()) / 2.0;
        let deviation_score = if bias_avg < 2.0 {
            80.0
        } else if bias_avg < 5.0 {
            60.0
        } else if bias_avg < 10.0 {
            40.0
        } else {
            20.0
        };

        let macd_score = match indicators.macd_signal.as_str() {
            "金叉" => 90.0,
            "多头运行" => 70.0,
            "死叉" => 30.0,
            "空头运行" => 20.0,
            _ => 50.0,
        };

        let volume_score = match indicators.volume_signal.as_str() {
            "放量上涨" => 90.0,
            "缩量回调" => 60.0,
            "正常" => 50.0,
            "缩量上涨" => 40.0,
            "放量下跌" => 20.0,
            _ => 50.0,
        };

        let rsi_score = if indicators.rsi6 > 80.0 {
            25.0
        } else if indicators.rsi6 > 70.0 {
            45.0
        } else if indicators.rsi6 > 50.0 {
            75.0
        } else if indicators.rsi6 > 30.0 {
            55.0
        } else {
            80.0
        };

        let support_score =
            if !indicators.support_levels.is_empty() && !indicators.resistance_levels.is_empty() {
                let dist_support = (quote.price - indicators.support_levels[0]).abs();
                let dist_resist = (indicators.resistance_levels[0] - quote.price).abs();
                let total = dist_support + dist_resist;
                if total > 0.0 {
                    (dist_support / total) * 100.0
                } else {
                    50.0
                }
            } else {
                50.0
            };

        let mut value_adj = 0.0;
        if let Some(pe) = quote.pe {
            if pe > 0.0 && pe < 15.0 {
                value_adj = 10.0;
            } else if (15.0..30.0).contains(&pe) {
                value_adj = 5.0;
            } else if pe >= 50.0 {
                value_adj = -10.0;
            }
        }
        if let Some(pb) = quote.pb {
            if pb > 0.0 && pb < 1.0 {
                value_adj += 10.0;
            } else if (1.0..3.0).contains(&pb) {
                value_adj += 5.0;
            } else if pb >= 6.0 {
                value_adj -= 5.0;
            }
        }

        let weights = [0.20, 0.10, 0.20, 0.15, 0.15, 0.20];
        let scores = [
            trend_score,
            deviation_score,
            macd_score,
            volume_score,
            rsi_score,
            support_score,
        ];
        let weighted: f64 = scores.iter().zip(weights.iter()).map(|(s, w)| s * w).sum();
        let total_score = (weighted + value_adj).clamp(0.0, 100.0);

        let mut warnings: Vec<Value> = Vec::new();
        if indicators.bias_ma5.is_nan() || indicators.bias_ma20.is_nan() {
            warnings.push(Value::String("BIAS指标异常：MA数据不足".into()));
        }
        if quote.price <= 0.0 {
            warnings.push(Value::String("当前价格无效（可能停牌或未开盘）".into()));
        }
        let freshness = if indicators
            .latest_date
            .contains(&Local::now().format("%Y-%m-%d").to_string())
        {
            "today"
        } else if quote.price > 0.0 {
            "delayed"
        } else {
            "stale"
        };

        let rating = if total_score >= 80.0 {
            "强烈推荐"
        } else if total_score >= 65.0 {
            "推荐"
        } else if total_score >= 50.0 {
            "中性"
        } else if total_score >= 35.0 {
            "谨慎"
        } else {
            "回避"
        };

        let result = json!({
            "stockCode": code,
            "stockName": quote.name,
            "latestDate": indicators.latest_date,
            "totalScore": (total_score * 10.0).round() / 10.0,
            "rating": rating,
            "dimensions": {
                "trend": {"score": trend_score, "detail": indicators.ma_alignment},
                "deviation": {"score": deviation_score, "detail": format!("BIAS5={:.1}% BIAS20={:.1}%", indicators.bias_ma5, indicators.bias_ma20)},
                "macd": {"score": macd_score, "detail": indicators.macd_signal},
                "volume": {"score": volume_score, "detail": format!("{} 量比={:.2}", indicators.volume_signal, indicators.volume_ratio)},
                "rsi": {"score": rsi_score, "detail": format!("RSI6={:.1} RSI12={:.1} RSI24={:.1} {}", indicators.rsi6, indicators.rsi12, indicators.rsi24, indicators.rsi_signal)},
                "support": {"score": support_score, "detail": format!("支撑={:?} 压力={:?} 布林={}", indicators.support_levels, indicators.resistance_levels, indicators.boll_position)},
            },
            "valueAdjustment": value_adj,
            "indicators": indicators,
            "credibility": {
                "dataCompleteness": 100.0,
                "dataFreshness": freshness,
                "source": "tencent|eastmoney",
                "warnings": Value::Array(warnings)
            },
        });
        Ok(ToolResult::success(serde_json::to_string(&result).unwrap_or_default()))
    }
}

pub struct ComputeValuationTool {
    pub client: Arc<AStockClient>,
}
impl ComputeValuationTool {
    pub fn new(c: Arc<AStockClient>) -> Self {
        Self { client: c }
    }
}
#[async_trait]
impl Tool for ComputeValuationTool {
    fn name(&self) -> &str {
        "compute_valuation"
    }
    fn description(&self) -> &str {
        "DCF两阶段+格雷厄姆+F-Score+护城河量化估值"
    }
    fn input_schema(&self) -> Value {
        json!({"type":"object","properties":{"stock_code":{"type":"string","description":"6位股票代码"}},"required":["stock_code"]})
    }
    fn category(&self) -> ToolCategory {
        ToolCategory::Finance
    }
    async fn call(&self, input: Value, _ctx: &ToolContext) -> Result<ToolResult, ToolError> {
        let code = input["stock_code"]
            .as_str()
            .ok_or_else(|| te("stock_code不能为空".into()))?;
        let (quote_r, financials_r) =
            tokio::join!(self.client.get_quote(code), self.client.get_financials(code));
        let quote = quote_r.map_err(|e| te(e.to_string()))?;
        let financials = financials_r.map_err(|e| te(e.to_string()))?;

        let latest_fin = financials.first();
        let eps = latest_fin.and_then(|f| f.eps).unwrap_or(0.0);
        let bps = latest_fin.and_then(|f| f.bps).unwrap_or(0.0);
        let roe = latest_fin.and_then(|f| f.roe).unwrap_or(0.0);
        let debt_ratio = latest_fin.and_then(|f| f.debt_ratio).unwrap_or(0.0);
        let gross_margin = latest_fin.and_then(|f| f.gross_margin).unwrap_or(0.0);
        let net_margin = latest_fin.and_then(|f| f.net_margin).unwrap_or(0.0);
        let revenue_yoy = latest_fin.and_then(|f| f.revenue_yoy).unwrap_or(0.0);
        let profit_yoy = latest_fin.and_then(|f| f.profit_yoy).unwrap_or(0.0);

        let discount_rate = 0.10;
        let growth_high = if revenue_yoy > 0.0 {
            revenue_yoy.min(0.30)
        } else {
            0.05
        };
        let growth_stable = 0.03;
        let high_years = 5.0;
        let terminal_multiple = 15.0;

        let mut dcf_value: f64 = 0.0;
        if eps > 0.0 {
            let mut pv: f64 = 0.0;
            for year in 1..=(high_years as i32) {
                let projected_eps = eps * (1.0_f64 + growth_high).powi(year);
                pv += projected_eps / (1.0_f64 + discount_rate).powi(year);
            }
            let terminal_eps =
                eps * (1.0_f64 + growth_high).powi(high_years as i32) * (1.0_f64 + growth_stable);
            let terminal_value = terminal_eps * terminal_multiple
                / (1.0_f64 + discount_rate).powi(high_years as i32);
            dcf_value = pv + terminal_value;
        }

        let graham_value = if eps > 0.0 && bps > 0.0 {
            (22.5 * eps * bps).sqrt()
        } else {
            0.0
        };

        let mut f_score = 0i32;
        if profit_yoy > 0.0 {
            f_score += 1;
        }
        if revenue_yoy > 0.0 {
            f_score += 1;
        }
        if roe > 0.10 {
            f_score += 1;
        }
        if gross_margin > 0.30 {
            f_score += 1;
        }
        if net_margin > 0.10 {
            f_score += 1;
        }
        if debt_ratio < 0.60 {
            f_score += 1;
        }
        if debt_ratio > 0.0
            && let Some(prev) = financials.get(1)
            && let Some(prev_dr) = prev.debt_ratio
            && debt_ratio < prev_dr
        {
            f_score += 1;
        }
        if financials.len() >= 2
            && let Some(prev) = financials.get(1)
            && let Some(prev_roe) = prev.roe
            && roe > prev_roe
        {
            f_score += 1;
        }
        if let Some(pe_val) = quote.pe
            && pe_val > 0.0
            && pe_val < 20.0
        {
            f_score += 1;
        }

        let mut moat_score = 0i32;
        if gross_margin > 0.40 {
            moat_score += 3;
        } else if gross_margin > 0.25 {
            moat_score += 2;
        } else if gross_margin > 0.15 {
            moat_score += 1;
        }
        if roe > 0.15 {
            moat_score += 3;
        } else if roe > 0.10 {
            moat_score += 2;
        } else if roe > 0.05 {
            moat_score += 1;
        }
        if debt_ratio < 0.40 {
            moat_score += 2;
        } else if debt_ratio < 0.60 {
            moat_score += 1;
        }
        if profit_yoy > 0.10 {
            moat_score += 2;
        } else if profit_yoy > 0.0 {
            moat_score += 1;
        }

        let moat_label = if moat_score >= 8 {
            "强护城河"
        } else if moat_score >= 5 {
            "中等护城河"
        } else if moat_score >= 3 {
            "弱护城河"
        } else {
            "无护城河"
        };

        let current_price = quote.price;
        let dcf_upside = if dcf_value > 0.0 {
            (dcf_value - current_price) / current_price * 100.0
        } else {
            0.0
        };
        let mut v_warnings: Vec<Value> = Vec::new();
        if eps <= 0.0 {
            v_warnings.push(Value::String("EPS≤0，DCF估值不可靠".into()));
        }
        let fin_count = financials.len();
        if fin_count < 2 {
            v_warnings
                .push(Value::String(format!("财务数据仅{}期，估值模型依赖多期数据", fin_count)));
        }
        let v_freshness = if let Some(ref lf) = latest_fin {
            if lf
                .report_date
                .contains(&Local::now().format("%Y-%m").to_string())
            {
                "current_quarter"
            } else if let Ok(now) = chrono::NaiveDate::parse_from_str(
                &Local::now().format("%Y-%m-%d").to_string(),
                "%Y-%m-%d",
            ) {
                if let Some(ref rd) = lf.report_date.strip_suffix("00:00:00") {
                    if let Ok(report_date) =
                        chrono::NaiveDate::parse_from_str(rd.trim(), "%Y-%m-%d")
                    {
                        let days_old = (now - report_date).num_days();
                        if days_old <= 90 {
                            "recent_quarter"
                        } else {
                            "outdated"
                        }
                    } else {
                        "unknown"
                    }
                } else {
                    "unknown"
                }
            } else {
                "unknown"
            }
        } else {
            "no_data"
        };

        let graham_upside = if graham_value > 0.0 {
            (graham_value - current_price) / current_price * 100.0
        } else {
            0.0
        };

        let result = json!({
            "stockCode": code,
            "stockName": quote.name,
            "currentPrice": current_price,
            "dcf": {
                "intrinsicValue": (dcf_value * 100.0).round() / 100.0,
                "upsidePct": (dcf_upside * 100.0).round() / 100.0 / 100.0,
                "assumptions": {
                    "eps": eps,
                    "highGrowthRate": growth_high,
                    "stableGrowthRate": growth_stable,
                    "highGrowthYears": high_years,
                    "discountRate": discount_rate,
                    "terminalPE": terminal_multiple,
                },
            },
            "graham": {
                "intrinsicValue": (graham_value * 100.0).round() / 100.0,
                "upsidePct": (graham_upside * 100.0).round() / 100.0 / 100.0,
                "formula": "sqrt(22.5 * EPS * BPS)",
                "eps": eps,
                "bps": bps,
            },
            "fScore": {
                "score": f_score,
                "maxScore": 9,
                "interpretation": if f_score >= 7 { "财务强劲" } else if f_score >= 5 { "财务稳健" } else if f_score >= 3 { "财务一般" } else { "财务薄弱" },
            },
            "moat": {
                "score": moat_score,
                "maxScore": 10,
                "label": moat_label,
                "grossMargin": gross_margin,
                "roe": roe,
                "debtRatio": debt_ratio,
                "profitYoy": profit_yoy,
            },
            "financialsUsed": latest_fin.map(|f| f.report_date.clone()).unwrap_or_default(),
            "credibility": {
                "dataCompleteness": if fin_count >= 2 { 100.0 } else { fin_count as f64 * 50.0 },
                "dataFreshness": v_freshness,
                "source": "eastmoney",
                "warnings": Value::Array(v_warnings)
            },
        });
        Ok(ToolResult::success(serde_json::to_string(&result).unwrap_or_default()))
    }
}

pub struct ComputeRiskTool {
    pub client: Arc<AStockClient>,
}
impl ComputeRiskTool {
    pub fn new(c: Arc<AStockClient>) -> Self {
        Self { client: c }
    }
}
#[async_trait]
impl Tool for ComputeRiskTool {
    fn name(&self) -> &str {
        "compute_portfolio_risk"
    }
    fn description(&self) -> &str {
        "组合风险指标：集中度/分散度/行业暴露"
    }
    fn input_schema(&self) -> Value {
        json!({"type":"object","properties":{"stock_codes":{"type":"string","description":"逗号分隔的股票代码列表"},"weights":{"type":"string","description":"逗号分隔的持仓权重(0-1)，不填则等权"}},"required":["stock_codes"]})
    }
    fn category(&self) -> ToolCategory {
        ToolCategory::Finance
    }
    async fn call(&self, input: Value, _ctx: &ToolContext) -> Result<ToolResult, ToolError> {
        let codes_str = input["stock_codes"].as_str().unwrap_or("");
        let codes: Vec<&str> = codes_str
            .split(',')
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .collect();
        if codes.is_empty() {
            return Err(te("stock_codes不能为空".into()));
        }

        let weights: Vec<f64> = if let Some(w_str) = input["weights"].as_str() {
            let parsed: Vec<f64> = w_str
                .split(',')
                .map(|s| s.trim().parse::<f64>().unwrap_or(0.0))
                .collect();
            if parsed.len() == codes.len() {
                parsed
            } else {
                vec![1.0 / codes.len() as f64; codes.len()]
            }
        } else {
            vec![1.0 / codes.len() as f64; codes.len()]
        };

        let mut fetch_tasks = Vec::new();
        for &c in &codes {
            fetch_tasks.push(self.client.get_quote(c));
        }
        let results = futures::future::join_all(fetch_tasks).await;

        let mut positions = Vec::new();
        for (i, res) in results.into_iter().enumerate() {
            match res {
                Ok(q) => positions.push((q, weights.get(i).copied().unwrap_or(0.0))),
                Err(e) => {
                    let code = codes.get(i).unwrap_or(&"");
                    tracing::warn!("获取{}行情失败: {}", code, e);
                },
            }
        }

        if positions.is_empty() {
            return Err(te("所有股票行情获取失败".into()));
        }

        let total_weight: f64 = positions.iter().map(|(_, w)| w).sum();
        let norm_weights: Vec<f64> = if total_weight > 0.0 {
            positions.iter().map(|(_, w)| w / total_weight).collect()
        } else {
            vec![1.0 / positions.len() as f64; positions.len()]
        };

        let hhi: f64 = norm_weights.iter().map(|w| w * w).sum();
        let effective_n = if hhi > 0.0 { 1.0 / hhi } else { 1.0 };

        let mut sector_map: std::collections::HashMap<String, f64> =
            std::collections::HashMap::new();
        let sector_tasks: Vec<_> = positions
            .iter()
            .map(|(q, _)| self.client.get_sector_info(&q.code))
            .collect();
        let sector_results = futures::future::join_all(sector_tasks).await;

        for (i, sr) in sector_results.into_iter().enumerate() {
            let sector_name = if let Ok(Some(si)) = sr {
                si.sector_name.clone()
            } else {
                "未知".to_string()
            };
            let w = norm_weights.get(i).copied().unwrap_or(0.0);
            *sector_map.entry(sector_name).or_insert(0.0) += w;
        }

        let max_sector_concentration = sector_map.values().copied().fold(0.0_f64, f64::max);
        let max_sector_name = sector_map
            .iter()
            .max_by_key(|(_, v)| (*v * 10000.0) as i64)
            .map(|(k, _)| k.clone())
            .unwrap_or_default();

        let concentration_label = if hhi > 0.25 {
            "高度集中"
        } else if hhi > 0.15 {
            "中度集中"
        } else {
            "分散"
        };
        let requested_count = codes.len();
        let loaded_count = positions.len();
        let r_warnings: Vec<Value> = if requested_count > loaded_count {
            vec![Value::String(format!(
                "{}/{} 只股票行情加载失败",
                requested_count - loaded_count,
                requested_count
            ))]
        } else {
            vec![]
        };

        let diversification_label = if effective_n >= 8.0 {
            "充分分散"
        } else if effective_n >= 4.0 {
            "适度分散"
        } else {
            "集中风险"
        };

        let result = json!({
            "stockCount": positions.len(),
            "concentration": {
                "hhi": (hhi * 10000.0).round() / 10000.0,
                "effectiveN": (effective_n * 100.0).round() / 100.0,
                "label": concentration_label,
            },
            "diversification": {
                "effectiveStocks": (effective_n * 100.0).round() / 100.0,
                "label": diversification_label,
            },
            "sectorExposure": sector_map.into_iter().map(|(k, v)| json!({"sector": k, "weight": (v * 100.0).round() / 100.0})).collect::<Vec<_>>(),
            "maxSectorConcentration": {
                "sector": max_sector_name,
                "weightPct": (max_sector_concentration * 100.0).round() / 100.0,
            },
            "positions": positions.iter().zip(norm_weights.iter()).map(|((q, _), w)| json!({
                "code": q.code,
                "name": q.name,
                "price": q.price,
                "changePct": q.change_pct,
                "weightPct": (*w * 100.0).round() / 100.0,
            })).collect::<Vec<_>>(),
            "credibility": {
                "dataCompleteness": if requested_count > 0 { (loaded_count as f64 / requested_count as f64) * 100.0 } else { 0.0 },
                "dataFreshness": "realtime",
                "source": "tencent|eastmoney",
                "warnings": Value::Array(r_warnings)
            },
        });
        Ok(ToolResult::success(serde_json::to_string(&result).unwrap_or_default()))
    }
}

// ── 14. StockBlockTradesTool ──
pub struct StockBlockTradesTool {
    pub client: Arc<AStockClient>,
}
impl StockBlockTradesTool {
    pub fn new(c: Arc<AStockClient>) -> Self {
        Self { client: c }
    }
}
#[async_trait]
impl Tool for StockBlockTradesTool {
    fn name(&self) -> &str {
        "get_block_trades"
    }
    fn description(&self) -> &str {
        "获取A股大宗交易记录：成交价、成交量、买卖双方营业部、折价率"
    }
    fn input_schema(&self) -> Value {
        json!({"type":"object","properties":{"stock_code":{"type":"string","description":"6位股票代码"}},"required":["stock_code"]})
    }
    fn category(&self) -> ToolCategory {
        ToolCategory::Finance
    }
    async fn call(&self, input: Value, _ctx: &ToolContext) -> Result<ToolResult, ToolError> {
        let code = input["stock_code"]
            .as_str()
            .ok_or_else(|| te("stock_code不能为空".into()))?;
        let r = self
            .client
            .get_block_trades(code)
            .await
            .map_err(|e| te(e.to_string()))?;
        Ok(ToolResult::success(serde_json::to_string(&r).unwrap_or_default()))
    }
}

// ── 15. StockInstitutionalVisitsTool ──
pub struct StockInstitutionalVisitsTool {
    pub client: Arc<AStockClient>,
}
impl StockInstitutionalVisitsTool {
    pub fn new(c: Arc<AStockClient>) -> Self {
        Self { client: c }
    }
}
#[async_trait]
impl Tool for StockInstitutionalVisitsTool {
    fn name(&self) -> &str {
        "get_institutional_visits"
    }
    fn description(&self) -> &str {
        "获取A股机构调研记录：调研日期、机构数量、调研内容、调研方式"
    }
    fn input_schema(&self) -> Value {
        json!({"type":"object","properties":{"stock_code":{"type":"string","description":"6位股票代码"}},"required":["stock_code"]})
    }
    fn category(&self) -> ToolCategory {
        ToolCategory::Finance
    }
    async fn call(&self, input: Value, _ctx: &ToolContext) -> Result<ToolResult, ToolError> {
        let code = input["stock_code"]
            .as_str()
            .ok_or_else(|| te("stock_code不能为空".into()))?;
        let r = self
            .client
            .get_institutional_visits(code)
            .await
            .map_err(|e| te(e.to_string()))?;
        Ok(ToolResult::success(serde_json::to_string(&r).unwrap_or_default()))
    }
}

// ── 16. StockPeersTool ──
pub struct StockPeersTool {
    pub client: Arc<AStockClient>,
}
impl StockPeersTool {
    pub fn new(c: Arc<AStockClient>) -> Self {
        Self { client: c }
    }
}
#[async_trait]
impl Tool for StockPeersTool {
    fn name(&self) -> &str {
        "get_stock_peers"
    }
    fn description(&self) -> &str {
        "获取同行业可比公司估值（PE/PB/ROE/涨跌幅/市值）"
    }
    fn input_schema(&self) -> Value {
        json!({"type":"object","properties":{"stock_code":{"type":"string","description":"6位股票代码"}},"required":["stock_code"]})
    }
    fn category(&self) -> ToolCategory {
        ToolCategory::Finance
    }
    async fn call(&self, input: Value, _ctx: &ToolContext) -> Result<ToolResult, ToolError> {
        let code = input["stock_code"]
            .as_str()
            .filter(|s| !s.is_empty())
            .ok_or_else(|| te("stock_code不能为空".into()))?;
        let r = self
            .client
            .get_peers(code)
            .await
            .map_err(|e| te(e.to_string()))?;
        Ok(ToolResult::success(serde_json::to_string(&r).unwrap_or_default()))
    }
}

// ── 17. StockResearchReportsTool ──
pub struct StockResearchReportsTool {
    pub client: Arc<AStockClient>,
}
impl StockResearchReportsTool {
    pub fn new(c: Arc<AStockClient>) -> Self {
        Self { client: c }
    }
}
#[async_trait]
impl Tool for StockResearchReportsTool {
    fn name(&self) -> &str {
        "get_research_reports"
    }
    fn description(&self) -> &str {
        "获取券商研报列表（机构、评级、目标价、EPS预测）"
    }
    fn input_schema(&self) -> Value {
        json!({"type":"object","properties":{"stock_code":{"type":"string","description":"6位股票代码"}},"required":["stock_code"]})
    }
    fn category(&self) -> ToolCategory {
        ToolCategory::Finance
    }
    async fn call(&self, input: Value, _ctx: &ToolContext) -> Result<ToolResult, ToolError> {
        let code = input["stock_code"]
            .as_str()
            .filter(|s| !s.is_empty())
            .ok_or_else(|| te("stock_code不能为空".into()))?;
        let r = self
            .client
            .get_research_reports(code)
            .await
            .map_err(|e| te(e.to_string()))?;
        Ok(ToolResult::success(serde_json::to_string(&r).unwrap_or_default()))
    }
}

// ── 18. StockConceptBlocksTool ──
pub struct StockConceptBlocksTool {
    pub client: Arc<AStockClient>,
}
impl StockConceptBlocksTool {
    pub fn new(c: Arc<AStockClient>) -> Self {
        Self { client: c }
    }
}
#[async_trait]
impl Tool for StockConceptBlocksTool {
    fn name(&self) -> &str {
        "get_concept_blocks"
    }
    fn description(&self) -> &str {
        "获取概念板块三维归属（行业/概念/地域）"
    }
    fn input_schema(&self) -> Value {
        json!({"type":"object","properties":{"stock_code":{"type":"string","description":"6位股票代码"}},"required":["stock_code"]})
    }
    fn category(&self) -> ToolCategory {
        ToolCategory::Finance
    }
    async fn call(&self, input: Value, _ctx: &ToolContext) -> Result<ToolResult, ToolError> {
        let code = input["stock_code"]
            .as_str()
            .filter(|s| !s.is_empty())
            .ok_or_else(|| te("stock_code不能为空".into()))?;
        let r = self
            .client
            .get_concept_blocks(code)
            .await
            .map_err(|e| te(e.to_string()))?;
        Ok(ToolResult::success(serde_json::to_string(&r).unwrap_or_default()))
    }
}

// ── 19. StockOptionPCRTool ──
pub struct StockOptionPCRTool {
    pub client: Arc<AStockClient>,
}
impl StockOptionPCRTool {
    pub fn new(c: Arc<AStockClient>) -> Self {
        Self { client: c }
    }
}
#[async_trait]
impl Tool for StockOptionPCRTool {
    fn name(&self) -> &str {
        "get_stock_option_pcr"
    }
    fn description(&self) -> &str {
        "获取期权PCR（看跌/看涨成交量和持仓量比率，市场情绪前瞻指标）"
    }
    fn input_schema(&self) -> Value {
        json!({"type":"object","properties":{"stock_code":{"type":"string","description":"6位股票代码"}},"required":["stock_code"]})
    }
    fn category(&self) -> ToolCategory {
        ToolCategory::Finance
    }
    async fn call(&self, input: Value, _ctx: &ToolContext) -> Result<ToolResult, ToolError> {
        let code = input["stock_code"]
            .as_str()
            .filter(|s| !s.is_empty())
            .ok_or_else(|| te("stock_code不能为空".into()))?;
        let r = self
            .client
            .get_option_pcr(code)
            .await
            .map_err(|e| te(e.to_string()))?;
        Ok(ToolResult::success(serde_json::to_string(&r).unwrap_or_default()))
    }
}

// ── 20. StockCLSFlashTool ──
pub struct StockCLSFlashTool {
    pub client: Arc<AStockClient>,
}
impl StockCLSFlashTool {
    pub fn new(c: Arc<AStockClient>) -> Self {
        Self { client: c }
    }
}
#[async_trait]
impl Tool for StockCLSFlashTool {
    fn name(&self) -> &str {
        "get_cls_flash"
    }
    fn description(&self) -> &str {
        "获取财联社实时快讯（分钟级电报）"
    }
    fn input_schema(&self) -> Value {
        json!({"type":"object","properties":{}})
    }
    fn category(&self) -> ToolCategory {
        ToolCategory::Finance
    }
    async fn call(&self, _input: Value, _ctx: &ToolContext) -> Result<ToolResult, ToolError> {
        let r = self
            .client
            .get_cls_flash()
            .await
            .map_err(|e| te(e.to_string()))?;
        Ok(ToolResult::success(serde_json::to_string(&r).unwrap_or_default()))
    }
}

// ── 21. StockNorthBoundFlowTool ──
pub struct StockNorthBoundFlowTool {
    pub client: Arc<AStockClient>,
}
impl StockNorthBoundFlowTool {
    pub fn new(c: Arc<AStockClient>) -> Self {
        Self { client: c }
    }
}
#[async_trait]
impl Tool for StockNorthBoundFlowTool {
    fn name(&self) -> &str {
        "get_north_bound_flow"
    }
    fn description(&self) -> &str {
        "获取北向资金分钟级流向（沪深股通）"
    }
    fn input_schema(&self) -> Value {
        json!({"type":"object","properties":{}})
    }
    fn category(&self) -> ToolCategory {
        ToolCategory::Finance
    }
    async fn call(&self, _input: Value, _ctx: &ToolContext) -> Result<ToolResult, ToolError> {
        let r = self
            .client
            .get_north_bound_flow()
            .await
            .map_err(|e| te(e.to_string()))?;
        Ok(ToolResult::success(serde_json::to_string(&r).unwrap_or_default()))
    }
}

// ── 22. StockMarketDragonTigerTool ──
pub struct StockMarketDragonTigerTool {
    pub client: Arc<AStockClient>,
}
impl StockMarketDragonTigerTool {
    pub fn new(c: Arc<AStockClient>) -> Self {
        Self { client: c }
    }
}
#[async_trait]
impl Tool for StockMarketDragonTigerTool {
    fn name(&self) -> &str {
        "get_market_dragon_tiger"
    }
    fn description(&self) -> &str {
        "获取全市场龙虎榜（每日上榜股票+净买额排名）"
    }
    fn input_schema(&self) -> Value {
        json!({"type":"object","properties":{}})
    }
    fn category(&self) -> ToolCategory {
        ToolCategory::Finance
    }
    async fn call(&self, _input: Value, _ctx: &ToolContext) -> Result<ToolResult, ToolError> {
        let r = self
            .client
            .get_market_dragon_tiger()
            .await
            .map_err(|e| te(e.to_string()))?;
        Ok(ToolResult::success(serde_json::to_string(&r).unwrap_or_default()))
    }
}

// ── 23. StockIndexQuotesTool ──
pub struct StockIndexQuotesTool {
    pub client: Arc<AStockClient>,
}
impl StockIndexQuotesTool {
    pub fn new(c: Arc<AStockClient>) -> Self {
        Self { client: c }
    }
}
#[async_trait]
impl Tool for StockIndexQuotesTool {
    fn name(&self) -> &str {
        "get_index_quotes"
    }
    fn description(&self) -> &str {
        "获取大盘指数行情（上证指数、深证成指、创业板指）"
    }
    fn input_schema(&self) -> Value {
        json!({"type":"object","properties":{}})
    }
    fn category(&self) -> ToolCategory {
        ToolCategory::Finance
    }
    async fn call(&self, _input: Value, _ctx: &ToolContext) -> Result<ToolResult, ToolError> {
        let r = self
            .client
            .get_index_quotes()
            .await
            .map_err(|e| te(e.to_string()))?;
        Ok(ToolResult::success(serde_json::to_string(&r).unwrap_or_default()))
    }
}

// ── 24. StockLockupTool ──
pub struct StockLockupTool {
    pub client: Arc<AStockClient>,
}
impl StockLockupTool {
    pub fn new(c: Arc<AStockClient>) -> Self {
        Self { client: c }
    }
}
#[async_trait]
impl Tool for StockLockupTool {
    fn name(&self) -> &str {
        "get_stock_lockup"
    }
    fn description(&self) -> &str {
        "获取限售解禁日程（解禁日期、股数、比例、股东名称）"
    }
    fn input_schema(&self) -> Value {
        json!({"type":"object","properties":{"stock_code":{"type":"string","description":"6位股票代码"}},"required":["stock_code"]})
    }
    fn category(&self) -> ToolCategory {
        ToolCategory::Finance
    }
    async fn call(&self, input: Value, _ctx: &ToolContext) -> Result<ToolResult, ToolError> {
        let code = input["stock_code"]
            .as_str()
            .filter(|s| !s.is_empty())
            .ok_or_else(|| te("stock_code不能为空".into()))?;
        let r = self
            .client
            .get_lockup_schedule(code)
            .await
            .map_err(|e| te(e.to_string()))?;
        Ok(ToolResult::success(serde_json::to_string(&r).unwrap_or_default()))
    }
}

// ── 25. StockShareholderTradesTool ──
pub struct StockShareholderTradesTool {
    pub client: Arc<AStockClient>,
}
impl StockShareholderTradesTool {
    pub fn new(c: Arc<AStockClient>) -> Self {
        Self { client: c }
    }
}
#[async_trait]
impl Tool for StockShareholderTradesTool {
    fn name(&self) -> &str {
        "get_stock_shareholder_trades"
    }
    fn description(&self) -> &str {
        "获取大股东增减持记录（变动类型、数量、均价、原因）"
    }
    fn input_schema(&self) -> Value {
        json!({"type":"object","properties":{"stock_code":{"type":"string","description":"6位股票代码"}},"required":["stock_code"]})
    }
    fn category(&self) -> ToolCategory {
        ToolCategory::Finance
    }
    async fn call(&self, input: Value, _ctx: &ToolContext) -> Result<ToolResult, ToolError> {
        let code = input["stock_code"]
            .as_str()
            .filter(|s| !s.is_empty())
            .ok_or_else(|| te("stock_code不能为空".into()))?;
        let r = self
            .client
            .get_shareholder_trades(code)
            .await
            .map_err(|e| te(e.to_string()))?;
        Ok(ToolResult::success(serde_json::to_string(&r).unwrap_or_default()))
    }
}

// ── 26. StockDividendRecordsTool ──
pub struct StockDividendRecordsTool {
    pub client: Arc<AStockClient>,
}
impl StockDividendRecordsTool {
    pub fn new(c: Arc<AStockClient>) -> Self {
        Self { client: c }
    }
}
#[async_trait]
impl Tool for StockDividendRecordsTool {
    fn name(&self) -> &str {
        "get_stock_dividend_records"
    }
    fn description(&self) -> &str {
        "获取除权除息/分红送配记录"
    }
    fn input_schema(&self) -> Value {
        json!({"type":"object","properties":{"stock_code":{"type":"string","description":"6位股票代码"}},"required":["stock_code"]})
    }
    fn category(&self) -> ToolCategory {
        ToolCategory::Finance
    }
    async fn call(&self, input: Value, _ctx: &ToolContext) -> Result<ToolResult, ToolError> {
        let code = input["stock_code"]
            .as_str()
            .filter(|s| !s.is_empty())
            .ok_or_else(|| te("stock_code不能为空".into()))?;
        let r = self
            .client
            .get_dividend_records(code)
            .await
            .map_err(|e| te(e.to_string()))?;
        Ok(ToolResult::success(serde_json::to_string(&r).unwrap_or_default()))
    }
}

// ── 27. StockNorthBoundHoldingTool ──
pub struct StockNorthBoundHoldingTool {
    pub client: Arc<AStockClient>,
}
impl StockNorthBoundHoldingTool {
    pub fn new(c: Arc<AStockClient>) -> Self {
        Self { client: c }
    }
}
#[async_trait]
impl Tool for StockNorthBoundHoldingTool {
    fn name(&self) -> &str {
        "get_stock_north_bound"
    }
    fn description(&self) -> &str {
        "获取北向资金个股持仓（持股数量、占比）"
    }
    fn input_schema(&self) -> Value {
        json!({"type":"object","properties":{"stock_code":{"type":"string","description":"6位股票代码"}},"required":["stock_code"]})
    }
    fn category(&self) -> ToolCategory {
        ToolCategory::Finance
    }
    async fn call(&self, input: Value, _ctx: &ToolContext) -> Result<ToolResult, ToolError> {
        let code = input["stock_code"]
            .as_str()
            .filter(|s| !s.is_empty())
            .ok_or_else(|| te("stock_code不能为空".into()))?;
        let r = self
            .client
            .get_north_bound_holding(code)
            .await
            .map_err(|e| te(e.to_string()))?;
        Ok(ToolResult::success(serde_json::to_string(&r).unwrap_or_default()))
    }
}

// ── 28. StockDragonTigerTool ──
pub struct StockDragonTigerTool {
    pub client: Arc<AStockClient>,
}
impl StockDragonTigerTool {
    pub fn new(c: Arc<AStockClient>) -> Self {
        Self { client: c }
    }
}
#[async_trait]
impl Tool for StockDragonTigerTool {
    fn name(&self) -> &str {
        "get_stock_dragon_tiger"
    }
    fn description(&self) -> &str {
        "获取个股龙虎榜数据（营业部买卖、上榜原因）"
    }
    fn input_schema(&self) -> Value {
        json!({"type":"object","properties":{"stock_code":{"type":"string","description":"6位股票代码"}},"required":["stock_code"]})
    }
    fn category(&self) -> ToolCategory {
        ToolCategory::Finance
    }
    async fn call(&self, input: Value, _ctx: &ToolContext) -> Result<ToolResult, ToolError> {
        let code = input["stock_code"]
            .as_str()
            .filter(|s| !s.is_empty())
            .ok_or_else(|| te("stock_code不能为空".into()))?;
        let r = self
            .client
            .get_dragon_tiger(code)
            .await
            .map_err(|e| te(e.to_string()))?;
        Ok(ToolResult::success(serde_json::to_string(&r).unwrap_or_default()))
    }
}

// ── 29. StockMarginDataTool ──
pub struct StockMarginDataTool {
    pub client: Arc<AStockClient>,
}
impl StockMarginDataTool {
    pub fn new(c: Arc<AStockClient>) -> Self {
        Self { client: c }
    }
}
#[async_trait]
impl Tool for StockMarginDataTool {
    fn name(&self) -> &str {
        "get_stock_margin_data"
    }
    fn description(&self) -> &str {
        "获取融资融券数据（融资买入额、余额、融券卖出量、余量）"
    }
    fn input_schema(&self) -> Value {
        json!({"type":"object","properties":{"stock_code":{"type":"string","description":"6位股票代码"}},"required":["stock_code"]})
    }
    fn category(&self) -> ToolCategory {
        ToolCategory::Finance
    }
    async fn call(&self, input: Value, _ctx: &ToolContext) -> Result<ToolResult, ToolError> {
        let code = input["stock_code"]
            .as_str()
            .filter(|s| !s.is_empty())
            .ok_or_else(|| te("stock_code不能为空".into()))?;
        let r = self
            .client
            .get_margin_data(code)
            .await
            .map_err(|e| te(e.to_string()))?;
        Ok(ToolResult::success(serde_json::to_string(&r).unwrap_or_default()))
    }
}

// ── 30. StockSectorInfoTool ──
pub struct StockSectorInfoTool {
    pub client: Arc<AStockClient>,
}
impl StockSectorInfoTool {
    pub fn new(c: Arc<AStockClient>) -> Self {
        Self { client: c }
    }
}
#[async_trait]
impl Tool for StockSectorInfoTool {
    fn name(&self) -> &str {
        "get_stock_sector_info"
    }
    fn description(&self) -> &str {
        "获取行业分类（申万一级/二级、概念板块标签）"
    }
    fn input_schema(&self) -> Value {
        json!({"type":"object","properties":{"stock_code":{"type":"string","description":"6位股票代码"}},"required":["stock_code"]})
    }
    fn category(&self) -> ToolCategory {
        ToolCategory::Finance
    }
    async fn call(&self, input: Value, _ctx: &ToolContext) -> Result<ToolResult, ToolError> {
        let code = input["stock_code"]
            .as_str()
            .filter(|s| !s.is_empty())
            .ok_or_else(|| te("stock_code不能为空".into()))?;
        let r = self
            .client
            .get_sector_info(code)
            .await
            .map_err(|e| te(e.to_string()))?;
        Ok(ToolResult::success(serde_json::to_string(&r).unwrap_or_default()))
    }
}

// ── Registration ──
pub fn register_stock_tools(
    registry: &mut crate::registry::ToolRegistry,
    client: Arc<AStockClient>,
) {
    registry.register_all(vec![
        Arc::new(StockQuoteTool::new(client.clone())),
        Arc::new(StockKlineTool::new(client.clone())),
        Arc::new(StockFinancialsTool::new(client.clone())),
        Arc::new(StockNewsTool::new(client.clone())),
        Arc::new(StockMoneyFlowTool::new(client.clone())),
        Arc::new(StockHotStocksTool::new(client.clone())),
        Arc::new(StockIndustryRankTool::new(client.clone())),
        Arc::new(StockAnnouncementsTool::new(client.clone())),
        Arc::new(StockConsensusEPSTool::new(client.clone())),
        Arc::new(SearchStockTool::new(client.clone())),
        Arc::new(ComputeScoringTool::new(client.clone())),
        Arc::new(ComputeValuationTool::new(client.clone())),
        Arc::new(ComputeRiskTool::new(client.clone())),
        Arc::new(StockBlockTradesTool::new(client.clone())),
        Arc::new(StockInstitutionalVisitsTool::new(client.clone())),
        Arc::new(StockPeersTool::new(client.clone())),
        Arc::new(StockResearchReportsTool::new(client.clone())),
        Arc::new(StockConceptBlocksTool::new(client.clone())),
        Arc::new(StockOptionPCRTool::new(client.clone())),
        Arc::new(StockCLSFlashTool::new(client.clone())),
        Arc::new(StockNorthBoundFlowTool::new(client.clone())),
        Arc::new(StockMarketDragonTigerTool::new(client.clone())),
        Arc::new(StockIndexQuotesTool::new(client.clone())),
        Arc::new(StockLockupTool::new(client.clone())),
        Arc::new(StockShareholderTradesTool::new(client.clone())),
        Arc::new(StockDividendRecordsTool::new(client.clone())),
        Arc::new(StockNorthBoundHoldingTool::new(client.clone())),
        Arc::new(StockDragonTigerTool::new(client.clone())),
        Arc::new(StockMarginDataTool::new(client.clone())),
        Arc::new(StockSectorInfoTool::new(client)),
    ]);
}
