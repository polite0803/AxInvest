// SPDX-License-Identifier: AGPL-3.0-only

//! OPC 域包 Token 预算管理 — 分区缓存 + 上下文压缩 + 干叶分离
//!
//! 为每个域包工作流提供独立的 token 预算管理：
//! - 分区缓存：按域包/会话隔离 token 预算，避免跨域包干扰
//! - 上下文压缩：当 token 用量接近阈值时自动压缩历史消息
//! - 干叶分离：将活跃上下文（当前任务）与历史上下文（干叶）分离，
//!   干叶信息压缩为摘要保留在域包知识缓存中

use std::collections::HashMap;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

/// 域包 Token 预算配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DomainPackTokenConfig {
    /// 上下文窗口大小（token 数）
    #[serde(default = "default_context_window")]
    pub context_window: u32,
    /// 压缩阈值（百分比，0-100），超过此值触发压缩
    #[serde(default = "default_compact_threshold")]
    pub compact_threshold_pct: u32,
    /// 干叶分离阈值（百分比），超过此值将历史消息转为干叶
    #[serde(default = "default_dry_leaf_threshold")]
    pub dry_leaf_threshold_pct: u32,
    /// 最大保留历史消息数（压缩后）
    #[serde(default = "default_max_history_messages")]
    pub max_history_messages: usize,
    /// 域包名称
    pub domain_pack_name: String,
}

fn default_context_window() -> u32 {
    200_000
}

fn default_compact_threshold() -> u32 {
    80
}

fn default_dry_leaf_threshold() -> u32 {
    60
}

fn default_max_history_messages() -> usize {
    20
}

impl Default for DomainPackTokenConfig {
    fn default() -> Self {
        Self {
            context_window: default_context_window(),
            compact_threshold_pct: default_compact_threshold(),
            dry_leaf_threshold_pct: default_dry_leaf_threshold(),
            max_history_messages: default_max_history_messages(),
            domain_pack_name: "通用".to_string(),
        }
    }
}

/// Token 使用快照
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenUsageSnapshot {
    /// 时间戳（毫秒）
    pub timestamp_ms: u64,
    /// 输入 token 数
    pub input_tokens: u32,
    /// 输出 token 数
    pub output_tokens: u32,
    /// 总 token 数
    pub total_tokens: u32,
}

/// 干叶条目 — 被压缩的历史上下文摘要
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DryLeafEntry {
    /// 条目 ID
    pub id: String,
    /// 域包 ID
    pub domain_pack_id: String,
    /// 会话 ID
    pub session_id: String,
    /// 原始消息摘要
    pub summary: String,
    /// 关联的关键词（用于检索）
    pub keywords: Vec<String>,
    /// 时间戳（毫秒）
    pub created_at_ms: u64,
    /// 预估节省的 token 数
    pub saved_tokens: u32,
}

/// 压缩结果
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompactionResult {
    /// 是否执行了压缩
    pub compacted: bool,
    /// 压缩前 token 数
    pub before_tokens: u32,
    /// 压缩后 token 数
    pub after_tokens: u32,
    /// 节省的 token 数
    pub saved_tokens: u32,
    /// 被转为干叶的消息摘要
    pub dry_leaf_summaries: Vec<String>,
}

/// 预算评估决策
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum BudgetDecision {
    /// 正常，无需操作
    Proceed,
    /// 建议压缩
    CompactRecommended { current_pct: u32 },
    /// 强制压缩
    CompactRequired { current_pct: u32 },
    /// 建议干叶分离
    DryLeafSeparationRecommended { current_pct: u32 },
}

/// 域包 Token 预算管理器
///
/// 为每个域包工作流提供独立的 token 预算管理，支持：
/// - 分区缓存：按域包/会话隔离 token 预算
/// - 上下文压缩：当 token 用量接近阈值时自动压缩
/// - 干叶分离：将历史消息转为摘要存储
#[derive(Debug)]
pub struct DomainPackTokenBudgetManager {
    /// 各域包配置
    configs: Arc<Mutex<HashMap<String, DomainPackTokenConfig>>>,
    /// 各域包/会话的 token 使用历史
    usage_history: Arc<Mutex<HashMap<String, Vec<TokenUsageSnapshot>>>>,
    /// 各域包的干叶缓存
    dry_leaves: Arc<Mutex<HashMap<String, Vec<DryLeafEntry>>>>,
}

