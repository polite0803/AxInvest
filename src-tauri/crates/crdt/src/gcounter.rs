// SPDX-License-Identifier: AGPL-3.0-only

//! GCounter: Grow-only Counter
//!
//! 增长计数器 CRDT，每个站点只能递增自己的计数器值，
//! 通过合并各站点的值实现分布式计数。

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// GCounter 实现
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GCounter {
    /// 各站点的计数器值
    counters: HashMap<String, u64>,
    /// 站点 ID
    site_id: String,
}

impl GCounter {
    /// 创建新的 GCounter
    pub fn new(site_id: String) -> Self {
        Self { counters: HashMap::new(), site_id }
    }

    /// 获取计数器总值
    pub fn value(&self) -> u64 {
        self.counters.values().sum()
    }

    /// 获取指定站点的值
    pub fn get(&self, site_id: &str) -> u64 {
        self.counters.get(site_id).copied().unwrap_or(0)
    }

    /// 递增计数器（本地操作）
    pub fn increment(&mut self, amount: u64) {
        if amount > 0 {
            let counter = self.counters.entry(self.site_id.clone()).or_insert(0);
            *counter += amount;
        }
    }

    /// 并入远端某站点的**水位**（取 `max`，幂等）—— state-based 语义。
    ///
    /// 与 [`merge`](Self::merge) 的**单站点特化完全等价**（同一 `max` 规则），
    /// 用于「逐条吸收远端向量」的场景，语义与 `VersionVector::observe` 对齐。
    ///
    /// ⚠ **不是 op-based 的「增量累加」**：重复并入同一 `(site_id, value)` 不会翻倍，
    /// 但也**不会**把 `value` 累加到已有值上。要按增量推进请用 [`increment`](Self::increment)
    /// 并自行做 op 去重（取 max 幂等、`+=` 不幂等）。
    ///
    /// 原名 `apply_remote_increment` —— 「increment」措辞与实际行为（取 max）不符，
    /// 会误导调用方以为可以累加，故改名以消除该陷阱。
    pub fn observe_remote(&mut self, site_id: &str, value: u64) {
        if value > 0 {
            let counter = self.counters.entry(site_id.to_string()).or_insert(0);
            *counter = (*counter).max(value);
        }
    }

    /// 合并另一个 GCounter
    pub fn merge(&mut self, other: &GCounter) {
        for (site_id, value) in &other.counters {
            let current = self.counters.entry(site_id.clone()).or_insert(0);
            *current = (*current).max(*value);
        }
    }

    /// 获取快照
    pub fn snapshot(&self) -> GCounterSnapshot {
        GCounterSnapshot { counters: self.counters.clone() }
    }

    /// 从快照恢复
    pub fn from_snapshot(snapshot: GCounterSnapshot, site_id: String) -> Self {
        Self { counters: snapshot.counters, site_id }
    }
}

/// GCounter 快照
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GCounterSnapshot {
    pub counters: HashMap<String, u64>,
}

