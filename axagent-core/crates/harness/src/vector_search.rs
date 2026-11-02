// SPDX-License-Identifier: AGPL-3.0-only

//! 向量检索抽象契约（任务 #37，`PLAN-shell-plus-plugins.md` §0.12.3 ① 随附前置）
//!
//! 历史上本 trait 定义在 consumer crate `agent/src/query_engine.rs`，而插件 SDK 不含
//! agent ⇒ 第三方够不着它。按 PLAN 裁定下沉到 harness（foundation，trait 契约层），
//! 原位置保留 `pub use axagent_harness::VectorSearch;` 不断链。
//!
//! 签名与下沉前逐字一致（零行为变更）；它是 wiki 检索注入口，与同目录
//! `vector_store` 模块的三个存储面 trait 是不同抽象，勿混用。

/// wiki 侧向量召回契约：按 `wiki_id` 拿最近邻 note 得分列表。
#[async_trait::async_trait]
pub trait VectorSearch: Send + Sync {
    async fn search(
        &self,
        wiki_id: &str,
        query_embedding: &[f32],
        top_k: usize,
    ) -> Result<Vec<(String, f64)>, String>;
}
