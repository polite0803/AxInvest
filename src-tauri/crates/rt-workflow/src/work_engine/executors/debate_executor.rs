// SPDX-License-Identifier: AGPL-3.0-only

//! 辩论容器执行器 —— 通过 `debate_body_dispatch` 回调驱动多轮辩论。
//!
//! 与 SwarmExecutor 语义一致（多 Agent × 多轮 + 相邻轮次相似度收敛），
//! 区别在于语义标签（debate vs swarm）和注入的上下文变量名。
//! 共享收敛检测与共识构建逻辑见 `executors::check_round_convergence` /
//! `executors::build_round_consensus`。
//!
//! 设计要点：
//!  1) 读取 `debater_steps` 配置，按 `max_rounds` 顺序驱动每个辩手节点。
//!  2) 通过 `ExecutionState.callbacks.debate_body_dispatch` 回调驱动辩手节点
//!     （回调由 `WorkEngine::build_debate_body_dispatch` 工厂构造，内部走
//!     dispatcher，保留 progress_callback / 节点状态切换 / node_records 统一埋点）。
//!  3) 每轮注入 `__debate_topic__` / `__debate_round__` / `__debate_history__`
//!     / `__debate_convergence_prompt__` 供辩手 LLM 参考上下文。
//!  4) 从第 2 轮起做收敛检测（相邻轮次输出相似度 >= 0.80 即停止）。

use crate::work_engine::execution_state::ExecutionState;
use crate::work_engine::node_executor_trait::{NodeError, NodeExecutorTrait, NodeOutput};
use async_trait::async_trait;
use axagent_harness::workflow_types::WorkflowNode;
use std::collections::HashMap;

pub struct DebateExecutor;
impl DebateExecutor {
    pub fn new() -> Self {
        Self
    }
}
impl Default for DebateExecutor {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl NodeExecutorTrait for DebateExecutor {
    fn node_type(&self) -> &'static str {
        "debate"
    }

