// SPDX-License-Identifier: AGPL-3.0-only

//! Graph DTOs — pure types migrated from dao.
//! Includes graph node/edge structures, relevance scoring, and the LinkGraph type.

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};

use crate::page_type::{PageType, cross_type_affinity, same_type_affinity};

// ── Graph primitives (from dao::repo::note) ──

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphNode {
    pub id: String,
    pub title: String,
    #[serde(rename = "type")]
    pub node_type: String,
    #[serde(default)]
    pub tags: Vec<String>,
    /// 传入/传出链接数（最近新增字段，旧版缓存 JSON 不含此字段）
    #[serde(default)]
    pub link_count: i32,
    #[serde(default)]
    pub backlink_count: i32,
    #[serde(default)]
    pub path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphEdge {
    pub source: String,
    pub target: String,
    /// **渲染类别**（`link` / `reference` / `mapping` …）—— 见
    /// [`crate::knowledge_graph::EDGE_TYPE_DECLS`]。它决定「画成什么样」，
    /// **不**代表这条边是什么关系。
    #[serde(rename = "type")]
    pub edge_type: String,
    /// **本体关系 id**（`has_concept` / `in_industry` / `董事` …）—— 见
    /// [`crate::knowledge_graph::RELATION_DECLS`] 与 `DATA_DRIVEN_COLUMN`。
    ///
    /// # 为什么必须有这个字段（2026-09-14）
    ///
    /// 在它出现之前，`get_knowledge_graph_edges_for_wiki` 把 DB 的 `relation_type`
    /// **读出来又扔掉**，`edge_type` 恒为常量 `"reference"` —— 后果是
    /// **DB 实测 56 个关系类型 / 112937 行全部塌成同一个值**（其中
    /// `lemonhu_knowledge_graph` 一个库就有 74766 条边 / 55 个类型），
    /// 图谱上「有概念」「属于行业」「高管任职」看起来一模一样，
    /// 而信息在**出口**就已丢失，前端无从补救。
    ///
    /// 拆成两个字段而不是把 `edge_type` 改成关系 id：渲染类别与本体身份是两件事，
    /// 合并会让「这个类型没有专属样式」和「这个类型后端不认识」变成同一个信号。
    ///
    /// **兼容性**：非关系边（笔记链接 / 合成边）为 `None`；`skip_serializing_if`
    /// ⇒ 不写进 JSON，旧前端忽略该字段不受影响；`#[serde(default)]` ⇒
    /// **旧版缓存 JSON 仍能解析**（字段缺失即 `None`）。与 `GraphNode::link_count`
    /// 同一套约定 ⇒ **不需要任何数据迁移**。
    #[serde(default, rename = "relationType", skip_serializing_if = "Option::is_none")]
    pub relation_type: Option<String>,
}

impl GraphEdge {
    /// 构造一条**非关系**的结构性边（笔记链接 / 合成边）。
    pub fn structural(
        source: impl Into<String>,
        target: impl Into<String>,
        edge_type: &str,
    ) -> Self {
        Self {
            source: source.into(),
            target: target.into(),
            edge_type: edge_type.to_string(),
            relation_type: None,
        }
    }

    /// 构造一条**知识库实体关系**边：渲染类别 + 真实本体关系 id。
    pub fn relation(
        source: impl Into<String>,
        target: impl Into<String>,
        relation_type: impl Into<String>,
    ) -> Self {
        Self {
            source: source.into(),
            target: target.into(),
            edge_type: "reference".to_string(),
            relation_type: Some(relation_type.into()),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphData {
    pub nodes: Vec<GraphNode>,
    pub edges: Vec<GraphEdge>,
    /// 类型字面量不在 [`PageType`] 词汇表内的节点统计（A2-升级，2026-09-14）。
    ///
    /// **为什么放进 DTO**：A2 把静默降级变成可观测，但此前只在 `LinkGraph` 内部可见
    /// （`unresolved_types()`），UI 拿不到 ⇒ 用户看到「节点少了几十个」却无从知道原因。
    /// 该字段把它送到界面上。
    ///
    /// **兼容性**：`#[serde(default)]` ⇒ **旧版缓存 JSON 仍能解析**（字段缺失即空清单）；
    /// `skip_serializing_if` ⇒ 空清单不写进 JSON，正常图不背这个字段。
    /// 两者合起来使得**不需要任何数据迁移**（与 `GraphNode::link_count` 同一套约定）。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub unresolved_types: Vec<UnresolvedTypeStat>,
    /// **边**侧的同一件事：标签无法被后端解释的边统计（P2-b，2026-09-14）。
    ///
    /// 与 [`Self::unresolved_types`] **完全对称**（同判据形态、同聚合、同样例上限、
    /// 同 serde 约定），差别只在对象是边、判据走
    /// [`crate::knowledge_graph::uninterpreted_edge_label`]。
    ///
    /// ⚠ **它是绊线，不是仪表盘**：只统计「后端解释不了」的值。
    /// 「前端没有专属配色」是**另一件事**，不在本字段里 —— 后者由前端图例的
    /// 关系类型分布回答（见 `GraphView.tsx` 的 `edgeRelationLegend`）。
    /// 把两者混为一谈会让本字段对存量数据直接刷屏（实测 53 个中文关系类型 / 74325 行）。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub unresolved_relations: Vec<UnresolvedRelationStat>,
    /// **图的自洽性**：端点不在 [`Self::nodes`] 里的边统计（P0，2026-09-17）。
    ///
    /// 与前两个字段**并列的第三类事实**，判据不同（那边是词汇表，这边是端点存在性，
    /// 详见 [`DanglingEdgeSummary`] 的口径表），所以是独立字段而不是塞进
    /// `unresolved_relations` 的一个 `raw_type`。
    ///
    /// **兼容性与另两个不同，刻意不 skip**：`unresolved_*` 用
    /// `skip_serializing_if = "Vec::is_empty"`（空清单不写进 JSON，正常图不背字段）；
    /// 本字段**总是序列化**。理由：`dangling == 0 && total_edges == 120000`
    /// 与「字段缺失」是**两件事**——前者是「查过了，很干净」，后者是
    /// 「这份数据是旧版写的，没查过」。用一个 `Option` + skip 表达会让二者再次混淆。
    ///
    /// `#[serde(default)]` 仍在：旧版缓存 JSON 缺该字段时解析为
    /// `total_edges = 0`（**不撒谎说「干净」**——`0` 只意味着「这份数据没统计过」，
    /// 而调用方在返回前必调 [`Self::refresh_unresolved`] 重算，实际读不到这个 0）。
    #[serde(default)]
    pub dangling_edges: DanglingEdgeSummary,
}

impl GraphData {
    /// 构造图谱数据，并**顺带算好**两侧的「未识别」统计。
    ///
    /// 用构造函数而不是散落的 `GraphData { nodes, edges, .. }`：字段对外是 `pub`，
    /// 但新增字段时若靠人记住「每个字面量都要补」，必然漏。统一走这里 = 只有一处要改。
    pub fn new(nodes: Vec<GraphNode>, edges: Vec<GraphEdge>) -> Self {
        let unresolved_types = collect_unresolved_type_stats(&nodes);
        let unresolved_relations = collect_unresolved_relation_stats(&edges);
        let dangling_edges = collect_dangling_edge_stats(&nodes, &edges);
        Self { nodes, edges, unresolved_types, unresolved_relations, dangling_edges }
    }

    /// 就地重算**全部三类**诊断统计（节点未识别类型 + 边未识别标签 + 悬空边）。
    ///
    /// 用在「节点/边被合并、过滤之后」（例如 `get_wiki_graph_cached` 融合了知识库实体
    /// 节点与其关系边）：统计必须对**最终返回给前端的集合**成立，而不是对融合前的中间态成立。
    ///
    /// 三类共用**一个**刷新入口而不是三个方法：它们总是一起被调用
    /// （融合后重算），拆开迟早出现「只调了一个」的图 ——
    /// 那种图的两个字段会互相矛盾，而矛盾的两半都在 JSON 里。
    /// 悬空边尤其如此：**融合恰恰是唯一会新增悬空边的步骤**
    /// （实体边来自另一张表、按另一个键过滤），漏刷就等于把这条绊线直接关掉。
    pub fn refresh_unresolved(&mut self) {
        self.unresolved_types = collect_unresolved_type_stats(&self.nodes);
        self.unresolved_relations = collect_unresolved_relation_stats(&self.edges);
        self.dangling_edges = collect_dangling_edge_stats(&self.nodes, &self.edges);
    }

    /// 丢掉**画不出来**的边（端点不在 [`Self::nodes`] 里），但把丢掉的规模**留在**统计里。
    ///
    /// 返回本次淘汰的条数；返回 `0` 时**不动** [`Self::dangling_edges`]。
    ///
    /// ## 为什么必须有这一步
    ///
    /// [`Self::refresh_unresolved`] 只**报**不**改**，于是「端点缺失」这件事长期被劈成两半：
    /// 统计字段说「172,926 条边里有 276 条画不出来」，而同一份 JSON 的 `edges` 长度
    /// 仍是 172,926 —— 渲染层跳过那 276 条（`idSet.has` 判否即 `continue`），
    /// 工具栏却照旧显示 `172926`（`GraphView` 的 `edgeCount = data.edges.length`）。
    /// **用户读到的边数里混着根本没画出来的边**，且这个偏差会随任何一次上游脏数据
    /// （跨库合并、写端域不一致）**重新出现** —— 只修数据不修这里，下次照样虚高。
    ///
    /// 本方法把「不可绘制」从 `edges` 里摘掉，同时**不让它变成静默**：
    /// 淘汰规模继续留在 [`Self::dangling_edges`] 里，界面照旧告警（判据不变）。
    ///
    /// ## 与 `refresh_unresolved` 的调用顺序
    ///
    /// **先 `refresh_unresolved()`，再本方法**（或只调本方法，它自带重算）。
    /// 反过来的后果是：先剔边、后刷新 ⇒ 刷新按剔后的边集重算 ⇒ `dangling` 归 0，
    /// 而脏数据已经不在 `edges` 里了 —— 一次静默的「修好了」。
    ///
    /// ## 幂等
    ///
    /// 已在 `wiki.rs` 的融合出口验证：二次调用时已无悬空边 ⇒ 统计为 0 ⇒ 提前返回，
    /// **不覆盖**上一次的淘汰记录。这不是洁癖 —— `refresh_unresolved` 是「重算」，
    /// 若本方法也重算并覆盖，任何一次多余的刷新都会把那 276 抹成 0，告警消失而脏数据仍在。
    pub fn retain_resolved_edges(&mut self) -> usize {
        if self.edges.is_empty() {
            return 0;
        }

        // 刻意**重算**而不是复用 `self.dangling_edges`：淘汰依据与淘汰结果必须同源，
        // 否则「调用方忘了先 refresh」会让剔除按过期统计执行（多剔/漏剔都无迹可查）。
        // 代价是一次 `O(|nodes| + |edges|)` —— 只在打开图谱时跑一次，不在逐帧路径上。
        let summary = collect_dangling_edge_stats(&self.nodes, &self.edges);
        if summary.dangling == 0 {
            return 0;
        }

        let ids: std::collections::HashSet<&str> =
            self.nodes.iter().map(|n| n.id.as_str()).collect();
        let before = self.edges.len();
        // 判据必须与 `collect_dangling_edge_stats` **逐字相同**（两端都在才保留），
        // 否则统计说「淘汰 276」而实际淘汰别的数，两者之差无处解释。
        self.edges.retain(|e| ids.contains(e.source.as_str()) && ids.contains(e.target.as_str()));
        let dropped = before - self.edges.len();
        debug_assert_eq!(dropped, summary.dangling, "淘汰数必须与统计数逐位相同");
        self.dangling_edges = summary;
        dropped
    }
}

// ── Relevance signal types ──

/// Raw relevance signals between two graph nodes.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RelevanceSignal {
    pub direct_link: f64,
    pub source_overlap: f64,
    pub adamic_adar: f64,
    pub type_affinity: f64,
}

/// A scored relevance edge between two nodes.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RelevanceEdge {
    pub source: String,
    pub target: String,
    pub signal: RelevanceSignal,
    pub total_score: f64,
}

// ── Extension traits (defined here, implemented in dao) ──

/// Scoring extension trait for `RelevanceSignal`.
pub trait RelevanceSignalExt {
    fn total_score(&self) -> f64;
    fn normalized_total(&self) -> f64;
}

impl RelevanceSignalExt for RelevanceSignal {
    fn total_score(&self) -> f64 {
        self.direct_link * 3.0
            + self.source_overlap * 4.0
            + self.adamic_adar * 1.5
            + self.type_affinity * 1.0
    }

    fn normalized_total(&self) -> f64 {
        let raw = self.total_score();
        let max = 3.0 + 4.0 + 1.5 + 1.0;
        (raw / max).min(1.0)
    }
}

// ── PageType 解析失败的**可观测化** ──

/// 页面类型解析失败的原因。
///
/// 原实现把两种情况都吞成 `PageType::Unknown`（`get_page_type` 内
/// `.and_then(|n| n.node_type.parse::<PageType>().ok()).unwrap_or(Unknown)`），
/// 于是「节点缺失」与「字面量不在词汇表」**都无法被调用方察觉**，
/// 下游亲和度静默落 `0.3`（同类型）/ `0.0`（跨类型）。
///
/// 本类型只做一件事：把这两种失败**变成可返回、可统计、可上报**的值。
/// 数值行为不变（见 `get_page_type` 的文档与等价性测试）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PageTypeLookupError {
    /// 图中不存在这个节点 id
    MissingNode(String),
    /// 节点存在，但 `node_type` 字面量不在 [`PageType`] 词汇表内
    UnrecognizedType { node_id: String, raw: String },
}

/// 一个「未识别的类型字面量」在图中造成的降级规模（A2-升级）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UnresolvedTypeStat {
    /// 节点上的原始 `node_type` 字面量（不在词汇表内的那个值）
    pub raw_type: String,
    /// 该字面量在图中出现的节点数
    pub count: usize,
    /// 样例节点 id（**最多 3 个**，升序）—— 让用户能直接去定位来源
    pub sample_node_ids: Vec<String>,
}

