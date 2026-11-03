// SPDX-License-Identifier: AGPL-3.0-only

//! `stock.monitor` 接缝的**宿主侧实现**（wiring 层，宿主独持 `RealtimeMonitor` + 聚合器）。
//!
//! 契约在 `axagent-core/crates/harness/src/stock_monitor.rs`（已冻结）。九个 op 里四个是写面
//! （`set_t0_config` / `upsert_alert` / `remove_alert` / `set_aggregator_config`），权限闸在
//! worker 分派层（`PLUGIN_PERMISSION_DENIED`），本层不重复校验。全部 op **不收 `as_of`**：
//! 读的是内存 / DB，或由宿主轮询循环自己去取数。
//!
//! ## 与命令层的关系（禁区 12）
//!
//! - `upsert_alert` **直接调** `commands::price_alert_upsert::upsert_price_alert_and_wire`，
//!   与面板命令 `create_price_alert`、agent 工具 `stock_create_price_alert` 同一份实现。
//!   再抄一遍「归一化 → 落库 → 造配置 → 进引擎」就会分叉（裁定 15 之前正是如此）。
//! - `remove_alert` 照 `commands::stock_analysis.rs` 的 `delete_price_alert` 三步
//!   （先查 → 删库 → 摘内存配置，连带回收冷却 / 边沿状态键）。SQL 落在 dao
//!   （`axagent_dao::repo::price_alert`），本文件不出现 `sea_orm::` / `axagent_entities::`
//!   字面量 —— 分层门 `check-layer-discipline.mjs` 规则 1 对新文件基线为 0。
//! - `status` 的九键映射抽成纯函数 [`status_view`]，与命令层 `monitor_status` 同口径。
//!
//! ## clamp 的唯一来源（裁定 N4）
//!
//! 聚合器四区间只引用 harness 的 `AGGREGATOR_*` 常量，本文件**不写第二套数字**；越界一律
//! 拒收（`Err`），不静默夹取 —— 静默夹取会让插件以为自己设成了那个被夹掉的值。
//! T+0 的两阈值与 `min_interval_minutes` 按契约文档（trait `set_t0_config` 注释）判：
//! 阈值必须 `> 0`、间隔必须 `≥ 1`（0 等于关掉防风暴那道闸）。
//!
//! ## 引擎未装配
//!
//! 本 provider 只在 `init/services.rs` 装配成功后就地注册（裁定 J8），故 `assembled` 恒
//! `true`；「未装配」这一缺席状态在接缝上是**注册表取不到 provider** ⇒ 分派层落
//! `SEAM_PROVIDER_UNAVAILABLE`，不是 provider 报错。`status` 仍经 [`status_view`] 保留
//! `assembled = false` 那一格的形状（计数全 0），并由单测钉住，免得上收时把这格丢掉。
//!
//! ## `alert_type` 先拒后归一
//!
//! `normalize_alert_type` 对认不出的输入**兜底成 `take_profit`**（面板那条路靠人在选控件里
//! 挑，可以宽容）。插件侧没有这道兜底：一条拼错的串会落成**反方向**的单 ⇒ 必须在归一之前
//! 按白名单拒绝。白名单 = 6 个标准值 + 5 个 legacy 写法；legacy 那一组不另抄字面量，直接问
//! 权威函数 `legacy_condition_to_alert_type` 认不认（它的键集合就是那 5 个串）。

use std::sync::Arc;

use async_trait::async_trait;
use axagent_analysis_engine::alert_mapping::{alert_types, legacy_condition_to_alert_type};
use axagent_analysis_engine::cross_stock_aggregator::CrossStockSignalAggregator;
use axagent_analysis_engine::monitor::RealtimeMonitor;
use axagent_dao::repo::price_alert::{self as alert_dao, AlertRow};
use axagent_harness::stock_monitor::{
    AGGREGATOR_COOLDOWN_SECS, AGGREGATOR_MIN_SIGNAL_COUNT, AGGREGATOR_MIN_STRENGTH,
    AGGREGATOR_WINDOW_SECS,
};
use axagent_harness::{
    AggregatorConfig, AlertUpsertOutcome, AlertUpsertRequest, AxAgentError, MonitorConfig,
    MonitorStatusView, PollRoundStats, PriceAlertView, RemoveAlertOutcome, StockMonitorProvider,
    StockSignal, TZeroConfig,
};

