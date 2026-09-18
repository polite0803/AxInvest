// SPDX-License-Identifier: AGPL-3.0-only

//! Louvain DTOs — pure types migrated from dao.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

const LOW_COHESION_THRESHOLD: f64 = 0.15;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LouvainResult {
    pub communities: HashMap<String, i32>,
    pub cohesion_scores: HashMap<i32, f64>,
    pub community_sizes: HashMap<i32, usize>,
    pub top_nodes: HashMap<i32, String>,
    pub modularity: f64,
    pub num_communities: usize,
    pub color_palette: Vec<String>,
    /// **实体侧**（知识图谱）子图单独跑一次 Louvain 的结果：`entity:<id>` → 社区 id（2026-09-18）。
    ///
    /// 为什么要单列而不是并进 `communities`：
    /// ① 两者的键空间本来就不重叠（实体 id 带 `entity:` 前缀），但**值域会重合** ——
    ///    这是两次**独立运行**，各自的社区 id 都从 0 起算。直接并进同一张映射，
    ///    「笔记社区 7」与「实体社区 7」在数值上就分不开了：不同社区数会被低估，
    ///    归并阶段（前端 `mergeCommunitiesTopologically`）会把两个拓扑无关的社区
    ///    当成一个簇强制合并，桶内边被丢弃 ⇒ 结构被抹掉。
    ///    故消费方**必须**先错开命名空间（前端 `mergeEntityCommunities`）。
    /// ② `communities` 的既有消费方（社区面板：cohesion / sizes / topNodes / 调色板）
    ///    描述的是**笔记图**的分区，把实体混进去会让面板语义跟着变 —— 那不是本次要改的事。
    ///
    /// `None` = 未算（该 wiki 没绑知识库 / 实体为空 / 后端旧缓存）。
    /// 反序列化上：`Option` 字段缺失时 serde 默认就是 `None`，故旧缓存 JSON 仍可解析
    /// （否则 `wiki_graph_cache::get_cached_graph` 会走「反序列化失败 ⇒ 清缓存」那条路，
    /// 把笔记侧算好的社区也一起丢掉）。
    #[serde(default)]
    pub entity_communities: Option<HashMap<String, i32>>,
}

impl LouvainResult {
    pub fn default_palette() -> Vec<String> {
        vec![
            "#4C72B0".to_string(),
            "#DD8452".to_string(),
            "#55A868".to_string(),
            "#C44E52".to_string(),
            "#8172B3".to_string(),
            "#937860".to_string(),
            "#DA8BC3".to_string(),
            "#8C8C8C".to_string(),
            "#CCB974".to_string(),
            "#64B5CD".to_string(),
            "#E18B6C".to_string(),
            "#7AA153".to_string(),
        ]
    }

    pub fn get_color(&self, community_id: i32) -> String {
        let idx = community_id.rem_euclid(self.color_palette.len() as i32) as usize;
        self.color_palette[idx].clone()
    }

    pub fn is_low_cohesion(&self, community_id: i32) -> bool {
        self.cohesion_scores
            .get(&community_id)
            .map(|s| *s < LOW_COHESION_THRESHOLD)
            .unwrap_or(false)
    }

    pub fn get_community_nodes(&self, community_id: i32) -> Vec<String> {
        self.communities
            .iter()
            .filter(|(_, c)| **c == community_id)
            .map(|(n, _)| n.clone())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    //! `entity_communities` 的三条**类型层面看不出来**的契约。
    //!
    //! 为什么这几条必须有：该字段带 `#[serde(default)]`，而 `Option` 在类型上
    //! 「天然允许缺失」—— 于是「去掉 default」「改掉键名」「改成 skip_serializing_if」
    //! 三种改动**都能编译、都能让其余测试全绿**，代价却各自落在运行期的不同地方：
    //! 前两者直接静默毁掉数据（见下），后者只是让前端多走一条等价分支。

    use super::*;

    /// 一份**改版前**的缓存形态：`wiki_graph_cache.communities_json` 里没有 `entityCommunities`。
    ///
    /// ⚠ 定界符必须是 `r##"…"##`：JSON 里有颜色值 `"#000000"`，其中 **`"#` 会提前终止
    /// `r#"…"#` 原始字符串** ⇒ 余下的 `000000"]` 被当成代码，报 `expected ';', found '000000'`。
    /// （实测代价：rustfmt 当场报错拦下；`cargo check` 因该文件尚未被编到而**不会**报。）
    const LEGACY_JSON: &str = r##"{
        "communities": {"n1": 0, "n2": 0, "n3": 7},
        "cohesionScores": {"0": 0.5},
        "communitySizes": {"0": 2, "7": 1},
        "topNodes": {"0": "n1"},
        "modularity": 0.66,
        "numCommunities": 2,
        "colorPalette": ["#000000"]
    }"##;

