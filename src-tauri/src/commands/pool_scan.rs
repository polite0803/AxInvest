// SPDX-License-Identifier: AGPL-3.0-only

//! 候选池逐只分析定时任务（task_type = `pool-scan`）。
//!
//! # 为什么需要它
//!
//! 用户要的链路是「定时建候选池 → 定时逐只分析候选池 → 定时按周期反思」。
//! 在这条链上，**「候选池 → 逐只分析」这一环此前完全缺失**（见
//! `AUDIT-scheduled-tasks-2026-09-13.md` 诉求②）：
//!
//! - 荐股 cron（`stock-recommendation`）只把结果推桌面通知，不落候选池；
//! - 已有的唯一「批量逐只分析」入口是 `watchlist-scan`，扫的是**自选股**，
//!   与 `reco_picks` 候选池是两回事；
//! - `stock_pipeline` 模板虽存在，但 trigger = Manual，永不自动调度。
//!
//! 结果就是：候选池里的股票**永远不会被自动分析**，也就永远不会进入反思队列。
//!
//! # 数据流
//!
//! 1. 读 `reco_picks` 近 `lookback_days` 天内、`synthetic = 0` 的候选
//!    （**含**智能荐股与趋势智选 `style='serenity'` 两种产物）
//! 2. 按 `stock_code` 去重（留 confidence 最高的一条，并带上它的 `period`）
//! 3. confidence 降序取前 `max_stocks` 只（成本闸门）
//! 4. 逐只 `run_single_stock_analysis`，把 pick 的周期映射成
//!    `expected_holding_days`（2/5/28/90）落进 `stock_analyses`
//!    ⇒ 同时写 `stock_reflections` pending row
//! 5. pending 由 `batch-reflection` 任务按 4 周期分档消费
//!
//! [services]: crate::init::services::start_cron_scheduler

use axagent_agent_macro::agent_command;
use axagent_analysis_engine::recommender::Period;
use axagent_runtime_core::{CronJob, CronJobStatus};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QueryOrder};
use serde::Serialize;
use tauri::State;

use crate::AppState;

/// `pool-scan` 任务类型标识（executor 路由键，勿改）
pub const POOL_SCAN_TASK_TYPE: &str = "pool-scan";

/// pool-scan 配置（JSON 写入 `CronJob.prompt`）。
///
/// 注意：**不能**写进 `CronJob.description` —— 那只是给人看的文本，
/// 执行体必须能从 `prompt` 反序列化出结构（同 `RecoCronConfig` 的约定）。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PoolScanConfig {
    /// 候选池回看窗口（天）：读最近 N 天内生成的 reco_picks
    pub lookback_days: u32,
    /// 单次最多分析只数（成本闸门，防止一个任务把配额打满）
    pub max_stocks: usize,
    /// 候选最低置信度（低于此值的 pick 不进分析队列）
    pub min_confidence: u8,
}

impl Default for PoolScanConfig {
    fn default() -> Self {
        Self { lookback_days: 3, max_stocks: 10, min_confidence: 60 }
    }
}

impl PoolScanConfig {
    pub fn from_json(s: &str) -> Result<Self, String> {
        serde_json::from_str(s).map_err(|e| format!("解析 pool-scan 配置失败: {e}"))
    }

    pub fn to_json(&self) -> Result<String, String> {
        serde_json::to_string(self).map_err(|e| format!("序列化 pool-scan 配置失败: {e}"))
    }
}

/// 候选池里待分析的标的（去重后的最小信息集）
#[derive(Debug, Clone)]
pub struct PoolCandidate {
    pub stock_code: String,
    pub stock_name: String,
    pub period: Period,
    pub confidence: i32,
}

