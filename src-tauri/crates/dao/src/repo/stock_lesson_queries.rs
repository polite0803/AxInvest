// SPDX-License-Identifier: AGPL-3.0-only
//! 反思教训注入的查询组（3 取 + 2 个「被筛掉多少」计数）+ 主档代理 —— **命令层不得直连的下沉实现**。
//!
//! ## 为什么在这里
//!
//! `commands/stock_workflow/core.rs::fetch_stock_lessons` 与
//! `commands/stock_workflow/hooks.rs::latest_known_horizon` 在组装反思注入上下文时
//! 需要按「周期档（horizon）」过滤 `stock_reflections` / `reflection_lessons` /
//! `stock_analyses`。`sea_orm::Condition::any()` 这类查询组合子一旦写在命令层，
//! 即命中分层门禁 `scripts/check-layer-discipline.mjs` 规则 1
//! （`commands-no-direct-db`：命令层不得出现 `sea_orm::` / `axagent_entities::`
//! 字面量，须经 dao / service）。
//!
//! 把查询搬到这里是**真下沉**（先例见同目录 `stock_analysis_snapshot.rs`）：
//! 实体、列、查询组合子不再出现在命令层，命令层只拿到 `Model` / `Option<String>`。
//!
//! ## 档过滤语义（调用方契约，勿改）
//!
//! `horizon = Some(h)` ⇒ 取「复盘 h 档产出的行」+ `horizon IS NULL` 的行
//! （NULL = 引入该列之前的旧行，复盘档未知，按既有行为继续注入）；
//! `horizon = None` ⇒ 不过滤（兼容旧调用方）。
//!
//! 跨档的行**不进**结果 —— 这是 〇-B v2 第 4 条「同档优先 + 通用规则，跨档不进 prompt」。
//!
//! ## 代过滤语义（§七十一 按版归属 + §五十一-② 起算代际，调用方契约）
//!
//! **规则是下限**：`template_version >= generation_floor` 的行 + `template_version IS NULL`
//! 的行才进 prompt。用下限而不是等号，理由见
//! `axagent_harness::holding_period::HORIZON_BRANCH_GENERATION_FLOOR` 的注释 ——
//! 等号会让每次换代时全部教训整批消失（本仓 v125→v132 只用了两周）。
//!
//! NULL 在此**保留**（代际未知：本列引入前的存量行，或对话直执行通道 —— 那条链不经过
//! `workflow_templates`，NULL 是 A4 裁定的正常形态）：一条来路不明的**文本教训**对模型
//! 仍有参考价值，只要声明它代际未知即可。
//! ⚠ 与统计侧（`reflection_stats`）**刻意不同** —— 那边 NULL 被排除，因为分母不能掺未知代。
//! 同一个 NULL 两处处置不同是有意的：prompt 容得下带标注的未知，分母容不下。
//!
//! 与档过滤不同的是：被筛掉的**旧代**条数要还给调用方（`count_pre_floor_generation_*`），
//! 由注入文本显式声明「另有 N 条早于起算代，未注入」。档可以默认「跨档本就不该要」，
//! 代际不行 —— 一个空结果既可能是「真没教训」也可能是「教训全在起算代之前」，
//! 两者对模型的含义相反，静默合并就是伪造（本仓「结构性缺口不得造成歧义」口径）。
//!
//! ## 依据过滤（2026-10-14 补）
//!
//! 反思行「跑完了」不等于「它有依据」：被复盘的那次分析若没落黑板快照
//! （`stock_analyses.blackboard_snapshot IS NULL`），反思是在**空记忆**上产出的 ——
//! `build_sub_analysis_from_snapshot` 对这种行注入空 map 并 WARN，产出的教训文本看着
//! 像结论，实际没有任何逐节点证据支撑。实证：15 条无快照的分析行已产出 14 条
//! `completed` 反思行，全部落在同股 90 天注入窗口内。
//!
//! 所以注入语料按**被复盘的分析有没有留下依据**筛，不按反思行自己写没写完。
//! 与代过滤同口径：被筛掉的条数一并交还调用方（两个 `fetch_*_completed` 返回元组的
//! 第二项），由注入文本显式声明。静默筛除会把「有教训但依据不可得」伪装成「没有教训」。

use axagent_entities::{
    reflection_lessons, stock_analyses, stock_reflections, strategy_performance,
};

/// 反思行类型重导出（命令层投影 / 分类函数签名走 dao 合规出口）。
pub use stock_reflections::Model as ReflectionModel;

// `count(db)` 由 `PaginatorTrait` 提供（sea-orm 2.x 把它从 QuerySelect 挪走了）⇒ 少这一个
// import 就退化成 `Iterator::count`，编译器报的是「not an iterator」，看着完全不像缺 import。
// 同一族：`select_only()` / `column()` / `columns()` / `into_tuple()` 全挂在 `QuerySelect` 上，
// 漏它报的是「no method named `select_only`」（2026-10-14 实测踩过一次）。
use sea_orm::{
    ActiveModelTrait, ColumnTrait, Condition, DatabaseConnection, EntityTrait, PaginatorTrait,
    QueryFilter, QueryOrder, QuerySelect, Set,
};

