// SPDX-License-Identifier: AGPL-3.0-only

//! 市场数据契约层 — 纯 DTO + Trait 抽象
//!
//! 让 `quant` / `gateway` 等消费者通过 trait 调用数据源，
//! 无需直接依赖 `axagent-astock-data` 实现。

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use ts_rs::TS;

use crate::as_of::AsOfContext;
use crate::core_error::{AxAgentError, Result};
use crate::data_outcome::DataOutcome;
use crate::stock_market_types::{
    Announcement, BlockTrade, BoardMember, ClsFlashItem, ConceptBlocks, ConceptBoard, ConsensusEPS,
    DividendRecord, DragonTigerEntry, EarningsEvent, HolderCount, HotStock, IndexQuote,
    IndustryRank, LimitUpPool, LockupSchedule, MarginData, MarketDragonTiger, MarketListPage,
    MoneyFlow, NewsItem, NorthBoundFlow, NorthBoundHolding, OptionPCR, PeerComparison, PledgeData,
    ResearchReport, SectorInfo, SocialSentiment,
};

// ── DTOs ─────────────────────────────────────────────────────────────────

/// 实时行情
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct StockQuote {
    pub code: String,
    pub name: String,
    pub price: f64,
    /// 昨收价
    pub pre_close: f64,
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub volume: f64,
    pub amount: f64,
    pub change_pct: f64,
    pub turnover_rate: f64,
    pub pe: Option<f64>,
    pub pb: Option<f64>,
    pub total_mv: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub circulating_mv: Option<f64>,
    /// 涨停价
    ///
    /// `default` 是给 ts-rs 看的：它只在 `default` 存在时把字段判为可选（`?:`），
    /// 单有 `skip_serializing_if` 仍生成必填 —— 而线上载荷确实可能缺这个键。
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub limit_up: Option<f64>,
    /// 跌停价
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub limit_down: Option<f64>,
    /// 是否ST股票（含*ST）
    #[serde(default)]
    pub is_st: bool,
    pub timestamp: String,
}

/// K线数据
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct KLine {
    pub date: String,
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
    pub volume: f64,
    pub amount: f64,
    pub turnover_rate: Option<f64>,
    /// 累计复权因子 (R3-A); None 表示未应用复权
    #[serde(default)]
    pub adj_factor: Option<f64>,
}

/// 复权类型
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum AdjType {
    None,
    #[default]
    Forward,
    Backward,
}

/// 股票搜索结果
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct StockSearchResult {
    pub code: String,
    pub name: String,
    pub market: String,
}

/// 财务报告 DTO
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FinancialReport {
    pub stock_code: String,
    pub report_date: String,
    pub revenue: Option<f64>,
    pub net_profit: Option<f64>,
    pub eps: Option<f64>,
    pub bps: Option<f64>,
    pub roe: Option<f64>,
    pub debt_ratio: Option<f64>,
    pub gross_margin: Option<f64>,
    pub net_margin: Option<f64>,
    pub revenue_yoy: Option<f64>,
    pub profit_yoy: Option<f64>,
    #[serde(default)]
    pub total_assets: Option<f64>,
    #[serde(default)]
    pub operating_cash_flow: Option<f64>,
    #[serde(default)]
    pub capital_expenditure: Option<f64>,
    #[serde(default)]
    pub free_cash_flow: Option<f64>,
    #[serde(default)]
    pub current_ratio: Option<f64>,
    #[serde(default)]
    pub quick_ratio: Option<f64>,
    /// #8 修复(2026-07-22): 新增商誉字段(资产负债表科目)
    /// 用于商誉占比风险评估(goodwill / total_assets)。
    /// 数据来源:东方财富 ZcfzbAjaxNew 资产负债表接口。
    #[serde(default)]
    pub goodwill: Option<f64>,
    /// #8 修复(2026-07-22): 新增应收账款字段(资产负债表科目)
    /// 用于应收账款风险评估(accounts_receivable / revenue)。
    /// 数据来源:东方财富 ZcfzbAjaxNew 资产负债表接口。
    #[serde(default)]
    pub accounts_receivable: Option<f64>,
    /// 标记该报告是否为估值/行业均值 fallback（非真实财报数据）
    #[serde(default)]
    pub estimated: Option<bool>,
    /// 该期报告的**实际披露（公告）日** `YYYY-MM-DD`。
    ///
    /// vendor 自证时填此字段；多数财务接口只回报告期不回公告日 ⇒ 留 `None`，
    /// 由 [`FinancialReport::effective_disclosure_date`] 按 A 股法定披露截止日保守推断。
    /// 为什么必须有它：as-of 回放按 `report_date` 截断会让「报告期 9-30、10 月底才披露」
    /// 的财报在 10-01 就可见 = 前视泄露（P1-1，2026-10-03）。
    #[serde(default)]
    pub disclosure_date: Option<String>,
}

