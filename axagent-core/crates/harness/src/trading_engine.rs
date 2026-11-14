// SPDX-License-Identifier: AGPL-3.0-only

//! 交易引擎接缝（`trading.engine`）— DTO + Trait。
//!
//! 宿主独占 `TradingEngine`（`axagent-quant/crates/analysis-engine/src/trading.rs`），
//! 插件只经本接缝调用，**拿不到引擎实例，也拿不到 `axagent-entities` 的 ORM 类型**
//! （`entities` 已依赖 `harness`，反向依赖即 `harness ⇄ entities` 成环）。
//!
//! ## `as_of` 约定
//!
//! 只有会**触发行情取数**的 op 才收 `as_of`，且**恒为末参**：`get_positions` /
//! `validate_trade` / `execute_trade`。`get_trades` / `compare_trade` /
//! `trading_enabled` 是纯 DB 读，不收 `as_of`。
//! `as_of` 的落地由**宿主**用 `axagent_astock_data::as_of::with_optional_asof` 包裹成
//! RAII 作用域（离开即按 token 摘层），provider 实现方不要自行改写全局状态。
//!
//! ## 语义约定
//!
//! - **无结果缓存**：`get_positions` 每次都重新取行情与行业分类（成本 ≈ 3N+1 次往返，
//!   N = 持仓数）。接缝不做缓存，缓存策略若要做也只能在宿主 provider 内部做并显式记账。
//! - **业务拒绝不是错误**：`execute_trade` 的 `accepted = false`（开关关闭 / 校验不过 /
//!   重复提交）是**正常业务结果**，走 [`TradeExecutionOutcome::reason`]，不借用错误码。
//!   只有真故障（DB 错、行情全挂）才返回 `Err`。
//! - **开关口径唯一**：`trading_enabled` 读的是 settings 表的 `trading_enabled`；
//!   引擎结构体上的 `pub enabled: bool` 是死字段（恒 `false`、全仓无写入点），
//!   **不得**作为闸，也不得被 provider 赋值（否则会造出「内存 true / DB false」的第二权威）。
//! - **无 owner 隔离**：交易账本属于**用户**而非插件，所有 op 都不带 owner 参数
//!   （对比 `workflow.run` 的按发起插件隔离）。写权限由 worker 分派层按 `write` 校验。
//!
//! 权限与超时都在分派层：`execute_trade` 是**写 op**，要求插件已声明 `write`
//! （拒绝码 `PLUGIN_PERMISSION_DENIED`）；本层只提供契约，不重复校验。

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::AsOfContext;
use crate::core_error::{AxAgentError, Result};

// ── DTO（跨进程视图；ORM 类型不过线）────────────────────────────────────

/// 持仓汇总（含实时盈亏）—— 上收自 `analysis-engine/src/trading.rs`，字段逐字不变。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PositionSummary {
    pub stock_code: String,
    pub stock_name: String,
    pub total_shares: i32,
    pub avg_cost: f64,
    pub current_price: Option<f64>,
    pub market_value: Option<f64>,
    pub unrealized_pnl: Option<f64>,
    pub unrealized_pnl_pct: Option<f64>,
    pub total_realized_pnl: f64,
    pub sector_name: Option<String>,
}

/// 交易记录视图 —— 镜像 `axagent_entities::trades::Model` 的**全部 13 个列**。
///
/// 不带 ORM 泛型 / 关系字段：`entities` 依赖 `harness`，ORM 类型反向过线即依赖成环。
/// 字段类型照实体：时间为 `String`（`YYYY-MM-DD` / `HH:MM`），`created_at` 为毫秒时间戳 `i64`。
/// 注意实体**没有** `analysis_id` 列（引擎当前直接丢弃该入参），故本视图也不带该字段。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TradeView {
    pub id: String,
    pub stock_code: String,
    pub stock_name: String,
    /// 交易方向：`"buy"` | `"sell"`。
    pub direction: String,
    /// 成交价。
    pub price: f64,
    /// 股数。
    pub quantity: i32,
    /// 交易日期 `YYYY-MM-DD`。
    pub trade_date: String,
    /// 交易时间 `HH:MM`。
    pub trade_time: String,
    /// 手续费（单条录入恒 `None`；CSV 导入会写真实值）。
    pub fee: Option<f64>,
    /// 卖出时计算的已实现盈亏。
    pub realized_pnl: Option<f64>,
    /// 所属策略（自动策略预留；手动录入恒 `None`）。
    pub strategy: Option<String>,
    pub notes: Option<String>,
    /// 记录创建时刻（毫秒时间戳）。
    pub created_at: i64,
}

