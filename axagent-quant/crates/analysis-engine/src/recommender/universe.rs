//! 全市场候选快照 → 按策略分桶（W3，2026-11-01）。
//!
//! # 解决什么
//!
//! 旧建池只用「热榜 50 + 行业龙头 30 + 硬编码 48 + 涨幅榜 80」约 190 只（全市场 ~3.5%），
//! 且热榜/行业/涨幅榜全部是「已经涨了」的票 ⇒ 策略只能在同一批强势股里做确认型排序。
//! 本模块把 `market_daily_close` 的全市场收盘快照（现网 ~5,570 只）压成每策略一桶：
//!
//! - `trend`：20 日相对强度为正且靠前；
//! - `reversion`：20 日跌幅靠前（超跌）；
//! - `capital`：最近 5 日成交额放大倍数靠前；
//! - `value`：盈利收益率（1/PE）与账面收益率（1/PB）的**截面百分位**均值靠前；
//! - `watchlist` / `serenity`：仍用基础池（watchlist 走用户标的；serenity 走工作流候选缓存）。
//!
//! # 价值桶为什么必须带覆盖率判据
//!
//! 前两桶的特征每只票都有（收盘、成交额），价值桶的两列是按日采集的可选字段 ⇒
//! 「有多少票带估值」决定了百分位是不是全市场口径。样本只剩几个百分点时，百分位算的是
//! 「那几只内部谁更贵」，与全市场无关，结果会稳定选出同一小撮票。低于
//! [`VALUE_MIN_COVERAGE`] 就不建桶，并把缺席理由交回调用方声明（不静默回落）。
//!
//! ⚠ 价值桶**不读** `financial_snapshots`：那张表只在单票分析时写（现网最新一天 1 行、
//! 最好的一天 290 行，且全是分析过的票），用它建桶等于「在我们已经看过的票里挑便宜的」=
//! 自指样本。本桶只读 `market_daily_close.pe_ttm/pb`（每日全市场收盘快照同一行带来）。
//!
//! # 为什么是「按策略分桶」而不是「一张更大的池」
//!
//! 同一张池子让四个策略互相稀释：趋势需要的强势票、反弹需要的弱势票、资金需要的放量票
//! 同时存在时，每个策略都要为「不属于自己的票」付出 K 线取数成本。分桶后每个策略只扫
//! 与自身假设一致的候选，总扫描量与旧实现同量级（旧实现四个策略各扫同一张 200 只）。
//!
//! # 边界
//!
//! - 只读 DB 快照，不增加网络请求；详细 K 线仍由各策略按需取。
//! - 快照不足 21 根 / 成交额 < 1 亿 / ST / 价格非法 ⇒ 不入桶（缺席显式，不凑数）。
//! - 分桶只决定「谁进入详细扫描」，不改变策略自身的判据；策略仍然可以全部拒绝。
//! - as-of 由调用方按截止日取快照，本模块不读时钟。

use super::pool::SeedItem;
use super::types::Style;
use std::collections::{BTreeMap, HashMap, HashSet};

/// 一根收盘快照（`market_daily_close` 的窄投影；不把整表 Model 带进 consumer crate）。
#[derive(Debug, Clone, PartialEq)]
pub struct CloseRow {
    pub code: String,
    pub name: String,
    /// `YYYY-MM-DD`（该列现网是带连字符形态；字典序即时间序）
    pub trade_date: String,
    pub close: f64,
    pub amount: f64,
    pub change_pct: f64,
}

/// 全市场单只票的廉价特征。
#[derive(Debug, Clone, PartialEq)]
pub struct UniverseItem {
    pub code: String,
    pub name: String,
    pub industry: Option<String>,
    pub close: f64,
    pub change_pct: f64,
    pub amount: f64,
    /// 近 20 个交易日收益（%）
    pub ret_20d: f64,
    /// 近 20 个交易日最大回撤（%，≤0）
    pub drawdown_20d: f64,
    /// 最新成交额 / 近 5 日均额（1.0 = 持平）
    pub amount_ratio_5d: f64,
    /// 市盈率（TTM）；`None` = 该日未采集（不是「没有估值」）。价值桶只用 **>0** 的值。
    pub pe: Option<f64>,
    /// 市净率；`None` 同上。
    pub pb: Option<f64>,
}

