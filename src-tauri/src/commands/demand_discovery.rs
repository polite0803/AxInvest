// SPDX-License-Identifier: AGPL-3.0-only

//! 需求发现（Demand Discovery）领域 Tauri 命令层
//!
//! 暴露能力扫描、主动扫描入库、高价值通知、能力缺口分析、交付工作流执行等命令。
//!
//! ## 链路统一（2026-09-09）
//!
//! 本文件原为 B 链路（废弃表 `opc_demand_lead` 单数）：自带扫描装配+评估+
//! 裸 insert（无去重），与 A 链路（`opc_demand_leads` 复数表，指纹去重）
//! 并行漂移。现已收敛为 A 链路薄壳：
//! - 扫描入库：`opc_proactive_evaluate_and_save_leads` / `run_demand_discovery_cron`
//!   → `axagent_tools::tools::opc_demand_scan::run_discovery_scan`（与订阅、
//!   工作流工具节点同源）
//! - 线索读写：`axagent_dao::repo::opc_demand`（get_lead/update_lead_status/list_leads）
//! - 交付进度：单一事实来源 `opc_delivery` 表，不回写 lead 状态
//! - 线索状态机：new/evaluated/contacted/won/lost（销售转化线，见 repo 状态机）
//!
//! 已删除的 B 链路命令：opc_proactive_discover_leads / opc_discover_leads /
//! opc_confirm_lead / opc_mark_lead_status / opc_test_platform_connection
//! （连同废弃实体 opc_demand_lead / opc_market_platform；线上孤儿表由
//! 运维侧手动 DROP，不走迁移）。
//!
//! 启动接线：init/services.rs 的 start_demand_discovery_cron 调用
//! run_demand_discovery_cron 扫描已启用平台。

use axagent_agent_macro::agent_command;
use sea_orm::sea_query::Expr;
use tauri::State;

use crate::AppState;
use crate::commands::error::ErrorResponse;

// ── 能力扫描 ──────────────────────────────────────────────────

/// 扫描系统当前可用的能力清单（工具/技能/MCP/工作流）
///
/// 复用上游能力发现索引（`capability_indexer`）的能力护照，按能力类型分组组装
/// `CapabilityInventory`。不再重复扫描注册表并落库到 `opc_capability` 表，
/// 避免与上游 `register_all_capabilities` 的能力基座重复收集。
#[agent_command(domain = "automation", safety = Safe, call_mode = StateOnly, description = "扫描系统能力清单")]
#[tauri::command]
pub async fn opc_scan_capabilities(
    state: State<'_, AppState>,
) -> Result<serde_json::Value, String> {
    use axagent_analysis_engine::opc::capability::{
        CapabilityEntry, CapabilityInventory, CapabilitySource,
    };
    use axagent_harness::CapabilityKind;

    let now = chrono::Utc::now().timestamp();

    // 从上游能力索引读取全部护照，按类型分组（来源与 kind 保持一致）
    let mut tools: Vec<CapabilityEntry> = Vec::new();
    let mut skills: Vec<CapabilityEntry> = Vec::new();
    let mut mcp_tools: Vec<CapabilityEntry> = Vec::new();
    let mut workflows: Vec<CapabilityEntry> = Vec::new();
    let mut agents: Vec<CapabilityEntry> = Vec::new();

    let ids = state.capability_indexer.list_capability_ids().await;
    for id in ids {
        if let Some(p) = state.capability_indexer.get_passport(&id).await {
            // 系统专用护照（如认知编排器）不进入业务能力清单
            if p.visibility.is_system_only() {
                continue;
            }
            let source = match p.kind {
                CapabilityKind::Skill => CapabilitySource::Skill,
                CapabilityKind::Workflow => CapabilitySource::Workflow,
                CapabilityKind::Tool if p.capability_id.starts_with("mcp:") => {
                    CapabilitySource::McpTool
                },
                CapabilityKind::Tool => CapabilitySource::Tool,
                CapabilityKind::Agent => CapabilitySource::Agent,
                _ => continue,
            };
            let entry = CapabilityEntry {
                id: p.capability_id.clone(),
                name: p.name.clone(),
                description: p.description.clone(),
                source: source.clone(),
                source_id: p.capability_id.clone(),
                capability_type: p.kind.as_str().to_string(),
                applicable_scenarios: Vec::new(),
                example_deliverables: Vec::new(),
                metadata: serde_json::json!({
                    "enabled": p.enabled,
                    "domain": p.domain.as_str(),
                    "sub_category": p.sub_category,
                }),
            };
            match source {
                CapabilitySource::Tool => tools.push(entry),
                CapabilitySource::Skill => skills.push(entry),
                CapabilitySource::McpTool => mcp_tools.push(entry),
                CapabilitySource::Workflow => workflows.push(entry),
                CapabilitySource::Agent => agents.push(entry),
            }
        }
    }

    let mut inv = CapabilityInventory {
        tools,
        skills,
        mcp_tools,
        workflows,
        agents,
        scanned_at: now,
        total_count: 0,
    };
    inv.recalc_count();

    serde_json::to_value(&inv).map_err(|e| {
        ErrorResponse::from_error(e, crate::commands::error::ErrorCategory::Unrecoverable)
            .to_string()
    })
}

