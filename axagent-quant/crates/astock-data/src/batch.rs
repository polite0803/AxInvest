//! MarketBatchQuery —— DataFrame 风格的批量行情/财务查询
//!
//! ## 背景
//!
//! TradingAgents-CN 的 `unified_dataframe.py` 返回 `pandas.DataFrame`,
//! 把"列名标准化 + 批量取数 + 单次 HTTP 拉全市场"封装好,给筛选/回测/统计用。
//! AxAgent 是 Rust 桌面端,没有 pandas 生态,但可以借鉴:
//!
//! 1. **列名标准化**:全市场拉回的每只股票字段一致(open/high/low/close/volume/amount/
//!    turnover_rate/change_pct),不再按 vendor 各自解析;
//! 2. **批量并发**:一次 `get_quotes_batch` 内部用 `FuturesUnordered` 并发拉多只股票,
//!    受 `DomainGate` 限流约束;
//! 3. **可空字段明确化**:`Option<f64>` 标记 vendor 缺失的字段,前端/Agent 不会拿到 NaN。
//!
//! ## 命名约定
//!
//! - `get_quotes_batch`:批量拉实时行情(类比 akshare `stock_zh_a_spot_em`)
//! - `get_klines_batch`:批量拉 K 线(类比 akshare `stock_zh_a_hist` 全市场版)
//! - `get_financials_batch`:批量拉财务快照
//! - `to_dataframe_json`:转 DataFrame 风格 JSON `{columns, rows}`(前端渲染用)

use crate::types::*;
use async_trait::async_trait;
use futures::stream::{FuturesUnordered, StreamExt};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

#[derive(Debug, Clone)]
pub struct BatchRequest {
    pub codes: Vec<String>,
    pub per_stock_timeout: Duration,
    pub total_timeout: Duration,
    pub max_failures: usize,
    /// P3-D12: 最大并发数。
    /// - `0`：自动按 vendor 健康度决定（由 `BatchRunner::resolve_concurrency` 解析），
    ///   健康时取 `BATCH_DEFAULT_CONCURRENCY`，部分降级降速，大面积降级进保命模式
    /// - `>0`：显式指定并发上限（调用方已知 vendor 状态时可用，跳过健康快照查询）
    pub max_concurrent: usize,
}

impl Default for BatchRequest {
    fn default() -> Self {
        Self {
            codes: Vec::new(),
            per_stock_timeout: Duration::from_secs(8),
            total_timeout: Duration::from_secs(30),
            max_failures: 0,
            max_concurrent: 0,
        }
    }
}

impl BatchRequest {
    pub fn new(codes: Vec<String>) -> Self {
        Self { codes, ..Default::default() }
    }

    pub fn with_per_stock_timeout(mut self, d: Duration) -> Self {
        self.per_stock_timeout = d;
        self
    }

    pub fn with_total_timeout(mut self, d: Duration) -> Self {
        self.total_timeout = d;
        self
    }

    pub fn with_max_failures(mut self, n: usize) -> Self {
        self.max_failures = n;
        self
    }

    /// P3-D12: 显式指定并发上限（`0` 仍表示自动）。
    pub fn with_max_concurrent(mut self, n: usize) -> Self {
        self.max_concurrent = n;
        self
    }
}

// ── BatchResult / MarketBatchQuotesRequest：P6.1 迁移时上收至 harness（consumer
// 只能依赖 harness）。本模块再导出原类型，保持 `batch::BatchResult` 旧导入路径
// 与 JSON 线上形态逐字节兼容（camelCase + earlyTerminated 缺省容忍）。
pub use axagent_harness::market_data::{BatchResult, MarketBatchQuotesRequest};

#[async_trait]
pub trait BatchVendor: Send + Sync {
    async fn batch_get_quotes(
        &self,
        codes: &[String],
    ) -> Result<Vec<StockQuote>, crate::error::DataError> {
        let _ = codes;
        Err(crate::error::DataError::VendorError {
            vendor: "batch".into(),
            message: "vendor 未实现 batch_get_quotes".into(),
        })
    }
}

