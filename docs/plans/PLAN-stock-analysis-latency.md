# PLAN — 股票分析流耗时优化（质量优先）

> 状态：active ｜ 日期：2026-10-25 ｜ 对应版本：v2.9.9


- 日期：2026-10-25
- 基线：模板 `v179`，现网真实运行 `b6008eea-6a03-4c8c-9c0b-3f42f8f4e100`（status=`partially_completed`，唯一失败是 `t-sentiment-data` 股吧限流，与耗时无关）
- 基线总耗时：**679 s（11.3 min）**
- 目标：稳妥 7 min；无损项全部落地后争取 5–6 min；不追 3 min

> 本文只描述方案，不改代码。每个阶段独立提交、独立开关、独立回滚；
> 有损项默认关闭，必须用同一批股票做 A/B 后才允许改默认值。

---

## 0. 结论先行

1. **真正的瓶颈不是并发不足，而是单节点上下文体积与串行尾链。**
   基线里 `max_concurrent=10`，21 个分析师已并行；风险三视角也已经是并行。
   最慢的链路是：分析师(mid) 98 s → 辩论(mid) 129 s → 收敛 13 s → value-investor 78 s
   → 风险三视角 62 s → 风险收敛 44 s → research-mgr 52 s → trader 144 s → rule-check 47 s。
2. **单节点输入 token 异常大**：`a-market-analyst--short` 82,103 input tokens；
   `trader` 197,923 input tokens；`risk-agg` 79,190 input tokens。
   根因是同一批 `context_sources` 在 system prompt 和 user prompt 里**各注入一次**
   （见 §2.2）。这是当前最大的无损优化点。
3. **三项与需求假设不符**（详见 §3）：
   - 风险三视角**已经并行**（`p-risk-assess` 三条 branch，无互相依赖边）；
   - 公式决策 `portfolio-mgr` 实测 **99 ms**，与大模型决策并行没有收益，且会切断 f7 腿；
   - 分析师原生工具调用**只发一轮**（`tools` 仅 round 0 下发），多轮来自文本内联工具协议，
     不是原生 function calling 多轮。

---

## 1. 基线读数

### 1.1 关键节点耗时（run `b6008eea`）

| 阶段 | 节点 | 耗时 | input tokens | output tokens |
|---|---|---:|---:|---:|
| 分析师 | `a-market-analyst--short` | 154.6 s | 82,103 | 4,932 |
| 分析师 | `a-market-analyst--ultra_short` | 120.2 s | 82,040 | 5,884 |
| 分析师 | `a-market-analyst--long` | 111.2 s | 85,437 | 5,066 |
| 分析师 | `a-market-analyst--mid` | 97.9 s | 82,079 | 900 |
| 辩论 | `debate-bull-bear--ultra_short` | 178.5 s | — | — |
| 辩论 | `debate-bull-bear--mid` | 128.7 s | — | — |
| 收敛 | `debate-convergence--mid` | 13.0 s | — | — |
| 价值 | `value-investor--mid` | 78.5 s | 29,917 | 1,345 |
| 风险 | `risk-agg` / `risk-con` / `risk-neu` | 62.3 / 50.4 / 38.9 s | 79,190（agg） | 1,398 |
| 风险收敛 | `risk-convergence` | 44.2 s | 12,011 | 2,234 |
| 研究 | `research-mgr` | 52.0 s | 50,208 | 2,946 |
| LLM 决策 | `trader` | 144.2 s | 197,923 | 2,313 |
| 公式决策 | `portfolio-mgr` | **0.099 s** | — | — |
| 风控门 | `portfolio-risk-gate` | **0.017 s** | — | — |
| 规则检查 | `rule-check` | 47.0 s | 55,175 | 2,268 |

三视角开始时间完全相同（14:19:57），结束 14:20:36–20:59 ⇒ **已并行**。
公式决策与四个逐档子工作流合计 < 0.5 s，不是决策阶段的耗时来源。

### 1.2 现网配置（DB）

| 变量 | 现网值 |
|---|---|
| `debate_rounds` | 2（需求口述为 3；改值会触发建图漂移门重播种） |
| `max_concurrent` | 10 |
| `agent_max_tokens` | 4096 |
| `agent_timeout_secs` | 300 |

---

## 2. 定位结果

### 2.1 DAG 定义与节点注册

- 模板版本：`src-tauri/src/commands/stock_analysis_setup/seed_stock_analysis.rs:1610`（`TEMPLATE_VERSION = 179`）。
- 建图入口：`seed_stock_analysis.rs:1765`（`seed_stock_analysis_workflow_template`）。
- 节点装配：`seed_stock_analysis.rs:3458` 起 `nodes: Vec<WorkflowNode>` / `edges: Vec<WorkflowEdge>`，
  闭包 `tool_node` / `tool_node_noarg` / `agent` / `edge` 分别负责造节点与边。
- 分析师：`seed_stock_analysis.rs:3847-3958`，21 个 `a-*` 逐档实例，挂在装饰容器 `p-analysts`
  （`seed_stock_analysis.rs:3996`，21 条 branch）。
- 风险三视角：`seed_stock_analysis.rs:4805-4853` 的 `p-risk-assess`，三条 branch
  `risk-agg/risk-con/risk-neu`；三者之间**零依赖边**，共同出边到 `agg-risk`
  （`seed_stock_analysis.rs:5035`）。
- 决策：`trader`（Agent，`seed_stock_analysis.rs:5878`）→ `portfolio-mgr`（Code/Rhai，
  `seed_stock_analysis.rs:6037`），边 `seed_stock_analysis.rs:6446`。
- 执行引擎：`src-tauri/crates/rt-workflow/src/work_engine/engine/mod.rs:1437`
  （`compute_ready_nodes`）与 `:3144-3191`（按类型并发上限、批量 `JoinSet::spawn`）。

### 2.2 分析师节点与工具调用

- 分析师 `max_tool_rounds`：普通 2、`a-catalyst` 3（`seed_stock_analysis.rs:3928-3932`）。
- 工具集来自 `PROFILE_TOOLS`（`src-tauri/src/commands/stock_analysis_setup/mod.rs:466`），
  每个分析师 4–9 个工具。
- 执行循环：`src-tauri/crates/rt-workflow/src/work_engine/executors/agent_executor.rs:1141`
  `for round in 0..max_rounds`；默认上限 `:1101`（`unwrap_or(5)`）。
- **只有 round 0 下发原生 tools**：`agent_executor.rs:1179`
  `tools: if round == 0 { tools.clone() } else { None }`。
  后续轮次的工具调用来自文本内联协议（`agent_executor.rs:1512-1526`、
  `parse_inline_tool_calls` `:3726`）。
- 到上限的兜底指令已存在：`agent_executor.rs:1629-1645`、`:1661-1685`；
  VERDICT 缺失兜底重试 1 次：`:1688-1746`。
- 基线实证：分析师确实调用了 2–6 个工具，参数绝大多数是 `{"stock_code": "003006"}`
  或 `{}`，工具结果单条截断 3000 字符（`agent_executor.rs:1593-1604`）。

### 2.3 辩论轮次、上下文与压缩

- 轮数来源：`seed_variables.rs:37` `DEFAULT_DEBATE_ROUNDS = 1`，现网 DB 为 2；
  建图求值点 `seed_stock_analysis.rs:4130-4135`，漂移门 `debate_rounds_graph_drift`
  会在变量与图不一致时重播种。
- 每轮是**独立节点 + 边串联**，不是 `DebateExecutor` 的 `max_rounds` 循环：
  `seed_stock_analysis.rs:4213-4222` 容器 `max_rounds: 1`，轮次由 `bull-rN/bear-rN` 节点与边展开。
- 上下文注入（超长根因）：
  - 多方：`seed_stock_analysis.rs:4321-4328`，`context_sources` 累加前序所有
    `bull-r1..r(N-1)`、`bear-r1..r(N-1)` + 本档 `analyst-brief`；
  - 空方：`seed_stock_analysis.rs:4363-4371`，再加上本轮多方全文。
  - `agent_executor.rs:648-675` 把每个 source 的 `content` 全文放进 system prompt；
  - `agent_executor.rs:989-998` 又把**同一个 source 的完整 raw JSON**（含 `usage`/`thinking`）
    放进 user prompt ⇒ **同一内容注入两遍**。
- 已有的压缩只有上游 `analyst-brief.rhai`（把 9 份分析师报告压成 VERDICT + 关键论据），
  辩手输出本身**没有任何跨轮摘要**。
- `debate-convergence--{档}`（`seed_stock_analysis.rs:4410-4467`）产出
  `consensus_score / remaining_disputes / decisive_*_acks`，但它是辩论**下游**，
  不回流到 R2/R3。

### 2.4 风险分析三视角

- 三个独立 Agent：`risk-agg` / `risk-con` / `risk-neu`（`seed_stock_analysis.rs:4875-4925`），
  均 `tools=[]`、`max_tool_rounds=0`。
- 并行容器 `p-risk-assess`（`seed_stock_analysis.rs:4805-4853`），`wait_for_all: true`。
- 入边：`p-risk-assess → risk-*`（`seed_stock_analysis.rs:4997`）；出边：`risk-* → agg-risk`
  （`:5035`）。三者之间无互相依赖 ⇒ **已经是并行**。
- 每个视角的 `context_sources`：9 份代表档分析师全文 + 末轮 bull/bear + 收敛 + `t-scoring`
  + `t-valuation`（`seed_stock_analysis.rs:4928-4960`）⇒ 79k input tokens 的来源。
- 最终风险决策：`risk-convergence`（`:5041-5059`）→ `research-mgr`（`:5769`）→ `portfolio-risk-gate`
  （`:6453`）。三视角串行依赖只存在于“三路 → 聚合 → 收敛”这一层，无法再并行化。

### 2.5 决策节点（公式 vs 大模型）

- LLM 侧：`trader`（Agent，`OutputMode::Json`，`seed_stock_analysis.rs:5878-5970`），
  `context_sources` 8 项，`input_mapping` 覆盖参考价/技术指标/估值/催化剂/资金/共识/风险/数据质量。
- 公式侧：`portfolio-mgr`（CodeNode + Rhai，`seed_stock_analysis.rs:6037-6055`），
  `timeout=30 s`，实测 **99 ms**。
- 图上串行：`trader → portfolio-mgr`（`:6446`），因为公式有 **f7 腿**直接消费
  `trader.content.verdict.*`（`:6165-6181`）。
- 事后对比：`compute_decision_agreement`（`stock_workflow/decision.rs:2313`）、
  `compute_horizon_agreements`（`:1944`）、`attribute_horizon_divergence`（`:2125`）、
  `conflict_type`（`:2503-2531`）、`f7_weight_pct`（`:2448-2467`）。
- 结论：**“两种决策并行”不成立**——公式侧是亚秒级确定性计算，且与 LLM 侧存在语义依赖；
  真正的 1–2 min 全部来自 `trader` 的 144 s。

### 2.6 结构化输出配置

- `ChatRequest.response_format` 已存在：`crates/harness/src/types/settings_chat.rs:410-437`；
  `ResponseFormat::{JsonObject, JsonSchema}` 定义在 `:396-406`。
- OpenAI 兼容映射：`crates/providers/src/openai.rs:581-591`；
  DeepSeek/Kimi/MiniMax 对 `JsonSchema` 降级为 `json_object` + prompt 注入：
  `openai.rs:615-639`、`crates/providers/src/structured_output.rs:38-50`。
- **Agent 执行器六处 `response_format: None`**：
  `agent_executor.rs:1188 / 1869 / 2270 / 2815 / 3031 / 3554`。
  当前“结构化”完全靠 prompt 里的 VERDICT 契约 + `validate_strict_mode_output`（`:5289`）。
- 校验失败重试：主流程失败后用 `fallback_model` 重试 1 次（`:2214-2319`），
  仍失败则 `strict_mode_fallback` 降级（`:2349-2400`），**没有无限续接**。
- 分析师是 `OutputMode::Text`，一次响应同时产出正文 + `<!-- VERDICT: {...} -->`，
  不存在“先文本后 JSON 的多轮”。结构化输出加速对分析师**无收益**；
  对 `trader` / `debate-convergence` / `risk-convergence` 等 `OutputMode::Json` 节点有收益。

### 2.7 编排引擎与耗时埋点

- 就绪扫描与批量派发：`engine/mod.rs:1437`、`:3144-3191`；并发上限按类型：
  `core.rs:990-1002`（tool/file=10，llm/agent=用户的 `max_concurrent`）。
- 节点记录：`NodeExecutionRecord`（`crates/harness/src/workflow_types.rs:1963-1976`）含
  `execution_time_ms / started_at / completed_at`；落库在
  `engine/mod.rs:6722-6738`（成功）/ `:6786-6800`（失败），整体一次性写入
  `workflow_executions.node_executions`（`:5881-5900`）。
- 没有阶段级埋点；需要基准脚本从 `node_executions` 聚合。
- 每次运行会从 DB 载入模板并按 `create_workflow_with_hooks` 建运行态图
  （`stock_workflow/core.rs:833-840`）。103 个节点的就绪扫描是纯内存操作，
  基线中节点间空隙为亚秒级 ⇒ **编译缓存收益 < 1 s，不建议做**（与需求中“几十秒到一两分钟”不符）。

