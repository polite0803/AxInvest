//! 决策时刻因子计算（纯函数，无 IO、无时间依赖）。
//!
//! ## 为什么要有这个模块
//!
//! `AUDIT-factor-evidence-2026-10-25.md` 实测：只用收盘价就能算出的两个因子在两个持有期
//! 都显著 —— `chg1`（当日涨跌幅，正向）与 `dist_lo20`（距 20 日最低收盘价的涨幅，负向），
//! 且**剔除涨跌停后 IC 几乎不变**（⇒ 信号不是涨停股假象，可买入执行）。
//!
//! 但系统从未把因子写进 `factor_snapshot`（`build_pick_validation` 里是硬编码 `None`），
//! 所以 1,149 行判定样本**一行因子都没有**，IC 归因无从做起。
//!
//! 本模块只做一件事：给定**已按日期升序、且已截断到决策日**的价格序列，算出因子值。
//!
//! ## 前视偏差防线（最重要的一节）
//!
//! **本模块不做任何截断。** 调用方必须传入「截止到决策日（含）」的序列。
//!
//! 若把全量序列传进来、指望模块「只看前面几根」，任何跨索引统计都会读到未来 ——
//! 这是 `AUDIT-replay-sample-feasibility-2026-10-25.md` 已识别的污染模式
//! （1,145 行提前窗口污染就是这么产生的）。
//! 该约定在此显式声明，避免下游误用：**截断是调用方的责任，不是本模块的责任。**
//!
//! ## 为什么 quintile / universe_n 不在这里算
//!
//! 分位需要**当日全市场横截面**（约 5,500 只股票），单票路径拿不到。
//! ⇒ 本模块只产**原始因子值**；分位（`quintile` / `score_pct` / `universe_n`）
//! 由下游在横截面上计算。硬要在单票路径里算分位，只能伪造一个分母 ——
//! 而分位离开分母没有意义。

use std::collections::HashMap;

/// 因子口径版本。
///
/// **改动任何因子定义（窗口、公式、字段含义）必须递增**，否则新旧快照会被当成同一代
/// 混进同一个 IC 统计里 —— 同 `RECO_ALGORITHM_VERSION` / `TEMPLATE_VERSION` 的纪律。
pub const FACTOR_VERSION: f64 = 1.0;

/// 滚动窗口长度（交易日）。
pub const WINDOW: usize = 20;

/// 计算 `rev20` 所需的最小 bar 数。
///
/// `rev20 = close[n-1] / close[n-21] - 1`，故至少需要 21 根。
/// 少一根就返回 `None`，**不得用短窗口凑**（会把 10 日收益标成 20 日，与
/// `fetch_window_series` 里 T+N 窗口不足的修复同族）。
pub const MIN_BARS: usize = 21;

/// 计算因子所需的最小 bar。只关心收盘价与当日涨跌幅。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FactorBar {
    pub close: f64,
    /// 当日涨跌幅（%）。
    ///
    /// **不可由本模块从收盘价反推**：除权日反推会得到假跌幅，
    /// 而 `chg1` 恰恰是实测 IC 最高的因子，错的 `chg1` 比没有更糟。
    pub change_pct: f64,
}

/// 单票在决策时刻的因子集合。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DecisionFactors {
    /// 当日涨跌幅（%）。实测 IC 正向，T+5 约 +0.18。
    pub chg1: f64,
    /// 收盘价相对近 20 日最低收盘价的涨幅（%），恒 ≥ 0。实测 IC 负向，T+5 约 −0.16。
    pub dist_lo20: f64,
    /// 近 20 个交易日累计收益（%）。实测 IC 负向（20 日反转）。
    pub rev20: f64,
    /// 近 20 个交易日上涨天数占比（0~100）。实测 IC 负向。
    pub posshare20: f64,
}

impl DecisionFactors {
    /// 转成 `PickValidation.factor_snapshot` 用的扁平 map（`HashMap<String, f64>`）。
    ///
    /// 键名与 `AUDIT-factor-evidence-2026-10-25.md` 的 SQL 列名**逐字对应**，
    /// 便于把「Rust 记录的因子」与「SQL 重算的因子」直接比对（口径漂移检测）。
    pub fn to_snapshot(self) -> HashMap<String, f64> {
        let mut m = HashMap::with_capacity(5);
        m.insert("chg1".to_string(), self.chg1);
        m.insert("dist_lo20".to_string(), self.dist_lo20);
        m.insert("rev20".to_string(), self.rev20);
        m.insert("posshare20".to_string(), self.posshare20);
        m.insert("factor_version".to_string(), FACTOR_VERSION);
        m
    }
}

