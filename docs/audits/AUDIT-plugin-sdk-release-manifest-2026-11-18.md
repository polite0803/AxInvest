# AUDIT — 插件 SDK 发布物清单（2026-11-18）

> 状态：active ｜ 日期：2026-11-18 ｜ 对应版本：v3.0.1
> 关联：`PLAN-p62-domain-pack-plugins.md`、`PLAN-shell-plus-plugins.md`
> 性质：由 `generate-plugin-sdk-release-manifest.mjs` 生成的可复跑清单；**不表示已发布**。

## 一、发布位

| crate | manifest | version | license | publish | package-list sha256 前缀 | 入口 |
|---|---|---|---|---|---|---|
| `axagent-plugin-proto` | `axagent-core/crates/plugin-proto/Cargo.toml` | 3.0.0 | AGPL-3.0-only | false | 55bc9d177a860d5c | ✓ |
| `axagent-harness` | `axagent-core/crates/harness/Cargo.toml` | workspace | AGPL-3.0-only | false | 91f83f1a73364570 | ✓ |
| `axagent-plugin-testkit` | `axagent-plugins/crates/plugin-testkit/Cargo.toml` | workspace | workspace | false | 66a638cc28e71b84 | ✓ |

## 二、依赖边

| crate | 依赖 | 解析形态 |
|---|---|---|
| `axagent-plugin-proto` | `serde` | registry / workspace |
| `axagent-plugin-proto` | `serde_json` | registry / workspace |
| `axagent-plugin-proto` | `thiserror` | registry / workspace |
| `axagent-harness` | `anyhow` | registry / workspace |
| `axagent-harness` | `async-trait` | registry / workspace |
| `axagent-harness` | `chrono` | registry / workspace |
| `axagent-harness` | `futures` | registry / workspace |
| `axagent-harness` | `hex` | registry / workspace |
| `axagent-harness` | `parking_lot` | registry / workspace |
| `axagent-harness` | `regex` | registry / workspace |
| `axagent-harness` | `reqwest` | registry / workspace |
| `axagent-harness` | `rhai` | registry / workspace |
| `axagent-harness` | `schemars` | registry / workspace |
| `axagent-harness` | `sea-orm` | registry / workspace |
| `axagent-harness` | `serde` | registry / workspace |
| `axagent-harness` | `serde_json` | registry / workspace |
| `axagent-harness` | `serde_yaml` | registry / workspace |
| `axagent-harness` | `sha2` | registry / workspace |
| `axagent-harness` | `static_assertions` | registry / workspace |
| `axagent-harness` | `thiserror` | registry / workspace |
| `axagent-harness` | `tokio` | registry / workspace |
| `axagent-harness` | `tracing` | registry / workspace |
| `axagent-harness` | `ts-rs` | registry / workspace |
| `axagent-harness` | `uuid` | registry / workspace |
| `axagent-plugin-testkit` | `axagent-plugin-proto` | `../../../axagent-core/crates/plugin-proto` |
| `axagent-plugin-testkit` | `serde_json` | registry / workspace |

## 三、包内文件

### `axagent-plugin-proto`

仓库：https://github.com/polite0803/AxInvest

包内文件：

- `.cargo_vcs_info.json`
- `Cargo.lock`
- `Cargo.toml`
- `Cargo.toml.orig`
- `src/lib.rs`

### `axagent-harness`

仓库：https://github.com/polite0803/AxInvest

包内文件：

