//! 技术指标纯函数模块（SMA / EMA / RSI / stddev）
//!
//! P2-C7: 将原本散落在 `astock-data`、`quant`、`market-sim`、`stock-analysis`
//! 的重复实现统一收口到 harness foundation 层。所有共享数据模型的 crate
//! （implementor / consumer / hybrid / wiring）均可通过 `pub use` 引用，
//! 消除 DRY 违规，确保算法一致性。
//!
//! ## 算法约定
//!
//! - **SMA**: 取最近 `period` 个数据的算术平均；数据不足返回 `None`
//! - **EMA 序列**: 首值用前 `period` 个数据的 SMA 初始化（标准 EMA 初始化），
//!   返回与输入等长的序列
//! - **RSI (Wilder 平滑)**: 首轮简单平均，后续用 `(n-1)/n` 指数平滑；
//!   数据不足（`len < period + 1` 或 `period == 0`）返回 `None`
//! - **样本标准差**: n-1 分母（Bessel 校正），用于布林带等统计场景
//!
//! ## 返回值语义
//!
//! - `Option<f64>` 版本：数据不足返回 `None`，调用方自行决定中性默认值
//! - 序列版本：输入为空或 `period == 0` 返回 `vec![0.0]`（保持与历史调用方兼容）
//!
//! ## 不变量
//!
//! - 所有函数对 `period == 0` 做防御性处理，不会 panic
//! - 输入为空切片时不会 panic
//! - 不依赖任何外部 crate，仅用 std（符合 foundation 层零 axagent-* 依赖约束）

// ===================== SMA =====================

/// 简单移动平均（取最后 `period` 个数据点的算术平均）
///
/// - 数据不足（`data.len() < period`）或 `period == 0` 时返回 `None`
/// - 调用方需自行决定回退值（如用最新收盘价或 50.0 中性值）
///
/// # 示例
///
/// ```
/// use axagent_harness::indicators::sma;
/// assert_eq!(sma(&[1.0, 2.0, 3.0, 4.0], 2), Some(3.5));
/// assert_eq!(sma(&[1.0, 2.0], 5), None);
/// assert_eq!(sma(&[1.0, 2.0, 3.0], 0), None);
/// ```
pub fn sma(data: &[f64], period: usize) -> Option<f64> {
    if data.len() < period || period == 0 {
        return None;
    }
    let start = data.len() - period;
    Some(data[start..].iter().sum::<f64>() / period as f64)
}

// ===================== EMA =====================

/// 构建完整 EMA 序列（与输入等长）
///
/// 首值用前 `period` 个数据的 SMA 初始化（标准 EMA 初始化），
/// 后续按 `multiplier = 2 / (period + 1)` 递推。
///
/// - 输入为空或 `period == 0` 时返回 `vec![0.0]`（保持与历史调用方兼容）
/// - 返回序列长度等于输入长度
///
/// # 示例
///
/// ```
/// use axagent_harness::indicators::build_ema_series;
/// let series = build_ema_series(&[1.0, 2.0, 3.0, 4.0], 2);
/// assert_eq!(series.len(), 4);
/// // 首值 = SMA(1, 2) = 1.5
/// assert!((series[0] - 1.5).abs() < 1e-10);
/// ```
pub fn build_ema_series(data: &[f64], period: usize) -> Vec<f64> {
    if data.is_empty() || period == 0 {
        return vec![0.0];
    }
    let multiplier = 2.0 / (period as f64 + 1.0);
    let mut result = Vec::with_capacity(data.len());
    let init_n = period.min(data.len());
    let init_sma: f64 = data[..init_n].iter().sum::<f64>() / init_n as f64;
    let mut ema_val = init_sma;
    result.push(ema_val);
    for &val in &data[1..] {
        ema_val = (val - ema_val) * multiplier + ema_val;
        result.push(ema_val);
    }
    result
}

/// 仅取 EMA 序列的末值（便捷函数）
///
/// 等价于 `build_ema_series(data, period).last().copied().unwrap_or(0.0)`，
/// 但避免分配整个 Vec。数据为空时返回 `0.0`。
#[inline]
pub fn ema_last(data: &[f64], period: usize) -> f64 {
    if data.is_empty() || period == 0 {
        return 0.0;
    }
    let multiplier = 2.0 / (period as f64 + 1.0);
    let init_n = period.min(data.len());
    let init_sma: f64 = data[..init_n].iter().sum::<f64>() / init_n as f64;
    let mut ema_val = init_sma;
    for &val in &data[1..] {
        ema_val = (val - ema_val) * multiplier + ema_val;
    }
    ema_val
}

// ===================== RSI (Wilder 平滑) =====================

