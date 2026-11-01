//! R2 组合监控主模块
//!
//! 5 个职责：
//! 1. `compute_dashboard` —— 组合聚合指标（5 个 metric card + 行业集中度 + 警告）
//! 2. `compute_correlation_matrix` —— 两两相关性（最多 20 只持仓一次算完）
//! 3. `run_stress_scenario` —— 3 场景压测（大盘 -10% / -20% / 黑天鹅）
//! 4. `refresh_metrics` —— 把当前快照写库
//! 5. `get_dashboard` / `get_timeline` —— 读快照（时间旅行按 as_of_date 走）
//!
//! 时间旅行：as_of_date 存在时，从 portfolio_metrics_daily 表查 <= as_of_date 的最新一行；
//! 缺数据返回默认空 dashboard（带 `isHistorical: true` 标记）。

use std::collections::HashMap;

use sea_orm::ActiveModelTrait;
use sea_orm::ColumnTrait;
use sea_orm::DatabaseConnection;
use sea_orm::EntityTrait;
use sea_orm::QueryFilter;
use sea_orm::QueryOrder;
use sea_orm::QuerySelect;
use sea_orm::Set;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use axagent_harness::market_data::AdjType;
use axagent_harness::market_data::KLine;
use axagent_harness::market_data::MarketDataProvider;

use super::trading::PositionSummary;

