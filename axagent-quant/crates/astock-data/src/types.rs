pub use axagent_harness::market_data::{KLine, StockQuote, StockSearchResult};
// 以下来自 harness as_of DTO 契约
pub use axagent_harness::as_of::{
    AsOfContext, AsOfDataKind, AsOfDataScope, AsOfSource, DegradationEntry,
};
// 财务报告 DTO — 权威定义在 harness
pub use axagent_harness::market_data::FinancialReport;
// A 股市场工具函数 — 权威定义在 harness
pub use axagent_harness::market_data::{
    detect_market_type, get_price_limit_pct, get_st_price_limit_pct,
};

/// 创建行业均值估算的财务报告（所有 API 数据源均失败时的 fallback）
pub fn estimated_financial_report(stock_code: &str) -> FinancialReport {
    let today = crate::as_of::current_date_or_now();
    let market_type = detect_market_type(stock_code);
    let (eps, bps, roe, debt_ratio, gross_margin, net_margin) = match market_type {
        "star" | "chinext" => (0.35, 5.0, 6.0, 35.0, 35.0, 8.0),
        "bj" => (0.20, 3.0, 5.0, 40.0, 30.0, 5.0),
        _ => (0.50, 6.0, 8.0, 50.0, 25.0, 10.0),
    };
    FinancialReport {
        stock_code: stock_code.to_string(),
        report_date: today,
        // 行业均值兜底报告没有真实披露日（P1-1：as-of 按披露日裁，缺即判不可得）
        disclosure_date: None,
        revenue: Some(eps * 20.0 * 100_000_000.0),
        net_profit: Some(eps * 100_000_000.0),
        eps: Some(eps),
        bps: Some(bps),
        roe: Some(roe),
        debt_ratio: Some(debt_ratio),
        gross_margin: Some(gross_margin),
        net_margin: Some(net_margin),
        revenue_yoy: Some(5.0),
        profit_yoy: Some(3.0),
        total_assets: Some(bps * 100_000_000.0),
        operating_cash_flow: None,
        capital_expenditure: None,
        free_cash_flow: None,
        current_ratio: Some(1.5),
        quick_ratio: Some(1.0),
        goodwill: None,
        accounts_receivable: None,
        estimated: Some(true),
    }
}

// ── 行情 DTO 权威定义已上收至 harness（2026-11-02）──
// 保留本模块路径：`astock_data::types::X` 与 `astock_data::X`（lib.rs 的 `pub use types::*`）继续可用，
// 且与 `axagent_harness::stock_market_types::X` 是同一类型。
pub use axagent_harness::stock_market_types::*;

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
            adj_factor: None,
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

    /// 类型身份：`astock_data::types::X` 与 harness 权威定义必须是同一类型。
    ///
    /// DTO 上收后本模块只是 re-export 垫片；若两边各留一份定义（同名异型），
    /// 下面的跨 crate 形参绑定在**编译期**即失败。
    #[test]
    fn test_dto_type_identity_with_harness() {
        fn assert_harness_news_item(_: &axagent_harness::stock_market_types::NewsItem) {}

        let item = NewsItem {
            title: "t".to_string(),
            summary: "s".to_string(),
            source: "src".to_string(),
            url: "u".to_string(),
            publish_time: "2025-01-01".to_string(),
            sentiment_score: None,
        };
        assert_harness_news_item(&item);

        // 反向同理：同一个值可直接以 harness 路径接收，无需任何转换。
        let _: &axagent_harness::stock_market_types::KLinePeriod = &KLinePeriod::Daily;
    }
}
