// SPDX-License-Identifier: AGPL-3.0-only

//! 通用节点质量反馈命令
//!
//! 支持所有节点类型（分析师/辩论/决策/工具/估值/风险）的质量反馈存储和查询。
//!
//! ## IPC 契约（2026-09-29 查明并修）
//! 本文件的命令此前有**两道独立断点**，上报从未进过 handler（真库 `analyst_feedbacks` 实测 0 行）：
//! 1. **参数名**：命令签名用 `req`，而调用方传 `request` ⇒ Tauri 在 IPC 层就报
//!    `missing required key request`（命令参数名 = JS 侧键名，见 AGENTS.md 禁区 13）；
//! 2. **字段名**：六个 DTO 缺 `#[serde(rename_all = "camelCase")]` ⇒ 即便参数名对了，
//!    camelCase 的载荷也满足不了必填的 snake_case 字段。
//!
//! 而调用点是 fire-and-forget + `.catch()` ⇒ 两道失败都被吞成一条 console.warn，界面毫无痕迹。
//!
//! `check-serde-annotations` 只扫 `harness/` 与 `commands/`，本文件**在扫描面内却一直报红**
//! （该门长期红且不在 CI）—— 所以键名契约由文件末尾的 serde 往返测试逐键锁住，
//! 而不是依赖目录式扫描脚本（同 `reflection_stats::hitrate_dto_serializes_camel_case_for_ipc`）。

use crate::AppState;
use axagent_agent_macro::agent_command;
use axagent_entities::analyst_feedback;
use axagent_entities::analyst_feedback::Entity as AnalystFeedback;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, QueryOrder, QuerySelect, Set,
};
use serde::{Deserialize, Serialize};
use tauri::State;

/// 通用节点质量反馈的请求 DTO
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SaveNodeFeedbackRequest {
    /// 节点类型 (analyst/debate/decision/tool/valuation/risk/other)
    pub node_type: String,
    /// 节点 ID
    pub node_id: String,
    /// 报告 ID
    pub report_id: String,
    /// 股票代码
    pub stock_code: String,
    /// 执行 ID (workflow run id)
    pub execution_id: String,
    /// 质量评分 (0-100)
    pub quality_score: i32,
    /// 评分等级 (A/B/C/D/F)
    pub grade: String,
    /// 检测到的问题数量
    pub issue_count: i32,
    /// 检测到的警告数量
    pub warning_count: i32,
    /// 检测到的良好项数量
    pub good_count: i32,
    /// 详细检查结果 (JSON)
    pub checks_json: String,
    /// 通用质量指标 (JSON) - 存储节点特有的质量指标
    pub quality_metrics_json: String,
}

/// 节点反馈摘要（用于前端列表展示）
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NodeFeedbackSummary {
    pub id: String,
    pub node_type: String,
    pub node_id: String,
    pub quality_score: i32,
    pub grade: String,
    pub issue_count: i32,
    pub warning_count: i32,
    pub created_at: String,
}

/// 获取节点反馈列表的请求 DTO
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GetNodeFeedbacksRequest {
    /// 节点类型 (可选，用于过滤)
    pub node_type: Option<String>,
    /// 节点 ID
    pub node_id: Option<String>,
    /// 限制数量
    pub limit: Option<u64>,
    /// 仅显示有问题的
    pub only_issues: Option<bool>,
}

