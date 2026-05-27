use serde::{Deserialize, Serialize};

/// 实时行情
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StockQuote {
    pub code: String,
    pub name: String,
    pub price: f64,
    /// 昨收价
    pub pre_close: f64,
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
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub circulating_mv: Option<f64>,
    /// 涨停价
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit_up: Option<f64>,
    /// 跌停价
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit_down: Option<f64>,
    /// 是否ST股票（含*ST）
    #[serde(default)]
    pub is_st: bool,
    pub timestamp: String,
}

/// 判断A股市场类型
///
/// 根据股票代码前缀识别市场板块：
/// - `6` 开头且 "688" → 科创板 (star)
/// - `6` 开头（非688） → 上海主板 (main_sh)
/// - `0` 开头 → 深圳主板 (main_sz)
/// - `3` 开头 → 创业板 (chinext)
/// - `8` 开头 → 北交所 (bj)
pub fn detect_market_type(code: &str) -> &str {
    match code.chars().next() {
        Some('6') if code.starts_with("688") => "star",
        Some('6') => "main_sh",
        Some('0') => "main_sz",
        Some('3') => "chinext",
        Some('8') => "bj",
        _ => "unknown",
    }
}

/// 获取A股各板块涨跌停幅度（百分比）
///
/// - 科创板/创业板: ±20%
/// - 北交所: ±30%
/// - 主板: ±10%
pub fn get_price_limit_pct(market_type: &str) -> f64 {
    match market_type {
        "star" | "chinext" => 20.0,
        "bj" => 30.0,
        _ => 10.0,
    }
}

/// 获取ST股票的涨跌停幅度
///
/// ST股票统一±5%，非ST按板块规则
pub fn get_st_price_limit_pct(is_st: bool, market_type: &str) -> f64 {
    if is_st {
        5.0
    } else {
        get_price_limit_pct(market_type)
    }
}

/// K线数据
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
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
#[serde(rename_all = "camelCase")]
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
    #[serde(default)]
    pub total_assets: Option<f64>,
    #[serde(default)]
    pub operating_cash_flow: Option<f64>,
    #[serde(default)]
    pub capital_expenditure: Option<f64>,
    #[serde(default)]
    pub free_cash_flow: Option<f64>,
    #[serde(default)]
    pub current_ratio: Option<f64>,
    #[serde(default)]
    pub quick_ratio: Option<f64>,
}

/// 新闻/公告条目
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
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
#[serde(rename_all = "camelCase")]
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
#[serde(rename_all = "camelCase")]
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
#[serde(rename_all = "camelCase")]
pub struct LockupSchedule {
    pub stock_code: String,
    pub stock_name: String,
    pub unlock_date: String,
    pub unlock_shares: f64,
    pub unlock_ratio: f64,
    pub shareholder: Option<String>,
}

/// 融资融券数据
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MarginData {
    pub stock_code: String,
    pub date: String,
    pub margin_buy: f64,        // 融资买入额
    pub margin_balance: f64,    // 融资余额
    pub short_sell_volume: f64, // 融券卖出量
    pub short_balance: f64,     // 融券余量
}

/// 北向资金持仓
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NorthBoundHolding {
    pub stock_code: String,
    pub date: String,
    pub holding_shares: f64, // 持股数量
    pub holding_ratio: f64,  // 持股占比
    pub change_shares: f64,  // 变动数量
}

/// 行业分类
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SectorInfo {
    pub stock_code: String,
    pub sector_name: String, // 申万一级行业
    pub sub_sector: String,  // 申万二级行业
    pub concept_tags: Vec<String>,
    #[serde(default)]
    pub avg_pe: Option<f64>,
    #[serde(default)]
    pub avg_pb: Option<f64>,
}

/// 股东增减持
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShareholderTrade {
    pub stock_code: String,
    pub date: String,
    pub shareholder_name: String,
    pub trade_type: String, // 增持/减持
    pub shares: f64,
    pub price: f64,
    pub reason: Option<String>,
}

/// 除权除息
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DividendRecord {
    pub stock_code: String,
    pub ex_date: String,
    pub dividend_per_share: f64, // 每股分红
    pub bonus_share_ratio: f64,  // 送转比例
    pub record_date: String,
}