---

## 3. 与需求假设的偏差（必须先对齐）

| 需求假设 | 代码/现网事实 | 处置 |
|---|---|---|
| 风险三视角可能串行，改成并行可 4→2 min | 已并行；基线 62 s（最慢视角）+ 44 s 收敛 | **不做并行化**；改为输入瘦身（P2-A） |
| 公式决策与大模型决策并行 | 公式 99 ms，且 f7 吃 LLM 决策；但公式不依赖 trader 也能算 | **不做并行化**；f7 换确定性风险收益比（B），独立公式与 trader 并行、只读对比 |
| 分析师是多轮“模型决定调工具” | 原生 tools 只在 round 0 下发；多轮来自内联协议 | 做**固定序列预取**（P0-B），保留工具集与数据 |
| 分析师先文本后 JSON 多轮 | 分析师一次响应即正文 + VERDICT 标签 | 不改分析师输出模式；只对 Json 节点开 `response_format` |
| 3 轮辩论可压到 2 min | mid 档 4 次调用 129 s ⇒ 单次约 32 s；6 次约 190 s | 去重 + 状态摘要后可到 ~2.2 min；**不承诺 2 min 整** |
| 5–6 min | 无损项后约 8 min；要 5–6 min 需改模型路由或串行语义 | 7 min 为稳妥目标；5–6 min 另立 A/B |

---

## 4. 分阶段实施计划

### P0-A 上下文去重（无损，先做）

**问题**：同一份 `context_sources` 在 system prompt（格式化正文）与 user prompt（raw JSON）
各注入一次。基线中分析师 82k input tokens、trader 198k input tokens，约一半是重复。

**改动文件**

- `src-tauri/crates/rt-workflow/src/work_engine/executors/agent_executor.rs`
  - 函数：`AgentExecutor::execute`（内联 ReAct 路径）
  - 插入点：`agent_executor.rs:989-1012` 的 user_prompt 组装段
- `src-tauri/src/commands/stock_analysis_setup/seed_variables.rs`
  - 新增模板变量 `context_prompt_mode`，默认 `system_only`
- `src-tauri/src/commands/stock_analysis_setup/seed_stock_analysis.rs`
  - `TEMPLATE_VERSION` 179 → 180（变量种子变更需重播种）

**Diff（核心）**

```rust
// agent_executor.rs:989 附近
// 2026-10-25：上下文注入去重。默认 legacy 保持历史行为；模板变量
// `context_prompt_mode=system_only` 时，system 段（4d）已注入同一批
// context_sources 的格式化正文，user 段不再重复整份 raw JSON（含 usage/thinking）。
let context_prompt_mode = context
    .variables
    .get("context_prompt_mode")
    .and_then(|v| v.as_str())
    .unwrap_or("legacy");

if !an.config.context_sources.is_empty() {
    if context_prompt_mode != "system_only" {
        for source in &an.config.context_sources {
            if let Some(value) = context.variables.get(source) {
                if source == super::USER_INPUT_VAR || source == super::USER_MESSAGE_VAR {
                    continue;
                }
                user_prompt_parts.push(format!("{source}: {value}"));
            }
        }
    }
} else {
    // 无 context_sources 时 system 段没有注入这些数据变量 ⇒ 必须保留原行为
    for (k, v) in super::var_filter::collect_data_vars(&context.variables) {
        if k == super::USER_INPUT_VAR || k == super::USER_MESSAGE_VAR {
            continue;
        }
        user_prompt_parts.push(format!("{k}: {v}"));
    }
}

// 去重后 user 段可能为空；部分 provider 拒绝空 user 消息
if user_prompt_parts.is_empty() {
    user_prompt_parts.push(
        "请基于系统提示中的上游节点输出完成本次分析任务。".to_string()
    );
}
let user_prompt = user_prompt_parts.join("\n");
```

**为什么无损**：`context_sources` 的正文仍在 system prompt 中（`4d` 段），
只是不再重复 raw JSON；`input_mapping` 的结构化字段不走这条路径，不受影响。
**风险**：少数模型依赖 user 段的 raw JSON 定位字段。用变量默认 `system_only`，
A/B 不通过时一键改回 `legacy`。
**预期**：所有 Agent 节点 input tokens 约减半；关键路径预计 −2.5 ~ −3 min。
**开关**：模板变量 `context_prompt_mode = system_only | legacy`。

---

### P0-B 分析师工具调用程序化（固定序列预取）

**目标**：把“模型决定调工具 → 执行 → 回填 → 再决定”改成“系统按固定序列并行预取 → 模型单次输出”。
保留工具集、保留数据、保留 prompt 语义；只去掉模型自主选工具的轮次。

**改动文件**

- `agent_executor.rs`
  - 新增模块级常量 `PREFETCHABLE_TOOLS`（只收录参数可由 `stock_code` 确定的工具）；
  - 新增 `build_prefetch_args(tool_name, stock_code) -> Value`；
  - 新增 `prefetch_tools(context, tools, stock_code) -> Vec<(String, Result<Value, String>)>`；
  - 在 `agent_executor.rs:1030` 附近（messages 构造后、`for round` 之前）插入预取；
  - `agent_executor.rs:1179` 的 `tools` 改为 `tools_for_llm`（剔除已预取的工具）；
  - 预取结果追加到 system prompt 的“上游节点输出”段之后。
- `seed_variables.rs`：新增 `analyst_tool_mode`，默认 `auto`（保持现状）；
  A/B 通过后改 `fixed_prefetch`。
- `seed_stock_analysis.rs`：`TEMPLATE_VERSION` 再 +1（变量种子）。

**固定序列（建议白名单）**

```rust
const PREFETCHABLE_TOOLS: &[&str] = &[
    "get_stock_quote", "get_stock_kline", "get_stock_news",
    "get_stock_money_flow", "get_stock_dragon_tiger", "get_north_bound_flow",
    "get_stock_margin_data", "get_stock_pledge_data", "get_stock_announcements",
    "get_stock_consensus_eps", "get_stock_institutional_visits",
    "get_stock_peers", "get_stock_concept_blocks", "get_stock_option_pcr",
    "get_limit_up_pool", "macro_data_snapshot", "get_hot_stocks",
    "compute_scoring", "compute_kdj", "compute_obv", "compute_valuation",
    "detect_earnings_surprise",
];
// 参数：默认 {"stock_code": <stock_code>}；无参数工具（macro/limitup/hot）传 {}
// get_stock_kline 追加 {"limit": kline_limit}
// search_stock / search_news / get_cls_flash 不预取，仍交给模型
```

**Diff（骨架）**

```rust
// 1) messages 构造后
let tool_mode = context.variables.get("analyst_tool_mode")
    .and_then(|v| v.as_str()).unwrap_or("auto");
let mut tools_for_llm = tools.clone();
let mut prefetch_note = String::new();

if tool_mode == "fixed_prefetch" && !tools.is_empty() {
    let stock_code = context.variables.get("stock_code")
        .and_then(|v| v.as_str()).unwrap_or_default().to_string();
    let kline_limit = context.variables.get("kline_limit")
        .and_then(|v| v.as_u64()).unwrap_or(120);
    let targets: Vec<_> = tools.iter()
        .filter(|t| PREFETCHABLE_TOOLS.contains(&t.name.as_str()))
        .cloned()
        .collect();
    let futs = targets.iter().map(|t| {
        let args = build_prefetch_args(&t.name, &stock_code, kline_limit);
        let name = t.name.clone();
        async move {
            let r = tokio::time::timeout(
                Duration::from_secs(20),
                execute_tool(context, &name, args),
            ).await;
            (name, r)
        }
    });
    let results = futures::future::join_all(futs).await;
    prefetch_note.push_str("\n\n--- 系统预取工具结果（无需再次调用）---\n");
    for (name, r) in results {
        match r {
            Ok(Ok(v)) => prefetch_note.push_str(&format!(
                "\n[{name}] {}\n", serde_json::to_string(&v).unwrap_or_default()
            )),
            Ok(Err(e)) => prefetch_note.push_str(&format!("\n[{name}] 预取失败: {e}\n")),
            Err(_) => prefetch_note.push_str(&format!("\n[{name}] 预取超时\n")),
        }
    }
    tools_for_llm.retain(|t| !PREFETCHABLE_TOOLS.contains(&t.name.as_str()));
}

// 2) render 之后追加（不破坏现有模板变量渲染）
let system_prompt = format!("{system_prompt}{prefetch_note}");

// 3) 请求构造
tools: if round == 0 { tools_for_llm.clone() } else { None },
```

**质量影响**：模型拿到的是**更多**数据（固定序列覆盖它通常会调的 2–6 个工具），
但失去“临时决定再调一次”的自由；`search_stock/search_news` 仍保留自主调用。
因此标为 **低损**，先 `auto` 默认，A/B 通过后切 `fixed_prefetch`。
**预期**：分析师单节点 154 s → ~90–100 s；阶段最慢节点 ~1.5–1.7 min。
**开关**：`analyst_tool_mode = auto | fixed_prefetch`。

---

### P0-C 结构化输出（Json 节点）

> ⚠ **本节下方的「改动文件 / Diff / 开关」是 2026-10-25 的原始方案，已被 2026-10-26 的
> v187 订正取代**（见本节末尾「⚠ 2026-10-26 订正」）。保留原文是为了留下「它当时怎么被
> 做成了默认关的开关」这一步决策记录。

**范围**：只对 `OutputMode::Json` 的节点（`trader`、`debate-convergence--*`、
`risk-convergence`、`value-investor--*`）开启 `response_format=json_object`。
分析师是 `Text` 模式，不受影响。

**改动文件**

- `agent_executor.rs:1188`（主请求）与 `:1869`（VERDICT 重试）：
  `response_format` 由 `None` 改为条件值；
- `seed_variables.rs`：新增 `structured_output_mode`，默认 `off`；
- `seed_stock_analysis.rs`：版本 +1。

**Diff**

```rust
// 主请求之前
let json_mode = matches!(
    an.config.output_mode,
    axagent_harness::workflow_types::OutputMode::Json
) && context.variables.get("structured_output_mode")
    .and_then(|v| v.as_str()) == Some("json_object");

let response_format = json_mode
    .then_some(axagent_harness::types::ResponseFormat::JsonObject);

// ChatRequest { ..., response_format, }
```

**重试策略**：保持现状（校验失败只走 1 次 `fallback_model`，仍失败降级），
新增 `max_schema_retries` 不必要；在计划里显式写明“禁止无限续接”，
由 `verdict_retry_attempted` 与 `strict_mode_fallback` 两个已有字段兜底。
**预期**：减少 Json 节点解析失败/降级；对耗时影响小（~5–10 s），主要是质量稳定性。

**⚠ 2026-10-26 订正（v187，已落地）**：上面那版把本项做成了**默认 `off` 的模板变量开关**
（`structured_output_mode`），且只接了主通路两处 —— 这两点都与「开启原生结构化输出」的
要求相反：

1. **默认 off** ⇒ 要求等于没有落地（现网库实测该值一直是 `off`）。v187 退役该变量，
   改为**由节点输出模式直接派生**：`OutputMode::Json` 恒下发，不留「忘了打开」的位置。
2. **只接两处** ⇒ `fallback` 重试、`fallback` 无工具重试、`try_fallback_stream` 三处仍是
   `None`，而这三处正是「主通路失败/静默无输出」时的兜底 ⇒ 越需要约束的那一次越没有。
   v187 四处一并接上，并加接线门 `node_output_requests_never_drop_structured_output`
   （源码扫描 + `execute_plan_mode` 排除区间 + 正控）。

**范围澄清**：`Text`（分析师等）与 `Artifact` 节点**不下发** —— 分析师一次响应同时产出
正文 + `VERDICT` 标签，`json_object` 会把正文逼成 JSON，同时改坏正文与解析契约。
**严格 `json_schema` 仍无生产调用点**：Json 节点在模板里没有声明输出 schema，
`ResponseFormat::JsonSchema` 目前只在 provider 端被消费（见 §2.6）。

---

### P0-D 工具调用硬上限

**现状**：`max_tool_rounds` 已有（普通 2、catalyst 3），但没有单节点“总工具调用数”上限；
到轮次上限有兜底指令，但没有“本次已执行 N 个工具，立即收敛”的计数式兜底。

**改动文件**

- `agent_executor.rs:1564` 的 `for tc in tc_list` 循环；
- `seed_variables.rs`：新增 `max_tool_calls_per_node`，默认 8；
- `seed_stock_analysis.rs`：版本 +1。

**Diff**

```rust
let max_tool_calls = context.variables
    .get("max_tool_calls_per_node")
    .and_then(|v| v.as_u64())
    .unwrap_or(8) as usize;
let mut tool_calls_executed = 0usize;

for tc in tc_list {
    if tool_calls_executed >= max_tool_calls {
        messages.push(ChatMessage {
            role: "system".to_string(),
            content: ChatContent::Text(format!(
                "已达到本节点工具调用上限（{max_tool_calls}）。\
                 请基于现有信息立即输出最终分析结果与 JSON/VERDICT，不要再调用任何工具。"
            )),
            tool_calls: None,
            tool_call_id: None,
            thinking: None,
        });
        break;
    }
    // ... 原 execute_tool / 回填 ...
    tool_calls_executed += 1;
}
```