/// 保存节点质量反馈（通用版）
#[tauri::command]
#[agent_command(domain = "finance", safety = Safe, call_mode = StateInput, description = "保存节点质量反馈数据，用于自我进化")]
pub async fn save_node_feedback(
    state: State<'_, AppState>,
    request: SaveNodeFeedbackRequest,
) -> Result<String, String> {
    let db = state.harness.db();

    let new_feedback = analyst_feedback::ActiveModel {
        id: Set(uuid::Uuid::new_v4().to_string()),
        node_type: Set(request.node_type),
        node_id: Set(request.node_id),
        report_id: Set(request.report_id),
        stock_code: Set(request.stock_code),
        execution_id: Set(request.execution_id),
        quality_score: Set(request.quality_score),
        grade: Set(request.grade),
        issue_count: Set(request.issue_count),
        warning_count: Set(request.warning_count),
        good_count: Set(request.good_count),
        checks_json: Set(request.checks_json),
        quality_metrics_json: Set(request.quality_metrics_json),
        consumed: Set(false),
        evolution_triggered: Set(false),
        created_at: Set(chrono::Utc::now().to_rfc3339()),
    };

    let result = new_feedback.insert(db).await.map_err(|e| {
        String::from(crate::commands::error::ErrorResponse::from_error(
            e,
            crate::commands::error::ErrorCategory::Unrecoverable,
        ))
    })?;
    Ok(result.id)
}

/// 保存分析师反馈（向后兼容包装）
#[tauri::command]
#[agent_command(domain = "finance", safety = Safe, call_mode = StateInput, description = "保存分析师质量反馈数据（向后兼容）")]
pub async fn save_analyst_feedback(
    state: State<'_, AppState>,
    request: SaveAnalystFeedbackRequest,
) -> Result<String, String> {
    // 将旧格式转换为新格式
    let quality_metrics = serde_json::json!({
        "bull_score": request.bull_score,
        "bear_score": request.bear_score,
        "confidence": request.confidence,
        "score_consistent": request.score_consistent,
        "direction_consistent": request.direction_consistent,
    });

    save_node_feedback(
        state,
        SaveNodeFeedbackRequest {
            node_type: "analyst".to_string(),
            node_id: request.analyst_id,
            report_id: request.report_id,
            stock_code: request.stock_code,
            execution_id: request.execution_id,
            quality_score: request.quality_score,
            grade: request.grade,
            issue_count: request.issue_count,
            warning_count: request.warning_count,
            good_count: request.good_count,
            checks_json: request.checks_json,
            quality_metrics_json: quality_metrics.to_string(),
        },
    )
    .await
}

/// 分析师反馈请求 DTO（向后兼容）
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SaveAnalystFeedbackRequest {
    pub analyst_id: String,
    pub report_id: String,
    pub stock_code: String,
    pub execution_id: String,
    pub quality_score: i32,
    pub grade: String,
    pub issue_count: i32,
    pub warning_count: i32,
    pub good_count: i32,
    pub checks_json: String,
    pub bull_score: Option<f64>,
    pub bear_score: Option<f64>,
    pub confidence: Option<f64>,
    pub score_consistent: bool,
    pub direction_consistent: bool,
}

/// 获取节点反馈历史（通用版）
#[tauri::command]
#[agent_command(domain = "finance", safety = Safe, call_mode = StateOnly, description = "获取指定节点的质量反馈历史")]
pub async fn get_node_feedbacks(
    state: State<'_, AppState>,
    request: GetNodeFeedbacksRequest,
) -> Result<Vec<NodeFeedbackSummary>, String> {
    let db = state.harness.db();

    let limit = request.limit.unwrap_or(50);
    let only_issues = request.only_issues.unwrap_or(false);

    let mut query = AnalystFeedback::find();

    if let Some(ref node_type) = request.node_type {
        query = query.filter(analyst_feedback::Column::NodeType.eq(node_type));
    }
    if let Some(ref node_id) = request.node_id {
        query = query.filter(analyst_feedback::Column::NodeId.eq(node_id));
    }

    query = query.order_by_desc(analyst_feedback::Column::CreatedAt).limit(limit);

    if only_issues {
        query = query.filter(analyst_feedback::Column::IssueCount.gt(0));
    }

    let results = query.all(db).await.map_err(|e| {
        String::from(crate::commands::error::ErrorResponse::from_error(
            e,
            crate::commands::error::ErrorCategory::Unrecoverable,
        ))
    })?;

    let summaries: Vec<NodeFeedbackSummary> = results
        .into_iter()
        .map(|m| NodeFeedbackSummary {
            id: m.id,
            node_type: m.node_type,
            node_id: m.node_id,
            quality_score: m.quality_score,
            grade: m.grade,
            issue_count: m.issue_count,
            warning_count: m.warning_count,
            created_at: m.created_at,
        })
        .collect();

    Ok(summaries)
}