use crate::commands::price_alert_upsert::upsert_price_alert_and_wire;

/// 六个标准告警类型（取自 `alert_mapping::alert_types` 的常量，不重抄字符串）。
const STANDARD_ALERT_TYPES: &[&str] = &[
    alert_types::STOP_LOSS,
    alert_types::TAKE_PROFIT,
    alert_types::RESISTANCE,
    alert_types::SUPPORT,
    alert_types::CHANGE,
    alert_types::VOLUME,
];

/// `stock.monitor` 的宿主实现。
pub struct HostStockMonitor {
    /// 与 `AppState.stock_monitor` **同一个 `Arc`**（克隆持有，不反向引用 AppState）。
    monitor: Arc<RealtimeMonitor>,
    /// 与 `AppState.cross_stock_aggregator` 同一个 `Arc`。
    ///
    /// ⚠ 聚合器的 `feed` / `clear` / `set_snapshot_provider` 不过线（契约的「主权边界」）：
    /// 本字段只读配置与缓冲，写它们才是本接缝的全部权限。
    aggregator: Arc<CrossStockSignalAggregator>,
    /// 与 AppState 的 DB 同一连接池（`upsert_alert` / `remove_alert` 用）。
    db: Arc<axagent_dao::DatabaseConnection>,
}

impl HostStockMonitor {
    pub fn new(
        monitor: Arc<RealtimeMonitor>,
        aggregator: Arc<CrossStockSignalAggregator>,
        db: Arc<axagent_dao::DatabaseConnection>,
    ) -> Self {
        Self { monitor, aggregator, db }
    }
}

/// 告警类型是否落在白名单内（6 标准 + 5 legacy）。
///
/// 未知串 ⇒ `false` ⇒ 调用方拒绝，**绝不**交给 `normalize_alert_type` 归一（它会兜底成
/// `take_profit`，把「拼错」变成一张反方向的单）。
fn is_allowed_alert_type(input: &str) -> bool {
    STANDARD_ALERT_TYPES.contains(&input) || legacy_condition_to_alert_type(input).is_some()
}

/// 把运行读数折成契约视图（纯函数，`assembled = false` 那一格由单测钉住）。
///
/// 命令层 `monitor_status` 与本接缝读的是同一组引擎方法，键集合逐字一致（harness
/// `MonitorStatusView` 就是那份 DTO 的迁入体）；九个字段一一对应，不做任何收敛（H-4）。
fn status_view(
    assembled: bool,
    session_code: &str,
    trading_now: bool,
    poll_interval_secs: u64,
    configs_active: u32,
    stats: &PollRoundStats,
) -> MonitorStatusView {
    MonitorStatusView {
        assembled,
        session: session_code.to_string(),
        trading_now,
        poll_interval_secs,
        configs_active,
        round_codes_total: stats.codes_total,
        round_codes_failed: stats.codes_failed,
        consecutive_all_failed_rounds: stats.consecutive_all_failed_rounds,
        round_finished_at_ms: stats.finished_at_ms,
    }
}

/// T+0 配置的合法性判据（越界 ⇒ `Err`，由分派层落 `SEAM_CALL_INVALID_ARGS`）。
///
/// NaN 显式判：`v <= 0.0` 对 NaN 恒 `false`，只写它会把 NaN 放过去。
fn validate_t0_config(config: &TZeroConfig) -> Result<(), AxAgentError> {
    if config.change_pct_threshold.is_some_and(|v| v.is_nan() || v <= 0.0) {
        return Err(AxAgentError::Validation(format!(
            "T+0 的 changePctThreshold 必须 > 0（实收 {:?}）",
            config.change_pct_threshold
        )));
    }
    if config.turnover_rate_threshold.is_some_and(|v| v.is_nan() || v <= 0.0) {
        return Err(AxAgentError::Validation(format!(
            "T+0 的 turnoverRateThreshold 必须 > 0（实收 {:?}）",
            config.turnover_rate_threshold
        )));
    }
    if config.min_interval_minutes < 1 {
        return Err(AxAgentError::Validation(format!(
            "T+0 的 minIntervalMinutes 必须 ≥ 1（实收 {}；0 等于关掉同股防风暴那道闸）",
            config.min_interval_minutes
        )));
    }
    Ok(())
}

