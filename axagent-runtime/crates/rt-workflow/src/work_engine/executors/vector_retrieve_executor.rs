// SPDX-License-Identifier: AGPL-3.0-only

//! 向量检索执行器 —— 经 `VectorSearchSeam` 接缝真检索。
//!
//! P6.0-⑥ 批 D（PLAN §211）：此前本执行器**恒返回 `not_configured` 却仍是 Ok**
//! ——即「节点成功、结果为空态」的伪装（用户从工作流编辑器能建出该节点、
//! AI 生成路径也会造，跑出来永远是假成功）。
//! 现在：走 `ExecutionState.vector_search` 接缝真检索；接缝缺失 ⇒ **如实报错**
//! （`VECTOR_SEARCH_SEAM_NOT_INJECTED`），不再返回假成功。
//!
//! 注入链（照 §211 批 B/C/D 同型）：harness 契约 `VectorSearchSeam`
//! → 宿主实现转调既有检索全链（`indexing::search_knowledge`：embedding + 向量库
//! + 混合检索都在实现层）→ WorkEngine setter/注入 → init 装配。

use crate::work_engine::execution_state::ExecutionState;
use crate::work_engine::node_executor_trait::{NodeError, NodeExecutorTrait, NodeOutput};
use axagent_harness::workflow_types::WorkflowNode;

pub struct VectorRetrieveExecutor;

impl VectorRetrieveExecutor {
    pub fn new() -> Self {
        Self
    }
}
impl Default for VectorRetrieveExecutor {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait::async_trait]
impl NodeExecutorTrait for VectorRetrieveExecutor {
    fn node_type(&self) -> &'static str {
        "vectorRetrieve"
    }

    async fn execute(
        &self,
        node: &WorkflowNode,
        context: &ExecutionState,
    ) -> Result<NodeOutput, NodeError> {
        let WorkflowNode::VectorRetrieve(vr) = node else {
            return Err(NodeError::type_mismatch(
                "vectorRetrieve".to_string(),
                super::node_type_name(node).to_string(),
            ));
        };
        let resolved_query = resolve_query_template(&vr.config.query, &context.variables);

        let seam = context.vector_search.as_ref().ok_or_else(|| {
            NodeError::exec_failed(
                "VECTOR_SEARCH_SEAM_NOT_INJECTED",
                "vector search seam not injected into ExecutionState",
            )
        })?;

        let hits = seam
            .search_text(
                &vr.config.knowledge_base_id,
                &resolved_query,
                vr.config.top_k as usize,
                vr.config.similarity_threshold,
            )
            .await
            .map_err(|e| NodeError::exec_failed("VECTOR_SEARCH_FAILED", e))?;

        let results: Vec<serde_json::Value> = hits
            .iter()
            .map(|h| {
                serde_json::json!({
                    "id": h.id,
                    "document_id": h.document_id,
                    "chunk_index": h.chunk_index,
                    "content": h.content,
                    "score": h.score,
                })
            })
            .collect();

        Ok(NodeOutput {
            output: serde_json::json!({
                "query": resolved_query,
                "knowledge_base_id": vr.config.knowledge_base_id,
                "top_k": vr.config.top_k,
                "results": results,
                "result_count": hits.len(),
                "node_id": node.base_id(),
            }),
            output_var: Some(vr.config.output_var.clone()),
            control: None,
        })
    }
}

fn resolve_query_template(
    template: &str,
    vars: &std::collections::HashMap<String, serde_json::Value>,
) -> String {
    let mut result = template.to_string();
    for (k, v) in vars {
        let placeholder = format!("{{{{{}}}}}", k);
        let owned = v.to_string();
        let replacement = v.as_str().unwrap_or(&owned);
        result = result.replace(&placeholder, replacement);
    }
    result
}

#[cfg(test)]
mod tests {
    // SAFETY: 测试替身 MockSeam 的 std::sync::Mutex 不跨 await 使用（async 方法内
    // push 后即释放，锁持有不跨越 await 点），临界区仅同步操作；整 mod 的用点
    // 同型，故 mod 级豁免。
    #![allow(clippy::disallowed_types)]
    use super::*;
    use serde_json::json;
    use std::sync::Arc;
    use std::sync::Mutex as StdMutex;

    /// 测试替身：记录调用参数并回投固定命中。
    struct MockSeam {
        calls: StdMutex<Vec<MockSeamCall>>,
        hits: Vec<axagent_harness::VectorSearchResult>,
    }

