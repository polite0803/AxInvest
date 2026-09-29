//! 市场 Regime 识别 (Phase 3 - TradingAgents-CN 借鉴)
//!
//! 借鉴 TradingAgents-CN 的"市场 regime 自适应 prompt"机制:
//! 根据 20/60 日均线、布林带宽度、波动率、连涨/连跌天数判断当前市场
//! 状态(Bull/Bear/Sideways/Volatile),让 expert prompt 在不同 regime 下
//! 切换 bias,避免用同一模板看所有周期。
//!
//! 设计原则:
//! - **多维度融合**:单一指标不可靠,综合均线/波动/趋势/连涨连跌
//! - **可解释**:每个 regime 都给出 triggered_rules,LLM 可理解
//! - **A 股定制**:10% 涨跌停、连板/跌停不算连续日(单日波动不可持续)
//! - **轻量**:不需要 ML,纯统计 + 启发式规则

use crate::KLine;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MarketRegime {
    /// 牛市:20/60 日均线多头排列,价格在均线上方,波动率温和
    Bull,
    /// 熊市:20/60 日均线空头排列,价格在均线下方,连续下跌
    Bear,
    /// 震荡:均线缠绕,价格在区间内反复
    Sideways,
    /// 高波动:布林带宽度突增,日波动率超过 50%
    Volatile,
    /// 数据不足(样本 < 20 日)
    #[default]
    Unknown,
}

