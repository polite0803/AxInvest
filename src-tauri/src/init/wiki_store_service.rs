// SPDX-License-Identifier: AGPL-3.0-only

//! `wiki.store` 接缝的宿主侧实现（P6.3 第一片）。
//!
//! 命令体从 `commands/wiki.rs::sync_note_to_knowledge_base` 逐字下沉到此处：笔记读取、
//! markdown 临时文件、知识库文档行与索引入队全留在宿主，插件只提交两个 id。
//! `AppHandle` 与 `Arc<AppState>` 是索引入队链的既有依赖，provider 在宿主启动期装配。

use std::path::PathBuf;

use async_trait::async_trait;
use axagent_dao::repo::index_jobs as jobs;
use axagent_harness::core_error::{AxAgentError, Result};
use axagent_harness::{
    SyncNoteToKnowledgeBaseOutcome, SyncNoteToKnowledgeBaseRequest, WikiStoreProvider,
};
use tauri::{AppHandle, Manager};

use crate::commands::error::{ErrorCategory, ErrorResponse};

/// Wiki 同步写接缝的宿主 provider。
///
/// 持 `AppHandle` 而不是 `Arc<AppState>`：`AppState` 由 Tauri 管理且**不是 `Clone`**，
/// 复制它等于造第二份运行态。`AppHandle::state` 取回的是同一实例的引用。
pub struct HostWikiStore {
    app: AppHandle,
}

impl HostWikiStore {
    pub fn new(app: AppHandle) -> Self {
        Self { app }
    }

    pub async fn sync(
        &self,
        request: SyncNoteToKnowledgeBaseRequest,
    ) -> Result<SyncNoteToKnowledgeBaseOutcome> {
        let SyncNoteToKnowledgeBaseRequest { note_id, knowledge_base_id } = request;
        if note_id.trim().is_empty() || knowledge_base_id.trim().is_empty() {
            return Err(AxAgentError::Validation(
                "wiki.store::sync_note_to_knowledge_base 需要非空 noteId / knowledgeBaseId".into(),
            ));
        }

        let state = self.app.state::<crate::AppState>();
        let note = axagent_dao::repo::note::get_note(state.harness.db(), &note_id)
            .await
            .map_err(to_provider_error)?;

        let file_name = format!("{}.md", note.title.replace('/', "_"));
        let data_dir = state.app_data_dir.join("wiki_sync").join(&note.vault_id);
        tokio::task::spawn_blocking({
            let dir = data_dir.clone();
            move || std::fs::create_dir_all(&dir)
        })
        .await
        .map_err(|e| AxAgentError::Provider(format!("wiki 同步目录创建任务失败: {e}")))?
        .map_err(to_provider_error)?;

        let full_path: PathBuf = data_dir.join(&file_name);
        let content = note.content.into_bytes();
        let write_path = full_path.clone();
        tokio::task::spawn_blocking(move || std::fs::write(&write_path, &content))
            .await
            .map_err(|e| AxAgentError::Provider(format!("wiki 同步文件写入任务失败: {e}")))?
            .map_err(to_provider_error)?;

        let source_path = full_path.to_string_lossy().to_string();
        let doc = axagent_dao::repo::knowledge::add_document(
            state.harness.db(),
            &knowledge_base_id,
            &note.title,
            &source_path,
            "text/markdown",
            Some("wiki-sync"),
        )
        .await
        .map_err(to_provider_error)?;

        let kb = axagent_dao::repo::knowledge::get_knowledge_base(
            state.harness.db(),
            &knowledge_base_id,
        )
        .await
        .map_err(to_provider_error)?;

        let mut index_queued = false;
        if kb.embedding_provider.is_some() {
            let _ = axagent_dao::repo::knowledge::update_document_status(
                state.harness.db(),
                &doc.id,
                "pending",
            )
            .await;
            // 索引任务入队失败不改变同步本身的成功状态 —— 与宿主原命令同口径：
            // 文档行已落库，队列服务会在启动扫描时捞回 pending 条目。
            index_queued = crate::index_queue::enqueue_job_sync(
                &state,
                &self.app,
                jobs::JOB_TYPE_INDEX_DOCUMENT,
                "kb",
                &knowledge_base_id,
                &doc.id,
                None,
                None,
            )
            .is_ok();
        }

        Ok(SyncNoteToKnowledgeBaseOutcome { document_id: doc.id, index_queued })
    }
}

