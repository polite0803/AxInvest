// SPDX-License-Identifier: AGPL-3.0-only

//! `workflow.run` 接缝的**宿主侧实现**（wiring 层，宿主独持 `WorkEngine`）。
//!
//! 契约在 `axagent-core/crates/harness/src/workflow_run.rs`（已冻结）：`start` 只受理并
//! **立即**返回 `execution_id`（接缝等待预算 15 s，绝不能 await DAG），执行体在
//! `tokio::spawn` 里跑；进度与终态经 `event.dispatch` 推给插件；`status` / `cancel` /
//! `list_active` 一律按 owner 隔离（跨 owner 视同查无）。
//!
//! ## 宿主自决的执行参数（插件既不给也不该读回）
//!
//! | 参数 | 来源 | 理由 |
//! |---|---|---|
//! | `input_schema` / `output_schema` | 模板行（`load_and_inject_template`） | 插件改 schema = 绕过校验 |
//! | `tool_permissions`（strict） | `stock_workflow::strict_tool_permissions()` | 宿主业务策略，三道 strict 防线 |
//! | `as_of` | `WorkflowRunRequest::as_of_date` → `AS_OF` scope + `node_task_scope` | 双层缺一即「回放跑成实盘」 |
//! | `execution_id` | 宿主预置（插件否则永远拿不到 id） | 引擎自 mint 的 id 不回传 |
//! | 并发/超时四面 | 模板变量（`ExecutionParams`）+ 请求覆盖 | 与宿主既有 5 个调用点同源 |
//!
//! ## 事件载荷（`event.dispatch`，`DispatchMode::Emit`）
//!
//! - `kind = "workflow/step"`：`{ executionId, workflowId, seq, event: <StepProgressEvent> }`
//! - `kind = "workflow/end"` ：`{ executionId, workflowId, seq, status, error }`
//!
//! 外层键沿用 `DomainEvent` 自身的 camelCase 外壳（`harness/src/event_bus.rs:43-57`）；
//! 内嵌的 `event` 是引擎原生 DTO，序列化即 `StepProgressEvent` 的 snake_case 字段。
//! `streaming` 增量**一律丢弃**（与 `stock_workflow/serenity.rs:1217-1219` 同口径），
//! 派发**一律 fire-and-forget**（`Emit` 是串行 await，见 `typed_event.rs:207-212`，
//! 直接 await 会把插件 RTT 塞进节点调度热路径）。
//!
//! ## 首版刻意不做（对齐契约注释）
//!
//! - 不做跨宿主重启的 `status` 兜底（DB 里可能残留 `running` 幽灵行）；
//! - 不做「插件卸载即取消」（登记保留，执行仍归该 owner）；
//! - 不实现通用（非 `stock-*`）模板的业务注入 —— 白名单当前只开出 `stock-` 一族。

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use axagent_harness::workflow_types::{ExecutionStatus, Variable, WorkflowStatus};
use axagent_harness::{
    AsOfContext, AxAgentError, DispatchMode, DomainEvent, EventCategory, EventDispatchBus,
    ExecutionView, WorkflowRunActivePage, WorkflowRunActiveView, WorkflowRunCancelOutcome,
    WorkflowRunHandle, WorkflowRunProvider, WorkflowRunRequest,
};
use axagent_rt_workflow::work_engine::engine::node_task_scope_hook;
use axagent_rt_workflow::work_engine::{
    ProgressCallback, RunOptions, StepProgressEvent, WorkEngine, WorkEngineError,
};
use sea_orm::DatabaseConnection;
use serde_json::json;
use tokio::sync::RwLock;

use crate::commands::stock_workflow::decision::{ExecutionParams, load_and_inject_template};

/// 可运行模板的**前缀**白名单（唯一来源）。
///
/// 当前只开出股票族（`stock-analysis` / `stock-reflection` / `stock-pipeline` /
/// `stock-serenity-*` 等）。将来放开别的模板族时**只改这一处**：
/// `start` 的前置校验、`report` 里的说明都读它，不要在别处再写一份字符字面量。
const ALLOWED_TEMPLATE_PREFIXES: &[&str] = &["stock-"];

/// 需要宿主做业务注入（种子兜底 + Trigger 节点 `stock_code` 改写 + schema/variables/hooks）
/// 的模板前缀。与 `ALLOWED_TEMPLATE_PREFIXES` 不同：前者是「能不能跑」，本常量是「按哪条路载图」。
const STOCK_TEMPLATE_PREFIX: &str = "stock-";