#[async_trait]
pub trait MarketBatchQuery: Send + Sync {
    async fn get_quotes_batch(&self, req: BatchRequest) -> BatchResult<StockQuote>;
    async fn get_klines_batch(
        &self,
        codes: Vec<String>,
        period: &str,
        limit: u32,
    ) -> BatchResult<Vec<KLine>>;
    async fn get_financials_batch(&self, codes: Vec<String>) -> BatchResult<Vec<FinancialReport>>;
}

pub const DATAFRAME_QUOTE_COLUMNS: &[&str] = &[
    "code",
    "name",
    "open",
    "high",
    "low",
    "close",
    "pre_close",
    "volume",
    "amount",
    "change_pct",
    "turnover_rate",
    "pe",
    "pb",
    "total_mv",
    "is_st",
    "timestamp",
];

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DataFrame {
    pub columns: Vec<String>,
    pub rows: Vec<Vec<serde_json::Value>>,
}

impl DataFrame {
    pub fn from_quotes(quotes: &[StockQuote]) -> Self {
        let columns: Vec<String> = DATAFRAME_QUOTE_COLUMNS.iter().map(|s| s.to_string()).collect();
        let rows = quotes
            .iter()
            .map(|q| {
                let opt_f64 = |v: Option<f64>| -> serde_json::Value {
                    match v.and_then(serde_json::Number::from_f64) {
                        Some(n) => serde_json::Value::Number(n),
                        None => serde_json::Value::Null,
                    }
                };
                vec![
                    serde_json::json!(q.code),
                    serde_json::json!(q.name),
                    serde_json::json!(q.open),
                    serde_json::json!(q.high),
                    serde_json::json!(q.low),
                    serde_json::json!(q.price),
                    serde_json::json!(q.pre_close),
                    serde_json::json!(q.volume),
                    serde_json::json!(q.amount),
                    serde_json::json!(q.change_pct),
                    serde_json::json!(q.turnover_rate),
                    opt_f64(q.pe),
                    opt_f64(q.pb),
                    opt_f64(q.total_mv),
                    serde_json::json!(q.is_st),
                    serde_json::json!(q.timestamp),
                ]
            })
            .collect();
        Self { columns, rows }
    }

    /// 修复 P0-A2: 序列化失败时记录错误并返回 `""`，避免污染调用方期望
    /// （`to_dataframe_json` 仍是主要消费方，行为不变）。空串仍可被上游
    /// 视为"渲染为空"——但错误已落 tracing，运维可据此排查。
    pub fn to_json(&self) -> String {
        match serde_json::to_string(self) {
            Ok(s) => s,
            Err(e) => {
                tracing::error!("[batch::DataFrame] to_json 序列化失败: {e}");
                String::new()
            },
        }
    }
}

pub struct BatchRunner {
    inner: Arc<crate::AStockClient>,
}

/// P3-D12: 批量查询默认并发上限。
///
/// 选 16 的依据：
/// - 单个 HTTP 请求约 200-500ms，16 并发下 200 只股票总耗时约 2.5-6s
/// - 比无限制并发（200+ task 同时跑）显著降低 socket / fd 压力
/// - 比串行（1 并发）快 16 倍，批量场景串行会导致整体超时
pub const BATCH_DEFAULT_CONCURRENCY: usize = 16;

/// P3-D12: 保命模式下的最小并发数（vendor 大面积降级时使用）。
///
/// 不降到 1 的原因：批量场景下串行 200 只股票 × 8s 超时 = 1600s，
/// 远超 `total_timeout`（默认 30s），会全部超时。
/// 2 并发能在保命模式下勉强完成小批量请求。
pub const BATCH_MIN_CONCURRENCY: usize = 2;

impl BatchRunner {
    pub fn new(client: Arc<crate::AStockClient>) -> Self {
        Self { inner: client }
    }