/// 从 `reco_picks` 读取候选池（按 confidence 降序去重 + 截断到 max_stocks）。
pub async fn load_pool_candidates(
    db: &sea_orm::DatabaseConnection,
    config: &PoolScanConfig,
) -> Result<Vec<PoolCandidate>, String> {
    use axagent_entities::reco_picks;

    // 窗口起点必须与 `reco_picks.generated_at` 的写入时区一致 ——
    // 落库用的是 `chrono::Local::now()`（本机时区），这里若用 `Utc::now()`
    // 会平白多出 8 小时窗口（把更早的候选也算进来）。
    let since = (chrono::Local::now() - chrono::Duration::days(config.lookback_days as i64))
        .format("%Y-%m-%dT%H:%M:%S%.3f")
        .to_string();

    let rows = reco_picks::Entity::find()
        // 只取回看窗口内的候选，避免拿历史陈票重复分析
        .filter(reco_picks::Column::GeneratedAt.gte(&since))
        // synthetic = 1 是数据稀疏时的兜底合成 pick，没有技术信号支撑，分析它没有意义
        .filter(reco_picks::Column::Synthetic.eq(0))
        // [2026-09-13 修正] 这里**不排除** style='serenity'（趋势智选工作流的产物）。
        // 用户诉求明确是「荐股 **和** 趋势智选 创建候选池 → 逐只分析」，
        // 排除它等于把趋势智选的候选全部挡在门外。
        // 经 DB 实证：serenity 行 period='mid'、synthetic=0、pick_data 非空、
        // 置信度（均值 65 / 最高 85）反而高于兜底合成行 —— 完全可用。
        // （`get_cached_recommendation` 排除 serenity 是**缓存读取**语义 —— 避免它
        //   抢占 mid 的一批缓存导致前端 STYLE_KEYS 匹配不上；与本处无关。）
        .filter(reco_picks::Column::Confidence.gte(config.min_confidence as i32))
        // v007 之前的旧行没有 pick_data，无法还原 pick 详情
        .filter(reco_picks::Column::PickData.is_not_null())
        .order_by_desc(reco_picks::Column::Confidence)
        .all(db)
        .await
        .map_err(|e| format!("读取候选池失败: {e}"))?;

    let mut seen = std::collections::HashSet::new();
    let mut out: Vec<PoolCandidate> = Vec::new();
    for r in rows {
        // 同一只股票可能被多个 period / 多个 style 同时命中，
        // 只分析一次（保留 confidence 最高的那条，它带着该票的推荐周期）
        if !seen.insert(r.stock_code.clone()) {
            continue;
        }
        out.push(PoolCandidate {
            stock_code: r.stock_code,
            stock_name: r.stock_name,
            // period 解析失败（脏数据）时退到 Mid，不静默跳过该标的
            period: r.period.parse().unwrap_or(Period::Mid),
            confidence: r.confidence,
        });
        if out.len() >= config.max_stocks {
            break;
        }
    }
    Ok(out)
}

/// 一次 pool-scan 的执行结果
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PoolScanOutcome {
    /// 候选池中命中窗口的标的数（去重、截断后）
    pub candidates: usize,
    pub ok: usize,
    pub fail: usize,
    pub first_error: Option<String>,
    /// 每只一行的人类可读明细（写入 TaskRunResult.output）
    pub details: Vec<String>,
}

impl PoolScanOutcome {
    /// 渲染为任务输出文本
    pub fn render(&self) -> String {
        let mut s = format!(
            "候选池扫描完成: {} 只候选, 分析成功 {}, 失败 {}",
            self.candidates, self.ok, self.fail
        );
        if self.details.is_empty() {
            s.push_str("\n（候选池为空：请确认荐股/趋势智选定时任务已产生 reco_picks）");
        } else {
            s.push('\n');
            s.push_str(&self.details.join("\n"));
        }
        s
    }
}

/// 执行一次候选池扫描。
///
/// 定时任务（cron executor）与手动触发共用 —— 保证两条入口的候选筛选口径完全一致。
pub async fn run_pool_scan(
    db: &sea_orm::DatabaseConnection,
    client: &axagent_astock_data::AStockClient,
    engine: &std::sync::Arc<axagent_rt_workflow::work_engine::WorkEngine>,
    config: &PoolScanConfig,
) -> Result<PoolScanOutcome, String> {
    let candidates = load_pool_candidates(db, config).await?;
    let mut ok = 0usize;
    let mut fail = 0usize;
    let mut first_error: Option<String> = None;
    let mut details: Vec<String> = Vec::with_capacity(candidates.len());

    for c in &candidates {
        // 周期 → 持有天数：这是「4 周期语义」的写入口。
        // 落进 stock_analyses.decision_expected_holding_days 后，
        // batch-reflection 的按周期分档筛选才有依据。
        let holding_days = c.period.default_holding_days();
        match crate::commands::stock_workflow::run_single_stock_analysis(
            db,
            client,
            engine,
            &c.stock_code,
            &c.stock_name,
            Some(holding_days),
            None, // template_id — 候选池扫描恒走完整分析链
        )
        .await
        {
            Ok(analysis_id) => {
                ok += 1;
                details.push(format!(
                    "{} {} [{}·{}天·置信{}] ✓ {}",
                    c.stock_code,
                    c.stock_name,
                    c.period.as_str(),
                    holding_days,
                    c.confidence,
                    analysis_id
                ));
            },
            Err(e) => {
                fail += 1;
                if first_error.is_none() {
                    first_error = Some(e.clone());
                }
                details.push(format!(
                    "{} {} [置信{}] ✗ {e}",
                    c.stock_code, c.stock_name, c.confidence
                ));
            },
        }
    }

    Ok(PoolScanOutcome { candidates: candidates.len(), ok, fail, first_error, details })
}

