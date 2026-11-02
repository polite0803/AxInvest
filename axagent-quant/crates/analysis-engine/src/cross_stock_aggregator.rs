// SPDX-License-Identifier: AGPL-3.0-only

//! 跨股票信号聚合器（P3-1）
//!
//! 在 `signals.rs`（单股信号检测）和 `portfolio_monitor.rs`（持仓指标聚合）
//! 之间填补"信号聚合层"架构缺口。
//!
//! ## 核心抽象
//!
//! - [`SignalType`] — 统一信号类型枚举（替代 signals.rs 中分散的 String 字面量）
//! - [`StockSignal`] — 单股信号载体（stock_code / signal_type / direction / strength / source / timestamp）
//! - [`PortfolioSignal`] — 聚合后的组合级信号（多股同方向 → 组合级卖出/买入提示）
//! - [`CrossStockSignalAggregator`] — 运行时聚合器，接收单股信号流，按时间窗口聚合
//!
//! ## 聚合规则
//!
//! 1. **同向聚集规则**：在 `window_secs` 时间窗内，同方向（多头/空头）信号数 ≥ `min_signal_count`
//!    时触发组合级信号（避免单股噪声误判）
//! 2. **强度加权**：组合信号强度 = Σ(单股强度) / max_possible_strength，归一化到 [0, 1]
//! 3. **去重冷却**：同类型组合信号在 `cooldown_secs` 内不重复触发
//!
//! ## 使用场景
//!
//! - `RealtimeMonitor` 在每次轮询后将单股告警喂给聚合器
//! - 聚合器返回 `PortfolioSignal`，由 `RealtimeMonitor` 经 `MonitorEventEmitter`
//!   以 `IpcEventName::PortfolioSignal` 推给前端（G-6）
//!
//! ⚠ 这里**没有** `broadcast` 订阅口：`feed()` 的返回值就是唯一出口，进程内那份
//! `signal_tx` 自建自听、零消费者，已随 G-6 一起退役（留着一个没人听的频道，
//! 与本轮在阶段 D 退役的那批假旋钮同形）。

use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

// 聚合器的六个 DTO 与两个枚举已**上收 harness**（`stock.monitor` 接缝的 `aggregator_*` 三 op
// 直接把它们的 camelCase 投影当线上契约，见 harness `stock_monitor.rs`）。
// 此处只 re-export，不重定义（AGENTS 禁区 12）；`SignalType::direction/label`、
// `SignalDirection::from_i8`、`AggregatorConfig` 的 `Default` 与 clamp 区间常量随类型一起上收
// —— Rust 不允许在别的 crate 给 foreign 类型写 inherent impl。
pub use axagent_harness::{
    AggregatorConfig, PortfolioSignal, PortfolioSnapshot, SignalDirection, SignalType, StockSignal,
};

/// 组合快照回调类型别名（避免 clippy::type_complexity 警告）
pub type SnapshotProvider = Arc<dyn Fn() -> Option<PortfolioSnapshot> + Send + Sync>;

/// 跨股票信号聚合器。
///
/// 内部维护时间窗口内的信号缓冲区，每次喂入新信号时尝试触发组合级信号。
/// 通过 `broadcast::Sender<PortfolioSignal>` 推给订阅者。
///
/// # 线程安全
///
/// 内部状态用 `RwLock` 保护，可被 `Arc<RealtimeMonitor>` 等多 owner 共享。
pub struct CrossStockSignalAggregator {
    config: RwLock<AggregatorConfig>,
    /// 时间窗内的信号缓冲区（key = stock_code, value = 该股票最新信号）
    /// 注：同一只股票在窗内只保留最新信号，避免单股高频刷屏
    signal_buffer: RwLock<HashMap<String, StockSignal>>,
    /// 各方向最后触发组合信号的时间戳（用于冷却去重）
    last_triggered: RwLock<HashMap<SignalDirection, i64>>,
    /// 可选的组合快照回调（每次触发聚合时调用以获取当前持仓上下文）
    snapshot_provider: RwLock<Option<SnapshotProvider>>,
}

impl CrossStockSignalAggregator {
    pub fn new(config: AggregatorConfig) -> Self {
        Self {
            config: RwLock::new(config),
            signal_buffer: RwLock::new(HashMap::new()),
            last_triggered: RwLock::new(HashMap::new()),
            snapshot_provider: RwLock::new(None),
        }
    }

    /// 更新聚合器配置（运行时热更新）
    pub async fn set_config(&self, config: AggregatorConfig) {
        *self.config.write().await = config;
    }

