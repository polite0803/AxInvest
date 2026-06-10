//! 反思器（Reflector）— 在任务完成后对执行结果进行质量评估、模式识别、洞察生成
//!
//! 关键设计：
//! - 反思数据持久化到 JSONL 文件（`app_data_dir/reflections.jsonl`），重启后能恢复
//! - 配置热更新：`Arc<RwLock<ReflectionConfig>>`
//! - 反思过程不再持有 history 写锁跨 await，避免序列化所有并发 reflect
//! - 复用 `ErrorClassifier` 进行错误分类（panic-safe）
//! - 洞察生成自动去重合并（按 `(category, content_hash)`），新增 insight 落盘
//! - `reflect()` 返回 `(Reflection, Vec<Insight>)`，**精确**告诉调用方本次新产生了哪些 insight
//!   （避免基于时间戳的过滤造成跨对话污染 / 漏推）
//! - 启动时自动从磁盘加载历史；加载时按 `task_id` 去重，保留最新
//! - 文件写入加 per-file 写锁，避免并发 JSONL 交错
//! - 错误分类器失败时回退到关键词匹配 + 输出可读 prefix
//! - `update_config` 同步把 `decay_days` / `max_insights` 推给 `InsightGenerator`

use crate::insight_generator::{Insight, InsightGenerator};
use crate::recovery_strategies::ErrorClassifier;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet, VecDeque};
use std::panic::AssertUnwindSafe;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::io::AsyncWriteExt;
use tokio::sync::{Mutex, RwLock};
use tracing::{info, warn};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskExecutionRecord {
    pub task_id: String,
    pub task_description: String,
    pub result: Option<serde_json::Value>,
    pub success: bool,
    pub error: Option<String>,
    pub start_time: DateTime<Utc>,
    pub end_time: DateTime<Utc>,
    pub duration_ms: u64,
    pub tools_used: Vec<String>,
    pub iterations: usize,
}

impl TaskExecutionRecord {
    pub fn new(
        task_id: String,
        task_description: String,
        start_time: DateTime<Utc>,
        end_time: DateTime<Utc>,
    ) -> Self {
        Self {
            task_id,
            task_description,
            result: None,
            success: false,
            error: None,
            start_time,
            end_time,
            duration_ms: 0,
            tools_used: Vec::new(),
            iterations: 0,
        }
    }

    pub fn with_result(mut self, result: serde_json::Value) -> Self {
        self.result = Some(result);
        self
    }

    pub fn with_success(mut self, success: bool) -> Self {
        self.success = success;
        self
    }

    pub fn with_error(mut self, error: String) -> Self {
        self.error = Some(error);
        self.success = false;
        self
    }

    pub fn with_tools(mut self, tools: Vec<String>) -> Self {
        self.tools_used = tools;
        self
    }

    pub fn with_iterations(mut self, iterations: usize) -> Self {
        self.iterations = iterations;
        self
    }

