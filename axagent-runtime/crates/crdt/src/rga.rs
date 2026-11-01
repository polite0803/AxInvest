// SPDX-License-Identifier: AGPL-3.0-only

//! RGA: Replicated Growable Array
//!
//! 可复制增长数组，用于有序数据的 CRDT 实现。
//! 每个元素有唯一 ID 和位置信息（基于左右邻居 ID），
//! 通过比较位置关系确定元素顺序。

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// RGA 数组条目
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RGAEntry {
    /// 条目唯一 ID
    pub id: String,
    /// 条目值
    pub value: Value,
    /// 左邻居 ID（None 表示头部）
    pub left_id: Option<String>,
    /// 右邻居 ID（None 表示尾部）
    pub right_id: Option<String>,
    /// 创建站点 ID
    pub site_id: String,
    /// 创建逻辑时钟
    pub clock: u64,
    /// 是否已删除
    pub deleted: bool,
}

impl RGAEntry {
    /// 创建新条目
    pub fn new(value: Value, site_id: String, clock: u64) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            value,
            left_id: None,
            right_id: None,
            site_id,
            clock,
            deleted: false,
        }
    }
}

/// RGA 实现
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RGA {
    /// 所有条目（包括已删除的）
    entries: Vec<RGAEntry>,
    /// 头部条目 ID
    head_id: Option<String>,
    /// 站点 ID
    site_id: String,
    /// 逻辑时钟
    logical_clock: u64,
}

impl RGA {
    /// 创建新的 RGA
    pub fn new(site_id: String) -> Self {
        Self { entries: Vec::new(), head_id: None, site_id, logical_clock: 0 }
    }

    /// 获取可见元素列表
    pub fn to_vec(&self) -> Vec<&RGAEntry> {
        let mut result = Vec::new();
        let mut current_id = self.head_id.clone();

        while let Some(id) = current_id {
            if let Some(entry) = self.find_entry(&id) {
                if !entry.deleted {
                    result.push(entry);
                }
                current_id = entry.right_id.clone();
            } else {
                break;
            }
        }
        result
    }

    /// 获取值列表
    pub fn values(&self) -> Vec<&Value> {
        self.to_vec().iter().map(|e| &e.value).collect()
    }

    /// 获取长度
    pub fn len(&self) -> usize {
        self.to_vec().len()
    }

    /// 检查是否为空
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// 在指定位置插入（本地操作）
    pub fn insert(&mut self, index: usize, value: Value) -> String {
        self.logical_clock += 1;

        let (left_id, right_id) = self.find_position(index);
        let mut entry = RGAEntry::new(value, self.site_id.clone(), self.logical_clock);
        entry.left_id = left_id.clone();
        entry.right_id = right_id.clone();

        // 更新邻居链接
        if let Some(rid) = right_id {
            if let Some(right_entry) = self.find_entry_mut(&rid) {
                right_entry.left_id = Some(entry.id.clone());
            }
        } else {
            // 插入到末尾
            if self.head_id.is_none() {
                self.head_id = Some(entry.id.clone());
            }
        }

        if let Some(lid) = left_id {
            if let Some(left_entry) = self.find_entry_mut(&lid) {
                left_entry.right_id = Some(entry.id.clone());
            }
        } else {
            // 插入到头部
            self.head_id = Some(entry.id.clone());
        }

        let entry_id = entry.id.clone();
        self.entries.push(entry);
        entry_id
    }

    /// 删除指定位置的元素
    pub fn remove(&mut self, index: usize) -> bool {
        let visible = self.to_vec();
        if index >= visible.len() {
            return false;
        }

        let entry_id = visible[index].id.clone();
        self.mark_deleted(&entry_id)
    }

    /// 标记条目为已删除（墓碑化）
    ///
    /// # 为什么墓碑必须保留左右指针
    ///
    /// `left_id` / `right_id` 是远端并发插入时**定位锚点的唯一依据**（见
    /// [`Self::find_position_for_merge`]）。若删除时清除指针对并重连邻居，
    /// 远端「在已删条目旁插入」的操作会锚回一个已失去位置的墓碑，
    /// 新条目虽存在于 `entries` 却从可见链表断裂 —— `to_vec` 遍历不到它，
    /// 表现为**元素静默丢失**（本地能查到、序列化出去就没了）。
    ///
    /// 正确做法：墓碑**留在链上**（指针不动），由 [`Self::to_vec`] 遍历时跳过
    /// `deleted` 条目。这样删除只影响可见性，不影响定位结构。
    fn mark_deleted(&mut self, entry_id: &str) -> bool {
        let Some(entry) = self.entries.iter_mut().find(|e| e.id == entry_id) else {
            return false;
        };
        if entry.deleted {
            return false;
        }
        entry.deleted = true;
        true
    }

