// SPDX-License-Identifier: AGPL-3.0-only

//! 端口公理写路径门禁的**端到端实证**（连真库，2026-09-14 P0）。
//!
//! # 为什么需要它
//!
//! 单测只能证明 `enforce_port_axioms` 这个**纯函数**的行为。P0 的诉求是
//! 「门禁真的接在写路径上、真的会挡住落库」—— 这只能由**真库 + 真写入函数**回答：
//! 反序列化库里那份真实的非法形态，喂给 `update_workflow_template`，看它是否 Err。
//!
//! 本 example 的每一步都调用**生产实现**（`repo::*` / `is_port_axiom_clean`），
//! 不重写任何判据 —— 两份实现必然漂移，审计就失去意义。
//!
//! # 用法
//!
//! ```text
//! cargo run -p axagent-dao --example port_axiom_gate_probe -- "$(node ../scripts/pg-connect.mjs url)"
//! ```
//!
//! # 它做四件事
//!
//! | # | 动作 | 期望 | 是否写库 |
//! |---|---|---|---|
//! | ① | 只读扫全表，列出端口公理非法的模板 | 打印清单 | 否 |
//! | ② | 把**库里那份非法形态**喂给 `update_workflow_template` | `Err`（被挡） | **否**（门禁在 `find_by_id` 之前） |
//! | ③ | 合法形态 + **不存在的 id** 喂给同一个函数 | `Ok(false)`（过了门禁，没找到行） | 否 |
//! | ④ | `reset_port_axiom_illegal_versions` 跑两次 | 第一次列出归零项，第二次为空 | 是（`version` 归零） |
//!
//! ②/③ 是刻意配对的**正负对照**：只有负对照时，「门禁恒报错」也能通过；
//! 只有正对照时，「门禁没接」也能通过。
//!
//! # 退出码
//!
//! 0 = 四步全部符合预期；1 = 有反例；2 = 参数/连库/查询失败。

use axagent_dao::db::create_pool;
use axagent_dao::repo::workflow_template as repo;
use axagent_entities::workflow_template;
use axagent_harness::workflow_types::{
    EdgeType, Position, RetryConfig, SwitchCase, SwitchNode, SwitchNodeConfig, WorkflowEdge,
    WorkflowNode, WorkflowNodeBase,
};
use sea_orm::EntityTrait;

fn base(id: &str) -> WorkflowNodeBase {
    WorkflowNodeBase {
        id: id.to_string(),
        title: format!("node_{id}"),
        description: None,
        position: Position::default(),
        retry: RetryConfig::default(),
        timeout: None,
        enabled: true,
        parent_id: None,
        continue_on_fail: false,
        compensation: None,
    }
}

fn switch_node(id: &str, labels: &[&str], default_case: Option<&str>) -> WorkflowNode {
    WorkflowNode::Switch(SwitchNode {
        base: base(id),
        config: SwitchNodeConfig {
            input_var: "x".into(),
            cases: labels
                .iter()
                .map(|l| SwitchCase { value: l.to_string(), label: l.to_string() })
                .collect(),
            default_case: default_case.map(str::to_string),
            match_mode: "exact".into(),
            use_llm: None,
            llm_prompt: None,
            llm_model: None,
            output_var: String::new(),
        },
    })
}

fn edge(id: &str, source: &str, handle: Option<&str>) -> WorkflowEdge {
    WorkflowEdge {
        id: id.to_string(),
        source: source.to_string(),
        source_handle: handle.map(str::to_string),
        target: "a-target".to_string(),
        target_handle: None,
        edge_type: EdgeType::Direct,
        label: None,
    }
}

/// 合法样本：具名出边 + `default_case` 也是具名分支（现网模板的形态）。
fn legal_graph() -> (Vec<WorkflowNode>, Vec<WorkflowEdge>) {
    let nodes = vec![switch_node("s", &["go", "no-go"], Some("no-go"))];
    let edges = vec![edge("e-go", "s", Some("go")), edge("e-nogo", "s", Some("no-go"))];
    (nodes, edges)
}

/// 非法样本（构造）：`handle` 落不到任何 case ⇒ 死边（Error 级）。
fn illegal_graph() -> (Vec<WorkflowNode>, Vec<WorkflowEdge>) {
    let nodes = vec![switch_node("s", &["go"], None)];
    let edges = vec![edge("e-go", "s", Some("go")), edge("e-typo", "s", Some("g0"))];
    (nodes, edges)
}

