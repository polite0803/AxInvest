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

use std::collections::{HashMap, HashSet};

use sea_orm::{ColumnTrait, EntityTrait, PaginatorTrait, QueryFilter, QueryOrder};
use serde::Serialize;

use axagent_entities::{market_daily_close, market_stock_universe, reco_picks};

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
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MoverEvent {
    pub stock_code: String,
    pub stock_name: String,
    /// 档位（`ultra_short` | `short` | `mid` | `long`）
    pub period: String,
    /// 窗口右端（= 达标确认日）ISO 日期
    pub anchor_date: String,
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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MissLayer {
    /// 该票该档已被推荐 ⇒ 命中，不入漏检分母
    Recommended,
    /// 推荐时点晚于窗口起点 ⇒「晚推荐」，不计漏检
    LateTiming,
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
    pub fn as_str(self) -> &'static str {
        match self {
            MissLayer::Recommended => "recommended",
            MissLayer::LateTiming => "late_timing",
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
    /// 该票是否出现在本轮任一候选池快照
    pub in_pool: bool,
    /// 本轮候选池是否降级（空/取数失败）
    pub pool_degraded: bool,
    /// 该 (风格, 档位) 是否成立（`style_matrix::is_active` 口径）
    pub matrix_active: bool,
    /// 窗口内**有截断留痕**的风格键（`reco_scan_audit` 命中，矩阵名目归一后的写法）——
    /// 非空即「该格算出过它却被 top-N 淘汰」，可归因、可降权
    pub l3_scored_out_styles: Vec<String>,
}

/// 纯函数归因：顺序即优先级，先排掉「其实不是漏检」，再定位到可判的层。
///
/// `L3ScoredOut` 排在 `OtherPeriod` 之后：别的档位已经出过票时，「本档漏」的
/// 结论会被系统性高估（同一只票本就不必四档齐发）。
pub fn attribute(layer_period_ok: bool, ev: &Evidence) -> MissLayer {
    if !ev.picks_for_period.is_empty() {
        return MissLayer::Recommended;
    }
    if !layer_period_ok || !ev.matrix_active {
        return MissLayer::ByDesign;
    }
    if ev.pool_degraded {
        return MissLayer::PoolDegraded;
    }
    if !ev.in_pool {
        return MissLayer::NotInPool;
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
    let in_pool = get(MissLayer::L3ScoredOut)
        + get(MissLayer::PoolDegraded)
        + get(MissLayer::OtherPeriod)
        + get(MissLayer::Unexplained)
        + recommended;
    let not_in_pool = get(MissLayer::NotInPool);
    let misses = get(MissLayer::NotInPool)
        + get(MissLayer::PoolDegraded)
        + get(MissLayer::L3ScoredOut)
        + get(MissLayer::OtherPeriod)
        + get(MissLayer::Unexplained);
    let reachability = if events == 0 {
        f64::NAN
    } else {
        in_pool as f64 * 100.0 / events as f64
    };
    // 覆盖率分母 = 入池事件（含降级桶）：与 `in_pool` 同口径，不另立一套
    let coverage = if in_pool == 0 {
        f64::NAN
    } else {
        recommended as f64 * 100.0
            / (recommended
                + get(MissLayer::L3ScoredOut)
                + get(MissLayer::OtherPeriod)
                + get(MissLayer::Unexplained)) as f64
    };
    let unexplained_share = if misses == 0 {
        f64::NAN
    } else {
        get(MissLayer::Unexplained) as f64 * 100.0 / misses as f64
    };
    let _ = not_in_pool;
    RecallRates { reachability, coverage, unexplained_share, events, misses }
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

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LayerRow {
    pub layer: MissLayer,
    pub count: usize,
    /// 按板块分组的小计，避免混池假象
    pub by_market_type: Vec<(String, usize)>,
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

    let trims = load_trim_evidence(db, &from, &to).await?;
    let picks = load_picks_in_range(db, &from, &to).await?;

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
        for ev in events_for_tier(&by_stock, rule, &anchor_from, &to) {
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
pub async fn load_close_dates(db: &sea_orm::DatabaseConnection) -> Result<Vec<String>, String> {
    let rows = market_daily_close::Entity::find()
        .order_by_asc(market_daily_close::Column::TradeDate)
        .all(db)
        .await
        .map_err(|e| format!("读快照日期失败: {e}"))?;
    let mut seen: Vec<String> = Vec::new();
    let mut set = HashSet::new();
    for r in rows {
        if set.insert(r.trade_date.clone()) {
            seen.push(r.trade_date);
        }
    }
    Ok(seen)
}

/// 某档在 `[start, end]` 内的达标事件（纯函数，便于单测直接喂构造数据）。
///
/// `closes` 必须是**日期升序**的连续交易日序列；窗口取「anchor 当日及其前 window_days−1 天」。
pub fn events_for_tier(
    closes: &CloseSeriesByCode,
    rule: &TierRule,
    anchor_from: &str,
    anchor_to: &str,
) -> Vec<MoverEvent> {
    let mut out = Vec::new();
    for (code, (name, series)) in closes {
        if series.len() < rule.window_days as usize {
            // 该票历史长度不足 ⇒ 窗口算不出来，显式跳过（不当 0 处理）
            continue;
        }
        for (pos, (date, _)) in series.iter().enumerate() {
            if date.as_str() < anchor_from || date.as_str() > anchor_to {
                continue;
            }
            if pos + 1 < rule.window_days as usize {
                continue;
            }
            let win: Vec<f64> =
                series[pos + 1 - rule.window_days as usize..=pos].iter().map(|(_, r)| *r).collect();
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
) -> Result<HashMap<(String, String), Vec<String>>, String> {
    use axagent_entities::reco_scan_audit;
    let rows = reco_scan_audit::Entity::find()
        .filter(reco_scan_audit::Column::GeneratedAt.gte(from))
        .filter(reco_scan_audit::Column::GeneratedAt.lte(to))
        .all(db)
        .await
        .map_err(|e| format!("读 reco_scan_audit 失败: {e}"))?;
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
    let Some(raw) = pool_json else { return false };
    let Ok(parsed) = serde_json::from_str::<Vec<serde_json::Value>>(raw) else {
        return false;
    };
    parsed.iter().any(|item| match item {
        serde_json::Value::Array(a) => a.first().and_then(|v| v.as_str()) == Some(code),
        serde_json::Value::Object(o) => {
            o.get("stockCode").and_then(|v| v.as_str()) == Some(code)
                || o.get("code").and_then(|v| v.as_str()) == Some(code)
        },
        _ => false,
    })
}

/// 读某档在时间区间内的全部推荐（含候选池快照），供归因用。
pub async fn load_picks_in_range(
    db: &sea_orm::DatabaseConnection,
    from: &str,
    to: &str,
) -> Result<Vec<reco_picks::Model>, String> {
    reco_picks::Entity::find()
        .filter(reco_picks::Column::GeneratedAt.gte(from.to_string()))
        .filter(reco_picks::Column::GeneratedAt.lte(format!("{to}T23:59:59")))
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

    #[test]
    fn event_requires_full_window_length() {
        let closes =
            closes_map(vec![("600059".into(), vec![("2026-09-28", 6.0), ("2026-09-29", 6.0)])]);
        // 窗口 3 天但只有 2 行 ⇒ 算不出，必须显式跳过（不得按 2 天判达标）
        let ev =
            events_for_tier(&closes, &rule(Period::Short, 10.0, 3), "2026-09-01", "2026-12-31");
        assert!(ev.is_empty());
        let ev2 =
            events_for_tier(&closes, &rule(Period::Short, 10.0, 2), "2026-09-01", "2026-12-31");
        assert_eq!(ev2.len(), 1, "窗口 2 天时应在 09-29 出事件");
        assert!((ev2[0].cum_gain_pct - 12.36).abs() < 1e-6);
        assert!((ev2[0].max_daily_pct - 6.0).abs() < 1e-6);
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

    /// 默认「该档确实出票」，各用例只改自己要判的那一件事
    fn ev(mutate: impl FnOnce(&mut Evidence)) -> Evidence {
        let mut e = Evidence { matrix_active: true, ..Default::default() };
        mutate(&mut e);
        e
    }

    #[test]
    fn attribution_priority_is_mechanical() {
        assert_eq!(
            attribute(true, &ev(|e| e.picks_for_period.push("2026-09-20T15:00:00".into()))),
            MissLayer::Recommended
        );
        // 按设计不出票 ⇒ 优先于池层（严禁进待优化清单）
        assert_eq!(attribute(true, &ev(|e| e.matrix_active = false)), MissLayer::ByDesign);
        assert_eq!(attribute(true, &ev(|e| e.pool_degraded = true)), MissLayer::PoolDegraded);
        assert_eq!(attribute(true, &ev(|_| {})), MissLayer::NotInPool);
        assert_eq!(
            attribute(
                true,
                &ev(|e| {
                    e.in_pool = true;
                    e.picked_in_other_period = true;
                })
            ),
            MissLayer::OtherPeriod
        );
        // 入池但无截断留痕 ⇒ 判不出（策略内部否决不落痕），进兜底桶
        assert_eq!(attribute(true, &ev(|e| e.in_pool = true)), MissLayer::Unexplained);
        // 入池 + 截断留痕 ⇒ 唯一可归因到具体风格、可据此降权的一层
        assert_eq!(
            attribute(
                true,
                &ev(|e| {
                    e.in_pool = true;
                    e.l3_scored_out_styles.push("trend".into());
                })
            ),
            MissLayer::L3ScoredOut
        );
        assert!(!MissLayer::ByDesign.counts_as_miss());
        assert!(MissLayer::L3ScoredOut.counts_as_miss());
        assert!(MissLayer::Unexplained.counts_as_miss());
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
}
