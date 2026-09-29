//! 本地估值算法 —— 反向 DCF + 相对估值路由（2026-09-28）
//!
//! ## 为什么独立成模块（用户裁决「推翻现有估值架构」）
//!
//! 旧架构的估值腿（正向 DCF 三档 + 历史估值带）在**一大类标的上系统性失效**：
//! 华大九天（301269）现价 87.56 元，DCF 给 `2.16–4.42 元`（−97.5%）。
//! 复算证明**不是算错，而是问错了问题**：DCF 锚定当期 FCF（0.686 亿，FCF 收益率
//! 0.14%），而市场按 10 年后的现金流定价 —— 对「当期几乎不产生 FCF」的成长股，
//! 正向 DCF 结构上无法表达其定价逻辑（要让 DCF 等于现价，5 年 FCF 须复合 +130%/年）。
//!
//! 故本模块补两件旧架构缺的能力：
//!
//! 1. **反向 DCF**（[`reverse_dcf`]）：由现价反解「市场隐含的 FCF 复合增速」，
//!    把「估值贵不贵」转成「市场假设是否可信」。这是**永远有答案**的口径 ——
//!    它不需要模型前提成立，只需要一个正锚（锚的来源与置信度另由调用方披露）。
//! 2. **二阶段 DCF 现值**（[`two_stage_dcf`]）：全仓**唯一**实现，正向 DCF
//!    与反向 DCF 共用，避免同一公式两份实现漂移（铁律 12）。

use serde::Serialize;

/// 二阶段 DCF 现值 —— 全仓唯一实现（正向 DCF 与反向 DCF 共用）。
///
/// `fcf_ps` 当期每股 FCF（元/股）；`g` 预测期增长率；`p` 永续增长率；
/// `d` 折现率；`years` 预测期年数；`min_terminal_spread` 终值分母利差地板。
/// 返回 `(总现值, 永续终值现值)`。
///
/// ⚠️ `years` / `min_terminal_spread` 由调用方显式传入（而非读模块常量）：
/// 本模块不持有估值参数常量，参数权威来源仍是 `mcp_tools` 的常量区
/// （`FORECAST_YEARS` / `MIN_TERMINAL_SPREAD`），此处只是**公式**的唯一实现。
pub fn two_stage_dcf(
    fcf_ps: f64,
    g: f64,
    p: f64,
    d: f64,
    years: i32,
    min_terminal_spread: f64,
) -> (f64, f64) {
    let mut pv = 0.0;
    let mut current_fcf = fcf_ps;
    for year in 1..=years {
        current_fcf *= 1.0 + g;
        pv += current_fcf / (1.0 + d).powi(year);
    }
    let terminal_fcf = current_fcf * (1.0 + p);
    // 第二道保险：正常配置下 `p` 已被上游钳到 `d − 地板` 之下，本行不可达。
    let terminal_spread = (d - p).max(min_terminal_spread);
    let terminal_value = terminal_fcf / terminal_spread;
    let terminal_pv = terminal_value / (1.0 + d).powi(years);
    (pv + terminal_pv, terminal_pv)
}

/// 反向 DCF 的搜索下界（年复合 −95%，极端萎缩）。
const REVERSE_DCF_MIN_G: f64 = -0.95;
/// 反向 DCF 的搜索上界（年复合 +900%）。越过此上界说明现价无法用任何合理
/// 增长率解释 ⇒ 结果标记 `impliedCagrExceedsRange`，结论层据此判「定价与现金流脱钩」。
const REVERSE_DCF_MAX_G: f64 = 9.0;
/// 二分迭代次数（区间宽 9.95 ⇒ 2⁻¹⁰⁰ 已远超 f64 精度，取 100 次足够且开销可忽略）。
const REVERSE_DCF_ITERS: u32 = 100;

/// 可行性阈值：隐含第 5 年 FCF 占预测期营收之比。
/// `> 1.0` ⇒ 物理不可能（现金流超过全部营收）。
const IMPLIED_MARGIN_IMPOSSIBLE: f64 = 1.0;
/// `(0.5, 1.0]` ⇒ 极度紧张（一家公司要把一半以上营收变成自由现金流）。
const IMPLIED_MARGIN_STRAINED: f64 = 0.5;

/// 反向 DCF 输入（与正向 DCF 同源，避免调用方各自拼装参数）。
#[derive(Debug, Clone, Copy)]
pub struct ReverseDcfInputs {
    /// 锚定 FCF 总额（元/年）—— 与正向 DCF 的 `fcf_anchor` 同源（恒 > 0）
    pub fcf_anchor: f64,
    /// 总股本（股）
    pub total_shares: f64,
    /// 现价（元/股）
    pub current_price: f64,
    /// 折现率（小数）
    pub discount_rate: f64,
    /// 永续增长率（小数）
    pub perpetual_growth: f64,
    /// 终值分母利差地板（小数）
    pub min_terminal_spread: f64,
    /// 预测期年数
    pub forecast_years: i32,
    /// 当期营收（元）—— 用于把隐含 FCF 换算成「隐含利润率」
    pub revenue_0: Option<f64>,
    /// 当期营收同比（**百分数**，如 32.5 表示 +32.5%）—— 用于推预测期营收
    pub revenue_yoy: Option<f64>,
}

