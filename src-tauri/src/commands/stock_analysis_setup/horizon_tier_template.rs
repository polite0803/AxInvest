// SPDX-License-Identifier: AGPL-3.0-only

//! 四张「档子模板」的节点与边定义（B-2b，PLAN `PLAN-four-horizon-workflow-alignment.md` §九十一）。
//!
//! 粒度裁定（2026-10-06 用户拍）：整段搬 —— 每档的「评分 + 按档风险 + 分支决策」进子模板，
//! 主图只留四个 SubWorkflow 节点。可搬运的边界与「为什么共享上游留在父图」记在 §九十一(0)：
//! `t-scoring`（日线）/ `t-risk` / `t-limitup-pool` 等是四路共用的取数上游，搬进任一子模板
//! 就会让另外三路各拉一遍同样的数据，且违反「一次分析产四档」的既定设计。
//!
//! ⚠ 为什么是**四份逐字定义**而不是一个循环：`scripts/audit-inject-coverage.mjs` 按
//! `input_mapping: [` 块内的**字面量**配对 Rhai 脚本的 `present()` 面，`check-tier-purity.mjs`
//! 的 R2 按字面量判「档内不读他档」。用 `format!` 拼装源路径会让这两道门**静默失效**
//! （与 `commands/stock_analysis_setup/seed_stock_analysis.rs:5859-5862` 拒绝循环是同一条理由，
//! 同文件 `:4604-4607` 也记着同一条史）。
//! 这里重复的是**节点声明**，不是类型或函数定义，不触 AGENTS.md 禁区 12。
//!
//! ⚠ 步骤 2 当年**留的一处偏离**，已在 v140 补齐（保留原因，因为它是「为什么曾经不是三节点」的史）：
//! §九十一(0) 的可搬运边界把三节点都算进子模板，但普查后 `portfolio-mgr` 有**四个父侧读面**
//! （`seed_stock_analysis.rs` 的 `overall_risk_{ultra_short,short,mid,long}` 四条映射 + 四条供给边，
//! 消费点是 `portfolio-mgr.rhai` 的 `tier_risk_raise`）。子执行只把父扇出节点的
//! `node_id`/`output_var` 双键写回父池（`work_engine/engine/mod.rs:1772-1775`），
//! 于是风险节点一旦进子模板，这四条映射就变成「指向不存在的路径」⇒ `present()` 恒假 ⇒
//! **v128 B1 的按档风险收紧整条静默退役**（正是本仓登记的「配置项空接线」族）。
//!
//! v140 的补齐形态（同一批四处一起改，缺一处就是那次静默退役的复现）：
//!   ① 子模板**多一个节点**（[`tier_risk_node`]，九条映射与父图旧形态逐字相同 ⇒ 数值零变化）；
//!   ② 父扇出的身份键从 `cls-risk-level-<档>` 换成 **`t-risk`**（风险节点吃的仍是父侧 `t-risk` 的产出）；
//!   ③ 分支脚本把读到的本档风险档**回写进决策行**（`riskCategory`）⇒ 双键写回把它带回父池；
//!   ④ 主链四条读面改指 `pm-h-<档>.result.riskCategory`，并**删掉** `cls-risk-level-<档> → portfolio-mgr`
//!      四条父侧边（节点已不在父图；时序由既有的 `pm-h-<档> → portfolio-mgr` 供给边传递覆盖）。

use axagent_harness::holding_period::Period;
use axagent_harness::workflow_types::{
    CodeNode, CodeNodeConfig, DataTransformerNode, DataTransformerNodeConfig, EndNode,
    EndNodeConfig, Position, RetryConfig, ToolNode, ToolNodeConfig, WorkflowEdge, WorkflowNode,
    WorkflowNodeBase,
};
use std::collections::HashMap;

use super::seed_stock_analysis::direct_edge;

/// 档子模板的模板 id（父图扇出节点的 `sub_workflow_id` 与播种行**共用这一个来源**）。
///
/// 命名 `stock-horizon-<档 kebab>`（§九十一(2)）：与逐档分支节点 id 前缀 `pm-h-<档 kebab>` 同形，
/// `check-tier-purity.mjs` 的归属判据可直接复用。档位名不手抄 —— 由 `Period::as_str()` 现推。
pub(crate) fn horizon_tier_template_id(period: Period) -> String {
    format!("stock-horizon-{}", period.as_str().replace('_', "-"))
}

/// 一张档子模板的全部内容：本档尺度常量 → 本档评分节点 → 本档分支决策节点 → 终值节点。
///
/// 节点 id 与主图里**完全一致**（`t-scoring-*` / `pm-h-*`）—— 它们是另一张图，
/// 不与父图节点共存；父图那个 SubWorkflow 节点继续叫 `pm-h-<档>`（§九十一(0) 的双键写回理由：
/// 引擎把节点结果同时写在 `node_id` 与 `output_var` 上，`portfolio-mgr` 读的是 `pm-h-<档>.result`）。
pub(crate) fn horizon_tier_template_nodes(
    period: Period,
) -> (Vec<WorkflowNode>, Vec<WorkflowEdge>) {
    match period {
        Period::UltraShort => ultra_short_tier_template(),
        Period::Short => short_tier_template(),
        Period::Mid => mid_tier_template(),
        Period::Long => long_tier_template(),
    }
}

/// 子图终值节点：把本档分支节点的 `output_var` 选作子执行终值。
///
/// 父侧拿到的形状与今天逐位相同 —— `extract_end_output`（`work_engine/engine/output_builder.rs:41-45`）按
/// **平键**取 `results[var]`，取到的就是那个 CodeNode 的完整结果对象（含 `result` 字段），
/// 于是父图里 `pm-h-<档>.result` 仍指向 Rhai 返回的那棵树。
/// 注意：End 的 `output_var` **不支持点分路径**（`work_engine/executors/end_executor.rs:45` 是 `variables.get(var)`），
/// 所以这里只能写平键 `h_<档>`，不能写 `pm-h-<档>.result`。
fn tier_end_node(decision_output_var: &str, y: f64) -> WorkflowNode {
    WorkflowNode::End(EndNode {
        base: WorkflowNodeBase {
            id: "end".into(),
            title: "本档结论".into(),
            description: Some("把本档分支决策的输出选作子执行终值交回父图".into()),
            position: Position { x: 1560.0, y },
            retry: RetryConfig::default(),
            timeout: None,
            enabled: true,
            parent_id: None,
            compensation: None,
            continue_on_fail: false,
        },
        config: EndNodeConfig { output_var: Some(decision_output_var.into()) },
    })
}