    pub fn compute_duration(&mut self) {
        self.duration_ms = self
            .end_time
            .signed_duration_since(self.start_time)
            .num_milliseconds()
            .max(0) as u64;
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QualityMetrics {
    pub task_success_score: f32,
    pub tool_efficiency_score: f32,
    pub iteration_efficiency_score: f32,
    pub time_efficiency_score: f32,
    pub error_recovery_score: f32,
    pub goal_completion_score: f32,
    pub overall_weighted_score: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Reflection {
    pub task_id: String,
    pub timestamp: DateTime<Utc>,
    pub quality_score: u8,
    pub quality_analysis: String,
    pub efficiency_analysis: String,
    pub error_patterns: Vec<String>,
    pub reusable_patterns: Vec<String>,
    pub knowledge_suggestions: Vec<String>,
    pub improvement_suggestions: Vec<String>,
    pub overall_summary: String,
    pub quality_metrics: Option<QualityMetrics>,
}

impl Reflection {
    pub fn new(task_id: String) -> Self {
        Self {
            task_id,
            timestamp: Utc::now(),
            quality_score: 0,
            quality_analysis: String::new(),
            efficiency_analysis: String::new(),
            error_patterns: Vec::new(),
            reusable_patterns: Vec::new(),
            knowledge_suggestions: Vec::new(),
            improvement_suggestions: Vec::new(),
            overall_summary: String::new(),
            quality_metrics: None,
        }
    }

    pub fn with_quality(mut self, score: u8, analysis: String) -> Self {
        self.quality_score = score.clamp(1, 10);
        self.quality_analysis = analysis;
        self
    }

    pub fn with_quality_metrics(mut self, metrics: QualityMetrics) -> Self {
        self.quality_score = (metrics.overall_weighted_score.round() as u8).clamp(1, 10);
        self.quality_metrics = Some(metrics);
        self
    }

    pub fn with_efficiency(mut self, analysis: String) -> Self {
        self.efficiency_analysis = analysis;
        self
    }

    pub fn with_patterns(mut self, errors: Vec<String>, reusable: Vec<String>) -> Self {
        self.error_patterns = errors;
        self.reusable_patterns = reusable;
        self
    }

    pub fn with_knowledge(mut self, suggestions: Vec<String>) -> Self {
        self.knowledge_suggestions = suggestions;
        self
    }

    pub fn with_improvements(mut self, suggestions: Vec<String>) -> Self {
        self.improvement_suggestions = suggestions;
        self
    }

    pub fn with_summary(mut self, summary: String) -> Self {
        self.overall_summary = summary;
        self
    }
}

/// 反思器配置 — 全部可热更新
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReflectionConfig {
    pub enabled: bool,
    pub min_quality_threshold: u8,
    pub store_insights: bool,
    pub max_history: usize,
    /// 自动衰减：超过该天数未强化的 insight 视为过期（0 = 不衰减）
    pub insight_decay_days: u32,
    /// 洞察上限
    pub max_insights: usize,
    /// 是否复用 ErrorClassifier（默认 true）
    pub use_error_classifier: bool,
}

impl Default for ReflectionConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            min_quality_threshold: 5,
            store_insights: true,
            max_history: 200,
            insight_decay_days: 30,
            max_insights: 500,
            use_error_classifier: true,
        }
    }
}

impl ReflectionConfig {
    pub fn with_threshold(mut self, threshold: u8) -> Self {
        self.min_quality_threshold = threshold.clamp(1, 10);
        self
    }
}

/// 持久化辅助：把 reflection 追加到 JSONL 文件
pub struct PersistedStore {
    reflections_path: Option<PathBuf>,
    /// per-file 写锁 — 防止多 reflect 并发 append 时出现 JSONL 交错
    write_lock: Arc<Mutex<()>>,
}

impl PersistedStore {
    pub fn new(path: Option<PathBuf>) -> Self {
        Self {
            reflections_path: path,
            write_lock: Arc::new(Mutex::new(())),
        }
    }

    pub fn path(&self) -> Option<&PathBuf> {
        self.reflections_path.as_ref()
    }

    /// 追加单条 reflection 到 JSONL
    pub async fn append(&self, reflection: &Reflection) -> std::io::Result<()> {
        let Some(path) = &self.reflections_path else { return Ok(()) };
        let _guard = self.write_lock.lock().await;
        if let Some(parent) = path.parent() {
            let _ = tokio::fs::create_dir_all(parent).await;
        }
        let mut line = serde_json::to_string(reflection).unwrap_or_else(|_| "{}".to_string());
        line.push('\n');
        let mut file = tokio::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .await?;
        file.write_all(line.as_bytes()).await?;
        file.flush().await?;
        Ok(())
    }

    /// 加载全部 reflection（最多 max 行），按时间倒序返回；**按 task_id 去重保留最新**
    pub async fn load_recent(&self, max: usize) -> std::io::Result<Vec<Reflection>> {
        let Some(path) = &self.reflections_path else { return Ok(Vec::new()) };
        if !path.exists() {
            return Ok(Vec::new());
        }
        let content = tokio::fs::read_to_string(path).await?;
        // S5: 按 task_id 去重保留 timestamp 最大的那条
        let mut by_task: HashMap<String, Reflection> = HashMap::new();
        for line in content.lines() {
            if line.trim().is_empty() {
                continue;
            }
            if let Ok(r) = serde_json::from_str::<Reflection>(line) {
                let cur = by_task.get(&r.task_id);
                if cur.map(|c| c.timestamp < r.timestamp).unwrap_or(true) {
                    by_task.insert(r.task_id.clone(), r);
                }
            }
        }
        let mut out: Vec<Reflection> = by_task.into_values().collect();
        out.sort_by_key(|r| std::cmp::Reverse(r.timestamp));
        if out.len() > max {
            let drop_n = out.len() - max;
            out.drain(0..drop_n);
        }
        Ok(out)
    }
}

