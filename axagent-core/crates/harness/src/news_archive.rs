// SPDX-License-Identifier: AGPL-3.0-only

//! 新闻入库端口（端口在 harness，实现由宿主注入）。
//!
//! 本模块只包含 `NewsArchiveSink` trait 与 DTO 引用，不依赖 dao/entities；
//! 具体落库实现仍在 `src-tauri/src/init/news_archive_sink.rs`，经
//! `with_news_archive_sink` 注入到 astock-data 的持有方。

use crate::stock_market_types::NewsItem;

/// P6: 新闻入库 sink 抽象。
///
/// astock-data 不直接依赖 dao/entities,所以用 trait 抽象把"入库"延迟到
/// main crate 实现并通过 `with_news_archive_sink` 注入。这样:
/// - 保持 astock-data 轻量(避免引入 sea-orm 等大依赖)
/// - 实现可替换(测试时可换 in-memory mock)
/// - 即使 sink 未注入也能正常工作(降级为不写库)
#[async_trait::async_trait]
pub trait NewsArchiveSink: Send + Sync {
    /// 批量 upsert NewsItem。失败仅记录日志,不影响主流程。
    async fn upsert(
        &self,
        source: &str,
        stock_code: Option<&str>,
        keyword: Option<&str>,
        items: &[NewsItem],
    );

    /// as-of 模式查询:`publish_time_ms <= as_of_ts` 的最新 limit 条,
    /// 关键词匹配 title/summary 子串,stock_code 给定时额外过滤。
    /// 返回空 vec 表示无数据(由调用方决定是否降级)。
    async fn search_asof(
        &self,
        keyword: &str,
        stock_code: Option<&str>,
        as_of_ts_ms: i64,
        limit: u32,
    ) -> Vec<NewsItem>;
}
