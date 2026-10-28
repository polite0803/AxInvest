//! 核心接口**日级健康检查**（没有面板也必须有记录的那一层）。
//!
//! ## 为什么需要它（2026-10-28）
//!
//! `vendor_health` 是 30 秒滑动窗口，`quality` 是报告产出后的评分，`validation`
//! 是单字段校验 —— 三者都**不回答**「今天这条核心取数通道到底有没有正常给出数据」。
//! 一个 `Ok(空数组)` 的接口可以连续几天看起来正常，直到有人翻报告才发现「大盘指数」
//! 板块一直是空的。本模块每个交易日收盘后跑一次，把**每个核心接口的结论**（四态）落成
//! 一行日级记录，让「昨天开始就坏了」可被直接看见，而不是靠猜。
//!
//! ## 归类判据（唯一来源）
//!
//! - 传输/结构错误的四态由 [`crate::outcome::classify_error`] 判；
//! - 「返回空且没报错」的归因由 [`Expect`] 决定：
//!   `MustHaveRows` 的核心维度在收盘后**应该**有数据 ⇒ 空 = 疑似静默降级（`source_failed`）；
//!   `MayBeEmpty` 的维度（新闻/快讯/龙虎榜）在清淡日**本来**可以没有 ⇒ `empty_by_time`。
//!   ⚠ 这两档不得互换 —— 把前者写成「时段性无数据」正是本轮要修的形态。
//!
//! ## 存放位置
//!
//! 复用**每日快照归档**（`astock_daily_snapshot`）而不是新建表：它本来就是
//! 「某天某个时点的一份归档」，端口 [`crate::daily_snapshot::DailySnapshotStore`] 已具备
//! 幂等读写与跨日聚合能力。为避免污染真实数据方法的覆盖率统计，预留方法名
//! [`HEALTH_SNAPSHOT_METHOD`] 不进 `SNAPSHOT_METHODS`、也不参与回放读取。
//!
//! ⚠ 归档未启用时检查**照跑**（结果只进日志），历史查询会明确返回
//! `storeEnabled=false`，不得把「没留存」显示成「没检查」。

use serde::{Deserialize, Serialize};

use crate::error::DataError;
use crate::outcome::{classify_error, outcome_from_opt, outcome_from_vec, DataStatus, Expect};
use crate::AStockClient;

/// 健康检查结果在每日快照归档里的保留方法名。
///
/// `__` 前缀 + 不在 `SNAPSHOT_METHODS` 中 ⇒ 不会被回放读取、不会被 sweep 重复采集。
pub const HEALTH_SNAPSHOT_METHOD: &str = "__interface_health__";

/// 探针标的：一只流动性充足、各维度都应有数据的主板股票。
pub const CORE_PROBE_CODE: &str = "600519";

/// 一个核心接口的探针。
#[derive(Debug, Clone, Copy)]
pub struct CoreInterface {
    /// 与 `execute_mcp_tool` / 报告 / data-quality 同一套方法名。
    pub method: &'static str,
    pub probe: Probe,
    pub expect: Expect,
}

/// 探针种类（枚举而非闭包：常量表要能放进 `const`）。
#[derive(Debug, Clone, Copy)]
pub enum Probe {
    IndexQuotes,
    Quote,
    Klines,
    Financials,
    MoneyFlow,
    IndustryRanking,
    HotStocks,
    News,
    ClsFlash,
    SocialSentiment,
    NorthBoundFlow,
    MarketDragonTiger,
}

