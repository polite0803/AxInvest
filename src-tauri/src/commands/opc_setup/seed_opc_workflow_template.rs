// SPDX-License-Identifier: AGPL-3.0-only

//! OPC 需求发现工作流模板（v5）— 持久化到 workflow_template 表
//!
//! v5（2026-09-09）：**回到 docs/PLAN-demand-discovery.md 原方案的 workflow 形态**，
//! 能力驱动（capability → 检索词 → 扫描），替代 v4 的手填关键词 + 死工具节点
//! （RedditScanner / XianyuScanner / DemandValueEvaluator 从未注册进
//! ToolRegistry，v4 模板的 Tool 节点在运行期必然报"工具未注册"）。
//!
//! ```text
//! trigger ──► t-capability ──► a-keywords ──► c-keywords ──► loop-scan ──► end
//!          (CapabilityBrowse)  (Agent 检索词   (rhai 清洗     (ForEach 逐词)
//!           能力集扫描          规划，焦点)     + 兜底词表)        │
//!                                                            t-discover
//!                                                     (OpcDiscoverLeads
//!                                                       扫描→评估→入库)
//! ```
//!
//! 与原方案 §3 节点映射的对应：
//! - 能力集扫描（Tool）= `t-capability`，复用渐进式披露导航工具 `CapabilityBrowse`
//! - 需求提炼（Agent 焦点）= `a-keywords`，能力清单 → 平台检索词
//! - 平台需求发现（Tool）= `loop-scan` + `t-discover`，复用与手动扫描/订阅定时
//!   扫描完全同一条管线 `run_discovery_scan`（去重窗口/限流/合规跳过全继承）
//!
//! 不重复造的部分（AGENTS.md 禁区 12）：
//! - 扫描/评估/入库管线 = `axagent_tools::tools::opc_demand_scan`（单一权威来源）
//! - 定时触发 = 既有 `opc_demand_scan` CronJob（订阅词表机制），本模板保持 Manual
//! - 人工确认 = 线索页状态机（new→evaluated→contacted→won/lost），不设死分支
//!
//! Rhai 脚本 `demand-keywords.rhai` 编译期 include_str! 嵌入，改脚本必递增
//! TEMPLATE_VERSION（否则 DB 不重种子，修复静默失效）。

use axagent_entities::workflow_template;
use axagent_harness::workflow_types::{
    AgentNode, AgentNodeConfig, BackoffType, CodeNode, CodeNodeConfig, EdgeType, EndNode,
    EndNodeConfig, JsonSchema, JsonSchemaProperty, LoopNode, LoopNodeConfig, LoopType, OutputMode,
    Position, RetryConfig, ToolNode, ToolNodeConfig, TriggerConfig, TriggerNode, TriggerType,
    WorkflowEdge, WorkflowNode, WorkflowNodeBase,
};
use sea_orm::{ActiveModelTrait, DatabaseConnection, EntityTrait, Set};

const TEMPLATE_ID: &str = "opc-demand-discovery";

// v5（2026-09-09）：回到原方案 workflow 形态 —— 能力驱动（CapabilityBrowse +
// Agent 检索词规划 + Loop 逐词调用 OpcDiscoverLeads）；v4 的死工具节点全部移除。
const TEMPLATE_VERSION: i32 = 6;