impl FinancialReport {
    /// 检查该记录是否包含有效的核心财务数据
    pub fn has_valid_data(&self) -> bool {
        self.revenue.is_some()
            || self.net_profit.is_some()
            || self.eps.is_some()
            || self.bps.is_some()
            || self.roe.is_some()
            || self.debt_ratio.is_some()
            || self.gross_margin.is_some()
            || self.net_margin.is_some()
            || self.revenue_yoy.is_some()
            || self.profit_yoy.is_some()
    }

    /// 该期报告在 A 股规则下的**法定披露截止日**（定期报告：季报 +2 个月、半年报 +2 个月、
    /// 年报次年 4-30）。只认四类期末，其它报告期返回 `None` —— 不猜。
    pub fn statutory_disclosure_deadline(&self) -> Option<String> {
        let d = self.report_date.get(..10)?;
        let b = d.as_bytes();
        if b.len() != 10 || b[4] != b'-' || b[7] != b'-' {
            return None;
        }
        let year: i32 = d.get(0..4)?.parse().ok()?;
        let md = d.get(5..10)?;
        let (y, rest) = match md {
            "03-31" => (year, "04-30"),
            "06-30" => (year, "08-31"),
            "09-30" => (year, "10-31"),
            "12-31" => (year + 1, "04-30"),
            _ => return None,
        };
        Some(format!("{y:04}-{rest}"))
    }

    /// as-of 判「截止日时该期数据是否已公开」所用的披露日。
    ///
    /// `(日期, 来源)`：vendor 自证 ⇒ `reported`；只有报告期 ⇒ 按法定截止日推断 ⇒ `estimated`；
    /// 两者都拿不到 ⇒ `None` ⇒ 调用方在回放里**必须丢弃**，不得当成「已公开」。
    pub fn effective_disclosure_date(&self) -> Option<(String, &'static str)> {
        if let Some(dd) =
            self.disclosure_date.as_deref().and_then(|s| s.get(..10)).filter(|s| !s.is_empty())
        {
            return Some((dd.to_string(), "reported"));
        }
        self.statutory_disclosure_deadline().map(|d| (d, "estimated"))
    }
}

// ── 批量查询 DTO（P6.1 迁移：自 astock-data::batch 上收，consumer 只能依赖 harness）──

/// 批量查询结果信封 — `MarketDataProvider::{batch_get_quotes, batch_get_financials}` 的返回契约。
///
/// 逐 code 归因：`success`/`failures` 双 map；**不保证**两者之和等于入参数量
/// （提前终止、或单只取数 task panic 时该 code 两边都不出现——与迁移前宿主命令逐字节兼容）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BatchResult<T> {
    pub success: HashMap<String, T>,
    pub failures: HashMap<String, String>,
    pub elapsed_ms: u64,
    /// 是否因失败数超过 `max_failures` 阈值而提前终止
    /// （true 表示还有未完成的任务被丢弃；false 表示所有任务都跑完）。
    #[serde(default)]
    pub early_terminated: bool,
}

impl<T> BatchResult<T> {
    pub fn new() -> Self {
        Self {
            success: HashMap::new(),
            failures: HashMap::new(),
            elapsed_ms: 0,
            early_terminated: false,
        }
    }

    pub fn total(&self) -> usize {
        self.success.len() + self.failures.len()
    }

    pub fn is_all_success(&self) -> bool {
        self.failures.is_empty()
    }
}

impl<T> Default for BatchResult<T> {
    fn default() -> Self {
        Self::new()
    }
}

/// `batch_get_quotes` 的请求参数（原宿主命令 `BatchQuotesRequest` 的强类型上收形态）。
///
/// 字段全 camelCase 序列化，与原宿主命令入参逐字节兼容；`None` 一律落到 provider 端默认策略。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct MarketBatchQuotesRequest {
    pub codes: Vec<String>,
    /// 单只取数超时（毫秒）；`None` ⇒ provider 默认（quotes 8s）。
    pub per_stock_timeout_ms: Option<u64>,
    /// 整体超时（毫秒）。**行为口径与迁移前一致：接受但不生效**——原
    /// `BatchRunner::run_batch` 从未强制 `total_timeout`，此处保持同样的惰性字段，
    /// 不做「顺手修复」，以免改变既有调用方的可观测行为。
    pub total_timeout_ms: Option<u64>,
    /// 失败数超过该阈值即提前终止；`None`/`0` ⇒ 不提前终止、收集全部失败。
    pub max_failures: Option<usize>,
}

