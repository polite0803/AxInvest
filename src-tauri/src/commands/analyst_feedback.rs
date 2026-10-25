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
use axagent_analysis_engine::blackboard::data_quality_report;
use axagent_entities::analyst_feedback;
use axagent_entities::analyst_feedback::Entity as AnalystFeedback;
use axagent_harness::DatabaseConnection;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, QueryOrder, QuerySelect, Set,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::HashMap;
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

    // 一致性指标：与进化判定共用同一份聚合实现（`node_quality::aggregate_consistency_rates`）。
    // 此前这里自己写了一遍「累加后除以总行数」，与 `node_evolution` 那份是同逻辑两份实现，
    // 两边一旦漂移，同一张表会在两个面板显示不同的一致率（2026-10-25 收口）。
    // 分母是「带上该指标的行数」：缺键的行既不当一致、也不当不一致。
    let node_type_enum = parse_node_type(&node_type);
    let consistency_metrics = axagent_analysis_engine::node_quality::aggregate_consistency_rates(
        &node_type_enum,
        all_feedbacks.iter().map(|f| f.quality_metrics_json.as_str()),
    );

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

// ── 分析完成后的反馈落库（2026-10-25：`analyst_feedbacks` 的**唯一**写入路径）────────
//
// 为什么搬进后端而不是留在前端：原先写入口只有一个 ——「用户点开某位分析师的数据质量弹窗」
// （2026-09-29 修好命令参数名与 serde 注解之后仍然只有那一条）。于是每日自动化批次、
// 时间旅行回放、T+0 重跑这些**不经过界面**的通道一行都不写。自我进化的输入不能取决于
// 有没有人点面板。现在它与 `stock_workflow::core.rs` 的 `record_dqi_observation` 同位
// （分析完成、黑板快照已成形），所有通道共用同一趟落库。
//
// 三条口径，与读侧一一对齐（改一条要同时改另一条）：
// 1. **全通道都写**（live / 时间旅行回放 / 妖股补分析）：本表存的是**节点自身产物**的质量
//    诊断（方向自洽、评分自洽、报告是否成形），与「哪一天的市场」无关 —— 回放轮的诊断同样
//    有效。而且回放轮的「缺数据」已被 data-quality 层按 `asof_exempt` 单独豁免
//    （见 `data-quality.rhai`），不会把「历史截断」冒充成「节点没跑」。
//    但本表**没有代际列** ⇒ 每行必须自带代际标签（`analysis_kind` / `analysis_source` /
//    `analysis_synthetic` / `analysis_as_of_date`），否则「这行来自实盘还是回放」永远查不回来，
//    两类样本事后无法分桶。
// 2. **一行 = 一位分析师 × 一个持有期**；`node_id` 取
//    `holding_period::dq_feedback_node_id`（去 `a-` 前缀、**不**带档后缀），档名进
//    `quality_metrics_json.tier`。理由见那个函数上的注释（读侧按精确相等过滤）。
// 3. **不知道就不写那个键**：`direction_consistent` 只在诊断带 `inconsistent_reason` 时写、
//    `score_consistent` 只在带 `score_sum_reason`（v184 新增）时写。读侧
//    `node_quality::calc_consistency_metrics` 同纪律（缺键 ⇒ 该指标不产，而不是 `unwrap_or`
//    默认成某个方向）。

/// 一轮分析的代际标签（写进每行的 `quality_metrics_json`）。
///
/// 本表没有代际列 ⇒ 这几个键是「事后分桶」的唯一依据：读侧要区分实盘样本与回放/补分析
/// 样本时，只能从这里取。
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct FeedbackGeneration {
    /// `stock_analyses.analysis_kind`：`live` / `replay`
    pub analysis_kind: String,
    /// `stock_analyses.source`：`live` / `time_travel` / `mover_recall` / `chat`
    pub analysis_source: String,
    /// `stock_analyses.synthetic`：1 = 事后回放/补分析
    pub analysis_synthetic: i32,
    /// `stock_analyses.as_of_date`：回放锚点（live 轮也可能有值，取原样不裁剪）
    pub analysis_as_of_date: Option<String>,
}

