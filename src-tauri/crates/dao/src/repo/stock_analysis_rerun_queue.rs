// SPDX-License-Identifier: AGPL-3.0-only
//! `stock_analyses` 的**中断重跑取件口** —— 持久化边界的唯一实现。
//!
//! ## 为什么在 dao 而不是命令里直接查
//!
//! `commands/stock_analysis_rerun.rs` 需要三样东西：按状态计数、按状态取一批待跑行、
//! 轮询某条行的 `status`。这三样最初把 `axagent_entities::stock_analyses::Entity::…`
//! 与 `sea_orm` 直接写在命令层 ⇒ 命中分层门禁
//! `scripts/check-layer-discipline.mjs` 规则 1（`commands-no-direct-db`：命令层不得直连
//! sea_orm / axagent_entities，须经 dao）。
//!
//! 与 `stock_analysis_snapshot.rs` 同因同解：实体与 `sea_query` 表达式留在 dao，
//! 命令层只拿到本模块的窄 DTO（**不给 `stock_analyses::Model`** —— 那等于把实体类型
//! 重新漏回命令层，下一个调用方就会顺手去读别的列）。
//!
//! ## 取件管线的三道判据（缺一不可）
//!
//! 计数（[`count_analyses_by_status`]）与取件（[`load_rerun_candidates`]）共用
//! [`pending_rows`] 同一实现 ⇒「剩余 N 条」与「本轮实际取到 N 条」永远同集：
//!
//! 1. **版本链结算**（[`resolved_source_ids`]）：来源行已有落定的子版本 ⇒ 排除；
//! 2. **键结算**（[`resolved_replay_keys`]）：同 `(stock_code, as_of)` 已有落定的
//!    回放行 ⇒ 排除；
//! 3. **同键去重**：同一键仍有多条待跑行时只取排序最早的一条 —— 回放覆盖分支的
//!    原地 UPDATE 目标按 `(stock_code, as_of, kind='replay')` 定位，同键多跑一次
//!    就是多覆盖一次。
//!
//! ## 为什么排序是本模块的契约而非调用方的偏好
//!
//! 取件顺序必须**确定性**：同一批在两次运行里顺序一致，日志才可比对。
//! 升序 = 先补最老的 —— 持续产生的新中断若排在前面，老记录会被永远挤在队尾。
//! 排序键为 `(analysis_date, stock_code, created_at, id)`：后两项是「同键去重保留
//! 哪一条」的裁决依据 —— 全键有序（`id` 唯一）⇒ 去重结果不随 DB 在无 ORDER BY
//! 时的返回顺序漂移。

use std::collections::HashSet;

use axagent_entities::stock_analyses;
use axagent_harness::AnalysisKind;
use sea_orm::{
    ColumnTrait, DatabaseConnection, EntityTrait, FromQueryResult, QueryFilter, QuerySelect,
};

/// 取件口只需要这九列 —— **绝不能拉整行**。
///
/// ## 为什么这不是「顺手优化」（2026-10-18 实测）
///
/// `stock_analyses` 是宽表，两列大 JSON 是行膨胀的全部来源：
/// 全表 226 行里 `blackboard_snapshot` 合计 **504 MB**（单行最大 7.4 MB、均值 3.0 MB）。
/// 而本模块的三道判据原本用 `Entity::find().all(db)` 取整行（即 `SELECT *`），
/// 命中集合是：
///
/// | 判据 | 行数 | 大列字节 |
/// |---|---|---|
/// | `parent_analysis_id IS NOT NULL` | 54 | **80 MB** |
/// | `analysis_kind = 'replay'` | 40 | **49 MB** |
/// | `status IN (可重试集合)` | 35 | 0 B（占位行） |
///
/// 也就是说**一次 `pending_rows` 就要解压 + 反序列化 129 MB TOAST**，而
/// `run_rerun` 一轮里要跑 2 次（计数 + 取件）、3 轮 + 首尾各 1 次 ≈ **7 次 ≈ 900 MB**，
/// 全部发生在 tokio worker 上。实测启动钩子那条「无待补跑」也要 582 ms
/// （其中 10 s 是钩子自身的 sleep，净耗时≈0.6 s），且它**每次启动都跑**。
///
/// 判据本身只读得到这七列 ⇒ 多拉一列大列就是纯浪费，且随历史积累线性恶化。
///
/// 七列 = `id`（版本链排除 + 去重键）、`stock_code` / `analysis_date` / `as_of_date`
/// （排序与 as-of 锚）、`created_at`（排序）、`template_id`（透传给重跑）。
/// `status` **不在此列**：状态过滤全部下推到 SQL（`filter`），取回后再判一次等于
/// 把同一判据做两遍，且白拉一列。
#[derive(Debug, FromQueryResult)]
struct PendingRowLite {
    id: String,
    stock_code: String,
    analysis_date: String,
    as_of_date: Option<String>,
    template_id: Option<String>,
    created_at: i64,
}

