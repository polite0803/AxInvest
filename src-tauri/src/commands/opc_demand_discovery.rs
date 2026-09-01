// SPDX-License-Identifier: AGPL-3.0-only

//! OPC 需求发现命令层
//!
//! 接线「平台配置 → 扫描器 → 评估 → 持久化 → 查询」完整链路：
//! - 平台配置 CRUD：`opc_list_platforms` / `opc_save_platform` / `opc_delete_platform`
//! - 扫描执行：`opc_discover_and_evaluate_leads`（按 DB 配置装配扫描器，
//!   并发扫描 → 评估 → 按去重窗口入库 → 回写平台同步状态）
//! - 线索查询：`opc_list_leads`（按商业价值分降序）
//! - 扫描策略：`opc_get_scan_policy` / `opc_save_scan_policy`
//!
//! 评估与扫描实现在 `axagent_tools::tools::marketplace_scanner`；
//! 扫描策略（并发/限流/重试/去重窗口）在 `axagent_tools::tools::scan_policy`；
//! 数据落地在 `axagent_dao::repo::opc_demand`（v131）；策略持久化走通用设置表。

use crate::AppState;
use axagent_harness::types::{
    DemandLeadDto, DemandPlatform, DiscoverLeadsSummary, SaveDemandPlatformInput,
};
use axagent_tools::tools::marketplace_scanner::{AggregateMarketplaceScanner, EvaluatedDemandLead};
use axagent_tools::tools::scan_policy::{SCAN_POLICY_SETTING_KEY, ScanPolicy};
use tauri::State;

/// 高价值门槛（与 opportunity_level 的 "high" 档对齐）
const HIGH_VALUE_THRESHOLD: f64 = 60.0;
/// 摘要中返回的高价值线索明细上限
const SUMMARY_LEADS_LIMIT: usize = 20;

/// 列出需求平台配置（表空时自动填充内置默认平台）
#[tauri::command]
pub async fn opc_list_platforms(state: State<'_, AppState>) -> Result<Vec<DemandPlatform>, String> {
    let db = state.harness.db();
    axagent_dao::repo::opc_demand::seed_default_platforms_if_empty(db).await.map_err(err)?;
    axagent_dao::repo::opc_demand::list_platforms(db).await.map_err(err)
}

/// 保存（新增或更新）需求平台配置
#[tauri::command]
pub async fn opc_save_platform(
    state: State<'_, AppState>,
    input: SaveDemandPlatformInput,
) -> Result<DemandPlatform, String> {
    axagent_dao::repo::opc_demand::save_platform(state.harness.db(), input).await.map_err(err)
}

/// 删除需求平台配置
#[tauri::command]
pub async fn opc_delete_platform(state: State<'_, AppState>, id: String) -> Result<(), String> {
    axagent_dao::repo::opc_demand::delete_platform(state.harness.db(), &id).await.map_err(err)
}

/// 列出需求线索（按商业价值分降序）
#[tauri::command]
pub async fn opc_list_leads(
    state: State<'_, AppState>,
    limit: Option<u64>,
    min_score: Option<f64>,
) -> Result<Vec<DemandLeadDto>, String> {
    axagent_dao::repo::opc_demand::list_leads(
        state.harness.db(),
        limit.unwrap_or(100).min(500),
        min_score,
    )
    .await
    .map_err(err)
}

/// 读取当前扫描策略（设置表缺失时返回默认策略）
#[tauri::command]
pub async fn opc_get_scan_policy(state: State<'_, AppState>) -> Result<ScanPolicy, String> {
    load_scan_policy(state.harness.db()).await
}

/// 保存扫描策略（写入通用设置表，值会做范围钳制）
#[tauri::command]
pub async fn opc_save_scan_policy(
    state: State<'_, AppState>,
    policy: ScanPolicy,
) -> Result<ScanPolicy, String> {
    let normalized = policy.normalized();
    let json = serde_json::to_string(&normalized).map_err(|e| e.to_string())?;
    axagent_dao::repo::settings::set_setting(state.harness.db(), SCAN_POLICY_SETTING_KEY, &json)
        .await
        .map_err(err)?;
    tracing::info!(
        concurrency = normalized.concurrency,
        rate_limit = normalized.rate_limit_per_min,
        retry_max = normalized.retry_max,
        dedup_window_hours = normalized.dedup_window_hours,
        "[opc_demand] 扫描策略已保存"
    );
    Ok(normalized)
}

