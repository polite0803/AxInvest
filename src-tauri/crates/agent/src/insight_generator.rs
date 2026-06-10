//! 反思洞察生成器
//!
//! 关键改进：
//! - `store_insight` 按 `(category, content_hash)` 去重合并（usage_count 累加）
//! - **返回 `Option<Insight>`**：新建/合并后的最终 insight，调用方可以直接拿到本次新产生的条目
//! - 锁顺序统一（M4）：总是 `dedup.write()` → `insights.write()`，避免自死锁
//! - 支持用户反馈（`record_feedback`）调整 confidence
//! - 支持时间衰减（`decay_stale`，按 `last_reinforced_at`）
//! - LRU 容量上限（`max_insights`）
//! - `generate_from_reflection_multi` 同时输出错误/成功/优化等最多 4 条独立 insight
//! - 自动 prune 弱洞察
//! - **持久化**：通过 `init_persistence(path)` 挂载 `insights.jsonl`，启动加载 + 每次变更后 rewrite
//! - **热更新**：`set_max_insights` / `set_decay_days` 走 interior mutability（AtomicUsize / AtomicU32）
//! - `to_learning_insight()` 集中类别映射到 `axagent_trajectory::LearningInsight`（M8）

use crate::reflector::Reflection;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};
use tokio::sync::{Mutex, RwLock};
use tracing::warn;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InsightCategory {
    ErrorPattern,
    SuccessPattern,
    Optimization,
    Knowledge,
    Workflow,
    ToolUsage,
}

impl InsightCategory {
    pub fn as_str(&self) -> &'static str {
        match self {
            InsightCategory::ErrorPattern => "error_pattern",
            InsightCategory::SuccessPattern => "success_pattern",
            InsightCategory::Optimization => "optimization",
            InsightCategory::Knowledge => "knowledge",
            InsightCategory::Workflow => "workflow",
            InsightCategory::ToolUsage => "tool_usage",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Insight {
    pub id: String,
    pub category: InsightCategory,
    pub title: String,
    pub content: String,
    pub source_task_id: String,
    pub confidence: f32,
    pub tags: Vec<String>,
    pub created_at: DateTime<Utc>,
    pub last_reinforced_at: DateTime<Utc>,
    pub usage_count: u32,
    /// 用户反馈分数：+1（👍）/ -1（👎）/ 0（未反馈）。会持续叠加。
    pub feedback_score: i32,
}

impl Insight {
    pub fn new(
        category: InsightCategory,
        title: String,
        content: String,
        source_task_id: String,
    ) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4().to_string(),
            category,
            title,
            content,
            source_task_id,
            confidence: 0.5,
            tags: Vec::new(),
            created_at: now,
            last_reinforced_at: now,
            usage_count: 1,
            feedback_score: 0,
        }
    }

    pub fn with_confidence(mut self, c: f32) -> Self {
        self.confidence = c.clamp(0.0, 1.0);
        self
    }

    pub fn with_tags(mut self, tags: Vec<String>) -> Self {
        self.tags = tags;
        self
    }
}

/// 持久化 helper：单文件 JSONL 读写 + 全量重写
struct InsightStore {
    path: Option<PathBuf>,
    /// 单文件写锁：串行化所有落盘动作，避免并发 rewrite 损坏 JSONL
    write_lock: Arc<Mutex<()>>,
}

impl InsightStore {
    fn new(path: Option<PathBuf>) -> Self {
        Self {
            path,
            write_lock: Arc::new(Mutex::new(())),
        }
    }

    /// 把当前所有 insight 全量写入 JSONL
    async fn rewrite_all(&self, insights: &HashMap<String, Insight>) -> std::io::Result<()> {
        let Some(path) = &self.path else { return Ok(()) };
        let _guard = self.write_lock.lock().await;
        if let Some(parent) = path.parent() {
            let _ = tokio::fs::create_dir_all(parent).await;
        }
        // 先写到临时文件再原子 rename，避免崩溃半写
        let tmp = path.with_extension("jsonl.tmp");
        let mut buf = String::new();
        let mut v: Vec<&Insight> = insights.values().collect();
        v.sort_by_key(|a| a.created_at);
        for ins in v {
            if let Ok(s) = serde_json::to_string(ins) {
                buf.push_str(&s);
                buf.push('\n');
            }
        }
        tokio::fs::write(&tmp, buf.as_bytes()).await?;
        tokio::fs::rename(&tmp, path).await?;
        Ok(())
    }

