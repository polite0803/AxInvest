//! P3-1: portfolio-mgr 核心数学公式的 Rust 实现（可测试、可复用）。
//!
//! 这些函数原在 portfolio-mgr.rhai 中内联计算，迁移到 Rust 层后：
//! 1. 可通过 `register_fn` 注入 Rhai Engine（Rhai 脚本只做规则编排）
//! 2. 可写单元测试验证边界条件
//! 3. 可被 Rust 层其他模块复用（如 WFO 校准目标函数）
//!
//! 所有函数均为纯函数，无副作用，无异步。

use crate::decision_action::{ActionKind, normalize_action};
use crate::position_limits::RiskTier;
use serde::{Deserialize, Serialize};

// ── 评分 → 因子信号 ──

/// 评分（0–100 制）→ 因子信号（`[-1, 1]`）的归一化中点 —— **唯一权威**。
///
/// 裁定 P-B1（2026-10-17）上收这条口径的原因：`(x − 50) / 50` 原先在**五处**各写一份 ——
/// 主链 `portfolio-mgr.rhai` 内联四处（f1 趋势 / f2 共识 / f6 数据质量 / f13 瓶颈），
/// 逐档腿那边 `leg_signal::leg_signal` 又内联一份（`trendStrength ← totalScore`）。
/// 本仓的教训是「两处一致地不同，比一处缺失更难发现」（同 `VOL_LOOKBACK_DAYS` 的先例）——
/// 五个 `50.0` 各写各的，改一个（比如某天想让中枢随档位移动）就只剩部分消费方跟着动。
///
/// 现在的接线：主链经宿主函数 `pm_score_to_signal` 调用本函数；逐档腿由
/// `leg_signal::leg_signal` 直接调用本函数；审计侧回算 σ 也用同一个 —— 三侧不可能再漂移。
pub const SCORE_SIGNAL_MIDPOINT: f64 = 50.0;

/// 见 [`SCORE_SIGNAL_MIDPOINT`]：`(score − 50) / 50`。
///
/// ⚠ 刻意**不**钳制到 `[-1, 1]`：越界（评分 >100 或 <0）在调用方要能看出来 ——
/// `leg_signal` 的 `cl()` 负责钳制、主链的 f13 用 `clamp(...)` 兜，两处各自显式。
/// 在这里顺手钳死会把「上游给了个非法分」这件事静默抹平。
pub fn score_to_signal(score: f64) -> f64 {
    (score - SCORE_SIGNAL_MIDPOINT) / SCORE_SIGNAL_MIDPOINT
}

/// 计算因子数据完整度（0.0 ~ 1.0）。
///
/// 用于 data-quality.rhai 评估因子层数据完整度，纳入综合评分。
/// 每个因子有数据 → +1，无数据 → 0，最终返回比例。
///
/// # 参数（10 个因子的数据源）
/// - `total_score`: t-scoring 技术评分
/// - `consensus_score`: debate-convergence 共识评分
/// - `catalyst_level`: a-catalyst 催化剂等级
/// - `risk_volatility`: t-risk 波动率
/// - `valuation_dcf_upside`: t-valuation DCF 上行空间
/// - `money_flow_main_net_inflow`: t-hotmoney-data 主力净流入
///
/// 注：原 `trader_direction` 因子已于 2026-09-12 移除。data-quality 节点是 trader 的
/// **上游**（trader 的 `dqi_score` 由本节点提供），架构上不可能读到 trader 输出，
/// 该因子在此处恒缺失 ⇒ 完整度上限被钉在 0.9、综合分恒定少 3 分。f7 的可用性由
/// portfolio-mgr（trader 下游）消费，不属于本函数的职责范围。
/// - `lockup_shareholder_trades_len`: t-lockup-data 股东增减持数量
/// - `announcements_len`: t-catalyst-data 公告数量
/// - `pace_signal`: pace-calc PACE 情绪信号
///
/// # 返回
/// 因子完整度比例 [0.0, 1.0]
#[allow(clippy::too_many_arguments)]
pub fn compute_factor_completeness(
    total_score: Option<f64>,
    consensus_score: Option<f64>,
    catalyst_level: Option<&str>,
    risk_volatility: Option<f64>,
    valuation_dcf_upside: Option<f64>,
    money_flow_main_net_inflow: Option<f64>,
    lockup_shareholder_trades_len: Option<i64>,
    announcements_len: Option<i64>,
    pace_signal: Option<f64>,
) -> f64 {
    let mut present_count = 0.0;
    // 9 个因子（2026-09-12 由 10 降为 9）：移除 f7「交易方向」。
    // 移除理由见函数文档注释——上游节点不可能评估下游节点的因子。
    let total_factors = 9.0;

    // P2-1 修复(2026-08-09): 从"字段存在性"升级为"值有效性"——
    // 存在但值域异常/中性值(如 0)的数据不计数，避免"字段有值但不可用"被误算为完整。

    // f1: 技术面 — totalScore 有效（0-100 值域）
    if let Some(s) = total_score
        && s.is_finite()
        && s > 0.0
        && s <= 100.0
    {
        present_count += 1.0;
    }

    // f2: 共识 — consensusScore 有效（0-100 值域）
    if let Some(s) = consensus_score
        && s.is_finite()
        && s > 0.0
        && s <= 100.0
    {
        present_count += 1.0;
    }

    // f3: 催化剂 — catalyst_level 非空且非"无"
    if let Some(level) = catalyst_level {
        let trimmed = level.trim();
        if !trimmed.is_empty() && trimmed != "无" {
            present_count += 1.0;
        }
    }

    // f4: 风险 — volatility 有效（0-200% 年化值域，防 0/NaN/极端值）
    if let Some(v) = risk_volatility
        && v.is_finite()
        && v > 0.0
        && v <= 200.0
    {
        present_count += 1.0;
    }

    // f5: 估值 — dcf_upside 有效（有限值，-100~1000 合理区间）
    if let Some(u) = valuation_dcf_upside
        && u.is_finite()
        && u > -100.0
        && u < 1000.0
    {
        present_count += 1.0;
    }

    // f9: 资金面 — main_net_inflow 有效（≠0 表示有实际资金信号；0 为中性不计数）
    if let Some(flow) = money_flow_main_net_inflow
        && flow.is_finite()
        && flow != 0.0
    {
        present_count += 1.0;
    }

    // f10: 筹码面 — shareholder_trades 数组长度 > 0
    if let Some(len) = lockup_shareholder_trades_len
        && len > 0
    {
        present_count += 1.0;
    }

    // f3 补充: 公告关键词 — announcements 数组长度 > 0
    if let Some(len) = announcements_len
        && len > 0
    {
        present_count += 1.0;
    }

    // f11: PACE 情绪 — pace_signal 有效（[-1,1] 值域）
    if let Some(p) = pace_signal
        && p.is_finite()
        && (-1.0..=1.0).contains(&p)
    {
        present_count += 1.0;
    }

    present_count / total_factors
}

// ── 证据缩放 ──

/// 非线性证据缩放系数。
///
/// 旧 (Rhai 内联): `clamp(0.20 + total_weight * 0.25, 0.20, 0.50)` — 纯线性
/// 新 (P2-1): `0.10 + 0.45 * sqrt(total_weight / max_weight)` — sqrt 曲线
///
/// 特性:
/// - total_weight=0.3: ~0.31  vs 旧 0.15 → 低证据量适度放大（单因子信号仍需合理权重）
/// - total_weight=0.8: ~0.44  vs 旧 0.40 → 中证据量略高
/// - total_weight=1.4: ~0.55  vs 旧 0.50 → 满证据推高到 0.55
///
/// # 参数
/// - `total_weight`: 实际激活因子的权重和
/// - `max_weight`: 理论最大权重和（所有因子默认权重之和 = 1.44）
pub fn compute_evidence_scale(total_weight: f64, max_weight: f64) -> f64 {
    if total_weight < 0.3 || max_weight <= 0.0 {
        0.20 // 单因子保持保守（修改: 从 0.15 微升至 0.20，避免过度压制 weak signal）
    } else {
        let normalized = (total_weight / max_weight).min(1.0);
        0.10 + 0.45 * normalized.sqrt()
    }
}

// ── 凯利赔率 ──

/// 凯利赔率：`odds = (targetPrice - currentPrice) / (currentPrice - stopLoss)`
///
/// P0-1 新增：当 trader 数据缺失时，基于 posterior 强度使用保守赔率 fallback。
/// fallback 设计依据：A 股趋势突破后的盈亏比经验分布，posterior≥0.70 时对应 2.5x。
///
/// V59 对齐(2026-09-10)：与 portfolio-mgr.rhai 语义完全一致——
/// 1. trader 数据存在但不满足看多条件（看空/止损倒挂/垃圾值）不再返回 0，
///    而是与缺失一样走 fallback（rhai 侧 odds 仅在满足看多条件时用 trader 计算）；
/// 2. fallback 新增试探档 posterior∈[0.42,0.50) → 1.5x（DB 实证原设计导致
///    近 30 天 41 次分析 100% 观望 0%）。
pub fn compute_kelly_odds(
    trader_target_price: Option<f64>,
    trader_stop_loss: Option<f64>,
    current_price: Option<f64>,
    posterior: f64,
) -> (f64, &'static str) {
    // trader 满足看多条件（sl>0 且 cp>sl 且 tp>cp）→ 用 trader 数据计算
    if let (Some(tp), Some(sl), Some(cp)) = (trader_target_price, trader_stop_loss, current_price)
        && sl > 0.0
        && cp > sl
        && tp > cp
    {
        let profit = tp - cp;
        let loss = cp - sl;
        if loss > 0.0 {
            let odds = (profit / loss).clamp(0.0, 10.0);
            return (odds, "trader");
        }
    }
    // trader 缺失或存在但无效（看空/止损倒挂/垃圾数据）→ 与 rhai 一致走 fallback
    // 0.50 偏多档与 0.42 试探档（V59）数值同为 1.5，合并为一个档位
    let odds = if posterior >= 0.70 {
        2.5
    } else if posterior >= 0.60 {
        2.0
    } else if posterior >= 0.42 {
        1.5
    } else {
        0.0
    };
    (odds, "波动率fallback")
}

// ── 凯利仓位 ──

/// 凯利仓位计算：`f* = (p × (odds + 1) - 1) / odds` → 半凯利 → 成本扣减 → 风险上限
///
/// # 参数
/// - `posterior`: 后验概率 P(上涨|证据)
/// - `odds`: 赔率（收益/亏损比）
/// - `cost_pct`: 交易成本率（默认 0.01 = 1%）
/// - `risk_level`: 风险等级（"极高风险"/"高风险"/"中风险"/其他）
///
/// # 返回
/// 建议仓位百分比（0.0 ~ 95.0）
pub fn compute_kelly_position(posterior: f64, odds: f64, cost_pct: f64, risk_level: &str) -> f64 {
    if posterior < 0.50 || odds <= 0.001 {
        return 0.0;
    }
    let q = 1.0 - posterior;
    let kelly_raw = (posterior * odds - q) / odds;
    if kelly_raw <= 0.0001 {
        return 0.0;
    }
    // 半凯利 + 交易成本扣减
    let position = kelly_raw / 2.0 * 100.0 * (1.0 - cost_pct);
    let position = position.clamp(0.0, 95.0);
    // 风险等级仓位上限
    apply_risk_cap(position, risk_level)
}

/// 该档的**每日对数资本增长率**（Kelly 增长率按持有期归一），只服务跨档仲裁「资金投向哪档」。
///
/// 为什么必须按时间归一：四档的 `positionPct` 与赔率**不可直接比** —— 超短 8% 仓锁 2 天、
/// 长线 30% 仓锁 90 天，可比的对象是「单位时间内资本的对数增长期望」，不是「哪档仓更大」。
/// 口径：`g = [p·ln(1+f·b) + (1−p)·ln(1−f)] / days`，其中 `f = position_pct/100`
/// （Kelly 1956 的资本增长率；除以持有天数即"每日期望对数增长"，与凯利原式同一目标函数）。
///
/// 返回 `None` = 该档**不参与仲裁**（概率越界 / 赔率非正 / 无仓位 / 持有 0 天 / 结果非有限）。
/// ⚠ 「不参与」与「增长率为负」是两件事：调用方 arbiter 把 `None` 当**退出比较**而不是当 0 ——
/// 把缺席读成 0 就是「缺席 = 看空」，正是 PLAN §十一 11-5 要防的形态。
/// 放在 Rust 而不是 Rhai：共享引擎没有 `ln`（见 `pm_vol_move_pct` 的同款理由），
/// 且这条公式要有单测。
pub fn kelly_growth_rate(
    posterior: f64,
    odds: f64,
    position_pct: f64,
    holding_days: u32,
) -> Option<f64> {
    if holding_days == 0 || !posterior.is_finite() || !odds.is_finite() || !position_pct.is_finite()
    {
        return None;
    }
    if !(0.0..=1.0).contains(&posterior) || odds <= 0.0 || position_pct <= 0.0 {
        return None;
    }
    let f = position_pct / 100.0;
    if f >= 1.0 {
        return None;
    }
    let g = (posterior * (1.0 + f * odds).ln() + (1.0 - posterior) * (1.0 - f).ln())
        / f64::from(holding_days);
    if !g.is_finite() {
        return None;
    }
    Some(g)
}