**预期**：防止异常模型一次返回大量 tool_call 导致线性膨胀；对正常样本无影响。
**开关**：`max_tool_calls_per_node`（0 表示不限制，保持旧行为）。

---

### P1-A 辩论每轮结构化状态摘要（低损，默认关闭）

**当前实现（2026-10-25）**：引擎侧**确定性压缩**，不新增图节点、不新增 LLM 调用、不改 prompt。
自报 `state` 字段作为后续增强（需要改 strict-mode 解析器），本轮不做。

1. `agent_executor.rs::format_debate_state` 从辩手正文确定性提取：
   - 核心论点：报告小标题（`#` / `一、` …）；
   - 已让步：含“承认/让步/接受/退让”的句子；
   - 剩余分歧：含“分歧/质询/反驳/未回答/争议”的句子；
   - 证据引用：含数字 + `%`/元/倍/亿 的句子；
   - 立场：解析包裹 JSON 里的 `verdict.stance/strength_score/confidence`。
   识别不到的分组写明确的“未识别到”句，不拿原文冒充摘要。
2. 只压缩**更早轮次**（`source_round + 1 < current_round`）；紧邻上一轮仍传全文。
3. 由 `context.variables["debate_context_mode"]` 控制：`full`（默认，完全保持现状）/
   `state`（启用压缩）。
4. Agent 输出新增 `promptStats`（system/user/上下文各 source 字符数），
   由 `emit_prompt_stats` 控制，默认关闭。

**为什么这样最优**：
- 不增加 LLM 调用（3 轮仍是 6 次调用，不比现在多）；
- 只压掉“更早轮次”的全文，上一轮全文保留 ⇒ 低损；
- 运行时变量开关，不需要重建图；
- 与 P0-A 去重叠加后，第 3 轮不再出现 85.8 KB 请求体；
- 不改 prompt/解析器 ⇒ 不引入模型不吐 `state` 的新失败面。

**改动文件**

- `agent_executor.rs`：`format_debate_context_source` / `format_debate_state`、`promptStats`；
- `seed_variables.rs`：新增 `debate_context_mode`、`emit_prompt_stats`。

**预期**：3 轮辩论 mid 档 ~190 s → ~130–150 s；消除第 3 轮 504。
**质量风险**：低损但非无损；必须 A/B（§5.2）。

---

### P1-B 辩论轮次 A/B（2 vs 3）

**前置**：P1-A 上线并稳定。
**方法**：

- 固定样本：从 `stock_analyses` 取 20 只股票（覆盖 4 档、不同市值/行业），
  同一 `as_of_date`，每只跑 2 轮与 3 轮各一次；两批结果用
  `node scripts/stock-analysis-debate-ab.mjs --a-ids <2轮批次> --b-ids <3轮批次>` 比较。
- 质量指标（从 `stock_analyses.decision_json` 与 `workflow_executions.node_executions` 提取）：
  1. `decision_action` 翻转率（同股同截止日，2 轮 vs 3 轮）；
  2. `position_pct` / `confidence` 差值；
  3. `debate-convergence.consensus_score` 差值；
  4. `remaining_disputes` 条数与 `decisive_bull_acks/bear_acks` 覆盖度；
  5. 风险 `converged_risk_level` / `disagreement_score` 差值；
  6. 字段完整率（schema 缺口数）。
- 放行判据（已写进 `scripts/stock-analysis-debate-ab.mjs`，用两批 `analysis_id` 配对；改轮数不换 `template_version`，不能按版本配对）：
  - 方向翻转率 ≤ 基线重复运行的噪声（同一配置跑两遍的翻转率）；
  - `consensus_score` 中位数差 ≤ 5；
  - `remaining_disputes` 覆盖度不低于 3 轮的 90%；
  - 字段完整率不下降。
- 只有全部满足，才允许把 `DEFAULT_DEBATE_ROUNDS` 从 3 改 2；
  否则保留 3 轮，并把“2 轮”作为快速模式显式标注。

---

### P2-A 决策阶段：不做并行化，做 trader 输入瘦身

**事实**：公式 99 ms，`trader` 144 s。并行化最多省 0.1 s，且会切断 f7。
**真正可做的无损/低损项**：

- `trader.context_sources` 里 `t-scoring`、`t-valuation`、`a-catalyst--mid`、
  `a-hot-money--mid` 的**结构化字段已经通过 `input_mapping` 注入**
  （`seed_stock_analysis.rs:5970-6000`），全文再注入一遍是冗余。
- 保留 `research-mgr`（风险叙事）与 `debate-convergence--mid`（共识/分歧）全文；
  其余四项从 `context_sources` 移除，仅保留 `input_mapping`。
- 预期 `trader` 198k → ~50–70k input tokens，144 s → ~70–90 s。
- 标为 **低损**（字段仍在 prompt 中，去掉的是重复全文），A/B 通过后默认。
- 同样手法可用于 `risk-agg/con/neu`：9 份分析师全文 → `analyst-brief--mid`
  + `debate-convergence--mid` + `t-scoring/t-valuation`（低损；A/B）。

**改动文件**：`seed_stock_analysis.rs` 的 `context_sources` 与 `TEMPLATE_VERSION`。

---

### P2-B 独立公式：f7 由“LLM 决策”换成“确定性风险收益比”（B 胜出）

**决策依据（2026-10-25，359 次含 `portfolio-mgr` 的股票运行）**

| 候选 | 覆盖率 | 与 f2 辩论共识 | 与 f3 催化剂 | 与 f4 风险 | 与 f5 估值 | 性质 |
|---|---:|---:|---:|---:|---:|---|
| A `risk-convergence.converged_position_pct` | 280/359 (78%) | 0.498 | 0.287 | 0.242 | — | LLM 仓位建议；与 f2 共享约 25% 方差 |
| B `margin_of_safety / annualizedVolatility` | 291/359 (81%；边际 331、波动率 358) | 0.237 | 0.027 | 0.148 | 0.390 | 确定性风险收益比；与 LLM 因子低相关 |

**结论：选 B。**
- A 是又一个 LLM 综合判断，与 f2 的相关系数 0.50（r²≈0.25），作为 0.10 腿会把辩论共识再加权一遍；它也不满足“公式独立于 LLM 输出”的目标。
- B 是确定性、可复现、可回测的“风险收益比”，与 f2/f3/f4 相关性 ≤0.24；与 f5 的 0.39 主要来自同用估值边际，但 B 的**下行/波动率**维度是 f5 没有的。
- 覆盖率：估值边际 92%、年化波动率 99.7%；组合后 81%。缺失时 f7 权重退出（`f7_signal=0`），不伪造。
- 分布：估值边际有极端值（-2.4e11、+1434%），B 必须先 winsorize（±100）再除以波动率（下限 5%），否则一个坏值会污染后验。

**B 的信号定义（单一权威，放 `analysis-engine::portfolio_formula`）**

```rust
pub fn risk_reward_signal(upside_pct: f64, annualized_vol_pct: f64) -> Option<f64> {
    if !upside_pct.is_finite() || !annualized_vol_pct.is_finite() { return None; }
    if annualized_vol_pct <= 0.0 { return None; }
    let upside = upside_pct.clamp(-100.0, 100.0);
    let vol = annualized_vol_pct.clamp(5.0, 100.0);
    let rr = (upside / vol).clamp(-3.0, 3.0);
    Some(rr / (1.0 + rr.abs()))
}
```

**接线方式（最小侵入、可回滚）**

1. `portfolio-mgr.rhai` 的 f7 增加一个前置分支：`risk_reward_margin_pct` + `risk_reward_vol_pct` 都存在时走 B；否则保持现有 trader 路径。`f7_default=0.10` 与 `leg_bases` 不变（槽位仍有活跃信号，分母不漂移）。
2. 新增 `portfolio-mgr-independent` CodeNode：克隆主 `portfolio-mgr` 节点，改 id/output_var，移除 `trader_*`/`untrusted_trader`/`untrusted_research_mgr` 映射，注入 `risk_reward_margin_pct`/`risk_reward_vol_pct`；从 `risk-convergence`、`data-quality`、`t-*`、四档收敛/`pm-h-*` 取显式边，**不接 trader**，因此可与 trader 并行。
3. `core.rs` 在 `decision_json` 里新增只读 `independentFormula`（action/posterior/f7Source），生产 `decision_action` 仍由主公式 + 风控门决定。
4. 版本 +1（新增节点/边），旧图不重播种则行为不变；回滚 = 回退版本/删除该节点，主公式不受影响。

**为什么不是直接删 f7 / 直接改生产公式**

- 直接删 f7 会让 f7 槽位无活跃信号，`leg_bases` 必须同步去掉 0.10，否则 `weights_collapsed` 会误触发（V66 同型）；
- 直接把生产 f7 换成 B 是业务语义变更，先以独立公式并行产出、做样本外 A/B，避免未经数据就改生产结论。

**A/B 判据**

- 主指标：样本外后涨命中率（买/增/持/减/卖的 `hit_outcome`），不是“与 LLM 一致率”；
- 次指标：动作翻转率、仓位差、置信度差、字段完整率；
- 若独立公式命中率不低于现公式（差值在噪声内），再把生产 f7 切到 B；否则保留主公式，只保留只读对比。

**明确不做**：不把两侧融合成单一置信度分数（会制造第二套权威，违反单一权威裁定）。

- **分歧解释**：把 `attribute_horizon_divergence.drivers` 映射为人类可读的
  “分歧来自风险否决/入场门/缺失腿/负向腿/LLM 档缺失”，不新增综合评分。

---

### P2-C 编排引擎开销

**结论：不建议做编译缓存。**
- 103 个节点的就绪扫描是内存操作；基线节点间空隙亚秒级；
- 每次运行需要建独立的运行态状态机（节点状态、取消、进度回调），缓存静态图仍需克隆；
- 收益 < 1 s，风险和回归面（多执行并发、可变图）不划算。
- 编译缓存**不做**；改为 workflow 结束时一条 `tracing::info!` 阶段耗时汇总
  （`core.rs::log_stock_analysis_stage_timing`，与 bench 脚本同口径），以及基准脚本。

---

## 5. 配置变更

新增模板变量（全部有默认值，旧值/旧图行为不变）：

| 变量 | 默认 | 取值 | 作用 |
|---|---|---|---|
| `context_prompt_mode` | `system_only` | `system_only` / `legacy` | P0-A 上下文去重 |
| `analyst_tool_mode` | `auto` | `auto` / `fixed_prefetch` | P0-B 工具固定序列预取 |
| ~~`structured_output_mode`~~ | — | — | **v187 已退役**：结构化输出由 `OutputMode::Json` 直接决定（见 §P0-C 订正） |
| `max_tool_calls_per_node` | `8` | `0`=不限 | P0-D 单节点工具调用上限 |
| `debate_context_mode` | `full` | `full` / `state` | P1-A 辩论跨轮压缩 |
| `emit_prompt_stats` | `false` | `true` / `false` | 输出 promptStats，供基准脚本 |
| `trader_context_slim` | `false` | `true` / `false` | P2-A trader 输入瘦身（建图时生效） |
| `risk_context_slim` | `false` | `true` / `false` | P2-A 风险输入瘦身（建图时生效） |

**默认值策略**：
- P0-A 默认 `system_only`（无损，先上）；
- P0-B / P1-A / P2-A 默认保持旧行为，A/B 通过后再改默认；
- **P0-C 例外（2026-10-26 订正）**：它曾被做成默认 `off` 的开关，等于没落地 ⇒ **v187 起
  由节点输出模式直接派生**，不再有开关可切；
- 任何一项出问题，改回旧值即可，不需要回滚代码（只有 P2-A 是建图期开关，需要重播种；
  P1-A 的 `debate_context_mode` 是运行时变量，不需要重播种）。

---

## 6. 测试与基准脚本

### 6.1 `scripts/stock-analysis-bench.mjs`

输入：`--runs N`、`--version 179|180`、`--stock 003006`。
数据源：`workflow_executions.node_executions`（JSON 数组）+ `stock_analyses`。

按节点 id 分组：

| 阶段 | 匹配 |
|---|---|
| analyst | `a-*` |
| brief | `analyst-brief--*` |
| debate | `debate-bull-bear--*`、`bull-r*`、`bear-r*` |
| convergence | `debate-convergence--*` |
| value | `value-investor--*`、`value-verify--*` |
| risk | `risk-agg`、`risk-con`、`risk-neu`、`risk-convergence` |
| research | `research-mgr` |
| llm_decision | `trader` |
| independent_formula | `portfolio-mgr-independent` |
| horizon | `pm-h-*`、`pm-arbiter`（四档子工作流，单列，避免拉长 formula） |
| formula | `portfolio-mgr`、`portfolio-risk-gate` |
| tail | `rule-check`、`quality-gate`、`decision-explainer`、`notify-*`、`store-*` |
| misc | trigger/tool 等其余节点 |

