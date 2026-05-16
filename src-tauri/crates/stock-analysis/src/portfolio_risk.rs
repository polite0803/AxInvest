use std::collections::HashMap;

/// 组合风险指标
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PortfolioRiskMetrics {
    pub total_positions: usize,
    pub total_market_value: f64,
    pub top_concentration_pct: f64,
    pub sector_exposure: HashMap<String, f64>,
    pub diversification_score: u32,
    pub risk_level: String,
    pub warning: Option<String>,
}

pub struct PortfolioRiskManager;

impl PortfolioRiskManager {
    /// 计算组合风险指标
    pub fn compute_from_positions(
        positions: &[super::trading::PositionSummary],
    ) -> PortfolioRiskMetrics {
        let total_positions = positions.len();
        if total_positions == 0 {
            return PortfolioRiskMetrics {
                total_positions: 0,
                total_market_value: 0.0,
                top_concentration_pct: 0.0,
                sector_exposure: HashMap::new(),
                diversification_score: 0,
                risk_level: "无持仓".to_string(),
                warning: Some("暂无持仓记录，请先添加交易记录或手动录入持仓。".to_string()),
            };
        }
        let total_mv: f64 = positions.iter().filter_map(|p| p.market_value).sum();

        // 最大单股集中度
        let max_mv = positions
            .iter()
            .filter_map(|p| p.market_value)
            .max_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))
            .unwrap_or(0.0);
        let concentration = if total_mv > 0.0 {
            (max_mv / total_mv) * 100.0
        } else {
            0.0
        };

        // 分散度评分
        let diversification = if total_positions >= 8 && concentration <= 15.0 {
            90
        } else if total_positions >= 5 && concentration <= 25.0 {
            70
        } else if total_positions >= 3 && concentration <= 35.0 {
            50
        } else if total_positions >= 1 {
            30
        } else {
            0
        };

        // 风险等级
        let risk_level = if concentration > 50.0 {
            "高风险".to_string()
        } else if concentration > 30.0 {
            "中高风险".to_string()
        } else if concentration > 20.0 {
            "中等风险".to_string()
        } else {
            "低风险".to_string()
        };

        // 生成警告
        let mut warning = None;
        if concentration > 40.0 {
            warning = Some(format!(
                "单股集中度 {:.0}% 过高，建议 ≤30%。当前最大持仓占比过高。",
                concentration
            ));
        } else if concentration > 30.0 {
            warning = Some(format!("单股集中度 {:.0}% 偏高，关注分散风险。", concentration));
        }
        if total_positions < 3 && total_positions > 0 {
            let msg = format!("仅{}只持仓，分散度不足，建议 ≥3 只。", total_positions);
            warning = Some(warning.map_or(msg.clone(), |w| format!("{} {}", w, msg)));
        }

        // 行业暴露计算
        let mut sector_exposure: HashMap<String, f64> = HashMap::new();
        for p in positions {
            if let (Some(mv), Some(sector)) = (p.market_value, &p.sector_name) {
                if !sector.is_empty() && total_mv > 0.0 {
                    *sector_exposure.entry(sector.clone()).or_default() += (mv / total_mv) * 100.0;
                }
            }
        }

        PortfolioRiskMetrics {
            total_positions,
            total_market_value: total_mv,
            top_concentration_pct: concentration,
            sector_exposure,
            diversification_score: diversification,
            risk_level,
            warning,
        }
    }
}
