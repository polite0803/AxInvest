// SPDX-License-Identifier: AGPL-3.0-only

//! 荐股定时任务：配置解析与辅助函数
//!
//! 与 [stock_cron] 不同的是：
//! - task_type = `stock-recommendation`（用于在 [services] 的 cron executor 中路由到荐股 handler）
//! - 配置（periods / min_confidence / top_n）以 JSON 形式写入 `CronJob.prompt`
//! - 不绑定 workflow（不走 work_engine）
//!
//! [2026-09-12 接线恢复] 本文件旧注释称「`axagent_analysis_engine` crate 已删除、
//! `run_recommendation_cron` 为存根」——**与事实不符**：该 crate 一直存在并参与编译，
//! 且 `recommender::notify` 里的扫描/通知逻辑完好（含单测）。真相是
//! `init/services.rs` 的 CronExecutor 缺 `stock-recommendation` 分支，
//! 任务到点不执行任何东西（同构缺陷已于 2026-09-03 在 `watchlist-scan` 侧修过一次）。
//! 现已在该处恢复接线：解析本文件的 `RecoCronConfig` → 调 `run_recommendation_scan`
//! → 过滤 synthetic/低置信 → 取 top N → 推桌面通知。
//!
//! [stock_cron]: crate::commands::stock_analysis::create_stock_cron
//! [services]: crate::init::services::start_cron_scheduler

/// K 线周期 —— 直接复用引擎的权威定义，禁止本地再定义一份。
///
/// 权威源：`axagent_analysis_engine::recommender::Period`（4 变体，含 `UltraShort`）。
/// 序列化为 `ultra_short` / `short` / `mid` / `long`，与前端 `PeriodKey` 一致。
pub use axagent_analysis_engine::recommender::Period;

/// 推荐 cron 配置（写入 `CronJob.prompt`）
#[derive(Debug, serde::Serialize, serde::Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct RecoCronConfig {
    pub periods: Vec<Period>,
    pub min_confidence: u8,
    pub top_n: usize,
}

impl RecoCronConfig {
    pub fn from_json(s: &str) -> Result<Self, String> {
        serde_json::from_str(s).map_err(|e| format!("解析荐股 cron 配置失败: {e}"))
    }
}

// ── Tauri 命令：荐股定时任务 CRUD ──

use axagent_agent_macro::agent_command;
use axagent_runtime_core::{CronJob, CronJobStatus};
use serde::Serialize;
use tauri::State;

use crate::AppState;

/// 与前端 `RecoCronRow` 对齐的响应结构
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecoCronJobResponse {
    id: String,
    name: String,
    description: String,
    schedule: String,
    status: String,
    recurring: bool,
    run_count: u32,
    last_run_at: Option<i64>,
    next_run_at: Option<i64>,
    /// 解析后的配置（periods / min_confidence / top_n）
    config: RecoCronConfig,
    /// 上次推送的 picks 数量（从 last_result.output 反序列化）
    last_picks_count: Option<usize>,
}

impl RecoCronJobResponse {
    /// 从 CronJob 构造响应；prompt 解析失败时 config 用默认值
    fn from_job(j: &CronJob) -> Self {
        let config = RecoCronConfig::from_json(&j.prompt).unwrap_or(RecoCronConfig {
            periods: vec![Period::Short],
            // 与模板变量 `reco_min_confidence` 的出厂默认值保持一致。
            // 两处若不同步，用户在定时任务面板用的阈值会比「智能荐股」页面严，
            // 表现为同一批推荐在定时推送里被静默过滤掉（超短线尤其明显：
            // 其置信度经 ×0.85 反身性折扣后落在 50~64 区间）。
            min_confidence: 50,
            top_n: 5,
        });
        // last_result.output 是执行结果的 JSON
        let last_picks_count = j
            .last_result
            .as_ref()
            .and_then(|r| r.output.as_deref())
            .and_then(|s| serde_json::from_str::<serde_json::Value>(s).ok())
            .and_then(|v| v.get("pushed").and_then(|n| n.as_u64()).map(|n| n as usize));
        Self {
            id: j.id.clone(),
            name: j.name.clone(),
            description: j.description.clone(),
            schedule: j.schedule.clone(),
            status: format!("{:?}", j.status).to_lowercase(),
            recurring: j.recurring,
            run_count: j.run_count,
            last_run_at: j.last_run_at,
            next_run_at: j.next_run_at,
            config,
            last_picks_count,
        }
    }
}