/// 聚合器配置的合法性判据：**只**引用 harness 的 `AGGREGATOR_*` 常量（裁定 N4）。
///
/// `min_strength` 用 `RangeInclusive::contains` 判：NaN 恒 `false` ⇒ 拒收，正是这里想要的
/// 结果（NaN 强度等于把「低于阈值不参与聚合」这道闸整条关掉）。
fn validate_aggregator_config(config: &AggregatorConfig) -> Result<(), AxAgentError> {
    if !(AGGREGATOR_WINDOW_SECS.0..=AGGREGATOR_WINDOW_SECS.1).contains(&config.window_secs) {
        return Err(AxAgentError::Validation(format!(
            "windowSecs = {} 越界，合法区间 {}..={}",
            config.window_secs, AGGREGATOR_WINDOW_SECS.0, AGGREGATOR_WINDOW_SECS.1
        )));
    }
    if !(AGGREGATOR_MIN_SIGNAL_COUNT.0..=AGGREGATOR_MIN_SIGNAL_COUNT.1)
        .contains(&config.min_signal_count)
    {
        return Err(AxAgentError::Validation(format!(
            "minSignalCount = {} 越界，合法区间 {}..={}（取 1 就等于关掉同向聚集规则）",
            config.min_signal_count, AGGREGATOR_MIN_SIGNAL_COUNT.0, AGGREGATOR_MIN_SIGNAL_COUNT.1
        )));
    }
    if !(AGGREGATOR_COOLDOWN_SECS.0..=AGGREGATOR_COOLDOWN_SECS.1).contains(&config.cooldown_secs) {
        return Err(AxAgentError::Validation(format!(
            "cooldownSecs = {} 越界，合法区间 {}..={}（下界 0 是合法的「不去重」）",
            config.cooldown_secs, AGGREGATOR_COOLDOWN_SECS.0, AGGREGATOR_COOLDOWN_SECS.1
        )));
    }
    if !(AGGREGATOR_MIN_STRENGTH.0..=AGGREGATOR_MIN_STRENGTH.1).contains(&config.min_strength) {
        return Err(AxAgentError::Validation(format!(
            "minStrength = {:?} 越界，合法区间 {}..={}（NaN 一并拒收）",
            config.min_strength, AGGREGATOR_MIN_STRENGTH.0, AGGREGATOR_MIN_STRENGTH.1
        )));
    }
    Ok(())
}

/// `price_alerts` 行 → 契约视图（12 列逐字镜像，`isTriggered` 保持 `i32`）。
///
/// ORM 类型不过线：`entities` 依赖 `harness`，反向过线即依赖成环（同
/// `axagent_harness::trading_engine::TradeView` 的理由，裁定 N3）。
fn price_alert_view(row: &AlertRow) -> PriceAlertView {
    PriceAlertView {
        id: row.id.clone(),
        stock_code: row.stock_code.clone(),
        stock_name: row.stock_name.clone(),
        condition: row.condition.clone(),
        target_price: row.target_price,
        alert_type: row.alert_type.clone(),
        condition_type: row.condition_type.clone(),
        threshold: row.threshold,
        is_triggered: row.is_triggered,
        triggered_at: row.triggered_at,
        created_at: row.created_at,
        updated_at: row.updated_at,
    }
}

/// 引擎内存里是否正盯着这一条（`memory_removed` 的判据，而不是「调用没报错」）。
async fn monitors_alert(monitor: &RealtimeMonitor, alert_id: &str) -> bool {
    monitor.list_configs().await.iter().any(|c| c.alert_id == alert_id)
}

#[async_trait]
impl StockMonitorProvider for HostStockMonitor {
    async fn status(&self) -> Result<MonitorStatusView, AxAgentError> {
        let session = axagent_astock_data::calendar::current_trading_session();
        let stats = self.monitor.poll_round_stats().await;
        let configs_active =
            self.monitor.list_configs().await.into_iter().filter(|c| c.enabled).count() as u32;
        // 本 provider 只在装配成功后注册 ⇒ assembled 恒 true（见文件头「引擎未装配」一节）
        Ok(status_view(
            true,
            session.code(),
            session.is_open(),
            self.monitor.poll_interval_secs().await,
            configs_active,
            &stats,
        ))
    }