/// 进度事件 kind（沿用 `agent/request` / `llm/stream` 的斜杠命名）。
const EVENT_STEP_KIND: &str = "workflow/step";
/// 终态事件 kind（执行体收尾补发；引擎自己的 `ProgressBrief` 不在插件总线上）。
const EVENT_END_KIND: &str = "workflow/end";
/// 事件来源标签（插件按 `source` 可粗筛宿主产生的执行事件）。
const EVENT_SOURCE: &str = "workflow.run";
/// 宿主侧「总超时」缺省值（秒）。与 `decision.rs` 的 `DEFAULT_TOTAL_TIMEOUT_SECS`
/// 同值（1800s）：引擎自己不实现总超时，缺了它插件可以起一次永不收尾的执行。
const DEFAULT_TOTAL_TIMEOUT_SECS: u64 = 1800;
/// `total_timeout_secs` 可接受上限（与 `ExecutionParams` 的 clamp 区间一致）。
const MAX_TOTAL_TIMEOUT_SECS: u64 = 7200;
/// `list_active` 的页大小上限（宿主再夹一层，防插件一次拉爆内存）。
const MAX_ACTIVE_PAGE: usize = 200;

/// 准入上限（per-owner / 全局「在跑数」）。
///
/// 引擎没有任何全局并发/预算闸（`RunOptions.max_concurrent` 只是单次执行内的节点并发），
/// 故准入只能建在接缝层：一个插件最多同时跑 2 个，全宿主最多 8 个。
#[derive(Debug, Clone, Copy)]
struct RunLimits {
    per_owner: usize,
    global: usize,
}

impl Default for RunLimits {
    fn default() -> Self {
        Self { per_owner: 2, global: 8 }
    }
}

/// 单次执行的宿主侧登记项（`execution_id` → 本结构）。
///
/// 存在理由：`ActiveExecutionInfo` 没有 owner 字段，引擎也从不清理 `executions`，
/// 所以「谁发起的」「配额是否已释放」「终态要不要覆盖」只能登记在接缝层。
/// **插件卸载不删登记**（执行仍归该 owner，`status` 仍可查）。
#[derive(Debug, Clone)]
struct RunMeta {
    owner: String,
    /// 本次执行的 workflow id（`create_workflow_with_hooks` / `load_workflow_template`
    /// 产出）。受理时为空串，建图后回填 —— 回填前 `status` 尚未对插件可见。
    workflow_id: String,
    /// 受理时刻（毫秒），用于 `list_active` 的次级排序与运维排查。
    created_at: i64,
    /// 是否仍占着一个准入额度（受理即 true，执行体收尾即 false）。
    active: bool,
    /// 宿主侧终态覆盖：目前只有「总超时 ⇒ failed」与「执行体异常终止 ⇒ failed」两种。
    /// 引擎的 `cancel` 会无条件把状态覆写成 `cancelled`（`engine/mod.rs:5313`），
    /// 不覆盖的话插件看到的总超时会被误判成「用户取消」。
    terminal_override: Option<ExecutionStatus>,
}

/// 登记表 + 配额计数（同一把锁内改，避免「判上限」与「计数」分叉）。
#[derive(Debug, Default)]
struct RunRegistry {
    runs: HashMap<String, RunMeta>,
    per_owner_active: HashMap<String, usize>,
    total_active: usize,
}

/// 准入配额守卫：受理即占额，执行体结束（正常 / 失败 / 超时 / **panic**）即还额。
///
/// 正常路径由执行体末尾 `release_slot(..)` 显式释放；本守卫的 `Drop` 兜的是
/// 「执行体 panic / 提前 return」—— 漏还一次就会让该 owner 永久少一个额度
/// （per-owner 默认 2，两次漏还即该插件再也起不了执行，且只有重启能恢复）。
struct ActiveRunGuard {
    runs: Arc<RwLock<RunRegistry>>,
    execution_id: String,
    owner: String,
}

impl Drop for ActiveRunGuard {
    fn drop(&mut self) {
        // `Drop` 不能 await：扔给 runtime 异步释放；运行时不在了（进程退出）则无所谓。
        let Ok(handle) = tokio::runtime::Handle::try_current() else {
            return;
        };
        let runs = Arc::clone(&self.runs);
        let execution_id = std::mem::take(&mut self.execution_id);
        let owner = std::mem::take(&mut self.owner);
        handle.spawn(async move {
            release_slot(&runs, &execution_id, &owner, Some(ExecutionStatus::Failed)).await;
        });
    }
}