/// 反向 DCF 结果：把「现价」翻译成「市场隐含的现金流假设」。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReverseDcf {
    /// 隐含的预测期 FCF 年复合增速（小数）；负值表示市场隐含萎缩
    pub implied_cagr: f64,
    /// 隐含的 FCF 预测期总倍数 = `(1 + g)^years`
    pub implied_multiple: f64,
    /// 隐含的预测期末年 FCF 总额（元）
    pub implied_fcf: f64,
    /// 隐含的预测期末年每股 FCF（元/股）
    pub implied_fcf_per_share: f64,
    /// 预测期年数（口径披露：隐含增速是「多少年内」的复合）
    pub forecast_years: i32,
    /// 隐含末年 FCF / 当期营收（倍）—— 无营收数据时为 `None`
    pub implied_fcf_to_current_revenue: Option<f64>,
    /// 隐含末年 FCF / 预测期末年营收（= 隐含 FCF 利润率）—— 无营收数据时为 `None`
    pub implied_fcf_margin_on_implied_revenue: Option<f64>,
    /// 可行性：`Impossible`（物理不可能）/ `Strained`（极度紧张）/ `Plausible`（可行）
    pub feasibility: String,
    /// 隐含增速是否顶到搜索上界（现价无法用 ≤900%/年 的增速解释）
    pub exceeds_search_range: bool,
    /// 口径说明（锚来源 + 判据），供面板与 LLM 原文引用
    pub note: String,
}

/// 是否**可用的正数** —— 拒绝 `0` / 负数 / `NaN`。
///
/// ⚠️ 不要改写成 `v <= 0.0` 取反：NaN 与任何值比较皆为 `false`（`NaN <= 0.0` 亦为假），
/// 单靠 `<=` 会让 NaN 冒充「正数」通过校验；`!(v > 0.0)` 才是正确的拒绝式，
/// 而 clippy 的 `neg_cmp_op_on_partial_ord` 不接受 `!` 直接套比较，故封成此函数。
fn is_usable_positive(v: f64) -> bool {
    v > 0.0 && !v.is_nan()
}