/// RSI 指标（Wilder 平滑法）
///
/// 首轮对前 `period` 个涨跌幅做简单平均，后续用 `(n-1)/n` 指数平滑。
/// 数据不足（`len < period + 1` 或 `period == 0`）返回 `None`，
/// 调用方自行决定中性默认值（如 50.0）。
///
/// # 边界情况
///
/// - `avg_loss < 1e-10`（持续上涨无回调）返回 `Some(100.0)`
/// - 数据不足返回 `None`
///
/// # 示例
///
/// ```
/// use axagent_harness::indicators::rsi_wilder;
/// // 持续上涨 → RSI = 100
/// let closes = vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0];
/// assert_eq!(rsi_wilder(&closes, 5), Some(100.0));
/// // 数据不足
/// assert_eq!(rsi_wilder(&[1.0, 2.0], 5), None);
/// ```
pub fn rsi_wilder(closes: &[f64], period: usize) -> Option<f64> {
    if closes.len() < period + 1 || period == 0 {
        return None;
    }
    let mut avg_gain = 0.0;
    let mut avg_loss = 0.0;
    for i in 1..=period {
        let diff = closes[i] - closes[i - 1];
        if diff > 0.0 {
            avg_gain += diff;
        } else {
            avg_loss += -diff;
        }
    }
    avg_gain /= period as f64;
    avg_loss /= period as f64;
    for i in (period + 1)..closes.len() {
        let diff = closes[i] - closes[i - 1];
        let gain = if diff > 0.0 { diff } else { 0.0 };
        let loss = if diff < 0.0 { -diff } else { 0.0 };
        avg_gain = (avg_gain * (period - 1) as f64 + gain) / period as f64;
        avg_loss = (avg_loss * (period - 1) as f64 + loss) / period as f64;
    }
    // 修复 L5: 区分「持续上涨」与「完全平盘」
    //   - 持续上涨（avg_gain>0, avg_loss≈0）→ RSI=100（超买）
    //   - 完全平盘（avg_gain≈0 且 avg_loss≈0）→ RSI=50（中性），避免平盘误报极端超买
    if avg_loss < 1e-10 {
        return if avg_gain < 1e-10 {
            Some(50.0)
        } else {
            Some(100.0)
        };
    }
    let rs = avg_gain / avg_loss;
    Some(100.0 - (100.0 / (1.0 + rs)))
}

// ===================== 样本标准差 =====================

/// 样本标准差（n-1 分母，Bessel 校正）
///
/// 用于布林带等统计场景。数据少于 2 个返回 `0.0`。
///
/// # 示例
///
/// ```
/// use axagent_harness::indicators::stddev_sample;
/// let data = vec![2.0, 4.0, 4.0, 4.0, 5.0, 5.0, 7.0, 9.0];
/// let mean = data.iter().sum::<f64>() / data.len() as f64;
/// let sd = stddev_sample(&data, mean);
/// assert!((sd - 2.138).abs() < 0.01);
/// ```
pub fn stddev_sample(data: &[f64], mean: f64) -> f64 {
    let n = data.len() as f64;
    if n < 2.0 {
        return 0.0;
    }
    let variance = data.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / (n - 1.0);
    variance.sqrt()
}

// ===================== Sharpe Ratio（P3-C8 统一实现）=====================

/// A 股每年实际交易日数（约 244 天，而非美股的 252 天）。
///
/// P3-C8: 将原本散落在 `stock-analysis/risk.rs` (252)、`astock-data/mcp_tools.rs` (252)、
/// `tools/finance.rs` (252)、`quant/metrics.rs` (244) 的年化因子统一收口。
/// 所有 A 股相关计算应使用本常量，避免 252/244 混用导致的 Sharpe / 波动率偏差。
pub const A_SHARE_TRADING_DAYS_PER_YEAR: f64 = 244.0;

/// 默认年无风险利率（2.5%，参考 10 年期国债收益率中枢）。
///
/// 各调用方可根据自身语义覆盖（如 `astock-data/mcp_tools.rs` 历史使用 3.0%）。
pub const RISK_FREE_ANNUAL_DEFAULT: f64 = 0.025;

/// Sharpe 计算的完整结果（与历史 `SharpeResult` / `SharpeR` 字段对齐）。
///
/// P3-C8: 统一 DTO，消除 stock-analysis/risk.rs `SharpeResult` 与 tools/finance.rs `SharpeR`
/// 两套同义结构体的 DRY 违规。下游 crate 通过 `pub use axagent_harness::indicators::SharpeComponents`
/// 复用，避免重复定义。
#[derive(Debug, Clone, serde::Serialize)]
pub struct SharpeComponents {
    /// 日频 Sharpe：(mean - risk_free_daily) / stddev
    pub sharpe: f64,
    /// 年化 Sharpe：`sharpe * sqrt(annualization)`
    pub annualized: f64,
    /// 日均收益率（原始值，未缩放）
    pub mean_return: f64,
    /// 日收益率样本标准差（n-1 分母）
    pub stddev: f64,
}