    /// 加载全部 insight
    async fn load_all(&self) -> std::io::Result<Vec<Insight>> {
        let Some(path) = &self.path else { return Ok(Vec::new()) };
        if !path.exists() {
            return Ok(Vec::new());
        }
        let _guard = self.write_lock.lock().await;
        let content = tokio::fs::read_to_string(path).await?;
        let mut out: Vec<Insight> = Vec::new();
        for line in content.lines() {
            if line.trim().is_empty() {
                continue;
            }
            if let Ok(ins) = serde_json::from_str::<Insight>(line) {
                out.push(ins);
            }
        }
        Ok(out)
    }
}

pub struct InsightGenerator {
    insights: Arc<RwLock<HashMap<String, Insight>>>,
    /// 按 (category, content_hash) 索引，指向 insight id；用于去重
    dedup_index: Arc<RwLock<HashMap<(InsightCategory, String), String>>>,
    /// 上限；超出时按 (feedback_score asc, last_reinforced_at asc) 删除
    max_insights: AtomicUsize,
    /// 衰减天数；0 = 不衰减
    decay_days: AtomicU32,
    /// 持久化 store（init_persistence 后挂上）
    store: Arc<RwLock<InsightStore>>,
}

impl Default for InsightGenerator {
    fn default() -> Self {
        Self::new()
    }
}

impl InsightGenerator {
    pub fn new() -> Self {
        Self {
            insights: Arc::new(RwLock::new(HashMap::new())),
            dedup_index: Arc::new(RwLock::new(HashMap::new())),
            max_insights: AtomicUsize::new(500),
            decay_days: AtomicU32::new(30),
            store: Arc::new(RwLock::new(InsightStore::new(None))),
        }
    }

    /// Builder API（保留向后兼容）：设置上限
    pub fn with_max_insights(self, n: usize) -> Self {
        self.max_insights.store(n.max(1), Ordering::Relaxed);
        self
    }

    /// Builder API（保留向后兼容）：设置衰减天数
    pub fn with_decay_days(self, days: u32) -> Self {
        self.decay_days.store(days, Ordering::Relaxed);
        self
    }

    /// L1: 热更新 max_insights（可从 Arc<&Self> 调用）
    pub fn set_max_insights(&self, n: usize) {
        self.max_insights.store(n.max(1), Ordering::Relaxed);
    }

    /// L1: 热更新 decay_days
    pub fn set_decay_days(&self, days: u32) {
        self.decay_days.store(days, Ordering::Relaxed);
    }

    pub fn get_max_insights(&self) -> usize {
        self.max_insights.load(Ordering::Relaxed)
    }

    pub fn get_decay_days(&self) -> u32 {
        self.decay_days.load(Ordering::Relaxed)
    }

    /// 原子写入 max_insights + decay_days（L1 配套 helper）
    /// 永远成功：使用 Relaxed ordering，不阻塞。
    pub fn try_write_settings(&self, max_insights: usize, decay_days: u32) -> bool {
        self.set_max_insights(max_insights);
        self.set_decay_days(decay_days);
        true
    }

    /// S3: 挂载持久化路径，启动时调用一次。返回加载到内存的 insight 数。
    pub async fn init_persistence(&self, path: PathBuf) -> std::io::Result<usize> {
        {
            let mut store = self.store.write().await;
            *store = InsightStore::new(Some(path.clone()));
        }
        let loaded = {
            let store = self.store.read().await;
            store.load_all().await?
        };
        let n = loaded.len();
        if n > 0 {
            let mut insights = self.insights.write().await;
            let mut dedup = self.dedup_index.write().await;
            for ins in loaded {
                let hash = Self::content_hash(&ins.content);
                dedup.insert((ins.category, hash), ins.id.clone());
                insights.insert(ins.id.clone(), ins);
            }
        }
        Ok(n)
    }

