// SPDX-License-Identifier: AGPL-3.0-only

//! Trajectory storage module using SeaORM

use crate::fts5::{FTS5Config, FTS5Health, FTS5Query, FTS5RebuildReport, FTS5Result, FTS5Search};
use crate::memory::{Entity, Relationship, RelationshipType};
use crate::skill::Skill;
use crate::trajectory::{
    MessageRole, RLTrainingEntry, RewardSignal, Trajectory, TrajectoryExportOptions,
    TrajectoryOutcome, TrajectoryPattern, TrajectoryQuery, TrajectoryStep,
};
use anyhow::{Context, Result};
use axagent_entities::{
    knowledge_entities, knowledge_relations, memory_items, trajectories,
    trajectory_learned_patterns, trajectory_messages, trajectory_patterns, trajectory_preferences,
    trajectory_rewards, trajectory_sessions, trajectory_skill_executions, trajectory_skills,
    trajectory_steps, trajectory_workflow_reflections,
};
use chrono::Utc;
use futures::FutureExt;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, DatabaseConnection, EntityTrait, ExprTrait, IntoActiveModel,
    PaginatorTrait, QueryFilter, QueryOrder, Set, TransactionTrait,
};
use std::panic::AssertUnwindSafe;
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::{info, warn};
use uuid::Uuid;

pub struct TrajectoryStorage {
    db: Arc<DatabaseConnection>,
    fts_searcher: Option<FTS5Search>,
    /// 保存轨迹时是否抽取因果观测（默认关闭，见 [`TrajectoryStorage::set_causal_enabled`]）
    causal_enabled: bool,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TrajectoryCleanupConfig {
    pub max_age_days: Option<u32>,
    pub max_trajectories: Option<u32>,
}

impl Default for TrajectoryCleanupConfig {
    fn default() -> Self {
        Self { max_age_days: Some(90), max_trajectories: Some(10000) }
    }
}

impl TrajectoryStorage {
    pub fn new(db: Arc<DatabaseConnection>) -> Self {
        Self { db, fts_searcher: None, causal_enabled: false }
    }

    /// 底层数据库连接（供因果边/校准等跨表查询复用同一连接）
    pub fn db(&self) -> &DatabaseConnection {
        &self.db
    }

    pub fn with_fts(
        db: Arc<DatabaseConnection>,
        fts_conn: Arc<Mutex<rusqlite::Connection>>,
    ) -> Self {
        Self {
            db,
            fts_searcher: Some(FTS5Search::new(fts_conn, FTS5Config::default())),
            causal_enabled: false,
        }
    }

    /// 开启/关闭轨迹保存时的因果观测抽取。
    ///
    /// 关闭时 `save_trajectory` 的行为与启用前完全一致。
    pub fn set_causal_enabled(&mut self, enabled: bool) {
        self.causal_enabled = enabled;
    }

    /// 因果观测是否已开启
    pub fn is_causal_enabled(&self) -> bool {
        self.causal_enabled
    }

    /// 记录一次意图转移观测 `intent:A → intent:B`。
    ///
    /// 开关关闭时为空操作；失败只记日志，不影响调用方。
    pub async fn observe_intent_transition(&self, from: &str, to: &str, delay_ms: Option<i64>) {
        if !self.causal_enabled || from == to {
            return;
        }
        if let Err(e) = crate::causal::observe_edge(
            self.db.as_ref(),
            from,
            to,
            true,
            delay_ms,
            "intent_transition",
        )
        .await
        {
            tracing::warn!("causal: intent transition observation failed: {e:#}");
        }
    }

    /// 依据因果边为当前预测生成可解释建议。开关关闭时返回空表。
    pub async fn causal_suggestions(
        &self,
        prediction: &crate::proactive_assistant::ContextPrediction,
        max: usize,
    ) -> Vec<crate::proactive_assistant::ProactiveSuggestion> {
        if !self.causal_enabled {
            return Vec::new();
        }
        crate::causal::causal_suggestions_for_intent(self.db.as_ref(), prediction, max).await
    }

    /// 从数据库文件路径创建带 FTS5 全文搜索的存储实例。
    /// 自动创建 FTS5 虚拟表（如不存在）。
    pub async fn with_fts_path(db: Arc<DatabaseConnection>, db_file_path: &str) -> Result<Self> {
        let db_file_path = db_file_path.to_string();
        let conn = tokio::task::spawn_blocking(move || {
            let conn = rusqlite::Connection::open(&db_file_path)
                .context("Failed to open FTS5 database")?;
            conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA busy_timeout=5000;")
                .context("Failed to set FTS5 connection pragmas")?;
            Ok::<_, anyhow::Error>(conn)
        })
        .await??;
        let conn = Arc::new(Mutex::new(conn));
        let fts = FTS5Search::new(conn, FTS5Config::default());
        fts.create_fts_tables().await?;
        Ok(Self { db, fts_searcher: Some(fts), causal_enabled: false })
    }

    // ── Trajectories ──

    /// 保存轨迹（事务化：轨迹主体 + steps + rewards 在同一事务中）
    /// FTS 索引在事务外执行，避免与 SeaORM 事务争用。
    pub async fn save_trajectory(&self, t: &Trajectory) -> Result<()> {
        // P0-8: 整个写入流程包在事务中
        let txn = self.db.begin().await?;

        let am = trajectories::ActiveModel {
            id: Set(t.id.clone()),
            session_id: Set(t.session_id.clone()),
            user_id: Set(t.user_id.clone()),
            agent_name: Set(t.agent_name.clone()),
            topic: Set(t.topic.clone()),
            summary: Set(t.summary.clone()),
            outcome: Set(format!("{:?}", t.outcome).to_lowercase()),
            duration_ms: Set(t.duration_ms as i64),
            quality_overall: Set(t.quality.overall),
            quality_task_completion: Set(t.quality.task_completion),
            quality_tool_efficiency: Set(t.quality.tool_efficiency),
            quality_reasoning_quality: Set(t.quality.reasoning_quality),
            quality_user_satisfaction: Set(t.quality.user_satisfaction),
            value_score: Set(t.value_score),
            patterns: Set(serde_json::to_string(&t.patterns)?),
            created_at: Set(t.created_at.to_rfc3339()),
            replay_count: Set(t.replay_count as i32),
            last_replay_at: Set(t.last_replay_at.map(|dt| dt.to_rfc3339())),
            // 新轨迹默认有效（append-only 证据链，v120 新增字段）
            is_invalidated: Set(0),
        };
        // P1-2: on_conflict 不再更新 CreatedAt（保留原创建时间）
        trajectories::Entity::insert(am)
            .on_conflict(
                sea_orm::sea_query::OnConflict::column(trajectories::Column::Id)
                    .update_columns([
                        trajectories::Column::SessionId,
                        trajectories::Column::AgentName,
                        trajectories::Column::Topic,
                        trajectories::Column::Summary,
                        trajectories::Column::Outcome,
                        trajectories::Column::DurationMs,
                        trajectories::Column::QualityOverall,
                        trajectories::Column::QualityTaskCompletion,
                        trajectories::Column::QualityToolEfficiency,
                        trajectories::Column::QualityReasoningQuality,
                        trajectories::Column::QualityUserSatisfaction,
                        trajectories::Column::ValueScore,
                        trajectories::Column::Patterns,
                        trajectories::Column::ReplayCount,
                        trajectories::Column::LastReplayAt,
                        // 重保存视为重新启用该轨迹：清除失效标记（append-only 证据链可恢复）
                        trajectories::Column::IsInvalidated,
                    ])
                    .to_owned(),
            )
            .exec(&txn)
            .await?;

        trajectory_steps::Entity::delete_many()
            .filter(trajectory_steps::Column::TrajectoryId.eq(&t.id))
            .exec(&txn)
            .await?;
        for (idx, step) in t.steps.iter().enumerate() {
            trajectory_steps::ActiveModel {
                trajectory_id: Set(t.id.clone()),
                step_index: Set(idx as i32),
                timestamp_ms: Set(step.timestamp_ms as i64),
                role: Set(format!("{:?}", step.role).to_lowercase()),
                content: Set(step.content.clone()),
                reasoning: Set(step.reasoning.clone()),
                tool_calls: Set(step
                    .tool_calls
                    .as_ref()
                    .and_then(|c| serde_json::to_string(c).ok())),
                tool_results: Set(step
                    .tool_results
                    .as_ref()
                    .and_then(|r| serde_json::to_string(r).ok())),
                ..Default::default()
            }
            .insert(&txn)
            .await?;
        }

        trajectory_rewards::Entity::delete_many()
            .filter(trajectory_rewards::Column::TrajectoryId.eq(&t.id))
            .exec(&txn)
            .await?;
        for r in &t.rewards {
            trajectory_rewards::ActiveModel {
                id: Set(Uuid::new_v4().to_string()),
                trajectory_id: Set(t.id.clone()),
                reward_type: Set(format!("{:?}", r.reward_type)),
                value: Set(r.value),
                step_index: Set(r.step_index as i32),
                // ⚠ 该列**不是日历时间**，它承载的是 `RewardSignal.timestamp_ms`。
                // `timestamp_ms` 的基准由契约声明为「未定义 / 相对」（`causal.rs` 模块头与
                // `causal_delay` 的文档注释都写明「不假设基准」），生产端写的是
                // **相对轨迹起点的毫秒偏移**（`agent/src/trajectory_recorder.rs:142`）。
                // 历史实现把它喂给 `DateTime::from_timestamp_millis` ⇒ 偏移 56867 被落成
                // `1970-01-01T00:00:56.867Z`（实测 13 行）：**用一个假日期掩盖了真偏移**，
                // 且 `.unwrap_or_else(Utc::now)` 永不触发（`from_timestamp_millis` 对任何
                // i64 都返回 `Some`），所以坏值只能一路落库、不会被兜底拦下。
                // 现改存原样十进制毫秒；读端 `get_trajectory_rewards` 双向前兼容
                // （先按数字解析，再回退 RFC3339 以兼容 2026-09-18 之前的存量行）。
                created_at: Set(r.timestamp_ms.to_string()),
            }
            .insert(&txn)
            .await?;
        }

        txn.commit().await?;

        // FTS 索引在事务外执行
        let _ = self.index_trajectory_fts(t).await;

        // 因果观测同样在事务外执行。失败仅告警——因果边是增强特性，
        // 不得因观测失败影响已经落库的轨迹。
        if self.causal_enabled {
            match crate::causal::observe_from_trajectory(self.db.as_ref(), t).await {
                Ok(count) => {
                    tracing::debug!("causal: observed {count} edges from trajectory {}", t.id)
                },
                Err(e) => {
                    tracing::warn!("causal: observation failed for trajectory {}: {:#}", t.id, e)
                },
            }
        }

        Ok(())
    }

    pub async fn get_trajectory(&self, id: &str) -> Result<Option<Trajectory>> {
        match trajectories::Entity::find_by_id(id).one(self.db.as_ref()).await? {
            Some(m) => Ok(Some(model_to_trajectory(
                &m,
                self.get_trajectory_steps(&m.id).await?,
                self.get_trajectory_rewards(&m.id).await?,
            ))),
            None => Ok(None),
        }
    }

    /// 获取有效的轨迹列表（已标记失效的 append-only 证据不参与活动查询）。
    pub async fn get_trajectories(&self, limit: Option<usize>) -> Result<Vec<Trajectory>> {
        let models = trajectories::Entity::find()
            .filter(trajectories::Column::IsInvalidated.eq(0))
            .order_by_desc(trajectories::Column::CreatedAt)
            .all(self.db.as_ref())
            .await?;
        let mut r = Vec::new();
        let end = limit.unwrap_or(models.len()).min(models.len());
        for m in models.into_iter().take(end) {
            r.push(model_to_trajectory(
                &m,
                self.get_trajectory_steps(&m.id).await?,
                self.get_trajectory_rewards(&m.id).await?,
            ));
        }
        Ok(r)
    }

