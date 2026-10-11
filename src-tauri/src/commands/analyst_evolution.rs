// SPDX-License-Identifier: AGPL-3.0-only

//! 通用节点进化命令
//!
//! 委托 `axagent_analysis_engine::node_evolution`：基于历史质量反馈统计节点
//! 常见病，产出进化状态与 Prompt/配置优化建议。
//!
//! ## 断点修复（2026-10-11）
//! 本文件在「清理 1092 文件」提交中被改成恒返回「功能已移除」的 stub，后续提交
//! 恢复了 `analysis-engine` crate 与 Cargo 依赖，却漏接本命令层 ⇒ 前端点击
//! 「触发通用节点自我进化」必进 catch，弹窗恒显「通用节点进化失败」。
//! 同时 stub 用平铺参数 `(node_type, node_id)`，而前端实际发送
//! `{ request: { nodeType, nodeId } }`（与 `analyst_feedback.rs` 2026-09-29
//! 修过的 IPC 契约坑同源）。现按调用方真实载荷接回 crate 实现。

use crate::AppState;
use axagent_agent_macro::agent_command;
use axagent_analysis_engine::node_quality::NodeType;
use axagent_analysis_engine::{NodeEvolutionStatus, evolve_node, get_node_evolution_status};
use serde::{Deserialize, Serialize};
use tauri::State;

/// 通用节点进化请求 DTO（前端键名 camelCase）
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NodeEvolveRequest {
    /// 节点类型 (analyst/debate/decision/tool/valuation/risk/other)
    pub node_type: String,
    /// 节点 ID
    pub node_id: String,
}

/// 分析师节点进化请求 DTO（向后兼容）
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AnalystEvolveRequest {
    /// 分析师节点 ID
    pub analyst_id: String,
}

/// 字符串 → NodeType。
///
/// 权威映射复用 `NodeType` 的 serde 表示（`#[serde(rename_all = "snake_case")]`），
/// 与前端 `detectNodeType` 的 7 个输出逐字对齐；未知值兜底 `Other`。
/// 不要手搓 match 重定义枚举映射（违反禁区 12「禁止重复定义」）。
fn parse_node_type(type_str: &str) -> NodeType {
    serde_json::from_value(serde_json::Value::String(type_str.to_string()))
        .unwrap_or(NodeType::Other)
}

// ── 通用节点进化命令 ──

/// 触发节点自我进化（通用版）
#[tauri::command]
#[agent_command(domain = "finance", safety = Safe, call_mode = StateInput, description = "触发节点自我进化：基于历史反馈优化 Prompt/配置")]
pub async fn evolve_node_command(
    state: State<'_, AppState>,
    request: NodeEvolveRequest,
) -> Result<NodeEvolutionStatus, String> {
    let node_type = parse_node_type(&request.node_type);
    evolve_node(state.harness.db(), &node_type, &request.node_id).await
}

/// 获取节点进化状态（通用版，不触发进化）
#[tauri::command]
#[agent_command(domain = "finance", safety = Safe, call_mode = StateInput, description = "获取节点当前进化状态")]
pub async fn get_node_evolution_status_command(
    state: State<'_, AppState>,
    request: NodeEvolveRequest,
) -> Result<NodeEvolutionStatus, String> {
    let node_type = parse_node_type(&request.node_type);
    get_node_evolution_status(state.harness.db(), &node_type, &request.node_id).await
}

// ── 分析师节点专用进化命令（向后兼容包装）──

/// 触发分析师节点自我进化
#[tauri::command]
#[agent_command(domain = "finance", safety = Safe, call_mode = StateInput, description = "触发分析师节点自我进化")]
pub async fn evolve_analyst_command(
    state: State<'_, AppState>,
    request: AnalystEvolveRequest,
) -> Result<NodeEvolutionStatus, String> {
    evolve_node(state.harness.db(), &NodeType::Analyst, &request.analyst_id).await
}

/// 获取分析师节点进化状态
#[tauri::command]
#[agent_command(domain = "finance", safety = Safe, call_mode = StateInput, description = "获取分析师节点进化状态")]
pub async fn get_analyst_evolution_status_command(
    state: State<'_, AppState>,
    request: AnalystEvolveRequest,
) -> Result<NodeEvolutionStatus, String> {
    get_node_evolution_status(state.harness.db(), &NodeType::Analyst, &request.analyst_id).await
}

