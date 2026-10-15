use axagent_harness::IpcEventName;
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use tokio::sync::RwLock;
use tokio::time::{interval, interval_at, Duration};

use axagent_harness::market_data::{MarketDataProvider, StockQuote};

/// 面板键：监控轮询间隔（秒）。权威在 `seed_variables.rs` / 设置面板，落点在本模块。
const PANEL_POLL_INTERVAL_KEY: &str = "monitor_poll_interval_secs";
/// 面板键：同一告警条目的冷却窗口（秒）。
const PANEL_ALERT_COOLDOWN_KEY: &str = "monitor_alert_cooldown_secs";
/// 轮询间隔可采纳区间：下界 1 秒（0 会让 `tokio::time::interval` panic），上界 1 小时。
const POLL_INTERVAL_RANGE: (u64, u64) = (1, 3600);

/// 前端事件推送桥接 trait
///
/// 由 wiring 层（如 Tauri AppHandle 包装）注入具体实现，
/// 使 stock-analysis crate 不直接依赖 Tauri 框架。
/// 未注入时（None）告警仅走内部 broadcast channel。
pub trait MonitorEventEmitter: Send + Sync {
    fn emit(&self, event: &str, payload: serde_json::Value);
}

/// T+0 自动重跑配置 (P2-6)
///
/// 当 RealtimeMonitor 检测到异常行情时,除了发告警,还会触发 T+0 工作流重跑
/// (即在交易时段内立即用最新行情重新跑一次 stock analysis workflow)。
/// - `enabled`: 全局开关
/// - `change_pct_threshold`: 涨跌幅超过 N% 触发 (None 表示不按此条件触发)
/// - `turnover_rate_threshold`: 换手率超过 N% 触发 (None 表示不按此条件触发)
/// - `min_interval_minutes`: 同一只股票两次 T+0 触发的最小间隔 (默认 30 分钟),
///   避免短时间内连续触发造成工作流风暴
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TZeroConfig {
    pub enabled: bool,
    pub change_pct_threshold: Option<f64>,
    pub turnover_rate_threshold: Option<f64>,
    pub min_interval_minutes: i64,
}

impl Default for TZeroConfig {
    fn default() -> Self {
        Self {
            enabled: false, // 默认关闭 — 用户需在设置里显式开启
            change_pct_threshold: Some(3.0),
            turnover_rate_threshold: Some(8.0),
            min_interval_minutes: 30,
        }
    }
}

/// T+0 触发回调: 接收 stock_code,异步启动工作流重跑
pub type TZeroCallback = Arc<
    dyn Fn(String) -> Pin<Box<dyn Future<Output = Result<String, String>> + Send>> + Send + Sync,
>;

/// 监控条件
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct MonitorConfig {
    /// `price_alerts.id` —— 一条告警一行配置。
    ///
    /// 此前配置按 `stock_code` 存 `HashMap`，同一只股票的第二条告警会静默顶掉第一条
    /// （现网 56 行 / 16 只股票 ⇒ 实际生效上限 16 条），且触发后无法只摘掉这一条。
    pub alert_id: String,
    pub stock_code: String,
    pub stock_name: String,
    /// 止损价（跌破告警）
    pub stop_loss: Option<f64>,
    /// 止盈价（突破告警）
    pub take_profit: Option<f64>,
    /// 突破压力位告警
    pub resistance_break: Option<f64>,
    /// 跌破支撑位告警
    pub support_break: Option<f64>,
    /// 涨跌幅超过N%告警
    pub change_pct_alert: Option<f64>,
    /// 成交量异常（换手率>N告警）
    pub turnover_rate_alert: Option<f64>,
    /// 是否启用
    pub enabled: bool,
}

/// 告警事件
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MonitorAlert {
    /// `price_alerts.id` —— 回写触发状态时精确定位这一行（同股同类型可能有重复行）
    pub alert_id: String,
    pub stock_code: String,
    pub stock_name: String,
    /// "stop_loss" | "take_profit" | "resistance" | "support" | "change" | "volume"
    pub alert_type: String,
    pub alert_message: String,
    pub current_price: f64,
    pub change_pct: f64,
    pub timestamp: String,
    /// 操作建议（如 "考虑减仓50%，现价低于止损"）
    pub suggested_action: Option<String>,
}

/// 封板状态（阶段 E-2，裁定 6 = 加标注不抑制告警）。
///
/// 四态而不是两态：`limit_up` / `limit_down` 由 vendor 给，sina / xueqiu 那两臂恒返回
/// `None`（见 `astock-data/src/sina.rs` 与 `xueqiu.rs` 的构造处）—— 那种情况必须读成
/// 「未知」，不能压成「未封板」，否则取数缺口会被当成一条正常结论卖给用户。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LimitState {
    /// 现价已达涨停价
    LimitUp,
    /// 现价已达跌停价
    LimitDown,
    /// 有涨跌停价数据且未达
    NotLimited,
    /// vendor 未提供涨跌停价 —— 未知，不等于未封板
    Unknown,
}

/// 从行情推封板状态。涨跌停两个方向都给了值又都没达，才算 `NotLimited`。
pub fn limit_state_of(quote: &StockQuote) -> LimitState {
    if quote.limit_up.is_none() && quote.limit_down.is_none() {
        return LimitState::Unknown;
    }
    if quote.limit_up.is_some_and(|up| quote.price >= up) {
        return LimitState::LimitUp;
    }
    if quote.limit_down.is_some_and(|down| quote.price <= down) {
        return LimitState::LimitDown;
    }
    LimitState::NotLimited
}

/// 一轮轮询的取数读数（裁定 12 ①：「监控实际未在取数」必须是能被面板消费的聚合面）。
///
/// 改造前只有逐只 `tracing::warn!`：整条链路长期全挂时，唯一发现方式是人工翻日志，
/// 而监控面板仍显示「监控运行中」。三条数各自只说一件事，不并排成可比的两个计数：
/// - [`Self::codes_total`] / [`Self::codes_failed`]：**本轮**应取数的股票数与其中失败数；
/// - [`Self::consecutive_all_failed_rounds`]：连续「整轮全失败」的轮数，用于判「通道挂了」；
/// - [`Self::finished_at_ms`]：本轮落数时刻。非交易时段整轮跳过时**不更新**，
///   所以它是「最后一次真取数的时刻」，不是「上一次心跳」。
#[derive(Debug, Clone, Default)]
pub struct PollRoundStats {
    pub codes_total: u32,
    pub codes_failed: u32,
    pub consecutive_all_failed_rounds: u32,
    pub finished_at_ms: i64,
}

/// 连续「整轮全失败」轮数的转移函数（纯函数，判据直接打在这一格上）。
///
/// 三档语义：
/// - `codes_total == 0` ⇒ 归零。没有启用中的告警就没有「失败的取数」，把旧连击留着
///   会让面板在空配置下说「连续 N 轮全失败」——那是把「没单」演成「通道坏了」。
/// - 全失败 ⇒ 上一值 +1。
/// - 任一成功 ⇒ 归零（半挂的轮次不算「未在取数」，由 `codes_failed/codes_total` 自己说话）。
pub fn next_all_failed_streak(prev: u32, codes_total: u32, codes_failed: u32) -> u32 {
    if codes_total == 0 {
        0
    } else if codes_failed >= codes_total {
        prev + 1
    } else {
        0
    }
}

