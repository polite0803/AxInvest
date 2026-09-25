//! 快速趋势智选（`serenity-screening-fast`）模板种子 —— 消除原链运行时间过长的问题。
//!
//! ## 三条硬约束
//!
//! - **H1**：不改原链（`serenity-screening`）的图 —— 本文件**只读**复用
//!   `seed_serenity.rs` 的构建器 / 工具集 / variables，原链节点与边一行不动。
//! - **H2**：脚本零副本零版本分支 —— `strategy-scorer.rhai` / `consistency-check.rhai` /
//!   `data-verifier.rhai` 两链 `include_str!` **同一份文件**，差异只落在节点
//!   `input_mapping`（注入哪条上游 → 给出什么形态的入参）。
//! - **H3**：落库口径与原链一致（候选/置信度/价位字段名不变），前端与持久化零改。
//!
//! ## 原链慢在哪 → 快速链如何消除
//!
//! 原链 21 个节点里有 **11 个 Agent 串行/并发腿**（1 trend-scanner + 5 chain-decomposer +
//! 5 chokepoint-identifier）与 **1 个候选映射 Agent**，每条腿都是一次完整 LLM 生成
//! （`max_tool_rounds: 8`、`timeout: 600s`）＋ 各自的多轮工具调用。
//! 快速链把「非 Agent 部分」全部确定性化并只保留 **1 个生成 Agent**：
//!
//! ```text
//! 段 A 数据采集（4 Tool，并行）      ← 复用原链同名 ToolDef / 同一批数据工具
//! 段 B c-scanner-brief（Rhai）       ← 行业排名+快讯+政策 → brief/keyword/hot_industries（零 token）
//! 段 C 3 个 LlmClassifier（Jev 决策） ← 策略类型 / 瓶颈判定 / 候选首选（判定而非生成）
//! 段 D t-candidate-search + c-candidate-pool（Rhai） ← 确定性候选池（零 token）
//! 段 E a-trend-scanner（唯一生成 Agent） ← 趋势识别 + 产业链拆解 + 候选筛选**一次性完成**
//! 段 F 5×c-scorer + c-consistency-check + c-data-verifier（Rhai）
//!        + a-candidate-mapper（Code，替原链的候选映射 Agent）
//! ```
//!
//! 相对原链的节点级替换（§8 三项裁决）：
//! 1. **11 个 Agent 腿 → 1 个**（`a-trend-scanner` 沿用 id，原链的 5×`a-chain-trend{i}`
//!    与 `a-candidate-mapper`(Agent) 全部下沉为确定性脚本或合入该 Agent 的输出）
//! 2. `a-candidate-mapper` **沿用 id**，类型由 Agent 改为 Code(Rhai)
//!    （`serenity.rs` 按 id 硬读该节点的输出，改名会断链）
//! 3. 两链并存、两个按钮独立触发 —— 本模板 id 与原链不同，互不影响
//!
//! ## ⚠️ 两处实施期偏差（相对原始 PLAN，已向用户汇报）
//!
//! 1. `j-bottleneck`：PLAN 写的是 `Condition(judge_by_llm)`，改为 **LlmClassifier(2 类)**。
//!    原因：`condition_executor` 会把 `context.variables` **全量**拼进 prompt
//!    （4 个 Tool 的原始 JSON 约 20–35KB）⇒ 直接引爆 Jev 的 32k 上下文预算。
//!    LlmClassifier 只取 `input_var` 指向的那一个值（这里是 `brief`），可控。
//! 2. 新增 `c-trend-split` 节点 + `trend-split.rhai`：引擎的 `resolve_var_path`
//!    不支持数组下标，PLAN 原文的 `a-trend-scanner.content.trends[{i}]` 恒 resolve 为
//!    unit。改为确定性脚本把 `trends[]` 切成 `trend1..trend5` 五个**恒存在**的具名键，
//!    下游一律走纯对象导航。（详细论证见 `trend-split.rhai` 头部注释。）
//!
//! ## 复用（禁止重复定义）
//!
//! 构建器 / 工具集 / variables 一律取自 `super::seed_serenity`（两链共用同一份定义），
//! 本文件不自建任何同义构建器或工具表。

use crate::commands::error_code::stock_setup;
use crate::commands::stock_analysis_setup::seed_serenity::{
    SERENITY_CANDIDATE_TOOLS, SERENITY_CHAIN_TOOLS, SERENITY_PHASE0_TOOLS,
    build_serenity_tool_defs, build_serenity_variables, serenity_agent_node, serenity_code_node,
    serenity_edge, serenity_resolve_tools, serenity_tool_node,
};
use axagent_harness::workflow_types::{
    LlmClassifierNode, LlmClassifierNodeConfig, Position, RetryConfig, TriggerConfig, TriggerNode,
    TriggerType, WorkflowEdge, WorkflowNode, WorkflowNodeBase,
};
use std::collections::HashMap;

/// 模板 id —— 与原链 `serenity-screening` 并存，两个按钮各自触发。
const TEMPLATE_ID: &str = "serenity-screening-fast";

/// 模板版本（新建模板，从 1 起）。
///
/// ⚠️ 升版判据是「**严格大于 DB 现值**」，不是「比上一版 +1」——原链历史上踩过
/// `existing.version (58) >= TEMPLATE_VERSION (55)` 导致改动一字不落库的坑
/// （见 `seed_stock_analysis.rs` v55 段）。
///
/// ⚠️⚠️ 本模板行是**用户可写**的（设置面板的 `apply_update_variable` 会双写两链，
/// 见 `SerenityScreeningPanel.tsx` 的 `handleSerenityVarChange`），而
/// `axagent_dao::repo::workflow_template::update_workflow_template` 在每次保存时
/// 都会 `version = 现值 + 1`（`crates/dao/src/repo/workflow_template.rs` 的
/// `active_model.version = Set(t.version + 1)`）。**因此 DB 现值会随用户改设置而
/// 自行抬高，与代码里的这个常量无关。**
/// 注意 `seed_serenity.rs` 里「前端保存不递增 version」的注释已与现行 dao 代码不符
/// （原文陈述的是 2026-07-31 的一次改法，后续被改回），**不要据该注释推断本门稳定**。
///
/// ⚠⚠⚠ **不要为了「让本门可靠」而去改 dao、删掉 `version = 现值 + 1`** —— 已取证
/// （2026-09-24），那个递增是**负载承载的**，删它等于砍功能：
///
/// 1. 它是**从 upstream master 拉回来的**，不是本地临时措施：`git log -S 't.version + 1'`
///    命中 `488cf9896 🐛 fix: 系统模板列表空 — dao upsert 补 route_path/cluster_id 更新 +
///    版本号递增触发重灌`（2026-08-28），提交正文写明「从 upstream master 拉取完整 upsert 实现」。
/// 2. 快照表 `workflow_template_version` 的主键是 `{id}_v{version}` +
///    `on_conflict_do_nothing`（`crates/dao/src/repo/workflow_template.rs` 的
///    「D9: save old version as a snapshot before updating」段）—— 若保存不递增，
///    快照永远只有种子那一版，版本历史无从区分。
/// 3. 前端**真实依赖**版本历史与恢复：`workflowStore.ts` 的 `getVersionHistory` /
///    `restoreVersion` 与 `workflowEditorStore.ts` 的恢复分支，都消费
///    `get_template_versions` / `get_template_by_version`。
///
/// ## 版本门的两条判据（`version` + 图谱指纹，缺一不可）
///
/// 设计意图是**两条并存**的约束，偏废任一条都会坏：
///
/// 1. **用户修改优先** —— 用户在工作流编辑器里改图、或在设置面板改变量，保存都会经
///    `update_workflow_template` 把 `version` 抬到「现值 + 1」。故一旦 DB `version`
///    **高于**本常量，本函数就永不覆盖它（只跳过，必要时告警）。这正是「用户保存必须
///    递增版本」这条设计的用途 —— **不要为了「让升版更可靠」去删 dao 的递增**。
/// 2. **能重种子化** —— 代码改了图必须落库。判据分两层，`version` 与图谱指纹各管一半：
///    - `DB version < 本常量`：显式升版 ⇒ 重建；
///    - `DB version == 本常量`：DB 仍是本函数上次写入的形态（用户从未保存过）
///      ⇒ 图谱一致即幂等跳过；**图谱不一致只能是代码改了图 ⇒ 自动重建**
///      （不必人工查 DB 现值，这才是「能重种子化」的常态路径）。
///
/// 唯一需要人工介入的情形是**两者同时成立**：用户已保存过（`DB version > 本常量`）
/// **且**图谱与代码不一致 —— 此时无法从 DB 状态区分「用户改的图」与「代码改的图」，
/// 按「用户修改优先」保守跳过并打响亮 warn。要落库代码这一版，就把本常量设为
/// DB 现值 +1（也可直接改为 `i32::MAX` 语义的显式重灌，但那会连用户变量一起覆盖）：
/// ```sql
/// SELECT version FROM workflow_templates WHERE id='serenity-screening-fast';
/// ```
const TEMPLATE_VERSION: i32 = 1;

