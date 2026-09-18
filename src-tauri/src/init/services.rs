// SPDX-License-Identifier: AGPL-3.0-only

use crate::AppState;
use crate::index_queue::IndexJobService;
use chrono;
use notify::{Event, RecursiveMode, Watcher};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::Emitter;

/// [AxAgent 残留移除] 原始 block_on_safe 函数已移除（AxAgent stock pipeline 专用）。
/// start_background_services 始终运行在 tokio runtime 上下文中，无需额外阻塞辅助。
pub async fn start_background_services(
    app: &tauri::AppHandle,
    state: &AppState,
    app_dir: std::path::PathBuf,
    _tray_language: String,
) {
    // [2026-09-09 修复] 恢复被误删的共享 Rhai Engine pm_* 注册（原
    // register_portfolio_mgr_rhai_functions，fork 清理「AxAgent 残留」时连调用带实现
    // 一起移除，只留下下方 doc 注释）。workflow code_executor 的 shared_rhai_engine
    // 仅通过此回调获得 pm_* 函数；缺失会导致 data-quality.rhai /
    // portfolio-mgr.rhai / portfolio-risk-gate.rhai 全部报
    // Function not found（2026-09-09 data-quality 节点 VALIDATION_FAILED 实证）。
    // 必须在任何工作流执行前调用（shared_rhai_engine 是 OnceLock 单例）。
    axagent_rt_workflow::work_engine::executors::register_shared_engine_initializer(Box::new(
        crate::commands::stock_workflow::rhai_pm::register_pm_functions,
    ));
    init_mcp_oauth(state);
    start_auto_backup(app, state, app_dir.clone());
    start_webdav_sync(app, state, app_dir.clone());
    #[cfg(not(mobile))]
    start_tray(app, &_tray_language);
    start_closed_loop_service(app, state);
    start_insight_generation(state);
    start_pattern_learning(state);
    start_cross_session_learning(state);
    start_rl_reward_computation(state, app_dir);
    start_batch_processing(state);
    start_user_profile_persistence(state);
    start_skill_evolution(state);
    start_dream_consolidation(state);
    start_dream_task_executor(state);
    start_coevolution_task_executor(state);
    start_pattern_analyzer_task_executor(state);
    start_insight_generator_task_executor(state);
    start_auto_tool_observation(state);
    start_text_grad_analysis(state);
    start_cron_scheduler(app, state).await;
    start_trigger_recovery(state);
    start_scheduler_recovery(app, state);
    start_approval_event_bridge(app, state);
    start_platform_adapters(state);
    start_skill_watcher(app, state);
    start_memory_decay_tick(state);
    start_memory_maintenance_tick(state);
    start_retrieval_feedback_tick(state);
    start_obsidian_vaults_registration(state);
    start_knowledge_consolidation_tick(state);
    start_trajectory_cleanup(state);
    start_session_state_cleanup_tick(state);
    start_index_job_service(app, state);
    start_plugins(state);

    // [2026-09-03 投资域接线恢复] 以下为 AxInvest 业务专属后台服务，
    // 此前被标记为「AxAgent 残留移除」，但它们是 AxInvest 投资域的核心功能：
    //   - start_realtime_monitor：价格告警轮询 + T+0 自动重跑触发（trigger_t0_rerun）
    //   - start_realtime_quote_watcher：实时行情推送（前端 stock-quote-update 事件）
    //   - start_batch_reflection：股票分析批量反思 cron（run_batch_reflection_inner）
    //   - start_demand_discovery_cron：OPC 需求发现定时扫描（run_demand_discovery_cron）
    //   - spawn_opc_workflows_seeding：OPC 域包/领域工作流模板种子化（ensure_opc_workflows_seeded）
    start_realtime_monitor(app);
    start_realtime_quote_watcher(app, state);
    start_batch_reflection(state);
    start_demand_discovery_cron(app, state);
    spawn_opc_workflows_seeding(state);
    // register_dojo_sdk_executor 中的 DojoSdkExecutorImpl 注册（依赖 astock_client）已移除，
    // 仅保留 Plan 三件套正常后台任务——PLANS_REGISTRY TTL 清理
    crate::commands::dojo_sdk::spawn_plan_ttl_cleanup(state.shutdown_token.clone());

    // PTY 事件转发器
    #[cfg(not(mobile))]
    start_pty_event_forwarder(app, state);
}

/// 启动时批量注册 ConnectedVault 类型 KB 到全局 VaultRegistry。
///
/// 此前 `register_vault` 仅在创建/转换 KB 时调用，应用重启后
/// VaultRegistry 为空，导致 9 个 `obsidian_*` 工具全部报 `NotBound` 错误。
/// 本函数在启动后异步查询所有 `kind = connected_vault` 且 `enabled = true` 的 KB，
/// 重新注册到 VaultRegistry，修复 Obsidian 集成链路断裂问题。
fn start_obsidian_vaults_registration(state: &AppState) {
    let harness_state = state.harness.clone();
    tauri::async_runtime::spawn(async move {
        // 延迟 2 秒，确保数据库初始化完成
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;

        let db = harness_state.db();
        match axagent_dao::repo::knowledge::list_knowledge_bases(db).await {
            Ok(all_kbs) => {
                // 用 filter_map 同时过滤 + 提取 vault_path，避免后续 unwrap() panic
                let vault_kbs: Vec<_> = all_kbs
                    .iter()
                    .filter_map(|kb| {
                        if kb.enabled && matches!(kb.kind, axagent_harness::KbKind::ConnectedVault)
                        {
                            kb.vault_path.as_ref().map(|path| (kb, path.clone()))
                        } else {
                            None
                        }
                    })
                    .collect();

                if vault_kbs.is_empty() {
                    tracing::info!("[obsidian] 启动时未发现 ConnectedVault KB，跳过注册");
                    return;
                }

                let mut registered = 0usize;
                let mut failed = 0usize;
                for (kb, vault_path) in &vault_kbs {
                    let root = std::path::PathBuf::from(vault_path);
                    match axagent_tools::tools::obsidian::register_vault(&kb.id, root) {
                        Ok(()) => {
                            tracing::info!(
                                "[obsidian] 启动注册 ConnectedVault KB: id={} name={} vault={}",
                                kb.id,
                                kb.name,
                                vault_path
                            );
                            registered += 1;
                        },
                        Err(e) => {
                            tracing::warn!(
                                "[obsidian] 启动注册 ConnectedVault KB 失败: id={} name={} error={}",
                                kb.id,
                                kb.name,
                                e
                            );
                            failed += 1;
                        },
                    }
                }
                tracing::info!(
                    "[obsidian] 启动批量注册完成：成功 {} 个，失败 {} 个，总计 {} 个",
                    registered,
                    failed,
                    vault_kbs.len()
                );
            },
            Err(e) => {
                tracing::warn!("[obsidian] 启动时查询 knowledge_bases 失败: {}", e);
            },
        }
    });
}

/// 知识转换定时任务：定期将 Wiki/Memory 中的实体回流到知识图谱。
///
/// 解决"三套实体系统（Wiki 笔记、Memory 记忆、Knowledge 实体）各自为政"的问题：
/// 1. Wiki → Knowledge：查询所有 ConnectedVault KB，触发实体抽取（已存在的 extract_entities_from_wiki）
/// 2. Memory → Knowledge：将 Memory 中的高重要性条目转换为知识图谱实体
/// 3. 跨源实体合并：调用 merge_duplicate_entities_across_all 去重
///
/// 每 6 小时执行一次，避免频繁 LLM 调用。
/// 失败时仅记录警告，不影响主流程。
fn start_knowledge_consolidation_tick(state: &AppState) {
    let harness_state = state.harness.clone();
    tauri::async_runtime::spawn(async move {
        // 延迟 30 秒，确保所有启动任务完成
        tokio::time::sleep(std::time::Duration::from_secs(30)).await;
        let interval = std::time::Duration::from_secs(6 * 3600); // 6 小时
        loop {
            tokio::time::sleep(interval).await;

            tracing::info!("[knowledge_consolidation] 开始知识转换周期");
            let started = std::time::Instant::now();

            // ── 步骤 0：域一致性体检（只读；2026-09-17 新增） ──
            //
            // 放在合并**之前**：合并是唯一会改变域的步骤，体检若放在它后面，
            // 「合并把域弄坏了、又因不变量回滚」这类情况就只能看到回滚日志，
            // 看不到「库当前长什么样」。放前面 ⇒ 每轮都有一份「此刻的域状态」基线。
            //
            // 为什么值得每轮花一次全表扫描（72k 实体 + 113k 边）：这类损坏
            // （边的 `knowledge_base_id` ≠ 端点所属域）在库层面**无任何症状** ——
            // 外键不违反、行数不少、没有任何报错，唯一暴露它的是「按域算有多少条边
            // 画得出来」。实测曾让一个 wiki 的 38,171 条边全部不可见而无人知晓。
            match axagent_dao::repo::knowledge_graph::audit_kb_domain_consistency(
                harness_state.db(),
            )
            .await
            {
                Ok(rows) => {
                    let broken: Vec<_> = rows.iter().filter(|r| r.dangling > 0).collect();
                    if broken.is_empty() {
                        tracing::info!(
                            domains = rows.len(),
                            "[knowledge_consolidation] 域一致性体检：全部通过"
                        );
                    } else {
                        tracing::warn!(
                            broken_domains = broken.len(),
                            total_domains = rows.len(),
                            "[knowledge_consolidation] 域一致性体检：存在悬空边的域（这些边在任何按域过滤的读端上都画不出来）"
                        );
                        for r in broken {
                            tracing::warn!(
                                kb = %r.knowledge_base_id,
                                visible_entities = r.visible_entities,
                                edges = r.edges,
                                edges_both_visible = r.edges_both_visible,
                                edges_both_missing = r.edges_both_missing,
                                dangling = r.dangling,
                                ratio = r.dangling_ratio(),
                                "[knowledge_consolidation] 域悬空明细"
                            );
                        }
                    }
                },
                Err(e) => {
                    tracing::warn!("[knowledge_consolidation] 域一致性体检失败: {}", e);
                },
            }

            // ── 步骤 1：跨源实体合并（轻量，纯数据库操作） ──
            match axagent_dao::repo::knowledge_graph::merge_duplicate_entities_across_all(
                harness_state.db(),
            )
            .await
            {
                Ok(result) => {
                    if result.groups_found > 0 || result.cross_kb_groups_skipped > 0 {
                        tracing::info!(
                            "[knowledge_consolidation] 同域实体合并：{} 个分组，{} 个实体合并，{} 个关系更新，跨库分组跳过 {} 个，悬空边 {} -> {}",
                            result.groups_found,
                            result.entities_merged,
                            result.relations_updated,
                            result.cross_kb_groups_skipped,
                            result.dangling_before,
                            result.dangling_after
                        );
                    }
                },
                Err(e) => {
                    // ⚠ 这里现在也会接到「域一致性验收失败 ⇒ 已回滚」的错误。
                    //   那条错误意味着**有人把跨库合并又打开了**（或出现了同型的新路径）——
                    //   它不是噪声，需要有人去看。故用 `error` 而不是 `warn`。
                    tracing::error!("[knowledge_consolidation] 同域实体合并失败: {}", e);
                },
            }

            // ── 步骤 2：Wiki → Knowledge 实体抽取（重量级，需要 LLM） ──
            // 查询所有 ConnectedVault KB，逐个触发实体抽取
            match axagent_dao::repo::knowledge::list_knowledge_bases(harness_state.db()).await {
                Ok(all_kbs) => {
                    let vault_kbs: Vec<_> = all_kbs
                        .iter()
                        .filter(|kb| {
                            kb.enabled
                                && matches!(kb.kind, axagent_harness::KbKind::ConnectedVault)
                                && kb.vault_path.is_some()
                        })
                        .collect();

                    for kb in &vault_kbs {
                        tracing::info!(
                            "[knowledge_consolidation] 处理 ConnectedVault KB: id={} name={}",
                            kb.id,
                            kb.name
                        );
                        // 使用已有的 extract_entities_from_wiki 逻辑
                        // 通过 index_job 机制异步执行（避免阻塞定时任务）
                        let metadata = serde_json::json!({
                            "auto_extract": true,
                            "triggered_by": "consolidation_tick",
                        });
                        let input = axagent_dao::repo::index_jobs::CreateIndexJobInput {
                            job_type: axagent_dao::repo::index_jobs::JOB_TYPE_EXTRACT_ENTITIES
                                .to_string(),
                            container_type: "Wiki".to_string(),
                            container_id: kb.id.clone(),
                            item_id: kb.id.clone(),
                            max_retries: Some(1),
                            priority: Some(5),
                            metadata: Some(serde_json::to_string(&metadata).unwrap_or_default()),
                        };
                        let _ =
                            axagent_dao::repo::index_jobs::enqueue_job(harness_state.db(), input)
                                .await
                                .map_err(|e| {
                                    tracing::warn!(
                                        "[knowledge_consolidation] 队列实体抽取任务失败 kb={}: {}",
                                        kb.id,
                                        e
                                    );
                                    e
                                });
                    }
                },
                Err(e) => {
                    tracing::warn!("[knowledge_consolidation] 查询 ConnectedVault KB 失败: {}", e);
                },
            }

            // ── 步骤 3：Memory → Knowledge 实体回流 ──
            //
            // ⚠ 2026-09-17：本段原先在此**就地实现**，与
            //   `src/commands/knowledge_graph.rs` 的 Tauri 命令 `sync_memory_to_knowledge_graph`
            //   **逐字重复**，且两处都把 `memory_namespaces.id` 当作 `knowledge_bases.id` 用
            //   —— 那是**两个 id 空间**（都是 TEXT，类型系统拦不住）⇒
            //   `knowledge_entities.knowledge_base_id` 的外键违反 ⇒ 失败又被 `debug!` 吞掉
            //   ⇒ 整条路径在 PG 上**静默空转**（判据：fail-silent 是 fail-open 的镜像形态）。
            //
            //   现收敛到 `dao::repo::knowledge_graph::reflow_memory_to_knowledge` 单一实现，
            //   KB 归属改走**哨兵常量**（该 KB 行由 `dao::seed::ensure_sentinels` 幂等播种）。
            match axagent_dao::repo::knowledge_graph::reflow_memory_to_knowledge(
                harness_state.db(),
                axagent_harness::constants::sentinel::MEMORY_REFLOW_KB_ID,
                axagent_dao::repo::knowledge_graph::REFLOW_IMPORTANCE_THRESHOLD,
                axagent_dao::repo::knowledge_graph::REFLOW_MAX_ITEMS,
            )
            .await
            {
                Ok(stats) => {
                    // 逐条失败已在 dao 内汇总成**一条** warn（避免 100 行/轮 噪声）；
                    // 这里只报总数，便于与那一行 warn 对照。
                    tracing::info!(
                        "[knowledge_consolidation] Memory→Knowledge 回流：读取 {} 条，成功 {}，失败 {}",
                        stats.items_read,
                        stats.entities_created,
                        stats.failures
                    );
                },
                Err(e) => {
                    // ⚠ 这条是「本轮整段没跑」（查询/连接失败），与「部分条目失败」不同层级，
                    //   故由调用点发 warn —— dao 内那条 warn 只覆盖逐条失败。
                    tracing::warn!(
                        "[knowledge_consolidation] Memory→Knowledge 回流未能执行: {}",
                        e
                    );
                },
            }

            // ── 步骤 4：Agent 工具调用结果 → Memory 沉淀 ──
            // 扫描最近 24 小时的对话，将工具结果（WebSearch/CodeInterpreter 等）
            // 自动沉淀为 Memory 条目，让 Agent 的执行结果可被后续 RAG 检索使用
            match axagent_dao::repo::memory::deposit_tool_results_from_recent_messages(
                harness_state.db(),
                Some(24),
            )
            .await
            {
                Ok(count) if count > 0 => {
                    tracing::info!(
                        "[knowledge_consolidation] 工具结果沉积：{} 条新 Memory 条目",
                        count
                    );
                },
                Ok(_) => {
                    // 无新条目，静默跳过
                },
                Err(e) => {
                    tracing::warn!("[knowledge_consolidation] 工具结果沉积失败: {}", e);
                },
            }

            // ── 步骤 5：KB 文档 → Wiki 自动同步 ──
            // 对 ConnectedVault KB 中新增的文档，自动在 Wiki 中创建对应笔记
            // 形成"KB↔Wiki"双向同步闭环
            if let Ok(all_kbs) =
                axagent_dao::repo::knowledge::list_knowledge_bases(harness_state.db()).await
            {
                use axagent_harness::note_dtos::CreateNoteInput;

                let vault_kbs: Vec<_> = all_kbs
                    .iter()
                    .filter(|kb| {
                        kb.enabled
                            && matches!(kb.kind, axagent_harness::KbKind::ConnectedVault)
                            && kb.vault_path.is_some()
                    })
                    .collect();

                for kb in &vault_kbs {
                    if let Ok(docs) =
                        axagent_dao::repo::knowledge::list_documents(harness_state.db(), &kb.id)
                            .await
                    {
                        let vault_id = kb.vault_path.as_deref().unwrap_or("");
                        let mut synced = 0usize;
                        for doc in &docs {
                            // 检查文档是否已同步到 Wiki
                            let already_synced = axagent_dao::repo::wiki::note_exists_for_document(
                                harness_state.db(),
                                vault_id,
                                &doc.id,
                            )
                            .await
                            .unwrap_or(false);

                            if !already_synced {
                                // 创建 Wiki 笔记
                                let input = CreateNoteInput {
                                    vault_id: vault_id.to_string(),
                                    title: doc.title.clone(),
                                    file_path: doc.source_path.clone(),
                                    content: String::new(),
                                    author: "system".to_string(),
                                    page_type: Some("knowledge_document".to_string()),
                                    source_refs: Some(vec![format!("kb:{}:doc:{}", kb.id, doc.id)]),
                                };
                                match axagent_dao::repo::note::create_note(
                                    harness_state.db(),
                                    input,
                                )
                                .await
                                {
                                    Ok(_) => synced += 1,
                                    Err(e) => {
                                        tracing::debug!(
                                            "[knowledge_consolidation] Wiki 同步失败 doc={}: {}",
                                            doc.id,
                                            e
                                        );
                                    },
                                }
                            }
                        }
                        if synced > 0 {
                            tracing::info!(
                                "[knowledge_consolidation] KB→Wiki 同步：kb={} 新增 {} 篇笔记",
                                kb.id,
                                synced
                            );
                        }
                    }
                }
            }

            tracing::info!(
                "[knowledge_consolidation] 知识转换周期完成，耗时 {}ms",
                started.elapsed().as_millis()
            );
        }
    });
    tracing::info!("[knowledge_consolidation] 知识转换定时任务已启动（每 6 小时）");
}