/// 连击达到这个轮数时打一条 `error`（一轮 30s ⇒ 约 90s 确认整条取数链没在工作）。
///
/// 只在**恰好命中**这一档时打，不每轮重复：连击数本身仍在往上累加并暴露在
/// [`PollRoundStats`] 里，日志只负责「让运维面知道」，不承担计数。
pub const ALL_FAILED_STREAK_LOG_AT: u32 = 3;

/// 实时监控引擎
pub struct RealtimeMonitor {
    client: Arc<dyn MarketDataProvider>,
    /// 键 = `MonitorConfig::alert_id`（一条 `price_alerts` 行一个键）
    configs: RwLock<HashMap<String, MonitorConfig>>,
    alert_tx: tokio::sync::broadcast::Sender<MonitorAlert>,
    running: RwLock<bool>,
    event_emitter: RwLock<Option<Arc<dyn MonitorEventEmitter>>>,
    last_alerts: RwLock<HashMap<String, i64>>,
    /// 每条告警**上一次求值时是否越界**（键 = `alert_id:alert_type`），阶段 E-3 的边沿判据用。
    alert_states: RwLock<HashMap<String, bool>>,
    poll_interval_secs: RwLock<u64>,
    alert_cooldown_secs: RwLock<i64>,
    /// 最近一轮轮询的取数读数（裁定 12）—— 面板「本轮 x/y 只取数失败」的唯一来源。
    round_stats: RwLock<PollRoundStats>,
    // P2-6: T+0 自动重跑相关字段
    t0_config: RwLock<TZeroConfig>,
    t0_callback: RwLock<Option<TZeroCallback>>,
    /// 记录每只股票最近一次 T+0 触发的时间戳(秒),用于 cooldown 控制
    t0_last_trigger_ts: RwLock<HashMap<String, i64>>,
    /// P3-3: 跨股票信号聚合器（组合级告警）
    /// 不设置时为 None，单股告警不会聚合到组合层
    aggregator: RwLock<Option<Arc<crate::cross_stock_aggregator::CrossStockSignalAggregator>>>,
}

impl RealtimeMonitor {
    pub fn new(client: Arc<dyn MarketDataProvider>) -> Self {
        let (alert_tx, _) = tokio::sync::broadcast::channel(128);
        Self {
            client,
            configs: RwLock::new(HashMap::new()),
            alert_tx,
            running: RwLock::new(false),
            event_emitter: RwLock::new(None),
            last_alerts: RwLock::new(HashMap::new()),
            alert_states: RwLock::new(HashMap::new()),
            poll_interval_secs: RwLock::new(30),
            alert_cooldown_secs: RwLock::new(300),
            round_stats: RwLock::new(PollRoundStats::default()),
            t0_config: RwLock::new(TZeroConfig::default()),
            t0_callback: RwLock::new(None),
            t0_last_trigger_ts: RwLock::new(HashMap::new()),
            aggregator: RwLock::new(None),
        }
    }

    /// P2-6: 设置 T+0 自动重跑配置
    pub async fn set_t0_config(&self, config: TZeroConfig) {
        *self.t0_config.write().await = config;
    }

    /// P2-6: 设置 T+0 触发回调。
    /// 回调签名: 收到 stock_code,异步启动 workflow 重跑,返回 workflow_id 或错误信息。
    /// 不设置 callback 即关闭 T+0 触发(T+0 配置只是"如果 callback 存在就触发")。
    pub async fn set_t0_callback(&self, cb: TZeroCallback) {
        *self.t0_callback.write().await = Some(cb);
    }

    /// P2-6: 查询当前 T+0 配置
    pub async fn t0_config(&self) -> TZeroConfig {
        self.t0_config.read().await.clone()
    }

    /// 设置前端事件推送器以桥接告警到前端
    pub async fn set_event_emitter(&self, emitter: Arc<dyn MonitorEventEmitter>) {
        *self.event_emitter.write().await = Some(emitter);
    }

    pub fn subscribe(&self) -> tokio::sync::broadcast::Receiver<MonitorAlert> {
        self.alert_tx.subscribe()
    }

    /// P3-3: 注入跨股票信号聚合器，启用组合级告警。
    ///
    /// 不调用此方法时，RealtimeMonitor 仅产出单股告警；
    /// 调用后，每次 `check_alerts` 触发告警时会同步喂给聚合器，
    /// 当多只股票同方向触发信号时，聚合器产出 `PortfolioSignal` 并以
    /// `IpcEventName::PortfolioSignal` 推给前端（G-6）。
    ///
    /// 这里刻意**不**提供 `subscribe_portfolio_signals()`：进程内订阅口在本仓零消费者，
    /// 而真正在用的那条路是 `event_emitter.emit(..)`（与价格告警同一出口）。
    /// 留一个没人调的订阅口，正是本轮在阶段 D 退役的那类假旋钮。
    pub async fn set_aggregator(
        &self,
        agg: Arc<crate::cross_stock_aggregator::CrossStockSignalAggregator>,
    ) {
        *self.aggregator.write().await = Some(agg);
    }

    /// 添加/替换一条监控配置（键 = `price_alerts.id`，同 id 覆盖、不同 id 并存）
    pub async fn add_config(&self, config: MonitorConfig) {
        let mut configs = self.configs.write().await;
        configs.insert(config.alert_id.clone(), config);
    }

    /// 按告警条目摘除监控（删一条只退这一条），并回收它留下的冷却/边沿状态。
    ///
    /// 状态不清会随删除次数单调增长（`last_alerts` 与 `alert_states` 都没有别的清理路径）。
    pub async fn remove_config_by_id(&self, alert_id: &str) {
        let mut configs = self.configs.write().await;
        configs.remove(alert_id);
        let mut last = self.last_alerts.write().await;
        last.remove(alert_id);
        let prefix = format!("{alert_id}:");
        let mut states = self.alert_states.write().await;
        states.retain(|k, _| !k.starts_with(&prefix));
    }

    pub fn serialize_configs(configs: &HashMap<String, MonitorConfig>) -> String {
        serde_json::to_string(&configs.values().collect::<Vec<_>>()).unwrap_or_default()
    }

    pub fn deserialize_configs(json: &str) -> Vec<MonitorConfig> {
        serde_json::from_str::<Vec<MonitorConfig>>(json).unwrap_or_default()
    }

    /// 获取所有监控配置
    pub async fn list_configs(&self) -> Vec<MonitorConfig> {
        let configs = self.configs.read().await;
        configs.values().cloned().collect()
    }

    /// 测试用读数：当前留存的边沿状态键集合（`alert_id:alert_type`）。
    ///
    /// 有它才能断「摘除一条告警时状态被回收」—— 那两个 HashMap 否则只有涨没有落点。
    #[cfg(test)]
    pub(crate) async fn edge_state_keys(&self) -> Vec<String> {
        let mut keys: Vec<String> = self.alert_states.read().await.keys().cloned().collect();
        keys.sort();
        keys
    }