pub(crate) async fn seed_serenity_fast_workflow_template(
    db: &sea_orm::DatabaseConnection,
) -> Result<(), String> {
    use crate::commands::error::ErrorResponse;
    use axagent_entities::workflow_template;
    use sea_orm::{ActiveModelTrait, EntityTrait, Set};

    let now = chrono::Utc::now().timestamp_millis();

    // ── 版本门（前置查询：只登记候选，**不在此处 return**）──
    // 判据与原链**完全一致**（见 `seed_serenity.rs`「版本门裁决」段）：`version` + 图谱指纹
    // 三态裁决，两链共用同一套语义。之所以在本处只查询、不裁决 —— 图谱指纹要等 nodes/edges
    // 序列化之后才在手上，故裁决一律推迟到下方「版本门裁决」段。
    // 代价是「已最新」时也要走一遍建图（微秒级），换来的是「代码改了图却静默不落库」
    // 这类失效可见。
    let existing_row =
        workflow_template::Entity::find_by_id(TEMPLATE_ID).one(db).await.map_err(|e| {
            ErrorResponse::new(stock_setup::INTERNAL)
                .with_detail(format!("查询快速趋势智选模板失败: {e}"))
        })?;
    // 这里**不打任何「跳过 / 更新」日志**：只有图谱比对完成后才能给出确定结论，
    // 日志一律交给下方「版本门裁决」段，避免同一件事在两处各说一遍。
    if existing_row.is_none() {
        tracing::info!("[stock_analysis_setup] 快速趋势智选模板不存在，准备创建");
    }

    // ── 共用资产（H2：与原链同一份定义，零副本）──
    let tool_defs = build_serenity_tool_defs();
    let tool_defs_json = serde_json::to_string(&tool_defs).map_err(|e| {
        ErrorResponse::new(stock_setup::INTERNAL).with_detail(format!("序列化 ToolDef 失败: {e}"))
    })?;
    let tool_node = serenity_tool_node;
    let agent_node = serenity_agent_node;
    let code_node = serenity_code_node;
    let edge = serenity_edge;

    // Jev 决策模型（供段 C 的 3 个 LlmClassifier 使用；未配置则留空回落默认模型）
    let decision_model = resolve_decision_model(db).await;

    let mut nodes: Vec<WorkflowNode> = Vec::new();
    let mut edges: Vec<WorkflowEdge> = Vec::new();

    // ═══════════════════════════════════════════════
    // Trigger：与原链同形（手动触发，无参数）
    // ═══════════════════════════════════════════════
    nodes.push(WorkflowNode::Trigger(TriggerNode {
        base: WorkflowNodeBase {
            id: "trigger".into(),
            title: "启动快速趋势智选".into(),
            description: Some("确定性简报 + 单 Agent 生成，快速扫描产业瓶颈机会".into()),
            position: Position { x: 340.0, y: 0.0 },
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
                "description": "快速趋势智选: 确定性市场简报 + 单 Agent 完成趋势/产业链/候选",
                "required_params": []
            }),
        },
    }));

    // ═══════════════════════════════════════════════
    // 段 A：数据采集（4 Tool 并行，与原链同名同工具）
    // ═══════════════════════════════════════════════
    // 复用原链的 4 个 Phase-0 数据工具（同一批 ToolDef）。t-policy-news 沿用原链特例：
    // 关键词走模板变量 `policy_news_keywords`（tool_executor 只解析变量路径，
    // 写字面值会查表失败 → null，这是原链 v19 连续 6 轮 keyword= 空的根因），
    // 且 `continue_on_fail: true`（政策新闻抓不到不该阻断整条链）。
    let t_names = [
        ("t-industry-rank", "行业排名", "get_industry_ranking", 240.0, 80.0),
        ("t-cls-flash", "实时快讯", "get_cls_flash", 440.0, 80.0),
        ("t-northbound", "北向资金", "get_north_bound_flow", 640.0, 80.0),
        ("t-policy-news", "政策新闻", "search_news", 840.0, 80.0),
    ];
    let mut phase0_ids: Vec<String> = Vec::new();
    for (id, title, tool_name, x, y) in t_names {
        let mut node = tool_node(id, title, tool_name, id, x, y);
        if id == "t-policy-news" {
            if let WorkflowNode::Tool(t) = &mut node {
                t.config
                    .input_mapping
                    .insert("keyword".to_string(), "policy_news_keywords".to_string());
                t.base.description = Some("搜索近期政策类新闻".to_string());
                t.base.continue_on_fail = true;
                t.base.timeout = Some(120);
            }
        }
        nodes.push(node);
        phase0_ids.push(id.to_string());
        edges.push(edge(&format!("e-trigger-{id}"), "trigger", id));
    }

    // ═══════════════════════════════════════════════
    // 段 B：确定性市场简报（Rhai，零 token）
    // ═══════════════════════════════════════════════
    // 替代原链 a-trend-scanner 里「读行业排名 → 定阈值 → 选行业」这段**纯算术**工作。
    // 输出 brief（人读）/ keyword（供 search_stock）/ hot_industries / excluded。
    let brief_code = include_str!("../scanner-brief.rhai").to_string();
    let mut brief_map = HashMap::new();
    brief_map.insert("industry_ranking".to_string(), "t-industry-rank.result".to_string());
    brief_map.insert("cls_flash".to_string(), "t-cls-flash.result".to_string());
    brief_map.insert("policy_news".to_string(), "t-policy-news.result".to_string());
    brief_map.insert("user_themes".to_string(), "user_themes".to_string());
    nodes.push(code_node(
        "c-scanner-brief",
        "市场简报",
        "确定性简报：行业排名严格档 + 领涨股 + 政策新闻要点",
        &brief_code,
        brief_map,
        240.0,
        200.0,
        15,
    ));
    for sid in ["t-industry-rank", "t-cls-flash", "t-policy-news"] {
        edges.push(edge(&format!("e-{sid}-c-scanner-brief"), sid, "c-scanner-brief"));
    }

    // ═══════════════════════════════════════════════
    // 段 C：3 个 LlmClassifier（Jev 决策模型 = 判定，不是生成）
    // ═══════════════════════════════════════════════
    // 全部走 `input_var` 单值注入（**不得留空** —— 留空会把全部 variables 拼进
    // prompt，4 个 Tool 的原始 JSON 直接撑爆 32k 预算）。
    // 全部配 `fallback_label` + `continue_on_fail: true`，双向兜底：
    // ① LlmClassifierExecutor 在 LLM 调用失败时降级输出 fallback_label（不报错、不阻塞下游）；
    // ② 即便节点彻底失败，`continue_on_fail` 也不阻塞下游。

    // 策略类型：类别**就是 6 个策略码本身**（消费端 strategy-scorer.rhai 按码分支，
    // 中文解释只能放 prompt —— 匹配逻辑是「精确 → 包含 → 原样文本」，
    // 类别写成中文解释会命中「原样文本」回退，把中文塞进 trend_strategy）。
    let strategy_cats = vec![
        "bottleneck".to_string(),
        "policy".to_string(),
        "earnings".to_string(),
        "capital".to_string(),
        "event".to_string(),
        "technical".to_string(),
    ];
    nodes.push(classifier_node(
        "j-strategy-type",
        "策略类型判定",
        "你是 A 股产业趋势的策略分类器。根据市场简报判断当前主导策略：\n\
         - bottleneck：产业链供给瓶颈（**默认选项**，市场简报未明确指向其它驱动时一律选它）\n\
         - policy：政策驱动（简报政策要点中出现明确的行业级扶持/规划落地）\n\
         - earnings：业绩驱动（简报出现明确的业绩超预期/订单落地）\n\
         - capital：资金面驱动（简报显示主力资金持续大幅净流入某行业）\n\
         - event：事件驱动（简报出现并购/重组/大单等事件）\n\
         - technical：技术形态驱动（简报出现放量突破形态）\n\
         不确定时输出 bottleneck。",
        strategy_cats,
        None,
        "c-scanner-brief.result.brief",
        Some("bottleneck"),
        None,
        decision_model.clone(),
        100.0,
        320.0,
    ));

    // 瓶颈判定：2 类互斥，结果作为**参考输入**喂给生成 Agent（不参与确定性路由）。
    nodes.push(classifier_node(
        "j-bottleneck",
        "瓶颈存在性判定",
        "判断下述市场简报所反映的产业方向，是否存在**供给瓶颈**（供给刚性/高壁垒/扩产周期长，\
         而非单纯需求旺盛）。只回答类别名称。",
        vec!["存在供给瓶颈".to_string(), "不存在供给瓶颈".to_string()],
        None,
        "c-scanner-brief.result.brief",
        Some("不存在供给瓶颈"),
        None,
        decision_model.clone(),
        300.0,
        320.0,
    ));
    edges.push(edge("e-c-scanner-brief-j-strategy-type", "c-scanner-brief", "j-strategy-type"));
    edges.push(edge("e-c-scanner-brief-j-bottleneck", "c-scanner-brief", "j-bottleneck"));

    // ═══════════════════════════════════════════════
    // 段 D：确定性候选池（Tool + Rhai，零 token）
    // ═══════════════════════════════════════════════
    // t-candidate-search 用简报的 keyword 检索股票，c-candidate-pool 汇总
    // 「检索命中 + 各行业领涨股」两路来源，去 ST/退市/非 6 位代码，截断 20 只。
    let mut search_node = tool_node(
        "t-candidate-search",
        "候选检索",
        "search_stock",
        "t-candidate-search",
        520.0,
        320.0,
    );
    if let WorkflowNode::Tool(t) = &mut search_node {
        t.config
            .input_mapping
            .insert("keyword".to_string(), "c-scanner-brief.result.keyword".to_string());
        t.base.description = Some("按简报关键词检索候选股票".to_string());
        // 检索失败不该阻断：候选池还会从行业领涨股这一路补齐
        t.base.continue_on_fail = true;
    }
    nodes.push(search_node);

    let pool_code = include_str!("../candidate-pool.rhai").to_string();
    let mut pool_map = HashMap::new();
    pool_map.insert("search_results".to_string(), "t-candidate-search.result".to_string());
    pool_map.insert("scanner_brief".to_string(), "c-scanner-brief.result".to_string());
    nodes.push(code_node(
        "c-candidate-pool",
        "候选池",
        "确定性候选池：检索命中 + 行业领涨股，去 ST/退市并截断",
        &pool_code,
        pool_map,
        520.0,
        430.0,
        15,
    ));
    edges.push(edge(
        "e-c-scanner-brief-t-candidate-search",
        "c-scanner-brief",
        "t-candidate-search",
    ));
    edges.push(edge(
        "e-t-candidate-search-c-candidate-pool",
        "t-candidate-search",
        "c-candidate-pool",
    ));
    // 直连边：满足端口公理（候选池另需 brief 的 hot_industries 领涨股）并确立调度依赖
    edges.push(edge("e-c-scanner-brief-c-candidate-pool", "c-scanner-brief", "c-candidate-pool"));

    // 候选首选：`categories_var` 从候选池动态读 labels（空/读取失败时回落静态兜底项）。
    // labels 不带序号 —— LlmClassifierExecutor 会自己加 "N. " 前缀
    // （llm_classifier_executor.rs:76-81），脚本再带序号会出现双重编号。
    nodes.push(classifier_node(
        "j-candidate-pick",
        "候选首选",
        "从候选列表中选出与当前产业趋势最匹配、且最可能处于瓶颈环节的一只标的。\
         只输出候选项原文（形如 `600036 招商银行`），不要输出任何其它内容。",
        vec!["无候选".to_string()],
        Some("c-candidate-pool.result.labels"),
        "c-candidate-pool.result.pool_text",
        Some("无候选"),
        None,
        decision_model.clone(),
        520.0,
        540.0,
    ));
    edges.push(edge("e-c-candidate-pool-j-candidate-pick", "c-candidate-pool", "j-candidate-pick"));

    // ═══════════════════════════════════════════════
    // 段 E：唯一生成 Agent（趋势识别 + 产业链拆解 + 候选筛选 一次完成）
    // ═══════════════════════════════════════════════
    // 节点 id **沿用 `a-trend-scanner`**：`serenity.rs` 的 trends 通路按该 id 读输出。
    // 该 Agent 取代原链的 1 trend-scanner + 5 chain-decomposer + 5 chokepoint-identifier
    // + candidate-mapper 共 12 次 LLM 生成，是快速链省时的主来源。
    let merged_prompt = r#"你的任务：基于上游**确定性**结果，一次性完成「趋势识别 → 产业链拆解 → 候选筛选」三步，
