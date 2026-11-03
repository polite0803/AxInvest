// SPDX-License-Identifier: AGPL-3.0-only

//! `trading.engine` 接缝的**宿主侧实现**（wiring 层，宿主独持 `TradingEngine`）。
//!
//! 契约在 `axagent-core/crates/harness/src/trading_engine.rs`（已冻结）：六个 op 里
//! `get_positions` / `validate_trade` / `execute_trade` 会触发行情取数 ⇒ 收 `as_of`；
//! `get_trades` / `compare_trade` / `trading_enabled` 是纯 DB 读，不收 `as_of`。
//! ORM 类型（`trades::Model`）**不过线** —— 统一经 `axagent_analysis_engine::trading::trade_view`
//! 转 `TradeView`（该函数是唯一转换点，本文件不重写字段映射）。
//!
//! 同键查重的 SQL 收在 `axagent_dao::repo::trade`（分层门禁规则 1：本文件基线 0，
//! 不得出现 ORM 字面量）；本文件只保留「命中 ⇒ 按 `duplicate` 拒绝」的编排与错误映射。
//!
//! ## 六 op 的宿主侧处理
//!
//! | op | 串行锁 | `as_of` | 计时 | 去重 | 错误映射 |
//! |---|---|---|---|---|---|
//! | `get_positions` | 无（读锁跨 N+1，与既有 4 个命令点同形态） | 有 | **有**（供决定是否进 `SEAM_OP_TIMEOUT_OVERRIDES`） | 无 | `Err(String)` ⇒ `Provider` |
//! | `get_trades` | 无 | 无 | 无 | 无 | 同上 |
//! | `validate_trade` | 无 | 有 | 无 | 无 | 无 `Err`（校验不过 ⇒ `valid=false`） |
//! | `execute_trade` | **按 `stock_code`** | 有 | 无 | **同键 6 字段** | 同键命中 ⇒ `duplicate`；引擎内部拒绝 ⇒ outcome |
//! | `compare_trade` | 无 | 无 | 无 | 无 | 查无 ⇒ `Ok(None)` |
//! | `trading_enabled` | 无 | 无 | 无 | 无 | settings 读失败 ⇒ `false`（引擎口径） |
//!
//! ## 刻意不做
//!
//! - **不读引擎的 `enabled` 字段**：它是死字段（恒 false、全仓无写入点），开关的
//!   唯一权威是 settings 的 `trading_enabled`（`trading.rs::is_trading_enabled`）。
//! - **不加超时 override**：`SEAM_OP_TIMEOUT_OVERRIDES` 的条目要求「文件:行」实测出处，
//!   且测试会对未进表的 op 断言 `None`。先埋 `get_positions` 的计时，拿到分布再议。
//! - **不持 `Arc<AppState>`**：`AppState` 按值交给 Tauri，构造期尚未成型，拿不到 `'static`。
//!   本 provider 只持引擎与 DB 的 `Arc`（与 `jobs.store` / `workflow.run` 同构）。

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use async_trait::async_trait;
use axagent_analysis_engine::trading::{TradingEngine, is_trading_enabled, trade_view};
use axagent_dao::DatabaseConnection;
use axagent_dao::repo::trade as trade_dao;
use axagent_harness::{
    AsOfContext, AxAgentError, PositionSummary, TradeExecutionOutcome, TradeExecutionRequest,
    TradePredictionComparison, TradeValidation, TradeValidationRequest, TradeView,
    TradingEngineProvider,
};
use tokio::sync::{Mutex, RwLock};

/// 交易历史单次返回上限（分派层缺省 50；这里再夹一层，防插件一次拉爆内存）。
const MAX_TRADES_LIMIT: u32 = 500;

/// 重复提交的拒绝原因（契约 `TradeExecutionOutcome::reason` 的既有取值）。
const REASON_DUPLICATE: &str = "duplicate";

/// 落库故障的原因字面量（引擎产出；宿主侧映射为 `Err`，不当作业务拒绝）。
const REASON_WRITE_FAILED: &str = "write_failed";

