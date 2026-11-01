// SPDX-License-Identifier: AGPL-3.0-only

//! Wiki 图谱**边侧关系类型保真**的端到端实证（连真库，2026-09-14 P2-b）。
//!
//! # 为什么需要它
//!
//! P2-b 的诉求是「边侧的降级不再静默」。而实测发现真正的缺陷不是「前端不认」，
//! 而是 `get_knowledge_graph_edges_for_wiki` 把 `relation_type` **读出来又扔掉**
//! （`edge_type` 恒为常量 `"reference"`）—— 于是信息在**出口**就已丢失。
//!
//! 「丢掉多少」这件事只能由**真库 + 真生产函数**回答：单测里的 4 条边说明不了
//! 112937 行里有多少个类型被塌成一个值。本 example 的每一步都调用**生产实现**
//! （`repo::get_knowledge_graph_edges_for_wiki` / `GraphData::new` /
//! `collect_unresolved_relation_stats`），不重写任何判据。
//!
//! # 用法
//!
//! ```text
//! cargo run -p axagent-dao --example wiki_edge_relation_probe -- "$(node ../scripts/pg-connect.mjs url)"
//! ```
//!
//! # 它做四件事
//!
//! | # | 动作 | 期望 |
//! |---|---|---|
//! | ① | 每个知识库跑**生产函数**，统计关系类型保真度 | 只读；打印 distinct / None / top |
//! | ② | **反事实复现**：把同一批边按**旧实现**（常量 `"reference"`）重放 | distinct `type` = 1、distinct `relationType` = 0 —— 即「信息量归零」 |
//! | ③ | `GraphData::new` 算 `unresolved_relations` | 空（中文关系值按形态放行 —— 不是刷屏器） |
//! | ④ | 在**同一批边上**注入一个真未知标签 | 非空且计数正确 —— 绊线在真实数据上**能响** |
//!
//! ③/④ 是刻意的**正负对照**：只有 ③ 时「函数恒返回空」也能通过；
//! 只有 ④ 时「函数恒返回非空」也能通过。
//!
//! # 退出码
//!
//! 0 = 全部符合预期；1 = 有反例；2 = 参数/连库/查询失败。

use std::collections::BTreeMap;

use axagent_dao::db::create_pool;
use axagent_dao::repo::knowledge_graph as repo;
use axagent_entities::knowledge_relations;
use axagent_harness::graph_dtos::{GraphData, GraphEdge};
use sea_orm::{EntityTrait, QuerySelect};

/// 边上的关系类型分布：`raw -> 出现次数`。
fn relation_histogram(edges: &[GraphEdge]) -> BTreeMap<String, usize> {
    let mut hist: BTreeMap<String, usize> = BTreeMap::new();
    for e in edges {
        let key = e.relation_type.clone().unwrap_or_else(|| "<none>".to_string());
        *hist.entry(key).or_insert(0) += 1;
    }
    hist
}

/// 取前 `n` 项（按次数降序、同次数按名字升序 —— 与生产聚合的排序规则一致）。
fn top_n(hist: &BTreeMap<String, usize>, n: usize) -> Vec<(String, usize)> {
    let mut v: Vec<(String, usize)> = hist.iter().map(|(k, c)| (k.clone(), *c)).collect();
    v.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    v.truncate(n);
    v
}