    async fn list_configs(&self) -> Result<Vec<MonitorConfig>, AxAgentError> {
        // 引擎的 configs 是 HashMap ⇒ 无序。与 `aggregator_buffer` / `list_watched` 同口径
        // 先排序（按 alert_id），否则插件侧断言不可复现。
        let mut configs = self.monitor.list_configs().await;
        configs.sort_by_key(|c| c.alert_id.clone());
        Ok(configs)
    }

    async fn t0_config(&self) -> Result<TZeroConfig, AxAgentError> {
        Ok(self.monitor.t0_config().await)
    }

    async fn set_t0_config(&self, config: TZeroConfig) -> Result<(), AxAgentError> {
        // ⚠ 全局开关、无 owner 隔离：开启后每轮告警都可能经宿主回调启动工作流，
        // 影响所有插件与前端看到的宿主行为（契约 trait 注释点名的那一格）。
        validate_t0_config(&config)?;
        self.monitor.set_t0_config(config).await;
        Ok(())
    }

    async fn upsert_alert(
        &self,
        request: AlertUpsertRequest,
    ) -> Result<AlertUpsertOutcome, AxAgentError> {
        // 先拒后归一（见文件头）；NaN / 无穷大的阈值同样拒，落库后每轮求值都判不出意义。
        if !is_allowed_alert_type(&request.alert_type) {
            return Err(AxAgentError::Validation(format!(
                "alertType = {:?} 不在白名单（6 个标准值 + above/below/change_up/change_down/volume_spike 五个 legacy 写法），已拒绝落库",
                request.alert_type
            )));
        }
        if !request.threshold.is_finite() {
            return Err(AxAgentError::Validation(format!(
                "threshold = {:?} 不是有限数值，已拒绝落库",
                request.threshold
            )));
        }

        // 落库 + 进引擎走同一份实现；`monitor` 是本结构持有的 Arc ⇒ 恒 Some，
        // `wired` 为 false 只可能是「六类之外落不成配置」或引擎侧的既成事实，原样透出。
        let (row, reused, wired) = upsert_price_alert_and_wire(
            self.db.as_ref(),
            Some(&self.monitor),
            &request.stock_code,
            &request.stock_name,
            &request.alert_type,
            request.threshold,
        )
        .await
        .map_err(|e| AxAgentError::Provider(format!("价格告警落库/接线失败: {e}")))?;

        Ok(AlertUpsertOutcome { alert: price_alert_view(&row), reused, wired })
    }

    async fn remove_alert(&self, alert_id: &str) -> Result<RemoveAlertOutcome, AxAgentError> {
        // ① 先查（拿这一行，并区分「查无」与「删成」）
        let row = alert_dao::find_by_id(self.db.as_ref(), alert_id)
            .await
            .map_err(|e| AxAgentError::Provider(format!("查询价格告警失败: {e}")))?;
        let Some(row) = row else {
            // 行不存在 ⇒ 两个 false 的正常响应（照 jobs.store get 的「查无即 null」口径）
            return Ok(RemoveAlertOutcome { db_deleted: false, memory_removed: false });
        };

        // ② 删库
        let deleted = alert_dao::delete_by_id(self.db.as_ref(), &row.id)
            .await
            .map_err(|e| AxAgentError::Provider(format!("删除价格告警失败: {e}")))?;

        // ③ 摘内存配置（连带回收它的冷却 / 边沿状态键，否则那两个 HashMap 只涨不落）
        let memory_removed = monitors_alert(self.monitor.as_ref(), &row.id).await;
        if memory_removed {
            self.monitor.remove_config_by_id(&row.id).await;
        }

        Ok(RemoveAlertOutcome { db_deleted: deleted > 0, memory_removed })
    }

    async fn aggregator_config(&self) -> Result<AggregatorConfig, AxAgentError> {
        Ok(self.aggregator.config().await)
    }

    async fn set_aggregator_config(&self, config: AggregatorConfig) -> Result<(), AxAgentError> {
        // 这四个数是组合告警可信度的全部防线 ⇒ 越界拒收、不静默夹取（区间只在 harness）
        validate_aggregator_config(&config)?;
        self.aggregator.set_config(config).await;
        Ok(())
    }

