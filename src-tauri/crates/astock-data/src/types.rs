pub use axagent_harness::market_data::{KLine, StockQuote, StockSearchResult};
// 以下来自 harness as_of DTO 契约
pub use axagent_harness::as_of::{
    AsOfContext, AsOfDataKind, AsOfDataScope, AsOfSource, DegradationEntry,
};
// 财务报告 DTO — 权威定义在 harness
pub use axagent_harness::market_data::FinancialReport;
// A 股市场工具函数 — 权威定义在 harness
pub use axagent_harness::market_data::{
    detect_market_type, get_price_limit_pct, get_st_price_limit_pct,
};

/// 创建行业均值估算的财务报告（所有 API 数据源均失败时的 fallback）
pub fn estimated_financial_report(stock_code: &str) -> FinancialReport {
    let today = crate::as_of::current_date_or_now();
    let market_type = detect_market_type(stock_code);
    let (eps, bps, roe, debt_ratio, gross_margin, net_margin) = match market_type {
        "star" | "chinext" => (0.35, 5.0, 6.0, 35.0, 35.0, 8.0),
        "bj" => (0.20, 3.0, 5.0, 40.0, 30.0, 5.0),
        _ => (0.50, 6.0, 8.0, 50.0, 25.0, 10.0),
    };
    FinancialReport {
        stock_code: stock_code.to_string(),
        report_date: today,
        // 行业均值兜底报告没有真实披露日（P1-1：as-of 按披露日裁，缺即判不可得）
        disclosure_date: None,
        revenue: Some(eps * 20.0 * 100_000_000.0),
        net_profit: Some(eps * 100_000_000.0),
        eps: Some(eps),
        bps: Some(bps),
        roe: Some(roe),
        debt_ratio: Some(debt_ratio),
        gross_margin: Some(gross_margin),
        net_margin: Some(net_margin),
        revenue_yoy: Some(5.0),
        profit_yoy: Some(3.0),
        total_assets: Some(bps * 100_000_000.0),
        operating_cash_flow: None,
        capital_expenditure: None,
        free_cash_flow: None,
        current_ratio: Some(1.5),
        quick_ratio: Some(1.0),
        goodwill: None,
        accounts_receivable: None,
        estimated: Some(true),
    }
}

use serde::{Deserialize, Serialize};

/// 新闻/公告条目
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NewsItem {
    pub title: String,
    pub summary: String,
    pub source: String,
    pub url: String,
    pub publish_time: String,
    pub sentiment_score: Option<f64>,
}

/// 资金流向
///
/// ⚠ 四档净额是 `Option<f64>`：**`None` = 该数据源不披露这一档**，`Some(0.0)` = 披露了且净额恰为零。
/// 此前四档是裸 `f64`，各源缺一项就写 `0.0`（tencent 只有散户档、baidu 解析失败也归 0），
/// 下游把「没有这个字段」读成「这一档净额为零」—— 与全仓「缺失不得兜底成 0/伪造」的口径冲突。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MoneyFlow {
    pub date: String,
    pub main_net_inflow: f64,
    #[serde(default)]
    pub super_large_net: Option<f64>,
    #[serde(default)]
    pub large_net: Option<f64>,
    #[serde(default)]
    pub medium_net: Option<f64>,
    #[serde(default)]
    pub small_net: Option<f64>,
    /// 近 N 日历史资金流向（按日期降序，第 0 条 = 最新日 = 与顶层字段同一天）。
    /// 只有支持多日查询的 vendor（如 eastmoney）会填充，其他 vendor 留空 Vec。
    /// prompt 要求"连续 3-5 日趋势"分析，单日数据无法支撑。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub history: Vec<MoneyFlowDaily>,
}

/// 单日资金流向（历史序列中的一天）—— 四档同样 `Option`，理由见 [`MoneyFlow`]
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MoneyFlowDaily {
    pub date: String,
    pub main_net_inflow: f64,
    // default：改造前写入的快照若缺这些键，反序列化不该整条失败
    #[serde(default)]
    pub super_large_net: Option<f64>,
    #[serde(default)]
    pub large_net: Option<f64>,
    #[serde(default)]
    pub medium_net: Option<f64>,
    #[serde(default)]
    pub small_net: Option<f64>,
}

/// 龙虎榜条目
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DragonTigerEntry {
    pub stock_code: String,
    pub date: String,
    pub dept_name: String,
    pub buy_amount: f64,
    pub sell_amount: f64,
    pub net_amount: f64,
    pub reason: Option<String>,
}