/// 一轮落库的结果。随 `workflow-completed` 事件下发（界面的两句缺席陈述读它），也进日志。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AnalystFeedbackOutcome {
    /// 已落库行数
    pub written: u32,
    /// 单行写库失败的条数（一行失败不影响其余，但必须计数 —— 静默就是「伪装成正常」）
    pub failed: u32,
    /// 一行都没写时的成因：`fast_chain` / `bad_snapshot` / `no_analysis_row` /
    /// `no_diagnosis`。写过任何一行 ⇒ `None`。
    ///
    /// ⚠ 成因必须由**产生它的那一方**给出，界面不得从「written=0」反推原因 ——
    /// 反推会把两种不同的缺席并成一句（快速链是设计如此，`no_diagnosis` 才是缺陷）。
    pub skipped: Option<&'static str>,
}

impl AnalystFeedbackOutcome {
    fn not_written(reason: &'static str) -> Self {
        Self { written: 0, failed: 0, skipped: Some(reason) }
    }
}

/// 纯函数：data-quality 诊断报告 → 反馈行载荷（不碰 DB ⇒ 坏样本可直接单测）。
///
/// 逐档行优先（`diagnostics_by_tier`），某缩写整体没有逐档行时才回落代表实例
/// （`diagnostics`，≤v132 快照 / 快速链）。**没有任何诊断行的分析师不产行** ——
/// 那说明该分析师本轮没有产物，写成一行会把「没跑」上报成「跑了但质量差」。
pub(crate) fn build_analyst_feedback_rows(
    report: &Value,
    generation: &FeedbackGeneration,
) -> Vec<SaveNodeFeedbackRequest> {
    let global_grade = report.get("grade").and_then(Value::as_str).unwrap_or_default().to_string();
    let global_score = report.get("score").and_then(Value::as_f64);
    let by_tier = report.get("diagnostics_by_tier").and_then(Value::as_object);
    let representative = report.get("diagnostics").and_then(Value::as_object);

    let mut rows = Vec::new();
    for (abbr, base) in axagent_harness::holding_period::DQ_ABBR_TO_BASE {
        let mut produced = false;
        if let Some(tiers) = by_tier.and_then(|d| d.get(*abbr)).and_then(Value::as_object) {
            for (tier, item) in tiers {
                if let Some(d) = item.as_object() {
                    rows.push(make_feedback_row(
                        abbr,
                        base,
                        Some(tier),
                        d,
                        &global_grade,
                        global_score,
                        generation,
                    ));
                    produced = true;
                }
            }
        }
        if produced {
            continue;
        }
        if let Some(d) = representative.and_then(|d| d.get(*abbr)).and_then(Value::as_object) {
            rows.push(make_feedback_row(
                abbr,
                base,
                None,
                d,
                &global_grade,
                global_score,
                generation,
            ));
        }
    }
    rows
}

/// 后端 `status` → 读侧认的严重度 + 三项计数。
///
/// ⚠ 读侧 `node_evolution.rs` 的高频问题统计只认 `checks[].status == "issue"` 这个**字面量**；
/// 直接塞后端的 `missing`/`low`/`normal` 会让该统计恒为空（弹窗旧载荷即如此）。
/// 未知状态按 `issue` 记：宁可多报一题，也不把「不认识」渲染成「正常」。
fn severity_of(status: &str) -> (&'static str, i32, i32, i32) {
    match status {
        "normal" => ("good", 0, 0, 1),
        "low" => ("warning", 0, 1, 0),
        _ => ("issue", 1, 0, 0),
    }
}

/// 只在诊断真带那个键时才写对应指标（三态里「不知道」的落法）。
fn put_consistency(
    metrics: &mut serde_json::Map<String, Value>,
    d: &serde_json::Map<String, Value>,
    reason_key: &str,
    known_key: &str,
    consistent_key: &str,
) {
    match d.get(reason_key).and_then(Value::as_str) {
        Some(reason) => {
            metrics.insert(known_key.to_string(), json!(true));
            metrics.insert(consistent_key.to_string(), json!(reason.is_empty()));
        },
        None => {
            metrics.insert(known_key.to_string(), json!(false));
        },
    }
}