输出不超过 3 个产业趋势，每个趋势自带 chain_nodes（产业链环节）与 candidates（候选标的）。

## 输入（均为上游确定性结果，优先采信，不要自行编造行业）
- `brief`：确定性市场简报（行业排名严格档 + 领涨股 + 政策新闻要点）。**行业选择以它为准。**
- `industry_ranking`：原始行业排名（涨跌幅 / 主力净流入），用于交叉验证。
- `policy_news`：政策类新闻原文。
- `strategy_type`：上游判定的主导策略（bottleneck/policy/earnings/capital/event/technical）。
- `bottleneck_verdict`：上游对「是否存在供给瓶颈」的判定（仅作参考）。
- `candidate_pool`：确定性候选池（代码 + 名称，已去 ST/退市）。
- `top_pick`：上游从候选池挑出的首选标的（可能为空）。
- `user_themes`：**用户指定主题**。非空时必须优先围绕该主题展开，行业排名/快讯仅作辅助验证，
  不因数据缺失而拒绝输出。

## 效率约束（本工作流的核心目标：快 —— 违反将失去存在意义）
1. 趋势数 **不超过 3 个**（brief 无值得分析的方向 → 返回空 trends）。
2. 每个趋势的 chain_nodes **不超过 5 个环节**。
3. 每个趋势的 candidates **不超过 2 只**。
4. **工具调用总数不超过 6 次**：优先 search_stock / get_stock_financials / get_stock_quote；
   信息不足时按行业常识估算并在对应字段标 `"estimated": true`，严禁对同一标的重复调用。