/// 版本链结算只需要 `(status, parent_analysis_id)` 两列（`status` 用于判「子版本是否已落定」）。
#[derive(Debug, FromQueryResult)]
struct ChildLite {
    status: String,
    parent_analysis_id: Option<String>,
}

/// 键结算只需要 `(stock_code, as_of_date)` —— `status` 与 `analysis_date` 都在 SQL 侧过滤掉了。
#[derive(Debug, FromQueryResult)]
struct ReplayLite {
    stock_code: String,
    as_of_date: Option<String>,
}

/// 一条待重跑的分析 —— 补跑所需的**全部**字段，刻意只有这四个。
///
/// 刻意不暴露 `stock_analyses::Model`：命令层拿到实体就能顺手去读别的列，
/// 而补跑只该认「哪一行、哪个标的、哪天 as-of、原模板」。多给一个字段就多一条
/// 未声明的依赖，而依赖关系变化时本 DTO 不会自动跟着变。
#[derive(Debug, Clone)]
pub struct RerunCandidate {
    /// `stock_analyses.id` —— replay 同日覆盖分支的原地 UPDATE 目标。
    pub id: String,
    /// `stock_code` —— 重跑标的。
    pub stock_code: String,
    /// **as-of 锚点**（不是执行日）：`as_of_date` 为主，空则回退 `analysis_date`。
    ///
    /// 回放行的 `analysis_date` 记的是**执行日**（某次补跑发生的那天），而它复现的
    /// 行情日是 `as_of_date`。按执行日发起重跑会以错日期解析覆盖目标 ⇒ 新建一条
    /// 无关记录，而不是命中同日覆盖分支（2026-10-17 实测：`d3a74ec8` 的
    /// `analysis_date=10-17`、`as_of_date=10-14`）。
    pub as_of: String,
    /// 原行模板（可能是快速链 `stock-analysis-fast`）。
    ///
    /// 必须原样带出：强行换成完整链会把「一条快速链记录」重跑成完整链结论 ——
    /// 那是语义变更，不是补跑。`None` = 交由工作流走缺省解析。
    pub template_id: Option<String>,
}

/// 行内 as-of 锚的解析（与 [`RerunCandidate::as_of`] 同判据）：`as_of_date` 空则回退
/// `analysis_date`。键结算与候选投影必须共用本函数 —— 否则「按哪个日期结账」与
/// 「按哪个日期发起」会出现两套口径。
fn as_of_of(m: &PendingRowLite) -> String {
    pick_as_of(m.as_of_date.as_ref(), &m.analysis_date)
}

fn pick_as_of(as_of_date: Option<&String>, analysis_date: &str) -> String {
    as_of_date.cloned().unwrap_or_else(|| analysis_date.to_string())
}