    /// P3-1（阶段三）：标记轨迹失效（软删除，append-only 证据存储）。
    ///
    /// 轨迹及其 steps/rewards/skill_executions 作为进化证据**不可物理删除**，
    /// 仅置 `is_invalidated = 1` 使其退出活动查询（get_trajectories /
    /// get_session_trajectories / query_trajectories）；同时清理 FTS 索引，
    /// 避免全文搜索命中已失效证据。证据本体保留，供贝叶斯后验回溯。
    pub async fn delete_trajectory(&self, id: &str) -> Result<()> {
        let m = trajectories::Entity::find_by_id(id)
            .one(self.db.as_ref())
            .await?
            .context("Trajectory not found")?;
        let mut am: trajectories::ActiveModel = m.into_active_model();
        am.is_invalidated = Set(1);
        am.update(self.db.as_ref()).await?;
        let _ = self.delete_trajectory_fts(id).await;
        info!("Invalidated trajectory {}", id);
        Ok(())
    }

    /// P1-5: 用字符串比较 ISO8601 / RFC3339 时间戳（字典序与时序一致）。
    /// 阶段三起清理为软删除：仅标记失效，证据本体保留（append-only）。
    pub async fn cleanup_old_trajectories_by_age(&self, max_age_days: u32) -> Result<usize> {
        let cutoff = Utc::now() - chrono::Duration::days(max_age_days as i64);
        let cutoff_str = cutoff.to_rfc3339();
        // ISO8601 / RFC3339 格式为 year-first、zero-padded，字符串字典序与时序一致，
        // 不需要 datetime() 函数（该函数是 SQLite 专有，PostgreSQL 不存在）。
        let old_trajectories = trajectories::Entity::find()
            .filter(trajectories::Column::IsInvalidated.eq(0))
            .filter(sea_orm::sea_query::Expr::cust(format!("created_at < '{}'", cutoff_str)))
            .all(self.db.as_ref())
            .await?;
        let count = old_trajectories.len();
        for traj in old_trajectories {
            self.delete_trajectory(&traj.id).await?;
        }
        Ok(count)
    }

    /// P1-4: 避免全表加载，使用 NOT IN 子查询找出需清理的 ID。
    /// 阶段三起清理为软删除：仅标记失效，证据本体保留（append-only）。
    pub async fn cleanup_old_trajectories_by_count(&self, max_trajectories: u32) -> Result<usize> {
        // 先查总数判断是否需要清理（仅统计有效轨迹）
        let total = trajectories::Entity::find()
            .filter(trajectories::Column::IsInvalidated.eq(0))
            .count(self.db.as_ref())
            .await?;
        if total <= max_trajectories as u64 {
            return Ok(0);
        }
        // 用 NOT IN 子查询找出需要保留的 ID 集合
        let to_delete_ids: Vec<String> = {
            use sea_orm::PaginatorTrait;
            // 取第二页（跳过前 max_trajectories 条），即为超出保留阈值的最旧轨迹
            let page_size: u64 = std::cmp::max(max_trajectories as u64, 1);
            let paginator = trajectories::Entity::find()
                .filter(trajectories::Column::IsInvalidated.eq(0))
                .order_by_desc(trajectories::Column::CreatedAt)
                .paginate(self.db.as_ref(), page_size);
            let extra = paginator.fetch_page(1).await?;
            extra.into_iter().map(|t| t.id).collect()
        };
        let count = to_delete_ids.len();
        for id in to_delete_ids {
            let _ = self.delete_trajectory(&id).await;
        }
        Ok(count)
    }

    pub async fn cleanup(&self, config: &TrajectoryCleanupConfig) -> Result<usize> {
        let mut total_deleted = 0;
        if let Some(max_age_days) = config.max_age_days {
            total_deleted += self.cleanup_old_trajectories_by_age(max_age_days).await?;
        }
        if let Some(max_trajectories) = config.max_trajectories {
            total_deleted += self.cleanup_old_trajectories_by_count(max_trajectories).await?;
        }
        Ok(total_deleted)
    }

    pub async fn get_session_trajectories(&self, session_id: &str) -> Result<Vec<Trajectory>> {
        let models = trajectories::Entity::find()
            .filter(trajectories::Column::SessionId.eq(session_id))
            .filter(trajectories::Column::IsInvalidated.eq(0))
            .order_by_asc(trajectories::Column::CreatedAt)
            .all(self.db.as_ref())
            .await?;
        let mut r = Vec::new();
        for m in models {
            r.push(model_to_trajectory(
                &m,
                self.get_trajectory_steps(&m.id).await?,
                self.get_trajectory_rewards(&m.id).await?,
            ));
        }
        Ok(r)
    }

    pub async fn query_trajectories(&self, query: &TrajectoryQuery) -> Result<Vec<Trajectory>> {
        let mut q = trajectories::Entity::find();
        // 已标记失效的 append-only 证据不参与活动查询（贝叶斯后验回溯走 get_trajectory）
        q = q.filter(trajectories::Column::IsInvalidated.eq(0));
        if let Some(ref sid) = query.session_id {
            q = q.filter(trajectories::Column::SessionId.eq(sid));
        }
        if let Some(ref uid) = query.user_id {
            q = q.filter(trajectories::Column::UserId.eq(uid));
        }
        if let Some(ref topic) = query.topic {
            q = q.filter(trajectories::Column::Topic.like(format!("%{}%", topic)));
        }
        if let Some(mq) = query.min_quality {
            q = q.filter(trajectories::Column::QualityOverall.gte(mq));
        }
        if let Some(mv) = query.min_value_score {
            q = q.filter(trajectories::Column::ValueScore.gte(mv));
        }
        if let Some(ref outcome) = query.outcome {
            q = q.filter(trajectories::Column::Outcome.eq(format!("{:?}", outcome)));
        }
        if let Some((start, end)) = query.time_range {
            q = q
                .filter(trajectories::Column::CreatedAt.gte(start.to_rfc3339()))
                .filter(trajectories::Column::CreatedAt.lte(end.to_rfc3339()));
        }
        q = q.order_by_desc(trajectories::Column::CreatedAt);
        let models = q.all(self.db.as_ref()).await?;
        let end = query.limit.unwrap_or(models.len()).min(models.len());
        let mut r = Vec::new();
        for m in models.into_iter().take(end) {
            r.push(model_to_trajectory(
                &m,
                self.get_trajectory_steps(&m.id).await?,
                self.get_trajectory_rewards(&m.id).await?,
            ));
        }
        Ok(r)
    }

    async fn get_trajectory_steps(&self, trajectory_id: &str) -> Result<Vec<TrajectoryStep>> {
        Ok(trajectory_steps::Entity::find()
            .filter(trajectory_steps::Column::TrajectoryId.eq(trajectory_id))
            .order_by_asc(trajectory_steps::Column::StepIndex)
            .all(self.db.as_ref())
            .await?
            .into_iter()
            .map(|s| TrajectoryStep {
                timestamp_ms: s.timestamp_ms as u64,
                role: serde_json::from_str(&format!("\"{}\"", s.role))
                    .unwrap_or(MessageRole::Assistant),
                content: s.content,
                reasoning: s.reasoning,
                tool_calls: s.tool_calls.and_then(|c| serde_json::from_str(&c).ok()),
                tool_results: s.tool_results.and_then(|r| serde_json::from_str(&r).ok()),
            })
            .collect())
    }

    async fn get_trajectory_rewards(&self, trajectory_id: &str) -> Result<Vec<RewardSignal>> {
        Ok(trajectory_rewards::Entity::find()
            .filter(trajectory_rewards::Column::TrajectoryId.eq(trajectory_id))
            .all(self.db.as_ref())
            .await?
            .into_iter()
            .map(|r| {
                let rt = match r.reward_type.as_str() {
                    "task_completion" => crate::trajectory::RewardType::TaskCompletion,
                    "tool_efficiency" => crate::trajectory::RewardType::ToolEfficiency,
                    "reasoning_quality" => crate::trajectory::RewardType::ReasoningQuality,
                    _ => crate::trajectory::RewardType::UserFeedback,
                };
                // `created_at` 双向前兼容：新行为存十进制毫秒（= `timestamp_ms` 原样），
                // 旧行为存 `from_timestamp_millis` 产出的 RFC3339 —— 两者还原成同一个毫秒数。
                // 刻意**不**用「解析失败 ⇒ `Utc::now()`」兜底：那会在读不出时**静默伪造一个当前
                // 时刻**，让「没读到」看起来像「读到了」（同族：静默降级 / 兜底档永不触发）。
                // 改为告警 + `0` —— 在本字段的语义（相对轨迹起点的偏移）里 0 是诚实的缺省。
                let timestamp_ms = match parse_reward_timestamp_ms(&r.created_at) {
                    Some(ms) => ms,
                    None => {
                        tracing::warn!(
                            "trajectory_rewards.created_at 既非毫秒数也非 RFC3339 ⇒ timestamp_ms 取 0（id={}, value={:?}）",
                            r.id,
                            r.created_at
                        );
                        0
                    },
                };
                RewardSignal {
                    reward_type: rt,
                    value: r.value,
                    step_index: r.step_index as usize,
                    timestamp_ms,
                    metadata: serde_json::Value::Null,
                }
            })
            .collect())
    }

    // ── Patterns ──

    pub async fn save_pattern(&self, p: &TrajectoryPattern) -> Result<()> {
        trajectory_patterns::Entity::insert(trajectory_patterns::ActiveModel {
            id: Set(p.id.clone()),
            name: Set(p.name.clone()),
            description: Set(p.description.clone()),
            pattern_type: Set(p.pattern_type.clone()),
            trajectory_ids: Set(serde_json::to_string(&p.trajectory_ids)?),
            frequency: Set(p.frequency as i32),
            success_rate: Set(p.success_rate),
            average_quality: Set(p.average_quality),
            average_value_score: Set(p.average_value_score),
            reward_profile: Set(serde_json::to_string(&p.reward_profile)?),
            created_at: Set(p.created_at.to_rfc3339()),
        })
        // ⚠ 本冲突键是 `Id`，其**有效性前提**是「同一逻辑模式每次拿到同一 id」——
        //   由 `TrajectoryPattern::new` 用 `name` 派生主键来保证（见
        //   `harness/src/trajectory_types.rs::stable_id_for_name` 的根因说明）。
        //   调用方自定义主键时（如 `rl_checkpoint:` 检查点）语义是「同一 id 覆盖」。
        //
        //   更新列必须覆盖**全部聚合列**：只更新 frequency/success_rate 而漏掉
        //   `trajectory_ids` 会让同一行自相矛盾（频率涨到 5 而行内只有 1 个轨迹 id）。
        .on_conflict(
            sea_orm::sea_query::OnConflict::column(trajectory_patterns::Column::Id)
                .update_columns([
                    trajectory_patterns::Column::Name,
                    trajectory_patterns::Column::Description,
                    trajectory_patterns::Column::TrajectoryIds,
                    trajectory_patterns::Column::Frequency,
                    trajectory_patterns::Column::SuccessRate,
                    trajectory_patterns::Column::AverageQuality,
                    trajectory_patterns::Column::AverageValueScore,
                    trajectory_patterns::Column::RewardProfile,
                ])
                .to_owned(),
        )
        .exec(self.db.as_ref())
        .await?;
        Ok(())
    }

    pub async fn get_patterns(&self) -> Result<Vec<TrajectoryPattern>> {
        Ok(trajectory_patterns::Entity::find()
            .order_by_desc(trajectory_patterns::Column::Frequency)
            .all(self.db.as_ref())
            .await?
            .iter()
            .map(model_to_traj_pattern)
            .collect())
    }