/// 本档尺度常量节点 —— 产出子模板自有的 `scoring_period` 字符串变量。
///
/// 为什么必须有它（PLAN §一○○(1)，根因见 §九十二(4)）：`tool_executor.rs:75-80` 对
/// `input_mapping` 的**值**一律走 `resolve_var_path`，取不到就落 `Value::Null` —— **没有**
/// 「原样当字面量」的兜底。旧种子把 extra 写成 `("period","hourly")`，即「工具参数 period ←
/// **变量** hourly」，而全仓不存在名为 `hourly/weekly/monthly/quarterly` 的工作流变量
/// ⇒ `compute_scoring` 的 `as_str().unwrap_or("daily")` 恒命中缺省 ⇒ 四档评分实际全是日线
/// （§九十二(4) 普查出的既有死参数）。
/// 尺度**不做成面板可调**（`seed_variables.rs` 已记这条裁定，且新增可调参数要满五点对账）⇒
/// 不新增种子变量，改由每张档子模板自己产常量。字面量逐档写死在各模板里（不用
/// `period.scale_key()`），这样「常量与本档尺度是否一致」有字面量可对 —— 那条一致性
/// 由本文件末尾的 `scoring_period_constant_matches_scale_key_authority` 现场核对权威。
fn tier_scoring_period_const_node(period_literal: &str, y: f64) -> WorkflowNode {
    WorkflowNode::DataTransformer(DataTransformerNode {
        base: WorkflowNodeBase {
            id: "const-scoring-period".into(),
            title: "本档评分尺度常量".into(),
            description: Some(
                "产出 scoring_period（本档尺度名），供 compute_scoring 的 period 参数取用".into(),
            ),
            position: Position { x: 900.0, y },
            retry: RetryConfig::default(),
            timeout: None,
            enabled: true,
            parent_id: None,
            compensation: None,
            continue_on_fail: false,
        },
        config: DataTransformerNodeConfig {
            input_var: String::new(),
            expression: format!("\"{period_literal}\""),
            output_var: "scoring_period".into(),
        },
    })
}

/// 本档评分工具节点（`compute_scoring`）。`period` 取**变量** `scoring_period`
/// —— 由同模板内的 [`tier_scoring_period_const_node`] 产出，不是字面量（见其文档）。
///
/// ⚠ v137 起本节点**只接阈值域三条**（布林标准差倍数 / 放量比 / 缩量比），
/// 窗口域五条（MACD 快慢信号、布林周期、量能回看）**一律不接**：档侧的「几根 bar」
/// 由该档的 `ScaleWindowPlan` 决定（#41 片 A），把面板的 5 个窗口值接到四档上
/// 等于把四档的指标窗口重新焊成同一份 —— 正是片 A 刚拆掉的那个缺陷。
/// 工具侧对这种接线**显式失败**（`mcp_tools::effective_indicator_config`），
/// 种子侧由 `scripts/check-indicator-config-scope.mjs` 的 P2 双向锁（接了要红、漏了也要红）。
fn tier_scoring_tool_node(id: &str, title: &str, x: f64, y: f64) -> WorkflowNode {
    let mut input_mapping = HashMap::new();
    input_mapping.insert("stock_code".to_string(), "stock_code".to_string());
    input_mapping.insert("period".to_string(), "scoring_period".to_string());
    // 阈值域三条**逐字写**（不循环生成、不从别处拼）：与 §九十一(2.5) 拒绝 `format!` 拼装同一条
    // 理由 —— 按字面量配对的那些 node 门（`audit-inject-coverage.mjs` / `check-tier-purity.mjs`）
    // 读的就是这里的字面量，拼装会让它们静默失明。
    input_mapping.insert("ind_boll_stddev".to_string(), "boll_stddev".to_string());
    input_mapping.insert("ind_volume_surge_ratio".to_string(), "volume_surge_ratio".to_string());
    input_mapping.insert("ind_volume_shrink_ratio".to_string(), "volume_shrink_ratio".to_string());
    WorkflowNode::Tool(ToolNode {
        base: WorkflowNodeBase {
            id: id.into(),
            title: title.into(),
            description: Some("获取数据: compute_scoring".into()),
            position: Position { x, y },
            retry: RetryConfig { enabled: true, max_retries: 2, ..Default::default() },
            // 继承 RunOptions.tool_timeout（来自 tool_timeout_secs 设置），与 `tool_node` 闭包一致。
            timeout: None,
            enabled: true,
            parent_id: None,
            compensation: None,
            continue_on_fail: false,
        },
        config: ToolNodeConfig {
            tool_name: "compute_scoring".into(),
            input_mapping,
            output_var: id.into(),
        },
    })
}

/// 本档**按档风险节点**（v140 从父图搬进来，补齐 §九十一(0) 的三节点形状）。
///
/// 输入面 = 7 条全局轴 + 2 条本档轴（`riskWindows.<camel档>`），与父图旧形态**逐字一致** ⇒
/// 数值零变化。`t-risk` 由父扇出以恒等键传进子快照（它不是子节点，所以子图里**不给它建边**：
/// 子图的根仍只有 `const-scoring-period`，风险节点无入边 = 与评分节点并行起跑，
/// 而它要的 `t-risk` 在扇出开工前就已到账 —— 父侧的 `t-risk → pm-h-<档>` 供给边保证这点）。
///
/// ⚠ 九个映射的**字面量在四个调用点各写一遍**（含七条全局轴），不在这个 helper 里用 `format!` 拼、
/// 也不从别处的常量合：拼起来就没有任何字面量可让文本门读
/// （`check-tier-purity.mjs` 的 R2 与 Rust 侧的 `tier_branch_reads_own_risk_cell` 都靠字面量
/// 判「读本档那一格」）—— 同 §九十一(2.5) 拒绝 `format!` 拼装评分脚本路径是同一条理由。
///
/// `continue_on_fail = false`（父图旧形态是 `true`）：**这是刻意的**，为了让失败面与搬动前逐位相同。
/// 搬动前父节点失败 ⇒ 它的产出不进父池 ⇒ 扇出的严格 `map_inputs` 取不到该键 ⇒ **整档子执行失败**；
/// 若在子图里留 `true`，风险节点失败就只剩「分支读不到 overall_risk」这种软降级 ⇒
/// 等于趁搬图偷偷把「该档失败」放宽成「该档少一腿」。要改这个语义，得单独裁定并点名消费者。
fn tier_risk_node(
    id: &str,
    title: &str,
    description: &str,
    args: [(&str, &str); 9],
    x: f64,
    y: f64,
) -> WorkflowNode {
    WorkflowNode::Code(CodeNode {
        base: WorkflowNodeBase {
            id: id.into(),
            title: title.into(),
            description: Some(description.into()),
            position: Position { x, y },
            retry: RetryConfig::default(),
            timeout: Some(10),
            enabled: true,
            parent_id: None,
            compensation: None,
            continue_on_fail: false,
        },
        config: CodeNodeConfig {
            language: "rhai".into(),
            code: include_str!("../risk-level.rhai").to_string(),
            // output_var 与节点 id 不同名（父图旧形态就是这样；下游读的是 `{id}.result.category`）
            output_var: format!("risk-level-{}", id.replace("cls-risk-level-", "")),
            tool_name: None,
            execute_directly: true,
            input_mapping: args.into_iter().map(|(k, v)| (k.to_string(), v.to_string())).collect(),
        },
    })
}

// ══════════════════════════════════════════════════════════════════════════
// 超短档
// ══════════════════════════════════════════════════════════════════════════