    /// 启动监控循环（轮询间隔**每轮重读** `poll_interval_secs`，默认 30 秒；改动下一轮生效）
    pub async fn start(&self) {
        {
            let mut running = self.running.write().await;
            if *running {
                return;
            }
            *running = true;
        }

        // D-2：间隔**每轮重读**。原先是 `let interval_secs = …; let mut ticker = interval(…)`，
        // 只在循环前读一次 ⇒ `set_poll_interval_secs` 与面板保存通路都成了摆设
        // （「面板可调、改了没反应」，落点门 P2 的 calls 判定也照样绿）。
        // 变化时用 `interval_at` 重建（不是 `interval`：后者的首个 tick 立即完成，
        // 会把「等一个周期」吃掉，等于换档那一轮连打两次）。
        // `.max(1)`：0 会让 tokio 的 interval panic —— 面板能把值改到 0，这条守卫必须在。
        let mut ticker_secs = (*self.poll_interval_secs.read().await).max(1);
        let mut ticker = interval(Duration::from_secs(ticker_secs));
        loop {
            ticker.tick().await;
            {
                let running = self.running.read().await;
                if !*running {
                    break;
                }
            }

            let want = (*self.poll_interval_secs.read().await).max(1);
            if want != ticker_secs {
                tracing::info!("[monitor] 轮询间隔 {}s → {}s（下一轮起按新值）", ticker_secs, want);
                ticker_secs = want;
                let period = Duration::from_secs(want);
                ticker = interval_at(tokio::time::Instant::now() + period, period);
            }

            let configs = {
                let c = self.configs.read().await;
                c.values().cloned().collect::<Vec<_>>()
            };

            // 按股票聚合：同一只股票的多条告警共用一次行情。配置粒度改成「按条」之后，
            // 请求数必须仍按股票数走 —— 现网 56 条告警分布在 16 只股票上，逐条取数会把
            // 每轮请求从 16 次撑到 56 次。
            let mut by_code: HashMap<String, Vec<MonitorConfig>> = HashMap::new();
            for config in configs {
                if !config.enabled {
                    continue;
                }
                by_code.entry(config.stock_code.clone()).or_default().push(config);
            }

            // 非交易时段整轮跳过（判据与行情监视器共用 `calendar::current_trading_session`）
            let session = axagent_astock_data::calendar::current_trading_session();
            if !session.is_open() {
                tracing::debug!("[monitor] 非交易时段（{session:?}），本轮不取数不求值");
                continue;
            }

            let codes_total = by_code.len() as u32;
            let mut codes_failed = 0_u32;
            for (code, group) in by_code {
                match self.client.get_quote(&code).await {
                    Ok(quote) => {
                        for config in &group {
                            self.check_alerts(config, &quote).await;
                        }
                    },
                    // E-1 的「不静默」与裁定 12 的「聚合」在这一格合流：逐只 warn 保留
                    // （要看得见是哪只挂的），同时计入本轮读数，轮末由 `record_poll_round`
                    // 汇成面板那句「本轮 x/y 只取数失败」。此前只有前者 ⇒ 整条链路长期全挂
                    // 时唯一发现方式是人工翻日志，而面板仍显示「监控运行中」。
                    Err(err) => {
                        codes_failed += 1;
                        tracing::warn!(
                            "[monitor] 取数失败，本轮该股票未求值: stock={code} err={err}"
                        );
                    },
                }
            }
            self.record_poll_round(codes_total, codes_failed).await;
        }
    }

    /// 停止监控循环
    pub async fn stop(&self) {
        let mut running = self.running.write().await;
        *running = false;
    }

    /// 以自定义参数启动监控循环。
    /// 设置轮询间隔和告警冷却时间后自动调用 `start()`。
    pub async fn start_with_config(&self, poll_interval_secs: u64, alert_cooldown_secs: i64) {
        *self.poll_interval_secs.write().await = poll_interval_secs;
        *self.alert_cooldown_secs.write().await = alert_cooldown_secs;
        self.start().await;
    }

    /// P1-1: 运行时热更新告警冷却时间（无需重启 monitor）。
    ///
    /// 用户在 settings 修改 `monitor_alert_cooldown_secs` 后立即生效，
    /// `check_alerts` 下一次循环就会读取新值做去重判定。
    pub async fn set_alert_cooldown_secs(&self, secs: i64) {
        *self.alert_cooldown_secs.write().await = secs;
    }

    /// P1-1: 查询当前告警冷却时间（供 Tauri 命令读取展示给前端）
    pub async fn alert_cooldown_secs(&self) -> i64 {
        *self.alert_cooldown_secs.read().await
    }

    /// P1-1: 运行时热更新轮询间隔（无需重启 monitor）。
    ///
    /// ⚠ 光有这个 setter 不够：`start()` 的 ticker 必须**每轮重读**本值才会生效
    /// （阶段 D-2 修的就是「读一次就定型」那个形态）。
    pub async fn set_poll_interval_secs(&self, secs: u64) {
        *self.poll_interval_secs.write().await = secs;
    }

    /// 保存设置后立即回灌两个运行期参数（阶段 D 的「保存即生效」通路）。
    ///
    /// 为什么不能只靠 `refresh_from_db`：这两条**不走快照读取** —— 轮询间隔在循环里读、
    /// 冷却在每轮求值时读，而面板保存写的是变量表行 ⇒ 不显式回灌就退回「重启才生效」。
    ///
    /// 非法值**拒绝覆盖**并 warn（同 `PositionLimits::from_panel_vars` 的口径）：
    /// 间隔 0 会让 `tokio::time::interval` panic（整条监控链停摆），冷却为负等于关掉去重
    /// （每轮重复报同一只）。面板写错不该把现网参数改坏。
    pub async fn apply_panel_runtime_vars(&self) {
        let vars = axagent_harness::panel_variables::panel_variables();
        self.apply_runtime_vars(&vars).await;
    }

    /// [`apply_panel_runtime_vars`] 的纯函数版（判据在此；直接测快照版会踩进程内全局、并行互踩）。
    pub async fn apply_runtime_vars(&self, vars: &HashMap<String, serde_json::Value>) {
        match axagent_harness::panel_variables::integer_in(vars, PANEL_POLL_INTERVAL_KEY) {
            Some(v) if (POLL_INTERVAL_RANGE.0 as i64..=POLL_INTERVAL_RANGE.1 as i64).contains(&v) => {
                self.set_poll_interval_secs(v as u64).await;
            },
            Some(v) => tracing::warn!(
                "[monitor] 面板 {PANEL_POLL_INTERVAL_KEY} = {v} 不在 {}..={} 秒 ⇒ 拒绝覆盖，仍按现值轮询",
                POLL_INTERVAL_RANGE.0,
                POLL_INTERVAL_RANGE.1
            ),
            None => {},
        }
        match axagent_harness::panel_variables::integer_in(vars, PANEL_ALERT_COOLDOWN_KEY) {
            Some(v) if v >= 0 => self.set_alert_cooldown_secs(v).await,
            Some(v) => tracing::warn!(
                "[monitor] 面板 {PANEL_ALERT_COOLDOWN_KEY} = {v} 为负 ⇒ 拒绝覆盖（负冷却等于关掉去重）"
            ),
            None => {},
        }
    }

    /// P1-1: 查询当前轮询间隔
    pub async fn poll_interval_secs(&self) -> u64 {
        *self.poll_interval_secs.read().await
    }

