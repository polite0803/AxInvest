// SPDX-License-Identifier: AGPL-3.0-only

//! 任务状态真源（Task Ledger — P1-C）
//!
//! ## 为什么需要这个模块
//!
//! 本项目历史上「任务」概念散落在多张表、多套状态字符串里：
//!
//! - `background_tasks.status`（`pending|running|completed|failed|stopped`）
//! - `index_jobs.status` / `import_jobs.status` / `tool_executions.status`
//! - `workflow_executions.status` / `plans.status` / `opc_work_items.status`
//!
//! 每套都是**裸字符串**：写入点直接 `Set("running".to_string())`，
//! 没有任何编译期或运行期约束。后果是三类静默失效：
//!
//! 1. **非法回环**：`completed → running` 这种回环无人拦；
//! 2. **拼写漂移**：`"stopped"` / `"stop"` / `"cancelled"` 各写各的，
//!    消费者 `match` 落空后走 `_ =>` 兜底，看起来「正常」；
//! 3. **状态变化不可追溯**：没有事件脊，事后无法回答「谁在什么时候改的」。
//!
//! 本模块提供**单一真源**：枚举 + 静态迁移表 + 解析/序列化。
//! 写入收敛由 `axagent_dao::task_ledger::transition_task()` 承担。
//!
//! ## 与 `business_state_machine` 的分工（勿重复定义）
//!
//! `harness::business_state_machine` 是**工作流设计者可配置**的 FSM
//! （状态/转移由模板声明，带 guard 闭包，由 `rt-workflow` 执行）。
//! 本模块是**固定的任务生命周期**：状态集合与合法迁移由代码硬编码，
//! 不接受外部配置 —— 因为「任务能否从完成回退到运行中」不是业务可选项。
//! 两者正交，不应合并。
//!
//! ## 关于 `awaiting_close`（刻意未落地）
//!
//! P1-C 计划里的「待闭环」态（结案必须带 handler → 软回执唤醒上级）**本模块
//! 尚未定义该变体**，原因是它需要两个真实入边才能有意义：① 一个「验收动作」
//! 写它；② 一个推进器（超时回收或上级回执）把它推到终态。
//! 目前两者都不存在 —— 先加枚举变体等于制造「声明了但无入边供给」的静默失效
//! （见项目铁律 #6）。待验收动作 + 回收器一起落地时再加，并同步迁移表与 UI 文案。

use serde::{Deserialize, Serialize};

/// 任务生命周期的规范状态。
///
/// 与数据库中 `background_tasks.status` 的**线格式字符串**一一对应；
/// 数据库值保持不变（前端 `STATUS_CONFIG` 与存量行都依赖它）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    /// 已创建，尚未开始执行。
    Pending,
    /// 执行中。
    Running,
    /// 正常完成。
    Completed,
    /// 执行失败（含 panic 兜底、非零退出码）。
    Failed,
    /// 被主动中止（用户点停止 / 系统取消）。
    ///
    /// 线格式字符串是 `"stopped"` 而非 `"cancelled"` —— 存量数据与前端
    /// 都使用它，改名属破坏性变更，不做。
    Stopped,
    /// **未知值**：数据库里出现了不认识的字符串。
    ///
    /// 刻意保留为独立变体而不是映射到某个已知态：把未知值当成 `Pending`
    /// 会让「状态字段在说谎」这件事被掩盖（项目铁律 #12）。
    /// `transition_task` 遇到当前状态为 `Unknown` 时**拒绝迁移**并告警。
    Unknown,
}

