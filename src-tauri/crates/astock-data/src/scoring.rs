//! 100 分制客观评分引擎（从 stock-analysis crate 下沉到 astock-data，P1-1）
//!
//! 基于技术指标（趋势/乖离率/MACD/量能/RSI/支撑/布林带）计算客观评分。
//! 评分范围 0-100，信号分类从 "强烈买入" 到 "强烈卖出"。
//!
//! 原于 stock-analysis/src/scoring.rs，为供 tools crate（hybrid）直接复用而下沉。

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::indicators::TechnicalIndicators;

/// 评分权重
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScoringWeights {
    pub trend: f64,
    pub deviation: f64,
    pub macd: f64,
    pub volume: f64,
    pub rsi: f64,
    pub support: f64,
    pub boll: f64,
}

impl Default for ScoringWeights {
    fn default() -> Self {
        Self {
            trend: 30.0,
            deviation: 20.0,
            macd: 15.0,
            volume: 15.0,
            rsi: 10.0,
            support: 10.0,
            boll: 5.0,
        }
    }
}

/// 100分制客观评分
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ObjectiveScore {
    pub total: u32,
    pub trend_score: u32,
    pub deviation_score: u32,
    pub macd_score: u32,
    pub volume_score: u32,
    pub rsi_score: u32,
    pub support_score: u32,
    pub boll_score: u32,
    /// 基本面调整（PE / PB / ROE）—— 由 `apply_fundamental_adjustment` 写入。
    pub fundamental_adjustment: i32,
    /// 行业相对估值调整（个股 PE/PB 相对行业中位数的偏离）——
    /// 由 `apply_industry_adjustment` 写入。2026-09-21 新增。
    #[serde(default)]
    pub industry_adjustment: i32,
    /// 合计调整 = `fundamental_adjustment + industry_adjustment`；`total` 实际加减的就是它。
    ///
    /// 2026-09-21 拆字段：此前**只有一个** `fundamentalAdjustment` 字段同时承载两个来源
    /// （两个 `apply_*` 都往它累加）⇒ 字段名给了 LLM 一个**错的归因**（它会把自己与行业的
    /// 相对估值偏离读成「基本面调整」）。拆后 `total` 的**数值完全不变**，只是让 LLM
    /// 可自查恒等式 `totalAdjustment == fundamentalAdjustment + industryAdjustment`。
    #[serde(default)]
    pub total_adjustment: i32,
    pub signal: String,
    pub signal_code: String,
}

/// 参数化评分分段阈值
#[derive(Debug, Clone)]
pub struct ScoreBands {
    pub deviation_band_1: f64,
    pub deviation_score_1: u32,
    pub deviation_band_2: f64,
    pub deviation_score_2: u32,
    pub deviation_band_3: f64,
    pub deviation_score_3: u32,
    pub deviation_band_4: f64,
    pub deviation_score_4: u32,
    pub deviation_band_5: f64,
    pub deviation_score_5: u32,
    pub rsi_oversold_deep: f64,
    pub rsi_oversold: f64,
    pub rsi_neutral_low: f64,
    pub rsi_neutral_high: f64,
    pub rsi_overbought: f64,
    pub rsi_overbought_high: f64,
    pub support_tolerance_pct: f64,
    pub boll_half_std_factor: f64,
}

impl Default for ScoreBands {
    fn default() -> Self {
        Self {
            deviation_band_1: 1.0,
            deviation_score_1: 20,
            deviation_band_2: 2.0,
            deviation_score_2: 18,
            deviation_band_3: 3.0,
            deviation_score_3: 15,
            deviation_band_4: 5.0,
            deviation_score_4: 10,
            deviation_band_5: 8.0,
            deviation_score_5: 5,
            rsi_oversold_deep: 20.0,
            rsi_oversold: 30.0,
            rsi_neutral_low: 40.0,
            rsi_neutral_high: 60.0,
            rsi_overbought: 70.0,
            rsi_overbought_high: 80.0,
            support_tolerance_pct: 0.03,
            boll_half_std_factor: 0.5,
        }
    }
}

/// 100分评分引擎
pub struct ScoringEngine;

impl ScoreBands {
    /// 按**评分尺度**缩放「价格偏离度」与「支撑位容差」两类阈值（四周期科学化 Phase B-1）。
    ///
    /// 为什么必须缩放：`deviation_band_*` 是「收盘价对 MA5 的偏离百分比」的分档边界，
    /// 而偏离的**典型幅度**随 bar 覆盖的时间跨度增长 —— 对随机游走价格，
    /// 位移的标准差 ∝ √时间，故周线的典型偏离约为日线的 √5 倍、月线 √20、季线 √60。
    /// 沿用日线标定的 1%/2%/3%/5%/8% 去评周/月/季线，会让粗尺度的偏离**长期落在最高档**
    /// （或最低档），同一只票的四个尺度分数不可比 —— 这正是「四档只是换个标签」的算法根因之一。
    ///
    /// **刻意不缩放**的字段（每个都有理由，不是漏掉）：
    /// - `rsi_*`：RSI 有界于 0-100 且按「涨跌相对幅度」归一，本身尺度无关；
    /// - `boll_half_std_factor`：布林带用**该尺度自身** bar 的标准差算，已自适配；
    /// - `deviation_score_*`：分档分值是打分刻度（序数），不是价格量纲。
    pub fn scaled_for(profile: &crate::scale::ScaleProfile) -> Self {
        let mut bands = Self::default();
        let f = profile.trading_days_per_bar.sqrt();
        if (f - 1.0).abs() > 1e-12 {
            for band in [
                &mut bands.deviation_band_1,
                &mut bands.deviation_band_2,
                &mut bands.deviation_band_3,
                &mut bands.deviation_band_4,
                &mut bands.deviation_band_5,
                &mut bands.support_tolerance_pct,
            ] {
                *band *= f;
            }
        }
        bands
    }