/// 获取分析师反馈历史（向后兼容）
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GetAnalystFeedbacksRequest {
    pub analyst_id: String,
    pub limit: Option<u64>,
    pub only_issues: Option<bool>,
}

#[tauri::command]
#[agent_command(domain = "finance", safety = Safe, call_mode = StateOnly, description = "获取指定分析师的质量反馈历史（向后兼容）")]
pub async fn get_analyst_feedbacks(
    state: State<'_, AppState>,
    request: GetAnalystFeedbacksRequest,
) -> Result<Vec<NodeFeedbackSummary>, String> {
    get_node_feedbacks(
        state,
        GetNodeFeedbacksRequest {
            node_type: Some("analyst".to_string()),
            node_id: Some(request.analyst_id),
            limit: request.limit,
            only_issues: request.only_issues,
        },
    )
    .await
}

/// 节点反馈统计（用于判断是否需要触发进化）
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NodeFeedbackStats {
    pub node_type: String,
    pub node_id: String,
    pub total_count: u64,
    pub issue_count_total: u64,
    pub avg_quality_score: f64,
    pub low_score_count: u64,
    pub needs_evolution: bool,
    pub consistency_metrics: std::collections::HashMap<String, f64>,
}

/// 获取节点反馈统计（通用版）
#[tauri::command]
#[agent_command(domain = "finance", safety = Safe, call_mode = StateOnly, description = "获取节点的反馈统计数据，判断是否需要触发进化")]
pub async fn get_node_feedback_stats(
    state: State<'_, AppState>,
    node_type: String,
    node_id: String,
) -> Result<NodeFeedbackStats, String> {
    let db = state.harness.db();

    let all_feedbacks = AnalystFeedback::find()
        .filter(analyst_feedback::Column::NodeType.eq(&node_type))
        .filter(analyst_feedback::Column::NodeId.eq(&node_id))
        .all(db)
        .await
        .map_err(|e| {
            String::from(crate::commands::error::ErrorResponse::from_error(
                e,
                crate::commands::error::ErrorCategory::Unrecoverable,
            ))
        })?;

    let total_count = all_feedbacks.len() as u64;

    if total_count == 0 {
        return Ok(NodeFeedbackStats {
            node_type,
            node_id,
            total_count: 0,
            issue_count_total: 0,
            avg_quality_score: 100.0,
            low_score_count: 0,
            needs_evolution: false,
            consistency_metrics: std::collections::HashMap::new(),
        });
    }

    let issue_count_total = all_feedbacks.iter().map(|f| f.issue_count as u64).sum();
    let avg_quality_score =
        all_feedbacks.iter().map(|f| f.quality_score as f64).sum::<f64>() / total_count as f64;
    let low_score_count = all_feedbacks.iter().filter(|f| f.quality_score < 60).count() as u64;

    // 计算一致性指标
    let mut consistency_metrics = std::collections::HashMap::new();
    let node_type_enum = parse_node_type(&node_type);
    for f in &all_feedbacks {
        if let Ok(metrics) = serde_json::from_str::<serde_json::Value>(&f.quality_metrics_json) {
            let node_metrics = axagent_analysis_engine::node_quality::calc_consistency_metrics(
                &node_type_enum,
                &metrics,
            );
            for (key, value) in node_metrics {
                *consistency_metrics.entry(key).or_insert(0.0) += value;
            }
        }
    }
    // 计算平均值
    let total = total_count as f64;
    for value in consistency_metrics.values_mut() {
        *value /= total;
    }

    // 触发进化的条件：
    // 1. 至少有 3 次反馈
    // 2. 平均分 < 70 或 任一致性率 < 0.7 或 低分之多
    let needs_evolution = total_count >= 3
        && (avg_quality_score < 70.0
            || low_score_count >= 2
            || consistency_metrics.values().any(|v| *v < 0.7));

    Ok(NodeFeedbackStats {
        node_type,
        node_id,
        total_count,
        issue_count_total,
        avg_quality_score,
        low_score_count,
        needs_evolution,
        consistency_metrics,
    })
}

