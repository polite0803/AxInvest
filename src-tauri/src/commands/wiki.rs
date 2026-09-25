// SPDX-License-Identifier: AGPL-3.0-only

use crate::AppState;
use crate::commands::spawn_guard::catch_unwind_logged;
use axagent_agent_macro::agent_command;
use axagent_dao::repo::index_jobs as jobs;
use axagent_dao::repo::louvain;
use axagent_dao::repo::note::{CreateNoteInput, GraphData, Note, NoteLink, UpdateNoteInput};
use axagent_dao::repo::wiki::{
    self, CreateWikiTemplateInput, NoteVersion, UpdateWikiTemplateInput, WikiTemplate,
};
use axagent_harness::IpcEventName;
use axagent_harness::graph_dtos::{GraphEdge, LinkGraph};
use axagent_harness::louvain_dtos::LouvainResult;
use axagent_harness::types::NoteSearchResult;
use axagent_search::hybrid_search::HybridSearcher;
use axagent_search::rag::{RAGSource, WikiVaultRAG, collection_id};
use sea_orm::{ConnectionTrait, DatabaseConnection};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, State};

/// 实体融合短 TTL 缓存：30 秒内存缓存，避免每次请求都全量查询知识图谱实体。
/// 知识图谱实体变更后最多 30 秒最终一致，无需跨模块失效机制注入。
/// 缓存键为 wiki_id，值 = (存入时间戳, 融合后的完整 GraphData)。
fn get_entity_cache() -> &'static tokio::sync::Mutex<HashMap<String, (Instant, GraphData)>> {
    static CACHE: std::sync::OnceLock<tokio::sync::Mutex<HashMap<String, (Instant, GraphData)>>> =
        std::sync::OnceLock::new();
    CACHE.get_or_init(|| tokio::sync::Mutex::new(HashMap::new()))
}

const ENTITY_CACHE_TTL: Duration = Duration::from_secs(30);

/// 悬空边告警节流：kb → 上次告警时的 `(边总数, 悬空数)`。
///
/// 本函数由前端**按需**调用（打开/刷新图谱页），而「有边画不出来」是一个**持续状态**：
/// 不做节流就会把同一条事实刷进日志成百上千遍，真正的新告警反而被淹没。
///
/// 节流判据取「`(total_edges, dangling)` 组合是否与上次不同」，**不是**「距上次告警的
/// 时间」—— 时间节流会在「数据变了」的那一刻保持沉默，而数据变了恰恰是唯一要看的事。
/// 组合而非单看 `dangling`：从「38171/38171 悬空」变成「40000/38171 悬空」时
/// `dangling` 没变，但图的规模变了，那是另一个事实。
// SAFETY: 告警节流是纯同步临界区，static 缓存（OnceLock + Mutex）不跨 await、
// 不参与异步锁，std::sync::Mutex 在此是正确选择，符合 clippy.toml 的合法例外
//（对照 harness/src/test_support.rs 既有先例）。
#[allow(clippy::disallowed_types)]
fn should_warn_dangling(kb_id: &str, total_edges: usize, dangling: usize) -> bool {
    type Seen = HashMap<String, (usize, usize)>;
    static SEEN: std::sync::OnceLock<std::sync::Mutex<Seen>> = std::sync::OnceLock::new();
    let guard = SEEN.get_or_init(|| std::sync::Mutex::new(HashMap::new()));
    // 中毒恢复：本锁保护的是一张纯统计表，一次 panic 不应让后续所有告警永久失效。
    let mut seen = match guard.lock() {
        Ok(g) => g,
        Err(poisoned) => poisoned.into_inner(),
    };
    if seen.get(kb_id).copied() == Some((total_edges, dangling)) {
        return false;
    }
    seen.insert(kb_id.to_string(), (total_edges, dangling));
    true
}

/// 校验容器 ID（vault_id / note_id 等）格式，防止 SQL 注入和路径穿越。
/// 规则：1-128 字符，仅允许字母数字、连字符、下划线。
fn validate_container_id(id: &str, field_name: &str) -> Result<(), String> {
    if id.is_empty()
        || id.len() > 128
        || id.contains(|c: char| !c.is_alphanumeric() && c != '-' && c != '_')
    {
        return Err(String::from(crate::commands::error::ErrorResponse::from_error(
            format!(
                "Invalid {field_name}: must be 1-128 alphanumeric/hyphen/underscore characters"
            ),
            crate::commands::error::ErrorCategory::Unrecoverable,
        )));
    }
    Ok(())
}

/// 同步 IO 包装：把 std::fs 调用扔到 spawn_blocking 线程池，避免阻塞 tokio runtime。
/// 多个小文件操作适合 inline `spawn_blocking`。
async fn write_file_blocking(path: PathBuf, content: Vec<u8>) -> std::io::Result<()> {
    tokio::task::spawn_blocking(move || std::fs::write(&path, &content))
        .await
        .map_err(std::io::Error::other)?
}

async fn read_to_string_blocking(path: PathBuf) -> std::io::Result<String> {
    tokio::task::spawn_blocking(move || std::fs::read_to_string(&path))
        .await
        .map_err(std::io::Error::other)?
}

async fn create_dir_all_blocking(path: PathBuf) -> std::io::Result<()> {
    tokio::task::spawn_blocking(move || std::fs::create_dir_all(&path))
        .await
        .map_err(std::io::Error::other)?
}