/// 限售解禁
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LockupSchedule {
    pub stock_code: String,
    pub stock_name: String,
    pub unlock_date: String,
    pub unlock_shares: f64,
    pub unlock_ratio: f64,
    pub shareholder: Option<String>,
}

/// 融资融券数据
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MarginData {
    pub stock_code: String,
    pub date: String,
    pub margin_buy: f64,        // 融资买入额
    pub margin_balance: f64,    // 融资余额
    pub short_sell_volume: f64, // 融券卖出量
    pub short_balance: f64,     // 融券余量
}

/// 北向资金持仓
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NorthBoundHolding {
    pub stock_code: String,
    pub date: String,
    pub holding_shares: f64, // 持股数量
    pub holding_ratio: f64,  // 持股占比
    pub change_shares: f64,  // 变动数量
}

/// 股权质押数据
/// 新增(2026-07-22 #4): 原 astock-data 无质押数据接口,
/// LLM 调用 detect_pledge_risk 时无 pledge_pct 可用,导致报告 8 处标注质押数据缺失。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PledgeData {
    pub stock_code: String,
    /// 大股东质押总比例(%)
    pub pledge_ratio: f64,
    /// 质押股数(股)
    pub pledge_shares: f64,
    /// 质押笔数
    pub pledge_count: i32,
    /// 控股股东质押比例(%)
    pub controlling_pledge_ratio: f64,
    /// 风险等级("安全"/"低风险"/"中风险"/"高风险"/"极高风险")
    pub risk_level: String,
}

/// 股东户数（筹码集中度）—— `RPT_HOLDERNUMLATEST` 的「最新一期」快照。
///
/// 新增(2026-10-01)：`lockup-watcher.md` 的方法论/工作流程/自检清单**三处**都要求
/// 「股东人数（户均持股）」，而它当时既不在该分析师的 `data_sources`（只有
/// `get_stock_lockup_bundle` + `get_stock_pledge_data`），也没有任何工具能取 ——
/// 分析师只能写「`data_gaps`：股东人数数据缺失」⇒ 命中失败标记词表 ⇒ 判「⚠️ 低置信」
/// （300604 运行 `cd044375` 实证，自评 75.0 却因这一处标记被判低置信）。
/// 消费方：`get_stock_lockup_bundle` 的 `holder_count` 段（bundle 第四方）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HolderCount {
    pub stock_code: String,
    /// 数据截止日（`END_DATE`，`YYYY-MM-DD`）
    pub end_date: String,
    /// 股东户数（户）
    pub holder_num: Option<f64>,
    /// 户数较上期变化率(%，`HOLDER_NUM_RATIO`)：**下降=筹码集中**（通常偏多），上升=分散
    pub holder_num_ratio: Option<f64>,
    /// 户均持股（股，`AVG_HOLD_NUM`）
    pub avg_hold_num: Option<f64>,
    /// 公告日（`HOLD_NOTICE_DATE`，`YYYY-MM-DD`）—— 判「数据是否已过时」要看它而非截止日
    pub notice_date: Option<String>,
}

/// 行业分类
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SectorInfo {
    pub stock_code: String,
    pub sector_name: String, // 申万一级行业
    pub sub_sector: String,  // 申万二级行业
    pub concept_tags: Vec<String>,
    #[serde(default)]
    pub avg_pe: Option<f64>,
    #[serde(default)]
    pub avg_pb: Option<f64>,
}

/// 股东增减持
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShareholderTrade {
    pub stock_code: String,
    pub date: String,
    pub shareholder_name: String,
    pub trade_type: String, // 增持/减持
    pub shares: f64,
    pub price: f64,
    pub reason: Option<String>,
}

/// 除权除息
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DividendRecord {
    pub stock_code: String,
    pub ex_date: String,
    pub dividend_per_share: f64, // 每股分红
    pub bonus_share_ratio: f64,  // 送转比例
    pub record_date: String,
}

/// K线周期枚举（兼容券商API代码）
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum KLinePeriod {
    #[serde(rename = "5")]
    Min5,
    #[serde(rename = "15")]
    Min15,
    #[serde(rename = "30")]
    Min30,
    #[serde(rename = "60")]
    Min60,
    Daily,
    Weekly,
    Monthly,
}

