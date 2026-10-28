//! 窗口涨幅达标漏检核查（`PLAN-mover-recall-attribution.md` Phase 2/3）
//!
//! ## 这里只放「可判定的部分」
//!
//! 事件 = 某票在某档窗口内**累计涨幅**达标；漏检层 = 用已落库的事实机械判出的那一层。
//! 两件事都不碰模型、不碰 K 线：累计涨幅由 `market_daily_close` 复利连乘，
//! 推荐与否由 `reco_picks` 与其 `seed_pool_json` 判。
//!
//! ## 命名边界（用户裁定）
//!
//! 判据是**绝对涨幅**，不含板块涨停语义 ⇒ 任何对外文案一律写「窗口涨幅达标」，
//! 不得写「涨停」。板块归属（`market_type`）只用于分组出数：主板涨 10% ≈ 封板常买不进，
//! 创业板涨 10% ≈ 寻常波动，混在一格里报数会把优化方向带偏。
//!
//! ## 缺席的三种口径（不得互相冒充）
//!
//! - [`MissLayer::ByDesign`]：该 (风格, 档位) 按设计不出票（[`style_matrix`] 判），不是缺陷；
//! - [`MissLayer::PoolDegraded`]：本轮候选池因取数降级而缺它，属数据可达性，不是算法；
//! - [`MissLayer::Unexplained`]：兜底桶，**必须显示占比**；占比高说明留痕不足，
//!   此时不得据此下优化结论。

use std::collections::HashMap;

use sea_orm::{ColumnTrait, EntityTrait, PaginatorTrait, QueryFilter, QueryOrder};
use serde::{Deserialize, Serialize};

/// 池留痕行的类型由本模块**产出**（`load_picks_in_range` 的返回元素），故在此 re-export 给
/// 命令层：权威定义仍在 `axagent_entities::reco_picks`（禁区 12 只允许一份定义），而命令层
/// 不得直连 entities（分层棘轮 `commands-no-direct-db`，新文件基线 0）。
pub use axagent_entities::reco_picks;
use axagent_entities::{market_daily_close, market_stock_universe};

use crate::recommender::style_matrix;
use crate::recommender::types::Period;

/// 取数面：股票代码 → （名称，按交易日升序的 `(日期, 收盘价)` 序列）。
pub type CloseSeriesByCode = HashMap<String, (String, Vec<(String, f64)>)>;

/// 出厂阈值（用户裁定：超短 10%、短 20%、中 30%、长 40%）。
///
/// 这是**模板变量的缺省值**，不是判据本身 —— 判据读 `mover_gain_*` 变量
/// （`seed_variables.rs`），与 `default_holding_days` 同族登记。缺省值就地兜底是刻意的：
/// 反思/用户还没调过参数时，核查必须仍可运行。
pub const DEFAULT_GAIN_THRESHOLDS: [(&str, f64); 4] = [
    ("mover_gain_ultra_short", 10.0),
    ("mover_gain_short", 20.0),
    ("mover_gain_mid", 30.0),
    ("mover_gain_long", 40.0),
];

/// 按变量名回查登记表（读回**已落库核查轮次**的 `rules_json` 用）。
///
/// 为什么需要它而不是直接 `Deserialize` [`TierRule`]：`var_name` 是 `&'static str`，
/// 不满足 `DeserializeOwned`。读回时把名字当字符串取出、再用本函数指回这张表 ——
/// 名字不在表内返回 `None`，调用方必须显式失败，**不得**静默丢掉一档或另编一个变量名
/// （那等于把「登记表里没有这档」演成「这档没有判据」）。
pub fn gain_var_by_name(name: &str) -> Option<&'static str> {
    DEFAULT_GAIN_THRESHOLDS.iter().find(|(registered, _)| *registered == name).map(|(n, _)| *n)
}

/// 漏检归因的**链**维度：一条达标事件要对两条链各问一次「你当时为什么没选它」。
///
/// 判据只有一条，且刻意**不用** [`normalize_style_key`]：`style == "serenity"` 的行由
/// 趋势智选工作流写（`stock_workflow/serenity.rs` 把策略子类型放进 `pick_data.strategy_type`，
/// `style` 恒为 `serenity`）；其余 style —— **包括荐股矩阵里的 `bottleneck` 智选格** ——
/// 都是智能荐股链的产出。别名归一（bottleneck→serenity）是给矩阵取格用的，
/// 拿它判链会把荐股的智选格算进工作流链，两条链的账就此互相顶替。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecoChain {
    /// 智能荐股（四档共享池 + 策略矩阵）
    Reco,
    /// 趋势智选（serenity-screening 工作流）
    Serenity,
}

impl RecoChain {
    /// 固定顺序 = 面板与落库的枚举顺序（荐股在前）。
    pub const ALL: [RecoChain; 2] = [RecoChain::Reco, RecoChain::Serenity];

    pub fn as_str(self) -> &'static str {
        match self {
            RecoChain::Reco => "reco",
            RecoChain::Serenity => "serenity",
        }
    }

    /// 一条 pick 的 style 属于哪条链。
    pub fn of_style(style: &str) -> Self {
        if style == Self::Serenity.as_str() {
            Self::Serenity
        } else {
            Self::Reco
        }
    }
}

/// 分链改造（2026-10-20 A-2）之前落库的轮次/事件没有链标记 ⇒ 第三态，读侧单独成句。
/// 把它当成 `reco` 就是拿混池数字冒充荐股，当成 `serenity` 同理。
pub const CHAIN_UNLABELED: &str = "unlabeled";

/// 已落库 `rules_json` 的投影。
///
/// 只用于**呈现**当轮事实（面板按轮次读回时显示那一代用到的阈值与窗口），
/// 不参与任何达标判定 ⇒ 不构成第二套判据。`var_name` 在这里是 `String`，
/// 转回 [`TierRule`] 必须经 [`gain_var_by_name`] 指认登记表。
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StoredTierRule {
    pub period: Period,
    pub var_name: String,
    pub gain_pct: f64,
    pub window_days: u32,
}

/// 把已落库的判据投影转回 [`TierRule`]；任一 `varName` 不在登记表 ⇒ `Err`。
pub fn rules_from_stored(stored: &[StoredTierRule]) -> Result<Vec<TierRule>, String> {
    stored
        .iter()
        .map(|r| {
            let var_name = gain_var_by_name(&r.var_name)
                .ok_or_else(|| format!("已落库判据的变量名不在登记表内: {}", r.var_name))?;
            Ok(TierRule {
                period: r.period,
                var_name,
                gain_pct: r.gain_pct,
                window_days: r.window_days,
            })
        })
        .collect()
}

/// 一档的判据参数。
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TierRule {
    pub period: Period,
    /// 模板变量名 `mover_gain_{period}`
    pub var_name: &'static str,
    /// 达标阈值（%）
    pub gain_pct: f64,
    /// 窗口长度（交易日），来自 `default_holding_days` 的既有单源
    pub window_days: u32,
}

/// 从模板变量组装四档判据。
///
/// 窗口天数直接取 `Period::default_holding_days()`（全仓唯一天数来源），本函数不另存
/// 一份天数表 —— 否则就出现第二套档位尺度，口径门 f 段判红。
pub fn tier_rules_from_vars(vars: &[(String, serde_json::Value)]) -> Vec<TierRule> {
    DEFAULT_GAIN_THRESHOLDS
        .iter()
        .filter_map(|(var_name, default)| {
            let period = var_name.strip_prefix("mover_gain_")?.parse::<Period>().ok()?;
            let gain_pct = vars
                .iter()
                .find(|(k, _)| *k == *var_name)
                .and_then(|(_, v)| v.as_f64().or_else(|| v.as_str().and_then(|s| s.parse().ok())))
                .unwrap_or(*default);
            let window_days = period.default_holding_days();
            // 阈值非正/非有限 ⇒ 该档判据不成立，显式跳过而不是拿默认值蒙混
            if !(gain_pct.is_finite() && gain_pct > 0.0) {
                return None;
            }
            Some(TierRule { period, var_name, gain_pct, window_days })
        })
        .collect()
}

/// 一行的收盘输入（从 `market_daily_close` 投影）。
#[derive(Debug, Clone, PartialEq)]
pub struct CloseRow {
    pub stock_code: String,
    pub stock_name: String,
    pub trade_date: String,
    pub change_pct: f64,
}

/// 窗口累计涨幅 = `∏(1 + r_i/100) − 1`，返回百分数。
///
/// 空窗口 ⇒ `None`（无数据 ≠ 涨幅 0）。含非有限值 ⇒ `None`（脏数据不进事件表）。
pub fn window_cum_gain_pct(rows: &[f64]) -> Option<f64> {
    if rows.is_empty() || !rows.iter().all(|r| r.is_finite()) {
        return None;
    }
    let product = rows.iter().fold(1.0f64, |acc, r| acc * (1.0 + r / 100.0));
    if !product.is_finite() || product <= 0.0 {
        return None;
    }
    Some((product - 1.0) * 100.0)
}

/// #10 P7 妖股标签的**唯一判据**（逐档反思行用；反思侧见
/// `stock_workflow/reflection.rs` 的 `compute_mover_label`）。
///
/// 与达标核查（[`window_cum_gain_pct`]）刻意不是同一个量：那条量的是「窗口内逐日收盘累计」，
/// 而标签要回答「从**分析时价格**到**反思时价格**涨了多少」⇒ 入参 `gross_gain_pct` 必须由调用方
/// 传行情快照的 `price_change_pct`（原始涨跌幅，**不扣成本** —— 阈值是给涨幅定的，扣费会把
/// 39.8% 判成非妖股，属口径错位）。
///
/// 返回 `None` = **不判定**（该档阈值判据不可用）。调用方不得把它写成 `"normal"`：
/// 「没配判据」与「算过且没达标」是两件事，混成一个值就是伪装成有结论。
/// 其余四态的语义与 NULL 的边界写在 `entities/src/stock_reflections.rs` 的列文档里。
pub fn mover_label_for(
    threshold_pct: Option<f64>,
    gross_gain_pct: Option<f64>,
    window_complete: bool,
) -> Option<&'static str> {
    // 判据不可用优先于一切：阈值缺失/非正/非有限 ⇒ `None`（调用方记 rule_unavailable）。
    // `tier_rules_from_vars` 本来就会丢掉非正的档，这里再兜一次是为了让本函数**独立可证**。
    let threshold = threshold_pct.filter(|t| t.is_finite() && *t > 0.0)?;
    // 拿不到涨幅 ⇒ 无从判定（不得冒充「算过且未达标」）；非有限值同样归到这里
    let Some(gain) = gross_gain_pct.filter(|g| g.is_finite()) else {
        return Some("no_market_data");
    };
    // 持有期未满 ⇒ 不判定：面板留空并注明未满，不拿当前价冒充到期价
    if !window_complete {
        return Some("window_incomplete");
    }
    Some(if gain >= threshold { "mover" } else { "normal" })
}

