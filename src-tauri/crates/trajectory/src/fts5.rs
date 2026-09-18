// SPDX-License-Identifier: AGPL-3.0-only

//! FTS5 Full-Text Search module for enhanced cross-session retrieval
//!
//! Features:
//! - SQLite FTS5 virtual tables for trajectories, memories, skills
//! - BM25 ranking with configurable parameters
//! - Phrase matching and proximity search
//! - Snippet generation with highlight markers

use crate::trajectory::{Trajectory, TrajectoryOutcome};
use anyhow::{Context, Result};
use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::{debug, info};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FTS5Config {
    pub bm25_k1: f64,
    pub bm25_b: f64,
    pub snippet_size: usize,
    pub highlight_open: String,
    pub highlight_close: String,
}

impl Default for FTS5Config {
    fn default() -> Self {
        Self {
            bm25_k1: 1.5,
            bm25_b: 0.75,
            snippet_size: 300,
            highlight_open: "【".to_string(),
            highlight_close: "】".to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FTS5Query {
    pub query: String,
    pub limit: usize,
    pub offset: usize,
    pub filter_type: Option<String>,
    pub filter_session_id: Option<String>,
    pub min_relevance: Option<f64>,
}

impl Default for FTS5Query {
    fn default() -> Self {
        Self {
            query: String::new(),
            limit: 10,
            offset: 0,
            filter_type: None,
            filter_session_id: None,
            min_relevance: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FTS5Result {
    pub id: String,
    pub doc_type: String,
    pub content: String,
    pub snippet: String,
    pub rank: f64,
    pub bm25_score: f64,
    pub session_id: Option<String>,
    pub timestamp: i64,
    pub metadata: Option<String>,
}

pub struct FTS5Search {
    conn: Arc<Mutex<Connection>>,
    config: FTS5Config,
}

impl FTS5Search {
    pub fn new(conn: Arc<Mutex<Connection>>, config: FTS5Config) -> Self {
        Self { conn, config }
    }

    pub async fn create_fts_tables(&self) -> Result<()> {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || -> Result<()> {
            let conn = conn.blocking_lock();
            conn.execute_batch(
                r#"
                CREATE VIRTUAL TABLE IF NOT EXISTS trajectories_fts USING fts5(
                    id UNINDEXED,
                    session_id UNINDEXED,
                    topic,
                    summary,
                    content,
                    outcome UNINDEXED,
                    quality_score UNINDEXED,
                    created_at UNINDEXED,
                    tokenize='porter unicode61'
                );

                CREATE VIRTUAL TABLE IF NOT EXISTS memory_items_fts USING fts5(
                    id UNINDEXED,
                    memory_type UNINDEXED,
                    content,
                    entities,
                    created_at UNINDEXED,
                    tokenize='porter unicode61'
                );

                CREATE VIRTUAL TABLE IF NOT EXISTS trajectory_skills_fts USING fts5(
                    id UNINDEXED,
                    name,
                    description,
                    content,
                    category UNINDEXED,
                    tags,
                    created_at UNINDEXED,
                    tokenize='porter unicode61'
                );

                CREATE VIRTUAL TABLE IF NOT EXISTS trajectory_messages_fts USING fts5(
                    id UNINDEXED,
                    session_id UNINDEXED,
                    role UNINDEXED,
                    content,
                    created_at UNINDEXED,
                    tokenize='porter unicode61'
                );

                "#,
            )
            .context("Failed to create FTS5 tables")?;

            info!("FTS5 tables created successfully");
            Ok(())
        })
        .await??;
        Ok(())
    }

    pub async fn index_trajectory(&self, trajectory: &Trajectory, session_id: &str) -> Result<()> {
        let conn = self.conn.clone();
        let trajectory = trajectory.clone();
        let session_id = session_id.to_string();
        tokio::task::spawn_blocking(move || -> Result<()> {
            let conn = conn.blocking_lock();

            let content = trajectory
                .steps
                .iter()
                .map(|s| s.content.clone())
                .collect::<Vec<_>>()
                .join("\n");

            let outcome_str = match trajectory.outcome {
                TrajectoryOutcome::Success => "success",
                TrajectoryOutcome::Partial => "partial",
                TrajectoryOutcome::Failure => "failure",
                TrajectoryOutcome::Abandoned => "abandoned",
            };

            conn.execute(
                r#"INSERT INTO trajectories_fts (id, session_id, topic, summary, content, outcome, quality_score, created_at)
                   VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)"#,
                params![
                    trajectory.id,
                    session_id,
                    trajectory.topic,
                    trajectory.summary,
                    content,
                    outcome_str,
                    trajectory.quality.overall,
                    trajectory.created_at.timestamp()
                ],
            )?;

            debug!("Indexed trajectory {} for FTS5", trajectory.id);
            Ok(())
        })
        .await??;
        Ok(())
    }

    pub async fn index_memory(
        &self,
        id: &str,
        memory_type: &str,
        content: &str,
        entities: &[String],
    ) -> Result<()> {
        let conn = self.conn.clone();
        let id = id.to_string();
        let memory_type = memory_type.to_string();
        let content = content.to_string();
        let entities = entities.to_vec();
        tokio::task::spawn_blocking(move || -> Result<()> {
            let conn = conn.blocking_lock();

            conn.execute(
                r#"INSERT INTO memory_items_fts (id, memory_type, content, entities, created_at)
                   VALUES (?1, ?2, ?3, ?4, ?5)"#,
                params![
                    id,
                    memory_type,
                    content,
                    entities.join(" "),
                    chrono::Utc::now().timestamp()
                ],
            )?;

            debug!("Indexed memory {} for FTS5", id);
            Ok(())
        })
        .await??;
        Ok(())
    }

    pub async fn index_skill(
        &self,
        id: &str,
        name: &str,
        description: &str,
        content: &str,
        category: &str,
        tags: &[String],
    ) -> Result<()> {
        let conn = self.conn.clone();
        let id = id.to_string();
        let name = name.to_string();
        let description = description.to_string();
        let content = content.to_string();
        let category = category.to_string();
        let tags = tags.to_vec();
        tokio::task::spawn_blocking(move || -> Result<()> {
            let conn = conn.blocking_lock();

            conn.execute(
                r#"INSERT INTO trajectory_skills_fts (id, name, description, content, category, tags, created_at)
                   VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)"#,
                params![
                    id,
                    name,
                    description,
                    content,
                    category,
                    tags.join(" "),
                    chrono::Utc::now().timestamp()
                ],
            )?;

            debug!("Indexed skill {} for FTS5", id);
            Ok(())
        })
        .await??;
        Ok(())
    }

    pub async fn index_message(&self, msg: &crate::storage::Message) -> Result<()> {
        let conn = self.conn.clone();
        let msg = msg.clone();
        tokio::task::spawn_blocking(move || -> Result<()> {
            let conn = conn.blocking_lock();
            conn.execute(
                r#"INSERT INTO trajectory_messages_fts (id, session_id, role, content, created_at)
                   VALUES (?1, ?2, ?3, ?4, ?5)"#,
                params![msg.id, msg.session_id, msg.role, msg.content, msg.created_at.timestamp()],
            )?;
            debug!("Indexed message {} for FTS5", msg.id);
            Ok(())
        })
        .await??;
        Ok(())
    }

    fn validate_table_name(name: &str) -> anyhow::Result<()> {
        if name.is_empty() || !name.chars().all(|c| c.is_alphanumeric() || c == '_') {
            anyhow::bail!("Invalid table name: {}", name);
        }
        Ok(())
    }

    /// 校验 FTS5 列名白名单，防止 `search_phrase` 将用户可控的字段名直接拼入 SQL 造成注入。
    fn validate_field_name(name: &str) -> anyhow::Result<()> {
        const ALLOWED: &[&str] = &["topic", "summary", "content"];
        if !ALLOWED.contains(&name) {
            anyhow::bail!("Invalid field name for phrase search: {}", name);
        }
        Ok(())
    }

    pub async fn delete_from_fts(&self, table: &str, id: &str) -> Result<()> {
        Self::validate_table_name(table)?;
        let conn = self.conn.clone();
        let table = table.to_string();
        let id = id.to_string();
        tokio::task::spawn_blocking(move || -> Result<()> {
            let conn = conn.blocking_lock();
            // FTS5 'delete' 命令要求 rowid 位置传入整数隐式 rowid，而非我们的业务字符串 id。
            // `id` 列只是普通 UNINDEXED 文本列，因此必须先查出该行的 rowid 与 content，
            // 再用 (rowid, content) 形式发出 delete 命令，否则会匹配不到任何 rowid 而删除失效。
            let row: Option<(i64, String)> = conn
                .query_row(
                    &format!("SELECT rowid, content FROM {} WHERE id = ?1", table),
                    params![id],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .ok();
            if let Some((rowid, content)) = row {
                let sql = format!(
                    "INSERT INTO {}({}, rowid, content) VALUES('delete', ?1, ?2)",
                    table, table
                );
                conn.execute(&sql, params![rowid, content])?;
            }
            // 若 rowid 不存在（该行已不在 FTS 索引中），无需任何操作。
            debug!("Deleted {} from FTS5 table {}", id, table);
            Ok(())
        })
        .await??;
        Ok(())
    }

    pub async fn search(&self, query: FTS5Query) -> Result<Vec<FTS5Result>> {
        let conn = self.conn.clone();
        let config = self.config.clone();
        let results = tokio::task::spawn_blocking(move || -> Result<Vec<FTS5Result>> {
            let conn = conn.blocking_lock();
            let mut results = Vec::new();

            let tables = if let Some(ref filter) = query.filter_type {
                vec![filter.clone()]
            } else {
                vec![
                    "trajectories_fts".to_string(),
                    "memory_items_fts".to_string(),
                    "trajectory_skills_fts".to_string(),
                    "trajectory_messages_fts".to_string(),
                ]
            };

            for table in tables {
                let sql = match table.as_str() {
                    "trajectories_fts" => {
                        r#"
                        SELECT 
                            t.id,
                            'trajectory' as doc_type,
                            COALESCE(t.topic, '') || ' ' || COALESCE(t.summary, '') || ' ' || COALESCE(t.content, '') as content,
                            t.session_id,
                            t.created_at,
                            t.quality_score,
                            t.outcome,
                            bm25(trajectories_fts) as rank
                        FROM trajectories_fts t
                        WHERE trajectories_fts MATCH ?1
                        ORDER BY rank
                        LIMIT ?2 OFFSET ?3
                        "#
                    },
                    "memory_items_fts" => {
                        r#"
                        SELECT 
                            m.id,
                            'memory' as doc_type,
                            m.content,
                            NULL as session_id,
                            m.created_at,
                            NULL as quality_score,
                            NULL as outcome,
                            bm25(memory_items_fts) as rank
                        FROM memory_items_fts m
                        WHERE memory_items_fts MATCH ?1
                        ORDER BY rank
                        LIMIT ?2 OFFSET ?3
                        "#
                    },
                    "trajectory_skills_fts" => {
                        r#"
                        SELECT 
                            s.id,
                            'skill' as doc_type,
                            s.name || ' ' || s.description || ' ' || s.content as content,
                            NULL as session_id,
                            s.created_at,
                            NULL as quality_score,
                            NULL as outcome,
                            bm25(trajectory_skills_fts) as rank
                        FROM trajectory_skills_fts s
                        WHERE trajectory_skills_fts MATCH ?1
                        ORDER BY rank
                        LIMIT ?2 OFFSET ?3
                        "#
                    },
                    "trajectory_messages_fts" => {
                        r#"
                        SELECT 
                            m.id,
                            'message' as doc_type,
                            m.content,
                            m.session_id,
                            m.created_at,
                            NULL as quality_score,
                            NULL as outcome,
                            bm25(trajectory_messages_fts) as rank
                        FROM trajectory_messages_fts m
                        WHERE trajectory_messages_fts MATCH ?1
                        ORDER BY rank
                        LIMIT ?2 OFFSET ?3
                        "#
                    },
                    _ => continue,
                };

                let mut stmt = conn.prepare(sql)?;

                let rows = stmt.query_map(
                    params![query.query, query.limit as i64, query.offset as i64],
                    |row| {
                        Ok(FTS5Result {
                            id: row.get(0)?,
                            doc_type: row.get(1)?,
                            content: row.get(2)?,
                            snippet: String::new(),
                            rank: row.get(7)?,
                            bm25_score: row.get(7)?,
                            session_id: row.get(3)?,
                            timestamp: row.get(4)?,
                            metadata: None,
                        })
                    },
                )?;

                for row in rows.filter_map(|r| r.ok()) {
                    let mut result = row;
                    result.snippet = Self::generate_snippet(&result.content, &query.query, &config);
                    results.push(result);
                }
            }

            results.sort_by(|a, b| {
                b.rank
                    .partial_cmp(&a.rank)
                    .unwrap_or(std::cmp::Ordering::Equal)
            });

            if let Some(min_rel) = query.min_relevance {
                results.retain(|r| -r.rank >= min_rel);
            }

            results.truncate(query.limit);

            Ok(results)
        })
        .await??;
        Ok(results)
    }

    pub async fn search_phrase(&self, phrase: &str, in_field: &str) -> Result<Vec<FTS5Result>> {
        Self::validate_field_name(in_field)?;
        let conn = self.conn.clone();
        let config = self.config.clone();
        let phrase = phrase.to_string();
        let in_field = in_field.to_string();
        let results = tokio::task::spawn_blocking(move || -> Result<Vec<FTS5Result>> {
            let conn = conn.blocking_lock();

            let mut stmt = conn.prepare(&format!(
                r#"
                SELECT
                    id,
                    '{}' as doc_type,
                    {field},
                    session_id,
                    created_at,
                    quality_score,
                    outcome,
                    bm25({table}) as rank
                FROM {table}
                WHERE {table} MATCH ?1
                ORDER BY rank
                LIMIT 10
                "#,
                in_field,
                field = in_field,
                table = "trajectories_fts"
            ))?;

            let query_str = format!("\"{}\"", phrase.replace("\"", "\"\""));
            let rows = stmt.query_map(params![query_str], |row| {
                Ok(FTS5Result {
                    id: row.get(0)?,
                    doc_type: row.get(1)?,
                    content: row.get(2)?,
                    snippet: String::new(),
                    rank: row.get(7)?,
                    bm25_score: row.get(7)?,
                    session_id: row.get(3)?,
                    timestamp: row.get(4)?,
                    metadata: None,
                })
            })?;

            let mut results: Vec<FTS5Result> = rows.filter_map(|r| r.ok()).collect();
            for result in &mut results {
                result.snippet = Self::generate_snippet(&result.content, &phrase, &config);
            }

            Ok(results)
        })
        .await??;
        Ok(results)
    }

    pub async fn search_proximity(
        &self,
        term1: &str,
        term2: &str,
        distance: i32,
    ) -> Result<Vec<FTS5Result>> {
        let conn = self.conn.clone();
        let config = self.config.clone();
        let term1 = term1.to_string();
        let term2 = term2.to_string();
        let results = tokio::task::spawn_blocking(move || -> Result<Vec<FTS5Result>> {
            let conn = conn.blocking_lock();

            let query = format!("\"{}\" NEAR/{} \"{}\"", term1, distance, term2);

            let mut stmt = conn.prepare(
                r#"
                SELECT 
                    t.id,
                    'trajectory' as doc_type,
                    t.topic || ' ' || t.summary || ' ' || t.content as content,
                    t.session_id,
                    t.created_at,
                    t.quality_score,
                    t.outcome,
                    bm25(trajectories_fts) as rank
                FROM trajectories_fts t
                WHERE trajectories_fts MATCH ?1
                ORDER BY rank
                LIMIT 10
                "#,
            )?;

            let rows = stmt.query_map(params![query], |row| {
                Ok(FTS5Result {
                    id: row.get(0)?,
                    doc_type: row.get(1)?,
                    content: row.get(2)?,
                    snippet: String::new(),
                    rank: row.get(7)?,
                    bm25_score: row.get(7)?,
                    session_id: row.get(3)?,
                    timestamp: row.get(4)?,
                    metadata: None,
                })
            })?;

            let mut results: Vec<FTS5Result> = rows.filter_map(|r| r.ok()).collect();
            for result in &mut results {
                result.snippet = Self::generate_snippet(
                    &result.content,
                    &format!("{} NEAR/{} {}", term1, distance, term2),
                    &config,
                );
            }

            Ok(results)
        })
        .await??;
        Ok(results)
    }

    fn generate_snippet(content: &str, query: &str, config: &FTS5Config) -> String {
        let query_terms: Vec<&str> = query.split_whitespace().collect();
        let content_lower = content.to_lowercase();

        // P0-6: 使用 char_indices 找最匹配位置，但 snippet 截取用 chars
        // 避免在多字节字符中间切断引发 panic
        let mut best_pos = 0usize;
        let mut best_matches = 0;
        for (i, _) in content.char_indices() {
            let window_end = std::cmp::min(i + 200, content_lower.len());
            // i 来自 char_indices 所以必为 char boundary；window_end 同样需要落到 char boundary
            let window_end = floor_char_boundary(&content_lower, window_end);
            let window = &content_lower[i..window_end];
            let matches = query_terms.iter().filter(|t| window.contains(&t.to_lowercase())).count();
            if matches > best_matches {
                best_matches = matches;
                best_pos = i;
            }
        }

        // 计算字符维度的 start/end，先按字节减 50，再向最近的 char boundary 对齐
        let start_byte = best_pos.saturating_sub(50);
        let start_byte = floor_char_boundary(content, start_byte);
        let desired_end_byte = std::cmp::min(start_byte + config.snippet_size, content.len());
        let end_byte = floor_char_boundary(content, desired_end_byte);

        let mut snippet = content[start_byte..end_byte].to_string();

        for term in query_terms {
            let pattern = format!("(?i){}", regex::escape(term));
            if let Ok(re) = regex::Regex::new(&pattern) {
                snippet = re
                    .replace_all(
                        &snippet,
                        format!("{}{}{}", config.highlight_open, term, config.highlight_close),
                    )
                    .to_string();
            }
        }

        if start_byte > 0 {
            snippet = format!("...{}", snippet);
        }
        if end_byte < content.len() {
            snippet = format!("{}...", snippet);
        }

        snippet
    }

    pub async fn optimize(&self) -> Result<()> {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || -> Result<()> {
            let conn = conn.blocking_lock();
            conn.execute_batch(
                r#"
                INSERT INTO trajectories_fts(trajectories_fts) VALUES('optimize');
                INSERT INTO memory_items_fts(memory_items_fts) VALUES('optimize');
                INSERT INTO trajectory_skills_fts(trajectory_skills_fts) VALUES('optimize');
                INSERT INTO trajectory_messages_fts(trajectory_messages_fts) VALUES('optimize');
                "#,
            )?;
            info!("FTS5 indexes optimized");
            Ok(())
        })
        .await??;
        Ok(())
    }

    pub async fn vacuum(&self) -> Result<()> {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || -> Result<()> {
            let conn = conn.blocking_lock();
            conn.execute_batch(
                r#"
                INSERT INTO trajectories_fts(trajectories_fts) VALUES('vacuum');
                INSERT INTO memory_items_fts(memory_items_fts) VALUES('vacuum');
                INSERT INTO trajectory_skills_fts(trajectory_skills_fts) VALUES('vacuum');
                INSERT INTO trajectory_messages_fts(trajectory_messages_fts) VALUES('vacuum');
                "#,
            )?;
            info!("FTS5 indexes vacuumed");
            Ok(())
        })
        .await??;
        Ok(())
    }

    // ── CJK 支持 ────────────────────────────────────────────────────

    /// 检测查询是否包含 CJK 字符
    fn contains_cjk(text: &str) -> bool {
        text.chars().any(|c| {
            let cp = c as u32;
            // CJK Unified Ideographs + Extension A + Hangul + Kana
            (0x4E00..=0x9FFF).contains(&cp)
                || (0x3400..=0x4DBF).contains(&cp)
                || (0xAC00..=0xD7AF).contains(&cp)
                || (0x3040..=0x30FF).contains(&cp)
        })
    }

    /// 将 CJK 查询拆分为 trigram token 以支持中文搜索
    /// 对于纯 CJK 查询，生成所有可能的双字/三字组合作为 fallback
    fn tokenize_cjk_query(query: &str) -> Vec<String> {
        let mut result = Vec::new();

        // 原始查询
        result.push(query.to_string());

        // 双字组合 (bigrams)
        let chars: Vec<char> = query.chars().collect();
        if chars.len() >= 2 {
            for window in chars.windows(2) {
                result.push(window.iter().collect());
            }
        }

        // 单字 fallback (对短查询)
        if chars.len() <= 3 {
            for c in &chars {
                result.push(c.to_string());
            }
        }

        result
    }

    /// 带 CJK 支持的增强搜索
    pub async fn search_with_cjk(&self, query: &str, limit: usize) -> Result<Vec<FTS5Result>> {
        if !Self::contains_cjk(query) {
            let fts_query = FTS5Query { query: query.to_string(), limit, ..Default::default() };
            return self.search(fts_query).await;
        }

        // CJK 查询: 先用原始查询搜索
        let mut results = Vec::new();
        let mut seen_ids = std::collections::HashSet::new();

        // 1. 原始查询搜索 (FTS5 unicode61 tokenizer 可处理部分 CJK)
        let primary_query = FTS5Query { query: query.to_string(), limit, ..Default::default() };
        if let Ok(primary_results) = self.search(primary_query).await {
            for r in &primary_results {
                seen_ids.insert(r.id.clone());
            }
            results.extend(primary_results);
        }

        // 2. 如果结果不足，用 CJK trigram token 做 fallback
        if results.len() < limit {
            let cjk_tokens = Self::tokenize_cjk_query(query);
            for token in &cjk_tokens {
                if token == query {
                    continue;
                }
                let fallback_query = FTS5Query {
                    query: token.clone(),
                    limit: limit.saturating_sub(results.len()),
                    ..Default::default()
                };
                if let Ok(fallback_results) = self.search(fallback_query).await {
                    for r in fallback_results {
                        if seen_ids.insert(r.id.clone()) {
                            results.push(r);
                        }
                    }
                }
                if results.len() >= limit {
                    break;
                }
            }
        }

        results.truncate(limit);
        Ok(results)
    }

    // ── 会话谱系搜索 ────────────────────────────────────────────────

    /// 按会话谱系搜索 — 搜索当前会话及其所有祖先/后代会话
    ///
    /// 注意：当前 trajectory FTS5 索引不包含 parent_session_id 谱系数据，
    /// 因此仅支持「直接匹配当前会话」（lineage_distance = 0）。
    /// 真实的祖先/后代回溯需由命令层基于 conversations.parent_conversation_id
    /// 实现（见 commands::conversations_search::get_conversation_lineage）。
    pub async fn search_session_lineage(
        &self,
        query: &str,
        session_id: &str,
        _lineage_depth: u32,
        limit: usize,
    ) -> Result<Vec<LineageSearchResult>> {
        let config = FTS5Query {
            query: query.to_string(),
            limit: limit * 2, // 多取一些结果用于过滤
            ..Default::default()
        };

        let all_results = if Self::contains_cjk(query) {
            self.search_with_cjk(query, limit * 2).await?
        } else {
            self.search(config).await?
        };

        let mut lineage_results = Vec::new();

        // 仅保留当前会话的直接匹配；无谱系表时不做前缀猜测（避免假阳性召回）
        for result in &all_results {
            if let Some(ref sid) = result.session_id
                && sid == session_id
            {
                lineage_results.push(LineageSearchResult {
                    result: result.clone(),
                    related_session_ids: vec![session_id.to_string()],
                    lineage_distance: 0,
                });
            }
            if lineage_results.len() >= limit {
                break;
            }
        }

        Ok(lineage_results)
    }

    // ── FTS5 健康检查 ──────────────────────────────────────────────

    /// 检查 FTS5 索引健康状态。
    ///
    /// `memory_namespace_id` 必须传入轨迹记忆的命名空间：`memory_items` 是全项目
    /// 共用的表（知识库文档也写它），而 `memory_items_fts` **只索引轨迹记忆**
    /// （唯一写入点是 `memory_providers/service.rs` 的 `index_memory_fts`）。
    /// 不按命名空间收口，源行数会把知识库记忆一并算进来
    /// ⇒ `needs_rebuild` 永远为真且永远无法清除（就是本次修掉的那类「恒真标志位」）。
    pub async fn health_check(&self, memory_namespace_id: Option<&str>) -> Result<FTS5Health> {
        let conn = self.conn.clone();
        let ns = memory_namespace_id.map(|s| s.to_string());
        let health = tokio::task::spawn_blocking(move || -> Result<FTS5Health> {
            let conn = conn.blocking_lock();

            let tables_exist = conn
                .query_row(
                    "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='trajectories_fts'",
                    [],
                    |row| row.get::<_, i64>(0),
                )
                .unwrap_or(0)
                > 0;

            let count_table = |table: &str| -> u64 {
                conn.query_row(
                    &format!("SELECT COUNT(*) FROM {}", table),
                    [],
                    |row| row.get::<_, i64>(0),
                )
                .unwrap_or(0) as u64
            };

            let trajectories_count = count_table("trajectories_fts");
            let memory_items_count = count_table("memory_items_fts");
            let skills_count = count_table("trajectory_skills_fts");
            let messages_count = count_table("trajectory_messages_fts");

            // 源表行数：只统计轨迹命名空间下的记忆（见方法文档）。
            // 未提供命名空间时取 0 —— 宁可让 drift 判定退化成「只看结构」，
            // 也不用全表计数伪造一个「索引落后」的结论。
            let memory_items_source_count = match ns.as_deref() {
                Some(ns) => conn
                    .query_row(
                        "SELECT COUNT(*) FROM memory_items WHERE namespace_id = ?1",
                        params![ns],
                        |row| row.get::<_, i64>(0),
                    )
                    .unwrap_or(0) as u64,
                None => 0,
            };

            // `needs_rebuild` 的语义修正。原实现是「四张索引表都为空」，两个方向都错：
            //   ① 全新空库（无任何数据）也判 true —— 假警报，且重建无法消除（无数据可索引）
            //   ② 重建后索引全空 ⇒ 恒为 true，永远无法满足
            // 改为判定「有源数据但索引为空」这一确定无疑的落后状态。
            // 部分落后（索引少于源）不在此标志位里表态，而是通过下方两个计数并排暴露，
            // 由消费方自行计算 —— 硬编码一个比例阈值只会制造第二个不可满足的判定。
            let needs_rebuild = !tables_exist
                || (memory_items_source_count > 0 && memory_items_count == 0);

            Ok(FTS5Health {
                available: true,
                unavailable_reason: None,
                tables_exist,
                trajectories_count,
                memory_items_count,
                skills_count,
                messages_count,
                memory_items_source_count,
                needs_rebuild,
            })
        })
        .await??;

        Ok(health)
    }

    /// 重建 FTS5 索引。
    ///
    /// ## 与原实现的差异（本次修复的核心）
    ///
    /// 原实现无条件 `DROP` 全部 4 张 FTS 表、再建成空表，**没有任何回填步骤**，
    /// 然后打印「rebuilt successfully」并返回 `Ok(())`。也就是说：
    /// 「重建」= 清空索引 + 报告成功。方向是**越修越坏** —— 重建前可能还有部分索引，
    /// 重建后必然全空；而因为返回 `Ok(())`，调用方无从发现。
    ///
    /// 本实现遵守一条硬规则：**不 DROP 一张没有回填路径的表**。
    /// 当前只有 `memory_items_fts` 具备回填能力（源 `memory_items`，列映射见下方 SQL），
    /// 其余三张原样保留并在 `skipped` 中说明原因 —— 保留一份"不完整但真实"的索引，
    /// 永远优于一份"完整但为空"的索引。
    ///
    /// 回填后**必须自校验**（源行数 == 索引行数）。重建这种破坏性操作不能靠
    /// 「没报错」证明成功，只能靠计数相等证明；不等即 `Err` 并把两个计数带回。
    pub async fn rebuild_indexes(&self, memory_namespace_id: &str) -> Result<FTS5RebuildReport> {
        let conn = self.conn.clone();
        let ns = memory_namespace_id.to_string();
        info!("Starting FTS5 index rebuild...");

        let report = tokio::task::spawn_blocking(move || -> Result<FTS5RebuildReport> {
            let conn = conn.blocking_lock();

            // 1. 只删除「能在本函数内回填」的表。见方法文档的硬规则。
            conn.execute_batch("DROP TABLE IF EXISTS memory_items_fts;")?;

            // 2. 重建该表（列定义与 `create_fts_tables` 保持一字不差）
            conn.execute_batch(
                r#"
                CREATE VIRTUAL TABLE memory_items_fts USING fts5(
                    id UNINDEXED,
                    memory_type UNINDEXED,
                    content,
                    entities,
                    created_at UNINDEXED,
                    tokenize='porter unicode61'
                );
                "#,
            )?;

            // 3. 从源表回填。列映射的依据是 `storage.rs::save_memory`（唯一的写入路径）：
            //      memory_items_fts.memory_type ← memory_items.title
            //          —— `save_memory` 把 `MemoryEntry.memory_type` 存进了 `title` 列
            //             （crates/trajectory/src/storage.rs:1191 `title: Set(mem.memory_type.clone())`）
            //      memory_items_fts.entities    ← memory_items.tags
            //          —— `index_memory` 写入的是 `entities.join(" ")`，
            //             而 `save_memory` 存的是 JSON 数组串（`["a","b"]`）。
            //             直接照搬会把 `[` `]` `"` `,` 一起塞进 FTS 词流，
            //             故在此剥掉这些标点，保持与增量写入路径同构。
            //      memory_items_fts.created_at  ← memory_items.updated_at
            //          —— `memory_items` **没有** created_at 列；该列在 FTS 里是 UNINDEXED，
            //             仅用于回显，取最近的写入时间比取 0 更有信息量。
            let source_rows = conn.query_row(
                "SELECT COUNT(*) FROM memory_items WHERE namespace_id = ?1",
                params![ns],
                |row| row.get::<_, i64>(0),
            )? as u64;

            conn.execute(
                r#"INSERT INTO memory_items_fts (id, memory_type, content, entities, created_at)
                   SELECT
                       id,
                       COALESCE(title, ''),
                       COALESCE(content, ''),
                       REPLACE(REPLACE(REPLACE(REPLACE(COALESCE(tags, ''), '[', ' '), ']', ' '), '"', ' '), ',', ' '),
                       COALESCE(CAST(updated_at AS INTEGER) / 1000, 0)
                   FROM memory_items
                   WHERE namespace_id = ?1"#,
                params![ns],
            )?;

            // 4. 自校验：出口必须严。计数不等 = 回填不完整，必须以 Err 返回，
            //    否则「重建成功」又与「索引缺行」共存，回到原缺陷同形态。
            let indexed_rows = conn.query_row(
                "SELECT COUNT(*) FROM memory_items_fts",
                [],
                |row| row.get::<_, i64>(0),
            )? as u64;

            if indexed_rows != source_rows {
                anyhow::bail!(
                    "FTS5 重建自校验失败：memory_items 源行数 {} != 索引行数 {}（命名空间 {}）。\
                     索引态可能不完整，已保留现有索引不再继续操作。",
                    source_rows,
                    indexed_rows,
                    ns
                );
            }

            info!(
                "FTS5 memory_items_fts rebuilt: {} / {} rows",
                indexed_rows, source_rows
            );

            Ok(FTS5RebuildReport {
                rebuilt: vec!["memory_items_fts".to_string()],
                skipped: vec![
                    "trajectories_fts（本版本无回填路径，保留原索引）".to_string(),
                    "trajectory_skills_fts（本版本无回填路径，保留原索引）".to_string(),
                    "trajectory_messages_fts（本版本无回填路径，保留原索引）".to_string(),
                ],
                source_rows,
                indexed_rows,
            })
        })
        .await??;

        Ok(report)
    }
}

// ── 谱系与健康检查 DTO ──────────────────────────────────────────────

/// 带谱系的会话搜索结果
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct LineageSearchResult {
    pub result: FTS5Result,
    pub related_session_ids: Vec<String>,
    pub lineage_distance: u32,
}

/// FTS5 健康状态
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct FTS5Health {
    /// FTS5 子系统是否真正被挂载。
    ///
    /// 这一位是本次修复的核心补充。修复前，「FTS 不可用」（PG 后端下
    /// `fts_searcher` 为 `None`，见 `src/init/state.rs:151`）与「搜索无命中」
    /// 在 API 层完全同形 —— 都表现为 `Ok(Vec::new())`。
    /// 于是 UI 上的「0 条结果」是一句**无法证伪的话**：它既可能是真的没有匹配，
    /// 也可能是整个检索子系统从未工作过。这一位把它拆开。
    pub available: bool,
    /// `available == false` 时的原因（人类可读）
    pub unavailable_reason: Option<String>,
    pub tables_exist: bool,
    pub trajectories_count: u64,
    pub memory_items_count: u64,
    pub skills_count: u64,
    pub messages_count: u64,
    /// 轨迹记忆的**源表**行数（`memory_items` 中属于轨迹命名空间的行）。
    ///
    /// 必须与 `memory_items_count` 并排暴露：只给索引侧计数回答不了「有没有漏」，
    /// 而「有没有漏」正是「索引落后」的定义。消费方（UI / 诊断命令）
    /// 用 `source - indexed` 即得落后幅度。
    pub memory_items_source_count: u64,
    pub needs_rebuild: bool,
}

impl FTS5Health {
    /// 构造「FTS 未挂载」的健康状态。
    pub fn unavailable(reason: impl Into<String>) -> Self {
        Self {
            available: false,
            unavailable_reason: Some(reason.into()),
            tables_exist: false,
            trajectories_count: 0,
            memory_items_count: 0,
            skills_count: 0,
            messages_count: 0,
            memory_items_source_count: 0,
            // 不可用时**刻意不置** `needs_rebuild`。置 true 会诱导调用方去「重建」，
            // 而没有 searcher 时重建同样是 no-op —— 那会造出一个永远无法满足的标志位
            // （与本文件里 `needs_rebuild` 原来那个恒真缺陷同形态）。
            // 正确处置是先解决可用性（可用性由 `available` + `unavailable_reason` 表达）。
            needs_rebuild: false,
        }
    }
}

/// FTS5 重建结果。
///
/// 做成返回值而不是只打一行 `info!`：原实现无论实际做了什么（包括**什么都没回填**）
/// 都打印「rebuilt successfully」，调用方无法区分「重建成功」与「索引被清空」。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct FTS5RebuildReport {
    /// 完成「清空 + 回填」的表
    pub rebuilt: Vec<String>,
    /// 未重建的表及原因（本版本只有 `memory_items_fts` 具备回填路径）
    pub skipped: Vec<String>,
    /// 回填源行数
    pub source_rows: u64,
    /// 回填后索引行数
    pub indexed_rows: u64,
}

/// 把任意字节偏移向下对齐到最近的 char boundary，避免在多字节字符中间切片触发 panic。
/// 如果 `index` 已经超过字符串长度，返回字符串总长度。
fn floor_char_boundary(s: &str, index: usize) -> usize {
    if index >= s.len() {
        return s.len();
    }
    // 从 index 向前找最近的 char boundary
    let mut i = index;
    while i > 0 && !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::{Connection, params};
    use std::sync::Arc;
    use tokio::sync::Mutex;

    /// 与生产同源 —— 测试夹具也必须指向 harness 的权威定义，
    /// 否则「夹具用的 id」与「迁移写入的 id」会各自漂移而不报错。
    const NS: &str = axagent_harness::constants::sentinel::TRAJECTORY_MEM_NS_ID;

    async fn setup() -> (FTS5Search, Arc<Mutex<Connection>>) {
        let raw = Connection::open_in_memory().expect("in-memory sqlite");
        // 只建本模块回填需要的列，不引全量迁移 —— 否则本测试会变成迁移测试
        raw.execute_batch(
            "CREATE TABLE memory_items (\
                 id TEXT PRIMARY KEY NOT NULL,\
                 namespace_id TEXT NOT NULL,\
                 title TEXT NOT NULL,\
                 content TEXT NOT NULL,\
                 tags TEXT NOT NULL,\
                 updated_at TEXT NOT NULL\
             );",
        )
        .expect("create memory_items");
        let conn = Arc::new(Mutex::new(raw));
        let searcher = FTS5Search::new(conn.clone(), FTS5Config::default());
        searcher.create_fts_tables().await.expect("create fts tables");
        (searcher, conn)
    }

    async fn insert_memory(
        conn: &Arc<Mutex<Connection>>,
        id: &str,
        ns: &str,
        title: &str,
        tags: &str,
    ) {
        conn.lock()
            .await
            .execute(
                "INSERT INTO memory_items (id, namespace_id, title, content, tags, updated_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![id, ns, title, "正文内容", tags, "1700000000000"],
            )
            .expect("insert memory_items");
    }

    async fn table_row_count(conn: &Arc<Mutex<Connection>>, table: &str) -> u64 {
        conn.lock()
            .await
            .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get::<_, i64>(0))
            .unwrap_or(0) as u64
    }

