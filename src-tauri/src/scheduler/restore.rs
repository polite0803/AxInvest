// SPDX-License-Identifier: AGPL-3.0-only

//! 任务「不丢失、不重复」能力 — 恢复引导。
//!
//! 启动时扫描 `background_tasks` 中未完成（pending / running）的任务，重新入队。
//! - `bash` 类型：`attempt += 1` 后重跑（临时失败重试）。
//! - `agent` 类型：从 `resume_from`（断点位置）续跑；无断点则复位为 pending。
//!
//! Tauri 命令入口 `restore_pending_tasks`（手动触发，调试用），启动钩子
//! 对应实现由 `init/services.rs` 调用，通过传入 `DatabaseConnection` 与
//! 可选的 `AppHandle` 发送事件，保持本模块与 `AppState` 解耦。
//!
//! 另含**僵死分析回收** [`reclaim_stale_stock_analyses`]：`stock_analyses` 停在 `running`
//! 的行**不适用**上面的「重新入队」语义 —— 股票分析的 DAG 是进程内 fire-and-forget
//! 任务、没有跨进程恢复机制，只能标 `interrupted`。判据、实测与残留风险见该函数文档。

use axagent_entities::{background_tasks, stock_analyses};
use axagent_harness::task_state::{TaskSource, TaskStatus};
use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, Set};
use tauri::{Emitter, State};
use tracing::{error, info};

use crate::AppState;

/// 未完成任务将重新入队的可见状态集合
const INCOMPLETE: &[&str] = &["pending", "running"];

/// Tauri 命令：手动触发恢复引导（调试/运维用）。
#[tauri::command]
pub async fn restore_pending_tasks(
    state: State<'_, AppState>,
    app_handle: tauri::AppHandle,
) -> Result<Vec<String>, String> {
    let db = state.harness.db().clone();
    restore_incomplete_tasks(&db, Some(&app_handle)).await.map_err(|e| {
        crate::commands::error::ErrorResponse::from_error(
            e,
            crate::commands::error::ErrorCategory::Retryable,
        )
        .to_string()
    })
}

/// 恢复引导核心逻辑：扫描未完成任务 → 置回 pending（attempt 自增）。
///
/// 幂等：每个任务原子地复位为 pending；重复调用不会重复计数。
/// 返回被恢复的任务 id 列表。
pub async fn restore_incomplete_tasks(
    db: &sea_orm::DatabaseConnection,
    app_handle: Option<&tauri::AppHandle>,
) -> Result<Vec<String>, sea_orm::DbErr> {
    let tasks = background_tasks::Entity::find()
        .filter(background_tasks::Column::Status.is_in(INCOMPLETE.to_vec()))
        .all(db)
        .await?;

    let mut recovered = Vec::new();
    for task in tasks {
        let task_id = task.id.clone();
        let current = task.status.clone();
        let next_attempt = task.attempt.saturating_add(1);
        // agent 任务无断点则从零开始；有断点保留 resume_from 供续跑。
        let resume_from = if task.task_type == "agent" {
            task.resume_from.clone()
        } else {
            None
        };

        // ── 状态列走唯一写路径（P1-C）──
        // 修复前这里直接 `am.status = Set("pending")`，是 `background_tasks.status`
        // 的第二个写路径（另一个在 `commands/background_tasks.rs`），且 `updated_at`
        // 用的是秒（命令层用毫秒）。现在状态迁移经账本，附带迁移校验 + 事件脊 + 毫秒。
        let from = TaskStatus::from_db_str(&current);
        let already_pending = from == TaskStatus::Pending;
        if !already_pending {
            if let Err(e) = axagent_dao::task_ledger::transition_task(
                db,
                axagent_dao::task_ledger::TransitionRequest::new(
                    &task_id,
                    TaskStatus::Pending,
                    TaskSource::Restore,
                    "scheduler",
                )
                .reason("启动恢复：未完成任务重新入队"),
            )
            .await
            {
                // 迁移被拒（例如状态未知）：**跳过**该任务而不是继续写 attempt。
                // 若继续写 attempt 而不改状态，会出现「attempt 增加了但状态没变」
                // 的账实不符 —— 比起漏恢复一个任务，静默的账实不符更难查。
                error!(
                    task_id = %task_id, raw_status = %current,
                    "[scheduler.restore] 跳过：状态迁移被拒: {}", e
                );
                continue;
            }
        }

        // ── 非状态列（attempt / resume_from）：状态列已由账本处理 ──
        // 这两列不参与状态迁移，故在此单独更新；`status` **不在这里写**，
        // 以保证「status 只有一个写路径」这条不变量成立。
        let mut am: background_tasks::ActiveModel = task.into();
        am.resume_from = Set(resume_from);
        am.attempt = Set(next_attempt);
        am.updated_at = Set(chrono::Utc::now().timestamp_millis());
        am.update(db).await?;
        recovered.push(task_id);
    }

    if !recovered.is_empty() {
        info!("[scheduler.restore] 恢复 {} 个未完成任务: {:?}", recovered.len(), recovered);
        if let Some(app) = app_handle {
            let _ = app.emit("background-task:restored", &recovered);
        }
    } else {
        info!("[scheduler.restore] 无不完整后台任务，无需恢复");
    }
    Ok(recovered)
}

