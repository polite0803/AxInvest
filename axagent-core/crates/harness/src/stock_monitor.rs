// SPDX-License-Identifier: AGPL-3.0-only

//! 股票实时监控接缝（`stock.monitor`，含跨股票聚合器面）— DTO + Trait。
//!
//! 宿主把 `analysis-engine` 的 `RealtimeMonitor` 与 `CrossStockSignalAggregator` 经本接缝
//! 暴露给插件侧；插件经 `call_seam("stock.monitor", op, args)` 读运行读数与告警配置、
//! 写告警 / T+0 开关 / 聚合器阈值。
//!
//! ## 为什么这些 DTO 住在 harness
//!
//! [`MonitorConfig`] / [`TZeroConfig`] / [`PollRoundStats`] / [`MonitorAlert`] 与聚合器的
//! [`AggregatorConfig`] / [`StockSignal`] / [`PortfolioSignal`] / [`PortfolioSnapshot`] /
//! [`SignalType`] / [`SignalDirection`] 原先定义在 `axagent-quant/crates/analysis-engine`。
//! 接缝要把它们当线上契约，而 harness 是 foundation 层、**不得依赖任何 axagent-\* crate**
//! （AGENTS 铁律 1）⇒ 定义**上收本模块**，原处改 `pub use axagent_harness::{…}`。
//! 另建同义镜像会立刻造出第二套真相（禁区 12），故禁止。
//!
//! ## 主权边界：七件事必须留宿主
//!
//! 轮询循环、交易时段闸门、按股票聚合请求数、边沿 + 冷却判据、触发回写与通知外推、
//! T+0 启动工作流、聚合喂入。⇒ [`crate::SignalDirection`] 相关的 `feed` / `clear` /
//! `set_snapshot_provider` **不发**给插件：`feed` 是组合信号的唯一触发口，插件喂入等于
//! 伪造「某只股票触发某类信号」，会把组合级告警变成可外部编造的出口。
//!
//! ## 不过线的成员（及理由）
//!
//! - `start` / `start_with_config`：循环不返回；`stop`：`running` 是全局单值，一句话让
//!   **所有**插件与前端同时失去告警 ⇒ 跨插件 DoS 面
//! - `set_event_emitter` / `set_t0_callback` / `set_aggregator` / `subscribe`：
//!   闭包 / trait object / `broadcast::Receiver`，跨不过帧协议
//! - `add_config` / `remove_config_by_id`：半成品面。引擎的 `add_config` **只写内存不写库**，
//!   只调它的插件会造出「`price_alerts` 无行、重启即丢」的告警（启动恢复只从表里读），
//!   且触发回写会找不到行 ⇒ 必须走 [`StockMonitorProvider::upsert_alert`] /
//!   [`StockMonitorProvider::remove_alert`] 这两个复合 op
//! - `record_poll_round`：循环内部计数，插件写它就是伪造读数
//! - `set_poll_interval_secs` / `set_alert_cooldown_secs`：绕过引擎里的 `1..=3600` clamp
//!   与负值守卫
//! - `apply_panel_runtime_vars` / `apply_runtime_vars`：读进程内面板快照，跨边界无意义
//!
//! ## `as_of`
//!
//! 本接缝所有 op 要么读内存 / DB，要么由宿主轮询循环自己去取数 ⇒ **一律不收 `as_of`**。
//!
//! ## 写面权限
//!
//! `upsert_alert` / `remove_alert` / `set_t0_config` / `set_aggregator_config` 要求插件已声明
//! `write` 权限（由 worker 分派层强制，拒绝码 `PLUGIN_PERMISSION_DENIED`）；本层只提供契约，
//! 不重复校验。
//!
//! ## 错误码归属（与既有四条接缝逐字同源）
//!
//! | 码 | 触发点 |
//! |---|---|
//! | `SEAM_CALL_INVALID_ARGS` | 必填缺失 / 类型不符 / `alert_type` 不在白名单 / 阈值越界 |
//! | `SEAM_PROVIDER_UNAVAILABLE` | 注册表取不到 provider（启动窗口内 / 提供方卸载后） |
//! | `PLUGIN_CALL_FAILED` | provider 返回 `Err` 或响应序列化失败 |
//!
//! ⚠ `status` 的「引擎未装配」**不是**错误：照 `commands/stock_analysis.rs` 的
//! `monitor_status`，返回 `assembled = false` 的正常响应（计数全 0，由 `assembled` 告诉
//! 消费方「这些 0 没有含义」）。

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::core_error::{AxAgentError, Result};

// ── 监控引擎 DTO（上收自 analysis-engine/src/monitor.rs）─────────────────

