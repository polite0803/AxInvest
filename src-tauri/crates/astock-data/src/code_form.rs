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
}