/// PTY 事件转发器：从 PtyManager 的 mpsc 通道消费输出/退出事件，
/// 通过 Tauri 事件总线 emit 到前端（事件名 `pty_output` / `pty_exit`）。
#[cfg(not(mobile))]
fn start_pty_event_forwarder(app: &tauri::AppHandle, state: &AppState) {
    use axagent_runtime::pty::{PtyExitEvent, PtyOutputEvent};

    let app_handle_output = app.clone();
    let pty_manager_output = state.pty_manager.clone();
    tauri::async_runtime::spawn(async move {
        loop {
            let event: Option<PtyOutputEvent> = pty_manager_output.recv_output().await;
            let Some(event) = event else {
                break;
            };
            if let Err(e) = app_handle_output.emit("pty_output", event) {
                tracing::warn!("pty_output emit failed: {}", e);
            }
        }
    });

    let app_handle_exit = app.clone();
    let pty_manager_exit = state.pty_manager.clone();
    tauri::async_runtime::spawn(async move {
        loop {
            let event: Option<PtyExitEvent> = pty_manager_exit.recv_exit().await;
            let Some(event) = event else {
                break;
            };
            if let Err(e) = app_handle_exit.emit("pty_exit", event) {
                tracing::warn!("pty_exit emit failed: {}", e);
            }
        }
    });
}

/// 初始化 MCP OAuth 全局凭据存储。
///
/// （历史备注）本函数头上曾挂着一段 P1-D10 的 doc 注释，描述
/// 「在应用启动时调用 register_shared_engine_initializer 注入 pm_* 函数」，
/// 但对应的调用在 fork 清理「AxAgent 残留」时被整体移除，只剩注释——
/// 2026-09-09 已在 `start_background_services` 开头恢复该调用，
/// pm_* 注册的权威实现见
/// `crate::commands::stock_workflow::rhai_pm::register_pm_functions`。
fn init_mcp_oauth(state: &AppState) {
    let master_key = state.harness.master_key_owned();
    let crypto = std::sync::Arc::new(
        axagent_crypto::platform_adapter_impl::DefaultCryptoService::new(master_key),
    );
    let store = std::sync::Arc::new(axagent_mcp::mcp_oauth::McpOAuthStore::new(crypto));
    axagent_mcp::mcp_oauth::McpOAuthStore::init_global(store);
    tracing::info!("[McpOAuth] 全局 OAuth 凭据存储已初始化");
}

fn start_plugins(state: &AppState) {
    let plugin_manager = state.plugin_manager.clone();
    let dashboard_registry = state.dashboard_registry.clone();

    tauri::async_runtime::spawn(async move {
        tracing::info!("Initializing plugin system...");

        let mut manager = plugin_manager.write().await;
        let _started = match manager.start_enabled_plugins() {
            Ok(started) => {
                if !started.is_empty() {
                    tracing::info!(
                        "Started {} enabled plugin(s): {}",
                        started.len(),
                        started.join(", ")
                    );
                } else {
                    tracing::info!("No enabled plugins to start");
                }
                started
            },
            Err(e) => {
                tracing::error!("Failed to start enabled plugins: {e}");
                Vec::new()
            },
        };

        drop(manager);

        if let Some(registry) = dashboard_registry {
            if let Err(e) = registry.reload().await {
                tracing::warn!("Failed to reload dashboard plugins: {e}");
            } else {
                let count = registry.list_plugins().await.len();
                tracing::info!("Loaded {count} dashboard plugin(s)");
            }
        }

        tracing::info!("Plugin system initialization complete");
    });
}

fn start_auto_backup(app: &tauri::AppHandle, state: &AppState, app_dir: std::path::PathBuf) {
    let db = state.harness.db().clone();
    let app_data = app_dir.clone();
    let handle = state.auto_backup_handle.clone();
    let shutdown_token = state.shutdown_token.clone();
    let app_for_emit = app.clone();
    tauri::async_runtime::spawn(async move {
        if let Ok(settings) = axagent_dao::repo::settings::get_settings(&db).await {
            if settings.auto_backup_enabled && settings.auto_backup_interval_hours > 0 {
                let backup_dir_setting =
                    axagent_storage::path_vars::decode_path_opt(&settings.backup_dir);
                let interval = settings.auto_backup_interval_hours;
                let max_count = settings.auto_backup_max_count;
                let interval_secs = interval as u64 * 3600;
                let db2 = db.clone();
                let app_dir2 = app_data.clone();
                let shutdown_token = shutdown_token.clone();
                let app_for_backup = app_for_emit.clone();

                let initial_delay_secs = match axagent_dao::repo::backup::list_backups(
                    &db,
                    &axagent_storage::DefaultPathEncoder,
                )
                .await
                {
                    Ok(backups) if !backups.is_empty() => {
                        let last_ts = &backups[0].created_at;
                        if let Ok(last_time) =
                            chrono::NaiveDateTime::parse_from_str(last_ts, "%Y-%m-%d %H:%M:%S")
                        {
                            let elapsed = chrono::Utc::now()
                                .naive_utc()
                                .signed_duration_since(last_time)
                                .num_seconds()
                                .max(0) as u64;
                            interval_secs.saturating_sub(elapsed)
                        } else {
                            interval_secs
                        }
                    },
                    _ => interval_secs,
                };

                let task = tokio::spawn(async move {
                    let dur = std::time::Duration::from_secs(interval_secs);
                    tokio::time::sleep(std::time::Duration::from_secs(initial_delay_secs)).await;
                    loop {
                        tokio::select! {
                            _ = shutdown_token.cancelled() => {
                                tracing::info!("[auto_backup] 收到关闭信号，停止自动备份");
                                break;
                            }
                            _ = tokio::time::sleep(dur) => {
                                let backup_dir = axagent_dao::repo::backup::resolve_backup_dir(
                                    backup_dir_setting.as_deref(),
                                    &app_dir2,
                                );
                                if let Err(e) =
                                    axagent_dao::repo::backup::create_backup(&db2, "sqlite", &backup_dir, &axagent_storage::DefaultPathEncoder)
                                        .await
                                {
                                    tracing::warn!("Auto-backup failed: {}", e);
                                    let _ = app_for_backup.emit("auto-backup-completed", serde_json::json!({
                                        "success": false,
                                        "error": e.to_string(),
                                    }));
                                } else {
                                    tracing::info!("Auto-backup created");
                                    let _ = app_for_backup.emit("auto-backup-completed", serde_json::json!({
                                        "success": true,
                                        "message": "Auto-backup created successfully",
                                    }));
                                    let _ =
                                        axagent_dao::repo::backup::cleanup_old_backups(&db2, max_count, &axagent_storage::DefaultPathEncoder)
                                            .await;
                                }
                            }
                        }
                    }
                });
                *handle.lock().await = Some(task);
            }
        }
    });
}

fn start_memory_maintenance_tick(state: &AppState) {
    let memory_service = state.memory_service.clone();
    let token = state.shutdown_token.clone();
    state.task_manager.spawn("memory_maintenance", async move {
        let interval = std::time::Duration::from_secs(7200);
        loop {
            tokio::select! {
                _ = token.cancelled() => {
                    tracing::info!("[memory_maintenance] 收到关闭信号");
                    break;
                }
                _ = tokio::time::sleep(interval) => {
                    let ms = memory_service.read().await;
                    let disambiguation = ms.disambiguate_entities().await;
                    drop(ms);
                    if disambiguation.merged > 0 {
                        tracing::info!(
                            "[memory_maintenance] Disambiguated entities: merged {} of {}",
                            disambiguation.merged,
                            disambiguation.total
                        );
                    }
                    let ms = memory_service.read().await;
                    let clusters = ms.find_similar_clusters(0.75).await;
                    drop(ms);
                    if !clusters.is_empty() {
                        tracing::info!(
                            "[memory_maintenance] Found {} similar memory clusters (potential duplicates)",
                            clusters.len()
                        );
                    }
                }
            }
        }
    });
}

/// 反思工作流定时任务：定期扫描 pending row 并执行反思。
///
/// 每 6 小时运行一次，首次延迟 60 秒（避免启动时抢资源）。
/// 调用 `run_batch_reflection_inner` 处理最多 20 条 pending row。
/// 监听 `shutdown_token` 支持优雅关闭。
fn start_platform_adapters(state: &AppState) {
    let platform_manager = state.platform_manager.clone();
    let db = state.harness.db().clone();

    tauri::async_runtime::spawn(async move {
        let config = axagent_dao::repo::platform_config::get_platform_config(&db).await;
        match platform_manager.reconcile(&config).await {
            Ok(report) => {
                if !report.started.is_empty() {
                    tracing::info!(
                        "[PlatformManager] boot reconcile: started {:?}",
                        report.started
                    );
                }
                if !report.errors.is_empty() {
                    for (name, err) in &report.errors {
                        tracing::error!(
                            "[PlatformManager] boot reconcile: {} error: {}",
                            name,
                            err
                        );
                    }
                }
            },
            Err(e) => {
                tracing::error!("[PlatformManager] boot reconcile failed: {}", e);
            },
        }
    });
}

fn start_webdav_sync(app: &tauri::AppHandle, state: &AppState, app_dir: std::path::PathBuf) {
    let db = state.harness.db().clone();
    let master_key = state.harness.master_key_owned();
    let app_data_dir = app_dir.clone();
    let handle = state.webdav_sync_handle.clone();
    let shutdown_token = state.shutdown_token.clone();
    let app_clone = app.clone();
    tauri::async_runtime::spawn(async move {
        if let Ok(settings) = axagent_dao::repo::settings::get_settings(&db).await {
            if settings.webdav_sync_enabled && settings.webdav_sync_interval_minutes > 0 {
                let db2 = db.clone();
                let interval = settings.webdav_sync_interval_minutes;
                let interval_secs = interval as u64 * 60;

                let initial_delay_secs =
                    match axagent_dao::repo::settings::get_setting(&db, "webdav_last_sync_time")
                        .await
                    {
                        Ok(Some(ts)) => {
                            if let Ok(last_time) = chrono::DateTime::parse_from_rfc3339(&ts) {
                                let elapsed = chrono::Utc::now()
                                    .signed_duration_since(last_time)
                                    .num_seconds()
                                    .max(0) as u64;
                                interval_secs.saturating_sub(elapsed)
                            } else {
                                interval_secs
                            }
                        },
                        _ => interval_secs,
                    };

                let task = crate::commands::webdav::spawn_webdav_sync_task(
                    app_clone,
                    db2,
                    master_key,
                    app_data_dir,
                    interval,
                    initial_delay_secs,
                    shutdown_token,
                );
                *handle.lock().await = Some(task);
            }
        }
    });
}

#[cfg(not(mobile))]
fn start_tray(app: &tauri::AppHandle, tray_language: &str) {
    if let Err(e) = crate::tray::create_tray(app, tray_language) {
        tracing::warn!("Failed to create system tray: {}", e);
    }
}