// ── 市场需求发现 ──────────────────────────────────────────────

/// 从配置中提取领域关键词，生成主动扫描的查询列表
///
/// 读取 workflow_template(id="demand-discovery") 中的 domain_* 变量，
/// 将每个领域的关键词展开为独立的搜索查询。
async fn extract_domain_queries(db: &sea_orm::DatabaseConnection) -> Result<Vec<String>, String> {
    use axagent_entities::workflow_template;
    use sea_orm::*;

    let template = workflow_template::Entity::find_by_id("demand-discovery")
        .one(db)
        .await
        .map_err(|e| format!("读取需求发现配置失败: {e}"))?;

    let config_json = template
        .and_then(|t| t.variables)
        .and_then(|c| serde_json::from_str::<serde_json::Value>(&c).ok())
        .unwrap_or_else(|| serde_json::json!({}));

    let mut queries = Vec::new();

    // 提取所有 domain_* 开头的变量
    if let Some(vars) = config_json.get("variables").and_then(|v| v.as_array()) {
        for var in vars {
            let name = var.get("name").and_then(|n| n.as_str()).unwrap_or("");
            if name.starts_with("domain_") {
                if let Some(value) = var.get("value").and_then(|v| v.as_str()) {
                    // 将 "科技/AI/软件" 拆分为独立关键词
                    for kw in value.split('/') {
                        let trimmed = kw.trim();
                        if !trimmed.is_empty() {
                            queries.push(trimmed.to_string());
                        }
                    }
                }
            }
        }
    }

    // 如果没有配置任何领域关键词，使用默认种子
    if queries.is_empty() {
        queries = vec![
            "AI".to_string(),
            "软件".to_string(),
            "设计".to_string(),
            "营销".to_string(),
            "写作".to_string(),
            "翻译".to_string(),
        ];
    }

    Ok(queries)
}

/// 主动扫描流水线（统一走 A 链路 [`axagent_tools::tools::opc_demand_scan::
/// run_discovery_scan`]，与订阅定时扫描/工作流工具节点共用去重入库规则）
///
/// 逐关键词调用扫描核心并聚合统计。此前本命令在 B 链路自实现了一遍
/// 扫描装配+评估+裸 insert（无去重、写到废弃的 `opc_demand_lead` 表），
/// 已收敛为薄壳 —— 扫描器装配与去重入库规则不再有第二份拷贝。
async fn run_proactive_scan_pipeline(
    db: &sea_orm::DatabaseConnection,
    queries: &[String],
) -> (u32, u32, u32, Vec<serde_json::Value>) {
    let mut total_scanned = 0u32;
    let mut total_saved = 0u32;
    let mut high_value_count = 0u32;
    let mut query_stats = Vec::new();
    for query in queries {
        match axagent_tools::tools::opc_demand_scan::run_discovery_scan(db, query, &[]).await {
            Ok(summary) => {
                total_scanned += summary.total_scanned;
                total_saved += summary.total_saved;
                high_value_count += summary.high_value_count;
                query_stats.push(serde_json::json!({
                    "query": query,
                    "scanned": summary.total_scanned,
                    "saved": summary.total_saved,
                    "refreshed": summary.total_refreshed,
                }));
            },
            Err(e) => {
                tracing::warn!("[proactive_scan] 关键词 '{query}' 扫描失败: {e}");
                query_stats.push(serde_json::json!({
                    "query": query,
                    "scanned": 0,
                    "saved": 0,
                    "error": e.to_string(),
                }));
            },
        }
    }
    (total_scanned, total_saved, high_value_count, query_stats)
}