/// 「同档 OR 复盘档未知(NULL)」组合子（〇-B v2 第 4 条）。
///
/// `fetch_*` 三函数内的同源逻辑；`reflection.rs` 的 pending 扫描因查询主体各异
/// 无法共用整查询函数，但档过滤子句同源 ⇒ 提成泛型 helper，避免各写一遍
/// `Condition::any()`（命令层写它即分层门禁规则 1 命中）。
pub fn horizon_eq_or_null<C>(col: C, horizon: &str) -> Condition
where
    C: ColumnTrait,
{
    Condition::any().add(col.eq(horizon)).add(col.is_null())
}

/// 「**起算代之后** OR 代际未知(NULL)」组合子（§五十一-②，与 `horizon_eq_or_null` 同族）。
///
/// 不复用成一个通用 `eq_or_null`：两条维度的**比较符不同**（档是等值，代是下限），
/// 合成一个函数最容易被后来者想当然写成等值 —— 那会让每次换代教训整批消失。
/// `gte` 是唯一实现，改动它请连同 `HORIZON_BRANCH_GENERATION_FLOOR` 的注释一起读。
pub fn generation_floor_or_null<C>(col: C, generation_floor: i32) -> Condition
where
    C: ColumnTrait,
{
    Condition::any().add(col.gte(generation_floor)).add(col.is_null())
}

/// 「被复盘的分析留有黑板快照」的分析行 id 集合。
///
/// 只投影 `id`（`into_tuple`）：本表快照单行可达数 MB，为判「有没有」把整列读进内存是纯浪费。
async fn analysis_ids_with_snapshot(
    db: &DatabaseConnection,
) -> Result<std::collections::HashSet<String>, sea_orm::DbErr> {
    Ok(stock_analyses::Entity::find()
        .select_only()
        .column(stock_analyses::Column::Id)
        .filter(stock_analyses::Column::BlackboardSnapshot.is_not_null())
        .into_tuple::<String>()
        .all(db)
        .await?
        .into_iter()
        .collect())
}

/// 按「被复盘的分析有没有留依据」切分反思行 ⇒ `(可注入, 因无依据被筛除的条数)`。
///
/// 纯函数：判据与取数分离，筛除逻辑可在不连库的情况下逐情形验。指向**已删除**分析行的
/// 反思与「行在但快照 NULL」同处置（两者都拿不到逐节点依据），不另立名目。
fn partition_by_analysis_evidence(
    rows: Vec<stock_reflections::Model>,
    with_snapshot: &std::collections::HashSet<String>,
) -> (Vec<stock_reflections::Model>, usize) {
    let (kept, dropped): (Vec<_>, Vec<_>) =
        rows.into_iter().partition(|r| with_snapshot.contains(&r.original_analysis_id));
    (kept, dropped.len())
}

/// 同 ticker 近期已完成反思（按 `CreatedAt` 倒序）。
///
/// 调用方（`fetch_stock_lessons` 的 same_ticker 段）传「90 天前毫秒时间戳」，
/// 取回后自行 `take(3)` —— 截断数是呈现策略，留给调用方。
///
/// 返回元组第二项 = 因**源分析没留黑板快照**而被筛除的条数（见模块文档「依据过滤」）。
/// 筛除发生在截断**之前**，否则被筛掉的行会让 `take(3)` 少拿行而无人知晓。
pub async fn fetch_same_ticker_completed(
    db: &DatabaseConnection,
    stock_code: &str,
    since_ms: i64,
    horizon: Option<&str>,
    // 当前分析所属代际（`None` = 不过滤，语义见模块头「代过滤语义」）
    generation_floor: i32,
) -> Result<(Vec<stock_reflections::Model>, usize), sea_orm::DbErr> {
    let q = stock_reflections::Entity::find()
        .filter(stock_reflections::Column::StockCode.eq(stock_code))
        .filter(stock_reflections::Column::Status.eq("completed")) // 只注入已 resolve 的教训
        .filter(stock_reflections::Column::CreatedAt.gte(since_ms));
    let q = match horizon {
        Some(h) => q.filter(
            Condition::any()
                .add(stock_reflections::Column::Horizon.eq(h))
                .add(stock_reflections::Column::Horizon.is_null()),
        ),
        None => q,
    };
    // §七十一 按版归属：代际同档同理 —— 异代的反思不进 prompt，代际未知的（NULL）保留。
    let q = q.filter(generation_floor_or_null(
        stock_reflections::Column::TemplateVersion,
        generation_floor,
    ));
    let rows = q.order_by_desc(stock_reflections::Column::CreatedAt).all(db).await?;
    let with_snapshot = analysis_ids_with_snapshot(db).await?;
    Ok(partition_by_analysis_evidence(rows, &with_snapshot))
}