/// T+0 自动重跑配置（P2-6）。
///
/// 当 `RealtimeMonitor` 检测到异常行情时，除了发告警，还会触发 T+0 工作流重跑
/// （即在交易时段内立即用最新行情重新跑一次 stock analysis workflow）。
/// - `enabled`: 全局开关
/// - `change_pct_threshold`: 涨跌幅超过 N% 触发（None 表示不按此条件触发）
/// - `turnover_rate_threshold`: 换手率超过 N% 触发（None 表示不按此条件触发）
/// - `min_interval_minutes`: 同一只股票两次 T+0 触发的最小间隔（默认 30 分钟），
///   避免短时间内连续触发造成工作流风暴
///
/// ⚠ `#[serde(rename_all = "camelCase")]` 是**上收时补的既存缺陷修复**：前端按 camelCase
/// 读写（`src/stores/feature/stockAnalysisStore.ts` 的 `t0Config` / `fetchT0Config` /
/// `setT0Config`），而本结构体原先没有该注解 ⇒ `get_t0_config` 的 `changePctThreshold` /
/// `turnoverRateThreshold` 恒 `undefined`；`set_t0_config` 因 JSON 里没有
/// `min_interval_minutes` 这一键而**整条反序列化失败**（`enabled` 同名侥幸命中，两个
/// `Option` 字段被 serde 当缺省 `None`，必填 `i64` 缺席即 reject）。键集合与注解由
/// `scripts/check-monitor-event-authority.mjs` 的 R18 钉住。
///
/// 刻意**不加** `#[serde(default)]`：保留「缺字段即报错」这条防线。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
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

/// 监控条件。
///
/// ⚠ `#[serde(rename_all = "camelCase")]` 为上收时新加。线上此前**没有**消费者
/// （引擎的 `serialize_configs` / `deserialize_configs` 全仓零调用，也没有任何命令返回它）
/// ⇒ 加注解零风险；而本接缝的 `list_configs` 是它第一次上线，必须按 camelCase 给。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
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

/// 告警事件（derive 与 serde 注解逐字原样上收，它早已是前端 `price-alert-triggered`
/// 的 payload 本体，键集合由 `check-monitor-event-authority.mjs` 的 R4 钉住）。
#[derive(Debug, Clone, Serialize, Deserialize)]
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

/// 一轮轮询的取数读数（裁定 12 ①：「监控实际未在取数」必须是能被面板消费的聚合面）。
///
/// 改造前只有逐只 `tracing::warn!`：整条链路长期全挂时，唯一发现方式是人工翻日志，
/// 而监控面板仍显示「监控运行中」。三条数各自只说一件事，不并排成可比的两个计数：
/// - [`Self::codes_total`] / [`Self::codes_failed`]：**本轮**应取数的股票数与其中失败数；
/// - [`Self::consecutive_all_failed_rounds`]：连续「整轮全失败」的轮数，用于判「通道挂了」；
/// - [`Self::finished_at_ms`]：本轮落数时刻。非交易时段整轮跳过时**不更新**，
///   所以它是「最后一次真取数的时刻」，不是「上一次心跳」。
///
/// `Serialize` / `Deserialize` + camelCase 为上收时补（原先只有 `Debug, Clone, Default`，
/// 过不了帧协议）。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PollRoundStats {
    pub codes_total: u32,
    pub codes_failed: u32,
    pub consecutive_all_failed_rounds: u32,
    pub finished_at_ms: i64,
}

// ── 跨股票聚合器 DTO（上收自 analysis-engine/src/cross_stock_aggregator.rs）──

/// 统一信号类型枚举。
///
/// 替代 `signals.rs` 中 `MACrossResult.signal: String` 和 `BreakoutResult.breakout_type: String`
/// 的字符串字面量，提供类型安全。新增信号类型时在此扩展。
/// 线上值是 snake_case（`rename_all`），与本枚举的变体名逐字对应。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SignalType {
    /// 金叉（短期均线上穿长期均线）
    GoldenCross,
    /// 死叉（短期均线下穿长期均线）
    DeathCross,
    /// 突破压力位
    ResistanceBreak,
    /// 跌破支撑位
    SupportBreak,
    /// 涨跌幅异常
    ChangeSpike,
    /// 换手率异常（量能异动）
    VolumeSpike,
    /// 触及止损
    StopLossHit,
    /// 触及止盈
    TakeProfitHit,
}

impl SignalType {
    /// 信号方向：+1 = 多头/看涨，-1 = 空头/看跌，0 = 中性
    pub fn direction(&self) -> i8 {
        match self {
            SignalType::GoldenCross | SignalType::ResistanceBreak | SignalType::TakeProfitHit => 1,
            SignalType::DeathCross
            | SignalType::SupportBreak
            | SignalType::StopLossHit
            | SignalType::ChangeSpike
            | SignalType::VolumeSpike => -1,
        }
    }

    /// 人类可读名称（用于告警消息）
    pub fn label(&self) -> &'static str {
        match self {
            SignalType::GoldenCross => "金叉",
            SignalType::DeathCross => "死叉",
            SignalType::ResistanceBreak => "突破压力位",
            SignalType::SupportBreak => "跌破支撑位",
            SignalType::ChangeSpike => "涨跌幅异常",
            SignalType::VolumeSpike => "换手率异常",
            SignalType::StopLossHit => "触及止损",
            SignalType::TakeProfitHit => "触及止盈",
        }
    }
}

/// 信号方向枚举（用于组合级聚合时的同向判定）
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SignalDirection {
    Bullish,
    Bearish,
    Neutral,
}

