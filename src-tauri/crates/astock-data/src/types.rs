use serde::{Deserialize, Serialize};

/// 实时行情
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StockQuote {
    pub code: String,
    pub name: String,
    pub price: f64,
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub volume: f64,
    pub amount: f64,
    pub change_pct: f64,
    pub turnover_rate: f64,
    pub pe: Option<f64>,
    pub pb: Option<f64>,
    pub total_mv: Option<f64>,
    pub timestamp: String,
}

/// K线数据
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KLine {
    pub date: String,
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
    pub volume: f64,
    pub amount: f64,
    pub turnover_rate: Option<f64>,
}

/// 财务报告
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FinancialReport {
    pub stock_code: String,
    pub report_date: String,
    pub revenue: Option<f64>,
    pub net_profit: Option<f64>,
    pub eps: Option<f64>,
    pub bps: Option<f64>,
    pub roe: Option<f64>,
    pub debt_ratio: Option<f64>,
    pub gross_margin: Option<f64>,
    pub net_margin: Option<f64>,
    pub revenue_yoy: Option<f64>,
    pub profit_yoy: Option<f64>,
}

/// 新闻/公告条目
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NewsItem {
    pub title: String,
    pub summary: String,
    pub source: String,
    pub url: String,
    pub publish_time: String,
    pub sentiment_score: Option<f64>,
}

/// 资金流向
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MoneyFlow {
    pub date: String,
    pub main_net_inflow: f64,
    pub super_large_net: f64,
    pub large_net: f64,
    pub medium_net: f64,
    pub small_net: f64,
}

/// 龙虎榜条目
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DragonTigerEntry {
    pub stock_code: String,
    pub date: String,
    pub dept_name: String,
    pub buy_amount: f64,
    pub sell_amount: f64,
    pub net_amount: f64,
    pub reason: Option<String>,
}

/// 限售解禁
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LockupSchedule {
    pub stock_code: String,
    pub stock_name: String,
    pub unlock_date: String,
    pub unlock_shares: f64,
    pub unlock_ratio: f64,
    pub shareholder: Option<String>,
}

/// 股票搜索结果
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StockSearchResult {
    pub code: String,
    pub name: String,
    pub market: String,
}

/// 批量原始数据
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StockRawData {
    pub quote: StockQuote,
    pub klines: Vec<KLine>,
    pub financials: Vec<FinancialReport>,
    pub news: Vec<NewsItem>,
    pub money_flow: Option<MoneyFlow>,
    pub dragon_tiger: Vec<DragonTigerEntry>,
    pub lockup: Vec<LockupSchedule>,
}
