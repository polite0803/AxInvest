//! 股票分析专家与工作流模板种子化。
//!
//! 子模块：
//! - seed_stock_analysis: 股票分析主工作流模板种子
//! - seed_serenity: Serenity 瓶颈筛选工作流模板种子
//! - seed_daily_market_events: G4 每日市场主线提炼工作流模板种子
//! - seed_screenshot_portfolio_diagnosis: G6 截图持仓诊断工作流模板种子
//! - seed_news_cross_market: G3.3 新闻→跨市场传导分析工作流模板种子
//!
//! 注：Multi-Agent 固定角色（analyst/implementer/reviewer）种子化已迁移到上游
//! `commands/multi_agent_setup/seed_multi_agent_roles`，本模块不再负责。

pub mod seed_concept_index;
pub mod seed_daily_market_events;
pub mod seed_news_cross_market;
pub mod seed_screenshot_portfolio_diagnosis;
pub mod seed_serenity;
pub mod seed_serenity_fast;
pub mod seed_stock_analysis;
pub mod seed_variables;

// 四张档子模板的节点/边定义 + 播种（B-2b 步骤 2/2.5，PLAN §九十一 / §九十二 / §一○○）。
// v135 起生产可见：主图的四个 `pm-h-<档>` SubWorkflow 扇出按 `horizon_tier_template_id` 指向它们。
pub(crate) mod horizon_tier_template;

// 仅测试构建：seed 工具声明 ↔ 运行时解析空间 一致性校验
#[cfg(test)]
mod seed_consistency_tests;

// 股票分析专家/角色/Profile 自动种子化到 agency_experts/agent_roles/agent_profiles 表。
// 使用 include_str! 编译期嵌入 .md 内容，打包后无需文件 I/O。

use crate::commands::error::ErrorResponse;
use crate::commands::error_code::stock_setup;
use axagent_dao::repo;
use seed_daily_market_events::seed_daily_market_events_template;
use seed_screenshot_portfolio_diagnosis::seed_screenshot_portfolio_diagnosis_template;
use seed_serenity::seed_serenity_screening_workflow_template;
use seed_serenity_fast::seed_serenity_fast_workflow_template;
use seed_stock_analysis::{
    seed_stock_analysis_fast_workflow_template, seed_stock_analysis_workflow_template,
};

/// 逐**值**比较两段 JSON —— **不是**字符串比较。
///
/// 三链种子共用（原链 `seed_serenity` / 快速趋势智选 `seed_serenity_fast` /
/// 股票分析快速链 `seed_stock_analysis`）：三处的版本门都要拿「DB 现有内容」与
/// 「本轮代码将写入的内容」比对，判据必须只有一份，否则会各自漂移。
///
/// 必要性（实测）：`variables` 在本模块要经过 `merge_variable_values` 往返，而那个函数是
/// `serde_json::from_str` → `to_string`。`serde_json::Value` 的 Map 默认是 **BTreeMap**
/// （按 key 排序），而 `serde_json::to_string(&Vec<Variable>)` 输出的是**结构体字段声明序**
/// ⇒ 同一份变量集合，两次序列化出的字节串**不同**。
/// 若门禁按字符串比对，会恒判「不一致」⇒ **每次启动都重建模板**（单测的哨兵名当场被覆盖，
/// 2026-09-24 实测）。
///
/// 图谱指纹（`nodes` / `edges`）同样走本函数：DB 文本可能来自旧序列化器或工作流编辑器保存，
/// 键序 / 空白 / 浮点写法（`1.0` vs `1`）都可能与本轮 `serde_json::to_string` 的输出不同。
/// 对象比较键序无关；数组比较**保序** —— 节点被重排会判为「不一致」并触发重建 / 告警，
/// 这正是期望：那张图已经不是代码产出的那张了。
///
/// 判据边界：任一侧不是合法 JSON ⇒ 返回 `false`（保守方向 = 判定「不同」⇒ 重建或告警），
/// 不做「两侧都解析失败 ⇒ 视为相同」的推断 —— 那会让坏数据静默留在库里。
pub(crate) fn same_json(left: &str, right: &str) -> bool {
    match (
        serde_json::from_str::<serde_json::Value>(left),
        serde_json::from_str::<serde_json::Value>(right),
    ) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    }
}

/// 已注销的专家 id。
///
/// ⚠ 从 `EMBEDDED_PROMPTS` / `EXPERT_ROLE_MAP` / `PROFILE_TOOLS` 移除某个专家时，
/// **必须**把它的 id 登记到这里：上述三个数组只是 seed 的「建」侧，而
/// `seed_agency_experts` / `seed_agent_profiles` 只做 UPSERT（有则 update、无则
/// insert），**从不删除**。因此已注销专家会以旧行形式残留在 DB 中，前端按
/// `source_dir = "stock-analysis"` 聚合专家列表时会继续展示它
/// （典型症状：「界面里还能选，但它已不参与工作流」）。
/// 每次种子化结束会对本清单执行定向删除（幂等；失败仅告警，不阻断启动）。
///
/// 2026-09-14: `data-quality-inspector` —— 其职责已由确定性 CodeNode
/// `data-quality.rhai` 承担（该节点输出 `grade` / `score` / `diagnostics`，
/// 是数据质量分档的唯一权威；见 `AUDIT-data-quality-dual-algorithm-2026-09-14.md`）。
const RETIRED_EXPERT_IDS: &[&str] = &["data-quality-inspector"];

/// 编译期嵌入的专家提示词（include_str 确保打包后可用）
const EMBEDDED_PROMPTS: &[(&str, &str)] = &[
    ("market-analyst", include_str!("../../../agency_experts/stock-analysis/market-analyst.md")),
    (
        "sentiment-analyst",
        include_str!("../../../agency_experts/stock-analysis/sentiment-analyst.md"),
    ),
    ("news-analyst", include_str!("../../../agency_experts/stock-analysis/news-analyst.md")),
    (
        "fundamentals-analyst",
        include_str!("../../../agency_experts/stock-analysis/fundamentals-analyst.md"),
    ),
    ("policy-analyst", include_str!("../../../agency_experts/stock-analysis/policy-analyst.md")),
    (
        "hot-money-tracker",
        include_str!("../../../agency_experts/stock-analysis/hot-money-tracker.md"),
    ),
    ("lockup-watcher", include_str!("../../../agency_experts/stock-analysis/lockup-watcher.md")),
    (
        "research-analyst",
        include_str!("../../../agency_experts/stock-analysis/research-analyst.md"),
    ),
    ("sector-analyst", include_str!("../../../agency_experts/stock-analysis/sector-analyst.md")),
    ("bull-researcher", include_str!("../../../agency_experts/stock-analysis/bull-researcher.md")),
    ("bear-researcher", include_str!("../../../agency_experts/stock-analysis/bear-researcher.md")),
    ("bull-r2", include_str!("../../../agency_experts/stock-analysis/bull-r2.md")),
    ("bear-r2", include_str!("../../../agency_experts/stock-analysis/bear-r2.md")),
    ("bull-r3", include_str!("../../../agency_experts/stock-analysis/bull-r3.md")),
    ("bear-r3", include_str!("../../../agency_experts/stock-analysis/bear-r3.md")),
    (
        "aggressive-debator",
        include_str!("../../../agency_experts/stock-analysis/aggressive-debator.md"),
    ),
    (
        "conservative-debator",
        include_str!("../../../agency_experts/stock-analysis/conservative-debator.md"),
    ),
    ("neutral-debator", include_str!("../../../agency_experts/stock-analysis/neutral-debator.md")),
    (
        "research-manager",
        include_str!("../../../agency_experts/stock-analysis/research-manager.md"),
    ),
    ("trader", include_str!("../../../agency_experts/stock-analysis/trader.md")),
    (
        "value-investor",
        include_str!("../../../agency_experts/stock-analysis/custom/value-investor.md"),
    ),
    (
        "quality-fallback",
        include_str!("../../../agency_experts/stock-analysis/quality-fallback.md"),
    ),
    ("rule-checker", include_str!("../../../agency_experts/stock-analysis/rule-checker.md")),
    (
        "catalyst-analyst",
        include_str!("../../../agency_experts/stock-analysis/catalyst-analyst.md"),
    ),
    (
        "debate-convergence",
        include_str!("../../../agency_experts/stock-analysis/debate-convergence.md"),
    ),
    (
        "risk-convergence",
        include_str!("../../../agency_experts/stock-analysis/risk-convergence.md"),
    ),
    ("reflection", include_str!("../../../agency_experts/stock-analysis/reflection.md")),
    // ── Serenity 瓶颈分析 4 专家 ──
    ("trend-scanner", include_str!("../../../agency_experts/stock-analysis/trend-scanner.md")),
    (
        "chain-decomposer",
        include_str!("../../../agency_experts/stock-analysis/chain-decomposer.md"),
    ),
    (
        "chokepoint-identifier",
        include_str!("../../../agency_experts/stock-analysis/chokepoint-identifier.md"),
    ),
    (
        "candidate-mapper",
        include_str!("../../../agency_experts/stock-analysis/candidate-mapper.md"),
    ),
    // ── P2: 借鉴 TradingAgents 的新分析师 ──
    (
        "social-media-analyst",
        include_str!("../../../agency_experts/stock-analysis/social-media-analyst.md"),
    ),
    (
        "volume-price-analyst",
        include_str!("../../../agency_experts/stock-analysis/volume-price-analyst.md"),
    ),
    // ── 简化模板升级：3 个新专家 ──
    (
        "market-synthesizer",
        include_str!("../../../agency_experts/stock-analysis/market-synthesizer.md"),
    ),
    (
        "industry-chain-analyzer",
        include_str!("../../../agency_experts/stock-analysis/industry-chain-analyzer.md"),
    ),
    (
        "screenshot-diagnoser",
        include_str!("../../../agency_experts/stock-analysis/screenshot-diagnoser.md"),
    ),
    // ── 事件驱动模板：仓位规划与止损复查 ──
    (
        "position-planner",
        include_str!("../../../agency_experts/stock-analysis/position-planner.md"),
    ),
    (
        "stop-loss-reviewer",
        include_str!("../../../agency_experts/stock-analysis/stop-loss-reviewer.md"),
    ),
    // ── P0 补齐: decision-explainer（三明治第三段的「翻译说明书」节点）──
    // 背景：模板节点 `decision-explainer` 的 agent_profile_id = "stock-explainer"，
    // 但三处注册（.md / EMBEDDED_PROMPTS / EXPERT_ROLE_MAP）此前**全无** explainer
    // ⇒ profile `stock-explainer` 不存在 ⇒ agent_executor 的 profile 解析返回 None
    // ⇒ expert 提示词整段跳过（只打一条 WARN，节点不失败）—— 静默降级。
    // 该节点只做「把符号裁决翻译成人话」，故 tools=[]（见 PROFILE_TOOLS）。
    ("explainer", include_str!("../../../agency_experts/stock-analysis/explainer.md")),
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
    ("bull-r3", "debater"),
    ("bear-r3", "debater"),
    ("aggressive-debator", "risk-evaluator"),
    ("conservative-debator", "risk-evaluator"),
    ("neutral-debator", "risk-evaluator"),
    ("research-manager", "decision-maker"),
    ("trader", "trader"),
    ("value-investor", "stock-analyst"),
    ("quality-fallback", "decision-maker"),
    ("rule-checker", "risk-evaluator"),
    ("catalyst-analyst", "stock-analyst"),
    ("debate-convergence", "debater"),
    ("risk-convergence", "risk-evaluator"),
    ("reflection", "decision-maker"),
    // ── Serenity 瓶颈分析师 ──
    ("trend-scanner", "stock-analyst"),
    ("chain-decomposer", "stock-analyst"),
    ("chokepoint-identifier", "stock-analyst"),
    ("candidate-mapper", "stock-analyst"),
    ("social-media-analyst", "stock-analyst"),
    ("volume-price-analyst", "stock-analyst"),
    // ── 简化模板升级：3 个新专家角色映射 ──
    ("market-synthesizer", "stock-analyst"),
    ("industry-chain-analyzer", "stock-analyst"),
    ("screenshot-diagnoser", "stock-analyst"),
    // ── 事件驱动模板：仓位规划与止损复查角色映射 ──
    ("position-planner", "decision-maker"),
    ("stop-loss-reviewer", "decision-maker"),
    // 决策解释官：产出是人话说明书，归入决策者层（与 research-manager 等同层）
    ("explainer", "decision-maker"),
];

struct StockRoleDef {
    id: &'static str,
    name: &'static str,
    description: &'static str,
    system_prompt: &'static str,
    max_concurrent: i32,
    timeout_seconds: i64,
}

/// AxInvest 专属角色 — 证券投资负责人。
///
/// 本角色（stock-investment-lead）seed 进 agent_roles，作为股票专家 profile 的
/// agent_role 引用，其 system_prompt 注入最外层身份（投资决策责任与合规边界）。
const STOCK_AGENT_ROLE_ID: &str = "stock-investment-lead";

struct StockAgentRoleDef {
    id: &'static str,
    name: &'static str,
    description: &'static str,
    responsibilities: &'static [&'static str],
    decision_authority: &'static str,
    required_certifications: &'static [&'static str],
    active_domains: &'static [&'static str],
    system_prompt: &'static str,
    icon: &'static str,
    color: &'static str,
}

const STOCK_AGENT_ROLE: StockAgentRoleDef = StockAgentRoleDef {
    id: STOCK_AGENT_ROLE_ID,
    name: "证券投资负责人",
    description: "领导多专家团队进行 A 股证券投资分析与决策，对决策合规性与风险调整后收益负责",
    responsibilities: &[
        "组织多专家团队完成 A 股标的的多维度分析",
        "评估投资风险与仓位边界，制定风险调整后收益最大化方案",
        "决策买入 / 持有 / 卖出动作，维护决策链路的可追溯性",
        "确保分析过程遵循监管要求与合规边界",
    ],
    decision_authority: r#"{"max_position_pct":100,"scopes":["stock-analysis","portfolio-mgmt","risk-assessment"]}"#,
    required_certifications: &["证券从业资格", "5 年 A 股研究经验"],
    // 修复: 原 "stock-analysis"/"finance" 均非合法 ToolDomain 字符串（parse_domain_str
    // 只认 core/general/devops/ai_media/invest/opc），解析为全集 → 分析师 AgentNode 经
    // get_chat_tools_for_domains 只拿到 MCP 工具，丢失所有非-MCP 本地工具（含 invest 域）。
    // 改为合法域: invest（投资域）+ core/general（通用能力），使领域过滤真正生效且不过窄。
    active_domains: &["invest", "core", "general"],
    // 分层原则：
    // - Role: 身份 + 职责 + 权限 + 合规边界（通用、稳定）
    // - Expert: 方法论 + 评分体系 + 输出格式（专业、可演进）
    system_prompt: "你是证券投资负责人，领导多专家团队进行 A 股投资分析与决策。\
    \n\n职责：组织多维度分析，评估风险与收益，决策买入/持有/卖出。\
    \n权限：对所有投资建议承担可追溯的合规责任。\
    \n合规：杜绝内幕信息与市场操纵，所有结论基于公开数据。\
    \n目标：以风险调整后收益最大化为目标。",
    icon: "📈",
    color: "#dc2626",
};

/// 投研办公室子岗位 — 对应 INVESTMENT_OFFICE_TEMPLATE 中的 6 个房间。
///
/// 这些岗位作为 `stock-investment-lead` 的下属存在
/// （reports_to = STOCK_AGENT_ROLE_ID），用于：
/// - AddMemberModal 的角色下拉中可按房间选择对应角色
/// - 角色 system_prompt 注入到 dispatcher 路由上下文，引导 LLM 将股票相关
///   消息路由到合适房间（如「查询行情」→ data-lead，「下单」→ trading-lead）
///
/// 颜色与 sceneTemplates.ts 中的房间 color 保持一致，前端卡片与 Sprite
/// 渲染时通过 role 反查颜色，无需再维护 ROLE_COLORS 映射表。
const STOCK_AGENT_SUB_ROLES: &[StockAgentRoleDef] = &[
    StockAgentRoleDef {
        id: "stock-research-lead",
        name: "投研负责人",
        description: "领导行业研究、基本面分析与研报撰写，对应办公室「投研室」",
        responsibilities: &[
            "组织行业景气度跟踪与上下游调研",
            "统筹基本面分析（财务 / 估值 / 成长性）",
            "撰写深度研报并标注证据链与置信度",
        ],
        decision_authority: r#"{"max_position_pct":50,"scopes":["research","fundamental-analysis"]}"#,
        required_certifications: &["证券从业资格", "3 年行业研究经验"],
        active_domains: &["invest", "core"],
        system_prompt: "你是投研负责人，专注行业景气度跟踪、基本面深度分析与研报撰写。所有结论必须标注证据来源（公告/财报/调研/数据接口）与置信度（high/medium/low）。对不确定性显式标注 data_gaps，禁止编造未公开数据。在办公室中常驻「投研室」，对接消息涉及行业研究、基本面、研报、公告解读时主动接手。",
        icon: "🔬",
        color: "#1677ff",
    },
    StockAgentRoleDef {
        id: "stock-data-lead",
        name: "数据负责人",
        description: "对接 astock-data 行情/财务/新闻接口，对应办公室「数据室」",
        responsibilities: &[
            "对接 astock-data MCP 工具集（行情/K线/财务/新闻）",
            "校验数据质量并标注 dqi_score",
            "为其他角色提供数据上下文与回测样本",
        ],
        decision_authority: r#"{"max_position_pct":0,"scopes":["data-query","data-quality"]}"#,
        required_certifications: &["证券从业资格", "熟悉量化数据接口"],
        active_domains: &["invest", "core", "general"],
        system_prompt: "你是数据负责人，对接 astock-data MCP 工具集，提供行情、K线、财务、新闻等数据查询与质量校验。返回结果必须包含数据时间戳、来源、dqi_score；数据缺失或异常时显式标注 untrusted=true 并触发 weights collapse。在办公室中常驻「数据室」，消息涉及查询行情/财务/新闻数据时主动接手。",
        icon: "📡",
        color: "#13c2c2",
    },
    StockAgentRoleDef {
        id: "stock-meeting-host",
        name: "晨会主持",
        description: "组织晨会、投研会议与多空辩论，对应办公室「会议室」",
        responsibilities: &[
            "组织每日晨会议题与市场主线提炼",
            "主持多空辩论与同行评估",
            "汇总分歧并形成会议纪要",
        ],
        decision_authority: r#"{"max_position_pct":0,"scopes":["meeting","debate"]}"#,
        required_certifications: &["证券从业资格", "2 年投研经验"],
        active_domains: &["invest", "core"],
        system_prompt: "你是晨会主持，组织每日晨会议题、市场主线提炼与多空辩论。所有议题须基于已验证数据，对分歧观点要求辩手给出可证伪的判定条件。会议纪要须包含：议题 / 主线 / 分歧 / 多空观点 / 决议。在办公室中常驻「会议室」，消息涉及晨会议题、主线提炼、辩论组织时主动接手。",
        icon: "🎤",
        color: "#722ed1",
    },
    StockAgentRoleDef {
        id: "stock-strategy-lead",
        name: "策略负责人",
        description: "策略研发、回测与组合优化，对应办公室「策略室」",
        responsibilities: &[
            "研发并验证投资策略（趋势/价值/量化）",
            "对接 quant crate 进行回测与 walkforward 验证",
            "输出策略列表与建议仓位上限",
        ],
        decision_authority: r#"{"max_position_pct":80,"scopes":["strategy","backtest","portfolio"]}"#,
        required_certifications: &["证券从业资格", "3 年策略研发经验"],
        active_domains: &["invest", "core"],
        system_prompt: "你是策略负责人，负责研发、回测与组合优化。对接 quant crate 进行 walkforward 验证，输出策略列表与建议仓位上限。所有策略须附回测报告（年化收益/最大回撤/夏普/胜率），禁止推荐未回测的策略。在办公室中常驻「策略室」，消息涉及策略研发、回测、组合优化时主动接手。",
        icon: "🎯",
        color: "#eb2f96",
    },
    StockAgentRoleDef {
        id: "stock-trading-lead",
        name: "交易负责人",
        description: "执行下单、止损止盈与 T+1 涨跌停合规检查，对应办公室「交易室」",
        responsibilities: &[
            "制定入场/出场/分批方案",
            "执行 T+1、涨跌停、停牌合规检查",
            "对接 paper_portfolio 模拟成交记录",
        ],
        decision_authority: r#"{"max_position_pct":100,"scopes":["trading","execution","paper-portfolio"]}"#,
        required_certifications: &["证券从业资格", "熟悉 A 股交易规则"],
        active_domains: &["invest", "core"],
        system_prompt: "你是交易负责人，制定入场/出场/分批方案并执行 T+1、涨跌停、停牌合规检查。所有交易指令须附合规检查结果与 paper_portfolio 记录。禁止违反 T+1 与涨跌停规则。在办公室中常驻「交易室」（投研办公室默认房间），消息涉及下单、改单、撤单、止损止盈时主动接手。",
        icon: "⚡",
        color: "#f5222d",
    },
    StockAgentRoleDef {
        id: "stock-risk-lead",
        name: "风控负责人",
        description: "风险评估、压力测试与合规边界，对应办公室「风控室」",
        responsibilities: &[
            "识别投资风险（系统性/行业/个股）并量化评估",
            "组织压力测试与情景分析",
            "对违规操作触发 weights collapse 与仓位上限",
        ],
        decision_authority: r#"{"max_position_pct":100,"scopes":["risk","stress-test","compliance"]}"#,
        required_certifications: &["证券从业资格", "FRM 或 3 年风控经验"],
        active_domains: &["invest", "core"],
        system_prompt: "你是风控负责人，识别投资风险并量化评估，组织压力测试与情景分析。对数据质量 F 级（dqi_score<25）触发 weights collapse：position_pct=0、confidence×0.5、action 降级为「观望」。对所有建议保留合规审计追溯链。在办公室中常驻「风控室」，消息涉及回撤、压测、行业暴露、相关性、合规时主动接手。",
        icon: "🛡️",
        color: "#fa8c16",
    },
];