impl KLinePeriod {
    /// 转换为东方财富 API 的 period 代码
    pub fn to_em_code(&self) -> &str {
        match self {
            KLinePeriod::Min5 => "5",
            KLinePeriod::Min15 => "15",
            KLinePeriod::Min30 => "30",
            KLinePeriod::Min60 => "60",
            KLinePeriod::Daily => "101",
            KLinePeriod::Weekly => "102",
            KLinePeriod::Monthly => "103",
        }
    }
}

/// 研报
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResearchReport {
    pub title: String,
    pub institution: String,
    pub analyst: Option<String>,
    pub rating: Option<String>,
    pub target_price: Option<f64>,
    pub eps_forecast: Vec<EpsForecast>,
    pub publish_date: String,
    pub pdf_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EpsForecast {
    pub year: String,
    pub eps: Option<f64>,
}

/// 机构一致预期EPS
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConsensusEPS {
    pub stock_code: String,
    pub consensus_eps: Option<f64>,
    pub consensus_target_price: Option<f64>,
    pub rating_avg: Option<String>,
    pub rating_count: Option<i32>,
    pub year: String,
    /// 该 EPS 是否为**估算值**（非真实一致预期 / 财报数据）。
    ///
    /// `true` 只出现在 vendor 全部失败的兜底分支（`lib.rs` 的 C-fallback）：
    /// 按挂牌板块取常数（科创/创业 0.40、北交所 0.25、其余主板 0.55）。
    ///
    /// **下游不得用估算值做「超预期 / 不及预期」类判定** —— 那是拿一个假基准
    /// 算出来的相对量（`tools/src/tools/finance.rs` 的 `detect_earnings` 即此类消费）。
    /// 形态与 `is_fallback_anchor`（`mcp_tools.rs` 的 DCF 锚点标记）一致：
    /// bool 标记 + 消费端数值降级 / 拒判。
    ///
    /// 为什么必须是**显式标记**而不是让下游猜：常数 0.55 完全可能是某只股票的真实
    /// 一致预期，数值层面不可区分 —— 只有产出端知道自己是不是编的。
    #[serde(default)]
    pub is_estimated: bool,
    /// 估算来源，仅 `is_estimated = true` 时有值（如 `"board_constant:star"`）。
    ///
    /// 留痕用：使「这条数据是编的」可归因到具体兜底规则，而非只留一句 `warn!` 日志。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub estimate_source: Option<String>,
}

/// 热股榜一行（同花顺 fuyao `hot_list`，真·热度榜）。
///
/// 名目收编(2026-10-03)：本类型此前被三个源各自灌入**不同语义** ——
/// `vendors/ths.rs` 灌涨停池前 20 行、`vendors/iwencai.rs` 灌「今日涨幅前20」、
/// `vendors/neodata.rs` 从模型生成的文本里刮行 —— 而面板标题与 `SocialSentiment.hot_rank`
/// 回填都按「热度榜」读它。现只有同花顺真热股榜供应，并带上名次与热度值。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HotStock {
    pub stock_code: String,
    pub stock_name: String,
    /// 涨跌幅 %（热股榜的 `rise_and_fall` **实测已是百分比原值**，勿再乘 100）
    pub change_pct: f64,
    /// 换手率 % —— 热股榜不提供，恒 `None`（真值在 `LimitUpPoolEntry` 那边）
    pub turnover_rate: Option<f64>,
    /// 题材标签（热股榜 `tag.concept_tag`）
    pub reason_tags: Vec<String>,
    /// 行业名 —— 热股榜不提供，恒 `None`
    pub sector: Option<String>,
    /// 榜内名次（`order`，1 起；榜单长度实测恒 100）
    pub rank: Option<u32>,
    /// 热度值（`rate`，字符串原值转 f64；`type=hour` 与 `type=day` 量级不同，只可同期比较）
    pub hot_value: Option<f64>,
}