fn start_closed_loop_service(_app: &tauri::AppHandle, state: &AppState) {
    let db = state.harness.db().clone();
    let closed_loop = state.closed_loop_service.clone();
    let nudge_service = state.nudge_service.clone();
    let saliency_arbiter = state.memory.saliency_arbiter.clone();
    let saliency_enabled = state.memory.saliency_enabled.clone();
    tauri::async_runtime::spawn(async move {
        if let Ok(settings) = axagent_dao::repo::settings::get_settings(&db).await {
            if settings.closed_loop_enabled {
                closed_loop.start();
                let interval_minutes = settings.closed_loop_interval_minutes.max(1);
                let interval = std::time::Duration::from_secs(interval_minutes as u64 * 60);
                loop {
                    tokio::time::sleep(interval).await;
                    let new_nudges: Vec<axagent_trajectory::PeriodicNudge> =
                        closed_loop.tick().await;
                    if !new_nudges.is_empty() {
                        tracing::info!(
                            "[closed_loop] Generated {} periodic nudges",
                            new_nudges.len()
                        );
                        let mut candidates: Vec<axagent_trajectory::NudgeCandidate> = new_nudges
                            .iter()
                            .map(|pn| axagent_trajectory::NudgeCandidate {
                                entity: axagent_trajectory::NudgeEntity {
                                    id: pn.id.clone(),
                                    name: pn.title.clone(),
                                    entity_type: format!("{:?}", pn.nudge_type),
                                    confidence: if pn.urgency == "high" {
                                        0.9
                                    } else if pn.urgency == "medium" {
                                        0.7
                                    } else {
                                        0.5
                                    },
                                },
                                reason: pn.description.clone(),
                                urgency: match pn.urgency.as_str() {
                                    "high" => axagent_trajectory::Urgency::High,
                                    "medium" => axagent_trajectory::Urgency::Medium,
                                    _ => axagent_trajectory::Urgency::Low,
                                },
                                suggested_action: Some(pn.suggested_action.clone()),
                            })
                            .collect();

                        // 显著性仲裁（默认关闭；开启时仅广播胜者进入 nudge 生成）
                        if saliency_enabled.load(std::sync::atomic::Ordering::Relaxed) {
                            let signals = candidates
                                .iter()
                                .map(|c| {
                                    axagent_trajectory::SaliencySignal::new(
                                        axagent_trajectory::SignalSource::Nudge,
                                        c.entity.confidence,
                                        c.entity.id.clone(),
                                    )
                                })
                                .collect();
                            let packet = saliency_arbiter.lock().await.arbitrate(signals);
                            let winner_ids: std::collections::HashSet<String> =
                                packet.winners.iter().map(|w| w.signal.origin_id.clone()).collect();
                            candidates.retain(|c| winner_ids.contains(&c.entity.id));
                        }

                        let mut ns: tokio::sync::MutexGuard<'_, axagent_trajectory::NudgeService> =
                            nudge_service.lock().await;
                        let ctx = axagent_trajectory::NudgeContext {
                            current_task: None,
                            recent_entities: None,
                            session_id: "closed_loop_bg".to_string(),
                        };
                        ns.generate_nudges(ctx, candidates);
                    }
                }
            }
        }
    });
}

fn start_insight_generation(state: &AppState) {
    let realtime_learning = state.realtime_learning.clone();
    let insight_system = state.insight_system.clone();
    tauri::async_runtime::spawn(async move {
        let interval = std::time::Duration::from_secs(10 * 60);
        loop {
            tokio::time::sleep(interval).await;
            let new_insights = {
                let rl: tokio::sync::MutexGuard<'_, axagent_trajectory::RealTimeLearning> =
                    realtime_learning.lock().await;
                rl.generate_insights()
            };
            if !new_insights.is_empty() {
                tracing::info!(
                    "[insight] Generated {} learning insights from feedback",
                    new_insights.len()
                );
                let mut is = insight_system.write().await;
                for insight in new_insights {
                    is.add_insight(insight);
                }
            }
        }
    });
}

fn start_pattern_learning(state: &AppState) {
    let trajectory_storage = state.trajectory_storage.clone();
    let pattern_learner = state.pattern_learner.clone();
    tauri::async_runtime::spawn(async move {
        let interval = std::time::Duration::from_secs(15 * 60);
        loop {
            tokio::time::sleep(interval).await;
            let trajectories: Vec<axagent_trajectory::Trajectory> =
                match trajectory_storage.get_trajectories(Some(20)).await {
                    Ok(t) => t,
                    Err(e) => {
                        tracing::warn!("[pattern] Failed to fetch trajectories: {}", e);
                        continue;
                    },
                };
            if trajectories.is_empty() {
                continue;
            }
            let mut pl = pattern_learner.write().await;
            let new_patterns = pl.update_from_batch(&trajectories);
            drop(pl);
            if !new_patterns.is_empty() {
                tracing::info!(
                    "[pattern] Learned {} new patterns from {} trajectories",
                    new_patterns.len(),
                    trajectories.len()
                );
                for pattern in &new_patterns {
                    if let Err(e) = trajectory_storage.save_pattern(pattern).await {
                        tracing::warn!("[pattern] Failed to persist pattern: {}", e);
                    }
                }
            }
        }
    });
}

fn start_cross_session_learning(state: &AppState) {
    let trajectory_storage = state.trajectory_storage.clone();
    let cross_session_learner = state.cross_session_learner.clone();
    let insight_system = state.insight_system.clone();
    tauri::async_runtime::spawn(async move {
        let interval = std::time::Duration::from_secs(30 * 60);
        loop {
            tokio::time::sleep(interval).await;
            let trajectories: Vec<axagent_trajectory::Trajectory> =
                match trajectory_storage.get_trajectories(Some(50)).await {
                    Ok(t) => t,
                    Err(e) => {
                        tracing::warn!("[cross_session] Failed to fetch trajectories: {}", e);
                        continue;
                    },
                };
            if trajectories.len() < 3 {
                continue;
            }
            let mut by_session: std::collections::HashMap<
                String,
                Vec<axagent_trajectory::Trajectory>,
            > = std::collections::HashMap::new();
            for t in trajectories {
                by_session.entry(t.session_id.clone()).or_default().push(t);
            }
            if by_session.len() < 2 {
                continue;
            }
            let mut csl = cross_session_learner.write().await;
            let new_patterns = csl.learn_from_sessions(by_session);
            drop(csl);
            if !new_patterns.is_empty() {
                tracing::info!(
                    "[cross_session] Discovered {} cross-session patterns",
                    new_patterns.len()
                );
                let mut is = insight_system.write().await;
                for pattern in &new_patterns {
                    if let Err(e) = trajectory_storage.save_pattern(pattern).await {
                        tracing::warn!("[cross_session] Failed to persist pattern: {}", e);
                    }
                    if pattern.success_rate >= 0.7 && pattern.frequency >= 3 {
                        is.add_insight(axagent_trajectory::LearningInsight {
                            id: format!("cs_{}", pattern.id),
                            category: axagent_trajectory::InsightCategory::Pattern,
                            title: format!("Cross-session pattern: {}", pattern.name),
                            description: pattern.description.clone(),
                            confidence: pattern.success_rate,
                            evidence: pattern.trajectory_ids.iter().take(3).cloned().collect(),
                            suggested_action: Some(
                                "Consider creating a skill for this recurring pattern".to_string(),
                            ),
                            created_at: chrono::Utc::now().timestamp_millis(),
                        });
                    }
                }
            }
        }
    });
}

fn start_rl_reward_computation(state: &AppState, app_dir: std::path::PathBuf) {
    let trajectory_storage = state.trajectory_storage.clone();
    let rl_engine = state.rl_engine.clone();
    let insight_system = state.insight_system.clone();
    let process_reward_model = state.process_reward_model.clone();
    let _db = state.harness.db().clone();
    let master_key = state.harness.master_key_owned();
    tauri::async_runtime::spawn(async move {
        if let Some(bridge) =
            axagent_runtime::llm_bridge::build_llm_bridge_from_db(&master_key).await
        {
            {
                let mut rl = rl_engine.write().await;
                rl.set_llm_judge(Box::new(bridge.clone()));
            }
            tracing::info!("[rl] LLM judge injected into RLEngine");

            {
                let mut prm = process_reward_model.lock().await;
                prm.set_provider(Box::new(bridge));
            }
            tracing::info!("[rl] LLM PRM provider injected into ProcessRewardModel");
        }

        let interval = std::time::Duration::from_secs(20 * 60);
        let mut reward_normalizer = axagent_trajectory::RewardNormalizer::new();
        loop {
            tokio::time::sleep(interval).await;
            let mut trajectories: Vec<axagent_trajectory::Trajectory> =
                match trajectory_storage.get_trajectories(Some(15)).await {
                    Ok(t) => t,
                    Err(e) => {
                        tracing::warn!("[rl] Failed to fetch trajectories: {}", e);
                        continue;
                    },
                };
            // P0 修复（2026-09-08）：只评「未评分且已结束」的轨迹，并限制单轮上限。
            //
            // 此前每轮无条件对最近 15 条轨迹全量重评分（每步每工具调用一次 LLM judge
            // + PRM 也是 LLM 调用），同一批轨迹每 20 分钟被重复判一遍 —— 纯浪费。
            // judge 从 64 token 修复为 512 后，每条 judge 需生成完整思维链（数秒），
            // 整个 sweep 长时间持续抢占 LLM provider，与前台工作流（股票分析等）
            // 争抢配额，导致工作流阶段更新极慢、后台请求日志刷屏。
            //
            // 两条过滤规则：
            // 1. rewards 非空 = 已评过分，跳过（storage 事务化持久化 rewards，回读可信）
            // 2. created_at 距今不足 15 分钟 = 轨迹可能仍属于进行中的工作流执行，
            //    此时评分会与该工作流抢 LLM 配额，等下一轮再评
            // 3. 单轮最多评 5 条：兜底控制最坏情况下的 LLM 用量
            let now = chrono::Utc::now();
            trajectories.retain(|t| {
                t.rewards.is_empty()
                    && now.signed_duration_since(t.created_at) > chrono::Duration::minutes(15)
            });
            trajectories.truncate(5);
            if trajectories.is_empty() {
                continue;
            }
            let rl = rl_engine.read().await;
            let mut total_rewards = 0;
            let mut total_advantages = 0;
            let mut total_prm_rewards = 0;
            for trajectory in &mut trajectories {
                {
                    let mut rewards = rl.compute_rewards(trajectory).await;
                    total_rewards += rewards.len();
                    rl.shape_rewards(&mut rewards);
                    reward_normalizer.normalize(&mut rewards);
                    trajectory.rewards = rewards;

                    {
                        let prm = process_reward_model.lock().await;
                        let prm_result = prm.compute_trajectory_rewards(trajectory).await;
                        if !prm_result.step_rewards.is_empty() {
                            total_prm_rewards += prm_result.step_rewards.len();
                            let combined_value =
                                trajectory.value_score * 0.5 + prm_result.weighted_reward * 0.5;
                            trajectory.value_score = combined_value;
                            tracing::debug!(
                                "[rl] PRM for trajectory {}: aggregate={:.3}, outcome={:.3}, weighted={:.3}",
                                &trajectory.id[..trajectory.id.len().min(8)],
                                prm_result.aggregate_reward,
                                prm_result.outcome_reward,
                                prm_result.weighted_reward
                            );
                        }
                    }

                    let values = rl.estimate_value_function(trajectory);
                    if !values.is_empty() {
                        let advantages = rl.compute_advantages(&trajectory.rewards, &values);
                        total_advantages += advantages.len();
                        let avg_advantage: f64 = if !advantages.is_empty() {
                            advantages.iter().sum::<f64>() / advantages.len() as f64
                        } else {
                            0.0
                        };
                        if avg_advantage > 0.3 {
                            let gradients = rl.compute_policy_gradient(trajectory, &advantages);
                            tracing::debug!(
                                "[rl] High-advantage trajectory {}: avg_adv={:.3}, gradients={:?}",
                                &trajectory.id[..trajectory.id.len().min(8)],
                                avg_advantage,
                                gradients
                            );
                            // M7-C: 桥接 compute_policy_gradient → RLOptimizer 权重更新
                            if !gradients.is_empty() {
                                let mut opt =
                                    crate::commands::_shared_state::SHARED_OPTIMIZER.write().await;
                                opt.apply_gradients(&gradients);
                            }
                        }
                    }
                    let total_reward: f64 = trajectory.rewards.iter().map(|r| r.value).sum();
                    trajectory.value_score = (trajectory.value_score + total_reward) / 2.0;
                    if let Err(e) = trajectory_storage.save_trajectory(trajectory).await {
                        tracing::warn!("[rl] Failed to update trajectory: {}", e);
                    }
                }
            }
            drop(rl);
            if total_rewards > 0 {
                tracing::info!(
                    "[rl] Computed {} rewards, {} advantages, {} PRM step-evals across {} trajectories",
                    total_rewards,
                    total_advantages,
                    total_prm_rewards,
                    trajectories.len()
                );
                let reward_trajectories: Vec<_> =
                    trajectories.iter().filter(|t| !t.rewards.is_empty()).collect();
                if reward_trajectories.len() >= 3 {
                    let avg_reward: f64 = reward_trajectories
                        .iter()
                        .map(|t| t.rewards.iter().map(|r| r.value).sum::<f64>())
                        .sum::<f64>()
                        / reward_trajectories.len() as f64;
                    let high_reward_count = reward_trajectories
                        .iter()
                        .filter(|t| t.rewards.iter().map(|r| r.value).sum::<f64>() > avg_reward)
                        .count();
                    let mut is = insight_system.write().await;
                    is.add_insight(axagent_trajectory::LearningInsight {
                        id: format!("rl_{}", chrono::Utc::now().timestamp_millis()),
                        category: if avg_reward > 0.0 { axagent_trajectory::InsightCategory::Pattern } else { axagent_trajectory::InsightCategory::Warning },
                        title: format!("RL reward analysis: avg={:.2}", avg_reward),
                        description: format!("{} trajectories analyzed, {} above average reward. Average reward: {:.3}",
                            reward_trajectories.len(), high_reward_count, avg_reward),
                        confidence: (avg_reward.abs() * 2.0).min(0.9),
                        evidence: vec![],
                        suggested_action: if avg_reward < 0.0 {
                            Some("Recent interactions have negative reward signals. Consider adjusting tool usage patterns.".to_string())
                        } else { None },
                        created_at: chrono::Utc::now().timestamp_millis(),
                    });
                }
            }
            // M7-E: 每轮遍历结束保存 RLOptimizer 状态
            let save_dir = app_dir.clone();
            let _ = tokio::task::spawn_blocking(move || {
                crate::commands::_shared_state::save_rl_optimizer(&save_dir);
            })
            .await;
        }
    });
}

fn start_batch_processing(state: &AppState) {
    let trajectory_storage = state.trajectory_storage.clone();
    let batch_processor = state.batch_processor.clone();
    let insight_system = state.insight_system.clone();
    let token = state.shutdown_token.clone();
    state.task_manager.spawn("batch_processing", async move {
        let interval = std::time::Duration::from_secs(60 * 60);
        loop {
            tokio::select! {
                _ = token.cancelled() => {
                    tracing::info!("[batch_processing] 收到关闭信号");
                    break;
                }
                _ = tokio::time::sleep(interval) => {
            let bp = &*batch_processor;
            let trajectories: Vec<axagent_trajectory::Trajectory> =
                match trajectory_storage.get_trajectories(Some(50)).await {
                    Ok(t) => t,
                    Err(_) => continue,
                };
            if trajectories.len() < 5 {
                continue;
            }
            let quality_filtered = bp.filter_by_quality(&trajectories, 0.3);
            if quality_filtered.is_empty() {
                continue;
            }
            let analysis = bp.analyze_batch(&quality_filtered);
            let mut is = insight_system.write().await;
            is.add_insight(axagent_trajectory::LearningInsight {
                id: format!("batch_{}", chrono::Utc::now().timestamp_millis()),
                category: axagent_trajectory::InsightCategory::Improvement,
                title: format!("Batch analysis: {} trajectories, {:.0}% success",
                    analysis.total,
                    if analysis.total > 0 { analysis.outcome_counts.values().sum::<usize>() as f64 / analysis.total as f64 * 100.0 } else { 0.0 }),
                description: format!("Quality: avg={:.2}, value={:.2}. Patterns: {}.",
                    analysis.avg_quality, analysis.avg_value, analysis.top_patterns.len().min(5)),
                confidence: (analysis.avg_quality * 1.5).min(0.9),
                evidence: vec![],
                suggested_action: if analysis.avg_quality < 0.4 {
                    Some("Batch quality is low. Consider reviewing recent interactions for improvement opportunities.".to_string())
                } else { None },
                created_at: chrono::Utc::now().timestamp_millis(),
            });
                }
            }
        }
    });
}