/// 核心接口清单（**单一权威**）：分析链会直接消费的取数维度。
///
/// 不把「算法腿」（compute_*）放进来：它们不取数，输入坏掉是上游的事，
/// 混进来会让「接口健康」变成「整条链健康」，失去定位价值。
pub const CORE_INTERFACES: &[CoreInterface] = &[
    CoreInterface { method: "get_quote", probe: Probe::Quote, expect: Expect::MustHaveRows },
    CoreInterface { method: "get_klines", probe: Probe::Klines, expect: Expect::MustHaveRows },
    CoreInterface {
        method: "get_index_quotes",
        probe: Probe::IndexQuotes,
        expect: Expect::MustHaveRows,
    },
    CoreInterface {
        method: "get_financials",
        probe: Probe::Financials,
        expect: Expect::MustHaveRows,
    },
    CoreInterface {
        method: "get_money_flow",
        probe: Probe::MoneyFlow,
        expect: Expect::MustHaveRows,
    },
    CoreInterface {
        method: "get_industry_ranking",
        probe: Probe::IndustryRanking,
        expect: Expect::MustHaveRows,
    },
    CoreInterface {
        method: "get_hot_stocks",
        probe: Probe::HotStocks,
        expect: Expect::MustHaveRows,
    },
    CoreInterface { method: "search_news", probe: Probe::News, expect: Expect::MayBeEmpty },
    CoreInterface { method: "get_cls_flash", probe: Probe::ClsFlash, expect: Expect::MayBeEmpty },
    CoreInterface {
        method: "get_social_sentiment",
        probe: Probe::SocialSentiment,
        expect: Expect::MayBeEmpty,
    },
    CoreInterface {
        method: "get_north_bound_flow",
        probe: Probe::NorthBoundFlow,
        expect: Expect::MayBeEmpty,
    },
    CoreInterface {
        method: "get_market_dragon_tiger",
        probe: Probe::MarketDragonTiger,
        expect: Expect::MayBeEmpty,
    },
];

/// 单个接口的检查结论。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InterfaceHealthRow {
    pub method: String,
    pub status: DataStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    pub elapsed_ms: u64,
    pub rows: usize,
}

/// 一次日级检查的完整记录（落库与读回同形）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InterfaceHealthReport {
    /// 检查发生的本地时间（RFC3339）
    pub checked_at: String,
    /// 归属日（与每日快照同一口径：最近一个已完成交易日）
    pub anchor_date: String,
    pub probe_code: String,
    pub rows: Vec<InterfaceHealthRow>,
}

impl InterfaceHealthReport {
    /// 非 ok 的条目数（用于日志与面板摘要）。
    pub fn unhealthy(&self) -> usize {
        self.rows.iter().filter(|r| !r.status.is_ok()).count()
    }

    /// 一行摘要：`共 12 项；正常 10；时段性空 1；故障 1；结构变更 0`。
    pub fn summary(&self) -> String {
        let count = |st: DataStatus| self.rows.iter().filter(|r| r.status == st).count();
        format!(
            "共 {} 项；正常 {}；时段性空 {}；故障 {}；结构变更 {}",
            self.rows.len(),
            count(DataStatus::Ok),
            count(DataStatus::EmptyByTime),
            count(DataStatus::SourceFailed),
            count(DataStatus::SchemaChanged),
        )
    }
}

/// 历史查询结果（`storeEnabled=false` 时必须与「检查过但全好」分开）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InterfaceHealthHistory {
    pub store_enabled: bool,
    /// 按日期倒序
    pub days: Vec<InterfaceHealthReport>,
}

fn from_vec<T>(
    res: Result<Vec<T>, DataError>,
    expect: Expect,
) -> (DataStatus, Option<String>, usize) {
    let out = outcome_from_vec(res, expect);
    let rows = out.data.len();
    (out.status, out.detail, rows)
}

fn from_opt<T>(
    res: Result<Option<T>, DataError>,
    expect: Expect,
) -> (DataStatus, Option<String>, usize) {
    let out = outcome_from_opt(res, expect);
    let rows = usize::from(out.data.is_some());
    (out.status, out.detail, rows)
}

fn to_row(
    method: &str,
    (status, detail, rows): (DataStatus, Option<String>, usize),
    elapsed_ms: u64,
) -> InterfaceHealthRow {
    InterfaceHealthRow { method: method.to_string(), status, detail, elapsed_ms, rows }
}

