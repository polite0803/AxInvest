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
//! 1. 读 `reco_picks` 中 `synthetic = 0` 的候选（**含**智能荐股与趋势智选
//!    `style='serenity'` 两种产物）；`lookback_days = 0` ⇒ **全池**，不限进池日期
//!    （2026-09-30 自动化闭环改造：没排到/失败的候选次日晚自然重新进队，
//!    补跑由「尚未分析」判据承担，不靠窗口顺延）
//! 2. 排除**当日已分析**的 code（`stock_analyses.created_at >= 本地今日 00:00`
//!    且 `analysis_kind = 'live'`；历史某天分析过但当日没分析的**照常分析**）
//! 3. 按 `stock_code` 去重（留 confidence 最高的一条，并带上它的 `period`）
//! 4. confidence 降序；`max_stocks > 0` 才截断（0 = 不限，依次全量）
//! 5. 逐只 `run_single_stock_analysis`，把 pick 的周期映射成
//!    `expected_holding_days`（2/5/28/90）落进 `stock_analyses`
//!    ⇒ 同时写 `stock_reflections` pending row
//! 6. pending 由 `batch-reflection` 任务按 4 周期分档消费
//!
//! [services]: crate::init::services::start_cron_scheduler

use axagent_agent_macro::agent_command;
use axagent_analysis_engine::recommender::Period;
use axagent_runtime_core::{CronJob, CronJobStatus};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QuerySelect};
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
    /// 候选池回看窗口（天）：**0 = 全池**（自动化闭环默认——未分析的积压跨日必补）
    pub lookback_days: u32,
    /// 单次最多分析只数：**0 = 不限**（依次全量；旧「必须大于 0」的成本闸门语义已改）
    pub max_stocks: usize,
    /// 候选最低置信度（低于此值的 pick 不进分析队列）
    pub min_confidence: u8,
}