/// 样例 id 的上限（节点 id / 边 id 共用）。给 UI 用，太多只会在提示条里被截断。
///
/// ⚠ 名字不带 `UNRESOLVED_`：悬空边（[`DanglingEdgeSummary`]）也用同一个上限，
/// 三处样例长度保持一致 —— 名字里写死一类用途，下一类统计就会「顺手再定一个 5」。
const SAMPLE_ID_LIMIT: usize = 3;

/// 收集未识别的节点 —— `(node_id, raw_type)`，**按 node_id 升序**（确定性）。
///
/// ⚠ **判据只此一份**（`PageType::parse_strict(...).is_none()`）：
/// `LinkGraph::from_graph_data` 与 [`collect_unresolved_type_stats`] 都调用它，
/// 不允许任何一侧「顺手再判一次」—— 两份判据迟早会分叉。
pub fn collect_unresolved_nodes<'a>(
    nodes: impl IntoIterator<Item = &'a GraphNode>,
) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = nodes
        .into_iter()
        .filter(|n| PageType::parse_strict(&n.node_type).is_none())
        .map(|n| (n.id.clone(), n.node_type.clone()))
        .collect();
    out.sort();
    out
}

/// 一个「未识别的边标签」在图中造成的降级规模（P2-b，2026-09-14）。
///
/// 与 [`UnresolvedTypeStat`] 是**并列**的两个 DTO，不是同一个东西的两种叫法：
/// 那个的 id 列是**节点 id**，这个的是**边 id**。共用同一个 struct 会让
/// `sampleNodeIds` 这个字段名在边侧撒谎 —— 而字段名撒谎的成本，比多一个 struct 高。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UnresolvedRelationStat {
    /// 边上那个后端解释不了的原始标签（`type` 或 `relation_type` 之一）
    pub raw_type: String,
    /// 该字面量在图中出现的**边**数
    pub count: usize,
    /// 样例边标识（**最多 3 个**，升序）—— 形如 `source -> target`，让用户能直接定位
    pub sample_edge_ids: Vec<String>,
}

