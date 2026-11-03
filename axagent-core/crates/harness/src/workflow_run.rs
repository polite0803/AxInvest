// SPDX-License-Identifier: AGPL-3.0-only

//! 工作流执行接缝（`workflow.run`）— DTO + Trait（**异步 job 形态**）。
//!
//! 宿主独占 `WorkEngine` 实例；插件**只持有 id**：`start` 立即返回
//! [`WorkflowRunHandle`]（`execution_id` + `workflow_id`），不等 DAG 跑完
//! （接缝等待预算 15 s，`start` 绝不能 await 执行体）；执行进度与终态靠
//! `event.dispatch`（`workflow/step` / `workflow/end`）推给插件，
//! **不进本契约**（进度是推送面，不是请求/响应面）。
//!
//! **owner 隔离**：`owner` 由 worker 分派层从连接身份（插件 id）注入，
//! 插件不可伪造、也不能自报他人的 id。`status` / `cancel` / `list_active`
//! 三个查询/取消方法**一律按 owner 过滤**，跨 owner 视同查无：
//! - `status` 查无（含跨 owner）⇒ `Ok(None)`；
//! - `cancel` 查无（含跨 owner）⇒ `{"cancelled":false,"status":null}`；
//! - `list_active` 只返回本插件发起的执行（每项带 `owner_plugin_id`，便于日志核对）。
//!
//! **权限由分派层强制**：`start` / `cancel` 要求插件已声明 `write` 权限
//! （`PLUGIN_PERMISSION_DENIED`）；`status` / `list_active` 只读不设闸。
//! 模板白名单与准入拒绝也在宿主侧判断，provider 用
//! [`AxAgentError::Validation`] 表达（分派层的 `workflow_run_error` 把它映射为 `WORKFLOW_RUN_REJECTED`）；
//! 本层只提供契约，不自行校验任何一项。
//!
//! **首版不支持跨宿主重启**：`execution_id` 登记在宿主内存里，宿主进程重启即失效
//! ⇒ `status` 返回 `null`（不查 DB 兜底，DB 里可能残留 `running` 幽灵行）。
//!
//! 查询视图刻意**不暴露**以下内容：
//! - `variables`：其 `is_secret` 只是标记，值里可能是 vendor token（密钥会随视图泄漏）；
//! - 每节点 `results` / 完整 `Workflow`：单次执行 20+ 节点时 clone 可达 MB 级；
//! - `input` / `input_schema` / `output_schema` / `tool_permissions`：
//!   宿主自决的执行参数，插件既不给也不该读回。
//!
//! `status` 默认**不带** `output`（全量 results 可能很大），`include_output = true`
//! 才填 [`ExecutionView::output`]。

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::core_error::{AxAgentError, Result};
use crate::workflow_types::{ExecutionStatus, Variable};

/// `start` 的入参。
///
/// 字段集刻意只覆盖「插件真正该决定的量」：执行参数（schema / `tool_permissions` /
/// `node_task_scope`）由宿主按模板与业务策略自决，不进本结构体。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowRunRequest {
    /// 模板 id。**必填**，由宿主按白名单校验（不存在或不可运行 ⇒ 拒绝）。
    pub template_id: String,
    /// 工作流入参。`None` 等价 `{}`。
    #[serde(default)]
    pub input: Option<serde_json::Value>,
    /// 运行时变量（业务记忆 / 选股条件等）。`is_secret` 的项同样只在宿主侧展开。
    #[serde(default)]
    pub variables: Vec<Variable>,
    /// 指定模型 id；`None` 时宿主按默认策略选。
    #[serde(default)]
    pub model_id: Option<String>,
    /// 指定 provider id；`None` 时宿主按模型推导。
    #[serde(default)]
    pub provider_id: Option<String>,
    /// 单次执行内**节点级**并发上限；`None` 用宿主默认。
    #[serde(default)]
    pub max_concurrent: Option<u32>,
    /// 单步超时（秒）。
    #[serde(default)]
    pub step_timeout_secs: Option<u64>,
    /// 单次工具调用超时（秒）。
    #[serde(default)]
    pub tool_timeout_secs: Option<u64>,
    /// 执行总超时（秒）。到点由宿主 `cancel`，引擎自身不实现总超时。
    #[serde(default)]
    pub total_timeout_secs: Option<u64>,
    /// 空跑（只校验图与入参，不真正调用 LLM / 工具）。
    #[serde(default)]
    pub dry_run: bool,
    /// 回放基准日 `"YYYY-MM-DD"`。宿主负责包 `AS_OF` scope ——
    /// 漏传会退化成「回放跑成实盘」的前视事故，故**只能由宿主落地**。
    #[serde(default)]
    pub as_of_date: Option<String>,
}

