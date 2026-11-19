// SPDX-License-Identifier: AGPL-3.0-only

//! Wiki 业务数据面接缝（`wiki.store`）— DTO + Trait。
//!
//! 宿主把「笔记 → markdown 临时文件 → 知识库文档 → 可选索引任务」这条需要
//! `AppState` / `AppHandle` 的链路经本接缝暴露给插件侧。插件只提交两个 id，
//! 不持有数据库连接、不读文件、不猜宿主的数据目录；命令体的唯一权威在宿主
//! provider。
//!
//! 写面权限：`sync_note_to_knowledge_base` 会落文档行并可能触发 embedding 计费，
//! 要求插件声明 `write`；权限检查在 worker 分派层完成，本 trait 不重复校验。
//! 读面 `list_notes` / `get_note` / `get_note_by_path` 只回宿主笔记视图
//! （`Note` DTO 已在 harness），不设权限闸。

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::core_error::{AxAgentError, Result};
use crate::graph_dtos::GraphData;
use crate::louvain_dtos::LouvainResult;
use crate::types::{BacklinkInfo, NoteLink};
use crate::wiki_dtos::NoteVersion;

/// `sync_note_to_knowledge_base` op 的入参。
///
/// 两个 id 都必须是宿主内的稳定标识；数据源正文由宿主按 note id 读取，插件侧
/// 不传 title / content / source path，避免把宿主数据主权搬到插件进程。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncNoteToKnowledgeBaseRequest {
    pub note_id: String,
    pub knowledge_base_id: String,
}

/// `sync_note_to_knowledge_base` op 的返回。
///
/// `index_queued` 只说明宿主是否把文档送入索引队列，不代表索引已经完成；
/// 索引仍是异步任务，前端不得把它当作检索立即可用的承诺。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncNoteToKnowledgeBaseOutcome {
    pub document_id: String,
    pub index_queued: bool,
}

/// `sync_note_links` op 的入参。
///
/// `links` 是 `(target_title, link_text, link_type)` 三元组；正文与文件主权
/// 仍在宿主，插件只转发前端已经解析出的链接关系。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncNoteLinksRequest {
    pub vault_id: String,
    pub source_note_id: String,
    pub links: Vec<(String, String, String)>,
}

/// Wiki 写接缝的宿主 provider。
#[async_trait]
pub trait WikiStoreProvider: Send + Sync {
    /// 列出笔记的出链（`note_links` 中以该 note 为 source 的行）。
    async fn get_note_links(&self, _note_id: String) -> Result<Vec<NoteLink>> {
        Err(AxAgentError::Provider(
            "WikiStoreProvider::get_note_links 未实现（该提供者不支持此数据面）".into(),
        ))
    }

    /// 列出指向该笔记的反链，含 source 笔记标题与上下文片段。
    async fn get_note_backlinks(&self, _note_id: String) -> Result<Vec<BacklinkInfo>> {
        Err(AxAgentError::Provider(
            "WikiStoreProvider::get_note_backlinks 未实现（该提供者不支持此数据面）".into(),
        ))
    }

    /// 同步一条笔记的出链关系（写）。
    ///
    /// 宿主侧完成 DB 落行；要求插件声明 `write`，权限检查在 worker 分派层完成。
    async fn sync_note_links(&self, _request: SyncNoteLinksRequest) -> Result<()> {
        Err(AxAgentError::Provider(
            "WikiStoreProvider::sync_note_links 未实现（该提供者不支持此数据面）".into(),
        ))
    }

    /// 列出一个 vault 下的 Wiki 模板（读）。
    async fn list_wiki_templates(
        &self,
        _wiki_id: String,
    ) -> Result<Vec<crate::wiki_dtos::WikiTemplate>> {
        Err(AxAgentError::Provider(
            "WikiStoreProvider::list_wiki_templates 未实现（该提供者不支持此数据面）".into(),
        ))
    }

    /// 创建一个 Wiki 模板（写）。
    async fn create_wiki_template(
        &self,
        _input: crate::wiki_dtos::CreateWikiTemplateInput,
    ) -> Result<crate::wiki_dtos::WikiTemplate> {
        Err(AxAgentError::Provider(
            "WikiStoreProvider::create_wiki_template 未实现（该提供者不支持此数据面）".into(),
        ))
    }

    /// 更新一个 Wiki 模板（写）。
    async fn update_wiki_template(
        &self,
        _input: crate::wiki_dtos::UpdateWikiTemplateInput,
    ) -> Result<crate::wiki_dtos::WikiTemplate> {
        Err(AxAgentError::Provider(
            "WikiStoreProvider::update_wiki_template 未实现（该提供者不支持此数据面）".into(),
        ))
    }

    /// 读取带缓存的 Wiki 融合图谱（读）。
    ///
    /// 缓存命中、知识图谱实体融合、悬空边诊断与告警节流全部留在宿主 provider；
    /// 插件只提交 `wikiId`，不复制算法，也不接触 DAO / 数据库连接。
    async fn get_cached_graph(&self, _wiki_id: String) -> Result<GraphData> {
        Err(AxAgentError::Provider(
            "WikiStoreProvider::get_cached_graph 未实现（该提供者不支持此数据面）".into(),
        ))
    }