#[allow(clippy::too_many_arguments)]
fn make_feedback_row(
    abbr: &str,
    base: &str,
    tier: Option<&str>,
    d: &serde_json::Map<String, Value>,
    global_grade: &str,
    global_score: Option<f64>,
    generation: &FeedbackGeneration,
) -> SaveNodeFeedbackRequest {
    let status = d.get("status").and_then(Value::as_str).unwrap_or("missing");
    let (severity, issue_count, warning_count, good_count) = severity_of(status);
    // 分析师自己的报告质量分优先；`report_quality` 缺席（旧快照）才退全局分，并在 metrics
    // 里留痕 `score_is_own=false`，避免把全局分数冒充成该分析师的分数。
    let own_score = d.get("report_quality").and_then(Value::as_f64);
    let quality_score = own_score.or(global_score).unwrap_or(0.0).round() as i32;
    let confidence = d.get("confidence").and_then(Value::as_f64).unwrap_or(-1.0);

    let mut metrics = serde_json::Map::new();
    metrics.insert("dqi_grade".to_string(), json!(global_grade));
    metrics.insert("dqi_score".to_string(), global_score.map(|v| json!(v)).unwrap_or(Value::Null));
    metrics.insert("node_status".to_string(), json!(status));
    metrics.insert("node_confidence".to_string(), json!(confidence));
    metrics.insert(
        "node_placeholder_hits".to_string(),
        json!(d.get("placeholder_hits").and_then(Value::as_i64).unwrap_or(0)),
    );
    metrics.insert(
        "node_report_quality".to_string(),
        own_score.map(|v| json!(v)).unwrap_or(Value::Null),
    );
    metrics.insert("score_is_own".to_string(), json!(own_score.is_some()));
    metrics.insert("dq_abbr".to_string(), json!(abbr));
    metrics.insert("tier".to_string(), tier.map(|v| json!(v)).unwrap_or(Value::Null));
    metrics.insert("period".to_string(), tier.map(|v| json!(v)).unwrap_or(Value::Null));
    put_consistency(
        &mut metrics,
        d,
        "inconsistent_reason",
        "direction_known",
        "direction_consistent",
    );
    put_consistency(&mut metrics, d, "score_sum_reason", "score_sum_known", "score_consistent");
    // 代际标签：让「这行来自实盘还是回放/补分析」事后可查（见 `FeedbackGeneration`）。
    metrics.insert("analysis_kind".to_string(), json!(generation.analysis_kind));
    metrics.insert("analysis_source".to_string(), json!(generation.analysis_source));
    metrics.insert("analysis_synthetic".to_string(), json!(generation.analysis_synthetic));
    metrics.insert(
        "analysis_as_of_date".to_string(),
        generation.analysis_as_of_date.clone().map(|v| json!(v)).unwrap_or(Value::Null),
    );

    let checks = json!([{
        "field": "node_status",
        "status": severity,
        "node_status": status,
        "confidence": confidence,
        "placeholder_hits": d.get("placeholder_hits").and_then(Value::as_i64).unwrap_or(0),
        "gap_reason": d.get("gap_reason").and_then(Value::as_str).unwrap_or(""),
        "inconsistent_reason": d.get("inconsistent_reason").and_then(Value::as_str),
        "score_sum_reason": d.get("score_sum_reason").and_then(Value::as_str),
    }]);

    SaveNodeFeedbackRequest {
        node_type: "analyst".to_string(),
        node_id: axagent_harness::holding_period::dq_feedback_node_id(base).to_string(),
        // report_id / stock_code / execution_id 由落库那一步按**本次分析的行**补齐
        // （`build_analyst_feedback_rows` 是纯函数，不知道行的事实）。
        report_id: String::new(),
        stock_code: String::new(),
        execution_id: String::new(),
        quality_score,
        // 等级只有全局口径（后端 grade 按 10 维代表实例算），逐分析师没有独立等级
        grade: global_grade.to_string(),
        issue_count,
        warning_count,
        good_count,
        checks_json: checks.to_string(),
        quality_metrics_json: Value::Object(metrics).to_string(),
    }
}

