// SPDX-License-Identifier: AGPL-3.0-only

//! 部门运行配额解析与校验（docs/bot-integration/P4/ADR-P4-1/2/8、MODEL-P4 §2）。
//!
//! 作用域只有一种：`owner_scope='department'` 的编排及其运行子树。实例级编排与
//! 纯委派链直接拿 P3 常量，**零 DB 读**（ADR-P4-8 的 R2 收口）。
//!
//! 一次解析：run 创建时 `resolve_workflow_limits` 查一次库，结果经
//! [`AssignmentContext`] 按 `workflow_run_id` 登记；子孙 assignment 复用同一组
//! 上限，不在派发热路径重复查库。超额一律拒绝并报字段（D3=A），从不静默夹紧。

// SAFETY: 本文件的 std::sync::Mutex 仅保护同步登记表，guard 只在单次调用内存活，
// 不跨 await。start_run/orchestrate 在 tokio runtime 内调用本表，tokio::sync::Mutex::blocking_lock
// 会 panic（Cannot block the current thread from within a runtime）。符合 clippy.toml 合法例外。
#![allow(clippy::disallowed_types)]

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use axagent_dao::DatabaseConnection;
use axagent_dao::repo::bot_department_quota as quota_repo;
use axagent_dao::repo::bot_workflow_run as run_repo;
use axagent_harness::core_error::AxAgentError;
use axagent_harness::types::{BotDepartmentQuota, BotWorkflow, BotWorkflowStep, quota_defaults};

use crate::commands::error::{ErrorCategory, ErrorResponse};
use crate::commands::error_code::bot as bot_err;

/// 一次 run 的生效运行上限。无部门配额时逐项等于 P3 冻结常量，
/// 有配额时逐项取 `min(定义值, 部门配额)`（ADR-P4-1）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuotaLimits {
    pub max_concurrency: i32,
    pub total_timeout_secs: i32,
    pub max_steps: i32,
    pub max_in_flight_per_depth: i32,
    pub step_timeout_secs: i32,
    pub max_concurrent_runs: i32,
}

impl QuotaLimits {
    /// P3 默认（ADR-P4-1：无配额行 = P3 行为逐字节一致）。
    pub fn p3_defaults() -> Self {
        Self {
            max_concurrency: quota_defaults::MAX_CONCURRENCY,
            total_timeout_secs: quota_defaults::TOTAL_TIMEOUT_SECS,
            max_steps: quota_defaults::MAX_STEPS,
            max_in_flight_per_depth: quota_defaults::MAX_IN_FLIGHT_PER_DEPTH,
            step_timeout_secs: quota_defaults::STEP_TIMEOUT_SECS,
            max_concurrent_runs: quota_defaults::MAX_CONCURRENT_RUNS,
        }
    }

    /// 定义侧基线：并发与总超时取定义值（P3 就是按定义值执行），其余四项
    /// （步数上限、每层在途、单步超时、同时在跑数）取 P3 常量。
    /// 无配额行时本结构与 P3 行为逐项一致。
    pub fn from_workflow(workflow: &BotWorkflow) -> Self {
        let mut limits = Self::p3_defaults();
        limits.max_concurrency = workflow.max_concurrency;
        limits.total_timeout_secs = workflow.total_timeout_secs;
        limits
    }

    /// 收紧到部门配额：非部门作用域不调用本方法。
    pub fn tighten(&self, quota: &BotDepartmentQuota) -> Self {
        Self {
            max_concurrency: self.max_concurrency.min(quota.max_concurrency),
            total_timeout_secs: self.total_timeout_secs.min(quota.total_timeout_secs),
            max_steps: self.max_steps.min(quota.max_steps),
            max_in_flight_per_depth: self
                .max_in_flight_per_depth
                .min(quota.max_in_flight_per_depth),
            step_timeout_secs: self.step_timeout_secs.min(quota.step_timeout_secs),
            max_concurrent_runs: self.max_concurrent_runs.min(quota.max_concurrent_runs),
        }
    }

    /// 单轮执行超时封顶（workflow 步骤与子链 dispatch 共用）。
    pub fn cap_turn(&self, requested: Duration) -> Duration {
        requested.min(Duration::from_secs(self.step_timeout_secs.max(1) as u64))
    }
}

