// SPDX-License-Identifier: AGPL-3.0-only

//! 决策事后验证（V55 新增）—— hit_rate / 假阳性率 / 9 因子 IC 测算
//!
//! ## 背景
//! 股票分析系统的最大问题是"决策可采信度低"。本模块用历史数据反推当前决策系统的
//! 真实命中率与因子有效性，把"事后验证"做成系统的一等公民。
//!
//! ## 数据流
//! ```text
//! reco_picks 表 (决策输出)
//!     ↓ run_decision_backtest 命令
//! 拉取 T+5 / T+20 / T+60 实际价格 (行情 API)
//!     ↓
//! validate_pick(): 推断 action → 计算 hit_outcome → 写 decision_validations
//!     ↓
//! compute_hit_rate_report(): 聚合 hit_rate + 9 因子 IC
//!     ↓
//! 输出给前端 / 反馈到 portfolio-mgr.rhai 因子权重
//! ```
//!
//! ## 关键设计
//! - 推断 action（不依赖显式 action 字段）：target_price > price × 1.01 → "buy"；
//!   target_price < price × 0.99 → "sell"，否则 "hold"（阈值见 [`infer_action`]）
//! - hit_outcome 判定（buy）：**净收益** > 0 → "hit"；触及 stop_loss → "miss" 优先；
//!   触及 target → "hit"；净收益 < -5% → "false_hit"；介于 -5%~0 → "partial"
//! - **交易成本**：自 2026-10-25（G4）起，判定与统计一律用**净**收益
//!   （毛收益 − [`crate::costs::DEFAULT_ROUND_TRIP_COST_PCT`]）。此前完全不扣成本、
//!   且判 `ret > 0` ⇒ `+0.01%` 也算赢，胜率与均值系统性虚高（越短档越严重）。
//!   落库的 `final_return_pct` 仍是**毛**收益（历史可比），净口径由
//!   [`PickValidation::net_return_pct`] 恒时现算 ⇒ 历史样本也自动纳入净统计。
//! - factor IC = Spearman(因子值, 实际收益) — 衡量该因子对未来收益的预测能力
//! - 9 因子权重若无 IC 标定则全为 0.15/0.20/0.25 等"专家拍脑袋"值，
//!   这是 portfolio-mgr.rhai 可信度低的根因之一。

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::recommender::types::RecoPick;

// ── 数据结构 ──

/// 单条决策的事后验证结果（对应 decision_validations 表的一行）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PickValidation {
    pub pick_id: String,
    pub stock_code: String,
    pub stock_name: String,
    pub style: String,
    pub period: String,
    pub generated_at: String,
    pub t_plus_n: i32,
    pub entry_price: f64,
    pub target_price: f64,
    pub stop_loss: f64,
    pub position_pct: f64,
    pub confidence: i32,
    /// 由 target_price vs price 推断（"buy" | "sell" | "hold"）
    pub inferred_action: String,
    pub t_plus_n_price: Option<f64>,
    pub max_price: Option<f64>,
    pub min_price: Option<f64>,
    pub max_return_pct: Option<f64>,
    pub max_drawdown_pct: Option<f64>,
    pub final_return_pct: Option<f64>,
    pub hit_stop_loss: Option<i32>,
    pub hit_target: Option<i32>,
    /// "hit" | "miss" | "false_hit" | "partial" | "insufficient"
    pub hit_outcome: Option<String>,
    /// 9 因子快照（key=factor_id, value=0-1 数值），供 IC 重标定
    pub factor_snapshot: Option<HashMap<String, f64>>,
    pub data_source: String,
    /// **代际标记**：`None` = 验证实盘出票；`Some(YYYY-MM-DD)` = 验证时间旅行回放的
    /// 出票，值同时是该样本的**决策日**（回放 pick 的验证窗口从这天往后推，而不是
    /// 从入库时刻 `generated_at` 推 —— 后者会用决策之后的行情验证一个过去的决策）。
    /// 与 `reco_picks.as_of_date` / `decision_validations.as_of_date` 同值。
    pub as_of_date: Option<String>,
}

impl PickValidation {
    /// **净收益（%）** = `final_return_pct` − 默认双边成本。
    ///
    /// ## 为什么是方法而不是字段
    ///
    /// ① 字段会要求**所有构造点**同步（含本文件与 `serenity_hit_rate_backtest`
    ///    及大量单测），只要有一处漏填，净口径就**静默退回毛口径** —— 那正是本次
    ///    要修的病（两套口径并存且无人知道该看哪个）；
    /// ② `final_return_pct` 是**已落库列**，历史行存的是毛收益；方法是**恒可算**的，
    ///    历史行读出来也自动获得净口径，不需要 migration、不需要回填。
    ///
    /// ## 2026-10-25 (G4) 背景
    ///
    /// 此前本模块完全不扣成本，且 `compute_hit_outcome` 判 `ret > 0.0 → "hit"`
    /// ⇒ **+0.01% 也算赢**。真实读数因此系统性虚高，越短的档位虚高越多。
    /// 见 `AUDIT-stock-profitability-verdict-2026-10-25.md` §1.3。
    pub fn net_return_pct(&self) -> Option<f64> {
        self.final_return_pct.map(crate::costs::net_return_pct)
    }
}

/// 按 action / style / t_plus_n 维度的统计
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ActionStats {
    pub total: usize,
    pub hit: usize,
    pub miss: usize,
    pub false_hit: usize,
    pub partial: usize,
    pub insufficient: usize,
    /// **毛**平均收益（%）—— 不扣成本，仅作对照，不可据此判断是否赚钱。
    pub avg_return_pct: f64,
    pub median_return_pct: f64,
    /// **毛**命中率 = hit / (hit + miss + false_hit)，`partial` 与 `insufficient` 不进分母。
    ///
    /// ⚠ **legacy 口径**：`partial`（-5%~0 的微亏）不计入分母 ⇒ 读数系统性偏高。
    /// 新代码请读 [`Self::strict_win_rate`]（partial 计输）与 [`Self::net_win_rate`]
    /// （扣成本后为正）。保留本字段只为让既有面板与历史基线可比。
    pub hit_rate: f64,
    /// **strict 胜率** = hit / (hit + miss + false_hit + partial)，`partial` 计输。
    ///
    /// 与 `hit_rate` 的**唯一差别**是 partial 的归属；`insufficient`（无行情）两边都不进
    /// 分母。这是 `AUDIT-stock-live-data-2026-10-24.md` §4.1 用的 42.5% 那个口径。
    pub strict_win_rate: f64,
    /// strict 胜率的分母（有方向判定的样本数）。为 0 时不可展示成 0% 胜率。
    ///
    /// ⚠ 与 [`Self::net_win_denom`] **不同源**：本字段吃落库的 `hit_outcome`，
    /// 后者吃 `final_return_pct`。正常数据下同源（`hit_outcome` 由收益派生），
    /// 但「有判定标签、缺收益列」的残缺行会让两者分叉 —— 不得假定恒等。
    pub strict_denom: usize,
    /// **净**平均收益（%）：逐条扣 [`crate::costs::DEFAULT_ROUND_TRIP_COST_PCT`] 后求均值。
    /// 这是「能不能赚钱」唯一该看的均值。
    pub net_avg_return_pct: f64,
    /// **净**胜率 = 净收益 > 0 的样本 / 有净收益的样本。实盘意义上的「赢」。
    pub net_win_rate: f64,
    /// 净胜率的分母。为 0 ⇒ 无样本，`net_win_rate` 不得被读成 0% 胜率。
    pub net_win_denom: usize,
}