    /// 最近一轮取数读数（`monitor_status` 命令的取数侧来源）
    pub async fn poll_round_stats(&self) -> PollRoundStats {
        self.round_stats.read().await.clone()
    }

    /// 收下一轮的取数读数：算连击、落盘、到档打一条 error，并返回新读数。
    ///
    /// 拆成独立方法而不是写在循环体里，是为了让判据能直接打在这一格上 ——
    /// `start()` 是个不返回的 loop，没有这个方法就只能靠肉眼读循环代码。
    pub async fn record_poll_round(&self, codes_total: u32, codes_failed: u32) -> PollRoundStats {
        let stats = {
            let mut s = self.round_stats.write().await;
            s.consecutive_all_failed_rounds =
                next_all_failed_streak(s.consecutive_all_failed_rounds, codes_total, codes_failed);
            s.codes_total = codes_total;
            s.codes_failed = codes_failed;
            s.finished_at_ms = chrono::Utc::now().timestamp_millis();
            s.clone()
        };
        if stats.consecutive_all_failed_rounds == ALL_FAILED_STREAK_LOG_AT {
            tracing::error!(
                "[monitor] 连续 {} 轮 {} 只全部取数失败 ⇒ 监控实际未在产出告警（此后连击继续累加，本条日志只打一次）",
                stats.consecutive_all_failed_rounds,
                stats.codes_total
            );
        }
        stats
    }

    async fn check_alerts(&self, config: &MonitorConfig, quote: &StockQuote) {
        let mut alerts = Vec::new();
        let now = chrono::Utc::now().format("%Y-%m-%d %H:%M:%S").to_string();

        // 止损检查
        if let Some(stop) = config.stop_loss {
            if quote.price <= stop {
                let suggested =
                    Some(format!("建议: 考虑减仓50%，现价{:.2}低于止损{:.2}", quote.price, stop));
                alerts.push(MonitorAlert {
                    alert_id: config.alert_id.clone(),
                    stock_code: config.stock_code.clone(),
                    stock_name: config.stock_name.clone(),
                    alert_type: "stop_loss".into(),
                    alert_message: format!(
                        "跌破止损价 {}: 现价 {:.2} <= 止损 {:.2}",
                        config.stock_name, quote.price, stop
                    ),
                    current_price: quote.price,
                    change_pct: quote.change_pct,
                    timestamp: now.clone(),
                    suggested_action: suggested,
                });
            }
        }

        // 止盈检查
        if let Some(tp) = config.take_profit {
            if quote.price >= tp {
                let suggested =
                    Some(format!("建议: 考虑卖出50%锁利，现价{:.2}≥止盈{:.2}", quote.price, tp));
                alerts.push(MonitorAlert {
                    alert_id: config.alert_id.clone(),
                    stock_code: config.stock_code.clone(),
                    stock_name: config.stock_name.clone(),
                    alert_type: "take_profit".into(),
                    alert_message: format!(
                        "突破止盈价 {}: 现价 {:.2} >= 止盈 {:.2}",
                        config.stock_name, quote.price, tp
                    ),
                    current_price: quote.price,
                    change_pct: quote.change_pct,
                    timestamp: now.clone(),
                    suggested_action: suggested,
                });
            }
        }

        // 压力位突破
        if let Some(res) = config.resistance_break {
            if quote.price >= res {
                let suggested = Some(format!("建议: 压力位突破{:.2}，关注持续性和量能配合", res));
                alerts.push(MonitorAlert {
                    alert_id: config.alert_id.clone(),
                    stock_code: config.stock_code.clone(),
                    stock_name: config.stock_name.clone(),
                    alert_type: "resistance".into(),
                    alert_message: format!(
                        "突破压力位 {}: 现价 {:.2} >= 压力 {:.2}",
                        config.stock_name, quote.price, res
                    ),
                    current_price: quote.price,
                    change_pct: quote.change_pct,
                    timestamp: now.clone(),
                    suggested_action: suggested,
                });
            }
        }

        // 支撑位跌破
        if let Some(sup) = config.support_break {
            if quote.price <= sup {
                let suggested =
                    Some(format!("建议: 支撑位{:.2}跌破，考虑减仓或设立更严格止损", sup));
                alerts.push(MonitorAlert {
                    alert_id: config.alert_id.clone(),
                    stock_code: config.stock_code.clone(),
                    stock_name: config.stock_name.clone(),
                    alert_type: "support".into(),
                    alert_message: format!(
                        "跌破支撑位 {}: 现价 {:.2} <= 支撑 {:.2}",
                        config.stock_name, quote.price, sup
                    ),
                    current_price: quote.price,
                    change_pct: quote.change_pct,
                    timestamp: now.clone(),
                    suggested_action: suggested,
                });
            }
        }

        // 涨跌幅异常
        if let Some(pct) = config.change_pct_alert {
            if quote.change_pct.abs() >= pct {
                let dir = if quote.change_pct > 0.0 { "涨" } else { "跌" };
                let suggested = Some(format!(
                    "建议: 异常{}幅{:.2}%，关注消息面和资金流向",
                    dir, quote.change_pct
                ));
                alerts.push(MonitorAlert {
                    alert_id: config.alert_id.clone(),
                    stock_code: config.stock_code.clone(),
                    stock_name: config.stock_name.clone(),
                    alert_type: "change".into(),
                    alert_message: format!(
                        "异常{}幅 {}: {:.2}%",
                        dir, config.stock_name, quote.change_pct
                    ),
                    current_price: quote.price,
                    change_pct: quote.change_pct,
                    timestamp: now.clone(),
                    suggested_action: suggested,
                });
            }
        }

        // 换手率异常
        if let Some(ratio) = config.turnover_rate_alert {
            if quote.turnover_rate >= ratio {
                let suggested =
                    Some(format!("建议: 换手率异常{:.2}%，关注主力进出痕迹", quote.turnover_rate));
                alerts.push(MonitorAlert {
                    alert_id: config.alert_id.clone(),
                    stock_code: config.stock_code.clone(),
                    stock_name: config.stock_name.clone(),
                    alert_type: "volume".into(),
                    alert_message: format!(
                        "换手率异常 {}: {:.2}% > {:.2}%",
                        config.stock_name, quote.turnover_rate, ratio
                    ),
                    current_price: quote.price,
                    change_pct: quote.change_pct,
                    timestamp: now.clone(),
                    suggested_action: suggested,
                });
            }
        }

        // ── 阶段 E-2：封板状态只加标注，不吞告警（裁定 6）──
        // 涨停时的「突破压力位」跌停时的「跌破止损」是打不出去的价；把整条吞掉就等于把
        // 「这条我现在操作不了」伪装成「没触发」，所以只在文案上点名。
        // `Unknown` 也要点名：sina / xueqiu 两臂不返回涨跌停价，那不等于「未封板」。
        let limit_note = match limit_state_of(quote) {
            LimitState::LimitUp => "（涨停封板，该价暂时无法成交）",
            LimitState::LimitDown => "（跌停封板，该价暂时无法成交）",
            LimitState::Unknown => "（涨跌停价未知：该数据源不返回 limit_up/limit_down）",
            LimitState::NotLimited => "",
        };
        // ST 只在「确实是」时点名 —— vendor 侧 `is_st` 有恒 false 的臂，不能反过来说它不是 ST。
        let st_note = if quote.is_st { "（ST 股）" } else { "" };
        if !limit_note.is_empty() || !st_note.is_empty() {
            for alert in &mut alerts {
                alert.alert_message.push_str(limit_note);
                alert.alert_message.push_str(st_note);
            }
        }

        // ── 阶段 E-3：只在「未越界 → 越界」的边沿报（裁定 2）──
        // 旧形态是单边比较 + 按时间冷却 ⇒ 价格停在阈值那一侧时，每过一个冷却窗口就把同一件事
        // 再报一次（收盘后读到冻结价尤其明显）。
        // 「本轮哪几类越界」从已算出的 alerts 反推，不把六个比较各写第二遍（那会留第二套判据）。
        let in_range: Vec<String> =
            alerts.iter().map(|a| format!("{}:{}", a.alert_id, a.alert_type)).collect();
        let configured_kinds = [
            config.stop_loss.is_some().then_some("stop_loss"),
            config.take_profit.is_some().then_some("take_profit"),
            config.resistance_break.is_some().then_some("resistance"),
            config.support_break.is_some().then_some("support"),
            config.change_pct_alert.is_some().then_some("change"),
            config.turnover_rate_alert.is_some().then_some("volume"),
        ];
        let mut states = self.alert_states.write().await;
        let mut first_sight: Vec<String> = Vec::new();
        alerts.retain(|a| {
            let key = format!("{}:{}", a.alert_id, a.alert_type);
            match states.get(&key).copied() {
                // 上一次也越界 ⇒ 不是边沿 ⇒ 不再报
                Some(true) => false,
                Some(false) => true,
                // 该条目从没有过读数（刚建 / 刚重启）⇒ 按裁定 3 报一次并标注原因
                None => {
                    first_sight.push(key);
                    true
                },
            }
        });
        if !first_sight.is_empty() {
            for alert in &mut alerts {
                let key = format!("{}:{}", alert.alert_id, alert.alert_type);
                if first_sight.contains(&key) {
                    alert.alert_message.push_str("（进入监控时已越界）");
                }
            }
        }
        // 状态要为**该配置求过的每一类**更新（含这轮没越界的），否则「回落 → 再越界」取不到边沿。
        for kind in configured_kinds.into_iter().flatten() {
            let key = format!("{}:{}", config.alert_id, kind);
            let value = in_range.iter().any(|k| k == &key);
            states.insert(key, value);
        }

        let now_ts = chrono::Utc::now().timestamp();
        let cooldown_secs = *self.alert_cooldown_secs.read().await;
        let mut last = self.last_alerts.write().await;
        alerts.retain(|a| {
            // 冷却按告警条目计（此前是 `stock_code:alert_type`，同一股同类型的重复行会互相顶掉冷却槽）
            let key = a.alert_id.clone();
            if let Some(&last_ts) = last.get(&key) {
                if now_ts - last_ts < cooldown_secs {
                    return false;
                }
            }
            last.insert(key, now_ts);
            true
        });

        // 发送告警 — 内部 broadcast channel + 前端事件桥接
        let emitter = self.event_emitter.read().await.clone();
        for alert in &alerts {
            let _ = self.alert_tx.send(alert.clone());
            if let Some(ref e) = emitter {
                // payload 直接由 `MonitorAlert` 序列化得到（其 `rename_all = "camelCase"`）——
                // 不再手写 `json!` 摊平字段：那份手抄副本与前端消费端的漂移正是这一层造出来的，
                // 且后续给 `MonitorAlert` 加字段时，手抄副本会静默少一键。
                match serde_json::to_value(alert) {
                    Ok(payload) => e.emit(IpcEventName::PriceAlertTriggered.as_str(), payload),
                    Err(err) => tracing::warn!(
                        "[monitor] 告警序列化失败，该条不外推（已进内部 broadcast）: {err}"
                    ),
                }
            }
        }

        // P2-6: T+0 自动重跑
        // 在告警发送完毕后,根据本次触发的 alert 集合判断是否需要重跑工作流。
        // - 必须 enabled=true
        // - 必须有 alert 触发 (没触发就不重跑,避免每分钟空转)
        // - 必须通过 cooldown 检查 (同一只股票两次触发间隔 ≥ min_interval_minutes)
        if !alerts.is_empty() {
            self.maybe_trigger_t0(&alerts, &config.stock_code, quote).await;
        }

        // P3-3: 将本次告警喂给跨股票信号聚合器
        // 聚合器在多只股票同方向触发时生成 PortfolioSignal（组合级告警）
        if !alerts.is_empty() {
            self.feed_aggregator(&alerts, config, quote).await;
        }
    }

