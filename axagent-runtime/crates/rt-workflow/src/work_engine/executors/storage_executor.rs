// SPDX-License-Identifier: AGPL-3.0-only

use async_trait::async_trait;
use axagent_harness::workflow_types::WorkflowNode;

use crate::work_engine::execution_state::ExecutionState;
use crate::work_engine::node_executor_trait::{
    NodeError, NodeExecutorTrait, NodeOutput, error_code,
};

pub struct StorageExecutor;

impl StorageExecutor {
    pub fn new() -> Self {
        Self
    }
}

impl Default for StorageExecutor {
    fn default() -> Self {
        Self::new()
    }
}

/// 从 ExecutionState.variables 按 path 提取值
fn resolve_var_path(path: &str, ctx: &ExecutionState) -> Option<serde_json::Value> {
    if path.is_empty() {
        return None;
    }
    let mut current = ctx.variables.get(path)?.clone();
    for segment in path.split('.').skip(1) {
        current = current.get(segment)?.clone();
    }
    Some(current)
}

#[async_trait]
impl NodeExecutorTrait for StorageExecutor {
    fn node_type(&self) -> &'static str {
        "storage"
    }
    async fn execute(
        &self,
        node: &WorkflowNode,
        context: &ExecutionState,
    ) -> Result<NodeOutput, NodeError> {
        let WorkflowNode::Storage(n) = node else {
            return Err(NodeError::type_mismatch("storage", self.node_type()));
        };
        let c = &n.config;

        // 1. 获取输入数据
        let input_data = match resolve_var_path(&c.input_var, context) {
            Some(v) => v,
            None => {
                return Err(NodeError::exec_failed(
                    error_code::VARIABLE_NOT_FOUND,
                    format!("Storage: input_var '{}' not found", c.input_var),
                ));
            },
        };

        // 2. 获取 upsert key（可选）
        let key = if let Some(ref key_var) = c.key_var {
            resolve_var_path(key_var, context)
        } else {
            None
        };

        // 3. 根据后端和操作模式执行存储
        let (rows_affected, status_msg) = match c.backend.as_str() {
            "sqlite" => {
                // SQLite 后端：经 WorkflowKvStore 接缝真写入宿主 KV 存储。
                // 无接缝时如实报错（不再返回假成功）。
                let kv = context.kv_store.as_ref().ok_or_else(|| {
                    NodeError::exec_failed(
                        "WORKFLOW_KV_STORE_NOT_INJECTED",
                        "workflow kv store not injected into ExecutionState",
                    )
                })?;
                let mode = match c.operation.as_str() {
                    "upsert" => "upsert",
                    "append" => "append",
                    _ => "insert",
                };
                let key = key.as_ref().and_then(|v| v.as_str()).unwrap_or(&c.collection);
                let content =
                    serde_json::to_string(&input_data).unwrap_or_else(|_| input_data.to_string());
                kv.write(&c.collection, key, &content, mode).await.map_err(|e| {
                    NodeError::exec_failed(
                        error_code::IO_ERROR,
                        format!("Storage kv write failed: {e}"),
                    )
                })?;
                (1, format!("sqlite_{mode}"))
            },
            "vectorDb" => {
                // VectorDB 后端：经 WorkflowVectorWriter 接缝真写入向量库元数据文本。
                // storage 节点输入无 embedding 生成环节，故只落文本（复用宿主
                // insert_metadata_only_chunk 语义），不伪造向量；无接缝如实报错。
                let writer = context.vector_writer.as_ref().ok_or_else(|| {
                    NodeError::exec_failed(
                        "WORKFLOW_VECTOR_WRITER_NOT_INJECTED",
                        "workflow vector writer not injected into ExecutionState",
                    )
                })?;
                let mode = match c.operation.as_str() {
                    "upsert" => "upsert",
                    _ => "insert",
                };
                let key = key.as_ref().and_then(|v| v.as_str()).unwrap_or(&c.collection);
                let content =
                    serde_json::to_string(&input_data).unwrap_or_else(|_| input_data.to_string());
                writer.insert_text(&c.collection, key, &content, mode).await.map_err(|e| {
                    NodeError::exec_failed(
                        error_code::IO_ERROR,
                        format!("Storage vector write failed: {e}"),
                    )
                })?;
                (1, format!("vectorDb_{mode}"))
            },
            "fileSystem" => {
                // P6.0-⑥ 批 E：std::fs 归实现层；append 语义（先读后拼再写）留在执行器。
                let io = context.file_io.as_ref().ok_or_else(|| {
                    NodeError::exec_failed(
                        "WORKFLOW_FILE_IO_NOT_INJECTED",
                        "workflow file io not injected into ExecutionState",
                    )
                })?;
                match c.operation.as_str() {
                    "append" => {
                        let existing = io.read_to_string(&c.collection).unwrap_or_default();
                        let content = input_data.to_string();
                        io.write(&c.collection, &format!("{}{}", existing, content), false)
                            .map_err(|e| {
                                NodeError::exec_failed(
                                    error_code::IO_ERROR,
                                    format!("Storage write failed: {e}"),
                                )
                            })?;
                        (1, "fileSystem_append".to_string())
                    },
                    _ => {
                        // insert / upsert → 覆盖写（父目录由实现层按需建）
                        let content = input_data.to_string();
                        io.write(&c.collection, &content, true).map_err(|e| {
                            NodeError::exec_failed(
                                error_code::IO_ERROR,
                                format!("Storage write failed: {e}"),
                            )
                        })?;
                        (1, "fileSystem_write".to_string())
                    },
                }
            },
            other => {
                return Err(NodeError::exec_failed(
                    error_code::VALIDATION_FAILED,
                    format!("Storage: unknown backend '{other}'"),
                ));
            },
        };

        Ok(NodeOutput {
            output: serde_json::json!({
                "backend": c.backend,
                "operation": c.operation,
                "collection": c.collection,
                "rows_affected": rows_affected,
                "status": status_msg,
                "key": key,
                "node_id": node.base_id(),
            }),
            output_var: if c.output_var.is_empty() {
                None
            } else {
                Some(c.output_var.clone())
            },
            control: None,
        })
    }
}