fn start_user_profile_persistence(state: &AppState) {
    let user_profile = state.user_profile.clone();
    tauri::async_runtime::spawn(async move {
        let interval = std::time::Duration::from_secs(10 * 60);
        loop {
            tokio::time::sleep(interval).await;
            let profile = user_profile.read().await;
            let md_content = profile.to_user_md();
            drop(profile);
            if let Some(home) = {
                #[cfg(mobile)]
                {
                    dirs::data_dir().or_else(dirs::home_dir)
                }
                #[cfg(not(mobile))]
                {
                    dirs::home_dir()
                }
            } {
                let user_md_path = home.join(".axagent").join("USER.md");
                if let Some(parent) = user_md_path.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                if let Err(e) = std::fs::write(&user_md_path, &md_content) {
                    tracing::warn!("[user-profile] Failed to write USER.md: {}", e);
                }
            }
        }
    });
}

fn start_skill_evolution(state: &AppState) {
    let trajectory_storage = state.trajectory_storage.clone();
    let skill_evolution_engine = state.skill_evolution_engine.clone();
    let insight_system = state.insight_system.clone();
    let constitution = state.constitution.clone();
    let intrinsic_motivation = state.intrinsic_motivation.clone();
    let coevolution_env = state.coevolution_env.clone();
    let _db = state.harness.db().clone();
    let master_key = state.harness.master_key_owned();
    tauri::async_runtime::spawn(async move {
        if let Some(bridge) =
            axagent_runtime::llm_bridge::build_llm_bridge_from_db(&master_key).await
        {
            let engine = skill_evolution_engine.lock().await;
            engine.set_llm_provider(std::sync::Arc::new(bridge)).await;
            drop(engine);
            tracing::info!("[evolution] LLM provider injected into SkillEvolutionEngine");
        }

        let interval = std::time::Duration::from_secs(45 * 60);
        // T3.2: 弱技能扫描由 if-else 启发式改为贝叶斯后验决策
        // （EvolutionDecider::from_skill 从累计统计构建 Beta 后验，
        //   融合连续失败加权 + 95% 置信下界小样本保护）。
        let evolve_threshold = 0.4;
        let stable_threshold = 0.7;
        let min_evidence = 3.0;
        loop {
            tokio::time::sleep(interval).await;
            let skills: Vec<axagent_trajectory::Skill> = match trajectory_storage.get_skills().await
            {
                Ok(s) => s,
                Err(e) => {
                    tracing::warn!("[evolution] Failed to fetch skills: {}", e);
                    continue;
                },
            };
            let weak_skills: Vec<_> = skills
                .into_iter()
                .filter(|s| {
                    let decider = axagent_trajectory::EvolutionDecider::from_skill(s)
                        .with_thresholds(evolve_threshold, stable_threshold, min_evidence);
                    matches!(decider.decide(), axagent_trajectory::EvolutionDecision::Evolve)
                })
                .collect();
            if weak_skills.is_empty() {
                continue;
            }
            tracing::info!(
                "[evolution] Found {} skills to evolve (bayesian P(success)<{:.0}%)",
                weak_skills.len(),
                evolve_threshold * 100.0
            );
            let test_trajectories: Vec<axagent_trajectory::Trajectory> =
                match trajectory_storage.get_trajectories(Some(30)).await {
                    Ok(t) => t,
                    Err(_) => continue,
                };
            let test_refs: Vec<&axagent_trajectory::Trajectory> =
                test_trajectories.iter().collect();
            for skill in weak_skills.iter().take(2) {
                let mut engine: tokio::sync::MutexGuard<
                    '_,
                    axagent_trajectory::SkillEvolutionEngine,
                > = skill_evolution_engine.lock().await;
                let result = engine.run(skill, &test_refs).await;
                if let Some(modification) = result {
                    if let Err(violations) = constitution.validate_evolution(&modification) {
                        let has_fatal = violations
                            .iter()
                            .any(|v| v.severity == axagent_trajectory::ViolationSeverity::Fatal);
                        let has_critical = violations
                            .iter()
                            .any(|v| v.severity == axagent_trajectory::ViolationSeverity::Critical);
                        if has_fatal || has_critical {
                            tracing::warn!(
                                "[evolution] Constitution blocked skill '{}' evolution: {} violations (fatal={}, critical={})",
                                skill.name,
                                violations.len(),
                                has_fatal,
                                has_critical
                            );
                            continue;
                        }
                        tracing::info!(
                            "[evolution] Constitution warnings for skill '{}' evolution: {:?}",
                            skill.name,
                            violations.iter().map(|v| &v.description).collect::<Vec<_>>()
                        );
                    }
                    if modification.validation_result.as_ref().is_some_and(|v| v.success) {
                        tracing::info!(
                            "[evolution] Skill '{}' evolved: {} (confidence={:.3})",
                            skill.name,
                            modification.reason,
                            modification.confidence
                        );
                        let mut updated_skill = skill.clone();
                        updated_skill.content = modification.new_content.clone();
                        updated_skill.quality_score = modification.confidence;
                        updated_skill.version = format!(
                            "{}.e{}",
                            updated_skill
                                .version
                                .trim_end_matches(|c: char| c == '.' || c.is_ascii_digit()),
                            chrono::Utc::now().timestamp_millis() % 10000
                        );
                        if let Err(e) = trajectory_storage.save_skill(&updated_skill).await {
                            tracing::warn!("[evolution] Failed to save evolved skill: {}", e);
                        }

                        {
                            let mut im = intrinsic_motivation.lock().await;
                            for traj in &test_trajectories {
                                let _intrinsic_reward = im.compute_intrinsic_reward(traj);
                            }
                        }

                        {
                            let mut env = coevolution_env.lock().await;
                            env.update_performance(modification.confidence);
                        }

                        let mut is = insight_system.write().await;
                        is.add_insight(axagent_trajectory::LearningInsight {
                            id: format!("evo_{}", chrono::Utc::now().timestamp_millis()),
                            category: axagent_trajectory::InsightCategory::Improvement,
                            title: format!("Skill '{}' auto-evolved", skill.name),
                            description: modification.reason.clone(),
                            confidence: modification.confidence,
                            evidence: vec![],
                            suggested_action: Some(format!(
                                "Review evolved skill '{}' for correctness",
                                skill.name
                            )),
                            created_at: chrono::Utc::now().timestamp_millis(),
                        });
                    } else {
                        tracing::info!(
                            "[evolution] Skill '{}' evolution did not improve fitness",
                            skill.name
                        );
                    }
                }
            }
        }
    });
}

fn start_skill_watcher(app: &tauri::AppHandle, state: &AppState) {
    let home = {
        #[cfg(mobile)]
        {
            dirs::data_dir().or_else(dirs::home_dir).unwrap_or_default()
        }
        #[cfg(not(mobile))]
        {
            dirs::home_dir().unwrap_or_default()
        }
    };
    let skill_dirs: Vec<std::path::PathBuf> = vec![
        home.join(".axagent").join("skills"),
        home.join(".claude").join("skills"),
        home.join(".trae").join("skills"),
        home.join(".codebuddy").join("skills"),
        home.join(".workbuddy").join("skills"),
        home.join(".agents").join("skills"),
    ];

    let app_handle = app.clone();
    let shutdown = Arc::new(AtomicBool::new(false));
    let _ = state.skill_watcher_shutdown.set(shutdown.clone());
    std::thread::spawn(move || {
        let (tx, rx) = std::sync::mpsc::channel();

        let mut watcher =
            match notify::recommended_watcher(move |res: Result<Event, notify::Error>| {
                if let Ok(event) = res {
                    let _ = tx.send(event);
                }
            }) {
                Ok(w) => w,
                Err(e) => {
                    tracing::warn!("Failed to create skill watcher: {}", e);
                    return;
                },
            };

        for dir in &skill_dirs {
            if dir.exists() {
                if let Err(e) = watcher.watch(dir, RecursiveMode::NonRecursive) {
                    tracing::warn!("Failed to watch skill dir {:?}: {}", dir, e);
                }
            }
        }

        tracing::info!("Skill file watcher started");

        let mut pending: std::collections::HashMap<String, std::time::Instant> =
            std::collections::HashMap::new();
        let debounce = std::time::Duration::from_secs(2);

        loop {
            if shutdown.load(Ordering::Relaxed) {
                tracing::info!("Skill file watcher 收到关闭信号");
                return;
            }
            match rx.recv_timeout(std::time::Duration::from_secs(1)) {
                Ok(event) => {
                    if !event.kind.is_modify() {
                        continue;
                    }
                    for path in &event.paths {
                        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
                        let is_skill_file = matches!(
                            name,
                            "SKILL.md" | "manifest.json" | "skill-manifest.json" | "frontend.json"
                        );
                        if !is_skill_file {
                            continue;
                        }

                        if let Some(parent) = path.parent() {
                            if let Some(skill_name) = parent.file_name().and_then(|n| n.to_str()) {
                                pending
                                    .entry(skill_name.to_string())
                                    .or_insert(std::time::Instant::now());
                            }
                        }
                    }
                },
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                    // 检查是否有到期的事件需要发送
                    let now = std::time::Instant::now();
                    let mut ready: Vec<String> = vec![];
                    pending.retain(|name, ts| {
                        if now.duration_since(*ts) >= debounce {
                            ready.push(name.clone());
                            false
                        } else {
                            true
                        }
                    });

                    if ready.is_empty() {
                        continue;
                    }

                    let app = app_handle.clone();
                    tauri::async_runtime::spawn(async move {
                        for name in ready {
                            let _ = app.emit("skill:file-changed", name);
                        }
                    });
                },
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                    tracing::info!("Skill file watcher stopped");
                    return;
                },
            }
        }
    });
}

/// Dream 巩固定时任务
///
/// 每 30 分钟检查一次：
/// 1. 通过 trajectory 数量增量检测新会话，调用 record_new_session 累加计数
/// 2. 检查 should_consolidate 门控（启用/未运行/间隔/会话数/锁）
/// 3. 满足门控则执行 consolidate（经验回放→知识蒸馏→对比学习→建议生成）
fn start_dream_consolidation(state: &AppState) {
    let consolidator = state.dream_consolidator.clone();
    let trajectory_storage = state.trajectory_storage.clone();
    let last_trajectory_count = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(u64::MAX));
    let last_count = last_trajectory_count.clone();
    tauri::async_runtime::spawn(async move {
        // 初始化 trajectory 基线计数
        if let Ok(trajs) = trajectory_storage.get_trajectories(Some(10000)).await {
            last_count.store(trajs.len() as u64, std::sync::atomic::Ordering::Relaxed);
        }

        let interval = std::time::Duration::from_secs(30 * 60);
        loop {
            tokio::time::sleep(interval).await;

            // 检测新会话：trajectory 数量增量即为新会话数
            let current_count = match trajectory_storage.get_trajectories(Some(10000)).await {
                Ok(trajs) => trajs.len() as u64,
                Err(e) => {
                    tracing::warn!("[dream] 获取 trajectory 失败: {}", e);
                    continue;
                },
            };
            let prev_count = last_count.swap(current_count, std::sync::atomic::Ordering::Relaxed);
            // 首次循环 prev_count == u64::MAX（基线），跳过计数
            if prev_count != u64::MAX && current_count > prev_count {
                let new_sessions = (current_count - prev_count) as usize;
                for _ in 0..new_sessions {
                    consolidator.record_new_session().await;
                }
                tracing::info!("[dream] 记录 {} 个新会话", new_sessions);
            }

            // 检查门控条件
            if !consolidator.should_consolidate().await {
                continue;
            }

            tracing::info!("[dream] 开始执行巩固...");
            let result = consolidator
                .consolidate(
                    Some(&|n| tracing::info!("[dream] 提取 {} 条记忆", n)),
                    Some(&|n| tracing::info!("[dream] 发现 {} 个模式", n)),
                    Some(&|n| tracing::info!("[dream] 生成 {} 个建议", n)),
                )
                .await;

            if result.executed {
                tracing::info!(
                    "[dream] 巩固完成: {} 条记忆, {} 个模式, {} 个建议, 耗时 {} 秒",
                    result.memories_extracted,
                    result.patterns_discovered,
                    result.suggestions_generated,
                    result.duration_secs
                );

                // Dream↔Evolution 联动：发现新模式时提示可能需要触发技能进化
                // 注意：不直接调用 SkillEvolutionEngine（避免循环依赖），
                // 仅记录日志，由独立的 start_skill_evolution 定时任务在下一轮自动检测弱技能并进化
                if result.patterns_discovered > 0 {
                    tracing::info!(
                        "[dream] 发现 {} 个新模式，下一轮 skill evolution 将评估是否需要进化相关技能",
                        result.patterns_discovered
                    );
                }
            } else {
                tracing::warn!(
                    "[dream] 巩固未执行: {}",
                    result.error.as_deref().unwrap_or("未知原因")
                );
            }
        }
    });
}

fn start_memory_decay_tick(state: &AppState) {
    let memory_service = state.memory_service.clone();
    let harness_state = state.harness.clone();
    tauri::async_runtime::spawn(async move {
        let interval = std::time::Duration::from_secs(3600);
        loop {
            tokio::time::sleep(interval).await;
            // 1. trajectory working memory 衰减（sentinel namespace）
            let ms = memory_service.read().await;
            let evicted = ms.apply_decay_tick().await;
            drop(ms);
            if evicted > 0 {
                tracing::info!("[memory_decay] trajectory evicted {} entries", evicted);
            }
            // 2. 用户 namespace 全表衰减（三层记忆系统）
            match axagent_dao::repo::memory::apply_decay_tick(harness_state.db()).await {
                Ok((expired, low_score, capacity)) => {
                    if expired + low_score + capacity > 0 {
                        tracing::info!(
                            "[memory_decay] user ns: expired={}, low_score={}, capacity={}",
                            expired,
                            low_score,
                            capacity
                        );
                    }
                },
                Err(e) => {
                    tracing::warn!("[memory_decay] user ns tick failed: {}", e);
                },
            }
        }
    });
}

/// DreamTask 全量清理定时任务
///
/// 每 60 分钟执行一次 FullCleanup，涵盖：
/// - 轨迹整合（consolidator，与 start_dream_consolidation 共享实例，内部有门控）
/// - 记忆压缩（auto_memory_extractor + FTS5 optimize）
/// - 技能更新（skill_evolution_engine，与 start_skill_evolution 共享实例）
/// - 僵尸 SubAgent 清理（sub_agent_registry）
/// - FTS5 索引优化（optimize + vacuum）
///
/// 与 start_dream_consolidation 的关系：
/// - start_dream_consolidation 30 分钟一次，仅做轨迹巩固（轻量）
/// - start_dream_task_executor 60 分钟一次，做全量清理（重量）
///
/// 两者共享 DreamConsolidator 实例，consolidator 内部 should_consolidate 门控
/// 会避免重复执行实际的巩固操作。
fn start_dream_task_executor(state: &AppState) {
    // 组装 DreamTaskContext（所有依赖均为 AppState 中的 Arc 克隆）
    let ctx = axagent_runtime::tasks::dream_task::DreamTaskContext {
        consolidator: Some(state.dream_consolidator.clone()),
        trajectory_storage: Some(state.trajectory_storage.clone()),
        skill_evolution_engine: Some(state.skill_evolution_engine.clone()),
        auto_memory_extractor: Some(state.auto_memory_extractor.clone()),
        sub_agent_registry: Some(state.sub_agent_registry.clone()),
    };

    tauri::async_runtime::spawn(async move {
        // 启动后延迟 10 分钟首次执行，避免与启动期间的其它密集任务冲突
        let initial_delay = std::time::Duration::from_secs(10 * 60);
        tokio::time::sleep(initial_delay).await;

        let interval = std::time::Duration::from_secs(60 * 60);
        loop {
            let task = axagent_runtime::tasks::dream_task::DreamTask::on_session_end();
            tracing::info!("[dream_task_executor] 触发全量清理 (scope={:?})", task.scope);
            let result =
                axagent_runtime::tasks::dream_task::DreamTaskExecutor::execute(&task, &ctx).await;
            if !result.errors.is_empty() {
                tracing::warn!(
                    "[dream_task_executor] 本次执行有 {} 个子任务跳过/失败: {:?}",
                    result.errors.len(),
                    result.errors
                );
            }
            tokio::time::sleep(interval).await;
        }
    });
}

