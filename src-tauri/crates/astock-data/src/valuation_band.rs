//! 估值带（Valuation Band）— R3-C
//!
//! 给定一只股票历史财务快照(`FinancialSnapshot`)和当前快照,计算 PE / PB / PS 的
//! 5/10/25/50/75/90/95 分位带 + 当前值在分布中的位置。
//!
//! 用途：
//! - 估值带用于估值偏离度判断:当 PE 当前分位 < 25% 时认为是"历史低位"；> 75% 时为"历史高位"。
//! - 可视化为"PE/PB/PS 历年分布 + 当前红点"。

use serde::{Deserialize, Serialize};

use crate::types::ValuationSnapshot;

/// 单一指标的"分位带"
///
/// `percentile` 数组:索引 0..6 对应 [5, 10, 25, 50, 75, 90, 95] 分位值;
/// `current` = 当前快照该指标;`current_percentile` = 当前值在历史样本中的分位(0..100)。
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct MetricBand {
    /// 5 / 10 / 25 / 50 / 75 / 90 / 95 百分位
    pub percentiles: [f64; 7],
    pub current: Option<f64>,
    pub current_percentile: Option<f64>,
    /// 样本数(去掉 None 后的有效值)
    pub sample_size: usize,
}

/// 估值带综合结果
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ValuationBand {
    pub stock_code: String,
    pub metric_pe: MetricBand,
    pub metric_pb: MetricBand,
    pub metric_ps: MetricBand,
    /// 历史样本的时间范围(最早日期 ~ 最晚日期)
    pub sample_start: Option<String>,
    pub sample_end: Option<String>,
    /// 评估结论
    pub verdict: String,
    /// 数据来源描述
    pub note: Option<String>,
}

const PERCENTILE_KEYS: [f64; 7] = [5.0, 10.0, 25.0, 50.0, 75.0, 90.0, 95.0];

/// 从一组有效值计算分位带。
///
/// 内部使用 nearest-rank 方法(简单稳健);要求输入非空。
fn compute_percentiles(values: &[f64], keys: &[f64; 7]) -> [f64; 7] {
    let mut sorted: Vec<f64> = values.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let n = sorted.len();
    let mut out = [0.0; 7];
    if n == 0 {
        return out;
    }
    for (i, &p) in keys.iter().enumerate() {
        // rank = ceil(p/100 * n), 用 1-indexed; clamp 到 [1, n]
        let rank_f = (p / 100.0) * (n as f64);
        let mut rank = rank_f.ceil() as usize;
        if rank < 1 {
            rank = 1;
        }
        if rank > n {
            rank = n;
        }
        out[i] = sorted[rank - 1];
    }
    out
}

/// 计算"当前值"在历史样本中的百分位。
///
/// 用 `(n_lower + 0.5 * n_equal) / n_total * 100` 近似(连续分布假设),
/// 返回 0..100 范围;当 `current` 不在样本范围时返回 0 或 100。
pub fn current_percentile(values: &[f64], current: f64) -> f64 {
    let n = values.len();
    if n == 0 {
        return 50.0;
    }
    let mut lower = 0usize;
    let mut equal = 0usize;
    for &v in values {
        if v < current {
            lower += 1;
        } else if v == current {
            equal += 1;
        }
    }
    let rank = (lower as f64) + 0.5 * (equal as f64);
    (rank / (n as f64)) * 100.0
}

