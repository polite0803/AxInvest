//! 周期股估值开关（P0-4，`PLAN-stock-decision-reliability.md` R1）
//!
//! ## 为什么需要行业开关
//!
//! 紫金矿业（601899，有色金属）2026-10-09 实证：最近完整年报 FCF 444 亿处在
//! 商品价格周期高位，正向 DCF 拿它当永续基数、再叠加含上行周期的历史增速，
//! 给出保守档 +25.7% 的「便宜」结论；同期 PE 因周期高位 E 大而落在自身 5 年
//! 8% 分位，两条估值口径**同向**把 long 档推成买入，随后 4 个交易日连续回落。
//!
//! 既有的数据形态判据（`is_peak_annual_fcf` 一次性峰值检测、FCF/净利天花板）
//! 对本形态**结构性失效**：资源景气可持续多年，近 5 年 FCF 序列整体偏高、
//! 没有「含亏损年份的一次性跃迁」，峰值倍数不触发 —— 故必须在数据形态判据
//! 之外补一条**行业先验**开关：行业命中标定的周期组时，估值链切换到
//! 周期中性化口径，不再假设最新年报现金流可永续外推。
//!
//! ## 触发后的四处口径变化（与 PLAN P0-4 四子项一一对应）
//!
//! 1. DCF 锚：最近年报 FCF → 近 5 年年报 FCF **中位数**（本模块
//!    [`cyclical_median_annual_fcf`]）；
//! 2. PE band：周期高位 E 高 ⇒ PE 分位系统性偏低，信号置空不进方向
//!    （`valuation_band::compute_valuation_band` 的 `pe_suppressed`）；
//! 3. long 档安全边际门：阈值 20% → 35%（周期股估值中枢摆动大，
//!    见 `portfolio-mgr-h-long.rhai` 的周期门形态）；
//! 4. 反向 DCF 隐含 g<0 时禁止「合理偏低」结论（`valuation::build_conclusion`）。
//!
//! ## 行业名口径
//!
//! 数据源是东财行业（EM2016，**不是申万**），一级名如「有色金属」「煤炭采选」
//! 「钢铁行业」「石油行业」「港口水运」「农牧饲渔」「券商信托」。判定同时
//! 覆盖一级 `sector_name` 与二级 `sub_sector`，按**包含关键词**命中 ——
//! vendor 行业名历史上有过措辞微调，等值匹配脆。

use crate::valuation::median_of;
// `FinancialReport` 权威定义在 harness（`market_data`），由 crate 根 re-export。
use crate::FinancialReport;

/// 周期组关键词（PLAN P0-4 标定的 7 组：有色 / 煤炭 / 钢铁 / 石化 / 航运 /
/// 养殖 / 券商）。
///
/// 取舍：
/// - 「石化」组收在 **石油 / 石化**，**不含**泛「化工」—— 化工子行业多数
///   不随商品价格同幅摆动，纳入会扩大误伤面；
/// - 「航运」用「航运」+「港口水运」（东财一级旧名），不含「港口」
///   （港口是收费型基础设施，现金流不随运价周期）；
/// - 「券商」用「证券」+「券商」（东财旧名「券商信托」），不含泛「金融」。
pub const CYCLICAL_KEYWORDS: &[&str] = &[
    // 有色（含贵金属细分）
    "有色",
    "贵金属",
    "工业金属",
    "小金属", // 煤炭
    "煤炭",   // 钢铁
    "钢铁",   // 石化（石油开采 / 石油化工；不含泛化工）
    "石油",
    "石化", // 航运（东财一级旧名「港口水运」）
    "航运",
    "港口水运", // 养殖（东财一级「农牧饲渔」）
    "农牧",
    "饲渔",
    "养殖",
    "畜牧",
    "渔业", // 券商
    "证券",
    "券商",
];

/// 行业（一级 + 二级）是否命中周期组。空串 / 空名不命中 ——
/// 取不到行业时开关**不触发**（按普通股处理），不把「不知道」升级成「周期股」。
pub fn is_cyclical_sector(sector_name: &str, sub_sector: &str) -> bool {
    let hit = |name: &str| -> bool {
        if name.trim().is_empty() {
            return false;
        }
        CYCLICAL_KEYWORDS.iter().any(|k| name.contains(k))
    };
    hit(sector_name) || hit(sub_sector)
}

/// 中位数锚要求的**最少年报样本数**：只有 1 个年度值时「中位数」就是单点，
/// 不构成跨周期中性化（与最新年锚没有任何区别）；2 个起步才有位置统计量意义。
pub const CYCLE_MEDIAN_MIN_SAMPLE: usize = 2;

