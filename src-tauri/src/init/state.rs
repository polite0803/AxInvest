use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use tokio::sync::Mutex;
use tokio::sync::RwLock as TokioRwLock;

use super::database::DatabaseInitResult;
use crate::AppState;
use crate::app_state::SemanticCacheState;
use crate::commands::proactive::ProactiveService;
use crate::semantic_cache::{CacheConfig, SemanticCache};
use axagent_astock_data::AStockClient;
use axagent_core::cloud_storage::{CloudStorageConfig, SyncEngine};
use axagent_plugins::{PluginManager, PluginManagerConfig};
use axagent_runtime_core::prompt_cache::PromptCache;
use tokio_util::sync::CancellationToken;

/// 构造 AppState。
///
/// 失败时返回结构化错误，由调用方决定如何处理（错误展示 / 重试 / 退出）。
/// 不再 `process::exit(1)`——harness 架构要求启动错误可被前端感知。
pub fn create_app_state(db_result: DatabaseInitResult) -> Result<AppState, String> {
    let DatabaseInitResult {
        db_handle,
        master_key,
        db_path,
        app_dir,
        ..
    } = db_result;

    // db_handle 进入 harness（Step 4）；同时克隆 conn 给其它需要 DatabaseConnection 的
    // 旧式组件（vector_store / trajectory_storage / cron / semantic_cache 等）。
    // 这些组件后续在 Step 5/6 也会迁到 harness 内部。
    let sea_db = db_handle.conn.clone();

    let vector_store = axagent_core::vector_store::VectorStore::new(sea_db.clone());
    let vector_store_arc = Arc::new(vector_store);

    {
        let db_conn = sea_db.clone();
        let mk = master_key;
        let vs = vector_store_arc.clone();
        axagent_tools::knowledge_callback::set_knowledge_search_callback(std::sync::Arc::new(
            move |base_id: &str, query: &str, top_k: usize| {
                let db = db_conn.clone();
                let vs2 = vs.clone();
                let bid = base_id.to_string();
                let q = query.to_string();
                Box::pin(async move {
                    let results =
                        crate::indexing::search_knowledge(&db, &mk, &vs2, &bid, &q, top_k).await?;
                    Ok(results
                        .into_iter()
                        .map(|r| axagent_tools::knowledge_callback::KnowledgeSearchHit {
                            document_id: r.document_id,
                            chunk_index: r.chunk_index,
                            content: r.content,
                            score: r.score,
                        })
                        .collect())
                })
            },
        ));
    }

    let rt = tokio::runtime::Runtime::new()
        .or_else(|e| {
            tracing::warn!("Failed to create multi-threaded runtime for state init: {} — falling back to current-thread", e);
            tokio::runtime::Builder::new_current_thread().enable_all().build()
        })
        .map_err(|e| {
            tracing::error!("Failed to create init runtime in state: {}", e);
            crate::android_utils::report_fatal_error(&format!(
                "Init runtime creation failed in state: {}",
                e
            ));
            format!("Init runtime creation failed in state: {}", e)
        })?;
    // ensure_preset_servers / migrate_hardcoded_paths / migrate_legacy_keys
    // 已合并到 axagent_core::db::create_pool() 中，无需在此重复调用

    let app_settings = rt
        .block_on(axagent_core::repo::settings::get_settings(&sea_db))
        .unwrap_or_default();

    axagent_core::storage_paths::init_documents_root(
        app_settings
            .documents_root_override
            .as_ref()
            .map(PathBuf::from),
    );
    axagent_core::storage_paths::ensure_documents_dirs().unwrap_or_else(|e| {
        tracing::warn!("Failed to create documents storage dirs (non-critical on mobile): {}", e);
    });

    let shared_trajectory_storage: Arc<axagent_trajectory::TrajectoryStorage> = {
        let db_file_path = db_path.strip_prefix("sqlite:").unwrap_or(&db_path);
        let storage = rt
            .block_on(axagent_trajectory::TrajectoryStorage::with_fts_path(
                Arc::new(sea_db.clone()),
                db_file_path,
            ))
            .unwrap_or_else(|e| {
                tracing::warn!("Failed to init trajectory FTS5, falling back to no-FTS: {}", e);
                axagent_trajectory::TrajectoryStorage::new(Arc::new(sea_db.clone()))
            });
        Arc::new(storage)
    };

    let memory_service = {
        let ms = match axagent_trajectory::MemoryService::new(shared_trajectory_storage.clone()) {
            Ok(ms) => ms,
            Err(e) => {
                tracing::error!("Failed to create MemoryService: {} — retrying once", e);
                match axagent_trajectory::MemoryService::new(shared_trajectory_storage.clone()) {
                    Ok(ms) => ms,
                    Err(e2) => {
                        tracing::error!(
                            "MemoryService creation failed after retry: {} — creating with fresh storage",
                            e2
                        );
                        // 用新 TrajectoryStorage 兜底，避免 panic 导致 Android 静默崩溃
                        let fallback_storage =
                            std::sync::Arc::new(axagent_trajectory::TrajectoryStorage::new(
                                std::sync::Arc::new(sea_db.clone()),
                            ));
                        match axagent_trajectory::MemoryService::new(fallback_storage) {
                            Ok(ms) => ms,
                            Err(e3) => {
                                let msg = format!("MemoryService unreachable path reached: {}", e3);
                                crate::android_utils::report_fatal_error(&msg);
                                return Err(msg);
                            },
                        }
                    },
                }
            },
        };
        if let Err(e) = ms.initialize() {
            tracing::warn!("Failed to initialize MemoryService: {}", e);
        }
        Arc::new(TokioRwLock::new(ms))
    };

    // ── 初始化 Harness 容器（统一管理核心基础设施注入） ──
    let harness =
        axagent_runtime::harness::RuntimeHarness::new(axagent_runtime::harness::HarnessDeps {
            persistence: Arc::new(db_handle) as axagent_harness::SharedPersistence,
            master_key,
            provider_registry: Arc::new(
                axagent_providers::registry::ProviderRegistry::create_default(),
            )
                as Arc<dyn axagent_harness::registry::ProviderRegistry>,
        });
    let harness_registry = harness.provider_registry().clone();

    let platform_manager =
        Arc::new(axagent_runtime::message_gateway::platform_manager::PlatformManager::new());

    let platform_bridge = harness.build_platform_bridge(platform_manager.clone());

    rt.block_on(platform_manager.set_message_callback(platform_bridge.clone()));

    let sync_engine = create_sync_engine(&sea_db, &app_settings, rt.handle());

    // 共享 AStockClient：astock_client 和 stock_monitor 共用同一实例（共享缓存）
    // 缺陷 D 修复: 注入 L2 磁盘缓存(持久化跨进程) + 启动后台 flush 任务。
    let (astock_client, l2_handle) = {
        let l2_path: PathBuf = dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".axagent")
            .join("astock_l2_cache.json");
        if let Some(parent) = l2_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        AStockClient::new().with_l2_cache(l2_path)
    };
    let astock_client = Arc::new(astock_client);
    axagent_tools::global_state::set_astock_client(astock_client.clone());
    // 启动 30s flush loop(后台 tokio task)
    let _guard = rt.enter();
    axagent_astock_data::disk_cache::spawn_flush_loop(l2_handle);
    drop(_guard);
    tracing::info!("[l2] 磁盘缓存已注入,后台 flush 任务已启动");

    // 缺陷 A 修复:启动后异步拉一次 A 股交易日历,填充 calendar.rs 远程节假日缓存。
    // fire-and-forget,失败也不影响主流程(会 fallback 到硬编码 2025-2026)。
    {
        let astock_for_calendar = astock_client.clone();
        rt.spawn(async move {
            // 给主流程 5s 时间先起来,避免冷启动 IO 拥塞
            tokio::time::sleep(std::time::Duration::from_secs(5)).await;
            match axagent_astock_data::calendar::init_holiday_calendar().await {
                Ok(n) => tracing::info!("[calendar] 远程节假日缓存初始化: {} 条", n),
                Err(e) => {
                    tracing::warn!("[calendar] 远程节假日缓存初始化失败,fallback 硬编码: {}", e)
                },
            }
            // 抑制 unused 警告
            let _ = astock_for_calendar;
        });
    }
    let stock_monitor = Some(Arc::new(axagent_stock_analysis::monitor::RealtimeMonitor::new(
        astock_client.clone(),
    )));
    let trading_engine =
        Arc::new(tokio::sync::RwLock::new(axagent_stock_analysis::trading::TradingEngine::new(
            Arc::new(sea_db.clone()),
            astock_client.clone(),
        )));

    let home = dirs::home_dir().unwrap_or_default();
    let config_home = home.join(".claw");
    let mut plugin_config = PluginManagerConfig::new(config_home.clone());
    plugin_config.external_dirs = axagent_core::skill_dirs::all_skills_dirs();
    let plugin_manager = Arc::new(tokio::sync::RwLock::new(PluginManager::new(plugin_config)));

    Ok(AppState {
        harness,
        gateway: Arc::new(Mutex::new(None)),
        close_to_tray: Arc::new(AtomicBool::new(false)),
        app_data_dir: app_dir.clone(),
        auto_backup_handle: Arc::new(Mutex::new(None)),
        webdav_sync_handle: Arc::new(Mutex::new(None)),
        api_server_handle: Arc::new(Mutex::new(None)),
        trajectory_cleanup_handle: Arc::new(Mutex::new(None)),
        task_manager: Arc::new(axagent_runtime::task_manager::TaskManager::new()),
        skill_watcher_shutdown: std::sync::OnceLock::new(),
        shutdown_token: CancellationToken::new(),
        vector_store: vector_store_arc,
        indexing_semaphore: Arc::new(tokio::sync::Semaphore::new(2)),
        stream_cancel_flags: Arc::new(Mutex::new(std::collections::HashMap::new())),
        agent_permission_senders: Arc::new(Mutex::new(std::collections::HashMap::new())),
        agent_ask_senders: Arc::new(Mutex::new(std::collections::HashMap::new())),
        agent_always_allowed: Arc::new(Mutex::new(std::collections::HashMap::new())),
        agent_prompters: Arc::new(Mutex::new(std::collections::HashMap::new())),
        agent_session_manager: Arc::new(axagent_agent::SessionManager::new(sea_db.clone())),
        agent_cancel_tokens: Arc::new(Mutex::new(std::collections::HashMap::new())),
        agent_paused: Arc::new(Mutex::new(std::collections::HashSet::new())),
        running_agents: Arc::new(tokio::sync::RwLock::new(std::collections::HashSet::new())),
        reflector: {
            let r = Arc::new(axagent_agent::Reflector::new());
            // 异步加载历史反思（不阻塞主流程）
            // 注意:此处必须用 `rt.spawn` 而非裸 `tokio::spawn`。
            // create_app_state 由 lib.rs 中的 `std::thread::spawn` 调用,
            // 当前线程不是 Tokio 线程、没有 reactor,裸 tokio::spawn 会 panic:
            //   "there is no reactor running, must be called from the context of a Tokio 1.x runtime"
            let r_clone = r.clone();
            let reflection_path: std::path::PathBuf = app_dir.join("reflections.jsonl");
            // S3: 同时挂载 insight 持久化路径
            let insight_path: std::path::PathBuf = app_dir.join("insights.jsonl");
            let ig_clone = r_clone.get_insight_generator();
            rt.spawn(async move {
                if let Err(e) = r_clone.init_persistence(reflection_path, 200).await {
                    tracing::warn!("[reflector] init_persistence failed: {}", e);
                }
                match ig_clone.init_persistence(insight_path).await {
                    Ok(n) => {
                        tracing::info!("[insight] loaded {} insights from disk", n);
                    },
                    Err(e) => {
                        tracing::warn!("[insight] init_persistence failed: {}", e);
                    },
                }
            });
            r
        },
        shared_memory: Arc::new(TokioRwLock::new(
            axagent_runtime::shared_memory::SharedMemory::new(),
        )),
        sub_agent_registry: Arc::new(TokioRwLock::new(
            axagent_trajectory::SubAgentRegistry::new().unwrap_or_default(),
        )),
        memory_service: memory_service.clone(),
        nudge_service: Arc::new(tokio::sync::Mutex::new(axagent_trajectory::NudgeService::new())),
        closed_loop_service: Arc::new(axagent_trajectory::ClosedLoopService::new(
            shared_trajectory_storage.clone(),
        )),
        trajectory_storage: shared_trajectory_storage.clone(),
        insight_system: Arc::new(TokioRwLock::new(
            axagent_trajectory::LearningInsightSystem::new().with_storage_limits(200, 30),
        )),
        realtime_learning: Arc::new(tokio::sync::Mutex::new(
            axagent_trajectory::RealTimeLearning::new(),
        )),
        pattern_learner: Arc::new(TokioRwLock::new(axagent_trajectory::PatternLearner::new(
            axagent_trajectory::PatternConfig::default(),
        ))),
        cross_session_learner: Arc::new(TokioRwLock::new(
            axagent_trajectory::CrossSessionLearner::new(),
        )),
        rl_engine: Arc::new(TokioRwLock::new(axagent_trajectory::RLEngine::new(
            axagent_trajectory::RLConfig::default(),
            axagent_trajectory::RewardWeights::default(),
        ))),
        batch_processor: Arc::new(axagent_trajectory::BatchProcessor::new(
            shared_trajectory_storage.clone(),
            axagent_trajectory::BatchConfig::default(),
        )),
        #[cfg(not(target_os = "android"))]
        skill_evolution_engine: Arc::new(tokio::sync::Mutex::new({
            let mut engine = axagent_trajectory::SkillEvolutionEngine::new();
            engine.set_sandbox(Arc::new(
                axagent_trajectory::SkillSandboxExecutor::with_default_policy(),
            ));
            engine
        })),
        #[cfg(target_os = "android")]
        skill_evolution_engine: Arc::new(tokio::sync::Mutex::new(
            axagent_trajectory::SkillEvolutionEngine::new(),
        )),
        skill_proposal_service: Arc::new(TokioRwLock::new(
            axagent_trajectory::SkillProposalService::new(shared_trajectory_storage.clone()),
        )),
        auto_memory_extractor: {
            let auto_ms = match axagent_trajectory::MemoryService::new(
                shared_trajectory_storage.clone(),
            ) {
                Ok(ms) => ms,
                Err(e) => {
                    tracing::warn!(
                        "Failed to create MemoryService for AutoMemory: {} — falling back to primary memory service",
                        e
                    );
                    // 回退到主 memory_service（克隆引用），避免 panic 导致 Android 静默崩溃
                    match axagent_trajectory::MemoryService::new(shared_trajectory_storage.clone())
                    {
                        Ok(ms) => ms,
                        Err(e2) => {
                            tracing::error!(
                                "AutoMemory MemoryService fallback also failed: {} — creating with fresh storage",
                                e2
                            );
                            let fallback_storage =
                                std::sync::Arc::new(axagent_trajectory::TrajectoryStorage::new(
                                    std::sync::Arc::new(sea_db.clone()),
                                ));
                            match axagent_trajectory::MemoryService::new(fallback_storage) {
                                Ok(ms) => ms,
                                Err(e3) => {
                                    let msg =
                                        format!("AutoMemory MemoryService unreachable: {}", e3,);
                                    crate::android_utils::report_fatal_error(&msg);
                                    return Err(msg);
                                },
                            }
                        },
                    }
                },
            };
            if let Err(e) = auto_ms.initialize() {
                tracing::warn!("Failed to initialize MemoryService for AutoMemory: {}", e);
            }
            let auto_ms = Arc::new(tokio::sync::RwLock::new(auto_ms));
            let auto_pl =
                Arc::new(tokio::sync::RwLock::new(axagent_trajectory::PatternLearner::new(
                    axagent_trajectory::PatternConfig::default(),
                )));
            Arc::new(TokioRwLock::new(axagent_trajectory::AutoMemoryExtractor::new(
                shared_trajectory_storage.clone(),
                auto_ms,
                auto_pl,
            )))
        },
        parallel_execution_service: Arc::new(tokio::sync::RwLock::new(
            axagent_trajectory::ParallelExecutionService::new(10),
        )),
        cron_job_store: Arc::new(
            rt.block_on(axagent_runtime_core::CronJobStore::new(Arc::new(sea_db.clone()))),
        ),
        platform_manager: platform_manager.clone(),
        platform_bridge: platform_bridge.clone(),
        user_profile: Arc::new(TokioRwLock::new(axagent_trajectory::UserProfile::new())),
        local_tool_registry: {
            let mut registry = axagent_tools::registry::UnifiedToolRegistry::new();
            rt.block_on(registry.load_enabled_state(&sea_db));
            Arc::new(tokio::sync::Mutex::new(registry))
        },
        work_engine: {
            let engine = Arc::new(axagent_runtime::work_engine::WorkEngine::new(
                Arc::new(sea_db.clone()),
                master_key,
                harness_registry.clone(),
            ));
            // Plan 模式：AgentExecutor 注入 engine 引用以创建/执行临时工作流
            rt.block_on(engine.inject_into_agent_executor(engine.clone()));
            // 注册领域约束：股票角色走特定约束（as-of 时间锚定 + A 股规则），
            // 其他角色走通用 DomainConstraints::by_role
            rt.block_on(engine.set_domain_constraints(Arc::new(|role_name: &str| {
                let as_of_date: Option<String> =
                    axagent_astock_data::as_of::current_as_of().map(|c| c.as_string());
                let is_stock = axagent_stock_analysis::prompts::is_stock_role(role_name);
                if is_stock {
                    if let Some(date) = as_of_date.as_deref() {
                        let asof_block = axagent_stock_analysis::prompts::asof_system_prompt(date);
                        let head = format!(
                            "{asof_block}

{}",
                            axagent_stock_analysis::prompts::STOCK_HARD_CONSTRAINTS
                        );
                        ConstraintBlocks::default()
                            .with_head(head)
                            .with_tail(axagent_stock_analysis::prompts::STOCK_COLLAB_REMINDER)
                    } else {
                        ConstraintBlocks::default()
                            .with_head(axagent_stock_analysis::prompts::STOCK_HARD_CONSTRAINTS)
                            .with_tail(axagent_stock_analysis::prompts::STOCK_COLLAB_REMINDER)
                    }
                } else {
                    // 非股票角色：使用通用领域约束
                    axagent_rt_workflow::work_engine::domain_constraints::DomainConstraints::by_role(role_name)
                }
            })));
            })));
            engine
        },
        skill_decomposer: Arc::new(tokio::sync::RwLock::new(
            axagent_trajectory::SkillDecomposer::new(),
        )),
        proactive_service: Arc::new(tokio::sync::RwLock::new(ProactiveService::new())),
        dashboard_registry: Some(Arc::new(
            axagent_runtime::dashboard_registry::DashboardRegistry::new_with_config(
                axagent_runtime::dashboard_registry::DashboardRegistryConfig {
                    plugin_dirs: vec![
                        axagent_core::storage_paths::documents_root().join("dashboard-plugins"),
                    ],
                    auto_load: true,
                },
            ),
        )),
        webhook_subscription_manager: Some(Arc::new(
            axagent_runtime::webhook_subscription::WebhookSubscriptionManager::new(),
        )),
        semantic_cache: {
            let cache = match rt
                .block_on(SemanticCache::new(sea_db.clone(), CacheConfig::default()))
            {
                Ok(c) => c,
                Err(e) => {
                    tracing::error!("Semantic cache init failed: {} — retrying once", e);
                    match rt.block_on(SemanticCache::new(sea_db.clone(), CacheConfig::default())) {
                        Ok(c) => c,
                        Err(e2) => {
                            // 数据库初始化已成功，两次失败表明 CREATE TABLE 持续出错。
                            // 回退到内存 SQLite，应用正常运行但缓存不持久化。
                            tracing::error!(
                                "Semantic cache failed permanently: {} — using in-memory fallback (non-persistent cache)",
                                e2
                            );
                            let fallback_db =
                                rt.block_on(sea_orm::Database::connect("sqlite::memory:"));
                            match fallback_db {
                                Ok(mem_db) => rt
                                    .block_on(SemanticCache::new(mem_db, CacheConfig::default()))
                                    .map_err(|e3| {
                                        crate::android_utils::report_fatal_error(&format!(
                                            "SemanticCache in-memory fallback failed: {}",
                                            e3,
                                        ));
                                        format!("SemanticCache in-memory fallback failed: {}", e3)
                                    })?,
                                Err(e3) => {
                                    let msg = format!(
                                        "SemanticCache in-memory DB connect failed: {}",
                                        e3,
                                    );
                                    crate::android_utils::report_fatal_error(&msg);
                                    return Err(msg);
                                },
                            }
                        },
                    }
                },
            };
            Arc::new(tokio::sync::Mutex::new(SemanticCacheState {
                cache,
                enabled: true,
                in_memory_entries: Vec::new(),
                similarity_threshold: 0.85,
            }))
        },
        prompt_cache: Arc::new(PromptCache::new()),
        tot_sessions: Arc::new(tokio::sync::Mutex::new(std::collections::HashMap::new())),
        planner_sessions: Arc::new(tokio::sync::Mutex::new(std::collections::HashMap::new())),
        browser_client: Arc::new(tokio::sync::Mutex::new(None)),
        dream_consolidator: Arc::new(
            axagent_trajectory::DreamConsolidator::new().with_data_provider(Arc::new(
                axagent_trajectory::TrajectoryDreamDataProvider::new(
                    shared_trajectory_storage.clone(),
                ),
            )),
        ),
        text_grad_engine: Arc::new(tokio::sync::Mutex::new(
            axagent_trajectory::TextGradEngine::new(
                axagent_trajectory::ComputationGraph::new(),
                axagent_trajectory::TextGradConfig::default(),
            ),
        )),
        auto_tool_creator: Arc::new(tokio::sync::Mutex::new(
            axagent_trajectory::AutoToolCreator::new(
                axagent_trajectory::AutoToolCreatorConfig::default(),
                Box::new(axagent_trajectory::DefaultLlmToolProvider::new()),
                Box::new(axagent_trajectory::DefaultSandboxToolTester),
            ),
        )),
        intrinsic_motivation: Arc::new(tokio::sync::Mutex::new(
            axagent_trajectory::IntrinsicMotivationEngine::new(
                axagent_trajectory::IntrinsicMotivationConfig::default(),
            ),
        )),
        coevolution_env: Arc::new(tokio::sync::Mutex::new(
            axagent_trajectory::CoevolutionEnvironment::new(
                axagent_trajectory::CoevolutionConfig::default(),
            ),
        )),
        constitution: Arc::new(axagent_trajectory::ImmutableConstitution::new(
            vec![
                axagent_trajectory::ConstitutionalRule::NoSelfModificationOfReward,
                axagent_trajectory::ConstitutionalRule::NoCodeExecutionWithoutSandbox,
                axagent_trajectory::ConstitutionalRule::PreserveUserIntent,
                axagent_trajectory::ConstitutionalRule::MaxModificationSize(0.5),
            ],
            axagent_trajectory::ConstitutionConfig::default(),
        )),
        process_reward_model: Arc::new(tokio::sync::Mutex::new(
            axagent_trajectory::ProcessRewardModel::default().with_default_provider("general"),
        )),
        dream_data_provider: Arc::new(axagent_trajectory::TrajectoryDreamDataProvider::new(
            shared_trajectory_storage.clone(),
        )),
        #[cfg(not(target_os = "android"))]
        sandbox_executor: Arc::new(axagent_trajectory::SkillSandboxExecutor::with_default_policy()),
        #[cfg(target_os = "android")]
        sandbox_executor: Arc::new(()),
        sync_engine,
        astock_client,
        stock_monitor,
        trading_engine,
        plugin_manager,
        file_authorizer: Arc::new(axagent_core::file_authorizer::FileAuthorizer::new()),
        session_share_manager: Arc::new(TokioRwLock::new(std::collections::HashMap::new())),
    })
}

