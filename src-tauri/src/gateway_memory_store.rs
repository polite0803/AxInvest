// SPDX-License-Identifier: AGPL-3.0-only

//! Gateway MemoryStore 接缝的 DAO 运行时实现。
//!
//! 把 `axagent_dao::repo::memory`（真实三层记忆系统：namespace/item/tier/importance）
//! 适配为 `axagent_harness::memory::MemoryStore`，供 gateway 记忆外溢 HTTP handlers
//! 消费（消除 gateway → dao/main-crate 直接依赖，沿用 mcp_store / marketplace_service
//! 同款接缝注入模式）。
//!
//! 设计取舍：
//! - embedding 检索走关键词 LIKE（`search_items`）；向量索引队列需要 AppHandle，
//!   gateway 启动上下文不持有，落库条目 index_status 由 dao 默认处理。
//! - `MemoryAddRequest.importance`（u8 0~100）映射到 item 的 f64 0.0~1.0。
//! - `submit_feedback`：helpful → promote_item（tier 晋升），否则 demote_item，
//!   与三层记忆系统的升降级语义一一对应。
//! - `get_working_memory`：取 tier=working 条目按 importance 降序拼接。

use axagent_dao::repo::memory as mem;
use axagent_harness::memory::{
    MemoryActionResultDto, MemoryAddRequest, MemoryFeedbackRequest, MemoryGroupedDto,
    MemorySearchItem, MemorySearchRequest, MemoryStore, MemoryTreeItem, MemoryUpdateRequest,
};
use axagent_harness::types::{
    CreateMemoryItemInput, CreateMemoryNamespaceInput, UpdateMemoryItemInput,
};
use sea_orm::DatabaseConnection;
use std::collections::BTreeMap;

/// DAO 后端的 MemoryStore 运行时（主 crate wiring 层构造，注入 GatewayAppState）。
pub struct DaoMemoryStore {
    db: DatabaseConnection,
}

impl DaoMemoryStore {
    pub fn new(db: DatabaseConnection) -> Self {
        Self { db }
    }

    /// 按名称解析命名空间；不存在时创建（scope=global）。`None` 回退第一个命名空间，
    /// 库为空时创建 `default`。
    async fn resolve_namespace(&self, name: Option<&str>) -> Result<String, String> {
        let namespaces =
            mem::list_namespaces(&self.db).await.map_err(|e| format!("list namespaces: {e}"))?;

        if let Some(want) = name {
            if let Some(ns) = namespaces.iter().find(|ns| ns.name == want) {
                return Ok(ns.id.clone());
            }
            let created = mem::create_namespace(
                &self.db,
                CreateMemoryNamespaceInput {
                    name: want.to_string(),
                    scope: "global".to_string(),
                    embedding_provider: None,
                    embedding_dimensions: None,
                    retrieval_threshold: None,
                    retrieval_top_k: None,
                    icon_type: None,
                    icon_value: None,
                },
            )
            .await
            .map_err(|e| format!("create namespace: {e}"))?;
            return Ok(created.id);
        }

        if let Some(first) = namespaces.first() {
            return Ok(first.id.clone());
        }
        let created = mem::create_namespace(
            &self.db,
            CreateMemoryNamespaceInput {
                name: "default".to_string(),
                scope: "global".to_string(),
                embedding_provider: None,
                embedding_dimensions: None,
                retrieval_threshold: None,
                retrieval_top_k: None,
                icon_type: None,
                icon_value: None,
            },
        )
        .await
        .map_err(|e| format!("create default namespace: {e}"))?;
        Ok(created.id)
    }

    fn to_search_item(
        item: &axagent_harness::types::MemoryItem,
        ns_name: Option<&str>,
    ) -> MemorySearchItem {
        MemorySearchItem {
            id: item.id.clone(),
            content: item.content.clone(),
            namespace: ns_name.map(str::to_string),
            importance: ((item.importance * 100.0).round() as u8).min(100),
            tags: item.tags.clone(),
            created_at: item.updated_at.clone(),
        }
    }
}

#[async_trait::async_trait]
impl MemoryStore for DaoMemoryStore {
    async fn add_memory(&self, req: MemoryAddRequest) -> Result<MemoryActionResultDto, String> {
        let namespace_id = self.resolve_namespace(req.namespace.as_deref()).await?;
        // 字符边界安全截断（标题仅用于列表展示）
        let title: String = req.content.chars().take(30).collect();
        let importance = Some((req.importance.unwrap_or(50).clamp(0, 100) as f64) / 100.0);

        mem::add_item(
            &self.db,
            CreateMemoryItemInput {
                namespace_id,
                title,
                content: req.content,
                source: Some("gateway".to_string()),
                tier: None,
                importance,
                memory_nature: None,
                tags: req.tags,
                decay_rate: None,
                expires_at: None,
                applicability_tags: None,
                confirmed: None,
                source_conversation_id: None,
                source_message_id: None,
            },
        )
        .await
        .map(|_| MemoryActionResultDto { success: true, error: None })
        .map_err(|e| format!("add memory item: {e}"))
    }