/// 由现价反解市场隐含的 FCF 复合增速。
///
/// ## 算法
///
/// 二分法解 `two_stage_dcf(fcf_ps, g, p, d, years, spread).0 == current_price`，
/// 搜索 `g ∈ [−0.95, 9.0]`。`PV(g)` 关于 `g` **单调递增**（`fcf_ps > 0`、`g > −1`
/// 时每一项都随 `g` 增大）⇒ 二分法收敛且解唯一。
///
/// ## 可行性判据（为什么用「隐含利润率」而不是「增速」）
///
/// 同样 +50% 的复合增速，对高毛利软件公司与低毛利制造业的可信度完全不同。
/// 故用**可验证的经营约束**判定：把隐含末年 FCF 换算成对预测期营收的占比，
/// `> 100%` 即物理不可能（自由现金流不可能超过全部营收 —— 公司还要付成本）。
///
/// 无营收数据时退回「增速阈值」口径（`> 100%/年` 判不可能），并在 `note` 中说明。
///
/// 锚缺失或非正（`fcf_anchor ≤ 0` / 股本非正 / 现价非正）⇒ 返回 `None`。
/// ⚠️ 生产路径上正向 DCF 的锚**恒 > 0**（fallback 锚 = 近 5 年正净利均值 × 0.90，
/// 仅在「持续亏损」时整体不可用），故反向 DCF 的可用面**显著大于**正向 DCF。
pub fn reverse_dcf(inputs: ReverseDcfInputs) -> Option<ReverseDcf> {
    if !is_usable_positive(inputs.fcf_anchor)
        || !is_usable_positive(inputs.total_shares)
        || !is_usable_positive(inputs.current_price)
    {
        return None;
    }
    let fcf_ps = inputs.fcf_anchor / inputs.total_shares;
    let pv_at = |g: f64| {
        two_stage_dcf(
            fcf_ps,
            g,
            inputs.perpetual_growth,
            inputs.discount_rate,
            inputs.forecast_years,
            inputs.min_terminal_spread,
        )
        .0
    };

    let target = inputs.current_price;
    let mut exceeds_search_range = false;
    let g = if target >= pv_at(REVERSE_DCF_MAX_G) {
        exceeds_search_range = true;
        REVERSE_DCF_MAX_G
    } else if target <= pv_at(REVERSE_DCF_MIN_G) {
        REVERSE_DCF_MIN_G
    } else {
        let mut lo = REVERSE_DCF_MIN_G;
        let mut hi = REVERSE_DCF_MAX_G;
        for _ in 0..REVERSE_DCF_ITERS {
            let mid = 0.5 * (lo + hi);
            if pv_at(mid) < target {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        0.5 * (lo + hi)
    };

    let implied_multiple = (1.0 + g).powi(inputs.forecast_years);
    let implied_fcf = inputs.fcf_anchor * implied_multiple;
    let implied_fcf_per_share = fcf_ps * implied_multiple;

    // 预测期末年营收：按当期营收同比复合（缺失则假设零增长，属**偏严格**口径 ——
    // 营收不增长时对 FCF 利润率的要求更高，故不会把不可能说成可行）。
    let g_rev = inputs.revenue_yoy.unwrap_or(0.0) / 100.0;
    let implied_revenue = inputs
        .revenue_0
        .filter(|r| *r > 0.0)
        .map(|r| r * (1.0 + g_rev).powi(inputs.forecast_years));
    let implied_fcf_to_current_revenue =
        inputs.revenue_0.filter(|r| *r > 0.0).map(|r| implied_fcf / r);
    let implied_fcf_margin_on_implied_revenue = implied_revenue.map(|r| implied_fcf / r);

    let (feasibility, basis_text) = match implied_fcf_margin_on_implied_revenue {
        Some(margin) if margin > IMPLIED_MARGIN_IMPOSSIBLE => (
            "Impossible",
            format!(
                "隐含末年 FCF / 预测期营收 = {:.0}% > 100%（自由现金流超过全部营收，物理不可能）",
                margin * 100.0
            ),
        ),
        Some(margin) if margin > IMPLIED_MARGIN_STRAINED => (
            "Strained",
            format!(
                "隐含末年 FCF / 预测期营收 = {:.0}%（须把 {:.0}% 以上营收转为自由现金流，极度紧张）",
                margin * 100.0,
                margin * 100.0
            ),
        ),
        Some(margin) => (
            "Plausible",
            format!("隐含末年 FCF / 预测期营收 = {:.0}%（经营上可达）", margin * 100.0),
        ),
        None => {
            if g > 1.0 {
                ("Impossible", format!("无营收数据可校验；隐含 FCF 年复合增速 {:.0}% > 100%", g * 100.0))
            } else if g > 0.5 {
                ("Strained", format!("无营收数据可校验；隐含 FCF 年复合增速 {:.0}% 偏高", g * 100.0))
            } else {
                ("Plausible", format!("无营收数据可校验；隐含 FCF 年复合增速 {:.0}%", g * 100.0))
            }
        },
    };

    let range_text = if exceeds_search_range {
        format!(
            "⚠️ 现价超出本模型搜索上界（FCF 年复合 {:.0}%），隐含增速 ≥ {:.0}% —— 定价与当期现金流完全脱钩",
            REVERSE_DCF_MAX_G * 100.0,
            REVERSE_DCF_MAX_G * 100.0
        )
    } else {
        String::new()
    };
    let note = format!(
        "由现价 {:.2} 元反解：要让当前锚定 FCF（{:.2} 亿/年）支撑现价，\
         需在 {} 年内以 {:.1}% 年复合增长至 {:.2} 亿（末年每股 {:.2} 元）。判据：{}。{}",
        inputs.current_price,
        inputs.fcf_anchor / 1e8,
        inputs.forecast_years,
        g * 100.0,
        implied_fcf / 1e8,
        implied_fcf_per_share,
        basis_text,
        range_text
    );

    Some(ReverseDcf {
        implied_cagr: g,
        implied_multiple,
        implied_fcf,
        implied_fcf_per_share,
        forecast_years: inputs.forecast_years,
        implied_fcf_to_current_revenue,
        implied_fcf_margin_on_implied_revenue,
        feasibility: feasibility.to_string(),
        exceeds_search_range,
        note,
    })
}

// ── 相对估值：按数据形态路由有效指标（2026-09-28）──────────────────────────
//
// 旧架构的定性判定只用 PE + PB 的历史分位（`valuation_band::verdict_from_bands`），
// 而 301269 实测「PE 高得无意义（盈利趋零）、PB 分位 15.5%、PS 分位 1.04%」——
// PS 从未参与判定，于是「4 年最便宜的 PS」被完全忽略。本层把「哪个指标有效」
// 做成显式路由，并接入同行分位。

/// PE 有效性上界：`0 < PE < 150` 视为有效。
///
/// 为什么需要上界：亏损公司 PE 为负（无效），而「微利 + 高股价」会把 PE 顶到数百
/// 甚至上千 —— 此时 PE 已不含定价信息（分母趋零），拿它判分位会得到
/// 「PE 分位极低 ⇒ 低估」这类反向结论。高于本上界的标的应由 PS 主导。
pub const PE_VALID_MAX: f64 = 150.0;

/// 单个估值指标的输入。
#[derive(Debug, Clone)]
pub struct MetricInput {
    /// 指标名（`PE` / `PS` / `PB`）
    pub name: &'static str,
    /// 当前值（缺数据为 `None`）
    pub value: Option<f64>,
    /// 当前值在**自身历史**分布中的分位（0–100）
    pub percentile: Option<f64>,
    /// 同行样本值（算同行分位与中位数用）
    pub peer_values: Vec<f64>,
}

/// 单指标读数（输出）。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MetricReading {
    pub name: String,
    pub value: Option<f64>,
    /// 该指标是否**有效**（可参与判定）
    pub valid: bool,
    /// 无效原因（有效时为空串）
    pub invalid_reason: String,
    /// 自身历史分位
    pub percentile: Option<f64>,
    /// 同行分位（在同行样本中的相对位置）
    pub peer_percentile: Option<f64>,
    /// 同行中位数
    pub peer_median: Option<f64>,
    pub peer_count: usize,
}

/// 相对估值（输出）。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RelativeValuation {
    /// 主指标名（首个有效者，按输入顺序即 PE → PS → PB 优先级）
    pub primary: Option<String>,
    /// 综合口径：`deep_value` / `undervalued` / `fair` / `expensive` / `overvalued` / `unknown`
    pub verdict: String,
    pub metrics: Vec<MetricReading>,
    /// 口径说明（未纳入判定的指标及原因）
    pub note: String,
}