impl SignalDirection {
    pub fn from_i8(d: i8) -> Self {
        if d > 0 {
            SignalDirection::Bullish
        } else if d < 0 {
            SignalDirection::Bearish
        } else {
            SignalDirection::Neutral
        }
    }
}

/// 单股信号载体。
///
/// 由 `RealtimeMonitor` 在每次轮询时构造，喂给 `CrossStockSignalAggregator`。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StockSignal {
    pub stock_code: String,
    pub stock_name: String,
    pub signal_type: SignalType,
    /// 信号强度 [0.0, 1.0]，由调用方根据价格偏离度 / 量能放大倍数等估算
    pub strength: f64,
    /// 信号来源（"monitor" / "signals.detect_ma_cross" / "signals.detect_breakout"）
    pub source: String,
    /// Unix 时间戳（秒）
    pub timestamp: i64,
    /// 触发信号时的当前价（可选，便于前端展示）
    pub current_price: Option<f64>,
    /// 触发信号时的涨跌幅（可选）
    pub change_pct: Option<f64>,
}

/// 组合级信号（聚合后输出）。
///
/// 当多只股票在时间窗内同方向触发信号时生成。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PortfolioSignal {
    /// 组合信号方向（多头聚集 / 空头聚集 / 中性）
    pub direction: SignalDirection,
    /// 触发该组合信号的股票集合
    pub stocks: Vec<String>,
    /// 主导信号类型（多数派）
    pub dominant_signal: SignalType,
    /// 组合信号强度 [0.0, 1.0]
    pub strength: f64,
    /// 触发时的市场快照（可选，便于前端展示）
    pub snapshot: Option<PortfolioSnapshot>,
    /// 建议操作（如 "组合集中度风险升高，建议减仓 20%"）
    pub suggested_action: String,
    /// Unix 时间戳（秒）
    pub timestamp: i64,
}

/// 组合快照（聚合时记录的持仓上下文）
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PortfolioSnapshot {
    /// 持仓股票数
    pub position_count: usize,
    /// 头部集中度（top1 市值占比）
    pub top_concentration_pct: Option<f64>,
    /// 行业最大集中度
    pub max_sector_pct: Option<f64>,
    /// 风险等级
    pub risk_level: Option<String>,
}

/// 聚合器配置。
///
/// ⚠ 这四个数是**组合告警可信度的全部防线**，宿主 provider 必须 clamp（见
/// [`StockMonitorProvider::set_aggregator_config`]）：`min_signal_count = 0` 会让
/// 「候选数 < 最小数」恒 false ⇒ 单只股票一条信号就报组合告警，且强度归一化除零恒 1.0；
/// `cooldown_secs = 0` ⇒ 去重失效；`min_strength < 0` ⇒ 全量放行；`window_secs ≤ 0` ⇒
/// 只留当前这一条。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AggregatorConfig {
    /// 时间窗口（秒），默认 300（5 分钟）
    pub window_secs: i64,
    /// 触发组合信号的最小同向信号数，默认 3
    pub min_signal_count: usize,
    /// 同类组合信号冷却时间（秒），默认 600（10 分钟）
    pub cooldown_secs: i64,
    /// 最小强度阈值（单股强度低于此值不参与聚合），默认 0.3
    pub min_strength: f64,
}

impl Default for AggregatorConfig {
    fn default() -> Self {
        Self { window_secs: 300, min_signal_count: 3, cooldown_secs: 600, min_strength: 0.3 }
    }
}

/// 聚合器配置的合法区间（宿主 provider 据此 clamp；契约级数字，不落引擎）。
///
/// `cooldown_secs` 的下界 0 是合法的「不去重」，`min_strength` 的上界 1.0 是合法的
/// 「只报最强」。`min_signal_count` 下界刻意取 2 —— 取 1 就等于关掉同向聚集规则。
pub const AGGREGATOR_WINDOW_SECS: (i64, i64) = (1, 3600);
/// 见 [`AGGREGATOR_WINDOW_SECS`]。
pub const AGGREGATOR_MIN_SIGNAL_COUNT: (usize, usize) = (2, 50);
/// 见 [`AGGREGATOR_WINDOW_SECS`]。
pub const AGGREGATOR_COOLDOWN_SECS: (i64, i64) = (0, 86400);
/// 见 [`AGGREGATOR_WINDOW_SECS`]。
pub const AGGREGATOR_MIN_STRENGTH: (f64, f64) = (0.0, 1.0);

// ── 本接缝新增的 DTO ────────────────────────────────────────────────────