/// 整体 hit_rate 报告
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HitRateReport {
    pub total: usize,
    pub generated_at: String,
    /// key=action, value=stats
    pub by_action: HashMap<String, ActionStats>,
    /// key=style, value=stats
    pub by_style: HashMap<String, ActionStats>,
    /// key=t_plus_n (5/20/60), value=stats
    pub by_t_plus_n: HashMap<String, ActionStats>,
    /// 9 因子 IC（Spearman 等级相关）
    /// key=factor_id (f1_technical, f2_consensus, ...), value=IC in [-1, 1]
    /// IC 接近 0 = 该因子无预测力
    /// IC > 0.1 = 有正向预测力
    /// IC < -0.05 = 反向预测（应做空或反权重）
    pub factor_ic: HashMap<String, f64>,
    /// IC 绝对值排序：最有预测力的因子 → 最没预测力的因子
    pub factor_ic_ranked: Vec<(String, f64)>,
    /// 命中最好的 5 条决策（用于"亮点"展示）
    pub best_picks: Vec<PickValidation>,
    /// 命中最差的 5 条决策（用于"教训"展示）
    pub worst_picks: Vec<PickValidation>,
}

// ── 核心函数 ──

/// 推断 action：从 target_price / price 关系推断（不依赖显式 action 字段）
///
/// 规则：
/// - target_price / price > 1.01 → "buy"（隐含预期上涨 1%+）
/// - target_price / price < 0.99 → "sell"（隐含预期下跌 1%+）
/// - 其他 → "hold"
///
/// 修复 P1: 原阈值 3% 偏高 —— A 股 T+1 持有至少 1 日，策略计算出的 target_price
/// 通常已包含最小预期涨幅。3% 阈值会把"轻微看涨"（1-3%）误判为 "hold"，
/// 导致命中率统计分母偏大。降到 1% 更贴合实际信号强度。
pub fn infer_action(price: f64, target_price: f64) -> &'static str {
    if price <= 0.0 {
        return "hold";
    }
    let ratio = target_price / price;
    if ratio > 1.01 {
        "buy"
    } else if ratio < 0.99 {
        "sell"
    } else {
        "hold"
    }
}

/// 计算单条决策的 hit_outcome（基于 T+N 实际价格）
///
/// ## ⚠ 入参必须是**净**收益（已扣交易成本）
///
/// 2026-10-25（G4）：本函数原文档写 `final_return_pct > 0% → "hit"`，而调用方传的是
/// **毛**收益 ⇒ **+0.01% 也算命中**，实盘却是亏成本。现约定入参为净收益，
/// 调用方用 [`PriceMetrics::net_return_pct_default`] / [`PickValidation::net_return_pct`]。
/// 判定阈值本身不变（`> 0` 就是「实盘赚钱」），变的是**零点的基准**。
///
/// ## 判定规则（**顺序即优先级**，与函数体逐行一致）
/// - **buy**:
///   - 触及 stop_loss → "miss"（止损优先于任何盈利判定）
///   - 触及 target 或 净收益 > 0% → "hit"
///   - 净收益 <= -5% → "false_hit"（"看多却跌" 是最严重的误判）
///   - 其余（-5% ~ 0%）→ "partial"
/// - **sell**:
///   - 净收益 < 0% → "hit"（看空看对了）
///   - 净收益 > 5% → "false_hit"
///   - 其他 → "partial"
/// - **hold**: 窄幅震荡判断
///   - |净收益| <= 1% → "hit"（震荡判断正确，价格确实未明显变动）
///   - |净收益| > 5% → "false_hit"（判断震荡但市场走出趋势）
///   - 其他 → "partial"
/// - **数据不足**: 任意 None → "insufficient"
pub fn compute_hit_outcome(
    action: &str,
    net_return_pct: Option<f64>,
    hit_stop_loss: Option<i32>,
    hit_target: Option<i32>,
) -> Option<String> {
    let Some(ret) = net_return_pct else {
        return Some("insufficient".to_string());
    };
    match action {
        "buy" => {
            if hit_stop_loss == Some(1) {
                Some("miss".to_string())
            } else if hit_target == Some(1) || ret > 0.0 {
                Some("hit".to_string())
            } else if ret < -5.0 {
                Some("false_hit".to_string())
            } else {
                Some("partial".to_string())
            }
        },
        "sell" => {
            if ret < 0.0 {
                Some("hit".to_string())
            } else if ret > 5.0 {
                Some("false_hit".to_string())
            } else {
                Some("partial".to_string())
            }
        },
        // 修复 P1: hold 原一律 "partial"，未反映震荡判断的正确性。
        // 窄幅震荡（|ret| ≤ 1%）→ hit；明显趋势（|ret| > 5%）→ false_hit。
        "hold" => {
            let abs_ret = ret.abs();
            if abs_ret <= 1.0 {
                Some("hit".to_string())
            } else if abs_ret > 5.0 {
                Some("false_hit".to_string())
            } else {
                Some("partial".to_string())
            }
        },
        _ => Some("insufficient".to_string()),
    }
}

/// 从 K 线数据计算验证指标（T+N 收盘价 / 期间最大最小价 / 收益等）
///
/// ## 入参
/// - `entry_price`: 决策时的价格
/// - `closes`: 从决策日 T+1 到 T+N 的每日收盘价（按时间升序）
/// - `highs`: 对应每日最高价
/// - `lows`: 对应每日最低价
/// - `target_price`: 目标位
/// - `stop_loss`: 止损
///
/// ## 返回
/// 返回的字段都是 Option —— 数据缺失（k 线不足）时为 None
/// `compute_price_metrics` 的返回结果（全部为 Option，数据缺失时为 None）
#[derive(Debug, Clone, Copy, Default)]
pub struct PriceMetrics {
    pub t_plus_n_price: Option<f64>,
    pub max_price: Option<f64>,
    pub min_price: Option<f64>,
    pub max_return_pct: Option<f64>,
    pub max_drawdown_pct: Option<f64>,
    pub final_return_pct: Option<f64>,
    pub hit_stop_loss: Option<i32>,
    pub hit_target: Option<i32>,
}

impl PriceMetrics {
    /// 净收益（%）= 毛收益 − 指定双边成本。
    ///
    /// 不 clamp：完全没动的票（毛 0%）实盘是亏成本的，必须为负。
    pub fn net_return_pct(&self, cost_pct: f64) -> Option<f64> {
        self.final_return_pct.map(|g| crate::costs::net_of_cost(g, cost_pct))
    }

    /// 净收益（%）— 用 [`crate::costs::DEFAULT_ROUND_TRIP_COST_PCT`]。
    pub fn net_return_pct_default(&self) -> Option<f64> {
        self.net_return_pct(crate::costs::DEFAULT_ROUND_TRIP_COST_PCT)
    }
}

