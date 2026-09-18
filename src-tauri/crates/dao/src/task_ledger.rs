// SPDX-License-Identifier: AGPL-3.0-only

//! 任务账本写入器（P1-C 统一任务账本 · 第一阶段）
//!
//! ## 唯一写路径
//!
//! 本模块是 `background_tasks.status` 的**唯一合法写路径**。修复前存在两条
//! 互不知晓的写路径：
//!
//! - Tauri 命令层 `commands/background_tasks.rs::update_status`（前端走这条）
//! - DAO 仓库层 `dao/background_task_repository.rs::{update_status, stop_task}`（agent 工具走这条）
//!
//! 两者对合法状态、终态集合、时间戳单位（毫秒 / 秒）都各写各的。现在两条都改为
//! 调用 [`transition_task`]，于是：
//!
//! - **状态语义**：由 `harness::task_state::TaskStatus` 单一真源决定；
//! - **时间单位**：统一毫秒（`background_tasks` 命令层原本就是毫秒，
//!   DAO 层与 `scheduler/restore.rs` 原本是秒 —— 混存会让按时间排序静默错序）；
//! - **审计**：每次迁移在同一事务内追加一行 `task_events`；
//! - **时间线全序**：同一任务内的事件 `created_at` **严格递增**（见 `next_event_at`）。
//!   同一毫秒内写多条事件时排序键相等，而数据库对等值键之间的顺序不作保证 ——
//!   那会让「按时间正序」的时间线间歇性倒过来。因此时间戳取「该任务当前最大值 + 1」，
//!   代价是可能略超前墙钟（有意，见该函数文档）。
//!
//! ## 为什么迁移与事件写入必须在同一事务
//!
//! 「状态改了但事件没写」会让审计脊出现空洞，而空洞无法与「这条迁移本来就不该有事件」
//! 区分 —— 事后排障会把空洞当成噪声忽略。同事务保证二者原子。
//!
//! ## 失败语义（fail-closed）
//!
//! 非法迁移返回 `Err` 且**不写任何东西**。刻意不做「尽力而为地写成目标值」：
//! 那正是 EvoFlow 的 `except Exception: return request` 一类 fail-open 的形态，
//! 本项目已在多处审计中将其认定为缺陷（铁律 #12：归因字段不许说谎）。
//!
//! 调用方若只想「确保是某状态」，用幂等写（同值迁移被 [`TaskStatus::can_transition`]
//! 放行），不要吞掉非法迁移的错误。

use chrono::Utc;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, DatabaseConnection, EntityTrait, QueryFilter,
    QueryOrder, QuerySelect, Set, TransactionTrait,
};
use uuid::Uuid;

use axagent_entities::{background_tasks, task_events};
use axagent_harness::task_state::{
    TaskSource, TaskStatus, TransitionRejection, validate_transition_from,
};

/// 迁移失败的原因。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TaskLedgerError {
    /// 任务不存在。
    NotFound(String),
    /// 迁移被状态机拒绝（含未知当前状态 / 未知目标 / 非法迁移）。
    Rejected(TransitionRejection),
    /// 数据库错误（查询 / 更新 / 事件写入）。
    Db(String),
}

impl std::fmt::Display for TaskLedgerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotFound(id) => write!(f, "任务不存在: {id}"),
            Self::Rejected(r) => write!(f, "状态迁移被拒绝: {r}"),
            Self::Db(e) => write!(f, "任务账本数据库错误: {e}"),
        }
    }
}

impl std::error::Error for TaskLedgerError {}

/// 一次状态迁移的输入。
pub struct TransitionRequest<'a> {
    pub task_id: &'a str,
    pub to: TaskStatus,
    pub source: TaskSource,
    /// 触发者：`user` | `system` | `scheduler` | `agent`。
    pub actor: &'a str,
    pub reason: Option<&'a str>,
    /// 附加上下文 JSON（如 `{"exitCode": -1}`）。
    pub payload: Option<serde_json::Value>,
    /// 进程退出码（`background_tasks.exit_code`）。
    ///
    /// 这是本阶段唯一一个「迁移时顺带写的业务列」。放在这里而不是让调用方
    /// 迁移完再补一次 `UPDATE`，是因为补写会产生第二个写路径 —— 而「唯一写路径」
    /// 正是本模块存在的理由。跨域汇入（P1-C 终态）时它应随 `tasks` 表结构泛化。
    pub exit_code: Option<i32>,
}