#[async_trait]
impl WikiStoreProvider for HostWikiStore {
    async fn get_note_links(
        &self,
        note_id: String,
    ) -> Result<Vec<axagent_harness::types::NoteLink>> {
        let state = self.app.state::<crate::AppState>();
        axagent_dao::repo::note::get_note_links(state.harness.db(), &note_id)
            .await
            .map_err(to_provider_error)
    }

    async fn create_note(
        &self,
        input: axagent_harness::note_dtos::CreateNoteInput,
    ) -> Result<axagent_harness::note_dtos::Note> {
        let state = self.app.state::<crate::AppState>();
        let note = axagent_dao::repo::note::create_note(state.harness.db(), input)
            .await
            .map_err(to_provider_error)?;

        let _ = axagent_dao::repo::wiki_graph_cache::invalidate_cache(
            state.harness.db(),
            &note.vault_id,
        )
        .await;

        if let Some(lake) = axagent_harness::feedback_data_lake::global_feedback_lake() {
            let record = axagent_harness::WikiEditRecord {
                id: uuid::Uuid::new_v4().to_string(),
                conversation_id: None,
                wiki_id: note.vault_id.clone(),
                note_id: note.id.clone(),
                operation: "create".to_string(),
                before_snippet: None,
                after_snippet: Some(
                    axagent_harness::util_fns::truncate_to_char_boundary(&note.content, 500)
                        .to_string(),
                ),
                reason: None,
                quality_score: None,
                created_at: chrono::Utc::now().timestamp_millis(),
            };
            if let Err(e) = lake.insert_wiki_edit(record).await {
                tracing::warn!("Wiki 编辑反馈写入失败 note_id={}: {}", note.id, e);
            }
        }

        let _ = crate::index_queue::enqueue_job_sync(
            &state,
            &self.app,
            jobs::JOB_TYPE_INDEX_WIKI_NOTE,
            "wiki",
            &note.vault_id,
            &note.id,
            None,
            None,
        );
        Ok(note)
    }

    async fn update_note(
        &self,
        id: String,
        input: axagent_harness::note_dtos::UpdateNoteInput,
    ) -> Result<axagent_harness::note_dtos::Note> {
        if id.trim().is_empty() {
            return Err(AxAgentError::Validation("wiki.store::update_note 需要非空 id".into()));
        }
        let state = self.app.state::<crate::AppState>();
        let existing_content = if input.content.is_some() || input.title.is_some() {
            match axagent_dao::repo::note::get_note(state.harness.db(), &id).await {
                Ok(existing) => {
                    if let Err(e) = axagent_dao::repo::wiki::create_version(
                        state.harness.db(),
                        &existing.vault_id,
                        &existing.id,
                        &existing.title,
                        &existing.content,
                        &existing.author,
                    )
                    .await
                    {
                        tracing::error!("[wiki] 笔记 {} 版本备份失败，原始内容将被覆盖: {}", id, e);
                    }
                    Some(existing)
                },
                Err(e) => {
                    tracing::warn!("[wiki] 更新前获取笔记 {} 失败，跳过版本备份: {}", id, e);
                    None
                },
            }
        } else {
            None
        };

        let updated = axagent_dao::repo::note::update_note(state.harness.db(), &id, input)
            .await
            .map_err(to_provider_error)?;

        let _ = axagent_dao::repo::wiki::delete_old_versions(state.harness.db(), &id, 20).await;
        let _ = axagent_dao::repo::wiki_graph_cache::invalidate_cache(
            state.harness.db(),
            &updated.vault_id,
        )
        .await;

        if let Some(lake) = axagent_harness::feedback_data_lake::global_feedback_lake() {
            let before_snippet = existing_content.map(|e| {
                axagent_harness::util_fns::truncate_to_char_boundary(&e.content, 500).to_string()
            });
            let after_snippet = Some(
                axagent_harness::util_fns::truncate_to_char_boundary(&updated.content, 500)
                    .to_string(),
            );
            let record = axagent_harness::WikiEditRecord {
                id: uuid::Uuid::new_v4().to_string(),
                conversation_id: None,
                wiki_id: updated.vault_id.clone(),
                note_id: updated.id.clone(),
                operation: "update".to_string(),
                before_snippet,
                after_snippet,
                reason: None,
                quality_score: None,
                created_at: chrono::Utc::now().timestamp_millis(),
            };
            if let Err(e) = lake.insert_wiki_edit(record).await {
                tracing::warn!("Wiki 编辑反馈写入失败 note_id={}: {}", updated.id, e);
            }
        }

        let _ = crate::index_queue::enqueue_job_sync(
            &state,
            &self.app,
            jobs::JOB_TYPE_INDEX_WIKI_NOTE,
            "wiki",
            &updated.vault_id,
            &updated.id,
            None,
            None,
        );
        Ok(updated)
    }

