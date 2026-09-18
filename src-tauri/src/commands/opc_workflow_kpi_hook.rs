// SPDX-License-Identifier: AGPL-3.0-only

//! OPC 工作流产出 → KPI 落库钩子（业务侧实现，运行时注册进 WorkEngine）。
//!
//! ## 为什么需要它
//!
//! 创作类 KPI（`word_count` / `completion_rate` / `revision_rounds`）的数值只在
//! 工作流执行上下文里算出来（`lc-extract-fulltext.result.char_count` 等）；
//! `opc_capability_pack_actions::run_template_via_engine` 把执行结果返回前端后即丢弃，
//! 于是 `opc_kpi_records` 长期为空、仪表盘只能显示「未接数据源」。
//!
//! 本模块实现 `WorkflowLifecycleHook`（协议见 `axagent_harness::workflow_lifecycle`），
//! 在 DAG 终态（post_exec）把产出写进 `opc_kpi_records`。读端为
//! `OpcDataService::latest_kpi` + `capability_pack_kpi_service::KpiSource::RecordedKpi`。
//!
//! ## 触发条件（零业务名硬编码）
//!
//! **只有把钩子名写进模板 `hooks_config.post_exec` 的模板才会被调用**，
//! 通用层不感知任何业务名。例如写作模板 `workflow-cm-literary-creation`：
//!
//! ```json
//! {"post_exec": ["content-media-kpi-persist"]}
//! ```
//!
//! 未注册的钩子名由引擎 `warn` 跳过、不阻断执行；本钩子内部失败也**只记日志**，
//! 不阻断（post_exec 语义：结果已产生，不可回滚）。
//!
//! ## 与 runtime.yaml 的边界
//!
//! 钩子只负责**写值**。KPI 的 name/unit 元数据以域包 `runtime.yaml` 为唯一权威；
//! 写入 `opc_kpi_records.unit` 只是让记录自描述，展示层仍以 YAML 为准。
//!
//! ## 域包归属从哪来（P2 通道，勿改成硬编码或 LLM 自报）
//!
//! `opc_kpi_records` 是**多域包共用**的一张表（v230 起有 `domain_pack_id` 列），
//! 而 `word_count` 这类 key 在不同域包下是完全不同的量 ⇒ 写入必须带域包归属，
//! 否则读侧（域包仪表盘）只能按 `name` 取最新一条、跨域包串号。
//!
//! 归属来源是**调用方注入的运行上下文**，不是模板名、不是 LLM：
//!
//! ```text
//! opc_capability_pack_actions::run_template_via_engine(domain_pack_id, …)
//!   └─ RunOptions.input = { "domain_pack_id": "content-media" }      ← 注入点
//!        └─ engine 透传 options.input → HookExecContext.input       ← 引擎已支持
//!             └─ 本钩子读 ctx.input["domain_pack_id"]                  ← 消费点（本文件）
//! ```
//!
//! 三个被否决的替代来源（各自的否决理由，避免以后有人「顺手简化」）：
//! - **硬编码 `"content_media"`**：本文件改前的写法。它让钩子只对 content_media 正确，
//!   别的域包一旦声明本钩子，值就会被写进 content_media 的桶（静默错归属）；
//!   而且它当时只用于查 unit、并不落库 —— 落库那一列根本不存在。
//! - **从 `ctx.template_id` 反推域包**：`HookExecContext.template_id` 装的是
//!   workflow **实例** id（`engine/mod.rs:1106` 的 `format!("workflow_{}", uuid)`，
//!   下划线），而模板 id 形如 `workflow-cm-*`（连字符）⇒ 前缀匹配不命中。
//! - **从 `ctx.execution_id` 回查 DB**：`workflow_executions.input_params` 里没有域包
//!   （股票管线放的是 `stock_code`），且 `execution_state_json` 只在 `pause()` 时写
//!   ⇒ 回查必空。
//!
//! 取不到域包时**不落库**（`warn` 留痕）：无归属的行在读侧查不出来，
//! 那会把「钩子没生效」伪装成「工作流没产出」，正是本文件反复要消灭的那类静默失败。
//!
//! ## 数值可信性口径（必读）
//!
//! 只落**代码算出来的**值。**LLM 自报的数字一律不作数据源**，哪怕它看起来可信 ——
//! 一个「看起来可信的假数」比缺失更糟。具体：
//! - `word_count` ← `lc-extract-fulltext.result.char_count`（Rhai `.len()` 算的字符数），
//!   降级为 `lc-draft-loop.items[*].chapter_text` 的字符数求和（**不是** `word_count`
//!   字段 —— 该键在真实元素形态里不存在）；`lc-assemble.total_word_count` 档位**已删除**。
//! - `completion_rate` 分子 ← `lc-extract-fulltext.chapter_count`（代码计数），
//!   分母 ← `lc-outline-chapters.result.chapters`（**引擎实际迭代的那份数组**），
//!   取不到即 `None`。
//! - `revision_rounds` ← `lc-tolerance-agent.content.verdict`（AgentNode 的 JSON 字符串），
//!   取值域由 prompt 明文枚举（`pass`/`rewrite`），与引擎 ConditionNode 的路由判据同源；
//!   `pass ⇒ 0`（真值）、`rewrite ⇒ 1`、读不到 ⇒ `None`。
//!
//! ## 消费端形态（本次缺陷的根因，详见 [`node_output`] 的形态表）
//!
//! 四类节点的信封**各不相同**：CodeNode 业务字段在 `.result` 对象里；ToolNode 在
//! `.result`/`.output`；**AgentNode 的信封顶层没有业务字段，它们在 `content` 这个
//! JSON 字符串里**，必须再 `json_parse` 一层（[`agent_content_json`]）；DataTransformer
//! 无信封。曾把 AgentNode 当成「裸对象直接有业务字段」⇒ 三个 KPI 全部恒 `None`。
//!
//! ## 未验证项（如实声明，勿当作已验证）
//!
//! 1. 上述**信封形态**均已**读源码核实**（CodeNode `code_executor.rs:520-532`；
//!    AgentNode `agent_executor.rs:2618-2634`；LoopNode `loop_executor.rs:535-549`；
//!    双键写入 `engine/mod.rs:1669-1674`），且两个 CodeNode「会落哪个分支」的不确定性
//!    也已关掉（`make_code_node` 把 `execute_directly + rhai` 硬编码，见常量 doc）。
//!    但**未用真实一次执行结果核对**过实际 `results` 快照。
//! 2. `lc-draft-loop`（Loop 容器）聚合产物的确切片形态**已定论**（静态源码级，双源同判）：
//!    信封是 `loop_executor.rs:535-549`，数组在 `items` 键；元素是末位 body 节点
//!    `lc-chapter-bare`（DataTransformer，唯一不套信封的执行器）原样吐出的
//!    `{chapter_text, summary}`（`seed_content_media.rs:954-994`；
//!    `executor-output-shapes.md` §2.4/§4 同判）。**降级档位已据此收敛**
//!    （字数取 `chapter_text` 字符数求和；`as_array` 只认 `items`）。
//!    仍未验证的只是**运行期**：本文件与矩阵都未用真实一次执行的 `results` 快照核对。
//! 3. 提取不到时**不写库**并 `warn` 留痕，绝不用 0 冒充；相关提取逻辑有单元测试
//!    （含「不得采信 LLM 自报值」「AgentNode 业务字段在 content 里」等夹具）。
//!
//! ## 跨文件行号引用的时效（读本文件的人必看）
//!
//! 本文件注释里所有 `xxx.rs:NNN` 形式的引用，均为 **2026-09-15 写就时的时点快照**。
//! `seed_content_media.rs` 等文件仍会被改动 ⇒ 行号必然漂移，**漂移后的行号是错误路标**
//! （本项目已多次踩到：同一处 `hooks_config` 声明先后位于 `:120`/`:166`/`:183`）。
//!
//! ⇒ **引用请优先认「符号名 + 代码片段」**，行号只作快速定位的辅助；
//! 若发现某个行号对不上，**以符号名/片段为准**，并顺手把该行号更新为你的当前读数。