fn enqueue_wiki_note_indexing(
    state: &State<'_, AppState>,
    app: &AppHandle,
    wiki_id: &str,
    note_id: &str,
) {
    let _ = crate::index_queue::enqueue_job_sync(
        state,
        app,
        jobs::JOB_TYPE_INDEX_WIKI_NOTE,
        "wiki",
        wiki_id,
        note_id,
        None,
        None,
    );
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BacklinkInfo {
    pub note_id: String,
    pub title: String,
    pub snippets: Vec<String>,
}

#[agent_command(domain = wiki, safety = Safe, call_mode = StateInput, description = "列出 Wiki 下的笔记")]
#[tauri::command]
pub async fn wiki_notes_list(
    state: State<'_, AppState>,
    vault_id: String,
) -> Result<Vec<Note>, String> {
    axagent_dao::repo::note::list_notes(state.harness.db(), &vault_id).await.map_err(|e| {
        String::from(crate::commands::error::ErrorResponse::from_error(
            e,
            crate::commands::error::ErrorCategory::Unrecoverable,
        ))
    })
}

/// 更新 Wiki 元数据，目前主要用于修改 embedding_provider。
/// 仅当字段非空时才更新对应列。
#[agent_command(domain = wiki, safety = Caution, call_mode = StateInput, description = "更新 Wiki 信息")]
#[tauri::command]
pub async fn update_wiki(
    state: State<'_, AppState>,
    id: String,
    name: Option<String>,
    description: Option<String>,
    embedding_provider: Option<String>,
    knowledge_base_id: Option<String>,
) -> Result<WikiUpdateResult, String> {
    let db = state.harness.db();

    // 取出原始 embedding_provider，便于前端判断是否需要重建索引
    let before = wiki::get_wiki(db, &id).await.map_err(|e| {
        String::from(crate::commands::error::ErrorResponse::from_error(
            e,
            crate::commands::error::ErrorCategory::Unrecoverable,
        ))
    })?;

    let updated = wiki::update_wiki(
        db,
        &id,
        name,
        description,
        embedding_provider.clone(),
        knowledge_base_id.map(Some),
    )
    .await
    .map_err(|e| {
        String::from(crate::commands::error::ErrorResponse::from_error(
            e,
            crate::commands::error::ErrorCategory::Unrecoverable,
        ))
    })?;

    Ok(WikiUpdateResult {
        id: updated.id.clone(),
        name: updated.name.clone(),
        description: updated.description.clone(),
        embedding_provider: updated.embedding_provider.clone(),
        knowledge_base_id: updated.knowledge_base_id.clone(),
        embedding_changed: before.embedding_provider != updated.embedding_provider,
    })
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WikiUpdateResult {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub embedding_provider: Option<String>,
    /// v118: 关联的知识库 ID
    pub knowledge_base_id: Option<String>,
    /// 旧 provider 与新 provider 是否不同；前端据此决定是否触发重建索引
    pub embedding_changed: bool,
}

/// 删除 Wiki 容器：清理向量集合 + 删除数据库记录。
/// 与 `delete_knowledge_base` / `delete_memory_namespace` 行为对齐。
#[agent_command(domain = wiki, safety = Dangerous, call_mode = StateInput, description = "删除 Wiki")]
#[tauri::command]
pub async fn delete_wiki(state: State<'_, AppState>, id: String) -> Result<(), String> {
    validate_container_id(&id, "wiki_id")?;

    // 与 llm_wiki_delete 保持一致的 collection_id 命名规则
    let collection_id = format!("wiki_{}", id);
    if let Err(e) = state.vector_store.delete_collection(&collection_id).await {
        tracing::warn!("Failed to delete vector collection {}: {}", collection_id, e);
    }

    wiki::delete_wiki(state.harness.db(), &id).await.map_err(|e| {
        String::from(crate::commands::error::ErrorResponse::from_error(
            e,
            crate::commands::error::ErrorCategory::Unrecoverable,
        ))
    })
}

#[agent_command(domain = wiki, safety = Safe, call_mode = StateInput, description = "获取 Wiki 笔记")]
#[tauri::command]
pub async fn wiki_notes_get(state: State<'_, AppState>, id: String) -> Result<Note, String> {
    axagent_dao::repo::note::get_note(state.harness.db(), &id).await.map_err(|e| {
        String::from(crate::commands::error::ErrorResponse::from_error(
            e,
            crate::commands::error::ErrorCategory::Unrecoverable,
        ))
    })
}

#[agent_command(domain = wiki, safety = Safe, call_mode = StateInput, description = "按路径获取 Wiki 笔记")]
#[tauri::command]
pub async fn wiki_notes_get_by_path(
    state: State<'_, AppState>,
    vault_id: String,
    file_path: String,
) -> Result<Note, String> {
    axagent_dao::repo::note::get_note_by_path(state.harness.db(), &vault_id, &file_path)
        .await
        .map_err(|e| {
            String::from(crate::commands::error::ErrorResponse::from_error(
                e,
                crate::commands::error::ErrorCategory::Unrecoverable,
            ))
        })
}

#[agent_command(domain = wiki, safety = Caution, call_mode = StateInput, description = "创建 Wiki 笔记")]
#[tauri::command]
pub async fn wiki_notes_create(
    app: AppHandle,
    state: State<'_, AppState>,
    input: CreateNoteInput,
) -> Result<Note, String> {
    let note =
        axagent_dao::repo::note::create_note(state.harness.db(), input).await.map_err(|e| {
            String::from(crate::commands::error::ErrorResponse::from_error(
                e,
                crate::commands::error::ErrorCategory::Unrecoverable,
            ))
        })?;

    // 失效图谱缓存（notes 表有写入）
    let _ =
        axagent_dao::repo::wiki_graph_cache::invalidate_cache(state.harness.db(), &note.vault_id)
            .await;

    // 写入反馈数据湖
    if let Some(lake) = axagent_harness::feedback_data_lake::global_feedback_lake() {
        let record = axagent_harness::WikiEditRecord {
            id: uuid::Uuid::new_v4().to_string(),
            conversation_id: None,
            wiki_id: note.vault_id.clone(),
            note_id: note.id.clone(),
            operation: "create".to_string(),
            before_snippet: None,
            // 按字节截取需对齐 UTF-8 字符边界，否则中文内容 panic（每字 3 字节）
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

    enqueue_wiki_note_indexing(&state, &app, &note.vault_id, &note.id);

    Ok(note)
}

#[agent_command(domain = wiki, safety = Caution, call_mode = StateInput, description = "更新 Wiki 笔记")]
#[tauri::command]
pub async fn wiki_notes_update(
    app: AppHandle,
    state: State<'_, AppState>,
    id: String,
    input: UpdateNoteInput,
) -> Result<Note, String> {
    let existing_content = if input.content.is_some() || input.title.is_some() {
        match axagent_dao::repo::note::get_note(state.harness.db(), &id).await {
            Ok(existing) => {
                // 版本备份失败时记录错误日志但不阻止更新（版本备份是辅助功能，
                // 不应因备份失败阻止用户修改内容；但需记录以便排查）
                if let Err(e) = wiki::create_version(
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
        .map_err(|e| {
            String::from(crate::commands::error::ErrorResponse::from_error(
                e,
                crate::commands::error::ErrorCategory::Unrecoverable,
            ))
        })?;

    let _ = wiki::delete_old_versions(state.harness.db(), &id, 20).await;

    // 失效图谱缓存（notes 表有更新）
    let _ = axagent_dao::repo::wiki_graph_cache::invalidate_cache(
        state.harness.db(),
        &updated.vault_id,
    )
    .await;

    // 写入反馈数据湖
    if let Some(lake) = axagent_harness::feedback_data_lake::global_feedback_lake() {
        let before_snippet = existing_content.map(|e| {
            if e.content.len() > 500 {
                e.content[..500].to_string()
            } else {
                e.content
            }
        });
        let after_snippet = if updated.content.len() > 500 {
            Some(updated.content[..500].to_string())
        } else {
            Some(updated.content.clone())
        };
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

    enqueue_wiki_note_indexing(&state, &app, &updated.vault_id, &updated.id);

    Ok(updated)
}

#[agent_command(domain = wiki, safety = Dangerous, call_mode = StateInput, description = "删除 Wiki 笔记")]
#[tauri::command]
pub async fn wiki_notes_delete(state: State<'_, AppState>, id: String) -> Result<(), String> {
    // 删除前先取出 vault_id，用于清理向量嵌入和失效图谱缓存
    // 区分 NotFound（直接返回 Ok）和 DB 错误（返回错误），避免向量残留
    let (vault_id, existing_content) =
        match axagent_dao::repo::note::get_note(state.harness.db(), &id).await {
            Ok(existing) => {
                let collection_id = format!("wiki_{}", existing.vault_id);
                // 向量删不掉就中止删除（2026-09-15 修）：此前 `let _ =` 吞错，于是笔记
                // 记录被删而旧向量残留 —— 已删除笔记的内容继续被检索命中。
                state.vector_store.delete_document_embeddings(&collection_id, &id).await.map_err(
                    |e| {
                        crate::commands::error::ErrorResponse::err_with_detail(
                            crate::commands::error_code::wiki::DELETE_NOTE_FAILED,
                            format!("清理笔记 {id} 的向量失败: {e}"),
                        )
                    },
                )?;
                (Some(existing.vault_id), Some(existing.content))
            },
            Err(e) if e.to_string().contains("NotFound") || e.to_string().contains("not found") => {
                // 笔记不存在，视为已删除，直接返回成功
                return Ok(());
            },
            Err(e) => {
                // DB 错误：返回错误，避免在不知道 vault_id 的情况下删除导致向量残留
                return Err(String::from(crate::commands::error::ErrorResponse::from_error(
                    e,
                    crate::commands::error::ErrorCategory::Unrecoverable,
                )));
            },
        };

    axagent_dao::repo::note::delete_note(state.harness.db(), &id).await.map_err(|e| {
        String::from(crate::commands::error::ErrorResponse::from_error(
            e,
            crate::commands::error::ErrorCategory::Unrecoverable,
        ))
    })?;

    // 失效图谱缓存（notes 表有删除）
    if let Some(ref vid) = vault_id {
        let _ =
            axagent_dao::repo::wiki_graph_cache::invalidate_cache(state.harness.db(), vid).await;
    }

    // 写入反馈数据湖
    if let (Some(vid), Some(content)) = (&vault_id, &existing_content) {
        if let Some(lake) = axagent_harness::feedback_data_lake::global_feedback_lake() {
            let before_snippet = if content.len() > 500 {
                content[..500].to_string()
            } else {
                content.clone()
            };
            let record = axagent_harness::WikiEditRecord {
                id: uuid::Uuid::new_v4().to_string(),
                conversation_id: None,
                wiki_id: vid.clone(),
                note_id: id.clone(),
                operation: "delete".to_string(),
                before_snippet: Some(before_snippet),
                after_snippet: None,
                reason: None,
                quality_score: None,
                created_at: chrono::Utc::now().timestamp_millis(),
            };
            if let Err(e) = lake.insert_wiki_edit(record).await {
                tracing::warn!("Wiki 编辑反馈写入失败 note_id={}: {}", id, e);
            }
        }
    }

    Ok(())
}

#[agent_command(domain = wiki, safety = Caution, call_mode = StateInput, description = "重建 Wiki 索引")]
#[tauri::command]
pub async fn rebuild_wiki_index(
    app: AppHandle,
    state: State<'_, AppState>,
    wiki_id: String,
) -> Result<(), String> {
    let wiki =
        axagent_dao::repo::wiki::get_wiki(state.harness.db(), &wiki_id).await.map_err(|e| {
            String::from(crate::commands::error::ErrorResponse::from_error(
                e,
                crate::commands::error::ErrorCategory::Unrecoverable,
            ))
        })?;

    let _embedding_provider = wiki.embedding_provider.as_ref().ok_or_else(|| {
        crate::commands::error::ErrorResponse::err(
            crate::commands::error_code::knowledge::NO_EMBEDDING_PROVIDER,
        )
    })?;

    let container = axagent_search::rag::KnowledgeContainer::from_wiki(&wiki);

    let collection_id = format!("wiki_{}", wiki_id);
    let _ = state.vector_store.delete_collection(&collection_id).await;

    let notes =
        axagent_dao::repo::note::list_notes(state.harness.db(), &wiki_id).await.map_err(|e| {
            String::from(crate::commands::error::ErrorResponse::from_error(
                e,
                crate::commands::error::ErrorCategory::Unrecoverable,
            ))
        })?;

    let db = state.harness.db().clone();
    let master_key = state.harness.master_key_owned();
    let vector_store = state.vector_store.clone();
    let wid = wiki_id.clone();

    tokio::spawn(catch_unwind_logged("wiki.rebuild_index", async move {
        for note in &notes {
            let result = crate::indexing::index_source(
                &db,
                &master_key,
                &vector_store,
                &container,
                &note.id,
                &note.content,
                None,
                None,
            )
            .await;

            if let Err(e) = &result {
                tracing::error!("Wiki re-indexing failed for note {}: {}", note.id, e);
            }

            let _ = app.emit(
                IpcEventName::WikiNoteIndexed.as_str(),
                serde_json::json!({
                    "noteId": note.id,
                    "success": result.is_ok(),
                    "error": result.as_ref().err().map(|e| e.to_string()),
                    "isRebuild": true,
                }),
            );
        }

        let _ = app
            .emit(IpcEventName::WikiRebuildComplete.as_str(), serde_json::json!({ "wikiId": wid }));
    }));

    Ok(())
}

#[agent_command(domain = wiki, safety = Safe, call_mode = StateInput, description = "获取笔记链接列表")]
#[tauri::command]
pub async fn wiki_notes_get_links(
    state: State<'_, AppState>,
    note_id: String,
) -> Result<Vec<NoteLink>, String> {
    axagent_dao::repo::note::get_note_links(state.harness.db(), &note_id).await.map_err(|e| {
        String::from(crate::commands::error::ErrorResponse::from_error(
            e,
            crate::commands::error::ErrorCategory::Unrecoverable,
        ))
    })
}

#[agent_command(domain = wiki, safety = Safe, call_mode = StateInput, description = "获取笔记反向链接")]
#[tauri::command]
pub async fn wiki_notes_get_backlinks(
    state: State<'_, AppState>,
    note_id: String,
) -> Result<Vec<BacklinkInfo>, String> {
    let links = axagent_dao::repo::note::get_note_backlinks(state.harness.db(), &note_id)
        .await
        .map_err(|e| {
            String::from(crate::commands::error::ErrorResponse::from_error(
                e,
                crate::commands::error::ErrorCategory::Unrecoverable,
            ))
        })?;

    let target_note = axagent_dao::repo::note::get_note(state.harness.db(), &note_id).await.ok();
    let target_title = target_note.as_ref().map(|n| n.title.as_str()).unwrap_or("");

    // 优化：批量查询所有 source_notes，避免 N+1（原实现每个 backlink 单独查一次）
    let source_ids: Vec<String> = links.iter().map(|l| l.source_note_id.clone()).collect();
    let source_notes_map: std::collections::HashMap<String, axagent_dao::repo::note::Note> =
        if source_ids.is_empty() {
            std::collections::HashMap::new()
        } else {
            use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
            let models = axagent_entities::notes::Entity::find()
                .filter(axagent_entities::notes::Column::Id.is_in(source_ids))
                .all(state.harness.db())
                .await
                .map_err(|e| {
                    String::from(crate::commands::error::ErrorResponse::from_error(
                        e,
                        crate::commands::error::ErrorCategory::Unrecoverable,
                    ))
                })?;
            models
                .into_iter()
                .map(|m| (m.id.clone(), axagent_dao::repo::note::model_to_note(m)))
                .collect()
        };

    let mut map: std::collections::HashMap<String, BacklinkInfo> = std::collections::HashMap::new();

    for link in &links {
        let source_note = match source_notes_map.get(&link.source_note_id) {
            Some(n) => n,
            None => continue,
        };

        let snippets = extract_link_context_snippets(&source_note.content, target_title, 80);

        let entry = map.entry(link.source_note_id.clone()).or_insert_with(|| BacklinkInfo {
            note_id: link.source_note_id.clone(),
            title: source_note.title.clone(),
            snippets: Vec::new(),
        });
        entry.snippets.extend(snippets);
    }

    Ok(map.into_values().collect())
}

fn extract_link_context_snippets(
    content: &str,
    target_title: &str,
    context_chars: usize,
) -> Vec<String> {
    if target_title.is_empty() {
        return Vec::new();
    }

    let link_pattern = format!("[[{}]]", target_title);
    let chars: Vec<char> = content.chars().collect();
    let total_len = chars.len();
    let pattern_chars: Vec<char> = link_pattern.chars().collect();
    let pattern_len = pattern_chars.len();

    let mut snippets = Vec::new();
    let mut i = 0;

    while i + pattern_len <= total_len {
        let window: Vec<char> = chars[i..i + pattern_len].to_vec();
        if window == pattern_chars {
            let start = i.saturating_sub(context_chars);
            let end = (i + pattern_len + context_chars).min(total_len);

            let mut snippet = String::new();
            if start > 0 {
                snippet.push_str("...");
            }
            snippet.push_str(&chars[start..end].iter().collect::<String>());
            if end < total_len {
                snippet.push_str("...");
            }

            snippets.push(snippet);
            i += pattern_len;
        } else {
            i += 1;
        }
    }

    snippets
}

#[agent_command(domain = wiki, safety = Caution, call_mode = StateInput, description = "同步笔记链接关系")]
#[tauri::command]
pub async fn wiki_notes_sync_links(
    state: State<'_, AppState>,
    vault_id: String,
    source_note_id: String,
    links: Vec<(String, String, String)>,
) -> Result<(), String> {
    axagent_dao::repo::note::sync_note_links(state.harness.db(), &vault_id, &source_note_id, links)
        .await
        .map_err(|e| {
            String::from(crate::commands::error::ErrorResponse::from_error(
                e,
                crate::commands::error::ErrorCategory::Unrecoverable,
            ))
        })
}

#[agent_command(domain = wiki, safety = Safe, call_mode = StateInput, description = "搜索 Wiki 笔记")]
#[tauri::command]
pub async fn wiki_notes_search(
    state: State<'_, AppState>,
    vault_id: String,
    query: String,
    top_k: Option<usize>,
) -> Result<Vec<NoteSearchResult>, String> {
    validate_container_id(&vault_id, "vault_id")?;
    let top_k = top_k.unwrap_or(10);

    let wiki =
        axagent_dao::repo::wiki::get_wiki(state.harness.db(), &vault_id).await.map_err(|e| {
            String::from(crate::commands::error::ErrorResponse::from_error(
                e,
                crate::commands::error::ErrorCategory::Unrecoverable,
            ))
        })?;

    if wiki.embedding_provider.is_some() {
        match wiki_notes_search_hybrid(&state, &vault_id, &query, top_k).await {
            Ok(results) => {
                // ⚠ 2026-09-15 二次修（过滤位置）：阈值过滤已**上移到检索层**
                // （`wiki_notes_search_hybrid` 里写进 `options.min_score`，在
                // `truncate(top_k)` 之前生效），此处不再重复筛 —— 再筛一次筛的是
                // 已截断的结果（「先截断后过滤」），且同一条规则两处存在必然漂移。
                //
                // 保留这段说明是因为它是**方向事故**的现场：这里对比的
                // `NoteSearchResult.score` 语义是**相关度、越大越相关**
                // （`harness::rag_config::NoteSearchResult` 的文档明确写了「与
                // `VectorSearchResult.score`（L2 距离，越小越好）语义相反」），
                // 而原代码写的是 `r.score <= effective_threshold` —— 拿相关度去和
                // 距离上限比。取默认值 20.0 时恒真（空过滤，无害）；取前端默认值
                // 0.1 时 `score <= 0.1` 把**最不相关的那批**留下、把最相关的全丢掉
                // ⇒ 不是「搜不到」，而是「搜到最差的」。
                return Ok(results);
            },
            Err(e) => {
                tracing::warn!(
                    "Hybrid search failed for wiki {}, falling back to keyword: {}",
                    vault_id,
                    e
                );
            },
        }
    }

    wiki_notes_search_keyword(&state, &vault_id, &query, top_k).await
}

async fn wiki_notes_search_hybrid(
    state: &AppState,
    vault_id: &str,
    query: &str,
    top_k: usize,
) -> Result<Vec<NoteSearchResult>, String> {
    let wiki =
        axagent_dao::repo::wiki::get_wiki(state.harness.db(), vault_id).await.map_err(|e| {
            String::from(crate::commands::error::ErrorResponse::from_error(
                e,
                crate::commands::error::ErrorCategory::Unrecoverable,
            ))
        })?;

    let ep = wiki.embedding_provider.as_ref().ok_or_else(|| {
        crate::commands::error::ErrorResponse::err(
            crate::commands::error_code::knowledge::NO_EMBEDDING_PROVIDER,
        )
    })?;
    let dimensions = wiki.embedding_dimensions.map(|d| d as usize);

    let embed_fn = crate::indexing::ProviderEmbedFn;
    let embed_response = axagent_search::rag::AsyncEmbedFn::generate(
        &embed_fn,
        state.harness.db(),
        state.harness.master_key(),
        ep,
        vec![query.to_string()],
        dimensions,
    )
    .await
    .map_err(|e| {
        String::from(crate::commands::error::ErrorResponse::from_error(
            e,
            crate::commands::error::ErrorCategory::Unrecoverable,
        ))
    })?;

    let query_embedding = embed_response.embeddings.into_iter().next().ok_or_else(|| {
        crate::commands::error::ErrorResponse::err_with_detail(
            crate::commands::error_code::vector::EMBEDDING_FAILED,
            "No query embedding returned",
        )
    })?;

    let collection_id = collection_id(WikiVaultRAG.collection_prefix(), vault_id);
    let searcher = HybridSearcher::new(state.harness.db().clone());

    // 混合检索权重来自用户设置（`ragPipelineConfig.hybrid`）—— 此前这里写死
    // 0.7/0.3/RRF/60，配置改了不生效且无告警（2026-09-15 接线）。
    let hybrid = crate::indexing::load_hybrid_config(state.harness.db()).await;
    let mut options = axagent_search::hybrid_search::hybrid_options_from_config(&hybrid, top_k);
    // ⚠ 2026-09-15 二次修（过滤位置）：阈值原先由调用方在拿到**已截断**的 top_k
    // 之后再按 `score >= min_similarity` 筛。此处两侧**同为** `combined_score` 标尺
    // ⇒ 「先截断后过滤」与「先过滤后截断」结果**等价**（合格集必为前缀）
    // ⇒ 本项收益是**结构**（过滤只有一处真源），不是「修了丢结果」（见 #205）。
    // 写进 `min_score` 后由 `hybrid_search_with_filter` 在 `truncate(top_k)` **之前**
    // 统一应用。标尺说明：`options.min_score` 与 `hybrid_result.combined_score`
    // 同为 [0,1] 相关度（越大越相关）。
    options.min_score = Some(axagent_search::rag::similarity_floor_from_threshold(
        wiki.retrieval_threshold.unwrap_or(0.0),
    ));

    let hybrid_results = searcher
        .hybrid_search(&collection_id, query, query_embedding, options)
        .await
        .map_err(|e| {
            String::from(crate::commands::error::ErrorResponse::from_error(
                e,
                crate::commands::error::ErrorCategory::Unrecoverable,
            ))
        })?;

    let mut results = Vec::new();
    for hybrid_result in &hybrid_results {
        let note =
            match axagent_dao::repo::note::get_note(state.harness.db(), &hybrid_result.document_id)
                .await
            {
                Ok(n) => n,
                Err(_) => continue,
            };

        let snippet = extract_highlight_snippet(&note.content, query, 50, 150);
        let score = hybrid_result.combined_score as f64;

        results.push(NoteSearchResult { note, snippet, score });
    }

    Ok(results)
}

async fn wiki_notes_search_keyword(
    state: &AppState,
    vault_id: &str,
    query: &str,
    top_k: usize,
) -> Result<Vec<NoteSearchResult>, String> {
    // 走数据库全文索引（2026-09-16 前由 v104_notes_fts 迁移建立；该迁移已随 74 个迁移
    // 文件删除，现在由声明式引擎按 `reconcile/extras.rs` 的 L2 声明建出 `notes_fts`
    // 虚拟表与三条同步触发器）。避免把 10 万节点灌进内存做 BM25。
    // - SQLite: notes_fts 虚拟表 + MATCH 操作符，bm25() 函数返回相关性得分
    // - PostgreSQL: tsv @@ plainto_tsquery + ts_rank
    //
    // 任意一种后端都通过参数化查询绑定 vault_id / query / top_k，避免 SQL 注入。
    let db = state.harness.db();
    let backend = db.get_database_backend();

    // 空 query 直接返回，避免 MATCH ' ' 报错
    let trimmed = query.trim();
    if trimmed.is_empty() {
        return Ok(Vec::new());
    }

    // 转义 SQLite FTS5 的特殊字符（双引号），用 "..." 包裹做短语查询
    // 多个 token 用 OR 连接以提升召回
    let sqlite_match_query = build_fts5_match_query(trimmed);

    let rows = if backend == sea_orm::DbBackend::Postgres {
        db.query_all_raw(sea_orm::Statement::from_sql_and_values(
            sea_orm::DbBackend::Postgres,
            "SELECT n.id, n.vault_id, n.title, n.file_path, n.content, n.content_hash, \
                    n.author, n.page_type, n.tags, n.source_refs, n.related_pages, n.quality_score, \
                    n.last_linted_at, n.last_compiled_at, n.compiled_source_hash, \
                    n.user_edited, n.user_edited_at, n.created_at, n.updated_at, n.is_deleted, \
                    ts_rank(n.tsv, plainto_tsquery('simple', $1)) AS rank \
             FROM notes n \
             WHERE n.vault_id = $2 AND n.is_deleted = 0 \
               AND n.tsv @@ plainto_tsquery('simple', $1) \
             ORDER BY rank DESC \
             LIMIT $3",
            [trimmed.into(), vault_id.into(), (top_k as i64).into()],
        ))
        .await
    } else {
        db.query_all_raw(sea_orm::Statement::from_sql_and_values(
            sea_orm::DbBackend::Sqlite,
            "SELECT n.id, n.vault_id, n.title, n.file_path, n.content, n.content_hash, \
                    n.author, n.page_type, n.tags, n.source_refs, n.related_pages, n.quality_score, \
                    n.last_linted_at, n.last_compiled_at, n.compiled_source_hash, \
                    n.user_edited, n.user_edited_at, n.created_at, n.updated_at, n.is_deleted, \
                    bm25(notes_fts) AS rank \
             FROM notes_fts f \
             JOIN notes n ON n.rowid = f.rowid \
             WHERE n.vault_id = ? AND n.is_deleted = 0 \
               AND notes_fts MATCH ? \
             ORDER BY rank ASC \
             LIMIT ?",
            [vault_id.into(), sqlite_match_query.into(), (top_k as i64).into()],
        ))
        .await
    };

    let rows = rows.map_err(|e| {
        String::from(crate::commands::error::ErrorResponse::from_error(
            e,
            crate::commands::error::ErrorCategory::Unrecoverable,
        ))
    })?;

    let mut results: Vec<NoteSearchResult> = Vec::with_capacity(rows.len());
    for row in rows {
        // bm25() 在 SQLite 中返回负数（越小越相关），统一转成正数排序
        // ts_rank 在 PG 中返回正数（越大越相关）
        let raw_rank: f64 = row.try_get_by("rank").unwrap_or(0.0);
        let rank = if backend == sea_orm::DbBackend::Postgres {
            raw_rank
        } else {
            // SQLite bm25 返回负值，取绝对值并取反得到正的相关性分数
            -raw_rank
        };

        // 解析 tags JSON 数组
        let tags: Vec<String> = row
            .try_get_by::<sea_orm::JsonValue, _>("tags")
            .ok()
            .and_then(|j| serde_json::from_value(j).ok())
            .unwrap_or_default();

        let note = Note {
            id: row.try_get_by("id").unwrap_or_default(),
            vault_id: row.try_get_by("vault_id").unwrap_or_default(),
            title: row.try_get_by("title").unwrap_or_default(),
            file_path: row.try_get_by("file_path").unwrap_or_default(),
            content: row.try_get_by("content").unwrap_or_default(),
            content_hash: row.try_get_by("content_hash").unwrap_or_default(),
            author: row.try_get_by("author").unwrap_or_default(),
            page_type: row.try_get_by("page_type").ok(),
            tags,
            source_refs: row
                .try_get_by::<sea_orm::JsonValue, _>("source_refs")
                .ok()
                .and_then(|j| serde_json::from_value(j).ok()),
            related_pages: row
                .try_get_by::<sea_orm::JsonValue, _>("related_pages")
                .ok()
                .and_then(|j| serde_json::from_value(j).ok()),
            quality_score: row.try_get_by("quality_score").ok(),
            last_linted_at: row.try_get_by("last_linted_at").ok(),
            last_compiled_at: row.try_get_by("last_compiled_at").ok(),
            compiled_source_hash: row.try_get_by("compiled_source_hash").ok(),
            user_edited: row.try_get_by::<i32, _>("user_edited").unwrap_or(0) != 0,
            user_edited_at: row.try_get_by("user_edited_at").ok(),
            created_at: row.try_get_by("created_at").unwrap_or(0),
            updated_at: row.try_get_by("updated_at").unwrap_or(0),
            is_deleted: row.try_get_by::<i32, _>("is_deleted").unwrap_or(0) != 0,
        };

        let snippet = extract_highlight_snippet(&note.content, query, 50, 150);

        // 综合 score = rank + quality_score * 0.3（保留原有质量分加权）
        let score = rank + note.quality_score.unwrap_or(0.0) * 0.3;

        results.push(NoteSearchResult { note, snippet, score });
    }

    // PG 已在 SQL 内排序，SQLite 的 bm25 排序也已生效，但综合 quality_score 后需要重新排
    results.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
    results.truncate(top_k);

    Ok(results)
}

/// 把用户输入转为 SQLite FTS5 MATCH 表达式。
///
/// FTS5 中双引号是特殊字符，需要转义；多个 token 之间默认 OR 不行（FTS5 默认 AND），
/// 这里显式用 OR 连接提升召回率。空 query 返回空串（调用方应已 trim 检查）。
fn build_fts5_match_query(query: &str) -> String {
    let tokens: Vec<String> = query
        .split_whitespace()
        .filter(|s| !s.is_empty())
        .map(|tok| {
            // 转义双引号：FTS5 字符串字面量用 "..." 包裹
            let escaped = tok.replace('"', "\"\"");
            format!("\"{}\"", escaped)
        })
        .collect();

    if tokens.is_empty() {
        return String::new();
    }

    tokens.join(" OR ")
}

fn extract_highlight_snippet(
    content: &str,
    query: &str,
    context_chars: usize,
    max_snippet_len: usize,
) -> String {
    let content_lower = content.to_lowercase();
    let query_lower = query.to_lowercase();
    let query_words: Vec<&str> = query_lower.split_whitespace().collect();

    let best_pos = if !query_lower.is_empty() {
        content_lower.find(&query_lower)
    } else {
        None
    };

    let best_pos =
        best_pos.or_else(|| query_words.iter().filter_map(|w| content_lower.find(w)).min());

    let start = match best_pos {
        Some(pos) => pos.saturating_sub(context_chars),
        None => 0,
    };

    let chars: Vec<char> = content.chars().collect();
    let total_len = chars.len();

    let start_char = start.min(total_len);
    let end_char = (start_char + max_snippet_len).min(total_len);

    let mut snippet: String = chars[start_char..end_char].iter().collect();

    if end_char < total_len {
        snippet.push_str("...");
    }
    if start_char > 0 {
        snippet = format!("...{}", snippet);
    }

    snippet
}

#[agent_command(domain = wiki, safety = Safe, call_mode = StateInput, description = "获取 Wiki 图谱数据")]
#[tauri::command]
pub async fn get_wiki_graph(
    state: State<'_, AppState>,
    wiki_id: String,
) -> Result<GraphData, String> {
    axagent_dao::repo::note::get_vault_graph(state.harness.db(), &wiki_id).await.map_err(|e| {
        String::from(crate::commands::error::ErrorResponse::from_error(
            e,
            crate::commands::error::ErrorCategory::Unrecoverable,
        ))
    })
}

#[agent_command(domain = wiki, safety = Safe, call_mode = StateInput, description = "Wiki 社区发现")]
#[tauri::command]
pub async fn wiki_graph_communities(
    state: State<'_, AppState>,
    wiki_id: String,
) -> Result<LouvainResult, String> {
    let graph_data = axagent_dao::repo::note::get_vault_graph(state.harness.db(), &wiki_id)
        .await
        .map_err(|e| {
        String::from(crate::commands::error::ErrorResponse::from_error(
            e,
            crate::commands::error::ErrorCategory::Unrecoverable,
        ))
    })?;

    // Louvain 为 CPU 密集型同步计算（大图数秒），必须放到阻塞线程池，
    // 避免占死 tokio worker 导致同线程其他命令/事件全部延迟
    let result = tokio::task::spawn_blocking(move || {
        let link_graph = LinkGraph::from_graph_data(graph_data);
        louvain::detect_communities(link_graph)
    })
    .await
    .map_err(|e| {
        String::from(crate::commands::error::ErrorResponse::from_error(
            e,
            crate::commands::error::ErrorCategory::Unrecoverable,
        ))
    })?;
    Ok(result)
}

/// 实体侧子图（实体节点 + 实体关系边），带 30 秒 TTL 内存缓存。
///
/// **提取自** `get_wiki_graph_cached` 的第 3 步（2026-09-18）：现在有两个消费方 ——
/// 融合图谱（`get_wiki_graph_cached`）与**实体侧社区检测**
/// （`wiki_graph_communities_cached`）。两者必须拿到**同一份**实体集，
/// 否则「图上有、社区映射里没有」的节点会重新出现，而那正是
/// `assignMissingCommunities` 当初要兜的形态（见前端 `communityMerge.ts`）。
///
/// 返回的是**原始形态**（实体 id 与其边两端都加 `entity:` 前缀），
/// 但**不含**笔记节点与 `mapping` 边 —— 融合与映射由调用方各自完成：
///   · 融合图谱要的是「全部」；
///   · 实体侧社区要的**只是实体子图**（把 24k 笔记混进来会让实体分区被稀释，
///     而单独跑一遍的意义正是让实体按自己的拓扑分区）。
///
/// 缓存键是 `kb_id`（不是 `wiki_id`）：本函数的结果只依赖知识库。
async fn load_fused_entities(db: &DatabaseConnection, kb_id: &str) -> GraphData {
    let now = Instant::now();
    {
        let guard = get_entity_cache().lock().await;
        if let Some((at, graph)) = guard.get(kb_id)
            && now.duration_since(*at) < ENTITY_CACHE_TTL
        {
            return graph.clone();
        }
    }

    // 未命中：实时查询实体并写缓存（仅缓存实体部分，不含笔记与 mapping）
    let mut entity_nodes = Vec::new();

    // 获取实体节点（ID 加 "entity:" 前缀避免与笔记 ID 冲突）
    let raw_nodes =
        axagent_dao::repo::knowledge_graph::get_knowledge_graph_nodes_for_wiki(db, kb_id)
            .await
            .unwrap_or_default();
    for mut node in raw_nodes {
        node.id = format!("entity:{}", node.id);
        entity_nodes.push(node);
    }

    // 获取实体关系边
    let mut reference_edges = Vec::new();
    let raw_edges =
        axagent_dao::repo::knowledge_graph::get_knowledge_graph_edges_for_wiki(db, kb_id)
            .await
            .unwrap_or_default();
    for mut edge in raw_edges {
        edge.source = format!("entity:{}", edge.source);
        edge.target = format!("entity:{}", edge.target);
        reference_edges.push(edge);
    }

    let fused = GraphData::new(entity_nodes, reference_edges);
    get_entity_cache().lock().await.insert(kb_id.to_string(), (Instant::now(), fused.clone()));
    fused
}

/// 带缓存的图谱查询：优先读 `wiki_graph_cache` 表，未命中则实时计算并写缓存。
///
/// 10 万节点规模下，实时计算单次数秒；命中缓存 < 10ms。
/// 缓存在 notes 写入/更新/删除时自动失效。
///
/// P1-2: 融合知识图谱实体关系到 Wiki 图谱：
/// - 在 Wiki 笔记图谱基础上追加知识图谱实体节点（type="entity"）和关系边（type="reference"）
/// - 实体节点 ID 加 "entity:" 前缀确保与笔记 ID 不冲突
/// - 空知识库不影响原有图谱显示
#[agent_command(domain = wiki, safety = Safe, call_mode = StateInput, description = "获取 Wiki 图谱（缓存）")]
#[tauri::command]
pub async fn get_wiki_graph_cached(
    state: State<'_, AppState>,
    wiki_id: String,
) -> Result<GraphData, String> {
    let db = state.harness.db();

    // 1. 尝试命中缓存（仅缓存 Wiki 笔记部分）
    let mut base_graph = if let Some(entry) =
        axagent_dao::repo::wiki_graph_cache::get_cached_graph(db, &wiki_id).await.map_err(|e| {
            String::from(crate::commands::error::ErrorResponse::from_error(
                e,
                crate::commands::error::ErrorCategory::Unrecoverable,
            ))
        })? {
        entry.graph_data
    } else {
        // 未命中：实时计算 Wiki 笔记图谱并写缓存
        let graph_data =
            axagent_dao::repo::note::get_vault_graph(db, &wiki_id).await.map_err(|e| {
                String::from(crate::commands::error::ErrorResponse::from_error(
                    e,
                    crate::commands::error::ErrorCategory::Unrecoverable,
                ))
            })?;

        axagent_dao::repo::wiki_graph_cache::save_cached_graph(db, &wiki_id, &graph_data, None)
            .await
            .map_err(|e| {
                String::from(crate::commands::error::ErrorResponse::from_error(
                    e,
                    crate::commands::error::ErrorCategory::Unrecoverable,
                ))
            })?;

        graph_data
    };

    // 2. 查询 Wiki 关联的知识库 ID（v118：不再硬编码假设 wiki_id == kb_id）
    let kb_id = axagent_dao::repo::wiki::get_wiki(db, &wiki_id)
        .await
        .ok()
        .and_then(|w| w.knowledge_base_id)
        .unwrap_or_else(|| wiki_id.clone());

    // 3. 融合知识图谱实体关系
    // 知识图谱实体写路径分散（命令层 + 后台索引任务），无法可靠逐点失效；
    // 采用 30 秒短 TTL 内存缓存：既避免每次打开图谱都全量查询实体，
    // 又保证实体变更后最多 30 秒最终一致。mapping 边在下方基于最新笔记实时重建。
    // （缓存与查询已提取到 `load_fused_entities` —— 实体侧社区检测要用**同一份**实体集。）
    let fused_entities = load_fused_entities(db, &kb_id).await;
    let (entity_nodes, reference_edges) = (fused_entities.nodes, fused_entities.edges);

    // 4. 建立实体节点与 Wiki 笔记节点的映射边（v118：消除孤岛）
    let mut mapping_edges = Vec::new();
    let note_title_map: std::collections::HashMap<String, String> = base_graph
        .nodes
        .iter()
        .filter(|n| !n.id.starts_with("entity:"))
        .map(|n| (n.title.to_lowercase(), n.id.clone()))
        .collect();

    for entity_node in &entity_nodes {
        let entity_name_lower = entity_node.title.to_lowercase();
        if let Some(note_id) = note_title_map.get(&entity_name_lower) {
            // 合成边：不对应任何 DB 行 ⇒ 刻意不带 `relationType`
            mapping_edges.push(GraphEdge::structural(
                entity_node.id.clone(),
                note_id.clone(),
                "mapping",
            ));
        }
    }

    // 5. 合并到统一的 GraphData
    base_graph.nodes.extend(entity_nodes);
    base_graph.edges.extend(reference_edges);
    base_graph.edges.extend(mapping_edges);

    // 6. A2-升级（2026-09-14）：把「类型解析失败」的规模送到界面。
    //
    // 必须在**这一步**（融合之后、返回之前）重算，理由有两条：
    // ① 统计要对**最终返回给前端的节点集**成立 —— `get_vault_graph` 那边算过一次，
    //    但那时还没有实体节点；融合后节点集变了，统计必须跟着变。
    // ② 缓存命中的分支读的是 **DB 里存的 JSON**，那份数据可能是升级前写的
    //    （`#[serde(default)]` ⇒ 字段缺失即空清单）⇒ 不在这里重算，老缓存就永远看不到提示。
    // 判据与聚合都在 `harness::graph_dtos` 里（只有一份实现），此处只调用。
    // P2-b（2026-09-14）：这个方法现在**同时**刷节点侧与边侧 ——
    // 融合进来的实体关系边正是边侧统计唯一有内容的来源（笔记链接恒为 `link`）。
    base_graph.refresh_unresolved();

    // 7. 画不出来的边从 `edges` 里摘掉，但**保留淘汰规模**（B，2026-09-18）
    //
    // 上一步只**报**不**改**，于是同一份 JSON 里两半互相矛盾：统计说「172,926 条里有
    // 276 条端点缺失」，而 `edges` 长度仍是 172,926 —— 渲染层 `idSet.has` 判否即
    // `continue` 跳过那 276 条，工具栏却照旧显示 `edges.length`（`GraphView` 的
    // `edgeCount`）。**用户读到的边数里混着根本没画出来的边**，而且这个偏差会随
    // 任何一次上游脏数据（跨库合并、写端域不一致）重新出现 —— 只修数据不修这里，
    // 下次照样虚高。
    //
    // 摘掉它们**不等于**让它静默：淘汰规模继续留在 `dangling_edges` 里，
    // 界面照旧告警（判据未变），下一步的日志也照旧打。两条一起才成立 ——
    // 只删边 = 把异常变静默；只报不改 = 边数继续撒谎。
    let dropped_edges = base_graph.retain_resolved_edges();
    if dropped_edges > 0 {
        tracing::info!(
            kb = %kb_id,
            dropped = dropped_edges,
            remaining_edges = base_graph.edges.len(),
            "已从 Wiki 图谱边集里剔除端点缺失的边：它们画不出来，此前却仍计进工具栏的边数；淘汰规模保留在 danglingEdges 里继续告警"
        );
    }

    // 8. 悬空边必须**在返回前说一次话**（D-2，2026-09-17）
    //
    // 在此之前，端点找不到的边在融合层与渲染层是**两处静默**：
    // 后端照常把它放进 `edges`，前端 `if (!idSet.has(...)) continue;` 直接跳过 ——
    // 后果是界面「一个节点一条线都没有」而工具栏照旧显示 `74711E`，
    // 用户与开发者都拿不到「边的绝大多数没画出来」这条信息。
    //
    // 本步负责**告警**那一半（上一步负责把它从 `edges` 里摘掉）；丢悬空边本身是对的
    // （画不出就是画不出），错的是没人知道丢了多少。彻底修复脏数据的路径是修数据
    // （见 `audit_kb_domain_consistency`）或修合并逻辑（kb 域），不是让前端硬画 ——
    // 但在数据修好之前，这两步保证界面读到的边数已经是「真的画得出来的条数」。
    //
    // 字段同时随 `GraphData` 序列化给前端（`danglingEdges`）⇒ 界面可以自己决定
    // 怎么提示；日志只负责让「没打开界面」的时刻也留下痕迹。
    let dangling = &base_graph.dangling_edges;
    if dangling.dangling > 0
        && should_warn_dangling(&kb_id, dangling.total_edges, dangling.dangling)
    {
        tracing::warn!(
            kb = %kb_id,
            total_edges = dangling.total_edges,
            dangling = dangling.dangling,
            missing_source_only = dangling.missing_source_only,
            missing_target_only = dangling.missing_target_only,
            missing_both = dangling.missing_both,
            samples = %dangling.sample_edge_ids.join(" | "),
            "Wiki 图谱存在端点缺失的边：这些边**不会被画出**，但它们仍在 edges 计数里 —— 界面上表现为「节点之间没有关联」；若 missing_both 占多数，通常是知识库实体的域与边的域不一致（见 audit_kb_domain_consistency）"
        );
    }

    Ok(base_graph)
}

/// 带缓存的社区检测：优先读缓存，未命中则跑 Louvain 并写缓存。
///
/// ⚠ 本函数只算**笔记侧**，并且**刻意不挂** `#[tauri::command]`：
/// 命令入口是下面的 `wiki_graph_communities_cached` —— 它在本函数的三处 `return`
/// **之后**统一附加实体侧社区。把命令属性留在这一层，就等于要求三个早返回分支
/// 各自记得执行附加步骤，漏一个就是「同一条数据两条路径给出不同结果」。
///
/// 参数取 `&State` 而不是 `State`：调用方（命令入口）需要在调用前后各用一次
/// `state.harness.db()`，move 会让那个连接的借用与 move 冲突。
async fn note_communities_cached(
    state: &State<'_, AppState>,
    wiki_id: String,
) -> Result<LouvainResult, String> {
    let db = state.harness.db();

    // 1. 尝试命中缓存（含 communities）
    if let Some(entry) =
        axagent_dao::repo::wiki_graph_cache::get_cached_graph(db, &wiki_id).await.map_err(|e| {
            String::from(crate::commands::error::ErrorResponse::from_error(
                e,
                crate::commands::error::ErrorCategory::Unrecoverable,
            ))
        })?
    {
        if let Some(communities) = entry.communities {
            return Ok(communities);
        }
        // graph_data 已缓存但 communities 未算：用缓存 graph_data 跑 Louvain
        // CPU 密集计算放阻塞线程池，避免阻塞 tokio worker
        let result = tokio::task::spawn_blocking(move || {
            let link_graph = LinkGraph::from_graph_data(entry.graph_data);
            louvain::detect_communities(link_graph)
        })
        .await
        .map_err(|e| {
            String::from(crate::commands::error::ErrorResponse::from_error(
                e,
                crate::commands::error::ErrorCategory::Unrecoverable,
            ))
        })?;
        axagent_dao::repo::wiki_graph_cache::save_cached_communities(db, &wiki_id, &result)
            .await
            .map_err(|e| {
            String::from(crate::commands::error::ErrorResponse::from_error(
                e,
                crate::commands::error::ErrorCategory::Unrecoverable,
            ))
        })?;
        return Ok(result);
    }

    // 2. 未命中：实时计算 graph + communities 并写缓存
    let graph_data = axagent_dao::repo::note::get_vault_graph(db, &wiki_id).await.map_err(|e| {
        String::from(crate::commands::error::ErrorResponse::from_error(
            e,
            crate::commands::error::ErrorCategory::Unrecoverable,
        ))
    })?;

    // B1 修复：Louvain 为 CPU 密集型同步计算（大图数秒），必须放到阻塞线程池，
    // 与上方 L996/L1167 两处保持一致，避免占死 tokio worker
    let result = {
        let graph_for_louvain = graph_data.clone();
        tokio::task::spawn_blocking(move || {
            let link_graph = LinkGraph::from_graph_data(graph_for_louvain);
            louvain::detect_communities(link_graph)
        })
        .await
        .map_err(|e| {
            String::from(crate::commands::error::ErrorResponse::from_error(
                e,
                crate::commands::error::ErrorCategory::Unrecoverable,
            ))
        })?
    };

    axagent_dao::repo::wiki_graph_cache::save_cached_graph(
        db,
        &wiki_id,
        &graph_data,
        Some(&result),
    )
    .await
    .map_err(|e| {
        String::from(crate::commands::error::ErrorResponse::from_error(
            e,
            crate::commands::error::ErrorCategory::Unrecoverable,
        ))
    })?;

    Ok(result)
}

/// 带缓存的社区检测**命令入口**：笔记侧 ∪ **实体侧**社区（2026-09-18）。
///
/// 笔记侧由 `note_communities_cached` 算（含缓存），本函数只负责在**所有**
/// 早返回分支之后统一附加一次实体侧结果 —— 这就是把命令属性放在这一层的原因。
#[agent_command(domain = wiki, safety = Safe, call_mode = StateInput, description = "获取 Wiki 社区发现（缓存）")]
#[tauri::command]
pub async fn wiki_graph_communities_cached(
    state: State<'_, AppState>,
    wiki_id: String,
) -> Result<LouvainResult, String> {
    let db = state.harness.db();
    let mut result = note_communities_cached(&state, wiki_id.clone()).await?;

    // ── 实体侧单独算一遍社区 ──
    //
    // 为什么必须单独算，而不是让实体继续靠 `mapping` 锚点继承同名笔记的桶：
    // 融合图里 22,608 个实体节点带着 75,251 条 `reference` 边，而 Louvain 此前
    // 只跑**笔记图**（`note::get_vault_graph`，不含实体）⇒ 实体在社区映射里没有任何
    // 条目 ⇒ 它们的桶完全由「名字是否恰好等于某个笔记标题」决定，与实体在知识图谱
    // 里的实际位置无关。实测代价（真实载荷 46,896 节点 / 172,650 边，见
    // `__tests__/kgFusedScale.test.ts` 的密度对照）：锚点路径下**全部 22,608 条
    // `mapping` 边与 50,059 条 `reference` 边落在桶内、被聚合层直接丢弃** ——
    // 桶级密度 9.8% 这个好看的数字正是这么换来的。
    //
    // 边界（刻意不修的部分）：实体子图上跑 Louvain 与笔记图同量级（数秒），
    // 故结果随 `LouvainResult` 一起写进 `wiki_graph_cache`，只在「未算过」时执行一次。
    // 实体图缓存是 30 秒 TTL、而社区缓存随 notes 写入失效 ⇒ 两者**可能不同源**，
    // 新出现的实体暂时拿不到社区；那批节点由前端锚点兜底且**可观测**
    // （`GraphView.warnOnHashFallback` 会在锚点也兜不住时报出来），不是静默丢弃。
    if result.entity_communities.is_none() {
        let kb_id = axagent_dao::repo::wiki::get_wiki(db, &wiki_id)
            .await
            .ok()
            .and_then(|w| w.knowledge_base_id)
            .unwrap_or_else(|| wiki_id.clone());
        let subgraph = load_fused_entities(db, &kb_id).await;
        if !subgraph.nodes.is_empty() {
            let entity_result = tokio::task::spawn_blocking(move || {
                louvain::detect_communities(LinkGraph::from_graph_data(subgraph))
            })
            .await
            .map_err(|e| {
                String::from(crate::commands::error::ErrorResponse::from_error(
                    e,
                    crate::commands::error::ErrorCategory::Unrecoverable,
                ))
            })?;
            if !entity_result.communities.is_empty() {
                result.entity_communities = Some(entity_result.communities);
                // 回写缓存：否则每次打开图谱页都要白等这几秒
                axagent_dao::repo::wiki_graph_cache::save_cached_communities(db, &wiki_id, &result)
                    .await
                    .map_err(|e| {
                        String::from(crate::commands::error::ErrorResponse::from_error(
                            e,
                            crate::commands::error::ErrorCategory::Unrecoverable,
                        ))
                    })?;
            }
        }
    }

    Ok(result)
}

/// 手动失效缓存（前端在导入/批量编辑后可调用）。
#[agent_command(domain = wiki, safety = Caution, call_mode = StateInput, description = "失效 Wiki 图谱缓存")]
#[tauri::command]
pub async fn invalidate_wiki_graph_cache(
    state: State<'_, AppState>,
    wiki_id: String,
) -> Result<(), String> {
    axagent_dao::repo::wiki_graph_cache::invalidate_cache(state.harness.db(), &wiki_id)
        .await
        .map_err(|e| {
            String::from(crate::commands::error::ErrorResponse::from_error(
                e,
                crate::commands::error::ErrorCategory::Unrecoverable,
            ))
        })
}

#[agent_command(domain = wiki, safety = Caution, call_mode = StateInput, description = "同步笔记到知识库")]
#[tauri::command]
pub async fn sync_note_to_knowledge_base(
    app: AppHandle,
    state: State<'_, AppState>,
    note_id: String,
    knowledge_base_id: String,
) -> Result<(), String> {
    let note =
        axagent_dao::repo::note::get_note(state.harness.db(), &note_id).await.map_err(|e| {
            String::from(crate::commands::error::ErrorResponse::from_error(
                e,
                crate::commands::error::ErrorCategory::Unrecoverable,
            ))
        })?;

    let file_name = format!("{}.md", note.title.replace('/', "_"));
    let data_dir = state.app_data_dir.join("wiki_sync").join(&note.vault_id);
    create_dir_all_blocking(data_dir.clone()).await.map_err(|e| {
        String::from(crate::commands::error::ErrorResponse::from_error(
            e,
            crate::commands::error::ErrorCategory::Unrecoverable,
        ))
    })?;

    let full_path = data_dir.join(&file_name);
    write_file_blocking(full_path.clone(), note.content.into_bytes()).await.map_err(|e| {
        String::from(crate::commands::error::ErrorResponse::from_error(
            e,
            crate::commands::error::ErrorCategory::Unrecoverable,
        ))
    })?;

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
    .map_err(|e| {
        String::from(crate::commands::error::ErrorResponse::from_error(
            e,
            crate::commands::error::ErrorCategory::Unrecoverable,
        ))
    })?;

    let kb =
        axagent_dao::repo::knowledge::get_knowledge_base(state.harness.db(), &knowledge_base_id)
            .await
            .map_err(|e| {
                String::from(crate::commands::error::ErrorResponse::from_error(
                    e,
                    crate::commands::error::ErrorCategory::Unrecoverable,
                ))
            })?;

    if kb.embedding_provider.is_some() {
        let _ = axagent_dao::repo::knowledge::update_document_status(
            state.harness.db(),
            &doc.id,
            "pending",
        )
        .await;
        let _ = crate::index_queue::enqueue_job_sync(
            &state,
            &app,
            jobs::JOB_TYPE_INDEX_DOCUMENT,
            "kb",
            &knowledge_base_id,
            &doc.id,
            None,
            None,
        );
    }

    Ok(())
}

#[agent_command(domain = wiki, safety = Caution, call_mode = StateInput, description = "同步知识库文档到 Wiki")]
#[tauri::command]
pub async fn sync_knowledge_document_to_wiki(
    app: AppHandle,
    state: State<'_, AppState>,
    document_id: String,
    vault_id: String,
) -> Result<(), String> {
    let doc = axagent_dao::repo::knowledge::get_document(state.harness.db(), &document_id)
        .await
        .map_err(|e| {
            String::from(crate::commands::error::ErrorResponse::from_error(
                e,
                crate::commands::error::ErrorCategory::Unrecoverable,
            ))
        })?;

    let content = {
        let path = std::path::Path::new(&doc.source_path);
        if path.exists() {
            axagent_search::sources::parser().extract_text(path, &doc.mime_type).map_err(|e| {
                crate::commands::error::ErrorResponse::err_with_detail(
                    crate::commands::error_code::wiki::IMPORT_FAILED,
                    format!("Failed to extract text: {e}"),
                )
            })?
        } else {
            let collection_name = format!("kb_{}", doc.knowledge_base_id);
            match state.vector_store.list_document_chunks(&collection_name, &doc.id).await {
                Ok(chunks) if !chunks.is_empty() => {
                    chunks.into_iter().map(|c| c.content).collect::<Vec<_>>().join("\n\n")
                },
                _ => {
                    return Err(crate::commands::error::ErrorResponse::err_with_detail(
                        crate::commands::error_code::wiki::IMPORT_FAILED,
                        format!(
                            "Document file not found at '{}' and no indexed chunks available. \
                             The document may have been deleted or the source is a remote URL.",
                            doc.source_path
                        ),
                    ));
                },
            }
        }
    };

    let input = CreateNoteInput {
        vault_id: vault_id.clone(),
        title: doc.title.clone(),
        file_path: format!("synced/{}.md", doc.title.replace('/', "_")),
        content,
        author: "knowledge-sync".to_string(),
        page_type: Some("synced".to_string()),
        source_refs: Some(vec![doc.id.clone()]),
    };

    let note =
        axagent_dao::repo::note::create_note(state.harness.db(), input).await.map_err(|e| {
            String::from(crate::commands::error::ErrorResponse::from_error(
                e,
                crate::commands::error::ErrorCategory::Unrecoverable,
            ))
        })?;

    enqueue_wiki_note_indexing(&state, &app, &vault_id, &note.id);

    Ok(())
}

#[agent_command(domain = wiki, safety = Safe, call_mode = StateInput, description = "获取笔记版本历史")]
#[tauri::command]
pub async fn wiki_note_versions(
    state: State<'_, AppState>,
    note_id: String,
) -> Result<Vec<NoteVersion>, String> {
    wiki::list_versions(state.harness.db(), &note_id).await.map_err(|e| {
        String::from(crate::commands::error::ErrorResponse::from_error(
            e,
            crate::commands::error::ErrorCategory::Unrecoverable,
        ))
    })
}

#[agent_command(domain = wiki, safety = Safe, call_mode = StateInput, description = "获取笔记特定版本")]
#[tauri::command]
pub async fn wiki_note_get_version(
    state: State<'_, AppState>,
    version_id: i64,
) -> Result<NoteVersion, String> {
    wiki::get_version(state.harness.db(), version_id).await.map_err(|e| {
        String::from(crate::commands::error::ErrorResponse::from_error(
            e,
            crate::commands::error::ErrorCategory::Unrecoverable,
        ))
    })
}

#[agent_command(domain = wiki, safety = Caution, call_mode = StateInput, description = "恢复笔记历史版本")]
#[tauri::command]
pub async fn wiki_note_restore_version(
    app: AppHandle,
    state: State<'_, AppState>,
    note_id: String,
    version_id: i64,
) -> Result<Note, String> {
    let version = wiki::get_version(state.harness.db(), version_id).await.map_err(|e| {
        String::from(crate::commands::error::ErrorResponse::from_error(
            e,
            crate::commands::error::ErrorCategory::Unrecoverable,
        ))
    })?;

    let note =
        axagent_dao::repo::note::get_note(state.harness.db(), &note_id).await.map_err(|e| {
            String::from(crate::commands::error::ErrorResponse::from_error(
                e,
                crate::commands::error::ErrorCategory::Unrecoverable,
            ))
        })?;

    wiki::create_version(
        state.harness.db(),
        &note.vault_id,
        &note.id,
        &note.title,
        &note.content,
        &note.author,
    )
    .await
    .map_err(|e| {
        String::from(crate::commands::error::ErrorResponse::from_error(
            e,
            crate::commands::error::ErrorCategory::Unrecoverable,
        ))
    })?;

    let input = UpdateNoteInput {
        title: Some(version.title.clone()),
        content: Some(version.content.clone()),
        page_type: None,
        related_pages: None,
    };

    let updated = axagent_dao::repo::note::update_note(state.harness.db(), &note_id, input)
        .await
        .map_err(|e| {
        String::from(crate::commands::error::ErrorResponse::from_error(
            e,
            crate::commands::error::ErrorCategory::Unrecoverable,
        ))
    })?;

    let _ = wiki::delete_old_versions(state.harness.db(), &note_id, 20).await;

    enqueue_wiki_note_indexing(&state, &app, &updated.vault_id, &updated.id);

    // 操作历史：restore 成功落一条审计记录（失败仅告警，不打断主流程）
    if let Err(e) = wiki::log_wiki_operation(
        state.harness.db(),
        wiki::WikiOperationEntry {
            wiki_id: updated.vault_id.clone(),
            operation_type: "restore".to_string(),
            target_type: "note".to_string(),
            target_id: updated.id.clone(),
            status: "completed".to_string(),
            details: Some(serde_json::json!({ "versionId": version_id })),
            error_message: None,
        },
    )
    .await
    {
        tracing::warn!("[wiki] 记录 restore 操作历史失败: {e}");
    }

    Ok(updated)
}

#[agent_command(domain = wiki, safety = Safe, call_mode = StateInput, description = "列出 Wiki 模板")]
#[tauri::command]
pub async fn wiki_template_list(
    state: State<'_, AppState>,
    wiki_id: String,
) -> Result<Vec<WikiTemplate>, String> {
    wiki::list_wiki_templates(state.harness.db(), &wiki_id).await.map_err(|e| {
        String::from(crate::commands::error::ErrorResponse::from_error(
            e,
            crate::commands::error::ErrorCategory::Unrecoverable,
        ))
    })
}

#[agent_command(domain = wiki, safety = Caution, call_mode = StateInput, description = "创建 Wiki 模板")]
#[tauri::command]
pub async fn wiki_template_create(
    state: State<'_, AppState>,
    input: CreateWikiTemplateInput,
) -> Result<WikiTemplate, String> {
    wiki::create_wiki_template(state.harness.db(), input).await.map_err(|e| {
        String::from(crate::commands::error::ErrorResponse::from_error(
            e,
            crate::commands::error::ErrorCategory::Unrecoverable,
        ))
    })
}

#[agent_command(domain = wiki, safety = Caution, call_mode = StateInput, description = "更新 Wiki 模板")]
#[tauri::command]
pub async fn wiki_template_update(
    state: State<'_, AppState>,
    input: UpdateWikiTemplateInput,
) -> Result<WikiTemplate, String> {
    wiki::update_wiki_template(state.harness.db(), input).await.map_err(|e| {
        String::from(crate::commands::error::ErrorResponse::from_error(
            e,
            crate::commands::error::ErrorCategory::Unrecoverable,
        ))
    })
}

#[agent_command(domain = wiki, safety = Dangerous, call_mode = StateInput, description = "删除 Wiki 模板")]
#[tauri::command]
pub async fn wiki_template_delete(state: State<'_, AppState>, id: String) -> Result<(), String> {
    wiki::delete_wiki_template(state.harness.db(), &id).await.map_err(|e| {
        String::from(crate::commands::error::ErrorResponse::from_error(
            e,
            crate::commands::error::ErrorCategory::Unrecoverable,
        ))
    })
}

#[agent_command(domain = wiki, safety = Caution, call_mode = StateInput, description = "从模板创建笔记")]
#[tauri::command]
pub async fn wiki_note_create_from_template(
    app: AppHandle,
    state: State<'_, AppState>,
    vault_id: String,
    template_id: String,
    title: Option<String>,
) -> Result<Note, String> {
    let template =
        wiki::get_wiki_template(state.harness.db(), &template_id).await.map_err(|e| {
            String::from(crate::commands::error::ErrorResponse::from_error(
                e,
                crate::commands::error::ErrorCategory::Unrecoverable,
            ))
        })?;

    let wiki_obj = wiki::get_wiki(state.harness.db(), &vault_id).await.map_err(|e| {
        String::from(crate::commands::error::ErrorResponse::from_error(
            e,
            crate::commands::error::ErrorCategory::Unrecoverable,
        ))
    })?;

    let content = wiki::apply_template_variables(&template.content, &wiki_obj.name);

    let note_title = title.unwrap_or_else(|| template.name.clone());
    let now = chrono::Utc::now().timestamp();
    let file_path =
        format!("templates/{}-{}.md", template.name.replace(' ', "_").to_lowercase(), now);

    let input = CreateNoteInput {
        vault_id: vault_id.clone(),
        title: note_title,
        file_path,
        content,
        author: "template".to_string(),
        page_type: template.page_type,
        source_refs: None,
    };

    let note =
        axagent_dao::repo::note::create_note(state.harness.db(), input).await.map_err(|e| {
            String::from(crate::commands::error::ErrorResponse::from_error(
                e,
                crate::commands::error::ErrorCategory::Unrecoverable,
            ))
        })?;

    enqueue_wiki_note_indexing(&state, &app, &vault_id, &note.id);

    Ok(note)
}

#[agent_command(domain = wiki, safety = Caution, call_mode = StateInput, description = "创建每日笔记")]
#[tauri::command]
pub async fn wiki_create_daily_note(
    app: AppHandle,
    state: State<'_, AppState>,
    vault_id: String,
) -> Result<Note, String> {
    let today = chrono::Utc::now().format("%Y-%m-%d").to_string();
    let file_path = format!("daily/{}.md", today);

    match axagent_dao::repo::note::get_note_by_path(state.harness.db(), &vault_id, &file_path).await
    {
        Ok(note) => Ok(note),
        Err(_) => {
            let content = format!("# {}\n\n## Tasks\n\n## Notes\n\n## Ideas\n", today);

            let input = CreateNoteInput {
                vault_id: vault_id.clone(),
                title: today.clone(),
                file_path,
                content,
                author: "user".to_string(),
                page_type: Some("daily".to_string()),
                source_refs: None,
            };

            let note = axagent_dao::repo::note::create_note(state.harness.db(), input)
                .await
                .map_err(|e| {
                    String::from(crate::commands::error::ErrorResponse::from_error(
                        e,
                        crate::commands::error::ErrorCategory::Unrecoverable,
                    ))
                })?;

            enqueue_wiki_note_indexing(&state, &app, &vault_id, &note.id);

            Ok(note)
        },
    }
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportStats {
    pub imported: usize,
    pub failed: usize,
    pub skipped: usize,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportStats {
    pub exported: usize,
    pub failed: usize,
}

#[agent_command(domain = wiki, safety = Caution, call_mode = StateInput, description = "导入 Obsidian Vault 到 Wiki")]
#[tauri::command]
pub async fn wiki_import_obsidian_vault(
    app: AppHandle,
    state: State<'_, AppState>,
    wiki_id: String,
    vault_path: String,
) -> Result<ImportStats, String> {
    let root = std::path::Path::new(&vault_path);
    if !root.is_dir() {
        return Err(crate::commands::error::ErrorResponse::err_with_detail(
            crate::commands::error_code::common::INVALID_INPUT,
            format!("Path is not a directory: {vault_path}"),
        ));
    }

    let existing =
        axagent_dao::repo::note::list_notes(state.harness.db(), &wiki_id).await.map_err(|e| {
            String::from(crate::commands::error::ErrorResponse::from_error(
                e,
                crate::commands::error::ErrorCategory::Unrecoverable,
            ))
        })?;
    let existing_titles: std::collections::HashSet<String> =
        existing.iter().map(|n| n.title.clone()).collect();

    let mut md_files: Vec<std::path::PathBuf> = Vec::new();
    collect_md_files(root, &mut md_files);

    let mut imported = 0usize;
    let mut failed = 0usize;
    let mut skipped = 0usize;

    for file_path in &md_files {
        let raw = match read_to_string_blocking(file_path.clone()).await {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!("Failed to read {}: {}", file_path.display(), e);
                failed += 1;
                continue;
            },
        };

        let (frontmatter, content) = parse_frontmatter(&raw);

        let title = frontmatter
            .get("title")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .unwrap_or_else(|| {
                file_path
                    .file_stem()
                    .map(|s| s.to_string_lossy().to_string())
                    .unwrap_or_else(|| "Untitled".to_string())
            });

        if existing_titles.contains(&title) {
            skipped += 1;
            continue;
        }

        let tags: Vec<String> = frontmatter
            .get("tags")
            .and_then(|v| {
                if v.is_sequence() {
                    v.as_sequence().map(|seq| {
                        seq.iter().filter_map(|item| item.as_str().map(String::from)).collect()
                    })
                } else if v.is_string() {
                    v.as_str().map(|s| s.split(',').map(|t| t.trim().to_string()).collect())
                } else {
                    None
                }
            })
            .unwrap_or_default();

        let relative =
            file_path.strip_prefix(root).unwrap_or(file_path).to_string_lossy().to_string();

        let content_with_tags = if tags.is_empty() {
            content.clone()
        } else {
            let tag_lines: Vec<String> = tags.iter().map(|t| format!("#{}", t)).collect();
            format!("{}\n\n{}", tag_lines.join("\n"), content)
        };

        let input = CreateNoteInput {
            vault_id: wiki_id.clone(),
            title,
            file_path: relative,
            content: content_with_tags,
            author: "obsidian-import".to_string(),
            page_type: None,
            source_refs: None,
        };

        match axagent_dao::repo::note::create_note(state.harness.db(), input).await {
            Ok(note) => {
                enqueue_wiki_note_indexing(&state, &app, &wiki_id, &note.id);
                imported += 1;
            },
            Err(e) => {
                tracing::warn!("Failed to create note from {}: {}", file_path.display(), e);
                failed += 1;
            },
        }
    }

    Ok(ImportStats { imported, failed, skipped })
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KnowledgeMdImportStats {
    pub imported: usize,
    pub failed: usize,
    pub skipped: usize,
    pub total: usize,
}

/// 将 KNOWLEDGE.md（精炼知识源）导入为 Wiki 笔记。
/// 按 `## ` 标题分割章节，每个章节创建一条笔记。
/// 自动触发向量索引，使知识可通过 RAG 管道检索。
#[agent_command(domain = wiki, safety = Caution, call_mode = StateInput, description = "导入 Knowledge.md 到 Wiki")]
#[tauri::command]
pub async fn wiki_import_knowledge_md(
    app: AppHandle,
    state: State<'_, AppState>,
    wiki_id: String,
    file_path: Option<String>,
) -> Result<KnowledgeMdImportStats, String> {
    let default_path = std::path::Path::new(".workbuddy/memory/KNOWLEDGE.md");
    let path = file_path.as_deref().unwrap_or_default();
    let knowledge_path = if path.is_empty() {
        default_path
    } else {
        std::path::Path::new(path)
    };

    // 检查文件是否存在
    if !knowledge_path.exists() {
        return Err(crate::commands::error::ErrorResponse::err_with_detail(
            crate::commands::error_code::wiki::NOT_FOUND,
            format!("KNOWLEDGE.md not found at: {}", knowledge_path.display()),
        ));
    }

    let raw = read_to_string_blocking(knowledge_path.to_path_buf()).await.map_err(|e| {
        crate::commands::error::ErrorResponse::err_with_detail(
            crate::commands::error_code::wiki::IMPORT_FAILED,
            format!("Failed to read KNOWLEDGE.md: {e}"),
        )
    })?;
    // 归一化换行符：Windows 下 KNOWLEDGE.md 常为 CRLF，若不在分割前统一，
    // 则 "\n## " 无法匹配（实际为 "\r## "），导致整篇无法按章节导入。
    let raw = raw.replace("\r\n", "\n").replace('\r', "\n");

    // 解析章节：按 `## ` 分割，跳过第一个（标题/引言）
    let sections: Vec<&str> = raw.split("\n## ").collect();
    if sections.is_empty() {
        return Ok(KnowledgeMdImportStats { imported: 0, failed: 0, skipped: 0, total: 0 });
    }

    // 获取已有笔记标题，跳过重复
    let existing =
        axagent_dao::repo::note::list_notes(state.harness.db(), &wiki_id).await.map_err(|e| {
            String::from(crate::commands::error::ErrorResponse::from_error(
                e,
                crate::commands::error::ErrorCategory::Unrecoverable,
            ))
        })?;
    let existing_titles: std::collections::HashSet<String> =
        existing.iter().map(|n| n.title.clone()).collect();

    let mut imported = 0usize;
    let mut failed = 0usize;
    let mut skipped = 0usize;

    for section in &sections {
        // 提取标题和内容
        let (title, content) = if let Some(pos) = section.find('\n') {
            let title_raw = section[..pos].trim().to_string();
            let body = section[pos + 1..].trim().to_string();
            // 跳过 INTRODUCTION 和元信息
            if title_raw.is_empty()
                || title_raw.to_lowercase().contains("introduction")
                || title_raw.starts_with("---")
            {
                continue;
            }
            (title_raw, body)
        } else {
            // 无换行的短片段，跳过
            continue;
        };

        if title.is_empty() {
            continue;
        }

        if existing_titles.contains(&title) {
            skipped += 1;
            continue;
        }

        let input = CreateNoteInput {
            vault_id: wiki_id.clone(),
            title: title.clone(),
            file_path: format!("knowledge/{}.md", title),
            content: format!("## {}\n\n{}", title, content),
            author: "knowledge-md-import".to_string(),
            page_type: Some("knowledge".to_string()),
            source_refs: Some(vec![knowledge_path.to_string_lossy().to_string()]),
        };

        match axagent_dao::repo::note::create_note(state.harness.db(), input).await {
            Ok(note) => {
                enqueue_wiki_note_indexing(&state, &app, &wiki_id, &note.id);
                imported += 1;
            },
            Err(e) => {
                tracing::warn!("Failed to create note from section '{}': {}", title, e);
                failed += 1;
            },
        }
    }

    Ok(KnowledgeMdImportStats { imported, failed, skipped, total: sections.len() })
}

fn collect_md_files(current: &std::path::Path, files: &mut Vec<std::path::PathBuf>) {
    if let Ok(entries) = std::fs::read_dir(current) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                let dir_name =
                    path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
                if dir_name.starts_with('.') {
                    continue;
                }
                collect_md_files(&path, files);
            } else if path.extension().map(|e| e == "md").unwrap_or(false) {
                files.push(path);
            }
        }
    }
}

fn parse_frontmatter(raw: &str) -> (serde_yaml::Value, String) {
    if !raw.starts_with("---") {
        return (serde_yaml::Value::Null, raw.to_string());
    }

    let rest = &raw[3..];
    let end = match rest.find("---") {
        Some(pos) => pos,
        None => return (serde_yaml::Value::Null, raw.to_string()),
    };

    let yaml_str = &rest[..end];
    let body = rest[end + 3..].trim_start_matches('\n').trim_start_matches('\r');

    let frontmatter = match serde_yaml::from_str::<serde_yaml::Value>(yaml_str) {
        Ok(v) => v,
        Err(_) => serde_yaml::Value::Null,
    };

    (frontmatter, body.to_string())
}

#[agent_command(domain = wiki, safety = Safe, call_mode = StateInput, description = "导出 Wiki 为 Markdown")]
#[tauri::command]
pub async fn wiki_export_markdown(
    state: State<'_, AppState>,
    wiki_id: String,
    output_path: String,
) -> Result<ExportStats, String> {
    let notes =
        axagent_dao::repo::note::list_notes(state.harness.db(), &wiki_id).await.map_err(|e| {
            String::from(crate::commands::error::ErrorResponse::from_error(
                e,
                crate::commands::error::ErrorCategory::Unrecoverable,
            ))
        })?;

    let output_dir = std::path::Path::new(&output_path);
    create_dir_all_blocking(output_dir.to_path_buf()).await.map_err(|e| {
        String::from(crate::commands::error::ErrorResponse::from_error(
            e,
            crate::commands::error::ErrorCategory::Unrecoverable,
        ))
    })?;

    let mut exported = 0usize;
    let mut failed = 0usize;

    for note in &notes {
        let sub_dir = if let Some(ref pt) = note.page_type {
            if pt.is_empty() {
                output_dir.to_path_buf()
            } else {
                let d = output_dir.join(sanitize_filename(pt));
                create_dir_all_blocking(d.clone()).await.map_err(|e| {
                    String::from(crate::commands::error::ErrorResponse::from_error(
                        e,
                        crate::commands::error::ErrorCategory::Unrecoverable,
                    ))
                })?;
                d
            }
        } else {
            output_dir.to_path_buf()
        };

        let file_name = format!("{}.md", sanitize_filename(&note.title));
        let full_path = sub_dir.join(&file_name);

        let created_str = format_timestamp(note.created_at);
        let updated_str = format_timestamp(note.updated_at);

        let tags = extract_tags_from_note_content(&note.content);
        let tags_yaml = if tags.is_empty() {
            "[]".to_string()
        } else {
            let items: Vec<String> = tags.iter().map(|t| format!("  - {}", t)).collect();
            format!("\n{}", items.join("\n"))
        };

        let frontmatter = format!(
            "---\ntitle: {}\ntags:{}\ncreated_at: {}\nupdated_at: {}\n---\n",
            escape_yaml_string(&note.title),
            tags_yaml,
            created_str,
            updated_str,
        );

        let file_content = format!("{}{}", frontmatter, note.content);

        match write_file_blocking(full_path.clone(), file_content.into_bytes()).await {
            Ok(_) => exported += 1,
            Err(e) => {
                tracing::warn!("Failed to write {}: {}", full_path.display(), e);
                failed += 1;
            },
        }
    }

    Ok(ExportStats { exported, failed })
}

#[agent_command(domain = wiki, safety = Safe, call_mode = StateInput, description = "导出 Wiki 为 HTML")]
#[tauri::command]
pub async fn wiki_export_html(
    state: State<'_, AppState>,
    wiki_id: String,
    output_path: String,
) -> Result<ExportStats, String> {
    let notes =
        axagent_dao::repo::note::list_notes(state.harness.db(), &wiki_id).await.map_err(|e| {
            String::from(crate::commands::error::ErrorResponse::from_error(
                e,
                crate::commands::error::ErrorCategory::Unrecoverable,
            ))
        })?;

    let output_dir = std::path::Path::new(&output_path);
    create_dir_all_blocking(output_dir.to_path_buf()).await.map_err(|e| {
        String::from(crate::commands::error::ErrorResponse::from_error(
            e,
            crate::commands::error::ErrorCategory::Unrecoverable,
        ))
    })?;

    let mut exported = 0usize;
    let mut failed = 0usize;

    let note_titles: std::collections::HashMap<String, String> =
        notes.iter().map(|n| (n.title.clone(), sanitize_filename(&n.title))).collect();

    for note in &notes {
        let html_file_name = format!("{}.html", sanitize_filename(&note.title));
        let full_path = output_dir.join(&html_file_name);

        let html_body = markdown_to_simple_html(&note.content, &note_titles);

        let html = format!(
            r#"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="UTF-8">
<meta name="viewport" content="width=device-width, initial-scale=1.0">
<title>{}</title>
<style>
body {{ font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Helvetica, Arial, sans-serif; max-width: 980px; margin: 0 auto; padding: 45px; color: #24292e; line-height: 1.6; }}
h1 {{ border-bottom: 1px solid #eaecef; padding-bottom: 0.3em; }}
h2 {{ border-bottom: 1px solid #eaecef; padding-bottom: 0.3em; }}
a {{ color: #0366d6; text-decoration: none; }}
a:hover {{ text-decoration: underline; }}
code {{ background: #f6f8fa; padding: 0.2em 0.4em; border-radius: 3px; font-size: 85%; }}
pre {{ background: #f6f8fa; padding: 16px; border-radius: 6px; overflow: auto; }}
blockquote {{ border-left: 4px solid #dfe2e5; padding: 0 1em; color: #6a737d; margin: 0 0 16px 0; }}
ul, ol {{ padding-left: 2em; }}
.wikilink {{ color: #0366d6; background: #f1f8ff; padding: 1px 4px; border-radius: 3px; }}
</style>
</head>
<body>
<h1>{}</h1>
{}
</body>
</html>"#,
            escape_html(&note.title),
            escape_html(&note.title),
            html_body,
        );

        match write_file_blocking(full_path.clone(), html.into_bytes()).await {
            Ok(_) => exported += 1,
            Err(e) => {
                tracing::warn!("Failed to write {}: {}", full_path.display(), e);
                failed += 1;
            },
        }
    }

    let index_path = output_dir.join("index.html");
    let mut index_items = String::new();
    for note in &notes {
        let href = format!("{}.html", sanitize_filename(&note.title));
        index_items.push_str(&format!(
            r#"<li><a href="{}">{}</a></li>"#,
            href,
            escape_html(&note.title),
        ));
    }

    let index_html = format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="UTF-8">
<meta name="viewport" content="width=device-width, initial-scale=1.0">
<title>Wiki Index</title>
<style>
body {{ font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Helvetica, Arial, sans-serif; max-width: 980px; margin: 0 auto; padding: 45px; color: #24292e; line-height: 1.6; }}
h1 {{ border-bottom: 1px solid #eaecef; padding-bottom: 0.3em; }}
a {{ color: #0366d6; text-decoration: none; }}
a:hover {{ text-decoration: underline; }}
ul {{ list-style: none; padding-left: 0; }}
li {{ padding: 4px 0; }}
</style>
</head>
<body>
<h1>Wiki Index</h1>
<ul>
{}
</ul>
</body>
</html>"#,
        index_items,
    );

    write_file_blocking(index_path.clone(), index_html.into_bytes()).await.map_err(|e| {
        String::from(crate::commands::error::ErrorResponse::from_error(
            e,
            crate::commands::error::ErrorCategory::Unrecoverable,
        ))
    })?;

    Ok(ExportStats { exported, failed })
}

#[agent_command(domain = wiki, safety = Safe, call_mode = StateInput, description = "导出笔记为 HTML")]
#[tauri::command]
pub async fn wiki_note_export_html(
    state: State<'_, AppState>,
    note_id: String,
    output_path: String,
) -> Result<String, String> {
    let note =
        axagent_dao::repo::note::get_note(state.harness.db(), &note_id).await.map_err(|e| {
            String::from(crate::commands::error::ErrorResponse::from_error(
                e,
                crate::commands::error::ErrorCategory::Unrecoverable,
            ))
        })?;

    let output = std::path::Path::new(&output_path);
    if let Some(parent) = output.parent() {
        create_dir_all_blocking(parent.to_path_buf()).await.map_err(|e| {
            String::from(crate::commands::error::ErrorResponse::from_error(
                e,
                crate::commands::error::ErrorCategory::Unrecoverable,
            ))
        })?;
    }

    let html_body = markdown_to_simple_html(&note.content, &std::collections::HashMap::new());

    let html = format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="UTF-8">
<title>{}</title>
<style>
body {{ font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Helvetica, Arial, sans-serif; max-width: 980px; margin: 0 auto; padding: 45px; color: #24292e; line-height: 1.6; }}
h1 {{ border-bottom: 1px solid #eaecef; padding-bottom: 0.3em; }}
h2 {{ border-bottom: 1px solid #eaecef; padding-bottom: 0.3em; }}
a {{ color: #0366d6; text-decoration: none; }}
code {{ background: #f6f8fa; padding: 0.2em 0.4em; border-radius: 3px; font-size: 85%; }}
pre {{ background: #f6f8fa; padding: 16px; border-radius: 6px; overflow: auto; }}
blockquote {{ border-left: 4px solid #dfe2e5; padding: 0 1em; color: #6a737d; margin: 0 0 16px 0; }}
ul, ol {{ padding-left: 2em; }}
.wikilink {{ color: #0366d6; background: #f1f8ff; padding: 1px 4px; border-radius: 3px; }}
@media print {{ body {{ padding: 0; max-width: none; }} }}
</style>
</head>
<body>
<h1>{}</h1>
{}
</body>
</html>"#,
        escape_html(&note.title),
        escape_html(&note.title),
        html_body,
    );

    // 内容始终为 HTML，统一输出为 .html 文件（即便调用方传入 .pdf 后缀也改写为 .html，
    // 本命令并不真正生成 PDF，避免名实不符）。
    let html_output = output.with_extension("html");

    write_file_blocking(html_output.clone(), html.into_bytes()).await.map_err(|e| {
        String::from(crate::commands::error::ErrorResponse::from_error(
            e,
            crate::commands::error::ErrorCategory::Unrecoverable,
        ))
    })?;

    let html_path = html_output.to_string_lossy().to_string();
    let _ = open::that(&html_output);

    Ok(html_path)
}

fn sanitize_filename(name: &str) -> String {
    let sanitized: String = name
        .chars()
        .map(|c| {
            if c == '/'
                || c == '\\'
                || c == ':'
                || c == '*'
                || c == '?'
                || c == '"'
                || c == '<'
                || c == '>'
                || c == '|'
            {
                '_'
            } else {
                c
            }
        })
        .collect();
    let trimmed = sanitized.trim();
    if trimmed.is_empty() {
        "untitled".to_string()
    } else {
        trimmed.to_string()
    }
}

/// P1-2: Wiki 一键建链 — 基于 embedding 相似度自动发现 note 间的关联并创建链接
///
/// 流程：
/// 1. 加载 vault 中所有 notes
/// 2. 为每个 note 计算 embedding（复用 wiki 索引的向量）
/// 3. 两两计算余弦相似度
/// 4. 超过阈值的 note 对自动创建 note_links 记录
/// 5. 返回创建的链接数量
///
/// 参数：
/// - `vault_id`: Wiki vault ID
/// - `threshold`: 相似度阈值（0-1，默认 0.7）
/// - `max_links_per_note`: 每个 note 最多创建的链接数（默认 5）
/// - `dry_run`: 仅计算不写入 DB（用于预览效果）
#[derive(Debug, Deserialize)]
pub struct AutoConnectOptions {
    pub threshold: Option<f64>,
    pub max_links_per_note: Option<usize>,
    pub dry_run: Option<bool>,
}

#[derive(Debug, Serialize)]
pub struct AutoConnectResult {
    pub links_created: usize,
    pub pairs_analyzed: usize,
    pub elapsed_ms: u64,
}

#[agent_command(domain = wiki, safety = Safe, call_mode = StateInput, description = "自动连接 Wiki")]
#[tauri::command]
pub async fn auto_connect_wiki(
    state: State<'_, AppState>,
    vault_id: String,
    options: Option<AutoConnectOptions>,
) -> Result<AutoConnectResult, String> {
    let started = std::time::Instant::now();
    let db = state.harness.db();
    let opts = options.unwrap_or(AutoConnectOptions {
        threshold: None,
        max_links_per_note: None,
        dry_run: None,
    });

    let threshold = opts.threshold.unwrap_or(0.7).clamp(0.0, 1.0);
    let max_links = opts.max_links_per_note.unwrap_or(5).clamp(1, 20);
    let dry_run = opts.dry_run.unwrap_or(false);

    // 1. 加载所有 notes
    let notes = axagent_dao::repo::note::list_notes(db, &vault_id).await.map_err(|e| {
        String::from(crate::commands::error::ErrorResponse::from_error(
            e,
            crate::commands::error::ErrorCategory::Unrecoverable,
        ))
    })?;

    if notes.len() < 2 {
        return Ok(AutoConnectResult {
            links_created: 0,
            pairs_analyzed: 0,
            elapsed_ms: started.elapsed().as_millis() as u64,
        });
    }

    // 2. 获取 embedding provider 并生成 embeddings
    let collection_id = collection_id(WikiVaultRAG.collection_prefix(), &vault_id);
    let embeddings_map = compute_note_embeddings(&state, &notes, &collection_id).await?;

    if embeddings_map.is_empty() {
        return Ok(AutoConnectResult {
            links_created: 0,
            pairs_analyzed: 0,
            elapsed_ms: started.elapsed().as_millis() as u64,
        });
    }

    // 3. 计算两两相似度并建链
    let mut links_to_create: Vec<(String, String, String, String)> = Vec::new(); // (source_id, target_id, link_text, link_type)
    let mut pairs_analyzed = 0;

    for i in 0..notes.len() {
        let source_id = &notes[i].id;
        let _source_title = &notes[i].title;
        let source_emb = match embeddings_map.get(source_id) {
            Some(emb) => emb,
            None => continue,
        };

        // 收集与其他 notes 的相似度
        let mut candidates: Vec<(String, f64)> = Vec::new();
        for (j, target_note) in notes.iter().enumerate() {
            if i == j {
                continue;
            }
            let target_id = &target_note.id;
            let target_emb = match embeddings_map.get(target_id) {
                Some(emb) => emb,
                None => continue,
            };
            pairs_analyzed += 1;

            let similarity = cosine_similarity(source_emb, target_emb);
            if similarity >= threshold {
                candidates.push((target_id.clone(), similarity));
            }
        }

        // 按相似度排序，取 top-N
        candidates.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        candidates.truncate(max_links);

        for (target_id, _sim) in &candidates {
            let target_title = notes
                .iter()
                .find(|n| n.id == *target_id)
                .map(|n| n.title.clone())
                .unwrap_or_else(|| "unknown".to_string());
            let link_text = format!("相关内容：{}", target_title);
            links_to_create.push((
                source_id.clone(),
                target_id.clone(),
                link_text,
                "auto_similar".to_string(),
            ));
        }
    }

    // 4. 写入 DB（或 dry-run 仅返回统计）
    if !dry_run && !links_to_create.is_empty() {
        // 按 source_note_id 分组，批量写入
        use std::collections::HashMap;
        let mut grouped: HashMap<String, Vec<(String, String, String)>> = HashMap::new();
        for (source_id, target_id, link_text, link_type) in &links_to_create {
            grouped.entry(source_id.clone()).or_default().push((
                target_id.clone(),
                link_text.clone(),
                link_type.clone(),
            ));
        }

        for (source_id, links) in &grouped {
            let _ =
                axagent_dao::repo::note::sync_note_links(db, &vault_id, source_id, links.clone())
                    .await;
        }
    }

    Ok(AutoConnectResult {
        links_created: links_to_create.len(),
        pairs_analyzed,
        elapsed_ms: started.elapsed().as_millis() as u64,
    })
}

/// 计算 cosine 相似度
fn cosine_similarity(a: &[f64], b: &[f64]) -> f64 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }
    let dot_product: f64 = a.iter().zip(b.iter()).map(|(x, y)| x * y).sum();
    let norm_a: f64 = a.iter().map(|x| x * x).sum::<f64>().sqrt();
    let norm_b: f64 = b.iter().map(|x| x * x).sum::<f64>().sqrt();
    if norm_a == 0.0 || norm_b == 0.0 {
        return 0.0;
    }
    dot_product / (norm_a * norm_b)
}

/// 为 notes 计算 embedding（使用轻量级哈希向量，避免重度依赖 LLM embedding）
async fn compute_note_embeddings(
    _state: &State<'_, AppState>,
    notes: &[Note],
    _collection_id: &str,
) -> Result<std::collections::HashMap<String, Vec<f64>>, String> {
    let mut embeddings_map: std::collections::HashMap<String, Vec<f64>> =
        std::collections::HashMap::new();

    // 使用 note 内容的字符 n-gram 哈希作为轻量级"语义指纹"
    // 注意：这里使用简单的关键词哈希向量，避免重度依赖 LLM embedding
    for note in notes {
        let hash_emb = compute_hash_embedding(&note.content, note.title.as_str());
        embeddings_map.insert(note.id.clone(), hash_emb);
    }

    Ok(embeddings_map)
}

/// 基于字符 n-gram 的哈希向量（简化版 embedding，维度 256）
/// 用于快速相似度计算，不依赖外部 embedding 服务
fn compute_hash_embedding(content: &str, title: &str) -> Vec<f64> {
    const DIMS: usize = 256;
    let mut vec = vec![0.0f64; DIMS];

    // 结合标题和内容，给标题更高权重
    let combined = format!("{} {}", title, content);
    let chars: Vec<char> = combined.chars().collect();

    if chars.is_empty() {
        return vec;
    }

    // 3-gram 哈希
    for window in chars.windows(3) {
        let ngram: String = window.iter().collect();
        let hash = simple_hash(&ngram);
        let idx = (hash as usize) % DIMS;
        vec[idx] += 1.0;
    }

    // 归一化
    let norm: f64 = vec.iter().map(|x| x * x).sum::<f64>().sqrt();
    if norm > 0.0 {
        for v in &mut vec {
            *v /= norm;
        }
    }

    vec
}

/// 简单字符串哈希（FNV-1a 变体）
fn simple_hash(s: &str) -> u64 {
    let mut hash: u64 = 14695981039346656037;
    for byte in s.bytes() {
        hash ^= byte as u64;
        hash = hash.wrapping_mul(1099511628211u64);
    }
    hash
}

fn format_timestamp(ts: i64) -> String {
    chrono::DateTime::from_timestamp(ts, 0)
        .map(|dt| dt.format("%Y-%m-%dT%H:%M:%SZ").to_string())
        .unwrap_or_else(|| ts.to_string())
}

fn escape_yaml_string(s: &str) -> String {
    if s.contains(':') || s.contains('#') || s.contains('"') || s.contains('\'') || s.contains('\n')
    {
        format!("\"{}\"", s.replace('"', "\\\""))
    } else {
        s.to_string()
    }
}

fn escape_html(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

fn extract_tags_from_note_content(content: &str) -> Vec<String> {
    let mut tags = Vec::new();
    for line in content.lines() {
        let line = line.trim();
        if line.starts_with('#') && !line.starts_with("##") {
            let tag = line.trim_start_matches('#').trim().to_string();
            if !tag.is_empty() {
                tags.push(tag);
            }
        }
    }
    tags
}

fn markdown_to_simple_html(
    md: &str,
    note_titles: &std::collections::HashMap<String, String>,
) -> String {
    let mut html = String::new();
    let mut in_list = false;
    let mut in_code_block = false;
    let mut code_content = String::new();

    for line in md.lines() {
        if line.trim().starts_with("```") {
            if in_code_block {
                html.push_str(&format!("<pre><code>{}</code></pre>\n", escape_html(&code_content)));
                code_content.clear();
                in_code_block = false;
            } else {
                if in_list {
                    html.push_str("</ul>\n");
                    in_list = false;
                }
                in_code_block = true;
            }
            continue;
        }

        if in_code_block {
            code_content.push_str(line);
            code_content.push('\n');
            continue;
        }

        let trimmed = line.trim();

        if trimmed.is_empty() {
            if in_list {
                html.push_str("</ul>\n");
                in_list = false;
            }
            continue;
        }

        if let Some(stripped) = trimmed.strip_prefix("### ") {
            if in_list {
                html.push_str("</ul>\n");
                in_list = false;
            }
            html.push_str(&format!("<h3>{}</h3>\n", escape_html(stripped)));
        } else if let Some(stripped) = trimmed.strip_prefix("## ") {
            if in_list {
                html.push_str("</ul>\n");
                in_list = false;
            }
            html.push_str(&format!("<h2>{}</h2>\n", escape_html(stripped)));
        } else if let Some(stripped) = trimmed.strip_prefix("# ") {
            if in_list {
                html.push_str("</ul>\n");
                in_list = false;
            }
            html.push_str(&format!("<h1>{}</h1>\n", escape_html(stripped)));
        } else if trimmed.starts_with("- ") || trimmed.starts_with("* ") {
            if !in_list {
                html.push_str("<ul>\n");
                in_list = true;
            }
            let item_text = inline_markdown_to_html(&trimmed[2..], note_titles);
            html.push_str(&format!("<li>{}</li>\n", item_text));
        } else if let Some(stripped) = trimmed.strip_prefix("> ") {
            if in_list {
                html.push_str("</ul>\n");
                in_list = false;
            }
            html.push_str(&format!(
                "<blockquote>{}</blockquote>\n",
                inline_markdown_to_html(stripped, note_titles)
            ));
        } else {
            if in_list {
                html.push_str("</ul>\n");
                in_list = false;
            }
            html.push_str(&format!("<p>{}</p>\n", inline_markdown_to_html(trimmed, note_titles)));
        }
    }

    if in_list {
        html.push_str("</ul>\n");
    }
    if in_code_block {
        html.push_str(&format!("<pre><code>{}</code></pre>\n", escape_html(&code_content)));
    }

    html
}

fn inline_markdown_to_html(
    text: &str,
    note_titles: &std::collections::HashMap<String, String>,
) -> String {
    let result = escape_html(text);
    let result = replace_wikilinks(&result, note_titles);
    let result = replace_inline_pairs(&result, "**", "<strong>", "</strong>");
    let result = replace_inline_pairs(&result, "*", "<em>", "</em>");
    let result = replace_inline_backticks(&result);
    replace_inline_links(&result)
}

fn replace_wikilinks(
    text: &str,
    note_titles: &std::collections::HashMap<String, String>,
) -> String {
    let mut result = String::new();
    let text_len = text.len();

    let bytes: &[u8] = text.as_bytes();
    let mut pos = 0usize;

    while pos < text_len {
        if bytes[pos] == b'[' && pos + 1 < text_len && bytes[pos + 1] == b'[' {
            if let Some(end) = find_closing_brackets(bytes, pos + 2) {
                let link_text = &text[pos + 2..end];
                let file_name = note_titles
                    .get(link_text)
                    .cloned()
                    .unwrap_or_else(|| sanitize_filename(link_text));
                result.push_str(&format!(
                    "<a href=\"{}.html\" class=\"wikilink\">{}</a>",
                    file_name, link_text
                ));
                pos = end + 2;
                continue;
            }
        }
        result.push(bytes[pos] as char);
        pos += 1;
    }

    result
}

fn find_closing_brackets(bytes: &[u8], start: usize) -> Option<usize> {
    let mut pos = start;
    while pos + 1 < bytes.len() {
        if bytes[pos] == b']' && bytes[pos + 1] == b']' {
            return Some(pos);
        }
        pos += 1;
    }
    None
}

fn replace_inline_pairs(text: &str, marker: &str, open_tag: &str, close_tag: &str) -> String {
    let mut result = String::new();
    let mut remaining = text;
    let marker_len = marker.len();

    while let Some(start) = remaining.find(marker) {
        result.push_str(&remaining[..start]);
        let after_first = &remaining[start + marker_len..];
        if let Some(end) = after_first.find(marker) {
            let inner = &after_first[..end];
            result.push_str(open_tag);
            result.push_str(inner);
            result.push_str(close_tag);
            remaining = &after_first[end + marker_len..];
        } else {
            result.push_str(marker);
            remaining = after_first;
        }
    }
    result.push_str(remaining);
    result
}

fn replace_inline_backticks(text: &str) -> String {
    let mut result = String::new();
    let mut remaining = text;

    while let Some(start) = remaining.find('`') {
        result.push_str(&remaining[..start]);
        let after_first = &remaining[start + 1..];
        if let Some(end) = after_first.find('`') {
            let inner = &after_first[..end];
            result.push_str("<code>");
            result.push_str(inner);
            result.push_str("</code>");
            remaining = &after_first[end + 1..];
        } else {
            result.push('`');
            remaining = after_first;
        }
    }
    result.push_str(remaining);
    result
}

fn replace_inline_links(text: &str) -> String {
    let mut result = String::new();
    let mut remaining = text;

    while let Some(start) = remaining.find('[') {
        result.push_str(&remaining[..start]);
        let after_bracket = &remaining[start + 1..];

        if let Some(close_bracket) = after_bracket.find(']') {
            let link_text = &after_bracket[..close_bracket];
            let after_close = &after_bracket[close_bracket + 1..];

            if let Some(after_open) = after_close.strip_prefix('(') {
                if let Some(close_paren) = after_open.find(')') {
                    let url = &after_open[..close_paren];
                    result.push_str(&format!("<a href=\"{}\">{}</a>", url, link_text));
                    remaining = &after_close[close_paren + 2..];
                    continue;
                }
            }
            result.push('[');
            remaining = after_bracket;
        } else {
            result.push('[');
            remaining = after_bracket;
        }
    }
    result.push_str(remaining);
    result
}

/// 修复 Wiki 图谱关联结果
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RepairWikiGraphResult {
    /// Wiki ID
    pub wiki_id: String,
    /// 关联的 KB ID
    pub kb_id: Option<String>,
    /// 修复的笔记数量
    pub repaired_notes: usize,
    /// 是否成功关联了 KB
    pub kb_linked: bool,
    /// 消息
    pub message: String,
}

/// 修复 Wiki 图谱关联：
/// 1. 自动关联 Wiki 与 KB（如未关联）
/// 2. 遍历所有笔记，重新解析 [[wikilink]] 并同步到 note_links 表
/// 3. 失效图谱缓存，确保下次加载获取最新数据
#[agent_command(domain = wiki, safety = Caution, call_mode = StateInput, description = "修复 Wiki 图谱关联")]
#[tauri::command]
pub async fn repair_wiki_graph(
    state: State<'_, AppState>,
    wiki_id: String,
) -> Result<RepairWikiGraphResult, String> {
    let db = state.harness.db();

    // 1. 获取 Wiki 信息
    let wiki = axagent_dao::repo::wiki::get_wiki(db, &wiki_id).await.map_err(|e| {
        String::from(crate::commands::error::ErrorResponse::from_error(
            e,
            crate::commands::error::ErrorCategory::Unrecoverable,
        ))
    })?;

    let mut kb_id = wiki.knowledge_base_id.clone();
    let mut kb_linked = kb_id.is_some();

    // 2. 如果 Wiki 未关联 KB，尝试通过名称匹配自动关联
    if kb_id.is_none() {
        let expected_kb_name = format!("{}图谱", wiki.name);
        let kbs = axagent_dao::repo::knowledge::list_knowledge_bases(db).await.map_err(|e| {
            String::from(crate::commands::error::ErrorResponse::from_error(
                e,
                crate::commands::error::ErrorCategory::Unrecoverable,
            ))
        })?;

        if let Some(matching_kb) = kbs.into_iter().find(|kb| kb.name == expected_kb_name) {
            // 自动关联
            axagent_dao::repo::wiki::update_wiki(
                db,
                &wiki_id,
                None,
                None,
                None,
                Some(Some(matching_kb.id.clone())),
            )
            .await
            .map_err(|e| {
                String::from(crate::commands::error::ErrorResponse::from_error(
                    e,
                    crate::commands::error::ErrorCategory::Unrecoverable,
                ))
            })?;

            kb_id = Some(matching_kb.id);
            kb_linked = true;
            tracing::info!("[repair_graph] 自动关联 Wiki {} 与 KB", wiki_id);
        }
    }

    // 3. 修复 wikilink：批量重建全 vault 链接表
    //    旧实现逐篇调用 sync_note_links_from_content（每篇全量加载 vault 笔记，O(N²)），
    //    2 万篇笔记永远跑不完；且与导入顺序无关的全量映射才能修复批量导入时
    //    被丢弃的前向引用（指向尚未导入笔记的 [[wikilink]]）。
    let (repaired_notes, link_count) =
        axagent_dao::repo::note::resync_vault_note_links(db, &wiki_id).await.map_err(|e| {
            String::from(crate::commands::error::ErrorResponse::from_error(
                e,
                crate::commands::error::ErrorCategory::Unrecoverable,
            ))
        })?;
    tracing::info!(
        "[repair_graph] 批量重建链接完成: {} 篇笔记, {} 条链接 (wiki={})",
        repaired_notes,
        link_count,
        wiki_id
    );

    // 4. 失效图谱缓存
    let _ = axagent_dao::repo::wiki_graph_cache::invalidate_cache(db, &wiki_id).await;

    let message = if kb_linked {
        format!("修复完成：已关联 KB，修复 {} 篇笔记的链接", repaired_notes)
    } else {
        format!("修复完成：修复 {} 篇笔记的链接（未找到匹配的 KB）", repaired_notes)
    };

    Ok(RepairWikiGraphResult { wiki_id, kb_id, repaired_notes, kb_linked, message })
}