    pub async fn get_patterns_by_success_rate(
        &self,
        min_sr: f64,
        limit: Option<usize>,
    ) -> Result<Vec<TrajectoryPattern>> {
        let models = trajectory_patterns::Entity::find()
            .filter(trajectory_patterns::Column::SuccessRate.gte(min_sr))
            .order_by_desc(trajectory_patterns::Column::SuccessRate)
            .all(self.db.as_ref())
            .await?;
        let end = limit.unwrap_or(models.len()).min(models.len());
        Ok(models.iter().take(end).map(model_to_traj_pattern).collect())
    }

    // ── Workflow Reflections（优化 3：反思历史持久化） ──
    //
    // 由 `WorkflowReflectorImpl::with_storage()` 注入 storage 后，在每次
    // `reflect()` / `reflect_node()` 内调用 `save_workflow_reflection` 落库。
    // `WorkflowOptimizer` / `WorkflowEvolver` 通过 `get_workflow_reflections`
    // 读取跨会话历史反思驱动优化（替代内存 `get_history` 的进程内限制）。

    /// 把单次反思落库。
    ///
    /// - `workflow_id`：工作流 ID（用于按模板聚合历史）
    /// - `template_id`：可选模板 ID（来自 `WorkflowExecutionRecord.template_id`）
    /// - `reflection`：反思结果（含 quality_score / patterns / metadata）
    ///
    /// 主键 `id` 使用 `uuid`（每条反思独立 ID，与 `Reflection.task_id` 解耦，
    /// 因为 `task_id = execution_id` 可能在同一工作流的多次反思中重复——
    /// 节点级反思也以 `execution_id` 作为 `task_id`）。
    pub async fn save_workflow_reflection(
        &self,
        workflow_id: &str,
        template_id: Option<&str>,
        reflection: &axagent_harness::reflection_types::Reflection,
    ) -> Result<()> {
        let error_patterns_json =
            serde_json::to_string(&reflection.error_patterns).unwrap_or_else(|_| "[]".to_string());
        let reusable_patterns_json = serde_json::to_string(&reflection.reusable_patterns)
            .unwrap_or_else(|_| "[]".to_string());
        let metadata_json = reflection
            .metadata
            .as_ref()
            .map(|v| serde_json::to_string(v).unwrap_or_else(|_| "{}".to_string()))
            .unwrap_or_else(|| "{}".to_string());
        let now = Utc::now().to_rfc3339();

        trajectory_workflow_reflections::Entity::insert(
            trajectory_workflow_reflections::ActiveModel {
                id: Set(Uuid::new_v4().to_string()),
                workflow_id: Set(workflow_id.to_string()),
                execution_id: Set(reflection.task_id.clone()),
                template_id: Set(template_id.map(|s| s.to_string())),
                quality_score: Set(i32::from(reflection.quality_score)),
                summary: Set(reflection.overall_summary.clone()),
                error_patterns_json: Set(error_patterns_json),
                reusable_patterns_json: Set(reusable_patterns_json),
                metadata_json: Set(metadata_json),
                timestamp: Set(reflection.timestamp.to_rfc3339()),
                created_at: Set(now),
            },
        )
        .exec(self.db.as_ref())
        .await?;
        Ok(())
    }

    /// 查询某工作流的最近 N 条反思（按时间戳倒序）。
    ///
    /// 用于 `WorkflowOptimizer::suggest()` / `WorkflowEvolver::run()` 读取跨会话历史，
    /// 替代 `WorkflowReflector::get_history()` 的内存限制（默认上限 100 条 / workflow）。
    pub async fn get_workflow_reflections(
        &self,
        workflow_id: &str,
        limit: usize,
    ) -> Result<Vec<axagent_harness::reflection_types::Reflection>> {
        use axagent_harness::reflection_types::{QualityMetrics, Reflection};

        let models = trajectory_workflow_reflections::Entity::find()
            .filter(trajectory_workflow_reflections::Column::WorkflowId.eq(workflow_id))
            .order_by_desc(trajectory_workflow_reflections::Column::Timestamp)
            .all(self.db.as_ref())
            .await?;

        let end = limit.min(models.len());
        Ok(models
            .into_iter()
            .take(end)
            .map(|m| {
                let error_patterns: Vec<String> =
                    serde_json::from_str(&m.error_patterns_json).unwrap_or_default();
                let reusable_patterns: Vec<String> =
                    serde_json::from_str(&m.reusable_patterns_json).unwrap_or_default();
                let metadata: Option<serde_json::Value> =
                    serde_json::from_str(&m.metadata_json).ok();
                let timestamp = chrono::DateTime::parse_from_rfc3339(&m.timestamp)
                    .map(|dt| dt.with_timezone(&Utc))
                    .unwrap_or_else(|_| Utc::now());

                // 重建 Reflection：持久化字段不包含 quality_analysis /
                // efficiency_analysis / knowledge_suggestions / improvement_suggestions
                // / quality_metrics（这些字段在重载时丢失，置为空值）。
                // 核心驱动字段（quality_score / patterns / metadata / summary）完整保留。
                Reflection {
                    task_id: m.execution_id,
                    timestamp,
                    quality_score: m.quality_score.clamp(0, 255) as u8,
                    quality_analysis: String::new(),
                    efficiency_analysis: String::new(),
                    error_patterns,
                    reusable_patterns,
                    knowledge_suggestions: Vec::new(),
                    improvement_suggestions: Vec::new(),
                    overall_summary: m.summary,
                    quality_metrics: None::<QualityMetrics>,
                    metadata,
                }
            })
            .collect())
    }

    // ── Skills ──

    pub async fn save_skill(&self, skill: &Skill) -> Result<()> {
        trajectory_skills::Entity::insert(trajectory_skills::ActiveModel {
            id: Set(skill.id.clone()),
            name: Set(skill.name.clone()),
            description: Set(skill.description.clone()),
            skill_type: Set(skill.category.clone()),
            content: Set(skill.content.clone()),
            category: Set(skill.category.clone()),
            tags: Set(serde_json::to_string(&skill.tags)?),
            scenarios: Set(serde_json::to_string(&skill.scenarios)?),
            parameters: Set(serde_json::json!({}).to_string()),
            created_at: Set(skill.created_at.to_rfc3339()),
            updated_at: Set(skill.updated_at.to_rfc3339()),
            usage_count: Set(skill.total_usages as i32),
            success_rate: Set(skill.success_rate),
            avg_execution_time_ms: Set(skill.avg_execution_time_ms as i64),
            consecutive_failures: Set(skill.consecutive_failures as i32),
            last_failure_at: Set(skill.last_failure_at.map(|dt| dt.to_rfc3339())),
        })
        .on_conflict(
            sea_orm::sea_query::OnConflict::column(trajectory_skills::Column::Id)
                .update_columns([
                    trajectory_skills::Column::Name,
                    trajectory_skills::Column::Content,
                    trajectory_skills::Column::UpdatedAt,
                    trajectory_skills::Column::UsageCount,
                    trajectory_skills::Column::SuccessRate,
                    trajectory_skills::Column::AvgExecutionTimeMs,
                    trajectory_skills::Column::ConsecutiveFailures,
                    trajectory_skills::Column::LastFailureAt,
                ])
                .to_owned(),
        )
        .exec(self.db.as_ref())
        .await?;
        let _ = self.index_skill_fts(skill).await;
        Ok(())
    }

    pub async fn get_skill(&self, id: &str) -> Result<Option<Skill>> {
        Ok(trajectory_skills::Entity::find_by_id(id)
            .one(self.db.as_ref())
            .await?
            .map(|s| model_to_skill(&s)))
    }

    pub async fn get_skills(&self) -> Result<Vec<Skill>> {
        Ok(trajectory_skills::Entity::find()
            .order_by_desc(trajectory_skills::Column::UsageCount)
            .all(self.db.as_ref())
            .await?
            .iter()
            .map(model_to_skill)
            .collect())
    }

    /// P1-3: 级联删除 skills + 关联 skill_executions + FTS
    pub async fn delete_skill(&self, id: &str) -> Result<()> {
        let txn = self.db.begin().await?;
        trajectory_skill_executions::Entity::delete_many()
            .filter(trajectory_skill_executions::Column::SkillId.eq(id))
            .exec(&txn)
            .await?;
        trajectory_skills::Entity::delete_by_id(id).exec(&txn).await?;
        txn.commit().await?;
        let _ = self.delete_skill_fts(id).await;
        info!("Deleted skill {}", id);
        Ok(())
    }

    pub async fn record_skill_execution(
        &self,
        sid: &str,
        tid: Option<&str>,
        success: bool,
        et: u64,
        ia: Option<&serde_json::Value>,
        or: Option<&serde_json::Value>,
    ) -> Result<()> {
        trajectory_skill_executions::ActiveModel {
            id: Set(Uuid::new_v4().to_string()),
            skill_id: Set(sid.to_string()),
            trajectory_id: Set(tid.map(|s| s.to_string())),
            success: Set(success as i32),
            execution_time_ms: Set(et as i64),
            created_at: Set(Utc::now().to_rfc3339()),
            input_args: Set(ia.map(|v| serde_json::to_string(v).unwrap_or_default())),
            output_result: Set(or.map(|v| serde_json::to_string(v).unwrap_or_default())),
        }
        .insert(self.db.as_ref())
        .await?;

        // P1: 同步更新 skill 的统计字段（total_usages/success_rate/avg_execution_time/
        // consecutive_failures/last_failure_at）。这里直接在数据库层做增量更新，
        // 避免先 read 再 write 的竞态。读取 skill 时由 model_to_skill 还原这些字段。
        let now = Utc::now();
        let now_str = now.to_rfc3339();
        // 累加使用次数
        let _ = trajectory_skills::Entity::update_many()
            .col_expr(
                trajectory_skills::Column::UsageCount,
                sea_orm::sea_query::Expr::col(trajectory_skills::Column::UsageCount).add(1),
            )
            .col_expr(
                trajectory_skills::Column::AvgExecutionTimeMs,
                sea_orm::sea_query::Expr::col(trajectory_skills::Column::AvgExecutionTimeMs)
                    .add(et as i64)
                    .div(2),
            )
            .col_expr(
                trajectory_skills::Column::UpdatedAt,
                sea_orm::sea_query::Expr::value(now_str.clone()),
            )
            .filter(trajectory_skills::Column::Id.eq(sid))
            .exec(self.db.as_ref())
            .await;

        // 根据 success 更新 success_rate 和 consecutive_failures
        // 简化处理：success=true 视为成功（清零失败计数），false 视为失败（累加）
        if success {
            let _ = trajectory_skills::Entity::update_many()
                .col_expr(
                    trajectory_skills::Column::ConsecutiveFailures,
                    sea_orm::sea_query::Expr::value(0i32),
                )
                .col_expr(
                    trajectory_skills::Column::SuccessRate,
                    // 简化：success=true 时把 success_rate 推向 1.0（保留旧值 70% + 30%）
                    sea_orm::sea_query::Expr::col(trajectory_skills::Column::SuccessRate)
                        .mul(0.7)
                        .add(0.3),
                )
                .filter(trajectory_skills::Column::Id.eq(sid))
                .exec(self.db.as_ref())
                .await;
        } else {
            let _ = trajectory_skills::Entity::update_many()
                .col_expr(
                    trajectory_skills::Column::ConsecutiveFailures,
                    sea_orm::sea_query::Expr::col(trajectory_skills::Column::ConsecutiveFailures)
                        .add(1),
                )
                .col_expr(
                    trajectory_skills::Column::LastFailureAt,
                    sea_orm::sea_query::Expr::value(now_str),
                )
                .col_expr(
                    trajectory_skills::Column::SuccessRate,
                    // 简化：success=false 时把 success_rate 推向 0.0（保留旧值 70%）
                    sea_orm::sea_query::Expr::col(trajectory_skills::Column::SuccessRate).mul(0.7),
                )
                .filter(trajectory_skills::Column::Id.eq(sid))
                .exec(self.db.as_ref())
                .await;
        }

        Ok(())
    }

