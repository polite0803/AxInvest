// SPDX-License-Identifier: AGPL-3.0-only

//! Harness Agent trait 适配器 — 包装 ReActEngine 实现 harness Agent。
//!
//! 接线说明：
//! - 2026-09-03：本模块曾因 lib.rs 缺 `mod harness_adapter;` 从未编译，
//!   其间 harness `AgentResult` / `PlanStep` 字段已收敛（见 `axagent-harness::agent`），
//!   此处按现行契约适配。
//! - 2026-09-04：注入可选 SessionManager，execute 时自动创建会话并返回真实
//!   session_id；同时为运行中取消预留 Arc<AtomicBool> token 检查钩子。

use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::Instant;

use async_trait::async_trait;
use axagent_harness::agent::{
    Agent, AgentCapability, AgentExecuteRequest, AgentPlan, AgentResult, PlanStep,
};
use uuid::Uuid;

use crate::react_engine::ReActEngine;
use crate::session_manager::SessionManager;

pub struct HarnessAgentAdapter {
    name: String,
    caps: Vec<AgentCapability>,
    engine: tokio::sync::Mutex<ReActEngine>,
    /// 可选：如果注入了 SessionManager，execute 会创建真实会话并返回 session_id。
    session_manager: Option<Arc<SessionManager>>,
    /// 可选：全局取消信号。置 true 后所有正在 run 的 execute 应尽快退出。
    /// 供 AgentSessionBroker::cancel_session 调用时唤醒。
    cancellation_flag: Option<Arc<AtomicBool>>,
}

// ReActEngine 含 trait object 字段无法自动 derive Debug，
// 而 harness `Agent` trait 要求 `fmt::Debug` —— 手动实现。
impl std::fmt::Debug for HarnessAgentAdapter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HarnessAgentAdapter")
            .field("name", &self.name)
            .field("caps", &self.caps)
            .field("has_session_manager", &self.session_manager.is_some())
            .field("has_cancellation", &self.cancellation_flag.is_some())
            .finish()
    }
}

impl HarnessAgentAdapter {
    pub fn new(name: &str) -> Self {
        Self {
            name: name.to_string(),
            caps: vec![
                AgentCapability {
                    name: "reasoning".into(), description: "ReAct 推理循环".into()
                },
                AgentCapability {
                    name: "tool_use".into(),
                    description: "使用注册工具执行操作".into(),
                },
            ],
            engine: tokio::sync::Mutex::new(ReActEngine::new()),
            session_manager: None,
            cancellation_flag: None,
        }
    }

    /// 注入 SessionManager —— execute 时创建真实会话并返回 session_id。
    pub fn with_session_manager(mut self, sm: Arc<SessionManager>) -> Self {
        self.session_manager = Some(sm);
        self
    }

    /// 注入全局取消信号。置 true 后 ReActEngine::run 循环应尽快退出。
    pub fn with_cancellation_flag(mut self, flag: Arc<AtomicBool>) -> Self {
        self.cancellation_flag = Some(flag);
        self
    }

    /// 便捷 builder：同时注入 session_manager + 一个共享取消 flag。
    pub fn with_runtime(self, sm: Arc<SessionManager>) -> Self {
        let flag = Arc::new(AtomicBool::new(false));
        self.with_session_manager(sm).with_cancellation_flag(flag)
    }

    /// 注入已装配好的 `ReActEngine`（保留 `new()` 的默认 caps）。
    ///
    /// **wiring 层必须走本方法，不要直接用 `new()` 的默认引擎**：`new()` 内部的
    /// `ReActEngine::new()` 未注入 `LlmReasoningProvider`，`reasoning_provider`
    /// 停留在 `DefaultReasoningProvider`，其每个 trait 方法直接返回
    /// `Err("...not configured: inject a real LlmReasoningProvider...")`
    /// ⇒ 每次 `execute` 都会在 Analyzing 阶段失败（`ReActEngine::run` 重试
    /// `max_retry_attempts` 次后返回 `ReActResult::failure`）。
    ///
    /// 注入示例（`src/init/state.rs`）：
    /// ```ignore
    /// let engine = ReActEngine::new().with_reasoning_provider(provider);
    /// HarnessAgentAdapter::new("default").with_engine(engine)
    /// ```
    pub fn with_engine(mut self, engine: ReActEngine) -> Self {
        self.engine = tokio::sync::Mutex::new(engine);
        self
    }
}

#[async_trait]
impl Agent for HarnessAgentAdapter {
    fn name(&self) -> &str {
        &self.name
    }

    fn capabilities(&self) -> Vec<AgentCapability> {
        self.caps.clone()
    }

