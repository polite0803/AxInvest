// SPDX-License-Identifier: AGPL-3.0-only

//! 编码段盲扫：**只登记代码到 `market_stock_universe`，不产事件**（PLAN C-1 / 妖-3）。
//!
//! 为什么只剩这条路：`clist` 全量刷新与板块枚举两条扩域路径已实测判废（2026-10-19）；
//! 段扫把「建清单」与「出涨幅榜」解耦 —— 每轮只问行情（腾讯批量，正门），拿到名字与价格就登记，
//! **不写 `market_daily_close`、不产漏检事件**。覆盖率随累积上调，面板继续如实写
//! 「漏检域 = 已枚举 N 只，非全市场」。
//!
//! 语义边界：
//! - 登记行 `origin = "code_scan"`、`confirm = false` ⇒ `last_confirmed_at` 保持 0（未经东财全量确认），
//!   面板的「确认 N 只」不会被段扫灌水；行业列也不覆写（`upsert_universe` 的 confirm 分支）。
//! - 分片按 `day_seed % shards` 轮转 ⇒ 每天只扫 1/shards 的段，摊平请求量；同一分片重复跑是幂等的（upsert）。
//! - 指数/ETF/基金/债/权证不得登记：腾讯对 399xxx、sh000300 等也会回报价，只按「6 位数字 + 价格有效」
//!   会把它们当股票写进清单 ⇒ `is_plausible_stock_quote` 是唯一的过滤口（纯函数，带负控）。

use sea_orm::DatabaseConnection;

use crate::market_close_store::upsert_universe_registered;

/// 扫描段：`(起始码, 结束码)`，两端含，数值形式（输出时补足 6 位）。
///
/// 只收 A 股正股所在段：深主板 / 创业板 / 沪主板（含 605）/ 科创板 / 北交所。
/// 北交所含 83x（旧代码）与 87x/92x（新代码段）；刻意**不含** 430xxx（新三板，
/// 非上市股票段）与 399xxx（指数段）。
pub const CODE_SEGMENTS: &[(u32, u32)] = &[
    (1, 3_999),
    (300_000, 301_999),
    (600_000, 603_999),
    (605_000, 605_999),
    (688_000, 688_999),
    (830_000, 839_999),
    (870_000, 879_999),
    (920_000, 920_999),
];

/// 允许登记的板块（`detect_market_type` 的取值域）—— 其余一律不登记。
pub const STOCK_MARKETS: [&str; 5] = ["main_sz", "main_sh", "chinext", "star", "bj"];

/// 名字里出现这些词一律不当股票（腾讯对指数/基金/债也会回报价）。
const NON_STOCK_NAME_PARTS: [&str; 6] = ["指数", "ETF", "基金", "债", "购", "沽"];

/// 会与深市股票段**撞码**的知名指数（沪市指数用 sh 前缀，但我们只按裸码问行情）。
///
/// 这份名单刻意是小而显式的：它只挡「码落在 000xxx 段、名字里又不含指数字样」的常见碰撞；
/// 名字过滤才是主判据。名单不全不会让判据失效（多登记几只指数的后果是清单 size 略高），
/// 但**假阴性是静默的** ⇒ 新增碰撞码时把测试里的负控一并加进来。
const INDEX_CODE_COLLISIONS: [&str; 5] = ["000016", "000300", "000905", "000852", "000688"];

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanStats {
    /// 发出的批量行情请求数（成本读数）
    pub requests: u32,
    /// 请求覆盖的代码数（含不存在的代码）
    pub codes_tried: u32,
    /// 有报价的代码数（存在但不一定可登记）
    pub quotes: u32,
    /// 实际登记/更新的清单行数
    pub registered: u32,
    /// 批量行情失败的批次数（失败批不中断整片，下一批继续；下轮会重扫该段）
    pub failed_batches: u32,
}

/// 段内全部代码，补足 6 位（`1` → `000001`）。
pub fn codes_in_segment(start: u32, end: u32) -> Vec<String> {
    (start..=end).map(|n| format!("{n:06}")).collect()
}

/// 第 `shard` 片（共 `shards` 片）负责的段 —— 按段下标轮转分配。
///
/// 判据：所有分片并起来恰好是全部段、且互不重叠（由单测钉住）。分片数 > 段数时
/// 后面的分片为空（不是错误：调用方按日轮转，空片只是那天不扫）。
pub fn segments_for_shard(shard: u32, shards: u32) -> Vec<(u32, u32)> {
    let shards = shards.max(1) as usize;
    CODE_SEGMENTS
        .iter()
        .enumerate()
        .filter(|(i, _)| i % shards == (shard as usize) % shards)
        .map(|(_, seg)| *seg)
        .collect()
}

/// 当天应扫的分片号（按 UTC 日序号轮转，调用方不必自己算）。
pub fn shard_for_day(day_index: i64, shards: u32) -> u32 {
    (day_index.rem_euclid(shards.max(1) as i64)) as u32
}

/// 报价是否可能是一只**可登记的 A 股正股**。
///
/// 负控都在单测里：指数（000300 走 sz 前缀）、ETF/基金、债券、零价、空名、6 位以外一律 false。
pub fn is_plausible_stock_quote(code: &str, name: &str, price: f64, pre_close: f64) -> bool {
    if code.len() != 6 || !code.chars().all(|c| c.is_ascii_digit()) {
        return false;
    }
    if !(price.is_finite() && pre_close.is_finite()) || price <= 0.0 || pre_close <= 0.0 {
        return false;
    }
    if INDEX_CODE_COLLISIONS.contains(&code) {
        return false;
    }
    let n = name.trim();
    if n.is_empty() || NON_STOCK_NAME_PARTS.iter().any(|k| n.contains(k)) {
        return false;
    }
    STOCK_MARKETS.contains(&axagent_harness::market_data::detect_market_type(code))
}