/// 夏普比率核心计算 —— 接受 **日频** 无风险利率。
///
/// 统一算法约定：
/// - 样本方差（n-1 分母，Bessel 校正）
/// - 数据 < 2 个返回全零 `SharpeComponents`
/// - `stddev == 0`（常数序列）返回全零，避免除零
/// - 不做四舍五入，由调用方按需 round（保留精度供下游复用）
///
/// # 参数
///
/// - `returns`: 日收益率切片（如 0.01 表示 +1%）
/// - `risk_free_daily`: **日频** 无风险利率（如 0.03/244 ≈ 0.000123）
/// - `annualization`: 年化因子（A 股 = 244，美股 = 252，周频 = 52，月频 = 12）
///
/// # 示例
///
/// ```
/// use axagent_harness::indicators::{sharpe_components, A_SHARE_TRADING_DAYS_PER_YEAR};
/// let returns = vec![0.01, 0.02, -0.01, 0.005, 0.015];
/// let r = sharpe_components(&returns, 0.03 / A_SHARE_TRADING_DAYS_PER_YEAR, A_SHARE_TRADING_DAYS_PER_YEAR);
/// assert!(r.sharpe > 0.0, "正均值应有正 sharpe");
/// assert!(r.annualized > r.sharpe, "年化值应放大");
/// ```
pub fn sharpe_components(
    returns: &[f64],
    risk_free_daily: f64,
    annualization: f64,
) -> SharpeComponents {
    let n = returns.len();
    if n < 2 {
        return SharpeComponents { sharpe: 0.0, annualized: 0.0, mean_return: 0.0, stddev: 0.0 };
    }
    let mean: f64 = returns.iter().sum::<f64>() / n as f64;
    let variance: f64 = returns.iter().map(|r| (r - mean).powi(2)).sum::<f64>() / (n - 1) as f64;
    let stddev = variance.sqrt();
    if stddev == 0.0 {
        return SharpeComponents { sharpe: 0.0, annualized: 0.0, mean_return: mean, stddev: 0.0 };
    }
    let excess = mean - risk_free_daily;
    let sharpe = excess / stddev;
    let annualized = sharpe * annualization.sqrt();
    SharpeComponents { sharpe, annualized, mean_return: mean, stddev }
}

/// 便捷函数：A 股日频夏普比率（年化），使用默认 244 天年化。
///
/// 等价于 `sharpe_components(returns, risk_free_daily, A_SHARE_TRADING_DAYS_PER_YEAR).annualized`。
/// 数据不足或常数序列返回 `0.0`。
#[inline]
pub fn sharpe_ratio(returns: &[f64], risk_free_daily: f64) -> f64 {
    sharpe_components(returns, risk_free_daily, A_SHARE_TRADING_DAYS_PER_YEAR).annualized
}

/// 便捷函数：带自定义年化因子的夏普比率（年化）。
///
/// 适用于周频（52）、月频（12）或美股日频（252）等非 A 股场景。
#[inline]
pub fn sharpe_ratio_with_annualization(
    returns: &[f64],
    risk_free_daily: f64,
    annualization: f64,
) -> f64 {
    sharpe_components(returns, risk_free_daily, annualization).annualized
}

/// 便捷函数：接受 **年频** 无风险利率的夏普比率（年化）。
///
/// 内部将年利率转换为日利率（`risk_free_annual / days_per_year`）后调用核心函数。
/// 适用于 `quant::metrics::sharpe_ratio(curve, risk_free_annual, days_per_year)` 这类
/// 以年利率为输入的回测场景。
#[inline]
pub fn sharpe_ratio_annual(returns: &[f64], risk_free_annual: f64, days_per_year: f64) -> f64 {
    if days_per_year <= 0.0 {
        return 0.0;
    }
    let daily_rf = risk_free_annual / days_per_year;
    sharpe_components(returns, daily_rf, days_per_year).annualized
}

// ===================== 单元测试 =====================