/// 全市场近期已完成反思（按 `CreatedAt` 倒序，调用方自行 `take(2)`）。
///
/// `exclude_stock` = 「跨 ticker 注入要排除本股」。**这条排除下推到 dao 而不是留在调用方**：
/// 依据筛除的条数必须与「进 prompt 的行」同一条分母，否则调用方先按股票滤一遍、dao 再按
/// 依据滤一遍，报出来的「另有 N 条未注入」会把同股那些本来就不要的行也算进去 —— 数字偏大
/// 且名目混，正是要避免的「两套计数域并排显示成可比」。
///
/// 返回元组第二项 = 因源分析无黑板快照被筛除的条数（与 `exclude_stock` 同分母）。
pub async fn fetch_recent_completed(
    db: &DatabaseConnection,
    since_ms: i64,
    horizon: Option<&str>,
    // 当前分析所属代际（`None` = 不过滤，语义见模块头「代过滤语义」）
    generation_floor: i32,
    exclude_stock: Option<&str>,
) -> Result<(Vec<stock_reflections::Model>, usize), sea_orm::DbErr> {
    let q = stock_reflections::Entity::find()
        .filter(stock_reflections::Column::CreatedAt.gte(since_ms))
        .filter(stock_reflections::Column::Status.eq("completed")); // 只看已 resolve 的
    let q = match exclude_stock {
        Some(code) => q.filter(stock_reflections::Column::StockCode.ne(code)),
        None => q,
    };
    let q = match horizon {
        Some(h) => q.filter(
            Condition::any()
                .add(stock_reflections::Column::Horizon.eq(h))
                .add(stock_reflections::Column::Horizon.is_null()),
        ),
        None => q,
    };
    // §七十一 按版归属：代际同档同理 —— 异代的反思不进 prompt，代际未知的（NULL）保留。
    let q = q.filter(generation_floor_or_null(
        stock_reflections::Column::TemplateVersion,
        generation_floor,
    ));
    let rows = q.order_by_desc(stock_reflections::Column::CreatedAt).all(db).await?;
    let with_snapshot = analysis_ids_with_snapshot(db).await?;
    Ok(partition_by_analysis_evidence(rows, &with_snapshot))
}

/// 规则化教训（按 `Confidence` 倒序，调用方自行 `take(5)`）。
pub async fn fetch_rule_lessons(
    db: &DatabaseConnection,
    stock_code: &str,
    min_confidence: f64,
    horizon: Option<&str>,
    // 当前分析所属代际（§七十一；`None` = 不过滤）
    generation_floor: i32,
) -> Result<Vec<reflection_lessons::Model>, sea_orm::DbErr> {
    let q = reflection_lessons::Entity::find()
        .filter(reflection_lessons::Column::StockCode.eq(stock_code))
        .filter(reflection_lessons::Column::Confidence.gte(min_confidence)); // 过滤低质量/已废弃规则
    let q = match horizon {
        Some(h) => q.filter(
            Condition::any()
                .add(reflection_lessons::Column::Horizon.eq(h))
                .add(reflection_lessons::Column::Horizon.is_null()),
        ),
        None => q,
    };
    let q = q.filter(generation_floor_or_null(
        reflection_lessons::Column::TemplateVersion,
        generation_floor,
    ));
    q.order_by_desc(reflection_lessons::Column::Confidence).all(db).await
}

/// 该股最近一条带主档（`decision_time_horizon` 非空）的分析的主档。
///
/// 返回 `Ok(None)` 仅表示「没有符合条件的行」；查询失败走 `Err` ——
// 调用方（`latest_known_horizon`）把两种情形都按「不按档过滤」处理并 WARN。
pub async fn latest_analysis_horizon(
    db: &DatabaseConnection,
    stock_code: &str,
) -> Result<Option<String>, sea_orm::DbErr> {
    let row = stock_analyses::Entity::find()
        .filter(stock_analyses::Column::StockCode.eq(stock_code))
        .filter(stock_analyses::Column::DecisionTimeHorizon.is_not_null())
        .order_by_desc(stock_analyses::Column::CreatedAt)
        .one(db)
        .await?;
    Ok(row.and_then(|a| a.decision_time_horizon))
}

/// 被**起算代**筛掉的已完成反思条数（§五十一-② 的声明侧）。
///
/// 只数「早于起算代」的行（`template_version IS NOT NULL AND < floor`）—— NULL 行本就随
/// 一起注入，把它们也计进「未注入」会把实话写成假话。
///
/// `stock_code = Some(c)` 对应 same_ticker 组，`None` 对应跨股组（调用方另有「排除本股」
/// 的呈现层过滤，不在这里 —— 与 `fetch_*` 的分工一致）。
/// 的呈现层过滤，不在这里 —— 与 `fetch_*` 的分工一致）。
pub async fn count_pre_floor_generation_reflections(
    db: &DatabaseConnection,
    stock_code: Option<&str>,
    since_ms: i64,
    horizon: Option<&str>,
    generation_floor: i32,
) -> Result<u64, sea_orm::DbErr> {
    let q = stock_reflections::Entity::find()
        .filter(stock_reflections::Column::Status.eq("completed"))
        .filter(stock_reflections::Column::CreatedAt.gte(since_ms));
    let q = match stock_code {
        Some(c) => q.filter(stock_reflections::Column::StockCode.eq(c)),
        None => q,
    };
    let q = match horizon {
        Some(h) => q.filter(
            Condition::any()
                .add(stock_reflections::Column::Horizon.eq(h))
                .add(stock_reflections::Column::Horizon.is_null()),
        ),
        None => q,
    };
    let foreign = Condition::all()
        .add(stock_reflections::Column::TemplateVersion.is_not_null())
        .add(stock_reflections::Column::TemplateVersion.lt(generation_floor));
    q.filter(foreign).count(db).await
}

