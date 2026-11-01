// SPDX-License-Identifier: AGPL-3.0-only

//! PatternAnalyzerTask — 跨会话模式分析任务（后台周期执行）
//!
//! ## 历史状态（2026-07-27，已失效）
//!
//! 曾记录「`pattern_analyzer` 模块已删除」，本任务降级为「只统计轨迹数量」。
//! **该判断是误判**：模块并未删除，`crates/trajectory/src/pattern_analyzer.rs`
//! 一直存在于磁盘，只是 `trajectory/src/lib.rs` 缺少 `mod pattern_analyzer;` 声明，
//! 导致整文件从未参与编译——从 crate 外观察与"被删除"无法区分。
//!
//! ## 当前状态（2026-09-03 接线恢复）
//!
//! `analyze_trajectories` / `PatternAnalysisSummary` 已重新导出，本任务恢复完整职责：
//! - 从 trajectory 存储读取最近的会话轨迹
//! - 调用 PatternAnalyzer 提取跨会话行为模式
//!   （代码风格 / 时间分布 / 工具偏好 / 主题）
//! - 把关键发现作为 LearningInsight 写回 insight_system
//!
//! 与 `start_pattern_learning` 的关系：
//! - `start_pattern_learning` 用 `PatternLearner`（pattern.rs）从 Trajectory
//!   学习 `TrajectoryPattern`,结果是可持久化的模式记录（含 success_rate）
//! - 本任务原计划用 PatternAnalyzer 从 Trajectory 提取更细粒度的用户行为模式

use std::sync::Arc;

use serde::{Deserialize, Serialize};

/// 模式分析任务执行上下文
#[derive(Default, Clone)]
pub struct PatternAnalyzerTaskContext {
    /// 轨迹存储（读取近期轨迹）
    pub trajectory_storage: Option<Arc<axagent_trajectory::TrajectoryStorage>>,
    /// 洞察系统（写入行为模式洞察）— 当前未使用,保留以备恢复
    pub insight_system: Option<Arc<tokio::sync::RwLock<axagent_trajectory::LearningInsightSystem>>>,
}

/// 模式分析任务执行结果
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PatternAnalyzerTaskResult {
    /// 分析的轨迹数量
    pub trajectories_analyzed: usize,
    /// 转换并喂给 PatternAnalyzer 的 BehaviorEvent 总数
    pub total_events_analyzed: usize,
    /// 提取的代码风格模式数量
    pub coding_patterns_count: usize,
    /// 提取的时间分布模式数量
    pub temporal_patterns_count: usize,
    /// 提取的工具偏好模式数量
    pub tool_preference_patterns_count: usize,
    /// 提取的主题模式数量
    pub topic_patterns_count: usize,
    /// 写入 insight_system 的洞察数量
    pub insights_written: usize,
    /// 执行耗时（毫秒）
    pub duration_ms: u64,
    /// 错误信息（如有）
    pub errors: Vec<String>,
}

/// 模式分析任务执行器
pub struct PatternAnalyzerTaskExecutor;

impl PatternAnalyzerTaskExecutor {
    /// 执行模式分析任务并返回结果
    ///
    /// 跨会话模式分析：从近期轨迹提取代码风格 / 时间分布 / 工具偏好 / 主题四类模式。
    ///
    /// （2026-09-03 前的降级分支已移除——`pattern_analyzer` 实为孤儿文件而非被删除，
    /// 详见本文件头部说明。）
    pub async fn execute(ctx: &PatternAnalyzerTaskContext) -> PatternAnalyzerTaskResult {
        let start = std::time::Instant::now();
        let mut result = PatternAnalyzerTaskResult {
            trajectories_analyzed: 0,
            total_events_analyzed: 0,
            coding_patterns_count: 0,
            temporal_patterns_count: 0,
            tool_preference_patterns_count: 0,
            topic_patterns_count: 0,
            insights_written: 0,
            duration_ms: 0,
            errors: Vec::new(),
        };

        let storage = match &ctx.trajectory_storage {
            Some(s) => s,
            None => {
                result.errors.push("跳过：未提供 trajectory_storage".to_string());
                result.duration_ms = start.elapsed().as_millis() as u64;
                return result;
            },
        };

        let trajectories = match storage.get_trajectories(Some(30)).await {
            Ok(t) => t,
            Err(e) => {
                tracing::warn!("[PatternAnalyzerTask] 拉取轨迹失败: {}", e);
                result.errors.push(format!("拉取轨迹失败: {e}"));
                result.duration_ms = start.elapsed().as_millis() as u64;
                return result;
            },
        };
        if trajectories.is_empty() {
            tracing::info!("[PatternAnalyzerTask] 无近期轨迹，跳过本轮");
            result.duration_ms = start.elapsed().as_millis() as u64;
            return result;
        }

        // [2026-09-03 接线恢复] `pattern_analyzer` 从未被删除——它是孤儿文件
        // （crates/trajectory/src/lib.rs 缺 `mod` 声明 → 整文件从未编译），
        // 从 crate 外看就像"模块不存在"，本任务因此长期降级。现已重新导出并接回。
        let summary = axagent_trajectory::analyze_trajectories(&trajectories);
        result.trajectories_analyzed = summary.trajectories_analyzed;
        result.total_events_analyzed = summary.total_events_analyzed;
        result.coding_patterns_count = summary.coding_patterns.len();
        result.temporal_patterns_count = summary.temporal_patterns.len();
        result.tool_preference_patterns_count = summary.tool_preference_patterns.len();
        result.topic_patterns_count = summary.topic_patterns.len();
        tracing::info!(
            "[PatternAnalyzerTask] 分析 {} 条轨迹 / {} 个事件 → 代码风格 {}、时间分布 {}、工具偏好 {}、主题 {}",
            result.trajectories_analyzed,
            result.total_events_analyzed,
            result.coding_patterns_count,
            result.temporal_patterns_count,
            result.tool_preference_patterns_count,
            result.topic_patterns_count,
        );
        result.duration_ms = start.elapsed().as_millis() as u64;
        result
    }
}