    // ── Entities (stored in knowledge_entities table, v101 merge) ──

    /// Sentinel KB id —— **值不在此重复**，转发 harness 的权威定义
    /// （AGENTS.md 禁区 12）。用关联常量转发是为了让调用点继续写
    /// `Self::TRAJECTORY_KB_ID`。
    const TRAJECTORY_KB_ID: &str = axagent_harness::constants::sentinel::TRAJECTORY_KB_ID;

    pub async fn save_entity(&self, e: &Entity) -> Result<()> {
        use knowledge_entities::Column;
        use sea_orm::sea_query::OnConflict;

        let now_ts = Utc::now().timestamp();
        knowledge_entities::Entity::insert(knowledge_entities::ActiveModel {
            id: Set(e.id.clone()),
            knowledge_base_id: Set(Self::TRAJECTORY_KB_ID.to_string()),
            name: Set(e.name.clone()),
            entity_type: Set(serde_json::to_string(&e.entity_type).unwrap_or_default()),
            description: Set(None),
            source_path: Set(String::new()),
            source_language: Set(None),
            properties: Set(serde_json::Value::Object(
                e.properties.iter().map(|(k, v)| (k.clone(), v.clone())).collect(),
            )),
            lifecycle: Set(None),
            behaviors: Set(None),
            metadata: Set(None),
            created_at: Set(now_ts),
            updated_at: Set(now_ts),
            aliases: Set(serde_json::to_string(&e.aliases).unwrap_or_else(|_| "[]".to_string())),
            mention_count: Set(e.mention_count as i32),
            confidence: Set(e.confidence),
            first_seen_at: Set(Some(e.first_seen_at.to_rfc3339())),
            last_seen_at: Set(Some(e.last_seen_at.to_rfc3339())),
            source_type: Set(String::from("knowledge_base")),
            source_id: Set(String::new()),
            node_type: Set(String::from(
                axagent_harness::knowledge_graph::GraphNodeType::Entity.as_str(),
            )),
            external_id: Set(None),
        })
        .on_conflict(
            OnConflict::column(knowledge_entities::Column::Id)
                .update_columns([
                    Column::Name,
                    Column::LastSeenAt,
                    Column::MentionCount,
                    Column::Confidence,
                    Column::UpdatedAt,
                ])
                .to_owned(),
        )
        .exec(self.db.as_ref())
        .await?;
        Ok(())
    }

    pub async fn get_entity(&self, id: &str) -> Result<Option<Entity>> {
        Ok(knowledge_entities::Entity::find_by_id(id)
            .one(self.db.as_ref())
            .await?
            .map(|e| ke_to_entity(&e)))
    }

    pub async fn get_all_entities(&self) -> Result<Vec<Entity>> {
        Ok(knowledge_entities::Entity::find()
            .filter(knowledge_entities::Column::Lifecycle.is_null())
            .filter(knowledge_entities::Column::KnowledgeBaseId.eq(Self::TRAJECTORY_KB_ID))
            .order_by_desc(knowledge_entities::Column::UpdatedAt)
            .all(self.db.as_ref())
            .await?
            .iter()
            .map(ke_to_entity)
            .collect())
    }

    pub async fn search_entities(&self, query: &str, limit: usize) -> Result<Vec<Entity>> {
        let pattern = format!("%{}%", query);
        Ok(knowledge_entities::Entity::find()
            .filter(knowledge_entities::Column::Lifecycle.is_null())
            .filter(
                knowledge_entities::Column::KnowledgeBaseId
                    .eq(Self::TRAJECTORY_KB_ID)
                    .and(knowledge_entities::Column::Name.like(&pattern)),
            )
            .all(self.db.as_ref())
            .await?
            .iter()
            .take(limit)
            .map(ke_to_entity)
            .collect())
    }

    /// P1-3: 删除实体时级联删除其所有 relationships
    pub async fn delete_entity(&self, id: &str) -> Result<()> {
        let txn = self.db.begin().await?;
        knowledge_relations::Entity::delete_many()
            .filter(
                knowledge_relations::Column::SourceEntityId
                    .eq(id)
                    .or(knowledge_relations::Column::TargetEntityId.eq(id)),
            )
            .exec(&txn)
            .await?;
        knowledge_entities::Entity::delete_by_id(id).exec(&txn).await?;
        txn.commit().await?;
        Ok(())
    }

    // ── Relationships (stored in knowledge_relations table, v101 merge) ──

    /// 保存一条关系（按 `rel.id` upsert）。
    ///
    /// # id 契约（调用方必须遵守 —— 2026-09-17 补文档）
    ///
    /// 本函数的幂等性**完全**来自 `ON CONFLICT (id)`：它只在「同一逻辑关系每次传入
    /// 同一个 `id`」时才会触发。由此有两种**合法但语义不同**的用法：
    ///
    /// - **就地改写**：传入**已存在行**的 id —— 典型是实体合并时把关系从被合并实体
    ///   重定向到保留实体（`memory_providers/service.rs` 的合并循环传的是 `rel.clone()`
    ///   的原 id）⇒ 冲突命中原行 ⇒ 该行被更新。
    /// - **按自然键去重**：id 由自然键派生
    ///   （`axagent_harness::knowledge_graph::stable_relation_id`）⇒ 同一
    ///   `(kb, source, target, type)` 反复写入只留一行。
    ///
    /// ⚠ **不要每次新造随机 id**（如 `Uuid::new_v4()`）：冲突键就是本次刚生成的那个值，
    /// 永不与任何已存在行相同 ⇒ `on_conflict` 静默失效、退化为纯 `INSERT` ⇒ 调用方
    /// 每跑一轮就为同一逻辑关系再插一行，**无界增长且无任何报错**。实测形态见
    /// `docs/plans/PLAN-memory-kb-reflow-id-space.md` §5d 类 C（同一缺陷家族：
    /// `trajectory_patterns` 2038 行却只有 3 个 `name`）。
    pub async fn save_relationship(&self, rel: &Relationship) -> Result<()> {
        use knowledge_relations::Column;
        use sea_orm::sea_query::OnConflict;

        let now_ts = Utc::now().timestamp();
        knowledge_relations::Entity::insert(knowledge_relations::ActiveModel {
            id: Set(rel.id.clone()),
            knowledge_base_id: Set(Self::TRAJECTORY_KB_ID.to_string()),
            source_entity_id: Set(rel.source_id.clone()),
            target_entity_id: Set(rel.target_id.clone()),
            // D5（2026-09-14）：写**裸字面量**（`part_of`），不再 `serde_json::to_string`。
            //
            // 历史：此处曾用 `serde_json::to_string(&rel.relation_type)` ⇒ 落库是**带引号**的
            // `"part_of"`。该编码自成一体（读端用 `from_str(&format!("\"{}\"", …))` 反解），
            // 但**与裸字面量不互通**：任何按字面量过滤的读方（`GraphEnhancedSearchInput::
            // relation_type_filters`、`dao::repo::knowledge_graph` 排除因果边）都匹配不到它。
            // 收敛后整列只有一种编码；读取端（`parse_stored_relation_type`）永久兼容两态，
            // 故**存量数据无需迁移**（本机实测带引号行 = 0，但已发布版本写过）。
            relation_type: Set(rel.relation_type.to_string()),
            description: Set(None),
            properties: Set(if rel.properties.is_empty() {
                None
            } else {
                Some(serde_json::Value::Object(
                    rel.properties.iter().map(|(k, v)| (k.clone(), v.clone())).collect(),
                ))
            }),
            metadata: Set(None),
            created_at: Set(now_ts),
            updated_at: Set(now_ts),
            weight: Set(rel.weight),
            source_type: Set(String::from("knowledge_base")),
            source_id: Set(String::new()),
        })
        .on_conflict(
            OnConflict::column(knowledge_relations::Column::Id)
                .update_columns([Column::Weight, Column::UpdatedAt])
                .to_owned(),
        )
        .exec(self.db.as_ref())
        .await?;
        Ok(())
    }

    pub async fn get_relationships_by_entity(&self, eid: &str) -> Result<Vec<Relationship>> {
        Ok(knowledge_relations::Entity::find()
            .filter(
                knowledge_relations::Column::SourceEntityId
                    .eq(eid)
                    .or(knowledge_relations::Column::TargetEntityId.eq(eid)),
            )
            .all(self.db.as_ref())
            .await?
            .iter()
            .map(kr_to_relationship)
            .collect())
    }

    pub async fn get_all_relationships(&self) -> Result<Vec<Relationship>> {
        Ok(knowledge_relations::Entity::find()
            .filter(knowledge_relations::Column::KnowledgeBaseId.eq(Self::TRAJECTORY_KB_ID))
            .order_by_desc(knowledge_relations::Column::CreatedAt)
            .all(self.db.as_ref())
            .await?
            .iter()
            .map(kr_to_relationship)
            .collect())
    }

    pub async fn delete_relationship(&self, id: &str) -> Result<()> {
        knowledge_relations::Entity::delete_by_id(id).exec(self.db.as_ref()).await?;
        Ok(())
    }

    // ── Sessions ──

    pub async fn save_session(&self, s: &TrajectorySession) -> Result<()> {
        trajectory_sessions::Entity::insert(trajectory_sessions::ActiveModel {
            id: Set(s.id.clone()),
            title: Set(s.title.clone()),
            platform: Set(s.platform.clone()),
            user_id: Set(s.user_id.clone()),
            model: Set(s.model.clone()),
            system_prompt: Set(s.system_prompt.clone()),
            created_at: Set(s.created_at.to_rfc3339()),
            updated_at: Set(s.updated_at.to_rfc3339()),
            parent_session_id: Set(s.parent_session_id.clone()),
            token_input: Set(s.token_input),
            token_output: Set(s.token_output),
        })
        .on_conflict(
            sea_orm::sea_query::OnConflict::column(trajectory_sessions::Column::Id)
                .update_columns([
                    trajectory_sessions::Column::Title,
                    trajectory_sessions::Column::UpdatedAt,
                    trajectory_sessions::Column::TokenInput,
                    trajectory_sessions::Column::TokenOutput,
                ])
                .to_owned(),
        )
        .exec(self.db.as_ref())
        .await?;
        Ok(())
    }

    pub async fn get_session(&self, id: &str) -> Result<Option<TrajectorySession>> {
        Ok(trajectory_sessions::Entity::find_by_id(id)
            .one(self.db.as_ref())
            .await?
            .map(|s| model_to_sess(&s)))
    }

    pub async fn get_all_sessions(&self) -> Result<Vec<TrajectorySession>> {
        Ok(trajectory_sessions::Entity::find()
            .order_by_desc(trajectory_sessions::Column::UpdatedAt)
            .all(self.db.as_ref())
            .await?
            .iter()
            .map(model_to_sess)
            .collect())
    }

    pub async fn update_session(&self, id: &str, updates: &SessionUpdate) -> Result<()> {
        let m = trajectory_sessions::Entity::find_by_id(id)
            .one(self.db.as_ref())
            .await?
            .context("Session not found")?;
        let mut am: trajectory_sessions::ActiveModel = m.into_active_model();
        if let Some(ref t) = updates.title {
            am.title = Set(t.clone());
        }
        if let Some(ti) = updates.token_input {
            am.token_input = Set(ti);
        }
        if let Some(to) = updates.token_output {
            am.token_output = Set(to);
        }
        am.updated_at = Set(Utc::now().to_rfc3339());
        am.update(self.db.as_ref()).await?;
        Ok(())
    }