/// 计算截止到最后一根 bar（含）的因子。
///
/// 返回 `None` 的情形（全部是「宁可不算，也不产假值」）：
/// - bar 数 < [`MIN_BARS`]
/// - 任一用于计算的收盘价非正或非有限（停牌/缺数据/零价）
/// - 20 日最低收盘价非正（会让 `dist_lo20` 变成 inf）
/// - 结果非有限
pub fn compute_factors(bars: &[FactorBar]) -> Option<DecisionFactors> {
    let n = bars.len();
    if n < MIN_BARS {
        return None;
    }
    let last = bars[n - 1];
    if !last.close.is_finite() || last.close <= 0.0 {
        return None;
    }
    // 20 日窗口 = 最后 WINDOW 根（含当前根）
    let window = &bars[n - WINDOW..];
    let mut lo = f64::INFINITY;
    let mut up_days = 0usize;
    for b in window {
        if !b.close.is_finite() || b.close <= 0.0 {
            return None;
        }
        if b.close < lo {
            lo = b.close;
        }
        if b.change_pct > 0.0 {
            up_days += 1;
        }
    }
    // rev20 的基期：n-WINDOW-1（即 n-21）
    let base = bars[n - MIN_BARS].close;
    if !base.is_finite() || base <= 0.0 || !lo.is_finite() || lo <= 0.0 {
        return None;
    }
    let dist_lo20 = (last.close / lo - 1.0) * 100.0;
    let rev20 = (last.close / base - 1.0) * 100.0;
    let posshare20 = up_days as f64 / WINDOW as f64 * 100.0;
    if !last.change_pct.is_finite() || !dist_lo20.is_finite() || !rev20.is_finite() {
        return None;
    }
    Some(DecisionFactors { chg1: last.change_pct, dist_lo20, rev20, posshare20 })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bar(close: f64, change_pct: f64) -> FactorBar {
        FactorBar { close, change_pct }
    }

    /// bar 数不足 ⇒ 必须 None，不得用短窗口凑。
    #[test]
    fn insufficient_bars_yield_none() {
        assert!(compute_factors(&[]).is_none());
        // MIN_BARS-1 根：差一根就不算
        let bars: Vec<FactorBar> = (0..MIN_BARS - 1).map(|i| bar(100.0 + i as f64, 1.0)).collect();
        assert!(compute_factors(&bars).is_none(), "少一根不能算 rev20");
        // 恰好 MIN_BARS 根：可以算
        let bars: Vec<FactorBar> = (0..MIN_BARS).map(|i| bar(100.0 + i as f64, 1.0)).collect();
        assert!(compute_factors(&bars).is_some(), "恰好 {MIN_BARS} 根应当可算");
    }

    /// 手工构造已知序列，逐因子核对。
    ///
    /// closes: 100,101,...,120（共 21 根，步长 1）
    /// ⇒ lo20 = 101（窗口是 bars[1..21]）
    /// ⇒ dist_lo20 = (120/101 - 1)*100
    /// ⇒ rev20      = (120/100 - 1)*100 = 20
    /// ⇒ posshare20 = 100（20 天全涨）
    #[test]
    fn known_series_matches_hand_calculation() {
        let mut bars: Vec<FactorBar> = (0..MIN_BARS).map(|i| bar(100.0 + i as f64, 1.0)).collect();
        bars[MIN_BARS - 1].change_pct = 2.0; // 当日涨幅单列
        let f = compute_factors(&bars).expect("应当可算");

        assert!((f.chg1 - 2.0).abs() < 1e-12, "chg1 应等于最后一根的 change_pct");
        assert!((f.rev20 - 20.0).abs() < 1e-12, "rev20 = 120/100-1 = 20%，实测 {}", f.rev20);
        assert!(
            (f.dist_lo20 - (120.0 / 101.0 - 1.0) * 100.0).abs() < 1e-9,
            "dist_lo20 应以窗口内最低收盘 101 为基期，实测 {}",
            f.dist_lo20
        );
        assert!((f.posshare20 - 100.0).abs() < 1e-12, "20 天全涨 ⇒ 100%");
    }

    /// posshare20 只数**严格大于 0** 的天，平盘不算上涨。
    ///
    /// 窗口 bars[1..21] 共 20 根，索引 2,4,…,20 为 +1（10 根），1,3,…,19 为 0（10 根）
    /// ⇒ 正确值 50%。若把平盘也算上涨会得到 100% —— 这条断言就是区分两者的判据。
    #[test]
    fn posshare_counts_strictly_positive_days() {
        let bars: Vec<FactorBar> = (0..MIN_BARS)
            .map(|i| {
                let chg = if i % 2 == 0 { 1.0 } else { 0.0 };
                bar(100.0 + i as f64, chg)
            })
            .collect();
        let f = compute_factors(&bars).expect("应当可算");
        assert!(
            (f.posshare20 - 50.0).abs() < 1e-12,
            "20 根里 10 根上涨 ⇒ 50%，实测 {}（若为 100 则平盘被算成上涨）",
            f.posshare20
        );
    }

    /// 零价 / 负价必须返回 None，不得产出 inf 或 NaN。
    #[test]
    fn non_positive_close_yields_none() {
        let mut bars: Vec<FactorBar> = (0..MIN_BARS).map(|i| bar(100.0 + i as f64, 1.0)).collect();
        // 基期（n-MIN_BARS = 0）置 0
        bars[0].close = 0.0;
        assert!(compute_factors(&bars).is_none(), "基期为 0 会让 rev20 变 inf");

        let mut bars: Vec<FactorBar> = (0..MIN_BARS).map(|i| bar(100.0 + i as f64, 1.0)).collect();
        // 窗口内（最后一根）置 0
        let last = MIN_BARS - 1;
        bars[last].close = 0.0;
        assert!(compute_factors(&bars).is_none(), "当前价为 0 应拒绝");
    }

    /// NaN 输入不得穿透成 NaN 因子。
    #[test]
    fn nan_inputs_yield_none() {
        let mut bars: Vec<FactorBar> = (0..MIN_BARS).map(|i| bar(100.0 + i as f64, 1.0)).collect();
        bars[MIN_BARS - 1].close = f64::NAN;
        assert!(compute_factors(&bars).is_none());
        let mut bars: Vec<FactorBar> = (0..MIN_BARS).map(|i| bar(100.0 + i as f64, 1.0)).collect();
        bars[MIN_BARS - 1].change_pct = f64::NAN;
        assert!(compute_factors(&bars).is_none(), "chg1 是 IC 最高的因子，污染了必须拒绝");
    }

    /// 快照键名必须与 SQL 列名逐字一致（口径漂移检测的锚点），且带版本号。
    #[test]
    fn snapshot_keys_and_version() {
        let bars: Vec<FactorBar> = (0..MIN_BARS).map(|i| bar(100.0 + i as f64, 1.0)).collect();
        let snap = compute_factors(&bars).unwrap().to_snapshot();
        for k in ["chg1", "dist_lo20", "rev20", "posshare20"] {
            assert!(snap.contains_key(k), "缺少因子键 {k}");
        }
        // 版本号必须落进快照 —— 否则将来改口径无法区分新旧行
        assert_eq!(snap.get("factor_version").copied(), Some(FACTOR_VERSION));
    }

    /// 负控：单调下跌序列里 dist_lo20 必须为 0（当前价就是 20 日最低），
    /// 而 posshare20 必须为 0。若这对断言不过，说明分母/窗口取错了。
    #[test]
    fn monotonic_down_series_has_zero_distance_and_zero_updays() {
        let bars: Vec<FactorBar> = (0..MIN_BARS).map(|i| bar(200.0 - i as f64, -1.0)).collect();
        let f = compute_factors(&bars).expect("应当可算");
        assert!(f.dist_lo20.abs() < 1e-12, "当前价即最低价 ⇒ dist_lo20 = 0，实测 {}", f.dist_lo20);
        assert!(f.posshare20.abs() < 1e-12, "全跌 ⇒ 上涨天数 0");
        assert!(f.rev20 < 0.0, "单调下跌 ⇒ rev20 为负");
    }
}