// ── MarketDataProvider Trait ─────────────────────────────────────────────

/// 市场数据提供者接口
///
/// 实现方：`axagent-astock-data` 的 `AStockClient`
/// 业务实现层（implementor）：`stock-analysis`（依赖 astock-data + dao + entities，属 implementor，非 consumer）
/// 消费者（consumer）：`quant`、`gateway`、`tools`
#[async_trait]
pub trait MarketDataProvider: Send + Sync {
    /// 获取实时行情（含涨跌停价、ST标记）
    async fn get_quote(&self, stock_code: &str) -> Result<StockQuote>;

    /// 获取K线数据
    ///
    /// - `adj_type`: `Some(Forward)` 前复权 / `Some(Backward)` 后复权 / `None` 不复权
    async fn get_klines(
        &self,
        stock_code: &str,
        period: &str,
        limit: u32,
        adj_type: Option<AdjType>,
    ) -> Result<Vec<KLine>>;

    /// 搜索股票
    async fn search_stock(&self, keyword: &str) -> Result<Vec<StockSearchResult>>;

    // ── 个股数据面（P6.1 A 批：全部带默认「不支持」实现）────────────────
    // 每个方法的末参 `as_of`：`None` 表示实盘（live）；`Some` 表示按该日期
    // 截止（as-of 回放）。跨进程后插件读不到宿主 task_local，必须显式透传。

    /// 获取个股财务报告序列（按报告期倒序由实现方决定）。
    ///
    /// `as_of = None` 为实盘，`Some` 则按披露日截止回放；空 `Vec` 表示无报告数据。
    async fn get_financials(
        &self,
        _stock_code: &str,
        _as_of: Option<AsOfContext>,
    ) -> Result<Vec<FinancialReport>> {
        Err(AxAgentError::Provider(
            "MarketDataProvider::get_financials 未实现（该提供者不支持此数据面）".into(),
        ))
    }

    /// 获取个股最近一日资金流。
    ///
    /// `as_of = None` 为实盘，`Some` 按该日截止回放；`Ok(None)` 表示业务上无该数据，非错误。
    async fn get_money_flow(
        &self,
        _stock_code: &str,
        _as_of: Option<AsOfContext>,
    ) -> Result<Option<MoneyFlow>> {
        Err(AxAgentError::Provider(
            "MarketDataProvider::get_money_flow 未实现（该提供者不支持此数据面）".into(),
        ))
    }

    /// 获取个股北向持股快照。
    ///
    /// `as_of = None` 为实盘，`Some` 按该日截止回放；`Ok(None)` 表示无持仓数据，非错误。
    async fn get_north_bound_holding(
        &self,
        _stock_code: &str,
        _as_of: Option<AsOfContext>,
    ) -> Result<Option<NorthBoundHolding>> {
        Err(AxAgentError::Provider(
            "MarketDataProvider::get_north_bound_holding 未实现（该提供者不支持此数据面）".into(),
        ))
    }

    /// 获取**个股**龙虎榜记录（全市场榜单不在此接口内）。
    ///
    /// `as_of = None` 为实盘，`Some` 按该日截止回放；空 `Vec` 表示未上榜或无数据。
    async fn get_dragon_tiger(
        &self,
        _stock_code: &str,
        _as_of: Option<AsOfContext>,
    ) -> Result<Vec<DragonTigerEntry>> {
        Err(AxAgentError::Provider(
            "MarketDataProvider::get_dragon_tiger 未实现（该提供者不支持此数据面）".into(),
        ))
    }

    /// 获取个股近 `days` 个交易日的融资融券余额序列。
    ///
    /// `as_of = None` 为实盘，`Some` 按该日截止回放；空 `Vec` 表示无序列数据。
    async fn get_margin_balance_trend(
        &self,
        _stock_code: &str,
        _days: u32,
        _as_of: Option<AsOfContext>,
    ) -> Result<Vec<MarginData>> {
        Err(AxAgentError::Provider(
            "MarketDataProvider::get_margin_balance_trend 未实现（该提供者不支持此数据面）".into(),
        ))
    }

    /// 获取个股最新一期融资融券数据。
    ///
    /// `as_of = None` 为实盘，`Some` 按该日截止回放；`Ok(None)` 表示非两融标的或无数据，非错误。
    async fn get_margin_data(
        &self,
        _stock_code: &str,
        _as_of: Option<AsOfContext>,
    ) -> Result<Option<MarginData>> {
        Err(AxAgentError::Provider(
            "MarketDataProvider::get_margin_data 未实现（该提供者不支持此数据面）".into(),
        ))
    }