/// 指标有效性判定 —— 锚定**数据形态**，不锚定行业标签。
fn metric_validity(name: &str, value: Option<f64>) -> (Option<f64>, String) {
    match value {
        None => (None, "数据缺失".to_string()),
        Some(v) if !is_usable_positive(v) => {
            (None, format!("{name} ≤ 0（亏损 / 净资产或营收为负），无金融含义"))
        },
        Some(v) if name == "PE" && v >= PE_VALID_MAX => {
            (None, format!("PE = {v:.1} ≥ {PE_VALID_MAX:.0}（盈利趋零，PE 已不含定价信息）"))
        },
        Some(v) => (Some(v), String::new()),
    }
}

/// 中位数（偶数个取中间两个的均值）；空样本返回 `None`。
fn median_of(mut v: Vec<f64>) -> Option<f64> {
    if v.is_empty() {
        return None;
    }
    v.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let n = v.len();
    Some(if n % 2 == 1 {
        v[n / 2]
    } else {
        (v[n / 2 - 1] + v[n / 2]) / 2.0
    })
}

/// 相对估值：按数据形态路由有效指标 + 同行分位。
///
/// `inputs` 的**顺序即优先级**（调用方传 `[PE, PS, PB]`）。
/// `verdict` 只用**有效**指标的历史分位（缺失时回落同行分位）计算，
/// 阈值与估值带共用 [`crate::valuation_band::verdict_from_avg_percentile`]。
pub fn relative_valuation(inputs: &[MetricInput]) -> RelativeValuation {
    let mut readings = Vec::new();
    for m in inputs {
        let (valid_value, invalid_reason) = metric_validity(m.name, m.value);
        let peer_percentile = match valid_value {
            Some(v) if !m.peer_values.is_empty() => {
                Some(crate::valuation_band::current_percentile(&m.peer_values, v))
            },
            _ => None,
        };
        let peer_median = median_of(m.peer_values.clone());
        readings.push(MetricReading {
            name: m.name.to_string(),
            value: m.value,
            valid: valid_value.is_some(),
            invalid_reason,
            percentile: if valid_value.is_some() {
                m.percentile
            } else {
                None
            },
            peer_percentile,
            peer_median,
            peer_count: m.peer_values.len(),
        });
    }

    let primary = readings.iter().find(|r| r.valid).map(|r| r.name.clone());

    // 历史分位优先；历史缺失时用同行分位。只取**有效**指标。
    let pcts: Vec<f64> = readings
        .iter()
        .filter(|r| r.valid)
        .filter_map(|r| r.percentile.or(r.peer_percentile))
        .collect();
    let verdict = if pcts.is_empty() {
        "unknown".to_string()
    } else {
        let avg = pcts.iter().sum::<f64>() / pcts.len() as f64;
        crate::valuation_band::verdict_from_avg_percentile(avg).to_string()
    };

    let invalid: Vec<String> = readings
        .iter()
        .filter(|r| !r.valid)
        .map(|r| format!("{}（{}）", r.name, r.invalid_reason))
        .collect();
    let note = if invalid.is_empty() {
        "全部指标有效".to_string()
    } else {
        format!("未纳入判定的指标：{}", invalid.join("；"))
    };

    RelativeValuation { primary, verdict, metrics: readings, note }
}