/// 按关键词扫描全部启用平台并评估入库
///
/// 流程：读取 DB 平台配置 + 扫描策略 → 装配聚合扫描器 → 并发扫描（受并发/限流/
/// 重试/超时约束）→ 逐条评估 → 按去重窗口入库 → 回写平台同步状态 → 返回摘要。
#[tauri::command]
pub async fn opc_discover_and_evaluate_leads(
    state: State<'_, AppState>,
    query: String,
) -> Result<DiscoverLeadsSummary, String> {
    let query = query.trim().to_string();
    if query.is_empty() {
        return Err("query 不能为空".to_string());
    }

    let db = state.harness.db();
    let policy = load_scan_policy(db).await?;
    let dedup_window_secs = policy.dedup_window_secs();
    let max_leads = policy.max_leads_per_scan;

    axagent_dao::repo::opc_demand::seed_default_platforms_if_empty(db).await.map_err(err)?;
    let platforms = axagent_dao::repo::opc_demand::list_enabled_platforms(db).await.map_err(err)?;

    // 装配扫描器：无配置行时回退默认（全部内置扫描器）
    let mut scanner = AggregateMarketplaceScanner::with_policy(policy.clone());
    if platforms.is_empty() {
        let mut default_scanner = AggregateMarketplaceScanner::default();
        default_scanner.set_policy(policy);
        scanner = default_scanner;
    } else {
        for p in &platforms {
            let base_url = p.base_url.as_deref();
            let config = &p.config;
            // DemandPlatform.config 是 Value；add_platform 接受引用。
            // 空配置需绑定到具名变量 —— 直接内联 &Value::Object(..) 是临时值，语句结束即 drop（E0716）。
            let empty_config = serde_json::Value::Object(Default::default());
            let cfg = if config.is_null() {
                &empty_config
            } else {
                config
            };
            scanner.add_platform(&p.id, &p.platform_type, base_url, cfg);
        }
    }

    let results = scanner.scan_and_evaluate_platforms(&query).await;
    let mut summary = DiscoverLeadsSummary::default();
    // 逐平台的同步状态：platform → 是否成功
    let mut platform_status: Vec<(String, bool, bool)> = Vec::new(); // (platform, ok, compliance_skipped)

    for result in results {
        platform_status.push((
            result.platform.clone(),
            result.error.is_none(),
            result.compliance_skipped,
        ));

        if let Some(e) = &result.error {
            tracing::warn!(
                platform = result.platform,
                attempts = result.attempts,
                error = %e,
                "[opc_demand] 平台扫描失败"
            );
        }

        for evaluated in result.leads {
            if summary.total_scanned as usize >= max_leads {
                break;
            }
            summary.total_scanned += 1;
            match persist_evaluated(db, &evaluated, dedup_window_secs).await? {
                axagent_dao::repo::opc_demand::LeadWriteOutcome::Inserted => {
                    summary.total_saved += 1;
                },
                axagent_dao::repo::opc_demand::LeadWriteOutcome::Refreshed => {
                    summary.total_refreshed += 1;
                },
                axagent_dao::repo::opc_demand::LeadWriteOutcome::Skipped => {},
            }
            summary.total_evaluated += 1;
            if evaluated.value_score() >= HIGH_VALUE_THRESHOLD {
                summary.high_value_count += 1;
            }
        }
    }

    // 高价值线索明细
    summary.leads = axagent_dao::repo::opc_demand::list_leads(
        db,
        SUMMARY_LEADS_LIMIT as u64,
        Some(HIGH_VALUE_THRESHOLD),
    )
    .await
    .map_err(err)?;

    // 回写平台同步状态（单平台失败不阻断整体结果）
    // 合规跳过不算失败：无凭证是配置状态，不是运行故障。
    for (platform_id, ok, compliance_skipped) in &platform_status {
        let final_ok = *ok || *compliance_skipped;
        if let Err(e) =
            axagent_dao::repo::opc_demand::mark_platform_synced(db, platform_id, final_ok).await
        {
            tracing::warn!(platform = platform_id, error = %e, "[opc_demand] 更新同步状态失败");
        }
    }

    tracing::info!(
        scanned = summary.total_scanned,
        saved = summary.total_saved,
        refreshed = summary.total_refreshed,
        high_value = summary.high_value_count,
        "[opc_demand] 扫描评估完成"
    );
    Ok(summary)
}

/// 从通用设置表读取扫描策略；缺失或解析失败时返回默认策略
async fn load_scan_policy(db: &sea_orm::DatabaseConnection) -> Result<ScanPolicy, String> {
    match axagent_dao::repo::settings::get_setting(db, SCAN_POLICY_SETTING_KEY)
        .await
        .map_err(err)?
    {
        Some(json) => Ok(ScanPolicy::from_json(&json)),
        None => Ok(ScanPolicy::default()),
    }
}

/// 持久化一条评估后的线索，返回写入结果（新入库 / 刷新 / 跳过）
async fn persist_evaluated(
    db: &sea_orm::DatabaseConnection,
    evaluated: &EvaluatedDemandLead,
    dedup_window_secs: Option<i64>,
) -> Result<axagent_dao::repo::opc_demand::LeadWriteOutcome, String> {
    let lead = &evaluated.lead;
    let evaluation = &evaluated.evaluation;
    let row = axagent_dao::repo::opc_demand::NewLeadRow {
        id: lead.id.clone(),
        platform: lead.platform.clone(),
        title: lead.title.clone(),
        description: lead.description.clone(),
        budget_min: lead.budget_min,
        budget_max: lead.budget_max,
        budget_currency: lead.budget_currency.clone(),
        contact_name: lead.contact_name.clone(),
        contact_email: lead.contact_email.clone(),
        contact_phone: lead.contact_phone.clone(),
        source_url: lead.source_url.clone(),
        raw_snapshot: lead.raw_snapshot.clone(),
        confidence: evaluation.confidence(),
        pain_score: evaluation.pain_score(),
        market_gap_score: evaluation.market_gap_score(),
        commercial_value_score: evaluation.commercial_value_score(),
        demand_type: evaluation.demand_type().as_str().to_string(),
    };
    axagent_dao::repo::opc_demand::upsert_lead_within_window(db, row, dedup_window_secs)
        .await
        .map_err(err)
}

/// DAO 错误 → 命令层错误串（走错误码映射层）
fn err(e: axagent_harness::core_error::AxAgentError) -> String {
    String::from(crate::commands::error::ErrorResponse::from_error(
        e,
        crate::commands::error::ErrorCategory::Unrecoverable,
    ))
}