    /// 获取个股股东户数（最新一期）。
    ///
    /// `as_of = None` 为实盘，`Some` 按公告日截止回放；`Ok(None)` 表示截止日前无披露，非错误。
    async fn get_holder_count(
        &self,
        _stock_code: &str,
        _as_of: Option<AsOfContext>,
    ) -> Result<Option<HolderCount>> {
        Err(AxAgentError::Provider(
            "MarketDataProvider::get_holder_count 未实现（该提供者不支持此数据面）".into(),
        ))
    }

    /// 获取个股大宗交易记录。
    ///
    /// `as_of = None` 为实盘，`Some` 按该日截止回放；空 `Vec` 表示无大宗交易数据。
    async fn get_block_trades(
        &self,
        _stock_code: &str,
        _as_of: Option<AsOfContext>,
    ) -> Result<Vec<BlockTrade>> {
        Err(AxAgentError::Provider(
            "MarketDataProvider::get_block_trades 未实现（该提供者不支持此数据面）".into(),
        ))
    }

    /// 获取个股股权质押数据。
    ///
    /// `as_of = None` 为实盘，`Some` 按该日截止回放；`Ok(None)` 表示无质押数据，非错误。
    async fn get_pledge_data(
        &self,
        _stock_code: &str,
        _as_of: Option<AsOfContext>,
    ) -> Result<Option<PledgeData>> {
        Err(AxAgentError::Provider(
            "MarketDataProvider::get_pledge_data 未实现（该提供者不支持此数据面）".into(),
        ))
    }

    /// 获取个股所属板块/行业信息。
    ///
    /// `as_of = None` 为实盘，`Some` 按该日截止回放；`Ok(None)` 表示无板块归属，非错误。
    async fn get_sector_info(
        &self,
        _stock_code: &str,
        _as_of: Option<AsOfContext>,
    ) -> Result<Option<SectorInfo>> {
        Err(AxAgentError::Provider(
            "MarketDataProvider::get_sector_info 未实现（该提供者不支持此数据面）".into(),
        ))
    }

    /// 获取个股新闻，最多 `limit` 条。
    ///
    /// `as_of = None` 为实盘，`Some` 按发布时间截止回放；空 `Vec` 表示无新闻数据。
    async fn get_news(
        &self,
        _stock_code: &str,
        _limit: u32,
        _as_of: Option<AsOfContext>,
    ) -> Result<Vec<NewsItem>> {
        Err(AxAgentError::Provider(
            "MarketDataProvider::get_news 未实现（该提供者不支持此数据面）".into(),
        ))
    }

    /// 批量获取多只股票的最新行情快照。
    ///
    /// `as_of = None` 为实盘，`Some` 按该日截止回放；空 `Vec` 表示无可用快照（不保证与入参等长）。
    async fn market_snapshot_batch(
        &self,
        _codes: &[String],
        _as_of: Option<AsOfContext>,
    ) -> Result<Vec<StockQuote>> {
        Err(AxAgentError::Provider(
            "MarketDataProvider::market_snapshot_batch 未实现（该提供者不支持此数据面）".into(),
        ))
    }

    /// 逐只并发批量拉实时行情：带单只超时、失败归因与 vendor 健康度自适应并发。
    ///
    /// 与 `market_snapshot_batch`（单请求整体快照、无逐 code 失败归因）**不等价**，本面
    /// 返回 `BatchResult` 双 map。`as_of = None` 为实盘，`Some` 按该日截止回放。
    async fn batch_get_quotes(
        &self,
        _request: &MarketBatchQuotesRequest,
        _as_of: Option<AsOfContext>,
    ) -> Result<BatchResult<StockQuote>> {
        Err(AxAgentError::Provider(
            "MarketDataProvider::batch_get_quotes 未实现（该提供者不支持此数据面）".into(),
        ))
    }

    /// 逐只并发批量拉财务报告序列。
    ///
    /// 失败归因与并发策略同 `batch_get_quotes`；`as_of = None` 为实盘，`Some` 按披露
    /// 日截止过滤回放。
    async fn batch_get_financials(
        &self,
        _codes: &[String],
        _as_of: Option<AsOfContext>,
    ) -> Result<BatchResult<Vec<FinancialReport>>> {
        Err(AxAgentError::Provider(
            "MarketDataProvider::batch_get_financials 未实现（该提供者不支持此数据面）".into(),
        ))
    }

    // ── 剩余数据面（P6.1 C 批：全部带默认「不支持」实现）────────────────
    // 与 A 批同口径：末参 `as_of` 显式透传（`None` = 实盘，`Some` = 按该日截止回放）。