/// 单指标计算:从 snapshots 提取有效值 → 计算分位 → 用 current 计算当前位置。
///
/// **非正值一律视为无效**：PE / PB / PS ≤ 0 意味着亏损、净资产为负或营收为负，
/// 此时"分位"没有金融含义。未过滤时实测会出现 P5=-119、P25=-6.8 这类负分位带，
/// 把亏损公司（如 600876 当前 PE=-4.31）误判成"估值偏低"。
/// 注意 `current` 本身仍原样返回（前端要展示"当前 PE=-4.31"），
/// 仅当其 > 0 时才计算分位，否则 `current_percentile` = None（前端显示 "—"）。
pub fn metric_band_from(samples: &[Option<f64>], current: Option<f64>) -> MetricBand {
    let values: Vec<f64> = samples.iter().filter_map(|v| *v).filter(|v| *v > 0.0).collect();
    let sample_size = values.len();
    if sample_size == 0 {
        return MetricBand {
            percentiles: [0.0; 7],
            current,
            current_percentile: None,
            sample_size: 0,
        };
    }
    let percentiles = compute_percentiles(&values, &PERCENTILE_KEYS);
    let current_percentile = current.filter(|c| *c > 0.0).map(|c| current_percentile(&values, c));
    MetricBand { percentiles, current, current_percentile, sample_size }
}

/// 从历史快照 + 当前快照,计算完整估值带。
///
/// 建议 historical 长度 ≥ 20 才有意义;不足时会设置 verdict = "insufficient"。
pub fn compute_valuation_band<S: FinancialSnapshotLike>(
    stock_code: &str,
    historical: &[S],
    current: Option<&S>,
) -> ValuationBand {
    let pe_vals: Vec<Option<f64>> = historical.iter().map(|s| s.pe_ttm()).collect();
    let pb_vals: Vec<Option<f64>> = historical.iter().map(|s| s.pb()).collect();
    let ps_vals: Vec<Option<f64>> = historical.iter().map(|s| s.ps_ttm()).collect();

    let pe = metric_band_from(&pe_vals, current.and_then(|c| c.pe_ttm()));
    let pb = metric_band_from(&pb_vals, current.and_then(|c| c.pb()));
    let ps = metric_band_from(&ps_vals, current.and_then(|c| c.ps_ttm()));

    let sample_start = historical.iter().map(|s| s.snapshot_date().to_string()).min();
    let sample_end = historical.iter().map(|s| s.snapshot_date().to_string()).max();

    let verdict = if pe.sample_size < 20 || pb.sample_size < 20 {
        "insufficient"
    } else {
        verdict_from_bands(&pe, &pb)
    };

    // note 需区分两种"样本不足"：数据本来就少 vs 因亏损/负值被剔除。
    // 后者（长期亏损股）若只说"历史样本不足"，会让人误以为数据没拉到。
    let pe_dropped = pe_vals.iter().filter(|v| v.is_some()).count() - pe.sample_size;
    let note = if pe.sample_size < 20 {
        if pe_dropped > 0 {
            Some(format!(
                "PE 有效样本不足（剔除 {} 条亏损/负值后仅 {} < 20），分位仅供参考",
                pe_dropped, pe.sample_size
            ))
        } else {
            Some(format!("历史样本不足(PE {} < 20),分位仅供参考", pe.sample_size))
        }
    } else {
        None
    };

    ValuationBand {
        stock_code: stock_code.to_string(),
        metric_pe: pe,
        metric_pb: pb,
        metric_ps: ps,
        sample_start,
        sample_end,
        verdict: verdict.to_string(),
        note,
    }
}

/// 把「回溯年数」换算成窗口起始日期（`YYYY-MM-DD`）—— **窗口口径的唯一来源**。
///
/// ## 为什么必须共享这一条（而不是各调用方自己算）
///
/// 估值带的结论**完全取决于窗口**：同一个 PE 在 3 年窗口与 8 年窗口里的分位可以差 30 个百分点。
/// 本仓有**两条**互不相干的生产路径会算同一个 band：
/// - 命令层 `commands::stock_analysis::compute_valuation_band`（前端 `ValuationBandChart` 用）；
/// - MCP 工具 `mcp_tools` 的 `compute_valuation_band` 分支（工作流 `t-valuation-band` 用）。
///
/// 两者若各自算窗口，就会出现「图上显示 PE 分位 22%（低位），工作流锚腿却按 61%（高位）给折价」
/// 这种**同一指标两个结论**的形态 —— 且没有任何报错。故两侧都调本函数。
///
/// 行为与命令层原实现**逐位一致**（`365 * years`，下溢时兜底 `"0000-00-00"`）：
/// 刻意**不**加 `max(1)` —— 加了会把「显式传 0 年」的语义从「窗口为空」改成「1 年窗口」，
/// 属于静默改变既有前端图的口径。
pub fn since_date_from_years(years: u32) -> String {
    let days = 365i64 * years as i64;
    chrono::Local::now()
        .date_naive()
        .checked_sub_signed(chrono::Duration::days(days))
        .map(|d| d.format("%Y-%m-%d").to_string())
        .unwrap_or_else(|| "0000-00-00".to_string())
}