fn ultra_short_tier_template() -> (Vec<WorkflowNode>, Vec<WorkflowEdge>) {
    let nodes = vec![
        tier_scoring_period_const_node("hourly", 2700.0),
        tier_scoring_tool_node("t-scoring-hour", "技术评分（60 分钟）", 1020.0, 2700.0),
        // v140：本档按档风险节点（父图搬入，见 `tier_risk_node` 的文档）。
        tier_risk_node(
            "cls-risk-level-ultra-short",
            "超短线风险等级分类",
            "按档风险分类（v128 B1）：阈值与全局节点一致，另加本档（2 日）回撤深度判据",
            [
                (
                    "risk_volatility",
                    "t-risk.result.content.stockRiskProfile.annualizedVolatilityPct",
                ),
                ("risk_drawdown", "t-risk.result.content.stockRiskProfile.maxDrawdownPct"),
                ("risk_sharpe", "t-risk.result.content.stockRiskProfile.sharpeRatio"),
                ("risk_roe", "t-risk.result.content.stockRiskProfile.roeTTMPct"),
                ("risk_gross_margin", "t-risk.result.content.stockRiskProfile.grossMarginPct"),
                ("risk_debt_ratio", "t-risk.result.content.stockRiskProfile.debtRatioPct"),
                (
                    "risk_revenue_growth",
                    "t-risk.result.content.stockRiskProfile.revenueGrowthYoYPct",
                ),
                (
                    "risk_drawdown_depth",
                    "t-risk.result.content.stockRiskProfile.riskWindows.ultraShort.drawdownDepth",
                ),
                (
                    "risk_window_days",
                    "t-risk.result.content.stockRiskProfile.riskWindows.ultraShort.windowDays",
                ),
            ],
            1020.0,
            2950.0,
        ),
        WorkflowNode::Code(CodeNode {
            base: WorkflowNodeBase {
                id: "pm-h-ultra-short".into(),
                title: "超短分支决策".into(),
                description: Some(
                    "只吃本档证据的决策分支（R-11）；输出供后续 arbiter 读，不改主链结论".into(),
                ),
                position: Position { x: 1300.0, y: 4200.0 },
                retry: RetryConfig::default(),
                timeout: Some(10),
                enabled: true,
                parent_id: None,
                compensation: None,
                continue_on_fail: true,
            },
            config: CodeNodeConfig {
                language: "rhai".into(),
                code: include_str!("../portfolio-mgr-h-ultra-short.rhai").to_string(),
                output_var: "h_ultra_short".into(),
                tool_name: None,
                execute_directly: true,
                input_mapping: [
                    ("branch_json", "horizon_branch_json.ultra_short"),
                    ("horizon_prior_json", "horizon_prior_json"),
                    ("tier_score", "t-scoring-hour.result.content.totalScore"),
                    ("macd_dif", "t-scoring-hour.result.content.indicators.macdDif"),
                    ("macd_dea", "t-scoring-hour.result.content.indicators.macdDea"),
                    ("rsi_value", "t-scoring-hour.result.content.indicators.scaleMomentum.value"),
                    // v138（裁定 3「让用户看出各档实际几根」，PLAN §一○六）：本档**实际用的指标窗口**
                    // + 所在尺度一起带进决策行 ⇒ 界面能把「这一档算得粗」与「这一档观点不同」分开读。
                    // 读的是**本档自己的**评分节点（子图内产出）⇒ 父扇出面一条都不用加。
                    // `windows` 是节点回显而非「想要的配置」（产端 = `indicators::IndicatorWindows`）。
                    ("scoring_windows", "t-scoring-hour.result.content.indicators.windows"),
                    ("scoring_scale", "t-scoring-hour.result.content.period"),
                    // v139（呈现层补齐，PLAN §一○八）：两带与动量的**数值**也上屏，
                    // 不只窗口根数 —— 读者要能看出「这一档的两带差是多少」。
                    ("scale_trend", "t-scoring-hour.result.content.indicators.scaleTrend"),
                    ("scale_momentum", "t-scoring-hour.result.content.indicators.scaleMomentum"),
                    // v128（B1）：本档风险档 ← **本档**的 cls-risk-level-ultra-short 节点
                    ("overall_risk", "cls-risk-level-ultra-short.result.category"),
                    // `kline_bars` 取**日线** `t-scoring` 而非本档尺度节点：本档要的量是 `σ_daily`
                    // （`pm_vol_move_pct` 的口径就是日收益标准差 × √持有天数），日线才是它的正解。
                    ("kline_bars", "t-scoring.result.content.kline_json"),
                    // 面板乘数**必须恒等映射**：脚本里 `else { 1.2 }` 的默认值若顶替了面板值，
                    // 面板调到 1.5 时本分支仍按 1.2 算 —— 那是拿默认值冒充面板值，不是缺席。
                    ("stop_vol_mult", "stop_vol_mult"),
                    // 涨停池广度（本档 entryGate 的必要条件 + breadthState 腿的原料）；
                    // `breadth` 为 None 时导航失败 ⇒ 注入补 unit ⇒ 脚本走「缺席」分支并留痕。
                    ("seal_rate", "t-limitup-pool.result.content.breadth.sealRate"),
                    ("pool_break_count", "t-limitup-pool.result.content.breadth.breakCount"),
                ]
                .into_iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
            },
        }),
        tier_end_node("h_ultra_short", 4200.0),
    ];
    let edges = vec![
        // 子图内的顺序边：常量 → 本档评分 → 本档分支决策 → 终值。
        // 常量节点是子图唯一的根（它不读任何变量），评分节点必须有它的入边，
        // 否则 DAG 上评分会在常量之前起跑 —— 「有边才等」，无入边的非根节点立刻跑。
        direct_edge(
            "e-const-scoring-period-t-scoring-hour",
            "const-scoring-period",
            "t-scoring-hour",
        ),
        direct_edge("e-t-scoring-hour-pm-h-ultra-short", "t-scoring-hour", "pm-h-ultra-short"),
        // v140：风险节点无入边（它只读扇出传进来的 `t-risk`）⇒ 与评分节点并行起跑；
        // 分支必须等它 ⇒ 这条边是「本档风险档在分支之前算出来」的唯一时序保证。
        direct_edge(
            "e-cls-risk-level-ultra-short-pm-h-ultra-short",
            "cls-risk-level-ultra-short",
            "pm-h-ultra-short",
        ),
        direct_edge("e-pm-h-ultra-short-end", "pm-h-ultra-short", "end"),
    ];
    (nodes, edges)
}

// ══════════════════════════════════════════════════════════════════════════
// 短线档
// ══════════════════════════════════════════════════════════════════════════