/// 每策略保留的详细扫描上限。用户标的与涨幅榜另计（它们必须被看到）。
pub const UNIVERSE_CAP_PER_STYLE: usize = 80;

/// 硬过滤门槛：成交额 1 亿（与 `pool.rs` 的流动性门同口径）。
pub const MIN_AMOUNT: f64 = 100_000_000.0;

/// 价值桶的最低估值覆盖率：**带 PE 或 PB 的票**至少要有快照的一半。
///
/// 取值依据：百分位排序的可解释前提是「样本能代表总体」。2026-11-01 现网只有全市场
/// 收盘快照带不上估值列（该列本轮新增），回放窗口更是 0 覆盖 ⇒ 判据必须能真的拦住，
/// 而不是留一个恒真的下限。0.5 是「截面还说得过半市场」的最低线。
pub const VALUE_MIN_COVERAGE: f64 = 0.5;

/// 全市场收盘快照合并估值后的产物：`code → (pe_ttm, pb)`，只含**至少一列非空**的票。
pub type Valuations = HashMap<String, (Option<f64>, Option<f64>)>;

/// 把收盘历史压成全市场特征表。
///
/// 输入行可以乱序；按 code 分组后按日期升序。不足 21 根、ST、成交额不达标、价格非法
/// 的票直接剔除（不补零、不缩水计算 —— 20 日收益必须真的是 20 日）。
///
/// `valuations` 只取**截止日那一行**的估值（由调用方按同一代际日取好）：本函数不做
/// 「用最近一个有值的日期」这种回填 —— 那会把后一天的估值贴到前一天的候选上（前视）。
pub fn build_universe(
    rows: &[CloseRow],
    industries: &HashMap<String, String>,
    valuations: &Valuations,
) -> Vec<UniverseItem> {
    let mut by_code: BTreeMap<&str, Vec<&CloseRow>> = BTreeMap::new();
    for r in rows {
        by_code.entry(r.code.as_str()).or_default().push(r);
    }
    let mut out = Vec::with_capacity(by_code.len());
    for (code, mut bars) in by_code {
        bars.sort_by(|a, b| a.trade_date.cmp(&b.trade_date));
        if bars.len() < 21 {
            continue;
        }
        let latest = bars[bars.len() - 1];
        if !latest.close.is_finite()
            || latest.close <= 0.0
            || !latest.amount.is_finite()
            || latest.amount < MIN_AMOUNT
            || latest.name.contains("ST")
            || latest.name.contains("st")
        {
            continue;
        }
        let closes: Vec<f64> = bars.iter().map(|b| b.close).filter(|c| c.is_finite()).collect();
        if closes.len() < 21 {
            continue;
        }
        let window = &closes[closes.len() - 21..];
        let base = window[0];
        if base <= 0.0 {
            continue;
        }
        let ret_20d = (window[window.len() - 1] / base - 1.0) * 100.0;
        if !ret_20d.is_finite() {
            continue;
        }
        let mut peak = window[0];
        let mut max_dd = 0.0_f64;
        for &c in window {
            if c > peak {
                peak = c;
            }
            if peak > 0.0 {
                let dd = (c / peak - 1.0) * 100.0;
                if dd < max_dd {
                    max_dd = dd;
                }
            }
        }
        // 近 5 日必须**五天都有成交额**才计算放大倍数：老数据的 amount 列可能为 0，
        // 只对非零项取均值会把「3 天有量」算成「5 天均量」并放大倍数（假信号）。
        let last5: Vec<f64> =
            bars[bars.len().saturating_sub(5)..].iter().map(|b| b.amount).collect();
        let amount_ratio_5d = if last5.len() == 5 && last5.iter().all(|a| a.is_finite() && *a > 0.0)
        {
            latest.amount / (last5.iter().sum::<f64>() / 5.0)
        } else {
            0.0
        };
        out.push(UniverseItem {
            code: code.to_string(),
            name: latest.name.clone(),
            industry: industries.get(code).cloned().filter(|s| !s.is_empty()),
            close: latest.close,
            change_pct: latest.change_pct,
            amount: latest.amount,
            ret_20d,
            drawdown_20d: max_dd,
            amount_ratio_5d,
            pe: valuations.get(code).and_then(|(pe, _)| *pe),
            pb: valuations.get(code).and_then(|(_, pb)| *pb),
        });
    }
    out
}