/// 监控引擎的运行读数视图。
///
/// 由 `commands/stock_analysis.rs` 的 `MonitorStatusDto` 迁入 harness 并补
/// `Deserialize`（键集合逐字不变，前端 `MonitorStatus` 无需改动；R12 仍钉命令层那一份，
/// 宿主批把命令切到本类型后即同一份）。
///
/// 每个键只说一件事，消费端不得把它们收敛成一句（H-4）：
/// - `assembled = false` ⇒ 引擎没装配，**此时其余计数一律不得渲染**（0 不是「没有失败」，
///   是「没人在取数」）
/// - `trading_now = false` ⇒ 非交易时段整轮跳过，轮次计数是上一盘的旧值
/// - `round_codes_failed` / `round_codes_total` ⇒ 本轮取数失败的股票数 / 应取数的股票数
/// - `consecutive_all_failed_rounds` ⇒ 连续「整轮全失败」轮数
/// - `round_finished_at_ms = 0` ⇒ 装配好了但还没跑完第一轮，不能读成「本轮 0/0 失败」
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MonitorStatusView {
    pub assembled: bool,
    /// 时段档位码（`TradingSession::code()`），前端按它查 i18n，不在这里出中文
    pub session: String,
    pub trading_now: bool,
    pub poll_interval_secs: u64,
    pub configs_active: u32,
    pub round_codes_total: u32,
    pub round_codes_failed: u32,
    pub consecutive_all_failed_rounds: u32,
    pub round_finished_at_ms: i64,
}

/// 新建 / 改阈值一条价格告警的入参（`stock.monitor` 的 `upsert_alert` op，四个必填）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AlertUpsertRequest {
    pub stock_code: String,
    pub stock_name: String,
    /// 告警类型 —— **严格白名单**：6 类标准值 `stop_loss` / `take_profit` / `resistance` /
    /// `support` / `change` / `volume`，外加 5 个 legacy 写法 `above` / `below` /
    /// `change_up` / `change_down` / `volume_spike`（它们归一后分别落在
    /// take_profit / stop_loss / change / change / volume）。
    ///
    /// ⚠ **未知类型 ⇒ 拒绝**（`SEAM_CALL_INVALID_ARGS`），不落库。白名单常量与归一函数在
    /// `analysis-engine::alert_mapping`（`normalize_alert_type` / `alert_types`），本层不
    /// 重复定义（禁区 12）；但归一函数对认不出的输入**兜底成 `take_profit`**，命令层靠人
    /// 在面板选类型尚可，插件侧一条拼错的串会落成**反方向**的单 ⇒ 宿主 provider 必须在
    /// 调用归一函数**之前**按白名单拒绝。
    pub alert_type: String,
    /// 阈值语义随 `alert_type` 变：price 类是绝对价，`change` 是涨跌幅百分比，
    /// `volume` 是换手率百分比（与 `price_alerts.threshold` 同义）。
    pub threshold: f64,
}

/// `price_alerts` 行的跨边界视图 —— 镜像实体全部 12 列。
///
/// 不带 ORM 类型：`entities` 依赖 `harness`，反向过线即依赖成环（同
/// [`crate::trading_engine::TradeView`] 的理由）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PriceAlertView {
    pub id: String,
    pub stock_code: String,
    pub stock_name: String,
    /// 老字段（兼容保留）：`"above"` 或 `"below"`。**6→5 有损**，
    /// 不得拿它当去重键或分支依据，一律读 [`Self::alert_type`]。
    pub condition: String,
    /// 老字段（兼容保留）：仅 price 类告警有效。
    pub target_price: f64,
    /// 取值 `stop_loss` / `take_profit` / `resistance` / `support` / `change` / `volume`。
    /// 迁移过渡期的老数据可能为 `None`。
    pub alert_type: Option<String>,
    /// 阈值语义：`price` / `change_pct` / `turnover_rate`。
    pub condition_type: Option<String>,
    /// 通用阈值（与 [`AlertUpsertRequest::threshold`] 同源同值）。
    pub threshold: Option<f64>,
    /// integer 列（`0` / `1`），**不得**收敛成 boolean —— 库里就是整数，
    /// 用 bool 字面量比较会让查询直接报错并被吞成「没有告警」。
    pub is_triggered: i32,
    pub triggered_at: Option<i64>,
    pub created_at: i64,
    pub updated_at: i64,
}

/// `upsert_alert` 的结果。三个判据各有定义，不要收敛成一个 bool。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AlertUpsertOutcome {
    /// 落库后的行（复用时 `id` 不变）。
    pub alert: PriceAlertView,
    /// **复用**了同股同类型的未触发行（裁定 10：改阈值而不是每建一次多一行）。
    pub reused: bool,
    /// 已进监控引擎内存。`false` 有两种合法成因：引擎未装配、或该类型落不成配置
    /// （六类之外）⇒ 库里有一行但**不会被盯**，消费方须如实提示而不是当成功。
    pub wired: bool,
}

/// `remove_alert` 的结果：**两个计数域并列**，不合成一个 bool。
///
/// 步骤「删库」与「摘内存」在引擎未装配时合法地只做前一半 ⇒ 合成一个数就分不清
/// 「行不存在」与「引擎没接」。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoveAlertOutcome {
    /// 是否真删掉了一行 `price_alerts`。行不存在 ⇒ `false`，**不是错误**
    /// （照 `jobs.store get` 的「查无即 null」口径）。
    pub db_deleted: bool,
    /// 是否从引擎内存摘掉了对应配置（含冷却 / 边沿状态回收）。
    pub memory_removed: bool,
}

// ── Trait ───────────────────────────────────────────────────────────────