5. 数据不可用（industry_ranking 为空或全为 0）时，若 user_themes 也为空 →
   直接返回 {"trends": [], "summary": "实时市场数据不可用，无法识别趋势"}，不要用训练知识编造。

## 第一步：趋势识别
- 行业来源：`brief` 中「涨幅 2-15% 且主力净流入为正」的萌芽→加速行业；user_themes 非空时优先用户主题。
- 排除：brief 中被标为「涨幅过高(>15%)，已非萌芽期」的行业。
- 每个趋势必须给出：trend_name / confidence(0-100) / core_logic / causal_chain(上下游因果链) /
  strategy_type / bottleneck_candidate / bottleneck_rationale / demand_evidence / downstream_giants。
- 每个趋势必须有可验证的 CapEx/订单/政策证据支撑，纯推测不可接受。

## 第二步：产业链拆解（chain_nodes）
- 拆到具体产品或工艺层面（如 HBM3E 环氧塑封料），不超过 5 个关键环节。
- **无论 strategy_type 是什么，都必须给出 chain_nodes** —— 下游确定性评分只消费它。
- 每个环节必须标注：node_name / global_supplier_count / top3_market_share / 
  tech_barrier(high|medium|low) / expansion_cycle_months / bottleneck_potential(high|medium|low) /
  bottleneck_rationale。
- 需求验证：demand_validation{direct_downstream, final_demand_driver,
  demand_certainty(high|medium|low), evidence, order_visibility}。
- 代表公司财务：financial_data{gross_margin, revenue_growth_yoy, debt_ratio, roe,
  rnd_ratio, capex_dep_ratio}。**A 股必须用 get_stock_financials 取真实数据**；
  非 A 股（如 NVIDIA、台积电）或工具失败时给合理估算，并加 `"estimated": true`。
  严禁编造不存在的财务数字。

## 第三步：候选筛选（candidates）
- 来源优先级：`candidate_pool`（确定性池）→ chain_nodes 中的代表性公司 → top_pick。
- 每只候选必须给出：stock_code(6 位 A 股代码) / stock_name / relevance(core|related) /
  serenity_score(0-100) / confidence(0-100) / bottleneck_product / primary_risk / catalysts[] /
  exit_signals{technology_disruption_risk, capacity_oversupply_risk, new_entrant_risk,
  demand_slowdown_risk, overall_exit_urgency} /
  attention_metrics{} / financial_snapshot{}。
- **不要写 strategy_type 字段**（策略类型由上游节点决定，写了会被下游丢弃）。
- 拿不出真实依据的标的不要列。

## 输出格式强约束（必须严格遵守）
1. 你的回复必须且只能包含一个代码块，开头三个反引号紧跟 tool_json。
2. 代码块内容为单一 JSON 对象，结构：{"name": "submit_trends", "arguments": <数据>}。
3. <数据> 结构：
   {"trends": [{"trend_name": "...", "confidence": 75, "core_logic": "...", "causal_chain": "...",
     "strategy_type": "bottleneck | policy | earnings | capital | event | technical",
     "bottleneck_candidate": "...", "bottleneck_rationale": "...",
     "demand_evidence": {"type": "capex | policy_mandate | order_backlog", "source": "...",
       "confidence": 75, "detail": "..."},
     "downstream_giants": ["..."],
     "chain_nodes": [{"node_name": "...", "global_supplier_count": 3, "top3_market_share": 65,
       "tech_barrier": "high", "expansion_cycle_months": 24, "bottleneck_potential": "high",
       "bottleneck_rationale": "...",
       "financial_data": {"gross_margin": 45.0, "revenue_growth_yoy": 25.0, "debt_ratio": 35.0,
         "roe": 12.0, "rnd_ratio": 15.0, "capex_dep_ratio": 3.5},
       "demand_validation": {"direct_downstream": "...", "final_demand_driver": "...",
         "demand_certainty": "high", "evidence": "...", "order_visibility": "..."}}],
     "candidates": [{"stock_code": "600036", "stock_name": "...", "relevance": "core",
       "serenity_score": 78, "confidence": 70, "bottleneck_product": "...", "primary_risk": "...",
       "catalysts": ["..."],
       "exit_signals": {"technology_disruption_risk": "low", "capacity_oversupply_risk": "low",
         "new_entrant_risk": "low", "demand_slowdown_risk": "medium",
         "overall_exit_urgency": "low"},
       "attention_metrics": {"institutional_visits": 3, "research_reports": 5,
         "news_heat": "medium"},
       "financial_snapshot": {"pe": 25.0, "pb": 3.2}}]}],
    "summary": "最终判断总结；trends 为空时说明具体原因"}
4. 代码块外禁止任何文字：不要写「以下是」「输出：」、注释、解释、前缀、后缀。
5. 字段值为空时用 null，不要省略字段；数字字段必须是 JSON 数字，不要加引号。
6. 严禁在 JSON 字符串值中夹带思考文字或自述注解。