/// 被**起算代**筛掉的规则化教训条数（语义同 `count_pre_floor_generation_reflections`）。
pub async fn count_pre_floor_generation_lessons(
    db: &DatabaseConnection,
    stock_code: &str,
    min_confidence: f64,
    horizon: Option<&str>,
    generation_floor: i32,
) -> Result<u64, sea_orm::DbErr> {
    let q = reflection_lessons::Entity::find()
        .filter(reflection_lessons::Column::StockCode.eq(stock_code))
        .filter(reflection_lessons::Column::Confidence.gte(min_confidence));
    let q = match horizon {
        Some(h) => q.filter(
            Condition::any()
                .add(reflection_lessons::Column::Horizon.eq(h))
                .add(reflection_lessons::Column::Horizon.is_null()),
        ),
        None => q,
    };
    let foreign = Condition::all()
        .add(reflection_lessons::Column::TemplateVersion.is_not_null())
        .add(reflection_lessons::Column::TemplateVersion.lt(generation_floor));
    q.filter(foreign).count(db).await
}

/// 按 `original_analysis_id` 批量取回反思行（时间旅行批次四档汇总用）。
///
/// 与教训注入的三个 `fetch_*` 刻意不同：这里**不做**档/代/状态过滤 —— 分档与
/// pending/completed/failed 计数是批次复盘命令层的呈现策略，repo 只做原样取数。
/// 空 id 列表短路返回空 Vec，避免 `is_in([])` 的方言兼容问题。
pub async fn fetch_by_original_analysis_ids(
    db: &DatabaseConnection,
    analysis_ids: &[String],
) -> Result<Vec<stock_reflections::Model>, sea_orm::DbErr> {
    if analysis_ids.is_empty() {
        return Ok(Vec::new());
    }
    stock_reflections::Entity::find()
        .filter(stock_reflections::Column::OriginalAnalysisId.is_in(analysis_ids.to_vec()))
        .all(db)
        .await
}

/// 把「拿不到逐节点依据」的 pending 反思行落成显式终局，返回被处置的行数。
///
/// **为什么在 dao**：命令层写 `update_many` 会命中分层门禁（`commands-no-direct-db`），
/// 而这件事本身就是一次数据自愈 —— 与命令层的 `reclaim_stale_running`（卡死 running 退回
/// pending）同族，由每轮批处理**开头**执行一次，而不是在逐条循环里边判边写。
///
/// **治的是什么**：这类行在旧形态下只被计入 `skippedYoung` 后 continue，状态仍是 pending ⇒
/// 每天 18:00 重新捞到、重新空转，队列永不自愈（现网实证：6 条指向已删除分析的行连续数月
/// `processed=6 / skippedYoung=6 / resolved=0`，到期日早已过去两个月）。把它们冒充成
/// 「太新未到期」是双重失真：既读不出真因，又让分母里一直挂着不会到期的行。
///
/// **两种成因分开写码**（同一格「拿不到依据」，但下一步动作完全不同）：
/// - `ORIGINAL_ANALYSIS_MISSING`：分析行已不存在 ⇒ 永远等不到，孤儿。
/// - `ANALYSIS_SNAPSHOT_MISSING`：分析行在、已出结论，却没落黑板快照 ⇒ 反思会在**空记忆**
///   上跑并产出无依据的教训（实证：15 条无快照分析已产出 14 条 completed 反思行）。
///   这种行**重跑该分析**就能取回依据，与孤儿不可恢复是两件事，所以必须另立码。
/// - 分析行仍在跑（非 `completed`）⇒ **不处置**，快照缺席此时是正常中间态，等它落库。
///
/// 状态串沿用本仓既有的 `failed: <码>: <人话>` 形态 ⇒ 面板与复盘按现成的失败呈现读它，
/// 不新增一套状态值域（新增一个状态值要同时改前端识别逻辑，那是更大的面）。
pub async fn retire_orphan_pending_reflections(
    db: &DatabaseConnection,
    now_ms: i64,
) -> Result<usize, sea_orm::DbErr> {
    use sea_orm::sea_query::Expr;

    let pending = stock_reflections::Entity::find()
        .filter(stock_reflections::Column::Status.eq("pending"))
        .all(db)
        .await?;
    if pending.is_empty() {
        return Ok(0);
    }
    // 一次取回 pending 行引用的分析行 (id, status)，避免逐条 find_by_id 的 N+1。
    // 只投影这两列：快照单行可达数 MB，判「有没有」不需要把它读进内存。
    let analysis_ids: Vec<String> =
        pending.iter().map(|r| r.original_analysis_id.clone()).collect();
    let analysis_status: std::collections::HashMap<String, String> = stock_analyses::Entity::find()
        .select_only()
        .columns([stock_analyses::Column::Id, stock_analyses::Column::Status])
        .filter(stock_analyses::Column::Id.is_in(analysis_ids))
        .into_tuple::<(String, String)>()
        .all(db)
        .await?
        .into_iter()
        .collect();
    let with_evidence = analysis_ids_with_snapshot(db).await?;

    let mut retired = 0usize;
    for r in pending {
        let reason = match analysis_status.get(&r.original_analysis_id) {
            None => Some(format!(
                "failed: ORIGINAL_ANALYSIS_MISSING: 原始分析 {} 已不存在，反思行已成孤儿",
                r.original_analysis_id
            )),
            Some(st) if st == "completed" && !with_evidence.contains(&r.original_analysis_id) => {
                Some(format!(
                    "failed: ANALYSIS_SNAPSHOT_MISSING: 原始分析 {} 已出结论但未落黑板快照，\
                     逐节点依据不可得，反思不予执行（重跑该分析可取回依据）",
                    r.original_analysis_id
                ))
            },
            Some(_) => None,
        };
        let Some(status_text) = reason else {
            continue;
        };
        stock_reflections::Entity::update_many()
            .col_expr(stock_reflections::Column::Status, Expr::value(status_text))
            .col_expr(stock_reflections::Column::UpdatedAt, Expr::value(now_ms))
            .filter(stock_reflections::Column::Id.eq(r.id))
            .exec(db)
            .await?;
        retired += 1;
    }
    Ok(retired)
}