use axagent_analysis_engine::opc::{
    AnalyticsService, CreateKpiInput, DefaultAnalyticsService, canonical_domain_pack_id,
};
// 连接类型经 dao 出口（`crates/dao/src/db.rs` 明文：消费者只需 `use axagent_*::DatabaseConnection`）。
// 本模块**不含任何 SQL** —— 落库全在 `axagent_analysis_engine::opc::DefaultAnalyticsService`
// （唯一用法：`DefaultAnalyticsService::new(self.db.clone())`）；直接引 `sea_orm::` 路径
// 会命中分层门禁 `commands-no-direct-db`（判据：命令层不得直连 sea_orm / axagent_entities）。
use axagent_dao::db::DatabaseConnection;
use axagent_harness::constants::capability_pack::{INPUT_KEY, KPI_HOOK_NAME};
use axagent_harness::workflow_types::Variable;
use axagent_harness::{HookExecContext, HookOutcome, WorkflowLifecycleHook};
use axagent_rt_workflow::work_engine::WorkEngine;
use serde_json::Value;
use std::path::PathBuf;
use std::sync::Arc;

// 钩子名与 `ctx.input` 键名的**权威源在 `axagent_harness::constants::capability_pack`**：
// 命令层不得自定义。原先定义在此处，导致每个消费方都必须「跨命令模块 import」，
// 命令依赖图退化成网（分层门禁 `commands-no-sibling-call` 拦的正是这个）。
// 本模块与 `seed_content_media`、`opc_capability_pack_actions` 共用同一份，改名只有一处可改。

/// 节点 ID：全文抽取（Rhai **CodeNode**，产出 `full_text` / `char_count` / `chapter_count`）。
///
/// ## 到达路径（已读源码核实，勿按 Agent 节点外推）
///
/// `lc-extract-fulltext` 是 **CodeNode**（`make_code_node`，`output_var = "lc-extract-fulltext"`），
/// 不是 ToolNode/Agent 节点，信封形态不同：
/// - `rt-workflow .../executors/code_executor.rs:520-532`（`execute_directly` 分支）返回
///   `{status, language, result: <Rhai map>, input_params, node_id, params: <同一 map>}`；
/// - `engine/mod.rs:1669-1674` 把该值**同时**写进 `results[node_id]` 与 `results[output_var]`。
///
/// ⇒ 运行时取值路径 = `results["lc-extract-fulltext"].result.char_count`
/// （`.params.char_count` 等价）。seed 自身的消费写法也印证同一层：
/// `seed_content_media.rs:1920` `("markdown", "lc-extract-fulltext.result.full_text")`。
/// 本文件的 [`node_output`] 恰好做 `.result` → `.output` → 裸对象的三级解包，落点正确。
///
/// ## 为什么确定落在「有 `result`」的那个分支（关掉 CodeNode 分支不确定性）
///
/// `CodeNode` 有 4 个返回出口，**只有 `execute_directly + language=="rhai"` 那个带 `result` 键**
/// （`output/executor-output-shapes.md` §2.3 已穷举：dry_run 分支的 `result` 是字符串、
/// `tool_registered`/非 rhai 分支**根本没有 `result`**）。本节点由 `make_code_node` 构造
/// （`seed_content_media.rs:1821-1840`），该辅助函数把 **`language: "rhai"`（`:595`）与
/// `execute_directly: true`（`:599`）硬编码** ⇒ 其余三个分支在生产路径上不可达
/// （dry_run 仅在 `context.dry_run` 下短路）。故 `.result` 必在。
const NODE_EXTRACT_FULLTEXT: &str = "lc-extract-fulltext";
/// 节点 ID：大纲脱壳（Rhai **CodeNode**，产出真数组 `chapters`）。
///
/// **这是「引擎实际迭代的那份数据」**：`lc-draft-loop.iter_input_var =
/// "lc-outline-chapters.result.chapters"`（`seed_content_media.rs:1734`）
/// ⇒ 完成率分母必须与它同源。
///
/// 同样由 `make_code_node` 构造（`seed_content_media.rs:1556-1564`）⇒
/// `execute_directly + rhai` 硬编码，`.result` 必在（同上）。
const NODE_OUTLINE_CHAPTERS: &str = "lc-outline-chapters";
/// 节点 ID：逐章创作循环（Loop，聚合元素为裸 `{chapter_text, summary}`）。
///
/// 是本钩子**两个降级档的唯一来源**：字数降级 = 逐元素 `chapter_text` 字符数求和；
/// 完成率分子回退 = `items` 元素个数（`loop_executor.rs:541` 的 `items`）。
/// ⚠️ 元素**没有** `word_count` 键 —— 判词见 `seed_content_media.rs:955`。
const NODE_DRAFT_LOOP: &str = "lc-draft-loop";
/// 节点 ID：容错评审（**AgentNode**，`verdict` 在 `content` 这个 JSON 字符串里）。
///
/// ⚠️ 它的 `output_var` 也叫 `"lc-tolerance"`，与 **ConditionNode** `lc-tolerance`
/// 的 node_id **重名**（`seed_content_media.rs:1867` vs `:1885`），两者都往
/// `results` 里写 ⇒ `results["lc-tolerance"]` 是**写入碰撞区**，只能按 node_id
/// `lc-tolerance-agent` 读。
const NODE_TOLERANCE_AGENT: &str = "lc-tolerance-agent";