// ── 结论合成（2026-09-28）──────────────────────────────────────────────────
//
// 用户硬约束：**必须给出结论**，严禁「一路加判据导致全部标的被排除」。
// 故本层是**兜底出口** —— 无论各腿可用与否，`action` / `headline` 都非空。

/// DCF 腿成为主口径所需的最低锚定 FCF 收益率。
///
/// 低于 1% 时，当期现金流的折现值对现价几乎没有解释力
/// （301269 实测锚定 FCF 收益率 0.14%，正向 DCF 给出现价 −97.5% 的「估值」）
/// ⇒ 此时主口径必须换成反向 DCF。
pub const FCF_YIELD_MIN_FOR_DCF_PRIMARY: f64 = 0.01;

/// 结论腿（每条独立可核查）。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConclusionLeg {
    /// `dcf` / `reverse_dcf` / `relative` / `graham`
    pub method: String,
    /// `bullish` / `neutral` / `bearish`
    pub stance: String,
    /// 一句话证据（含关键数值）
    pub evidence: String,
}

/// 估值结论 —— **必产出**，下游 Agent 必须引用 `headline`。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ValuationConclusion {
    /// 明确动作：`低估` / `合理偏低` / `合理` / `偏高` / `高估` / `数据不足`
    pub action: String,
    /// 主结论（一句话，含关键证据与主口径）
    pub headline: String,
    /// 主口径：`dcf` / `reverse_dcf` / `relative` / `graham` / `none`
    pub primary_method: String,
    pub legs: Vec<ConclusionLeg>,
    /// 未纳入的腿及原因（透明度，不是「拒答」）
    pub not_applicable: Vec<String>,
}

/// 结论合成的输入（全部为**已遮蔽/已量化**的值，本层不再取数）。
pub struct ConclusionInputs<'a> {
    /// 正向 DCF 前提是否成立（`DcfAssumptions.applicable`）
    pub dcf_applicable: bool,
    /// 正向 DCF 保守档上行空间（%）
    pub dcf_upside_pct: Option<f64>,
    /// 锚定 FCF / 总市值
    pub fcf_yield: Option<f64>,
    pub reverse: Option<&'a ReverseDcf>,
    pub relative: &'a RelativeValuation,
    /// 格雷厄姆上行空间（%）
    pub graham_upside_pct: Option<f64>,
}

fn relative_stance(verdict: &str) -> &'static str {
    match verdict {
        "deep_value" | "undervalued" => "bullish",
        "expensive" | "overvalued" => "bearish",
        "unknown" => "unavailable",
        _ => "neutral",
    }
}

/// 由相对估值口径给出动作词。
fn action_from_relative(verdict: &str) -> &'static str {
    match verdict {
        "deep_value" => "低估",
        "undervalued" => "合理偏低",
        "expensive" => "偏高",
        "overvalued" => "高估",
        "unknown" => "数据不足",
        _ => "合理",
    }
}

/// 由「上行空间百分比」给出动作词（DCF / 格雷厄姆共用同一刻度）。
fn action_from_upside(up: f64) -> &'static str {
    if up > 30.0 {
        "低估"
    } else if up > 15.0 {
        "合理偏低"
    } else if up > 0.0 {
        "合理"
    } else {
        "偏高"
    }
}