    async fn table_exists(conn: &Arc<Mutex<Connection>>, table: &str) -> bool {
        conn.lock()
            .await
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name=?1",
                params![table],
                |r| r.get::<_, i64>(0),
            )
            .unwrap_or(0)
            > 0
    }

    /// 回归：重建**不得**删除没有回填路径的表。
    ///
    /// 修复前 `rebuild_indexes` 无条件 DROP 全部 4 张 FTS 表，且没有任何回填步骤，
    /// 然后返回 `Ok(())` 并打印「rebuilt successfully」。本测试同时钉住两件事：
    ///   ① 有回填路径的表被真正回填（计数相等）
    ///   ② 无回填路径的表**原样保留**（行数与存在性都不变）
    #[tokio::test]
    async fn rebuild_backfills_memory_items_and_preserves_unbackfillable_tables() {
        let (searcher, conn) = setup().await;

        insert_memory(&conn, "m1", NS, "preference", "[\"a\",\"b\"]").await;
        insert_memory(&conn, "m2", NS, "fact", "[]").await;
        // 另一命名空间的记忆不属于轨迹域，不应被回填
        insert_memory(&conn, "k1", "other_ns", "knowledge", "[]").await;

        // 往一张「本版本无回填路径」的表里塞一行，用来验证它不会被清空
        conn.lock()
            .await
            .execute(
                "INSERT INTO trajectories_fts (id, session_id, topic, summary, content, outcome, \
                 quality_score, created_at) VALUES ('t1','s1','topic','summary','body','success',1.0,0)",
                [],
            )
            .expect("seed trajectories_fts");

        let report = searcher.rebuild_indexes(NS).await.expect("rebuild");

        assert_eq!(report.rebuilt, vec!["memory_items_fts".to_string()]);
        assert_eq!(report.skipped.len(), 3, "三张无回填路径的表必须列入 skipped");
        assert_eq!(report.source_rows, 2, "只统计轨迹命名空间");
        assert_eq!(report.indexed_rows, 2);
        assert_eq!(table_row_count(&conn, "memory_items_fts").await, 2);

        // 关键否定断言：无回填路径的表既不能被删，也不能被清空
        assert!(table_exists(&conn, "trajectories_fts").await, "trajectories_fts 被删除");
        assert_eq!(
            table_row_count(&conn, "trajectories_fts").await,
            1,
            "trajectories_fts 被清空 —— 这正是修复前「重建 = 越修越坏」的形态"
        );
    }

    /// 回归：源行数与索引行数必须分开暴露，且 `needs_rebuild` 不能恒真。
    ///
    /// 修复前 `needs_rebuild = 四张索引表都为空`：全新空库也判 true（假警报且无法消除），
    /// 重建后索引为空又恒为 true（永远无法满足）。
    #[tokio::test]
    async fn health_separates_source_and_index_counts() {
        let (searcher, conn) = setup().await;
        insert_memory(&conn, "m1", NS, "preference", "[]").await;
        insert_memory(&conn, "m2", NS, "fact", "[]").await;

        // 回填前：有源数据但索引为空 ⇒ 确定无疑的落后
        let before = searcher.health_check(Some(NS)).await.expect("health");
        assert!(before.available);
        assert_eq!(before.memory_items_source_count, 2);
        assert_eq!(before.memory_items_count, 0);
        assert!(before.needs_rebuild, "有源数据而索引为空时应判需要重建");

        searcher.rebuild_indexes(NS).await.expect("rebuild");

        // 回填后：标志位必须能被满足（修复前这里恒为 true）
        let after = searcher.health_check(Some(NS)).await.expect("health");
        assert_eq!(after.memory_items_source_count, 2);
        assert_eq!(after.memory_items_count, 2);
        assert!(!after.needs_rebuild, "重建后 needs_rebuild 必须变为 false，否则是恒真标志位");
    }

    /// 未提供命名空间时不得伪造「索引落后」的判断。
    ///
    /// `memory_items` 是全项目共用的表，只有轨迹命名空间的行才归 `memory_items_fts`。
    /// 不做收口就会把知识库记忆也算进来，从而制造一个永远无法清除的落后判定。
    #[tokio::test]
    async fn health_without_namespace_does_not_fabricate_drift() {
        let (searcher, conn) = setup().await;
        insert_memory(&conn, "m1", NS, "preference", "[]").await;

        let h = searcher.health_check(None).await.expect("health");
        assert_eq!(h.memory_items_source_count, 0, "无命名空间时不得用全表计数");
        assert!(!h.needs_rebuild, "无命名空间时不得声称索引落后");
    }

    /// 不可用状态必须是**可观测的**，而不是伪装成「索引为空」。
    #[test]
    fn unavailable_health_is_explicit_and_does_not_claim_rebuild_needed() {
        let h = FTS5Health::unavailable("测试原因");
        assert!(!h.available);
        assert_eq!(h.unavailable_reason.as_deref(), Some("测试原因"));
        // 不可用时置 needs_rebuild 会诱导调用方去做一个同样是 no-op 的「重建」
        assert!(!h.needs_rebuild);
    }
}