/// 股票监控提供者（宿主独占 `RealtimeMonitor` + `CrossStockSignalAggregator`）。
///
/// 默认实现统一返回「该提供者不支持此数据面」，便于外部实现只覆盖关心的 op。
///
/// 写方法（`upsert_alert` / `remove_alert` / `set_t0_config` / `set_aggregator_config`）要求
/// 插件已声明 `write` 权限 —— 该检查在 worker 分派层完成（`PLUGIN_PERMISSION_DENIED`），
/// 本 trait 不重复校验。
#[async_trait]
pub trait StockMonitorProvider: Send + Sync {
    /// 运行读数（`status` op，读面）。
    ///
    /// 引擎未装配 ⇒ 返回 `assembled = false` 的**正常响应**，不报 `SEAM_PROVIDER_UNAVAILABLE`
    /// （未装配本身是一种要如实暴露的缺席状态，不是调用失败）。
    async fn status(&self) -> Result<MonitorStatusView> {
        Err(AxAgentError::Provider(
            "StockMonitorProvider::status 未实现（该提供者不支持此数据面）".into(),
        ))
    }

    /// 列出内存中的监控配置（`list_configs` op，读面）。
    ///
    /// 这是「引擎实际在盯哪些条」的真相，与 `price_alerts` 表的两边差集正是
    /// `wired = false` 留下的缺口 ⇒ 消费方可以据此发现脱监的行。
    async fn list_configs(&self) -> Result<Vec<MonitorConfig>> {
        Err(AxAgentError::Provider(
            "StockMonitorProvider::list_configs 未实现（该提供者不支持此数据面）".into(),
        ))
    }

    /// 查询 T+0 自动重跑配置（`get_t0_config` op，读面）。
    async fn t0_config(&self) -> Result<TZeroConfig> {
        Err(AxAgentError::Provider(
            "StockMonitorProvider::t0_config 未实现（该提供者不支持此数据面）".into(),
        ))
    }

    /// 设置 T+0 自动重跑配置（`set_t0_config` op，**写**，要求 `write` 权限）。
    ///
    /// ⚠ 这是**全局开关，无 owner 隔离**：开启后每一轮告警都会经引擎的
    /// `maybe_trigger_t0` → 宿主回调**启动工作流**（一个插件的写入会影响所有插件与前端
    /// 看到的宿主行为）。宿主须 clamp：两个阈值必须 > 0、`min_interval_minutes ≥ 1`
    /// （0 会关掉防风暴的那道闸）；非法值 ⇒ `SEAM_CALL_INVALID_ARGS`。
    /// 入参是**全量结构体**（照命令层 `set_t0_config`），不是 patch。
    async fn set_t0_config(&self, _config: TZeroConfig) -> Result<()> {
        Err(AxAgentError::Provider(
            "StockMonitorProvider::set_t0_config 未实现（该提供者不支持此数据面）".into(),
        ))
    }

    /// 建告警或改其阈值（`upsert_alert` op，**写**，要求 `write` 权限）。
    ///
    /// 复合 op：**必须同时**完成「写 `price_alerts` 行」+「进引擎内存」，宿主实现直接复用
    /// `commands::price_alert_upsert::upsert_price_alert_and_wire`，不要抄第二遍（禁区 12）。
    /// 同股同类型的未触行为**复用并改阈值**（`id` 不变 ⇒ 内存按 `alert_id` 覆盖），
    /// 未命中才新建（`id` = uuid v4）。
    ///
    /// `alert_type` 不在白名单 ⇒ `SEAM_CALL_INVALID_ARGS`（**先拒后归一**，理由见
    /// [`AlertUpsertRequest::alert_type`]）；DB 失败 ⇒ provider 返回 `Err`，跨层落成
    /// `PLUGIN_CALL_FAILED`。
    async fn upsert_alert(&self, _request: AlertUpsertRequest) -> Result<AlertUpsertOutcome> {
        Err(AxAgentError::Provider(
            "StockMonitorProvider::upsert_alert 未实现（该提供者不支持此数据面）".into(),
        ))
    }

    /// 删告警（`remove_alert` op，**写**，要求 `write` 权限）。
    ///
    /// 对称路径三步：先查行（拿 `id` / `stock_code`）→ 删库 → 摘内存配置（连带回收它的
    /// 冷却与边沿状态键，否则那两个 HashMap 只涨不落）。行不存在 ⇒ `Ok` + 两个 false，
    /// **不是错误**。
    async fn remove_alert(&self, _alert_id: &str) -> Result<RemoveAlertOutcome> {
        Err(AxAgentError::Provider(
            "StockMonitorProvider::remove_alert 未实现（该提供者不支持此数据面）".into(),
        ))
    }

    /// 聚合器配置（`aggregator_config` op，读面）。
    async fn aggregator_config(&self) -> Result<AggregatorConfig> {
        Err(AxAgentError::Provider(
            "StockMonitorProvider::aggregator_config 未实现（该提供者不支持此数据面）".into(),
        ))
    }