/// 释放一次准入额度（幂等）。
///
/// `terminal` 只在**本次真的从 active → inactive** 时写入覆盖：
/// 正常收尾已显式释放过，随后 `ActiveRunGuard::drop` 再进来只会命中「已释放」而
/// 直接返回 —— 否则它会把一次成功执行的终态覆盖成 `failed`。
async fn release_slot(
    runs: &Arc<RwLock<RunRegistry>>,
    execution_id: &str,
    owner: &str,
    terminal: Option<ExecutionStatus>,
) {
    let mut reg = runs.write().await;
    let Some(meta) = reg.runs.get_mut(execution_id) else {
        return;
    };
    if !meta.active {
        return;
    }
    meta.active = false;
    if let Some(t) = terminal {
        meta.terminal_override = Some(t);
    }
    if let Some(n) = reg.per_owner_active.get_mut(owner) {
        *n = n.saturating_sub(1);
    }
    reg.total_active = reg.total_active.saturating_sub(1);
}

/// 把 `WorkflowStatus`（引擎视角，含 `Created`）折成 `ExecutionStatus`（六态运行视角）。
///
/// `Created` 在执行结束时不该出现；与引擎收尾的 `_ => Completed` 同口径兜底
/// （`engine/mod.rs:5019-5024`），不新造第七态。
fn execution_status_of(status: WorkflowStatus) -> ExecutionStatus {
    match status {
        WorkflowStatus::Completed | WorkflowStatus::Created | WorkflowStatus::Running => {
            ExecutionStatus::Completed
        },
        WorkflowStatus::PartiallyCompleted => ExecutionStatus::PartiallyCompleted,
        WorkflowStatus::Failed => ExecutionStatus::Failed,
        WorkflowStatus::Cancelled => ExecutionStatus::Cancelled,
    }
}

/// 模板是否在白名单内（纯函数，便于单测锁住「只开 stock- 一族」的口径）。
fn is_template_allowed(template_id: &str) -> bool {
    ALLOWED_TEMPLATE_PREFIXES.iter().any(|p| template_id.starts_with(p))
}

