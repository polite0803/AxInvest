//! 风险模型与估值指标 — 独立纯函数，可注册为工作流 Tool handler。
//!
//! 包含：最大回撤、夏普比率、VaR、PE 分位数、PEG、凯利公式、风险平价。

use serde::Serialize;

// P3-C8: 年化因子改用 harness 统一常量（A 股 244 天），消除 252/244 混用。
// 保留 `ANNUALIZATION_FACTOR_DAILY` 名称作为对外 API 稳定性兼容（portfolio_monitor 等下游引用），
// 但语义已从"美股 252"切换为"A 股 244"。
pub use axagent_harness::indicators::A_SHARE_TRADING_DAYS_PER_YEAR as ANNUALIZATION_FACTOR_DAILY;
/// 凯利公式默认重仓阈值
pub const KELLY_HEAVY_THRESHOLD: f64 = 0.25;
/// 凯利公式默认中仓阈值
pub const KELLY_MEDIUM_THRESHOLD: f64 = 0.1;

// ── 最大回撤 ──

/// 峰值到谷底的最大回撤比例 (0.0~1.0)。
///
/// 本模块内最大回撤的唯一核心实现；`portfolio_monitor::compute_max_drawdown_pct`
/// 复用本函数（结果 ×100 得到百分比）。
pub(crate) fn peak_trough_drawdown(prices: &[f64]) -> f64 {
    if prices.is_empty() || prices.iter().all(|&p| p <= 0.0) {
        return 0.0;
    }
    let mut peak = prices.iter().find(|&&p| p > 0.0).copied().unwrap_or(0.0);
    let mut max_dd = 0.0;
    for &p in prices.iter() {
        if p > peak {
            peak = p;
        }
        if peak > 0.0 {
            let dd = (peak - p) / peak;
            if dd > max_dd {
                max_dd = dd;
            }
        }
    }
    max_dd
}

/// 计算峰值到谷底的最大回撤比例 (0.0~1.0)，复用 `peak_trough_drawdown`。
pub fn max_drawdown(prices: &[f64]) -> f64 {
    peak_trough_drawdown(prices)
}

// ── 夏普比率 ──

/// 计算夏普比率：(mean_return - risk_free) / stddev_return。
/// 使用 `ANNUALIZATION_FACTOR_DAILY` 作为默认年化因子。
pub fn sharpe_ratio(returns: &[f64], risk_free: f64) -> SharpeResult {
    sharpe_ratio_with_annualization(returns, risk_free, ANNUALIZATION_FACTOR_DAILY)
}

/// 夏普比率核心计算：返回 (sharpe, annualized, mean_return, stddev)。
///
/// P3-C8: 委托 `axagent_harness::indicators::sharpe_components` 统一实现，
/// 消除本 crate 与 astock-data/tools/quant 的算法分叉（252/244、n/n-1）。
/// `portfolio_monitor::compute_sharpe` 复用本函数避免重复实现。
///
/// 保留四舍五入到 3-4 位小数的历史行为，确保下游序列化输出稳定。
pub(crate) fn sharpe_components(
    returns: &[f64],
    risk_free: f64,
    annualization_factor: f64,
) -> (f64, f64, f64, f64) {
    let c =
        axagent_harness::indicators::sharpe_components(returns, risk_free, annualization_factor);
    (
        (c.sharpe * 1000.0).round() / 1000.0,
        (c.annualized * 1000.0).round() / 1000.0,
        (c.mean_return * 10000.0).round() / 100.0,
        (c.stddev * 10000.0).round() / 100.0,
    )
}

/// 带自定义年化因子的夏普比率。
/// `annualization_factor` 为年化时的周期数（A 股日频=244，周频=52，月频=12）。
pub fn sharpe_ratio_with_annualization(
    returns: &[f64],
    risk_free: f64,
    annualization_factor: f64,
) -> SharpeResult {
    let (sharpe, annualized, mean_return, stddev) =
        sharpe_components(returns, risk_free, annualization_factor);
    SharpeResult { sharpe, annualized, mean_return, stddev }
}

#[derive(Debug, Clone, Serialize)]
pub struct SharpeResult {
    pub sharpe: f64,
    pub annualized: f64,
    pub mean_return: f64,
    pub stddev: f64,
}

// ── VaR (Value at Risk) ──