输出字段（已实现）：
- 每阶段 `wall_ms`（组内最早 `started_at` → 最晚 `completed_at`）；
- 每阶段 token 合计（agent 节点 `output.usage`）；
- 每节点 `execution_time_ms`、`input_len`、`content_len`；
- 辩论每轮 `promptStats`（若 `emit_prompt_stats=true`）；
- `criticalPathApproxMs`（关键阶段 `maxNodeMs` 之和，近似值）。

用法（已实现）：
```bash
node scripts/stock-analysis-bench.mjs --runs 5 --version 180
node scripts/stock-analysis-bench.mjs --runs 20 --stock 003006
node scripts/stock-analysis-bench.mjs --compare 179 180 --out output/bench.json
```

### 6.2 `scripts/stock-analysis-quality-diff.mjs`

输入：两个 `template_version`（`--a/--b`）或两批 `analysis_id`（`--a-ids/--b-ids`）。
输出（已实现）：
- 同股同 `as_of_date` 的 `decision_action` / `position_pct` / `confidence` 差值；
- `horizon_decisions` 四档动作与仓位差（`horizonChanged` + 每档明细）；
- `decision_json.agreementBreakdown` / `conflictType` / `f7WeightPct`；
- `debate-convergence` 的 `consensus_score`、`remaining_disputes` 条数、
  `decisive_*_acks` 条数；
- `risk-convergence` 的 `converged_risk_level`、`disagreement_score`；
- schema 完整率（action/positionPct/confidence/riskLevel/agreementBreakdown）；
- `outcome` 胜率（win/loss 标签，可缺省）。

判据：
- 不以“单只股票结论相同”为通过标准；以**批次统计**为准；
- 方向翻转率、字段完整率、分歧覆盖度三项全部不劣于基线噪声；
- 任何一项劣化 > 10%，该项优化回退或进入专门 A/B。

### 6.3 单元/集成测试

- `context_prompt_mode`：同一节点在 `legacy` / `system_only` 下
  断言 system 段含正文、user 段不再含 raw JSON；`context_sources` 为空时行为不变。
- `max_tool_calls_per_node`：构造一次返回 12 个 tool_call 的测试替身，
  断言只执行 N 个且最后一条 system 消息含“立即输出”。
- `fixed_prefetch`：构造 3 个可预取工具 + 1 个不可预取工具，
  断言预取并发执行（时间上不串行）、不可预取工具仍下发给模型。
- `debate_context_mode=state`：R3 的 context 中 R1 为 state 摘要、R2 为全文；
  `state` 缺失时退化为确定性摘要。
- 负控：每个开关做“打开/关闭”两组，断言红条数只对应被破坏的那一格。

**已补（2026-10-25）**：`agent_executor::context_optimization_tests` **10/10 通过**，覆盖
开关谓词、去重 user 段、12-call 上限、预取白名单/保留不可预取工具、R3 压缩边界、
state 缺失退路、UTF-8 截断，以及**真实 tool callback 的并发预取**（3 个 60ms 工具
`join_all`，实测 <170ms）与失败工具保留；`analysis-engine::risk_reward_signal_tests` 6/6。
**仍需补**：完整 LLM 流 adapter mock（需要 ProviderRegistry/Adapter 替身；它验证的是
既有请求装配，不是本批新逻辑；strict-mode 解析已有既有测试覆盖）。

---

## 7. 回滚方案

| 改动 | 开关 | 回滚动作 | 是否需重播种 |
|---|---|---|---|
| P0-A 去重 | `context_prompt_mode` | 改 `legacy` | 否 |
| P0-B 预取 | `analyst_tool_mode` | 改 `auto` | 否 |
| P0-C 结构化输出 | **无开关（v187 结构性）** | 只能回滚代码；变量开关不能关掉它 | 否 |
| P0-D 工具上限 | `max_tool_calls_per_node` | 改 `0` | 否 |
| P1-A 辩论压缩 | `debate_context_mode` | 改 `full` | 否（prompt 的 state 字段是附加的） |
| P2-A 输入瘦身 | `trader_context_slim` / `risk_context_slim` | 改 `false` | 是（建图时生效） |
| 模板版本 | `TEMPLATE_VERSION` | 回退版本号 + 重播种 | 是 |

**兜底**：所有代码改动默认行为 = 旧行为；最坏情况回退到 v179 的模板快照
（`workflow_templates` 的 `nodes/edges/variables` 三列可整体还原）。

---

## 8. 验收对照

| 需求验收 | 本方案对应 | 预期 |
|---|---|---|
| 总耗时 ~7 min | P0-A + P0-B + P2-A | 11.3 → ~7.0–7.5 min |
| 分析师 ~1.5 min | P0-A + P0-B | 154 s → ~90–100 s |
| 辩论 3 轮 ~2 min，第三轮不超长 | P0-A + P1-A | mid 档 ~2.2 min；请求体从 85.8 KB → ~20–30 KB |
| 风险 ~2 min，三视角并行且结果一致 | 已并行；P0-A + P2-A | 106 s → ~60–80 s；串行/并行结果逐字段一致 |
| 决策 1–1.5 min，两决策并行 | **不并行**；P0-A + P2-A | trader 144 s → ~70–90 s；公式仍 0.1 s |
| 结论质量不低于基线 | §6.2 A/B + 默认关闭有损项 | 无损项无需 A/B；有损项全部 A/B 后放行 |

---

## 9. 提交顺序（2026-10-25 合并为一批）

用户裁定：并行会话在改同一棵树，**整批代码先落地，最后统一跑全量门禁**，不在阶段边界反复编译。

1. 一次性落地 `P0-A/B/C/D` + `P1-A` + `P2-A/B` + 变量 + 两个脚本；`TEMPLATE_VERSION = 180`。
2. 全量门禁：`cargo fmt --all --check`、
   `cargo clippy --workspace --all-targets --all-features -- -D warnings`、
   `__TAURI_WORKSPACE__=true cargo test --workspace`、前端 `tsc/dprint/oxlint`、`node --check scripts/*.mjs`。
3. 门禁绿后按变量分项 A/B（`analyst_tool_mode` / `debate_context_mode` /
   `trader_context_slim` / `risk_context_slim`），用 `stock-analysis-bench.mjs` 与
   `stock-analysis-quality-diff.mjs` 出读数，再决定默认值。

---

## 10. 实施状态（2026-10-25）

| 项 | 状态 | 默认开关 |
|---|---|---|
| P0-A 上下文去重 | 代码已落 | `context_prompt_mode=system_only` |
| P0-B 工具固定序列预取 | 代码已落 | `analyst_tool_mode=auto`（A/B 后切） |
| P0-C Json 节点 `response_format` | 代码已落（v187 订正：四处接线 + 结构性开启） | 恒开（无开关） |
| P0-D 单节点工具总上限 | 代码已落 | `max_tool_calls_per_node=8` |
| P1-A 辩论跨轮确定性压缩 | 代码已落 | `debate_context_mode=full`（A/B 后切） |
| P1-A `promptStats` | 代码已落 | `emit_prompt_stats=false` |
| P2-A trader / risk 输入瘦身 | 代码已落（建图期） | 两个 `*_context_slim=false` |
| P2-B 独立公式（f7 换源，详见 §12） | 代码已落 + `decision_json.f7ReferenceFormula` | 常开（只读、0.1 s） |
| P2-C 编排编译缓存 | 不做 | — |
| P2-C 阶段耗时日志 | 已落（`core.rs::log_stock_analysis_stage_timing`） | — |
| §6.3 纯函数/开关测试 | 已补（`context_optimization_tests` + `risk_reward_signal_tests`） | — |
| §6.3 execute 级 mock/负控 | 工具回调+并发预取已补（10/10）；完整 LLM 流 adapter mock 未做（非新逻辑） | — |
| §6.1 / §6.2 基准与质量脚本 | 已扩（version/stock/compare/out、promptStats、input/content、critical path；质量脚本含 confidence/horizon/conflict/schema/outcome/辩论风险指标） | — |
| P1-B 2 轮 vs 3 轮 A/B 脚本 | 已落（`scripts/stock-analysis-debate-ab.mjs`，两批 analysis_id） | — |

**门禁读数（2026-10-25，代码落完后）**

- `cargo fmt -p axagent` / `-p axagent-rt-workflow` / `-p axagent-analysis-engine`：**0**。
- `cargo test -p axagent-analysis-engine risk_reward_signal_tests`：**6/6 通过**。
- `node --check scripts/stock-analysis-bench.mjs` / `stock-analysis-quality-diff.mjs`：**OK**。
- `npm run typecheck`：**0**；`npm run lint:eslint`（oxlint）：**0**；
  `dprint check`（改动的 TS/脚本文件）：**0**。
- 基准脚本已对现网跑通（run `b6008eea`）：analyst 154.6s / debate 200.8s（max 178.5s）/
  risk 106.5s / trader 144.2s / formula **0.2s** / horizon 0.4s；质量脚本可正常配对（当前 v180 尚无样本）。
- `cargo test -p axagent-rt-workflow --lib context_optimization_tests`：**10/10 通过**
  （含真实 tool callback 的并发预取测试）。
- `cargo check -p axagent-rt-workflow --lib`：**0**（15m00s，含 agent_executor 全部改动）。
- `cargo check -p axagent --lib`：**0**（13m42s + 阶段日志后复跑 10m28s，含 seed/core/rhai_pm/decision 全部改动）。
- 主 crate 测试目标已可编译（并行 `axagent-dao` 的 E0063 已由该批收口）：
  `portfolio_mgr_rhai_compiles` **1/1 通过**（`portfolio-mgr.rhai` 语法门）。
- 独立公式图断言 `seeded_template_has_independent_formula_without_trader_dependency`：
  **1/1 通过**（临时 SQLite 建库 + 播种，3.78s）。
- 全量 `clippy --workspace --all-targets --all-features -- -D warnings` 已启动；
  全量 `test --workspace` 待 clippy 结束后跑。
- 新增脚本已对现网/单样本跑通：bench（含 `--compare/--stock/--out`）、
  quality（含 `--a-ids/--b-ids` 与辩论/风险节点指标）、debate-ab（单样本冒烟通过）。
- `core.rs` 阶段日志与 `agent_executor` 新测试是最后两批改动；主 crate/test 目标的
  复跑仍受并行构建锁影响，读数待补（代码已过 `cargo fmt`）。


---

## 11. 呈现层批次（2026-10-25 追加，用户裁定「要加」）

P2-B 落地后 `independentFormula` 只在库与脚本层可比，前端只有类型声明、零消费 ⇒ 两套决策
在界面上从未并排过。本批补的是**这一条链**，不改任何生产动作。

### 11.1 接线（五处，缺一处就静默失效）

| 环节 | 落点 | 干什么 |
|---|---|---|
| 产端 | `src-tauri/src/commands/stock_workflow/core.rs:1423` | `decision_json` 顶层插 `independentFormula`（仅当独立节点产出） |
| 归一化 | `src/lib/agentOutput.ts:921`（`normalizeIndependentFormula`）+ `:511` | **白名单重建必须显式带上**，漏一行＝「字段一直发、界面从未显示」（2026-09-11 `weightsCollapsed` 同一族） |
| 类型 | `src/types/stock-analysis.ts:548`（`IndependentFormulaSnapshot`） | 计数域与四种缺席语义写在类型文档里，消费方不用另查 |
| 数据 | `src/components/stock-analysis/StockAnalysisPage.tsx:83` | `dualViewData.independentFormula` |
| 渲染 | `dual-view/DecisionComparisonPanel.tsx:348` + `stock-analysis/IndependentFormulaComparison.tsx` | 对照块，落在「公式 vs LLM」两列**之后**（不是并成三列） |

### 11.2 为什么不做成三列

上面那一对比的是**两条链互不认同**；本块比的是**同一个公式换掉 f7 输入后的位移**。
并成一张三列表，用户会把后者也读成「LLM 不同意公式」，正是要消除的歧义。
生产侧的三个数（方向/置信/仓位）只进差值句、不重复渲染整列。

### 11.3 缺席四态各占一句（不得压成 0 或空白）

1. 键不存在 ⇒ `…notProduced`（该记录产自 v180 之前，本侧从未跑）；
2. `f7Source` 四态 ⇒ `f7.riskReward` / `f7.trader`（与生产同口径，对照无效）/
   `f7.absent`（该腿无输入，差值只说明「这条腿为空」）/ `f7.unknown`；
3. `weightsCollapsed=true` ⇒ 单列「本侧结论不可用」；
4. 方向档是权威哨兵（`数据缺失`）⇒ `actionAbsent` + `actionNotComparable`，不套档位标签。
另外：生产侧或本侧数值缺位 ⇒ `valueNotComparable`，**不算 0 差**（0 差＝宣称两侧一致）。

### 11.4 本批被门抓到的一处真实跨面缺口

`portfolio-mgr.rhai` 的 f7 腿名自本批起变成**按 `f7_source` 二选一的变量**
（`let f7_name = if … { "risk_reward" } else { "trade_signal" };`）。
`crossCheckDrivers.test.ts` 的抽取器原先只扫 `#{ "name": "字面量"` ⇒ 该腿被静默跳过，
症状是「已有译名 `trade_signal` 被说成死码、真新出现的 `risk_reward` 反而查不出缺译」。
按「改门口径、不削断言」处理：