    /// P1-3: 级联删除 session → 该 session 的所有 trajectories
    /// (trajectories 通过 session_id 关联；trajectory_steps/rewards 由 delete_trajectory 自身级联)
    pub async fn delete_session(&self, id: &str) -> Result<()> {
        // 先查出该 session 的所有 trajectory
        let traj_ids: Vec<String> = trajectories::Entity::find()
            .filter(trajectories::Column::SessionId.eq(id))
            .all(self.db.as_ref())
            .await?
            .into_iter()
            .map(|t| t.id)
            .collect();
        // 级联删除每条 trajectory
        for tid in &traj_ids {
            let _ = self.delete_trajectory(tid).await;
        }
        // 删除该 session 的所有 messages
        trajectory_messages::Entity::delete_many()
            .filter(trajectory_messages::Column::SessionId.eq(id))
            .exec(self.db.as_ref())
            .await?;
        // 最后删除 session 自身
        trajectory_sessions::Entity::delete_by_id(id).exec(self.db.as_ref()).await?;
        Ok(())
    }

    // ── Messages ──

    pub async fn save_message(&self, msg: &Message) -> Result<()> {
        trajectory_messages::ActiveModel {
            id: Set(msg.id.clone()),
            session_id: Set(msg.session_id.clone()),
            role: Set(msg.role.clone()),
            content: Set(msg.content.clone()),
            tool_calls: Set(msg.tool_calls.clone()),
            tool_results: Set(msg.tool_results.clone()),
            usage: Set(msg.usage.clone()),
            created_at: Set(msg.created_at.to_rfc3339()),
        }
        .insert(self.db.as_ref())
        .await?;
        let _ = self.index_message_fts(msg).await;
        Ok(())
    }

    pub async fn get_messages_by_session(&self, sid: &str) -> Result<Vec<Message>> {
        Ok(trajectory_messages::Entity::find()
            .filter(trajectory_messages::Column::SessionId.eq(sid))
            .order_by_asc(trajectory_messages::Column::CreatedAt)
            .all(self.db.as_ref())
            .await?
            .iter()
            .map(model_to_msg)
            .collect())
    }

    pub async fn search_messages(&self, query: &str, limit: usize) -> Result<Vec<Message>> {
        Ok(trajectory_messages::Entity::find()
            .filter(trajectory_messages::Column::Content.like(format!("%{}%", query)))
            .order_by_desc(trajectory_messages::Column::CreatedAt)
            .all(self.db.as_ref())
            .await?
            .iter()
            .take(limit)
            .map(model_to_msg)
            .collect())
    }

    // ── Memories (stored in memory_items table, v101 merge) ──

    /// Sentinel 命名空间 id —— 同上，转发 harness 权威定义，值不重复。
    const TRAJECTORY_MEM_NS_ID: &str = axagent_harness::constants::sentinel::TRAJECTORY_MEM_NS_ID;

    pub async fn get_all_memories(&self) -> Result<Vec<crate::memory::MemoryEntry>> {
        use sea_orm::QueryFilter;
        Ok(memory_items::Entity::find()
            .filter(memory_items::Column::NamespaceId.eq(Self::TRAJECTORY_MEM_NS_ID))
            .all(self.db.as_ref())
            .await?
            .into_iter()
            .map(|m| crate::memory::MemoryEntry {
                id: m.id,
                content: m.content,
                memory_type: m.title, // memory_items.title maps to memory_type
                tier: crate::memory::MemoryTier::from_str(&m.tier),
                importance: m.importance,
                access_count: m.access_count as u64,
                last_accessed: m.last_accessed.unwrap_or(0),
                decay_rate: m.decay_rate,
                created_at: 0,
                updated_at: m.updated_at.parse().unwrap_or(0),
                expires_at: m.expires_at,
                nature: crate::memory::MemoryNature::from_str(&m.memory_nature),
                provenance: Some(crate::memory::MemoryProvenance {
                    conversation_id: m.source_conversation_id,
                    message_id: m.source_message_id,
                    extraction_method: "unknown".to_string(),
                }),
                tags: serde_json::from_str(&m.tags).unwrap_or_default(),
                namespace_id: Some(m.namespace_id),
            })
            .collect())
    }

    pub async fn save_memory(&self, mem: &crate::memory::MemoryEntry) -> Result<()> {
        use memory_items::Column;
        use sea_orm::sea_query::OnConflict;

        let source_conv_id = mem.provenance.as_ref().and_then(|p| p.conversation_id.clone());
        let source_msg_id = mem.provenance.as_ref().and_then(|p| p.message_id.clone());
        let now = chrono::Utc::now().timestamp_millis().to_string();

        memory_items::Entity::insert(memory_items::ActiveModel {
            id: Set(mem.id.clone()),
            namespace_id: Set(Self::TRAJECTORY_MEM_NS_ID.to_string()),
            title: Set(mem.memory_type.clone()),
            content: Set(mem.content.clone()),
            source: Set(source_conv_id.clone().unwrap_or_else(|| "trajectory".to_string())),
            // 状态取 `pending`：这条记忆**确实**等待向量化，推进责任在
            // `IndexJobService` 的孤儿扫描器（它按 index_status='pending' 扫全表并为
            // 无活跃作业的条目补入队）。本层（DAO/entity 层）拿不到 AppHandle，
            // 无法自行入队 —— 这正是「写入 pending 却不入队」这一架构缺口的来源，
            // 也正因如此，兜底必须做在扫描器侧而不是每个写入点。
            //
            // 旧值 `"ready"` 是一句无法成立的断言：它声称"向量索引已完成"，而该记录
            // 从未被向量化过。后果不只是显示错误 —— 状态机里的 `ready` 意味着"无需再
            // 处理"，扫描器会跳过它，于是这条记忆永久缺失向量表示且永不重试。
            //
            // 若 `__sys_trajectory_memory__` 命名空间未配置 embedding provider，
            // 扫描器会把它诚实降为 `skipped` 并写明原因，不会反复入队。
            index_status: Set(axagent_harness::constants::status::PENDING.to_string()),
            index_error: Set(None),
            updated_at: Set(now.clone()),
            tier: Set(mem.tier.as_str().to_string()),
            importance: Set(mem.importance),
            access_count: Set(mem.access_count as i32),
            last_accessed: Set(Some(mem.last_accessed)),
            decay_rate: Set(mem.decay_rate),
            expires_at: Set(mem.expires_at),
            source_conversation_id: Set(source_conv_id),
            source_message_id: Set(source_msg_id),
            memory_nature: Set(mem.nature.as_str().to_string()),
            tags: Set(serde_json::to_string(&mem.tags).unwrap_or_else(|_| "[]".to_string())),
            // v108: 自进化闭环 — trajectory 存储默认未确认 + 空适用范围
            applicability_tags: Set("[]".to_string()),
            confirmed: Set(0),
        })
        .on_conflict(
            OnConflict::column(memory_items::Column::Id)
                .update_columns([
                    Column::Content,
                    Column::UpdatedAt,
                    Column::Tier,
                    Column::Importance,
                    Column::AccessCount,
                    Column::LastAccessed,
                    Column::DecayRate,
                    Column::ExpiresAt,
                    Column::MemoryNature,
                    Column::Tags,
                    Column::SourceConversationId,
                    Column::SourceMessageId,
                ])
                .to_owned(),
        )
        .exec(self.db.as_ref())
        .await?;
        Ok(())
    }

    /// P1-3: 删除 memory 时也清理 FTS 索引
    pub async fn delete_memory(&self, id: &str) -> Result<()> {
        memory_items::Entity::delete_by_id(id).exec(self.db.as_ref()).await?;
        let _ = self.delete_memory_fts(id).await;
        Ok(())
    }

    // ── Learned Patterns ──

    pub async fn save_learning_pattern(&self, p: &Pattern) -> Result<()> {
        trajectory_learned_patterns::Entity::insert(trajectory_learned_patterns::ActiveModel {
            id: Set(p.id.clone()),
            pattern: Set(p.pattern.clone()),
            pattern_type: Set(p.pattern_type.clone()),
            success: Set(p.success),
            failure: Set(p.failure),
            last_used: Set(p.last_used.to_rfc3339()),
            created_at: Set(p.created_at.to_rfc3339()),
            metadata: Set(p.metadata.clone()),
        })
        .on_conflict(
            sea_orm::sea_query::OnConflict::column(trajectory_learned_patterns::Column::Id)
                .update_columns([
                    trajectory_learned_patterns::Column::Success,
                    trajectory_learned_patterns::Column::Failure,
                    trajectory_learned_patterns::Column::LastUsed,
                ])
                .to_owned(),
        )
        .exec(self.db.as_ref())
        .await?;
        Ok(())
    }

    pub async fn get_patterns_list(&self) -> Result<Vec<Pattern>> {
        Ok(trajectory_learned_patterns::Entity::find()
            .all(self.db.as_ref())
            .await?
            .iter()
            .map(|p| Pattern {
                id: p.id.clone(),
                pattern: p.pattern.clone(),
                pattern_type: p.pattern_type.clone(),
                success: p.success,
                failure: p.failure,
                last_used: chrono::DateTime::parse_from_rfc3339(&p.last_used)
                    .map(|dt| dt.with_timezone(&Utc))
                    .unwrap_or_else(|_| Utc::now()),
                created_at: chrono::DateTime::parse_from_rfc3339(&p.created_at)
                    .map(|dt| dt.with_timezone(&Utc))
                    .unwrap_or_else(|_| Utc::now()),
                metadata: p.metadata.clone(),
            })
            .collect())
    }

    pub async fn update_pattern_stats(&self, id: &str, sd: i32, fd: i32) -> Result<()> {
        if let Some(m) =
            trajectory_learned_patterns::Entity::find_by_id(id).one(self.db.as_ref()).await?
        {
            let mut am: trajectory_learned_patterns::ActiveModel = m.into_active_model();
            am.success = Set(am.success.take().unwrap_or(0) + sd);
            am.failure = Set(am.failure.take().unwrap_or(0) + fd);
            am.last_used = Set(Utc::now().to_rfc3339());
            am.update(self.db.as_ref()).await?;
        }
        Ok(())
    }

    // ── Preferences ──

    pub async fn save_preference(&self, pref: &Preference) -> Result<()> {
        trajectory_preferences::Entity::insert(trajectory_preferences::ActiveModel {
            id: Set(pref.id.clone()),
            key: Set(pref.key.clone()),
            value: Set(pref.value.clone()),
            confidence: Set(pref.confidence),
            updated_at: Set(pref.updated_at.to_rfc3339()),
        })
        .on_conflict(
            sea_orm::sea_query::OnConflict::column(trajectory_preferences::Column::Key)
                .update_columns([
                    trajectory_preferences::Column::Value,
                    trajectory_preferences::Column::Confidence,
                    trajectory_preferences::Column::UpdatedAt,
                ])
                .to_owned(),
        )
        .exec(self.db.as_ref())
        .await?;
        Ok(())
    }

    pub async fn get_preferences_list(&self) -> Result<Vec<Preference>> {
        Ok(trajectory_preferences::Entity::find()
            .all(self.db.as_ref())
            .await?
            .iter()
            .map(|p| Preference {
                id: p.id.clone(),
                key: p.key.clone(),
                value: p.value.clone(),
                confidence: p.confidence,
                updated_at: chrono::DateTime::parse_from_rfc3339(&p.updated_at)
                    .map(|dt| dt.with_timezone(&Utc))
                    .unwrap_or_else(|_| Utc::now()),
            })
            .collect())
    }

