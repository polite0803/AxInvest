// SPDX-License-Identifier: AGPL-3.0-only

//! ORSet: Observed-Remove Set
//!
//! 基于标记的集合 CRDT，添加操作带唯一标签，
//! 只有当元素**已观察到的所有添加标签**都被移除时，元素才真正从集合中删除。
//!
//! # 语义（add-wins / observed-remove）
//!
//! 维护两个集合：
//! - `A(e)` = `elements[e]` —— 所有被应用过的 `add` 标签
//! - `R(e)` = `removed[e]` —— 所有被 `remove` 记录过的标签（墓碑）
//!
//! 元素存活 ⟺ `A(e) \ R(e) ≠ ∅`。`remove(e)` **不删除** `A(e)` 中的标签，
//! 而是把「当前已观察到的标签」复制进 `R(e)`（这正是 observed-remove 的含义：
//! 只能移除自己看到过的那些标签）。
//!
//! # 为什么墓碑不可省（历史缺陷）
//!
//! 本实现曾让 `remove` 直接 `elements[e].clear()`。该写法在单机顺序语义下看似等价，
//! 但在多副本下**静默失效**：一个空的 `A(e)` 与「从未听说过 e」在结构上完全无法区分，
//! 而 merge 只能取并集，于是承载删除意图的空集**传递不出任何信息** ——
//! A 删除后 `B.merge(A)`，B 仍保留自己的标签并认为元素存在，删除永不收敛。
//!
//! 保留墓碑后 `R(e)` 随 merge 一起传播，删除意图才能抵达对端。
//!
//! # 代价
//!
//! 墓碑只增不减 ⇒ 长期运行内存单调增长。生产环境需要按「最慢副本已同步水位」
//! 做 GC（本 crate 目前未实现 GC，接入前请评估该前提）。

use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

/// ORSet 实现
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ORSet {
    /// 元素 → 已添加标签集合 `A(e)`
    elements: HashMap<String, HashSet<String>>,
    /// 元素 → 已删除标签集合 `R(e)`（墓碑）
    #[serde(default)]
    removed: HashMap<String, HashSet<String>>,
    /// 站点 ID
    site_id: String,
}

impl ORSet {
    /// 创建新的 ORSet
    pub fn new(site_id: String) -> Self {
        Self { elements: HashMap::new(), removed: HashMap::new(), site_id }
    }

    /// 获取当前存活元素集合（`A \ R`）
    pub fn elements(&self) -> HashSet<String> {
        self.elements.keys().filter(|e| self.contains(e.as_str())).cloned().collect()
    }

    /// 检查元素是否存在（`A(e) \ R(e) ≠ ∅`）
    pub fn contains(&self, element: &str) -> bool {
        let Some(tags) = self.elements.get(element) else {
            return false;
        };
        match self.removed.get(element) {
            Some(removed) => tags.iter().any(|tag| !removed.contains(tag)),
            // 无墓碑时直接看是否存在非空标签
            None => tags.iter().any(|tag| !tag.is_empty()),
        }
    }

    /// 添加元素（本地操作）
    ///
    /// 返回新生成的标签。每次调用都是**全新标签**，因此「删除后重新添加」
    /// 产生的是未被墓碑覆盖的新标签，元素正确地重新存活。
    pub fn add(&mut self, element: String) -> String {
        let tag = self.generate_tag();
        self.add_with_tag(element, tag.clone());
        tag
    }

    /// 使用特定标签添加元素
    fn add_with_tag(&mut self, element: String, tag: String) {
        self.elements.entry(element).or_default().insert(tag);
    }

    /// 移除元素（本地操作）
    ///
    /// 把**当前已观察到的全部标签**记入墓碑 `R(e)`，而非清空 `A(e)`。
    /// 保留 `A(e)` 是让删除意图可被 merge 传播的唯一途径（见模块文档）。
    pub fn remove(&mut self, element: &str) {
        let Some(tags) = self.elements.get(element) else {
            return;
        };
        // 先收集，避免与后续对 self.removed 的可变借用冲突
        let observed: Vec<String> = tags.iter().cloned().collect();
        if observed.is_empty() {
            return;
        }
        let removed = self.removed.entry(element.to_string()).or_default();
        for tag in observed {
            removed.insert(tag);
        }
    }