/// CoevolutionTask 协同进化定时任务
///
/// 每 30 分钟执行一次，根据近期轨迹成功率驱动难度调整 + 生成针对薄弱类别的新任务。
/// 与 `start_skill_evolution` 共享同一 `CoevolutionEnvironment` 实例：
/// - `start_skill_evolution` 在技能进化成功后被动更新性能
/// - 本任务主动周期性地用整体成功率驱动协同进化
///
/// 依赖：`coevolution_env` / `trajectory_storage` / `insight_system`
/// 任一缺失会跳过对应子功能并在 `result.errors` 中记录。
fn start_coevolution_task_executor(state: &AppState) {
    let ctx = axagent_runtime::tasks::coevolution_task::CoevolutionTaskContext {
        coevolution_env: Some(state.coevolution_env.clone()),
        trajectory_storage: Some(state.trajectory_storage.clone()),
        insight_system: Some(state.insight_system.clone()),
    };

    tauri::async_runtime::spawn(async move {
        // 启动后延迟 10 分钟首次执行，避免与启动期间的其它密集任务冲突
        let initial_delay = std::time::Duration::from_secs(10 * 60);
        tokio::time::sleep(initial_delay).await;

        let interval = std::time::Duration::from_secs(30 * 60);
        loop {
            tracing::info!("[coevolution_task_executor] 触发协同进化周期任务");
            let result =
                axagent_runtime::tasks::coevolution_task::CoevolutionTaskExecutor::execute(&ctx)
                    .await;
            if !result.errors.is_empty() {
                tracing::warn!(
                    "[coevolution_task_executor] 本次执行有 {} 个子任务跳过/失败: {:?}",
                    result.errors.len(),
                    result.errors
                );
            }
            tokio::time::sleep(interval).await;
        }
    });
}

/// PatternAnalyzerTask 跨会话模式分析定时任务
///
/// 每 2 小时执行一次，从近期轨迹提取代码风格 / 工具偏好 / 时间分布模式，
/// 把关键发现作为 `LearningInsight` 写入 `insight_system`。
///
/// 与 `start_pattern_learning` 互补：
/// - `start_pattern_learning` 学习任务级 `TrajectoryPattern`（含 success_rate）
/// - 本任务提取更细粒度的用户行为模式，用于丰富用户画像与行为洞察
///
/// 依赖：`trajectory_storage` / `insight_system`
fn start_pattern_analyzer_task_executor(state: &AppState) {
    let ctx = axagent_runtime::tasks::pattern_task::PatternAnalyzerTaskContext {
        trajectory_storage: Some(state.trajectory_storage.clone()),
        insight_system: Some(state.insight_system.clone()),
    };

    tauri::async_runtime::spawn(async move {
        // 启动后延迟 15 分钟首次执行，比 coevolution 稍晚以错峰
        let initial_delay = std::time::Duration::from_secs(15 * 60);
        tokio::time::sleep(initial_delay).await;

        let interval = std::time::Duration::from_secs(2 * 60 * 60);
        loop {
            tracing::info!("[pattern_analyzer_task_executor] 触发模式分析周期任务");
            let result =
                axagent_runtime::tasks::pattern_task::PatternAnalyzerTaskExecutor::execute(&ctx)
                    .await;
            if !result.errors.is_empty() {
                tracing::warn!(
                    "[pattern_analyzer_task_executor] 本次执行有 {} 个子任务跳过/失败: {:?}",
                    result.errors.len(),
                    result.errors
                );
            }
            tokio::time::sleep(interval).await;
        }
    });
}

/// InsightGeneratorTask 学习洞察生成定时任务
///
/// 每 6 小时执行一次，从近期轨迹分析整体趋势（成功率 / 质量分布），
/// 生成趋势洞察 + 日报。与 `start_insight_generation` 互补：
/// - `start_insight_generation` 从实时反馈生成洞察（10 分钟一次，关注实时）
/// - 本任务从轨迹存储的整体趋势生成洞察 + 日报（周期更长，关注长期）
///
/// 依赖：`trajectory_storage` / `insight_system`
fn start_insight_generator_task_executor(state: &AppState) {
    let ctx = axagent_runtime::tasks::insight_task::InsightGeneratorTaskContext {
        trajectory_storage: Some(state.trajectory_storage.clone()),
        insight_system: Some(state.insight_system.clone()),
    };

    tauri::async_runtime::spawn(async move {
        // 启动后延迟 20 分钟首次执行，与其它任务错峰
        let initial_delay = std::time::Duration::from_secs(20 * 60);
        tokio::time::sleep(initial_delay).await;

        let interval = std::time::Duration::from_secs(6 * 60 * 60);
        loop {
            tracing::info!("[insight_generator_task_executor] 触发洞察生成周期任务");
            let result =
                axagent_runtime::tasks::insight_task::InsightGeneratorTaskExecutor::execute(&ctx)
                    .await;
            if !result.errors.is_empty() {
                tracing::warn!(
                    "[insight_generator_task_executor] 本次执行有 {} 个子任务跳过/失败: {:?}",
                    result.errors.len(),
                    result.errors
                );
            }
            tokio::time::sleep(interval).await;
        }
    });
}

fn start_auto_tool_observation(state: &AppState) {
    let trajectory_storage = state.trajectory_storage.clone();
    let auto_tool_creator = state.auto_tool_creator.clone();
    let _db = state.harness.db().clone();
    let master_key = state.harness.master_key_owned();
    tauri::async_runtime::spawn(async move {
        if let Some(bridge) =
            axagent_runtime::llm_bridge::build_llm_bridge_from_db(&master_key).await
        {
            let mut atc = auto_tool_creator.lock().await;
            atc.set_llm_provider(Box::new(bridge));
            drop(atc);
            tracing::info!("[auto_tool] LLM provider injected into AutoToolCreator");
        }

        let interval = std::time::Duration::from_secs(60 * 60);
        loop {
            tokio::time::sleep(interval).await;
            let trajectories: Vec<axagent_trajectory::Trajectory> =
                match trajectory_storage.get_trajectories(Some(30)).await {
                    Ok(t) => t,
                    Err(e) => {
                        tracing::warn!("[auto_tool] Failed to fetch trajectories: {}", e);
                        continue;
                    },
                };

            let mut atc = auto_tool_creator.lock().await;
            for trajectory in &trajectories {
                atc.observe_trajectory(trajectory);
            }

            let frequent = atc.get_frequent_patterns(3);
            if !frequent.is_empty() {
                tracing::info!(
                    "[auto_tool] Observed {} frequent tool patterns (top: {:?})",
                    frequent.len(),
                    &frequent[..frequent.len().min(5)]
                );

                for (pattern, count) in frequent.iter().take(2) {
                    if atc.get_tool(&axagent_trajectory::slugify(pattern)).is_none() {
                        match atc
                            .create_tool_from_pattern(
                                pattern,
                                &format!("Auto-observed pattern ({} occurrences)", count),
                                vec![],
                            )
                            .await
                        {
                            Ok(tool) => {
                                tracing::info!(
                                    "[auto_tool] Created tool '{}' from pattern '{}' (freq={})",
                                    tool.name,
                                    pattern,
                                    count
                                );
                            },
                            Err(e) => {
                                tracing::debug!(
                                    "[auto_tool] Could not create tool from '{}': {}",
                                    pattern,
                                    e
                                );
                            },
                        }
                    }
                }
            }
        }
    });
}

fn start_text_grad_analysis(state: &AppState) {
    let trajectory_storage = state.trajectory_storage.clone();
    let text_grad_engine = state.text_grad_engine.clone();
    let _db = state.harness.db().clone();
    let master_key = state.harness.master_key_owned();
    tauri::async_runtime::spawn(async move {
        if let Some(bridge) =
            axagent_runtime::llm_bridge::build_llm_bridge_from_db(&master_key).await
        {
            let mut engine = text_grad_engine.lock().await;
            engine.set_provider(bridge);
            drop(engine);
            tracing::info!("[text_grad] LLM provider injected into TextGradEngine");
        }

        let interval = std::time::Duration::from_secs(2 * 60 * 60);
        loop {
            tokio::time::sleep(interval).await;
            let trajectories: Vec<axagent_trajectory::Trajectory> =
                match trajectory_storage.get_trajectories(Some(10)).await {
                    Ok(t) => t,
                    Err(e) => {
                        tracing::warn!("[text_grad] Failed to fetch trajectories: {}", e);
                        continue;
                    },
                };

            let mut engine = text_grad_engine.lock().await;
            for trajectory in &trajectories {
                if trajectory.steps.len() < 3 {
                    continue;
                }
                let session_id = &trajectory.session_id;
                let topic = &trajectory.topic;

                for (i, step) in trajectory.steps.iter().enumerate() {
                    let content_summary: String = step.content.chars().take(200).collect();
                    let node_id = format!("{}_{}", &session_id[..session_id.len().min(8)], i);
                    engine.add_node(node_id.clone(), content_summary, Some(format!("step_{}", i)));
                    if i > 0 {
                        let prev_id =
                            format!("{}_{}", &session_id[..session_id.len().min(8)], i - 1);
                        engine.add_edge(prev_id, node_id, 1.0);
                    }
                }

                if !trajectory.steps.is_empty() {
                    if let Some(last_step) = trajectory.steps.last() {
                        let feedback = match trajectory.outcome {
                            axagent_trajectory::TrajectoryOutcome::Success => {
                                format!("Task succeeded: {}", topic)
                            },
                            axagent_trajectory::TrajectoryOutcome::Failure => {
                                format!(
                                    "Task failed: {} - last step: {}",
                                    topic,
                                    last_step.content.chars().take(100).collect::<String>()
                                )
                            },
                            axagent_trajectory::TrajectoryOutcome::Partial => {
                                format!("Task partially completed: {}", topic)
                            },
                            axagent_trajectory::TrajectoryOutcome::Abandoned => {
                                format!("Task abandoned: {}", topic)
                            },
                        };
                        let last_id = format!(
                            "{}_{}",
                            &session_id[..session_id.len().min(8)],
                            trajectory.steps.len() - 1
                        );
                        let _ = engine.forward();
                        let _ = engine.backward(&last_id, &feedback).await;
                    }
                }
            }

            let stats = engine.stats();
            tracing::info!(
                "[text_grad] Graph stats: {} nodes, {} edges, {} gradients computed",
                stats.node_count,
                stats.edge_count,
                stats.gradient_count
            );
        }
    });
}