    pub async fn update_preference_by_key(&self, key: &str, updates: &Preference) -> Result<()> {
        if let Some(m) = trajectory_preferences::Entity::find()
            .filter(trajectory_preferences::Column::Key.eq(key))
            .one(self.db.as_ref())
            .await?
        {
            let mut am: trajectory_preferences::ActiveModel = m.into_active_model();
            am.value = Set(updates.value.clone());
            am.confidence = Set(updates.confidence);
            am.updated_at = Set(Utc::now().to_rfc3339());
            am.update(self.db.as_ref()).await?;
        }
        Ok(())
    }

    // ── Utilities ──

    pub async fn get_trajectory_stats(&self) -> Result<TrajectoryStatistics> {
        let trajs = self.get_trajectories(None).await?;
        let total = trajs.len();
        if total == 0 {
            return Ok(TrajectoryStatistics {
                total_trajectories: 0,
                total_sessions: 0,
                total_patterns: 0,
                avg_quality: 0.0,
                avg_value_score: 0.0,
                success_rate: 0.0,
                recent_trajectories: 0,
            });
        }
        let mut tq = 0.0;
        let mut tv = 0.0;
        let mut sc = 0;
        for t in &trajs {
            tq += t.quality.overall;
            tv += t.value_score;
            if t.outcome == TrajectoryOutcome::Success {
                sc += 1;
            }
        }
        Ok(TrajectoryStatistics {
            total_trajectories: total,
            total_sessions: 0,
            total_patterns: 0,
            avg_quality: tq / total as f64,
            avg_value_score: tv / total as f64,
            success_rate: sc as f64 / total as f64,
            recent_trajectories: total.min(10),
        })
    }

    pub async fn export_trajectories(
        &self,
        opts: &TrajectoryExportOptions,
    ) -> Result<Vec<RLTrainingEntry>> {
        Ok(self
            .query_trajectories(&TrajectoryQuery {
                session_id: None,
                user_id: None,
                topic: None,
                min_quality: opts.min_quality,
                min_value_score: opts.min_value_score,
                outcome: opts.outcome_filter,
                time_range: None,
                limit: opts.limit,
            })
            .await?
            .into_iter()
            .map(|t| axagent_harness::trajectory_scorer::TrajectoryScorer::export_as_rl(&t))
            .collect())
    }

    /// P0-2: 修复嵌套 block_on - 全部用 async 查询
    pub async fn search_trajectories(&self, fts_query: &FTS5Query) -> Result<Vec<String>> {
        // 优先使用 FTS5 全文搜索，不可用时降级为 LIKE 查询
        if let Some(ref fts) = self.fts_searcher {
            let mut query = fts_query.clone();
            query.filter_type = Some("trajectories_fts".to_string());
            match fts.search(query).await {
                Ok(results) if !results.is_empty() => {
                    return Ok(results.into_iter().map(|r| r.id).collect());
                },
                _ => {},
            }
        }
        // 降级：直接 async 查询
        let pattern = format!("%{}%", fts_query.query);
        Ok(trajectories::Entity::find()
            .filter(
                trajectories::Column::Topic
                    .like(&pattern)
                    .or(trajectories::Column::Summary.like(&pattern)),
            )
            .all(self.db.as_ref())
            .await?
            .into_iter()
            .take(fts_query.limit)
            .map(|t| t.id)
            .collect())
    }

    pub fn init_memory_tables(&self) -> Result<()> {
        info!("Memory tables initialized");
        Ok(())
    }
    pub async fn get_all_skills(&self) -> Result<Vec<Skill>> {
        self.get_skills().await
    }
    pub async fn get_all_patterns(&self) -> Result<Vec<TrajectoryPattern>> {
        self.get_patterns().await
    }
    pub async fn get_statistics(&self) -> Result<TrajectoryStatistics> {
        self.get_trajectory_stats().await
    }

    // FTS delegates
    pub async fn create_fts_tables(&self) -> Result<()> {
        if let Some(ref fts) = self.fts_searcher {
            fts.create_fts_tables().await
        } else {
            Ok(())
        }
    }
    pub async fn search_fts(&self, query: FTS5Query) -> Result<Vec<FTS5Result>> {
        if let Some(ref fts) = self.fts_searcher {
            fts.search(query).await
        } else {
            Ok(Vec::new())
        }
    }
    pub async fn index_trajectory_fts(&self, t: &Trajectory) -> Result<()> {
        if let Some(ref fts) = self.fts_searcher {
            fts.index_trajectory(t, &t.session_id).await
        } else {
            Ok(())
        }
    }
    pub async fn index_skill_fts(&self, skill: &Skill) -> Result<()> {
        if let Some(ref fts) = self.fts_searcher {
            fts.index_skill(
                &skill.id,
                &skill.name,
                &skill.description,
                &skill.content,
                &skill.category,
                &skill.tags,
            )
            .await
        } else {
            Ok(())
        }
    }
    pub async fn index_message_fts(&self, msg: &Message) -> Result<()> {
        if let Some(ref fts) = self.fts_searcher {
            fts.index_message(msg).await
        } else {
            Ok(())
        }
    }
    pub async fn index_memory_fts(
        &self,
        id: &str,
        mt: &str,
        content: &str,
        entities: &[String],
    ) -> Result<()> {
        if let Some(ref fts) = self.fts_searcher {
            fts.index_memory(id, mt, content, entities).await
        } else {
            Ok(())
        }
    }
    /// 从 FTS 索引移除一条记忆。
    ///
    /// **修复点**：原实现是 `let _ = fts.delete_from_fts("memory_items_fts", id).await;`
    /// —— 内层错误被丢弃，本函数**恒返回 `Ok(())`**。而 `delete_from_fts` 内部
    /// 已经处理了「该行不在索引中」的正常情形（查不到 rowid 即静默返回），
    /// 所以任何 `Err` 都是真实故障（表损坏 / SQL 失败），不能吞。
    ///
    /// 吞掉的后果有两个，都不可见：
    ///   ① 四个调用点（含淘汰 / 过期 / 衰减清理）的 `tracing::warn!` 成为**不可达死代码**；
    ///   ② 索引残留无人知晓 —— 搜索会返回一条**已经删除的记忆**。
    pub async fn delete_memory_fts(&self, id: &str) -> Result<()> {
        if let Some(ref fts) = self.fts_searcher {
            fts.delete_from_fts("memory_items_fts", id).await?;
        }
        Ok(())
    }

    /// FTS5 索引健康状态（**读路径**）。
    ///
    /// 没有这个出口，`health_check` 本身就是死代码 —— 索引是否落后于主数据
    /// 将永远只存在于 `tracing::warn!` 里，UI 与诊断入口都看不到。
    ///
    /// `fts_searcher` 为 `None` 时返回 `available: false` 的健康报告，**不是** `Err`：
    /// PG 后端下这就是设计内的降级（`src/init/state.rs:151` 有明确注释），
    /// 用错误表达它会让上层把「已知的降级」当成「故障」处理。
    /// 但也不能像修复前那样返回一份全零报告 —— 那会让调用方以为「索引是空的，
    /// 一切正常」，而事实是这个存储的 FTS 读写**全部是 no-op**。
    pub async fn fts_health(&self) -> Result<FTS5Health> {
        match self.fts_searcher {
            Some(ref fts) => fts.health_check(Some(Self::TRAJECTORY_MEM_NS_ID)).await,
            None => Ok(FTS5Health::unavailable(
                "FTS5 未挂载（fts_searcher 为 None）。\
                 PostgreSQL 后端下即为设计内降级，此时向量检索可用但全文检索整体为空；\
                 基表的 tsvector 列已在 v001 预留，PG 侧全文检索尚未实现",
            )),
        }
    }

    /// 重建 FTS5 索引（仅限具备回填路径的表，详见 `fts5::rebuild_indexes` 文档）。
    pub async fn rebuild_fts_indexes(&self) -> Result<FTS5RebuildReport> {
        match self.fts_searcher {
            Some(ref fts) => fts.rebuild_indexes(Self::TRAJECTORY_MEM_NS_ID).await,
            None => anyhow::bail!("FTS5 未挂载（fts_searcher 为 None），无法重建索引"),
        }
    }

    pub async fn delete_skill_fts(&self, id: &str) -> Result<()> {
        if let Some(ref fts) = self.fts_searcher {
            fts.delete_from_fts("trajectory_skills_fts", id).await
        } else {
            Ok(())
        }
    }
    pub async fn delete_trajectory_fts(&self, id: &str) -> Result<()> {
        if let Some(ref fts) = self.fts_searcher {
            fts.delete_from_fts("trajectories_fts", id).await
        } else {
            Ok(())
        }
    }
    pub async fn optimize_fts(&self) -> Result<()> {
        if let Some(ref fts) = self.fts_searcher {
            fts.optimize().await
        } else {
            Ok(())
        }
    }