## ⚠️ 终极约束（违反将导致整个工作流报废）
1. 你**唯一**合法的输出方式是一个 tool_json 代码块，绝不输出任何自然语言。
2. 「抱歉」「我无法回答」「数据不足」等自然语言会直接破坏下游所有节点。不要这样做。
3. 无法识别趋势时返回 {"trends": [], "summary": "原因"} —— 这是合法的结构化输出。
4. 你是一个函数，你的输出必须是 JSON。你不是在对话，你是在向系统返回值。"#;

    let mut ts_input_mapping = HashMap::new();
    ts_input_mapping.insert("industry_ranking".to_string(), "t-industry-rank.result".to_string());
    ts_input_mapping.insert("policy_news".to_string(), "t-policy-news.result".to_string());
    ts_input_mapping.insert("user_themes".to_string(), "user_themes".to_string());
    ts_input_mapping.insert("brief".to_string(), "c-scanner-brief.result.brief".to_string());
    ts_input_mapping.insert("strategy_type".to_string(), "j-strategy-type.category".to_string());
    ts_input_mapping.insert("bottleneck_verdict".to_string(), "j-bottleneck.category".to_string());
    ts_input_mapping
        .insert("candidate_pool".to_string(), "c-candidate-pool.result.pool_text".to_string());
    ts_input_mapping.insert("top_pick".to_string(), "j-candidate-pick.category".to_string());

    let mut trend_agent = agent_node(
        "a-trend-scanner",
        "趋势识别与产业链拆解",
        "trend-scanner",
        merged_prompt,
        // `serenity_agent_node` 的 `context_sources` 形参是 `Vec<&str>`，
        // 而 `phase0_ids` 是 `Vec<String>`（下游 502 行还要按 String 复用）⇒ 在此借用转换。
        phase0_ids.iter().map(String::as_str).collect(),
        ts_input_mapping,
        340.0,
        660.0,
    );
    // ⚠️ 工具集必须**在此直接赋值**，不能沿用原链「按 agent_profile_id 后处理」的做法
    // —— 该 Agent 的 profile 是 `stock-trend-scanner`，后处理循环会把它覆盖成
    // 仅 PHASE0 的 5 个工具，而本 Agent 还要做产业链拆解与候选筛选。
    // 合并集 = PHASE0 ∪ CHAIN ∪ CANDIDATE（保序去重，16 个）。
    if let WorkflowNode::Agent(a) = &mut trend_agent {
        let mut merged_names: Vec<&str> = Vec::new();
        for name in SERENITY_PHASE0_TOOLS
            .iter()
            .chain(SERENITY_CHAIN_TOOLS.iter())
            .chain(SERENITY_CANDIDATE_TOOLS.iter())
        {
            if !merged_names.contains(name) {
                merged_names.push(*name);
            }
        }
        a.config.tools = serenity_resolve_tools(&tool_defs, &merged_names);
        // 原链是 8 轮 —— 快速链收到 3 轮（效率约束已把工具调用总预算压到 6 次）
        a.config.max_tool_rounds = Some(3);
    }
    nodes.push(trend_agent);
    for tid in &phase0_ids {
        edges.push(edge(&format!("e-{tid}-a-trend-scanner"), tid, "a-trend-scanner"));
    }
    for jid in ["j-strategy-type", "j-bottleneck", "j-candidate-pick"] {
        edges.push(edge(&format!("e-{jid}-a-trend-scanner"), jid, "a-trend-scanner"));
    }

    // ═══════════════════════════════════════════════
    // 段 F：确定性评分 + 一致性检查 + 候选组装 + 数据校验
    // ═══════════════════════════════════════════════
    // ① c-trend-split：把 Agent 输出的 trends[] 切成 trend1..trend5 具名槽位
    //    （引擎 resolve_var_path 不支持数组下标，见 trend-split.rhai 头部论证）
    let split_code = include_str!("../trend-split.rhai").to_string();
    let mut split_map = HashMap::new();
    split_map.insert("scanner_raw".to_string(), "a-trend-scanner.content".to_string());
    nodes.push(code_node(
        "c-trend-split",
        "趋势分槽",
        "把合并腿的 trends[] 切成具名槽位（引擎不支持数组下标）",
        &split_code,
        split_map,
        340.0,
        780.0,
        15,
    ));
    edges.push(edge("e-a-trend-scanner-c-trend-split", "a-trend-scanner", "c-trend-split"));

    // ② 5 个 c-scorer：与原链 `include_str!` **同一份** strategy-scorer.rhai，
    //    差异只在 input_mapping —— 原链注入 `a-chain-{tn}.content`（改后 Agent 的字符串），
    //    快速链注入 `c-trend-split.result.trend{i}`（纯对象导航，必然 resolve）。
    let scorer_code = include_str!("../strategy-scorer.rhai").to_string();
    let trend_names = ["trend1", "trend2", "trend3", "trend4", "trend5"];
    let scorer_x = [60.0, 220.0, 380.0, 540.0, 700.0];
    let mut scorer_ids: Vec<String> = Vec::new();
    for (i, tn) in trend_names.iter().enumerate() {
        let sid = format!("c-scorer-{tn}");
        let mut m = HashMap::new();
        m.insert("chain_analysis".to_string(), format!("c-trend-split.result.trend{}", i + 1));
        m.insert("industry_ranking".to_string(), "t-industry-rank.result".to_string());
        m.insert("trend_strategy".to_string(), "j-strategy-type.category".to_string());
        m.insert("w_supply".to_string(), "w_supply".to_string());
        m.insert("w_demand".to_string(), "w_demand".to_string());
        m.insert("w_irreplace".to_string(), "w_irreplace".to_string());
        nodes.push(code_node(
            &sid,
            &format!("瓶颈评分 #{}", i + 1),
            "确定性瓶颈评分（strategy-scorer.rhai，两链共用）",
            &scorer_code,
            m,
            scorer_x[i],
            900.0,
            30,
        ));
        // 三条入边：分槽（数据）/ 行业排名（行业动量）/ 策略类型（分支选择）。
        // ⚠️ 缺任一条，Code 节点就拿不到该输入 —— code_executor 只注入 edges 直接上游
        // 的输出（get_node_dependency_results），Code 节点不支持 context_sources 软依赖
        // （那是 Agent 节点专属），原链 v42 为此补过边，此处一次到位。
        edges.push(edge(&format!("e-c-trend-split-{sid}"), "c-trend-split", &sid));
        edges.push(edge(&format!("e-t-industry-rank-{sid}"), "t-industry-rank", &sid));
        edges.push(edge(&format!("e-j-strategy-type-{sid}"), "j-strategy-type", &sid));
        scorer_ids.push(sid);
    }

    // ③ c-consistency-check：与原链同一份脚本。
    //    两侧都注入 map（快速链的 trend 槽位是对象、c-scorer 的 result 是对象）。
    //    `consistency-check.rhai` 的 `safe_parse` 只接受字符串 ⇒ 两份 map 都返回 ()，
    //    total_comparisons == 0 ⇒ consistency_score 保持初值 100。**这与原链有效行为一致**：
    //    原链的 `strategy_{tn} ← c-scorer-{tn}.result` 本来就是 Code 节点的 map，
    //    其 strategy 侧同样全是 ()、同样 total==0、同样恒为 100 ⇒ 两链一致性分相同。
    //    （不改共享脚本的理由：改了就是一个版本分支，违反 H2。）
    let cc_code = include_str!("../consistency-check.rhai").to_string();
    let mut cc_map = HashMap::new();
    for (i, tn) in trend_names.iter().enumerate() {
        cc_map.insert(format!("chain_node_{tn}"), format!("c-trend-split.result.trend{}", i + 1));
        cc_map.insert(format!("strategy_{tn}"), format!("c-scorer-{tn}.result"));
    }
    nodes.push(code_node(
        "c-consistency-check",
        "一致性检查",
        "确定性一致性检查（consistency-check.rhai，两链共用）",
        &cc_code,
        cc_map,
        380.0,
        1020.0,
        15,
    ));
    for sid in &scorer_ids {
        edges.push(edge(&format!("e-{sid}-c-consistency-check"), sid, "c-consistency-check"));
    }
    edges.push(edge("e-c-trend-split-c-consistency-check", "c-trend-split", "c-consistency-check"));

    // ④ a-candidate-mapper：**沿用 id**（serenity.rs 按 id 硬读），类型 Agent → Code(Rhai)。
    //    替代原链的 stock-candidate-mapper Agent（省掉又一次完整 LLM 生成 + 多轮工具调用）。
    let assembler_code = include_str!("../candidate-assembler.rhai").to_string();
    let mut asm_map = HashMap::new();
    asm_map.insert("scanner_raw".to_string(), "a-trend-scanner.content".to_string());
    for (i, tn) in trend_names.iter().enumerate() {
        asm_map.insert(format!("strategy_trend{}", i + 1), format!("c-scorer-{tn}.result"));
    }
    asm_map.insert("consistency".to_string(), "c-consistency-check.result".to_string());
    asm_map.insert("strategy_type".to_string(), "j-strategy-type.category".to_string());
    nodes.push(code_node(
        "a-candidate-mapper",
        "候选组装",
        "确定性候选组装（candidate-assembler.rhai）",
        &assembler_code,
        asm_map,
        380.0,
        1140.0,
        30,
    ));
    for sid in &scorer_ids {
        edges.push(edge(&format!("e-{sid}-a-candidate-mapper"), sid, "a-candidate-mapper"));
    }
    edges.push(edge("e-c-trend-split-a-candidate-mapper", "c-trend-split", "a-candidate-mapper"));
    edges.push(edge(
        "e-a-trend-scanner-a-candidate-mapper",
        "a-trend-scanner",
        "a-candidate-mapper",
    ));
    edges.push(edge(
        "e-c-consistency-check-a-candidate-mapper",
        "c-consistency-check",
        "a-candidate-mapper",
    ));
    edges.push(edge(
        "e-j-strategy-type-a-candidate-mapper",
        "j-strategy-type",
        "a-candidate-mapper",
    ));

    // ⑤ c-data-verifier：与原链同一份脚本。原链的 4 条 input_mapping 里
    //    `candidates ← a-candidate-mapper.content.arguments.candidates` 与
    //    `candidates_raw ← a-candidate-mapper.content` 是**按 Agent 输出形态**写的；
    //    快速链该节点已是 Code 节点（输出包在 `result` 内、无 `content` 字段），
    //    故路径一律穿透 `.result`。`data-verifier.rhai` 三个入口都有 `present()` 守卫
    //    ⇒ 省略 `tool_calls_made`（Code 节点无该字段）。
    let dv_code = include_str!("../data-verifier.rhai").to_string();
    let mut dv_map = HashMap::new();
    dv_map.insert("candidates".to_string(), "a-candidate-mapper.result.candidates".to_string());
    dv_map.insert(
        "candidates_direct".to_string(),
        "a-candidate-mapper.result.candidates".to_string(),
    );
    dv_map.insert("candidates_raw".to_string(), "a-candidate-mapper.result".to_string());
    nodes.push(code_node(
        "c-data-verifier",
        "候选校验",
        "确定性候选校验（data-verifier.rhai，两链共用）",
        &dv_code,
        dv_map,
        380.0,
        1260.0,
        30,
    ));
    edges.push(edge(
        "e-a-candidate-mapper-c-data-verifier",
        "a-candidate-mapper",
        "c-data-verifier",
    ));

    // ── 序列化 ──
    let nodes_json = serde_json::to_string(&nodes).map_err(|e| {
        ErrorResponse::new(stock_setup::INTERNAL).with_detail(format!("序列化节点失败: {e}"))
    })?;
    let edges_json = serde_json::to_string(&edges).map_err(|e| {
        ErrorResponse::new(stock_setup::INTERNAL).with_detail(format!("序列化边失败: {e}"))
    })?;

    // ── 版本门裁决（后置：图谱已在手，可做指纹比对）──
    // 为什么必须挪到这里：前置门只看得到 `version` 数字，看不到**图谱内容**。本模板行的
    // version 会被用户保存（设置面板 / 工作流编辑器）抬高，于是「代码改了图却没抬
    // TEMPLATE_VERSION」与「确实是最新版」在版本号上无法区分 —— 原实现两者都静默
    // `return`，改脚本 / 改拓扑 / 改 input_mapping 全部一字不落库。改用图谱指纹后，
    // 由「版本号 + 图谱是否一致」两个维度共同裁决（见下）。
    //
    // 判据（详见 TEMPLATE_VERSION 的「两条判据」段）：
    //   DB version >  常量 ⇒ 用户保存过 ⇒ **永不覆盖**（图谱不一致时告警提醒开发者）
    //   DB version == 常量 ⇒ DB 仍是本函数上次写入的形态 ⇒ 图谱一致则跳过；
    //                        不一致只能是代码改了图 ⇒ **重建**（「能重种子化」的常态路径）
    //   DB version <  常量 ⇒ 显式升版 ⇒ 重建
    //
    // ⚠ 本段必须在下方 `delete_by_id` **之前**：一旦先删行再跳过，模板会凭空消失。
    if let Some(existing) = &existing_row {
        // 判据：两侧都按 JSON 结构比较（见 `super::same_json`），不比字符串 ——
        // DB 文本可能来自旧序列化器或工作流编辑器保存，键序 / 空白 / 浮点写法都可能不同。
        let graph_same = super::same_json(&existing.nodes, &nodes_json)
            && super::same_json(&existing.edges, &edges_json);

        // ① 用户保存过 ⇒ 用户修改优先，本次绝不覆盖它。
        if existing.version > TEMPLATE_VERSION {
            if graph_same {
                tracing::info!(
                    "[stock_analysis_setup] 快速趋势智选模板 v{} 已高于代码 v{TEMPLATE_VERSION}\
                     （用户已保存过，图谱一致），跳过",
                    existing.version
                );
            } else {
                tracing::warn!(
                    "[stock_analysis_setup] ⚠ 快速趋势智选模板 v{} 高于代码 v{TEMPLATE_VERSION}，\
                     且图谱与代码不一致。二者之一：(a) 用户在工作流编辑器里改过本图 —— 正常，\
                     忽略本条；(b) 代码已改图、但 TEMPLATE_VERSION 未高于 DB 现值 —— 要落库\
                     代码这一版，请把 seed_serenity_fast.rs 的 TEMPLATE_VERSION 设为 DB 现值 +1\
                     （SELECT version FROM workflow_templates WHERE id='{TEMPLATE_ID}';）。\
                     本次**跳过重建**以免覆盖用户改动。DB nodes/edges {} / {} 字节，\
                     代码 {} / {} 字节",
                    existing.version,
                    existing.nodes.len(),
                    existing.edges.len(),
                    nodes_json.len(),
                    edges_json.len()
                );
            }
            return Ok(());
        }

        // ② DB 版本等于代码常量且图谱一致 ⇒ 幂等跳过（最常见的情形）。
        if existing.version == TEMPLATE_VERSION && graph_same {
            tracing::info!(
                "[stock_analysis_setup] 快速趋势智选模板已是最新 v{TEMPLATE_VERSION}（图谱一致），跳过"
            );
            return Ok(());
        }

        // ③ 走到这里只剩两种情形，都该重建：`version < 常量`（显式升版）或
        //    `version == 常量 && 图谱不一致`（代码改了图而没抬常量）。
        //    后者是安全的：`version == 常量` 意味着用户从未保存过本行，
        //    重建不会覆盖任何用户改动 —— 这正是「能重种子化」不必人工查 DB 的路径。
        tracing::info!(
            "[stock_analysis_setup] 重建快速趋势智选模板：DB v{} → 代码 v{TEMPLATE_VERSION}（图谱{}）",
            existing.version,
            if graph_same { "一致" } else { "不一致" }
        );
    }

    // ── Variables：与原链共用同一份定义（H1/H2 —— 参数口径不产生版本分支）──
    let serenity_vars = build_serenity_variables();
    let variables_json = serde_json::to_string(&serenity_vars).map_err(|e| {
        ErrorResponse::new(stock_setup::INTERNAL).with_detail(format!("序列化变量失败: {e}"))
    })?;

    // ── Tags：与原链区分，便于前端按标签筛选 ──
    let tags_json = serde_json::to_string(&["serenity", "bottleneck", "screening", "fast"])
        .map_err(|e| {
            ErrorResponse::new(stock_setup::INTERNAL).with_detail(format!("序列化标签失败: {e}"))
        })?;

    let _ = workflow_template::Entity::delete_by_id(TEMPLATE_ID).exec(db).await;

    // P0 软门禁（C1）：端口公理结构性死链在此记录（不阻断启动），判据复用 harness。
    axagent_harness::workflow_port_axioms::warn_port_axioms_json(
        &format!("stock_analysis_setup:seed_serenity_fast:{TEMPLATE_ID}"),
        &nodes_json,
        &edges_json,
    );

    workflow_template::ActiveModel {
        hooks_config: Set(None),
        id: Set(TEMPLATE_ID.to_string()),
        cluster_id: Set(Some("trend".to_string())),
        route_path: Set(Some("/finance/trend/serenity-fast".to_string())),
        name: Set("快速趋势智选".to_string()),
        description: Set(Some(
            "快速趋势智选：确定性市场简报 + 单 Agent 一次完成趋势/产业链/候选，复用原链全部算法与工具"
                .to_string(),
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
                    "description": "快速趋势智选: 确定性简报 + 单 Agent 完成趋势/产业链/候选",
                    "required_params": []
                }),
            })
            .map_err(|e| {
                ErrorResponse::new(stock_setup::INTERNAL)
                    .with_detail(format!("序列化触发器配置失败: {e}"))
            })?,
        )),
        nodes: Set(nodes_json),
        edges: Set(edges_json),
        input_schema: Set(None),
        output_schema: Set(None),
        variables: Set(Some(variables_json)),
        error_config: Set(None),
        composite_source: Set(None),
        tool_defs: Set(Some(tool_defs_json)),
        mission_hash: Set(None),
        created_at: Set(now),
        updated_at: Set(now),
    }
    .insert(db)
    .await
    .map_err(|e| {
        ErrorResponse::new(stock_setup::INTERNAL)
            .with_detail(format!("写入快速趋势智选模板失败: {e}"))
    })?;

    tracing::info!(
        "[stock_analysis_setup] 快速趋势智选工作流模板已创建 ({TEMPLATE_ID}, {} 节点 / {} 边)",
        nodes.len(),
        edges.len()
    );
    Ok(())
}