async fn start_cron_scheduler(app: &tauri::AppHandle, state: &AppState) {
    use axagent_runtime::cron::{CronExecutor, CronScheduler};
    use std::sync::Arc;

    // 荐股定时任务的推送通道是 OS 桌面通知，需要 AppHandle。
    // 此前该分支整个不存在，所以没传 app；恢复接线后必须带上。
    let app_handle = app.clone();

    let store = state.cron_job_store.clone();

    // 注入共享存储到 tools crate，使 CronCreateTool 等可用
    axagent_tools::tools::cron::init_cron_store(store.clone());

    // 设置工具解析器（从全局 registry 按需自动注册工作流中引用的工具）
    {
        let registry = state.local_tool_registry.clone();
        let work_engine = state.work_engine.clone();
        let resolver: axagent_runtime::work_engine::ToolResolver = std::sync::Arc::new(
            move |tool_name: String| {
                let registry = registry.clone();
                let work_engine = work_engine.clone();
                tracing::info!("[ToolResolver] 被调用: tool_name={}", tool_name);
                Box::pin(async move {
                    let reg = registry.lock().await;
                    let known = reg.list_all_tool_names().contains(&tool_name)
                        || reg.mcp.mcp_tools.contains_key(&tool_name);
                    tracing::info!("[ToolResolver] 解析 tool_name={}, known={}", tool_name, known);
                    if known {
                        let registry = registry.clone();
                        let cb: axagent_runtime::work_engine::ToolCallback =
                            std::sync::Arc::new(move |tn: String, args: serde_json::Value| {
                                let registry = registry.clone();
                                Box::pin(async move {
                                    let reg = registry.lock().await;
                                    let input_str = serde_json::to_string(&args)
                                        .unwrap_or_else(|_| "{}".to_string());
                                    match reg.execute(&tn, &input_str).await {
                                        Ok(output) => {
                                            Ok(serde_json::json!({"content": output.content}))
                                        },
                                        Err(e) => Err(format!("Tool execution error: {}", e)),
                                    }
                                })
                            });
                        Some(cb)
                    } else if let Some(template_id) = tool_name.strip_prefix("workflow::") {
                        // 工作流注册为工具：workflow::<template_id>
                        let engine = work_engine.clone();
                        let template_id = template_id.to_string();
                        let cb: axagent_runtime::work_engine::ToolCallback =
                            std::sync::Arc::new(move |_tn: String, args: serde_json::Value| {
                                let engine = engine.clone();
                                let tid = template_id.clone();
                                Box::pin(async move {
                                    let mut opts =
                                        axagent_runtime::work_engine::RunOptions::default();
                                    if let Some(input) = args.get("input") {
                                        opts.input = Some(input.clone());
                                    }
                                    match engine.run_workflow(&tid, opts).await {
                                        Ok(wf) => Ok(serde_json::json!({
                                            "content": serde_json::json!({
                                                "status": format!("{:?}", wf.status),
                                                "results": wf.results,
                                            }).to_string()
                                        })),
                                        Err(e) => {
                                            Err(format!("Workflow tool '{}' failed: {:?}", tid, e))
                                        },
                                    }
                                })
                            });
                        Some(cb)
                    } else {
                        tracing::warn!(
                            "[ToolResolver] 工具 '{}' 未匹配任何解析路径 (known=false, not workflow::)",
                            tool_name
                        );
                        None
                    }
                })
            },
        );
        // 复用 Tauri 全局 runtime，避免一次性创建/销毁 runtime 的开销。
        state.work_engine.set_tool_resolver(resolver).await;
    }

    // 设置 RAG 知识源检索回调（供工作流 Agent 节点从知识库/记忆/Wiki 检索上下文）
    {
        let db = state.harness.db().clone();
        let master_key = state.harness.master_key_owned();
        let vector_store = state.vector_store.clone();
        let rag_callback: axagent_rt_workflow::work_engine::RagCallback = std::sync::Arc::new(
            move |kb_ids: Vec<String>,
                  mem_ids: Vec<String>,
                  wiki_ids: Vec<String>,
                  query: String| {
                let db = db.clone();
                let vector_store = vector_store.clone();
                Box::pin(async move {
                    let embed_fn = crate::indexing::ProviderEmbedFn;
                    // 混合检索权重来自用户设置（全局 `ragPipelineConfig.hybrid`）
                    let hybrid = crate::indexing::load_hybrid_config(&db).await;
                    let result = axagent_search::rag::collect_rag_context(
                        &db,
                        &master_key,
                        &vector_store,
                        &kb_ids,
                        &mem_ids,
                        &wiki_ids,
                        &query,
                        5,
                        embed_fn,
                        Some(&hybrid),
                    )
                    .await;
                    Ok(result)
                })
            },
        );
        // 复用 Tauri 全局 runtime，避免一次性创建/销毁 runtime 的开销。
        state.work_engine.set_rag_callback(rag_callback).await;
    }

    // 注入实体图谱提供者（Graph RAG 阶段 4）。
    //
    // 该能力此前**永不执行**：`rag.rs` 调用 `RAGPipeline::new` 时第 4 参恒为 `None`，
    // 而 `rag_pipeline.rs` 的图增强分支在 `None` 时直接跳过（2026-09-15 接线）。
    //
    // 类型化读取：此前这里直接 `Value::get("entityGraph")` 按字符串键摸字段，而
    // `RAGPipelineConfig` 原本**没有** `entity_graph` 字段 ⇒ 那个键只存在于本处代码里，
    // 前端类型 / 设置 UI 均无，等于一个**永远没人能打开的开关**。现已补齐四条链。
    //
    // ⚠ 2026-09-15 二次修：逻辑已抽成 `indexing::sync_entity_graph_provider` 并在此调用。
    // 原因是注入点由 `OnceLock` 改为**可替换槽位**后，同一个函数还要在「保存设置」时
    // 再调一次，才能让开关**无需重启**生效（见 `commands::settings::save_settings`）。
    // 抽成一个幂等函数可避免两处装配逻辑各写一遍而分叉。
    crate::indexing::sync_entity_graph_provider(state.harness.db()).await;

    let work_engine = state.work_engine.clone();
    let cron_store = state.cron_job_store.clone();
    let sync_db = state.harness.db().clone();
    let astock_client = state.astock_client.clone();
    // [2026-09-13] 批量反思/决策校验分支需要这三个依赖（此前它们只在
    // `start_batch_reflection` 的硬编码服务里被捕获，cron 分支无从取得）
    let cron_vector_store = state.vector_store.clone();
    let cron_master_key = state.harness.master_key_owned();
    let cron_trajectory_storage = state.trajectory_storage.clone();
    // G17 delivery：此前 create_cron_delivery_sink 建好了 sink 却从未接进
    // executor，导致所有定时任务的 delivery 配置都是死配置。这里接上后，
    // 任务可通过 CronDeliveryConfig 把执行结果推送到 webhook / 文件 / 通知渠道。
    let delivery_sink: std::sync::Arc<dyn axagent_harness::cron_delivery::CronDeliverySink> =
        create_cron_delivery_sink(state);
    let mut executor = CronExecutor::new();
    executor.set_handler(move |job| {
        // 知识源定时刷新：task_type = knowledge_source_fetch_all
        if job.task_type.as_deref() == Some("knowledge_source_fetch_all") {
            let store = cron_store.clone();
            let db = sync_db.clone();
            let job_id = job.id.clone();
            let job_name = job.name.clone();
            let recurring = job.recurring;
            tokio::task::spawn(async move {
                let started = axagent_runtime_core::cron_job::now_millis();
                let results =
                    crate::commands::knowledge_source::run_knowledge_source_sync(&db).await;
                let errors = results.iter().filter(|r| r.action == "error").count();
                let ok = results.len().saturating_sub(errors);
                let result = axagent_runtime_core::TaskRunResult {
                    success: errors == 0,
                    output: Some(format!(
                        "知识源同步完成: {} 成功, {} 失败, 共 {} 源",
                        ok,
                        errors,
                        results.len()
                    )),
                    error: (errors > 0).then(|| format!("{errors} 个知识源抓取失败")).or(None),
                    duration_ms: (axagent_runtime_core::cron_job::now_millis() - started) as u64,
                    executed_at: started,
                };
                tracing::info!(
                    "[CronScheduler] 知识源刷新任务 '{}' 完成: {:?}",
                    job_name,
                    result.output
                );
                store.record_run(&job_id, result).await;
                if !recurring {
                    let _ = store
                        .set_status(&job_id, axagent_runtime_core::CronJobStatus::Disabled)
                        .await;
                }
            });
            return;
        }
        // 需求订阅扫描：task_type = opc_demand_scan（v133）
        //
        // 按订阅词表挑出到期订阅逐个扫描，命中推送门槛的线索才走 delivery ——
        // 无命中时传 None sink，避免每个 tick 空转轰炸推送渠道。
        if job.task_type.as_deref()
            == Some(crate::commands::opc_demand_subscription::SCAN_JOB_TASK_TYPE)
        {
            let store = cron_store.clone();
            let db = sync_db.clone();
            let sink = delivery_sink.clone();
            let job_id = job.id.clone();
            let job_name = job.name.clone();
            let recurring = job.recurring;
            tokio::task::spawn(async move {
                let started = axagent_runtime_core::cron_job::now_millis();
                let (result, should_deliver) =
                    match crate::commands::opc_demand_subscription::run_scan_for_scheduler(&db)
                        .await
                    {
                        Ok((summary, text)) => {
                            let has_hits = summary.high_value_hits > 0;
                            tracing::info!(
                                "[CronScheduler] 需求订阅扫描 '{}' 完成: {} 个订阅, {} 条高价值命中",
                                job_name,
                                summary.scanned_subscriptions,
                                summary.high_value_hits
                            );
                            (
                                axagent_runtime_core::TaskRunResult {
                                    success: true,
                                    output: Some(text),
                                    error: None,
                                    duration_ms: (axagent_runtime_core::cron_job::now_millis()
                                        - started)
                                        as u64,
                                    executed_at: started,
                                },
                                has_hits,
                            )
                        },
                        Err(e) => {
                            tracing::error!(
                                "[CronScheduler] 需求订阅扫描 '{}' 失败: {e}",
                                job_name
                            );
                            (
                                axagent_runtime_core::TaskRunResult {
                                    success: false,
                                    output: None,
                                    error: Some(e),
                                    duration_ms: (axagent_runtime_core::cron_job::now_millis()
                                        - started)
                                        as u64,
                                    executed_at: started,
                                },
                                true,
                            )
                        },
                    };
                // 无高价值命中 → 不投递（success 但静默）
                let sink_ref = if should_deliver { Some(sink.as_ref()) } else { None };
                store.record_run_with_delivery(&job_id, result, sink_ref).await;
                if !recurring {
                    let _ = store
                        .set_status(&job_id, axagent_runtime_core::CronJobStatus::Disabled)
                        .await;
                }
            });
            return;
        }
        // 荐股定时推送：task_type = stock-recommendation
        // （前端 ScheduledRecommendationTab 创建/管理；配置以 JSON 存 CronJob.prompt）
        //
        // [2026-09-12 接线恢复] 此前这里只有一句「stock-recommendation 分支已移除」，
        // 于是形成一条半拆除的功能链：前端 UI 完整 + 4 个 CRUD 命令可用 +
        // recommender::notify 的扫描/通知逻辑完好（含单测），唯独到点没有执行入口。
        // 用户能创建任务、能看列表、能启停，但永远等不到推送。
        // 同构缺陷已于 2026-09-03 在下方 watchlist-scan 侧修过一次，荐股这侧一直漏修。
        //
        // 执行：解析 RecoCronConfig → 读 stock-analysis 模板变量（vendor 开关与各策略阈值）
        // → run_recommendation_scan（多周期并行 + 过滤 synthetic + 阈值 + top N）
        // → 桌面通知 + delivery sink。
        if job.task_type.as_deref() == Some("stock-recommendation") {
            let store = cron_store.clone();
            let db = sync_db.clone();
            let client = astock_client.clone();
            let app = app_handle.clone();
            let sink = delivery_sink.clone();
            let job_id = job.id.clone();
            let job_name = job.name.clone();
            let recurring = job.recurring;
            let prompt = job.prompt.clone();
            tokio::task::spawn(async move {
                let started = axagent_runtime_core::cron_job::now_millis();
                let elapsed =
                    || (axagent_runtime_core::cron_job::now_millis() - started) as u64;

                // 1. 解析任务配置（periods / min_confidence / top_n）。
                //    解析失败按任务失败记录，不做静默兜底 —— 否则用户设的周期会被
                //    「默认值」悄悄替换，而列表里显示的仍是原文。
                let config = match crate::commands::recommendation_cron::RecoCronConfig::from_json(
                    &prompt,
                ) {
                    Ok(c) => c,
                    Err(e) => {
                        tracing::error!(
                            "[CronScheduler] 荐股任务 '{}' 配置解析失败: {}",
                            job_name,
                            e
                        );
                        let result = axagent_runtime_core::TaskRunResult {
                            success: false,
                            output: None,
                            error: Some(e),
                            duration_ms: elapsed(),
                            executed_at: started,
                        };
                        store.record_run(&job_id, result).await;
                        if !recurring {
                            let _ = store
                                .set_status(
                                    &job_id,
                                    axagent_runtime_core::CronJobStatus::Disabled,
                                )
                                .await;
                        }
                        return;
                    },
                };

                // 2. 读模板变量：vendor 启用集合 + reco_*_enabled + 各策略阈值。
                //    读不到不算失败（走代码内硬编码默认值），但要留痕，
                //    否则「面板上调了阈值却毫无效果」会变成无从下手的现象。
                use sea_orm::EntityTrait;
                let vars =
                    match axagent_entities::workflow_template::Entity::find_by_id("stock-analysis")
                        .one(&db)
                        .await
                    {
                        Ok(Some(t)) => {
                            crate::commands::stock_analysis::extract_template_vars(&t)
                        },
                        Ok(None) => {
                            tracing::warn!(
                                "[CronScheduler] 荐股任务 '{}': 未找到 stock-analysis 模板，\
                                 本次扫描按硬编码默认参数执行",
                                job_name
                            );
                            Vec::new()
                        },
                        Err(e) => {
                            tracing::warn!(
                                "[CronScheduler] 荐股任务 '{}': 读取模板变量失败 ({e})，\
                                 本次扫描按硬编码默认参数执行",
                                job_name
                            );
                            Vec::new()
                        },
                    };

                // 3. 扫描（多周期并行，内部已过滤 synthetic 与低于阈值的 pick）
                let scan = axagent_analysis_engine::recommender::run_recommendation_scan(
                    client,
                    &config.periods,
                    &vars,
                    config.min_confidence,
                    config.top_n,
                )
                .await;
                let picks = scan.picks;

                // 3.5 [2026-09-13] 扫描结果落库到 reco_picks。
                //     此前定时荐股只推桌面通知、**从不写候选池**（落库逻辑只在命令层
                //     `recommend_stocks`，即"前端点刷新才落库"），于是下游
                //     「候选池→逐只分析」（pool-scan）任务永远取不到候选。
                //     与命令层共用 `persist_reco_picks`，保证两条入口写入同一张表、
                //     同一套字段。
                if !picks.is_empty() {
                    let generated_at =
                        chrono::Local::now().format("%Y-%m-%dT%H:%M:%S%.3f").to_string();
                    let strategy_weights_json = vars
                        .iter()
                        .find(|(k, _)| k == "reco_strategy_weights")
                        .and_then(|(_, v)| v.is_object().then(|| v.to_string()));
                    let written = crate::commands::stock_analysis::persist_reco_picks(
                        &db,
                        &picks,
                        scan.seed_pool_snapshot.clone(),
                        strategy_weights_json,
                        &generated_at,
                    )
                    .await;
                    tracing::info!(
                        "[CronScheduler] 荐股任务 '{}' 落库 reco_picks: {written}/{} 行",
                        job_name,
                        picks.len()
                    );
                }

                // 4. 无 pick 时静默（不推空通知），但有 pick 必须推 —— 这是本任务的存在意义
                let (title, body) = axagent_analysis_engine::recommender::build_notification(
                    &picks,
                );
                if !picks.is_empty() {
                    if let Err(e) = crate::commands::desktop::send_desktop_notification(
                        app,
                        title.clone(),
                        body.clone(),
                    )
                    .await
                    {
                        // 通知失败不影响任务成功（结果已落库，可在列表中查看）
                        tracing::warn!(
                            "[CronScheduler] 荐股任务 '{}' 桌面通知发送失败: {e}",
                            job_name
                        );
                    }
                }
                tracing::info!(
                    "[CronScheduler] 荐股任务 '{}' 完成: {} 只（周期 {:?}, 阈值 {}）",
                    job_name,
                    picks.len(),
                    config.periods,
                    config.min_confidence
                );

                // 5. output 必须是 JSON 且含 `pushed` 键 ——
                //    前端 `RecoCronJobResponse.last_picks_count` 读的正是 `output.pushed`，
                //    换键名会让「上次推送 N 只」恒显示为空。
                let output = serde_json::json!({
                    "pushed": picks.len(),
                    "periods": config.periods.iter().map(|p| p.as_str()).collect::<Vec<_>>(),
                    "minConfidence": config.min_confidence,
                    "title": title,
                    "body": body,
                })
                .to_string();
                let result = axagent_runtime_core::TaskRunResult {
                    success: true,
                    output: Some(output),
                    error: None,
                    duration_ms: elapsed(),
                    executed_at: started,
                };
                // 无 pick → 不投递（success 但静默），避免定时轰炸通知渠道
                let sink_ref = if picks.is_empty() { None } else { Some(sink.as_ref()) };
                store.record_run_with_delivery(&job_id, result, sink_ref).await;
                if !recurring {
                    let _ = store
                        .set_status(&job_id, axagent_runtime_core::CronJobStatus::Disabled)
                        .await;
                }
            });
            return;
        }
        // 自选股定时扫描：task_type = watchlist-scan（前端 ScheduledAnalysisTab 创建/管理）。
        // [2026-09-03 接线恢复] 此前 CronExecutor 无此分支，UI 创建的任务到点后不执行任何分析。
        // 执行：遍历 watchlist_items → 逐只 run_single_stock_analysis（写 stock_analyses
        // 并落 stock_reflections pending row，由 6h batch reflection 消费）。
        if job.task_type.as_deref() == Some("watchlist-scan") {
            let store = cron_store.clone();
            let db = sync_db.clone();
            let client = astock_client.clone();
            let engine = work_engine.clone();
            let job_id = job.id.clone();
            let job_name = job.name.clone();
            let recurring = job.recurring;
            tokio::task::spawn(async move {
                let started = axagent_runtime_core::cron_job::now_millis();
                use sea_orm::EntityTrait;
                let items = axagent_entities::watchlist_items::Entity::find()
                    .all(&db)
                    .await
                    .unwrap_or_default();
                if items.is_empty() {
                    tracing::info!("[CronScheduler] 自选股扫描 '{job_name}': 自选股为空，跳过");
                    let result = axagent_runtime_core::TaskRunResult {
                        success: true,
                        output: Some("自选股为空，未执行分析".to_string()),
                        error: None,
                        duration_ms: (axagent_runtime_core::cron_job::now_millis() - started)
                            as u64,
                        executed_at: started,
                    };
                    store.record_run(&job_id, result).await;
                    if !recurring {
                        let _ = store
                            .set_status(&job_id, axagent_runtime_core::CronJobStatus::Disabled)
                            .await;
                    }
                    return;
                }
                let mut ok = 0usize;
                let mut fail = 0usize;
                let mut first_err: Option<String> = None;
                for item in &items {
                    match crate::commands::stock_workflow::run_single_stock_analysis(
                        &db,
                        &client,
                        &engine,
                        &item.stock_code,
                        &item.stock_name,
                        // 自选股扫描无周期语义：持有期交给决策自身声明，缺失时兜底 28 天
                        None,
                    )
                    .await
                    {
                        Ok(id) => {
                            ok += 1;
                            tracing::info!(
                                "[CronScheduler] 自选股扫描 '{}' {} 分析完成: {id}",
                                job_name,
                                item.stock_code
                            );
                        },
                        Err(e) => {
                            fail += 1;
                            if first_err.is_none() {
                                first_err = Some(e.clone());
                            }
                            tracing::warn!(
                                "[CronScheduler] 自选股扫描 '{}' {} 分析失败: {e}",
                                job_name,
                                item.stock_code
                            );
                        },
                    }
                }
                let result = axagent_runtime_core::TaskRunResult {
                    success: fail == 0,
                    output: Some(format!(
                        "自选股扫描完成: {} 只, 成功 {ok}, 失败 {fail}",
                        items.len()
                    )),
                    error: first_err,
                    duration_ms: (axagent_runtime_core::cron_job::now_millis() - started) as u64,
                    executed_at: started,
                };
                store.record_run(&job_id, result).await;
                if !recurring {
                    let _ = store
                        .set_status(&job_id, axagent_runtime_core::CronJobStatus::Disabled)
                        .await;
                }
            });
            return;
        }
        // 候选池逐只分析：task_type = pool-scan
        //
        // [2026-09-13 接线] 补齐「候选池 → 逐只分析」这一环。此前：
        //   - 荐股 cron 扫出的 pick 只推通知、不写候选池（已在上方分支补落库）；
        //   - 唯一的批量逐只分析入口 watchlist-scan 扫的是**自选股**，与候选池无关；
        //   - `stock_pipeline` 模板 trigger = Manual，永不自动调度。
        // 于是候选池里的股票永远不会被自动分析，也就永远进不了反思队列。
        //
        // 配置（lookbackDays / maxStocks / minConfidence）以 JSON 存 CronJob.prompt。
        if job.task_type.as_deref() == Some(crate::commands::pool_scan::POOL_SCAN_TASK_TYPE) {
            let store = cron_store.clone();
            let db = sync_db.clone();
            let client = astock_client.clone();
            let engine = work_engine.clone();
            let job_id = job.id.clone();
            let job_name = job.name.clone();
            let recurring = job.recurring;
            let prompt = job.prompt.clone();
            tokio::task::spawn(async move {
                let started = axagent_runtime_core::cron_job::now_millis();
                let elapsed = || (axagent_runtime_core::cron_job::now_millis() - started) as u64;

                // 解析失败按任务失败记录，不做静默兜底 —— 否则用户设的最大只数
                // 会被"默认值"悄悄替换，而任务列表里显示的仍是原文。
                let config = match crate::commands::pool_scan::PoolScanConfig::from_json(&prompt) {
                    Ok(c) => c,
                    Err(e) => {
                        tracing::error!(
                            "[CronScheduler] 候选池扫描 '{}' 配置解析失败: {e}",
                            job_name
                        );
                        let result = axagent_runtime_core::TaskRunResult {
                            success: false,
                            output: None,
                            error: Some(e),
                            duration_ms: elapsed(),
                            executed_at: started,
                        };
                        store.record_run(&job_id, result).await;
                        if !recurring {
                            let _ = store
                                .set_status(
                                    &job_id,
                                    axagent_runtime_core::CronJobStatus::Disabled,
                                )
                                .await;
                        }
                        return;
                    },
                };

                let result =
                    match crate::commands::pool_scan::run_pool_scan(&db, &client, &engine, &config)
                        .await
                    {
                        Ok(outcome) => {
                            tracing::info!(
                                "[CronScheduler] 候选池扫描 '{}' 完成: {} 只候选, 成功 {}, 失败 {}",
                                job_name,
                                outcome.candidates,
                                outcome.ok,
                                outcome.fail
                            );
                            axagent_runtime_core::TaskRunResult {
                                // 候选池为空不算失败（可能只是当天没扫出 pick），
                                // 但有个别分析失败要如实标记
                                success: outcome.fail == 0,
                                output: Some(outcome.render()),
                                error: outcome.first_error.clone(),
                                duration_ms: elapsed(),
                                executed_at: started,
                            }
                        },
                        Err(e) => {
                            tracing::error!("[CronScheduler] 候选池扫描 '{}' 失败: {e}", job_name);
                            axagent_runtime_core::TaskRunResult {
                                success: false,
                                output: None,
                                error: Some(e),
                                duration_ms: elapsed(),
                                executed_at: started,
                            }
                        },
                    };
                store.record_run(&job_id, result).await;
                if !recurring {
                    let _ = store
                        .set_status(&job_id, axagent_runtime_core::CronJobStatus::Disabled)
                        .await;
                }
            });
            return;
        }
        // 反思：task_type = batch-reflection | validate-decisions
        //
        // [2026-09-13 接线] 此前这两个 task_type 在 CronExecutor 里**没有任何分支**：
        //   - `validate-decisions`：反思面板的开关绑的就是它（ReflectionPanel →
        //     `create_validate_decisions_cron`）⇒ 用户打开开关后，到点什么都不发生；
        //   - `batch-reflection`：设置页可建、前端零调用，真在跑的只有
        //     `init/services.rs::start_batch_reflection` 那个 6 小时硬编码服务。
        //
        // 现在两者都路由到 `run_batch_reflection_inner`（同一套 pending 队列消费逻辑），
        // 差异只在配置：
        //   - batch-reflection: { period, dueOnly, maxCount }
        //       period = ultra_short|short|mid|long（各自间隔 2/5/28/90 天）
        //       ⇒ **按 4 周期间隔分别自动反思**
        //   - validate-decisions: { minConfidence, reflectionDepth, maxCount }
        //       minConfidence 作为 filter 实际参与筛选（此前该配置无人消费）
        if matches!(
            job.task_type.as_deref(),
            Some("batch-reflection") | Some("validate-decisions")
        ) {
            let store = cron_store.clone();
            let db = sync_db.clone();
            let client = astock_client.clone();
            let engine = work_engine.clone();
            let vector_store = cron_vector_store.clone();
            let master_key = cron_master_key;
            let trajectory_storage = cron_trajectory_storage.clone();
            let job_id = job.id.clone();
            let job_name = job.name.clone();
            let recurring = job.recurring;
            let task_type = job.task_type.clone().unwrap_or_default();
            let prompt = job.prompt.clone();
            tokio::task::spawn(async move {
                let started = axagent_runtime_core::cron_job::now_millis();
                let elapsed = || (axagent_runtime_core::cron_job::now_millis() - started) as u64;

                // 1. 解析配置 → 统一 filter + max_count
                let parsed: Result<
                    (
                        crate::commands::stock_workflow::ReflectionFilter,
                        Option<u32>,
                    ),
                    String,
                > = if task_type == "batch-reflection" {
                    crate::commands::stock_workflow::BatchReflectionConfig::from_json(&prompt)
                        .map(|c| (c.to_filter(), c.max_count))
                } else {
                    crate::commands::stock_workflow::ValidateDecisionsConfig::from_json(&prompt)
                        .map(|c| (c.to_filter(), c.max_count))
                };
                let (filter, max_count) = match parsed {
                    Ok(v) => v,
                    Err(e) => {
                        tracing::error!(
                            "[CronScheduler] 反思任务 '{}'({task_type}) 配置解析失败: {e}",
                            job_name
                        );
                        let result = axagent_runtime_core::TaskRunResult {
                            success: false,
                            output: None,
                            error: Some(e),
                            duration_ms: elapsed(),
                            executed_at: started,
                        };
                        store.record_run(&job_id, result).await;
                        if !recurring {
                            let _ = store
                                .set_status(
                                    &job_id,
                                    axagent_runtime_core::CronJobStatus::Disabled,
                                )
                                .await;
                        }
                        return;
                    },
                };

                // 2. 跑一轮批量反思（内部已含卡死 running 回收）
                let r = crate::commands::stock_workflow::run_batch_reflection_inner(
                    &db,
                    &client,
                    &engine,
                    &vector_store,
                    &master_key,
                    max_count,
                    Some(&trajectory_storage),
                    Some(&filter),
                )
                .await;

                let result = match r {
                    Ok(summary) => {
                        tracing::info!(
                            "[CronScheduler] 反思任务 '{}'({task_type}) 完成: {summary}",
                            job_name
                        );
                        axagent_runtime_core::TaskRunResult {
                            success: true,
                            output: Some(summary.to_string()),
                            error: None,
                            duration_ms: elapsed(),
                            executed_at: started,
                        }
                    },
                    Err(e) => {
                        tracing::error!(
                            "[CronScheduler] 反思任务 '{}'({task_type}) 失败: {e}",
                            job_name
                        );
                        axagent_runtime_core::TaskRunResult {
                            success: false,
                            output: None,
                            error: Some(e),
                            duration_ms: elapsed(),
                            executed_at: started,
                        }
                    },
                };
                store.record_run(&job_id, result).await;
                if !recurring {
                    let _ = store
                        .set_status(&job_id, axagent_runtime_core::CronJobStatus::Disabled)
                        .await;
                }
            });
            return;
        }
        if let Some(ref wf_id) = job.workflow_id {
            let engine = work_engine.clone();
            let store = cron_store.clone();
            let wf_id = wf_id.clone();
            let job_id = job.id.clone();
            let job_name = job.name.clone();
            let recurring = job.recurring;
            tokio::task::spawn(async move {
                let started = axagent_runtime_core::cron_job::now_millis();
                let opts = axagent_runtime::work_engine::RunOptions::default();
                let result = match engine.run_workflow(&wf_id, opts).await {
                    Ok(workflow) => {
                        tracing::info!(
                            "[CronScheduler] 工作流任务 '{}' 完成: {:?}",
                            job_name,
                            workflow.status
                        );
                        axagent_runtime_core::TaskRunResult {
                            success: true,
                            output: Some(format!("{:?}", workflow.status)),
                            error: None,
                            duration_ms: (axagent_runtime_core::cron_job::now_millis() - started)
                                as u64,
                            executed_at: started,
                        }
                    },
                    Err(e) => {
                        let err_msg = format!("{:?}", e);
                        tracing::error!(
                            "[CronScheduler] 工作流任务 '{}' 失败: {err_msg}",
                            job_name
                        );
                        axagent_runtime_core::TaskRunResult {
                            success: false,
                            output: None,
                            error: Some(err_msg),
                            duration_ms: (axagent_runtime_core::cron_job::now_millis() - started)
                                as u64,
                            executed_at: started,
                        }
                    },
                };
                store.record_run(&job_id, result).await;
                // 非循环任务执行后禁用
                if !recurring {
                    let _ = store
                        .set_status(&job_id, axagent_runtime_core::CronJobStatus::Disabled)
                        .await;
                }
            });
        } else {
            tracing::info!(
                "[CronScheduler] 触发任务 '{}': {}",
                job.name,
                &job.prompt[..std::cmp::min(job.prompt.len(), 200)]
            );
        }
    });

    let scheduler = Arc::new(CronScheduler::new(store, Arc::new(executor)));

    // 保存到 AppState 以便外部控制（停止/重启）
    {
        let mut state_scheduler = state.cron_scheduler.write().await;
        *state_scheduler = Some(scheduler.clone());
    }

    tauri::async_runtime::spawn(async move {
        scheduler.start().await;
    });

    tracing::info!("[CronScheduler] 已启动（统一 Cron + ScheduledTask），每30秒轮询一次");
}