    /// P3-D12: 根据 vendor 健康度解析并发数。
    ///
    /// 算法与 `stock_pipeline::PipelineConfig::resolve_concurrency_with_vendor_health` 一致，
    /// 但批量场景的默认上限更高（`BATCH_DEFAULT_CONCURRENCY = 16`）：
    /// - `healthy_ratio < 0.3`（大面积降级）→ `BATCH_MIN_CONCURRENCY`（保命模式，2 并发）
    /// - `healthy_ratio < 0.6`（部分降级）→ `max(BATCH_MIN, BATCH_DEFAULT / 2)` = 8
    /// - 否则 → `BATCH_DEFAULT_CONCURRENCY`（16）
    ///
    /// `max_concurrent > 0` 时直接返回该值（调用方显式指定，跳过健康度查询）。
    /// 无 vendor 健康数据（首次运行）→ 返回默认值，不阻断业务。
    pub async fn resolve_concurrency(&self, max_concurrent: usize) -> usize {
        if max_concurrent > 0 {
            return max_concurrent;
        }
        let health = self.inner.health_tracker.get_all_health().await;
        Self::concurrency_from_health(&health)
    }

    /// P3-D12: 纯函数版并发解析（便于单元测试）。
    pub fn concurrency_from_health(health: &[crate::vendor_health::VendorHealth]) -> usize {
        use crate::vendor_health::VendorStatus;

        let active: Vec<_> = health.iter().filter(|h| h.status != VendorStatus::Disabled).collect();
        if active.is_empty() {
            return BATCH_DEFAULT_CONCURRENCY;
        }
        let healthy_count = active.iter().filter(|h| h.status == VendorStatus::Healthy).count();
        let healthy_ratio = healthy_count as f64 / active.len() as f64;

        if healthy_ratio < 0.3 {
            BATCH_MIN_CONCURRENCY
        } else if healthy_ratio < 0.6 {
            (BATCH_DEFAULT_CONCURRENCY / 2).max(BATCH_MIN_CONCURRENCY)
        } else {
            BATCH_DEFAULT_CONCURRENCY
        }
    }

    async fn run_batch<T, F, Fut>(
        &self,
        req: BatchRequest,
        method: &str,
        fetcher: F,
    ) -> BatchResult<T>
    where
        F: Fn(String) -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = Result<T, crate::error::DataError>> + Send + 'static,
        T: Send + 'static,
    {
        let start = std::time::Instant::now();
        // P3-D12: 解析并发上限。
        // - `max_concurrent == 0`：根据 vendor 健康度自动决定
        // - `max_concurrent > 0`：显式指定
        // 无限制（unbounded）只在健康度查询失败且无显式值时作为兜底，避免阻塞业务。
        // `req.total_timeout` 与迁移前一致：接受但从不强制。
        let concurrency = self.resolve_concurrency(req.max_concurrent).await;
        run_batch_core(
            start,
            req.codes,
            req.per_stock_timeout,
            req.max_failures,
            concurrency,
            method,
            fetcher,
        )
        .await
    }
}