// ── 数据结构 ──

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PortfolioDashboard {
    /// 是否为历史快照（time travel 下为 true）
    pub is_historical: bool,
    pub as_of_date: Option<String>,
    pub total_market_value: f64,
    pub total_pnl: f64,
    pub total_pnl_pct: f64,
    /// 拿不到现价的持仓只数（`market_value` 为 `None`）。
    ///
    /// 必须跟着上屏：风险等级与分散度现在按**已定价的只数**判，而面板的持仓表仍列全部行
    /// ⇒ 「按 5 只算出的分散度 70」配着「8 行持仓」显示就是两套口径，这个数用来把差额说清楚。
    pub unpriced_positions: u32,
    /// 现金占总资产百分比。**当前无数据来源，恒 0**（`cash_pct` 那一列照旧落库 0）：
    /// 全仓没有任何地方存账户现金，唯一像是入口的工作流变量 `portfolio_cash` 注的是常量 0.0。
    /// 见 PLAN `PLAN-monitor-chain-closure.md` 裁定 8 —— 在拿到来源之前这一格不进面板。
    pub cash_pct: f64,
    /// 由 [`refresh_metrics`] 从历史净值序列补算（G-3）；[`compute_dashboard`] 自己
    /// 手上没有历史，默认 0.0。
    pub max_drawdown_pct: f64,
    pub beta: Option<f64>,
    pub sharpe_30d: Option<f64>,
    pub correlation_avg: Option<f64>,
    pub top_concentration_pct: f64,
    pub sector_exposure: HashMap<String, f64>,
    pub concentration_warning: Option<String>,
    pub risk_level: String,
    pub diversification_score: u32,
    pub stress_test: StressTestBundle,
    pub positions: Vec<PositionSummary>,
    pub snapshot_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct StressTestBundle {
    pub m10: Option<StressTestResult>,
    pub m20: Option<StressTestResult>,
    pub black_swan: Option<StressTestResult>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StressTestResult {
    pub scenario: String,
    pub label: String,
    /// 组合整体 P&L 估值（元）
    pub portfolio_pnl: f64,
    /// 组合整体 P&L 百分比
    pub portfolio_pnl_pct: f64,
    /// 受影响最大的持仓（按 code / name / pnl_pct）
    pub top_hit: Option<PositionHit>,
    pub note: String,
    /// beta 取值来源统计 —— 让「有几只标的其实只是回退到了中性 1.0」可见。
    ///
    /// ⚠️ 必须带 `#[serde(default)]`：历史快照
    /// （`portfolio_metrics_daily.stress_test_json`）里没有这个键，缺省会让
    /// `get_dashboard` 的反序列化**整段失败**，而该处用 `.ok()` 吞错 ⇒ 静默退化成
    /// `StressTestBundle::default()`，等于**整个压测数据凭空消失**。
    #[serde(default)]
    pub beta_provenance: BetaProvenance,
}

/// beta 取值来源统计（2026-09-14 新增，配套「真实历史 beta」替换行业查表）
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct BetaProvenance {
    /// 用真实历史 beta 估计的持仓数
    pub historical: usize,
    /// 回退到中性默认值（`DEFAULT_BETA`）的持仓数
    pub fallback: usize,
    /// 回退标的的代码 —— 便于直接定位「是谁缺历史数据」，不必再靠猜
    pub fallback_codes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PositionHit {
    pub stock_code: String,
    pub stock_name: String,
    pub pnl_pct: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CorrelationCell {
    pub code_a: String,
    pub code_b: String,
    pub correlation: f64,
}

// ── 纯函数：可独立测试 ──

/// 把"组合净值水平序列"折算成最大回撤（百分比），复用 `crate::risk::peak_trough_drawdown`。
///
/// ⚠ 入参是**水平值**（市值/净值本身，如 100 → 120 → 88），不是百分比序列 ——
/// 原来的参数名 `equity_curve_pct` 会让人误以为要先除以 100，套错输入得到的是假数。
pub fn compute_max_drawdown_pct(equity_curve: &[f64]) -> f64 {
    // peak_trough_drawdown 返回 0~1 比例，此处转为百分比
    crate::risk::peak_trough_drawdown(equity_curve) * 100.0
}

/// Sharpe ratio（年化）—— 复用 `crate::risk::sharpe_components`（样本方差 n-1）。
pub fn compute_sharpe(returns_pct: &[f64], annualization: f64) -> Option<f64> {
    if returns_pct.len() < 5 {
        return None;
    }
    // sharpe_components 返回 (sharpe, annualized, mean_return, stddev)，取 annualized
    Some(crate::risk::sharpe_components(returns_pct, 0.0, annualization).1)
}

/// Pearson 相关系数（长度必须一致，>5 个点）
///
/// 数学实现复用 [`axagent_harness::indicators::pearson`]（禁区 12：本仓此前在
/// `hit_rate_backtest` 另有一份）；本函数只保留自己的**门槛**：少于 5 点在本模块
/// 的用途（组合相关性）下不足以谈相关。
pub fn pearson_correlation(x: &[f64], y: &[f64]) -> Option<f64> {
    if x.len() != y.len() || x.len() < 5 {
        return None;
    }
    axagent_harness::indicators::pearson(x, y)
}

/// 纯函数：按**日期交集**对齐两条「日期 → 日收益」序列，再求相关。
///
/// 取代原先的「取较短者的尾部」：两只票的停牌日/上市日不同 ⇒ 尾部对齐会把 A 的第 3 个
/// 交易日和 B 的第 5 个交易日算成同一天，相关系数照常有值、却是错位的。本文件的
/// `beta_from_aligned` 早就按日期交集对齐，`refresh_correlation` 却用另一套 ——
/// 同一模块内两个组合级指标各按一套对齐规则（禁区 12 的同一形态）。
/// 样本是否够用由 [`pearson_correlation`] 自己的门槛判（不足 5 点 ⇒ `None`）。
pub fn aligned_pearson(a: &[(String, f64)], b: &[(String, f64)]) -> Option<f64> {
    let by_date_b: HashMap<&str, f64> = b.iter().map(|(d, r)| (d.as_str(), *r)).collect();
    let mut xa: Vec<f64> = Vec::new();
    let mut xb: Vec<f64> = Vec::new();
    for (d, r) in a {
        if let Some(other) = by_date_b.get(d.as_str()) {
            xa.push(*r);
            xb.push(*other);
        }
    }
    pearson_correlation(&xa, &xb)
}

/// 组合 beta = Cov(组合, 市场) / Var(市场)
pub fn compute_beta(portfolio_returns: &[f64], market_returns: &[f64]) -> Option<f64> {
    if portfolio_returns.len() != market_returns.len() || portfolio_returns.len() < 10 {
        return None;
    }
    let n = portfolio_returns.len() as f64;
    let mp = portfolio_returns.iter().sum::<f64>() / n;
    let mm = market_returns.iter().sum::<f64>() / n;
    let mut num = 0.0;
    let mut dm2 = 0.0;
    for (a, b) in portfolio_returns.iter().zip(market_returns.iter()) {
        let dp = a - mp;
        let dm = b - mm;
        num += dp * dm;
        dm2 += dm * dm;
    }
    if dm2 < 1e-9 { None } else { Some(num / dm2) }
}

/// 集中度（refactored from portfolio_risk::compute_from_positions）
///
/// 口径：**分母 = 拿到现价的持仓市值之和**。写成显式 `filter_map` 而不是原来的
/// `unwrap_or(0.0)` —— 两者数值相同（往分母里加 0 不改变任何一只的占比），所以
/// **这一处原先不是缺陷**；真正的口径缺陷在「按几只算」的只数 `n` 与盈亏的分母，
/// 见 [`compute_dashboard`]。这里只是让「分母只含已定价」由构造说明，不靠 0 的巧合。
pub fn compute_concentration(positions: &[PositionSummary]) -> (f64, HashMap<String, f64>, f64) {
    let priced: Vec<f64> = positions.iter().filter_map(|p| p.market_value).collect();
    let total_mv: f64 = priced.iter().sum();
    let max_mv = priced.into_iter().fold(0.0_f64, f64::max);
    let top_pct = if total_mv > 0.0 {
        (max_mv / total_mv) * 100.0
    } else {
        0.0
    };

    let mut sector: HashMap<String, f64> = HashMap::new();
    for p in positions {
        if let (Some(mv), Some(s)) = (p.market_value, &p.sector_name)
            && !s.is_empty()
            && total_mv > 0.0
        {
            *sector.entry(s.clone()).or_default() += (mv / total_mv) * 100.0;
        }
    }
    let max_sector_pct = sector.values().cloned().fold(0.0_f64, f64::max);
    (top_pct, sector, max_sector_pct)
}

/// 拿不到现价的持仓数（`market_value` 为 `None`）。
///
/// 这个数必须跟着走：把缺价从分母里剔掉之后，「按 8 只算出的集中度」不再对应
/// 「8 只持仓」——不报出来就是换了口径却不告知。
pub fn count_unpriced(positions: &[PositionSummary]) -> usize {
    positions.iter().filter(|p| p.market_value.is_none()).count()
}

/// 风险等级（与 portfolio_risk 对齐）
pub fn compute_risk_level(top_pct: f64, max_sector_pct: f64, n: usize) -> String {
    if n == 0 {
        return "无持仓".to_string();
    }
    if top_pct > 50.0 || max_sector_pct > 60.0 {
        "高风险".to_string()
    } else if top_pct > 30.0 || max_sector_pct > 40.0 {
        "中高风险".to_string()
    } else if top_pct > 20.0 || max_sector_pct > 30.0 {
        "中等风险".to_string()
    } else {
        "低风险".to_string()
    }
}

/// 分散度评分 0-100
pub fn compute_diversification_score(n: usize, top_pct: f64, max_sector_pct: f64) -> u32 {
    if n >= 8 && top_pct <= 15.0 && max_sector_pct < 30.0 {
        90
    } else if n >= 5 && top_pct <= 25.0 && max_sector_pct < 40.0 {
        70
    } else if n >= 3 && top_pct <= 35.0 {
        50
    } else if n >= 1 {
        30
    } else {
        0
    }
}

/// 集中度警告文本
pub fn compute_concentration_warning(
    top_pct: f64,
    max_sector_pct: f64,
    n: usize,
) -> Option<String> {
    let mut warns = Vec::new();
    if top_pct > 40.0 {
        warns.push(format!("单股集中度 {:.0}% 过高，建议 ≤30%", top_pct));
    } else if top_pct > 30.0 {
        warns.push(format!("单股集中度 {:.0}% 偏高，关注分散风险", top_pct));
    }
    if max_sector_pct > 50.0 {
        warns.push(format!("行业暴露 {:.0}% 过高，建议 ≤40%", max_sector_pct));
    }
    if n < 3 && n > 0 {
        warns.push(format!("仅 {} 只持仓，分散度不足，建议 ≥3 只", n));
    }
    if warns.is_empty() {
        None
    } else {
        Some(warns.join("；"))
    }
}

/// 压测：单股 i 在 scenario 下预计跌幅 = β_i × market_drop
///
/// **β 来源（2026-09-14 变更）**：`betas` 由 `estimate_betas` 提供 —— 用该股自己的
/// 日收益对沪深300日收益做回归得到的**真实历史 beta**；不可得时回退中性 1.0，
/// 并在 `beta_provenance` 里记账。此处**不再按 `sector_name` 查表**。
///
/// 修复 P2-10: 原代码接收 `sector_exposure` 参数却完全未使用（`let _ = sector_exposure`），
/// 行业集中度风险被忽略。改为：当某行业暴露占比超过阈值（40%）时，该行业持仓的
/// 损失放大 1.2 倍——行业越集中，下跌时踩踏越严重（流动性折价 + 相关性坍缩）。
/// 返回组合总 P&L / P&L% / 受损最大持仓 / beta 来源统计
pub fn run_stress_scenario(
    positions: &[PositionSummary],
    sector_exposure: &HashMap<String, f64>,
    betas: &BetaMap,
    scenario: StressScenario,
) -> StressTestResult {
    // G-1：与集中度同口径 —— 缺现价的持仓不参与压测（它没有市值可放大，
    // 按 0 计入只会让「组合下跌 x%」的百分比对着一个被削小的分母）。
    let priced: Vec<&PositionSummary> =
        positions.iter().filter(|p| p.market_value.is_some()).collect();
    let total_mv: f64 = priced.iter().map(|p| p.market_value.unwrap_or(0.0)).sum();
    if total_mv <= 0.0 || priced.is_empty() {
        return StressTestResult {
            scenario: scenario.code().to_string(),
            label: scenario.label().to_string(),
            portfolio_pnl: 0.0,
            portfolio_pnl_pct: 0.0,
            top_hit: None,
            note: "无持仓，跳过压测".to_string(),
            beta_provenance: BetaProvenance::default(),
        };
    }
    let market_drop = scenario.market_drop();
    let mut total_pnl = 0.0;
    let mut worst_hit: Option<(f64, &PositionSummary)> = None;
    let mut concentration_penalty_applied = false;
    let mut provenance = BetaProvenance::default();
    for p in priced {
        let mv = p.market_value.unwrap_or(0.0);
        // β 取自该股对沪深300的**真实历史回归**（`estimate_betas`）；无估计值时回退
        // 中性 1.0 并记账。**不再读 `p.sector_name` 查表** —— 见文件内
        // 「真实历史 beta 估计」区块的说明。
        let sector = p.sector_name.as_deref().unwrap_or("");
        let est = betas.get(&p.stock_code).copied();
        let base_beta = est.map(|e| e.beta).unwrap_or(DEFAULT_BETA);
        if est.map(|e| e.historical).unwrap_or(false) {
            provenance.historical += 1;
        } else {
            provenance.fallback += 1;
            provenance.fallback_codes.push(p.stock_code.clone());
        }
        // 行业集中度惩罚：`sector_exposure[sector] > 40%` 时放大 beta。
        // ⚠️ 这与「按行业归属决定个股判据」**语义不同**：观测对象是「组合在该行业的
        //    暴露占比」这一**组合级事实**（越集中，下跌时踩踏越重：流动性折价 +
        //    相关性坍缩），不改变任何个股的独立判断，且占比不超阈值时完全不生效。
        let sector_pct = sector_exposure.get(sector).copied().unwrap_or(0.0);
        let adjusted_beta = if sector_pct > SECTOR_CONCENTRATION_THRESHOLD {
            concentration_penalty_applied = true;
            base_beta * SECTOR_CONCENTRATION_PENALTY
        } else {
            base_beta
        };
        let pct = adjusted_beta * market_drop * 100.0;
        let pnl = mv * adjusted_beta * market_drop;
        total_pnl += pnl;
        if worst_hit.as_ref().map(|(w, _)| pct < *w).unwrap_or(true) {
            worst_hit = Some((pct, p));
        }
    }
    let top = worst_hit.map(|(_, p)| PositionHit {
        stock_code: p.stock_code.clone(),
        stock_name: p.stock_name.clone(),
        pnl_pct: worst_hit.as_ref().map(|(w, _)| *w).unwrap_or(0.0),
    });
    let mut note = if provenance.fallback == 0 {
        format!(
            "线性近似：单股跌幅 = β_i（{} 日真实历史 beta，对沪深300回归）× 大盘跌幅",
            BETA_LOOKBACK_DAYS
        )
    } else {
        format!(
            "线性近似：单股跌幅 = β_i × 大盘跌幅；{}/{} 只无足够历史样本，β 回退中性 {}",
            provenance.fallback,
            positions.len(),
            DEFAULT_BETA
        )
    };
    if concentration_penalty_applied {
        note.push_str("；行业暴露>40% 的持仓 beta × 1.2（流动性折价 + 相关性坍缩）");
    }
    StressTestResult {
        scenario: scenario.code().to_string(),
        label: scenario.label().to_string(),
        portfolio_pnl: total_pnl,
        portfolio_pnl_pct: (total_pnl / total_mv) * 100.0,
        top_hit: top,
        note,
        beta_provenance: provenance,
    }
}

/// 行业集中度阈值：超过此值时压测中该行业持仓 beta 放大
const SECTOR_CONCENTRATION_THRESHOLD: f64 = 40.0;
/// 行业集中度惩罚系数：beta × 1.2（模拟流动性折价 + 相关性坍缩）
const SECTOR_CONCENTRATION_PENALTY: f64 = 1.2;

// ── 真实历史 beta 估计（2026-09-14，替代原「行业关键词 → 固定 beta」查表）──
//
// 为什么删掉那张表：它是**按行业归属下判据**的形态 —— 观测量是「这只股票被归到哪个
// 行业」，而不是「它实际怎么波动」。两个硬伤：
//   ① 口径必然落空：实测 `stock_sector` 取值域是**门类级**（电子设备 / 电气设备 /
//      信息技术 / 化石能源 / 机械设备 / 交运设备 / 金融 / 建材），而原表键是科技 /
//      消费 / 银行 / 医药 / 能源 / 地产 / 公用 ⇒ 除「金融」「化石能源」外**全部落到
//      else 1.0**，即这张表看起来在区分行业、实际基本没生效（静默退化为中性）。
//   ② 即便口径对上，「行业 X 的 beta 恒为 Y」本身也不成立 —— beta 是个股对市场的
//      回归系数，同行业内不同个股的差异远大于行业间均值的差异。
// 替代者直接观测数据形态：**用该股自己的日收益对沪深300日收益做回归**。
// 观测对象是「它实际怎么波动」这一事实 ⇒ 与行业分类无关，天然覆盖全部标的。
// 缺失 / 样本不足时回退中性 1.0，并把「哪些标的回退了」显式暴露出来
// （见 `BetaProvenance`）—— 静默退化正是本项目反复踩的坑。

/// 中性默认 beta —— 真实历史 beta 不可得时的回退值
pub const DEFAULT_BETA: f64 = 1.0;
/// beta 估计回看窗口（交易日）
pub const BETA_LOOKBACK_DAYS: u32 = 120;
/// 估计 beta 所需的最少重叠日收益样本（低于此值视为不可估计 ⇒ 回退）
pub const BETA_MIN_SAMPLES: usize = 30;
/// beta 估计的市场基准代码（沪深300）
pub const BETA_BENCHMARK_CODE: &str = "000300";
/// 相关性回看窗口的**回落默认值**（个交易日 K 线）—— 与 `seed_variables.rs` 里
/// `risk_correlation_lookback_days` 的声明值、设置面板 `b()` 的兜底值逐字相同。
pub const CORRELATION_LOOKBACK_DAYS: u32 = 60;
/// 面板值可采纳的区间：下界 20 —— 重叠日收益少于 20 时两两 pearson 基本是噪声
/// （与 [`BETA_MIN_SAMPLES`] 同一理由）；上界 250 —— 一年，再长会把停牌期外的旧价对拉进窗口。
const CORRELATION_LOOKBACK_RANGE: (u32, u32) = (20, 250);

/// 相关性快照实际使用的回看窗口 —— 面板 `risk_correlation_lookback_days` **真实生效**。
///
/// G-8：这个键在设置面板可编辑、也播种进了变量表，但刷新链此前只吃调用方写的裸 `60`
/// ⇒ 「面板改了没反应」的空接线（`check-panel-var-landing.mjs` 拦的正是这一形态，
/// 现已随本条落地进清单）。缺省/非法一律回落 [`CORRELATION_LOOKBACK_DAYS`]：
/// 回落值就是接线前的现值，所以接上之后**默认行为逐位不变**。
///
/// 拆成「纯函数 + 快照包装」两份（同 `PositionLimits::from_panel_vars` / `panel_effective`
/// 的形态）：区间判据住在纯函数里才有可测面，直接测快照版会改到进程内全局、并行互踩。
pub fn correlation_lookback_days_in(vars: &HashMap<String, serde_json::Value>) -> u32 {
    let (lo, hi) = CORRELATION_LOOKBACK_RANGE;
    match axagent_harness::panel_variables::integer_in(vars, "risk_correlation_lookback_days") {
        Some(v) if (lo as i64..=hi as i64).contains(&v) => v as u32,
        Some(v) => {
            tracing::warn!(
                "[portfolio_monitor] 面板 risk_correlation_lookback_days = {v} 不在 {lo}..={hi} ⇒ \
                 回落 {CORRELATION_LOOKBACK_DAYS}（该窗口是日 K 根数，过小则相关性是噪声）"
            );
            CORRELATION_LOOKBACK_DAYS
        },
        // 键缺席 = 变量表还没装进来（首启空库 / 模板被删），属正常态，不 warn
        None => CORRELATION_LOOKBACK_DAYS,
    }
}

/// [`correlation_lookback_days_in`] 的快照版（生产路径用它）。
pub fn correlation_lookback_days() -> u32 {
    correlation_lookback_days_in(&axagent_harness::panel_variables::panel_variables())
}

/// 单只标的的 beta 估计结果
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct BetaEstimate {
    /// beta 值（真实估计或回退默认）
    pub beta: f64,
    /// 用于估计的重叠日收益样本数（回退时为 0）
    pub samples: usize,
    /// 是否来自真实历史估计（`false` = 回退 `DEFAULT_BETA`）
    pub historical: bool,
}

impl BetaEstimate {
    /// 回退值构造器（唯一入口，避免各处手写 1.0）
    pub fn fallback() -> Self {
        Self { beta: DEFAULT_BETA, samples: 0, historical: false }
    }
    pub fn estimated(beta: f64, samples: usize) -> Self {
        Self { beta, samples, historical: true }
    }
}

/// 逐标的 beta 估计表（key = 股票代码）
pub type BetaMap = HashMap<String, BetaEstimate>;

/// 收盘价序列 → `(日期, 日简单收益率)` 配对。
///
/// 用**日期**而非位置对齐：停牌会让两条序列的 bar 数不同，按位置对齐会把
/// 「停牌日」与「邻近日」错配成假收益（`refresh_correlation` 用的是尾部对齐，
/// 单看相关性影响有限，但 beta 的分子分母都会因此偏掉）。
pub fn date_returns(klines: &[KLine]) -> Vec<(String, f64)> {
    klines
        .windows(2)
        .filter_map(|w| {
            let (a, b) = (&w[0], &w[1]);
            if a.close > 0.0 && b.close > 0.0 {
                Some((b.date.clone(), (b.close - a.close) / a.close))
            } else {
                None
            }
        })
        .collect()
}

/// 纯函数：按**日期交集**对齐两条收益序列，求个股对市场的 beta。
///
/// 返回 `Some((beta, 重叠样本数))`；重叠样本 < `BETA_MIN_SAMPLES`，或市场收益方差
/// 为 0（`compute_beta` 内部判据）时返回 `None` ⇒ 调用方回退 `DEFAULT_BETA`。
pub fn beta_from_aligned(
    stock: &[(String, f64)],
    market: &[(String, f64)],
) -> Option<(f64, usize)> {
    let stock_by_date: HashMap<&str, f64> = stock.iter().map(|(d, r)| (d.as_str(), *r)).collect();
    // 以**市场序列的日期顺序**为基准（它是交易日历的权威），只取双方都有的日期
    let mut s_ret: Vec<f64> = Vec::new();
    let mut m_ret: Vec<f64> = Vec::new();
    for (d, mr) in market {
        if let Some(sr) = stock_by_date.get(d.as_str()) {
            m_ret.push(*mr);
            s_ret.push(*sr);
        }
    }
    if s_ret.len() < BETA_MIN_SAMPLES {
        return None;
    }
    compute_beta(&s_ret, &m_ret).map(|b| (b, s_ret.len()))
}

/// 拉取基准与各持仓 K 线，估计真实历史 beta。
///
/// - 基准：`BETA_BENCHMARK_CODE`（沪深300）**不复权**日线 —— 指数无复权概念，
///   与 `backtest.rs` / `market_regime.rs` 的既有取法一致
/// - 个股：**前复权**日线 —— 未复权价在除权日会产生假收益、直接污染 beta；
///   前复权是项目里算收益的标准做法（`backtest.rs:117`）
/// - 任一环节失败 ⇒ 该标的回退 `DEFAULT_BETA`（**不阻断压测主链路**）
///
/// 顺序拉取（不并发）：持仓通常 < 20 只，且本项目历史上因并发过高触发过供应商
/// 降级（429 处置），此处不引入新的并发面。
pub async fn estimate_betas(
    client: &dyn MarketDataProvider,
    positions: &[PositionSummary],
    lookback_days: u32,
) -> BetaMap {
    let mut out: BetaMap = HashMap::new();
    if positions.is_empty() {
        return out;
    }
    let market = match client.get_klines(BETA_BENCHMARK_CODE, "daily", lookback_days, None).await {
        Ok(ks) => date_returns(&ks),
        Err(e) => {
            tracing::warn!(
                "[portfolio_monitor] 基准 {} K 线获取失败，全部 beta 回退 {}: {e}",
                BETA_BENCHMARK_CODE,
                DEFAULT_BETA
            );
            Vec::new()
        },
    };
    if market.is_empty() {
        // 基准缺失 ⇒ 无法估计任何标的。显式回退（`historical=false`），
        // 不做「用 1.0 冒充估计值」的静默降级。
        for p in positions {
            out.insert(p.stock_code.clone(), BetaEstimate::fallback());
        }
        return out;
    }
    for p in positions {
        let est = match client
            .get_klines(&p.stock_code, "daily", lookback_days, Some(AdjType::Forward))
            .await
        {
            Ok(ks) => match beta_from_aligned(&date_returns(&ks), &market) {
                Some((b, n)) => BetaEstimate::estimated(b, n),
                None => BetaEstimate::fallback(),
            },
            Err(e) => {
                tracing::warn!(
                    "[portfolio_monitor] {} K 线获取失败，beta 回退 {}: {e}",
                    p.stock_code,
                    DEFAULT_BETA
                );
                BetaEstimate::fallback()
            },
        };
        out.insert(p.stock_code.clone(), est);
    }
    out
}

#[derive(Debug, Clone, Copy)]
pub enum StressScenario {
    MarketDown10,
    MarketDown20,
    BlackSwan,
}

impl StressScenario {
    pub fn code(&self) -> &'static str {
        match self {
            StressScenario::MarketDown10 => "m10",
            StressScenario::MarketDown20 => "m20",
            StressScenario::BlackSwan => "blackSwan",
        }
    }
    pub fn label(&self) -> &'static str {
        match self {
            StressScenario::MarketDown10 => "大盘 -10%",
            StressScenario::MarketDown20 => "大盘 -20%",
            StressScenario::BlackSwan => "黑天鹅 (-30%)",
        }
    }
    pub fn market_drop(&self) -> f64 {
        match self {
            StressScenario::MarketDown10 => -0.10,
            StressScenario::MarketDown20 => -0.20,
            StressScenario::BlackSwan => -0.30,
        }
    }
}

pub fn run_all_scenarios(
    positions: &[PositionSummary],
    sector_exposure: &HashMap<String, f64>,
    betas: &BetaMap,
) -> StressTestBundle {
    StressTestBundle {
        m10: Some(run_stress_scenario(
            positions,
            sector_exposure,
            betas,
            StressScenario::MarketDown10,
        )),
        m20: Some(run_stress_scenario(
            positions,
            sector_exposure,
            betas,
            StressScenario::MarketDown20,
        )),
        black_swan: Some(run_stress_scenario(
            positions,
            sector_exposure,
            betas,
            StressScenario::BlackSwan,
        )),
    }
}

// ── 整合：组合 dashboard ──
// 原先这里收一个 `_limits: &PositionLimits` 却从不读 —— 面板上并没有「仓位限制」这一行，
// 传进来的三条上限在这条链上没有消费者（G-5 取「撤死参数」而不是「顺手造一行判据」）。
pub fn compute_dashboard(
    positions: &[PositionSummary],
    beta: Option<f64>,
    sharpe_30d: Option<f64>,
    correlation_avg: Option<f64>,
    stress: StressTestBundle,
    is_historical: bool,
    as_of_date: Option<String>,
) -> PortfolioDashboard {
    // P0-2: 计算当前组合中各标的市值占比，用于暴露度分析
    // 注: portfolio-mgr 的"建议仓位"存在 stock_analyses 表，不在此处聚合。
    //     真正的组合仓位归一化由 normalize_position_weights 函数提供，
    //     待前端/命令层集成后启用（当前版本保持 API 可用）。
    // 市值：已定价持仓之和（与改造前**数值相同** —— 往分母加 0 不改和）。这一处不是缺陷，
    // 真正的两条口径缺陷是下面的「盈亏分母」与「按几只算」。
    let priced: Vec<&PositionSummary> =
        positions.iter().filter(|p| p.market_value.is_some()).collect();
    let total_mv: f64 = priced.iter().map(|p| p.market_value.unwrap_or(0.0)).sum();
    // 盈亏与其分母**同源**：原先分子把 `unrealized_pnl` 缺失的按 0 计、分母却按
    // 「全部持仓的成本」算 ⇒ 拿不到盈亏的那几只只撑大分母，收益率被系统性低估。
    let pnl_pairs: Vec<(f64, f64)> = positions
        .iter()
        .filter_map(|p| p.unrealized_pnl.map(|v| (v, p.avg_cost * p.total_shares as f64)))
        .collect();
    let total_pnl: f64 = pnl_pairs.iter().map(|(v, _)| *v).sum();
    let total_cost: f64 = pnl_pairs.iter().map(|(_, c)| *c).sum();
    let total_pnl_pct = if total_cost > 0.0 {
        (total_pnl / total_cost) * 100.0
    } else {
        0.0
    };
    // G-1 的真缺陷之一：分散度与风险等级收的 `n` 原先是**表里的行数**（含拿不到价的），
    // 而占比一律按已定价算 ⇒ 「8 只里 3 只没价」会被判成「8 只很分散」，
    // 同一张面板里两套口径。现在 `n` 与占比同源，缺价只数另走 `unpriced_positions`。
    let n = priced.len();
    let (top_pct, sector, max_sector_pct) = compute_concentration(positions);
    let risk_level = compute_risk_level(top_pct, max_sector_pct, n);
    let div_score = compute_diversification_score(n, top_pct, max_sector_pct);
    let warning = compute_concentration_warning(top_pct, max_sector_pct, n);

    PortfolioDashboard {
        is_historical,
        as_of_date,
        total_market_value: total_mv,
        total_pnl,
        total_pnl_pct,
        cash_pct: 0.0, // 无现金来源 ⇒ 恒 0。接进面板前不得声称「由 refresh 补算」（PLAN 裁定 8）
        max_drawdown_pct: 0.0, // 由 `refresh_metrics` 从历史净值序列补算（本函数手上没有历史）
        unpriced_positions: count_unpriced(positions) as u32,
        beta,
        sharpe_30d,
        correlation_avg,
        top_concentration_pct: top_pct,
        sector_exposure: sector,
        concentration_warning: warning,
        risk_level,
        diversification_score: div_score,
        stress_test: stress,
        positions: positions.to_vec(),
        snapshot_at: chrono::Utc::now().timestamp_millis(),
    }
}

// ── P0-2: 组合仓位归一化 ──
// portfolio-mgr 对每只股票独立输出 position_pct，各标的仓位之和可能远超 100%。
// 例如 4 只标的同时建议 35% → 总仓位 140%，实际无法执行。
// 本函数将传入的仓位列表按比例压缩到总上限内，同时保持相对权重不变。
//
// 场景：
//   A. raw_sum ≤ cap → 不压缩（仓位已经自洽）
//   B. raw_sum > cap → 按 cap/raw_sum 比例压缩
//
// 例: [30%, 25%, 20%] sum=75% ≤ 100% → 不变
//     [50%, 40%, 30%] sum=120% > 100% → 各 ×100/120=[41.7%, 33.3%, 25.0%]
pub fn normalize_position_weights(positions: &[f64], max_total_pct: f64) -> Vec<f64> {
    let raw_sum: f64 = positions.iter().sum();
    if raw_sum <= max_total_pct || raw_sum <= 0.0 {
        return positions.to_vec();
    }
    let factor = max_total_pct / raw_sum;
    positions.iter().map(|&p| (p * factor * 10.0).round() / 10.0).collect()
}

// ── 持久化层 ──

/// 回撤序列往回取多少个日历日（约一个季度）。
///
/// 不设上限的话，一年多前那次真实暴跌会永远压着今天的「最大回撤」，面板读到的
/// 是「历史最惨一次」而不是「近期回撤」——与本模块其余 30 日口径（`sharpe_30d`）不自洽。
const DRAWDOWN_LOOKBACK_DAYS: i64 = 90;

/// 从 `portfolio_metrics_daily` 的历史市值序列 + 当前市值算最大回撤（百分比）。
///
/// 两条真实约束各有一条会被红起来的判据（`.filter(...lte...)` 与回看窗那两行）：
/// - **严格早于** `before_date` ⇒ 同一天重复刷新不会把自己算进历史（否则第二次刷新
///   会把第一次那行当成「跌过一次」）；
/// - 90 日上限 ⇒ 见 [`DRAWDOWN_LOOKBACK_DAYS`]。
///
/// 非正值（空组合那天的 0 市值）**不在这里过滤**：共享实现
/// [`axagent_harness::indicators::max_drawdown_fraction`] 自己 `p <= 0 ⇒ continue`，
/// 且「没有任何正值」时返回 `None` → `peak_trough_drawdown` 映射为 0.0。本批第一版
/// 在本层又写了一遍 `filter` + `len < 2` 早退，两条变异体因此恒绿 —— 那是等价死码，
/// 已删（见 §7.1）。
async fn drawdown_from_history(
    db: &DatabaseConnection,
    before_date: &str,
    now_mv: f64,
) -> Result<f64, String> {
    use axagent_entities::portfolio_metrics_daily;
    let floor = match chrono::NaiveDate::parse_from_str(before_date, "%Y-%m-%d") {
        Ok(d) => {
            (d - chrono::Duration::days(DRAWDOWN_LOOKBACK_DAYS)).format("%Y-%m-%d").to_string()
        },
        Err(e) => {
            tracing::warn!(
                "[portfolio] 快照日期 {before_date} 不是 YYYY-MM-DD（{e}）⇒ 本轮不补算回撤，记 0.0"
            );
            return Ok(0.0);
        },
    };
    let history = portfolio_metrics_daily::Entity::find()
        .filter(portfolio_metrics_daily::Column::SnapshotDate.gte(floor))
        .filter(portfolio_metrics_daily::Column::SnapshotDate.lt(before_date))
        .order_by_asc(portfolio_metrics_daily::Column::SnapshotDate)
        .all(db)
        .await
        .map_err(|e| format!("query portfolio_metrics_daily history: {e}"))?;
    let mut curve: Vec<f64> = history.iter().map(|r| r.total_market_value).collect();
    curve.push(now_mv);
    Ok(compute_max_drawdown_pct(&curve))
}

// 参数 7 个：这是「组合监控一次刷新」的统一入参，拆结构体只会让调用方多一层构造。
pub async fn refresh_metrics(
    db: &DatabaseConnection,
    positions: &[PositionSummary],
    betas: &BetaMap,
    beta: Option<f64>,
    sharpe_30d: Option<f64>,
    correlation_avg: Option<f64>,
    as_of_date: Option<&str>,
) -> Result<(String, u32), String> {
    // G-2：快照日期按**北京会话日**盖，不用 `Utc::now()`。原先 UTC 日期在北京早上 8 点前
    //      会落到前一天（UTC 还是昨天）⇒ 同一天的两次刷新写出两行、日期还倒退一天。
    let today = as_of_date.map(|s| s.to_string()).unwrap_or_else(|| {
        axagent_astock_data::calendar::beijing_today().format("%Y-%m-%d").to_string()
    });
    // 落库快照与实时面板必须**同口径**：`betas` 由调用方（持有 `MarketDataProvider`
    // 的那一层）传入。若此处自己传空表，快照会恒用回退 1.0，而面板用真实 beta ⇒
    // 「时间旅行」对比等于在比两套方法。
    let stress = run_all_scenarios(positions, &compute_concentration(positions).1, betas);
    let mut dashboard = compute_dashboard(
        positions,
        beta,
        sharpe_30d,
        correlation_avg,
        stress,
        false,
        Some(today.clone()),
    );
    // G-3：最大回撤真算。原先这列被 `compute_dashboard` 写死 0.0、注释却声称
    //      「由 refresh_metrics 走历史」，而这段代码从来不存在 ⇒ 落库行的该列恒 0。
    dashboard.max_drawdown_pct =
        drawdown_from_history(db, &today, dashboard.total_market_value).await?;

    let sector_json = serde_json::to_string(&dashboard.sector_exposure)
        .map_err(|e| format!("serialize sector_exposure: {e}"))?;
    let stress_json = serde_json::to_string(&dashboard.stress_test)
        .map_err(|e| format!("serialize stress_test: {e}"))?;

    let id = Uuid::new_v4().to_string();
    let now = chrono::Utc::now().timestamp_millis();
    let new_row = axagent_entities::portfolio_metrics_daily::ActiveModel {
        id: Set(id.clone()),
        snapshot_date: Set(today),
        total_market_value: Set(dashboard.total_market_value),
        cash_pct: Set(dashboard.cash_pct),
        total_pnl: Set(dashboard.total_pnl),
        total_pnl_pct: Set(dashboard.total_pnl_pct),
        max_drawdown_pct: Set(dashboard.max_drawdown_pct),
        beta: Set(dashboard.beta),
        sharpe_30d: Set(dashboard.sharpe_30d),
        correlation_avg: Set(dashboard.correlation_avg),
        top_concentration_pct: Set(dashboard.top_concentration_pct),
        // G-5：把「按几只算的」与「几只没价」一起落库 ⇒ 历史快照读回来时能复原
        //      风险等级与分散度，而不是像原先那样硬编码 `"—"` / `0`。
        position_count: Set(Some(
            dashboard.positions.iter().filter(|p| p.market_value.is_some()).count() as i32,
        )),
        unpriced_count: Set(Some(dashboard.unpriced_positions as i32)),
        sector_exposure_json: Set(sector_json),
        stress_test_json: Set(Some(stress_json)),
        created_at: Set(now),
    };
    new_row.insert(db).await.map_err(|e| format!("insert portfolio_metrics_daily: {e}"))?;
    // 第二条是**本次快照覆盖的持仓只数**（含缺价的那几只）。原先恒返 `1`（= 写了几行），
    // 而命令面与前端类型都把它叫 `positionsSnapshotted` ⇒ 名字说的是只数、值给的是行数。
    Ok((id, dashboard.positions.len() as u32))
}

/// 一次完整刷新的产物（面板「立即刷新」与收盘定时任务共用，避免两处各拼一份 JSON）
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RefreshSummary {
    pub metrics_id: String,
    /// 本次快照**覆盖的持仓只数**（含缺价那几只）；不是「写了几行」（行恒为一行聚合）
    pub positions_snapshotted: u32,
    pub correlation_pairs_written: u32,
    pub as_of_date: Option<String>,
}

/// 组合快照刷新的**唯一**通路：β（真实历史回归）→ 指标落库 → 相关性落库。
///
/// G-7：这三步原先只写在 `commands::stock_analysis::refresh_portfolio_metrics` 的命令体里，
/// 定时任务无从复用。收盘后的日更任务若自己再拼一遍，面板与 cron 落的就不是同一套口径
/// （本项目反复出现的「同一逻辑两份实现，只改一处」形态）。
///
/// `as_of_date = None` ⇒ 按北京会话日盖当天（live）；面板时间旅行刷新时透传所选日期。
pub async fn refresh_snapshot(
    db: &DatabaseConnection,
    client: &dyn MarketDataProvider,
    positions: &[PositionSummary],
    as_of_date: Option<&str>,
) -> Result<RefreshSummary, String> {
    // β 与实时面板同源同口径：若这里自己传空表，快照会恒用回退 1.0，
    // 而面板用真实历史 beta ⇒ 按 as_of_date 做时间旅行对比等于在比两套方法。
    let betas = estimate_betas(client, positions, BETA_LOOKBACK_DAYS).await;
    // 仓位三条上限在这条链上从不被读（G-5 撤死参数）。它们的现役消费者是风控门
    // `trading.rs` 与分析命令里的那几处 `PositionLimits::panel_effective()`，与此无关。
    let (metrics_id, positions_snapshotted) =
        refresh_metrics(db, positions, &betas, None, None, None, as_of_date).await?;
    let correlation_pairs_written =
        refresh_correlation(db, client, positions, correlation_lookback_days(), as_of_date).await?;

    Ok(RefreshSummary {
        metrics_id,
        positions_snapshotted,
        correlation_pairs_written,
        as_of_date: as_of_date.map(|s| s.to_string()),
    })
}

pub async fn get_dashboard(
    db: &DatabaseConnection,
    as_of_date: Option<&str>,
) -> Result<PortfolioDashboard, String> {
    use axagent_entities::portfolio_metrics_daily;

    let row = if let Some(date) = as_of_date {
        // time travel：取 <= as_of_date 的最新一行
        portfolio_metrics_daily::Entity::find()
            .filter(portfolio_metrics_daily::Column::SnapshotDate.lte(date.to_string()))
            .order_by_desc(portfolio_metrics_daily::Column::SnapshotDate)
            .one(db)
            .await
            .map_err(|e| format!("query portfolio_metrics_daily: {e}"))?
    } else {
        portfolio_metrics_daily::Entity::find()
            .order_by_desc(portfolio_metrics_daily::Column::SnapshotDate)
            .one(db)
            .await
            .map_err(|e| format!("query portfolio_metrics_daily: {e}"))?
    };

    match row {
        Some(m) => {
            let sector: HashMap<String, f64> =
                serde_json::from_str(&m.sector_exposure_json).unwrap_or_default();
            let max_sector_pct = sector.values().cloned().fold(0.0_f64, f64::max);
            let stress: StressTestBundle = m
                .stress_test_json
                .as_deref()
                .and_then(|s| serde_json::from_str(s).ok())
                .unwrap_or_default();
            // G-5：风险等级 / 分散度 / 集中度告警这三样原先在这条读取路径上被**硬编码**
            // 成 `"—"`、`0` 与「阈值传 0」（`n<3` 那一档因此永不命中）——面板拿到的是常量，
            // 不是快照。现在用落库的 `position_count` + 已存的集中度/行业暴露复原。
            // `position_count` 为 `None`（本列之前落的老行）⇒ 保持 `"—"` 与 0，不硬造一个数。
            let n_history = m.position_count.unwrap_or(0).max(0) as usize;
            let (risk_level, diversification_score) = if m.position_count.is_some() {
                (
                    compute_risk_level(m.top_concentration_pct, max_sector_pct, n_history),
                    compute_diversification_score(
                        n_history,
                        m.top_concentration_pct,
                        max_sector_pct,
                    ),
                )
            } else {
                ("—".to_string(), 0)
            };
            Ok(PortfolioDashboard {
                is_historical: as_of_date.is_some(),
                as_of_date: as_of_date.map(|s| s.to_string()).or(Some(m.snapshot_date.clone())),
                total_market_value: m.total_market_value,
                total_pnl: m.total_pnl,
                total_pnl_pct: m.total_pnl_pct,
                cash_pct: m.cash_pct,
                max_drawdown_pct: m.max_drawdown_pct,
                unpriced_positions: m.unpriced_count.unwrap_or(0).max(0) as u32,
                beta: m.beta,
                sharpe_30d: m.sharpe_30d,
                correlation_avg: m.correlation_avg,
                top_concentration_pct: m.top_concentration_pct,
                sector_exposure: sector,
                concentration_warning: if m.position_count.is_some() {
                    compute_concentration_warning(
                        m.top_concentration_pct,
                        max_sector_pct,
                        n_history,
                    )
                } else {
                    None
                },
                risk_level,
                diversification_score,
                stress_test: stress,
                // 快照行不存持仓明细（本表按设计只存聚合值）⇒ 历史视图拿不到逐只持仓，
                // 这是**数据结构**的限制，不是这条路径漏读；面板不得把空表读成「没有持仓」。
                positions: vec![],
                snapshot_at: m.created_at,
            })
        },
        None => {
            // 空 dashboard
            Ok(PortfolioDashboard {
                is_historical: as_of_date.is_some(),
                as_of_date: as_of_date.map(|s| s.to_string()),
                total_market_value: 0.0,
                total_pnl: 0.0,
                total_pnl_pct: 0.0,
                cash_pct: 0.0,
                max_drawdown_pct: 0.0,
                unpriced_positions: 0,
                beta: None,
                sharpe_30d: None,
                correlation_avg: None,
                top_concentration_pct: 0.0,
                sector_exposure: HashMap::new(),
                concentration_warning: Some("尚无快照数据，请点击「刷新」".to_string()),
                risk_level: "无持仓".to_string(),
                diversification_score: 0,
                stress_test: StressTestBundle::default(),
                positions: vec![],
                snapshot_at: 0,
            })
        },
    }
}

/// 计算并落库两两相关性（拉 K 线、pearson、写库）
pub async fn refresh_correlation(
    db: &DatabaseConnection,
    client: &dyn MarketDataProvider,
    positions: &[PositionSummary],
    lookback_days: u32,
    as_of_date: Option<&str>,
) -> Result<u32, String> {
    use axagent_entities::portfolio_correlation_snapshot;

    if positions.len() < 2 {
        return Ok(0);
    }
    // N≤20 全算；N>20 退化为只算与最大持仓的相关性
    // G-1：锚点只在**拿到现价**的持仓里选 —— 原先 `unwrap_or(0.0)` 让缺价的票参与比较，
    //      三只都缺价时它会选中第一只（0.0 与 0.0 取 max），锚点就成了一次任意选择。
    let anchor = positions
        .iter()
        .filter(|p| p.market_value.is_some())
        .max_by(|a, b| {
            a.market_value
                .unwrap_or(0.0)
                .partial_cmp(&b.market_value.unwrap_or(0.0))
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .map(|p| p.stock_code.clone())
        .unwrap_or_default();

    let codes: Vec<String> = if positions.len() <= 20 {
        positions.iter().map(|p| p.stock_code.clone()).collect()
    } else {
        vec![anchor.clone()]
    };
    let pairs: Vec<(String, String)> = if positions.len() <= 20 {
        let mut out = Vec::new();
        for i in 0..codes.len() {
            for j in (i + 1)..codes.len() {
                out.push((codes[i].clone(), codes[j].clone()));
            }
        }
        out
    } else {
        positions
            .iter()
            .filter(|p| p.stock_code != anchor)
            .map(|p| (anchor.clone(), p.stock_code.clone()))
            .collect()
    };
    if pairs.is_empty() {
        return Ok(0);
    }

    // 拉每只股票的 K 线并转成「日期 → 日收益」序列。
    // G-2 两处口径订正：
    // ① 原先传 `None`＝**不复权** ⇒ 除权日那根假价差直接进相关系数，而同一个文件里
    //    `estimate_betas` 对个股用的是前复权（本文件 :513）—— 两个组合级指标各按一套价格算；
    // ② 原先直接对**收盘价水平**求 pearson 且「取较短者尾部」对齐 —— 这正是本文件
    //    `date_returns` 上方自己点过的缺陷（停牌/上市晚一天会整条序列错位）。
    let mut series: HashMap<String, Vec<(String, f64)>> = HashMap::new();
    for code in &codes {
        match client.get_klines(code, "daily", lookback_days, Some(AdjType::Forward)).await {
            Ok(ks) => {
                let rets = date_returns(&ks);
                // 收益点比 K 线根数少一（`date_returns` 用 windows(2)）⇒ 门槛按收益点数判
                if rets.len() >= 5 {
                    series.insert(code.clone(), rets);
                }
            },
            Err(e) => {
                // 原先是 `eprintln!` —— 它不进 tracing sink，生产日志里查不到这条失败
                tracing::warn!("[portfolio_monitor] 相关性取 K 线失败: code={code} err={e}");
            },
        }
    }
    if series.len() < 2 {
        return Ok(0);
    }

    // G-2：快照日期按北京会话日盖（与 `refresh_metrics` 同口径），不用 `Utc::now()`
    let today = as_of_date.map(|s| s.to_string()).unwrap_or_else(|| {
        axagent_astock_data::calendar::beijing_today().format("%Y-%m-%d").to_string()
    });
    let now = chrono::Utc::now().timestamp_millis();
    let mut written = 0u32;
    for (a, b) in pairs {
        let (Some(x), Some(y)) = (series.get(&a), series.get(&b)) else {
            continue;
        };
        let corr = match aligned_pearson(x, y) {
            Some(c) => c,
            None => continue,
        };
        let id = Uuid::new_v4().to_string();
        let row = portfolio_correlation_snapshot::ActiveModel {
            id: Set(id),
            snapshot_date: Set(today.clone()),
            lookback_days: Set(lookback_days as i32),
            code_a: Set(a),
            code_b: Set(b),
            correlation: Set(corr),
            created_at: Set(now),
        };
        // 写失败不再被 `if ...is_ok()` 吞掉：计数照旧只记成功，但每条失败要有读数
        if let Err(e) = row.insert(db).await {
            tracing::warn!("[portfolio_monitor] 相关性快照写库失败: {e}");
        } else {
            written += 1;
        }
    }
    Ok(written)
}

/// 读最近一次相关性快照（按 snapshot_date desc）
pub async fn get_correlation_snapshot(
    db: &DatabaseConnection,
    as_of_date: Option<&str>,
) -> Result<Vec<CorrelationCell>, String> {
    use axagent_entities::portfolio_correlation_snapshot;

    // 找到最近一次 snapshot_date
    let latest_date: Option<String> = if let Some(date) = as_of_date {
        portfolio_correlation_snapshot::Entity::find()
            .filter(portfolio_correlation_snapshot::Column::SnapshotDate.lte(date.to_string()))
            .select_only()
            .column(portfolio_correlation_snapshot::Column::SnapshotDate)
            .order_by_desc(portfolio_correlation_snapshot::Column::SnapshotDate)
            .into_tuple()
            .one(db)
            .await
            .map_err(|e| format!("query latest corr date: {e}"))?
    } else {
        portfolio_correlation_snapshot::Entity::find()
            .select_only()
            .column(portfolio_correlation_snapshot::Column::SnapshotDate)
            .order_by_desc(portfolio_correlation_snapshot::Column::SnapshotDate)
            .into_tuple()
            .one(db)
            .await
            .map_err(|e| format!("query latest corr date: {e}"))?
    };
    let Some(date) = latest_date else { return Ok(vec![]) };
    let rows = portfolio_correlation_snapshot::Entity::find()
        .filter(portfolio_correlation_snapshot::Column::SnapshotDate.eq(date.clone()))
        .all(db)
        .await
        .map_err(|e| format!("query corr rows: {e}"))?;
    Ok(rows
        .into_iter()
        .map(|r| CorrelationCell { code_a: r.code_a, code_b: r.code_b, correlation: r.correlation })
        .collect())
}

// ── 单元测试 ──

#[cfg(test)]
mod tests {
    use super::*;

    fn ps(code: &str, mv: f64, sector: Option<&str>, cost: f64) -> PositionSummary {
        PositionSummary {
            stock_code: code.into(),
            stock_name: code.into(),
            total_shares: 100,
            avg_cost: cost,
            current_price: Some(mv / 100.0),
            market_value: Some(mv),
            unrealized_pnl: Some(mv - cost * 100.0),
            unrealized_pnl_pct: Some(((mv - cost * 100.0) / (cost * 100.0)) * 100.0),
            total_realized_pnl: 0.0,
            sector_name: sector.map(|s| s.to_string()),
        }
    }

    /// 缺现价的持仓：`market_value` 与 `unrealized_pnl` 都是 `None`（取不到价 ⇒ 盈亏也算不出）。
    fn ps_unpriced(code: &str, cost: f64) -> PositionSummary {
        let mut p = ps(code, cost * 100.0, Some("未知"), cost);
        p.current_price = None;
        p.market_value = None;
        p.unrealized_pnl = None;
        p.unrealized_pnl_pct = None;
        p.sector_name = None;
        p
    }

    #[test]
    fn empty_positions_returns_zero_concentration() {
        let (top, sector, max_sec) = compute_concentration(&[]);
        assert_eq!(top, 0.0);
        assert!(sector.is_empty());
        assert_eq!(max_sec, 0.0);
    }

    #[test]
    fn single_position_full_concentration() {
        let p = ps("000001", 10000.0, Some("银行"), 50.0);
        let (top, sector, _max) = compute_concentration(&[p]);
        assert!((top - 100.0).abs() < 1e-6);
        assert_eq!(sector.get("银行"), Some(&100.0));
    }

    #[test]
    fn multi_position_concentration_proportional() {
        let pos = vec![
            ps("a", 6000.0, Some("科技"), 50.0),
            ps("b", 3000.0, Some("科技"), 50.0),
            ps("c", 1000.0, Some("消费"), 50.0),
        ];
        let (top, sector, max_sec) = compute_concentration(&pos);
        assert!((top - 60.0).abs() < 1e-6);
        assert!((sector["科技"] - 90.0).abs() < 1e-6);
        assert!((sector["消费"] - 10.0).abs() < 1e-6);
        assert!((max_sec - 90.0).abs() < 1e-6);
    }

    /// G-1 的真缺陷之一：盈亏的**分母**必须与分子同源。
    /// 改造前 `total_cost` 按全部持仓的成本算，而分子把拿不到盈亏的那只按 0 计 ⇒
    /// 缺价的那只只撑大分母，收益率被系统性低估（下面这组数：-10% 会被算成 -6.67%）。
    #[test]
    fn unpriced_position_excluded_from_pnl_denominator() {
        let pos = vec![
            ps("a", 6000.0, Some("科技"), 50.0),
            ps("b", 3000.0, Some("科技"), 50.0),
            ps_unpriced("c", 50.0),
        ];
        let d = compute_dashboard(&pos, None, None, None, StressTestBundle::default(), false, None);
        assert_eq!(d.total_market_value, 9000.0, "缺价的那只不计入市值");
        assert_eq!(d.total_pnl, -1000.0);
        assert!(
            (d.total_pnl_pct - (-10.0)).abs() < 1e-9,
            "分母应是两只已知成本 10000，不是三只 15000：实得 {}",
            d.total_pnl_pct
        );
        assert_eq!(d.unpriced_positions, 1, "缺价只数必须跟着报出来");
    }

    /// G-1 的真缺陷之二：分散度/风险等级收的只数必须与占比同源。
    /// 8 行里 4 行没价 ⇒ 占比按 4 只算（各 25%），原先 `n` 却按 8 行判 ⇒
    /// `compute_diversification_score` 落到「≥5 只且单股≤25%」那一档（70 分），
    /// 把「一半持仓拿不到价」演成「很分散」。
    #[test]
    fn diversification_uses_priced_count_not_row_count() {
        let pos = vec![
            ps("a", 2500.0, Some("科技"), 50.0),
            ps("b", 2500.0, Some("消费"), 50.0),
            ps("c", 2500.0, Some("医药"), 50.0),
            ps("d", 2500.0, Some("金融"), 50.0),
            ps_unpriced("e", 50.0),
            ps_unpriced("f", 50.0),
            ps_unpriced("g", 50.0),
            ps_unpriced("h", 50.0),
        ];
        let d = compute_dashboard(&pos, None, None, None, StressTestBundle::default(), false, None);
        assert_eq!(d.unpriced_positions, 4);
        assert_eq!(
            d.diversification_score, 50,
            "只数口径变了这一档就要跟着变（按 8 行判会得到 70，那是把缺价当分散）"
        );
    }

    /// G-2：相关性必须按**日期交集**对齐，不能按尾部位置。
    /// A 有 d1..d7、B 停牌少 d4 ⇒ 交集 6 个点且逐日同值 ⇒ 真相关是 1.0；
    /// 原先的「取较短者尾部」会把 A 的 d2 配到 B 的 d1，算出 ≈0.98 的假数。
    #[test]
    fn aligned_pearson_pairs_by_date_not_by_tail_position() {
        let a: Vec<(String, f64)> = (1..=7).map(|i| (format!("d{i}"), i as f64)).collect();
        let b: Vec<(String, f64)> =
            [1, 2, 3, 5, 6, 7].iter().map(|i| (format!("d{i}"), *i as f64)).collect();
        let c = aligned_pearson(&a, &b).expect("交集 6 个点，够算");
        assert!((c - 1.0).abs() < 1e-9, "逐日同值的相关必须是 1.0，实得 {c}");
    }

    /// 两段完全不相交的日期窗口 ⇒ 不得产出一个相关数（老口径会照样算出 1.0）。
    #[test]
    fn aligned_pearson_refuses_disjoint_windows() {
        let a: Vec<(String, f64)> = (1..=5).map(|i| (format!("d{i}"), i as f64)).collect();
        let b: Vec<(String, f64)> = (6..=10).map(|i| (format!("d{i}"), (i - 5) as f64)).collect();
        assert_eq!(aligned_pearson(&a, &b), None, "零重叠的窗口不可比，不是「相关性为 1」");
    }

    #[test]
    fn risk_level_thresholds() {
        assert_eq!(compute_risk_level(10.0, 10.0, 5), "低风险");
        assert_eq!(compute_risk_level(25.0, 35.0, 3), "中等风险");
        assert_eq!(compute_risk_level(35.0, 35.0, 3), "中高风险");
        assert_eq!(compute_risk_level(60.0, 60.0, 3), "高风险");
        assert_eq!(compute_risk_level(0.0, 0.0, 0), "无持仓");
    }

    #[test]
    fn max_drawdown_handles_empty() {
        assert_eq!(compute_max_drawdown_pct(&[]), 0.0);
        assert_eq!(compute_max_drawdown_pct(&[0.0]), 0.0);
    }

    #[test]
    fn max_drawdown_basic_curve() {
        // 价格水平曲线：100 → 120（peak）→ 96 → 88（trough）
        // dd = (120 - 88) / 120 = 26.67%
        let curve = vec![100.0, 110.0, 120.0, 96.0, 88.0];
        let dd = compute_max_drawdown_pct(&curve);
        assert!(dd > 26.0 && dd < 28.0, "dd = {dd}");
    }

    #[test]
    fn sharpe_rejects_too_few_points() {
        // P3-C8: 年化因子切换为 A 股 244 天
        assert!(compute_sharpe(&[1.0, 2.0, 3.0], 244.0).is_none());
    }

    #[test]
    fn sharpe_basic_calculation() {
        // 6 个点 1% mean 0.5% std → sharpe = 1/0.5 * sqrt(244) ≈ 31.2
        let r = vec![0.5, 1.0, 1.5, 1.0, 0.5, 1.5];
        let s = compute_sharpe(&r, 244.0).unwrap();
        assert!(s > 25.0 && s < 40.0, "sharpe = {s}");
    }

    #[test]
    fn pearson_perfect_positive_correlation() {
        let x = vec![1.0, 2.0, 3.0, 4.0, 5.0];
        let y = vec![2.0, 4.0, 6.0, 8.0, 10.0];
        assert!((pearson_correlation(&x, &y).unwrap() - 1.0).abs() < 1e-9);
    }

    #[test]
    fn pearson_perfect_negative_correlation() {
        let x = vec![1.0, 2.0, 3.0, 4.0, 5.0];
        let y = vec![5.0, 4.0, 3.0, 2.0, 1.0];
        let c = pearson_correlation(&x, &y).unwrap();
        assert!((-1.0..-0.99).contains(&c));
    }

    #[test]
    fn pearson_too_short_returns_none() {
        assert!(pearson_correlation(&[1.0, 2.0], &[3.0, 4.0]).is_none());
    }

    #[test]
    fn stress_scenario_m10_basic() {
        let pos = vec![ps("a", 10000.0, Some("科技"), 50.0)];
        let sector: HashMap<String, f64> = [("科技".into(), 100.0)].into_iter().collect();
        let betas: BetaMap =
            [("a".to_string(), BetaEstimate::estimated(1.3, 100))].into_iter().collect();
        let r = run_stress_scenario(&pos, &sector, &betas, StressScenario::MarketDown10);
        // β=1.3（真实历史估计，显式注入）, m10=10%, sector_pct=100%>40% → β×1.2=1.56 → 单股 -15.6%
        assert!(
            r.portfolio_pnl_pct < -14.0 && r.portfolio_pnl_pct > -17.0,
            "pct = {}",
            r.portfolio_pnl_pct
        );
        assert_eq!(r.top_hit.as_ref().unwrap().stock_code, "a");
        assert!(r.note.contains("行业暴露>40%"), "note = {}", r.note);
        assert!(r.note.contains("真实历史 beta"), "note = {}", r.note);
        assert_eq!(r.beta_provenance.historical, 1);
        assert_eq!(r.beta_provenance.fallback, 0);
        assert!(r.beta_provenance.fallback_codes.is_empty());
    }

    #[test]
    fn stress_scenario_no_penalty_when_sector_low() {
        let pos = vec![ps("a", 10000.0, Some("科技"), 50.0)];
        let sector: HashMap<String, f64> = [("科技".into(), 30.0)].into_iter().collect();
        let betas: BetaMap =
            [("a".to_string(), BetaEstimate::estimated(1.3, 100))].into_iter().collect();
        let r = run_stress_scenario(&pos, &sector, &betas, StressScenario::MarketDown10);
        // sector_pct=30% < 40% 阈值 → 无惩罚，β=1.3 → 单股 -13%
        assert!(
            r.portfolio_pnl_pct < -12.0 && r.portfolio_pnl_pct > -14.0,
            "pct = {}",
            r.portfolio_pnl_pct
        );
        assert!(!r.note.contains("行业暴露>40%"), "note = {}", r.note);
    }

    #[test]
    fn stress_scenario_empty_positions() {
        let r =
            run_stress_scenario(&[], &HashMap::new(), &HashMap::new(), StressScenario::BlackSwan);
        assert_eq!(r.portfolio_pnl, 0.0);
        assert!(r.top_hit.is_none());
        assert!(r.note.contains("无持仓"));
    }

    #[test]
    fn concentration_warning_under_threshold() {
        assert!(compute_concentration_warning(20.0, 30.0, 5).is_none());
    }

    #[test]
    fn concentration_warning_multi_issues() {
        let w = compute_concentration_warning(45.0, 55.0, 2).unwrap();
        assert!(w.contains("单股集中度"));
        assert!(w.contains("行业暴露"));
        assert!(w.contains("分散度不足"));
    }

    #[test]
    fn diversification_score_buckets() {
        assert_eq!(compute_diversification_score(0, 0.0, 0.0), 0);
        assert_eq!(compute_diversification_score(2, 40.0, 50.0), 30);
        assert_eq!(compute_diversification_score(5, 20.0, 35.0), 70);
        assert_eq!(compute_diversification_score(10, 10.0, 20.0), 90);
    }

    #[test]
    fn run_all_scenarios_returns_three() {
        let pos = vec![ps("a", 10000.0, Some("银行"), 50.0)];
        let s = run_all_scenarios(&pos, &HashMap::new(), &HashMap::new());
        assert!(s.m10.is_some());
        assert!(s.m20.is_some());
        assert!(s.black_swan.is_some());
    }

    // ── β 取值来源（2026-09-14：真实历史 beta 取代行业关键词查表）──

    /// **负控**：无历史 beta 时必须回退中性 1.0，且这次退化必须**可见**。
    ///
    /// 这正是「静默降级」最爱的藏身处 —— 旧实现里所有未命中行业关键词的标的都
    /// 悄悄用 1.0，报表上看不出任何异常（实测 `stock_sector` 取值域是门类级，
    /// 与旧表的键大面积不匹配 ⇒ 除「金融」「化石能源」外全部落到 else 1.0）。
    #[test]
    fn stress_scenario_fallback_is_neutral_and_visible() {
        let pos = vec![ps("600000", 10000.0, Some("金融"), 50.0)];
        let r = run_stress_scenario(
            &pos,
            &HashMap::new(),
            &HashMap::new(),
            StressScenario::MarketDown10,
        );
        assert!(
            (r.portfolio_pnl_pct + 10.0).abs() < 1e-9,
            "回退 β=1.0 ⇒ 恰好 -10%，实际 {}",
            r.portfolio_pnl_pct
        );
        assert_eq!(r.beta_provenance.historical, 0);
        assert_eq!(r.beta_provenance.fallback, 1);
        assert_eq!(r.beta_provenance.fallback_codes, vec!["600000".to_string()]);
        assert!(r.note.contains("回退中性"), "note = {}", r.note);
    }

    /// 行为变更的显式留痕：旧口径下 `sector_beta("金融") == 0.5` ⇒ 同一输入是 -5%。
    /// 断言新口径**不再是** -5%，防止有人日后把行业查表悄悄加回来。
    #[test]
    fn stress_scenario_no_longer_uses_sector_keyword_table() {
        let pos = vec![ps("600000", 10000.0, Some("金融"), 50.0)];
        let r = run_stress_scenario(
            &pos,
            &HashMap::new(),
            &HashMap::new(),
            StressScenario::MarketDown10,
        );
        assert!(
            (r.portfolio_pnl_pct + 5.0).abs() > 1e-6,
            "旧口径 sector_beta(\"金融\")=0.5 仍然生效 ⇒ 行业查表被加了回来"
        );
    }

    // ── `date_returns` / `beta_from_aligned`（纯函数，无 IO）──

    fn mk_kline(date: &str, close: f64) -> KLine {
        KLine {
            date: date.to_string(),
            open: close,
            high: close,
            low: close,
            close,
            volume: 0.0,
            amount: 0.0,
            turnover_rate: None,
            adj_factor: None,
        }
    }

    /// 连续日期序列（不用真交易日历：两条序列共用同一日历即可）
    fn mk_dates(n: usize) -> Vec<String> {
        let start = chrono::NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
        (0..n)
            .map(|i| (start + chrono::Duration::days(i as i64)).format("%Y-%m-%d").to_string())
            .collect()
    }

    /// 用给定日期与日收益序列造 K 线（第 i 个收益作用于第 i+1 个 bar）
    fn klines_from_dates(dates: &[String], rets: &[f64]) -> Vec<KLine> {
        let mut close = 100.0;
        let mut out = vec![mk_kline(&dates[0], close)];
        for (i, r) in rets.iter().enumerate() {
            close *= 1.0 + r;
            out.push(mk_kline(&dates[i + 1], close));
        }
        out
    }

    #[test]
    fn date_returns_drops_pairs_involving_nonpositive_close() {
        let ks = vec![
            mk_kline("2026-01-01", 100.0),
            mk_kline("2026-01-02", 110.0),
            mk_kline("2026-01-03", 0.0), // 异常 bar（部分供应商停牌日给 0）
            mk_kline("2026-01-04", 121.0),
        ];
        let r = date_returns(&ks);
        assert_eq!(r.len(), 1, "0 价 bar 参与的两个配对都必须丢弃: {r:?}");
        assert_eq!(r[0].0, "2026-01-02", "收益挂在**后一根** bar 的日期上");
        assert!((r[0].1 - 0.10).abs() < 1e-12);
    }

    #[test]
    fn beta_from_aligned_recovers_known_slope() {
        // stock = 1.5 × market（严格线性、无噪声）⇒ 回归斜率必须精确等于 1.5
        let n = 60;
        let market: Vec<(String, f64)> = mk_dates(n)
            .into_iter()
            .enumerate()
            .map(|(i, d)| (d, ((i as f64 * 0.37).sin()) * 0.01))
            .collect();
        let stock: Vec<(String, f64)> = market.iter().map(|(d, r)| (d.clone(), r * 1.5)).collect();
        let (beta, samples) = beta_from_aligned(&stock, &market).expect("样本充足，应能估计");
        assert_eq!(samples, n);
        assert!((beta - 1.5).abs() < 1e-9, "beta = {beta}，应精确为 1.5");
    }

    /// 按**日期交集**对齐（而非按位置）：market 40 日、stock 只取 `dates[1..37]` 共 36 日
    /// ⇒ 重叠恰好 36；且 stock 收益在重叠区间上严格是 market 的 1.5 倍 ⇒ beta 必须精确 1.5。
    ///
    /// ⚠ 数据**必须带波动**：常数序列会让市场方差为 0，落到「不可估计」分支，
    /// 于是这条测试会以 `None` panic 而**看起来像对齐逻辑错了**（本轮实测踩到）。
    ///
    /// 本测试**自证区分力**：把同一组 stock 收益强行贴到 market **尾部 36 日**上（时间轴整体
    /// 后移 3 天），斜率必须显著偏离 1.5；否则说明这组数据对两种对齐等价，测了等于没测。
    #[test]
    fn beta_from_aligned_aligns_by_date_not_by_position() {
        let dates = mk_dates(40);
        let market: Vec<(String, f64)> = dates
            .iter()
            .enumerate()
            .map(|(i, d)| (d.clone(), (i as f64 * 0.41).sin() * 0.01))
            .collect();
        let stock: Vec<(String, f64)> =
            market[1..37].iter().map(|(d, r)| (d.clone(), r * 1.5)).collect();

        let (beta, samples) = beta_from_aligned(&stock, &market).expect("36 ≥ 30 应可估计");
        assert_eq!(samples, 36, "必须按日期交集对齐：40 ∩ 36 = 36");
        assert!((beta - 1.5).abs() < 1e-9, "严格线性 ⇒ beta 精确 1.5，实际 {beta}");

        // 错位对照：同一组收益挂到 market 尾部 36 日的日期上（只改日期，不改数值）
        let shifted: Vec<(String, f64)> =
            stock.iter().enumerate().map(|(k, (_, r))| (market[4 + k].0.clone(), *r)).collect();
        let (shifted_beta, _) = beta_from_aligned(&shifted, &market).expect("同样 36 个样本");
        assert!(
            (shifted_beta - 1.5).abs() > 0.05,
            "错位 3 天后仍得 {shifted_beta} ⇒ 本测试无法区分两种对齐，需换数据"
        );
    }

    #[test]
    fn beta_from_aligned_rejects_unestimable_input() {
        let dates = mk_dates(60);
        // ① 市场收益恒为常数 ⇒ 市场方差 0（compute_beta 内部判据）
        let flat_market: Vec<(String, f64)> = dates.iter().map(|d| (d.clone(), 0.0)).collect();
        let stock: Vec<(String, f64)> = dates.iter().map(|d| (d.clone(), 0.01)).collect();
        assert!(beta_from_aligned(&stock, &flat_market).is_none(), "市场方差 0 应判不可估计");
        // ② 重叠样本 < BETA_MIN_SAMPLES
        let few = &dates[..BETA_MIN_SAMPLES - 1];
        let m2: Vec<(String, f64)> = few.iter().map(|d| (d.clone(), 0.001)).collect();
        let s2: Vec<(String, f64)> = few.iter().map(|d| (d.clone(), 0.002)).collect();
        assert!(
            beta_from_aligned(&s2, &m2).is_none(),
            "样本 < {BETA_MIN_SAMPLES} 必须判不可估计（而非用够不着的样本硬算）"
        );
    }

    // ── `estimate_betas`：拉取层（用测试替身）──

    use axagent_harness::core_error::AxAgentError;
    use axagent_harness::market_data::{StockQuote, StockSearchResult};

    /// 可编程的 `MarketDataProvider` 替身。
    ///
    /// 记录每次 `get_klines` 的 `(code, adj_type)` —— 用于钉住「基准不复权 / 个股前复权」
    /// 这一口径选择；否则它只是两行裸参数，改动无人拦。
    // SAFETY: 此处 std::sync::Mutex 不跨 await 使用 —— `seen` 仅在同步临界区内读写，
    // lock guard 是语句级临时量，在函数内任何 `.await` 之前就已 drop。
    // 依据 `clippy.toml` 的合法例外规则（铁律 #8 只禁「跨 await 的 std guard」）。
    #[allow(clippy::disallowed_types)]
    struct MockProvider {
        series: HashMap<String, Vec<KLine>>,
        seen: std::sync::Mutex<Vec<(String, Option<AdjType>)>>,
    }

    // SAFETY: 同上 —— `new` / `adj_for` 均为同步函数，临界区内无 await 点。
    #[allow(clippy::disallowed_types)]
    impl MockProvider {
        fn new(series: HashMap<String, Vec<KLine>>) -> Self {
            Self { series, seen: std::sync::Mutex::new(Vec::new()) }
        }
        fn adj_for(&self, code: &str) -> Option<Option<AdjType>> {
            self.seen.lock().unwrap().iter().find(|(c, _)| c.as_str() == code).map(|(_, a)| *a)
        }
    }

    #[async_trait::async_trait]
    impl MarketDataProvider for MockProvider {
        async fn get_quote(
            &self,
            _stock_code: &str,
        ) -> axagent_harness::core_error::Result<StockQuote> {
            Err(AxAgentError::Provider("mock: get_quote 未实现".into()))
        }

        async fn get_klines(
            &self,
            stock_code: &str,
            _period: &str,
            _limit: u32,
            adj_type: Option<AdjType>,
        ) -> axagent_harness::core_error::Result<Vec<KLine>> {
            self.seen.lock().unwrap().push((stock_code.to_string(), adj_type));
            self.series
                .get(stock_code)
                .cloned()
                .ok_or_else(|| AxAgentError::Provider(format!("mock: 无 {stock_code} 数据")))
        }

        async fn search_stock(
            &self,
            _keyword: &str,
        ) -> axagent_harness::core_error::Result<Vec<StockSearchResult>> {
            Ok(vec![])
        }
    }

    #[tokio::test]
    async fn estimate_betas_uses_real_regression_and_correct_adj_type() {
        let n = 60;
        let dates = mk_dates(n);
        let rets: Vec<f64> = (0..n - 1).map(|i| ((i as f64 * 0.41).cos()) * 0.012).collect();
        let market = klines_from_dates(&dates, &rets);
        let stock_rets: Vec<f64> = rets.iter().map(|r| r * 2.0).collect();
        let stock = klines_from_dates(&dates, &stock_rets);
        let mut series = HashMap::new();
        series.insert(BETA_BENCHMARK_CODE.to_string(), market);
        series.insert("600000".to_string(), stock);
        let p = MockProvider::new(series);

        let positions = vec![ps("600000", 10000.0, None, 50.0)];
        let m = estimate_betas(&p, &positions, 120).await;

        let e = m.get("600000").copied().expect("每个持仓都必须有条目（含回退）");
        assert!(e.historical, "数据齐全时应给出真实历史估计");
        assert!((e.beta - 2.0).abs() < 1e-6, "beta = {}，应≈2.0", e.beta);
        assert_eq!(e.samples, n - 1);
        // 口径钉死
        assert_eq!(p.adj_for(BETA_BENCHMARK_CODE), Some(None), "基准指数无复权概念");
        assert_eq!(
            p.adj_for("600000"),
            Some(Some(AdjType::Forward)),
            "个股必须前复权 —— 除权日的假收益会直接污染 beta"
        );
    }

    #[tokio::test]
    async fn estimate_betas_falls_back_when_benchmark_unavailable() {
        // 基准也拉不到 ⇒ 无法估计任何标的，必须显式回退（而不是跳过条目）
        let p = MockProvider::new(HashMap::new());
        let positions = vec![ps("600000", 10000.0, None, 50.0)];
        let m = estimate_betas(&p, &positions, 120).await;
        let e = m.get("600000").copied().expect("回退也必须有条目");
        assert!(!e.historical);
        assert!((e.beta - DEFAULT_BETA).abs() < 1e-12);
        assert_eq!(e.samples, 0);
    }

    #[tokio::test]
    async fn estimate_betas_falls_back_when_stock_klines_fail() {
        let n = 60;
        let dates = mk_dates(n);
        let rets: Vec<f64> = (0..n - 1).map(|i| ((i as f64 * 0.41).cos()) * 0.012).collect();
        let mut series = HashMap::new();
        series.insert(BETA_BENCHMARK_CODE.to_string(), klines_from_dates(&dates, &rets));
        // 故意不提供个股 ⇒ 该股回退，但基准存在（不被基准缺失的早退分支吞掉）
        let p = MockProvider::new(series);
        let positions = vec![ps("600000", 10000.0, None, 50.0)];
        let m = estimate_betas(&p, &positions, 120).await;
        assert!(!m["600000"].historical);
        assert!((m["600000"].beta - DEFAULT_BETA).abs() < 1e-12);
    }

    #[tokio::test]
    async fn estimate_betas_empty_positions_is_empty_without_any_fetch() {
        let p = MockProvider::new(HashMap::new());
        let m = estimate_betas(&p, &[], 120).await;
        assert!(m.is_empty());
        assert!(p.seen.lock().unwrap().is_empty(), "无持仓时不应发起任何取数");
    }

    /// G-3：刷新一次要把**非零**的最大回撤与两个只数一起落库。
    ///
    /// 改造前 `max_drawdown_pct` 这一列恒 0（`compute_dashboard` 写死、`refresh_metrics`
    /// 原样写回，注释却声称「走历史」）。夹具日期一律由「北京今天」往回推 ⇒ 保证
    /// 落在生产查询的 90 日回看窗内（写死日期会在几周后静默失效，门却照绿）。
    #[tokio::test]
    async fn refresh_metrics_persists_nonzero_drawdown_and_position_counts() {
        use axagent_entities::portfolio_metrics_daily;
        let h = axagent_dao::db::create_test_pool().await.expect("测试库应可创建");
        let db = &h.conn;
        let today = axagent_astock_data::calendar::beijing_today();

        async fn seed(db: &DatabaseConnection, date: &str, mv: f64) {
            let row = portfolio_metrics_daily::ActiveModel {
                id: Set(Uuid::new_v4().to_string()),
                snapshot_date: Set(date.to_string()),
                total_market_value: Set(mv),
                cash_pct: Set(0.0),
                total_pnl: Set(0.0),
                total_pnl_pct: Set(0.0),
                max_drawdown_pct: Set(0.0),
                beta: Set(None),
                sharpe_30d: Set(None),
                correlation_avg: Set(None),
                top_concentration_pct: Set(0.0),
                position_count: Set(None),
                unpriced_count: Set(None),
                sector_exposure_json: Set("{}".to_string()),
                stress_test_json: Set(None),
                created_at: Set(1),
            };
            row.insert(db).await.unwrap();
        }
        // 历史：9000 → 12000（峰）→ 8800；本次市值 6000（谷）⇒ 回撤 (12000-6000)/12000 = 50%
        for (offset, mv) in [(20_i64, 9000.0_f64), (10, 12000.0), (3, 8800.0)] {
            let d = (today - chrono::Duration::days(offset)).format("%Y-%m-%d").to_string();
            seed(db, &d, mv).await;
        }

        let pos = vec![ps("a", 6000.0, Some("科技"), 50.0), ps_unpriced("b", 50.0)];
        let (id, written) =
            refresh_metrics(db, &pos, &BetaMap::new(), None, None, None, None).await.unwrap();
        // 2 = 覆盖的**持仓只数**（含缺价那只），不是「写了 1 行」。命令面与前端把这个字段
        // 叫 positionsSnapshotted ⇒ 恒返 1 会让「8 只持仓」显示成 1（G-9 订正）。
        assert_eq!(written, 2, "第二条返回的是覆盖的持仓只数");

        let row = portfolio_metrics_daily::Entity::find_by_id(id)
            .one(db)
            .await
            .unwrap()
            .expect("刷新必须留下那一行");
        assert!(
            (row.max_drawdown_pct - 50.0).abs() < 1e-6,
            "峰值 12000 → 谷值 6000 应得 50%，实得 {}",
            row.max_drawdown_pct
        );
        assert_eq!(row.position_count, Some(1), "只有拿到现价的那只参与判据");
        assert_eq!(row.unpriced_count, Some(1), "缺价只数要一起落库");
        assert_eq!(row.snapshot_date, today.format("%Y-%m-%d").to_string(), "日期按北京会话日盖");
    }

    /// 回撤序列的过滤规则（三条各一条测试，见下面三个用例的注释）。
    ///
    /// 第一版我把「空组合行」放在序列**开头**并断言结果为 0 ⇒ 去掉过滤的变异体也是 0
    /// （0 在低点不构成回撤）、把 `lt` 改成 `lte` 也是 0 ⇒ 两个变异体都绿，探针根本没打在
    /// 决定该断言的那道守卫上。现在 0 行/同日行/窗口外行一律放在**能改变读数**的位置。
    async fn seed_day(db: &DatabaseConnection, day: chrono::NaiveDate, mv: f64) {
        use axagent_entities::portfolio_metrics_daily;
        portfolio_metrics_daily::ActiveModel {
            id: Set(Uuid::new_v4().to_string()),
            snapshot_date: Set(day.format("%Y-%m-%d").to_string()),
            total_market_value: Set(mv),
            cash_pct: Set(0.0),
            total_pnl: Set(0.0),
            total_pnl_pct: Set(0.0),
            max_drawdown_pct: Set(0.0),
            beta: Set(None),
            sharpe_30d: Set(None),
            correlation_avg: Set(None),
            top_concentration_pct: Set(0.0),
            position_count: Set(None),
            unpriced_count: Set(None),
            sector_exposure_json: Set("{}".to_string()),
            stress_test_json: Set(None),
            created_at: Set(1),
        }
        .insert(db)
        .await
        .unwrap();
    }

    /// 回撤按**日期升序**的峰谷算：峰 12000（20 天前）→ 谷 8800（5 天前）→ 本次回升到 11000
    /// ⇒ 读数应是「这次下跌的最深一段」26.67%，不是「最后一段」8.33%。
    /// 序列里那行 0（空组合日）由 `max_drawdown_fraction` 自己跳过 —— 本层不再重复过滤。
    #[tokio::test]
    async fn drawdown_uses_peak_and_trough_in_date_order() {
        let h = axagent_dao::db::create_test_pool().await.expect("测试库应可创建");
        let db = &h.conn;
        let today = axagent_astock_data::calendar::beijing_today();
        seed_day(db, today - chrono::Duration::days(20), 12000.0).await;
        seed_day(db, today - chrono::Duration::days(10), 0.0).await;
        seed_day(db, today - chrono::Duration::days(5), 8800.0).await;

        let dd = drawdown_from_history(db, &today.format("%Y-%m-%d").to_string(), 11000.0)
            .await
            .unwrap();
        assert!(
            (dd - 80.0 / 3.0).abs() < 1e-6,
            "序列必须按日期升序喂进去（改成降序会读成 8.33%）：实得 {dd}"
        );
    }

    /// 同一天已落的那行是**自己**，不能被当成历史的一部分（严格早于 before_date）。
    #[tokio::test]
    async fn drawdown_excludes_the_same_day_row() {
        let h = axagent_dao::db::create_test_pool().await.expect("测试库应可创建");
        let db = &h.conn;
        let today = axagent_astock_data::calendar::beijing_today();
        let today_str = today.format("%Y-%m-%d").to_string();
        seed_day(db, today - chrono::Duration::days(20), 12000.0).await;
        seed_day(db, today, 3000.0).await;

        let dd = drawdown_from_history(db, &today_str, 8800.0).await.unwrap();
        assert!(
            (dd - 80.0 / 3.0).abs() < 1e-6,
            "今天那行（3000）不该进序列，否则会被读成跌过 75%：实得 {dd}"
        );
    }

    /// 窗口外的老行不参与「近期回撤」：与 `sharpe_30d` 一类口径保持一致。
    #[tokio::test]
    async fn drawdown_honors_the_lookback_window() {
        let h = axagent_dao::db::create_test_pool().await.expect("测试库应可创建");
        let db = &h.conn;
        let today = axagent_astock_data::calendar::beijing_today();
        let today_str = today.format("%Y-%m-%d").to_string();
        seed_day(db, today - chrono::Duration::days(200), 12000.0).await;
        seed_day(db, today - chrono::Duration::days(20), 9000.0).await;

        let dd = drawdown_from_history(db, &today_str, 8800.0).await.unwrap();
        assert!(
            (dd - (9000.0 - 8800.0) / 9000.0 * 100.0).abs() < 1e-6,
            "200 天前那行在 90 日窗口之外，不该把回撤撑到 26.7%：实得 {dd}"
        );
    }

    /// 面板值 → 回看窗口：区间内采纳、区间外回落、非数值回落、键缺席回落 —— 四格各一条。
    ///
    /// 只有**纯函数版**有可测面：快照版会改进程内全局变量表，与并行用例互踩
    /// （同 `PositionLimits::from_panel_vars` 的拆法）。
    #[test]
    fn correlation_lookback_window_adopts_panel_value_only_within_range() {
        let vars = |v: serde_json::Value| {
            let mut m = HashMap::new();
            m.insert("risk_correlation_lookback_days".to_string(), v);
            m
        };
        assert_eq!(
            correlation_lookback_days_in(&vars(serde_json::json!(90))),
            90,
            "区间内必须采纳"
        );
        assert_eq!(correlation_lookback_days_in(&vars(serde_json::json!(20))), 20, "下界本身可用");
        assert_eq!(
            correlation_lookback_days_in(&vars(serde_json::json!(250))),
            250,
            "上界本身可用"
        );
        assert_eq!(
            correlation_lookback_days_in(&vars(serde_json::json!(19))),
            CORRELATION_LOOKBACK_DAYS,
            "过小（重叠收益不足以算相关）⇒ 回落"
        );
        assert_eq!(
            correlation_lookback_days_in(&vars(serde_json::json!(251))),
            CORRELATION_LOOKBACK_DAYS,
            "超过一年 ⇒ 回落，别把停牌期外的旧价对拉进窗口"
        );
        assert_eq!(
            correlation_lookback_days_in(&vars(serde_json::json!("60"))),
            CORRELATION_LOOKBACK_DAYS,
            "非数值 ⇒ 回落"
        );
        assert_eq!(
            correlation_lookback_days_in(&HashMap::new()),
            CORRELATION_LOOKBACK_DAYS,
            "键缺席（变量表未装）⇒ 回落默认，与接线前的现值逐字相同"
        );
    }

    /// G-7：`refresh_snapshot` 是面板「立即刷新」与收盘定时任务共用的**唯一**通路，
    /// 三步（β → 指标落库 → 相关性落库）必须一次跑完并各自落到自己的表里。
    ///
    /// 判据分两层：先看返回值，再看**产物表真有几行** —— 只看前者会放过「函数被调用、
    /// 但写库那步静默失败」的形态。
    #[tokio::test]
    async fn refresh_snapshot_runs_beta_metrics_and_correlation_in_one_pass() {
        use axagent_entities::{portfolio_correlation_snapshot, portfolio_metrics_daily};

        let h = axagent_dao::db::create_test_pool().await.expect("测试库应可创建");
        let db = &h.conn;

        let n = 60;
        let dates = mk_dates(n);
        let rets: Vec<f64> = (0..n - 1).map(|i| ((i as f64 * 0.41).cos()) * 0.012).collect();
        let double: Vec<f64> = rets.iter().map(|r| r * 2.0).collect();
        let inverse: Vec<f64> = rets.iter().map(|r| -r).collect();
        let mut series = HashMap::new();
        series.insert(BETA_BENCHMARK_CODE.to_string(), klines_from_dates(&dates, &rets));
        series.insert("600000".to_string(), klines_from_dates(&dates, &double));
        series.insert("600001".to_string(), klines_from_dates(&dates, &inverse));
        let p = MockProvider::new(series);

        let positions = vec![
            ps("600000", 10000.0, Some("科技"), 50.0),
            ps("600001", 5000.0, Some("医药"), 20.0),
        ];
        let summary = refresh_snapshot(db, &p, &positions, None).await.unwrap();

        assert_eq!(summary.positions_snapshotted, 2, "返回的是覆盖的持仓只数，不是行数");
        assert!(
            summary.correlation_pairs_written >= 1,
            "两只持仓至少该算出一对相关性，实得 {}",
            summary.correlation_pairs_written
        );
        assert!(summary.as_of_date.is_none(), "cron 侧不传日期 ⇒ 由引擎按北京会话日盖当天");
        // β 确实被算过（三步链的第一步不是摆设）：基准 + 两只个股都发起过取数
        assert!(p.adj_for("600000").is_some(), "β 与相关性都要走过个股日 K 的取数");

        assert_eq!(
            portfolio_metrics_daily::Entity::find().all(db).await.unwrap().len(),
            1,
            "指标表应恰好落一行"
        );
        let pairs = portfolio_correlation_snapshot::Entity::find().all(db).await.unwrap();
        assert!(!pairs.is_empty(), "相关性快照必须真落库，不只是算完返回个数");
        assert_eq!(
            pairs[0].lookback_days, CORRELATION_LOOKBACK_DAYS as i32,
            "变量表未装 ⇒ 用回落默认"
        );
    }
}
