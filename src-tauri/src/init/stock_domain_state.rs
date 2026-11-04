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

impl crate::AppState {
    /// 股票域运行时状态（B6：替代原先六个 `AppState` 字段）。
    pub fn stock(&self) -> &'static StockDomainState {
        stock_domain()
    }
}