/// 一条达标事件。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MoverEvent {
    pub stock_code: String,
    pub stock_name: String,
    /// 档位（`ultra_short` | `short` | `mid` | `long`）
    pub period: String,
    /// 窗口右端（= 达标确认日）ISO 日期
    pub anchor_date: String,
    /// 窗口起点 ISO 日期（anchor 当日及其前 window_days−1 个交易日的首日）。
    /// P2 漏检补分析以此为 as-of 锚点：复盘问的是「窗口起步时系统该不该看出它」。
    pub window_start: String,
    pub window_days: u32,
    /// 窗口累计涨幅（%），主判据
    pub cum_gain_pct: f64,
    /// 窗口内最大单日涨幅（%），强度标签
    pub max_daily_pct: f64,
    pub threshold_pct: f64,
    /// 板块归属（`detect_market_type` 的取值，仅用于分组）
    pub market_type: String,
    /// 该档在窗口内是否已有任意一条推荐（true ⇒ 不算漏检）
    pub recommended: bool,
}

/// 漏检归因层（每层都必须机械可判，判不出的进 `Unexplained`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MissLayer {
    /// 该票该档已被推荐 ⇒ 命中，不入漏检分母
    Recommended,
    /// 窗口起点前一个窗口长度内已同档推荐过 ⇒「已覆盖」，不计漏检。
    ///
    /// 旧值 `late_timing` 保留为反序列化别名：历史轮次的 `layers_json` 里可能存过它，
    /// 改名后仍要能读回（新写只写 `prior_covered`）。
    #[serde(alias = "late_timing")]
    PriorCovered,
    /// 完全不在任何一次候选池快照里
    NotInPool,
    /// 候选池本轮降级（取数失败/为空）⇒ 数据可达性问题，不记算法账
    PoolDegraded,
    /// 该 (风格, 档位) 按设计不出票
    ByDesign,
    /// 入池、且**有截断留痕**证明该格算出了它却在组内 top-N 被淘汰
    /// （`reco_scan_audit` 有行）⇒ 唯一可归因到具体风格、可据此降权的一层
    L3ScoredOut,
    /// 该股在别的档位出了票，事件落在本档窗口
    OtherPeriod,
    /// 兜底桶：**含「入池但无截断留痕」**（策略内部否决不落痕），必须显示占比
    Unexplained,
}

impl MissLayer {
    /// 全部归因层 —— 逐层可达性测试（`every_miss_layer_is_reachable_from_attribute`）的遍历面。
    ///
    /// 条数必须与枚举变体数一致：`scripts/check-reco-horizon-parity.mjs` 的门 p 拿枚举现场
    /// 对这张表，加了变体忘了进这里 ⇒ 门红（否则那条 Rust 测试会安静地少判一层）。
    pub const ALL: [MissLayer; 8] = [
        MissLayer::Recommended,
        MissLayer::PriorCovered,
        MissLayer::NotInPool,
        MissLayer::PoolDegraded,
        MissLayer::ByDesign,
        MissLayer::L3ScoredOut,
        MissLayer::OtherPeriod,
        MissLayer::Unexplained,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            MissLayer::Recommended => "recommended",
            MissLayer::PriorCovered => "prior_covered",
            MissLayer::NotInPool => "not_in_pool",
            MissLayer::PoolDegraded => "pool_degraded",
            MissLayer::ByDesign => "by_design",
            MissLayer::L3ScoredOut => "l3_scored_out",
            MissLayer::OtherPeriod => "other_period",
            MissLayer::Unexplained => "unexplained",
        }
    }

    /// 计入漏检分母的层（命中与「非缺陷」的层都不算漏检）。
    pub fn counts_as_miss(self) -> bool {
        matches!(
            self,
            MissLayer::NotInPool
                | MissLayer::PoolDegraded
                | MissLayer::L3ScoredOut
                | MissLayer::OtherPeriod
                | MissLayer::Unexplained
        )
    }
}

/// 归因入参：一次核查所需的全部已知事实，由调用方从库里取好。
#[derive(Debug, Default, Clone)]
pub struct Evidence {
    /// 该股在事件窗口起点之后、该档下的推荐（`generated_at` ISO）
    pub picks_for_period: Vec<String>,
    /// 该股在其他档位有过推荐
    pub picked_in_other_period: bool,
    /// 该票是否出现在本轮任一候选池快照。
    ///
    /// `None` 的语义是**本窗内一条荐股批次都没有** ⇒ 无从判定它在不在池里，这不是「不在池里」。
    /// 旧实现用 `bool`，把「没扫描过」和「扫描了但宇宙里没有它」压成同一个 `false`：
    /// 现网实测 46 个漏检事件里 **25 个被打成 `not_in_pool`**，而那 4400 条回放行的
    /// `seed_pool_json` 全是 `"[]"`（2026-10-19 取数）——归因层因此把一个留痕问题报成了
    /// 候选宇宙问题，优化方向会被带去「扩池」，而真缺的是批次本身。
    /// `None` 走 [`MissLayer::Unexplained`]（既有兜底桶，语义正是「留痕不足无法归因」），不新造一层。
    pub in_pool: Option<bool>,
    /// 本轮候选池是否降级（空/取数失败）
    pub pool_degraded: bool,
    /// **本链**在该档是否按设计出票（`chain_period_has_active_style` 口径，A-3）。
    ///
    /// 名字刻意不叫 `matrix_active`：荐股链问的是「矩阵里除智选行外任一风格出票」，
    /// 智选链只看 `serenity` 那一行 ⇒ 同一个布尔值在两条链上是两个问题，
    /// 沿用旧名会让读代码的人以为两链同口径。
    pub chain_tier_active: bool,
    /// 窗口起点**之前一个窗口长度内**该链已在同档出过票（`PriorCovered` 判据）。
    ///
    /// 存在的理由：`in_window` 只认 `generated_at` 落在窗口内，早于起点的那次推荐会被
    /// 当成「从没推荐过」⇒ 事件落进 `not_in_pool`/`unexplained`，优化方向被带去「扩池」，
    /// 而真相是链看见过它，只是时点不落在本窗。
    pub picked_before_window: bool,
    /// 窗口内**有截断留痕**的风格键（`reco_scan_audit` 命中，矩阵名目归一后的写法）——
    /// 非空即「该格算出过它却被 top-N 淘汰」，可归因、可降权
    pub l3_scored_out_styles: Vec<String>,
}

/// 纯函数归因：顺序即优先级，先排掉「其实不是漏检」，再定位到可判的层。
///
/// `PriorCovered` 紧跟 `Recommended`：同档在窗口之前一个窗口长度内出过票时，
/// 「池子里没有它」「按设计不做这档」都已不成立 —— 链看见过它，只是时点不落在本窗。
/// `L3ScoredOut` 排在 `OtherPeriod` 之后：别的档位已经出过票时，「本档漏」的
/// 结论会被系统性高估（同一只票本就不必四档齐发）。
pub fn attribute(ev: &Evidence) -> MissLayer {
    if !ev.picks_for_period.is_empty() {
        return MissLayer::Recommended;
    }
    if ev.picked_before_window {
        return MissLayer::PriorCovered;
    }
    if !ev.chain_tier_active {
        return MissLayer::ByDesign;
    }
    if ev.pool_degraded {
        return MissLayer::PoolDegraded;
    }
    match ev.in_pool {
        // 本窗内没有任何荐股批次 ⇒ 无从判定，落「留痕不足」而不是自证「池子漏了它」
        None => return MissLayer::Unexplained,
        Some(false) => return MissLayer::NotInPool,
        Some(true) => {},
    }
    if ev.picked_in_other_period {
        return MissLayer::OtherPeriod;
    }
    if !ev.l3_scored_out_styles.is_empty() {
        return MissLayer::L3ScoredOut;
    }
    // 入池、无留痕 ⇒ 不可判（策略内部否决不落痕），显式落兜底桶而不是自造一层
    MissLayer::Unexplained
}

/// 三个率（缺任何一个都会把优化方向带偏，见 PLAN Phase 3）。
///
/// `NaN` 的跨边界处理：`f64::NAN` 在 JSON 里**不可表示**（serde_json 对非有限值直接
/// 返回错误）⇒ 若原样序列化，「样本不足以至于算不出率」会让整条命令失败。
/// 这里把非有限值映射成 `null`，前端按「样本不足」成句显示，而不是让面板整块消失。
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecallRates {
    /// 事件中曾进入候选池的比例（受 universe 与池层决定，不是算法能力）
    #[serde(serialize_with = "nan_to_null")]
    pub reachability: f64,
    /// 池内事件中被评为推荐的比例
    #[serde(serialize_with = "nan_to_null")]
    pub coverage: f64,
    /// `unexplained` 占全部漏检的比例 —— 决定「优化」的期望收益上限
    #[serde(serialize_with = "nan_to_null")]
    pub unexplained_share: f64,
    pub events: usize,
    pub misses: usize,
    /// 可判事件数（排除 `ByDesign` / `PriorCovered`）—— reachability 的分母。
    /// `None` = 旧轮次未记录（面板显示「未记录」，不拿 events 冒充）。
    pub judgeable_events: Option<usize>,
}

/// 已落库 `rates_json` 的投影（读回**那一轮**的三个率用）。
///
/// `RecallRates` 本身不能 `Deserialize`：它的 NaN 在序列化时已被 [`nan_to_null`] 折成
/// `null`，而 `null` 解不回 `f64`。这里用 `Option<f64>` 接住，`into_rates` 再把
/// 「算不出来」还原成 NaN ⇒ 面板仍走同一条呈现判据（显示「样本不足」，**不是** 0%）。
#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StoredRates {
    pub reachability: Option<f64>,
    pub coverage: Option<f64>,
    pub unexplained_share: Option<f64>,
    pub events: usize,
    pub misses: usize,
    pub judgeable_events: Option<usize>,
}

impl StoredRates {
    pub fn into_rates(self) -> RecallRates {
        RecallRates {
            // `null` 与「字段缺失」都归到 NaN（= 算不出来），不得折成 0.0
            reachability: self.reachability.unwrap_or(f64::NAN),
            coverage: self.coverage.unwrap_or(f64::NAN),
            unexplained_share: self.unexplained_share.unwrap_or(f64::NAN),
            events: self.events,
            misses: self.misses,
            judgeable_events: self.judgeable_events,
        }
    }
}

fn nan_to_null<S: serde::Serializer>(v: &f64, s: S) -> Result<S::Ok, S::Error> {
    if v.is_finite() {
        s.serialize_f64(*v)
    } else {
        s.serialize_none()
    }
}