/// ── 批量执行策略内核（BatchRunner 与 MarketDataProvider trait 实现共用）──────────
///
/// P6.1 迁移：原实现直接 `tokio::spawn` + `Arc<AStockClient>`；trait 实现只能拿到
/// `&self`、凑不齐 spawn 的 `'static` 要求，故把策略循环抽成**当前任务内**轮询的
/// `FuturesUnordered<BoxFuture>` 执行器，future 生命周期泛型化（不要求 `'static`）。
/// 可观测行为与迁移前逐条对齐：
/// - 单只取数 panic：迁移前经 JoinError 让该 code **两个 map 都不出现**；此处
///   `catch_unwind` 捕获后同样静默跳过（`None`），不改变归属；
/// - 提前终止：`max_failures > 0` 且失败数超过阈值 ⇒ break 后 drain 剩余任务；
///   0 ⇒ 收集全部失败（`>` 判定，非 `>=`，与迁移前一致）；
/// - `total_timeout` 从未被强制（迁移前后一致，本函数干脆不接收该参数）；
/// - `elapsed_ms`：由调用方在**解析并发度之前**传入 `start`，健康度查询耗时同样计入。
pub(crate) async fn run_batch_core<'a, T, F, Fut>(
    start: std::time::Instant,
    codes: Vec<String>,
    per_stock_timeout: Duration,
    max_failures: usize,
    concurrency: usize,
    method: &str,
    fetcher: F,
) -> BatchResult<T>
where
    F: Fn(String) -> Fut + Send + Sync + 'a,
    Fut: std::future::Future<Output = Result<T, crate::error::DataError>> + Send + 'a,
    T: Send + 'a,
{
    use futures::FutureExt;
    use std::panic::AssertUnwindSafe;

    let sem = Arc::new(tokio::sync::Semaphore::new(concurrency));

    tracing::info!("[batch:{method}] P3-D12 并发上限 = {concurrency}（codes={}）", codes.len(),);

    // 每只股票一个 boxed future：`None` ⇒ panic 被捕获，按迁移前 JoinError 口径跳过。
    let mut tasks = FuturesUnordered::new();
    let fetcher = &fetcher;
    for code in codes {
        let sem = sem.clone();
        tasks.push(Box::pin(async move {
            // P3-D12: 通过 Semaphore 限制并发，避免一次性发起 200+ 请求打爆 socket / vendor
            let _permit = match sem.acquire_owned().await {
                Ok(p) => p,
                Err(_) => return (code, Some(Err(format!("{method}: Semaphore 已关闭")))),
            };
            // 内层再 Box 一次保证 Unpin，让 timeout 与 catch_unwind 都能安全轮询它
            let mut inner = Box::pin(fetcher(code.clone()));
            let mut timed = Box::pin(tokio::time::timeout(per_stock_timeout, &mut inner));
            match AssertUnwindSafe(&mut timed).catch_unwind().await {
                // 外层 Ok ⇒ 未超时；内层再拆 fetcher 的 Result，失败原因按迁移前
                // `e.to_string()` 口径归因进 failures
                Ok(Ok(Ok(v))) => (code, Some(Ok(v))),
                Ok(Ok(Err(e))) => (code, Some(Err(e.to_string()))),
                Ok(Err(_elapsed)) => {
                    (code, Some(Err(format!("{method}: 单只超时 {per_stock_timeout:?}"))))
                },
                // panic ⇒ 与迁移前 spawn+JoinError 完全同口径：该 code 不进任何 map
                Err(_panic) => (code, None),
            }
        }));
    }

    let mut success = HashMap::new();
    let mut failures = HashMap::new();
    let mut failure_count = 0usize;
    // H1.8 修复:超过 max_failures 阈值时真正 break 终止后续任务
    let mut early_terminated = false;
    while let Some((code, outcome)) = tasks.next().await {
        match outcome {
            Some(Ok(v)) => {
                success.insert(code, v);
            },
            Some(Err(e)) => {
                failure_count += 1;
                failures.insert(code, e);
                if max_failures > 0 && failure_count > max_failures {
                    tracing::warn!(
                        "[batch:{method}] 失败数 {failure_count} 超过阈值 {max_failures}，提前终止后续任务"
                    );
                    early_terminated = true;
                    break;
                }
            },
            // panic 静默跳过（等同迁移前 `let Ok(..) = joined else { continue }`）
            None => continue,
        }
    }

    // H1.8:如提前终止,丢弃仍在运行的剩余任务(避免泄漏)
    if early_terminated {
        while tasks.next().await.is_some() {}
    }

    BatchResult {
        success,
        failures,
        elapsed_ms: start.elapsed().as_millis() as u64,
        early_terminated,
    }
}