fn create_sync_engine(
    _sea_db: &sea_orm::DatabaseConnection,
    _app_settings: &axagent_harness::types::AppSettings,
    rt_handle: &tokio::runtime::Handle,
) -> Option<Arc<SyncEngine>> {
    let cloud_config = load_cloud_storage_config(_sea_db, _app_settings, rt_handle)?;
    let backend = cloud_config.create_backend().ok()?;
    let device_id = hostname_or_uuid();
    let profile_name = cloud_config.profile_name.clone();
    Some(Arc::new(SyncEngine::new(backend, &profile_name, &device_id)))
}

fn load_cloud_storage_config(
    sea_db: &sea_orm::DatabaseConnection,
    _app_settings: &axagent_harness::types::AppSettings,
    rt_handle: &tokio::runtime::Handle,
) -> Option<CloudStorageConfig> {
    use axagent_core::cloud_storage::{BackendType, S3Config, S3ProviderPreset, SyncMode};
    let settings = rt_handle
        .block_on(axagent_core::repo::settings::get_settings(sea_db))
        .ok()?;

    if !settings.cloud_sync_enabled {
        return None;
    }

    let backend_type = match settings.cloud_backend.as_deref() {
        Some("s3") => BackendType::S3,
        Some("webdav") => BackendType::WebDav,
        _ => return None,
    };

    let cloud_config = CloudStorageConfig {
        provider_preset: settings
            .s3_provider_preset
            .and_then(|v| serde_json::from_value(v).ok())
            .unwrap_or(S3ProviderPreset::Custom),
        backend_type,
        sync_enabled: true,
        sync_mode: SyncMode::Sync,
        profile_name: settings
            .sync_profile_name
            .clone()
            .unwrap_or_else(|| "default".to_string()),
        webdav: settings
            .webdav_host
            .as_ref()
            .map(|h| axagent_core::cloud_storage::WebDavConfig {
                host: h.clone(),
                username: settings.webdav_username.clone().unwrap_or_default(),
                password: settings.webdav_password.clone().unwrap_or_default(),
                path: settings.webdav_path.clone().unwrap_or_default(),
                accept_invalid_certs: settings.webdav_accept_invalid_certs,
            }),
        s3: settings.s3_endpoint.as_ref().map(|e| S3Config {
            endpoint: e.clone(),
            region: settings.s3_region.clone().unwrap_or_default(),
            bucket: settings.s3_bucket.clone().unwrap_or_default(),
            access_key_id: settings.s3_access_key_id.clone().unwrap_or_default(),
            secret_access_key: settings.s3_secret_access_key.clone().unwrap_or_default(),
            root: settings.s3_root.clone().unwrap_or_default(),
            use_path_style: settings.s3_use_path_style,
        }),
    };

    Some(cloud_config)
}

fn hostname_or_uuid() -> String {
    std::env::var("HOSTNAME")
        .ok()
        .or_else(|| std::env::var("COMPUTERNAME").ok())
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string())
}