/// 从 `input` 取 `stock_code`（`stock-*` 模板的硬前置）。
fn extract_stock_code(input: Option<&serde_json::Value>) -> Result<String, AxAgentError> {
    input
        .and_then(|v| v.get("stock_code"))
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .ok_or_else(|| {
            AxAgentError::Validation(
                "stock-* 模板必须提供 input.stock_code（宿主据此做种子兜底与 Trigger 节点注入）"
                    .into(),
            )
        })
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

/// 合并变量：**模板为底 + 请求按变量名覆盖**。
///
/// 为什么不直接「请求非空就整表替换」：模板变量不是装饰，它承载着
/// `max_concurrent` / `agent_timeout_secs` / `tool_timeout_secs` / 各 `input_mapping`
/// 兜底值等业务参数（`decision.rs:2782-2816`）—— 整表替换会让插件只传一个变量时
/// 其余参数**静默退回模块常量**（「面板上改了、实际没生效」那族缺陷）。
/// 故按 `name` 覆盖、模板顺序保留、请求新增的追加到尾部。
fn merge_variables(base: Option<Vec<Variable>>, overrides: &[Variable]) -> Option<Vec<Variable>> {
    match base {
        None if overrides.is_empty() => None,
        None => Some(overrides.to_vec()),
        Some(mut base) => {
            for v in overrides {
                match base.iter_mut().find(|b| b.name == v.name) {
                    Some(slot) => *slot = v.clone(),
                    None => base.push(v.clone()),
                }
            }
            Some(base)
        },
    }
}

/// 派发一个领域事件（fire-and-forget）。
///
/// 无匹配订阅者时直接返回（`would_dispatch` 短路，避免为空总线白做 JSON 序列化与 spawn）。
fn dispatch_event(dispatcher: &Arc<EventDispatchBus>, kind: &str, payload: serde_json::Value) {
    let mut event = DomainEvent::new(EventCategory::Workflow, kind, payload, EVENT_SOURCE);
    if !dispatcher.would_dispatch(&event) {
        return;
    }
    let dispatcher = Arc::clone(dispatcher);
    tokio::spawn(async move {
        let outcome = dispatcher.dispatch(&mut event, DispatchMode::Emit).await;
        tracing::trace!(
            invoked = outcome.invoked,
            rejected = outcome.rejected,
            "[workflow.run] 进度/终态事件已派发"
        );
    });
}

/// `workflow.run` 的宿主实现。
pub struct WorkflowRunService {
    /// 宿主独占的引擎（与命令层、触发器共用同一个 Arc）。
    engine: Arc<WorkEngine>,
    /// 仅供 `load_and_inject_template` 读模板行（种子兜底 + 业务注入）。
    db: Arc<DatabaseConnection>,
    /// `event.dispatch` 内置总线（与插件订阅者同一个对象）。
    dispatcher: Arc<EventDispatchBus>,
    /// `execution_id → 归属/配额/终态覆盖`。
    runs: Arc<RwLock<RunRegistry>>,
    limits: RunLimits,
}

impl WorkflowRunService {
    pub fn new(
        engine: Arc<WorkEngine>,
        db: Arc<DatabaseConnection>,
        dispatcher: Arc<EventDispatchBus>,
    ) -> Self {
        Self {
            engine,
            db,
            dispatcher,
            runs: Arc::new(RwLock::new(RunRegistry::default())),
            limits: RunLimits::default(),
        }
    }

    /// 准入判定 + 登记（同一次写锁内完成「判上限 / 计数 +1 / 落登记」）。
    ///
    /// 超限返回 [`AxAgentError::Validation`]（分派层将来映射 `WORKFLOW_RUN_REJECTED`：
    /// 语义上必须区别于「执行失败」—— 前者可稍后重试，后者是配置错）。
    async fn admit(&self, execution_id: &str, owner: &str) -> Result<ActiveRunGuard, AxAgentError> {
        let mut reg = self.runs.write().await;
        let owner_active = reg.per_owner_active.get(owner).copied().unwrap_or(0);
        if owner_active >= self.limits.per_owner {
            // 拒绝信息带「最久那次已跑多久」：并发被拒多数是上行执行没退出，
            // 光说「已达上限 2」无法判断是卡死还是正常排队（`created_at` 的唯一消费点）。
            let oldest_ms = reg
                .runs
                .values()
                .filter(|m| m.owner == owner && m.active)
                .map(|m| now_ms().saturating_sub(m.created_at))
                .max()
                .unwrap_or(0);
            return Err(AxAgentError::Validation(format!(
                "workflow.run 并发上限：owner `{owner}` 在跑 {owner_active} 次，已达每 owner 上限 {}（最久一次已跑 {oldest_ms}ms；等前一次收尾后重试）",
                self.limits.per_owner
            )));
        }
        if reg.total_active >= self.limits.global {
            return Err(AxAgentError::Validation(format!(
                "workflow.run 全局并发上限：在跑 {} 次，已达上限 {}（等前一次收尾后重试）",
                reg.total_active, self.limits.global
            )));
        }
        reg.per_owner_active.insert(owner.to_string(), owner_active + 1);
        reg.total_active += 1;
        reg.runs.insert(
            execution_id.to_string(),
            RunMeta {
                owner: owner.to_string(),
                workflow_id: String::new(),
                created_at: now_ms(),
                active: true,
                terminal_override: None,
            },
        );
        drop(reg);
        Ok(ActiveRunGuard {
            runs: Arc::clone(&self.runs),
            execution_id: execution_id.to_string(),
            owner: owner.to_string(),
        })
    }

    /// 建图后回填 `workflow_id`（受理时还不知道它）。
    async fn bind_workflow_id(&self, execution_id: &str, workflow_id: &str) {
        let mut reg = self.runs.write().await;
        if let Some(meta) = reg.runs.get_mut(execution_id) {
            meta.workflow_id = workflow_id.to_string();
        }
    }

    /// 组装进度回调：`StepProgressEvent` → `DomainEvent(workflow/step)`。
    ///
    /// `seq` 与终态事件共用同一个计数器 ⇒ 插件可据它丢陈旧（派发全是 spawn，
    /// 到达顺序无保证；`Emit` 又是串行 await，更不能阻塞节点调度）。
    fn progress_callback(
        &self,
        execution_id: &str,
        workflow_id: &str,
        seq: Arc<AtomicU64>,
    ) -> ProgressCallback {
        let dispatcher = Arc::clone(&self.dispatcher);
        let execution_id = execution_id.to_string();
        let workflow_id = workflow_id.to_string();
        Arc::new(move |event: StepProgressEvent| {
            let dispatcher = Arc::clone(&dispatcher);
            let execution_id = execution_id.clone();
            let workflow_id = workflow_id.clone();
            let seq = Arc::clone(&seq);
            Box::pin(async move {
                // 丢弃 streaming 增量（AgentExecutor 每 2s 一次）：过 seam 会放大
                // 15s 预算与插件忙时背压，Serenity 自己也已经把它丢掉。
                if event.status == "streaming" {
                    return;
                }
                let n = seq.fetch_add(1, Ordering::Relaxed);
                dispatch_event(
                    &dispatcher,
                    EVENT_STEP_KIND,
                    json!({
                        "executionId": execution_id,
                        "workflowId": workflow_id,
                        "seq": n,
                        "event": event,
                    }),
                );
            })
        })
    }
}

#[async_trait]
impl WorkflowRunProvider for WorkflowRunService {
    async fn start(
        &self,
        owner: &str,
        request: WorkflowRunRequest,
    ) -> Result<WorkflowRunHandle, AxAgentError> {
        // ① 模板白名单（唯一来源 `ALLOWED_TEMPLATE_PREFIXES`）
        if !is_template_allowed(&request.template_id) {
            return Err(AxAgentError::Validation(format!(
                "模板 `{}` 不在 workflow.run 可运行白名单内（当前仅允许前缀 {ALLOWED_TEMPLATE_PREFIXES:?}）",
                request.template_id
            )));
        }

        // ② as_of：宿主兜底解析（非法日期 / 未来日期都当场拒，早于任何调度）
        let as_of_ctx = AsOfContext::parse_optional(request.as_of_date.as_deref())
            .map_err(AxAgentError::Validation)?;

        // ③ stock-* 的业务前置入参
        let is_stock = request.template_id.starts_with(STOCK_TEMPLATE_PREFIX);
        let stock_code = if is_stock {
            Some(extract_stock_code(request.input.as_ref())?)
        } else {
            None
        };

        // ④ execution_id 由宿主预置（引擎自 mint 的 id 不回传，插件永远拿不到）
        let execution_id = uuid::Uuid::new_v4().to_string();

        // ⑤ 准入 + 登记（超限即拒；守卫在失败/panic 路径上也会还额）
        let guard = self.admit(&execution_id, owner).await?;

        // ⑥ 载图：stock-* 复用宿主既有业务注入（种子兜底 + Trigger 节点 stock_code 改写 +
        //    schema/variables/hooks）；通用模板走引擎 `load_workflow_template`（无 schema）。
        let mut input = request.input.clone().unwrap_or_else(|| json!({}));
        // 与对话直执行链同键名：enhance 钩子按 `input.as_of_date` 取基准日
        // （`hooks.rs:1125`）。请求里显式给的 as_of 必须让钩子也看到，否则钩子按 live 算。
        if let Some(as_of) = as_of_ctx.map(|c| c.as_string())
            && input.get("as_of_date").is_none_or(serde_json::Value::is_null)
        {
            input["as_of_date"] = serde_json::Value::String(as_of);
        }

        let workflow = if let Some(stock_code) = stock_code.as_deref() {
            let stock_name = input.get("stock_name").and_then(|v| v.as_str()).unwrap_or_default();
            let loaded = load_and_inject_template(
                self.db.as_ref(),
                stock_code,
                stock_name,
                &request.template_id,
            )
            .await
            .map_err(AxAgentError::Provider)?;
            // 与 `core.rs:822` / `reflection.rs:508` / `serenity.rs:1040` 同款：把模板变量里的
            // `vendor_*` 密钥/代理注入进程级客户端。少了这一步，走接缝的首次股票执行可能
            // 「跑得动但取不到数」（客户端的 vendor 启用状态还是空的）。
            if let Some(client) = axagent_tools::global_state::get_astock_client() {
                crate::commands::stock_workflow::decision::inject_vendor_state(
                    client.as_ref(),
                    loaded.variables.as_ref(),
                );
            }
            let name = format!("{}-plugin-{owner}", request.template_id);
            // 必须带 hooks_config，否则 stock-analysis-enhance 不跑、market_regime 等
            // 业务变量永不注入（`decision.rs:334-337` 的实证说明）。
            let wf = self
                .engine
                .create_workflow_with_hooks(&name, loaded.nodes, loaded.edges, loaded.hooks_config)
                .await
                .map_err(|e| AxAgentError::Provider(format!("建图失败: {e}")))?;
            (wf, loaded.input_schema, loaded.output_schema, loaded.variables)
        } else {
            // ⚠ 当前白名单只开 stock- 一族 ⇒ 本分支暂不可达；保留是为「将来扩前缀」时
            //    不必重写 start（通用模板无 schema / 无业务注入，由引擎按 template_id 载图）。
            let wf = self
                .engine
                .load_workflow_template(&request.template_id)
                .await
                .map_err(|e| AxAgentError::Provider(format!("模板装载失败: {e}")))?;
            (wf, None, None, None)
        };
        let (workflow, input_schema, output_schema, template_vars) = workflow;
        let workflow_id = workflow.id.clone();

        // ⑦ 回填 workflow_id（受理时还不知道）
        self.bind_workflow_id(&execution_id, &workflow_id).await;

        // ⑧ 变量：模板为底 + 请求按名覆盖（见 `merge_variables` 的理由）；
        //    四面默认按**合并后**的变量解析（插件给的 `max_concurrent` 等因此能生效）。
        let variables = merge_variables(template_vars, &request.variables);
        let exec = ExecutionParams::from_template_vars(variables.as_deref());
        let total_timeout_secs = request
            .total_timeout_secs
            .unwrap_or(DEFAULT_TOTAL_TIMEOUT_SECS)
            .clamp(1, MAX_TOTAL_TIMEOUT_SECS);

        // ⑨ 组装 RunOptions（schema / strict 权限 / as_of 双包裹都由宿主自决）
        let seq = Arc::new(AtomicU64::new(0));
        let mut opts = RunOptions {
            execution_id: Some(execution_id.clone()),
            input: Some(input),
            input_schema,
            output_schema,
            variables,
            dry_run: request.dry_run,
            tool_permissions: if is_stock {
                Some(crate::commands::stock_workflow::strict_tool_permissions())
            } else {
                None
            },
            progress_callback: Some(self.progress_callback(
                &execution_id,
                &workflow_id,
                Arc::clone(&seq),
            )),
            // 节点级 as_of：引擎为每个节点单独 spawn，task_local 不跨 spawn ⇒ 必须逐节点罩
            // （与 `stock_workflow/serenity.rs:1267-1272` 同款）。缺它 = 节点取数丢 as_of。
            node_task_scope: Some(node_task_scope_hook(move |fut| {
                axagent_astock_data::as_of::with_optional_asof(as_of_ctx, fut)
            })),
            model_id: request.model_id,
            provider_id: request.provider_id,
            ..Default::default()
        };
        exec.apply_to(&mut opts);
        if let Some(n) = request.max_concurrent {
            opts = opts.with_max_concurrent(n.clamp(1, 32) as usize);
        }
        if let Some(secs) = request.step_timeout_secs {
            opts.step_timeout = Duration::from_secs(secs.max(1));
        }
        if let Some(secs) = request.tool_timeout_secs {
            opts.tool_timeout = Duration::from_secs(secs.max(1));
        }

        // ⑩ 执行体：`tokio::spawn` + 总超时 + as_of 外层包裹（三重都在这一处收口）
        let engine = Arc::clone(&self.engine);
        let dispatcher = Arc::clone(&self.dispatcher);
        let runs = Arc::clone(&self.runs);
        let exec_id = execution_id.clone();
        let wf_id = workflow_id.clone();
        let owner_owned = owner.to_string();
        tokio::spawn(async move {
            // 守卫随任务移动：正常收尾由 release_slot 显式还额，`Drop` 兜 panic 路径
            let _guard = guard;
            let engine_run = Arc::clone(&engine);
            // as_of 外层包裹：进程级全局栈 + task_local（RAII 压栈/按 token 移除）。
            // 总超时（`clamp(1, 7200)` 后恒 > 0）再包一层：引擎自己**不实现**总超时
            // （`TaskContract.sla_max_seconds` 只在收尾事后盖章，`engine/mod.rs:5067-5068`）。
            let fut = axagent_astock_data::as_of::with_optional_asof(
                as_of_ctx,
                engine_run.run_workflow(&wf_id, opts),
            );
            let outcome =
                match tokio::time::timeout(Duration::from_secs(total_timeout_secs), fut).await {
                    Ok(result) => result.map(Some),
                    Err(_) => {
                        // 到点：先让引擎协作退出（cancel 会写 DB cancelled），再按 failed 收尾
                        if let Err(e) = engine.cancel(&exec_id).await {
                            tracing::warn!(
                                execution_id = %exec_id,
                                "[workflow.run] 总超时后取消失败: {e}"
                            );
                        }
                        Ok(None)
                    },
                };

            let (status, error) = match outcome {
                Ok(Some(wf)) => (execution_status_of(wf.status), None),
                Ok(None) => (
                    ExecutionStatus::Failed,
                    Some(format!("执行超过总超时 {total_timeout_secs}s，宿主已取消")),
                ),
                Err(e) => (ExecutionStatus::Failed, Some(e.to_string())),
            };

            // ⑪ 还额（显式；幂等）+ 补发终态事件
            let terminal = if status == ExecutionStatus::Failed {
                Some(ExecutionStatus::Failed)
            } else {
                None
            };
            release_slot(&runs, &exec_id, &owner_owned, terminal).await;
            let n = seq.fetch_add(1, Ordering::Relaxed);
            dispatch_event(
                &dispatcher,
                EVENT_END_KIND,
                json!({
                    "executionId": exec_id,
                    "workflowId": wf_id,
                    "seq": n,
                    "status": status.to_string(),
                    "error": error,
                }),
            );
            tracing::info!(
                execution_id = %exec_id,
                workflow_id = %wf_id,
                owner = %owner_owned,
                status = %status,
                error = ?error,
                "[workflow.run] 执行体收尾"
            );
        });

        // ⑫ 立即返回句柄（不等 DAG；接缝预算 15s）
        Ok(WorkflowRunHandle { execution_id, workflow_id, accepted_at: now_ms() })
    }

    async fn status(
        &self,
        owner: &str,
        execution_id: &str,
        include_output: bool,
    ) -> Result<Option<ExecutionView>, AxAgentError> {
        // owner 隔离：跨 owner 一律按查无处理（不泄漏他人执行的存在性）
        let meta = {
            let reg = self.runs.read().await;
            reg.runs.get(execution_id).filter(|m| m.owner == owner).cloned()
        };
        let Some(meta) = meta else {
            return Ok(None);
        };

        let state = match self.engine.get_status(execution_id).await {
            Ok(state) => state,
            // 宿主重启后引擎内存已丢 ⇒ 与「查无」同语义（不查 DB 兜底，避免幽灵 running）
            Err(WorkEngineError::NotFound(_)) => return Ok(None),
            Err(e) => return Err(AxAgentError::Provider(format!("查询执行状态失败: {e}"))),
        };

        // 终态覆盖：总超时 / 执行体异常终止时引擎那边可能是 cancelled，
        // 对插件必须呈现为 failed（契约：超时 = failed + reason=timeout）。
        let status = meta.terminal_override.clone().unwrap_or(state.status);
        let workflow_id = if meta.workflow_id.is_empty() {
            state.workflow_id.clone()
        } else {
            meta.workflow_id.clone()
        };
        let output = if include_output {
            self.engine.get_execution_output(execution_id).await
        } else {
            None
        };

        Ok(Some(ExecutionView {
            execution_id: execution_id.to_string(),
            workflow_id,
            status,
            current_node_id: state.current_node_id,
            total_time_ms: (state.updated_at - state.created_at).max(0) as u64,
            created_at: state.created_at,
            updated_at: state.updated_at,
            output,
        }))
    }

    async fn cancel(
        &self,
        owner: &str,
        execution_id: &str,
    ) -> Result<WorkflowRunCancelOutcome, AxAgentError> {
        // owner 隔离：跨 owner 视同查无
        let meta = {
            let reg = self.runs.read().await;
            reg.runs.get(execution_id).filter(|m| m.owner == owner).cloned()
        };
        let Some(meta) = meta else {
            return Ok(WorkflowRunCancelOutcome { cancelled: false, status: None });
        };

        // **先查状态**再决定是否调引擎：`engine.cancel` 不校验当前状态，
        // 对已完成执行会把 `completed` 无条件覆写成 `cancelled`（内存 + DB 双写，
        // 见 `engine/mod.rs:5313-5315`）—— 这是接缝最需要防的一条。
        let state = match self.engine.get_status(execution_id).await {
            Ok(state) => state,
            Err(WorkEngineError::NotFound(_)) => {
                return Ok(WorkflowRunCancelOutcome { cancelled: false, status: None });
            },
            Err(e) => return Err(AxAgentError::Provider(format!("取消前查询状态失败: {e}"))),
        };
        let current = meta.terminal_override.clone().unwrap_or(state.status);
        if !matches!(current, ExecutionStatus::Running | ExecutionStatus::Paused) {
            // 幂等：终态执行回现值，**绝不调引擎**
            return Ok(WorkflowRunCancelOutcome { cancelled: false, status: Some(current) });
        }

        self.engine
            .cancel(execution_id)
            .await
            .map_err(|e| AxAgentError::Provider(format!("取消失败: {e}")))?;
        // 配额不在这里还：DAG 退出后由执行体还（同一 run 只还一次，见 `release_slot` 幂等）
        Ok(WorkflowRunCancelOutcome { cancelled: true, status: Some(ExecutionStatus::Cancelled) })
    }

    async fn list_active(
        &self,
        owner: &str,
        workflow_id: Option<&str>,
        limit: usize,
    ) -> Result<WorkflowRunActivePage, AxAgentError> {
        let limit = limit.clamp(1, MAX_ACTIVE_PAGE);
        // 引擎侧一次内存遍历（只含 running / paused），再用登记表按 owner 过滤：
        // `ActiveExecutionInfo` 没有 owner 字段，不过滤就会把别的插件的执行列出来。
        let active = self.engine.list_active_executions().await;
        let mut views: Vec<WorkflowRunActiveView> = {
            let reg = self.runs.read().await;
            active
                .into_iter()
                .filter(|info| {
                    reg.runs
                        .get(&info.execution_id)
                        .map(|m| m.owner == owner && m.active)
                        .unwrap_or(false)
                })
                .filter(|info| workflow_id.map(|want| info.workflow_id == want).unwrap_or(true))
                .map(|info| WorkflowRunActiveView {
                    execution_id: info.execution_id,
                    workflow_id: info.workflow_id,
                    status: info.status,
                    current_node_id: info.current_node_id,
                    owner_plugin_id: owner.to_string(),
                    created_at: info.created_at,
                    updated_at: info.updated_at,
                })
                .collect()
        };

        // 按 `updated_at` 降序（契约：本页按最近更新优先）
        views.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
        let total = views.len();
        views.truncate(limit);
        Ok(WorkflowRunActivePage { executions: views, total })
    }
}

#[cfg(test)]
mod tests {
    use super::{
        ALLOWED_TEMPLATE_PREFIXES, RunLimits, extract_stock_code, is_template_allowed,
        merge_variables,
    };

    #[test]
    fn whitelist_only_opens_stock_family() {
        // 白名单当前只开 stock- 一族（放开别的族必须先改这个常量 + 报告口径）
        assert_eq!(ALLOWED_TEMPLATE_PREFIXES, &["stock-"]);

        for id in ["stock-analysis", "stock-reflection", "stock-pipeline", "stock-serenity-x"] {
            assert!(is_template_allowed(id), "{id} 应可运行");
        }
        // 反向：非 stock- 前缀一律拒（含大小写、含空串、含路径穿越式 id）
        for id in ["", "Stock-analysis", "stock", "ops-cleanup", "../stock-analysis", "xstock-"] {
            assert!(!is_template_allowed(id), "{id} 不应可运行");
        }
    }

    #[test]
    fn stock_code_is_required_and_trimmed() {
        let ok = serde_json::json!({"stock_code": "  600519 "});
        assert_eq!(extract_stock_code(Some(&ok)).expect("应解析成功"), "600519");

        for bad in [
            serde_json::json!({}),
            serde_json::json!({"stock_code": ""}),
            serde_json::json!({"stock_code": "   "}),
            serde_json::json!({"stock_code": 600519}),
        ] {
            let err = extract_stock_code(Some(&bad)).expect_err("应缺参报错");
            assert!(matches!(err, axagent_harness::AxAgentError::Validation(_)), "{err:?}");
        }
        assert!(extract_stock_code(None).is_err());
    }

    #[test]
    fn default_limits_are_per_owner_2_and_global_8() {
        let limits = RunLimits::default();
        assert_eq!(limits.per_owner, 2);
        assert_eq!(limits.global, 8);
        // 全局必须容纳「至少两个 owner 各自跑到上限」，否则 per-owner 形同虚设
        assert!(limits.global >= limits.per_owner * 2);
    }

    fn var(name: &str, value: serde_json::Value) -> axagent_harness::workflow_types::Variable {
        axagent_harness::workflow_types::Variable {
            name: name.into(),
            var_type: "string".into(),
            value,
            description: None,
            is_secret: false,
        }
    }

    #[test]
    fn merge_variables_overrides_by_name_and_keeps_template_rest() {
        let base =
            vec![var("max_concurrent", serde_json::json!(8)), var("keep", serde_json::json!("t"))];
        let over =
            vec![var("max_concurrent", serde_json::json!(2)), var("new", serde_json::json!(1))];

        let merged = merge_variables(Some(base), &over).expect("应返回合并结果");
        let names: Vec<&str> = merged.iter().map(|v| v.name.as_str()).collect();
        // 模板顺序保留，请求新增项追加到尾部
        assert_eq!(names, vec!["max_concurrent", "keep", "new"]);
        // 同名项被请求值覆盖 —— 这是「插件改了并发必须生效」的直接锁
        assert_eq!(merged[0].value, serde_json::json!(2));
        // 模板独有项不得丢（丢了会让 input_mapping 静默退回模块常量）
        assert_eq!(merged[1].value, serde_json::json!("t"));
    }

    #[test]
    fn merge_variables_handles_empty_sides() {
        // 两边都空 ⇒ None（不造空表，保持 `RunOptions.variables` 的既有 None 语义）
        assert!(merge_variables(None, &[]).is_none());
        // 仅请求 ⇒ 原文拷贝
        let only_req = merge_variables(None, &[var("a", serde_json::json!(1))]).expect("应有值");
        assert_eq!(only_req.len(), 1);
        // 仅模板 ⇒ 原样保留
        let only_base =
            merge_variables(Some(vec![var("a", serde_json::json!(1))]), &[]).expect("应有值");
        assert_eq!(only_base.len(), 1);
    }
}