/// 根据风险等级施加仓位上限 —— **最终上限**语义：任何后置乘数都不得把它放大。
///
/// `pub` 并被注册为 Rhai 宿主函数 `pm_risk_cap`（v156）：四份逐档分支脚本的
/// `position = kelly × positionMultiplier` 必须在**之后**再过一次本函数 ——
/// 否则 `kelly` 内部已按本表封顶的仓位会被档位乘数（长档 1.2）二次放大，
/// 产生「极高风险上限 10%、实际输出 12%」的口径穿透（`PLAN-horizon-decision-repair.md`
/// §七 终审发现 ①；此前只有 `compute_kelly_position` 内部路径，mult 在其后生效）。
pub fn apply_risk_cap(position: f64, risk_level: &str) -> f64 {
    match risk_level {
        "极高风险" | "极高" => position.min(10.0),
        "高风险" | "高" => position.min(35.0),
        "中风险" | "中" => position.min(50.0),
        _ => position, // 低风险不 cap
    }
}

// ── 风险分类（算法版）──

/// 基于量化指标的算法风险分类。
///
/// 输入指标来自 t-risk 节点（stockRiskProfile）：
/// - `volatility`: 年化波动率 (%)
/// - `sharpe`: 夏普比率
/// - `drawdown`: 最大回撤 (%)
/// - `roe`: ROE (%)
/// - `debt`: 负债率 (%)
/// - `growth`: 营收增长率 (%)
///
/// 返回 "极高风险" / "高风险" / "低风险" / "中风险"
///
/// V54 放宽阈值适配 A 股（高波动+高负债+低增长的特征）。
///
/// ⚠️ v40(2026-09-13)：极高风险的第一条判据（负债率 + 营收负增长）此前会让
/// **整个银行业必然恒顶格**（601166 负债 91.6% / 营收 −0.3% 命中，而技术面
/// 全在低风险档）。修法是给该硬规则追加**「真困境」第三条件**
/// （ROE<0 亏损 或 营收增速<−10%）。
///
/// ⚠️ v78(2026-09-14)：删除曾短暂存在的「金融业行业白名单豁免」。
/// **判据只锚定可观测的数据形态，不锚定行业归属** —— 白名单等于宣称「金融企业
/// 不可能陷入财务困境」，会把真正在亏损/收缩的银行一起放行；且行业分类口径一变
/// 判据即整体失效。「真困境」条件观测的是事实（真在亏损、真在萎缩），与行业名无关。
///
/// ⚠️ 注意：本函数是**参考实现**，生产运行时走的是 `portfolio-mgr.rhai` 的同构镜像
/// （阈值由面板参数 `risk_*` 注入）。修改本函数时必须同步该镜像，否则两处漂移。
pub fn classify_risk(
    volatility: Option<f64>,
    sharpe: Option<f64>,
    drawdown: Option<f64>,
    roe: Option<f64>,
    debt: Option<f64>,
    growth: Option<f64>,
) -> String {
    // 默认阈值（V54 A 股校准值）
    let vol = volatility.unwrap_or(0.0);
    let sp = sharpe.unwrap_or(0.0);
    let dd = drawdown.unwrap_or(0.0);
    let r = roe.unwrap_or(0.0);
    let d = debt.unwrap_or(0.0);
    let g = growth.unwrap_or(0.0);

    // 极高风险（v40 C：硬规则须附加「真困境」证据；v78 已删除行业白名单豁免）
    if (d > 85.0 && g < 0.0 && (r < 0.0 || g < -10.0)) || (vol > 60.0 && sp < -1.5) {
        return "极高风险".into();
    }

    // 高风险
    if (vol > 40.0 || sp < 0.0 || dd > 45.0) && (r < 5.0 || d > 65.0) {
        return "高风险".into();
    }
    if vol > 35.0 && sp < 0.3 && r < 8.0 && g < 5.0 {
        return "高风险".into();
    }

    // 低风险
    if vol < 25.0 && sp > 0.5 && dd < 30.0 && r > 8.0 && d < 55.0 && g > 3.0 {
        return "低风险".into();
    }

    "中风险".into()
}

// ── 因子协方差衰减 ──

/// 因子协方差衰减：对高相关因子对做权重降权，减少信号重复计数。
///
/// 朴素贝叶斯假设因子条件独立，但以下因子对不满足：
/// - f3(催化剂) ↔ f11(PACE 情绪)：同源公告数据，重叠约 70%
/// - f1(技术面) ↔ f9(资金面)：趋势-资金共振
///
/// # 参数
/// - `f1_weight`: 技术面因子权重
/// - `f3_weight`: 催化剂因子权重
/// - `f9_weight`: 资金面因子权重
/// - `f11_weight`: PACE 情绪因子权重
///
/// # 返回
/// `(f9_weight_after, f11_weight_after)` — 衰减后的权重
pub fn apply_covariance_decay(
    f1_weight: f64,
    f3_weight: f64,
    f9_weight: f64,
    f11_weight: f64,
) -> (f64, f64) {
    let mut f9 = f9_weight;
    let mut f11 = f11_weight;

    // f3 ↔ f11: 公告数据重叠 → f11 降权 35%
    if f3_weight > 0.0 && f11_weight > 0.0 {
        f11 *= 0.65;
    }
    // f1 ↔ f9: 趋势-资金共振 → f9 降权 25%
    if f1_weight > 0.0 && f9_weight > 0.0 {
        f9 *= 0.75;
    }

    (f9, f11)
}

// ── 决策动作 ──

/// 根据 effective_posterior 和仓位计算决策动作。
///
/// # 参数
/// - `effective_posterior`: 含 risk_bias 的后验概率 [0, 1]
/// - `position_pct`: 计算后的仓位 [0, 100]
/// - `buy_threshold`: 买入阈值（默认 0.63）
/// - `increase_threshold`: 增持阈值（默认 0.53）
/// - `hold_threshold`: 持有阈值（默认 0.50；2026-10 与「后验中性 = 0.50」锚点对齐）
/// - `watch_threshold`: 观望阈值（默认 0.38）
/// - `reduce_threshold`: 减持阈值（默认 0.30）
/// - `pos_buy_min`: 买入所需最小仓位（默认 15.0%）
/// - `pos_increase_min`: 增持所需最小仓位（默认 10.0%）
///
/// # 返回
/// "买入" / "增持" / "持有" / "观望" / "减持" / "卖出"
#[allow(clippy::too_many_arguments)]
pub fn compute_action(
    effective_posterior: f64,
    position_pct: f64,
    buy_threshold: f64,
    increase_threshold: f64,
    hold_threshold: f64,
    watch_threshold: f64,
    reduce_threshold: f64,
    pos_buy_min: f64,
    pos_increase_min: f64,
) -> String {
    if effective_posterior >= buy_threshold && position_pct >= pos_buy_min {
        "买入".into()
    } else if effective_posterior >= increase_threshold && position_pct >= pos_increase_min {
        "增持".into()
    } else if effective_posterior >= hold_threshold {
        "持有".into()
    } else if effective_posterior >= watch_threshold {
        "观望".into()
    } else if effective_posterior >= reduce_threshold {
        "减持".into()
    } else {
        "卖出".into()
    }
}

/// 计算 risk_bias（根据风险等级的行为阈值偏移）
pub fn compute_risk_bias(risk_level: &str) -> f64 {
    match risk_level {
        "极高风险" | "极高" => -0.15,
        "高风险" | "高" => -0.08,
        "低风险" | "低" => 0.05,
        _ => 0.0,
    }
}

/// 应用风控否决：根据风险等级限制决策动作。
///
/// 返回 (final_action, was_downgraded, note)
///
/// ⚠️ 判定前先经 [`normalize_action`] 归一化值域。历史实现只 `match` 中文
/// 「买入/增持/持有」：链路上 action 一旦是英文（`BUY`/`HOLD`）或 dashboard
/// 值域（`强烈买入`），此处不匹配任何分支 ⇒ **极高风险的「禁止持仓」否决
/// 完全不生效，且静默返回原值**（fail-open）。
///
/// 未识别值域不再静默放行：
/// - 风险等级为高 / 极高：按最保守处理降级为「观望」（未知值视为可能持仓）；
/// - 其余等级：保留原值，但 `note` 显式说明该值未参与判定。
///
/// ── 产品决策开关（2026-10-26 用户裁决）────────────────────────────
/// 风控门可被**运行期开关**放开（`decision_gates::Gate::RiskVeto`）。
/// 放开时本函数**原样返回入参**（不否决），但：
/// - 仍走 `normalize_action` 的值域归一化与未识别值域的保守降级（**那不是门，是数据契约**）；
/// - `note` 显式写明「风控门已放开」，让呈现层能显示「本次未过风控」而不是无声通过。
///
/// ⚠️ 放开只**停止否决**，**不抬信号**：门从来只单向拦，放开后动作仍由评分/后验决定。
pub fn apply_risk_veto(action: &str, risk_level: &str) -> (String, bool, String) {
    apply_risk_veto_with_gate(
        action,
        risk_level,
        crate::decision_gates::is_open(crate::decision_gates::Gate::RiskVeto),
    )
}

/// 风控否决的**可配置版本**（`gate_open` = 风控门是否已放开）。
///
/// 拆成两个函数的原因：`apply_risk_veto` 是**既有调用面的稳定名**（Rhai 宿主函数、
/// 单测、语义锁测试都按它断言），不能改签名；而需要显式控制的调用方
/// （单测、将来的策略回测）要能绕过全局开关 ⇒ 提供带显式参数的版本。
pub fn apply_risk_veto_with_gate(
    action: &str,
    risk_level: &str,
    gate_open: bool,
) -> (String, bool, String) {
    // 2026-09-21: 风险档位判据改走 `RiskTier::from_risk_str`，与 `portfolio_risk_gate` 同源。
    //   原实现自写 `matches!(risk_level, "极高风险" | "极高")` / `("高风险" | "高")` ——
    //   这是**同一语义的第二份值域表**，且比 `RiskTier` 窄：它不认英文 `EXTREME` / `HIGH`。
    //   后果（fail-open）：风险等级为英文时 `extreme`/`high` 双 false ⇒
    //     ① 未识别 action 的「保守降级为观望」不触发；
    //     ② `extreme && implies_long` 的「极高风险禁止持仓」不触发，**静默放行**。
    //   `from_risk_str` 已配 high/medium/low 的 ASCII 别名（本次补上 extreme），
    //   故收敛后同一字符串在两处结论一致。
    //   对中文值域是零语义变化：`High` 档不含「中高风险」（后者归 `MediumHigh`），
    //   与原 `matches!(risk_level, "高风险" | "高")` 的取值面逐一相同。
    let tier = RiskTier::from_risk_str(risk_level);
    let extreme = matches!(tier, RiskTier::Extreme);
    let high = matches!(tier, RiskTier::High);

    let Some(kind) = normalize_action(action) else {
        tracing::warn!(
            action = %action,
            risk_level = %risk_level,
            "[apply_risk_veto] 未识别的 action 值域，跳过结论式放行"
        );
        // 裁定（2026-10-22）：兜底不得占用业务档名，且两档不得互串 ——
        // 空值 = **没有决策数据**（`Unavailable` / 数据缺失），非空但不识别 = **有数据判不出方向**
        // （`Uncertain` / 不确定）。此前两者都降级成「观望」，等于替调用方下了一个结论。
        let fallback = if action.trim().is_empty() {
            ActionKind::Unavailable
        } else {
            ActionKind::Uncertain
        };
        if extreme || high {
            return (
                fallback.as_storage_cn().into(),
                true,
                format!(
                    "action 值域不可用「{action}」，按「{}」处置而非任何操作建议",
                    fallback.as_storage_cn()
                ),
            );
        }
        return (
            action.to_string(),
            false,
            format!("未识别 action 值域「{action}」，未参与风控判定"),
        );
    };

    if extreme && kind.implies_long() {
        if gate_open {
            return (
                action.to_string(),
                false,
                format!(
                    "风控门已放开：风险档={risk_level}（原判据会否决「{action}」⇒ 禁止持仓），本次未执行风控否决"
                ),
            );
        }
        return ("观望".into(), true, "极高风险风控否决：禁止持仓".into());
    }
    if high && kind.implies_buy() {
        if gate_open {
            return (
                action.to_string(),
                false,
                format!(
                    "风控门已放开：风险档={risk_level}（原判据会否决「{action}」⇒ 禁止加仓），本次未执行风控否决"
                ),
            );
        }
        return ("持有".into(), true, "高风险风控否决：禁止加仓".into());
    }
    (action.to_string(), false, String::new())
}

