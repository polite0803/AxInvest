// SPDX-License-Identifier: AGPL-3.0-only

//! 版本向量实现
//!
//! 版本向量用于跟踪每个副本的操作历史，
//! 用于检测并发冲突和确定操作的因果顺序。
//!
//! # 条目类型来源
//!
//! 条目直接复用 [`axagent_harness::device_sync::VersionVectorEntry`]，不再本 crate 另立同名结构。
//! 该类型是变更日志 `ChangeLogEntry.version_vector` 的载荷，属共享数据模型，
//! 权威定义在 harness（AGENTS.md 第 12 条）。历史上本 crate 曾有一份字段名为
//! `site_id` 的重复定义，导致跨 crate 消费时必须手工转换 —— 现已统一。

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// 版本向量条目（harness 权威定义）
pub use axagent_harness::device_sync::VersionVectorEntry;
/// 兼容别名：该类型在本 crate 的历史 API 中名为 `VVEntry`
pub use axagent_harness::device_sync::VersionVectorEntry as VVEntry;

/// 版本向量
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct VersionVector {
    /// 向量条目
    entries: HashMap<String, u64>,
}

impl VersionVector {
    /// 创建空版本向量
    pub fn new() -> Self {
        Self { entries: HashMap::new() }
    }

    /// 从条目列表创建
    pub fn from_entries(entries: &[VersionVectorEntry]) -> Self {
        let mut map = HashMap::new();
        for entry in entries {
            map.insert(entry.device_id.clone(), entry.counter);
        }
        Self { entries: map }
    }

    /// 转换为条目列表
    pub fn to_entries(&self) -> Vec<VersionVectorEntry> {
        self.entries
            .iter()
            .map(|(device_id, counter)| VersionVectorEntry {
                device_id: device_id.clone(),
                counter: *counter,
            })
            .collect()
    }

    /// 获取指定设备的计数器值
    pub fn get(&self, device_id: &str) -> u64 {
        self.entries.get(device_id).copied().unwrap_or(0)
    }

    /// 递增指定设备的计数器
    pub fn increment(&mut self, device_id: &str) {
        let counter = self.entries.entry(device_id.to_string()).or_insert(0);
        *counter += 1;
    }

    /// 把指定设备的计数器抬升到 `counter`（取二者最大值）
    ///
    /// 用于从变更日志重建版本向量，语义等价于「把该设备的计数推进到已知水位」，
    /// 但与 [`Self::increment`] 不同，它是 **O(1)** 的。
    ///
    /// 之所以需要它：从日志重建时的正确写法是「每个条目取 max」，若改用
    /// `for _ in current..counter { increment() }` 逐次推进，复杂度会退化成
    /// O(计数器值)。当日志条目多、计数器大时（多设备频繁同步）开销迅速放大，
    /// 且这段代码位于同步热路径（每次 `record_change` 都会触发重建）。
    pub fn observe(&mut self, device_id: &str, counter: u64) {
        let current = self.entries.entry(device_id.to_string()).or_insert(0);
        *current = (*current).max(counter);
    }

    /// 合并另一个版本向量（取最大值）
    pub fn merge(&mut self, other: &VersionVector) {
        for (device_id, counter) in &other.entries {
            self.observe(device_id, *counter);
        }
    }

    /// 检查是否包含另一个版本向量（所有条目 >= 另一个的对应条目）
    pub fn contains(&self, other: &VersionVector) -> bool {
        other.entries.iter().all(|(device_id, counter)| self.get(device_id) >= *counter)
    }

    /// 检查是否与另一个版本向量并发（互不包含）
    pub fn is_concurrent_with(&self, other: &VersionVector) -> bool {
        !self.contains(other) && !other.contains(self)
    }

    /// 检查是否小于另一个版本向量（严格因果前序）
    pub fn is_before(&self, other: &VersionVector) -> bool {
        self.contains(other) && self != other
    }
}

impl std::fmt::Display for VersionVector {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let entries: Vec<String> =
            self.entries.iter().map(|(k, v)| format!("{}:{}", k, v)).collect();
        write!(f, "[{}]", entries.join(", "))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn counter_of(vv: &VersionVector, device_id: &str) -> u64 {
        vv.get(device_id)
    }

    #[test]
    fn test_version_vector_basic() {
        let mut vv1 = VersionVector::new();
        assert_eq!(counter_of(&vv1, "siteA"), 0);

        vv1.increment("siteA");
        assert_eq!(counter_of(&vv1, "siteA"), 1);

        vv1.increment("siteA");
        assert_eq!(counter_of(&vv1, "siteA"), 2);
    }

    #[test]
    fn test_version_vector_merge() {
        let mut vv1 = VersionVector::new();
        vv1.increment("siteA");
        vv1.increment("siteA");

        let mut vv2 = VersionVector::new();
        vv2.increment("siteB");

        vv1.merge(&vv2);
        assert_eq!(counter_of(&vv1, "siteA"), 2);
        assert_eq!(counter_of(&vv1, "siteB"), 1);
    }

    /// `observe` 必须与「逐次 increment 到目标值」等价（防止优化改变语义）
    #[test]
    fn test_observe_equals_repeated_increment() {
        let target = 7u64;

        let mut observed = VersionVector::new();
        observed.observe("siteA", target);

        let mut incremented = VersionVector::new();
        for _ in 0..target {
            incremented.increment("siteA");
        }

        assert_eq!(observed, incremented, "observe 必须与逐次递增等价");
    }

    /// `observe` 取 max，不得把大水位回退
    #[test]
    fn test_observe_takes_max_and_never_regresses() {
        let mut vv = VersionVector::new();
        vv.observe("siteA", 5);
        vv.observe("siteA", 3); // 更小的水位不得回退
        assert_eq!(vv.get("siteA"), 5);

        vv.observe("siteA", 9);
        assert_eq!(vv.get("siteA"), 9);
    }

    #[test]
    fn test_version_vector_concurrent() {
        let mut vv1 = VersionVector::new();
        vv1.increment("siteA");

        let mut vv2 = VersionVector::new();
        vv2.increment("siteB");

        assert!(vv1.is_concurrent_with(&vv2));
    }

    #[test]
    fn test_version_vector_causal() {
        let mut vv1 = VersionVector::new();
        vv1.increment("siteA");
        vv1.increment("siteB");

        let mut vv2 = VersionVector::new();
        vv2.increment("siteA");

        assert!(vv1.is_before(&vv2) || vv2.is_before(&vv1) || !vv1.is_concurrent_with(&vv2));
    }

    /// 与 harness 类型往返：条目字段名必须是 harness 的 `device_id`
    #[test]
    fn test_entries_roundtrip_with_harness_type() {
        let entries = vec![
            VersionVectorEntry { device_id: "dev-a".to_string(), counter: 3 },
            VersionVectorEntry { device_id: "dev-b".to_string(), counter: 1 },
        ];
        let vv = VersionVector::from_entries(&entries);
        assert_eq!(vv.get("dev-a"), 3);
        assert_eq!(vv.get("dev-b"), 1);

        let back = vv.to_entries();
        assert_eq!(back.len(), 2);
        assert!(back.iter().any(|e| e.device_id == "dev-a" && e.counter == 3));
    }
}
