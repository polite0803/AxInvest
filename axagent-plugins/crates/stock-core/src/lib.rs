// SPDX-License-Identifier: AGPL-3.0-only
//! 股票域**纯逻辑**（foundation）。
//!
//! 为什么单独成 crate 而不是留在 `analysis-engine`：插件侧 worker 只能依赖 foundation
//! （零 `axagent-*` 依赖），而 `analysis-engine` 是 implementor —— 插件链接它会把
//! `dao` / `entities` / `astock-data` 一起拖进插件进程。把纯函数放在这一层，
//! 宿主与插件就能链接**同一份实现**（不是复制粘贴的第二份词表）。
//!
//! ⚠ 本 crate 只收纯函数：不查库、不读文件、不取时钟、不碰 AppState；
//! 需要宿主状态或外部数据的能力走接缝，不进这里。

// 纯函数权威已上收 `axagent-harness`（foundation），本 crate 保留路径级 re-export：
// 插件侧 `axagent_stock_core::*` 的既有消费路径不变；宿主则直接引 harness，
// 不再因这些纯函数链接插件内 foundation crate（避免第二套定义与宿主反向依赖）。
pub use axagent_harness::{
    stock_costs as costs, stock_factors as factors, stock_intent_parser as intent_parser,
};

pub use axagent_harness::stock_hit_outcome as hit_outcome;

// 股票 worker 本地计算命令的纯函数权威。原实现位于 analysis-engine / astock-data
// （implementor，会把 dao/entities/网络客户端拖进插件进程）；这里迁入插件侧 foundation，
// 宿主仍可继续链接原 crate，插件独立仓只需本 crate + 协议 SDK。
pub mod concept_index;
pub mod evidence_weight;
pub mod factor_analysis;
pub mod fundamentals_report;
pub mod screener;
pub mod sector_coherence;
pub mod sentiment_analysis;

/// 热门候选兜底池（沪深300 核心成分股，覆盖主要行业）。
///
/// 与 analysis-engine::recommender::FALLBACK_STOCKS 同源；股票 worker 的
/// `stock_discover_stock_candidates` 仅用这份常量，不把整个推荐器拖进插件进程。
pub const FALLBACK_STOCKS: &[(&str, &str)] = &[
    ("600519", "贵州茅台"),
    ("000858", "五粮液"),
    ("300750", "宁德时代"),
    ("600036", "招商银行"),
    ("601318", "中国平安"),
    ("000333", "美的集团"),
    ("002475", "立讯精密"),
    ("600276", "恒瑞医药"),
    ("300059", "东方财富"),
    ("000651", "格力电器"),
    ("002415", "海康威视"),
    ("600900", "长江电力"),
    ("601888", "中国中免"),
    ("300014", "亿纬锂能"),
    ("002594", "比亚迪"),
    ("601012", "隆基绿能"),
    ("000001", "平安银行"),
    ("600030", "中信证券"),
    ("000002", "万科A"),
    ("601166", "兴业银行"),
    ("601899", "紫金矿业"),
    ("300124", "汇川技术"),
    ("600809", "山西汾酒"),
    ("002714", "牧原股份"),
    ("000568", "泸州老窖"),
    ("603259", "药明康德"),
    ("600887", "伊利股份"),
    ("002230", "科大讯飞"),
    ("300274", "阳光电源"),
];

/// 持仓摘要（与 harness::PositionSummary 同形）及组合风险纯计算。
///
/// 原 `analysis-engine::portfolio_risk::PortfolioRiskManager` 的薄封装依赖 trading 模块
/// 的 DB 实现；这里只复制算法入口与 DTO，供插件 worker 反序列化接缝出参并计算指标。
pub mod portfolio_risk {
    use std::collections::HashMap;

    use axagent_harness::PositionSummary;
    use serde::{Deserialize, Serialize};

    #[derive(Debug, Clone, Serialize, Deserialize)]
    #[serde(rename_all = "camelCase")]
    pub struct PortfolioRiskMetrics {
        pub total_positions: usize,
        pub total_market_value: f64,
        pub top_concentration_pct: f64,
        pub sector_exposure: HashMap<String, f64>,
        pub diversification_score: u32,
        pub risk_level: String,
        pub warning: Option<String>,
        pub correlation_risk: String,
    }

    pub struct PortfolioRiskManager;

    impl PortfolioRiskManager {
        pub fn compute_from_positions(positions: &[PositionSummary]) -> PortfolioRiskMetrics {
            let priced: Vec<f64> = positions.iter().filter_map(|p| p.market_value).collect();
            let total_mv: f64 = priced.iter().sum();
            let max_mv = priced.into_iter().fold(0.0_f64, f64::max);
            let top_concentration_pct = if total_mv > 0.0 {
                max_mv / total_mv * 100.0
            } else {
                0.0
            };
            let mut sector_exposure: HashMap<String, f64> = HashMap::new();
            for p in positions {
                if let (Some(mv), Some(sector)) = (p.market_value, &p.sector_name)
                    && !sector.is_empty()
                    && total_mv > 0.0
                {
                    *sector_exposure.entry(sector.clone()).or_default() += mv / total_mv * 100.0;
                }
            }
            let max_sector_pct = sector_exposure.values().copied().fold(0.0_f64, f64::max);
            let total_market_value: f64 =
                positions.iter().map(|p| p.market_value.unwrap_or(0.0)).sum();
            let n = positions.len();
            let risk_level = if n == 0 {
                "无持仓".to_string()
            } else if top_concentration_pct > 50.0 || max_sector_pct > 60.0 {
                "高风险".to_string()
            } else if top_concentration_pct > 30.0 || max_sector_pct > 40.0 {
                "中高风险".to_string()
            } else if top_concentration_pct > 20.0 || max_sector_pct > 30.0 {
                "中等风险".to_string()
            } else {
                "低风险".to_string()
            };
            let diversification_score =
                if n >= 8 && top_concentration_pct <= 15.0 && max_sector_pct < 30.0 {
                    90
                } else if n >= 5 && top_concentration_pct <= 25.0 && max_sector_pct < 40.0 {
                    70
                } else if n >= 3 && top_concentration_pct <= 35.0 {
                    50
                } else if n >= 1 {
                    30
                } else {
                    0
                };
            let warning = {
                let mut warns = Vec::new();
                if top_concentration_pct > 40.0 {
                    warns.push(format!("单股集中度 {top_concentration_pct:.0}% 过高，建议 ≤30%"));
                } else if top_concentration_pct > 30.0 {
                    warns
                        .push(format!("单股集中度 {top_concentration_pct:.0}% 偏高，关注分散风险"));
                }
                if max_sector_pct > 50.0 {
                    warns.push(format!("行业暴露 {max_sector_pct:.0}% 过高，建议 ≤40%"));
                }
                if n < 3 && n > 0 {
                    warns.push(format!("仅 {n} 只持仓，分散度不足，建议 ≥3 只"));
                }
                (!warns.is_empty()).then(|| warns.join("；"))
            };
            let correlation_risk = if n >= 5 && max_sector_pct < 30.0 {
                "低"
            } else if n >= 3 && max_sector_pct < 50.0 {
                "中"
            } else {
                "高"
            }
            .to_string();

            PortfolioRiskMetrics {
                total_positions: n,
                total_market_value,
                top_concentration_pct,
                sector_exposure,
                diversification_score,
                risk_level,
                warning,
                correlation_risk,
            }
        }
    }
}