/// 「这一行已经补出结果了，别再取它」的来源行 id 集合。
///
/// ## 为什么这是取件口的契约，而不是调用方的补丁
///
/// `rerun_one` 走 as-of 回放，而回放**不改来源行**：同日无回放行时新建一条回放版本
/// （`stock_workflow/core.rs:591`），有同日回放行时覆盖的是**那一条** target 行。
/// 两种情形下来源行的 `status` 都停在 `interrupted` ⇒ 调用方「按状态数剩余」这条
/// 收敛判据永远归不了零，`max_rounds` 的每一轮都会把同一批行重新发起一遍。
/// 实证（2026-10-17 09:07）：库里 35 条中断行 ⇒ 3 轮 = **105 次**工作流执行，
/// LLM 提供商全线 429、0 条产出结论。
///
/// ## 判据形态
///
/// 排除条件 = 存在**任一**子版本（`parent_analysis_id` 指向本行）其 `status` **不在**
/// 本轮的可重试状态集内。刻意不只认 `completed`：子版本还在 `running` 时也必须排除，
/// 否则第二轮会在同一标的上并行起第二条 DAG（正是上面那次 429 的形态）。
///
/// ⚠ 计数与取件必须共用本函数，否则「剩余 N 条」与「本轮实际取到 N 条」不同集 ⇒
/// 报告数字与行为互相矛盾，且下一轮又开始一批算不进总数的行。
async fn resolved_source_ids(
    db: &DatabaseConnection,
    statuses: &[&str],
) -> Result<Vec<String>, sea_orm::DbErr> {
    // 只取两列：整行会把 80 MB 的 `blackboard_snapshot` 一并解压出来（见 PendingRowLite 头注）
    let children = stock_analyses::Entity::find()
        .select_only()
        .column(stock_analyses::Column::Status)
        .column(stock_analyses::Column::ParentAnalysisId)
        .filter(stock_analyses::Column::ParentAnalysisId.is_not_null())
        .into_model::<ChildLite>()
        .all(db)
        .await?;
    Ok(children
        .into_iter()
        .filter(|c| !statuses.contains(&c.status.as_str()))
        .filter_map(|c| c.parent_analysis_id)
        .collect())
}

/// 「这个 `(stock_code, as_of)` 的账已经结了」的键集合 —— 落定的回放行即结账。
///
/// ## 为什么版本链排除（[`resolved_source_ids`]）还不够
///
/// 回放行只挂**一条**来源行的 parent：同一 `(stock_code, as_of)` 若有多条来源行
/// （2026-10-17 实况：603466@2026-10-14 有 `26fd5746` 与 `6dc93b6b` 两条 interrupted，
/// 回放行 `d3a74ec8` 只挂前者），parent 规则只能让挂着的行离队；另一条没有任何子行
/// ⇒ 永远留在队列里（实测收敛判据 remain=1 的正是它），每一批都拿它重新发起、
/// 对同一回放目标原地覆盖一次。
///
/// ## 判据形态（与 [`resolved_source_ids`] 同构）
///
/// 键 = `(stock_code, as_of_date)`，来源 = `analysis_kind='replay'` 且 `as_of_date`
/// 非空、且 `status` **不在**本轮可重试集合内的行（与 `stock_workflow/core.rs` 覆盖
/// 目标的判据同口径：回放行只以 `kind='replay'` 诞生）。回放行自身还在可重试状态时
/// **不算结账** —— 它正是本轮要跑的目标；这与 parent 规则的对称性也一致（第一轮里
/// failed 的回放算结账，第二轮把它纳回重试集合后就不算了）。
async fn resolved_replay_keys(
    db: &DatabaseConnection,
    statuses: &[&str],
) -> Result<HashSet<(String, String)>, sea_orm::DbErr> {
    // 只取两列：这 40 行回放行合计 49 MB 大列，整行取回纯属浪费（见 PendingRowLite 头注）
    let rows = stock_analyses::Entity::find()
        .select_only()
        .column(stock_analyses::Column::StockCode)
        .column(stock_analyses::Column::AsOfDate)
        .filter(stock_analyses::Column::AnalysisKind.eq(AnalysisKind::Replay.as_storage()))
        .filter(stock_analyses::Column::AsOfDate.is_not_null())
        .filter(stock_analyses::Column::Status.is_not_in(statuses.to_vec()))
        .into_model::<ReplayLite>()
        .all(db)
        .await?;
    Ok(rows
        .into_iter()
        .filter_map(|m| {
            let as_of = m.as_of_date?;
            Some((m.stock_code, as_of))
        })
        .collect())
}

