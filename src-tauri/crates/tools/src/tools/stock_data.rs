use crate::{Tool, ToolCategory, ToolContext, ToolError, ToolResult};
use async_trait::async_trait;
use axagent_astock_data::AStockClient;
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
        let code = input["stock_code"].as_str().unwrap_or("000001");
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
        let code = input["stock_code"].as_str().unwrap_or("000001");
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
        let code = input["stock_code"].as_str().unwrap_or("000001");
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
        let code = input["stock_code"].as_str().unwrap_or("000001");
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
        let code = input["stock_code"].as_str().unwrap_or("000001");
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
        let code = input["stock_code"].as_str().unwrap_or("000001");
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
        let code = input["stock_code"].as_str().unwrap_or("000001");
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
        let kw = input["keyword"].as_str().unwrap_or("000001");
        let r = self
            .client
            .search_stock(kw)
            .await
            .map_err(|e| te(e.to_string()))?;
        Ok(ToolResult::success(serde_json::to_string(&r).unwrap_or_default()))
    }
}

// ── 11-13. Algorithm tools (stubs — real logic in workflow engine) ──
macro_rules! algo_tool {
    ($name:ident, $tool_name:literal, $desc:literal) => {
        pub struct $name { pub client: Arc<AStockClient> }
        impl $name { pub fn new(c: Arc<AStockClient>) -> Self { Self { client: c } } }
        #[async_trait]
        impl Tool for $name {
            fn name(&self) -> &str { $tool_name }
            fn description(&self) -> &str { $desc }
            fn input_schema(&self) -> Value { json!({"type":"object","properties":{}}) }
            fn category(&self) -> ToolCategory { ToolCategory::Finance }
            async fn call(&self, _input: Value, _ctx: &ToolContext) -> Result<ToolResult, ToolError> {
                Ok(ToolResult::success(r#"{"status":"ok"}"#))
            }
        }
    };
}
algo_tool!(
    ComputeScoringTool,
    "compute_scoring",
    "六维度技术评分：趋势/乖离/MACD/量能/RSI/支撑+价值修正"
);
algo_tool!(
    ComputeValuationTool,
    "compute_valuation",
    "DCF两阶段+格雷厄姆+F-Score+护城河量化估值"
);
algo_tool!(
    ComputeRiskTool,
    "compute_portfolio_risk",
    "组合风险指标：集中度/分散度/行业暴露"
);

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
        let code = input["stock_code"].as_str().unwrap_or("000001");
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
        let code = input["stock_code"].as_str().unwrap_or("000001");
        let r = self
            .client
            .get_institutional_visits(code)
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
        Arc::new(StockInstitutionalVisitsTool::new(client)),
    ]);
}