impl TaskStatus {
    /// 数据库/线格式字符串。
    pub fn as_db_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Running => "running",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Stopped => "stopped",
            // `Unknown` 不是可写入的状态：它是「读到了不认识的值」这一事实的载体。
            // 返回空串会让调用方写出一个更隐晦的坏值；返回哨兵字符串则可能被误当真值。
            // 因此显式返回空串，且 `transition_task` 的入参类型是 `TaskStatus`
            // 而非 `Unknown` 的可写路径（见 `TaskStatus::writable`）。
            Self::Unknown => "",
        }
    }

    /// 解析数据库字符串。不认识的值解析为 [`TaskStatus::Unknown`]（不 panic）。
    pub fn from_db_str(s: &str) -> Self {
        match s {
            "pending" => Self::Pending,
            "running" => Self::Running,
            "completed" => Self::Completed,
            "failed" => Self::Failed,
            "stopped" => Self::Stopped,
            _ => Self::Unknown,
        }
    }

    /// 全部**可写入**的状态（不含 `Unknown`）。
    pub fn writable() -> &'static [TaskStatus] {
        &[Self::Pending, Self::Running, Self::Completed, Self::Failed, Self::Stopped]
    }

    /// 终态：不再接受除自身以外的迁移。
    ///
    /// `Stopped` 视为终态 —— 重跑必须显式经过 `restore` 路径（先置 `Pending`），
    /// 见 [`TaskStatus::allowed_next`] 对 `Stopped → Pending` 的放行。
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Completed | Self::Failed | Self::Stopped)
    }

    /// 静态迁移表：`from` 可以合法迁移到哪些状态。
    ///
    /// 这张表是**唯一真源**。修改它等于修改任务生命周期语义，
    /// 必须同步 `task_state::tests` 里的不变量测试。
    ///
    /// 注意：本表不表达「谁有权触发」。「回到队列」（`→ Pending`）另有来源约束，
    /// 由 `axagent_dao::task_ledger::transition_task` 强制（只允许 `TaskSource::Restore`）——
    /// 静态表无法表达该约束，所以它必须是运行期检查，且**必须被测试覆盖**。
    pub fn allowed_next(self) -> &'static [TaskStatus] {
        match self {
            // 创建后：可以开始、可以被直接停掉、也可以在启动前就失败
            // （`spawn_background_task` 的 Drop guard 在 never-started 时写 `failed`）。
            Self::Pending => &[Self::Running, Self::Stopped, Self::Failed],
            // 执行中：正常完成 / 失败 / 被停掉 / 被启动恢复复位回队列。
            Self::Running => &[Self::Completed, Self::Failed, Self::Stopped, Self::Pending],
            // 被停掉的任务可以复位回队列（夜间恢复 / 手动重跑）。
            Self::Stopped => &[Self::Pending],
            // 真正不可复活的两个终态：重跑必须新建任务，`attempt` 记在父任务上。
            // 放行 `Completed → Running` 会让「这个任务到底跑过几次」无法回答。
            Self::Completed => &[],
            Self::Failed => &[],
            // 未知状态：没有可证明合法的迁移，全部拒绝。
            Self::Unknown => &[],
        }
    }

    /// `from → to` 是否合法。
    pub fn can_transition(self, to: TaskStatus) -> bool {
        // 幂等写：同值重写不报错（大量调用点是「确保它是 X」而非「改成 X」，
        // 例如 `stop_background_task` 在已 stopped 时重复调用）。
        // 但 `Unknown → Unknown` 不算合法 —— 那是纯噪声。
        if self == to && self != Self::Unknown {
            return true;
        }
        self.allowed_next().contains(&to)
    }

    /// 该状态是否代表「已结束且不再占用执行资源」。
    ///
    /// 与 [`TaskStatus::is_terminal`] 的区别：`Stopped` 可以复位回 `Pending`，
    /// 所以它不是**永久**终态；但就「是否还在跑」而言它是结束态。
    /// 该判据用于 UI 计数与队列扫描，不要拿 `is_terminal` 代替（== 漏算 stopped，
    /// 与前端 `STATUS_CONFIG` 的 `finished` 语义不一致）。
    pub fn is_settled(self) -> bool {
        self.is_terminal() || self == Self::Unknown
    }
}

impl std::fmt::Display for TaskStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_db_str())
    }
}

/// 任务来源域。
///
/// P1-C 的目标是**三源（chat / workflow / role）写同一张账本**；本阶段先把
/// 任务域内部的两条并行写路径（Tauri 命令层 / DAO 仓库层）收敛，`TaskSource`
/// 用于事件脊上标注来源，为后续跨域汇入留出字段。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskSource {
    /// Tauri 命令层（`commands/background_tasks.rs`）—— 前端 TaskPanel 走这条。
    Command,
    /// 工具层（`TaskCreate` 等 agent 工具）—— 经 `BackgroundTaskRepository` 走这条。
    Tool,
    /// Scheduler 启动恢复（`scheduler/restore.rs`）。
    Restore,
    /// 未知/其他来源。
    Other,
}

impl TaskSource {
    pub fn as_db_str(self) -> &'static str {
        match self {
            Self::Command => "command",
            Self::Tool => "tool",
            Self::Restore => "restore",
            Self::Other => "other",
        }
    }

    pub fn from_db_str(s: &str) -> Self {
        match s {
            "command" => Self::Command,
            "tool" => Self::Tool,
            "restore" => Self::Restore,
            _ => Self::Other,
        }
    }
}