// ── 单元测试 ── 追加在文件末尾（防 clippy::items_after_test_module）
#[cfg(test)]
mod analyst_evolution_ipc_tests {
    use super::*;

    /// 逐字复刻调用方（`AnalystDataQualityModal.tsx` 的
    /// `invoke("evolve_node_command", { request: { nodeType, nodeId } })`）发送的载荷。
    fn frontend_payload() -> serde_json::Value {
        serde_json::json!({
            "nodeType": "analyst",
            "nodeId": "a-technical"
        })
    }

    /// 正控：camelCase 载荷必须能进 handler（stub 时期命令签名要的是平铺
    /// `nodeType`/`nodeId`，包了一层 `request` 会在 IPC 层直接被拒）。
    #[test]
    fn evolve_request_accepts_the_camel_case_payload_the_ui_sends() {
        let req: NodeEvolveRequest = serde_json::from_value(frontend_payload())
            .expect("camelCase 载荷必须可反序列化 —— 报红说明 DTO 的 rename_all 又掉了");
        assert_eq!(req.node_type, "analyst");
        assert_eq!(req.node_id, "a-technical");
    }

    /// 反控：纯 snake_case 载荷必须失败，锁住「有人摘掉 rename_all 后前端重新进不来」
    /// 的复发路径（同 `analyst_feedback.rs` 的测试形态）。
    #[test]
    fn evolve_request_rejects_pure_snake_case_keys() {
        let snake = serde_json::json!({
            "node_type": "analyst",
            "node_id": "a-technical"
        });
        assert!(serde_json::from_value::<NodeEvolveRequest>(snake).is_err());
    }

    /// 前端 `detectNodeType` 的 7 个输出必须逐字映射到权威枚举，未知值兜底 Other ——
    /// 映射走 serde 单一来源，不允许在命令层重定义（此处只是哨兵，防止枚举改名后静默错位）。
    #[test]
    fn parse_node_type_matches_frontend_detect_outputs() {
        assert_eq!(parse_node_type("analyst"), NodeType::Analyst);
        assert_eq!(parse_node_type("debate"), NodeType::Debate);
        assert_eq!(parse_node_type("decision"), NodeType::Decision);
        assert_eq!(parse_node_type("tool"), NodeType::Tool);
        assert_eq!(parse_node_type("valuation"), NodeType::Valuation);
        assert_eq!(parse_node_type("risk"), NodeType::Risk);
        assert_eq!(parse_node_type("other"), NodeType::Other);
        // 估值/风险不得有损塌缩成 Tool：evolve_node 用 as_str() 过滤 DB 反馈，
        // 塌缩会让 valuation/risk 节点查不到自己的历史反馈。
        assert_ne!(parse_node_type("valuation"), NodeType::Tool);
        // 前后端版本错位等未知值：兜底 Other，不 panic。
        assert_eq!(parse_node_type("unknown-future-type"), NodeType::Other);
    }

    /// 出站契约：NodeEvolutionStatus 必须序列化为 camelCase（禁区 13）。
    #[test]
    fn evolution_status_serializes_camel_case_for_ipc() {
        let status = evolve_node_status_sample();
        let v = serde_json::to_value(&status).expect("序列化失败");
        assert!(v.get("totalFeedbacks").is_some(), "缺少 camelCase 字段 totalFeedbacks");
        assert!(v.get("issueRate").is_some(), "缺少 camelCase 字段 issueRate");
        assert!(v.get("lastEvolutionTime").is_some(), "缺少 camelCase 字段 lastEvolutionTime");
        assert!(v.get("evolutionCount").is_some(), "缺少 camelCase 字段 evolutionCount");
        // snake_case 旧键不得再泄漏到前端。
        assert!(v.get("total_feedbacks").is_none());
        assert_eq!(v.get("status").and_then(|s| s.as_str()), Some("no_data"));
    }

    fn evolve_node_status_sample() -> NodeEvolutionStatus {
        NodeEvolutionStatus {
            node_type: "analyst".to_string(),
            node_id: "a-technical".to_string(),
            total_feedbacks: 0,
            issue_rate: 0.0,
            consistency_metrics: std::collections::HashMap::new(),
            last_evolution_time: None,
            evolution_count: 0,
            status: "no_data".to_string(),
            suggestions: vec![],
        }
    }
}