/// `stock_analyses.status` 的「已中断」取值 —— 进程在分析跑完前消失，终态从未写入。
///
/// ## 为什么需要这个值（2026-10-17 用户报障实证）
///
/// `run_stock_workflow_inner` 与 `run_single_stock_analysis` 都是 **fire-and-forget**：
/// 建行即写 `status="running"`，再把 DAG `tokio::spawn` 到后台并立即返回
/// （`commands/stock_workflow/core.rs:621` / `:2028`）。而四个终态分支
/// （`:1081` timeout、`:1112` cancelled、`:2254` completed、`:2369` failed）
/// **全在那个后台任务里** ⇒ 进程一旦非优雅退出（硬杀 / 崩溃 / 关机 / runtime 先于
/// select 分支被 poll 到），**没有任何分支有机会写终态**，该行就永久停在 `running`。
///
/// 后果不止是历史列表难看：前端 `loadAnalysis` 把任何非 `completed` 的记录一律置为
/// `running`（`src/stores/feature/stockAnalysisStore.ts:1451`），而 `running` 同时触发
/// `StockSearchBar` 的按钮 `disabled` 与 `startAnalysis` 的防重入 `return`
/// ⇒ **打开一条僵死记录就把「开始分析」永久锁死**（请求根本发不到后端）。
///
/// 实测（2026-10-17 06:59，生产库）：35 条 `running` 的四个产出列
/// （`template_version` / `decision_action` / `decision_json` / `blackboard_snapshot`）
/// 非空计数**全为 0** —— 与建行占位行逐字段一致，即建行后一个字段都没再写。
///
/// ## 语义边界（别当成"另一种失败"）
///
/// 本值**只由启动回收通路写**（见下方 `reclaim_stale_stock_analyses`）。运行期的正常
/// 终态是 `completed` / `failed` / `timeout` / `cancelled`，各由上面那四个分支写。
/// `failed` 是**跑过了并失败**，`interrupted` 是**没有结论地消失**，两者不可互替。
pub const STOCK_ANALYSIS_STATUS_INTERRUPTED: &str = "interrupted";

