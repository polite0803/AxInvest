// SPDX-License-Identifier: AGPL-3.0-only

//! axagent-dao — 数据访问层
//!
//! 包含数据库连接管理 (db)、SeaORM 仓库 (repo/) 和 DDL 操作 (ddl)。
//! 也包含跨多个 repo 的服务（如 marketplace_service）— 这些服务全部是
//! SeaORM 数据访问逻辑，留在 dao 层。

// Re-export SeaORM entities for crates that go through the DAO layer
pub use axagent_entities;

pub mod agent_repositories;
pub mod background_task_repository;
pub mod cjk_ngram;
pub mod config;
pub mod conversation_repository;
pub mod db;
pub mod ddl;
pub mod generated_tool_repository;
pub mod integrity;
pub mod knowledge_crud_repository;
pub mod knowledge_graph_provider;
pub mod loop_checkpoint_repository;
pub mod marketplace_service;
pub mod memory_repository;
pub mod message_repository;
pub mod migrations;
pub mod platform_adapter_impl;
pub mod platform_config_repository;
pub mod provider_repository;
/// 声明式 schema 同步引擎（P2：L2 例外清单）。
///
/// ⚠ 2026-09-16 起**已接入启动路径，会影响运行时行为**：建表链是
/// `db::create_pool` → `initialize_schema` → [`reconcile::apply::bootstrap_schema`]，
/// 迁移清单清空后它是**唯一的建表来源**。启动路径接触引擎只能经这一个出口
/// （配置被钉死为「只做纯新增 + 真执行」，见 `bootstrap_options`），
/// 绕过它直接调 `apply::cycle` / `apply::run` 会被守卫
/// `reconcile::tests::only_bootstrap_may_reach_apply_from_startup` 拦下。
pub mod reconcile;
pub mod rl_experience_store;
pub mod seed;
pub mod sync_storage_impl;
pub use sync_storage_impl::SyncStorageDb;
pub mod repo;
pub mod search_sources_impl;
pub mod session_state_store;
pub use session_state_store::DaoSessionStateStore;
pub mod settings_repository;
pub mod stored_file_repository;
/// 任务账本唯一写路径（P1-C）：状态迁移校验 + 事件脊同事务写入。
pub mod task_ledger;
pub mod tool_execution_repository;
pub mod trajectory_repository;
pub mod workflow_conversions;
pub mod workflow_execution_repository;
pub mod workflow_template_repository;
pub use workflow_template_repository::DaoWorkflowTemplateRepository;
