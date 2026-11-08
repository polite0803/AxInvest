// SPDX-License-Identifier: AGPL-3.0-only

use serde::{Deserialize, Serialize};

use async_trait::async_trait;

use axagent_entities::opc_content_assets;

use super::error::OpcResult;

/// 内容资产 DTO
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContentAsset {
    pub id: String,
    pub title: String,
    pub content_type: String,
    pub body: String,
    pub tags: Vec<String>,
    pub status: String,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateContentAssetInput {
    pub title: String,
    pub content_type: String,
    pub body: String,
    pub tags: Vec<String>,
}

/// 内容资产更新入参（禁区 12：权威定义在 `axagent_harness::opc_store` —— 它要跨进程过帧，
/// 插件进程链接不到本实现层）。本层保留 re-export 垫片 ⇒ `ContentAssetService` 的签名与
/// 所有既有调用方**零改动**；新增 `Option` 字段时只改 harness 一处。
pub use axagent_harness::opc_store::UpdateContentAssetInput;

// ── Entity ↔ DTO 转换 ──────────────────────────────────────────

pub(crate) fn content_asset_entity_to_dto(e: opc_content_assets::Model) -> ContentAsset {
    let tags: Vec<String> = serde_json::from_str(&e.tags_json).unwrap_or_default();
    ContentAsset {
        id: e.id,
        title: e.title,
        content_type: e.content_type,
        body: e.body,
        tags,
        status: e.status,
        created_at: e.created_at,
        updated_at: e.updated_at,
    }
}

// ── ContentAssetService trait ─────────────────────────────────────

/// ContentAsset 相关方法
#[async_trait]
pub trait ContentAssetService: Send + Sync {
    async fn create_content_asset(&self, input: CreateContentAssetInput)
    -> OpcResult<ContentAsset>;
    async fn get_content_asset(&self, id: &str) -> OpcResult<ContentAsset>;
    async fn list_content_assets(&self) -> OpcResult<Vec<ContentAsset>>;
    async fn update_content_asset(
        &self,
        id: &str,
        input: UpdateContentAssetInput,
    ) -> OpcResult<ContentAsset>;
    async fn delete_content_asset(&self, id: &str) -> OpcResult<()>;
}
