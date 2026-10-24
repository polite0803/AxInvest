// SPDX-License-Identifier: AGPL-3.0-only

//! 股票代码的「形态 ↔ 市场位」归一。
//!
//! 仓内同时流通三种形态：`000001.SH`（分析任务文本与 `to_em_secucode` 产出）、
//! `sh000001`（腾讯系习惯）、`000001`（裸码）。此前每个 vendor 各自
//! `trim_start_matches("sh")` 再**按首位数字推断市场** —— 该推断对股票成立、对指数必错：
//! 上证综指与平安银行同为 `000001`，前者属沪（东财 secid `1.000001`），后者属深。
//! 于是 `sh000001` 在东财会静默取回平安银行的 K 线，在雪球变成 `SZ000001`，
//! 而 `000001.SH` 在腾讯被拼成 `sz000001.SH`（接口回 `param error`）。
//!
//! 判据：**显式市场标记永远优先于首位数字推断**。无标记时仍由调用方按首位推断
//! （那是股票的既有行为，本模块不改）。

/// 显式交易所标记（与 vendor 无关的中立表达）。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Exchange {
    Shanghai,
    Shenzhen,
    Beijing,
}

impl Exchange {
    /// 东方财富 secid 的市场位（沪=1，深/北=0）。
    pub(crate) fn em_market(self) -> &'static str {
        match self {
            Self::Shanghai => "1",
            Self::Shenzhen | Self::Beijing => "0",
        }
    }

    /// 腾讯代码前缀（小写）。
    pub(crate) fn tencent_prefix(self) -> &'static str {
        match self {
            Self::Shanghai => "sh",
            Self::Shenzhen => "sz",
            Self::Beijing => "bj",
        }
    }

    /// 雪球 symbol 前缀（大写）。
    pub(crate) fn xueqiu_tag(self) -> &'static str {
        match self {
            Self::Shanghai => "SH",
            Self::Shenzhen => "SZ",
            Self::Beijing => "BJ",
        }
    }

    /// 通达信（mootdx）市场号。
    pub(crate) fn tdx_market(self) -> u8 {
        match self {
            Self::Shanghai => 1,
            Self::Shenzhen => 0,
            Self::Beijing => 2,
        }
    }
}

fn exchange_from_tag(tag: &str) -> Option<Exchange> {
    if tag.eq_ignore_ascii_case("SH") {
        Some(Exchange::Shanghai)
    } else if tag.eq_ignore_ascii_case("SZ") {
        Some(Exchange::Shenzhen)
    } else if tag.eq_ignore_ascii_case("BJ") {
        Some(Exchange::Beijing)
    } else {
        None
    }
}

/// 裸码市场推断 —— 各 vendor「按首位数字推断」的**单一权威**。
///
/// 北交所 2025 年起把存量老码（83/87 段）批量切换到 920 号段 ⇒ 「9 开头=沪」的
/// 既有假设对 920 段失效：实测 `sz920402`（腾讯）/`SH920402`（雪球）/`1.920402`
/// （东财 secid）全部查空，唯一正确市场是 BJ（东财 `0.920402`）。沪 B（900xxx）
/// 不在 A 股清单枚举域内，维持「9 开头=沪」。
///
/// 与 `axagent_harness::market_data::detect_market_type` 的 A 股分档必须一致
/// （一致性由单测钉住）；返回 `Exchange` 便于调用方按各自 vendor 的记法取前缀。
pub(crate) fn infer_market_bare(code: &str) -> Exchange {
    if code.starts_with("920") {
        Exchange::Beijing
    } else if code.starts_with('6') || code.starts_with('9') {
        Exchange::Shanghai
    } else if code.starts_with('8') || code.starts_with('4') {
        Exchange::Beijing
    } else {
        Exchange::Shenzhen
    }
}

/// 国际指数别名（**不是**美股个股）：取数通道与个股不同，不可当个股处理。
///
/// 与 `vendors/tencent.rs` 的 `TC_INTL_INDEX` / `vendors/international.rs` 的
/// `EM_INTL_INDEX` 是同一批别名的不同落点映射；此处只回答「是不是指数」这一个问题。
const INTL_INDEX_ALIASES: [&str; 7] = ["SPX", "INX", "IXIC", "NDX", "DJI", "DJIA", "HSI"];

/// 美股**个股**代码归一：`AAPL` / `AAPL.US` / `105.AAPL` → `Some("AAPL")`。
///
/// 返回 `None` 覆盖：空串、外汇对（含 `/`）、非纯字母的裸码（A 股/港股数字码）、
/// 超过 5 位、以及国际指数别名。
///
/// 存在的理由：跨市场 K 线兜底需要把「美股个股」与「指数 / 港股」分流到不同源 ——
/// 腾讯 `kline/kline` 对美股个股只回 2 根日线（实测 usAAPL/usTSLA/usMSFT 均为
/// 「2011-06-02 + 最新一日」），只能由新浪美股日线承接。分流判据放这里，
/// 避免 lib 与 sina 各写一份（共 3 张表会把别名集合撕裂）。
pub(crate) fn us_stock_symbol(code: &str) -> Option<String> {
    let t = code.trim();
    if t.is_empty() || t.contains('/') {
        return None;
    }
    // 东财 secid 的美股市场位（105=纳斯达克 / 106=纽交所 / 107=美交所）
    let bare = match t.split_once('.') {
        Some(("105" | "106" | "107", rest)) => rest,
        _ => t,
    };
    let bare = bare.strip_suffix(".US").or_else(|| bare.strip_suffix(".us")).unwrap_or(bare);
    if bare.is_empty() || bare.len() > 5 || !bare.chars().all(|c| c.is_ascii_alphabetic()) {
        return None;
    }
    let upper = bare.to_ascii_uppercase();
    if INTL_INDEX_ALIASES.contains(&upper.as_str()) {
        return None;
    }
    Some(upper)
}