/// 分桶产物。`value` 与其余三桶分开返回，因为它的可用性是一个**独立结论**：
/// 覆盖率不达标时必须让调用方拿得到「没建桶」这件事，否则它会静默回落基础池，
/// 面板上看起来像「价值桶跑过了」。
#[derive(Debug, Clone)]
pub struct StrategySeeds {
    pub by_style: HashMap<Style, Vec<SeedItem>>,
    /// 快照为空 ⇒ `None`（连总体都没有，谈不上覆盖率）；否则 Built / Degraded 二选一。
    pub value: Option<ValueBucket>,
}

#[derive(Debug, Clone)]
pub enum ValueBucket {
    /// 覆盖率达标：桶内按价值分降序，并已前插用户标的与涨幅榜。
    Built { seeds: Vec<SeedItem>, valued: usize, universe_size: usize },
    /// 覆盖率不达标：**不建桶**。调用方回落基础池并原样声明 [`Self::degraded_reason`]。
    Degraded { valued: usize, universe_size: usize },
}

impl ValueBucket {
    /// 缺席声明文本；`Built` 时为 `None`。
    pub fn degraded_reason(&self) -> Option<String> {
        match self {
            Self::Built { .. } => None,
            Self::Degraded { valued, universe_size } => {
                let pct = *valued as f64 / *universe_size as f64 * 100.0;
                Some(format!(
                    "全市场估值覆盖率 {valued}/{universe_size}（{pct:.1}%）低于门限 {:.0}% ⇒ \
                     价值桶未建（百分位样本不足以代表截面），本档回落基础池扫描。 \
                     结构性边界：PE/PB 两列自 2026-11-01 起才随每日收盘快照采集，\
                     早于该日的历史行是 NULL，回放那几天没有价值面可用。",
                    VALUE_MIN_COVERAGE * 100.0
                ))
            },
        }
    }
}

/// 按策略分桶：`trend` / `reversion` / `capital` / `value` 各一桶。
///
/// 用户标的与涨幅榜**前插**到每个桶：它们来自用户意图/当日事实，不能因为没进前 80 就消失。
/// 去重按 code；桶内顺序 = 用户 → 涨幅榜 → 排名（策略扫描顺序不影响判据，但保序便于留痕）。
pub fn build_strategy_seeds(
    universe: &[UniverseItem],
    user_seed: &[SeedItem],
    mover_seed: &[SeedItem],
) -> StrategySeeds {
    let mut by_style = HashMap::new();
    if universe.is_empty() {
        return StrategySeeds { by_style, value: None };
    }
    for style in [Style::Trend, Style::Reversion, Style::Capital] {
        by_style.insert(style, merge_front(user_seed, mover_seed, ranked(universe, style)));
    }
    let value = build_value_bucket(universe, user_seed, mover_seed);
    if let ValueBucket::Built { seeds, .. } = &value {
        by_style.insert(Style::Value, seeds.clone());
    }
    StrategySeeds { by_style, value: Some(value) }
}

/// 单票的可收益率：`(1/PE, 1/PB)`，两列都不可用 ⇒ `None`（不参与价值截面）。
///
/// 只取 **>0** 的值：PE≤0 是亏损票，它不是「便宜」，也不该在盈利收益率里占一个名次；
/// 但只要 PB>0，它仍按账面收益率参与价值截面 —— 这是价值策略的常规口径，不是放水。
fn yields(item: &UniverseItem) -> Option<(Option<f64>, Option<f64>)> {
    let earnings = item.pe.filter(|v| v.is_finite() && *v > 0.0).map(|v| 1.0 / v);
    let book = item.pb.filter(|v| v.is_finite() && *v > 0.0).map(|v| 1.0 / v);
    earnings.or(book).map(|_| (earnings, book))
}