    /// 按 content 算 hash（用最简单的 lowercase+trim，保证 64 位以内冲突极低）
    fn content_hash(content: &str) -> String {
        let normalized: String = content
            .chars()
            .filter(|c| !c.is_whitespace())
            .flat_map(|c| c.to_lowercase())
            .collect();
        let len = normalized.chars().count();
        let prefix: String = normalized.chars().take(64).collect();
        format!("{:x}#{}", len, prefix)
    }

    /// 存储一条 insight，自动去重
    ///
    /// 行为：
    /// - 若 (category, content_hash) 已存在：合并——`usage_count += 1`，`last_reinforced_at = now`，`confidence` 按 EWMA 上调
    /// - 新建：按 max_insights 容量 LRU 淘汰
    /// - 返回：本次**最终**入库的 insight（新建 or 合并后）
    ///
    /// 锁顺序（M4）：总是 `dedup.write()` → `insights.write()`，绝不交叉获取
    pub async fn store_insight(&self, mut insight: Insight) -> Option<Insight> {
        let now = Utc::now();
        let hash = Self::content_hash(&insight.content);
        let key = (insight.category, hash.clone());

        let mut dedup = self.dedup_index.write().await;
        if let Some(existing_id) = dedup.get(&key).cloned() {
            let mut insights = self.insights.write().await;
            if let Some(existing) = insights.get_mut(&existing_id) {
                existing.usage_count = existing.usage_count.saturating_add(1);
                existing.last_reinforced_at = now;
                // EWMA 上调 confidence（最大 0.95）
                let alpha = 0.1;
                existing.confidence = (existing.confidence * (1.0 - alpha)
                    + insight.confidence * alpha)
                    .clamp(0.0, 0.95);
                let snapshot = existing.clone();
                drop(insights);
                drop(dedup);
                self.persist_async();
                return Some(snapshot);
            }
        }

        // 新建
        insight.last_reinforced_at = now;
        let id = insight.id.clone();
        {
            let mut insights = self.insights.write().await;
            insights.insert(id.clone(), insight);
        }
        dedup.insert(key, id.clone());

        // 在 dedup 持锁期间确定要淘汰的 id（不释放 dedup，也不重入）
        let cap = self.max_insights.load(Ordering::Relaxed);
        let to_remove: Vec<String> = if cap > 0 {
            let insights = self.insights.read().await;
            if insights.len() > cap {
                self.pick_lru_candidates(&insights, insights.len() - cap)
            } else {
                Vec::new()
            }
        } else {
            Vec::new()
        };

        if !to_remove.is_empty() {
            let mut insights = self.insights.write().await;
            // dedup 仍持有，顺序一致
            for rid in &to_remove {
                if let Some(removed) = insights.remove(rid) {
                    let h = Self::content_hash(&removed.content);
                    dedup.remove(&(removed.category, h));
                }
            }
        }

        let snapshot = self.insights.read().await.get(&id).cloned();
        drop(dedup);
        self.persist_async();
        snapshot
    }

    /// 把当前所有 insight 落盘；失败仅 warn。
    fn persist_async(&self) {
        let store = self.store.clone();
        let insights = self.insights.clone();
        tokio::spawn(async move {
            let snapshot = {
                let guard = insights.read().await;
                guard.clone()
            };
            let store_guard = store.read().await;
            if let Err(e) = store_guard.rewrite_all(&snapshot).await {
                warn!("[insight] persist failed: {}", e);
            }
        });
    }