/// LlmClassifier 节点构建器（趋势智选快速链与股票分析快速链**共用**，故不放进
/// `seed_serenity.rs` 的共用构建器 —— 那份构建器只服务各自的源链）。
///
/// 参数超 7 个（clippy `too_many_arguments` 阈值），显式 allow —— 与 `serenity_tool_node`
/// 同一处理：这是节点构造的自然参数集，包成 builder 结构体只是把同一组参数换个地方写。
///
/// `retry` 保持 `RetryConfig::default()`（`enabled: false`）：这些节点都配了
/// `fallback_label`，LLM 调用失败时执行器会**降级输出**而非报错；重试只会平白拉长耗时，
/// 与「快速链」的目标相反。`continue_on_fail: true` 做第二层兜底（节点彻底失败也不阻塞下游）。
///
/// `confidence_threshold` 的两种用法（见 `llm_classifier_executor.rs:322-352`）：
///   · `None` —— 执行器「整段文本即类别」，**不输出 `confidence`**（趋势智选段 C 三节点
///     只取 `category`，用不上置信度，也不需要额外的 JSON 解析失败面）；
///   · `Some(t)` —— 执行器走 JSON 模式（`{label, confidence}`）并在输出里带真实
///     `confidence`；`confidence < t` 时降级为 `fallback_label`。股票分析快速链的
///     `j-confidence` 要的是**数值**，故取 `Some(0.0)`：阈值 0 使该分支恒不成立，
///     既拿到 `confidence` 又不会把结果替换成兜底档。
#[allow(clippy::too_many_arguments)]
pub(crate) fn classifier_node(
    id: &str,
    title: &str,
    prompt: &str,
    categories: Vec<String>,
    categories_var: Option<&str>,
    input_var: &str,
    fallback_label: Option<&str>,
    confidence_threshold: Option<f64>,
    model: Option<String>,
    x: f64,
    y: f64,
) -> WorkflowNode {
    WorkflowNode::LlmClassifier(LlmClassifierNode {
        base: WorkflowNodeBase {
            id: id.into(),
            title: title.into(),
            description: Some(format!("LLM 分类判定: {id}")),
            position: Position { x, y },
            retry: RetryConfig::default(),
            timeout: Some(120),
            enabled: true,
            parent_id: None,
            compensation: None,
            continue_on_fail: true,
        },
        config: LlmClassifierNodeConfig {
            categories,
            categories_var: categories_var.map(str::to_string),
            prompt: prompt.into(),
            model,
            input_var: input_var.into(),
            output_var: id.into(),
            confidence_threshold,
            fallback_label: fallback_label.map(str::to_string),
            consistency_check: None,
        },
    })
}