/// 主动评估入库：基于配置的领域关键词自动扫描、评估并入库
///
/// 无需用户输入关键词，系统自动从配置中提取 domain_* 关键词，
/// 对每个领域执行「扫描 + 评估 + 去重入库」完整流水线。
///
/// `min_score` 为历史遗留参数：A 链路全量入库（评估分随行存储），
/// 阈值过滤由订阅推送侧按 `min_score` 承担，此处不再截断。
#[agent_command(domain = "automation", safety = Safe, call_mode = StateOnly, description = "主动评估并入库需求")]
#[tauri::command]
pub async fn opc_proactive_evaluate_and_save_leads(
    state: State<'_, AppState>,
    min_score: Option<f64>,
) -> Result<serde_json::Value, String> {
    let _ = min_score;
    let db = state.harness.db();

    let queries = extract_domain_queries(db).await?;
    let (total_scanned, total_saved, high_value_count, query_stats) =
        run_proactive_scan_pipeline(db, &queries).await;

    let result = serde_json::json!({
        "total_queries": queries.len(),
        "total_scanned": total_scanned,
        "total_saved": total_saved,
        "high_value_count": high_value_count,
        "query_stats": query_stats,
    });

    serde_json::to_value(&result).map_err(|e| {
        ErrorResponse::from_error(e, crate::commands::error::ErrorCategory::Unrecoverable)
            .to_string()
    })
}

/// 主动需求发现 Cron：扫描 → 评估 → 去重入库 → 高价值通知
///
/// 供 CronExecutor 调用。当 query 为 None 或空字符串时，自动从配置中
/// 提取领域关键词进行主动扫描。入库与去重统一走 A 链路
/// `run_discovery_scan`（订阅定时扫描同源，无第二份实现）。
///
/// # 参数
/// - `db`: 数据库连接
/// - `query`: 搜索关键词（None 或空则自动从配置提取）
/// - `app_handle`: Tauri AppHandle（用于发送桌面通知，可选）
pub async fn run_demand_discovery_cron(
    db: &sea_orm::DatabaseConnection,
    query: Option<&str>,
    app_handle: Option<&tauri::AppHandle>,
) -> Result<String, String> {
    // 1) 确定查询关键词列表
    let queries: Vec<String> = match query {
        Some(q) if !q.trim().is_empty() => vec![q.to_string()],
        _ => extract_domain_queries(db).await?,
    };

    // 2) 逐关键词扫描入库（去重规则与订阅/工作流同源）
    let mut high_value_leads: Vec<(String, f64, String)> = Vec::new();
    for q in &queries {
        if let Ok(summary) =
            axagent_tools::tools::opc_demand_scan::run_discovery_scan(db, q, &[]).await
        {
            for lead in &summary.leads {
                high_value_leads.push((
                    lead.id.clone(),
                    lead.commercial_value_score,
                    lead.title.clone(),
                ));
            }
        }
    }
    let high_value_count = high_value_leads.len() as u32;

    // 3) 发送高价值需求通知
    if high_value_count > 0 {
        send_high_value_notification(app_handle, &high_value_leads).await;
    }

    Ok(format!("主动需求发现完成: {} 个关键词, 高价值 {} 条", queries.len(), high_value_count))
}

// ── Cron 路由辅助函数 ───────────────────────────────────────────

/// 发送高价值需求通知
///
/// 通过 Tauri 桌面通知 + 前端事件推送，提醒用户关注高价值需求。
async fn send_high_value_notification(
    app_handle: Option<&tauri::AppHandle>,
    high_value_leads: &[(String, f64, String)],
) {
    // 移动端无桌面通知通道，app_handle 仅用于桌面端，避免 unused 警告
    #[cfg(mobile)]
    let _ = app_handle;

    if high_value_leads.is_empty() {
        return;
    }

    // count / titles 均只用于下方 `not(mobile)` 的桌面通知 body ⇒ 一并纳入非 mobile编译，
    // 否则 Android(mobile) 下为未使用变量触发 `unused_variables` 警告。
    #[cfg(not(mobile))]
    let count = high_value_leads.len();
    #[cfg(not(mobile))]
    let titles: Vec<String> = high_value_leads
        .iter()
        .take(3)
        .map(|(_, score, title)| format!("{} (评分: {:.1})", title, score))
        .collect();

    #[allow(unused_variables)]
    #[cfg(not(mobile))]
    let body = if count > 3 {
        format!("{} 条高价值需求: {} ...等", count, titles.join(", "))
    } else {
        format!("{} 条高价值需求: {}", count, titles.join(", "))
    };

    // 发送 Tauri 桌面通知（仅桌面端；移动端无桌面通知通道，仅靠下方日志记录）
    #[cfg(not(mobile))]
    if let Some(app) = app_handle {
        if let Err(e) = crate::commands::desktop::send_desktop_notification(
            app.clone(),
            "🔔 OPC 需求发现：发现高价值需求".to_string(),
            body.clone(),
        )
        .await
        {
            tracing::warn!("[DemandDiscovery] 桌面通知发送失败: {}", e);
        }
    }

    // 同时通过日志记录，便于排查。
    // ⚠ 这里必须直接取 `high_value_leads.len()` 而非上面的 `count` —— 后者被
    // `#[cfg(not(mobile))]` 门控，在 Android(mobile) 下不存在（曾导致 E0425 编译失败）。
    tracing::info!(
        "[DemandDiscovery] 高价值需求通知: 发现 {} 条高价值需求",
        high_value_leads.len()
    );
    for (id, score, title) in high_value_leads {
        tracing::info!(
            "[DemandDiscovery] 高价值需求详情: id={}, score={:.1}, title={}",
            id,
            score,
            title
        );
    }
}

