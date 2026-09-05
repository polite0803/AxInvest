// SPDX-License-Identifier: AGPL-3.0-only

//! Gateway `StockStore` 接缝的 entities 后端实现。
//!
//! 复用 `axagent_entities` 的 `stock_analyses` / `watchlist_items`，
//! 以 `serde_json::Value` 返回（gateway 侧无需镜像 ~30 字段 DTO）。
//! 与 `gateway_memory_store::DaoMemoryStore` 同款接缝注入模式：
//! 主 crate wiring 层构造，注入 `GatewayAppState`。

use axagent_entities::{stock_analyses, watchlist_items};
use axagent_harness::core_error::Result;
use axagent_harness::stock_service::StockStore;
use sea_orm::{ActiveModelTrait, EntityTrait, QueryOrder, QuerySelect, Set};
use serde_json::Value;

/// entities 后端的 StockStore 运行时。
pub struct DaoStockStore {
    db: sea_orm::DatabaseConnection,
}

impl DaoStockStore {
    pub fn new(db: sea_orm::DatabaseConnection) -> Self {
        Self { db }
    }

    fn to_value<T: serde::Serialize>(value: T) -> Value {
        serde_json::to_value(value).unwrap_or(Value::Null)
    }
}

#[async_trait::async_trait]
impl StockStore for DaoStockStore {
    async fn list_analyses(&self, limit: u64, offset: u64) -> Result<Vec<Value>> {
        let records = stock_analyses::Entity::find()
            .order_by_desc(stock_analyses::Column::CreatedAt)
            .limit(Some(limit))
            .offset(Some(offset))
            .all(&self.db)
            .await?
            .into_iter()
            .map(Self::to_value)
            .collect();
        Ok(records)
    }

    async fn get_analysis(&self, analysis_id: &str) -> Result<Option<Value>> {
        let record = stock_analyses::Entity::find_by_id(analysis_id).one(&self.db).await?;
        Ok(record.map(Self::to_value))
    }

    async fn list_watchlist(&self) -> Result<Vec<Value>> {
        let items = watchlist_items::Entity::find()
            .order_by_desc(watchlist_items::Column::CreatedAt)
            .all(&self.db)
            .await?
            .into_iter()
            .map(Self::to_value)
            .collect();
        Ok(items)
    }

    async fn add_watchlist(&self, stock_code: &str, stock_name: &str) -> Result<Value> {
        let now = chrono::Utc::now().timestamp_millis();
        let model = watchlist_items::ActiveModel {
            id: Set(uuid::Uuid::new_v4().to_string()),
            stock_code: Set(stock_code.to_string()),
            stock_name: Set(stock_name.to_string()),
            notes: Set(None),
            created_at: Set(now),
            updated_at: Set(now),
        };
        let record = model.insert(&self.db).await?;
        Ok(Self::to_value(record))
    }

    async fn delete_watchlist(&self, id: &str) -> Result<bool> {
        let result = watchlist_items::Entity::delete_by_id(id).exec(&self.db).await?;
        Ok(result.rows_affected > 0)
    }
}
