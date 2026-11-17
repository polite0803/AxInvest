// SPDX-License-Identifier: AGPL-3.0-only

//! 文档解析执行器 —— 经 `DocumentParser` 接缝真解析。
//!
//! P6.0-⑥（PLAN §211）：解析实现（PDF/Office/OCR 等格式分派）归 document-parser
//! crate，本执行器只做「变量取值分路 → 调接缝 → 包装 NodeOutput」：
//! - 变量值是**现存文件路径** ⇒ 经 `ExecutionState.document_parser` 真解析（全文）；
//! - 变量值是**纯文本** ⇒ 视为上游已解析内容直通（mode=inline），不伪造解析；
//! - 两者都拿不到 ⇒ 如实报错，绝不返回假 `parsed` 成功。

use async_trait::async_trait;
use axagent_harness::workflow_types::WorkflowNode;
use std::path::Path;

use crate::work_engine::execution_state::ExecutionState;
use crate::work_engine::node_executor_trait::{NodeError, NodeExecutorTrait, NodeOutput};

pub struct DocumentParserExecutor;

impl DocumentParserExecutor {
    pub fn new() -> Self {
        Self
    }
}

impl Default for DocumentParserExecutor {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl NodeExecutorTrait for DocumentParserExecutor {
    fn node_type(&self) -> &'static str {
        "documentParser"
    }

    async fn execute(
        &self,
        node: &WorkflowNode,
        context: &ExecutionState,
    ) -> Result<NodeOutput, NodeError> {
        let WorkflowNode::DocumentParser(dp) = node else {
            return Err(NodeError::type_mismatch(
                "documentParser".to_string(),
                super::node_type_name(node).to_string(),
            ));
        };

        let input = context
            .variables
            .get(&dp.config.input_var)
            .ok_or_else(|| NodeError::VariableNotFound(dp.config.input_var.clone()))?;
        let input_str = input.as_str().ok_or_else(|| {
            NodeError::exec_failed(
                "DOCUMENT_INPUT_NOT_STRING",
                format!("变量 {} 不是字符串", dp.config.input_var),
            )
        })?;

        // 路径存在性判断是路由分叉（非解析实现）；解析本身全部经接缝。
        let (mode, content) = if Path::new(input_str).is_file() {
            let parser = context.document_parser.as_ref().ok_or_else(|| {
                NodeError::exec_failed(
                    "DOCUMENT_PARSER_NOT_INJECTED",
                    "document parser not injected into ExecutionState",
                )
            })?;
            let text = parser
                .extract_text_by_path(Path::new(input_str))
                .map_err(|e| NodeError::exec_failed("DOCUMENT_PARSE_FAILED", e))?;
            ("parsed", text)
        } else {
            ("inline", input_str.to_string())
        };

        let content_preview: String = content.chars().take(500).collect();
        let output = serde_json::json!({
            "status": mode,
            "parser_type": dp.config.parser_type,
            "input_var": dp.config.input_var,
            "content_length": content.len(),
            "content_preview": content_preview,
            "content": content,
            "node_id": node.base_id(),
        });

        Ok(NodeOutput { output, output_var: Some(dp.config.output_var.clone()), control: None })
    }
}

#[cfg(test)]
mod tests {
    // SAFETY: 测试替身 MockParser 的 std::sync::Mutex 不跨 await 使用（同步方法内
    // push 后即释放），临界区仅同步操作；整 mod 的用点同型，故 mod 级豁免。
    #![allow(clippy::disallowed_types)]
    use super::*;
    use serde_json::json;
    use std::sync::Arc;
    use std::sync::Mutex as StdMutex;

    struct MockParser {
        calls: StdMutex<Vec<String>>,
        result: Result<String, String>,
    }

    impl axagent_harness::DocumentParser for MockParser {
        fn extract_text(
            &self,
            file_path: &Path,
            _mime_type: &str,
        ) -> axagent_harness::core_error::Result<String> {
            self.calls.lock().unwrap().push(file_path.display().to_string());
            self.result.clone().map_err(axagent_harness::AxAgentError::Validation)
        }
    }

    fn node_with(input_var: &str) -> WorkflowNode {
        serde_json::from_value(json!({
            "type": "documentParser",
            "id": "dp1",
            "title": "文档解析",
            "position": {"x": 0.0, "y": 0.0},
            "retry": {"enabled": false, "maxRetries": 0, "backoffType": "Fixed", "baseDelayMs": 1000, "maxDelayMs": 1000},
            "enabled": true,
            "config": {"input_var": input_var, "parser_type": "pdf", "output_var": "doc_text"}
        }))
        .unwrap()
    }