/// 概念板块三维归属
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConceptBlocks {
    pub stock_code: String,
    pub industry: String,
    pub concepts: Vec<BlockItem>,
    pub regions: Vec<BlockItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BlockItem {
    pub name: String,
    pub change_pct: Option<f64>,
}

/// 公告
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Announcement {
    pub title: String,
    pub stock_code: String,
    pub stock_name: Option<String>,
    pub announce_date: String,
    pub ann_type: Option<String>,
    pub pdf_url: Option<String>,
}

/// 行业排名
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IndustryRank {
    pub industry_name: String,
    pub change_pct: f64,
    pub turnover: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub main_inflow: Option<f64>,
    pub leader_code: Option<String>,
    pub leader_name: Option<String>,
    pub leader_change_pct: Option<f64>,
}

/// 财联社快讯
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClsFlashItem {
    pub title: String,
    pub content: String,
    pub publish_time: String,
    pub source: Option<String>,
}

/// 社交舆情数据（股吧/雪球热度）
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SocialSentiment {
    pub stock_code: String,
    pub stock_name: String,
    /// 平台标识（"guba" / "xueqiu" / "weibo"）
    pub platform: String,
    /// 帖子/讨论数
    pub post_count: u32,
    /// 热度排名（平台内）
    pub hot_rank: Option<u32>,
    /// 情感倾向（-1.0 极度看空 ~ 1.0 极度看多）
    pub sentiment_score: Option<f64>,
    /// 看多占比（0.0 ~ 1.0）
    pub bull_ratio: Option<f64>,
    /// 数据采集时间戳
    pub fetched_at: i64,
}

/// 全市场龙虎榜
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MarketDragonTiger {
    pub stock_code: String,
    pub stock_name: String,
    pub date: String,
    pub net_buy: f64,
    pub buy_amount: f64,
    pub sell_amount: f64,
    pub reason: Option<String>,
}

/// 大宗交易
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BlockTrade {
    pub stock_code: String,
    pub stock_name: String,
    pub trade_date: String,
    pub price: f64,
    pub volume: f64,
    pub amount: f64,
    pub buyer_dept: Option<String>,
    pub seller_dept: Option<String>,
    pub discount_pct: Option<f64>,
}

/// 机构调研记录
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstitutionalVisit {
    pub stock_code: String,
    pub stock_name: String,
    pub visit_date: String,
    pub institution_count: i32,
    pub main_content: String,
    pub visit_type: Option<String>,
}

/// 北向资金分钟级流向
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NorthBoundFlow {
    pub date: String,
    pub sh_flow: f64,
    pub sz_flow: f64,
    pub total_flow: f64,
    pub timestamp: Option<String>,
    /// 最近若干个交易日的资金流明细(从最新到最旧),用于趋势观察
    /// 仅当 vendor 返回多日数据时填充,默认空数组
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub recent_history: Vec<NorthBoundFlowDaily>,
}

/// 北向资金单日明细(用于 recent_history)
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NorthBoundFlowDaily {
    pub date: String,
    pub sh_flow: f64,
    pub sz_flow: f64,
    pub total_flow: f64,
}

/// 大盘指数行情
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IndexQuote {
    pub code: String,
    pub name: String,
    pub price: f64,
    pub pre_close: f64,
    pub change_pct: f64,
    pub volume: f64,
    pub amount: f64,
}

/// 同行业可比公司估值
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PeerComparison {
    pub stock_code: String,
    pub stock_name: String,
    pub pe: Option<f64>,
    pub pb: Option<f64>,
    pub roe: Option<f64>,
    /// `roe` 的**实际口径**（报告期，`YYYY-MM-DD`；取不到时为 `None`）。
    ///
    /// 为什么必须与值一起返回：`ROEJQ` 是**年内累计值**，一季报/中报/三季报都不是全年数，
    /// 而本结构的 `roe` 消费端是**横截面**比较（同行 vs 本公司）。不带口径时，
    /// 「只披露到中报的同侪」会被读成「盈利能力只有年报同侪的一半」——
    /// 实测 600887 中报 10.09 vs 年报 20.87（差 2.07 倍）。取数侧统一按**年报优先**挑选
    /// （见 `eastmoney::pick_peer_roe`），此字段把该口径如实暴露给消费端。
    pub roe_period: Option<String>,
    pub change_pct: f64,
    pub market_cap: Option<f64>,
}

/// 期权PCR（看跌/看涨比率）
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OptionPCR {
    pub stock_code: String,
    pub date: String,
    pub call_volume: f64,
    pub put_volume: f64,
    pub call_oi: f64,
    pub put_oi: f64,
    pub volume_pcr: f64,
    pub oi_pcr: f64,
}

/// 批量原始数据
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StockRawData {
    pub quote: StockQuote,
    pub klines: Vec<KLine>,
    pub financials: Vec<FinancialReport>,
    pub news: Vec<NewsItem>,
    pub money_flow: Option<MoneyFlow>,
    pub dragon_tiger: Vec<DragonTigerEntry>,
    pub lockup: Vec<LockupSchedule>,
    pub margin_data: Option<MarginData>,
    pub north_bound: Option<NorthBoundHolding>,
    pub sector_info: Option<SectorInfo>,
    pub shareholder_trades: Vec<ShareholderTrade>,
    pub dividend_records: Vec<DividendRecord>,
    pub research_reports: Vec<ResearchReport>,
    pub consensus_eps: Option<ConsensusEPS>,
    pub concept_blocks: Option<ConceptBlocks>,
    pub announcements: Vec<Announcement>,
    pub block_trades: Vec<BlockTrade>,
    pub institutional_visits: Vec<InstitutionalVisit>,
    pub peers: Vec<PeerComparison>,
    pub option_pcr: Option<OptionPCR>,
    /// H1.4 修复:fetch_all 中各子查询失败时的错误信息集合
    /// (空 Vec 表示全部成功;非空时调用方可据此判断数据完整性,
    /// 决定是降级使用部分数据还是直接报错)
    #[serde(default)]
    pub errors: Vec<String>,
}

/// 市场级原始数据
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MarketRawData {
    pub hot_stocks: Vec<HotStock>,
    pub industry_ranking: Vec<IndustryRank>,
    pub cls_flash: Vec<ClsFlashItem>,
    pub market_dragon_tiger: Vec<MarketDragonTiger>,
    pub north_bound_flow: Option<NorthBoundFlow>,
    pub index_quotes: Vec<IndexQuote>,
}

// ─── R3-A 复权 ───

pub use axagent_harness::market_data::AdjType;

/// 单次除权除息事件 (R3-A)
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AdjustmentEvent {
    /// 股票代码
    pub stock_code: String,
    /// 除权除息日 (YYYY-MM-DD)
    pub ex_date: String,
    /// 每股现金分红（元）
    pub cash_dividend: f64,
    /// 送转股比例（如 0.2 = 10送2）
    pub bonus_share_ratio: f64,
    /// 配股比例
    pub rights_ratio: f64,
    /// 配股价
    pub rights_price: f64,
}

// ─── R3-B 财报日历 ───

/// 财报披露事件 (R3-B)
///
/// `event_type` 取值:
/// - "preliminary"        业绩预告
/// - "express"           业绩快报
/// - "formal"            正式财报
/// - "shareholders_meeting" 股东大会
/// - "other"             其它披露
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct EarningsEvent {
    pub stock_code: String,
    pub stock_name: String,
    /// YYYY-MM-DD
    pub event_date: String,
    /// "preliminary" | "express" | "formal" | "shareholders_meeting" | "other"
    pub event_type: String,
    /// 财报期间（"2025Q3" / "2025年报"）
    pub period: Option<String>,
    /// 摘要/标题
    pub detail: Option<String>,
    /// vendor 标识（"cninfo" / "ths"）
    pub source: Option<String>,
    pub created_at: i64,
}

/// 概念板块
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConceptBoard {
    pub board_code: String,
    pub board_name: String,
    pub stock_count: u32,
}

/// 板块成分股
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BoardMember {
    pub stock_code: String,
    pub stock_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub change_pct: Option<f64>,
}

/// 单日历史估值快照（估值带的数据供应方）
///
/// 来源：东财数据中心 `RPT_VALUEANALYSIS_DET`（每交易日一行，可回溯 8 年+）。
/// 用途：回填本地 `financial_snapshots` 表 —— 该表原设计为"每日 EOD 写一行"，
/// 但当时没有任何写入路径，导致估值带永远算不出分位（样本恒为 0）。
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ValuationSnapshot {
    /// 交易日，YYYY-MM-DD
    pub trade_date: String,
    /// 证券简称（RPT_VALUEANALYSIS_DET.SECURITY_NAME_ABBR）。
    /// S8(2026-09-26)：as-of 合成 quote 的 name 恒为代码 ⇒ 历史记录分组名显示成代码，
    /// 回放路径用本字段回填真实名称。历史序列消费方不读取该字段（serde 增量兼容）。
    #[serde(default)]
    pub security_name: Option<String>,
    pub pe_ttm: Option<f64>,
    /// 市净率（MRQ）
    pub pb: Option<f64>,
    pub ps_ttm: Option<f64>,
    /// 市现率（经营现金流 TTM）
    pub pcf: Option<f64>,
    /// 当日收盘价（未复权）
    pub close_price: Option<f64>,
    /// 当日总市值
    pub total_market_cap: Option<f64>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_market_sh_main() {
        assert_eq!(detect_market_type("600519"), "main_sh");
    }

    #[test]
    fn test_detect_market_star() {
        assert_eq!(detect_market_type("688001"), "star");
    }

    #[test]
    fn test_detect_market_sz_main() {
        assert_eq!(detect_market_type("000001"), "main_sz");
    }

    #[test]
    fn test_detect_market_chinext() {
        assert_eq!(detect_market_type("300750"), "chinext");
    }

    #[test]
    fn test_detect_market_bj() {
        assert_eq!(detect_market_type("830946"), "bj");
    }

    #[test]
    fn test_price_limit_main() {
        assert!((get_price_limit_pct("main_sh") - 10.0).abs() < 1e-6);
    }

    #[test]
    fn test_price_limit_star() {
        assert!((get_price_limit_pct("star") - 20.0).abs() < 1e-6);
    }

    #[test]
    fn test_price_limit_bj() {
        assert!((get_price_limit_pct("bj") - 30.0).abs() < 1e-6);
    }

    #[test]
    fn test_st_price_limit() {
        assert!((get_st_price_limit_pct(true, "main_sh") - 5.0).abs() < 1e-6);
        assert!((get_st_price_limit_pct(false, "main_sh") - 10.0).abs() < 1e-6);
    }

    #[test]
    fn test_kline_period_to_em_code() {
        assert_eq!(KLinePeriod::Daily.to_em_code(), "101");
        assert_eq!(KLinePeriod::Weekly.to_em_code(), "102");
        assert_eq!(KLinePeriod::Min5.to_em_code(), "5");
    }

    #[test]
    fn test_stock_quote_serialization() {
        let quote = StockQuote {
            code: "600519".to_string(),
            name: "茅台".to_string(),
            price: 1800.0,
            pre_close: 1785.0,
            open: 1790.0,
            high: 1810.0,
            low: 1785.0,
            volume: 5000000.0,
            amount: 9000000000.0,
            change_pct: 0.56,
            turnover_rate: 0.3,
            pe: Some(35.0),
            pb: Some(12.0),
            total_mv: Some(2250000000000.0),
            circulating_mv: None,
            limit_up: None,
            limit_down: None,
            is_st: false,
            timestamp: "2025-01-15 14:00:00".to_string(),
        };
        let json = serde_json::to_string(&quote).unwrap();
        assert!(json.contains("600519"));
        assert!(json.contains("camelCase") || json.contains("changePct"));
        let parsed: StockQuote = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.code, "600519");
    }

    #[test]
    fn test_kline_serialization() {
        let kline = KLine {
            date: "2025-01-15".to_string(),
            open: 10.0,
            high: 11.0,
            low: 9.5,
            close: 10.5,
            volume: 10000.0,
            amount: 105000.0,
            turnover_rate: Some(0.5),
            adj_factor: None,
        };
        let json = serde_json::to_string(&kline).unwrap();
        assert!(json.contains("2025-01-15"));
        let _parsed: KLine = serde_json::from_str(&json).unwrap();
    }

    #[test]
    fn test_stock_search_result_serialization() {
        let result = StockSearchResult {
            code: "600519".to_string(),
            name: "贵州茅台".to_string(),
            market: "上海".to_string(),
        };
        let json = serde_json::to_string(&result).unwrap();
        assert!(json.contains("贵州茅台"));
        let _parsed: StockSearchResult = serde_json::from_str(&json).unwrap();
    }
}