fn short_tier_template() -> (Vec<WorkflowNode>, Vec<WorkflowEdge>) {
    let nodes = vec![
        tier_scoring_period_const_node("weekly", 2700.0),
        tier_scoring_tool_node("t-scoring-week", "技术评分（周线）", 1140.0, 2700.0),
        // v140：本档按档风险节点（父图搬入，见 `tier_risk_node` 的文档）。
        tier_risk_node(
            "cls-risk-level-short",
            "短线风险等级分类",
            "按档风险分类（v128 B1）：阈值与全局节点一致，另加本档（5 日）回撤深度判据",
            [
                (
                    "risk_volatility",
                    "t-risk.result.content.stockRiskProfile.annualizedVolatilityPct",
                ),
                ("risk_drawdown", "t-risk.result.content.stockRiskProfile.maxDrawdownPct"),
                ("risk_sharpe", "t-risk.result.content.stockRiskProfile.sharpeRatio"),
                ("risk_roe", "t-risk.result.content.stockRiskProfile.roeTTMPct"),
                ("risk_gross_margin", "t-risk.result.content.stockRiskProfile.grossMarginPct"),
                ("risk_debt_ratio", "t-risk.result.content.stockRiskProfile.debtRatioPct"),
                (
                    "risk_revenue_growth",
                    "t-risk.result.content.stockRiskProfile.revenueGrowthYoYPct",
                ),
                (
                    "risk_drawdown_depth",
                    "t-risk.result.content.stockRiskProfile.riskWindows.short.drawdownDepth",
                ),
                (
                    "risk_window_days",
                    "t-risk.result.content.stockRiskProfile.riskWindows.short.windowDays",
                ),
            ],
            1140.0,
            2950.0,
        ),
        WorkflowNode::Code(CodeNode {
            base: WorkflowNodeBase {
                id: "pm-h-short".into(),
                title: "短线分支决策".into(),
                description: Some(
                    "只吃本档证据的决策分支（R-11）；输出供 pm-arbiter 读，不改主链结论".into(),
                ),
                position: Position { x: 1300.0, y: 4320.0 },
                retry: RetryConfig::default(),
                timeout: Some(10),
                enabled: true,
                parent_id: None,
                compensation: None,
                continue_on_fail: true,
            },
            config: CodeNodeConfig {
                language: "rhai".into(),
                code: include_str!("../portfolio-mgr-h-short.rhai").to_string(),
                output_var: "h_short".into(),
                tool_name: None,
                execute_directly: true,
                input_mapping: [
                    ("branch_json", "horizon_branch_json.short"),
                    ("horizon_prior_json", "horizon_prior_json"),
                    // 本档尺度节点 = 周线（阶段 2 拍板：短=周线 / 中=月线 / 长=季线）
                    ("tier_score", "t-scoring-week.result.content.totalScore"),
                    ("macd_dif", "t-scoring-week.result.content.indicators.macdDif"),
                    ("macd_dea", "t-scoring-week.result.content.indicators.macdDea"),
                    ("rsi_value", "t-scoring-week.result.content.indicators.scaleMomentum.value"),
                    // v138 裁定 3：本档窗口回显 + 尺度（详注见超短模板那一处）
                    ("scoring_windows", "t-scoring-week.result.content.indicators.windows"),
                    ("scoring_scale", "t-scoring-week.result.content.period"),
                    // v139 呈现层补齐（详注见超短模板那一处）
                    ("scale_trend", "t-scoring-week.result.content.indicators.scaleTrend"),
                    ("scale_momentum", "t-scoring-week.result.content.indicators.scaleMomentum"),
                    // #23（v132）：本档解禁供给占比 —— 取数层按**权威交易日窗**归约后的**小数**占比
                    // （`Σ解禁市值 ÷ 流通市值`，锚点 = as-of 截止日）。
                    (
                        "lockup_float_ratio",
                        "t-lockup-data.result.content.supply_shock.windows.short.ratio",
                    ),
                    // 缺席分两种，文案必须不同：`unavailableReason` 在 ⇒ 取数层拿不到分母；
                    // 两者都不在 ⇒ 该节点整段没跑成。
                    (
                        "lockup_supply_reason",
                        "t-lockup-data.result.content.supply_shock.unavailableReason",
                    ),
                    ("seal_rate", "t-limitup-pool.result.content.breadth.sealRate"),
                    // v133（B2-2）：verdict 路径按**本档实例**生成（`a-hot-money--short`）。
                    ("flow_persistence", "a-hot-money--short.content.verdict.flowPersistence"),
                    // v128（B1）：本档风险档 ← **本档**的 cls-risk-level-short 节点
                    ("overall_risk", "cls-risk-level-short.result.category"),
                    ("kline_bars", "t-scoring.result.content.kline_json"),
                    ("stop_vol_mult", "stop_vol_mult"),
                    ("take_profit_vol_mult", "take_profit_vol_mult"),
                ]
                .into_iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
            },
        }),
        tier_end_node("h_short", 4320.0),
    ];
    let edges = vec![
        direct_edge(
            "e-const-scoring-period-t-scoring-week",
            "const-scoring-period",
            "t-scoring-week",
        ),
        direct_edge("e-t-scoring-week-pm-h-short", "t-scoring-week", "pm-h-short"),
        // v140：风险节点 → 本档分支（同超短那一处注释）
        direct_edge("e-cls-risk-level-short-pm-h-short", "cls-risk-level-short", "pm-h-short"),
        direct_edge("e-pm-h-short-end", "pm-h-short", "end"),
    ];
    (nodes, edges)
}

// ══════════════════════════════════════════════════════════════════════════
// 中线档
// ══════════════════════════════════════════════════════════════════════════