/// 分析完成后把本轮诊断写成反馈行。与 `record_dqi_observation` 同位、同容错纪律：
/// 落库是旁路事实，写不进去**不允许**把已完成的分析判成失败。
///
/// `bb_snapshot` 是刚序列化好的黑板快照；`execution_id` 是本轮真实执行 id
/// （由调用方在 `RunOptions.execution_id` 里生成并传入，缺位时传空串 ⇒ 行里也留空，
/// **不拿 analysis_id 顶替**：两个标识分属 `stock_analyses` 与 `execution_workflows`）。
pub(crate) async fn record_analyst_feedbacks(
    db: &DatabaseConnection,
    analysis_id: &str,
    template_id: &str,
    bb_snapshot: &str,
    execution_id: &str,
) -> AnalystFeedbackOutcome {
    // 快速链没有那 10 个分析师节点（`record_dqi_observation` 同一判据、同一理由：
    // 把结构性缺产物记成质量异常，三轮之后就永久熔断在假理由上）。
    if template_id == axagent_harness::constants::FAST_TEMPLATE_ID {
        return AnalystFeedbackOutcome::not_written("fast_chain");
    }

    let snap: HashMap<String, Value> = match serde_json::from_str(bb_snapshot) {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!(
                "[analyst_feedback] 快照不是合法 JSON，本轮不写反馈（analysis_id={analysis_id}）: {e}"
            );
            return AnalystFeedbackOutcome::not_written("bad_snapshot");
        },
    };

    // 代际标签只从本行取（窄 DTO，不拉整行 —— 同 dao::repo::stock_analysis_rerun_queue 的纪律）
    let meta = axagent_dao::repo::stock_analysis_rerun_queue::fetch_analysis_generation_meta(
        db,
        analysis_id,
    )
    .await
    .ok()
    .flatten();
    let Some(meta) = meta else {
        tracing::warn!(
            "[analyst_feedback] 查不到分析记录，本轮不写反馈: analysis_id={analysis_id}"
        );
        return AnalystFeedbackOutcome::not_written("no_analysis_row");
    };

    let Some(report) = data_quality_report(&snap) else {
        tracing::warn!(
            "[analyst_feedback] 本轮没有 data-quality 诊断（节点未跑或快照无该键），analysis_id={analysis_id}"
        );
        return AnalystFeedbackOutcome::not_written("no_diagnosis");
    };

    let generation = FeedbackGeneration {
        analysis_kind: meta.analysis_kind,
        analysis_source: meta.source,
        analysis_synthetic: meta.synthetic,
        analysis_as_of_date: meta.as_of_date,
    };

    let drafts = build_analyst_feedback_rows(&report, &generation);
    if drafts.is_empty() {
        tracing::warn!(
            "[analyst_feedback] 诊断里没有任何已登记的分析师槽位，analysis_id={analysis_id}（新增分析师须同步 harness::DQ_ABBR_TO_BASE 与前端镜像表）"
        );
        return AnalystFeedbackOutcome::not_written("no_diagnosis");
    }

    let mut written = 0_u32;
    let mut failed = 0_u32;
    let mut first_error: Option<String> = None;
    for draft in drafts {
        // 标识在这一步补齐：行是本次分析的行，与 draft 里的载荷字段同源。
        let payload = SaveNodeFeedbackRequest {
            report_id: analysis_id.to_string(),
            stock_code: meta.stock_code.clone(),
            execution_id: execution_id.to_string(),
            ..draft
        };
        let active = analyst_feedback::ActiveModel {
            id: Set(uuid::Uuid::new_v4().to_string()),
            node_type: Set(payload.node_type),
            node_id: Set(payload.node_id),
            report_id: Set(payload.report_id),
            stock_code: Set(payload.stock_code),
            execution_id: Set(payload.execution_id),
            quality_score: Set(payload.quality_score),
            grade: Set(payload.grade),
            issue_count: Set(payload.issue_count),
            warning_count: Set(payload.warning_count),
            good_count: Set(payload.good_count),
            checks_json: Set(payload.checks_json),
            quality_metrics_json: Set(payload.quality_metrics_json),
            consumed: Set(false),
            evolution_triggered: Set(false),
            created_at: Set(chrono::Utc::now().to_rfc3339()),
        };
        match active.insert(db).await {
            Ok(_) => written += 1,
            Err(e) => {
                failed += 1;
                if first_error.is_none() {
                    first_error = Some(e.to_string());
                }
            },
        }
    }
    if failed > 0 {
        tracing::warn!(
            "[analyst_feedback] 本轮 {failed} 条反馈写库失败（已写 {written} 条），首个错误: {}",
            first_error.unwrap_or_default()
        );
    }
    AnalystFeedbackOutcome { written, failed, skipped: None }
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