// ── 结果提取（纯函数，可单测）─────────────────────────────────────

/// 取某 `node_id` 的输出值，解掉**信封**那一层（`.result` / `.output`）。
///
/// ## 它只负责「信封解包」，不负责「业务字段在字符串里」——这就是本次缺陷的根因
///
/// 本仓四类节点的产出信封**各不相同**，`node_output` 对它们分别落到：
///
/// | 节点类 | 信封顶层键 | `node_output` 落到 | 业务字段还需再剥 |
/// |---|---|---|---|
/// | **CodeNode**（`execute_directly`） | `status/language/result/input_params/node_id/params` | `.result`（已是对象） | 不需要 |
/// | **ToolNode** | `tool_name/result/node_id`（另经 `attach_investigation` 附加调查字段） | `.result` / `.output` | **视路径而定**：ToolRegistry 路径的 `result` 是**字符串**（`tool_executor.rs:142-155`），回调路径是对象（`:183-194`） |
/// | **AgentNode** | `role/model/content/thinking/usage/tool_calls_made/node_id/streamTruncated/truncationReason` | **信封本身**（无 `result`/`output` ⇒ `.or(Some(v))`） | **需要**：业务字段在 `content` 这个 **JSON 字符串**里 |
/// | **DataTransformer** | 无信封，直接是转换结果（种子注释 `seed_content_media.rs:1686-1690`） | 结果本身 | 不需要 |
///
/// 所以对 AgentNode，`node_output(...).get("verdict")` **恒 `None`** ——
/// 层级没解错，是字段不在这一层。读 AgentNode 业务字段请用
/// [`agent_content_json`]（它会再 `json_parse` 一次 `content`）。
///
/// AgentNode 信封的键集合出处：`.../executors/agent_executor.rs:2618-2634`。
pub(crate) fn node_output<'a>(results: &'a Value, node_id: &str) -> Option<&'a Value> {
    let v = results.get(node_id)?;
    match v {
        Value::Object(obj) => obj.get("result").or_else(|| obj.get("output")).or(Some(v)),
        _ => Some(v),
    }
}

/// 取 **AgentNode** 产出里的业务对象 —— 把 `content` 这个 JSON **字符串**解析开。
///
/// 存在理由见 [`node_output`] 的形态表：AgentNode 信封里**没有**业务字段，
/// 它们在 `content`（JSON 文本）里。
///
/// 解析严格度与 seed 内既有消费端**一致**：`LC_CHAPTERS_RHAI` / `LC_FULLTEXT_RHAI`
/// 都是 `json_parse(raw)` 后做 `type_of` 校验，**不做围栏剥离之类的启发式**
/// （`seed_content_media.rs:939-947`、`:895-903`）。故此处也只做 `serde_json::from_str`：
/// 解析失败即 `None`，由调用方按「缺失」处理 —— 绝不猜、不新增第二套宽松口径。
pub(crate) fn agent_content_json(results: &Value, node_id: &str) -> Option<Value> {
    let raw = results.get(node_id)?;
    // 兼容：Agent 输出若被某条路径套了 result/output，先解一层（本仓当前无此形态）
    let env = match raw {
        Value::Object(o) => o.get("result").or_else(|| o.get("output")).unwrap_or(raw),
        _ => raw,
    };
    let content = env.get("content")?.as_str()?;
    serde_json::from_str::<Value>(content.trim()).ok()
}

/// 计划章节数组 —— **引擎实际迭代的那份数据**。
///
/// `lc-draft-loop.iter_input_var = "lc-outline-chapters.result.chapters"`
/// （`seed_content_media.rs:1734`）⇒ Loop 逐项迭代的就是这个数组，
/// 完成率分母必须与它同源，否则「计划数」与「实际迭代数」会脱钩。
///
/// **刻意不用 `lc-outline`**：它是 AgentNode，`content` 只是 LLM 的描述字符串；
/// 且 `loop_executor.rs:163-174` 只在拿到**真 Array** 时才逐项迭代，字符串一律单轮
/// （`seed_content_media.rs:927-931` 有实证）⇒ 它**不是**引擎迭代的那份数据。
pub(crate) fn outline_chapters(results: &Value) -> Option<&Vec<Value>> {
    match node_output(results, NODE_OUTLINE_CHAPTERS)? {
        Value::Array(a) => Some(a),
        Value::Object(o) => o.get("chapters").and_then(Value::as_array),
        _ => None,
    }
}

/// 把 LoopNode 信封 / 裸数组统一取成数组引用（**只认真实存在的两个形状**）。
///
/// - `Value::Array` —— Loop 迭代输入经 `resolve_var_path` 直接给出数组时的形状；
/// - `Value::Object` 的 **`items`** —— LoopNode 生产信封的固定键
///   （`loop_executor.rs:541`：`"items": partial`）。
///
/// **刻意只认 `items`**：曾顺带接受 `chapters` / `drafts`，但两者在 Loop 聚合产物上
/// **没有任何生产端**（`lc-draft-loop.items` 是唯一真实来源，元素形状见
/// `seed_content_media.rs:954-987`）。为不存在的键留档位 = 掩盖形态错误，
/// 故删除 —— 与本文件删掉假设性 `rewrite_count`、改掉不存在的 `word_count` 同一标准。
fn as_array(v: &Value) -> Option<&Vec<Value>> {
    match v {
        Value::Array(a) => Some(a),
        Value::Object(o) => o.get("items").and_then(Value::as_array),
        _ => None,
    }
}

