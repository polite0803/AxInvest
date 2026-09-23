// SPDX-License-Identifier: AGPL-3.0-only

//! 版本向量实现
//!
//! 版本向量用于跟踪每个副本的操作历史，
//! 用于检测并发冲突和确定操作的因果顺序。
//!
//! # 权威定义来源
//!
//! 版本向量的权威定义在 `axagent-harness`（AGENTS.md 铁律: 共享类型权威在 harness，
//! 本仓库禁止在非 foundation 层另立同名类型）。本模块不再重复实现，改为 re-export：
//!
//! - [`VersionVector`] 本体 → `axagent_harness::device_sync::VersionVector`
//! - [`VersionVectorEntry`]（条目）→ `axagent_harness::device_sync::VersionVectorEntry`
//!
//! 历史上一段存在于本 crate 的完整 `VersionVector` 实现（含 `HashMap` 字段与
//! `from_entries`/`observe`/`merge` 等方法）已按铁律 4「下沉 + `pub use`」收敛到 harness，
//! 此处保持对外 API 不变（`axagent_crdt::VersionVector` 仍可用），但指向 harness 权威类型。

/// 版本向量（harness 权威定义）
pub use axagent_harness::device_sync::VersionVector;
/// 版本向量条目（harness 权威定义）
pub use axagent_harness::device_sync::VersionVectorEntry;
/// 兼容别名：该类型在本 crate 的历史 API 中名为 `VVEntry`
pub use axagent_harness::device_sync::VersionVectorEntry as VVEntry;

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