/// 截面百分位（0 = 最不便宜，1 = 最便宜）。同值取中位秩；只有一个样本时记 0.5
/// （单样本没有截面信息，判它「最便宜」是把无知当结论）。
fn percentile_ranks(values: &[(String, f64)]) -> HashMap<String, f64> {
    let mut sorted = values.to_vec();
    sorted.sort_by(|a, b| a.1.total_cmp(&b.1).then_with(|| a.0.cmp(&b.0)));
    let n = sorted.len();
    let mut out = HashMap::with_capacity(n);
    let mut i = 0;
    while i < n {
        let mut j = i;
        while j + 1 < n && sorted[j + 1].1 == sorted[i].1 {
            j += 1;
        }
        // 中位秩用**浮点**除法：`(i+j)/2` 的整数除法会让偶数长度的并列组整体偏低
        // （下侧组丢 0.5、上侧组也丢 0.5 ⇒ 同样的并列在分布两端不对称）。
        let mid = (i + j) as f64 / 2.0;
        let pct = if n == 1 { 0.5 } else { mid / (n - 1) as f64 };
        for item in &sorted[i..=j] {
            out.insert(item.0.clone(), pct);
        }
        i = j + 1;
    }
    out
}

/// 价值桶：先判覆盖率，再按「盈利收益率百分位」与「账面收益率百分位」的均值降序取前 N。
///
/// 两个收益率各排一次截面再平均，而不是把 PE、PB 混成一个数：两者量纲不同（一个含盈利、
/// 一个只含净资产），且缺 PE 的票（亏损）恰恰应该只按 PB 排名。
///
/// ⚠ 两次截面的**样本域不同**（盈利收益率只含 PE>0 的票，账面收益率只含 PB>0 的票），
/// 所以两个百分位不是同一把尺。只有 PB 的票拿到的是「它在亏损票里相对多便宜」，与同时有两列
/// 的票的均值不同域 —— 这是截面平均的已知代价，不是缺陷：桶只决定谁进入详细扫描，
/// 策略侧仍用自己的 quote 判据 fail-closed（`strategies/value.rs` 取不到估值就不出票）。
fn build_value_bucket(
    universe: &[UniverseItem],
    user_seed: &[SeedItem],
    mover_seed: &[SeedItem],
) -> ValueBucket {
    let mut earnings: Vec<(String, f64)> = Vec::new();
    let mut book: Vec<(String, f64)> = Vec::new();
    let mut usable: Vec<&UniverseItem> = Vec::new();
    for item in universe {
        if let Some((e, b)) = yields(item) {
            usable.push(item);
            if let Some(e) = e {
                earnings.push((item.code.clone(), e));
            }
            if let Some(b) = b {
                book.push((item.code.clone(), b));
            }
        }
    }
    let universe_size = universe.len();
    if (usable.len() as f64) < VALUE_MIN_COVERAGE * universe_size as f64 {
        return ValueBucket::Degraded { valued: usable.len(), universe_size };
    }
    let pe_pct = percentile_ranks(&earnings);
    let pb_pct = percentile_ranks(&book);
    let mut scored: Vec<(f64, &UniverseItem)> = usable
        .iter()
        .map(|item| {
            let score = match (pe_pct.get(&item.code), pb_pct.get(&item.code)) {
                (Some(e), Some(b)) => (e + b) / 2.0,
                (Some(v), None) | (None, Some(v)) => *v,
                (None, None) => 0.0,
            };
            (score, *item)
        })
        .collect();
    scored.sort_by(|a, b| b.0.total_cmp(&a.0).then_with(|| a.1.code.cmp(&b.1.code)));
    let ranked: Vec<SeedItem> = scored
        .into_iter()
        .take(UNIVERSE_CAP_PER_STYLE)
        .map(|(_, item)| (item.code.clone(), item.name.clone(), item.industry.clone()))
        .collect();
    ValueBucket::Built {
        seeds: merge_front(user_seed, mover_seed, ranked),
        valued: usable.len(),
        universe_size,
    }
}

/// 单桶排名。判据只做「便宜特征」：趋势看 20 日强度、反弹看 20 日跌幅、资金看量能放大。
fn ranked(universe: &[UniverseItem], style: Style) -> Vec<SeedItem> {
    let mut picked: Vec<&UniverseItem> = universe
        .iter()
        .filter(|i| match style {
            Style::Trend => i.ret_20d.is_finite() && i.ret_20d > 0.0,
            Style::Reversion => i.ret_20d.is_finite() && i.ret_20d < 0.0,
            Style::Capital => i.amount_ratio_5d.is_finite() && i.amount_ratio_5d > 1.2,
            _ => false,
        })
        .collect();
    match style {
        Style::Trend => picked.sort_by(|a, b| {
            b.ret_20d
                .partial_cmp(&a.ret_20d)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then(a.code.cmp(&b.code))
        }),
        Style::Reversion => picked.sort_by(|a, b| {
            a.ret_20d
                .partial_cmp(&b.ret_20d)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then(a.code.cmp(&b.code))
        }),
        Style::Capital => picked.sort_by(|a, b| {
            b.amount_ratio_5d
                .partial_cmp(&a.amount_ratio_5d)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then(a.code.cmp(&b.code))
        }),
        _ => {},
    }
    picked
        .into_iter()
        .take(UNIVERSE_CAP_PER_STYLE)
        .map(|i| (i.code.clone(), i.name.clone(), i.industry.clone()))
        .collect()
}