// ── Tauri 命令：候选池扫描 cron CRUD ──

/// 创建候选池扫描定时任务
///
/// - 配置以 JSON 写入 `CronJob.prompt`，由 executor 在 `pool-scan` 分支解析
/// - task_type = `pool-scan`，不绑定 workflow
#[agent_command(domain = "finance", safety = Caution, call_mode = StateInput, description = "创建候选池分析定时任务")]
#[tauri::command]
pub async fn create_pool_scan_cron(
    state: State<'_, AppState>,
    cron_expression: Option<String>,
    lookback_days: Option<u32>,
    max_stocks: Option<usize>,
    min_confidence: Option<u8>,
    enabled: Option<bool>,
) -> Result<crate::commands::stock_analysis::CronJobResponse, String> {
    let defaults = PoolScanConfig::default();
    let config = PoolScanConfig {
        lookback_days: lookback_days.unwrap_or(defaults.lookback_days),
        max_stocks: max_stocks.unwrap_or(defaults.max_stocks),
        min_confidence: min_confidence.unwrap_or(defaults.min_confidence),
    };
    if config.max_stocks == 0 {
        return Err("max_stocks 必须大于 0".to_string());
    }
    if config.min_confidence > 100 {
        return Err("min_confidence 必须在 0-100 之间".to_string());
    }
    // 回看窗口至少 1 天：0 会把窗口压成"今天 00:00 之后"，几乎必然筛不出候选
    if config.lookback_days == 0 {
        return Err("lookback_days 必须大于 0".to_string());
    }

    let expr = cron_expression.unwrap_or_else(|| "0 17 * * *".to_string());
    let prompt = config.to_json()?;
    let desc = format!(
        "扫描近 {} 天候选池（置信度≥{}%，最多 {} 只）逐只执行完整分析",
        config.lookback_days, config.min_confidence, config.max_stocks
    );
    let id =
        format!("poolscan-{}", uuid::Uuid::new_v4().to_string().split('-').next().unwrap_or("x"));
    let mut job = CronJob::new(&id, &expr, &prompt, &desc).with_task_type(POOL_SCAN_TASK_TYPE);
    if !enabled.unwrap_or(true) {
        job.status = CronJobStatus::Paused;
    }
    state.cron_job_store.add(job.clone()).await;
    Ok(crate::commands::stock_analysis::CronJobResponse::from(&job))
}

/// 列出所有候选池扫描定时任务
#[agent_command(domain = "finance", safety = Safe, call_mode = StateInput, description = "列出候选池分析定时任务")]
#[tauri::command]
pub async fn list_pool_scan_crons(
    state: State<'_, AppState>,
) -> Result<Vec<crate::commands::stock_analysis::CronJobResponse>, String> {
    let jobs = state.cron_job_store.list().await;
    Ok(jobs
        .iter()
        .filter(|j| j.task_type.as_deref() == Some(POOL_SCAN_TASK_TYPE))
        .map(crate::commands::stock_analysis::CronJobResponse::from)
        .collect())
}

/// 启停候选池扫描定时任务
#[agent_command(domain = "finance", safety = Caution, call_mode = StateOnly, description = "开关候选池分析定时任务")]
#[tauri::command]
pub async fn toggle_pool_scan_cron(
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

/// 删除候选池扫描定时任务
#[agent_command(domain = "finance", safety = Dangerous, call_mode = StateInput, description = "删除候选池分析定时任务")]
#[tauri::command]
pub async fn delete_pool_scan_cron(state: State<'_, AppState>, id: String) -> Result<(), String> {
    state.cron_job_store.remove(&id).await;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pool_scan_config_roundtrip() {
        let c = PoolScanConfig { lookback_days: 5, max_stocks: 8, min_confidence: 70 };
        let json = c.to_json().unwrap();
        // 必须是 camelCase，与前端 TS 类型一致（契约见 AGENTS.md 第 13 条）
        assert!(json.contains("lookbackDays"), "json = {json}");
        assert!(json.contains("\"maxStocks\""), "json = {json}");
        assert!(json.contains("\"minConfidence\""), "json = {json}");
        let back = PoolScanConfig::from_json(&json).unwrap();
        assert_eq!(back.lookback_days, 5);
        assert_eq!(back.max_stocks, 8);
        assert_eq!(back.min_confidence, 70);
    }

    #[test]
    fn pool_scan_config_invalid_json() {
        assert!(PoolScanConfig::from_json("not json").is_err());
    }
}