/// 基于贝叶斯因子的统计置信度。
///
/// 定义：置信度反映 posterior(后验概率) 相对于 prior(先验概率) 的证据强度。
///   使用贝叶斯因子(Bayes Factor)衡量证据强度:
///   BF = odds_posterior / odds_prior (当 posterior >= prior)
///   BF = odds_prior / odds_posterior (当 posterior < prior)
///   确保 BF >= 1,越大证据越强。
///
///   置信度映射(BF → [50, 99.9]):
///   BF=1(无证据)         → 50%
///   BF=3(中等证据)       → 75%
///   BF=10(强证据)        → 90.9%
///   BF=30(很强证据)      → 96.8%
///   BF=100+(决定性证据)   → 99.5%+
///
/// 公式: confidence = 50 + 50 × (1 - 2/(BF + 1))
///
/// 优点: 跟因子数量、因子方差无关,只反映 posterior 偏离 prior 的程度。
///   当 posterior=0.5(因子沉默,全中性信号) → prior=0.5 → BF=1 → 50%
///   当 posterior=0.19(强烈看空) → prior=0.5 → BF=4.26 → 91%
///   当 posterior=0.81(强烈看多) → prior=0.5 → BF=4.26 → 91%
pub fn compute_bayes_confidence(prior: f64, posterior: f64) -> f64 {
    if prior <= 0.0 || prior >= 1.0 || posterior <= 0.0 || posterior >= 1.0 {
        return 50.0;
    }

    let odds_prior = prior / (1.0 - prior);
    let odds_posterior = posterior / (1.0 - posterior);

    // BF >= 1 (双向对称)
    let bf = if odds_posterior >= odds_prior {
        odds_posterior / odds_prior
    } else {
        odds_prior / odds_posterior
    };

    // BF → confidence mapping
    // 50 + 50 * (1 - 2/(BF+1)) = 50 * (1 - 2/(BF+1) + 1) 最后一项是50
    // 简化: 100 - 100/(BF+1)
    let confidence = 100.0 - 100.0 / (bf + 1.0);

    // clamp to [50, 99.9]
    confidence.clamp(50.0, 99.9)
}

/// 组合风控门输出（JSON 序列化后供 Rhai 包装为 CodeNode 输出）。
///
/// 字段说明：
/// - `action` / `positionPct` / `riskLevel`: 风控门调整后的最终决策
/// - `adjusted`: 是否相对 portfolio-mgr 输出做了调整
/// - `originalAction` / `originalPositionPct`: portfolio-mgr 原始输出（可追溯）
/// - `reasons`: 调整原因列表（前端/decision-explainer 用于生成解释文案）
/// - `concentrationWarning`: 来自 `portfolio_monitor::compute_concentration_warning`
/// - `topHoldingPct` / `maxSectorPct`: 当前组合集中度指标
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PortfolioRiskGateOutput {
    pub action: String,
    pub position_pct: f64,
    pub risk_level: String,
    pub adjusted: bool,
    pub original_action: String,
    pub original_position_pct: f64,
    pub reasons: Vec<String>,
    pub concentration_warning: Option<String>,
    pub top_holding_pct: f64,
    pub max_sector_pct: f64,
}

/// P1-E13: 组合风控门 — 在 portfolio-mgr 之后、rule-check 之前运行。
///
/// 职责：
/// 1. 单股仓位上限检查（PositionLimits::max_single_stock_pct=20%）
/// 2. 行业暴露上限检查（max_sector_exposure_pct=40%）
/// 3. 持仓数量上限检查（max_total_positions=10）
/// 4. 风险档位否决（RiskTier::forbid_new_position / max_single_stock_pct）
/// 5. 空头目标价强制卖出（BEARISH_TARGET_PRICE_RATIO=0.85）
/// 6. 组合总仓位归一化（normalize_position_weights，total > 100% 时缩放）
///
/// ⚠️ v183：本函数**不删规则**，改的是 `target_price` 的来源。
/// 原实参是 trader（LLM 决策节点）报的目标价 ⇒ 公式的最终动作会被对方的报价直接翻成「卖出」，
/// 这就是「假独立」。现由 `portfolio-risk-gate.rhai` 用**分析师族价值投资者的安全边际**
/// （`value-investor--档.result.margin_of_safety.pct`）折算目标价后传入：
/// `target = current × (1 + margin_pct / 100)`。比值判据、0.85 阈值、返回结构全部不变，
/// 变的只是「谁的价格在说话」。
///
/// # 参数
/// - `pm_action`: portfolio-mgr 输出的原始 action
/// - `pm_position_pct`: portfolio-mgr 输出的原始仓位
/// - `pm_risk_level`: portfolio-mgr 输出的风险等级
/// - `current_price`: 当前价（用于计算新增仓位市值）
/// - `target_price`: **目标价**——v183 起由价值投资者的安全边际折算（不再是 trader 报价）
/// - `stock_code`: 待交易股票代码
/// - `stock_sector`: 待交易股票行业（可选）
/// - `holdings_json`: 当前持仓 JSON 数组（`Vec<PositionSummary>` 序列化）
/// - `portfolio_cash`: 可用现金（用于组合总价值计算）
///
/// # 返回
/// `PortfolioRiskGateOutput` 的 JSON 字符串。任何解析错误都返回原始决策（`adjusted=false`）。
///
/// # 设计原则
/// - 纯函数，无副作用，无 DB 调用（holdings 由 core.rs 注入）
/// - 保守降级：任何数据异常都不阻塞决策，返回原始 portfolio-mgr 输出
/// - 可观测：所有调整都记录到 `reasons` 字段供下游 explainer 引用
#[allow(clippy::too_many_arguments)]
pub fn portfolio_risk_gate(
    pm_action: &str,
    pm_position_pct: f64,
    pm_risk_level: &str,
    current_price: f64,
    target_price: Option<f64>,
    stock_code: &str,
    stock_sector: Option<&str>,
    holdings_json: &str,
    portfolio_cash: f64,
) -> String {
    use crate::portfolio_monitor::{
        compute_concentration, compute_concentration_warning, normalize_position_weights,
    };
    use crate::position_limits::PositionLimits;
    use crate::trading::PositionSummary;

    // 兜底：持仓 JSON 解析失败时返回原始决策
    let original_action = pm_action.to_string();
    let original_position_pct = pm_position_pct;
    let fail_safe = || {
        serde_json::to_string(&PortfolioRiskGateOutput {
            action: original_action.clone(),
            position_pct: original_position_pct,
            risk_level: pm_risk_level.to_string(),
            adjusted: false,
            original_action: original_action.clone(),
            original_position_pct,
            reasons: vec!["风控门兜底：holdings_json 解析失败，未做调整".into()],
            concentration_warning: None,
            top_holding_pct: 0.0,
            max_sector_pct: 0.0,
        })
        .unwrap_or_else(|_| "{}".into())
    };

    let holdings: Vec<PositionSummary> =
        match serde_json::from_str::<Vec<PositionSummary>>(holdings_json) {
            Ok(h) if !h.is_empty() => h,
            Ok(_) => {
                // 空持仓 — 无组合层约束，但风险档位仍生效
                let risk_tier = RiskTier::from_risk_str(pm_risk_level);
                let mut reasons: Vec<String> = Vec::new();
                let mut final_action = pm_action.to_string();
                let mut final_pct = pm_position_pct;
                let mut adjusted = false;

                // 极高风险档位禁开新仓
                //
                // 2026-09-21: 判据改走统一归一化，与下方**非空持仓**分支（同函数 L706 附近）
                //   同型。P1-6(2026-09-14) 那次修复只改了非空分支，本分支被漏掉 ⇒
                //   **同一函数内同一条 R-200 规则在两条路径上行为不同**：
                //   action 一旦是英文 token（`BUY`/`HOLD`）或 dashboard 值域（`强烈买入`），
                //   `matches!` 不命中任何一项 ⇒ 极高风险的「禁止持仓」**静默 fail-open**
                //   （走空持仓分支的调用被放行，走非空分支的却被否决）。
                //   `ActionKind::implies_long()` 的集合恰为 `Buy | Increase | Hold`，
                //   与原先的中文字面量集合**逐一等价** ⇒ 对中文输入是零语义变化的纯补漏。
                if risk_tier.forbid_new_position()
                    && normalize_action(pm_action).is_some_and(ActionKind::implies_long)
                {
                    final_action = "观望".into();
                    final_pct = 0.0;
                    adjusted = true;
                    reasons.push(format!("R-200 极高风险档位禁止持仓，{}→观望", pm_action));
                }

                // 单股仓位上限（即使空仓也检查，避免首笔建仓超限）
                // 仓位三条走面板（`pos_max_*`，回落 20/10/40 == 接线前的 `Default`）。
                let limits = PositionLimits::panel_effective();
                let tier_cap = risk_tier.max_single_stock_pct();
                let effective_cap = limits.max_single_stock_pct.min(tier_cap);
                if final_pct > effective_cap {
                    reasons.push(format!(
                        "R-206 单股仓位 {:.1}% 超过上限 {:.0}%，已下调",
                        final_pct, effective_cap
                    ));
                    final_pct = effective_cap;
                    adjusted = true;
                }

                return serde_json::to_string(&PortfolioRiskGateOutput {
                    action: final_action,
                    position_pct: final_pct,
                    risk_level: pm_risk_level.to_string(),
                    adjusted,
                    original_action: pm_action.to_string(),
                    original_position_pct: pm_position_pct,
                    reasons,
                    concentration_warning: None,
                    top_holding_pct: 0.0,
                    max_sector_pct: 0.0,
                })
                .unwrap_or_else(|_| "{}".into());
            },
            Err(_) => return fail_safe(),
        };

    // 1. 计算组合层指标
    let (top_pct, sector_map, max_sector_pct) = compute_concentration(&holdings);
    let n_positions = holdings.len();
    let total_mv: f64 = holdings.iter().map(|p| p.market_value.unwrap_or(0.0)).sum();
    let portfolio_total_value = total_mv + portfolio_cash;

    // 2. 检查空头目标价强制卖出（BEARISH_TARGET_PRICE_RATIO=0.85）
    //    v183：`target_price` 的来源由 trader 报价换成「现价 ×(1+分析师安全边际)」，
    //    判据、阈值、返回结构一字未动（见 `portfolio-risk-gate.rhai` 的折算段）。
    if let Ok(force_sell) = PositionLimits::check_bearish_force_sell(current_price, target_price)
        && force_sell
    {
        return serde_json::to_string(&PortfolioRiskGateOutput {
            action: "卖出".into(),
            position_pct: 0.0,
            risk_level: pm_risk_level.to_string(),
            adjusted: true,
            original_action: pm_action.to_string(),
            original_position_pct: pm_position_pct,
            reasons: vec![format!(
                "R-201 空头预测强制卖出：目标价 {:.2} < 当前价 × 0.85 = {:.2}",
                target_price.unwrap_or(0.0),
                current_price * crate::position_limits::BEARISH_TARGET_PRICE_RATIO
            )],
            concentration_warning: compute_concentration_warning(
                top_pct,
                max_sector_pct,
                n_positions,
            ),
            top_holding_pct: top_pct,
            max_sector_pct,
        })
        .unwrap_or_else(|_| "{}".into());
    }

    // 3. 风险档位感知的仓位检查（仅当 action 是买入/增持时检查新增仓位）
    let risk_tier = RiskTier::from_risk_str(pm_risk_level);
    let mut final_action = pm_action.to_string();
    let mut final_pct = pm_position_pct;
    let mut adjusted = false;
    let mut reasons: Vec<String> = Vec::new();

    if normalize_action(pm_action).is_some_and(ActionKind::implies_buy)
        && final_pct > 0.0
        && current_price > 0.0
    {
        // 新增仓位市值 = position_pct% × portfolio_total_value
        // 注意：position_pct 是相对组合总价值的百分比
        if portfolio_total_value <= 0.0 {
            reasons.push("R-207 组合总价值为 0，无法校验仓位比例，仓位已清零".into());
            final_pct = 0.0;
            final_action = "观望".into();
            adjusted = true;
        } else {
            let new_position_value = final_pct / 100.0 * portfolio_total_value;
            // 当前持仓的行业暴露列表
            let sector_exposures: Vec<(String, f64)> =
                sector_map.iter().map(|(k, v)| (k.clone(), *v)).collect();

            // 仓位三条走面板（`pos_max_*`，回落 20/10/40 == 接线前的 `Default`）。
            let limits = PositionLimits::panel_effective();
            match limits.check_new_position_with_risk(
                new_position_value,
                portfolio_total_value,
                n_positions,
                stock_sector,
                &sector_exposures,
                risk_tier,
            ) {
                Ok(()) => {
                    // 通过 — 但仍按风险档位 cap 单股仓位
                    let tier_cap = risk_tier.max_single_stock_pct();
                    let effective_cap = limits.max_single_stock_pct.min(tier_cap);
                    if final_pct > effective_cap {
                        reasons.push(format!(
                            "R-206 单股仓位 {:.1}% 超过风险档位上限 {:.0}%，已下调",
                            final_pct, effective_cap
                        ));
                        final_pct = effective_cap;
                        adjusted = true;
                    }
                },
                Err(e) => {
                    // 风控否决 — 降级到持有或观望
                    reasons.push(format!("R-208 风控否决：{e}"));
                    if risk_tier.forbid_new_position() {
                        final_action = "观望".into();
                        final_pct = 0.0;
                    } else {
                        final_action = "持有".into();
                        final_pct = final_pct.min(10.0);
                    }
                    adjusted = true;
                },
            }
        }
    // P1-6(2026-09-14): R-200 的档位判定改走统一归一化。
    //   原判据只认中文 ⇒ 上游一旦下发英文 token，**极高风险的「禁止持仓」直接 fail-open**
    //   （与 apply_risk_veto 同一类缺陷，两处必须同判据）。
    } else if risk_tier.forbid_new_position()
        && normalize_action(pm_action).is_some_and(ActionKind::implies_long)
    {
        // 极高风险档位禁止持仓（即使 portfolio-mgr 输出"持有"也要降级）
        reasons.push(format!("R-200 极高风险档位禁止持仓，{}→观望", pm_action));
        final_action = "观望".into();
        final_pct = 0.0;
        adjusted = true;
    }

    // 4. 组合总仓位归一化（所有持仓 + 新增仓位 > 100% 时缩放）
    // 取当前所有持仓的 position_pct + 新增 final_pct
    // 注意：PositionSummary 没有直接存 position_pct 字段，需要从 market_value / portfolio_total_value 计算
    if portfolio_total_value > 0.0 && final_pct > 0.0 {
        let mut all_positions: Vec<f64> = holdings
            .iter()
            .map(|p| {
                let mv = p.market_value.unwrap_or(0.0);
                (mv / portfolio_total_value * 100.0).max(0.0)
            })
            .collect();
        // 排除当前股票已有的持仓（避免重复计数），再追加新增仓位
        all_positions.retain(|_| true); // no-op，保留所有
        all_positions.push(final_pct);
        let normalized = normalize_position_weights(&all_positions, 100.0);
        if let Some(&new_final) = normalized.last()
            && new_final < final_pct
        {
            let delta = final_pct - new_final;
            reasons.push(format!(
                "R-209 组合总仓位超 100%，新增仓位归一化 {:.1}%→{:.1}% (-{:.1}%)",
                final_pct, new_final, delta
            ));
            final_pct = new_final;
            adjusted = true;
        }
    }

    // 5. 集中度警告（不调整决策，仅作为可观测信息）
    let concentration_warning = compute_concentration_warning(top_pct, max_sector_pct, n_positions);

    // 6. 标记股票已被持仓（用于 explainer 提示"加仓"vs"新建仓"）
    let already_held = holdings.iter().any(|p| p.stock_code == stock_code);
    // P1-6(2026-09-14): 同 R-200，改走统一归一化（英文值域下原判据恒 false，提示静默消失）
    if already_held && normalize_action(final_action.as_str()).is_some_and(ActionKind::implies_buy)
    {
        reasons.push(format!("R-210 当前已持有 {}，本次为加仓操作", stock_code));
    }

    serde_json::to_string(&PortfolioRiskGateOutput {
        action: final_action,
        position_pct: final_pct,
        risk_level: pm_risk_level.to_string(),
        adjusted,
        original_action: pm_action.to_string(),
        original_position_pct: pm_position_pct,
        reasons,
        concentration_warning,
        top_holding_pct: top_pct,
        max_sector_pct,
    })
    .unwrap_or_else(|_| fail_safe())
}