    async fn aggregator_buffer(&self) -> Result<Vec<StockSignal>, AxAgentError> {
        // 引擎缓冲是 HashMap（键 = stock_code）⇒ 无序；裁定 N5：provider 侧排序。
        // 键 = `(stock_code, timestamp)`：同一条信号在不同运行里给同一个位置。
        let mut signals = self.aggregator.buffer_snapshot().await;
        signals.sort_by_key(|s| (s.stock_code.clone(), s.timestamp));
        Ok(signals)
    }
}

#[cfg(test)]
mod tests {
    //! 宿主 provider 的形态锁：白名单、clamp 单一源、截断之外的「查无即双 false」、
    //! 以及缓冲 / 配置列表的**有序**（HashMap 无序 ⇒ 不排序则插件侧断言不可复现）。

    use super::*;
    use axagent_harness::SignalType;
    use axagent_harness::market_data::{AdjType, KLine, MarketDataProvider, StockQuote};

    /// 只回固定行情的取数替身（本模块锁的是接缝形状，不是取数）。
    struct StubProvider;

    #[async_trait::async_trait]
    impl MarketDataProvider for StubProvider {
        async fn get_quote(
            &self,
            stock_code: &str,
        ) -> axagent_harness::core_error::Result<StockQuote> {
            Ok(StockQuote {
                code: stock_code.to_string(),
                name: "测试股".to_string(),
                price: 12.34,
                pre_close: 12.0,
                open: 12.1,
                high: 12.5,
                low: 11.9,
                volume: 1_000_000.0,
                amount: 12_340_000.0,
                change_pct: 2.83,
                turnover_rate: 1.5,
                pe: Some(10.0),
                pb: Some(1.2),
                total_mv: Some(1.2e10),
                circulating_mv: Some(1.0e10),
                limit_up: Some(13.2),
                limit_down: Some(10.8),
                is_st: false,
                timestamp: String::new(),
            })
        }

        async fn get_klines(
            &self,
            _stock_code: &str,
            _period: &str,
            _limit: u32,
            _adj_type: Option<AdjType>,
        ) -> axagent_harness::core_error::Result<Vec<KLine>> {
            Err(AxAgentError::Provider("mock: 接缝单测不该取 K 线".into()))
        }

        async fn search_stock(
            &self,
            _keyword: &str,
        ) -> axagent_harness::core_error::Result<Vec<axagent_harness::market_data::StockSearchResult>>
        {
            Err(AxAgentError::Provider("mock: 接缝单测不该搜股".into()))
        }
    }

    fn monitor() -> Arc<RealtimeMonitor> {
        Arc::new(RealtimeMonitor::new(Arc::new(StubProvider)))
    }

    fn aggregator() -> Arc<CrossStockSignalAggregator> {
        Arc::new(CrossStockSignalAggregator::new(AggregatorConfig::default()))
    }

    fn sample_config() -> TZeroConfig {
        TZeroConfig {
            enabled: true,
            change_pct_threshold: Some(3.0),
            turnover_rate_threshold: Some(8.0),
            min_interval_minutes: 30,
        }
    }

    fn request(alert_type: &str) -> AlertUpsertRequest {
        AlertUpsertRequest {
            stock_code: "600519".to_string(),
            stock_name: "贵州茅台".to_string(),
            alert_type: alert_type.to_string(),
            threshold: 100.0,
        }
    }

    // ── 白名单 ──────────────────────────────────────────────────────────

    /// 6 个标准值 + **5 个** legacy 写法全部放行（res104 记的「4 个 legacy」不准）。
    #[test]
    fn alert_type_whitelist_covers_standard_and_all_five_legacy_spellings() {
        for ok in [
            "stop_loss",
            "take_profit",
            "resistance",
            "support",
            "change",
            "volume",
            "above",
            "below",
            "change_up",
            "change_down",
            "volume_spike",
        ] {
            assert!(is_allowed_alert_type(ok), "{ok} 应在白名单内");
        }
    }

    /// 认不得的一律拒：不得继承 `normalize_alert_type` 静默回落 `take_profit` 的宽容。
    #[test]
    fn unknown_alert_type_is_rejected_not_normalised() {
        for bad in ["", " STOP_LOSS", "STOP_LOSS", "take_profit ", "above_or_equal", "bg", "止盈"]
        {
            assert!(!is_allowed_alert_type(bad), "未知写法 {bad:?} 必须被拒绝");
            // 反证：归一函数本身会把这些串兜底成 take_profit ⇒ 白名单必须在它之前
            assert_eq!(
                axagent_analysis_engine::alert_mapping::normalize_alert_type(bad),
                alert_types::TAKE_PROFIT,
                "{bad:?} 若落到归一函数会被静默当止盈单"
            );
        }
    }