    /// 按关键词搜索新闻，最多 `limit` 条。
    async fn search_news(
        &self,
        _keyword: &str,
        _limit: u32,
        _as_of: Option<AsOfContext>,
    ) -> Result<Vec<NewsItem>> {
        Err(AxAgentError::Provider(
            "MarketDataProvider::search_news 未实现（该提供者不支持此数据面）".into(),
        ))
    }

    /// 按关键词搜索概念板块。
    async fn search_concept_boards(
        &self,
        _keyword: &str,
        _as_of: Option<AsOfContext>,
    ) -> Result<Vec<ConceptBoard>> {
        Err(AxAgentError::Provider(
            "MarketDataProvider::search_concept_boards 未实现（该提供者不支持此数据面）".into(),
        ))
    }

    /// 获取个股社交舆情（股吧/雪球热度）。
    async fn get_social_sentiment(
        &self,
        _stock_code: &str,
        _as_of: Option<AsOfContext>,
    ) -> Result<Vec<SocialSentiment>> {
        Err(AxAgentError::Provider(
            "MarketDataProvider::get_social_sentiment 未实现（该提供者不支持此数据面）".into(),
        ))
    }

    /// 获取个股研报列表。
    async fn get_research_reports(
        &self,
        _stock_code: &str,
        _as_of: Option<AsOfContext>,
    ) -> Result<Vec<ResearchReport>> {
        Err(AxAgentError::Provider(
            "MarketDataProvider::get_research_reports 未实现（该提供者不支持此数据面）".into(),
        ))
    }

    /// 获取同行业可比公司估值。
    async fn get_peers(
        &self,
        _stock_code: &str,
        _as_of: Option<AsOfContext>,
    ) -> Result<Vec<PeerComparison>> {
        Err(AxAgentError::Provider(
            "MarketDataProvider::get_peers 未实现（该提供者不支持此数据面）".into(),
        ))
    }

    /// 获取期权 PCR（看跌/看涨比率）；`Ok(None)` 表示非期权标的或无数据。
    async fn get_option_pcr(
        &self,
        _stock_code: &str,
        _as_of: Option<AsOfContext>,
    ) -> Result<Option<OptionPCR>> {
        Err(AxAgentError::Provider(
            "MarketDataProvider::get_option_pcr 未实现（该提供者不支持此数据面）".into(),
        ))
    }

    /// 获取北向资金分钟级流向；`Ok(None)` 表示无数据。
    async fn get_north_bound_flow(
        &self,
        _as_of: Option<AsOfContext>,
    ) -> Result<Option<NorthBoundFlow>> {
        Err(AxAgentError::Provider(
            "MarketDataProvider::get_north_bound_flow 未实现（该提供者不支持此数据面）".into(),
        ))
    }

    /// 获取全市场龙虎榜。
    async fn get_market_dragon_tiger(
        &self,
        _as_of: Option<AsOfContext>,
    ) -> Result<Vec<MarketDragonTiger>> {
        Err(AxAgentError::Provider(
            "MarketDataProvider::get_market_dragon_tiger 未实现（该提供者不支持此数据面）".into(),
        ))
    }

    /// 获取个股限售解禁安排。
    async fn get_lockup_schedule(
        &self,
        _stock_code: &str,
        _as_of: Option<AsOfContext>,
    ) -> Result<Vec<LockupSchedule>> {
        Err(AxAgentError::Provider(
            "MarketDataProvider::get_lockup_schedule 未实现（该提供者不支持此数据面）".into(),
        ))
    }

    /// 获取涨停池；`requested_date = None` 表示取当下（回放时按截止日）。
    async fn get_limit_up_pool(
        &self,
        _requested_date: Option<&str>,
        _as_of: Option<AsOfContext>,
    ) -> Result<Option<LimitUpPool>> {
        Err(AxAgentError::Provider(
            "MarketDataProvider::get_limit_up_pool 未实现（该提供者不支持此数据面）".into(),
        ))
    }

    /// 获取港股/美股行情。
    async fn get_international_quote(
        &self,
        _stock_code: &str,
        _as_of: Option<AsOfContext>,
    ) -> Result<StockQuote> {
        Err(AxAgentError::Provider(
            "MarketDataProvider::get_international_quote 未实现（该提供者不支持此数据面）".into(),
        ))
    }

    /// 获取港股/美股 K 线。
    async fn get_international_klines(
        &self,
        _stock_code: &str,
        _period: &str,
        _limit: u32,
        _adj_type: Option<AdjType>,
        _as_of: Option<AsOfContext>,
    ) -> Result<Vec<KLine>> {
        Err(AxAgentError::Provider(
            "MarketDataProvider::get_international_klines 未实现（该提供者不支持此数据面）".into(),
        ))
    }