/// 拆出「裸代码 + 显式市场」。识别 `<裸码>.<SH|SZ|BJ>` 后缀与 `<sh|sz|bj><裸码>` 前缀，
/// 大小写均不敏感。
///
/// 返回 `None` 表示输入**没有**显式市场标记（裸 6 位码，或 `116.00700` 这类
/// 市场位已给的东财 secid，或 `00700.HK` / `AAPL.US`），由调用方沿用既有分支。
pub(crate) fn split_explicit_market(code: &str) -> Option<(&str, Exchange)> {
    // 后缀形态：只在点号右侧是交易所标记时才当标记用，
    // 否则会把东财 secid `1.000001` 与 `00700.HK` 误判成带标记形态。
    if let Some((bare, ex)) = code
        .rsplit_once('.')
        .filter(|(bare, _)| !bare.is_empty())
        .and_then(|(bare, tag)| exchange_from_tag(tag).map(|ex| (bare, ex)))
    {
        return Some((bare, ex));
    }
    // 前缀形态：`sh000001` / `SH000001`
    if code.len() > 2 {
        let (head, rest) = code.split_at(2);
        if let Some(ex) = exchange_from_tag(head) {
            return Some((rest, ex));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn suffix_and_prefix_forms_carry_the_same_market() {
        // 上证综指：后缀与前缀都必须落在沪，不能被首位数字「0」拽到深市
        assert_eq!(split_explicit_market("000001.SH"), Some(("000001", Exchange::Shanghai)));
        assert_eq!(split_explicit_market("sh000001"), Some(("000001", Exchange::Shanghai)));
        assert_eq!(split_explicit_market("SH000001"), Some(("000001", Exchange::Shanghai)));
        assert_eq!(split_explicit_market("399006.sz"), Some(("399006", Exchange::Shenzhen)));
        assert_eq!(split_explicit_market("bj430047"), Some(("430047", Exchange::Beijing)));
    }

    #[test]
    fn forms_without_explicit_market_are_left_to_the_caller() {
        // 东财 secid：市场位已显式给出，右侧是数字不是标记
        assert_eq!(split_explicit_market("1.000001"), None);
        assert_eq!(split_explicit_market("0.000001"), None);
        // 港股/美股标记不属于 A 股交易所，留给调用方的 HK/US 分支
        assert_eq!(split_explicit_market("00700.HK"), None);
        assert_eq!(split_explicit_market("AAPL.US"), None);
        // 裸码
        assert_eq!(split_explicit_market("000001"), None);
    }

    #[test]
    fn market_numbering_matches_each_source_convention() {
        assert_eq!(Exchange::Shanghai.em_market(), "1");
        assert_eq!(Exchange::Shenzhen.em_market(), "0");
        assert_eq!(Exchange::Beijing.em_market(), "0");
        assert_eq!(Exchange::Shanghai.tdx_market(), 1);
        assert_eq!(Exchange::Beijing.tdx_market(), 2);
    }

    #[test]
    fn us_stock_symbol_normalizes_the_three_forms() {
        assert_eq!(us_stock_symbol("AAPL"), Some("AAPL".to_string()));
        assert_eq!(us_stock_symbol("AAPL.US"), Some("AAPL".to_string()));
        assert_eq!(us_stock_symbol("aapl.us"), Some("AAPL".to_string()));
        assert_eq!(us_stock_symbol("105.AAPL"), Some("AAPL".to_string()));
        assert_eq!(us_stock_symbol("106.BABA"), Some("BABA".to_string()));
    }

    #[test]
    fn us_stock_symbol_rejects_indices_and_non_us_forms() {
        // 指数不是个股（通道不同：指数走腾讯 `usINX` 一族）
        assert_eq!(us_stock_symbol("SPX"), None);
        assert_eq!(us_stock_symbol("100.SPX"), None);
        assert_eq!(us_stock_symbol("HSI"), None);
        assert_eq!(us_stock_symbol("DJIA"), None);
        // A 股 / 港股 / 外汇 / 超长
        assert_eq!(us_stock_symbol("600519"), None);
        assert_eq!(us_stock_symbol("000001.SH"), None);
        assert_eq!(us_stock_symbol("00700.HK"), None);
        assert_eq!(us_stock_symbol("usr/CNY"), None);
        assert_eq!(us_stock_symbol("TOOLONG"), None);
        assert_eq!(us_stock_symbol(""), None);
    }

    /// 裸码推断与 harness 的 A 股分档逐格对齐（漂移即红）。
    /// 430 段两边口径本就不同（harness 记 neeq / vendor 记 bj），不入这组对照。
    #[test]
    fn bare_inference_agrees_with_harness_market_type() {
        assert_eq!(infer_market_bare("600519"), Exchange::Shanghai);
        assert_eq!(infer_market_bare("000001"), Exchange::Shenzhen);
        assert_eq!(infer_market_bare("300750"), Exchange::Shenzhen);
        assert_eq!(infer_market_bare("830799"), Exchange::Beijing);
        // 920 新号段必须归 BJ（否则 9 开头按沪 B 推断会整段查空）
        assert_eq!(infer_market_bare("920402"), Exchange::Beijing);
        assert_eq!(infer_market_bare("900939"), Exchange::Shanghai);
        assert_eq!(axagent_harness::market_data::detect_market_type("920402"), "bj");
        assert_eq!(axagent_harness::market_data::detect_market_type("900939"), "b_share");
    }
}