    #[tokio::test]
    async fn upsert_alert_rejects_unknown_type_before_touching_the_database() {
        let provider = HostStockMonitor::new(monitor(), aggregator(), Arc::new(db().await));
        let err =
            provider.upsert_alert(request("sell_above")).await.expect_err("白名单外的类型必须拒收");
        assert!(
            matches!(err, AxAgentError::Validation(_)),
            "非法入参应是 Validation（分派层映射拒绝码），实际 {err:?}"
        );
    }

    // ── clamp 单一源（裁定 N4）─────────────────────────────────────────

    /// 区间数字住在契约层：provider 只引用它们，本条把它们钉在测试里，漂移即红。
    #[test]
    fn aggregator_ranges_are_the_harness_numbers() {
        assert_eq!(AGGREGATOR_WINDOW_SECS, (1, 3600));
        assert_eq!(AGGREGATOR_MIN_SIGNAL_COUNT, (2, 50));
        assert_eq!(AGGREGATOR_COOLDOWN_SECS, (0, 86400));
        assert_eq!(AGGREGATOR_MIN_STRENGTH, (0.0, 1.0));
        // 默认值必须落在自己的区间内（否则一启动就「配置非法」）
        assert!(validate_aggregator_config(&AggregatorConfig::default()).is_ok());
    }

    #[test]
    fn aggregator_config_out_of_range_is_rejected_not_silently_clamped() {
        let base = AggregatorConfig::default();

        let mut too_wide = base.clone();
        too_wide.window_secs = 0;
        assert!(matches!(validate_aggregator_config(&too_wide), Err(AxAgentError::Validation(_))));

        let mut single_signal = base.clone();
        // 下界刻意取 2：取 1 就等于关掉同向聚集规则
        single_signal.min_signal_count = 1;
        assert!(validate_aggregator_config(&single_signal).is_err());

        let mut negative_cooldown = base.clone();
        negative_cooldown.cooldown_secs = -1;
        assert!(validate_aggregator_config(&negative_cooldown).is_err());

        // 合法边界：cooldown 0 = 「不去重」、min_strength 1.0 = 「只报最强」
        let mut legal_edges = base.clone();
        legal_edges.cooldown_secs = 0;
        legal_edges.min_strength = 1.0;
        assert!(validate_aggregator_config(&legal_edges).is_ok());

        // NaN：`RangeInclusive::contains` 对 NaN 恒 false ⇒ 必须拒（强度 NaN 等于关掉闸门）
        let mut nan_strength = base;
        nan_strength.min_strength = f64::NAN;
        assert!(validate_aggregator_config(&nan_strength).is_err());
    }

    #[test]
    fn t0_config_rejects_non_positive_and_nan_but_keeps_none_thresholds() {
        assert!(validate_t0_config(&sample_config()).is_ok());
        // `None` 是合法的「不按这个条件触发」
        let mut none_thresholds = sample_config();
        none_thresholds.change_pct_threshold = None;
        none_thresholds.turnover_rate_threshold = None;
        assert!(validate_t0_config(&none_thresholds).is_ok());

        let mut zero_interval = sample_config();
        zero_interval.min_interval_minutes = 0;
        assert!(validate_t0_config(&zero_interval).is_err());

        let mut negative_threshold = sample_config();
        negative_threshold.change_pct_threshold = Some(-1.0);
        assert!(validate_t0_config(&negative_threshold).is_err());

        // NaN 过不了 `v <= 0.0`，必须靠显式 is_nan 判据拦住
        let mut nan_threshold = sample_config();
        nan_threshold.turnover_rate_threshold = Some(f64::NAN);
        assert!(validate_t0_config(&nan_threshold).is_err());
    }

    // ── status 的两格 ───────────────────────────────────────────────────