    /// 获取行业涨跌排名。
    async fn get_industry_ranking(&self, _as_of: Option<AsOfContext>) -> Result<Vec<IndustryRank>> {
        Err(AxAgentError::Provider(
            "MarketDataProvider::get_industry_ranking 未实现（该提供者不支持此数据面）".into(),
        ))
    }

    /// 获取大盘指数行情（带成因信封）。
    async fn get_index_quotes_outcome(
        &self,
        _as_of: Option<AsOfContext>,
    ) -> Result<DataOutcome<Vec<IndexQuote>>> {
        Err(AxAgentError::Provider(
            "MarketDataProvider::get_index_quotes_outcome 未实现（该提供者不支持此数据面）".into(),
        ))
    }

    /// 获取热股榜。
    async fn get_hot_stocks(&self, _as_of: Option<AsOfContext>) -> Result<Vec<HotStock>> {
        Err(AxAgentError::Provider(
            "MarketDataProvider::get_hot_stocks 未实现（该提供者不支持此数据面）".into(),
        ))
    }

    /// 获取外汇 K 线。
    async fn get_forex_klines(
        &self,
        _pair: &str,
        _period: &str,
        _limit: u32,
        _as_of: Option<AsOfContext>,
    ) -> Result<Vec<KLine>> {
        Err(AxAgentError::Provider(
            "MarketDataProvider::get_forex_klines 未实现（该提供者不支持此数据面）".into(),
        ))
    }

    /// 获取个股财报披露日历。
    async fn get_earnings_calendar(
        &self,
        _stock_code: &str,
        _as_of: Option<AsOfContext>,
    ) -> Result<Vec<EarningsEvent>> {
        Err(AxAgentError::Provider(
            "MarketDataProvider::get_earnings_calendar 未实现（该提供者不支持此数据面）".into(),
        ))
    }

    /// 获取个股除权除息记录。
    async fn get_dividend_records(
        &self,
        _stock_code: &str,
        _as_of: Option<AsOfContext>,
    ) -> Result<Vec<DividendRecord>> {
        Err(AxAgentError::Provider(
            "MarketDataProvider::get_dividend_records 未实现（该提供者不支持此数据面）".into(),
        ))
    }

    /// 获取机构一致预期 EPS；`Ok(None)` 表示无数据。
    async fn get_consensus_eps(
        &self,
        _stock_code: &str,
        _as_of: Option<AsOfContext>,
    ) -> Result<Option<ConsensusEPS>> {
        Err(AxAgentError::Provider(
            "MarketDataProvider::get_consensus_eps 未实现（该提供者不支持此数据面）".into(),
        ))
    }

    /// 获取概念板块成分股。
    async fn get_concept_board_members(
        &self,
        _board_code: &str,
        _as_of: Option<AsOfContext>,
    ) -> Result<Vec<BoardMember>> {
        Err(AxAgentError::Provider(
            "MarketDataProvider::get_concept_board_members 未实现（该提供者不支持此数据面）".into(),
        ))
    }

    /// 获取个股概念板块三维归属；`Ok(None)` 表示无归属数据。
    async fn get_concept_blocks(
        &self,
        _stock_code: &str,
        _as_of: Option<AsOfContext>,
    ) -> Result<Option<ConceptBlocks>> {
        Err(AxAgentError::Provider(
            "MarketDataProvider::get_concept_blocks 未实现（该提供者不支持此数据面）".into(),
        ))
    }

    /// 获取财联社快讯。
    async fn get_cls_flash(&self, _as_of: Option<AsOfContext>) -> Result<Vec<ClsFlashItem>> {
        Err(AxAgentError::Provider(
            "MarketDataProvider::get_cls_flash 未实现（该提供者不支持此数据面）".into(),
        ))
    }

    /// 获取基准指数 K 线（标普 500 / 纳指 / 恒生 / 上证等）。
    async fn get_benchmark_klines(
        &self,
        _benchmark_code: &str,
        _period: &str,
        _limit: u32,
        _as_of: Option<AsOfContext>,
    ) -> Result<Vec<KLine>> {
        Err(AxAgentError::Provider(
            "MarketDataProvider::get_benchmark_klines 未实现（该提供者不支持此数据面）".into(),
        ))
    }

    /// 获取个股公告。
    async fn get_announcements(
        &self,
        _stock_code: &str,
        _as_of: Option<AsOfContext>,
    ) -> Result<Vec<Announcement>> {
        Err(AxAgentError::Provider(
            "MarketDataProvider::get_announcements 未实现（该提供者不支持此数据面）".into(),
        ))
    }

