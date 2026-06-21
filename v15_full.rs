//! 股票分析专家/角色/Profile 自动种子化到 agency_experts/agent_roles/agent_profiles 表。
//! 使用 include_str! 编译期嵌入 .md 内容，打包后无需文件 I/O。

use axagent_core::repo;

/// 编译期嵌入的专家提示词（include_str 确保打包后可用）
const EMBEDDED_PROMPTS: &[(&str, &str)] = &[
    (
        "market-analyst",
        include_str!("../../agency_experts/stock-analysis/market-analyst.md"),
    ),
    (
        "sentiment-analyst",
        include_str!("../../agency_experts/stock-analysis/sentiment-analyst.md"),
    ),
    (
        "news-analyst",
        include_str!("../../agency_experts/stock-analysis/news-analyst.md"),
    ),
    (
        "fundamentals-analyst",
        include_str!("../../agency_experts/stock-analysis/fundamentals-analyst.md"),
    ),
    (
        "policy-analyst",
        include_str!("../../agency_experts/stock-analysis/policy-analyst.md"),
    ),
    (
        "hot-money-tracker",
        include_str!("../../agency_experts/stock-analysis/hot-money-tracker.md"),
    ),
    (
        "lockup-watcher",
        include_str!("../../agency_experts/stock-analysis/lockup-watcher.md"),
    ),
    (
        "research-analyst",
        include_str!("../../agency_experts/stock-analysis/research-analyst.md"),
    ),
    (
        "sector-analyst",
        include_str!("../../agency_experts/stock-analysis/sector-analyst.md"),
    ),
    (
        "bull-researcher",
        include_str!("../../agency_experts/stock-analysis/bull-researcher.md"),
    ),
    (
        "bear-researcher",
        include_str!("../../agency_experts/stock-analysis/bear-researcher.md"),
    ),
    ("bull-r2", include_str!("../../agency_experts/stock-analysis/bull-r2.md")),
    ("bear-r2", include_str!("../../agency_experts/stock-analysis/bear-r2.md")),
    (
        "aggressive-debator",
        include_str!("../../agency_experts/stock-analysis/aggressive-debator.md"),
    ),
    (
        "conservative-debator",
        include_str!("../../agency_experts/stock-analysis/conservative-debator.md"),
    ),
    (
        "neutral-debator",
        include_str!("../../agency_experts/stock-analysis/neutral-debator.md"),
    ),
    (
        "research-manager",
        include_str!("../../agency_experts/stock-analysis/research-manager.md"),
    ),
    ("trader", include_str!("../../agency_experts/stock-analysis/trader.md")),
    (
        "value-investor",
        include_str!("../../agency_experts/stock-analysis/custom/value-investor.md"),
    ),
    (
        "data-quality-inspector",
        include_str!("../../agency_experts/stock-analysis/data-quality-inspector.md"),
    ),
    (
        "rule-checker",
        include_str!("../../agency_experts/stock-analysis/rule-checker.md"),
    ),
    (
        "catalyst-analyst",
        include_str!("../../agency_experts/stock-analysis/catalyst-analyst.md"),
    ),
    (
        "debate-convergence",
        include_str!("../../agency_experts/stock-analysis/debate-convergence.md"),
    ),
    ("reflection", include_str!("../../agency_experts/stock-analysis/reflection.md")),
    // ── Serenity 瓶颈分析 4 专家 ──
    (
        "trend-scanner",
        include_str!("../../agency_experts/stock-analysis/trend-scanner.md"),
    ),
    (
        "chain-decomposer",
        include_str!("../../agency_experts/stock-analysis/chain-decomposer.md"),
    ),
    (
        "chokepoint-identifier",
        include_str!("../../agency_experts/stock-analysis/chokepoint-identifier.md"),
    ),
    (
        "candidate-mapper",
        include_str!("../../agency_experts/stock-analysis/candidate-mapper.md"),
    ),
];

const EXPERT_ROLE_MAP: &[(&str, &str)] = &[
    ("market-analyst", "stock-analyst"),
    ("sentiment-analyst", "stock-analyst"),
    ("news-analyst", "stock-analyst"),
    ("fundamentals-analyst", "stock-analyst"),
    ("policy-analyst", "stock-analyst"),
    ("hot-money-tracker", "stock-analyst"),
    ("lockup-watcher", "stock-analyst"),
    ("research-analyst", "stock-analyst"),
    ("sector-analyst", "stock-analyst"),
    ("bull-researcher", "debater"),
    ("bear-researcher", "debater"),
    ("bull-r2", "debater"),
    ("bear-r2", "debater"),
    ("aggressive-debator", "risk-evaluator"),
    ("conservative-debator", "risk-evaluator"),
    ("neutral-debator", "risk-evaluator"),
    ("research-manager", "decision-maker"),
    ("trader", "trader"),
    ("value-investor", "stock-analyst"),
    ("data-quality-inspector", "stock-analyst"),
    ("rule-checker", "risk-evaluator"),
    ("catalyst-analyst", "stock-analyst"),
    ("debate-convergence", "debater"),
    ("reflection", "decision-maker"),
    // ── Serenity 瓶颈分析师 ──
    ("trend-scanner", "stock-analyst"),
    ("chain-decomposer", "stock-analyst"),
    ("chokepoint-identifier", "stock-analyst"),
    ("candidate-mapper", "stock-analyst"),
];

struct StockRoleDef {
    id: &'static str,
    name: &'static str,
    description: &'static str,
    system_prompt: &'static str,
    max_concurrent: i32,
    timeout_seconds: i64,
}

const STOCK_ROLES: &[StockRoleDef] = &[
    StockRoleDef {
        id: "stock-analyst",
        name: "股票分析师",
        description: "A股多维分析",
        system_prompt: "你是专业的 A 股分析师，基于行情数据、财务数据、新闻资讯等对股票进行深度分析。",
        // 修复 Defect #6: 提升到 12 以容纳 9 个 a-* + value-investor + data-quality-inspector + catalyst-analyst
        // （共 12 个 stock-analyst 角色节点），留 1 槽位余量。
        max_concurrent: 13,
        timeout_seconds: 600,
    },
    StockRoleDef {
        id: "debater",
        name: "辩论研究员",
        description: "多空辩论",
        system_prompt: "你是投资辩论研究员，从多/空角度审视分析结论。",
        max_concurrent: 2,
        timeout_seconds: 300,
    },
    StockRoleDef {
        id: "risk-evaluator",
        name: "风险评估师",
        description: "风险评估",
        system_prompt: "你是风险评估师，识别投资中的各类风险并量化评估。",
        max_concurrent: 4,
        timeout_seconds: 300,
    },
    StockRoleDef {
        id: "trader",
        name: "交易员",
        description: "制定交易执行方案",
        system_prompt: "你是 A 股交易员，制定具体入场/出场/仓位方案，遵守 T+1、涨跌停规则。",
        max_concurrent: 1,
        timeout_seconds: 300,
    },
    StockRoleDef {
        id: "decision-maker",
        name: "决策者",
        description: "最终投资决策",
        system_prompt: "你是投资决策者，综合所有分析结果做出最终决策。",
        max_concurrent: 1,
        timeout_seconds: 300,
    },
];

/// Profile → 工具映射（模块级，模板 seed 和 agent_profiles seed 共用）
static PROFILE_TOOLS: &[(&str, &[&str])] = &[
    (
        "market-analyst",
        &[
            "get_stock_kline",
            "get_stock_quote",
            "compute_scoring",
            "compute_kdj",
            "compute_obv",
            "search_stock",
        ],
    ),
    (
        "sentiment-analyst",
        &[
            "get_stock_news",
            "get_stock_money_flow",
            "get_hot_stocks",
            "get_stock_option_pcr",
            "search_stock",
        ],
    ),
    (
        "news-analyst",
        &[
            "get_stock_news",
            "get_announcements",
            "get_cls_flash",
            "get_stock_option_pcr",
            "search_stock",
        ],
    ),
    (
        "fundamentals-analyst",
        &[
            "get_stock_financials",
            "compute_valuation",
            "get_consensus_eps",
            "get_institutional_visits",
            "get_stock_peers",
            "search_stock",
        ],
    ),
    ("policy-analyst", &["get_stock_news", "get_announcements", "search_stock"]),
    (
        "hot-money-tracker",
        &[
            "get_stock_money_flow",
            "get_hot_stocks",
            "get_north_bound_flow",
            "get_market_dragon_tiger",
            "get_block_trades",
            "search_stock",
        ],
    ),
    (
        "lockup-watcher",
        &[
            "get_stock_lockup",
            "get_stock_shareholder_trades",
            "get_stock_margin_data",
            "get_announcements",
            "get_block_trades",
            "search_stock",
        ],
    ),
    (
        "research-analyst",
        &[
            "get_consensus_eps",
            "get_stock_financials",
            "get_stock_news",
            "get_research_reports",
            "get_institutional_visits",
            "search_stock",
        ],
    ),
    (
        "sector-analyst",
        &[
            "get_industry_ranking",
            "get_hot_stocks",
            "get_stock_quote",
            "get_concept_blocks",
            "get_stock_peers",
            "search_stock",
        ],
    ),
    ("bull-researcher", &["compute_scoring", "compute_valuation", "search_stock"]),
    ("bear-researcher", &["compute_scoring", "compute_valuation", "search_stock"]),
    // v16: R2 质询型辩手也需要 compute_scoring / compute_valuation 来核实对方论据中的
    // 技术评分与估值结论，否则质询问题缺乏数据支撑，容易产出空泛内容。
    ("bull-r2", &["compute_scoring", "compute_valuation", "search_stock"]),
    ("bear-r2", &["compute_scoring", "compute_valuation", "search_stock"]),
    ("aggressive-debator", &["compute_portfolio_risk", "search_stock"]),
    ("conservative-debator", &["compute_portfolio_risk", "search_stock"]),
    ("neutral-debator", &["compute_portfolio_risk", "search_stock"]),
    (
        "research-manager",
        &[
            "compute_scoring",
            "compute_valuation",
            "compute_portfolio_risk",
            "search_stock",
        ],
    ),
    ("trader", &["get_stock_quote", "compute_scoring", "search_stock"]),
    (
        "value-investor",
        &[
            "get_stock_financials",
            "compute_valuation",
            "get_consensus_eps",
            "get_institutional_visits",
            "get_stock_peers",
            "search_stock",
        ],
    ),
    // ── P3 (real-nodes): 数据质量检查员 + 规则检查员 ──
    // data-quality-inspector 只需阅读上游分析师报告（context_sources 注入），
    // 不需要外部工具调用
    ("data-quality-inspector", &["search_stock"]),
    // rule-checker 需要读取技术指标与估值/风控结果
    (
        "rule-checker",
        &[
            "compute_scoring",
            "compute_valuation",
            "compute_portfolio_risk",
            "search_stock",
        ],
    ),
    // ── Catalyst & Narrative Analyst ──
    // 需要读取新闻/公告做催化剂判断 + K线/量价做机构行为分析
    (
        "catalyst-analyst",
        &[
            "get_stock_news",
            "get_announcements",
            "get_concept_blocks",
            "get_stock_peers",
            "get_stock_kline",
            "get_stock_quote",
            "search_stock",
        ],
    ),
    // ── Serenity 瓶颈分析 4 专家工具映射 ──
    // trend-scanner: 扫描宏观数据发现产业趋势，需全天候监控类工具
    (
        "trend-scanner",
        &[
            "get_hot_stocks",
            "get_industry_ranking",
            "get_cls_flash",
            "get_concept_blocks",
            "get_north_bound_flow",
            "get_market_dragon_tiger",
            "search_stock",
        ],
    ),
    // chain-decomposer: 拆解产业链，需行业/概念/同业数据
    (
        "chain-decomposer",
        &[
            "get_concept_blocks",
            "get_stock_peers",
            "get_stock_news",
            "get_industry_ranking",
            "search_stock",
        ],
    ),
    // chokepoint-identifier: 验证瓶颈假设，需财务/研报数据
    (
        "chokepoint-identifier",
        &[
            "get_stock_financials",
            "get_research_reports",
            "get_consensus_eps",
            "get_stock_peers",
            "get_stock_news",
            "search_stock",
        ],
    ),
    // candidate-mapper: 映射候选公司，需财务/估值/调研数据
    (
        "candidate-mapper",
        &[
            "get_stock_financials",
            "get_stock_quote",
            "compute_valuation",
            "get_institutional_visits",
            "get_research_reports",
            "get_stock_news",
            "search_stock",
        ],
    ),
];

pub async fn ensure_stock_analysis_experts_seeded(
    db: &sea_orm::DatabaseConnection,
) -> Result<(), String> {
    seed_agency_experts(db).await?;
    seed_agent_roles(db).await?;
    seed_agent_profiles(db).await?;
    seed_stock_analysis_workflow_template(db).await?;
    seed_reflection_workflow_template(db).await?;
    seed_serenity_screening_workflow_template(db).await?;
    // seed_debate_subworkflow(db).await?;  // 辩论子工作流未引用，暂不种子化
    Ok(())
}