/// 涨停池一行（同花顺 `data.10jqka.com.cn/dataapi/limit_up/limit_up_pool`）。
///
/// 一行同时给出**妖股三条初筛判据里的两条**（连板数 `limit_up_streak`、换手率 `turnover_rate`）
/// 加上封板资金与炸板次数。除 `stock_code`/`stock_name` 外一律 `Option`：
/// 实测 `open_num`（炸板次数）约 2/3 的行是 `null`，`limit_up_suc_rate` 偶发 `null`，
/// 把它们折成 `0.0` 就是把「接口没说」伪装成「接口说是零」。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LimitUpPoolEntry {
    pub stock_code: String,
    pub stock_name: String,
    /// 连板数（`high_days_value` 高 16 位；「3天2板」里的「2板」）
    pub limit_up_streak: Option<i32>,
    /// 涨停天数（`high_days_value` 低 16 位；「3天2板」里的「3天」）
    ///
    /// 与连板数不同轴：3天2板 是断过板的弱连板，7天7板 是连续板 —— 妖股判据要区分。
    pub limit_up_days: Option<i32>,
    /// 当日换手率 %（`turnover_rate`，接口原值）
    pub turnover_rate: Option<f64>,
    /// 涨跌幅 %（`change_rate`）
    pub change_pct: Option<f64>,
    /// 封单金额（元，`order_amount`）
    pub seal_amount: Option<f64>,
    /// 封单量（股，`order_volume`）
    pub seal_volume: Option<f64>,
    /// 炸板次数（`open_num`；实测多为 `null`）
    pub break_count: Option<i32>,
    /// 首次封板时间 `HH:MM:SS`（`first_limit_up_time`，字符串 Unix 秒按 UTC+8 换算）
    pub first_seal_time: Option<String>,
    /// 最后封板时间 `HH:MM:SS`（`last_limit_up_time` 同上）
    pub last_seal_time: Option<String>,
    /// 板型：一字板 / 换手板 / T字板（`limit_up_type`，实测仅此三值）
    pub limit_up_type: Option<String>,
    /// 炸板后又回封（`is_again_limit`；实测 =1 的行同时带 `change_tag="LIMIT_BACK"`）
    pub re_sealed: Option<bool>,
    /// 流通市值（元，`currency_value`）
    pub float_market_cap: Option<f64>,
    /// 池中快照价（元，`latest`）—— 面板显示现价用，省掉逐只再打一次 quote
    pub latest_price: Option<f64>,
    /// 涨停逻辑标签（`reason_type`，**实测分隔符是 `+` 不是逗号**）
    pub reason_tags: Vec<String>,
}