    /// 获取全市场清单单页（100 只/页，`total` 为服务端自报总数）。
    async fn market_list_page(
        &self,
        _page: u32,
        _as_of: Option<AsOfContext>,
    ) -> Result<MarketListPage> {
        Err(AxAgentError::Provider(
            "MarketDataProvider::market_list_page 未实现（该提供者不支持此数据面）".into(),
        ))
    }
}

// ── MarketDataStreamer Trait (P3: WebSocket 升级架构占位) ────────────────

/// 行情变更事件（流式推送的载荷）
///
/// 与 `axagent_astock_data::realtime_quote::QuoteChangeEvent` 的语义一致,
/// 但放在 harness 以便 consumer crate（gateway/quant）不依赖 astock-data。
/// astock-data 实现层负责把内部 `QuoteChangeEvent` 转换为本类型。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QuoteUpdate {
    pub stock_code: String,
    pub current: StockQuote,
    /// 当日涨跌幅（相对于前收盘）
    pub change_pct: f64,
    /// 触发类型: "tick" | "price_change" | "significant_move"
    pub trigger: String,
    /// 数据源类型: "http_poll" | "websocket"
    pub source: String,
    pub timestamp_ms: i64,
}

/// 市场数据流式推送接口（P3: WebSocket 升级架构占位）
///
/// 与 `MarketDataProvider` 共存：
/// - `MarketDataProvider`：同步请求-响应（适合按需查询）
/// - `MarketDataStreamer`：异步流式推送（适合实时监控、ws 转发）
///
/// 实现方：
/// - `HttpPollingStreamer`（astock-data）：用 tokio::interval 轮询 `AStockClient`，
///   生成 `QuoteUpdate` 流。当前默认数据源。
/// - `WebSocketStreamer`（未来）：连接东方财富/新浪等 WS 接口，推送真实 tick。
///   架构占位，尚未实现（A 股免费 WS 数据源不稳定，需要供应商调研）。
///
/// 消费者：gateway（ws 转发）、RealtimeMonitor（监控循环）。
#[async_trait]
pub trait MarketDataStreamer: Send + Sync {
    /// 订阅指定股票的行情更新流
    ///
    /// 返回的 stream 会持续推送 `QuoteUpdate`，直到调用方 drop。
    /// 实现方负责内部去重（仅在价格/成交量变化时推送）。
    async fn subscribe(
        &self,
        codes: Vec<String>,
    ) -> Result<tokio::sync::mpsc::Receiver<QuoteUpdate>>;

    /// 数据源类型标识（"http_poll" / "websocket"），用于日志和监控
    fn source_type(&self) -> &'static str;
}

// ── 市场工具函数（A股 / 港股 / 美股）────────────────────────────

/// 根据股票代码识别市场板块
///
/// 支持的后缀格式：
/// - `.HK` → 港股（如 `00700.HK`）
/// - `.US` → 美股（如 `AAPL.US`）
/// - 无后缀 → 按首字符识别 A 股板块
pub fn detect_market_type(code: &str) -> &str {
    // 港股/美股后缀识别（不区分大小写）
    if code.ends_with(".HK") || code.ends_with(".hk") {
        return "hk";
    }
    if code.ends_with(".US") || code.ends_with(".us") {
        return "us";
    }
    // A 股板块识别
    match code.chars().next() {
        Some('6') if code.starts_with("688") => "star",
        Some('6') => "main_sh",
        Some('0') => "main_sz",
        Some('3') => "chinext",
        Some('8') => "bj",
        Some('4') => "neeq",
        // 北交所 2025 起启用 920 代码段；其余 9 开头仍是沪市 B 股。
        Some('9') if code.starts_with("920") => "bj",
        Some('9') => "b_share",
        _ => "unknown",
    }
}

/// 获取各市场涨跌停幅度（百分比）
///
/// 港股和美股不设涨跌停限制，返回 `f64::MAX` 表示无限制。
pub fn get_price_limit_pct(market_type: &str) -> f64 {
    match market_type {
        "star" | "chinext" => 20.0,
        "bj" => 30.0,
        "hk" | "us" => f64::MAX,
        _ => 10.0,
    }
}

/// 获取ST股票的涨跌停幅度
///
/// 港股美股无 ST 概念，直接返回无限制。
pub fn get_st_price_limit_pct(is_st: bool, market_type: &str) -> f64 {
    if is_st && !matches!(market_type, "hk" | "us") {
        5.0
    } else {
        get_price_limit_pct(market_type)
    }
}