    #[derive(Clone, Debug, PartialEq)]
    struct MockSeamCall {
        knowledge_base_id: String,
        query: String,
        top_k: usize,
        min_similarity: Option<f32>,
    }

    #[async_trait::async_trait]
    impl axagent_harness::VectorSearchSeam for MockSeam {
        async fn search_text(
            &self,
            knowledge_base_id: &str,
            query: &str,
            top_k: usize,
            min_similarity: Option<f32>,
        ) -> axagent_harness::core_error::Result<Vec<axagent_harness::VectorSearchResult>> {
            self.calls.lock().unwrap().push(MockSeamCall {
                knowledge_base_id: knowledge_base_id.to_string(),
                query: query.to_string(),
                top_k,
                min_similarity,
            });
            Ok(self.hits.clone())
        }
    }

    fn node_with() -> WorkflowNode {
        serde_json::from_value(json!({
            "type": "vectorRetrieve",
            "id": "vr1",
            "title": "向量检索",
            "position": {"x": 0.0, "y": 0.0},
            "retry": {"enabled": false, "maxRetries": 0, "backoffType": "Fixed", "baseDelayMs": 1000, "maxDelayMs": 1000},
            "enabled": true,
            "config": {
                "query": "查 {{q}}",
                "knowledge_base_id": "kb1",
                "top_k": 5,
                "output_var": "vr_out"
            }
        }))
        .unwrap()
    }

    fn ctx_with_seam(seam: Option<Arc<dyn axagent_harness::VectorSearchSeam>>) -> ExecutionState {
        let mut ctx = ExecutionState::new("e1".into(), "w1".into(), json!({}));
        ctx.variables.insert("q".to_string(), json!("苹果"));
        ctx.vector_search = seam;
        ctx
    }

    #[tokio::test]
    async fn missing_seam_reports_not_injected() {
        let ctx = ctx_with_seam(None);
        let err = VectorRetrieveExecutor.execute(&node_with(), &ctx).await.unwrap_err();
        match err {
            NodeError::ExecutionFailed { code, .. } => {
                assert_eq!(code, "VECTOR_SEARCH_SEAM_NOT_INJECTED")
            },
            other => panic!("期望 ExecutionFailed，实际 {other:?}"),
        }
    }

    #[tokio::test]
    async fn seam_receives_resolved_query_and_returns_hits() {
        let seam = Arc::new(MockSeam {
            calls: StdMutex::new(vec![]),
            hits: vec![axagent_harness::VectorSearchResult {
                id: "c1".to_string(),
                document_id: "d1".to_string(),
                chunk_index: 0,
                content: "命中内容".to_string(),
                score: 0.25,
                has_embedding: true,
            }],
        });
        let ctx = ctx_with_seam(Some(seam.clone()));
        let out = VectorRetrieveExecutor.execute(&node_with(), &ctx).await.unwrap();
        let calls = seam.calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].knowledge_base_id, "kb1");
        assert_eq!(calls[0].query, "查 苹果");
        assert_eq!(calls[0].top_k, 5);
        assert_eq!(calls[0].min_similarity, None);
        drop(calls);
        assert_eq!(out.output["result_count"], 1);
        assert_eq!(out.output["results"][0]["content"], "命中内容");
        assert_eq!(out.output_var, Some("vr_out".to_string()));
    }

    #[tokio::test]
    async fn seam_error_maps_to_search_failed() {
        struct FailingSeam;
        #[async_trait::async_trait]
        impl axagent_harness::VectorSearchSeam for FailingSeam {
            async fn search_text(
                &self,
                _knowledge_base_id: &str,
                _query: &str,
                _top_k: usize,
                _min_similarity: Option<f32>,
            ) -> axagent_harness::core_error::Result<Vec<axagent_harness::VectorSearchResult>>
            {
                Err(axagent_harness::AxAgentError::internal("向量库炸了".to_string()))
            }
        }
        let ctx = ctx_with_seam(Some(Arc::new(FailingSeam)));
        let err = VectorRetrieveExecutor.execute(&node_with(), &ctx).await.unwrap_err();
        match err {
            NodeError::ExecutionFailed { code, .. } => assert_eq!(code, "VECTOR_SEARCH_FAILED"),
            other => panic!("期望 ExecutionFailed，实际 {other:?}"),
        }
    }
}