/// 入场价相对「决策日真实收盘」的最大容许偏离（%）。
///
/// ## 阈值不是拍的（2026-10-25 R-3a 实测）
///
/// `RecoPick.price` 来自 as-of 报价通道，可能与库内真实收盘价不符。实测偏离分布：
///
/// | 代际 | n | <0.5% | 0.5~5% | **≥5%** | 最大 |
/// |---|---:|---:|---:|---:|---:|
/// | 实盘 | 311 | 299 | 12 | **0** | **4.4%** |
/// | 回放 | 173 | 167 | 3 | **3** | **84.6%** |
///
/// ⇒ 5% 是能把「正常噪声」与「价格错配」分开的最小阈值：
/// **实盘 0 条误杀，回放精确命中那 3 条**（如 `000779` 记 1.63，真实收盘 10.60，
/// 而 1.63 恰是 `000980` 当日收盘 ⇒ 批量取价的行错位，不是复权、不是缓存）。
///
/// ## 为什么错价行必须**剔除**而不是「用真实价替换」
///
/// 错价行里 `targetPrice` / `stopLoss` 也是按那个错价算的（实测 `002812` 记 11.17，
/// 目标 11.5051 = +3%、止损 10.9466 = −2%），而该票真实收盘 48.27。
/// 只换入场价会让「目标价 < 入场价」⇒ 整行价格体系自相矛盾。
/// **一行价格体系错了，整行不可用**，只能剔除。
pub const PRICE_DIVERGENCE_TOLERANCE_PCT: f64 = 5.0;

/// 入场价相对锚定收盘价的偏离百分比（绝对值）。
///
/// 任一入参非有限正数时返回 `None`（**不返回 0**：0 会被读成「完全一致」，
/// 把「无法判定」伪装成「校验通过」）。
pub fn price_divergence_pct(pick_price: f64, anchor_close: f64) -> Option<f64> {
    if !pick_price.is_finite()
        || !anchor_close.is_finite()
        || pick_price <= 0.0
        || anchor_close <= 0.0
    {
        return None;
    }
    Some((pick_price / anchor_close - 1.0).abs() * 100.0)
}

/// 该 pick 的入场价是否与决策日真实收盘显著不符（即整行价格体系不可信）。
///
/// 无法判定（缺锚定价 / 非法价格）时返回 `false` —— 由调用方另行处理
/// 「拿不到锚定价」的情形，**不在此处把它当成错价**。
pub fn price_is_divergent(pick_price: f64, anchor_close: f64) -> bool {
    price_divergence_pct(pick_price, anchor_close)
        .is_some_and(|d| d > PRICE_DIVERGENCE_TOLERANCE_PCT)
}

pub fn compute_price_metrics(
    entry_price: f64,
    closes: &[f64],
    highs: &[f64],
    lows: &[f64],
    target_price: f64,
    stop_loss: f64,
) -> PriceMetrics {
    if closes.is_empty() || entry_price <= 0.0 {
        return PriceMetrics::default();
    }

    let t_plus_n_price = *closes.last().unwrap();

    let max_price = highs.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    let min_price = lows.iter().copied().fold(f64::INFINITY, f64::min);

    let max_return_pct = (max_price - entry_price) / entry_price * 100.0;

    // 修复 P0: max_drawdown 原误算为"最大亏损幅度"（从入场价到最低价的跌幅），
    // 正确定义是"从持有期间最高峰值到后续最低谷值的跌幅"。
    // 反例: entry=10, closes=[12,8,11]
    //   误算 = (8-10)/10*100 = -20%（从入场价到最低价）
    //   正确 = (12-8)/12*100 = 33.3%（从峰值 12 到谷值 8 的回撤）
    let mut peak = entry_price;
    let mut max_dd_pct = 0.0_f64;
    for &close in closes {
        if close > peak {
            peak = close;
        }
        if peak > 0.0 {
            let dd = (peak - close) / peak * 100.0;
            if dd > max_dd_pct {
                max_dd_pct = dd;
            }
        }
    }
    let max_drawdown_pct = max_dd_pct;
    let final_return_pct = (t_plus_n_price - entry_price) / entry_price * 100.0;

    let hit_stop_loss = min_price <= stop_loss;
    let hit_target = max_price >= target_price;

    PriceMetrics {
        t_plus_n_price: Some(t_plus_n_price),
        max_price: Some(max_price),
        min_price: Some(min_price),
        max_return_pct: Some(max_return_pct),
        max_drawdown_pct: Some(max_drawdown_pct),
        final_return_pct: Some(final_return_pct),
        hit_stop_loss: Some(if hit_stop_loss { 1 } else { 0 }),
        hit_target: Some(if hit_target { 1 } else { 0 }),
    }
}