- 抽取器补认变量形态，按 `let <ident> = …;` 右侧字面量收全值域，并单列 `unresolved` 清单
  （解不出即红，不放行成「这一格不存在」）+ 一条 `ghost_name` 负控；
- `stockAnalysis.crossCheck.leg.risk_reward` 补进 11 语言（`DecisionBanner.tsx:2354` 按
  `crossCheck.leg.<名>` 取文案，缺键会退回渲染原始码串）。

### 11.5 门禁读数（本批）

- `IndependentFormulaComparison.test.tsx`：**13/13**（四态各只命中自己那句 + 数值缺位不算 0 差）
- `agentOutput.independentFormula.test.ts`：**6/6**（透传本身，含「键不存在⇒undefined 而非空对象」）
- `DecisionComparisonPanel.independentFormula.test.tsx`：**7/7**（接线：生产侧数值不被 LLM 侧顶替、
  回退分支不渲染对照块）
- `crossCheckDrivers.test.ts`：**11/11**（原 8 条 + 抽取器自证 3 条）
- 全量 `npm run test:run`：**2791 条 / 260 文件全绿**（含本批 4 个新测试文件 26 条）
- `cargo clippy -p axagent-analysis-engine -p axagent-rt-workflow --all-targets --all-features -- -D warnings`：**0**（5m03s；本批 Rust 面无改动，取的是上一批 agent_executor 的终局 lint）
- i18n 四门（parity / untranslated / placeholder-usage / hardcoded）+ `check_i18n.py`：全绿；
  parity 16217→16238 键 × 11 语言路径集合一致
- `oxlint` 0、`dprint check`（本批 9 个文件）0；`npm run typecheck` **全包只报 1 条，非本批**：
  `src/components/stock-analysis/__tests__/useAnalystConsensusByTier.test.ts:79` `TS2304: Cannot find name 'DataQualityReport'`
  （该 hook 与其测试是并行会话正在写的未跟踪新增，同批已把 hook 接进 `StockAnalysisPage.tsx`）
- `check-contracts` / `check-decision-action-single-source` / `check-horizon-weight-parity`：PASS
- 11 份 locale 整档重写前后 `json.dumps(indent=2)` roundtrip 逐字相等、LF、占位符集合与 zh-CN 一致

### 11.6 遗留

- 回退分支（LLM 整体不可用且无一致性分）不摆对照块：那时生产侧 f7 也没有 trader 输入，
  「差值＝LLM 观点净贡献」这句解释不成立。已由测试钉住，属设计而非缺口。
- 对照块要出真实数据需**重启应用重跑一条分析**（v180 之后的记录才有 `independentFormula`）；
  v180 之前的历史行一律显示 `notProduced`。

### 11.7 全量门禁为何仍取不到「绿」（截至 2026-10-25 05:00）

`cargo clippy --workspace --all-targets --all-features -- -D warnings` 在**主 crate** 短路，5 条编译错误
全部不在本批文件内（本批主 crate 改动 = `stock_analysis_setup/`、`stock_workflow/`、`commands/stock_analysis.rs`）：

| 位置 | 码 | 说的什么事 |
|---|---|---|
| `src-tauri/src/commands/agent_profile.rs:150` | E0061 | 调用 19 个实参，`crates/dao/src/repo/agent_profile.rs:171` 的函数收 16 个 |
| `src-tauri/src/commands/backtest_validation.rs:201`、`:525` | E0599 | `sea_orm::Select` 上没有 `order_by_desc`（SeaORM 2.0 已改名） |
| `src-tauri/src/commands/backtest_validation.rs:247`、`:549` | E0277 | `let Some(ref x) = pick_model.pick_data` 里 `str` 不定长 |

同一条 dao 线在本轮内已换过一次故障形态（`agent_profiles::Column::SuggestedModelId/Temperature/MaxTokens`
的 E0599 → 该批把列与用法两侧一起撤掉后解除 → 现在红在调用参数与 SeaORM API 上）。
按纪律**不代修、不补**：`cargo test --workspace` 依赖同一份构建产物，排在它后面，故本轮终局读数只有
定点那一份。**待那两条线落地后重跑**：`cargo clippy --workspace --all-targets --all-features -- -D warnings`
与 `__TAURI_WORKSPACE__=true cargo test --workspace`。

---

## 12. f7 换源到分析师结构票（2026-10-25 追加，用户两次裁定）

裁定链：①「公式决策与 LLM 决策必须绝对独立，不允许假独立」→ ②「不能退役 f7，把它的计算方法
改为从分析师/辩论/风险分析节点的输出计算」→ ③「f7 与 f2 同源就是重复处理，公式会丧失科学客观性吗」
→ ④「按二做」（给 f7 一份互斥信息，而不是复用 f2/f4 已在吃的变量）。

### 12.1 三条被实证推翻的原方案

| 原方案 | 被什么推翻 | 证据 |
|---|---|---|
| f7 = 确定性风险收益比（B 分支） | `margin_of_safety ≡ dcf.upsidePct`，而 f5 已在吃 `dcf.upsidePct` ⇒ 同一份证据加权两次 | `src-tauri/src/commands/value-verify.rhai:28` + `seed_stock_analysis.rs` 的 `valuation_dcf_upside` |
| f7 = 辩论方向 × 辩论置信 × 风险档（我提的换源表） | f2 已吃辩论方向×收敛度、f4 已吃风险档 ⇒ 同一要素二次计权（共线性），`shared_evidence_weight_pct` 只是事后打折、不消除重复 | 用户反问「那不是同一要素的重复处理吗」 |
| 退役 R-201 空头强制卖出 | 「不允许一删了之」⇒ 改为**换源保留**，判据/0.85 阈值/返回结构未动 | `portfolio_risk_gate` 第 2 步原样 |

### 12.2 现在的 f7（C 分支）：分析师结构票

计算方法保持 V65 的四槽形态，逐槽换到**决策链从未消费**的那组量：
`dir` ← 各位分析师 `(bull_score − bear_score)/100` 的净票；`conf` ← 其 `confidence` 均值；
调制① ← 票间离散度 `1 − sd`（连续折减，不新增阈值表）；调制② ← `risk_llm_eff` 的风险档
（f4 作为加权证据消费，这里只作调制 ⇒ 不产生第二条同权重腿）。
σ=0 沿用既有「退出权重」语义；`untrusted:true` 的实例直接剔除。

聚合放在 `data-quality.rhai`（新输出 `analyst_votes_by_tier`）：它是全图唯一已按「档 × base」
持有各 verdict 的节点 ⇒ 公式侧只需一条映射 + 一条已有边，而不是 69 条映射 + 19 条边。
`industry-chain-analyzer` / `candidate-mapper` 的契约里没有 bull/bear_score ⇒ 如实跳过并计入
coverage，**不按 0 票代入**（0 票会被读成「该分析师中性」）。

### 12.3 建图开关（回滚位）

模板变量 `formula_independent_f7`（boolean，默认 true）决定**哪个节点摘掉 8 个 trader 键**：
true ⇒ 生产 `portfolio-mgr` 摘（走票），参照 `portfolio-mgr-f7-ref` 保留（走 trader，做对照）；
false ⇒ 反过来。trader/research-mgr 的供给边由 `f7_view_target` 单独接到保留 trader 那一侧；
快速链的 `trader-proxy` 出边改为**按派生图实际带 trader 前缀的节点**动态求（原先硬编码 → pm）。
空头否决的报价来源：`portfolio-risk-gate` 的 `target_price` 不再由映射注入，改由
`margin_of_safety_pct ← t-valuation.result.content.margin_of_safety.pct` 在脚本里折算
`current × (1 + pct/100)`；取工具路径而非 value-investor 节点，是因为该形态有实测记录
（DB 现网 `{"level":"无（高估风险）","pct":-86.9}`），而分析师侧形态跨两代 schema。

### 12.4 本轮读数与**未闭环清单**

已取：`cargo test -p axagent-analysis-engine --lib` **859/0**；
`cargo test -p axagent-rt-workflow --test rhai_syntax_check` **9/0**（三个 .rhai 编译通过）；
`crossCheckDrivers.test.ts` **12/12**（新增抽取器负控：分支条件里的字面量不得当腿名）；
i18n parity 11 语言一致（`crossCheck.leg.risk_reward` 随死码撤回、新增 `analyst_vote`）；
`audit-inject-coverage` ①②③ 全绿；`check-horizon-weight-parity` 绿（12 腿未变）；
`cargo fmt --all --check` 仅剩 `rt-workflow/src/work_engine/engine/mod.rs:317` 一处，非本批文件。

未闭环（下一步，勿当已完成）：
1. `stock_workflow/decision.rs:899` 仍读 `wf.results["portfolio-mgr-independent"]`，`core.rs:1423`
   仍写 payload 键 `independentFormula` ⇒ 节点改名后这条链**当前是断的**（前端会恒显示「未产出」）。
   须改成 `portfolio-mgr-f7-ref` + 键名与前端五环、11 语言文案同批换。
2. `decision.rs` 的 `merge_price_fields_from_llm` 仍会在公式价位为 null 时塞进 LLM 的
   targetPrice/stopLoss ⇒ 独立口径下必须改为不合并并显式声明。
3. 凯利赔率（`portfolio-mgr.rhai:2128` 区）生产侧因无 trader 输入自动落到 posterior 分档表，
   但归因标签仍写「波动率fallback」（说谎：自变量是 effective_posterior）⇒ 按 12.2 的口径改名并
   决定赔率分子是否改用安全边际、分母用逐档 `sl_pct_for(档)`。
4. 逐档 `sanity` / R-202 / R-204 那族针对 trader 报价的自洽检查，在生产侧成为死分支 ⇒
   要么把判据对象换成「票/估值量的取值域」，要么显式声明不适用，不得静默。
5. 主 crate 编译与测试读数仍拿不到：`agent_profile.rs:150` E0061（他人）与
   `backtest_validation.rs` 的 `order_by_desc` E0599 ×2、`str` 不定长 E0277 ×2 未清。
6. 现网验证：需重启 + 重播种到 v183，跑一条分析，看 `f7_source == "analyst_votes"`、
   票 coverage、以及空头否决的 `bearishRuleCoverage` 是否如实成句。

### 12.5 断链已接通的四处（2026-10-25 第二轮）

| 原断链 | 现状 |
|---|---|
| `decision.rs` 按旧节点名取数、`core.rs` 写旧 payload 键 ⇒ 面板恒显示「未产出」 | 节点 `portfolio-mgr-f7-ref`、键 `f7ReferenceFormula`、提取函数改名 `extract_f7_reference_formula_json`；阶段耗时表的节点名同步 |
| 公式价位为空时仍借 LLM 的 targetPrice/stopLoss | `merge_price_fields_from_llm` 加门：`f7_source` 在场且 `!= "trader"` ⇒ 不合并（老记录无该键 ⇒ 维持原行为，不改历史） |
| 赔率来源标签「波动率fallback」说谎 | 新增第二口径 `估值安全边际 ÷ 本档止损%`（`valuation_margin_pct` 新映射），三态标签 `trader / 估值安全边际÷本档止损 / effective_posterior 分档表` |
| 自洽检查族与脚本内空头否决静默不跑 | 新 `decision_trail` 条目 `R-213`：本侧未注入 trader 决策输入 ⇒ R-202/R-203/R-204 与 R-201 均不适用（句里点名，不写成「无问题」） |

呈现层随之改名并补第 5 环：`F7ReferenceComparison`（原 IndependentFormulaComparison）、类型
`F7ReferenceSnapshot`、i18n 组 `stockAnalysis.f7Reference`（24 叶 × 11 语言，占位符
`independent`→`reference` 同步）、主决策新增透传字段 `f7Source`（面板靠它说「本条到底读没读
LLM 决策」）；两侧同口径时出 `sameBasis` 那句并置 `data-same-basis`，负控已配。基准/质量脚本
同步改读新键（不改会恒读 null ⇒ 把「读不到」显示成「对照匹配率 0/0」的假绿）。

第二轮读数：全量 `npm run test:run` **2816 条 / 264 文件全绿**；i18n 五门全绿；
`check-contracts` / `check-decision-action-single-source` / `check-horizon-weight-parity` /
`check-line-endings` PASS；`audit-inject-coverage` ①②③ 绿；`cargo fmt --all --check` **0**；
`crossCheckDrivers` 12/12；`rhai_syntax_check` 9/9；analysis-engine 单测 859/0。
`check-single-source-facts` 由 2 处新增客观错回到 1 处：多出来的那条是 `crates/harness/src/rhai_ast_cache.rs`
引用 `stock_workflow/decision.rs:2693` 被我的插入推成空行 —— 查 HEAD 后确认**该引用本就腐烂**
（HEAD 上那行是一条与深度配置无关的白名单注释），已按内容重指到权威 `RhaiSandboxLimits`
（`rhai_registry.rs`）并去掉行号写法；文档类改动、只改不提交。

