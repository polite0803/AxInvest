// SPDX-License-Identifier: AGPL-3.0-only

//! 端口公理的**真实数据**审计：用生产实现 [`validate_port_axioms`] 扫一遍存量工作流模板。
//!
//! ## 为什么需要它
//!
//! C1-升级（2026-09-14）把 4 类「结构性死链」从 warning 升为 error ⇒
//! `is_valid = errors.is_empty()` 会因此变 false、模板保存被挡。**升级的正当性取决于
//! 「存量模板是否违规」**，而这件事只能由真实数据回答 —— 库里 111 个模板可能被手工改过，
//! 「种子文件里 grep 过 `edge_cond(...)`」不足以代替。
//!
//! 它调用的是**同一个** `validate_port_axioms`（不是 SQL/脚本重写一份判据）——
//! 两份实现必然漂移，审计就失去意义。
//!
//! ## 用法
//!
//! ```text
//! # 1) 导出（口令走 scripts/pg-connect.mjs 解密，不落明文）
//! psql "$(node scripts/pg-connect.mjs url)" -tA -c \
//!   "select json_agg(json_build_object('id',id,'nodes',nodes,'edges',edges) order by id)::text \
//!    from workflow_templates" > output/tpl-dump.json
//! # 2) 审计
//! cargo run -p axagent-harness --example port_axiom_audit -- ../output/tpl-dump.json
//! ```
//!
//! ## 退出码
//!
//! | 码 | 含义 |
//! |---|---|
//! | 0 | 无 error 级违规 |
//! | 1 | 存在 error 级违规，**或输入里 0 个模板**（防「扫 0 个文件报通过」，见判据 #7） |
//! | 2 | 参数/读取/解析失败 |
//!
//! ⚠ 本 example 的 `nodes` / `edges` 在导出 JSON 里是**字符串**（DB 里是 `text` 列），
//! 所以要二次 `from_str` 解析。

use std::collections::BTreeMap;

use axagent_harness::workflow_port_axioms::{PortAxiomSeverity, validate_port_axioms};
use axagent_harness::workflow_types::{WorkflowEdge, WorkflowNode};
use serde::Deserialize;

#[derive(Deserialize)]
struct TemplateRow {
    id: String,
    nodes: String,
    edges: String,
}

fn main() {
    let Some(path) = std::env::args().nth(1) else {
        eprintln!("用法: port_axiom_audit <tpl-dump.json>");
        std::process::exit(2);
    };
    let raw = match std::fs::read_to_string(&path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("读取 {path} 失败: {e}");
            std::process::exit(2);
        },
    };
    let rows: Vec<TemplateRow> = match serde_json::from_str(&raw) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("解析 {path} 失败: {e}");
            std::process::exit(2);
        },
    };
    if rows.is_empty() {
        eprintln!("输入里没有任何模板 —— 判为失败（防「扫 0 个」被当成通过）");
        std::process::exit(1);
    }

    let mut by_kind: BTreeMap<&'static str, (usize, PortAxiomSeverity)> = BTreeMap::new();
    let mut error_templates: Vec<String> = Vec::new();
    let mut parsed_nodes = 0usize;
    let mut parse_failed: Vec<String> = Vec::new();

    for row in &rows {
        let nodes: Vec<WorkflowNode> = match serde_json::from_str(&row.nodes) {
            Ok(n) => n,
            Err(e) => {
                parse_failed.push(format!("{}: nodes 解析失败 {e}", row.id));
                continue;
            },
        };
        let edges: Vec<WorkflowEdge> = match serde_json::from_str(&row.edges) {
            Ok(e) => e,
            Err(e) => {
                parse_failed.push(format!("{}: edges 解析失败 {e}", row.id));
                continue;
            },
        };
        parsed_nodes += nodes.len();

        let violations = validate_port_axioms(&nodes, &edges);
        let mut has_error = false;
        for v in &violations {
            let sev = v.severity();
            let slot = by_kind.entry(v.kind()).or_insert((0, sev));
            slot.0 += 1;
            if sev == PortAxiomSeverity::Error {
                has_error = true;
                println!("[ERROR] {} :: {}", row.id, v.describe());
            } else {
                println!("[warn ] {} :: {}", row.id, v.describe());
            }
        }
        if has_error {
            error_templates.push(row.id.clone());
        }
    }

    println!("\n── 汇总 ──");
    println!(
        "模板数: {}｜解析成功 {}｜解析失败 {}｜节点总数 {}",
        rows.len(),
        rows.len() - parse_failed.len(),
        parse_failed.len(),
        parsed_nodes
    );
    if by_kind.is_empty() {
        println!("违规: 0（6 类公理全部零命中）");
    } else {
        println!("{:<38} {:>5}  级别", "违规类型", "数量");
        for (kind, (count, sev)) in &by_kind {
            println!("{kind:<38} {count:>5}  {sev:?}");
        }
    }
    for f in &parse_failed {
        eprintln!("解析失败: {f}");
    }

    if !error_templates.is_empty() {
        eprintln!(
            "\n存在 error 级违规的模板 {} 个 ⇒ 升为 error 会挡住这些模板的保存:",
            error_templates.len()
        );
        for id in &error_templates {
            eprintln!("  - {id}");
        }
    }
    if !parse_failed.is_empty() {
        std::process::exit(1);
    }
    std::process::exit(if error_templates.is_empty() { 0 } else { 1 });
}