    /// 基准带 + **设置面板的 RSI 内带覆盖**（`signal_rsi_oversold` / `signal_rsi_overbought`）。
    ///
    /// 落点为什么在这里：这两条变量此前只有「声明 + 面板可读」两面，全仓零消费者
    /// ⇒ 用户改了没有任何东西读它（本仓登记的「配置项空接线」族）。而真正决定 RSI 档分的
    /// 就是下面 `score_rsi` 读的 `rsi_oversold` / `rsi_overbought` 两个字段。
    ///
    /// 判据（与 `check-panel-var-landing.mjs` 的默认值对账同一形）：
    /// - **基准值就是权威**：`Self::default()` 的 30 / 70 与面板两条变量的默认值逐字相等
    ///   ⇒ 「接线」这件事本身不改现网任何一个分数（这是 A 批的入场券）。
    /// - 面板值**只在可用且落在相邻界之间**时覆盖。越界不是「按越界值算」而是**拒绝覆盖 + warn**：
    ///   越过 `rsi_oversold_deep` / `rsi_neutral_low`（或 `neutral_high` / `overbought_high`）
    ///   会让 ladder 里某一档**永久不可达** —— 那种「看起来正常、实际少一档」的产物
    ///   比报错难查得多。这里没有 `Result` 通道（落点在同步纯函数里），所以「失败」表达成
    ///   「不覆盖 + 留痕」，与 v137 那批 `panel_bar_arg` 的显式失败同判据、不同机制。
    pub fn with_panel_overlay(mut self, vars: &HashMap<String, serde_json::Value>) -> Self {
        for (key, bounds, set) in RSI_BAND_VARS {
            let Some(value) = axagent_harness::panel_variables::numeric_in(vars, key) else {
                continue; // 键缺失 / 非数值 ⇒ 已在 harness 留痕，这里按基准值走
            };
            let (lo, hi) = bounds(&self);
            if value <= lo || value >= hi {
                tracing::warn!(
                    "[scoring] 面板 {key} = {value} 不在相邻界 ({lo}, {hi}) 内 ⇒ 拒绝覆盖（覆盖会让 ladder 某一档永久不可达）"
                );
                continue;
            }
            set(&mut self, value);
        }
        self
    }

    /// 生产路径的默认带 = `Default` + 面板覆盖（快照版，见 [`Self::with_panel_overlay`]）。
    pub fn panel_effective() -> Self {
        Self::default().with_panel_overlay(&axagent_harness::panel_variables::panel_variables())
    }

    /// 按尺度缩放 + 面板覆盖。
    ///
    /// 顺序无所谓但**刻意放在缩放之后**：`scaled_for` 明确不动 `rsi_*`（RSI 有界 0-100、
    /// 尺度无关，见其文档），所以覆盖进来的面板值不会被 √d 二次缩放。
    pub fn scaled_for_panel(profile: &crate::scale::ScaleProfile) -> Self {
        Self::scaled_for(profile)
            .with_panel_overlay(&axagent_harness::panel_variables::panel_variables())
    }
}

/// 面板可调的两条 RSI 内带：`(变量名, 相邻界取法, 落点)` **同表**。
///
/// 为什么键与落点必须在一张表里（照 `mcp_tools::INDICATOR_BAR_ARGS` 的形）：
/// 加第三个键却忘了写落点 ⇒ 那个参数被静默忽略 ⇒ 又是一次「面板改了没反应」。
/// `fn` 指针让「表里有」与「落得了地」成为同一件事，生产路径不需要 `unreachable!`。
///
/// 相邻界**不写字面量**而从基准带现取：抄一份 20/40/60/80 进来就是第二权威，
/// 那四个界哪天动了，这里会静默失配（判据也就成了假的）。
type RsiBandVar = (&'static str, fn(&ScoreBands) -> (f64, f64), fn(&mut ScoreBands, f64));

const RSI_BAND_VARS: [RsiBandVar; 2] = [
    (
        "signal_rsi_oversold",
        |b| (b.rsi_oversold_deep, b.rsi_neutral_low),
        |b, v| b.rsi_oversold = v,
    ),
    (
        "signal_rsi_overbought",
        |b| (b.rsi_neutral_high, b.rsi_overbought_high),
        |b, v| b.rsi_overbought = v,
    ),
];

/// 基本面修正的 PE 两档阈值 —— **唯一权威就是这里的 `Default`**（面板 `val_pe_*` 默认值逐字相等）。
///
/// 为什么要抽成类型而不是继续在 `apply_fundamental_adjustment` 里写 `15.0` / `50.0`：
/// 「接面板」要求这两个数有一个可覆盖的载体，而把字面量留在判据里就会同时存在
/// 「写死的数」和「面板的数」两处权威 —— 本仓那族「同一个量两处、值不同」的成因。
///
/// ⚠ `high` 参与的是 `!(0.0..=high).contains(&pe)`。该写法对 **NaN 恒 `true`**
///   （`RangeInclusive::contains` 对 NaN 返回 `false`），所以判据必须继续带 `!pe.is_nan()`，
///   见 `apply_fundamental_adjustment_with_pe` 的注释与 `test_fundamental_adjustment_nan_and_inf`。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PeBands {
    /// PE 低估界（`0 < pe < low` ⇒ +5）。
    pub low: f64,
    /// PE 高估界（`pe > high` 或 `pe < 0` ⇒ −5）。
    pub high: f64,
}

impl Default for PeBands {
    fn default() -> Self {
        Self { low: 15.0, high: 50.0 }
    }
}