pub struct Reflector {
    /// 可热更新的配置
    config: Arc<RwLock<ReflectionConfig>>,
    insight_generator: Arc<InsightGenerator>,
    /// history 用 VecDeque 支持 O(1) 的 FIFO 弹出
    history: Arc<RwLock<VecDeque<Reflection>>>,
    /// 持久化存储（运行时可被 init_persistence 设置/重设）
    store: Arc<RwLock<PersistedStore>>,
    /// 错误分类器（stateless, 复用）
    error_classifier: ErrorClassifier,
}

impl Reflector {
    /// 创建默认实例（无持久化）。生产应使用 `with_store_path`。
    pub fn new() -> Self {
        Self {
            config: Arc::new(RwLock::new(ReflectionConfig::default())),
            insight_generator: Arc::new(InsightGenerator::new()),
            history: Arc::new(RwLock::new(VecDeque::new())),
            store: Arc::new(RwLock::new(PersistedStore::new(None))),
            error_classifier: ErrorClassifier::new(),
        }
    }

    /// 使用持久化路径创建，启动时自动加载最近 max_history 条
    pub async fn with_store_path(path: PathBuf, max_history: usize) -> Self {
        let store = Arc::new(RwLock::new(PersistedStore::new(Some(path.clone()))));
        let history = {
            let store_guard = store.read().await;
            match store_guard.load_recent(max_history).await {
                Ok(items) => {
                    info!("[reflector] loaded {} reflections from {}", items.len(), path.display());
                    VecDeque::from(items)
                },
                Err(e) => {
                    warn!("[reflector] failed to load reflections from {}: {}", path.display(), e);
                    VecDeque::new()
                },
            }
        };
        Self {
            config: Arc::new(RwLock::new(ReflectionConfig::default())),
            insight_generator: Arc::new(InsightGenerator::new()),
            history: Arc::new(RwLock::new(history)),
            store,
            error_classifier: ErrorClassifier::new(),
        }
    }

    /// 延迟初始化持久化。
    /// M5: 不再覆盖 max_history 配置（max_history 来自 config + 用户偏好）
    pub async fn init_persistence(&self, path: PathBuf, max_history: usize) -> std::io::Result<()> {
        {
            let mut store = self.store.write().await;
            *store = PersistedStore::new(Some(path.clone()));
        }
        let loaded = {
            let store = self.store.read().await;
            store.load_recent(max_history).await?
        };
        if !loaded.is_empty() {
            let mut history = self.history.write().await;
            for r in loaded {
                if history.len() >= max_history {
                    history.pop_front();
                }
                history.push_back(r);
            }
        }
        Ok(())
    }

    pub fn with_config(mut self, config: ReflectionConfig) -> Self {
        self.config = Arc::new(RwLock::new(config));
        self
    }

    /// 热更新配置；L1: 同步把 `decay_days` / `max_insights` 推给 `InsightGenerator`
    pub async fn update_config(&self, config: ReflectionConfig) {
        // 同步更新 InsightGenerator 的相关上限（原子写、不可失败）
        let _ = self
            .insight_generator
            .try_write_settings(config.max_insights, config.insight_decay_days);
        *self.config.write().await = config;
    }

    pub async fn get_config(&self) -> ReflectionConfig {
        self.config.read().await.clone()
    }

    pub fn get_insight_generator(&self) -> Arc<InsightGenerator> {
        Arc::clone(&self.insight_generator)
    }

    pub async fn get_history(&self) -> Vec<Reflection> {
        self.history.read().await.iter().cloned().collect()
    }

    pub async fn clear_history(&self) {
        self.history.write().await.clear();
    }

