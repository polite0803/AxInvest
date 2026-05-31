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
            "search_stock",
        ],
    ),
    (
        "news-analyst",
        &[
            "get_stock_news",
            "get_announcements",
            "get_cls_flash",
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
    ("lockup-watcher", &["get_stock_financials", "get_announcements", "search_stock"]),
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
            "search_stock",
        ],
    ),
    ("bull-researcher", &["compute_scoring", "compute_valuation", "search_stock"]),
    ("bear-researcher", &["compute_scoring", "compute_valuation", "search_stock"]),
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
        "portfolio-manager",
        &[
            "compute_scoring",
            "compute_valuation",
            "compute_portfolio_risk",
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
    seed_debate_subworkflow(db).await?;
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
        AgentNode, AgentNodeConfig, Branch, ConditionNode, ConditionNodeConfig, EdgeType,
        ErrorConfig, JsonSchema, JsonSchemaProperty, LogicalOperator, OnFailureAction, OutputMode,
        ParallelNode, ParallelNodeConfig, Position, RetryConfig, RetryPolicy, ToolDef, ToolNode,
        ToolNodeConfig, TriggerConfig, TriggerNode, TriggerType, ValidationAssertion,
        ValidationNode, ValidationNodeConfig, Variable, WorkflowEdge, WorkflowNode,
        WorkflowNodeBase,
    };
    use sea_orm::{ActiveModelTrait, EntityTrait, Set};

    const TEMPLATE_ID: &str = "stock-analysis";
    const TEMPLATE_VERSION: i32 = 1;

    if let Some(existing) = workflow_template::Entity::find_by_id(TEMPLATE_ID)
        .one(db)
        .await
        .map_err(|e| format!("查询工作流模板失败: {e}"))?
    {
        // 始终删除重建，确保模板与代码一致
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
                        enabled: false,
                        max_retries: 0,
                        ..Default::default()
                    },
                    timeout: Some(120),
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
        parameters: Some(JsonSchema {
            schema_type: "object".into(),
            description: None,
            properties: Some(std::collections::HashMap::new()),
            required: None,
            items: None,
        }),
    };
    let td_kline = ToolDef {
        name: "get_stock_kline".into(),
        description: Some("获取K线数据：OHLCV，可指定周期和数量".into()),
        parameters: Some(JsonSchema {
            schema_type: "object".into(),
            description: None,
            properties: Some(std::collections::HashMap::new()),
            required: None,
            items: None,
        }),
    };
    let td_fin = ToolDef {
        name: "get_stock_financials".into(),
        description: Some("获取财务数据：营收、净利润、EPS、ROE、毛利率等".into()),
        parameters: Some(JsonSchema {
            schema_type: "object".into(),
            description: None,
            properties: Some(std::collections::HashMap::new()),
            required: None,
            items: None,
        }),
    };
    let td_news = ToolDef {
        name: "get_stock_news".into(),
        description: Some("获取近期新闻公告".into()),
        parameters: Some(JsonSchema {
            schema_type: "object".into(),
            description: None,
            properties: Some(std::collections::HashMap::new()),
            required: None,
            items: None,
        }),
    };
    let td_mf = ToolDef {
        name: "get_stock_money_flow".into(),
        description: Some("获取资金流向：主力/超大单/大单/中单/小单净流入".into()),
        parameters: Some(JsonSchema {
            schema_type: "object".into(),
            description: None,
            properties: Some(std::collections::HashMap::new()),
            required: None,
            items: None,
        }),
    };
    let td_score = ToolDef {
        name: "compute_scoring".into(),
        description: Some("计算技术评分：基于趋势、偏离度、MACD、成交量、RSI、支撑阻力".into()),
        parameters: Some(JsonSchema {
            schema_type: "object".into(),
            description: None,
            properties: Some(std::collections::HashMap::new()),
            required: None,
            items: None,
        }),
    };
    let td_val = ToolDef {
        name: "compute_valuation".into(),
        description: Some("计算估值指标：DCF、F-Score、护城河量化、安全边际".into()),
        parameters: Some(JsonSchema {
            schema_type: "object".into(),
            description: None,
            properties: Some(std::collections::HashMap::new()),
            required: None,
            items: None,
        }),
    };
    let td_risk = ToolDef {
        name: "compute_portfolio_risk".into(),
        description: Some("计算组合风险：总市值、集中度、风险等级".into()),
        parameters: Some(JsonSchema {
            schema_type: "object".into(),
            description: None,
            properties: Some(std::collections::HashMap::new()),
            required: None,
            items: None,
        }),
    };
    // ── 新增 12 个金融模型 ToolDef ──
    let td_maxdd = ToolDef {
        name: "calc_max_drawdown".into(),
        description: Some("计算最大回撤比例".into()),
        parameters: Some(JsonSchema {
            schema_type: "object".into(),
            description: None,
            properties: Some(std::collections::HashMap::new()),
            required: None,
            items: None,
        }),
    };
    let td_sharpe = ToolDef {
        name: "calc_sharpe_ratio".into(),
        description: Some("计算夏普比率".into()),
        parameters: Some(JsonSchema {
            schema_type: "object".into(),
            description: None,
            properties: Some(std::collections::HashMap::new()),
            required: None,
            items: None,
        }),
    };
    let td_var = ToolDef {
        name: "calc_var".into(),
        description: Some("历史模拟法 VaR 计算".into()),
        parameters: Some(JsonSchema {
            schema_type: "object".into(),
            description: None,
            properties: Some(std::collections::HashMap::new()),
            required: None,
            items: None,
        }),
    };
    let td_pe_pct = ToolDef {
        name: "calc_pe_percentile".into(),
        description: Some("PE 历史分位数".into()),
        parameters: Some(JsonSchema {
            schema_type: "object".into(),
            description: None,
            properties: Some(std::collections::HashMap::new()),
            required: None,
            items: None,
        }),
    };
    let td_peg = ToolDef {
        name: "calc_peg".into(),
        description: Some("PEG 估值指标".into()),
        parameters: Some(JsonSchema {
            schema_type: "object".into(),
            description: None,
            properties: Some(std::collections::HashMap::new()),
            required: None,
            items: None,
        }),
    };
    let td_ma_cross = ToolDef {
        name: "detect_ma_cross".into(),
        description: Some("MA 金叉死叉检测".into()),
        parameters: Some(JsonSchema {
            schema_type: "object".into(),
            description: None,
            properties: Some(std::collections::HashMap::new()),
            required: None,
            items: None,
        }),
    };
    let td_breakout = ToolDef {
        name: "detect_breakout".into(),
        description: Some("支撑阻力突破检测".into()),
        parameters: Some(JsonSchema {
            schema_type: "object".into(),
            description: None,
            properties: Some(std::collections::HashMap::new()),
            required: None,
            items: None,
        }),
    };
    let td_kelly = ToolDef {
        name: "calc_kelly".into(),
        description: Some("凯利公式仓位计算".into()),
        parameters: Some(JsonSchema {
            schema_type: "object".into(),
            description: None,
            properties: Some(std::collections::HashMap::new()),
            required: None,
            items: None,
        }),
    };
    let td_rp = ToolDef {
        name: "calc_risk_parity".into(),
        description: Some("风险平价权重计算".into()),
        parameters: Some(JsonSchema {
            schema_type: "object".into(),
            description: None,
            properties: Some(std::collections::HashMap::new()),
            required: None,
            items: None,
        }),
    };
    let td_outliers = ToolDef {
        name: "clean_outliers".into(),
        description: Some("异常值剔除 (zscore/iqr)".into()),
        parameters: Some(JsonSchema {
            schema_type: "object".into(),
            description: None,
            properties: Some(std::collections::HashMap::new()),
            required: None,
            items: None,
        }),
    };
    let td_fill = ToolDef {
        name: "clean_fill_missing".into(),
        description: Some("缺失值填充 (forward/linear)".into()),
        parameters: Some(JsonSchema {
            schema_type: "object".into(),
            description: None,
            properties: Some(std::collections::HashMap::new()),
            required: None,
            items: None,
        }),
    };
    let td_adjust = ToolDef {
        name: "adjust_prices".into(),
        description: Some("前复权价格调整".into()),
        parameters: Some(JsonSchema {
            schema_type: "object".into(),
            description: None,
            properties: Some(std::collections::HashMap::new()),
            required: None,
            items: None,
        }),
    };
    // ── 新增 9 个数据 API ToolDef ──
    let td_research = ToolDef {
        name: "get_research_reports".into(),
        description: Some("获取券商研报".into()),
        parameters: Some(JsonSchema {
            schema_type: "object".into(),
            description: None,
            properties: Some(std::collections::HashMap::new()),
            required: None,
            items: None,
        }),
    };
    let td_consensus = ToolDef {
        name: "get_consensus_eps".into(),
        description: Some("获取一致性预期EPS".into()),
        parameters: Some(JsonSchema {
            schema_type: "object".into(),
            description: None,
            properties: Some(std::collections::HashMap::new()),
            required: None,
            items: None,
        }),
    };
    let td_concepts = ToolDef {
        name: "get_concept_blocks".into(),
        description: Some("获取概念板块归属".into()),
        parameters: Some(JsonSchema {
            schema_type: "object".into(),
            description: None,
            properties: Some(std::collections::HashMap::new()),
            required: None,
            items: None,
        }),
    };
    let td_announce = ToolDef {
        name: "get_announcements".into(),
        description: Some("获取公司公告".into()),
        parameters: Some(JsonSchema {
            schema_type: "object".into(),
            description: None,
            properties: Some(std::collections::HashMap::new()),
            required: None,
            items: None,
        }),
    };
    let td_north = ToolDef {
        name: "get_north_bound_flow".into(),
        description: Some("获取北向资金流向".into()),
        parameters: Some(JsonSchema {
            schema_type: "object".into(),
            description: None,
            properties: Some(std::collections::HashMap::new()),
            required: None,
            items: None,
        }),
    };
    let td_dragon = ToolDef {
        name: "get_market_dragon_tiger".into(),
        description: Some("获取龙虎榜数据".into()),
        parameters: Some(JsonSchema {
            schema_type: "object".into(),
            description: None,
            properties: Some(std::collections::HashMap::new()),
            required: None,
            items: None,
        }),
    };
    let td_hot = ToolDef {
        name: "get_hot_stocks".into(),
        description: Some("获取市场热门股".into()),
        parameters: Some(JsonSchema {
            schema_type: "object".into(),
            description: None,
            properties: Some(std::collections::HashMap::new()),
            required: None,
            items: None,
        }),
    };
    let td_industry = ToolDef {
        name: "get_industry_ranking".into(),
        description: Some("获取行业涨跌排名".into()),
        parameters: Some(JsonSchema {
            schema_type: "object".into(),
            description: None,
            properties: Some(std::collections::HashMap::new()),
            required: None,
            items: None,
        }),
    };
    let td_cls = ToolDef {
        name: "get_cls_flash".into(),
        description: Some("获取财联社实时快讯".into()),
        parameters: Some(JsonSchema {
            schema_type: "object".into(),
            description: None,
            properties: Some(std::collections::HashMap::new()),
            required: None,
            items: None,
        }),
    };
    // ── P1: 4 个技术指标 ToolDef ──
    let td_atr = ToolDef {
        name: "compute_atr".into(),
        description: Some("计算 ATR 平均真实波幅".into()),
        parameters: Some(JsonSchema {
            schema_type: "object".into(),
            description: None,
            properties: Some(std::collections::HashMap::new()),
            required: None,
            items: None,
        }),
    };
    let td_kdj = ToolDef {
        name: "compute_kdj".into(),
        description: Some("计算 KDJ 随机指标".into()),
        parameters: Some(JsonSchema {
            schema_type: "object".into(),
            description: None,
            properties: Some(std::collections::HashMap::new()),
            required: None,
            items: None,
        }),
    };
    let td_obv = ToolDef {
        name: "compute_obv".into(),
        description: Some("计算 OBV 能量潮".into()),
        parameters: Some(JsonSchema {
            schema_type: "object".into(),
            description: None,
            properties: Some(std::collections::HashMap::new()),
            required: None,
            items: None,
        }),
    };
    let td_beta = ToolDef {
        name: "calc_beta".into(),
        description: Some("计算 Beta 系数".into()),
        parameters: Some(JsonSchema {
            schema_type: "object".into(),
            description: None,
            properties: Some(std::collections::HashMap::new()),
            required: None,
            items: None,
        }),
    };
    // ── P2: 事件检测 + 组合分析 ToolDef ──
    let td_earnings = ToolDef {
        name: "detect_earnings_surprise".into(),
        description: Some("检测业绩超预期/低于预期".into()),
        parameters: Some(JsonSchema {
            schema_type: "object".into(),
            description: None,
            properties: Some(std::collections::HashMap::new()),
            required: None,
            items: None,
        }),
    };
    let td_pledge = ToolDef {
        name: "detect_pledge_risk".into(),
        description: Some("检测大股东质押风险".into()),
        parameters: Some(JsonSchema {
            schema_type: "object".into(),
            description: None,
            properties: Some(std::collections::HashMap::new()),
            required: None,
            items: None,
        }),
    };
    let td_corr = ToolDef {
        name: "calc_correlation_matrix".into(),
        description: Some("计算收益率相关系数矩阵".into()),
        parameters: Some(JsonSchema {
            schema_type: "object".into(),
            description: None,
            properties: Some(std::collections::HashMap::new()),
            required: None,
            items: None,
        }),
    };
    // ── P3: 独立新能力 ToolDef ──
    let td_mc = ToolDef {
        name: "run_monte_carlo".into(),
        description: Some("蒙特卡洛模拟价格路径".into()),
        parameters: Some(JsonSchema {
            schema_type: "object".into(),
            description: None,
            properties: Some(std::collections::HashMap::new()),
            required: None,
            items: None,
        }),
    };
    let td_ind = ToolDef {
        name: "analyze_industry_position".into(),
        description: Some("行业内估值/增长对比分析".into()),
        parameters: Some(JsonSchema {
            schema_type: "object".into(),
            description: None,
            properties: Some(std::collections::HashMap::new()),
            required: None,
            items: None,
        }),
    };
    let td_lup = ToolDef {
        name: "detect_limit_up_potential".into(),
        description: Some("涨停潜力评估".into()),
        parameters: Some(JsonSchema {
            schema_type: "object".into(),
            description: None,
            properties: Some(std::collections::HashMap::new()),
            required: None,
            items: None,
        }),
    };
    let td_block = ToolDef {
        name: "get_block_trades".into(),
        description: Some("获取大宗交易记录：成交价、成交量、买卖方营业部、折价率".into()),
        parameters: Some(JsonSchema {
            schema_type: "object".into(),
            description: None,
            properties: Some(std::collections::HashMap::new()),
            required: None,
            items: None,
        }),
    };
    let td_visit = ToolDef {
        name: "get_institutional_visits".into(),
        description: Some("获取机构调研记录：调研日期、机构数量、调研内容".into()),
        parameters: Some(JsonSchema {
            schema_type: "object".into(),
            description: None,
            properties: Some(std::collections::HashMap::new()),
            required: None,
            items: None,
        }),
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
                // inline system_prompt 只放任务指令，专家 prompt 由 agent_profile 自动加载，
                // 行情数据通过 context_sources 由上游 Tool 节点输出自动注入
                system_prompt: format!("你的任务: {title}"),
                context_sources: vec![],
                output_var: id.into(),
                model: None,
                temperature: Some(0.3),
                max_tokens: Some(4096),
                tools: vec![],
                exposed_tools: vec![],
                output_mode: OutputMode::Text,
                agent_profile_id: Some(format!("stock-{expert_id}")),
                max_tool_rounds: None,
                execution_mode: None,
                rag_source_ids: vec![],
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
        ("t-market-data", "获取K线+行情", "get_stock_kline", "stock_code"),
        ("t-sentiment-data", "获取新闻+热门", "get_hot_stocks", "stock_code"),
        ("t-news-data", "获取新闻+公告", "get_announcements", "stock_code"),
        ("t-fundamentals-data", "获取财务+一致预期", "get_consensus_eps", "stock_code"),
        ("t-policy-data", "获取新闻+公告", "get_announcements", "stock_code"),
        ("t-hotmoney-data", "获取资金流向", "get_stock_money_flow", "stock_code"),
        ("t-lockup-data", "获取财务+公告", "get_announcements", "stock_code"),
        ("t-research-data", "获取新闻+一致预期", "get_consensus_eps", "stock_code"),
        ("t-sector-data", "获取行情+行业排名", "get_industry_ranking", "stock_code"),
    ];

    // ── Phase 1: ParallelNode 包裹 9 组 Tool+Agent 分析师 ──
    let analyst_branches: Vec<Branch> = tool_assignments
        .iter()
        .enumerate()
        .map(|(i, (tool_id, _tool_title, tool_name, arg_key))| {
            let analyst_id = a_ids[i];
            // 每个分支内部：ToolNode → AgentNode
            nodes.push(tool_node(tool_id, "获取数据", tool_name, tool_id, arg_key));
            edges.push(edge(&format!("e-p-analysts-{tool_id}"), "p-analysts", tool_id));
            edges.push(edge(&format!("e-{tool_id}-{analyst_id}"), tool_id, analyst_id));
            Branch {
                id: analyst_id.to_string(),
                title: analysts[i].1.to_string(),
                steps: vec![tool_id.to_string(), analyst_id.to_string()],
            }
        })
        .collect();

    // 工具由模板节点 config.tools 统一管理
    for (i, (id, title, _expert)) in analysts.iter().enumerate() {
        let tool_id = tool_assignments[i].0;
        let fixed_tool_name = tool_assignments[i].2;
        let mut an = agent(id, title, _expert);
        if let WorkflowNode::Agent(ref mut a) = an {
            a.config.context_sources = vec![tool_id.to_string()];
            a.config.max_tool_rounds = Some(2);
            let tool_names = PROFILE_TOOLS
                .iter()
                .find(|(k, _)| **k == **_expert)
                .map(|(_, v)| *v)
                .unwrap_or(&[]);
            a.config.tools = tool_names
                .iter()
                .filter_map(|&tn| tool_def_map.get(tn).cloned())
                .collect();
            a.config.exposed_tools = tool_names
                .iter()
                .filter(|&&tn| tn != fixed_tool_name)
                .map(|&tn| tn.to_string())
                .collect();
            a.config.system_prompt =
                format!("{}{}", a.config.system_prompt, tool_prompt(&a.config.tools));
        }
        nodes.push(an);
    }

    // 分析师节点 → c-need-debate 的出边（编辑器可视化 + 运行时依赖）
    for aid in &a_ids {
        edges.push(edge(&format!("e-{aid}-c-debate"), aid, "c-need-debate"));
    }

    // ParallelNode: 将 9 组分析师封装为统一并行节点
    nodes.push(WorkflowNode::Parallel(ParallelNode {
        base: WorkflowNodeBase {
            id: "p-analysts".into(),
            title: "9 维度分析师并行".into(),
            description: Some("行情/情绪/新闻/基本面/政策/游资/解禁/研报/行业".into()),
            position: Position { x: 300.0, y: 100.0 },
            retry: RetryConfig::default(),
            timeout: Some(600),
            enabled: true,
        },
        config: ParallelNodeConfig {
            branches: analyst_branches,
            wait_for_all: true,
            timeout: Some(600),
            aggregation: Some("all".into()),
        },
    }));
    edges.push(edge("e-trigger-p-analysts", "trigger", "p-analysts"));

    // Phase 2: 决策检查点 — 记录分析师完成状态，辩论始终执行
    nodes.push(WorkflowNode::Condition(ConditionNode {
        base: WorkflowNodeBase {
            id: "c-need-debate".into(),
            title: "分析师检查点".into(),
            description: Some("确认9位分析师已全部完成，是否需辩论由后续流程决定".into()),
            position: Position { x: 300.0, y: 350.0 },
            retry: RetryConfig::default(),
            timeout: Some(60),
            enabled: true,
        },
        config: ConditionNodeConfig {
            conditions: vec![],
            logical_op: LogicalOperator::And,
            judge_by_llm: None,
            routing_prompt: None,
            routing_model: None,
        },
    }));
    edges.push(edge("e-p-analysts-c-debate", "p-analysts", "c-need-debate"));

    // 辩论 6 轮 — 线性执行，ConditionNode 记录判断结果但不路由分支
    let debate_pairs = [
        (
            "bull-r1",
            "基于9位分析师的报告和是否辩论的判断，构建该股票的买入论点",
            "bull-researcher",
            &["c-need-debate"][..],
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
    for (id, title, expert, deps, _next) in &debate_pairs {
        let mut an = agent(id, title, expert);
        let is_bull = expert.contains("bull");
        if let WorkflowNode::Agent(ref mut a) = an {
            a.config.tools = if is_bull {
                bull_tools.clone()
            } else {
                bear_tools.clone()
            };
            a.config.max_tool_rounds = Some(2);
            a.config.system_prompt =
                format!("{}{}", a.config.system_prompt, tool_prompt(&a.config.tools));
        }
        nodes.push(an);
        for dep in *deps {
            edges.push(edge(&format!("e-{dep}-{id}"), dep, id));
        }
    }

    // 风险评估（3 个并行，均依赖 bear-r3）
    for (rid, rtitle, rexpert, rtools) in &[
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
    ] {
        let mut an = agent(rid, rtitle, rexpert);
        if let WorkflowNode::Agent(ref mut a) = an {
            a.config.tools = rtools.clone();
            a.config.max_tool_rounds = Some(2);
            a.config.system_prompt = format!("{}{}", a.config.system_prompt, tool_prompt(rtools));
        }
        nodes.push(an);
        edges.push(edge(&format!("e-bear-r3-{rid}"), "bear-r3", rid));
    }

    // ── 算法 Tool 节点：仅 3 个核心评分/估值/风控 ——
    let algo_tools: &[(&str, &str, &str, &str)] = &[
        ("t-scoring", "技术评分", "compute_scoring", "stock_code"),
        ("t-valuation", "估值计算", "compute_valuation", "stock_code"),
        ("t-risk", "风险评估", "compute_portfolio_risk", "positions_json"),
    ];
    for (tool_id, title, tool_name, arg_key) in algo_tools {
        nodes.push(tool_node(tool_id, title, tool_name, tool_id, arg_key));
    }
    for rid in &["risk-agg", "risk-con", "risk-neu"] {
        edges.push(edge(&format!("e-{rid}-t-scoring"), rid, "t-scoring"));
    }
    edges.push(edge("e-t-scoring-t-valuation", "t-scoring", "t-valuation"));
    edges.push(edge("e-t-valuation-t-risk", "t-valuation", "t-risk"));

    // ── Validation: 结果完整性校验 ──
    nodes.push(WorkflowNode::Validation(ValidationNode {
        base: WorkflowNodeBase {
            id: "v-validate".into(),
            title: "结果完整性校验".into(),
            description: Some("确保分析报告包含必要字段，缺失时降级处理".into()),
            position: Position {
                x: 300.0,
                y: 1000.0,
            },
            retry: RetryConfig::default(),
            timeout: Some(60),
            enabled: true,
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
    edges.push(edge("e-t-risk-v-validate", "t-risk", "v-validate"));

    // research-mgr → trader → portfolio-mgr
    let mut rm = agent(
        "research-mgr",
        "综合三种风险偏好的评估结果，给出该股票的总体风险评级（低/中/高）及主要风险点清单",
        "research-manager",
    );
    if let WorkflowNode::Agent(ref mut a) = rm {
        a.config.context_sources = vec!["t-scoring".into(), "t-valuation".into(), "t-risk".into()];
        a.config.tools = vec![
            td_score.clone(),
            td_val.clone(),
            td_risk.clone(),
            td_fin.clone(),
            td_quote.clone(),
            td_kline.clone(),
            td_mf.clone(),
            td_maxdd.clone(),
            td_sharpe.clone(),
            td_var.clone(),
            td_pe_pct.clone(),
            td_peg.clone(),
            td_ma_cross.clone(),
            td_breakout.clone(),
            td_kelly.clone(),
            td_rp.clone(),
            td_beta.clone(),
            td_outliers.clone(),
            td_fill.clone(),
            td_adjust.clone(),
            td_research.clone(),
            td_consensus.clone(),
            td_concepts.clone(),
            td_announce.clone(),
            td_north.clone(),
            td_dragon.clone(),
            td_hot.clone(),
            td_industry.clone(),
            td_news.clone(),
            td_cls.clone(),
            td_atr.clone(),
            td_kdj.clone(),
            td_obv.clone(),
            td_earnings.clone(),
            td_pledge.clone(),
            td_corr.clone(),
            td_mc.clone(),
            td_ind.clone(),
            td_lup.clone(),
            td_block.clone(),
            td_visit.clone(),
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
    edges.push(edge("e-v-validate-research-mgr", "v-validate", "research-mgr"));

    // trader: 执行方案 — 实时行情 + 技术指标 + 凯利仓位
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
    }
    nodes.push(trader);
    edges.push(edge("e-research-mgr-trader", "research-mgr", "trader"));

    // portfolio-mgr: 最终决策 — 全量工具验证
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
            td_beta.clone(),
            td_earnings.clone(),
            td_pledge.clone(),
            td_corr.clone(),
            td_mc.clone(),
            td_ind.clone(),
            td_lup.clone(),
        ];
        a.config.system_prompt =
            format!("{}{}", a.config.system_prompt, tool_prompt(&a.config.tools));
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
        Variable {
            name: "analysis_dry_run".into(),
            var_type: "boolean".into(),
            value: serde_json::json!(false),
            description: Some("干跑模式：不调用 LLM，用 mock 输出验证流程".into()),
            is_secret: false,
        },
    ];
    let variables_val =
        serde_json::to_string(&variables).map_err(|e| format!("序列化变量失败: {e}"))?;

    // ── Phase 3/4: Rhai 综合评分工具 + ErrorConfig ──
    use axagent_core::workflow_types::RhaiToolDef;
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

    workflow_template::ActiveModel {
        id: Set(TEMPLATE_ID.to_string()),
        name: Set("A股多维度分析".to_string()),
        description: Set(Some(
            "9 维度分析师 → LLM 智能辩论 → 3 风险维度 → Rhai 评分 → 交易方案 → 投资决策"
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
                    "enabled": false,
                    "timezone": "Asia/Shanghai",
                }),
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

/// 种子化多空辩论子工作流模板，供 stock-analysis 主模板的 SubWorkflowNode 调用。
pub async fn seed_debate_subworkflow(db: &sea_orm::DatabaseConnection) -> Result<(), String> {
    use axagent_core::entity::workflow_template;
    use axagent_core::workflow_types::{
        AgentNode, AgentNodeConfig, EdgeType, OutputMode, Position, RetryConfig, WorkflowEdge,
        WorkflowNode, WorkflowNodeBase,
    };
    use sea_orm::{ActiveModelTrait, EntityTrait, Set};

    const DEBATE_ID: &str = "stock-debate";
    const DEBATE_VERSION: i32 = 1;

    if workflow_template::Entity::find_by_id(DEBATE_ID)
        .one(db)
        .await
        .map_err(|e| format!("查询辩论模板失败: {e}"))?
        .is_some()
    {
        return Ok(());
    }

    let now = chrono::Utc::now().timestamp_millis();
    let agent = |id: &str,
                 title: &str,
                 expert_id: &str,
                 tools: Vec<&str>,
                 deps: Vec<&str>,
                 _next: &str|
     -> (WorkflowNode, Vec<WorkflowEdge>) {
        let tool_names = tools.iter().map(|&n| n.to_string()).collect::<Vec<_>>();
        let n = WorkflowNode::Agent(AgentNode {
            base: WorkflowNodeBase {
                id: id.into(),
                title: title.into(),
                description: Some(format!("辩论: {expert_id}")),
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
                system_prompt: format!("你的任务: {title}\n工具: {}", tool_names.join(", ")),
                context_sources: vec![],
                output_var: id.into(),
                model: None,
                temperature: Some(0.3),
                max_tokens: Some(4096),
                tools: vec![],
                exposed_tools: vec![],
                output_mode: OutputMode::Text,
                agent_profile_id: Some(format!("stock-{expert_id}")),
                max_tool_rounds: Some(2),
                execution_mode: None,
                rag_source_ids: vec![],
            },
        });
        let edges: Vec<WorkflowEdge> = deps
            .iter()
            .map(|dep| WorkflowEdge {
                id: format!("e-{dep}-{id}"),
                source: dep.to_string(),
                source_handle: None,
                target: id.to_string(),
                target_handle: None,
                edge_type: EdgeType::Direct,
                label: None,
            })
            .collect();
        (n, edges)
    };

    let pairs = [
        (
            "bull-r1",
            "构建买入论点",
            "bull-researcher",
            vec!["compute_scoring"],
            vec![],
            "bear-r1",
        ),
        (
            "bear-r1",
            "反驳多方论点",
            "bear-researcher",
            vec!["compute_scoring"],
            vec!["bull-r1"],
            "bull-r2",
        ),
        (
            "bull-r2",
            "二次反击",
            "bull-researcher",
            vec!["compute_scoring"],
            vec!["bear-r1"],
            "bear-r2",
        ),
        (
            "bear-r2",
            "挖掘隐藏风险",
            "bear-researcher",
            vec!["compute_scoring"],
            vec!["bull-r2"],
            "bull-r3",
        ),
        (
            "bull-r3",
            "终极多方辩护",
            "bull-researcher",
            vec!["compute_scoring"],
            vec!["bear-r2"],
            "bear-r3",
        ),
        (
            "bear-r3",
            "终极空方陈词",
            "bear-researcher",
            vec!["compute_scoring"],
            vec!["bull-r3"],
            "",
        ),
    ];

    let mut nodes = Vec::new();
    let mut edges = Vec::new();
    for (id, title, expert, tools, deps, _next) in &pairs {
        let (n, e) = agent(id, title, expert, tools.clone(), deps.to_vec(), "");
        nodes.push(n);
        edges.extend(e);
    }

    let nodes_json =
        serde_json::to_string(&nodes).map_err(|e| format!("序列化辩论节点失败: {e}"))?;
    let edges_json = serde_json::to_string(&edges).map_err(|e| format!("序列化辩论边失败: {e}"))?;

    workflow_template::ActiveModel {
        id: Set(DEBATE_ID.to_string()),
        name: Set("多空辩论".to_string()),
        description: Set(Some(
            "6 轮多空辩论：bull-r1→bear-r1→bull-r2→bear-r2→bull-r3→bear-r3".into(),
        )),
        icon: Set("swords".into()),
        tags: Set(Some(serde_json::to_string(&["debate", "stock"]).unwrap())),
        version: Set(DEBATE_VERSION),
        is_preset: Set(true),
        is_editable: Set(false),
        is_public: Set(true),
        trigger_config: Set(None),
        nodes: Set(nodes_json),
        edges: Set(edges_json),
        input_schema: Set(None),
        output_schema: Set(None),
        variables: Set(Some("[]".into())),
        error_config: Set(None),
        composite_source: Set(None),
        tool_defs: Set(None),
        created_at: Set(now),
        updated_at: Set(now),
    }
    .insert(db)
    .await
    .map_err(|e| format!("写入辩论模板失败: {e}"))?;

    tracing::info!("[stock_analysis_setup] 辩论子工作流模板已种子化 ({DEBATE_ID})");
    Ok(())
}
