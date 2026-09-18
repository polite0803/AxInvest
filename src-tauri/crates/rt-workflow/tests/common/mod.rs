// SPDX-License-Identifier: AGPL-3.0-only

//! rt-workflow 集成测试的**公共夹具**。
//!
//! 收敛范围（2026-09-14）：原先每个 `tests/*.rs` 都是独立 crate，各写一份相同的 mock：
//! - `EmptyProviderRegistry` —— 4 个文件各一份（**逐字相同**）：`WorkEngine::new` 需要
//!   `Arc<dyn ProviderRegistry>`，但测试不消费任何 provider 能力。
//! - `RecordingWorkflowExecutionRepo` —— 2 个文件各一份（**逐字相同**）；
//!   `per_node_exec_ctx_vars.rs` 的 `MockWorkflowExecRepo` 是它的空桩子集，一并统一。
//!
//! 引用方式：各测试文件顶部 `mod common;` + `use common::{…};`。
//!
//! `#![allow(dead_code)]`：并非每个测试文件都会用到全部夹具（例如只用 ProviderRegistry
//! 的文件不会引用 repo），而 `cargo clippy --all-targets -- -D warnings` 会对每个测试
//! crate 单独判定 ⇒ 必须在此显式放行。判据由「各测试文件的真实使用点」承担。
#![allow(dead_code)]

use std::sync::Arc;

use async_trait::async_trait;
use axagent_harness::registry::ProviderRegistry;
use axagent_harness::repo_dtos::WorkflowExecutionData;
use axagent_harness::repositories::WorkflowExecutionRepository;
use tokio::sync::Mutex;

// ── 最小 ProviderRegistry ────────────────────────────────────────────

/// `WorkEngine::new` 构造时需要 `Arc<dyn ProviderRegistry>`；只用 tool 节点的测试
/// 不消费 provider 能力，故 `get` 恒返回 `None`。
pub struct EmptyProviderRegistry;

impl ProviderRegistry for EmptyProviderRegistry {
    fn get(&self, _provider_type: &str) -> Option<Arc<dyn axagent_harness::ProviderAdapter>> {
        None
    }
}

// ── 记录型 WorkflowExecutionRepository ───────────────────────────────

/// 记录型 repo 的 update 日志：`(exec_id, status, total_time_ms)`
pub type UpdateLog = Arc<Mutex<Vec<(String, String, Option<i32>)>>>;

/// 记录每次 `update_workflow_execution_status` 调用，便于断言 DB 是否收到
/// 终态与总耗时（`total_time_ms`）。其余方法为空实现。
#[derive(Clone)]
pub struct RecordingWorkflowExecutionRepo {
    /// (exec_id, status, total_time_ms)
    pub updates: UpdateLog,
}

impl Default for RecordingWorkflowExecutionRepo {
    fn default() -> Self {
        Self { updates: Arc::new(Mutex::new(Vec::new())) }
    }
}

#[async_trait]
impl WorkflowExecutionRepository for RecordingWorkflowExecutionRepo {
    async fn create_workflow_execution(
        &self,
        _id: &str,
        _workflow_id: &str,
        _input_params: Option<&str>,
    ) -> Result<(), String> {
        Ok(())
    }
    async fn update_workflow_execution_status(
        &self,
        id: &str,
        status: &str,
        _output_result: Option<&str>,
        _node_executions: Option<&str>,
        total_time_ms: Option<i32>,
    ) -> Result<bool, String> {
        self.updates.lock().await.push((id.to_string(), status.to_string(), total_time_ms));
        Ok(true)
    }
    async fn list_workflow_executions(
        &self,
        _workflow_id: &str,
    ) -> Result<Vec<WorkflowExecutionData>, String> {
        Ok(vec![])
    }
    async fn save_execution_state(
        &self,
        _id: &str,
        _status: &str,
        _execution_state_json: &str,
    ) -> Result<bool, String> {
        Ok(true)
    }
    async fn clear_execution_state(&self, _id: &str, _status: &str) -> Result<bool, String> {
        Ok(true)
    }
    async fn list_paused_executions(&self) -> Result<Vec<WorkflowExecutionData>, String> {
        Ok(vec![])
    }
}