/// 历史模拟法 VaR：将收益率排序后取第 (1-confidence) 分位数。
/// 返回正数表示损失的百分比。
///
/// 口径（2026-09-21 定案）：`idx = floor((1-confidence) * n)` 作 0-based 下标**直接**
/// 取顺序统计量 —— 数学上恰有 `idx` 个样本小于它 ⇒ `idx / n ≈ (1-confidence)`，
/// 命中的正是该置信水平对应的历史分位点。
///
/// ⚠ 本函数此前的算式是 `floor((1-confidence) * (n+1))` 再 `-1`（净效果取**前一位**，
/// 即多含一个尾部样本）—— 那是 `+1` 与 `-1` 两个偏移叠加后的混合口径，
/// **不对应任何标准分位定义**。
///
/// ⚠⚠ **浮点 floor 边界**（实测踩过，写测试时最容易被它骗）：`1.0 - c` 的浮点值可能
/// **略小于**数学值（`1.0-0.9 = 0.09999999999999998`、`1.0-0.8 = 0.19999999999999996`），
/// 于是当 `(1-c) * n` 数学上恰为整数时 `floor` 会**掉到下一档** —— `c=0.9, n=10`：
/// 数学 `1.0`，浮点 `0.9999999999999998`，得 `idx = 0`（取了最小值）。
/// 这是**既存行为**（生产实现亦如此，非本次引入）。危险之处在它同时影响**测试**：
/// **「整十置信度 × n=10」组合下新旧两口径输出恰好相同**（实测 c=0.9 时新旧均为
/// `var_pct = 0.05`）⇒ 拿这种参数当测试点，得到的是「区分力为 0 的假绿」。
/// 故 `test_var` 改用 `n=10 + c=0.875 / 0.75` —— `1-c` 分别为 0.125 / 0.25，
/// **二进制精确**、无边界歧义，且两口径输出差 0.05 / 0.10，可清晰区分。
///
/// 为什么改这里而不是改另一份：本函数**全仓零调用**，改它零行为风险；而另一份
/// （`tools/finance.rs` 的 `value_at_risk`，`calc_var` 工具在消费）是生产在跑的口径，
/// 改它会让所有历史产物不可复现，且没有任何正确性依据支持那么做。
/// 两份的实测量级差异：1980 组参数扫描下索引不同 **45.5%**、最终 2 位小数输出不同
/// **9.44%**、最大 Δ **0.032**（≈6.5 倍舍入半格）、样例甚至**符号翻转**
/// （证据：`output/tmp/verify-var-divergence.mjs`）。
pub fn value_at_risk(returns: &[f64], confidence: f64) -> VarResult {
    let n = returns.len();
    if n < 5 {
        return VarResult { var_pct: 0.0, confidence, cvar_pct: 0.0 };
    }
    let mut sorted = returns.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let idx = ((1.0 - confidence) * n as f64).floor() as usize;
    let var_val = if idx < n { -sorted[idx] } else { 0.0 };
    let tail: f64 = sorted[..=idx.min(n - 1)].iter().map(|r| -r).sum::<f64>();
    let cvar = tail / (idx + 1) as f64;
    VarResult {
        var_pct: (var_val * 100.0).round() / 100.0,
        confidence,
        cvar_pct: (cvar * 100.0).round() / 100.0,
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct VarResult {
    pub var_pct: f64,
    pub confidence: f64,
    pub cvar_pct: f64,
}

// ── PE 分位数 ──

/// 计算当前 PE 在历史 PE 序列中的分位数 (0-100)。
pub fn pe_percentile(current_pe: f64, historical_pes: &[f64]) -> PEPercentileResult {
    let mut sorted = historical_pes.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    // 2026-09-21 修复：亏损企业（PE < 0）的 PE 没有分位含义 —— 负 cur 在历史正 PE
    // 序列里命中 0 条 ⇒ percentile = 0 ⇒ level = "极低"，即「亏损」被读成「历史估值
    // 极低分位」。上游 vendor 已放开负 PE（负值＝亏损）⇒ t-risk 的 peTTM 可能为负。
    // ⚠ 本函数当前**无生产调用**（仅本文件单测），活路径是
    //   `crates/tools/src/tools/finance.rs` 的同名私有实现 —— 两处守卫必须保持等价，
    //   否则将来接线到本函数时缺陷复现（两份副本的分叉已由 2026-09-21 一并修齐）。
    if current_pe <= 0.0 {
        let median = if !sorted.is_empty() {
            sorted[sorted.len() / 2]
        } else {
            current_pe
        };
        return PEPercentileResult { percentile: 0.0, level: "无意义".into(), median };
    }
    let below = sorted.iter().filter(|&&pe| pe <= current_pe).count();
    let pct = if sorted.is_empty() {
        50.0
    } else {
        below as f64 / sorted.len() as f64 * 100.0
    };
    let level = if pct < 20.0 {
        "极低"
    } else if pct < 40.0 {
        "偏低"
    } else if pct < 60.0 {
        "合理"
    } else if pct < 80.0 {
        "偏高"
    } else {
        "极高"
    };
    PEPercentileResult {
        percentile: (pct * 10.0).round() / 10.0,
        level: level.into(),
        median: if !sorted.is_empty() {
            sorted[sorted.len() / 2]
        } else {
            current_pe
        },
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct PEPercentileResult {
    pub percentile: f64,
    pub level: String,
    pub median: f64,
}

// ── PEG ──

/// PEG = PE / 增长率。增长率以 % 表示（如 25 表示 25%）。
pub fn peg_ratio(pe: f64, growth_rate: f64) -> PEGResult {
    if growth_rate <= 0.0 {
        return PEGResult { peg: f64::INFINITY, level: "无意义".into(), pe, growth_rate };
    }
    // 2026-09-21 修复：亏损企业（PE < 0）的 PEG 无含义 —— 原实现只守 growth_rate，
    // 负 pe 会算出负 peg 落进 `peg < 0.5` ⇒ "严重低估"（亏损被读成严重低估）。
    // ⚠ 同 `pe_percentile`：本函数当前无生产调用，活路径在
    //   `crates/tools/src/tools/finance.rs`，两处守卫须保持等价。
    if pe <= 0.0 {
        return PEGResult { peg: f64::INFINITY, level: "无意义".into(), pe, growth_rate };
    }
    let peg = pe / growth_rate;
    let level = if peg < 0.5 {
        "严重低估"
    } else if peg < 1.0 {
        "低估"
    } else if peg < 2.0 {
        "合理"
    } else {
        "高估"
    };
    PEGResult { peg: (peg * 100.0).round() / 100.0, level: level.into(), pe, growth_rate }
}

#[derive(Debug, Clone, Serialize)]
pub struct PEGResult {
    pub peg: f64,
    pub level: String,
    pub pe: f64,
    pub growth_rate: f64,
}

// ── 凯利公式 ──

/// Kelly Criterion: f* = p - q / (W/L) = p - (1-p) / (avg_win / avg_loss)
/// 返回建议仓位比例。使用 `KELLY_HEAVY_THRESHOLD` / `KELLY_MEDIUM_THRESHOLD` 作为默认阈值。
pub fn kelly_criterion(win_rate: f64, avg_win: f64, avg_loss: f64) -> KellyResult {
    kelly_criterion_with_thresholds(
        win_rate,
        avg_win,
        avg_loss,
        KELLY_HEAVY_THRESHOLD,
        KELLY_MEDIUM_THRESHOLD,
    )
}

/// 带自定义仓位信号阈值的凯利公式。
/// - `heavy_threshold`: 超过此值视为"重仓"（默认 0.25）
/// - `medium_threshold`: 超过此值视为"中等"（默认 0.1），低于此值且 >0 为"轻仓"
pub fn kelly_criterion_with_thresholds(
    win_rate: f64,
    avg_win: f64,
    avg_loss: f64,
    heavy_threshold: f64,
    medium_threshold: f64,
) -> KellyResult {
    if avg_loss <= 0.0 || avg_win <= 0.0 || win_rate <= 0.0 {
        return KellyResult {
            kelly_fraction: 0.0,
            half_kelly: 0.0,
            position_pct: 0.0,
            signal: "不适用".into(),
        };
    }
    let odds = avg_win / avg_loss;
    let kelly = ((win_rate * (odds + 1.0) - 1.0) / odds).max(0.0);
    let half = kelly / 2.0;
    let signal = if kelly > heavy_threshold {
        "重仓"
    } else if kelly > medium_threshold {
        "中等"
    } else if kelly > 0.0 {
        "轻仓"
    } else {
        "不建议"
    };
    KellyResult {
        kelly_fraction: (kelly * 1000.0).round() / 1000.0,
        half_kelly: (half * 1000.0).round() / 1000.0,
        position_pct: (half * 10000.0).round() / 100.0,
        signal: signal.into(),
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct KellyResult {
    pub kelly_fraction: f64,
    pub half_kelly: f64,
    pub position_pct: f64,
    pub signal: String,
}

// ── 风险平价 ──

/// 风险平价权重：每项资产权重 ∝ 1/volatility，归一化到总和=1。
pub fn risk_parity_weights(volatilities: &[f64], correlations_json: &str) -> RiskParityResult {
    let n = volatilities.len();
    if n == 0 {
        return RiskParityResult { weights: vec![], divers_ratio: 0.0 };
    }
    let corr_matrix: Option<Vec<Vec<f64>>> = serde_json::from_str(correlations_json)
        .ok()
        .filter(|m: &Vec<Vec<f64>>| m.len() == n && m.iter().all(|r| r.len() == n));
    let inv_vols: Vec<f64> =
        volatilities.iter().map(|&v| if v > 0.0 { 1.0 / v } else { 0.0 }).collect();
    let total: f64 = inv_vols.iter().sum();
    let weights = if let Some(corr) = corr_matrix {
        let mut w: Vec<f64> = inv_vols.clone();
        let w_sum: f64 = w.iter().sum();
        if w_sum > 0.0 {
            for wi in w.iter_mut() {
                *wi /= w_sum;
            }
        }
        for _ in 0..20 {
            let mut risk_contrib = vec![0.0; n];
            for i in 0..n {
                for j in 0..n {
                    risk_contrib[i] += w[i] * w[j] * volatilities[i] * volatilities[j] * corr[i][j];
                }
            }
            let total_risk: f64 = risk_contrib.iter().sum();
            if total_risk <= 0.0 {
                break;
            }
            let target = total_risk / n as f64;
            for i in 0..n {
                if risk_contrib[i] > 0.0 {
                    w[i] *= (target / risk_contrib[i]).sqrt().clamp(0.5, 2.0);
                }
            }
            let ws: f64 = w.iter().sum();
            if ws > 0.0 {
                for wi in w.iter_mut() {
                    *wi /= ws;
                }
            }
        }
        w.iter().map(|&x| (x * 10000.0).round() / 10000.0).collect()
    } else if total > 0.0 {
        inv_vols.iter().map(|&w| (w / total * 10000.0).round() / 10000.0).collect()
    } else {
        vec![1.0 / n as f64; n]
    };
    let hhi: f64 = weights.iter().map(|w| w * w).sum();
    let divers_ratio = if hhi > 0.0 {
        (1.0 / (hhi * n as f64)).min(1.0)
    } else {
        1.0
    };
    RiskParityResult { weights, divers_ratio: (divers_ratio * 100.0).round() / 100.0 }
}

#[derive(Debug, Clone, Serialize)]
pub struct RiskParityResult {
    pub weights: Vec<f64>,
    pub divers_ratio: f64,
}

// ── 测试 ──

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_max_drawdown_normal() {
        let prices = vec![100.0, 110.0, 90.0, 95.0, 105.0];
        let dd = max_drawdown(&prices);
        assert!((dd - 0.1818).abs() < 0.01, "expected ~0.1818, got {dd}"); // (110-90)/110
    }

    #[test]
    fn test_max_drawdown_empty() {
        assert_eq!(max_drawdown(&[]), 0.0);
    }

    #[test]
    fn test_sharpe_ratio() {
        let returns = vec![0.01, 0.02, -0.01, 0.005, 0.015];
        // P3-C8: 年化因子切换为 A 股 244 天
        let r = sharpe_ratio(&returns, 0.02 / 244.0);
        assert!(r.sharpe > 0.0, "positive mean return should give positive sharpe");
    }

    #[test]
    fn test_var() {
        // 10 个样本、步长 0.10、已升序（idx 的语义——取第几个顺序统计量——肉眼可核）。
        //
        // ⚠ 为什么**不**用「n=10 × 整十置信度（0.95/0.9/0.8）」：见函数 doc 的
        //   「浮点 floor 边界」——那类组合下 `1.0-c` 的浮点值略小于数学值 ⇒ `floor` 掉一档，
        //   而旧口径的 `+1` 恰好把它补回来 ⇒ **两口径输出相同**，测试丧失区分力
        //   （实测 c=0.9 时新旧均得 `var_pct = 0.05`）。
        //   改用 `c = 0.875 / 0.75`：`1-c` = 0.125 / 0.25 为**二进制精确值**，
        //   无边界歧义，且两口径输出分别差 0.05 / 0.10，可清晰区分。
        let returns = [-0.50, -0.40, -0.30, -0.20, -0.10, 0.00, 0.10, 0.20, 0.30, 0.40];

        // ── 口径锁：idx = floor((1-c) * n) 直接作 0-based 下标 ──
        // c=0.875 ⇒ idx = floor(0.125*10) = 1 ⇒ 第 2 小值 0.40；尾均值 (0.50+0.40)/2 = 0.45。
        let r875 = value_at_risk(&returns, 0.875);
        assert!((r875.var_pct - 0.40).abs() < 1e-9, "c=0.875 ⇒ idx=1 ⇒ 0.40");
        assert!((r875.cvar_pct - 0.45).abs() < 1e-9, "尾均值取 s[..=1] ⇒ 0.45");
        // 反面断言（锚定被测对象自身形态）：旧口径在此得 idx = floor(1.375) - 1 = 0
        // ⇒ 0.50 / 0.50。谁把实现改回旧口径，这条立刻红。
        assert!((r875.var_pct - 0.50).abs() > 1e-9, "不得退化为旧口径（取最小值 0.50）");

        // c=0.75 ⇒ idx = floor(2.5) = 2 ⇒ 0.30；尾均值 (0.50+0.40+0.30)/3 = 0.40。
        let r75 = value_at_risk(&returns, 0.75);
        assert!((r75.var_pct - 0.30).abs() < 1e-9, "c=0.75 ⇒ idx=2 ⇒ 0.30");
        assert!((r75.cvar_pct - 0.40).abs() < 1e-9, "尾均值取 s[..=2] ⇒ 0.40");
        assert!(r875.cvar_pct >= r875.var_pct, "CVaR 不应小于 VaR");

        // ── 浮点边界：锁住**既存行为**（非本次引入，但与生产实现一致）──
        // `1.0-0.95 = 0.050000000000000044` → `*10 = 0.5000000000000004` → floor = 0。
        // 不直观，可它是生产在跑的行为；谁想「修正」它，得先想清历史产物怎么办。
        let r95 = value_at_risk(&returns, 0.95);
        assert!(
            (r95.var_pct - 0.50).abs() < 1e-9,
            "c=0.95, n=10 落在浮点 floor 边界下方 ⇒ idx=0（取最小值）"
        );

        // `n < 5` 的早退分支：三个字段都要有确定值，不得留 NaN。
        let short = value_at_risk(&[0.01, -0.02, 0.03], 0.95);
        assert_eq!(short.var_pct, 0.0);
        assert_eq!(short.cvar_pct, 0.0);
        assert_eq!(short.confidence, 0.95);
    }

    #[test]
    fn test_pe_percentile() {
        let pes = vec![10.0, 12.0, 15.0, 18.0, 20.0, 22.0, 25.0, 30.0];
        let r = pe_percentile(16.0, &pes);
        assert!(r.percentile > 30.0 && r.percentile < 60.0);
    }

    #[test]
    fn test_peg_ratio() {
        let r = peg_ratio(20.0, 25.0);
        assert!((r.peg - 0.8).abs() < 0.01);
        assert_eq!(r.level, "低估");
    }

    /// 2026-09-21 新增（A 修复）：亏损企业（PE < 0）不得被判成「历史估值极低分位」。
    /// 负 cur 在历史正 PE 序列里命中 0 条 ⇒ 旧实现 `percentile = 0` ⇒ `level = "极低"`。
    #[test]
    fn test_pe_percentile_negative_pe_is_meaningless() {
        let pes = vec![10.0, 12.0, 15.0, 18.0, 20.0, 22.0, 25.0, 30.0];
        let r = pe_percentile(-144.08, &pes);
        assert_eq!(r.level, "无意义", "亏损 PE 不得被读成极低分位");
        // 正控：同一序列下正 PE 仍走原路径（守卫不得吃掉正常输入）
        let ok = pe_percentile(16.0, &pes);
        assert!(ok.percentile > 30.0 && ok.percentile < 60.0);
        assert_ne!(ok.level, "无意义");
    }

    /// 2026-09-21 新增（A 修复）：亏损企业（PE < 0）不得被判成「严重低估」。
    /// 旧实现只守 `growth_rate <= 0` ⇒ −144.08/25 = −5.76 落进 `peg < 0.5`。
    #[test]
    fn test_peg_ratio_negative_pe_is_meaningless() {
        let r = peg_ratio(-144.08, 25.0);
        assert_eq!(r.level, "无意义", "亏损 PE 不得被读成严重低估");
        assert!(r.peg.is_infinite());
        // 正控
        let ok = peg_ratio(20.0, 25.0);
        assert_eq!(ok.level, "低估");
    }

    #[test]
    fn test_kelly() {
        let r = kelly_criterion(0.55, 0.08, 0.05);
        assert!(r.kelly_fraction > 0.0);
        assert!(r.half_kelly > 0.0);
    }

    #[test]
    fn test_risk_parity() {
        let vols = vec![0.2, 0.3, 0.4];
        let r = risk_parity_weights(&vols, "[]");
        assert_eq!(r.weights.len(), 3);
        assert!((r.weights.iter().sum::<f64>() - 1.0).abs() < 0.001);
        // 高波动资产权重应更低
        assert!(r.weights[2] < r.weights[0]);
    }
}
