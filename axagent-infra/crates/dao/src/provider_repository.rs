// SPDX-License-Identifier: AGPL-3.0-only

use async_trait::async_trait;
use sea_orm::DatabaseConnection;

use axagent_harness::core_error::{AxAgentError, Result};
use axagent_harness::repositories::ProviderRepository;
use axagent_harness::types::ProviderConfig;
use axagent_harness::types::ProviderKey;

/// DAO implementation of ProviderRepository.
///
/// Wraps the existing `crate::repo::provider` free functions.
pub struct DaoProviderRepository {
    db: DatabaseConnection,
}

impl DaoProviderRepository {
    pub fn new(db: DatabaseConnection) -> Self {
        Self { db }
    }
}

#[async_trait]
impl ProviderRepository for DaoProviderRepository {
    async fn list_providers(&self) -> Result<Vec<ProviderConfig>> {
        crate::repo::provider::list_providers(&self.db).await
    }

    async fn get_provider(&self, id: &str) -> Result<ProviderConfig> {
        crate::repo::provider::get_provider(&self.db, id).await
    }

    async fn get_active_key(&self, provider_id: &str) -> Result<ProviderKey> {
        crate::repo::provider::get_active_key(&self.db, provider_id).await
    }

    async fn resolve_model_for_node(
        &self,
        node_model: Option<&str>,
        session_model: Option<&str>,
        session_provider_id: Option<&str>,
        profile_suggested_provider: Option<&str>,
    ) -> Result<(ProviderConfig, ProviderKey, String)> {
        // repo 层此函数仍以 `String` 表达错误（保留既有文案），在此适配为权威错误类型。
        crate::repo::provider::resolve_model_for_node(
            &self.db,
            node_model,
            session_model,
            session_provider_id,
            profile_suggested_provider,
        )
        .await
        .map_err(AxAgentError::from)
    }
}