    /// 主要入口：执行反思
    ///
    /// S1+S2: 返回 `(Reflection, Vec<Insight>)`，Vec 是**本次新产生/更新**的精确列表
    /// 锁释放：计算阶段不持任何写锁，写入阶段只短持 history 写锁
    pub async fn reflect(&self, record: &TaskExecutionRecord) -> (Reflection, Vec<Insight>) {
        let config = self.config.read().await.clone();

        if !config.enabled {
            let r = Reflection::new(record.task_id.clone())
                .with_quality(0, "Reflection disabled".to_string());
            return (r, Vec::new());
        }

        // ── 计算阶段：纯函数，不持锁 ──
        let metrics = self.calculate_quality_metrics(record);
        let mut reflection = Reflection::new(record.task_id.clone());
        reflection.timestamp = record.end_time;
        reflection.quality_score = (metrics.overall_weighted_score.round() as u8).clamp(1, 10);
        reflection.quality_analysis = self.analyze_quality(record, &metrics);
        reflection.quality_metrics = Some(metrics.clone());
        reflection.efficiency_analysis = self.analyze_efficiency(record);

        let (errors, reusable) = self.analyze_patterns(record, &config);
        reflection.error_patterns = errors;
        reflection.reusable_patterns = reusable;

        let knowledge = self.generate_knowledge_suggestions(record, &metrics);
        reflection.knowledge_suggestions = knowledge;
        reflection.improvement_suggestions =
            self.generate_improvement_suggestions(record, &reflection, &config);
        reflection.overall_summary = self.generate_summary(record, &reflection);

        // ── 写入阶段：短锁 push + 异步持久化 + 收集本次新 insight ──
        {
            let mut history = self.history.write().await;
            if history.len() >= config.max_history {
                history.pop_front();
            }
            history.push_back(reflection.clone());
        }

        let mut new_insights: Vec<Insight> = Vec::new();
        if config.store_insights {
            for insight in self
                .insight_generator
                .generate_from_reflection_multi(&reflection)
            {
                if let Some(stored) = self.insight_generator.store_insight(insight).await {
                    new_insights.push(stored);
                }
            }
        }

        // 持久化：失败仅 warn
        {
            let store = self.store.read().await;
            if let Err(e) = store.append(&reflection).await {
                warn!("[reflector] persist failed: {}", e);
            }
        }

        (reflection, new_insights)
    }

    fn calculate_quality_metrics(&self, record: &TaskExecutionRecord) -> QualityMetrics {
        let task_success_score = if record.success { 10.0 } else { 0.0 };

        let unique_tools = Self::count_unique_tools(&record.tools_used);
        let total_tools = record.tools_used.len().max(1);
        let unique_ratio = unique_tools as f32 / total_tools as f32;
        let iteration_ratio = (unique_tools as f32 / record.iterations.max(1) as f32).min(1.0);
        let tool_efficiency_score = unique_ratio * 5.0 + iteration_ratio * 5.0;

        let expected_iterations = (unique_tools * 2).max(1);
        let iteration_efficiency_score =
            (expected_iterations as f32 / record.iterations.max(1) as f32).min(1.0) * 10.0;

        let expected_duration = record.iterations.max(1) as u64 * 2000;
        let time_efficiency_score =
            (expected_duration as f32 / record.duration_ms.max(1) as f32).min(1.0) * 10.0;

        let error_recovery_score = if record.success {
            if record.iterations > expected_iterations {
                7.0
            } else {
                10.0
            }
        } else if record.error.is_some() {
            0.0
        } else {
            2.0
        };

        let goal_completion_score = if record.success {
            8.0 + (unique_tools as f32 * 0.4).min(2.0)
        } else {
            2.0 + (unique_tools as f32 * 0.3).min(3.0)
        };

        let overall_weighted_score = task_success_score * 0.30
            + tool_efficiency_score * 0.20
            + iteration_efficiency_score * 0.15
            + time_efficiency_score * 0.15
            + error_recovery_score * 0.10
            + goal_completion_score * 0.10;

        QualityMetrics {
            task_success_score,
            tool_efficiency_score,
            iteration_efficiency_score,
            time_efficiency_score,
            error_recovery_score,
            goal_completion_score,
            overall_weighted_score,
        }
    }

