// SPDX-License-Identifier: AGPL-3.0-only

//! PageType —— WIKI / 知识图谱**载体层**的页面类型词汇表。
//!
//! 本文件同时是该词汇表的**本体**（2026-09-14 落地）：
//!
//! 1. **词汇表全量清单** [`ALL_PAGE_TYPES`] —— 新增枚举变体必须在此登记，覆盖测试会拦；
//! 2. **亲和度公理表** —— 此前硬编码在 `graph_dtos.rs::compute_type_affinity` 的
//!    `match (&type_a, &type_b)` 里（属「**嵌入式本体**」：有类、有不相交公理、且被
//!    `RelevanceSignal::total_score` 以权重 1.0 消费，只是埋在函数体中）。此处把它
//!    **提取成可测的声明**，判定逻辑改为查询本表 —— 这是本体**首次有真实生产消费**；
//! 3. **严格解析** [`PageType::parse_strict`] —— 区分「识别出的类型」与「不在词汇表中的字面量」。
//!
//! ## 为什么亲和度表放在这里，而不是 `domain_ontology.rs`
//!
//! `domain_ontology` 是**领域层**（域包 / 产业链 / 瓶颈三力）；本模块是**载体层**（页面形态）。
//! 两层不互相包含 —— **本体不是一个大文件，每条词汇表自己的公理表就放在它旁边**。
//! 把亲和度写进 `domain_ontology` 会造出第三处「并列词汇表」（2026-09-14 已犯过一次）。
//!
//! ## D3（2026-09-14）：词汇表对齐「实际写入值」，并成为唯一权威
//!
//! 改造前仓库里有**三套互不相同的页面类型词表**，各有各的取值集合：
//!
//! | 词表 | 位置 | 与其它两套的冲突 |
//! |---|---|---|
//! | `PageType` 8 变体 | 本文件 | 缺 `doc` / `daily` / `knowledge` / `knowledge_document` / `synced` / `log` |
//! | `is_valid_page_type` | `crates/agent/src/wiki_compiler.rs` | 允许 `log`、**拒绝 `note`**（而 `note` 是本枚举的成员） |
//! | `page_type_dir` | 同上 | `log → ""`，其余未知值落 `_ => "pages"` |
//!
//! 冲突的**代价是可计算的**：`dao/src/repo/note.rs` 把 `notes.page_type` 直接当作图谱节点的
//! `node_type`（NULL → `"note"`），所以不在本词汇表内的值会让该节点在图谱里解析失败 ⇒
//! 亲和度静默落兜底值，并在 `graph_insights.rs:136` 被算作「跨类型连接 +1.5」，与真实的
//! 跨类型连接**无法区分**。
//!
//! **处置（推荐值）：认可实际写入值 ⇒ 补变体**（而非改写入端），理由是这些取值都是
//! 真实功能的产物（日报 / 知识条目 / 文档导入 / 同步文档），改写入端会丢信息且需迁移存量；
//! 而补变体对亲和度**几乎无数值影响**（新变体一律落 A1′ 兜底 `0.3`、跨类型仍是 `0.0`）。
//!
//! | 新增变体 | 写入方（实测） |
//! |---|---|
//! | `Doc` | `src/init/opc_knowledge.rs:254`；DB 实测 `notes.page_type='doc'` **15 行** |
//! | `Daily` | `src/commands/wiki.rs:1830` |
//! | `Knowledge` | `src/commands/wiki.rs:2076`、`src/commands/knowledge_source.rs:237` |
//! | `KnowledgeDocument` | `src/init/services.rs:410` |
//! | `Synced` | `src/commands/wiki.rs:1576` |
//! | `Log` | `crates/agent/src/wiki_compiler.rs:671`（白名单允许）、`:782`（`page_type_dir` 的 `"log" => ""` 分支） |
//!
//! 同时 `is_valid_page_type` 改为委派本模块 ⇒ 三套词表收敛成一套（`note` 不再被拒）。
//! 仍**未**归一大小写与空白：那是独立的产品决策（DB 里确实存在同一个值的不同大小写形态），
//! 由 `graph_dtos::unresolved_types` 让它可见，不在此处夹带。