    /// 应用远程添加操作
    pub fn apply_add(&mut self, element: String, tag: String) {
        self.add_with_tag(element, tag);
    }

    /// 应用远程移除操作
    ///
    /// 远程移除同样是**记墓碑**：即便本地尚未见过该标签（乱序到达），
    /// 先记进 `R(e)` 也能保证标签稍后到达时不复活元素。
    pub fn apply_remove(&mut self, element: &str, tag: &str) {
        if tag.is_empty() {
            return;
        }
        self.removed.entry(element.to_string()).or_default().insert(tag.to_string());
    }

    /// 合并另一个 ORSet
    ///
    /// `A` 与 `R` **都取并集** —— 只合并 `A` 而丢弃 `R` 会让墓碑失效，
    /// 删除意图随之丢失（这正是历史缺陷的形态）。
    pub fn merge(&mut self, other: &ORSet) {
        for (element, tags) in &other.elements {
            for tag in tags {
                if !tag.is_empty() {
                    self.add_with_tag(element.clone(), tag.clone());
                }
            }
        }
        for (element, tags) in &other.removed {
            let removed = self.removed.entry(element.clone()).or_default();
            for tag in tags {
                if !tag.is_empty() {
                    removed.insert(tag.clone());
                }
            }
        }
    }

    /// 生成唯一标签
    fn generate_tag(&self) -> String {
        format!("{}-{}", self.site_id, uuid::Uuid::new_v4())
    }

    /// 获取快照
    pub fn snapshot(&self) -> ORSetSnapshot {
        ORSetSnapshot { elements: self.elements.clone(), removed: self.removed.clone() }
    }

    /// 从快照恢复
    pub fn from_snapshot(snapshot: ORSetSnapshot, site_id: String) -> Self {
        Self { elements: snapshot.elements, removed: snapshot.removed, site_id }
    }
}