/// 全文总字数 —— **只接受代码算出的值**。
///
/// 取值优先级（**两档都是代码计算，语义都是「字符数」**）：
/// 1. `lc-extract-fulltext.result.char_count` —— Rhai CodeNode 用 `.len()` 算出的
///    **字符数**（Rhai 1.26.0 的 `String::len` 是 `chars().count()`，非字节数；
///    `seed_content_media.rs:862-870` 有源码级实证；上游产出契约见同文件 `:869-874`）；
/// 2. `lc-draft-loop.items[*].chapter_text` 的**字符数求和**（Rust `chars().count()`，
///    与①的计数语义一致 —— 中文一字计 1，不按字节）。
///
/// ## 为什么第二档**不是**「逐章 `word_count` 求和」（键不存在，非风格选择）
///
/// 判词（两处独立同判，均为源码级）：
/// - `seed_content_media.rs:989-1004`：Loop 末位 body 节点是 `lc-chapter-bare`
///   （DataTransformer，**唯一不套信封的执行器**），其 Rhai 表达式原样返回
///   `lc-chapter-pack` 的 `#{ chapter_text: chapter_text, summary: summary }`
///   —— 文档明文写「**包成只含这两个键的对象**」「`items` 的元素顶层才是裸
///   `{chapter_text, summary}`」；
/// - `output/executor-output-shapes.md` §4 对同一格点的判词：**「数组取到；
///   `word_count` 取不到」**（元素无该键 ⇒ 原写法恒 `None`）。
///
/// ⇒ 按 `word_count` 取值是**为不存在的上游留档位**，与本文件对
/// 「显式 `rewrite_count`」的处理标准一致（无生产端即删）。改用真实存在的
/// `chapter_text` 求和，既保住 team-lead 要求的「逐章求和」降级，又落在真形态上。
///
/// ## 删除 `lc-assemble.total_word_count` 档位的两个理由（叠加，缺一不可）
///
/// 1. **它本来就到不了这一层**：`lc-assemble` 是 AgentNode，其 `content` 是 JSON 字符串，
///    而 `total_word_count` 在 `content` 里面 —— `node_output` 只解信封、不解字符串
///    （见 [`node_output`] 的形态表）⇒ 这一档**恒取不到**，不是"取到过但被否掉"。
///    这一条已由上游注明（`seed_content_media.rs:1243-1250` 直接引用本函数的形态分析）。
/// 2. **即便穿透到 `content` 也不该采信**：该字段由 `assemble_prompt` 的输出 schema
///    要求 LLM 自己报，属「把可确定性计算的东西交给 LLM 自评」。
///    上游现已把该字段从两个 prompt 的 schema 中**删除**
///    （`seed_content_media.rs:1251` 小说路径、`:1270` 非小说路径同族）。
///
/// ⇒ 结论：那一档**删除**（而非降级兜底）。取不到任何一档 ⇒ `None`，绝不回退 0。
pub(crate) fn extract_word_count(results: &Value) -> Option<f64> {
    if let Some(n) = node_output(results, NODE_EXTRACT_FULLTEXT)
        .and_then(|o| o.get("char_count"))
        .and_then(Value::as_f64)
    {
        return Some(n);
    }
    // 降级：逐章正文字符数求和（`chapter_text` 是 Loop 聚合元素上真实存在的键）
    let items = node_output(results, NODE_DRAFT_LOOP).and_then(as_array)?;
    let mut sum = 0.0_f64;
    let mut found = false;
    for it in items {
        if let Some(text) = it.get("chapter_text").and_then(Value::as_str) {
            sum += text.chars().count() as f64;
            found = true;
        }
    }
    found.then_some(sum)
}

/// 章节完成率（%）：已产出章节数 ÷ 计划章节数 × 100。
///
/// - **分母** = [`outline_chapters`]（`lc-outline-chapters.result.chapters`），
///   即**引擎实际迭代的那份数据**。**不用 `lc-outline`**：它是 AgentNode，取到的只是
///   LLM 的描述字符串（且引擎拿到字符串只会单轮迭代）。取不到 ⇒ `None`，
///   绝不用 LLM 自报的章节数兜底（沿用 `completion_rate_none_without_outline` 的语义）。
/// - **分子**优先 `lc-extract-fulltext.result.chapter_count`（上游 CodeNode 按
///   `lc-draft-loop.items` 数组长度算出的确定性计数），回退 `lc-draft-loop` 元素个数；
///   两者都取不到 ⇒ `None`（见 `chapter_count_absent_does_not_become_zero`）。
pub(crate) fn extract_completion_rate(results: &Value) -> Option<f64> {
    let planned = outline_chapters(results)?.len();
    if planned == 0 {
        return None;
    }
    let drafted = node_output(results, NODE_EXTRACT_FULLTEXT)
        .and_then(|o| o.get("chapter_count"))
        .and_then(Value::as_f64)
        .filter(|n| *n >= 0.0)
        .map(|n| n as usize)
        .or_else(|| node_output(results, NODE_DRAFT_LOOP).and_then(as_array).map(|a| a.len()))?;
    Some(drafted.min(planned) as f64 / planned as f64 * 100.0)
}

/// 本轮执行的改写轮次 —— 取自**引擎自己用来路由的那个字段**。
///
/// ## 信号源（已核实的节点类型与层数）
///
/// 只认 `lc-tolerance-agent`：**AgentNode**（`make_agent_node_with_inputs`，
/// `seed_content_media.rs:1843-1858`）⇒ `verdict` 在 `content` 这个 **JSON 字符串**里，
/// 必须走 [`agent_content_json`]（再剥一层），`node_output(...).get("verdict")` 恒 `None`。
///
/// ⚠️ **必须按 node_id `lc-tolerance-agent` 读，不能读 `"lc-tolerance"`**：后者是
/// **ConditionNode** 的 node_id（`:1866-1877`），而该 AgentNode 的 `output_var` 恰好也叫
/// `"lc-tolerance"` ⇒ 两个节点都往 `results["lc-tolerance"]` 写，是**碰撞区**。
///
/// ## 判据与语义
///
/// `verdict` 的取值域由 prompt 明文枚举（`seed_content_media.rs:1305-1315`）：
/// `pass`（含「仅 1 项 fail 但容错不触发重写」）/ `rewrite`（2 项及以上 fail）。
/// 引擎的 ConditionNode 正是用正则 `"verdict"\s*:\s*"pass"` 路由（`:1872`）
/// ⇒ 本 KPI 与引擎**共用同一判据**，不引入第二套口径。
///
/// - `pass` ⇒ `Some(0)`：本轮**确认**无需改写，是真值（不是占位）。
/// - `rewrite` ⇒ `Some(1)`：本轮发生过一次改写判定。
///   单次执行内不可能 >1 —— 前端回环会另起一次执行（跨执行累计属产品口径，未定，故不做）。
/// - 读不到 / 取值未知 ⇒ `None`（不写 0 冒充）。
///
/// ## 刻意不解析的两处（附理由）
///
/// - `lc-approval`（**ApprovalNode**）：它先 `NodeControl::Suspend` 挂起等人工，
///   恢复后由命令层把结果覆写为 `{"status":"approved","result":true}`
///   （`approval_executor.rs:18`、`:70-101`），人工的 `decision`（accept|rewrite）
///   不在该形态里；且挂起态不构成一次终态执行 ⇒ 取不到就不写。
/// - 「显式 `rewrite_count`」档位：全仓 grep `rewrite_count` **只命中本文件**
///   （无任何生产端产出）。为不存在的上游留档位属于假设性设计，故删除。
pub(crate) fn extract_revision_rounds(results: &Value) -> Option<f64> {
    let verdict = agent_content_json(results, NODE_TOLERANCE_AGENT)?
        .get("verdict")
        .and_then(Value::as_str)
        .map(str::to_owned)?;
    match verdict.as_str() {
        // prompt 明文的通过态（含「1 项 fail 但容错」）
        "pass" => Some(0.0),
        // prompt 明文的重写态
        "rewrite" => Some(1.0),
        other => {
            tracing::warn!(
                "[opc-kpi-hook] {NODE_TOLERANCE_AGENT}.verdict 取值未知({other})，按缺失处理，不写 0"
            );
            None
        },
    }
}

