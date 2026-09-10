// SPDX-License-Identifier: AGPL-3.0-only

//! 能力发现系统的真实嵌入提供者
//!
//! 能力发现系统（tools crate 的 CapabilityIndexer / CapabilityRetriever）通过
//! harness 层的 `EmbeddingProvider` trait 生成嵌入向量。早期实现使用
//! `MockEmbeddingProvider`（FNV 哈希伪随机向量，无语义含义）。
//!
//! 本模块在 **wiring 层**（src/）提供真实实现：内部复用 `indexing::generate_embeddings`
//! （与 RAG 知识库同源的嵌入链路），通过 DB 中的 embedding provider 配置 +
//! master_key + provider registry 调用真实 LLM 嵌入服务。
//!
//! # 架构定位
//! 能力发现系统作为"基座"（tools crate）不应持有具体 provider 配置（API key、
//! provider id 等），因此真实实现的依赖注入（db / master_key / registry /
//! embedding_provider 字符串）由 wiring 层完成，harness 与 tools 均不感知。

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use axagent_harness::rag_provider::EmbeddingProvider;
use axagent_harness::registry::ProviderRegistry;
use axagent_harness::types::ModelType;
use sea_orm::DatabaseConnection;

use crate::indexing::generate_embeddings;

/// 真实嵌入提供者
///
/// 复用 RAG 系统的 `generate_embeddings`（含 token 预算分片、过大自动二分、重试退避），
/// 保证任意长度的文本都能稳定生成指定维度的向量。
#[derive(Clone)]
pub struct CapabilityEmbeddingProvider {
    db: DatabaseConnection,
    master_key: [u8; 32],
    provider_registry: Arc<dyn ProviderRegistry>,
    /// `"providerId::model_id"` 格式的嵌入模型配置
    embedding_provider: String,
    /// 固定向量维度（由启动时探测得到，保证与既有向量库维度一致）
    dimensions: usize,
}

#[async_trait]
impl EmbeddingProvider for CapabilityEmbeddingProvider {
    async fn embed(&self, text: &str) -> Result<Vec<f32>, String> {
        let resp = generate_embeddings(
            &self.db,
            &self.master_key,
            &self.provider_registry,
            &self.embedding_provider,
            vec![text.to_string()],
            Some(self.dimensions),
        )
        .await
        // BE-I1 修复：嵌入失败返回携带错误码的结构化错误，前端可 i18n。
        .map_err(|e| {
            axagent_harness::error_codes::error_json(
                axagent_harness::error_codes::capability::EMBEDDING_FAILED,
                format!("能力嵌入失败: {e}"),
            )
        })?;
        Ok(resp.embeddings.into_iter().next().unwrap_or_default())
    }

