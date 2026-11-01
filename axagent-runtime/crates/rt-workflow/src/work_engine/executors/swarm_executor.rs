// SPDX-License-Identifier: AGPL-3.0-only

//! Swarm 容器执行器 —— 驱动多 Agent 团队协作执行。
//!
//! 通过 `debate_body_dispatch` 回调（同 LoopBodyDispatchFn 签名）分步驱动
//! 每个群组成员节点（LLM/Agent），在多轮协作中收集各方结果。
//!
//! 完整 Swarm 团队管理（跨进程 JSON 行协议通信、Teammate/Team CRUD）
//! 位于 runtime crate 的 swarm 模块中，供独立团队场景使用。
//! 本 Executor 提供 Workflow DAG 引擎内的 Swarm 节点执行能力。

use crate::work_engine::execution_state::ExecutionState;
use crate::work_engine::node_executor_trait::{NodeError, NodeExecutorTrait, NodeOutput};
use async_trait::async_trait;
use axagent_harness::workflow_types::WorkflowNode;
use std::collections::HashMap;

pub struct SwarmExecutor;
impl SwarmExecutor {
    pub fn new() -> Self {
        Self
    }
}
impl Default for SwarmExecutor {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl NodeExecutorTrait for SwarmExecutor {
    fn node_type(&self) -> &'static str {
        "swarm"
    }

    async fn execute(
        &self,
        node: &WorkflowNode,
        ctx: &ExecutionState,
    ) -> Result<NodeOutput, NodeError> {
        let WorkflowNode::Swarm(sn) = node else {
            return Err(NodeError::type_mismatch(
                "swarm".to_string(),
                super::node_type_name(node).to_string(),
            ));
        };

        let agent_steps = &sn.config.agent_steps;
        let max_rounds = sn.config.max_rounds.max(1);
        let topic_var = &sn.config.topic_var;
        let convergence_prompt = sn.config.convergence_prompt.as_deref();
        let output_var = sn.config.output_var.clone();

        if agent_steps.is_empty() {
            return Ok(NodeOutput {
                output: serde_json::json!({
                    "status": "no_agents",
                    "agent_steps": [],
                }),
                output_var: Some(output_var),
                control: None,
            });
        }

        let dispatch_fn = ctx
            .callbacks
            .as_ref()
            .and_then(|cb| cb.debate_body_dispatch.clone())
            .ok_or_else(|| {
                NodeError::exec_failed(
                    "swarm_no_dispatch",
                    "Swarm body dispatch callback not available. Ensure the engine sets debate_body_dispatch."
                        .to_string(),
                )
            })?;

        let topic = ctx
            .variables
            .get(topic_var.as_str())
            .cloned()
            .unwrap_or(serde_json::Value::String("swarm topic".to_string()));

        let mut round_outputs: Vec<HashMap<String, serde_json::Value>> = Vec::new();
        let mut prev_round_snapshot: Option<Vec<serde_json::Value>> = None;

        // A2（2026-09-14）：与 `debate_executor` 同款修复 —— 旧的硬编码
        // `"status": "completed"` 是有 agent 失败时的 fail-open 说谎字段。
        // 职责边界同 debate：本层只「说真话」，不改变控制流；重试粒度在单个
        // 子节点（`dispatch_container_body_with_retry`），不是整个容器。
        let mut attempted: u32 = 0;
        let mut succeeded: u32 = 0;
        let mut failed_agents: Vec<serde_json::Value> = Vec::new();

        for round in 0..max_rounds {
            tracing::info!(
                "Swarm round {}/{} with {} agents",
                round + 1,
                max_rounds,
                agent_steps.len()
            );

            let mut round_results: HashMap<String, serde_json::Value> = HashMap::new();

            for step_id in agent_steps {
                let mut round_ctx = ctx.clone();
                round_ctx.variables.insert("__swarm_topic__".to_string(), topic.clone());
                round_ctx.variables.insert("__swarm_round__".to_string(), serde_json::json!(round));
                round_ctx
                    .variables
                    .insert("__swarm_max_rounds__".to_string(), serde_json::json!(max_rounds));
                // 注入收敛提示（LLM 可用此判断是否已达成共识）
                if let Some(cp) = convergence_prompt {
                    round_ctx.variables.insert(
                        "__swarm_convergence_prompt__".to_string(),
                        serde_json::Value::String(cp.to_string()),
                    );
                }
                // 注入前几轮输出供参考
                if let Some(ref snapshot) = prev_round_snapshot {
                    round_ctx
                        .variables
                        .insert("__swarm_history__".to_string(), serde_json::json!(snapshot));
                }

                attempted += 1;
                match dispatch_fn(step_id.clone(), round_ctx).await {
                    Ok(output) => {
                        succeeded += 1;
                        round_results.insert(step_id.clone(), output.output);
                    },
                    Err(e) => {
                        let err_text = e.to_string();
                        tracing::warn!(
                            "Swarm agent '{}' failed in round {}: {}",
                            step_id,
                            round,
                            err_text
                        );
                        failed_agents.push(serde_json::json!({
                            "agent": step_id,
                            "round": round,
                            "error": err_text,
                        }));
                        round_results.insert(
                            step_id.clone(),
                            serde_json::json!({
                                "error": err_text,
                                "round": round,
                            }),
                        );
                    },
                }
            }

            round_outputs.push(round_results.clone());

            // 收敛检测（从第 2 轮开始）。check_round_convergence 是纯文本相似度
            // 比较，不依赖 convergence_prompt；旧门控导致未配置该变量的模板
            // 永远跑满 max_rounds 轮（与 debate_executor 同步修复 2026-09-08）。
            if round > 0
                && super::check_round_convergence(
                    &round_results,
                    &round_outputs[round.saturating_sub(1) as usize],
                )
            {
                tracing::info!("Swarm converged at round {}/{}", round + 1, max_rounds);
                break;
            }

            prev_round_snapshot =
                Some(round_results.values().cloned().collect::<Vec<serde_json::Value>>());
        }

        // 状态三档（A2）：全成功 completed / 部分成功 degraded / 全失败 failed。
        let status = if failed_agents.is_empty() {
            "completed"
        } else if succeeded == 0 {
            "failed"
        } else {
            "degraded"
        };
        let degraded = !failed_agents.is_empty();
        if degraded {
            tracing::warn!(
                workflow = "swarm",
                status,
                failed = failed_agents.len(),
                attempted,
                rounds_used = round_outputs.len(),
                max_rounds,
                "Swarm 容器降级完成：有 agent 失败，内容不完整（下游质量闸门应据此拦截）"
            );
        }

        let final_output = serde_json::json!({
            "status": status,
            "degraded": degraded,
            "total_rounds": round_outputs.len(),
            "max_rounds": max_rounds,
            "attempted": attempted,
            "succeeded": succeeded,
            "failed_agents": failed_agents,
            "rounds": round_outputs,
            "consensus": super::build_round_consensus(&round_outputs),
        });

        Ok(NodeOutput { output: final_output, output_var: Some(output_var), control: None })
    }
}
