// SPDX-License-Identifier: AGPL-3.0-only

//! SeaORM entity definitions for AxAgent database tables.

// Smart Router 路由历史（v100 consolidated migration 创建）
pub mod route_history;

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
pub mod credentials;
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

// Feedback data lake entities (v112)
pub mod memory_access_logs;
pub mod tool_call_logs;
pub mod wiki_edit_logs;

pub mod stored_files;

pub mod workflow_snapshots;

pub mod workflow_template;

pub mod workflow_template_version;

pub mod workflow_tools;

// 能力路由相关实体
pub mod capability_clusters;

pub mod prompt_template;
pub mod prompt_template_version;

pub mod agent_profiles;
pub mod agent_roles;
pub mod agent_sessions;

// Wave 3: Atomic Skill & Work Engine entities
pub mod capability_domain_overrides;
pub mod capability_policies;
pub mod capability_relationships;
pub mod capability_stats;
pub mod generated_tools;
pub mod session_events;
pub mod session_states;
pub mod workflow_approvals;
pub mod workflow_execution_stats;
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
// trajectory_entities/trajectory_relationships/trajectory_memories 已合并到 knowledge_entities/knowledge_relations/memory_items (v101)
pub mod trajectory_learned_patterns;
pub mod trajectory_messages;
pub mod trajectory_patterns;
pub mod trajectory_preferences;
pub mod trajectory_rewards;
pub mod trajectory_sessions;
pub mod trajectory_skill_executions;
pub mod trajectory_skills;
pub mod trajectory_steps;
pub mod trajectory_workflow_reflections;

// Dynamic UI entities
pub mod dynamic_ui_form_data;
pub mod dynamic_ui_pins;
pub mod dynamic_ui_schema_versions;
pub mod dynamic_ui_schemas;

// Index queue entities
pub mod index_jobs;

// Vector store entities
pub mod vec_collections;

// fleet_members / fleets 是 v102 创建的 AxAgent 通用实体
pub mod fleet_members;
// fleet_messages 是 v229 新增的群聊 / DM 消息持久化实体（协调门的地基）
pub mod fleet_messages;
pub mod fleets;

// Sync entities
pub mod sync_audit_log;
pub mod sync_change_log;
pub mod sync_device;
pub mod sync_history;
pub mod sync_permission;
pub mod sync_policy;

// Paper Overview Engine + Reading List & Queue
pub mod paper_overviews;
pub mod reading_list_items;
pub mod reading_lists;

// 叙事结构（v126）—— 文学创作工作流的弧线/交汇点/伏笔持久化
pub mod narrative_structures;

// OPC 需求发现（v131）—— 平台配置 + 需求线索持久化
pub mod opc_demand_leads;
pub mod opc_demand_platforms;
// OPC 需求发现（v133）—— 订阅词表
pub mod opc_demand_subscriptions;
// OPC 交付（v134）—— 发票账本
pub mod opc_invoices;

// === OPC 完整业务实体（本地独有的 OPC crate 依赖，上游合并时丢失）===
pub mod opc_automation_rules;
pub mod opc_blog_posts;
pub mod opc_capability_gap;
pub mod opc_capability_packs;
pub mod opc_contact_submissions;
pub mod opc_content_assets;
pub mod opc_customers;
pub mod opc_delivery;
pub mod opc_experience_records;
pub mod opc_follow_up_tasks;
pub mod opc_kpi_records;
pub mod opc_landing_pages;
pub mod opc_org_employees;
pub mod opc_org_roles;
pub mod opc_orgs;
pub mod opc_playbooks;
pub mod opc_projects;
pub mod opc_publish_schedules;
pub mod opc_revenue_records;
pub mod opc_talent_templates;
pub mod opc_work_items;

// === 股票业务 / 量化分析 / 投资组合实体（上游合并时丢失，cf923b07^ 恢复）===
pub mod analyst_feedback;
pub mod business_roles;
pub mod decision_validations;
pub mod earnings_events;
pub mod financial_snapshots;
pub mod fund_transfers;
pub mod lesson_applications;
pub mod market_mainlines;
pub mod news_archive;
pub mod opc_rl_experience;
pub mod opc_rl_training_stats;
pub mod paper_portfolios;
pub mod paper_positions;
pub mod portfolio_correlation_snapshot;
pub mod portfolio_holdings;
pub mod portfolio_metrics_daily;
pub mod price_alerts;
pub mod quant_paper_trades;
pub mod quant_runs;
pub mod quant_signals;
pub mod quant_strategies;
pub mod reco_picks;
pub mod reflection_lessons;
pub mod screenshot_diagnoses;
pub mod stock_analyses;
pub mod stock_evolution_history;
pub mod stock_pipeline_runs;
pub mod stock_reflections;
pub mod strategy_performance;
pub mod strategy_weight_history;
pub mod task_events;
pub mod trades;
pub mod watchlist_items;