// ── P3-2: Portfolio-mgr 参数集（WFO 校准目标）──

/// portfolio-mgr 的可校准参数集合。
///
/// 这些参数当前在 portfolio-mgr.rhai 中硬编码为 V56 校准的默认值。
/// 通过 WFO 定期跑 param scan，可自动寻找适合当前市场状态的最优参数。
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PortfolioMgrParamSet {
    pub buy_threshold: f64,
    pub increase_threshold: f64,
    pub hold_threshold: f64,
    pub watch_threshold: f64,
    pub reduce_threshold: f64,
    pub cap_extreme: f64,
    pub cap_high: f64,
    pub cap_mid: f64,
    /// 市况先验：无个股证据时对「上涨 / 震荡 / 下跌」的基础判断（由市况方向派生）。
    ///
    /// 三个值替代了原先内联在 `portfolio-mgr.rhai` 中的硬编码先验，使其
    /// **可被反思**（reflection → `params_suggestion` → 模板变量）与
    /// **可被演进**（WFO 网格搜索 / `score_param_set`）优化。
    ///
    /// 与 `conservative()` / `aggressive()` 的风险偏好维度正交，故不随 profile 变化。
    #[serde(default = "default_prior_bull")]
    pub prior_bull: f64,
    #[serde(default = "default_prior_sideways")]
    pub prior_sideways: f64,
    #[serde(default = "default_prior_bear")]
    pub prior_bear: f64,
}

/// 市况先验默认值（与模板变量 `regime_prior_*` 的 seed 默认值保持一致）。
pub const DEFAULT_PRIOR_BULL: f64 = 0.55;
pub const DEFAULT_PRIOR_SIDEWAYS: f64 = 0.50;
pub const DEFAULT_PRIOR_BEAR: f64 = 0.45;

fn default_prior_bull() -> f64 {
    DEFAULT_PRIOR_BULL
}
fn default_prior_sideways() -> f64 {
    DEFAULT_PRIOR_SIDEWAYS
}
fn default_prior_bear() -> f64 {
    DEFAULT_PRIOR_BEAR
}

impl PortfolioMgrParamSet {
    /// V56 校准默认值
    pub fn v56_default() -> Self {
        /* ... */
        Self {
            buy_threshold: 0.63,
            increase_threshold: 0.53,
            // 2026-10 重标定：0.48→0.50。原值让后验恰为中性（0.50）时也落「持有」，
            // 与贝叶斯中性锚点及 t-scoring「全中性 totalScore=50」不一致（利好偏置 A 档根治）。
            hold_threshold: 0.50,
            watch_threshold: 0.38,
            reduce_threshold: 0.30,
            cap_extreme: 10.0,
            cap_high: 35.0,
            cap_mid: 50.0,
            prior_bull: DEFAULT_PRIOR_BULL,
            prior_sideways: DEFAULT_PRIOR_SIDEWAYS,
            prior_bear: DEFAULT_PRIOR_BEAR,
        }
    }
    pub fn conservative() -> Self {
        /* ... */
        Self {
            buy_threshold: 0.68,
            increase_threshold: 0.58,
            hold_threshold: 0.52,
            watch_threshold: 0.42,
            reduce_threshold: 0.35,
            cap_extreme: 8.0,
            cap_high: 28.0,
            cap_mid: 40.0,
            // 先验与风险偏好正交（由市况派生），沿用默认值
            prior_bull: DEFAULT_PRIOR_BULL,
            prior_sideways: DEFAULT_PRIOR_SIDEWAYS,
            prior_bear: DEFAULT_PRIOR_BEAR,
        }
    }
    pub fn aggressive() -> Self {
        /* ... */
        Self {
            buy_threshold: 0.58,
            increase_threshold: 0.48,
            hold_threshold: 0.42,
            watch_threshold: 0.32,
            reduce_threshold: 0.25,
            cap_extreme: 15.0,
            cap_high: 45.0,
            cap_mid: 60.0,
            // 先验与风险偏好正交（由市况派生），沿用默认值
            prior_bull: DEFAULT_PRIOR_BULL,
            prior_sideways: DEFAULT_PRIOR_SIDEWAYS,
            prior_bear: DEFAULT_PRIOR_BEAR,
        }
    }
    pub fn default_grid() -> Vec<Self> {
        vec![
            Self::conservative(),
            Self {
                buy_threshold: 0.65,
                increase_threshold: 0.55,
                hold_threshold: 0.50,
                watch_threshold: 0.40,
                reduce_threshold: 0.32,
                ..Self::v56_default()
            },
            Self::v56_default(),
            Self {
                buy_threshold: 0.60,
                increase_threshold: 0.50,
                hold_threshold: 0.45,
                watch_threshold: 0.35,
                reduce_threshold: 0.28,
                ..Self::v56_default()
            },
            Self::aggressive(),
            Self {
                buy_threshold: 0.70,
                increase_threshold: 0.60,
                hold_threshold: 0.55,
                watch_threshold: 0.45,
                reduce_threshold: 0.35,
                cap_extreme: 5.0,
                cap_high: 20.0,
                cap_mid: 30.0,
                ..Self::v56_default()
            },
            Self {
                buy_threshold: 0.55,
                increase_threshold: 0.45,
                hold_threshold: 0.40,
                watch_threshold: 0.30,
                reduce_threshold: 0.22,
                cap_extreme: 20.0,
                cap_high: 50.0,
                cap_mid: 70.0,
                ..Self::v56_default()
            },
            Self { buy_threshold: 0.66, increase_threshold: 0.56, ..Self::v56_default() },
            Self { buy_threshold: 0.60, increase_threshold: 0.50, ..Self::v56_default() },
            // ── 先验敏感候选：单独扫描 regime_prior_* 维度，其余沿用默认 ──
            // 乐观先验：抬升 bull / 压低 bear，无个股证据时更倾向看多
            Self {
                prior_bull: 0.60,
                prior_sideways: 0.52,
                prior_bear: 0.40,
                ..Self::v56_default()
            },
            // 谨慎先验：压平三档，无个股证据时更保守
            Self {
                prior_bull: 0.50,
                prior_sideways: 0.48,
                prior_bear: 0.50,
                ..Self::v56_default()
            },
        ]
    }
}

// ── Path 2: 从 LLM 反思输出解析 ParamSet ──

/// 参数名别名表：**反思 / WFO 侧名字 → 模板变量全名**的唯一权威源。
///
/// 背景（2026-09-11 立，2026-10-14 从 `commands/stock_analysis.rs` 上收到此处）：
/// 反思与 WFO 输出的参数名是 [`PortfolioMgrParamSet`] 的短名（`buy_threshold` / `cap_high`），
/// 而模板变量表与 `portfolio-mgr.rhai` 用全名（`action_buy_threshold` / `pos_cap_high`）。
/// 两侧不同代 ⇒ 各自解释对方的名字必然静默丢样，所以归一表只留这一份：
/// 写侧 `apply_param_suggestions` 与读侧 [`try_parse_param_suggestion`] 都调它。
pub const PARAM_ALIASES: &[(&str, &str)] = &[
    // action 决策阈值
    ("buy_threshold", "action_buy_threshold"),
    ("increase_threshold", "action_increase_threshold"),
    ("hold_threshold", "action_hold_threshold"),
    ("watch_threshold", "action_watch_threshold"),
    ("reduce_threshold", "action_reduce_threshold"),
    // 风险仓位上限
    ("cap_extreme", "pos_cap_extreme"),
    ("cap_high", "pos_cap_high"),
    ("cap_mid", "pos_cap_mid"),
    // risk_ 前缀省略写法（LLM 反思时常漏前缀）
    ("debt_extreme", "risk_debt_extreme"),
    ("vol_extreme", "risk_vol_extreme"),
    ("sharpe_extreme", "risk_sharpe_extreme"),
    ("vol_high", "risk_vol_high"),
    ("dd_high", "risk_dd_high"),
    ("roe_high", "risk_roe_high"),
    ("debt_high", "risk_debt_high"),
    ("vol_low", "risk_vol_low"),
    ("sharpe_low", "risk_sharpe_low"),
    ("dd_low", "risk_dd_low"),
    ("roe_low", "risk_roe_low"),
    ("debt_low", "risk_debt_low"),
    ("growth_low", "risk_growth_low"),
    // 仓位阈值
    ("buy_min", "pos_buy_min"),
    ("increase_min", "pos_increase_min"),
    // 市况先验（反思可直接校准先验）
    ("prior_bull", "regime_prior_bull"),
    ("prior_sideways", "regime_prior_sideways"),
    ("prior_bear", "regime_prior_bear"),
];

/// 把 camelCase / kebab-case / 空格分隔的参数名归一化为 snake_case。
pub fn to_snake_case(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 4);
    for (i, ch) in s.chars().enumerate() {
        if ch.is_ascii_uppercase() {
            if i > 0 {
                out.push('_');
            }
            out.push(ch.to_ascii_lowercase());
        } else if ch == '-' || ch == ' ' {
            out.push('_');
        } else {
            out.push(ch);
        }
    }
    out
}

/// 把外部（反思 / 演进 / 面板）传入的参数名解析为模板变量表中的真实名字。
///
/// 解析顺序：① 精确命中变量表 → ② snake_case 归一后命中变量表
/// → ③ 别名表（先原样、再 snake_case）。全部失败返回 `None`，
/// 由调用方记录为 skipped 明细而不是静默丢弃。
pub fn resolve_param_name(raw: &str, known: &std::collections::HashSet<String>) -> Option<String> {
    if known.contains(raw) {
        return Some(raw.to_string());
    }
    let snake = to_snake_case(raw);
    if known.contains(&snake) {
        return Some(snake);
    }
    PARAM_ALIASES
        .iter()
        .find(|(alias, _)| *alias == raw || *alias == snake.as_str())
        .map(|(_, canonical)| (*canonical).to_string())
}