/// 种子化 OPC 需求发现工作流模板到数据库
pub(crate) async fn seed_opc_workflow_template(db: &DatabaseConnection) -> Result<(), String> {
    let existing = workflow_template::Entity::find_by_id(TEMPLATE_ID)
        .one(db)
        .await
        .map_err(|e| format!("查询工作流模板失败: {e}"))?;

    if let Some(existing) = existing {
        if existing.version >= TEMPLATE_VERSION {
            tracing::info!("[opc_setup] 模板已是最新版本 v{}，跳过种子化", existing.version);
            return Ok(());
        }
        // 写版本快照（对齐 seed_stock_analysis 的 snapshot 机制）
        let ver_id = format!("{}_v{}", TEMPLATE_ID, existing.version);
        if axagent_entities::workflow_template_version::Entity::find_by_id(&ver_id)
            .one(db)
            .await
            .map_err(|e| format!("查询版本快照失败: {e}"))?
            .is_none()
        {
            let snapshot = axagent_entities::workflow_template_version::ActiveModel {
                id: Set(ver_id.clone()),
                template_id: Set(TEMPLATE_ID.to_string()),
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
                created_at: Set(chrono::Utc::now().timestamp_millis()),
            };
            snapshot.insert(db).await.map_err(|e| format!("写入版本快照失败: {e}"))?;
            tracing::info!("[opc_setup] 旧版本快照已保存: {ver_id}");
        }
        tracing::info!(
            "[opc_setup] 更新需求发现工作流模板 v{} → v{TEMPLATE_VERSION}",
            existing.version
        );
    }

    let now = chrono::Utc::now().timestamp_millis();
    let kw_code = include_str!("demand-keywords.rhai").to_string();

    let base = |id: &str, title: &str, desc: &str, x: f64, y: f64| -> WorkflowNodeBase {
        WorkflowNodeBase {
            id: id.into(),
            title: title.into(),
            description: Some(desc.into()),
            position: Position { x, y },
            retry: RetryConfig::default(),
            timeout: None,
            enabled: true,
            parent_id: None,
            compensation: None,
            continue_on_fail: false,
        }
    };

    let edge = |source: &str, target: &str| -> WorkflowEdge {
        WorkflowEdge {
            id: format!("e-{source}-{target}"),
            source: source.into(),
            source_handle: None,
            target: target.into(),
            target_handle: None,
            edge_type: EdgeType::Direct,
            label: None,
        }
    };

    let mut nodes: Vec<WorkflowNode> = Vec::new();
    let mut edges: Vec<WorkflowEdge> = Vec::new();

    // ── Trigger（Manual；定时触发走既有 opc_demand_scan 订阅 CronJob，不重复建）──
    nodes.push(WorkflowNode::Trigger(TriggerNode {
        base: base("trigger", "开始需求发现", "手动触发能力驱动的需求发现", 0.0, 0.0),
        config: TriggerConfig {
            trigger_type: TriggerType::Manual,
            config: serde_json::json!({"description": "能力驱动需求发现"}),
        },
    }));

    // ── t-capability：能力集扫描（原方案 §4，复用能力树导航工具）──
    // 不传 path → 返回全部能力域列表。continue_on_fail=true：能力索引未初始化时
    // 降级继续（Agent 收不到清单会输出泛化检索词，c-keywords 还有默认词表兜底）。
    nodes.push(WorkflowNode::Tool(ToolNode {
        base: WorkflowNodeBase {
            continue_on_fail: true,
            ..base(
                "t-capability",
                "能力集扫描",
                "浏览能力树全部能力域（渐进式披露 L0 导航层）",
                240.0,
                0.0,
            )
        },
        config: ToolNodeConfig {
            tool_name: "CapabilityBrowse".into(),
            input_mapping: std::collections::HashMap::new(),
            output_var: "capability_inventory".into(),
        },
    }));

    // ── a-keywords：需求提炼（原方案 §5，Agent 焦点）──
    // 能力清单 → 平台检索词。能力越具体词越具体；focus 变量注入焦点方向。
    nodes.push(WorkflowNode::Agent(AgentNode {
        base: WorkflowNodeBase {
            retry: RetryConfig {
                enabled: true,
                max_retries: 2,
                base_delay_ms: 3000,
                max_delay_ms: 30000,
                backoff_type: BackoffType::Exponential,
            },
            continue_on_fail: true,
            ..base("a-keywords", "检索词规划", "根据能力清单规划平台检索词（原方案需求提炼焦点）", 480.0, 0.0)
        },
        config: AgentNodeConfig {
            system_prompt: "你是 OPC 需求发现的检索词规划师。\n\n\
                任务：根据下方能力清单，规划一批用于在自由职业/众包平台搜索需求线索的检索词。\n\n\
                策略：\n\
                1. 优先选择与能力清单中「可交付、有竞争力」方向匹配的词——能力越具体，词越具体；\n\
                2. 覆盖能力清单中的主要能力域，不要全部集中在单一方向；\n\
                3. 用买家会用的需求侧词汇（如\"开发\"\"代做\"\"自动化脚本\"），不要用供给侧内部术语；\n\
                4. 每个词 2-10 个字，适合平台搜索框；\n\
                5. 若【focus】非空，至少一半检索词围绕该焦点展开。\n\n\
                输出格式（严格遵守）：只输出一个 JSON 字符串数组，不要 markdown 代码块、不要解释：\n\
                [\"检索词1\",\"检索词2\"]"
                .into(),
            context_sources: vec![],
            input_mapping: [
                ("capability_inventory".to_string(), "t-capability.result".to_string()),
                ("focus".to_string(), "focus".to_string()),
            ]
            .into_iter()
            .collect(),
            output_var: "keyword_plan".into(),
            model: None,
            temperature: Some(0.3),
            max_tokens: Some(4096),
            tools: vec![],
            exposed_tools: vec![],
            output_mode: OutputMode::Text,
            agent_profile_id: None,
            max_tool_rounds: None,
            execution_mode: None,
            rag_source_ids: vec![],
            model_role: None,
            consistency_check: None,
            hallucination_guard: None,
            fallback_model: None,
            task_scene: None,
            stream_chunk_timeout_secs: Some(300),
        },
    }));

    // ── c-keywords：检索词确定性清洗（json_parse + 去重 + 截断 + 默认词表兜底）──
    nodes.push(WorkflowNode::Code(CodeNode {
        base: WorkflowNodeBase {
            timeout: Some(30),
            continue_on_fail: true,
            ..base(
                "c-keywords",
                "检索词清洗",
                "解析 LLM 输出为关键词数组，去重截断并兜底默认词表",
                720.0,
                0.0,
            )
        },
        config: CodeNodeConfig {
            language: "rhai".into(),
            code: kw_code,
            output_var: "scan_keywords".into(),
            tool_name: None,
            execute_directly: true,
            input_mapping: [
                ("keyword_plan".to_string(), "a-keywords.content".to_string()),
                ("max_keywords".to_string(), "max_keywords".to_string()),
            ]
            .into_iter()
            .collect(),
        },
    }));

    // ── t-discover：平台需求发现（Loop 体，单一工具复用扫描管线）──
    // input_mapping 单键 "scan_keyword" → iteratee_var 注入的当前关键词。
    nodes.push(WorkflowNode::Tool(ToolNode {
        base: WorkflowNodeBase {
            retry: RetryConfig { enabled: true, max_retries: 1, ..Default::default() },
            continue_on_fail: true,
            ..base(
                "t-discover",
                "平台扫描入库",
                "OpcDiscoverLeads：按关键词扫描全部启用平台，评估并去重入库",
                1040.0,
                200.0,
            )
        },
        config: ToolNodeConfig {
            tool_name: "OpcDiscoverLeads".into(),
            input_mapping: [("query".to_string(), "scan_keyword".to_string())]
                .into_iter()
                .collect(),
            output_var: "discover_result".into(),
        },
    }));

    // ── loop-scan：逐检索词扫描（ForEach）──
    // iter_input_var 用点路径直取 c-keywords 输出内的数组（Loop 执行器已支持
    // resolve_var_path 点路径解析）。continue_on_error：单关键词失败不影响其余词。
    nodes.push(WorkflowNode::Loop(LoopNode {
        base: WorkflowNodeBase {
            timeout: Some(3600),
            ..base(
                "loop-scan",
                "逐词扫描",
                "对每个检索词调用 OpcDiscoverLeads 扫描评估入库",
                960.0,
                0.0,
            )
        },
        config: LoopNodeConfig {
            loop_type: LoopType::ForEach,
            items_var: None,
            iter_input_var: Some("c-keywords.result".into()),
            iteratee_var: Some("scan_keyword".into()),
            iter_output_var: Some("scan_results".into()),
            partial_result_var: Some("scan_results__partial".into()),
            // 硬上限与 max_keywords 钳制（1..=10）双保险，防 LLM 失控放大请求量
            max_iterations: Some(10),
            continue_condition: None,
            continue_on_error: true,
            body_steps: vec!["t-discover".into()],
            sub_graph: None,
            interrupt_after_each: false,
            interrupt_nodes: vec![],
        },
    }));

    // ── End ──
    nodes.push(WorkflowNode::End(EndNode {
        base: base("end", "完成", "需求发现完成，线索已入库供人工筛选", 1200.0, 0.0),
        config: EndNodeConfig { output_var: None },
    }));

    // ── 边：主链 DAG；Loop 体节点由 body_steps 驱动，不参与主 DAG ──
    edges.push(edge("trigger", "t-capability"));
    edges.push(edge("t-capability", "a-keywords"));
    edges.push(edge("a-keywords", "c-keywords"));
    edges.push(edge("c-keywords", "loop-scan"));
    edges.push(edge("loop-scan", "end"));

    // ── 序列化 ──
    let nodes_json = serde_json::to_string(&nodes).map_err(|e| format!("序列化节点失败: {e}"))?;
    let edges_json = serde_json::to_string(&edges).map_err(|e| format!("序列化边失败: {e}"))?;
    let tags = serde_json::to_string(&vec![
        "opc".to_string(),
        "demand-discovery".to_string(),
        "preset".to_string(),
    ])
    .unwrap_or_default();

    // 输入 Schema：focus / max_keywords
    let input_schema_val = {
        let mut props = std::collections::HashMap::new();
        props.insert(
            "focus".into(),
            JsonSchemaProperty {
                schema_type: "string".into(),
                description: Some("补充焦点方向（可选）：给出后至少一半检索词围绕它展开".into()),
                default: Some(serde_json::json!("")),
                enum_values: None,
                format: None,
            },
        );
        props.insert(
            "max_keywords".into(),
            JsonSchemaProperty {
                schema_type: "number".into(),
                description: Some("每轮生成的检索词数量上限（1-10，默认 5）".into()),
                default: Some(serde_json::json!(5)),
                enum_values: None,
                format: None,
            },
        );
        let schema = JsonSchema {
            schema_type: "object".into(),
            description: Some("能力驱动需求发现输入".into()),
            properties: Some(props),
            required: None,
            items: None,
        };
        serde_json::to_string(&schema).unwrap_or_default()
    };

    // 输出 Schema：扫描摘要（Loop 聚合结果在 scan_results）
    let output_schema_val = {
        let mut props = std::collections::HashMap::new();
        props.insert(
            "scan_keywords".into(),
            JsonSchemaProperty {
                schema_type: "array".into(),
                description: Some("本轮实际使用的检索词列表".into()),
                default: None,
                enum_values: None,
                format: None,
            },
        );
        props.insert(
            "scan_results".into(),
            JsonSchemaProperty {
                schema_type: "array".into(),
                description: Some("逐词扫描摘要（含 scanned/saved/high_value_count）".into()),
                default: None,
                enum_values: None,
                format: None,
            },
        );
        let schema = JsonSchema {
            schema_type: "object".into(),
            description: Some("需求发现输出".into()),
            properties: Some(props),
            required: None,
            items: None,
        };
        serde_json::to_string(&schema).unwrap_or_default()
    };

    // 模板变量（前端配置面板渲染；运行时经 RunOptions.variables 注入）
    let variables_val = serde_json::to_string(&vec![
        serde_json::json!({
            "name": "focus",
            "description": "补充焦点方向（可选）：给出后至少一半检索词围绕它展开",
            "value": "",
            "type": "string",
        }),
        serde_json::json!({
            "name": "max_keywords",
            "description": "每轮生成的检索词数量上限（1-10）",
            "value": 5,
            "type": "number",
        }),
    ])
    .unwrap_or_default();

    let error_config_val = serde_json::json!({
        "on_error": "continue",
        "max_retries": 1,
        "fallback_to_previous": false,
    })
    .to_string();

    // 先删再插（幂等；版本比对只决定是否重写）
    let _ = workflow_template::Entity::delete_by_id(TEMPLATE_ID).exec(db).await;

    // P0 软门禁（C1，2026-09-14）：种子的端口公理 —— 结构性死链在此被记录（不阻断启动）。
    // 判据复用 harness 的 `warn_port_axioms_json`，不在本文件另写一份。
    axagent_harness::workflow_port_axioms::warn_port_axioms_json(
        &format!("opc-setup:seed_opc_workflow_template:{TEMPLATE_ID}"),
        &nodes_json,
        &edges_json,
    );

    workflow_template::ActiveModel {
        hooks_config: Set(None),
        id: Set(TEMPLATE_ID.to_string()),
        cluster_id: Set(Some("opc".to_string())),
        route_path: Set(Some("/automation/opc/demand-discovery".to_string())),
        name: Set("OPC 需求发现（能力驱动）".to_string()),
        description: Set(Some(
            "原方案 workflow 形态：能力集扫描（CapabilityBrowse）→ Agent 规划检索词 \
             → Rhai 清洗兜底 → Loop 逐词扫描评估入库（OpcDiscoverLeads，与手动扫描/订阅同管线）\
             → 线索页人工筛选"
                .to_string(),
        )),
        icon: Set("lightbulb".into()),
        tags: Set(Some(tags)),
        version: Set(TEMPLATE_VERSION),
        is_preset: Set(true),
        is_editable: Set(true),
        is_public: Set(true),
        trigger_config: Set(Some(
            serde_json::to_string(&TriggerConfig {
                trigger_type: TriggerType::Manual,
                config: serde_json::json!({"description": "能力驱动需求发现"}),
            })
            .unwrap_or_default(),
        )),
        nodes: Set(nodes_json),
        edges: Set(edges_json),
        input_schema: Set(Some(input_schema_val)),
        output_schema: Set(Some(output_schema_val)),
        variables: Set(Some(variables_val)),
        error_config: Set(Some(error_config_val)),
        composite_source: Set(None),
        // v5 起不再预置 ToolDef 白名单：Agent 节点不携带工具，扫描经 ToolNode 执行
        tool_defs: Set(None),
        mission_hash: Set(None),
        created_at: Set(now),
        updated_at: Set(now),
    }
    .insert(db)
    .await
    .map_err(|e| format!("写入工作流模板失败: {e}"))?;

    tracing::info!("[opc_setup] 需求发现工作流模板已种子化 ({TEMPLATE_ID} v{TEMPLATE_VERSION})");
    Ok(())
}