/// 跑一遍全部核心接口，返回报告（**不落库**；落库走 [`run_and_store`]）。
///
/// 显式 `AS_OF.scope(None, ..)`：后台任务没有 task-local 作用域时会读全局栈，
/// 若此刻正有一轮回放在跑，探针会拿到截止日数据并被当成 live —— 那个坑已在
/// 快照采集侧踩过一次（见 `daily_snapshot::usable` 的注释）。这里显式声明 live。
pub async fn run_core_health_check(client: &AStockClient) -> InterfaceHealthReport {
    crate::as_of::AS_OF
        .scope(None, async {
            let mut rows = Vec::with_capacity(CORE_INTERFACES.len());
            for iface in CORE_INTERFACES {
                let started = std::time::Instant::now();
                let result = match iface.probe {
                    Probe::IndexQuotes => {
                        let outcome = client.get_index_quotes_outcome().await;
                        (outcome.status, outcome.detail, outcome.data.len())
                    },
                    Probe::Quote => match client.get_quote(CORE_PROBE_CODE).await {
                        Ok(_) => (DataStatus::Ok, None, 1),
                        Err(e) => (classify_error(&e), Some(e.to_string()), 0),
                    },
                    Probe::Klines => from_vec(
                        client.get_klines(CORE_PROBE_CODE, "daily", 10).await,
                        iface.expect,
                    ),
                    Probe::Financials => {
                        from_vec(client.get_financials(CORE_PROBE_CODE).await, iface.expect)
                    },
                    Probe::MoneyFlow => {
                        from_opt(client.get_money_flow(CORE_PROBE_CODE).await, iface.expect)
                    },
                    Probe::IndustryRanking => {
                        from_vec(client.get_industry_ranking().await, iface.expect)
                    },
                    Probe::HotStocks => from_vec(client.get_hot_stocks().await, iface.expect),
                    Probe::News => {
                        from_vec(client.search_news(CORE_PROBE_CODE, 5).await, iface.expect)
                    },
                    Probe::ClsFlash => from_vec(client.get_cls_flash().await, iface.expect),
                    Probe::SocialSentiment => {
                        from_vec(client.get_social_sentiment(CORE_PROBE_CODE).await, iface.expect)
                    },
                    Probe::NorthBoundFlow => {
                        from_opt(client.get_north_bound_flow().await, iface.expect)
                    },
                    Probe::MarketDragonTiger => {
                        from_vec(client.get_market_dragon_tiger().await, iface.expect)
                    },
                };
                rows.push(to_row(iface.method, result, started.elapsed().as_millis() as u64));
            }
            InterfaceHealthReport {
                checked_at: chrono::Local::now().to_rfc3339(),
                // 由调用方覆写成归属日更合适；这里先给当天，`run_and_store` 会覆盖。
                anchor_date: chrono::Local::now().format("%Y-%m-%d").to_string(),
                probe_code: CORE_PROBE_CODE.to_string(),
                rows,
            }
        })
        .await
}

/// 跑检查并写入当日归档；`anchor_date` 必须与快照采集同一归属日。
///
/// 归档未启用时仍返回报告（调用方负责日志），但**不**假装已留存。
pub async fn run_and_store(
    client: &AStockClient,
    anchor_date: &str,
) -> Result<InterfaceHealthReport, DataError> {
    let mut report = run_core_health_check(client).await;
    let checked_at = report.checked_at.clone();
    report.anchor_date = anchor_date.to_string();
    if report.unhealthy() > 0 {
        let bad: Vec<String> = report
            .rows
            .iter()
            .filter(|r| !r.status.is_ok())
            .map(|r| format!("{}={}", r.method, r.status.as_str()))
            .collect();
        tracing::warn!(
            "[interface-health] {anchor_date} 日检发现 {} 项异常: {}（检查时间 {checked_at}）",
            report.unhealthy(),
            bad.join("; ")
        );
    } else {
        tracing::info!("[interface-health] {anchor_date} 日检全部正常（{}）", report.summary());
    }
    if client.daily_snapshot_enabled() {
        let json = serde_json::to_string(&report)
            .map_err(|e| DataError::ParseError(format!("日检报告序列化失败: {e}")))?;
        client.set_health_snapshot(anchor_date, &json).await;
    } else {
        tracing::warn!(
            "[interface-health] {anchor_date} 每日归档未启用，日检结果只进日志（历史查询将为空）"
        );
    }
    Ok(report)
}

impl AStockClient {
    /// 日检报告写入每日快照归档（保留方法名，不进 `SNAPSHOT_METHODS`）。
    pub async fn set_health_snapshot(&self, anchor_date: &str, json: &str) {
        if let Some(store) = self.daily_snapshot_store.as_ref() {
            store.put(HEALTH_SNAPSHOT_METHOD, None, anchor_date, json).await;
        }
    }

    /// 读某归属日的日检报告原文（未启用归档时恒 `None`）。
    pub async fn get_health_snapshot(&self, anchor_date: &str) -> Option<String> {
        self.daily_snapshot_store.as_ref()?.get(HEALTH_SNAPSHOT_METHOD, anchor_date).await
    }