// ── 能力缺口 ──────────────────────────────────────────────────

/// 列出能力缺口记录（可按状态过滤）
#[agent_command(domain = "automation", safety = Safe, call_mode = StateOnly, description = "列出能力缺口")]
#[tauri::command]
pub async fn opc_list_capability_gaps(
    state: State<'_, AppState>,
    status: Option<String>,
) -> Result<serde_json::Value, String> {
    use axagent_entities::opc_capability_gap;
    use sea_orm::*;

    let db = state.harness.db();
    let mut qs = opc_capability_gap::Entity::find();
    if let Some(ref s) = status {
        qs = qs.filter(opc_capability_gap::Column::Status.eq(s));
    }

    let results =
        qs.order_by_desc(opc_capability_gap::Column::CreatedAt).all(db).await.map_err(|e| {
            ErrorResponse::from_error(e, crate::commands::error::ErrorCategory::Unrecoverable)
                .to_string()
        })?;

    serde_json::to_value(&results).map_err(|e| {
        ErrorResponse::from_error(e, crate::commands::error::ErrorCategory::Unrecoverable)
            .to_string()
    })
}

/// 关闭能力缺口（能力建设完成后标记 resolved）
#[agent_command(domain = "automation", safety = Caution, call_mode = StateInput, description = "关闭能力缺口")]
#[tauri::command]
pub async fn opc_close_capability_gap(
    state: State<'_, AppState>,
    id: String,
) -> Result<serde_json::Value, String> {
    use axagent_entities::opc_capability_gap;
    use sea_orm::*;

    let db = state.harness.db();
    let now = chrono::Utc::now().timestamp();

    let result = opc_capability_gap::Entity::find_by_id(&id)
        .one(db)
        .await
        .map_err(|e| {
            ErrorResponse::from_error(e, crate::commands::error::ErrorCategory::Unrecoverable)
                .to_string()
        })?
        .ok_or_else(|| format!("能力缺口不存在: {id}"))?;

    let mut am: opc_capability_gap::ActiveModel = result.into();
    am.status = Set("resolved".to_string());
    am.closed_at = Set(Some(now));
    am.updated_at = Set(now);

    let saved = am.update(db).await.map_err(|e| {
        ErrorResponse::from_error(e, crate::commands::error::ErrorCategory::Unrecoverable)
            .to_string()
    })?;

    serde_json::to_value(&saved).map_err(|e| {
        ErrorResponse::from_error(e, crate::commands::error::ErrorCategory::Unrecoverable)
            .to_string()
    })
}

