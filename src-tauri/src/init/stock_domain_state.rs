// SPDX-License-Identifier: AGPL-3.0-only

//! 股票域运行时状态（B6：从 `AppState` 摘出六个域字段）。
//!
//! # 为什么
//!
//! `AppState` 是主壳装配结构；股票域的客户端 / 监视器 / 引擎一旦作为它的字段，
//! `app_state.rs` 与 `state.rs` 就被钉在域 crate 上（`l1ForeignStock` 的 14 处），
//! 域整体迁出为插件时这两个文件必须跟着改。改为「域自有模块持有 + 装配期注入」后：
//! - `app_state.rs` 不再出现任何域 crate 路径（字段声明与类型全部移到这里）；
//! - `state.rs` 只做一次 [`init_stock_domain_state`] 注入；
//! - 命令层取用方式改为 `state.stock().<字段>`（[`AppState::stock`] 是 4 行胶水，
//!   返回 `&'static`，不引入临时 Arc 跨 `.await` 的借用问题）。
//!
//! ⚠ 进程级单例（与 `global_state` / `sandbox_policy` 同形），首次注入生效：
//! 生产路径只注入一次；同进程重复 `create_app_state` 会保留首份（测试场景下
//! 股票字段无人访问，只影响「多 AppState 共用首份域状态」这一不可达情形）。

use std::sync::{Arc, OnceLock};

// ── 域侧门面（供 `init/services.rs` 与离线 bin 经 `axagent_lib` 取用）────────────
// 权威定义都在域 crate 内；宿主只保留**指向域模块的转发点**。
// 落点必须是本模块（`ownedRe` 命中 `init/stock*`），不能写进 `lib.rs` —— 否则主壳
// 又会因门面自己重新被钉在域 crate 上（`l1ForeignStock` 原地不降）。
pub use axagent_analysis_engine::alert_mapping as stock_alert_mapping;
pub use axagent_analysis_engine::conditional_order_bridge as stock_conditional_order_bridge;
pub use axagent_analysis_engine::cross_stock_aggregator as stock_cross_stock_aggregator;
pub use axagent_analysis_engine::evolution_drift as stock_evolution_drift;
pub use axagent_analysis_engine::market_close_store as stock_market_close_store;
pub use axagent_analysis_engine::monitor as stock_monitor;
pub use axagent_analysis_engine::recommender as stock_recommender;
pub use axagent_analysis_engine::serenity_hit_rate_backtest as stock_serenity_hit_rate_backtest;
pub use axagent_astock_data::AStockClient;
pub use axagent_astock_data::as_of as stock_as_of;
pub use axagent_astock_data::daily_snapshot as stock_daily_snapshot;
pub use axagent_astock_data::health_check as stock_health_check;
pub use axagent_astock_data::realtime_quote as stock_realtime_quote;
pub use axagent_stock_core::{
    costs as stock_costs, factors as stock_factors, hit_outcome as stock_hit_outcome,
};

/// 股票域运行时状态。
pub struct StockDomainState {
    /// A 股数据客户端（行情 / K 线 / 财报 / 自选）。
    pub astock_client: Arc<axagent_astock_data::AStockClient>,
    /// 实时行情监视器（OnceLock 惰性初始化；未初始化时相关命令返回友好错误）。
    pub stock_monitor: OnceLock<Arc<axagent_analysis_engine::monitor::RealtimeMonitor>>,
    /// 实时行情流监视器（`quote.watcher` 接缝宿主 provider 与轮询循环共用同一个集合）。
    pub quote_watcher: OnceLock<Arc<axagent_astock_data::RealTimeQuoteWatcher>>,
    /// 模拟交易引擎（依赖 DB + astock_client，启动时构造）。
    pub trading_engine: Arc<tokio::sync::RwLock<axagent_analysis_engine::trading::TradingEngine>>,
    /// 跨股聚合器（OnceLock 惰性初始化）。
    pub cross_stock_aggregator:
        OnceLock<Arc<axagent_analysis_engine::cross_stock_aggregator::CrossStockSignalAggregator>>,
    /// 股票自适应引擎（反思 + 进化 + 编排闭环）。
    pub stock_adaptive_engine:
        Arc<axagent_analysis_engine::stock_adaptive_engine::StockAdaptiveEngine>,
}