/// 2.7 P1:启动时从 DB 恢复工作流触发器到运行时 `TriggerManager`。
///
/// 在 `start_cron_scheduler` 之后调用 — `init_trigger_manager` 已在
/// `create_app_state` 中执行,这里只需扫描 `workflow_templates.trigger_config`
/// 字段,对非 Manual 类型触发器批量调用 `register_*`。
///
/// 失败仅 warn 日志,不阻断启动 — 即使所有触发器恢复失败,工作流模板
/// 本身仍然可用,用户可手动触发或通过 update 命令重新激活。
fn start_trigger_recovery(state: &AppState) {
    let db = state.harness.db().clone();
    let trigger_manager = state.work_engine.trigger_manager.clone();
    tauri::async_runtime::spawn(async move {
        let (sched, webhook, event) =
            crate::init::trigger_recovery::recover_workflow_triggers(&db, &trigger_manager).await;
        tracing::info!(
            "[start_trigger_recovery] 触发器恢复完成: {} schedule, {} webhook, {} event",
            sched,
            webhook,
            event
        );
    });
}

/// 夜间长时任务「不丢失」启动钩子：扫描 background_tasks 未完成任务 → 重新入队。
///
/// 设计见 `docs/夜间长时自主任务运行-详细设计.md` ①（Scheduler::restore）。
/// 在 cron 调度器就绪后延迟短暂执行，避免与启动期 DB 初始化竞争。
fn start_scheduler_recovery(app: &tauri::AppHandle, state: &AppState) {
    let app = app.clone();
    let db = state.harness.db().clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_secs(5)).await;
        match crate::scheduler::restore::restore_incomplete_tasks(&db, Some(&app)).await {
            Ok(ids) => {
                if ids.is_empty() {
                    tracing::info!("[start_scheduler_recovery] 无待恢复任务");
                } else {
                    tracing::info!(
                        "[start_scheduler_recovery] 恢复 {} 个任务: {:?}",
                        ids.len(),
                        ids
                    );
                }
            },
            Err(e) => {
                tracing::error!("[start_scheduler_recovery] 恢复引导失败: {}", e);
            },
        }
    });
}

/// 审批事件桥接：订阅统一事件总线，将 rt-workflow 发布的 `ApprovalRequested`
/// 领域事件转发为前端 Tauri 事件 `workflow:approval-requested`。
///
/// 前端审批面板借此实现"推送式"唤醒（主动刷新 pending 并打开面板），
/// 替代纯轮询；不依赖外部 LLM，纯内存总线订阅，启动即挂载。
fn start_approval_event_bridge(app: &tauri::AppHandle, state: &AppState) {
    let app = app.clone();
    let event_bus = state.event_bus.clone();
    tauri::async_runtime::spawn(async move {
        let mut sub = event_bus.subscribe().await;
        loop {
            match sub.recv().await {
                Some(evt) => {
                    if evt.kind == "ApprovalRequested" {
                        let _ = app.emit("workflow:approval-requested", &evt.payload);
                    }
                },
                None => {
                    tracing::debug!("[start_approval_event_bridge] 事件总线已关闭，桥接退出");
                    break;
                },
            }
        }
    });
}

fn start_trajectory_cleanup(state: &AppState) {
    let trajectory_storage = state.trajectory_storage.clone();
    let handle = state.trajectory_cleanup_handle.clone();
    let shutdown_token = state.shutdown_token.clone();
    let config = axagent_trajectory::TrajectoryCleanupConfig::default();
    let config_for_log = config.clone();
    let interval = std::time::Duration::from_secs(24 * 3600);

    tauri::async_runtime::spawn(async move {
        let task = tokio::spawn(async move {
            let mut tick = tokio::time::interval(interval);
            tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                tokio::select! {
                    _ = tick.tick() => {
                        match trajectory_storage.cleanup(&config).await {
                            Ok(count) if count > 0 => {
                                tracing::info!(
                                    "[trajectory_cleanup] Cleaned up {} old trajectories",
                                    count
                                );
                            }
                            Ok(_) => {}
                            Err(e) => {
                                tracing::warn!(
                                    "[trajectory_cleanup] cleanup failed: {}",
                                    e
                                );
                            }
                        }
                    }
                    _ = shutdown_token.cancelled() => {
                        tracing::info!(
                            "[trajectory_cleanup] Received shutdown signal, stopping"
                        );
                        break;
                    }
                }
            }
        });
        *handle.lock().await = Some(task);
    });
    tracing::info!(
        "[trajectory_cleanup] Started with max_age_days={:?}, max_trajectories={:?}, interval=24h",
        config_for_log.max_age_days,
        config_for_log.max_trajectories
    );
}