fn mid_tier_template() -> (Vec<WorkflowNode>, Vec<WorkflowEdge>) {
    let nodes = vec![
        tier_scoring_period_const_node("monthly", 2700.0),
        tier_scoring_tool_node("t-scoring-month", "技术评分（月线）", 1260.0, 2700.0),
        // v140：本档按档风险节点（父图搬入，见 `tier_risk_node` 的文档）。
        tier_risk_node(
            "cls-risk-level-mid",
            "中线风险等级分类",
            "按档风险分类（v128 B1）：阈值与全局节点一致，另加本档（28 日）回撤深度判据",
            [
                (
                    "risk_volatility",
                    "t-risk.result.content.stockRiskProfile.annualizedVolatilityPct",
                ),
                ("risk_drawdown", "t-risk.result.content.stockRiskProfile.maxDrawdownPct"),
                ("risk_sharpe", "t-risk.result.content.stockRiskProfile.sharpeRatio"),
                ("risk_roe", "t-risk.result.content.stockRiskProfile.roeTTMPct"),
                ("risk_gross_margin", "t-risk.result.content.stockRiskProfile.grossMarginPct"),
                ("risk_debt_ratio", "t-risk.result.content.stockRiskProfile.debtRatioPct"),
                (
                    "risk_revenue_growth",
                    "t-risk.result.content.stockRiskProfile.revenueGrowthYoYPct",
                ),
                (
                    "risk_drawdown_depth",
                    "t-risk.result.content.stockRiskProfile.riskWindows.mid.drawdownDepth",
                ),
                (
                    "risk_window_days",
                    "t-risk.result.content.stockRiskProfile.riskWindows.mid.windowDays",
                ),
            ],
            1260.0,
            2950.0,
        ),
        WorkflowNode::Code(CodeNode {
            base: WorkflowNodeBase {
                id: "pm-h-mid".into(),
                title: "中线分支决策".into(),
                description: Some(
                    "只吃本档证据的决策分支（R-11）；输出供 pm-arbiter 读，不改主链结论".into(),
                ),
                position: Position { x: 1300.0, y: 4440.0 },
                retry: RetryConfig::default(),
                timeout: Some(10),
                enabled: true,
                parent_id: None,
                compensation: None,
                continue_on_fail: true,
            },
            config: CodeNodeConfig {
                language: "rhai".into(),
                code: include_str!("../portfolio-mgr-h-mid.rhai").to_string(),
                output_var: "h_mid".into(),
                tool_name: None,
                execute_directly: true,
                input_mapping: [
                    ("branch_json", "horizon_branch_json.mid"),
                    ("horizon_prior_json", "horizon_prior_json"),
                    ("tier_score", "t-scoring-month.result.content.totalScore"),
                    ("macd_dif", "t-scoring-month.result.content.indicators.macdDif"),
                    ("macd_dea", "t-scoring-month.result.content.indicators.macdDea"),
                    ("rsi_value", "t-scoring-month.result.content.indicators.scaleMomentum.value"),
                    // v138 裁定 3：本档窗口回显（详注见超短模板那一处）
                    ("scoring_windows", "t-scoring-month.result.content.indicators.windows"),
                    ("scoring_scale", "t-scoring-month.result.content.period"),
                    // v139 呈现层补齐（详注见超短模板那一处）
                    ("scale_trend", "t-scoring-month.result.content.indicators.scaleTrend"),
                    ("scale_momentum", "t-scoring-month.result.content.indicators.scaleMomentum"),
                    // #23（v132）：mid 的 `supplyShock` 是 riskNote（权重恒 0、不进方向），
                    // 但「有数可报」与「无数可报」是两件事 —— 接通后 riskNotes 才真能给出该档窗口内的解禁占比。
                    (
                        "lockup_float_ratio",
                        "t-lockup-data.result.content.supply_shock.windows.mid.ratio",
                    ),
                    // 缺席原因（与 #24 的 `basis` 同一条理由）：「该窗没有解禁」与「分母取不到」
                    // 是两种缺席，合成一句就分不出标的属性与接线状态。
                    (
                        "lockup_supply_reason",
                        "t-lockup-data.result.content.supply_shock.unavailableReason",
                    ),
                    // PE 历史分位（valuationBand 腿）：样本不足时该字段是 null ⇒ 导航失败
                    // ⇒ unit ⇒ 诚实缺席（不拿现价 PE 近似）。
                    ("pe_percentile", "t-valuation-band.result.content.metricPe.currentPercentile"),
                    ("f_score", "t-valuation.result.content.fScore.score"),
                    ("consensus_eps", "t-consensus-data.result.content.consensusEps"),
                    ("consensus_estimated", "t-consensus-data.result.content.isEstimated"),
                    // #24（v131）：`expectationRevision` 的**分母** —— 最近一个已披露年报的 EPS
                    // （年度口径，不做年化外推）。`basis` 与 `latest_eps` 分开是因为缺席有两种：
                    // 前者是标的属性，后者是接线状态，文案必须不同。
                    ("latest_eps", "t-valuation.result.content.latestEps.value"),
                    ("latest_eps_basis", "t-valuation.result.content.latestEps.basis"),
                    // v133（B2-2）：verdict 路径按**本档实例**生成（`a-hot-money--mid`）。
                    ("flow_persistence", "a-hot-money--mid.content.verdict.flowPersistence"),
                    // v128（B1）：本档风险档 ← **本档**的 cls-risk-level-mid 节点
                    ("overall_risk", "cls-risk-level-mid.result.category"),
                    ("kline_bars", "t-scoring.result.content.kline_json"),
                    ("stop_vol_mult", "stop_vol_mult"),
                    ("take_profit_vol_mult", "take_profit_vol_mult"),
                ]
                .into_iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
            },
        }),
        tier_end_node("h_mid", 4440.0),
    ];
    let edges = vec![
        direct_edge(
            "e-const-scoring-period-t-scoring-month",
            "const-scoring-period",
            "t-scoring-month",
        ),
        direct_edge("e-t-scoring-month-pm-h-mid", "t-scoring-month", "pm-h-mid"),
        // v140：风险节点 → 本档分支（同超短那一处注释）
        direct_edge("e-cls-risk-level-mid-pm-h-mid", "cls-risk-level-mid", "pm-h-mid"),
        direct_edge("e-pm-h-mid-end", "pm-h-mid", "end"),
    ];
    (nodes, edges)
}

// ══════════════════════════════════════════════════════════════════════════
// 长线档
// ══════════════════════════════════════════════════════════════════════════