/// 迁移被拒绝的原因（供日志与错误信息使用，不泄漏到用户可见文案）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TransitionRejection {
    /// 当前状态未知，无法证明迁移合法。
    UnknownCurrentState(String),
    /// 目标是 `Unknown`（不是可写状态）。
    UnknownTarget,
    /// 迁移表不允许。
    Illegal { from: TaskStatus, to: TaskStatus },
    /// 「回到队列」（`→ Pending`）被非恢复来源触发。
    ///
    /// 单独一个变体而不是并进 `Illegal`：这是**策略**拒绝而非**结构**拒绝，
    /// 排障时需要区分「这张边根本不存在」和「这张边存在但你不该走」。
    ResetNotFromRestore { from: TaskStatus },
}

impl std::fmt::Display for TransitionRejection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnknownCurrentState(s) => {
                write!(f, "当前状态为未知值 {s:?}，无法证明迁移合法")
            },
            Self::UnknownTarget => write!(f, "目标是 Unknown，不是可写入状态"),
            Self::Illegal { from, to } => write!(
                f,
                "非法迁移 {from} → {to}（允许的后续状态：{:?}）",
                from.allowed_next().iter().map(|s| s.as_db_str()).collect::<Vec<_>>()
            ),
            Self::ResetNotFromRestore { from } => {
                write!(f, "{from} → pending 只允许由启动恢复（restore）触发，当前来源不是 restore")
            },
        }
    }
}

impl std::error::Error for TransitionRejection {}

/// 结构校验：`raw_from → to` 在静态迁移表上是否合法。`raw_from` 是数据库原始字符串。
///
/// 不含「来源约束」—— 那是 [`validate_transition_from`] 的职责。
pub fn validate_transition(
    raw_from: &str,
    to: TaskStatus,
) -> Result<TaskStatus, TransitionRejection> {
    let from = TaskStatus::from_db_str(raw_from);
    if from == TaskStatus::Unknown {
        return Err(TransitionRejection::UnknownCurrentState(raw_from.to_string()));
    }
    if to == TaskStatus::Unknown {
        return Err(TransitionRejection::UnknownTarget);
    }
    if !from.can_transition(to) {
        return Err(TransitionRejection::Illegal { from, to });
    }
    Ok(from)
}