/// 聚合一组 PickValidation 生成 hit_rate 报告
pub fn compute_hit_rate_report(validations: &[PickValidation]) -> HitRateReport {
    let mut by_action: HashMap<String, Vec<&PickValidation>> = HashMap::new();
    let mut by_style: HashMap<String, Vec<&PickValidation>> = HashMap::new();
    let mut by_t_plus_n: HashMap<String, Vec<&PickValidation>> = HashMap::new();

    for v in validations {
        by_action.entry(v.inferred_action.clone()).or_default().push(v);
        by_style.entry(v.style.clone()).or_default().push(v);
        by_t_plus_n.entry(v.t_plus_n.to_string()).or_default().push(v);
    }

    let action_stats = |group: &[&PickValidation]| -> ActionStats {
        let mut stats = ActionStats { total: group.len(), ..Default::default() };
        let mut returns: Vec<f64> = Vec::new();
        let mut net_returns: Vec<f64> = Vec::new();
        for v in group {
            match v.hit_outcome.as_deref() {
                Some("hit") => stats.hit += 1,
                Some("miss") => stats.miss += 1,
                Some("false_hit") => stats.false_hit += 1,
                Some("partial") => stats.partial += 1,
                _ => stats.insufficient += 1,
            }
            if let Some(ret) = v.final_return_pct {
                returns.push(ret);
            }
            // 净口径**不依赖落库的 `hit_outcome`**：历史行的 hit_outcome 是毛口径判的，
            // 而 `net_return_pct()` 恒可从毛收益现算 ⇒ 历史样本一并进净胜率，新旧同口径。
            if let Some(net) = v.net_return_pct() {
                net_returns.push(net);
            }
        }

        // legacy 口径：partial 不进分母（读数偏高，仅为历史可比而保留）
        let denom = stats.hit + stats.miss + stats.false_hit;
        stats.hit_rate = if denom > 0 {
            stats.hit as f64 / denom as f64
        } else {
            0.0
        };

        // strict 口径：partial 计输 —— 与 `AUDIT-stock-live-data-2026-10-24.md` §4.1
        // 的 42.5% 同一算法，避免「两套胜率各说各话」。
        let strict_denom = denom + stats.partial;
        stats.strict_denom = strict_denom;
        stats.strict_win_rate = if strict_denom > 0 {
            stats.hit as f64 / strict_denom as f64
        } else {
            0.0
        };

        if !returns.is_empty() {
            stats.avg_return_pct = returns.iter().sum::<f64>() / returns.len() as f64;
            let mut sorted = returns.clone();
            sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
            stats.median_return_pct = sorted[sorted.len() / 2];
        }

        // 净口径：唯一能回答「扣掉摩擦后赚不赚」的两个数
        if !net_returns.is_empty() {
            stats.net_avg_return_pct = net_returns.iter().sum::<f64>() / net_returns.len() as f64;
            let wins = net_returns.iter().filter(|n| **n > 0.0).count();
            stats.net_win_denom = net_returns.len();
            stats.net_win_rate = wins as f64 / net_returns.len() as f64;
        }
        stats
    };

    let mut report = HitRateReport {
        total: validations.len(),
        generated_at: chrono::Utc::now().to_rfc3339(),
        by_action: by_action.iter().map(|(k, v)| (k.clone(), action_stats(v))).collect(),
        by_style: by_style.iter().map(|(k, v)| (k.clone(), action_stats(v))).collect(),
        by_t_plus_n: by_t_plus_n.iter().map(|(k, v)| (k.clone(), action_stats(v))).collect(),
        factor_ic: HashMap::new(),
        factor_ic_ranked: Vec::new(),
        best_picks: Vec::new(),
        worst_picks: Vec::new(),
    };

    // 计算 9 因子 IC（Spearman 等级相关系数）
    report.factor_ic = compute_factor_ic(validations);
    let mut ranked: Vec<(String, f64)> =
        report.factor_ic.iter().map(|(k, v)| (k.clone(), *v)).collect();
    ranked.sort_by(|a, b| b.1.abs().partial_cmp(&a.1.abs()).unwrap_or(std::cmp::Ordering::Equal));
    report.factor_ic_ranked = ranked;

    // 排序 best / worst picks（按 final_return_pct 降序 / 升序）
    let mut sorted: Vec<PickValidation> =
        validations.iter().filter(|v| v.final_return_pct.is_some()).cloned().collect();
    sorted.sort_by(|a, b| {
        b.final_return_pct
            .unwrap_or(0.0)
            .partial_cmp(&a.final_return_pct.unwrap_or(0.0))
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    report.best_picks = sorted.iter().take(5).cloned().collect();
    sorted.reverse();
    report.worst_picks = sorted.iter().take(5).cloned().collect();

    report
}

/// 档位 → 该档的**验证窗口**（交易日）。
///
/// 与 `Period::default_holding_days()` 同口径，但本模块只拿得到落库的 period 字符串，
/// 不把 harness 的枚举拖进纯聚合层；未知写法返回 `None`（该行不参与期窗匹配）。
pub fn period_expected_window(period: &str) -> Option<i32> {
    match period {
        "ultra_short" | "ultrashort" => Some(2),
        "short" => Some(5),
        "mid" => Some(28),
        "long" => Some(90),
        _ => None,
    }
}

/// **期窗匹配**的命中率报告：每档只计它自己的持有窗口，不把 2/5/28/90 天混成一个分母。
///
/// 为什么需要它（2026-10-24 生产实证）：`compute_hit_rate_report` 的 by_action /
/// by_style 是跨窗口池化的；同一批 short 票既有 T+2 也有 T+28/T+90 的验证行，
/// 池化后 `capital_short` 的 +3.41%（T+5）被 T+28 的 -3.59%、T+90 的 -8.18% 拉成
/// 负值，面板看起来「策略全面亏损」，而按档位自己的窗口看它是正期望。
/// 反过来，`reversion_short` 的 T+2/T+5 全 partial（≈0 收益）也会被短窗掩盖。
///
/// 口径：
/// - `by_action` / `by_style` / `total` / best / worst 只含 `t_plus_n == 本档窗口` 的行；
/// - `by_t_plus_n` 仍按窗口分组（保留跨窗口原始读数）；
/// - period 无法识别的行**不猜窗口**，原样保留（否则会把未知档直接丢掉）；
/// - 匹配后为空时回退到原报告（不把「没有可匹配窗口」演成「没有样本」）。
pub fn compute_period_matched_report(validations: &[PickValidation]) -> HitRateReport {
    let matched: Vec<PickValidation> = validations
        .iter()
        .filter(|v| match period_expected_window(&v.period) {
            Some(window) => v.t_plus_n == window,
            None => true,
        })
        .cloned()
        .collect();
    if matched.is_empty() {
        compute_hit_rate_report(validations)
    } else {
        compute_hit_rate_report(&matched)
    }
}

/// 计算每个因子 vs 实际收益的 Spearman 等级相关系数（IC）
///
/// IC 越接近 0 → 该因子对未来收益无预测力
/// IC > 0.1 → 有正向预测力，应增加该因子权重
/// IC < -0.05 → 反向预测，应反向使用或减权
///
/// ## 算法
/// 对每个因子（factor_id）：
/// 1. 收集所有验证样本的 (factor_value, final_return_pct) 对
/// 2. 各自排序得等级
/// 3. Spearman = Pearson(等级_x, 等级_y)
pub fn compute_factor_ic(validations: &[PickValidation]) -> HashMap<String, f64> {
    let mut factor_to_pairs: HashMap<String, Vec<(f64, f64)>> = HashMap::new();

    for v in validations {
        let Some(ret) = v.final_return_pct else { continue };
        let Some(ref factors) = v.factor_snapshot else { continue };
        for (factor_id, factor_val) in factors {
            factor_to_pairs.entry(factor_id.clone()).or_default().push((*factor_val, ret));
        }
    }

    let mut ics = HashMap::new();
    for (factor_id, pairs) in factor_to_pairs {
        if pairs.len() < 5 {
            // 样本太少，IC 不可靠
            ics.insert(factor_id, 0.0);
            continue;
        }
        // 退化（一侧秩无方差 ⇒ ρ 无定义）沿用本模块既有口径：记 0.0 = 无预测力。
        // 与 `reflection_stats` 的 IC 表不同——那里把缺席分状态报出来，因为它是
        // **给人读的观测面**；本表的消费方是权重调整，只消费数值。
        ics.insert(factor_id, axagent_harness::indicators::spearman_rank_ic(&pairs).unwrap_or(0.0));
    }
    ics
}

// ── 工具函数：把 RecoPick 转 PickValidation ──

/// 把 hit_outcome 归一化为决策级二进制 outcome（"win" / "loss"），供决策记录回填。
///
/// ## 口径（这是**方向判定**，不是盈利判定）
/// - `hit` / `partial` → `win`
/// - `miss` / `false_hit` → `loss`
/// - `insufficient` / `None` → 数据不足不判定 → `None`（跳过）
///
/// B3「决策-结果闭环」要求回填口径与离线回测口径一致：回填决策记录（stock_analyses.outcome）
/// 与计算命中率必须复用同一套判定，避免两处各自实现导致口径漂移（如一侧把 partial 算 hit、
/// 另一侧不算）。本函数是二者共用的唯一判定源。
///
/// ## ⚠ 与 `ActionStats::strict_win_rate` 的关系（2026-10-25 G4 澄清）
///
/// 下面三个出口**刻意不同**，不是口径漂移 —— 它们回答的不是同一个问题：
///
/// | 出口 | 回答的问题 | `partial`（-5%~0 微亏）归属 |
/// |---|---|---|
/// | **本函数** → `stock_analyses.outcome` | 方向判对了吗 | **win**（微亏但仍看对方向） |
/// | `ActionStats::strict_win_rate` | 按这个判断会不会亏 | **输**（partial 计输） |
/// | `ActionStats::net_win_rate` | 扣成本后赚没赚 | 由净收益符号决定 |
///
/// 把本函数改成 `partial → loss` 会让「方向对了、只是成本吃掉利润」被记成决策失败，
/// 污染的是**决策质量**归因；而 strict / net 只关心**钱**。两件事不可合并。
pub fn hit_outcome_to_binary_outcome(hit_outcome: Option<&str>) -> Option<&'static str> {
    match hit_outcome {
        Some("hit") | Some("partial") => Some("win"),
        Some("miss") | Some("false_hit") => Some("loss"),
        _ => None,
    }
}

/// 从 RecoPick + 已拉取的 T+N K 线数据构建 PickValidation
pub fn build_pick_validation(
    pick: &RecoPick,
    pick_id: &str,
    t_plus_n: i32,
    closes: &[f64],
    highs: &[f64],
    lows: &[f64],
    data_source: &str,
) -> PickValidation {
    let action = infer_action(pick.price, pick.target_price).to_string();
    let m =
        compute_price_metrics(pick.price, closes, highs, lows, pick.target_price, pick.stop_loss);
    // G4（2026-10-25）：判定用**净**收益。毛 +0.10% 的样本实盘是净亏 0.15%，
    //   不得记命中 —— 此前传 `m.final_return_pct`（毛）导致胜率系统性虚高。
    //   `hit_stop_loss` / `hit_target` 仍由价格触发，与成本无关，原样保留。
    let net_return_pct = m.net_return_pct_default();
    let hit_outcome = compute_hit_outcome(&action, net_return_pct, m.hit_stop_loss, m.hit_target);

    PickValidation {
        pick_id: pick_id.to_string(),
        stock_code: pick.stock_code.clone(),
        stock_name: pick.stock_name.clone(),
        style: format!("{:?}", pick.style).to_lowercase(),
        period: format!("{:?}", pick.period).to_lowercase(),
        generated_at: chrono::Utc::now().to_rfc3339(),
        t_plus_n,
        entry_price: pick.price,
        target_price: pick.target_price,
        stop_loss: pick.stop_loss,
        position_pct: pick.position_pct,
        confidence: pick.confidence as i32,
        inferred_action: action,
        // 代际由调用方（知道这条 pick 来自实盘还是哪次回放的）事后赋值，见
        // `backtest_validation::fetch_and_validate`
        as_of_date: None,
        t_plus_n_price: m.t_plus_n_price,
        max_price: m.max_price,
        min_price: m.min_price,
        max_return_pct: m.max_return_pct,
        max_drawdown_pct: m.max_drawdown_pct,
        final_return_pct: m.final_return_pct,
        hit_stop_loss: m.hit_stop_loss,
        hit_target: m.hit_target,
        hit_outcome,
        factor_snapshot: None, // TODO: 从 portfolio-mgr 节点结果中提取
        data_source: net_cost_data_source(data_source),
    }
}

/// 在 `data_source` 上追加「`hit_outcome` 按净收益判定」的标记（**幂等**）。
///
/// ## 为什么必须标
///
/// `final_return_pct` 落的是**毛**收益（列语义刻意未改，历史行才可比），
/// 而 `hit_outcome` 自 2026-10-25（G4）起按**净**收益判定 ⇒ **同一行里两列口径不同**。
/// 不加标记就无法区分「毛判定的历史行」与「净判定的新行」，
/// 又会退回本次要修的形态：两套口径混在同一个分母里，且无人知道该看哪个。
///
/// 幂等：`refresh_miss_seed` 那类流程会把 `data_source` 传进来再追加，
/// 重复调用不得产生 `+net_cost+net_cost`。
fn net_cost_data_source(base: &str) -> String {
    const MARK: &str = "+net_cost";
    if base.contains(MARK) {
        base.to_string()
    } else {
        format!("{base}{MARK}")
    }
}

// ── 单元测试 ──

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_infer_action_buy() {
        assert_eq!(infer_action(10.0, 12.0), "buy"); // 20% 上涨
        assert_eq!(infer_action(10.0, 10.5), "buy"); // 5% 上涨
    }

    #[test]
    fn test_infer_action_sell() {
        assert_eq!(infer_action(10.0, 8.0), "sell");
        assert_eq!(infer_action(10.0, 9.0), "sell");
    }

    #[test]
    fn test_infer_action_hold() {
        assert_eq!(infer_action(10.0, 10.0), "hold");
        assert_eq!(infer_action(10.0, 10.05), "hold"); // 0.5% < 1%
        assert_eq!(infer_action(0.0, 10.0), "hold"); // 边界
    }

    #[test]
    fn test_hit_outcome_hold_narrow_range() {
        // 修复 P1: hold 窄幅震荡判定
        assert_eq!(compute_hit_outcome("hold", Some(0.5), None, None), Some("hit".to_string()));
        assert_eq!(compute_hit_outcome("hold", Some(-0.8), None, None), Some("hit".to_string()));
        assert_eq!(compute_hit_outcome("hold", Some(3.0), None, None), Some("partial".to_string()));
        assert_eq!(
            compute_hit_outcome("hold", Some(6.0), None, None),
            Some("false_hit".to_string())
        );
    }

    #[test]
    fn test_hit_outcome_buy_hit() {
        assert_eq!(compute_hit_outcome("buy", Some(2.0), None, None), Some("hit".to_string()));
        assert_eq!(compute_hit_outcome("buy", Some(0.5), None, Some(1)), Some("hit".to_string()));
    }

    #[test]
    fn test_hit_outcome_buy_false_hit() {
        assert_eq!(
            compute_hit_outcome("buy", Some(-7.0), None, None),
            Some("false_hit".to_string())
        );
    }

    #[test]
    fn test_hit_outcome_buy_miss_stop_loss() {
        assert_eq!(compute_hit_outcome("buy", Some(-3.0), Some(1), None), Some("miss".to_string()));
    }

    #[test]
    fn test_hit_outcome_sell_hit() {
        assert_eq!(compute_hit_outcome("sell", Some(-2.0), None, None), Some("hit".to_string()));
    }

    #[test]
    fn test_hit_outcome_sell_false_hit() {
        assert_eq!(
            compute_hit_outcome("sell", Some(8.0), None, None),
            Some("false_hit".to_string())
        );
    }

    #[test]
    fn test_hit_outcome_insufficient() {
        assert_eq!(compute_hit_outcome("buy", None, None, None), Some("insufficient".to_string()));
    }

    #[test]
    fn test_compute_price_metrics_uptrend() {
        let closes = vec![10.5, 11.0, 11.5, 12.0];
        let highs = vec![10.8, 11.2, 11.7, 12.2];
        let lows = vec![10.3, 10.9, 11.3, 11.8];
        let m = compute_price_metrics(10.0, &closes, &highs, &lows, 12.0, 9.5);
        assert_eq!(m.t_plus_n_price, Some(12.0));
        assert_eq!(m.max_price, Some(12.2));
        assert_eq!(m.min_price, Some(10.3));
        assert!((m.max_return_pct.unwrap() - 22.0).abs() < 1e-6);
        // 纯上行（closes 严格递增）无回撤，peak-to-trough 定义为 0
        assert!((m.max_drawdown_pct.unwrap() - 0.0).abs() < 1e-6);
        assert!((m.final_return_pct.unwrap() - 20.0).abs() < 1e-6);
        assert_eq!(m.hit_target, Some(1));
    }

    #[test]
    fn test_compute_price_metrics_drawdown_peak_to_trough() {
        // 验证 P0 修复：最大回撤 = 持有期间峰值到后续谷值的跌幅，
        // 而非「从入场价到最低价的跌幅」。
        // entry=10, closes=[12,8,11]:
        //   正确 = (12-8)/12*100 ≈ 33.333%（峰值 12 → 谷值 8）
        //   旧误算 = (10-8)/10*100 = 20%（从入场价到谷值，漏算入场后涨幅）
        let closes = vec![12.0, 8.0, 11.0];
        let highs = vec![12.0, 8.0, 11.0];
        let lows = vec![12.0, 8.0, 11.0];
        let m = compute_price_metrics(10.0, &closes, &highs, &lows, 20.0, 5.0);
        assert!((m.max_drawdown_pct.unwrap() - 100.0 * (12.0 - 8.0) / 12.0).abs() < 1e-6);
        // 回归守卫：绝不能回到旧实现的 20.0
        assert!((m.max_drawdown_pct.unwrap() - 20.0).abs() >= 1e-6);
    }

    #[test]
    fn test_compute_price_metrics_stop_loss_hit() {
        let closes = vec![9.8, 9.5, 9.2];
        let highs = vec![10.0, 9.7, 9.4];
        let lows = vec![9.5, 9.3, 9.0]; // 9.0 < stop_loss 9.5
        let m = compute_price_metrics(10.0, &closes, &highs, &lows, 12.0, 9.5);
        assert_eq!(m.hit_stop_loss, Some(1));
    }

    #[test]
    fn test_factor_ic_positive() {
        // 完美正相关：因子值越大，收益越大
        let mut validations = Vec::new();
        for (f, r) in [(0.1, -5.0), (0.3, -2.0), (0.5, 1.0), (0.7, 3.0), (0.9, 6.0)] {
            let mut factors = HashMap::new();
            factors.insert("f1_technical".to_string(), f);
            validations.push(PickValidation {
                pick_id: "p".into(),
                stock_code: "x".into(),
                stock_name: "x".into(),
                style: "trend".into(),
                period: "short".into(),
                generated_at: "t".into(),
                t_plus_n: 20,
                entry_price: 10.0,
                target_price: 12.0,
                stop_loss: 9.0,
                position_pct: 10.0,
                confidence: 50,
                inferred_action: "buy".into(),
                t_plus_n_price: Some(10.0 + r / 10.0),
                max_price: None,
                min_price: None,
                max_return_pct: None,
                max_drawdown_pct: None,
                final_return_pct: Some(r),
                hit_stop_loss: None,
                hit_target: None,
                hit_outcome: None,
                factor_snapshot: Some(factors),
                data_source: "test".into(),
                as_of_date: None,
            });
        }
        let ic = compute_factor_ic(&validations);
        let v = ic.get("f1_technical").copied().unwrap_or(0.0);
        assert!(v > 0.9, "expected strong positive IC, got {v}");
    }

    #[test]
    fn test_factor_ic_no_correlation() {
        let mut validations = Vec::new();
        // 因子值 vs 收益完全无关
        for (f, r) in [(0.1, 1.0), (0.3, -1.0), (0.5, 1.0), (0.7, -1.0), (0.9, 1.0)] {
            let mut factors = HashMap::new();
            factors.insert("f1".to_string(), f);
            validations.push(PickValidation {
                pick_id: "p".into(),
                stock_code: "x".into(),
                stock_name: "x".into(),
                style: "trend".into(),
                period: "short".into(),
                generated_at: "t".into(),
                t_plus_n: 20,
                entry_price: 10.0,
                target_price: 12.0,
                stop_loss: 9.0,
                position_pct: 10.0,
                confidence: 50,
                inferred_action: "buy".into(),
                t_plus_n_price: None,
                max_price: None,
                min_price: None,
                max_return_pct: None,
                max_drawdown_pct: None,
                final_return_pct: Some(r),
                hit_stop_loss: None,
                hit_target: None,
                hit_outcome: None,
                factor_snapshot: Some(factors),
                data_source: "test".into(),
                as_of_date: None,
            });
        }
        let ic = compute_factor_ic(&validations);
        let v = ic.get("f1").copied().unwrap_or(0.0);
        assert!(v.abs() < 0.3, "expected low IC for random data, got {v}");
    }

    #[test]
    fn test_compute_hit_rate_report_basic() {
        let make = |outcome: &str, ret: f64| PickValidation {
            pick_id: "p".into(),
            stock_code: "x".into(),
            stock_name: "x".into(),
            style: "trend".into(),
            period: "short".into(),
            generated_at: "t".into(),
            t_plus_n: 5,
            entry_price: 10.0,
            target_price: 12.0,
            stop_loss: 9.0,
            position_pct: 10.0,
            confidence: 50,
            inferred_action: "buy".into(),
            t_plus_n_price: Some(10.0 + ret / 100.0),
            max_price: None,
            min_price: None,
            max_return_pct: None,
            max_drawdown_pct: None,
            final_return_pct: Some(ret),
            hit_stop_loss: None,
            hit_target: None,
            hit_outcome: Some(outcome.into()),
            factor_snapshot: None,
            data_source: "test".into(),
            as_of_date: None,
        };
        let validations = vec![
            make("hit", 3.0),
            make("hit", 5.0),
            make("miss", -2.0),
            make("false_hit", -8.0),
            make("partial", -1.0),
        ];
        let report = compute_hit_rate_report(&validations);
        assert_eq!(report.total, 5);
        let buy_stats = report.by_action.get("buy").unwrap();
        assert_eq!(buy_stats.hit, 2);
        assert_eq!(buy_stats.miss, 1);
        assert_eq!(buy_stats.false_hit, 1);
        // hit_rate = 2 / (2+1+1) = 0.5
        assert!((buy_stats.hit_rate - 0.5).abs() < 1e-9);
    }

    #[test]
    fn period_matched_report_excludes_other_windows() {
        let make = |period: &str, t_plus_n: i32, outcome: &str, ret: f64| PickValidation {
            pick_id: format!("p-{period}-{t_plus_n}"),
            stock_code: "x".into(),
            stock_name: "x".into(),
            style: format!("{period}_style"),
            period: period.into(),
            generated_at: "t".into(),
            t_plus_n,
            entry_price: 10.0,
            target_price: 12.0,
            stop_loss: 9.0,
            position_pct: 10.0,
            confidence: 50,
            inferred_action: "buy".into(),
            t_plus_n_price: Some(10.0 + ret / 10.0),
            max_price: None,
            min_price: None,
            max_return_pct: None,
            max_drawdown_pct: None,
            final_return_pct: Some(ret),
            hit_stop_loss: None,
            hit_target: None,
            hit_outcome: Some(outcome.into()),
            factor_snapshot: None,
            data_source: "test".into(),
            as_of_date: None,
        };
        let validations = vec![
            make("short", 5, "hit", 3.0),
            // 同一批 short 的 T+28：不是本档窗口，必须被期窗匹配挡在分母外
            make("short", 28, "miss", -10.0),
            make("mid", 28, "hit", 5.0),
            make("long", 90, "miss", -6.0),
        ];
        let report = compute_period_matched_report(&validations);
        assert_eq!(report.total, 3, "只保留 short@5 / mid@28 / long@90: {report:?}");
        assert_eq!(report.by_style.get("short_style").unwrap().total, 1);
        assert!(!report.by_t_plus_n.contains_key("5") || report.by_t_plus_n["5"].total == 1);
        // 匹配后 short 的均值只含 +3%，不应被 T+28 的 -10% 拉负。
        assert!(report.by_style["short_style"].avg_return_pct > 0.0);
    }

    #[test]
    fn period_matched_report_falls_back_when_no_window_matches() {
        let v = PickValidation {
            pick_id: "p".into(),
            stock_code: "x".into(),
            stock_name: "x".into(),
            style: "trend".into(),
            period: "short".into(),
            generated_at: "t".into(),
            t_plus_n: 20, // 旧窗口，不匹配 short=5
            entry_price: 10.0,
            target_price: 12.0,
            stop_loss: 9.0,
            position_pct: 10.0,
            confidence: 50,
            inferred_action: "buy".into(),
            t_plus_n_price: Some(10.5),
            max_price: None,
            min_price: None,
            max_return_pct: None,
            max_drawdown_pct: None,
            final_return_pct: Some(5.0),
            hit_stop_loss: None,
            hit_target: None,
            hit_outcome: Some("hit".into()),
            factor_snapshot: None,
            data_source: "test".into(),
            as_of_date: None,
        };
        let report = compute_period_matched_report(&[v]);
        assert_eq!(report.total, 1, "没有可匹配窗口时回退原样本，不得变 0 分母");
    }

    #[test]
    fn hit_outcome_to_binary_outcome_matches_hit_rate_denominator() {
        // B3 口径守卫：回填决策记录的 win/loss 判定，必须与 compute_hit_rate_report
        // 统计命中率所用的分母口径一致 —— 只有 miss/false_hit 会降低命中率分子差，
        // 而 insufficient 不参与判定（None 跳过）。
        // hit/partial → win（计入 hit 或 partial，均视为决策正确侧）
        assert_eq!(hit_outcome_to_binary_outcome(Some("hit")), Some("win"));
        assert_eq!(hit_outcome_to_binary_outcome(Some("partial")), Some("win"));
        // miss/false_hit → loss
        assert_eq!(hit_outcome_to_binary_outcome(Some("miss")), Some("loss"));
        assert_eq!(hit_outcome_to_binary_outcome(Some("false_hit")), Some("loss"));
        // insufficient / None → 数据不足不判定
        assert_eq!(hit_outcome_to_binary_outcome(Some("insufficient")), None);
        assert_eq!(hit_outcome_to_binary_outcome(None), None);
        assert_eq!(hit_outcome_to_binary_outcome(Some("unknown")), None);
    }

    // ── G4（2026-10-25）：成本口径 ──────────────────────────────────
    //
    // 这一组测试锁的是「尺子」本身。它们故意不依赖 `build_pick_validation`
    // （要构造 RecoPick），而是直接验证「毛收益 → 净收益 → 判定」的链条，
    // 因为那正是出过错的地方。

    /// 构造一条最小可用样本（G4 新增测试共用）。
    fn mk(outcome: &str, gross_ret: f64) -> PickValidation {
        PickValidation {
            pick_id: "p".into(),
            stock_code: "x".into(),
            stock_name: "x".into(),
            style: "trend".into(),
            period: "short".into(),
            generated_at: "t".into(),
            t_plus_n: 5,
            entry_price: 10.0,
            target_price: 12.0,
            stop_loss: 9.0,
            position_pct: 10.0,
            confidence: 50,
            inferred_action: "buy".into(),
            t_plus_n_price: Some(10.0 * (1.0 + gross_ret / 100.0)),
            max_price: None,
            min_price: None,
            max_return_pct: None,
            max_drawdown_pct: None,
            final_return_pct: Some(gross_ret),
            hit_stop_loss: None,
            hit_target: None,
            hit_outcome: Some(outcome.into()),
            factor_snapshot: None,
            data_source: "test".into(),
            as_of_date: None,
        }
    }

    /// **核心负控**：毛收益 +0.10% 的样本实盘是净亏，不得被判成 `hit`。
    ///
    /// 这是本次修复要堵的那个洞 —— 修复前 `compute_hit_outcome` 吃的是毛收益，
    /// `+0.10% > 0` 直接记命中，而实盘扣 0.25% 后是 −0.15%。
    #[test]
    fn tiny_gross_gain_is_not_a_hit_after_cost() {
        let px = [10.01]; // +0.10%
        let m = compute_price_metrics(10.0, &px, &px, &px, 99.0, 1.0);

        // 先证明输入有效：毛收益确实是正的、且是微利。
        // （否则「不是 hit」可能只是因为收益本来就是负的 —— 那是假绿。）
        let gross = m.final_return_pct.expect("应有毛收益");
        assert!(gross > 0.0, "毛收益应为正，实际 {gross}");
        assert!(gross < 0.5, "毛收益应为微利（<0.5%），实际 {gross}");

        // 净收益必须为负：成本 0.25% > 微利 0.10%
        let net = m.net_return_pct_default().expect("应有净收益");
        assert!(net < 0.0, "毛 +{gross:.2}% 扣成本后应为负，实际 {net}");

        // 目标价 99 / 止损 1 都不会触发 ⇒ 判定只由收益决定
        assert_eq!(m.hit_target, Some(0));
        assert_eq!(m.hit_stop_loss, Some(0));

        // 净口径：不得是 hit
        assert_ne!(
            compute_hit_outcome("buy", Some(net), m.hit_stop_loss, m.hit_target),
            Some("hit".to_string()),
            "毛 +0.10% 在实盘净亏，不能记命中"
        );

        // 反向对照：若错误地传毛收益就会被记成 hit —— 证明上一条断言真有区分力
        assert_eq!(
            compute_hit_outcome("buy", Some(gross), m.hit_stop_loss, m.hit_target),
            Some("hit".to_string()),
            "传毛收益会误判命中（这正是修复前的行为）"
        );
    }

    /// `strict_win_rate` 把 `partial` 计输，与 `hit_rate`（不计输）必须明确区分；
    /// strict 口径与 `AUDIT-stock-live-data-2026-10-24.md` §4.1 的 42.5% 同一算法。
    #[test]
    fn strict_win_rate_counts_partial_as_loss() {
        let validations = vec![
            mk("hit", 3.0),
            mk("hit", 5.0),
            mk("miss", -2.0),
            mk("false_hit", -8.0),
            mk("partial", -1.0),
            mk("partial", -2.0),
        ];
        let report = compute_hit_rate_report(&validations);
        let s = report.by_action.get("buy").unwrap();

        // legacy：2 / (2+1+1) = 0.5
        assert!((s.hit_rate - 0.5).abs() < 1e-9, "legacy hit_rate={}", s.hit_rate);
        // strict：2 / (2+1+1+2) = 1/3
        assert_eq!(s.strict_denom, 6);
        assert!(
            (s.strict_win_rate - 2.0 / 6.0).abs() < 1e-9,
            "strict_win_rate={}",
            s.strict_win_rate
        );
        // 两者必须真的不同 —— 相等说明 partial 没进 strict 分母
        assert!((s.hit_rate - s.strict_win_rate).abs() > 1e-6);
    }

    /// 净胜率用「扣成本后 > 0」作分界，比毛口径更严。
    #[test]
    fn net_win_rate_uses_cost_adjusted_threshold() {
        let validations = vec![
            mk("hit", 0.10), // 毛赚 → 净亏（成本 0.25%）
            mk("hit", 3.0),  // 净赚
            mk("partial", -1.0),
            mk("miss", -0.20), // 毛亏 → 净亏
        ];
        let report = compute_hit_rate_report(&validations);
        let s = report.by_action.get("buy").unwrap();

        assert_eq!(s.net_win_denom, 4, "四条都有收益，全部进净分母");
        // 只有 +3.0% 扣成本后仍为正
        assert!((s.net_win_rate - 0.25).abs() < 1e-9, "净胜率应为 1/4，实际 {}", s.net_win_rate);

        // 毛口径下 +0.10% 也算赚 ⇒ 2/4，证明净口径确实更严（非等价改写）
        let gross_wins = validations
            .iter()
            .filter(|v| v.final_return_pct.map(|r| r > 0.0).unwrap_or(false))
            .count();
        assert_eq!(gross_wins, 2);
        assert!(s.net_win_rate < gross_wins as f64 / 4.0);
    }

    /// 无收益的样本不得被读成「0% 胜率」—— 分母必须为 0，让调用方能区分
    /// 「一条没赢」与「没有样本」。
    ///
    /// ## 构造说明（首版写错在这里）
    ///
    /// 首版用 `mk("hit", 1.0)` 再把收益置 None —— 那构造出的是**现实中不可能**的状态：
    /// `hit` 只可能由"有收益且为正"派生。真实缺行情时 `hit_outcome` 是 `insufficient`。
    /// 两个分母的口径本来就不同：
    ///
    /// - `strict_denom` 吃落库的 `hit_outcome`（方向判定）；
    /// - `net_win_denom` 吃 `final_return_pct`（有无收益）。
    ///
    /// 正常数据下同源（`hit_outcome` 由收益派生），但**残缺行会分叉** ——
    /// 所以要分别断言，不能假定二者恒等。
    #[test]
    fn no_net_samples_yield_zero_denominator_not_zero_win_rate() {
        let mut v = mk("insufficient", 1.0);
        v.final_return_pct = None;
        let report = compute_hit_rate_report(&[v]);
        let s = report.by_action.get("buy").unwrap();

        // 无收益 ⇒ 净口径无样本
        assert_eq!(s.net_win_denom, 0);
        assert_eq!(s.net_win_rate, 0.0);
        // insufficient 不进方向分母
        assert_eq!(s.strict_denom, 0);
        assert_eq!(s.strict_win_rate, 0.0);
        assert_eq!(s.insufficient, 1);
    }

    /// 负控：**有** `hit_outcome` 但**无**收益时，两个分母会分叉 ——
    /// 这条锁住上一条测试注释里声明的差异，防止日后有人把它们当同一个数。
    #[test]
    fn strict_and_net_denominators_diverge_on_incomplete_rows() {
        let mut v = mk("hit", 1.0);
        v.final_return_pct = None; // 落库 hit 标签在，但收益列缺失
        let report = compute_hit_rate_report(&[v]);
        let s = report.by_action.get("buy").unwrap();

        assert_eq!(s.strict_denom, 1, "方向分母看 hit_outcome");
        assert_eq!(s.net_win_denom, 0, "净分母看 final_return_pct");
        assert_ne!(s.strict_denom, s.net_win_denom, "两者本就不同源，不得假定恒等");
    }

    /// `+net_cost` 标记必须幂等 —— 重复追加会让同一行出现 `+net_cost+net_cost`，
    /// 日后按标记筛样就会漏掉。
    #[test]
    fn net_cost_marker_is_idempotent() {
        assert_eq!(net_cost_data_source("akshare_daily"), "akshare_daily+net_cost");
        assert_eq!(net_cost_data_source("akshare_daily+net_cost"), "akshare_daily+net_cost");
        // 与既有 `+premature_window` 这类标记可共存
        assert_eq!(
            net_cost_data_source("akshare_daily+premature_window"),
            "akshare_daily+premature_window+net_cost"
        );
    }

    // ── R-3a 价源平价校验（2026-10-25）──

    /// **核心正控**：实录的错价样本必须被判为偏离。
    /// `000779` 记 1.63，决策日真实收盘 10.60 —— 偏离 84.6%，而这个「1.63」
    /// 恰好是 `000980` 当日收盘（批量取价行错位的实证）。
    #[test]
    fn real_misaligned_price_is_detected() {
        let dev = price_divergence_pct(1.63, 10.60).expect("两个正价应可算");
        assert!((dev - 84.62).abs() < 0.05, "实测偏离 {dev}");
        assert!(price_is_divergent(1.63, 10.60), "84.6% 必须超阈");
        // 反向同样算偏离（错成高价也是错）
        assert!(price_is_divergent(48.27, 11.17), "偏高 332% 同样必须被判偏离");
    }

    /// **核心负控**：实盘实测最大偏离仅 4.4%（311 条里 0 条 ≥5%），
    /// 阈值不得误杀这类正常噪声，否则会把干净样本剔光。
    #[test]
    fn live_noise_level_is_not_flagged() {
        // 4.4% 是实盘 311 条的实测最大偏离
        assert!(!price_is_divergent(104.40, 100.0), "4.4% 属正常噪声，不得判偏离");
        assert!(!price_is_divergent(100.0, 104.40), "反向 4.2% 同样不得判偏离");
        assert!(!price_is_divergent(102.0, 100.0), "2% 必须放行");
        // ⚠ 刻意**不**断言「恰等于 5%」：`105.0/100.0` 的浮点结果是
        // `5.000000000000004`，边界落在哪一侧取决于浮点路径 —— 锁死它等于锁死一个
        // 偶然实现，日后换写法就会假红。真正要锁的是明确两侧。
        assert!(!price_is_divergent(104.99, 100.0), "4.99% 低于阈值，不得判偏离");
        assert!(price_is_divergent(105.01, 100.0), "5.01% 高于阈值，必须判偏离");
    }

    /// 无法判定的情形必须返回 `None`，**不得返回 `Some(0.0)`** ——
    /// 0 会被读成「完全一致」，把「没基准」伪装成「校验通过」。
    #[test]
    fn undeterminable_divergence_is_none_not_zero() {
        assert_eq!(price_divergence_pct(0.0, 10.0), None, "入场价 0 不可判定");
        assert_eq!(price_divergence_pct(10.0, 0.0), None, "缺锚定价不可判定");
        assert_eq!(price_divergence_pct(f64::NAN, 10.0), None, "NaN 不可判定");
        assert_eq!(price_divergence_pct(10.0, f64::INFINITY), None, "INF 不可判定");
        // 无法判定 ⇒ 不判偏离（由调用方另路处理）
        assert!(!price_is_divergent(0.0, 10.0));
        assert!(!price_is_divergent(10.0, 0.0));
    }
}