impl Default for DomainPackTokenBudgetManager {
    fn default() -> Self {
        Self::new()
    }
}

impl DomainPackTokenBudgetManager {
    /// 创建新的预算管理器
    pub fn new() -> Self {
        Self {
            configs: Arc::new(Mutex::new(HashMap::new())),
            usage_history: Arc::new(Mutex::new(HashMap::new())),
            dry_leaves: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// 注册域包配置
    pub async fn register_domain_pack(&self, domain_pack_id: &str, config: DomainPackTokenConfig) {
        let mut configs = self.configs.lock().await;
        configs.insert(domain_pack_id.to_string(), config);
    }

    /// 获取域包配置
    pub async fn get_config(&self, domain_pack_id: &str) -> DomainPackTokenConfig {
        let configs = self.configs.lock().await;
        configs.get(domain_pack_id).cloned().unwrap_or_default()
    }

    /// 记录 token 使用快照
    pub async fn record_usage(
        &self,
        domain_pack_id: &str,
        session_id: &str,
        input_tokens: u32,
        output_tokens: u32,
    ) {
        let key = format!("{}:{}", domain_pack_id, session_id);
        let mut history = self.usage_history.lock().await;
        let snapshots = history.entry(key).or_default();

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;

        snapshots.push(TokenUsageSnapshot {
            timestamp_ms: now,
            input_tokens,
            output_tokens,
            total_tokens: input_tokens + output_tokens,
        });

        // 保留最近 50 条记录
        if snapshots.len() > 50 {
            snapshots.drain(..snapshots.len() - 50);
        }
    }

    /// 评估预算并返回决策
    pub async fn evaluate_budget(
        &self,
        domain_pack_id: &str,
        _session_id: &str,
        current_tokens: u32,
    ) -> BudgetDecision {
        let config = self.get_config(domain_pack_id).await;
        let pct = ((current_tokens as f64 / config.context_window as f64) * 100.0) as u32;

        if pct >= 95 {
            BudgetDecision::CompactRequired { current_pct: pct }
        } else if pct >= config.compact_threshold_pct {
            BudgetDecision::CompactRecommended { current_pct: pct }
        } else if pct >= config.dry_leaf_threshold_pct {
            BudgetDecision::DryLeafSeparationRecommended { current_pct: pct }
        } else {
            BudgetDecision::Proceed
        }
    }

    /// 执行上下文压缩
    pub async fn compact_context(
        &self,
        domain_pack_id: &str,
        session_id: &str,
        messages: &[String],
    ) -> CompactionResult {
        let config = self.get_config(domain_pack_id).await;
        let before_tokens = Self::estimate_message_tokens(messages);

        if before_tokens == 0 {
            return CompactionResult {
                compacted: false,
                before_tokens: 0,
                after_tokens: 0,
                saved_tokens: 0,
                dry_leaf_summaries: Vec::new(),
            };
        }

        // 保留最近的消息，压缩历史消息
        let keep_count = config.max_history_messages.min(messages.len());
        let mut dry_leaf_summaries = Vec::new();
        let mut compressed_messages = Vec::new();

        // 生成历史消息的摘要
        for (i, msg) in messages.iter().enumerate().take(messages.len() - keep_count) {
            let summary = generate_brief_summary(msg);
            dry_leaf_summaries.push(summary.clone());

            // 保存为干叶
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64;

            let leaf = DryLeafEntry {
                id: format!("dry-leaf-{}-{}", domain_pack_id, i),
                domain_pack_id: domain_pack_id.to_string(),
                session_id: session_id.to_string(),
                summary,
                keywords: extract_keywords(msg),
                created_at_ms: now,
                saved_tokens: (msg.len() as u32 / 4).max(10),
            };

            let mut dry_leaves = self.dry_leaves.lock().await;
            let domain_pack_leaves = dry_leaves.entry(domain_pack_id.to_string()).or_default();
            domain_pack_leaves.push(leaf);
        }

        // 保留最近的消息
        compressed_messages.extend(messages.iter().rev().take(keep_count).rev().cloned());

        let after_tokens = Self::estimate_message_tokens(&compressed_messages);

        CompactionResult {
            compacted: true,
            before_tokens,
            after_tokens,
            saved_tokens: before_tokens.saturating_sub(after_tokens),
            dry_leaf_summaries,
        }
    }

    /// 执行干叶分离
    pub async fn separate_dry_leaves(
        &self,
        domain_pack_id: &str,
        session_id: &str,
        messages: &[String],
    ) -> (Vec<String>, Vec<DryLeafEntry>) {
        let config = self.get_config(domain_pack_id).await;
        let total_tokens = Self::estimate_message_tokens(messages);

        if total_tokens == 0 {
            return (messages.to_vec(), Vec::new());
        }

        let pct = ((total_tokens as f64 / config.context_window as f64) * 100.0) as u32;

        // 如果低于干叶分离阈值，直接返回原消息
        if pct < config.dry_leaf_threshold_pct {
            return (messages.to_vec(), Vec::new());
        }

        // 分离：保留最近的消息作为活跃上下文
        let active_count = (messages.len() as f64 * 0.3).ceil() as usize;
        let active_count = active_count.max(3).min(messages.len());

        let active_messages: Vec<String> =
            messages.iter().rev().take(active_count).rev().cloned().collect();
        let history_messages: Vec<String> =
            messages.iter().take(messages.len() - active_count).cloned().collect();

        // 将历史消息转为干叶
        let mut new_leaves = Vec::new();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;

        for (i, msg) in history_messages.iter().enumerate() {
            let leaf = DryLeafEntry {
                id: format!("dry-leaf-{}-{}-{}", domain_pack_id, session_id, i),
                domain_pack_id: domain_pack_id.to_string(),
                session_id: session_id.to_string(),
                summary: generate_brief_summary(msg),
                keywords: extract_keywords(msg),
                created_at_ms: now,
                saved_tokens: (msg.len() as u32 / 4).max(10),
            };
            new_leaves.push(leaf);
        }

        // 保存干叶
        let mut dry_leaves = self.dry_leaves.lock().await;
        let domain_pack_leaves = dry_leaves.entry(domain_pack_id.to_string()).or_default();
        domain_pack_leaves.extend(new_leaves.clone());

        (active_messages, new_leaves)
    }

    /// 检索域包干叶缓存
    pub async fn retrieve_dry_leaves(
        &self,
        domain_pack_id: &str,
        query: &str,
        limit: usize,
    ) -> Vec<DryLeafEntry> {
        let dry_leaves = self.dry_leaves.lock().await;
        let domain_pack_leaves = match dry_leaves.get(domain_pack_id) {
            Some(leaves) => leaves,
            None => return Vec::new(),
        };

        // 简单关键词匹配
        let query_lower = query.to_lowercase();
        let mut scored_leaves: Vec<(&DryLeafEntry, usize)> = domain_pack_leaves
            .iter()
            .map(|leaf| {
                let score = leaf
                    .keywords
                    .iter()
                    .filter(|kw| kw.to_lowercase().contains(&query_lower))
                    .count();
                (leaf, score)
            })
            .collect();

        // 按得分排序
        scored_leaves.sort_by_key(|b| std::cmp::Reverse(b.1));

        scored_leaves.into_iter().take(limit).map(|(leaf, _)| leaf.clone()).collect()
    }

    /// 估算消息 token 数（使用字符数/4的启发式估算）
    pub fn estimate_message_tokens(messages: &[String]) -> u32 {
        messages.iter().map(|m| (m.chars().count() as u32 / 4).max(1)).sum()
    }

    /// 获取域包统计信息
    pub async fn get_domain_pack_stats(&self, domain_pack_id: &str) -> DomainPackTokenStats {
        let usage_history = self.usage_history.lock().await;
        let dry_leaves = self.dry_leaves.lock().await;

        let total_usage: u32 = usage_history
            .values()
            .flat_map(|snapshots| snapshots.iter())
            .map(|s| s.total_tokens)
            .sum();

        let domain_pack_leaves = dry_leaves.get(domain_pack_id).cloned().unwrap_or_default();
        let total_saved: u32 = domain_pack_leaves.iter().map(|l| l.saved_tokens).sum();

        DomainPackTokenStats {
            domain_pack_id: domain_pack_id.to_string(),
            total_usage,
            dry_leaf_count: domain_pack_leaves.len() as u32,
            total_saved,
        }
    }

    /// 清理过期的干叶条目（保留最近 N 条）
    pub async fn cleanup_old_leaves(&self, domain_pack_id: &str, keep_count: usize) {
        let mut dry_leaves = self.dry_leaves.lock().await;
        if let Some(leaves) = dry_leaves.get_mut(domain_pack_id)
            && leaves.len() > keep_count
        {
            leaves.drain(..leaves.len() - keep_count);
        }
    }
}

/// 域包 Token 统计
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DomainPackTokenStats {
    pub domain_pack_id: String,
    pub total_usage: u32,
    pub dry_leaf_count: u32,
    pub total_saved: u32,
}

/// 生成简短摘要
fn generate_brief_summary(text: &str) -> String {
    let max_chars = 100;
    let cleaned = text.trim().replace('\n', " ");
    if cleaned.chars().count() <= max_chars {
        cleaned
    } else {
        let truncated: String = cleaned.chars().take(max_chars).collect();
        format!("{}...", truncated)
    }
}

/// 提取关键词
fn extract_keywords(text: &str) -> Vec<String> {
    let cleaned = text.trim().to_lowercase();
    let words: Vec<String> = cleaned
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| w.len() >= 3)
        .map(|w| w.to_string())
        .collect();

    // 去重并限制数量
    let mut seen = std::collections::HashSet::new();
    words.into_iter().filter(|w| seen.insert(w.clone())).take(10).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_register_domain_pack() {
        let manager = DomainPackTokenBudgetManager::new();
        manager
            .register_domain_pack(
                "test-domain-pack",
                DomainPackTokenConfig {
                    domain_pack_name: "测试域包".to_string(),
                    context_window: 100_000,
                    ..Default::default()
                },
            )
            .await;

        let config = manager.get_config("test-domain-pack").await;
        assert_eq!(config.domain_pack_name, "测试域包");
        assert_eq!(config.context_window, 100_000);
    }

    #[tokio::test]
    async fn test_evaluate_budget_proceed() {
        let manager = DomainPackTokenBudgetManager::new();
        manager
            .register_domain_pack(
                "test",
                DomainPackTokenConfig {
                    context_window: 100_000,
                    compact_threshold_pct: 80,
                    dry_leaf_threshold_pct: 60,
                    ..Default::default()
                },
            )
            .await;

        let decision = manager.evaluate_budget("test", "session-1", 50_000).await;
        assert!(matches!(decision, BudgetDecision::Proceed));
    }

    #[tokio::test]
    async fn test_evaluate_budget_compact_recommended() {
        let manager = DomainPackTokenBudgetManager::new();
        manager
            .register_domain_pack(
                "test",
                DomainPackTokenConfig {
                    context_window: 100_000,
                    compact_threshold_pct: 80,
                    dry_leaf_threshold_pct: 60,
                    ..Default::default()
                },
            )
            .await;

        let decision = manager.evaluate_budget("test", "session-1", 85_000).await;
        assert!(matches!(decision, BudgetDecision::CompactRecommended { .. }));
    }

    #[tokio::test]
    async fn test_evaluate_budget_compact_required() {
        let manager = DomainPackTokenBudgetManager::new();
        manager
            .register_domain_pack(
                "test",
                DomainPackTokenConfig {
                    context_window: 100_000,
                    compact_threshold_pct: 80,
                    dry_leaf_threshold_pct: 60,
                    ..Default::default()
                },
            )
            .await;

        let decision = manager.evaluate_budget("test", "session-1", 96_000).await;
        assert!(matches!(decision, BudgetDecision::CompactRequired { .. }));
    }

    #[tokio::test]
    async fn test_compact_context() {
        let manager = DomainPackTokenBudgetManager::new();
        manager
            .register_domain_pack(
                "test",
                DomainPackTokenConfig {
                    context_window: 100_000,
                    max_history_messages: 3,
                    ..Default::default()
                },
            )
            .await;

        let messages = vec![
            "消息 1: 这是第一条历史消息".to_string(),
            "消息 2: 这是第二条历史消息".to_string(),
            "消息 3: 这是第三条历史消息".to_string(),
            "消息 4: 这是第四条历史消息".to_string(),
            "消息 5: 这是第五条历史消息".to_string(),
            "消息 6: 这是第六条历史消息".to_string(),
        ];

        let result = manager.compact_context("test", "session-1", &messages).await;

        assert!(result.compacted);
        assert!(result.saved_tokens > 0);
        assert_eq!(result.dry_leaf_summaries.len(), 3); // 保留 3 条，压缩 3 条
    }

    #[tokio::test]
    async fn test_dry_leaf_separation() {
        let manager = DomainPackTokenBudgetManager::new();
        manager
            .register_domain_pack(
                "test",
                DomainPackTokenConfig {
                    context_window: 2_000,     // 较小的上下文窗口便于测试
                    dry_leaf_threshold_pct: 1, // 极低阈值以便测试
                    ..Default::default()
                },
            )
            .await;

        // 每条消息约 40-50 字符，7 条共约 300 字符 ≈ 75 tokens
        // 75/2000 = 3.75% > 1% 阈值
        let messages = vec![
            "历史消息 A - 这是一段关于系统架构设计的详细讨论，包括微服务架构、数据库选择"
                .to_string(),
            "历史消息 B - 讨论了API接口设计、数据模型定义、错误处理机制和性能优化".to_string(),
            "历史消息 C - 分析了现有代码库的技术债务，提出了重构方案和测试覆盖提升".to_string(),
            "历史消息 D - 总结了前三个版本的功能迭代过程，记录了关键决策和经验教训".to_string(),
            "活跃消息 1 - 当前任务：实现新的用户认证功能".to_string(),
            "活跃消息 2 - 需要支持OAuth2.0和JWT两种认证方式".to_string(),
            "活跃消息 3 - 需要考虑安全性和性能平衡".to_string(),
        ];

        let (active, leaves) = manager.separate_dry_leaves("test", "session-1", &messages).await;

        assert!(!active.is_empty());
        assert!(!leaves.is_empty());
        assert!(active.len() < messages.len());
    }

    #[tokio::test]
    async fn test_retrieve_dry_leaves() {
        let manager = DomainPackTokenBudgetManager::new();
        manager
            .register_domain_pack(
                "test",
                DomainPackTokenConfig {
                    context_window: 100,       // 非常小的上下文窗口便于测试
                    dry_leaf_threshold_pct: 1, // 极低阈值以便测试
                    ..Default::default()
                },
            )
            .await;

        // 使用至少 4 条消息以确保有历史消息被分离为干叶
        let messages = vec![
            "关于人工智能和机器学习的讨论".to_string(),
            "深度学习模型训练方法".to_string(),
            "数据预处理和特征工程".to_string(),
            "模型架构设计和超参数调优".to_string(),
        ];

        let (_, leaves) = manager.separate_dry_leaves("test", "session-1", &messages).await;

        assert!(!leaves.is_empty());

        let retrieved = manager.retrieve_dry_leaves("test", "人工智能", 5).await;

        assert!(!retrieved.is_empty());
    }

    #[test]
    fn test_generate_brief_summary() {
        let short = "短消息";
        assert_eq!(generate_brief_summary(short), "短消息");

        let long = "a".repeat(200);
        let summary = generate_brief_summary(&long);
        assert!(summary.len() <= 103); // 100 + "..."
    }

    #[test]
    fn test_extract_keywords() {
        let text = "machine learning artificial intelligence deep learning";
        let keywords = extract_keywords(text);
        assert!(!keywords.is_empty());
        assert!(keywords.contains(&"machine".to_string()));
        assert!(keywords.contains(&"learning".to_string()));
    }

    #[tokio::test]
    async fn test_get_domain_pack_stats() {
        let manager = DomainPackTokenBudgetManager::new();
        manager.register_domain_pack("test", DomainPackTokenConfig::default()).await;

        manager.record_usage("test", "session-1", 1000, 500).await;

        let stats = manager.get_domain_pack_stats("test").await;
        assert_eq!(stats.domain_pack_id, "test");
        assert!(stats.total_usage > 0);
    }
}