/// 合成估值结论 —— **永不返回空结论**。
pub fn build_conclusion(i: ConclusionInputs<'_>) -> ValuationConclusion {
    let mut legs: Vec<ConclusionLeg> = Vec::new();
    let mut not_applicable: Vec<String> = Vec::new();

    // ① 正向 DCF 腿
    let dcf_ok = i.dcf_applicable
        && i.dcf_upside_pct.is_some()
        && i.fcf_yield.is_some_and(|y| y >= FCF_YIELD_MIN_FOR_DCF_PRIMARY);
    if dcf_ok {
        let up = i.dcf_upside_pct.unwrap_or_default();
        let y = i.fcf_yield.unwrap_or_default() * 100.0;
        legs.push(ConclusionLeg {
            method: "dcf".to_string(),
            stance: if up > 15.0 {
                "bullish"
            } else if up < 0.0 {
                "bearish"
            } else {
                "neutral"
            }
            .to_string(),
            evidence: format!("正向 DCF（保守档）上行空间 {up:.1}%，锚定 FCF 收益率 {y:.2}%"),
        });
    } else {
        let reason = if !i.dcf_applicable {
            "模型前提不成立（数据形态命中不适用判据）"
        } else if i.dcf_upside_pct.is_none() {
            "三档数值缺失或超出量程"
        } else {
            "当期锚定 FCF 收益率 < 1%，正向折现对现价无解释力"
        };
        not_applicable.push(format!("正向 DCF：{reason}"));
    }

    // ② 反向 DCF 腿
    if let Some(r) = i.reverse {
        legs.push(ConclusionLeg {
            method: "reverse_dcf".to_string(),
            stance: match r.feasibility.as_str() {
                "Impossible" | "Strained" => "bearish",
                _ => "neutral",
            }
            .to_string(),
            evidence: format!(
                "反向 DCF：现价隐含 FCF 年复合 {:.0}%、末年 FCF {:.2} 亿，可行性 {}",
                r.implied_cagr * 100.0,
                r.implied_fcf / 1e8,
                r.feasibility
            ),
        });
    } else {
        not_applicable.push("反向 DCF：锚定 FCF 缺失或非正".to_string());
    }

    // ③ 相对估值腿
    if let Some(name) = i.relative.primary.as_deref() {
        let read = i.relative.metrics.iter().find(|r| r.name == name);
        let pct_text = read
            .and_then(|r| r.percentile)
            .map(|p| format!("自身历史分位 {p:.0}%"))
            .unwrap_or_else(|| "自身历史分位不可得".to_string());
        let peer_text = match read.and_then(|r| r.peer_percentile) {
            Some(p) => format!("，同行分位 {p:.0}%"),
            None => String::new(),
        };
        legs.push(ConclusionLeg {
            method: "relative".to_string(),
            stance: relative_stance(&i.relative.verdict).to_string(),
            evidence: format!(
                "相对估值：主指标 {name}（{pct_text}{peer_text}），综合口径 {}",
                i.relative.verdict
            ),
        });
    } else {
        not_applicable.push(format!("相对估值：无有效指标（{}）", i.relative.note));
    }

    // ④ 格雷厄姆腿
    if let Some(up) = i.graham_upside_pct {
        legs.push(ConclusionLeg {
            method: "graham".to_string(),
            stance: if up > 15.0 {
                "bullish"
            } else if up < 0.0 {
                "bearish"
            } else {
                "neutral"
            }
            .to_string(),
            evidence: format!("格雷厄姆内在价值上行空间 {up:.1}%"),
        });
    } else {
        not_applicable.push("格雷厄姆：EPS 不可用或超出量程".to_string());
    }

    // 主口径路由：正向 DCF 仅在「前提成立 + 锚有定价能力」时主导；
    // 否则让位给反向 DCF（它不依赖当期现金流对现价的解释力）。
    let primary_method = if dcf_ok {
        "dcf"
    } else if i.reverse.is_some() {
        "reverse_dcf"
    } else if i.relative.primary.is_some() {
        "relative"
    } else if i.graham_upside_pct.is_some() {
        "graham"
    } else {
        "none"
    };

    let (action, headline) = match primary_method {
        "dcf" => {
            let up = i.dcf_upside_pct.unwrap_or_default();
            let act = action_from_upside(up);
            (
                act.to_string(),
                format!(
                    "主力口径为正向 DCF（保守档）：内在价值较现价 {}{:.1}%，判为「{}」。",
                    if up >= 0.0 { "高" } else { "低" },
                    up.abs(),
                    act
                ),
            )
        },
        "reverse_dcf" => {
            let r = i.reverse.expect("primary_method == reverse_dcf ⇒ reverse 存在");
            match r.feasibility.as_str() {
                "Impossible" | "Strained" => {
                    let qualifier = if r.feasibility == "Impossible" {
                        "超 100% 属物理不可能"
                    } else {
                        "须把过半营收转为自由现金流，极度紧张"
                    };
                    (
                        "高估".to_string(),
                        format!(
                            "正向 DCF 锚不成立（当期现金流对现价无解释力）；反向 DCF 显示现价隐含 FCF \
                             需在 {:.0} 年内年复合增长 {:.0}%、末年 FCF {:.2} 亿（占预测期营收 {:.0}%，{}）\
                             ⇒ 定价与当期现金流脱钩，判为「高估」。",
                            r.forecast_years,
                            r.implied_cagr * 100.0,
                            r.implied_fcf / 1e8,
                            r.implied_fcf_margin_on_implied_revenue.unwrap_or_default() * 100.0,
                            qualifier
                        ),
                    )
                },
                _ => {
                    let act = action_from_relative(&i.relative.verdict);
                    (
                        act.to_string(),
                        format!(
                            "反向 DCF：现价隐含 FCF 年复合 {:.0}%（经营上可达）⇒ 市场假设不极端；\
                             相对估值口径 {}，判为「{}」。",
                            r.implied_cagr * 100.0,
                            i.relative.verdict,
                            act
                        ),
                    )
                },
            }
        },
        "relative" => {
            let act = action_from_relative(&i.relative.verdict);
            (
                act.to_string(),
                format!(
                    "主力口径为相对估值：主指标 {}，综合口径 {}，判为「{}」。{}",
                    i.relative.primary.clone().unwrap_or_default(),
                    i.relative.verdict,
                    act,
                    i.relative.note
                ),
            )
        },
        "graham" => {
            let up = i.graham_upside_pct.unwrap_or_default();
            let act = action_from_upside(up);
            (
                act.to_string(),
                format!("主力口径为格雷厄姆内在价值：较现价 {:.1}%，判为「{}」。", up, act),
            )
        },
        _ => (
            "数据不足".to_string(),
            format!(
                "估值结论：数据不足 —— 正向 DCF 前提不成立、反向 DCF 锚缺失、\
                 相对估值无有效指标。未纳入的腿：{}。需补齐财务/估值数据后重估。",
                if not_applicable.is_empty() {
                    "（无）".to_string()
                } else {
                    not_applicable.join("；")
                }
            ),
        ),
    };

    ValuationConclusion {
        action,
        headline,
        primary_method: primary_method.to_string(),
        legs,
        not_applicable,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 二阶段 DCF：现值关于增长率单调递增（反向 DCF 二分法的前提）。
    #[test]
    fn two_stage_dcf_monotonic_in_growth() {
        let pv = |g: f64| two_stage_dcf(0.5, g, 0.013, 0.077, 5, 0.015).0;
        assert!(pv(0.0) < pv(0.1));
        assert!(pv(0.1) < pv(0.5));
        assert!(pv(-0.2) < pv(0.0));
    }

    /// 终值现值不得超过总现值，且均为正（正常参数下）。
    #[test]
    fn two_stage_dcf_terminal_pv_bounded() {
        let (total, terminal) = two_stage_dcf(0.5, 0.1, 0.013, 0.077, 5, 0.015);
        assert!(total > 0.0);
        assert!(terminal > 0.0);
        assert!(terminal < total);
    }

    /// 反向 DCF 往返：在隐含增速处回代，现值应等于现价（误差 < 1e-6）。
    #[test]
    fn reverse_dcf_round_trips_to_price() {
        let inputs = ReverseDcfInputs {
            fcf_anchor: 0.686e8,
            total_shares: 5.44e8,
            current_price: 87.56,
            discount_rate: 0.077,
            perpetual_growth: 0.013,
            min_terminal_spread: 0.015,
            forecast_years: 5,
            revenue_0: Some(14.0e8),
            revenue_yoy: Some(30.0),
        };
        let r = reverse_dcf(inputs).expect("正锚 + 正现价 ⇒ 必有解");
        let fcf_ps = inputs.fcf_anchor / inputs.total_shares;
        let pv = two_stage_dcf(
            fcf_ps,
            r.implied_cagr,
            inputs.perpetual_growth,
            inputs.discount_rate,
            inputs.forecast_years,
            inputs.min_terminal_spread,
        )
        .0;
        assert!((pv - inputs.current_price).abs() < 1e-6, "回代现值 {pv} 应等于现价");
    }

    /// 301269 形态：当期 FCF 极低 + 现价极高 ⇒ 隐含增速离谱、可行性为「物理不可能」。
    ///
    /// 这是本次重构的直接动因 —— 旧正向 DCF 对同一组数据给 −97.5%，
    /// 反向 DCF 给「要支撑现价需 +130% 年复合」⇒ 定价与现金流脱钩，是有信息量的结论。
    #[test]
    fn reverse_dcf_flags_impossible_for_high_price_low_fcf() {
        let r = reverse_dcf(ReverseDcfInputs {
            fcf_anchor: 0.686e8,
            total_shares: 5.44e8,
            current_price: 87.56,
            discount_rate: 0.077,
            perpetual_growth: 0.013,
            min_terminal_spread: 0.015,
            forecast_years: 5,
            revenue_0: Some(14.0e8),
            revenue_yoy: Some(30.0),
        })
        .expect("有解");
        assert!(r.implied_cagr > 1.0, "隐含增速应远超 100%：{}", r.implied_cagr);
        // 判据落在哪一档取决于营收口径（本例按 +30% 复合推期末营收）：无论压线到
        // 100% 之上还是之下，都必须**不是**「可行」—— 一家公司不可能把 ≈75% 的
        // 营收变成自由现金流（扣除成本/费用后没有这么多可分配）。
        assert_ne!(r.feasibility, "Plausible");
        let margin = r.implied_fcf_margin_on_implied_revenue.expect("有营收 ⇒ 有隐含利润率");
        assert!(margin > 0.5, "隐含 FCF 利润率应远超 50%，实测 {margin}");
    }

    /// 反向 DCF 对「锚非正」返回 None（不做无意义的反解）。
    #[test]
    fn reverse_dcf_none_on_non_positive_anchor() {
        let base = ReverseDcfInputs {
            fcf_anchor: 0.0,
            total_shares: 5.44e8,
            current_price: 87.56,
            discount_rate: 0.077,
            perpetual_growth: 0.013,
            min_terminal_spread: 0.015,
            forecast_years: 5,
            revenue_0: Some(14.0e8),
            revenue_yoy: None,
        };
        assert!(reverse_dcf(base).is_none());
    }

    /// 缺营收数据时退回增速阈值口径，仍给出可行性。
    #[test]
    fn reverse_dcf_falls_back_without_revenue() {
        let r = reverse_dcf(ReverseDcfInputs {
            fcf_anchor: 3.0e8,
            total_shares: 1.0e9,
            current_price: 5.0,
            discount_rate: 0.077,
            perpetual_growth: 0.013,
            min_terminal_spread: 0.015,
            forecast_years: 5,
            revenue_0: None,
            revenue_yoy: None,
        })
        .expect("有解");
        assert!(r.implied_fcf_margin_on_implied_revenue.is_none());
        assert!(!r.feasibility.is_empty());
    }

    /// 301269 形态的结论合成：正向 DCF 锚不成立 ⇒ 主口径切到反向 DCF ⇒
    /// 必须给出明确动作与一句话结论（**不允许「无结论」**）。
    #[test]
    fn conclusion_falls_back_to_reverse_dcf_and_never_empty() {
        let r = reverse_dcf(ReverseDcfInputs {
            fcf_anchor: 0.686e8,
            total_shares: 5.44e8,
            current_price: 87.56,
            discount_rate: 0.077,
            perpetual_growth: 0.013,
            min_terminal_spread: 0.015,
            forecast_years: 5,
            revenue_0: Some(14.0e8),
            revenue_yoy: Some(30.0),
        })
        .expect("有解");
        let rel = relative_valuation(&[
            MetricInput {
                name: "PE",
                value: Some(980.0),
                percentile: Some(60.0),
                peer_values: vec![],
            },
            MetricInput {
                name: "PS",
                value: Some(4.2),
                percentile: Some(1.04),
                peer_values: vec![6.0, 8.0, 10.0],
            },
            MetricInput {
                name: "PB",
                value: Some(3.1),
                percentile: Some(15.5),
                peer_values: vec![],
            },
        ]);
        assert_eq!(rel.primary.as_deref(), Some("PS"), "PE 超上界无效 ⇒ PS 路由为主指标");
        assert_eq!(rel.verdict, "deep_value");

        let c = build_conclusion(ConclusionInputs {
            dcf_applicable: true,
            dcf_upside_pct: Some(-97.5),
            fcf_yield: Some(0.0014),
            reverse: Some(&r),
            relative: &rel,
            graham_upside_pct: None,
        });
        assert_eq!(c.primary_method, "reverse_dcf");
        assert_eq!(c.action, "高估");
        assert!(c.headline.contains("反向 DCF"), "headline 应给出主证据：{}", c.headline);
        assert!(c.not_applicable.iter().any(|s| s.contains("正向 DCF")));
    }

    /// 极端退化：全部腿不可用 ⇒ 仍须给出 `数据不足` 结论而非空结论。
    #[test]
    fn conclusion_present_even_when_all_legs_unavailable() {
        let rel = relative_valuation(&[MetricInput {
            name: "PE",
            value: Some(-3.0),
            percentile: None,
            peer_values: vec![],
        }]);
        let c = build_conclusion(ConclusionInputs {
            dcf_applicable: false,
            dcf_upside_pct: None,
            fcf_yield: None,
            reverse: None,
            relative: &rel,
            graham_upside_pct: None,
        });
        assert_eq!(c.primary_method, "none");
        assert_eq!(c.action, "数据不足");
        assert!(!c.headline.is_empty());
    }

    /// 正常标的：正向 DCF 锚**有定价能力**（FCF 收益率 ≥ 1%）⇒ 主口径仍是 DCF。
    #[test]
    fn conclusion_keeps_dcf_primary_when_anchor_is_pricing() {
        let rel = relative_valuation(&[MetricInput {
            name: "PE",
            value: Some(18.0),
            percentile: Some(30.0),
            peer_values: vec![],
        }]);
        let c = build_conclusion(ConclusionInputs {
            dcf_applicable: true,
            dcf_upside_pct: Some(42.0),
            fcf_yield: Some(0.05),
            reverse: None,
            relative: &rel,
            graham_upside_pct: Some(10.0),
        });
        assert_eq!(c.primary_method, "dcf");
        assert_eq!(c.action, "低估");
        assert_eq!(c.legs.iter().filter(|l| l.method == "dcf").count(), 1);
    }

    /// 同行分位：自身历史分位缺失时回落同行分位。
    #[test]
    fn relative_uses_peer_percentile_when_history_missing() {
        let rel = relative_valuation(&[MetricInput {
            name: "PE",
            value: Some(10.0),
            percentile: None,
            peer_values: vec![20.0, 30.0, 40.0],
        }]);
        let m = &rel.metrics[0];
        assert_eq!(m.peer_median, Some(30.0));
        let p = m.peer_percentile.expect("有同行样本 ⇒ 有同行分位");
        assert!(p < 25.0, "10 相对 [20,30,40] 应落在低分位，实测 {p}");
    }
}
