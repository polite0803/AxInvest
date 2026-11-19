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

/// `update_wiki` op 的入参。
///
/// 只有 `wikiId` 必填；其余四个字段缺省 / `null` 都表示「保持原值」，
/// 与宿主 dao `update_wiki` 的判空语义逐字对齐。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateWikiRequest {
    pub wiki_id: String,
    pub name: Option<String>,
    pub description: Option<String>,
    pub embedding_provider: Option<String>,
    pub knowledge_base_id: Option<String>,
}

/// `update_wiki` op 的回执。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WikiUpdateResult {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub embedding_provider: Option<String>,
    /// 关联的知识库 ID
    pub knowledge_base_id: Option<String>,
    /// 旧 provider 与新 provider 是否不同；前端据此决定是否触发重建索引
    pub embedding_changed: bool,
}

/// `repair_wiki_graph` op 的回执。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RepairWikiGraphResult {
    pub wiki_id: String,
    /// 修复后关联到的 KB（无匹配 KB 时为 `None`）
    pub kb_id: Option<String>,
    pub repaired_notes: usize,
    pub kb_linked: bool,
    /// 宿主生成的本地化中文消息；插件侧只透传，不参与判定
    pub message: String,
}

/// Wiki 写接缝的宿主 provider。
#[async_trait]
pub trait WikiStoreProvider: Send + Sync {
    /// 更新一个 Wiki 的元数据（写）。
    ///
    /// 只有 `request.wikiId` 必填；其余四个字段缺省 / `null` 都表示「保持原值」，
    /// 与宿主 dao `update_wiki` 的判空语义逐字对齐。回执里的 `embeddingChanged`
    /// 由宿主对比更新前后的 provider 得出，前端据此决定是否触发一次重建索引。
    async fn update_wiki(&self, _request: UpdateWikiRequest) -> Result<WikiUpdateResult> {
        Err(AxAgentError::Provider(
            "WikiStoreProvider::update_wiki 未实现（该提供者不支持此数据面）".into(),
        ))
    }

    /// 重建一个 Wiki 的向量索引（写，**异步**）。
    ///
    /// 宿主清掉旧 collection 后在后台逐篇重索引，并按篇发 `wiki-note-indexed` 事件、
    /// 完成时发 `wiki-rebuild-complete` 事件；本 op 在**清库后立即返回**，
    /// 不等后台任务结束（前端靠事件刷新进度，不靠本 op 的回执）。
    async fn rebuild_index(&self, _wiki_id: String) -> Result<()> {
        Err(AxAgentError::Provider(
            "WikiStoreProvider::rebuild_index 未实现（该提供者不支持此数据面）".into(),
        ))
    }

    /// 修复一个 Wiki 的图谱关联（写）：自动关联 KB + 全量重建链接表 + 失效图谱缓存。
    async fn repair_graph(&self, _wiki_id: String) -> Result<RepairWikiGraphResult> {
        Err(AxAgentError::Provider(
            "WikiStoreProvider::repair_graph 未实现（该提供者不支持此数据面）".into(),
        ))
    }

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