/// 由 (层, 计数) 表算三个率。分母为 0 ⇒ 该率记 `NaN`，由呈现层显式报「样本不足」，
/// 不得显示成 0%（0% 与「算不出来」是两件事）。
pub fn compute_rates(counts: &HashMap<MissLayer, usize>, events: usize) -> RecallRates {
    let get = |l: MissLayer| *counts.get(&l).unwrap_or(&0);
    let recommended = get(MissLayer::Recommended);
    // 「可判」= 排除按设计不做与窗口前已覆盖；它们不是漏检，也不该稀释 reachability。
    let judgeable_events =
        events.saturating_sub(get(MissLayer::ByDesign) + get(MissLayer::PriorCovered));
    // reachability 的分子是「入池或已推荐」；PoolDegraded 是「池不可用」，不是「可达」，
    // 旧实现把它算进分子，会把降级轮次的到达率抬高。
    let reachable = get(MissLayer::L3ScoredOut)
        + get(MissLayer::OtherPeriod)
        + get(MissLayer::Unexplained)
        + recommended;
    let misses = get(MissLayer::NotInPool)
        + get(MissLayer::PoolDegraded)
        + get(MissLayer::L3ScoredOut)
        + get(MissLayer::OtherPeriod)
        + get(MissLayer::Unexplained);
    let reachability = if judgeable_events == 0 {
        f64::NAN
    } else {
        reachable as f64 * 100.0 / judgeable_events as f64
    };
    // 覆盖率分母 = 入池且可判的事件（不含降级桶）：降级是数据可达性问题，
    // 既不能当命中，也不能当算法淘汰 —— 单列在 reachability 之外。
    let coverage_den = recommended
        + get(MissLayer::L3ScoredOut)
        + get(MissLayer::OtherPeriod)
        + get(MissLayer::Unexplained);
    let coverage = if coverage_den == 0 {
        f64::NAN
    } else {
        recommended as f64 * 100.0 / coverage_den as f64
    };
    let unexplained_share = if misses == 0 {
        f64::NAN
    } else {
        get(MissLayer::Unexplained) as f64 * 100.0 / misses as f64
    };
    RecallRates {
        reachability,
        coverage,
        unexplained_share,
        events,
        misses,
        judgeable_events: Some(judgeable_events),
    }
}

