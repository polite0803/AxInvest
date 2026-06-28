//! 数据源健康追踪与 fallback 链增强 (P1.5-3)
//!
//! 借鉴 TradingAgents-AShare 的 "failsafe fallback 链" 设计:
//! - 追踪每个 vendor 的连续失败次数
//! - 在连续失败后自动降级该 vendor
//! - 记录 fallback 路径供调试
//! - 定时恢复降级的 vendor

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::warn;

/// 单个 vendor 的健康状态
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VendorHealth {
    /// vendor 名称
    pub name: String,
    /// 连续失败次数
    pub consecutive_failures: u32,
    /// 总成功次数
    pub total_successes: u64,
    /// 总失败次数
    pub total_failures: u64,
    /// 当前状态
    pub status: VendorStatus,
    /// 最后错误信息
    pub last_error: Option<String>,
    /// 最后成功时间 (epoch ms)
    pub last_success_at: Option<i64>,
    /// 最后失败时间 (epoch ms)
    pub last_failure_at: Option<i64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VendorStatus {
    /// 正常
    Healthy,
    /// 已降级（连续失败超过阈值，暂不使用）
    Degraded,
    /// 完全禁用（手动或严重错误）
    Disabled,
}

/// 健康检查配置
#[derive(Debug, Clone)]
pub struct VendorHealthConfig {
    /// 连续失败多少次后降级
    pub degraded_threshold: u32,
    /// 降级后多久恢复尝试（秒）
    pub recovery_interval_secs: u64,
    /// 是否记录 fallback 路径
    pub track_fallback_path: bool,
}

impl Default for VendorHealthConfig {
    fn default() -> Self {
        Self {
            degraded_threshold: 8, // P1.5-3: 从 3 提升到 8 — 荐股批量请求下避免 burst 误降级
            recovery_interval_secs: 300, // 5 分钟
            track_fallback_path: true,
        }
    }
}

/// 健康追踪器
pub struct VendorHealthTracker {
    vendors: Arc<RwLock<HashMap<String, VendorHealth>>>,
    config: VendorHealthConfig,
    /// fallback 路径记录（环形缓冲区）
    fallback_log: Arc<RwLock<Vec<FallbackRecord>>>,
}

/// 一次 fallback 记录
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FallbackRecord {
    /// 数据类 (quote, klines, news...)
    pub data_type: String,
    /// 股票代码
    pub stock_code: String,
    /// 首选 vendor
    pub primary_vendor: String,
    /// 实际使用的 vendor
    pub used_vendor: String,
    /// 失败原因
    pub reason: String,
    /// 时间戳
    pub timestamp: i64,
}

impl VendorHealthTracker {
    pub fn new(config: VendorHealthConfig) -> Self {
        Self {
            vendors: Arc::new(RwLock::new(HashMap::new())),
            config,
            fallback_log: Arc::new(RwLock::new(Vec::with_capacity(100))),
        }
    }

    /// 记录成功
    pub async fn record_success(&self, name: &str) {
        let mut vendors = self.vendors.write().await;
        let now = chrono::Utc::now().timestamp_millis();
        let entry = vendors
            .entry(name.to_string())
            .or_insert_with(|| VendorHealth {
                name: name.to_string(),
                consecutive_failures: 0,
                total_successes: 0,
                total_failures: 0,
                status: VendorStatus::Healthy,
                last_error: None,
                last_success_at: None,
                last_failure_at: None,
            });
        entry.consecutive_failures = 0;
        entry.total_successes += 1;
        entry.status = VendorStatus::Healthy;
        entry.last_success_at = Some(now);
    }

    /// 记录失败，返回是否已降级
    pub async fn record_failure(&self, name: &str, error: &str) -> bool {
        let mut vendors = self.vendors.write().await;
        let now = chrono::Utc::now().timestamp_millis();
        let entry = vendors
            .entry(name.to_string())
            .or_insert_with(|| VendorHealth {
                name: name.to_string(),
                consecutive_failures: 0,
                total_successes: 0,
                total_failures: 0,
                status: VendorStatus::Healthy,
                last_error: None,
                last_success_at: None,
                last_failure_at: None,
            });
        entry.consecutive_failures += 1;
        entry.total_failures += 1;
        entry.last_error = Some(error.to_string());
        entry.last_failure_at = Some(now);

        if entry.consecutive_failures >= self.config.degraded_threshold {
            entry.status = VendorStatus::Degraded;
            warn!(
                "[VendorHealth] {} 连续失败 {} 次，已降级。最后错误: {}",
                name, entry.consecutive_failures, error
            );
            true
        } else {
            false
        }
    }

    /// 获取可用 vendor 列表（按优先级排序，排除降级的）
    pub async fn get_healthy_vendors(&self, names: &[String]) -> Vec<String> {
        let vendors = self.vendors.read().await;
        let now = chrono::Utc::now().timestamp_millis();
        let recovery_ms = (self.config.recovery_interval_secs * 1000) as i64;

        names
            .iter()
            .filter(|name| {
                if let Some(health) = vendors.get(name.as_str()) {
                    match health.status {
                        VendorStatus::Healthy => true,
                        VendorStatus::Degraded => {
                            // 超过恢复间隔后自动恢复尝试
                            if let Some(last_fail) = health.last_failure_at {
                                now - last_fail >= recovery_ms
                            } else {
                                false
                            }
                        },
                        VendorStatus::Disabled => false,
                    }
                } else {
                    true // 未知 vendor 默认可用
                }
            })
            .cloned()
            .collect()
    }

    /// 获取所有 vendor 的健康状态
    pub async fn get_all_health(&self) -> Vec<VendorHealth> {
        let vendors = self.vendors.read().await;
        let mut result: Vec<VendorHealth> = vendors.values().cloned().collect();
        result.sort_by(|a, b| a.name.cmp(&b.name));
        result
    }

    /// 记录 fallback 路径
    pub async fn record_fallback(
        &self,
        data_type: &str,
        stock_code: &str,
        primary: &str,
        used: &str,
        reason: &str,
    ) {
        if !self.config.track_fallback_path {
            return;
        }
        let mut log = self.fallback_log.write().await;
        if log.len() >= 100 {
            log.remove(0);
        }
        log.push(FallbackRecord {
            data_type: data_type.to_string(),
            stock_code: stock_code.to_string(),
            primary_vendor: primary.to_string(),
            used_vendor: used.to_string(),
            reason: reason.to_string(),
            timestamp: chrono::Utc::now().timestamp_millis(),
        });
    }

    /// 获取 fallback 日志
    pub async fn get_fallback_log(&self) -> Vec<FallbackRecord> {
        self.fallback_log.read().await.clone()
    }

    /// 获取缺省 vendor 列表（Healthy 优先，再试 Degraded）
    pub async fn try_vendors<'a>(&self, names: &'a [String]) -> Vec<&'a String> {
        let vendors = self.vendors.read().await;
        let now = chrono::Utc::now().timestamp_millis();
        let recovery_ms = (self.config.recovery_interval_secs * 1000) as i64;

        // 先 Healthy, 再 Degraded 超过恢复间隔
        let mut healthy = Vec::new();
        let mut recoverable = Vec::new();

        for name in names {
            match vendors.get(name.as_str()) {
                Some(h) if h.status == VendorStatus::Healthy => healthy.push(name),
                Some(h) if h.status == VendorStatus::Degraded => {
                    if let Some(last_fail) = h.last_failure_at {
                        if now - last_fail >= recovery_ms {
                            recoverable.push(name);
                        }
                    }
                },
                None => healthy.push(name),
                _ => {},
            }
        }

        healthy.extend(recoverable);
        healthy
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn healthy_vendor_available() {
        let tracker = VendorHealthTracker::new(VendorHealthConfig::default());
        tracker.record_success("tencent").await;
        let healthy = tracker
            .get_healthy_vendors(&["tencent".to_string(), "sina".to_string()])
            .await;
        assert!(healthy.contains(&"tencent".to_string()));
    }

    #[tokio::test]
    async fn degraded_vendor_filtered() {
        let tracker = VendorHealthTracker::new(VendorHealthConfig::default());
        for _ in 0..8 {
            tracker.record_failure("bad-vendor", "timeout").await;
        }
        let healthy = tracker
            .get_healthy_vendors(&["bad-vendor".to_string()])
            .await;
        assert!(healthy.is_empty(), "降级后的 vendor 不应出现在健康列表");
    }

    #[tokio::test]
    async fn recovery_after_interval() {
        let config = VendorHealthConfig {
            degraded_threshold: 2,
            recovery_interval_secs: 0, // 立即恢复
            track_fallback_path: true,
        };
        let tracker = VendorHealthTracker::new(config);
        for _ in 0..2 {
            tracker.record_failure("flaky", "error").await;
        }
        // 立即恢复
        let healthy = tracker.get_healthy_vendors(&["flaky".to_string()]).await;
        assert_eq!(healthy.len(), 1, "recovery_interval_secs=0 应立即恢复");
    }

    #[tokio::test]
    async fn tracks_fallback_path() {
        let tracker = VendorHealthTracker::new(VendorHealthConfig::default());
        tracker
            .record_fallback("quotes", "000001", "tencent", "sina", "timeout")
            .await;
        let log = tracker.get_fallback_log().await;
        assert_eq!(log.len(), 1);
        assert_eq!(log[0].primary_vendor, "tencent");
    }

    #[tokio::test]
    async fn success_resets_failures() {
        let tracker = VendorHealthTracker::new(VendorHealthConfig::default());
        tracker.record_failure("flaky", "error").await;
        tracker.record_success("flaky").await;
        let health = tracker.get_all_health().await;
        let h = health.iter().find(|h| h.name == "flaky").unwrap();
        assert_eq!(h.consecutive_failures, 0);
        assert_eq!(h.status, VendorStatus::Healthy);
    }
}