#[cfg(test)]
mod tests {
    // SAFETY: 测试替身的 std::sync::Mutex 不跨 await 使用（push 后即释放，
    // 锁持有不跨越 await 点），临界区仅同步操作；整 mod 的用点同型，故 mod 级豁免。
    #![allow(clippy::disallowed_types)]
    use super::*;
    use axagent_harness::WorkflowFileIo;
    use serde_json::json;
    use std::sync::Arc;
    use std::sync::Mutex as StdMutex;

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

    fn node_with(backend: &str, op: &str) -> WorkflowNode {
        serde_json::from_value(json!({
            "type": "storage",
            "id": "s1",
            "title": "存储",
            "position": {"x": 0.0, "y": 0.0},
            "retry": {"enabled": false, "maxRetries": 0, "backoffType": "Fixed", "baseDelayMs": 1000, "maxDelayMs": 1000},
            "enabled": true,
            "config": {
                "backend": backend,
                "operation": op,
                "collection": "/data/coll.json",
                "input_var": "v",
                "output_var": "s_out"
            }
        }))
        .unwrap()
    }

    fn ctx_with(io: Option<Arc<dyn axagent_harness::WorkflowFileIo>>) -> ExecutionState {
        let mut ctx = ExecutionState::new("e1".into(), "w1".into(), json!({"v": {"a": 1}}));
        ctx.variables.insert("v".to_string(), json!({"a": 1}));
        ctx.file_io = io;
        ctx
    }

    /// 同时实现两个新写口的内存替身，记录每次写入的 (collection, key, content, mode)。
    #[derive(Default)]
    struct MemWrites {
        kv: StdMutex<Vec<(String, String, String, String)>>,
        vec: StdMutex<Vec<(String, String, String, String)>>,
    }

    #[async_trait::async_trait]
    impl axagent_harness::WorkflowKvStore for MemWrites {
        async fn write(
            &self,
            collection: &str,
            key: &str,
            value: &str,
            mode: &str,
        ) -> Result<(), String> {
            self.kv.lock().unwrap().push((
                collection.to_string(),
                key.to_string(),
                value.to_string(),
                mode.to_string(),
            ));
            Ok(())
        }
    }

    #[async_trait::async_trait]
    impl axagent_harness::WorkflowVectorWriter for MemWrites {
        async fn insert_text(
            &self,
            collection: &str,
            key: &str,
            content: &str,
            mode: &str,
        ) -> Result<(), String> {
            self.vec.lock().unwrap().push((
                collection.to_string(),
                key.to_string(),
                content.to_string(),
                mode.to_string(),
            ));
            Ok(())
        }
    }

    #[tokio::test]
    async fn file_system_without_seam_reports_not_injected() {
        let ctx = ctx_with(None);
        let err =
            StorageExecutor.execute(&node_with("fileSystem", "insert"), &ctx).await.unwrap_err();
        match err {
            NodeError::ExecutionFailed { code, .. } => {
                assert_eq!(code, "WORKFLOW_FILE_IO_NOT_INJECTED")
            },
            other => panic!("期望 ExecutionFailed，实际 {other:?}"),
        }
    }