    /// 应用远程插入操作
    pub fn apply_insert(
        &mut self,
        entry: RGAEntry,
        left_id: Option<String>,
        right_id: Option<String>,
    ) {
        let mut new_entry = entry;
        new_entry.left_id = left_id;
        new_entry.right_id = right_id;

        // 更新邻居链接
        if let Some(rid) = &new_entry.right_id
            && let Some(right_entry) = self.find_entry_mut(rid)
        {
            right_entry.left_id = Some(new_entry.id.clone());
        }

        if let Some(lid) = &new_entry.left_id {
            if let Some(left_entry) = self.find_entry_mut(lid) {
                left_entry.right_id = Some(new_entry.id.clone());
            }
        } else {
            // 插入到头部
            self.head_id = Some(new_entry.id.clone());
        }

        self.entries.push(new_entry);
    }

    /// 应用远程删除操作
    pub fn apply_delete(&mut self, entry_id: &str) -> bool {
        self.mark_deleted(entry_id)
    }

    /// 查找位置（返回应该插入的左、右邻居 ID）
    fn find_position(&self, index: usize) -> (Option<String>, Option<String>) {
        if self.head_id.is_none() || index == 0 {
            // 找到第一个可见条目
            let mut current_id = self.head_id.clone();
            while let Some(id) = current_id {
                if let Some(entry) = self.find_entry(&id) {
                    if !entry.deleted {
                        return (None, Some(id));
                    }
                    current_id = entry.right_id.clone();
                } else {
                    break;
                }
            }
            return (None, None);
        }

        let mut count = 0;
        let mut current_id = self.head_id.clone();
        let mut prev_visible_id: Option<String> = None;

        while let Some(id) = current_id {
            if let Some(entry) = self.find_entry(&id) {
                if !entry.deleted {
                    if count == index {
                        return (prev_visible_id, Some(id));
                    }
                    count += 1;
                    prev_visible_id = Some(id);
                }
                current_id = entry.right_id.clone();
            } else {
                break;
            }
        }

        // 到达末尾
        (prev_visible_id, None)
    }

    /// 查找条目
    fn find_entry(&self, id: &str) -> Option<&RGAEntry> {
        self.entries.iter().find(|e| e.id == id)
    }

    /// 查找可变条目
    fn find_entry_mut(&mut self, id: &str) -> Option<&mut RGAEntry> {
        self.entries.iter_mut().find(|e| e.id == id)
    }

    /// 合并另一个 RGA
    pub fn merge(&mut self, other: &RGA) {
        for entry in &other.entries {
            if self.find_entry(&entry.id).is_none() {
                if entry.deleted {
                    let new_entry = entry.clone();
                    self.entries.push(new_entry);
                } else {
                    let pos = self.find_position_for_merge(entry);
                    self.apply_insert(entry.clone(), pos.0, pos.1);
                }
            } else if entry.deleted {
                self.mark_deleted(&entry.id);
            }
        }
    }

    /// 为合并操作找到合适的位置
    fn find_position_for_merge(&self, entry: &RGAEntry) -> (Option<String>, Option<String>) {
        // 使用 entry 的 left_id 和 right_id
        let mut left_id = entry.left_id.clone();
        let mut right_id = entry.right_id.clone();

        // 如果 left_id 存在但找不到，尝试查找左邻居
        if let Some(lid) = &left_id
            && self.find_entry(lid).is_none()
        {
            left_id = None;
        }

        // 如果 right_id 存在但找不到，尝试查找右邻居
        if let Some(rid) = &right_id
            && self.find_entry(rid).is_none()
        {
            right_id = None;
        }

        // 如果两个邻居都找不到，比较 clock 值决定位置
        if left_id.is_none() && right_id.is_none() {
            // 找到合适的位置
            let mut current_id = self.head_id.clone();
            let mut prev_id = None;

            while let Some(id) = current_id {
                if let Some(existing) = self.find_entry(&id) {
                    if !existing.deleted {
                        // 比较逻辑时钟
                        if entry.clock < existing.clock
                            || (entry.clock == existing.clock && entry.site_id < existing.site_id)
                        {
                            return (prev_id, Some(id));
                        }
                        prev_id = Some(id);
                    }
                    current_id = existing.right_id.clone();
                } else {
                    break;
                }
            }

            // 插入到末尾
            (prev_id, None)
        } else {
            (left_id, right_id)
        }
    }

    /// 获取快照
    pub fn snapshot(&self) -> RGA {
        self.clone()
    }