/// 将股票分析 DAG 作为工作流模板持久化到 workflow_templates 表。
/// 模板中的 system_prompt 使用 {{stock_code}} / {{stock_name}} / {{data_ctx}} 占位符，
/// 运行时由 run_stock_workflow 替换为实际行情数据。
///
/// ───────────────────────────────────────────────────────────────────────
/// 【装饰节点模式 / Decorative Container Pattern】
/// ───────────────────────────────────────────────────────────────────────
/// 本模板中以下三个"容器节点"是**纯视觉装饰**，不参与实际流程控制：
///
///   1. `p-analysts`       (ParallelNode)  包裹 9 组 (Tool + Agent)
///   2. `debate-bull-bear` (DebateNode)    包裹 6 个真实辩手 (bull-r1..r3, bear-r1..r3)
///   3. `p-risk-assess`    (ParallelNode)  包裹 3 个风险偏好 Agent
///
/// 关键约定：
///   • 容器在引擎中**立即 Completed**，不等子节点
///   • 实际依赖通过**显式 edge** 表达，不依赖容器的调度语义
///   • `parent_id` 字段仅供前端编辑器嵌套渲染，**运行时调度忽略**
///   • 子节点的 context_sources 直接指向"父节点"（容器）的 id，
///     但因为容器瞬时完成，运行时等同于"等触发边到齐即可启动"
///
/// 为什么需要这种设计？
///   前端画布需要把多组节点画在一个可折叠的分组框内，单纯靠 edge
///   拓扑无法表达"视觉从属关系"。容器节点是"调度语义 + 视觉语义"
///   的解耦产物：调度走 edge，视觉走 parent_id。
///
/// 维护警示：
///   任何把"等下游数据"的节点直接连到容器都是错的——容器返回的是
///   配置元数据而非子节点输出。正确接法是连到最后一个真实子节点
///   （如 value-investor 应连到 `bear-r{debate_max_rounds}`，详见 P0 修复）。
/// ───────────────────────────────────────────────────────────────────────
async fn seed_stock_analysis_workflow_template(
    db: &sea_orm::DatabaseConnection,
) -> Result<(), String> {
    use axagent_core::entity::workflow_template;
    use axagent_harness::workflow_types::{
        AgentNode, AgentNodeConfig, AggregatorNode, AggregatorNodeConfig, Branch, CodeNode,
        CodeNodeConfig, DebateNode, DebateNodeConfig, EdgeType, EndNode, EndNodeConfig,
        ErrorConfig, JsonSchema, JsonSchemaProperty, LlmClassifierNode, LlmClassifierNodeConfig,
        MergeStrategy, NotificationNode, NotificationNodeConfig, OnFailureAction, OutputMode,
        ParallelNode, ParallelNodeConfig, Position, RetryConfig, RetryPolicy, StorageNode,
        StorageNodeConfig, SubGraph, SwitchCase, SwitchNode, SwitchNodeConfig, ToolDef, ToolNode,
        ToolNodeConfig, TriggerConfig, TriggerNode, TriggerType, ValidationAssertion,
        ValidationNode, ValidationNodeConfig, Variable, WorkflowEdge, WorkflowNode,
        WorkflowNodeBase,
    };
    use sea_orm::{ActiveModelTrait, EntityTrait, Set};

    const TEMPLATE_ID: &str = "stock-analysis";
    // v15: 修复风险评估 Agent（aggressive/conservative/neutral-debator）缺少
    //   context_sources 的问题。之前 3 个风险评估节点没有配置 context_sources，
    //   LLM 看不到上游 9 个分析师报告和辩论结果，没有分析素材，
    //   因此不会主动调用工具，输出空泛结论。
    //   现在注入所有分析师报告、辩论结果、技术指标，让风险评估有依据。
    // v14: 进一步加强 system_prompt：
    //   - 明确告知 LLM 工具返回空数组/空对象是正常情况（数据源暂无记录）
    //   - 严禁使用 '数据缺失'/'无法获取'/'does not exist'/'error' 等负面措辞
    //   - 研报分析师即使没有机构覆盖数据，也要基于公开信息给出独立分析
    // v13: 修复 tool_def_map 缺失 7 个筹码面/基本面工具定义
    //   （get_stock_lockup/get_stock_shareholder_trades/get_stock_margin_data 等），
    //   导致 LLM 看不到这些工具。同时更新 lockup-watcher 的 PROFILE_TOOLS
    //   和 t-lockup-data 前置工具，让筹码面分析师能获取真正的解禁/增减持数据。
    //   修改 system_prompt 禁止 LLM 使用"工具调用失败"等负面措辞。
    // v12: 修改 Agent system_prompt 使用 {{stock_code}}/{{stock_name}} 模板语法，
    //   让 LLM 在 system_prompt 中直接看到目标股票代码，减少工具调用时遗漏参数。
    //   同时修复 tool_def_map 中缺失 get_stock_peers/get_research_reports 等
    //   工具定义，导致 Agent 暴露给 LLM 的工具列表不完整。
    // v11: 为所有 AgentNodeConfig 添加 input_mapping 字段，自动将 stock_code/stock_name
    //   注入 system_prompt。之前 v10 模板中 Agent 节点没有 input_mapping，导致 LLM
    //   不知道目标股票代码，所有分析师输出为空或"请提供股票代码"。
    // v10: input_var 从 "t-risk.result" 改为 "t-risk"。
    //   之前 v8/v9 尝试用 "t-risk.result" 定位工具包裹层的 result 字段，
    //   但用户反馈仍 VALIDATION_FAILED。观察 v9 新增的可用 key 列表日志
    //   后发现，运行时 context.variables["t-risk"] 在某些场景下不存在。
    //   v10 改用 "t-risk"（整个工具输出 JSON 对象），
    //   让 value_to_input_text 走 pretty JSON 序列化，LLM 直接看到
    //   tool_name/result/truncated 全貌，不再依赖深层字段下钻。
    // v16: 修复 R2 辩论节点（bull-r2/bear-r2）无数据输出。
    //   R2 节点只有 search_stock 一个工具，LLM 若不调用工具则产出空泛；
    //   同时未设置 output_mode: Json，LLM 输出格式不固定，前端解析失败。
    //   现在给 R2 与 R1/R3 相同工具集，并强制 JSON 输出。
    // v17: value-investor 节点设置 output_mode: Json，
    //   同时 prompt 改为直接输出 JSON（不再用代码块包裹），
    //   彻底解决前端解析失败导致显示原始 JSON 的问题。
    // v18: 修复 portfolio-manager confidence 公式（consensus_split 项归一化，所有输入统一到 0-1 * 权重）。
    //   修复 data-quality 节点 context_sources 缺少 t-scoring/t-valuation/t-risk，
    //   导致 data-quality-inspector 无法读取工具 credibility 元数据，工具可信度分始终缺失。
    // v19: 修复情绪类数据工具（get_news JSONP 解析 bug + t-sentiment-data 调用错误工具）。
    //   修复 trader 节点输出纯文本导致前端无法解析（改为 JSON 输出）。
    // v19: 修复情绪类数据工具（get_news JSONP 解析 bug + t-sentiment-data 调用错误工具）。
    //   修复 trader 节点输出纯文本导致前端无法解析（改为 JSON 输出）。
    // v20: 全面布局与连接修复
    //   F-1  3×3 网格重排 trigger/p-analysts/9 组 tool+agent,消除重叠
    //   F-2  tool 节点 title 由硬编码"获取数据"改为 tool_assignments 中已声明的描述
    //   F-3  p-risk-assess / t-risk 同名,分别改名为"三档风险评估分组"/"组合风险计算"
    //   F-5  raw-data aggregator 补 e-raw-data-portfolio-mgr 显式出边
    //   F-6  data-quality 注释明确"context_sources 消费"为预期设计
    //   F-8  a-hot-money / a-lockup / a-research 与 tool_assignments 索引错位修正
    //   F-9  trigger_config.enabled: false → true(启用 schedule)
    //   F-10 反思复盘 trigger_config: None → Manual + as_of_date 必填参数
    // v22: stock-analysis 模板补充 actual_outcome / reflection_depth 变量声明，
    //   portfolio-manager 的 {{actual_outcome}} 在正常分析时为 ""（正常模式），
    //   在反思复盘时 runtime variables 覆盖为实际走势结果。此前仅 reflection 模板声明了
    //   这两个变量，导致 quality-fallback 节点渲染 portfolio-manager 时报 VARIABLE_NOT_FOUND。
    // stock-analysis 模板版本管理从 v1 开始。v4: 重新种子化以应用 Rhai default→dflt 修复
    const TEMPLATE_VERSION: i32 = 8;

    // 升级前保留旧模板的变量自定义值，在函数体外声明以延长生命周期
    let mut old_variables: Option<String> = None;

    if let Some(existing) = workflow_template::Entity::find_by_id(TEMPLATE_ID)
        .one(db)
        .await
        .map_err(|e| format!("查询工作流模板失败: {e}"))?
    {
        if existing.version >= TEMPLATE_VERSION {
            tracing::info!(
                "[stock_analysis_setup] 模板已是最新版本 v{}，跳过种子化以保留用户修改",
                existing.version
            );
            return Ok(());
        }
        tracing::info!(
            "[stock_analysis_setup] 更新股票分析工作流模板 v{} → v{TEMPLATE_VERSION}",
            existing.version
        );
        // 写版本快照（复用 update_workflow_template 的 snapshot 机制）
        let ver_id = format!("{}_v{}", TEMPLATE_ID, existing.version);
        if axagent_core::entity::workflow_template_version::Entity::find_by_id(&ver_id)
            .one(db)
            .await
            .map_err(|e| format!("查重失败: {e}"))?
            .is_none()
        {
            use sea_orm::ActiveModelTrait;
            let snapshot = axagent_core::entity::workflow_template_version::ActiveModel {
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
            snapshot
                .insert(db)
                .await
                .map_err(|e| format!("写入版本快照失败: {e}"))?;
            tracing::info!("[stock_analysis_setup] 旧版本快照已保存: {ver_id}");
        }
        old_variables = existing.variables.clone();
        // 用 UPDATE 替代 DELETE，保留用户自定义变量
    }

    let now = chrono::Utc::now().timestamp_millis();

    let tool_node = |id: &str,
                     title: &str,
                     tool_name: &str,
                     output_var: &str,
                     arg_key: &str,
                     parent_id: Option<&str>,
                     x: f64,
                     y: f64|
     -> WorkflowNode {
        let mut input_mapping = std::collections::HashMap::new();
        input_mapping.insert(arg_key.to_string(), "stock_code".to_string());
        WorkflowNode::Tool(ToolNode {
            base: WorkflowNodeBase {
                id: id.into(),
                title: title.into(),
                description: Some(format!("获取数据: {tool_name}")),
                position: Position { x, y },
                retry: RetryConfig {
                    enabled: true,
                    max_retries: 2,
                    ..Default::default()
                },
                timeout: Some(120),
                enabled: true,
                parent_id: parent_id.map(String::from),
                compensation: None,
            },
            config: ToolNodeConfig {
                tool_name: tool_name.into(),
                input_mapping,
                output_var: output_var.into(),
            },
        })
    };

    // ── ToolDef 参数 schema 辅助构建 ──
    fn sc_prop(desc: &str) -> JsonSchemaProperty {
        JsonSchemaProperty {
            schema_type: "string".into(),
            description: Some(desc.into()),
            default: None,
            enum_values: None,
            format: None,
        }
    }
    fn sc_prop_default(desc: &str, default: &str) -> JsonSchemaProperty {
        JsonSchemaProperty {
            schema_type: "string".into(),
            description: Some(desc.into()),
            default: Some(serde_json::Value::String(default.into())),
            enum_values: None,
            format: None,
        }
    }
    fn int_prop(desc: &str, default: Option<i64>) -> JsonSchemaProperty {
        JsonSchemaProperty {
            schema_type: "integer".into(),
            description: Some(desc.into()),
            default: default.map(|v| serde_json::json!(v)),
            enum_values: None,
            format: None,
        }
    }
    fn stock_code_params() -> Option<JsonSchema> {
        let mut props = std::collections::HashMap::new();
        props.insert("stock_code".into(), sc_prop("6位股票代码，如 600519"));
        Some(JsonSchema {
            schema_type: "object".into(),
            description: None,
            properties: Some(props),
            required: Some(vec!["stock_code".into()]),
            items: None,
        })
    }
    fn no_params() -> Option<JsonSchema> {
        Some(JsonSchema {
            schema_type: "object".into(),
            description: None,
            properties: Some(std::collections::HashMap::new()),
            required: None,
            items: None,
        })
    }
    fn data_params() -> Option<JsonSchema> {
        let mut props = std::collections::HashMap::new();
        props.insert(
            "data".into(),
            JsonSchemaProperty {
                schema_type: "string".into(),
                description: Some("JSON 格式的数值数组或数据序列".into()),
                default: None,
                enum_values: None,
                format: None,
            },
        );
        Some(JsonSchema {
            schema_type: "object".into(),
            description: None,
            properties: Some(props),
            required: None,
            items: None,
        })
    }

    // 常用工具定义
    let td_quote = ToolDef {
        name: "get_stock_quote".into(),
        description: Some("获取股票实时行情：现价、涨跌幅、PE、PB、市值".into()),
        parameters: stock_code_params(),
    };
    let mut kline_props = std::collections::HashMap::new();
    kline_props.insert("stock_code".into(), sc_prop("6位股票代码"));
    kline_props.insert("period".into(), sc_prop_default("周期: daily/weekly/monthly", "daily"));
    kline_props.insert("limit".into(), int_prop("K线数量", Some(120)));
    let td_kline = ToolDef {
        name: "get_stock_kline".into(),
        description: Some("获取K线数据：OHLCV，可指定周期和数量".into()),
        parameters: Some(JsonSchema {
            schema_type: "object".into(),
            description: None,
            properties: Some(kline_props),
            required: Some(vec!["stock_code".into()]),
            items: None,
        }),
    };
    let td_fin = ToolDef {
        name: "get_stock_financials".into(),
        description: Some("获取财务数据：营收、净利润、EPS、ROE、毛利率等".into()),
        parameters: stock_code_params(),
    };
    let mut news_props = std::collections::HashMap::new();
    news_props.insert("stock_code".into(), sc_prop("6位股票代码"));
    news_props.insert("limit".into(), int_prop("新闻数量", Some(30)));
    let td_news = ToolDef {
        name: "get_stock_news".into(),
        description: Some("获取近期新闻公告".into()),
        parameters: Some(JsonSchema {
            schema_type: "object".into(),
            description: None,
            properties: Some(news_props),
            required: Some(vec!["stock_code".into()]),
            items: None,
        }),
    };
    let td_mf = ToolDef {
        name: "get_stock_money_flow".into(),
        description: Some("获取资金流向：主力/超大单/大单/中单/小单净流入".into()),
        parameters: stock_code_params(),
    };
    let td_score = ToolDef {
        name: "compute_scoring".into(),
        description: Some("计算技术评分：基于趋势、偏离度、MACD、成交量、RSI、支撑阻力".into()),
        parameters: stock_code_params(),
    };
    let td_val = ToolDef {
        name: "compute_valuation".into(),
        description: Some("计算估值指标：DCF、F-Score、护城河量化、安全边际".into()),
        parameters: stock_code_params(),
    };
    let mut risk_props = std::collections::HashMap::new();
    risk_props.insert("stock_codes".into(), sc_prop("逗号分隔的股票代码列表"));
    risk_props.insert("weights".into(), sc_prop("逗号分隔的持仓权重(0-1)，不填则等权"));
    let td_risk = ToolDef {
        name: "compute_portfolio_risk".into(),
        description: Some("计算组合风险：总市值、集中度、风险等级".into()),
        parameters: Some(JsonSchema {
            schema_type: "object".into(),
            description: None,
            properties: Some(risk_props),
            required: Some(vec!["stock_codes".into()]),
            items: None,
        }),
    };
    // ── 新增 12 个金融模型 ToolDef ──
    let td_maxdd = ToolDef {
        name: "calc_max_drawdown".into(),
        description: Some("计算最大回撤比例".into()),
        parameters: data_params(),
    };
    let td_sharpe = ToolDef {
        name: "calc_sharpe_ratio".into(),
        description: Some("计算夏普比率".into()),
        parameters: data_params(),
    };
    let td_var = ToolDef {
        name: "calc_var".into(),
        description: Some("历史模拟法 VaR 计算".into()),
        parameters: data_params(),
    };
    let td_pe_pct = ToolDef {
        name: "calc_pe_percentile".into(),
        description: Some("PE 历史分位数".into()),
        parameters: data_params(),
    };
    let td_peg = ToolDef {
        name: "calc_peg".into(),
        description: Some("PEG 估值指标".into()),
        parameters: data_params(),
    };
    let td_ma_cross = ToolDef {
        name: "detect_ma_cross".into(),
        description: Some("MA 金叉死叉检测".into()),
        parameters: data_params(),
    };
    let td_breakout = ToolDef {
        name: "detect_breakout".into(),
        description: Some("支撑阻力突破检测".into()),
        parameters: data_params(),
    };
    let td_kelly = ToolDef {
        name: "calc_kelly".into(),
        description: Some("凯利公式仓位计算".into()),
        parameters: data_params(),
    };
    let td_rp = ToolDef {
        name: "calc_risk_parity".into(),
        description: Some("风险平价权重计算".into()),
        parameters: data_params(),
    };
    // ── 新增 9 个数据 API ToolDef ──
    let td_research = ToolDef {
        name: "get_research_reports".into(),
        description: Some("获取券商研报".into()),
        parameters: stock_code_params(),
    };
    let td_consensus = ToolDef {
        name: "get_consensus_eps".into(),
        description: Some("获取一致性预期EPS".into()),
        parameters: stock_code_params(),
    };
    let td_concepts = ToolDef {
        name: "get_concept_blocks".into(),
        description: Some("获取概念板块归属".into()),
        parameters: stock_code_params(),
    };
    let td_announce = ToolDef {
        name: "get_announcements".into(),
        description: Some("获取公司公告".into()),
        parameters: stock_code_params(),
    };
    let td_north = ToolDef {
        name: "get_north_bound_flow".into(),
        description: Some("获取北向资金流向".into()),
        parameters: no_params(),
    };
    let td_dragon = ToolDef {
        name: "get_market_dragon_tiger".into(),
        description: Some("获取龙虎榜数据".into()),
        parameters: no_params(),
    };
    let td_hot = ToolDef {
        name: "get_hot_stocks".into(),
        description: Some("获取市场热门股".into()),
        parameters: no_params(),
    };
    let td_industry = ToolDef {
        name: "get_industry_ranking".into(),
        description: Some("获取行业涨跌排名".into()),
        parameters: no_params(),
    };
    let td_cls = ToolDef {
        name: "get_cls_flash".into(),
        description: Some("获取财联社实时快讯".into()),
        parameters: no_params(),
    };
    // ── P1: 4 个技术指标 ToolDef ──
    let mut atr_props = std::collections::HashMap::new();
    atr_props.insert("klines_json".into(), sc_prop("K线JSON(含high/low/close)"));
    atr_props.insert("period".into(), int_prop("ATR周期", Some(14)));
    let td_atr = ToolDef {
        name: "compute_atr".into(),
        description: Some("计算 ATR 平均真实波幅".into()),
        parameters: Some(JsonSchema {
            schema_type: "object".into(),
            description: None,
            properties: Some(atr_props),
            required: None,
            items: None,
        }),
    };
    let mut kdj_props = std::collections::HashMap::new();
    kdj_props.insert("klines_json".into(), sc_prop("K线JSON(含high/low/close)"));
    kdj_props.insert("n".into(), int_prop("KDJ周期N", Some(9)));
    let td_kdj = ToolDef {
        name: "compute_kdj".into(),
        description: Some("计算 KDJ 随机指标".into()),
        parameters: Some(JsonSchema {
            schema_type: "object".into(),
            description: None,
            properties: Some(kdj_props),
            required: None,
            items: None,
        }),
    };
    let td_obv = ToolDef {
        name: "compute_obv".into(),
        description: Some("计算 OBV 能量潮".into()),
        parameters: {
            let mut p = std::collections::HashMap::new();
            p.insert("klines_json".into(), sc_prop("K线JSON(含close/volume)"));
            Some(JsonSchema {
                schema_type: "object".into(),
                description: None,
                properties: Some(p),
                required: None,
                items: None,
            })
        },
    };
    let mut beta_props = std::collections::HashMap::new();
    beta_props.insert("stock_returns_json".into(), sc_prop("个股收益率JSON数组"));
    beta_props.insert("market_returns_json".into(), sc_prop("大盘收益率JSON数组"));
    let _td_beta = ToolDef {
        name: "calc_beta".into(),
        description: Some("计算 Beta 系数".into()),
        parameters: Some(JsonSchema {
            schema_type: "object".into(),
            description: None,
            properties: Some(beta_props),
            required: None,
            items: None,
        }),
    };
    // ── P2: 事件检测 + 组合分析 ToolDef ──
    let mut earn_props = std::collections::HashMap::new();
    earn_props.insert("actual_eps".into(), sc_prop("实际EPS"));
    earn_props.insert("consensus_eps".into(), sc_prop("一致预期EPS"));
    let td_earnings = ToolDef {
        name: "detect_earnings_surprise".into(),
        description: Some("检测业绩超预期/低于预期".into()),
        parameters: Some(JsonSchema {
            schema_type: "object".into(),
            description: None,
            properties: Some(earn_props),
            required: None,
            items: None,
        }),
    };
    let mut pledge_props = std::collections::HashMap::new();
    pledge_props.insert("pledge_pct".into(), sc_prop("质押比例(%)"));
    pledge_props.insert("warning_line".into(), sc_prop("预警线(默认50)"));
    pledge_props.insert("liquidation_line".into(), sc_prop("平仓线(默认70)"));
    let td_pledge = ToolDef {
        name: "detect_pledge_risk".into(),
        description: Some("检测大股东质押风险".into()),
        parameters: Some(JsonSchema {
            schema_type: "object".into(),
            description: None,
            properties: Some(pledge_props),
            required: None,
            items: None,
        }),
    };
    let td_corr = ToolDef {
        name: "calc_correlation_matrix".into(),
        description: Some("计算收益率相关系数矩阵".into()),
        parameters: {
            let mut p = std::collections::HashMap::new();
            p.insert("returns_matrix_json".into(), sc_prop("收益率矩阵JSON(二维数组)"));
            Some(JsonSchema {
                schema_type: "object".into(),
                description: None,
                properties: Some(p),
                required: None,
                items: None,
            })
        },
    };
    // ── P3: 独立新能力 ToolDef ──
    let mut mc_props = std::collections::HashMap::new();
    mc_props.insert("current_price".into(), sc_prop("当前价格"));
    mc_props.insert("annual_return".into(), sc_prop("年化收益率(默认0.08)"));
    mc_props.insert("annual_volatility".into(), sc_prop("年化波动率(默认0.3)"));
    mc_props.insert("days".into(), int_prop("模拟天数", Some(30)));
    mc_props.insert("simulations".into(), int_prop("模拟次数", Some(1000)));
    let td_mc = ToolDef {
        name: "run_monte_carlo".into(),
        description: Some("蒙特卡洛模拟价格路径".into()),
        parameters: Some(JsonSchema {
            schema_type: "object".into(),
            description: None,
            properties: Some(mc_props),
            required: None,
            items: None,
        }),
    };
    let mut ind_props = std::collections::HashMap::new();
    ind_props.insert("stock_pe".into(), sc_prop("个股PE"));
    ind_props.insert("stock_growth".into(), sc_prop("个股增长率"));
    ind_props.insert("industry_avg_pe".into(), sc_prop("行业平均PE"));
    ind_props.insert("industry_avg_growth".into(), sc_prop("行业平均增长率"));
    let td_ind = ToolDef {
        name: "analyze_industry_position".into(),
        description: Some("行业内估值/增长对比分析".into()),
        parameters: Some(JsonSchema {
            schema_type: "object".into(),
            description: None,
            properties: Some(ind_props),
            required: None,
            items: None,
        }),
    };
    let mut lup_props = std::collections::HashMap::new();
    lup_props.insert("klines_json".into(), sc_prop("K线JSON(含close/high/volume)"));
    lup_props.insert("market_type".into(), sc_prop("板块: main/star/chinext/bj"));
    let td_lup = ToolDef {
        name: "detect_limit_up_potential".into(),
        description: Some("涨停潜力评估".into()),
        parameters: Some(JsonSchema {
            schema_type: "object".into(),
            description: None,
            properties: Some(lup_props),
            required: None,
            items: None,
        }),
    };
    let td_block = ToolDef {
        name: "get_block_trades".into(),
        description: Some("获取大宗交易记录：成交价、成交量、买卖方营业部、折价率".into()),
        parameters: stock_code_params(),
    };
    let td_visit = ToolDef {
        name: "get_institutional_visits".into(),
        description: Some("获取机构调研记录：调研日期、机构数量、调研内容".into()),
        parameters: stock_code_params(),
    };
    let td_idx = ToolDef {
        name: "get_index_quotes".into(),
        description: Some("获取大盘指数行情（上证指数、深证成指、创业板指）".into()),
        parameters: no_params(),
    };
    let td_peers = ToolDef {
        name: "get_stock_peers".into(),
        description: Some("获取同行业可比公司估值（PE/PB/ROE/涨跌幅/市值）".into()),
        parameters: stock_code_params(),
    };
    let td_pcr = ToolDef {
        name: "get_stock_option_pcr".into(),
        description: Some("获取期权PCR（看跌/看涨比率和持仓量比率，市场情绪前瞻指标）".into()),
        parameters: stock_code_params(),
    };
    let td_lockup = ToolDef {
        name: "get_stock_lockup".into(),
        description: Some("获取限售解禁日程（解禁日期、股数、比例、股东名称）".into()),
        parameters: stock_code_params(),
    };
    let td_sh_trades = ToolDef {
        name: "get_stock_shareholder_trades".into(),
        description: Some("获取大股东增减持记录（变动类型、数量、均价、原因）".into()),
        parameters: stock_code_params(),
    };
    let td_dividend = ToolDef {
        name: "get_stock_dividend_records".into(),
        description: Some("获取除权除息/分红送配记录".into()),
        parameters: stock_code_params(),
    };
    let td_nb_holding = ToolDef {
        name: "get_stock_north_bound".into(),
        description: Some("获取北向资金个股持仓（持股数量、占比）".into()),
        parameters: stock_code_params(),
    };
    let td_dt = ToolDef {
        name: "get_stock_dragon_tiger".into(),
        description: Some("获取个股龙虎榜数据（营业部买卖、上榜原因）".into()),
        parameters: stock_code_params(),
    };
    let td_margin = ToolDef {
        name: "get_stock_margin_data".into(),
        description: Some("获取融资融券数据（融资买入额、余额、融券卖出量、余量）".into()),
        parameters: stock_code_params(),
    };
    let td_sector_info = ToolDef {
        name: "get_stock_sector_info".into(),
        description: Some("获取行业分类（申万一级/二级、概念板块标签）".into()),
        parameters: stock_code_params(),
    };

    // 工具名 → ToolDef 映射（用于按名查找，给节点填充 config.tools）
    let tool_def_map: std::collections::HashMap<&str, ToolDef> = [
        ("get_stock_quote", td_quote.clone()),
        ("get_stock_kline", td_kline.clone()),
        ("get_stock_financials", td_fin.clone()),
        ("get_stock_news", td_news.clone()),
        ("get_stock_money_flow", td_mf.clone()),
        ("compute_scoring", td_score.clone()),
        ("compute_valuation", td_val.clone()),
        ("compute_portfolio_risk", td_risk.clone()),
        (
            "search_stock",
            ToolDef {
                name: "search_stock".into(),
                description: Some("按代码或名称模糊搜索A股".into()),
                parameters: None,
            },
        ),
        ("get_hot_stocks", td_hot.clone()),
        ("get_industry_ranking", td_industry.clone()),
        ("get_announcements", td_announce.clone()),
        ("get_consensus_eps", td_consensus.clone()),
        ("compute_kdj", td_kdj.clone()),
        ("compute_obv", td_obv.clone()),
        ("get_cls_flash", td_cls.clone()),
        ("get_north_bound_flow", td_north.clone()),
        ("get_market_dragon_tiger", td_dragon.clone()),
        ("get_research_reports", td_research.clone()),
        ("get_concept_blocks", td_concepts.clone()),
        ("get_block_trades", td_block.clone()),
        ("get_institutional_visits", td_visit.clone()),
        ("get_index_quotes", td_idx.clone()),
        ("get_stock_peers", td_peers.clone()),
        ("get_stock_option_pcr", td_pcr.clone()),
        ("get_stock_lockup", td_lockup.clone()),
        ("get_stock_shareholder_trades", td_sh_trades.clone()),
        ("get_stock_dividend_records", td_dividend.clone()),
        ("get_stock_north_bound", td_nb_holding.clone()),
        ("get_stock_dragon_tiger", td_dt.clone()),
        ("get_stock_margin_data", td_margin.clone()),
        ("get_stock_sector_info", td_sector_info.clone()),
    ]
    .into_iter()
    .collect();

    // 从 ToolDef 列表生成 "可用工具" prompt 片段
    fn tool_prompt(tools: &[ToolDef]) -> String {
        if tools.is_empty() {
            return String::new();
        }
        let names: Vec<&str> = tools.iter().map(|t| t.name.as_str()).collect();
        format!(
            "\n\n你可以调用以下工具获取最新数据或计算指标：{}。请先调用相关工具获取数据，再基于返回结果进行分析。",
            names.join("、")
        )
    }

    let agent = |id: &str,
                 title: &str,
                 expert_id: &str,
                 parent_id: Option<&str>,
                 x: f64,
                 y: f64|
     -> WorkflowNode {
        WorkflowNode::Agent(AgentNode {
            base: WorkflowNodeBase {
                id: id.into(),
                title: title.into(),
                description: Some(format!("股票分析: {expert_id}")),
                position: Position { x, y },
                retry: RetryConfig {
                    enabled: true,
                    max_retries: 2,
                    ..Default::default()
                },
                timeout: Some(300),
                enabled: true,
                parent_id: parent_id.map(String::from),
                compensation: None,
            },
            config: AgentNodeConfig {
                // inline system_prompt 只放任务指令，专家 prompt 由 agent_profile 自动加载，
                // 行情数据通过 context_sources 由上游 Tool 节点输出自动注入
                system_prompt: format!(
                    "你的任务: {title}\n目标股票代码: {{stock_code}}，股票名称: {{stock_name}}\n\n重要原则：\n1. 如果上游数据节点返回为空，请主动调用可用工具获取补充数据。\n2. 如果确实无法获取某些数据，基于你已知的公开信息和通用分析框架给出尽可能有价值的分析，不要只列 data_gaps。\n3. 始终针对目标股票给出明确的观点（看多/看空/中性）和论据，不要输出空结果。\n4. 调用任何需要 stock_code 参数的工具时，必须始终传递 stock_code={{stock_code}}。\n5. 分析输出中严禁出现'工具调用失败'、'在当前环境中不可用'、'上游数据获取为空'、'数据缺失'、'无法获取'、'does not exist'、'error'等负面措辞。工具返回空数组[]或空对象{{}}是正常情况（表示该数据源暂无记录），请直接基于已有信息给出分析结论。\n6. 如果你是研报分析师，目标是从券商研报、一致预期EPS、机构调研等维度给出观点。如果这些数据源返回空，说明该股票暂无机构覆盖，你可以基于公司基本面、行业地位、新闻公告等公开信息给出独立分析，不要强调'无券商研报'。",
                ),
                context_sources: vec![],
                // 通过 input_mapping 自动注入股票代码/名称到 system_prompt
                input_mapping: [
                    ("stock_code".to_string(), "stock_code".to_string()),
                    ("stock_name".to_string(), "stock_name".to_string()),
                ]
                .into_iter()
                .collect(),
                output_var: id.into(),
                model: None,
                temperature: Some(0.3),
                max_tokens: Some(8192),
                tools: vec![],
                exposed_tools: vec![],
                output_mode: OutputMode::Text,
                agent_profile_id: Some(format!("stock-{expert_id}")),
                max_tool_rounds: None,
                execution_mode: None,
                rag_source_ids: vec![],
                model_role: None,
                consistency_check: None,
                hallucination_guard: None,
            },
        })
    };

    let edge = |id: &str, source: &str, target: &str| -> WorkflowEdge {
        WorkflowEdge {
            id: id.into(),
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

    // Trigger
    nodes.push(WorkflowNode::Trigger(TriggerNode {
        base: WorkflowNodeBase {
            id: "trigger".into(),
            title: "开始分析".into(),
            description: Some("输入股票代码启动分析".into()),
            // F-1 修复: 3×3 网格最右列 x=1240+200=1440, 居中 trigger x=520
            position: Position { x: 520.0, y: 0.0 },
            retry: RetryConfig::default(),
            timeout: None,
            enabled: true,
            parent_id: None,
            compensation: None,
        },
        config: TriggerConfig {
            trigger_type: TriggerType::Manual,
            config: serde_json::json!({"stock_code": "{{stock_code}}"}),
        },
    }));

    // 9 个分析师 + catalyst-analyst
    let analysts = [
        (
            "a-market-analyst",
            "技术面分析：K线形态、MACD/RSI、支撑阻力位",
            "market-analyst",
        ),
        ("a-sentiment", "市场情绪分析：资金流向、散户/机构态度", "sentiment-analyst"),
        ("a-news", "新闻公告影响评估", "news-analyst"),
        ("a-fundamentals", "基本面估值分析：PE/PB/ROE等", "fundamentals-analyst"),
        ("a-policy", "宏观政策与行业政策影响分析", "policy-analyst"),
        ("a-hot-money", "游资动向与主力资金追踪", "hot-money-tracker"),
        ("a-lockup", "解禁减持与质押风险排查", "lockup-watcher"),
        ("a-research", "券商研报观点汇总", "research-analyst"),
        ("a-sector", "行业景气度与轮动分析", "sector-analyst"),
        ("a-catalyst", "催化剂与叙事完整度评估", "catalyst-analyst"),
    ];
    let a_ids: Vec<&str> = analysts.iter().map(|(id, _, _)| *id).collect();

    // 为每个分析师插入对应的数据获取 Tool 节点
    // 注：节点工具决定了下游 analyst 拿到的"前置数据"。LLM agent 自身仍可调用
    // PROFILE_TOOLS 中的工具，但首屏/冷启动数据由这些 tool 节点预拉。
    //
    // F-8 修复: 顺序必须与上面的 `analysts` 数组完全一致：
    //   [0] a-market-analyst   ↔ t-market-data
    //   [1] a-sentiment        ↔ t-sentiment-data
    //   [2] a-news             ↔ t-news-data
    //   [3] a-fundamentals     ↔ t-fundamentals-data
    //   [4] a-policy           ↔ t-policy-data
    //   [5] a-hot-money        ↔ t-hotmoney-data   (原: t-research-data 错位)
    //   [6] a-lockup           ↔ t-lockup-data     (原: t-hotmoney-data 错位)
    //   [7] a-research         ↔ t-research-data   (原: t-lockup-data 错位)
    //   [8] a-sector           ↔ t-sector-data
    // 错位会导致 hot-money analyst 拿到研报数据、research analyst 拿到解禁数据，
    // 9 个分析师产出的报告与各自的角色语义不符。
    let tool_assignments: &[(&str, &str, &str, &str)] = &[
        ("t-market-data", "获取K线+行情", "get_stock_kline", "stock_code"),
        // 修复: t-sentiment-data 原调用 get_hot_stocks（热门股票列表，非个股新闻），
        // 导致情绪面分析师拿不到个股新闻舆情数据。改为 get_stock_news。
        ("t-sentiment-data", "获取新闻+热门", "get_stock_news", "stock_code"),
        // 修复: t-news-data 原调用 get_announcements（公告），导致消息面分析师
        // 拿不到个股新闻数据。改为 get_stock_news 与 a-news 的 data_sources 匹配。
        // 公告数据已由 t-catalyst-data 负责获取。
        ("t-news-data", "获取新闻+公告", "get_stock_news", "stock_code"),
        // 修复 P1: 基本面分析师前置数据改用 get_stock_financials（财报）而非
        // get_consensus_eps（一致预期），让 a-fundamentals 启动时就能拿到
        // 营收/利润/资产负债等核心财务数据。
        ("t-fundamentals-data", "获取财务数据", "get_stock_financials", "stock_code"),
        // 修复 P1: 政策分析师前置数据改用 get_stock_news（新闻）而非
        // get_announcements（公告）。新闻覆盖宏观/产业政策动态更广，
        // 与 a-news 的公告视角形成互补。
        //
        // F-4 待办: 当前 get_stock_news 与 t-sentiment-data 完全重复调用,
        //   实际差异化靠 a-policy 的 system_prompt 提示词过滤"政策类"新闻。
        //   理想方案: 在 src-tauri/src/tools/finance.rs 注册新工具
        //   `get_policy_news`,接受参数 category=policy 走单独数据源(政府/官媒/
        //   监管公告),此处把 tool_name 改为 "get_policy_news" 即可。
        //   本次仅修 title 让 a-policy 与 a-sentiment 在画布上可区分。
        ("t-policy-data", "获取政策新闻", "get_stock_news", "stock_code"),
        // F-8 重排: a-hot-money 前置改为资金流向工具
        ("t-hotmoney-data", "获取资金流向", "get_stock_money_flow", "stock_code"),
        // F-8 重排: a-lockup 前置改为解禁质押工具
        ("t-lockup-data", "获取解禁质押", "get_stock_lockup", "stock_code"),
        // F-8 重排: a-research 前置改为研报工具
        ("t-research-data", "获取研报+新闻", "get_research_reports", "stock_code"),
        ("t-sector-data", "获取行情+行业排名", "get_industry_ranking", "stock_code"),
        ("t-catalyst-data", "获取公司公告", "get_announcements", "stock_code"),
    ];

    // ── Phase 1: ParallelNode 作为视觉分组，包裹 9 组 Tool + Agent ──
    // F-1 修复: 布局从"2 列 9 行"改为"3 列 3 行"网格。
    //   原布局 (x=20 单一列, 9 行 80px) 存在 3 类重叠：
    //     1) trigger (x=250, y=0) 与 a-market-analyst (x=240, y=40) 边界框重叠 ~7600 px²
    //     2) p-analysts 容器 (x=300, y=200) 与 a-fundamentals (x=240, y=200)
    //        等 3 行 analyst 节点重叠
    //     3) 单一纵列 9 行总高 720px 浪费大量垂直空间
    //   新布局: 3×3 网格,col_width=480 (tool 200 + gap 40 + agent 200 + 余量 40)
    //     col_x = [40, 520, 1000]
    //     tool x = col_x[col], agent x = col_x[col] + 240
    //     row_y = 100 + row*120  (节点高 80, 行距 40)
    //   trigger 居中放置 x=580 (3 列总宽 1200, 居中后左侧 580),y=0
    //   p-analysts 容器 (20, 80) 起,完整包络 3×3 网格
    let col_x = [40.0_f64, 520.0, 1000.0];
    let row_y_base = 100.0;
    // FIX: agent 节点高度 160px, 之前 row_dy=120 导致连续行重叠 40px
    let row_dy = 180.0;
    let mut analyst_branches: Vec<Branch> = Vec::with_capacity(tool_assignments.len());
    for (i, (tool_id, tool_title, tool_name, arg_key)) in tool_assignments.iter().enumerate() {
        let analyst_id = a_ids[i];
        let col = i % 3;
        let row = i / 3;
        let x_tool = col_x[col];
        let y = row_y_base + row as f64 * row_dy;
        nodes.push(tool_node(
            tool_id,
            // F-2 修复: 原本硬编码 "获取数据" 导致 9 个 tool 节点 title 完全一致、
            // 编辑器画布无法区分。改用 tool_assignments 中已经声明的中文描述。
            tool_title,
            tool_name,
            tool_id,
            arg_key,
            Some("p-analysts"),
            x_tool,
            y,
        ));
        edges.push(edge(&format!("e-trigger-{tool_id}"), "trigger", tool_id));
        edges.push(edge(&format!("e-{tool_id}-{analyst_id}"), tool_id, analyst_id));
        analyst_branches.push(Branch {
            id: format!("branch-{analyst_id}"),
            title: tool_title.to_string(),
            steps: vec![tool_id.to_string(), analyst_id.to_string()],
            branch_timeout_ms: None,
            degrade_strategy: Default::default(),
        });
    }

    // 工具由模板节点 config.tools 统一管理
    // 第 10 个 a-catalyst 放置在 3×3 网格下方（col 0, row 3），作为额外独立行
    for (i, (id, title, _expert)) in analysts.iter().enumerate() {
        let tool_id = tool_assignments[i].0;
        let _fixed_tool_name = tool_assignments[i].2;
        let col = i % 3;
        let row = i / 3;
        let x_agent = col_x[col] + 240.0;
        let row_y = row_y_base + row as f64 * row_dy;
        let mut an = agent(id, title, _expert, Some("p-analysts"), x_agent, row_y);
        if let WorkflowNode::Agent(ref mut a) = an {
            a.config.context_sources = vec![tool_id.to_string()];
            // catalyst-analyst 需要 3 轮：R1 读公告→确认催化剂,R2 调 K线/概念验证,R3 综合评估叙事
            a.config.max_tool_rounds = if *id == "a-catalyst" {
                Some(3)
            } else {
                Some(2)
            };
            a.config.model_role = Some("stock-analyst".into());
            let tool_names = PROFILE_TOOLS
                .iter()
                .find(|(k, _)| **k == **_expert)
                .map(|(_, v)| *v)
                .unwrap_or(&[]);
            a.config.tools = tool_names
                .iter()
                .filter_map(|&tn| tool_def_map.get(tn).cloned())
                .collect();
            a.config.exposed_tools = vec![]; // 空 = 暴露全部 tools，允许 Agent 在 ToolNode 数据不足时自行补充调用
            a.config.system_prompt =
                format!("{}{}", a.config.system_prompt, tool_prompt(&a.config.tools));
            // 环 A: 注入历史反思教训，让分析师看到该股之前的错因和改进建议
            a.config.input_mapping =
                std::collections::HashMap::from([("stock_lessons".into(), "stock_lessons".into())]);
        }
        nodes.push(an);
    }

    // 分析师节点 → c-need-debate 的出边（编辑器可视化 + 运行时依赖）
    for aid in &a_ids {
        edges.push(edge(&format!("e-{aid}-debate"), aid, "debate-bull-bear"));
    }

    // ═══════════════════════════════════════════════════════════════════════
    // 【装饰节点 / Decorative Container】p-analysts
    // ═══════════════════════════════════════════════════════════════════════
    // 语义：视觉分组容器，包裹 9 组 (Tool + Agent) 子节点
    // 调度：容器本身在引擎中立即 Completed（不参与流程控制）
    //      - wait_for_all=true, aggregation=All: 等所有子节点完成后聚合
    //      - auto_input_from_parent=false: 不自动从父节点拉数据
    //      - 实际依赖通过显式 edge 表达（e-trigger-{tool_id} 和 e-{tool_id}-{aid}）
    // parent_id：仅供前端编辑器嵌套渲染用，运行时调度忽略此字段
    //
    // 为什么不直接用 Edges 表达？
    //   前端需要把 9 个 Tool 和 9 个 Agent 画在一个可折叠的分组框内，
    //   单纯靠 edge 拓扑无法表达"视觉从属关系"。ParallelNode 在此
    //   充当"虚拟容器"，是 workflow_types 中两种角色之一的产物：
    //     1) 真正的并行控制器（wait_for_all + 聚合）
    //     2) 纯装饰性容器（仅前端展示，调度无意义）  ← 属于此类
    // ═══════════════════════════════════════════════════════════════════════
    nodes.push(WorkflowNode::Parallel(ParallelNode {
        base: WorkflowNodeBase {
            id: "p-analysts".into(),
            title: "10 维度分析师分组".into(),
            description: Some("行情/情绪/新闻/基本面/政策/游资/解禁/研报/行业/催化剂".into()),
            // F-1 修复: 原 (300, 200) 恰好压在 a-fundamentals (240, 200) 上。
            //   3×3 网格范围 x∈[40, 1400] y∈[100, 460],容器左上放 (20, 80),
            //   让前端能正确按 bbox 渲染分组框。
            position: Position { x: 20.0, y: 80.0 },
            retry: RetryConfig::default(),
            timeout: Some(120),
            enabled: true,
            parent_id: None,
            compensation: None,
        },
        config: ParallelNodeConfig {
            branches: analyst_branches,
            wait_for_all: true,
            timeout: Some(600),
            aggregation: Some(MergeStrategy::All),
            auto_input_from_parent: false, // 不自动从父节点接收输入
            sub_graph: None,               // v23+：稍后通过 inject_container_subgraphs 注入
        },
    }));

    // Phase 2: 决策检查点 — 记录分析师完成状态，辩论始终执行
    // 分析师节点已直接连接 DebateNode（无中间条件节点）

    // ── 辩论轮数（DAG 展开为 max_rounds 轮顺序执行） ──
    // 用户在「股票分析设置 → 参数 → 工作流 → 多空辩论轮数」中调整的 `debate_rounds`
    // 会在旧模板升级时被 merge_variable_values 保留到 old_variables 里；这里
    // 优先读旧值，确保重建后的 DAG 与用户当前意图一致；缺失/越界时回退到 3。
    let debate_max_rounds: usize = match old_variables.as_deref() {
        Some(s) if !s.is_empty() => serde_json::from_str::<Vec<serde_json::Value>>(s)
            .ok()
            .and_then(|arr| {
                arr.into_iter().find_map(|v| {
                    let name = v.get("name")?.as_str()?;
                    if name != "debate_rounds" {
                        return None;
                    }
                    v.get("value")?.as_u64().map(|n| n as usize)
                })
            })
            .map(|n| n.clamp(1, 10))
            .unwrap_or(3),
        _ => 3,
    };

    // ═══════════════════════════════════════════════════════════════════════
    // 【装饰节点 / Decorative Container】debate-bull-bear
    // ═══════════════════════════════════════════════════════════════════════
    // 语义：多空辩论的视觉分组容器，配置 max_rounds=3 的辩论元数据
    // 调度：容器本身在引擎中立即 Completed（返回 debater_steps 配置，不返回辩论结果）
    //      - debater_steps: 6 个真实辩手节点 (bull-r1..r3, bear-r1..r3)
    //      - max_rounds=3: 固定 3 轮，无"是否收敛"循环控制
    //      - convergence_prompt/model: 配置就绪但当前未启用（避免辩论死循环）
    //
    // ⚠️ 关键陷阱（P0 已修复）：
    //   历史 bug：曾将 value-investor 的入边连到本容器，导致 value-investor
    //   在容器 Completed 时立即启动——拿到的是"辩论配置"而非"辩论结果"。
    //   正确接法：value-investor 应等待最后一个真实辩手节点 bear-r3 完成。
    //
    // 真实调度依赖链（首轮 bull-r1 启动条件）：
    //   trigger → tool → a-* → debate-bull-bear（立即完成）→ bull-r1
    //   后续轮次：bull-r{r+1} 等 bear-r{r}，bear-r{r} 等 bull-r{r}
    // parent_id：仅供前端编辑器嵌套渲染用
    //
    // ⚠️ 坐标约定（FIX: 所有节点位置为画布绝对坐标）：
    //   容器 debate-bull-bear 放在 (DEBATE_X, DEBATE_Y)
    //   辩手节点 x = DEBATE_X + 20px（容器内偏移）
    //   辩手节点 y = DEBATE_Y + 40px + round*2*180px（按轮次纵向排列）
    //   前端 WorkflowEditor 通过 parentId 减去容器坐标得到相对坐标交给 ReactFlow。
    // ═══════════════════════════════════════════════════════════════════════
    const DEBATE_X: f64 = 300.0;
    const DEBATE_Y: f64 = 1280.0;
    nodes.push(WorkflowNode::Debate(DebateNode {
        base: WorkflowNodeBase {
            id: "debate-bull-bear".into(),
            title: "多空辩论".into(),
            description: Some(format!(
                "{debate_max_rounds} 轮多空辩论：多方构建论点 → 空方反驳 → 循环"
            )),
            position: Position {
                x: DEBATE_X,
                y: DEBATE_Y,
            },
            retry: RetryConfig {
                enabled: true,
                max_retries: 1,
                ..Default::default()
            },
            timeout: Some(900),
            enabled: true,
            parent_id: None,
            compensation: None,
        },
        config: DebateNodeConfig {
            debater_steps: (0..debate_max_rounds)
                .flat_map(|r| vec![format!("bull-r{}", r + 1), format!("bear-r{}", r + 1)])
                .collect(),
            max_rounds: debate_max_rounds as u32,
            convergence_prompt: None,
            convergence_model: None,
            convergence_model_role: None,
            topic_var: "trigger.output".into(),
            output_var: String::new(),
            sub_graph: None, // v23+：稍后通过 inject_container_subgraphs 注入
        },
    }));

    // DebateNode 的子节点：按轮次展开多方辩手和空方辩手
    // parentId 指向容器节点，前端将它们渲染在 DebateNode 内部
    // 位置：容器内 20px 左偏移，按轮次纵向排列（绝对坐标 = 容器坐标 + 偏移）
    let bull_tools = vec![
        td_quote.clone(),
        td_kline.clone(),
        td_fin.clone(),
        td_news.clone(),
        td_score.clone(),
        td_earnings.clone(),
        td_ma_cross.clone(),
    ];
    let bear_tools = vec![
        td_quote.clone(),
        td_kline.clone(),
        td_fin.clone(),
        td_news.clone(),
        td_var.clone(),
        td_maxdd.clone(),
        td_pledge.clone(),
        td_corr.clone(),
    ];

    for round in 0..debate_max_rounds {
        let round_num = round + 1;
        let bull_id = format!("bull-r{round_num}");
        let bear_id = format!("bear-r{round_num}");
        // 修复 Defect #4: 第 2 轮用 bull-r2/bear-r2（质询型 prompt），第 1/3 轮
        // 继续用 bull-researcher/bear-researcher（初始论据 / 最终反驳）。
        let bull_expert = if round_num == 2 {
            "bull-r2"
        } else {
            "bull-researcher"
        };
        let bear_expert = if round_num == 2 {
            "bear-r2"
        } else {
            "bear-researcher"
        };
        let bull_title = format!("多方研究员·第{round_num}轮");
        let bear_title = format!("空方研究员·第{round_num}轮");
        // 绝对坐标 = 容器基准 + 内部偏移
        let bull_x = DEBATE_X + 20.0;
        let bull_y = DEBATE_Y + 40.0 + (round * 2) as f64 * 180.0;
        let bear_x = DEBATE_X + 20.0;
        let bear_y = DEBATE_Y + 40.0 + (round * 2 + 1) as f64 * 180.0;

        // 多方辩手：首轮无前置辩论上下文，后续轮次引用所有前序辩论输出
        let mut bull_an =
            agent(&bull_id, &bull_title, bull_expert, Some("debate-bull-bear"), bull_x, bull_y);
        if let WorkflowNode::Agent(ref mut a) = bull_an {
            // v16: R2 质询型辩手强制 JSON 输出，工具轮次提升到 2
            if round_num == 2 {
                if let Some(names) = PROFILE_TOOLS
                    .iter()
                    .find(|(k, _)| *k == bull_expert)
                    .map(|(_, v)| *v)
                {
                    a.config.tools = names
                        .iter()
                        .filter_map(|&tn| tool_def_map.get(tn).cloned())
                        .collect();
                    a.config.exposed_tools = names.iter().map(|&tn| tn.to_string()).collect();
                    a.config.system_prompt =
                        format!("{}{}", a.config.system_prompt, tool_prompt(&a.config.tools));
                }
                a.config.max_tool_rounds = Some(2);
                a.config.output_mode = OutputMode::Json;
            } else {
                a.config.tools = bull_tools.clone();
                a.config.max_tool_rounds = Some(2);
                a.config.system_prompt =
                    format!("{}{}", a.config.system_prompt, tool_prompt(&bull_tools));
            }
            a.config.model_role = Some("debater".into());
            // 注入前序轮次辩论输出 + 所有分析师报告作为上下文
            let mut ctx: Vec<String> = Vec::new();
            for r in 1..round_num {
                ctx.push(format!("bull-r{r}"));
                ctx.push(format!("bear-r{r}"));
            }
            // 添加所有分析师报告，让辩手有素材可辩论
            for aid in &a_ids {
                ctx.push(aid.to_string());
            }
            a.config.context_sources = ctx;
            // 注入分析师 params 作为结构化输入（resolve_var_path 支持点号路径）
            a.config.input_mapping = build_analyst_input_mapping(&a_ids);
        }
        nodes.push(bull_an);

        // 空方辩手：引用本轮多方输出 + 前序轮次辩论输出
        let mut bear_an =
            agent(&bear_id, &bear_title, bear_expert, Some("debate-bull-bear"), bear_x, bear_y);
        if let WorkflowNode::Agent(ref mut a) = bear_an {
            // v16: R2 质询型辩手强制 JSON 输出，工具轮次提升到 2
            if round_num == 2 {
                if let Some(names) = PROFILE_TOOLS
                    .iter()
                    .find(|(k, _)| *k == bear_expert)
                    .map(|(_, v)| *v)
                {
                    a.config.tools = names
                        .iter()
                        .filter_map(|&tn| tool_def_map.get(tn).cloned())
                        .collect();
                    a.config.exposed_tools = names.iter().map(|&tn| tn.to_string()).collect();
                    a.config.system_prompt =
                        format!("{}{}", a.config.system_prompt, tool_prompt(&a.config.tools));
                }
                a.config.max_tool_rounds = Some(2);
                a.config.output_mode = OutputMode::Json;
            } else {
                a.config.tools = bear_tools.clone();
                a.config.max_tool_rounds = Some(2);
                a.config.system_prompt =
                    format!("{}{}", a.config.system_prompt, tool_prompt(&bear_tools));
            }
            a.config.model_role = Some("debater".into());
            // 注入前序轮次 + 本轮多方输出 + 所有分析师报告作为上下文
            let mut ctx: Vec<String> = Vec::new();
            for r in 1..round_num {
                ctx.push(format!("bull-r{r}"));
                ctx.push(format!("bear-r{r}"));
            }
            ctx.push(bull_id.clone());
            // 添加所有分析师报告
            for aid in &a_ids {
                ctx.push(aid.to_string());
            }
            a.config.context_sources = ctx;
            // 注入分析师 params 作为结构化输入
            a.config.input_mapping = build_analyst_input_mapping(&a_ids);
        }
        nodes.push(bear_an);

        // ── 轮次依赖边 ──
        if round == 0 {
            // 首轮：从 DebateNode 容器出发
            edges.push(edge(&format!("e-debate-bull-r{round_num}"), "debate-bull-bear", &bull_id));
        } else {
            // 后续轮次：上一轮空方完成后启动本轮多方
            let prev_bear = format!("bear-r{}", round);
            edges.push(edge(&format!("e-r{round}-bull-r{round_num}"), &prev_bear, &bull_id));
        }
        // 每轮：多方 → 空方（空方看到多方论点后反驳）
        edges.push(edge(&format!("e-bull-r{round_num}-bear-r{round_num}"), &bull_id, &bear_id));
    }

    // ── debate-convergence（辩论收敛分析）──
    // 读取全部 6 轮辩手输出，输出 consensus_score 供 portfolio-mgr 公式使用。
    // 入边从 bear-r{debate_max_rounds} 出发，确保等真辩论结束后再启动收敛。
    // 出边到 value-investor 和 portfolio-mgr，确保收敛结果在决策前可用。
    {
        let last_debate_node = format!("bear-r{debate_max_rounds}");
        let mut dc = agent(
            "debate-convergence",
            "辩论结果收敛：consensus_score 聚合",
            "debate-convergence",
            None,
            500.0,
            1420.0,
        );
        if let WorkflowNode::Agent(ref mut a) = dc {
            // 动态构建 context_sources：根据实际辩论轮数引用所有辩手输出
            let mut ctx: Vec<String> = Vec::new();
            for r in 1..=debate_max_rounds {
                ctx.push(format!("bull-r{r}"));
                ctx.push(format!("bear-r{r}"));
            }
            a.config.context_sources = ctx;
            a.config.model_role = Some("debater".into());
            a.config.max_tool_rounds = Some(1);
            a.config.output_mode = OutputMode::Json;
            a.config.input_mapping = build_analyst_input_mapping(&a_ids);
        }
        nodes.push(dc);
        edges.push(edge("e-bear-r3-debate-convergence", &last_debate_node, "debate-convergence"));
    }

    // ── value-investor（巴菲特框架）：在辩论之后、与风险评估并行运行 ──
    // 入边从 bear-r{debate_max_rounds} 出发，确保等真辩论收敛后再启动
    // （debate-bull-bear 是 DebateNode 容器，立即 Completed，返回的是配置而非辩论结果）
    {
        let vi_id = "value-investor";
        let vi_title = "以巴菲特-芒格价值投资理念评估该标的，分析护城河、财务健康度、管理层、安全边际，输出结构化估值框架";
        let vi_y = 1540.0;
        let last_debate_node = format!("bear-r{debate_max_rounds}");
        let mut vi = agent(vi_id, vi_title, "value-investor", None, 20.0, vi_y);
        if let WorkflowNode::Agent(ref mut a) = vi {
            a.config.context_sources = vec![
                "a-fundamentals".into(),
                "a-research".into(),
                "a-sector".into(),
                // 改为辩论最后一轮空方的输出（真辩论结论），而非 DebateNode 容器
                last_debate_node.clone(),
                "debate-convergence".into(),
            ];
            a.config.model_role = Some("stock-analyst".into());
            a.config.max_tool_rounds = Some(2);
            a.config.output_mode = OutputMode::Json;
            let tool_names = PROFILE_TOOLS
                .iter()
                .find(|(k, _)| *k == "value-investor")
                .map(|(_, v)| *v)
                .unwrap_or(&[]);
            a.config.tools = tool_names
                .iter()
                .filter_map(|&tn| tool_def_map.get(tn).cloned())
                .collect();
            a.config.exposed_tools = tool_names.iter().map(|&tn| tn.to_string()).collect();
            a.config.system_prompt =
                format!("{}{}", a.config.system_prompt, tool_prompt(&a.config.tools));
            // 环 A: 注入历史反思教训
            a.config.input_mapping =
                std::collections::HashMap::from([("stock_lessons".into(), "stock_lessons".into())]);
        }
        nodes.push(vi);
        edges.push(edge("e-debate-value-investor", &last_debate_node, vi_id));
    }

    // ═══════════════════════════════════════════════════════════════════════
    // 【装饰节点 / Decorative Container】p-risk-assess
    // ═══════════════════════════════════════════════════════════════════════
    // 语义：风险评估的视觉分组容器，包裹 3 个并行风险偏好 Agent
    // 调度：与 p-analysts 相同——容器立即 Completed，子节点独立调度
    //      - aggressive-debator / conservative-debator / neutral-debator
    //      - 3 个子节点共享同一份风险输入（来自聚合后的辩论+分析师输出）
    //      - 实际依赖通过显式 edge 表达：e-bear-r3 → p-risk-assess（容器）→ 3 个子节点
    //        容器完成是"瞬时"的，3 个子节点会同时被引擎解锁
    // parent_id：仅供前端编辑器嵌套渲染用
    //
    // 与 p-analysts 的区别：
    //   p-analysts 包裹 9 组 (Tool+Agent) 强调"数据预拉 + 分析"两阶段
    //   p-risk-assess 包裹纯 Agent，强调"同输入多视角并行评估"
    //
    // ⚠️ 坐标约定（FIX: 所有节点位置为画布绝对坐标）：
    //   容器 p-risk-assess 放在 (RISK_X, RISK_Y)
    //   子节点 x = RISK_X + 20px, y = RISK_Y + 40px + i*180px
    //   前端 WorkflowEditor 通过 parentId 减去容器坐标得到相对坐标。
    // ═══════════════════════════════════════════════════════════════════════
    const RISK_X: f64 = 300.0;
    const RISK_Y: f64 = 1800.0;
    nodes.push(WorkflowNode::Parallel(ParallelNode {
        base: WorkflowNodeBase {
            id: "p-risk-assess".into(),
            // F-3 修复: 原本 title="风险评估" 与下面的 t-risk (compute_portfolio_risk) 同名，
            // 编辑器画布上无法区分视觉分组与单 tool。改为"三档风险评估分组"。
            title: "三档风险评估分组".into(),
            description: Some("三种风险偏好并行评估".into()),
            position: Position {
                x: RISK_X,
                y: RISK_Y,
            },
            retry: RetryConfig::default(),
            timeout: Some(600),
            enabled: true,
            parent_id: None,
            compensation: None,
        },
        config: ParallelNodeConfig {
            branches: vec![
                Branch {
                    id: "risk-agg".into(),
                    title: "激进评估".into(),
                    steps: vec!["risk-agg".into()],
                    branch_timeout_ms: None,
                    degrade_strategy: Default::default(),
                },
                Branch {
                    id: "risk-con".into(),
                    title: "保守评估".into(),
                    steps: vec!["risk-con".into()],
                    branch_timeout_ms: None,
                    degrade_strategy: Default::default(),
                },
                Branch {
                    id: "risk-neu".into(),
                    title: "中性评估".into(),
                    steps: vec!["risk-neu".into()],
                    branch_timeout_ms: None,
                    degrade_strategy: Default::default(),
                },
            ],
            wait_for_all: true,
            aggregation: Some(MergeStrategy::All),
            auto_input_from_parent: false,
            timeout: Some(600),
            sub_graph: None, // v23+：稍后通过 inject_container_subgraphs 注入
        },
    }));
    edges.push(edge(
        "e-debate-p-risk-assess",
        &format!("bear-r{debate_max_rounds}"),
        "p-risk-assess",
    ));

    for (i, (rid, rtitle, rexpert, rtools)) in [
        (
            "risk-agg",
            "以最激进的风险偏好评估该股票",
            "aggressive-debator",
            vec![
                td_score.clone(),
                td_risk.clone(),
                td_maxdd.clone(),
                td_var.clone(),
                td_kelly.clone(),
                td_mc.clone(),
            ],
        ),
        (
            "risk-con",
            "以最保守的风险偏好评估该股票",
            "conservative-debator",
            vec![
                td_score.clone(),
                td_risk.clone(),
                td_sharpe.clone(),
                td_maxdd.clone(),
                td_pledge.clone(),
                td_corr.clone(),
            ],
        ),
        (
            "risk-neu",
            "以中性风险偏好评估该股票",
            "neutral-debator",
            vec![
                td_score.clone(),
                td_risk.clone(),
                td_val.clone(),
                td_pe_pct.clone(),
                td_peg.clone(),
                td_rp.clone(),
                td_ind.clone(),
            ],
        ),
    ]
    .iter()
    .enumerate()
    {
        let risk_y = RISK_Y + 40.0 + i as f64 * 180.0;
        let risk_x = RISK_X + 20.0;
        let mut an = agent(rid, rtitle, rexpert, Some("p-risk-assess"), risk_x, risk_y);
        if let WorkflowNode::Agent(ref mut a) = an {
            a.config.tools = rtools.clone();
            a.config.max_tool_rounds = Some(2);
            a.config.system_prompt = format!("{}{}", a.config.system_prompt, tool_prompt(rtools));
            a.config.model_role = Some("risk-evaluator".into());
            // 修复：风险评估 Agent 需要读到上游分析师报告 + 辩论结果 + 技术指标，
            // 否则 LLM 没有分析素材，不会主动调用工具。
            a.config.context_sources = vec![
                "a-market-analyst".into(),
                "a-sentiment".into(),
                "a-news".into(),
                "a-fundamentals".into(),
                "a-policy".into(),
                "a-hot-money".into(),
                "a-lockup".into(),
                "a-research".into(),
                "a-sector".into(),
                "a-catalyst".into(),
                format!("bull-r{debate_max_rounds}"),
                format!("bear-r{debate_max_rounds}"),
                "debate-convergence".into(),
                "t-scoring".into(),
                "t-valuation".into(),
            ];
            a.config.input_mapping = {
                let mut m = build_analyst_input_mapping(&a_ids);
                // 注入辩论收敛的 consensus_score 供 Kelly 公式使用
                m.insert(
                    "consensus_score".to_string(),
                    "debate-convergence.params.consensus_score".to_string(),
                );
                m
            };
        }
        nodes.push(an);
        // p-risk-assess 容器 → 子节点依赖边：防止子节点被独立调度
        edges.push(edge(&format!("e-p-risk-{rid}"), "p-risk-assess", rid));
    }

    // ── AggregatorNode: 聚合三种风险偏好评估结果 ──
    nodes.push(WorkflowNode::Aggregator(AggregatorNode {
        base: WorkflowNodeBase {
            id: "agg-risk".into(),
            title: "风险偏好聚合".into(),
            description: Some("聚合激进/保守/中性三种风险偏好评估".into()),
            position: Position {
                x: 300.0,
                y: 2400.0,
            },
            retry: RetryConfig::default(),
            timeout: Some(60),
            enabled: true,
            parent_id: None,
            compensation: None,
        },
        config: AggregatorNodeConfig {
            strategy: "all".into(),
            input_sources: vec!["risk-agg".into(), "risk-con".into(), "risk-neu".into()],
            output_var: "risk-aggregated".into(),
            wait_for_all: true,
            weights: vec![],
            summarize_prompt: None,
            summarize_model: None,
            sub_graph: None,
        },
    }));
    for rid in &["risk-agg", "risk-con", "risk-neu"] {
        edges.push(edge(&format!("e-{rid}-agg-risk"), rid, "agg-risk"));
    }

    // ── 算法 Tool 节点：仅 3 个核心评分/估值/风控（独立画布节点，parent_id = None）──
    // 位置：agg-risk 节点 (300, 2400) 之后横排，间距 180
    let algo_tools: &[(&str, &str, &str, &str, f64, f64)] = &[
        ("t-scoring", "技术评分", "compute_scoring", "stock_code", 300.0, 2700.0),
        ("t-valuation", "估值计算", "compute_valuation", "stock_code", 480.0, 2700.0),
        // F-3 修复: title 由 "风险评估" 改为 "组合风险计算"，避免与
        // 上面的 p-risk-assess 容器（"三档风险评估分组"）同名混淆。
        ("t-risk", "组合风险计算", "compute_portfolio_risk", "stock_codes", 660.0, 2700.0),
    ];
    for (tool_id, title, tool_name, arg_key, x, y) in algo_tools {
        nodes.push(tool_node(tool_id, title, tool_name, tool_id, arg_key, None, *x, *y));
    }
    edges.push(edge("e-agg-risk-t-scoring", "agg-risk", "t-scoring"));
    edges.push(edge("e-t-scoring-t-valuation", "t-scoring", "t-valuation"));
    edges.push(edge("e-t-valuation-t-risk", "t-valuation", "t-risk"));

    // ── P3 (real-nodes): raw-data 聚合节点 ──
    // 把 13 个 t-* / algo 工具节点的输出聚合成单个 raw 对象，供 portfolio-mgr 决策时
    // 通过 context_sources 读取 "raw-data-aggregated" 变量。
    //
    // F-5 修复: 显式追加 e-raw-data-portfolio-mgr 边。
    //   原设计 raw-data 入度 12、出度 0，仅靠 portfolio-mgr.context_sources 消费。
    //   1) 上游 validate_workflow 会把 raw-data 标为"data_blackhole"硬错误
    //   2) 画布上 raw-data 与 portfolio-mgr 之间无连线，可视化上看像断头
    //   aggregator 节点本身是纯数据合并（不调 LLM），调度等待成本可忽略；
    //   加边后 portfolio-mgr 启动前的等待时间依然是 max(trader, raw-data)，
    //   raw-data 远快于 trader，无可观察的延迟变化。
    let raw_input_sources: Vec<String> = algo_tools
        .iter()
        .map(|(id, _, _, _, _, _)| id.to_string())
        .chain(tool_assignments.iter().map(|(id, _, _, _)| id.to_string()))
        .collect();
    nodes.push(WorkflowNode::Aggregator(AggregatorNode {
        base: WorkflowNodeBase {
            id: "raw-data".into(),
            title: "原始数据聚合".into(),
            description: Some("聚合 13 个工具节点的原始输出（10 个数据源 + 3 个算法）".into()),
            position: Position {
                x: 840.0,
                y: 2700.0,
            },
            retry: RetryConfig::default(),
            timeout: Some(30),
            enabled: true,
            parent_id: None,
            compensation: None,
        },
        config: AggregatorNodeConfig {
            strategy: "all".into(),
            input_sources: raw_input_sources,
            output_var: "raw-data-aggregated".into(),
            wait_for_all: true,
            weights: vec![],
            summarize_prompt: None,
            summarize_model: None,
            sub_graph: None,
        },
    }));
    // 修复 Defect #8: 为 raw-data 显式添加 13 个 tool 节点的入边。
    // 之前只有 1 条 e-t-risk-raw-data 边，依赖关系是"隐性"的（依赖 t-risk 是
    // 13 个 tool 节点中最深的间接前置）。改成显式声明后，调度器会等待所有 13
    // 个上游 tool 节点都完成才启动 raw-data，input_sources 才有数据可读。
    // 迭代器自然包含 e-t-risk-raw-data（来自 algo_tools 末项）。
    for src in algo_tools
        .iter()
        .map(|(id, _, _, _, _, _)| *id)
        .chain(tool_assignments.iter().map(|(id, _, _, _)| *id))
    {
        edges.push(edge(&format!("e-{src}-raw-data"), src, "raw-data"));
    }
    // F-5: 显式出边到 portfolio-mgr，让上游 validate_workflow 的"data_blackhole"
    //      规则不再误报，同时让画布上能看到 raw-data → portfolio-mgr 的连线。
    //      注意：portfolio-mgr.config.context_sources 仍保留 "raw-data"，不影响
    //      数据读取路径，只补一条调度提示边。
    edges.push(edge("e-raw-data-portfolio-mgr", "raw-data", "portfolio-mgr"));

    // ── LlmClassifierNode: 风险等级分类 ──
    nodes.push(WorkflowNode::LlmClassifier(LlmClassifierNode {
        base: WorkflowNodeBase {
            id: "cls-risk-level".into(),
            title: "风险等级分类".into(),
            description: Some("基于算法评分结果自动分类风险等级".into()),
            position: Position {
                x: 300.0,
                y: 3000.0,
            },
            retry: RetryConfig::default(),
            timeout: Some(30),
            enabled: true,
            parent_id: None,
            compensation: None,
        },
        config: LlmClassifierNodeConfig {
            categories: vec!["低风险".into(), "中风险".into(), "高风险".into()],
            prompt: "根据技术评分、估值计算和风险评估的输出结果，判断该股票的整体风险等级。\
                     低风险：评分>70、估值合理、风险指标正常；\
                     中风险：评分40-70或部分指标异常；\
                     高风险：评分<40或多个风险指标触发"
                .into(),
            model: None,
            // v10: input_var 从 "t-risk.result" 改为 "t-risk"，使用完整工具输出
            // 让 LLM 直接读取 tool_name/result/truncated 全貌，不再依赖深层字段下钻
            input_var: "t-risk".into(),
            output_var: "risk-level".into(),
            confidence_threshold: None,
            fallback_label: None,
            consistency_check: None,
        },
    }));
    edges.push(edge("e-t-risk-cls-risk", "t-risk", "cls-risk-level"));

    // ── Validation: 结果完整性校验 ──
    nodes.push(WorkflowNode::Validation(ValidationNode {
        base: WorkflowNodeBase {
            id: "v-validate".into(),
            title: "结果完整性校验".into(),
            description: Some("确保分析报告包含必要字段，缺失时降级处理".into()),
            position: Position {
                x: 300.0,
                y: 3300.0,
            },
            retry: RetryConfig::default(),
            timeout: Some(60),
            enabled: true,
            parent_id: None,
            compensation: None,
        },
        config: ValidationNodeConfig {
            assertions: vec![ValidationAssertion {
                assertion_type: "exists".into(),
                expected: None,
                actual: Some("t-risk.output".into()),
                expression: None,
            }],
            on_fail: "skip".into(),
            max_retries: 1,
        },
    }));
    edges.push(edge("e-cls-risk-v-validate", "cls-risk-level", "v-validate"));

    // ── P3 (real-nodes): data-quality 数据质量检查 Agent ──
    // 等待 v-validate 完成（在 cls-risk-level + t-* algo 全跑完后），
    // 然后评估所有分析师报告的覆盖度、字数、占位检测、一致性。
    // 与 research-mgr 并行启动，输出通过 portfolio-mgr.context_sources 注入
    // 最终决策（见 portfolio-mgr 节点的 context_sources 配置）。
    //
    // F-6 修复: data-quality 是有意"仅靠 context_sources 消费"的终态。
    //   data-quality 是慢速 LLM agent（约 5-10s）,与 research-mgr → trader 链路
    //   并行执行。如果加 e-data-quality-portfolio-mgr 显式边,调度器会强制
    //   portfolio-mgr 等待 data-quality 完成,串行化整条路径,引入不必要的延迟。
    //   正确做法是保持 context_sources 消费模式,允许并行。
    //   画布上 data-quality 看似"断头"是预期设计,非真实 bug。
    //
    //   注: 如果未来上游 validate_workflow 把 data-quality 标为 data_blackhole
    //       或 orphan,可考虑给节点加 kind="context_sink" 标记让校验跳过。
    {
        let dq_id = "data-quality";
        let dq_title = "数据质量评估：覆盖度、字数、占位检测，输出 A/B/C/D/F 等级";
        let dq_y = 3300.0;
        let mut dq = agent(dq_id, dq_title, "data-quality-inspector", None, 840.0, dq_y);
        if let WorkflowNode::Agent(ref mut a) = dq {
            a.config.context_sources = vec![
                "a-market-analyst".into(),
                "a-sentiment".into(),
                "a-news".into(),
                "a-fundamentals".into(),
                "a-policy".into(),
                "a-hot-money".into(),
                "a-lockup".into(),
                "a-research".into(),
                "a-sector".into(),
                "a-catalyst".into(),
                // 注入算法工具节点的 credibility 元数据，支持数据质量检查员
                // 评估工具可信度分的 4 个维度（freshness/completeness/warnings/source）
                "t-scoring".into(),
                "t-valuation".into(),
                "t-risk".into(),
            ];
            // ── 结构化参数注入（结构化参数方案 Phase 2）──
            // 注入各分析师的 confidence 结构化值，使 DQI 可直接判断
            // "信心低迷（confidence < 30）" 条件，无需从文本中重新提取。
            a.config.input_mapping = [
                ("mk_confidence", "a-market-analyst.params.confidence"),
                ("sent_confidence", "a-sentiment.params.confidence"),
                ("news_confidence", "a-news.params.confidence"),
                ("fund_confidence", "a-fundamentals.params.confidence"),
                ("pol_confidence", "a-policy.params.confidence"),
                ("hm_confidence", "a-hot-money.params.confidence"),
                ("lk_confidence", "a-lockup.params.confidence"),
                ("res_confidence", "a-research.params.confidence"),
                ("sec_confidence", "a-sector.params.confidence"),
                ("cat_confidence", "a-catalyst.params.confidence"),
                // 注入各分析师的 if_data_gaps 布尔值，无需扫描全文检查缺失项
                ("mk_data_gaps", "a-market-analyst.params.if_data_gaps"),
                ("sent_data_gaps", "a-sentiment.params.if_data_gaps"),
                ("news_data_gaps", "a-news.params.if_data_gaps"),
                ("fund_data_gaps", "a-fundamentals.params.if_data_gaps"),
                ("pol_data_gaps", "a-policy.params.if_data_gaps"),
                ("hm_data_gaps", "a-hot-money.params.if_data_gaps"),
                ("lk_data_gaps", "a-lockup.params.if_data_gaps"),
                ("res_data_gaps", "a-research.params.if_data_gaps"),
                ("sec_data_gaps", "a-sector.params.if_data_gaps"),
                ("cat_data_gaps", "a-catalyst.params.if_data_gaps"),
            ]
            .into_iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
            a.config.model_role = Some("stock-analyst".into());
            let tool_names = PROFILE_TOOLS
                .iter()
                .find(|(k, _)| *k == "data-quality-inspector")
                .map(|(_, v)| *v)
                .unwrap_or(&[]);
            a.config.tools = tool_names
                .iter()
                .filter_map(|&tn| tool_def_map.get(tn).cloned())
                .collect();
            a.config.exposed_tools = tool_names.iter().map(|&tn| tn.to_string()).collect();
            a.config.system_prompt =
                format!("{}{}", a.config.system_prompt, tool_prompt(&a.config.tools));
        }
        nodes.push(dq);
        edges.push(edge("e-v-validate-data-quality", "v-validate", dq_id));
    }

    // research-mgr → trader → portfolio-mgr
    let mut rm = agent(
        "research-mgr",
        "综合风险评估：总体风险评级与主要风险点清单",
        "research-manager",
        None,
        240.0,
        3600.0,
    );
    if let WorkflowNode::Agent(ref mut a) = rm {
        a.config.context_sources = vec![
            "value-investor".into(),
            "t-scoring".into(),
            "t-valuation".into(),
            "t-risk".into(),
            "risk-aggregated".into(),
            "risk-level".into(),
        ];
        // ── 结构化参数注入（结构化参数方案 Phase 2）──
        // 注入风险聚合的结构化评分，使 research-mgr 可在 system_prompt 中
        // 直接使用 risk_level 等值，无需从文本中重新提取。
        a.config.input_mapping = [
            ("overall_risk", "risk-level.params.overall_risk"),
            ("agg_risk_pos", "risk-aggregated.params.aggressive_pct"),
            ("cons_risk_pos", "risk-aggregated.params.conservative_pct"),
            ("neut_risk_pos", "risk-aggregated.params.neutral_pct"),
            ("consensus_score", "debate-convergence.params.consensus_score"),
            ("stock_lessons", "stock_lessons"),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
        a.config.model_role = Some("decision-maker".into());
        a.config.tools = vec![
            td_score.clone(),
            td_val.clone(),
            td_risk.clone(),
            td_maxdd.clone(),
            td_sharpe.clone(),
            td_var.clone(),
            td_pe_pct.clone(),
            td_peg.clone(),
            td_kelly.clone(),
            td_rp.clone(),
            td_corr.clone(),
            td_ind.clone(),
        ];
        a.config.system_prompt =
            format!("{}{}", a.config.system_prompt, tool_prompt(&a.config.tools));
        a.config.max_tool_rounds = Some(3);
        // exposed_tools 排除已由 t-scoring/t-valuation/t-risk 注入的算法工具
        a.config.exposed_tools = a
            .config
            .tools
            .iter()
            .map(|td| td.name.clone())
            .filter(|n| {
                n != "compute_scoring" && n != "compute_valuation" && n != "compute_portfolio_risk"
            })
            .collect();
    }
    nodes.push(rm);
    edges.push(edge("e-value-investor-research-mgr", "value-investor", "research-mgr"));
    edges.push(edge("e-v-validate-research-mgr", "v-validate", "research-mgr"));

    // trader: 执行方案 — 实时行情 + 技术指标 + 凯利仓位
    let mut trader = agent(
        "trader",
        "制定A股交易方案：入场价、目标价、止损价、仓位比例。遵守T+1和涨跌停规则",
        "trader",
        None,
        240.0,
        3900.0,
    );
    if let WorkflowNode::Agent(ref mut a) = trader {
        a.config.context_sources = vec!["research-mgr".into()];
        a.config.model_role = Some("trader".into());
        a.config.tools = vec![
            td_quote.clone(),
            td_kline.clone(),
            td_mf.clone(),
            td_score.clone(),
            td_atr.clone(),
            td_ma_cross.clone(),
            td_breakout.clone(),
            td_kelly.clone(),
            td_mc.clone(),
            td_lup.clone(),
        ];
        a.config.system_prompt =
            format!("{}{}", a.config.system_prompt, tool_prompt(&a.config.tools));
        a.config.max_tool_rounds = Some(3);
        a.config.input_mapping = [
            ("consensus_score", "debate-convergence.params.consensus_score"),
            ("stock_lessons", "stock_lessons"),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
    }
    nodes.push(trader);
    edges.push(edge("e-research-mgr-trader", "research-mgr", "trader"));

    // portfolio-mgr: 最终决策 — 确定性计算（CodeNode + Rhai）
    // ── 结构化参数方案 Phase 3 ──
    // 原为 Agent 节点（LLM 执行公式），现改为 CodeNode（Rhai 确定性执行）。
    //
    // 公式逻辑（与 portfolio-manager prompt 保持一致）：
    //   confidence = clamp(totalScore + adjustment, 0, 100)
    //   adjustment = 共识调整 + 数据质量调整 + 风险调整 + 催化剂加成 + 机构加成
    let pm_code = include_str!("portfolio-mgr.rhai").to_string();
    let pm = WorkflowNode::Code(CodeNode {
        base: WorkflowNodeBase {
            id: "portfolio-mgr".into(),
            title: "投资组合经理（确定性决策）".into(),
            description: Some("基于结构化参数，用确定性公式计算最终决策".into()),
            position: Position {
                x: 240.0,
                y: 4200.0,
            },
            retry: RetryConfig::default(),
            timeout: Some(30),
            enabled: true,
            parent_id: None,
            compensation: None,
        },
        config: CodeNodeConfig {
            language: "rhai".into(),
            code: pm_code,
            output_var: "portfolio-mgr".into(),
            tool_name: None,
            execute_directly: true,
            input_mapping: [
                ("totalScore", "t-scoring.result.totalScore"),
                ("dqi_score", "data-quality.params.score"),
                ("overall_risk", "risk-level.params.overall_risk"),
                ("catalyst_level", "a-catalyst.params.catalyst_level"),
                ("institutional_trace", "a-catalyst.params.institutional_trace"),
                ("consensusScore", "debate-convergence.params.consensus_score"),
                ("trader_time_horizon", "trader.params.timeHorizon"),
                ("trader_holding_days", "trader.params.expectedHoldingDays"),
                (
                    "aggregate_direction",
                    "debate-convergence.params.aggregate_prediction.direction",
                ),
                (
                    "aggregate_confidence",
                    "debate-convergence.params.aggregate_prediction.confidence",
                ),
                // 可选 Bayesian/回测变量：无对应上游节点，resolve 返回 None → safe_num() 走默认值
                ("market_regime", ""),
                ("signal_quality_win_rate", ""),
                ("signal_quality_sample_count", ""),
            ]
            .into_iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect(),
        },
    });
    nodes.push(pm);
    edges.push(edge("e-trader-portfolio-mgr", "trader", "portfolio-mgr"));
    edges.push(edge("e-research-mgr-portfolio-mgr", "research-mgr", "portfolio-mgr"));
    // debate-convergence → portfolio-mgr: 显式边确保 consensus_score 在公式执行前就绪
    edges.push(edge(
        "e-debate-convergence-portfolio-mgr",
        "debate-convergence",
        "portfolio-mgr",
    ));
    // data-quality → portfolio-mgr: 显式边确保 dqi_score 在 Rhai 公式执行前就绪
    edges.push(edge("e-data-quality-portfolio-mgr", "data-quality", "portfolio-mgr"));

    // ── P3 (real-nodes): rule-check 规则检查 Agent ──
    // 在 portfolio-mgr 完成后启动，对照硬性规则阈值（RSI/乖离率/止损/放量下跌/空头排列）
    // 检查交易方案是否违规，输出 violations / corrections / force_signals
    {
        let rc_id = "rule-check";
        let rc_title = "硬性规则检查：RSI超买/乖离率追高/缺失止损/放量下跌/空头排列";
        let rc_y = 4200.0;
        let mut rc = agent(rc_id, rc_title, "rule-checker", None, 700.0, rc_y);
        if let WorkflowNode::Agent(ref mut a) = rc {
            a.config.context_sources = vec![
                "portfolio-mgr".into(),
                "t-scoring".into(),
                "t-valuation".into(),
                "t-risk".into(),
                "trader".into(),
            ];
            a.config.model_role = Some("risk-evaluator".into());
            let tool_names = PROFILE_TOOLS
                .iter()
                .find(|(k, _)| *k == "rule-checker")
                .map(|(_, v)| *v)
                .unwrap_or(&[]);
            a.config.tools = tool_names
                .iter()
                .filter_map(|&tn| tool_def_map.get(tn).cloned())
                .collect();
            a.config.exposed_tools = tool_names.iter().map(|&tn| tn.to_string()).collect();
            a.config.system_prompt =
                format!("{}{}", a.config.system_prompt, tool_prompt(&a.config.tools));
        }
        nodes.push(rc);
        // ── NotificationNode 由 rule-check 完成后触发（不再保留 portfolio-mgr → notify
        // 直连，避免通知在规则检查改写决策之前发出）──
        edges.push(edge("e-portfolio-mgr-rule-check", "portfolio-mgr", rc_id));
        edges.push(edge("e-rule-check-quality-gate", rc_id, "quality-gate"));
        // data-quality → quality-gate: 显式边确保 data-quality 变量在 switch 判断前就绪
        edges.push(edge("e-data-quality-quality-gate", "data-quality", "quality-gate"));
    }

    // ── SwitchNode: 数据质量门禁 ──
    // 检查 data-quality Agent 的输出等级（A/B/C/D/F），C 级以上继续，D/F 走降级路径。
    // data-quality 输出为文本，包含质量等级标签如 "A", "B", "C" 等。
    nodes.push(WorkflowNode::Switch(SwitchNode {
        base: WorkflowNodeBase {
            id: "quality-gate".into(),
            title: "数据质量门禁".into(),
            description: Some("检查数据质量等级，A/B/C 级以上继续，D/F 走保守降级路径".into()),
            position: Position {
                x: 700.0,
                y: 4500.0,
            },
            retry: RetryConfig::default(),
            timeout: Some(10),
            enabled: true,
            parent_id: None,
            compensation: None,
        },
        config: SwitchNodeConfig {
            input_var: "data-quality.params.grade".into(),
            cases: vec![SwitchCase {
                value: "_value == \"A\" || _value == \"B\" || _value == \"C\"".into(),
                label: "acceptable".into(),
            }],
            default_case: Some("low-quality".into()),
            match_mode: "expression".into(),
            use_llm: None,
            llm_prompt: None,
            llm_model: None,
            output_var: "quality-gate-result".into(),
        },
    }));

    // ── Agent: 降级处理路径（数据质量不足时生成保守决策）──
    {
        let fq_id = "quality-fallback";
        let fq_title = "数据不足→保守决策：持仓不变/减仓观望";
        let fq_y = 4500.0;
        let mut fq = agent(
            fq_id,
            fq_title,
            "value-investor", // 仅用于占位，system_prompt 会完全覆盖
            None,
            20.0,
            fq_y,
        );
        if let WorkflowNode::Agent(ref mut a) = fq {
            a.config.context_sources = vec![
                "rule-check".into(),
                "data-quality".into(),
                "t-scoring".into(),
                "t-valuation".into(),
                "t-risk".into(),
            ];
            a.config.output_mode = OutputMode::Json;
            a.config.model_role = Some("decision-maker".into());
            a.config.tools = vec![td_quote.clone(), td_kline.clone(), td_score.clone()];
            a.config.system_prompt =
                "数据质量评估为 D 或 F，上游分析数据不可靠。你需要在数据不足的情况下做出最保守的投资决策。\
                 输出JSON格式（严格模式）：{\"action\":\"持有/减持/卖出\",\"positionPct\":0-20,\"reasoning\":\"保守决策理由\"}}\
                 只输出上述JSON对象，前后不要有任何其他文字"
                    .to_string();
            a.config.exposed_tools = vec![
                "get_stock_quote".into(),
                "get_stock_kline".into(),
                "compute_scoring".into(),
            ];
            a.config.max_tool_rounds = Some(1);
        }
        nodes.push(fq);
        // Switch 出边：
        //   case "acceptable" → notify-result（source_handle = 匹配的 case label）
        //   default → quality-fallback（无 source_handle）
        edges.push(WorkflowEdge {
            id: "e-quality-gate-notify".into(),
            source: "quality-gate".into(),
            source_handle: Some("acceptable".into()),
            target: "notify-result".into(),
            target_handle: None,
            edge_type: EdgeType::Direct,
            label: Some("通过 ✓".into()),
        });
        edges.push(WorkflowEdge {
            id: "e-quality-gate-quality-fallback".into(),
            source: "quality-gate".into(),
            source_handle: None,
            target: fq_id.into(),
            target_handle: None,
            edge_type: EdgeType::Direct,
            label: Some("降级 →".into()),
        });
    }
    // quality-fallback 降级完成后同样触发通知
    edges.push(edge("e-quality-fallback-notify", "quality-fallback", "notify-result"));

    // ── NotificationNode: 分析完成通知 ──
    nodes.push(WorkflowNode::Notification(NotificationNode {
        base: WorkflowNodeBase {
            id: "notify-result".into(),
            title: "分析完成通知".into(),
            description: Some("股票分析完成后发送通知".into()),
            position: Position {
                x: 300.0,
                y: 4500.0,
            },
            retry: RetryConfig::default(),
            timeout: Some(10),
            enabled: true,
            parent_id: None,
            compensation: None,
        },
        config: NotificationNodeConfig {
            channel: "system".into(),
            message: "股票分析已完成，请查看决策结果".into(),
            webhook_url: None,
            recipients: vec![],
            subject: Some("股票分析完成".into()),
            enabled: true,
            output_var: "notification".into(),
        },
    }));
    // 注：移除 e-portfolio-mgr-notify 直连，notify-result 现在仅由 rule-check 完成后触发

    // ── StorageNode: 分析结果持久化 ──
    // 将完整分析结果（portfolio-mgr 决策）写入 SQLite history 表，供后续回测/复盘引用。
    nodes.push(WorkflowNode::Storage(StorageNode {
        base: WorkflowNodeBase {
            id: "store-result".into(),
            title: "分析结果持久化".into(),
            description: Some("写入分析结果到历史记录表".into()),
            position: Position {
                x: 300.0,
                y: 4800.0,
            },
            retry: RetryConfig {
                enabled: true,
                max_retries: 2,
                ..Default::default()
            },
            timeout: Some(30),
            enabled: true,
            parent_id: None,
            compensation: None,
        },
        config: StorageNodeConfig {
            backend: "sqlite".into(),
            operation: "insert".into(),
            input_var: "portfolio-mgr".into(),
            collection: "analysis_history".into(),
            key_var: None,
            output_var: "storage-result".into(),
        },
    }));
    edges.push(edge("e-notify-store-result", "notify-result", "store-result"));
    // store-result 直接从 portfolio-mgr 取决策变量，绕过 state.variables 查找
    edges.push(edge("e-portfolio-mgr-store-result", "portfolio-mgr", "store-result"));

    // EndNode: 把 portfolio-mgr 输出提升为工作流顶层输出
    nodes.push(WorkflowNode::End(EndNode {
        base: WorkflowNodeBase {
            id: "end-output".into(),
            title: "最终输出".into(),
            description: Some("将 portfolio-mgr 决策结果提升到工作流输出".into()),
            position: Position {
                x: 300.0,
                y: 5100.0,
            },
            retry: RetryConfig::default(),
            timeout: None,
            enabled: true,
            parent_id: None,
            compensation: None,
        },
        config: EndNodeConfig {
            output_var: Some("portfolio-mgr".into()),
        },
    }));
    edges.push(edge("e-store-end", "store-result", "end-output"));

    // 构建 input_schema / output_schema / variables
    let mut input_props = std::collections::HashMap::new();
    input_props.insert(
        "stock_code".to_string(),
        JsonSchemaProperty {
            schema_type: "string".to_string(),
            description: Some("股票代码，如 000001、600519".to_string()),
            default: None,
            enum_values: None,
            format: None,
        },
    );
    let input_schema_val = serde_json::to_string(&JsonSchema {
        schema_type: "object".to_string(),
        description: Some("股票分析运行时输入".to_string()),
        properties: Some(input_props),
        required: Some(vec!["stock_code".to_string()]),
        items: None,
    })
    .unwrap();

    let mut output_props = std::collections::HashMap::new();
    output_props.insert(
        "action".to_string(),
        JsonSchemaProperty {
            schema_type: "string".to_string(),
            description: Some("投资决策: 买入/增持/持有/减持/卖出".to_string()),
            default: None,
            enum_values: None,
            format: None,
        },
    );
    output_props.insert(
        "positionPct".to_string(),
        JsonSchemaProperty {
            schema_type: "number".to_string(),
            description: Some("建议仓位百分比 (0-100)".to_string()),
            default: None,
            enum_values: None,
            format: None,
        },
    );
    output_props.insert(
        "targetPrice".to_string(),
        JsonSchemaProperty {
            schema_type: "number".to_string(),
            description: Some("目标价".to_string()),
            default: None,
            enum_values: None,
            format: None,
        },
    );
    output_props.insert(
        "stopLoss".to_string(),
        JsonSchemaProperty {
            schema_type: "number".to_string(),
            description: Some("止损价".to_string()),
            default: None,
            enum_values: None,
            format: None,
        },
    );
    output_props.insert(
        "reasoning".to_string(),
        JsonSchemaProperty {
            schema_type: "string".to_string(),
            description: Some("决策理由 (300字以内)".to_string()),
            default: None,
            enum_values: None,
            format: None,
        },
    );
    output_props.insert(
        "riskLevel".to_string(),
        JsonSchemaProperty {
            schema_type: "string".to_string(),
            description: Some("风险等级: 低/中/高".to_string()),
            default: None,
            enum_values: None,
            format: None,
        },
    );
    output_props.insert(
        "confidence".to_string(),
        JsonSchemaProperty {
            schema_type: "number".to_string(),
            description: Some("置信度 (0-100)".to_string()),
            default: None,
            enum_values: None,
            format: None,
        },
    );
    let output_schema_val = serde_json::to_string(&JsonSchema {
        schema_type: "object".to_string(),
        description: Some("股票分析最终决策输出".to_string()),
        properties: Some(output_props),
        required: None,
        items: None,
    })
    .unwrap();

    let variables: Vec<Variable> = vec![
        // ── 分析流程参数 ──
        Variable {
            name: "analysis_depth".into(),
            var_type: "enum".into(),
            value: serde_json::json!("standard"),
            description: Some("分析深度: quick / standard / deep".into()),
            is_secret: false,
        },
        Variable {
            name: "debate_rounds".into(),
            var_type: "number".into(),
            // 与 seed 时使用的常量保持一致（seed 函数里硬编码 3）。
            // 用户在「股票分析设置 → 参数」里调成 6 后，下次模板升级会
            // 用 merge_variable_values 保留用户的 6，并据此展开 DAG。
            value: serde_json::json!(3),
            description: Some("多空辩论轮数 (1-10)".into()),
            is_secret: false,
        },
        Variable {
            name: "max_concurrent".into(),
            var_type: "number".into(),
            // 修复 Defect #6: 与 stock-analyst 角色 max_concurrent=12 对齐，
            // 留 1 槽位余量；旧值 9 不足以同时调度 9 个分析师 + value-investor + data-quality。
            value: serde_json::json!(12),
            description: Some("并行分析的 Agent 数量上限".into()),
            is_secret: false,
        },
        // ── 数据源参数 ──
        Variable {
            name: "kline_period".into(),
            var_type: "enum".into(),
            value: serde_json::json!("daily"),
            description: Some("K线周期: daily / weekly / monthly".into()),
            is_secret: false,
        },
        Variable {
            name: "kline_limit".into(),
            var_type: "number".into(),
            value: serde_json::json!(120),
            description: Some("K线获取根数 (1-500)".into()),
            is_secret: false,
        },
        Variable {
            name: "news_limit".into(),
            var_type: "number".into(),
            value: serde_json::json!(30),
            description: Some("新闻获取条数 (1-100)".into()),
            is_secret: false,
        },
        // ── Agent 节点 LLM 参数 ──
        Variable {
            name: "agent_temperature".into(),
            var_type: "number".into(),
            value: serde_json::json!(0.3),
            description: Some("所有 Agent 节点 LLM 温度 (0-2)".into()),
            is_secret: false,
        },
        Variable {
            name: "agent_max_tokens".into(),
            var_type: "number".into(),
            value: serde_json::json!(4096),
            description: Some("所有 Agent 节点最大输出 token 数".into()),
            is_secret: false,
        },
        Variable {
            name: "agent_timeout_secs".into(),
            var_type: "number".into(),
            value: serde_json::json!(300),
            description: Some("每个 Agent 节点执行超时秒数".into()),
            is_secret: false,
        },
        Variable {
            name: "agent_retry_max".into(),
            var_type: "number".into(),
            value: serde_json::json!(2),
            description: Some("每个 Agent 节点最大重试次数".into()),
            is_secret: false,
        },
        // ── Tool 节点参数 ──
        Variable {
            name: "tool_timeout_secs".into(),
            var_type: "number".into(),
            value: serde_json::json!(30),
            description: Some("每个 Tool 节点执行超时秒数".into()),
            is_secret: false,
        },
        Variable {
            name: "tool_retry_max".into(),
            var_type: "number".into(),
            value: serde_json::json!(2),
            description: Some("每个 Tool 节点最大重试次数".into()),
            is_secret: false,
        },
        // ── 评分权重 (ScoringWeights) ──
        Variable {
            name: "scoring_trend".into(),
            var_type: "number".into(),
            value: serde_json::json!(30.0),
            description: Some("趋势评分权重 (0-100)".into()),
            is_secret: false,
        },
        Variable {
            name: "scoring_deviation".into(),
            var_type: "number".into(),
            value: serde_json::json!(20.0),
            description: Some("偏离度评分权重 (0-100)".into()),
            is_secret: false,
        },
        Variable {
            name: "scoring_macd".into(),
            var_type: "number".into(),
            value: serde_json::json!(15.0),
            description: Some("MACD 评分权重 (0-100)".into()),
            is_secret: false,
        },
        Variable {
            name: "scoring_volume".into(),
            var_type: "number".into(),
            value: serde_json::json!(15.0),
            description: Some("成交量评分权重 (0-100)".into()),
            is_secret: false,
        },
        Variable {
            name: "scoring_rsi".into(),
            var_type: "number".into(),
            value: serde_json::json!(10.0),
            description: Some("RSI 评分权重 (0-100)".into()),
            is_secret: false,
        },
        Variable {
            name: "scoring_support".into(),
            var_type: "number".into(),
            value: serde_json::json!(10.0),
            description: Some("支撑阻力评分权重 (0-100)".into()),
            is_secret: false,
        },
        // 补全：decision.rs:75 的 ScoringWeights 里有这个字段，但模板里漏了种子化
        Variable {
            name: "scoring_boll".into(),
            var_type: "number".into(),
            value: serde_json::json!(5.0),
            description: Some("布林带评分权重 (0-100)".into()),
            is_secret: false,
        },
        // ── 规则引擎阈值 (RuleConfig) ──
        Variable {
            name: "rule_rsi_overbought".into(),
            var_type: "number".into(),
            value: serde_json::json!(80.0),
            description: Some("RSI 超买阈值".into()),
            is_secret: false,
        },
        Variable {
            name: "rule_rsi_oversold".into(),
            var_type: "number".into(),
            value: serde_json::json!(20.0),
            description: Some("RSI 超卖阈值".into()),
            is_secret: false,
        },
        Variable {
            name: "rule_bias_limit_pct".into(),
            var_type: "number".into(),
            value: serde_json::json!(5.0),
            description: Some("均线偏离极限 (%)".into()),
            is_secret: false,
        },
        Variable {
            name: "rule_volume_signal_block".into(),
            var_type: "boolean".into(),
            value: serde_json::json!(true),
            description: Some("成交量异常时是否阻塞信号".into()),
            is_secret: false,
        },
        Variable {
            name: "rule_bear_low_score".into(),
            var_type: "number".into(),
            value: serde_json::json!(30),
            description: Some("空方低分阈值 (低于此分数触发警告)".into()),
            is_secret: false,
        },
        Variable {
            name: "rule_auto_stop_loss_pct".into(),
            var_type: "number".into(),
            value: serde_json::json!(5.0),
            description: Some("自动止损线 (%)".into()),
            is_secret: false,
        },
        // ── 仓位限制 (PositionLimitsConfig) ──
        Variable {
            name: "pos_max_single_pct".into(),
            var_type: "number".into(),
            value: serde_json::json!(20.0),
            description: Some("单只股票最大仓位占比 (%)".into()),
            is_secret: false,
        },
        Variable {
            name: "pos_max_total".into(),
            var_type: "number".into(),
            value: serde_json::json!(10),
            description: Some("最大持仓数量".into()),
            is_secret: false,
        },
        Variable {
            name: "pos_max_sector_pct".into(),
            var_type: "number".into(),
            value: serde_json::json!(40.0),
            description: Some("最大行业暴露占比 (%)".into()),
            is_secret: false,
        },
        // ── 估值参数 (ValueConfig) ──
        Variable {
            name: "value_dcf_growth_rate".into(),
            var_type: "number".into(),
            value: serde_json::json!(8.0),
            description: Some("DCF 增长率 (%)".into()),
            is_secret: false,
        },
        Variable {
            name: "value_dcf_perpetual_rate".into(),
            var_type: "number".into(),
            value: serde_json::json!(3.0),
            description: Some("DCF 永续增长率 (%)".into()),
            is_secret: false,
        },
        Variable {
            name: "value_dcf_discount_rate".into(),
            var_type: "number".into(),
            value: serde_json::json!(10.0),
            description: Some("DCF 折现率 (%)".into()),
            is_secret: false,
        },
        Variable {
            name: "value_moat_threshold".into(),
            var_type: "number".into(),
            value: serde_json::json!(60),
            description: Some("护城河评分阈值 (0-100)".into()),
            is_secret: false,
        },
        Variable {
            name: "value_fscore_buy".into(),
            var_type: "number".into(),
            value: serde_json::json!(7),
            description: Some("F-Score 买入阈值 (0-9)".into()),
            is_secret: false,
        },
        Variable {
            name: "value_safety_margin".into(),
            var_type: "number".into(),
            value: serde_json::json!(20.0),
            description: Some("安全边际最低折扣 (%)".into()),
            is_secret: false,
        },
        // ── 监控参数 (MonitorConfig) ──
        Variable {
            name: "monitor_poll_interval_secs".into(),
            var_type: "number".into(),
            value: serde_json::json!(30),
            description: Some("监控轮询间隔秒数".into()),
            is_secret: false,
        },
        Variable {
            name: "monitor_change_pct".into(),
            var_type: "number".into(),
            value: serde_json::json!(5.0),
            description: Some("价格异动提醒阈值 (%)".into()),
            is_secret: false,
        },
        Variable {
            name: "monitor_turnover".into(),
            var_type: "number".into(),
            value: serde_json::json!(10.0),
            description: Some("换手率异动提醒阈值 (%)".into()),
            is_secret: false,
        },
        // ── 置信度参数 ──
        Variable {
            name: "min_confidence".into(),
            var_type: "number".into(),
            value: serde_json::json!(60),
            description: Some("最低置信度阈值 (低于此值建议观望)".into()),
            is_secret: false,
        },
        // ── 数据源 (vendor_ 前缀，健康检查关联) ──
        Variable {
            name: "vendor_tencent".into(),
            var_type: "boolean".into(),
            value: serde_json::json!(true),
            description: Some("腾讯财经 — 报价数据".into()),
            is_secret: false,
        },
        Variable {
            name: "vendor_eastmoney".into(),
            var_type: "boolean".into(),
            value: serde_json::json!(true),
            description: Some("东方财富 — 财务/K线数据".into()),
            is_secret: false,
        },
        Variable {
            name: "vendor_sina".into(),
            var_type: "boolean".into(),
            value: serde_json::json!(true),
            description: Some("新浪财经 — 新闻数据".into()),
            is_secret: false,
        },
        Variable {
            name: "vendor_ths".into(),
            var_type: "boolean".into(),
            value: serde_json::json!(true),
            description: Some("同花顺 — 综合数据".into()),
            is_secret: false,
        },
        Variable {
            name: "vendor_cninfo".into(),
            var_type: "boolean".into(),
            value: serde_json::json!(true),
            description: Some("巨潮资讯 — 信息披露".into()),
            is_secret: false,
        },
        Variable {
            name: "vendor_baidu_stock".into(),
            var_type: "boolean".into(),
            value: serde_json::json!(true),
            description: Some("百度股票 — 数据".into()),
            is_secret: false,
        },
        Variable {
            name: "vendor_iwencai".into(),
            var_type: "boolean".into(),
            value: serde_json::json!(true),
            description: Some("问财 — 选股数据".into()),
            is_secret: false,
        },
        Variable {
            name: "vendor_akshare".into(),
            var_type: "boolean".into(),
            value: serde_json::json!(true),
            description: Some("AKShare — 开源数据".into()),
            is_secret: false,
        },
        Variable {
            name: "vendor_mootdx".into(),
            var_type: "boolean".into(),
            value: serde_json::json!(true),
            description: Some("Mootdx — 本地行情接口".into()),
            is_secret: false,
        },
        // ── 金融模型参数（通过 prompt 模板 {{var}} 传入 agent）──
        Variable {
            name: "risk_free_rate".into(),
            var_type: "number".into(),
            value: serde_json::json!(0.03),
            description: Some("无风险利率".into()),
            is_secret: false,
        },
        Variable {
            name: "var_confidence".into(),
            var_type: "number".into(),
            value: serde_json::json!(0.95),
            description: Some("VaR 置信度 (0-1)".into()),
            is_secret: false,
        },
        Variable {
            name: "outlier_method".into(),
            var_type: "enum".into(),
            value: serde_json::json!("zscore"),
            description: Some("异常值检测方法: zscore / iqr".into()),
            is_secret: false,
        },
        Variable {
            name: "outlier_threshold".into(),
            var_type: "number".into(),
            value: serde_json::json!(2.0),
            description: Some("异常值 Z-score 阈值".into()),
            is_secret: false,
        },
        Variable {
            name: "kelly_fraction".into(),
            var_type: "number".into(),
            value: serde_json::json!(0.5),
            description: Some("凯利仓位系数 (建议仓位 = half_kelly × 此系数)".into()),
            is_secret: false,
        },
        // ── A 类补全：凯利前置条件（risk.rs:188-198）──
        Variable {
            name: "kelly_min_win_rate".into(),
            var_type: "number".into(),
            value: serde_json::json!(0.4),
            description: Some("凯利最低胜率要求 (0-1)，低于此值返回不适用".into()),
            is_secret: false,
        },
        Variable {
            name: "kelly_min_odds".into(),
            var_type: "number".into(),
            value: serde_json::json!(1.0),
            description: Some("凯利最低赔率要求 (avg_win/avg_loss)，低于此值降权".into()),
            is_secret: false,
        },
        // ── A 类补全：组合风控（trading.rs:200 / risk.rs）──
        Variable {
            name: "risk_max_drawdown_limit".into(),
            var_type: "number".into(),
            value: serde_json::json!(15.0),
            description: Some("组合最大回撤熔断线 (%)，超过则暂停新开仓".into()),
            is_secret: false,
        },
        Variable {
            name: "risk_max_daily_loss_pct".into(),
            var_type: "number".into(),
            value: serde_json::json!(3.0),
            description: Some("单日最大亏损 (%)，超过则停手".into()),
            is_secret: false,
        },
        Variable {
            name: "risk_correlation_lookback_days".into(),
            var_type: "number".into(),
            value: serde_json::json!(60),
            description: Some("风险平价 / 相关性矩阵的回看窗口 (交易日)".into()),
            is_secret: false,
        },
        // ── A 类补全：仓位限制扩展（position_limits.rs）──
        Variable {
            name: "pos_min_cash_pct".into(),
            var_type: "number".into(),
            value: serde_json::json!(5.0),
            description: Some("最低现金比例 (%)，低于则禁止新开仓".into()),
            is_secret: false,
        },
        Variable {
            name: "pos_max_turnover_pct".into(),
            var_type: "number".into(),
            value: serde_json::json!(100.0),
            description: Some("单期最大换手率 (%)，超过则分批调仓".into()),
            is_secret: false,
        },
        // ── A 类补全：护城河量化阈值（value.rs:320-434）──
        Variable {
            name: "moat_roe_years_min".into(),
            var_type: "number".into(),
            value: serde_json::json!(3),
            description: Some("ROE>15% 最少连续年数 (0-10)".into()),
            is_secret: false,
        },
        Variable {
            name: "moat_avg_gross_margin_min".into(),
            var_type: "number".into(),
            value: serde_json::json!(20.0),
            description: Some("平均毛利率下限 (%)".into()),
            is_secret: false,
        },
        Variable {
            name: "moat_margin_stable_std_max".into(),
            var_type: "number".into(),
            value: serde_json::json!(5.0),
            description: Some("毛利率稳定性标准差上限 (σ，%)".into()),
            is_secret: false,
        },
        Variable {
            name: "moat_fcf_ratio_min".into(),
            var_type: "number".into(),
            value: serde_json::json!(0.5),
            description: Some("FCF/净利润 比率下限 (0-1)".into()),
            is_secret: false,
        },
        // ── A 类补全：选股筛选（screener.rs:8 ScreenCriteria）──
        Variable {
            name: "screener_min_change_pct".into(),
            var_type: "number".into(),
            value: serde_json::json!(-30.0),
            description: Some("选股最小涨跌幅下限 (%)".into()),
            is_secret: false,
        },
        Variable {
            name: "screener_max_change_pct".into(),
            var_type: "number".into(),
            value: serde_json::json!(30.0),
            description: Some("选股最大涨跌幅上限 (%)".into()),
            is_secret: false,
        },
        Variable {
            name: "screener_main_inflow_min".into(),
            var_type: "number".into(),
            value: serde_json::json!(0.0),
            description: Some("主力净流入下限 (万元)，0=不限".into()),
            is_secret: false,
        },
        Variable {
            name: "screener_northbound_ratio_min".into(),
            var_type: "number".into(),
            value: serde_json::json!(0.0),
            description: Some("北向持仓占比下限 (%)，0=不限".into()),
            is_secret: false,
        },
        Variable {
            name: "screener_turnover_rate_min".into(),
            var_type: "number".into(),
            value: serde_json::json!(0.0),
            description: Some("换手率下限 (%)，0=不限".into()),
            is_secret: false,
        },
        Variable {
            name: "screener_rsi_oversold".into(),
            var_type: "boolean".into(),
            value: serde_json::json!(false),
            description: Some("选股时要求 RSI 超卖 (<30)".into()),
            is_secret: false,
        },
        Variable {
            name: "screener_rsi_overbought".into(),
            var_type: "boolean".into(),
            value: serde_json::json!(false),
            description: Some("选股时要求 RSI 超买 (>70)".into()),
            is_secret: false,
        },
        // ── A 类补全：信号检测（signals.rs detect_ma_cross / detect_breakout）──
        Variable {
            name: "signal_ma_fast".into(),
            var_type: "number".into(),
            value: serde_json::json!(5),
            description: Some("MA 金叉检测快线周期 (3-30)".into()),
            is_secret: false,
        },
        Variable {
            name: "signal_ma_slow".into(),
            var_type: "number".into(),
            value: serde_json::json!(20),
            description: Some("MA 金叉检测慢线周期 (10-120)".into()),
            is_secret: false,
        },
        Variable {
            name: "signal_breakout_volume_mult".into(),
            var_type: "number".into(),
            value: serde_json::json!(1.5),
            description: Some("突破/破位放量倍数阈值 (1.0-3.0)".into()),
            is_secret: false,
        },
        // ── A 类补全：关键价位（key_levels.rs KeyLevelTracker）──
        Variable {
            name: "keylevel_lookback_days".into(),
            var_type: "number".into(),
            value: serde_json::json!(60),
            description: Some("关键价位回看窗口 (交易日，10-250)".into()),
            is_secret: false,
        },
        Variable {
            name: "keylevel_touch_tolerance_pct".into(),
            var_type: "number".into(),
            value: serde_json::json!(1.0),
            description: Some("关键价位触碰容差 (%，0.1-5.0)".into()),
            is_secret: false,
        },
        Variable {
            name: "keylevel_min_touches".into(),
            var_type: "number".into(),
            value: serde_json::json!(2),
            description: Some("确认支撑/阻力最少触碰次数 (1-10)".into()),
            is_secret: false,
        },
        // ── A 类补全：监控告警（monitor.rs MonitorConfig）──
        Variable {
            name: "monitor_alert_cooldown_secs".into(),
            var_type: "number".into(),
            value: serde_json::json!(300),
            description: Some("同一标的告警冷却时间 (秒，10-3600)".into()),
            is_secret: false,
        },
        Variable {
            name: "monitor_min_severity".into(),
            var_type: "enum".into(),
            value: serde_json::json!("info"),
            description: Some("最低推送告警等级: info / warn / critical".into()),
            is_secret: false,
        },
        Variable {
            name: "monitor_channels".into(),
            var_type: "string".into(),
            value: serde_json::json!("in_app"),
            description: Some("推送渠道，逗号分隔: in_app / lark / email / webhook".into()),
            is_secret: false,
        },
        // ── A 类补全：推荐器策略开关（recommender/strategies）──
        Variable {
            name: "reco_trend_enabled".into(),
            var_type: "boolean".into(),
            value: serde_json::json!(true),
            description: Some("启用趋势跟踪子策略".into()),
            is_secret: false,
        },
        Variable {
            name: "reco_reversion_enabled".into(),
            var_type: "boolean".into(),
            value: serde_json::json!(true),
            description: Some("启用超跌反弹子策略".into()),
            is_secret: false,
        },
        Variable {
            name: "reco_value_enabled".into(),
            var_type: "boolean".into(),
            value: serde_json::json!(true),
            description: Some("启用价值选股子策略".into()),
            is_secret: false,
        },
        Variable {
            name: "reco_capital_enabled".into(),
            var_type: "boolean".into(),
            value: serde_json::json!(true),
            description: Some("启用资金流向子策略".into()),
            is_secret: false,
        },
        Variable {
            name: "reco_watchlist_enabled".into(),
            var_type: "boolean".into(),
            value: serde_json::json!(true),
            description: Some("启用自选股策略".into()),
            is_secret: false,
        },
        Variable {
            name: "reco_min_confidence".into(),
            var_type: "number".into(),
            value: serde_json::json!(60),
            description: Some("推荐器最低置信度 (0-100)，低于此值不入选".into()),
            is_secret: false,
        },
        // ── A 类补全：决策回溯（decision_tracker.rs）──
        Variable {
            name: "decision_max_history_per_stock".into(),
            var_type: "number".into(),
            value: serde_json::json!(50),
            description: Some("每只股票保留的历史决策条数 (10-200)".into()),
            is_secret: false,
        },
        // ── B 类补全：技术指标周期（indicators.rs IndicatorConfig）──
        Variable {
            name: "macd_fast".into(),
            var_type: "number".into(),
            value: serde_json::json!(12),
            description: Some("MACD 快线周期 (5-30)".into()),
            is_secret: false,
        },
        Variable {
            name: "macd_slow".into(),
            var_type: "number".into(),
            value: serde_json::json!(26),
            description: Some("MACD 慢线周期 (10-60)".into()),
            is_secret: false,
        },
        Variable {
            name: "macd_signal".into(),
            var_type: "number".into(),
            value: serde_json::json!(9),
            description: Some("MACD 信号线周期 (3-20)".into()),
            is_secret: false,
        },
        Variable {
            name: "boll_period".into(),
            var_type: "number".into(),
            value: serde_json::json!(20),
            description: Some("布林带周期 (10-50)".into()),
            is_secret: false,
        },
        Variable {
            name: "boll_stddev".into(),
            var_type: "number".into(),
            value: serde_json::json!(2.0),
            description: Some("布林带标准差倍数 (1.0-3.0)".into()),
            is_secret: false,
        },
        Variable {
            name: "volume_lookback".into(),
            var_type: "number".into(),
            value: serde_json::json!(5),
            description: Some("均量计算回看周期 (3-30，交易日)".into()),
            is_secret: false,
        },
        Variable {
            name: "volume_surge_ratio".into(),
            var_type: "number".into(),
            value: serde_json::json!(1.5),
            description: Some("放量阈值 (量比 > 此值判为放量)".into()),
            is_secret: false,
        },
        Variable {
            name: "volume_shrink_ratio".into(),
            var_type: "number".into(),
            value: serde_json::json!(0.7),
            description: Some("缩量阈值 (量比 < 此值判为缩量)".into()),
            is_secret: false,
        },
        // ── B 类补全：推荐器参数（recommender/strategies）──
        Variable {
            name: "trend_kline_limit".into(),
            var_type: "number".into(),
            value: serde_json::json!(250),
            description: Some("趋势策略读取 K 线上限".into()),
            is_secret: false,
        },
        Variable {
            name: "trend_amount_ratio_min".into(),
            var_type: "number".into(),
            value: serde_json::json!(0.8),
            description: Some("趋势策略最低量比".into()),
            is_secret: false,
        },
        Variable {
            name: "rev_rsi_short_max".into(),
            var_type: "number".into(),
            value: serde_json::json!(35.0),
            description: Some("超跌反弹短线 RSI 上限".into()),
            is_secret: false,
        },
        Variable {
            name: "rev_drawdown_min_pct".into(),
            var_type: "number".into(),
            value: serde_json::json!(20.0),
            description: Some("超跌反弹中线最低回撤 (%)".into()),
            is_secret: false,
        },
        Variable {
            name: "rev_rsi_monthly_max".into(),
            var_type: "number".into(),
            value: serde_json::json!(50.0),
            description: Some("超跌反弹月线 RSI 上限".into()),
            is_secret: false,
        },
        Variable {
            name: "val_pe_short_max".into(),
            var_type: "number".into(),
            value: serde_json::json!(50.0),
            description: Some("价值策略短线 PE 上限".into()),
            is_secret: false,
        },
        Variable {
            name: "val_pe_mid_max".into(),
            var_type: "number".into(),
            value: serde_json::json!(40.0),
            description: Some("价值策略中线 PE 上限".into()),
            is_secret: false,
        },
        Variable {
            name: "val_pb_mid_max".into(),
            var_type: "number".into(),
            value: serde_json::json!(8.0),
            description: Some("价值策略中线 PB 上限".into()),
            is_secret: false,
        },
        Variable {
            name: "cap_inflow_short_min".into(),
            var_type: "number".into(),
            value: serde_json::json!(200.0),
            description: Some("资金策略短线主力净流入下限 (万元)".into()),
            is_secret: false,
        },
        Variable {
            name: "cap_inflow_mid_min".into(),
            var_type: "number".into(),
            value: serde_json::json!(500.0),
            description: Some("资金策略中线主力净流入下限 (万元)".into()),
            is_secret: false,
        },
        Variable {
            name: "cap_turnover_min".into(),
            var_type: "number".into(),
            value: serde_json::json!(2.0),
            description: Some("资金策略最低换手率 (%)".into()),
            is_secret: false,
        },
        Variable {
            name: "cap_nb_ratio_min".into(),
            var_type: "number".into(),
            value: serde_json::json!(0.3),
            description: Some("资金策略北向持仓占比下限 (%)".into()),
            is_secret: false,
        },
        // ── B 类补全：交易决策（trading.rs）──
        Variable {
            name: "trading_price_deviation_limit".into(),
            var_type: "number".into(),
            value: serde_json::json!(5.0),
            description: Some("交易价偏离分析目标价最大容忍度 (%)".into()),
            is_secret: false,
        },
        // ── B 类补全：风险模型（risk.rs）──
        Variable {
            name: "risk_sharpe_annualization".into(),
            var_type: "number".into(),
            value: serde_json::json!(252),
            description: Some("夏普比率年化因子（252=日频，12=月频，4=季频，1=年频）".into()),
            is_secret: false,
        },
        Variable {
            name: "risk_kelly_heavy_threshold".into(),
            var_type: "number".into(),
            value: serde_json::json!(0.25),
            description: Some("凯利公式重仓阈值（>此值判为重仓）".into()),
            is_secret: false,
        },
        Variable {
            name: "risk_kelly_medium_threshold".into(),
            var_type: "number".into(),
            value: serde_json::json!(0.1),
            description: Some("凯利公式中仓阈值（>此值判为中仓）".into()),
            is_secret: false,
        },
        // ── B 类补全：compute_scoring / compute_valuation 工具内部参数 ──
        Variable {
            name: "fscore_roe_min".into(),
            var_type: "number".into(),
            value: serde_json::json!(0.10),
            description: Some("F-Score ROE 最低要求 (小数)".into()),
            is_secret: false,
        },
        Variable {
            name: "fscore_gross_margin_min".into(),
            var_type: "number".into(),
            value: serde_json::json!(0.30),
            description: Some("F-Score 毛利率最低要求 (小数)".into()),
            is_secret: false,
        },
        Variable {
            name: "fscore_net_margin_min".into(),
            var_type: "number".into(),
            value: serde_json::json!(0.10),
            description: Some("F-Score 净利率最低要求 (小数)".into()),
            is_secret: false,
        },
        Variable {
            name: "fscore_debt_max".into(),
            var_type: "number".into(),
            value: serde_json::json!(0.60),
            description: Some("F-Score 负债率上限 (小数)".into()),
            is_secret: false,
        },
        Variable {
            name: "fscore_pe_max".into(),
            var_type: "number".into(),
            value: serde_json::json!(20.0),
            description: Some("F-Score PE 上限".into()),
            is_secret: false,
        },
        Variable {
            name: "val_pe_low".into(),
            var_type: "number".into(),
            value: serde_json::json!(15.0),
            description: Some("基本面修正 PE 低估阈值".into()),
            is_secret: false,
        },
        Variable {
            name: "val_pe_high".into(),
            var_type: "number".into(),
            value: serde_json::json!(50.0),
            description: Some("基本面修正 PE 高估阈值".into()),
            is_secret: false,
        },
        Variable {
            name: "val_pb_low".into(),
            var_type: "number".into(),
            value: serde_json::json!(1.0),
            description: Some("基本面修正 PB 低估阈值".into()),
            is_secret: false,
        },
        Variable {
            name: "val_pb_high".into(),
            var_type: "number".into(),
            value: serde_json::json!(6.0),
            description: Some("基本面修正 PB 高估阈值".into()),
            is_secret: false,
        },
        // ── B 类补全：组合风控 compute_portfolio_risk ──
        Variable {
            name: "risk_hhi_concentrated".into(),
            var_type: "number".into(),
            value: serde_json::json!(0.25),
            description: Some("组合 HHI 高度集中阈值 (0-1)".into()),
            is_secret: false,
        },
        Variable {
            name: "risk_hhi_medium".into(),
            var_type: "number".into(),
            value: serde_json::json!(0.15),
            description: Some("组合 HHI 中度集中阈值 (0-1)".into()),
            is_secret: false,
        },
        Variable {
            name: "risk_divers_high".into(),
            var_type: "number".into(),
            value: serde_json::json!(8.0),
            description: Some("组合有效股票数充分分散阈值".into()),
            is_secret: false,
        },
        Variable {
            name: "risk_divers_medium".into(),
            var_type: "number".into(),
            value: serde_json::json!(4.0),
            description: Some("组合有效股票数适度分散阈值".into()),
            is_secret: false,
        },
        Variable {
            name: "analysis_dry_run".into(),
            var_type: "boolean".into(),
            value: serde_json::json!(false),
            description: Some("干跑模式：不调用 LLM，用 mock 输出验证流程".into()),
            is_secret: false,
        },
        // ── 业绩超预期分级阈值
        Variable {
            name: "earnings_th_huge_pos".into(),
            var_type: "number".into(),
            value: serde_json::json!(50.0),
            description: Some("业绩超预期: 大幅超预期下界 (%)".into()),
            is_secret: false,
        },
        Variable {
            name: "earnings_th_strong_pos".into(),
            var_type: "number".into(),
            value: serde_json::json!(20.0),
            description: Some("业绩超预期: 强超预期下界 (%)".into()),
            is_secret: false,
        },
        Variable {
            name: "earnings_th_mild_pos".into(),
            var_type: "number".into(),
            value: serde_json::json!(5.0),
            description: Some("业绩超预期: 略超预期下界 (%)".into()),
            is_secret: false,
        },
        Variable {
            name: "earnings_th_mild_neg".into(),
            var_type: "number".into(),
            value: serde_json::json!(-5.0),
            description: Some("业绩超预期: 略低于预期下界 (%)".into()),
            is_secret: false,
        },
        Variable {
            name: "earnings_th_strong_neg".into(),
            var_type: "number".into(),
            value: serde_json::json!(-20.0),
            description: Some("业绩超预期: 强低于预期下界 (%)".into()),
            is_secret: false,
        },
        Variable {
            name: "earnings_th_huge_neg".into(),
            var_type: "number".into(),
            value: serde_json::json!(-50.0),
            description: Some("业绩超预期: 大幅低于预期下界 (%)".into()),
            is_secret: false,
        },
        // 质押风险分级阈值
        Variable {
            name: "pledge_warning_line".into(),
            var_type: "number".into(),
            value: serde_json::json!(50.0),
            description: Some("大股东质押比例预警线 (%)".into()),
            is_secret: false,
        },
        Variable {
            name: "pledge_liquidation_line".into(),
            var_type: "number".into(),
            value: serde_json::json!(70.0),
            description: Some("大股东质押比例平仓线 (%)".into()),
            is_secret: false,
        },
        Variable {
            name: "pledge_medium_line".into(),
            var_type: "number".into(),
            value: serde_json::json!(30.0),
            description: Some("大股东质押中风险阈值 (%)".into()),
            is_secret: false,
        },
        Variable {
            name: "pledge_low_line".into(),
            var_type: "number".into(),
            value: serde_json::json!(10.0),
            description: Some("大股东质押低风险阈值 (%)".into()),
            is_secret: false,
        },
        // 蒙特卡洛模拟默认参数
        Variable {
            name: "mc_default_price".into(),
            var_type: "number".into(),
            value: serde_json::json!(10.0),
            description: Some("蒙特卡洛模拟默认价格".into()),
            is_secret: false,
        },
        Variable {
            name: "mc_default_return".into(),
            var_type: "number".into(),
            value: serde_json::json!(0.08),
            description: Some("蒙特卡洛模拟默认年化收益".into()),
            is_secret: false,
        },
        Variable {
            name: "mc_default_volatility".into(),
            var_type: "number".into(),
            value: serde_json::json!(0.3),
            description: Some("蒙特卡洛模拟默认年化波动率".into()),
            is_secret: false,
        },
        Variable {
            name: "mc_default_days".into(),
            var_type: "number".into(),
            value: serde_json::json!(30),
            description: Some("蒙特卡洛模拟默认天数".into()),
            is_secret: false,
        },
        Variable {
            name: "mc_default_simulations".into(),
            var_type: "number".into(),
            value: serde_json::json!(1000),
            description: Some("蒙特卡洛模拟默认路径数".into()),
            is_secret: false,
        },
        // 行业内估值/增长对比阈值
        Variable {
            name: "industry_pe_cheap".into(),
            var_type: "number".into(),
            value: serde_json::json!(1.0),
            description: Some("行业内 PE 相对低估阈值".into()),
            is_secret: false,
        },
        Variable {
            name: "industry_pe_expensive".into(),
            var_type: "number".into(),
            value: serde_json::json!(1.5),
            description: Some("行业内 PE 相对高估阈值".into()),
            is_secret: false,
        },
        Variable {
            name: "industry_growth_high".into(),
            var_type: "number".into(),
            value: serde_json::json!(1.2),
            description: Some("行业内高增长阈值".into()),
            is_secret: false,
        },
        // 涨停潜力评分
        Variable {
            name: "limit_pct_main".into(),
            var_type: "number".into(),
            value: serde_json::json!(10.0),
            description: Some("主板涨停幅度 (%)".into()),
            is_secret: false,
        },
        Variable {
            name: "limit_pct_star".into(),
            var_type: "number".into(),
            value: serde_json::json!(20.0),
            description: Some("创业板/科创板涨停幅度 (%)".into()),
            is_secret: false,
        },
        Variable {
            name: "limit_pct_bj".into(),
            var_type: "number".into(),
            value: serde_json::json!(30.0),
            description: Some("北交所涨停幅度 (%)".into()),
            is_secret: false,
        },
        Variable {
            name: "limit_up_w_trend".into(),
            var_type: "number".into(),
            value: serde_json::json!(40.0),
            description: Some("涨停潜力评分 - 趋势权重".into()),
            is_secret: false,
        },
        Variable {
            name: "limit_up_w_volume".into(),
            var_type: "number".into(),
            value: serde_json::json!(20.0),
            description: Some("涨停潜力评分 - 量能权重".into()),
            is_secret: false,
        },
        Variable {
            name: "limit_up_w_hits".into(),
            var_type: "number".into(),
            value: serde_json::json!(15.0),
            description: Some("涨停潜力评分 - 历史涨停权重".into()),
            is_secret: false,
        },
        Variable {
            name: "limit_up_th_high".into(),
            var_type: "number".into(),
            value: serde_json::json!(60.0),
            description: Some("涨停潜力 - 高潜力阈值".into()),
            is_secret: false,
        },
        Variable {
            name: "limit_up_th_med".into(),
            var_type: "number".into(),
            value: serde_json::json!(30.0),
            description: Some("涨停潜力 - 中潜力阈值".into()),
            is_secret: false,
        },
        Variable {
            name: "limit_up_th_low".into(),
            var_type: "number".into(),
            value: serde_json::json!(10.0),
            description: Some("涨停潜力 - 低潜力阈值".into()),
            is_secret: false,
        },
        // ── 反思复盘参数（quality-fallback / portfolio-mgr 复用 portfolio-manager 模板）──
        Variable {
            name: "actual_outcome".into(),
            var_type: "string".into(),
            value: serde_json::json!(""),
            description: Some("实际走势结果，如 '30天跌8% → 失败'，非空时切换反思模式".into()),
            is_secret: false,
        },
        Variable {
            name: "reflection_depth".into(),
            var_type: "string".into(),
            value: serde_json::json!("light"),
            description: Some("反思深度：light(简要) / deep(详细推理链)".into()),
            is_secret: false,
        },
    ];
    let variables_val =
        serde_json::to_string(&variables).map_err(|e| format!("序列化变量失败: {e}"))?;

    // ── 合并旧版本的变量值（保留用户自定义的评分权重/阈值等）──
    let variables_val = match old_variables {
        Some(ref ov) if !ov.is_empty() => {
            merge_variable_values(&variables_val, ov).unwrap_or_else(|_| variables_val.clone())
        },
        _ => variables_val,
    };

    // ── Phase 3/4: Rhai 综合评分工具 + ErrorConfig ──
    use axagent_harness::workflow_types::RhaiToolDef;
    let stock_score_rhai = r##"
// 综合评分脚本：技术面(30%) + 基本面(25%) + 情绪面(20%) + 资金面(15%) + 政策面(10%)
let w_tech = ctx.variables.weight_technical ?? 30.0;
let w_fund = ctx.variables.weight_fundamental ?? 25.0;
let w_sent = ctx.variables.weight_sentiment ?? 20.0;
let w_flow = ctx.variables.weight_money_flow ?? 15.0;
let w_pol = ctx.variables.weight_policy ?? 10.0;

let tech = ctx.results["a-market-analyst"] ?? 50.0;
let fund = ctx.results["a-fundamentals"] ?? 50.0;
let sent = ctx.results["a-sentiment"] ?? 50.0;
let flow = ctx.results["a-hot-money"] ?? 50.0;
let pol = ctx.results["a-policy"] ?? 50.0;

let score = (tech * w_tech + fund * w_fund + sent * w_sent + flow * w_flow + pol * w_pol) / 100.0;
#{
    score: score,
    level: if score >= 80 { "强烈推荐" }
           else if score >= 60 { "推荐" }
           else if score >= 40 { "中性" }
           else { "回避" }
}
"##;
    let rhai_tool_defs: Vec<RhaiToolDef> = vec![RhaiToolDef {
        tool_name: "compute_stock_score".into(),
        description: Some("综合技术面/基本面/情绪面/资金面/政策面计算 0-100 评分".into()),
        code: stock_score_rhai.into(),
    }];
    let tool_defs_val = serde_json::to_string(&rhai_tool_defs)
        .map_err(|e| format!("序列化 Rhai 工具定义失败: {e}"))?;

    let error_config = ErrorConfig {
        retry_policy: Some(RetryPolicy {
            max_retries: 3,
            base_delay_ms: 1000,
            max_delay_ms: 30000,
        }),
        on_failure: OnFailureAction::ContinueWithDefault,
        error_branch: None,
        compensation_steps: None,
    };

    let error_config_val = serde_json::to_string(&error_config)
        .map_err(|e| format!("序列化 ErrorConfig 失败: {e}"))?;

    /// 将子图节点坐标从绝对坐标转换为相对容器的偏移。
    /// 种子数据中的节点坐标是画布绝对坐标，但编辑器 Phase 3 的 subGraph 注入
    /// 将 subGraph 节点 position 视为相对容器的偏移（editor 叠加容器 position
    /// 计算绝对坐标），因此必须在注入前转换。
    fn adjust_positions_to_relative(
        mut sub_nodes: Vec<WorkflowNode>,
        container_id: &str,
        all_nodes: &[WorkflowNode],
    ) -> Vec<WorkflowNode> {
        let container_pos = all_nodes
            .iter()
            .find(|n| n.base_id() == container_id)
            .map(|n| n.base().position.clone())
            .unwrap_or(Position { x: 0.0, y: 0.0 });
        for node in sub_nodes.iter_mut() {
            match node {
                WorkflowNode::Trigger(n) => {
                    n.base.position.x -= container_pos.x;
                    n.base.position.y -= container_pos.y;
                },
                WorkflowNode::Agent(n) => {
                    n.base.position.x -= container_pos.x;
                    n.base.position.y -= container_pos.y;
                },
                WorkflowNode::Llm(n) => {
                    n.base.position.x -= container_pos.x;
                    n.base.position.y -= container_pos.y;
                },
                WorkflowNode::Condition(n) => {
                    n.base.position.x -= container_pos.x;
                    n.base.position.y -= container_pos.y;
                },
                WorkflowNode::Parallel(n) => {
                    n.base.position.x -= container_pos.x;
                    n.base.position.y -= container_pos.y;
                },
                WorkflowNode::Loop(n) => {
                    n.base.position.x -= container_pos.x;
                    n.base.position.y -= container_pos.y;
                },
                WorkflowNode::Merge(n) => {
                    n.base.position.x -= container_pos.x;
                    n.base.position.y -= container_pos.y;
                },
                WorkflowNode::Delay(n) => {
                    n.base.position.x -= container_pos.x;
                    n.base.position.y -= container_pos.y;
                },
                WorkflowNode::Validation(n) => {
                    n.base.position.x -= container_pos.x;
                    n.base.position.y -= container_pos.y;
                },
                WorkflowNode::SubWorkflow(n) => {
                    n.base.position.x -= container_pos.x;
                    n.base.position.y -= container_pos.y;
                },
                WorkflowNode::DocumentParser(n) => {
                    n.base.position.x -= container_pos.x;
                    n.base.position.y -= container_pos.y;
                },
                WorkflowNode::VectorRetrieve(n) => {
                    n.base.position.x -= container_pos.x;
                    n.base.position.y -= container_pos.y;
                },
                WorkflowNode::End(n) => {
                    n.base.position.x -= container_pos.x;
                    n.base.position.y -= container_pos.y;
                },
                WorkflowNode::HttpRequest(n) => {
                    n.base.position.x -= container_pos.x;
                    n.base.position.y -= container_pos.y;
                },
                WorkflowNode::Switch(n) => {
                    n.base.position.x -= container_pos.x;
                    n.base.position.y -= container_pos.y;
                },
                WorkflowNode::DatabaseQuery(n) => {
                    n.base.position.x -= container_pos.x;
                    n.base.position.y -= container_pos.y;
                },
                WorkflowNode::Notification(n) => {
                    n.base.position.x -= container_pos.x;
                    n.base.position.y -= container_pos.y;
                },
                WorkflowNode::Approval(n) => {
                    n.base.position.x -= container_pos.x;
                    n.base.position.y -= container_pos.y;
                },
                WorkflowNode::FileOperation(n) => {
                    n.base.position.x -= container_pos.x;
                    n.base.position.y -= container_pos.y;
                },
                WorkflowNode::DataTransformer(n) => {
                    n.base.position.x -= container_pos.x;
                    n.base.position.y -= container_pos.y;
                },
                WorkflowNode::WebhookSend(n) => {
                    n.base.position.x -= container_pos.x;
                    n.base.position.y -= container_pos.y;
                },
                WorkflowNode::Logging(n) => {
                    n.base.position.x -= container_pos.x;
                    n.base.position.y -= container_pos.y;
                },
                WorkflowNode::LlmClassifier(n) => {
                    n.base.position.x -= container_pos.x;
                    n.base.position.y -= container_pos.y;
                },
                WorkflowNode::Aggregator(n) => {
                    n.base.position.x -= container_pos.x;
                    n.base.position.y -= container_pos.y;
                },
                WorkflowNode::Email(n) => {
                    n.base.position.x -= container_pos.x;
                    n.base.position.y -= container_pos.y;
                },
                WorkflowNode::Debate(n) => {
                    n.base.position.x -= container_pos.x;
                    n.base.position.y -= container_pos.y;
                },
                WorkflowNode::Swarm(n) => {
                    n.base.position.x -= container_pos.x;
                    n.base.position.y -= container_pos.y;
                },
                WorkflowNode::Storage(n) => {
                    n.base.position.x -= container_pos.x;
                    n.base.position.y -= container_pos.y;
                },
                WorkflowNode::Tool(n) => {
                    n.base.position.x -= container_pos.x;
                    n.base.position.y -= container_pos.y;
                },
                WorkflowNode::Code(n) => {
                    n.base.position.x -= container_pos.x;
                    n.base.position.y -= container_pos.y;
                },
                WorkflowNode::WorkflowRef(n) => {
                    n.base.position.x -= container_pos.x;
                    n.base.position.y -= container_pos.y;
                },
            }
        }
        sub_nodes
    }

    // ── 注入容器节点子图（subGraph）用于编辑器嵌套渲染 ──
    // 子图仅在编辑器的 ReactFlow 渲染层中用于坐标转换（绝对→相对），
    // 运行时引擎仍从顶层 nodes 读取所有节点。
    // 编辑器保存时会自动去重（上游 WorkflowEditor.tsx save 路径过滤 subGraph 子节点）。
    let container_nodes: &[&str] = &["p-analysts", "debate-bull-bear", "p-risk-assess"];
    for &cid in container_nodes {
        let child_ids: Vec<String> = nodes
            .iter()
            .filter(|n| n.base().parent_id.as_deref() == Some(cid))
            .map(|n| n.base_id().to_string())
            .collect();
        if child_ids.is_empty() {
            continue;
        }
        let child_node_ids: std::collections::HashSet<&str> =
            child_ids.iter().map(|s| s.as_str()).collect();
        let sub_edges: Vec<WorkflowEdge> = edges
            .iter()
            .filter(|e| {
                child_node_ids.contains(e.source.as_str())
                    && child_node_ids.contains(e.target.as_str())
            })
            .cloned()
            .collect();
        let sub_nodes: Vec<WorkflowNode> = nodes
            .iter()
            .filter(|n| child_node_ids.contains(n.base_id()))
            .cloned()
            .collect();
        let sub_graph = SubGraph {
            // 子图节点坐标必须相对于容器（Phase 3 编辑器将 subGraph 节点视为相对偏移，
            // 计算绝对坐标时叠加 container.position）。种子数据中的坐标是绝对坐标，
            // 因此在注入前转换为相对坐标。
            nodes: adjust_positions_to_relative(sub_nodes, cid, &nodes),
            edges: sub_edges,
        };
        // 注入到容器节点 config 中
        for n in nodes.iter_mut() {
            if n.base_id() != cid {
                continue;
            }
            match n {
                WorkflowNode::Parallel(p) => {
                    p.config.sub_graph = Some(sub_graph);
                },
                WorkflowNode::Debate(d) => {
                    d.config.sub_graph = Some(sub_graph);
                },
                _ => {},
            }
            break;
        }
    }
    // 写入 DB
    let nodes_json = serde_json::to_string(&nodes).map_err(|e| format!("序列化节点失败: {e}"))?;
    // DEBUG: 验证前几个 Tool 节点的 type 字段
    for n in nodes.iter().take(5) {
        let json = serde_json::to_string(n).unwrap_or_default();
        let preview = if json.len() > 200 {
            &json[..200]
        } else {
            &json
        };
        tracing::info!(node_id = %n.base_id(), json_preview = %preview, "seed_node_type");
    }
    let edges_json = serde_json::to_string(&edges).map_err(|e| format!("序列化边失败: {e}"))?;
    let tags = serde_json::to_string(&["stock", "analysis", "A股"])
        .map_err(|e| format!("序列化标签失败: {e}"))?;

    // 先删再插，避免 SeaORM .save() 对已存在记录的 update 失败
    let _ = workflow_template::Entity::delete_by_id(TEMPLATE_ID)
        .exec(db)
        .await;
    workflow_template::ActiveModel {
        id: Set(TEMPLATE_ID.to_string()),
        name: Set("A股多维度分析".to_string()),
        description: Set(Some(
            "9 维度分析师 → LLM 智能辩论 → 价值投资（巴菲特框架）→ 3 风险维度 → Rhai 评分 → 交易方案 → 投资决策"
                .to_string(),
        )),
        icon: Set("chart-bar".into()),
        tags: Set(Some(tags)),
        version: Set(TEMPLATE_VERSION),
        is_preset: Set(true),
        is_editable: Set(true),
        is_public: Set(true),
        trigger_config: Set(Some(
            serde_json::to_string(&TriggerConfig {
                trigger_type: TriggerType::Schedule,
                config: serde_json::json!({
                    "schedules": {
                        "morning": "0 9 * * 1-5",
                        "afternoon": "0 14 * * 1-5",
                    },
                    // F-9 修复: 原 enabled=false 导致工作流不会自动调度。
                    //   既然有 schedule 配置,就应该是自动跑。改为 true,
                    //   用户仍可在 UI 切换到 "未启用" 状态临时停止。
                    "enabled": true,
                    "timezone": "Asia/Shanghai",
                }),
            })
            .unwrap_or_default(),
        )),
        nodes: Set(nodes_json),
        edges: Set(edges_json),
        input_schema: Set(Some(input_schema_val)),
        output_schema: Set(Some(output_schema_val)),
        variables: Set(if let Some(ref ov) = old_variables {
            // 升级时保留用户自定义的变量值
            let new_vars: Vec<serde_json::Value> =
                serde_json::from_str(&variables_val).unwrap_or_default();
            let mut final_vars = new_vars.clone();
            if let Ok(old_parsed) = serde_json::from_str::<Vec<serde_json::Value>>(ov) {
                for nv in &mut final_vars {
                    let nv_name = nv.get("name").and_then(|n| n.as_str());
                    if let Some(nv_name) = nv_name {
                        if let Some(old_v) = old_parsed
                            .iter()
                            .find(|ov| ov.get("name").and_then(|n| n.as_str()) == Some(nv_name))
                        {
                            if let Some(old_val) = old_v.get("value") {
                                nv.as_object_mut()
                                    .map(|o| o.insert("value".into(), old_val.clone()));
                            }
                        }
                    }
                }
            }
            Some(serde_json::to_string(&final_vars).unwrap_or(variables_val.clone()))
        } else {
            Some(variables_val.clone())
        }),
        error_config: Set(Some(error_config_val)),
        composite_source: Set(None),
        tool_defs: Set(Some(tool_defs_val)),
        created_at: Set(now),
        updated_at: Set(now),
    }
    .insert(db)
    .await
    .map_err(|e| format!("写入工作流模板失败: {e}"))?;

    tracing::info!("[stock_analysis_setup] 股票分析工作流模板已种子化 ({TEMPLATE_ID})");
    Ok(())
}

async fn seed_agency_experts(db: &sea_orm::DatabaseConnection) -> Result<(), String> {
    use axagent_core::entity::agency_experts;
    use sea_orm::{ActiveModelTrait, EntityTrait, Set};

    let mut count = 0u32;
    for &(expert_id, content) in EMBEDDED_PROMPTS {
        let (name, desc, body, color) = parse_expert_md(content, expert_id);
        let agency_id = format!("agency-stock-analysis-{expert_id}");
        if agency_experts::Entity::find_by_id(&agency_id)
            .one(db)
            .await
            .map_err(|e| e.to_string())?
            .is_some()
        {
            continue;
        }
        let now = chrono::Utc::now().timestamp();
        let model = agency_experts::ActiveModel {
            id: Set(agency_id.clone()),
            name: Set(name),
            description: Set(if desc.is_empty() { None } else { Some(desc) }),
            category: Set("finance".into()),
            system_prompt: Set(body),
            color: Set(color),
            source_dir: Set("stock-analysis".into()),
            is_enabled: Set(1),
            imported_at: Set(now),
            recommended_workflows: Set(None),
            recommended_tools: Set(None),
        };
        model.insert(db).await.map_err(|e| e.to_string())?;
        count += 1;
    }
    tracing::info!("[stock_analysis_setup] 已种子化 {count} 个 agency_experts");
    Ok(())
}

async fn seed_agent_roles(db: &sea_orm::DatabaseConnection) -> Result<(), String> {
    let mut count = 0u32;
    for role in STOCK_ROLES {
        if repo::agent_role::get_agent_role(db, role.id)
            .await
            .map_err(|e| e.to_string())?
            .is_some()
        {
            continue;
        }
        repo::agent_role::upsert_agent_role(
            db,
            role.id,
            role.name,
            Some(role.description),
            role.system_prompt,
            &[],
            role.max_concurrent,
            role.timeout_seconds,
            "stock-analysis",
        )
        .await
        .map_err(|e| e.to_string())?;
        count += 1;
    }
    tracing::info!("[stock_analysis_setup] 已种子化 {count} 个 agent_roles");
    Ok(())
}

async fn seed_agent_profiles(db: &sea_orm::DatabaseConnection) -> Result<(), String> {
    use axagent_core::entity::agent_profiles;
    use sea_orm::{ActiveModelTrait, EntityTrait, Set};

    // Profile → 工具映射（从模块级 PROFILE_TOOLS 构建）
    let profile_tools: std::collections::HashMap<&str, &[&str]> =
        PROFILE_TOOLS.iter().cloned().collect();

    let mut count = 0u32;
    for &(expert_id, role_id) in EXPERT_ROLE_MAP {
        let profile_id = format!("stock-{expert_id}");

        if agent_profiles::Entity::find_by_id(&profile_id)
            .one(db)
            .await
            .map_err(|e| e.to_string())?
            .is_some()
        {
            continue;
        }

        let tools_json = profile_tools
            .get(expert_id)
            .map(|tools| serde_json::to_string(tools).unwrap_or_default());
        let now = chrono::Utc::now().timestamp_millis();
        let model = agent_profiles::ActiveModel {
            id: Set(profile_id.clone()),
            name: Set(format!("📈 {}", expert_id_to_display(expert_id))),
            description: Set(Some(format!("股票分析专家 — {}", role_id_to_display(role_id)))),
            category: Set("stock-analysis".into()),
            icon: Set("📈".into()),
            agent_role: Set(Some(role_id.into())),
            source: Set("stock-analysis".into()),
            tags: Set(None),
            suggested_provider_id: Set(None),
            suggested_model_id: Set(None),
            suggested_temperature: Set(None),
            suggested_max_tokens: Set(None),
            search_enabled: Set(None),
            recommend_permission_mode: Set(None),
            recommended_tools: Set(tools_json),
            disallowed_tools: Set(None),
            recommended_workflows: Set(None),
            sort_order: Set(0),
            is_enabled: Set(1),
            expert_id: Set(Some(format!("agency-stock-analysis-{expert_id}"))),
            created_at: Set(now),
            updated_at: Set(now),
        };
        model.insert(db).await.map_err(|e| e.to_string())?;
        count += 1;
    }
    tracing::info!("[stock_analysis_setup] 已种子化/更新 {count} 个 agent_profiles");
    Ok(())
}

fn parse_expert_md(content: &str, fallback: &str) -> (String, String, String, Option<String>) {
    let mut name = String::new();
    let mut desc = String::new();
    let mut color: Option<String> = None;
    let body = if let Some(rest) = content.strip_prefix("---") {
        if let Some(end) = rest.find("\n---") {
            let fm = &rest[..end];
            for line in fm.lines() {
                // title: 作为 name: 的别名（多份 .md 沿用 old frontmatter 习惯）
                if let Some(v) = line.trim().strip_prefix("name:") {
                    name = v.trim().into();
                } else if let Some(v) = line.trim().strip_prefix("title:") {
                    if name.is_empty() {
                        name = v.trim().into();
                    }
                } else if let Some(v) = line.trim().strip_prefix("description:") {
                    desc = v.trim().into();
                } else if let Some(v) = line.trim().strip_prefix("color:") {
                    let c = v.trim();
                    if !c.is_empty() {
                        color = Some(c.into());
                    }
                }
            }
            rest[end + 4..].trim().to_string()
        } else {
            content.to_string()
        }
    } else {
        content.to_string()
    };
    if name.is_empty() {
        name = expert_id_to_display(fallback);
    }
    (name, desc, body, color)
}

fn expert_id_to_display(id: &str) -> String {
    match id {
        "market-analyst" => "市场技术分析师".to_string(),
        "sentiment-analyst" => "情绪面分析师".to_string(),
        "news-analyst" => "消息面分析师".to_string(),
        "fundamentals-analyst" => "基本面分析师".to_string(),
        "policy-analyst" => "政策面分析师".to_string(),
        "hot-money-tracker" => "资金面追踪".to_string(),
        "lockup-watcher" => "筹码限售观察".to_string(),
        "research-analyst" => "研报分析师".to_string(),
        "sector-analyst" => "板块题材分析师".to_string(),
        "bull-researcher" => "多方研究员".to_string(),
        "bear-researcher" => "空方研究员".to_string(),
        "aggressive-debator" => "激进风险评估".to_string(),
        "conservative-debator" => "保守风险评估".to_string(),
        "neutral-debator" => "中性风险评估".to_string(),
        "research-manager" => "研究经理".to_string(),
        "trader" => "交易员".to_string(),
        "value-investor" => "价值投资者（巴菲特框架）".to_string(),
        "catalyst-analyst" => "催化剂与叙事分析师".to_string(),
        // ── Serenity 瓶颈分析师 ──
        "trend-scanner" => "产业趋势扫描器".to_string(),
        "chain-decomposer" => "产业链拆解师".to_string(),
        "chokepoint-identifier" => "瓶颈鉴定师".to_string(),
        "candidate-mapper" => "候选公司映射器".to_string(),
        o => o.to_string(),
    }
}

fn role_id_to_display(id: &str) -> String {
    match id {
        "stock-analyst" => "股票分析师".to_string(),
        "debater" => "辩论研究员".to_string(),
        "risk-evaluator" => "风险评估师".to_string(),
        "trader" => "交易员".to_string(),
        "decision-maker" => "决策者".to_string(),
        "reflection" => "投资复盘官".to_string(),
        o => o.to_string(),
    }
}

/// 构建分析师 params 的 input_mapping：为每个分析师注入 bull_score/bear_score/consensus_score
/// 例如 a-market-analyst → 【market_bull_score】:75 【market_bear_score】:25
fn build_analyst_input_mapping(a_ids: &[&str]) -> std::collections::HashMap<String, String> {
    use std::collections::HashMap;
    let mut map = HashMap::new();
    for aid in a_ids {
        // a-market-analyst → market, a-sentiment → sentiment, etc.
        let prefix = aid.strip_prefix("a-").unwrap_or(aid);
        map.insert(format!("{prefix}_bull_score"), format!("{aid}.params.bull_score"));
        map.insert(format!("{prefix}_bear_score"), format!("{aid}.params.bear_score"));
        // consensus_score = bull - bear（聚合分数）
        map.insert(format!("{prefix}_consensus"), format!("{aid}.params.consensus_score"));
    }
    // 为所有辩论/风险节点注入历史反思教训
    map.insert("stock_lessons".into(), "stock_lessons".into());
    map
}

/// 合并新模板变量与旧模板变量的值。
/// 对于同名的变量，保留旧变量的 value（用户的修改），字段定义以新模板为准。
fn merge_variable_values(
    new_variables_json: &str,
    old_variables_json: &str,
) -> Result<String, String> {
    let new_vars: Vec<serde_json::Value> =
        serde_json::from_str(new_variables_json).map_err(|e| format!("解析新变量失败: {e}"))?;
    let old_vars: Vec<serde_json::Value> =
        serde_json::from_str(old_variables_json).map_err(|e| format!("解析旧变量失败: {e}"))?;

    // 变量迁移映射表：旧名称 → 新名称（模板升级时变量被重命名的情况）
    //
    // 老 UI 用的 camelCase 命名在 stock-analysis 模板 v15→v19 升级时统一改为 snake_case
    // 并补全前缀（agent_/tool_/rule_/pos_/value_/monitor_/kline_/news_/vendor_）。
    // 旧用户在设置面板调整过的值会留在 DB 的 workflow_template.variables 列里，
    // 升级时如果新模板没有同 key 的变量就会被丢弃。这里建立别名映射，
    // 升级时把旧 key 的 value 复制到新 key 上，避免用户调参失效。
    const RENAME_MAP: &[(&str, &str)] = &[
        // 分析流程
        ("analysis_maxDebateRounds", "debate_rounds"),
        ("analysis_maxConcurrent", "max_concurrent"),
        // 数据源
        ("analysis_klinePeriod", "kline_period"),
        ("analysis_klineLimit", "kline_limit"),
        ("analysis_newsLimit", "news_limit"),
        // Agent / Tool
        ("analysis_temperature", "agent_temperature"),
        ("analysis_maxTokens", "agent_max_tokens"),
        ("analysis_timeoutSecs", "agent_timeout_secs"),
        ("tool_timeoutSecs", "tool_timeout_secs"),
        ("tool_retryMax", "tool_retry_max"),
        // 规则
        ("rule_rsiOverbought", "rule_rsi_overbought"),
        ("rule_rsiOversold", "rule_rsi_oversold"),
        ("rule_biasLimit", "rule_bias_limit_pct"),
        ("rule_volumeSignalBlock", "rule_volume_signal_block"),
        ("rule_bearLowScore", "rule_bear_low_score"),
        ("rule_autoStopLossPct", "rule_auto_stop_loss_pct"),
        // 仓位
        ("pos_maxSingleStockPct", "pos_max_single_pct"),
        ("pos_maxTotalPositions", "pos_max_total"),
        ("pos_maxSectorExposurePct", "pos_max_sector_pct"),
        // 估值
        ("value_dcfGrowthRate", "value_dcf_growth_rate"),
        ("value_dcfPerpetualRate", "value_dcf_perpetual_rate"),
        ("value_dcfDiscountRate", "value_dcf_discount_rate"),
        ("value_moatThreshold", "value_moat_threshold"),
        ("value_fScoreBuyThreshold", "value_fscore_buy"),
        ("value_safetyMarginMin", "value_safety_margin"),
        // 监控
        ("monitor_pollIntervalSecs", "monitor_poll_interval_secs"),
        ("monitor_changePctThreshold", "monitor_change_pct"),
        ("monitor_turnoverThreshold", "monitor_turnover"),
    ];

    // 构建旧变量名 → value 的映射（处理重命名别名）
    let old_values: std::collections::HashMap<String, serde_json::Value> = old_vars
        .into_iter()
        .filter_map(|v| {
            let name = v.get("name")?.as_str()?;
            let value = v.get("value")?.clone();
            // 主名称
            let mut entries = vec![(name.to_string(), value.clone())];
            // 如果该变量有重命名别名，也加入映射
            for (old, new) in RENAME_MAP {
                if *new == name {
                    entries.push((old.to_string(), value.clone()));
                }
            }
            Some(entries)
        })
        .flatten()
        .collect();

    // 合并：新变量定义 + 旧变量值（如有）
    let merged: Vec<serde_json::Value> = new_vars
        .into_iter()
        .map(|mut v| {
            if let Some(name) = v.get("name").and_then(|n| n.as_str()) {
                if let Some(old_val) = old_values.get(name) {
                    v["value"] = old_val.clone();
                }
            }
            v
        })
        .collect();

    serde_json::to_string(&merged).map_err(|e| format!("序列化合变量失败: {e}"))
}

// seed_debate_subworkflow: 辩论已通过 DebateNode 容器直接嵌入主模板，旧独立模板已移除

/// 种子化反思复盘工作流模板（stock-reflection）。
///
/// 与 stock-analysis 同款：用 Rust 类型（`WorkflowNode` / `WorkflowNodeBase` /
/// `WorkflowNodeConfig::*`）构造节点，再 `serde_json::to_string` 序列化入库。
/// 这样编译器会强制要求所有必填字段（id/title/position/retry/enabled…），
/// 避免 `serde_json::json!()` 裸写漏字段导致反序列化静默失败、编辑器看不到节点。
///
/// 运行时 portfolio-manager 通过 `{{actual_outcome}}` 变量切换到反思模式。
async fn seed_reflection_workflow_template(db: &sea_orm::DatabaseConnection) -> Result<(), String> {
    use axagent_core::entity::workflow_template;
    use axagent_harness::workflow_types::{
        AgentNode, AgentNodeConfig, EdgeType, OutputMode, Position, RetryConfig, StorageNode,
        StorageNodeConfig, SubWorkflowNode, SubWorkflowNodeConfig, TriggerConfig, TriggerNode,
        TriggerType, Variable, WorkflowEdge, WorkflowNode, WorkflowNodeBase,
    };
    use sea_orm::{ActiveModelTrait, EntityTrait, Set};

    let now = chrono::Utc::now().timestamp_millis();

    // ── 节点定义（与 stock-analysis 同款：Rust 类型构造，编译期校验必填字段）──
    let nodes: Vec<WorkflowNode> = vec![
        // 1. 触发器：手动模式，传入 stock_code / as_of_date / actual_outcome / reflection_depth
        WorkflowNode::Trigger(TriggerNode {
            base: WorkflowNodeBase {
                id: "trigger".into(),
                title: "反思复盘触发器".into(),
                description: Some("触发反思复盘工作流，传入 stock_code / as_of_date".into()),
                position: Position { x: 20.0, y: 20.0 },
                retry: RetryConfig::default(),
                timeout: None,
                enabled: true,
                parent_id: None,
                compensation: None,
            },
            config: TriggerConfig {
                trigger_type: TriggerType::Manual,
                config: serde_json::json!({
                    "description": "as-of 重放: 选择历史日期对分析结果进行反思复盘",
                    "required_params": ["as_of_date", "stock_code"],
                    "param_schema": {
                        "as_of_date": { "type": "date", "description": "原始分析日期，决定数据时间锚点" },
                        "stock_code": { "type": "string", "description": "股票代码" }
                    }
                }),
            },
        }),
        // 2. 嵌套子工作流：复用 stock-analysis 模板的分析能力
        WorkflowNode::SubWorkflow(SubWorkflowNode {
            base: WorkflowNodeBase {
                id: "sub-analysis".into(),
                title: "调用股票分析子工作流".into(),
                description: Some("嵌套 stock-analysis 子工作流，复用其 9 维度分析能力".into()),
                position: Position { x: 20.0, y: 120.0 },
                retry: RetryConfig {
                    enabled: true,
                    max_retries: 2,
                    ..Default::default()
                },
                timeout: Some(600),
                enabled: true,
                parent_id: None,
                compensation: None,
            },
            config: SubWorkflowNodeConfig {
                sub_workflow_id: "stock-analysis".into(),
                input_mapping: [
                    ("stock_code".to_string(), "trigger".to_string()),
                    ("as_of_date".to_string(), "trigger".to_string()),
                ]
                .into_iter()
                .collect(),
                output_var: "sub-analysis".into(),
                is_async: false,
                sub_graph: None,
            },
        }),
        // 3. 反思复盘 Agent：注入实际走势结果 + 反思深度，驱动 portfolio-manager 切反思模式
        WorkflowNode::Agent(AgentNode {
            base: WorkflowNodeBase {
                id: "reflection-agent".into(),
                title: "反思复盘".into(),
                description: Some("基于实际走势结果对历史分析做反思复盘".into()),
                position: Position { x: 20.0, y: 260.0 },
                retry: RetryConfig {
                    enabled: true,
                    max_retries: 2,
                    ..Default::default()
                },
                timeout: Some(300),
                enabled: true,
                parent_id: None,
                compensation: None,
            },
            config: AgentNodeConfig {
                system_prompt: "你的任务：对历史股票分析进行反思复盘。\n\
                    目标股票代码: {{stock_code}}，股票名称: {{stock_name}}\n\
                    实际走势结果: {{actual_outcome}}（非空 → 反思模式）\n\
                    反思深度: {{reflection_depth}}（light = 简要；deep = 详细推理链）\n\n\
                    重要原则：\n\
                    1. 必须严格基于 actual_outcome 提供的实际走势与上游分析结论做对比，识别错因。\n\
                    2. 严禁输出空结果或只列 data_gaps。\n\
                    3. 反思深度=deep 时给出可执行的检查清单（具体指标阈值、信号确认步骤）。\n\n\
                    你必须输出严格 JSON 格式（不要 Markdown 代码块，不要多余文本），字段如下：\n\
                    {\n\
                      \"what_went_wrong\": \"哪里判断错了，简要说明\",\n\
                      \"missed_signals\": [\"被忽略的信号1\", \"被忽略的信号2\"],\n\
                      \"fix_for_future\": \"下次如何避免同样的错误\",\n\
                      \"params_suggestion\": {\"参数名\": \"调整建议\"}\n\
                    }"
                .into(),
                context_sources: vec!["sub-analysis".into()],
                input_mapping: [
                    ("stock_code".to_string(), "trigger".to_string()),
                    ("stock_name".to_string(), "trigger".to_string()),
                    ("actual_outcome".to_string(), "trigger".to_string()),
                    ("reflection_depth".to_string(), "trigger".to_string()),
                ]
                .into_iter()
                .collect(),
                output_var: "reflection".into(),
                model: None,
                temperature: Some(0.3),
                max_tokens: Some(8192),
                tools: vec![],
                exposed_tools: vec![],
                output_mode: OutputMode::Json,
                agent_profile_id: Some("stock-reflection".into()),
                max_tool_rounds: None,
                execution_mode: None,
                rag_source_ids: vec![],
                model_role: Some("decision-maker".into()),
                consistency_check: None,
                hallucination_guard: None,
            },
        }),
        // 4. 反思记录持久化：写入 stock_reflections 表供后续查询/复盘
        WorkflowNode::Storage(StorageNode {
            base: WorkflowNodeBase {
                id: "store-ref".into(),
                title: "反思记录持久化".into(),
                description: Some("写入反思记录到 stock_reflections 表".into()),
                position: Position { x: 20.0, y: 400.0 },
                retry: RetryConfig {
                    enabled: true,
                    max_retries: 2,
                    ..Default::default()
                },
                timeout: Some(30),
                enabled: true,
                parent_id: None,
                compensation: None,
            },
            config: StorageNodeConfig {
                backend: "sqlite".into(),
                operation: "insert".into(),
                input_var: "reflection".into(),
                collection: "stock_reflections".into(),
                key_var: None,
                output_var: "storage-result".into(),
            },
        }),
    ];

    let edges: Vec<WorkflowEdge> = vec![
        WorkflowEdge {
            id: "e-trigger-sub-analysis".into(),
            source: "trigger".into(),
            source_handle: None,
            target: "sub-analysis".into(),
            target_handle: None,
            edge_type: EdgeType::Direct,
            label: None,
        },
        WorkflowEdge {
            id: "e-sub-analysis-reflection".into(),
            source: "sub-analysis".into(),
            source_handle: None,
            target: "reflection-agent".into(),
            target_handle: None,
            edge_type: EdgeType::Direct,
            label: None,
        },
        WorkflowEdge {
            id: "e-reflection-store".into(),
            source: "reflection-agent".into(),
            source_handle: None,
            target: "store-ref".into(),
            target_handle: None,
            edge_type: EdgeType::Direct,
            label: None,
        },
    ];

    let variables: Vec<Variable> = vec![
        Variable {
            name: "actual_outcome".into(),
            var_type: "string".into(),
            value: serde_json::Value::String("".into()),
            description: Some("实际走势结果，如 '30天跌8% → 失败'，非空时触发反思模式".into()),
            is_secret: false,
        },
        Variable {
            name: "reflection_depth".into(),
            var_type: "string".into(),
            value: serde_json::Value::String("light".into()),
            description: Some("反思深度：light(简要) / deep(详细推理链)".into()),
            is_secret: false,
        },
    ];

    // serenity-reflection 模板版本。v2: 重新种子化
    const REFLECTION_TEMPLATE_VERSION: i32 = 2;

    // 版本检查：已有同版本或更新的记录则跳过
    if let Some(ref existing) =
        axagent_core::entity::workflow_template::Entity::find_by_id("stock-reflection")
            .one(db)
            .await
            .map_err(|e| format!("查重失败: {e}"))?
    {
        if existing.version >= REFLECTION_TEMPLATE_VERSION {
            tracing::info!(
                "[stock_analysis_setup] 反思模板已是最新 v{}，跳过种子化",
                existing.version
            );
            return Ok(());
        }
        // 旧版本 → 保存快照
        let ver_id = format!("stock-reflection_v{}", existing.version);
        if axagent_core::entity::workflow_template_version::Entity::find_by_id(&ver_id)
            .one(db)
            .await
            .map_err(|e| format!("查重失败: {e}"))?
            .is_none()
        {
            use sea_orm::ActiveModelTrait;
            let snapshot = axagent_core::entity::workflow_template_version::ActiveModel {
                id: Set(ver_id.clone()),
                template_id: Set("stock-reflection".to_string()),
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
            snapshot
                .insert(db)
                .await
                .map_err(|e| format!("写入版本快照失败: {e}"))?;
            tracing::info!("[stock_analysis_setup] 反思模板旧版本快照已保存: {ver_id}");
        }
    }

    // 走 stock-analysis 同款序列化路径：编译期校验 + 字段齐全
    let nodes_json =
        serde_json::to_string(&nodes).map_err(|e| format!("序列化反思节点失败: {e}"))?;
    let edges_json = serde_json::to_string(&edges).map_err(|e| format!("序列化反思边失败: {e}"))?;
    let variables_json =
        serde_json::to_string(&variables).map_err(|e| format!("序列化反思变量失败: {e}"))?;
    let tags_json = serde_json::to_string(&["stock", "reflection", "A股"])
        .map_err(|e| format!("序列化反思标签失败: {e}"))?;

    // 先删再插，避免 SeaORM .save() 对已存在记录的 update 失败
    let _ = workflow_template::Entity::delete_by_id("stock-reflection")
        .exec(db)
        .await;
    workflow_template::ActiveModel {
        id: Set("stock-reflection".to_string()),
        name: Set("A股反思复盘".to_string()),
        description: Set(Some(
            "嵌套 stock-analysis 子工作流的 as-of 重放，注入实际走势结果后反思".to_string(),
        )),
        icon: Set("search".into()),
        tags: Set(Some(tags_json)),
        version: Set(REFLECTION_TEMPLATE_VERSION),
        is_preset: Set(true),
        is_editable: Set(true),
        is_public: Set(true),
        trigger_config: Set(Some(
            serde_json::to_string(&TriggerConfig {
                trigger_type: TriggerType::Manual,
                config: serde_json::json!({
                    "description": "as-of 重放: 选择历史日期对分析结果进行反思复盘",
                    "required_params": ["as_of_date", "stock_code"],
                    "param_schema": {
                        "as_of_date": { "type": "date", "description": "原始分析日期，决定数据时间锚点" },
                        "stock_code": { "type": "string", "description": "股票代码" }
                    }
                }),
            })
            .map_err(|e| format!("序列化触发器配置失败: {e}"))?,
        )),
        nodes: Set(nodes_json),
        edges: Set(edges_json),
        input_schema: Set(None),
        output_schema: Set(None),
        variables: Set(Some(variables_json)),
        error_config: Set(None),
        composite_source: Set(None),
        tool_defs: Set(None),
        created_at: Set(now),
        updated_at: Set(now),
    }
    .insert(db)
    .await
    .map_err(|e| format!("写入反思模板失败: {e}"))?;

    tracing::info!(
        "[stock_analysis_setup] 反思复盘工作流模板已创建 (stock-reflection, SubWorkflowNode 嵌套)"
    );
    Ok(())
}

/// 创建 Serenity 瓶颈筛选工作流模板（serenity-screening）。
///
/// ── Phase 0: 趋势扫描 ──
///   t-hot-stocks / t-industry-rank / t-cls-flash / t-concept / t-northbound
///   → a-trend-scanner (LLM Agent, 输出 2-3 个趋势)
///
/// ── Phase 1: 并行瓶颈分析（对每个趋势）──
///   a-chain-decomposer   → 供应链图谱
///   a-chokepoint-id      → 瓶颈验证
///
/// ── Phase 2: 候选映射 ──
///   t-candidates          → 财务数据验证
///   a-candidate-mapper    → 最终候选股清单
///
/// ── Phase 3: 持久化 ──
///   StorageNode → 写入 serenity_candidate_pool 表，
///                 供 SerenityStrategy 读取作为 seed pool
///
/// 输入：无（自驱动，从市场数据中发现趋势）
/// 输出：JSON { candidates: [{stock_code, stock_name, serenity_score, ...}, ...] }
async fn seed_serenity_screening_workflow_template(
    db: &sea_orm::DatabaseConnection,
) -> Result<(), String> {
    use axagent_core::entity::workflow_template;
    use axagent_harness::workflow_types::{
        AgentNode, AgentNodeConfig, EdgeType, JsonSchema, JsonSchemaProperty, OutputMode, Position,
        RetryConfig, StorageNode, StorageNodeConfig, ToolDef, ToolNode, ToolNodeConfig,
        TriggerConfig, TriggerNode, TriggerType, Variable, WorkflowEdge, WorkflowNode,
        WorkflowNodeBase,
    };
    use sea_orm::{ActiveModelTrait, EntityTrait, Set};

    const TEMPLATE_ID: &str = "serenity-screening";
    const TEMPLATE_VERSION: i32 = 9;

    // 检查模板是否已存在且是最新版本
    if let Some(existing) = workflow_template::Entity::find_by_id(TEMPLATE_ID)
        .one(db)
        .await
        .map_err(|e| format!("查询工作流模板失败: {e}"))?
    {
        if existing.version >= TEMPLATE_VERSION {
            tracing::info!(
                "[stock_analysis_setup] Serenity 模板已是最新 v{TEMPLATE_VERSION}，跳过"
            );
            return Ok(());
        }
        tracing::info!(
            "[stock_analysis_setup] 更新 Serenity 模板 v{} → v{TEMPLATE_VERSION}",
            existing.version
        );
    }

    let now = chrono::Utc::now().timestamp_millis();

    // ── ToolDef 定义 ──
    let td_hot = ToolDef {
        name: "get_hot_stocks".into(),
        description: Some("获取市场热门股".into()),
        parameters: None,
    };
    let td_industry = ToolDef {
        name: "get_industry_ranking".into(),
        description: Some("获取行业涨跌排名".into()),
        parameters: None,
    };
    let td_cls = ToolDef {
        name: "get_cls_flash".into(),
        description: Some("获取财联社实时快讯".into()),
        parameters: None,
    };
    let td_concept = ToolDef {
        name: "get_concept_blocks".into(),
        description: Some("获取概念板块归属".into()),
        parameters: None,
    };
    let td_north = ToolDef {
        name: "get_north_bound_flow".into(),
        description: Some("获取北向资金流向".into()),
        parameters: None,
    };
    let td_dragon = ToolDef {
        name: "get_market_dragon_tiger".into(),
        description: Some("获取龙虎榜数据".into()),
        parameters: None,
    };
    let td_fin = ToolDef {
        name: "get_stock_financials".into(),
        description: Some("获取财务数据：营收、净利润、EPS、ROE、毛利率等".into()),
        parameters: Some(JsonSchema {
            schema_type: "object".into(),
            description: None,
            properties: Some(std::collections::HashMap::from([(
                "stock_code".into(),
                JsonSchemaProperty {
                    schema_type: "string".into(),
                    description: Some("6位股票代码".into()),
                    default: None,
                    enum_values: None,
                    format: None,
                },
            )])),
            required: Some(vec!["stock_code".into()]),
            items: None,
        }),
    };
    let td_quote = ToolDef {
        name: "get_stock_quote".into(),
        description: Some("获取股票实时行情".into()),
        parameters: Some(JsonSchema {
            schema_type: "object".into(),
            description: None,
            properties: Some(std::collections::HashMap::from([(
                "stock_code".into(),
                JsonSchemaProperty {
                    schema_type: "string".into(),
                    description: Some("6位股票代码".into()),
                    default: None,
                    enum_values: None,
                    format: None,
                },
            )])),
            required: Some(vec!["stock_code".into()]),
            items: None,
        }),
    };
    let td_visits = ToolDef {
        name: "get_institutional_visits".into(),
        description: Some("获取机构调研数据".into()),
        parameters: Some(JsonSchema {
            schema_type: "object".into(),
            description: None,
            properties: Some(std::collections::HashMap::from([(
                "stock_code".into(),
                JsonSchemaProperty {
                    schema_type: "string".into(),
                    description: Some("6位股票代码".into()),
                    default: None,
                    enum_values: None,
                    format: None,
                },
            )])),
            required: Some(vec!["stock_code".into()]),
            items: None,
        }),
    };
    // 新闻 + 研报工具，供 Agent 节点动态搜索验证（催化剂/CapEx/退出信号）
    let td_serenity_news = ToolDef {
        name: "get_stock_news".into(),
        description: Some("获取个股近期新闻公告，验证催化剂/退出信号".into()),
        parameters: Some(JsonSchema {
            schema_type: "object".into(),
            description: None,
            properties: Some(std::collections::HashMap::from([
                (
                    "stock_code".into(),
                    JsonSchemaProperty {
                        schema_type: "string".into(),
                        description: Some("6位股票代码".into()),
                        default: None,
                        enum_values: None,
                        format: None,
                    },
                ),
                (
                    "limit".into(),
                    JsonSchemaProperty {
                        schema_type: "integer".into(),
                        description: Some("返回数量".into()),
                        default: Some(serde_json::json!(30)),
                        enum_values: None,
                        format: None,
                    },
                ),
            ])),
            required: Some(vec!["stock_code".into()]),
            items: None,
        }),
    };
    let td_serenity_research = ToolDef {
        name: "get_research_reports".into(),
        description: Some("获取券商研报，验证需求/壁垒/CapEx逻辑".into()),
        parameters: Some(JsonSchema {
            schema_type: "object".into(),
            description: None,
            properties: Some(std::collections::HashMap::from([(
                "stock_code".into(),
                JsonSchemaProperty {
                    schema_type: "string".into(),
                    description: Some("6位股票代码".into()),
                    default: None,
                    enum_values: None,
                    format: None,
                },
            )])),
            required: Some(vec!["stock_code".into()]),
            items: None,
        }),
    };
    // 关键词新闻搜索工具，供 Agent 验证催化剂/CapEx/行业趋势
    let td_search_news = ToolDef {
        name: "search_news".into(),
        description: Some("按关键词搜索财经新闻，用于验证催化剂/CapEx/行业趋势".into()),
        parameters: Some(JsonSchema {
            schema_type: "object".into(),
            description: None,
            properties: Some(std::collections::HashMap::from([
                (
                    "keyword".into(),
                    JsonSchemaProperty {
                        schema_type: "string".into(),
                        description: Some("搜索关键词".into()),
                        default: None,
                        enum_values: None,
                        format: None,
                    },
                ),
                (
                    "limit".into(),
                    JsonSchemaProperty {
                        schema_type: "integer".into(),
                        description: Some("返回条数".into()),
                        default: Some(serde_json::json!(10)),
                        enum_values: None,
                        format: None,
                    },
                ),
            ])),
            required: Some(vec!["keyword".into()]),
            items: None,
        }),
    };

    // 关注度评分工具（方案B）
    let td_attention = ToolDef {
        name: "compute_attention_score".into(),
        description: Some("计算个股关注度评分 0-100，越低越冷门，用于验证低关注度因子".into()),
        parameters: Some(JsonSchema {
            schema_type: "object".into(),
            description: None,
            properties: Some(std::collections::HashMap::from([(
                "stock_code".into(),
                JsonSchemaProperty {
                    schema_type: "string".into(),
                    description: Some("6位股票代码".into()),
                    default: None,
                    enum_values: None,
                    format: None,
                },
            )])),
            required: Some(vec!["stock_code".into()]),
            items: None,
        }),
    };
    // 行业竞争地位分析工具（方案C）
    let td_industry_pos = ToolDef {
        name: "compute_industry_position".into(),
        description: Some(
            "行业竞争地位分析：同行对比毛利率/ROE，产能指标（资本开支/折旧比）".into(),
        ),
        parameters: Some(JsonSchema {
            schema_type: "object".into(),
            description: None,
            properties: Some(std::collections::HashMap::from([(
                "stock_code".into(),
                JsonSchemaProperty {
                    schema_type: "string".into(),
                    description: Some("6位股票代码".into()),
                    default: None,
                    enum_values: None,
                    format: None,
                },
            )])),
            required: Some(vec!["stock_code".into()]),
            items: None,
        }),
    };

    // 退出信号检查工具（Phase 3）
    let td_exit = ToolDef {
        name: "check_exit_signals".into(),
        description: Some(
            "检查个股退出信号：价格止损、技术替代新闻、毛利率趋势。返回 exit_urgency".into(),
        ),
        parameters: Some(JsonSchema {
            schema_type: "object".into(),
            description: None,
            properties: Some(std::collections::HashMap::from([
                (
                    "stock_code".into(),
                    JsonSchemaProperty {
                        schema_type: "string".into(),
                        description: Some("6位股票代码".into()),
                        default: None,
                        enum_values: None,
                        format: None,
                    },
                ),
                (
                    "entry_price".into(),
                    JsonSchemaProperty {
                        schema_type: "number".into(),
                        description: Some("买入价（用于计算止损触发）".into()),
                        default: None,
                        enum_values: None,
                        format: None,
                    },
                ),
                (
                    "stop_loss_price".into(),
                    JsonSchemaProperty {
                        schema_type: "number".into(),
                        description: Some("止损价".into()),
                        default: None,
                        enum_values: None,
                        format: None,
                    },
                ),
            ])),
            required: Some(vec!["stock_code".into()]),
            items: None,
        }),
    };

    // 回馈闭环工具
    let td_perf = ToolDef {
        name: "compute_serenity_performance".into(),
        description: Some("计算 Serenity 候选推荐后表现".into()),
        parameters: Some(JsonSchema {
            schema_type: "object".into(),
            description: None,
            properties: Some(std::collections::HashMap::from([
                (
                    "stock_code".into(),
                    JsonSchemaProperty {
                        schema_type: "string".into(),
                        description: Some("6位股票代码".into()),
                        default: None,
                        enum_values: None,
                        format: None,
                    },
                ),
                (
                    "recommend_date".into(),
                    JsonSchemaProperty {
                        schema_type: "string".into(),
                        description: Some("推荐日期 YYYY-MM-DD".into()),
                        default: None,
                        enum_values: None,
                        format: None,
                    },
                ),
            ])),
            required: Some(vec!["stock_code".into(), "recommend_date".into()]),
            items: None,
        }),
    };
    let td_cat = ToolDef {
        name: "verify_catalysts".into(),
        description: Some("验证 Serenity 候选的催化剂是否兑现".into()),
        parameters: Some(JsonSchema {
            schema_type: "object".into(),
            description: None,
            properties: Some(std::collections::HashMap::from([
                (
                    "stock_code".into(),
                    JsonSchemaProperty {
                        schema_type: "string".into(),
                        description: Some("6位股票代码".into()),
                        default: None,
                        enum_values: None,
                        format: None,
                    },
                ),
                (
                    "catalyst_descriptions".into(),
                    JsonSchemaProperty {
                        schema_type: "array".into(),
                        description: Some("催化剂描述列表".into()),
                        default: None,
                        enum_values: None,
                        format: None,
                    },
                ),
            ])),
            required: Some(vec!["stock_code".into()]),
            items: None,
        }),
    };
    let td_opt = ToolDef {
        name: "optimize_attention_weights".into(),
        description: Some("基于历史表现调优关注度评分权重".into()),
        parameters: Some(JsonSchema {
            schema_type: "object".into(),
            description: None,
            properties: Some(std::collections::HashMap::from([(
                "samples".into(),
                JsonSchemaProperty {
                    schema_type: "array".into(),
                    description: Some("样本列表".into()),
                    default: None,
                    enum_values: None,
                    format: None,
                },
            )])),
            required: Some(vec!["samples".into()]),
            items: None,
        }),
    };

    let tool_defs: Vec<ToolDef> = vec![
        td_hot,
        td_industry,
        td_cls,
        td_concept,
        td_north,
        td_dragon,
        td_fin,
        td_quote,
        td_visits,
        td_serenity_news,
        td_serenity_research,
        td_search_news,
        td_attention,
        td_industry_pos,
        td_exit,
        td_perf,
        td_cat,
        td_opt,
    ];
    let tool_defs_json =
        serde_json::to_string(&tool_defs).map_err(|e| format!("序列化 ToolDef 失败: {e}"))?;

    // ── 快捷构建函数 ──
    let tool_node = |id: &str,
                     title: &str,
                     tool_name: &str,
                     output_var: &str,
                     x: f64,
                     y: f64|
     -> WorkflowNode {
        WorkflowNode::Tool(ToolNode {
            base: WorkflowNodeBase {
                id: id.into(),
                title: title.into(),
                description: Some(format!("获取数据: {tool_name}")),
                position: Position { x, y },
                retry: RetryConfig {
                    enabled: true,
                    max_retries: 2,
                    ..Default::default()
                },
                timeout: Some(120),
                enabled: true,
                parent_id: None,
                compensation: None,
            },
            config: ToolNodeConfig {
                tool_name: tool_name.into(),
                input_mapping: std::collections::HashMap::new(),
                output_var: output_var.into(),
            },
        })
    };

    let agent_node = |id: &str,
                      title: &str,
                      expert_id: &str,
                      system_prompt: &str,
                      context_sources: Vec<&str>,
                      input_mapping: std::collections::HashMap<String, String>,
                      x: f64,
                      y: f64|
     -> WorkflowNode {
        WorkflowNode::Agent(AgentNode {
            base: WorkflowNodeBase {
                id: id.into(),
                title: title.into(),
                description: Some(format!("Serenity 分析: {expert_id}")),
                position: Position { x, y },
                retry: RetryConfig {
                    enabled: true,
                    max_retries: 2,
                    ..Default::default()
                },
                timeout: Some(300),
                enabled: true,
                parent_id: None,
                compensation: None,
            },
            config: AgentNodeConfig {
                system_prompt: system_prompt.into(),
                context_sources: context_sources.into_iter().map(String::from).collect(),
                input_mapping,
                output_var: id.into(),
                model: None,
                temperature: Some(0.3),
                max_tokens: Some(8192),
                tools: vec![],
                exposed_tools: vec![],
                output_mode: OutputMode::Json,
                agent_profile_id: Some(format!("stock-{expert_id}")),
                max_tool_rounds: None,
                execution_mode: None,
                rag_source_ids: vec![],
                model_role: None,
                consistency_check: None,
                hallucination_guard: None,
            },
        })
    };

    let edge = |id: &str, source: &str, target: &str| -> WorkflowEdge {
        WorkflowEdge {
            id: id.into(),
            source: source.into(),
            source_handle: None,
            target: target.into(),
            target_handle: None,
            edge_type: EdgeType::Direct,
            label: None,
        }
    };

    // ── 构建节点 ──
    let mut nodes: Vec<WorkflowNode> = Vec::new();
    let mut edges: Vec<WorkflowEdge> = Vec::new();

    // Trigger: 手动触发，无需参数（自驱动扫描市场）
    nodes.push(WorkflowNode::Trigger(TriggerNode {
        base: WorkflowNodeBase {
            id: "trigger".into(),
            title: "启动 Serenity 筛选".into(),
            description: Some("自动扫描市场数据，发现产业瓶颈机会".into()),
            position: Position { x: 340.0, y: 0.0 },
            retry: RetryConfig::default(),
            timeout: None,
            enabled: true,
            parent_id: None,
            compensation: None,
        },
        config: TriggerConfig {
            trigger_type: TriggerType::Manual,
            config: serde_json::json!({
                "description": "Serenity 瓶颈筛选: 从市场数据中识别瓶颈环节候选公司",
                "required_params": []
            }),
        },
    }));

    // ── Phase 0: 数据采集工具（并行） ──
    let t_names = [
        ("t-hot-stocks", "市场热门股", "get_hot_stocks", "t-hot-stocks", 40.0, 80.0),
        (
            "t-industry-rank",
            "行业排名",
            "get_industry_ranking",
            "t-industry-rank",
            240.0,
            80.0,
        ),
        ("t-cls-flash", "实时快讯", "get_cls_flash", "t-cls-flash", 440.0, 80.0),
        ("t-northbound", "北向资金", "get_north_bound_flow", "t-northbound", 840.0, 80.0),
    ];
    let t_trend_ids: Vec<&str> = t_names.iter().map(|(id, _, _, _, _, _)| *id).collect();
    for (id, title, tool, output, x, y) in &t_names {
        nodes.push(tool_node(id, title, tool, output, *x, *y));
        edges.push(edge(&format!("e-trigger-{id}"), "trigger", id));
    }

    // ── a-trend-scanner: 综合分析，输出 2-3 个趋势 ──
    // 强约束输出：必须且只能输出一个 tool_json 代码块，无任何前后文。
    // tool_json 块由项目 IR Normalizer 直接解析为 ContentBlock::ToolUse。
    let trend_scanner_prompt = "你的任务：综合分析市场热门股、行业排名、实时快讯、北向资金流向，\
         识别出当前最具潜力的 2-3 个产业方向。\
         \n\n\
         核心原则：\n\
         1. 排除已过度上涨的赛道（近 1 月板块涨幅 > 30%）。\n\
         2. 只选「萌芽→加速」阶段的产业方向，不要已经充分定价的热点。\n\
         3. **需求确定性是前提**：每个趋势必须有可验证的 CapEx/订单/政策证据支撑。\
         纯 LLM 推测（\"未来可能增长\"）不可接受。\n\
         4. 每个趋势必须给出明确的上下游因果链。\n\
         5. 必须输出一个 bottleneck_candidate（初步判断的瓶颈环节）。\n\
         \n\
         ============== 输出格式强约束（必须严格遵守） ==============\n\
         1. 你的回复必须且只能包含一个代码块，开头三个反引号紧跟 tool_json。\n\
         2. 代码块内容为单一 JSON 对象，结构：{\"name\": \"submit_trends\", \"arguments\": <数据>}\n\
         3. <数据> 字段即下面的趋势数据。\n\
         4. 代码块外禁止任何文字：不要写\"以下是...\"、\"输出：\"、注释、解释、前缀、后缀。\n\
         5. 字段值为空时用 null，不要省略字段。\n\
         6. 数字字段（confidence 等）必须是 JSON 数字，不要加引号。\n\
         7. 严禁在 JSON 字符串值中夹带思考文字或自述注解。\n\
         ============================================================\n\
         \n\
         <数据> 结构：\n\
         {\"trends\": [{\"trend_name\": \"...\", \"confidence\": 75, \"phase\": \"accelerating\",\
         \"core_logic\": \"...\", \"causal_chain\": \"...\",\
         \"bottleneck_candidate\": \"...\", \"bottleneck_rationale\": \"...\",\
         \"demand_evidence\": {\"type\": \"capex | policy_mandate | order_backlog\",\
         \"source\": \"具体证据来源\", \"confidence\": 75, \"detail\": \"...\"},\
         \"downstream_giants\": [\"直接受益/推动的下游巨头名称\"]}]}\n\
         \n\n\
         重要：如果获取到的数据不足，基于你已知的公开信息和市场常识给出合理推断，\
         不要只列 data_gaps。严禁使用'数据缺失'、'无法获取'等负面措辞。";
    nodes.push(agent_node(
        "a-trend-scanner",
        "产业趋势扫描",
        "trend-scanner",
        trend_scanner_prompt,
        t_trend_ids.clone(),
        std::collections::HashMap::new(),
        340.0,
        180.0,
    ));
    for tid in &t_trend_ids {
        edges.push(edge(&format!("e-{tid}-a-trend-scanner"), tid, "a-trend-scanner"));
    }

    // ── Phase 1: 对每个趋势拆解产业链+瓶颈鉴定 ──
    // 使用 3 个并行的 chain-decomposer + 3 个 chokepoint-identifier
    let trend_names = ["trend1", "trend2", "trend3"];
    let trend_x_positions = [100.0, 340.0, 580.0];

    for (i, tn) in trend_names.iter().enumerate() {
        let decomposer_id = format!("a-chain-{tn}");
        let decomposer_prompt = format!(
            "你的任务：对上游 a-trend-scanner 输出的趋势 #{i} 进行产业链拆解。\
             将产业从上到下拆解为 5-8 个关键环节，标注每个环节的供应商数量、技术壁垒、扩产周期。\
             \n\n\
             核心要求：\n\
             1. 拆解到具体产品或工艺层面（如 HBM3E 环氧塑封料）。\n\
             2. 每个环节必须标注 global_supplier_count / tech_barrier / expansion_cycle_months。\n\
             3. 标注 bottleneck_potential（high/medium/low）及理由。\n\
             4. **额外标注每个环节的需求验证信息**：直接下游厂商是谁、最终需求驱动方、是否有已公开的\
             订单/合同负债/CapEx 支撑。\n\
             \n\n\
             ============== 输出格式强约束（必须严格遵守） ==============\n\
             1. 你的回复必须且只能包含一个代码块，开头三个反引号紧跟 tool_json。\n\
             2. 代码块内容为单一 JSON 对象，结构：{{\"name\": \"submit_chain\", \"arguments\": <数据>}}\n\
             3. <数据> 字段即下面的产业链数据。\n\
             4. 代码块外禁止任何文字：不要写\"以下是...\"、\"输出：\"、注释、解释、前缀、后缀。\n\
             5. 字段值为空时用 null，不要省略字段。\n\
             6. 数字字段（global_supplier_count、expansion_cycle_months 等）必须是 JSON 数字。\n\
             7. 严禁在 JSON 字符串值中夹带思考文字或自述注解。\n\
             ============================================================\n\
             \n\n\
             <数据> 结构：\n\
             {{\"trend_name\": \"...\", \"chain_nodes\": [{{\"node_name\": \"...\",\
             \"global_supplier_count\": 3, \"tech_barrier\": \"high\",\
             \"expansion_cycle_months\": 24, \"bottleneck_potential\": \"high\",\
             \"bottleneck_rationale\": \"...\",\
             \"demand_validation\": {{\"direct_downstream\": \"直接下游厂商\",\
             \"final_demand_driver\": \"最终需求驱动方\", \"demand_certainty\": \"high | medium | low\",\
             \"evidence\": \"关键证据，如英伟达 FY2025 CapEx $80B\",\
             \"order_visibility\": \"有已公开长协/订单 | 合同负债增长 | 产能预订 | 无公开证据\"}}}}]}}"
        );
        nodes.push(agent_node(
            &decomposer_id,
            &format!("产业链拆解 #{i}"),
            "chain-decomposer",
            &decomposer_prompt,
            vec!["a-trend-scanner"],
            std::collections::HashMap::new(),
            trend_x_positions[i],
            300.0,
        ));
        edges.push(edge(
            &format!("e-a-trend-scanner-{decomposer_id}"),
            "a-trend-scanner",
            &decomposer_id,
        ));

        // chokepoint-identifier 接在 chain-decomposer 之后
        let chokepoint_id = format!("a-chokepoint-{tn}");
        let chokepoint_prompt = format!(
            "你的任务：对上游产业链拆解结果（trend #{i}）进行瓶颈验证。\
             从供给刚性、需求弹性、不可替代性三个维度量化评分。\
             \n\n\
             核心要求：\n\
             1. composite_score >= 80 才是真正的瓶颈（三力评分都 >= 70）。\n\
             2. 区分 capacity 和 technology 两类瓶颈，technology 更偏好。\n\
             3. 给出至少 1 个 A 股候选公司（需含具体 stock_code）。\n\
             4. **追加催化剂识别**：每个验证的瓶颈必须给出近期（1-6 月）的催化剂事件。\
             催化剂类型包括：财报/客户量产/政策节点/供给冲击/产能释放。\n\
             5. **需求确定性验证**：验证下游需求是否由\
             巨头 CapEx 指引/政府专项/强制性法规/已签长协支撑。\
             使用 search_news 工具主动搜索关键词如\"英伟达 CapEx\"、\"台积电 扩产\"来获取真实新闻证据。\
             纯推测性需求不可接受。\n\
             \n\n\
             ============== 输出格式强约束（必须严格遵守） ==============\n\
             1. 你的回复必须且只能包含一个代码块，开头三个反引号紧跟 tool_json。\n\
             2. 代码块内容为单一 JSON 对象，结构：{{\"name\": \"submit_bottleneck\", \"arguments\": <数据>}}\n\
             3. <数据> 字段即下面的瓶颈验证数据。\n\
             4. 代码块外禁止任何文字：不要写\"以下是...\"、\"输出：\"、注释、解释、前缀、后缀。\n\
             5. 字段值为空时用 null，不要省略字段。\n\
             6. 数字字段（composite_score、scores.*）必须是 JSON 数字。\n\
             7. 严禁在 JSON 字符串值中夹带思考文字或自述注解。\n\
             ============================================================\n\
             \n\n\
             <数据> 结构：\n\
             {{\"verified_bottleneck\": {{\"node_name\": \"...\", \"composite_score\": 85,\
             \"bottleneck_type\": \"technology\",\
             \"scores\": {{\"supply_rigidity\": 85, \"demand_elasticity\": 80, \"irreplaceability\": 90}},\
             \"catalysts\": [{{\"type\": \"earnings | production_ramp | policy | supply_shock\",\
             \"description\": \"催化剂描述\", \"expected_timeframe\": \"short_term | mid_term | long_term\",\
             \"confidence\": 70, \"trigger_condition\": \"触发条件\"}}],\
             \"a_share_candidates\": [{{\"stock_code\": \"...\", \"stock_name\": \"...\",\
             \"relevance\": \"direct\", \"advantage\": \"...\"}}]}}}}"
        );
        nodes.push(agent_node(
            &chokepoint_id,
            &format!("瓶颈鉴定 #{i}"),
            "chokepoint-identifier",
            &chokepoint_prompt,
            vec![&decomposer_id],
            std::collections::HashMap::new(),
            trend_x_positions[i],
            420.0,
        ));
        edges.push(edge(
            &format!("e-{decomposer_id}-{chokepoint_id}"),
            &decomposer_id,
            &chokepoint_id,
        ));
    }

    // ── Phase 2: 候选公司映射 ──
    // a-candidate-mapper: Agent 直接调用工具筛选，无需前置 ToolNode。
    // 综合所有瓶颈鉴定结果，输出最终候选股清单（含催化剂、退出信号、关注度评分）
    let mapper_prompt = "你的任务：综合所有瓶颈鉴定结果，对候选公司进行二次筛选和打分。\
         \n\n\
         ⚠️ 字段硬性要求（违反则输出无效）：\
         \n\
         - 每个 candidates 数组元素**必须**包含 stock_code（6位数字）和 stock_name（中文简称）。\
         \n- 缺少这两个字段任一的候选将被系统自动丢弃，前端不会显示。\
         \n- stock_code 必须是真实 A 股代码（如 300285、002371），不可用占位符。\
         \n\n\
         核心原则：\n\
         1. 优先选择市值 50-500 亿、机构覆盖少的公司（Serenity 偏好）。\n\
         2. 客户质量高于一切：已进入头部客户供应链的优先级 > 有技术但无客户验证的。\n\
         3. 排除股价已过度上涨的（近 3 月 > 100% 涨幅）。\n\
         4. 排除高负债率（> 70%）或频繁定增的公司。\n\
         5. **催化剂决定何时买入**：每个候选必须至少给出 1 个近期催化剂（财报/量产/政策/供给冲击/产能释放）。\
         没有催化剂的候选不得输出。\n\
         6. **退出信号决定何时卖出**：每个候选必须评估技术替代、产能过剩、新进入者、需求放缓四大退出风险。\
         exit_now 的候选直接排除。\n\
         7. **低关注度量化**：评估机构覆盖变化、搜索热度、相对交易量、市场预期差。\
         关注度越低弹性越大，attention_score > 70 扣分。\n\
         8. **需求确定性验证**：检查上级节点提供的 demand_validation/demand_evidence，\
         确保需求由 CapEx/订单/政策硬证据支撑而非 LLM 推测。\
         使用 search_news 工具搜索关键词验证，如搜索\"英伟达 CapEx\"确认需求真实性。无硬证据扣 20 分。\n\
         9. 每个候选必须给出具体的 serenity_score 和风险提示。\n\
        10. **输出数量**：输出 3-5 个最优质的候选公司，宁缺毋滥。\
        若某个瓶颈趋势下所有公司都不符合标准，不要强行输出。\
         \n\n\
         ============== 输出格式强约束（必须严格遵守） ==============\n\
         1. 你的回复必须且只能包含一个代码块，开头三个反引号紧跟 tool_json。\n\
         2. 代码块内容为单一 JSON 对象，结构：{\"name\": \"submit_candidates\", \"arguments\": <数据>}\n\
         3. <数据> 字段即下面的候选股数据。\n\
         4. 代码块外禁止任何文字：不要写\"以下是...\"、\"输出：\"、注释、解释、前缀、后缀。\n\
         5. 字段值为空时用 null，不要省略字段。\n\
         6. 数字字段（serenity_score、confidence 等）必须是 JSON 数字，不要加引号。\n\
         7. 严禁在 JSON 字符串值中夹带思考文字或自述注解。\n\
         ============================================================\n\
         \n\n\
         <数据> 结构：\n\
         {\"candidates\": [{\"stock_code\": \"300285\", \"stock_name\": \"国瓷材料\",\
         \"relevance\": \"direct\", \"serenity_score\": 75, \"confidence\": 70,\
         \"bottleneck_product\": \"半导体级高纯氮化铝(AlN)粉体\",\
         \"primary_risk\": \"客户验证周期较长\",\
         \"catalysts\": [{\"type\": \"earnings | production_ramp | policy | supply_shock\",\
         \"description\": \"催化剂描述\", \"expected_timeframe\": \"short_term | mid_term | long_term\",\
         \"confidence\": 70, \"trigger_condition\": \"触发条件\"}],\
         \"exit_signals\": {\"technology_disruption_risk\": \"风险描述\",\
         \"capacity_oversupply_risk\": \"风险描述\", \"new_entrant_risk\": \"风险描述\",\
         \"demand_slowdown_risk\": \"风险描述\",\
         \"overall_exit_urgency\": \"no_urgency | watch | caution | exit_now\"},\
         \"attention_metrics\": {\"coverage_change_3m\": \"新增 N 篇 | 减少 N 篇 | 无变化\",\
         \"search_heat\": \"冷门 | 正常 | 热门\",\
         \"relative_volume\": \"低于均值 N% | 正常 | 高于均值 N%\",\
         \"consensus_gap\": \"明显低估 | 合理 | 高估\", \"attention_score\": 30}}], \"summary\": \"...\"}";
    nodes.push(agent_node(
        "a-candidate-mapper",
        "候选公司筛选",
        "candidate-mapper",
        mapper_prompt,
        vec![
            "a-chokepoint-trend1",
            "a-chokepoint-trend2",
            "a-chokepoint-trend3",
        ],
        std::collections::HashMap::new(),
        340.0,
        660.0,
    ));
    for tn in &trend_names {
        let cid = format!("a-chokepoint-{tn}");
        edges.push(edge(&format!("e-{cid}-a-candidate-mapper"), &cid, "a-candidate-mapper"));
    }

    // ── StorageNode: 持久化候选结果 ──
    nodes.push(WorkflowNode::Storage(StorageNode {
        base: WorkflowNodeBase {
            id: "s-save-candidates".into(),
            title: "保存候选结果".into(),
            description: Some("将 Serenity 筛选结果持久化到 serenity_candidate_pool 表".into()),
            position: Position { x: 340.0, y: 780.0 },
            retry: RetryConfig::default(),
            timeout: Some(30),
            enabled: true,
            parent_id: None,
            compensation: None,
        },
        config: StorageNodeConfig {
            backend: "sqlite".into(),
            operation: "insert".into(),
            input_var: "a-candidate-mapper".into(),
            collection: "serenity_candidates".into(),
            key_var: None,
            output_var: "s-save-candidates-result".into(),
        },
    }));
    edges.push(edge("e-a-candidate-mapper-s-save", "a-candidate-mapper", "s-save-candidates"));

    // ── 序列化 ──
    let nodes_json = serde_json::to_string(&nodes).map_err(|e| format!("序列化节点失败: {e}"))?;
    let edges_json = serde_json::to_string(&edges).map_err(|e| format!("序列化边失败: {e}"))?;

    // ── Variables ──
    let variables_json = serde_json::to_string(&Vec::<Variable>::new())
        .map_err(|e| format!("序列化变量失败: {e}"))?;

    // ── Tags ──
    let tags_json = serde_json::to_string(&["serenity", "bottleneck", "screening"])
        .map_err(|e| format!("序列化标签失败: {e}"))?;

    // ── 写入 DB ──
    let _ = workflow_template::Entity::delete_by_id(TEMPLATE_ID)
        .exec(db)
        .await;
    workflow_template::ActiveModel {
        id: Set(TEMPLATE_ID.to_string()),
        name: Set("Serenity 瓶颈筛选".to_string()),
        description: Set(Some(
            "自动扫描市场数据，识别产业瓶颈环节，输出候选股清单（Serenity 投资方法论）".to_string(),
        )),
        icon: Set("search".into()),
        tags: Set(Some(tags_json)),
        version: Set(TEMPLATE_VERSION),
        is_preset: Set(true),
        is_editable: Set(true),
        is_public: Set(true),
        trigger_config: Set(Some(
            serde_json::to_string(&TriggerConfig {
                trigger_type: TriggerType::Manual,
                config: serde_json::json!({
                    "description": "Serenity 瓶颈筛选: 自动扫描市场发现产业链瓶颈机会",
                    "required_params": []
                }),
            })
            .map_err(|e| format!("序列化触发器配置失败: {e}"))?,
        )),
        nodes: Set(nodes_json),
        edges: Set(edges_json),
        input_schema: Set(None),
        output_schema: Set(None),
        variables: Set(Some(variables_json)),
        error_config: Set(None),
        composite_source: Set(None),
        tool_defs: Set(Some(tool_defs_json)),
        created_at: Set(now),
        updated_at: Set(now),
    }
    .insert(db)
    .await
    .map_err(|e| format!("写入 Serenity 模板失败: {e}"))?;

    tracing::info!("[stock_analysis_setup] Serenity 瓶颈筛选工作流模板已创建 (serenity-screening)");
    Ok(())
}