impl PartialEq for GCounter {
    /// 比较**逐站点计数向量**，而非总值。
    ///
    /// GCounter 的状态是 `{站点 → 计数}` 映射。仅比较总值会让状态不同的副本被判等：
    /// `{siteA: 5}` 与 `{siteB: 5}` 总值同为 5，但两者并未收敛 ——
    /// 这类假阳性会污染「本地与远端状态是否一致」的收敛判断。
    fn eq(&self, other: &Self) -> bool {
        self.counters == other.counters
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_gcounter_basic() {
        let mut counter = GCounter::new("siteA".to_string());
        assert_eq!(counter.value(), 0);

        counter.increment(5);
        assert_eq!(counter.value(), 5);
        assert_eq!(counter.get("siteA"), 5);
    }

    #[test]
    fn test_gcounter_merge() {
        let mut counter1 = GCounter::new("siteA".to_string());
        counter1.increment(10);

        let mut counter2 = GCounter::new("siteB".to_string());
        counter2.increment(20);

        counter1.merge(&counter2);
        assert_eq!(counter1.value(), 30);
        assert_eq!(counter1.get("siteA"), 10);
        assert_eq!(counter1.get("siteB"), 20);
    }

    #[test]
    fn test_gcounter_concurrent() {
        let mut counter1 = GCounter::new("siteA".to_string());
        counter1.increment(5);

        let mut counter2 = GCounter::new("siteA".to_string());
        counter2.increment(10);

        // 同一站点的合并取最大值
        counter1.merge(&counter2);
        assert_eq!(counter1.get("siteA"), 10);
        assert_eq!(counter1.value(), 10);
    }

    #[test]
    fn test_gcounter_snapshot() {
        let mut counter = GCounter::new("siteA".to_string());
        counter.increment(42);

        let snapshot = counter.snapshot();
        let restored = GCounter::from_snapshot(snapshot, "siteA".to_string());

        assert_eq!(restored.value(), 42);
    }

    // ─── 回归测试：相等性必须比较状态而非总值（修复前为红） ───────────

    /// 回归：`PartialEq` 曾比较 `value()` 而非逐站点向量
    ///
    /// 修复前 `{siteA: 5}` 与 `{siteB: 5}` 会因总值相同被判等，
    /// 使「两副本是否收敛」的判断产生假阳性。
    #[test]
    fn test_equality_compares_state_not_total() {
        let mut a = GCounter::new("siteA".to_string());
        a.increment(5);

        let mut b = GCounter::new("siteB".to_string());
        b.increment(5);

        assert_eq!(a.value(), b.value(), "前提：两者总值确实巧合相同");
        assert_ne!(a, b, "状态不同（{{A:5}} vs {{B:5}}）的 GCounter 不应判等");

        // 真正收敛后必须判等
        let mut c = GCounter::new("siteC".to_string());
        c.merge(&a);
        assert_eq!(a, c, "合并后状态一致应判等");
    }

    /// 回归：相等性必须自反且对称（`PartialEq` 契约）
    #[test]
    fn test_equality_is_reflexive_and_symmetric() {
        let mut a = GCounter::new("siteA".to_string());
        a.increment(3);
        let mut b = GCounter::new("siteB".to_string());
        b.merge(&a);

        assert_eq!(a, a, "相等性必须自反");
        assert_eq!(a, b);
        assert_eq!(b, a, "相等性必须对称");
    }

    // ─── 回归测试：`observe_remote` 必须是「取水位」而非「累加增量」 ──────────

    /// 回归：原 `apply_remote_increment` 名实不符 —— 名为 increment、实为取 max。
    /// 本测试把**实际语义**锁死：幂等、不回退、可推进。
    #[test]
    fn test_observe_remote_is_idempotent_and_never_regresses() {
        let mut c = GCounter::new("siteA".to_string());

        c.observe_remote("siteB", 7);
        assert_eq!(c.get("siteB"), 7);

        // 重复并入同一水位 ⇒ 不翻倍（这条会把「误以为是 += 」的实现判红）
        c.observe_remote("siteB", 7);
        assert_eq!(c.get("siteB"), 7, "重复并入同一水位不得累加");

        // 旧水位不得回退
        c.observe_remote("siteB", 3);
        assert_eq!(c.get("siteB"), 7, "更低的水位不得回退已有值");

        // 新水位正常推进
        c.observe_remote("siteB", 11);
        assert_eq!(c.get("siteB"), 11);
    }

    /// 回归：`observe_remote` 必须与「单站点 `merge`」等价（它是 merge 的特化，不是另一套语义）
    #[test]
    fn test_observe_remote_equivalent_to_single_site_merge() {
        let mut remote = GCounter::new("siteB".to_string());
        remote.increment(9); // ⇒ {siteB: 9}

        let mut via_observe = GCounter::new("siteA".to_string());
        via_observe.observe_remote("siteB", 9);

        let mut via_merge = GCounter::new("siteA".to_string());
        via_merge.merge(&remote);

        assert_eq!(via_observe, via_merge, "observe_remote 应与单站点 merge 结果一致");
    }
}