/// 主动分析能力缺口：基于已有需求线索统计高频缺失能力
///
/// 与被动"匹配失败即缺口"不同，此命令主动分析：
/// 1. 统计高价值需求中未匹配能力的高频关键词
/// 2. 分析领域需求趋势与现有能力库的覆盖差距
/// 3. 基于配置的领域关键词对比能力库覆盖
#[agent_command(domain = "automation", safety = Safe, call_mode = StateOnly, description = "主动分析能力缺口")]
#[tauri::command]
pub async fn opc_analyze_capability_gaps(
    state: State<'_, AppState>,
) -> Result<serde_json::Value, String> {
    use axagent_entities::opc_capability_gap;
    use sea_orm::*;
    use std::collections::HashMap;

    let db = state.harness.db();
    let now = chrono::Utc::now().timestamp();

    // 1) 扫描现有能力库（复用上游能力索引的能力护照）
    let capability_keywords: Vec<String> = {
        let mut keywords: Vec<String> = Vec::new();
        for id in state.capability_indexer.list_capability_ids().await {
            if let Some(p) = state.capability_indexer.get_passport(&id).await {
                if p.visibility.is_system_only() {
                    continue;
                }
                keywords.push(
                    format!("{} {} {}", p.name, p.description, p.kind.as_str()).to_lowercase(),
                );
            }
        }
        keywords
    };

    // 2) 统计高价值需求中的高频关键词（统一读 A 链路 opc_demand_leads；
    //    交付终态 delivered/failed 已随 B 链路废弃，won/lost 为销售终态）
    let all_leads = axagent_dao::repo::opc_demand::list_leads(db, 10_000, Some(50.0), None)
        .await
        .map_err(|e| format!("读取需求线索失败: {e}"))?;
    let leads: Vec<_> =
        all_leads.iter().filter(|l| l.status != "won" && l.status != "lost").collect();

    let mut keyword_freq: HashMap<String, (usize, f64)> = HashMap::new(); // (出现次数, 累计评分)
    for lead in &leads {
        let text = format!("{} {}", lead.title, lead.description).to_lowercase();
        let score = lead.commercial_value_score;

        // 简单分词：按空格和常见标点
        for word in
            text.split(|c: char| c.is_whitespace() || "，。！？、；：\"'（）【】".contains(c))
        {
            let trimmed = word.trim();
            if trimmed.len() >= 2 && trimmed.len() <= 10 {
                let entry = keyword_freq.entry(trimmed.to_string()).or_insert((0, 0.0));
                entry.0 += 1;
                entry.1 += score;
            }
        }
    }

    // 3) 识别未被现有能力覆盖的高频关键词
    let mut missing_keywords: Vec<(String, usize, f64)> = Vec::new();
    for (keyword, (freq, total_score)) in &keyword_freq {
        let covered = capability_keywords.iter().any(|ck| ck.contains(keyword));
        if !covered && *freq >= 2 {
            missing_keywords.push((keyword.clone(), *freq, *total_score));
        }
    }

    // 按频率排序（降序）
    missing_keywords.sort_by_key(|a| std::cmp::Reverse(a.1));

    // 4) 基于配置的领域关键词分析覆盖情况
    let domain_queries = extract_domain_queries(db).await?;
    let mut domain_coverage = Vec::new();
    for domain_kw in &domain_queries {
        let domain_kw_lower = domain_kw.to_lowercase();
        let covered = capability_keywords.iter().any(|ck| ck.contains(&domain_kw_lower));
        let demand_count = leads
            .iter()
            .filter(|l| {
                let text = format!("{} {}", l.title, l.description).to_lowercase();
                text.contains(&domain_kw_lower)
            })
            .count();

        domain_coverage.push(serde_json::json!({
            "domain": domain_kw,
            "covered": covered,
            "demand_count": demand_count,
        }));
    }

    // 5) 自动创建高优先级缺口（Top 5 高频缺失）
    let auto_created = if missing_keywords.len() >= 2 {
        let mut created = Vec::new();
        for (keyword, freq, _) in missing_keywords.iter().take(5) {
            let gap_id = format!("gap-auto-{}", uuid::Uuid::new_v4().simple());
            let priority = if *freq >= 5 {
                1
            } else if *freq >= 3 {
                2
            } else {
                3
            };

            let result = opc_capability_gap::ActiveModel {
                id: Set(gap_id.clone()),
                lead_id: Set(None),
                title: Set(format!("[主动分析] 高频缺失能力: {}", keyword)),
                description: Set(format!(
                    "关键词 '{}' 在 {} 条高价值需求中出现，但现有能力库未覆盖。建议新增对应能力。",
                    keyword, freq
                )),
                missing_capability: Set(keyword.clone()),
                gap_type: Set("proactive".to_string()),
                suggested_action: Set(format!(
                    "针对 '{}' 领域新增工具/技能/工作流模板，或扫描市场平台获取该领域需求详情",
                    keyword
                )),
                priority: Set(priority),
                status: Set("open".to_string()),
                created_at: Set(now),
                updated_at: Set(now),
                closed_at: Set(None),
            }
            .insert(db)
            .await;

            if result.is_ok() {
                created.push(keyword.clone());
            }
        }
        created
    } else {
        Vec::new()
    };

    // 6) 返回分析结果
    let result = serde_json::json!({
        "total_leads_analyzed": leads.len(),
        "total_capabilities": capability_keywords.len(),
        "missing_keywords_count": missing_keywords.len(),
        "top_missing_keywords": missing_keywords.iter().take(10).map(|(k, f, s)| {
            serde_json::json!({
                "keyword": k,
                "frequency": f,
                "total_score": s,
            })
        }).collect::<Vec<_>>(),
        "domain_coverage": domain_coverage,
        "auto_created_gaps": auto_created,
    });

    serde_json::to_value(&result).map_err(|e| {
        ErrorResponse::from_error(e, crate::commands::error::ErrorCategory::Unrecoverable)
            .to_string()
    })
}