/// 扫一个分片：逐段按 `batch_size` 分批请求，把「像股票」的报价登记进清单。
///
/// 只登记、不出事件：不写收盘快照，因此这些代码要等到**下一次全量收盘采集**才会进入涨幅域。
pub async fn scan_segment_shard(
    db: &DatabaseConnection,
    client: &axagent_astock_data::AStockClient,
    shard: u32,
    shards: u32,
    batch_size: usize,
) -> Result<ScanStats, String> {
    let mut stats = ScanStats::default();
    let now_ms = chrono::Utc::now().timestamp_millis();
    let batch_size = batch_size.max(1);
    for (start, end) in segments_for_shard(shard, shards) {
        let codes = codes_in_segment(start, end);
        for chunk in codes.chunks(batch_size) {
            stats.requests += 1;
            stats.codes_tried += chunk.len() as u32;
            let quotes = match client.market_snapshot_batch(chunk).await {
                Ok(q) => q,
                Err(e) => {
                    // 单批失败不中断整片：旧实现用 `?` 会把这一批之后的代码全部丢掉，
                    // 而段扫是每天轮转，下一次要等一整轮才回来。
                    stats.failed_batches += 1;
                    tracing::warn!(
                        "[universe-scan] 批量行情失败（段 {start:06}-{end:06}，{} 只）: {e}",
                        chunk.len()
                    );
                    continue;
                },
            };
            stats.quotes += quotes.len() as u32;
            let entries: Vec<(String, String, Option<String>)> = quotes
                .iter()
                .filter(|q| is_plausible_stock_quote(&q.code, &q.name, q.price, q.pre_close))
                .map(|q| (q.code.clone(), q.name.clone(), None))
                .collect();
            if entries.is_empty() {
                continue;
            }
            stats.registered +=
                upsert_universe_registered(db, &entries, "code_scan", now_ms).await?;
        }
    }
    Ok(stats)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 段内代码必须补足 6 位（`1` → `000001`），且长度与区间宽度一致。
    #[test]
    fn codes_are_zero_padded_six_digits() {
        let codes = codes_in_segment(430_000, 430_002);
        assert_eq!(codes, vec!["430000", "430001", "430002"]);
        let first = codes_in_segment(1, 1);
        assert_eq!(first, vec!["000001"]);
        assert!(codes_in_segment(830_000, 830_000)[0].starts_with('8'));
    }

    /// 分片必须**无重叠、全覆盖**：把所有分片并起来正好是全部段（漏一段＝那段的票永远登不上）。
    #[test]
    fn shards_partition_all_segments_without_overlap() {
        for shards in 1..=8u32 {
            let mut all: Vec<(u32, u32)> = Vec::new();
            for shard in 0..shards {
                all.extend(segments_for_shard(shard, shards));
            }
            all.sort_unstable();
            let mut expect: Vec<(u32, u32)> = CODE_SEGMENTS.to_vec();
            expect.sort_unstable();
            assert_eq!(all, expect, "shards={shards} 的分片并集必须等于全部段");
        }
    }

    /// 日轮转：同一天同一分片，连续 `shards` 天覆盖全部分片。
    #[test]
    fn shard_for_day_rotates_and_wraps() {
        let seen: std::collections::BTreeSet<u32> =
            (0..3).map(|d| shard_for_day(17_000 + d, 3)).collect();
        assert_eq!(seen.len(), 3, "三天必须轮完三个分片");
        assert_eq!(shard_for_day(-1, 3), 2, "负数取模必须落在值域内");
    }

    /// 正控：正常 A 股报价（含北交所 8 开头、创业板 3 开头）都要认。
    #[test]
    fn plausible_quotes_are_accepted() {
        assert!(is_plausible_stock_quote("002696", "百洋股份", 12.3, 11.2));
        assert!(is_plausible_stock_quote("600519", "贵州茅台", 1500.0, 1490.0));
        assert!(is_plausible_stock_quote("300750", "宁德时代", 200.0, 198.0));
        assert!(is_plausible_stock_quote("688981", "中芯国际", 50.0, 49.0));
        assert!(is_plausible_stock_quote("830799", "艾融软件", 10.0, 9.9));
        // 北交所 87x/92x 新代码段：detect_market_type 必须归 bj，不能被 9 开头误判成 B 股
        assert!(is_plausible_stock_quote("870001", "北交样本", 10.0, 9.9));
        assert!(is_plausible_stock_quote("920001", "北交样本", 10.0, 9.9));
    }

    /// 负控：指数 / 基金 / 债 / 零价 / 空名 / 非 6 位一律不得登记（否则清单被非股票灌水）。
    #[test]
    fn non_stock_quotes_are_rejected() {
        assert!(!is_plausible_stock_quote("000300", "沪深300", 4000.0, 3980.0));
        assert!(!is_plausible_stock_quote("510300", "沪深300ETF", 4.0, 3.98));
        assert!(!is_plausible_stock_quote("113050", "南银转债", 110.0, 109.0));
        assert!(!is_plausible_stock_quote("002696", "百洋股份", 0.0, 11.2));
        assert!(!is_plausible_stock_quote("002696", "百洋股份", f64::NAN, 11.2));
        assert!(!is_plausible_stock_quote("002696", "   ", 12.3, 11.2));
        assert!(!is_plausible_stock_quote("00269", "百洋股份", 12.3, 11.2));
        assert!(!is_plausible_stock_quote("430047", "诺思兰德", 10.0, 9.8), "新三板段不登记");
        assert!(!is_plausible_stock_quote("900901", "云赛B股", 1.0, 0.98), "B 股不登记");
    }
}