    async fn list_notes(&self, vault_id: String) -> Result<Vec<axagent_harness::note_dtos::Note>> {
        if vault_id.trim().is_empty() {
            return Err(AxAgentError::Validation("wiki.store::list_notes 需要非空 vaultId".into()));
        }
        let state = self.app.state::<crate::AppState>();
        axagent_dao::repo::note::list_notes(state.harness.db(), &vault_id)
            .await
            .map_err(to_provider_error)
    }

    async fn delete_note(&self, id: String) -> Result<()> {
        if id.trim().is_empty() {
            return Err(AxAgentError::Validation("wiki.store::delete_note 需要非空 id".into()));
        }
        let state = self.app.state::<crate::AppState>();

        // 删除前先取出 vault_id，用于清理向量嵌入和失效图谱缓存
        // 区分 NotFound（直接返回 Ok）和 DB 错误（返回错误），避免向量残留
        let (vault_id, existing_content) =
            match axagent_dao::repo::note::get_note(state.harness.db(), &id).await {
                Ok(existing) => {
                    let collection_id = format!("wiki_{}", existing.vault_id);
                    // 向量删不掉就中止删除，避免向量残留
                    state
                        .vector_store
                        .delete_document_embeddings(&collection_id, &id)
                        .await
                        .map_err(|e| {
                            crate::commands::error::ErrorResponse::err_with_detail(
                                crate::commands::error_code::wiki::DELETE_NOTE_FAILED,
                                format!("清理笔记 {id} 的向量失败: {e}"),
                            )
                        })?;
                    (existing.vault_id, Some(existing.content))
                },
                Err(e) => {
                    if e.to_string().contains("NotFound") {
                        // 已不存在，直接返回 Ok
                        return Ok(());
                    }
                    return Err(AxAgentError::Provider(format!(
                        "wiki.store::delete_note 读取笔记失败: {e}"
                    )));
                },
            };

        // 删除笔记行
        axagent_dao::repo::note::delete_note(state.harness.db(), &id)
            .await
            .map_err(to_provider_error)?;

        // 失效图谱缓存
        let _ =
            axagent_dao::repo::wiki_graph_cache::invalidate_cache(state.harness.db(), &vault_id)
                .await;

        // 写入反馈数据湖
        if let Some(lake) = axagent_harness::feedback_data_lake::global_feedback_lake() {
            let record = axagent_harness::WikiEditRecord {
                id: uuid::Uuid::new_v4().to_string(),
                conversation_id: None,
                wiki_id: vault_id.clone(),
                note_id: id.clone(),
                operation: "delete".to_string(),
                before_snippet: existing_content.map(|c| {
                    axagent_harness::util_fns::truncate_to_char_boundary(&c, 500).to_string()
                }),
                after_snippet: None,
                reason: None,
                quality_score: None,
                created_at: chrono::Utc::now().timestamp_millis(),
            };
            if let Err(e) = lake.insert_wiki_edit(record).await {
                tracing::warn!("Wiki 删除反馈写入失败 note_id={}: {}", id, e);
            }
        }

        Ok(())
    }

    async fn get_note(&self, id: String) -> Result<axagent_harness::note_dtos::Note> {
        if id.trim().is_empty() {
            return Err(AxAgentError::Validation("wiki.store::get_note 需要非空 id".into()));
        }
        let state = self.app.state::<crate::AppState>();
        axagent_dao::repo::note::get_note(state.harness.db(), &id).await.map_err(to_provider_error)
    }

    async fn get_note_by_path(
        &self,
        vault_id: String,
        file_path: String,
    ) -> Result<axagent_harness::note_dtos::Note> {
        if vault_id.trim().is_empty() || file_path.trim().is_empty() {
            return Err(AxAgentError::Validation(
                "wiki.store::get_note_by_path 需要非空 vaultId / filePath".into(),
            ));
        }
        let state = self.app.state::<crate::AppState>();
        axagent_dao::repo::note::get_note_by_path(state.harness.db(), &vault_id, &file_path)
            .await
            .map_err(to_provider_error)
    }

    async fn sync_note_to_knowledge_base(
        &self,
        request: SyncNoteToKnowledgeBaseRequest,
    ) -> Result<SyncNoteToKnowledgeBaseOutcome> {
        self.sync(request).await
    }
}

fn to_provider_error(error: impl std::fmt::Display) -> AxAgentError {
    AxAgentError::Provider(
        ErrorResponse::from_error(error, ErrorCategory::Unrecoverable).to_string(),
    )
}