    /// P3-3: 将单股告警转换为 StockSignal 喂给聚合器
    async fn feed_aggregator(
        &self,
        alerts: &[MonitorAlert],
        config: &MonitorConfig,
        quote: &StockQuote,
    ) {
        use crate::cross_stock_aggregator::{SignalType, StockSignal};

        let agg = self.aggregator.read().await.clone();
        let Some(agg) = agg else {
            return;
        };

        let now_ts = chrono::Utc::now().timestamp();
        // 取本次最严重的告警作为该股票的信号（按 alert_type 优先级）
        // 优先级：stop_loss > support_break > take_profit > resistance_break > change > volume
        let signal_type = alerts
            .iter()
            .map(|a| match a.alert_type.as_str() {
                "stop_loss" => (SignalType::StopLossHit, 6),
                "support" => (SignalType::SupportBreak, 5),
                "take_profit" => (SignalType::TakeProfitHit, 4),
                "resistance" => (SignalType::ResistanceBreak, 3),
                "change" => (SignalType::ChangeSpike, 2),
                "volume" => (SignalType::VolumeSpike, 1),
                _ => (SignalType::ChangeSpike, 0),
            })
            .max_by_key(|(_, p)| *p)
            .map(|(t, _)| t)
            .unwrap_or(SignalType::ChangeSpike);

        // 信号强度估算：基于涨跌幅绝对值（+5% → 强度 0.5，+10% → 强度 1.0）
        let strength = (quote.change_pct.abs() / 10.0).clamp(0.1, 1.0);

        let signal = StockSignal {
            stock_code: config.stock_code.clone(),
            stock_name: config.stock_name.clone(),
            signal_type,
            strength,
            source: "realtime_monitor".to_string(),
            timestamp: now_ts,
            current_price: Some(quote.price),
            change_pct: Some(quote.change_pct),
        };

        // 喂给聚合器；若触发组合级信号，通过 broadcast 自动推送给订阅者
        if let Some(portfolio_signal) = agg.feed(signal).await {
            tracing::info!(
                "[portfolio_alert] 组合级信号触发: dir={:?} stocks={} strength={:.2} action={}",
                portfolio_signal.direction,
                portfolio_signal.stocks.len(),
                portfolio_signal.strength,
                portfolio_signal.suggested_action
            );
            // 通过 event_emitter 推送给前端（事件名只认权威枚举，见 R14）
            let emitter = self.event_emitter.read().await.clone();
            if let Some(ref e) = emitter {
                e.emit(
                    IpcEventName::PortfolioSignal.as_str(),
                    serde_json::to_value(&portfolio_signal).unwrap_or(serde_json::Value::Null),
                );
            }
        }
    }