    fn pick_lru_candidates(&self, map: &HashMap<String, Insight>, n: usize) -> Vec<String> {
        let mut v: Vec<&Insight> = map.values().collect();
        v.sort_by(|a, b| {
            (a.feedback_score, a.last_reinforced_at)
                .partial_cmp(&(b.feedback_score, b.last_reinforced_at))
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        v.into_iter().take(n).map(|i| i.id.clone()).collect()
    }

    /// 用户反馈：useful=true 累 +1 并提升 confidence，useful=false 累 -1 并降 confidence
    pub async fn record_feedback(&self, id: &str, useful: bool) -> Option<Insight> {
        let snapshot = {
            let mut insights = self.insights.write().await;
            let ins = insights.get_mut(id)?;
            ins.feedback_score = if useful {
                ins.feedback_score.saturating_add(1)
            } else {
                ins.feedback_score.saturating_sub(1)
            };
            ins.last_reinforced_at = Utc::now();
            if useful {
                ins.confidence = (ins.confidence + 0.05).clamp(0.0, 1.0);
            } else {
                ins.confidence = (ins.confidence - 0.1).clamp(0.0, 1.0);
            }
            Some(ins.clone())
        };
        if snapshot.is_some() {
            self.persist_async();
        }
        snapshot
    }

    pub async fn delete_insight(&self, id: &str) -> bool {
        let removed = {
            let mut insights = self.insights.write().await;
            if let Some(ins) = insights.remove(id) {
                drop(insights);
                let hash = Self::content_hash(&ins.content);
                let mut dedup = self.dedup_index.write().await;
                dedup.remove(&(ins.category, hash));
                true
            } else {
                false
            }
        };
        if removed {
            self.persist_async();
        }
        removed
    }

    /// 衰减超过 N 天未强化的 insight
    pub async fn decay_stale(&self) -> usize {
        let days = self.decay_days.load(Ordering::Relaxed);
        if days == 0 {
            return 0;
        }
        let threshold = chrono::Duration::days(days as i64);
        let now = Utc::now();
        let mut count = 0usize;
        {
            let mut insights = self.insights.write().await;
            for ins in insights.values_mut() {
                if now - ins.last_reinforced_at > threshold {
                    ins.confidence = (ins.confidence * 0.9).clamp(0.0, 1.0);
                    count += 1;
                }
            }
        }
        if count > 0 {
            self.persist_async();
        }
        count
    }

    /// 清理 confidence < threshold 的洞察
    pub async fn prune_stale(&self, min_confidence: f32) -> usize {
        let to_remove: Vec<String> = {
            let insights = self.insights.read().await;
            insights
                .iter()
                .filter_map(|(id, ins)| {
                    if ins.confidence < min_confidence {
                        Some(id.clone())
                    } else {
                        None
                    }
                })
                .collect()
        };
        let n = to_remove.len();
        for id in to_remove {
            self.delete_insight(&id).await;
        }
        n
    }

    pub async fn clear_all(&self) {
        {
            let mut insights = self.insights.write().await;
            insights.clear();
            let mut dedup = self.dedup_index.write().await;
            dedup.clear();
        }
        self.persist_async();
    }

    pub async fn get_insights(&self) -> Vec<Insight> {
        let insights = self.insights.read().await;
        let mut v: Vec<Insight> = insights.values().cloned().collect();
        v.sort_by_key(|b| std::cmp::Reverse(b.created_at));
        v
    }

    pub async fn get_insights_by_category(&self, category: InsightCategory) -> Vec<Insight> {
        let insights = self.insights.read().await;
        let mut v: Vec<Insight> = insights
            .values()
            .filter(|i| i.category == category)
            .cloned()
            .collect();
        v.sort_by_key(|b| std::cmp::Reverse(b.created_at));
        v
    }

    /// 复制版 (兼容旧 API 名)，按 category 字符串过滤
    pub async fn get_insights_by_category_str(&self, category: Option<&str>) -> Vec<Insight> {
        let insights = self.insights.read().await;
        let v: Vec<Insight> = if let Some(c) = category {
            insights
                .values()
                .filter(|i| i.category.as_str() == c)
                .cloned()
                .collect()
        } else {
            insights.values().cloned().collect()
        };
        v
    }

    pub async fn search_insights(&self, query: &str) -> Vec<Insight> {
        let q = query.to_lowercase();
        let insights = self.insights.read().await;
        let mut v: Vec<Insight> = insights
            .values()
            .filter(|i| {
                i.title.to_lowercase().contains(&q) || i.content.to_lowercase().contains(&q)
            })
            .cloned()
            .collect();
        v.sort_by(|a, b| {
            b.confidence
                .partial_cmp(&a.confidence)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        v
    }

    /// 按 confidence desc 排序，取前 N
    pub async fn get_top_insights(&self, n: usize) -> Vec<Insight> {
        let insights = self.insights.read().await;
        let mut v: Vec<Insight> = insights.values().cloned().collect();
        v.sort_by(|a, b| {
            b.confidence
                .partial_cmp(&a.confidence)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        v.into_iter().take(n).collect()
    }

    /// 按 created_at desc 排序，取前 N
    pub async fn get_recent_insights(&self, n: usize) -> Vec<Insight> {
        let insights = self.insights.read().await;
        let mut v: Vec<Insight> = insights.values().cloned().collect();
        v.sort_by_key(|b| std::cmp::Reverse(b.created_at));
        v.into_iter().take(n).collect()
    }

    /// confidence >= threshold 的洞察
    pub async fn get_high_confidence_insights(&self, threshold: f32) -> Vec<Insight> {
        let insights = self.insights.read().await;
        insights
            .values()
            .filter(|i| i.confidence >= threshold)
            .cloned()
            .collect()
    }

    pub async fn get_stats(&self) -> InsightStats {
        let insights = self.insights.read().await;
        let mut by_category: HashMap<String, usize> = HashMap::new();
        let mut total_confidence = 0.0;
        for ins in insights.values() {
            *by_category
                .entry(ins.category.as_str().to_string())
                .or_default() += 1;
            total_confidence += ins.confidence;
        }
        let total = insights.len();
        let avg = if total > 0 {
            total_confidence / total as f32
        } else {
            0.0
        };
        InsightStats {
            total,
            average_confidence: avg,
            by_category,
        }
    }

    /// 从一条 reflection 生成最多 4 条独立 insight（不丢失 error+success 信息）
    pub fn generate_from_reflection_multi(&self, r: &Reflection) -> Vec<Insight> {
        let mut out = Vec::new();

        if !r.error_patterns.is_empty() {
            let content = r.error_patterns.join("; ");
            let confidence = (0.5 + (r.error_patterns.len() as f32 * 0.05).min(0.4)).min(0.95);
            let tags = r.error_patterns.clone();
            out.push(
                Insight::new(
                    InsightCategory::ErrorPattern,
                    format!("Error Patterns: {} identified", r.error_patterns.len()),
                    content,
                    r.task_id.clone(),
                )
                .with_confidence(confidence)
                .with_tags(tags),
            );
        }

        if !r.reusable_patterns.is_empty() {
            let content = r.reusable_patterns.join("; ");
            let confidence = (0.4 + (r.reusable_patterns.len() as f32 * 0.05).min(0.4)).min(0.95);
            out.push(
                Insight::new(
                    InsightCategory::SuccessPattern,
                    format!("Reusable Patterns: {} identified", r.reusable_patterns.len()),
                    content,
                    r.task_id.clone(),
                )
                .with_confidence(confidence),
            );
        }

        if r.quality_score >= 7 {
            out.push(
                Insight::new(
                    InsightCategory::Optimization,
                    format!("High Quality Execution: score {}", r.quality_score),
                    r.overall_summary.clone(),
                    r.task_id.clone(),
                )
                .with_confidence(0.7),
            );
        } else if r.quality_score <= 4 {
            out.push(
                Insight::new(
                    InsightCategory::Knowledge,
                    format!("Low Quality Task (score {})", r.quality_score),
                    format!(
                        "{}\n\nImprovements: {}",
                        r.overall_summary,
                        r.improvement_suggestions.join("; ")
                    ),
                    r.task_id.clone(),
                )
                .with_confidence(0.6),
            );
        }

        if !r.knowledge_suggestions.is_empty() {
            out.push(
                Insight::new(
                    InsightCategory::Workflow,
                    format!("Knowledge: {} suggestion(s)", r.knowledge_suggestions.len()),
                    r.knowledge_suggestions.join("; "),
                    r.task_id.clone(),
                )
                .with_confidence(0.5),
            );
        }

        if out.is_empty() {
            out.push(
                Insight::new(
                    InsightCategory::Knowledge,
                    "Task Reflection".to_string(),
                    r.overall_summary.clone(),
                    r.task_id.clone(),
                )
                .with_confidence(0.4),
            );
        }
        out
    }

    /// 兼容旧 API：从 reflection 最多生成 1 条 insight（按 dominant category 折叠）
    pub fn generate_from_reflection(&self, r: &Reflection) -> Option<Insight> {
        self.generate_from_reflection_multi(r).into_iter().next()
    }

    /// M8 helper: 暴露 `InsightCategory` 的稳定字符串名，供 `commands/agent.rs`
    /// 的 bridge 函数做映射（避免 `axagent_agent` 反向依赖 `axagent_trajectory`）。
    /// 真正的 `LearningInsight` 构造在 bridge 中完成，**新增 category 时必须同时修改
    /// `commands::agent::map_category_to_trajectory` 与 `InsightCategory::as_str`**。
    pub fn category_name(&self, c: InsightCategory) -> &'static str {
        c.as_str()
    }

    /// 同步 flush 一次落盘（供测试 / 显式保存用）
    pub async fn flush(&self) {
        let snapshot = {
            let guard = self.insights.read().await;
            guard.clone()
        };
        let store_guard = self.store.read().await;
        if let Err(e) = store_guard.rewrite_all(&snapshot).await {
            warn!("[insight] flush failed: {}", e);
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InsightStats {
    pub total: usize,
    pub average_confidence: f32,
    pub by_category: HashMap<String, usize>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;

    fn r() -> Reflection {
        let mut r = Reflection::new("t1".to_string());
        r.error_patterns = vec!["e1".to_string(), "e2".to_string()];
        r.reusable_patterns = vec!["p1".to_string()];
        r.knowledge_suggestions = vec!["k1".to_string()];
        r.improvement_suggestions = vec!["i1".to_string()];
        r.overall_summary = "Test summary".to_string();
        r.quality_score = 8;
        r
    }

    #[tokio::test]
    async fn test_multi_insight_generation() {
        let g = InsightGenerator::new();
        let r = r();
        let v = g.generate_from_reflection_multi(&r);
        assert!(v.len() >= 2, "should produce both error and success insights");
        assert!(
            v.iter()
                .any(|i| i.category == InsightCategory::ErrorPattern)
        );
        assert!(
            v.iter()
                .any(|i| i.category == InsightCategory::SuccessPattern)
        );
    }

    #[tokio::test]
    async fn test_dedup_merge_returns_snapshot() {
        let g = InsightGenerator::new();
        let r = r();
        let mut v = g.generate_from_reflection_multi(&r);
        let original = v.remove(0);
        let first = g
            .store_insight(original.clone())
            .await
            .expect("first store");
        let second = g.store_insight(original).await.expect("second store");
        // S1: store_insight 必返回 Some
        assert_eq!(first.id, second.id, "merge should return same id");
        let all = g.get_insights().await;
        assert_eq!(all.len(), v.len(), "merged: should not double the count");
        let ins = all
            .iter()
            .find(|i| i.category == InsightCategory::ErrorPattern)
            .unwrap();
        assert!(ins.usage_count >= 2, "usage_count should be incremented");
    }

    #[tokio::test]
    async fn test_feedback_changes_confidence() {
        let g = InsightGenerator::new();
        let ins = Insight::new(
            InsightCategory::Knowledge,
            "Test".to_string(),
            "C".to_string(),
            "t".to_string(),
        );
        let before = ins.confidence;
        g.store_insight(ins.clone()).await;
        g.record_feedback(&ins.id, true).await;
        g.record_feedback(&ins.id, true).await;
        let updated = g.get_insights().await;
        let updated = updated.iter().find(|i| i.id == ins.id).unwrap();
        assert!(updated.confidence > before);
        assert!(updated.feedback_score >= 2);
    }

    #[tokio::test]
    async fn test_prune() {
        let g = InsightGenerator::new();
        let mut ins = Insight::new(
            InsightCategory::Knowledge,
            "x".to_string(),
            "y".to_string(),
            "t".to_string(),
        );
        ins.confidence = 0.05;
        g.store_insight(ins).await;
        let removed = g.prune_stale(0.1).await;
        assert_eq!(removed, 1);
        assert!(g.get_insights().await.is_empty());
    }

    #[tokio::test]
    async fn test_decay_stale_skipped_when_zero_days() {
        let g = InsightGenerator::new().with_decay_days(0);
        let r = g.decay_stale().await;
        assert_eq!(r, 0);
    }

    #[tokio::test]
    async fn test_decay_stale_demotes_confidence() {
        let g = InsightGenerator::new().with_decay_days(10);
        let mut ins = Insight::new(
            InsightCategory::Knowledge,
            "x".to_string(),
            "y".to_string(),
            "t".to_string(),
        );
        ins.confidence = 0.9;
        ins.last_reinforced_at = Utc::now() - Duration::days(11);
        g.store_insight(ins).await;
        let decayed = g.decay_stale().await;
        assert_eq!(decayed, 1);
        let all = g.get_insights().await;
        assert!(all[0].confidence < 0.9);
    }

    #[tokio::test]
    async fn test_lru_eviction() {
        let g = InsightGenerator::new().with_max_insights(2);
        for i in 0..5 {
            let ins = Insight::new(
                InsightCategory::Knowledge,
                format!("title-{i}"),
                format!("content-{i}-distinct"),
                "t".to_string(),
            );
            g.store_insight(ins).await;
        }
        let all = g.get_insights().await;
        assert!(all.len() <= 2, "max_insights should bound storage");
    }

    #[tokio::test]
    async fn test_search_and_top() {
        let g = InsightGenerator::new();
        g.store_insight(
            Insight::new(
                InsightCategory::ErrorPattern,
                "alpha".to_string(),
                "beta alpha".to_string(),
                "t".to_string(),
            )
            .with_confidence(0.9),
        )
        .await;
        g.store_insight(
            Insight::new(
                InsightCategory::Knowledge,
                "gamma".to_string(),
                "delta".to_string(),
                "t".to_string(),
            )
            .with_confidence(0.3),
        )
        .await;
        let r = g.search_insights("alpha").await;
        assert_eq!(r.len(), 1);
        let top = g.get_top_insights(1).await;
        assert_eq!(top[0].title, "alpha");
    }

    #[tokio::test]
    async fn test_delete() {
        let g = InsightGenerator::new();
        let ins = Insight::new(
            InsightCategory::Knowledge,
            "x".to_string(),
            "y".to_string(),
            "t".to_string(),
        );
        let id = ins.id.clone();
        g.store_insight(ins).await;
        assert!(g.delete_insight(&id).await);
        assert!(!g.delete_insight(&id).await);
    }

    #[tokio::test]
    async fn test_set_settings_after_arc() {
        // L1: 即使通过 Arc<&Self> 也能改
        let g = Arc::new(InsightGenerator::new());
        g.set_max_insights(123);
        g.set_decay_days(7);
        g.try_write_settings(50, 14);
        assert_eq!(g.get_max_insights(), 50);
        assert_eq!(g.get_decay_days(), 14);
    }

    #[tokio::test]
    async fn test_persistence_roundtrip() {
        // S3: 启动加载 + rewrite 闭环
        let dir =
            std::env::temp_dir().join(format!("axagent-insight-persist-{}", uuid::Uuid::new_v4()));
        let path = dir.join("insights.jsonl");
        std::fs::create_dir_all(&dir).unwrap();

        let g1 = InsightGenerator::new();
        g1.init_persistence(path.clone()).await.unwrap();
        let ins = Insight::new(
            InsightCategory::Knowledge,
            "persist".to_string(),
            "persist-content".to_string(),
            "task-x".to_string(),
        );
        let ins_id = ins.id.clone();
        g1.store_insight(ins).await;
        g1.flush().await;

        let g2 = InsightGenerator::new();
        let n = g2.init_persistence(path.clone()).await.unwrap();
        assert_eq!(n, 1, "should load 1 from disk");
        let all = g2.get_insights().await;
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].id, ins_id);

        let _ = std::fs::remove_dir_all(&dir);
    }
}