/// 解析节点类型字符串
fn parse_node_type(type_str: &str) -> axagent_analysis_engine::NodeType {
    match type_str {
        "analyst" => axagent_analysis_engine::NodeType::Analyst,
        "debate" => axagent_analysis_engine::NodeType::Debate,
        "decision" => axagent_analysis_engine::NodeType::Decision,
        "tool" | "valuation" | "risk" => axagent_analysis_engine::NodeType::Tool,
        _ => axagent_analysis_engine::NodeType::Other,
    }
}

/// 获取分析师反馈统计（向后兼容）
#[tauri::command]
#[agent_command(domain = "finance", safety = Safe, call_mode = StateOnly, description = "获取分析师的反馈统计数据（向后兼容）")]
pub async fn get_analyst_feedback_stats(
    state: State<'_, AppState>,
    analyst_id: String,
) -> Result<NodeFeedbackStats, String> {
    get_node_feedback_stats(state, "analyst".to_string(), analyst_id).await
}

// ── 单元测试 ── 追加在文件末尾（防 clippy::items_after_test_module）
#[cfg(test)]
mod analyst_feedback_ipc_tests {
    use super::*;

    /// 逐字复刻调用方（`AnalystDataQualityModal.tsx` 的 `invoke("save_node_feedback", …)`）
    /// 实际发送的载荷 —— 门必须钉在**真实形态**上，而不是钉在一个我们自己编的形状上。
    fn frontend_payload() -> serde_json::Value {
        serde_json::json!({
            "nodeType": "analyst",
            "nodeId": "a-technical",
            "reportId": "report-1770000000000",
            "stockCode": "600519",
            "executionId": "run-1",
            "qualityScore": 82,
            "grade": "B",
            "issueCount": 0,
            "warningCount": 1,
            "goodCount": 1,
            "checksJson": "[{\"field\":\"node_status\"}]",
            "qualityMetricsJson": "{\"dqi_grade\":\"B\"}"
        })
    }

    /// 正控：camelCase 载荷必须能进 handler（修复前它缺 `node_type` 等必填字段 ⇒ Err）。
    #[test]
    fn save_request_accepts_the_camel_case_payload_the_ui_sends() {
        let req: SaveNodeFeedbackRequest = serde_json::from_value(frontend_payload())
            .expect("camelCase 载荷必须可反序列化 —— 报红说明 DTO 的 rename_all 又掉了");
        assert_eq!(req.node_type, "analyst");
        assert_eq!(req.node_id, "a-technical");
        assert_eq!(req.quality_score, 82);
        assert_eq!(req.warning_count, 1);
        assert_eq!(req.checks_json, "[{\"field\":\"node_status\"}]");
    }

    /// 反控：**纯 snake_case** 载荷必须失败（键名整族改掉，不是在 camelCase 载荷上「补」一个
    /// snake 键 —— 那样 camelCase 键仍在，必然照样解析成功，断言就成了恒真）。
    /// 这条锁的是「有人摘掉 rename_all 后前端重新变成静默丢数据」这一复发路径。
    #[test]
    fn save_request_rejects_pure_snake_case_keys() {
        let snake = serde_json::json!({
            "node_type": "analyst",
            "node_id": "a-technical",
            "report_id": "report-1",
            "stock_code": "600519",
            "execution_id": "run-1",
            "quality_score": 82,
            "grade": "B",
            "issue_count": 0,
            "warning_count": 1,
            "good_count": 1,
            "checks_json": "[]",
            "quality_metrics_json": "{}"
        });
        assert!(
            serde_json::from_value::<SaveNodeFeedbackRequest>(snake).is_err(),
            "纯 snake_case 载荷被收下 ⇒ DTO 已不是 camelCase 契约，前端会再次静默丢数据"
        );
    }

