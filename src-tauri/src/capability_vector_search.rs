// SPDX-License-Identifier: AGPL-3.0-only

//! 知识库文本检索接缝的宿主实现（P6.0-⑥ 批 D）。
//!
//! 工作流 `vectorRetrieve` 节点此前**恒返回 `not_configured` 却仍是 Ok**（假成功）。
//! 本实现把节点接到宿主既有的检索全链（`indexing::search_knowledge`：embedding
//! 生成 + 向量库检索 + 混合检索 + 阈值过滤全在那一层），**不新造第二套检索**。
//!
//! 架构定位：wiring 层（src/）持有 DB 句柄 / master_key / VectorStore 实例，
//! harness 与 rt-workflow 只认 `VectorSearchSeam` 契约。

use std::sync::Arc;

use async_trait::async_trait;
use axagent_harness::core_error::{AxAgentError, Result};
use axagent_harness::vector_store::{VectorSearchResult, VectorSearchSeam};
use axagent_search::vector_store::VectorStore;
use sea_orm::DatabaseConnection;

/// 宿主侧知识库检索接缝实现。
pub struct HostVectorSearch {
    db: DatabaseConnection,
    master_key: [u8; 32],
    vector_store: Arc<VectorStore>,
}

impl HostVectorSearch {
    pub fn new(
        db: DatabaseConnection,
        master_key: [u8; 32],
        vector_store: Arc<VectorStore>,
    ) -> Self {
        Self { db, master_key, vector_store }
    }
}

#[async_trait]
impl VectorSearchSeam for HostVectorSearch {
    async fn search_text(
        &self,
        knowledge_base_id: &str,
        query: &str,
        top_k: usize,
        min_similarity: Option<f32>,
    ) -> Result<Vec<VectorSearchResult>> {
        crate::indexing::search_knowledge(
            &self.db,
            &self.master_key,
            &self.vector_store,
            knowledge_base_id,
            query,
            top_k,
            min_similarity,
        )
        .await
        .map_err(|e| AxAgentError::internal(e.to_string()))
    }
}