/// 按起始日期裁剪历史估值序列，并**归一到升序**（`YYYY-MM-DD` 字符串比较）。
///
/// ## 三条必须同时说明的事（任一缺失都会静默算出错的分位）
///
/// 1. **为什么必须裁剪**：上游 `AStockClient::get_valuation_history` 的入参是**年数**，
///    而它的分页策略（`vendors/eastmoney.rs`：`max_pages = want/PAGE_SIZE + 2`）
///    会**多取约 2 页 ≈ 2 年** ⇒ 「年数入参」本身**不足以**定窗口。
/// 2. **为什么必须归一排序**：`compute_valuation_band` 的调用约定是
///    **`current` 取 `historical.last()`**（"最后一条 = 最新一条"），
///    而两个来源的顺序**原本相反** —— vendor 接口按 `TRADE_DATE` **降序**返回，
///    且 `AStockClient::get_valuation_history` **原样返回不重排**
///    （`vendors/eastmoney.rs:711`、`lib.rs:2295-2296`）；命令层读表则是
///    `.order_by_asc(SnapshotDate)` ⇒ **升序**（`commands/stock_analysis.rs`）。
///    ⇒ 不归一的话，**同一条腿在两条路径上会拿不同的 `current`**：
///    工具路径会把窗口内**最旧**那天的 PE 当"当前 PE"去算分位（数字离谱且无报错）。
///    本函数统一输出**升序**，把这条约定钉在唯一点上。
/// 3. **分位数本身与顺序无关**（`metric_band_from` 内部排序），受顺序影响的**只有 `current`**
///    以及 `sample_start`/`sample_end`（后者用 min/max，也无关）—— 所以这个缺陷**只会**
///    表现为"当前分位"错，最容易被人当成数据问题而非代码问题。
pub fn clip_valuation_history(
    mut snaps: Vec<ValuationSnapshot>,
    since_date: &str,
) -> Vec<ValuationSnapshot> {
    snaps.retain(|s| !s.trade_date.is_empty() && s.trade_date.as_str() >= since_date);
    snaps.sort_by(|a, b| a.trade_date.cmp(&b.trade_date));
    snaps
}

fn verdict_from_bands(pe: &MetricBand, pb: &MetricBand) -> &'static str {
    let pe_pct = pe.current_percentile.unwrap_or(50.0);
    let pb_pct = pb.current_percentile.unwrap_or(50.0);
    let avg = (pe_pct + pb_pct) / 2.0;
    if avg < 25.0 {
        "deep_value"
    } else if avg < 40.0 {
        "undervalued"
    } else if avg > 75.0 {
        "overvalued"
    } else if avg > 60.0 {
        "expensive"
    } else {
        "fair"
    }
}

/// 最小化的"快照"接口,避免与 ORM 模型耦合。
///
/// `FinancialSnapshot` 实体实现此 trait,测试中的 mock struct 也实现。
pub trait FinancialSnapshotLike {
    fn snapshot_date(&self) -> &str;
    fn pe_ttm(&self) -> Option<f64>;
    fn pb(&self) -> Option<f64>;
    fn ps_ttm(&self) -> Option<f64>;
}