/// `start` 的受理回执（**已受理，未必已跑完**）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowRunHandle {
    /// 执行 id：后续 `status` / `cancel` 的唯一句柄，宿主内存登记。
    pub execution_id: String,
    /// 本次执行的 workflow id（与 `execution_id` **互相独立**，不可互相替代）。
    pub workflow_id: String,
    /// 受理时刻（毫秒时间戳，宿主生成）。
    pub accepted_at: i64,
}

/// 单次执行的运行态视图（`status` 的返回项）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecutionView {
    pub execution_id: String,
    pub workflow_id: String,
    /// 六态运行状态，字面量 snake_case（`running` / `paused` / `completed` /
    /// `partially_completed` / `failed` / `cancelled`），与引擎枚举一致。
    pub status: ExecutionStatus,
    /// 当前（或最后完成的）节点 id；未开始或已收尾时可为 `None`。
    pub current_node_id: Option<String>,
    /// 从受理到 `updated_at` 的累计耗时（毫秒）。
    pub total_time_ms: u64,
    pub created_at: i64,
    pub updated_at: i64,
    /// 执行输出：**仅 `include_output = true` 时出现**，其余情况整体省略该键。
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub output: Option<serde_json::Value>,
}

/// `cancel` 的结果。
///
/// 三态自解释：真取消 ⇒ `cancelled = true`；终态执行 ⇒ `cancelled = false` +
/// 现值（**幂等，不得覆写终态**）；查无 / 跨 owner ⇒ `cancelled = false` + `None`。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowRunCancelOutcome {
    pub cancelled: bool,
    #[serde(default)]
    pub status: Option<ExecutionStatus>,
}

/// 在跑执行列表的一项（`list_active` 的返回项）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowRunActiveView {
    pub execution_id: String,
    pub workflow_id: String,
    pub status: ExecutionStatus,
    pub current_node_id: Option<String>,
    /// 发起者插件 id（分派层注入的 owner），恒等于调用方自身。
    pub owner_plugin_id: String,
    pub created_at: i64,
    pub updated_at: i64,
}

/// `list_active` 的分页结果。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowRunActivePage {
    /// 本页执行（按 `updated_at` 降序，至多 `limit` 条）。
    pub executions: Vec<WorkflowRunActiveView>,
    /// `limit` **之前**的匹配总数，供插件判断是否还有下一页。
    pub total: usize,
}

/// 工作流执行提供者（异步 job：`start` 受理、后续按 id 查询/取消）。
///
/// 实现方：宿主 wiring 层（`axagent-runtime`，独持 `WorkEngine`）。
/// 默认实现统一返回「该提供者不支持此数据面」，便于外部实现只覆盖关心的 op。
///
/// 所有方法的 `owner` 参数由 worker 分派层从连接身份注入（插件不可伪造）；
/// 四个方法都必须**秒回** —— `start` 只受理不等待执行体，否则会撞上接缝 15 s 预算。
#[async_trait]
pub trait WorkflowRunProvider: Send + Sync {
    /// 受理一次执行，立即返回句柄（不等 DAG 跑完）。
    ///
    /// 要求插件已声明 `write` 权限（分派层强制）。模板不存在 / 不在白名单 /
    /// 准入被拒 ⇒ 实现方返回 [`AxAgentError::Validation`]。
    async fn start(&self, _owner: &str, _request: WorkflowRunRequest) -> Result<WorkflowRunHandle> {
        Err(AxAgentError::Provider(
            "WorkflowRunProvider::start 未实现（该提供者不支持此数据面）".into(),
        ))
    }

    /// 查单次执行状态。
    ///
    /// **查无（含跨 owner、宿主已重启）⇒ `Ok(None)`**，不借用错误码。
    /// `include_output = false` 时 `output` 必须为 `None`（不进帧）。
    async fn status(
        &self,
        _owner: &str,
        _execution_id: &str,
        _include_output: bool,
    ) -> Result<Option<ExecutionView>> {
        Err(AxAgentError::Provider(
            "WorkflowRunProvider::status 未实现（该提供者不支持此数据面）".into(),
        ))
    }

    /// 取消一次执行。
    ///
    /// 要求插件已声明 `write` 权限（分派层强制）。**幂等**：
    /// - 查无 / 跨 owner ⇒ `{"cancelled": false, "status": null}`；
    /// - 已在终态（completed / partially_completed / failed / cancelled）⇒
    ///   `{"cancelled": false, "status": Some(现值)}`，**不得调引擎**（引擎 `cancel`
    ///   会无条件覆写终态，把 `completed` 改成 `cancelled`）；
    /// - 仅在 running / paused 上真取消 ⇒ `{"cancelled": true, "status": "cancelled"}`。
    async fn cancel(&self, _owner: &str, _execution_id: &str) -> Result<WorkflowRunCancelOutcome> {
        Err(AxAgentError::Provider(
            "WorkflowRunProvider::cancel 未实现（该提供者不支持此数据面）".into(),
        ))
    }