impl MarketRegime {
    pub fn label(&self) -> &'static str {
        match self {
            MarketRegime::Bull => "牛市",
            MarketRegime::Bear => "熊市",
            MarketRegime::Sideways => "震荡",
            MarketRegime::Volatile => "高波动",
            MarketRegime::Unknown => "未知",
        }
    }

    pub fn emoji(&self) -> &'static str {
        match self {
            MarketRegime::Bull => "🐂",
            MarketRegime::Bear => "🐻",
            MarketRegime::Sideways => "〰️",
            MarketRegime::Volatile => "⚡",
            MarketRegime::Unknown => "❓",
        }
    }

    /// expert prompt 切换建议
    pub fn prompt_bias(&self) -> &'static str {
        match self {
            MarketRegime::Bull => "顺势偏多:关注业绩超预期+资金流入,警惕追高",
            MarketRegime::Bear => "防御为主:关注低估值+稳健现金流,警惕杀估值",
            MarketRegime::Sideways => "精选个股:关注催化剂+预期差,警惕无主线",
            MarketRegime::Volatile => "降低仓位:关注风控+对冲,警惕情绪化交易",
            MarketRegime::Unknown => "数据不足,采用中性策略",
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RegimeReport {
    pub regime: MarketRegime,
    /// 触发该 regime 的规则列表(可解释)
    pub triggered_rules: Vec<String>,
    /// 信心度 0-100
    pub confidence: u8,
    /// 20 日均价
    pub ma20: Option<f64>,
    /// 60 日均价
    pub ma60: Option<f64>,
    /// 20 日年化波动率(%)
    pub volatility_20d: Option<f64>,
    /// 布林带宽度(0-1 比例)
    pub bollinger_width: Option<f64>,
    /// 连涨天数
    pub consecutive_up: i32,
    /// 连跌天数
    pub consecutive_down: i32,
    /// 检测时使用的样本数
    pub samples: usize,
}

pub struct RegimeDetector;

impl RegimeDetector {
    /// 核心检测入口:输入 K 线历史,输出 RegimeReport
    /// 要求至少 20 个交易日样本,否则返回 Unknown
    pub fn detect(klines: &[KLine]) -> RegimeReport {
        if klines.len() < 20 {
            return RegimeReport {
                regime: MarketRegime::Unknown,
                samples: klines.len(),
                ..Default::default()
            };
        }

        let closes: Vec<f64> = klines.iter().map(|k| k.close).collect();
        let mut report = RegimeReport { samples: closes.len(), ..Default::default() };

        // 1. 均线
        if let Some(ma20) = crate::indicators::sma(&closes, 20) {
            report.ma20 = Some(ma20);
        }
        if closes.len() >= 60 {
            if let Some(ma60) = crate::indicators::sma(&closes, 60) {
                report.ma60 = Some(ma60);
            }
        }

        // 2. 波动率(20 日年化,%)
        if let Some(vol) = Self::volatility(&closes, 20) {
            report.volatility_20d = Some(vol);
        }

        // 3. 布林带宽度
        if let Some(bw) = Self::bollinger_width(&closes, 20, 2.0) {
            report.bollinger_width = Some(bw);
        }

        // 4. 连涨/连跌天数
        report.consecutive_up = Self::consecutive_streak(&closes, true);
        report.consecutive_down = Self::consecutive_streak(&closes, false);

        // 5. 判定 regime
        let (regime, rules, confidence) = Self::classify(&report);
        report.regime = regime;
        report.triggered_rules = rules;
        report.confidence = confidence;

        report
    }

    /// 20 日波动率(年化,%) = `σ_daily(样本) × √252`
    ///
    /// 2026-09-29 口径统一（四周期科学化 Phase D）：此前本函数用**总体方差 ÷n**，
    /// 而本仓 `harness::indicators::sharpe_components` 早已修成**样本方差 ÷(n−1)**
    /// （测试名 `sharpe_matches_astock_data_legacy_formula_after_fix` 记录了那次修复），
    /// `harness::indicators::realized_vol_pct` 也是样本口径 ⇒ 同一仓库两种 σ 就是口径漂移。
    /// 现统一委托给 `realized_vol_pct`，本仓只剩一份 σ 实现。
    ///
    /// 两处**已知且刻意**的行为变化：
    /// 1. 数值整体抬高 `√(n/(n−1))`（n=20 ⇒ ×1.026，即 +2.6%）。分类阈值 50/30/15 是
    ///    **市场波动水平的经济刻度**，不是估计器刻度 ⇒ 不随之平移；边界附近的样本
    ///    （如旧值 49.0% → 新值 50.3%）改判属于偏差修正，不是回归。
    /// 2. 含非正价格时返回 `None`（旧实现会算出 inf/NaN 并继续参与打分）。
    fn volatility(closes: &[f64], n: usize) -> Option<f64> {
        axagent_harness::indicators::realized_vol_pct(closes, n)
            .map(|sigma_daily_pct| sigma_daily_pct * (252_f64).sqrt())
    }

    /// 布林带宽度(upper - lower) / mid
    fn bollinger_width(closes: &[f64], n: usize, k: f64) -> Option<f64> {
        crate::indicators::sma(closes, n).and_then(|mid| {
            let slice = &closes[closes.len() - n..];
            let variance: f64 = slice.iter().map(|c| (c - mid).powi(2)).sum::<f64>() / n as f64;
            let std = variance.sqrt();
            let upper = mid + k * std;
            let lower = mid - k * std;
            if mid > 0.0 {
                Some((upper - lower) / mid)
            } else {
                None
            }
        })
    }

    /// 连续上涨(true) 或 连续下跌(false) 的天数
    /// 跳空涨跌停(>=9.5% 单日) 不计入连续日
    fn consecutive_streak(closes: &[f64], up: bool) -> i32 {
        let mut count = 0i32;
        for w in closes.windows(2).rev() {
            let prev = w[0];
            let curr = w[1];
            if prev <= 0.0 {
                break;
            }
            let pct = (curr - prev) / prev;
            let is_limit = pct.abs() >= 0.095;
            if is_limit {
                continue;
            }
            if up && pct > 0.0 {
                count += 1;
            } else if !up && pct < 0.0 {
                count += 1;
            } else {
                break;
            }
        }
        count
    }

    /// 综合判定
    fn classify(report: &RegimeReport) -> (MarketRegime, Vec<String>, u8) {
        let mut rules = Vec::new();
        let mut bull_score = 0i32;
        let mut bear_score = 0i32;
        let mut sideways_score = 0i32;
        let mut volatile_score = 0i32;

        // 规则 1: 均线多头排列 (价格在 MA20 上方 + MA20 > MA60)
        if let (Some(ma20), Some(ma60)) = (report.ma20, report.ma60) {
            // 用 ma20 作为近似"最后价格"位置
            if ma20 > ma60 {
                bull_score += 20;
                rules.push(format!("MA20 {:.2} > MA60 {:.2}(短期均线上穿长期)", ma20, ma60));
            } else if ma20 < ma60 {
                bear_score += 20;
                rules.push(format!("MA20 {:.2} < MA60 {:.2}(短期均线下穿长期)", ma20, ma60));
            } else {
                sideways_score += 10;
                rules.push("MA20 == MA60 临界".into());
            }
        }

        // 规则 2: 连涨/连跌
        if report.consecutive_up >= 5 {
            bull_score += 15;
            rules.push(format!("连涨 {} 日", report.consecutive_up));
        } else if report.consecutive_down >= 5 {
            bear_score += 15;
            rules.push(format!("连跌 {} 日", report.consecutive_down));
        }

        // 规则 3: 波动率
        if let Some(vol) = report.volatility_20d {
            if vol > 50.0 {
                volatile_score += 30;
                rules.push(format!("20 日年化波动率 {:.1}%(高波动阈值 > 50%)", vol));
            } else if vol > 30.0 {
                volatile_score += 10;
            } else if vol < 15.0 {
                // 低波动,纳入震荡候选
                sideways_score += 5;
            }
        }

        // 规则 4: 布林带宽度
        if let Some(bw) = report.bollinger_width {
            if bw > 0.15 {
                volatile_score += 20;
                rules.push(format!("布林带宽度 {:.1}%(>15%%)", bw * 100.0));
            } else if bw < 0.05 {
                sideways_score += 15;
                rules.push(format!("布林带宽度 {:.1}%(收窄,5% 以下)", bw * 100.0));
            }
        }

        // 决策:取最高分
        let scores = [
            (MarketRegime::Bull, bull_score),
            (MarketRegime::Bear, bear_score),
            (MarketRegime::Sideways, sideways_score),
            (MarketRegime::Volatile, volatile_score),
        ];
        let (regime, _top_score) =
            scores.iter().max_by_key(|(_, s)| *s).copied().unwrap_or((MarketRegime::Unknown, 0));

        // 信心度:基于最高分与次高分的差距
        let mut sorted: Vec<i32> = scores.iter().map(|(_, s)| *s).collect();
        sorted.sort_by(|a, b| b.cmp(a));
        let confidence = if sorted[0] == 0 {
            0
        } else if sorted.len() > 1 && sorted[0] - sorted[1] >= 20 {
            (sorted[0] as u8).min(100)
        } else {
            (sorted[0] as u8 / 2).min(60)
        };

        if rules.is_empty() {
            rules.push("数据不足以判断 regime".into());
        }

        (regime, rules, confidence)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::KLine;

    fn fake_kline(close: f64) -> KLine {
        KLine {
            date: "2026-01-01".into(),
            open: close * 0.99,
            high: close * 1.01,
            low: close * 0.98,
            close,
            volume: 1e6,
            amount: 1e8,
            turnover_rate: None,
            adj_factor: None,
        }
    }

    fn kline_series(prices: &[f64]) -> Vec<KLine> {
        prices.iter().map(|&p| fake_kline(p)).collect()
    }

    #[test]
    fn detects_bull_when_ma_aligned_up() {
        // 60 日单调上涨 → MA20 > MA60
        let prices: Vec<f64> = (0..70).map(|i| 10.0 + i as f64 * 0.15).collect();
        let r = RegimeDetector::detect(&kline_series(&prices));
        assert!(r.ma20.is_some());
        assert!(r.ma60.is_some());
        assert!(r.ma20.unwrap() > r.ma60.unwrap());
        // 至少应该是 Bull 或 Sideways(取决于波动率/布林带)
        assert!(matches!(r.regime, MarketRegime::Bull | MarketRegime::Sideways));
    }

    #[test]
    fn detects_bear_when_ma_aligned_down() {
        // 60 日单调下跌 → MA20 < MA60
        let prices: Vec<f64> = (0..70).map(|i| 30.0 - i as f64 * 0.15).collect();
        let r = RegimeDetector::detect(&kline_series(&prices));
        assert!(r.ma20.is_some());
        assert!(r.ma60.is_some());
        assert!(r.ma20.unwrap() < r.ma60.unwrap());
        // 至少应该是 Bear 或 Sideways
        assert!(matches!(r.regime, MarketRegime::Bear | MarketRegime::Sideways));
    }

    #[test]
    fn detects_sideways_when_range_bound() {
        // 60 日震荡 [10.0, 10.05] 微小振幅,真正无趋势
        let prices: Vec<f64> = (0..70).map(|i| 10.0 + (i as f64 * 0.628).sin() * 0.025).collect();
        let r = RegimeDetector::detect(&kline_series(&prices));
        // 震荡市特征: ma20 ≈ ma60 (差距 < 0.05)
        if let (Some(m20), Some(m60)) = (r.ma20, r.ma60) {
            assert!((m20 - m60).abs() < 0.05, "震荡市 ma20 ({}) 和 ma60 ({}) 应非常接近", m20, m60);
        }
    }

    #[test]
    fn detects_volatile_when_big_swings() {
        // 大幅震荡
        let prices: Vec<f64> = (0..30).map(|i| 10.0 + (i as f64 * 0.7).sin() * 3.0).collect();
        let r = RegimeDetector::detect(&kline_series(&prices));
        // 高波动场景
        assert!(r.bollinger_width.unwrap_or(0.0) > 0.1 || r.volatility_20d.unwrap_or(0.0) > 30.0);
    }

    #[test]
    fn returns_unknown_for_insufficient_data() {
        let prices: Vec<f64> = (0..10).map(|i| 10.0 + i as f64 * 0.1).collect();
        let r = RegimeDetector::detect(&kline_series(&prices));
        assert_eq!(r.regime, MarketRegime::Unknown);
        assert_eq!(r.samples, 10);
    }

    #[test]
    fn consecutive_streak_skips_limit_days() {
        // 涨停日不应计入连涨
        let mut prices = vec![10.0; 5];
        prices.push(10.95); // 9.5% 涨停
        prices.push(11.5);
        prices.push(12.0);
        let r = RegimeDetector::detect(&kline_series(&prices));
        // 只有 8 个数据点,样本不够
        assert_eq!(r.regime, MarketRegime::Unknown);
    }

    #[test]
    fn regime_label_and_prompt_bias_present() {
        for r in
            [MarketRegime::Bull, MarketRegime::Bear, MarketRegime::Sideways, MarketRegime::Volatile]
        {
            assert!(!r.label().is_empty());
            assert!(!r.prompt_bias().is_empty());
            assert!(!r.emoji().is_empty());
        }
    }

    #[test]
    fn volatility_calculation() {
        let prices: Vec<f64> = (0..30).map(|i| 10.0 + (i as f64 * 0.1).sin()).collect();
        let vol = RegimeDetector::volatility(&prices, 20);
        assert!(vol.is_some());
        assert!(vol.unwrap() > 0.0);
    }

    /// 口径统一的**量化影响**必须可复算：新值 = 旧总体方差值 × √(n/(n−1))。
    /// 有人日后说「σ 没变」时，这条测试会替他纠正。
    #[test]
    fn volatility_now_uses_sample_variance_and_the_shift_is_exactly_sqrt_n_over_n_minus_1() {
        let prices: Vec<f64> = (0..30).map(|i| 10.0 + (i as f64 * 0.37).sin()).collect();
        let n = 20usize;
        let slice = &prices[prices.len() - n - 1..];
        let rets: Vec<f64> = slice.windows(2).map(|w| (w[1] - w[0]) / w[0]).collect();
        let m = rets.iter().sum::<f64>() / rets.len() as f64;
        let nn = rets.len() as f64;
        let pop = (rets.iter().map(|r| (r - m).powi(2)).sum::<f64>() / nn).sqrt();
        let want = pop * (nn / (nn - 1.0)).sqrt() * (252_f64).sqrt() * 100.0;
        let got = RegimeDetector::volatility(&prices, n).expect("应可计算");
        assert!(
            (got - want).abs() < 1e-8,
            "σ 口径与「总体 × √(n/(n−1))」不符：got {got}, want {want}"
        );
    }

    /// 含非正价格 ⇒ 收益率无定义，返回 None（旧实现会算出 inf/NaN 继续参与打分）。
    #[test]
    fn volatility_rejects_non_positive_prices() {
        let mut prices: Vec<f64> = (0..30).map(|_| 10.0).collect();
        prices[15] = 0.0;
        assert!(RegimeDetector::volatility(&prices, 20).is_none());
    }
}