/// `trading.engine` 的宿主实现。
pub struct HostTradingEngine {
    /// 与 `AppState.trading_engine` **同一个 `Arc`**（克隆持有，不反向引用 AppState）。
    engine: Arc<RwLock<TradingEngine>>,
    /// 查重 / 开关读取用；与 AppState 的 DB 是同一连接池。
    db: Arc<DatabaseConnection>,
    /// 按 `stock_code` 的串行锁表。
    ///
    /// 为什么需要：引擎的 `upsert_position` 是 SELECT→UPDATE 的读-改-写
    /// （`trading.rs` 的 `upsert_position`），两个并发下单可能各读到旧值、各写回，
    /// 后写覆盖先写 ⇒ 丢一次加仓/减仓。SQLite 的单写入序列化兜不住 Postgres 后端
    /// （`init/state.rs` 有 Postgres 分支）。锁粒度选「每股一把」而不是「全表一把」：
    /// 不同股票的下单（含各自 N+1 取行情）可以并行，同股严格串行。
    execute_locks: Mutex<HashMap<String, Arc<Mutex<()>>>>,
}

impl HostTradingEngine {
    pub fn new(engine: Arc<RwLock<TradingEngine>>, db: Arc<DatabaseConnection>) -> Self {
        Self { engine, db, execute_locks: Mutex::new(HashMap::new()) }
    }

    /// 取某只股票的串行锁（首次使用时建）。
    ///
    /// 外层 `Mutex` 只护「取/建表项」这一步，出手即释放；真正的串行区是返回的
    /// per-stock `Mutex`。表项**不回收**（每只股票一个 `Arc<Mutex<()>>`，几百只持仓
    /// 也只是几百个空锁，回收反而要引入「谁在持有」的引用计数判断）。
    async fn stock_lock(&self, stock_code: &str) -> Arc<Mutex<()>> {
        let mut locks = self.execute_locks.lock().await;
        locks.entry(stock_code.to_string()).or_insert_with(|| Arc::new(Mutex::new(()))).clone()
    }
}

#[async_trait]
impl TradingEngineProvider for HostTradingEngine {
    async fn get_positions(
        &self,
        as_of: Option<AsOfContext>,
    ) -> Result<Vec<PositionSummary>, AxAgentError> {
        // as_of 会被 move 进作用域，先留一份用于日志（-1/-1 的「是否回放」很影响耗时分布）
        let replay = as_of.is_some();
        let started = Instant::now();
        let positions = {
            // 读锁跨全部 N+1 await —— 与既有 4 个命令点（get_trade_positions 等）同形态；
            // 全仓无 `write()`，暂无写者被饿。缩短锁窗口属后续项，不在本批改。
            let engine = self.engine.read().await;
            axagent_astock_data::as_of::with_optional_asof(as_of, engine.get_positions()).await
        }
        .map_err(AxAgentError::Provider)?;

        let elapsed = started.elapsed();
        let count = positions.len();
        // 计时是**决策依据**：`SEAM_OP_TIMEOUT_OVERRIDES` 要求「文件:行」实测出处，
        // 没有分布就进表 ⇒ 契约测试直接红。拿到数据前不加 override。
        tracing::info!(
            holdings = count,
            elapsed_ms = elapsed.as_millis() as u64,
            per_stock_ms = if count > 0 {
                (elapsed.as_millis() as u64) / count as u64
            } else {
                0
            },
            replay,
            "[trading.engine] get_positions 计时（总耗时 / 单只均耗）"
        );
        Ok(positions)
    }

    async fn get_trades(
        &self,
        stock_code: Option<&str>,
        limit: u32,
    ) -> Result<Vec<TradeView>, AxAgentError> {
        let limit = clamp_trades_limit(limit);
        let engine = self.engine.read().await;
        let models = engine.get_trades(stock_code, limit).await.map_err(AxAgentError::Provider)?;
        Ok(models.iter().map(trade_view).collect())
    }

    async fn validate_trade(
        &self,
        request: TradeValidationRequest,
        as_of: Option<AsOfContext>,
    ) -> Result<TradeValidation, AxAgentError> {
        let engine = self.engine.read().await;
        // 校验本身**不返回 Err**：判据不满足走 `valid = false` + `errors`。
        Ok(axagent_astock_data::as_of::with_optional_asof(as_of, async {
            match request.price_deviation_limit {
                // 插件显式给阈值 ⇒ 接上引擎里一直零调用的参数化入口
                Some(limit) => {
                    engine
                        .validate_trade_with_config(
                            &request.stock_code,
                            &request.direction,
                            request.quantity,
                            request.price,
                            limit,
                        )
                        .await
                },
                // 缺省 ⇒ 引擎自己的 5.0（不要在宿主侧再写一份缺省值）
                None => {
                    engine
                        .validate_trade(
                            &request.stock_code,
                            &request.direction,
                            request.quantity,
                            request.price,
                        )
                        .await
                },
            }
        })
        .await)
    }