    async fn embed_batch(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, String> {
        let resp = generate_embeddings(
            &self.db,
            &self.master_key,
            &self.provider_registry,
            &self.embedding_provider,
            texts.to_vec(),
            Some(self.dimensions),
        )
        .await
        // BE-I1 修复：嵌入失败返回携带错误码的结构化错误，前端可 i18n。
        .map_err(|e| {
            axagent_harness::error_codes::error_json(
                axagent_harness::error_codes::capability::EMBEDDING_FAILED,
                format!("能力嵌入失败: {e}"),
            )
        })?;
        Ok(resp.embeddings)
    }

    fn dimension(&self) -> usize {
        self.dimensions
    }
}

/// 创建能力发现系统的嵌入提供者
///
/// 自动发现系统中第一个启用的 embedding 类型 provider + model，拼接成
/// `"providerId::model_id"` 并确定向量维度。未配置任何 embedding provider
/// （或探测失败）时回退 `MockEmbeddingProvider`，保证能力发现系统始终可用
/// （语义检索质量受限，启动时打 warning 提示）。
///
/// 维度确定策略（惰性持久化，避免启动期网络探测阻塞首屏）：
/// 1. 优先读本地缓存文件 `{axagent_home}/capability_embedding_dims.json`
///    （键 = `"providerId::model_id"`，值 = 上次探测到的维度）；
/// 2. 缓存命中 → 直接构造，**零网络请求**；
/// 3. 缓存未命中（首次运行 / 换了 provider 或模型）→ 才发起真实探测
///    （带 5s 硬超时），成功后写回缓存，下次启动不再探测。
pub async fn create_capability_embedding_provider(
    sea_db: &DatabaseConnection,
    master_key: &[u8; 32],
    harness: &axagent_runtime::harness::RuntimeHarness,
) -> Arc<dyn EmbeddingProvider> {
    let registry = harness.provider_registry().clone();
    match discover_system_embedding_provider(sea_db, master_key, &registry).await {
        Some((embedding_provider, dimensions)) => {
            tracing::info!(
                "[capability] 使用真实嵌入服务 {}（维度 {}）",
                embedding_provider,
                dimensions
            );
            Arc::new(CapabilityEmbeddingProvider {
                db: sea_db.clone(),
                master_key: *master_key,
                provider_registry: registry,
                embedding_provider,
                dimensions,
            })
        },
        None => {
            tracing::warn!(
                "[capability] 未发现可用的 embedding provider，回退 MockEmbeddingProvider，语义检索质量受限"
            );
            Arc::new(axagent_tools::MockEmbeddingProvider::new(1536))
        },
    }
}

// ── 维度缓存（本地 JSON 文件） ─────────────────────────────────────────────

/// 维度缓存文件路径：`{axagent_home}/capability_embedding_dims.json`。
fn dims_cache_path() -> PathBuf {
    crate::paths::axagent_home().join("capability_embedding_dims.json")
}

/// 读取维度缓存。文件不存在 / 解析失败时返回空 map（下次探测成功后重建）。
fn load_dims_cache() -> HashMap<String, usize> {
    let path = dims_cache_path();
    match std::fs::read_to_string(&path) {
        Ok(content) => serde_json::from_str(&content).unwrap_or_else(|e| {
            tracing::warn!(
                "[capability] 维度缓存解析失败（忽略，将重新探测）: {}: {e}",
                path.display()
            );
            HashMap::new()
        }),
        Err(_) => HashMap::new(),
    }
}

/// 写维度缓存。失败仅告警（下次启动重新探测，不影响正确性）。
fn save_dims_cache(cache: &HashMap<String, usize>) {
    let path = dims_cache_path();
    match serde_json::to_string_pretty(cache) {
        Ok(content) => {
            if let Err(e) = std::fs::write(&path, content) {
                tracing::warn!("[capability] 维度缓存写入失败: {}: {e}", path.display());
            }
        },
        Err(e) => tracing::warn!("[capability] 维度缓存序列化失败: {e}"),
    }
}

/// 从 providers 表发现第一个启用的 embedding provider + model，
/// 确定向量维度。按顺序尝试：第一个候选不可用时继续尝试下一个，全部失败返回 `None`。
///
/// 维度来源优先级：
/// 1. **本地缓存**（`capability_embedding_dims.json`）——命中则零网络请求，
///    启动路径不再被不可达的 embedding 服务阻塞（P0-启动阻塞修复）；
/// 2. **真实探测**——仅缓存未命中时执行，带 5s 硬超时（含内部重试），
///    超时/失败视为该 provider 不可用，立即换下一个候选。
///    之前无超时保护时，网络不可达的 provider 会走 3 次重试 × 30s
///    connect_timeout，单个候选最坏卡 ~92s，多个叠加导致安装版长时间白屏。
async fn discover_system_embedding_provider(
    db: &DatabaseConnection,
    master_key: &[u8; 32],
    provider_registry: &Arc<dyn ProviderRegistry>,
) -> Option<(String, usize)> {
    // BE-S2 修复：查询 provider 列表失败时记录日志而非 `.ok()?` 静默吞错，
    // 避免整个能力向量库空缺却无告警。
    let providers = match axagent_dao::repo::provider::list_providers_merged(db).await {
        Ok(p) => p,
        Err(e) => {
            tracing::warn!("[capability] 查询 embedding provider 列表失败: {e}");
            return None;
        },
    };

    let mut dims_cache = load_dims_cache();

    for provider in &providers {
        if !provider.enabled {
            continue;
        }
        let Some(model) =
            provider.models.iter().find(|m| m.model_type == ModelType::Embedding && m.enabled)
        else {
            // 当前 provider 无启用的 embedding 模型，尝试下一个
            continue;
        };
        let embedding_provider = format!("{}::{}", provider.id, model.model_id);

        // 1) 缓存命中：直接使用持久化维度，不发网络请求
        if let Some(&dims) = dims_cache.get(&embedding_provider) {
            if dims > 0 {
                tracing::info!(
                    "[capability] embedding provider {} 维度命中本地缓存（{dims}），跳过网络探测",
                    embedding_provider
                );
                return Some((embedding_provider, dims));
            }
        }

        // 2) 缓存未命中：才发起真实探测（传 None 让服务端返回默认维度），
        //    5s 硬超时包住整个调用（含 embed_with_retry 内部重试）。
        match tokio::time::timeout(
            std::time::Duration::from_secs(5),
            generate_embeddings(
                db,
                master_key,
                provider_registry,
                &embedding_provider,
                vec!["capability-dimension-probe".to_string()],
                None,
            ),
        )
        .await
        {
            Ok(Ok(resp)) if resp.dimensions > 0 => {
                dims_cache.insert(embedding_provider.clone(), resp.dimensions);
                save_dims_cache(&dims_cache);
                return Some((embedding_provider, resp.dimensions));
            },
            Ok(Ok(_)) => {
                tracing::warn!(
                    "[capability] embedding provider {} 探测无维度信息，尝试下一个",
                    embedding_provider
                );
            },
            Ok(Err(e)) => {
                tracing::warn!(
                    "[capability] embedding provider {} 探测失败: {}",
                    embedding_provider,
                    e
                );
            },
            Err(_) => {
                tracing::warn!(
                    "[capability] embedding provider {} 探测超时（5s，疑似服务未启动或网络不可达），尝试下一个",
                    embedding_provider
                );
            },
        }
    }
    None
}