/// 失败反思行是否属于「可安全重试」的一类（纯判定，便于单测）。
///
/// 判据：
/// - 必须是 `failed:` 开头的终局行；
/// - 两类结构性终局**不可**重试：
///   * `ORIGINAL_ANALYSIS_MISSING` —— 原始分析已删，重试永远拿不到依据；
///   * `ANALYSIS_SNAPSHOT_MISSING` —— 须先重跑原分析补黑板快照，直接重试只会再失败一次。
///
/// 其余失败（反思工作流执行错误、LLM 节点失败、上下文超限等）均可原地重试：
/// 三层上下文预算修复（2026-10-14，提交 c247e03e2）之后，旧的上下文超限行重试
/// 不会再撞同一错误（现网实证：4 条 72 万 token 级超限行因此永久挂死）。
pub fn is_retryable_failed_reflection(status: &str) -> bool {
    let s = status.trim();
    if !s.starts_with("failed:") {
        return false;
    }
    // 两个码都含 `_MISSING`，但保留全名匹配，避免未来新增含 MISSING 的可重试码被误杀。
    let terminal =
        s.contains("ORIGINAL_ANALYSIS_MISSING") || s.contains("ANALYSIS_SNAPSHOT_MISSING");
    !terminal
}

/// `requeue_failed_reflections` 的结果分桶。
#[derive(Default, Debug, PartialEq, Eq)]
pub struct RequeueFailedOutcome {
    /// 已置回 pending、下一轮可被批量反思消费的行 id（原地 UPDATE，不造新行）。
    pub requeued: Vec<String>,
    /// 结构性终局（原始分析缺失 / 快照缺失）未重置，需先修复上游，不能静默吞掉。
    pub skipped_terminal: Vec<String>,
}

/// 把可重试的 failed 反思行**原地**重置为 pending（P0-8 死信重试）。
///
/// ## 为什么原地 UPDATE 而非新建
///
/// 表上 `(original_analysis_id, horizon)` 无唯一约束（见同表索引声明），
/// 新建行会造出同档重复反思；原地重置同一行后，下一轮 `run_batch_reflection_inner`
/// 自动捞起（这些行的 hindsight_date 是历史日期，due_only 立即命中）。
/// 清空结论列的集合与重放 bin（`src/bin/axagent-batch-rerun.rs` 的重置段）保持一致；
/// `raw_return` / `alpha_return` / `holding_days` / `benchmark_name` 故意不清 ——
/// 它们是分析日到评估时点的客观行情事实，与决策内容无关。
///
/// ## 参数
///
/// - `ids`：None 扫描全部 failed 行；Some 时仅处理给定 id（手动勾选重试）。
/// - `limit`：单轮重置上限。表上暂无 attempts 列，某条持续失败时不能让它每轮都被
///   重置并空烧 LLM 配额，由调用方控制节奏；超量的可重试行留在 failed 留待下一轮。
pub async fn requeue_failed_reflections(
    db: &DatabaseConnection,
    now_ms: i64,
    ids: Option<&[String]>,
    limit: u64,
) -> Result<RequeueFailedOutcome, sea_orm::DbErr> {
    use sea_orm::ActiveModelTrait;
    use sea_orm::Set;

    let mut query = stock_reflections::Entity::find()
        .filter(stock_reflections::Column::Status.starts_with("failed:"));
    if let Some(ids) = ids {
        if ids.is_empty() {
            return Ok(RequeueFailedOutcome::default());
        }
        query = query.filter(stock_reflections::Column::Id.is_in(ids.to_vec()));
    }
    // 最早失败的先重试：它们积压最久，且大概率是已被后续修复覆盖的历史性失败。
    let failed_rows = query.order_by_asc(stock_reflections::Column::UpdatedAt).all(db).await?;

    let mut outcome = RequeueFailedOutcome::default();
    for row in failed_rows {
        if !is_retryable_failed_reflection(&row.status) {
            outcome.skipped_terminal.push(row.id);
            continue;
        }
        if outcome.requeued.len() as u64 >= limit {
            // 达到本轮上限即停：剩余可重试行原样保留，不计入 skipped（它们不是结构性终局）。
            break;
        }
        let id = row.id.clone();
        let mut am: stock_reflections::ActiveModel = row.into();
        am.status = Set("pending".to_string());
        am.verdict = Set(None);
        am.alpha_cited = Set(None);
        am.lesson_summary = Set(None);
        am.what_went_wrong = Set(None);
        am.missed_signals = Set(None);
        am.fix_for_future = Set(None);
        // 归因层是上一轮反思的结论性盖章，重试时旧结论失效，随三问一并清空。
        am.miss_layer = Set(None);
        am.parameter_suggestions_json = Set(None);
        am.decision_json = Set(None);
        am.blackboard_snapshot = Set(None);
        am.actual_outcome = Set(String::new());
        am.updated_at = Set(now_ms);
        am.update(db).await?;
        outcome.requeued.push(id);
    }
    Ok(outcome)
}