/// 待重跑行的**唯一取数实现**：三道判据（版本链结算、键结算、同键去重）+ 确定性排序。
///
/// 计数与取件都经过这里（见模块文档）⇒ 两者不可能走出不同集。
async fn pending_rows(
    db: &DatabaseConnection,
    statuses: &[&str],
) -> Result<Vec<PendingRowLite>, sea_orm::DbErr> {
    let resolved = resolved_source_ids(db, statuses).await?;
    let resolved_keys = resolved_replay_keys(db, statuses).await?;

    // 只投影候选集必需的列（**不是** `SELECT *`）：候选集本身就是大列的宿主行
    // —— 这里每多拉一列，一次计数就多解压几 MB TOAST。
    let mut rows = stock_analyses::Entity::find()
        .select_only()
        .column(stock_analyses::Column::Id)
        .column(stock_analyses::Column::StockCode)
        .column(stock_analyses::Column::AnalysisDate)
        .column(stock_analyses::Column::AsOfDate)
        .column(stock_analyses::Column::TemplateId)
        .column(stock_analyses::Column::CreatedAt)
        .filter(stock_analyses::Column::Status.is_in(statuses.to_vec()))
        .into_model::<PendingRowLite>()
        .all(db)
        .await?;

    if !resolved.is_empty() {
        let resolved: HashSet<&str> = resolved.iter().map(String::as_str).collect();
        rows.retain(|m| !resolved.contains(m.id.as_str()));
    }
    if !resolved_keys.is_empty() {
        rows.retain(|m| !resolved_keys.contains(&(m.stock_code.clone(), as_of_of(m))));
    }

    // 排序先于去重：同键「保留哪一条」必须可复现（`id` 唯一，兜底成全序）。
    rows.sort_by(|a, b| {
        a.analysis_date
            .cmp(&b.analysis_date)
            .then_with(|| a.stock_code.cmp(&b.stock_code))
            .then_with(|| a.created_at.cmp(&b.created_at))
            .then_with(|| a.id.cmp(&b.id))
    });
    let mut seen = HashSet::new();
    rows.retain(|m| seen.insert((m.stock_code.clone(), as_of_of(m))));
    Ok(rows)
}

/// 廉价预检：给定状态集合里**是否还有任何一行**（不含任何结算判据）。
///
/// ## 为什么它可以安全地当作前置闸门
///
/// 三道判据（版本链结算 / 键结算 / 同键去重）**只从命中集合里删行，不会新增**
/// ⇒「原始命中数为 0」蕴含「[`pending_rows`] 必为 0」。所以本函数可以当成
/// [`count_analyses_by_status`] 的**充分早退条件**，而不会改变补跑口径。
///
/// ## 代价对比（2026-10-18 实测）
///
/// 可重试集合的 35 行是建行占位行（大列 0 字节），本查询只取 `id` + `LIMIT 1`
/// ⇒ 毫秒级；而 [`pending_rows`] 还要连带拉 `parent_analysis_id IS NOT NULL`
/// （54 行 / 80 MB）与 `analysis_kind='replay'`（40 行 / 49 MB）两批整行。
/// 启动补跑钩子**每次启动**都会跑一次计数，这条闸门省的正是这份常驻开销。
pub async fn has_any_in_status(
    db: &DatabaseConnection,
    statuses: &[&str],
) -> Result<bool, sea_orm::DbErr> {
    let hit = stock_analyses::Entity::find()
        .select_only()
        .column(stock_analyses::Column::Id)
        .filter(stock_analyses::Column::Status.is_in(statuses.to_vec()))
        .limit(1)
        .into_tuple::<String>()
        .one(db)
        .await?;
    Ok(hit.is_some())
}

/// 统计命中给定状态、且尚未被补跑出结果的行数。
///
/// 判据 = 取件的判据（同在 [`pending_rows`] 内），**含同键去重** —— 两条同键来源行
/// 在收敛账目上只算 1 条，因为它们总共只会被发起 1 次补跑。
pub async fn count_analyses_by_status(
    db: &DatabaseConnection,
    statuses: &[&str],
) -> Result<usize, sea_orm::DbErr> {
    Ok(pending_rows(db, statuses).await?.len())
}