/// 涨停池的当日盘面情绪汇总（接口 `limit_up_count.today` / `limit_down_count.today`）。
///
/// 只取 `today` 段：`yesterday` 段能靠再请求前一交易日拿到，而回放要的正是「按日取」，
/// 把两日的数塞进同一行会让「哪一天」重新变成歧义。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LimitUpBreadth {
    /// 收盘涨停家数
    pub limit_up_count: i32,
    /// 触板次数（含炸板），`history_num`
    pub touched_count: Option<i32>,
    /// 封板率 = 涨停家数 / 触板次数
    pub seal_rate: Option<f64>,
    /// 炸板家数
    pub break_count: Option<i32>,
    /// 收盘跌停家数
    pub limit_down_count: Option<i32>,
}

/// 一次涨停池查询的结果。
///
/// 实测（2026-10-03）同花顺该接口**真正按 `date` 返回历史池**：`data.date` 恒等于请求日，
/// 越界日期回 `status_code=-1 / status_msg="date参数不合法"`，非交易日回 `total=0`。
/// ⇒ 三种情形天然可分：回显不等 = 接口行为变了（实现侧直接报错），
/// `status_code != 0` = **该日不可得**，`total=0` 且回显相等 = **该日确无涨停**。
///
/// 对照：东财 `push2ex/getTopicZTPool` 的 `date` 参数**不被采纳**（`qdate` 恒为最新交易日），
/// 故本仓涨停池只走同花顺一条通道。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LimitUpPool {
    /// 接口自报的生效日期 `YYYY-MM-DD`（来自 `data.date`，紧凑形已展开成 ISO）
    pub pool_date: String,
    /// 调用方请求的日期；`None` = 取当下
    pub requested_date: Option<String>,
    pub entries: Vec<LimitUpPoolEntry>,
    /// 盘面情绪汇总（接口未给该段时为 `None`，不编造）
    pub breadth: Option<LimitUpBreadth>,
}

impl LimitUpPool {
    /// 这份池子是否真的属于请求的那一天。
    ///
    /// 未指定请求日期（取当下）时恒为真 —— 此时 `pool_date` 就是答案本身。
    /// ⚠ 消费方拿到 `false` 时必须按「不可得」处理，**不得**读成「当天没有涨停」。
    pub fn is_for_requested_date(&self) -> bool {
        match self.requested_date.as_deref() {
            None => true,
            Some(req) => self.pool_date == req,
        }
    }
}