仍未取到的读数：`cargo check -p axagent --lib` 正在跑（本批改了 decision/core/seed/rhai_pm 四处主 crate
文件，必须过编译）；`clippy --workspace` 与 `cargo test --workspace` 仍排在并行会话那两条线之后
（`backtest_validation.rs` 的 `order_by_desc` E0599、`agent_profile.rs:150` E0061）；
现网验证需重启 + 重播种到 v183 跑一条真分析。

### 12.6 `cargo check -p axagent --lib` 抓回的两处自错（2026-10-25 第三轮）

「断链接通」那一批改完只跑了前端与 rhai 编译门，主 crate 的编译读数直到这一轮才拿到，抓出两条本批自错：

| 位置 | 码 | 成因 |
|---|---|---|
| `seed_stock_analysis.rs`（生产侧摘 trader 键那一段） | E0596 | `pm` 是不可变绑定，`if let WorkflowNode::Code(ref mut c) = pm` 要 `&mut` ⇒ 改成影子绑定 `let mut pm = pm;`，并把 `nodes.push` 移进两个分支 |
| `rhai_pm.rs` 的 `pm_portfolio_risk_gate` | E0061 | 我把纯函数签名回退成 9 参（R-201 从退役改为换源），注册闭包还按 8 参在调 ⇒ 三处同进退只改了两处 |

教训入档：**换源的收口点有三处（rhai 调用 / 注册闭包 / 纯函数签名），改一处而不同批改另两处，编译只在其中一处报错**，
而那一处恰是主 crate —— 主 crate 被他人错误挡住时，这条自错可以整轮不被发现。定点门只覆盖
analysis-engine 与 rt-workflow，不等于主 crate 已过。

同轮清掉昨天 B 分支留下的死链：`portfolio_formula::risk_reward_signal`（含 6 条边界单测）与
`pm_risk_reward_signal` 注册点整体删除 —— 调用方归零后不留尸体（否则下一个读代码的人会以为
f7 还有第三条口径）。`crossCheck.leg.risk_reward` 译名已在上一轮随死码撤回。

### 12.7 前缀错与路径存在性门（2026-10-25 第四轮）

`analyst_votes_by_tier` 起初写成 `data-quality.content.…` —— `data-quality` 是 CodeNode，产物在
`.result.` 下（同表 `dqi_score ← data-quality.result.score` 为证；`.content.verdict` 只属于 AgentNode，
真库 `workflow_templates.nodes` 亦核过）。错前缀的症状不是报错而是路径解析为 None ⇒ 票恒空 ⇒
f7 σ=0 退出权重 ⇒ **同一批里第二次出现「映射写了、数据到不了」**，且
`audit-inject-coverage.mjs`（键名↔键名）与 `rhai_syntax_check`（脚本可编译）都抓不到它。

于是新增门面 `seed_stock_analysis_setup::rhai_producer_paths_exist_in_node_scripts`：
凡 `<CodeNode id>.result.<首段>` 形态的映射值，该节点脚本里必须出现这个首段（`"k"` 或 `k:` 两种写法都认），
否则逐条点名「源 CodeNode 的脚本里找不到产物键」。ToolNode/AgentNode 刻意不在门面内
（前者 JSON 在 Rust 侧构造、后者受 LLM 契约约束，纳进来只会造假阳）；门面另带
`checked > 20` 的「判据没电自证」下限。

读数（截至本轮，`cargo check -p axagent --lib`）：analysis-engine 全量 **853/0**
（删死链后由 859 减 6，条数自洽）；主 crate 只剩 **4 条错误，全部在
`src/commands/backtest_validation.rs`**（`order_by_desc` E0599 ×2、`str` 不定长 E0277 ×2，非本批文件）
—— 第三轮我那两条（E0596 / E0061）已消失，说明本批主 crate 代码通过类型检查。
但**测试目标因此整体无法执行**：`cargo test -p axagent --lib`（含新增的两条播种断言与门面）
要等那条线落地后才取得读数，现报「已写未跑」。

### 12.8 播种测试首次执行（2026-10-25 第五轮）

他人那 4 条 `backtest_validation.rs` 错误落地修好后，`cargo test -p axagent --lib
commands::stock_analysis_setup` 第一次能执行，连抓三条本批自错：

| 症状 | 根因 | 处置 |
|---|---|---|
| 快速链 7 条派生测试全红 | 参照节点没进 `FAST_OPTIONAL_NODE_IDS` ⇒ 派生图里一条 `trader.content.verdict.*` 都改不写成 0 ⇒ 「proxy 产出无人消费」守卫按设计报错 | 把参照节点加入可选保留集（**不放宽守卫**，它这次判得对） |
| 新门面自证红：只查了 15 条 | 我把 `checked > 20` 当判据写死，等于把条数锁成判据 | 改 `>= 10` 并注明「防扫描面塌掉，不防合法增减」 |
| f7 接线断言红 | 我的测试期待 `.content.` 前缀，而种子已修成正确的 `.result.` | 修断言（种子侧不动） |

终局读数：`cargo test -p axagent --lib commands::stock_analysis_setup` **95 通过 / 0 失败**
（含两条本批新增：`seeded_template_moves_f7_to_analyst_votes_and_keeps_trader_only_in_reference`
读真落库 JSON 断言「生产无 trader 键 + 参照有 + 边各就各位 + 门改吃 `margin_of_safety_pct` 且
`t-valuation → 门` 边在」，`rhai_producer_paths_exist_in_node_scripts` 扫全图 15 条 CodeNode 产物路径零违规）。

随后启动 CI 口径全量：`cargo clippy --workspace --all-targets --all-features -- -D warnings`
与 `__TAURI_WORKSPACE__=true cargo test --workspace`（串行，共用同一份构建锁），读数待补。

### 12.9 全量门禁终局（2026-10-25 第六轮）

| 门禁 | 命令 | 读数 |
|---|---|---|
| 全量 lint | `cargo clippy --workspace --all-targets --all-features -- -D warnings` | **0** |
| 全量测试 | `__TAURI_WORKSPACE__=true cargo test --workspace --all-features` | **8943 通过 / 0 失败**（150 个 test binary） |
| 播种模块 | `cargo test -p axagent --lib commands::stock_analysis_setup` | **95 / 0** |
| 格式化 | `cargo fmt --all --check` | **0** |
| CI 的测试门 | `cargo nextest run --workspace` | **本地不可执行**（未装 `cargo-nextest`，与既有记忆一致）⇒ 该格只能由 CI 取 |

过程中一次红需要记账：第一次 `cargo test --workspace`（不带 `--all-features`）在
`axagent-disk-cache (lib test)` 报 `cannot determine resolution for the attribute macro tokio::test`。
两条排查都不支持「特性配置」解释：该 crate 无 `[features]`、`tokio = { workspace = true }` 且
`[workspace.dependencies] tokio` 是 `features=["full"]`（`macros` 必在），且它自 2026-09-18
之后无人改动（`git log -- crates/disk-cache/` 最近一条即 09-18）。⇒ 按机制推演应当恒可解析，
**第二次读数已到位**：完全同一条命令
（`__TAURI_WORKSPACE__=true cargo test --workspace`，不带 `--all-features`）复跑 =
**8943 通过 / 0 失败 / 149 个 test binary、退出码 0**。同命令两样结果 ⇒ 第一次是
构建产物/缓存层面的假错，不是代码缺陷；处置是**不改代码**，若 CI 上复发则按 sccache/target
污染路径排查（`cargo clean -p axagent-disk-cache` 重编即可复现判定）。

### 12.10 状态收口

代码与本仓可取的门禁全部绿：全量 clippy 0 / 全量 test 8943 条 0（两种特性组合各一次）/
fmt 0 / 播种模块 95 条 0 / 前端 vitest 2816 条 0 / i18n 五门 / 契约与权重对账门 /
注入覆盖门 / 新加的 CodeNode 产物路径门 15 条零违规。

唯一未闭环：**现网验证**（要人做）—— 重启应用重播种到 v183 → 跑一条真分析 →
看 `decision.f7Source == "analyst_votes"`、`analyst_votes_by_tier` 各档收了几位分析师
（产业链/候选映射两个 base 契约无 bull/bear_score ⇒ 应当被跳过而非按 0 票计入）、
空头否决的 `bearishRuleCoverage` 是否如实成句、以及 f7 口径对照块两侧的行是否分别说对口径。

## 13. 乙方案消费面普查（2026-10-25 追加，用户「先起乙的普查」）

### 13.0 普查口径

「消费点」= 任何在运行时读主链 13 因子模型产出列的地方，分四类：
**A 结论类**（读不到就改结论或改分类）｜**B 留痕类**（读不到就少说一段话）｜
**C 门禁类**（读不到就红）｜**D 呈现类**（前端白名单收着的那几格）。

审计对象 = `portfolio-mgr.rhai` 顶层输出块（`:3783-3906` 后验族 / 因子表 / 坍缩族、
`:4011-4020` f7 族 / computation_logs、`:4033+` effective_params、`:3984` decision_trail），
外加「整包透传后再取键」的地方（`decision_json` 落库后被多处二次解析）。

### 13.1 A 类：结论会变（退役后必须逐条处置）

| # | 消费点 | 读的键 | 键缺失时的实际表现 | 严重度 |
|---|---|---|---|---|
| A0 | `stock_analysis.rs:200-299` `compute_what_if` | **整份 `portfolio-mgr.rhai` 重跑**（`include_str!` 同一文件）+ 28 项可调参数 | **What-If 面板整块没有对象可重跑**；同时 `decision_trail` / `technical_veto` / `simulation_gate` 三个产出一起消失 | 高 |
| A1 | `stock_workflow/decision.rs:5559` `merge_price_fields_from_llm` | 顶层 `f7_source` | **方向翻转**：键消失 ⇒ `is_some_and(src != "trader")` 为假 ⇒ 放行 LLM 价位合并 ⇒ 公式侧 targetPrice/stopLoss 被对手方覆盖 ⇒ 假独立重新出现 | 高 |
| A2 | `stock_analysis_setup/mod.rs:1706` → `reflection-comparator.rhai:176` | `sub-analysis.portfolio-mgr.posterior` | 缺席 ⇒ `post = 0.0` ⇒ `classify_confidence_level(0.0)` = 非常低 ⇒ 反思归因文本被写成「决策置信度非常低(posterior=0.0)」—— 把「读不到」写成「非常低」 | 高 |
| A3 | `stock_workflow/decision.rs:2462-2480` | `f7_free.{f7_weight,total_weight,posterior,action}` | `f7_free_info = None` ⇒ `f7_free_action_score = None` ⇒ 在「LLM 无视角输入」分支落到 `f7_dominant`（本应是 f7_low/moderate/high_influence）⇒ 静默改冲突分类；`f7_weight_pct` 同时变 null | 中高 |
| A4 | `portfolio-mgr.rhai:461-464 / 2335 / 2490 / 2677 / 2695` | 四路分支全缺席时的兜底 | **因子模型是四档架构的失败兜底**：四个 `pm-h-<档>` 节点是 `continue_on_fail: true`（`seed_stock_analysis.rs:6858` 等）⇒ 任一档失败即静默容忍 ⇒ 四路全缺席 ⇒ 档位按 `effective_posterior` 阈值定、仓位走主链凯利、置信走 `final_confidence_pure` ⇒ **这条路径上因子模型确实产出结论** | 高 |
| A5 | `stock_workflow/hooks.rs:190-192` `divergence_attribution` | 顶层 `posteriorRaw` + `posterior` + `effective_params/action_*_threshold` | 整块「风险门槛是否改了档」不产 ⇒ 前端 `DecisionBanner.tsx:2396-2398` 那三行退化成 `?` | 中 |

⇒ **「主链因子模型不产出结论」这句话只在四档分支全部在场时成立**。四档在场时 action /
positionPct / confidence / 档位确实全取自所选档行（`portfolio-mgr.rhai:2490 / 2677 / 2695 / 2335`）；
一旦四档缺席，它就是唯一结论源。

### 13.2 B 类：留痕会少四段

| 消费点 | 读的键 | 键缺失时的实际表现 | 严重度 |
|---|---|---|---|
| `stock_workflow/hooks.rs:152` | `/evidence/factors` 的 σ/w/name | 负贡献腿列表为空 ⇒ `drivers` 少 `negative_legs` ⇒ 决策卡不再点「哪条腿把结论压下去了」 | 中 |
| `crates/analysis-engine/src/evidence_audit.rs:681` | `/evidence/factors` | A 面因子腿整段为空；`unverified_weight_ratio` 被读成 `0.0`（＝「全部对账通过」）⇒ 语义反了 | 中 |
| `evidence_audit.rs:770-771 / 143-171` | `/evidence/weightSum`、`/evidence/avgSignal`、`/risk_quant/f4_adjusted`、f9..f12 扁平信号 | 基准缺失 ⇒ `closure = NotComputable`、各腿 `Reconcile::NoBasis` | 低 |
| `stock_workflow/core.rs:1430-1437` | `f7ReferenceFormula.{posterior, f7_source, f7_free.f7_weight_pct, weightsCollapsed}` | 写进 `decision_json` 的就是 null（不 panic、不省略）⇒ 参照块那几格变空 | 中 |
| `seed_stock_analysis.rs:7463-7465` | `portfolio-risk-gate.result.decision_trail` / `.computation_logs` | `decision-explainer`（LLM 节点）失去规则追溯码与量化留痕 ⇒ 决策依据说明书那两段空 | 中 |