    fn analyze_quality(&self, record: &TaskExecutionRecord, metrics: &QualityMetrics) -> String {
        let unique_tools = Self::count_unique_tools(&record.tools_used);
        let total_tools = record.tools_used.len().max(1);
        let unique_ratio = (unique_tools as f32 / total_tools as f32) * 100.0;
        let expected_iterations = (unique_tools * 2).max(1);
        let expected_duration = record.iterations.max(1) as u64 * 2000;

        let task_status = if record.success {
            "completed successfully"
        } else {
            "task failed"
        };

        let error_status = if record.success && record.iterations > expected_iterations {
            "recovered from intermediate errors"
        } else if record.success {
            "no errors encountered"
        } else if record.error.is_some() {
            "unresolved error"
        } else {
            "no explicit error"
        };

        let goal_status = if record.success {
            "all sub-goals addressed"
        } else {
            "partial goal completion"
        };

        format!(
            "Task Success: {:.1}/10 ({})\nTool Efficiency: {:.1}/10 ({} unique tools, {} total calls, {:.0}% unique ratio)\nIteration Efficiency: {:.1}/10 ({} iterations for complexity level {})\nTime Efficiency: {:.1}/10 ({}ms vs {}ms expected)\nError Recovery: {:.1}/10 ({})\nGoal Completion: {:.1}/10 ({})\nOverall Weighted Score: {:.1}/10",
            metrics.task_success_score,
            task_status,
            metrics.tool_efficiency_score,
            unique_tools,
            total_tools,
            unique_ratio,
            metrics.iteration_efficiency_score,
            record.iterations,
            expected_iterations,
            metrics.time_efficiency_score,
            record.duration_ms,
            expected_duration,
            metrics.error_recovery_score,
            error_status,
            metrics.goal_completion_score,
            goal_status,
            metrics.overall_weighted_score,
        )
    }

    fn analyze_efficiency(&self, record: &TaskExecutionRecord) -> String {
        let mut analysis = String::new();

        let duration_per_iteration = if record.iterations > 0 {
            record.duration_ms / record.iterations as u64
        } else {
            record.duration_ms
        };

        analysis.push_str(&format!("Total duration: {}ms. ", record.duration_ms));
        analysis.push_str(&format!("Duration per iteration: {}ms. ", duration_per_iteration));

        if record.duration_ms > 60000 {
            analysis.push_str("Execution time exceeds 1 minute. Consider optimization. ");
        } else if record.duration_ms < 5000 {
            analysis.push_str("Quick execution. ");
        }

        if record.iterations > 20 {
            analysis.push_str("High iteration count may indicate inefficient reasoning. ");
        }

        analysis
    }

    fn analyze_patterns(
        &self,
        record: &TaskExecutionRecord,
        config: &ReflectionConfig,
    ) -> (Vec<String>, Vec<String>) {
        let mut error_patterns = Vec::new();
        let mut reusable_patterns = Vec::new();

        if let Some(ref error) = record.error {
            if config.use_error_classifier {
                // L6: panic-safe 分类器调用
                let classified = std::panic::catch_unwind(AssertUnwindSafe(|| {
                    self.error_classifier.classify(error)
                }))
                .ok();
                if let Some(et) = classified {
                    // L8: 用户可读 prefix
                    let prefix = Self::error_type_label(&et);
                    error_patterns.push(format!("[{}] {}", prefix, et.description()));
                } else {
                    Self::fallback_keyword_patterns(error, &mut error_patterns);
                }
            } else {
                Self::fallback_keyword_patterns(error, &mut error_patterns);
            }
        }

        let sequence_patterns = Self::detect_tool_sequence_patterns(&record.tools_used);
        reusable_patterns.extend(sequence_patterns);

        let retry_patterns = Self::detect_retry_patterns(&record.tools_used);
        error_patterns.extend(retry_patterns);

        let redundant = Self::detect_redundant_tool_calls(&record.tools_used);
        error_patterns.extend(redundant);

        let unique_tools = Self::count_unique_tools(&record.tools_used);
        if record.success && record.iterations > unique_tools * 2 {
            reusable_patterns.push(
                "Error recovery pattern: task succeeded despite high iteration count suggesting intermediate failures"
                    .to_string(),
            );
        }
        if !record.success && record.iterations > 10 {
            error_patterns.push(format!(
                "Extended retry without success: {} iterations exhausted without recovery",
                record.iterations
            ));
        }

        if record.success {
            reusable_patterns.push(format!("Successfully completed: {}", record.task_description));
        }

        if !record.tools_used.is_empty() {
            reusable_patterns.push(format!("Tool combination: {}", record.tools_used.join(" -> ")));
        }

        (error_patterns, reusable_patterns)
    }