- `.cargo_vcs_info.json`
- `Cargo.lock`
- `Cargo.toml`
- `Cargo.toml.orig`
- `bindings/Model.ts`
- `bindings/ModelCapability.ts`
- `bindings/ModelParamOverrides.ts`
- `bindings/ModelType.ts`
- `bindings/ProviderConfig.ts`
- `bindings/ProviderKey.ts`
- `bindings/ProviderProxyConfig.ts`
- `bindings/ProviderType.ts`
- `examples/port_axiom_audit.rs`
- `generated/task_scene.ts`
- `src/adaptation.rs`
- `src/agent.rs`
- `src/agent_session_broker.rs`
- `src/agent_session_repo.rs`
- `src/agent_turn_runner.rs`
- `src/analysis_kind.rs`
- `src/analysis_record.rs`
- `src/approval_policy.rs`
- `src/approval_rules.rs`
- `src/as_of.rs`
- `src/assembly_builder.rs`
- `src/audit_trail.rs`
- `src/auxiliary_client.rs`
- `src/benchmark.rs`
- `src/browser.rs`
- `src/business_rules.rs`
- `src/business_state_machine.rs`
- `src/cache_interceptor.rs`
- `src/cache_service.rs`
- `src/candlestick_pattern.rs`
- `src/capability.rs`
- `src/capability_circuit.rs`
- `src/capability_clusters.rs`
- `src/capability_filter.rs`
- `src/capability_indexer.rs`
- `src/capability_pack_orchestration/mod.rs`
- `src/capability_pack_orchestration/plan.rs`
- `src/capability_pack_orchestration/subgraph.rs`
- `src/capability_pack_orchestration/types.rs`
- `src/capability_pack_registry.rs`
- `src/capability_ranker.rs`
- `src/capability_registry.rs`
- `src/capability_retriever.rs`
- `src/capability_router.rs`
- `src/channel_adapter.rs`
- `src/circuit_breaker.rs`
- `src/cluster_router.rs`
- `src/code_verifier.rs`
- `src/cognitive_router.rs`
- `src/compact_session.rs`
- `src/confidence.rs`
- `src/consistency_check.rs`
- `src/constants.rs`
- `src/content_filter.rs`
- `src/context_budget.rs`
- `src/context_builder.rs`
- `src/context_contributor.rs`
- `src/context_engine.rs`
- `src/contracts.rs`
- `src/conversation_model.rs`
- `src/core_error.rs`
- `src/credential_service.rs`
- `src/cron_blueprint.rs`
- `src/cron_delivery.rs`
- `src/cron_persistence.rs`
- `src/dashboard_report.rs`
- `src/data_outcome.rs`
- `src/database_query_service.rs`
- `src/decision_action.rs`
- `src/delegation.rs`
- `src/dependency_metrics.rs`
- `src/dev_experience.rs`
- `src/device_sync.rs`
- `src/divergence.rs`
- `src/domain_ontology.rs`
- `src/domain_registry.rs`
- `src/domain_router.rs`
- `src/domain_semantics.rs`
- `src/dream.rs`
- `src/dto_locks.rs`
- `src/dual_registry.rs`
- `src/dynamic_tools.rs`
- `src/entity_extractor.rs`
- `src/error.rs`
- `src/error_classifier.rs`
- `src/error_codes.rs`
- `src/event_bus.rs`
- `src/event_names.rs`
- `src/execution_trace.rs`
- `src/feature_flag_provider.rs`
- `src/feedback_data_lake.rs`
- `src/fleet.rs`
- `src/gateway_operations.rs`
- `src/gateway_service.rs`
- `src/graph_dtos.rs`
- `src/hallucination_guard.rs`
- `src/has_provider_registry.rs`
- `src/holding_period.rs`
- `src/hook_service.rs`
- `src/i18n.rs`
- `src/indexer.rs`
- `src/indicators.rs`
- `src/inference_engine.rs`
- `src/insight_dashboard.rs`
- `src/interceptor.rs`
- `src/jobs_store.rs`
- `src/json_schema.rs`
- `src/kit_bridge.rs`
- `src/knowledge_graph.rs`
- `src/knowledge_vector.rs`
- `src/layered_prompt_engine.rs`
- `src/learning_graph.rs`
- `src/lib.rs`
- `src/llm_execution.rs`
- `src/llm_executor.rs`
- `src/louvain_dtos.rs`
- `src/lsp_integration.rs`
- `src/market_data.rs`
- `src/marketplace.rs`
- `src/mcp_service.rs`
- `src/mcp_types.rs`
- `src/memory.rs`
- `src/message_adapter.rs`
- `src/messaging_webhook.rs`
- `src/migration_types.rs`
- `src/moa_degradation.rs`
- `src/model_cascade.rs`
- `src/model_knowledge.rs`
- `src/multi_agent.rs`
- `src/narrative.rs`
- `src/news_archive.rs`
- `src/node_output_status.rs`
- `src/note_dtos.rs`
- `src/notification_channel.rs`
- `src/npm_registry.rs`
- `src/observability.rs`
- `src/opc_store.rs`
- `src/orchestration_dispatch.rs`
- `src/output_sanitizer.rs`
- `src/page_type.rs`
- `src/panel_variables.rs`
- `src/path_vars.rs`
- `src/persistence.rs`
- `src/persistence_mod.rs`
- `src/plan_compiler.rs`
- `src/plan_types.rs`
- `src/planner.rs`
- `src/platform_adapter.rs`
- `src/platform_callback.rs`
- `src/platform_config.rs`
- `src/platform_manager.rs`
- `src/plugin_hook.rs`
- `src/profile.rs`
- `src/prompt_guard.rs`
- `src/prompt_provider.rs`
- `src/provider.rs`
- `src/provider_continuation.rs`
- `src/quote_watcher.rs`
- `src/rag_config.rs`
- `src/rag_provider.rs`
- `src/rar_recaller.rs`
- `src/rar_router.rs`
- `src/rate_limiter.rs`
- `src/reflection_types.rs`
- `src/registry.rs`
- `src/reliability.rs`
- `src/replay_executor.rs`
- `src/repo_dtos.rs`
- `src/repositories.rs`
- `src/response_normalizer.rs`
- `src/retry_policy.rs`
- `src/reversible_effect.rs`
- `src/rhai_ast_cache.rs`
- `src/rhai_engine.rs`
- `src/risk.rs`
- `src/rl.rs`
- `src/route_bridge.rs`
- `src/route_engine.rs`
- `src/routing_path.rs`
- `src/runtime_types/capability_gap.rs`
- `src/runtime_types/compact.rs`
- `src/runtime_types/conversation.rs`
- `src/runtime_types/execution_progress.rs`
- `src/runtime_types/fork_bridge.rs`
- `src/runtime_types/hooks.rs`
- `src/runtime_types/mod.rs`
- `src/runtime_types/permission_enforcer.rs`
- `src/runtime_types/permissions.rs`
- `src/runtime_types/runtime_mutation.rs`
- `src/runtime_types/session.rs`
- `src/sandbox_policy.rs`
- `src/scanner.rs`
- `src/schema.rs`
- `src/seam_invoker.rs`
- `src/search_sources.rs`
- `src/self_improving_loop.rs`
- `src/serialization.rs`
- `src/service_registry.rs`
- `src/session_events.rs`
- `src/session_log_invariant.rs`
- `src/session_state.rs`
- `src/session_tracer.rs`
- `src/skill_enhancement.rs`
- `src/skill_evolution_hook.rs`
- `src/speech.rs`
- `src/ssrf_guard.rs`
- `src/stock_analysis.rs`
- `src/stock_market_types.rs`
- `src/stock_monitor.rs`
- `src/stock_service.rs`
- `src/storage_backend.rs`
- `src/strategy_contract.rs`
- `src/streaming.rs`
- `src/style.rs`
- `src/task_cancel.rs`
- `src/task_scene.rs`
- `src/task_shape.rs`
- `src/task_state.rs`
- `src/template_patch.rs`
- `src/terminal_enhancement.rs`
- `src/test_support.rs`
- `src/text_clean.rs`
- `src/tool.rs`
- `src/tool_access.rs`
- `src/tool_audit.rs`
- `src/tool_metrics.rs`
- `src/tool_permissions.rs`
- `src/tool_service.rs`
- `src/tool_validation.rs`
- `src/tools_ext.rs`
- `src/trading_calendar.rs`
- `src/trading_engine.rs`
- `src/training_pipeline.rs`
- `src/trajectory_scorer.rs`
- `src/trajectory_service.rs`
- `src/trajectory_store.rs`
- `src/trajectory_types.rs`
- `src/typed_event.rs`
- `src/types/conversation.rs`
- `src/types/gateway.rs`
- `src/types/mod.rs`
- `src/types/opc_demand.rs`
- `src/types/paper_reading.rs`
- `src/types/provider_model.rs`
- `src/types/rag_voice_etc.rs`
- `src/types/search.rs`
- `src/types/session_state.rs`
- `src/types/settings_chat.rs`
- `src/ui_tree.rs`
- `src/url_utils.rs`
- `src/usage_pricing.rs`
- `src/util_fns.rs`
- `src/vault.rs`
- `src/vector_search.rs`
- `src/vector_store.rs`
- `src/verdict_shape.rs`
- `src/verification_recipe.rs`
- `src/webhook_subscription.rs`
- `src/wiki_dtos.rs`
- `src/workflow_evolution.rs`
- `src/workflow_graph.rs`
- `src/workflow_hook_sink.rs`
- `src/workflow_io.rs`
- `src/workflow_lifecycle.rs`
- `src/workflow_node_descriptors.rs`
- `src/workflow_node_deserializer.rs`
- `src/workflow_node_dyn.rs`
- `src/workflow_optimization.rs`
- `src/workflow_port_axioms.rs`
- `src/workflow_reflection.rs`
- `src/workflow_run.rs`
- `src/workflow_template_repo.rs`
- `src/workflow_types.rs`
- `tests/serialization_contracts.rs`

### `axagent-plugin-testkit`

仓库：https://github.com/polite0803/AxInvest

包内文件：

- `.cargo_vcs_info.json`
- `Cargo.lock`
- `Cargo.toml`
- `Cargo.toml.orig`
- `src/lib.rs`

## 四、判读

- 三件套当前均满足“可组装包表面”：包内含 `Cargo.toml` 与 `src/lib.rs`。
- 当前 `publish` 仍为 `false`（或由 workspace 继承），因此**没有正式发布事实**。
- `plugin-testkit` 仍有指向 `axagent-plugin-proto` 的跨根 path 依赖；正式发布前必须改由 registry / 版本解析，或建立私有 registry。
- 本清单只记录发布前置，不替代 `cargo publish --dry-run`、签名/撤销通道或私有 registry 实测。
