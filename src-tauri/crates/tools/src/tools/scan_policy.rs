// SPDX-License-Identifier: AGPL-3.0-only

//! 需求发现扫描策略
//!
//! 把 i18n 里早就声明、但此前**没有任何消费方**的四个旋钮真正接到扫描管线上：
//! - `scanConcurrency` → [`ScanPolicy::concurrency`]，并发扫描的平台数上限
//! - `scanRateLimit` → [`ScanPolicy::rate_limit_per_min`]，全局速率上限（次/分钟）
//! - `scanRetryMax` → [`ScanPolicy::retry_max`]，单平台失败重试次数
//! - `scanDeduplicateWindowHours` → [`ScanPolicy::dedup_window_hours`]，去重时间窗口
//!
//! 策略以 JSON 形式持久化在通用设置表（`settings`），键名见
//! [`SCAN_POLICY_SETTING_KEY`]；读取失败或键不存在时回落到 [`ScanPolicy::default`]。

use serde::{Deserialize, Serialize};
use std::time::Duration;

/// 扫描策略在通用设置表中的键名
pub const SCAN_POLICY_SETTING_KEY: &str = "opc.demand.scanPolicy";

/// 需求发现扫描策略
///
/// 所有字段都有默认值且会做范围钳制，避免脏配置把扫描管线上打挂
///（如 `concurrency = 0` 导致永不执行、或 `retry_max` 过大导致长尾阻塞）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ScanPolicy {
    /// 并发扫描的平台数上限（>= 1）
    pub concurrency: usize,
    /// 全局速率上限（次/分钟）；0 表示不限速
    pub rate_limit_per_min: u32,
    /// 单个平台失败后的最大重试次数（不含首次尝试）
    pub retry_max: u32,
    /// 重试退避基数（毫秒）；第 n 次重试等待 `base * 2^(n-1)`
    pub retry_backoff_ms: u64,
    /// 单平台单次请求超时（秒）
    pub timeout_secs: u64,
    /// 去重时间窗口（小时）
    ///
    /// 命中 `(platform, source_url)` 的既有线索：
    /// - 在窗口内 → 跳过（判定为同一轮需求的重复曝光）
    /// - 超出窗口 → **刷新**该行（更新评分与 `updated_at`），而不是插入新行
    ///   —— 去重唯一索引是 `(platform, source_url)`，插入必然冲突
    pub dedup_window_hours: u32,
    /// 单次扫描保留的线索数上限（防御扫描器异常返回海量数据）
    pub max_leads_per_scan: usize,
}

impl Default for ScanPolicy {
    fn default() -> Self {
        Self {
            concurrency: 4,
            rate_limit_per_min: 60,
            retry_max: 2,
            retry_backoff_ms: 500,
            timeout_secs: 15,
            dedup_window_hours: 24 * 7,
            max_leads_per_scan: 200,
        }
    }
}

impl ScanPolicy {
    /// 并发数（已钳到 >= 1）
    pub fn concurrency(&self) -> usize {
        self.concurrency.max(1)
    }

    /// 单次请求超时（已钳到 >= 1s）
    pub fn timeout(&self) -> Duration {
        Duration::from_secs(self.timeout_secs.max(1))
    }

    /// 相邻两次扫描请求的最小间隔；`None` 表示不限速
    pub fn min_request_interval(&self) -> Option<Duration> {
        if self.rate_limit_per_min == 0 {
            return None;
        }
        let per_request = 60.0 / f64::from(self.rate_limit_per_min);
        Some(Duration::from_secs_f64(per_request.max(0.0)))
    }

    /// 第 `attempt` 次重试的退避时长（`attempt` 从 1 开始）
    pub fn retry_backoff(&self, attempt: u32) -> Duration {
        let shift = attempt.saturating_sub(1).min(10);
        let factor = 1u64 << shift;
        Duration::from_millis(self.retry_backoff_ms.saturating_mul(factor))
    }

    /// 去重窗口（秒）；`None` 表示永久去重
    pub fn dedup_window_secs(&self) -> Option<i64> {
        if self.dedup_window_hours == 0 {
            return None;
        }
        Some(i64::from(self.dedup_window_hours) * 3600)
    }