#[cfg(test)]
mod financial_disclosure_tests {
    use super::*;

    fn rep(report_date: &str, disclosure_date: Option<&str>) -> FinancialReport {
        FinancialReport {
            stock_code: "000001".into(),
            report_date: report_date.into(),
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
            operating_cash_flow: None,
            capital_expenditure: None,
            free_cash_flow: None,
            current_ratio: None,
            quick_ratio: None,
            goodwill: None,
            accounts_receivable: None,
            estimated: Some(false),
            disclosure_date: disclosure_date.map(Into::into),
        }
    }

    #[test]
    fn statutory_deadline_covers_four_period_ends_and_rolls_the_year() {
        assert_eq!(
            rep("2026-03-31", None).statutory_disclosure_deadline().as_deref(),
            Some("2026-04-30")
        );
        assert_eq!(
            rep("2026-06-30", None).statutory_disclosure_deadline().as_deref(),
            Some("2026-08-31")
        );
        assert_eq!(
            rep("2026-09-30", None).statutory_disclosure_deadline().as_deref(),
            Some("2026-10-31")
        );
        // 年报落到**次年** 4-30 —— 跨年是最容易写错的一处
        assert_eq!(
            rep("2025-12-31", None).statutory_disclosure_deadline().as_deref(),
            Some("2026-04-30")
        );
    }

    #[test]
    fn statutory_deadline_refuses_to_guess_offcycle_periods() {
        // 带时间戳的期末（"2026-09-30 00:00:00"）**应当**推断，见下面那条测试；
        // 这里只列真正不是期末的形态。
        for d in ["2026-08-15", "", "garbage", "26-09-30", "2026-9-30"] {
            assert_eq!(
                rep(d, None).statutory_disclosure_deadline(),
                None,
                "非四类期末不推断: {d:?}"
            );
        }
    }

    #[test]
    fn effective_disclosure_date_prefers_vendor_notice_date() {
        let r = rep("2026-09-30", Some("2026-10-08 00:00:00"));
        assert_eq!(r.effective_disclosure_date(), Some(("2026-10-08".to_string(), "reported")));
        // 公告日为空串 ⇒ 退回推断（而不是把空串当日期比出去）
        let r2 = rep("2026-09-30", Some(""));
        assert_eq!(r2.effective_disclosure_date(), Some(("2026-10-31".to_string(), "estimated")));
    }

    #[test]
    fn long_timestamp_report_date_still_resolves_deadline() {
        // 多数 vendor 回 "YYYY-MM-DD 00:00:00"
        let r = rep("2026-12-31 00:00:00", None);
        assert_eq!(r.statutory_disclosure_deadline().as_deref(), Some("2027-04-30"));
    }
}

#[cfg(test)]
mod batch_dto_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn market_batch_quotes_request_round_trips_camel_case_wire() {
        let req: MarketBatchQuotesRequest = serde_json::from_value(json!({
            "codes": ["600519", "000001"],
            "perStockTimeoutMs": 5000u64,
            "totalTimeoutMs": 10000u64,
            "maxFailures": 3usize,
        }))
        .expect("camelCase 线上形态必须可反序列化（与原宿主 BatchQuotesRequest 兼容）");
        assert_eq!(req.per_stock_timeout_ms, Some(5000));
        assert_eq!(req.total_timeout_ms, Some(10000));
        assert_eq!(req.max_failures, Some(3));

        // Option 字段全部可缺省 ⇒ None，落到 provider 端默认策略
        let bare: MarketBatchQuotesRequest =
            serde_json::from_value(json!({"codes": ["600519"]})).unwrap();
        assert!(bare.per_stock_timeout_ms.is_none());
        assert!(bare.total_timeout_ms.is_none());
        assert!(bare.max_failures.is_none());
    }

    #[test]
    fn batch_result_wire_shape_matches_legacy_astock_data_definition() {
        // 上收前后 JSON 形状必须逐字节一致：camelCase + earlyTerminated 缺省容忍
        let r: BatchResult<i32> = serde_json::from_value(json!({
            "success": {"600519": 1i32},
            "failures": {"000001": "单只超时"},
            "elapsedMs": 42u64,
        }))
        .unwrap();
        assert_eq!(r.elapsed_ms, 42);
        assert!(!r.early_terminated);
        assert_eq!(r.total(), 2);
        assert!(!r.is_all_success());

        let out = serde_json::to_value(&r).unwrap();
        assert_eq!(out["elapsedMs"], json!(42));
        assert_eq!(out["earlyTerminated"], json!(false));
        assert!(out.get("elapsed_ms").is_none());
    }
}