### 13.3 C 类：会红的门禁与测试（退役必须同批处置）

| 位置 | 锁的是什么 | 退役后 |
|---|---|---|
| `evidence_audit.rs:1462-1470` | `portfolio-mgr.rhai` 的**源码字面量**（`"avgSignal": (avg_signal*100.0)...`） | 必红 |
| `stock_workflow/rhai_registry.rs:2210-2240` | `R-F12-OVERBOUGHT` 追溯条目（产自主链 f12 权重下限，`portfolio-mgr.rhai:1890-1899`） | 必红 |
| `stock_workflow/hooks.rs:1342-1490` | 归因单测夹具（`evidence.factors` + `posteriorRaw`） | 必红 |
| `evidence_audit.rs:1171-1419` | 一批夹具与断言（closure / reconcile / basis source 分支） | 必红 |
| `crates/rt-workflow/tests/portfolio_mgr_horizon_lock_rhai.rs` | 四路全缺席时 `effective_posterior` 兜底定档 | 判据源消失 |
| `crates/analysis-engine/src/portfolio_formula.rs:2266` | `score_to_signal(78.0)==0.56` 锚点绑定主链 f1 σ 口径 | 语义绑定（常量锚，不读生产键） |

### 13.4 D 类：前端真正在读的那几格

前端 `normalizeDecision`（`src/lib/agentOutput.ts:433-514`）是**白名单式重建**，
收着的只有：`weightsCollapsed` / `collapseReason` / `weightRatio` / `untrustedCount` /
`decisionConfidence` / `signalStrength` / `f7Source` / `f7ReferenceFormula` / `dataGaps` /
`actionSource` / `confidenceSource` / `horizonSource` / `actionConfidenceBasis` /
`valuationReconciliation` / `crossCheck`。

| 消费点 | 读的键 | 键缺失时的实际表现 | 严重度 |
|---|---|---|---|
| `DecisionTrustNotice.tsx:90 / 115-117` | `weightsCollapsed` / `dataGaps` / `dqiFuseState` | 三者全空 ⇒ 整块组件 `return null` ⇒ 「本次观望为数据不足导致的被动降级，非看空判断」这句话永不再现 —— 这正是该组件的立论理由（`:9-18` 记录的黑盒观望体感） | 高 |
| `DecisionBanner.tsx:714-722` | `decisionConfidence` | null ⇒ 「决策置信度」数字块整体不渲染 | 中 |
| `F7ReferenceComparison.tsx:44-68` | `f7Source` / `weightsCollapsed` / `posterior` / `f7WeightPct` | 整块降级为「本记录未产出 f7 口径对照」一句 | 中 |
| `EvidenceCitationPanel.tsx:272,438,471-494` | 后端 `audit_decision_evidence`（读 `evidence.factors.fN.{sigma,weight}`） | A 面「主档因子闭合对账」整块不渲染 | 高 |
| `WhatIfBacktest.tsx:300-381` | 原始 JSON 的 `decision_trail` / `technical_veto` / `simulation_gate`（**绕过白名单直读**） | 「决策追溯链」整块不渲染 | 中 |
| `scripts/lib/stock-analysis-data.mjs:106,114,116` | `f7_source` / `f7Source` | 基准/质量报表「生产侧 f7 口径」为空 ⇒ 对照匹配率退化 | 中 |

**关键一条**：主链顶层输出里的 `posterior` / `posteriorRaw` / `evidence`（含 f1..f13 的
name/σ/w）/ `risk_quant` / `f7_free` / `computation_logs` / `effective_params` /
`decision_trail`，**本来就没有一格进前端白名单** ⇒ 退役这批键对界面几乎零影响；
界面真正依赖的是上表那几格。

### 13.5 覆盖面：12 条主链腿 vs 四档腿

主链是 12 条腿（f1..f7 + f9..f13，无 f8）。逐档腿集权威源 = `crates/harness/src/holding_period.rs:556-666`
（`Period::verdict_spec()`），另用四份档脚本的 `present()` 全集实测核对。

| 主链腿 | 四档有没有对应 | 依据 |
|---|---|---|
| f1 trend | 有（短/中=方向腿；长=只作过滤；超短不适用） | `verdict_spec()` |
| f2 consensus（辩论共识分 × 方向） | **没有** | 四档脚本没有共识分；中/长读的是 `consensus_eps` / `latest_eps`（预期修正），不是辩论共识 |
| f3 catalyst | 有（超短=eventCatalyst；各档另有公告关键词修正） | |
| f4 risk | **口径不同**：主链是「风险档 → 后验概率偏置」，逐档是「本档风险 → 动作否决」+ 独立风控节点 | `portfolio-mgr.rhai:696-706` vs 各档 `pm_risk_veto` |
| f5 valuation | 有（中=方向腿；长=只作严重高估否决） | |
| f6 data_quality | **没有**：逐档没有 DQI 腿，也没有权重坍缩；只有逐腿 `absentLegs` / `dataGaps` 声明 | 四档脚本无 `dqi_score` |
| f7 trade_signal | **没有**（v183 已换源到分析师票，但票只进主链 f7） | |
| f9 money_flow | 有（短/中=flowPersistence 方向腿） | |
| f10 chip | **只覆盖三分之一**：主链 f10 = 增减持 0.6 + 解禁 0.25 + 大宗折溢价 0.15；逐档 supplyShock 只吃解禁占比 | `portfolio-mgr.rhai:1548-1617` vs 档脚本只读 `lockup_float_ratio` |
| f11 pace（公告四维情绪） | **没有** | 四档无 PACE；超短的 `catalyst_level` 是催化等级，不是 PACE |
| f12 momentum | 有（超短/短=momentumSignal） | |
| f13 bottleneck（serenity_score） | **没有** | |

逐档有、主链没有的腿：relativeStrength（短/中）、breadthState（超短/短，只作入场门）、
microstructure（超短，部分来源）、macroRegime（长）、sectorRotation（长，条件）、
earningsQuality（中/长）、expectationRevision（中/长）、supplyShock（短/中，风险提示）。

⇒ 12 条主链腿里：**5 条在四档完全无对应**（f2 / f6 / f7 / f11 / f13）、
**1 条只覆盖三分之一**（f10）、**1 条口径不同**（f4）；其余 5 条有对应且逐档相当或更强。

### 13.6 结论：乙不是「删一段代码」

1. **可执行结论层**：四档分支在场时，主链因子模型**不产出**结论（action / positionPct /
   confidence / 档位全取自所选档行）⇒ 退役它不改任何一次分析的买卖结论。
   **但四路全缺席时它是唯一结论源**（A4）⇒ 退役前必须先回答「逐档节点失败时给什么结论」。
2. **真正会掉的是四块**：① What-If 面板（重跑对象没了）② 证据审计面板 A 面（σ/w 对账）
   ③ 决策可信度提示（`weightsCollapsed` 一族，「被动降级 vs 主动看空」的区分）
   ④ 反思侧的后验置信度归因（`portfolio_posterior` 读空被写成「非常低」）。
3. **覆盖代价**：5 条腿无对应 + 1 条只覆盖三分之一 + 1 条口径不同 ⇒ 乙方案要么把这些信息
   迁进逐档（要动 `verdict_spec` 的因子集与契约，改的是四档分析师的输出要求），
   要么逐条显式声明退役。**不能靠「反正四档行才是结论」把这件事带过** ——
   那是把「拿不到」伪装成「不需要」。

### 13.7 实现层陷阱（无论选甲选乙都要记）

`decision.rs:5559` 的门是「`f7_source` **在场**且 ≠ trader ⇒ 不合并 LLM 价位」。
键**消失**时 `is_some_and` 对 None 为假 ⇒ 门反向放行 ⇒ 公式侧价位被 LLM 覆盖 ⇒ 假独立重现。
⇒ 退役或改名时必须把门改成「无该键 ⇒ 也不合并」，而不是只改值域。

---

## 十四、裁定落地：主档 = 纯兜底，且**不得依赖 LLM 决策**

> 裁定原文（2026-10-25）：「**可以保留主档作为逐档节点失败时的兜底，但是主档也不能依赖 LLM 决策的输出**」
>
> 本章是这条例定的决策档案。§十三（乙方案普查）结论是「退役主链因子模型 ⇒ 六条门禁必须同批重写」；
> 本裁定**不走退役路线** ⇒ §13.3 的六条退役全部免除（见 §14.6），代价比普查时预估的小得多。

### 14.0 裁定拆成三条可判定的判据

| 判据 | 内容 | 可判定形式 |
|---|---|---|
| **D1** | 主档**只在四路逐档分支全缺席时**才产出结论；其余时候它只提供上限与证据 | `horizon_source` / `confidence_source` / `position_source` 三处来源标签 |
| **D2** | 主档的**输入**不得包含 LLM 决策（trader 的方向/置信/报价/风险档），**不论开关怎么设** | 建图期输入映射 + 供给边 |
| **D3** | 主档的**输出**不得借用 LLM 决策的价位（含 Rhai 异常的降级记录） | `merge_price_fields_from_llm` 的合并门 |

D1 是「谁是结论」的判据；D2 是「谁喂它」的判据；D3 是「它会不会沾 LLM」的判据。
三条**互不替代**：D2 成立不代表 D3 成立（价位可以在显示期被嫁接进来，见 §14.4）。

### 14.1 现状复核（四条读数，全部实测）

**读数①：生产侧其实已经不吃 trader 了 —— 但那是「开关默认」给的，不是结构给的。**

- `seed_stock_analysis.rs:4147-4152` 读 `resolve_bool_flag(old_variables, "formula_independent_f7", true)`；
  默认 `true` ⇒ 生产 `portfolio-mgr` 被 `drop_trader_view` 摘掉全部 8 个 `TRADER_F7_KEYS`
  （:6507-6568），f7 改读 `analyst_votes_by_tier`（:6242）。
- 两条供给边（`trader → f7_view_target` / `research-mgr → f7_view_target`，:6566-6568）
  随开关一起移动 ⇒ 默认形态下 trader 只喂 `portfolio-mgr-f7-ref`（参照侧）。
- 该开关**不在** `seed_variables.rs` 里 ⇒ 面板不可设；全仓仅 4 处引用（1 处读点 + 3 处前端注释）。
  但它读的是 `old_variables`（= 库里 `workflow_template.variables` 原样）⇒ **手改库就能打开**。

**读数②：建图断言已经无条件钉住「生产侧零 trader 键」，但它测不到回滚位。**

- `mod.rs:3912-3982` 的断言（生产 pm 不含 8 个 trader 键 / trader 边只指向参照侧 / 风控门无 trader 边）
  **无条件**执行 ⇒ 若库里真把开关设成 false，seed 产出的形态与这条断言直接矛盾。
- 而该断言跑在 `fresh_db()` 上（`old_variables` 为空 ⇒ 恒取默认 true）⇒ **回滚位在测试里永远测不到**。
  ⇒ 现状是「开关能改形态、断言却只认一种形态」的**两套真相**。

**读数③：主链内部对 trader 的耦合全部是 `present()` 守卫，缺席是安全的、且有留痕。**

- `trader_data_valid` 在 trader 键缺席时仍为 `true`（`portfolio-mgr.rhai:1906`）；
- `odds_from_trader` 恒 false ⇒ 赔率改走 `odds_from_valuation` / `odds_fallback`（:2238-2242）；
- `posterior_cap`（f7 权重封顶）**已按 `f7_source == "trader"` 门控**（:2100-2131）⇒ 非 trader 口径不吃这条封顶；
- 缺键时脚本**显式留痕** R-213（:2009-2016）：「本侧未注入 trader 决策输入 ⇒ R-202/R-203/R-204 与 R-201 均不适用」。

⇒ D2 的**实现层**已经就位；缺的是「谁来保证 trader 键永远不被摘回来」。

**读数④：决策三字段不含 LLM 决策 —— 但「价位」会在显示期被嫁接进来。**

- 决策链尾唯一实现 `pick_chain_tail_decision`（`decision.rs:926-1033`）：
  `quality-fallback` > `portfolio-risk-gate` > `portfolio-mgr`；
  落库与最终输出都取 `portfolio-risk-gate`（`seed_stock_analysis.rs:7620-7644`）⇒
  **action / positionPct / confidence 三字段出自公式链，不含 trader**。
- `rule-check` / `decision-explainer` 的 `context_sources` 里**有** trader
  （:7196-7202 / :7408-7413）⇒ 它们看得见 LLM 交易方案，但**只产解释文本与追溯码**，
  不回写决策字段（`force_watch_tag` 只出现在分歧提示串里，`core.rs:1401-1412`）。
  ⇒ 记为**解释层耦合**，不改（见 §14.7）。