    /// 从快照恢复
    pub fn from_snapshot(snapshot: RGA, site_id: String) -> Self {
        Self {
            entries: snapshot.entries,
            head_id: snapshot.head_id,
            site_id,
            logical_clock: snapshot.logical_clock,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_rga_basic() {
        let mut rga = RGA::new("siteA".to_string());
        assert!(rga.is_empty());

        rga.insert(0, json!("first"));
        rga.insert(1, json!("second"));
        rga.insert(2, json!("third"));

        assert_eq!(rga.len(), 3);
        assert_eq!(rga.values()[0], &json!("first"));
        assert_eq!(rga.values()[1], &json!("second"));
        assert_eq!(rga.values()[2], &json!("third"));
    }

    #[test]
    fn test_rga_insert_middle() {
        let mut rga = RGA::new("siteA".to_string());
        rga.insert(0, json!("a"));
        rga.insert(1, json!("c"));
        rga.insert(1, json!("b"));

        assert_eq!(rga.len(), 3);
        let values = rga.values();
        assert_eq!(*values[0], json!("a"));
        assert_eq!(*values[1], json!("b"));
        assert_eq!(*values[2], json!("c"));
    }

    #[test]
    fn test_rga_remove() {
        let mut rga = RGA::new("siteA".to_string());
        rga.insert(0, json!("a"));
        rga.insert(1, json!("b"));
        rga.insert(2, json!("c"));

        assert!(rga.remove(1));
        assert_eq!(rga.len(), 2);
        let values = rga.values();
        assert_eq!(*values[0], json!("a"));
        assert_eq!(*values[1], json!("c"));
    }

    #[test]
    fn test_rga_concurrent_insert() {
        let mut rga1 = RGA::new("siteA".to_string());
        rga1.insert(0, json!("fromA"));

        let mut rga2 = RGA::new("siteB".to_string());
        rga2.insert(0, json!("fromB"));

        // 合并两个 RGA
        rga1.merge(&rga2);

        assert_eq!(rga1.len(), 2);
        let values = rga1.values();
        // 两个元素都应该存在
        assert!(values.contains(&&json!("fromA")));
        assert!(values.contains(&&json!("fromB")));
    }

    // ─── 回归测试：墓碑必须保留邻接指针（修复前为红） ───────────────────

    /// 回归：远端「在已删条目旁插入」的元素必须可见
    ///
    /// 修复前 `mark_deleted` 会把已删条目的 `left_id` / `right_id` 置 `None` 并重连邻居，
    /// 于是远端插入时锚回一个**已失去位置的墓碑** —— 新条目仍存在于 `entries`，
    /// 却从可见链表断裂（`to_vec` 遍历不到），表现为元素静默丢失。
    #[test]
    fn test_merge_insert_adjacent_to_tombstone() {
        let mut a = RGA::new("siteA".to_string());
        a.insert(0, json!("a"));
        a.insert(1, json!("b"));
        a.insert(2, json!("c"));

        // B 复制 A 的初始状态，随后在 b 与 c 之间插入 x
        let mut b = RGA::new("siteB".to_string());
        b.merge(&a);
        b.insert(2, json!("x"));

        // A 删除 b（此时 x 尚未到达 A）
        assert!(a.remove(1));
        assert_eq!(a.len(), 2, "A 删除后应剩两个可见元素");

        // 双向合并必须收敛到同一顺序，且 x 可见
        a.merge(&b);
        b.merge(&a);

        let expected = vec![json!("a"), json!("x"), json!("c")];
        let a_vals: Vec<Value> = a.values().into_iter().cloned().collect();
        let b_vals: Vec<Value> = b.values().into_iter().cloned().collect();

        assert_eq!(a_vals, expected, "A 侧应看到 [a,x,c]，实际 {a_vals:?}");
        assert_eq!(b_vals, expected, "B 侧应看到 [a,x,c]，实际 {b_vals:?}");
    }

    /// 回归：连续删除多个中间条目后，链表仍必须可达
    #[test]
    fn test_multiple_tombstones_keep_chain_intact() {
        let mut a = RGA::new("siteA".to_string());
        a.insert(0, json!("a"));
        a.insert(1, json!("b"));
        a.insert(2, json!("c"));
        a.insert(3, json!("d"));

        assert!(a.remove(1)); // 删 b
        assert!(a.remove(1)); // 删 c（此时它是 index 1）

        let vals: Vec<Value> = a.values().into_iter().cloned().collect();
        assert_eq!(vals, vec![json!("a"), json!("d")], "连续删除后链表必须仍可达，实际 {vals:?}");
    }

    /// 回归：删除头部条目后，遍历必须从下一个可见条目继续
    #[test]
    fn test_delete_head_keeps_traversal_alive() {
        let mut a = RGA::new("siteA".to_string());
        a.insert(0, json!("a"));
        a.insert(1, json!("b"));
        a.insert(2, json!("c"));

        assert!(a.remove(0)); // 删头部

        let vals: Vec<Value> = a.values().into_iter().cloned().collect();
        assert_eq!(vals, vec![json!("b"), json!("c")], "删除头部后不应截断后续元素，实际 {vals:?}");

        // 删除头部后继续插入，新元素仍应可见
        a.insert(0, json!("z"));
        let vals: Vec<Value> = a.values().into_iter().cloned().collect();
        assert_eq!(vals, vec![json!("z"), json!("b"), json!("c")], "实际 {vals:?}");
    }
}