/// 一次核查的完整产出。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MoverReport {
    pub as_of: String,
    pub rules: Vec<TierRuleView>,
    /// 事件总数（含命中）
    pub events: Vec<MoverEvent>,
    pub layers: Vec<LayerRow>,
    pub rates: RecallRates,
    /// 数据边界声明：清单规模与快照日期，供面板显式说明枚举域完整性
    pub universe_size: i64,
    pub close_dates_available: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TierRuleView {
    pub period: String,
    pub gain_pct: f64,
    pub window_days: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LayerRow {
    pub layer: MissLayer,
    pub count: usize,
    /// 按板块分组的小计，避免混池假象
    pub by_market_type: Vec<(String, usize)>,
}

/// 历史回补行（`source='tencent_kline'`）不带名称 ⇒ `stock_name` 为空串。
/// 事件名/组标题都读这一格，故用清单表（名称唯一权威）补齐**空名**，已有名不覆盖。
async fn fill_missing_names_from_universe(
    db: &sea_orm::DatabaseConnection,
    series: &mut CloseSeriesByCode,
) -> Result<(), String> {
    let missing: Vec<String> = series
        .iter()
        .filter(|(_, (name, _))| name.is_empty())
        .map(|(code, _)| code.clone())
        .collect();
    if missing.is_empty() {
        return Ok(());
    }
    let rows = market_stock_universe::Entity::find()
        .filter(market_stock_universe::Column::StockCode.is_in(missing))
        .all(db)
        .await
        .map_err(|e| format!("读市场清单(补名称)失败: {e}"))?;
    for r in rows {
        if let Some((name, _)) = series.get_mut(&r.stock_code) {
            if name.is_empty() {
                *name = r.stock_name;
            }
        }
    }
    Ok(())
}

/// 收盘数据按票分组：`code -> (name, [(date, change_pct)])`，日期升序。
pub async fn load_closes_by_stock(
    db: &sea_orm::DatabaseConnection,
    from: &str,
    to: &str,
) -> Result<CloseSeriesByCode, String> {
    let rows = market_daily_close::Entity::find()
        .filter(market_daily_close::Column::TradeDate.gte(from))
        .filter(market_daily_close::Column::TradeDate.lte(to))
        .order_by_asc(market_daily_close::Column::TradeDate)
        .all(db)
        .await
        .map_err(|e| format!("读 market_daily_close 失败: {e}"))?;
    let mut out: CloseSeriesByCode = HashMap::new();
    for r in rows {
        out.entry(r.stock_code.clone())
            .or_insert_with(|| (r.stock_name.clone(), Vec::new()))
            .1
            .push((r.trade_date, r.change_pct));
    }
    fill_missing_names_from_universe(db, &mut out).await?;
    Ok(out)
}

/// 只要指定票的收盘（闭环信号用）：事件表能缩小到「留痕 ∪ 出票」这一小撮票，
/// 避免每次重算/开面板都把全市场 90 天收盘读进来。
pub async fn load_closes_for_codes(
    db: &sea_orm::DatabaseConnection,
    codes: &[String],
    from: &str,
    to: &str,
) -> Result<CloseSeriesByCode, String> {
    if codes.is_empty() {
        return Ok(HashMap::new());
    }
    let rows = market_daily_close::Entity::find()
        .filter(market_daily_close::Column::StockCode.is_in(codes.iter().cloned()))
        .filter(market_daily_close::Column::TradeDate.gte(from))
        .filter(market_daily_close::Column::TradeDate.lte(to))
        .order_by_asc(market_daily_close::Column::TradeDate)
        .all(db)
        .await
        .map_err(|e| format!("读 market_daily_close(指定票) 失败: {e}"))?;
    let mut out: CloseSeriesByCode = HashMap::new();
    for r in rows {
        out.entry(r.stock_code.clone())
            .or_insert_with(|| (r.stock_name.clone(), Vec::new()))
            .1
            .push((r.trade_date, r.change_pct));
    }
    fill_missing_names_from_universe(db, &mut out).await?;
    Ok(out)
}

/// 逐格 mover 证据装配（Phase 4）：把核查结论压成闭环可消费的一张表。
///
/// 与核查同口径、不同规模：只对「留痕 ∪ 出票」涉及的票取收盘（见
/// [`load_closes_for_codes`]），因此可以随闭环重算与面板刷新一起跑。
///
/// - `scored_out`：`reco_scan_audit` 有行 ⇒ 该格算出了该票却排在 top-N 之外；
/// - `hits`：该格在窗口内**非合成**地推荐过该票（合成兜底不算策略主张）。
///
/// 样本不足的格不出现（或缺席）⇒ 上游 `MoverCellSignal::penalty` 返回 `None`，不动权重。
pub async fn load_mover_cell_signal(
    db: &sea_orm::DatabaseConnection,
    vars: &[(String, serde_json::Value)],
    as_of_date: Option<&str>,
) -> Result<crate::recommender::reco_loop::MoverCellSignal, String> {
    use crate::recommender::reco_loop::MoverCellSignal;

    let mut signal = MoverCellSignal::default();
    let rules = tier_rules_from_vars(vars);
    if rules.is_empty() {
        return Ok(signal);
    }
    let dates = load_close_dates(db).await?;
    if dates.is_empty() {
        return Ok(signal);
    }
    let last = dates.last().cloned().unwrap_or_default();
    let to = match as_of_date {
        Some(d) if d < last.as_str() => d.to_string(),
        _ => last,
    };
    let Some(end_idx) = dates.iter().rposition(|d| d.as_str() <= to.as_str()) else {
        return Ok(signal);
    };
    let max_window = rules.iter().map(|r| r.window_days).max().unwrap_or(0) as usize;
    let Some(from_idx) = end_idx.checked_sub(max_window - 1) else {
        return Ok(signal);
    };
    let from = dates[from_idx].clone();

    // 代际 = None：本函数算的是**实盘**格子的漏检信号（`to` 只做到期右端截断，与代际
    // 是两件事）。回放的核查要按代际取留痕，走显式入口，不与实盘分子/分母并池。
    let trims = load_trim_evidence(db, &from, &to, None).await?;
    let picks = load_picks_in_range(db, &from, &to, None).await?;
    // A-2：本信号喂的是**荐股链**的格子权重 ⇒ 趋势智选工作流的出票（style = `serenity`）
    // 不能算成荐股矩阵里 serenity/bottleneck 格的「命中」。混进去的后果不是报错而是
    // penalty = (hits+1)/(hits+scored_out+1) 的分子被外链抬高 ⇒ 该降的格降不下来。
    let picks: Vec<reco_picks::Model> =
        picks.into_iter().filter(|p| RecoChain::of_style(&p.style) == RecoChain::Reco).collect();

    // 候选票集合 = 留痕涉及的 ∪ 该窗口内出过票的（其余票不可能进入任何一格的分子/分母）
    let mut codes: Vec<String> = trims.keys().map(|(_, c)| c.clone()).collect();
    for p in &picks {
        codes.push(p.stock_code.clone());
    }
    codes.sort();
    codes.dedup();

    let by_stock = load_closes_for_codes(db, &codes, &from, &to).await?;

    for rule in &rules {
        let window = rule.window_days as usize;
        if end_idx + 1 < window {
            continue;
        }
        let anchor_from = dates[end_idx + 1 - window].clone();
        // 与核查同口径：只看最新收盘日的事件。旧实现会把窗口内每个 anchor 都算一遍，
        // 同一票连续多日达标会在 `scored_out`/`hits` 里重复计数，penalty 被放大。
        for ev in events_for_tier(&by_stock, rule, &anchor_from, &to, &dates)
            .into_iter()
            .filter(|ev| ev.anchor_date == to)
        {
            let Some(anchor_idx) = dates.iter().position(|d| *d == ev.anchor_date) else {
                continue;
            };
            if anchor_idx + 1 < window {
                continue;
            }
            let window_start = dates[anchor_idx + 1 - window].clone();
            let key = (rule.period.as_str().to_string(), ev.stock_code.clone());
            for style in trims.get(&key).map(Vec::as_slice).unwrap_or(&[]) {
                signal
                    .cells
                    .entry((style.clone(), rule.period.as_str().to_string()))
                    .or_default()
                    .0 += 1;
            }
            let mut hit_styles: Vec<String> = picks
                .iter()
                .filter(|p| {
                    p.stock_code == ev.stock_code
                        && p.period == rule.period.as_str()
                        && p.synthetic == 0
                        && p.generated_at.as_str() >= window_start.as_str()
                        && p.generated_at.as_str() <= ev.anchor_date.as_str()
                })
                .map(|p| normalize_style_key(&p.style).to_string())
                .collect();
            hit_styles.sort();
            hit_styles.dedup();
            for style in hit_styles {
                signal.cells.entry((style, rule.period.as_str().to_string())).or_default().1 += 1;
            }
        }
    }
    Ok(signal)
}

/// 清单规模。
pub async fn load_universe_size(db: &sea_orm::DatabaseConnection) -> Result<i64, String> {
    market_stock_universe::Entity::find()
        .count(db)
        .await
        .map(|n| n as i64)
        .map_err(|e| format!("统计清单失败: {e}"))
}

/// 清单规模 + 被东财全量确认过的只数（后者低于前者 ⇒ 枚举域由本地票拼成，必须显式声明）。
pub async fn load_universe_counts(db: &sea_orm::DatabaseConnection) -> Result<(i64, i64), String> {
    use sea_orm::Condition;
    let all = market_stock_universe::Entity::find()
        .count(db)
        .await
        .map_err(|e| format!("统计清单失败: {e}"))? as i64;
    let confirmed = market_stock_universe::Entity::find()
        .filter(Condition::all().add(market_stock_universe::Column::LastConfirmedAt.gt(0)))
        .count(db)
        .await
        .map_err(|e| format!("统计清单确认数失败: {e}"))? as i64;
    Ok((all, confirmed))
}

/// 已落库的交易日（升序，去重）。
///
/// **必须由 SQL 做 DISTINCT**：旧实现 `.all(db)` 把整张 `market_daily_close` 逐行装配成
/// Model 再在内存去重 —— 现网该表 146 万行 / 305 MB，而结果只有 ~6500 个交易日。
/// 2026-10-28 实测：闭环面板每次刷新都在这一步耗掉 10s 级（`reco_ic_stats` 的
/// `loop_view=10232ms`），SQL 侧 `DISTINCT + ORDER BY trade_date` 走既有索引仅 ~285ms。
/// 只回一列字符串，语义与旧实现逐字相同（升序、去重）。
pub async fn load_close_dates(db: &sea_orm::DatabaseConnection) -> Result<Vec<String>, String> {
    use sea_orm::QuerySelect;
    market_daily_close::Entity::find()
        .select_only()
        .column(market_daily_close::Column::TradeDate)
        .distinct()
        .order_by_asc(market_daily_close::Column::TradeDate)
        .into_tuple::<String>()
        .all(db)
        .await
        .map_err(|e| format!("读快照日期失败: {e}"))
}

/// 窗口必须覆盖的真实交易日比例（分子/分母）：28 个交易日的窗口里至少 23 行。
/// 停牌可以缺几天，但不能用「28 行横跨十几年」的历史回补行凑窗口（现网 22 条伪窗口）。
pub const WINDOW_COVERAGE_NUM: usize = 4;
pub const WINDOW_COVERAGE_DEN: usize = 5;

/// 某票在 `[anchor_from, anchor_to]` 内、某档的达标事件（纯函数，便于单测直接喂构造数据）。
///
/// 窗口起点取**全局交易日历**（`calendar`，升序；`market_daily_close` 的 distinct 日期），
/// 不再取「该票自己的第 N 行」：停牌/稀疏历史会让后者把一个 28 日窗口拼成十几年。
/// 该票在全局窗口内必须有 `WINDOW_COVERAGE_NUM/WINDOW_COVERAGE_DEN` 以上的真实行，
/// 否则显式跳过（不产事件、不计入可判票数）。
pub fn events_for_tier(
    closes: &CloseSeriesByCode,
    rule: &TierRule,
    anchor_from: &str,
    anchor_to: &str,
    calendar: &[String],
) -> Vec<MoverEvent> {
    let mut out = Vec::new();
    let window = rule.window_days as usize;
    if window == 0 || calendar.len() < window {
        return out;
    }
    for (code, (name, series)) in closes {
        if series.len() < window {
            // 该票历史行数不足 ⇒ 窗口算不出来，显式跳过（不当 0 处理）
            continue;
        }
        for (pos, (date, _)) in series.iter().enumerate() {
            if date.as_str() < anchor_from || date.as_str() > anchor_to {
                continue;
            }
            let Ok(anchor_idx) = calendar.binary_search_by(|d| d.as_str().cmp(date.as_str()))
            else {
                // 锚点不在全局交易日历里 ⇒ 数据域不一致，跳过并留待后续告警
                continue;
            };
            if anchor_idx + 1 < window {
                continue;
            }
            let window_start = &calendar[anchor_idx + 1 - window];
            // 该票在全局窗口内的行（停牌日可以缺；start_pos 取第一根 >= window_start 的行）
            let start_pos = series.partition_point(|(d, _)| d.as_str() < window_start.as_str());
            if start_pos > pos {
                continue;
            }
            let win: Vec<f64> = series[start_pos..=pos].iter().map(|(_, r)| *r).collect();
            if win.len() * WINDOW_COVERAGE_DEN < window * WINDOW_COVERAGE_NUM {
                continue;
            }
            let Some(cum) = window_cum_gain_pct(&win) else { continue };
            if cum < rule.gain_pct {
                continue;
            }
            let max_daily = win.iter().cloned().fold(f64::MIN, f64::max);
            out.push(MoverEvent {
                stock_code: code.clone(),
                stock_name: name.clone(),
                period: rule.period.as_str().to_string(),
                anchor_date: date.clone(),
                window_start: window_start.clone(),
                window_days: rule.window_days,
                cum_gain_pct: cum,
                max_daily_pct: max_daily,
                threshold_pct: rule.gain_pct,
                market_type: axagent_harness::market_data::detect_market_type(code).to_string(),
                recommended: false,
            });
        }
    }
    out
}

/// 该票在全局日历上是否存在**至少一个**可判窗口（用于 `judgeable_codes_by_tier`）。
///
/// 与 [`events_for_tier`] 逐字同判据（两道门都要过）：该票历史行数 ≥ 窗口天数，
/// 且窗口内真实行覆盖 ≥ `NUM/DEN`。只看「该票自己的行数」会把稀疏历史算成可判。
pub fn has_judgeable_window(
    series: &[(String, f64)],
    calendar: &[String],
    window_days: u32,
) -> bool {
    let window = window_days as usize;
    if window == 0 || series.len() < window || calendar.len() < window {
        return false;
    }
    series.iter().enumerate().any(|(pos, (date, _))| {
        let Ok(anchor_idx) = calendar.binary_search_by(|d| d.as_str().cmp(date.as_str())) else {
            return false;
        };
        if anchor_idx + 1 < window {
            return false;
        }
        let window_start = &calendar[anchor_idx + 1 - window];
        let start_pos = series.partition_point(|(d, _)| d.as_str() < window_start.as_str());
        if start_pos > pos {
            return false;
        }
        (pos + 1 - start_pos) * WINDOW_COVERAGE_DEN >= window * WINDOW_COVERAGE_NUM
    })
}

/// 某票在某档窗口起点之前是否已被推荐（`generated_at` 落在 `[window_start, anchor]` 内 ⇒ 命中）。
pub fn pick_generated_in_range(
    picks: &[reco_picks::Model],
    code: &str,
    period: &str,
    from: &str,
    to: &str,
) -> bool {
    picks.iter().any(|p| {
        p.stock_code == code
            && p.period == period
            && p.generated_at.as_str() >= from
            && p.generated_at.as_str() <= to
    })
}

/// 窗口内全部截断留痕，按 `(period, code)` 归组为**风格键集合**（去重升序）。
///
/// 一次查库供全事件复用：逐事件查会把核查变成 N 次往返。
/// 键是矩阵名目归一后的写法（`serenity` ⇔ `bottleneck` 合并，见 `style_matrix`）。
pub async fn load_trim_evidence(
    db: &sea_orm::DatabaseConnection,
    from: &str,
    to: &str,
    as_of_date: Option<&str>,
) -> Result<HashMap<(String, String), Vec<String>>, String> {
    use axagent_entities::reco_scan_audit;
    let query = reco_scan_audit::Entity::find()
        .filter(reco_scan_audit::Column::GeneratedAt.gte(from))
        .filter(reco_scan_audit::Column::GeneratedAt.lte(to))
        // 代际：`None` = 只看实盘留痕（既有口径不变）；`Some(YYYY-MM-DD)` = 只看那一次
        // 回放的留痕。回放行的 `generated_at` 是入库时刻（今天），不加这一条就会让
        // 「今天的池」被历史池冒充 —— 两套计数域并排比是禁止形态。
        .filter(match as_of_date {
            Some(d) => reco_scan_audit::Column::AsOfDate.eq(d.to_string()),
            None => reco_scan_audit::Column::AsOfDate.is_null(),
        });
    let rows = query.all(db).await.map_err(|e| format!("读 reco_scan_audit 失败: {e}"))?;
    let mut out: HashMap<(String, String), Vec<String>> = HashMap::new();
    for r in rows {
        let key = (r.period.clone(), r.stock_code.clone());
        let normalized = normalize_style_key(&r.style);
        let bucket = out.entry(key).or_default();
        if !bucket.iter().any(|s| s == normalized) {
            bucket.push(normalized.to_string());
        }
    }
    for v in out.values_mut() {
        v.sort();
    }
    Ok(out)
}

/// 风格键归一：落库写法（`bottleneck`）与矩阵名目（`serenity`）合并为一个键。
pub fn normalize_style_key(style: &str) -> &str {
    match style {
        "bottleneck" => "serenity",
        other => other,
    }
}

/// 该票是否出现在任一候选池快照里（`seed_pool_json` 是 `[code, name]` 数组的 JSON）。
pub fn code_in_pool_snapshot(pool_json: Option<&str>, code: &str) -> bool {
    code_in_pool_snapshot_opt(pool_json, code).unwrap_or(false)
}

/// 三态版：`Some(true)` 在池 / `Some(false)` 不在池 / `None` **无从判定**。
///
/// `None` 的两种来源都必须与「不在池」分开：
/// - 快照缺失（`pool_json=None`）；
/// - **顶层对象形态**（趋势智选把单个候选写进 `seed_pool_json`）且对象不是该 code ——
///   那只能说明「这一只是当时那个候选」，**不能**说明「池里没有别的票」。
///   旧实现对此恒返回 `false`，现网 21/21 智选事件的 `not_in_pool` 正是这么来的伪证据。
pub fn code_in_pool_snapshot_opt(pool_json: Option<&str>, code: &str) -> Option<bool> {
    let raw = pool_json?;
    // 顶层对象：趋势智选把**单个候选**整体写进 seed_pool_json（荐股链写的是数组）。
    if let Ok(serde_json::Value::Object(o)) = serde_json::from_str::<serde_json::Value>(raw) {
        let same = o.get("stockCode").and_then(|v| v.as_str()) == Some(code)
            || o.get("code").and_then(|v| v.as_str()) == Some(code);
        return if same { Some(true) } else { None };
    }
    let Ok(parsed) = serde_json::from_str::<Vec<serde_json::Value>>(raw) else {
        return None;
    };
    Some(parsed.iter().any(|item| match item {
        serde_json::Value::Array(a) => a.first().and_then(|v| v.as_str()) == Some(code),
        serde_json::Value::Object(o) => {
            o.get("stockCode").and_then(|v| v.as_str()) == Some(code)
                || o.get("code").and_then(|v| v.as_str()) == Some(code)
        },
        _ => false,
    }))
}

/// 读某档在时间区间内的全部推荐（含候选池快照），供归因用。
pub async fn load_picks_in_range(
    db: &sea_orm::DatabaseConnection,
    from: &str,
    to: &str,
    as_of_date: Option<&str>,
) -> Result<Vec<reco_picks::Model>, String> {
    reco_picks::Entity::find()
        .filter(reco_picks::Column::GeneratedAt.gte(from.to_string()))
        .filter(reco_picks::Column::GeneratedAt.lte(format!("{to}T23:59:59")))
        // 代际与 `load_trim_evidence` 同一条谓词：候选池快照（`seed_pool_json`）的
        // 归因必须与留痕同代，否则会出现「实盘的票 + 回放的池」这种拼出来的证据。
        .filter(match as_of_date {
            Some(d) => reco_picks::Column::AsOfDate.eq(d.to_string()),
            None => reco_picks::Column::AsOfDate.is_null(),
        })
        .all(db)
        .await
        .map_err(|e| format!("读 reco_picks 失败: {e}"))
}

/// 某 (style, period) 是否按设计出票 —— 复用唯一权威矩阵，不在此另写档位集合。
/// 某档位是否有**任一**风格出票 —— 个股事件不绑定单一风格，档位级判据看整列。
pub fn period_has_active_style(period: Period) -> bool {
    style_matrix::style_keys()
        .iter()
        .any(|k| style_matrix::reason_code_by_key(k, period) == "cell_is_active")
}

pub fn matrix_active(style_key: &str, period: Period) -> bool {
    // 走 reason_code_by_key（字符串名目 + serenity/bottleneck 别名归一）：
    // 落库两名同格，不经别名合并永远判错。
    style_matrix::reason_code_by_key(style_key, period) == "cell_is_active"
}

/// 智选家族在矩阵里那一行的名目（`style_keys()` 里的写法，非落库别名 `bottleneck`）。
const SERENITY_MATRIX_KEY: &str = "serenity";

/// 某**链**在某档是否按设计出票（A-3 的 `ByDesign` 唯一判据入口）。
///
/// - 智选链只看 `serenity` 那一行：趋势智选按设计不做超短/短档（矩阵声明
///   `serenity_needs_week_or_longer_realization`）。那些达标事件不是「漏检」而是「按设计」，
///   混进漏检分母会把优化方向带去补一个本就不该出的格。
/// - 荐股链沿用档位级判据（矩阵任一风格出票）⇒ 行为与 A-3 之前逐字一致，
///   不因拆链而改口径；「整档无任何风格」这种真告警也仍由这一支给出。
pub fn chain_period_has_active_style(chain: RecoChain, period: Period) -> bool {
    match chain {
        RecoChain::Serenity => matrix_active(SERENITY_MATRIX_KEY, period),
        RecoChain::Reco => period_has_active_style(period),
    }
}

/// 回灌清单转正所需的**已结算样本**下限（事前注册；D-4 第二段的 fail-closed 判据）。
///
/// 判据语义：样本不足 ⇒ 保持 shadow（**不是**自动放行）。三项质量指标（命中率不降 / 池内覆盖率不降 /
/// 逐档 rank IC 不劣化）在打开人工开关前由人核对；代码只守这道下限，防止「没数据就转正」。
pub const REFLOW_MIN_SETTLED: usize = 30;

/// 是否已达转正样本下限（供面板成句与 `refresh_reflow_seed_db` 的 shadow 判据共用）。
pub fn reflow_ready(settled: usize) -> bool {
    settled >= REFLOW_MIN_SETTLED
}

/// as-of 回放时注入智选的显式声明（**不**取 live 的达标清单，开了就是前视）。
pub const MOVER_WATCHLIST_ASOF_DECLARATION: &str =
    "本代无达标清单（as-of 回放不取 live 的窗口涨幅清单，避免前视）";

/// 纯函数：把「最新收盘日的窗口达标事件」拼成注入智选提示词的清单文本。
///
/// 这是**输入面**素材，不是新判据：达标判据仍是 `mover_gain_*` + `default_holding_days()`，
/// 这里只做「把同一判据的结果摆到 LLM 眼前」——清单不构成入选理由，证据门槛不因此下降。
/// 同票多档只留累计涨幅最大的那条（清单是「票」的集合，不是「票×档」）。
pub fn format_mover_watchlist(events: &[MoverEvent], to: &str, limit: usize) -> String {
    let mut rows: Vec<&MoverEvent> = events.iter().filter(|e| e.anchor_date == to).collect();
    rows.sort_by(|a, b| b.cum_gain_pct.total_cmp(&a.cum_gain_pct));
    let mut seen = std::collections::HashSet::new();
    rows.retain(|e| seen.insert(e.stock_code.clone()));
    rows.truncate(limit);
    if rows.is_empty() {
        return format!(
            "（{to} 收盘窗口：无达标票 —— 判据为窗口累计涨幅，四档阈值见模板变量 mover_gain_*）"
        );
    }
    let mut out = format!(
        "{to} 收盘窗口达标 {} 只（判据=窗口累计涨幅；清单只供优先核对，不构成入选理由）：\n",
        rows.len()
    );
    for e in rows {
        out.push_str(&format!(
            "- {} {} [{}] 累计{:+.1}%（窗口 {} 日）\n",
            e.stock_code, e.stock_name, e.period, e.cum_gain_pct, e.window_days
        ));
    }
    out
}

/// 取最新收盘日的窗口达标清单（B-1：智选 `a-trend-scanner` 的第 4 个输入）。
///
/// 只回看最长窗口那么多天 ⇒ 代价与一次核查的取数同阶，不随采集历史增长。
pub async fn mover_watchlist_digest(
    db: &sea_orm::DatabaseConnection,
    limit: usize,
) -> Result<String, String> {
    let served = crate::recommender::reco_loop::load_reco_served_vars_db(db).await?;
    let rules = tier_rules_from_vars(&served);
    let close_dates = load_close_dates(db).await?;
    let Some(to) = close_dates.last().cloned() else {
        return Ok("（尚无收盘快照：无达标清单）".to_string());
    };
    // 窗口天数只从规则来（规则本身取 default_holding_days）；没有规则 ⇒ 显式声明，不写死下限
    let Some(max_days) = rules.iter().map(|r| r.window_days).max() else {
        return Ok("（本代无达标清单：四档阈值变量全部无效）".to_string());
    };
    let max_days = max_days as usize;
    let from = close_dates
        .get(close_dates.len().saturating_sub(max_days))
        .cloned()
        .unwrap_or_else(|| close_dates[0].clone());
    let by_stock = load_closes_by_stock(db, &from, &to).await?;
    let mut events = Vec::new();
    for rule in &rules {
        events.extend(events_for_tier(&by_stock, rule, &from, &to, &close_dates));
    }
    Ok(format_mover_watchlist(&events, &to, limit))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rule(period: Period, gain: f64, days: u32) -> TierRule {
        TierRule { period, var_name: "mover_gain_mid", gain_pct: gain, window_days: days }
    }

    fn closes_map(entries: Vec<(String, Vec<(&str, f64)>)>) -> CloseSeriesByCode {
        entries
            .into_iter()
            .map(|(code, rows)| {
                (
                    code.clone(),
                    (
                        format!("{code}-name"),
                        rows.into_iter().map(|(d, r)| (d.to_string(), r)).collect(),
                    ),
                )
            })
            .collect()
    }

    #[test]
    fn cum_gain_compounds_instead_of_summing() {
        // 两日各 +10% ⇒ 累计 21%，不是 20%
        let v = window_cum_gain_pct(&[10.0, 10.0]).unwrap();
        assert!((v - 21.0).abs() < 1e-6, "得 {v}");
    }

    #[test]
    fn empty_or_dirty_window_is_none_not_zero() {
        assert_eq!(window_cum_gain_pct(&[]), None);
        assert_eq!(window_cum_gain_pct(&[f64::NAN]), None);
    }

    fn calendar_of(entries: &[&str]) -> Vec<String> {
        entries.iter().map(|d| d.to_string()).collect()
    }

    #[test]
    fn event_requires_full_window_length() {
        let closes =
            closes_map(vec![("600059".into(), vec![("2026-09-28", 6.0), ("2026-09-29", 6.0)])]);
        let calendar = calendar_of(&["2026-09-28", "2026-09-29"]);
        // 窗口 3 天但全局窗口只有 2 行 ⇒ 算不出，必须显式跳过（不得按 2 天判达标）
        let ev = events_for_tier(
            &closes,
            &rule(Period::Short, 10.0, 3),
            "2026-09-01",
            "2026-12-31",
            &calendar,
        );
        assert!(ev.is_empty());
        let ev2 = events_for_tier(
            &closes,
            &rule(Period::Short, 10.0, 2),
            "2026-09-01",
            "2026-12-31",
            &calendar,
        );
        assert_eq!(ev2.len(), 1, "窗口 2 天时应在 09-29 出事件");
        assert!((ev2[0].cum_gain_pct - 12.36).abs() < 1e-6);
        assert!((ev2[0].max_daily_pct - 6.0).abs() < 1e-6);
        // P2：window_start 必须是窗口首日（补分析的 as-of 锚点），
        // anchor_date 是窗口右端；二者不得互换或都取右端。
        assert_eq!(ev2[0].anchor_date, "2026-09-29");
        assert_eq!(ev2[0].window_start, "2026-09-28");
    }

    #[test]
    fn window_start_rolls_with_each_anchor() {
        // 连续 4 个交易日、窗口 3 天、低阈值 ⇒ 09-30 / 10-09 两个 anchor 各出一事件，
        // window_start 分别为窗口首日（09-28 / 09-29）。
        let closes = closes_map(vec![(
            "600059".into(),
            vec![
                ("2026-09-28", 6.0),
                ("2026-09-29", 6.0),
                ("2026-09-30", 6.0),
                ("2026-10-09", 6.0),
            ],
        )]);
        let calendar = calendar_of(&["2026-09-28", "2026-09-29", "2026-09-30", "2026-10-09"]);
        let ev = events_for_tier(
            &closes,
            &rule(Period::Short, 1.0, 3),
            "2026-09-01",
            "2026-12-31",
            &calendar,
        );
        assert_eq!(ev.len(), 2, "09-30 与 10-09 各满 3 日窗口");
        assert_eq!(ev[0].anchor_date, "2026-09-30");
        assert_eq!(ev[0].window_start, "2026-09-28");
        assert_eq!(ev[1].anchor_date, "2026-10-09");
        assert_eq!(ev[1].window_start, "2026-09-29");
    }

    /// 回归：稀疏历史行（2005 与 2026）不得拼成一个 28 行窗口。
    /// 旧实现按该票自己的行数取窗口 ⇒ window_start 落到 2005，累计涨幅是旧价对新价的伪值。
    #[test]
    fn sparse_history_rows_do_not_form_a_window() {
        let closes = closes_map(vec![(
            "000562".into(),
            vec![("2005-04-06", 1.0), ("2026-09-28", 2.0), ("2026-09-30", 6.0)],
        )]);
        let calendar = calendar_of(&["2005-04-06", "2026-09-28", "2026-09-29", "2026-09-30"]);
        let ev = events_for_tier(
            &closes,
            &rule(Period::Mid, 30.0, 3),
            "2026-09-01",
            "2026-12-31",
            &calendar,
        );
        assert!(ev.is_empty(), "2026 窗口内只有 2 行（覆盖不足 80%）⇒ 不得产事件: {ev:?}");
        assert!(!has_judgeable_window(&closes["000562"].1, &calendar, 3), "稀疏行不得计入可判票数");
    }

    #[test]
    fn tier_rules_read_vars_and_take_windows_from_single_source() {
        let vars = vec![
            ("mover_gain_ultra_short".to_string(), serde_json::json!(8.0)),
            ("mover_gain_mid".to_string(), serde_json::json!("35")),
            // 非法阈值 ⇒ 该档显式跳过，不吃默认值蒙混
            ("mover_gain_short".to_string(), serde_json::json!(0.0)),
        ];
        let rules = tier_rules_from_vars(&vars);
        // 四档里 short 被非法值剔除，其余三档成立（long 未配变量 ⇒ 吃出厂缺省 40）
        assert_eq!(rules.len(), 3, "short 阈值为 0 应被跳过，其余三档保留");
        assert!(rules.iter().all(|r| r.period != Period::Short), "非法阈值的档必须显式缺席");
        let ultra = rules.iter().find(|r| r.period == Period::UltraShort).unwrap();
        assert_eq!(ultra.gain_pct, 8.0);
        assert_eq!(
            ultra.var_name, "mover_gain_ultra_short",
            "DTO 必须报模板变量全名，供面板指认来源"
        );
        assert_eq!(ultra.window_days, 2, "窗口天数必须取唯一天数来源");
        let mid = rules.iter().find(|r| r.period == Period::Mid).unwrap();
        assert_eq!(mid.gain_pct, 35.0, "字符串形态的变量值也要吃");
    }

    /// 默认「该档确实出票」，各用例只改自己要判的那一件事。
    ///
    /// ⚠ 默认构造里 `in_pool = None`（= 本窗没有任何荐股批次），因此**什么都不改的那条
    /// 断言现在落 `Unexplained` 而不是 `NotInPool`** —— 这正是阶段 5 / G1 的修复点：
    /// 「没扫描过」以前被压成「不在池里」，把留痕问题报成了候选宇宙问题。
    fn ev(mutate: impl FnOnce(&mut Evidence)) -> Evidence {
        let mut e = Evidence { chain_tier_active: true, ..Default::default() };
        mutate(&mut e);
        e
    }

    #[test]
    fn attribution_priority_is_mechanical() {
        assert_eq!(
            attribute(&ev(|e| e.picks_for_period.push("2026-09-20T15:00:00".into()))),
            MissLayer::Recommended
        );
        // 同档在窗口起点之前就出过票 ⇒ 晚推荐（A-3 接的就是这一层），排在池层之前
        assert_eq!(attribute(&ev(|e| e.picked_before_window = true)), MissLayer::PriorCovered);
        // 本链按设计不出这一档 ⇒ 优先于池层（严禁进待优化清单）
        assert_eq!(attribute(&ev(|e| e.chain_tier_active = false)), MissLayer::ByDesign);
        assert_eq!(attribute(&ev(|e| e.pool_degraded = true)), MissLayer::PoolDegraded);
        // 确有批次、快照里查无此票 ⇒ 这才是「不在候选宇宙」
        assert_eq!(attribute(&ev(|e| e.in_pool = Some(false))), MissLayer::NotInPool);
        // G1 负控：无批次（None）与「不在池里」必须落在不同的桶
        assert_eq!(attribute(&ev(|_| {})), MissLayer::Unexplained);
        assert_eq!(
            attribute(&ev(|e| {
                e.in_pool = Some(true);
                e.picked_in_other_period = true;
            })),
            MissLayer::OtherPeriod
        );
        // 入池但无截断留痕 ⇒ 判不出（策略内部否决不落痕），进兜底桶
        assert_eq!(attribute(&ev(|e| e.in_pool = Some(true))), MissLayer::Unexplained);
        // 入池 + 截断留痕 ⇒ 唯一可归因到具体风格、可据此降权的一层
        assert_eq!(
            attribute(&ev(|e| {
                e.in_pool = Some(true);
                e.l3_scored_out_styles.push("trend".into());
            })),
            MissLayer::L3ScoredOut
        );
        assert!(!MissLayer::ByDesign.counts_as_miss());
        assert!(
            !MissLayer::PriorCovered.counts_as_miss(),
            "晚推荐不是漏检：它的优化方向是时机，不是候选宇宙"
        );
        assert!(MissLayer::L3ScoredOut.counts_as_miss());
        assert!(MissLayer::Unexplained.counts_as_miss());
    }

    /// 逐层可达性（**充分条件**）：每一层都必须能用一组真实形态的证据造出来。
    ///
    /// 门 p（`check-reco-horizon-parity.mjs`）只证必要条件「`attribute()` 里写了这一层」；
    /// 写了但条件永远凑不齐，照样是死料 —— `PriorCovered` 与 `ByDesign` 两层在 2026-10-21 之前
    /// 就是这么躺着的：枚举、`as_str`、面板格、11 语言全齐备，看着像八层归因，实际只产五层。
    /// 遍历面是 `MissLayer::ALL`，门 p 拿枚举对这张表 ⇒ 加一层不写夹具就红。
    #[test]
    fn every_miss_layer_is_reachable_from_attribute() {
        type Fixture = (MissLayer, fn(&mut Evidence));
        let fixtures: [Fixture; 8] = [
            (MissLayer::Recommended, |e| e.picks_for_period.push("2026-10-09T15:00:00".into())),
            (MissLayer::PriorCovered, |e| e.picked_before_window = true),
            (MissLayer::ByDesign, |e| e.chain_tier_active = false),
            (MissLayer::PoolDegraded, |e| e.pool_degraded = true),
            (MissLayer::NotInPool, |e| e.in_pool = Some(false)),
            (MissLayer::OtherPeriod, |e| {
                e.in_pool = Some(true);
                e.picked_in_other_period = true;
            }),
            (MissLayer::L3ScoredOut, |e| {
                e.in_pool = Some(true);
                e.l3_scored_out_styles.push("trend".into());
            }),
            // 兜底桶的证据就是「齐备但判不出」：入池、无留痕、无别档出票
            (MissLayer::Unexplained, |e| e.in_pool = Some(true)),
        ];
        let mut produced: Vec<&'static str> = Vec::new();
        for (want, mutate) in fixtures {
            let got = attribute(&ev(mutate));
            assert_eq!(
                got,
                want,
                "层 {} 造不出来 ⇒ 判据不可达（要么接进生产路径，要么整层退役：枚举 + as_str + ALL + i18n 一起删）",
                want.as_str()
            );
            produced.push(want.as_str());
        }
        assert_eq!(MissLayer::ALL.len(), produced.len(), "ALL 与逐层夹具必须同数");
        for layer in MissLayer::ALL {
            assert!(produced.contains(&layer.as_str()), "ALL 里的 {} 没有对应夹具", layer.as_str());
        }

        // 负控拆在优先级本身：同一份证据同时满足两支时，必须落在前一支上，
        // 否则「写了这一层」可以靠任意顺序蒙过上面的逐层断言。
        let both = attribute(&ev(|e| {
            e.chain_tier_active = false;
            e.picked_before_window = true;
        }));
        assert_eq!(both, MissLayer::PriorCovered, "PriorCovered 必须排在 ByDesign 之前");
        let hit_wins = attribute(&ev(|e| {
            e.picks_for_period.push("2026-10-09T15:00:00".into());
            e.picked_before_window = true;
        }));
        assert_eq!(hit_wins, MissLayer::Recommended, "窗口内命中必须排在晚推荐之前");
    }

    /// D-4：转正样本门槛是**下限**语义 —— 少一个都只能留在 shadow，边界两侧逐一钉住。
    #[test]
    fn reflow_ready_is_a_hard_floor() {
        assert!(!reflow_ready(0));
        assert!(!reflow_ready(REFLOW_MIN_SETTLED - 1));
        assert!(reflow_ready(REFLOW_MIN_SETTLED));
        assert!(reflow_ready(REFLOW_MIN_SETTLED + 1));
    }

    /// B-1：达标清单是**输入面**素材 ⇒ 只影响 LLM 看到什么，不新增判据。
    /// 三条形状锁：声明层（空清单必须成句且带判据来源）、排序与截断、同票多档只留一条。
    #[test]
    fn watchlist_digest_is_a_sorted_deduped_declaration() {
        let mk = |code: &str, period: &str, gain: f64| MoverEvent {
            stock_code: code.to_string(),
            stock_name: format!("股{code}"),
            period: period.to_string(),
            anchor_date: "2026-10-20".to_string(),
            window_start: "2026-10-19".to_string(),
            // 天数必须来自唯一权威（门 k 的判据就是「窗口天数不得出现第二份字面量」）
            window_days: Period::UltraShort.default_holding_days(),
            cum_gain_pct: gain,
            max_daily_pct: 10.0,
            threshold_pct: 10.0,
            market_type: "main_sz".to_string(),
            recommended: false,
        };
        let empty = format_mover_watchlist(&[], "2026-10-20", 20);
        assert!(empty.contains("无达标票"), "空清单必须成句：{empty}");
        assert!(empty.contains("mover_gain_*"), "必须写明判据来源，别让人以为是新判据：{empty}");

        // 同一只票在超短与短档各一条 ⇒ 只留累计涨幅大的那条；排序按降序；limit 生效
        let events = vec![
            mk("002696", "ultra_short", 21.0),
            mk("600519", "short", 35.0),
            mk("002696", "short", 24.0),
            mk("000001", "ultra_short", 12.0),
        ];
        let text = format_mover_watchlist(&events, "2026-10-20", 2);
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 3, "标题 + 两行（limit=2）：{text}");
        assert!(lines[1].contains("600519"), "第一行应是涨幅最大的：{text}");
        assert!(lines[2].contains("002696"), "第二行应是同票里较高的那条：{text}");
        // 按行首匹配：夹具的名字里也含代码（股002696），数裸子串会把一行数成两次
        let code_lines = text.lines().filter(|l| l.starts_with("- 002696 ")).count();
        assert_eq!(code_lines, 1, "同票多档只能留一条：{text}");
        assert!(!text.contains("000001"), "limit 之外的票不得出现：{text}");

        // 只认 anchor == to 的事件（清单是「最新收盘日窗口达标」，不是历史全集）
        let stale = format_mover_watchlist(&[mk("000001", "ultra_short", 99.0)], "2026-10-21", 5);
        assert!(stale.contains("无达标票"), "非当日锚点必须不算：{stale}");
    }

    #[test]
    fn zero_denominators_yield_nan_not_zero() {
        let r = compute_rates(&HashMap::new(), 0);
        assert!(r.reachability.is_nan(), "无事件时必须算不出来，不能显示 0%");
        assert!(r.coverage.is_nan());
        assert!(r.unexplained_share.is_nan());
    }

    /// NaN 进 JSON 是硬错误（serde_json 拒绝非有限值）⇒ 跨 IPC 必须转成 null，
    /// 否则「样本不足」会把整条命令带崩，而不是在面板上显示成一句说明。
    #[test]
    fn nan_rates_serialize_as_null_not_error() {
        let r = compute_rates(&HashMap::new(), 0);
        let v = serde_json::to_value(r).expect("NaN 不得让序列化失败");
        assert_eq!(v["reachability"], serde_json::Value::Null);
        assert_eq!(v["coverage"], serde_json::Value::Null);
        assert_eq!(v["unexplainedShare"], serde_json::Value::Null);
        assert_eq!(v["events"], 0);
    }

    #[test]
    fn rates_use_pool_scoped_denominators() {
        let mut counts = HashMap::new();
        counts.insert(MissLayer::Recommended, 3);
        counts.insert(MissLayer::L3ScoredOut, 6);
        counts.insert(MissLayer::Unexplained, 1);
        counts.insert(MissLayer::NotInPool, 10);
        let r = compute_rates(&counts, 20);
        assert!((r.reachability - 50.0).abs() < 1e-6, "得 {}", r.reachability);
        assert!((r.coverage - 30.0).abs() < 1e-6, "得 {}", r.coverage);
        assert_eq!(r.misses, 17);
        assert_eq!(r.judgeable_events, Some(20));
    }

    /// 可判分母排除 `ByDesign` / `PriorCovered`；`PoolDegraded` 不算「可达」。
    #[test]
    fn judgeable_events_exclude_non_judgeable_layers() {
        let mut counts = HashMap::new();
        counts.insert(MissLayer::Recommended, 3);
        counts.insert(MissLayer::NotInPool, 3);
        counts.insert(MissLayer::ByDesign, 2);
        counts.insert(MissLayer::PriorCovered, 1);
        counts.insert(MissLayer::PoolDegraded, 1);
        let r = compute_rates(&counts, 10);
        assert_eq!(r.judgeable_events, Some(7), "10 - 2 - 1 = 7");
        assert!(
            (r.reachability - 42.857_142_857).abs() < 1e-5,
            "PoolDegraded 不算可达，得 {}",
            r.reachability
        );
        assert!((r.coverage - 100.0).abs() < 1e-6, "3 命中 / 3 可判池内，得 {}", r.coverage);
        assert_eq!(r.misses, 4, "NotInPool 3 + PoolDegraded 1");
    }

    #[test]
    fn pool_snapshot_accepts_pair_and_object_shapes() {
        assert!(code_in_pool_snapshot(Some("[[\"600059\",\"name\"]]"), "600059"));
        assert!(code_in_pool_snapshot(Some("[{\"stockCode\":\"600059\"}]"), "600059"));
        assert!(!code_in_pool_snapshot(Some("[]"), "600059"));
        assert!(!code_in_pool_snapshot(Some("not json"), "600059"));
        assert!(!code_in_pool_snapshot(None, "600059"));
    }

    #[test]
    fn by_design_cells_come_from_the_matrix_not_from_here() {
        // 趋势智选超短/短两档 = 按设计不出票（档位集合的唯一权威是 style_matrix）
        assert!(!matrix_active("serenity", Period::UltraShort));
        assert!(!matrix_active("serenity", Period::Short));
        assert!(matrix_active("serenity", Period::Mid));
        assert!(matrix_active("serenity", Period::Long));
        // value 超短是「出票但已登记档-因子错配」（MISFIT_DECLARATIONS），不是缺席
        assert!(matrix_active("value", Period::UltraShort));
        assert!(matrix_active("trend", Period::Short));
        // 矩阵里没有的名目 ⇒ 判不成立（实现漏格不能被当成按设计不做）
        assert!(!matrix_active("no_such_style", Period::Mid));
    }
    /// A-3 的链级档位判据：矩阵里 `serenity` 那一行按设计不做超短/短，mid/long 才出票；
    /// 荐股链那一支与档位级判据**逐字同义** ⇒ 拆链不改动荐股已有口径（只把智选的账分开算）。
    /// 名目存在性也钉住：`SERENITY_MATRIX_KEY` 若不是 `style_keys()` 真有的写法，
    /// 智选链那一支会恒假 —— 于是每条智选事件都被判成「按设计」，门与面板全都看不出来。
    #[test]
    fn chain_tier_activity_is_scoped_per_chain() {
        assert!(
            style_matrix::style_keys().contains(&SERENITY_MATRIX_KEY),
            "判据依赖的名目必须真的是矩阵行，否则智选链那一支恒假"
        );
        assert!(!chain_period_has_active_style(RecoChain::Serenity, Period::UltraShort));
        assert!(!chain_period_has_active_style(RecoChain::Serenity, Period::Short));
        assert!(chain_period_has_active_style(RecoChain::Serenity, Period::Mid));
        assert!(chain_period_has_active_style(RecoChain::Serenity, Period::Long));
        for p in Period::ALL {
            assert_eq!(
                chain_period_has_active_style(RecoChain::Reco, p),
                period_has_active_style(p),
                "荐股链那一支必须与档位级判据同义（{p:?}）"
            );
        }
    }

    /// #10 P7：妖股标签四态必须各归其位，**两种「拿不到」都不许冒充 `normal`**。
    /// 阈值取 20.0（短线档出厂值）只作夹具，不代表本函数读表 —— 读表由调用方负责。
    #[test]
    fn mover_label_splits_states_and_never_invents_normal() {
        assert_eq!(
            mover_label_for(Some(20.0), Some(20.0), true),
            Some("mover"),
            "恰等于阈值算达标（下限含等号）"
        );
        assert_eq!(mover_label_for(Some(20.0), Some(19.99), true), Some("normal"));
        // 窗口未满：涨得再多也不判定 —— 否则就是拿「当前价」冒充「到期价」
        assert_eq!(mover_label_for(Some(20.0), Some(80.0), false), Some("window_incomplete"));
        // 快照不可得优先于未满（连量都没有，谈不上窗口）
        assert_eq!(mover_label_for(Some(20.0), None, false), Some("no_market_data"));
        // 判据不可用 ⇒ None（调用方写 rule_unavailable；写 normal 就是伪装成算过）
        assert_eq!(mover_label_for(None, Some(80.0), true), None);
        // 脏阈值/脏涨幅都不进比较（`NaN >= x` 恒 false 会被判成「未达标」）
        assert_eq!(mover_label_for(Some(f64::NAN), Some(80.0), true), None);
        assert_eq!(mover_label_for(Some(-1.0), Some(80.0), true), None);
        assert_eq!(mover_label_for(Some(20.0), Some(f64::NAN), true), Some("no_market_data"));
    }

    /// #10 P7：出厂阈值表**防漂移锁**。
    ///
    /// 唯一权威仍是 [`DEFAULT_GAIN_THRESHOLDS`]（达标核查、面板缺省、妖股标签三处都读它）；
    /// 本测试逐字钉住用户裁定的四档值（超短 10 / 短 20 / 中 30 / 长 40），作用是「改动必须显式改这里」，
    /// 不是另开一份判据。键名域同时锁成 `mover_gain_{档}` —— 面板变量名改了这里会红。
    #[test]
    fn mover_threshold_single_source_is_the_exported_table() {
        let defaults: Vec<(&str, f64)> =
            DEFAULT_GAIN_THRESHOLDS.iter().map(|(k, v)| (*k, *v)).collect();
        assert_eq!(
            defaults,
            vec![
                ("mover_gain_ultra_short", 10.0),
                ("mover_gain_short", 20.0),
                ("mover_gain_mid", 30.0),
                ("mover_gain_long", 40.0),
            ]
        );
        let keys: Vec<&str> = defaults.iter().map(|(k, _)| *k).collect();
        let want: Vec<String> =
            Period::ALL.iter().map(|p| format!("mover_gain_{}", p.as_str())).collect();
        assert_eq!(keys, want, "四档各一条、键名必须是 mover_gain_ + 档位 snake 名");
    }

    /// 读回已落库轮次：`rules_json` → `StoredTierRule` → `TierRule` 必须逐字段回到当轮原值，
    /// 且 `var_name` 仍指回登记表（面板显示的「阈值来源变量名」不能换人）。
    #[test]
    fn stored_rules_round_trip_returns_the_same_tier_rules() {
        let live = tier_rules_from_vars(&[("mover_gain_mid".to_string(), serde_json::json!(35.0))]);
        assert_eq!(live.len(), 4, "四档都产出（未配的档吃出厂缺省，不是缺席）");
        let mid = live.iter().find(|r| r.period == Period::Mid).unwrap();
        assert_eq!(mid.gain_pct, 35.0, "配过服务值的档必须吃服务值，不能被出厂缺省顶替");

        let json = serde_json::to_string(&live).unwrap();
        let stored: Vec<StoredTierRule> = serde_json::from_str(&json).unwrap();
        let back = rules_from_stored(&stored).unwrap();
        assert_eq!(back, live, "读回与当轮逐字段相同 => 面板显示的仍是那一代阈值");
        assert_eq!(back[0].var_name, "mover_gain_ultra_short");
    }

    /// 坏样本：登记表里没有的变量名 => 显式 `Err`，不得静默丢一档或另编名字。
    #[test]
    fn rules_from_stored_rejects_unregistered_var_name() {
        let stored: Vec<StoredTierRule> = serde_json::from_str(
            r#"[{"period":"mid","varName":"mover_gain_quarter","gainPct":30.0,"windowDays":28}]"#,
        )
        .unwrap();
        let err = rules_from_stored(&stored).unwrap_err();
        assert!(err.contains("mover_gain_quarter"), "错误消息要点名被拒的变量名，实得: {err}");
    }

    /// 坏样本：NaN 落库已是 `null` => 读回必须是 NaN（「算不出来」），不得折成 0%。
    #[test]
    fn stored_rates_null_restores_nan_not_zero() {
        let live = compute_rates(&HashMap::new(), 0);
        assert!(live.reachability.is_nan() && live.coverage.is_nan());
        let json = serde_json::to_string(&live).unwrap();
        assert!(json.contains("null"), "序列化侧 NaN 已折成 null，实得: {json}");
        let stored: StoredRates = serde_json::from_str(&json).unwrap();
        let back = stored.into_rates();
        assert!(
            back.reachability.is_nan() && back.coverage.is_nan() && back.unexplained_share.is_nan(),
            "三率都必须还原成 NaN"
        );
        assert_eq!(back.events, 0);
    }

    /// A-2 链归属：`bottleneck` 是**荐股矩阵**里的智选格，不是趋势智选工作流的产出 ⇒ 归荐股链。
    /// 这条断言专门与 `normalize_style_key`（bottleneck→serenity，为矩阵取格服务）对撞：
    /// 判链若误用别名归一，两条链的账会互相顶替，而两边看起来都「有数」。
    #[test]
    fn chain_of_style_is_not_alias_normalized() {
        assert_eq!(RecoChain::of_style("serenity"), RecoChain::Serenity);
        assert_eq!(RecoChain::of_style("bottleneck"), RecoChain::Reco);
        assert_eq!(RecoChain::of_style("capital_flow"), RecoChain::Reco);
        assert_eq!(RecoChain::ALL, [RecoChain::Reco, RecoChain::Serenity]);
        assert_eq!(RecoChain::Reco.as_str(), "reco");
        assert_eq!(RecoChain::Serenity.as_str(), "serenity");
    }

    /// A-2 池快照：趋势智选写的是**顶层单对象**，旧实现只吃顶层数组 ⇒ 智选链永远判「不在池」。
    #[test]
    fn pool_snapshot_accepts_top_level_object_shape() {
        assert!(
            code_in_pool_snapshot(Some(r#"{"stockCode":"002696"}"#), "002696"),
            "单对象形态必须认出来"
        );
        assert!(
            !code_in_pool_snapshot(Some(r#"{"stockCode":"002696"}"#), "000001"),
            "同形态换码必须仍判不在池"
        );
        assert!(
            code_in_pool_snapshot(Some(r#"{"code":"001231"}"#), "001231"),
            "键名 code 同样要认"
        );
        assert!(
            !code_in_pool_snapshot(Some(r#"{"name":"只有名字"}"#), "001231"),
            "无码对象不得算命中"
        );
    }
}

#[cfg(test)]
mod asof_generation_tests {
    use super::*;
    use axagent_dao::repo::reco_pool::{
        self as pool_dao, NewPoolBatch, NewRecoPick, NewScanAuditTrim, purpose,
    };
    use axagent_entities::reco_scan_audit;
    use sea_orm::EntityTrait;

    /// 生产写入口落三表（picks / 池批次 / L3 留痕），共用同一 `generated_at` 与代际。
    async fn insert_batch(
        db: &sea_orm::DatabaseConnection,
        generated_at: &str,
        as_of: Option<&str>,
        purpose: &str,
        code: &str,
    ) {
        let pick = NewRecoPick {
            period: "mid".to_string(),
            stock_code: code.to_string(),
            stock_name: format!("股{code}"),
            style: "trend".to_string(),
            confidence: 80,
            synthetic: false,
            pick_data: None,
        };
        let batch = pool_dao::BatchKey { generated_at, as_of_date: as_of, purpose };
        let written = pool_dao::insert_picks(
            db,
            &[pick],
            Some(format!(r#"[["{code}","股{code}"]]"#)),
            None,
            &batch,
            Some(1),
        )
        .await;
        assert_eq!(written, 1, "候选行应写入成功（写失败只 warn，不断言会让夹具静默空表）");
        pool_dao::insert_pool_batch(
            db,
            &NewPoolBatch {
                id: format!("rpb-{generated_at}"),
                chain: "reco".to_string(),
                period: "mid".to_string(),
                generated_at: generated_at.to_string(),
                as_of_date: as_of.map(str::to_string),
                purpose: purpose.to_string(),
                seed_pool_json: Some(format!(r#"[["{code}","股{code}"]]"#)),
                stock_count: 1,
                degraded: false,
                created_at_ms: 1,
            },
        )
        .await
        .expect("池批次应可写入");
        let audit = pool_dao::insert_scan_audit(
            db,
            &[NewScanAuditTrim {
                stock_code: code.to_string(),
                stock_name: format!("股{code}"),
                style: "capital".to_string(),
                confidence: 55,
                rank: 3,
            }],
            "mid",
            &batch,
        )
        .await;
        assert_eq!(audit, 1, "截断留痕应写入成功");
    }

    /// 三张表共用同一批次键 / 代际 / 用途 —— 跨代拼证据（实盘票 + 回放池）在库里就不可读。
    #[tokio::test]
    async fn asof_replay_rows_share_batch_key_generation_and_purpose() {
        let db = axagent_dao::db::create_test_pool().await.expect("测试库应可建立").conn;
        let at = "2026-10-24T10:00:00.000";
        insert_batch(&db, at, Some("2026-09-04"), purpose::MOVER_REPLAY, "002696").await;

        let picks = reco_picks::Entity::find().all(&db).await.unwrap();
        assert_eq!(picks.len(), 1);
        assert_eq!(picks[0].generated_at, at);
        assert_eq!(picks[0].as_of_date.as_deref(), Some("2026-09-04"));
        assert_eq!(picks[0].purpose.as_deref(), Some(purpose::MOVER_REPLAY));

        let batches = pool_dao::load_pool_batches(
            &db,
            "reco",
            "2026-01-01",
            "2026-12-31",
            Some("2026-09-04"),
        )
        .await
        .unwrap();
        assert_eq!(batches.len(), 1);
        assert_eq!(batches[0].generated_at, at);
        assert_eq!(batches[0].purpose.as_deref(), Some(purpose::MOVER_REPLAY));

        let audits = reco_scan_audit::Entity::find().all(&db).await.unwrap();
        assert_eq!(audits.len(), 1);
        assert_eq!(audits[0].generated_at, at);
        assert_eq!(audits[0].as_of_date.as_deref(), Some("2026-09-04"));
        assert_eq!(audits[0].purpose.as_deref(), Some(purpose::MOVER_REPLAY));
    }

    /// 前视门（A7 第四条）：as-of 回放的 picks / 留痕**不得**进入 live 归因。
    ///
    /// 两侧都有电：把 `None` 谓词去掉，live 查询会读到回放行（命中 2 条）；
    /// 把 `Some(day)` 谓词去掉，回放查询会读到实盘行。`load_picks_in_range` 与
    /// `load_trim_evidence` 共用同一条代际谓词，两处都要查。
    #[tokio::test]
    async fn asof_rows_are_invisible_to_live_attribution() {
        let db = axagent_dao::db::create_test_pool().await.expect("测试库应可建立").conn;
        insert_batch(&db, "2026-10-24T10:00:00.000", None, purpose::LIVE, "600000").await;
        insert_batch(
            &db,
            "2026-10-24T10:00:01.000",
            Some("2026-09-04"),
            purpose::MOVER_REPLAY,
            "002696",
        )
        .await;

        let live = load_picks_in_range(&db, "2026-01-01", "2026-12-31", None).await.unwrap();
        assert_eq!(live.len(), 1, "live 归因只许看到实盘行");
        assert_eq!(live[0].stock_code, "600000");

        let replay =
            load_picks_in_range(&db, "2026-01-01", "2026-12-31", Some("2026-09-04")).await.unwrap();
        assert_eq!(replay.len(), 1, "回放归因只许看到同代行");
        assert_eq!(replay[0].stock_code, "002696");

        let live_trim = load_trim_evidence(&db, "2026-01-01", "2026-12-31", None).await.unwrap();
        assert!(live_trim.contains_key(&("mid".to_string(), "600000".to_string())));
        assert!(
            !live_trim.contains_key(&("mid".to_string(), "002696".to_string())),
            "回放留痕混进 live 归因 ⇒ 会把回放的截断算成实盘证据"
        );
    }
}

/// **as-of 漏检信号**（阶段 C-1，2026-10-24）：从最新一轮荐股链核查的 as-of 差异
/// 装配逐格证据，供闭环权重消费。
///
/// 与 live 版 [`load_mover_cell_signal`] 的差别只有证据来源：
/// - `hits` = 该事件档的 as-of 出票风格（差异 JSON 的 `pickedStyles`，写侧已在回放时
///   过滤 `synthetic`）；
/// - `scored_out` = as-of 截断风格（`trimmedStyles`，源头是 as-of `reco_scan_audit`）。
///
/// 口径：只读**最新一轮 `chain='reco'` 核查**里已回填 as-of 的事件（`asof_layer` 非空）；
/// 旧轮的 as-of 证据不参与 —— 否则同一批证据每轮重复收费（与 C4 复利修复同族的坑）。
pub async fn load_mover_cell_signal_asof(
    db: &sea_orm::DatabaseConnection,
) -> Result<crate::recommender::reco_loop::MoverCellSignal, String> {
    use crate::recommender::reco_loop::MoverCellSignal;
    use axagent_entities::{mover_recall_events, mover_recall_runs};
    use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QueryOrder};

    let mut signal = MoverCellSignal::default();
    let Some(run) = mover_recall_runs::Entity::find()
        .filter(mover_recall_runs::Column::Chain.eq("reco"))
        .order_by_desc(mover_recall_runs::Column::CreatedAtMs)
        .one(db)
        .await
        .map_err(|e| format!("读最新漏检轮次失败: {e}"))?
    else {
        return Ok(signal);
    };
    let rows = mover_recall_events::Entity::find()
        .filter(mover_recall_events::Column::RunId.eq(run.id))
        .filter(mover_recall_events::Column::Chain.eq("reco"))
        .filter(mover_recall_events::Column::AsofLayer.is_not_null())
        .all(db)
        .await
        .map_err(|e| format!("读 as-of 事件失败: {e}"))?;

    for row in &rows {
        let Some(diff) = row
            .asof_diff_json
            .as_deref()
            .and_then(|s| serde_json::from_str::<serde_json::Value>(s).ok())
        else {
            continue;
        };
        let period = row.period.clone();
        if let Some(arr) = diff.get("pickedStyles").and_then(|v| v.as_array()) {
            let mut styles: Vec<String> = arr
                .iter()
                .filter_map(|v| v.as_str())
                .map(|s| normalize_style_key(s).to_string())
                .collect();
            styles.sort();
            styles.dedup();
            for style in styles {
                signal.cells.entry((style, period.clone())).or_default().1 += 1;
            }
        }
        if let Some(arr) = diff.get("trimmedStyles").and_then(|v| v.as_array()) {
            let mut styles: Vec<String> = arr
                .iter()
                .filter_map(|v| v.as_str())
                .map(|s| normalize_style_key(s).to_string())
                .collect();
            styles.sort();
            styles.dedup();
            for style in styles {
                signal.cells.entry((style, period.clone())).or_default().0 += 1;
            }
        }
    }
    Ok(signal)
}

/// 闭环权重实际消费的 mover 线（阶段 C-2「不双罚」的唯一入口）。
///
/// 优先 as-of 线；as-of 线缺席（还没有任何事件跑过回放）时才回退 live 线。
/// **绝不相加**：同一笔漏检在两条线上各计一次 = 同一证据罚两遍。
pub async fn load_mover_cell_signal_preferring_asof(
    db: &sea_orm::DatabaseConnection,
    vars: &[(String, serde_json::Value)],
    as_of_date: Option<&str>,
) -> Result<crate::recommender::reco_loop::MoverCellSignal, String> {
    let asof = load_mover_cell_signal_asof(db).await?;
    if !asof.cells.is_empty() {
        return Ok(asof);
    }
    tracing::info!("[mover_recall] as-of 信号缺席（尚无事件跑过回放）⇒ 权重环回退 live mover 线");
    load_mover_cell_signal(db, vars, as_of_date).await
}

#[cfg(test)]
mod pool_membership_tristate_tests {
    use super::*;

    /// F-1：单对象池快照只证明它自己那只票；别的票是**无从判定**，不是「不在池」。
    /// 现网 21/21 智选事件的 not_in_pool 就是这条判错出来的伪证据。
    #[test]
    fn single_object_snapshot_only_proves_its_own_code() {
        let obj = r#"{"stockCode":"002696","stockName":"百洋股份"}"#;
        assert_eq!(code_in_pool_snapshot_opt(Some(obj), "002696"), Some(true));
        assert_eq!(
            code_in_pool_snapshot_opt(Some(obj), "600000"),
            None,
            "单对象对别的票只能是未知"
        );
        // 数组是真正的池快照：查不到 = 确定的 false
        assert_eq!(
            code_in_pool_snapshot_opt(Some(r#"[["600000","浦发银行"]]"#), "002696"),
            Some(false)
        );
        assert_eq!(code_in_pool_snapshot_opt(None, "002696"), None);
        assert_eq!(code_in_pool_snapshot_opt(Some("not json"), "002696"), None);
        // bool 包装保持旧行为（其余调用方不因本次改造改变语义）
        assert!(!code_in_pool_snapshot(Some(obj), "600000"));
    }
}