/// 取某时间戳之后新增的全部 completed 反思行（进化计划聚合的样本来源）。
///
/// 与 [`fetch_recent_completed`] 刻意不同：这里不做档/代过滤，也不按票过滤 ——
/// 聚合分母是「窗口内所有已到期反思」，数据可达性分桶由命令层按行字段判定。
///
/// 状态串必须是 `"completed"`：反思状态机 pending → running → completed / failed。
/// （历史坑：P1 曾误写 `"resolved"` —— 那是 demand_discovery 表的状态值，
/// 此查询永远为 0；不要改成同义串。）
pub async fn fetch_completed_reflections_since(
    db: &DatabaseConnection,
    since_ms: i64,
) -> Result<Vec<stock_reflections::Model>, sea_orm::DbErr> {
    stock_reflections::Entity::find()
        .filter(stock_reflections::Column::Status.eq("completed"))
        .filter(stock_reflections::Column::CreatedAt.gte(since_ms))
        .all(db)
        .await
}

/// 按补分析行 id 取逐档反思行，按 `horizon` 升序（漏检事件复盘展开行）。
///
/// 不做状态过滤 —— pending/failed 行也要照实投影，前端按行状态分态展示。
pub async fn fetch_by_analysis_id(
    db: &DatabaseConnection,
    analysis_id: &str,
) -> Result<Vec<stock_reflections::Model>, sea_orm::DbErr> {
    stock_reflections::Entity::find()
        .filter(stock_reflections::Column::OriginalAnalysisId.eq(analysis_id.to_owned()))
        .order_by_asc(stock_reflections::Column::Horizon)
        .all(db)
        .await
}

/// 反思行 → 前端 JSON 投影（单一来源）。
///
/// `list_reflections` 与 P3 妖股漏检回放的 `get_mover_recall_event_review`
/// 共用这一份投影：字段增改只许动这里，禁止在第二处再抄一遍
/// （AGENTS 禁区 12：同一 DTO 两个投影面必然漂移）。
pub fn reflection_review_json(r: &stock_reflections::Model) -> serde_json::Value {
    let horizon_results = r
        .horizon_results_json
        .as_deref()
        .and_then(|s| serde_json::from_str::<serde_json::Value>(s).ok());
    serde_json::json!({
        "id": r.id,
        "stockCode": r.stock_code,
        "stockName": r.stock_name,
        "originalAnalysisId": r.original_analysis_id,
        "asOfDate": r.as_of_date,
        "hindsightDate": r.hindsight_date,
        "minConfidenceThreshold": r.min_confidence_threshold,
        "reflectionDepth": r.reflection_depth,
        // 〇-B v2 第 4 条：本次反思**复盘的档位**（NULL = 本列引入前，档未知）
        "horizon": r.horizon,
        "actualOutcome": r.actual_outcome,
        "whatWentWrong": r.what_went_wrong,
        "missedSignals": r.missed_signals,
        "fixForFuture": r.fix_for_future,
        // P3：漏检回放面板展开用 —— 裁决/教训摘要/原始收益/来源桶/权威归因层。
        // source 读侧口径：`mover_recall` 一桶，其余（含老库空串）皆常规 due。
        "verdict": r.verdict,
        "lessonSummary": r.lesson_summary,
        "rawReturn": r.raw_return,
        "source": r.source,
        "missLayer": r.miss_layer,
        // P2-b：本行的失败模式标注（JSON 数组文本，值域见 analysis_engine::failure_mode）。
        // NULL = 模板未产出该字段，与「产出但全被白名单丢弃」的空数组是两件事。
        "failureModes": r.failure_modes,
        "decisionJson": r.decision_json,
        "blackboardSnapshot": r.blackboard_snapshot,
        "status": r.status,
        "createdAt": r.created_at,
        "alphaReturn": r.alpha_return,
        // #10 P7 妖股标签（按档挂在逐档反思行上）。四态语义见
        // `entities/src/stock_reflections.rs` 的列文档：达标/未达标/窗口未满/判据不可用，
        // NULL=未复盘或本列引入前的存量行 ⇒ 面板必须分得开「没达标」与「没判定」。
        "moverLabel": r.mover_label,
        "horizonResults": horizon_results,
    })
}