/// 聚合的**唯一实现**：把 `(id, raw_label)` 按 raw 分组，产出
/// `(raw_label, 该 raw 的 id 数, 前 N 个样例 id)`，并按确定性规则排序。
///
/// 节点侧与边侧共用它 —— 排序规则、样例上限、分组语义都只写一遍。
/// 两侧各自的公开函数只负责「判据 + DTO 映射」这一薄层。
///
/// ⚠ 计数与样例都取自**同一次遍历**的结果：若把计数改成「拿样例长度」，
/// 样例上限一改就静默改了计数 —— 这正是本函数返回 `(raw, count, samples)` 三元组的原因。
fn group_by_raw_label(pairs: &[(String, String)]) -> Vec<(String, usize, Vec<String>)> {
    let mut by_type: std::collections::BTreeMap<&str, Vec<&str>> =
        std::collections::BTreeMap::new();
    for (id, raw) in pairs {
        by_type.entry(raw.as_str()).or_default().push(id.as_str());
    }
    let mut grouped: Vec<(String, usize, Vec<String>)> = by_type
        .into_iter()
        .map(|(raw, mut ids)| {
            ids.sort_unstable();
            let count = ids.len();
            (
                raw.to_string(),
                count,
                ids.iter().take(SAMPLE_ID_LIMIT).map(|s| s.to_string()).collect(),
            )
        })
        .collect();
    // 排序规则（确定性，便于测试与「同一份数据每次渲染一样」）：
    // `count` 降序 → `raw_type` 升序。`count` 相同再按字面量排，避免 HashMap 迭代顺序泄漏到输出。
    grouped.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    grouped
}

/// 把 `(node_id, raw_type)` 聚合成面向 UI 的统计。
pub fn aggregate_unresolved_stats(pairs: &[(String, String)]) -> Vec<UnresolvedTypeStat> {
    group_by_raw_label(pairs)
        .into_iter()
        .map(|(raw_type, count, sample_node_ids)| UnresolvedTypeStat {
            raw_type,
            count,
            sample_node_ids,
        })
        .collect()
}

/// 直接按 `raw_type` 聚合（`count` 降序）。
///
/// ⚠ **按 `raw_type` 聚合会把同名但大小写不同的字面量分开计数**（这是刻意的：`Doc` 与 `doc`
/// 在词汇表里就是两个不同的事实，静默合并会让「大小写不一致的数据质量问题」不可见）。
pub fn collect_unresolved_type_stats(nodes: &[GraphNode]) -> Vec<UnresolvedTypeStat> {
    aggregate_unresolved_stats(&collect_unresolved_nodes(nodes.iter()))
}

/// 收集标签无法被后端解释的边 —— `(edge_id, raw_label)`，**按 edge_id 升序**（确定性）。
///
/// ⚠ **判据只此一份**：[`crate::knowledge_graph::uninterpreted_edge_label`]。
/// 这里只负责「给一条边生成稳定的 id」与「取值」，判断「什么算可解释」不在本文件里。
///
/// `edge_id` 取 `source -> target`（带一个空格，避免与含 `->` 的 id 混淆到无法人工分辨——
/// 图上的边就是由这两个端点唯一标示的；同端点多条边会得到同一个 id，
/// 于是计数变成「按端点对」，这与样例的用途（让人找到那条边）一致）。
pub fn collect_unresolved_edges(edges: &[GraphEdge]) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = edges
        .iter()
        .filter_map(|e| {
            crate::knowledge_graph::uninterpreted_edge_label(
                &e.edge_type,
                e.relation_type.as_deref(),
            )
            .map(|raw| (format!("{} -> {}", e.source, e.target), raw))
        })
        .collect();
    out.sort();
    out.dedup();
    out
}

/// 边侧统计（与 [`collect_unresolved_type_stats`] 对称）。
pub fn collect_unresolved_relation_stats(edges: &[GraphEdge]) -> Vec<UnresolvedRelationStat> {
    group_by_raw_label(&collect_unresolved_edges(edges))
        .into_iter()
        .map(|(raw_type, count, sample_edge_ids)| UnresolvedRelationStat {
            raw_type,
            count,
            sample_edge_ids,
        })
        .collect()
}

// ── 悬空边（端点缺失）统计 ──

/// 端点**不在同一份节点集里**的边统计（P0，2026-09-17）。
///
/// # 为什么需要它
///
/// 读端对「端点找不到」的边是**静默跳过**的（`GraphView.tsx` 的
/// `if (!idSet.has(em.source) || !idSet.has(em.target)) { continue; }`），
/// 后端融合侧也同样一言不发。实测后果：某个 wiki 返回 24,288 个节点 / 74,791 条边，
/// 其中 **38,171 条 `reference` 边两端全部不在节点集里**（知识库实体被一次
/// **跨库合并**搬到了另一个 `knowledge_base_id`，边没跟着搬）——
/// 界面上表现为「满屏孤立气泡、一条连线都没有」，而工具栏照旧写着 `74791E`，
/// **系统里没有任何一处说得出「边的绝大多数根本没画出来」**。
///
/// 所以它与 [`UnresolvedTypeStat`] / [`UnresolvedRelationStat`] 同属**绊线**族：
/// 正常图上恒为 `dangling == 0`。差别在绊的是**图的自洽性**而不是**词汇表**。
///
/// # 判据口径（⚠ 与 dao 层 `audit_kb_domain_consistency` **不是同一个判据**，禁止互相顶替）
///
/// 本函数回答的是「**返回给前端的这张图**是否自洽」——判据是边的两端是否出现在
/// **同一份 `nodes` 数组**里。dao 层那个回答的是「**DB** 里的边端点在 DB 里是否可见」
/// （`lifecycle IS NULL`）。两者在「读端按 kb 过滤掉一批节点」时会给出**不同**答案，
/// 而那个差异本身就是诊断信息：
///
/// | DB 自洽 | 返回图悬空 | 断点位置 |
/// |---|---|---|
/// | ✅ | ❌ | 融合 / 过滤层（本字段报，见 `get_wiki_graph_cached`） |
/// | ❌ | ❌ | 写入层（dao 的 `dangling` 报，典型 = 跨库合并搬走了实体） |
/// | ✅ | ✅ | 正常 |
///
/// 另一种「两个判据」的错法是把本字段与 [`Self::unresolved_relations`] 混为一谈：
/// 后者统计「标签后端解释不了」，**与端点存不存在完全无关**，存量数据上
/// 实测有 53 个中文关系类型（74325 行）—— 拿它当「边没画出来」的指标会直接刷屏。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DanglingEdgeSummary {
    /// 统计基准：**淘汰前**参与统计的边数
    ///
    /// 两种取值，取决于是否调用过 [`GraphData::retain_resolved_edges`]：
    /// * 未调用 ⇒ 等于 `edges.len()`（`GraphData::new` 之后就是这个形态）；
    /// * 已调用 ⇒ **大于** `edges.len()` —— 差额正是被淘汰掉的那批。
    ///
    /// 两种形态都满足 `edges.len() + dangling == total_edges`，
    /// 所以读这个字段时**不要**假设它与 `edges.len()` 相等（那正是本字段要修的病）。
    pub total_edges: usize,
    /// 至少一端缺失的边数（= 下面三项之和）
    pub dangling: usize,
    /// **只有** `source` 缺失
    pub missing_source_only: usize,
    /// **只有** `target` 缺失
    pub missing_target_only: usize,
    /// 两端都缺失
    pub missing_both: usize,
    /// 样例边标识（**最多 [`SAMPLE_ID_LIMIT`] 个**，形如 `source -> target`）
    pub sample_edge_ids: Vec<String>,
}

