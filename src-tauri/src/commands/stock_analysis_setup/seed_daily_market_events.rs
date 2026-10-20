// SPDX-License-Identifier: AGPL-3.0-only
//! G4 daily-market-events 工作流模板种子
//!
//! ## 模板用途
//!
//! 每日 18:00 自动触发，多源数据采集 + LLM 主题分类 + LLM 综合主线 + 持久化到
//! `market_mainlines` 表，对齐 DojoAgents 宣传场景 4「市场发现」。
//!
//! ## DAG 结构
//!
//! ```text
//! trigger (Schedule 18:00)
//!   → synthesize-mainlines (Agent 节点，LLM 综合输出主线 JSON)
//!   → persist-mainlines    (Tool 节点，确定性调 market_mainline_batch_upsert 落库)
//!   → end        ↑ const-mainline-archive (DataTransformer，产出 archive_missing 常量)
//! ```
//!
//! ⚠ **v1→v2 的改动就是「落库不再靠模型自愿调工具」**：v1 只有 trigger/agent/end 三个节点，
//! 持久化全靠提示词要求 Agent 自己调 `market_mainline_batch_upsert`，实测该调用近 8 次运行
//! 一次都没发生（`tool_calls_made` 全 NULL，表恒 0 行）——「幂等只保证对象存在，不保证行为发生」
//! 在这里的表现是：LLM 把契约 JSON 写在文本里，却没人把它写进库。
//! 故本模板把持久化升格为图上的确定性 Tool 节点，并从 Agent 的工具清单里摘掉该工具，
//! 保证 `market_mainlines` **只有一个写入口**。

use axagent_entities::workflow_template;
use sea_orm::{ActiveModelTrait, EntityTrait, Set};

use axagent_harness::workflow_types::{
    AgentNode, AgentNodeConfig, DataTransformerNode, DataTransformerNodeConfig, EdgeType, EndNode,
    EndNodeConfig, JsonSchema, JsonSchemaProperty, OutputMode, Position, RetryConfig,
    ScheduleTriggerConfig, ToolDef, ToolNode, ToolNodeConfig, TriggerConfig, TriggerNode,
    TriggerType, Variable, WorkflowEdge, WorkflowNode, WorkflowNodeBase,
};

const TEMPLATE_ID: &str = "daily-market-events";
/// v2：加确定性持久化节点（见文件头 DAG 说明）。
///
/// ⚠ 播种门是 `existing.version >= TEMPLATE_VERSION` 先 return —— **不抬版本就不重播种**，
/// 改动只留在代码里、库里仍是 v1 的三节点图（与 `debate_rounds` 那次同一个坑）。
const TEMPLATE_VERSION: i32 = 2;