/// 交易校验结果 —— 上收自 `analysis-engine/src/trading.rs`，本层补 serde。
///
/// `valid = false` 是**正常结果**（不是错误），故引擎侧与本接缝都不包 `Result`。
/// `errors` / `warnings` 目前是引擎硬编码中文文案（既存债，本层原样透传）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TradeValidation {
    pub valid: bool,
    pub errors: Vec<String>,
    pub warnings: Vec<String>,
}

/// 交易校验入参（一个对象，避免 5 个平铺参数过帧）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TradeValidationRequest {
    pub stock_code: String,
    /// 交易方向：`"buy"` | `"sell"`。
    pub direction: String,
    pub quantity: i32,
    pub price: f64,
    /// 入场价偏离分析目标价的允许百分比；`None` 用引擎缺省（5.0）。
    #[serde(default)]
    pub price_deviation_limit: Option<f64>,
}

/// 下单入参。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TradeExecutionRequest {
    pub stock_code: String,
    pub stock_name: String,
    /// 交易方向：`"buy"` | `"sell"`。
    pub direction: String,
    pub price: f64,
    pub quantity: i32,
    /// 交易日期 `YYYY-MM-DD`。
    pub trade_date: String,
    /// 交易时间 `HH:MM`。
    pub trade_time: String,
    #[serde(default)]
    pub notes: Option<String>,
    /// 关联的分析 id。⚠ 引擎当前未持久化该字段（`trades` 实体无此列），
    /// 接缝照实透传，**不承诺**它能被读回。
    #[serde(default)]
    pub analysis_id: Option<String>,
}

/// 下单结果 —— 业务拒绝做成**数据**，而不是错误码。
///
/// 三种结局自解释：
/// - 受理 ⇒ `accepted = true`，`trade` 为落库后的记录，`reason` 为 `None`；
/// - 业务拒绝 ⇒ `accepted = false` + `reason`（见下），`trade` 为 `None`；
/// - 真故障（DB 错等）⇒ 返回 `Err`，不落到本结构体。
///
/// `reason` 的**已知取值**（字符串而非常量枚举，便于后续扩展而不破旧插件）：
/// - `"trading_disabled"`：settings 的 `trading_enabled` 为 false（对应命令层错误码
///   `TRADING_DISABLED` 的语义，但这里是正常业务态，不走错误码）；
/// - `"validation_failed"`：校验未通过，明细在 `errors`（开关关闭时引擎**不**跑校验，
///   故此时 `errors` 为空）；
/// - `"duplicate"`：与既有记录同键（实现方可选，本层只约定取值）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TradeExecutionOutcome {
    pub accepted: bool,
    /// 拒绝原因，取值见类型文档；受理时为 `None`。
    pub reason: Option<String>,
    /// 受理时落库的交易记录。
    pub trade: Option<TradeView>,
    /// 校验错误明细（`accepted = false` 且 `reason = "validation_failed"` 时非空）。
    pub errors: Vec<String>,
    /// 校验警告（不影响受理）。
    pub warnings: Vec<String>,
}

/// 实际交易 vs 分析预测对比 —— 上收自 `analysis-engine/src/trading.rs`，字段逐字不变。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TradePredictionComparison {
    pub analysis_action: String,
    pub analysis_target: Option<f64>,
    pub analysis_stop: Option<f64>,
    pub actual_price: f64,
    pub target_deviation_pct: f64,
}

// ── Trait ──────────────────────────────────────────────────────────────