/// 腿预测力报告的配对取数（P2-a 裁决层的唯一 IO 面）。
///
/// 只取「completed 且带四周期结果、且原始分析行带决策 JSON」的行，返回
/// `(分析 decision_json, 反思 horizon_results_json)` 配对。缺任一一侧就不出：
/// 没有后验收益的信号不是样本，硬配会造出伪观测（装配层同样拒收，这里连 IO 面也不给）。
///
/// 下沉到 dao 的理由同文件头：`commands-no-direct-db` 棘轮（命令层不得出现实体/ORM 字面量）。
pub async fn leg_ic_pairs(
    db: &DatabaseConnection,
) -> Result<Vec<(String, String)>, sea_orm::DbErr> {
    let rows = stock_reflections::Entity::find()
        .filter(stock_reflections::Column::Status.starts_with("completed"))
        .filter(stock_reflections::Column::HorizonResultsJson.is_not_null())
        .all(db)
        .await?;
    if rows.is_empty() {
        return Ok(Vec::new());
    }
    let ids: Vec<&str> = rows.iter().map(|r| r.original_analysis_id.as_str()).collect();
    let anas = stock_analyses::Entity::find()
        .filter(stock_analyses::Column::Id.is_in(ids))
        .all(db)
        .await?;
    let by_id: std::collections::HashMap<&str, &stock_analyses::Model> =
        anas.iter().map(|a| (a.id.as_str(), a)).collect();
    Ok(rows
        .iter()
        .filter_map(|r| {
            let results = r.horizon_results_json.as_deref()?;
            if results.is_empty() {
                return None;
            }
            let dec = by_id.get(r.original_analysis_id.as_str())?.decision_json.as_deref()?;
            if dec.is_empty() {
                return None;
            }
            Some((dec.to_string(), results.to_string()))
        })
        .collect())
}

/// M1 确定性样本的落库草稿（P1-1② 写侧下沉）。字段语义见 `strategy_performance` 实体的列文档。
///
/// 判定（`was_correct`）由命令层的纯函数算好再传 —— dao 不碰业务判据，只负责「查在不在 / 写一行」。
pub struct ReflectionM1Draft {
    pub id: String,
    pub reflection_id: String,
    pub stock_code: String,
    pub stock_name: String,
    pub period: String,
    pub decision_at: i64,
    pub exit_at: i64,
    pub holding_days: i32,
    pub return_pct: f64,
    /// `was_correct` 列是 i32（0/1），不是 bool
    pub was_correct: i32,
    pub decision_confidence: i32,
    pub template_version: Option<i32>,
    pub horizon_pnl_json: Option<String>,
    /// B3：该行所属**主档**那一对的五维归一分（产端 = 被复盘分析的
    /// `decision_json.agreementBreakdown.total`，判据 = `decision.rs::compute_decision_agreement`）。
    ///
    /// 为什么由本通道写：全仓只有这里同时拿得到「被复盘决策的 decision_json」与「到期判定」，
    /// 而 `evolution_drift::record_performance` 那条唯一调用方（分析回测）入参只有
    /// `analysis_date`，没有分析行 id 也没有 LLM 侧 ⇒ 它给不出这一对，只能继续传 None。
    /// `None` = 该分析没有一致性诊断（LLM 侧缺席 / 该记录产自诊断上线前）⇒ 面板按缺席处理，不补 0。
    pub agreement_score: Option<i32>,
}

/// 该反思行是否已经落过 M1 样本（`strategy_id` 固定 `reflection_verdict`）。幂等查重用。
pub async fn has_reflection_m1_sample(
    db: &DatabaseConnection,
    reflection_id: &str,
) -> Result<bool, sea_orm::DbErr> {
    let n = strategy_performance::Entity::find()
        .filter(strategy_performance::Column::ReflectionId.eq(reflection_id))
        .filter(strategy_performance::Column::StrategyId.eq("reflection_verdict"))
        .count(db)
        .await?;
    Ok(n > 0)
}

