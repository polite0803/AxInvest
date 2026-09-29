//! 荐股链的波动率风控（Phase R-D）
//!
//! ## 为什么单独一层
//!
//! 此前每张策略各自带一张**固定百分比**价带（`trend.rs` 的 0.98/0.95/0.92/0.85 与
//! 1.05/1.10/1.20/1.30 等），于是同一组数字既套低波蓝筹也套高波题材、既套 2 天也套 90 天
//! —— 止损既不是波动率刻度、也不是持有期函数（诊断 R5/E5）。
//!
//! 本层把止损/止盈改为 `k · σ_daily · √h`（波动率目标法与三阶障碍法上下轨的共同基础），
//! 仓位改为**风险预算** `100·R/止损距离%`（止损被打掉时组合恰好损失 R%），
//! 并按 `1/持有天数` 摊薄换手成本（超短档不做这层修正，净期望必然被高估）。
//!
//! σ 的实现**只有一份**：`harness::indicators::realized_vol_pct`（样本标准差口径，
//! 全仓唯一 σ），本层不重写方差。σ 不可得 ⇒ 显式退回固定百分比并标 `stopSource="fallback_pct"`，
//! 不伪装成波动率口径。

use std::collections::HashMap;
use std::sync::{Arc, LazyLock};
use std::time::{Duration, Instant};

use axagent_astock_data::AStockClient;
use axagent_harness::indicators::realized_vol_pct;
use parking_lot::Mutex;

/// 日收益率样本窗口（σ 的估计口径：近 40 个交易日）。
pub const SIGMA_LOOKBACK: usize = 40;
/// 止损下限/上限（%）：防止 σ 估计异常时给出 0 或荒谬宽度的止损。
pub const STOP_FLOOR_PCT: f64 = 0.5;
pub const STOP_CAP_PCT: f64 = 15.0;
/// 止盈下限/上限（%）。
pub const TARGET_FLOOR_PCT: f64 = 1.0;
pub const TARGET_CAP_PCT: f64 = 40.0;
/// 风险预算仓位上限（%）——与「全仓单票不超过 95%」的既有护栏一致。
pub const POSITION_CAP_PCT: f64 = 95.0;

/// 日线收盘价的进程内备忘（按 `(code, as-of 后缀)` 键）。
///
/// 为什么要备忘：四档各自的尺度不同（小时/日/周/季），但 **σ 必须是日线口径**才能可比；
/// 不缓存就是每档每票各取一次日线（四倍取数）。TTL 与荐股结果缓存同级（60 s），
/// 键空间与 `RESULT_CACHE` 一样带 as-of 后缀 ⇒ live/replay 互不污染。
type DailyClosesEntry = (Arc<Vec<f64>>, Instant);
type DailyClosesCache = Mutex<HashMap<(String, String), DailyClosesEntry>>;

static DAILY_CLOSES: LazyLock<DailyClosesCache> = LazyLock::new(|| Mutex::new(HashMap::new()));
const DAILY_TTL: Duration = Duration::from_secs(60);

/// 取（或复用）某只票的日线收盘序列。失败返回 `None`，调用方必须显式降级。
pub async fn daily_closes(client: &AStockClient, code: &str) -> Option<Arc<Vec<f64>>> {
    let suffix = axagent_astock_data::as_of::cache_suffix();
    let key = (code.to_string(), suffix.clone());
    if let Some((v, ts)) = DAILY_CLOSES.lock().get(&key) {
        if ts.elapsed() < DAILY_TTL {
            return Some(v.clone());
        }
    }
    let klines = client.get_klines(code, "daily", 120).await.ok()?;
    let closes: Vec<f64> = klines.iter().map(|k| k.close).collect();
    let arc = Arc::new(closes);
    DAILY_CLOSES.lock().insert(key, (arc.clone(), Instant::now()));
    Some(arc)
}

/// 该档的止损距离（%）= `k1 · σ_daily · √h`，夹在 `[STOP_FLOOR_PCT, STOP_CAP_PCT]`。
///
/// `σ` 不可得 / 样本不足 ⇒ `None`（调用方退回固定百分比并标注来源）。
pub fn stop_pct(closes: &[f64], holding_days: usize, k1: f64) -> Option<f64> {
    let sigma = realized_vol_pct(closes, SIGMA_LOOKBACK)?;
    let raw = k1 * sigma * (holding_days as f64).sqrt();
    if !raw.is_finite() {
        return None;
    }
    Some(raw.clamp(STOP_FLOOR_PCT, STOP_CAP_PCT))
}

/// 该档的目标位移（%）= `k2 · σ_daily · √h`，夹在 `[TARGET_FLOOR_PCT, TARGET_CAP_PCT]`。
pub fn target_pct(closes: &[f64], holding_days: usize, k2: f64) -> Option<f64> {
    let sigma = realized_vol_pct(closes, SIGMA_LOOKBACK)?;
    let raw = k2 * sigma * (holding_days as f64).sqrt();
    if !raw.is_finite() {
        return None;
    }
    Some(raw.clamp(TARGET_FLOOR_PCT, TARGET_CAP_PCT))
}