impl<'a> TransitionRequest<'a> {
    pub fn new(task_id: &'a str, to: TaskStatus, source: TaskSource, actor: &'a str) -> Self {
        Self { task_id, to, source, actor, reason: None, payload: None, exit_code: None }
    }

    pub fn reason(mut self, reason: &'a str) -> Self {
        self.reason = Some(reason);
        self
    }

    pub fn payload(mut self, payload: serde_json::Value) -> Self {
        self.payload = Some(payload);
        self
    }

    /// 顺带写进程退出码。
    pub fn exit_code(mut self, code: i32) -> Self {
        self.exit_code = Some(code);
        self
    }
}

/// 计算下一条事件的 `created_at`：**同一任务内严格递增**。
///
/// 为什么不能直接用墙钟：同一任务内连续两次写入常落在同一毫秒（`record_task_created`
/// 紧跟着一次迁移、或同一批次里的连续迁移）。此时 `list_task_events` 的排序键相等，
/// 而数据库只保证 `ORDER BY` 列有序，**对等值键之间的顺序不作任何保证** ——
/// 于是「按时间正序」的时间线可能整体倒过来，且这种错序只在撞毫秒时出现，
/// 表现为间歇性失败的测试与间歇性倒序的 UI（最难归因的一类缺陷）。
///
/// 取「该任务当前最大 `created_at` 与 `now` 的较大者，相等则 `+1`」，让单一排序键成为全序。
/// 代价：返回值可能略**超前**墙钟（同一毫秒内写 N 条时最多超前 N-1 毫秒）。这是**有意**的
/// —— 时间线的顺序语义比「与墙钟逐毫秒对齐」更重要。反过来说，本列只用于排序与展示，
/// 任何超时 / 租约 / 过期判定都**不得**借用它（那些需要独立的列）。
///
/// 调用方的**顺序要求**：必须在拿到写锁之后调用（`transition_task` 在 `am.update` 之后
/// 才生成事件）。否则两个并发写者会读到同一个 max 再各自 `+1`，又回到「排序键相等」。
async fn next_event_at<C: ConnectionTrait>(
    conn: &C,
    task_id: &str,
    now: i64,
) -> Result<i64, TaskLedgerError> {
    let latest = task_events::Entity::find()
        .filter(task_events::Column::TaskId.eq(task_id))
        .order_by_desc(task_events::Column::CreatedAt)
        .limit(1)
        .one(conn)
        .await
        .map_err(|e| TaskLedgerError::Db(format!("读取任务事件最大时间戳失败: {e}")))?
        .map(|row| row.created_at);
    Ok(match latest {
        Some(max) if max >= now => max + 1,
        _ => now,
    })
}