use serde::{Deserialize, Serialize};
use std::str::FromStr;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PageType {
    Entity,
    Concept,
    SourceSummary,
    Comparison,
    Index,
    Overview,
    Note,
    Unknown,
    // ── D3（2026-09-14）：以下 6 个是**实测存在的写入值**，此前不在词汇表内 ──
    // 出处与影响见模块文档「D3」一节；它们一律落 A1′ 兜底亲和度 `0.3`。
    /// 知识文档（OPC 知识导入：`src/init/opc_knowledge.rs:254`；DB 实测 15 行）
    Doc,
    /// 日报（`src/commands/wiki.rs:1830`）
    Daily,
    /// 知识条目（`src/commands/wiki.rs:2076`、`src/commands/knowledge_source.rs:237`）
    Knowledge,
    /// 知识库文档（`src/init/services.rs:410`）
    KnowledgeDocument,
    /// 已同步文档（`src/commands/wiki.rs:1576`）
    Synced,
    /// 编译日志页（`crates/agent/src/wiki_compiler.rs:671/782`）
    Log,
}

impl FromStr for PageType {
    type Err = String;

    /// ⚠ **宽松（历史契约，未改动）**：未识别的字面量 ⇒ `Ok(Self::Unknown)`，
    /// 调用方**无法区分**「真的是 unknown」与「拼错 / 不在词汇表」。
    ///
    /// 需要区分时用 [`PageType::parse_strict`]（返回 `None`）。
    /// 图谱侧已有严格入口：`LinkGraph::get_page_type_strict`。
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Self::parse_strict(s).unwrap_or(Self::Unknown))
    }
}

impl PageType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Entity => "entity",
            Self::Concept => "concept",
            Self::SourceSummary => "source_summary",
            Self::Comparison => "comparison",
            Self::Index => "index",
            Self::Overview => "overview",
            Self::Note => "note",
            Self::Unknown => "unknown",
            Self::Doc => "doc",
            Self::Daily => "daily",
            Self::Knowledge => "knowledge",
            Self::KnowledgeDocument => "knowledge_document",
            Self::Synced => "synced",
            Self::Log => "log",
        }
    }

    /// **严格**解析：字面量不在词汇表内 ⇒ `None`。
    ///
    /// 与 [`FromStr`] 的唯一差别就是「未识别」的处置：这里是 `None`（可被上报），
    /// 那里是 `Ok(Unknown)`（静默）。两者对**已识别**字面量的结果必须一致，
    /// 由 `test_from_str_and_parse_strict_agree_on_recognized` 锁定。
    ///
    /// 刻意**不做** trim / 大小写归一：那会改变现有行为（例如 `"Entity"` 目前 ⇒ `Unknown`），
    /// 属于独立的产品决策，不夹带在本轮里。
    pub fn parse_strict(s: &str) -> Option<Self> {
        match s {
            "entity" => Some(Self::Entity),
            "concept" => Some(Self::Concept),
            "source_summary" | "source-summary" => Some(Self::SourceSummary),
            "comparison" => Some(Self::Comparison),
            "index" => Some(Self::Index),
            "overview" => Some(Self::Overview),
            "note" => Some(Self::Note),
            "unknown" => Some(Self::Unknown),
            // D3：实测写入值（见模块文档）
            "doc" => Some(Self::Doc),
            "daily" => Some(Self::Daily),
            "knowledge" => Some(Self::Knowledge),
            "knowledge_document" => Some(Self::KnowledgeDocument),
            "synced" => Some(Self::Synced),
            "log" => Some(Self::Log),
            _ => None,
        }
    }
}

// ─────────────────────────── 本体：载体层亲和度公理 ───────────────────────────

/// 词汇表**全量清单** —— 与 `PageType` 的变体一一对应。
///
/// ⚠ 新增变体而忘记在此登记 / 忘记归入 A1 或 A1′ ⇒ `test_vocabulary_coverage_is_total` 失败。
pub const ALL_PAGE_TYPES: &[PageType] = &[
    PageType::Entity,
    PageType::Concept,
    PageType::SourceSummary,
    PageType::Comparison,
    PageType::Index,
    PageType::Overview,
    PageType::Note,
    PageType::Unknown,
    PageType::Doc,
    PageType::Daily,
    PageType::Knowledge,
    PageType::KnowledgeDocument,
    PageType::Synced,
    PageType::Log,
];

/// **公理 A1**｜同类型亲和度（显式登记，**不是兜底**）。
///
/// 取值来源：`graph_dtos.rs` 原 `compute_type_affinity` 的 `match type_a` 分支（逐值等价搬移）。
pub const SAME_TYPE_AFFINITY: &[(PageType, f64)] =
    &[(PageType::Entity, 1.0), (PageType::Concept, 1.0), (PageType::SourceSummary, 0.5)];

/// **公理 A1′**｜同类型**兜底**亲和度。
pub const SAME_TYPE_FALLBACK: f64 = 0.3;