    /// 从 JSON 反序列化；失败或字段越界时回落到默认值并做范围钳制
    pub fn from_json(json: &str) -> Self {
        match serde_json::from_str::<Self>(json) {
            Ok(p) => p.normalized(),
            Err(e) => {
                tracing::warn!(error = %e, "[ScanPolicy] 解析失败，回落到默认策略");
                Self::default()
            },
        }
    }

    /// 序列化为 JSON（用于写回设置表）
    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        // normalized() 按值接收 self，而这里只有 &self，必须先 clone
        serde_json::to_string(&self.clone().normalized())
    }

    /// 范围钳制：并发/超时/上限设下限，重试次数与退避设上限
    pub fn normalized(mut self) -> Self {
        self.concurrency = self.concurrency.clamp(1, 32);
        self.rate_limit_per_min = self.rate_limit_per_min.min(6000);
        self.retry_max = self.retry_max.min(5);
        self.retry_backoff_ms = self.retry_backoff_ms.clamp(0, 30_000);
        self.timeout_secs = self.timeout_secs.clamp(1, 120);
        self.dedup_window_hours = self.dedup_window_hours.min(24 * 365);
        self.max_leads_per_scan = self.max_leads_per_scan.clamp(1, 5_000);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_policy_is_valid() {
        let p = ScanPolicy::default();
        assert!(p.concurrency() >= 1);
        assert!(p.timeout() >= Duration::from_secs(1));
        assert!(p.retry_max <= 5);
    }

    #[test]
    fn zero_values_are_clamped() {
        let p = ScanPolicy {
            concurrency: 0,
            rate_limit_per_min: 0,
            retry_max: 0,
            retry_backoff_ms: 0,
            timeout_secs: 0,
            dedup_window_hours: 0,
            max_leads_per_scan: 0,
        }
        .normalized();

        assert_eq!(p.concurrency(), 1);
        assert_eq!(p.timeout(), Duration::from_secs(1));
        assert_eq!(p.min_request_interval(), None, "rate_limit=0 表示不限速");
        assert_eq!(p.max_leads_per_scan, 1);
        assert_eq!(p.dedup_window_secs(), None, "窗口为 0 表示永久去重");
    }

    #[test]
    fn rate_limit_maps_to_interval() {
        let p = ScanPolicy { rate_limit_per_min: 120, ..Default::default() }.normalized();
        assert_eq!(p.min_request_interval(), Some(Duration::from_millis(500)));
    }

    #[test]
    fn retry_backoff_grows_exponentially() {
        let p = ScanPolicy { retry_backoff_ms: 500, ..Default::default() };
        assert_eq!(p.retry_backoff(1), Duration::from_millis(500));
        assert_eq!(p.retry_backoff(2), Duration::from_millis(1000));
        assert_eq!(p.retry_backoff(3), Duration::from_millis(2000));
    }

    #[test]
    fn dedup_window_converts_to_seconds() {
        let p = ScanPolicy { dedup_window_hours: 24, ..Default::default() };
        assert_eq!(p.dedup_window_secs(), Some(86_400));
    }

    #[test]
    fn json_roundtrip_preserves_fields() {
        let p = ScanPolicy { concurrency: 8, retry_max: 3, ..Default::default() };
        let json = p.to_json().expect("序列化应成功");
        let back = ScanPolicy::from_json(&json);
        assert_eq!(back.concurrency, 8);
        assert_eq!(back.retry_max, 3);
    }

    #[test]
    fn malformed_json_falls_back_to_default() {
        assert_eq!(ScanPolicy::from_json("{not json"), ScanPolicy::default());
        // 缺字段走 serde(default)，不应整体回落
        assert_eq!(ScanPolicy::from_json("{}"), ScanPolicy::default());
    }

    #[test]
    fn out_of_range_json_is_clamped() {
        let p = ScanPolicy::from_json(r#"{"concurrency":9999,"retryMax":99,"timeoutSecs":99999}"#);
        assert_eq!(p.concurrency, 32);
        assert_eq!(p.retry_max, 5);
        assert_eq!(p.timeout_secs, 120);
    }
}
