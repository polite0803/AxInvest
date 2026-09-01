// SPDX-License-Identifier: AGPL-3.0-only

//! OPC 需求订阅命令层（v133，需求全链路审计断点 3）
//!
//! 把「发现」从手动单关键词点按钮升级为**订阅词表驱动的定时扫描**：
//!
//! - 订阅 CRUD：`opc_list_subscriptions` / `opc_save_subscription` / `opc_delete_subscription`
//! - 批量扫描：`opc_run_subscription_scan`（挑出到期订阅 → 复用 [`run_discovery_for_query`]
//!   → 按 `min_score` 过滤高价值命中 → 推进到期时间）
//! - 定时任务装配：`opc_ensure_demand_scan_job`（幂等创建/更新
//!   `task_type = "opc_demand_scan"` 的 CronJob，带 delivery 推送配置）
//!
//! ## 设计决策
//!
//! - **扫描逻辑不复制**：直接复用 `commands::opc_demand_discovery::run_discovery_for_query`，
//!   避免扫描器装配与去重入库规则在两处漂移（AGENTS.md 禁区 12）。
//! - **推送门槛 vs 入库门槛**：线索无论分数都会入库，`min_score` 只决定
//!   是否计入高价值命中并触发推送 —— 低分线索仍然留在库里供人工翻查。
//! - **失败也推进到期时间**：平台限流/网络失败时不重扫，否则每个 tick 都会
//!   重试打爆限流（扫描策略已有 retry，这里不再叠加）。错误记入 outcome。
//! - **定时启停用既有命令**：`pause_scheduled_task` / `resume_scheduled_task` 已足够，
//!   不新增停止命令。

use crate::AppState;
use crate::commands::error_code::common as common_err;
use crate::commands::opc_demand_discovery::run_discovery_for_query;
use crate::commands::scheduled_task::{ScheduledTaskDto, cron_to_dto, validate_cron_expression};
use axagent_harness::cron_delivery::{CronDeliveryChannel, CronDeliveryConfig};
use axagent_harness::types::{
    DemandSubscription, KeywordScanOutcome, SaveDemandSubscriptionInput, SubscriptionScanSummary,
};
use axagent_harness::util_fns::now_ts;
use axagent_runtime_core::{CronJob, CronJobStatus};
use tauri::State;

/// `opc_demand_scan` 定时任务的 task_type 标识（scheduler 分支键）
pub const SCAN_JOB_TASK_TYPE: &str = "opc_demand_scan";
/// 默认扫描频率：每 6 小时一次（00/06/12/18 点整）
pub const DEFAULT_SCAN_CRON: &str = "0 */6 * * *";

/// 列出全部需求订阅
#[tauri::command]
pub async fn opc_list_subscriptions(
    state: State<'_, AppState>,
) -> Result<Vec<DemandSubscription>, String> {
    axagent_dao::repo::opc_demand::list_subscriptions(state.harness.db()).await.map_err(err)
}

/// 新增或更新需求订阅（`id` 为空则新增）
#[tauri::command]
pub async fn opc_save_subscription(
    state: State<'_, AppState>,
    input: SaveDemandSubscriptionInput,
) -> Result<DemandSubscription, String> {
    axagent_dao::repo::opc_demand::save_subscription(state.harness.db(), input).await.map_err(err)
}

/// 删除需求订阅
#[tauri::command]
pub async fn opc_delete_subscription(state: State<'_, AppState>, id: String) -> Result<(), String> {
    axagent_dao::repo::opc_demand::delete_subscription(state.harness.db(), &id).await.map_err(err)
}

/// 执行一轮订阅扫描
///
/// `only_due` 默认 `true`：只扫到期订阅（间隔未到的跳过）。传 `false` 可强制
/// 全量重扫（前端「立即扫描」按钮用）。
///
/// 单个订阅词扫描失败不影响其他词 —— 失败会记入对应 outcome 的 `error` 字段。
#[tauri::command]
pub async fn opc_run_subscription_scan(
    state: State<'_, AppState>,
    only_due: Option<bool>,
) -> Result<SubscriptionScanSummary, String> {
    let db = state.harness.db();
    let now = now_ts();
    let subs = if only_due.unwrap_or(true) {
        axagent_dao::repo::opc_demand::list_due_subscriptions(db, now).await
    } else {
        axagent_dao::repo::opc_demand::list_subscriptions(db)
            .await
            .map(|rows| rows.into_iter().filter(|s| s.enabled).collect())
    }
    .map_err(err)?;

    Ok(run_scan(db, subs).await)
}