    async fn execute(&self, req: AgentExecuteRequest) -> Result<AgentResult, String> {
        let _start = Instant::now();

        // 1. 如果注入了 SessionManager，先创建会话
        let (provider_id, conversation_id) = self
            .session_manager
            .as_ref()
            .map(|_sm| {
                // MCP agent_run 每次调用独立创建 conversation_id
                let provider = "default".to_string();
                let conv = format!("mcp-{}", Uuid::new_v4());
                (provider, conv)
            })
            .unwrap_or_else(|| ("default".to_string(), "standalone".to_string()));

        let session_id = if let Some(sm) = &self.session_manager {
            match sm.create_session(provider_id.clone(), conversation_id.clone()).await {
                Ok(session) => Some(session.session().session_id.clone()),
                Err(e) => {
                    tracing::warn!(
                        "[HarnessAgentAdapter] create_session failed: {e}, falling back to no-session mode"
                    );
                    None
                },
            }
        } else {
            None
        };

        // 2. 获取本次执行对应的取消 token（优先 per-session，回退全局）
        let cancel_token: Option<Arc<AtomicBool>> =
            if let (Some(sm), Some(sid)) = (&self.session_manager, &session_id) {
                // per-session token：SessionManager.create_session 时注册
                sm.get_cancel_token(sid).await
            } else if let Some(ref flag) = self.cancellation_flag {
                // 回退全局 flag（无 SessionManager 场景）
                flag.store(false, std::sync::atomic::Ordering::SeqCst);
                Some(Arc::clone(flag))
            } else {
                None
            };

        // 3. 执行推理循环（带取消检查钩子）
        let result = {
            let mut engine = self.engine.lock().await;
            if let Some(token) = cancel_token {
                engine.set_cancel_flag(token);
            }

            // 调用级预算：消费 `max_steps`（入边为 MCP `agent_run` 的请求参数，
            // 见 `crates/mcp/src/server.rs` `agent_run` → `max_steps: req.max_steps`）。
            // engine 在 Mutex 内**跨调用复用**，故必须 save/restore —— 直接改写
            // 会污染后续调用。`0` 视为未指定（否则立即触发 "Max iterations (0) reached"）。
            let prev_max_iterations = engine.max_iterations();
            let override_max_iterations = req.max_steps.filter(|v| *v > 0).map(|v| v as usize);
            if let Some(n) = override_max_iterations {
                engine.set_max_iterations(n);
            }

            let out = engine.run(&req.goal).await;

            if override_max_iterations.is_some() {
                engine.set_max_iterations(prev_max_iterations);
            }
            out
        };

        // 3. 返回结果
        Ok(AgentResult {
            output: result.final_response,
            success: result.success,
            steps_taken: result.iterations as u32,
            session_id,
        })
    }

    async fn plan(&self, goal: &str) -> Result<AgentPlan, String> {
        Ok(AgentPlan {
            steps: vec![
                PlanStep { description: format!("分析目标：{goal}"), agent: None },
                PlanStep { description: "执行推理循环".into(), agent: None },
                PlanStep { description: "生成最终结果".into(), agent: None },
            ],
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::react_engine::ReActEngine;
    use crate::reasoning_state::ReActConfig;

    /// `with_engine` 必须保留 `new()` 建立的默认能力集。
    ///
    /// 回归护栏：被它替代的 `from_engine` 曾把 `caps` 置空（`caps: vec![]`），
    /// 使经该路径构造的适配器对外声明零能力。
    #[test]
    fn with_engine_preserves_default_capabilities() {
        let adapter = HarnessAgentAdapter::new("t").with_engine(ReActEngine::new());
        assert_eq!(adapter.capabilities().len(), 2, "with_engine 不应清空默认 caps");
    }

    /// 调用级 `max_steps` 覆盖必须在 `execute` 结束后还原。
    ///
    /// engine 在 `Mutex` 内**跨调用复用** —— 覆盖后不还原会让后续调用的
    /// `max_iterations` 被上一次的 `max_steps` 永久污染。
    /// 本用例走**失败路径**（未注入 reasoning provider ⇒ `run()` 返回 failure），
    /// 用于确认失败路径同样会还原 config。
    #[tokio::test]
    async fn execute_restores_max_iterations_after_override() {
        let adapter = HarnessAgentAdapter::new("t");
        let baseline = ReActConfig::default().max_iterations;

        let _ = adapter
            .execute(AgentExecuteRequest {
                goal: "noop".to_string(),
                context: None,
                max_steps: Some(7),
            })
            .await;

        let engine = adapter.engine.lock().await;
        assert_eq!(engine.max_iterations(), baseline, "max_steps 覆盖未还原：engine 被跨调用污染");
    }

    /// `max_steps: Some(0)` 视为未指定 —— 否则会立即触发 "Max iterations (0) reached"。
    #[tokio::test]
    async fn zero_max_steps_is_treated_as_unspecified() {
        let adapter = HarnessAgentAdapter::new("t");
        let baseline = ReActConfig::default().max_iterations;

        let _ = adapter
            .execute(AgentExecuteRequest {
                goal: "noop".to_string(),
                context: None,
                max_steps: Some(0),
            })
            .await;

        let engine = adapter.engine.lock().await;
        assert_eq!(engine.max_iterations(), baseline);
    }
}