    /// 某归属日是否已跑过日检（幂等闸门；走 `get` ⇒ 继承 `usable` 过滤）。
    pub async fn has_health_snapshot(&self, anchor_date: &str) -> bool {
        match self.daily_snapshot_store.as_ref() {
            Some(store) => store.contains(HEALTH_SNAPSHOT_METHOD, None, anchor_date).await,
            None => false,
        }
    }

    /// 最近 `days` 个**交易日**的日检历史（倒序）。缺失的日子直接不出现。
    pub async fn interface_health_history(&self, days: usize) -> InterfaceHealthHistory {
        let enabled = self.daily_snapshot_enabled();
        if !enabled || days == 0 {
            return InterfaceHealthHistory { store_enabled: enabled, days: vec![] };
        }
        let mut out = Vec::new();
        let now = chrono::Local::now();
        // 归属日判据与采集侧同源：最近一个已完成交易日。
        // 盘中（`snapshot_target_date` 返回 None）时要退到**上一交易日**，
        // 否则今天那条永远没记录、窗口白占一格。
        let mut cursor = match crate::daily_snapshot::snapshot_target_date(&now) {
            Some(d) => d,
            None => {
                crate::calendar::previous_trading_day(now.date_naive() - chrono::Duration::days(1))
            },
        };
        for _ in 0..days {
            let date = cursor.format("%Y-%m-%d").to_string();
            if let Some(raw) = self.get_health_snapshot(&date).await {
                match serde_json::from_str::<InterfaceHealthReport>(&raw) {
                    Ok(report) => out.push(report),
                    Err(e) => tracing::warn!(
                        "[interface-health] {date} 日检记录反序列化失败（跳过该日）: {e}"
                    ),
                }
            }
            cursor = crate::calendar::previous_trading_day(cursor - chrono::Duration::days(1));
        }
        InterfaceHealthHistory { store_enabled: true, days: out }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 「本应有数据」的空返回必须归到故障档，不得写成时段性空值。
    #[test]
    fn must_have_rows_empty_is_source_failed() {
        let (st, detail, rows) = from_vec::<u8>(Ok(vec![]), Expect::MustHaveRows);
        assert_eq!(st, DataStatus::SourceFailed);
        assert!(detail.unwrap().contains("未报错"), "必须点明「空且没报错」这一形态");
        assert_eq!(rows, 0);

        let (st, _, _) = from_vec::<u8>(Ok(vec![]), Expect::MayBeEmpty);
        assert_eq!(st, DataStatus::EmptyByTime);
    }

    /// 结构错误优先：即使期望是 MayBeEmpty，解析失败也必须归 schema_changed。
    #[test]
    fn schema_error_is_not_downgraded_to_empty() {
        let (st, detail, _) =
            from_vec::<u8>(Err(DataError::ParseError("字段 f43 缺失".into())), Expect::MayBeEmpty);
        assert_eq!(st, DataStatus::SchemaChanged);
        assert!(detail.unwrap().contains("f43"));
    }

    /// 摘要必须分档计数（面板/日志直接读它）。
    #[test]
    fn summary_counts_each_status() {
        let report = InterfaceHealthReport {
            checked_at: "2026-10-28T16:00:00+08:00".into(),
            anchor_date: "2026-10-27".into(),
            probe_code: CORE_PROBE_CODE.into(),
            rows: vec![
                InterfaceHealthRow {
                    method: "get_quote".into(),
                    status: DataStatus::Ok,
                    detail: None,
                    elapsed_ms: 1,
                    rows: 1,
                },
                InterfaceHealthRow {
                    method: "search_news".into(),
                    status: DataStatus::EmptyByTime,
                    detail: None,
                    elapsed_ms: 1,
                    rows: 0,
                },
                InterfaceHealthRow {
                    method: "get_index_quotes".into(),
                    status: DataStatus::SourceFailed,
                    detail: Some("eastmoney_push2=source_failed".into()),
                    elapsed_ms: 1,
                    rows: 0,
                },
            ],
        };
        assert_eq!(report.unhealthy(), 2);
        let s = report.summary();
        assert!(s.contains("共 3 项"), "{s}");
        assert!(s.contains("正常 1"), "{s}");
        assert!(s.contains("时段性空 1"), "{s}");
        assert!(s.contains("故障 1"), "{s}");
    }
}