/// 解析 Jev（TypeSafe）决策模型，返回 `providerId::modelId` 复合串
/// （`dao::repo::provider::resolve_model_for_node` 的入参格式）。
///
/// 未配置可用 TypeSafe 供应商（provider 未启用 / 无启用中的 key / 无 Decision 模型）
/// → `None` ⇒ 节点 `model` 留空，回落会话或全局默认模型。分类器仍能工作，
/// 只是不再走 Jev 决策模型（`ModelType::Decision` 的正确去处本就是
/// `llmClassifier` / `condition` 这类动态路由节点，见 `provider_model.rs` 的说明）。
///
/// ⚠️ 探测只在种子写入时跑一次，结果固化进模板 JSON ⇒ **新增/启用 TypeSafe 供应商后，
/// 必须升 `TEMPLATE_VERSION` 重建模板**，否则 DB 里那 3 个节点仍是空 `model`。
///
/// 股票分析快速链（`seed_stock_analysis.rs`）复用本函数解析同一族 Jev 决策模型。
pub(crate) async fn resolve_decision_model(db: &sea_orm::DatabaseConnection) -> Option<String> {
    use axagent_harness::types::{ModelType, ProviderType, resolve_model_type};

    let providers = axagent_dao::repo::provider::list_providers(db).await.ok()?;
    let provider = providers.iter().find(|p| {
        p.enabled && p.provider_type == ProviderType::TypeSafe && p.keys.iter().any(|k| k.enabled)
    })?;
    let model = provider.models.iter().find(|m| {
        m.enabled && matches!(resolve_model_type(provider, &m.model_id), ModelType::Decision)
    })?;
    tracing::info!(
        "[stock_analysis_setup] 快速趋势智选：LlmClassifier 使用 Jev 决策模型 {}::{}",
        provider.id,
        model.model_id
    );
    Some(format!("{}::{}", provider.id, model.model_id))
}