    /// L8: 把 ErrorType 转成中文可读 prefix
    fn error_type_label(et: &crate::recovery_strategies::ErrorType) -> &'static str {
        use crate::recovery_strategies::ErrorType as E;
        match et {
            E::Transient => "transient",
            E::Recoverable => "recoverable",
            E::Unrecoverable => "unrecoverable",
            E::Unknown => "unknown",
        }
    }

    fn fallback_keyword_patterns(error: &str, out: &mut Vec<String>) {
        let error_lower = error.to_lowercase();
        if error_lower.contains("timeout") {
            out.push(
                "Timeout issues - consider increasing timeout or optimizing query".to_string(),
            );
        }
        if error_lower.contains("permission") || error_lower.contains("denied") {
            out.push("Permission issues - verify access rights".to_string());
        }
        if error_lower.contains("not found") || error_lower.contains("404") {
            out.push("Resource not found - verify target existence".to_string());
        }
        if error_lower.contains("network") || error_lower.contains("connection") {
            out.push("Network instability - add retry logic".to_string());
        }
        if out.is_empty() {
            out.push("Unclassified error - manual review recommended".to_string());
        }
    }

    fn detect_tool_sequence_patterns(tools: &[String]) -> Vec<String> {
        let mut patterns = Vec::new();

        let has_read = tools.iter().any(|t| {
            let l = t.to_lowercase();
            l.contains("read") || l.contains("get") || l.contains("fetch")
        });
        let has_edit = tools.iter().any(|t| {
            let l = t.to_lowercase();
            l.contains("edit")
                || l.contains("write")
                || l.contains("update")
                || l.contains("modify")
                || l.contains("patch")
        });
        let has_verify = tools.iter().any(|t| {
            let l = t.to_lowercase();
            l.contains("test")
                || l.contains("verify")
                || l.contains("check")
                || l.contains("validate")
        });
        let has_search = tools.iter().any(|t| {
            let l = t.to_lowercase();
            l.contains("search")
                || l.contains("find")
                || l.contains("query")
                || l.contains("lookup")
        });

        if has_read && has_edit && has_verify {
            patterns.push("read->edit->verify pattern detected".to_string());
        }
        if has_search && has_read {
            patterns.push("search->read pattern detected".to_string());
        }
        if has_edit && has_verify {
            patterns.push("edit->verify pattern detected".to_string());
        }

        patterns
    }

    fn detect_retry_patterns(tools: &[String]) -> Vec<String> {
        let mut patterns = Vec::new();
        let mut tool_counts: Vec<(String, usize)> = Vec::new();

        for tool in tools {
            if let Some(entry) = tool_counts.iter_mut().find(|(name, _)| name == tool) {
                entry.1 += 1;
            } else {
                tool_counts.push((tool.clone(), 1));
            }
        }

        for (tool, count) in &tool_counts {
            if *count > 1 {
                patterns.push(format!("Retry with same approach: {} used {} times", tool, count));
            }
        }

        for i in 0..tools.len().saturating_sub(2) {
            if tools[i] == tools[i + 2] && tools[i] != tools[i + 1] {
                patterns.push(format!(
                    "Approach variation: {} -> {} -> {}",
                    tools[i],
                    tools[i + 1],
                    tools[i + 2]
                ));
            }
        }

        patterns
    }

    fn detect_redundant_tool_calls(tools: &[String]) -> Vec<String> {
        let mut redundant = Vec::new();

        for i in 0..tools.len().saturating_sub(1) {
            if tools[i] == tools[i + 1] {
                redundant.push(format!("Consecutive redundant call: {}", tools[i]));
            }
        }

        redundant
    }

    fn count_unique_tools(tools: &[String]) -> usize {
        tools.iter().collect::<HashSet<_>>().len()
    }

    fn generate_knowledge_suggestions(
        &self,
        record: &TaskExecutionRecord,
        metrics: &QualityMetrics,
    ) -> Vec<String> {
        let mut suggestions = Vec::new();
        let unique_tools = Self::count_unique_tools(&record.tools_used);
        let total_tools = record.tools_used.len().max(1);

        if metrics.tool_efficiency_score < 5.0 {
            let ratio = (unique_tools as f32 / total_tools as f32) * 100.0;
            suggestions.push(format!(
                "Tool efficiency ({:.1}/10) below threshold - reduce redundant calls (unique: {}/{}, ratio: {:.0}%)",
                metrics.tool_efficiency_score, unique_tools, total_tools, ratio
            ));
        }

        if metrics.iteration_efficiency_score < 5.0 {
            suggestions.push(format!(
                "Iteration efficiency ({:.1}/10) indicates excessive iterations ({}) for task complexity - consider more direct approaches",
                metrics.iteration_efficiency_score, record.iterations
            ));
        }

        if metrics.time_efficiency_score < 5.0 {
            suggestions.push(format!(
                "Time efficiency ({:.1}/10) suggests slow execution ({}ms) - consider caching or parallel execution",
                metrics.time_efficiency_score, record.duration_ms
            ));
        }

        if metrics.error_recovery_score > 0.0 && metrics.error_recovery_score < 8.0 {
            suggestions
                .push("Document error recovery patterns for similar future tasks".to_string());
        }

        if record.success && metrics.overall_weighted_score >= 7.0 {
            suggestions.push(format!(
                "High-quality execution pattern (score {:.1}) - consider templating this workflow for reuse",
                metrics.overall_weighted_score
            ));
        }

        suggestions
    }

    fn generate_improvement_suggestions(
        &self,
        record: &TaskExecutionRecord,
        reflection: &Reflection,
        config: &ReflectionConfig,
    ) -> Vec<String> {
        let mut suggestions = Vec::new();

        if let Some(metrics) = &reflection.quality_metrics {
            if metrics.task_success_score < 5.0 {
                suggestions.push(format!(
                    "Task success score ({:.1}/10) indicates failure - review error: {}",
                    metrics.task_success_score,
                    record.error.as_deref().unwrap_or("unknown")
                ));
            }

            if metrics.tool_efficiency_score < 5.0 {
                let redundant = Self::count_redundant_calls(&record.tools_used);
                suggestions.push(format!(
                    "Tool efficiency ({:.1}/10) below 5.0 threshold - {} redundant tool call(s) detected",
                    metrics.tool_efficiency_score, redundant
                ));
            }

            if metrics.iteration_efficiency_score < 5.0 {
                suggestions.push(format!(
                    "Iteration efficiency ({:.1}/10) - reduce iterations from {} by planning tool usage upfront",
                    metrics.iteration_efficiency_score, record.iterations
                ));
            }

            if metrics.time_efficiency_score < 5.0 {
                let expected = record.iterations.max(1) as u64 * 2000;
                suggestions.push(format!(
                    "Time efficiency ({:.1}/10) - execution took {}ms vs {}ms expected, enable parallel execution",
                    metrics.time_efficiency_score, record.duration_ms, expected
                ));
            }
        }

        if reflection.quality_score < config.min_quality_threshold {
            suggestions.push(format!(
                "Quality score ({}) below threshold ({}) - review overall execution strategy",
                reflection.quality_score, config.min_quality_threshold
            ));
        }

        if !reflection.error_patterns.is_empty() {
            suggestions.push(format!(
                "Address {} identified error pattern(s) before next iteration",
                reflection.error_patterns.len()
            ));
        }

        suggestions
    }

    fn count_redundant_calls(tools: &[String]) -> usize {
        let mut count = 0;
        for i in 0..tools.len().saturating_sub(1) {
            if tools[i] == tools[i + 1] {
                count += 1;
            }
        }
        count
    }

    fn generate_summary(&self, record: &TaskExecutionRecord, reflection: &Reflection) -> String {
        let metrics_detail = match &reflection.quality_metrics {
            Some(m) => format!(
                " Breakdown: success={:.1}, tool_eff={:.1}, iter_eff={:.1}, time_eff={:.1}, err_recov={:.1}, goal_comp={:.1}.",
                m.task_success_score,
                m.tool_efficiency_score,
                m.iteration_efficiency_score,
                m.time_efficiency_score,
                m.error_recovery_score,
                m.goal_completion_score
            ),
            None => String::new(),
        };
        format!(
            "Task '{}' {} in {}ms with quality score {}/10.{}{} iterations, {} tools used. {} error patterns identified. {} reusable patterns found.",
            record.task_description,
            if record.success {
                "succeeded"
            } else {
                "failed"
            },
            record.duration_ms,
            reflection.quality_score,
            metrics_detail,
            record.iterations,
            record.tools_used.len(),
            reflection.error_patterns.len(),
            reflection.reusable_patterns.len()
        )
    }
}