    #[test]
    fn unassembled_status_keeps_counters_zero_and_flags_them_meaningless() {
        // `assembled = false` 时那些 0 不是「没有失败」而是「没人在取数」⇒ 由 assembled 说话
        let view = status_view(false, "closed", false, 0, 0, &PollRoundStats::default());
        assert!(!view.assembled);
        assert_eq!(view.session, "closed");
        assert_eq!(view.poll_interval_secs, 0);
        assert_eq!(view.round_codes_total, 0);
        assert_eq!(view.round_finished_at_ms, 0);
    }

    #[tokio::test]
    async fn status_reports_assembled_engine_with_its_own_readings() {
        let m = monitor();
        m.add_config(MonitorConfig {
            alert_id: "a-1".to_string(),
            stock_code: "600519".to_string(),
            stock_name: "贵州茅台".to_string(),
            stop_loss: Some(1_400.0),
            take_profit: None,
            resistance_break: None,
            support_break: None,
            change_pct_alert: None,
            turnover_rate_alert: None,
            enabled: true,
        })
        .await;
        let provider = HostStockMonitor::new(m, aggregator(), Arc::new(db().await));
        let view = provider.status().await.expect("已装配的引擎应返回正常读数");
        assert!(view.assembled, "本 provider 只在装配成功后注册 ⇒ assembled 恒 true");
        assert_eq!(view.configs_active, 1);
        // 未跑过任何一轮 ⇒ roundFinishedAtMs 仍是 0，不能被读成「本轮 0/0 失败」
        assert_eq!(view.round_finished_at_ms, 0);
    }

    // ── remove_alert：查无即双 false ────────────────────────────────────

    #[tokio::test]
    async fn remove_alert_on_missing_row_is_ok_with_two_falses() {
        let provider = HostStockMonitor::new(monitor(), aggregator(), Arc::new(db().await));
        let outcome = provider
            .remove_alert("00000000-0000-4000-8000-000000000000")
            .await
            .expect("行不存在是正常结果，不是错误");
        assert!(!outcome.db_deleted);
        assert!(!outcome.memory_removed);
    }

    /// 复合 op 的两步真的都发生：有行 **且** 引擎在盯；删完两边都落。
    #[tokio::test]
    async fn upsert_then_remove_round_trips_db_and_engine_memory() {
        let m = monitor();
        let provider = HostStockMonitor::new(m.clone(), aggregator(), Arc::new(db().await));

        let outcome =
            provider.upsert_alert(request("stop_loss")).await.expect("标准类型应落库并接线");
        assert!(outcome.wired, "止损单应进引擎内存");
        assert_eq!(outcome.alert.alert_type.as_deref(), Some(alert_types::STOP_LOSS));
        assert_eq!(outcome.alert.is_triggered, 0, "新行是未触发态（integer 列，不是 bool）");
        let id = outcome.alert.id.clone();

        // 引擎侧按 alert_id 覆盖 ⇒ 列表里就是这一条，且排序稳定
        let configs = provider.list_configs().await.expect("列表读的是内存引擎");
        assert_eq!(configs.len(), 1);
        assert_eq!(configs[0].alert_id, id);

        let removed = provider.remove_alert(&id).await.expect("删除应成功");
        assert!(removed.db_deleted, "删掉了那一行");
        assert!(removed.memory_removed, "同步摘掉了引擎内存里的配置");
        assert!(provider.list_configs().await.expect("再读列表").is_empty(), "内存不再盯它");
        // 再删一次 ⇒ 回到「查无即双 false」
        let again = provider.remove_alert(&id).await.expect("重复删除不是错误");
        assert_eq!(again, RemoveAlertOutcome { db_deleted: false, memory_removed: false });
    }

    /// 同股同类型的第二次 upsert 复用未触发行（`id` 不变 ⇒ 内存按 alert_id 覆盖）。
    #[tokio::test]
    async fn second_upsert_of_same_type_reuses_the_row_instead_of_duplicating() {
        let provider = HostStockMonitor::new(monitor(), aggregator(), Arc::new(db().await));
        let first = provider.upsert_alert(request("take_profit")).await.expect("首次建单");
        let mut second_request = request("take_profit");
        second_request.threshold = 180.0;
        let second = provider.upsert_alert(second_request).await.expect("第二次建单");
        assert!(second.reused, "同股同类型的未触发行必须复用");
        assert_eq!(second.alert.id, first.alert.id, "复用时 id 不变");
        assert_eq!(second.alert.threshold, Some(180.0));
    }