/// 该次执行是否应当落库（终态成功）。
fn is_success_status(status: &str) -> bool {
    matches!(status, "completed" | "partially_completed")
}

/// 从 `HookExecContext.input` 取域包归属（P2 通道的消费端）。
///
/// 归一与校验都转调 [`canonical_domain_pack_id`]（写侧与读侧共用的唯一真源）：
/// 若这里自己写一套 `replace('-', "_")`，两边迟早会漂移出一个「同一域包两个桶」
/// 的缺陷 —— 那正是本次要消灭的形态。
///
/// 返回 `None` 的三种情形（一律视为「域包不可得」）：
/// - `ctx.input` 为 `None`（调用方没走 P2 注入，如手工在编辑器里跑模板）；
/// - `input` 里没有该键（同上）；
/// - 键存在但取出失败（不是字符串 / 空白 / 空串）—— `''` 在 DB 里表示
///   「未标注」的存量语义，不是一个域包 id。
pub(crate) fn capability_pack_from_context(ctx: &HookExecContext) -> Option<String> {
    ctx.input
        .as_ref()
        .and_then(|v| v.get(INPUT_KEY))
        .and_then(Value::as_str)
        .and_then(|s| canonical_domain_pack_id(s).ok())
}

// ── 钩子实现 ──────────────────────────────────────────────────────

/// 把 content-media 创作类工作流的产出写入 `opc_kpi_records`。
pub(crate) struct ContentMediaKpiPersistHook {
    db: DatabaseConnection,
    /// 用户数据目录：用于解析域包内 runtime.yaml 的 KPI 单位（元数据权威）。
    app_dir: PathBuf,
}

impl ContentMediaKpiPersistHook {
    pub(crate) fn new(db: DatabaseConnection, app_dir: PathBuf) -> Self {
        Self { db, app_dir }
    }

    /// 从 runtime.yaml 取该 KPI 的单位；取不到返回空串（不伪造单位）。
    fn unit_of(&self, domain_pack_id: &str, key: &str) -> String {
        use axagent_analysis_engine::opc::capability_pack;

        let base = axagent_analysis_engine::opc::resolve_capability_packs_dir(Some(&self.app_dir));
        let dir = base.join(domain_pack_id);
        capability_pack::analysis_schema::load_capability_pack(&dir)
            .and_then(|b| b.runtime)
            .and_then(|c| c.definition(key).and_then(|d| d.unit.clone()))
            .unwrap_or_default()
    }
}

#[async_trait::async_trait]
impl WorkflowLifecycleHook for ContentMediaKpiPersistHook {
    fn name(&self) -> &str {
        KPI_HOOK_NAME
    }

    async fn pre_exec(&self, ctx: HookExecContext) -> Result<Vec<Variable>, String> {
        Ok(ctx.variables)
    }

    async fn post_exec(&self, ctx: HookExecContext, outcome: &HookOutcome) -> Result<(), String> {
        if !is_success_status(&outcome.status) {
            tracing::warn!(
                "[opc-kpi-hook] {} 终态 {} 非成功，跳过 KPI 落库",
                ctx.template_id,
                outcome.status
            );
            return Ok(());
        }

        let period = chrono::Utc::now().format("%Y-%m").to_string();
        // 域包归属从运行上下文注入（见文件头「域包归属从哪来」）。取不到 ⇒ 不落库。
        let Some(domain_pack_id) = capability_pack_from_context(&ctx) else {
            tracing::warn!(
                "[opc-kpi-hook] {} 的 ctx.input 未携带可用的 {INPUT_KEY}，本次不落库：\
                 无归属的 KPI 行在域包读侧查不出来（会把「钩子没生效」伪装成「工作流没产出」）",
                ctx.template_id
            );
            return Ok(());
        };
        let svc = DefaultAnalyticsService::new(self.db.clone());

        // (KPI key, 取到的值)；取不到的一律不写（缺失显式，不写假 0）
        let mut rows: Vec<(&str, f64)> = Vec::new();
        let word_count = extract_word_count(&outcome.results);
        if let Some(v) = word_count {
            rows.push(("word_count", v));
        }
        let completion_rate = extract_completion_rate(&outcome.results);
        if let Some(v) = completion_rate {
            rows.push(("completion_rate", v));
        }
        let revision_rounds = extract_revision_rounds(&outcome.results);
        if let Some(v) = revision_rounds {
            rows.push(("revision_rounds", v));
        }

        if rows.is_empty() {
            tracing::warn!(
                "[opc-kpi-hook] {} 终态结果中未提取到任何创作类 KPI（word_count/completion_rate/revision_rounds），本次不落库",
                ctx.template_id
            );
            return Ok(());
        }

        for (key, value) in rows {
            let unit = self.unit_of(&domain_pack_id, key);
            let input = CreateKpiInput {
                domain_pack_id: domain_pack_id.clone(),
                name: key.to_string(),
                value,
                unit,
                period: period.clone(),
            };
            match svc.record_kpi(input).await {
                Ok(rec) => tracing::info!(
                    "[opc-kpi-hook] {} 落库 KPI: capability_pack={} {}={} {} period={} id={}",
                    ctx.template_id,
                    rec.domain_pack_id,
                    key,
                    value,
                    if rec.unit.is_empty() {
                        "-"
                    } else {
                        rec.unit.as_str()
                    },
                    period,
                    rec.id
                ),
                Err(e) => tracing::warn!(
                    "[opc-kpi-hook] {} KPI {} 落库失败（post_exec 不阻断）: {e}",
                    ctx.template_id,
                    key
                ),
            }
        }
        Ok(())
    }
}