// === 孤儿表实体化（2026-09-16）===
//
// 以下 4 张表原先只有迁移 DDL、无实体声明，持久化层用原生 SQL 手写
// SQLite / PostgreSQL 双方言分支（`?N` vs `$N`）。补实体后 schema 真相源
// 统一到实体声明，双方言分支随之消失。
//
// 触发背景：`PLAN-declarative-schema-sync.md` 的 P0 盘点发现 12 张表无实体，
// 其中本组 4 张是「有真实程序访问但走原生 SQL」，故补实体而非删表。
pub mod evolution_execution_stats;
pub mod loop_checkpoints;
pub mod semantic_cache;
pub mod wiki_graph_cache;

// === 主库活跃表的实体化（2026-09-16）===
//
// `cron_jobs` / `cron_job_history` 是主库里的活跃表（`init/state.rs` 建 store），
// 但此前只有 `runtime-core/src/cron_job.rs` 里的手写 DDL + 5 条按 backend 分支的
// 原生 SQL（`INSERT OR REPLACE` vs `ON CONFLICT`、`json_extract` vs `data::json->>`、
// `?` vs `$N`）。补实体后：建表走 `Schema::create_table_from_entity`，
// 读写走实体 API，方言分支整体消失。
pub mod cron_job;
pub mod cron_job_history;

// `gateway_message_queue` 原先定义在 `crates/runtime/src/persistent_queue.rs` ——
// **不在本 crate**，于是不进 `entity_modules!` 注册清单，schema 引擎看不见它。
// 全仓 185 个 `table_name` 注解中唯此 1 处越界（详见该实体文件头）。
pub mod gateway_message_queue;

// === 审计流水表的实体化（2026-09-16）===
//
// `audit_log` 原先只有 `tools/src/audit.rs` 里的内联 DDL（连迁移文件都没有），
// 且建表代码挂在 `AuditConfig.audit_db_path` 上——而该配置在生产路径恒为 `None`
// （`UnifiedToolRegistry::new()` 用 `ToolAuditor::default()`），于是表从未被创建。
// 补实体 + 改为进程级连接注册后，审计真实落库。
pub mod audit_log;

// === 侧车库表（index.db / disk-cache 自持 SQLite 文件）（2026-09-16）===
//
// 下列 8 张表原先各自只有「内联手写 DDL + rusqlite 原生 SQL」，散落在
// `search/src/file_index.rs`、`search/src/ast_index.rs`、`disk-cache/src/lib.rs`。
// 实体化后 schema 有了唯一真相源。
//
// ⚠ 它们**不在主库**：`file_index` / `ast_*` 落在 `src/indexing_triggers.rs` 的
// `INDEX_DB_FILENAME`（`index.db`）；`l2_*` 由 disk-cache 自持一个 SQLite 文件。
// 在此声明的是 schema，与连接指向哪个库无关。
//
// ⚠ **一模块一实体**是本 crate 的硬契约：`dao/build.rs` 扫描本 crate 的全部 `pub mod`
// 生成 `entity_modules!`，并展开为 `axagent_entities::$module::Entity`。
// 故这 8 张表必须落在 8 个独立模块里 —— 曾把 5 张 AST 表聚合进一个 `ast_index` 容器模块，
// 直接导致 `dao` 报 `cannot find type Entity in module axagent_entities::ast_index`。
//
// ⚠ 原第 9 张 `l2_summaries` 已于 2026-09-16 整链删除：它与主库 `conversation_summaries`
// 功能重复（后者字段更全且是权威真相源），且**零调用**、侧车库无 FK ⇒ 会话删除后
// 摘要必然残留为孤儿且无人清理。理由与零损失论证见 `disk-cache/src/lib.rs` 的 crate 文档。
pub mod ast_call_edges;
pub mod ast_classes;
pub mod ast_functions;
pub mod ast_interfaces;
pub mod ast_variables;
pub mod file_index;
pub mod l2_index_snapshots;
pub mod l2_search_results;

pub use sea_orm;