    /// 对 FTS5 索引执行 VACUUM，回收已删除记录占用的磁盘空间。
    /// 与 `optimize_fts`（合并 segments）互补，通常在 cleanup 后调用。
    pub async fn vacuum_fts(&self) -> Result<()> {
        if let Some(ref fts) = self.fts_searcher {
            fts.vacuum().await
        } else {
            Ok(())
        }
    }
}

// ── Model conversion helpers ──

fn model_to_trajectory(
    m: &trajectories::Model,
    steps: Vec<TrajectoryStep>,
    rewards: Vec<RewardSignal>,
) -> Trajectory {
    Trajectory {
        id: m.id.clone(),
        session_id: m.session_id.clone(),
        user_id: m.user_id.clone(),
        agent_name: m.agent_name.clone(),
        topic: m.topic.clone(),
        summary: m.summary.clone(),
        outcome: serde_json::from_str(&format!("\"{}\"", m.outcome))
            .unwrap_or(TrajectoryOutcome::Success),
        duration_ms: m.duration_ms as u64,
        quality: crate::trajectory::TrajectoryQuality {
            overall: m.quality_overall,
            task_completion: m.quality_task_completion,
            tool_efficiency: m.quality_tool_efficiency,
            reasoning_quality: m.quality_reasoning_quality,
            user_satisfaction: m.quality_user_satisfaction,
        },
        value_score: m.value_score,
        patterns: serde_json::from_str(&m.patterns).unwrap_or_default(),
        steps,
        rewards,
        created_at: chrono::DateTime::parse_from_rfc3339(&m.created_at)
            .map(|dt| dt.with_timezone(&Utc))
            .unwrap_or_else(|_| Utc::now()),
        replay_count: m.replay_count as u32,
        last_replay_at: m.last_replay_at.as_ref().and_then(|s| {
            chrono::DateTime::parse_from_rfc3339(s).map(|dt| dt.with_timezone(&Utc)).ok()
        }),
    }
}

/// `trajectory_rewards.created_at` 是 text 列，承载的是 `RewardSignal.timestamp_ms`
/// —— **相对轨迹起点的毫秒偏移，不是日历时间**（理由见 `save_trajectory` 里对该列的注释）。
///
/// 双向前兼容（⇒ 存量零迁移）：
///   · 2026-09-18 起：十进制毫秒字面量（与本文件 `save_memory` 的
///     `timestamp_millis().to_string()` 同款编码）；
///   · 更早：`DateTime::from_timestamp_millis` 的 RFC3339 产物 —— 含把偏移 `56867` 落成
///     `1970-01-01T00:00:56.867Z` 的坏形态。
///
/// 两种形态还原出的毫秒数与写端原值**逐位相同**（毫秒精度在 RFC3339 里有小数秒承载），
/// 所以坏行不必迁移，读端一次兼容两种形态即可。
fn parse_reward_timestamp_ms(raw: &str) -> Option<u64> {
    let s = raw.trim();
    if let Ok(ms) = s.parse::<i64>() {
        // ⚠ 必须写 `Ord::max` 而非 `ms.max(0)`：本文件 `use sea_orm::ExprTrait` 也在作用域里，
        // 该 trait 为同型提供了 `max` ⇒ 点号写法报 `E0034 multiple applicable items in scope`
        // （实测：`cargo fmt --check` 报 EXIT=0 而同一份代码编译不过 —— 格式过 ≠ 编译过）。
        return Some(Ord::max(ms, 0) as u64);
    }
    chrono::DateTime::parse_from_rfc3339(s).ok().map(|dt| Ord::max(dt.timestamp_millis(), 0) as u64)
}

fn model_to_skill(s: &trajectory_skills::Model) -> Skill {
    Skill {
        id: s.id.clone(),
        name: s.name.clone(),
        description: s.description.clone(),
        version: "1.0.0".to_string(),
        content: s.content.clone(),
        category: s.category.clone(),
        tags: serde_json::from_str(&s.tags).unwrap_or_default(),
        platforms: Vec::new(),
        scenarios: serde_json::from_str(&s.scenarios).unwrap_or_default(),
        quality_score: 0.0,
        success_rate: s.success_rate,
        avg_execution_time_ms: s.avg_execution_time_ms as u64,
        total_usages: s.usage_count as u32,
        successful_usages: 0,
        created_at: chrono::DateTime::parse_from_rfc3339(&s.created_at)
            .map(|dt| dt.with_timezone(&Utc))
            .unwrap_or_else(|_| Utc::now()),
        updated_at: chrono::DateTime::parse_from_rfc3339(&s.updated_at)
            .map(|dt| dt.with_timezone(&Utc))
            .unwrap_or_else(|_| Utc::now()),
        last_used_at: None,
        consecutive_failures: Ord::max(s.consecutive_failures, 0) as u32,
        last_failure_at: s.last_failure_at.as_ref().and_then(|t| {
            chrono::DateTime::parse_from_rfc3339(t).map(|dt| dt.with_timezone(&Utc)).ok()
        }),
        metadata: crate::skill::SkillMetadata::default(),
    }
}

fn model_to_traj_pattern(p: &trajectory_patterns::Model) -> TrajectoryPattern {
    TrajectoryPattern {
        id: p.id.clone(),
        name: p.name.clone(),
        description: p.description.clone(),
        pattern_type: p.pattern_type.clone(),
        trajectory_ids: serde_json::from_str(&p.trajectory_ids).unwrap_or_default(),
        frequency: p.frequency as u32,
        success_rate: p.success_rate,
        average_quality: p.average_quality,
        average_value_score: p.average_value_score,
        reward_profile: serde_json::from_str(&p.reward_profile).unwrap_or_default(),
        created_at: chrono::DateTime::parse_from_rfc3339(&p.created_at)
            .map(|dt| dt.with_timezone(&Utc))
            .unwrap_or_else(|_| Utc::now()),
    }
}

// ── 阶段三 T3.1 / T3.5：append-only 证据存储集成测试 ─────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::trajectory::{Trajectory, TrajectoryOutcome, TrajectoryStep};

    /// 构造最小 `Trajectory`（内部自动生成 uuid 主键）。
    fn sample_trajectory(session: &str) -> Trajectory {
        Trajectory::new(
            session.to_string(),
            "user-1".to_string(),
            "测试主题".to_string(),
            "测试摘要".to_string(),
            TrajectoryOutcome::Success,
            1000,
            Vec::<TrajectoryStep>::new(),
        )
    }

    /// `trajectory_rewards.created_at` 读端兼容：**两种历史编码必须还原成同一个毫秒数**。
    ///
    /// 存在理由：该列存过两种编码，且旧编码会把「相对偏移」伪装成 1970 的日期。
    /// 只断言「能读出来」不够 —— 必须断言**逐位等于原偏移**，否则「读出一个值」与
    /// 「读出正确的值」在被测对象上区分不开（同族：测试数据须自证区分力）。
    ///
    /// 期望值一律**先算再写**：`2026-09-06T08:49:50.755+00:00` = `1788684590755`，
    /// 凭感觉会写成 `1783414190755`（差 61 天）。
    #[test]
    fn reward_timestamp_ms_roundtrip_both_encodings() {
        // 新编码：十进制毫秒（= `timestamp_ms` 原样），两种基准都要无损
        assert_eq!(parse_reward_timestamp_ms("56867"), Some(56867), "偏移毫秒应原样还原");
        assert_eq!(
            parse_reward_timestamp_ms("1789355135987"),
            Some(1789355135987),
            "绝对 epoch-ms 也应原样还原（该列历史上两种基准都出现过）"
        );
        // 旧编码：`from_timestamp_millis` 的产物 —— 含把偏移伪装成 1970 的坏形态。
        // 这一条就是「存量零迁移」的依据：坏行的毫秒语义本来就没丢。
        assert_eq!(
            parse_reward_timestamp_ms("1970-01-01T00:00:56.867+00:00"),
            Some(56867),
            "被伪装成 1970 的偏移必须仍还原成 56867"
        );
        assert_eq!(
            parse_reward_timestamp_ms("2026-09-06T08:49:50.755+00:00"),
            Some(1788684590755),
            "旧编码里的真实日期同样按毫秒还原"
        );
        // 强制走兜底分支：不可解析 ⇒ None（由调用方告警，**不得**静默伪造 `now`）
        assert_eq!(parse_reward_timestamp_ms("not-a-timestamp"), None);
        assert_eq!(parse_reward_timestamp_ms(""), None);
        // 列是 text，历史写入可能带空白
        assert_eq!(parse_reward_timestamp_ms("  56867 "), Some(56867));
    }

    /// 阶段三 T3.1：软删除（append-only）——`delete_trajectory` 仅置
    /// `is_invalidated = 1`，证据本体保留；活动查询（get_trajectories）不再可见。
    #[tokio::test]
    async fn delete_trajectory_marks_invalidated_keeps_evidence() {
        let db = axagent_dao::db::create_test_pool().await.expect("测试：创建连接池应成功").conn;
        let storage = TrajectoryStorage::new(Arc::new(db.clone()));
        let t = sample_trajectory("session-1");
        storage.save_trajectory(&t).await.expect("测试：保存轨迹应成功");

        assert_eq!(
            storage.get_trajectories(None).await.expect("测试：查询应成功").len(),
            1,
            "保存后活动查询应可见"
        );

        storage.delete_trajectory(&t.id).await.expect("测试：软删除应成功");

        assert!(
            storage.get_trajectories(None).await.expect("测试：查询应成功").is_empty(),
            "软删除后活动查询应不可见"
        );
        // 证据本体保留（append-only），is_invalidated = 1
        let row = trajectories::Entity::find_by_id(&t.id)
            .one(&db)
            .await
            .expect("测试：查询应成功")
            .expect("证据本体必须保留（append-only，不可物理删除）");
        assert_eq!(row.is_invalidated, 1);
    }

    /// 阶段三 T3.1：重新保存（on_conflict 清除失效标记）恢复活动可见。
    #[tokio::test]
    async fn resave_trajectory_reenables_invalidated_evidence() {
        let db = axagent_dao::db::create_test_pool().await.expect("测试：创建连接池应成功").conn;
        let storage = TrajectoryStorage::new(Arc::new(db.clone()));
        let t = sample_trajectory("session-1");
        storage.save_trajectory(&t).await.expect("测试：保存轨迹应成功");
        storage.delete_trajectory(&t.id).await.expect("测试：软删除应成功");

        storage.save_trajectory(&t).await.expect("测试：重新保存应成功");

        assert_eq!(
            storage.get_trajectories(None).await.expect("测试：查询应成功").len(),
            1,
            "重新保存应重新启用轨迹"
        );
        let row = trajectories::Entity::find_by_id(&t.id)
            .one(&db)
            .await
            .expect("测试：查询应成功")
            .expect("轨迹应存在");
        assert_eq!(row.is_invalidated, 0);
    }

    /// C-#2 回归（2026-09-17）：`save_pattern` 的幂等性必须**真的**生效。
    ///
    /// 修前 `.on_conflict(Column::Id)` 打的是 `TrajectoryPattern::new` 每次新生成的
    /// `Uuid::new_v4()` ⇒ 冲突永不发生 ⇒ 同一模式每次学习都插新行
    /// （生产实测：`trajectory_patterns` 2038 行 / 仅 3 个不同 `name`，
    ///  `tool-CapabilityView` 一名占 1019 行、当天仍在增）。
    #[tokio::test]
    async fn save_pattern_is_idempotent_on_natural_key() {
        let db = axagent_dao::db::create_test_pool().await.expect("测试：创建连接池应成功").conn;
        let storage = TrajectoryStorage::new(Arc::new(db.clone()));

        let mut p = TrajectoryPattern::new(
            "tool-read_file".to_string(),
            "Tool sequence: read_file->edit_file (2 steps)".to_string(),
            "tool_sequence".to_string(),
        );
        p.trajectory_ids.push("t-1".to_string());
        p.frequency = 1;
        p.success_rate = 1.0;
        storage.save_pattern(&p).await.expect("测试：首次保存应成功");

        // 第二轮：同一自然键 ⇒ 同一 id，频率与成功率推进
        let mut p2 = p.clone();
        p2.trajectory_ids.push("t-2".to_string());
        p2.frequency = 2;
        p2.success_rate = 0.5;
        storage.save_pattern(&p2).await.expect("测试：二次保存应成功");

        let rows = trajectory_patterns::Entity::find().all(&db).await.expect("测试：查询应成功");
        assert_eq!(
            rows.len(),
            1,
            "同一自然键必须只有一行 —— 多于一行即 ON CONFLICT (id) 没触发（C-#2 复发）"
        );
        assert_eq!(rows[0].frequency, 2, "冲突分支必须把频率推进到 2");
        assert_eq!(rows[0].success_rate, 0.5, "冲突分支必须更新成功率");
        assert!(
            rows[0].trajectory_ids.contains("t-2"),
            "聚合列必须随 upsert 一起更新，否则 frequency 与 trajectory_ids 自相矛盾"
        );
    }

    /// C-#2 反向守卫：**不同 `id` + 同名**必须仍是两行。
    ///
    /// `commands/rl_training.rs` 的检查点正是这个形态：身份是调用方给的 `id`
    /// （`load_checkpoint` 按 `p.id == checkpoint_id` 命中、`list_checkpoints` 按 `ckpt.id`
    /// 去重），`name` 是 `rl_checkpoint:{name}`。本项修法只把「学习器产出的模式」主键
    /// 确定性化，**没有**把「身份 = name」的假设塞进 repo 层 —— 否则同名检查点会被压成一行。
    #[tokio::test]
    async fn save_pattern_keeps_distinct_ids_with_same_name_apart() {
        let db = axagent_dao::db::create_test_pool().await.expect("测试：创建连接池应成功").conn;
        let storage = TrajectoryStorage::new(Arc::new(db.clone()));

        for id in ["ckpt-a", "ckpt-b"] {
            let p = TrajectoryPattern {
                id: id.to_string(),
                name: "rl_checkpoint:epoch-1".to_string(),
                description: format!("{{\"id\":\"{}\"}}", id),
                pattern_type: "rl_checkpoint".to_string(),
                trajectory_ids: Vec::new(),
                frequency: 1,
                success_rate: 0.5,
                average_quality: 0.5,
                average_value_score: 0.5,
                reward_profile: Vec::new(),
                created_at: Utc::now(),
            };
            storage.save_pattern(&p).await.expect("测试：保存检查点式模式应成功");
        }

        let rows = trajectory_patterns::Entity::find().all(&db).await.expect("测试：查询应成功");
        assert_eq!(rows.len(), 2, "同名但异 id 的两条检查点记录必须都保留");
    }