/// 已实现波动率（**日**，单位 %）：simple return 序列的样本标准差 ×100。
///
/// 用途（四周期科学化 Phase D）：止损/止盈不再用「按日线经验拍的固定百分比」，
/// 而按 `k · σ_daily · √持有天数` 推导 —— 位移的标准差随时间按 √ 增长，
/// 这是波动率目标法（volatility targeting）与三阶障碍法上下轨设定的共同基础。
///
/// 口径：**样本标准差（÷(n−1)）**，与本文件 `sharpe_components` 一致
/// （该处曾修过 astock-data 的「总体方差 bug」，见 `sharpe_matches_astock_data_legacy_formula_after_fix`）。
/// 2026-09-29 起 `astock-data/src/regime.rs::volatility` 也委托到本函数 ⇒ 全仓只剩这一份 σ
/// （切换使 20 日年化波动率整体 ×√(n/(n−1)) ≈ +2.6%，regime 的阈值是**序数口径**，档位不漂移）。
///
/// 返回 `None` 的情形（调用方必须显式降级，**不得当成 0 波动**）：
/// 样本不足 `lookback + 1`、序列含非正价格（收益率无定义）、结果非有限。
pub fn realized_vol_pct(closes: &[f64], lookback: usize) -> Option<f64> {
    if lookback < 2 || closes.len() < lookback + 1 {
        return None;
    }
    let slice = &closes[closes.len() - (lookback + 1)..];
    if slice.iter().any(|c| !c.is_finite() || *c <= 0.0) {
        return None;
    }
    let returns: Vec<f64> = slice.windows(2).map(|w| (w[1] - w[0]) / w[0]).collect();
    let mean = returns.iter().sum::<f64>() / returns.len() as f64;
    let sd = stddev_sample(&returns, mean);
    if !sd.is_finite() {
        return None;
    }
    Some(sd * 100.0)
}

/// 该持有期的**典型位移幅度**（%）= `σ_daily × √持有天数`。
///
/// 为什么在 Rust 侧算：本仓共享 Rhai 引擎未注册 `sqrt`/`Math`（脚本内 0 处使用），
/// 让脚本自己开方要么新增注册面、要么写成幂运算 —— 都不如把这一行放进已带单测的口径函数里。
/// `σ` 不可得 ⇒ `None`（调用方必须显式降级，见 [`realized_vol_pct`] 的口径说明）。
pub fn vol_move_pct(closes: &[f64], lookback: usize, holding_days: usize) -> Option<f64> {
    if holding_days == 0 {
        return None;
    }
    realized_vol_pct(closes, lookback).map(|sigma| sigma * (holding_days as f64).sqrt())
}

/// 把「当前后验胜率」按持有期折算成**该持有期的胜率**（判定侧的时间换空间）。
///
/// 依据：恒定日边缘下，信号均值随 h 线性累积、噪声随 √h 累积 ⇒ 信噪比 ∝ √h。
/// 故以中线为锚做开方折算：`conf = 0.5 + (p − 0.5) × √(h / anchor)`，
/// h = anchor 时不改；h 更短 ⇒ 边缘向 0.5 收缩；h 更长 ⇒ 边缘放大。
/// **对称**：负边缘（看空）同样被时间放大 —— 这是 SNR 的性质，不是对多空的态度。
///
/// 为什么放在判定侧而不是仓位侧：仓位只应承载可推导的量（σ、h、风险预算 R），
/// 「长线更值得」是收益/概率命题，塞进仓位乘数就等于把偏好伪装成计算。
///
/// 退化：`anchor == 0` 或 `h == 0` ⇒ 原样返回 `p`（无从折算，不猜）。
pub fn snr_confidence(p: f64, holding_days: usize, anchor_days: usize) -> f64 {
    if holding_days == 0 || anchor_days == 0 || !p.is_finite() {
        return p;
    }
    let scaled = 0.5 + (p - 0.5) * ((holding_days as f64) / (anchor_days as f64)).sqrt();
    scaled.clamp(0.0, 1.0)
}

/// 平均秩（mid-rank）：并列值取其占据秩区间的均值，1-indexed。
///
/// 并列判据用 `|a−b| < 1e-9` 而非 `==` —— 与本仓既有实现（`hit_rate_backtest::rank_average`）
/// 一致：因子分/置信度是计算得出的浮点，二进制相等但数学相等的值若不算并列，
/// Spearman 的闭式解 `1 − 6Σd²/(n(n²−1))` 就不再成立。
/// 输入含非有限值时按 `partial_cmp` 的 `Equal` 归入同组（调用方应先自行过滤）。
pub fn average_ranks(values: &[f64]) -> Vec<f64> {
    let n = values.len();
    let mut indexed: Vec<(usize, f64)> = values.iter().copied().enumerate().collect();
    indexed.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));

    let mut ranks = vec![0.0; n];
    let mut i = 0;
    while i < n {
        let mut j = i;
        while j < n && (indexed[j].1 - indexed[i].1).abs() < 1e-9 {
            j += 1;
        }
        // i..j 是同一并列组（1-indexed 秩区间为 i+1..=j）
        let avg = ((i + 1) + j) as f64 / 2.0;
        for k in i..j {
            ranks[indexed[k].0] = avg;
        }
        i = j;
    }
    ranks
}