    /// 删除一个 Wiki 模板（写，危险）。
    ///
    /// 宿主侧完成 DB 行删除。要求插件声明 `write`，权限检查在 worker 分派层完成。
    async fn delete_wiki_template(&self, _id: String) -> Result<()> {
        Err(AxAgentError::Provider(
            "WikiStoreProvider::delete_wiki_template 未实现（该提供者不支持此数据面）".into(),
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

    /// 读取一个 Wiki 的**原图**（笔记 + 链接，**不含**实体融合与缓存）（读）。
    ///
    /// 与 `get_cached_graph` 是**两个不同口径**：后者是「融合图谱 + 缓存 + 实体侧」，
    /// 本条是「笔记图原样 + 每次实时算」。两者都留在宿主 provider 内完成；
    /// 插件只提交 `wikiId`，不复制构图算法。
    async fn get_graph(&self, _wiki_id: String) -> Result<GraphData> {
        Err(AxAgentError::Provider(
            "WikiStoreProvider::get_graph 未实现（该提供者不支持此数据面）".into(),
        ))
    }

    /// 对一个 Wiki 的笔记图跑一次 Louvain 社区检测（读，**不写缓存**）。
    ///
    /// 与 `get_cached_communities` 的差别同上：后者读/写社区缓存，本条每次实时算。
    /// CPU 密集计算（`spawn_blocking`）留在宿主 provider 内。
    async fn detect_communities(&self, _wiki_id: String) -> Result<LouvainResult> {
        Err(AxAgentError::Provider(
            "WikiStoreProvider::detect_communities 未实现（该提供者不支持此数据面）".into(),
        ))
    }

    /// 手动失效一个 Wiki 的图谱缓存（写）。
    ///
    /// 缓存行删除属写面 ⇒ 要求插件声明 `write`，权限检查在 worker 分派层完成。
    async fn invalidate_graph_cache(&self, _wiki_id: String) -> Result<()> {
        Err(AxAgentError::Provider(
            "WikiStoreProvider::invalidate_graph_cache 未实现（该提供者不支持此数据面）".into(),
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

    /// 从 Wiki 模板创建笔记（写）。
    ///
    /// 模板读取、变量替换、标题/路径生成、笔记落库与索引入队全在宿主 provider 内完成；
    /// 插件只提交 `vaultId` / `templateId` / 可选 `title`。
    async fn create_note_from_template(
        &self,
        _vault_id: String,
        _template_id: String,
        _title: Option<String>,
    ) -> Result<crate::note_dtos::Note> {
        Err(AxAgentError::Provider(
            "WikiStoreProvider::create_note_from_template 未实现（该提供者不支持此数据面）".into(),
        ))
    }

    /// 取得或创建当天的 Wiki 每日笔记（写）。
    ///
    /// 日期、路径与幂等判重由宿主 provider 决定；插件只提交 `vaultId`。
    async fn create_daily_note(&self, _vault_id: String) -> Result<crate::note_dtos::Note> {
        Err(AxAgentError::Provider(
            "WikiStoreProvider::create_daily_note 未实现（该提供者不支持此数据面）".into(),
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

    /// 搜索一个 vault 下的 Wiki 笔记（读）。
    ///
    /// hybrid 检索、embedding 调用、FTS 回退、片段高亮与评分排序全部留在宿主 provider；
    /// 插件只提交 `vaultId` / `query` / 可选 `topK`。
    async fn search_notes(
        &self,
        _vault_id: String,
        _query: String,
        _top_k: Option<usize>,
    ) -> Result<Vec<crate::rag_config::NoteSearchResult>> {
        Err(AxAgentError::Provider(
            "WikiStoreProvider::search_notes 未实现（该提供者不支持此数据面）".into(),
        ))
    }

    /// 把一个 Obsidian vault 目录导入为 Wiki 笔记（写）。
    ///
    /// 目录遍历、frontmatter 解析、笔记落库与索引入队全部在宿主 provider 内完成；
    /// 插件只提交 `wikiId` 与 vault 目录路径。
    async fn import_obsidian_vault(
        &self,
        _wiki_id: String,
        _vault_path: String,
    ) -> Result<crate::wiki_dtos::ImportStats> {
        Err(AxAgentError::Provider(
            "WikiStoreProvider::import_obsidian_vault 未实现（该提供者不支持此数据面）".into(),
        ))
    }

    /// 把 KNOWLEDGE.md 按章节导入为 Wiki 笔记（写）。
    ///
    /// 换行归一化、章节切分、判重、笔记落库与索引入队全部在宿主 provider 内完成；
    /// 插件只提交 `wikiId` 与可选文件路径。
    async fn import_knowledge_md(
        &self,
        _wiki_id: String,
        _file_path: Option<String>,
    ) -> Result<crate::wiki_dtos::KnowledgeMdImportStats> {
        Err(AxAgentError::Provider(
            "WikiStoreProvider::import_knowledge_md 未实现（该提供者不支持此数据面）".into(),
        ))
    }

    /// 把一个 Wiki 导出为 Markdown 目录（写，落盘）。
    ///
    /// 目录创建、frontmatter 生成与文件写入全部在宿主 provider 内完成；
    /// 插件只提交 `wikiId` 与输出目录路径。
    async fn export_markdown(
        &self,
        _wiki_id: String,
        _output_path: String,
    ) -> Result<crate::wiki_dtos::ExportStats> {
        Err(AxAgentError::Provider(
            "WikiStoreProvider::export_markdown 未实现（该提供者不支持此数据面）".into(),
        ))
    }

    /// 把一个 Wiki 导出为 HTML 目录（写，落盘）。
    ///
    /// Markdown → HTML 转换、样式模板与 `index.html` 生成全部在宿主 provider 内完成；
    /// 插件只提交 `wikiId` 与输出目录路径。
    async fn export_html(
        &self,
        _wiki_id: String,
        _output_path: String,
    ) -> Result<crate::wiki_dtos::ExportStats> {
        Err(AxAgentError::Provider(
            "WikiStoreProvider::export_html 未实现（该提供者不支持此数据面）".into(),
        ))
    }

    /// 把一条笔记导出为 HTML 文件（写，落盘）并返回落盘路径。
    ///
    /// 宿主侧统一把扩展名改写为 `.html`（本能力不真正生成 PDF），
    /// 落盘后打开系统默认浏览器；插件只提交 `noteId` 与目标路径。
    async fn export_note_html(
        &self,
        _note_id: String,
        _output_path: String,
    ) -> Result<crate::wiki_dtos::NoteExportPath> {
        Err(AxAgentError::Provider(
            "WikiStoreProvider::export_note_html 未实现（该提供者不支持此数据面）".into(),
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