// ── 落库旁路的行构造判据（2026-10-25）──────────────────────────────────────────
//
// 这一批门禁针对**纯函数** `build_analyst_feedback_rows`（不碰 DB ⇒ 坏样本可直接喂进来）。
// 每条断言只锁一件事；「缺键 ≠ 一致 ≠ 不一致」的三态用三条断言分开表达，
// 不合并成「只要不是 true 就算是 false」那种恒真写法。
#[cfg(test)]
mod analyst_feedback_row_tests {
    use super::*;
    use axagent_harness::AnalysisKind;
    use serde_json::json;

    fn generation() -> FeedbackGeneration {
        FeedbackGeneration {
            analysis_kind: AnalysisKind::Live.as_storage().to_string(),
            analysis_source: "live".to_string(),
            analysis_synthetic: 0,
            analysis_as_of_date: Some("2026-10-25".to_string()),
        }
    }

    /// 三种 analyst 形态各一行：
    /// - `mk` 有逐档行 ⇒ 只产逐档行（代表行**不**再生效）
    /// - `sec` 有逐档行且代表行内容不同 ⇒ 同样只取逐档
    /// - `fund` 只有代表行 ⇒ 产一行且 `tier` 为空
    ///
    /// 其余 7 个缩写两种形态都没有 ⇒ 一行都不产（「没跑」不能上报成「质量差」）。
    fn report() -> Value {
        json!({
            "grade": "B",
            "score": 78,
            "diagnostics": {
                "mk": { "status": "normal", "report_quality": 90, "confidence": 70 },
                "sec": { "status": "missing" },
                "fund": { "status": "normal", "report_quality": 81, "confidence": 66 }
            },
            "diagnostics_by_tier": {
                "mk": {
                    "mid": {
                        "status": "low",
                        "report_quality": 60,
                        "confidence": 45,
                        "placeholder_hits": 2,
                        "inconsistent_reason": "verdict 看多 但 bull_score(30) <= bear_score(60)",
                        "score_sum_reason": "bull+bear=90 不等于 100"
                    }
                },
                "sec": {
                    "mid": { "status": "normal", "report_quality": 88, "confidence": 75 }
                }
            }
        })
    }

    fn metrics_of(row: &SaveNodeFeedbackRequest) -> Value {
        serde_json::from_str(&row.quality_metrics_json).expect("metrics 必须是合法 JSON 对象")
    }