/// 创建荐股定时任务
///
/// - 配置以 JSON 写入 `CronJob.prompt`，由 `run_recommendation_cron` 在执行时解析
/// - task_type = "stock-recommendation"，不绑定 workflow
#[agent_command(domain = "general", safety = Caution, call_mode = StateOnly, description =  "创建荐股定时任务")]
#[tauri::command]
pub async fn create_recommendation_cron(
    state: State<'_, AppState>,
    name: String,
    cron_expression: String,
    periods: Vec<Period>,
    min_confidence: u8,
    top_n: usize,
) -> Result<RecoCronJobResponse, String> {
    if periods.is_empty() {
        return Err("periods 不能为空".to_string());
    }
    if top_n == 0 {
        return Err("top_n 必须大于 0".to_string());
    }
    // 前端 Slider 是 0-100，但 IPC 层不校验数字范围（u8 会把 300 静默截成 44），
    // 会造出「用户以为阈值 300、实际按 44 过滤」的静默错配。
    if min_confidence > 100 {
        return Err("min_confidence 必须在 0-100 之间".to_string());
    }
    let config = RecoCronConfig { periods, min_confidence, top_n };
    let prompt =
        serde_json::to_string(&config).map_err(|e| format!("序列化荐股 cron 配置失败: {e}"))?;
    let desc = format!("荐股定时推送 (置信度≥{}%, 前{}只)", min_confidence, top_n);
    let job = CronJob::new(&name, &cron_expression, &prompt, &desc)
        .with_task_type("stock-recommendation");
    let id = state.cron_job_store.add(job).await;
    // 重新读回以拿到完整字段（next_run_at 等）
    let saved =
        state.cron_job_store.get(&id).await.ok_or_else(|| "保存后未找到任务".to_string())?;
    Ok(RecoCronJobResponse::from_job(&saved))
}

/// 列出所有荐股定时任务
#[agent_command(domain = "general", safety = Safe, call_mode = StateOnly, description =  "列出荐股定时任务")]
#[tauri::command]
pub async fn list_recommendation_crons(
    state: State<'_, AppState>,
) -> Result<Vec<RecoCronJobResponse>, String> {
    let jobs = state.cron_job_store.list().await;
    Ok(jobs
        .iter()
        .filter(|j| j.task_type.as_deref() == Some("stock-recommendation"))
        .map(RecoCronJobResponse::from_job)
        .collect())
}

/// 启停荐股定时任务
#[agent_command(domain = "general", safety = Caution, call_mode = StateOnly, description =  "启停荐股定时任务")]
#[tauri::command]
pub async fn toggle_recommendation_cron(
    state: State<'_, AppState>,
    id: String,
    enabled: bool,
) -> Result<(), String> {
    state
        .cron_job_store
        .set_status(
            &id,
            if enabled {
                CronJobStatus::Active
            } else {
                CronJobStatus::Paused
            },
        )
        .await;
    Ok(())
}

/// 删除荐股定时任务
#[agent_command(domain = "general", safety = Caution, call_mode = StateOnly, description =  "删除荐股定时任务")]
#[tauri::command]
pub async fn delete_recommendation_cron(
    state: State<'_, AppState>,
    id: String,
) -> Result<(), String> {
    state.cron_job_store.remove(&id).await;
    Ok(())
}

// ── Tauri 命令：趋势智选定时任务 CRUD ──
//
// [2026-09-13 新增] 用户诉求「定时启动智能荐股**和趋势智选**创建候选股票池」。
// 「智能荐股」已有 `stock-recommendation` 分支；「趋势智选」= `serenity-screening`
// 工作流，此前**只能手动点按钮跑**（`run_serenity_screening` 命令），
// 没有任何定时入口 —— 候选池里的趋势智选候选因此只能靠人手刷新才会更新。
//
// 实现：建一个带 `workflow_id = "serenity-screening"` 的 CronJob，
// executor 末尾的 `workflow_id` 兜底分支会按 cron 调度它；产物
// （`reco_picks` 里 `style='serenity'` 的行）直接进候选池，再由 `pool-scan` 逐只分析。
//
// `task_type = "trend-screening"` 仅作前端列表过滤标记 —— executor 没有它的
// 专用分支，因此会正常落到 `workflow_id` 分支（这正是我们要的行为）。