/// 执行需求交付工作流（为线索创建交付记录并触发工作流）
///
/// 线索读取统一走 A 链路 repo（`opc_demand_leads` 表）。交付进度单一事实
/// 来源是 `opc_delivery` 表自身（pending → completed/failed/cancelled），
/// 不再回写 lead 状态 —— lead 表状态机（new/evaluated/contacted/won/lost）
/// 属销售转化线，与交付执行线解耦。
#[agent_command(domain = "automation", safety = Caution, call_mode = StateInput, description = "执行需求交付")]
#[tauri::command]
pub async fn opc_execute_demand_workflow(
    state: State<'_, AppState>,
    lead_id: String,
    workflow_template_id: Option<String>,
) -> Result<serde_json::Value, String> {
    use axagent_entities::opc_delivery;
    use sea_orm::*;

    let db_owned = state.harness.db().clone();
    let db = &db_owned;
    let now = chrono::Utc::now().timestamp();

    let lead = axagent_dao::repo::opc_demand::get_lead(db, &lead_id).await.map_err(|e| {
        ErrorResponse::from_error(e, crate::commands::error::ErrorCategory::Unrecoverable)
            .to_string()
    })?;

    let delivery_id = format!("dv-{}", uuid::Uuid::new_v4().simple());

    let template_id = workflow_template_id.unwrap_or_else(|| {
        lead.linked_workflow_id.clone().unwrap_or_else(|| "default_demand_delivery".to_string())
    });

    let entity = opc_delivery::ActiveModel {
        id: Set(delivery_id.clone()),
        lead_id: Set(Some(lead_id.clone())),
        // A 表无 project/customer 归属（B 链路遗留字段，随表废弃）
        project_id: Set(None),
        customer_id: Set(None),
        title: Set(format!("交付: {}", lead.title)),
        workflow_template_id: Set(template_id.clone()),
        description: Set(lead.description.clone()),
        status: Set("pending".to_string()),
        progress: Set(0.0),
        started_at: Set(Some(now)),
        completed_at: Set(None),
        result_summary: Set(None),
        deliverables_json: Set("[]".to_string()),
        errors_json: Set("[]".to_string()),
        metadata_json: Set("{}".to_string()),
        created_at: Set(now),
        updated_at: Set(now),
    };

    let saved = entity.insert(db).await.map_err(|e| {
        ErrorResponse::from_error(e, crate::commands::error::ErrorCategory::Unrecoverable)
            .to_string()
    })?;

    // 真正调用工作流引擎下发（异步后台执行，不阻塞交付记录创建）
    let engine = std::sync::Arc::clone(&state.work_engine);
    let delivery_id_for_task = delivery_id.clone();
    let lead_title = saved.title.clone();
    let lead_desc = saved.description.clone();

    use axagent_harness::workflow_types::Variable;
    use axagent_rt_workflow::work_engine::RunOptions;

    let variables = vec![
        Variable {
            name: "lead_id".into(),
            var_type: "string".into(),
            value: serde_json::Value::String(lead_id.clone()),
            description: Some("需求线索 ID".into()),
            is_secret: false,
        },
        Variable {
            name: "delivery_id".into(),
            var_type: "string".into(),
            value: serde_json::Value::String(delivery_id_for_task.clone()),
            description: Some("交付记录 ID".into()),
            is_secret: false,
        },
        Variable {
            name: "demand_title".into(),
            var_type: "string".into(),
            value: serde_json::Value::String(lead_title.clone()),
            description: Some("需求标题".into()),
            is_secret: false,
        },
        Variable {
            name: "demand_description".into(),
            var_type: "string".into(),
            value: serde_json::Value::String(lead_desc.clone()),
            description: Some("需求描述".into()),
            is_secret: false,
        },
    ];

    let opts = RunOptions {
        max_concurrent: 2,
        step_timeout: std::time::Duration::from_secs(300),
        tool_timeout: std::time::Duration::from_secs(60),
        variables: Some(variables),
        progress_callback: None,
        ..Default::default()
    };

    let wf_id = template_id;
    let db_for_task = db_owned.clone();
    tauri::async_runtime::spawn(async move {
        let result = engine.run_workflow(&wf_id, opts).await;
        let task_now = chrono::Utc::now().timestamp();
        match result {
            Ok(wf) => {
                use axagent_entities::opc_delivery;
                use sea_orm::*;
                let _ = opc_delivery::Entity::update_many()
                    .col_expr(opc_delivery::Column::Status, Expr::value("completed"))
                    .col_expr(opc_delivery::Column::Progress, Expr::value(1.0))
                    .col_expr(opc_delivery::Column::CompletedAt, Expr::value(task_now))
                    .col_expr(opc_delivery::Column::UpdatedAt, Expr::value(task_now))
                    .col_expr(
                        opc_delivery::Column::ResultSummary,
                        Expr::value(format!("工作流已完成，节点数: {}", wf.nodes.len())),
                    )
                    .filter(opc_delivery::Column::Id.eq(&delivery_id_for_task))
                    .exec(&db_for_task)
                    .await
                    .ok();
            },
            Err(e) => {
                use axagent_entities::opc_delivery;
                use sea_orm::*;
                let _ = opc_delivery::Entity::update_many()
                    .col_expr(opc_delivery::Column::Status, Expr::value("failed"))
                    .col_expr(opc_delivery::Column::CompletedAt, Expr::value(task_now))
                    .col_expr(opc_delivery::Column::UpdatedAt, Expr::value(task_now))
                    .col_expr(
                        opc_delivery::Column::ErrorsJson,
                        Expr::value(serde_json::json!([{ "workflow": e.to_string() }]).to_string()),
                    )
                    .filter(opc_delivery::Column::Id.eq(&delivery_id_for_task))
                    .exec(&db_for_task)
                    .await
                    .ok();
            },
        }
    });

    serde_json::to_value(&saved).map_err(|e| {
        ErrorResponse::from_error(e, crate::commands::error::ErrorCategory::Unrecoverable)
            .to_string()
    })
}