fn merge_front(user: &[SeedItem], mover: &[SeedItem], ranked: Vec<SeedItem>) -> Vec<SeedItem> {
    let mut seen = HashSet::new();
    let mut out = Vec::with_capacity(user.len() + mover.len() + ranked.len());
    for item in user.iter().chain(mover.iter()).chain(ranked.iter()) {
        if seen.insert(item.0.clone()) {
            out.push(item.clone());
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rows(code: &str, name: &str, closes: &[f64], amounts: &[f64]) -> Vec<CloseRow> {
        closes
            .iter()
            .zip(amounts.iter())
            .enumerate()
            .map(|(i, (c, a))| CloseRow {
                code: code.into(),
                name: name.into(),
                trade_date: format!("2026-09-{:02}", i + 1),
                close: *c,
                amount: *a,
                change_pct: 0.0,
            })
            .collect()
    }

    fn flat(n: usize, close: f64, amount: f64) -> (Vec<f64>, Vec<f64>) {
        (vec![close; n], vec![amount; n])
    }

    /// 直接造特征行（跳过 `build_universe`）用的最小构造器。
    fn item(
        code: &str,
        ret_20d: f64,
        amount_ratio_5d: f64,
        pe: Option<f64>,
        pb: Option<f64>,
    ) -> UniverseItem {
        UniverseItem {
            code: code.into(),
            name: format!("N{code}"),
            industry: None,
            close: 10.0,
            change_pct: 0.0,
            amount: 200_000_000.0,
            ret_20d,
            drawdown_20d: 0.0,
            amount_ratio_5d,
            pe,
            pb,
        }
    }

    #[test]
    fn build_universe_computes_features_and_filters() {
        let (mut closes, mut amounts) = flat(21, 10.0, 200_000_000.0);
        closes[0] = 8.0; // 20 日 +25%
        closes[10] = 12.0; // 回撤 (10-12)/12 ≈ -16.7%
        // 近 5 日均额 200M、最新 300M ⇒ ratio 1.5
        for a in amounts.iter_mut().skip(16).take(4) {
            *a = 175_000_000.0;
        }
        amounts[20] = 300_000_000.0;
        let mut all = rows("600001", "甲", &closes, &amounts);
        // ST / 成交额不足 / 不足 21 根：都必须剔除
        let (c2, a2) = flat(21, 10.0, 200_000_000.0);
        all.extend(rows("600002", "*ST乙", &c2, &a2));
        let (c3, mut a3) = flat(21, 10.0, 200_000_000.0);
        a3[20] = 1.0;
        all.extend(rows("600003", "丙", &c3, &a3));
        let (c4, a4) = flat(10, 10.0, 200_000_000.0);
        all.extend(rows("600004", "丁", &c4, &a4));
        let mut industries = HashMap::new();
        industries.insert("600001".to_string(), "半导体".to_string());
        let mut valuations = HashMap::new();
        valuations.insert("600001".to_string(), (Some(18.0), Some(2.5)));
        valuations.insert("600002".to_string(), (Some(9.0), Some(1.0)));
        let got = build_universe(&all, &industries, &valuations);
        assert_eq!(got.len(), 1, "只应保留 600001: {got:?}");
        let it = &got[0];
        assert!((it.ret_20d - 25.0).abs() < 1e-9, "ret_20d={}", it.ret_20d);
        assert!(it.drawdown_20d < -16.0, "drawdown_20d={}", it.drawdown_20d);
        assert!((it.amount_ratio_5d - 1.5).abs() < 1e-9, "ratio={}", it.amount_ratio_5d);
        assert_eq!(it.industry.as_deref(), Some("半导体"));
        assert_eq!(it.pe, Some(18.0), "估值按 code 合并进快照");
        assert_eq!(it.pb, Some(2.5));
    }

    /// 估值表里没有的票 ⇒ `None`，不得填 0 或借别的票的值。
    #[test]
    fn build_universe_leaves_missing_valuations_absent() {
        let (closes, amounts) = flat(21, 10.0, 200_000_000.0);
        let all = rows("600001", "甲", &closes, &amounts);
        let mut valuations = HashMap::new();
        // 同一天的另一只票有估值，但 600001 没有
        valuations.insert("600009".to_string(), (Some(10.0), Some(1.0)));
        let got = build_universe(&all, &HashMap::new(), &valuations);
        assert_eq!(got.len(), 1);
        assert_eq!((got[0].pe, got[0].pb), (None, None), "缺席必须可分辨");
    }

    #[test]
    fn strategy_buckets_rank_and_include_user_and_mover() {
        let mut items = Vec::new();
        for (i, ret) in [10.0, 30.0, -15.0, -5.0].iter().enumerate() {
            items.push(item(
                &format!("60000{i}"),
                *ret,
                if i == 0 { 2.0 } else { 1.0 },
                None,
                None,
            ));
        }
        let user = vec![("600009".to_string(), "用户".to_string(), None)];
        let mover = vec![("600008".to_string(), "涨幅".to_string(), None)];
        let seeds = build_strategy_seeds(&items, &user, &mover);
        let map = &seeds.by_style;
        let trend: Vec<&str> = map[&Style::Trend].iter().map(|x| x.0.as_str()).collect();
        assert_eq!(trend[0], "600009", "用户标的前插");
        assert_eq!(trend[1], "600008", "涨幅榜第二");
        assert_eq!(trend[2], "600001", "20 日最强");
        let rev: Vec<&str> = map[&Style::Reversion].iter().map(|x| x.0.as_str()).collect();
        assert_eq!(rev[2], "600002", "20 日最弱");
        let cap: Vec<&str> = map[&Style::Capital].iter().map(|x| x.0.as_str()).collect();
        assert_eq!(cap[2], "600000", "量能放大最高");
        // 这批票一列估值都没有 ⇒ 覆盖率 0 ⇒ 价值桶不得「建成」
        assert!(!map.contains_key(&Style::Value), "覆盖率不达标时不建价值桶");
        let reason = seeds.value.as_ref().and_then(ValueBucket::degraded_reason);
        assert!(reason.unwrap().contains("0/4"), "缺席要能被声明出来");
    }

    #[test]
    fn empty_universe_returns_empty_map() {
        let seeds = build_strategy_seeds(&[], &[], &[]);
        assert!(seeds.by_style.is_empty());
        assert!(seeds.value.is_none(), "没有总体时不产出覆盖率结论");
    }

    /// 价值分 = 「盈利收益率百分位」与「账面收益率百分位」的均值（越便宜越靠前）。
    #[test]
    fn value_bucket_ranks_by_cross_sectional_cheapness() {
        let items = vec![
            item("600001", 0.0, 1.0, Some(5.0), Some(5.0)),
            item("600002", 0.0, 1.0, Some(10.0), Some(1.0)),
            item("600003", 0.0, 1.0, Some(50.0), Some(2.0)),
            item("600004", 0.0, 1.0, Some(20.0), Some(10.0)),
        ];
        let seeds = build_strategy_seeds(&items, &[], &[]);
        let got: Vec<&str> = seeds.by_style[&Style::Value].iter().map(|x| x.0.as_str()).collect();
        assert_eq!(got, vec!["600002", "600001", "600003", "600004"], "价值分降序");
        assert!(
            seeds.value.as_ref().unwrap().degraded_reason().is_none(),
            "覆盖率 100% 时不该声明降级"
        );
    }

    /// 覆盖率判据：等于下限放行、低于下限作废。负控就是「把门写成恒真」——
    /// 若去掉这个判据，25% 覆盖也会建出一个看起来正常的桶。
    #[test]
    fn value_bucket_needs_the_coverage_floor() {
        // 4 只里 2 只有 PB ⇒ 50% = 下限 ⇒ 建桶
        let half = vec![
            item("600001", 0.0, 1.0, None, Some(1.0)),
            item("600002", 0.0, 1.0, Some(9.0), None),
            item("600003", 0.0, 1.0, None, None),
            item("600004", 0.0, 1.0, None, None),
        ];
        let seeds = build_strategy_seeds(&half, &[], &[]);
        assert!(seeds.by_style.contains_key(&Style::Value), "恰好达线要放行");

        // 4 只里 1 只有估值 ⇒ 25% < 50% ⇒ 作废，并给出可显示的缺席理由
        let thin = vec![
            item("600001", 0.0, 1.0, Some(9.0), Some(1.0)),
            item("600002", 0.0, 1.0, None, None),
            item("600003", 0.0, 1.0, None, None),
            item("600004", 0.0, 1.0, None, None),
        ];
        let seeds = build_strategy_seeds(&thin, &[], &[]);
        assert!(!seeds.by_style.contains_key(&Style::Value), "低于下限不得建桶");
        let reason = seeds.value.as_ref().and_then(ValueBucket::degraded_reason).unwrap();
        assert!(reason.contains("1/4") && reason.contains("回落基础池"), "理由要带读数：{reason}");
    }

    /// PE≤0（亏损）不参与盈利收益率，但 PB 可用时仍按账面收益率参与 —— 且**不能**
    /// 因为「1/负数」被判成全市场最便宜。
    #[test]
    fn negative_pe_does_not_make_a_stock_look_cheapest() {
        let items = vec![
            item("600001", 0.0, 1.0, Some(-8.0), Some(12.0)),
            item("600002", 0.0, 1.0, Some(20.0), Some(2.0)),
            item("600003", 0.0, 1.0, Some(30.0), Some(3.0)),
            item("600004", 0.0, 1.0, Some(40.0), Some(4.0)),
        ];
        let seeds = build_strategy_seeds(&items, &[], &[]);
        let got: Vec<&str> = seeds.by_style[&Style::Value].iter().map(|x| x.0.as_str()).collect();
        assert_eq!(got.first().copied(), Some("600002"), "真便宜的前面");
        assert_eq!(got.last().copied(), Some("600001"), "亏损 + 最贵的 PB 应排最后");
    }

    #[test]
    fn nan_and_zero_amount_are_filtered_from_rankings() {
        let items = vec![
            item("600001", f64::NAN, f64::NAN, Some(10.0), Some(1.0)),
            item("600002", 5.0, 2.0, Some(10.0), Some(1.0)),
        ];
        let seeds = build_strategy_seeds(&items, &[], &[]);
        let trend: Vec<&str> = seeds.by_style[&Style::Trend].iter().map(|x| x.0.as_str()).collect();
        assert_eq!(trend, vec!["600002"], "NaN 不得参与排序");
    }

    /// 并列组取**中位秩**：同值必须同分，且分布在两端的并列组不得不对称
    /// （整数除法的写法会把下侧组压成 0、上侧组压成 2/3，同一判据两种结果）。
    #[test]
    fn tied_yields_get_the_mid_rank() {
        let ranks = percentile_ranks(&[
            ("A".to_string(), 1.0),
            ("B".to_string(), 1.0),
            ("C".to_string(), 2.0),
            ("D".to_string(), 3.0),
        ]);
        assert_eq!(ranks["A"], ranks["B"], "同值同分");
        assert!((ranks["A"] - 0.5 / 3.0).abs() < 1e-12, "下侧并列组取中位秩: {:?}", ranks["A"]);
        assert!((ranks["C"] - 2.0 / 3.0).abs() < 1e-12, "{:?}", ranks["C"]);
        assert!((ranks["D"] - 1.0).abs() < 1e-12, "收益率最高（最便宜）的一档应当是 1.0");
    }

    /// NaN 估值既不算「有值」（不抬高覆盖率），也不参与百分位。
    #[test]
    fn nan_valuations_count_as_absent() {
        let items = vec![
            item("600001", 0.0, 1.0, Some(f64::NAN), Some(f64::NAN)),
            item("600002", 0.0, 1.0, Some(f64::NAN), Some(f64::NAN)),
            item("600003", 0.0, 1.0, None, None),
            item("600004", 0.0, 1.0, None, None),
        ];
        let seeds = build_strategy_seeds(&items, &[], &[]);
        assert!(!seeds.by_style.contains_key(&Style::Value), "NaN 不算覆盖率");
    }
}
