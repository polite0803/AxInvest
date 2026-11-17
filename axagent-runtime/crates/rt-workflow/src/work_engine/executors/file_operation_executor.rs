// SPDX-License-Identifier: AGPL-3.0-only

use async_trait::async_trait;
use axagent_harness::workflow_types::WorkflowNode;

use crate::work_engine::execution_state::ExecutionState;
use crate::work_engine::node_executor_trait::{NodeError, NodeExecutorTrait, NodeOutput};

pub struct FileOperationExecutor;

impl FileOperationExecutor {
    pub fn new() -> Self {
        Self
    }
}

impl Default for FileOperationExecutor {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl NodeExecutorTrait for FileOperationExecutor {
    fn node_type(&self) -> &'static str {
        "fileOperation"
    }
    async fn execute(
        &self,
        node: &WorkflowNode,
        ctx: &ExecutionState,
    ) -> Result<NodeOutput, NodeError> {
        let WorkflowNode::FileOperation(n) = node else {
            return Err(NodeError::type_mismatch("fileOperation", self.node_type()));
        };
        let c = &n.config;

        // P6.0-⑥ 批 E：std::fs 归实现层；本执行器保留操作白名单与输出包装。
        let io = ctx.file_io.as_ref().ok_or_else(|| {
            NodeError::exec_failed(
                "WORKFLOW_FILE_IO_NOT_INJECTED",
                "workflow file io not injected into ExecutionState",
            )
        })?;

        let result = match c.operation.as_str() {
            "read" => io
                .read_to_string(&c.file_path)
                .map_err(|e| NodeError::exec_failed("file_read_error", e)),
            "write" => {
                if let Some(content) = &c.content {
                    io.write(&c.file_path, content, false)
                        .map_err(|e| NodeError::exec_failed("file_write_error", e))
                        .map(|_| String::new())
                } else {
                    Err(NodeError::exec_failed("empty_content", "No content to write"))
                }
            },
            "delete" => io
                .delete(&c.file_path)
                .map_err(|e| NodeError::exec_failed("file_delete_error", e))
                .map(|_| String::new()),
            "exists" => Ok(if io.exists(&c.file_path) {
                "true"
            } else {
                "false"
            }
            .to_string()),
            _ => Err(NodeError::exec_failed(
                "unknown_operation",
                format!("Unknown operation: {}", c.operation),
            )),
        }?;
        Ok(NodeOutput {
            output: serde_json::json!({"operation": c.operation, "path": c.file_path, "result": result, "node_id": node.base_id()}),
            control: None,
            output_var: if c.output_var.is_empty() {
                None
            } else {
                Some(c.output_var.clone())
            },
        })
    }
}

#[cfg(test)]
mod tests {
    // SAFETY: 测试替身的 std::sync::Mutex 不跨 await 使用（push 后即释放，
    // 锁持有不跨越 await 点），临界区仅同步操作；整 mod 的用点同型，故 mod 级豁免。
    #![allow(clippy::disallowed_types)]
    use super::*;
    use serde_json::json;
    use std::sync::Arc;
    use std::sync::Mutex as StdMutex;

    /// 测试替身：内存文件表。
    struct MemFileIo {
        files: StdMutex<std::collections::HashMap<String, String>>,
        write_parent_flags: StdMutex<Vec<bool>>,
    }

    impl axagent_harness::WorkflowFileIo for MemFileIo {
        fn read_to_string(&self, path: &str) -> Result<String, String> {
            self.files.lock().unwrap().get(path).cloned().ok_or_else(|| format!("不存在: {path}"))
        }
        fn write(&self, path: &str, content: &str, create_parent_dirs: bool) -> Result<(), String> {
            self.write_parent_flags.lock().unwrap().push(create_parent_dirs);
            self.files.lock().unwrap().insert(path.to_string(), content.to_string());
            Ok(())
        }
        fn delete(&self, path: &str) -> Result<(), String> {
            self.files.lock().unwrap().remove(path).map(|_| ()).ok_or_else(|| "不存在".to_string())
        }
        fn exists(&self, path: &str) -> bool {
            self.files.lock().unwrap().contains_key(path)
        }
    }

    fn node_with(op: &str, content: Option<&str>) -> WorkflowNode {
        serde_json::from_value(json!({
            "type": "fileOperation",
            "id": "f1",
            "title": "文件操作",
            "position": {"x": 0.0, "y": 0.0},
            "retry": {"enabled": false, "maxRetries": 0, "backoffType": "Fixed", "baseDelayMs": 1000, "maxDelayMs": 1000},
            "enabled": true,
            "config": {"operation": op, "file_path": "/tmp/x.txt", "content": content, "output_var": "f_out"}
        }))
        .unwrap()
    }

    fn ctx_with(io: Option<Arc<dyn axagent_harness::WorkflowFileIo>>) -> ExecutionState {
        let mut ctx = ExecutionState::new("e1".into(), "w1".into(), json!({}));
        ctx.file_io = io;
        ctx
    }

    #[tokio::test]
    async fn missing_seam_reports_not_injected() {
        let ctx = ctx_with(None);
        let err = FileOperationExecutor.execute(&node_with("read", None), &ctx).await.unwrap_err();
        match err {
            NodeError::ExecutionFailed { code, .. } => {
                assert_eq!(code, "WORKFLOW_FILE_IO_NOT_INJECTED")
            },
            other => panic!("期望 ExecutionFailed，实际 {other:?}"),
        }
    }

    #[tokio::test]
    async fn write_then_read_roundtrip_via_seam() {
        let io = Arc::new(MemFileIo {
            files: StdMutex::new(std::collections::HashMap::new()),
            write_parent_flags: StdMutex::new(vec![]),
        });
        let ctx = ctx_with(Some(io.clone()));
        FileOperationExecutor.execute(&node_with("write", Some("hello")), &ctx).await.unwrap();
        let out = FileOperationExecutor.execute(&node_with("read", None), &ctx).await.unwrap();
        assert_eq!(out.output["result"], "hello");
        assert_eq!(out.output_var, Some("f_out".to_string()));
        // fileOperation 的 write 语义 = 不建父目录（create_parent_dirs=false）
        assert_eq!(io.write_parent_flags.lock().unwrap().as_slice(), &[false]);
    }

    #[tokio::test]
    async fn exists_reflects_seam_state() {
        let io = Arc::new(MemFileIo {
            files: StdMutex::new(std::collections::HashMap::new()),
            write_parent_flags: StdMutex::new(vec![]),
        });
        let ctx = ctx_with(Some(io.clone()));
        let out = FileOperationExecutor.execute(&node_with("exists", None), &ctx).await.unwrap();
        assert_eq!(out.output["result"], "false");
        FileOperationExecutor.execute(&node_with("write", Some("x")), &ctx).await.unwrap();
        let out = FileOperationExecutor.execute(&node_with("exists", None), &ctx).await.unwrap();
        assert_eq!(out.output["result"], "true");
    }

    #[tokio::test]
    async fn unknown_operation_still_rejected() {
        let io = Arc::new(MemFileIo {
            files: StdMutex::new(std::collections::HashMap::new()),
            write_parent_flags: StdMutex::new(vec![]),
        });
        let ctx = ctx_with(Some(io));
        let err =
            FileOperationExecutor.execute(&node_with("rename", None), &ctx).await.unwrap_err();
        match err {
            NodeError::ExecutionFailed { code, .. } => assert_eq!(code, "unknown_operation"),
            other => panic!("期望 ExecutionFailed，实际 {other:?}"),
        }
    }
}
