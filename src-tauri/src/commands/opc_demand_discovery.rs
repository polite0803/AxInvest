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
use crate::commands::error_code::common as common_err;
use crate::commands::error_code::opc_setup as opc_setup_err;
use axagent_agent_macro::agent_command;
use axagent_harness::types::{
    DemandLeadDto, DemandPlatform, DiscoverLeadsSummary, SaveDemandLeadInput,
    SaveDemandPlatformInput,
};
use axagent_tools::tools::marketplace_scanner::{
    DemandLead, EvaluatedDemandLead, RawLead, evaluate_lead,
};
use axagent_tools::tools::scan_policy::{SCAN_POLICY_SETTING_KEY, ScanPolicy};
use tauri::State;

// 高价值门槛 / 摘要明细上限 / 扫描核心逻辑均已下沉到
// `axagent_tools::tools::opc_demand_scan`（单一权威来源，工作流工具节点共用）。

/// 列出需求平台配置（表空时自动填充内置默认平台）
#[agent_command(domain = "automation", safety = Safe, call_mode = StateOnly, description = "列出市场平台配置")]
#[tauri::command]
pub async fn opc_list_platforms(state: State<'_, AppState>) -> Result<Vec<DemandPlatform>, String> {
    let db = state.harness.db();
    axagent_dao::repo::opc_demand::seed_default_platforms_if_empty(db).await.map_err(err)?;
    axagent_dao::repo::opc_demand::list_platforms(db).await.map_err(err)
}

/// 保存（新增或更新）需求平台配置
#[agent_command(domain = "automation", safety = Caution, call_mode = StateInput, description = "保存平台配置")]
#[tauri::command]
pub async fn opc_save_platform(
    state: State<'_, AppState>,
    input: SaveDemandPlatformInput,
) -> Result<DemandPlatform, String> {
    axagent_dao::repo::opc_demand::save_platform(state.harness.db(), input).await.map_err(err)
}

/// 删除需求平台配置
#[agent_command(domain = "automation", safety = Caution, call_mode = StateInput, description = "删除平台配置")]
#[tauri::command]
pub async fn opc_delete_platform(state: State<'_, AppState>, id: String) -> Result<(), String> {
    axagent_dao::repo::opc_demand::delete_platform(state.harness.db(), &id).await.map_err(err)
}