fn long_tier_template() -> (Vec<WorkflowNode>, Vec<WorkflowEdge>) {
    let nodes = vec![
        tier_scoring_period_const_node("quarterly", 2700.0),
        tier_scoring_tool_node("t-scoring-quarter", "技术评分（季度）", 1380.0, 2700.0),
        // v140：本档按档风险节点（父图搬入，见 `tier_risk_node` 的文档）。
        // 季线的 `riskWindows.long` 由 `windows_for_horizon` 按 3 根季线 = 60 交易日口径给出。
        tier_risk_node(
            "cls-risk-level-long",
            "长线风险等级分类",
            "按档风险分类（v128 B1）：阈值与全局节点一致，另加本档（90 日）回撤深度判据",
            [
                (
                    "risk_volatility",
                    "t-risk.result.content.stockRiskProfile.annualizedVolatilityPct",
                ),
                ("risk_drawdown", "t-risk.result.content.stockRiskProfile.maxDrawdownPct"),
                ("risk_sharpe", "t-risk.result.content.stockRiskProfile.sharpeRatio"),
                ("risk_roe", "t-risk.result.content.stockRiskProfile.roeTTMPct"),
                ("risk_gross_margin", "t-risk.result.content.stockRiskProfile.grossMarginPct"),
                ("risk_debt_ratio", "t-risk.result.content.stockRiskProfile.debtRatioPct"),
                (
                    "risk_revenue_growth",
                    "t-risk.result.content.stockRiskProfile.revenueGrowthYoYPct",
                ),
                (
                    "risk_drawdown_depth",
                    "t-risk.result.content.stockRiskProfile.riskWindows.long.drawdownDepth",
                ),
                (
                    "risk_window_days",
                    "t-risk.result.content.stockRiskProfile.riskWindows.long.windowDays",
                ),
            ],
            1380.0,
            2950.0,
        ),
        WorkflowNode::Code(CodeNode {
            base: WorkflowNodeBase {
                id: "pm-h-long".into(),
                title: "长线分支决策".into(),
                description: Some(
                    "只吃本档证据的决策分支（R-11）；输出供 pm-arbiter 读，不改主链结论".into(),
                ),
                position: Position { x: 1300.0, y: 4560.0 },
                retry: RetryConfig::default(),
                timeout: Some(10),
                enabled: true,
                parent_id: None,
                compensation: None,
                continue_on_fail: true,
            },
            config: CodeNodeConfig {
                language: "rhai".into(),
                code: include_str!("../portfolio-mgr-h-long.rhai").to_string(),
                output_var: "h_long".into(),
                tool_name: None,
                execute_directly: true,
                input_mapping: [
                    ("branch_json", "horizon_branch_json.long"),
                    ("horizon_prior_json", "horizon_prior_json"),
                    ("tier_score", "t-scoring-quarter.result.content.totalScore"),
                    ("macd_dif", "t-scoring-quarter.result.content.indicators.macdDif"),
                    ("macd_dea", "t-scoring-quarter.result.content.indicators.macdDea"),
                    (
                        "rsi_value",
                        "t-scoring-quarter.result.content.indicators.scaleMomentum.value",
                    ),
                    // v138 裁定 3：本档窗口回显 + 尺度（详注见超短模板那一处）
                    ("scoring_windows", "t-scoring-quarter.result.content.indicators.windows"),
                    ("scoring_scale", "t-scoring-quarter.result.content.period"),
                    // v139 呈现层补齐（详注见超短模板那一处）
                    ("scale_trend", "t-scoring-quarter.result.content.indicators.scaleTrend"),
                    ("scale_momentum", "t-scoring-quarter.result.content.indicators.scaleMomentum"),
                    ("pe_percentile", "t-valuation-band.result.content.metricPe.currentPercentile"),
                    ("f_score", "t-valuation.result.content.fScore.score"),
                    ("consensus_eps", "t-consensus-data.result.content.consensusEps"),
                    ("consensus_estimated", "t-consensus-data.result.content.isEstimated"),
                    // #24（v131）：`expectationRevision` 的**分母** —— 最近一个已披露年报的 EPS
                    // （年度口径，不做年化外推；季报是年内累计值，不可当年度值用）。
                    ("latest_eps", "t-valuation.result.content.latestEps.value"),
                    ("latest_eps_basis", "t-valuation.result.content.latestEps.basis"),
                    // 目标价**只**来自估值结论；`applicable=false` ⇒ 按裁定③不做 PE 分位代理，
                    // 赔率无定义 ⇒ 本档不出仓位（脚本里点名 `takeProfitSource="no_target"`）。
                    ("valuation_dcf_upside", "t-valuation.result.content.dcf.upsidePct"),
                    (
                        "valuation_dcf_applicable",
                        "t-valuation.result.content.dcf.assumptions.applicable",
                    ),
                    // 宏观只取 PMI（本仓五条真序列里唯一自带荣枯线的量）
                    ("pmi", "t-macro-data.result.content.pmiManufacturing.value"),
                    // v128（B1）：本档风险档 ← **本档**的 cls-risk-level-long 节点
                    ("overall_risk", "cls-risk-level-long.result.category"),
                    ("kline_bars", "t-scoring.result.content.kline_json"),
                    // 本档出场口径 = 目标价止盈 + 论点证伪止损，没有「止盈倍数」这一说 ⇒ 不接
                    ("stop_vol_mult", "stop_vol_mult"),
                ]
                .into_iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
            },
        }),
        tier_end_node("h_long", 4560.0),
    ];
    let edges = vec![
        direct_edge(
            "e-const-scoring-period-t-scoring-quarter",
            "const-scoring-period",
            "t-scoring-quarter",
        ),
        direct_edge("e-t-scoring-quarter-pm-h-long", "t-scoring-quarter", "pm-h-long"),
        // v140：风险节点 → 本档分支（同超短那一处注释）
        direct_edge("e-cls-risk-level-long-pm-h-long", "cls-risk-level-long", "pm-h-long"),
        direct_edge("e-pm-h-long-end", "pm-h-long", "end"),
    ];
    (nodes, edges)
}

/// 档子模板的版本门常量。
///
/// **为什么不另取一个数字**：本批的扇出映射（父）与档模板内容（子）是**一对契约** ——
/// 少传一个键就是子执行运行期硬错（`subworkflow_executor.rs:97-105`），两者不同代必然错配。
/// 直接绑到主图常量上 ⇒ 「档模板版本 = 主图版本」由编译器保证，抄第二份数字迟早漂移
/// （本仓「清单由单一权威渲染」的纪律；版本门本身仍是各模板行各自的 `>=` 判定，
/// 先例 = `REFLECTION_TEMPLATE_VERSION` 与认知编排器的 L1/L2/L3）。
pub(crate) const HORIZON_TIER_TEMPLATE_VERSION: i32 = super::seed_stock_analysis::TEMPLATE_VERSION;

/// 本档模板的中文显示名（模板列表里要分得清是哪一档，四行同名会让「按档」在界面上退化）。
fn tier_template_name(period: Period) -> &'static str {
    match period {
        Period::UltraShort => "A股超短档分支",
        Period::Short => "A股短线档分支",
        Period::Mid => "A股中线档分支",
        Period::Long => "A股长线档分支",
    }
}