// ── P6.1 迁移：MarketDataProvider 批量数据面的进程内入口 ────────────────────
//
// trait 实现只拿得到 `&self`（宿主 state 里 provider 已擦除为 `Arc<dyn _>`），
// 用不了需要 `Arc<AStockClient>` 的 `BatchRunner`；批量循环因此走
// `run_batch_core`（当前任务内轮询）。这还顺带保证外层 `with_optional_asof`
// 压入的 as-of task_local 能被逐只取数读到——spawn 出去的独立任务读不到它。
impl crate::AStockClient {
    /// 逐只并发批量拉实时行情。口径与迁移前宿主 `batch_get_quotes` 命令一致：
    /// 单只默认 8s、`total_timeout_ms` 接受但不生效、`max_failures` 默认 0（不提前终止）、
    /// 并发按 vendor 健康度自适应（16 / 8 / 2 三档）。
    pub async fn batch_get_quotes(
        &self,
        request: &MarketBatchQuotesRequest,
    ) -> BatchResult<StockQuote> {
        let start = std::time::Instant::now();
        let per_stock = request
            .per_stock_timeout_ms
            .map(Duration::from_millis)
            .unwrap_or(Duration::from_secs(8));
        // request 无 max_concurrent 字段（迁移前命令也未暴露）⇒ 恒走健康度自适应
        let concurrency =
            BatchRunner::concurrency_from_health(&self.health_tracker.get_all_health().await);
        run_batch_core(
            start,
            request.codes.clone(),
            per_stock,
            request.max_failures.unwrap_or(0),
            concurrency,
            "get_quote",
            move |code| async move { self.get_quote(&code).await },
        )
        .await
    }

    /// 逐只并发批量拉财务报告序列。口径与迁移前 `BatchRunner::get_financials_batch`
    /// 一致：单只 15s、不提前终止、并发按健康度自适应。
    pub async fn batch_get_financials(
        &self,
        codes: &[String],
    ) -> BatchResult<Vec<FinancialReport>> {
        let start = std::time::Instant::now();
        let concurrency =
            BatchRunner::concurrency_from_health(&self.health_tracker.get_all_health().await);
        run_batch_core(
            start,
            codes.to_vec(),
            Duration::from_secs(15),
            0,
            concurrency,
            "get_financials",
            move |code| async move { self.get_financials(&code).await },
        )
        .await
    }
}

#[async_trait]
impl MarketBatchQuery for BatchRunner {
    async fn get_quotes_batch(&self, req: BatchRequest) -> BatchResult<StockQuote> {
        let client = self.inner.clone();
        self.run_batch(req, "get_quote", move |code| {
            let c = client.clone();
            async move { c.get_quote(&code).await }
        })
        .await
    }

    async fn get_klines_batch(
        &self,
        codes: Vec<String>,
        period: &str,
        limit: u32,
    ) -> BatchResult<Vec<KLine>> {
        let req = BatchRequest::new(codes)
            .with_per_stock_timeout(Duration::from_secs(10))
            .with_total_timeout(Duration::from_secs(60))
            .with_max_failures(0);
        let period = period.to_string();
        let client = self.inner.clone();
        self.run_batch(req, "get_klines", move |code| {
            let c = client.clone();
            let p = period.clone();
            async move { c.get_klines(&code, &p, limit).await }
        })
        .await
    }