    fn ctx_with(vars: serde_json::Value) -> ExecutionState {
        let mut ctx = ExecutionState::new("e1".into(), "w1".into(), vars.clone());
        if let Some(obj) = vars.as_object() {
            for (k, v) in obj {
                ctx.variables.insert(k.clone(), v.clone());
            }
        }
        ctx
    }

    #[tokio::test]
    async fn missing_variable_reports_not_found() {
        let ctx = ctx_with(json!({}));
        let err = DocumentParserExecutor.execute(&node_with("doc"), &ctx).await.unwrap_err();
        assert!(matches!(err, NodeError::VariableNotFound(_)));
    }

    #[tokio::test]
    async fn non_string_variable_reports_type_error() {
        let ctx = ctx_with(json!({"doc": {"a": 1}}));
        let err = DocumentParserExecutor.execute(&node_with("doc"), &ctx).await.unwrap_err();
        match err {
            NodeError::ExecutionFailed { code, .. } => {
                assert_eq!(code, "DOCUMENT_INPUT_NOT_STRING")
            },
            other => panic!("期望 ExecutionFailed，实际 {other:?}"),
        }
    }

    #[tokio::test]
    async fn plain_text_passes_through_as_inline() {
        let ctx = ctx_with(json!({"doc": "已经是纯文本内容"}));
        let out = DocumentParserExecutor.execute(&node_with("doc"), &ctx).await.unwrap();
        assert_eq!(out.output["status"], "inline");
        assert_eq!(out.output["content"], "已经是纯文本内容");
        assert_eq!(out.output["content_length"], "已经是纯文本内容".len());
        assert_eq!(out.output_var, Some("doc_text".to_string()));
    }

    #[tokio::test]
    async fn existing_file_path_goes_through_seam() {
        let tmp = std::env::temp_dir().join(format!("axagent-dp-{}-a.txt", std::process::id()));
        std::fs::write(&tmp, "file-body").unwrap();
        let parser = Arc::new(MockParser {
            calls: StdMutex::new(vec![]),
            result: Ok("解析后全文".into()),
        });
        let mut ctx = ctx_with(json!({"doc": tmp.display().to_string()}));
        ctx.document_parser = Some(parser.clone());
        let out = DocumentParserExecutor.execute(&node_with("doc"), &ctx).await.unwrap();
        let calls = parser.calls.lock().unwrap();
        assert_eq!(calls.as_slice(), &[tmp.display().to_string()]);
        drop(calls);
        assert_eq!(out.output["status"], "parsed");
        assert_eq!(out.output["content"], "解析后全文");
        let _ = std::fs::remove_file(&tmp);
    }

    #[tokio::test]
    async fn file_path_without_parser_reports_not_injected() {
        let tmp = std::env::temp_dir().join(format!("axagent-dp-{}-b.txt", std::process::id()));
        std::fs::write(&tmp, "x").unwrap();
        let ctx = ctx_with(json!({"doc": tmp.display().to_string()}));
        let err = DocumentParserExecutor.execute(&node_with("doc"), &ctx).await.unwrap_err();
        match err {
            NodeError::ExecutionFailed { code, .. } => {
                assert_eq!(code, "DOCUMENT_PARSER_NOT_INJECTED")
            },
            other => panic!("期望 ExecutionFailed，实际 {other:?}"),
        }
        let _ = std::fs::remove_file(&tmp);
    }

    #[tokio::test]
    async fn parser_error_maps_to_parse_failed() {
        let tmp = std::env::temp_dir().join(format!("axagent-dp-{}-c.txt", std::process::id()));
        std::fs::write(&tmp, "x").unwrap();
        let parser = Arc::new(MockParser {
            calls: StdMutex::new(vec![]),
            result: Err("解析器崩了".into()),
        });
        let mut ctx = ctx_with(json!({"doc": tmp.display().to_string()}));
        ctx.document_parser = Some(parser);
        let err = DocumentParserExecutor.execute(&node_with("doc"), &ctx).await.unwrap_err();
        match err {
            NodeError::ExecutionFailed { code, .. } => assert_eq!(code, "DOCUMENT_PARSE_FAILED"),
            other => panic!("期望 ExecutionFailed，实际 {other:?}"),
        }
        let _ = std::fs::remove_file(&tmp);
    }
}