/// 收集端点缺失的边统计（`nodes` 是**最终**节点集，`edges` 是**最终**边集）。
///
/// 三项分类**互斥**（`missing_source_only + missing_target_only + missing_both == dangling`）——
/// 刻意不定义成「source 缺失数 / target 缺失数」那种会重叠的口径：
/// 重叠口径下 `a + b > dangling`，而读的人一定会去加，加出来的数与 `dangling` 对不上时
/// 无法判断是「统计错了」还是「口径就是重叠的」。
///
/// ⚠ 复杂度 `O(|nodes| + |edges|)`：调用方在**构造时就地算**（见 [`GraphData::new`]），
/// 别放进逐帧渲染路径 —— 那会把一次性的诊断变成每帧的哈希表构建。
pub fn collect_dangling_edge_stats(
    nodes: &[GraphNode],
    edges: &[GraphEdge],
) -> DanglingEdgeSummary {
    if edges.is_empty() {
        return DanglingEdgeSummary::default();
    }

    let ids: std::collections::HashSet<&str> = nodes.iter().map(|n| n.id.as_str()).collect();
    let mut summary = DanglingEdgeSummary { total_edges: edges.len(), ..Default::default() };

    for edge in edges {
        let src_missing = !ids.contains(edge.source.as_str());
        let tgt_missing = !ids.contains(edge.target.as_str());
        if !src_missing && !tgt_missing {
            continue;
        }
        summary.dangling += 1;
        match (src_missing, tgt_missing) {
            (true, true) => summary.missing_both += 1,
            (true, false) => summary.missing_source_only += 1,
            (false, true) => summary.missing_target_only += 1,
            // 上面已 `continue`，此臂不可达；写成 `_ => {}` 而不是 `unreachable!()`：
            // 诊断代码不该有「统计不出来就 panic」的分支（那会让一个观测设施变成故障源）。
            _ => {},
        }
        if summary.sample_edge_ids.len() < SAMPLE_ID_LIMIT {
            summary.sample_edge_ids.push(format!("{} -> {}", edge.source, edge.target));
        }
    }

    summary
}

// ── LinkGraph — full graph data structure ──

#[derive(Debug, Clone)]
pub struct LinkGraph {
    nodes: HashMap<String, GraphNode>,
    edges: Vec<GraphEdge>,
    adjacency: HashMap<String, Vec<String>>,
    node_titles: HashMap<String, String>,
    /// 类型字面量不在词汇表内的节点 `(node_id, raw_type)`，**按 id 排序**（确定性）。
    ///
    /// 该清单在 [`LinkGraph::from_graph_data`] 里一次性算出并同时 `warn` 一次 ——
    /// 判定函数 `compute_type_affinity` 处在 O(n²) 路径上，不能在那里打日志。
    unresolved_types: Vec<(String, String)>,
}