/// 显式声明「适用公理 A1′」的类型 —— **禁止靠 `_` 静默兜底**。
///
/// A1 的键 ∪ 本表 必须 == [`ALL_PAGE_TYPES`]（不重不漏），否则新增变体会被静默地
/// 混进兜底值、无人察觉。
pub const SAME_TYPE_FALLBACK_TYPES: &[PageType] = &[
    PageType::Comparison,
    PageType::Index,
    PageType::Overview,
    PageType::Note,
    PageType::Unknown,
    // D3 新增的 6 个实测写入值同样落兜底值 —— 与它们此前「解析失败 ⇒ Unknown ⇒ 0.3」一致
    PageType::Doc,
    PageType::Daily,
    PageType::Knowledge,
    PageType::KnowledgeDocument,
    PageType::Synced,
    PageType::Log,
];

/// **公理 A2**｜跨类型亲和度（**对称**，只登记一个方向；查询函数双向查）。
pub const CROSS_TYPE_AFFINITY: &[(PageType, PageType, f64)] =
    &[(PageType::Entity, PageType::Concept, 0.5)];

/// **公理 A2′**｜**不相交公理**：除 A2 登记的类型对外，其余跨类型对亲和度 = 0 ——
/// 即「跨类关系默认不被认为是相关的」。
pub const CROSS_TYPE_FORBIDDEN: f64 = 0.0;

/// 亲和度**值域公理**：A1 / A1′ / A2 / A2′ 的取值必须落在 `[0, 1]`。
///
/// 该值域有一个下游依赖：`RelevanceSignalExt::total_score` 以权重 `1.0` 累加本值，
/// `normalized_total` 除以常数 `10.0`（= 3.0 + 4.0 + 1.5 + 1.0）——
/// 亲和度 > 1 会让归一化分数越界，故此处显式声明上界。
pub const AFFINITY_RANGE: (f64, f64) = (0.0, 1.0);

/// 查询：同类型亲和度（A1，未登记则落 A1′）。
pub fn same_type_affinity(t: PageType) -> f64 {
    SAME_TYPE_AFFINITY
        .iter()
        .find(|(declared, _)| *declared == t)
        .map(|(_, v)| *v)
        .unwrap_or(SAME_TYPE_FALLBACK)
}

/// 查询：跨类型亲和度（A2，未登记则落 A2′）。**对称**，两个方向都查。
pub fn cross_type_affinity(a: PageType, b: PageType) -> f64 {
    CROSS_TYPE_AFFINITY
        .iter()
        .find(|(x, y, _)| (*x == a && *y == b) || (*x == b && *y == a))
        .map(|(_, _, v)| *v)
        .unwrap_or(CROSS_TYPE_FORBIDDEN)
}

// ─────────────────────────── 纯校验器（供负向对照） ───────────────────────────

/// 公理表的**值域 + 无重复**校验（纯函数，便于负向对照）。
pub fn same_type_table_ok(entries: &[(PageType, f64)]) -> bool {
    let in_range = |v: f64| v >= AFFINITY_RANGE.0 && v <= AFFINITY_RANGE.1 && !v.is_nan();
    let mut seen: Vec<PageType> = Vec::new();
    for (t, v) in entries {
        if !in_range(*v) || seen.contains(t) {
            return false;
        }
        seen.push(*t);
    }
    true
}

/// 跨类型公理表的**值域 + 无重复 + 非自反对**校验。
///
/// 「非自反对」= 同类型对不得写进跨类型表（否则 A1 与 A2 对同一输入给出两个答案）。
pub fn cross_type_table_ok(entries: &[(PageType, PageType, f64)]) -> bool {
    let in_range = |v: f64| v >= AFFINITY_RANGE.0 && v <= AFFINITY_RANGE.1 && !v.is_nan();
    let mut seen: Vec<(PageType, PageType)> = Vec::new();
    for (x, y, v) in entries {
        if !in_range(*v) || x == y {
            return false;
        }
        let unordered = (*x, *y);
        let mirrored = (*y, *x);
        if seen.contains(&unordered) || seen.contains(&mirrored) {
            return false;
        }
        seen.push(unordered);
    }
    true
}