    /// D5（2026-09-14）：`parse_stored_relation_type` 必须**同时**吃下两种历史编码。
    ///
    /// 这是「写入端收敛、读取端宽容」这个方案的**唯一前提**：如果读端只认裸字面量，
    /// 那存量带引号的行会被静默解析成 `RelatedTo`（默认值）—— 不报错，但语义错了，
    /// 等于改写数据。本测试锁住这条。
    #[test]
    fn parse_stored_relation_type_tolerates_both_historical_encodings() {
        // `RelationshipType` 由模块级 `use` 引入，再经 `use super::*` 带进本测试模块
        // ⇒ 这里**不要**再 `use` 一次（重复导入会触发 unused_imports ⇒ clippy -D warnings 红）。
        // ① 现行形态：裸字面量（写入端 `Display` 产物）
        assert_eq!(parse_stored_relation_type("part_of"), RelationshipType::PartOf);
        assert_eq!(parse_stored_relation_type("contains"), RelationshipType::Contains);

        // ② 历史形态：JSON 编码（旧 `serde_json::to_string` 产物）—— 必须解析成**同一个**值，
        //    而不是落默认值 `RelatedTo`。
        assert_eq!(parse_stored_relation_type("\"part_of\""), RelationshipType::PartOf);
        assert_eq!(parse_stored_relation_type("\"contains\""), RelationshipType::Contains);
        assert_ne!(
            parse_stored_relation_type("\"part_of\""),
            RelationshipType::RelatedTo,
            "带引号行被解析成默认值 = 静默改写数据"
        );

        // ③ 形态边界：首尾空白 / 单边引号 / 空串 —— 一律不 panic，且不得造出假关系
        assert_eq!(parse_stored_relation_type("  part_of  "), RelationshipType::PartOf);
        assert_eq!(parse_stored_relation_type("\"part_of"), RelationshipType::RelatedTo);
        assert_eq!(parse_stored_relation_type("part_of\""), RelationshipType::RelatedTo);
        assert_eq!(parse_stored_relation_type(""), RelationshipType::RelatedTo);
        assert_eq!(parse_stored_relation_type("\""), RelationshipType::RelatedTo);

        // ④ 真实存量里存在的**数据驱动值**（`edges.csv` 的中文职位名）—— 未识别 ⇒ 落 `RelatedTo`。
        //    这仍然不是「静默改写」：它们在收敛前也解析不出枚举变体，行为一致。
        assert_eq!(parse_stored_relation_type("董事"), RelationshipType::RelatedTo);
        assert_eq!(parse_stored_relation_type("\"董事\""), RelationshipType::RelatedTo);
    }

    /// `strip_json_quotes` 的**UTF-8 边界安全性** —— 切片落在多字节字符上会 panic。
    #[test]
    fn strip_json_quotes_never_slices_inside_a_multibyte_char() {
        assert_eq!(strip_json_quotes("\"董事\""), "董事");
        assert_eq!(strip_json_quotes("\"\""), "");
        assert_eq!(strip_json_quotes("\""), "\"");
        assert_eq!(strip_json_quotes(""), "");
        assert_eq!(strip_json_quotes("董事"), "董事");
        // 首尾虽为引号但中间含多字节字符：切片点仍是 ASCII 字节边界 ⇒ 安全
        assert_eq!(strip_json_quotes("\"a董事b\""), "a董事b");
    }
}

fn ke_to_entity(e: &knowledge_entities::Model) -> Entity {
    use crate::memory::EntityType;
    Entity {
        id: e.id.clone(),
        name: e.name.clone(),
        entity_type: serde_json::from_str(&format!("\"{}\"", e.entity_type))
            .unwrap_or(EntityType::Concept),
        properties: match &e.properties {
            serde_json::Value::Object(map) => {
                map.iter().map(|(k, v)| (k.clone(), v.clone())).collect()
            },
            _ => std::collections::HashMap::new(),
        },
        aliases: serde_json::from_str(&e.aliases).unwrap_or_default(),
        first_seen_at: e
            .first_seen_at
            .as_ref()
            .and_then(|s| {
                chrono::DateTime::parse_from_rfc3339(s).map(|dt| dt.with_timezone(&Utc)).ok()
            })
            .unwrap_or_else(Utc::now),
        last_seen_at: e
            .last_seen_at
            .as_ref()
            .and_then(|s| {
                chrono::DateTime::parse_from_rfc3339(s).map(|dt| dt.with_timezone(&Utc)).ok()
            })
            .unwrap_or_else(Utc::now),
        mention_count: e.mention_count as u32,
        confidence: e.confidence,
        created_at: Some(Utc::now()),
        updated_at: Some(Utc::now()),
    }
}

/// 解析存量 `knowledge_relations.relation_type` 字面量，**兼容两种历史编码**。
///
/// | 形态 | 谁写的 | 例子 |
/// |---|---|---|
/// | 裸字面量 | 现行（D5 收敛后）与其余全部写入方（`causal.rs` / `dao::repo::conversation` / `edges.csv`） | `part_of` |
/// | JSON 编码（带引号） | 本文件 2026-09-14 之前的写法（`serde_json::to_string`） | `"part_of"` |
///
/// 读取端**永久**兼容两态：收敛只改了写入方，存量行不会自己变成裸字面量，
/// 而带引号的行在旧版本里确实可能已落库 —— 靠「迁移脚本」不如靠「读端宽容」来得稳
/// （迁移漏一次就永久丢数据，读端宽容则自愈）。本机实测带引号行 = 0，故无需迁移。
///
/// 未识别的取值落 `RelatedTo`（与收敛前的 `serde_json::from_str(..).unwrap_or(RelatedTo)` 同）。
fn parse_stored_relation_type(raw: &str) -> RelationshipType {
    RelationshipType::from(strip_json_quotes(raw.trim()))
}

/// 去掉成对的 JSON 引号：`"part_of"` → `part_of`；不成对或为空则原样返回。
///
/// 字节切片在此是安全的：首尾都是 1 字节的 ASCII `"`，切点必落在 UTF-8 边界上。
fn strip_json_quotes(raw: &str) -> &str {
    if raw.len() >= 2 && raw.starts_with('"') && raw.ends_with('"') {
        &raw[1..raw.len() - 1]
    } else {
        raw
    }
}

fn kr_to_relationship(r: &knowledge_relations::Model) -> Relationship {
    Relationship {
        id: r.id.clone(),
        source_id: r.source_entity_id.clone(),
        target_id: r.target_entity_id.clone(),
        relation_type: parse_stored_relation_type(&r.relation_type),
        properties: match &r.properties {
            Some(serde_json::Value::Object(map)) => {
                map.iter().map(|(k, v)| (k.clone(), v.clone())).collect()
            },
            _ => std::collections::HashMap::new(),
        },
        weight: r.weight,
        created_at: Utc::now(),
    }
}

fn model_to_sess(s: &trajectory_sessions::Model) -> TrajectorySession {
    TrajectorySession {
        id: s.id.clone(),
        title: s.title.clone(),
        platform: s.platform.clone(),
        user_id: s.user_id.clone(),
        model: s.model.clone(),
        system_prompt: s.system_prompt.clone(),
        created_at: chrono::DateTime::parse_from_rfc3339(&s.created_at)
            .map(|dt| dt.with_timezone(&Utc))
            .unwrap_or_else(|_| Utc::now()),
        updated_at: chrono::DateTime::parse_from_rfc3339(&s.updated_at)
            .map(|dt| dt.with_timezone(&Utc))
            .unwrap_or_else(|_| Utc::now()),
        parent_session_id: s.parent_session_id.clone(),
        token_input: s.token_input,
        token_output: s.token_output,
    }
}

fn model_to_msg(m: &trajectory_messages::Model) -> Message {
    Message {
        id: m.id.clone(),
        session_id: m.session_id.clone(),
        role: m.role.clone(),
        content: m.content.clone(),
        tool_calls: m.tool_calls.clone(),
        tool_results: m.tool_results.clone(),
        usage: m.usage.clone(),
        created_at: chrono::DateTime::parse_from_rfc3339(&m.created_at)
            .map(|dt| dt.with_timezone(&Utc))
            .unwrap_or_else(|_| Utc::now()),
    }
}

// ── Trajectory Cleanup Task ──

pub struct TrajectoryCleanupTask {
    storage: Arc<TrajectoryStorage>,
    config: TrajectoryCleanupConfig,
    interval: std::time::Duration,
    handle: Option<tokio::task::JoinHandle<()>>,
    shutdown_tx: Option<tokio::sync::oneshot::Sender<()>>,
}

impl TrajectoryCleanupTask {
    pub fn new(
        storage: Arc<TrajectoryStorage>,
        config: TrajectoryCleanupConfig,
        interval: std::time::Duration,
    ) -> Self {
        Self { storage, config, interval, handle: None, shutdown_tx: None }
    }

    pub fn start(&mut self) {
        if self.handle.is_some() {
            return;
        }
        let storage = self.storage.clone();
        let config = self.config.clone();
        let interval = self.interval;
        let (shutdown_tx, mut shutdown_rx) = tokio::sync::oneshot::channel::<()>();
        let handle = tokio::spawn(async move {
            let mut interval = tokio::time::interval(interval);
            loop {
                tokio::select! {
                    _ = interval.tick() => {
                        let result = AssertUnwindSafe(async {
                            match storage.cleanup(&config).await {
                                Ok(count) if count > 0 => {
                                    info!("Cleaned up {} old trajectories", count);
                                }
                                Ok(_) => {}
                                Err(e) => {
                                    warn!("[TrajectoryCleanupTask] cleanup failed: {}", e);
                                }
                            }
                        })
                        .catch_unwind()
                        .await;
                        if let Err(p) = result {
                            let msg = if let Some(s) = p.downcast_ref::<String>() {
                                s.clone()
                            } else if let Some(s) = p.downcast_ref::<&'static str>() {
                                (*s).to_owned()
                            } else {
                                "Unknown panic in trajectory cleanup".to_string()
                            };
                            warn!("[TrajectoryCleanupTask] PANIC in cleanup loop: {}", msg);
                        }
                    }
                    _ = &mut shutdown_rx => {
                        info!("Trajectory cleanup task shutting down");
                        break;
                    }
                }
            }
        });
        self.handle = Some(handle);
        self.shutdown_tx = Some(shutdown_tx);
    }

    pub async fn shutdown(self) {
        if let Some(tx) = self.shutdown_tx {
            let _ = tx.send(());
        }
        if let Some(handle) = self.handle {
            let _ = handle.await;
        }
    }
}

// ── Public types ──

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TrajectorySession {
    pub id: String,
    pub title: String,
    pub platform: String,
    pub user_id: String,
    pub model: String,
    pub system_prompt: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
    pub parent_session_id: Option<String>,
    pub token_input: i64,
    pub token_output: i64,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SessionUpdate {
    pub title: Option<String>,
    pub token_input: Option<i64>,
    pub token_output: Option<i64>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Message {
    pub id: String,
    pub session_id: String,
    pub role: String,
    pub content: String,
    pub tool_calls: Option<String>,
    pub tool_results: Option<String>,
    pub usage: Option<String>,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Pattern {
    pub id: String,
    pub pattern: String,
    pub pattern_type: String,
    pub success: i32,
    pub failure: i32,
    pub last_used: chrono::DateTime<chrono::Utc>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub metadata: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Preference {
    pub id: String,
    pub key: String,
    pub value: String,
    pub confidence: f64,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TrajectoryStatistics {
    pub total_trajectories: usize,
    pub total_sessions: usize,
    pub total_patterns: usize,
    pub avg_quality: f64,
    pub avg_value_score: f64,
    pub success_rate: f64,
    pub recent_trajectories: usize,
}