/// 交易引擎提供者（宿主独占 `TradingEngine`，插件只经本接缝调用）。
///
/// 默认实现统一返回「该提供者不支持此数据面」，便于外部实现只覆盖关心的 op。
///
/// 写方法 `execute_trade` 要求插件已声明 `write` 权限 —— 该检查在 worker 分派层完成
/// （`PLUGIN_PERMISSION_DENIED`），本 trait 不重复校验。
/// `as_of` 只出现在会触发行情取数的 op 上，且**恒为末参**；由宿主用
/// `with_optional_asof` 包裹，provider 实现方不得自行改写全局回放状态。
#[async_trait]
pub trait TradingEngineProvider: Send + Sync {
    /// 持仓汇总（含实时盈亏）。会取行情 + 行业分类 ⇒ 收 `as_of`。
    ///
    /// **不做结果缓存**：每次调用都重新取数（成本 ≈ 3N+1 次往返）。
    async fn get_positions(&self, _as_of: Option<AsOfContext>) -> Result<Vec<PositionSummary>> {
        Err(AxAgentError::Provider(
            "TradingEngineProvider::get_positions 未实现（该提供者不支持此数据面）".into(),
        ))
    }

    /// 交易历史（按 `created_at` 倒序）。纯 DB ⇒ 不收 `as_of`。
    ///
    /// `stock_code = None` 返回全部；`limit` 由宿主钳位（引擎本身无上限）。
    async fn get_trades(&self, _stock_code: Option<&str>, _limit: u32) -> Result<Vec<TradeView>> {
        Err(AxAgentError::Provider(
            "TradingEngineProvider::get_trades 未实现（该提供者不支持此数据面）".into(),
        ))
    }

    /// 交易校验（只读）。会取行情 ⇒ 收 `as_of`。
    ///
    /// 校验不过 ⇒ `valid = false` + `errors`，**不返回 `Err`**。
    async fn validate_trade(
        &self,
        _request: TradeValidationRequest,
        _as_of: Option<AsOfContext>,
    ) -> Result<TradeValidation> {
        Err(AxAgentError::Provider(
            "TradingEngineProvider::validate_trade 未实现（该提供者不支持此数据面）".into(),
        ))
    }

    /// 执行交易（**写**，要求 `write` 权限）。会经校验取行情 ⇒ 收 `as_of`。
    ///
    /// 「开关关闭」「校验不过」「重复提交」一律以 `accepted = false` + `reason` 表达，
    /// **不借用错误码**；只有真故障才 `Err`。
    async fn execute_trade(
        &self,
        _request: TradeExecutionRequest,
        _as_of: Option<AsOfContext>,
    ) -> Result<TradeExecutionOutcome> {
        Err(AxAgentError::Provider(
            "TradingEngineProvider::execute_trade 未实现（该提供者不支持此数据面）".into(),
        ))
    }

    /// 对比某笔交易与交易前最近一次已完成分析。纯 DB + `trade_id` ⇒ 不收 `as_of`。
    ///
    /// 查无（交易不存在 / 交易前无已完成分析）⇒ `Ok(None)`，不借错误码。
    async fn compare_trade(&self, _trade_id: &str) -> Result<Option<TradePredictionComparison>> {
        Err(AxAgentError::Provider(
            "TradingEngineProvider::compare_trade 未实现（该提供者不支持此数据面）".into(),
        ))
    }

    /// 交易开关的真实状态 —— 读 settings 的 `trading_enabled`，
    /// **不读**引擎的 `enabled` 死字段（恒 false）。
    async fn trading_enabled(&self) -> Result<bool> {
        Err(AxAgentError::Provider(
            "TradingEngineProvider::trading_enabled 未实现（该提供者不支持此数据面）".into(),
        ))
    }