    /// 设置聚合器配置（`set_aggregator_config` op，**写**，要求 `write` 权限）。
    ///
    /// ⚠ **要求全量结构体并由宿主 clamp**（范围见 [`AGGREGATOR_WINDOW_SECS`] 一族常量）：
    /// 这四个数是组合告警可信度的全部防线，不能让一条调用把它们清零。越界值一律
    /// `SEAM_CALL_INVALID_ARGS`，**不要静默夹取** —— 静默夹取会让插件以为自己设成了 0。
    async fn set_aggregator_config(&self, _config: AggregatorConfig) -> Result<()> {
        Err(AxAgentError::Provider(
            "StockMonitorProvider::set_aggregator_config 未实现（该提供者不支持此数据面）".into(),
        ))
    }

    /// 时间窗内的单股信号缓冲（`aggregator_buffer` op，读面）。
    ///
    /// 同一只股票在窗内只保留最新一条（引擎的缓冲区按 `stock_code` 覆盖）。
    /// 「窗口内有几只同向」由消费方用 [`SignalType::direction()`] 自己数 ⇒
    /// 首批**不发** `aggregator_stats`（那需要新引擎方法读私有的冷却表）。
    async fn aggregator_buffer(&self) -> Result<Vec<StockSignal>> {
        Err(AxAgentError::Provider(
            "StockMonitorProvider::aggregator_buffer 未实现（该提供者不支持此数据面）".into(),
        ))
    }
}

// ── 事件 kind（裁定 N6：发射点只准引用常量，禁止再抄一遍字面量）───────────

/// 价格告警触发时经 `event.dispatch` 推给插件的 `DomainEvent.kind`。
///
/// 前缀必须等于 `EventCategory::Stock::as_str()`：`&str` 常量无法在编译期拼一个方法调用的
/// 结果，故这一格由本模块单测 `stock_event_kind_prefix_matches_category` 在运行期钉住。
/// 发射点（`src-tauri/src/init/monitor_emitter.rs` 的第四跳）**只引用本常量**。
///
/// ⚠ 与 Tauri 事件名 `IpcEventName::PriceAlertTriggered`（`"price-alert-triggered"`）**不同域**：
/// 那一份是前端 `listen()` 的 IPC 名，这一份是插件总线 `event.dispatch` 的订阅匹配串；
/// 同一条告警两头各发一次，但键空间互不替代（裁定 N6 明确「不复用 `IpcEventName`」）。
pub const STOCK_PRICE_ALERT_KIND: &str = "stock/price_alert";

/// 组合级信号（跨股聚合器产出）的 `DomainEvent.kind`，前缀同上。
///
/// 与 `IpcEventName::PortfolioSignal` 的关系同 [`STOCK_PRICE_ALERT_KIND`]。
pub const STOCK_PORTFOLIO_SIGNAL_KIND: &str = "stock/portfolio_signal";

