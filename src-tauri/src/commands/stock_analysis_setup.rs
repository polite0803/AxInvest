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
        "portfolio-manager",
        include_str!("../../agency_experts/stock-analysis/portfolio-manager.md"),
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
    ("aggressive-debator", "risk-evaluator"),
    ("conservative-debator", "risk-evaluator"),
    ("neutral-debator", "risk-evaluator"),
    ("research-manager", "decision-maker"),
    ("trader", "trader"),
    ("portfolio-manager", "decision-maker"),
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
        max_concurrent: 7,
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

pub async fn ensure_stock_analysis_experts_seeded(
    db: &sea_orm::DatabaseConnection,
) -> Result<(), String> {
    seed_agency_experts(db).await?;
    seed_agent_roles(db).await?;
    seed_agent_profiles(db).await?;
    seed_stock_analysis_workflow_template(db).await?;
    Ok(())
}

/// 将股票分析 DAG 作为工作流模板持久化到 workflow_templates 表。
/// 模板中的 system_prompt 使用 {{stock_code}} / {{stock_name}} / {{data_ctx}} 占位符，
/// 运行时由 run_stock_workflow 替换为实际行情数据。
async fn seed_stock_analysis_workflow_template(
    db: &sea_orm::DatabaseConnection,
) -> Result<(), String> {
    use axagent_core::entity::workflow_template;
    use axagent_core::workflow_types::{
        AgentNode, AgentNodeConfig, EdgeType, JsonSchema, JsonSchemaProperty, OutputMode, Position,
        RetryConfig, ToolDef, ToolNode, ToolNodeConfig, TriggerConfig, TriggerNode, TriggerType,
        Variable, WorkflowEdge, WorkflowNode, WorkflowNodeBase,
    };
    use sea_orm::{ActiveModelTrait, EntityTrait, Set};

    const TEMPLATE_ID: &str = "stock-analysis";
    const TEMPLATE_VERSION: i32 = 9;

    if let Some(existing) = workflow_template::Entity::find_by_id(TEMPLATE_ID)
        .one(db)
        .await
        .map_err(|e| format!("查询工作流模板失败: {e}"))?
    {
        if existing.version >= TEMPLATE_VERSION {
            return Ok(()); // 版本已最新，跳过
        }
        // 旧版本 → 删除重建
        workflow_template::Entity::delete_by_id(TEMPLATE_ID)
            .exec(db)
            .await
            .map_err(|e| format!("删除旧模板失败: {e}"))?;
        tracing::info!(
            "[stock_analysis_setup] 更新股票分析工作流模板 v{} → v{TEMPLATE_VERSION}",
            existing.version
        );
    }

    let now = chrono::Utc::now().timestamp_millis();

    let tool_node =
        |id: &str, title: &str, tool_name: &str, output_var: &str, arg_key: &str| -> WorkflowNode {
            let mut input_mapping = std::collections::HashMap::new();
            input_mapping.insert(arg_key.to_string(), "trigger.config.stock_code".to_string());
            WorkflowNode::Tool(ToolNode {
                base: WorkflowNodeBase {
                    id: id.into(),
                    title: title.into(),
                    description: Some(format!("获取数据: {tool_name}")),
                    position: Position { x: 0.0, y: 0.0 },
                    retry: RetryConfig {
                        enabled: true,
                        max_retries: 2,
                        ..Default::default()
                    },
                    timeout: Some(30),
                    enabled: true,
                },
                config: ToolNodeConfig {
                    tool_name: tool_name.into(),
                    input_mapping,
                    output_var: output_var.into(),
                },
            })
        };

    // 常用工具定义
    let td_quote = ToolDef {
        name: "get_stock_quote".into(),
        description: Some("获取股票实时行情：现价、涨跌幅、PE、PB、市值".into()),
        parameters: None,
    };
    let td_kline = ToolDef {
        name: "get_stock_kline".into(),
        description: Some("获取K线数据：OHLCV，可指定周期和数量".into()),
        parameters: None,
    };
    let td_fin = ToolDef {
        name: "get_stock_financials".into(),
        description: Some("获取财务数据：营收、净利润、EPS、ROE、毛利率等".into()),
        parameters: None,
    };
    let td_mf = ToolDef {
        name: "get_stock_money_flow".into(),
        description: Some("获取资金流向：主力/超大单/大单/中单/小单净流入".into()),
        parameters: None,
    };
    let td_score = ToolDef {
        name: "compute_scoring".into(),
        description: Some("计算技术评分：基于趋势、偏离度、MACD、成交量、RSI、支撑阻力".into()),
        parameters: None,
    };
    let td_val = ToolDef {
        name: "compute_valuation".into(),
        description: Some("计算估值指标：DCF、F-Score、护城河量化、安全边际".into()),
        parameters: None,
    };
    let td_risk = ToolDef {
        name: "compute_portfolio_risk".into(),
        description: Some("计算组合风险：总市值、集中度、风险等级".into()),
        parameters: None,
    };
    let td_quality = ToolDef {
        name: "run_quality_gate".into(),
        description: Some("运行质量门控：检查各分析报告的一致性和完整性".into()),
        parameters: None,
    };
    // ── 新增 12 个金融模型 ToolDef ──
    let td_maxdd = ToolDef {
        name: "calc_max_drawdown".into(),
        description: Some("计算最大回撤比例".into()),
        parameters: None,
    };
    let td_sharpe = ToolDef {
        name: "calc_sharpe_ratio".into(),
        description: Some("计算夏普比率".into()),
        parameters: None,
    };
    let td_var = ToolDef {
        name: "calc_var".into(),
        description: Some("历史模拟法 VaR 计算".into()),
        parameters: None,
    };
    let td_pe_pct = ToolDef {
        name: "calc_pe_percentile".into(),
        description: Some("PE 历史分位数".into()),
        parameters: None,
    };
    let td_peg = ToolDef {
        name: "calc_peg".into(),
        description: Some("PEG 估值指标".into()),
        parameters: None,
    };
    let td_ma_cross = ToolDef {
        name: "detect_ma_cross".into(),
        description: Some("MA 金叉死叉检测".into()),
        parameters: None,
    };
    let td_breakout = ToolDef {
        name: "detect_breakout".into(),
        description: Some("支撑阻力突破检测".into()),
        parameters: None,
    };
    let td_kelly = ToolDef {
        name: "calc_kelly".into(),
        description: Some("凯利公式仓位计算".into()),
        parameters: None,
    };
    let td_rp = ToolDef {
        name: "calc_risk_parity".into(),
        description: Some("风险平价权重计算".into()),
        parameters: None,
    };
    let td_outliers = ToolDef {
        name: "clean_outliers".into(),
        description: Some("异常值剔除 (zscore/iqr)".into()),
        parameters: None,
    };
    let td_fill = ToolDef {
        name: "clean_fill_missing".into(),
        description: Some("缺失值填充 (forward/linear)".into()),
        parameters: None,
    };
    let td_adjust = ToolDef {
        name: "adjust_prices".into(),
        description: Some("前复权价格调整".into()),
        parameters: None,
    };

    let agent = |id: &str, title: &str, expert_id: &str| -> WorkflowNode {
        WorkflowNode::Agent(AgentNode {
            base: WorkflowNodeBase {
                id: id.into(),
                title: title.into(),
                description: Some(format!("股票分析: {expert_id}")),
                position: Position { x: 0.0, y: 0.0 },
                retry: RetryConfig {
                    enabled: true,
                    max_retries: 2,
                    ..Default::default()
                },
                timeout: Some(300),
                enabled: true,
            },
            config: AgentNodeConfig {
                role: None,
                // inline system_prompt 只放任务指令，专家 prompt 由 agent_profile 自动加载，
                // 行情数据通过 context_sources 由上游 Tool 节点输出自动注入
                system_prompt: format!("你的任务: {title}"),
                context_sources: vec![],
                output_var: id.into(),
                model: None,
                temperature: Some(0.3),
                max_tokens: Some(4096),
                tools: vec![],
                output_mode: OutputMode::Text,
                agent_profile_id: Some(format!("stock-{expert_id}")),
                agent_role_override: None,
                max_tool_rounds: None,
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
            position: Position { x: 250.0, y: 0.0 },
            retry: RetryConfig::default(),
            timeout: None,
            enabled: true,
        },
        config: TriggerConfig {
            trigger_type: TriggerType::Manual,
            config: serde_json::json!({"stock_code": "{{stock_code}}"}),
        },
    }));

    // 9 个分析师
    let analysts = [
        (
            "a-market-analyst",
            "基于行情数据对该股票进行技术面分析，覆盖K线形态、均线、MACD/RSI指标、支撑阻力位，输出结构化分析报告",
            "market-analyst",
        ),
        (
            "a-sentiment",
            "分析该股票的市场情绪，包括资金流向、散户/机构态度、社交媒体热度，给出情绪面评分",
            "sentiment-analyst",
        ),
        (
            "a-news",
            "梳理该股票近期重大新闻和公告，评估每条消息对股价的影响方向和力度",
            "news-analyst",
        ),
        (
            "a-fundamentals",
            "基于PE/PB/ROE/营收增长率等财务指标对该股票进行基本面估值分析",
            "fundamentals-analyst",
        ),
        (
            "a-policy",
            "分析当前宏观政策和行业政策对该股票的潜在影响，包括货币政策、产业政策、监管动态",
            "policy-analyst",
        ),
        (
            "a-hot-money",
            "追踪该股票的游资动向、龙虎榜数据、主力资金进出情况",
            "hot-money-tracker",
        ),
        (
            "a-lockup",
            "排查该股票近期解禁计划、大股东减持公告、股权质押风险",
            "lockup-watcher",
        ),
        (
            "a-research",
            "汇总该股票的最新券商研报观点，提取目标价、评级变化和核心逻辑",
            "research-analyst",
        ),
        (
            "a-sector",
            "分析该股票所属行业的景气度、板块轮动趋势、同业竞争格局",
            "sector-analyst",
        ),
    ];
    let a_ids: Vec<&str> = analysts.iter().map(|(id, _, _)| *id).collect();

    // 为每个分析师插入对应的数据获取 Tool 节点
    let tool_assignments: &[(&str, &str, &str, &str)] = &[
        ("t-market-data", "获取K线", "get_stock_kline", "stock_code"),
        ("t-sentiment-data", "获取新闻", "get_stock_news", "stock_code"),
        ("t-news-data", "获取新闻", "get_stock_news", "stock_code"),
        ("t-fundamentals-data", "获取财务", "get_stock_financials", "stock_code"),
        ("t-policy-data", "获取新闻", "get_stock_news", "stock_code"),
        ("t-hotmoney-data", "获取资金流向", "get_stock_money_flow", "stock_code"),
        ("t-lockup-data", "获取财务", "get_stock_financials", "stock_code"),
        ("t-research-data", "获取新闻", "get_stock_news", "stock_code"),
        ("t-sector-data", "获取行情", "get_stock_quote", "stock_code"),
    ];

    for (i, (tool_id, tool_title, tool_name, arg_key)) in tool_assignments.iter().enumerate() {
        let analyst_id = a_ids[i];
        nodes.push(tool_node(tool_id, tool_title, tool_name, tool_id, arg_key));
        edges.push(edge(&format!("e-trigger-{tool_id}"), "trigger", tool_id));
        edges.push(edge(&format!("e-{tool_id}-{analyst_id}"), tool_id, analyst_id));
    }

    for (i, (id, title, expert)) in analysts.iter().enumerate() {
        let tool_id = tool_assignments[i].0;
        let mut an = agent(id, title, expert);
        if let WorkflowNode::Agent(ref mut a) = an {
            a.config.context_sources = vec![tool_id.to_string()];
        }
        nodes.push(an);
    }

    // 辩论 6 轮
    let debate_pairs = [
        (
            "bull-r1",
            "基于9位分析师的报告，构建该股票的完整买入论点。引用分析师数据支撑，明确列出3-5个看涨理由，每个理由需有具体数据",
            "bull-researcher",
            &a_ids[..],
            "bear-r1",
        ),
        (
            "bear-r1",
            "针对多方论点逐条反驳。结合分析师报告中的风险信号，列出该股票3-5个看跌理由，指出多方论点中的数据漏洞",
            "bear-researcher",
            &["bull-r1"],
            "bull-r2",
        ),
        (
            "bull-r2",
            "对空方论点进行二次反击。补充新的看涨证据，修正被空方成功攻击的弱论点，强化剩余论点的数据支撑",
            "bull-researcher",
            &["bear-r1"],
            "bear-r2",
        ),
        (
            "bear-r2",
            "深入挖掘该股票的隐藏风险。不仅反驳多方新论点，还要提出多方未覆盖的风险维度（如流动性风险、汇率风险等）",
            "bear-researcher",
            &["bull-r2"],
            "bull-r3",
        ),
        (
            "bull-r3",
            "终极多方辩护。综合前两轮交锋，提炼最坚固的3个看涨核心逻辑，对每个逻辑给出置信度评分（0-100）",
            "bull-researcher",
            &["bear-r2"],
            "bear-r3",
        ),
        (
            "bear-r3",
            "终极空方陈词。给出该股票的风险综合评分（0-100），明确指出如果持仓应在什么条件下止损",
            "bear-researcher",
            &["bull-r3"],
            "",
        ),
    ];
    for (id, title, expert, deps, _next) in &debate_pairs {
        nodes.push(agent(id, title, expert));
        for dep in *deps {
            edges.push(edge(&format!("e-{dep}-{id}"), dep, id));
        }
    }

    // 风险评估（3 个并行，均依赖 bear-r3）
    for (rid, rtitle, rexpert) in &[
        (
            "risk-agg",
            "以最激进的风险偏好评估该股票：假设最大回撤容忍度30%，给出该股票是否值得重仓的结论",
            "aggressive-debator",
        ),
        (
            "risk-con",
            "以最保守的风险偏好评估该股票：本金安全第一，给出该股票是否适合配置的结论",
            "conservative-debator",
        ),
        (
            "risk-neu",
            "以中性风险偏好评估该股票：平衡收益与风险，给出该股票的合理仓位建议",
            "neutral-debator",
        ),
    ] {
        nodes.push(agent(rid, rtitle, rexpert));
        edges.push(edge(&format!("e-bear-r3-{rid}"), "bear-r3", rid));
    }

    // ── 算法 Tool 节点：内置算法作为工具嵌入工作流 ──
    let algo_tools: &[(&str, &str, &str, &str)] = &[
        ("t-scoring", "技术评分", "compute_scoring", "stock_code"),
        ("t-valuation", "估值计算", "compute_valuation", "stock_code"),
        ("t-risk", "风险评估", "compute_portfolio_risk", "positions_json"),
        ("t-quality", "质量门控", "run_quality_gate", "reports_json"),
        // 新增 12 个金融模型工具
        ("t-calc-maxdd", "最大回撤", "calc_max_drawdown", "prices_json"),
        ("t-calc-sharpe", "夏普比率", "calc_sharpe_ratio", "returns_json"),
        ("t-calc-var", "VaR 风险价值", "calc_var", "returns_json"),
        ("t-calc-pe-pct", "PE 分位数", "calc_pe_percentile", "current_pe"),
        ("t-calc-peg", "PEG 估值", "calc_peg", "pe"),
        ("t-signal-cross", "MA 交叉检测", "detect_ma_cross", "klines_json"),
        ("t-signal-brk", "突破检测", "detect_breakout", "klines_json"),
        ("t-calc-kelly", "凯利仓位", "calc_kelly", "win_rate"),
        ("t-calc-rp", "风险平价", "calc_risk_parity", "volatilities_json"),
        ("t-clean-outl", "异常值剔除", "clean_outliers", "prices_json"),
        ("t-clean-fill", "缺失值填充", "clean_fill_missing", "prices_json"),
        ("t-adjust-px", "复权调整", "adjust_prices", "klines_json"),
    ];
    for (tool_id, title, tool_name, arg_key) in algo_tools {
        nodes.push(tool_node(tool_id, title, tool_name, tool_id, arg_key));
    }
    // 风险节点 → 第一个算法工具（评分）
    for rid in &["risk-agg", "risk-con", "risk-neu"] {
        edges.push(edge(&format!("e-{rid}-t-scoring"), rid, "t-scoring"));
    }
    // 链式连接所有算法工具：scoring → valuation → risk → quality → 12 new tools
    edges.push(edge("e-t-scoring-t-valuation", "t-scoring", "t-valuation"));
    edges.push(edge("e-t-valuation-t-risk", "t-valuation", "t-risk"));
    edges.push(edge("e-t-risk-t-quality", "t-risk", "t-quality"));
    let algo_chain = [
        "t-calc-maxdd",
        "t-calc-sharpe",
        "t-calc-var",
        "t-calc-pe-pct",
        "t-calc-peg",
        "t-signal-cross",
        "t-signal-brk",
        "t-calc-kelly",
        "t-calc-rp",
        "t-clean-outl",
        "t-clean-fill",
        "t-adjust-px",
    ];
    edges.push(edge("e-t-quality-t-calc-maxdd", "t-quality", "t-calc-maxdd"));
    for w in algo_chain.windows(2) {
        edges.push(edge(&format!("e-{}-{}", w[0], w[1]), w[0], w[1]));
    }

    // research-mgr → trader → portfolio-mgr
    let mut rm = agent(
        "research-mgr",
        "综合三种风险偏好的评估结果，给出该股票的总体风险评级（低/中/高）及主要风险点清单",
        "research-manager",
    );
    if let WorkflowNode::Agent(ref mut a) = rm {
        a.config.context_sources = vec![
            "t-scoring".into(),
            "t-valuation".into(),
            "t-risk".into(),
            "t-quality".into(),
            "t-calc-maxdd".into(),
            "t-calc-sharpe".into(),
            "t-calc-var".into(),
            "t-calc-pe-pct".into(),
            "t-calc-peg".into(),
            "t-signal-cross".into(),
            "t-signal-brk".into(),
            "t-calc-kelly".into(),
            "t-calc-rp".into(),
            "t-clean-outl".into(),
            "t-clean-fill".into(),
            "t-adjust-px".into(),
        ];
        a.config.tools = vec![
            td_score.clone(),
            td_val.clone(),
            td_risk.clone(),
            td_quality.clone(),
            td_fin.clone(),
            td_maxdd.clone(),
            td_sharpe.clone(),
            td_var.clone(),
            td_pe_pct.clone(),
            td_peg.clone(),
            td_ma_cross.clone(),
            td_breakout.clone(),
            td_kelly.clone(),
            td_rp.clone(),
            td_outliers.clone(),
            td_fill.clone(),
            td_adjust.clone(),
        ];
        a.config.max_tool_rounds = Some(3);
    }
    nodes.push(rm);
    edges.push(edge("e-t-adjust-px-research-mgr", "t-adjust-px", "research-mgr"));

    let mut trader = agent(
        "trader",
        "基于风险总评和辩论结论，制定该股票的具体A股交易方案：入场价、目标价、止损价、仓位比例、分批建仓计划。必须遵守T+1和涨跌停规则",
        "trader",
    );
    if let WorkflowNode::Agent(ref mut a) = trader {
        a.config.context_sources = vec!["research-mgr".into()];
        a.config.tools = vec![
            td_quote.clone(),
            td_kline.clone(),
            td_mf.clone(),
            td_score.clone(),
        ];
        a.config.max_tool_rounds = Some(3);
    }
    nodes.push(trader);
    edges.push(edge("e-research-mgr-trader", "research-mgr", "trader"));

    let mut pm = agent(
        "portfolio-mgr",
        "作为最终决策者，综合所有分析结果，给出该股票的最终投资决策。输出JSON格式：{ action: 买入/增持/持有/减持/卖出, positionPct: 仓位百分比, targetPrice: 目标价, stopLoss: 止损价, reasoning: 决策理由(300字以内), riskLevel: 风险等级(低/中/高), confidence: 置信度(0-100) }",
        "portfolio-manager",
    );
    if let WorkflowNode::Agent(ref mut a) = pm {
        a.config.context_sources = vec!["trader".into(), "research-mgr".into()];
        a.config.tools = vec![
            td_quote.clone(),
            td_kline.clone(),
            td_fin.clone(),
            td_score.clone(),
            td_val.clone(),
            td_risk.clone(),
            td_maxdd.clone(),
            td_sharpe.clone(),
            td_var.clone(),
            td_pe_pct.clone(),
            td_peg.clone(),
            td_ma_cross.clone(),
            td_breakout.clone(),
            td_kelly.clone(),
            td_rp.clone(),
        ];
        a.config.max_tool_rounds = Some(3);
    }
    nodes.push(pm);
    edges.push(edge("e-trader-portfolio-mgr", "trader", "portfolio-mgr"));
    edges.push(edge("e-research-mgr-portfolio-mgr", "research-mgr", "portfolio-mgr"));

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
        required: Some(vec![
            "action".to_string(),
            "positionPct".to_string(),
            "reasoning".to_string(),
            "riskLevel".to_string(),
            "confidence".to_string(),
        ]),
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
            value: serde_json::json!(6),
            description: Some("多空辩论轮数 (1-10)".into()),
            is_secret: false,
        },
        Variable {
            name: "max_concurrent".into(),
            var_type: "number".into(),
            value: serde_json::json!(9),
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
            value: serde_json::json!(false),
            description: Some("同花顺 — 综合数据".into()),
            is_secret: false,
        },
        Variable {
            name: "vendor_cninfo".into(),
            var_type: "boolean".into(),
            value: serde_json::json!(false),
            description: Some("巨潮资讯 — 信息披露".into()),
            is_secret: false,
        },
        Variable {
            name: "vendor_baidu_stock".into(),
            var_type: "boolean".into(),
            value: serde_json::json!(false),
            description: Some("百度股票 — 数据".into()),
            is_secret: false,
        },
        Variable {
            name: "vendor_iwencai".into(),
            var_type: "boolean".into(),
            value: serde_json::json!(false),
            description: Some("问财 — 选股数据".into()),
            is_secret: false,
        },
        Variable {
            name: "vendor_akshare".into(),
            var_type: "boolean".into(),
            value: serde_json::json!(false),
            description: Some("AKShare — 开源数据".into()),
            is_secret: false,
        },
        Variable {
            name: "vendor_mootdx".into(),
            var_type: "boolean".into(),
            value: serde_json::json!(false),
            description: Some("Mootdx — 本地行情接口".into()),
            is_secret: false,
        },
        // ── 新增：金融模型参数 ──
        Variable {
            name: "risk_free_rate".into(),
            var_type: "number".into(),
            value: serde_json::json!(3.0),
            description: Some("无风险利率 (%)，用于夏普比率和 DCF".into()),
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
            description: Some("凯利比例系数 (0-1) — 建议仓位 = half_kelly × 此系数".into()),
            is_secret: false,
        },
    ];
    let variables_val =
        serde_json::to_string(&variables).map_err(|e| format!("序列化变量失败: {e}"))?;

    // 写入 DB
    let nodes_json = serde_json::to_string(&nodes).map_err(|e| format!("序列化节点失败: {e}"))?;
    let edges_json = serde_json::to_string(&edges).map_err(|e| format!("序列化边失败: {e}"))?;
    let tags = serde_json::to_string(&["stock", "analysis", "A股"])
        .map_err(|e| format!("序列化标签失败: {e}"))?;

    workflow_template::ActiveModel {
        id: Set(TEMPLATE_ID.to_string()),
        name: Set("A股多维度分析".to_string()),
        description: Set(Some(
            "9 维度分析师 → 6 轮多空辩论 → 3 风险维度 → 交易方案 → 投资决策".to_string(),
        )),
        icon: Set("chart-bar".to_string()),
        tags: Set(Some(tags)),
        version: Set(TEMPLATE_VERSION),
        is_preset: Set(true),
        is_editable: Set(true),
        is_public: Set(true),
        trigger_config: Set(None),
        nodes: Set(nodes_json),
        edges: Set(edges_json),
        input_schema: Set(Some(input_schema_val)),
        output_schema: Set(Some(output_schema_val)),
        variables: Set(Some(variables_val)),
        error_config: Set(None),
        composite_source: Set(None),
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
    use sea_orm::sea_query::Expr;
    use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, Set};

    // 构建 expert_id → 提示词正文 的查找表
    let expert_prompts: std::collections::HashMap<&str, &str> =
        EMBEDDED_PROMPTS.iter().copied().collect();

    let mut count = 0u32;
    for &(expert_id, role_id) in EXPERT_ROLE_MAP {
        let profile_id = format!("stock-{expert_id}");
        let expert_body = expert_prompts
            .get(expert_id)
            .map(|content| {
                let (_, _, body, _) = parse_expert_md(content, expert_id);
                body
            })
            .unwrap_or_default();

        if let Some(existing) = agent_profiles::Entity::find_by_id(&profile_id)
            .one(db)
            .await
            .map_err(|e| e.to_string())?
        {
            // 已有 profile 但 system_prompt 为空 → 补填
            if existing.system_prompt.is_empty() {
                agent_profiles::Entity::update_many()
                    .col_expr(agent_profiles::Column::SystemPrompt, Expr::value(expert_body))
                    .filter(agent_profiles::Column::Id.eq(&profile_id))
                    .exec(db)
                    .await
                    .map_err(|e| e.to_string())?;
                count += 1;
            }
            continue;
        }

        let now = chrono::Utc::now().timestamp_millis();
        let model = agent_profiles::ActiveModel {
            id: Set(profile_id.clone()),
            name: Set(format!("📈 {}", expert_id_to_display(expert_id))),
            description: Set(Some(format!("股票分析专家 — {}", role_id_to_display(role_id)))),
            category: Set("stock-analysis".into()),
            icon: Set("📈".into()),
            system_prompt: Set(expert_body),
            agent_role: Set(Some(role_id.into())),
            source: Set("stock-analysis".into()),
            tags: Set(None),
            suggested_provider_id: Set(None),
            suggested_model_id: Set(None),
            suggested_temperature: Set(None),
            suggested_max_tokens: Set(None),
            search_enabled: Set(None),
            recommend_permission_mode: Set(None),
            recommended_tools: Set(None),
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
                if let Some(v) = line.trim().strip_prefix("name:") {
                    name = v.trim().into();
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
        "portfolio-manager" => "投资组合经理".to_string(),
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
        o => o.to_string(),
    }
}