    // ── F2 组合与风控四条（#35，#322）：持仓写口 / 限额读口 / VLM 导入 / 压测 ────
    // 四条 DTO（`PositionLimits` / `StressTestBundle`）权威都在 `analysis-engine`
    // （implementor），harness 是 foundation 不得依赖 ⇒ 照 `stock_analysis` 接缝的
    // 归属判据走 **Value 透传**，不上收（上收即禁区 12 第二份定义）。
    // 权限闸登记在 `worker.rs` 的 `required_permission_for`：`update_portfolio_holding`
    // 与 `import_portfolio_from_vlm` 是写 op（要 `write`），`get_position_limits` 与
    // `run_stress_test` 纯读/纯计算面 ⇒ 无闸。`run_stress_test` 内部是 N+1 网络取数
    // （`estimate_betas` 逐持仓拉 K 线），**执行体整体留宿主**——不把长任务搬进
    // worker 帧，插件侧等一条汇总结果即可。

    /// 读取全局仓位限制（`get_position_limits` op）—— 读 op，无权限闸。
    ///
    /// 入参 `{}`；出参 = `analysis-engine` 的 `PositionLimits` 原样 JSON（camelCase）。
    /// ⚠ `PositionLimits::panel_effective()` 读 harness 全局面板变量，**只在宿主
    /// 进程内有效**（插件是独立进程，静态变量不同步）⇒ 必须走本 op 而非插件直调。
    async fn get_position_limits(&self, _input: Value) -> Result<Value> {
        Err(AxAgentError::Provider(
            "TradingEngineProvider::get_position_limits 未实现（该提供者不支持此数据面）".into(),
        ))
    }

    /// 更新单条持仓的数量与成本（`update_portfolio_holding` op）—— 写 op，要 `write`。
    ///
    /// 入参 snake_case 接缝风格 `{"id": str, "shares": f64, "avg_cost": f64}`；
    /// 出参 `null`（与被退役的宿主命令 `update_portfolio_holding` 回 `()` 同形状）。
    async fn update_portfolio_holding(&self, _input: Value) -> Result<Value> {
        Err(AxAgentError::Provider(
            "TradingEngineProvider::update_portfolio_holding 未实现（该提供者不支持此数据面）"
                .into(),
        ))
    }

    /// 批量导入 VLM 识别的持仓（`import_portfolio_from_vlm` op）—— 写 op，要 `write`。
    ///
    /// 入参 snake_case 接缝风格 `{"raw_vlm_output": str, "replace_existing": bool?}`；
    /// 出参 = 手拼 JSON `{success, imported, failed, stockCodes, errors}`（解析失败时
    /// `{success: false, error, holdings: []}`），与被退役的宿主命令逐字同形状。
    async fn import_portfolio_from_vlm(&self, _input: Value) -> Result<Value> {
        Err(AxAgentError::Provider(
            "TradingEngineProvider::import_portfolio_from_vlm 未实现（该提供者不支持此数据面）"
                .into(),
        ))
    }

    /// 运行组合压力测试（`run_stress_test` op）—— 纯计算 op（无 DB 副作用），无权限闸。
    ///
    /// 入参 `{}`；出参 = `analysis-engine` 的 `StressTestBundle` 原样 JSON（camelCase）。
    /// N+1 网络取数慢 op：宿主 provider 不得持锁跨 await（照原命令「先释放读锁再取数」
    /// 的结构）。⚠ 超时**不进** `SEAM_OP_TIMEOUT_OVERRIDES`——收录判据要求盘上实测
    /// 或宿主预算出处，两者皆无（Tauri 命令通道无预算概念）⇒ 留默认预算、标
    /// 「预算待实证」，与 `chat` / `reflect` 同款处理。
    async fn run_stress_test(&self, _input: Value) -> Result<Value> {
        Err(AxAgentError::Provider(
            "TradingEngineProvider::run_stress_test 未实现（该提供者不支持此数据面）".into(),
        ))
    }