/// scheduler 分支入口：只扫到期订阅，返回「摘要 + 可投递的文本」
///
/// 与命令层 [`opc_run_subscription_scan`] 共用 [`run_scan`]，只是额外把结果
/// 渲染成投递文本（写入执行历史 / 推送到 webhook 等渠道）。
pub async fn run_scan_for_scheduler(
    db: &sea_orm::DatabaseConnection,
) -> Result<(SubscriptionScanSummary, String), String> {
    let subs =
        axagent_dao::repo::opc_demand::list_due_subscriptions(db, now_ts()).await.map_err(err)?;
    let summary = run_scan(db, subs).await;
    let text = render_scan_text(&summary);
    Ok((summary, text))
}

/// 执行一轮订阅扫描（挑词逻辑由调用方决定）
async fn run_scan(
    db: &sea_orm::DatabaseConnection,
    subs: Vec<DemandSubscription>,
) -> SubscriptionScanSummary {
    let mut summary =
        SubscriptionScanSummary { scanned_subscriptions: subs.len() as u32, ..Default::default() };

    for sub in subs {
        let scan = scan_one_subscription(db, &sub).await;
        summary.total_saved += scan.saved;
        summary.total_refreshed += scan.refreshed;
        summary.high_value_hits += scan.outcome.hits.len() as u32;
        summary.outcomes.push(scan.outcome);
    }

    tracing::info!(
        subscriptions = summary.scanned_subscriptions,
        high_value_hits = summary.high_value_hits,
        "[opc_demand] 订阅扫描完成"
    );
    summary
}

/// 把扫描结果渲染成投递文本（写入执行历史 / 推送渠道）
fn render_scan_text(summary: &SubscriptionScanSummary) -> String {
    use axagent_harness::util_fns::truncate_to_char_boundary;

    /// 投递文本中列出的命中线索上限（防大结果撑爆 webhook body）
    const MAX_HITS_IN_TEXT: usize = 10;

    let mut text = format!(
        "需求订阅扫描完成：{} 个订阅，新入库 {} 条，刷新 {} 条，高价值命中 {} 条",
        summary.scanned_subscriptions,
        summary.total_saved,
        summary.total_refreshed,
        summary.high_value_hits,
    );

    let failed: Vec<&KeywordScanOutcome> = summary.outcomes.iter().filter(|o| !o.ok).collect();
    if !failed.is_empty() {
        text.push_str(&format!("\n失败 {} 个订阅：", failed.len()));
        for o in failed.iter().take(5) {
            text.push_str(&format!(
                "\n  - {}: {}",
                o.keyword,
                o.error.as_deref().unwrap_or("未知错误")
            ));
        }
    }

    let hits: Vec<&axagent_harness::types::DemandLeadDto> =
        summary.outcomes.iter().flat_map(|o| o.hits.iter()).collect();
    if !hits.is_empty() {
        text.push_str("\n高价值命中：");
        for lead in hits.iter().take(MAX_HITS_IN_TEXT) {
            text.push_str(&format!(
                "\n  - [{}分] {}（{}）",
                lead.commercial_value_score.round(),
                truncate_to_char_boundary(&lead.title, 80),
                lead.platform
            ));
        }
        if hits.len() > MAX_HITS_IN_TEXT {
            text.push_str(&format!("\n  ... 另有 {} 条", hits.len() - MAX_HITS_IN_TEXT));
        }
    }

    text
}

/// 单个订阅词的扫描结果（内部用：DTO outcome + 入库计数）
struct SubscriptionScanOutcome {
    outcome: KeywordScanOutcome,
    saved: u32,
    refreshed: u32,
}

/// 扫描单个订阅词并推进其到期时间
async fn scan_one_subscription(
    db: &sea_orm::DatabaseConnection,
    sub: &DemandSubscription,
) -> SubscriptionScanOutcome {
    match run_discovery_for_query(db, &sub.keyword, &sub.platforms).await {
        Ok(result) => {
            let hits: Vec<_> = result
                .round_leads
                .into_iter()
                .filter(|lead| lead.commercial_value_score >= sub.min_score)
                .collect();
            let hit_count = hits.len() as i32;
            // 失败也推进到期（见模块文档）：此处是成功路径，正常推进
            if let Err(e) =
                axagent_dao::repo::opc_demand::mark_subscription_scanned(db, &sub.id, hit_count)
                    .await
            {
                tracing::warn!(
                    subscription = %sub.keyword,
                    error = %e,
                    "[opc_demand] 推进订阅到期时间失败"
                );
            }
            SubscriptionScanOutcome {
                outcome: KeywordScanOutcome {
                    subscription_id: sub.id.clone(),
                    keyword: sub.keyword.clone(),
                    ok: true,
                    error: None,
                    hits,
                },
                saved: result.total_saved,
                refreshed: result.total_refreshed,
            }
        },
        Err(e) => {
            tracing::warn!(
                subscription = %sub.keyword,
                error = %e,
                "[opc_demand] 订阅扫描失败"
            );
            // 同样推进到期时间：避免失败订阅在每个 tick 被重试打爆限流
            if let Err(e) =
                axagent_dao::repo::opc_demand::mark_subscription_scanned(db, &sub.id, 0).await
            {
                tracing::warn!(
                    subscription = %sub.keyword,
                    error = %e,
                    "[opc_demand] 失败订阅推进到期时间失败（可能反复重扫）"
                );
            }
            SubscriptionScanOutcome {
                outcome: KeywordScanOutcome {
                    subscription_id: sub.id.clone(),
                    keyword: sub.keyword.clone(),
                    ok: false,
                    error: Some(e),
                    hits: Vec::new(),
                },
                saved: 0,
                refreshed: 0,
            }
        },
    }
}