/// 配额越界描述（D3=A：拒绝并报字段，detail 带字段名 + 定义值 + 上限）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuotaViolation {
    pub field: &'static str,
    pub defined: i64,
    pub ceiling: i64,
    /// 附加说明（如超时的步骤 key、并发 run 的口径），拼在 detail 末尾。
    pub note: Option<String>,
}

impl QuotaViolation {
    pub fn new(field: &'static str, defined: i64, ceiling: i64) -> Self {
        Self { field, defined, ceiling, note: None }
    }

    pub fn with_note(mut self, note: impl Into<String>) -> Self {
        self.note = Some(note.into());
        self
    }

    pub fn detail(&self) -> String {
        let mut detail =
            format!("{}: 定义值 {} 超过部门配额上限 {}", self.field, self.defined, self.ceiling);
        if let Some(note) = &self.note {
            detail.push('（');
            detail.push_str(note);
            detail.push('）');
        }
        detail
    }

    /// `BOT_QUOTA_EXCEEDED` 的标准错误出口（命令与 runner 共用）。
    pub fn response(&self) -> String {
        ErrorResponse::new(bot_err::QUOTA_EXCEEDED)
            .with_category(ErrorCategory::Validation)
            .with_param("field", self.field)
            .with_detail(self.detail())
            .to_string()
    }
}

fn quota_conflict(err: AxAgentError) -> String {
    ErrorResponse::from_error(err, ErrorCategory::Unrecoverable).to_string()
}

/// 部门作用域：只有 `owner_scope='department'` 且有部门 id 的编排才受配额约束。
pub fn quota_department(workflow: &BotWorkflow) -> Option<&str> {
    if workflow.owner_scope == "department" {
        workflow.department_id.as_deref()
    } else {
        None
    }
}

/// 定义 vs 部门配额逐字段校验（纯函数，D3=A 拒绝不夹紧）。
pub fn validate_workflow_against_quota(
    max_concurrency: i32,
    total_timeout_secs: i32,
    steps: &[BotWorkflowStep],
    quota: &BotDepartmentQuota,
) -> Result<(), QuotaViolation> {
    if max_concurrency > quota.max_concurrency {
        return Err(QuotaViolation::new(
            "maxConcurrency",
            max_concurrency as i64,
            quota.max_concurrency as i64,
        ));
    }
    if total_timeout_secs > quota.total_timeout_secs {
        return Err(QuotaViolation::new(
            "totalTimeoutSecs",
            total_timeout_secs as i64,
            quota.total_timeout_secs as i64,
        ));
    }
    if steps.len() as i32 > quota.max_steps {
        return Err(QuotaViolation::new("maxSteps", steps.len() as i64, quota.max_steps as i64));
    }
    if let Some(step) = steps.iter().find(|s| s.timeout_secs > quota.step_timeout_secs) {
        return Err(QuotaViolation::new(
            "stepTimeoutSecs",
            step.timeout_secs as i64,
            quota.step_timeout_secs as i64,
        )
        .with_note(format!("步骤 {}", step.step_key)));
    }
    Ok(())
}

/// run 创建时的解析结果登记表。键 = `workflow_run_id`；子孙 assignment 沿父链
/// 继承该 id（`bot_assignment` repo 的 inherited 五元组），因此不重复查库。
pub struct AssignmentContext;

static CONTEXTS: OnceLock<Mutex<HashMap<String, QuotaLimits>>> = OnceLock::new();

fn contexts() -> &'static Mutex<HashMap<String, QuotaLimits>> {
    CONTEXTS.get_or_init(|| Mutex::new(HashMap::new()))
}

impl AssignmentContext {
    pub fn install(run_id: &str, limits: QuotaLimits) {
        contexts().lock().expect("登记表锁不应毒化").insert(run_id.to_string(), limits);
    }

    pub fn get(run_id: &str) -> Option<QuotaLimits> {
        contexts().lock().expect("登记表锁不应毒化").get(run_id).cloned()
    }

    /// run 终态时移除，避免登记表随进程长跑无界增长。
    pub fn remove(run_id: &str) {
        contexts().lock().expect("登记表锁不应毒化").remove(run_id);
    }

    /// 未登记（实例级/纯委派/进程重启）→ P3 默认，零 DB 读。
    pub fn limits_for_run(run_id: Option<&str>) -> QuotaLimits {
        run_id.and_then(Self::get).unwrap_or_else(QuotaLimits::p3_defaults)
    }
}