    /// 列本插件**在跑**（running / paused）的执行。
    ///
    /// `workflow_id = None` 列出全部；`limit` 为页大小（宿主可再夹上限）。
    /// 返回的 `total` 是 `limit` **之前**的匹配总数。
    async fn list_active(
        &self,
        _owner: &str,
        _workflow_id: Option<&str>,
        _limit: usize,
    ) -> Result<WorkflowRunActivePage> {
        Err(AxAgentError::Provider(
            "WorkflowRunProvider::list_active 未实现（该提供者不支持此数据面）".into(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn empty_view(status: ExecutionStatus) -> ExecutionView {
        ExecutionView {
            execution_id: "exec-1".into(),
            workflow_id: "wf-1".into(),
            status,
            current_node_id: Some("node-3".into()),
            total_time_ms: 1234,
            created_at: 1_700_000_000_000,
            updated_at: 1_700_000_001_234,
            output: None,
        }
    }

    #[test]
    fn request_defaults() {
        let req: WorkflowRunRequest = serde_json::from_value(json!({
            "templateId": "stock-analysis"
        }))
        .expect("只给 templateId 应能反序列化");

        assert_eq!(req.template_id, "stock-analysis");
        assert!(req.input.is_none());
        assert!(req.variables.is_empty());
        assert!(req.model_id.is_none());
        assert!(req.provider_id.is_none());
        assert!(req.max_concurrent.is_none());
        assert!(req.step_timeout_secs.is_none());
        assert!(req.tool_timeout_secs.is_none());
        assert!(req.total_timeout_secs.is_none());
        assert!(!req.dry_run);
        assert!(req.as_of_date.is_none());

        // 键名必须是 camelCase：snake_case 的 `template_id` 不被识别（缺必填即报错）
        assert!(
            serde_json::from_value::<WorkflowRunRequest>(json!({
                "template_id": "stock-analysis"
            }))
            .is_err()
        );
    }

    #[test]
    fn execution_view_camel_case_and_status_snake_case() {
        let v = empty_view(ExecutionStatus::PartiallyCompleted);
        let val = serde_json::to_value(&v).expect("序列化应成功");

        assert_eq!(val["executionId"], json!("exec-1"));
        assert_eq!(val["workflowId"], json!("wf-1"));
        assert_eq!(val["currentNodeId"], json!("node-3"));
        assert_eq!(val["totalTimeMs"], json!(1234));
        assert_eq!(val["createdAt"], json!(1_700_000_000_000_i64));
        // 枚举值走枚举自身的 snake_case，不随外层 rename_all 变成 camelCase
        assert_eq!(val["status"], json!("partially_completed"));
        // 不得残留 snake_case 键
        assert!(val.get("execution_id").is_none());
        assert!(val.get("current_node_id").is_none());
        assert!(val.get("total_time_ms").is_none());
    }

    #[test]
    fn output_skipped_when_none() {
        let val = serde_json::to_value(empty_view(ExecutionStatus::Running)).expect("序列化应成功");
        assert!(val.get("output").is_none(), "output=None 时不应出现该键: {val}");

        // 有 output 时才出现（且不改变其它键）
        let mut with_out = empty_view(ExecutionStatus::Completed);
        with_out.output = Some(json!({"decision": "hold"}));
        let val = serde_json::to_value(&with_out).expect("序列化应成功");
        assert_eq!(val["output"], json!({"decision": "hold"}));
    }

    struct Unimplemented;

    impl WorkflowRunProvider for Unimplemented {}

    #[tokio::test]
    async fn default_trait_methods_are_unimplemented() {
        let p = Unimplemented;

        let err = p
            .start(
                "plugin-a",
                WorkflowRunRequest {
                    template_id: "t".into(),
                    input: None,
                    variables: Vec::new(),
                    model_id: None,
                    provider_id: None,
                    max_concurrent: None,
                    step_timeout_secs: None,
                    tool_timeout_secs: None,
                    total_timeout_secs: None,
                    dry_run: false,
                    as_of_date: None,
                },
            )
            .await
            .expect_err("start 默认实现必须是错误");
        assert!(matches!(err, AxAgentError::Provider(_)), "start: {err:?}");

        let err =
            p.status("plugin-a", "exec-1", false).await.expect_err("status 默认实现必须是错误");
        assert!(matches!(err, AxAgentError::Provider(_)), "status: {err:?}");

        let err = p.cancel("plugin-a", "exec-1").await.expect_err("cancel 默认实现必须是错误");
        assert!(matches!(err, AxAgentError::Provider(_)), "cancel: {err:?}");

        let err =
            p.list_active("plugin-a", None, 50).await.expect_err("list_active 默认实现必须是错误");
        assert!(matches!(err, AxAgentError::Provider(_)), "list_active: {err:?}");
    }
}
