//! 荐股定时任务：创建/列表/启停/删除
//!
//! 与 [stock_cron] 不同的是：
//! - task_type = `stock-recommendation`（用于在 [services] 的 cron executor 中路由到荐股 handler）
//! - 配置（periods / min_confidence / top_n）以 JSON 形式写入 `CronJob.prompt`
//! - 不绑定 workflow（不走 work_engine）
//!
//! [stock_cron]: crate::commands::stock_analysis::create_stock_cron
//! [services]: crate::init::services::start_cron_scheduler

use crate::AppState;
use axagent_runtime_core::{CronJob, CronJobStatus};
use axagent_stock_analysis::recommender::{Period, RecoPick};
use serde::Serialize;
use tauri::State;

/// 前端传入的"周期"枚举字符串 → Period 映射
fn parse_periods(raw: Vec<String>) -> Result<Vec<Period>, String> {
    raw.into_iter()
        .map(|s| match s.as_str() {
            "short" => Ok(Period::Short),
            "mid" => Ok(Period::Mid),
            "long" => Ok(Period::Long),
            other => Err(format!("未知的 period: {other}")),
        })
        .collect()
}

/// 推荐 cron 配置（写入 `CronJob.prompt`）
#[derive(Debug, Serialize, serde::Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct RecoCronConfig {
    pub periods: Vec<Period>,
    pub min_confidence: u8,
    pub top_n: usize,
}

impl RecoCronConfig {
    pub fn to_json(&self) -> Result<String, String> {
        serde_json::to_string(self).map_err(|e| e.to_string())
    }
    pub fn from_json(s: &str) -> Result<Self, String> {
        serde_json::from_str(s).map_err(|e| e.to_string())
    }
}

/// 列出所有 stock-recommendation 类型的 cron 任务（带解析后的 config）
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecommendationCronRow {
    pub id: String,
    pub name: String,
    pub description: String,
    pub schedule: String,
    pub status: String,
    pub run_count: u32,
    pub last_run_at: Option<i64>,
    pub next_run_at: Option<i64>,
    pub config: RecoCronConfig,
    /// 最近一次 run 推送的真实 picks 数量（来自 last_result.output）
    pub last_picks_count: Option<usize>,
}

/// 创建荐股定时任务
#[tauri::command]
pub async fn create_recommendation_cron(
    state: State<'_, AppState>,
    name: String,
    cron_expression: String,
    periods: Vec<String>,
    min_confidence: u8,
    top_n: usize,
) -> Result<RecommendationCronRow, String> {
    let parsed_periods = parse_periods(periods)?;
    let cfg = RecoCronConfig {
        periods: parsed_periods,
        min_confidence: min_confidence.min(100),
        top_n: top_n.clamp(1, 50),
    };

    let prompt = cfg.to_json()?;
    let desc = format!(
        "智能荐股 · 周期 {}+ · 最低置信度 {} · 推送 Top {}",
        cfg.periods
            .iter()
            .map(|p| p.as_str())
            .collect::<Vec<_>>()
            .join("/"),
        cfg.min_confidence,
        cfg.top_n
    );

    let job = CronJob::new(&name, &cron_expression, &prompt, &desc)
        .with_task_type("stock-recommendation");
    state.cron_job_store.add(job.clone()).await;

    Ok(row_from_job(&job))
}

/// 列出所有荐股定时任务
#[tauri::command]
pub async fn list_recommendation_crons(
    state: State<'_, AppState>,
) -> Result<Vec<RecommendationCronRow>, String> {
    let jobs = state.cron_job_store.list().await;
    let mut out = Vec::new();
    for j in jobs
        .iter()
        .filter(|j| j.task_type.as_deref() == Some("stock-recommendation"))
    {
        out.push(row_from_job(j));
    }
    Ok(out)
}

/// 启停荐股定时任务
#[tauri::command]
pub async fn toggle_recommendation_cron(
    state: State<'_, AppState>,
    id: String,
    enabled: bool,
) -> Result<(), String> {
    let status = if enabled {
        CronJobStatus::Active
    } else {
        CronJobStatus::Paused
    };
    state.cron_job_store.set_status(&id, status).await;
    Ok(())
}

/// 删除荐股定时任务
#[tauri::command]
pub async fn delete_recommendation_cron(
    state: State<'_, AppState>,
    id: String,
) -> Result<(), String> {
    state.cron_job_store.remove(&id).await;
    Ok(())
}

fn row_from_job(j: &CronJob) -> RecommendationCronRow {
    let config = RecoCronConfig::from_json(&j.prompt).unwrap_or(RecoCronConfig {
        periods: vec![Period::Short],
        min_confidence: 60,
        top_n: 5,
    });
    let last_picks_count = j.last_result.as_ref().and_then(|r| {
        r.output
            .as_deref()
            .and_then(|s| s.strip_prefix("picks="))
            .and_then(|s| s.parse::<usize>().ok())
    });
    RecommendationCronRow {
        id: j.id.clone(),
        name: j.name.clone(),
        description: j.description.clone(),
        schedule: j.schedule.clone(),
        status: format!("{:?}", j.status).to_lowercase(),
        run_count: j.run_count,
        last_run_at: j.last_run_at,
        next_run_at: j.next_run_at,
        config,
        last_picks_count,
    }
}

/// 提取 cron job 中的 config（供 [services] cron executor 调用）
pub fn extract_reco_config(j: &CronJob) -> Option<RecoCronConfig> {
    if j.task_type.as_deref() != Some("stock-recommendation") {
        return None;
    }
    RecoCronConfig::from_json(&j.prompt).ok()
}

/// 构造推送 payload：把 picks 数量写入 output 字段（前端展示用）
pub fn picks_to_run_output(picks: &[RecoPick]) -> String {
    format!("picks={}", picks.len())
}