    async fn execute(
        &self,
        node: &WorkflowNode,
        ctx: &ExecutionState,
    ) -> Result<NodeOutput, NodeError> {
        let WorkflowNode::Debate(dn) = node else {
            return Err(NodeError::type_mismatch(
                "debate".to_string(),
                super::node_type_name(node).to_string(),
            ));
        };

        let debater_steps = &dn.config.debater_steps;
        let max_rounds = dn.config.max_rounds.max(1);
        let topic_var = &dn.config.topic_var;
        let convergence_prompt = dn.config.convergence_prompt.as_deref();
        let output_var = dn.config.output_var.clone();

        if debater_steps.is_empty() {
            return Ok(NodeOutput {
                output: serde_json::json!({
                    "status": "no_debaters",
                    "debater_steps": [],
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
                    "debate_no_dispatch",
                    "Debate body dispatch callback not available. Ensure the engine sets debate_body_dispatch."
                        .to_string(),
                )
            })?;

        let topic = ctx
            .variables
            .get(topic_var.as_str())
            .cloned()
            .unwrap_or(serde_json::Value::String("debate topic".to_string()));

        let mut round_outputs: Vec<HashMap<String, serde_json::Value>> = Vec::new();
        let mut prev_round_snapshot: Option<Vec<serde_json::Value>> = None;

        // A2（2026-09-14）：如实统计辩手失败，取代旧的硬编码 `"status": "completed"`。
        //
        // 旧行为：无论几个辩手失败，容器输出恒定 `status="completed"` —— 与
        // `debate_body_dispatch` 已把该子节点标成 `NodeStatus::Failed` 的事实
        // 自相矛盾。后果两层：
        //   ① 下游（quality-gate / v-validate）拿到「成功」表象 + 残缺内容，
        //      质量闸门失去拦截依据；
        //   ② 排障只能人肉翻 `rounds[].<step>.error`，601166 审计里 `bear-r3`
        //      的 504 就是这样被藏了一整轮排查。
        //
        // ⚠️ 职责边界（重要）：本函数**不改变控制流**，仍返回 `Ok`。
        //   - 「子节点失败 ⇒ 下游不继续」由主图的 fail-closed 级联负责
        //     （`upstream ∈ {Failed, Skipped} ⇒ 下游标 Skipped`），用户裁决即此；
        //   - 「失败重试」的粒度是**单个子节点**，在
        //     `dispatch_container_body_with_retry` 里按该子节点自己的 `retry`
        //     配置执行 —— **不是整个容器重试**。容器这一层只负责「说真话」。
        //   - 因此这里刻意**不**在全部辩手失败时改返回 `Err`：那会引入第二个、
        //     更粗粒度的重试层（容器级），把已成功的辩手也卷进重跑。
        let mut attempted: u32 = 0;
        let mut succeeded: u32 = 0;
        let mut failed_debaters: Vec<serde_json::Value> = Vec::new();

        for round in 0..max_rounds {
            tracing::info!(
                "Debate round {}/{} with {} debaters",
                round + 1,
                max_rounds,
                debater_steps.len()
            );

            let mut round_results: HashMap<String, serde_json::Value> = HashMap::new();

            for step_id in debater_steps {
                let mut round_ctx = ctx.clone();
                round_ctx.variables.insert("__debate_topic__".to_string(), topic.clone());
                round_ctx
                    .variables
                    .insert("__debate_round__".to_string(), serde_json::json!(round));
                round_ctx
                    .variables
                    .insert("__debate_max_rounds__".to_string(), serde_json::json!(max_rounds));
                // 注入收敛提示（辩手可用此判断是否已达成共识）
                if let Some(cp) = convergence_prompt {
                    round_ctx.variables.insert(
                        "__debate_convergence_prompt__".to_string(),
                        serde_json::Value::String(cp.to_string()),
                    );
                }
                // 注入前几轮输出供参考
                if let Some(ref snapshot) = prev_round_snapshot {
                    round_ctx
                        .variables
                        .insert("__debate_history__".to_string(), serde_json::json!(snapshot));
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
                            "Debate debater '{}' failed in round {}: {}",
                            step_id,
                            round,
                            err_text
                        );
                        // A2：失败明细单独收集，供最终输出如实汇报（不再只靠翻 rounds）。
                        failed_debaters.push(serde_json::json!({
                            "debater": step_id,
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

            // 收敛检测（从第 2 轮开始）。注意：check_round_convergence 是纯文本
            // 相似度比较，不依赖 convergence_prompt——后者只是注入给辩手的参考
            // 变量。旧代码用 convergence_prompt.is_some() 门控收敛检测，导致
            // 未配置该变量的模板（如 stock-analysis）永远跑满 max_rounds 轮、
            // 同一批辩手被重复调用 N 倍次数（2026-09-08 实证 18 次调用）。
            if round > 0
                && super::check_round_convergence(
                    &round_results,
                    &round_outputs[round.saturating_sub(1) as usize],
                )
            {
                tracing::info!("Debate converged at round {}/{}", round + 1, max_rounds);
                break;
            }

            prev_round_snapshot =
                Some(round_results.values().cloned().collect::<Vec<serde_json::Value>>());
        }

        // 状态三档（A2）：全成功 completed / 部分成功 degraded / 全失败 failed。
        // 旧实现恒定 "completed"，是「报错但仍成功」的典型 fail-open 说谎字段。
        let status = if failed_debaters.is_empty() {
            "completed"
        } else if succeeded == 0 {
            "failed"
        } else {
            "degraded"
        };
        let degraded = !failed_debaters.is_empty();
        if degraded {
            tracing::warn!(
                workflow = "debate",
                status,
                failed = failed_debaters.len(),
                attempted,
                rounds_used = round_outputs.len(),
                max_rounds,
                "辩论容器降级完成：有辩手失败，内容不完整（下游质量闸门应据此拦截）"
            );
        }

        let final_output = serde_json::json!({
            "status": status,
            "degraded": degraded,
            "total_rounds": round_outputs.len(),
            "max_rounds": max_rounds,
            "attempted": attempted,
            "succeeded": succeeded,
            "failed_debaters": failed_debaters,
            "rounds": round_outputs,
            "consensus": super::build_round_consensus(&round_outputs),
        });

        Ok(NodeOutput { output: final_output, output_var: Some(output_var), control: None })
    }
}