/// 写入一条 M1 样本。`created_at` 与 `exit_at` 同源（= 本次判定时刻）。
pub async fn insert_reflection_m1_sample(
    db: &DatabaseConnection,
    d: ReflectionM1Draft,
) -> Result<(), sea_orm::DbErr> {
    let row = strategy_performance::ActiveModel {
        id: Set(d.id),
        strategy_id: Set("reflection_verdict".to_string()),
        period: Set(d.period),
        stock_code: Set(d.stock_code),
        stock_name: Set(d.stock_name),
        decision_at: Set(d.decision_at),
        exit_at: Set(d.exit_at),
        holding_days: Set(d.holding_days),
        return_pct: Set(d.return_pct),
        was_correct: Set(d.was_correct),
        decision_confidence: Set(d.decision_confidence),
        template_version: Set(d.template_version),
        horizon_pnl_json: Set(d.horizon_pnl_json),
        agreement_score: Set(d.agreement_score),
        // B3 逐档三键对本格一律 None：这条是「反思裁决」旧格（`strategy_id='reflection_verdict'`，
        // 不去重、每次重跑追加），既不是逐档物化行、也没有该档的三维分与 LLM 动作 ——
        // 拿 `was_correct` 反推 LLM 侧会把「公式判对」当成「LLM 判错」。
        horizon_agreement: Set(None),
        llm_action: Set(None),
        llm_was_correct: Set(None),
        source_ref: Set(None),
        // §二 显式外键：本行由**这条反思**产出（统计侧直连依据）。
        reflection_id: Set(Some(d.reflection_id)),
        created_at: Set(d.exit_at),
    };
    row.insert(db).await.map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    fn row(id: &str, analysis_id: &str) -> stock_reflections::Model {
        stock_reflections::Model {
            id: id.to_string(),
            stock_code: "000001".to_string(),
            stock_name: "测试".to_string(),
            original_analysis_id: analysis_id.to_string(),
            as_of_date: "2026-10-14".to_string(),
            hindsight_date: "2026-11-11".to_string(),
            min_confidence_threshold: 60,
            reflection_depth: "light".to_string(),
            horizon: None,
            actual_outcome: String::new(),
            raw_return: None,
            alpha_return: None,
            holding_days: None,
            mover_label: None,
            benchmark_name: None,
            verdict: None,
            alpha_cited: None,
            lesson_summary: Some("测试教训".to_string()),
            what_went_wrong: None,
            missed_signals: None,
            fix_for_future: None,
            parameter_suggestions_json: None,
            horizon_results_json: None,
            decision_json: None,
            blackboard_snapshot: None,
            template_version: None,
            source: "due".to_string(),
            miss_layer: None,
            failure_modes: None,
            status: "completed".to_string(),
            created_at: 0,
            updated_at: 0,
        }
    }

    fn ids(v: &[&str]) -> HashSet<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    fn kept_ids(kept: &[stock_reflections::Model]) -> Vec<&str> {
        kept.iter().map(|r| r.id.as_str()).collect()
    }

    #[test]
    fn drops_rows_whose_analysis_left_no_snapshot_and_counts_them() {
        let rows = vec![row("r1", "a-has-snap"), row("r2", "a-null-snap"), row("r3", "a-deleted")];
        let (kept, dropped) =
            partition_by_analysis_evidence(rows, &ids(&["a-has-snap", "unrelated"]));
        assert_eq!(kept_ids(&kept), vec!["r1"], "只有源分析留有快照的那行进语料");
        assert_eq!(
            dropped, 2,
            "「快照为 NULL」与「分析行已删」两族都算依据不可得，必须一起计数而非静默消失"
        );
    }

    #[test]
    fn keeps_relative_order_so_truncation_still_takes_the_newest() {
        // 调用方按 CreatedAt 倒序取回后再 `take(3)` ⇒ 筛除不得改变相对顺序，
        // 否则「取最新 3 条」会退化成「取任意 3 条」。
        let rows = vec![row("newest", "a1"), row("mid", "a-gone"), row("oldest", "a2")];
        let (kept, dropped) = partition_by_analysis_evidence(rows, &ids(&["a1", "a2"]));
        assert_eq!(kept_ids(&kept), vec!["newest", "oldest"]);
        assert_eq!(dropped, 1);
    }

    #[test]
    fn all_dropped_is_distinguishable_from_no_rows_at_all() {
        // 负控：若实现把「全被筛除」写成空输入即返回 (空, 0)，注入文本就会谎称
        // 「这段历史没有教训」—— 那正是本模块文档反对的「把结构性缺口读成空集」。
        let (kept, dropped) = partition_by_analysis_evidence(vec![row("r1", "a-gone")], &ids(&[]));
        assert!(kept.is_empty());
        assert_eq!(dropped, 1, "空结果 + 1 条被筛除，必须与「本来 0 行」可区分");
        let (kept0, dropped0) = partition_by_analysis_evidence(Vec::new(), &ids(&["a1"]));
        assert!(kept0.is_empty());
        assert_eq!(dropped0, 0, "真的没有行 ⇒ 计数为 0，注入文本不写这一句");
    }

    #[test]
    fn retryable_partitions_workflow_failures_from_structural_terminal_states() {
        // P0-8：可重试族 —— 工作流/LLM 执行类失败（含三层上下文预算修复前的超限行）。
        assert!(is_retryable_failed_reflection(
            "failed: reflection-agent: this model's context length is 727019 tokens"
        ));
        assert!(is_retryable_failed_reflection("failed: 反思工作流失败: connection reset by peer"));
        // 结构性终局 —— 必须先修上游，直接重试只会再失败一次。
        assert!(!is_retryable_failed_reflection(
            "failed: ORIGINAL_ANALYSIS_MISSING: 原始分析 a1 已不存在，反思行已成孤儿"
        ));
        assert!(!is_retryable_failed_reflection(
            "failed: ANALYSIS_SNAPSHOT_MISSING: 原始分析 a2 已出结论但未落黑板快照"
        ));
        // 非失败状态一律不参与重试。
        assert!(!is_retryable_failed_reflection("pending"));
        assert!(!is_retryable_failed_reflection("running"));
        assert!(!is_retryable_failed_reflection("completed"));
        // 容错：首尾空白不应改变判定。
        assert!(is_retryable_failed_reflection("  failed: reflection-agent: node timeout  "));
    }
}