/// 列出交付记录（支持按状态/线索ID过滤）
#[agent_command(domain = "automation", safety = Safe, call_mode = StateInput, description = "列出交付记录")]
#[tauri::command]
pub async fn opc_list_deliveries(
    state: State<'_, AppState>,
    status: Option<String>,
    lead_id: Option<String>,
) -> Result<serde_json::Value, String> {
    use axagent_entities::opc_delivery;
    use sea_orm::*;

    let db = state.harness.db();
    let mut qs = opc_delivery::Entity::find();

    if let Some(ref s) = status {
        qs = qs.filter(opc_delivery::Column::Status.eq(s));
    }
    if let Some(ref l) = lead_id {
        qs = qs.filter(opc_delivery::Column::LeadId.eq(l));
    }

    let results = qs.order_by_desc(opc_delivery::Column::CreatedAt).all(db).await.map_err(|e| {
        ErrorResponse::from_error(e, crate::commands::error::ErrorCategory::Unrecoverable)
            .to_string()
    })?;

    serde_json::to_value(&results).map_err(|e| {
        ErrorResponse::from_error(e, crate::commands::error::ErrorCategory::Unrecoverable)
            .to_string()
    })
}

/// 获取单个交付详情
#[agent_command(domain = "automation", safety = Safe, call_mode = StateInput, description = "获取交付详情")]
#[tauri::command]
pub async fn opc_get_delivery(
    state: State<'_, AppState>,
    id: String,
) -> Result<serde_json::Value, String> {
    use axagent_entities::opc_delivery;
    use sea_orm::*;

    let db = state.harness.db();
    let result = opc_delivery::Entity::find_by_id(&id)
        .one(db)
        .await
        .map_err(|e| {
            ErrorResponse::from_error(e, crate::commands::error::ErrorCategory::Unrecoverable)
                .to_string()
        })?
        .ok_or_else(|| format!("交付记录不存在: {id}"))?;

    serde_json::to_value(&result).map_err(|e| {
        ErrorResponse::from_error(e, crate::commands::error::ErrorCategory::Unrecoverable)
            .to_string()
    })
}