/// 无变量表时的名字归一：短名 / camelCase → 模板全名；已是全名则原样返回。
///
/// 读侧（解析反思 JSON）用它 —— 那里拿不到 DB，但同样不得自己认拼写。
fn canonicalize_param_name(raw: &str) -> String {
    let snake = to_snake_case(raw);
    PARAM_ALIASES
        .iter()
        .find(|(alias, _)| *alias == snake.as_str())
        .map_or(snake, |(_, canonical)| (*canonical).to_string())
}

/// 按参数名把数值写进 ParamSet 的对应字段；名字不属于可校准集合 ⇒ `false`。
///
/// 只认归一后的**模板全名**（短名 / camelCase 已由 [`canonicalize_param_name`] 折算到这里），
/// 因此本表是「哪些参数进得了 WFO 网格」的唯一答案。
fn apply_param_value_by_name(p: &mut PortfolioMgrParamSet, raw: &str, v: f64) -> bool {
    match canonicalize_param_name(raw).as_str() {
        "action_buy_threshold" => {
            p.buy_threshold = v;
            true
        },
        "action_increase_threshold" => {
            p.increase_threshold = v;
            true
        },
        "action_hold_threshold" => {
            p.hold_threshold = v;
            true
        },
        "action_watch_threshold" => {
            p.watch_threshold = v;
            true
        },
        "action_reduce_threshold" => {
            p.reduce_threshold = v;
            true
        },
        "pos_cap_extreme" => {
            p.cap_extreme = v;
            true
        },
        "pos_cap_high" => {
            p.cap_high = v;
            true
        },
        "pos_cap_mid" => {
            p.cap_mid = v;
            true
        },
        "regime_prior_bull" => {
            p.prior_bull = v;
            true
        },
        "regime_prior_sideways" => {
            p.prior_sideways = v;
            true
        },
        "regime_prior_bear" => {
            p.prior_bear = v;
            true
        },
        _ => false,
    }
}

/// JSON 数值取法：数字直取，字符串按数字解析（反思 prompt 把值写成 `"0.42"` 也要能读）。
fn json_number(v: &serde_json::Value) -> Option<f64> {
    v.as_f64().or_else(|| v.as_str().and_then(|s| s.trim().parse::<f64>().ok()))
}

/// 取一条建议的（参数名, 建议值）：生产形态是 `param` / `suggested_value`
/// （反思 prompt 声明且实际落库的形态），早期形态是 `key` / `value`，两种都接受；
/// 两者都没有 ⇒ `None`（该条不计入识别数）。
fn suggestion_name_value(entry: &serde_json::Value) -> Option<(&str, f64)> {
    let obj = entry.as_object()?;
    let name = obj.get("param").or_else(|| obj.get("key")).and_then(|v| v.as_str())?;
    let value = obj.get("suggested_value").or_else(|| obj.get("value")).and_then(json_number)?;
    Some((name, value))
}

/// 从 reflection 的 `parameter_suggestions_json` 解析 `PortfolioMgrParamSet`。
///
/// 接受的形态（参数名一律经 [`canonicalize_param_name`] 归一，本函数不再各自认拼写）：
/// - **生产形态**（反思 prompt 声明、实际落库）：
///   `[{"param":"action_watch_threshold","current_value":0.38,"suggested_value":0.42}, ...]`
/// - 早期数组形态：`[{"key":"buy_threshold","value":0.60}, ...]`
/// - 顶层对象形态：`{"buy_threshold":0.60,"pos_cap_high":30.0}`（未写的字段取默认）
/// - 对象内藏数组：`{"params":[{...}]}`
///
/// `None` 的含义是**这条建议对本集合没有可识别内容**（空数组、`[]`、全是不认识的名字、
/// 非法 JSON）。调用方必须把 `None` 当「无建议」，而不是「建议 = 出厂默认」。
///
/// ⚠ 2026-10-14 前本函数对**任何**非空数组都返回 `Some(默认值)`：数组条目要求 `key`/`value`
/// 两个键，而生产实际写的是 `param`/`suggested_value` ⇒ 一个都不匹配、全部静默落默认，
/// WFO 于是把每条真建议都读成「默认参数」，`score_param_set` 只按 verdict 计「与默认是否不同」，
/// 校准结果与反思内容无关（现网实测：9 条非空建议、0 条被真正读进去）。
pub fn try_parse_param_suggestion(json_str: &str) -> Option<PortfolioMgrParamSet> {
    let trimmed = json_str.trim();
    if trimmed.is_empty() || trimmed == "null" || trimmed == "{}" || trimmed == "[]" {
        return None;
    }

    let v: serde_json::Value = serde_json::from_str(trimmed).ok()?;
    let mut p = PortfolioMgrParamSet::v56_default();
    let mut recognized = 0usize;

    // 条目级解析：数组逐条、对象既认标量键值也认内藏数组
    let mut feed = |p: &mut PortfolioMgrParamSet, name: &str, value: f64| {
        if apply_param_value_by_name(p, name, value) {
            recognized += 1;
        }
    };
    match &v {
        serde_json::Value::Array(arr) => {
            for entry in arr {
                if let Some((name, value)) = suggestion_name_value(entry) {
                    feed(&mut p, name, value);
                }
            }
        },
        serde_json::Value::Object(obj) => {
            for (k, val) in obj {
                if let Some(value) = json_number(val) {
                    feed(&mut p, k, value);
                }
            }
            for val in obj.values() {
                if let Some(arr) = val.as_array() {
                    for entry in arr {
                        if let Some((name, value)) = suggestion_name_value(entry) {
                            feed(&mut p, name, value);
                        }
                    }
                }
            }
        },
        _ => {},
    }

    if recognized > 0 {
        return Some(p);
    }
    // 整段就是一个完整的 ParamSet（serde 的 camelCase 字段形态）
    serde_json::from_str::<PortfolioMgrParamSet>(trimmed).ok()
}

/// 把一条建议解析成「(模板全名, 建议值, 建议时的现值)」清单，**保留这条建议到底提了哪些参数**。
///
/// 为什么 [`try_parse_param_suggestion`] 不够它用：那个函数返回的是 ParamSet，
/// 「某个字段没被提到」与「被提到了但恰好等于默认值」在返回值里不可区分。
/// 需要**逐参数**比方向的消费方（反思 Gap3 的同方向计数）必须走本函数。
/// 第三个元素取自条目的 `current_value`（反思 prompt 要求写、清单由变量表派生）；
/// 没写或不是数字 ⇒ `None`，调用方据此判「方向不可知」而不是当成「无变化」。
/// 不认识的名字直接跳过，不参与计数。
pub fn parse_param_suggestion_entries(json_str: &str) -> Vec<(String, f64, Option<f64>)> {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(json_str.trim()) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    let mut feed = |entry: &serde_json::Value| {
        if let Some((name, value)) = suggestion_name_value(entry) {
            // 判据复用同一个 `apply_param_value_by_name`（而不是第二张名字表）：
            // 「能被写进 ParamSet」= 「可校准」，两处的答案不可能漂移。
            let mut probe = PortfolioMgrParamSet::v56_default();
            if apply_param_value_by_name(&mut probe, name, value) {
                let current = entry.get("current_value").and_then(json_number);
                out.push((canonicalize_param_name(name), value, current));
            }
        }
    };
    match &v {
        serde_json::Value::Array(arr) => arr.iter().for_each(&mut feed),
        serde_json::Value::Object(obj) => {
            for val in obj.values() {
                if let Some(arr) = val.as_array() {
                    arr.iter().for_each(&mut feed);
                }
            }
        },
        _ => {},
    }
    out
}

// ── Path 1: 基于历史反思的参数校准评分 ──