/// 周期中性化 FCF 锚：近 `max_years` 个**完整会计年度** FCF 的中位数（元）。
///
/// 与 [`crate::mcp_tools::annual_fcf_series`] 同序列口径（保留负值年份：
/// 周期底部的负 FCF 是真实的跨周期信息，剔掉它会把中位数系统性抬高）。
/// 中位数天然抗「最新年是顶 / 是底」—— 这正是周期股要的中性化。
///
/// 返回 `None`：年度样本 < [`CYCLE_MEDIAN_MIN_SAMPLE`]。调用方自行决定中位数
/// ≤ 0 时的回落（DCF 锚必须为正，非正锚落既有「近 5 年正净利均值」代理链）。
pub fn cyclical_median_annual_fcf(financials: &[FinancialReport], max_years: usize) -> Option<f64> {
    let series = crate::mcp_tools::annual_fcf_series(financials, max_years);
    if series.len() < CYCLE_MEDIAN_MIN_SAMPLE {
        return None;
    }
    median_of(series)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cyclical_sector_matches_seven_groups() {
        // 东财一级名（现行 + 旧名）
        assert!(is_cyclical_sector("有色金属", ""));
        assert!(is_cyclical_sector("煤炭采选", ""));
        assert!(is_cyclical_sector("钢铁行业", ""));
        assert!(is_cyclical_sector("石油行业", ""));
        assert!(is_cyclical_sector("港口水运", ""));
        assert!(is_cyclical_sector("农牧饲渔", ""));
        assert!(is_cyclical_sector("券商信托", ""));
        assert!(is_cyclical_sector("证券", ""));
        // 紫金矿业：一级 / 二级任一命中即可
        assert!(is_cyclical_sector("有色金属", "黄金"));
        // 二级细分命中（一级名泛化时）
        assert!(is_cyclical_sector("农牧饲渔", "畜禽养殖"));
    }

    #[test]
    fn non_cyclical_sectors_not_flagged() {
        // 泛化工不纳入石化组；银行 / 保险 / 医药 / 白酒 / 电力（公用事业型）
        // 都不是本开关标定的周期组
        for (one, two) in [
            ("化工行业", "化学制品"),
            ("银行", "国有大型银行"),
            ("保险", "人寿保险"),
            ("医药制造", "化学制剂"),
            ("食品饮料", "白酒"),
            ("电力行业", "火电"),
            ("电子信息", "半导体"),
        ] {
            assert!(!is_cyclical_sector(one, two), "{one}/{two} 不应命中周期组");
        }
        // 空名 / 缺数不触发（不把「不知道」当周期股）
        assert!(!is_cyclical_sector("", ""));
        assert!(!is_cyclical_sector("  ", ""));
    }

    fn report(date: &str, ocf: Option<f64>, capex: Option<f64>) -> FinancialReport {
        FinancialReport {
            stock_code: "601899".to_string(),
            report_date: date.to_string(),
            revenue: None,
            net_profit: None,
            eps: None,
            bps: None,
            roe: None,
            debt_ratio: None,
            gross_margin: None,
            net_margin: None,
            revenue_yoy: None,
            profit_yoy: None,
            total_assets: None,
            operating_cash_flow: ocf,
            capital_expenditure: capex,
            free_cash_flow: None,
            current_ratio: None,
            quick_ratio: None,
            goodwill: None,
            accounts_receivable: None,
            estimated: None,
            disclosure_date: None,
        }
    }

    #[test]
    fn median_anchor_neutralizes_cycle_peak() {
        // 紫金式形态：最新年（[0]）444 亿周期高位，前 4 年 120/200/90/160 亿。
        // 中位数 ≈ 160 亿（含全部年份排序后的位置统计量），锚不再是顶点 444。
        let financials = vec![
            report("2025-12-31", Some(444.0e8), Some(0.0)),
            report("2024-12-31", Some(200.0e8), Some(0.0)),
            report("2023-12-31", Some(160.0e8), Some(0.0)),
            report("2022-12-31", Some(120.0e8), Some(0.0)),
            report("2021-12-31", Some(90.0e8), Some(0.0)),
        ];
        let med = cyclical_median_annual_fcf(&financials, 5).expect("5 个年度值 ⇒ 有中位数");
        assert!((med - 160.0e8).abs() < 1.0, "中位数应为 160 亿，实得 {med}");
        assert!(med < 444.0e8 * 0.5, "中性化锚必须显著低于周期顶点");
    }

    #[test]
    fn median_anchor_keeps_negative_years() {
        // 周期底部负 FCF 年份必须参与（跨周期真实信息）：[-50, 100, 150] 中位数 100；
        // 若错误剔除负值只剩 [100,150]，中位数会变成 125。
        let financials = vec![
            report("2025-12-31", Some(150.0), Some(0.0)),
            report("2024-12-31", Some(100.0), Some(0.0)),
            report("2023-12-31", Some(-50.0), Some(0.0)),
        ];
        let med = cyclical_median_annual_fcf(&financials, 5).expect("3 个年度值");
        assert!((med - 100.0).abs() < 1e-9, "含负值中位数 = 100，实得 {med}");
    }

    #[test]
    fn median_anchor_none_with_insufficient_samples() {
        let only_one = vec![report("2025-12-31", Some(100.0), Some(0.0))];
        assert!(cyclical_median_annual_fcf(&only_one, 5).is_none());
        let none: Vec<FinancialReport> = vec![];
        assert!(cyclical_median_annual_fcf(&none, 5).is_none());
    }
}