impl LinkGraph {
    pub fn from_graph_data(data: GraphData) -> Self {
        let mut nodes = HashMap::new();
        let mut node_titles = HashMap::new();
        let mut adjacency: HashMap<String, Vec<String>> = HashMap::new();

        for node in data.nodes {
            node_titles.insert(node.id.clone(), node.title.clone());
            nodes.insert(node.id.clone(), node);
        }

        for edge in &data.edges {
            adjacency.entry(edge.source.clone()).or_default().push(edge.target.clone());
            adjacency.entry(edge.target.clone()).or_default().push(edge.source.clone());
        }

        // 类型解析失败**一次性**收集（A2：把静默降级变成可观测）。
        // 判据与聚合都在上方共享函数里（只有一份）—— 原先此处自己写了一版
        // `filter(parse_strict().is_none())`，与 `collect_unresolved_type_stats` 是两份判据。
        let unresolved_types = collect_unresolved_nodes(nodes.values());

        if !unresolved_types.is_empty() {
            tracing::warn!(
                count = unresolved_types.len(),
                total = nodes.len(),
                sample = ?unresolved_types.iter().take(3).collect::<Vec<_>>(),
                "图谱节点 node_type 不在 PageType 词汇表内 ⇒ 亲和度落兜底值（此前为静默降级，无任何信号）"
            );
        }

        Self { nodes, edges: data.edges, adjacency, node_titles, unresolved_types }
    }

    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }

    pub fn edge_count(&self) -> usize {
        self.edges.len()
    }

    pub fn get_neighbors(&self, node_id: &str) -> Vec<String> {
        self.adjacency.get(node_id).cloned().unwrap_or_default()
    }

    pub fn get_degree(&self, node_id: &str) -> usize {
        self.adjacency.get(node_id).map(|v| v.len()).unwrap_or(0)
    }

    pub fn has_direct_link(&self, a: &str, b: &str) -> bool {
        self.adjacency.get(a).map(|neighbors| neighbors.contains(&b.to_string())).unwrap_or(false)
    }

    pub fn get_node_ids(&self) -> Vec<String> {
        let mut ids: Vec<String> = self.nodes.keys().cloned().collect();
        // 排序消除 HashMap::keys() 迭代顺序的随机性：
        // Louvain 等下游算法依赖稳定的节点处理顺序，相同输入必须产生相同输出
        ids.sort();
        ids
    }

    pub fn get_node(&self, node_id: &str) -> Option<&GraphNode> {
        self.nodes.get(node_id)
    }

    pub fn get_node_title(&self, node_id: &str) -> Option<&str> {
        self.node_titles.get(node_id).map(|s| s.as_str())
    }

    /// ⚠ **有损**便利入口：解析失败（节点缺失 / 字面量不在词汇表）一律返回
    /// [`PageType::Unknown`]。**数值行为与改造前逐值等价**（改造前是
    /// `.and_then(parse().ok()).unwrap_or(Unknown)`，因 `FromStr` 从不返回 `Err`
    /// 而实际等价于「未识别 ⇒ `Unknown`」）。
    ///
    /// 需要区分「真的是 `unknown`」与「解析失败」时用 [`Self::get_page_type_strict`]；
    /// 需要知道降级规模时用 [`Self::unresolved_types`]。
    pub fn get_page_type(&self, node_id: &str) -> PageType {
        self.get_page_type_strict(node_id).unwrap_or(PageType::Unknown)
    }

    /// **严格**取类型：区分「节点缺失」与「字面量不在词汇表」。
    ///
    /// 这是 `get_page_type` 的唯一实现体 —— 有损版只是它的一层显式兜底，
    /// 于是「兜底发生在哪、兜成了什么」在源码里只有**一处**可看。
    pub fn get_page_type_strict(&self, node_id: &str) -> Result<PageType, PageTypeLookupError> {
        let node = self
            .nodes
            .get(node_id)
            .ok_or_else(|| PageTypeLookupError::MissingNode(node_id.to_string()))?;
        PageType::parse_strict(&node.node_type).ok_or_else(|| {
            PageTypeLookupError::UnrecognizedType {
                node_id: node_id.to_string(),
                raw: node.node_type.clone(),
            }
        })
    }

    /// 类型字面量不在词汇表内的节点清单 `(node_id, raw_type)`，按 id 排序。
    pub fn unresolved_types(&self) -> &[(String, String)] {
        &self.unresolved_types
    }

    pub fn compute_relevance(
        &self,
        page_a: &str,
        page_b: &str,
        source_map: &HashMap<String, Vec<String>>,
    ) -> RelevanceSignal {
        RelevanceSignal {
            direct_link: if self.has_direct_link(page_a, page_b) {
                1.0
            } else {
                0.0
            },
            source_overlap: self.compute_source_overlap(page_a, page_b, source_map),
            adamic_adar: self.compute_adamic_adar(page_a, page_b),
            type_affinity: self.compute_type_affinity(page_a, page_b),
        }
    }

    pub fn get_top_k_related(
        &self,
        node_id: &str,
        source_map: &HashMap<String, Vec<String>>,
        k: usize,
    ) -> Vec<(String, RelevanceSignal)> {
        let mut scored: Vec<_> = self
            .get_node_ids()
            .into_iter()
            .filter(|id| id != node_id)
            .map(|other| {
                let signal = self.compute_relevance(node_id, &other, source_map);
                (other, signal)
            })
            .filter(|(_, s)| s.total_score() > 0.0)
            .collect();

        scored.sort_by(|a, b| {
            b.1.total_score().partial_cmp(&a.1.total_score()).unwrap_or(std::cmp::Ordering::Equal)
        });
        scored.truncate(k);
        scored
    }

    pub fn expand_seeds(
        &self,
        seed_ids: &[String],
        source_map: &HashMap<String, Vec<String>>,
        top_k: usize,
        max_hops: usize,
    ) -> Vec<(String, RelevanceSignal)> {
        let mut visited: HashSet<String> = seed_ids.iter().cloned().collect();
        let mut current: HashSet<String> = seed_ids.iter().cloned().collect();
        let mut results: Vec<(String, RelevanceSignal)> = Vec::new();

        for hop in 0..max_hops {
            let decay = 1.0 / (hop + 1) as f64;
            let mut next: HashSet<String> = HashSet::new();

            for seed in &current {
                let neighbors = self.get_neighbors(seed);
                for neighbor in neighbors {
                    if visited.contains(&neighbor) {
                        continue;
                    }
                    visited.insert(neighbor.clone());
                    next.insert(neighbor.clone());

                    let mut signal = self.compute_relevance(seed, &neighbor, source_map);
                    signal.direct_link *= decay;
                    signal.source_overlap *= decay;
                    signal.adamic_adar *= decay;
                    signal.type_affinity *= decay;
                    results.push((neighbor, signal));
                }
            }

            current = next;
        }

        results.sort_by(|a, b| {
            b.1.total_score().partial_cmp(&a.1.total_score()).unwrap_or(std::cmp::Ordering::Equal)
        });
        results.truncate(top_k);
        results
    }

    pub fn build_relevance_edges(
        &self,
        source_map: &HashMap<String, Vec<String>>,
        threshold: f64,
    ) -> Vec<RelevanceEdge> {
        let nodes = self.get_node_ids();
        let mut edges = Vec::new();

        for i in 0..nodes.len() {
            for j in (i + 1)..nodes.len() {
                let signal = self.compute_relevance(&nodes[i], &nodes[j], source_map);
                let total = signal.total_score();
                if total >= threshold {
                    edges.push(RelevanceEdge {
                        source: nodes[i].clone(),
                        target: nodes[j].clone(),
                        signal,
                        total_score: total,
                    });
                }
            }
        }

        edges
    }

    fn compute_source_overlap(
        &self,
        page_a: &str,
        page_b: &str,
        source_map: &HashMap<String, Vec<String>>,
    ) -> f64 {
        let sources_a: HashSet<_> =
            source_map.get(page_a).cloned().unwrap_or_default().into_iter().collect();
        let sources_b: HashSet<_> =
            source_map.get(page_b).cloned().unwrap_or_default().into_iter().collect();

        if sources_a.is_empty() || sources_b.is_empty() {
            return 0.0;
        }

        let intersection = sources_a.intersection(&sources_b).count() as f64;
        let union = sources_a.union(&sources_b).count() as f64;
        if union == 0.0 {
            0.0
        } else {
            intersection / union
        }
    }

    fn compute_adamic_adar(&self, page_a: &str, page_b: &str) -> f64 {
        let neighbors_a: HashSet<_> = self.get_neighbors(page_a).into_iter().collect();
        let neighbors_b: HashSet<_> = self.get_neighbors(page_b).into_iter().collect();
        let common: Vec<_> = neighbors_a.intersection(&neighbors_b).collect();

        common
            .iter()
            .map(|n| {
                let degree = self.get_degree(n) as f64;
                // guard: degree-1 must be > 1 to avoid ln(0) = -inf → 1/0 = inf
                if degree > 2.0 {
                    1.0 / (degree - 1.0).ln()
                } else {
                    0.0
                }
            })
            .sum()
    }

    /// 类型对 → 亲和度。
    ///
    /// 2026-09-14：公理表已外迁到 [`crate::page_type`]（`SAME_TYPE_AFFINITY` /
    /// `SAME_TYPE_FALLBACK` / `CROSS_TYPE_AFFINITY` / `CROSS_TYPE_FORBIDDEN`），
    /// 本函数只做查询。外迁理由：原实现把「哪些类型对算相关、相关度多少」这条
    /// **不相交公理**埋在函数体的 `match` 里 —— 可执行、但不可测、不可审计、
    /// 也无法回答「仓库里一共有几套类型判定」。
    ///
    /// ⚠ 解析失败仍按 [`PageType::Unknown`] 参与计算 ⇒ **数值与旧实现逐值等价**，
    /// 由 `test_type_affinity_matches_legacy_hardcoded_table` 对全部 8×8 个词汇表类型对
    /// （外加 2 个越界字面量）锁定。
    /// 降级本身不再是静默的：见 [`LinkGraph::unresolved_types`]。
    fn compute_type_affinity(&self, page_a: &str, page_b: &str) -> f64 {
        let type_a = self.get_page_type(page_a);
        let type_b = self.get_page_type(page_b);

        if type_a == type_b {
            same_type_affinity(type_a)
        } else {
            cross_type_affinity(type_a, type_b)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(id: &str, node_type: &str) -> GraphNode {
        GraphNode {
            id: id.to_string(),
            title: id.to_string(),
            node_type: node_type.to_string(),
            tags: Vec::new(),
            link_count: 0,
            backlink_count: 0,
            path: String::new(),
        }
    }

    fn graph(pairs: &[(&str, &str)]) -> LinkGraph {
        LinkGraph::from_graph_data(GraphData::new(
            pairs.iter().map(|(id, ty)| node(id, ty)).collect(),
            Vec::new(),
        ))
    }

    /// **改造前的源码逐字复制**（`compute_type_affinity` 的旧实现 + 旧 `get_page_type`
    /// 的宽松解析），作为等价性基准。若哪天两套对不上，说明「外迁公理表」引入了行为变更。
    fn legacy_affinity(raw_a: &str, raw_b: &str) -> f64 {
        let type_a = raw_a.parse::<PageType>().unwrap_or(PageType::Unknown);
        let type_b = raw_b.parse::<PageType>().unwrap_or(PageType::Unknown);

        if type_a == type_b {
            match type_a {
                PageType::Entity | PageType::Concept => 1.0,
                PageType::SourceSummary => 0.5,
                _ => 0.3,
            }
        } else {
            match (&type_a, &type_b) {
                (PageType::Entity, PageType::Concept) | (PageType::Concept, PageType::Entity) => {
                    0.5
                },
                _ => 0.0,
            }
        }
    }

    /// 外迁公理表的**等价性锁**：新旧实现对每个类型对必须给出同一个数。
    ///
    /// 矩阵 = **全部**词汇表字面量（取自 `ALL_PAGE_TYPES`，D3 后为 14 个）+ 2 个
    /// **不在**词汇表内的字面量（`wiki_page` / `"Entity "`）—— 后者才是兜底路径真正的入口，
    /// 也防止矩阵只覆盖「好看的」值。
    ///
    /// ⚠ **D3（2026-09-14）后本锁的含义变了**：`log` / `daily` 已升格为词汇表成员，
    /// 不再是「越界值」样本。它们与其它非 A1 类型一样落兜底 `0.3`，故矩阵取值不变；
    /// 变的是「两个不同的越界值之间」从 `0.3`（都是 `Unknown`）变成 `0.0`（各自的类型）——
    /// 实测无影响（真实 `notes.page_type` 里只有一个越界值 `doc`，不存在成对的两个不同越界值）。
    #[test]
    fn test_type_affinity_matches_legacy_hardcoded_table() {
        let mut types: Vec<&str> =
            crate::page_type::ALL_PAGE_TYPES.iter().map(|t| t.as_str()).collect();
        types.push("wiki_page"); // 越界值（兜底入口）
        types.push("Entity "); // 大小写/空白不归一 ⇒ 同样是兜底入口
        let nodes: Vec<GraphNode> = types.iter().map(|t| node(&format!("n_{t}"), t)).collect();
        let g = LinkGraph::from_graph_data(GraphData::new(nodes, Vec::new()));

        let mut checked = 0usize;
        for a in &types {
            for b in &types {
                let got = g.compute_type_affinity(&format!("n_{a}"), &format!("n_{b}"));
                let want = legacy_affinity(a, b);
                assert_eq!(got, want, "类型对 ({a}, {b}) 亲和度与旧实现不一致");
                checked += 1;
            }
        }
        assert_eq!(checked, types.len() * types.len(), "必须覆盖全部类型对");
        assert!(types.len() >= 16, "矩阵必须含全部词汇表字面量 + 越界样本");
    }

    #[test]
    fn test_get_page_type_strict_distinguishes_failure_reasons() {
        // 注意：`log` 自 D3（2026-09-14）起**已登记为词汇表成员**，不再能充当「未识别」样本
        let g = graph(&[("a", "concept"), ("b", "wiki_page")]);

        assert_eq!(g.get_page_type_strict("a"), Ok(PageType::Concept));
        assert_eq!(g.get_page_type("a"), PageType::Concept);

        // 字面量不在词汇表 ⇒ 严格入口报 UnrecognizedType；有损入口行为不变（Unknown）
        assert_eq!(
            g.get_page_type_strict("b"),
            Err(PageTypeLookupError::UnrecognizedType {
                node_id: "b".to_string(),
                raw: "wiki_page".to_string()
            })
        );
        assert_eq!(g.get_page_type("b"), PageType::Unknown);

        // 节点缺失与字面量非法必须是**两种**原因（原实现无法区分）
        assert_eq!(
            g.get_page_type_strict("missing"),
            Err(PageTypeLookupError::MissingNode("missing".to_string()))
        );
        assert_eq!(g.get_page_type("missing"), PageType::Unknown);
    }

    #[test]
    fn test_unresolved_types_reported_sorted() {
        let g = graph(&[("z", "wiki_page"), ("a", "not_a_page_type"), ("ok", "entity")]);
        assert_eq!(
            g.unresolved_types(),
            &[
                ("a".to_string(), "not_a_page_type".to_string()),
                ("z".to_string(), "wiki_page".to_string())
            ],
            "必须按 id 排序（确定性）且只含解析失败项"
        );

        // 负向对照 1：全部在词汇表内 ⇒ 不得报出任何一项（含 D3 新登记的真实写入值）
        let clean = graph(&[
            ("a", "entity"),
            ("b", "note"),
            ("c", "unknown"),
            ("d", "doc"),
            ("e", "daily"),
            ("f", "knowledge"),
            ("g", "knowledge_document"),
            ("h", "synced"),
            ("i", "log"),
        ]);
        assert!(clean.unresolved_types().is_empty(), "全合规的图不得产生降级清单");
    }

    #[test]
    fn test_unrecognized_type_still_reaches_fallback_values() {
        // 锁定既有行为：不在词汇表的节点**不会消失**，只是静默降级到兜底值。
        // （「消失」是错的修法 —— 会改变打分结果；本轮只让降级可见，不动数值。）
        let same = graph(&[("a", "wiki_page"), ("b", "wiki_page")]);
        assert_eq!(same.compute_type_affinity("a", "b"), 0.3);

        let cross = graph(&[("a", "wiki_page"), ("b", "entity")]);
        assert_eq!(cross.compute_type_affinity("a", "b"), 0.0);

        // 这就是后果：未识别类型的页面与 Entity 页面在打分里亲和度 = 0，
        // 且在 graph_insights 里被算成「跨类型连接」（与真跨类型无法区分）。
        assert_ne!(
            cross.compute_type_affinity("a", "b"),
            cross_type_affinity(PageType::Entity, PageType::Concept)
        );

        // D3 对照：**已登记**的真实写入值走同一条兜底路径（取值不变），
        // 区别只在于它们不再出现在 `unresolved_types` 里 —— 降级信号留给真正的未知值。
        let declared_same = graph(&[("a", "doc"), ("b", "doc")]);
        assert_eq!(declared_same.compute_type_affinity("a", "b"), 0.3);
        assert!(declared_same.unresolved_types().is_empty());
    }

    // ── P2-b：边侧观测（与 A2 节点侧对称）──

    fn edge(src: &str, dst: &str, edge_type: &str, rel: Option<&str>) -> GraphEdge {
        match rel {
            Some(r) => GraphEdge::relation(src, dst, r),
            None => GraphEdge::structural(src, dst, edge_type),
        }
    }

    /// 边侧统计：排序确定、只含解释不了的项、样例上限生效。
    #[test]
    fn test_unresolved_relation_stats_report_and_sort() {
        let edges = vec![
            edge("e1", "e2", "reference", Some("has_concept")), // 可解释 ⇒ 不入清单
            edge("e1", "e3", "reference", Some("totally_made_up")),
            edge("e1", "e4", "reference", Some("also_made_up")),
            edge("e1", "e5", "reference", Some("totally_made_up")), // 与上面同标签、不同端点 ⇒ 计 2
            edge("note1", "note2", "link", None),                   // 笔记链接 ⇒ 可解释
            edge("x", "y", "not_a_kind", None),                     // 结构类型未登记
        ];
        let data = GraphData::new(Vec::new(), edges);

        let stats = &data.unresolved_relations;
        assert_eq!(
            stats.len(),
            3,
            "三个解释不了的标签：totally_made_up / also_made_up / not_a_kind"
        );
        // count 降序：totally_made_up(2) 在前
        assert_eq!(stats[0].raw_type, "totally_made_up");
        assert_eq!(stats[0].count, 2);
        // **并列时的次序**：count 相同则 raw_type 升序（确定性，不依赖 HashMap 迭代顺序）
        assert_eq!(stats[1].raw_type, "also_made_up");
        assert_eq!(stats[1].count, 1);
        assert_eq!(stats[2].raw_type, "not_a_kind");
        assert_eq!(stats[2].count, 1);

        // 样例是 `source -> target`，升序，最多 3 个
        assert_eq!(stats[0].sample_edge_ids, vec!["e1 -> e3", "e1 -> e5"]);
        assert!(stats.iter().all(|s| s.sample_edge_ids.len() <= 3), "样例必须受上限约束");
    }

    /// **反向对照（口径边界）**：可解释的图必须零清单 ——
    /// 尤其是**非 ASCII 数据驱动关系值**（DB 实测 53 个 / 74325 行）。
    /// 若哪天这条变红，说明判据被改成了「前端有没有配色」，
    /// 那会让本字段对存量数据直接刷屏（不是发现缺陷，是制造噪音）。
    #[test]
    fn test_unresolved_relations_are_empty_for_interpretable_graphs() {
        let clean = vec![
            edge("a", "b", "link", None),
            edge("c", "d", "mapping", None),
            edge("e", "f", "reference", Some("has_concept")),
            edge("g", "h", "reference", Some("in_industry")),
            edge("i", "j", "reference", Some("employ_董事")),
            edge("k", "l", "reference", Some("董事")),
            edge("m", "n", "reference", Some("独立董事")),
        ];
        let data = GraphData::new(Vec::new(), clean);
        assert!(data.unresolved_relations.is_empty(), "全可解释的图不得产生边侧清单");

        // 平行边只算一次（口径已文档化）：同一对端点三条同标签边 ⇒ count = 1
        let parallel = vec![
            edge("a", "b", "reference", Some("bogus")),
            edge("a", "b", "reference", Some("bogus")),
            edge("a", "b", "reference", Some("bogus")),
        ];
        let pd = GraphData::new(Vec::new(), parallel);
        assert_eq!(pd.unresolved_relations.len(), 1);
        assert_eq!(pd.unresolved_relations[0].count, 1, "平行边按端点对去重（口径见文档）");
    }

    /// `new()` 与 `refresh_unresolved()` 必须用**同一批判据** ——
    /// 否则「融合前后」两次统计会给出互相矛盾的结果，而两者会同时出现在 JSON 里。
    #[test]
    fn test_new_and_refresh_agree_on_both_sides() {
        // ⚠ wiki_page 在此是**故意的**：下面 assert!(!types_before.is_empty()) 需要节点侧清单非空。
        //    别把它当「干净基线」换成合规值 —— 那会让本用例失去两侧都非空的样本。
        let nodes = vec![node("a", "wiki_page"), node("b", "entity")];
        let edges =
            vec![edge("a", "b", "reference", Some("bogus_rel")), edge("b", "a", "link", None)];

        let mut data = GraphData::new(nodes, edges);
        let types_before = data.unresolved_types.clone();
        let rels_before = data.unresolved_relations.clone();
        assert!(!types_before.is_empty() && !rels_before.is_empty(), "样本必须两侧都有内容");

        // 集合未变 ⇒ 刷新后必须逐值相等
        data.refresh_unresolved();
        assert_eq!(data.unresolved_types, types_before);
        assert_eq!(data.unresolved_relations, rels_before);

        // 边被换掉 ⇒ 只有边侧变（节点侧不得跟着漂）
        data.edges = vec![edge("a", "b", "link", None)];
        data.refresh_unresolved();
        assert_eq!(data.unresolved_types, types_before, "节点侧不得因刷新边而改变");
        assert!(data.unresolved_relations.is_empty(), "边全合规 ⇒ 边侧清单清空");
    }

    /// `relationType` 的 serde 契约：缺省可解析（旧缓存）、`None` 不写进 JSON。
    #[test]
    fn test_graph_edge_relation_type_serde_contract() {
        // 旧版缓存里的边（没有 relationType 字段）必须仍能解析
        let legacy: GraphEdge =
            serde_json::from_str(r#"{"source":"a","target":"b","type":"link"}"#).unwrap();
        assert_eq!(legacy.edge_type, "link");
        assert_eq!(legacy.relation_type, None);

        // None ⇒ 不写字段（正常图不背这个 key）
        let s = serde_json::to_string(&GraphEdge::structural("a", "b", "link")).unwrap();
        assert!(!s.contains("relationType"), "None 不得写出 relationType：{s}");

        // Some ⇒ camelCase 写出
        let s2 = serde_json::to_string(&GraphEdge::relation("a", "b", "has_concept")).unwrap();
        assert!(s2.contains(r#""relationType":"has_concept""#), "关系边必须带上真实关系 id：{s2}");
        assert!(s2.contains(r#""type":"reference""#), "渲染类别保持 reference：{s2}");
    }

    /// 悬空边的四个计数**互斥**且三项之和 == `dangling`。
    ///
    /// 这条断言钉住的是一类具体误读：读的人一定会去加 `missingSource + missingTarget`，
    /// 若口径改成重叠（both 计入两者），加出来的数就 > `dangling`，
    /// 而那时无法判断是「统计错了」还是「口径如此」。
    #[test]
    fn test_dangling_edges_counts_are_exclusive_and_sum() {
        let nodes = vec![node("a", "entity"), node("b", "entity")]; // 本用例只关心悬空边，与节点类型无关 ⇒ 用合规值免噪声
        let edges = vec![
            edge("a", "b", "link", None),                                 // 干净
            edge("a", "ghost", "link", None),                             // 只缺 target
            edge("ghost", "b", "link", None),                             // 只缺 source
            edge("ghost", "phantom", "link", None),                       // 两端都缺
            edge("ghost2", "phantom2", "reference", Some("has_concept")), // 两端都缺（关系边同判）
        ];
        let s = GraphData::new(nodes, edges).dangling_edges;

        assert_eq!(s.total_edges, 5, "total 必须等于传入边数，与是否悬空无关");
        assert_eq!(s.dangling, 4);
        assert_eq!(s.missing_target_only, 1);
        assert_eq!(s.missing_source_only, 1);
        assert_eq!(s.missing_both, 2);
        assert_eq!(
            s.missing_source_only + s.missing_target_only + s.missing_both,
            s.dangling,
            "三项必须互斥且加总等于 dangling"
        );
        assert_eq!(s.sample_edge_ids.len(), super::SAMPLE_ID_LIMIT, "样例必须给满上限");
        assert_eq!(s.sample_edge_ids[0], "a -> ghost", "样例按边序取前 N 条（确定性）");
    }

    /// 干净图恒为 0 —— 这是它作为**绊线**而不是仪表盘的前提。
    #[test]
    fn test_dangling_edges_empty_for_clean_graph() {
        let nodes = vec![node("a", "entity"), node("b", "entity")]; // 干净图：节点侧也必须全合规
        let edges = vec![
            edge("a", "b", "link", None),
            edge("b", "a", "reference", Some("has_concept")),
            edge("a", "a", "link", None), // 自环：两端同一点，不算悬空
        ];
        let s = GraphData::new(nodes, edges).dangling_edges;
        assert_eq!(s.dangling, 0);
        assert!(s.sample_edge_ids.is_empty(), "干净图不得带样例");
        assert_eq!(s.total_edges, 3);
    }

    /// serde 契约与 `unresolved_*` **刻意相反**：总是序列化（含 `dangling == 0`）。
    ///
    /// 「查过了，很干净」与「这份数据没统计过」是两件事，靠「字段在不在」区分。
    #[test]
    fn test_dangling_edges_always_serialized_and_legacy_parseable() {
        let clean = GraphData::new(vec![node("a", "entity")], Vec::new()); // 干净基线：越界类型会让 unresolvedTypes 被写出来，直接撞下面第二条断言
        let s = serde_json::to_string(&clean).unwrap();
        assert!(s.contains(r#""danglingEdges""#), "空图也必须写出该字段：{s}");
        assert!(!s.contains(r#""unresolvedTypes""#), "对照组：空 unresolved 清单仍不写：{s}");

        // 旧版缓存 JSON（完全没有 danglingEdges）必须仍能解析
        let legacy: GraphData = serde_json::from_str(
            r#"{"nodes":[{"id":"a","title":"A","type":"wiki_page","tags":[],
                 "linkCount":0,"backlinkCount":0}],"edges":[]}"#,
        )
        .unwrap();
        assert_eq!(legacy.dangling_edges.total_edges, 0, "缺字段 ⇒ 0（= 未统计，而非「干净」）");
    }

    /// **融合必须重刷**：悬空边恰恰是融合步骤引入的（实体边按另一个键过滤）。
    ///
    /// 这条同时是「漏刷即等于把绊线关掉」的回归钉：把 `refresh_unresolved` 里的
    /// 第三行删掉，本测试必须变红。
    #[test]
    fn test_dangling_refresh_reports_edges_orphaned_by_fusion() {
        // 融合前：只有笔记节点，实体边尚未进来 ⇒ 干净
        let mut data = GraphData::new(
            vec![node("note:1", "note")], // 笔记节点就该是 note（D3 起为词汇表成员）
            vec![edge("note:1", "note:2", "link", None), edge("note:2", "note:1", "link", None)],
        );
        assert_eq!(data.dangling_edges.dangling, 2, "先制造一个悬空基线（note:2 不在节点集）");

        // 融合：实体节点被加入，实体边也加入 —— 端点仍然对不上（模拟跨库合并后的真实形态）
        data.nodes.push(node("entity:9", "entity"));
        data.edges.push(edge("entity:9", "entity:77", "reference", Some("in_industry")));
        data.refresh_unresolved();

        assert_eq!(data.dangling_edges.total_edges, 3);
        assert_eq!(data.dangling_edges.dangling, 3, "刷新后必须看到融合引入的悬空");
        assert_eq!(data.dangling_edges.missing_source_only, 1, "note:2 -> note:1 缺 source");
        assert_eq!(
            data.dangling_edges.missing_target_only, 2,
            "note:1 -> note:2 与 entity:9 -> entity:77"
        );
        assert_eq!(data.dangling_edges.missing_both, 0, "本轮样本里没有两端同缺的边");
    }

    /// 淘汰悬空边**不给工具栏留虚高**：`edges.len()` 降到可绘制数，而统计仍报原规模。
    ///
    /// 这条钉住的是真实缺陷的终态：`172926` 里混着 `276` 条画不出来的边，
    /// 界面按 `edges.length` 显示 ⇒ 用户读到的边数撒谎。断言里的两条
    /// （`edges.len()` 下降 + `dangling` 保留 + 恒等式）缺一不可：
    /// 只删边不保统计 = 把异常变静默；只保统计不删边 = 边数继续撒谎。
    #[test]
    fn test_retain_resolved_edges_drops_them_but_keeps_the_count() {
        let mut data = GraphData::new(
            vec![node("a", "entity"), node("b", "entity")],
            vec![
                edge("a", "b", "link", None),                               // 可绘制
                edge("a", "ghost", "link", None),                           // 只缺 target
                edge("ghost", "b", "link", None),                           // 只缺 source
                edge("ghost", "phantom", "reference", Some("has_concept")), // 两端都缺
            ],
        );
        assert_eq!(data.edges.len(), 4, "未淘汰前：统计基准 == edges.len()");
        assert_eq!(data.dangling_edges.dangling, 3);

        let dropped = data.retain_resolved_edges();

        assert_eq!(dropped, 3, "返回值必须等于被淘汰条数");
        assert_eq!(data.edges.len(), 1, "只剩两端都在节点集里的那条");
        assert_eq!(data.edges[0].source, "a");
        assert_eq!(data.edges[0].target, "b");

        // ★ 统计**不得**跟着归零 —— 否则「画不出来」这件事就此静默
        assert_eq!(data.dangling_edges.dangling, 3, "淘汰规模必须留在统计里");
        assert_eq!(data.dangling_edges.total_edges, 4, "total_edges 是淘汰前的基准");
        assert_eq!(data.dangling_edges.missing_source_only, 1);
        assert_eq!(data.dangling_edges.missing_target_only, 1);
        assert_eq!(data.dangling_edges.missing_both, 1);
        assert_eq!(data.dangling_edges.sample_edge_ids.len(), super::SAMPLE_ID_LIMIT);

        // 口径恒等式：淘汰后 `edges.len() + dangling == total_edges`
        assert_eq!(
            data.edges.len() + data.dangling_edges.dangling,
            data.dangling_edges.total_edges,
            "两种形态都必须满足 edges.len() + dangling == total_edges"
        );
    }

    /// ★ 幂等：二次淘汰**不得**把上一次的记录抹成 0。
    ///
    /// 这是最容易写坏的一条 —— 把 `if summary.dangling == 0 { return 0; }` 去掉、
    /// 改成无条件 `self.dangling_edges = summary;`，本测试必须变红。
    /// 现实后果：任何一次多余的 `refresh_unresolved`（例如融合缓存命中路径再刷一遍）
    /// 都会让界面告警消失，而脏数据一条没少。
    #[test]
    fn test_retain_resolved_edges_is_idempotent() {
        let mut data = GraphData::new(
            vec![node("a", "entity")],
            vec![edge("a", "b", "link", None), edge("a", "ghost", "link", None)],
        );

        assert_eq!(data.retain_resolved_edges(), 2);
        assert_eq!(data.dangling_edges.dangling, 2);

        // 第二次：已无可淘汰的边
        assert_eq!(data.retain_resolved_edges(), 0, "二次调用必须无操作");
        assert_eq!(data.dangling_edges.dangling, 2, "★ 记录不得被重算抹掉");
        assert_eq!(data.dangling_edges.total_edges, 2, "基准也不得被改写");
        assert_eq!(data.edges.len(), 0);
    }

    /// 干净图上不动作：`total_edges == edges.len()` 且不产生样例。
    ///
    /// 对照组存在的意义：证明恒等式在**未淘汰**形态下也成立 ——
    /// 否则读字段的人得先知道「有没有淘汰过」才能解释这两个数，那就又回到两套口径。
    #[test]
    fn test_retain_resolved_edges_noop_on_clean_graph() {
        let mut data = GraphData::new(
            vec![node("a", "entity"), node("b", "entity")],
            vec![
                edge("a", "b", "link", None),
                edge("b", "a", "reference", Some("has_concept")),
                edge("a", "a", "link", None), // 自环不算悬空
            ],
        );

        assert_eq!(data.retain_resolved_edges(), 0);
        assert_eq!(data.edges.len(), 3, "干净图一条都不许丢");
        assert_eq!(data.dangling_edges.total_edges, 3);
        assert_eq!(data.dangling_edges.dangling, 0);
        assert_eq!(
            data.edges.len() + data.dangling_edges.dangling,
            data.dangling_edges.total_edges
        );
    }

    /// 空边集：`edges.is_empty()` 的短路不得写出「无」以外的任何统计。
    #[test]
    fn test_retain_resolved_edges_on_empty_graph() {
        let mut data = GraphData::new(vec![node("a", "entity")], Vec::new());
        assert_eq!(data.retain_resolved_edges(), 0);
        assert!(data.edges.is_empty());
        assert_eq!(data.dangling_edges.total_edges, 0, "空图仍是「未统计」而非「干净」");
    }
}