/// 更新交付状态（工作流执行完成后回调）
#[agent_command(domain = "automation", safety = Caution, call_mode = StateInput, description = "更新交付状态")]
#[tauri::command]
pub async fn opc_update_delivery(
    state: State<'_, AppState>,
    id: String,
    status: String,
    progress: Option<f64>,
    result_summary: Option<String>,
    deliverables: Option<serde_json::Value>,
    errors: Option<serde_json::Value>,
) -> Result<serde_json::Value, String> {
    use axagent_entities::opc_delivery;
    use sea_orm::*;

    let db = state.harness.db();
    let now = chrono::Utc::now().timestamp();

    let result = opc_delivery::Entity::find_by_id(&id)
        .one(db)
        .await
        .map_err(|e| {
            ErrorResponse::from_error(e, crate::commands::error::ErrorCategory::Unrecoverable)
                .to_string()
        })?
        .ok_or_else(|| format!("交付记录不存在: {id}"))?;

    let mut am: opc_delivery::ActiveModel = result.into();
    am.status = Set(status.clone());
    if let Some(p) = progress {
        am.progress = Set(p);
    }
    if let Some(ref summary) = result_summary {
        am.result_summary = Set(Some(summary.clone()));
    }
    if let Some(d) = deliverables {
        am.deliverables_json = Set(serde_json::to_string(&d).unwrap_or_default());
    }
    if let Some(e) = errors {
        am.errors_json = Set(serde_json::to_string(&e).unwrap_or_default());
    }
    if status == "completed" || status == "failed" {
        am.completed_at = Set(Some(now));
    }
    am.updated_at = Set(now);

    let saved = am.update(db).await.map_err(|e| {
        ErrorResponse::from_error(e, crate::commands::error::ErrorCategory::Unrecoverable)
            .to_string()
    })?;

    serde_json::to_value(&saved).map_err(|e| {
        ErrorResponse::from_error(e, crate::commands::error::ErrorCategory::Unrecoverable)
            .to_string()
    })
}

/// 重试失败的交付任务
#[agent_command(domain = "automation", safety = Caution, call_mode = StateInput, description = "重试交付任务")]
#[tauri::command]
pub async fn opc_retry_delivery(
    state: State<'_, AppState>,
    id: String,
) -> Result<serde_json::Value, String> {
    use axagent_entities::opc_delivery;
    use sea_orm::*;

    let db = state.harness.db();
    let now = chrono::Utc::now().timestamp();

    let result = opc_delivery::Entity::find_by_id(&id)
        .one(db)
        .await
        .map_err(|e| {
            ErrorResponse::from_error(e, crate::commands::error::ErrorCategory::Unrecoverable)
                .to_string()
        })?
        .ok_or_else(|| format!("交付记录不存在: {id}"))?;

    let mut am: opc_delivery::ActiveModel = result.into();
    am.status = Set("pending".to_string());
    am.progress = Set(0.0);
    am.result_summary = Set(None);
    am.completed_at = Set(None);
    am.errors_json = Set("[]".to_string());
    am.updated_at = Set(now);

    let saved = am.update(db).await.map_err(|e| {
        ErrorResponse::from_error(e, crate::commands::error::ErrorCategory::Unrecoverable)
            .to_string()
    })?;

    serde_json::to_value(&saved).map_err(|e| {
        ErrorResponse::from_error(e, crate::commands::error::ErrorCategory::Unrecoverable)
            .to_string()
    })
}

/// 取消进行中的交付任务
#[agent_command(domain = "automation", safety = Caution, call_mode = StateInput, description = "取消交付任务")]
#[tauri::command]
pub async fn opc_cancel_delivery(
    state: State<'_, AppState>,
    id: String,
) -> Result<serde_json::Value, String> {
    use axagent_entities::opc_delivery;
    use sea_orm::*;

    let db = state.harness.db();
    let now = chrono::Utc::now().timestamp();

    let result = opc_delivery::Entity::find_by_id(&id)
        .one(db)
        .await
        .map_err(|e| {
            ErrorResponse::from_error(e, crate::commands::error::ErrorCategory::Unrecoverable)
                .to_string()
        })?
        .ok_or_else(|| format!("交付记录不存在: {id}"))?;

    let mut am: opc_delivery::ActiveModel = result.into();
    am.status = Set("cancelled".to_string());
    am.completed_at = Set(Some(now));
    am.updated_at = Set(now);

    let saved = am.update(db).await.map_err(|e| {
        ErrorResponse::from_error(e, crate::commands::error::ErrorCategory::Unrecoverable)
            .to_string()
    })?;

    serde_json::to_value(&saved).map_err(|e| {
        ErrorResponse::from_error(e, crate::commands::error::ErrorCategory::Unrecoverable)
            .to_string()
    })
}