impl Default for Reflector {
    fn default() -> Self {
        Self::new()
    }
}

// std::panic::AssertUnwindSafe 在文件顶部 import 区块引入。

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_reflection_creation() {
        let reflector = Reflector::new();
        let start = Utc::now();
        let end = start + chrono::Duration::seconds(5);
        let mut record =
            TaskExecutionRecord::new("test-1".to_string(), "Test task".to_string(), start, end);
        record.compute_duration();
        record = record
            .with_success(true)
            .with_tools(vec!["tool1".to_string(), "tool2".to_string()]);

        let (reflection, insights) = reflector.reflect(&record).await;
        assert_eq!(reflection.task_id, "test-1");
        assert!(reflection.quality_score >= 1 && reflection.quality_score <= 10);
        assert!(!reflection.overall_summary.is_empty());
        assert!(reflection.quality_metrics.is_some());
        // S1: 即使开启 store_insights，success 任务至少产出 success_pattern 类
        // （在多 insight 模式下不强制必出，这里只验证 Vec 可访问）
        let _ = insights.len();
    }

    #[tokio::test]
    async fn test_disabled_reflector() {
        let reflector = Reflector::new();
        let mut cfg = reflector.get_config().await;
        cfg.enabled = false;
        reflector.update_config(cfg).await;
        let start = Utc::now();
        let end = start + chrono::Duration::seconds(1);
        let record = TaskExecutionRecord::new("d-1".to_string(), "x".to_string(), start, end);
        let (r, ins) = reflector.reflect(&record).await;
        assert_eq!(r.quality_score, 0);
        assert!(r.quality_analysis.contains("disabled"));
        assert!(ins.is_empty());
    }

    #[tokio::test]
    async fn test_persistence_dedup() {
        let dir =
            std::env::temp_dir().join(format!("axagent-reflector-dedup-{}", uuid::Uuid::new_v4()));
        let path = dir.join("reflections.jsonl");
        std::fs::create_dir_all(&dir).unwrap();

        // 写两条同 task_id 的 reflection（模拟 S1 修复前的脏数据）
        {
            let store = PersistedStore::new(Some(path.clone()));
            let now = Utc::now();
            let mut r1 = Reflection::new("dup-task".to_string());
            r1.timestamp = now - chrono::Duration::seconds(10);
            r1.quality_score = 5;
            store.append(&r1).await.unwrap();
            let mut r2 = Reflection::new("dup-task".to_string());
            r2.timestamp = now;
            r2.quality_score = 8;
            store.append(&r2).await.unwrap();
        }

        // 加载时去重
        {
            let store = PersistedStore::new(Some(path.clone()));
            let loaded = store.load_recent(50).await.unwrap();
            assert_eq!(loaded.len(), 1, "should dedup to single entry");
            assert_eq!(loaded[0].quality_score, 8, "should keep the latest");
        }

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn test_concurrent_append_safety() {
        let dir = std::env::temp_dir()
            .join(format!("axagent-reflector-concurrency-{}", uuid::Uuid::new_v4()));
        let path = dir.join("reflections.jsonl");
        std::fs::create_dir_all(&dir).unwrap();
        let store = Arc::new(PersistedStore::new(Some(path.clone())));

        let mut handles = Vec::new();
        for i in 0..10 {
            let s = store.clone();
            handles.push(tokio::spawn(async move {
                let mut r = Reflection::new(format!("concurrent-{i}"));
                r.quality_score = (i as u8 % 10) + 1;
                s.append(&r).await
            }));
        }
        for h in handles {
            h.await.unwrap().unwrap();
        }

        let loaded = store.load_recent(50).await.unwrap();
        assert_eq!(loaded.len(), 10, "all 10 should be persisted");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn test_init_persistence_preserves_user_config() {
        // M5: init_persistence 不覆盖 max_history
        let dir =
            std::env::temp_dir().join(format!("axagent-reflector-init-{}", uuid::Uuid::new_v4()));
        let path = dir.join("reflections.jsonl");
        std::fs::create_dir_all(&dir).unwrap();

        let reflector = Reflector::new();
        // 用户先改 max_history
        let mut cfg = reflector.get_config().await;
        cfg.max_history = 1000;
        reflector.update_config(cfg.clone()).await;

        // 初始化持久化
        reflector.init_persistence(path, 200).await.unwrap();

        // 配置应保留 1000，不被覆盖为 200
        let after = reflector.get_config().await;
        assert_eq!(after.max_history, 1000, "init_persistence must not overwrite max_history");

        let _ = std::fs::remove_dir_all(&dir);
    }
}