const STOCK_ROLES: &[StockRoleDef] = &[
    StockRoleDef {
        id: "stock-analyst",
        name: "股票分析师",
        description: "A股多维分析",
        system_prompt: "你是专业的 A 股分析师，基于行情数据、财务数据、新闻资讯等对股票进行深度分析。",
        // stock-analyst 角色的并发上限：覆盖全部 a-* 分析师 + 专项分析师
        // （Serenity 链条、催化剂、社媒/量价等），留 1 槽位余量。
        max_concurrent: 15,
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
pub(crate) static PROFILE_TOOLS: &[(&str, &[&str])] = &[
    (
        "market-analyst",
        &[
            // P0 修复(2026-07-22): 移除 get_stock_kline——上游 t-market-data 已获取并通过
            // context_sources 注入，LLM 重新调用会重复获取数据 + 可能传入空 stock_code。
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
            // P0 修复(2026-07-22): 移除 get_social_sentiment——上游 t-sentiment-data 已获取。
            // 保留 get_stock_news/get_stock_money_flow：a-sentiment 的 context_sources 只有
            // t-sentiment-data，看不到 t-news-data/t-hotmoney-data 的数据，LLM 主动调用是合理补充。
            "get_stock_news",
            "get_stock_money_flow",
            "get_stock_option_pcr",
            "get_stock_dragon_tiger",
            // 2026-08-01 恢复 get_north_bound_flow：净流入停披但成交额仍披露（v3 返回成交额，
            // timestamp 标注非净流入），北向成交活跃度仍是资金面信号。
            "get_north_bound_flow",
            "get_stock_margin_data",
            "get_stock_quote",
            "search_stock",
            // P9-4：涨停池的**广度面**（涨停家数 / 触板数 / 封板率 / 炸板数）⇒ `breadthState`。
            "get_limit_up_pool",
        ],
    ),
    (
        "news-analyst",
        &[
            // P0 修复(2026-07-22): 移除 get_stock_news——上游 t-news-data 已获取。
            "get_stock_announcements",
            "get_cls_flash",
            "get_stock_option_pcr",
            "search_stock",
        ],
    ),
    (
        "fundamentals-analyst",
        &[
            // V63 修复(2026-07-23): 移除 get_stock_financials——上游 t-fundamentals-data
            // 用 get_fundamentals_report_markdown 已预聚合所有关键财务指标（含
            // PE/PB/ROE/毛利率/净利率/资产负债率/FCF收益率/同比增速 + 商誉/应收账款）。
            // LLM 再调 get_stock_financials 会返回多期原始财报 JSON（每期 ~20 字段），
            // 与预聚合报告数据重复 → input tokens 膨胀 → output 超 max_tokens 截断 →
            // VERDICT 标签被切掉。与 a-market-analyst 移除 get_stock_kline 同一模式。
            "compute_valuation",
            "get_stock_consensus_eps",
            "get_stock_institutional_visits",
            "get_stock_peers",
            "search_stock",
            // 2026-09-20：产品决策反转（§7.3 原「不接」→ 现接入）——
            // 把 detect_earnings_surprise 接给 fundamentals-analyst，用于基于
            // 一致预期EPS判业绩超预期/低于预期。
            "detect_earnings_surprise",
        ],
    ),
    (
        "policy-analyst",
        &[
            "search_news",
            "get_stock_news",
            "get_cls_flash",
            "search_stock",
            // P9-1：宏观真源快照 —— `macroRegime` 因子的数据侧。节点已前置取数，
            // 这里授权是给 LLM 一个按同口径重取的出口，**不是**让模型自己填日期。
            "macro_data_snapshot",
        ],
    ),
    (
        "hot-money-tracker",
        &[
            // P0 修复(2026-07-22): 移除 get_stock_money_flow——上游 t-hotmoney-data 已获取。
            // 2026-07-25 修复: 补充 get_stock_margin_data（融资融券）——lockup-watcher
            // 虽也有此工具，但 hot-money-tracker 的分析需要融资融券作为真金白银信号，
            // 且 lockup-watcher 不保证在 hot-money-tracker 之前运行。
            "get_stock_dragon_tiger",
            // 2026-08-01 恢复 get_north_bound_flow（v3 返回成交额，净流入停披但成交额仍披露）
            "get_north_bound_flow",
            "get_stock_institutional_visits",
            "get_stock_margin_data",
            // P9-4：涨停池的**资金面**（封单额/量、炸板次数、连板结构）⇒ `microstructure`。
            // 与 a-sentiment 同读一个工具不是重复计数：两个因子取的是响应里的两组字段。
            "get_limit_up_pool",
            "search_stock",
        ],
    ),
    (
        "lockup-watcher",
        &[
            // V63 修复(2026-07-23): 移除 get_stock_lockup / get_stock_shareholder_trades /
            //   get_stock_block_trades——上游 t-lockup-data 调用 get_stock_lockup_bundle
            //   已返回 {lockup_schedule, shareholder_trades, block_trades} 三个字段的
            //   bundled JSON。LLM 再分别调用这三个工具会获取完全相同的数据，
            //   三份重复 JSON 注入 messages → input tokens 膨胀 3 倍 → output 截断。
            // 保留 get_stock_margin_data（bundle 不含融资融券）和
            //   get_stock_announcements（bundle 不含公告）作为补充数据源。
            // 2026-09-21(v72): 补 `get_stock_pledge_data` —— 专家 prompt 的方法论第 4 条
            //   与自检清单都要求评估「质押比例 > 50% 高警戒线 / 质押风险敞口」，而
            //   bundle 是解禁+增减持+大宗交易**三方**（结构上不含质押），本白名单此前
            //   也没有任何质押工具 ⇒ 该维度**每轮必缺**，模型只能自己给缺口编原因
            //   （全库 15 轮里 12 轮写了质押缺口，最远漂到「工具调用被拒绝」的伪归因，
            //    见 `AUDIT-pledge-attribution-2026-09-21.md`）。
            //   上游预拉由节点 `t-pledge-data` 承担（seed_stock_analysis.rs），
            //   本行是**第二层**授权：预拉之外的补充取数 + 预拉失败时仍可自救。
            //   ⚠ 与 `lockup-watcher.md` frontmatter 的 `data_sources` 同源，改一处必改另一处。
            "get_stock_pledge_data",
            "get_stock_margin_data",
            "get_stock_announcements",
            "search_stock",
        ],
    ),
    (
        "research-analyst",
        &[
            // V63 修复(2026-07-23): 移除 get_stock_financials——研报分析师的核心数据是
            // 上游 t-research-data 预拉的研报列表（含分析师评级/EPS预测/目标价），
            // 不需要原始财报 JSON。get_stock_financials 返回多期原始财报（每期 ~20 字段），
            // 与研报中的财务预测数据重复 → input tokens 膨胀 → output 截断 → VERDICT 丢失。
            // 与 fundamentals-analyst 移除 get_stock_financials 同一模式。
            "get_stock_consensus_eps",
            "get_stock_news",
            "get_stock_institutional_visits",
            "search_stock",
        ],
    ),
    (
        "sector-analyst",
        &[
            // P0 修复(2026-07-22): 移除 get_industry_ranking——上游 t-sector-data 已获取。
            "get_hot_stocks",
            "get_stock_quote",
            "get_stock_concept_blocks",
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
    // R3 最终反驳型辩手同样需要 compute_scoring / compute_valuation 来核实对方 R2 质询
    // 背后的技术指标与估值假设，否则"逐条回应"会沦为文本辩论。
    ("bull-r3", &["compute_scoring", "compute_valuation", "search_stock"]),
    ("bear-r3", &["compute_scoring", "compute_valuation", "search_stock"]),
    ("aggressive-debator", &["compute_portfolio_risk", "search_stock"]),
    ("conservative-debator", &["compute_portfolio_risk", "search_stock"]),
    ("neutral-debator", &["compute_portfolio_risk", "search_stock"]),
    (
        "research-manager",
        &["compute_scoring", "compute_valuation", "compute_portfolio_risk", "search_stock"],
    ),
    ("trader", &["get_stock_quote", "compute_scoring", "search_stock"]),
    (
        "value-investor",
        &[
            "get_stock_financials",
            "compute_valuation",
            "get_stock_consensus_eps",
            "get_stock_institutional_visits",
            "get_stock_peers",
            "search_stock",
        ],
    ),
    // ── P3 (real-nodes): 规则检查员 ──
    // 2026-09-14: 原 "data-quality-inspector" 的工具白名单已随之移除——该职责现由
    // 确定性 CodeNode `data-quality.rhai` 承担，不再作为 LLM 专家参与工作流。
    // quality-fallback: 数据降级时的保守决策，只需少量查询
    ("quality-fallback", &["get_stock_quote", "get_stock_kline", "compute_scoring"]),
    // rule-checker 需要读取技术指标与估值/风控结果
    (
        "rule-checker",
        &["compute_scoring", "compute_valuation", "compute_portfolio_risk", "search_stock"],
    ),
    // ── Catalyst & Narrative Analyst ──
    // 需要读取新闻/公告做催化剂判断 + K线/量价做机构行为分析
    // P0 修复(2026-07-22): 移除未实现的 get_announcement_content（PDF 全文解析为 P2 功能，尚未落地）
    (
        "catalyst-analyst",
        &[
            "get_stock_news",
            "get_stock_announcements",
            "get_stock_concept_blocks",
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
            "get_stock_concept_blocks",
            // 2026-08-01 恢复 get_north_bound_flow（v3 返回成交额，净流入停披但成交额仍披露）
            "get_north_bound_flow",
            "get_market_dragon_tiger",
            "search_stock",
        ],
    ),
    // chain-decomposer: 拆解产业链，需行业/概念/同业数据
    (
        "chain-decomposer",
        &[
            "get_stock_concept_blocks",
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
            "get_stock_research_reports",
            "get_stock_consensus_eps",
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
            "get_stock_institutional_visits",
            "get_stock_research_reports",
            "get_stock_news",
            "search_stock",
        ],
    ),
    // ── 简化模板升级：3 个新专家工具映射 ──
    // market-synthesizer: 市场主线综合，需多源数据采集 + 持久化
    // 2026-09-19 订正 —— **判据空间是 chat 工具注册表，不是 `tool_def_map`**：
    //   本表有两条消费链：① stock_analysis 模板节点工具（`seed_stock_analysis.rs` 的
    //   `let tool_names = PROFILE_TOOLS` 经 `tool_def_map` 解析）
    //   ② `agent_profiles.recommended_tools`（本文件内锚点
    //   `PROFILE_TOOLS.iter().cloned()` → `recommended_tools: Set(tools_json)` 的两处 UPSERT）
    //   → chat 侧 `local_tool.rs` 的 `get_chat_tools_by_names` 追加给 LLM。
    //   ⚠ 本注释不写行号：本文件增删一行即失效（已实测腐烂两轮），认锚点。
    //   本专家**不在** stock_analysis 模板 ⇒ 链① 不生效，唯一生效的是链②，
    //   其名字空间 = `crates/astock-data/src/mcp_tools.rs` 的工具名表。
    //   原 3 项在该表中均不存在（静默过滤丢掉，但前端 ExpertSelector 照显），逐条处理：
    //     · `get_dragon_tiger_list` → `get_market_dragon_tiger`（同物异名，1:1）
    //     · `get_north_flow`        → `get_north_bound_flow`（同物异名，1:1）
    //     · `market_mainline_batch_upsert` 删除 —— 它**不是幽灵**：是 daily-market-events
    //       模板自己的 `ToolDef`，认字符串 `name: "market_mainline_batch_upsert"`（在
    //       `seed_daily_market_events.rs` 的 `agent_tools` 列表内）；该模板的 Agent 节点
    //       用 `tools: agent_tools` 自带它 ⇒ 工作流侧不依赖本表；chat 侧该表无此名。
    //       （此处不写行号：外部文件一改即失效。）
    //       2026-09-20 更新：命题「工作流侧不依赖本表」成立，但**当时它根本解析不到** ——
    //       工作流的工具解析走 ToolResolver（判据 = `register_all` 注册表 ∪ MCP 工具表），
    //       `#[agent_command]` 元数据不在其中，故该名字恒解析为 None、调用被静默降级。
    //       现已补为真实工具（`crates/tools/src/tools/market_mainline.rs`，注册于
    //       `tools/mod.rs` 的 register_all）⇒ 模板侧真正可用；本表仍**不加**它：
    //       本表链②查的是 `crates/astock-data/src/mcp_tools.rs` 的名字表，不含该工具。
    //       「是否也把市场主线工具暴露给 chat 专家」属产品决策，未擅自扩。
    (
        "market-synthesizer",
        &["get_hot_stocks", "get_cls_flash", "get_market_dragon_tiger", "get_north_bound_flow"],
    ),
    // industry-chain-analyzer: 产业链传导，需新闻 + 产业链追踪
    // 2026-09-19: 删除 `trace_industry_chain` —— chat 名字空间无此名（全仓仅本表引用）。
    //   语义相近的现有工具是 `compute_industry_position`（产业链位置），但**是否等价
    //   属产品判断**，未擅自替换；详见 AUDIT-codebase-review-roadmap-2026-09-19.md 待裁决 ④。
    (
        "industry-chain-analyzer",
        &["get_stock_news", "get_cls_flash", "get_stock_concept_blocks", "search_stock"],
    ),
    // screenshot-diagnoser: 持仓截图诊断，需基础分析工具
    (
        "screenshot-diagnoser",
        &["compute_portfolio_risk", "get_stock_quote", "get_stock_peers", "search_stock"],
    ),
    // ── 事件驱动模板：仓位规划与止损复查 ──
    // position-planner: 仓位规划，需基础行情 + 资金分配
    // 2026-09-19: 删除 `get_account_info` / `get_stock_risk_metrics` ——
    //   chat 名字空间（`crates/astock-data/src/mcp_tools.rs` 工具名表）中**不存在这两类工具**：
    //   该表有 `compute_portfolio_risk`，但那是**组合**级风险，与「个股风险指标」不等价；
    //   账户权益类工具全仓不存在。⇒ 原两项是被静默丢弃的白声明（`get_chat_tools_by_names`
    //   按名过滤、无告警），而前端 `ExpertSelector` 会照显 ⇒ 删的是「展示误导」，不是能力
    //   （功能侧本就解析不到，属零行为变更）。是否新补「个股风险指标 / 账户权益」工具
    //   属**新增能力**，待产品决策（见 AUDIT-codebase-review-roadmap-2026-09-19.md 待裁决 ④）。
    ("position-planner", &["get_stock_quote"]),
    // stop-loss-reviewer: 止损复查，需基础行情 + 退出信号
    // 2026-09-19: 删除 `get_stock_risk_metrics` / `compute_volatility`（chat 名字空间均无），
    //   并按「不新增工具、改用现存工具」补上 `check_exit_signals`：
    //   原声明里的 `compute_volatility` 想表达的是「止损该不该触发 / 波动是否异常」，
    //   而 `crates/astock-data/src/mcp_tools.rs` **已有语义更贴的现成工具** ——
    //   `check_exit_signals` 的描述即「检查个股退出信号…返回 `overall_exit_urgency`」，
    //   入参含 `entry_price` / `stop_loss_price`（**专为止损触发设计**），输出契约
    //   （technology_disruption / capacity_oversupply / new_entrant / demand_slowdown
    //   / overall_exit_urgency）还对齐了 mapper_prompt 的 `exit_signals` 字段。
    //   未同时接 `get_stock_kline`（波动率原料）：本 profile 职责是**复核结论**，
    //   K 线原料已由上游 trader 经 `kline_json` 传递，再接一层属重复取数。
    // ⚠ 生效面（勿误读）：本表只喂 **chat 侧**的工具挂载 —— `seed_agent_profiles`
    //   写入 `agent_profiles.recommended_tools`，再由 chat 侧 `local_tool.rs` 的
    //   `get_chat_tools_by_names` 追加给 LLM。工作流侧工具**一律**取
    //   `an.config.tools`（`agent_executor.rs` 的 `tool_defs_to_chat_tools(&an.config.tools)`），
    //   而 `auto-stop-loss-review` 模板的 AgentNode 是**显式 `tools: vec![]`** ⇒ 那个
    //   节点仍是纯推理节点（输入全靠 `context_sources` + `input_mapping` 注入），
    //   **本行改动不会让它多出工具**。`agent_profile_id` 在工作流侧只用于
    //   provider/model 解析与 role/prompt 拼接。
    ("stop-loss-reviewer", &["get_stock_quote", "check_exit_signals"]),
    // explainer: 决策解释官 —— 显式声明**无工具**。
    // 其输入（portfolio-risk-gate 的裁决结果）全部经 input_mapping 注入，职责是翻译
    // 而非取数；`&[]`（而非缺省）表示「已评估并确定为无」，与「未配置」区分开。
    ("explainer", &[]),
    // ── 补齐历史上「有专家、无工具行」的 5 个（recommended_tools 此前落 NULL）──
    // 二者是收敛节点：输入全部来自上游辩论 / 风险评估节点的 context_sources 注入，
    // 自身不取数 —— 显式登记为空。
    ("debate-convergence", &[]),
    ("risk-convergence", &[]),
    // 投资复盘官：输入来自历史分析与反思记录，不取实时数据。
    ("reflection", &[]),
    // ⚠️ 以下两个**当前未接入任何模板**（仅存在于专家库，`agent_profile_id` 无引用）。
    // 显式登记为空只是为了消除 NULL（「未配置」与「确认为空」在 DB 里无法区分）；
    // 若将来把它们接进模板，**必须先按其数据源确定工具白名单**，不要沿用本行。
    ("social-media-analyst", &[]),
    ("volume-price-analyst", &[]),
];

pub async fn ensure_stock_analysis_experts_seeded(
    db: &sea_orm::DatabaseConnection,
) -> Result<(), String> {
    // 0) 存量自愈（P0，2026-09-14）：端口公理非法的存量模板归零 `version`，
    //    使下面各 stock 种子函数的 `existing.version >= TEMPLATE_VERSION` 门放行重建。
    //    与 OPC 侧同一个函数、同一份判据（`port_axiom_errors`）；失败不阻断种子。
    match axagent_dao::repo::workflow_template::reset_port_axiom_illegal_versions(db).await {
        Ok(ids) if !ids.is_empty() => {
            tracing::warn!(
                "[stock_analysis_setup] 端口公理非法的存量模板已归零版本号，等待重建: {ids:?}"
            );
        },
        Ok(_) => {},
        Err(e) => {
            tracing::warn!("[stock_analysis_setup] 存量端口公理自愈检查失败（不阻断种子）: {e}");
        },
    }

    // 先执行 Serenity 种子，独立 try 避免被前序步骤阻塞
    tracing::info!("[stock_analysis_setup] === 开始种子 Serenity 模板 ===");
    if let Err(e) = seed_serenity_screening_workflow_template(db).await {
        tracing::error!("[stock_analysis_setup] Serenity 模板种子失败 (非致命): {e}");
    }
    tracing::info!("[stock_analysis_setup] === Serenity 模板种子完成 ===");

    // 快速趋势智选：与原链并存，独立 try（不依赖原链的行，失败互不影响）
    if let Err(e) = seed_serenity_fast_workflow_template(db).await {
        tracing::error!("[stock_analysis_setup] 快速趋势智选模板种子失败 (非致命): {e}");
    }

    seed_agency_experts(db).await?;
    seed_agent_roles(db).await?;
    seed_stock_agent_roles(db).await?;
    seed_agent_profiles(db).await?;

    // 股票分析核心工作流模板 — 失败不阻塞主流程（独立 try）
    // 原因：如果前置专家种子化失败，? 操作符会直接 return，
    // 导致工作流模板永远不会被种子化，编辑器打开时无内容显示
    tracing::info!("[stock_analysis_setup] === 开始种子股票分析工作流模板 ===");
    if let Err(e) = seed_stock_analysis_workflow_template(db).await {
        tracing::error!("[stock_analysis_setup] 股票分析工作流模板种子失败 (非致命): {e}");
    }
    // 种子化会改 `stock-analysis` 的变量表（`merge_variable_values` 保旧值、
    // `force_variable_value` 覆写、一次性迁移门如 `kline_limit`），而落点读的是
    // `init::panel_variables` 的进程内快照 ⇒ 重建后必须重读那一行，
    // 否则「升版后第一次跑」用的还是上一版的参数（快照是启动时装的）。
    crate::init::panel_variables::refresh_from_db(db).await;
    tracing::info!("[stock_analysis_setup] === 股票分析工作流模板种子完成 ===");

    // 快速链模板必须紧随原链种子（它从 stock-analysis 行派生，源行不存在则直接失败）
    if let Err(e) = seed_stock_analysis_fast_workflow_template(db).await {
        tracing::error!("[stock_analysis_setup] 快速链模板种子失败 (非致命): {e}");
    }

    // 四张档子模板（B-2b #36 = v135）：主图那四个 `pm-h-<档>` 扇出节点在**运行期**按
    // `sub_workflow_id` 从 workflow_templates 取它们 ⇒ 这四行缺一条，那一档就是
    // `Template <id> not found` 的显式子执行失败（不是「该档算不出来」的静默缺席）。
    // 失败仍按非致命处理（与主图同口径），但要打得响 —— 四档分支不能少一条还看不出来。
    if let Err(e) = horizon_tier_template::seed_horizon_tier_templates(db).await {
        tracing::error!("[stock_analysis_setup] 四张档子模板种子失败 (非致命): {e}");
    }

    if let Err(e) = seed_reflection_workflow_template(db).await {
        tracing::error!("[stock_analysis_setup] 反思工作流模板种子失败 (非致命): {e}");
    }
    // seed_debate_subworkflow(db).await?;  // 辩论子工作流未引用，暂不种子化

    // P2-2: 决策事件总线订阅方模板 — 失败不阻塞主流程（独立 try）
    // 两个模板都订阅 "decision.completed" 事件，由 stock_workflow/core.rs 的
    // publish_event 自动触发，实现决策→仓位规划/止损复查的联动编排。
    tracing::info!("[stock_analysis_setup] === 开始种子决策事件订阅模板 ===");
    if let Err(e) = seed_auto_position_plan_template(db).await {
        tracing::error!("[stock_analysis_setup] auto-position-plan 模板种子失败 (非致命): {e}");
    }
    if let Err(e) = seed_auto_stop_loss_review_template(db).await {
        tracing::error!("[stock_analysis_setup] auto-stop-loss-review 模板种子失败 (非致命): {e}");
    }
    tracing::info!("[stock_analysis_setup] === 决策事件订阅模板种子完成 ===");

    // G4: daily-market-events 每日市场主线提炼模板 — 失败不阻塞主流程
    tracing::info!("[stock_analysis_setup] === 开始种子 G4 市场主线模板 ===");
    if let Err(e) = seed_daily_market_events_template(db).await {
        tracing::error!("[stock_analysis_setup] daily-market-events 模板种子失败 (非致命): {e}");
    }
    tracing::info!("[stock_analysis_setup] === G4 市场主线模板种子完成 ===");

    // G6: screenshot-portfolio-diagnosis 截图持仓诊断模板 — 失败不阻塞主流程
    tracing::info!("[stock_analysis_setup] === 开始种子 G6 截图诊断模板 ===");
    if let Err(e) = seed_screenshot_portfolio_diagnosis_template(db).await {
        tracing::error!(
            "[stock_analysis_setup] screenshot-portfolio-diagnosis 模板种子失败 (非致命): {e}"
        );
    }
    tracing::info!("[stock_analysis_setup] === G6 截图诊断模板种子完成 ===");

    // G3.3: news-to-cross-market-analysis 新闻→跨市场传导分析模板 — 失败不阻塞主流程
    tracing::info!("[stock_analysis_setup] === 开始种子 G3.3 跨市场传导分析模板 ===");
    if let Err(e) = seed_news_cross_market::seed_news_cross_market_template(db).await {
        tracing::error!(
            "[stock_analysis_setup] news-to-cross-market-analysis 模板种子失败 (非致命): {e}"
        );
    }
    tracing::info!("[stock_analysis_setup] === G3.3 跨市场传导分析模板种子完成 ===");

    // stock-pipeline: 股票全业务管道模板 — 失败不阻塞主流程
    tracing::info!("[stock_analysis_setup] === 开始种子 stock-pipeline 模板 ===");
    if let Err(e) = crate::commands::stock_pipeline::seed_stock_pipeline_template(db).await {
        tracing::error!("[stock_analysis_setup] stock-pipeline 模板种子失败 (非致命): {e}");
    }
    tracing::info!("[stock_analysis_setup] === stock-pipeline 模板种子完成 ===");
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
async fn seed_agency_experts(db: &sea_orm::DatabaseConnection) -> Result<(), String> {
    use axagent_entities::agency_experts;
    use sea_orm::{ActiveModelTrait, EntityTrait, NotSet, Set};

    let mut count = 0u32;
    for &(expert_id, content) in EMBEDDED_PROMPTS {
        let (name, desc, body, color) = parse_expert_md(content, expert_id);
        let agency_id = format!("agency-stock-analysis-{expert_id}");
        let now = chrono::Utc::now().timestamp();
        let active = agency_experts::ActiveModel {
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
            active_domains: Set(None),
            seniority: NotSet,
            specialties: NotSet,
            parent_role_id: NotSet,
            success_rate: NotSet,
            avg_latency_ms: NotSet,
            avg_token_cost: NotSet,
        };
        // v24: 改为 UPSERT — 已存在则 update，确保 .md 改动和新增的 R3 专家能同步到 DB
        // 历史版本: 已存在则 continue 跳过,导致 .md 改动 / 新增 .md 文件 (bull-r3/bear-r3) 不写库,
        // 前端看到的是旧版 prompt,输出与代码不同步。
        if agency_experts::Entity::find_by_id(&agency_id)
            .one(db)
            .await
            .map_err(|e| {
                String::from(crate::commands::error::ErrorResponse::from_error(
                    e,
                    crate::commands::error::ErrorCategory::Unrecoverable,
                ))
            })?
            .is_some()
        {
            active.update(db).await.map_err(|e| {
                String::from(crate::commands::error::ErrorResponse::from_error(
                    e,
                    crate::commands::error::ErrorCategory::Unrecoverable,
                ))
            })?;
        } else {
            active.insert(db).await.map_err(|e| {
                String::from(crate::commands::error::ErrorResponse::from_error(
                    e,
                    crate::commands::error::ErrorCategory::Unrecoverable,
                ))
            })?;
        }
        count += 1;
    }

    // 清理已注销专家的残留行（UPSERT 只增改不删 —— 见 RETIRED_EXPERT_IDS 文档）
    for retired in RETIRED_EXPERT_IDS {
        let agency_id = format!("agency-stock-analysis-{retired}");
        if let Err(e) = agency_experts::Entity::delete_by_id(&agency_id).exec(db).await {
            tracing::warn!("[stock_analysis_setup] 清理已注销专家 {agency_id} 失败 (非致命): {e}");
        }
    }

    tracing::info!("[stock_analysis_setup] 已种子化/更新 {count} 个 agency_experts");
    Ok(())
}

async fn seed_agent_roles(db: &sea_orm::DatabaseConnection) -> Result<(), String> {
    let mut count = 0u32;
    // v24: 去掉"已存在则跳过"短路 — 无条件调 upsert_agent_role,确保 STOCK_ROLES 改动
    // (尤其是新增的 role) 能同步到 DB。
    for role in STOCK_ROLES {
        repo::agent_role::upsert_agent_role(
            db,
            role.id,
            role.name,
            Some(role.description),
            role.system_prompt,
            &[],
            &[],
            role.max_concurrent,
            role.timeout_seconds,
            "stock-analysis",
        )
        .await
        .map_err(|e| {
            String::from(crate::commands::error::ErrorResponse::from_error(
                e,
                crate::commands::error::ErrorCategory::Unrecoverable,
            ))
        })?;
        count += 1;
    }
    tracing::info!("[stock_analysis_setup] 已种子化/更新 {count} 个 agent_roles");
    Ok(())
}

/// 种子化 AxInvest 专属角色 `stock-investment-lead`（证券投资负责人）
/// 及其 6 个下属子岗位（投研/数据/会议/策略/交易/风控 负责人）。
///
/// 顶层 leader 的 system_prompt 作为最外层身份提示词，通过上游 agent_executor 4 层
/// prompt 拼接（AgentRole → Expert → 节点 inline）注入到所有
/// 股票专家 AgentProfile 的运行时上下文中。详见 STOCK_AGENT_ROLE 注释。
///
/// 6 个子岗位对应 INVESTMENT_OFFICE_TEMPLATE 中的 6 个房间，作为 AddMemberModal
/// 的角色下拉候选项，让投研办公室成员添加时可按房间选角色。
async fn seed_stock_agent_roles(db: &sea_orm::DatabaseConnection) -> Result<(), String> {
    // 顶层 leader
    upsert_stock_agent_role(db, &STOCK_AGENT_ROLE, None, 100).await?;
    // 6 个子岗位（投研办公室房间负责人），全部 reports_to = leader
    let mut count = 1u32;
    for sub in STOCK_AGENT_SUB_ROLES {
        upsert_stock_agent_role(db, sub, Some(STOCK_AGENT_ROLE_ID), 200 + count as i32).await?;
        count += 1;
    }
    tracing::info!("[stock_analysis_setup] 已种子化/更新 {} 个角色（1 leader + 6 子岗位）", count);
    Ok(())
}

/// 单个 StockAgentRoleDef 的 upsert 包装，避免重复样板代码。
async fn upsert_stock_agent_role(
    db: &sea_orm::DatabaseConnection,
    r: &StockAgentRoleDef,
    reports_to: Option<&str>,
    sort_order: i32,
) -> Result<(), String> {
    let responsibilities: Vec<String> = r.responsibilities.iter().map(|s| s.to_string()).collect();
    let certifications: Vec<String> =
        r.required_certifications.iter().map(|s| s.to_string()).collect();
    let domains: Vec<String> = r.active_domains.iter().map(|s| s.to_string()).collect();
    // managed_expert_ids 留空——股票专家众多且会动态增减，由前端按 source_dir="stock-analysis" 聚合
    repo::agent_role::upsert_agent_role_ext(
        db,
        r.id,
        r.name,
        Some(r.description),
        r.system_prompt,
        &[],
        &domains,
        3,
        600,
        "stock-analysis",
        Some(&serde_json::to_string(&responsibilities).unwrap_or_default()),
        Some(r.decision_authority),
        reports_to,
        None,
        Some(&serde_json::to_string(&certifications).unwrap_or_default()),
        Some(r.icon),
        Some(r.color),
        true,
        sort_order,
    )
    .await
    .map_err(|e| {
        ErrorResponse::new(stock_setup::INTERNAL).with_detail(format!("种子角色失败: {e}"))
    })?;
    tracing::info!("[stock_analysis_setup] 已种子化/更新角色岗位 {} ({})", r.id, r.name);
    Ok(())
}

async fn seed_agent_profiles(db: &sea_orm::DatabaseConnection) -> Result<(), String> {
    use axagent_entities::agent_profiles;
    use sea_orm::{ActiveModelTrait, EntityTrait, Set};

    // Profile → 工具映射（从模块级 PROFILE_TOOLS 构建）
    let profile_tools: std::collections::HashMap<&str, &[&str]> =
        PROFILE_TOOLS.iter().cloned().collect();

    let mut count = 0u32;
    for &(expert_id, role_id) in EXPERT_ROLE_MAP {
        let profile_id = format!("stock-{expert_id}");

        let tools_json = profile_tools
            .get(expert_id)
            .map(|tools| serde_json::to_string(tools).unwrap_or_default());
        let now = chrono::Utc::now().timestamp_millis();
        let active = agent_profiles::ActiveModel {
            id: Set(profile_id.clone()),
            name: Set(format!("📈 {}", expert_id_to_display(expert_id))),
            description: Set(Some(format!("股票分析专家 — {}", role_id_to_display(role_id)))),
            category: Set("stock-analysis".into()),
            icon: Set("📈".into()),
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
            // v218: 岗位即角色——agent_role 指向证券投资负责人（已并入 agent_roles），
            // 其 system_prompt 作为最外层身份注入；stock-analyst 等执行器标签原未入表，不再使用。
            agent_role: Set(Some(STOCK_AGENT_ROLE_ID.into())),
            created_at: Set(now),
            updated_at: Set(now),
        };
        // v24: 改为 UPSERT — 已存在则 update,确保 PROFILE_TOOLS 改动和新增 expert (bull-r3/bear-r3) 同步到 DB
        if agent_profiles::Entity::find_by_id(&profile_id)
            .one(db)
            .await
            .map_err(|e| {
                ErrorResponse::new(stock_setup::INTERNAL)
                    .with_detail(format!("查询 profile 失败: {e}"))
            })?
            .is_some()
        {
            active.update(db).await.map_err(|e| {
                ErrorResponse::new(stock_setup::INTERNAL)
                    .with_detail(format!("更新 profile 失败: {e}"))
            })?;
        } else {
            active.insert(db).await.map_err(|e| {
                ErrorResponse::new(stock_setup::INTERNAL)
                    .with_detail(format!("插入 profile 失败: {e}"))
            })?;
        }
        count += 1;
    }

    // 清理已注销专家的 profile 残留行（同样只增改不删）
    for retired in RETIRED_EXPERT_IDS {
        let profile_id = format!("stock-{retired}");
        if let Err(e) = agent_profiles::Entity::delete_by_id(&profile_id).exec(db).await {
            tracing::warn!(
                "[stock_analysis_setup] 清理已注销 profile {profile_id} 失败 (非致命): {e}"
            );
        }
    }

    tracing::info!("[stock_analysis_setup] 已种子化/更新 {count} 个 agent_profiles");
    Ok(())
}

pub(crate) fn parse_expert_md(
    content: &str,
    fallback: &str,
) -> (String, String, String, Option<String>) {
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

pub(crate) fn expert_id_to_display(id: &str) -> String {
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
        // 决策解释官（decision-explainer 节点的 profile 显示名）
        "explainer" => "决策解释官".to_string(),
        o => o.to_string(),
    }
}

pub(crate) fn role_id_to_display(id: &str) -> String {
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

/// 构建分析师 input_mapping：为**每个逐档实例**注入 bull_score/bear_score
/// 例如 a-market-analyst--mid → 【a-market-analyst--mid_bull_score】:75
///
/// 路径规则（V29 + v133 订正）：AgentNode 输出包裹在 {role, content: <json_string>, ...} 中，
/// resolve_var_path 遇到 Value::String 会自动 from_str 解析后再继续下钻 —— **但 V62 起
/// content 统一为 `{report, verdict: {...}}` 嵌套**，业务字段都在 `verdict` 层 ⇒ 原写法
/// `{id}.content.bull_score` 自 V62 起恒解析为 None（30 键全部静默失效、prompt 里
/// 一行【…】都注不出来）。v133 随按档重写一并订正为 `{id}.content.verdict.bull_score`。
/// 同时删除 `_consensus` 键：verdict 里**没有** consensus_score 字段（共识= bull−bear 由
/// 下游自算），该键从来无来源 —— Null 键与缺失键在 4g 注入上等价（None 一律跳过），
/// 留着只会让人以为有一路在供数。
///
/// v133（B2-2）：参数从「10 个 base id」改为**逐档实例清单**（`tiered`，23 条）——
/// 消费方（debate-convergence / 三个风险偏好节点）是全局面板/裁决层，需要同时看到
/// **全部档位实例**的评分（各档独立结论才是四档分支的输入；只给一个档会退化）。
/// 键前缀带上档位后缀（`a-market-analyst--mid`，与节点 id 同形 ⇒ LLM 看到的行
/// 与图里节点名可逐字对上）。
/// `value-investor` 的产出被 `value-verify` **原地覆写**（output_var 同名，v91 起的机制）
/// ⇒ 其权威形态是 CodeNode 的 `{status, result: {report, verdict, ...}}`（`.result` 段），
/// 与其他分析师的 `.content` 段不同 —— 路径按 base 分流（写错就是整段静默 Null）。
pub(crate) fn build_analyst_input_mapping(
    tiered: &[(&str, axagent_harness::holding_period::Period, &str, &str)],
) -> std::collections::HashMap<String, String> {
    use std::collections::HashMap;
    let mut map = HashMap::new();
    for (base, p, ..) in tiered {
        let node_id = axagent_harness::holding_period::analyst_node_id(base, p.as_str());
        let root = if *base == "value-investor" {
            format!("{node_id}.result")
        } else {
            format!("{node_id}.content")
        };
        map.insert(format!("{node_id}_bull_score"), format!("{root}.verdict.bull_score"));
        map.insert(format!("{node_id}_bear_score"), format!("{root}.verdict.bear_score"));
    }
    // 为所有辩论/风险节点注入历史反思教训
    map.insert("stock_lessons".into(), "stock_lessons".into());
    map
}

/// 合并新模板变量与旧模板变量的值。
/// 对于同名的变量，保留旧变量的 value（用户的修改），字段定义以新模板为准。
pub(crate) fn merge_variable_values(
    new_variables_json: &str,
    old_variables_json: &str,
) -> Result<String, String> {
    let new_vars: Vec<serde_json::Value> =
        serde_json::from_str(new_variables_json).map_err(|e| {
            ErrorResponse::new(stock_setup::INTERNAL).with_detail(format!("解析新变量失败: {e}"))
        })?;
    let old_vars: Vec<serde_json::Value> =
        serde_json::from_str(old_variables_json).map_err(|e| {
            ErrorResponse::new(stock_setup::INTERNAL).with_detail(format!("解析旧变量失败: {e}"))
        })?;

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
        ("analysis_klineLimit", "kline_limit"),
        ("analysis_newsLimit", "news_limit"),
        // Agent / Tool
        ("analysis_temperature", "agent_temperature"),
        ("analysis_maxTokens", "agent_max_tokens"),
        ("analysis_timeoutSecs", "agent_timeout_secs"),
        ("tool_timeoutSecs", "tool_timeout_secs"),
        ("tool_retryMax", "tool_retry_max"),
        // 规则
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
        ("monitor_alertCooldownSecs", "monitor_alert_cooldown_secs"),
        // 跨股票聚合器（P2 配置入口）
        ("aggregator_windowSecs", "aggregator_window_secs"),
        ("aggregator_minSignalCount", "aggregator_min_signal_count"),
        ("aggregator_cooldownSecs", "aggregator_cooldown_secs"),
        ("aggregator_minStrength", "aggregator_min_strength"),
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

    serde_json::to_string(&merged).map_err(|e| {
        ErrorResponse::new(stock_setup::INTERNAL)
            .with_detail(format!("序列化合变量失败: {e}"))
            .to_string()
    })
}

/// 强制覆写模板变量表中的某个变量值（**无视** `merge_variable_values` 的「旧值优先」）。
///
/// ## 什么时候需要它
///
/// `merge_variable_values` 的语义是「新定义 + 旧值」，即**无条件保留用户旧值** ——
/// 这对「用户自己调过的参数」是正确的，但对**语义发生变化的变量**是错的：
/// DB 里那个旧值本身就是本次要修掉的东西，不覆写就等于没改。
///
/// 首个用例（v48，2026-09-14）：`debate_rounds` 由 3 固定为 1。只改
/// `seed_variables.rs` 的默认值无效，因为 DB 里的 3 会在升级时覆盖回来。
///
/// ## 失败策略
///
/// 解析失败 / 变量不存在时**原样返回**并 warn，不 panic、不阻断种子：
/// 变量值错顶多让下游行为退化，而阻断会让整个模板卡在旧版本（代价更大）。
pub(crate) fn force_variable_value(
    variables_json: &str,
    name: &str,
    value: serde_json::Value,
) -> String {
    let Ok(mut vars) = serde_json::from_str::<Vec<serde_json::Value>>(variables_json) else {
        tracing::warn!("force_variable_value: 变量表解析失败，保持原样（name={name}）");
        return variables_json.to_string();
    };
    let mut hit = false;
    for v in vars.iter_mut() {
        if v.get("name").and_then(|n| n.as_str()) == Some(name) {
            v["value"] = value.clone();
            hit = true;
        }
    }
    if !hit {
        tracing::warn!("force_variable_value: 变量 '{name}' 不在变量表中，未覆写");
    }
    serde_json::to_string(&vars).unwrap_or_else(|_| variables_json.to_string())
}

/// 解析「辩论轮数」的当前生效值，作为建图展开节点数与下游锚点的唯一来源。
///
/// 从旧变量表（DB 存量，经 `merge_variable_values`、RENAME_MAP 保留用户自定义）
/// 读取 `debate_rounds`；未命中则用 `seed_variables::DEFAULT_DEBATE_ROUNDS` 默认。
///
/// ## 为什么需要它（不要直接写死 1）
///
/// 该值决定 DAG 展开成几对 `bull-rN`/`bear-rN` 辩手节点、以及下游锚点
/// `bear-r{name}` 指向第几轮。若建图轮数与落库的 `debate_rounds` 变量值不一致，
/// 轻则多跑/少跑轮次，重则产生悬空入边（曾致 `create_workflow` 启动期拒绝，
/// 见 `debater_round_refs_are_parameterized` 负控）。统一由本函数求值，
/// 保证「建图、下游锚点、变量表」三处同源。
pub(crate) fn resolve_debate_rounds(old_variables_json: Option<&str>) -> usize {
    use crate::commands::stock_analysis_setup::seed_variables::DEFAULT_DEBATE_ROUNDS;

    if let Some(json) = old_variables_json.filter(|v| !v.is_empty()) {
        if let Ok(vars) = serde_json::from_str::<Vec<serde_json::Value>>(json) {
            for v in vars {
                if v.get("name").and_then(|n| n.as_str()) == Some("debate_rounds") {
                    if let Some(n) = v.get("value").and_then(|val| val.as_u64()) {
                        // 防御性夹紧：上层解析为 usize，避免极端值把 DAG 撑爆
                        // （u64::clamp 不会越界；区间 [1, 10] 恒合法，不会 panic）
                        return n.clamp(1, 10) as usize;
                    }
                }
            }
        }
        tracing::warn!(
            "[stock_analysis_setup] 旧变量表中未找到 debate_rounds 数值，回退默认 {} 轮",
            DEFAULT_DEBATE_ROUNDS
        );
    }
    DEFAULT_DEBATE_ROUNDS as usize
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
    use axagent_entities::workflow_template;
    use axagent_harness::workflow_types::{
        AgentNode, AgentNodeConfig, CodeNode, CodeNodeConfig, EdgeType, OutputMode, Position,
        RetryConfig, StorageNode, StorageNodeConfig, ToolDef, TriggerConfig, TriggerNode,
        TriggerType, Variable, WorkflowEdge, WorkflowNode, WorkflowNodeBase,
    };
    use sea_orm::{ActiveModelTrait, EntityTrait, Set};

    let now = chrono::Utc::now().timestamp_millis();

    // ── 反思 Agent 可用工具定义（仅 K 线 + 公告全文，不暴露交易类工具）──
    let refl_tools: Vec<ToolDef> = {
        let mut kline_props = std::collections::HashMap::new();
        kline_props.insert(
            "stock_code".into(),
            axagent_harness::workflow_types::JsonSchemaProperty {
                schema_type: "string".into(),
                description: Some("6位股票代码".into()),
                default: None,
                enum_values: None,
                format: None,
            },
        );
        kline_props.insert(
            "period".into(),
            axagent_harness::workflow_types::JsonSchemaProperty {
                schema_type: "string".into(),
                description: Some("K线周期: daily(日线)/weekly(周线)/monthly(月线)".into()),
                default: Some(serde_json::json!("daily")),
                enum_values: None,
                format: None,
            },
        );
        kline_props.insert(
            "limit".into(),
            axagent_harness::workflow_types::JsonSchemaProperty {
                schema_type: "integer".into(),
                description: Some("K线数量".into()),
                default: Some(serde_json::json!(120)),
                enum_values: None,
                format: None,
            },
        );
        let td_kline = ToolDef {
            name: "get_stock_kline".into(),
            description: Some("获取K线数据：OHLCV，可指定周期和数量，用于事后对比走势".into()),
            parameters: Some(axagent_harness::workflow_types::JsonSchema {
                schema_type: "object".into(),
                description: None,
                properties: Some(kline_props),
                required: Some(vec!["stock_code".into()]),
                items: None,
            }),
        };
        // P0 修复(2026-07-22): 移除未实现的 get_announcement_content 工具引用
        // 该工具在 mcp_tools.rs 中未注册 dispatch，调用会触发 "Unknown MCP tool" 错误
        vec![td_kline]
    };

    // ── CodeNode: 定量对比脚本（sub-analysis → reflection-comparator → reflection-agent）──
    let comparator_code = include_str!("../reflection-comparator.rhai").to_string();
    let comparator_node = WorkflowNode::Code(CodeNode {
        base: WorkflowNodeBase {
            id: "reflection-comparator".into(),
            title: "预测vs实际定量对比".into(),
            description: Some("对比分析师预测与实际走势，输出结构化偏差报告".into()),
            position: Position { x: 20.0, y: 260.0 },
            retry: RetryConfig::default(),
            timeout: Some(10),
            enabled: true,
            parent_id: None,
            compensation: None,
            continue_on_fail: true, // 对比失败不阻塞反思
        },
        config: CodeNodeConfig {
            language: "rhai".into(),
            code: comparator_code,
            output_var: "reflection-comparator".into(),
            tool_name: None,
            execute_directly: true,
            input_mapping: [
                ("trader_action", "sub-analysis.trader.content.action"),
                ("trader_target_price", "sub-analysis.trader.content.targetPrice"),
                ("trader_confidence", "sub-analysis.trader.content.confidence"),
                ("portfolio_action", "sub-analysis.portfolio-mgr.action"),
                ("portfolio_posterior", "sub-analysis.portfolio-mgr.posterior"),
                ("debate_consensus", "sub-analysis.debate-convergence.content.consensus_score"),
                ("total_score", "sub-analysis.t-scoring.result.totalScore"),
                ("raw_return_pct", "raw_return_pct"),
                ("alpha_return_pct", "alpha_return_pct"),
                ("holding_days", "holding_days"),
                ("original_time_horizon", "original_time_horizon"),
                ("original_holding_days", "original_holding_days"),
                // [实际行情 2026-09-13] 价格层事实。来源 = 后端 compute_market_snapshots()
                // 用前复权 K 线确定性算出的 MarketSnapshot，经 actual_market_json 变量注入。
                // 在此之前本 comparator 只消费标量 raw_return_pct，价格/回撤/目标价
                // 全部不可见；而 trader_target_price 虽在映射表里却从未被脚本消费（死映射）。
                ("latest_price", "actual_market_json.latestPrice"),
                ("entry_price", "actual_market_json.entryPrice"),
                ("target_price", "actual_market_json.targetPrice"),
                ("target_progress_pct", "actual_market_json.targetProgressPct"),
                ("max_drawdown_pct", "actual_market_json.maxDrawdownPct"),
                ("period_high", "actual_market_json.periodHigh"),
                // ── 四周期行情事实 + 周期决策（批次 3）──
                // comparator 通过 horizon_correct 逐周期做确定性判定，
                // 写入 reflection-comparator.horizon_was_correct 供下游聚合。
                ("horizon_market_facts", "horizon_market_facts"),
                ("horizon_decisions_json", "horizon_decisions_json"),
                ("period_low", "actual_market_json.periodLow"),
                ("latest_date", "actual_market_json.latestDate"),
                ("within_expected_horizon", "actual_market_json.withinExpectedHorizon"),
                // __untrusted 标记（从子工作流各 Agent 节点提取）
                ("u_trader", "sub-analysis.trader.__untrusted"),
                ("u_research_mgr", "sub-analysis.research-mgr.__untrusted"),
                ("u_catalyst", "sub-analysis.a-catalyst.__untrusted"),
                ("u_debate_cnv", "sub-analysis.debate-convergence.__untrusted"),
                ("u_data_quality", "sub-analysis.data-quality.__untrusted"),
                ("u_risk_cnv", "sub-analysis.risk-convergence.__untrusted"),
            ]
            .into_iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect(),
        },
    });

    // ── CodeNode: 反思输出硬裁决验证层（reflection-agent → reflection-validator → store-ref）──
    // [P1-#1 修复] 原 reflection_validator.rhai 是死代码，DAG 未引用。
    // 现接入为 DAG 节点，在 reflection-agent 之后、store-ref 之前执行。
    // 验证 7 字段类型/枚举值/长度，自动修正 verdict 枚举、截断 lesson_summary、
    // 补全 missed_signals 数组等（R-302/R-303/R-304/R-305 硬裁决规则）。
    let validator_code = include_str!("../reflection_validator.rhai").to_string();
    let validator_node = WorkflowNode::Code(CodeNode {
        base: WorkflowNodeBase {
            id: "reflection-validator".into(),
            title: "反思输出硬裁决验证".into(),
            description: Some("验证 reflection-agent 输出的字段类型/枚举值/长度，自动修正".into()),
            position: Position { x: 20.0, y: 460.0 },
            retry: RetryConfig::default(),
            timeout: Some(5),
            enabled: true,
            parent_id: None,
            compensation: None,
            continue_on_fail: true, // 验证失败不阻塞落盘
        },
        config: CodeNodeConfig {
            language: "rhai".into(),
            code: validator_code,
            output_var: "reflection-validated".into(),
            tool_name: None,
            execute_directly: true,
            input_mapping: [("reflection_input", "reflection")]
                .into_iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
        },
    });

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
                continue_on_fail: false,
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
        // 2. 定量对比 CodeNode + 3. 反思复盘 Agent + 4. 硬裁决验证
        //    注: comparator_node / validator_node 在 nodes vec 外部构造(见前文),这里追加到 vec 末尾
        //    [v2] 删除 sub-analysis SubWorkflowNode — 不再重跑完整 stock-analysis DAG，
        //    改由 run_reflection_workflow 从 stock_analyses.blackboard_snapshot 加载记忆，
        //    构造名为 "sub-analysis" 的变量注入工作流（context_sources / input_mapping 路径不变）。
        comparator_node,
        // V68 修复(2026-09-10): reflection-agent 提示词中原引用 get_announcement_content，
        // 该工具未实现且不在白名单，LLM 调用必报 Unknown MCP tool 浪费反思轮次
        // （同 P0 2026-07-22 对其他提示词的同类清理，此处为漏网点）。
        WorkflowNode::Agent(AgentNode {
            base: WorkflowNodeBase {
                id: "reflection-agent".into(),
                title: "反思复盘".into(),
                description: Some("基于实际走势+偏差报告+数据工具做反思复盘".into()),
                position: Position { x: 20.0, y: 380.0 },
                retry: RetryConfig { enabled: true, max_retries: 2, ..Default::default() },
                timeout: Some(600),
                enabled: true,
                parent_id: None,
                compensation: None,
                continue_on_fail: false,
            },
            config: AgentNodeConfig {
                system_prompt: "你的任务：对历史股票分析进行反思复盘。\n\
                    目标股票代码: {{stock_code}}，股票名称: {{stock_name}}\n\
                    ——本次复盘唯一针对的周期档——\n\
                    复盘档: {{review_horizon}}（该档期望持有 {{review_expected_holding_days}} 个交易日）\n\
                    上面「实际走势结果」与所有硬数字（收益率/超额/持有天数/价格事实）\n\
                    都是**这一档窗口**的口径，与其余三档无关。\n\
                    实际走势结果: {{actual_outcome}}（非空 → 反思模式）\n\
                    ——结构化 outcome 变量（v008 C3 借鉴:硬数字,避免 LLM 脑补）——\n\
                    原始收益率: {{raw_return_pct}}%\n\
                    相对基准超额: {{alpha_return_pct}}%\n\
                    实际持有天数: {{holding_days}} 天\n\
                    基准名称: {{benchmark_name}}\n\
                    ——当前实际行情（价格层事实，由前复权 K 线确定性计算）——\n\
                    {{actual_market_text}}\n\
                    以上是「已完成的分析结论」与「该股票当前实际行情」的客观差异，是你的事实基础。\n\
                    若标记「尚未到期望持有期」，本次属期中观察：不要据此判定策略失效，结论权重放低。\n\
                    若标记「行情数据不可用」，只做定性反思，禁止对涨跌方向/幅度下结论。\n\
                    反思深度: {{reflection_depth}}（light = 简要；deep = 详细推理链）\n\n\
                    ——定量偏差报告（reflection-comparator 输出）——\n\
                    详见下方【输入上下文】的 deviation_report 字段。\n\
                    包含方向匹配度(direction_match)/收益分类(return_category)/时间维度检查。\n\
                    分析前务必先阅读，direction_match=false 说明方向误判需深入分析错因。\n\n\
                    历史反思教训（避免重蹈覆辙）:\n\
                    {{stock_lessons}}\n\n\
                    可用工具：\n\
                    - get_stock_kline: 获取实际操作期间的K线数据，对比预测走势与实际价格运动\n\n\
                    使用工具的原则：\n\
                    1. 先分析 deviation_report 中的定量发现，确认方向是否一致\n\
                    2. 如有必要，调用 get_stock_kline 查看实际K线走势验证\n\
                    3. 工具调用结论应与定量对比报告交叉验证\n\n\
                    重要原则：\n\
                    1. 必须严格基于 actual_outcome 提供的实际走势与上游分析结论做对比，识别错因。\n\
                    2. 结合 deviation_report 的定量发现验证而非替代 LLM 判断。\n\
                    3. 严禁输出空结果或只列 data_gaps。\n\
                    4. 强制简短：lesson_summary 字段必须 ≤200 字符、≤2 句。\n\
                    5. 反思深度=deep 时给出可执行的检查清单（具体指标阈值、信号确认步骤）。\n\
                    6. 用 verdict 字段标记本次反思判定（correct/partial/wrong 三选一）。\n\
                    7. 如果复盘发现本可优化决策，在 alpha_cited 字段说明关键 alpha 信号。\n\
                    8. 不要输出交易决策（买入/卖出/持有），不要输出 confidence/positionPct。\n\
                    9. 只复盘 {{review_horizon}} 这一档：lesson_summary / what_went_wrong /\n\
                       missed_signals / fix_for_future / params_suggestion 全部只能针对该档产出，\n\
                       禁止写「适用于所有周期」的结论。\n\
                    10. 其余三档的逐周期判定（deviation_report.horizon_correct）只作背景参照，\n\
                        **不得**为它们写教训或参数建议；跨周期矛盾只能作为本档结论的风险提示。\n\n\
                    你必须输出严格 JSON 格式（不要 Markdown 代码块，不要多余文本），字段如下：\n\
                    {\n\
                      \"verdict\": \"correct | partial | wrong\",\n\
                      \"alpha_cited\": \"引用本次未被重视但事后证明重要的 alpha 信号\",\n\
                      \"lesson_summary\": \"≤200 字符、≤2 句简短总结\",\n\
                      \"what_went_wrong\": \"哪里判断错了，简要说明\",\n\
                      \"missed_signals\": [\"被忽略的信号1\", \"被忽略的信号2\"],\n\
                      \"fix_for_future\": \"下次如何避免同样的错误\",\n\
                      \"implementation_tier\": \"L1 | L2 | L3\",\n\
                      \"code_diff_proposal\": \"具体修改方案描述（L1简述 / L2-L3含文件路径和代码段）\",\n\
                      \"params_suggestion\": [\n\
                        {\n\
                          \"param\": \"参数名（必须严格取自下方【可调参数清单】的 name，禁止自造名字）\",\n\
                          \"current_value\": \"当前值\",\n\
                          \"suggested_value\": \"建议值\",\n\
                          \"reason\": \"调整原因\"\n\
                        }\n\
                      ]\n\
                    }\n\n\
                    【可调参数清单】（只能建议以下参数；param 字段须写清单中的 name；\n\
                    若本次复盘没有充分证据支持调整某参数，就不要把它写进 params_suggestion）\n\
                    {{tunable_params_catalog}}"
                .into(),
                context_sources: vec!["sub-analysis".into(), "reflection-comparator".into()],
                input_mapping: [
                    // [BUGFIX] source 应为变量名而非节点 ID "trigger"。
                    // 这些变量已在 run_reflection_workflow 的 variables vec 中顶层注入,
                    // 用变量名才能正确从 context.variables 取到 string 值,
                    // 否则 map_inputs 会把整个 trigger 节点输出对象当变量值传递。
                    ("stock_code".to_string(), "stock_code".to_string()),
                    ("stock_name".to_string(), "stock_name".to_string()),
                    ("actual_outcome".to_string(), "actual_outcome".to_string()),
                    ("reflection_depth".to_string(), "reflection_depth".to_string()),
                    ("raw_return_pct".to_string(), "raw_return_pct".to_string()),
                    ("alpha_return_pct".to_string(), "alpha_return_pct".to_string()),
                    ("holding_days".to_string(), "holding_days".to_string()),
                    ("benchmark_name".to_string(), "benchmark_name".to_string()),
                    ("stock_lessons".to_string(), "stock_lessons".to_string()),
                    ("hindsight_date".to_string(), "hindsight_date".to_string()),
                    ("deviation_report".to_string(), "reflection-comparator".to_string()),
                    // [实际行情] 价格层事实文本块（入场价/最新价/涨跌/回撤/目标价实现度），
                    // 由 run_reflection_workflow 顶层注入。不接会让 prompt 里的
                    // {{actual_market_text}} 渲染为空或 VARIABLE_NOT_FOUND。
                    ("actual_market_text".to_string(), "actual_market_text".to_string()),
                    // 〇-B v2 第 4 条：本次复盘档。同样由 run_reflection_workflow 顶层注入，
                    // 漏映射 ⇒ {{review_horizon}} VARIABLE_NOT_FOUND ⇒ 整条反思链 Failed。
                    ("review_horizon".to_string(), "review_horizon".to_string()),
                    (
                        "review_expected_holding_days".to_string(),
                        "review_expected_holding_days".to_string(),
                    ),
                ]
                .into_iter()
                .collect(),
                output_var: "reflection".into(),
                model: None,
                temperature: Some(0.3),
                max_tokens: Some(32768),
                tools: refl_tools,
                exposed_tools: vec![],
                output_mode: OutputMode::Json,
                agent_profile_id: Some("stock-reflection".into()),
                max_tool_rounds: Some(3), // 限制工具调用轮数，防止过度拉数据
                execution_mode: None,
                // 从 stock_reflections 记忆空间检索语义相似的历史反思
                rag_source_ids: vec!["memory:stock_reflections".into()],
                consistency_check: Some(axagent_harness::ConsistencyCheckConfig {
                    enabled: true,
                    mode: axagent_harness::ConsistencyMode::SameModelRepeated,
                    secondary_model: None,
                    deviation_threshold: 0.3,
                }),
                hallucination_guard: Some(axagent_harness::HallucinationGuardConfig {
                    enabled: false,
                    match_threshold: 0.4,
                }),
                fallback_model: None,
                task_scene: None,
                stream_chunk_timeout_secs: None,
            },
        }),
        // 4. 硬裁决验证：reflection-agent → reflection-validator → store-ref
        //    [P1-#1] 接入原死代码 reflection_validator.rhai，自动修正字段类型/枚举值/长度
        validator_node,
        // 5. 反思记录持久化：写入 stock_reflections 表供后续查询/复盘
        WorkflowNode::Storage(StorageNode {
            base: WorkflowNodeBase {
                id: "store-ref".into(),
                title: "反思记录持久化".into(),
                description: Some("写入反思记录到 stock_reflections 表".into()),
                position: Position { x: 20.0, y: 500.0 },
                retry: RetryConfig { enabled: true, max_retries: 2, ..Default::default() },
                timeout: Some(30),
                enabled: true,
                parent_id: None,
                compensation: None,
                continue_on_fail: false,
            },
            config: StorageNodeConfig {
                backend: "sqlite".into(),
                // [BUGFIX] 改为 upsert：B3 路径下 run_reflection_workflow 已 UPDATE
                // pending row（通过 pending_id 匹配），store-ref 不应再 INSERT 重复 row。
                // upsert 语义：若 pending row 存在则 UPDATE，否则 INSERT。
                operation: "upsert".into(),
                // [P1-#1] 使用验证后的输出（reflection-validator 节点 output_var）
                input_var: "reflection-validated".into(),
                collection: "stock_reflections".into(),
                key_var: None,
                output_var: "storage-result".into(),
            },
        }),
    ];

    let edges: Vec<WorkflowEdge> = vec![
        // [v2] trigger → reflection-comparator 直连（删除 sub-analysis 中间节点）
        WorkflowEdge {
            id: "e-trigger-comparator".into(),
            source: "trigger".into(),
            source_handle: None,
            target: "reflection-comparator".into(),
            target_handle: None,
            edge_type: EdgeType::Direct,
            label: None,
        },
        WorkflowEdge {
            id: "e-comparator-reflection".into(),
            source: "reflection-comparator".into(),
            source_handle: None,
            target: "reflection-agent".into(),
            target_handle: None,
            edge_type: EdgeType::Direct,
            label: None,
        },
        // [P1-#1] reflection-agent → reflection-validator → store-ref
        WorkflowEdge {
            id: "e-reflection-validator".into(),
            source: "reflection-agent".into(),
            source_handle: None,
            target: "reflection-validator".into(),
            target_handle: None,
            edge_type: EdgeType::Direct,
            label: None,
        },
        WorkflowEdge {
            id: "e-validator-store".into(),
            source: "reflection-validator".into(),
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
        // [实际行情 v2] 价格层事实。运行时由 run_reflection_workflow 用
        // compute_market_snapshots() 的结果覆盖；此处声明默认值是为了：
        // ① 模板变量表完整（前端模板编辑器可见）；
        // ② input_mapping / context_sources 能找到 source 变量，避免静默取空。
        Variable {
            name: "actual_market_text".into(),
            var_type: "string".into(),
            value: serde_json::Value::String(String::new()),
            description: Some(
                "当前实际行情文本块（入场基准价/最新价/涨跌/回撤/超额/目标价实现度）".into(),
            ),
            is_secret: false,
        },
        Variable {
            name: "actual_market_json".into(),
            var_type: "object".into(),
            value: serde_json::json!({}),
            description: Some(
                "当前实际行情结构化快照（供 reflection-comparator 按路径下钻）".into(),
            ),
            is_secret: false,
        },
        // [v3 单档复盘] 〇-B v2 第 4 条：一行反思 = 一个周期档。
        // 运行时由 run_reflection_workflow 用盖章后的 primary_horizon 覆盖。
        Variable {
            name: "review_horizon".into(),
            var_type: "string".into(),
            value: serde_json::Value::String(String::new()),
            description: Some("本次反思唯一复盘的周期档（ultra_short/short/mid/long）".into()),
            is_secret: false,
        },
        Variable {
            name: "review_expected_holding_days".into(),
            var_type: "number".into(),
            value: serde_json::json!(0),
            description: Some("本次复盘档的期望持有天数（交易日）".into()),
            is_secret: false,
        },
        Variable {
            name: "analysis_primary_horizon".into(),
            var_type: "string".into(),
            value: serde_json::Value::String("none".into()),
            description: Some("原分析公式定档的主周期档，未必等于本次复盘档".into()),
            is_secret: false,
        },
    ];

    // serenity-reflection 模板版本。
    //
    // v2 (2026-09-13)：「结论 vs 当前实际行情」改造 ——
    //   ① reflection-agent prompt 增补 {{actual_market_text}} 价格层事实段
    //      （入场基准价/最新价/涨跌/回撤/目标价实现度/期中观察标记）；
    //   ② reflection-comparator input_mapping 接入 actual_market_json 的 9 个价格字段，
    //      并把此前是死映射的 trader_target_price 真正接线为「目标价兑现度」对比；
    //   ③ 新增变量定义 actual_market_text / actual_market_json（由 run_reflection_workflow
    //      **无条件注入** —— 缺失会让 comparator 与 prompt 双双 VARIABLE_NOT_FOUND）。
    //
    // v3 (2026-09-30)：一行反思 = 一个周期档 ——
    //   ① reflection-agent 新增 {{review_horizon}} / {{review_expected_holding_days}}，
    //      prompt 由「四周期分别判断」改为**单档复盘原则**（其余三档只作背景参照）；
    //   ② 新增变量 analysis_primary_horizon（原分析主档，与复盘档分开陈述，两者可不一致）；
    //   ③ 专家档案 reflection.md 同步改写 —— prompt 有**两处**（本节点内联 system_prompt
    //      与 reflection.md），只改一处不生效。
    //
    // ⚠ 升版必做：不升版 DB 里会保留 v1 模板行，本次全部改动静默失效。
    const REFLECTION_TEMPLATE_VERSION: i32 = 3;

    // 版本检查：已有同版本或更新的记录则跳过
    if let Some(ref existing) =
        axagent_entities::workflow_template::Entity::find_by_id("stock-reflection")
            .one(db)
            .await
            .map_err(|e| {
                ErrorResponse::new(stock_setup::INTERNAL).with_detail(format!("查重失败: {e}"))
            })?
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
        if axagent_entities::workflow_template_version::Entity::find_by_id(&ver_id)
            .one(db)
            .await
            .map_err(|e| {
                ErrorResponse::new(stock_setup::INTERNAL).with_detail(format!("查重失败: {e}"))
            })?
            .is_none()
        {
            use crate::commands::error::ErrorResponse;
            use sea_orm::ActiveModelTrait;
            let snapshot = axagent_entities::workflow_template_version::ActiveModel {
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
            snapshot.insert(db).await.map_err(|e| {
                ErrorResponse::new(stock_setup::INTERNAL)
                    .with_detail(format!("写入版本快照失败: {e}"))
            })?;
            tracing::info!("[stock_analysis_setup] 反思模板旧版本快照已保存: {ver_id}");
        }
    }

    // 走 stock-analysis 同款序列化路径：编译期校验 + 字段齐全
    let nodes_json = serde_json::to_string(&nodes).map_err(|e| {
        ErrorResponse::new(stock_setup::INTERNAL).with_detail(format!("序列化反思节点失败: {e}"))
    })?;
    let edges_json = serde_json::to_string(&edges).map_err(|e| {
        ErrorResponse::new(stock_setup::INTERNAL).with_detail(format!("序列化反思边失败: {e}"))
    })?;
    let variables_json = serde_json::to_string(&variables).map_err(|e| {
        ErrorResponse::new(stock_setup::INTERNAL).with_detail(format!("序列化反思变量失败: {e}"))
    })?;
    let tags_json = serde_json::to_string(&["stock", "reflection", "A股"]).map_err(|e| {
        ErrorResponse::new(stock_setup::INTERNAL).with_detail(format!("序列化反思标签失败: {e}"))
    })?;

    // 先删再插，避免 SeaORM .save() 对已存在记录的 update 失败
    let _ = workflow_template::Entity::delete_by_id("stock-reflection").exec(db).await;

    // P0 软门禁（C1，2026-09-14）：见 `opc_workflows::upsert_template` 同款说明。
    axagent_harness::workflow_port_axioms::warn_port_axioms_json(
        "stock_analysis_setup:seed_reflection_workflow_template:stock-reflection",
        &nodes_json,
        &edges_json,
    );

    workflow_template::ActiveModel {
        hooks_config: Set(None),
        id: Set("stock-reflection".to_string()),
        cluster_id: Set(None),
        route_path: Set(None),
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
            .map_err(|e| ErrorResponse::new(stock_setup::INTERNAL).with_detail(format!("序列化触发器配置失败: {e}")))?,
        )),
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
    .map_err(|e| ErrorResponse::new(stock_setup::INTERNAL).with_detail(format!("写入反思模板失败: {e}")))?;

    tracing::info!(
        "[stock_analysis_setup] 反思复盘工作流模板已创建 (stock-reflection, SubWorkflowNode 嵌套)"
    );
    Ok(())
}

// ───────────────────────────────────────────────────────────────────────────
// P2-2: 决策事件总线订阅方模板
//
// 两个模板都订阅 stock_workflow/core.rs 在决策落库后发布的 "decision.completed"
// 事件。事件 payload 字段（camelCase）:
//   { analysisId, stockCode, stockName, action, decisionJson, asOfDate,
//     parentAnalysisId, timestamp }
//
// publish_event → engine.run_workflow(wf_id, RunOptions { input: payload })
// → start_workflow 把 payload 存入 state.input_params
// → 节点执行时 merged_vars 合并顺序：deps_results → context_sources
//    → state.variables → state.input_params（兜底）
// → AgentNode 通过 input_mapping 引用 payload 字段（value = payload 字段名）
// ───────────────────────────────────────────────────────────────────────────

/// 事件订阅型决策联动模板的公共字段配置。
struct EventTriggeredTemplateSpec {
    /// 模板 ID（也是 workflow_id，注册到 TriggerManager）
    template_id: &'static str,
    /// 模板显示名
    name: &'static str,
    /// 模板描述
    description: &'static str,
    /// 图标
    icon: &'static str,
    /// Agent 节点 ID（用于前端节点标题国际化）
    agent_node_id: &'static str,
    /// Agent 节点标题
    agent_node_title: &'static str,
    /// Agent 系统提示词（支持 {{stock_code}} 等占位符）
    agent_system_prompt: &'static str,
    /// Agent 输出变量名
    output_var: &'static str,
    /// 模板版本
    version: i32,
    /// 标签
    tags: &'static [&'static str],
    /// Agent Profile ID（用于绑定 Role + Expert）
    agent_profile_id: Option<&'static str>,
}

/// 内部辅助函数：构建并持久化一个事件订阅型决策联动工作流模板。
///
/// 模板结构：TriggerNode(Event) → AgentNode → EndNode
/// - TriggerNode 配置 EventTriggerConfig { event_type: "decision.completed" }
/// - AgentNode 通过 input_mapping 引用 payload 字段，输出 JSON 结果
/// - EndNode 终止工作流
///
/// 顶层 `trigger_config` 字段也写入 EventTriggerConfig，供 trigger_recovery.rs
/// 在进程重启时恢复事件订阅到 TriggerManager。
async fn seed_event_triggered_decision_template(
    db: &sea_orm::DatabaseConnection,
    spec: EventTriggeredTemplateSpec,
) -> Result<(), String> {
    use axagent_entities::workflow_template;
    use axagent_harness::workflow_types::{
        AgentNode, AgentNodeConfig, EdgeType, EndNode, EndNodeConfig, EventTriggerConfig,
        OutputMode, Position, RetryConfig, TriggerConfig, TriggerNode, TriggerType, WorkflowEdge,
        WorkflowNode, WorkflowNodeBase,
    };
    use sea_orm::{ActiveModelTrait, EntityTrait, Set};

    let now = chrono::Utc::now().timestamp_millis();

    // 事件触发器配置（同时用于 TriggerNode 和顶层 trigger_config 字段）
    let event_cfg =
        EventTriggerConfig { event_type: "decision.completed".to_string(), filter: None };
    let event_cfg_value = serde_json::to_value(&event_cfg).map_err(|e| {
        ErrorResponse::new(stock_setup::INTERNAL).with_detail(format!("序列化事件配置失败: {e}"))
    })?;
    let trigger_config =
        TriggerConfig { trigger_type: TriggerType::Event, config: event_cfg_value.clone() };

    // ── 节点定义 ──
    let nodes: Vec<WorkflowNode> = vec![
        // 1. 触发器：订阅 decision.completed 事件
        WorkflowNode::Trigger(TriggerNode {
            base: WorkflowNodeBase {
                id: "trigger".into(),
                title: "决策事件触发器".into(),
                description: Some("订阅 decision.completed 事件，决策落库后自动触发".into()),
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
        // 2. Agent 节点：基于决策 payload 推理输出
        WorkflowNode::Agent(AgentNode {
            base: WorkflowNodeBase {
                id: spec.agent_node_id.into(),
                title: spec.agent_node_title.into(),
                description: Some(spec.description.into()),
                position: Position { x: 20.0, y: 180.0 },
                retry: RetryConfig { enabled: true, max_retries: 1, ..Default::default() },
                timeout: Some(300),
                enabled: true,
                parent_id: None,
                compensation: None,
                continue_on_fail: false,
            },
            config: AgentNodeConfig {
                system_prompt: spec.agent_system_prompt.into(),
                // 引用触发器节点输出（含 status/trigger_type/config/timestamp），
                // 实际决策数据通过 input_mapping + input_params 兜底注入。
                context_sources: vec!["trigger".into()],
                input_mapping: [
                    ("stock_code".to_string(), "stockCode".to_string()),
                    ("stock_name".to_string(), "stockName".to_string()),
                    ("action".to_string(), "action".to_string()),
                    ("decision_json".to_string(), "decisionJson".to_string()),
                    ("as_of_date".to_string(), "asOfDate".to_string()),
                    ("analysis_id".to_string(), "analysisId".to_string()),
                ]
                .into_iter()
                .collect(),
                output_var: spec.output_var.into(),
                model: None,
                temperature: Some(0.3),
                max_tokens: Some(4096),
                tools: vec![],
                exposed_tools: vec![],
                output_mode: OutputMode::Json,
                agent_profile_id: spec.agent_profile_id.map(|id| id.into()),
                max_tool_rounds: Some(1),
                execution_mode: None,
                rag_source_ids: vec![],
                consistency_check: None,
                hallucination_guard: None,
                fallback_model: None,
                task_scene: None,
                stream_chunk_timeout_secs: None,
            },
        }),
        // 3. 终止节点
        WorkflowNode::End(EndNode {
            base: WorkflowNodeBase {
                id: "end".into(),
                title: "结束".into(),
                description: None,
                position: Position { x: 20.0, y: 340.0 },
                retry: RetryConfig::default(),
                timeout: None,
                enabled: true,
                parent_id: None,
                compensation: None,
                continue_on_fail: false,
            },
            config: EndNodeConfig { output_var: Some(spec.output_var.into()) },
        }),
    ];

    let edges: Vec<WorkflowEdge> = vec![
        WorkflowEdge {
            id: "e-trigger-agent".into(),
            source: "trigger".into(),
            source_handle: None,
            target: spec.agent_node_id.into(),
            target_handle: None,
            edge_type: EdgeType::Direct,
            label: None,
        },
        WorkflowEdge {
            id: "e-agent-end".into(),
            source: spec.agent_node_id.into(),
            source_handle: None,
            target: "end".into(),
            target_handle: None,
            edge_type: EdgeType::Direct,
            label: None,
        },
    ];

    // ── 版本检查与快照（与 seed_reflection_workflow_template 同款逻辑）──
    if let Some(ref existing) =
        workflow_template::Entity::find_by_id(spec.template_id).one(db).await.map_err(|e| {
            ErrorResponse::new(stock_setup::INTERNAL).with_detail(format!("查重失败: {e}"))
        })?
    {
        if existing.version >= spec.version {
            tracing::info!(
                "[stock_analysis_setup] {} 模板已是最新 v{}，跳过种子化",
                spec.template_id,
                existing.version
            );
            return Ok(());
        }
        // 旧版本 → 保存快照
        let ver_id = format!("{}_v{}", spec.template_id, existing.version);
        if axagent_entities::workflow_template_version::Entity::find_by_id(&ver_id)
            .one(db)
            .await
            .map_err(|e| {
                ErrorResponse::new(stock_setup::INTERNAL).with_detail(format!("查重失败: {e}"))
            })?
            .is_none()
        {
            let snapshot = axagent_entities::workflow_template_version::ActiveModel {
                id: Set(ver_id.clone()),
                template_id: Set(spec.template_id.to_string()),
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
            snapshot.insert(db).await.map_err(|e| {
                ErrorResponse::new(stock_setup::INTERNAL)
                    .with_detail(format!("写入版本快照失败: {e}"))
            })?;
            tracing::info!(
                "[stock_analysis_setup] {} 模板旧版本快照已保存: {ver_id}",
                spec.template_id
            );
        }
    }

    // 序列化节点/边/标签
    let nodes_json = serde_json::to_string(&nodes).map_err(|e| {
        ErrorResponse::new(stock_setup::INTERNAL).with_detail(format!("序列化节点失败: {e}"))
    })?;
    let edges_json = serde_json::to_string(&edges).map_err(|e| {
        ErrorResponse::new(stock_setup::INTERNAL).with_detail(format!("序列化边失败: {e}"))
    })?;
    let tags_json = serde_json::to_string(spec.tags).map_err(|e| {
        ErrorResponse::new(stock_setup::INTERNAL).with_detail(format!("序列化标签失败: {e}"))
    })?;
    let trigger_config_json = serde_json::to_string(&trigger_config).map_err(|e| {
        ErrorResponse::new(stock_setup::INTERNAL).with_detail(format!("序列化触发器配置失败: {e}"))
    })?;

    // 先删再插，避免 .save() 对已存在记录的 update 失败
    let _ = workflow_template::Entity::delete_by_id(spec.template_id).exec(db).await;

    // P0 软门禁（C1，2026-09-14）：见 `opc_workflows::upsert_template` 同款说明。
    axagent_harness::workflow_port_axioms::warn_port_axioms_json(
        &format!(
            "stock_analysis_setup:seed_event_triggered_decision_template:{}",
            spec.template_id
        ),
        &nodes_json,
        &edges_json,
    );

    workflow_template::ActiveModel {
        hooks_config: Set(None),
        id: Set(spec.template_id.to_string()),
        cluster_id: Set(None),
        route_path: Set(None),
        name: Set(spec.name.to_string()),
        description: Set(Some(spec.description.to_string())),
        icon: Set(spec.icon.into()),
        tags: Set(Some(tags_json)),
        version: Set(spec.version),
        is_preset: Set(true),
        is_editable: Set(true),
        is_public: Set(true),
        trigger_config: Set(Some(trigger_config_json)),
        nodes: Set(nodes_json),
        edges: Set(edges_json),
        input_schema: Set(None),
        output_schema: Set(None),
        variables: Set(None),
        error_config: Set(None),
        composite_source: Set(None),
        tool_defs: Set(None),
        mission_hash: Set(None),
        created_at: Set(now),
        updated_at: Set(now),
    }
    .insert(db)
    .await
    .map_err(|e| {
        ErrorResponse::new(stock_setup::INTERNAL)
            .with_detail(format!("写入 {} 模板失败: {e}", spec.template_id))
    })?;

    tracing::info!(
        "[stock_analysis_setup] 决策事件订阅模板已创建: {} ({})",
        spec.template_id,
        spec.name
    );
    Ok(())
}

/// P2-2: 自动仓位规划模板。
///
/// 订阅 `decision.completed` 事件，决策落库后自动触发，基于决策的 action /
/// confidence / positionPct 等字段生成分批建仓 / 止损位 / 止盈位 / 资金分配
/// 方案。输出 JSON 结果到工作流执行历史，供前端展示与后续追溯。
async fn seed_auto_position_plan_template(db: &sea_orm::DatabaseConnection) -> Result<(), String> {
    let spec = EventTriggeredTemplateSpec {
        template_id: "auto-position-plan",
        name: "自动仓位规划",
        description: "订阅决策完成事件，自动生成分批建仓/止损/止盈/资金分配方案",
        icon: "wallet",
        agent_node_id: "position-planner",
        agent_node_title: "仓位规划助手",
        version: 1,
        tags: &["stock", "position", "auto", "A股"],
        agent_system_prompt: r#"基于上游交易决策，输出结构化的仓位执行方案。

【输入决策上下文】
- 股票代码: {{stock_code}}
- 股票名称: {{stock_name}}
- 决策动作: {{action}}（买入/增持/持有/减持/卖出/观望）
- 决策详情(JSON): {{decision_json}}
- 分析日期: {{as_of_date}}
- 分析记录 ID: {{analysis_id}}

请根据仓位规划方法论完成任务，输出 JSON 结果。"#,
        output_var: "position-plan",
        agent_profile_id: Some("stock-position-planner"),
    };
    seed_event_triggered_decision_template(db, spec).await
}

/// P2-2: 自动止损复查模板。
///
/// 订阅 `decision.completed` 事件，对决策的止损合理性进行独立复查，
/// 输出复查结论与调整建议。与 auto-position-plan 形成双视角对照。
async fn seed_auto_stop_loss_review_template(
    db: &sea_orm::DatabaseConnection,
) -> Result<(), String> {
    let spec = EventTriggeredTemplateSpec {
        template_id: "auto-stop-loss-review",
        name: "自动止损复查",
        description: "订阅决策完成事件，独立复查止损位合理性并输出调整建议",
        icon: "shield",
        agent_node_id: "stop-loss-reviewer",
        agent_node_title: "止损复查助手",
        version: 1,
        tags: &["stock", "risk", "auto", "A股"],
        agent_system_prompt: r#"对上游交易决策的止损合理性进行风控视角的二次审视。

【输入决策上下文】
- 股票代码: {{stock_code}}
- 股票名称: {{stock_name}}
- 决策动作: {{action}}
- 决策详情(JSON): {{decision_json}}
- 分析日期: {{as_of_date}}
- 分析记录 ID: {{analysis_id}}

请根据止损复查方法论完成任务，输出 JSON 结果。"#,
        output_var: "stop-loss-review",
        agent_profile_id: Some("stock-stop-loss-reviewer"),
    };
    seed_event_triggered_decision_template(db, spec).await
}

#[cfg(test)]
mod force_variable_value_tests {
    use super::force_variable_value;

    /// v48 用例：`debate_rounds` 的 DB 旧值 3 必须被覆写为 1
    /// （`merge_variable_values` 的「旧值优先」语义让默认值修改失效）。
    #[test]
    fn 覆写已存在变量的值() {
        let input = r#"[
            {"name":"analysis_depth","var_type":"enum","value":"standard","is_secret":false},
            {"name":"debate_rounds","var_type":"number","value":3,"is_secret":false},
            {"name":"kline_limit","var_type":"number","value":120,"is_secret":false}
        ]"#;
        let out = force_variable_value(input, "debate_rounds", serde_json::json!(1));
        let vars: Vec<serde_json::Value> = serde_json::from_str(&out).expect("输出必须是合法 JSON");
        let get = |n: &str| vars.iter().find(|v| v.get("name").and_then(|x| x.as_str()) == Some(n));
        assert_eq!(get("debate_rounds").unwrap()["value"], 1);
        // 其余变量必须原样保留（只动目标，不做整表替换）
        assert_eq!(get("analysis_depth").unwrap()["value"], "standard");
        assert_eq!(get("kline_limit").unwrap()["value"], 120);
        assert_eq!(vars.len(), 3);
    }

    /// 变量不存在 ⇒ 原样返回（不 panic、不阻断种子），仅告警
    #[test]
    fn 变量不存在时保持原样() {
        let input = r#"[{"name":"a","var_type":"number","value":1,"is_secret":false}]"#;
        let out = force_variable_value(input, "not_there", serde_json::json!(9));
        let vars: Vec<serde_json::Value> = serde_json::from_str(&out).unwrap();
        assert_eq!(vars.len(), 1);
        assert_eq!(vars[0]["value"], 1);
    }

    /// 非法 JSON ⇒ 原样返回（种子流程不能因为一个变量覆写失败而整体中断）
    #[test]
    fn 非法输入时保持原样() {
        let out = force_variable_value("not json at all", "x", serde_json::json!(1));
        assert_eq!(out, "not json at all");
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// 版本门落库守门（2026-09-20 新增）
//
// ## 为什么需要它
//
// 种子的版本门是 `existing.version >= TEMPLATE_VERSION ⇒ return Ok(())`（`>=`）。
// 这条判据把「代码改了要不要重建 DB 模板」系在一个**纯数字比较**上，而 DB 现值
// 可以被别的路径写高（并发会话 / 带更高常量的构建 / 历史遗留）。
//
// 实测事故（2026-09-20，真库 + `workflow_template_versions` 快照表双重取证）：
// 常量停留在 55，DB 已是 58 ⇒ 版本门**永久跳过** ⇒ **v53 / v54 / v55 三批改动
// 全部未落库**（含 `cls-risk-level` 下沉 Rhai），而 `cargo check` / `clippy` /
// `fmt` / `test` **四道门全绿** —— 它们只回答「代码能否编译」，没有一个能回答
// 「版本门会不会开门」。
//
// ## 它守什么、不守什么
//
// - **守**：门的三态语义（空库建、旧版升级、新版跳过）、以及种子**真的把内容写进去**
//   （版本号对而内容没变是另一种独立失败）。判据全部从 `TEMPLATE_VERSION` **现取**，
//   不写死数字 —— 所以改常量不会让本测试腐烂。
// - **不守**：「生产库现值是否低于常量」。那需要一个具体数字事实，属运维侧
//   （用 `output/verify-v59-seed.mjs` 之类的只读脚本查），不该固化成断言。
//
// ## 为什么不碰生产库
//
// 用 `axagent_dao::db::create_test_pool()`：它走**与生产同一个** `initialize_schema`
// 建表链（仅连接配置不同），落在 `temp_dir` 的唯一 SQLite 文件上 ⇒ 无外部依赖、
// 可直接进 CI，也不会像真跑生产库那样有「delete 成功 / insert 失败 ⇒ 模板丢失」的风险。
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod version_gate_tests {
    use super::seed_stock_analysis::{TEMPLATE_VERSION, seed_stock_analysis_workflow_template};
    // v140：按档风险节点已进档子模板 ⇒ 本模块的判据要能读**子模板的 typed 节点**
    // （`check_horizon_scoped_risk` 用它），不能只看父图 JSON。
    use axagent_entities::workflow_template;
    use axagent_harness::holding_period::Period;
    use axagent_harness::workflow_types::{WorkflowEdge, WorkflowNode};
    use sea_orm::{ActiveModelTrait, DatabaseConnection, EntityTrait, Set};

    /// 被测模板 id（与 `seed_stock_analysis.rs` 内的 `TEMPLATE_ID` 同值；
    /// 此处另取一份是因为它在那个函数体内，模块外不可见）。
    const TEMPLATE_ID: &str = "stock-analysis";

    async fn fresh_db() -> axagent_dao::db::DbHandle {
        axagent_dao::db::create_test_pool().await.expect("建临时测试库失败")
    }

    async fn seed(db: &DatabaseConnection) -> Result<(), String> {
        seed_stock_analysis_workflow_template(db).await
    }

    /// 当前 version（模板不存在时 `None`）。
    async fn version_of(db: &DatabaseConnection) -> Option<i32> {
        workflow_template::Entity::find_by_id(TEMPLATE_ID)
            .one(db)
            .await
            .expect("查模板失败")
            .map(|m| m.version)
    }

    async fn nodes_of(db: &DatabaseConnection) -> Vec<serde_json::Value> {
        let model = workflow_template::Entity::find_by_id(TEMPLATE_ID)
            .one(db)
            .await
            .expect("查模板失败")
            .expect("模板应已存在");
        serde_json::from_str(&model.nodes).expect("nodes 应是 JSON 数组")
    }

    /// 边清单（供给关系）。`input_mapping` 只声明「读谁」，不保证「它已跑完」——
    /// 所以边的缺失是**独立**的一格失效面，必须由门单独看。
    async fn edges_of(db: &DatabaseConnection) -> Vec<serde_json::Value> {
        let model = workflow_template::Entity::find_by_id(TEMPLATE_ID)
            .one(db)
            .await
            .expect("查模板失败")
            .expect("模板应已存在");
        serde_json::from_str(&model.edges).expect("edges 应是 JSON 数组")
    }

    /// 把 version 强改为指定值 —— 模拟「DB 现值被别的路径写高 / 写低」。
    async fn force_version(db: &DatabaseConnection, v: i32) {
        let model = workflow_template::Entity::find_by_id(TEMPLATE_ID)
            .one(db)
            .await
            .expect("查模板失败")
            .expect("模板应已存在");
        let mut am: workflow_template::ActiveModel = model.into();
        am.version = Set(v);
        am.update(db).await.expect("改 version 失败");
    }

    /// 门的**三态语义**（顺序敏感，必须同一个库）。
    #[tokio::test]
    async fn version_gate_seeds_then_upgrades_then_skips() {
        let handle = fresh_db().await;
        let db = &handle.conn;

        // ① 空库 ⇒ 放行，且 version 落在常量现值上
        assert!(version_of(db).await.is_none(), "前置：临时库应为空");
        seed(db).await.expect("空库种子化应成功");
        assert_eq!(
            version_of(db).await,
            Some(TEMPLATE_VERSION),
            "空库种子化后 version 应等于 TEMPLATE_VERSION"
        );

        // ② 旧版本 ⇒ **必须升级**（这正是 v53/v54/v55 卡住的那条路径）
        force_version(db, TEMPLATE_VERSION - 1).await;
        seed(db).await.expect("旧版本重种子化应成功");
        assert_eq!(
            version_of(db).await,
            Some(TEMPLATE_VERSION),
            "DB 版本低于常量时必须被升级 —— 否则就是「改了代码不生效」"
        );

        // ③ **门关闭**：DB 高于常量 ⇒ 跳过重建，且**不得篡改 version**
        //
        //    本仓真实事故：DB 58 / 常量 55 ⇒ 永久跳过 ⇒ 三批改动一字不落库。
        //    这里把「跳过」语义钉成契约（它是设计，不是缺陷），
        //    结论是**常量取值必须严格大于 DB 现值** —— 代价由注释与运维脚本承担，
        //    不由本测试包办（它拿不到生产库）。
        let higher = TEMPLATE_VERSION + 1;
        force_version(db, higher).await;
        seed(db).await.expect("门关闭时种子化应静默成功（不是报错）");
        assert_eq!(
            version_of(db).await,
            Some(higher),
            "DB 版本高于常量时必须跳过重建，且跳过时不得把 version 拉低"
        );
    }

    /// v55 的**实质内容**必须随种子一起落库（版本号对 ≠ 内容对）。
    ///
    /// 单独一条的理由：实测事故里 DB 的 `cls-risk-level` 一直是 `llmClassifier`
    /// 而版本门看着「正常」—— 版本号与内容是两种独立的失败面。
    #[tokio::test]
    async fn seeded_template_carries_rhai_risk_level_node() {
        let handle = fresh_db().await;
        let db = &handle.conn;
        seed(db).await.expect("种子化应成功");

        let nodes = nodes_of(db).await;
        let cls = nodes
            .iter()
            .find(|n| n.get("id").and_then(|v| v.as_str()) == Some("cls-risk-level"))
            .expect("模板里应有 cls-risk-level 节点");

        assert_eq!(
            cls.get("type").and_then(|v| v.as_str()),
            Some("code"),
            "cls-risk-level 必须是 CodeNode（Rhai 确定性实现）；\
             若为 llmClassifier，说明 v55 的下沉改动没进这份模板"
        );

        let code = cls.pointer("/config/code").and_then(|v| v.as_str()).unwrap_or("");
        assert!(!code.is_empty(), "CodeNode 的 code 字段不能为空（`include_str!` 嵌入失败？）");
        assert!(
            code.contains("matched_rules"),
            "code 字段应含 `risk-level.rhai` 的输出契约字段 `matched_rules`；\
             实际开头: {}",
            code.chars().take(200).collect::<String>()
        );
        // 反向判据：「两份实现被缝在一起」的**真实形态** = 一个节点同时带
        // `code` 与 `prompt` 字段 ⇒ 断言旧 LlmClassifierNode 的专属字段已消失。
        //
        // ⚠️⚠️ 不可写成「`code` 不得含 prompt 原文」—— `risk-level.rhai:113-114`
        //   写明该段 prompt「**完整保留作为本脚本的口径权威来源**」，第 116 行注释里
        //   就有「你是专业风险分析师…」这句。种子用 `include_str!` 嵌入**整个文件**，
        //   故该串**必然**出现在 `config.code` 里。按「不得含」写法本测试在 v59 上
        //   必红（false red），且检不出真正的缝合形态 —— 它只是把判据锚错了对象。
        //   （同型事故：`output/verify-v59-seed.mjs` 初版亦犯此错，已同步修正。）
        let cfg = cls.get("config").expect("cls-risk-level 应有 config 对象");
        let prompt = cfg.get("prompt");
        assert!(
            prompt.is_none_or(serde_json::Value::is_null),
            "CodeNode 的 config 不得残留 LLM 版专属字段 `prompt`（那是半新半旧的缝合形态）；\
             实际 config 键: {:?}",
            cfg.as_object().map(|o| o.keys().collect::<Vec<_>>())
        );

        // 配套映射也必须同批落库：portfolio-mgr 读的是 `...result.category`，
        // 而 CodeNode 的输出被包在 `result` 里 —— 只改节点、漏改下游 = 断链。
        let pm = nodes
            .iter()
            .find(|n| n.get("id").and_then(|v| v.as_str()) == Some("portfolio-mgr"))
            .expect("模板里应有 portfolio-mgr 节点");
        assert_eq!(
            pm.pointer("/config/input_mapping/overall_risk_llm").and_then(|v| v.as_str()),
            Some("cls-risk-level.result.category"),
            "portfolio-mgr 的 overall_risk_llm 必须指向 `cls-risk-level.result.category`\
             （CodeNode 输出多一层 `result` 包装）—— 否则风险档位整段读不到"
        );
    }

    /// v128（B1）「节点 id 后缀 ↔ `riskWindows` 的 camelCase 键 ↔ 档位 snake 键」对齐表。
    ///
    /// 三形并存是既成约定（节点 id 走 kebab、DTO 字段走 camelCase、档位值域走 snake），
    /// **本表是它们唯一的对齐点** —— 抄错任一处（超短节点去读 `mid` 那一格）当场红。
    const B1_TIER_KEYS: [(&str, &str, &str); 4] = [
        ("ultra-short", "ultraShort", "ultra_short"),
        ("short", "short", "short"),
        ("mid", "mid", "mid"),
        ("long", "long", "long"),
    ];

    const B1_GLOBAL_KEYS: [&str; 7] = [
        "risk_volatility",
        "risk_drawdown",
        "risk_sharpe",
        "risk_roe",
        "risk_gross_margin",
        "risk_debt_ratio",
        "risk_revenue_growth",
    ];

    /// 按档风险图的**谓词**（写成纯函数是为了能被**变异样本**调用 —— 见下面两条负控）。
    ///
    /// 判据面（缺任一格都能假绿）：
    ///   ① 四张档子模板各有本档 `cls-risk-level-{kebab}` CodeNode，其 code 含按档深度判据，
    ///      且子图内有「本档节点 → 本档分支」这条边（v140 起风险节点在**子模板**里）；
    ///   ② 每个节点的**按档两键**指向自己那一格（`riskWindows.<camelTier>.…`）——
    ///      「复制四遍得到四份相同结论」那种伪装，只有这一格检得出来（数节点数量检不出）；
    ///   ③ 七条全局键仍指全局（本版没按档的轴，声明必须与事实一致）；
    ///   ④ 四路扇出 `pm-h-{kebab}` 必须以恒等键把 **`t-risk`** 传进子快照（风险节点在子图里
    ///      读的就是它）—— `map_inputs` 严格 ⇒ 少这个键就是整档子执行硬错；
    ///   ⑤ 主链 `portfolio-mgr` 四个 `overall_risk_{snake}` 必须指 `pm-h-{kebab}.result.riskCategory`
    ///      （分支行带回的那一格，见 `portfolio-mgr-h-*.rhai`）；
    ///   ⑥ 父图**不得再有** `cls-risk-level-<档>` 节点，也不得以它为端点的边（搬干净了的正面
    ///      断言 —— 留着就是两份权威，父图那份照样跑出一个没人读的风险档）；父侧供给边
    ///      `t-risk → pm-h-<档>` 必须在；
    ///   ⑦ 全局节点仍在**且不注入**按档两键（注入了它就跟着加严 ⇒ 等于偷偷把 60 日整票
    ///      口径换成按档口径，research-mgr 与 `LLM回退` 两条消费面会跟着漂）。
    ///
    /// `child_of` 是**注入**而不是内部直调 builder：负控要能递一份「把 mid 的按档键改成读 long
    /// 那一格」的变异子模板进来 —— 判据面搬到子模板之后，再靠改父图 JSON 变异就打不到它了
    /// （打不到的负控＝另一种假绿）。
    fn check_horizon_scoped_risk(
        nodes: &[serde_json::Value],
        edges: &[serde_json::Value],
        child_of: impl Fn(Period) -> (Vec<WorkflowNode>, Vec<WorkflowEdge>),
    ) -> Result<(), String> {
        let find = |id: &str| nodes.iter().find(|n| n["id"].as_str() == Some(id));
        let has_edge = |src: &str, dst: &str| {
            edges
                .iter()
                .any(|e| e["source"].as_str() == Some(src) && e["target"].as_str() == Some(dst))
        };
        let mapping = |node: &serde_json::Value, key: &str| -> String {
            node.pointer(&format!("/config/input_mapping/{key}"))
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string()
        };

        let pm = find("portfolio-mgr").ok_or_else(|| "缺 portfolio-mgr 节点".to_string())?;
        for (suffix, camel, snake) in B1_TIER_KEYS {
            let node_id = format!("cls-risk-level-{suffix}");
            // v140：风险节点在**档子模板**里 ⇒ 判据面随之换成 typed 子节点（父图 JSON 里查不到它，
            // 而且「父图查不到」本身也是这条门要断言的事之一，见下面的 absence 检查）。
            let period = Period::ALL
                .into_iter()
                .find(|p| p.as_str() == snake)
                .ok_or_else(|| format!("B1_TIER_KEYS 的 {snake} 不在 Period 权威表里"))?;
            let (child_nodes, child_edges) = child_of(period);
            let n = child_nodes
                .iter()
                .find_map(|node| match node {
                    WorkflowNode::Code(c) if c.base.id == node_id => Some(c),
                    _ => None,
                })
                .ok_or_else(|| {
                    format!("档子模板 {snake} 里缺按档风险节点 {node_id} ⇒ v140 的搬动没落地")
                })?;
            let rmap = |key: &str| n.config.input_mapping.get(key).cloned().unwrap_or_default();
            if !n.config.code.contains("DEEP_DISPLACEMENT_RATIO") {
                return Err(format!(
                    "{node_id} 的 code 不含按档深度判据 —— include_str! 拿到的是旧版 risk-level.rhai？"
                ));
            }
            // 子图内的时序：风险节点 → 本档分支（它自己无入边 = 与评分节点并行，只读扇出传入的 t-risk）
            let want_child_edge = format!("{node_id} -> pm-h-{suffix}");
            if !child_edges
                .iter()
                .any(|e| e.source == node_id && e.target == format!("pm-h-{suffix}"))
            {
                return Err(format!(
                    "档子模板 {snake} 缺边 {want_child_edge} ⇒ 分支会在风险档算出来之前起跑"
                ));
            }
            let want_depth =
                format!("t-risk.result.content.stockRiskProfile.riskWindows.{camel}.drawdownDepth");
            let got_depth = rmap("risk_drawdown_depth");
            if got_depth != want_depth {
                return Err(format!(
                    "{node_id} 的 risk_drawdown_depth = {got_depth}，应为 {want_depth} \
                     —— 读错档就等于四份复制"
                ));
            }
            let want_days =
                format!("t-risk.result.content.stockRiskProfile.riskWindows.{camel}.windowDays");
            let got_days = rmap("risk_window_days");
            if got_days != want_days {
                return Err(format!(
                    "{node_id} 的 risk_window_days = {got_days}，应为 {want_days}"
                ));
            }
            for key in B1_GLOBAL_KEYS {
                let p = rmap(key);
                if p.is_empty() {
                    return Err(format!("{node_id} 缺全局键 {key}（七条轴必须齐）"));
                }
                if p.contains("riskWindows") {
                    return Err(format!(
                        "{node_id} 的 {key} 指向 riskWindows（本版只有 depth/windowDays 两键按档，\
                         其余七条仍须 60 日全局 —— 声明与事实不能分叉）"
                    ));
                }
            }

            // ⚠ 分支节点 id 用 **kebab 后缀**（`pm-h-ultra-short`），主链注入键用 **snake**
            //   （`overall_risk_ultra_short`）—— 同一档两种拼写，写错一族就是「门找不到节点」
            //   （实测首版按 snake 拼 `pm-h-ultra_short` ⇒ 当场红，属门的 bug 不是图的 bug）。
            let branch_id = format!("pm-h-{suffix}");
            let branch = find(&branch_id).ok_or_else(|| format!("缺分支节点 {branch_id}"))?;
            // v140 的出口键：主链读的不再是节点，而是**分支行带回的那一格**。
            let want_cat = format!("{branch_id}.result.riskCategory");
            if branch["type"].as_str() != Some("subWorkflow") {
                return Err(format!(
                    "{branch_id} 应是 SubWorkflow 扇出节点，实为 {:?}",
                    branch["type"]
                ));
            }
            // 扇出必须把 `t-risk` 以恒等键传进子快照（风险节点在子图里读的就是它）。
            // `map_inputs` 是严格的（`subworkflow_executor.rs:97-105`）⇒ 少这个键 = 整档子执行硬错。
            if mapping(branch, "t-risk") != "t-risk" {
                return Err(format!(
                    "{branch_id} 的 input_mapping 里 t-risk = {:?}，应为恒等映射 \"t-risk\" \
                     —— 子模板里的按档风险节点取不到输入",
                    mapping(branch, "t-risk")
                ));
            }
            if !has_edge("t-risk", &branch_id) {
                return Err(format!("缺供给边 t-risk → {branch_id}（子图内的风险节点靠它供数）"));
            }
            // 搬干净了的正面断言：父图**不应**再有任何 `cls-risk-level-<档>` 节点或以它为端点的边。
            // 留着就是「两份权威」—— 父图那份照样会跑出一个没人读的风险档（v140 之前的形态）。
            if find(&node_id).is_some() {
                return Err(format!(
                    "父图仍有 {node_id} 节点 ⇒ v140 只搬了一半（子模板里也有一份）"
                ));
            }
            for (src, dst) in [
                ("t-risk", node_id.as_str()),
                (node_id.as_str(), branch_id.as_str()),
                (node_id.as_str(), "portfolio-mgr"),
            ] {
                if has_edge(src, dst) {
                    return Err(format!("父图仍有以 {node_id} 为端点的边 {src} → {dst}"));
                }
            }
            let pm_key = format!("overall_risk_{snake}");
            if mapping(pm, &pm_key) != want_cat {
                return Err(format!(
                    "portfolio-mgr 的 {pm_key} = {}，应为 {want_cat}（主链按所选档 switch 的四格之一没接 \
                     ⇒ 按档风险收紧整条静默退役）",
                    mapping(pm, &pm_key)
                ));
            }
        }

        let g =
            find("cls-risk-level").ok_or_else(|| "全局 cls-risk-level 节点应保留".to_string())?;
        for key in ["risk_drawdown_depth", "risk_window_days"] {
            if !mapping(g, key).is_empty() {
                return Err(format!(
                    "全局节点不应注入按档键 {key} —— 它供 research-mgr 上下文与主链回退分支，\
                     要的是 60 日整票档；注入即等于把它的口径换成按档"
                ));
            }
        }
        Ok(())
    }

    /// v128（B1）：按档风险图必须**真落库**（版本号对 ≠ 内容对 —— 同 v55 那条理由）。
    #[tokio::test]
    async fn seeded_template_carries_horizon_scoped_risk_nodes() {
        let handle = fresh_db().await;
        let db = &handle.conn;
        seed(db).await.expect("种子化应成功");

        let nodes = nodes_of(db).await;
        let edges = edges_of(db).await;
        let clean_children = |p| super::horizon_tier_template::horizon_tier_template_nodes(p);
        if let Err(e) = check_horizon_scoped_risk(&nodes, &edges, clean_children) {
            panic!("v128 按档风险图不完整: {e}");
        }

        // 负控 ①：把 mid 子模板里风险节点的深度键改指 long 那一格（形状合法、档错位）⇒ 必须红。
        // 没有这一条，上面那句「读自己那一格」就是恒真断言（四份复制恰好长那样）。
        // v140 起这条**只能**通过子模板变异来打 —— 判据面已经不在父图 JSON 上了。
        let swapped_mid = |p: Period| {
            let (mut ns, es) = super::horizon_tier_template::horizon_tier_template_nodes(p);
            if p == Period::Mid {
                for n in ns.iter_mut() {
                    if let WorkflowNode::Code(c) = n {
                        if c.base.id == "cls-risk-level-mid" {
                            c.config.input_mapping.insert(
                                "risk_drawdown_depth".to_string(),
                                "t-risk.result.content.stockRiskProfile.riskWindows.long.drawdownDepth"
                                    .to_string(),
                            );
                        }
                    }
                }
            }
            (ns, es)
        };
        assert!(
            check_horizon_scoped_risk(&nodes, &edges, swapped_mid).is_err(),
            "mid 的风险节点改成读 long 那一格后判据仍通过 ⇒ 它检不出「四份复制」这一原始缺陷"
        );

        // 负控 ①′：抽掉子图内「风险节点 → 分支」这条边 ⇒ 必须红（分支会在风险档算出来前起跑）。
        let edgeless_child = |p: Period| {
            let (ns, mut es) = super::horizon_tier_template::horizon_tier_template_nodes(p);
            es.retain(|e| !(p == Period::Mid && e.source == "cls-risk-level-mid"));
            (ns, es)
        };
        assert!(
            check_horizon_scoped_risk(&nodes, &edges, edgeless_child).is_err(),
            "抽掉子图内 mid 的「风险节点 → 分支」边后仍绿 ⇒ 子图时序面不在判据面上"
        );

        // 负控 ②：短线扇出不再传 `t-risk`（改成只传全局风险节点）⇒ 必须红。
        // 键面是 v140 的新失效面：子模板里的风险节点取不到输入 ⇒ 整档子执行硬错。
        let mut reverted = nodes.clone();
        let br = reverted
            .iter_mut()
            .find(|n| n["id"].as_str() == Some("pm-h-short"))
            .expect("夹具：pm-h-short 应存在");
        br["config"]["input_mapping"]["cls-risk-level"] = serde_json::json!("cls-risk-level");
        br["config"]["input_mapping"]
            .as_object_mut()
            .expect("夹具：扇出应有 input_mapping 对象")
            .remove("t-risk");
        assert!(
            check_horizon_scoped_risk(&reverted, &edges, clean_children).is_err(),
            "短线扇出撤掉 t-risk 后仍绿 ⇒ 「子模板按档接线」没被锁住"
        );

        // 负控 ③：删掉父侧供给边 `t-risk → pm-h-mid` ⇒ 必须红（时序竞态是独立失效面）。
        let mut edgeless = edges.clone();
        edgeless.retain(|e| {
            !(e["source"].as_str() == Some("t-risk") && e["target"].as_str() == Some("pm-h-mid"))
        });
        assert!(
            check_horizon_scoped_risk(&nodes, &edgeless, clean_children).is_err(),
            "撤掉 t-risk → pm-h-mid 的边后仍绿 ⇒ 边面不在判据面上"
        );

        // 负控 ④：把主链某一格的出口键改回旧的 `cls-risk-level-<档>.result.category`
        // （父图已无该节点 ⇒ 变量永不到货）⇒ 必须红。
        let mut stale_pm = nodes.clone();
        let pm = stale_pm
            .iter_mut()
            .find(|n| n["id"].as_str() == Some("portfolio-mgr"))
            .expect("夹具：portfolio-mgr 应存在");
        pm["config"]["input_mapping"]["overall_risk_mid"] =
            serde_json::json!("cls-risk-level-mid.result.category");
        assert!(
            check_horizon_scoped_risk(&stale_pm, &edges, clean_children).is_err(),
            "主链读端留在旧路径仍判通过 ⇒ 这就是「按档风险静默退役」的复现，门必须拦住"
        );
    }

    /// v53 / v54 两批的**落库内容**判据（只验版本号是不够的 —— 版本号对 ≠ 内容对）。
    ///
    /// 这两条与 `output/verify-v59-seed.mjs` 的 ⑥/⑦ 是**同一组谓词**：
    /// 那个脚本面向生产 PG，本测试面向临时 SQLite。两者覆盖同一组谓词 ⇒
    /// 脚本的**正向**（已落库）行为由本测试在 CI 里持续保证，
    /// 它的**反向**（未落库）行为由对 v58 生产库的负对照实跑保证
    /// （实测 7 项判据全数报警，见 `output/_verify-negctl.txt`）。
    ///
    /// 单独成条而不并入上一条的理由：失败时**测试名直接指出是哪一批**，
    /// 不必再从断言消息里反推。
    #[tokio::test]
    async fn seeded_template_carries_v53_and_v54_changes() {
        let handle = fresh_db().await;
        let db = &handle.conn;
        seed(db).await.expect("种子化应成功");

        let nodes = nodes_of(db).await;
        let raw = serde_json::to_string(&nodes).expect("nodes 应可序列化");

        // ── v53：`detect_earnings_surprise` 的 ToolDef 契约（`consensus_eps_is_estimated`
        //    入参）必须保持 —— 判据锚在 seed 侧定义。
        //    2026-09-20：**产品决策反转**（§7.3 原 v53 定「不接」→ 现接入）——
        //    该工具已挂到 fundamentals-analyst 节点，nodes 序列化**应**出现该入参。
        //    （原「不接」决策见 `AUDIT-codebase-review-roadmap-2026-09-19.md` §7.3。）
        assert!(
            raw.contains("consensus_eps_is_estimated"),
            "模板 nodes 应含 `consensus_eps_is_estimated`（2026-09-20 产品决策反转：\
             detect_earnings_surprise 已接给 fundamentals-analyst）；缺失即说明接线未生效"
        );

        // ── v54：6 条悬空 `input_mapping` 必须已从 portfolio-mgr 删除 ──
        //    「悬空」判据 = 该键在 `portfolio-mgr.rhai` 全文零引用（纯白注入）。
        const V54_DELETED_KEYS: [&str; 6] = [
            "risk_gross_margin",
            "trader_action",
            "trader_data_gaps",
            "trader_position_pct",
            "trader_stop_loss_pct",
            "trader_take_profit_pct",
        ];
        let pm = nodes
            .iter()
            .find(|n| n.get("id").and_then(|v| v.as_str()) == Some("portfolio-mgr"))
            .expect("模板里应有 portfolio-mgr 节点");
        let mapping =
            pm.pointer("/config/input_mapping").expect("portfolio-mgr 应有 config.input_mapping");
        let obj = mapping.as_object().expect("input_mapping 应是对象");
        let left: Vec<&str> =
            V54_DELETED_KEYS.iter().copied().filter(|k| obj.contains_key(*k)).collect();
        assert!(
            left.is_empty(),
            "v54 未落库：portfolio-mgr 的 input_mapping 仍含悬空键 {left:?}\
             （这些键在脚本全文零引用，属纯白注入）"
        );
    }

    /// v143：**种子落库的主图必须零悬空边**（整图口径，不是逐条定点）。
    ///
    /// 现网实测先后撞到两条，症状都是用户点「开始分析」报
    /// `创建工作流失败: Node 'X' depends on non-existent 'a-…'`（`dag_store` 建图校验整图拒绝）：
    /// `data-quality ← a-market-analyst`（v142 只做了这一处的定点剔除）、
    /// `t-dragon-tiger-data ← a-hot-money`（库里 v142 的 `edges` 至今仍在）。
    /// 定点修法的失效面正是「下一处还按 base id 盲补边」，故这里按**每一条边**断言。
    #[tokio::test]
    async fn seeded_main_graph_has_no_dangling_edges() {
        let handle = fresh_db().await;
        let db = &handle.conn;
        seed(db).await.expect("种子化应成功（兜底对非分析师类悬空边会拒绝播种）");

        let node_ids: std::collections::HashSet<String> = nodes_of(db)
            .await
            .iter()
            .filter_map(|n| n.get("id").and_then(|v| v.as_str()).map(str::to_string))
            .collect();
        assert!(!node_ids.is_empty(), "前置：nodes 应能解析出 id");

        let mut dangling: Vec<String> = Vec::new();
        for e in edges_of(db).await {
            let id = e.get("id").and_then(|v| v.as_str()).unwrap_or("?");
            let src = e.get("source").and_then(|v| v.as_str()).unwrap_or("");
            let tgt = e.get("target").and_then(|v| v.as_str()).unwrap_or("");
            if !node_ids.contains(src) || !node_ids.contains(tgt) {
                dangling.push(format!("{id}（{src}→{tgt}）"));
            }
        }
        assert!(dangling.is_empty(), "主图存在悬空边 ⇒ create_workflow 整图拒绝：{dangling:?}");

        // 前提样本：库里的主图**确有**指向分析师的边被剔除过，否则本测试是在空集上恒真。
        // 判据取现场读数：`a-hot-money` 既不在节点集、也不该以任何形态出现在边里。
        assert!(
            !node_ids.contains("a-hot-money"),
            "前置：B-2b 后主图不应再有 `a-hot-money` 节点（本测试的剔除对象）"
        );
    }

    /// v144：**种子落库的主图必须无环**（引擎 `create_workflow` 的第三条校验，前两条已各有一道门）。
    ///
    /// 为什么单独一条：`dag_store`/`WorkEngine::create_workflow_inner` 的校验顺序是
    /// 重复 id → 悬空边 → **Kahn 环检测**，所以悬空边报错会**盖住**环 —— v143 修掉悬空边后，
    /// 用户端立刻换成 `创建工作流失败: Cycle detected in workflow`（现网实测：库里那份 v143 有
    /// 45 个节点成环，两条环都经过 `value-investor--档`）。同一条链上的三道校验必须三道门，
    /// 少一道就是「修一条露一条」。
    #[tokio::test]
    async fn seeded_main_graph_is_acyclic() {
        let handle = fresh_db().await;
        let db = &handle.conn;
        seed(db).await.expect("种子化应成功");

        let node_ids: Vec<String> = nodes_of(db)
            .await
            .iter()
            .filter_map(|n| n.get("id").and_then(|v| v.as_str()).map(str::to_string))
            .collect();
        let index: std::collections::HashMap<String, usize> =
            node_ids.iter().enumerate().map(|(i, id)| (id.clone(), i)).collect();
        let mut adj: Vec<Vec<usize>> = vec![Vec::new(); node_ids.len()];
        let mut indeg: Vec<usize> = vec![0; node_ids.len()];
        for e in edges_of(db).await {
            let (Some(s), Some(t)) = (
                e.get("source").and_then(|v| v.as_str()),
                e.get("target").and_then(|v| v.as_str()),
            ) else {
                continue;
            };
            // 悬空边由上一条门负责；这里跳过，免得两道门互相顶掉读数
            let (Some(&si), Some(&ti)) = (index.get(s), index.get(t)) else { continue };
            adj[si].push(ti);
            indeg[ti] += 1;
        }
        // Kahn：出队数 == 节点数 ⇒ 无环
        let mut queue: Vec<usize> = (0..node_ids.len()).filter(|i| indeg[*i] == 0).collect();
        let mut popped = 0usize;
        let mut head = 0;
        while head < queue.len() {
            let u = queue[head];
            head += 1;
            popped += 1;
            for &v in &adj[u] {
                indeg[v] -= 1;
                if indeg[v] == 0 {
                    queue.push(v);
                }
            }
        }
        assert_eq!(
            popped,
            node_ids.len(),
            "主图存在环：{} 个节点未被 Kahn 消解（引擎会报 Cycle detected in workflow），\
             残留集 = {:?}",
            node_ids.len() - popped,
            (0..node_ids.len())
                .filter(|i| indeg[*i] > 0)
                .map(|i| node_ids[i].as_str())
                .collect::<Vec<_>>()
        );
    }

    /// 兜底函数**两条分支**都要直接打得到，且带「看着像分析师但不该放行」的负控。
    /// 拆成 `prune_dangling_analyst_edges` 就是为了这一条：`Err` 那一支若留在种子函数体内，
    /// 要触发得构造一整张坏图，实际等于没有测试。
    #[test]
    fn dangling_edge_guard_prunes_analysts_and_rejects_others() {
        use axagent_harness::workflow_types::EdgeType;

        fn edge(id: &str, src: &str, tgt: &str) -> axagent_harness::workflow_types::WorkflowEdge {
            axagent_harness::workflow_types::WorkflowEdge {
                id: id.into(),
                source: src.into(),
                source_handle: None,
                target: tgt.into(),
                target_handle: None,
                edge_type: EdgeType::Direct,
                label: None,
            }
        }

        let present: std::collections::HashSet<&str> =
            ["trigger", "t-dragon-tiger-data", "data-quality"].into_iter().collect();
        let migrated = ["a-hot-money", "a-market-analyst"];

        // ① 正断言：端点是「迁进档子模板的分析师 base id」⇒ 剔除并回报清单，不报错
        let mut edges = vec![
            edge("e-ok", "trigger", "data-quality"),
            edge("e-dt", "t-dragon-tiger-data", "a-hot-money"),
            edge("e-dq", "data-quality", "a-market-analyst"),
        ];
        let pruned = super::seed_stock_analysis::prune_dangling_analyst_edges(
            &present, &mut edges, &migrated,
        )
        .expect("分析师类悬空边应被剔除而不是报错");
        assert_eq!(pruned.len(), 2, "两条分析师悬空边都应进剔除清单：{pruned:?}");
        assert_eq!(edges.len(), 1, "只应留下两端齐备的那条边");
        assert_eq!(edges[0].id, "e-ok");

        // ② 负控 A：悬空的另一端**不在**分析师清单里 ⇒ 必须 Err（否则新写错的边被静默吃掉）
        let mut edges2 = vec![edge("e-bad", "data-quality", "t-nonexistent")];
        let err = super::seed_stock_analysis::prune_dangling_analyst_edges(
            &present,
            &mut edges2,
            &migrated,
        )
        .expect_err("非分析师类悬空边必须拒绝播种");
        assert!(err.contains("e-bad"), "报错应点名那条边：{err}");

        // ③ 负控 B：形态像分析师（前缀 `a-`）但不在清单里 ⇒ 同样必须 Err。
        //    这一条锁住「判据是白名单而不是前缀匹配」—— 用前缀判的话 v135 之后任何
        //    `a-*` 拼错都会被兜底当成「迁走的分析师」静默吞掉。
        let mut edges3 = vec![edge("e-lookalike", "t-dragon-tiger-data", "a-hot-mony")];
        assert!(
            super::seed_stock_analysis::prune_dangling_analyst_edges(
                &present,
                &mut edges3,
                &migrated
            )
            .is_err(),
            "`a-hot-mony` 只与分析师名一字之差，不在清单里就必须报错"
        );

        // ④ 悬空在 **source** 侧同样要判（`present.contains(source)` 那一半分支）
        let mut edges4 = vec![edge("e-src", "a-hot-money", "data-quality")];
        let pruned4 = super::seed_stock_analysis::prune_dangling_analyst_edges(
            &present,
            &mut edges4,
            &migrated,
        )
        .expect("source 侧的分析师悬空边也应被剔除");
        assert_eq!(pruned4.len(), 1, "{pruned4:?}");
        assert!(edges4.is_empty());
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// B-2b #36（v135）：主图四个 `pm-h-<档>` 扇出 ↔ 四张档子模板 的**键齐备性**守门
//
// ## 为什么必须在改图的同一批里建（PLAN §九十 / §九十一(3) 步骤 3）
//
// 子执行只拿到父扇出 `input_mapping` **target 键**那一份变量快照，而
// `subworkflow_executor::map_inputs`（`crates/rt-workflow/src/work_engine/executors/subworkflow_executor.rs:97-105`）
// 是**严格**的：路径取不到就 `Variable 'x' not found` ⇒ 整个扇出节点硬错。
// 也就是说「漏传一个键」的报错点天然在**运行期**，而本仓的四档分支恰好大量引用
// 运行期才存在的名字（`horizon_branch_json` 由 hooks 注入、`a-hot-money--<档>` 由
// 播种按档生成）。判据必须由代码现算 needs，不能靠人抄清单 —— 抄漏的形态见 §八十 的 A1。
//
// ## 它守什么
//
// - ① **零缺口**：每张模板现算的 needs ⊆ 对应扇出传入的键；
// - ② **双向一致**：`fanouts`（图里真实存在的扇出目标）与 `registered`（本门登记了数据源的
//   模板）必须相等 —— 新增扇出要登记数据源，撤扇出要撤登记（否则判据对它没有数据源，
//   「没有缺口」是假的）；
// - ③ **无盲区**：`node_var_io` 必须认得档模板用到的每种节点类型（盲区让 needs **少报**，
//   于是本门绿而运行期红）；
// - ④ **扇出指向本档模板**：`pm-h-<档>` 的 `sub_workflow_id` 必须等于 `stock-horizon-<档>`
//   （名字由 `horizon_tier_template_id` 现推，不手抄）；
// - ⑤ **四行真落库**且版本 = `HORIZON_TIER_TEMPLATE_VERSION`（版本号对 ≠ 内容对，同 v55 那条理由）。
//
// 同批并到本模块的第二条判据（⑥）：**ToolNode 的参数必须有源**
// （`tool_node_argument_sources_exist` + 两侧负控）。它与扇出无关，但共用同一份
// 「本图产出 / 外部传入」集合与同一个 `workflow_fanout_audit`，且它要拦的正是
// §九十二(4) 那条让四档评分恒等于日线的死参数 —— v135 把 `period` 接成活变量之后，
// 没有这条门，下一次写错变量名照样全绿。
//
// 负控一条：从某个扇出里抽掉一个**子模板真要用**的键 ⇒ 必须由**同一个** `audit_fanouts`
// 报出缺口（另写一份比较公式的负控证不到真判据）。
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod horizon_tier_fanout_tests {
    use super::horizon_tier_template::{
        HORIZON_TIER_TEMPLATE_VERSION, horizon_tier_template_id, horizon_tier_template_nodes,
        seed_horizon_tier_templates,
    };
    use super::seed_stock_analysis::{
        SOURCE_TEMPLATE_ID, TEMPLATE_VERSION, seed_stock_analysis_workflow_template,
    };
    use crate::workflow_fanout_audit::{
        audit_fanouts, external_reads, external_reads_for_kind, graphs_to_audit,
    };
    use axagent_harness::holding_period::Period;
    use axagent_harness::workflow_types::WorkflowNode;
    use axagent_rt_workflow::work_engine::node_type_of;
    use sea_orm::{DatabaseConnection, EntityTrait};

    async fn fresh_db() -> axagent_dao::db::DbHandle {
        axagent_dao::db::create_test_pool().await.expect("建临时测试库失败")
    }

    async fn row_nodes(db: &DatabaseConnection, id: &str) -> Vec<WorkflowNode> {
        let model = axagent_entities::workflow_template::Entity::find_by_id(id)
            .one(db)
            .await
            .expect("查模板失败")
            .unwrap_or_else(|| panic!("模板 `{id}` 应已存在"));
        serde_json::from_str(&model.nodes).expect("nodes 应是 JSON 数组")
    }

    async fn row_version(db: &DatabaseConnection, id: &str) -> i32 {
        axagent_entities::workflow_template::Entity::find_by_id(id)
            .one(db)
            .await
            .expect("查模板失败")
            .unwrap_or_else(|| panic!("模板 `{id}` 应已存在"))
            .version
    }

    /// 把四张档模板从库里读回来（**读 DB 而不是直接调 builder**）：
    /// 判据要证的是「落库那四行」与主图扇出对得上，不是「代码里两份定义互相对得上」。
    async fn seeded_tier_templates(db: &DatabaseConnection) -> Vec<(String, Vec<WorkflowNode>)> {
        let mut out = Vec::new();
        for period in Period::ALL {
            let id = horizon_tier_template_id(period);
            let nodes = row_nodes(db, &id).await;
            out.push((id, nodes));
        }
        out
    }

    /// 正对照 ①②③④⑤。
    #[tokio::test]
    async fn tier_fanout_inputs_are_complete() {
        let handle = fresh_db().await;
        let db = &handle.conn;
        seed_stock_analysis_workflow_template(db).await.expect("主图种子应成功");
        seed_horizon_tier_templates(db).await.expect("四张档子模板种子应成功");

        // ⑤ 四行落库 + 版本与主图同代。本代是**编译期**保证
        //   （`HORIZON_TIER_TEMPLATE_VERSION` 直接绑 `seed_stock_analysis::TEMPLATE_VERSION`），
        //   这里验的是「运行期真的把那个版本写进了 DB 四行」—— 版本号对 ≠ 内容对（v55 那条理由），
        //   所以两条各管一格：不在此处再写死 135（那只会让下一次升版多一处必填项）。
        for period in Period::ALL {
            let id = horizon_tier_template_id(period);
            assert_eq!(
                row_version(db, &id).await,
                HORIZON_TIER_TEMPLATE_VERSION,
                "档模板 {id} 落库版本应为 {HORIZON_TIER_TEMPLATE_VERSION}"
            );
            assert_eq!(
                HORIZON_TIER_TEMPLATE_VERSION, TEMPLATE_VERSION,
                "档模板与主图必须同代 —— 扇出的键集是按档模板内容算的"
            );
        }

        let main = row_nodes(db, SOURCE_TEMPLATE_ID).await;
        let rows = seeded_tier_templates(db).await;
        let templates: std::collections::BTreeMap<&str, Vec<WorkflowNode>> =
            rows.iter().map(|(id, nodes)| (id.as_str(), nodes.clone())).collect();
        let graphs = graphs_to_audit(SOURCE_TEMPLATE_ID, &main, &templates);
        let audit = audit_fanouts(&graphs, &templates);

        // ① 零缺口
        assert!(audit.gaps.is_empty(), "扇出键不齐备（子模板要读而父没传）：{:?}", audit.gaps);
        // ② 双向一致
        assert!(
            audit.unregistered.is_empty(),
            "这些扇出指向未登记的子模板，判据对它们没有数据源：{:?}",
            audit.unregistered
        );
        assert_eq!(
            audit.fanouts.len(),
            4,
            "主图的图内扇出应恰为四个（{:?}）—— 多出来的是新增扇出，少了的是扇出被撤却没同步撤登记",
            audit.fanouts
        );
        assert_eq!(
            audit.fanouts, audit.registered,
            "登记的子模板集合与实际扇出集合必须**双向**一致"
        );
        // ③ 无盲区
        assert!(
            audit.blind_kinds.is_empty(),
            "node_var_io 未覆盖这些节点类型 {:?} ⇒ needs 会漏键",
            audit.blind_kinds
        );
        // ④ 每个扇出指向**本档**模板
        for period in Period::ALL {
            let want = horizon_tier_template_id(period);
            let node_id = format!("pm-h-{}", period.as_str().replace('_', "-"));
            let got = main
                .iter()
                .find_map(|n| match n {
                    WorkflowNode::SubWorkflow(s) if s.base.id == node_id => {
                        Some(s.config.sub_workflow_id.clone())
                    },
                    _ => None,
                })
                .unwrap_or_else(|| panic!("主图缺扇出节点 {node_id}"));
            assert_eq!(got, want, "{node_id} 应指向本档模板");
        }
        println!(
            "审计 {} 张图；图内扇出 {} 个（{}）；system_* 排除 {} 个",
            graphs.len(),
            audit.fanouts.len(),
            audit.fanouts.iter().cloned().collect::<Vec<_>>().join(","),
            audit.excluded_system.len()
        );
    }

    /// 负控：从某个扇出里抽掉一个子模板**真要用**的键 ⇒ 同一个 `audit_fanouts` 必须报缺口。
    ///
    /// 抽的键挑 `t-lockup-data`（中档模板的 `lockup_float_ratio` 腿读它）—— 它是
    /// 「缺席分两种」那条设计里的可选上游，父侧不传它就等于整档硬错，正是本门存在的理由。
    #[tokio::test]
    async fn tier_fanout_missing_key_is_caught() {
        let handle = fresh_db().await;
        let db = &handle.conn;
        seed_stock_analysis_workflow_template(db).await.expect("主图种子应成功");
        seed_horizon_tier_templates(db).await.expect("档模板种子应成功");

        let mut main = row_nodes(db, SOURCE_TEMPLATE_ID).await;
        let rows = seeded_tier_templates(db).await;
        let templates: std::collections::BTreeMap<&str, Vec<WorkflowNode>> =
            rows.iter().map(|(id, nodes)| (id.as_str(), nodes.clone())).collect();

        let mut removed_from: Option<String> = None;
        for node in &mut main {
            if let WorkflowNode::SubWorkflow(s) = node
                && s.config.input_mapping.remove("t-lockup-data").is_some()
            {
                removed_from = Some(s.base.id.clone());
                break;
            }
        }
        let removed_from =
            removed_from.expect("主图扇出里没有可拆的 t-lockup-data 键 ⇒ 本负控失去前提");
        let graphs = graphs_to_audit(SOURCE_TEMPLATE_ID, &main, &templates);
        let audit = audit_fanouts(&graphs, &templates);
        assert!(
            audit.gaps.iter().any(|(_, _, parent, missing)| parent == &removed_from
                && missing.contains("t-lockup-data")),
            "抽掉 {removed_from} 的 t-lockup-data 后仍报「无缺口」⇒ 齐备性判据没电（抽取面或比较面失效）"
        );
    }

    /// 形状锁：每张档模板必须恰好是「常量 → 本档评分 → **本档按档风险** → 本档分支 → 终值」
    /// 五个节点（v140 起；搬动前是四节点 + 风险节点留父图）。
    ///
    /// 为什么要单独一条：`audit_fanouts` 检的是**键**齐不齐，检不出**节点**被删/被换 ——
    /// 例如有人把评分节点从模板里摘掉、改成父侧再传一份，键面照样绿，而四档重新变成
    /// 同一份输入（§九十一(1) 的共享上游理由）。评分节点 id 取权威 `Period::scoring_node_id`
    /// （不手抄），所以档↔尺度↔节点一旦串了，这条与 `check-tier-purity` 的 R5 各红一次。
    /// 风险节点 id 同样**不手抄**：由 `Period::as_str()` 现推 kebab ⇒ 档位拼写只有一个来源。
    #[test]
    fn tier_template_shape_is_the_declared_five_nodes() {
        for period in Period::ALL {
            let (nodes, _) = horizon_tier_template_nodes(period);
            let ids: Vec<String> = nodes.iter().map(|n| n.base_id().to_string()).collect();
            assert_eq!(
                ids,
                vec![
                    "const-scoring-period".to_string(),
                    period.scoring_node_id().to_string(),
                    format!("cls-risk-level-{}", period.as_str().replace('_', "-")),
                    format!("pm-h-{}", period.as_str().replace('_', "-")),
                    "end".to_string(),
                ],
                "档模板 {ids:?} 的形状与本批声明的五节点形状不符 ⇒ 播种行与扇出键集要一起重算"
            );
        }
    }

    /// 运行期注入的变量名 —— **从注入点现场推导**，不在这里手抄第二份清单。
    ///
    /// 为什么必须算进「有源」：`stock_code` 这类名字**不在**模板变量表里（面板没有它、
    /// DB 的 `variables` 列也没有它），它们由 `stock_workflow/hooks.rs` / `core.rs` 在每次
    /// run 前 push 进 `merged_vars`。在这里手写一份白名单，就会与
    /// `src/components/settings/StockAnalysisConfigPanel.test.tsx` 的 `RUNTIME_INJECTED`
    /// 成了两份各自漂移的副本（本仓「清单由单一权威渲染 + 注入」的纪律）；真正的权威就是
    /// 那两处 `Variable { name: … }` / `("<名>", json!(…))` 的**构造点** ⇒ 直接扫它。
    /// 漂移方向也顺带被封住：hooks 摘掉某个注入 ⇒ 该名字自动离开集合 ⇒ 仍拿它当工具参数的图
    /// 立刻红，而不是继续绿着走工具缺省值。
    fn runtime_injected_names() -> std::collections::BTreeSet<String> {
        use regex::Regex;
        let mut out = std::collections::BTreeSet::new();
        for src in
            [include_str!("../stock_workflow/hooks.rs"), include_str!("../stock_workflow/core.rs")]
        {
            for re in [
                r#"name:\s*"([a-z_][a-z0-9_]*)"\.into\(\)"#,
                r#"\(\s*"([a-z_][a-z0-9_]*)"\s*,\s*json!\("#,
            ] {
                for cap in Regex::new(re).expect("正则应合法").captures_iter(src) {
                    out.insert(cap[1].to_string());
                }
            }
        }
        // 扫描面自证：推导失效时判据会「全红」（易被误读成图坏了），不会静默放行。
        assert!(
            out.len() > 10 && out.contains("stock_code"),
            "运行期注入名只推到 {} 个（含 stock_code? {}）⇒ 推导姿势已失效，本门不可信",
            out.len(),
            out.contains("stock_code")
        );
        out
    }

    /// 判据本体：**ToolNode 参数指向的变量必须「有源」**，返回问题清单（空 = 绿）。
    ///
    /// 「有源」= 本图自己产出（节点 id 或 `output_var`）**或** 在 `allowed_external` 里
    /// （主图 = 种子变量表的变量名；档模板 = 父扇出传进来的键）**或** 运行期注入
    /// （[`runtime_injected_names`] 现推）；引擎每个 run 自动注入的四个工作流级变量另算
    /// （名单直接取引擎侧常量，不在这里抄）。
    ///
    /// 为什么单列一条而不是复用 `audit_fanouts` 的 needs：工具**参数名**不是工作流变量，
    /// 它不会出现在「子模板要读什么、父得传什么」那张面上 ⇒ 死参数要从**声明读取**这一侧
    /// 按节点种类切开看（PLAN §九十二(4) 的实锤：`("period","hourly")` 让四档评分恒等于日线，
    /// 而 `cargo check` / `clippy` / 既有全套门**全绿**）。
    /// 公式与 needs 同源（`workflow_fanout_audit::produced_names`），不抄第二份。
    fn tool_arg_problems(
        label: &str,
        nodes: &[WorkflowNode],
        allowed_external: &std::collections::BTreeSet<String>,
    ) -> Vec<String> {
        let mut blind = std::collections::BTreeSet::new();
        let needs = external_reads_for_kind(nodes, "tool", &mut blind);
        // 本条判据的**载体**必须被 `node_var_io` 认得，否则 needs 会少报而门绿着空转。
        assert!(
            !blind.contains("tool"),
            "{label} 的 ToolNode 未被 node_var_io 覆盖 ⇒ 本门对它没有数据源"
        );
        // 其余未覆盖种类如实打印而不是判红：本门只按 `tool` 切读取面，别的种类覆盖与否
        // 不改变结论；而 `produced_names` 对它们是盲区 ⇒ 影响方向是**多报**
        // （某个 Switch/Parallel 写出的变量没进自产集 ⇒ 被当成无源），不是漏报。
        let disclosed: Vec<&str> = blind.iter().copied().filter(|k| *k != "tool").collect();
        if !disclosed.is_empty() {
            println!(
                "[tool 参数有源] {label}：node_var_io 未覆盖这些种类 {disclosed:?}（不影响本门，误差方向是多报）"
            );
        }
        let injected = runtime_injected_names();
        needs
            .iter()
            .filter(|n| {
                !allowed_external.contains(*n)
                    && !injected.contains(*n)
                    && !crate::workflow_fanout_audit::engine_injected_names().contains(&n.as_str())
            })
            .map(|n| {
                format!(
                    "{label}：ToolNode 的参数指向无源的变量 {n:?} \
                     ⇒ 工具会静默走自己的缺省值（不报错），这就是 §九十二(4) 那条死参数族"
                )
            })
            .collect()
    }

    /// 主图那一行的种子变量名（判据的「有源」集合之一）。
    async fn seeded_variable_names(
        db: &DatabaseConnection,
        id: &str,
    ) -> std::collections::BTreeSet<String> {
        let model = axagent_entities::workflow_template::Entity::find_by_id(id)
            .one(db)
            .await
            .expect("查模板失败")
            .unwrap_or_else(|| panic!("模板 `{id}` 应已存在"));
        let raw = model.variables.unwrap_or_default();
        let arr: Vec<serde_json::Value> =
            serde_json::from_str(&raw).expect("variables 应是 JSON 数组");
        let names: std::collections::BTreeSet<String> =
            arr.iter().filter_map(|v| v["name"].as_str().map(str::to_string)).collect();
        // 扫描面自证：变量表空 ⇒ `allowed_external` 空 ⇒ 判据会「全红」而不是「全绿」，
        // 但**解析姿势错**同样表现为空 —— 那时长红会被误读成「图坏了」。所以给下限。
        assert!(names.len() > 20, "种子变量表只解析到 {} 个名字 ⇒ 判据的读面不可信", names.len());
        names
    }

    /// 正对照：主图 + 四张档模板的 ToolNode 参数全部有源。
    #[tokio::test]
    async fn tool_node_argument_sources_exist() {
        let handle = fresh_db().await;
        let db = &handle.conn;
        seed_stock_analysis_workflow_template(db).await.expect("主图种子应成功");
        seed_horizon_tier_templates(db).await.expect("档模板种子应成功");

        let main = row_nodes(db, SOURCE_TEMPLATE_ID).await;
        let vars = seeded_variable_names(db, SOURCE_TEMPLATE_ID).await;
        let tool_nodes = main.iter().filter(|n| node_type_of(n) == "tool").count();
        assert!(tool_nodes >= 10, "主图只数到 {tool_nodes} 个 ToolNode ⇒ 扫描面塌了，本门失去对象");
        let mut problems = tool_arg_problems(SOURCE_TEMPLATE_ID, &main, &vars);

        // 每张档模板的「外部可源集」= 父扇出传进来的键（恒等键 ⇒ 与 needs 同集合）。
        for period in Period::ALL {
            let id = horizon_tier_template_id(period);
            let provided: std::collections::BTreeSet<String> = main
                .iter()
                .find_map(|n| match n {
                    WorkflowNode::SubWorkflow(s) if s.config.sub_workflow_id == id => {
                        Some(s.config.input_mapping.keys().cloned().collect())
                    },
                    _ => None,
                })
                .unwrap_or_else(|| panic!("主图没有指向 {id} 的扇出节点"));
            let nodes = row_nodes(db, &id).await;
            problems.extend(tool_arg_problems(&id, &nodes, &provided));
        }

        assert!(problems.is_empty(), "ToolNode 参数无源：\n{}", problems.join("\n"));
        println!("ToolNode 参数有源核对：主图 {tool_nodes} 个工具节点 + 4 张档模板，全绿");
    }

    /// 负控 A：把档模板评分节点的 `period` 指回一个**不存在**的变量 ⇒ 必须报出它。
    ///
    /// 这条锁的正是 v135 修掉的那个原始缺陷的形状（§九十二(4)：`("period","hourly")` 里
    /// `hourly` 从来不是变量）。没有它，正对照可能只是在「needs 恰好为空」的假象上绿。
    #[tokio::test]
    async fn dead_tool_argument_is_caught_in_tier_template() {
        let (mut nodes, _) = horizon_tier_template_nodes(Period::Mid);
        let provided: std::collections::BTreeSet<String> =
            external_reads(&nodes, &mut std::collections::BTreeSet::new());
        // 前提：未改动前本档全绿（`scoring_period` 由模板内的常量节点自产）。
        assert!(tool_arg_problems("stock-horizon-mid", &nodes, &provided).is_empty());
        let mut patched = false;
        for node in &mut nodes {
            if let WorkflowNode::Tool(t) = node {
                // 写成一个「看起来像变量名、但本图没人产出、父也没传」的串
                t.config.input_mapping.insert("period".into(), "monthlyx".into());
                patched = true;
            }
        }
        assert!(patched, "档模板里没有 ToolNode ⇒ 本负控失去前提");
        let problems = tool_arg_problems("stock-horizon-mid", &nodes, &provided);
        assert!(
            problems.iter().any(|p| p.contains("monthlyx")),
            "把 period 指向不存在的变量后仍不报 ⇒ 死参数判据没电：{problems:?}"
        );
    }

    /// 负控 B：主图同理 —— 把 `t-market-data` 的 `limit` 参数改指一个不存在的变量 ⇒ 必须红。
    ///
    /// 两侧各一条的理由（判据层 4az：命中判据要负控两头各一条）：主图的允许集是**种子变量表**，
    /// 档模板的允许集是**父扇出键** —— 两套输入，任一为空都会让另一侧的负控证不到自己那条。
    #[tokio::test]
    async fn dead_tool_argument_is_caught_in_main_graph() {
        let handle = fresh_db().await;
        let db = &handle.conn;
        seed_stock_analysis_workflow_template(db).await.expect("主图种子应成功");
        let mut main = row_nodes(db, SOURCE_TEMPLATE_ID).await;
        let vars = seeded_variable_names(db, SOURCE_TEMPLATE_ID).await;
        assert!(
            tool_arg_problems(SOURCE_TEMPLATE_ID, &main, &vars).is_empty(),
            "前提被破坏：现网图已有无源参数"
        );
        let mut patched = false;
        for node in &mut main {
            if let WorkflowNode::Tool(t) = node
                && t.base.id == "t-market-data"
            {
                t.config.input_mapping.insert("limit".into(), "kline_limitt".into());
                patched = true;
            }
        }
        assert!(patched, "主图没有 t-market-data 的 ToolNode ⇒ 本负控失去前提");
        let problems = tool_arg_problems(SOURCE_TEMPLATE_ID, &main, &vars);
        assert!(
            problems.iter().any(|p| p.contains("kline_limitt")),
            "把 kline_limit 改成一个不存在的变量名后仍不报 ⇒ 主图侧判据没电：{problems:?}"
        );
    }
}
