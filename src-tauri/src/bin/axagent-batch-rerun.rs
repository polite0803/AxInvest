// SPDX-License-Identifier: AGPL-3.0-only

//! axagent-batch-rerun —— 批量 as-of 重跑历史股票分析（离线，不需要 Tauri 前端）。
//!
//! ## 用途
//!
//! 工作流算法升版后（如 `TEMPLATE_VERSION` 一路上抬），库内历史分析的结论是按**旧公式**
//! 算出来的，与当前版本口径不一致。本 bin 把 `stock_analyses` 的历史记录用**当前版本**
//! 重算，使全库口径对齐。
//!
//! ## 为什么必须锚定 `as_of_date = 原分析日`
//!
//! 只有「同一时点、同一数据」重算，结论变化才能**唯一归因于公式改动**；
//! 若用 live（当前时点）重算，「公式变了」与「行情变了」混在一起，无法归因。
//!
//! 附带收益：重算后的分析，其 `hindsight_date`（= as_of + 期望持有期）**已经在过去**
//! ⇒ 落地即可进入反思队列，不必再等一个持有期。这是「对齐口径」与「快速积累反思数据」
//! 能用同一次重跑完成的原因。
//!
//! ## ⚠ 破坏性语义（动手前必读）
//!
//! 传 `parent_analysis_id` + 与原记录**同日**的 `as_of_date`，会命中
//! `run_stock_workflow_inner` 的「replay 同日覆盖」分支（`core.rs` 的版本化策略注释）：
//! **原地 UPDATE 原记录**并清空其决策字段，**不是**新建版本行。
//! ⇒ 执行前必须已有备份。本项目用库内备份表 `stock_analyses_backup_<YYYYMMDD>`
//! （见 `PLAN-analysis-data-repair.md` §9）。
//!
//! ## 用法
//!
//! ```text
//! cargo run --bin axagent-batch-rerun                       # 默认 dry-run，只打印清单
//! cargo run --bin axagent-batch-rerun -- --limit 5 --apply # 先跑 5 条验证链路
//! cargo run --bin axagent-batch-rerun -- --limit 0 --apply # 全量（0 = 不限）
//! cargo run --bin axagent-batch-rerun -- --codes 000001,600028 --apply
//! cargo run --bin axagent-batch-rerun -- --since 2026-08-01 --until 2026-08-31 --apply
//! cargo run --bin axagent-batch-rerun -- --only-stale --apply   # 只处理 template_version 为 NULL
//! ```
//!
//! 脚本化调用（与 `scripts/pg-connect.mjs` 同款理由：口令不进 argv、不烧进源码）：
//! 本 bin 自身不接触任何凭据 —— 它复用应用已初始化好的 `~/.axagent` 数据目录。

use std::sync::Arc;

use axagent_astock_data::as_of::{self, AsOfContext};
use axagent_entities::stock_analyses;
use axagent_entities::stock_reflections;
use axagent_entities::strategy_performance;
use axagent_lib::{
    AppState, axagent_home, create_app_state, init_database_with_dir, run_stock_workflow_inner,
};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, EntityTrait, PaginatorTrait, QueryFilter, QueryOrder, Set,
};

const HELP: &str = "\
axagent-batch-rerun —— 批量 as-of 重跑历史股票分析

用法:
  axagent-batch-rerun [选项]

选项:
  --apply              真正执行重跑（**不加则只 dry-run 打印清单**）
  --limit <N>          最多处理 N 条；0 或省略 = 不限
  --codes <A,B,C>      只处理这些股票代码
  --since <YYYY-MM-DD> 只处理 analysis_date >= 该日
  --until <YYYY-MM-DD> 只处理 analysis_date <= 该日
  --only-stale         只处理 template_version IS NULL 的记录
  --reflect-after      重跑完成后，在同一进程内立即跑一轮批量反思
  --reflect-max <N>    --reflect-after 单轮反思条数上限（默认 20）
  -h, --help           显示本帮助

说明:
  重跑锚定 as_of_date = 原 analysis_date，并传 parent_analysis_id ⇒
  **原地覆盖**原记录（非新建版本），执行前请确认已有备份。

  ⚠ 每次成功重跑都会**补建/重置**该分析的反思 pending 行：
  重跑对同日 as-of 是原地覆盖同一 analysis_id ⇒ 既有反思（无论 pending
  还是 completed）对应的旧决策已失效，故重置为 pending（清结论列、
  保留 raw_return/alpha_return/holding_days 等实测列）。
  这是必须的 —— 业务封装路径本身**不创建**反思 pending 行。

  为什么 --reflect-after 有价值：补建出的 pending 行其 hindsight_date
  （= as_of + 期望持有期）**已经落在过去** ⇒ 立即可反思，不必等应用内
  那个 6 小时兜底循环（init/services.rs 的 start_batch_reflection）被动消费。
  这正是「对齐口径」与「快速积累反思数据」共用一次重跑的关键。