impl StockDomainState {
    /// 装配期构造（B7）：两个引擎的**构造**下沉到域自有模块。
    ///
    /// 主壳只把「客户端 + DB」递进来，`state.rs` 不再出现
    /// `axagent_analysis_engine::trading::TradingEngine` /
    /// `stock_adaptive_engine::StockAdaptiveEngine` 等域 crate 路径；
    /// `trading.engine` 接缝注册所需的同一个 `Arc` 由调用方从
    /// [`Self::trading_engine`] clone（与 jobs.store / workflow.run 同款
    /// 「clone 注册、按值持有」）。
    pub fn new(
        astock_client: Arc<axagent_astock_data::AStockClient>,
        db: sea_orm::DatabaseConnection,
    ) -> Self {
        let trading_engine = Arc::new(tokio::sync::RwLock::new(
            axagent_analysis_engine::trading::TradingEngine::new(
                Arc::new(db.clone()),
                astock_client.clone(),
            ),
        ));
        let stock_adaptive_engine = Arc::new(
            axagent_analysis_engine::stock_adaptive_engine::StockAdaptiveEngine::new().with_db(db),
        );
        Self {
            astock_client,
            stock_monitor: OnceLock::new(),
            quote_watcher: OnceLock::new(),
            trading_engine,
            cross_stock_aggregator: OnceLock::new(),
            stock_adaptive_engine,
        }
    }
}

static STATE: OnceLock<Box<StockDomainState>> = OnceLock::new();

/// 注入股票域运行时状态（`create_app_state` 装配时调用一次）。
///
/// 重复注入保留首份（见模块文档的进程级单例说明）。
pub fn init_stock_domain_state(state: StockDomainState) {
    let _ = STATE.set(Box::new(state));
}

/// 取股票域运行时状态。
///
/// 未注入时 panic 并点名缺失的接线 —— 这是装配错误，不该静默退化为空实现。
pub fn stock_domain() -> &'static StockDomainState {
    STATE.get().map(|b| b.as_ref()).unwrap_or_else(|| {
        panic!(
            "股票域运行时状态未初始化（wiring 缺失：create_app_state 须调用 \
             init_stock_domain_state）"
        )
    })
}

/// 取股票域运行时状态的**非 panic** 版本。
///
/// 供「未注入时本来就有合法降级路径」的调用方使用（例：`workflow_run_service`
/// 的单测不跑 `create_app_state`，旧 `global_state` 槽位在那里同样返回 `None`）。
pub fn try_stock_domain() -> Option<&'static StockDomainState> {
    STATE.get().map(|b| b.as_ref())
}

impl crate::AppState {
    /// 股票域运行时状态（B6：替代原先六个 `AppState` 字段）。
    pub fn stock(&self) -> &'static StockDomainState {
        stock_domain()
    }
}

// ── 客户端构造与启动期域装配（B7：主壳不再命名域类型）──────────────

/// A 股客户端的 L2 磁盘缓存 flush 句柄。
///
/// 包一层是为了让 `state.rs` 不出现 `astock_data::disk_cache::FlushLoopHandle`
/// 这个名字；优雅退出语义与直连逐字一致（`shutdown_and_join`）。
pub struct DiskCacheFlush(axagent_astock_data::disk_cache::FlushLoopHandle);

impl DiskCacheFlush {
    /// 触发停止并等待最后一次落盘完成（关机路径调用）。
    pub async fn shutdown_and_join(self) {
        self.0.shutdown_and_join().await;
    }
}

/// 构造 A 股数据客户端，并把 L2 磁盘缓存接上。
///
/// 返回客户端与它的 flush 句柄：句柄必须由调用方持有到进程退出（否则后台
/// 任务被提前 join 掉、脏页不落盘）。其余注入（browser fetcher / 快照归档 /
/// 新闻归档）仍由装配层按原顺序链式完成。
pub fn create_astock_client(
    l2_path: std::path::PathBuf,
) -> (axagent_astock_data::AStockClient, DiskCacheFlush) {
    let (client, l2) = axagent_astock_data::AStockClient::new().with_l2_cache(l2_path);
    let flush = DiskCacheFlush(axagent_astock_data::disk_cache::spawn_flush_loop(l2));
    (client, flush)
}

/// 启动期加载决策否决门开关（settings → 进程内快照），并打统一日志。
///
/// 读失败**不阻断启动**：保持默认（全关 = 与引入前逐位一致）。
pub async fn refresh_decision_gates(db: &sea_orm::DatabaseConnection) {
    match axagent_analysis_engine::decision_gates::refresh_from_db(db).await {
        Ok(states) => {
            let names = ["风控门", "情绪门", "估值门", "顶层采纳档内买入"];
            let opened: Vec<String> = states
                .iter()
                .zip(names.iter())
                .filter(|(open, _)| **open)
                .map(|(_, n)| (*n).to_string())
                .collect();
            if opened.is_empty() {
                tracing::info!("[startup] 决策否决门：四门全关（保持原行为）");
            } else {
                tracing::info!("[startup] 决策否决门：已放开 {}", opened.join(" / "));
            }
        },
        Err(e) => {
            tracing::warn!(
                "[startup] 决策否决门开关加载失败（保持默认全关，与引入前行为一致）: {e}"
            );
        },
    }
}