#[tokio::main]
async fn main() {
    let Some(url) = std::env::args().nth(1) else {
        eprintln!("用法: port_axiom_gate_probe <postgres-url>");
        eprintln!("例:   port_axiom_gate_probe \"$(node scripts/pg-connect.mjs url)\"");
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

    // ── ① 只读扫全表 ──
    let rows = match workflow_template::Entity::find().all(db).await {
        Ok(r) => r,
        Err(e) => {
            eprintln!("查 workflow_templates 失败: {e}");
            std::process::exit(2);
        },
    };
    if rows.is_empty() {
        eprintln!("库里 0 个模板 —— 判为失败（防「扫 0 个」被当成通过）");
        std::process::exit(2);
    }
    let illegal: Vec<&workflow_template::Model> =
        rows.iter().filter(|r| !repo::is_port_axiom_clean(r)).collect();
    println!("① 只读扫描");
    println!("   模板总数 {}｜端口公理非法 {}", rows.len(), illegal.len());
    for r in &illegal {
        println!("     - {} (version={})", r.id, r.version);
    }

    // ── ② 负对照：库里的真实非法形态，走 DAO 写路径 ⇒ 必须 Err ──
    println!("\n② 负对照：把库里的非法形态喂给 update_workflow_template");
    match illegal.first() {
        None => println!("   （库里没有非法模板，跳过 —— 用构造样本代替）"),
        Some(row) => {
            let nodes: Vec<WorkflowNode> = serde_json::from_str(&row.nodes).expect("nodes 解析");
            let edges: Vec<WorkflowEdge> = serde_json::from_str(&row.edges).expect("edges 解析");
            let res = repo::update_workflow_template(
                db,
                &row.id,
                "NEGCTL".into(),
                None,
                "x".into(),
                vec![],
                None,
                nodes,
                edges,
                None,
                None,
                vec![],
                None,
                None,
            )
            .await;
            match res {
                Err(e) => println!("   [OK] 被门禁挡下（且门禁在 find_by_id 之前 ⇒ 未写库）:\n{e}"),
                Ok(v) => {
                    println!("   [FAIL] 非法形态竟然通过了门禁（返回值 {v}）");
                    failures += 1;
                },
            }
        },
    }

    // 用**构造的**非法样本再打一次，确保不依赖库里恰好有非法行
    {
        let (nodes, edges) = illegal_graph();
        let res = repo::update_workflow_template(
            db,
            "__probe_nonexistent__",
            "NEGCTL".into(),
            None,
            "x".into(),
            vec![],
            None,
            nodes,
            edges,
            None,
            None,
            vec![],
            None,
            None,
        )
        .await;
        match res {
            Err(e) => println!("   [OK] 构造的死边样本同样被挡下: {e}"),
            Ok(v) => {
                println!("   [FAIL] 构造的死边样本通过了门禁（返回值 {v}）");
                failures += 1;
            },
        }
    }

    // ── ③ 正对照：合法形态 + 不存在的 id ⇒ 过门禁、Ok(false)、不写库 ──
    println!("\n③ 正对照：合法形态 + 不存在的 id");
    {
        let (nodes, edges) = legal_graph();
        let res = repo::update_workflow_template(
            db,
            "__probe_nonexistent__",
            "PROBE".into(),
            None,
            "x".into(),
            vec![],
            None,
            nodes,
            edges,
            None,
            None,
            vec![],
            None,
            None,
        )
        .await;
        match res {
            Ok(false) => println!("   [OK] 通过门禁且未找到该 id ⇒ 未写库"),
            Ok(true) => {
                println!("   [FAIL] 竟然更新了某一行（不该存在这个 id）");
                failures += 1;
            },
            Err(e) => {
                println!("   [FAIL] 合法形态被误拦: {e}");
                failures += 1;
            },
        }
    }

    // ── ④ 存量自愈 + 幂等 ──
    println!("\n④ 存量自愈（把非法模板的 version 归零，等种子重建）");
    match repo::reset_port_axiom_illegal_versions(db).await {
        Ok(ids) => println!("   第一次: {ids:?}"),
        Err(e) => {
            println!("   [FAIL] 自愈失败: {e}");
            failures += 1;
        },
    }
    match repo::reset_port_axiom_illegal_versions(db).await {
        Ok(ids) if ids.is_empty() => println!("   第二次: 空 ⇒ 幂等 OK"),
        Ok(ids) => {
            println!("   [FAIL] 第二次仍有归零项 {ids:?} ⇒ 非幂等");
            failures += 1;
        },
        Err(e) => {
            println!("   [FAIL] 自愈第二次失败: {e}");
            failures += 1;
        },
    }

    println!(
        "\n{}",
        if failures == 0 {
            "全部符合预期"
        } else {
            "存在反例（见上）"
        }
    );
    std::process::exit(if failures == 0 { 0 } else { 1 });
}