    #[tokio::test]
    async fn insert_writes_with_parent_dirs() {
        let io = Arc::new(MemFileIo {
            files: StdMutex::new(std::collections::HashMap::new()),
            write_parent_flags: StdMutex::new(vec![]),
        });
        let ctx = ctx_with(Some(io.clone()));
        let out = StorageExecutor.execute(&node_with("fileSystem", "insert"), &ctx).await.unwrap();
        assert_eq!(out.output["status"], "fileSystem_write");
        assert_eq!(io.write_parent_flags.lock().unwrap().as_slice(), &[true]);
    }

    #[tokio::test]
    async fn append_preserves_existing_content() {
        let io = Arc::new(MemFileIo {
            files: StdMutex::new(std::collections::HashMap::new()),
            write_parent_flags: StdMutex::new(vec![]),
        });
        io.write("/data/coll.json", "HEAD;", false).unwrap();
        let ctx = ctx_with(Some(io.clone()));
        let out = StorageExecutor.execute(&node_with("fileSystem", "append"), &ctx).await.unwrap();
        assert_eq!(out.output["status"], "fileSystem_append");
        let stored = io.read_to_string("/data/coll.json").unwrap();
        assert!(stored.starts_with("HEAD;"), "append 必须保留已有内容");
        assert!(stored.contains("\"a\":1"), "append 必须追加新内容");
    }

    #[tokio::test]
    async fn unknown_backend_still_rejected() {
        let io = Arc::new(MemFileIo {
            files: StdMutex::new(std::collections::HashMap::new()),
            write_parent_flags: StdMutex::new(vec![]),
        });
        let ctx = ctx_with(Some(io));
        let err = StorageExecutor.execute(&node_with("redis", "insert"), &ctx).await.unwrap_err();
        match err {
            NodeError::ExecutionFailed { code, .. } => {
                assert_eq!(code, error_code::VALIDATION_FAILED)
            },
            other => panic!("期望 ExecutionFailed，实际 {other:?}"),
        }
    }

    // ---- sqlite / vectorDb 接真（替代骨架假成功）----

    #[tokio::test]
    async fn sqlite_without_kv_seam_reports_not_injected() {
        let ctx = ctx_with(None);
        let err = StorageExecutor.execute(&node_with("sqlite", "upsert"), &ctx).await.unwrap_err();
        match err {
            NodeError::ExecutionFailed { code, .. } => {
                assert_eq!(code, "WORKFLOW_KV_STORE_NOT_INJECTED")
            },
            other => panic!("期望 ExecutionFailed，实际 {other:?}"),
        }
    }

    #[tokio::test]
    async fn sqlite_writes_through_kv_seam() {
        let writes = Arc::new(MemWrites::default());
        let mut ctx = ctx_with(None);
        ctx.kv_store = Some(writes.clone());
        let out = StorageExecutor.execute(&node_with("sqlite", "upsert"), &ctx).await.unwrap();
        assert_eq!(out.output["status"], "sqlite_upsert");
        let calls = writes.kv.lock().unwrap();
        assert_eq!(calls.len(), 1);
        let (collection, key, content, mode) = &calls[0];
        assert_eq!(collection, "/data/coll.json");
        // key_var 未配置 ⇒ 回退到 collection 作为键
        assert_eq!(key, "/data/coll.json");
        assert_eq!(mode, "upsert");
        assert_eq!(serde_json::from_str::<serde_json::Value>(content).unwrap(), json!({"a": 1}));
        drop(calls);
        // 关键：无接缝时不再假成功
        let bare = ctx_with(None);
        assert!(StorageExecutor.execute(&node_with("sqlite", "upsert"), &bare).await.is_err());
    }

    #[tokio::test]
    async fn vector_db_without_writer_seam_reports_not_injected() {
        let ctx = ctx_with(None);
        let err =
            StorageExecutor.execute(&node_with("vectorDb", "insert"), &ctx).await.unwrap_err();
        match err {
            NodeError::ExecutionFailed { code, .. } => {
                assert_eq!(code, "WORKFLOW_VECTOR_WRITER_NOT_INJECTED")
            },
            other => panic!("期望 ExecutionFailed，实际 {other:?}"),
        }
    }

    #[tokio::test]
    async fn vector_db_writes_through_writer_seam() {
        let writes = Arc::new(MemWrites::default());
        let mut ctx = ctx_with(None);
        ctx.vector_writer = Some(writes.clone());
        let out = StorageExecutor.execute(&node_with("vectorDb", "insert"), &ctx).await.unwrap();
        assert_eq!(out.output["status"], "vectorDb_insert");
        let calls = writes.vec.lock().unwrap();
        assert_eq!(calls.len(), 1);
        let (collection, key, content, mode) = &calls[0];
        assert_eq!(collection, "/data/coll.json");
        assert_eq!(key, "/data/coll.json");
        assert_eq!(mode, "insert");
        assert_eq!(serde_json::from_str::<serde_json::Value>(content).unwrap(), json!({"a": 1}));
    }
}