/// 完整校验：结构迁移表 + 来源约束。
///
/// **来源约束为何必须存在**：`→ Pending` 是「回到队列」语义。它只应由启动恢复
/// （`scheduler/restore.rs`，`TaskSource::Restore`）触发；若任何入口都能把任务
/// 塞回队列，就无法区分「任务自己回到队列」与「被人为重置」，而这正是审计要回答的问题。
///
/// 该约束无法写进静态迁移表（表里没有「谁在调用」这一维），所以它是运行期检查 ——
/// 也正因如此，它必须被测试覆盖，否则就是一条「声明了但没人验证」的规则。
pub fn validate_transition_from(
    raw_from: &str,
    to: TaskStatus,
    source: TaskSource,
) -> Result<TaskStatus, TransitionRejection> {
    let from = validate_transition(raw_from, to)?;
    if to == TaskStatus::Pending && from != TaskStatus::Pending && source != TaskSource::Restore {
        return Err(TransitionRejection::ResetNotFromRestore { from });
    }
    Ok(from)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 线格式字符串必须与数据库存量值完全一致 —— 改名会静默破坏前端
    /// `STATUS_CONFIG` 与所有存量行。这条测试是那道闸门。
    #[test]
    fn db_str_roundtrip_is_stable() {
        for s in TaskStatus::writable() {
            let raw = s.as_db_str();
            assert_eq!(TaskStatus::from_db_str(raw), *s, "线格式字符串 {raw:?} 无法回环解析");
        }
        assert_eq!(TaskStatus::Pending.as_db_str(), "pending");
        assert_eq!(TaskStatus::Running.as_db_str(), "running");
        assert_eq!(TaskStatus::Completed.as_db_str(), "completed");
        assert_eq!(TaskStatus::Failed.as_db_str(), "failed");
        // 刻意不是 "cancelled"：存量数据用 "stopped"。
        assert_eq!(TaskStatus::Stopped.as_db_str(), "stopped");
    }

    #[test]
    fn unknown_is_not_writable_and_has_no_transitions() {
        assert!(!TaskStatus::writable().contains(&TaskStatus::Unknown));
        assert!(TaskStatus::Unknown.allowed_next().is_empty());
        assert!(TaskStatus::Unknown.can_transition(TaskStatus::Pending).eq(&false));
    }

    /// 不认识的字符串必须解析为 `Unknown` 而不是 panic，也不能被当成某个已知态。
    #[test]
    fn unrecognized_values_become_unknown_not_panic() {
        for raw in ["", "CANCELLED", "canceled", "stop", "pending ", "RUNNING"] {
            assert_eq!(
                TaskStatus::from_db_str(raw),
                TaskStatus::Unknown,
                "{raw:?} 被误判为已知状态"
            );
        }
    }

    /// 迁移表的不变量：终态不自环、可写状态必须能从 `Pending` 可达。
    #[test]
    fn transition_table_invariants() {
        // 幂等写：同值（非 Unknown）永远放行。
        for s in TaskStatus::writable() {
            assert!(s.can_transition(*s), "{s} 自环被拒，会让幂等写报错");
        }
        // 终态不自环以外的出边：
        assert!(TaskStatus::Completed.allowed_next().is_empty());
        assert!(TaskStatus::Failed.allowed_next().is_empty());
        assert_eq!(TaskStatus::Stopped.allowed_next(), &[TaskStatus::Pending]);
        // 每个可写状态都必须能从 Pending 经有限步到达 —— 否则就是死状态。
        for s in TaskStatus::writable() {
            if *s == TaskStatus::Pending {
                continue;
            }
            assert!(reachable_from_pending(*s, 3), "{s} 从 Pending 不可达，是死状态");
        }
    }

    fn reachable_from_pending(target: TaskStatus, depth: usize) -> bool {
        fn walk(cur: TaskStatus, target: TaskStatus, budget: usize) -> bool {
            if cur == target {
                return true;
            }
            if budget == 0 {
                return false;
            }
            cur.allowed_next().iter().any(|n| walk(*n, target, budget - 1))
        }
        walk(TaskStatus::Pending, target, depth)
    }

    /// 关键的**故意违反**样例：`completed → running` 必须被拒。
    /// 这是修复前真实存在、且无人拦截的回环。
    #[test]
    fn completed_cannot_go_back_to_running() {
        assert!(TaskStatus::Completed.can_transition(TaskStatus::Running).eq(&false));
        let err = validate_transition("completed", TaskStatus::Running).unwrap_err();
        assert!(matches!(err, TransitionRejection::Illegal { .. }));
    }

    #[test]
    fn unknown_current_state_is_rejected_not_guessed() {
        let err = validate_transition("totally_bogus", TaskStatus::Running).unwrap_err();
        match err {
            TransitionRejection::UnknownCurrentState(raw) => assert_eq!(raw, "totally_bogus"),
            other => panic!("期望 UnknownCurrentState，实际 {other:?}"),
        }
    }

    #[test]
    fn unknown_target_is_rejected() {
        let err = validate_transition("pending", TaskStatus::Unknown).unwrap_err();
        assert_eq!(err, TransitionRejection::UnknownTarget);
    }

    /// 「回到队列」只能由启动恢复触发 —— 这是运行期策略，静态迁移表表达不了，
    /// 因此必须在这里钉住。**故意违反**样例：`tool` 来源尝试 `running → pending`。
    #[test]
    fn reset_to_pending_requires_restore_source() {
        // restore 来源：放行
        validate_transition_from("running", TaskStatus::Pending, TaskSource::Restore)
            .expect("restore 来源应可复位 running → pending");
        validate_transition_from("stopped", TaskStatus::Pending, TaskSource::Restore)
            .expect("restore 来源应可复位 stopped → pending");
        // pending → pending 是幂等写，不该被来源约束拦
        validate_transition_from("pending", TaskStatus::Pending, TaskSource::Command)
            .expect("pending → pending 幂等写应放行");

        // 非 restore 来源：拒绝，且必须是 ResetNotFromRestore 而不是 Illegal
        for src in [TaskSource::Command, TaskSource::Tool, TaskSource::Other] {
            let err = validate_transition_from("running", TaskStatus::Pending, src).unwrap_err();
            assert_eq!(
                err,
                TransitionRejection::ResetNotFromRestore { from: TaskStatus::Running },
                "来源 {src:?} 应被策略拒绝而非结构拒绝"
            );
        }
    }

    #[test]
    fn settle_semantics_cover_stopped() {
        // `is_terminal` 是「永久终态」，`is_settled` 是「已停止占用资源」。
        assert!(TaskStatus::Stopped.is_settled());
        assert!(TaskStatus::Stopped.is_terminal());
        assert!(TaskStatus::Completed.is_settled());
        assert!(TaskStatus::Unknown.is_settled());
        assert!(!TaskStatus::Running.is_settled());
        assert!(!TaskStatus::Pending.is_settled());
    }

    #[test]
    fn source_roundtrip() {
        for s in [TaskSource::Command, TaskSource::Tool, TaskSource::Restore, TaskSource::Other] {
            assert_eq!(TaskSource::from_db_str(s.as_db_str()), s);
        }
        assert_eq!(TaskSource::from_db_str("nonsense"), TaskSource::Other);
    }
}