    /// 读取带缓存的 Wiki 社区检测结果（读）。
    ///
    /// Louvain 计算与实体侧社区附加都在宿主 provider 内完成；插件只透传结果 DTO。
    async fn get_cached_communities(&self, _wiki_id: String) -> Result<LouvainResult> {
        Err(AxAgentError::Provider(
            "WikiStoreProvider::get_cached_communities 未实现（该提供者不支持此数据面）".into(),
        ))
    }

    /// 列出一条笔记的历史版本（读）。
    async fn list_note_versions(&self, _note_id: String) -> Result<Vec<NoteVersion>> {
        Err(AxAgentError::Provider(
            "WikiStoreProvider::list_note_versions 未实现（该提供者不支持此数据面）".into(),
        ))
    }

    /// 按版本 id 读取一个笔记版本（读）。
    async fn get_note_version(&self, _version_id: i64) -> Result<NoteVersion> {
        Err(AxAgentError::Provider(
            "WikiStoreProvider::get_note_version 未实现（该提供者不支持此数据面）".into(),
        ))
    }

    /// 恢复一个笔记历史版本（写）。
    ///
    /// 宿主先把当前正文存成新版本，再更新笔记正文、清理超量旧版本、入队索引并写审计；
    /// 插件只提交 `noteId` + `versionId`，不接触数据库、文件与队列。
    async fn restore_note_version(
        &self,
        _note_id: String,
        _version_id: i64,
    ) -> Result<crate::note_dtos::Note> {
        Err(AxAgentError::Provider(
            "WikiStoreProvider::restore_note_version 未实现（该提供者不支持此数据面）".into(),
        ))
    }

    /// 创建一条 Wiki 笔记（写）。
    ///
    /// 入参 `crate::note_dtos::CreateNoteInput` 是 harness 权威 DTO；宿主 provider 在
    /// 同一处完成建行、图谱缓存失效、反馈数据湖写入与索引任务入队。
    async fn create_note(
        &self,
        _input: crate::note_dtos::CreateNoteInput,
    ) -> Result<crate::note_dtos::Note> {
        Err(AxAgentError::Provider(
            "WikiStoreProvider::create_note 未实现（该提供者不支持此数据面）".into(),
        ))
    }

    /// 更新一条 Wiki 笔记（写）。
    ///
    /// 旧内容在宿主侧做版本备份（最多 20 个版本）；随后更新笔记、失效图谱缓存、
    /// 写反馈数据湖并入队索引任务。入参用 harness 权威 `UpdateNoteInput`。
    async fn update_note(
        &self,
        _id: String,
        _input: crate::note_dtos::UpdateNoteInput,
    ) -> Result<crate::note_dtos::Note> {
        Err(AxAgentError::Provider(
            "WikiStoreProvider::update_note 未实现（该提供者不支持此数据面）".into(),
        ))
    }

    /// 删除一条 Wiki 笔记（写，危险）。
    ///
    /// 宿主侧同步完成向量删除、缓存失效与 DB 行删除。要求插件声明 `write`。
    async fn delete_note(&self, _id: String) -> Result<()> {
        Err(AxAgentError::Provider(
            "WikiStoreProvider::delete_note 未实现（该提供者不支持此数据面）".into(),
        ))
    }

    /// 列出一个 vault 下的全部未删除笔记（按标题升序，与宿主 `list_notes` 同口径）。
    async fn list_notes(&self, _vault_id: String) -> Result<Vec<crate::note_dtos::Note>> {
        Err(AxAgentError::Provider(
            "WikiStoreProvider::list_notes 未实现（该提供者不支持此数据面）".into(),
        ))
    }

    /// 按 id 读取一条 Wiki 笔记；不存在由实现方返回 `NotFound`。
    ///
    /// 返回 `crate::note_dtos::Note`（harness 权威 DTO），插件不接触 `notes::Model`。
    async fn get_note(&self, _id: String) -> Result<crate::note_dtos::Note> {
        Err(AxAgentError::Provider(
            "WikiStoreProvider::get_note 未实现（该提供者不支持此数据面）".into(),
        ))
    }

    /// 按 vault + 相对路径读取一条未删除笔记；不存在由实现方返回 `NotFound`。
    async fn get_note_by_path(
        &self,
        _vault_id: String,
        _file_path: String,
    ) -> Result<crate::note_dtos::Note> {
        Err(AxAgentError::Provider(
            "WikiStoreProvider::get_note_by_path 未实现（该提供者不支持此数据面）".into(),
        ))
    }

    /// 把一条宿主笔记同步进知识库；不存在 / 非法 id 由实现方返回 `Validation`。
    async fn sync_note_to_knowledge_base(
        &self,
        _request: SyncNoteToKnowledgeBaseRequest,
    ) -> Result<SyncNoteToKnowledgeBaseOutcome> {
        Err(AxAgentError::Provider(
            "WikiStoreProvider::sync_note_to_knowledge_base 未实现（该提供者不支持此数据面）"
                .into(),
        ))
    }
}