    async fn search_memories(
        &self,
        req: MemorySearchRequest,
    ) -> Result<Vec<MemorySearchItem>, String> {
        let limit = req.limit.unwrap_or(20).clamp(1, 200);
        let namespaces =
            mem::list_namespaces(&self.db).await.map_err(|e| format!("list namespaces: {e}"))?;

        let targets: Vec<&axagent_harness::types::MemoryNamespace> = match &req.namespace {
            Some(want) => namespaces.iter().filter(|ns| &ns.name == want).collect(),
            None => namespaces.iter().collect(),
        };

        let mut hits = Vec::new();
        for ns in targets {
            let items = mem::search_items(&self.db, &ns.id, &req.query, limit)
                .await
                .map_err(|e| format!("search items: {e}"))?;
            hits.extend(items.into_iter().map(|i| Self::to_search_item(&i, Some(&ns.name))));
        }
        hits.sort_by_key(|h| std::cmp::Reverse(h.importance));
        hits.truncate(limit);
        Ok(hits)
    }

    async fn get_memory_tree(&self) -> Result<Vec<MemoryTreeItem>, String> {
        let namespaces =
            mem::list_namespaces(&self.db).await.map_err(|e| format!("list namespaces: {e}"))?;

        let mut tree = Vec::new();
        for ns in &namespaces {
            let items =
                mem::list_items(&self.db, &ns.id).await.map_err(|e| format!("list items: {e}"))?;
            tree.push(MemoryTreeItem {
                id: ns.id.clone(),
                content: ns.name.clone(),
                children: items
                    .into_iter()
                    .map(|i| MemoryTreeItem { id: i.id, content: i.content, children: Vec::new() })
                    .collect(),
            });
        }
        Ok(tree)
    }

    async fn get_working_memory(&self) -> Result<Option<String>, String> {
        let items = mem::list_items_by_tier(&self.db, "working", Some(50))
            .await
            .map_err(|e| format!("list working tier: {e}"))?;
        if items.is_empty() {
            return Ok(None);
        }
        Ok(Some(items.into_iter().map(|i| i.content).collect::<Vec<_>>().join("\n\n")))
    }

    async fn get_grouped_memories(&self) -> Result<Vec<MemoryGroupedDto>, String> {
        let namespaces =
            mem::list_namespaces(&self.db).await.map_err(|e| format!("list namespaces: {e}"))?;

        // 按 updated_at 日期（YYYY-MM-DD）分组，BTreeMap 保证输出按日期有序。
        let mut grouped: BTreeMap<String, Vec<MemorySearchItem>> = BTreeMap::new();
        for ns in &namespaces {
            let items =
                mem::list_items(&self.db, &ns.id).await.map_err(|e| format!("list items: {e}"))?;
            for item in items {
                let date = item.updated_at.get(..10).unwrap_or(&item.updated_at).to_string();
                grouped.entry(date).or_default().push(Self::to_search_item(&item, Some(&ns.name)));
            }
        }
        Ok(grouped.into_iter().map(|(date, items)| MemoryGroupedDto { date, items }).collect())
    }

    async fn submit_feedback(
        &self,
        req: MemoryFeedbackRequest,
    ) -> Result<MemoryActionResultDto, String> {
        let result = if req.helpful {
            mem::promote_item(&self.db, &req.memory_id).await
        } else {
            mem::demote_item(&self.db, &req.memory_id).await
        };
        result
            .map(|_| MemoryActionResultDto { success: true, error: None })
            .map_err(|e| format!("feedback: {e}"))
    }

    async fn delete_memory(&self, id: &str) -> Result<MemoryActionResultDto, String> {
        mem::delete_item(&self.db, id)
            .await
            .map(|_| MemoryActionResultDto { success: true, error: None })
            .map_err(|e| format!("delete memory: {e}"))
    }

    async fn update_memory(
        &self,
        req: MemoryUpdateRequest,
    ) -> Result<MemoryActionResultDto, String> {
        mem::update_item(
            &self.db,
            &req.id,
            UpdateMemoryItemInput {
                title: None,
                content: req.content,
                tier: None,
                importance: req.importance.map(|v| (v.clamp(0, 100) as f64) / 100.0),
                memory_nature: None,
                tags: req.tags,
                applicability_tags: None,
            },
        )
        .await
        .map(|_| MemoryActionResultDto { success: true, error: None })
        .map_err(|e| format!("update memory: {e}"))
    }
}
