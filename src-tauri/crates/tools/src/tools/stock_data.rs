use crate::{Tool, ToolCategory, ToolContext, ToolError, ToolResult};
use async_trait::async_trait;
use axagent_astock_data::AStockClient;
use chrono::Local;
use serde_json::{Value, json};
use std::sync::Arc;

fn te(msg: String) -> ToolError {
    ToolError::execution_failed(msg)
}

/// 从 tool input 中提取 _template_vars 中指定 key 的值，取不到则返回默认值。
/// 模板变量在 tool_executor.rs 构建 `resolved_args` 时自动注入。
fn tv_f64(input: &Value, key: &str, default: f64) -> f64 {
    input
        .get("_template_vars")
        .and_then(|tv| tv.get(key))
        .and_then(|v| v.as_f64())
        .unwrap_or(default)
}
#[allow(dead_code)]
fn tv_i64(input: &Value, key: &str, default: i64) -> i64 {
    input
        .get("_template_vars")
        .and_then(|tv| tv.get(key))
        .and_then(|v| v.as_i64())
        .unwrap_or(default)
}
#[allow(dead_code)]
fn tv_bool(input: &Value, key: &str, default: bool) -> bool {
    input
        .get("_template_vars")
        .and_then(|tv| tv.get(key))
        .and_then(|v| v.as_bool())
        .unwrap_or(default)
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
        "获取行业板块涨跌排名，传入 stock_code 时自动筛选目标股票所属行业"
    }
    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "stock_code": {
                    "type": "string",
                    "description": "6位股票代码（可选，传入时返回目标股票的行业归属+该行业排名数据）"
                }
            }
        })
    }
    fn category(&self) -> ToolCategory {
        ToolCategory::Finance
    }
    async fn call(&self, input: Value, _ctx: &ToolContext) -> Result<ToolResult, ToolError> {
        let stock_code = input["stock_code"].as_str().filter(|s| !s.is_empty());
        let all_rankings = self
            .client
            .get_industry_ranking()
            .await
            .map_err(|e| te(e.to_string()))?;

        // 如果提供了 stock_code，查询该股票的行业归属并在返回数据中高亮
        if let Some(code) = stock_code {
            let maybe_blocks = self.client.get_concept_blocks(code).await.ok().flatten();

            let stock_industry: &str = maybe_blocks
                .as_ref()
                .map(|cb| cb.industry.as_str())
                .unwrap_or("");

            // 在返回结果中标记目标股票的行业位置
            let enriched: Vec<serde_json::Value> = all_rankings
                .iter()
                .map(|r| {
                    let is_target = !stock_industry.is_empty() && r.industry_name == stock_industry;
                    serde_json::json!({
                        "industryName": r.industry_name,
                        "changePct": r.change_pct,
                        "mainInflow": r.main_inflow,
                        "leaderCode": r.leader_code,
                        "leaderName": r.leader_name,
                        "leaderChangePct": r.leader_change_pct,
                        "isTargetStockIndustry": is_target,
                    })
                })
                .collect();

            let has_match = !stock_industry.is_empty()
                && all_rankings
                    .iter()
                    .any(|r| r.industry_name == stock_industry);

            let mut result = serde_json::json!({
                "rankings": enriched,
                "targetStockIndustry": stock_industry,
                "hasMatchInRankings": has_match,
            });

            // 附加概念/地域标签（用 as_ref 避免 move 冲突）
            if let Some(ref cb) = maybe_blocks {
                result["conceptTags"] =
                    serde_json::to_value(cb.concepts.iter().map(|c| &c.name).collect::<Vec<_>>())
                        .unwrap_or_default();
                result["regionTags"] =
                    serde_json::to_value(cb.regions.iter().map(|r| &r.name).collect::<Vec<_>>())
                        .unwrap_or_default();
            }

            Ok(ToolResult::success(serde_json::to_string_pretty(&result).unwrap_or_default()))
        } else {
            // 无 stock_code 时返回原始排名（向后兼容）
            Ok(ToolResult::success(serde_json::to_string(&all_rankings).unwrap_or_default()))
        }
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
        "get_stock_announcements"
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

// ── 10b. SearchNewsTool ──
pub struct SearchNewsTool {
    pub client: Arc<AStockClient>,
}
impl SearchNewsTool {
    pub fn new(c: Arc<AStockClient>) -> Self {
        Self { client: c }
    }
}
#[async_trait]
impl Tool for SearchNewsTool {
    fn name(&self) -> &str {
        "search_news"
    }
    fn description(&self) -> &str {
        "按关键词搜索财经新闻，用于验证催化剂/CapEx/行业趋势"
    }
    fn input_schema(&self) -> Value {
        json!({"type":"object","properties":{"keyword":{"type":"string","description":"搜索关键词"},"limit":{"type":"integer","description":"返回条数"}},"required":["keyword"]})
    }
    fn category(&self) -> ToolCategory {
        ToolCategory::Finance
    }
    async fn call(&self, input: Value, _ctx: &ToolContext) -> Result<ToolResult, ToolError> {
        let kw = input["keyword"]
            .as_str()
            .ok_or_else(|| te("keyword不能为空".into()))?;
        let limit = input["limit"].as_u64().unwrap_or(10) as u32;
        let r = self
            .client
            .search_news(kw, limit)
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

        // ── 从 _template_vars 读取评分权重（用户在设置面板配置） ──
        let w_trend = tv_f64(&input, "scoring_trend", 30.0);
        let w_deviation = tv_f64(&input, "scoring_deviation", 20.0);
        let w_macd = tv_f64(&input, "scoring_macd", 15.0);
        let w_volume = tv_f64(&input, "scoring_volume", 15.0);
        let w_rsi = tv_f64(&input, "scoring_rsi", 10.0);
        let w_support = tv_f64(&input, "scoring_support", 10.0);

        // ── 从 _template_vars 读取催化剂维度（由 a-catalyst 节点注入） ──
        // 修复：compute_scoring 之前只算纯技术分，a-catalyst 报告再强也加不进 base
        // 默认 50 表示中性；a-catalyst 输出 bull_score (0-100) 直接作为分值
        let catalyst_score = tv_f64(&input, "catalyst_analyst_score", 50.0);
        let catalyst_level = input
            .get("_template_vars")
            .and_then(|tv| tv.get("catalyst_level"))
            .and_then(|v| v.as_str())
            .unwrap_or("无")
            .to_string();
        let institutional_trace = input
            .get("_template_vars")
            .and_then(|tv| tv.get("institutional_trace"))
            .and_then(|v| v.as_str())
            .unwrap_or("无")
            .to_string();

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
            "放量突破" => 95.0, // 新增：突破型最高分（与 scoring.rs 保持一致）
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

        // 基本面修正（从 _template_vars 读取估值阈值）
        let pe_low = tv_f64(&input, "val_pe_low", 15.0);
        let pe_high = tv_f64(&input, "val_pe_high", 50.0);
        let pb_low = tv_f64(&input, "val_pb_low", 1.0);
        let pb_high = tv_f64(&input, "val_pb_high", 6.0);
        let mut value_adj = 0.0;
        if let Some(pe) = quote.pe {
            if pe > 0.0 && pe < pe_low {
                value_adj = 10.0;
            } else if (pe_low..pe_high).contains(&pe) {
                value_adj = 5.0;
            } else if pe >= pe_high {
                value_adj = -10.0;
            }
        }
        if let Some(pb) = quote.pb {
            if pb > 0.0 && pb < pb_low {
                value_adj += 10.0;
            } else if (pb_low..pb_high).contains(&pb) {
                value_adj += 5.0;
            } else if pb >= pb_high {
                value_adj -= 5.0;
            }
        }

        // ── 使用用户配置的权重计算最终评分 ──
        let weights = [
            w_trend / 100.0,
            w_deviation / 100.0,
            w_macd / 100.0,
            w_volume / 100.0,
            w_rsi / 100.0,
            w_support / 100.0,
        ];
        let scores = [
            trend_score,
            deviation_score,
            macd_score,
            volume_score,
            rsi_score,
            support_score,
        ];
        let weighted: f64 = scores.iter().zip(weights.iter()).map(|(s, w)| s * w).sum();

        // 催化剂加成：技术 6 维归 0.85，催化剂 1 维占 0.15
        // L3/L2 + 机构建仓时额外 +3~5 分（突破保守派的"安全边际"卡口）
        let catalyst_weight = 0.15;
        let technical_weight = 1.0 - catalyst_weight;
        let mut catalyst_bonus = 0.0;
        if catalyst_level == "L3估值体系级" {
            catalyst_bonus += 3.0;
        } else if catalyst_level == "L2业绩拐点级" {
            catalyst_bonus += 2.0;
        }
        if institutional_trace == "有建仓痕迹" || institutional_trace == "疑似建仓" {
            catalyst_bonus += 2.0;
        }
        let total_score = ((weighted * technical_weight)
            + (catalyst_score * catalyst_weight * 100.0)
            + value_adj
            + catalyst_bonus)
            .clamp(0.0, 100.0);

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
            "catalystScore": (catalyst_score * 10.0).round() / 10.0,
            "catalystLevel": catalyst_level,
            "institutionalTrace": institutional_trace,
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

        // ── 从 _template_vars 读取估值配置参数 ──
        let discount_rate = tv_f64(&input, "value_dcf_discount_rate", 10.0) / 100.0;
        let growth_stable = tv_f64(&input, "value_dcf_perpetual_rate", 3.0) / 100.0;
        let dcf_growth_rate = tv_f64(&input, "value_dcf_growth_rate", 8.0) / 100.0;
        let high_years = 5.0;
        let terminal_multiple = 15.0;

        let mut dcf_value: f64 = 0.0;
        if eps > 0.0 {
            let mut pv: f64 = 0.0;
            for year in 1..=(high_years as i32) {
                let projected_eps = eps * (1.0_f64 + dcf_growth_rate).powi(year);
                pv += projected_eps / (1.0_f64 + discount_rate).powi(year);
            }
            let terminal_eps = eps
                * (1.0_f64 + dcf_growth_rate).powi(high_years as i32)
                * (1.0_f64 + growth_stable);
            let terminal_value = terminal_eps * terminal_multiple
                / (1.0_f64 + discount_rate).powi(high_years as i32);
            dcf_value = pv + terminal_value;
        }

        let graham_value = if eps > 0.0 && bps > 0.0 {
            (22.5 * eps * bps).sqrt()
        } else {
            0.0
        };

        // F-Score: 从 _template_vars 读取阈值
        let fscore_roe_min = tv_f64(&input, "fscore_roe_min", 0.10);
        let fscore_gross_margin_min = tv_f64(&input, "fscore_gross_margin_min", 0.30);
        let fscore_net_margin_min = tv_f64(&input, "fscore_net_margin_min", 0.10);
        let fscore_debt_max = tv_f64(&input, "fscore_debt_max", 0.60);
        let fscore_pe_max = tv_f64(&input, "fscore_pe_max", 20.0);
        let mut f_score = 0i32;
        if profit_yoy > 0.0 {
            f_score += 1;
        }
        if revenue_yoy > 0.0 {
            f_score += 1;
        }
        if roe > fscore_roe_min {
            f_score += 1;
        }
        if gross_margin > fscore_gross_margin_min {
            f_score += 1;
        }
        if net_margin > fscore_net_margin_min {
            f_score += 1;
        }
        if debt_ratio < fscore_debt_max {
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
            && pe_val < fscore_pe_max
        {
            f_score += 1;
        }

        // 护城河量化：从 _template_vars 读取阈值
        let moat_gross_margin_high = tv_f64(&input, "moat_gross_margin_high", 0.40);
        let moat_roe_high = tv_f64(&input, "moat_roe_high", 0.15);
        let moat_roe_med = tv_f64(&input, "moat_roe_med", 0.10);
        let moat_debt_low = tv_f64(&input, "moat_debt_low", 0.40);
        let moat_debt_med = tv_f64(&input, "moat_debt_med", 0.60);
        let mut moat_score = 0i32;
        if gross_margin > moat_gross_margin_high {
            moat_score += 3;
        } else if gross_margin > 0.25 {
            moat_score += 2;
        } else if gross_margin > 0.15 {
            moat_score += 1;
        }
        if roe > moat_roe_high {
            moat_score += 3;
        } else if roe > moat_roe_med {
            moat_score += 2;
        } else if roe > 0.05 {
            moat_score += 1;
        }
        if debt_ratio < moat_debt_low {
            moat_score += 2;
        } else if debt_ratio < moat_debt_med {
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
        let v_freshness = if let Some(lf) = latest_fin {
            if lf
                .report_date
                .contains(&Local::now().format("%Y-%m").to_string())
            {
                "current_quarter"
            } else if let Ok(now) = chrono::NaiveDate::parse_from_str(
                &Local::now().format("%Y-%m-%d").to_string(),
                "%Y-%m-%d",
            ) {
                if let Some(rd) = lf.report_date.strip_suffix("00:00:00") {
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
                    "highGrowthRate": dcf_growth_rate,
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

        // 组合风控阈值：从 _template_vars 读取
        let hhi_concentrated = tv_f64(&input, "risk_hhi_concentrated", 0.25);
        let hhi_medium = tv_f64(&input, "risk_hhi_medium", 0.15);
        let divers_high = tv_f64(&input, "risk_divers_high", 8.0);
        let divers_medium = tv_f64(&input, "risk_divers_medium", 4.0);
        let concentration_label = if hhi > hhi_concentrated {
            "高度集中"
        } else if hhi > hhi_medium {
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

        let diversification_label = if effective_n >= divers_high {
            "充分分散"
        } else if effective_n >= divers_medium {
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
    // [BUGFIX] 工作流 t-research-data 节点 (stock_analysis_setup.rs:1341) 引用
    // 工具名 "get_stock_research_reports",而本工具主名是 "get_research_reports" ——
    // 不加 alias 会导致 tool_registry.find() 报 "工具未找到"。加 alias 后两个名字
    // 都能找到工具,MCP 层 (astock-data/src/mcp_tools.rs) 也用 "get_stock_research_reports"
    // 保持一致。
    fn aliases(&self) -> &[&str] {
        &["get_stock_research_reports"]
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
        "get_stock_concept_blocks"
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

// ── 30. ComputeAttentionScoreTool ──
// 方案B: 综合多信号计算个股关注度评分（0=冷门 100=过热）
// 权重可通过 input.weights 覆盖，用于回测优化
pub struct ComputeAttentionScoreTool {
    pub client: Arc<AStockClient>,
}
impl ComputeAttentionScoreTool {
    pub fn new(c: Arc<AStockClient>) -> Self {
        Self { client: c }
    }
}
#[async_trait]
impl Tool for ComputeAttentionScoreTool {
    fn name(&self) -> &str {
        "compute_attention_score"
    }
    fn description(&self) -> &str {
        "计算个股关注度评分 0-100（基于新闻量/研报/机构调研/换手率），越低越冷门"
    }
    fn input_schema(&self) -> Value {
        json!({"type":"object","properties":{
        "stock_code":{"type":"string","description":"6位股票代码"},
        "weights":{"type":"object","description":"可选权重覆盖，默认0.25/0.20/0.25/0.15/0.15","properties":{
            "turnover":{"type":"number","description":"换手率权重"},
            "news":{"type":"number","description":"新闻量权重"},
            "report":{"type":"number","description":"研报量权重"},
            "visit":{"type":"number","description":"机构调研权重"},
            "mcap":{"type":"number","description":"市值权重"}
        }}
    },"required":["stock_code"]})
    }
    fn category(&self) -> ToolCategory {
        ToolCategory::Finance
    }
    async fn call(&self, input: Value, _ctx: &ToolContext) -> Result<ToolResult, ToolError> {
        let code = input["stock_code"]
            .as_str()
            .filter(|s| !s.is_empty())
            .ok_or_else(|| te("stock_code不能为空".into()))?;

        // 读取权重（支持覆盖，默认 = 回测初值）
        let w = |key: &str, def: f64| -> f64 {
            input["weights"][key]
                .as_f64()
                .unwrap_or(def)
                .clamp(0.0, 1.0)
        };
        let w_turnover = w("turnover", 0.25);
        let w_news = w("news", 0.20);
        let w_report = w("report", 0.25);
        let w_visit = w("visit", 0.15);
        let w_mcap = w("mcap", 0.15);
        // 归一化到总和=1.0
        let total = w_turnover + w_news + w_report + w_visit + w_mcap;
        let (w_turnover, w_news, w_report, w_visit, w_mcap) = if total > 0.0 {
            (
                w_turnover / total,
                w_news / total,
                w_report / total,
                w_visit / total,
                w_mcap / total,
            )
        } else {
            (0.25, 0.20, 0.25, 0.15, 0.15)
        };

        // 1. 行情数据（换手率、市值）
        let quote = self
            .client
            .get_quote(code)
            .await
            .map_err(|e| te(e.to_string()))?;
        let turnover_rate = quote.turnover_rate;
        let market_cap = quote.total_mv.unwrap_or(0.0);

        // 2. 新闻量（近30条）
        let news_count = self
            .client
            .get_news(code, 10)
            .await
            .map(|v| v.len() as f64)
            .unwrap_or(0.0);

        // 3. 研报量
        let report_count = self
            .client
            .get_research_reports(code)
            .await
            .map(|v| v.len() as f64)
            .unwrap_or(0.0);

        // 4. 机构调研
        let visit_count = self
            .client
            .get_institutional_visits(code)
            .await
            .map(|v| v.len() as f64)
            .unwrap_or(0.0);

        // 5. 计算单项分（0-100）
        let score_turnover = (turnover_rate / 10.0 * 100.0).min(100.0);
        let score_news = (news_count / 5.0 * 100.0).min(100.0);
        let score_report = (report_count / 5.0 * 100.0).min(100.0);
        let score_visit = (visit_count / 3.0 * 100.0).min(100.0);
        let score_mcap = if market_cap > 0.0 {
            ((market_cap / 500.0) * 100.0).min(100.0)
        } else {
            50.0
        };

        let attention_score = (score_turnover * w_turnover
            + score_news * w_news
            + score_report * w_report
            + score_visit * w_visit
            + score_mcap * w_mcap) as u32;

        let result = serde_json::json!({
            "stock_code": code,
            "attention_score": attention_score.min(100),
            "detail": {
                "turnover_rate_pct": turnover_rate,
                "news_count_30d": news_count,
                "research_report_count": report_count,
                "institutional_visit_count": visit_count,
                "market_cap_billion": market_cap,
                "heat_label": if attention_score <= 30 { "冷门" } else if attention_score <= 60 { "正常" } else { "热门" }
            },
            "weights_used": {
                "turnover": w_turnover,
                "news": w_news,
                "report": w_report,
                "visit": w_visit,
                "mcap": w_mcap,
                "note": "可通过 input.weights 覆盖。回测时调用方可自动调优"
            }
        });
        Ok(ToolResult::success(result.to_string()))
    }
}

// ── 31. ComputeIndustryPositionTool ──
// 方案C: 行业竞争地位分析（同行对比 + 产能指标 + 排名）
pub struct ComputeIndustryPositionTool {
    pub client: Arc<AStockClient>,
}
impl ComputeIndustryPositionTool {
    pub fn new(c: Arc<AStockClient>) -> Self {
        Self { client: c }
    }
}
#[async_trait]
impl Tool for ComputeIndustryPositionTool {
    fn name(&self) -> &str {
        "compute_industry_position"
    }
    fn description(&self) -> &str {
        "行业竞争地位分析：同行对比毛利率/ROE/负债率，计算产能指标（固定资产周转率/资本开支比）和排名"
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

        // 1. 获取行业信息
        let sector_info = self.client.get_sector_info(code).await.ok().flatten();

        // 2. 获取同行对比
        let peers = self.client.get_peers(code).await.unwrap_or_default();

        // 3. 获取财务数据
        let financials = self.client.get_financials(code).await.ok();
        let latest = financials.as_ref().and_then(|f| f.first());

        // 4. 计算产能/竞争指标
        let gm = latest.and_then(|f| f.gross_margin).unwrap_or(0.0);
        let roe = latest.and_then(|f| f.roe).unwrap_or(0.0);
        let debt_ratio = latest.and_then(|f| f.debt_ratio).unwrap_or(0.0);
        let rnd_ratio = latest.and_then(|f| f.net_margin).unwrap_or(0.0); // net_margin 作为研发密度近似
        let capex = latest.and_then(|f| f.capital_expenditure).unwrap_or(0.0);
        let ocf = latest.and_then(|f| f.operating_cash_flow).unwrap_or(0.0);
        let revenue = latest.and_then(|f| f.revenue).unwrap_or(0.0);

        // 资本开支/折旧：用 operating_cash_flow 近似折旧
        let capex_dep_ratio = if ocf > 0.0 {
            (capex / ocf).round()
        } else {
            0.0
        };

        // 同行对比排名
        let mut peer_gms: Vec<f64> = peers.iter().filter_map(|p| p.roe).collect();
        peer_gms.sort_by(|a, b| b.partial_cmp(a).unwrap_or(std::cmp::Ordering::Equal));
        let gm_rank = peer_gms
            .iter()
            .position(|&v| v <= roe)
            .map(|i| i + 1)
            .unwrap_or(0);

        let result = serde_json::json!({
            "stock_code": code,
            "sector": sector_info.as_ref().map(|s| s.sector_name.clone()).unwrap_or_default(),
            "sub_sector": sector_info.as_ref().map(|s| s.sub_sector.clone()).unwrap_or_default(),
            "competitive_position": {
                "gross_margin_pct": gm,
                "roe_pct": roe,
                "debt_ratio_pct": debt_ratio,
                "rnd_intensity": rnd_ratio,
                "gm_rank_in_peers": gm_rank,
                "total_peer_count": peers.len(),
            },
            "capacity_indicators": {
                "capex_depreciation_ratio": capex_dep_ratio,
                "revenue": revenue,
                "capex": capex,
                "signal": if capex_dep_ratio >= 3.0 {
                    "积极扩产（资本开支/折旧 > 3）"
                } else if capex_dep_ratio >= 1.5 {
                    "温和扩产"
                } else {
                    "维持性投入"
                }
            },
            "peer_summary": peers.iter().take(5).map(|p| {
                serde_json::json!({
                    "stock_code": p.stock_code,
                    "stock_name": p.stock_name,
                    "roe": p.roe,
                    "market_cap": p.market_cap,
                })
            }).collect::<Vec<_>>(),
        });
        Ok(ToolResult::success(result.to_string()))
    }
}

// ── 32. CheckExitSignalsTool ──
// Phase 3: 退出信号持续监控 — 检查个股是否触发退出条件
pub struct CheckExitSignalsTool {
    pub client: Arc<AStockClient>,
}
impl CheckExitSignalsTool {
    pub fn new(c: Arc<AStockClient>) -> Self {
        Self { client: c }
    }
}
#[async_trait]
impl Tool for CheckExitSignalsTool {
    fn name(&self) -> &str {
        "check_exit_signals"
    }
    fn description(&self) -> &str {
        "检查个股的退出信号：价格止损、技术替代新闻、产能过剩信号。返回 exit_urgency"
    }
    fn input_schema(&self) -> Value {
        json!({"type":"object","properties":{
        "stock_code":{"type":"string","description":"6位股票代码"},
        "entry_price":{"type":"number","description":"买入价（用于计算止损触发）"},
        "stop_loss_price":{"type":"number","description":"止损价"}
    },"required":["stock_code"]})
    }
    fn category(&self) -> ToolCategory {
        ToolCategory::Finance
    }
    async fn call(&self, input: Value, _ctx: &ToolContext) -> Result<ToolResult, ToolError> {
        let code = input["stock_code"]
            .as_str()
            .filter(|s| !s.is_empty())
            .ok_or_else(|| te("stock_code不能为空".into()))?;
        let entry_price = input["entry_price"].as_f64();
        let stop_loss_price = input["stop_loss_price"].as_f64();

        // 1. 获取当前行情
        let quote = self
            .client
            .get_quote(code)
            .await
            .map_err(|e| te(e.to_string()))?;
        let price = quote.price;
        let change_pct_1m = quote.change_pct; // 当日涨跌幅（近似）

        // 2. 检查价格止损
        let stop_loss_hit = stop_loss_price.map(|sl| price < sl).unwrap_or(false);

        // 3. 搜索负面新闻
        let disruption_news = self
            .client
            .search_news(&format!("{} 技术替代 产能过剩 竞争", code), 5)
            .await
            .unwrap_or_default();
        let has_disruption_news = disruption_news.len() >= 2;

        // 4. 获取财务趋势（负债率和毛利率变化）
        let financials = self.client.get_financials(code).await.ok();
        let margin_declining = financials
            .as_ref()
            .and_then(|f| {
                if f.len() >= 2 {
                    let curr = f[0].gross_margin.unwrap_or(0.0);
                    let prev = f[1].gross_margin.unwrap_or(0.0);
                    Some(prev > 0.0 && curr < prev * 0.85) // 毛利率下降超过15%
                } else {
                    None
                }
            })
            .unwrap_or(false);

        // 5. 综合判断退出紧迫度
        let (urgency, reasons) = if stop_loss_hit {
            ("exit_now", vec!["止损价已触发".to_string()])
        } else if has_disruption_news && margin_declining {
            ("exit_now", vec!["技术替代/竞争加剧 + 毛利率持续下降".to_string()])
        } else if has_disruption_news {
            ("caution", vec!["检测到技术替代或产能过剩相关新闻".to_string()])
        } else if margin_declining {
            ("caution", vec!["毛利率明显下降，关注竞争格局变化".to_string()])
        } else if entry_price
            .map(|ep| (price - ep) / ep < -0.15)
            .unwrap_or(false)
        {
            (
                "watch",
                vec![format!(
                    "距入场价下跌 {:.1}%，接近止损",
                    (price / entry_price.unwrap_or(price) - 1.0) * 100.0
                )],
            )
        } else {
            ("no_urgency", vec!["退出信号未触发".to_string()])
        };

        let result = serde_json::json!({
            "stock_code": code,
            "current_price": price,
            "change_pct_today": change_pct_1m,
            "stop_loss_hit": stop_loss_hit,
            "has_disruption_news": has_disruption_news,
            "margin_declining": margin_declining,
            "exit_urgency": urgency,
            "reasons": reasons,
            "updated_at": chrono::Utc::now().to_rfc3339()
        });
        Ok(ToolResult::success(result.to_string()))
    }
}

// ── 33. ComputeSerenityPerformanceTool ──
// 回馈闭环: 跟踪 Serenity 候选的推荐后表现
pub struct ComputeSerenityPerformanceTool {
    pub client: Arc<AStockClient>,
}
impl ComputeSerenityPerformanceTool {
    pub fn new(c: Arc<AStockClient>) -> Self {
        Self { client: c }
    }
}
#[async_trait]
impl Tool for ComputeSerenityPerformanceTool {
    fn name(&self) -> &str {
        "compute_serenity_performance"
    }
    fn description(&self) -> &str {
        "计算 Serenity 候选股的推荐后表现：从 entry_date 到现在的收益率、最大回撤、波动率"
    }
    fn input_schema(&self) -> Value {
        json!({"type":"object","properties":{
        "stock_code":{"type":"string","description":"6位股票代码"},
        "recommend_date":{"type":"string","description":"推荐日期 YYYY-MM-DD"}
    },"required":["stock_code","recommend_date"]})
    }
    fn category(&self) -> ToolCategory {
        ToolCategory::Finance
    }
    async fn call(&self, input: Value, _ctx: &ToolContext) -> Result<ToolResult, ToolError> {
        let code = input["stock_code"]
            .as_str()
            .filter(|s| !s.is_empty())
            .ok_or_else(|| te("stock_code不能为空".into()))?;
        let date = input["recommend_date"]
            .as_str()
            .filter(|s| !s.is_empty())
            .ok_or_else(|| te("recommend_date不能为空".into()))?;

        // 获取推荐日 K 线（前复权）确定入场价
        let entry_kline = self
            .client
            .get_klines(code, "daily", 30)
            .await
            .map_err(|e| te(e.to_string()))?;
        let entry_price = entry_kline
            .iter()
            .find(|k| k.date.starts_with(date))
            .map(|k| k.close)
            .or_else(|| entry_kline.last().map(|k| k.close))
            .unwrap_or(0.0);

        // 获取当前行情
        let current_quote = self
            .client
            .get_quote(code)
            .await
            .map_err(|e| te(e.to_string()))?;
        let current_price = current_quote.price;

        // 计算收益率
        let return_pct = if entry_price > 0.0 {
            (current_price - entry_price) / entry_price * 100.0
        } else {
            0.0
        };

        // 从 K 线计算最大回撤和波动率
        let max_drawdown: f64 = entry_kline
            .iter()
            .filter(|k| k.date.as_str() >= date)
            .fold((0.0_f64, entry_price), |(max_dd, peak), k| {
                let new_peak = peak.max(k.close);
                let dd = (new_peak - k.close) / new_peak * 100.0;
                (max_dd.max(dd), new_peak)
            })
            .0;
        let volatility = if entry_kline.len() >= 5 {
            let returns: Vec<f64> = entry_kline
                .windows(2)
                .map(|w| (w[1].close - w[0].close) / w[0].close)
                .collect();
            let mean = returns.iter().sum::<f64>() / returns.len() as f64;
            let variance =
                returns.iter().map(|r| (r - mean).powi(2)).sum::<f64>() / returns.len() as f64;
            variance.sqrt() * 100.0
        } else {
            0.0
        };

        let result = serde_json::json!({
            "stock_code": code,
            "entry_price": entry_price,
            "current_price": current_price,
            "return_pct": (return_pct * 100.0).round() / 100.0,
            "max_drawdown_pct": (max_drawdown * 100.0_f64).round() / 100.0,
            "volatility_pct": (volatility * 100.0_f64).round() / 100.0,
            "days_held": entry_kline.iter().filter(|k| k.date.as_str() >= date).count(),
            "is_profitable": return_pct > 0.0,
        });
        Ok(ToolResult::success(result.to_string()))
    }
}

// ── 34. VerifyCatalystsTool ──
// 回馈闭环: 验证催化剂是否兑现
pub struct VerifyCatalystsTool {
    pub client: Arc<AStockClient>,
}
impl VerifyCatalystsTool {
    pub fn new(c: Arc<AStockClient>) -> Self {
        Self { client: c }
    }
}
#[async_trait]
impl Tool for VerifyCatalystsTool {
    fn name(&self) -> &str {
        "verify_catalysts"
    }
    fn description(&self) -> &str {
        "验证 Serenity 候选的催化剂是否兑现：搜索新闻确认事件是否发生"
    }
    fn input_schema(&self) -> Value {
        json!({"type":"object","properties":{
        "stock_code":{"type":"string","description":"6位股票代码"},
        "catalyst_descriptions":{"type":"array","items":{"type":"string"},"description":"催化剂描述列表"}
    },"required":["stock_code"]})
    }
    fn category(&self) -> ToolCategory {
        ToolCategory::Finance
    }
    async fn call(&self, input: Value, _ctx: &ToolContext) -> Result<ToolResult, ToolError> {
        let code = input["stock_code"]
            .as_str()
            .filter(|s| !s.is_empty())
            .ok_or_else(|| te("stock_code不能为空".into()))?;
        let descriptions: Vec<&str> = input["catalyst_descriptions"]
            .as_array()
            .map(|a| a.iter().filter_map(|v| v.as_str()).collect())
            .unwrap_or_default();

        let mut results = Vec::new();
        for desc in &descriptions {
            // 用搜索关键词找催化剂相关的新闻
            let news = self.client.search_news(desc, 5).await.unwrap_or_default();
            let found = news.iter().any(|n| {
                n.title.contains(*desc)
                    || n.summary.contains(*desc)
                    || desc
                        .chars()
                        .all(|c| n.title.contains(c) || n.summary.contains(c))
            });
            results.push(serde_json::json!({
                "description": desc,
                "verified": found,
                "evidence_count": news.len(),
                "top_match": news.first().map(|n| n.title.clone()).unwrap_or_default(),
            }));
        }

        let result = serde_json::json!({
            "stock_code": code,
            "catalysts_checked": results.len(),
            "verified_count": results.iter().filter(|r| r["verified"].as_bool().unwrap_or(false)).count(),
            "details": results,
        });
        Ok(ToolResult::success(result.to_string()))
    }
}

// ── 35. OptimizeAttentionWeightsTool ──
// 回馈闭环: 基于历史表现调优关注度评分权重
pub struct OptimizeAttentionWeightsTool {
    pub client: Arc<AStockClient>,
}
impl OptimizeAttentionWeightsTool {
    pub fn new(c: Arc<AStockClient>) -> Self {
        Self { client: c }
    }
}
#[async_trait]
impl Tool for OptimizeAttentionWeightsTool {
    fn name(&self) -> &str {
        "optimize_attention_weights"
    }
    fn description(&self) -> &str {
        "基于历史候选表现调优 compute_attention_score 的权重。输入候选表现列表，输出最优权重组合"
    }
    fn input_schema(&self) -> Value {
        json!({"type":"object","properties":{
        "samples":{"type":"array","description":"样本列表: [{attention_score, return_pct, ...}]","items":{
            "type":"object","properties":{
                "attention_score":{"type":"number"},"return_pct":{"type":"number"},
                "news_count":{"type":"number"},"report_count":{"type":"number"},
                "visit_count":{"type":"number"},"turnover_rate":{"type":"number"},"market_cap":{"type":"number"}
            }
        }}
    },"required":["samples"]})
    }
    fn category(&self) -> ToolCategory {
        ToolCategory::Finance
    }
    async fn call(&self, input: Value, _ctx: &ToolContext) -> Result<ToolResult, ToolError> {
        let samples = input["samples"]
            .as_array()
            .ok_or_else(|| te("samples不能为空".into()))?;
        if samples.is_empty() {
            return Ok(ToolResult::success(r#"{"error":"样本为空"}"#.to_string()));
        }

        // 提取样本数据
        let scores: Vec<f64> = samples
            .iter()
            .filter_map(|s| s["attention_score"].as_f64())
            .collect();
        let returns: Vec<f64> = samples
            .iter()
            .filter_map(|s| s["return_pct"].as_f64())
            .collect();
        if scores.len() < 5 {
            return Ok(ToolResult::success(format!(
                r#"{{"error":"样本不足，需要至少5个样本","samples_count":{}}}"#,
                samples.len()
            )));
        }

        // 简单网格搜索: 测试 5 组权重维度
        let weight_sets: Vec<(&str, Vec<f64>)> = vec![
            ("默认", vec![0.25, 0.20, 0.25, 0.15, 0.15]),
            ("强趋势", vec![0.35, 0.15, 0.15, 0.10, 0.25]),
            ("弱信号", vec![0.15, 0.25, 0.30, 0.20, 0.10]),
            ("均衡", vec![0.20, 0.20, 0.20, 0.20, 0.20]),
            ("极致冷门", vec![0.15, 0.30, 0.30, 0.20, 0.05]),
        ];

        // 对每个权重组合计算与收益的负相关性（低关注度→高收益是理想的）
        let mut results: Vec<serde_json::Value> = weight_sets
            .iter()
            .map(|(name, ws)| {
                let weighted_scores: Vec<f64> = samples
                    .iter()
                    .map(|s| {
                        let turnover = s["turnover_rate"].as_f64().unwrap_or(0.0);
                        let news = s["news_count"].as_f64().unwrap_or(0.0);
                        let report = s["report_count"].as_f64().unwrap_or(0.0);
                        let visit = s["visit_count"].as_f64().unwrap_or(0.0);
                        let mcap = s["market_cap"].as_f64().unwrap_or(0.0);
                        let score = turnover / 10.0 * ws[0]
                            + news / 5.0 * ws[1]
                            + report / 5.0 * ws[2]
                            + visit / 3.0 * ws[3]
                            + (mcap / 500.0).min(1.0) * ws[4];
                        score * 100.0
                    })
                    .collect();

                // 计算相关系数: 低分数→高收益 = 好的权重
                let n = samples.len() as f64;
                let mean_x = weighted_scores.iter().sum::<f64>() / n;
                let mean_y = returns.iter().sum::<f64>() / n;
                let (num, den_x, den_y) = weighted_scores.iter().zip(returns.iter()).fold(
                    (0.0, 0.0, 0.0),
                    |(n, dx, dy), (&x, &y)| {
                        (
                            n + (x - mean_x) * (y - mean_y),
                            dx + (x - mean_x).powi(2),
                            dy + (y - mean_y).powi(2),
                        )
                    },
                );
                let correlation = if den_x > 0.0 && den_y > 0.0 {
                    num / (den_x.sqrt() * den_y.sqrt())
                } else {
                    0.0
                };

                // 理想的相关性是负的（低分数→高收益）
                let effectiveness = -correlation;

                serde_json::json!({
                    "name": name,
                    "weights": ws,
                    "correlation": (correlation * 100.0).round() / 100.0,
                    "effectiveness": (effectiveness * 100.0).round() / 100.0,
                })
            })
            .collect();

        // 按效果排序
        results.sort_by(|a, b| {
            b["effectiveness"]
                .as_f64()
                .unwrap_or(0.0)
                .partial_cmp(&a["effectiveness"].as_f64().unwrap_or(0.0))
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        let best = results.first();
        Ok(ToolResult::success(
            serde_json::json!({
                "best_weights": best.map(|b| b["weights"].clone()),
                "best_name": best.map(|b| b["name"].clone()),
                "best_correlation": best.map(|b| b["correlation"].clone()),
                "samples": results.len(),
                "note": "理想权重应使 attention_score 与 return 负相关（低关注→高收益）"
            })
            .to_string(),
        ))
    }
}

// ── 3b. StockFundamentalsReportTool (Phase 2) ──
// 工作流 t-fundamentals-data 节点调用此工具,生成预聚合的 markdown 基本面报告
// (含 health_score / valuation_state / 同比环比 / 估值带),供 a-fundamentals agent
// 启动时直接消费。避免 LLM 在大量原始财报上重复计算基础比率。
pub struct StockFundamentalsReportTool {
    pub client: Arc<AStockClient>,
}
impl StockFundamentalsReportTool {
    pub fn new(c: Arc<AStockClient>) -> Self {
        Self { client: c }
    }
}
#[async_trait]
impl Tool for StockFundamentalsReportTool {
    fn name(&self) -> &str {
        "get_fundamentals_report_markdown"
    }
    fn description(&self) -> &str {
        "获取基本面分析报告(预聚合 markdown)：PE/PB/ROE/同比环比/估值带/0-100 健康度评分与质量等级。返回字符串,直接消费"
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
        // 1. 拉取实时行情
        let quote = self
            .client
            .get_quote(code)
            .await
            .map_err(|e| te(format!("get_quote 失败: {e}")))?;
        // 2. 拉取财务数据
        let financials = self
            .client
            .get_financials(code)
            .await
            .map_err(|e| te(format!("get_financials 失败: {e}")))?;
        // 3. 生成报告 + markdown (Phase 2 迁移后 FundamentalsAnalyzer 位于 astock-data)
        let report = axagent_astock_data::fundamentals_report::FundamentalsAnalyzer::generate(
            code,
            &quote,
            &financials,
        );
        Ok(ToolResult::success(report.to_markdown()))
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
        // Phase 2: 基本面报告工具,被 t-fundamentals-data 工作流节点调用
        Arc::new(StockFundamentalsReportTool::new(client.clone())),
        Arc::new(StockNewsTool::new(client.clone())),
        Arc::new(StockMoneyFlowTool::new(client.clone())),
        Arc::new(StockHotStocksTool::new(client.clone())),
        Arc::new(StockIndustryRankTool::new(client.clone())),
        Arc::new(StockAnnouncementsTool::new(client.clone())),
        Arc::new(StockConsensusEPSTool::new(client.clone())),
        Arc::new(SearchStockTool::new(client.clone())),
        Arc::new(SearchNewsTool::new(client.clone())),
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
        Arc::new(StockSectorInfoTool::new(client.clone())),
        Arc::new(ComputeAttentionScoreTool::new(client.clone())),
        Arc::new(ComputeIndustryPositionTool::new(client.clone())),
        Arc::new(CheckExitSignalsTool::new(client.clone())),
        Arc::new(ComputeSerenityPerformanceTool::new(client.clone())),
        Arc::new(VerifyCatalystsTool::new(client.clone())),
        Arc::new(OptimizeAttentionWeightsTool::new(client)),
    ]);
}