/// 列出需求线索（按商业价值分降序，可按生命周期状态过滤）
#[agent_command(domain = "automation", safety = Safe, call_mode = StateInput, description = "列出需求线索")]
#[tauri::command]
pub async fn opc_list_leads(
    state: State<'_, AppState>,
    limit: Option<u64>,
    min_score: Option<f64>,
    status: Option<String>,
) -> Result<Vec<DemandLeadDto>, String> {
    axagent_dao::repo::opc_demand::list_leads(
        state.harness.db(),
        limit.unwrap_or(100).min(500),
        min_score,
        status,
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
    let json = serde_json::to_string(&normalized).map_err(serialize_err)?;
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

/// 扫描核心的 Tauri 薄壳 —— 逻辑在 [`axagent_tools::tools::opc_demand_scan::
/// run_discovery_scan`]（装配扫描器 → 并发扫描 → 评估 → 入库 → 回写平台状态）。
///
/// 订阅定时扫描（`commands::opc_demand_subscription`）复用同一入口，
/// 避免扫描器装配与去重入库规则在两处漂移。
#[agent_command(domain = "automation", safety = Safe, call_mode = StateInput, description = "扫描并评估需求线索")]
#[tauri::command]
pub async fn opc_discover_and_evaluate_leads(
    state: State<'_, AppState>,
    query: String,
) -> Result<DiscoverLeadsSummary, String> {
    let query = query.trim().to_string();
    if query.is_empty() {
        return Err(crate::commands::error::ErrorResponse::err_with_detail(
            common_err::INVALID_INPUT,
            "query 不能为空",
        ));
    }
    run_discovery_for_query(state.harness.db(), &query, &[]).await
}

/// 扫描核心委托（供本命令与订阅定时扫描共用）
pub(crate) async fn run_discovery_for_query(
    db: &sea_orm::DatabaseConnection,
    query: &str,
    platform_filter: &[String],
) -> Result<DiscoverLeadsSummary, String> {
    axagent_tools::tools::opc_demand_scan::run_discovery_scan(db, query, platform_filter)
        .await
        .map_err(err)
}

/// 从通用设置表读取扫描策略（逻辑在 tools 层，此处仅做错误映射）
async fn load_scan_policy(db: &sea_orm::DatabaseConnection) -> Result<ScanPolicy, String> {
    axagent_tools::tools::opc_demand_scan::load_scan_policy(db).await.map_err(err)
}

/// 手动补录平台的固定 platform 标识
const MANUAL_PLATFORM: &str = "manual";

/// 手动补录一条需求线索（P1-4）
///
/// 复用扫描管线的归一化/评分/去重逻辑：`RawLead → new_from_raw → evaluate_lead
/// → upsert_lead_within_window`。手动填写的预算与 URL 覆盖自动提取结果；
/// 去重命中（窗口内同指纹/同 URL）时返回既有生效行而非报错。
#[agent_command(domain = "automation", safety = Caution, call_mode = StateInput, description = "创建需求线索")]
#[tauri::command]
pub async fn opc_create_lead(
    state: State<'_, AppState>,
    input: SaveDemandLeadInput,
) -> Result<DemandLeadDto, String> {
    let title = input.title.trim().to_string();
    let description = input.description.trim().to_string();
    if title.is_empty() || description.is_empty() {
        return Err(crate::commands::error::ErrorResponse::err_with_detail(
            common_err::INVALID_INPUT,
            "title 与 description 不能为空",
        ));
    }

    let db = state.harness.db();
    let policy = load_scan_policy(db).await?;

    // URL 清洗：空串归一为 None（否则 new_from_raw 会存出 Some("")）
    let source_url =
        input.source_url.clone().map(|s| s.trim().to_string()).filter(|s| !s.is_empty());

    let raw = RawLead {
        platform: MANUAL_PLATFORM.to_string(),
        title,
        description,
        url: source_url.clone().unwrap_or_default(),
        price_text: None, // 手动预算走结构化字段，不走价格文本解析
        contact: input.contact_name.clone(),
        contact_email: input.contact_email.clone(),
        contact_phone: input.contact_phone.clone(),
        snapshot: serde_json::json!({ "source": "manual" }),
    };
    let mut lead = DemandLead::new_from_raw(raw);
    // 手动填写的预算覆盖自动解析（用户没填的字段保留自动提取结果）
    if input.budget_min.is_some() || input.budget_max.is_some() {
        lead.budget_min = input.budget_min;
        lead.budget_max = input.budget_max;
    }
    if let Some(currency) =
        input.budget_currency.as_deref().map(str::trim).filter(|c| !c.is_empty())
    {
        lead.budget_currency = currency.to_string();
    }
    lead.source_url = source_url;

    let evaluation = evaluate_lead(&lead);
    let evaluated = EvaluatedDemandLead { lead, evaluation };
    axagent_dao::repo::opc_demand::create_manual_lead(
        db,
        axagent_tools::tools::opc_demand_scan::evaluated_to_row(&evaluated),
        policy.dedup_window_secs(),
    )
    .await
    .map_err(err)
}

/// 序列化等内部错误 → 命令层错误串（OPC 设置域错误码 + 技术详情）
fn serialize_err(e: impl std::fmt::Display) -> String {
    crate::commands::error::ErrorResponse::err_with_detail(
        opc_setup_err::INTERNAL,
        format!("序列化失败: {e}"),
    )
}

/// DAO 错误 → 命令层错误串（走错误码映射层）
fn err(e: axagent_harness::core_error::AxAgentError) -> String {
    String::from(crate::commands::error::ErrorResponse::from_error(
        e,
        crate::commands::error::ErrorCategory::Unrecoverable,
    ))
}