/// 面板 PE 两档：`(变量名, 落点)` 同表（理由同 [`RSI_BAND_VARS`]）。
///
/// 别名不是为了好看：`clippy::type_complexity` 对本行的 `(键, setter)` 元组数组直接报
/// deny（全量 `cargo clippy --workspace --all-targets --all-features -- -D warnings` 实测
/// `EXIT=101`，首个 error 就短路了其后所有 crate）。同形的 [`RsiBandVar`] 一开始就带了别名，
/// 这条漏了 —— 补别名而不是 `#[allow]`：后者会让「加了键没写落点」这层保护重新变成可选的。
type PeBandVar = (&'static str, fn(&mut PeBands, f64));

const PE_BAND_VARS: [PeBandVar; 2] =
    [("val_pe_low", |b, v| b.low = v), ("val_pe_high", |b, v| b.high = v)];

impl PeBands {
    /// 默认档 + 面板覆盖。
    ///
    /// **两档成对生效**：任一无效（缺失/非数值）或覆盖后 `low ≥ high`（带反了）
    /// ⇒ 整对回落 `Default`。只覆盖一档会把「低估界」与「高估界」拆成两套来源，
    /// 而反序的带会让同一支票既 +5 又 −5（两个分支的判据区间重叠）。
    pub fn with_panel_overlay(self, vars: &HashMap<String, serde_json::Value>) -> Self {
        let mut patched = self;
        let mut hit = 0usize;
        for (key, set) in PE_BAND_VARS {
            if let Some(value) = axagent_harness::panel_variables::numeric_in(vars, key) {
                if value <= 0.0 {
                    tracing::warn!("[scoring] 面板 {key} = {value} 不是正数 ⇒ 整对回落默认阈值");
                    continue;
                }
                set(&mut patched, value);
                hit += 1;
            }
        }
        if hit == 0 {
            return self;
        }
        if patched.low >= patched.high {
            tracing::warn!(
                "[scoring] 面板 PE 阈值反序（low={} ≥ high={}）⇒ 整对回落默认 {} / {}",
                patched.low,
                patched.high,
                Self::default().low,
                Self::default().high
            );
            return self;
        }
        patched
    }

    /// 生产路径的 PE 阈值 = `Default` + 面板覆盖（快照版）。
    pub fn panel_effective() -> Self {
        Self::default().with_panel_overlay(&axagent_harness::panel_variables::panel_variables())
    }
}

impl ScoringEngine {
    /// 从技术指标计算客观评分
    ///
    /// 分段阈值走 [`ScoreBands::panel_effective`]（= `Default` + 面板 `signal_rsi_*` 覆盖）：
    /// 面板默认值与 `Default` 逐字相等 ⇒ 未调面板时与历史逐分一致。
    pub fn score(
        indicators: &TechnicalIndicators,
        latest_price: f64,
        weights: Option<&ScoringWeights>,
    ) -> ObjectiveScore {
        Self::score_with_bands(indicators, latest_price, weights, &ScoreBands::panel_effective())
    }

    /// 从技术指标计算客观评分（可传入自定义权重和分段参数）
    pub fn score_with_bands(
        indicators: &TechnicalIndicators,
        latest_price: f64,
        weights: Option<&ScoringWeights>,
        bands: &ScoreBands,
    ) -> ObjectiveScore {
        let default_weights = ScoringWeights::default();
        let w = weights.unwrap_or(&default_weights);

        let trend = (Self::score_trend(&indicators.ma_alignment) as f64 * w.trend / 30.0) as u32;
        let deviation =
            (Self::score_deviation(indicators.bias_ma5, bands) as f64 * w.deviation / 20.0) as u32;
        let macd = (Self::score_macd(&indicators.macd_signal, indicators.macd_dif) as f64 * w.macd
            / 15.0) as u32;
        let volume =
            (Self::score_volume(&indicators.volume_signal) as f64 * w.volume / 15.0) as u32;
        let rsi = (Self::score_rsi(indicators.rsi6, bands) as f64 * w.rsi / 10.0) as u32;
        let support = (Self::score_support(latest_price, &indicators.support_levels, bands) as f64
            * w.support
            / 5.0) as u32;
        let boll =
            (Self::score_boll(&indicators.boll_position, bands) as f64 * w.boll / 5.0) as u32;
        let total = (trend + deviation + macd + volume + rsi + support + boll).min(100);

        let (signal, signal_code) = Self::map_signal(total, &indicators.ma_alignment);

        ObjectiveScore {
            total,
            trend_score: trend,
            deviation_score: deviation,
            macd_score: macd,
            volume_score: volume,
            rsi_score: rsi,
            support_score: support,
            boll_score: boll,
            fundamental_adjustment: 0,
            industry_adjustment: 0,
            total_adjustment: 0,
            signal: signal.to_string(),
            signal_code: signal_code.to_string(),
        }
    }

    /// **档位尺度**的评分路径（PLAN §九十八(3) 的乙，2026-10-06 用户拍板）。
    ///
    /// 与 [`Self::score_with_bands`] 的差别只在**取值来源**：§九十八(2) 实测七个分量里有五个
    /// 走命名槽（`ma_alignment` / `bias_ma5` / `rsi6` / `support_levels` / `map_signal` 的形态标签），
    /// 而命名槽是按**周期数值**认领的 ⇒ 档尺度传 `[8,24]` 时它们全部静默退回初值。
    /// 本路径那五项改读 `scaleTrend` / `scaleMomentum` / 两带本身：
    /// - trend：`diff_pct` 对 `±deviation_band_1`（已按 `√d` 缩放）与快带斜率定档，
    ///   分档值沿用既有的 30 / 20 / 12 / 0，**不新增常数**；
    /// - deviation：收盘价对**快带**的偏离（日线口径里那是「对 MA5」，同族量的粗尺度对应物）；
    /// - rsi：`scaleMomentum.value`（缺失 ⇒ 按中性 50 走既有分档，不是伪造读数）；
    /// - support：两条带本身（慢带 + 布林中轨）当支撑，沿用 `score_support` 的容差；
    /// - macd / volume / boll：这三项本来就是按 `cfg` 周期直接算的，不受命名槽影响 ⇒ 照旧。
    ///
    /// ⚠ `scaleTrend` 缺席这一支在**生产上不可达**：`compute_scoring` 先用 `min_bars` 拦样本不足
    /// （不足 ⇒ 显式失败，不出分），所以到这里必有两带。留着 `None` 分支只为「万一有人绕过入口」
    /// 时退回命名槽口径，而不是给一个空分 —— 退回比空分更接近「拿不到时按日线口径算，并在
    /// `windows` 里如实标出」。
    pub fn score_scale_aware(
        indicators: &TechnicalIndicators,
        latest_price: f64,
        weights: Option<&ScoringWeights>,
        profile: &crate::scale::ScaleProfile,
    ) -> ObjectiveScore {
        let Some(trend_facts) = indicators.scale_trend else {
            return Self::score_with_bands(
                indicators,
                latest_price,
                weights,
                &ScoreBands::scaled_for_panel(profile),
            );
        };
        let default_weights = ScoringWeights::default();
        let w = weights.unwrap_or(&default_weights);
        let bands = ScoreBands::scaled_for_panel(profile);
        let band = bands.deviation_band_1;
        let rising = trend_facts.fast_slope.unwrap_or(0.0) >= 0.0;
        let (trend_raw, alignment): (u32, &str) = match trend_facts.diff_pct {
            None => (12, "缠绕/交叉"),
            Some(d) if d >= band && rising => (30, "多头排列"),
            Some(d) if d >= band => (20, "弱多头"),
            Some(d) if d <= -band && !rising => (0, "空头排列"),
            // 空头但在收敛：既有阶梯里没有这一档，落在「缠绕」而不是新造一个分值
            Some(_) => (12, "缠绕/交叉"),
        };
        let deviation_input = if trend_facts.fast > 0.0 {
            (latest_price - trend_facts.fast) / trend_facts.fast * 100.0
        } else {
            0.0
        };
        let trend = (trend_raw as f64 * w.trend / 30.0) as u32;
        let deviation =
            (Self::score_deviation(deviation_input, &bands) as f64 * w.deviation / 20.0) as u32;
        let macd = (Self::score_macd(&indicators.macd_signal, indicators.macd_dif) as f64 * w.macd
            / 15.0) as u32;
        let volume =
            (Self::score_volume(&indicators.volume_signal) as f64 * w.volume / 15.0) as u32;
        let momentum = indicators.scale_momentum.map(|m| m.value).unwrap_or(50.0);
        let rsi = (Self::score_rsi(momentum, &bands) as f64 * w.rsi / 10.0) as u32;
        let levels = [trend_facts.slow, indicators.boll_mid];
        let support =
            (Self::score_support(latest_price, &levels, &bands) as f64 * w.support / 5.0) as u32;
        let boll =
            (Self::score_boll(&indicators.boll_position, &bands) as f64 * w.boll / 5.0) as u32;
        let total = (trend + deviation + macd + volume + rsi + support + boll).min(100);
        let (signal, signal_code) = Self::map_signal(total, alignment);

        ObjectiveScore {
            total,
            trend_score: trend,
            deviation_score: deviation,
            macd_score: macd,
            volume_score: volume,
            rsi_score: rsi,
            support_score: support,
            boll_score: boll,
            fundamental_adjustment: 0,
            industry_adjustment: 0,
            total_adjustment: 0,
            signal: signal.to_string(),
            signal_code: signal_code.to_string(),
        }
    }

    /// 基本面调整：根据 PE / PB / ROE 对客观评分做增量调整
    ///
    /// PE 两档阈值取自 [`PeBands::panel_effective`]（= `Default` 15/50 + 面板 `val_pe_*` 覆盖）。
    /// 需要显式指定阈值（测试 / 回放链）时走 [`Self::apply_fundamental_adjustment_with_pe`]。
    pub fn apply_fundamental_adjustment(
        score: &mut ObjectiveScore,
        pe: f64,
        pb: f64,
        roe: Option<f64>,
    ) {
        Self::apply_fundamental_adjustment_with_pe(score, pe, pb, roe, &PeBands::panel_effective());
    }

    /// [`Self::apply_fundamental_adjustment`] 的纯函数版：PE 阈值由入参给定。
    ///
    /// 为什么留一个纯版：接的是「面板变量」，但判据必须能在**不依赖进程内快照**的情况下
    /// 逐情形测（默认值 ⇒ 与今天逐位相同 / 变量缺失 ⇒ 回落 / 变量被改 ⇒ 新值真进到这条判据）。
    ///
    /// 入参三态（2026-09-21 明确）：`pe` / `pb` 由调用方以 `unwrap_or(0.0)` 传入，
    /// 故 **`0.0` 表示「上游未取到」**（不调整）；**负值表示企业亏损 / 净资产为负**
    /// （显式扣分）。两者语义不同，不可合并成一个「<= 0」判据。
    pub fn apply_fundamental_adjustment_with_pe(
        score: &mut ObjectiveScore,
        pe: f64,
        pb: f64,
        roe: Option<f64>,
        pe_bands: &PeBands,
    ) {
        let mut adj: i32 = 0;
        // ⚠ 两个来源**语义不同、后果同档**，故合并进一个判据：
        //     · `pe > pe_bands.high` —— 估值过高；
        //     · `pe < 0.0`  —— **亏损企业**（EPS < 0 ⇒ PE 无市盈率含义），2026-09-21 新增。
        //   两者都是「PE 不可用于估值」⇒ 同扣 −5。
        //   `pe < 0.0` 这条改动前**不存在**：负 PE 两个分支都不命中 ⇒ 既不加分也不扣分，
        //   等于把「亏损」与「无数据」同等对待。而负 PE 此前根本到不了这里（上游 vendor
        //   用 `filter(|v| *v > 0.0)` 把它抹成 None ⇒ 调用方 `unwrap_or(0.0)`），
        //   是 2026-09-21 放开负 PE 之后才必须显式守卫。
        //   量纲刻意与 `pe > high` 同档，不取更重值，以免与下方 `roe < 5.0` 重复叠加。
        // ⚠ `pe == 0.0` 落在两个条件之外 ⇒ **不调整**。那是调用方 `unwrap_or(0.0)` 造出的
        //   「上游未取到」占位，与「亏损」是两码事 —— **不可合流**（合流会把缺数据当亏损扣分）。
        //
        // 写法说明（2026-09-21，2026-10-08 阈值改为变量后同样成立）：`pe > high || pe < 0.0`
        //   触发 `clippy::manual_range_contains`（CI 用 `-D warnings` ⇒ 必红），而 lint 建议的
        //   `!(0.0..=high).contains(&pe)` **单独用会改 NaN 语义**：`RangeInclusive::contains`
        //   对 NaN 恒 `false` ⇒ NaN 被判成「估值过高」扣分；原式两个严格不等号都不命中 ⇒ 不调整。
        //   故显式补 `!pe.is_nan()` 保持逐情形等价（`pe == 0.0` 仍落在区间内 ⇒ 不调整，
        //   与上文「占位」语义一致；±INF 两侧同为「扣分」）。阈值来自面板后这条**一字未动**。
        if pe > 0.0 && pe < pe_bands.low {
            adj += 5;
        } else if !(0.0..=pe_bands.high).contains(&pe) && !pe.is_nan() {
            adj -= 5;
        }
        // 同上：`pb > 5.0` 估值过高；`pb < 0.0` 净资产为负（资不抵债，2026-09-21 新增）。
        // `pb == 0.0` 仍表「上游未取到」，不调整；`!pb.is_nan()` 的理由见上方写法说明。
        if pb > 0.0 && pb < 1.5 {
            adj += 3;
        } else if !(0.0..=5.0).contains(&pb) && !pb.is_nan() {
            adj -= 3;
        }
        if let Some(r) = roe {
            if r > 15.0 {
                adj += 5;
            } else if r < 5.0 {
                adj -= 3;
            }
        }
        score.fundamental_adjustment += adj;
        score.total_adjustment += adj;
        score.total = (score.total as i32 + adj).clamp(0, 100) as u32;
    }

    /// 行业相对估值调整：个股 PE/PB 相对行业中位数的偏离
    pub fn apply_industry_adjustment(
        score: &mut ObjectiveScore,
        pe: f64,
        industry_pe: Option<f64>,
        pb: f64,
        industry_pb: Option<f64>,
    ) {
        let mut adj: i32 = 0;
        if let Some(ind_pe) = industry_pe {
            if pe > 0.0 && ind_pe > 0.0 {
                if pe < ind_pe * 0.8 {
                    adj += 4;
                } else if pe > ind_pe * 1.2 {
                    adj -= 4;
                }
            }
        }
        if let Some(ind_pb) = industry_pb {
            if pb > 0.0 && ind_pb > 0.0 {
                if pb < ind_pb * 0.8 {
                    adj += 3;
                } else if pb > ind_pb * 1.2 {
                    adj -= 3;
                }
            }
        }
        score.industry_adjustment += adj;
        score.total_adjustment += adj;
        score.total = (score.total as i32 + adj).clamp(0, 100) as u32;
    }

    fn score_trend(alignment: &str) -> u32 {
        match alignment {
            "多头排列" => 30,
            "弱多头" => 20,
            "缠绕/交叉" => 12,
            "空头排列" => 0,
            _ => 12,
        }
    }

    fn score_deviation(bias_ma5: f64, bands: &ScoreBands) -> u32 {
        let abs_bias = bias_ma5.abs();
        if bias_ma5 > 0.0 && abs_bias < bands.deviation_band_1 {
            bands.deviation_score_1
        } else if abs_bias < bands.deviation_band_2 {
            bands.deviation_score_2
        } else if abs_bias < bands.deviation_band_3 {
            bands.deviation_score_3
        } else if abs_bias < bands.deviation_band_4 {
            bands.deviation_score_4
        } else {
            bands.deviation_score_5
        }
    }

    fn score_macd(signal: &str, macd_dif: f64) -> u32 {
        match signal {
            "金叉" if macd_dif > 0.0 => 20,
            "金叉" => 15,
            "多头运行" if macd_dif > 0.0 => 15,
            "多头运行" => 12,
            "死叉" if macd_dif < 0.0 => 3,
            "死叉" => 5,
            "空头运行" if macd_dif < 0.0 => 3,
            "空头运行" => 5,
            _ => 10,
        }
    }

    fn score_volume(signal: &str) -> u32 {
        match signal {
            "放量突破" => 20,
            "放量上涨" => 18,
            "缩量回调" => 12,
            "正常" => 10,
            "缩量上涨" => 8,
            "放量下跌" => 3,
            _ => 10,
        }
    }

    fn score_rsi(rsi: f64, bands: &ScoreBands) -> u32 {
        if rsi < bands.rsi_oversold_deep {
            15
        } else if rsi < bands.rsi_oversold {
            12
        } else if rsi < bands.rsi_neutral_low {
            8
        } else if rsi <= bands.rsi_neutral_high {
            5
        } else if rsi <= bands.rsi_overbought {
            3
        } else if rsi <= bands.rsi_overbought_high {
            2
        } else {
            0
        }
    }

    fn score_support(price: f64, supports: &[f64], _bands: &ScoreBands) -> u32 {
        if supports.is_empty() {
            return 3;
        }
        if price <= 0.0 {
            return 0;
        }
        let nearest = supports.iter().map(|s| (price - s).abs()).fold(f64::MAX, f64::min);
        if nearest < price * 0.02 {
            8
        } else if nearest < price * 0.05 {
            5
        } else {
            3
        }
    }

    fn score_boll(position: &str, _bands: &ScoreBands) -> u32 {
        match position {
            "下轨下方" => 8,
            "下轨附近" => 6,
            "中轨附近" => 4,
            "上轨附近" => 2,
            "上轨上方" => 0,
            _ => 4,
        }
    }

    fn map_signal(total: u32, alignment: &str) -> (&'static str, &'static str) {
        match total {
            85..=100 => match alignment {
                "多头排列" => ("🟢强烈买入", "strong_buy"),
                _ => ("🔵买入", "buy"),
            },
            70..=84 => ("🔵买入", "buy"),
            55..=69 => ("🟡持有", "hold"),
            40..=54 => match alignment {
                "空头排列" => ("🟠卖出", "sell"),
                _ => ("⚪观望", "watch"),
            },
            25..=39 => ("🟠卖出", "sell"),
            _ => ("🔴强烈卖出", "strong_sell"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_indicators(
        alignment: &str,
        bias: f64,
        macd_sig: &str,
        macd_dif: f64,
        vol_sig: &str,
        rsi: f64,
        boll_pos: &str,
    ) -> TechnicalIndicators {
        TechnicalIndicators {
            ma_alignment: alignment.into(),
            bias_ma5: bias,
            macd_signal: macd_sig.into(),
            macd_dif,
            volume_signal: vol_sig.into(),
            rsi6: rsi,
            boll_position: boll_pos.into(),
            support_levels: vec![100.0],
            resistance_levels: vec![200.0],
            ..Default::default()
        }
    }

    /// 中性槽类型（测试夹具要直接构造两带事实）
    use crate::indicators::{ScaleTrend, ScaleValue};

    /// 档位评分路径：命名槽被丢弃时**不再恒 12 分**，而按 `scaleTrend` 的两带定档。
    ///
    /// 夹具是「档尺度算出来的样子」：四个命名槽全是初值（`ma5..ma60 = 0.0` ⇒ `ma_alignment`
    /// 「无数据」），只有中性槽有值 —— 这正是 §九十八(2) 实测的退化输入。
    #[test]
    fn score_scale_aware_uses_neutral_slots_not_named_slots() {
        let profile = crate::scale::ScaleProfile::resolve("quarterly").unwrap();
        let base = |trend: Option<ScaleTrend>, mom: Option<ScaleValue>| TechnicalIndicators {
            scale_trend: trend,
            scale_momentum: mom,
            volume_signal: "正常".into(),
            macd_signal: "缠绕".into(),
            boll_position: "中轨附近".into(),
            boll_mid: 100.0,
            ..Default::default()
        };
        let t = |diff: f64, slope: f64| ScaleTrend {
            fast_bars: 8,
            slow_bars: 24,
            fast: 100.0,
            slow: 100.0,
            diff_pct: Some(diff),
            fast_slope: Some(slope),
        };
        // √60 ≈ 7.75 ⇒ 张开阈值 = 1% × 7.75；取 9% 明显越过、-9% 同理
        let bull =
            ScoringEngine::score_scale_aware(&base(Some(t(9.0, 0.5)), None), 100.0, None, &profile);
        let bear = ScoringEngine::score_scale_aware(
            &base(Some(t(-9.0, -0.5)), None),
            100.0,
            None,
            &profile,
        );
        let flat =
            ScoringEngine::score_scale_aware(&base(Some(t(0.2, 0.0)), None), 100.0, None, &profile);
        assert!(bull.trend_score > flat.trend_score, "多头张开应高于缠绕: {bull:?}");
        assert!(flat.trend_score > bear.trend_score, "空头张开应低于缠绕: {bear:?}");
        assert_eq!(bear.signal_code, ScoringEngine::map_signal(bear.total, "空头排列").1);
        // 命名槽口径在同样输入下会恒判缠绕（退化）—— 这条把「乙确实修掉了退化」钉住
        let legacy = ScoringEngine::score(&base(Some(t(9.0, 0.5)), None), 100.0, None);
        assert_eq!(legacy.trend_score, flat.trend_score, "命名槽全空时 legacy 路径认不出张开");
        assert_ne!(bull.trend_score, legacy.trend_score);
        // 动量缺席 ⇒ 按中性 50 走既有分档，不给伪造的高/低分
        let with_mom = ScoringEngine::score_scale_aware(
            &base(Some(t(0.2, 0.0)), Some(ScaleValue { period: 8, value: 20.0 })),
            100.0,
            None,
            &profile,
        );
        let without =
            ScoringEngine::score_scale_aware(&base(Some(t(0.2, 0.0)), None), 100.0, None, &profile);
        assert!(
            with_mom.rsi_score > without.rsi_score,
            "超卖读数应低于中性: {with_mom:?} {without:?}"
        );
    }

    #[test]
    fn test_bull_market_scores_high() {
        let ind = make_indicators("多头排列", 0.5, "金叉", 0.5, "放量上涨", 55.0, "中轨附近");
        let score = ScoringEngine::score(&ind, 150.0, None);
        assert!(score.total >= 70, "牛市指标应获得高分, 实际={}", score.total);
        assert!(score.trend_score >= 25);
    }

    #[test]
    fn test_bear_market_scores_low() {
        let ind = make_indicators("空头排列", -8.0, "死叉", -0.3, "放量下跌", 15.0, "上轨上方");
        let score = ScoringEngine::score(&ind, 150.0, None);
        assert!(score.total < 40, "熊市指标应获得低分, 实际={}", score.total);
    }

    #[test]
    fn test_score_with_custom_weights() {
        let ind = make_indicators("多头排列", 0.5, "金叉", 0.5, "放量上涨", 55.0, "中轨附近");
        let weights = ScoringWeights { trend: 40.0, ..Default::default() };
        let score = ScoringEngine::score(&ind, 150.0, Some(&weights));
        assert!(score.total > 0 && score.total <= 100);
    }

    /// 2026-09-21 新增（A 修复）：**亏损必须扣分，而「未取到」必须不调整** ——
    /// `pe == 0.0` 是调用方 `unwrap_or(0.0)` 造出的缺失占位，`pe < 0.0` 才是亏损。
    /// 两者曾被同一个「不命中任何档」的行为掩盖成一样。
    #[test]
    fn test_fundamental_adjustment_loss_vs_missing_pe() {
        let ind = make_indicators("多头排列", 0.5, "金叉", 0.5, "放量上涨", 55.0, "中轨附近");
        let adj = |pe: f64| {
            let mut s = ScoringEngine::score(&ind, 150.0, None);
            ScoringEngine::apply_fundamental_adjustment(&mut s, pe, 0.0, None);
            s.fundamental_adjustment
        };
        assert_eq!(adj(0.0), 0, "pe=0.0 表示上游未取到，不得产生调整");
        assert_eq!(adj(-144.08), -5, "亏损企业（负 PE）应扣 5 分");
        // 正控：原有分档不得被新守卫吃掉
        assert_eq!(adj(12.0), 5, "低 PE 仍应 +5");
        assert_eq!(adj(80.0), -5, "过高 PE 仍应 −5");
    }

    /// 2026-09-21 新增：净资产为负（`pb < 0`）扣分；`pb = 0.0` 仍表缺失、不调整。
    #[test]
    fn test_fundamental_adjustment_negative_pb() {
        let ind = make_indicators("多头排列", 0.5, "金叉", 0.5, "放量上涨", 55.0, "中轨附近");
        let adj = |pb: f64| {
            let mut s = ScoringEngine::score(&ind, 150.0, None);
            ScoringEngine::apply_fundamental_adjustment(&mut s, 0.0, pb, None);
            s.fundamental_adjustment
        };
        assert_eq!(adj(0.0), 0, "pb=0.0 表示上游未取到，不得产生调整");
        assert_eq!(adj(-1.5), -3, "净资产为负应扣 3 分");
        // 正控
        assert_eq!(adj(1.0), 3, "低 PB 仍应 +3");
        assert_eq!(adj(9.0), -3, "过高 PB 仍应 −3");
    }

    /// 2026-09-21 新增：NaN / ±INF / 区间端点与「严格不等号原式」逐情形等价。
    ///
    /// 背景：`clippy::manual_range_contains` 要求把 `pe > 50.0 || pe < 0.0` 改写成
    /// `!(0.0..=50.0).contains(&pe)`，但 `RangeInclusive::contains` 对 NaN 恒 `false`
    /// ⇒ **单独用会把 NaN 判成「估值过高」而扣分**（原式两个严格不等号都不命中 ⇒ 不调整）。
    /// 生产代码为此显式补了 `!pe.is_nan()`；本测试就是那条 `!is_nan()` 的回归防线 ——
    /// 谁把它当冗余删掉，这里立刻红。
    #[test]
    fn test_fundamental_adjustment_nan_and_inf() {
        let ind = make_indicators("多头排列", 0.5, "金叉", 0.5, "放量上涨", 55.0, "中轨附近");
        let adj = |pe: f64, pb: f64| {
            let mut s = ScoringEngine::score(&ind, 150.0, None);
            ScoringEngine::apply_fundamental_adjustment(&mut s, pe, pb, None);
            s.fundamental_adjustment
        };
        // NaN = 脏数据：既非「估值过高」亦非「亏损」⇒ 不调整
        assert_eq!(adj(f64::NAN, 0.0), 0, "NaN 不得被当成估值过高扣分");
        assert_eq!(adj(0.0, f64::NAN), 0, "NaN 不得被当成净资产为负扣分");
        // ±INF = 上面两个 0 的构造性对照，证明不是「所有异常值都不调整」
        assert_eq!(adj(f64::INFINITY, 0.0), -5, "+INF 属估值过高 ⇒ −5");
        assert_eq!(adj(f64::NEG_INFINITY, 0.0), -5, "−INF 属亏损 ⇒ −5");
        // 区间端点必须不影响「占位」语义：0.0 仍落在 [0,50] 内 ⇒ 不调整
        assert_eq!(adj(50.0, 0.0), 0, "PE=50 恰在端点内 ⇒ 不调整");
        assert_eq!(adj(15.0, 0.0), 0, "PE=15 属不调整带 ⇒ 不调整");
    }

    /// 2026-09-21 新增（拆字段）：三个调整字段必须**各归其位** ——
    /// 这是唯一能自动发现「基本面与行业又被合并回一个字段」的地方
    /// （合并后 `total` 的数值照样正确，LLM 拿到的归因却是错的，其它测试都不会红）。
    #[test]
    fn test_adjustment_components_are_attributed_separately() {
        let ind = make_indicators("多头排列", 0.5, "金叉", 0.5, "放量上涨", 55.0, "中轨附近");
        let mut s = ScoringEngine::score(&ind, 150.0, None);

        // 只打基本面：PE=12 ⇒ +5（`0 < pe < 15`）；PB=1.0 ⇒ +3（`0 < pb < 1.5`）。
        ScoringEngine::apply_fundamental_adjustment(&mut s, 12.0, 1.0, None);
        assert_eq!(s.fundamental_adjustment, 8, "PE=12 应 +5、PB=1.0 应 +3");
        assert_eq!(s.industry_adjustment, 0, "未调用行业调整 ⇒ 该分量必须保持 0");
        assert_eq!(s.total_adjustment, 8, "合计 = 基本面 + 行业");

        // 再打行业：PE=12 vs 行业中位 30（12 < 30*0.8 = 24）⇒ +4；
        // PB=1.0 vs 行业中位 3.0（1.0 < 3.0*0.8 = 2.4）⇒ +3。
        ScoringEngine::apply_industry_adjustment(&mut s, 12.0, Some(30.0), 1.0, Some(3.0));
        assert_eq!(s.fundamental_adjustment, 8, "行业调整不得污染基本面分量");
        assert_eq!(s.industry_adjustment, 7, "行业分量应为 +4(PE) 与 +3(PB)");
        assert_eq!(
            s.total_adjustment,
            s.fundamental_adjustment + s.industry_adjustment,
            "恒等式：totalAdjustment == fundamentalAdjustment + industryAdjustment"
        );
    }
}

/// 阈值尺度归一（四周期科学化 Phase B-1）的行为锁。
#[cfg(test)]
mod scale_band_tests {
    use super::*;
    use crate::scale::ScaleProfile;

    fn bands_of(period: &str) -> ScoreBands {
        ScoreBands::scaled_for(&ScaleProfile::resolve(period).expect("合法尺度"))
    }

    /// 日线必须与历史逐分一致（f = 1.0 ⇒ 零回归；这是敢改其他尺度的前提）。
    #[test]
    fn daily_is_untouched() {
        assert_eq!(bands_of("daily").deviation_band_1, ScoreBands::default().deviation_band_1);
        assert_eq!(bands_of("daily").support_tolerance_pct, 0.03);
    }

    /// 偏离度阈值按 √(每 bar 交易日数) 缩放，且随尺度单调变宽。
    #[test]
    fn deviation_bands_scale_by_sqrt_of_bar_horizon() {
        let d = bands_of("daily").deviation_band_1;
        let w = bands_of("weekly").deviation_band_1;
        let m = bands_of("monthly").deviation_band_1;
        let q = bands_of("quarterly").deviation_band_1;
        let h = bands_of("hourly").deviation_band_1;
        assert!((w - d * 5.0_f64.sqrt()).abs() < 1e-9, "周线应是日线的 √5 倍，实得 {w}");
        assert!((q - d * 60.0_f64.sqrt()).abs() < 1e-9, "季线应是日线的 √60 倍，实得 {q}");
        assert!((h - d * 0.5).abs() < 1e-9, "小时线应收窄到一半，实得 {h}");
        assert!(h < d && d < w && w < m && m < q, "阈值必须随尺度单调变宽: {h}/{d}/{w}/{m}/{q}");
    }

    /// 可比性核心命题：同一「相对该尺度典型波动的偏离」应得到同一档分。
    #[test]
    fn same_normalized_deviation_scores_the_same_across_scales() {
        let daily = bands_of("daily");
        let weekly = bands_of("weekly");
        let f = 5.0_f64.sqrt();
        for bias in [0.5, 1.5, 2.5, 4.0, 9.0] {
            let at_daily = ScoringEngine::score_deviation(bias, &daily);
            let at_weekly = ScoringEngine::score_deviation(bias * f, &weekly);
            assert_eq!(
                at_daily,
                at_weekly,
                "偏离 {bias}%(日线) 与 {}%(周线) 是同一相对幅度，分数不该不同",
                bias * f
            );
        }
    }

    /// 尺度无关的字段不得被顺手改掉（RSI 有界、BOLL 用本尺度标准差、分值是序数量纲）。
    #[test]
    fn scale_invariant_fields_are_deliberately_untouched() {
        let def = ScoreBands::default();
        for period in ["hourly", "daily", "weekly", "monthly", "quarterly"] {
            let b = bands_of(period);
            assert_eq!(b.rsi_oversold, def.rsi_oversold, "{period} 动了 RSI 阈值");
            assert_eq!(b.rsi_overbought, def.rsi_overbought, "{period} 动了 RSI 阈值");
            assert_eq!(b.boll_half_std_factor, def.boll_half_std_factor, "{period} 动了 BOLL 因子");
            assert_eq!(b.deviation_score_1, def.deviation_score_1, "{period} 动了分值");
        }
    }
}

/// 面板变量落地的行为锁（2026-10-08 A 批：`signal_rsi_*` / `val_pe_*`）。
///
/// 三条各锁一种「接了但没接上」的复发形态：
/// ① 默认值对账（面板的 30/70、15/50 与 Rust 权威逐字相等 ⇒ 接线本身零数值变化）；
/// ② 变量缺失 ⇒ 回落同一组默认（不是 0、不是报错）；
/// ③ 变量被改 ⇒ 新值真进到 `score_rsi` / PE 判据里。
#[cfg(test)]
mod panel_landing_tests {
    use super::*;
    use axagent_harness::panel_variables::variables_map;

    /// 用「变量表 JSON」构造 map，与 wiring 装入快照时读到的形态逐字一致。
    fn vars(pairs: &[(&str, serde_json::Value)]) -> HashMap<String, serde_json::Value> {
        let entries: Vec<serde_json::Value> = pairs
            .iter()
            .map(|(name, value)| serde_json::json!({ "name": name, "value": value }))
            .collect();
        variables_map(&serde_json::Value::Array(entries))
    }

    fn ind_with_rsi(rsi: f64) -> TechnicalIndicators {
        TechnicalIndicators {
            ma_alignment: "多头排列".into(),
            bias_ma5: 0.5,
            macd_signal: "金叉".into(),
            macd_dif: 0.5,
            volume_signal: "放量上涨".into(),
            rsi6: rsi,
            boll_position: "中轨附近".into(),
            ..Default::default()
        }
    }

    /// 面板两条 RSI 变量的默认值必须与 `ScoreBands::default()` 逐字相等。
    ///
    /// 这条断言是 A 批「现网数值逐位不变」的入场券：不等就意味着接线在改判据。
    /// 数字**手抄自 `seed_variables.rs` / 设置面板**（对账的就是两侧手抄的那一份），
    /// 而 ladder 的档分按被测公式现算 ⇒ 见下面两条生效测试。
    #[test]
    fn panel_default_rsi_bands_equal_todays_constants() {
        assert_eq!(ScoreBands::default().rsi_oversold, 30.0, "面板 signal_rsi_oversold=30");
        assert_eq!(ScoreBands::default().rsi_overbought, 70.0, "面板 signal_rsi_overbought=70");
        // 面板默认值（30/70）覆盖上去 ⇒ 带一字不变（证明「接」这件事本身不动数值）。
        let overlaid = ScoreBands::default().with_panel_overlay(&vars(&[
            ("signal_rsi_oversold", serde_json::json!(30.0)),
            ("signal_rsi_overbought", serde_json::json!(70.0)),
        ]));
        assert_eq!(overlaid.rsi_oversold, ScoreBands::default().rsi_oversold);
        assert_eq!(overlaid.rsi_overbought, ScoreBands::default().rsi_overbought);
    }

    /// 变量缺失 ⇒ 回落 `Default`（不是 0、不是报错），且分数与今天逐位相同。
    #[test]
    fn missing_panel_vars_fall_back_to_todays_bands() {
        let empty = vars(&[("unrelated_key", serde_json::json!(1))]);
        let bands = ScoreBands::default().with_panel_overlay(&empty);
        assert_eq!(bands.rsi_oversold, 30.0, "缺失不得把界改成 0 或留空");
        assert_eq!(bands.rsi_overbought, 70.0);
        // 今天 rsi6=25 ⇒ 落在 `rsi < 30` 档 ⇒ 12 分（w.rsi=10 ⇒ rsi_score 即档分）。
        let s = ScoringEngine::score_with_bands(&ind_with_rsi(25.0), 10.0, None, &bands);
        assert_eq!(s.rsi_score, 12, "rsi6=25 在默认带下应得 12 分档");
    }

    /// 变量被改 ⇒ 新值真进到 `score_rsi`（同一支票的档分随面板值移动）。
    #[test]
    fn panel_rsi_value_reaches_the_ladder() {
        // 把超卖界从 30 抬到 35：rsi6=32 从「5 分档（30..40 之间是 8？见下）」变到 12 档。
        let raised = ScoreBands::default()
            .with_panel_overlay(&vars(&[("signal_rsi_oversold", serde_json::json!(35.0))]));
        assert_eq!(raised.rsi_oversold, 35.0);
        // 按 ladder 现算：rsi6=32 → 默认带走 `32 < 40` 档 = 8；抬到 35 后走 `32 < 35` 档 = 12。
        let at_default = ScoringEngine::score_rsi(32.0, &ScoreBands::default());
        let at_panel = ScoringEngine::score_rsi(32.0, &raised);
        assert_eq!(at_default, 8, "默认带：32 落在 (30,40] ⇒ 8");
        assert_eq!(at_panel, 12, "面板带（35）：32 落在 (20,35) ⇒ 12");

        // 超买界从 70 抬到 75：rsi6=72 由「2 分档」变「3 分档」。
        let raised_ob = ScoreBands::default()
            .with_panel_overlay(&vars(&[("signal_rsi_overbought", serde_json::json!(75.0))]));
        assert_eq!(ScoringEngine::score_rsi(72.0, &ScoreBands::default()), 2);
        assert_eq!(ScoringEngine::score_rsi(72.0, &raised_ob), 3, "面板 75 界下 72 仍在超买首档");
    }

    /// 越界值 ⇒ 拒绝覆盖而不是把 ladder 压塌（覆盖会让某一档永久不可达）。
    #[test]
    fn out_of_range_panel_rsi_is_refused_not_applied() {
        // 45 ≥ rsi_neutral_low(40) ⇒ (40,45) 之间的读数没有档可落
        let bands = ScoreBands::default()
            .with_panel_overlay(&vars(&[("signal_rsi_oversold", serde_json::json!(45.0))]));
        assert_eq!(bands.rsi_oversold, 30.0, "越界必须不覆盖");
        // 文本形态（面板 number 不该出现，但 DB 里可能）同样不覆盖。
        let bands = ScoreBands::default()
            .with_panel_overlay(&vars(&[("signal_rsi_overbought", serde_json::json!("70"))]));
        assert_eq!(bands.rsi_overbought, 70.0, "非数值按缺失处理");
    }

    /// PE 两档：面板默认 == 现常量 ⇒ 接线零数值变化。
    #[test]
    fn panel_default_pe_bands_equal_todays_constants() {
        assert_eq!(PeBands::default().low, 15.0, "面板 val_pe_low=15");
        assert_eq!(PeBands::default().high, 50.0, "面板 val_pe_high=50");
        let overlaid = PeBands::default().with_panel_overlay(&vars(&[
            ("val_pe_low", serde_json::json!(15.0)),
            ("val_pe_high", serde_json::json!(50.0)),
        ]));
        assert_eq!(overlaid, PeBands::default());
        // 默认档下 PE=12 ⇒ +5、PE=55 ⇒ −5、PE=0（未取到占位）⇒ 0
        let adj = |pe: f64, bands: &PeBands| {
            let mut s = ScoringEngine::score(&ind_with_rsi(55.0), 10.0, None);
            ScoringEngine::apply_fundamental_adjustment_with_pe(&mut s, pe, 0.0, None, bands);
            s.fundamental_adjustment
        };
        assert_eq!(adj(12.0, &PeBands::default()), 5);
        assert_eq!(adj(55.0, &PeBands::default()), -5);
        assert_eq!(adj(0.0, &PeBands::default()), 0);
    }

    /// PE 变量被改 ⇒ 新阈值真进到判据里；缺失 ⇒ 回落默认。
    #[test]
    fn panel_pe_value_reaches_the_adjustment_and_missing_falls_back() {
        let adj = |pe: f64, bands: &PeBands| {
            let mut s = ScoringEngine::score(&ind_with_rsi(55.0), 10.0, None);
            ScoringEngine::apply_fundamental_adjustment_with_pe(&mut s, pe, 0.0, None, bands);
            s.fundamental_adjustment
        };
        // 低估界 15 → 10：PE=12 由「+5」变成「不调整」。
        let tighter = PeBands::default()
            .with_panel_overlay(&vars(&[("val_pe_low", serde_json::json!(10.0))]));
        assert_eq!(tighter.low, 10.0);
        assert_eq!(adj(12.0, &PeBands::default()), 5, "默认 15 界下 12 属低估");
        assert_eq!(adj(12.0, &tighter), 0, "面板 10 界下 12 不再算低估");
        // 高估界 50 → 60：PE=55 由「−5」变成「不调整」。
        let wider = PeBands::default()
            .with_panel_overlay(&vars(&[("val_pe_high", serde_json::json!(60.0))]));
        assert_eq!(adj(55.0, &wider), 0, "面板 60 界下 55 不扣");
        // 缺失 ⇒ 回落默认（不是 0、不是报错）。
        let absent =
            PeBands::default().with_panel_overlay(&vars(&[("news_limit", serde_json::json!(30))]));
        assert_eq!(absent, PeBands::default());
        assert_eq!(adj(55.0, &absent), -5);
    }

    /// 反序 / 非法的 PE 面板值 ⇒ 整对回落，不留下「低估界高于高估界」的带。
    #[test]
    fn inverted_pe_bands_fall_back_wholesale() {
        let inverted = PeBands::default().with_panel_overlay(&vars(&[
            ("val_pe_low", serde_json::json!(80.0)),
            ("val_pe_high", serde_json::json!(50.0)),
        ]));
        assert_eq!(inverted, PeBands::default(), "low ≥ high ⇒ 整对回落");
        let negative = PeBands::default()
            .with_panel_overlay(&vars(&[("val_pe_low", serde_json::json!(-3.0))]));
        assert_eq!(negative, PeBands::default(), "非正数按无效处理");
    }

    /// 阈值来自变量表之后，NaN 语义必须仍与 2026-09-21 那条锁逐情形等价。
    #[test]
    fn pe_overlay_keeps_nan_and_inf_semantics() {
        let adj = |pe: f64, bands: &PeBands| {
            let mut s = ScoringEngine::score(&ind_with_rsi(55.0), 10.0, None);
            ScoringEngine::apply_fundamental_adjustment_with_pe(&mut s, pe, 0.0, None, bands);
            s.fundamental_adjustment
        };
        for bands in [PeBands::default(), PeBands { low: 10.0, high: 60.0 }] {
            assert_eq!(adj(f64::NAN, &bands), 0, "NaN 不得被当成估值过高扣分");
            assert_eq!(adj(f64::INFINITY, &bands), -5);
            assert_eq!(adj(f64::NEG_INFINITY, &bands), -5, "负 PE 属亏损");
            assert_eq!(adj(0.0, &bands), 0, "0.0 是「上游未取到」占位");
            assert_eq!(adj(bands.high, &bands), 0, "恰在高估界端点内 ⇒ 不调整");
        }
    }
}