/// **覆盖校验**：`declared` ∪ `fallback` 必须恰好等于 `all`（不重、不漏）。
///
/// 这是「新增变体漏登记」的机器判据 —— 与 L1 门禁同型的**覆盖制**（而非黑名单制）。
pub fn vocabulary_coverage_is_total(
    all: &[PageType],
    declared: &[PageType],
    fallback: &[PageType],
) -> bool {
    if all.is_empty() {
        return false;
    }
    // declared 与 fallback 不得相交
    if declared.iter().any(|t| fallback.contains(t)) {
        return false;
    }
    let mut union: Vec<PageType> =
        declared.iter().copied().chain(fallback.iter().copied()).collect();
    for t in all {
        let n = union.iter().filter(|u| *u == t).count();
        if n != 1 {
            return false;
        }
        union.retain(|u| u != t);
    }
    // all 之外的幽灵项
    union.is_empty()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    fn declared_types() -> Vec<PageType> {
        SAME_TYPE_AFFINITY.iter().map(|(t, _)| *t).collect()
    }

    // ── 正向：公理表自洽 ──

    #[test]
    fn test_vocabulary_coverage_is_total() {
        assert!(
            vocabulary_coverage_is_total(
                ALL_PAGE_TYPES,
                &declared_types(),
                SAME_TYPE_FALLBACK_TYPES
            ),
            "A1 的键 ∪ A1′ 表 必须恰好覆盖 ALL_PAGE_TYPES（不重不漏）"
        );
    }

    #[test]
    fn test_affinity_tables_pass_validators() {
        assert!(same_type_table_ok(SAME_TYPE_AFFINITY));
        assert!(cross_type_table_ok(CROSS_TYPE_AFFINITY));
    }

    #[test]
    fn test_same_type_affinity_values_are_locked() {
        // 逐值锁定 = 与旧硬编码 match 的等价基准（改动会立即红）
        let expected = [
            (PageType::Entity, 1.0),
            (PageType::Concept, 1.0),
            (PageType::SourceSummary, 0.5),
            (PageType::Comparison, 0.3),
            (PageType::Index, 0.3),
            (PageType::Overview, 0.3),
            (PageType::Note, 0.3),
            (PageType::Unknown, 0.3),
            // D3：新增变体必须是兜底值（等价于它们此前「解析失败 ⇒ Unknown ⇒ 0.3」）
            (PageType::Doc, 0.3),
            (PageType::Daily, 0.3),
            (PageType::Knowledge, 0.3),
            (PageType::KnowledgeDocument, 0.3),
            (PageType::Synced, 0.3),
            (PageType::Log, 0.3),
        ];
        for (t, want) in expected {
            assert_eq!(same_type_affinity(t), want, "同类型亲和度漂移：{t:?}");
        }
    }

    #[test]
    fn test_as_str_is_unique_and_roundtrips() {
        let mut seen = HashSet::new();
        for t in ALL_PAGE_TYPES {
            let s = t.as_str();
            assert!(seen.insert(s), "as_str 出现重复字面量：{s}");
            assert_eq!(PageType::parse_strict(s), Some(*t), "as_str 与 parse_strict 不互逆：{s}");
        }
    }

    #[test]
    fn test_cross_type_affinity_is_symmetric() {
        for a in ALL_PAGE_TYPES {
            for b in ALL_PAGE_TYPES {
                assert_eq!(
                    cross_type_affinity(*a, *b),
                    cross_type_affinity(*b, *a),
                    "跨类型公理不对称：{a:?} / {b:?}"
                );
            }
        }
    }

    #[test]
    fn test_cross_type_only_allows_entity_concept() {
        // 不相交公理 A2′：除 Entity↔Concept 外全部为 0（含同类型对，同类型不走本函数）
        for a in ALL_PAGE_TYPES {
            for b in ALL_PAGE_TYPES {
                if a == b {
                    continue;
                }
                let allowed = (*a == PageType::Entity && *b == PageType::Concept)
                    || (*a == PageType::Concept && *b == PageType::Entity);
                let v = cross_type_affinity(*a, *b);
                if allowed {
                    assert_eq!(v, 0.5, "被允许的跨类型对取值漂移");
                } else {
                    assert_eq!(v, 0.0, "不相交公理被破坏：{a:?} / {b:?} 应为 0");
                }
            }
        }
    }

    #[test]
    fn test_from_str_and_parse_strict_agree_on_recognized() {
        // 两个入口的差异**只允许**出现在「未识别」这一种情况
        for t in ALL_PAGE_TYPES {
            assert_eq!(t.as_str().parse::<PageType>().unwrap(), *t);
        }
        assert_eq!("source-summary".parse::<PageType>().unwrap(), PageType::SourceSummary);
        assert_eq!(PageType::parse_strict("source-summary"), Some(PageType::SourceSummary));
    }

    // ── 负向对照：每条校验器都必须能红 ──

    #[test]
    fn test_parse_strict_rejects_out_of_vocabulary() {
        // 真正不在词汇表内的字面量（**不是** D3 登记的那 6 个实测写入值 ——
        // 它们已升格为词汇表成员，见 test_observed_write_values_are_in_vocabulary）
        for raw in ["", "invalid", "unknown_type", "Entity ", "entities", "wiki_page"] {
            assert_eq!(PageType::parse_strict(raw), None, "「{raw}」不应被识别为词汇表成员");
        }
        // 宽松入口把未识别值全部吞成 Unknown —— 这正是「静默降级」的入口
        assert_eq!("invalid".parse::<PageType>().unwrap(), PageType::Unknown);
        assert_eq!("wiki_page".parse::<PageType>().unwrap(), PageType::Unknown);
        // 大小写 / 空白**刻意**不归一（行为未变，避免夹带决策）
        assert_eq!(PageType::parse_strict("Entity"), None);
        assert_eq!(PageType::parse_strict(" note"), None);
    }

    /// D3 回归锁：**实际写入值必须在词汇表内**。
    ///
    /// 这些取值来自真实写入点（文件:行见模块文档 D3 表）。若有人再把某个写入值踢出词汇表，
    /// 本测试会红 —— 而那正是「图谱节点解析失败 ⇒ 亲和度静默落兜底」的入口。
    #[test]
    fn test_observed_write_values_are_in_vocabulary() {
        // 写入点：src/commands/wiki.rs:1830 / src/commands/wiki.rs:2076 / knowledge_source.rs:237 /
        //        opc_knowledge.rs:254 / src/init/services.rs:410 / src/commands/wiki.rs:1576 / wiki_compiler.rs:671
        for raw in ["doc", "daily", "knowledge", "knowledge_document", "synced", "log", "note"] {
            assert!(
                PageType::parse_strict(raw).is_some(),
                "「{raw}」是实测写入值，必须留在词汇表内（否则图谱侧解析失败、亲和度静默降级）"
            );
            assert_ne!(
                PageType::parse_strict(raw),
                Some(PageType::Unknown),
                "「{raw}」不得只映射成 Unknown —— 那就等于没登记"
            );
        }
        // 反向对照：本测试确实有判别力（未登记值仍必须为 None）
        assert_eq!(PageType::parse_strict("not_a_page_type"), None);
    }

    #[test]
    fn test_coverage_validator_has_negative_control() {
        let mut missing = declared_types();
        missing.retain(|t| *t != PageType::Concept);
        assert!(
            !vocabulary_coverage_is_total(ALL_PAGE_TYPES, &missing, SAME_TYPE_FALLBACK_TYPES),
            "漏登记的类型必须被判定为不覆盖"
        );

        let overlapping = declared_types();
        let mut fallback = SAME_TYPE_FALLBACK_TYPES.to_vec();
        fallback.push(PageType::Entity);
        assert!(
            !vocabulary_coverage_is_total(ALL_PAGE_TYPES, &overlapping, &fallback),
            "A1 与 A1′ 相交必须被判为不覆盖（同一输入两个答案）"
        );

        assert!(
            !vocabulary_coverage_is_total(ALL_PAGE_TYPES, &declared_types(), &[]),
            "fallback 为空必须红"
        );
        assert!(!vocabulary_coverage_is_total(&[], &[], &[]), "空清单必须红");
    }

    #[test]
    fn test_same_type_table_validator_has_negative_control() {
        assert!(!same_type_table_ok(&[(PageType::Entity, 1.5)]), "越上界必须红");
        assert!(!same_type_table_ok(&[(PageType::Entity, -0.1)]), "越下界必须红");
        assert!(!same_type_table_ok(&[(PageType::Entity, f64::NAN)]), "NaN 必须红");
        assert!(
            !same_type_table_ok(&[(PageType::Entity, 1.0), (PageType::Entity, 0.5)]),
            "重复登记必须红"
        );
        assert!(same_type_table_ok(&[]), "空表本身合法（由覆盖校验负责抓漏）");
    }

    #[test]
    fn test_cross_type_table_validator_has_negative_control() {
        assert!(
            !cross_type_table_ok(&[(PageType::Entity, PageType::Entity, 0.5)]),
            "同类型对不得进跨类型表（与 A1 冲突）"
        );
        assert!(
            !cross_type_table_ok(&[
                (PageType::Entity, PageType::Concept, 0.5),
                (PageType::Concept, PageType::Entity, 0.5)
            ]),
            "镜像重复登记必须红"
        );
        assert!(!cross_type_table_ok(&[(PageType::Entity, PageType::Concept, 2.0)]), "越界必须红");
    }
}
