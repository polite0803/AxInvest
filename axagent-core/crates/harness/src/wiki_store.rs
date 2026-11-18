// SPDX-License-Identifier: AGPL-3.0-only

//! Wiki 同步写接缝（`wiki.store`）— DTO + Trait。
//!
//! 宿主把「笔记 → markdown 临时文件 → 知识库文档 → 可选索引任务」这条需要
//! `AppState` / `AppHandle` 的链路经本接缝暴露给插件侧。插件只提交两个 id，
//! 不持有数据库连接、不读文件、不猜宿主的数据目录；命令体的唯一权威在宿主
//! provider。
//!
//! 写面权限：`sync_note_to_knowledge_base` 会落文档行并可能触发 embedding 计费，
//! 要求插件声明 `write`；权限检查在 worker 分派层完成，本 trait 不重复校验。

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::core_error::{AxAgentError, Result};

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

/// Wiki 写接缝的宿主 provider。
#[async_trait]
pub trait WikiStoreProvider: Send + Sync {
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