    async fn get_financials_batch(&self, codes: Vec<String>) -> BatchResult<Vec<FinancialReport>> {
        let req = BatchRequest::new(codes)
            .with_per_stock_timeout(Duration::from_secs(15))
            .with_total_timeout(Duration::from_secs(120))
            .with_max_failures(0);
        let client = self.inner.clone();
        self.run_batch(req, "get_financials", move |code| {
            let c = client.clone();
            async move { c.get_financials(&code).await }
        })
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dataframe_from_quotes_columns_order() {
        let q = StockQuote {
            code: "600519".into(),
            name: "贵州茅台".into(),
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
            timestamp: "2026-01-15 14:00:00".into(),
        };
        let df = DataFrame::from_quotes(&[q]);
        assert_eq!(df.columns.len(), 16);
        assert_eq!(df.columns[0], "code");
        assert_eq!(df.columns[3], "high");
        assert_eq!(df.columns[5], "close");
        assert_eq!(df.rows.len(), 1);
        assert!(df.rows[0][11].is_number());
    }

    #[test]
    fn batch_request_defaults() {
        let r = BatchRequest::default();
        assert_eq!(r.per_stock_timeout, Duration::from_secs(8));
        assert_eq!(r.max_failures, 0);
        let r = BatchRequest::new(vec!["600519".into()])
            .with_per_stock_timeout(Duration::from_secs(3))
            .with_max_failures(2);
        assert_eq!(r.codes.len(), 1);
        assert_eq!(r.per_stock_timeout, Duration::from_secs(3));
        assert_eq!(r.max_failures, 2);
    }

    #[test]
    fn batch_result_is_all_success() {
        let mut r: BatchResult<i32> = BatchResult::new();
        r.success.insert("a".into(), 1);
        assert!(r.is_all_success());
        r.failures.insert("b".into(), "err".into());
        assert!(!r.is_all_success());
        assert_eq!(r.total(), 2);
    }

    /// P3-D12: 验证并发数解析的纯函数逻辑
    #[test]
    fn d12_concurrency_from_health_all_healthy() {
        use crate::vendor_health::{VendorHealth, VendorStatus};
        let health = vec![
            VendorHealth { status: VendorStatus::Healthy, ..VendorHealth::new("tencent") },
            VendorHealth { status: VendorStatus::Healthy, ..VendorHealth::new("eastmoney") },
        ];
        assert_eq!(BatchRunner::concurrency_from_health(&health), BATCH_DEFAULT_CONCURRENCY);
    }

    #[test]
    fn d12_concurrency_from_health_partial_degraded() {
        use crate::vendor_health::{VendorHealth, VendorStatus};
        // 5 个 vendor，1 healthy + 4 degraded → healthy_ratio=0.2 < 0.3 → 保命模式
        let health = vec![
            VendorHealth { status: VendorStatus::Healthy, ..VendorHealth::new("tencent") },
            VendorHealth { status: VendorStatus::Degraded, ..VendorHealth::new("eastmoney") },
            VendorHealth { status: VendorStatus::Degraded, ..VendorHealth::new("sina") },
            VendorHealth { status: VendorStatus::Degraded, ..VendorHealth::new("ths") },
            VendorHealth { status: VendorStatus::Degraded, ..VendorHealth::new("akshare") },
        ];
        assert_eq!(BatchRunner::concurrency_from_health(&health), BATCH_MIN_CONCURRENCY);

        // 5 个 vendor，2 healthy + 3 degraded → healthy_ratio=0.4 ∈ [0.3, 0.6) → 降速
        let health = vec![
            VendorHealth { status: VendorStatus::Healthy, ..VendorHealth::new("tencent") },
            VendorHealth { status: VendorStatus::Healthy, ..VendorHealth::new("eastmoney") },
            VendorHealth { status: VendorStatus::Degraded, ..VendorHealth::new("sina") },
            VendorHealth { status: VendorStatus::Degraded, ..VendorHealth::new("ths") },
            VendorHealth { status: VendorStatus::Degraded, ..VendorHealth::new("akshare") },
        ];
        assert_eq!(
            BatchRunner::concurrency_from_health(&health),
            (BATCH_DEFAULT_CONCURRENCY / 2).max(BATCH_MIN_CONCURRENCY)
        );
    }

    #[test]
    fn d12_concurrency_from_health_all_disabled() {
        use crate::vendor_health::{VendorHealth, VendorStatus};
        // 全部 Disabled → active 为空 → 返回默认值（不阻断业务）
        let health = vec![
            VendorHealth { status: VendorStatus::Disabled, ..VendorHealth::new("tencent") },
            VendorHealth { status: VendorStatus::Disabled, ..VendorHealth::new("eastmoney") },
        ];
        assert_eq!(BatchRunner::concurrency_from_health(&health), BATCH_DEFAULT_CONCURRENCY);
    }

    #[test]
    fn d12_concurrency_from_health_empty() {
        // 无 vendor 健康数据（首次运行）→ 返回默认值
        let health: Vec<crate::vendor_health::VendorHealth> = vec![];
        assert_eq!(BatchRunner::concurrency_from_health(&health), BATCH_DEFAULT_CONCURRENCY);
    }

    #[test]
    fn d12_batch_request_with_max_concurrent() {
        let r = BatchRequest::new(vec!["600519".into()]).with_max_concurrent(4);
        assert_eq!(r.max_concurrent, 4);

        // 默认值 0 = 自动
        let r = BatchRequest::default();
        assert_eq!(r.max_concurrent, 0);
    }
}