/// **唯一写路径**：把 `background_tasks.status` 从当前值迁移到 `req.to`，
/// 并在同一事务内追加一条 `task_events`。
///
/// 返回迁移前的状态（供调用方判断「是否真的变了」——事件是**每次都写**的，
/// 因为幂等写本身也是「有人确认过它是这个状态」的证据）。
pub async fn transition_task(
    db: &DatabaseConnection,
    req: TransitionRequest<'_>,
) -> Result<TaskStatus, TaskLedgerError> {
    let row = background_tasks::Entity::find_by_id(req.task_id)
        .one(db)
        .await
        .map_err(|e| TaskLedgerError::Db(format!("读取任务失败: {e}")))?
        .ok_or_else(|| TaskLedgerError::NotFound(req.task_id.to_string()))?;

    // 校验在开启事务之前：非法迁移不应占用写锁。
    let from = validate_transition_from(&row.status, req.to, req.source)
        .map_err(TaskLedgerError::Rejected)?;

    let now = Utc::now().timestamp_millis();

    let txn = db.begin().await.map_err(|e| TaskLedgerError::Db(format!("开启事务失败: {e}")))?;

    // 1) 状态列
    let mut am: background_tasks::ActiveModel = row.into();
    am.status = Set(req.to.as_db_str().to_string());
    am.updated_at = Set(now);
    // `finished_at` 与「是否已结束」严格同构，不做开关：终态写 now，非终态写 NULL。
    // 单独开一个「是否动 finished_at」的开关会引入第二种语义（「保留旧结束时间」），
    // 那个语义在任何调用点上都没有意义，却能被误用成「已完成但显示未结束」。
    if req.to.is_settled() {
        am.finished_at = Set(Some(now));
    } else {
        // 复位（如 `restore` 回 pending）必须清掉结束时间，
        // 否则 UI 会同时看到「未完成」与「已结束」。
        am.finished_at = Set(None);
    }
    if let Some(code) = req.exit_code {
        am.exit_code = Set(Some(code));
    }
    am.update(&txn).await.map_err(|e| TaskLedgerError::Db(format!("更新任务状态失败: {e}")))?;

    // 2) 事件脊（同事务，保证不出现「状态变了但查不到事件」的空洞）
    let event = task_events::ActiveModel {
        id: Set(Uuid::new_v4().to_string()),
        task_id: Set(req.task_id.to_string()),
        source: Set(req.source.as_db_str().to_string()),
        from_status: Set(Some(from.as_db_str().to_string())),
        to_status: Set(req.to.as_db_str().to_string()),
        actor: Set(req.actor.to_string()),
        reason: Set(req.reason.map(|r| r.to_string())),
        payload: Set(req
            .payload
            .as_ref()
            .map(|p| serde_json::to_string(p).unwrap_or_else(|_| "null".to_string()))),
        // 用事务连接（`&txn`）：`am.update` 已取写锁，此后读 max 才与其他写者串行。
        created_at: Set(next_event_at(&txn, req.task_id, now).await?),
    };
    task_events::Entity::insert(event)
        .exec(&txn)
        .await
        .map_err(|e| TaskLedgerError::Db(format!("写入任务事件失败: {e}")))?;

    txn.commit().await.map_err(|e| TaskLedgerError::Db(format!("提交事务失败: {e}")))?;

    Ok(from)
}

/// 记录「任务被创建」这一事件（`from_status = NULL`）。
///
/// 单独一个函数而不是复用 [`transition_task`]，原因：创建时**没有前态**，
/// 且业务行是由调用方自己 insert 的（`spawn_background_task` 的插入带
/// 幂等键检查与命令注入校验，不宜收进本模块）。
///
/// **事件写入失败不应让创建整体失败** —— 但也不能静默。调用方必须把错误
/// 显式记日志（`tracing::error!`），不要 `let _ =`。
///
/// **不开事务（有意）**：本函数只写一行事件，单条 `INSERT` 本身就是原子的，包一层事务
/// 不增加任何原子性；而调用方的业务行是在**另一个已提交的写**里落库的
/// （`commands::background_tasks::spawn_background_task`、`BackgroundTaskRepository::spawn_task`），
/// 事务无法把两者绑成原子，只会在代码里制造「它们同事务」的错觉 ——
/// 那正好是本模块要消灭的那类语义谎报。两个调用点也都不在事务中，没有可复用的外层事务。
/// 并发上：排序键按 `task_id` 分区，不同任务之间无冲突；同一任务被并发重复创建属调用方缺陷，
/// 事务也拦不住（默认隔离级别下两个事务会读到同一个 max）。
pub async fn record_task_created(
    db: &DatabaseConnection,
    task_id: &str,
    source: TaskSource,
    actor: &str,
) -> Result<(), TaskLedgerError> {
    let now = Utc::now().timestamp_millis();
    let at = next_event_at(db, task_id, now).await?;
    let event = task_events::ActiveModel {
        id: Set(Uuid::new_v4().to_string()),
        task_id: Set(task_id.to_string()),
        source: Set(source.as_db_str().to_string()),
        from_status: Set(None),
        to_status: Set(TaskStatus::Pending.as_db_str().to_string()),
        actor: Set(actor.to_string()),
        reason: Set(Some("任务创建".to_string())),
        payload: Set(None),
        created_at: Set(at),
    };
    task_events::Entity::insert(event)
        .exec(db)
        .await
        .map_err(|e| TaskLedgerError::Db(format!("写入创建事件失败: {e}")))?;
    Ok(())
}