数据库（重要）:
  本 bin 复用应用配置（~/.axagent/db_config.json）。⚠ 若 Postgres 不可达，
  应用会**静默降级到本地 SQLite 空库** ⇒ 那会让「匹配 0 条」看起来像「已修完」。
  本 bin 已硬拦此情形（连到的不是 Postgres 就直接报错）。
  建议始终以 `AXAGENT_DB_FALLBACK_TO_SQLITE=0` 运行，让降级在初始化阶段即报错。";

/// 命令行参数。
///
/// 刻意不引入 clap —— 本项目 bin 的依赖面保持最小，且 `axagent-server` 已是同样风格。
#[derive(Debug, Default)]
struct Args {
    apply: bool,
    limit: usize,
    codes: Vec<String>,
    since: Option<String>,
    until: Option<String>,
    only_stale: bool,
    /// 重跑完成后，在同一进程内立即跑一轮批量反思。
    reflect_after: bool,
    /// 反思的 `max_count`（None ⇒ 用 `run_batch_reflection_inner` 内部默认 20）。
    reflect_max: Option<u32>,
}

impl Args {
    fn parse() -> Result<Self, String> {
        let mut a = Args::default();
        let mut it = std::env::args().skip(1);
        while let Some(arg) = it.next() {
            match arg.as_str() {
                "--apply" => a.apply = true,
                "--only-stale" => a.only_stale = true,
                "--reflect-after" => a.reflect_after = true,
                "--limit" => {
                    let v = it.next().ok_or_else(|| "--limit 需要一个数字".to_string())?;
                    a.limit = v.parse().map_err(|e| format!("--limit 解析失败: {e}"))?;
                },
                "--reflect-max" => {
                    let v = it.next().ok_or_else(|| "--reflect-max 需要一个数字".to_string())?;
                    a.reflect_max =
                        Some(v.parse().map_err(|e| format!("--reflect-max 解析失败: {e}"))?);
                },
                "--codes" => {
                    let v = it.next().ok_or_else(|| "--codes 需要逗号分隔的代码".to_string())?;
                    a.codes = v
                        .split(',')
                        .map(|s| s.trim().to_string())
                        .filter(|s| !s.is_empty())
                        .collect();
                },
                "--since" => {
                    a.since = Some(it.next().ok_or_else(|| "--since 需要 YYYY-MM-DD".to_string())?);
                },
                "--until" => {
                    a.until = Some(it.next().ok_or_else(|| "--until 需要 YYYY-MM-DD".to_string())?);
                },
                "-h" | "--help" => {
                    println!("{HELP}");
                    std::process::exit(0);
                },
                other => return Err(format!("未知参数: {other}（用 --help 看用法）")),
            }
        }
        Ok(a)
    }
}

/// 清理某标的的旧「反思绩效」行（`strategy_id = 'reflection_verdict'`），返回删除行数。
///
/// ## 为什么必须清
///
/// `strategy_performance` 的行是**反思结论的量化投影**：`was_correct` 由
/// `deterministic_was_correct(决策方向, 行情快照)` 产出（`reflection.rs:620`），
/// 决策被覆盖后旧行即失去依据。
///
/// 消费端 `compute_adjusted_weights` 按 **(strategy_id, period)** 聚合
/// （`weight_decay.rs:77`），**不看 stock_code**
/// ⇒ 同一标的的「旧决策行 + 新决策行」会**各计一次样本**，
/// 直接拉偏 `sample_size` 与 `win_rate`（还有 `weight_decay.rs:89` 的贝叶斯平滑分母）。
/// 该表只有 `id` 主键、**无 `(stock_code, strategy_id)` 唯一约束**（实测 `pg_index`）⇒
/// 不会自动去重，重复行会静默累积。
///
/// ## 粒度说明（为什么按 stock_code 而不是按某次分析）
///
/// 该表**没有 `analysis_id` 列**，无法精确关联到某一次分析。
/// 按 `stock_code` 删在本次数据修复中是**一致**的：11 条 completed 反思对应的分析
/// `template_version` 全为 NULL（实测），全部落在重跑范围内 ⇒ 该标的的旧行全部作废。
///
/// ⚠ 若将来在「只重跑部分分析」的场景复用它，应按本函数注释重新评估粒度。
///
/// 回滚点：`strategy_performance_backup_20260923`（10 行，2026-09-23 建）。
async fn delete_stale_reflection_perf(db: &sea_orm::DatabaseConnection, stock_code: &str) -> u64 {
    strategy_performance::Entity::delete_many()
        .filter(strategy_performance::Column::StockCode.eq(stock_code))
        // 只删反思产生的行 —— 该表将来若承载其它策略，不应被本工具波及。
        .filter(strategy_performance::Column::StrategyId.eq("reflection_verdict"))
        .exec(db)
        .await
        .map(|r| r.rows_affected)
        .unwrap_or(0)
}