/// 启动回收：把**上一进程遗留**、停在 `running` 的股票分析行标为 `interrupted`。
///
/// ## 判据（为什么可以一刀切）
///
/// 股票分析的 DAG 是**进程内** fire-and-forget 任务，没有任何跨进程恢复机制
/// （对比本文件上方对 `background_tasks` 的「重新入队」）。⇒ 本进程启动那一刻，
/// 内存中**零个**在跑的分析 ⇒ 此刻库里所有 `running` 行的持有者必然已经消失，
/// 它们不可能再前进到任何终态，回收是**状态订正**而非干预。
///
/// ## 为什么必须带 `cutoff`，不能无条件扫全表
///
/// 本函数在启动后**延迟**执行（避开启动期 DB 竞争，与 `start_scheduler_recovery` 同因）。
/// 若延迟期间用户已发起新分析，那条新行同样是 `running` —— 无条件回收会**误杀正在跑的
/// 分析**。故只回收 `created_at < cutoff` 的行；`cutoff` 取**启动钩子被调用的那一刻**
/// （早于任何用户操作），延迟期间新建的行天然落在边界之外。
///
/// ## 残留风险（已知且刻意接受）
///
/// 本判据的隐含前提是「本进程是唯一的分析执行者」。若**另一个进程**
/// （`axagent-server` 的 server 模式 / `axagent-batch-rerun` 离线批量）在本进程启动
/// **之前**就已在跑分析，它的行 `created_at < cutoff` 会被误标。当前形态下三者不并发；
/// 若将来并发，判据须换成「活跃工作流注册表」而不是启动边界。
pub async fn reclaim_stale_stock_analyses(
    // ⚠ 用全路径而非裸 `DatabaseConnection`：本文件**没有** import 该类型，
    // 既有函数 `restore_incomplete_tasks` 同样是全路径写法（import 面刻意保持最小）。
    // 2026-10-17 实测：写成裸名 ⇒ clippy 报 E0425，且因主 crate 是全量最后一环，
    // 前面 14 个 crate 全都 Checking 通过、只有这一处红。
    db: &sea_orm::DatabaseConnection,
    cutoff_ms: i64,
) -> Result<u64, sea_orm::DbErr> {
    use sea_orm::sea_query::Expr;
    let res = stock_analyses::Entity::update_many()
        .col_expr(stock_analyses::Column::Status, Expr::value(STOCK_ANALYSIS_STATUS_INTERRUPTED))
        .col_expr(
            stock_analyses::Column::UpdatedAt,
            Expr::value(chrono::Utc::now().timestamp_millis()),
        )
        .filter(stock_analyses::Column::Status.eq("running"))
        .filter(stock_analyses::Column::CreatedAt.lt(cutoff_ms))
        .exec(db)
        .await?;
    Ok(res.rows_affected)
}

/// 外部批量重跑（`axagent-batch-rerun`）的运行标记文件名（与 bin 内 `RERUN_LOCK_FILE` 同值）。
///
/// ⚠ 两处各写一份字面量是刻意的：bin 是**独立目标**，把 bin 专用常量提到 lib 公开 API
/// 只为共享一个字符串并不划算。**改一处必须改另一处** —— 判据是 bin 写
/// `~/.axagent/batch-rerun.lock`、本模块读同一路径。
pub const EXTERNAL_RERUN_LOCK_FILE: &str = "batch-rerun.lock";

/// 运行标记的「新鲜」窗口：超过它视为**陈旧残留**（bin 被硬杀时其 `Drop` 未执行），
/// 回收照常进行 —— 不需要人工清理那个文件。
const EXTERNAL_RERUN_FRESH: std::time::Duration = std::time::Duration::from_secs(6 * 3600);

/// 是否有**外部批量重跑**正在进行（依据 `~/.axagent/batch-rerun.lock` 的新鲜度）。
///
/// ## 为什么需要这个判据
///
/// `reclaim_stale_stock_analyses` 用「进程启动边界」判僵死，其隐含前提是
/// **本进程是唯一的分析执行者**。而 `axagent-batch-rerun` 是**另一个进程**，
/// 它重跑期间那些行同样是 `running` ⇒ 应用此刻启动会把它们误判成僵死，
/// 并进而让 bin 的 `wait_until_settled` 提前认定落定、启动下一条（两条并发跑）。
/// 细节见 bin 内 `RerunLockGuard` 的文档。
///
/// ## 不确定时取**保守侧**
///
/// 读不到 mtime（权限异常 / 时钟倒退）时返回 `true`（当作「正在跑」）：
/// 宁可这次不回收，也不要误杀正在进行的重跑 —— 回收是幂等的，下次启动还会做。
pub fn external_rerun_in_progress() -> bool {
    let path = crate::axagent_home().join(EXTERNAL_RERUN_LOCK_FILE);
    let Ok(meta) = std::fs::metadata(&path) else {
        // 文件不存在 ⇒ 没有外部重跑（这是**唯一**可以宣告「没有」的分支）。
        return false;
    };
    let Ok(modified) = meta.modified() else {
        return true;
    };
    match modified.elapsed() {
        Ok(age) => age < EXTERNAL_RERUN_FRESH,
        // 时钟倒退（`elapsed` 报 Err）⇒ 保守认为新鲜。
        Err(_) => true,
    }
}