    /// ★ 这条是 `#[serde(default)]` 存在的**全部理由**：旧缓存 JSON 必须仍能解析。
    ///
    /// 去掉 `default` 的后果不在本文件里，而在 `wiki_graph_cache::get_cached_graph` ——
    /// 它会走「反序列化失败 ⇒ 清缓存」，把**笔记侧**已算好的社区一起丢掉，
    /// 表现为「升级后社区面板集体失效一次」。只断言 `is_none()` 会漏掉
    /// 「整条结果被吃空」这一形态，故下面同时断言其余字段。
    #[test]
    fn legacy_cache_json_parses_and_entity_communities_defaults_to_none() {
        let r: LouvainResult = serde_json::from_str(LEGACY_JSON)
            .expect("旧缓存 JSON 必须仍可解析（这就是 #[serde(default)] 的理由）");
        assert!(r.entity_communities.is_none(), "字段缺失时应默认 None");
        assert_eq!(r.communities.len(), 3, "同一对象里的其余字段不受影响");
        assert_eq!(r.communities.get("n3"), Some(&7));
        assert_eq!(r.num_communities, 2);
    }

    /// ★ 跨语言字符串契约：前端 TS 侧读的键是 `entityCommunities`（camelCase）。
    /// 去掉 `rename_all = "camelCase"` 后后端照常编译、所有测试照常绿，
    /// 前端却**静默**拿不到实体侧社区（回落到锚点兜底，看不出错）。
    #[test]
    fn serialized_key_is_camel_case() {
        let r = sample(Some(HashMap::from([("entity:e1".to_string(), 3)])));
        let v = serde_json::to_value(&r).expect("序列化");
        assert!(v.get("entityCommunities").is_some(), "键必须是 camelCase");
        assert!(v.get("entity_communities").is_none(), "不得出现 snake_case 键");
        assert_eq!(v["entityCommunities"]["entity:e1"], 3);
    }

    /// `None` 未配 `skip_serializing_if` ⇒ 写成 `null`（**键仍在**，不是被省略）。
    /// 前端判定是 `communityResult.entityCommunities ? … : null`，`null` 为 falsy
    /// ⇒ 与「字段缺席」走同一条分支。把该事实钉住：若有人改成 skip 序列化，
    /// 前端行为不变，但这条测试会提醒他「这不是无副作用的重构」。
    #[test]
    fn none_serializes_as_null_not_omitted() {
        let v = serde_json::to_value(sample(None)).expect("序列化");
        assert!(
            v.get("entityCommunities").is_some_and(|x| x.is_null()),
            "None 应写成 null（键保留）"
        );
    }

    /// 往返保真：值与原样的 `entity:` 前缀键都不得被改写。
    #[test]
    fn round_trip_preserves_entity_communities() {
        let payload = HashMap::from([("entity:n1".to_string(), 0), ("entity:n2".to_string(), 41)]);
        let json = serde_json::to_string(&sample(Some(payload.clone()))).expect("序列化");
        let back: LouvainResult = serde_json::from_str(&json).expect("反序列化");
        assert_eq!(back.entity_communities, Some(payload));
    }

    fn sample(entity_communities: Option<HashMap<String, i32>>) -> LouvainResult {
        LouvainResult {
            communities: HashMap::from([("n1".to_string(), 0)]),
            cohesion_scores: HashMap::from([(0, 0.5)]),
            community_sizes: HashMap::from([(0, 1)]),
            top_nodes: HashMap::from([(0, "n1".to_string())]),
            modularity: 0.66,
            num_communities: 1,
            color_palette: LouvainResult::default_palette(),
            entity_communities,
        }
    }
}