/// 本 run 的单轮超时封顶；无登记时原样返回（P3 行为不变）。
pub fn cap_turn_timeout(run_id: Option<&str>, requested: Duration) -> Duration {
    match run_id.and_then(AssignmentContext::get) {
        Some(limits) => limits.cap_turn(requested),
        None => requested,
    }
}

/// 该编排的部门配额行（仅部门作用域；实例级返回 None 且不查库）。
pub async fn effective_quota(
    db: &DatabaseConnection,
    workflow: &BotWorkflow,
) -> Result<Option<BotDepartmentQuota>, AxAgentError> {
    let Some(department_id) = quota_department(workflow) else {
        return Ok(None);
    };
    quota_repo::get_by_department(db, department_id).await
}

/// run 创建时解析一次生效上限（ADR-P4-8）。实例级/无配额行 = P3 默认。
pub async fn resolve_workflow_limits(
    db: &DatabaseConnection,
    workflow: &BotWorkflow,
) -> Result<QuotaLimits, AxAgentError> {
    let base = QuotaLimits::from_workflow(workflow);
    Ok(match effective_quota(db, workflow).await? {
        Some(quota) => base.tighten(&quota),
        None => base,
    })
}

/// 定义保存前校验（create / update）：部门作用域且存在配额行时才检查。
pub async fn check_definition_allowed(
    db: &DatabaseConnection,
    department_id: &str,
    max_concurrency: i32,
    total_timeout_secs: i32,
    steps: &[BotWorkflowStep],
) -> Result<(), String> {
    let Some(quota) =
        quota_repo::get_by_department(db, department_id).await.map_err(quota_conflict)?
    else {
        return Ok(());
    };
    validate_workflow_against_quota(max_concurrency, total_timeout_secs, steps, &quota)
        .map_err(|v| v.response())
}