    /// P2-6: 检查并触发 T+0 重跑
    ///
    /// 设计原则: monitor 不直接调用 workflow 重跑(避免循环依赖),而是 emit
    /// `stock-monitor-t0-rerun-requested` 事件到前端,由前端 listener 调
    /// `run_stock_workflow` 命令触发重跑。这样:
    /// 1) 后端 monitor 保持纯监控职责,不依赖 workflow engine;
    /// 2) 前端可在 T+0 重跑前做去重、节流、UI 提示;
    /// 3) 如果用户没开前端 UI,事件会丢失但不阻塞监控。
    async fn maybe_trigger_t0(
        &self,
        alerts: &[MonitorAlert],
        stock_code: &str,
        quote: &StockQuote,
    ) {
        let t0_cfg = self.t0_config.read().await.clone();
        if !t0_cfg.enabled {
            return;
        }

        // 1) 判断本次告警是否命中 T+0 触发条件
        let mut hit_change = false;
        let mut hit_volume = false;
        for a in alerts {
            if a.alert_type == "change" {
                if let Some(th) = t0_cfg.change_pct_threshold {
                    if quote.change_pct.abs() >= th {
                        hit_change = true;
                    }
                }
            } else if a.alert_type == "volume" {
                if let Some(th) = t0_cfg.turnover_rate_threshold {
                    if quote.turnover_rate >= th {
                        hit_volume = true;
                    }
                }
            }
        }
        if !hit_change && !hit_volume {
            return; // 告警触发但未命中 T+0 阈值
        }

        // 2) Cooldown: 同一只股票两次 T+0 之间间隔 ≥ min_interval_minutes
        //    读+写在同一个 write lock 内完成，避免 TOCTOU 竞态
        let now_ts = chrono::Utc::now().timestamp();
        let cooldown_secs = t0_cfg.min_interval_minutes * 60;
        {
            let mut last_map = self.t0_last_trigger_ts.write().await;
            if let Some(&last_ts) = last_map.get(stock_code) {
                if now_ts - last_ts < cooldown_secs {
                    tracing::debug!(
                        "[t0] skip {}: cooldown 未到 ({}s < {}s)",
                        stock_code,
                        now_ts - last_ts,
                        cooldown_secs
                    );
                    return;
                }
            }
            last_map.insert(stock_code.to_string(), now_ts);
        }
        let reason = if hit_change && hit_volume {
            "change+volume"
        } else if hit_change {
            "change"
        } else {
            "volume"
        };
        tracing::info!(
            "[t0] 触发 T+0 重跑: stock={} reason={} change_pct={:.2} turnover={:.2}",
            stock_code,
            reason,
            quote.change_pct,
            quote.turnover_rate
        );

        // 4) 触发 T+0 重跑
        // P1-1: 优先调用 t0_callback（后端直接重跑，不依赖前端在线），
        //        同时 emit 前端事件做 UI 提示（toast / 状态刷新）。
        // - 若 callback 存在并触发：事件 payload 携带 backendTriggered=true，
        //   前端据此跳过 startAnalysis（避免后端+前端重复重跑），仅做 toast 提示。
        // - 若 callback 不存在（未注入）：backendTriggered=false，前端走原逻辑调 startAnalysis。
        let mut backend_triggered = false;
        let callback = self.t0_callback.read().await.clone();
        if let Some(ref cb) = callback {
            // 后端重跑失败不阻塞监控循环，仅记录日志
            match cb(stock_code.to_string()).await {
                Ok(workflow_id) => {
                    backend_triggered = true;
                    tracing::info!(
                        "[t0] 后端重跑已触发: stock={} workflow_id={}",
                        stock_code,
                        workflow_id
                    );
                },
                Err(e) => {
                    tracing::warn!("[t0] 后端重跑失败: stock={} err={}", stock_code, e);
                },
            }
        }

        // 同时 emit 前端事件 —— 用于 UI toast 提示和状态刷新
        let emitter = self.event_emitter.read().await.clone();
        if let Some(ref e) = emitter {
            e.emit(
                IpcEventName::StockMonitorT0RerunRequested.as_str(),
                serde_json::json!({
                    "stockCode": stock_code,
                    "reason": reason,
                    "currentPrice": quote.price,
                    "changePct": quote.change_pct,
                    "turnoverRate": quote.turnover_rate,
                    "timestamp": now_ts,
                    "backendTriggered": backend_triggered,
                }),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    //! 阶段 B 的形态锁：监控配置必须按**告警条目**存，冷却也必须按条目计。
    //!
    //! 回归对象（2026-10-14 缺陷评估）：`configs` 以 `stock_code` 为键，而每条告警只填六个
    //! 阈值字段中的一个 ⇒ 同一只股票的第二条静默顶掉第一条（现网 56 行分布在 16 只股票上，
    //! 实际生效上限就是 16）；冷却键 `code:type` 又让同股同类型的重复行互相抢冷却槽。

    use super::*;
    use axagent_harness::core_error::AxAgentError;
    use axagent_harness::market_data::{AdjType, KLine, StockSearchResult};

    /// 只回固定行情的取数替身 —— 本模块锁的是配置粒度与冷却，不是取数。
    struct StubProvider;

    #[async_trait::async_trait]
    impl MarketDataProvider for StubProvider {
        async fn get_quote(
            &self,
            stock_code: &str,
        ) -> axagent_harness::core_error::Result<StockQuote> {
            Ok(quote_of(stock_code, 12.34))
        }

        async fn get_klines(
            &self,
            _stock_code: &str,
            _period: &str,
            _limit: u32,
            _adj_type: Option<AdjType>,
        ) -> axagent_harness::core_error::Result<Vec<KLine>> {
            Err(AxAgentError::Provider("mock: 监控循环不该取 K 线".into()))
        }

        async fn search_stock(
            &self,
            _keyword: &str,
        ) -> axagent_harness::core_error::Result<Vec<StockSearchResult>> {
            Err(AxAgentError::Provider("mock: 监控循环不该搜股".into()))
        }
    }

    fn quote_of(code: &str, price: f64) -> StockQuote {
        StockQuote {
            code: code.to_string(),
            name: "测试股".to_string(),
            price,
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
            timestamp: "2026-10-15 10:00:00".to_string(),
        }
    }

    /// 记录发射器收到的告警 payload。
    // SAFETY: std::sync::Mutex 不跨 await —— `emit` 是同步 trait 方法，guard 是语句级临时量，
    // 在任何 await 点之前已 drop。依据 `clippy.toml` 的合法例外（形态同本模块
    // `portfolio_monitor.rs` 的 MockProvider）。
    #[allow(clippy::disallowed_types)]
    #[derive(Default)]
    struct Recorder {
        seen: std::sync::Mutex<Vec<serde_json::Value>>,
    }

    // SAFETY: 同上 —— `alerts()` 是同步函数，临界区内无 await 点。
    #[allow(clippy::disallowed_types)]
    impl Recorder {
        fn alerts(&self) -> Vec<(String, String)> {
            self.seen
                .lock()
                .unwrap()
                .iter()
                .map(|p| {
                    (
                        p.get("alertId").and_then(|v| v.as_str()).unwrap_or_default().to_string(),
                        p.get("alertType").and_then(|v| v.as_str()).unwrap_or_default().to_string(),
                    )
                })
                .collect()
        }

        /// 收到的告警文案（E-2 / E-3 断的是文案里点了什么名）。
        fn messages(&self) -> Vec<String> {
            self.seen
                .lock()
                .unwrap()
                .iter()
                .map(|p| {
                    p.get("alertMessage").and_then(|v| v.as_str()).unwrap_or_default().to_string()
                })
                .collect()
        }
    }

    impl MonitorEventEmitter for Recorder {
        fn emit(&self, _event: &str, payload: serde_json::Value) {
            self.seen.lock().unwrap().push(payload);
        }
    }

    fn stop_loss_config(alert_id: &str, code: &str, stop: f64) -> MonitorConfig {
        MonitorConfig {
            alert_id: alert_id.to_string(),
            stock_code: code.to_string(),
            stock_name: "测试股".to_string(),
            stop_loss: Some(stop),
            take_profit: None,
            resistance_break: None,
            support_break: None,
            change_pct_alert: None,
            turnover_rate_alert: None,
            enabled: true,
        }
    }

    async fn monitor_with_recorder() -> (RealtimeMonitor, Arc<Recorder>) {
        let recorder = Arc::new(Recorder::default());
        let monitor = RealtimeMonitor::new(Arc::new(StubProvider));
        monitor.set_event_emitter(recorder.clone()).await;
        (monitor, recorder)
    }

    /// 同一只股票的两条告警都必须留在监控里，并且各自报一次。
    #[tokio::test]
    async fn same_stock_two_alert_rows_both_survive_and_both_fire() {
        let (monitor, recorder) = monitor_with_recorder().await;
        monitor.add_config(stop_loss_config("alert-a", "601899", 13.0)).await;
        monitor.add_config(stop_loss_config("alert-b", "601899", 14.0)).await;

        assert_eq!(
            monitor.list_configs().await.len(),
            2,
            "同股两条告警不得互相顶掉（旧形态只剩 1 条）"
        );

        let quote = quote_of("601899", 12.34);
        for id in ["alert-a", "alert-b"] {
            let cfg = monitor
                .list_configs()
                .await
                .into_iter()
                .find(|c| c.alert_id == id)
                .expect("配置必须还在");
            monitor.check_alerts(&cfg, &quote).await;
        }

        let mut got = recorder.alerts();
        got.sort();
        assert_eq!(
            got,
            vec![
                ("alert-a".to_string(), "stop_loss".to_string()),
                ("alert-b".to_string(), "stop_loss".to_string()),
            ],
            "两条都要报，且 payload 带各自的 alertId（回写按它精确定位）"
        );
    }

    /// 同一条告警在冷却窗口内重复越界只报一次。
    #[tokio::test]
    async fn same_alert_row_is_cooled_within_window() {
        let (monitor, recorder) = monitor_with_recorder().await;
        let cfg = stop_loss_config("alert-a", "601899", 13.0);
        monitor.add_config(cfg.clone()).await;

        let quote = quote_of("601899", 12.34);
        monitor.check_alerts(&cfg, &quote).await;
        monitor.check_alerts(&cfg, &quote).await;

        assert_eq!(recorder.alerts().len(), 1, "冷却窗口内同一条只应报一次");
    }

    /// 同 id 再次注入是替换，不是叠加（重启恢复与手建走同一条路）。
    #[tokio::test]
    async fn add_config_with_same_id_replaces_instead_of_duplicating() {
        let (monitor, _recorder) = monitor_with_recorder().await;
        monitor.add_config(stop_loss_config("alert-a", "601899", 13.0)).await;
        monitor.add_config(stop_loss_config("alert-a", "601899", 11.0)).await;

        let configs = monitor.list_configs().await;
        assert_eq!(configs.len(), 1, "同 id 必须覆盖");
        assert_eq!(configs[0].stop_loss, Some(11.0), "覆盖必须真取到新值");
    }

    /// 封板状态必须四态齐全：vendor 不给涨跌停价时判「未知」，不得当成「未封板」。
    #[test]
    fn limit_state_has_four_distinct_outcomes() {
        let q = |price: f64, up: Option<f64>, down: Option<f64>| {
            let mut quote = quote_of("601899", price);
            quote.limit_up = up;
            quote.limit_down = down;
            quote
        };
        assert_eq!(limit_state_of(&q(13.5, Some(13.5), Some(12.5))), LimitState::LimitUp);
        assert_eq!(limit_state_of(&q(12.5, Some(13.5), Some(12.5))), LimitState::LimitDown);
        assert_eq!(limit_state_of(&q(13.0, Some(13.5), Some(12.5))), LimitState::NotLimited);
        assert_eq!(
            limit_state_of(&q(13.0, None, None)),
            LimitState::Unknown,
            "两列都缺 ⇒ 未知，不是未封板"
        );
        // 只给一边且未达：仍算「未封板」（有数据、结论成立）
        assert_eq!(limit_state_of(&q(13.0, Some(13.5), None)), LimitState::NotLimited);
    }

    /// 阶段 E-3 的形态锁：停在阈值那一侧不重复报，回落后再越界才算第二次边沿。
    #[tokio::test]
    async fn alert_fires_on_crossing_not_while_staying_beyond() {
        let (monitor, recorder) = monitor_with_recorder().await;
        // 冷却归零：本测试要数的是边沿，不是冷却窗口
        monitor.set_alert_cooldown_secs(0).await;
        let cfg = stop_loss_config("alert-a", "601899", 13.0);
        monitor.add_config(cfg.clone()).await;

        // ① 首次观察即越界 ⇒ 报（并按裁定 3 标注）
        monitor.check_alerts(&cfg, &quote_of("601899", 12.0)).await;
        assert_eq!(recorder.alerts().len(), 1, "首次越界应报");
        assert!(
            recorder.messages()[0].contains("进入监控时已越界"),
            "首次观察就处于越界侧，文案要说出原因：{}",
            recorder.messages()[0]
        );

        // ② 价格仍在一侧 ⇒ 不是边沿 ⇒ 不报
        monitor.check_alerts(&cfg, &quote_of("601899", 11.5)).await;
        assert_eq!(recorder.alerts().len(), 1, "停在阈值内侧不得每轮重报");

        // ③ 回到阈值之上 ⇒ 无告警，但要更新状态
        monitor.check_alerts(&cfg, &quote_of("601899", 13.8)).await;
        assert_eq!(recorder.alerts().len(), 1, "未越界不该报");

        // ④ 再次跌破 ⇒ 新边沿 ⇒ 第二次报
        monitor.check_alerts(&cfg, &quote_of("601899", 12.4)).await;
        assert_eq!(recorder.alerts().len(), 2, "回落后再越界必须能再报一次");
    }

    /// 阶段 E-2 的落地形态：封板/未知都在告警文案里点名（裁定 6 = 不抑制）。
    #[tokio::test]
    async fn limit_state_is_annotated_without_suppressing_the_alert() {
        let (monitor, recorder) = monitor_with_recorder().await;
        let cfg = stop_loss_config("alert-up", "601899", 13.0);
        monitor.add_config(cfg.clone()).await;

        let mut stuck_down = quote_of("601899", 12.0);
        stuck_down.limit_down = Some(12.0);
        stuck_down.is_st = true;
        monitor.check_alerts(&cfg, &stuck_down).await;

        let msgs = recorder.messages();
        assert_eq!(msgs.len(), 1, "封板不得把告警吞掉");
        assert!(msgs[0].contains("跌停封板"), "要点名跌停：{}", msgs[0]);
        assert!(msgs[0].contains("ST 股"), "ST 为真时要点名：{}", msgs[0]);

        // 未知也要点名（sina / xueqiu 那两臂的形态）
        let (monitor2, recorder2) = monitor_with_recorder().await;
        let cfg2 = stop_loss_config("alert-unknown", "601899", 13.0);
        monitor2.add_config(cfg2.clone()).await;
        let mut blind = quote_of("601899", 12.0);
        blind.limit_up = None;
        blind.limit_down = None;
        monitor2.check_alerts(&cfg2, &blind).await;
        let msgs2 = recorder2.messages();
        assert_eq!(msgs2.len(), 1);
        assert!(
            msgs2[0].contains("涨跌停价未知"),
            "取数缺口要说成未知，不能读成未封板：{}",
            msgs2[0]
        );
    }

    /// 删一条告警必须连它的边沿状态一起回收，否则重建同 id 会丢「首次观察」这层语义。
    #[tokio::test]
    async fn removing_an_alert_prunes_its_edge_state() {
        let (monitor, _recorder) = monitor_with_recorder().await;
        let cfg = stop_loss_config("alert-a", "601899", 13.0);
        monitor.add_config(cfg.clone()).await;
        monitor.check_alerts(&cfg, &quote_of("601899", 12.0)).await;
        assert!(
            monitor.edge_state_keys().await.iter().any(|k| k.starts_with("alert-a:")),
            "求过一次就该留下边沿状态"
        );

        monitor.remove_config_by_id("alert-a").await;
        assert!(
            monitor.edge_state_keys().await.iter().all(|k| !k.starts_with("alert-a:")),
            "摘除后不得留下按删除次数单调增长的状态"
        );
    }

    /// 连击转移函数的三档语义各钉一格（纯函数，负控就拆在这三格上）。
    #[test]
    fn all_failed_streak_has_three_distinct_buckets() {
        assert_eq!(next_all_failed_streak(0, 3, 3), 1, "整轮全失败从 0 起算 1");
        assert_eq!(next_all_failed_streak(4, 3, 3), 5, "继续全失败要累加");
        assert_eq!(next_all_failed_streak(7, 3, 2), 0, "任一成功即归零：半挂不该报「未在取数」");
        assert_eq!(next_all_failed_streak(7, 3, 0), 0, "全成功更要归零");
        // 空配置不得演成「通道全挂」—— 这是呈现层歧义的唯一来源
        assert_eq!(next_all_failed_streak(7, 0, 0), 0, "本轮没有启用中的告警 ⇒ 不得保留旧连击");
    }

    /// 轮末读数必须落在引擎上并能被 `poll_round_stats()` 读回 —— 面板那句
    /// 「本轮 x/y 只取数失败」只有这一个来源。`start()` 是个不返回的 loop，
    /// 判据打不进循环体，所以记数拆成了 `record_poll_round`。
    #[tokio::test]
    async fn record_poll_round_exposes_counters_and_resets_on_recovery() {
        let (monitor, _recorder) = monitor_with_recorder().await;

        monitor.record_poll_round(4, 4).await;
        let s = monitor.record_poll_round(4, 4).await;
        assert_eq!(s.codes_total, 4, "分母是本轮应取数的股票数，不是告警条数");
        assert_eq!(s.codes_failed, 4);
        assert_eq!(s.consecutive_all_failed_rounds, 2, "两轮整轮全失败要累加");
        assert!(s.finished_at_ms > 0, "落数时刻必须是真实时间戳");

        // 恢复一轮：连击归零，本轮失败数照实留着（两个计数域各说各话，不收敛成一个数）
        let after = monitor.record_poll_round(4, 1).await;
        assert_eq!(after.consecutive_all_failed_rounds, 0);
        assert_eq!(after.codes_failed, 1);

        let stored = monitor.poll_round_stats().await;
        assert_eq!(stored.codes_total, 4, "读回的必须是引擎里那一格，不是调用方的临时值");
        assert_eq!(stored.codes_failed, 1);
        assert_eq!(stored.consecutive_all_failed_rounds, 0);
    }

    /// 阶段 D：保存设置后回灌两个运行期参数 —— 合法值采纳、非法值**拒绝覆盖**。
    ///
    /// 走纯函数版（`apply_runtime_vars`）而不是快照版：快照是进程内全局，并行用例会互踩。
    /// 判据分四格各占一条：合法采纳 / 间隔 0 拒绝 / 间隔越上界拒绝 / 冷却为负拒绝，
    /// 外加「键缺席一律不动」（面板没播种时不得把默认值抹成别的）。
    #[tokio::test]
    async fn apply_runtime_vars_adopts_valid_and_rejects_invalid() {
        let monitor = RealtimeMonitor::new(Arc::new(StubProvider));
        let vars = |pairs: &[(&str, i64)]| {
            pairs
                .iter()
                .map(|(k, v)| (k.to_string(), serde_json::json!(*v)))
                .collect::<HashMap<_, _>>()
        };

        // ① 合法值采纳
        monitor
            .apply_runtime_vars(&vars(&[
                ("monitor_poll_interval_secs", 45),
                ("monitor_alert_cooldown_secs", 600),
            ]))
            .await;
        assert_eq!(monitor.poll_interval_secs().await, 45);
        assert_eq!(monitor.alert_cooldown_secs().await, 600);

        // ② 间隔 0 ⇒ 拒绝（0 会让 `tokio::time::interval` panic，整条监控链停摆）
        monitor.apply_runtime_vars(&vars(&[("monitor_poll_interval_secs", 0)])).await;
        assert_eq!(monitor.poll_interval_secs().await, 45, "0 秒必须被拒绝，不能带进循环");

        // ③ 间隔越上界（3601）⇒ 拒绝；边界值 3600 采纳
        monitor.apply_runtime_vars(&vars(&[("monitor_poll_interval_secs", 3601)])).await;
        assert_eq!(monitor.poll_interval_secs().await, 45);
        monitor.apply_runtime_vars(&vars(&[("monitor_poll_interval_secs", 3600)])).await;
        assert_eq!(monitor.poll_interval_secs().await, 3600, "上界本身可用");

        // ④ 冷却为负 ⇒ 拒绝（负冷却等于关掉去重）；0 合法（测试与「关冷却」都要用它）
        monitor.apply_runtime_vars(&vars(&[("monitor_alert_cooldown_secs", -5)])).await;
        assert_eq!(monitor.alert_cooldown_secs().await, 600);
        monitor.apply_runtime_vars(&vars(&[("monitor_alert_cooldown_secs", 0)])).await;
        assert_eq!(monitor.alert_cooldown_secs().await, 0);

        // ⑤ 键缺席 ⇒ 一律不动（快照空 = 面板还没播种，不是「把它设成 0」）
        monitor.apply_runtime_vars(&HashMap::new()).await;
        assert_eq!(monitor.poll_interval_secs().await, 3600);
        assert_eq!(monitor.alert_cooldown_secs().await, 0);
    }
}