#[tokio::main]
async fn main() {
    let Some(url) = std::env::args().nth(1) else {
        eprintln!("用法: wiki_edge_relation_probe <postgres-url>");
        eprintln!("例:   wiki_edge_relation_probe \"$(node scripts/pg-connect.mjs url)\"");
        std::process::exit(2);
    };
    let handle = match create_pool(&url).await {
        Ok(h) => h,
        Err(e) => {
            eprintln!("连库失败: {e}");
            std::process::exit(2);
        },
    };
    let db = &handle.conn;
    let mut failures = 0usize;

    // 实测库里有 2 个知识库；这里**动态**取（不硬编码 kb_id —— 硬编码会让探针
    // 在换库后静默变成「扫 0 个」）。
    let kb_ids: Vec<String> = match knowledge_relations::Entity::find()
        .select_only()
        .column(knowledge_relations::Column::KnowledgeBaseId)
        .distinct()
        .into_tuple::<String>()
        .all(db)
        .await
    {
        Ok(v) => v,
        Err(e) => {
            eprintln!("查知识库清单失败: {e}");
            std::process::exit(2);
        },
    };
    if kb_ids.is_empty() {
        eprintln!("knowledge_relations 里 0 个知识库 —— 判为失败（防「扫 0 个」被当成通过）");
        std::process::exit(2);
    }

    let mut grand_total = 0usize;
    let mut grand_types_after: BTreeMap<String, usize> = BTreeMap::new();
    let mut grand_types_before: BTreeMap<String, usize> = BTreeMap::new();
    let mut blank_rows = 0usize;

    for kb in &kb_ids {
        // ── ① 生产函数（改造后）──
        let edges = match repo::get_knowledge_graph_edges_for_wiki(db, kb).await {
            Ok(e) => e,
            Err(e) => {
                eprintln!("get_knowledge_graph_edges_for_wiki({kb}) 失败: {e}");
                std::process::exit(2);
            },
        };
        if edges.is_empty() {
            println!("① kb={kb}｜0 条边（跳过）");
            continue;
        }
        let hist_after = relation_histogram(&edges);
        let none_count = hist_after.get("<none>").copied().unwrap_or(0);
        blank_rows += none_count;

        // ── ② 反事实：旧实现（改造前该处直接把 `edge_type` 写死成常量
        //      `"reference".to_string()`，关系信息被丢弃；现状见
        //      `crates/dao/src/repo/knowledge_graph.rs:1794`）──
        //     同一批 source/target，只把关系信息抹掉。
        let legacy: Vec<GraphEdge> = edges
            .iter()
            .map(|e| GraphEdge::structural(e.source.clone(), e.target.clone(), "reference"))
            .collect();
        let hist_before = relation_histogram(&legacy);

        println!("① kb={kb}｜边 {} 条", edges.len());
        println!(
            "   改造后：distinct relationType = {}｜type 分布 = {}",
            hist_after.len() - usize::from(none_count > 0),
            {
                let mut t: BTreeMap<&str, usize> = BTreeMap::new();
                for e in &edges {
                    *t.entry(e.edge_type.as_str()).or_insert(0) += 1;
                }
                format!("{t:?}")
            }
        );
        println!(
            "   反事实（旧实现）：distinct relationType = {}｜distinct type = {}  ← 信息量归零",
            hist_before.len() - usize::from(hist_before.contains_key("<none>")),
            {
                let mut t: BTreeMap<&str, usize> = BTreeMap::new();
                for e in &legacy {
                    *t.entry(e.edge_type.as_str()).or_insert(0) += 1;
                }
                t.len()
            }
        );
        let top = top_n(&hist_after, 5);
        println!(
            "   top5：{}",
            top.iter().map(|(k, c)| format!("{k}×{c}")).collect::<Vec<_>>().join("｜")
        );

        // 期望：改造后 distinct ≥ 1；旧实现恒 = 1 个 type 且无关系类型
        if hist_after.is_empty() {
            eprintln!("   ✗ 改造后 relationType 分布为空 —— 字段没接上");
            failures += 1;
        }
        let legacy_types: std::collections::BTreeSet<&str> =
            legacy.iter().map(|e| e.edge_type.as_str()).collect();
        if legacy_types.len() != 1 || legacy_types.iter().next() != Some(&"reference") {
            eprintln!("   ✗ 反事实复现失败：旧实现应恒为单个 `reference` type");
            failures += 1;
        }
        // 旧实现的形态是「**全部**边都没有关系类型」⇒ 直方图恰好只有 `<none>` 一个键。
        // ⚠ 这里曾写成「不许含 `<none>`」—— 方向反了：含 `<none>` 正是旧实现的**特征**，
        // 断言写反会让每一步都误报失败（本探针第一版就是如此）。
        if !(hist_before.len() == 1 && hist_before.contains_key("<none>")) {
            eprintln!(
                "   ✗ 反事实复现失败：旧实现应恒为「无关系类型」单一形态，实得 {hist_before:?}"
            );
            failures += 1;
        }

        grand_total += edges.len();
        for (k, v) in hist_after {
            *grand_types_after.entry(k).or_insert(0) += v;
        }
        for (k, v) in hist_before {
            *grand_types_before.entry(k).or_insert(0) += v;
        }
    }

    println!(
        "\n合计：边 {grand_total} 条｜改造后 distinct 标签 {} 个｜反事实 distinct 标签 {} 个｜blank(关系类型缺失) {blank_rows} 条",
        grand_types_after.len() - usize::from(grand_types_after.contains_key("<none>")),
        grand_types_before.len() - usize::from(grand_types_before.contains_key("<none>")),
    );

    // ── ③ 生产观测（`GraphData::new`）：真数据上必须为空 ──
    let mut all_edges: Vec<GraphEdge> = Vec::new();
    for kb in &kb_ids {
        if let Ok(mut e) = repo::get_knowledge_graph_edges_for_wiki(db, kb).await {
            all_edges.append(&mut e);
        }
    }
    // 补一条笔记链接（结构性边，不带关系类型）—— 让样本同时含两侧
    all_edges.push(GraphEdge::structural("note:a", "note:b", "link"));

    let data = GraphData::new(Vec::new(), all_edges.clone());
    println!(
        "③ GraphData::new：unresolved_relations = {} 项（期望 0 —— 中文值按形态放行，不是刷屏器）",
        data.unresolved_relations.len()
    );
    if !data.unresolved_relations.is_empty() {
        eprintln!(
            "   ✗ 真数据上不该产生未识别清单（会被当成刷屏器）：{:?}",
            data.unresolved_relations
        );
        failures += 1;
    }

    // ── ④ 正对照：在**同一批边**上注入一个真未知标签，绊线必须响 ──
    let mut with_bogus = all_edges.clone();
    with_bogus.push(GraphEdge::relation("entity:x", "entity:y", "definitely_not_declared"));
    with_bogus.push(GraphEdge::relation("entity:x", "entity:z", "definitely_not_declared"));
    let bogus = GraphData::new(Vec::new(), with_bogus);
    println!(
        "④ 注入未知标签后：unresolved_relations = {} 项（期望 1 项 / count=2）",
        bogus.unresolved_relations.len()
    );
    match bogus.unresolved_relations.first() {
        Some(s)
            if bogus.unresolved_relations.len() == 1
                && s.raw_type == "definitely_not_declared"
                && s.count == 2 => {},
        other => {
            eprintln!("   ✗ 绊线没按预期响：{other:?}");
            failures += 1;
        },
    }

    if failures == 0 {
        println!("\n✅ 四步全部符合预期");
    } else {
        println!("\n❌ {failures} 项不符预期");
    }
    std::process::exit(if failures == 0 { 0 } else { 1 });
}
