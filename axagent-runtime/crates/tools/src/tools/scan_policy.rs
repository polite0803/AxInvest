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
//!
//! 2026-11-11：类型本体上收 `axagent_harness::types::opc_demand` —— `opc.store` 接缝要把
//! 它跨进程传给插件 worker，而 harness 不能反向依赖本 crate。此处保留同名 re-export，
//! 既有调用方的 `axagent_tools::tools::scan_policy::ScanPolicy` 路径不变。

pub use axagent_harness::types::opc_demand::{SCAN_POLICY_SETTING_KEY, ScanPolicy};