/// 等待某条分析离开 `running`，返回其**终态** status 字符串（或超时说明）。
///
/// ## 为什么必须等（这是本 bin 最容易被漏掉的一环）
///
/// 业务封装路径 `run_stock_workflow_inner` **不等待 DAG**：它把工作流
/// `tokio::spawn` 到后台后**立即返回**（`core.rs:826-837` 的 `tokio::spawn`
/// \+ `:885` 的 `engine.run_workflow`）。这是**为前端设计**的 fire-and-forget 形态：
/// 前端拿到 `analysisId` 后自己轮询 `status`。
///
/// 但在**离线 bin** 里，`run()` 一返回、`main` 一结束，tokio runtime 就被 drop，
/// 后台 DAG 随之被取消。实测表现（2026-09-23，`rerun-V4.log`）：
///
/// - 每条只耗 **2.7~10.6 秒**（正常单条分析 ≥1 分钟）；
/// - `stock_analyses.status` 停在 `running`；
/// - 日志出现 **`task was cancelled`**（`error communicating with database`）。
///
/// ⇒ 若不等待，本 bin 会**静默地什么都没做成**，却照样输出「已处理 N 条」的报告。
/// 判据：**目标是 fire-and-forget 时，调用方必须自己负责等待**；
/// 把 `await` 当成「已落定」是跨异步边界的经典误读
/// （同族：`Option`/`Result` 被 `unwrap_or_default()` 吞掉）。
///
/// ## 为什么轮询 DB 而不是拿内部句柄
///
/// DAG 已 spawn 走、句柄不可得；而 `status` 是**前端也在用**的同一权威落地信号
/// ⇒ 轮询它与「用户看到的状态」严格一致，且不依赖任何内部实现细节。
/// 超时上限按「实盘单条分析的最坏耗时」留足（30 分钟）。
async fn wait_until_settled(
    db: &sea_orm::DatabaseConnection,
    analysis_id: &str,
    timeout: std::time::Duration,
) -> String {
    let started = std::time::Instant::now();
    loop {
        let status = stock_analyses::Entity::find_by_id(analysis_id)
            .one(db)
            .await
            .ok()
            .flatten()
            .map(|m| m.status)
            .unwrap_or_else(|| "<记录消失>".to_string());
        if status != "running" {
            return status;
        }
        if started.elapsed() >= timeout {
            return format!("timeout（等待 {}s 仍未落定）", timeout.as_secs());
        }
        tokio::time::sleep(std::time::Duration::from_secs(3)).await;
    }
}