/// run 触发前校验：定义不超配额 + 部门同时在跑编排数未达上限。
/// `exclude_run_id` 用于 start_run 复核时排除本 run 自身。
pub async fn check_run_allowed(
    db: &DatabaseConnection,
    workflow: &BotWorkflow,
    exclude_run_id: Option<&str>,
) -> Result<QuotaLimits, String> {
    let base = QuotaLimits::from_workflow(workflow);
    let Some(department_id) = quota_department(workflow) else {
        return Ok(base);
    };
    let Some(quota) =
        quota_repo::get_by_department(db, department_id).await.map_err(quota_conflict)?
    else {
        return Ok(base);
    };
    validate_workflow_against_quota(
        workflow.max_concurrency,
        workflow.total_timeout_secs,
        &workflow.steps,
        &quota,
    )
    .map_err(|v| v.response())?;
    let active = run_repo::count_active_runs_by_department(db, department_id, exclude_run_id)
        .await
        .map_err(quota_conflict)?;
    if active as i32 >= quota.max_concurrent_runs {
        return Err(QuotaViolation::new(
            "maxConcurrentRuns",
            active as i64 + 1,
            quota.max_concurrent_runs as i64,
        )
        .with_note("部门同时在运行的编排数已达上限")
        .response());
    }
    Ok(base.tighten(&quota))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axagent_harness::types::BotDepartmentQuota;

    fn quota_row(dept: &str) -> BotDepartmentQuota {
        BotDepartmentQuota {
            department_id: dept.into(),
            max_concurrency: 1,
            total_timeout_secs: 300,
            max_steps: 3,
            max_in_flight_per_depth: 1,
            step_timeout_secs: 60,
            max_concurrent_runs: 1,
            is_default: false,
            updated_at: Some(1),
        }
    }

    fn step(key: &str, timeout_secs: i32) -> BotWorkflowStep {
        BotWorkflowStep {
            step_key: key.into(),
            bot_id: "bot".into(),
            prompt_template: String::new(),
            depends_on: Vec::new(),
            timeout_secs,
            on_failure: "abort".into(),
        }
    }

    #[test]
    fn defaults_equal_p3_constants() {
        let d = QuotaLimits::p3_defaults();
        assert_eq!(d.max_concurrency, crate::bot_workflow_runner::MAX_CONCURRENCY);
        assert_eq!(d.total_timeout_secs, crate::bot_workflow_runner::TOTAL_TIMEOUT_SECS);
        assert_eq!(d.max_steps as usize, crate::bot_workflow_runner::MAX_STEPS);
        assert_eq!(
            d.max_in_flight_per_depth as i64,
            crate::bot_assignment_runner::MAX_IN_FLIGHT_PER_DEPTH
        );
        assert_eq!(d.step_timeout_secs as u64, crate::bot_assignment_runner::DISPATCH_TIMEOUT_SECS);
    }

    #[test]
    fn tighten_takes_min_per_field() {
        let base = QuotaLimits {
            max_concurrency: 4,
            total_timeout_secs: 900,
            max_steps: 12,
            max_in_flight_per_depth: 2,
            step_timeout_secs: 300,
            max_concurrent_runs: 8,
        };
        let tight = base.tighten(&quota_row("d"));
        assert_eq!(tight.max_concurrency, 1);
        assert_eq!(tight.total_timeout_secs, 300);
        assert_eq!(tight.max_steps, 3);
        assert_eq!(tight.max_in_flight_per_depth, 1);
        assert_eq!(tight.step_timeout_secs, 60);
        assert_eq!(tight.max_concurrent_runs, 1);
        let loose = QuotaLimits { max_concurrency: 1, total_timeout_secs: 60, ..base.clone() };
        let merged = loose.tighten(&quota_row("d"));
        assert_eq!(merged.max_concurrency, 1);
        assert_eq!(merged.total_timeout_secs, 60);
    }

    #[test]
    fn over_quota_reports_field_and_values() {
        let q = quota_row("d");
        let err =
            validate_workflow_against_quota(4, 900, &[step("a", 30)], &q).expect_err("并发超限");
        assert_eq!(err.field, "maxConcurrency");
        let raw = err.response();
        assert!(raw.contains("BOT_QUOTA_EXCEEDED"));
        assert!(raw.contains("maxConcurrency"));
        assert!(raw.contains('4') && raw.contains('1'), "detail 带定义值与上限: {raw}");

        let err =
            validate_workflow_against_quota(1, 900, &[step("a", 30)], &q).expect_err("总超时超限");
        assert_eq!(err.field, "totalTimeoutSecs");

        let steps = vec![step("a", 30), step("b", 30), step("c", 30), step("d", 30)];
        let err = validate_workflow_against_quota(1, 300, &steps, &q).expect_err("步数超限");
        assert_eq!(err.field, "maxSteps");

        let err = validate_workflow_against_quota(1, 300, &[step("research", 600)], &q)
            .expect_err("单步超时超限");
        assert_eq!(err.field, "stepTimeoutSecs");
        assert!(err.detail().contains("research"));
    }

    #[test]
    fn within_quota_passes() {
        let q = quota_row("d");
        validate_workflow_against_quota(1, 300, &[step("a", 60)], &q).expect("未超限应通过");
    }

    #[test]
    fn context_registry_roundtrip() {
        let limits = QuotaLimits { max_concurrency: 1, ..QuotaLimits::p3_defaults() };
        AssignmentContext::install("run-x", limits.clone());
        assert_eq!(AssignmentContext::get("run-x"), Some(limits.clone()));
        assert_eq!(AssignmentContext::limits_for_run(Some("run-x")), limits);
        assert_eq!(AssignmentContext::limits_for_run(None), QuotaLimits::p3_defaults());
        assert_eq!(AssignmentContext::limits_for_run(Some("missing")), QuotaLimits::p3_defaults());
        AssignmentContext::remove("run-x");
        assert_eq!(AssignmentContext::get("run-x"), None);
    }

    #[test]
    fn cap_turn_uses_registered_step_timeout() {
        let limits = QuotaLimits { step_timeout_secs: 60, ..QuotaLimits::p3_defaults() };
        AssignmentContext::install("run-cap", limits);
        assert_eq!(
            cap_turn_timeout(Some("run-cap"), Duration::from_secs(300)),
            Duration::from_secs(60)
        );
        assert_eq!(
            cap_turn_timeout(Some("run-cap"), Duration::from_secs(30)),
            Duration::from_secs(30)
        );
        assert_eq!(
            cap_turn_timeout(None, Duration::from_secs(300)),
            Duration::from_secs(300),
            "无登记时 P3 行为不变"
        );
        AssignmentContext::remove("run-cap");
    }
}