/// 播种四张档子模板（`stock-horizon-{ultra_short,short,mid,long}` 的 kebab 形态）。
///
/// 形状照认知编排器（`init::cognitive_router_init::ensure_template`）：
/// 版本门 `existing.version >= 本常量 ⇒ 跳过`，否则 `upsert_workflow_template` 整行覆盖。
/// - 四行都 `is_preset=true`、`is_public=false`、`visibility=SystemOnly` —— 它们只由父图
///   那个 `pm-h-<档>` 扇出启动，不该出现在模板列表里让用户单独跑（跑也跑不起来：入参全靠扇出）；
/// - **无 trigger**：档子模板与 L1/L2/L3 同形，首节点是 `const-scoring-period`；
/// - **不写快照历史**：`upsert_workflow_template` 本身不写 `workflow_template_versions`
///   （写快照的是 `update_workflow_template`，那是「用户保存」路径），而代际回查只服务主链；
/// - `hooks_config=None`：子执行**继承**父注入的变量（`horizon_branch_json` 等已由父侧
///   hooks 注入并经扇出传入）；在子模板上再声明一套钩子会造成两套真相。
pub(crate) async fn seed_horizon_tier_templates(
    db: &axagent_dao::db::DatabaseConnection,
) -> Result<(), String> {
    use crate::commands::error::ErrorResponse;
    use crate::commands::error_code::stock_setup;
    for period in Period::ALL {
        let id = horizon_tier_template_id(period);
        let existing = axagent_dao::repo::workflow_template::get_workflow_template(db, &id)
            .await
            .map_err(|e| {
            ErrorResponse::new(stock_setup::INTERNAL)
                .with_detail(format!("查询档模板 {id} 失败: {e}"))
        })?;
        if let Some(t) = &existing {
            if t.version >= HORIZON_TIER_TEMPLATE_VERSION {
                tracing::debug!("[stock_analysis_setup] 档模板 {id} 已是最新 v{}，跳过", t.version);
                continue;
            }
            tracing::info!(
                "[stock_analysis_setup] 档模板 {id} 结构变更，版本 v{} → v{HORIZON_TIER_TEMPLATE_VERSION}，重新灌入",
                t.version
            );
        } else {
            tracing::info!("[stock_analysis_setup] 创建档模板: {id}");
        }
        let (nodes, edges) = horizon_tier_template_nodes(period);
        let now = chrono::Utc::now().timestamp_millis();
        let template = axagent_harness::workflow_types::WorkflowTemplateData {
            id: id.clone(),
            name: tier_template_name(period).to_string(),
            description: Some(
                "四周期分支的档子模板：本档尺度评分 + 本档分支决策（B-2b，主图 pm-h-<档> 扇出调用）"
                    .to_string(),
            ),
            icon: "chart-bar".to_string(),
            tags: vec!["stock".to_string(), "horizon-tier".to_string(), "A股".to_string()],
            version: HORIZON_TIER_TEMPLATE_VERSION,
            is_preset: true,
            is_editable: true,
            is_public: false,
            visibility: axagent_harness::capability::Visibility::SystemOnly,
            trigger_config: None,
            nodes,
            edges,
            input_schema: None,
            output_schema: None,
            variables: Vec::new(),
            error_config: None,
            error_workflow_id: None,
            tool_defs: Vec::new(),
            mission_hash: None,
            cluster_id: Some("equity".to_string()),
            hooks_config: None,
            route_path: Some(format!("/finance/equity/horizon-tier/{id}")),
            created_at: now,
            updated_at: now,
        };
        let active = axagent_dao::repo::workflow_template::build_active_model_from_data(&template);
        axagent_dao::repo::workflow_template::upsert_workflow_template(db, active).await.map_err(
            |e| {
                ErrorResponse::new(stock_setup::INTERNAL)
                    .with_detail(format!("写入档模板 {id} 失败: {e}"))
            },
        )?;
    }
    tracing::info!(
        "[stock_analysis_setup] 四张档子模板已种子化 (v{HORIZON_TIER_TEMPLATE_VERSION})"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workflow_fanout_audit::external_reads;
    use std::collections::BTreeSet;

    /// 判据本体：本档模板的 needs 里出现了**兄弟模板的节点 id** ⇒ 返回那些 id。
    ///
    /// §九十一(5) 决策点 3 要的计数断言（R2 的跨模板版）：错峰链在改图后物理断开，
    /// 若有人为了「复用月线」把兄弟档的评分节点接进季线子模板，四档就重新变成同一份输入。
    ///
    /// 兄弟档的节点 id 不是人抄的清单 —— 由 `horizon_tier_template_nodes(其他档)` **现算**，
    /// 所以改任何一档的节点命名都不会让本判据静默失效。四张模板**共用**的 id
    /// （`end` 与 `const-scoring-period`）先做差集剔除，否则会把「同名的节点」误报成跨档引用。
    ///
    /// ⚠ 口径边界要说清：`external_reads` 给的是**根名**，所以这条查「有没有引用兄弟档的节点」，
    /// 查不出「同一节点里读了他档那一格路径」（如 `riskWindows.mid` 出现在超短模板）——
    /// 后者由 `check-tier-purity.mjs` 的 R2 按字面量路径守（§九十一(3) 步骤 3 扩面）。
    fn sibling_id_hits(period: Period, nodes: &[WorkflowNode]) -> Vec<String> {
        let mine: BTreeSet<String> = nodes.iter().map(|n| n.base_id().to_string()).collect();
        let mut blind = BTreeSet::new();
        let needs = external_reads(nodes, &mut blind);
        let mut hits = Vec::new();
        for other in Period::ALL.iter().filter(|p| **p != period) {
            for n in horizon_tier_template_nodes(*other).0 {
                let id = n.base_id().to_string();
                if mine.contains(&id) {
                    continue;
                }
                if needs.contains(&id) {
                    hits.push(id);
                }
            }
        }
        hits
    }

    /// 诊断 + 两条真判据。
    ///
    /// 打印的是 §九十一(3) 步骤 1 要的机器答案：**父侧每档必须显式传哪些键**。
    /// 改图时这张清单不许由人抄（那正是 §九十 那道门建在改图之前的用途）。
    #[test]
    fn tier_template_external_reads_are_printed_and_tier_pure() {
        for period in Period::ALL {
            let (nodes, edges) = horizon_tier_template_nodes(period);
            let mut blind: BTreeSet<&'static str> = BTreeSet::new();
            let needs = external_reads(&nodes, &mut blind);
            println!(
                "[B-2b needs] {} ⇒ {} 个外部键：{}",
                period.as_str(),
                needs.len(),
                needs.iter().cloned().collect::<Vec<_>>().join("、")
            );
            println!(
                "[B-2b shape] {} ⇒ 节点 {} 个（{}），边 {} 条",
                period.as_str(),
                nodes.len(),
                nodes.iter().map(|n| n.base_id().to_string()).collect::<Vec<_>>().join("、"),
                edges.len()
            );

            // 判据一：`node_var_io` 必须认得本模板用到的每一种节点。盲区会让 needs **少报**，
            // 于是扇出门在改图后照样绿，而子执行运行期报 `Variable not found`。
            assert!(
                blind.is_empty(),
                "档模板 `{}` 里有 node_var_io 未覆盖的节点类型 {blind:?} ⇒ needs 会漏键",
                period.as_str()
            );
            // 判据二：不引用兄弟档的节点。
            assert!(
                sibling_id_hits(period, &nodes).is_empty(),
                "档模板 `{}` 引用了兄弟档的节点 {:?}",
                period.as_str(),
                sibling_id_hits(period, &nodes)
            );
        }
    }

    /// 负控：把超短模板里 `tier_score` 的源改成**周线**评分节点 ⇒ `sibling_id_hits` 必须报出它。
    ///
    /// 没有这条，判据二可能只是在「needs 根本不含任何节点 id」的假象上绿（本轮实测就踩过一次：
    /// `root_of` 丢段时 needs 里只剩 `result`/`content`，跨档断言恒真）。
    #[test]
    fn cross_tier_scoring_reference_is_caught() {
        let (mut nodes, _) = horizon_tier_template_nodes(Period::UltraShort);
        let before = sibling_id_hits(Period::UltraShort, &nodes);
        assert!(before.is_empty(), "前提被破坏：未改动前就已有跨档引用 {before:?}");
        let mut patched = false;
        for node in &mut nodes {
            if let WorkflowNode::Code(c) = node
                && c.base.id == "pm-h-ultra-short"
            {
                c.config
                    .input_mapping
                    .insert("tier_score".into(), "t-scoring-week.result.content.totalScore".into());
                patched = true;
            }
        }
        assert!(patched, "模板里没有 pm-h-ultra-short ⇒ 本负控失去前提");
        let hits = sibling_id_hits(Period::UltraShort, &nodes);
        assert!(
            hits.contains(&"t-scoring-week".to_string()),
            "把 tier_score 改成周线评分节点后仍不报 ⇒ 跨档判据没电（抽取面或比较面失效）"
        );
    }

    /// 判据本体：本档常量节点产出的 `scoring_period` 必须**逐字**等于本档权威尺度名。
    ///
    /// 返回问题清单（空 = 该档合格）。抽成函数而不是内联断言，是为了让下面的负控走
    /// **同一个**谓词 —— 另写一份比较公式的那种负控证不到真判据（判据层 4z 同族教训）。
    fn scoring_period_const_problems(period: Period, nodes: &[WorkflowNode]) -> Vec<String> {
        // 权威 = `Period::scale_key()`（harness 里档↔尺度那张表），四档各取自己那一臂。
        let want_expr = format!("\"{}\"", period.scale_key());
        let consts: Vec<&DataTransformerNode> = nodes
            .iter()
            .filter_map(|n| match n {
                WorkflowNode::DataTransformer(d) if d.base.id == "const-scoring-period" => Some(d),
                _ => None,
            })
            .collect();
        if consts.len() != 1 {
            return vec![format!(
                "{} 档模板里 `const-scoring-period` 节点有 {} 个（应为 1 个）⇒ 尺度常量面不在",
                period.as_str(),
                consts.len()
            )];
        }
        let c = &consts[0];
        let mut problems = Vec::new();
        if c.config.output_var != "scoring_period" {
            problems.push(format!(
                "{} 档常量节点的 output_var 是 {:?}，而评分节点的 period 取的是 `scoring_period`",
                period.as_str(),
                c.config.output_var
            ));
        }
        if c.config.expression != want_expr {
            problems.push(format!(
                "{} 档常量节点的 expression 是 {:?}，权威 `Period::scale_key()` 要的是 {want_expr:?} \
                 ⇒ 串档（四档取同一尺度）或拼错（compute_scoring 对未知尺度值是显式失败）",
                period.as_str(),
                c.config.expression
            ));
        }
        problems
    }

    /// 尺度常量与权威一致（正断言）。
    ///
    /// 为什么要钉这一条：四份常量是**逐字写**的（`check-tier-purity` 与本文件的 R2 要有
    /// 字面量读取面，所以不 `format!` 生成），写错一档不会有任何运行期报错 ——
    /// 串档时四档评分恒等，拼错时 `ScaleProfile::resolve` 显式失败（`mcp_tools.rs:1281`）。
    #[test]
    fn scoring_period_constant_matches_scale_key_authority() {
        for period in Period::ALL {
            let (nodes, _) = horizon_tier_template_nodes(period);
            let problems = scoring_period_const_problems(period, &nodes);
            assert!(problems.is_empty(), "{problems:?}");
        }
    }

    /// 负控：把超短档常量的字面量改成**周线**尺度 ⇒ 同一个谓词必须报出它。
    ///
    /// 没有这条，上面的正断言可能只是在「取不到常量节点却当没这回事」的假象上绿。
    #[test]
    fn wrong_scale_key_constant_is_caught() {
        let (mut nodes, _) = horizon_tier_template_nodes(Period::UltraShort);
        assert!(
            scoring_period_const_problems(Period::UltraShort, &nodes).is_empty(),
            "前提被破坏：未改动前常量就不一致"
        );
        let mut patched = false;
        for node in &mut nodes {
            if let WorkflowNode::DataTransformer(d) = node
                && d.base.id == "const-scoring-period"
            {
                d.config.expression = "\"weekly\"".into();
                patched = true;
            }
        }
        assert!(patched, "超短档模板里没有 const-scoring-period ⇒ 本负控失去前提");
        let problems = scoring_period_const_problems(Period::UltraShort, &nodes);
        assert!(
            problems.iter().any(|p| p.contains("weekly")),
            "把常量改成周线尺度后仍不报 ⇒ 尺度一致性判据没电：{problems:?}"
        );
    }

    /// 分支节点必须读**本档那一格**风险档（v128 B1 的按档接线，判据随节点一起搬进来）。
    ///
    /// 载体变了、判据没退役：原来这条住在 `mod.rs` 的
    /// `seeded_template_carries_horizon_scoped_risk_nodes`（拿落库的主图节点判
    /// `pm-h-<档>.input_mapping.overall_risk == cls-risk-level-<档>.result.category`），
    /// 而 v135 起 `pm-h-<档>` 是父图的 SubWorkflow 扇出、这条映射在**子模板**里 ⇒ 断言跟着
    /// 换载体，父侧那半（扇出是否把本档风险节点的产出传进子快照）仍由 `mod.rs` 那条门守。
    ///
    /// 风险节点本身**留在父图**（四份逐字定义里的偏离说明见本文件头部）：它对外的
    /// 唯一通路是扇出的恒等键，所以「读本档那一格」现在发生在子快照里。
    fn risk_cell_problems(period: Period, nodes: &[WorkflowNode]) -> Vec<String> {
        let suffix = period.as_str().replace('_', "-");
        let own = format!("cls-risk-level-{suffix}.result.category");
        let branch_id = format!("pm-h-{suffix}");
        let got = nodes.iter().find_map(|n| match n {
            WorkflowNode::Code(c) if c.base.id == branch_id => {
                c.config.input_mapping.get("overall_risk").cloned()
            },
            _ => None,
        });
        match got {
            None => vec![format!("{branch_id} 分支节点里没有 overall_risk 映射 ⇒ 判据失去对象")],
            Some(v) if v != own => vec![format!(
                "{branch_id} 的 overall_risk = {v}，应为 {own} —— 读错档就等于四份复制"
            )],
            Some(_) => Vec::new(),
        }
    }

    /// 正断言：四档各自读本档那一格。
    #[test]
    fn tier_branch_reads_own_risk_cell() {
        for period in Period::ALL {
            let (nodes, _) = horizon_tier_template_nodes(period);
            let problems = risk_cell_problems(period, &nodes);
            assert!(problems.is_empty(), "{problems:?}");
        }
    }

    /// 负控：把超短分支的 `overall_risk` 改回读**全局**那一格 ⇒ 必须红。
    ///
    /// 这一条锁的是「按档接线」本身而不是「节点存在」——四份复制恰好长成正断言的样子，
    /// 没有负控就检不出原始缺陷（同 `mod.rs` 那条门的负控 ①，只是样本换成了子模板节点）。
    #[test]
    fn branch_reading_global_risk_cell_is_caught() {
        let (mut nodes, _) = horizon_tier_template_nodes(Period::UltraShort);
        assert!(risk_cell_problems(Period::UltraShort, &nodes).is_empty(), "前提被破坏");
        let mut patched = false;
        for node in &mut nodes {
            if let WorkflowNode::Code(c) = node
                && c.base.id == "pm-h-ultra-short"
            {
                c.config
                    .input_mapping
                    .insert("overall_risk".into(), "cls-risk-level.result.category".into());
                patched = true;
            }
        }
        assert!(patched, "超短模板里没有 pm-h-ultra-short ⇒ 本负控失去前提");
        let problems = risk_cell_problems(Period::UltraShort, &nodes);
        assert!(
            problems.iter().any(|p| p.contains("cls-risk-level.result.category")),
            "改回读全局格后仍绿 ⇒ 按档接线没被锁住：{problems:?}"
        );
    }
}