/// 拉取某任务的事件时间线（按时间正序，最新 `limit` 条）。
///
/// 顺序**确定**的前提是同一任务内 `created_at` 严格递增（由 `next_event_at` 保证）：
/// 排序键相等时数据库不保证等值行之间的顺序，「先倒序取 N 条再反转」会把不确定的顺序
/// 原样交给调用方。
///
/// 这是 `task_events` 的**读路径** —— 写入型审计表若无读消费者就是死表。
pub async fn list_task_events(
    db: &DatabaseConnection,
    task_id: &str,
    limit: u64,
) -> Result<Vec<task_events::Model>, TaskLedgerError> {
    // 先按倒序取最新 N 条，再反转成正序返回（时间线从上到下）。
    let mut rows = task_events::Entity::find()
        .filter(task_events::Column::TaskId.eq(task_id))
        .order_by_desc(task_events::Column::CreatedAt)
        .limit(limit)
        .all(db)
        .await
        .map_err(|e| TaskLedgerError::Db(format!("读取任务事件失败: {e}")))?;
    rows.reverse();
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;
    use sea_orm::{ConnectionTrait, Database};

    /// 建一张最小可用的 `background_tasks`（只建本模块用到的列），
    /// 避免测试依赖全量 v100 迁移（那会让本模块的测试变成迁移测试）。
    async fn setup() -> DatabaseConnection {
        let db = Database::connect("sqlite::memory:").await.expect("in-memory db");
        db.execute_unprepared(
            "CREATE TABLE background_tasks (\
                 id TEXT PRIMARY KEY NOT NULL,\
                 title TEXT NOT NULL,\
                 description TEXT NOT NULL,\
                 task_type TEXT NOT NULL,\
                 command TEXT,\
                 prompt TEXT,\
                 status TEXT NOT NULL,\
                 output TEXT NOT NULL,\
                 exit_code INTEGER,\
                 conversation_id TEXT,\
                 created_by TEXT,\
                 idempotency_key TEXT,\
                 attempt INTEGER NOT NULL DEFAULT 0,\
                 resume_from TEXT,\
                 created_at BIGINT NOT NULL,\
                 updated_at BIGINT NOT NULL,\
                 finished_at BIGINT\
             )",
        )
        .await
        .expect("create background_tasks");
        db.execute_unprepared(
            "CREATE TABLE task_events (\
                 id TEXT PRIMARY KEY NOT NULL,\
                 task_id TEXT NOT NULL,\
                 source TEXT NOT NULL,\
                 from_status TEXT,\
                 to_status TEXT NOT NULL,\
                 actor TEXT NOT NULL,\
                 reason TEXT,\
                 payload TEXT,\
                 created_at BIGINT NOT NULL\
             )",
        )
        .await
        .expect("create task_events");
        db
    }

    async fn seed(db: &DatabaseConnection, id: &str, status: &str) {
        db.execute_unprepared(&format!(
            "INSERT INTO background_tasks \
             (id, title, description, task_type, status, output, attempt, created_at, updated_at) \
             VALUES ('{id}', 't', 'd', 'bash', '{status}', '', 0, 1700000000000, 1700000000000)"
        ))
        .await
        .expect("seed task");
    }

    async fn event_rows(db: &DatabaseConnection, task_id: &str) -> Vec<task_events::Model> {
        list_task_events(db, task_id, 100).await.expect("list events")
    }

    #[tokio::test]
    async fn legal_transition_updates_status_and_appends_event() {
        let db = setup().await;
        seed(&db, "t1", "pending").await;

        let from = transition_task(
            &db,
            TransitionRequest::new("t1", TaskStatus::Running, TaskSource::Command, "user")
                .reason("spawn"),
        )
        .await
        .expect("pending → running 应合法");
        assert_eq!(from, TaskStatus::Pending);

        let row =
            background_tasks::Entity::find_by_id("t1").one(&db).await.expect("query").expect("row");
        assert_eq!(row.status, "running");
        assert!(row.finished_at.is_none(), "running 不应写 finished_at");

        let evs = event_rows(&db, "t1").await;
        assert_eq!(evs.len(), 1, "每次迁移必须留下恰好一条事件");
        assert_eq!(evs[0].from_status.as_deref(), Some("pending"));
        assert_eq!(evs[0].to_status, "running");
        assert_eq!(evs[0].source, "command");
        assert_eq!(evs[0].actor, "user");
        assert_eq!(evs[0].reason.as_deref(), Some("spawn"));
    }

    /// **故意违反**样例：`completed → running` 这条修复前无人拦的回环。
    /// 断言两件事：状态未变 + 没有写出事件（不能留下「非法迁移的痕迹」污染审计）。
    #[tokio::test]
    async fn illegal_transition_is_rejected_and_leaves_no_trace() {
        let db = setup().await;
        seed(&db, "t1", "completed").await;

        let err = transition_task(
            &db,
            TransitionRequest::new("t1", TaskStatus::Running, TaskSource::Tool, "agent"),
        )
        .await
        .expect_err("completed → running 必须被拒");
        assert!(matches!(err, TaskLedgerError::Rejected(_)), "实际: {err:?}");

        let row =
            background_tasks::Entity::find_by_id("t1").one(&db).await.expect("query").expect("row");
        assert_eq!(row.status, "completed", "被拒的迁移不得改动状态");
        assert!(
            event_rows(&db, "t1").await.is_empty(),
            "被拒的迁移不得写出事件（否则审计脊里全是噪声）"
        );
    }

    /// 未知当前状态必须被拒而不是被猜成某个已知态（铁律 #12）。
    #[tokio::test]
    async fn unknown_current_state_is_rejected() {
        let db = setup().await;
        seed(&db, "t1", "totally_bogus").await;

        let err = transition_task(
            &db,
            TransitionRequest::new("t1", TaskStatus::Failed, TaskSource::Command, "system"),
        )
        .await
        .expect_err("未知状态下的迁移必须被拒");
        match err {
            TaskLedgerError::Rejected(TransitionRejection::UnknownCurrentState(raw)) => {
                assert_eq!(raw, "totally_bogus")
            },
            other => panic!("期望 UnknownCurrentState，实际 {other:?}"),
        }
    }

    /// 幂等写放行且**仍然留痕**：`stop_background_task` 可能重复调用，
    /// 第二次不应报错；而「有人确认过它是 stopped」本身也是审计信息。
    #[tokio::test]
    async fn idempotent_write_is_allowed_and_still_recorded() {
        let db = setup().await;
        seed(&db, "t1", "stopped").await;

        transition_task(
            &db,
            TransitionRequest::new("t1", TaskStatus::Stopped, TaskSource::Command, "user"),
        )
        .await
        .expect("同值写应放行");

        let evs = event_rows(&db, "t1").await;
        assert_eq!(evs.len(), 1);
        assert_eq!(evs[0].from_status.as_deref(), Some("stopped"));
        assert_eq!(evs[0].to_status, "stopped");
    }

    #[tokio::test]
    async fn settled_target_writes_finished_at_and_restore_clears_it() {
        let db = setup().await;
        seed(&db, "t1", "running").await;

        transition_task(
            &db,
            TransitionRequest::new("t1", TaskStatus::Completed, TaskSource::Command, "system"),
        )
        .await
        .expect("running → completed");

        let row =
            background_tasks::Entity::find_by_id("t1").one(&db).await.expect("query").expect("row");
        assert!(row.finished_at.is_some(), "终态必须写 finished_at");

        // stopped → pending 复位（夜间恢复）：finished_at 必须清掉
        seed(&db, "t2", "stopped").await;
        // 先补一个 finished_at，模拟「已结束」
        db.execute_unprepared(
            "UPDATE background_tasks SET finished_at = 1700000009999 WHERE id = 't2'",
        )
        .await
        .expect("set finished_at");

        transition_task(
            &db,
            TransitionRequest::new("t2", TaskStatus::Pending, TaskSource::Restore, "scheduler"),
        )
        .await
        .expect("stopped → pending 复位应合法");

        let row =
            background_tasks::Entity::find_by_id("t2").one(&db).await.expect("query").expect("row");
        assert_eq!(row.status, "pending");
        assert!(
            row.finished_at.is_none(),
            "复位为 pending 必须清 finished_at，否则 UI 同时显示「未完成」与「已结束」"
        );
    }

    /// 时间戳必须是**毫秒**。用「与当前毫秒时钟同量级」判定，而不是硬编码值 ——
    /// 这条断言的作用是拦住「有人改回 timestamp()」。
    #[tokio::test]
    async fn event_timestamp_is_millis_not_seconds() {
        let db = setup().await;
        seed(&db, "t1", "pending").await;
        transition_task(
            &db,
            TransitionRequest::new("t1", TaskStatus::Running, TaskSource::Command, "user"),
        )
        .await
        .expect("pending → running");

        let evs = event_rows(&db, "t1").await;
        let now_ms = Utc::now().timestamp_millis();
        assert!(
            (now_ms - evs[0].created_at).abs() < 60_000,
            "事件时间戳 {} 与当前毫秒时钟相差过大（疑似写成了秒）",
            evs[0].created_at
        );
        assert!(evs[0].created_at > 1_000_000_000_000, "事件时间戳量级不像毫秒");
    }

    #[tokio::test]
    async fn missing_task_reports_not_found() {
        let db = setup().await;
        let err = transition_task(
            &db,
            TransitionRequest::new("nope", TaskStatus::Running, TaskSource::Command, "user"),
        )
        .await
        .expect_err("不存在的任务应报错");
        assert!(matches!(err, TaskLedgerError::NotFound(_)), "实际: {err:?}");
    }

    /// 「回到队列」的来源约束必须在**账本这一层**真的生效 ——
    /// 只在 harness 的纯函数里测过不算，因为账本才是唯一写路径。
    #[tokio::test]
    async fn reset_to_pending_is_only_allowed_from_restore_source() {
        let db = setup().await;
        seed(&db, "t1", "running").await;

        // tool 来源复位：拒绝
        let err = transition_task(
            &db,
            TransitionRequest::new("t1", TaskStatus::Pending, TaskSource::Tool, "agent"),
        )
        .await
        .expect_err("非 restore 来源的复位必须被拒");
        assert!(
            matches!(
                err,
                TaskLedgerError::Rejected(TransitionRejection::ResetNotFromRestore { .. })
            ),
            "实际: {err:?}"
        );

        // restore 来源复位：放行
        transition_task(
            &db,
            TransitionRequest::new("t1", TaskStatus::Pending, TaskSource::Restore, "scheduler"),
        )
        .await
        .expect("restore 来源的复位应放行");

        let row =
            background_tasks::Entity::find_by_id("t1").one(&db).await.expect("query").expect("row");
        assert_eq!(row.status, "pending");
        // 只有成功那次留下事件
        let evs = event_rows(&db, "t1").await;
        assert_eq!(evs.len(), 1);
        assert_eq!(evs[0].source, "restore");
    }

    /// 创建事件必须可查（`from_status = NULL`），且时间线按正序返回。
    #[tokio::test]
    async fn created_event_is_queryable_and_timeline_is_ascending() {
        let db = setup().await;
        seed(&db, "t1", "pending").await;
        record_task_created(&db, "t1", TaskSource::Command, "user").await.expect("record created");
        transition_task(
            &db,
            TransitionRequest::new("t1", TaskStatus::Running, TaskSource::Command, "user"),
        )
        .await
        .expect("pending → running");

        let evs = event_rows(&db, "t1").await;
        assert_eq!(evs.len(), 2);
        assert!(evs[0].from_status.is_none(), "创建事件没有前态");
        assert_eq!(evs[0].to_status, "pending");
        assert_eq!(evs[1].to_status, "running");
        assert!(
            evs[0].created_at < evs[1].created_at,
            "时间线必须**严格**按时间正序（相等 ⇒ 顺序不确定，UI 可能倒着显示）"
        );
    }

    /// `payload` 序列化失败不得让整条事件写坏 —— 落 "null" 而不是 panic。
    #[tokio::test]
    async fn payload_is_serialized_as_json_text() {
        let db = setup().await;
        seed(&db, "t1", "running").await;
        transition_task(
            &db,
            TransitionRequest::new("t1", TaskStatus::Failed, TaskSource::Command, "system")
                .payload(serde_json::json!({"exitCode": -1})),
        )
        .await
        .expect("running → failed");

        let evs = event_rows(&db, "t1").await;
        let p = evs[0].payload.as_deref().expect("payload 应存在");
        assert!(p.contains("exitCode"), "payload 应保留结构化上下文，实际 {p}");
    }

    /// **回归测试**：同一任务紧循环写 20 条事件时 `created_at` 必须严格递增，
    /// 且读回来的顺序必须等于写入顺序。
    ///
    /// 为什么需要这条：原实现直接写墙钟，紧循环的 20 次写入大概率落在同一毫秒，
    /// 排序键相等时数据库对等值行的顺序不作保证 ⇒ 时间线可能倒序。
    /// 用逐条不同的 `reason` 标记写入序号，因此这条测试断言的不只是「时间戳单调」，
    /// 而是「时间线顺序 == 写入顺序」这个用户可见的结果。
    #[tokio::test]
    async fn rapid_writes_keep_timeline_strictly_ascending() {
        let db = setup().await;
        seed(&db, "t1", "pending").await;

        const N: usize = 20;
        // `pending → pending` 是幂等写，被状态机放行（见 `TaskStatus::can_transition`），
        // 因此可以不依赖迁移多样性就把事件脊写满。
        let reasons: Vec<String> = (0..N).map(|i| format!("n{i}")).collect();
        let wall = Utc::now().timestamp_millis();
        for reason in &reasons {
            transition_task(
                &db,
                TransitionRequest::new("t1", TaskStatus::Pending, TaskSource::Command, "user")
                    .reason(reason),
            )
            .await
            .expect("同值写应放行");
        }

        let evs = event_rows(&db, "t1").await;
        assert_eq!(evs.len(), N, "N 次迁移必须留下 N 条事件");
        for i in 0..N {
            assert_eq!(
                evs[i].reason.as_deref(),
                Some(reasons[i].as_str()),
                "第 {i} 条事件顺序错位：时间线不是写入顺序"
            );
            assert!(evs[i].created_at >= wall, "第 {i} 条事件时间戳早于写入前的墙钟");
            if i > 0 {
                assert!(
                    evs[i - 1].created_at < evs[i].created_at,
                    "第 {} → {} 条事件时间戳不严格递增（{} !< {}）",
                    i - 1,
                    i,
                    evs[i - 1].created_at,
                    evs[i].created_at
                );
            }
        }
    }

    /// **回归测试（不依赖墙钟分辨率）**：账本里已有一条「未来时间戳」的事件时，
    /// 新写的事件必须排到它**之后**。
    ///
    /// 这条是 `next_event_at` 的**确定性**探针：把实现退回裸墙钟，新事件取 `now`
    /// （早于那条未来事件），时间线就会把新事件排在前面 —— 必然变红，
    /// 不需要靠「两次写入撞同一毫秒」这种运气。
    #[tokio::test]
    async fn new_event_sorts_after_an_existing_future_timestamp() {
        let db = setup().await;
        seed(&db, "t1", "pending").await;
        let future = Utc::now().timestamp_millis() + 60_000;
        db.execute_unprepared(&format!(
            "INSERT INTO task_events \
             (id, task_id, source, from_status, to_status, actor, reason, payload, created_at) \
             VALUES ('ev-future', 't1', 'command', NULL, 'pending', 'system', '既有事件', NULL, {future})"
        ))
        .await
        .expect("插入未来事件");

        transition_task(
            &db,
            TransitionRequest::new("t1", TaskStatus::Running, TaskSource::Command, "user"),
        )
        .await
        .expect("pending → running");

        let evs = event_rows(&db, "t1").await;
        assert_eq!(evs.len(), 2);
        assert_eq!(
            evs[0].reason.as_deref(),
            Some("既有事件"),
            "既有事件的 created_at 更大，必须排在前面（否则「按时间正序」不成立）"
        );
        assert_eq!(evs[1].to_status, "running", "新事件必须排在最后");
        assert!(
            evs[0].created_at < evs[1].created_at,
            "新事件必须严格晚于既有最大值: {} !< {}",
            evs[0].created_at,
            evs[1].created_at
        );
    }
}