/// 为重跑后的分析补建（或重置）反思 pending 行，返回一行人类可读的结果说明。
///
/// ## 为什么必须由本 bin 做
///
/// 业务封装路径（`run_stock_workflow_inner`）**不产生**反思 pending 行：
/// `hooks.rs` 的 `stock-analysis-persist` 有守卫
/// `if input_has_analysis_id(&ctx.input) { return Ok(()) }`（`hooks.rs:779-784`），
/// 而业务封装路径的 input 恒带 `analysis_id` ⇒ 该 hook 被跳过；它只服务
/// **对话直执行通道**（`analysis_kind = "chat"`，见 `hooks.rs:18-20` 模块注释）。
///
/// DB 证据（2026-09-23）：现存 11 条 completed 反思**全部**来自
/// `workflow / live`（单股即时分析通道 `run_single_stock_analysis`），
/// 业务封装路径零产出 ⇒ 「对齐当前模板版本」与「积累反思样本」之间是**断的**。
///
/// 本 bin 作为**一次性数据修复工具**补上这一环。产品侧的持续缺口（所有业务封装
/// 路径都不产反思样本）另行登记，**不**在本 bin 内顺手改产品核心函数 ——
/// `run_stock_workflow_inner` 有 1300 行、多个成功分支（`core.rs:1053`/`:1368`
/// 各写一次 template_version），改动面与回归风险都过大。
///
/// ## 幂等与「重置」语义
///
/// 重跑对同日 as_of 是**原地覆盖**同一 `analysis_id` ⇒ 已存在的反思（无论 pending
/// 还是 completed）都对应**上一版决策**，其结论对新决策无效 ⇒ 必须**重置**为 pending：
/// 清空结论列，让下一轮反思基于新决策重写。
///
/// **保留** `raw_return` / `alpha_return` / `holding_days`：它们是**实测事实**，
/// 与决策版本无关，且反思重跑时会按最新行情覆盖。
///
/// ## 锚点
///
/// `as_of_date` = 分析记录自身的 `as_of_date`（as-of 重跑时 = 原分析日），
/// `hindsight_date` = 该锚点 + `decision_expected_holding_days`（默认 28）
/// ⇒ **落在过去** ⇒ 反思立即可执行（不必等 28 天）。
///
/// 这正是「重跑一次同时完成对齐口径 + 攒反思样本」的机制：`hindsight_date`
/// 以分析日而非重跑日为基准，故历史分析的反思时点天然已到。
/// 返回值 = `(是否已补建/重置, 人类可读说明)`。
///
/// `false` 表示**分析本身未成功**（`failed: …` / `timeout` / `cancelled`）⇒ 不补建。
/// 调用方据此区分「重跑动作成功」与「分析真的产出决策」，避免把失败分析计入成功数，
/// 也避免为其创建会污染样本集的反思行。
///
/// 为什么不用字符串前缀让调用方判断：靠解析人类可读文本做控制流是最脆的耦合
/// （文案一改，判断静默失效）。
async fn ensure_pending_reflection(
    db: &sea_orm::DatabaseConnection,
    analysis_id: &str,
    fallback_anchor: &str,
) -> Result<(bool, String), String> {
    let rec = stock_analyses::Entity::find_by_id(analysis_id)
        .one(db)
        .await
        .map_err(|e| format!("读分析记录失败: {e}"))?
        .ok_or_else(|| format!("分析 {analysis_id} 不存在"))?;

    // ⚠ 只对**成功**的分析补建反思样本。
    //
    // `run_stock_workflow_inner` 在 DAG 失败时**不返回 Err** —— 它把
    // `stock_analyses.status` 置为 `failed: {e}` 后照常返回 `Ok(json)`
    // （见 `core.rs:1645-1660`）。⇒ 若只看返回值，失败分析会被当成成功，
    // 于是为一条**根本没产出决策**的分析创建 pending 反思 ⇒ 污染样本集。
    //
    // 成功态字面值实测：`completed`（`core.rs:1020` / `:1335`）。
    // 其余值域：`running` / `failed: {e}` / `timeout`（`:925`）/ `cancelled`（`:956`）。
    if rec.status != "completed" {
        return Ok((false, format!("跳过反思补建（分析状态 {}）", rec.status)));
    }

    let anchor = rec.as_of_date.clone().unwrap_or_else(|| fallback_anchor.to_string());
    let hold_days = rec.decision_expected_holding_days.unwrap_or(28).max(1);
    let hindsight = chrono::NaiveDate::parse_from_str(&anchor, "%Y-%m-%d")
        .map(|d| d + chrono::Duration::days(hold_days))
        .map_err(|e| format!("锚点 {anchor} 解析失败: {e}"))?
        .format("%Y-%m-%d")
        .to_string();

    let now = chrono::Utc::now().timestamp_millis();
    let existing = stock_reflections::Entity::find()
        .filter(stock_reflections::Column::OriginalAnalysisId.eq(analysis_id))
        .all(db)
        .await
        .map_err(|e| format!("查反思行失败: {e}"))?;

    if existing.is_empty() {
        let id = uuid::Uuid::new_v4().to_string();
        stock_reflections::ActiveModel {
            id: Set(id.clone()),
            stock_code: Set(rec.stock_code.clone()),
            stock_name: Set(rec.stock_name.clone()),
            original_analysis_id: Set(analysis_id.to_string()),
            as_of_date: Set(anchor),
            hindsight_date: Set(hindsight),
            // 与 `hooks.rs:888`（对话通道）/ `core.rs`（单股通道）的产品默认值一致。
            // ⚠ 这是**手抄常量**：三处各写一份，变更时需同步；产品侧缺口修复后
            // 应改为共享函数（见本函数文档首段）。
            min_confidence_threshold: Set(70),
            reflection_depth: Set("light".to_string()),
            actual_outcome: Set(String::new()),
            raw_return: Set(None),
            alpha_return: Set(None),
            holding_days: Set(None),
            benchmark_name: Set(None),
            verdict: Set(None),
            alpha_cited: Set(None),
            lesson_summary: Set(None),
            what_went_wrong: Set(None),
            missed_signals: Set(None),
            fix_for_future: Set(None),
            parameter_suggestions_json: Set(None),
            decision_json: Set(None),
            blackboard_snapshot: Set(None),
            model_version: Set(None),
            status: Set("pending".to_string()),
            created_at: Set(now),
            updated_at: Set(now),
        }
        .insert(db)
        .await
        .map_err(|e| format!("写反思 pending 行失败: {e}"))?;
        let sp = delete_stale_reflection_perf(db, &rec.stock_code).await;
        return Ok((true, fmt_reflect_note(&format!("补建 pending {id}"), sp)));
    }

    // 已存在 ⇒ 重置为 pending（重跑已覆盖原分析，旧反思结论失效）。
    // 用 `all()` 而非 `one()`：同一 analysis_id 理论上唯一，但历史数据无唯一约束，
    // 出现多行时逐行重置比"只处理第一行"更安全（漏重置的行会被反思当作 pending 消费，
    // 但那行若仍是 completed，就会被跳过 ⇒ 静默少一个样本）。
    let n = existing.len();
    for row in existing {
        let id = row.id.clone();
        let mut am: stock_reflections::ActiveModel = row.into();
        am.as_of_date = Set(anchor.clone());
        am.hindsight_date = Set(hindsight.clone());
        am.status = Set("pending".to_string());
        am.verdict = Set(None);
        am.alpha_cited = Set(None);
        am.lesson_summary = Set(None);
        am.what_went_wrong = Set(None);
        am.missed_signals = Set(None);
        am.fix_for_future = Set(None);
        am.parameter_suggestions_json = Set(None);
        am.decision_json = Set(None);
        am.blackboard_snapshot = Set(None);
        am.actual_outcome = Set(String::new());
        // ⚠ `raw_return` / `alpha_return` / `holding_days` / `benchmark_name` **故意不清**。
        //
        // 直觉上它们「属于旧决策」，但实际不是：它们的语义是
        // **「从 `analysis_date` 到反思评估时点这段行情的客观事实」** ——
        // 起点是分析日（重跑不改变、仍是原 `analysis_date`），与**决策内容无关**。
        // 决策方向变了，同一段行情的收益不会变。
        //
        // 且反思重跑成功后会按最新行情**回写覆盖**这 4 列
        // （`reflection.rs:598-600` 的 `col_expr(Column::RawReturn, ...)` 等，即 D10 修复点）。
        // ⇒ 保留既可读（重置期间不至于把客观事实抹成 NULL），也不会残留错误值。
        //
        // （本条曾被我按「与决策绑定」误判为应清空，已纠正。判据：
        //  先问**这个量在物理上依赖决策内容吗**，再问它是否该随版本失效。）
        //
        // `model_version` 恒为 NULL（全仓只有 `Set(None)` 写入点、无实值赋值，
        // 见 PLAN D7）⇒ 无需处理。
        am.updated_at = Set(now);
        am.update(db).await.map_err(|e| format!("重置反思 {id} 失败: {e}"))?;
    }
    let sp = delete_stale_reflection_perf(db, &rec.stock_code).await;
    Ok((true, fmt_reflect_note(&format!("重置 {n} 条旧反思为 pending"), sp)))
}