/// `ValuationSnapshot`（东财历史估值日序列的行类型）直接可当样本用。
///
/// 有了它，任何拿到 `Vec<ValuationSnapshot>` 的调用方都不必再写一遍 adapter ——
/// 例如 `mcp_tools::execute_mcp_tool` 的 `compute_valuation_band` 分支
/// 与命令层 `commands::stock_analysis::compute_valuation_band` 的 DB 行 adapter
/// （后者来源是 ORM Model，仍各自需要自己的 adapter，但都收敛到同一个 trait）。
impl FinancialSnapshotLike for ValuationSnapshot {
    fn snapshot_date(&self) -> &str {
        &self.trade_date
    }
    fn pe_ttm(&self) -> Option<f64> {
        self.pe_ttm
    }
    fn pb(&self) -> Option<f64> {
        self.pb
    }
    fn ps_ttm(&self) -> Option<f64> {
        self.ps_ttm
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 测试用 mock
    #[derive(Debug, Clone)]
    struct MockSnap {
        date: String,
        pe: Option<f64>,
        pb: Option<f64>,
        ps: Option<f64>,
    }

    impl FinancialSnapshotLike for MockSnap {
        fn snapshot_date(&self) -> &str {
            &self.date
        }
        fn pe_ttm(&self) -> Option<f64> {
            self.pe
        }
        fn pb(&self) -> Option<f64> {
            self.pb
        }
        fn ps_ttm(&self) -> Option<f64> {
            self.ps
        }
    }

    fn sample_data(n: usize) -> Vec<MockSnap> {
        (0..n)
            .map(|i| MockSnap {
                date: format!("2024-{:02}-01", (i % 12) + 1),
                pe: Some(10.0 + (i as f64) * 0.5), // 10, 10.5, 11.0 ... 10 + 0.5*(n-1)
                pb: Some(1.0 + (i as f64) * 0.05),
                ps: Some(2.0 + (i as f64) * 0.02),
            })
            .collect()
    }

    #[test]
    fn percentiles_sorted() {
        let v: Vec<f64> = (1..=100).map(|i| i as f64).collect();
        let p = compute_percentiles(&v, &PERCENTILE_KEYS);
        // 5% → 5
        assert!((p[0] - 5.0).abs() < 1e-6, "p5={}", p[0]);
        // 50% → 50
        assert!((p[3] - 50.0).abs() < 1e-6, "p50={}", p[3]);
        // 95% → 95
        assert!((p[6] - 95.0).abs() < 1e-6, "p95={}", p[6]);
        // 升序
        for i in 0..6 {
            assert!(p[i] <= p[i + 1] + 1e-6);
        }
    }

    #[test]
    fn current_percentile_mid() {
        let v: Vec<f64> = (1..=10).map(|i| i as f64).collect();
        // 5.5 不在样本中,应在中间
        let p = current_percentile(&v, 5.5);
        // lower=5, equal=0, rank=5, 5/10=50%
        assert!((p - 50.0).abs() < 1e-6, "p={}", p);
    }

    #[test]
    fn current_percentile_below() {
        let v: Vec<f64> = (10..=20).map(|i| i as f64).collect();
        let p = current_percentile(&v, 5.0);
        // 全部都 > 5, lower=0, 0/11=0
        assert!((p - 0.0).abs() < 1e-6, "p={}", p);
    }

    #[test]
    fn band_full_sample() {
        let samples = sample_data(60);
        // 60 个样本,pe 范围 10..39.5,pb 范围 1.0..3.95
        // pe=30 落在 (30-10)/0.5 = 第 40 名 → 40/60 = 66.7% → 触发 "expensive" (>60)
        let current = MockSnap {
            date: "2025-01-01".to_string(),
            pe: Some(30.0),
            pb: Some(3.0),
            ps: Some(3.0),
        };
        let band = compute_valuation_band("000001", &samples, Some(&current));
        assert_eq!(band.stock_code, "000001");
        assert_eq!(band.metric_pe.sample_size, 60);
        // PE 30 落在 40/60 = 66.7%, PB 3.0 落在 (3.0-1.0)/0.05 = 第 40 名 → 66.7%
        let pe_pct = band.metric_pe.current_percentile.unwrap();
        assert!(pe_pct > 60.0 && pe_pct < 75.0, "pe_pct={}", pe_pct);
        let pb_pct = band.metric_pb.current_percentile.unwrap();
        assert!(pb_pct > 60.0 && pb_pct < 75.0, "pb_pct={}", pb_pct);
        assert_eq!(band.verdict, "expensive");
    }

    #[test]
    fn band_insufficient_sample() {
        let samples = sample_data(5);
        let band = compute_valuation_band("000002", &samples, None);
        assert_eq!(band.verdict, "insufficient");
        assert!(band.note.is_some());
    }

    #[test]
    fn band_deep_value() {
        // 让 current 远低于历史
        let samples = sample_data(60);
        let current = MockSnap {
            date: "2025-01-01".to_string(),
            pe: Some(11.0),
            pb: Some(1.05),
            ps: Some(2.05),
        };
        let band = compute_valuation_band("000003", &samples, Some(&current));
        assert_eq!(band.verdict, "deep_value");
    }

    #[test]
    fn band_empty() {
        let band: ValuationBand = compute_valuation_band::<MockSnap>("000004", &[], None);
        assert_eq!(band.verdict, "insufficient");
        assert_eq!(band.metric_pe.sample_size, 0);
    }

    #[test]
    fn band_overvalued() {
        let samples = sample_data(60);
        // 60 个样本 pe=10..39.5, 给一个 38 的 PE
        let current = MockSnap {
            date: "2025-01-01".to_string(),
            pe: Some(38.0),
            pb: Some(3.8),
            ps: Some(4.0),
        };
        let band = compute_valuation_band("000005", &samples, Some(&current));
        assert_eq!(band.verdict, "overvalued");
    }

    #[test]
    fn band_handles_none_in_samples() {
        // 插入一些 None 后有效样本 = 20,刚好达到 20 阈值,verdict 应为 "fair" 或正常输出
        let mut samples = sample_data(30);
        for s in samples.iter_mut().step_by(3) {
            s.pe = None;
            s.pb = None;
        }
        let band = compute_valuation_band("000006", &samples, None);
        assert_eq!(band.metric_pe.sample_size, 20);
        // 20 >= 20 阈值,verdict 不再是 "insufficient"
        assert_ne!(band.verdict, "insufficient");
    }

    #[test]
    fn band_handles_too_few_after_filter() {
        // 30 个里 15 个被过滤 → 15 < 20,应 insufficient
        let mut samples = sample_data(30);
        for s in samples.iter_mut().step_by(2) {
            s.pe = None;
            s.pb = None;
        }
        let band = compute_valuation_band("000007", &samples, None);
        assert_eq!(band.metric_pe.sample_size, 15);
        assert_eq!(band.verdict, "insufficient");
        assert!(band.note.is_some());
    }

    #[test]
    fn band_ignores_non_positive_values() {
        // 亏损期样本（PE/PB < 0）必须剔除：否则出现负分位带，
        // 并把"当前仍亏损"的公司误判为估值偏低（实测 600876 P5=-119 / 当前 PE=-4.31）。
        let mut samples = sample_data(40);
        for (i, s) in samples.iter_mut().enumerate() {
            if i < 10 {
                s.pe = Some(-5.0 - i as f64);
                s.pb = Some(-1.0);
            }
        }
        let current = MockSnap {
            date: "2025-01-01".to_string(),
            pe: Some(-3.0),
            pb: Some(2.0),
            ps: Some(3.0),
        };
        let band = compute_valuation_band("000008", &samples, Some(&current));
        // 有效样本 = 40 - 10
        assert_eq!(band.metric_pe.sample_size, 30);
        // 分位带必须全为正
        assert!(band.metric_pe.percentiles.iter().all(|v| *v > 0.0));
        // current 原样保留（前端要展示"当前 PE=-3.0"），但不参与分位计算
        assert_eq!(band.metric_pe.current, Some(-3.0));
        assert!(band.metric_pe.current_percentile.is_none());
        assert_ne!(band.verdict, "insufficient");
    }

    #[test]
    fn band_note_distinguishes_dropped_from_missing() {
        // 全部 PE 为负 → 有效样本 0；note 必须说明是"剔除负值"而非"数据没拉到"
        let mut samples = sample_data(30);
        for s in samples.iter_mut() {
            s.pe = Some(-8.0);
        }
        let band = compute_valuation_band("000009", &samples, None);
        assert_eq!(band.metric_pe.sample_size, 0);
        assert_eq!(band.verdict, "insufficient");
        assert!(band.metric_pe.percentiles.iter().all(|v| *v == 0.0));
        let note = band.note.unwrap_or_default();
        assert!(note.contains("剔除"), "note 应说明剔除了负值样本: {note}");
    }

    // ── V79(2026-09-21)：窗口裁剪 + 排序归一 ──
    //
    // 这三条挡的不是"锦上添花"，而是一个**只会错「当前分位」、且不报错**的缺陷：
    // vendor 原始顺序是**降序**、命令层读表是**升序**，而 `current` 的约定是
    // `historical.last()`（见 `compute_valuation_band` 的入参说明）。

    fn vsnap(date: &str, pe: f64) -> ValuationSnapshot {
        ValuationSnapshot {
            trade_date: date.to_string(),
            pe_ttm: Some(pe),
            pb: Some(1.0),
            ps_ttm: Some(1.0),
            ..Default::default()
        }
    }

    #[test]
    fn clip_normalizes_descending_input_to_ascending() {
        // 输入刻意按 vendor 原样给（降序）
        let got = clip_valuation_history(
            vec![vsnap("2025-03-01", 3.0), vsnap("2025-02-01", 2.0), vsnap("2025-01-01", 1.0)],
            "2025-01-01",
        );
        let dates: Vec<&str> = got.iter().map(|s| s.trade_date.as_str()).collect();
        assert_eq!(dates, vec!["2025-01-01", "2025-02-01", "2025-03-01"], "必须归一到升序");
        // 反向对照：`last()` 必须是**最新**那天。若实现退回"原样返回"，
        // 这里会拿到 2025-01-01（最旧）⇒ 当前分位会用最旧那天的 PE 去算。
        assert_eq!(got.last().map(|s| s.trade_date.as_str()), Some("2025-03-01"));
    }

    #[test]
    fn clip_drops_empty_dates_and_out_of_window() {
        let got = clip_valuation_history(
            vec![
                vsnap("", 9.0),           // 空日期必须丢
                vsnap("2024-12-31", 9.0), // 窗口外（< since）必须丢
                vsnap("2025-01-01", 1.0), // 边界包含
                vsnap("2025-06-30", 2.0),
            ],
            "2025-01-01",
        );
        assert_eq!(got.len(), 2);
        assert_eq!(got[0].trade_date, "2025-01-01");
    }

    #[test]
    fn since_date_from_years_keeps_zero_year_semantics() {
        // 刻意**不**加 `max(1)`：传 0 年 = 窗口从"今天"起（取不到历史样本），
        // 与命令层原实现逐位一致。若有人顺手补 `max(1)`，本断言会红 ——
        // 因为那是在**静默改变**既有前端图的口径，必须显式决策而不是顺手加。
        let y0 = since_date_from_years(0);
        assert_ne!(y0, since_date_from_years(1), "0 年窗口不得被 max(1) 抬成 1 年窗口");
        assert_eq!(y0.len(), 10);
        // 单调性 + 格式
        let (y5, y4) = (since_date_from_years(5), since_date_from_years(4));
        assert!(y5 < y4, "5 年窗口起点应早于 4 年: {y5} vs {y4}");
        assert_eq!(y5.len(), 10);
    }
}