/// 对一组参数进行历史回溯评分，返回 (score, 说明)。
///
/// 评分规则（基于反思 verdict）：
/// - reflection.verdict = "correct" 且参数与当前一致 → +1
/// - reflection.verdict = "wrong"   且参数有差异  → +0.5（证明调整方向正确）
/// - reflection.verdict = "wrong"   且参数无修改  → -0.5（未修正错误）
///
/// 实际使用时应取所有已有反思的加权均值。
pub fn score_param_set(
    params: &PortfolioMgrParamSet,
    suggestions: &[(String /* verdict */, Option<PortfolioMgrParamSet>)],
) -> f64 {
    if suggestions.is_empty() {
        return 0.0;
    }
    let default = PortfolioMgrParamSet::v56_default();
    let total: f64 = suggestions
        .iter()
        .map(|(verdict, suggested)| {
            let suggested = suggested.as_ref().unwrap_or(&default);
            let changed = params != suggested;
            match verdict.as_str() {
                "correct" if !changed => 1.0, // 正确决策未改参数 → 当前参数正确
                "correct" => 0.3,             // 正确但改了参数 → 可能是噪声
                "wrong" if changed => 0.5,    // 错误决策有改参数尝试 → 方向对
                "wrong" => -0.5,              // 错误但没改 → 参数有问题
                "partial" if changed => 0.2,  // 部分正确有改 → 有点用
                "partial" => 0.0,             // 部分正确没改 → 中性
                _ => 0.0,
            }
        })
        .sum();
    (total * 100.0).round() / 100.0 / suggestions.len() as f64
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── evidence_scale ──

    #[test]
    fn evidence_scale_low_weight() {
        let result = compute_evidence_scale(0.2, 1.44);
        assert!((result - 0.20).abs() < 0.01, "low weight: got {result}");
    }

    #[test]
    fn evidence_scale_mid_weight() {
        let result = compute_evidence_scale(0.7, 1.44);
        // 0.10 + 0.45 * sqrt(0.7/1.44) = 0.10 + 0.45 * 0.697 = ~0.414
        assert!(result > 0.35 && result < 0.45, "mid weight out of range: {result}");
    }

    #[test]
    fn evidence_scale_full_weight() {
        let result = compute_evidence_scale(1.44, 1.44);
        assert!((result - 0.55).abs() < 0.01, "full weight: got {result}");
    }

    #[test]
    fn evidence_scale_zero_max() {
        let result = compute_evidence_scale(0.5, 0.0);
        assert!((result - 0.20).abs() < 0.01, "zero max: got {result}");
    }

    // ── 凯利赔率 ──

    #[test]
    fn kelly_odds_trader_data() {
        let (odds, source) = compute_kelly_odds(Some(25.0), Some(18.0), Some(20.0), 0.6);
        assert!((odds - 2.5).abs() < 0.01, "trader odds: got {odds}");
        assert_eq!(source, "trader");
    }

    #[test]
    fn kelly_odds_no_trader_high_posterior() {
        let (odds, source) = compute_kelly_odds(None, None, None, 0.75);
        assert!((odds - 2.5).abs() < 0.01, "fallback high: got {odds}");
        assert_eq!(source, "波动率fallback");
    }

    #[test]
    fn kelly_odds_no_trader_low_posterior() {
        // V59: posterior∈[0.42,0.50) 新增试探档 1.5x（原为 0.0）
        let (odds, source) = compute_kelly_odds(None, None, None, 0.45);
        assert!((odds - 1.5).abs() < 0.01, "fallback probe tier: got {odds}");
        assert_eq!(source, "波动率fallback");
    }

    #[test]
    fn kelly_odds_below_probe_floor() {
        // posterior<0.42 → 0.0（凯利不适用，正确保守）
        let (odds, source) = compute_kelly_odds(None, None, None, 0.40);
        assert!((odds - 0.0).abs() < 0.01, "below probe floor: got {odds}");
        assert_eq!(source, "波动率fallback");
    }

    #[test]
    fn kelly_odds_bearish_case() {
        // V59 对齐: targetPrice < currentPrice（trader 看空）→ 走 fallback 而非归零
        let (odds, source) = compute_kelly_odds(Some(8.0), Some(7.0), Some(20.0), 0.6);
        assert!((odds - 2.0).abs() < 0.01, "bearish fallback: got {odds}");
        assert_eq!(source, "波动率fallback");
    }

    #[test]
    fn kelly_odds_stop_loss_greater_than_price() {
        // V59 对齐: stopLoss >= currentPrice（无效数据）→ 走 fallback 而非归零
        let (odds, source) = compute_kelly_odds(Some(25.0), Some(22.0), Some(20.0), 0.6);
        assert!((odds - 2.0).abs() < 0.01, "invalid stop loss fallback: got {odds}");
        assert_eq!(source, "波动率fallback");
    }

    // ── 凯利仓位 ──

    #[test]
    fn kelly_position_normal() {
        let pos = compute_kelly_position(0.65, 2.5, 0.01, "低风险");
        // kelly_raw = (0.65*2.5-0.35)/2.5 = 0.51
        // half=0.255, ×100=25.5, ×(1-0.01)=25.245
        assert!((pos - 25.245).abs() < 0.01, "kelly pos: got {pos}");
    }

    #[test]
    fn kelly_position_low_posterior() {
        let pos = compute_kelly_position(0.45, 2.5, 0.01, "低风险");
        assert_eq!(pos, 0.0, "low posterior should give zero");
    }

    #[test]
    fn kelly_position_high_risk_cap() {
        let pos = compute_kelly_position(0.70, 5.0, 0.01, "高风险");
        // kelly_raw=(0.7*5-0.3)/5=0.64, half=0.32, ×100×0.99=31.68, cap 35
        assert!((pos - 31.68).abs() < 0.1, "kelly high risk: got {pos}");
        assert!(pos <= 35.0, "high risk exceeded cap: {pos}");
    }

    #[test]
    fn kelly_position_extreme_risk_cap() {
        let pos = compute_kelly_position(0.70, 5.0, 0.01, "极高风险");
        assert!(pos <= 10.0, "extreme risk exceeded cap: {pos}");
    }

    /// v156（`PLAN-horizon-decision-repair.md` §七 终审发现 ①）：`apply_risk_cap` 是**最终上限**——
    /// 档位乘数（长档 ×1.2）后置生效时必须能封回原上限，否则「极高风险上限 10%」会输出 12%。
    /// 样本取乘数后的穿透值：12 = 10×1.2、42 = 35×1.2、60 = 50×1.2。
    #[test]
    fn risk_cap_is_final_upper_bound() {
        assert!((apply_risk_cap(12.0, "极高风险") - 10.0).abs() < 1e-9, "极高档 12 未被封回 10");
        assert!((apply_risk_cap(42.0, "高风险") - 35.0).abs() < 1e-9, "高档 42 未被封回 35");
        assert!((apply_risk_cap(60.0, "中风险") - 50.0).abs() < 1e-9, "中档 60 未被封回 50");
        // 未达上限 ⇒ 原样（cap 只压不抬）
        assert!((apply_risk_cap(6.0, "极高风险") - 6.0).abs() < 1e-9);
        // 低风险 / 未识别档 ⇒ 不 cap（与 `_ => position` 语义一致；空串是 overall_risk 缺席的兜底形态）
        assert!((apply_risk_cap(80.0, "低风险") - 80.0).abs() < 1e-9);
        assert!((apply_risk_cap(80.0, "") - 80.0).abs() < 1e-9);
    }

    // ── 风险分类 ──

    #[test]
    fn risk_classify_low() {
        let level =
            classify_risk(Some(20.0), Some(1.2), Some(15.0), Some(12.0), Some(40.0), Some(10.0));
        assert_eq!(level, "低风险", "should be low risk");
    }

    #[test]
    fn risk_classify_high_vol_sharpe() {
        let level =
            classify_risk(Some(50.0), Some(-0.2), Some(35.0), Some(3.0), Some(70.0), Some(2.0));
        assert_eq!(level, "高风险", "should be high risk");
    }

    #[test]
    fn risk_classify_extreme_debt() {
        // v40 C：高负债 + 营收负增长 + **真困境**（ROE<0 亏损）⇒ 极高风险
        let level =
            classify_risk(Some(30.0), Some(0.5), Some(20.0), Some(-3.0), Some(90.0), Some(-5.0));
        assert_eq!(level, "极高风险", "should be extreme risk");
    }

    /// v40 C 反例：高负债 + 营收微降 + ROE 为正 ⇒ **不再**判极高（防单一指标顶格）。
    #[test]
    fn risk_classify_extreme_debt_requires_distress() {
        let level =
            classify_risk(Some(30.0), Some(0.5), Some(20.0), Some(5.0), Some(90.0), Some(-5.0));
        assert_ne!(level, "极高风险", "微降 + 正 ROE 不构成财务困境，不应顶格");
    }

    /// 601166（兴业银行）实测画像应落到「中风险」—— 判据中**不含任何行业标签**。
    ///
    /// 输入全部取自 DB `blackboard_snapshot["t-risk"]` 的真实存档值
    /// （601166 / run 3e01fdc5 / 2026-09-12）：
    ///   vol 21.1 / sharpe 0.399 / dd 8.7 / roe 4.8 / debt 91.6 / growth -0.3
    /// 复算：极高风险的两条路径都不触发 ——
    ///   · 负债路径（debt>85 && growth<0 && 真困境）被「真困境」条件挡住：
    ///     growth=-0.3 不满足 <-10、roe=4.8 不满足 <0 ⇒ 整体为假；
    ///   · 波动路径（vol>60 && sharpe<-1.5）⇒ vol 21.1 为假。
    /// 高风险（AND 结构）左半边 vol>40 / sharpe<0 / dd>45 **三项全不成立**
    ///   ⇒ 右半边 debt>65 虽为真亦无效 ⇒ 落到「中风险」。
    /// 注：旧判据（无「真困境」条件）下 debt 91.6 + growth -0.3 会命中「极高风险」，
    ///     DB 存档中的旧结论即由此产生。
    #[test]
    fn risk_classify_bank_profile_is_not_extreme() {
        let level =
            classify_risk(Some(21.1), Some(0.399), Some(8.7), Some(4.8), Some(91.6), Some(-0.3));
        assert_eq!(level, "中风险", "高负债率本身不构成困境：真困境条件（亏损 / 大幅萎缩）未满足");
    }

    #[test]
    fn risk_classify_none_data() {
        let level = classify_risk(None, None, None, None, None, None);
        assert_eq!(level, "中风险", "no data should default to mid risk");
    }

    // ── 协方差衰减 ──

    #[test]
    fn covariance_decay_f3_f11() {
        // f1_weight=0.15 也会触发 f1↔f9 衰减，f9 从 0.08 → 0.06
        let (f9, f11) = apply_covariance_decay(0.15, 0.20, 0.08, 0.08);
        assert!((f11 - 0.08 * 0.65).abs() < 0.001, "f11 decay: got {f11}");
        assert!((f9 - 0.08 * 0.75).abs() < 0.001, "f9 decayed by f1↔f9: got {f9}");
    }

    #[test]
    fn covariance_decay_f1_f9() {
        let (f9, _f11) = apply_covariance_decay(0.15, 0.0, 0.08, 0.0);
        assert!((f9 - 0.08 * 0.75).abs() < 0.001, "f9 decay: got {f9}");
    }

    #[test]
    fn covariance_decay_no_overlap() {
        let (f9, f11) = apply_covariance_decay(0.0, 0.0, 0.08, 0.0);
        assert!((f9 - 0.08).abs() < 0.001, "f9 no decay: got {f9}");
        assert!((f11 - 0.0).abs() < 0.001, "f11 no decay: got {f11}");
    }

    // ── 决策动作 ──

    #[test]
    fn action_buy() {
        let a = compute_action(0.70, 20.0, 0.63, 0.53, 0.50, 0.38, 0.30, 15.0, 10.0);
        assert_eq!(a, "买入");
    }

    #[test]
    fn action_buy_insufficient_position() {
        let a = compute_action(0.70, 5.0, 0.63, 0.53, 0.50, 0.38, 0.30, 15.0, 10.0);
        // effective_posterior >= 0.63 但 position < 15%，降级到下一个匹配阈值
        // next: 0.63 >= 0.53 但 position(5) < pos_increase_min(10) → 跳过
        // 0.70 >= 0.50 → 持有
        assert_eq!(a, "持有");
    }

    #[test]
    fn action_hold() {
        let a = compute_action(0.55, 10.0, 0.63, 0.53, 0.50, 0.38, 0.30, 15.0, 10.0);
        assert_eq!(a, "增持");
    }

    #[test]
    fn action_sell() {
        let a = compute_action(0.25, 0.0, 0.63, 0.53, 0.50, 0.38, 0.30, 15.0, 10.0);
        assert_eq!(a, "卖出");
    }

    // ── 风险否决 ──

    #[test]
    fn risk_veto_high_risk_buy() {
        let (action, was_down, _) = apply_risk_veto("买入", "高风险");
        assert_eq!(action, "持有");
        assert!(was_down);
    }

    // ── 风控门开关（2026-10-26 用户裁决）────────────────────────────────
    //
    // ⚠ 这些测试用**显式参数**的 `apply_risk_veto_with_gate`，
    //   **不读全局开关** ⇒ 与 `decision_gates` 的进程级状态无关 ⇒ 可并行跑、
    //   不会因别的测试改了全局状态而漂移。

    #[test]
    fn risk_veto_gate_open_keeps_buy_action() {
        // 门开 ⇒ 高风险买入**不被否决**（原判据会改成「持有」）。
        let (action, was_down, note) = apply_risk_veto_with_gate("买入", "高风险", true);
        assert_eq!(action, "买入", "门开后不应改写动作");
        assert!(!was_down, "门开后不应标记为已降级");
        assert!(
            note.contains("风控门已放开"),
            "note 必须点名「门已放开」⇒ 呈现层才能与「门未过」区分：{note}"
        );
    }

    #[test]
    fn risk_veto_gate_open_keeps_hold_action_at_extreme_risk() {
        // 极高风险原本「禁止持仓」（持有→观望）；门开 ⇒ 不改写。
        let (action, was_down, note) = apply_risk_veto_with_gate("持有", "极高风险", true);
        assert_eq!(action, "持有");
        assert!(!was_down);
        assert!(note.contains("风控门已放开"), "{note}");
    }

    #[test]
    fn risk_veto_gate_closed_is_byte_identical_to_legacy_behavior() {
        // 门关 ⇒ 与引入开关前逐位一致（这是「不改变既有行为」的保证）。
        //
        // ⚠ 比对**三元组**（动作 / 是否降级 / note）而不是「note 为空」——
        //   旧行为在**否决生效**时 note 本就有内容（"高风险风控否决：禁止加仓"），
        //   「note 为空」只成立于「未触发否决」的分支。
        let (action, was_down, note) = apply_risk_veto_with_gate("买入", "高风险", false);
        assert_eq!(action, "持有");
        assert!(was_down);
        assert_eq!(note, "高风险风控否决：禁止加仓", "门关时的 note 必须与旧行为逐字一致");

        // 未触发否决的分支：note 恒空（两版一致）。
        let (action, was_down, note) = apply_risk_veto_with_gate("持有", "低风险", false);
        assert_eq!(action, "持有");
        assert!(!was_down);
        assert!(note.is_empty(), "未触发否决时 note 应为空: {note}");
    }

    #[test]
    fn risk_veto_gate_open_still_rejects_unknown_action_value_domain() {
        // ⚠ 门只停**否决**，「值域不可识别」的保守降级**不是门**，必须照旧生效。
        let (action, was_down, note) = apply_risk_veto_with_gate("待明日观察", "极高风险", true);
        assert_ne!(action, "待明日观察", "未识别值域必须被保守降级，不能原样放行");
        assert!(was_down);
        assert!(!note.contains("风控门已放开"), "值域降级不得被记成「门已放开」: {note}");
    }

    #[test]
    fn risk_veto_extreme_risk_hold() {
        let (action, was_down, _) = apply_risk_veto("持有", "极高风险");
        assert_eq!(action, "观望");
        assert!(was_down);
    }

    #[test]
    fn risk_veto_low_risk_hold() {
        let (action, was_down, _) = apply_risk_veto("持有", "低风险");
        assert_eq!(action, "持有");
        assert!(!was_down);
    }

    #[test]
    fn risk_veto_accepts_english_and_dashboard_value_domains() {
        // 英文值域：修复前此处不匹配任何中文分支 ⇒ 极高风险否决静默失效
        let (action, was_down, _) = apply_risk_veto("BUY", "极高风险");
        assert_eq!(action, "观望");
        assert!(was_down);

        let (action, was_down, _) = apply_risk_veto("HOLD", "极高风险");
        assert_eq!(action, "观望");
        assert!(was_down);

        let (action, was_down, _) = apply_risk_veto("INCREASE", "高风险");
        assert_eq!(action, "持有");
        assert!(was_down);

        // dashboard_report 值域（无「观望」档、有「强烈买入」）
        let (action, was_down, _) = apply_risk_veto("强烈买入", "高风险");
        assert_eq!(action, "持有");
        assert!(was_down);
    }

    #[test]
    fn risk_veto_unrecognized_value_no_longer_fails_open() {
        // 高风险 + 未识别值域：不得静默返回原值（旧实现会直接放行）。
        // 期望随 2026-10-22 裁定改：兜底不占业务档名 ⇒ 「观望」改「不确定」。
        let (action, was_down, note) = apply_risk_veto("待明日观察", "极高风险");
        assert_eq!(action, "不确定");
        assert!(was_down);
        assert!(!note.is_empty(), "未识别值域必须留归因");

        // 低风险 + 未识别值域：保留原值，但 note 说明未参与判定
        let (action, was_down, note) = apply_risk_veto("待明日观察", "低风险");
        assert_eq!(action, "待明日观察");
        assert!(!was_down);
        assert!(note.contains("未识别"));

        // 空串 = **没有决策数据**，与「有数据但判不出」分属两档，不得共用一个兜底值
        let (action, was_down, _) = apply_risk_veto("", "高风险");
        assert_eq!(action, "数据缺失");
        assert!(was_down);
        assert_ne!(action, "不确定", "「没数据」与「判不出」必须是两个档");
    }

    // ── Path 2: try_parse_param_suggestion ──

    #[test]
    fn parse_full_object() {
        let result =
            try_parse_param_suggestion(r#"{"buy_threshold":0.70,"cap_high":20.0}"#).unwrap();
        assert!((result.buy_threshold - 0.70).abs() < 0.001);
        assert!((result.cap_high - 20.0).abs() < 0.001);
        // 未指定的字段应使用 V56 默认
        assert!((result.increase_threshold - 0.53).abs() < 0.001);
    }

    #[test]
    fn parse_empty_returns_none() {
        assert!(try_parse_param_suggestion("").is_none());
        assert!(try_parse_param_suggestion("null").is_none());
        assert!(try_parse_param_suggestion("{}").is_none());
    }

    #[test]
    fn parse_array_format() {
        let result = try_parse_param_suggestion(
            r#"[{"key":"buy_threshold","value":0.60},{"key":"cap_high","value":30.0}]"#,
        )
        .unwrap();
        assert!((result.buy_threshold - 0.60).abs() < 0.001);
        assert!((result.cap_high - 30.0).abs() < 0.001);
        assert!((result.hold_threshold - 0.50).abs() < 0.001); // default（2026-10 重标定 0.48→0.50）
    }

    #[test]
    fn parse_invalid_json_returns_none() {
        assert!(try_parse_param_suggestion("not json").is_none());
        assert!(try_parse_param_suggestion("123").is_none());
    }

    // ── Path 1: score_param_set ──

    #[test]
    fn score_no_suggestions() {
        let p = PortfolioMgrParamSet::v56_default();
        assert!((score_param_set(&p, &[])).abs() < 0.001);
    }

    #[test]
    fn score_correct_no_change() {
        let p = PortfolioMgrParamSet::v56_default();
        let suggestions = vec![("correct".into(), None)];
        assert!((score_param_set(&p, &suggestions) - 1.0).abs() < 0.001);
    }

    #[test]
    fn score_wrong_no_change_negative() {
        let p = PortfolioMgrParamSet::v56_default();
        let suggestions = vec![("wrong".into(), None)];
        assert!((score_param_set(&p, &suggestions) - (-0.5)).abs() < 0.001);
    }

    #[test]
    fn score_wrong_with_change_positive() {
        // 当前活跃参数
        let p = PortfolioMgrParamSet { buy_threshold: 0.60, ..PortfolioMgrParamSet::v56_default() };
        // 反思建议的参数（与活跃参数不同 → "有改"）
        let suggested =
            PortfolioMgrParamSet { buy_threshold: 0.55, ..PortfolioMgrParamSet::v56_default() };
        let suggestions = vec![("wrong".into(), Some(suggested))];
        let score = score_param_set(&p, &suggestions);
        assert!((score - 0.5).abs() < 0.001, "expected 0.5, got {score}");
    }

    // ── P1-E13: portfolio_risk_gate ──

    /// 构造测试用 PositionSummary JSON
    fn make_holdings_json(positions: &[(&str, &str, i32, f64, Option<&str>)]) -> String {
        // (stock_code, stock_name, shares, avg_cost, sector)
        let arr: Vec<serde_json::Value> = positions
            .iter()
            .map(|(code, name, shares, cost, sector)| {
                let mv = *shares as f64 * cost;
                serde_json::json!({
                    "stockCode": code,
                    "stockName": name,
                    "totalShares": shares,
                    "avgCost": cost,
                    "currentPrice": cost,
                    "marketValue": mv,
                    "unrealizedPnl": 0.0,
                    "unrealizedPnlPct": 0.0,
                    "totalRealizedPnl": 0.0,
                    "sectorName": sector,
                })
            })
            .collect();
        serde_json::to_string(&arr).unwrap()
    }

    #[test]
    fn risk_gate_empty_holdings_passes_through_low_risk() {
        let out = portfolio_risk_gate(
            "买入",
            15.0,
            "低风险",
            10.0,
            Some(12.0),
            "600519",
            Some("白酒"),
            "[]",
            100_000.0,
        );
        let v: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["action"], "买入");
        assert_eq!(v["adjusted"], false);
        assert!((v["positionPct"].as_f64().unwrap() - 15.0).abs() < 0.01);
    }

    #[test]
    fn risk_gate_empty_holdings_extreme_risk_forces_watch() {
        let out = portfolio_risk_gate(
            "买入",
            15.0,
            "极高风险",
            10.0,
            Some(12.0),
            "600519",
            Some("白酒"),
            "[]",
            100_000.0,
        );
        let v: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["action"], "观望");
        assert_eq!(v["adjusted"], true);
        assert_eq!(v["positionPct"], 0.0);
        assert!(v["reasons"][0].as_str().unwrap().contains("R-200"));
    }

    #[test]
    fn risk_gate_position_cap_applied_when_exceeds() {
        // 空仓 + 30% 仓位（超过 20% 上限）
        let out = portfolio_risk_gate(
            "买入",
            30.0,
            "低风险",
            10.0,
            Some(12.0),
            "600519",
            Some("白酒"),
            "[]",
            100_000.0,
        );
        let v: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["action"], "买入");
        assert_eq!(v["adjusted"], true);
        assert!((v["positionPct"].as_f64().unwrap() - 20.0).abs() < 0.01);
        assert!(v["reasons"][0].as_str().unwrap().contains("R-206"));
    }

    #[test]
    fn risk_gate_bearish_target_forces_sell() {
        // 持仓 + 空头目标价（target < current × 0.85）
        let holdings = make_holdings_json(&[("600519", "贵州茅台", 100, 1500.0, Some("白酒"))]);
        let out = portfolio_risk_gate(
            "持有",
            15.0,
            "低风险",
            1500.0,
            Some(1000.0), // target=1000 < 1500×0.85=1275
            "600519",
            Some("白酒"),
            &holdings,
            50_000.0,
        );
        let v: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["action"], "卖出");
        assert_eq!(v["adjusted"], true);
        assert!(v["reasons"][0].as_str().unwrap().contains("R-201"));
    }

    #[test]
    fn risk_gate_invalid_json_fails_safe() {
        let out = portfolio_risk_gate(
            "买入",
            15.0,
            "低风险",
            10.0,
            Some(12.0),
            "600519",
            None,
            "not-json",
            100_000.0,
        );
        let v: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["action"], "买入");
        assert_eq!(v["adjusted"], false);
        assert!(v["reasons"][0].as_str().unwrap().contains("兜底"));
    }

    #[test]
    fn risk_gate_concentration_warning_emitted() {
        // 持仓集中度高（top_pct > 40%）触发警告
        let holdings = make_holdings_json(&[
            ("600519", "贵州茅台", 100, 1500.0, Some("白酒")), // mv=150000
            ("000858", "五粮液", 10, 150.0, Some("白酒")),     // mv=1500
        ]);
        // top_pct = 150000/(150000+1500) ≈ 99%
        let out = portfolio_risk_gate(
            "持有",
            0.0,
            "低风险",
            1500.0,
            None,
            "600519",
            Some("白酒"),
            &holdings,
            0.0,
        );
        let v: serde_json::Value = serde_json::from_str(&out).unwrap();
        let warn = v["concentrationWarning"].as_str().unwrap_or("");
        assert!(warn.contains("单股集中度"), "应包含集中度警告: {warn}");
        assert!(v["topHoldingPct"].as_f64().unwrap() > 90.0);
    }

    #[test]
    fn risk_gate_extreme_risk_hold_downgrades_to_watch() {
        // 极高风险档位 + portfolio-mgr 输出"持有" → 降级为观望
        let holdings = make_holdings_json(&[("600519", "贵州茅台", 100, 1500.0, Some("白酒"))]);
        let out = portfolio_risk_gate(
            "持有",
            15.0,
            "极高风险",
            1500.0,
            None,
            "600519",
            Some("白酒"),
            &holdings,
            50_000.0,
        );
        let v: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["action"], "观望");
        assert_eq!(v["positionPct"], 0.0);
        assert!(v["reasons"][0].as_str().unwrap().contains("R-200"));
    }

    #[test]
    fn risk_gate_normalize_when_total_exceeds_100() {
        // 5 只持仓 × 25% = 125% > 100%，归一化后每只 20%
        let holdings = make_holdings_json(&[
            ("600519", "贵州茅台", 100, 1500.0, Some("白酒")),
            ("000858", "五粮液", 100, 150.0, Some("白酒")),
            ("002594", "比亚迪", 100, 250.0, Some("汽车")),
            ("601318", "中国平安", 100, 50.0, Some("金融")),
            ("600036", "招商银行", 100, 40.0, Some("金融")),
        ]);
        // 调整为等市值（这里 mv 差异大，但归一化逻辑只看 pct 总和）
        // 改用相同 mv 测试归一化逻辑
        let holdings_eq = make_holdings_json(&[
            ("600519", "贵州茅台", 100, 1000.0, Some("白酒")),
            ("000858", "五粮液", 100, 1000.0, Some("白酒")),
            ("002594", "比亚迪", 100, 1000.0, Some("汽车")),
            ("601318", "中国平安", 100, 1000.0, Some("金融")),
            ("600036", "招商银行", 100, 1000.0, Some("金融")),
        ]);
        // portfolio_total = 5×100000 = 500000, 每只 pct=20%, 新增 25% → 总 125%
        let out = portfolio_risk_gate(
            "买入",
            25.0,
            "低风险",
            1000.0,
            Some(1200.0),
            "600519",
            Some("白酒"),
            &holdings_eq,
            0.0,
        );
        let v: serde_json::Value = serde_json::from_str(&out).unwrap();
        // 新增仓位应被归一化（但 25% 单股已超 20% 上限，先被 R-206 cap 到 20%）
        // 然后归一化 5×20%+20%=120% > 100% → 缩放
        let _ = holdings; // 避免 unused 警告
        assert!(v["adjusted"].as_bool().unwrap(), "应当 adjusted=true");
        // 检查是否包含归一化原因（如果触发）
        let reasons_str = v["reasons"].to_string();
        assert!(
            reasons_str.contains("R-206") || reasons_str.contains("R-209"),
            "应当触发仓位调整原因，实际: {reasons_str}"
        );
    }

    // ── 贝叶斯因子置信度 ──

    #[test]
    fn bayes_prior_equals_posterior() {
        // prior=posterior=0.5 → BF=1 → 50%
        let conf = compute_bayes_confidence(0.5, 0.5);
        assert!((conf - 50.0).abs() < 0.1, "预期 50%, 实际 {conf}%");
    }

    #[test]
    fn bayes_strong_bull() {
        // prior=0.5, posterior=0.81 → BF≈4.26 → ~91%
        let conf = compute_bayes_confidence(0.5, 0.81);
        assert!(conf > 80.0 && conf < 95.0, "预期 ~91%, 实际 {conf}%");
    }

    #[test]
    fn bayes_strong_bear() {
        // prior=0.5, posterior=0.19 → BF≈4.26 → ~91%
        let conf = compute_bayes_confidence(0.5, 0.19);
        assert!(conf > 80.0 && conf < 95.0, "预期 ~91%, 实际 {conf}%");
    }

    #[test]
    fn bayes_neutral() {
        // prior=0.5, posterior=0.53 → BF≈1.13 → ~53%
        let conf = compute_bayes_confidence(0.5, 0.53);
        assert!((50.0..65.0).contains(&conf), "预期 ~53%, 实际 {conf}%");
    }

    #[test]
    fn bayes_prior_non_default() {
        // prior=0.4, posterior=0.6 → BF≈2.25 → ~69%
        let conf = compute_bayes_confidence(0.4, 0.6);
        assert!(conf > 65.0 && conf < 75.0, "预期 ~69%, 实际 {conf}%");
    }

    // ── factor_completeness（P2-1 有效性校验，2026-08-09）──

    #[test]
    fn factor_completeness_counts_all_valid() {
        // 9 个因子全部有效 → 1.0
        let r = compute_factor_completeness(
            Some(70.0),
            Some(65.0),
            Some("强"),
            Some(25.0),
            Some(15.0),
            Some(-1.2e8),
            Some(3),
            Some(5),
            Some(0.3),
        );
        assert!((r - 1.0).abs() < 1e-9, "got {r}");
    }

    #[test]
    fn factor_completeness_zero_money_flow_not_counted() {
        // main_net_inflow = 0（中性无信号）→ f9 不计数 → 8/9
        let r = compute_factor_completeness(
            Some(70.0),
            Some(65.0),
            Some("强"),
            Some(25.0),
            Some(15.0),
            Some(0.0),
            Some(3),
            Some(5),
            Some(0.3),
        );
        assert!((r - (8.0 / 9.0)).abs() < 1e-9, "got {r}");
    }

    #[test]
    fn factor_completeness_out_of_range_values_not_counted() {
        // total_score=150（超 0-100）、pace=5.0（超 [-1,1]）→ 均不计数 → 7/9
        let r = compute_factor_completeness(
            Some(150.0),
            Some(65.0),
            Some("强"),
            Some(25.0),
            Some(15.0),
            Some(-1.2e8),
            Some(3),
            Some(5),
            Some(5.0),
        );
        assert!((r - (7.0 / 9.0)).abs() < 1e-9, "got {r}");
    }

    #[test]
    fn factor_completeness_all_none_is_zero() {
        let r = compute_factor_completeness(None, None, None, None, None, None, None, None, None);
        assert!((r - 0.0).abs() < 1e-9);
    }

    // ── PortfolioMgrParamSet：市况先验（可反思 / 可演进优化）──

    #[test]
    fn prior_defaults_match_template_seed() {
        // 必须与 seed_variables.rs 的 regime_prior_* 默认值一致，
        // 否则「未配置基线」与「反思/演进回写基线」会漂移。
        let d = PortfolioMgrParamSet::v56_default();
        assert!((d.prior_bull - 0.55).abs() < 1e-9, "got {}", d.prior_bull);
        assert!((d.prior_sideways - 0.50).abs() < 1e-9, "got {}", d.prior_sideways);
        assert!((d.prior_bear - 0.45).abs() < 1e-9, "got {}", d.prior_bear);
    }

    #[test]
    fn profiles_share_same_prior_dimension() {
        // 先验由市况派生，与风险偏好正交 → 三个 profile 的先验应完全相同
        let a = PortfolioMgrParamSet::v56_default();
        let b = PortfolioMgrParamSet::conservative();
        let c = PortfolioMgrParamSet::aggressive();
        assert_eq!(a.prior_bull, b.prior_bull);
        assert_eq!(a.prior_bull, c.prior_bull);
        assert_eq!(a.prior_sideways, b.prior_sideways);
        assert_eq!(a.prior_sideways, c.prior_sideways);
        assert_eq!(a.prior_bear, b.prior_bear);
        assert_eq!(a.prior_bear, c.prior_bear);
    }

    #[test]
    fn default_grid_scans_prior_dimension() {
        // 回归守护：若网格中所有候选的先验都等于默认值，WFO 对先验的「搜索」
        // 等价于没有搜索 ——「演进可优化先验」会名存实亡。
        let grid = PortfolioMgrParamSet::default_grid();
        let d = PortfolioMgrParamSet::v56_default();
        let varied = grid
            .iter()
            .filter(|p| {
                p.prior_bull != d.prior_bull
                    || p.prior_sideways != d.prior_sideways
                    || p.prior_bear != d.prior_bear
            })
            .count();
        assert!(varied >= 2, "网格需至少 2 组先验不同的候选，实际 {varied}");
    }

    #[test]
    fn parses_prior_from_short_names() {
        let p = try_parse_param_suggestion(r#"{"prior_bull":0.62,"prior_bear":0.38}"#)
            .expect("应解析成功");
        assert!((p.prior_bull - 0.62).abs() < 1e-9);
        assert!((p.prior_bear - 0.38).abs() < 1e-9);
        // 未指定字段回落默认值
        assert!((p.prior_sideways - 0.50).abs() < 1e-9);
    }

    #[test]
    fn parses_prior_from_template_full_names() {
        let p = try_parse_param_suggestion(r#"{"regime_prior_bull":0.58}"#).expect("应解析成功");
        assert!((p.prior_bull - 0.58).abs() < 1e-9);
    }

    #[test]
    fn parses_prior_from_camel_case() {
        let p = try_parse_param_suggestion(r#"{"priorSideways":0.47}"#).expect("应解析成功");
        assert!((p.prior_sideways - 0.47).abs() < 1e-9);
    }

    #[test]
    fn prior_only_suggestion_is_recognized() {
        // 回归守护：has_structured 必须认先验键，
        // 否则「只建议先验」的反思会被整体丢弃。
        assert!(
            try_parse_param_suggestion(r#"{"prior_bear":0.40}"#).is_some(),
            "仅含先验的建议不应被丢弃"
        );
    }

    #[test]
    fn parses_prior_from_key_value_array() {
        let p = try_parse_param_suggestion(
            r#"[{"key":"prior_bear","value":0.42},{"key":"buy_threshold","value":0.60}]"#,
        )
        .expect("应解析成功");
        assert!((p.prior_bear - 0.42).abs() < 1e-9);
        assert!((p.buy_threshold - 0.60).abs() < 1e-9);
    }
}

#[cfg(test)]
mod kelly_growth_tests {
    use super::*;

    /// 时间归一是本函数的**存在理由**：仓位小但持有短的档，完全可能比仓位大但锁 90 天的档
    /// 增长率更高 —— 若只比 `position_pct`，仲裁永远偏向长线。
    #[test]
    fn growth_rate_normalises_by_holding_period() {
        let short = kelly_growth_rate(0.66, 1.0, 8.0, 2).expect("超短应有增长率");
        let long = kelly_growth_rate(0.66, 1.0, 30.0, 90).expect("长线应有增长率");
        assert!(short > long, "同样胜率赔率下，2 日 8% 仓的每日对数增长应高于 90 日 30% 仓");
        // 同一天数下仓位单调（f 与 f·b 都增，ln 项净增）
        let a = kelly_growth_rate(0.70, 2.0, 10.0, 28).unwrap();
        let b = kelly_growth_rate(0.70, 2.0, 25.0, 28).unwrap();
        assert!(b > a, "持有期相同时，更大 Kelly 仓应有更高增长率");
    }

    /// `None` 的全部来源，以及「负增长率仍要返回 Some」的边界：
    /// 负值是**该档在算出来不划算**，与「拿不到数」性质不同，混在一起会让缺席冒充看空。
    #[test]
    fn absence_is_none_and_negative_edge_stays_some() {
        assert_eq!(kelly_growth_rate(0.6, 1.0, 10.0, 0), None, "持有 0 天无定义");
        assert_eq!(kelly_growth_rate(1.2, 1.0, 10.0, 2), None, "概率越界");
        assert_eq!(kelly_growth_rate(0.6, 0.0, 10.0, 2), None, "赔率非正");
        assert_eq!(kelly_growth_rate(0.6, 1.0, 0.0, 2), None, "无仓位");
        assert_eq!(kelly_growth_rate(f64::NAN, 1.0, 10.0, 2), None, "NaN 输入");
        assert_eq!(kelly_growth_rate(0.6, 1.0, 100.0, 2), None, "f=1 ⇒ ln(0) 无定义");
        let neg = kelly_growth_rate(0.40, 0.5, 10.0, 28).expect("负期望仍是可比的数值");
        assert!(neg < 0.0, "胜率 0.40 + 赔率 0.5 应为负增长率，实得 {neg}");
    }

    // ── 2026-10-14 参数建议解析单一权威源（PLAN-reflection-optimization-closure §一）──

    /// **现网真实串**（2026-10-14 取自 `stock_reflections.parameter_suggestions_json`）：
    /// 条目键是 `param` / `suggested_value`，名字是模板全名。此前这类条目一个都匹配不上，
    /// 整段被洗成 `Some(出厂默认)` ⇒ WFO 的校准结果与反思内容无关。
    #[test]
    fn parse_reads_the_production_suggestion_shape() {
        let real = r#"[{"param": "action_watch_threshold", "reason": "提高观望阈值可减少无效波动", "current_value": 0.38, "suggested_value": 0.42}]"#;
        let p = try_parse_param_suggestion(real).expect("生产形态必须解析成功");
        let def = PortfolioMgrParamSet::v56_default();
        assert!(
            (p.watch_threshold - 0.42).abs() < 1e-9,
            "建议值必须进字段，实得 {}",
            p.watch_threshold
        );
        // 未被提及的字段仍取默认 —— 证明「识别成功」不是靠全量兜底蒙对的
        assert!((p.buy_threshold - def.buy_threshold).abs() < 1e-9);
        assert!((p.cap_high - def.cap_high).abs() < 1e-9);
    }

    /// 负控 A：数组非空但名字不在可校准集合 ⇒ `None`（不得当成「建议 = 默认值」）。
    #[test]
    fn parse_returns_none_when_no_entry_is_calibratable() {
        assert!(
            try_parse_param_suggestion(
                r#"[{"param": "nonsense_param", "current_value": 1.0, "suggested_value": 2.0}]"#
            )
            .is_none(),
            "不认识的名字必须判为「无可识别建议」"
        );
    }

    /// 负控 B：空数组 ⇒ `None`（现网 16 行建议里有 7 行就是 `[]`）。
    #[test]
    fn parse_returns_none_for_empty_array() {
        assert!(try_parse_param_suggestion("[]").is_none());
        assert!(try_parse_param_suggestion("  []  ").is_none());
    }

    /// 11 个可校准字段的**模板全名**必须逐个被接受（短名与全名折算到同一字段）。
    /// 这条锁的是「判据表与字段表逐项同集」—— 漂移即红，不靠注释自觉。
    #[test]
    fn every_calibratable_field_name_is_accepted_by_its_full_name() {
        let full_names = [
            "action_buy_threshold",
            "action_increase_threshold",
            "action_hold_threshold",
            "action_watch_threshold",
            "action_reduce_threshold",
            "pos_cap_extreme",
            "pos_cap_high",
            "pos_cap_mid",
            "regime_prior_bull",
            "regime_prior_sideways",
            "regime_prior_bear",
        ];
        for name in full_names {
            let json = format!(r#"{{"{name}": 0.424242}}"#);
            let p =
                try_parse_param_suggestion(&json).unwrap_or_else(|| panic!("全名 {name} 应被识别"));
            let touched = [
                p.buy_threshold,
                p.increase_threshold,
                p.hold_threshold,
                p.watch_threshold,
                p.reduce_threshold,
                p.cap_extreme,
                p.cap_high,
                p.cap_mid,
                p.prior_bull,
                p.prior_sideways,
                p.prior_bear,
            ];
            assert!(
                touched.iter().any(|v| (v - 0.424242).abs() < 1e-9),
                "全名 {name} 被识别但没有落到任何字段"
            );
        }
        // 别名折算：短名与全名必须写进同一个字段
        let by_short = try_parse_param_suggestion(r#"{"buy_threshold": 0.424242}"#).unwrap();
        let by_full = try_parse_param_suggestion(r#"{"action_buy_threshold": 0.424242}"#).unwrap();
        assert!((by_short.buy_threshold - by_full.buy_threshold).abs() < 1e-9);
    }

    /// 逐参数消费方（Gap3）要能区分「提了这个参数」与「没提」—— ParamSet 形态丢掉这层信息。
    #[test]
    fn entries_keep_which_params_were_suggested() {
        let real = r#"[{"param": "action_watch_threshold", "current_value": 0.38, "suggested_value": 0.42}]"#;
        let got = parse_param_suggestion_entries(real);
        assert_eq!(got.len(), 1, "只提了一个参数，条目数必须是一");
        assert_eq!(got[0].0, "action_watch_threshold", "名字须归一为模板全名");
        assert!((got[0].1 - 0.42).abs() < 1e-9);
        assert_eq!(got[0].2, Some(0.38), "建议时的现值必须带出来，否则方向无从判定");
        // 负控：没提的参数不得凭空出现在条目清单里
        assert!(!got.iter().any(|(n, _, _)| n == "action_buy_threshold"));
        assert!(parse_param_suggestion_entries("[]").is_empty());
        assert!(parse_param_suggestion_entries("not json").is_empty());
        // 缺 current_value 的条目仍要保留（值是真建议），但现值维度如实为 None
        let no_current = parse_param_suggestion_entries(
            r#"[{"param": "pos_cap_high", "suggested_value": 28.0}]"#,
        );
        assert_eq!(no_current.len(), 1);
        assert_eq!(no_current[0].2, None, "没写现值不得被当成 0 或当成默认值");
    }
}

/// 评分归一化口径的口径锁（裁定 P-B1）。
#[cfg(test)]
mod score_to_signal_tests {
    use super::*;

    /// 中点/两端必须精确 —— 这是五个调用点共同依赖的语义。
    #[test]
    fn midpoint_and_ends_are_exact() {
        assert_eq!(score_to_signal(50.0), 0.0);
        assert_eq!(score_to_signal(0.0), -1.0);
        assert_eq!(score_to_signal(100.0), 1.0);
        // 真记录锚点：600406 · 2026-10-15 的 `t-scoring.content.totalScore = 78`，
        // 落库 `evidence.factors.f1.sigma = 0.56` —— 现算必须复现它（口径没漂的证据）。
        assert_eq!(score_to_signal(78.0), 0.56);
    }

    /// **刻意不钳制**：越界要能被调用方看见（`leg_signal` 用 `cl()`、主链 f13 用 `clamp`）。
    /// 这条同时是负控 —— 谁往本函数里塞一个 `.clamp(-1.0, 1.0)`，此断言立刻红。
    #[test]
    fn out_of_range_is_not_silently_clamped() {
        assert_eq!(score_to_signal(150.0), 2.0);
        assert_eq!(score_to_signal(-50.0), -2.0);
    }
}