    /// 非有限阈值（NaN / INF）拒收：落库后每轮求值都没有意义。
    #[tokio::test]
    async fn non_finite_threshold_is_rejected() {
        let provider = HostStockMonitor::new(monitor(), aggregator(), Arc::new(db().await));
        for bad in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            let mut req = request("stop_loss");
            req.threshold = bad;
            assert!(provider.upsert_alert(req).await.is_err(), "非有限阈值 {bad:?} 必须拒收");
        }
    }

    // ── 有序性 ──────────────────────────────────────────────────────────

    #[tokio::test]
    async fn aggregator_buffer_is_sorted_by_stock_code() {
        let agg = aggregator();
        // 逆序喂入，验证 provider 侧确实 sort 过（引擎缓冲是 HashMap）
        for (i, code) in ["600519", "000001", "300750"].iter().enumerate() {
            agg.feed(StockSignal {
                stock_code: code.to_string(),
                stock_name: format!("股票{code}"),
                signal_type: SignalType::DeathCross,
                strength: 0.6,
                source: "test".to_string(),
                timestamp: 1_700_000_000 + i as i64,
                current_price: Some(10.0),
                change_pct: Some(-2.0),
            })
            .await;
        }
        let provider = HostStockMonitor::new(monitor(), agg, Arc::new(db().await));
        let buffer = provider.aggregator_buffer().await.expect("缓冲读的是聚合器");
        let codes: Vec<&str> = buffer.iter().map(|s| s.stock_code.as_str()).collect();
        assert_eq!(codes, vec!["000001", "300750", "600519"], "缓冲必须按 stock_code 升序");
    }

    #[tokio::test]
    async fn list_configs_is_sorted_by_alert_id() {
        let m = monitor();
        for id in ["c", "a", "b"] {
            m.add_config(MonitorConfig {
                alert_id: id.to_string(),
                stock_code: "600519".to_string(),
                stock_name: "贵州茅台".to_string(),
                stop_loss: Some(1.0),
                take_profit: None,
                resistance_break: None,
                support_break: None,
                change_pct_alert: None,
                turnover_rate_alert: None,
                enabled: true,
            })
            .await;
        }
        let provider = HostStockMonitor::new(m, aggregator(), Arc::new(db().await));
        let ids: Vec<String> = provider
            .list_configs()
            .await
            .expect("列表读的是内存引擎")
            .into_iter()
            .map(|c| c.alert_id)
            .collect();
        assert_eq!(ids, vec!["a", "b", "c"], "配置列表必须按 alert_id 升序");
    }

    // ── T+0 / 聚合器配置的读写闭环 ──────────────────────────────────────

    #[tokio::test]
    async fn t0_config_roundtrip_and_illegal_write_leaves_the_old_value() {
        let m = monitor();
        let provider = HostStockMonitor::new(m.clone(), aggregator(), Arc::new(db().await));
        provider.set_t0_config(sample_config()).await.expect("合法写入应成功");
        assert_eq!(m.t0_config().await.min_interval_minutes, 30);

        let mut illegal = sample_config();
        illegal.min_interval_minutes = 0;
        assert!(provider.set_t0_config(illegal).await.is_err());
        // 拒收后引擎里仍是上一次那份（不是「先写后校验」）
        assert_eq!(provider.t0_config().await.expect("读回 T+0 配置").min_interval_minutes, 30);
    }

    #[tokio::test]
    async fn aggregator_config_roundtrip_rejects_out_of_range() {
        let agg = aggregator();
        let provider = HostStockMonitor::new(monitor(), agg.clone(), Arc::new(db().await));
        let tuned = AggregatorConfig { window_secs: 60, min_signal_count: 4, ..Default::default() };
        provider.set_aggregator_config(tuned.clone()).await.expect("区间内应写入成功");
        assert_eq!(agg.config().await.min_signal_count, 4);

        let illegal = AggregatorConfig { window_secs: 3601, ..tuned };
        assert!(provider.set_aggregator_config(illegal).await.is_err());
        assert_eq!(agg.config().await.window_secs, 60, "越界写入不得改动现值");
    }

    /// 测试库：与生产同一 `initialize_schema` 的临时 SQLite 文件库。
    async fn db() -> axagent_dao::DatabaseConnection {
        axagent_dao::db::create_test_pool().await.expect("测试库应可创建").conn
    }
}