    fn find<'a>(rows: &'a [SaveNodeFeedbackRequest], node_id: &str) -> &'a SaveNodeFeedbackRequest {
        rows.iter()
            .find(|r| r.node_id == node_id)
            .unwrap_or_else(|| panic!("没有 node_id={node_id} 的行"))
    }

    /// 一行 = 一位分析师 × 一个持有期；有逐档诊断的分析师**只**产逐档行。
    #[test]
    fn builds_one_row_per_analyst_and_tier() {
        let rows = build_analyst_feedback_rows(&report(), &generation());
        assert_eq!(rows.len(), 3, "只有 mk/sec/fund 有诊断产物，其余缩写不该产行");
        // 逐档行优先：mk 的代表行 status=normal、逐档行 status=low，取的必须是逐档那份
        let mk = find(&rows, "market-analyst");
        assert_eq!(metrics_of(mk)["node_status"], json!("low"));
        assert_eq!(metrics_of(mk)["tier"], json!("mid"));
        assert_eq!(metrics_of(mk)["period"], json!("mid"));
        // 只有代表行的分析师：tier 为空（不是空串、不是猜一个档）
        let fund = find(&rows, "fundamentals");
        assert_eq!(metrics_of(fund)["tier"], Value::Null);
        assert_eq!(metrics_of(fund)["node_status"], json!("normal"));
    }

    /// `node_id` 必须是**去 `a-` 前缀、不带档后缀**的 base —— 读侧按精确相等过滤，
    /// 写侧多一个前缀/后缀 ⇒ 新行一条也查不到（症状与前端缩写表一模一样）。
    #[test]
    fn node_id_matches_the_reader_filter_form() {
        let rows = build_analyst_feedback_rows(&report(), &generation());
        for row in &rows {
            assert!(
                !row.node_id.starts_with("a-"),
                "node_id 带 `a-` 前缀 ⇒ 读侧查不到：{}",
                row.node_id
            );
            assert!(!row.node_id.contains("--"), "node_id 带档后缀 ⇒ 读侧查不到：{}", row.node_id);
        }
        assert_eq!(find(&rows, "market-analyst").node_id, "market-analyst");
        assert_eq!(find(&rows, "sector").node_id, "sector");
    }

    /// 方向自洽三态：`inconsistent_reason` 非空 ⇒ false；为空串 ⇒ true；**键缺席 ⇒ 键不产**。
    /// 「键缺席时写 true」会把「没判过」冒充成「判过且自洽」⇒ 读侧一致率恒 100%。
    #[test]
    fn direction_consistency_is_tristate() {
        let rows = build_analyst_feedback_rows(&report(), &generation());
        let mk = metrics_of(find(&rows, "market-analyst"));
        assert_eq!(mk["direction_known"], json!(true));
        assert_eq!(mk["direction_consistent"], json!(false));

        let fund = metrics_of(find(&rows, "fundamentals"));
        assert_eq!(fund["direction_known"], json!(false));
        assert!(
            fund.get("direction_consistent").is_none(),
            "诊断没有 inconsistent_reason ⇒ 不得凭空产出自洽判定（读侧靠缺键表达「不知道」）"
        );

        // 空串 = 判过且自洽
        let empty_reason = json!({
            "grade": "A", "score": 90,
            "diagnostics": { "mk": { "status": "normal", "inconsistent_reason": "" } }
        });
        let rows2 = build_analyst_feedback_rows(&empty_reason, &generation());
        let mk2 = metrics_of(find(&rows2, "market-analyst"));
        assert_eq!(mk2["direction_known"], json!(true));
        assert_eq!(mk2["direction_consistent"], json!(true));
    }

    /// 评分自洽同三态（键名 `score_sum_reason`，v184 起由 rhai 逐节点产出）。
    #[test]
    fn score_consistency_is_tristate() {
        let rows = build_analyst_feedback_rows(&report(), &generation());
        let mk = metrics_of(find(&rows, "market-analyst"));
        assert_eq!(mk["score_sum_known"], json!(true));
        assert_eq!(mk["score_consistent"], json!(false));

        let fund = metrics_of(find(&rows, "fundamentals"));
        assert_eq!(fund["score_sum_known"], json!(false));
        assert!(
            fund.get("score_consistent").is_none(),
            "诊断没有 score_sum_reason ⇒ 不得凭空产出评分自洽判定"
        );
    }

    /// `checks[].status` 必须落读侧认的**字面量**（good / warning / issue）。
    /// 读侧 `node_evolution.rs` 只数 `status == "issue"`；直接塞后端的
    /// `missing` / `low` / `normal` 会让高频问题统计恒为空。
    #[test]
    fn checks_use_the_severity_literals_the_reader_counts() {
        let rows = build_analyst_feedback_rows(&report(), &generation());
        let mk = find(&rows, "market-analyst");
        let checks: Value = serde_json::from_str(&mk.checks_json).unwrap();
        assert_eq!(checks[0]["status"], json!("warning"), "status=low ⇒ warning");
        assert_eq!(checks[0]["node_status"], json!("low"), "原始状态要留痕");

        let fund = find(&rows, "fundamentals");
        let checks: Value = serde_json::from_str(&fund.checks_json).unwrap();
        assert_eq!(checks[0]["status"], json!("good"), "status=normal ⇒ good");
    }

    /// `missing` 归 `issue`（读侧会数它），而不是被当成「没产出 ⇒ 不报」。
    #[test]
    fn missing_status_counts_as_issue() {
        let report = json!({
            "grade": "D", "score": 40,
            "diagnostics": { "mk": { "status": "missing" } }
        });
        let rows = build_analyst_feedback_rows(&report, &generation());
        let mk = find(&rows, "market-analyst");
        assert_eq!(mk.issue_count, 1);
        assert_eq!(mk.warning_count, 0);
        assert_eq!(mk.good_count, 0);
        let checks: Value = serde_json::from_str(&mk.checks_json).unwrap();
        assert_eq!(checks[0]["status"], json!("issue"));
    }

    /// 代际标签必须落在 metrics 里（本表无代际列 ⇒ 这里不写就永远查不回来）。
    #[test]
    fn generation_tags_are_stamped_on_every_row() {
        let rows = build_analyst_feedback_rows(&report(), &generation());
        assert_eq!(rows.len(), 3);
        for row in &rows {
            let m = metrics_of(row);
            assert_eq!(m["analysis_kind"], json!(AnalysisKind::Live.as_storage()));
            assert_eq!(m["analysis_source"], json!("live"));
            assert_eq!(m["analysis_synthetic"], json!(0));
            assert_eq!(m["analysis_as_of_date"], json!("2026-10-25"));
        }
    }

    /// 没有诊断 ⇒ 一行都不产（不能拿全局分硬凑 10 行）。
    #[test]
    fn empty_diagnosis_produces_no_rows() {
        let empty =
            json!({ "grade": "A", "score": 95, "diagnostics": {}, "diagnostics_by_tier": {} });
        assert!(build_analyst_feedback_rows(&empty, &generation()).is_empty());
    }

    /// 质量分优先取该分析师自己的 `report_quality`；缺席才退全局分，且留痕 `score_is_own=false`。
    #[test]
    fn quality_score_prefers_the_analyst_own_score() {
        let rows = build_analyst_feedback_rows(&report(), &generation());
        let fund = find(&rows, "fundamentals");
        assert_eq!(fund.quality_score, 81);
        assert_eq!(metrics_of(fund)["score_is_own"], json!(true));

        let no_own = json!({
            "grade": "B", "score": 78,
            "diagnostics": { "mk": { "status": "normal" } }
        });
        let rows2 = build_analyst_feedback_rows(&no_own, &generation());
        let mk = find(&rows2, "market-analyst");
        assert_eq!(mk.quality_score, 78);
        assert_eq!(metrics_of(mk)["score_is_own"], json!(false));
    }

    /// 落库结果的载荷形状：前端按 camelCase 读 `written` / `failed` / `skipped`。
    #[test]
    fn outcome_serializes_camel_case_for_the_frontend() {
        let outcome = AnalystFeedbackOutcome { written: 3, failed: 1, skipped: None };
        let v = serde_json::to_value(&outcome).unwrap();
        assert_eq!(v["written"], json!(3));
        assert_eq!(v["failed"], json!(1));
        assert!(v["skipped"].is_null());

        let skipped =
            serde_json::to_value(AnalystFeedbackOutcome::not_written("no_diagnosis")).unwrap();
        assert_eq!(skipped["skipped"], json!("no_diagnosis"));
        assert_eq!(skipped["written"], json!(0));
    }
}