/// 取本轮要处理的行，按 `(analysis_date, stock_code, created_at, id)` **升序**，
/// 同键已去重。
///
/// `max_per_run == 0` 表示不限；非零时截断发生在**过滤与去重之后** —— 上限约束的是
/// 「真正要发起几次补跑」，不是「扫了多少行」。
pub async fn load_rerun_candidates(
    db: &DatabaseConnection,
    statuses: &[&str],
    max_per_run: usize,
) -> Result<Vec<RerunCandidate>, sea_orm::DbErr> {
    let mut rows = pending_rows(db, statuses).await?;
    if max_per_run > 0 {
        rows.truncate(max_per_run);
    }
    Ok(rows
        .into_iter()
        .map(|m| {
            let as_of = as_of_of(&m);
            RerunCandidate { id: m.id, stock_code: m.stock_code, as_of, template_id: m.template_id }
        })
        .collect())
}

/// 读单行的 `status`（**只读这一列**）。
///
/// 供无人值守轮询等终态用：`None` = 行已消失（≠ 某个 status 值，调用方须区分
/// 「还在跑」与「记录没了」两种情况，后者也该停止等待）。
pub async fn fetch_analysis_status(
    db: &DatabaseConnection,
    analysis_id: &str,
) -> Result<Option<String>, sea_orm::DbErr> {
    Ok(stock_analyses::Entity::find_by_id(analysis_id.to_string()).one(db).await?.map(|m| m.status))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 负控：`RerunCandidate` 只能由本模块从实体投影而来 —— 字段集就是补跑的完整输入面，
    /// 多一个字段都意味着「多一条未声明的依赖」，故把字段集钉死。
    #[test]
    fn rerun_candidate_carries_exactly_the_four_rerun_fields() {
        let c = RerunCandidate {
            id: "a1".to_string(),
            stock_code: "600000".to_string(),
            as_of: "2026-10-16".to_string(),
            template_id: None,
        };
        assert_eq!(c.id, "a1");
        assert_eq!(c.stock_code, "600000");
        assert_eq!(c.as_of, "2026-10-16");
        assert!(c.template_id.is_none(), "原样透传，不替调用方兜底默认值");
    }

    /// 直插一条实盘形态的 `stock_analyses` 行（`as_of_date` 为空、`analysis_kind`
    /// 走库默认 `'live'`），只给补跑取件用得上的几列。
    async fn insert_row(
        db: &DatabaseConnection,
        id: &str,
        stock_code: &str,
        status: &str,
        parent: Option<&str>,
    ) {
        use sea_orm::Set;
        let now = chrono::Utc::now().timestamp_millis();
        let active = stock_analyses::ActiveModel {
            id: Set(id.to_string()),
            stock_code: Set(stock_code.to_string()),
            stock_name: Set(String::new()),
            analysis_date: Set("2026-10-15".to_string()),
            provider_id: Set("test".to_string()),
            conversation_id: Set("test-conv".to_string()),
            status: Set(status.to_string()),
            parent_analysis_id: Set(parent.map(str::to_string)),
            created_at: Set(now),
            updated_at: Set(now),
            ..Default::default()
        };
        stock_analyses::Entity::insert(active).exec(db).await.expect("插入分析行应成功");
    }

    /// 直插一条**回放行**：`analysis_kind='replay'`、`as_of_date` = 复现的行情日、
    /// `analysis_date` = 执行日 —— 刻意不同（复现 2026-10-17 实况 `d3a74ec8` 的形态：
    /// 锚是 10-14，执行日是 10-17）。
    async fn insert_replay_row(
        db: &DatabaseConnection,
        id: &str,
        stock_code: &str,
        status: &str,
        parent: Option<&str>,
        as_of: &str,
    ) {
        use sea_orm::Set;
        let now = chrono::Utc::now().timestamp_millis();
        let active = stock_analyses::ActiveModel {
            id: Set(id.to_string()),
            stock_code: Set(stock_code.to_string()),
            stock_name: Set(String::new()),
            analysis_date: Set("2026-10-17".to_string()),
            provider_id: Set("test".to_string()),
            conversation_id: Set("test-conv".to_string()),
            status: Set(status.to_string()),
            parent_analysis_id: Set(parent.map(str::to_string)),
            analysis_kind: Set(AnalysisKind::Replay.as_storage().to_string()),
            as_of_date: Set(Some(as_of.to_string())),
            created_at: Set(now),
            updated_at: Set(now),
            ..Default::default()
        };
        stock_analyses::Entity::insert(active).exec(db).await.expect("插入回放行应成功");
    }

    /// 已补出结果 / 正在补跑的来源行都必须退出取件口，且**计数与取件同谓词**。
    ///
    /// 反向锁：若把 [`resolved_source_ids`] 摘掉，`p1`/`p2` 会重新进批次 ⇒ 本用例红。
    /// 那正是 2026-10-17 那 105 次 429 执行的成因形态：35 条中断行 × 3 轮全量重发，
    /// 而来源行状态永不改 ⇒ 收敛判据永远归不了零。
    #[tokio::test]
    async fn rerun_queue_excludes_sources_that_already_have_a_child_version() {
        use crate::db::create_test_pool;
        let handle = create_test_pool().await.expect("测试库应可创建");
        let db = &handle.conn;
        let statuses = ["interrupted", "failed", "timeout"];

        // 夹具刻意给父行与子行**不同**的 stock_code：断言只看「谁被排除」，
        // 不掺入同键去重与父子相对排序的变量。
        // p1：中断行，已补出 completed 回放版本 ⇒ 排除（来源行自身状态永不改）
        insert_row(db, "p1", "300001", "interrupted", None).await;
        insert_row(db, "p1c", "000001", "completed", Some("p1")).await;
        // p2：中断行，子版本仍 running ⇒ 排除（不得在同一标的上并行起第二条 DAG）
        insert_row(db, "p2", "300002", "interrupted", None).await;
        insert_row(db, "p2c", "000002", "running", Some("p2")).await;
        // p3：中断行，无子版本 ⇒ 保留
        insert_row(db, "p3", "300003", "interrupted", None).await;
        // p4：中断行，子版本 failed ⇒ 两行都保留。这不是疏漏：失败的回放版本本身就是
        //   「跑过但没结论」，按 `RETRY_ROUND_STATUSES` 的定义它下一轮该再补一次。
        insert_row(db, "p4", "300004", "interrupted", None).await;
        insert_row(db, "p4c", "000004", "failed", Some("p4")).await;

        let got: Vec<String> = load_rerun_candidates(db, &statuses, 0)
            .await
            .expect("取件应成功")
            .into_iter()
            .map(|c| c.id)
            .collect();
        assert_eq!(
            got,
            vec!["p4c".to_string(), "p3".to_string(), "p4".to_string()],
            "取件只该剩「真正没补出结论」的三行，且顺序由 stock_code 决定"
        );

        // 计数与取件必须同集：报「剩余 5 条」而实际只取 3 条，就是报告在撒谎。
        assert_eq!(
            count_analyses_by_status(db, &statuses).await.expect("计数应成功"),
            got.len(),
            "剩余计数必须等于取件条数"
        );
    }

    /// 同一 `(stock_code, as_of)` 有多条来源行、回放行只挂其中一条 parent 时，
    /// **键结算**（[`resolved_replay_keys`]）必须让全部同键来源行离队。
    ///
    /// 反向锁：若摘掉键结算，只有挂了 parent 的 `s1` 会被版本链规则排除，`s2` 仍留
    /// 在队列 ⇒ 本用例红。这正是 2026-10-17 的实况形态：603466@2026-10-14 的
    /// `26fd5746` 有回放子行、`6dc93b6b` 没有，收敛判据剩的那 1 条就是 `6dc93b6b` ——
    /// 每批都拿它重新发起、对同一回放目标原地覆盖。
    #[tokio::test]
    async fn rerun_queue_excludes_all_same_key_sources_once_a_replay_settles() {
        use crate::db::create_test_pool;
        let handle = create_test_pool().await.expect("测试库应可创建");
        let db = &handle.conn;
        let statuses = ["interrupted", "failed", "timeout"];

        insert_row(db, "s1", "603466", "interrupted", None).await;
        insert_row(db, "s2", "603466", "interrupted", None).await;
        // 落定的回放行：as_of 与两条来源行同键（来源行 as_of_date 为空 ⇒ 回退
        // analysis_date=10-15），只挂 s1 作 parent。
        insert_replay_row(db, "r1", "603466", "completed", Some("s1"), "2026-10-15").await;

        let got = load_rerun_candidates(db, &statuses, 0).await.expect("取件应成功");
        assert!(
            got.is_empty(),
            "同键回放已落定时两条来源行都必须离队，实际仍剩: {:?}",
            got.iter().map(|c| c.id.as_str()).collect::<Vec<_>>()
        );
        assert_eq!(
            count_analyses_by_status(db, &statuses).await.expect("计数应成功"),
            0,
            "计数与取件必须同集"
        );
    }

    /// 还在可重试状态的回放行**不算结账**（它正是本轮要跑的目标）：来源行保留；
    /// 且三条同键行（两条来源 + 一条回放）经去重后只取一条 —— 被发起的次数 = 1。
    ///
    /// 反向锁：若摘掉同键去重，取件会返回三条同键行 ⇒ 同一回放目标被覆盖 3 次
    /// （12:38 批的实测形态：后一次把前一次刚落库的结论打回 running）。
    #[tokio::test]
    async fn unresolved_replay_keeps_sources_and_dedupe_collapses_key_to_one() {
        use crate::db::create_test_pool;
        let handle = create_test_pool().await.expect("测试库应可创建");
        let db = &handle.conn;
        let statuses = ["interrupted", "failed", "timeout"];

        insert_row(db, "s1", "603466", "interrupted", None).await;
        insert_row(db, "s2", "603466", "interrupted", None).await;
        insert_replay_row(db, "r1", "603466", "interrupted", Some("s1"), "2026-10-15").await;

        let got = load_rerun_candidates(db, &statuses, 0).await.expect("取件应成功");
        assert_eq!(
            got.iter().map(|c| c.id.as_str()).collect::<Vec<_>>(),
            vec!["s1"],
            "账都没结、但同键只该发起一次（保留排序最早者）"
        );
        assert_eq!(
            count_analyses_by_status(db, &statuses).await.expect("计数应成功"),
            1,
            "计数与取件必须同集（两条同键来源行在收敛账目上只算 1 条）"
        );
    }

    /// 候选的 as_of 必须取**锚**（回放行 = `as_of_date`），不是执行日 `analysis_date`；
    /// `as_of_date` 为空的实盘行回退 `analysis_date`。
    ///
    /// 反向锁：投影改回 `analysis_date` ⇒ 本用例红（按执行日发起会新建无关记录，
    /// 而不是命中同日覆盖分支）。
    #[tokio::test]
    async fn candidate_as_of_prefers_anchor_over_execution_date() {
        use crate::db::create_test_pool;
        let handle = create_test_pool().await.expect("测试库应可创建");
        let db = &handle.conn;

        // 回放行：锚 10-14、执行日 10-17（与 d3a74ec8 同形态）；尚未落定，仍是候选。
        insert_replay_row(db, "r2", "603466", "interrupted", None, "2026-10-14").await;
        let got = load_rerun_candidates(db, &["interrupted"], 0).await.expect("取件应成功");
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].as_of, "2026-10-14", "锚是复现的行情日，不得取执行日 10-17");

        // 实盘行（as_of_date 为空）：回退 analysis_date。
        insert_row(db, "s3", "600001", "interrupted", None).await;
        let got = load_rerun_candidates(db, &["interrupted"], 0).await.expect("取件应成功");
        let s3 = got.iter().find(|c| c.id == "s3").expect("s3 应在候选中");
        assert_eq!(s3.as_of, "2026-10-15", "as_of_date 为空时回退 analysis_date");
    }
}
