// SPDX-License-Identifier: AGPL-3.0-only

//! SeaORM entity definitions for AxAgent database tables.

pub mod background_tasks;
pub mod conversation_categories;
pub mod conversation_summaries;
pub mod conversations;
pub mod desktop_state;
pub mod gateway_diagnostics;
pub mod gateway_keys;
pub mod gateway_link_activities;
pub mod gateway_link_policies;
pub mod gateway_links;
pub mod gateway_request_logs;
pub mod gateway_usage;
pub mod mcp_servers;
pub mod messages;
pub mod models;
pub mod program_policies;
pub mod provider_keys;
pub mod providers;
pub mod search_citations;
pub mod search_providers;
pub mod settings;
pub mod skill_states;
pub mod tool_descriptors;
pub mod tool_executions;

// Wave 2+ entities
pub mod artifacts;
pub mod backup_manifests;
pub mod backup_targets;
pub mod context_sources;
pub mod conversation_branches;
pub mod import_jobs;
pub mod knowledge_attributes;
pub mod knowledge_bases;
pub mod knowledge_documents;
pub mod knowledge_entities;
pub mod knowledge_flows;
pub mod knowledge_interfaces;
pub mod knowledge_relations;
pub mod memory_items;
pub mod memory_namespaces;
pub mod retrieval_hits;
pub mod rl_policies;

pub mod portfolio_holdings;
pub mod price_alerts;
pub mod reco_picks;
pub mod reflection_lessons;
pub mod stock_analyses;
pub mod stock_reflections;
pub mod trades;
pub mod watchlist_items;

pub mod stored_files;

pub mod workflow_snapshots;

pub mod workflow_template;

pub mod workflow_template_version;

pub mod prompt_template;
pub mod prompt_template_version;

pub mod agent_profiles;
pub mod agent_roles;
pub mod agent_sessions;

// Wave 3: Atomic Skill & Work Engine entities
pub mod generated_tools;
pub mod workflow_executions;
pub mod workflow_marketplace;
pub mod workflow_marketplace_review;

// Wiki / LLM Wiki entities
pub mod agency_experts;
pub mod note_backlinks;
pub mod note_links;
pub mod notes;
pub mod plans;
pub mod wiki_operations;
pub mod wiki_page_versions;
pub mod wiki_pages;
pub mod wiki_sources;
pub mod wiki_sync_queue;
pub mod wiki_templates;
pub mod wikis;

pub mod trajectories;
pub mod trajectory_entities;
pub mod trajectory_learned_patterns;
pub mod trajectory_memories;
pub mod trajectory_messages;
pub mod trajectory_patterns;
pub mod trajectory_preferences;
pub mod trajectory_relationships;
pub mod trajectory_rewards;
pub mod trajectory_sessions;
pub mod trajectory_skill_executions;
pub mod trajectory_skills;
pub mod trajectory_steps;

// R1: 复盘→进化闭环（策略表现 + 权重历史）
pub mod strategy_performance;
pub mod strategy_weight_history;

// R2: 组合监控（每日指标 + 两两相关性快照）
pub mod portfolio_correlation_snapshot;
pub mod portfolio_metrics_daily;

// R3: 数据层（估值带 + 财报日历）
pub mod earnings_events;
pub mod financial_snapshots;

// P6: 本地新闻语料库(as-of 模式 search_news 兜底)
pub mod news_archive;

// Quant: 量化交易 + 量化回测（4 张核心表）
pub mod quant_paper_trades;
pub mod quant_runs;
pub mod quant_signals;
pub mod quant_strategies;

pub use sea_orm;
