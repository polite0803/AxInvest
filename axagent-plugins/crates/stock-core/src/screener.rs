// SPDX-License-Identifier: AGPL-3.0-only
//! 股票候选打分纯函数。
//!
//! 从 `analysis-engine::screener` 迁入插件侧 foundation：股票 worker 只需要
//! `score_candidate`，不需要原模块的网络筛选器（后者会拖入 `astock-data` 客户端）。

#![allow(clippy::unnecessary_sort_by)]

pub use axagent_harness::market_data::StockQuote;
pub use axagent_harness::stock_market_types::{HotStock, IndustryRank};

/// 筛选结果（保留字段形状以兼容宿主出参契约）。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScreenResult {
    pub stock_code: String,
    pub stock_name: String,
    pub price: f64,
    pub change_pct: f64,
    pub reasons: Vec<String>,
    pub score: u32,
    pub matched_themes: Option<Vec<String>>,
}

/// 候选快照（主题筛选纯逻辑复用）。
#[derive(Debug, Clone)]
pub struct StockSnapshot {
    pub code: String,
    pub name: String,
    pub price: f64,
    pub change_pct: f64,
    pub turnover_rate: f64,
    pub pe: Option<f64>,
    pub pb: Option<f64>,
}

pub struct StockScreener;

impl StockScreener {
    /// 单只候选的量化打分（发现编排的纯段）。
    ///
    /// 编排（宇宙来源 + 逐票取行情）留在调用侧：宿主 `discover_stock_candidates` 已退役
    /// （#329），生产编排落在 plugin-stock 的 `stock_discover_stock_candidates`
    /// （宇宙 = 热门股前 30 + 行业龙头前 10，行情走 `market.quote` 接缝逐票），两边共享
    /// 这一份打分 ⇒ 单一权威。基线 5 分；涨幅 >2%、换手 >3%、低 PE 各加分；ST 扣 10。
    /// 排序与截断（score 降序取前 20）也属编排，不在本函数。
    pub fn score_candidate(stock_code: &str, stock_name: &str, quote: &StockQuote) -> ScreenResult {
        let mut reasons = Vec::new();
        let mut score = 5u32;
        if quote.change_pct.abs() > 2.0 {
            let dir = if quote.change_pct > 0.0 { "涨" } else { "跌" };
            reasons.push(format!("{}幅 {:.2}%", dir, quote.change_pct));
            score += (quote.change_pct.abs() * 2.0) as u32;
        }
        if quote.turnover_rate > 3.0 {
            reasons.push(format!("换手 {:.2}%", quote.turnover_rate));
            score += (quote.turnover_rate / 2.0) as u32;
        }

        // PE/PB 估值信号
        if let Some(pe) = quote.pe
            && pe < 15.0
            && pe > 0.0
        {
            reasons.push(format!("低PE {:.1}", pe));
            score += 5;
        }

        // ST 风险提示
        if quote.is_st {
            reasons.push("ST警示".to_string());
            score = score.saturating_sub(10);
        }

        ScreenResult {
            stock_code: stock_code.to_string(),
            stock_name: stock_name.to_string(),
            price: quote.price,
            change_pct: quote.change_pct,
            reasons,
            score,
            matched_themes: None,
        }
    }
}