// ── 测试 ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn sample_config() -> TZeroConfig {
        TZeroConfig {
            enabled: true,
            change_pct_threshold: Some(3.0),
            turnover_rate_threshold: Some(8.0),
            min_interval_minutes: 30,
        }
    }

    /// TZeroConfig 的线上键集合（camelCase）—— 钉住 res104 §0-3 那条既存缺陷的修复。
    #[test]
    fn t0_config_camel_keys() {
        let val = serde_json::to_value(sample_config()).expect("序列化应成功");
        assert_eq!(val["enabled"], json!(true));
        assert_eq!(val["changePctThreshold"], json!(3.0));
        assert_eq!(val["turnoverRateThreshold"], json!(8.0));
        assert_eq!(val["minIntervalMinutes"], json!(30));
        // 不得残留 snake_case 键（缺一个注解就会让前端读到 undefined）
        assert!(val.get("change_pct_threshold").is_none());
        assert!(val.get("turnover_rate_threshold").is_none());
        assert!(val.get("min_interval_minutes").is_none());

        // 前端那份 camelCase 载荷必须能整条反序列化回来（修复前因缺
        // `min_interval_minutes` 而整条 reject）
        let back: TZeroConfig = serde_json::from_value(val).expect("camelCase 载荷应能落库");
        assert_eq!(back.min_interval_minutes, 30);
        assert_eq!(back.change_pct_threshold, Some(3.0));
        assert_eq!(back.turnover_rate_threshold, Some(8.0));
    }

    #[test]
    fn monitor_config_camel_keys_and_options_are_null() {
        let cfg = MonitorConfig {
            alert_id: "alert-1".into(),
            stock_code: "600519".into(),
            stock_name: "贵州茅台".into(),
            stop_loss: Some(1400.0),
            take_profit: None,
            resistance_break: None,
            support_break: None,
            change_pct_alert: None,
            turnover_rate_alert: None,
            enabled: true,
        };
        let val = serde_json::to_value(cfg).expect("序列化应成功");
        assert_eq!(val["alertId"], json!("alert-1"));
        assert_eq!(val["stockCode"], json!("600519"));
        assert_eq!(val["stopLoss"], json!(1400.0));
        assert_eq!(val["changePctAlert"], json!(null));
        assert!(val.get("alert_id").is_none());
        assert!(val.get("take_profit").is_none());
    }

    #[test]
    fn poll_round_stats_serde_camel() {
        let stats = PollRoundStats {
            codes_total: 16,
            codes_failed: 3,
            consecutive_all_failed_rounds: 0,
            finished_at_ms: 1_700_000_000_000,
        };
        let val = serde_json::to_value(&stats).expect("补了 Serialize 才能过帧");
        assert_eq!(val["codesTotal"], json!(16));
        assert_eq!(val["codesFailed"], json!(3));
        assert_eq!(val["consecutiveAllFailedRounds"], json!(0));
        assert_eq!(val["finishedAtMs"], json!(1_700_000_000_000_i64));
        let back: PollRoundStats = serde_json::from_value(val).expect("往返应成立");
        assert_eq!(back.finished_at_ms, 1_700_000_000_000);
    }

    #[test]
    fn monitor_status_view_roundtrip() {
        let view = MonitorStatusView {
            assembled: true,
            session: "intraday".into(),
            trading_now: true,
            poll_interval_secs: 30,
            configs_active: 16,
            round_codes_total: 16,
            round_codes_failed: 0,
            consecutive_all_failed_rounds: 0,
            round_finished_at_ms: 1_700_000_000_000,
        };
        let json = serde_json::to_string(&view).expect("serialize");
        let back: MonitorStatusView = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back.poll_interval_secs, 30);
        assert_eq!(back.round_finished_at_ms, 1_700_000_000_000);
        assert!(back.assembled && back.trading_now);
        // 九个键逐项 camelCase，与命令层 `MonitorStatusDto` / 前端 `MonitorStatus` 同源
        let val = serde_json::to_value(&view).expect("serialize");
        for key in [
            "assembled",
            "session",
            "tradingNow",
            "pollIntervalSecs",
            "configsActive",
            "roundCodesTotal",
            "roundCodesFailed",
            "consecutiveAllFailedRounds",
            "roundFinishedAtMs",
        ] {
            assert!(!val.get(key).is_none(), "缺键 {key}");
        }
        assert!(val.get("trading_now").is_none());
    }

    #[test]
    fn upsert_outcome_camel() {
        let outcome = AlertUpsertOutcome {
            alert: PriceAlertView {
                id: "uuid-1".into(),
                stock_code: "600519".into(),
                stock_name: "贵州茅台".into(),
                condition: "below".into(),
                target_price: 1400.0,
                alert_type: Some("stop_loss".into()),
                condition_type: Some("price".into()),
                threshold: Some(1400.0),
                is_triggered: 0,
                triggered_at: None,
                created_at: 1_700_000_000_000,
                updated_at: 1_700_000_000_001,
            },
            reused: true,
            wired: false,
        };
        let val = serde_json::to_value(&outcome).expect("序列化应成功");
        assert_eq!(val["reused"], json!(true));
        // `wired = false` 必须原样透出：它是「库里有一行但不会被盯」的唯一信号
        assert_eq!(val["wired"], json!(false));
        assert_eq!(val["alert"]["alertType"], json!("stop_loss"));
        assert_eq!(val["alert"]["isTriggered"], json!(0));
        assert_eq!(val["alert"]["conditionType"], json!("price"));
        assert!(val["alert"].get("is_triggered").is_none());
        assert!(val["alert"].get("alert_type").is_none());

        let removed = RemoveAlertOutcome { db_deleted: true, memory_removed: false };
        let removed_val = serde_json::to_value(removed).expect("序列化应成功");
        assert_eq!(removed_val["dbDeleted"], json!(true));
        assert_eq!(removed_val["memoryRemoved"], json!(false));
        assert!(removed_val.get("db_deleted").is_none());
    }

    #[test]
    fn alert_upsert_request_camel_and_rejects_unknown_shape() {
        let request: AlertUpsertRequest = serde_json::from_value(json!({
            "stockCode": "600519",
            "stockName": "贵州茅台",
            "alertType": "stop_loss",
            "threshold": 1400.0,
        }))
        .expect("camelCase 入参应能反序列化");
        assert_eq!(request.stock_code, "600519");
        assert_eq!(request.threshold, 1400.0);

        // 四个必填缺一即 reject（本层不设 default：缺参要在分派层就报 invalid_args）
        assert!(
            serde_json::from_value::<AlertUpsertRequest>(json!({
                "stockCode": "600519",
                "alertType": "stop_loss",
                "threshold": 1400.0,
            }))
            .is_err()
        );
    }

    #[test]
    fn signal_type_serde_and_direction() {
        assert_eq!(
            serde_json::to_value(SignalType::TakeProfitHit).expect("serialize"),
            json!("take_profit_hit")
        );
        assert_eq!(
            serde_json::to_value(SignalDirection::Bearish).expect("serialize"),
            json!("bearish")
        );
        assert_eq!(SignalType::GoldenCross.direction(), 1);
        assert_eq!(SignalType::SupportBreak.direction(), -1);
        assert_eq!(SignalDirection::from_i8(0), SignalDirection::Neutral);
        assert_eq!(SignalType::DeathCross.label(), "死叉");
    }

    #[test]
    fn aggregator_config_defaults_and_bounds() {
        let cfg = AggregatorConfig::default();
        assert_eq!(cfg.window_secs, 300);
        assert_eq!(cfg.min_signal_count, 3);
        let val = serde_json::to_value(&cfg).expect("序列化应成功");
        assert_eq!(val["windowSecs"], json!(300));
        assert_eq!(val["minSignalCount"], json!(3));
        assert_eq!(val["cooldownSecs"], json!(600));
        assert_eq!(val["minStrength"], json!(0.3));
        // clamp 区间本身是契约的一部分：默认值必须落在区间内
        assert!(
            cfg.window_secs >= AGGREGATOR_WINDOW_SECS.0
                && cfg.window_secs <= AGGREGATOR_WINDOW_SECS.1
        );
        assert!(
            cfg.min_signal_count >= AGGREGATOR_MIN_SIGNAL_COUNT.0
                && cfg.min_signal_count <= AGGREGATOR_MIN_SIGNAL_COUNT.1
        );
        assert!(
            cfg.cooldown_secs >= AGGREGATOR_COOLDOWN_SECS.0
                && cfg.cooldown_secs <= AGGREGATOR_COOLDOWN_SECS.1
        );
        assert!(
            cfg.min_strength >= AGGREGATOR_MIN_STRENGTH.0
                && cfg.min_strength <= AGGREGATOR_MIN_STRENGTH.1
        );
    }

    /// 未覆盖任何方法的实现：九个 op 全部走默认实现 ⇒ 一律 `Err(Provider)`，不得 panic。
    struct BareMonitor;
    impl StockMonitorProvider for BareMonitor {}

    #[test]
    fn default_trait_methods_unimplemented() {
        let provider = BareMonitor;
        let checks: Vec<(&str, Result<serde_json::Value>)> = vec![
            ("status", futures::executor::block_on(provider.status()).map(|v| json!(v))),
            (
                "list_configs",
                futures::executor::block_on(provider.list_configs()).map(|v| json!(v)),
            ),
            ("t0_config", futures::executor::block_on(provider.t0_config()).map(|v| json!(v))),
            (
                "set_t0_config",
                futures::executor::block_on(provider.set_t0_config(sample_config()))
                    .map(|_| json!(null)),
            ),
            (
                "upsert_alert",
                futures::executor::block_on(provider.upsert_alert(AlertUpsertRequest {
                    stock_code: "600519".into(),
                    stock_name: "贵州茅台".into(),
                    alert_type: "stop_loss".into(),
                    threshold: 1400.0,
                }))
                .map(|v| json!(v)),
            ),
            (
                "remove_alert",
                futures::executor::block_on(provider.remove_alert("id-1")).map(|v| json!(v)),
            ),
            (
                "aggregator_config",
                futures::executor::block_on(provider.aggregator_config()).map(|v| json!(v)),
            ),
            (
                "set_aggregator_config",
                futures::executor::block_on(
                    provider.set_aggregator_config(AggregatorConfig::default()),
                )
                .map(|_| json!(null)),
            ),
            (
                "aggregator_buffer",
                futures::executor::block_on(provider.aggregator_buffer()).map(|v| json!(v)),
            ),
        ];
        assert_eq!(checks.len(), 9);
        for (op, outcome) in checks {
            let err = outcome.expect_err("默认实现必须返回 Err");
            match err {
                AxAgentError::Provider(msg) => {
                    assert!(msg.contains("StockMonitorProvider::"), "文案须点明 trait：{msg}");
                    assert!(msg.contains(op), "文案须点明 op {op}：{msg}");
                    assert!(msg.contains("未实现"), "文案须说明未实现：{msg}");
                },
                other => panic!("{op} 的默认实现应返回 AxAgentError::Provider，实际 {other:?}"),
            }
        }
    }

    /// trait 必须可作 `Arc<dyn …>` 注入（dyn 兼容是接缝成立的前提）。
    #[test]
    fn trait_is_object_safe() {
        let provider: std::sync::Arc<dyn StockMonitorProvider> = std::sync::Arc::new(BareMonitor);
        let outcome = futures::executor::block_on(provider.aggregator_buffer());
        assert!(outcome.is_err());
    }

    /// 裁定 N6 的形态锁：两条 stock 域 kind 的前缀必须等于 `EventCategory::Stock::as_str()`。
    ///
    /// 为什么不是编译期断言：`as_str()` 是方法调用，`&str` 常量拼不出它；把前缀抄成字面量
    /// 又正是本裁定要消灭的「两头各抄一遍」。故落在运行期，红得比漂移传出去更早。
    #[test]
    fn stock_event_kind_prefix_matches_category() {
        use crate::EventCategory;

        let prefix = format!("{}/", EventCategory::Stock.as_str());
        for kind in [STOCK_PRICE_ALERT_KIND, STOCK_PORTFOLIO_SIGNAL_KIND] {
            assert!(kind.starts_with(&prefix), "kind `{kind}` 的前缀不是 `{prefix}`");
            // 前缀之后必须还有内容：`"stock/"` 这种空 kind 会被订阅端当「全匹配」误收
            assert!(kind.len() > prefix.len(), "kind `{kind}` 只有前缀，没有具体事件名");
        }
        assert_ne!(STOCK_PRICE_ALERT_KIND, STOCK_PORTFOLIO_SIGNAL_KIND, "两条 kind 必须可区分");
    }
}