/// K线周期枚举（兼容券商API代码）
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum KLinePeriod {
    #[serde(rename = "5")]
    Min5,
    #[serde(rename = "15")]
    Min15,
    #[serde(rename = "30")]
    Min30,
    #[serde(rename = "60")]
    Min60,
    Daily,
    Weekly,
    Monthly,
}

impl KLinePeriod {
    /// 转换为东方财富 API 的 period 代码
    pub fn to_em_code(&self) -> &str {
        match self {
            KLinePeriod::Min5 => "5",
            KLinePeriod::Min15 => "15",
            KLinePeriod::Min30 => "30",
            KLinePeriod::Min60 => "60",
            KLinePeriod::Daily => "101",
            KLinePeriod::Weekly => "102",
            KLinePeriod::Monthly => "103",
        }
    }
}

/// 股票搜索结果
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StockSearchResult {
    pub code: String,
    pub name: String,
    pub market: String,
}

/// 研报
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResearchReport {
    pub title: String,
    pub institution: String,
    pub analyst: Option<String>,
    pub rating: Option<String>,
    pub target_price: Option<f64>,
    pub eps_forecast: Vec<EpsForecast>,
    pub publish_date: String,
    pub pdf_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EpsForecast {
    pub year: String,
    pub eps: Option<f64>,
}

/// 机构一致预期EPS
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConsensusEPS {
    pub stock_code: String,
    pub consensus_eps: Option<f64>,
    pub consensus_target_price: Option<f64>,
    pub rating_avg: Option<String>,
    pub rating_count: Option<i32>,
    pub year: String,
}

/// 同花顺强势股
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HotStock {
    pub stock_code: String,
    pub stock_name: String,
    pub change_pct: f64,
    pub turnover_rate: Option<f64>,
    pub reason_tags: Vec<String>,
    pub sector: Option<String>,
}

/// 概念板块三维归属
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConceptBlocks {
    pub stock_code: String,
    pub industry: String,
    pub concepts: Vec<BlockItem>,
    pub regions: Vec<BlockItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BlockItem {
    pub name: String,
    pub change_pct: Option<f64>,
}

/// 公告
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Announcement {
    pub title: String,
    pub stock_code: String,
    pub stock_name: Option<String>,
    pub announce_date: String,
    pub ann_type: Option<String>,
    pub pdf_url: Option<String>,
}

/// 行业排名
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IndustryRank {
    pub industry_name: String,
    pub change_pct: f64,
    pub turnover: Option<f64>,
    pub leader_code: Option<String>,
    pub leader_name: Option<String>,
    pub leader_change_pct: Option<f64>,
}

/// 财联社快讯
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClsFlashItem {
    pub title: String,
    pub content: String,
    pub publish_time: String,
    pub source: Option<String>,
}

/// 全市场龙虎榜
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MarketDragonTiger {
    pub stock_code: String,
    pub stock_name: String,
    pub date: String,
    pub net_buy: f64,
    pub buy_amount: f64,
    pub sell_amount: f64,
    pub reason: Option<String>,
}

/// 大宗交易
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BlockTrade {
    pub stock_code: String,
    pub stock_name: String,
    pub trade_date: String,
    pub price: f64,
    pub volume: f64,
    pub amount: f64,
    pub buyer_dept: Option<String>,
    pub seller_dept: Option<String>,
    pub discount_pct: Option<f64>,
}

/// 机构调研记录
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstitutionalVisit {
    pub stock_code: String,
    pub stock_name: String,
    pub visit_date: String,
    pub institution_count: i32,
    pub main_content: String,
    pub visit_type: Option<String>,
}

/// 北向资金分钟级流向
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NorthBoundFlow {
    pub date: String,
    pub sh_flow: f64,
    pub sz_flow: f64,
    pub total_flow: f64,
    pub timestamp: Option<String>,
}

/// 批量原始数据
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StockRawData {
    pub quote: StockQuote,
    pub klines: Vec<KLine>,
    pub financials: Vec<FinancialReport>,
    pub news: Vec<NewsItem>,
    pub money_flow: Option<MoneyFlow>,
    pub dragon_tiger: Vec<DragonTigerEntry>,
    pub lockup: Vec<LockupSchedule>,
    pub margin_data: Option<MarginData>,
    pub north_bound: Option<NorthBoundHolding>,
    pub sector_info: Option<SectorInfo>,
    pub shareholder_trades: Vec<ShareholderTrade>,
    pub dividend_records: Vec<DividendRecord>,
    pub research_reports: Vec<ResearchReport>,
    pub consensus_eps: Option<ConsensusEPS>,
    pub concept_blocks: Option<ConceptBlocks>,
    pub announcements: Vec<Announcement>,
}

/// 市场级原始数据
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MarketRawData {
    pub hot_stocks: Vec<HotStock>,
    pub industry_ranking: Vec<IndustryRank>,
    pub cls_flash: Vec<ClsFlashItem>,
    pub market_dragon_tiger: Vec<MarketDragonTiger>,
    pub north_bound_flow: Option<NorthBoundFlow>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_market_sh_main() {
        assert_eq!(detect_market_type("600519"), "main_sh");
    }

    #[test]
    fn test_detect_market_star() {
        assert_eq!(detect_market_type("688001"), "star");
    }

    #[test]
    fn test_detect_market_sz_main() {
        assert_eq!(detect_market_type("000001"), "main_sz");
    }

    #[test]
    fn test_detect_market_chinext() {
        assert_eq!(detect_market_type("300750"), "chinext");
    }

    #[test]
    fn test_detect_market_bj() {
        assert_eq!(detect_market_type("830946"), "bj");
    }

    #[test]
    fn test_price_limit_main() {
        assert!((get_price_limit_pct("main_sh") - 10.0).abs() < 1e-6);
    }

    #[test]
    fn test_price_limit_star() {
        assert!((get_price_limit_pct("star") - 20.0).abs() < 1e-6);
    }

    #[test]
    fn test_price_limit_bj() {
        assert!((get_price_limit_pct("bj") - 30.0).abs() < 1e-6);
    }

    #[test]
    fn test_st_price_limit() {
        assert!((get_st_price_limit_pct(true, "main_sh") - 5.0).abs() < 1e-6);
        assert!((get_st_price_limit_pct(false, "main_sh") - 10.0).abs() < 1e-6);
    }

    #[test]
    fn test_kline_period_to_em_code() {
        assert_eq!(KLinePeriod::Daily.to_em_code(), "101");
        assert_eq!(KLinePeriod::Weekly.to_em_code(), "102");
        assert_eq!(KLinePeriod::Min5.to_em_code(), "5");
    }

    #[test]
    fn test_stock_quote_serialization() {
        let quote = StockQuote {
            code: "600519".to_string(),
            name: "茅台".to_string(),
            price: 1800.0,
            pre_close: 1785.0,
            open: 1790.0,
            high: 1810.0,
            low: 1785.0,
            volume: 5000000.0,
            amount: 9000000000.0,
            change_pct: 0.56,
            turnover_rate: 0.3,
            pe: Some(35.0),
            pb: Some(12.0),
            total_mv: Some(2250000000000.0),
            circulating_mv: None,
            limit_up: None,
            limit_down: None,
            is_st: false,
            timestamp: "2025-01-15 14:00:00".to_string(),
        };
        let json = serde_json::to_string(&quote).unwrap();
        assert!(json.contains("600519"));
        assert!(json.contains("camelCase") || json.contains("changePct"));
        let parsed: StockQuote = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.code, "600519");
    }

    #[test]
    fn test_kline_serialization() {
        let kline = KLine {
            date: "2025-01-15".to_string(),
            open: 10.0,
            high: 11.0,
            low: 9.5,
            close: 10.5,
            volume: 10000.0,
            amount: 105000.0,
            turnover_rate: Some(0.5),
        };
        let json = serde_json::to_string(&kline).unwrap();
        assert!(json.contains("2025-01-15"));
        let _parsed: KLine = serde_json::from_str(&json).unwrap();
    }

    #[test]
    fn test_stock_search_result_serialization() {
        let result = StockSearchResult {
            code: "600519".to_string(),
            name: "贵州茅台".to_string(),
            market: "上海".to_string(),
        };
        let json = serde_json::to_string(&result).unwrap();
        assert!(json.contains("贵州茅台"));
        let _parsed: StockSearchResult = serde_json::from_str(&json).unwrap();
    }
}