/// 种子化 daily-market-events 工作流模板
pub async fn seed_daily_market_events_template(
    db: &sea_orm::DatabaseConnection,
) -> Result<(), String> {
    let now = chrono::Utc::now().timestamp_millis();

    // ── 触发器：每日 18:00（北京时间）──
    let schedule_cfg = ScheduleTriggerConfig {
        cron: "0 18 * * *".into(),
        schedules: None,
        timezone: "Asia/Shanghai".into(),
        enabled: true,
        input_params: Some(serde_json::json!({ "mainline_date": "{{today_cn}}" })),
    };
    let schedule_cfg_value = serde_json::to_value(&schedule_cfg)
        .map_err(|e| format!("序列化 ScheduleTriggerConfig 失败: {e}"))?;
    let trigger_config =
        TriggerConfig { trigger_type: TriggerType::Schedule, config: schedule_cfg_value.clone() };

    // ── Agent 可用工具：**只有** 4 个市场数据采集 MCP 工具 ──
    //
    // v2 起 `market_mainline_batch_upsert` **不在**本清单里：写 `market_mainlines` 的唯一入口
    // 是下面的 `persist-mainlines` Tool 节点。留着它等于同一逻辑两份实现（模型可手写一份入参
    // 落库、图上节点又落一份），而模型那一份没有契约校验 —— 实测它的产出是文本里的 JSON，
    // 工具调用一次都没发生，所以清单里挂着的是一条死路径。
    let agent_tools: Vec<ToolDef> = vec![
        ToolDef {
            name: "get_hot_stocks".into(),
            description: Some("获取热门股票列表（按关注度/资金流入排序）".into()),
            parameters: Some(JsonSchema {
                schema_type: "object".into(),
                description: None,
                properties: None,
                required: None,
                items: None,
            }),
        },
        ToolDef {
            name: "get_cls_flash".into(),
            description: Some("获取财联社最新电报快讯（A股市场实时新闻）".into()),
            parameters: Some(JsonSchema {
                schema_type: "object".into(),
                description: None,
                properties: Some({
                    let mut m = std::collections::HashMap::new();
                    m.insert(
                        "limit".into(),
                        JsonSchemaProperty {
                            schema_type: "integer".into(),
                            description: Some("返回条数（默认 30）".into()),
                            default: Some(serde_json::json!(30)),
                            enum_values: None,
                            format: None,
                        },
                    );
                    m
                }),
                required: None,
                items: None,
            }),
        },
        ToolDef {
            name: "get_market_dragon_tiger".into(),
            description: Some("获取龙虎榜数据（游资/机构席位明细）".into()),
            parameters: Some(JsonSchema {
                schema_type: "object".into(),
                description: None,
                properties: None,
                required: None,
                items: None,
            }),
        },
        ToolDef {
            name: "get_north_bound_flow".into(),
            description: Some("获取北向资金净流入数据（沪股通/深股通）".into()),
            parameters: Some(JsonSchema {
                schema_type: "object".into(),
                description: None,
                properties: None,
                required: None,
                items: None,
            }),
        },
    ];

    // ── 节点定义 ──
    let nodes: Vec<WorkflowNode> = vec![
        // 1. 触发器：每日 18:00 自动触发
        WorkflowNode::Trigger(TriggerNode {
            base: WorkflowNodeBase {
                id: "trigger".into(),
                title: "每日 18:00 定时触发".into(),
                description: Some("cron: 0 18 * * * (Asia/Shanghai)".into()),
                position: Position { x: 20.0, y: 20.0 },
                retry: RetryConfig::default(),
                timeout: None,
                enabled: true,
                parent_id: None,
                compensation: None,
                continue_on_fail: false,
            },
            config: trigger_config.clone(),
        }),
        // 2. Agent 节点：综合分析输出主线 JSON 数组
        WorkflowNode::Agent(AgentNode {
            base: WorkflowNodeBase {
                id: "synthesize-mainlines".into(),
                title: "LLM 综合市场主线".into(),
                description: Some(
                    "拉取热点股/快讯/龙虎榜/北向数据，分类主题，过滤噪音，综合 3-8 条主线".into(),
                ),
                position: Position { x: 20.0, y: 180.0 },
                retry: RetryConfig { enabled: true, max_retries: 1, ..Default::default() },
                timeout: Some(600), // 10 分钟超时
                enabled: true,
                parent_id: None,
                compensation: None,
                continue_on_fail: false,
            },
            config: AgentNodeConfig {
                system_prompt: r#"基于今日多源数据（热点股 / 财联社快讯 / 龙虎榜 / 北向资金），提炼 3-8 条市场主线。

请根据市场主线综合分析方法论完成任务，**只输出** JSON 结果：顶层字段 `mainline_date`（YYYY-MM-DD，取当日北京时间）与 `mainlines`（数组，每项至少含 `theme` 与 `narrative`）。
落库由下游的持久化节点负责，**不要**尝试调用工具写库，也不要在输出里夹带解释文字或代码块。"#.into(),
                context_sources: vec!["trigger".into()],
                input_mapping: std::collections::HashMap::new(),
                output_var: "mainlines-output".into(),
                model: None,
                temperature: Some(0.4),
                max_tokens: Some(8192),
                tools: agent_tools,
                exposed_tools: vec![],
                output_mode: OutputMode::Json,
                agent_profile_id: Some("stock-market-synthesizer".into()),
                max_tool_rounds: Some(8),
                execution_mode: Some("react".into()),
                rag_source_ids: vec![],
                consistency_check: None,
                hallucination_guard: None,
                fallback_model: None,
                task_scene: None,
                stream_chunk_timeout_secs: None,
            },
        }),
        // 3. 归档标记常量节点：产出 `mainline_archive_missing`
        //    （ToolNode 的 `input_mapping` 只认**变量路径**，不认字面量 —— 常量必须由节点产出，
        //     形态与 `horizon_tier_template.rs` 的 `const-scoring-horizon` 一致）
        WorkflowNode::DataTransformer(DataTransformerNode {
            base: WorkflowNodeBase {
                id: "const-mainline-archive".into(),
                title: "归档当日未提及主线".into(),
                description: Some(
                    "产出 mainline_archive_missing=true，供 persist-mainlines 的 archive_missing 取用".into(),
                ),
                position: Position { x: 300.0, y: 340.0 },
                retry: RetryConfig::default(),
                timeout: None,
                enabled: true,
                parent_id: None,
                compensation: None,
                continue_on_fail: false,
            },
            config: DataTransformerNodeConfig {
                input_var: String::new(),
                expression: "true".into(),
                output_var: "mainline_archive_missing".into(),
            },
        }),
        // 4. 持久化节点：**本模板唯一的 `market_mainlines` 写入口**
        //
        // 入参路径按 `<节点>.content.<字段>` 解析（`resolve_var_path` 会穿透 Agent 输出里
        // content 这段 JSON 字符串），与 `seed_serenity.rs` 的
        // `a-candidate-mapper.content.candidates` 同形态。
        //
        // ⚠ 三个入参**逐字写**、不用循环或 `format!` 拼：路径写错时 `resolve_var_path` 返回
        // `None` ⇒ 组装成 `null` ⇒ 工具侧 `BatchUpsertInput` 解析**显式报错**（不是静默写 0 行），
        // 配合 `continue_on_fail=false` 让整轮失败可见。这条链的失败必须响，不能又变成空表。
        WorkflowNode::Tool(ToolNode {
            base: WorkflowNodeBase {
                id: "persist-mainlines".into(),
                title: "落库市场主线".into(),
                description: Some(
                    "调用 market_mainline_batch_upsert 把 synthesize 的 JSON 写入 market_mainlines".into(),
                ),
                position: Position { x: 20.0, y: 340.0 },
                retry: RetryConfig { enabled: true, max_retries: 1, ..Default::default() },
                timeout: None,
                enabled: true,
                parent_id: None,
                compensation: None,
                continue_on_fail: false,
            },
            config: ToolNodeConfig {
                tool_name: "market_mainline_batch_upsert".into(),
                input_mapping: {
                    let mut m = std::collections::HashMap::new();
                    m.insert(
                        "mainline_date".to_string(),
                        "synthesize-mainlines.content.mainline_date".to_string(),
                    );
                    m.insert(
                        "mainlines".to_string(),
                        "synthesize-mainlines.content.mainlines".to_string(),
                    );
                    m.insert(
                        "archive_missing".to_string(),
                        "mainline_archive_missing".to_string(),
                    );
                    m
                },
                output_var: "persist-mainlines".into(),
            },
        }),
        // 5. 终止节点
        WorkflowNode::End(EndNode {
            base: WorkflowNodeBase {
                id: "end".into(),
                title: "结束".into(),
                description: None,
                position: Position { x: 20.0, y: 500.0 },
                retry: RetryConfig::default(),
                timeout: None,
                enabled: true,
                parent_id: None,
                compensation: None,
                continue_on_fail: false,
            },
            config: EndNodeConfig { output_var: Some("mainlines-output".into()) },
        }),
    ];

    let edges: Vec<WorkflowEdge> = vec![
        WorkflowEdge {
            id: "e-trigger-agent".into(),
            source: "trigger".into(),
            source_handle: None,
            target: "synthesize-mainlines".into(),
            target_handle: None,
            edge_type: EdgeType::Direct,
            label: None,
        },
        WorkflowEdge {
            id: "e-agent-persist".into(),
            source: "synthesize-mainlines".into(),
            source_handle: None,
            target: "persist-mainlines".into(),
            target_handle: None,
            edge_type: EdgeType::Direct,
            label: None,
        },
        // 常量节点供给归档标记（无此边则 `mainline_archive_missing` 不在变量池，
        // `archive_missing` 解析成 null ⇒ 工具按 false 处理，「归档当日未提及主线」静默失效）
        WorkflowEdge {
            id: "e-const-persist".into(),
            source: "const-mainline-archive".into(),
            source_handle: None,
            target: "persist-mainlines".into(),
            target_handle: None,
            edge_type: EdgeType::Direct,
            label: None,
        },
        WorkflowEdge {
            id: "e-persist-end".into(),
            source: "persist-mainlines".into(),
            source_handle: None,
            target: "end".into(),
            target_handle: None,
            edge_type: EdgeType::Direct,
            label: None,
        },
    ];

    // ── 模板变量（运行时由触发器填充）──
    let variables: Vec<Variable> = vec![Variable {
        name: "today_cn".into(),
        var_type: "string".into(),
        value: serde_json::json!("{{format_date_now \"yyyy-MM-dd\" \"Asia/Shanghai\"}}"),
        description: Some("当前北京时间日期 YYYY-MM-DD".into()),
        is_secret: false,
    }];

    // ── 版本检查与快照 ──
    let existing =
        workflow_template::Entity::find_by_id(TEMPLATE_ID).one(db).await.map_err(|e| {
            String::from(crate::commands::error::ErrorResponse::from_error(
                e,
                crate::commands::error::ErrorCategory::Unrecoverable,
            ))
        })?;
    if let Some(ref existing) = existing {
        if existing.version >= TEMPLATE_VERSION {
            tracing::info!(
                "[stock_analysis_setup] {TEMPLATE_ID} 模板已是最新 v{}，跳过种子化",
                existing.version
            );
            return Ok(());
        }
        let ver_id = format!("{TEMPLATE_ID}_v{}", existing.version);
        let ver_existing = axagent_entities::workflow_template_version::Entity::find_by_id(&ver_id)
            .one(db)
            .await
            .map_err(|e| {
                String::from(crate::commands::error::ErrorResponse::from_error(
                    e,
                    crate::commands::error::ErrorCategory::Unrecoverable,
                ))
            })?;
        if ver_existing.is_none() {
            let snapshot = axagent_entities::workflow_template_version::ActiveModel {
                id: Set(ver_id.clone()),
                template_id: Set(TEMPLATE_ID.into()),
                name: Set(existing.name.clone()),
                description: Set(existing.description.clone()),
                icon: Set(existing.icon.clone()),
                tags: Set(existing.tags.clone()),
                version: Set(existing.version),
                is_preset: Set(existing.is_preset),
                is_editable: Set(existing.is_editable),
                is_public: Set(existing.is_public),
                trigger_config: Set(existing.trigger_config.clone()),
                nodes: Set(existing.nodes.clone()),
                edges: Set(existing.edges.clone()),
                input_schema: Set(existing.input_schema.clone()),
                output_schema: Set(existing.output_schema.clone()),
                variables: Set(existing.variables.clone()),
                error_config: Set(existing.error_config.clone()),
                created_at: Set(now),
            };
            snapshot.insert(db).await.map_err(|e| {
                String::from(crate::commands::error::ErrorResponse::from_error(
                    e,
                    crate::commands::error::ErrorCategory::Unrecoverable,
                ))
            })?;
            tracing::info!("[stock_analysis_setup] {TEMPLATE_ID} 模板旧版本快照已保存: {ver_id}");
        }
    }

    let nodes_json = serde_json::to_string(&nodes).map_err(|e| format!("序列化节点失败: {e}"))?;
    let edges_json = serde_json::to_string(&edges).map_err(|e| format!("序列化边失败: {e}"))?;
    let variables_json =
        serde_json::to_string(&variables).map_err(|e| format!("序列化变量失败: {e}"))?;
    let trigger_config_json =
        serde_json::to_string(&trigger_config).map_err(|e| format!("序列化触发器配置失败: {e}"))?;
    let tags =
        serde_json::to_string(&["市场主线".to_string(), "每日定时".to_string(), "G4".to_string()])
            .map_err(|e| format!("序列化标签失败: {e}"))?;

    // 先删再插
    let _ = workflow_template::Entity::delete_by_id(TEMPLATE_ID).exec(db).await;

    // P0 软门禁（C1，2026-09-14）：种子的端口公理 —— 结构性死链在此被记录（不阻断启动）。
    // 判据复用 harness 的 `warn_port_axioms_json`，不在本文件另写一份。
    axagent_harness::workflow_port_axioms::warn_port_axioms_json(
        &format!("stock_analysis_setup:seed_daily_market_events:{TEMPLATE_ID}"),
        &nodes_json,
        &edges_json,
    );

    workflow_template::ActiveModel {
        hooks_config: Set(None),
        id: Set(TEMPLATE_ID.into()),
        cluster_id: Set(Some("market".to_string())),
        route_path: Set(Some("/finance/market/mainlines".to_string())),
        name: Set("每日市场主线提炼".into()),
        description: Set(Some(
            "每日 18:00 自动采集多源市场数据，LLM 综合提炼 3-8 条市场主线（含主题/叙述/代表标的/强度评分/持续性），持久化到 market_mainlines 表".into(),
        )),
        icon: Set("📅".into()),
        tags: Set(Some(tags)),
        version: Set(TEMPLATE_VERSION),
        is_preset: Set(true),
        is_editable: Set(true),
        is_public: Set(true),
        trigger_config: Set(Some(trigger_config_json)),
        nodes: Set(nodes_json),
        edges: Set(edges_json),
        input_schema: Set(None),
        output_schema: Set(None),
        variables: Set(Some(variables_json)),
        error_config: Set(None),
        composite_source: Set(None),
        tool_defs: Set(None),
        mission_hash: Set(None),
        created_at: Set(now),
        updated_at: Set(now),
    }
    .insert(db)
    .await
    .map_err(|e| format!("写入 {TEMPLATE_ID} 模板失败: {e}"))?;

    tracing::info!(
        "[stock_analysis_setup] G4 daily-market-events 模板已创建 (v{TEMPLATE_VERSION})"
    );
    Ok(())
}