    /// 查询当前配置
    pub async fn config(&self) -> AggregatorConfig {
        self.config.read().await.clone()
    }

    /// 设置组合快照回调（由 wiring 层注入，从 AppState 读取当前持仓指标）
    pub async fn set_snapshot_provider(&self, provider: SnapshotProvider) {
        *self.snapshot_provider.write().await = Some(provider);
    }

    /// 喂入单股信号并尝试触发组合级信号。
    ///
    /// 调用时机：`RealtimeMonitor::check_alerts` 检测到告警后调用此方法。
    /// 返回 `Some(PortfolioSignal)` 表示触发了组合级信号，`None` 表示未触发。
    pub async fn feed(&self, signal: StockSignal) -> Option<PortfolioSignal> {
        let cfg = self.config.read().await.clone();
        if signal.strength < cfg.min_strength {
            return None;
        }

        // 1) 写入缓冲区（覆盖同股票旧信号）
        {
            let mut buf = self.signal_buffer.write().await;
            buf.insert(signal.stock_code.clone(), signal.clone());
        }

        // 2) 清理过期信号（窗口外）
        let now = signal.timestamp;
        let window_start = now - cfg.window_secs;
        {
            let mut buf = self.signal_buffer.write().await;
            buf.retain(|_, s| s.timestamp >= window_start);
        }

        // 3) 按方向统计
        let buf = self.signal_buffer.read().await;
        let mut bull_stocks: Vec<&StockSignal> = Vec::new();
        let mut bear_stocks: Vec<&StockSignal> = Vec::new();
        for s in buf.values() {
            match SignalDirection::from_i8(s.signal_type.direction()) {
                SignalDirection::Bullish => bull_stocks.push(s),
                SignalDirection::Bearish => bear_stocks.push(s),
                SignalDirection::Neutral => {},
            }
        }

        // 4) 选择信号数较多的一方作为候选方向
        let (candidate_dir, candidate_stocks): (SignalDirection, Vec<&StockSignal>) =
            if bull_stocks.len() >= bear_stocks.len() {
                (SignalDirection::Bullish, bull_stocks)
            } else {
                (SignalDirection::Bearish, bear_stocks)
            };

        if candidate_stocks.len() < cfg.min_signal_count {
            return None;
        }

        // 5) 冷却检查
        {
            let last = self.last_triggered.read().await;
            if let Some(&last_ts) = last.get(&candidate_dir)
                && now - last_ts < cfg.cooldown_secs
            {
                return None;
            }
        }

        // 6) 计算组合信号强度（归一化 [0, 1]）
        // 强度公式：Σ(单股强度) / min_signal_count，上限 1.0
        let total_strength: f64 = candidate_stocks.iter().map(|s| s.strength).sum();
        let portfolio_strength = (total_strength / cfg.min_signal_count as f64).min(1.0);

        // 7) 找多数派信号类型（同向信号中的众数）
        let mut type_counts: HashMap<SignalType, usize> = HashMap::new();
        for s in &candidate_stocks {
            *type_counts.entry(s.signal_type).or_insert(0) += 1;
        }
        let dominant_signal = type_counts
            .into_iter()
            .max_by_key(|(_, c)| *c)
            .map(|(t, _)| t)
            .unwrap_or(candidate_stocks[0].signal_type);

        // 8) 调用 snapshot provider 获取持仓上下文
        let snapshot = {
            let provider = self.snapshot_provider.read().await;
            provider.as_ref().and_then(|p| p())
        };

        // 9) 构造建议操作文案
        let suggested_action = build_suggested_action(
            candidate_dir,
            candidate_stocks.len(),
            portfolio_strength,
            &snapshot,
        );

        let portfolio_signal = PortfolioSignal {
            direction: candidate_dir,
            stocks: candidate_stocks.iter().map(|s| s.stock_code.clone()).collect(),
            dominant_signal,
            strength: portfolio_strength,
            snapshot: snapshot.clone(),
            suggested_action,
            timestamp: now,
        };

        // 10) 更新冷却时间
        {
            let mut last = self.last_triggered.write().await;
            last.insert(candidate_dir, now);
        }

        // 唯一出口就是返回值：调用方（`RealtimeMonitor`）拿到后按
        // `IpcEventName::PortfolioSignal` emit 给前端。这里不再另开一条进程内广播。
        Some(portfolio_signal)
    }

    /// 清空信号缓冲区（用于交易日重置或测试）
    pub async fn clear(&self) {
        self.signal_buffer.write().await.clear();
        self.last_triggered.write().await.clear();
    }