// ⚠ 本测试模块**必须**留在文件末尾：`clippy::items_after_test_module` 只在 clippy 下暴露
//   （`cargo check` / `cargo test` 都不跑），插在中间会让后续所有代码踩该 lint。
#[cfg(test)]
mod fast_version_gate_tests {
    use super::{TEMPLATE_ID, TEMPLATE_VERSION, seed_serenity_fast_workflow_template};
    use axagent_entities::workflow_template;
    use sea_orm::{ActiveModelTrait, DatabaseConnection, EntityTrait, Set};

    async fn fresh_db() -> axagent_dao::db::DbHandle {
        axagent_dao::db::create_test_pool().await.expect("建临时测试库失败")
    }

    async fn row(db: &DatabaseConnection) -> workflow_template::Model {
        workflow_template::Entity::find_by_id(TEMPLATE_ID)
            .one(db)
            .await
            .expect("查模板失败")
            .unwrap_or_else(|| panic!("模板 `{TEMPLATE_ID}` 应已存在"))
    }

    fn node_ids(model: &workflow_template::Model) -> Vec<String> {
        serde_json::from_str::<Vec<serde_json::Value>>(&model.nodes)
            .expect("nodes 应是 JSON 数组")
            .iter()
            .filter_map(|n| n.get("id").and_then(|v| v.as_str()).map(str::to_string))
            .collect()
    }

    async fn set_name(db: &DatabaseConnection, name: &str) {
        let mut am: workflow_template::ActiveModel = row(db).await.into();
        am.name = Set(name.to_string());
        am.update(db).await.expect("改 name 失败");
    }

    async fn set_version(db: &DatabaseConnection, version: i32) {
        let mut am: workflow_template::ActiveModel = row(db).await.into();
        am.version = Set(version);
        am.update(db).await.expect("改 version 失败");
    }

    /// 删掉一个节点（模拟「库里的图与代码产出不一致」），**不动 version**。
    async fn drop_node(db: &DatabaseConnection, node_id: &str) {
        let model = row(db).await;
        let mut nodes: Vec<serde_json::Value> =
            serde_json::from_str(&model.nodes).expect("nodes 应是 JSON 数组");
        nodes.retain(|n| n.get("id").and_then(|v| v.as_str()) != Some(node_id));
        let mut am: workflow_template::ActiveModel = model.into();
        am.nodes = Set(serde_json::to_string(&nodes).expect("序列化失败"));
        am.update(db).await.expect("改 nodes 失败");
    }

    /// 版本门三态（判据 = `version` + 图谱指纹，缺一不可）—— 四个组合逐一锁住。
    ///
    /// 用哨兵 `name` 判断「是否跳过」而非比对 `updated_at`：同一毫秒内的两次写入
    /// 无法区分，而 `name` 被覆盖是**确定**的证据。
    #[tokio::test]
    async fn version_gate_three_states() {
        let handle = fresh_db().await;
        let db = &handle.conn;
        seed_serenity_fast_workflow_template(db).await.expect("首次种子化应成功");
        assert_eq!(row(db).await.version, TEMPLATE_VERSION, "首次写入应带代码常量版本");

        // ① version == 常量 && 图谱一致 ⇒ 幂等跳过（最常见的情形）
        set_name(db, "SENTINEL-KEEP").await;
        seed_serenity_fast_workflow_template(db).await.expect("二次种子化应成功");
        assert_eq!(
            row(db).await.name,
            "SENTINEL-KEEP",
            "图谱一致时应跳过 —— 哨兵被覆盖说明每次启动都会重写模板"
        );

        // ② version == 常量 && 图谱不一致 ⇒ **自动重建**（「能重种子化」的常态路径：
        //    version 仍等于常量本身即证明用户从未保存过本行，重建不覆盖任何用户改动）
        drop_node(db, "a-trend-scanner").await;
        seed_serenity_fast_workflow_template(db).await.expect("重建应成功");
        let rebuilt = row(db).await;
        assert!(
            node_ids(&rebuilt).iter().any(|id| id == "a-trend-scanner"),
            "图谱不一致（version 仍是常量）时应自动重建"
        );
        assert_ne!(rebuilt.name, "SENTINEL-KEEP", "重建应写回代码定义的名字");

        // ③ version > 常量 && 图谱一致 ⇒ 跳过（用户保存过，版本被抬高）
        set_name(db, "SENTINEL-KEEP").await;
        set_version(db, TEMPLATE_VERSION + 1).await;
        seed_serenity_fast_workflow_template(db).await.expect("种子化应成功");
        assert_eq!(row(db).await.name, "SENTINEL-KEEP", "版本更高且图谱一致时应跳过");

        // ④ version > 常量 && 图谱不一致 ⇒ **仍然跳过**（用户修改优先）。
        //    这是唯一需要人工介入的情形：DB 状态无法区分「用户改的图」与「代码改的图」，
        //    故保守跳过并打响亮 WARN。要落库代码这一版须把 TEMPLATE_VERSION 设为 DB 现值 +1。
        drop_node(db, "a-trend-scanner").await;
        seed_serenity_fast_workflow_template(db).await.expect("种子化应成功");
        let kept = row(db).await;
        assert!(
            !node_ids(&kept).iter().any(|id| id == "a-trend-scanner"),
            "用户保存过（version 更高）时不得重建 —— 否则用户改动被静默覆盖"
        );
        assert_eq!(kept.name, "SENTINEL-KEEP", "用户保存过的行整体不得被覆盖");
    }
}