/// ORSet 快照
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ORSetSnapshot {
    /// 已添加标签集合 `A`
    pub elements: HashMap<String, HashSet<String>>,
    /// 已删除标签集合 `R`（墓碑）
    ///
    /// `serde(default)`：兼容本次修复之前产出的旧快照（其无墓碑字段）。
    /// 旧快照恢复后 `R` 为空，等价于「按已删除的标签未知」处理，
    /// 不会把未删元素误判为已删。
    #[serde(default)]
    pub removed: HashMap<String, HashSet<String>>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_orset_basic() {
        let mut set = ORSet::new("siteA".to_string());
        assert!(!set.contains("item1"));

        let _tag = set.add("item1".to_string());
        assert!(set.contains("item1"));

        set.remove("item1");
        assert!(!set.contains("item1"));
    }

    #[test]
    fn test_orset_concurrent_add() {
        let mut set1 = ORSet::new("siteA".to_string());
        let tag1 = set1.add("item".to_string());

        let mut set2 = ORSet::new("siteB".to_string());
        set2.apply_add("item".to_string(), "tagB".to_string());

        // 合并后元素应该存在（有两个标签）
        set1.merge(&set2);
        assert!(set1.contains("item"));

        // 移除一个标签后元素仍存在
        set1.apply_remove("item", &tag1);
        assert!(set1.contains("item"));

        // 移除所有标签后元素不存在
        set1.apply_remove("item", "tagB");
        assert!(!set1.contains("item"));
    }

    #[test]
    fn test_orset_merge() {
        let mut set1 = ORSet::new("siteA".to_string());
        set1.add("a".to_string());
        set1.add("b".to_string());

        let mut set2 = ORSet::new("siteB".to_string());
        set2.add("b".to_string());
        set2.add("c".to_string());

        set1.merge(&set2);
        let elements = set1.elements();
        assert_eq!(elements.len(), 3);
        assert!(elements.contains("a"));
        assert!(elements.contains("b"));
        assert!(elements.contains("c"));
    }

    // ─── 回归测试：本次修复的两个缺陷（修复前为红） ─────────────────────

    /// 回归：删除必须能通过 merge 传播到对端
    ///
    /// 修复前 `remove` 清空 `elements`，对端 merge 到的是空集，
    /// 无法区分「已删除」与「从未见过」，删除静默失效。
    #[test]
    fn test_remove_propagates_through_merge() {
        let mut a = ORSet::new("siteA".to_string());
        let mut b = ORSet::new("siteB".to_string());

        a.add("x".to_string());
        b.add("x".to_string());

        // 双向同步：双方都观察到彼此的标签
        a.merge(&b);
        b.merge(&a);
        assert!(a.contains("x") && b.contains("x"), "同步后双方都应看到元素");

        // A 删除（此刻 A 已观察到全部标签）
        a.remove("x");
        assert!(!a.contains("x"), "A 本地删除应生效");

        // B 收到 A 状态后必须也认为已删除
        b.merge(&a);
        assert!(!b.contains("x"), "删除信息必须随 merge 传播到对端");
    }

    /// 回归：并发 remove 与 add —— add 胜出（add-wins）
    ///
    /// remove 只能移除**已观察到**的标签，无法覆盖并发的全新标签。
    #[test]
    fn test_concurrent_add_wins_over_remove() {
        let mut a = ORSet::new("siteA".to_string());
        a.add("x".to_string());

        let mut b = ORSet::new("siteB".to_string());
        b.merge(&a); // B 观察到 A 的标签

        a.remove("x"); // 并发：A 删除
        b.add("x".to_string()); // 并发：B 重新加入（新标签）

        a.merge(&b);
        b.merge(&a);
        assert!(a.contains("x"), "add-wins：A 侧应看到元素存活");
        assert!(b.contains("x"), "add-wins：B 侧应看到元素存活");
    }

    /// 回归：删除后重新添加必须真正复活（新标签不被旧墓碑覆盖）
    #[test]
    fn test_re_add_after_remove_revives() {
        let mut set = ORSet::new("siteA".to_string());
        set.add("x".to_string());
        set.remove("x");
        assert!(!set.contains("x"));

        set.add("x".to_string());
        assert!(set.contains("x"), "新标签不应被旧墓碑覆盖");
    }

    /// 回归：重复应用同一个远程 add 不得复活已删元素（乱序/重传下的幂等性）
    #[test]
    fn test_replayed_add_stays_deleted() {
        let mut set = ORSet::new("siteA".to_string());
        let tag = set.add("x".to_string());
        set.remove("x");
        assert!(!set.contains("x"));

        set.apply_add("x".to_string(), tag); // 同一标签重放
        assert!(!set.contains("x"), "已删标签的重放不应复活元素");
    }

    /// 回归：乱序到达 —— 先收到 remove 再收到 add，元素应保持已删除
    #[test]
    fn test_out_of_order_remove_before_add() {
        let mut set = ORSet::new("siteA".to_string());
        // remove 先到（本地还没有该标签）
        set.apply_remove("x", "late-tag");
        // add 后到
        set.apply_add("x".to_string(), "late-tag".to_string());
        assert!(!set.contains("x"), "墓碑先到达时，后到的同标签 add 不应复活元素");
    }

    /// 回归：墓碑必须随快照往返保留，否则重启后删除状态丢失
    #[test]
    fn test_snapshot_roundtrip_preserves_tombstones() {
        let mut set = ORSet::new("siteA".to_string());
        set.add("x".to_string());
        set.add("y".to_string());
        set.remove("x");

        let restored = ORSet::from_snapshot(set.snapshot(), "siteA".to_string());
        assert!(!restored.contains("x"), "墓碑必须随快照保留（x 仍为已删）");
        assert!(restored.contains("y"), "未删元素不应受影响");
    }

    /// 回归：合并必须带上墓碑 —— 只并 A 不并 R 会让删除失效
    #[test]
    fn test_merge_carries_tombstones() {
        let mut a = ORSet::new("siteA".to_string());
        a.add("x".to_string());
        a.remove("x");

        let mut b = ORSet::new("siteB".to_string());
        b.merge(&a);
        assert!(!b.contains("x"), "merge 必须携带墓碑，否则对端会把已删元素当存活");
    }
}