/// 幂等创建/更新「需求订阅扫描」定时任务
///
/// 已存在 `task_type = "opc_demand_scan"` 的任务则更新其 cron 与投递配置并激活，
/// 否则新建。停止/恢复复用既有 `pause_scheduled_task` / `resume_scheduled_task`。
///
/// 投递渠道（`webhook_url` / `file_path` 至少给一个才推送）：
/// - Webhook：HTTP POST 完整 payload（可配 HMAC 签名，本命令不暴露 secret）
/// - File：追加到本地文件（支持 `~/` 前缀）
///
/// 未配置任何渠道时 `delivery.channels` 为空 → 不推送，扫描结果只写执行历史。
#[tauri::command]
pub async fn opc_ensure_demand_scan_job(
    state: State<'_, AppState>,
    cron_expression: Option<String>,
    webhook_url: Option<String>,
    file_path: Option<String>,
) -> Result<ScheduledTaskDto, String> {
    let cron = cron_expression.unwrap_or_else(|| DEFAULT_SCAN_CRON.to_string());
    validate_cron_expression(&cron)?;

    let delivery = build_delivery_config(webhook_url, file_path);
    let store = state.cron_job_store.clone();

    let existing =
        store.list().await.into_iter().find(|j| j.task_type.as_deref() == Some(SCAN_JOB_TASK_TYPE));

    let job = match existing {
        Some(found) => {
            store
                .update(&found.id, |job| {
                    job.schedule = cron.clone();
                    job.delivery = Some(delivery);
                    job.status = CronJobStatus::Active;
                    job.next_run_at = None;
                })
                .await;
            store.get(&found.id).await.ok_or_else(|| {
                crate::commands::error::ErrorResponse::err_with_detail(
                    common_err::INVALID_INPUT,
                    format!("定时任务 {} 更新后无法读回", found.id),
                )
            })?
        },
        None => {
            let job = CronJob::new(
                "OPC 需求订阅扫描",
                &cron,
                "按订阅词表扫描需求平台，命中推送门槛的线索走 delivery 推送",
                "OPC 需求发现定时扫描",
            )
            .with_task_type(SCAN_JOB_TASK_TYPE)
            .with_delivery(delivery);
            let id = store.add(job).await;
            store.get(&id).await.ok_or_else(|| {
                crate::commands::error::ErrorResponse::err_with_detail(
                    common_err::INVALID_INPUT,
                    format!("定时任务 {id} 创建后无法读回"),
                )
            })?
        },
    };

    tracing::info!(
        job_id = job.id,
        schedule = %cron,
        channels = job.delivery.as_ref().map(|d| d.channels.len()).unwrap_or(0),
        "[opc_demand] 需求订阅扫描定时任务已就位"
    );
    Ok(cron_to_dto(&job))
}

/// 组装投递配置：没有任何渠道时返回空 channels（不推送）
fn build_delivery_config(
    webhook_url: Option<String>,
    file_path: Option<String>,
) -> CronDeliveryConfig {
    let mut channels = Vec::new();

    if let Some(url) = webhook_url {
        let url = url.trim().to_string();
        if !url.is_empty() {
            channels.push(CronDeliveryChannel::Webhook { url, headers: None, sign_secret: None });
        }
    }

    if let Some(path) = file_path {
        let path = path.trim().to_string();
        if !path.is_empty() {
            channels.push(CronDeliveryChannel::File { path, append: Some(true) });
        }
    }

    CronDeliveryConfig {
        channels,
        // 扫描「成功但没命中」也算成功，不推送（推送门控在 scheduler 分支按
        // high_value_hits > 0 判断，此处不能设 only_on_failure）
        only_on_failure: false,
        include_history: false,
        message_template: None,
    }
}

/// DAO 错误 → 命令层错误串（走错误码映射层）
fn err(e: axagent_harness::core_error::AxAgentError) -> String {
    String::from(crate::commands::error::ErrorResponse::from_error(
        e,
        crate::commands::error::ErrorCategory::Unrecoverable,
    ))
}
