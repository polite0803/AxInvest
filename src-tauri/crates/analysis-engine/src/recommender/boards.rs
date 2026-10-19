//! 板块划分与轮动（2026-10-20）：把全市场拆成 4 个板块分轮扫描，摊薄单次荐股的取数压力。
//!
//! ## 为什么要按板块分轮
//!
//! 候选池原先只有「热股 top50 ∪ 行业龙头 30 ∪ 内置样本 ∪ 用户标的」四源（约 190 只，
//! 覆盖全市场 ~3.5%）⇒ 两个榜都不在的妖股没有任何入池通道。要覆盖全市场，一次扫完
//! 约 1.2 万个代码空间（按 80 码/请求 ≈ 150 次批量请求），不适合压在每次荐股上；
//! 按板块轮动后单次只需 13~63 次请求（科创 13 / 创业 25 / 深主 50 / 沪主 63）。
//!
//! ## 代码空间为什么用编码段而不是全量清单
//!
//! 本仓的清单来源（东财 `clist`）被限流：实测 60 页只有 11 页成功，现网只确认了 595 只。
//! 编码段是**稳定**的（交易所规则），无效码由批量行情「缺行」自然过滤，不会随清单过期而漏票；
//! 代价是对稀疏段（北交所）盲扫成本过高，故北交所本轮不参与轮动（见 [`Board::ALL`]）。
//!
//! ## 口径边界
//!
//! - 只用于 **live**：`market_snapshot_batch` 在 as-of 下无历史语义（返回空 + 降级留痕），
//!   回放的历史只能由 `market_daily_close` 表提供。
//! - 轮动是「按本地日序号取模」：4 个板块 4 天覆盖一轮；某天没扫到的板块，当天的
//!   涨幅榜候选只来自收盘快照（`market_daily_close`），不是「全市场当日榜」。

/// A 股板块（按交易所编码段划分）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Board {
    /// 沪市主板：600/601/603/605
    ShMain,
    /// 深市主板：000/001/002/003
    SzMain,
    /// 创业板：300/301
    ChiNext,
    /// 科创板：688
    Star,
    /// 北交所：430/83x/87x/92x —— **不参与轮动**（编码稀疏，盲扫约 500 次请求）
    Bse,
}

impl Board {
    /// 参与轮动的板块（北交所刻意排除，理由见模块文档）。
    pub const ALL: [Board; 4] = [Board::ShMain, Board::SzMain, Board::ChiNext, Board::Star];

    pub const fn as_str(&self) -> &'static str {
        match self {
            Board::ShMain => "sh_main",
            Board::SzMain => "sz_main",
            Board::ChiNext => "chinext",
            Board::Star => "star",
            Board::Bse => "bse",
        }
    }

    pub const fn label_zh(&self) -> &'static str {
        match self {
            Board::ShMain => "沪市主板",
            Board::SzMain => "深市主板",
            Board::ChiNext => "创业板",
            Board::Star => "科创板",
            Board::Bse => "北交所",
        }
    }

    /// 该板块的 6 位数字编码段（闭区间）。
    pub const fn code_ranges(&self) -> &'static [(u32, u32)] {
        match self {
            Board::ShMain => &[(600_000, 601_999), (603_000, 603_999), (605_000, 605_999)],
            Board::SzMain => &[(1, 1_999), (2_000, 3_999)],
            Board::ChiNext => &[(300_000, 301_999)],
            Board::Star => &[(688_000, 688_999)],
            Board::Bse => {
                &[(430_000, 439_999), (830_000, 839_999), (870_000, 879_999), (920_000, 929_999)]
            },
        }
    }

    /// 按代码判板块（`sh`/`sz`/`bj` 前缀可有可无）。
    pub fn classify(code: &str) -> Option<Board> {
        let plain = code.trim_start_matches("sh").trim_start_matches("sz").trim_start_matches("bj");
        let n: u32 = plain.get(..6)?.parse().ok()?;
        Self::classify_numeric(n)
    }

    fn classify_numeric(n: u32) -> Option<Board> {
        let hit = |b: Board| b.code_ranges().iter().any(|(s, e)| (*s..=*e).contains(&n));
        Board::ALL.iter().copied().find(|b| hit(*b)).or_else(|| {
            if hit(Board::Bse) {
                Some(Board::Bse)
            } else {
                None
            }
        })
    }

    /// 本地日序号 → 本轮扫哪个板块（4 天一轮）。
    pub fn for_ordinal(ordinal: usize) -> Board {
        Self::ALL[ordinal % Self::ALL.len()]
    }

    /// 板块的代码空间（零填充 6 位；含无效码，由批量行情自然过滤）。
    pub fn codes(&self) -> Vec<String> {
        let mut out = Vec::new();
        for (start, end) in self.code_ranges() {
            for n in *start..=*end {
                out.push(format!("{n:06}"));
            }
        }
        out
    }
}

/// 用户可选的扫描范围（与 UI 下拉、`settings.reco_board_scope` 逐字一致）。
pub const VALID_SCOPES: [&str; 6] = ["auto", "sh_main", "sz_main", "chinext", "star", "all"];

/// 把用户的板块选择解析成**本次要扫的板块列表**。
///
/// - `auto` ⇒ 按本地日序号轮动一个板块（默认，4 天一轮）；
/// - `all` ⇒ 四个板块都扫（请求量约 138 次，用户显式选择才走）；
/// - 其余 ⇒ 单个板块键。
///
/// 非法值返回 `None`（调用方按 auto 处理并 warn，**不静默改成 all**——
/// 那会让一次手误把取数压力放大 4 倍）。
pub fn boards_for_scope(scope: &str, ordinal: usize) -> Option<Vec<Board>> {
    match scope {
        "auto" => Some(vec![Board::for_ordinal(ordinal)]),
        "all" => Some(Board::ALL.to_vec()),
        other => Board::ALL.iter().copied().find(|b| b.as_str() == other).map(|b| vec![b]),
    }
}