// ── 注册 ─────────────────────────────────────────────────────────

/// 启动期把 OPC 工作流 KPI 钩子注册进 WorkEngine。
///
/// 调用方：`init/state.rs`（与 stock 钩子同处）。注册后**仍需模板在
/// `hooks_config.post_exec` 中声明**才会被调用。
pub(crate) async fn register_opc_kpi_hooks(
    engine: &WorkEngine,
    db: DatabaseConnection,
    app_dir: PathBuf,
) {
    engine.register_lifecycle_hook(Arc::new(ContentMediaKpiPersistHook::new(db, app_dir))).await;
    tracing::info!("[opc-kpi-hook] 生命周期钩子已注册: {KPI_HOOK_NAME}");
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// **真实 AgentNode 信封**：键集合逐字取自 `agent_executor.rs:2618-2634`
    /// （`role` / `model` / `content` / `thinking` / `usage` / `tool_calls_made` /
    /// `node_id` / `streamTruncated` / `truncationReason`）。
    ///
    /// **不许简化成 `{content: "..."}`** —— 兄弟键在场正是本次缺陷的关键：
    /// 因为有兄弟键、且顶层没有 `result`/`output`，`node_output` 才会一路
    /// `or(Some(v))` 落到信封本身、拿不到 `content` 里的业务字段。
    fn agent_envelope(node_id: &str, content: &str) -> Value {
        json!({
            "role": "文案创作专家",
            "model": "gpt-4.1",
            "content": content,
            "thinking": "",
            "usage": {"input_tokens": 1200, "output_tokens": 3400},
            "tool_calls_made": 0,
            "node_id": node_id,
            "streamTruncated": false,
            "truncationReason": "",
        })
    }

    /// **真实 CodeNode 信封**：`code_executor.rs:520-532` 的六键形态。
    fn code_envelope(node_id: &str, result: Value) -> Value {
        json!({
            "status": "executed",
            "language": "rhai",
            "result": result,
            "input_params": {},
            "node_id": node_id,
            "params": result,
        })
    }

    /// **真实 LoopNode 信封**：键集合逐字取自 `loop_executor.rs:535-549`
    /// （`loop_type` / `iter_count` / `last_iter_index` / `resumed_from_checkpoint` /
    /// `interrupted` / `items` / `iter_output_var` / `iter_input_var` / `node_id`）。
    ///
    /// **不许简化成裸数组**：真实 `results["lc-draft-loop"]` 是这层信封，
    /// `node_output` 会落到信封本身、`as_array` 必须从 `items` 键取数组。
    /// 用裸数组做夹具会**绕过这条真实分支**（虽是 `as_array` 的合法入参），
    /// 于是「元素形状判错」这类缺陷在测试里恰好被掩盖 —— 补此夹具即为堵这个洞。
    fn loop_envelope(node_id: &str, items: Value) -> Value {
        let n = items.as_array().map_or(0, |a| a.len());
        json!({
            "loop_type": "for_each",
            "iter_count": n,
            "last_iter_index": n.saturating_sub(1),
            "resumed_from_checkpoint": false,
            "interrupted": false,
            "items": items,
            "iter_output_var": "lc-chapter-bare",
            "iter_input_var": "lc-outline-chapters.result.chapters",
            "node_id": node_id,
        })
    }

    /// Loop 聚合元素的**真实形态**：末位 body 节点 `lc-chapter-bare`
    /// （DataTransformer，**唯一不套信封的执行器**）原样返回
    /// `lc-chapter-pack` 的 `#{ chapter_text, summary }` —— **只含这两个键**。
    /// 判词出处：`seed_content_media.rs:955`（明文「包成只含这两个键的对象」）。
    fn chapter_item(text: &str, summary: &str) -> Value {
        json!({"chapter_text": text, "summary": summary})
    }

    /// 根因钉死：AgentNode 的业务字段**不在信封顶层**，在 `content` 字符串里。
    ///
    /// 这条测试就是本次「三个 KPI 恒 None」的缺陷表述本身。
    #[test]
    fn agent_node_business_fields_live_inside_content_string() {
        let results = json!({
            "lc-tolerance-agent": agent_envelope("lc-tolerance-agent", r#"{"verdict":"pass","confidence":0.9}"#),
        });
        // ① 信封解包这一层拿不到 verdict（层级没解错，是字段不在这层）
        assert_eq!(node_output(&results, "lc-tolerance-agent").unwrap().get("verdict"), None);
        // ② 兄弟键在场（证明确实是 AgentNode 信封，不是被简化过的夹具）
        let env = node_output(&results, "lc-tolerance-agent").unwrap();
        assert!(env.get("role").is_some());
        assert!(env.get("usage").is_some());
        assert!(env.get("streamTruncated").is_some());
        // ③ 必须再 json_parse 一次 content 才拿得到
        assert_eq!(
            agent_content_json(&results, "lc-tolerance-agent")
                .and_then(|v| v.get("verdict").cloned()),
            Some(json!("pass"))
        );
    }

    /// 首选来源必须是 CodeNode 代码算出的字符数。
    #[test]
    fn word_count_prefers_code_counted_char_count() {
        let results = json!({
            "lc-extract-fulltext": code_envelope(
                "lc-extract-fulltext",
                json!({"full_text": "正文…", "char_count": 43210, "chapter_count": 8}),
            ),
            // 降级档（逐章正文字符数求和 = 5）会给出完全不同的值 ⇒ 若取错档位立刻暴露
            "lc-draft-loop": loop_envelope("lc-draft-loop", json!([
                chapter_item("第一章正文", "甲"),
                chapter_item("第二章", "乙"),
            ])),
        });
        assert_eq!(extract_word_count(&results), Some(43210.0));
    }

    /// 反向夹具：**LLM 自报的总字数绝不能作为任何降级来源**。
    ///
    /// 夹具用**真实 AgentNode 信封**且 `content` 里确实带 `total_word_count`（35000）——
    /// 这是比"裸对象里放该字段"更强的夹具：证明**即使穿透到 content 也不采信**。
    /// - 不得返回 35000；
    /// - 有逐章正文时走逐章字符数求和（`第一章正文`=5 + `第二章`=3 ⇒ 8）；
    /// - 连逐章正文都没有时返回 `None`（不是 35000，也不是 0）。
    #[test]
    fn word_count_ignores_llm_reported_total() {
        let assemble_env = json!({"lc-assemble": agent_envelope("lc-assemble", r#"{"full_text":"…","total_word_count":35000}"#)});

        let mut with_chapters = assemble_env.clone();
        with_chapters["lc-draft-loop"] = loop_envelope(
            "lc-draft-loop",
            json!([chapter_item("第一章正文", "甲"), chapter_item("第二章", "乙"),]),
        );
        assert_eq!(
            extract_word_count(&with_chapters),
            Some(8.0),
            "LLM 自报的 35000 不得被采信，应回退逐章正文字符数求和"
        );

        assert_eq!(
            extract_word_count(&assemble_env),
            None,
            "只有 LLM 自报值时必须返回 None，不得拿它兜底"
        );
    }

    /// `lc-extract-fulltext` 的 `.result` 解包（与 seed 自身消费路径同层：
    /// `seed_content_media.rs:1920` 的 `lc-extract-fulltext.result.full_text`）。
    #[test]
    fn word_count_unwraps_result_envelope() {
        let results = json!({
            "lc-extract-fulltext": code_envelope("lc-extract-fulltext", json!({"char_count": 7})),
        });
        assert_eq!(extract_word_count(&results), Some(7.0));
    }

    /// 降级档 = 逐章 `chapter_text` 的**字符数**求和（不是字节数：中文一字计 1）。
    ///
    /// 夹具用**真实 Loop 聚合元素形态** `{chapter_text, summary}` + 真实 LoopNode 信封
    /// —— 见 `seed_content_media.rs:955` 的判词「包成只含这两个键的对象」。
    #[test]
    fn word_count_falls_back_to_chapter_text_char_sum() {
        let results = json!({
            "lc-draft-loop": loop_envelope("lc-draft-loop", json!([
                chapter_item("一二三四五", "甲乙"),
                chapter_item("六七八", "丙"),
            ])),
        });
        // 5 + 3 = 8（若误按字节数会是 24）
        assert_eq!(extract_word_count(&results), Some(8.0));
    }

    /// 元素带 `word_count` 键也不采信 —— 该键在真实形态里**不存在**，
    /// 出现即意味着上游被误改；无论如何都不该有第二条取值路径。
    #[test]
    fn word_count_ignores_nonexistent_word_count_key() {
        let results = json!({
            "lc-draft-loop": loop_envelope(
                "lc-draft-loop",
                json!([{"word_count": 10}, {"word_count": 20}]),
            ),
        });
        assert_eq!(
            extract_word_count(&results),
            None,
            "`items[*].word_count` 不是真实形态（`seed_content_media.rs:954-987`），\
             不得为它保留档位；元素无 `chapter_text` ⇒ 应返回 None"
        );
    }

    #[test]
    fn word_count_none_when_absent() {
        // 不得回退成 0 —— None 表示「取不到」，由调用方显式跳过落库
        assert_eq!(extract_word_count(&json!({})), None);
        assert_eq!(
            extract_word_count(
                &json!({"lc-draft-loop": loop_envelope("lc-draft-loop", json!([]))})
            ),
            None
        );
        // 有元素但无 chapter_text ⇒ 仍 None
        assert_eq!(
            extract_word_count(&json!({
                "lc-draft-loop": loop_envelope("lc-draft-loop", json!([{"summary": "只有摘要"}])),
            })),
            None
        );
    }

    /// 分母必须来自**引擎实际迭代的那份数据**：`lc-outline-chapters`（CodeNode 脱壳）。
    ///
    /// 同一份 results 里同时放 `lc-outline`（AgentNode，`content` 字符串里是 LLM 描述的
    /// 6 章）与 `lc-outline-chapters`（CodeNode，真数组 4 章）：必须以 **4** 为分母
    /// —— 因为 `iter_input_var = "lc-outline-chapters.result.chapters"` 才是引擎迭代的。
    #[test]
    fn completion_rate_denominator_is_the_iterated_code_node_array() {
        let results = json!({
            "lc-outline": agent_envelope(
                "lc-outline",
                r#"[{"num":1},{"num":2},{"num":3},{"num":4},{"num":5},{"num":6}]"#,
            ),
            "lc-outline-chapters": code_envelope(
                "lc-outline-chapters",
                json!({"chapters": [{"num":1},{"num":2},{"num":3},{"num":4}]}),
            ),
            "lc-draft-loop": loop_envelope("lc-draft-loop", json!([
                chapter_item("a", "甲"),
                chapter_item("b", "乙"),
                chapter_item("c", "丙"),
            ])),
        });
        // 3/4（分母取 CodeNode 真数组）而不是 3/6（分母取 LLM 描述字符串）
        assert_eq!(extract_completion_rate(&results), Some(75.0));
    }

    /// `chapter_count` 缺失 ≠ 0 章：必须回退到逐章个数，两者都无则 `None`。
    #[test]
    fn chapter_count_absent_does_not_become_zero() {
        let fallback = json!({
            "lc-outline-chapters": code_envelope(
                "lc-outline-chapters",
                json!({"chapters": [{"num":1},{"num":2},{"num":3},{"num":4}]}),
            ),
            "lc-draft-loop": loop_envelope("lc-draft-loop", json!([
                chapter_item("a", "甲"),
                chapter_item("b", "乙"),
                chapter_item("c", "丙"),
            ])),
        });
        assert_eq!(
            extract_completion_rate(&fallback),
            Some(75.0),
            "chapter_count 缺失时应回退逐章个数，而不是把已产出章节数当 0"
        );

        let no_numerator = json!({
            "lc-outline-chapters": code_envelope(
                "lc-outline-chapters",
                json!({"chapters": [{"num":1},{"num":2}]}),
            ),
        });
        assert_eq!(
            extract_completion_rate(&no_numerator),
            None,
            "分子完全取不到时返回 None，不得返回 0%"
        );
    }

    #[test]
    fn completion_rate_prefers_code_counted_chapter_count() {
        let results = json!({
            "lc-outline-chapters": code_envelope(
                "lc-outline-chapters",
                json!({"chapters": [{"num":1},{"num":2},{"num":3},{"num":4}]}),
            ),
            // 逐章数组有 2 个元素，但代码数出来的章节数是 3 ⇒ 必须信代码
            "lc-extract-fulltext": code_envelope(
                "lc-extract-fulltext",
                json!({"char_count": 1000, "chapter_count": 3}),
            ),
            "lc-draft-loop": loop_envelope("lc-draft-loop", json!([
                chapter_item("a", "甲"),
                chapter_item("b", "乙"),
            ])),
        });
        assert_eq!(extract_completion_rate(&results), Some(75.0));
    }

    #[test]
    fn completion_rate_none_without_outline() {
        // 分母（引擎迭代数组）缺失 ⇒ 不产出；不得退化成用 LLM 的 lc-outline 描述
        assert_eq!(
            extract_completion_rate(&json!({
                "lc-outline": agent_envelope("lc-outline", r#"[{"num":1},{"num":2}]"#),
                "lc-draft-loop": loop_envelope("lc-draft-loop", json!([1, 2])),
            })),
            None
        );
        // 计划章节为 0 时不做除法
        assert_eq!(
            extract_completion_rate(&json!({
                "lc-outline-chapters": code_envelope("lc-outline-chapters", json!({"chapters": []})),
                "lc-draft-loop": loop_envelope("lc-draft-loop", json!([])),
            })),
            None
        );
    }

    /// 通过态 ⇒ 真值 0（是「本轮确认无需改写」这个**读到的**事实，不是占位）。
    #[test]
    fn revision_rounds_pass_verdict_is_true_zero() {
        let results = json!({
            "lc-tolerance-agent": agent_envelope(
                "lc-tolerance-agent",
                r#"{"verdict":"pass","rewrite_sections":[],"confidence":0.85}"#,
            ),
        });
        assert_eq!(extract_revision_rounds(&results), Some(0.0));
    }

    /// 重写态 ⇒ 1（prompt 明文枚举的另一取值）。
    #[test]
    fn revision_rounds_rewrite_verdict_is_one() {
        let results = json!({
            "lc-tolerance-agent": agent_envelope(
                "lc-tolerance-agent",
                r#"{"verdict":"rewrite","rewrite_sections":[3,7],"confidence":0.6}"#,
            ),
        });
        assert_eq!(extract_revision_rounds(&results), Some(1.0));
    }

    /// 读不到 ⇒ `None`（三种不可读情形），绝不写 0 冒充。
    #[test]
    fn revision_rounds_none_when_verdict_unreadable() {
        // ① 节点缺失
        assert_eq!(extract_revision_rounds(&json!({})), None);
        // ② content 不是 JSON（Agent 返回了自然语言）
        let not_json = json!({
            "lc-tolerance-agent": agent_envelope("lc-tolerance-agent", "本次评审通过，无需重写。"),
        });
        assert_eq!(extract_revision_rounds(&not_json), None);
        // ③ verdict 取值未知（不在 prompt 枚举的 pass/rewrite 里）
        let unknown = json!({
            "lc-tolerance-agent": agent_envelope("lc-tolerance-agent", r#"{"verdict":"maybe"}"#),
        });
        assert_eq!(extract_revision_rounds(&unknown), None);
    }

    /// 必须按 **node_id `lc-tolerance-agent`** 读，不能读碰撞键 `"lc-tolerance"`。
    ///
    /// `lc-tolerance-agent`（AgentNode）的 `output_var` 也叫 `"lc-tolerance"`，
    /// 与 ConditionNode `lc-tolerance` 的 node_id 同名 ⇒ 二者都往同一 key 写。
    #[test]
    fn revision_rounds_reads_by_node_id_not_colliding_key() {
        // 碰撞键里放的是「rewrite」，真身 content 里是「pass」：必须以真身为准
        let results = json!({
            "lc-tolerance": {"verdict": "rewrite"}, // 碰撞区（ConditionNode 侧）
            "lc-tolerance-agent": agent_envelope("lc-tolerance-agent", r#"{"verdict":"pass"}"#),
        });
        assert_eq!(extract_revision_rounds(&results), Some(0.0));

        // 只有碰撞键时：判定不可得 ⇒ None（不猜）
        let only_collision = json!({ "lc-tolerance": {"verdict": "rewrite"} });
        assert_eq!(extract_revision_rounds(&only_collision), None);
    }

    #[test]
    fn success_status_only_completed() {
        assert!(is_success_status("completed"));
        assert!(is_success_status("partially_completed"));
        assert!(!is_success_status("failed"));
        assert!(!is_success_status("cancelled"));
    }

    /// P2 通道的消费端：`ctx.input["domain_pack_id"]` 归一后取出。
    ///
    /// 连字符形态必须归成下划线 —— 否则写侧落 `content-media`、读侧查
    /// `content_media`，刚写的值读不回来（「数据明明落了库却显示暂无数据」）。
    #[test]
    fn capability_pack_from_context_normalizes_hyphen_form() {
        let ctx = |input: Option<Value>| HookExecContext {
            template_id: "workflow_abc".into(),
            execution_id: "exec-1".into(),
            input,
            variables: Vec::new(),
        };
        assert_eq!(
            capability_pack_from_context(&ctx(Some(json!({"domain_pack_id": "content-media"})))),
            Some("content_media".to_string())
        );
        assert_eq!(
            capability_pack_from_context(&ctx(Some(json!({"domain_pack_id": "content_media"})))),
            Some("content_media".to_string())
        );
    }

    /// 域包不可得的四种情形一律 `None`（调用方据此 `warn` + 不落库）。
    ///
    /// **不得**退化成硬编码 `"content_media"` —— 那正是本文件改前的写法，
    /// 它让别的域包声明本钩子时把值写进 content_media 的桶。
    #[test]
    fn capability_pack_from_context_none_when_unavailable() {
        let ctx = |input: Option<Value>| HookExecContext {
            template_id: "workflow_abc".into(),
            execution_id: "exec-1".into(),
            input,
            variables: Vec::new(),
        };
        // ① 调用方没注入 input（如手工在编辑器里跑模板）
        assert_eq!(capability_pack_from_context(&ctx(None)), None);
        // ② input 里没有该键
        assert_eq!(capability_pack_from_context(&ctx(Some(json!({"other": 1})))), None);
        // ③ 空串 / 空白：`''` 是存量行的「未标注」，不是域包
        assert_eq!(capability_pack_from_context(&ctx(Some(json!({"domain_pack_id": ""})))), None);
        assert_eq!(
            capability_pack_from_context(&ctx(Some(json!({"domain_pack_id": "   "})))),
            None
        );
        // ④ 不是字符串（LLM/调用方塞了数字或对象）
        assert_eq!(capability_pack_from_context(&ctx(Some(json!({"domain_pack_id": 42})))), None);
        assert_eq!(
            capability_pack_from_context(&ctx(Some(json!({"domain_pack_id": {"a": 1}})))),
            None
        );
    }
}