/// 会话状态（SessionStateStore）过期条目周期清理。
///
/// SessionStateStore 中的条目（已加载能力、工具激活状态、临时中间数据）
/// 全部带 TTL，过期后读取侧视为不存在，但不会自动从数据库删除。
/// 本任务每 6 小时调用一次 `purge_expired`，防止 `session_states` 表持续膨胀。
fn start_session_state_cleanup_tick(state: &AppState) {
    let store = state.session_state_store.clone();
    let shutdown_token = state.shutdown_token.clone();

    tauri::async_runtime::spawn(async move {
        // 延迟 60 秒，确保数据库初始化完成且首轮对话正常运行
        tokio::time::sleep(std::time::Duration::from_secs(60)).await;

        let mut tick = tokio::time::interval(std::time::Duration::from_secs(6 * 3600));
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

        loop {
            tokio::select! {
                _ = tick.tick() => {
                    match store.purge_expired().await {
                        Ok(count) if count > 0 => {
                            tracing::info!(
                                "[session_state_cleanup] 清理 {} 条过期会话状态条目",
                                count
                            );
                        },
                        Ok(_) => {},
                        Err(e) => {
                            tracing::warn!(
                                "[session_state_cleanup] purge_expired 失败: {}",
                                e
                            );
                        },
                    }
                },
                _ = shutdown_token.cancelled() => {
                    tracing::info!(
                        "[session_state_cleanup] 收到关闭信号，停止清理任务"
                    );
                    break;
                },
            }
        }
    });
    tracing::info!("[session_state_cleanup] 已启动，间隔 6 小时");
}

fn start_index_job_service(app: &tauri::AppHandle, state: &AppState) {
    let db = state.harness.db().clone();
    let vector_store = state.vector_store.clone();
    let master_key = state.harness.master_key_owned();
    let shutdown_token = state.shutdown_token.clone();
    let app_handle = app.clone();

    tauri::async_runtime::spawn(async move {
        let service = std::sync::Arc::new(IndexJobService::new(
            db,
            vector_store,
            master_key,
            shutdown_token,
            app_handle,
        ));
        service.start().await;
    });
    tracing::info!("[index_queue] 已启动持久化索引队列 worker");
}

/// 股票全业务管道定时任务：每日 18:00 Asia/Shanghai 自动运行
///
/// 首次延迟 60 秒（等待其他服务初始化），之后计算下一个 18:00 北京时间。
/// 反思阶段由现有 6h cron 接力，此处只管发现+分析+持仓再评估。
pub fn create_cron_delivery_sink(
    _state: &AppState,
) -> Arc<dyn axagent_harness::cron_delivery::CronDeliverySink> {
    let gateway = Arc::new(axagent_rt_messaging::message_gateway::MessageGateway::new());
    Arc::new(crate::init::cron_delivery_sink::GatewayDeliverySink::new(gateway))
}

/// 检索命中反馈应用定时任务。
///
/// 此前 `retrieval_hits` 表只写不读，形成数据沼泽。本任务每小时：
/// 1. 聚合各 KB 的正/负/无关反馈计数（最近 24 小时窗口）
/// 2. 查询全局反馈统计
/// 3. 记录到日志，作为后续 RAG 自适应优化（RL 检索/embedder 微调）的输入信号
///
/// 第一阶段仅做数据采集与日志记录；真正的权重调整需要后续接入 RL 引擎。
fn start_retrieval_feedback_tick(state: &AppState) {
    let harness_state = state.harness.clone();
    tauri::async_runtime::spawn(async move {
        // 首次延迟 5 分钟启动，避免与启动初始化竞争
        tokio::time::sleep(std::time::Duration::from_secs(300)).await;
        let interval = std::time::Duration::from_secs(3600);
        loop {
            tokio::time::sleep(interval).await;

            // 24 小时滑动窗口
            let now = chrono::Utc::now().timestamp();
            let since = now - 86400;

            // 1. 按 KB 聚合反馈
            match axagent_dao::repo::retrieval_hit::aggregate_feedback_by_kb(
                harness_state.db(),
                Some(since),
            )
            .await
            {
                Ok(stats) => {
                    if !stats.is_empty() {
                        tracing::info!(
                            "[retrieval_feedback] 最近 24h KB 反馈聚合：{} 个 KB 有反馈数据",
                            stats.len()
                        );
                        for (kb_id, pos, neg, irr) in &stats {
                            tracing::info!(
                                "[retrieval_feedback] kb={} positive={} negative={} irrelevant={}",
                                kb_id,
                                pos,
                                neg,
                                irr
                            );
                        }
                    }
                },
                Err(e) => {
                    tracing::warn!("[retrieval_feedback] KB 聚合失败: {}", e);
                },
            }

            // 2. 全局反馈统计
            match axagent_dao::repo::retrieval_hit::get_feedback_stats(
                harness_state.db(),
                None,
                Some(since),
            )
            .await
            {
                Ok(stats) => {
                    if stats.total_hits > 0 {
                        tracing::info!(
                            "[retrieval_feedback] 最近 24h 全局统计：total={} positive={} negative={} irrelevant={} no_feedback={} used_in_response={} positive_rate={:.3}",
                            stats.total_hits,
                            stats.positive,
                            stats.negative,
                            stats.irrelevant,
                            stats.no_feedback,
                            stats.used_in_response,
                            stats.positive_rate
                        );
                    }
                },
                Err(e) => {
                    tracing::warn!("[retrieval_feedback] 全局统计失败: {}", e);
                },
            }
        }
    });
    tracing::info!("[retrieval_feedback] 反馈应用定时任务已启动（每小时）");
}

// ── [2026-09-03 投资域接线恢复] ──────────────────────────────────────────
// 以下 5 个服务此前随「另一分支」重构被移除启动接线，本次恢复。
// 功能组件本身一直存在（commands/stock_analysis.rs、stock_workflow、opc_workflows），
// 缺失的只是本文件的启动调用。

/// 前端事件推送桥接：把 RealtimeMonitor 告警桥接到 Tauri 前端事件
struct AppMonitorEmitter {
    app: tauri::AppHandle,
}

impl axagent_analysis_engine::monitor::MonitorEventEmitter for AppMonitorEmitter {
    fn emit(&self, event: &str, payload: serde_json::Value) {
        let _ = self.app.emit(event, payload);
    }
}

/// 启动实时价格监控：恢复 DB 中的价格告警 → RealtimeMonitor 轮询 →
/// 前端 `stock-monitor-alert` 事件 + P2-6 T+0 自动重跑（trigger_t0_rerun）+
/// P3-3 跨股票信号聚合器（组合级告警）。
fn start_realtime_monitor(app: &tauri::AppHandle) {
    use axagent_analysis_engine::alert_mapping::{alert_types, db_model_to_config_field};
    use axagent_analysis_engine::monitor::{MonitorConfig, RealtimeMonitor, TZeroCallback};
    use axagent_harness::market_data::MarketDataProvider;

    // astock_client 依赖 AppState，但在同步段通过 Manager::state 拿引用克隆即可
    {
        use tauri::Manager;
        let state = app.state::<AppState>();
        let client: std::sync::Arc<dyn MarketDataProvider> = state.astock_client.clone();

        // P3-3: 跨股票聚合器（OnceLock 注入，组合级告警）——同步段完成
        let aggregator = std::sync::Arc::new(
            axagent_analysis_engine::cross_stock_aggregator::CrossStockSignalAggregator::new(
                axagent_analysis_engine::cross_stock_aggregator::AggregatorConfig::default(),
            ),
        );
        if state.cross_stock_aggregator.set(aggregator.clone()).is_err() {
            tracing::warn!("[startup] cross_stock_aggregator 已初始化，跳过重复注入");
        }
        // state 守卫（State<'_, AppState> 无 Drop）在本同步块结束时释放
        // monitor 构造（同步，仅依赖 client，无 state 依赖）
        let monitor = RealtimeMonitor::new(client);

        let app_for_spawn = app.clone();
        tauri::async_runtime::spawn(async move {
            use tauri::Manager;
            // 短借 state：只克隆 db，State 守卫不跨 await 持有
            let db = {
                let state = app_for_spawn.state::<AppState>();
                state.harness.db().clone()
            };

            // 事件桥接（stock-monitor-alert）
            monitor
                .set_event_emitter(std::sync::Arc::new(AppMonitorEmitter {
                    app: app_for_spawn.clone(),
                }))
                .await;
            // 组合级告警
            monitor.set_aggregator(aggregator).await;

            // P2-6: T+0 自动重跑回调（异常行情 → 重跑股票分析工作流）
            let app_for_t0 = app_for_spawn.clone();
            let t0_callback: TZeroCallback = std::sync::Arc::new(move |stock_code: String| {
                let app = app_for_t0.clone();
                Box::pin(async move {
                    crate::commands::stock_workflow::core::trigger_t0_rerun(app, stock_code).await
                })
            });
            monitor.set_t0_callback(t0_callback).await;

            // 从 DB 恢复未触发的价格告警 → MonitorConfig（重启后监控不丢）
            use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
            match axagent_entities::price_alerts::Entity::find()
                .filter(axagent_entities::price_alerts::Column::IsTriggered.eq(0))
                .all(&db)
                .await
            {
                Ok(alerts) => {
                    let mut restored = 0usize;
                    for alert in &alerts {
                        let Some((field, value)) = db_model_to_config_field(
                            alert.alert_type.as_deref(),
                            alert.condition_type.as_deref(),
                            alert.threshold,
                            &alert.condition,
                            alert.target_price,
                        ) else {
                            continue;
                        };
                        let mut config = MonitorConfig {
                            stock_code: alert.stock_code.clone(),
                            stock_name: alert.stock_name.clone(),
                            stop_loss: None,
                            take_profit: None,
                            resistance_break: None,
                            support_break: None,
                            change_pct_alert: None,
                            turnover_rate_alert: None,
                            enabled: true,
                        };
                        match field {
                            alert_types::STOP_LOSS => config.stop_loss = Some(value),
                            alert_types::TAKE_PROFIT => config.take_profit = Some(value),
                            alert_types::RESISTANCE => config.resistance_break = Some(value),
                            alert_types::SUPPORT => config.support_break = Some(value),
                            alert_types::CHANGE => config.change_pct_alert = Some(value),
                            alert_types::VOLUME => config.turnover_rate_alert = Some(value),
                            _ => continue,
                        }
                        monitor.add_config(config).await;
                        restored += 1;
                    }
                    tracing::info!("[startup] 价格告警恢复完成：{restored} 条已注入实时监控");
                },
                Err(e) => {
                    tracing::warn!("[startup] 恢复价格告警失败（监控将从空配置启动）: {e}");
                },
            }

            let monitor_arc = std::sync::Arc::new(monitor);
            let injected = {
                let state = app_for_spawn.state::<AppState>();
                state.stock_monitor.set(monitor_arc.clone()).is_ok()
            };
            if !injected {
                tracing::warn!("[startup] stock_monitor 已初始化，跳过重复注入");
                return;
            }
            // 启动轮询（内部永久循环，进程退出即结束）
            monitor_arc.start().await;
        });
    }
}

/// 启动实时行情推送：RealTimeQuoteWatcher 轮询 → 前端 `stock-quote-update` 事件
fn start_realtime_quote_watcher(app: &tauri::AppHandle, state: &AppState) {
    use axagent_astock_data::realtime_quote::{QuoteCallback, RealTimeQuoteWatcher};

    let callback: QuoteCallback = {
        let app = app.clone();
        std::sync::Arc::new(move |event: axagent_astock_data::realtime_quote::QuoteChangeEvent| {
            let app = app.clone();
            // QuoteChangeEvent 未实现 Serialize，手动构造 camelCase payload
            let payload = serde_json::json!({
                "stockCode": event.stock_code,
                "changePct": event.change_pct,
                "trigger": event.trigger,
                "price": event.current.price,
                "name": event.current.name,
                "volume": event.current.volume,
                "turnoverRate": event.current.turnover_rate,
                "timestamp": event.current.timestamp,
            });
            Box::pin(async move {
                let _ = app.emit("stock-quote-update", payload);
            })
        })
    };

    let watcher = RealTimeQuoteWatcher::new(state.astock_client.clone(), Some(callback));
    if state.quote_watcher.set(watcher).is_err() {
        tracing::warn!("[startup] quote_watcher 已初始化，跳过重复注入");
        return;
    }
    if let Some(w) = state.quote_watcher.get() {
        w.start();
        tracing::info!("[startup] 实时行情推送已启动（stock-quote-update 事件）");
    }
}

/// 启动批量反思 cron（每 6 小时）：扫 pending 反思队列 → LLM 反思 → 回测验证
fn start_batch_reflection(state: &AppState) {
    let db = state.harness.db().clone();
    let client = state.astock_client.clone();
    let engine = state.work_engine.clone();
    let vector_store = state.vector_store.clone();
    let master_key = state.harness.master_key_owned();
    let trajectory_storage = state.trajectory_storage.clone();

    tauri::async_runtime::spawn(async move {
        // 启动延迟 5 分钟，避开启动高峰
        tokio::time::sleep(std::time::Duration::from_secs(300)).await;
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(6 * 3600));
        loop {
            interval.tick().await;
            match crate::commands::stock_workflow::run_batch_reflection_inner(
                &db,
                &client,
                &engine,
                &vector_store,
                &master_key,
                None,
                Some(&trajectory_storage),
                // 兜底服务不限定周期档位：处理全部 pending。
                // 「按 4 周期分档」由 cron 的 batch-reflection 任务各自指定 filter。
                None,
            )
            .await
            {
                Ok(summary) => {
                    tracing::info!("[batch-reflection] 批量反思完成: {summary}");
                },
                Err(e) => {
                    tracing::warn!("[batch-reflection] 批量反思失败: {e}");
                },
            }
        }
    });
    tracing::info!("[startup] 批量反思定时任务已启动（每 6 小时）");
}

/// 启动 OPC 需求发现 cron（每 12 小时）：扫描已启用平台 → 评估线索 → 高价值推送
fn start_demand_discovery_cron(app: &tauri::AppHandle, state: &AppState) {
    let db = state.harness.db().clone();
    let app = app.clone();

    tauri::async_runtime::spawn(async move {
        // 首轮延迟 10 分钟（等平台凭证配置完成后再扫）
        tokio::time::sleep(std::time::Duration::from_secs(600)).await;
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(12 * 3600));
        loop {
            interval.tick().await;
            match crate::commands::demand_discovery::run_demand_discovery_cron(
                &db,
                None,
                Some(&app),
            )
            .await
            {
                Ok(summary) => {
                    tracing::info!("[demand-discovery] 定时扫描完成: {summary}");
                },
                Err(e) => {
                    tracing::warn!("[demand-discovery] 定时扫描失败: {e}");
                },
            }
        }
    });
    tracing::info!("[startup] 需求发现定时任务已启动（每 12 小时）");
}

/// OPC 域包/领域工作流模板种子化（后台异步，幂等 upsert）
///
/// 激活 opc_workflows 全部 seed 模块：14 域包 + 17 领域 + 生产/内容媒体模板 +
/// route_path 回填。TEMPLATE_VERSION 校验在 seed 函数内部完成。
fn spawn_opc_workflows_seeding(state: &AppState) {
    let db = state.harness.db().clone();
    let app_dir = state.app_data_dir.clone();

    tauri::async_runtime::spawn(async move {
        // 延迟 3 秒，确保 migration 完成后再种子化
        tokio::time::sleep(std::time::Duration::from_secs(3)).await;
        match crate::commands::opc_workflows::ensure_opc_workflows_seeded(&db, Some(&app_dir)).await
        {
            Ok(()) => {
                tracing::info!("[opc-workflows] 工作流模板种子化完成");
            },
            Err(e) => {
                tracing::warn!("[opc-workflows] 工作流模板种子化失败: {e}");
            },
        }
    });
}