    /// 校验组合回测配置（`validate_portfolio_config` op）—— 读 op（纯计算，不落库、
    /// 不取行情、无 DB 副作用），无权限闸（与 `run_stress_test` 同口径）。
    ///
    /// ⚠ op 名与被退役的宿主命令名**不同形**：命令叫 `portfolio_backtest_run`，
    /// op 跟 trait 方法（照 `run_stress_test` 先例）。`PortfolioConfig` /
    /// `PortfolioEngine` 的权威在 `quant`（consumer crate），harness 是 foundation
    /// 不得依赖 ⇒ 照 `stock_analysis` 接缝的归属判据走 **Value 透传**，不上收 DTO。
    ///
    /// 入参 `{"config_json": str}`（宿主命令的 `configJson` 字符串**原样**——它才是
    /// 反序列化的形状权威）；出参 = `{status, strategies, message}` 三字段（与被退役的
    /// 宿主命令逐字同形状：`status = "config_valid"`、`strategies` = 配置内策略数、
    /// `message` = 「配置校验通过。回测执行需要前端传入K线数据。」）。宿主 provider 的
    /// 语义 = `PortfolioConfig` 反序列化 + `PortfolioEngine::new` 构造即弃（配置校验
    /// no-op，实际执行不在本 op 职责内）。
    async fn validate_portfolio_config(&self, _input: Value) -> Result<Value> {
        Err(AxAgentError::Provider(
            "TradingEngineProvider::validate_portfolio_config 未实现（该提供者不支持此数据面）"
                .into(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn sample_trade() -> TradeView {
        TradeView {
            id: "trade-1".into(),
            stock_code: "600519".into(),
            stock_name: "贵州茅台".into(),
            direction: "buy".into(),
            price: 1500.0,
            quantity: 100,
            trade_date: "2026-11-02".into(),
            trade_time: "14:30".into(),
            fee: None,
            realized_pnl: None,
            strategy: None,
            notes: Some("测试".into()),
            created_at: 1_700_000_000_000,
        }
    }

    #[test]
    fn trade_view_camel_case() {
        let val = serde_json::to_value(sample_trade()).expect("序列化应成功");

        assert_eq!(val["stockCode"], json!("600519"));
        assert_eq!(val["stockName"], json!("贵州茅台"));
        assert_eq!(val["tradeDate"], json!("2026-11-02"));
        assert_eq!(val["tradeTime"], json!("14:30"));
        assert_eq!(val["createdAt"], json!(1_700_000_000_000_i64));
        // 不得残留 snake_case 键
        assert!(val.get("stock_code").is_none());
        assert!(val.get("trade_date").is_none());
        assert!(val.get("trade_time").is_none());
        assert!(val.get("realized_pnl").is_none());
        // 实体无 analysis_id 列 ⇒ 视图也不得凭空造一个
        assert!(val.get("analysisId").is_none());
        assert!(val.get("analysis_id").is_none());
    }

    #[test]
    fn execution_outcome_roundtrip() {
        let outcome = TradeExecutionOutcome {
            accepted: false,
            reason: Some("trading_disabled".into()),
            trade: None,
            errors: Vec::new(),
            warnings: Vec::new(),
        };

        let val = serde_json::to_value(&outcome).expect("序列化应成功");
        assert_eq!(val["accepted"], json!(false));
        assert_eq!(val["reason"], json!("trading_disabled"));
        assert_eq!(val["trade"], json!(null));
        assert_eq!(val["errors"], json!([]));
        assert_eq!(val["warnings"], json!([]));

        let back: TradeExecutionOutcome = serde_json::from_value(val).expect("反序列化应成功");
        assert!(!back.accepted);
        assert_eq!(back.reason.as_deref(), Some("trading_disabled"));
        assert!(back.trade.is_none());

        // 受理态：带 trade 也要能原样回来
        let accepted = TradeExecutionOutcome {
            accepted: true,
            reason: None,
            trade: Some(sample_trade()),
            errors: Vec::new(),
            warnings: vec!["当前非交易日".into()],
        };
        let back: TradeExecutionOutcome =
            serde_json::from_value(serde_json::to_value(&accepted).unwrap()).unwrap();
        assert!(back.accepted);
        assert!(back.reason.is_none());
        assert_eq!(back.trade.expect("应带 trade").stock_code, "600519");
        assert_eq!(back.warnings.len(), 1);
    }

    #[test]
    fn validation_request_optional_threshold() {
        let req: TradeValidationRequest = serde_json::from_value(json!({
            "stockCode": "600519",
            "direction": "buy",
            "quantity": 100,
            "price": 1500.0
        }))
        .expect("省略 priceDeviationLimit 应能反序列化");
        assert!(req.price_deviation_limit.is_none(), "缺省必须是 None（引擎侧回落 5.0）");

        let req: TradeValidationRequest = serde_json::from_value(json!({
            "stockCode": "600519",
            "direction": "buy",
            "quantity": 100,
            "price": 1500.0,
            "priceDeviationLimit": 3.0
        }))
        .expect("带阈值应能反序列化");
        assert_eq!(req.price_deviation_limit, Some(3.0));
        assert_eq!(req.quantity, 100);
    }

    struct Unimplemented;

    impl TradingEngineProvider for Unimplemented {}

    #[tokio::test]
    async fn default_trait_methods_are_unimplemented() {
        let p = Unimplemented;

        let err = p.get_positions(None).await.expect_err("get_positions 默认实现必须是错误");
        assert!(matches!(err, AxAgentError::Provider(_)), "get_positions: {err:?}");

        let err = p.get_trades(None, 50).await.expect_err("get_trades 默认实现必须是错误");
        assert!(matches!(err, AxAgentError::Provider(_)), "get_trades: {err:?}");

        let err = p
            .validate_trade(
                TradeValidationRequest {
                    stock_code: "600519".into(),
                    direction: "buy".into(),
                    quantity: 100,
                    price: 1500.0,
                    price_deviation_limit: None,
                },
                None,
            )
            .await
            .expect_err("validate_trade 默认实现必须是错误");
        assert!(matches!(err, AxAgentError::Provider(_)), "validate_trade: {err:?}");

        let err = p
            .execute_trade(
                TradeExecutionRequest {
                    stock_code: "600519".into(),
                    stock_name: "贵州茅台".into(),
                    direction: "buy".into(),
                    price: 1500.0,
                    quantity: 100,
                    trade_date: "2026-11-02".into(),
                    trade_time: "14:30".into(),
                    notes: None,
                    analysis_id: None,
                },
                None,
            )
            .await
            .expect_err("execute_trade 默认实现必须是错误");
        assert!(matches!(err, AxAgentError::Provider(_)), "execute_trade: {err:?}");

        let err = p.compare_trade("trade-1").await.expect_err("compare_trade 默认实现必须是错误");
        assert!(matches!(err, AxAgentError::Provider(_)), "compare_trade: {err:?}");

        let err = p.trading_enabled().await.expect_err("trading_enabled 默认实现必须是错误");
        assert!(matches!(err, AxAgentError::Provider(_)), "trading_enabled: {err:?}");

        // F2 四 op（#322）：缺省实现同样必须显式报错，不得返回空值
        // （空对象会被插件读成「限额全是 0」，那是另一种谎）。
        let err = p
            .get_position_limits(Value::Null)
            .await
            .expect_err("get_position_limits 默认实现必须是错误");
        assert!(matches!(err, AxAgentError::Provider(_)), "get_position_limits: {err:?}");

        let err = p
            .update_portfolio_holding(Value::Null)
            .await
            .expect_err("update_portfolio_holding 默认实现必须是错误");
        assert!(matches!(err, AxAgentError::Provider(_)), "update_portfolio_holding: {err:?}");

        let err = p
            .import_portfolio_from_vlm(Value::Null)
            .await
            .expect_err("import_portfolio_from_vlm 默认实现必须是错误");
        assert!(matches!(err, AxAgentError::Provider(_)), "import_portfolio_from_vlm: {err:?}");

        let err =
            p.run_stress_test(Value::Null).await.expect_err("run_stress_test 默认实现必须是错误");
        assert!(matches!(err, AxAgentError::Provider(_)), "run_stress_test: {err:?}");

        // #323 T2 五命令批迁（子批④）：缺省实现同样必须显式报错，不得返回空值
        // （空对象会被插件读成「0 条策略校验通过」，那是另一种谎）。
        let err = p
            .validate_portfolio_config(Value::Null)
            .await
            .expect_err("validate_portfolio_config 默认实现必须是错误");
        assert!(matches!(err, AxAgentError::Provider(_)), "validate_portfolio_config: {err:?}");
    }
}