/// Pearson 相关系数。
///
/// 返回 `None`（= 该量在此样本上**无定义**，不得当 0 用）：长度不等 / 不足 2 点 /
/// 含非有限值 / 任一侧方差为 0。
pub fn pearson(xs: &[f64], ys: &[f64]) -> Option<f64> {
    if xs.len() != ys.len() || xs.len() < 2 {
        return None;
    }
    if xs.iter().chain(ys.iter()).any(|v| !v.is_finite()) {
        return None;
    }
    let n = xs.len() as f64;
    let mx = xs.iter().sum::<f64>() / n;
    let my = ys.iter().sum::<f64>() / n;
    let mut cov = 0.0;
    let mut vx = 0.0;
    let mut vy = 0.0;
    for i in 0..xs.len() {
        let dx = xs[i] - mx;
        let dy = ys[i] - my;
        cov += dx * dy;
        vx += dx * dx;
        vy += dy * dy;
    }
    let denom = (vx * vy).sqrt();
    if denom < 1e-9 {
        return None;
    }
    Some((cov / denom).clamp(-1.0, 1.0))
}

/// Spearman 秩相关（rank IC 的标准口径）= `pearson(rank(x), rank(y))`。
///
/// 为什么用秩不用 Pearson 原值：收益分布重尾，个别 −20%/+30% 的样本会把线性相关
/// 整条拖走；IC 关心的是**排序是否正确**（预测强的样本是否真的收益更高），
/// 秩相关对此天然稳健。
///
/// 返回 `None` 的情形与 [`pearson`] 同：秩无定义（样本 <2 / 含非有限 / 一侧无方差）。
/// 「样本够不够多」是**调用方**的门槛（统计显著性取决于用途），不在这里替所有人定。
pub fn spearman_rank_ic(pairs: &[(f64, f64)]) -> Option<f64> {
    if pairs.iter().any(|(x, y)| !x.is_finite() || !y.is_finite()) {
        return None;
    }
    let xs: Vec<f64> = pairs.iter().map(|p| p.0).collect();
    let ys: Vec<f64> = pairs.iter().map(|p| p.1).collect();
    pearson(&average_ranks(&xs), &average_ranks(&ys))
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── SMA ──

    #[test]
    fn sma_basic() {
        assert_eq!(sma(&[1.0, 2.0, 3.0, 4.0], 2), Some(3.5));
        assert_eq!(sma(&[1.0, 2.0, 3.0], 3), Some(2.0));
    }

    #[test]
    fn sma_insufficient_data_returns_none() {
        assert_eq!(sma(&[1.0, 2.0], 5), None);
    }

    #[test]
    fn sma_zero_period_returns_none() {
        assert_eq!(sma(&[1.0, 2.0, 3.0], 0), None);
    }

    #[test]
    fn sma_empty_input_returns_none() {
        assert_eq!(sma(&[], 1), None);
    }

    // ── EMA 序列 ──

    #[test]
    fn ema_series_basic() {
        let series = build_ema_series(&[1.0, 2.0, 3.0, 4.0], 2);
        assert_eq!(series.len(), 4);
        // 首值 = SMA(1, 2) = 1.5
        assert!((series[0] - 1.5).abs() < 1e-10);
        // multiplier = 2/3, ema[1] = (2 - 1.5) * 2/3 + 1.5 = 1.8333...
        assert!((series[1] - 1.8333_3333).abs() < 1e-6);
    }

    #[test]
    fn ema_series_empty_input() {
        assert_eq!(build_ema_series(&[], 5), vec![0.0]);
    }

    #[test]
    fn ema_series_zero_period() {
        assert_eq!(build_ema_series(&[1.0, 2.0], 0), vec![0.0]);
    }

    #[test]
    fn ema_last_matches_series_end() {
        let data = vec![1.0, 2.0, 3.0, 4.0, 5.0];
        let series = build_ema_series(&data, 3);
        let last = ema_last(&data, 3);
        assert!((last - series.last().copied().unwrap_or(0.0)).abs() < 1e-10);
    }

    // ── RSI ──

    #[test]
    fn rsi_all_up_is_100() {
        // 持续上涨：avg_loss = 0 → RSI = 100
        let closes = vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0];
        assert_eq!(rsi_wilder(&closes, 5), Some(100.0));
    }

    #[test]
    fn rsi_all_down_is_0() {
        // 持续下跌：avg_gain = 0, rs = 0 → RSI = 0
        let closes = vec![6.0, 5.0, 4.0, 3.0, 2.0, 1.0];
        assert_eq!(rsi_wilder(&closes, 5), Some(0.0));
    }

    #[test]
    fn rsi_insufficient_data_returns_none() {
        assert_eq!(rsi_wilder(&[1.0, 2.0], 5), None);
    }

    #[test]
    fn rsi_zero_period_returns_none() {
        assert_eq!(rsi_wilder(&[1.0, 2.0, 3.0], 0), None);
    }

    #[test]
    fn rsi_mixed_market_in_range() {
        // 涨跌交替：RSI 应在 (0, 100) 之间
        // 注意：Wilder 平滑用指数递归，对涨跌顺序敏感，不保证对称涨跌返回 50
        let closes = vec![10.0, 11.0, 10.0, 11.0, 10.0, 11.0];
        let rsi = rsi_wilder(&closes, 5).expect("数据充足应返回 Some");
        assert!(rsi > 0.0 && rsi < 100.0, "RSI 应在 (0, 100) 区间, 实际: {}", rsi);
    }

    // ── 样本标准差 ──

    #[test]
    fn stddev_basic() {
        let data = vec![2.0, 4.0, 4.0, 4.0, 5.0, 5.0, 7.0, 9.0];
        let mean = data.iter().sum::<f64>() / data.len() as f64;
        let sd = stddev_sample(&data, mean);
        // 经典样本标准差 = 2.138...
        assert!((sd - 2.138).abs() < 0.01, "sd = {}", sd);
    }

    #[test]
    fn stddev_single_element_is_zero() {
        assert_eq!(stddev_sample(&[5.0], 5.0), 0.0);
    }

    #[test]
    fn stddev_empty_is_zero() {
        assert_eq!(stddev_sample(&[], 0.0), 0.0);
    }

    #[test]
    fn stddev_constant_series_is_zero() {
        // 常数序列方差为 0
        let data = vec![5.0, 5.0, 5.0, 5.0];
        assert_eq!(stddev_sample(&data, 5.0), 0.0);
    }

    // ── Sharpe Ratio (P3-C8) ──

    #[test]
    fn sharpe_components_basic() {
        // 正均值 → 正 sharpe
        let returns = vec![0.01, 0.02, -0.01, 0.005, 0.015];
        let r = sharpe_components(&returns, 0.0, 244.0);
        assert!(r.sharpe > 0.0, "正均值应有正 sharpe, got {}", r.sharpe);
        assert!(r.annualized > r.sharpe, "年化值应放大, sharpe={}, ann={}", r.sharpe, r.annualized);
        assert!(r.mean_return > 0.0);
        assert!(r.stddev > 0.0);
    }

    #[test]
    fn sharpe_components_insufficient_data() {
        let r = sharpe_components(&[0.01], 0.0, 244.0);
        assert_eq!(r.sharpe, 0.0);
        assert_eq!(r.annualized, 0.0);
        assert_eq!(r.mean_return, 0.0);
        assert_eq!(r.stddev, 0.0);
    }

    #[test]
    fn sharpe_components_empty() {
        let r = sharpe_components(&[], 0.0, 244.0);
        assert_eq!(r.sharpe, 0.0);
    }

    #[test]
    fn sharpe_components_constant_series_returns_zero_sharpe() {
        // 常数序列 stddev=0 → sharpe=0，但 mean_return 保留
        let r = sharpe_components(&[0.01, 0.01, 0.01], 0.0, 244.0);
        assert_eq!(r.sharpe, 0.0);
        assert_eq!(r.annualized, 0.0);
        assert_eq!(r.stddev, 0.0);
        assert!((r.mean_return - 0.01).abs() < 1e-10, "mean_return 应保留, got {}", r.mean_return);
    }

    #[test]
    fn sharpe_components_uses_sample_variance() {
        // 验证使用 n-1 而非 n 分母
        // 数据: [1, 2, 3, 4, 5]
        // mean = 3, Σ(x-mean)² = 4+1+0+1+4 = 10
        // 样本方差 = 10/4 = 2.5 → stddev = √2.5 ≈ 1.5811
        // 总体方差 = 10/5 = 2.0 → stddev = √2 ≈ 1.4142
        let returns = vec![1.0, 2.0, 3.0, 4.0, 5.0];
        let r = sharpe_components(&returns, 0.0, 1.0);
        let expected_stddev = (2.5_f64).sqrt();
        assert!(
            (r.stddev - expected_stddev).abs() < 1e-10,
            "应使用样本方差 n-1, expected {}, got {}",
            expected_stddev,
            r.stddev
        );
    }

    #[test]
    fn sharpe_ratio_convenience_uses_a_share_default() {
        // sharpe_ratio 应默认使用 244 天年化
        let returns = vec![0.01, 0.02, -0.01, 0.005, 0.015];
        let convenience = sharpe_ratio(&returns, 0.0);
        let explicit =
            sharpe_ratio_with_annualization(&returns, 0.0, A_SHARE_TRADING_DAYS_PER_YEAR);
        assert!((convenience - explicit).abs() < 1e-10);
    }

    #[test]
    fn sharpe_ratio_annual_converts_rf_correctly() {
        // sharpe_ratio_annual(rf_annual=0.03, days=244) 应等价于
        // sharpe_components(daily_rf=0.03/244, annualization=244).annualized
        let returns = vec![0.01, 0.02, -0.01, 0.005, 0.015];
        let annual = sharpe_ratio_annual(&returns, 0.03, 244.0);
        let daily = sharpe_components(&returns, 0.03 / 244.0, 244.0).annualized;
        assert!((annual - daily).abs() < 1e-12, "annual={}, daily={}", annual, daily);
    }

    #[test]
    fn sharpe_ratio_annual_zero_days_returns_zero() {
        let returns = vec![0.01, 0.02, -0.01];
        assert_eq!(sharpe_ratio_annual(&returns, 0.03, 0.0), 0.0);
    }

    #[test]
    fn sharpe_matches_astock_data_legacy_formula_after_fix() {
        // 验证修复 astock-data 总体方差 bug 后的算法等价性:
        // 修复前(bug): variance = Σ(x-mean)² / n
        // 修复后(correct): variance = Σ(x-mean)² / (n-1) — 本 harness 实现
        let returns = vec![0.01, 0.02, -0.01, 0.005, 0.015, 0.0, 0.012, -0.005];
        let n = returns.len() as f64;
        let mean = returns.iter().sum::<f64>() / n;
        let variance_sample = returns.iter().map(|r| (r - mean).powi(2)).sum::<f64>() / (n - 1.0);
        let variance_population = returns.iter().map(|r| (r - mean).powi(2)).sum::<f64>() / n;
        // 样本方差 > 总体方差（n-1 < n）， stddev 也更大 → Sharpe 绝对值更小
        assert!(variance_sample > variance_population);
        let r = sharpe_components(&returns, 0.0, 244.0);
        assert!((r.stddev - variance_sample.sqrt()).abs() < 1e-12);
    }

    #[test]
    fn realized_vol_pct_matches_hand_computed_sample_stdev() {
        // 收盘价 100 → 102 → 101 → 103：三条 return 已知，手算样本标准差
        let closes = [100.0, 102.0, 101.0, 103.0];
        let got = realized_vol_pct(&closes, 3).expect("应可计算");
        let rets = [0.02, -1.0 / 102.0, 2.0 / 101.0];
        let m = rets.iter().sum::<f64>() / 3.0;
        let v = rets.iter().map(|r| (r - m).powi(2)).sum::<f64>() / 2.0;
        assert!((got - v.sqrt() * 100.0).abs() < 1e-9, "实得 {got}");
    }

    #[test]
    fn realized_vol_pct_refuses_insufficient_and_invalid_samples() {
        // 样本不足 ⇒ None（不是 0.0 —— 0 波动会让止损宽度塌成 0）
        assert!(realized_vol_pct(&[10.0, 10.1], 20).is_none());
        assert!(realized_vol_pct(&[], 2).is_none());
        // lookback < 2 无定义
        assert!(realized_vol_pct(&[10.0, 11.0, 12.0], 1).is_none());
        // 含非正价格 ⇒ 收益率无定义，不得静默跳过
        assert!(realized_vol_pct(&[10.0, 0.0, 11.0, 12.0], 2).is_none());
        assert!(realized_vol_pct(&[10.0, f64::NAN, 11.0, 12.0], 2).is_none());
    }

    #[test]
    fn realized_vol_pct_uses_sample_not_population_variance() {
        // 与总体方差口径可区分：n=3 个 return 时两者相差 √(3/2)
        let closes = [100.0, 102.0, 99.0, 103.0];
        let sample = realized_vol_pct(&closes, 3).expect("应可计算");
        let rets = [0.02, -3.0 / 102.0, 4.0 / 99.0];
        let m = rets.iter().sum::<f64>() / 3.0;
        let pop = (rets.iter().map(|r| (r - m).powi(2)).sum::<f64>() / 3.0).sqrt() * 100.0;
        assert!(
            (sample - pop * (3.0_f64 / 2.0).sqrt()).abs() < 1e-9,
            "σ 口径漂移：样本 {sample} vs 总体 {pop}"
        );
    }

    /// √t 缩放：h=4 的位移应是 h=1 的两倍；h=0 与样本不足都拒绝出数。
    #[test]
    fn vol_move_pct_scales_by_sqrt_of_holding_days() {
        let closes: Vec<f64> = (0..25).map(|i| 100.0 + (i % 5) as f64).collect();
        let one = vol_move_pct(&closes, 20, 1).expect("h=1 应可计算");
        let four = vol_move_pct(&closes, 20, 4).expect("h=4 应可计算");
        let nine = vol_move_pct(&closes, 20, 9).expect("h=9 应可计算");
        assert!((four - one * 2.0).abs() < 1e-9, "√4 应为 2 倍，实得 {four}/{one}");
        assert!((nine - one * 3.0).abs() < 1e-9, "√9 应为 3 倍，实得 {nine}/{one}");
        assert!(vol_move_pct(&closes, 20, 0).is_none(), "持有 0 天无定义");
        assert!(vol_move_pct(&[100.0, 101.0], 20, 5).is_none(), "样本不足不得硬算");
    }

    /// SNR 折算：锚定档不改、短档收缩、长档放大，且对 0.5 两侧对称。
    #[test]
    fn snr_confidence_scales_edge_by_sqrt_of_horizon() {
        assert!((snr_confidence(0.60, 28, 28) - 0.60).abs() < 1e-12, "锚定档不应改变");
        let up7 = snr_confidence(0.60, 63, 28); // √(63/28)=1.5
        assert!((up7 - 0.65).abs() < 1e-9, "长线应把 +0.10 边缘放大到 +0.15，实得 {up7}");
        let down = snr_confidence(0.40, 63, 28);
        assert!((down - 0.35).abs() < 1e-9, "看空必须对称放大，实得 {down}");
        let short = snr_confidence(0.60, 7, 28); // √0.25=0.5
        assert!((short - 0.55).abs() < 1e-9, "短线应收敛到 +0.05，实得 {short}");
    }

    #[test]
    fn snr_confidence_clamps_and_degrades_without_guessing() {
        assert_eq!(snr_confidence(0.99, 90, 28), 1.0, "放大后必须夹在 [0,1]");
        assert_eq!(snr_confidence(0.01, 90, 28), 0.0);
        assert_eq!(snr_confidence(0.62, 0, 28), 0.62, "h=0 无从折算 ⇒ 原样返回");
        assert_eq!(snr_confidence(0.62, 90, 0), 0.62, "anchor=0 无从折算 ⇒ 原样返回");
        assert!(snr_confidence(f64::NAN, 90, 28).is_nan(), "NaN 不得被夹成 0");
    }

    // ── 秩相关（rank IC 的唯一实现，Phase E 收编三处重复）──

    #[test]
    fn average_ranks_uses_one_based_mid_ranks_with_epsilon_ties() {
        // 精确并列与 1e-12 差值都要归入同组（沿用 hit_rate_backtest 的 ties 判据）
        assert_eq!(average_ranks(&[10.0, 10.0, 30.0]), vec![1.5, 1.5, 3.0]);
        // 1e-13 差值算并列 ⇒ 两者取 2、3 位的均值 2.5，最小者秩 1
        assert_eq!(average_ranks(&[20.0, 20.0000000000001, 5.0]), vec![2.5, 2.5, 1.0]);
        // 降序输入不影响秩分配（秩按值大小，不按位置）
        assert_eq!(average_ranks(&[30.0, 20.0, 10.0]), vec![3.0, 2.0, 1.0]);
        assert!(average_ranks(&[]).is_empty());
    }

    #[test]
    fn pearson_none_when_undefined_never_fake_zero() {
        assert!((pearson(&[1.0, 2.0, 3.0], &[2.0, 4.0, 6.0]).unwrap() - 1.0).abs() < 1e-12);
        assert!((pearson(&[1.0, 2.0, 3.0], &[6.0, 4.0, 2.0]).unwrap() + 1.0).abs() < 1e-12);
        // 长度不等 / 不足 2 点 / 一侧无方差 / 含非有限 ⇒ 全部 None
        assert_eq!(pearson(&[1.0, 2.0], &[1.0, 2.0, 3.0]), None);
        assert_eq!(pearson(&[1.0], &[1.0]), None);
        assert_eq!(pearson(&[3.0, 3.0, 3.0], &[1.0, 2.0, 3.0]), None);
        assert_eq!(pearson(&[f64::NAN, 2.0, 3.0], &[1.0, 2.0, 3.0]), None);
    }

    #[test]
    fn spearman_is_order_only_and_pins_closed_form_for_ties() {
        // 严格单调 ⇒ 1（与缩放量级无关）
        let mono: Vec<(f64, f64)> = (0..9).map(|i| (i as f64, (i as f64).powi(3))).collect();
        assert_eq!(spearman_rank_ic(&mono), Some(1.0));
        // 有并列：x=[1,1,2,3] y=[1,2,3,4] ⇒ 闭式解 4.5/√22.5 = 0.9486833
        let tied = vec![(1.0, 1.0), (1.0, 2.0), (2.0, 3.0), (3.0, 4.0)];
        assert!((spearman_rank_ic(&tied).unwrap() - 0.948_683_3).abs() < 1e-6);
        // 无定义 ⇒ None（不是 0）
        assert_eq!(spearman_rank_ic(&[(0.5, 1.0), (0.5, 2.0), (0.5, 3.0)]), None);
        assert_eq!(spearman_rank_ic(&[(0.5, f64::NAN)]), None);
    }
}