/// 把「清理旧绩效行数」拼进反思说明里；为 0 时不追加噪声。
fn fmt_reflect_note(base: &str, perf_deleted: u64) -> String {
    if perf_deleted > 0 {
        format!("{base}，清理旧绩效 {perf_deleted} 行")
    } else {
        base.to_string()
    }
}

#[tokio::main]
async fn main() {
    // 日志默认 info，可用 RUST_LOG 覆盖（与 server_main.rs 同款初始化）。
    let _ = tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .try_init();

    if let Err(e) = run().await {
        eprintln!("[batch-rerun] 失败: {e}");
        std::process::exit(1);
    }
}

async fn run() -> Result<(), String> {
    let args = Args::parse()?;

    // ── 初始化：与 `server_main.rs` 同路径，无 Tauri 依赖 ──
    let app_dir = axagent_home();
    std::fs::create_dir_all(&app_dir).map_err(|e| format!("创建数据目录失败: {e}"))?;
    let db_result = init_database_with_dir(app_dir.clone())
        .await
        .map_err(|e| format!("数据库初始化失败: {e}"))?;
    let state: Arc<AppState> = Arc::new(create_app_state(db_result).await?);

    // ── 工具解析器注入（**不是可选步骤**）──
    //
    // `create_app_state` 只跑「关键路径」，**不启动后台服务**；而工具解析器的注入恰好
    // 写在 `start_cron_scheduler`（`init/services.rs`）内部 —— 它要 `AppHandle`，本 bin 没有。
    //
    // 后果（2026-09-23 实测）：DAG 里**每个数据节点**都报
    // `TOOL_CALL_FAILED: 工具 'get_stock_kline' 未注册`（`tool_executor.rs:177`，
    // 因为 resolver / tool_registry / Rhai 缓存三条来源全空），
    // 4/4 分析「执行了但失败」**且耗时只有几秒** —— 极易被误读成「模型输出质量差」。
    //
    // 故此处显式注入，且与 Tauri 应用**共用同一个函数**，使 bin 与应用的装配路径不分叉。
    axagent_lib::init::services::inject_work_engine_tool_resolver(&state).await;

    let db = state.harness.db().clone();

    // ── 库身份断言：把「静默降级」变成硬失败 ──
    //
    // `init_database_with_dir` 在 **Postgres 不可达时会默认降级到本地 SQLite**
    // （`init/database.rs:163` 分支 + `fallback_enabled()` 的 `unwrap_or(true)`）。
    // 对普通客户端启动这是**合理的可用性设计**，但对**批量重跑**是灾难：
    // 那种情况下本 bin 会连到一个**空库**、匹配 0 条，而输出与
    // 「目标已全部修完 / 无需重跑」**一模一样** —— 一个假阴性，且不会被任何人察觉。
    //
    // 故此处硬拦：连到的不是 Postgres 就直接失败。宁可让使用者去修连接，
    // 也不要交出一份「看起来成功、实际什么都没做」的报告。
    // （同族纪律：D 组 fail-open / 降级吞 Err。）
    //
    // ⚠ 只取 scheme 前缀输出，**绝不打印 URL 全文** —— PG 连接串里带口令。
    let db_ident = state.harness.db_path();
    if !db_ident.starts_with("postgres://") && !db_ident.starts_with("postgresql://") {
        let scheme = db_ident.split("://").next().unwrap_or("<无法识别>");
        return Err(format!(
            "连到的是 `{scheme}`，不是 Postgres ⇒ 极可能是 Postgres 不可达、\
             `init_database_with_dir` 已**静默降级到本地 SQLite**。\n\
             此时任何「匹配 0 条」都**不代表**目标已修完（那是空库）。\n\
             处置：① 确认 Postgres 可达（`node scripts/pg-connect.mjs sql \"select 1\"`）；\n\
                ② 或用 `AXAGENT_DB_FALLBACK_TO_SQLITE=0` 重跑本 bin，\
             使降级变成**初始化阶段的硬报错**，而不是换个空库继续跑。"
        ));
    }

    // ── 目标集合 ──
    let mut q = stock_analyses::Entity::find();
    if args.only_stale {
        q = q.filter(stock_analyses::Column::TemplateVersion.is_null());
    }
    if !args.codes.is_empty() {
        q = q.filter(stock_analyses::Column::StockCode.is_in(args.codes.clone()));
    }
    if let Some(s) = &args.since {
        q = q.filter(stock_analyses::Column::AnalysisDate.gte(s.clone()));
    }
    if let Some(u) = &args.until {
        q = q.filter(stock_analyses::Column::AnalysisDate.lte(u.clone()));
    }
    let mut rows = q
        .order_by_asc(stock_analyses::Column::AnalysisDate)
        .order_by_asc(stock_analyses::Column::StockCode)
        .all(&db)
        .await
        .map_err(|e| format!("查询 stock_analyses 失败: {e}"))?;

    let matched = rows.len();
    if args.limit > 0 && rows.len() > args.limit {
        rows.truncate(args.limit);
    }

    println!("[batch-rerun] 匹配 {matched} 条，本次处理 {} 条：", rows.len());
    for r in &rows {
        println!(
            "  - {} {} analysis_date={} as_of_date={} template_version={}",
            r.stock_code,
            r.stock_name,
            r.analysis_date,
            r.as_of_date.as_deref().unwrap_or("<NULL>"),
            r.template_version.map(|v| v.to_string()).unwrap_or_else(|| "NULL".into()),
        );
    }

    if !args.apply {
        println!(
            "\n[batch-rerun] dry-run：未执行任何重跑。\n\
             确认清单无误后加 --apply 真正执行（注意：会**原地覆盖**上述记录）。"
        );
        return Ok(());
    }
    if rows.is_empty() {
        println!("[batch-rerun] 无对象，退出。");
        return Ok(());
    }

    // ── 逐条重跑 ──
    // 刻意**串行**：本项目有「高并发触发 LLM 供应商限流降级」的历史教训，
    // 而重跑是长任务，宁可慢也不要把降级样本混进反思数据集。
    //
    // ── as-of 降级体检（回放前的必要体检项）──
    // `record_degradation` 的全局累计计数器**不依赖** `with_degradation_log` 的
    // task_local scope（见 `astock-data/src/as_of.rs:224`：只要 `current_as_of()`
    // 是 `Some` 就无条件累加，`take_*` 才需要 scope）⇒ 本 bin 无需包 scope，
    // 直接读全局计数即可量化。
    //
    // 为什么要量化：降级 = 该 vendor 无历史语义（`NoHistoricalSemantic`），在 as-of 下会
    // **静默跳过**。降级率高 ⇒ 回放建立在残缺数据上 ⇒ 由此产出的反思会污染样本集。
    // 判据：若某条的降级增量显著高于同批其它，说明该标的的回放数据基础不可信，需单独复核。
    as_of::reset_global_degradation_log();
    let mut ok = 0usize;
    // 重跑**动作**成功、但**分析状态非 completed**（DAG 失败/超时/取消）的条数。
    // 必须单列：`run_stock_workflow_inner` 失败时也返回 Ok（`core.rs:1645-1660`），
    // 若与 `ok` 混在一起，报告的成功数会虚高，掩盖"重跑没有真产出决策"这一事实。
    let mut not_completed = 0usize;
    let mut not_completed_list: Vec<(String, String, String)> = Vec::new();
    let mut failed: Vec<(String, String, String)> = Vec::new();
    let mut degradations: Vec<(String, String, u64)> = Vec::new();
    for (i, r) in rows.iter().enumerate() {
        let deg_before = as_of::global_degradation_count();
        let seq = i + 1;
        let total = rows.len();
        let as_of = r.analysis_date.clone();
        let ctx = match AsOfContext::parse(&as_of) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("[{seq}/{total}] {} as_of({as_of}) 解析失败: {e}", r.stock_code);
                failed.push((r.stock_code.clone(), as_of, format!("as_of 解析失败: {e}")));
                continue;
            },
        };

        // 全部 clone 成 owned：跨 `await` 的 future 不能借用 `rows` / `state` 的局部切片。
        let code = r.stock_code.clone();
        let parent_id = r.id.clone();
        let as_of_for_call = as_of.clone();
        let state_ref = Arc::clone(&state);
        let started = std::time::Instant::now();
        println!("[{seq}/{total}] 重跑 {} as_of={as_of} ...", r.stock_code);

        // ⚠ 必须用 `with_optional_asof` 而**不是** `AS_OF.scope`：
        // 后者只设 task_local；而 vendor 调用链里有 `tokio::spawn` 出去的子任务，
        // 它们在 task_local 之外，靠**进程级全局** `current_as_of()` 读截止日。
        // 只设 task_local ⇒ 子任务退回 live 模式 ⇒ 用今日数据重算历史时点，
        // 「结论变化可唯一归因于公式」这一前提直接失效（会静默产出看似正常的错数据）。
        // 该坑的权威记载见 `astock-data/src/as_of.rs:258-270`（2026-08-01 实锤修复：
        // 原实现要求调用方显式清理全局，实际 90% 调用方没清 ⇒ 全局永久残留）。
        // `with_optional_asof` 同时设 task_local + 全局，并以 RAII 恢复进入前的全局值。
        let res = as_of::with_optional_asof(Some(ctx), async move {
            run_stock_workflow_inner(
                None, // 无前端监听 ⇒ 不发射 workflow-step-* 事件
                state_ref.as_ref(),
                code,
                None,                 // dry_run：完整执行
                Some(as_of_for_call), // 锚定原时点 ⇒ 结论变化可唯一归因于公式
                Some(parent_id),      // 同日 ⇒ 命中「覆盖」分支（非新建版本）
                None,                 // screening_source
                None,                 // language：默认中文
            )
            .await
        })
        .await;

        let deg_delta = as_of::global_degradation_count().saturating_sub(deg_before);
        degradations.push((r.stock_code.clone(), as_of.clone(), deg_delta));

        match res {
            Ok(v) => {
                let aid = v.get("analysisId").and_then(|x| x.as_str()).unwrap_or("?");
                // ⚠ 先等 DAG 落定，再判成败 —— 理由见 `wait_until_settled` 文档。
                // `run_stock_workflow_inner` 是 fire-and-forget（DAG 被 `tokio::spawn` 出去），
                // 此刻 status 仍是 `running`；不等就退出 = 把 DAG 连同进程一起取消。
                let settled = if aid == "?" {
                    "?".to_string()
                } else {
                    wait_until_settled(&db, aid, std::time::Duration::from_secs(1800)).await
                };
                // 重跑已**原地覆盖**原分析 ⇒ 其既有反思（若有）对应的旧决策已失效。
                // 补建/重置为 pending，使 `--reflect-after` 有素材，且样本与当前模板版本决策
                // 版本一致（否则会得到「新决策 + 旧反思结论」的错配样本）。
                // 失败只登记不中断：重跑本身已成功落库，反思可另跑一轮补。
                let (reflect_ok, reflect_note) = if aid == "?" {
                    (true, "反思跳过（未取到 analysisId）".to_string())
                } else {
                    match ensure_pending_reflection(&db, aid, &as_of).await {
                        // `written == false` ⇒ 分析未成功（DAG 失败也返回 Ok，见函数文档）
                        Ok((written, note)) => (written, note),
                        // 重跑成功、仅反思补建失败 ⇒ 仍算重跑成功，但显式标注出来
                        Err(e) => (true, format!("⚠ 反思补建失败: {e}")),
                    }
                };
                if reflect_ok {
                    ok += 1;
                } else {
                    not_completed += 1;
                    not_completed_list.push((
                        r.stock_code.clone(),
                        as_of.clone(),
                        reflect_note.clone(),
                    ));
                }
                println!(
                    "      {} {:.1}s  analysisId={aid}  status={settled}  as-of 降级 {deg_delta} 次  {reflect_note}",
                    if reflect_ok { "ok " } else { "BAD" },
                    started.elapsed().as_secs_f64()
                );
            },
            Err(e) => {
                eprintln!(
                    "      FAILED {:.1}s (as-of 降级 {deg_delta} 次): {e}",
                    started.elapsed().as_secs_f64()
                );
                failed.push((r.stock_code.clone(), as_of, e));
            },
        }
    }

    println!(
        "\n[batch-rerun] 完成：分析成功 {ok} / 分析未成功 {not_completed} / 执行失败 {} / 共 {}",
        failed.len(),
        rows.len()
    );
    for (code, d, e) in &not_completed_list {
        println!("  分析未成功 {code} {d}: {e}");
    }
    for (code, d, e) in &failed {
        println!("  执行失败 {code} {d}: {e}");
    }

    // ── as-of 降级报告 ──
    // 降级是**证据**不是**判据**：`record_degradation` 只在「该 vendor 无历史语义」时触发，
    // 非零属正常（例如某些实时流接口本就没有历史回放能力）。真正要看的是**分布**：
    // 同批重跑里某几条显著高于其余 ⇒ 这几条的回放建立在明显更残缺的数据上，
    // 其结论与由此产出的反思应单独复核，不要和其余样本混在一起做统计。
    let total_deg: u64 = degradations.iter().map(|(_, _, n)| *n).sum();
    let max_deg = degradations.iter().map(|(_, _, n)| *n).max().unwrap_or(0);
    println!(
        "\n[batch-rerun] as-of 降级合计 {total_deg} 次（单条最高 {max_deg} 次）。\n\
         ⚠ 降级 = vendor 无历史语义被静默跳过 ⇒ 与实盘回放相比数据基础更薄；\n\
         降级数明显偏高的标的，其回放结论应视为低可信。"
    );
    let mut sorted = degradations.clone();
    // 降序：降级次数多的排前（Reverse 包装三元组第三元素）
    sorted.sort_by_key(|(_code, _d, n)| std::cmp::Reverse(*n));
    for (code, d, n) in sorted.iter().take(10) {
        if *n > 0 {
            println!("  降级 {n:>4} 次  {code} as_of={d}");
        }
    }

    // ── 可选：重跑后立即批量反思 ──
    // 为什么放在同一进程内做：重跑后每条新分析的 `hindsight_date`
    // （= 分析锚点 + 期望持有期，见 `core.rs` 的 `[时间旅行模式]` 段）已经落在过去
    // ⇒ **立即可反思**，不必等应用内那个 6 小时兜底循环
    // （`init/services.rs` 的 `start_batch_reflection`）被动消费。
    // 这正是「对齐模板口径」与「快速积累反思数据」两个目标能共用同一次重跑的原因。
    if args.reflect_after {
        // 证据探针：反思**必须**在 live 模式（as-of = None）下跑 —— 它要看的是
        // `analysis_date` **之后**的真实行情，被 as-of 截断就什么都看不到。
        // 这里读到 `Some(…)` 即表示有 as-of 残留穿透了作用域边界。
        // 2026-09-23 实测：残留 2026-07-21 ⇒ 4/4 反思降级为「无行情」。
        println!(
            "[batch-rerun] 反思前生效的 as-of = {:?}（期望 None ⇒ live）",
            as_of::current_as_of().map(|c| c.as_string())
        );
        let count_pending = || async {
            stock_reflections::Entity::find()
                .filter(stock_reflections::Column::Status.eq("pending"))
                .count(&db)
                .await
                .unwrap_or(0)
        };

        let pending_before = count_pending().await;
        println!(
            "\n[batch-rerun] --reflect-after：当前 pending={pending_before}，开始批量反思 ..."
        );
        if pending_before == 0 {
            println!("[batch-rerun] 无 pending 行，跳过反思。");
        } else {
            let client = state.astock_client.clone();
            let engine = state.work_engine.clone();
            let vector_store = state.vector_store.clone();
            let master_key = state.harness.master_key_owned();
            let traj = state.trajectory_storage.clone();

            // 为什么要**循环**而不是跑一轮：
            //   ① `run_batch_reflection_inner` 单轮只处理 `max_count` 条
            //      （默认 20，见 `reflection.rs:1754` 的 `.take(max_count)`），
            //      而本次重跑已为**每条成功分析**补建一行 pending
            //      （见本文件 `ensure_pending_reflection`）
            //      ⇒ 47 条需要至少 3 轮才消化完；
            //      ⚠ 注意：**不是** `run_stock_workflow_inner` 自己产生的 pending ——
            //      业务封装路径不创建 pending（见 `ensure_pending_reflection` 文档的 D12）。
            //   ② 反思可能部分失败（LLM 抖动 / 行情缺失），失败行仍是 pending，
            //      再跑一轮能把本轮可恢复的收掉。
            //
            // 终止判据用**库内 pending 计数**（而非解析返回 JSON 的字段名）——
            // 后者一旦字段改名就会 `unwrap_or(0)` 静默变 0 而导致「第一轮就假装跑完」，
            // 那正是「判据自身撒谎」那一类（本项目有前科）。
            // `after >= before` 也算终止：说明剩下的 pending 本轮不可推进，
            // 再循环只是空转（同时兼作死循环保险）。
            const MAX_ROUNDS: u32 = 20;
            let per_round = args.reflect_max.unwrap_or(20);
            for round in 1..=MAX_ROUNDS {
                let before = count_pending().await;
                if before == 0 {
                    println!("[batch-rerun] pending 已清空，停止反思。");
                    break;
                }
                match axagent_lib::run_batch_reflection_inner(
                    &db,
                    client.as_ref(),
                    &engine,
                    vector_store.as_ref(),
                    &master_key,
                    Some(per_round),
                    Some(&traj),
                    // filter = None ⇒ 全周期档位、且不要求 hindsight 已到期。
                    // 重跑场景下 hindsight 本就已过期，缩不缩口径结果一致，故不必缩小。
                    None,
                )
                .await
                {
                    Ok(v) => {
                        let after = count_pending().await;
                        println!(
                            "[batch-rerun] 反思第 {round} 轮: pending {before} → {after}; {v}"
                        );
                        if after == 0 {
                            println!("[batch-rerun] 反思队列已清空。");
                            break;
                        }
                        if after >= before {
                            println!(
                                "[batch-rerun] pending 未减少（{before} → {after}）\
                                 ⇒ 剩余行本轮不可推进（多为永久失败或需人工介入），停止空转。"
                            );
                            break;
                        }
                    },
                    Err(e) => {
                        eprintln!(
                            "[batch-rerun] 反思第 {round} 轮失败（重跑结果已落库，不受影响）: {e}"
                        );
                        break;
                    },
                }
                if round == MAX_ROUNDS {
                    println!(
                        "[batch-rerun] 达到 {MAX_ROUNDS} 轮上限，停止（可能仍有 pending 残留）。"
                    );
                }
            }
        }
    }

    if !failed.is_empty() {
        return Err(format!("{} 条重跑失败（详见上方清单）", failed.len()));
    }
    Ok(())
}