- **真正的漏口在显示期**：`merge_price_fields_from_llm`（`decision.rs:5549-5590`）
  的门是「`f7_source` 存在且 ≠ `trader` ⇒ 不借」⇒ **缺键 ⇒ 借**。
  而**新格式里有三条降级路径根本产不出 `f7_source`**（见 §14.4 表）⇒
  一条从未被任何公式产出的价位，会挂到「Rhai 异常」「pm 无输出」「风控门异常」这三条记录上。
  该函数自己的文档注释（:2543-2547）已把这一条列为「**已知残留**」。

### 14.2 D1：主档的兜底身份 —— **已有，无需新增字段**

三处来源标签已在生产侧产出、且前端已在渲染：

| 标签 | 产端 | 值域 | 渲染端 |
|---|---|---|---|
| `horizon_source` | `portfolio-mgr.rhai:2340` | `branch_pick` / `formula_no_branch` | `DecisionBanner.tsx:763,873-875,1054`；`AnalysisHistoryButton.tsx:760`；`HistoricalAnalysisPanel.tsx:568` |
| `confidence_source` | `portfolio-mgr.rhai:2683` | `branch_row` / `main_chain_posterior` | `DecisionBanner.tsx:795-796`；`AnalysisHistoryButton.tsx:816` |
| `position_source` | `portfolio-mgr.rhai:2504-2506` | `branch_position` / `branch_position_capped` / `risk_budget` / `fallback_kelly_x_mult` | `AnalystReportCard.tsx:428` |

标签键在 `src/lib/stock-analysis-utils.ts:1874 / 1905 / 1937`，白名单在 `src/lib/agentOutput.ts:468-472`。

- `formula_no_branch` / `main_chain_posterior` / `risk_budget` / `fallback_kelly_x_mult`
  **就是「兜底」这个身份的显式表达** ⇒ D1 已满足，**不新增字段、不新增 i18n 键**。
- ⚠ **但要记下一条容易被误读的事实**：四档齐全时，主链的 12 条腿**仍在算**
  （`posterior` / `evidence.factors` / `f7_free` 都照常产出，并喂 `compute_horizon_agreements`）。
  它们此时**不产出结论**（结论取所选档分支行），但**产出证据**。
  ⇒ 「主档=纯兜底」在**结论层**成立，在**证据层**不成立 —— 这是既有设计（主链是「档位无关的证据层」），
  不是本裁定要改的东西。**改它会动到 `evidence_audit` / 反思链 / 证据支撑率四条链** ⇒ 不在本轮范围。

### 14.3 D2：把独立性从「开关」改成「结构」

**修法：退役 `formula_independent_f7` 的回滚能力，改为恒定形态。**

- 生产 `portfolio-mgr`：**恒**丢弃 8 个 `TRADER_F7_KEYS`；
- `portfolio-mgr-f7-ref`：**恒**保留 trader 口径 + **恒**接 `trader` / `research-mgr` 两条边；
- 开关读点删除（`seed_stock_analysis.rs:4147-4152`）；
- 变量表里那个键**不会残留**（`merge_variable_values` 只保留「新表里存在的名字」，`mod.rs:1414-1427`
  ⇒ 旧库里的 `formula_independent_f7` 随本次播种被丢弃）。

**为什么不再保留回滚位（三条理由）**：

1. 这个开关的**唯一作用**就是把 LLM 决策接回生产公式 ⇒ 与裁定直接冲突，不是「别的用途」。
2. 回滚位**本来就不可靠**：`mod.rs:3912-3982` 的断言无条件钉住「生产侧零 trader 键」，
   打开开关后的形态与断言矛盾 ⇒ 现状是「开关能改形态、门禁只认一种形态」的两套真相。
3. **不需要它兜底**：f7 的缺席态已有正确语义 —— `f7_source = "absent"` ⇒ σ=0 ⇒ 该腿退出权重
   （`portfolio-mgr.rhai:1473-1479` 的 `f7_weight = if f7_signal != 0.0 { f7_default } else { 0.0 }`），
   公式结论仍然成立，只是少一条腿。拿 trader 补 f7 等于用「有观点」冒充「有证据」。

**副作用（一并记下）**：生产主链与 trader 之间**不再有依赖边** ⇒ trader 失败/超时不再连坐主链；
同时 `untrusted_trader` / `untrusted_research_mgr` 两个哨兵也不再进入生产主链
⇒ 主链的 `untrusted_count` 只由 debate_conv / data_quality / risk_conv 三条决定
（`portfolio-mgr.rhai:205-210`）⇒ **trader 的 strict_mode 降级不再触发主链的 `weights_collapsed`**。
这是正确的：主链不消费 trader，trader 降级就不该压塌主链的权重。

### 14.4 D3：封死「借 LLM 价位」

**漏口全表（三条降级路径都产不出 `f7_source` ⇒ 都会被嫁接 LLM 价位）**：

| # | 产出点 | 产出内容 | 现状 |
|---|---|---|---|
| ① | `portfolio-mgr.rhai:4091-4141`（catch 块） | `action="数据缺失"`、价位为显式 null | **无 `f7_source` 键** |
| ② | `portfolio-risk-gate.rhai:141-158`（pm 无输出降级） | `action="观望"` | **无 `f7_source` 键** |
| ③ | `portfolio-risk-gate.rhai:312-330`（门异常且 pm 无输出） | `action="观望"` | **无 `f7_source` 键** |

⇒ 修法两件：

**修法①（承重）：合并门改白名单。**

```
旧门：f7_source 存在 且 ≠ "trader" ⇒ 不借；缺键 ⇒ 借
新门：f7_source == "trader" ⇒ 借；其余（含缺键）⇒ 不借
```

- 改白名单后，**缺键不再意味着「借」**，而意味着「不知道 ⇒ 不借」；
  ⇒ 漏产键的后果从「泄漏 LLM 价位」降级为「UI 少一格口径自述」⇒ **不再依赖「每个产出点都记得补键」**。
- **代价（明示）**：v183 之前产出的**存量记录**重开时，`targetPrice` / `stopLoss` 两格不再有 LLM 补值。
  - 这两格的值本来就出自 LLM 决策 ⇒ 按 D3 挂在公式决策上就是「伪装成本公式的结论」。
  - 影响面**只有展示**：落库与反思读的一直是**未合并**的原值（合并只发生在
    `decision.rs:5199 / 5423 / 5633` 三处 dashboard/复盘展示路径）。
- 参照侧不受影响：`extract_f7_reference_formula_json`（`decision.rs:897-906`）不经过该函数；
  且参照侧口径本就是 trader，其价位由它自己的公式腿给出。

**修法②（配套）：三处补 `f7_source`，让「本条读没读 LLM 决策」在 UI 上可自述。**

- 三条降级路径统一补 `"f7_source": "absent"`（值域 = `trader` / `analyst_votes` / `absent`，
  与主链成功路径同域，见 `portfolio-mgr.rhai:1466-1479`）。
- 语义：「这条记录不是 trader 口径的结论」。降级路径本就无交易计划，口径自述必须与之一致。

### 14.5 覆盖面：5 条「四档无对应」的腿 —— **保留，不需要迁移**

§13.5 点名 f2 / f6 / f7 / f11 / f13 在四档腿集里没有对应、f10 只覆盖三分之一、f4 口径不同。
**在本裁定下这不再是缺口**：

- 裁定保留主链作为**兜底路径**，兜底路径的腿集**就是主链自己的 12 条腿** —— 它不需要是四档腿的子集。
- 逐档腿集（`Period::verdict_spec()`，`crates/harness/src/holding_period.rs:556-666`）与
  主链腿集是**两套不同粒度的因子空间**：前者是「VERDICT 契约因子」（momentum / flowPersistence / …），
  后者是「信号级因子」（f1 trend / f9 money_flow / …）。f2（辩论共识分）、f6（数据质量）、
  f11（PACE 情绪）、f13（bottleneck）都是**信号级**，四档契约层没有对应物是**口径不同**，不是「拿不到」。
- 关键：f2 / f6 / f11 / f13 **都不读 trader**；f7 在兜底路径读的是**分析师结构票**（`analyst_votes_by_tier`），
  也不是 LLM 决策 ⇒ **没有一条腿构成 D2 的违反**。
- ⇒ 结论：**不迁移、不退役**，但要在兜底路径上保证这 5 条腿的缺席语义是显式的
  （`f6` 的 DQI 缺席 ⇒ 权重退出；`f11` 的 `pace_degraded` ⇒ 权重归零，`portfolio-mgr.rhai:1680-1690`）。

⚠ **不得用「反正四档行才是结论」把这件事带过** —— 那是把「拿不到」伪装成「不需要」。
本轮的正确表述是：**这些腿服务的是兜底路径，兜底路径要它们**（所以保留），
**而不是四档路径不需要它们**（所以不用迁移）。

### 14.6 门禁与测试：同批处置（§13.3 的六条退役全部免除）

| 原 §13.3 的处置 | 本裁定下的实际处置 |
|---|---|
| `evidence_audit.rs:1462-1470`（主链源码字面量） | **不退役**（主链保留） |
| `rhai_registry.rs:2210-2240`（R-F12-OVERBOUGHT 追溯） | **不退役** |
| `hooks.rs:1342-1490`（归因单测夹具） | **不退役** |
| `evidence_audit.rs:1171-1419`（夹具与断言） | **不退役** |
| `crates/rt-workflow/tests/portfolio_mgr_horizon_lock_rhai.rs`（四路全缺席兜底定档） | **不退役，且语义升级**：它测的正是「兜底路径」 |
| `crates/analysis-engine/src/portfolio_formula.rs:2266`（f1 σ 锚点） | **不退役** |

本轮**新增/改写**的门禁：

1. **合并门单测**（`decision.rs:2733-2749`）—— 判据改期望：
   ① `f7_source="analyst_votes"` ⇒ 不借（不变）；
   ② `f7_source="trader"` ⇒ 借（不变）；
   ③ **缺键 ⇒ 不借**（旧期望是「借」，**本裁定推翻**），
   并补 ④「`action="数据缺失"` 的降级记录（无键）⇒ 不借」。
2. **回滚位负控**（`mod.rs` 建图断言测试）—— 把库里 `variables` 写成
   `{"formula_independent_f7": false}` 再播种，断言**生产 pm 仍不含任何 trader 键**
   （本批改造前 ⇒ 红；改造后 ⇒ 绿）。**这条判据在改代码前是红的**（可自证）。
3. **降级路径口径自述**：三处补 `f7_source` 后，在既有 rhai 源码扫描门
   （`evidence_audit.rs:1462-1470` 那一族）加一条计数下限判据
   （`portfolio-mgr.rhai` 与 `portfolio-risk-gate.rhai` 的 `f7_source` 出现次数 ≥ 预期），
   防止「新增一条产出路径忘补口径」。

### 14.7 退役项清单（本批唯一退役的东西）

| 退役项 | 位置 | 退役理由 |
|---|---|---|
| `formula_independent_f7` 回滚位 | `seed_stock_analysis.rs:4147-4152` | 唯一作用是把 LLM 决策接回生产公式 ⇒ 与裁定冲突 |
| 「缺键 ⇒ 借 LLM 价位」 | `decision.rs:5561-5563` | 缺键含「新格式漏产键」一类 ⇒ 借即泄漏 |
| 前端「回滚位」注释 | `src/types/stock-analysis.ts:538`、`F7ReferenceComparison.tsx:16` | 描述的是已不存在的机制 |

**有意保留（记为「解释层耦合」，不改）**：

- `rule-check` / `decision-explainer` 的 `context_sources` 含 `trader`
  （`seed_stock_analysis.rs:7196-7202 / 7408-7413`）—— 它们只产解释文本与追溯码，
  不回写 action / positionPct / confidence（决策链尾 = `portfolio-risk-gate`）。
  ⚠ 代价是「规则检查讲的是 LLM 的方案、结论出自公式」这一**口径错位**仍存在 ⇒
  列为**待裁**（见 §14.9），不在本裁定范围内擅改。

### 14.8 实施顺序（单批，攒够一次编译）

1. `seed_stock_analysis.rs`：删读点 + 恒定形态 + `TEMPLATE_VERSION` 184 → 185；
2. `portfolio-mgr.rhai` catch 块 + `portfolio-risk-gate.rhai` 两处降级 map 补 `f7_source`；
3. `decision.rs`：合并门改白名单 + 单测改期望；
4. `mod.rs`：建图断言加回滚位负控；
5. 定点门禁：`cargo clippy -p axagent --all-targets` / `cargo test -p axagent --lib commands::stock_workflow` /
   `cargo test -p axagent-rt-workflow --test rhai_syntax_check` / 前端 vitest；
6. 全量门禁留到本批全部收尾时按 CI 口径跑一次。

### 14.9 待裁（本批不做，需用户拍板）

- **Q1**：「规则检查讲 LLM 方案、结论出自公式」的口径错位 —— 是否把 `trader` 从
  `rule-check` / `decision-explainer` 的 `context_sources` 里摘掉？
  （摘掉的代价：规则检查失去「交易方案」这一路输入，只能查公式方案）
- **Q2**：主链 12 条腿在四档齐全时仍产出证据（§14.2）—— 是否把它降级为「仅四档缺席时才算」？
  （会牵动 `evidence_audit` / 反思链 / 证据支撑率四条链 ⇒ 需单独一轮）