/// 趋势智选工作流模板 ID（与 `seed_serenity.rs` 的 TEMPLATE_ID 保持一致）
pub const TREND_SCREENING_WORKFLOW_ID: &str = "serenity-screening";
/// 前端列表过滤用的任务类型标记
pub const TREND_SCREENING_TASK_TYPE: &str = "trend-screening";

/// 创建趋势智选定时任务
#[agent_command(domain = "finance", safety = Caution, call_mode = StateInput, description = "创建趋势智选定时任务")]
#[tauri::command]
pub async fn create_trend_screening_cron(
    state: State<'_, AppState>,
    cron_expression: Option<String>,
    enabled: Option<bool>,
) -> Result<crate::commands::stock_analysis::CronJobResponse, String> {
    let expr = cron_expression.unwrap_or_else(|| "0 16 * * 1-5".to_string());
    let id = format!("trend-{}", uuid::Uuid::new_v4().to_string().split('-').next().unwrap_or("x"));
    let mut job = CronJob::new(
        &id,
        &expr,
        "定时运行趋势智选工作流，产物写入候选池（reco_picks）",
        "趋势智选定时筛选（产物进候选池，供候选池分析任务消费）",
    )
    .with_task_type(TREND_SCREENING_TASK_TYPE)
    .with_workflow_id(TREND_SCREENING_WORKFLOW_ID.to_string());
    if !enabled.unwrap_or(true) {
        job.status = CronJobStatus::Paused;
    }
    state.cron_job_store.add(job.clone()).await;
    Ok(crate::commands::stock_analysis::CronJobResponse::from(&job))
}

/// 列出所有趋势智选定时任务
#[agent_command(domain = "finance", safety = Safe, call_mode = StateInput, description = "列出趋势智选定时任务")]
#[tauri::command]
pub async fn list_trend_screening_crons(
    state: State<'_, AppState>,
) -> Result<Vec<crate::commands::stock_analysis::CronJobResponse>, String> {
    let jobs = state.cron_job_store.list().await;
    Ok(jobs
        .iter()
        .filter(|j| j.task_type.as_deref() == Some(TREND_SCREENING_TASK_TYPE))
        .map(crate::commands::stock_analysis::CronJobResponse::from)
        .collect())
}

/// 启停趋势智选定时任务
#[agent_command(domain = "finance", safety = Caution, call_mode = StateOnly, description = "开关趋势智选定时任务")]
#[tauri::command]
pub async fn toggle_trend_screening_cron(
    state: State<'_, AppState>,
    id: String,
    enabled: bool,
) -> Result<(), String> {
    state
        .cron_job_store
        .set_status(
            &id,
            if enabled {
                CronJobStatus::Active
            } else {
                CronJobStatus::Paused
            },
        )
        .await;
    Ok(())
}

/// 删除趋势智选定时任务
#[agent_command(domain = "finance", safety = Dangerous, call_mode = StateInput, description = "删除趋势智选定时任务")]
#[tauri::command]
pub async fn delete_trend_screening_cron(
    state: State<'_, AppState>,
    id: String,
) -> Result<(), String> {
    state.cron_job_store.remove(&id).await;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_reco_cron_config_from_json() {
        let json = r#"{"periods":["short","mid"],"minConfidence":70,"topN":5}"#;
        let config = RecoCronConfig::from_json(json).unwrap();
        assert_eq!(config.periods.len(), 2);
        assert_eq!(config.min_confidence, 70);
        assert_eq!(config.top_n, 5);
    }

    #[test]
    fn test_reco_cron_config_invalid_json() {
        let result = RecoCronConfig::from_json("invalid");
        assert!(result.is_err());
    }
}