impl Default for PoolScanConfig {
    /// 自动化闭环口径：全池、不截断、置信度 50（与荐股档出厂先验一致）。
    ///
    /// ⚠ 旧默认 `{3, 10, 60}` 的两处语义已废：`max_stocks=10` 会把排在 10 名之后、
    /// 尚未分析的候选静默挡掉；`min_confidence=60` 会挡掉大量超短档 pick
    /// （其置信度经反身性折扣后落在 50~64 区间）。
    fn default() -> Self {
        Self { lookback_days: 0, max_stocks: 0, min_confidence: 50 }
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

/// 一次 pool-scan 的候选筛选结果（三种缺席各报各的，不混成一个读数）
#[derive(Debug, Clone)]
pub struct PoolSelection {
    /// 待分析队列（去重后，confidence 降序，已按 max_stocks 截断）
    pub candidates: Vec<PoolCandidate>,
    /// 当日已分析过、本轮跳过的 code
    pub skipped_analyzed_today: Vec<String>,
    /// 队列长度被 max_stocks 截断（0 = 不限时恒 false）
    pub truncated: bool,
}

/// 纯函数筛选核：去重 + 「当日已分析」排除 + 截断（DB 查询结果与 code 集合进来，
/// 便于无库单测锁语义）。
///
/// 入参 `rows` 允许任意顺序；同一 code 多条 pick（四档各一排、荐股与趋势智选撞车）
/// 只保留 confidence 最高的一条并带上它的 period。
/// 「当日已分析」在**截断之前**排除 ⇒ 配额只消耗在未分析标的上。
fn select_candidates(
    rows: Vec<(String, String, Period, i32)>,
    analyzed_today: &std::collections::HashSet<String>,
    max_stocks: usize,
) -> PoolSelection {
    let mut best: std::collections::HashMap<String, (String, Period, i32)> =
        std::collections::HashMap::new();
    for (code, name, period, confidence) in rows {
        match best.get(&code) {
            Some((_, _, c)) if *c >= confidence => {},
            _ => {
                best.insert(code, (name, period, confidence));
            },
        }
    }

    let mut skipped: Vec<String> = Vec::new();
    let mut queue: Vec<PoolCandidate> = Vec::new();
    for (code, (name, period, confidence)) in best {
        if analyzed_today.contains(&code) {
            skipped.push(code);
            continue;
        }
        queue.push(PoolCandidate { stock_code: code, stock_name: name, period, confidence });
    }
    // confidence 降序稳定排队（同置信按 code，消除 HashMap 乱序带来的批次抖动）
    queue.sort_by(|a, b| {
        b.confidence.cmp(&a.confidence).then_with(|| a.stock_code.cmp(&b.stock_code))
    });

    let truncated = max_stocks > 0 && queue.len() > max_stocks;
    if max_stocks > 0 {
        queue.truncate(max_stocks);
    }
    PoolSelection { candidates: queue, skipped_analyzed_today: skipped, truncated }
}

/// 从 `reco_picks` 读取候选池并应用「当日未分析」判据（cron 与手动入口共用）。
pub async fn load_pool_selection(
    db: &sea_orm::DatabaseConnection,
    config: &PoolScanConfig,
) -> Result<PoolSelection, String> {
    use axagent_entities::{reco_picks, stock_analyses};

    let mut query = reco_picks::Entity::find()
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
        .filter(reco_picks::Column::PickData.is_not_null());

    // 窗口：0 = 全池（积压跨日必补，未分析的次日晚自然重新进队）；
    // >0 保留旧回看语义（存量手动任务不受影响）。serenity 行不排除，理由见下方
    // 原注释（2026-09-13 修正）：用户诉求是「荐股 **和** 趋势智选」都进逐只分析。
    if config.lookback_days > 0 {
        // 窗口起点必须与 `reco_picks.generated_at` 的写入时区一致 ——
        // 落库用的是 `chrono::Local::now()`（本机时区），这里若用 `Utc::now()`
        // 会平白多出 8 小时窗口（把更早的候选也算进来）。
        let since = (chrono::Local::now() - chrono::Duration::days(config.lookback_days as i64))
            .format("%Y-%m-%dT%H:%M:%S%.3f")
            .to_string();
        query = query.filter(reco_picks::Column::GeneratedAt.gte(&since));
    }

    // 全池扫描会把历史所有行载入内存，而 `pick_data` / `seed_pool_json` 是每行最大的
    // 两列且本函数不需要 ⇒ 显式只取四列（谓词仍用 pick_data IS NOT NULL 筛旧版本行）。
    let rows = query
        .select_only()
        .column(reco_picks::Column::StockCode)
        .column(reco_picks::Column::StockName)
        .column(reco_picks::Column::Period)
        .column(reco_picks::Column::Confidence)
        .into_tuple::<(String, String, String, i32)>()
        .all(db)
        .await
        .map_err(|e| format!("读取候选池失败: {e}"))?;

    // 「当日已分析」判据走 `stock_analyses.created_at`（epoch 毫秒，建行即写）。
    // ⚠ 不能用 `analysis_date` 判「今天」——批量路径（`run_single_stock_analysis`，
    // core.rs 建行处）写的是 **UTC 日**，本地 02:00 时 UTC 还是昨天，与本地日边界错配。
    let day_start_ms = {
        chrono::Local::now()
            .date_naive()
            .and_hms_opt(0, 0, 0)
            .expect("00:00 恒存在")
            .and_local_timezone(chrono::Local)
            .single()
            .map(|dt| dt.timestamp_millis())
            .unwrap_or_else(|| {
                // 时区歧义（DST 类；本机东八区不会发生）⇒ 宁可用 UTC 日界兜底
                tracing::warn!("[pool-scan] 本地日界解析失败，回退 UTC 日界");
                chrono::Utc::now()
                    .date_naive()
                    .and_hms_milli_opt(0, 0, 0, 0)
                    .expect("00:00:00.000 恒存在")
                    .and_utc()
                    .timestamp_millis()
            })
    };
    let analyzed_rows = stock_analyses::Entity::find()
        .filter(stock_analyses::Column::CreatedAt.gte(day_start_ms))
        // 历史回放（replay / ab_test）的分析不算「当日已分析」
        .filter(stock_analyses::Column::AnalysisKind.eq("live"))
        .select_only()
        .column(stock_analyses::Column::StockCode)
        .group_by(stock_analyses::Column::StockCode)
        .into_tuple::<String>()
        .all(db)
        .await
        .map_err(|e| format!("读取当日已分析集合失败: {e}"))?;
    let analyzed_today: std::collections::HashSet<String> = analyzed_rows.into_iter().collect();

    let triples = rows
        .into_iter()
        // period 解析失败（脏数据）时退到 Mid，不静默跳过该标的
        .map(|(code, name, period, confidence)| {
            (
                code,
                name,
                period.parse().unwrap_or(Period::Mid),
                confidence,
            )
        })
        .collect();
    Ok(select_candidates(triples, &analyzed_today, config.max_stocks))
}

/// 一次 pool-scan 的执行结果
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PoolScanOutcome {
    /// 本轮实际进入分析队列的标的数（去重、排除当日已分析、截断后）
    pub candidates: usize,
    pub ok: usize,
    pub fail: usize,
    /// 当日已分析过、本轮跳过的标的数
    pub skipped_analyzed_today: usize,
    /// 队列被 max_stocks 截断（maxStocks=0 不限时恒 false）
    pub truncated: bool,
    pub first_error: Option<String>,
    /// 每只一行的人类可读明细（写入 TaskRunResult.output）
    pub details: Vec<String>,
}

impl PoolScanOutcome {
    /// 渲染为任务输出文本。
    ///
    /// 三种缺席各报各的：空池 / 全部当日已分析跳过 / 被截断——互不冒充
    /// （否则「0 只分析」会被读成「池是空的」，见 feedback-structural-gap-no-ui-ambiguity）。
    pub fn render(&self) -> String {
        let mut s = format!(
            "候选池扫描完成: {} 只入队, 分析成功 {}, 失败 {}, 当日已分析跳过 {}",
            self.candidates, self.ok, self.fail, self.skipped_analyzed_today
        );
        if self.candidates == 0 && self.skipped_analyzed_today == 0 {
            s.push_str("\n（候选池为空：请确认荐股/趋势智选定时任务已产生 reco_picks）");
        } else if self.candidates == 0 {
            s.push_str("\n（全部候选当日已分析过，本轮无待分析标的）");
        }
        if self.truncated {
            s.push_str("\n（队列被 maxStocks 截断，未分析积压将于后续轮次继续）");
        }
        if !self.details.is_empty() {
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
    let selection = load_pool_selection(db, config).await?;
    let candidates = selection.candidates;
    let skipped_codes = selection.skipped_analyzed_today;
    let truncated = selection.truncated;
    let mut ok = 0usize;
    let mut fail = 0usize;
    let mut first_error: Option<String> = None;
    let mut details: Vec<String> = Vec::with_capacity(candidates.len());

    for c in &skipped_codes {
        details.push(format!("{c} （当日已分析，跳过）"));
    }

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

    Ok(PoolScanOutcome {
        candidates: candidates.len(),
        ok,
        fail,
        skipped_analyzed_today: skipped_codes.len(),
        truncated,
        first_error,
        details,
    })
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
    if config.min_confidence > 100 {
        return Err("min_confidence 必须在 0-100 之间".to_string());
    }

    let expr = cron_expression.unwrap_or_else(|| "0 17 * * *".to_string());
    let prompt = config.to_json()?;
    let window_desc = if config.lookback_days == 0 {
        "全池（不限进池日期）".to_string()
    } else {
        format!("近 {} 天", config.lookback_days)
    };
    let limit_desc = if config.max_stocks == 0 {
        "不限只数".to_string()
    } else {
        format!("最多 {} 只", config.max_stocks)
    };
    let desc = format!(
        "扫描{}候选池（置信度≥{}%，{}），跳过当日已分析的，逐只执行完整分析",
        window_desc, config.min_confidence, limit_desc
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

    fn cand(code: &str, period: Period, confidence: i32) -> (String, String, Period, i32) {
        (code.to_string(), format!("{code}-name"), period, confidence)
    }

    fn codes(sel: &PoolSelection) -> Vec<String> {
        sel.candidates.iter().map(|c| c.stock_code.clone()).collect()
    }

    /// 同一 code 多条 pick（四档 + serenity 撞车）→ 只留一条，取最高置信及其 period
    #[test]
    fn select_dedups_pool_by_confidence_keeping_best_period() {
        let rows = vec![
            cand("600000", Period::Short, 55),
            cand("600000", Period::Mid, 72),
            cand("600000", Period::UltraShort, 61),
        ];
        let sel = select_candidates(rows, &std::collections::HashSet::new(), 0);
        assert_eq!(codes(&sel), vec!["600000"]);
        assert_eq!(sel.candidates[0].confidence, 72);
        assert_eq!(sel.candidates[0].period, Period::Mid);
    }

    /// 诉求核心：当日已分析过的跳过；只进过历史池、当日没分析的**照常分析**
    #[test]
    fn select_skips_analyzed_today_but_keeps_backlog() {
        let analyzed: std::collections::HashSet<String> =
            ["600001".to_string()].into_iter().collect();
        let rows = vec![cand("600001", Period::Mid, 80), cand("600002", Period::Short, 70)];
        let sel = select_candidates(rows, &analyzed, 0);
        assert_eq!(codes(&sel), vec!["600002"]);
        assert_eq!(sel.skipped_analyzed_today, vec!["600001".to_string()]);
        assert!(!sel.truncated);
    }

    /// maxStocks=0 ⇒ 不限；>0 ⇒ 截断且 truncated 显式声明；
    /// 排除在截断**之前** ⇒ 配额不被当日已分析的标的消耗
    #[test]
    fn select_truncation_zero_means_unlimited_and_skip_precedes_limit() {
        let rows: Vec<_> =
            (1..=12).map(|i| cand(&format!("6000{i:02}"), Period::Mid, 90 - i)).collect();
        let unlimited = select_candidates(rows.clone(), &std::collections::HashSet::new(), 0);
        assert_eq!(unlimited.candidates.len(), 12);
        assert!(!unlimited.truncated);

        let limited = select_candidates(rows.clone(), &std::collections::HashSet::new(), 5);
        assert_eq!(limited.candidates.len(), 5);
        assert!(limited.truncated);

        // 前 4 名当日已分析 → 它们进 skip，不占 5 个配额；队列仍是 5 只（未分析的最高置信段）
        let analyzed: std::collections::HashSet<String> =
            (1..=4).map(|i| format!("6000{i:02}")).collect();
        let mixed = select_candidates(rows, &analyzed, 5);
        assert_eq!(mixed.candidates.len(), 5);
        assert_eq!(mixed.skipped_analyzed_today.len(), 4);
        assert_eq!(mixed.candidates[0].stock_code, "600005");
    }

    /// confidence 降序稳定：同置信按 code 升序，消除 HashMap 乱序的批次抖动
    #[test]
    fn select_orders_confidence_desc_then_code_asc() {
        let rows = vec![
            cand("600003", Period::Mid, 70),
            cand("600002", Period::Mid, 70),
            cand("600001", Period::Mid, 80),
        ];
        let sel = select_candidates(rows, &std::collections::HashSet::new(), 0);
        assert_eq!(codes(&sel), vec!["600001", "600002", "600003"]);
    }

    /// 缺席语义互不冒充：空池 ≠ 全跳过（render 两种说法必须不同）
    #[test]
    fn outcome_render_distinguishes_empty_pool_from_all_skipped() {
        let empty = PoolScanOutcome {
            candidates: 0,
            ok: 0,
            fail: 0,
            skipped_analyzed_today: 0,
            truncated: false,
            first_error: None,
            details: vec![],
        };
        assert!(empty.render().contains("候选池为空"));
        let all_skipped = PoolScanOutcome {
            candidates: 0,
            ok: 0,
            fail: 0,
            skipped_analyzed_today: 3,
            truncated: false,
            first_error: None,
            details: vec![],
        };
        let text = all_skipped.render();
        assert!(text.contains("当日已分析过"), "text = {text}");
        assert!(!text.contains("候选池为空"), "全跳过不得报成空池: {text}");
    }
}