    /// 响应侧同样逐键锁：前端按 camelCase 读，键名漂移 = 读到 undefined。
    #[test]
    fn response_dtos_serialize_camel_case() {
        let summary = serde_json::to_value(NodeFeedbackSummary {
            id: "1".into(),
            node_type: "analyst".into(),
            node_id: "a-technical".into(),
            quality_score: 82,
            grade: "B".into(),
            issue_count: 0,
            warning_count: 1,
            created_at: "2026-09-29T00:00:00Z".into(),
        })
        .unwrap();
        let obj = summary.as_object().unwrap();
        for key in [
            "id",
            "nodeType",
            "nodeId",
            "qualityScore",
            "grade",
            "issueCount",
            "warningCount",
            "createdAt",
        ] {
            assert!(obj.contains_key(key), "NodeFeedbackSummary 缺 camelCase 键 {key}");
        }
        assert!(!obj.contains_key("quality_score"), "又出现 snake_case 键");

        let stats = serde_json::to_value(NodeFeedbackStats {
            node_type: "analyst".into(),
            node_id: "a-technical".into(),
            total_count: 3,
            issue_count_total: 1,
            avg_quality_score: 74.5,
            low_score_count: 1,
            needs_evolution: true,
            consistency_metrics: std::collections::HashMap::new(),
        })
        .unwrap();
        let obj = stats.as_object().unwrap();
        for key in [
            "nodeType",
            "nodeId",
            "totalCount",
            "issueCountTotal",
            "avgQualityScore",
            "lowScoreCount",
            "needsEvolution",
            "consistencyMetrics",
        ] {
            assert!(obj.contains_key(key), "NodeFeedbackStats 缺 camelCase 键 {key}");
        }
        assert!(!obj.contains_key("avg_quality_score"), "又出现 snake_case 键");
    }

    /// 请求 DTO（列表 / 向后兼容版）也逐键锁 —— 它们同样跨 IPC。
    #[test]
    fn request_dtos_accept_camel_case_optionals() {
        let q: GetNodeFeedbacksRequest = serde_json::from_value(serde_json::json!({
            "nodeType": "analyst",
            "nodeId": "a-technical",
            "limit": 10,
            "onlyIssues": true
        }))
        .expect("GetNodeFeedbacksRequest 必须按 camelCase 收");
        assert_eq!(q.node_type.as_deref(), Some("analyst"));
        assert_eq!(q.only_issues, Some(true));

        let mut legacy = frontend_payload().as_object().unwrap().clone();
        legacy.insert("analystId".into(), serde_json::json!("a-technical"));
        legacy.insert("bullScore".into(), serde_json::json!(71.0));
        legacy.insert("scoreConsistent".into(), serde_json::json!(true));
        // 该 DTO 的 `directionConsistent` 是**必填 bool**（无 default）⇒ 夹具必须给全，
        // 否则这条测试会因为夹具自身缺字段而失败，锁不住它想锁的东西。
        legacy.insert("directionConsistent".into(), serde_json::json!(true));
        let parsed: SaveAnalystFeedbackRequest =
            serde_json::from_value(serde_json::Value::Object(legacy))
                .expect("SaveAnalystFeedbackRequest 必须按 camelCase 收");
        assert_eq!(parsed.analyst_id, "a-technical");
        assert_eq!(parsed.bull_score, Some(71.0));
        assert!(parsed.score_consistent);

        let g: GetAnalystFeedbacksRequest = serde_json::from_value(serde_json::json!({
            "analystId": "a-technical",
            "onlyIssues": false
        }))
        .expect("GetAnalystFeedbacksRequest 必须按 camelCase 收");
        assert_eq!(g.analyst_id, "a-technical");
        assert_eq!(g.only_issues, Some(false));
    }
}