/// 风险预算仓位（%）= `100 · R / 止损距离%` ⇒ 止损被打掉时组合恰好损失 R%。
///
/// 这是**仓位承载风险**的定义式，不含任何周期经验乘数；上限 `POSITION_CAP_PCT`。
pub fn risk_budget_position(stop_pct: f64, risk_budget_pct: f64) -> Option<f64> {
    // 写成 is_finite + <= 而不是 `!(x > 0.0)`：两者对 NaN 同判（都拒），但 clippy 的
    // `neg_cmp_op_on_partial_ord` 要求显式表达「可能不可比」。
    if !stop_pct.is_finite()
        || stop_pct <= 0.0
        || !risk_budget_pct.is_finite()
        || risk_budget_pct <= 0.0
    {
        return None;
    }
    Some((100.0 * risk_budget_pct / stop_pct).clamp(0.0, POSITION_CAP_PCT))
}

/// 换手成本拖累因子：`1 − 往返成本 / 该档目标位移`，夹在 `[0, 1]`。
///
/// 依据：一次荐股往返要付两趟成本（进+出），而这笔成本相对**该档能拿到的位移**
/// 才算数 ⇒ 短档位移小，成本占比自动放大（PLAN §一.6「成本 ∝ 1/持有期」的可实现形式）。
/// 目标位移不可得 ⇒ 返回 1.0（不假装做了修正）。
pub fn cost_drag_factor(target_pct: f64, round_trip_cost_pct: f64) -> f64 {
    if !target_pct.is_finite()
        || target_pct <= 0.0
        || !round_trip_cost_pct.is_finite()
        || round_trip_cost_pct <= 0.0
    {
        return 1.0;
    }
    (1.0 - round_trip_cost_pct / target_pct).clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 等比放大的价格序列：σ 稳定 ⇒ 止损随 √h 单调放宽，且**高波票止损宽于低波票**。
    #[test]
    fn stop_scales_with_sqrt_holding_and_sigma() {
        // 低波：每日 ±1% 交替；高波：每日 ±6% 交替
        let low: Vec<f64> = (0..60).map(|i| if i % 2 == 0 { 100.0 } else { 101.0 }).collect();
        let high: Vec<f64> = (0..60).map(|i| if i % 2 == 0 { 100.0 } else { 106.0 }).collect();
        let s_low_2 = stop_pct(&low, 2, 1.2).expect("可算");
        let s_low_90 = stop_pct(&low, 90, 1.2).expect("可算");
        let s_high_90 = stop_pct(&high, 90, 1.2).expect("可算");
        assert!(s_low_90 > s_low_2, "止损必须随持有期放宽: {s_low_2} → {s_low_90}");
        assert!(
            s_high_90 > s_low_90,
            "同一持有期下高波标的止损% 必须大于低波标的: {s_high_90} vs {s_low_90}"
        );
    }

    /// 样本不足 / 非正价格 ⇒ None（不许当成 0 波动给出一个正常读数）。
    #[test]
    fn refuses_when_sigma_undefined() {
        assert!(stop_pct(&[100.0, 101.0], 5, 1.2).is_none());
        let bad: Vec<f64> = (0..60).map(|i| if i == 30 { -1.0 } else { 100.0 }).collect();
        assert!(stop_pct(&bad, 5, 1.2).is_none());
    }

    /// 风险预算仓位：止损越宽 ⇒ 仓位越小（这正是「仓位承载风险」的含义）。
    #[test]
    fn risk_budget_position_is_inverse_of_stop_distance() {
        let tight = risk_budget_position(2.0, 1.5).expect("可算");
        let wide = risk_budget_position(10.0, 1.5).expect("可算");
        assert!((tight - 75.0).abs() < 1e-9, "100×1.5/2 = 75，实际 {tight}");
        assert!((wide - 15.0).abs() < 1e-9);
        assert!(risk_budget_position(0.0, 1.5).is_none(), "止损 0 ⇒ 不可算，不得给出仓位");
    }

    /// 成本拖累：同一往返成本，目标位移越小扣得越多；位移不可得时不修正。
    #[test]
    fn cost_drag_penalizes_short_horizons() {
        let near = cost_drag_factor(2.0, 0.6); // 往返成本占目标位移 30%
        let far = cost_drag_factor(20.0, 0.6); // 占 3%
        assert!(near < far, "小位移必须被成本吃掉更多: {near} vs {far}");
        assert!((far - 0.97).abs() < 1e-9);
        assert!((cost_drag_factor(0.0, 0.6) - 1.0).abs() < 1e-12, "位移不可得 ⇒ 不假装修正");
        assert!(cost_drag_factor(0.1, 0.6) == 0.0, "成本超过位移 ⇒ 拖到 0（净期望为负）");
    }
}