/// 从批量行情里挑「当日强势股」：剔除 ST、要求涨幅 ≥ `min_change_pct`，按涨幅降序取前 N。
///
/// 纯函数（唯一的筛选点），负控见测试：ST 不得入池、低于门槛不得凑数、NaN 不得排序。
pub fn pick_movers(
    quotes: &[axagent_harness::market_data::StockQuote],
    min_change_pct: f64,
    top_n: usize,
) -> Vec<(String, String, Option<String>)> {
    let mut hit: Vec<&axagent_harness::market_data::StockQuote> = quotes
        .iter()
        .filter(|q| {
            !q.name.contains("ST")
                && q.change_pct.is_finite()
                && q.change_pct >= min_change_pct
                && q.price > 0.0
        })
        .collect();
    hit.sort_by(|a, b| {
        b.change_pct
            .partial_cmp(&a.change_pct)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.code.cmp(&b.code))
    });
    hit.into_iter().take(top_n).map(|q| (q.code.clone(), q.name.clone(), None)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_covers_each_board_boundary() {
        assert_eq!(Board::classify("600000"), Some(Board::ShMain));
        assert_eq!(Board::classify("sh601318"), Some(Board::ShMain));
        assert_eq!(Board::classify("605599"), Some(Board::ShMain));
        assert_eq!(Board::classify("000001"), Some(Board::SzMain));
        assert_eq!(Board::classify("002696"), Some(Board::SzMain));
        assert_eq!(Board::classify("300750"), Some(Board::ChiNext));
        assert_eq!(Board::classify("301302"), Some(Board::ChiNext));
        assert_eq!(Board::classify("688981"), Some(Board::Star));
        assert_eq!(Board::classify("830799"), Some(Board::Bse));
        assert_eq!(Board::classify("920008"), Some(Board::Bse));
        // 负控：不存在的段不得被任何板块认领
        assert_eq!(Board::classify("400001"), None);
        assert_eq!(Board::classify("not-a-code"), None);
    }

    #[test]
    fn code_space_matches_ranges() {
        assert_eq!(Board::ShMain.codes().len(), 2_000 + 1_000 + 1_000);
        assert_eq!(Board::SzMain.codes().len(), 1_999 + 2_000);
        assert_eq!(Board::ChiNext.codes().len(), 2_000);
        assert_eq!(Board::Star.codes().len(), 1_000);
        assert!(Board::Star.codes().contains(&"688001".to_string()));
        assert!(Board::ChiNext.codes().contains(&"300001".to_string()));
        // 零填充必须是 6 位（"000001" 而非 "1"），否则批量行情会把它当非法码
        assert!(Board::SzMain.codes().contains(&"000001".to_string()));
    }

    #[test]
    fn rotation_cycles_through_all_four_boards() {
        let seen: Vec<Board> = (0..4).map(Board::for_ordinal).collect();
        assert_eq!(seen, Board::ALL.to_vec());
        assert_eq!(Board::for_ordinal(4), Board::ShMain, "4 天一轮循环");
    }

    #[test]
    fn scope_parsing_covers_auto_all_and_single_boards() {
        assert_eq!(boards_for_scope("auto", 2), Some(vec![Board::ChiNext]));
        assert_eq!(boards_for_scope("all", 0).map(|v| v.len()), Some(4));
        assert_eq!(boards_for_scope("star", 0), Some(vec![Board::Star]));
        assert_eq!(boards_for_scope("sh_main", 0), Some(vec![Board::ShMain]));
        // 负控：非法值不得被解析成 all（一次手误会把请求量放大 4 倍），也不得猜成 auto
        assert_eq!(boards_for_scope("ALL", 0), None);
        assert_eq!(boards_for_scope("", 0), None);
        assert_eq!(boards_for_scope("bse", 0), None, "北交所不在可选项内");
    }

    fn quote(code: &str, name: &str, chg: f64) -> axagent_harness::market_data::StockQuote {
        serde_json::from_value(serde_json::json!({
            "code": code, "name": name, "price": 10.0, "preClose": 9.0,
            "open": 9.5, "high": 10.0, "low": 9.4, "volume": 1.0, "amount": 1.0,
            "changePct": chg, "turnoverRate": 1.0, "timestamp": "2026-10-19",
        }))
        .expect("StockQuote 可从 JSON 构造")
    }

    /// NaN 不能走 `json!`（serde_json 会把它变成 null，再反序列化成 f64 就报
    /// 「invalid type: null」）—— 构造后直接改字段。
    fn quote_with_nan_change(code: &str, name: &str) -> axagent_harness::market_data::StockQuote {
        let mut q = quote(code, name, 0.0);
        q.change_pct = f64::NAN;
        q
    }

    #[test]
    fn pick_movers_filters_and_ranks() {
        let qs = vec![
            quote("600001", "甲", 9.9),
            quote("600002", "*ST乙", 10.0),
            quote("600003", "丙", 1.2),
            quote("600004", "丁", 5.0),
            quote_with_nan_change("600005", "戊"),
        ];
        let got = pick_movers(&qs, 3.0, 10);
        let codes: Vec<&str> = got.iter().map(|x| x.0.as_str()).collect();
        assert_eq!(codes, vec!["600001", "600004"], "ST/低涨幅/NaN 都不得入池，且按涨幅降序");
        assert_eq!(pick_movers(&qs, 3.0, 1).len(), 1, "top_n 必须生效");
        assert!(pick_movers(&qs, 20.0, 10).is_empty(), "无人达标时返回空，不得凑数");
    }
}