    async fn execute_trade(
        &self,
        request: TradeExecutionRequest,
        as_of: Option<AsOfContext>,
    ) -> Result<TradeExecutionOutcome, AxAgentError> {
        // ① 按股票串行：挡住「两笔同股并发下单各读到旧持仓、后写覆盖先写」的丢更新
        let lock = self.stock_lock(&request.stock_code).await;
        let _guard = lock.lock().await;

        // ② 同键查重（在锁内查：否则两次并发同单会同时判「不重复」）
        if is_duplicate_trade(self.db.as_ref(), &request).await? {
            tracing::info!(
                stock_code = %request.stock_code,
                direction = %request.direction,
                trade_date = %request.trade_date,
                trade_time = %request.trade_time,
                "[trading.engine] execute_trade 命中同键记录，按 duplicate 拒绝"
            );
            return Ok(TradeExecutionOutcome {
                accepted: false,
                reason: Some(REASON_DUPLICATE.to_string()),
                trade: None,
                errors: Vec::new(),
                warnings: Vec::new(),
            });
        }

        // ③ 走引擎的结构化下单（开关闸 + 只校验一次 + 落库都在引擎侧）
        let engine = self.engine.read().await;
        let outcome = axagent_astock_data::as_of::with_optional_asof(
            as_of,
            engine.execute_trade_outcome(request),
        )
        .await;
        // `write_failed` 是落库故障（基础设施面），不是业务拒绝：按契约回 Err，
        // 免得上层把它当「用户操作无效」而不重试。
        if outcome.reason.as_deref() == Some(REASON_WRITE_FAILED) {
            return Err(AxAgentError::Provider(format!(
                "交易落库失败：{}",
                outcome.errors.join("; ")
            )));
        }
        Ok(outcome)
    }

    async fn compare_trade(
        &self,
        trade_id: &str,
    ) -> Result<Option<TradePredictionComparison>, AxAgentError> {
        let engine = self.engine.read().await;
        engine.compare_trade_vs_prediction_opt(trade_id).await.map_err(AxAgentError::Provider)
    }

    async fn trading_enabled(&self) -> Result<bool, AxAgentError> {
        // 唯一口径：settings 的 `trading_enabled`（引擎的 `enabled` 是死字段，禁读禁写）
        Ok(is_trading_enabled(self.db.as_ref()).await)
    }
}

/// 历史条数钳位：下限 1（0 行没有语义）、上限 [`MAX_TRADES_LIMIT`]。
fn clamp_trades_limit(limit: u32) -> u32 {
    limit.clamp(1, MAX_TRADES_LIMIT)
}

/// 同键查重：键 = 股票 + 方向 + 日期 + 时间 + 价格 + 数量（**六字段**）。
///
/// 与 CSV 导入的 `is_duplicate_trade`（`axagent-quant/crates/analysis-engine/src/trade_import.rs:431-447`）
/// 口径逐字一致：两条写入路径必须认同一套「同一笔交易」。否则同一笔单子 CSV 导一次、
/// 插件再录一次就能重复入账。`trades` 表无唯一约束（加约束要写 migration，AGENTS 禁区 10），
/// 故这里只能做**尽力而为**的查重：窗口期内两笔真重复的、并发绕过该检查的，仍会入库。
async fn is_duplicate_trade(
    db: &DatabaseConnection,
    request: &TradeExecutionRequest,
) -> Result<bool, AxAgentError> {
    trade_dao::exists_same_key(
        db,
        &request.stock_code,
        &request.direction,
        &request.trade_date,
        &request.trade_time,
        request.price,
        request.quantity,
    )
    .await
    .map_err(|e| AxAgentError::Provider(format!("交易查重失败（trades 查询）: {e}")))
}

#[cfg(test)]
mod tests {
    use super::{MAX_TRADES_LIMIT, clamp_trades_limit};

    #[test]
    fn trades_limit_is_clamped() {
        // 分派层给缺省 50；宿主再夹一层：下限 1（0 行没有语义）、上限 500
        assert_eq!(clamp_trades_limit(50), 50);
        assert_eq!(clamp_trades_limit(0), 1);
        assert_eq!(clamp_trades_limit(10_000), MAX_TRADES_LIMIT);
    }
}