    /// 查询当前缓冲区快照（调试用）
    pub async fn buffer_snapshot(&self) -> Vec<StockSignal> {
        self.signal_buffer.read().await.values().cloned().collect()
    }
}

/// 构造组合信号的操作建议文案
fn build_suggested_action(
    direction: SignalDirection,
    count: usize,
    strength: f64,
    snapshot: &Option<PortfolioSnapshot>,
) -> String {
    let dir_label = match direction {
        SignalDirection::Bullish => "多头聚集",
        SignalDirection::Bearish => "空头聚集",
        SignalDirection::Neutral => "中性",
    };

    let strength_label = if strength >= 0.7 {
        "强"
    } else if strength >= 0.4 {
        "中"
    } else {
        "弱"
    };

    let risk_note = snapshot
        .as_ref()
        .and_then(|s| {
            s.top_concentration_pct.map(|pct| {
                if pct > 0.4 {
                    format!("（头部集中度 {:.0}% 已超 40% 警戒线）", pct * 100.0)
                } else {
                    String::new()
                }
            })
        })
        .unwrap_or_default();

    match direction {
        SignalDirection::Bearish => format!(
            "组合出现{}{}信号（{} 只股票同向，强度 {}），建议评估减仓或对冲{}",
            strength_label, dir_label, count, strength_label, risk_note
        ),
        SignalDirection::Bullish => format!(
            "组合出现{}{}信号（{} 只股票同向，强度 {}），可考虑分批加仓或布局相关 ETF{}",
            strength_label, dir_label, count, strength_label, risk_note
        ),
        SignalDirection::Neutral => format!("组合信号中性（{} 只），保持观察", count),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_signal(code: &str, signal_type: SignalType, strength: f64, ts: i64) -> StockSignal {
        StockSignal {
            stock_code: code.to_string(),
            stock_name: format!("股票{}", code),
            signal_type,
            strength,
            source: "test".to_string(),
            timestamp: ts,
            current_price: Some(10.0),
            change_pct: Some(-2.0),
        }
    }

    #[tokio::test]
    async fn test_aggregator_triggers_after_min_count() {
        let agg = CrossStockSignalAggregator::new(AggregatorConfig {
            window_secs: 300,
            min_signal_count: 3,
            cooldown_secs: 600,
            min_strength: 0.3,
        });
        let now = 1700000000;

        // 喂入 2 个空头信号，不应触发
        assert!(agg.feed(make_signal("000001", SignalType::StopLossHit, 0.6, now)).await.is_none());
        assert!(
            agg.feed(make_signal("000002", SignalType::SupportBreak, 0.5, now + 10))
                .await
                .is_none()
        );

        // 第 3 个空头信号应触发组合信号
        let result = agg.feed(make_signal("000003", SignalType::DeathCross, 0.7, now + 20)).await;
        assert!(result.is_some(), "第三个同向信号应触发组合信号");
        let ps = result.unwrap();
        assert_eq!(ps.direction, SignalDirection::Bearish);
        assert_eq!(ps.stocks.len(), 3);
        assert!(ps.strength > 0.0);
    }

    #[tokio::test]
    async fn test_aggregator_respects_cooldown() {
        let agg = CrossStockSignalAggregator::new(AggregatorConfig {
            window_secs: 300,
            min_signal_count: 2,
            cooldown_secs: 600,
            min_strength: 0.3,
        });
        let now = 1700000000;

        // 触发一次
        let r1 = agg.feed(make_signal("000001", SignalType::StopLossHit, 0.6, now)).await;
        assert!(r1.is_none()); // 只 1 个信号
        let r2 = agg.feed(make_signal("000002", SignalType::StopLossHit, 0.6, now + 10)).await;
        assert!(r2.is_some(), "2 个信号应触发");

        // 立即再喂入新信号，应被冷却拦截
        let r3 = agg.feed(make_signal("000003", SignalType::StopLossHit, 0.6, now + 20)).await;
        assert!(r3.is_none(), "冷却期内不应再次触发");
    }

    #[tokio::test]
    async fn test_low_strength_filtered() {
        let agg = CrossStockSignalAggregator::new(AggregatorConfig {
            window_secs: 300,
            min_signal_count: 2,
            cooldown_secs: 600,
            min_strength: 0.5,
        });
        let now = 1700000000;

        // 强度低于阈值，不应进入缓冲区
        let r1 = agg.feed(make_signal("000001", SignalType::StopLossHit, 0.3, now)).await;
        assert!(r1.is_none());
        let r2 = agg.feed(make_signal("000002", SignalType::StopLossHit, 0.3, now + 10)).await;
        assert!(r2.is_none(), "低强度信号不应参与聚合");
    }
}
