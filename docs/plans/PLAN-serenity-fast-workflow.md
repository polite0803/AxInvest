# PLAN — 趋势智选「快速趋势智选」（Jev 判定链路）

**日期**：2026-09-24
**状态**：**已批准（2026-09-24）**，待开工；§8 三项均按**推荐项**执行（`① 保留 1 个合并生成 Agent` / `search_stock` 优先 / `① 两链并存`）
**目的**：在「趋势智选」面板旁新增「快速趋势智选」，调一条以 Jev 判定为核心的快速工作流；**复用现有趋势智选的算法 / 工具 / Rhai 脚本 / 落库通路**，只用 Jev 承接原本由 Agent 承担的**判定**职责，从而消除「运行时间过长」。

## 0. 与既有方案的关系（必读）

上游文档：`PLAN-stock-analysis-fast-workflow.md`（2026-09-24 已批准并进入实施）、`PLAN-jev-integration.md`、`AUDIT-jev-adoption-stock-workflow-2026-09-20.md`。

本方案是 `PLAN-stock-analysis-fast-workflow.md` 在**趋势智选**上的同构落地 —— 同一套硬约束、同一套「复用非 Agent 资产 + Jev 承接判定 + 保留一个生成节点承担叙述」的形态。两处差异（必须单独处理，见 §3、§7）：

| # | 差异 | 后果 |
|---|---|---|
| D1 | 股票分析的落库提取按**节点 id 三级取值**（`quality-fallback` > `portfolio-risk-gate` > `portfolio-mgr`），而趋势智选按 id 两级取值（`c-data-verifier` > `a-candidate-mapper`） | 快速链若要「落库零改动」，必须让这两个 id 在快速链里**存在且契约同形** |
| D2 | 股票分析的前端解析集中在一层（`src/lib/agentOutput.ts`），趋势智选的前端有**节点 id → 阶段**的硬映射（`SerenityScreeningPanel.tsx:261-294`） | 快速链新增节点 id 时，面板的阶段映射要同步补 |

### 0.1 既有事实（已核实，含出处）

**模板与播种**

| 项 | 事实 | 出处 |
|---|---|---|
| 模板 id | `serenity-screening` | `src-tauri/src/commands/stock_analysis_setup/seed_serenity.rs:20` |
| 版本门 | `TEMPLATE_VERSION: i32 = 53`；`existing.version >= TEMPLATE_VERSION` 即跳过重建 | 同文件 `:92`、`:1411` |
| seed 调用点 | 在 stock 分析 setup 流程里被调用一次 | `src-tauri/src/commands/stock_analysis_setup/mod.rs:771` |
| 规模 | 19 节点（Trigger 1 / Tool 4 / Code 7 / Agent 7） | 见 §2 清单 |

**7 个 Agent 节点（这是「慢」的根源）**

| id | expert | 工具往返上限 | timeout | 说明 |
|---|---|---|---|---|
| `a-trend-scanner` | `stock-trend-scanner` | 8 轮 | 600s（chunk 300s） | 识别 2-3 个产业方向 + 标 `strategy_type` 六选一 |
| `a-chain-trend1`…`trend5` | `stock-chain-decomposer` | 8 轮 | 600s（chunk 300s） | 5 路**并行**产业链拆解，每路按需调 `search_stock` / `get_stock_financials` / `compute_industry_position` |
| `a-candidate-mapper` | `stock-candidate-mapper` | 8 轮 | 600s（chunk 300s） | 候选筛选打分，prompt 硬要求「至少尝试 3 轮不同关键词搜索」 |

> 关键路径 = 1 个扫描 Agent → 5 路并行拆解 Agent（各自多轮工具往返）→ 1 个 mapper Agent（多轮搜索 + 财务验证）。**LLM 往返次数**（而非并发度）决定总时长；5 路拆解即便并行，各自 8 轮工具往返也会把尾延迟拖长。
> 关闭幻觉守卫、`model: None`（回落会话/默认模型）见 `seed_serenity.rs:563`、`:575-578`。

**落库与前端（复用面）**

| 项 | 事实 | 出处 |
|---|---|---|
| 命令 | `run_serenity_screening(as_of_date, themes, run_id)`，**模板 id 硬编码** | `src-tauri/src/commands/stock_workflow/serenity.rs:868`、`:889` |
| 工作流实例名 | `serenity-screening-{timestamp_ms}` | 同文件 `:927` |
| 候选提取优先级 | ① `c-data-verifier` → ② `a-candidate-mapper`（后者为空则深搜/文本兜底） | 同文件 `:1016-1183` |
| `emptyReason` | 取自 `a-candidate-mapper.arguments.summary` | 同文件 `:1185-1237` |
| 落库 | `reco_picks`，`style="serenity"`；`pick_data` 为 camelCase JSON；落库前抓实时行情并按 `entry=±5% / stop=0.80 / target=1.30` 计算价位 | 同文件 `:1239-1387`（价位于 `:1302-1310`） |
| 事件 | `serenity-screening-step` / `serenity-screening-completed`，payload 含 `candidates` / `trends` / `emptyReason` / `status` / `source` | 同文件 `:979`、`:1408-1420` |
| 后续链路 | `refresh_serenity_exit_signals` / `refresh_serenity_feedback` 按 `style="serenity"` 回读同一张表 | 同文件 `:1479`、`:1581` |
| 前端 | 事件监听 `:323-440`、invoke `:919-943`、节点 id→阶段映射 `:261-294` | `src/components/stock-analysis/SerenityScreeningPanel.tsx` |

**Jev（判定模型）能力边界**

- 适配器：`src-tauri/crates/providers/src/typesafe.rs`；端点 `POST {base}/alpha/decisions`，默认 `typesafe/jev-1.13`，端到端 70–500ms，**输出免费**。
- 只做 `choice`（从 criteria 里选 1）与 `noul`（是非）；state 为纯文本，**context 仅 32k**（`:72`）。
- **不做文本生成、不做算术 / 计数 / 日期比较、无视觉、无流式**（`:40-45`）。
- 消费节点：`llmClassifier` 的 `config.model` / `condition` 的 `config.routing_model`，写 `providerId::modelId` 复合值；生成类节点（`llm` / `agent`）会被 `ensure_generation_model` 硬拒（`src-tauri/crates/rt-workflow/src/work_engine/executors/llm_resolve.rs:86-102`）。
- 节点类型已存在：`WorkflowNode::LlmClassifier`（`src-tauri/crates/harness/src/workflow_types.rs:1211`，config `:1180`：`categories` / `categories_var` / `prompt` / `model` / `input_var` / `output_var` / `confidence_threshold` / `fallback_label`）、`WorkflowNode::Condition`（`:479`，config `:459`：`conditions` / `logical_op` / `judge_by_llm` / `routing_prompt` / `routing_model` / `confidence_threshold`）。

### 0.2 三条硬约束（沿用上游，2026-09-24 用户明确）

| # | 约束 | 本方案落实 |
|---|---|---|
| **H1** | 不得修改原趋势智选工作流的图（节点 / 边 / 配置） | 新增独立模板 `serenity-screening-fast`；`serenity-screening` 的 seed 节点图与边**一行不动** |
| **H2** | 两链脚本必须一致：脚本可改，但**禁止 `*-fast` 副本、禁止版本分支**；改动必须同时服务两条链 | `strategy-scorer.rhai` / `consistency-check.rhai` / `data-verifier.rhai` 两链按**完全相同的变量契约**调用；差异只允许在**节点配置层**（`input_mapping` 指向哪个节点）。快速链新增的脚本是**新资产**（原链本无对应脚本），不构成同源双版本 |
| **H3** | 落库与原链**一致**；前端**复用现有前端**；新链路执行时卡片**不显示叙述文本**，但必须**正确解析出候选与结论** | 见 §4、§5 |

---

## 1. 入口与启动路径

| 项 | 设计 | 依据 |
|---|---|---|
| 按钮位置 | `src/components/stock-analysis/SerenityScreeningPanel.tsx:919-943` 现有「开始筛选」旁的「快速趋势智选」，同款 `isRunning` 联动 | 现有主按钮即此处的 `invoke("run_serenity_screening", ...)` |
| 前端动作 | 面板新增快速启动分支，参数复用现有 `{asOfDate, themes, runId}`，多传 `templateId: "serenity-screening-fast"` | 对齐 `stockAnalysisStore.ts` 的 `startFastAnalysis` 先例 |
| 后端最小改动 | `run_serenity_screening` 增加可选参数 `template_id`（缺省 `"serenity-screening"`），把 `serenity.rs:889` 的硬编码改为取值；`serenity.rs:927` 的实例名前缀随之带上模板标识 | 对齐 `src-tauri/src/commands/stock_workflow/core.rs:625-628` 的既有做法（**不参数化则两条工作流进不去**） |
| 落库 | 完全复用：`reco_picks` / `style="serenity"` / `pick_data` 结构 / 价位计算 / `set_serenity_seed` + `set_serenity_candidate_cache` 全部不动 | §4 |
| i18n | 11 种语言补「快速趋势智选」按钮文案 + Tooltip（键名沿用面板现有前缀） | 项目规范：新增 UI 文本必须 11 语言齐全 |

---

## 2. 现有节点清单（改造对照基准）

> 本章是**事实盘点**，用于对照「快速链用什么承接了同一职能」。原链节点一个都不动（H1）。

| id | 类型 | 关键配置 |
|---|---|---|
| `trigger` | Trigger(Manual) | 无参自驱动 |
| `t-industry-rank` | Tool `get_industry_ranking` | 行业涨跌排名（含主力资金流入） |
| `t-cls-flash` | Tool `get_cls_flash` | 财联社实时快讯 |
| `t-northbound` | Tool `get_north_bound_flow` | 北向成交活跃度 |
| `t-policy-news` | Tool `search_news` | **`input_mapping.keyword → policy_news_keywords`（模板变量）**，`continue_on_fail: true` |
| `a-trend-scanner` | Agent `stock-trend-scanner` | `input_mapping`: `industry_ranking←t-industry-rank.result`、`policy_news←t-policy-news.result`、`user_themes←user_themes`；`context_sources` = 上述 4 个 Tool；输出 `tool_json` 块 `{"name":"submit_trends","arguments":{trends:[{trend_name,confidence,phase,core_logic,causal_chain,strategy_type,bottleneck_candidate,bottleneck_rationale,demand_evidence,downstream_giants}],summary}}` |
| `a-chain-trend1`…`5` | Agent `stock-chain-decomposer` | `input_mapping.trend_strategy ← a-trend-scanner.content.trends[i].strategy_type`；`context_sources=[a-trend-scanner]`；输出按策略分 6 种形态，bottleneck 形态为 `{"name":"submit_chain","arguments":{trend_name,chain_nodes:[{node_name,global_supplier_count,tech_barrier,expansion_cycle_months,bottleneck_potential,bottleneck_rationale,financial_data,demand_validation}]}}` |
| `c-scorer-trend1`…`5` | Code `strategy-scorer.rhai` | `input_mapping`: `chain_analysis←a-chain-{tn}.content`、`industry_ranking←t-industry-rank.result`、`trend_strategy←a-trend-scanner.content.trends[i].strategy_type`、`w_supply/w_demand/w_irreplace←模板变量`；输出 `result.computed_nodes`（含 `bottleneck_composite` / `supply_rigidity_score` / `demand_elasticity_score` / `irreplaceability_score` / `data_reliability`） |
| `c-consistency-check` | Code `consistency-check.rhai` | 显式枚举 **5 个槽位**：`chain_node_{tn}←a-chain-{tn}.content`、`strategy_{tn}←c-scorer-{tn}.result` |
| `a-candidate-mapper` | Agent `stock-candidate-mapper` | `input_mapping.strategy_{tn}←c-scorer-{tn}.result`；`context_sources` = `a-trend-scanner` + 5×`a-chain-*` + 5×`c-scorer-*` + `c-consistency-check`；输出 `tool_json` 块 `{"name":"submit_candidates","arguments":{candidates:[…],summary}}` |
| `c-data-verifier` | Code `data-verifier.rhai` | `input_mapping`: `candidates←a-candidate-mapper.content.arguments.candidates`、`candidates_direct←a-candidate-mapper.content.candidates`、`candidates_raw←a-candidate-mapper.content`；**只做校验/过滤/标注 `data_verification`，不拉取外部数据**（脚本头注释 `data-verifier.rhai:1-10`） |

**手工补的边（非直观，快速链需同样处理）**

- `t-industry-rank → c-scorer-{tn}` 与 `a-trend-scanner → c-scorer-{tn}`：Code 节点只注入 **edges 直接上游**输出（`get_node_dependency_results`），不吃 Agent 专属的 `context_sources` —— 不补边则 `industry_ranking` / `trend_strategy` resolve 为 null（`seed_serenity.rs:913-929` 的 v42 修复）。
- 扇出结构：`trigger → 4 个数据 Tool`；`4 个 Tool → a-trend-scanner`；`a-trend-scanner → 5 路 a-chain-*`；每路 `a-chain-* → c-scorer-*`；`5×c-scorer-* + 5×a-chain-* → c-consistency-check`；`c-consistency-check + 5×c-scorer-* + a-trend-scanner + 5×a-chain-* → a-candidate-mapper`；`a-candidate-mapper → c-data-verifier`。

**模板变量**：`policy_news_keywords`、`user_themes`、`w_supply` / `w_demand` / `w_irreplace`（默认 0.35 / 0.35 / 0.30）、以及 `serenity_*` 系列过滤参数（见 `seed_serenity.rs:1259-1389`）。

---

## 3. 快速链逐节点清单（推荐形态）

> **口径声明**：本章所有节点都是 `serenity-screening-fast` 这张**新图里新建**的。原图一行不动（H1）。凡出现的原链节点名仅作职能对照。
>
> **id 策略（2026-09-24 实施期定稿，已实测校正）**：**凡被 `serenity.rs` 按 id 直接读取的节点，快速链一律沿用原链同名 id** —— 含 `a-trend-scanner`（合并生成腿）与 `a-candidate-mapper`（组装节点，类型由 Agent 变为 **Code/Rhai**）。其余数据 / 算法 / 校验节点同样沿用（`t-*` / `c-scorer-trend1..5` / `c-consistency-check` / `c-data-verifier`）；**仅纯新增节点**用新名 `j-*` / `c-scanner-brief` / `c-candidate-pool`。
>
> 依据（实测）：`serenity.rs` 里三条按 id 硬读的通路 —— ① trends 只读 `a-trend-scanner`（`:1186-1191`）；② 候选首选 `c-data-verifier`、回退 `a-candidate-mapper`（`:1016-1041`）；③ `emptyReason` 取自 `a-candidate-mapper` 输出的 `summary`（`:1232-1237`）。沿用同名 id ⇒ **三条通路全部零改动**，与上游先例「落库三节点 id 必须沿用同名」同构。
>
> ⚠ **一处必须的小改动**（新增 **R10**）：`serenity_extract_from_node` 只认 `raw["content"]` **字符串**（`:723-729`），而 Code 节点输出形如 `{"status":"executed","result":<rhai 返回>,"params":...}`，**没有 `content` 字段**（`code_executor.rs:580-591`）⇒ 回退分支会直接返回 `Null`，`emptyReason` 通路断裂。修法：把 `:1022-1027` 已在用的「Code 输出 → 包成 `{"content": <result 的 JSON 字符串>}`」逻辑**提成辅助函数，同样作用于 `.or_else` 的 `a-candidate-mapper` 分支**（约 5 行）。原链该节点是 Agent（`content` 本就存在）⇒ 对原链**行为零变化**，属纯兼容增强。

### 段 A · 取数（复用现有 ToolNode / 工具定义，零新增节点类型）

| 节点 id | 类型 | 处置 |
|---|---|---|
| `t-industry-rank` / `t-cls-flash` / `t-northbound` / `t-policy-news` | ToolNode | **原样复用**（含 `t-policy-news` 的 `keyword → policy_news_keywords` 变量通路、`continue_on_fail: true`） |
| `trigger` | Trigger | 原样复用 |
| 模板 `variables` / `tool_defs` | — | 同集合复用（生成腿若不再绑定工具，`tool_defs` 仍保留以供两链共用同一定义） |

### 段 B · state 裁剪（新增 Rhai 新资产，供 Jev 消费）

| 节点 id | 类型 | 脚本 | 职责 |
|---|---|---|---|
| `c-scanner-brief` | Code(rhai) | **新增** `scanner-brief.rhai` | 把 `t-industry-rank.result` 裁成「候选方向编号清单（1. 行业名 …）+ 客观指标摘要（涨幅 / 主力资金净流入 / 1 月涨幅）」，并附快讯 / 政策新闻**标题级**摘要 |

**裁剪是硬要求**：Jev `context` 仅 32k、且 `state` 仅占约一半（`typesafe.rs:71-72`），官方亦自陈精度随 state 长度劣化。**不得**把 `industry_ranking` 全量 + 快讯原文直接喂给 Jev。

**纯算术/阈值 judgement 一律留在 Rhai**（Jev 不做算术，`typesafe.rs:43`）：涨幅 2-15% 区间、主力资金净流入为正、1 月涨幅 > 30% 排除、负债率 > 85% 且营收增速 < 15% 排除、近 3 月 > 150% 涨幅排除、`attention_score > 70` 扣分 —— 这些**不写进 Jev 的 criteria 期望它计算**，而是由 Rhai 先算出「过滤后的候选方向清单」再交给 Jev 选。

### 段 C · Jev 判定（新图不建分析师 / 拆解 Agent 的**判定**部分）

| 节点 id | 类型 | 消费 | 判定内容 | 原链职能对照 |
|---|---|---|---|---|
| `j-strategy-type` | LlmClassifier | `c-scanner-brief.result` | 六选一：`bottleneck / policy / earnings / capital / event / technical`（`model` = Jev 复合值；Jev 的 `choice` 另回 `probabilities` 分布，可用于「分布分散 ⇒ 降级/送人工」） | `a-trend-scanner` prompt 内的 `strategy_type` 标注（原链由 LLM 自报 `confidence: 75`，可信度不及分布） |
| `j-bottleneck` | Condition（`judge_by_llm: true`，`routing_model` = Jev） | 段 B 输出 | 是非：所选方向是否满足「供给瓶颈」判据 | 原链 `bottleneck_potential` 的判定部分 |
| `j-candidate-pick` | LlmClassifier | 段 D 的候选池（编号清单） | 从候选中选出 3-5 只（`choice`，可多问一组） | `a-candidate-mapper` 的**取舍**部分 |

> ⚠ **`strategy_type` 单一权威**：原链由 `a-trend-scanner` 产出该字段，若快速链同时保留 Agent 产出 + Jev 判定，即同一语义双写（违反项目「禁止重复定义」铁律）。**推荐 Jev 独占**：快速链的生成腿 prompt **不再输出** `strategy_type`，改由 `j-strategy-type` 注入给 `c-scorer-*` 的 `trend_strategy`。
> ⚠ `confidence_threshold` 若启用，必须按自有语料重标定后再放开（Jev 的 `confidence` ≠ `max(probabilities)`）。

### 段 D · 候选池（Tool 取池 + Rhai 客观预筛）

| 节点 id | 类型 | 说明 |
|---|---|---|
| `t-candidate-search` | ToolNode（`search_stock` 或 `get_stock_concept_blocks`） | 关键词来自 `j-strategy-type` / 段 B 摘要；取真实 A 股候选池（**不新造资产**，工具已在现有 `tool_defs` 中） |
| `c-candidate-pool` | Code(rhai)（**新增** `candidate-pool.rhai`） | 清洗 + 套用**客观阈值**（§段 B 的算术规则）+ 去重，输出编号清单供 `j-candidate-pick` 的 criteria 提取 |

### 段 E · 生成腿（**§8 待拍板的关键项**，推荐选项为 ①）

Jev **无生成能力**，而 `chain_nodes`（供应商数量 / 技术壁垒 / 扩产周期 / 瓶颈潜力）是**知识**、`catalysts` / `exit_signals` / `attention_metrics` 是**叙述**，两者都**不可能**由 Jev 产出。同时 `c-scorer-*`（共享脚本，禁止改契约）**必须**吃到 `chain_analysis.content.chain_nodes`。

| 选项 | 内容 | 代价 |
|---|---|---|
| **①（已拍板，2026-09-24）** | 保留**一个**生成 Agent，**沿用 id `a-trend-scanner`**（合并原 `a-trend-scanner` + 5×`a-chain-trend1..5` 为 1 个节点），一次性输出 `trends[]` 且每项内嵌 `chain_nodes[]`，并承担 `catalysts` / `exit_signals` / `attention_metrics` / `bottleneck_product` / `primary_risk` 等叙述字段 | 保留一次 LLM 生成（无多轮工具往返或极少）；换来下游契约与前端展示不失真；trends 通路零改动 |
| ② | 完全无生成：`chain_nodes` 由固定槽位 Tool（`search_stock` / `get_stock_financials` / `compute_industry_position`）拼出 | `supply_rigidity` / `irreplaceability` 失去知识输入，只剩财务代理指标 ⇒ 与 `strategy-scorer.rhai` 的期望契约不符，评分语义失真 |
| ③ | 不建拆解腿，`c-scorer-*` 只吃行业排名 | 最省时、最失真；**不推荐** |

**若采纳 ①，下游接线方式**（脚本零改动）：
- `c-scorer-{tn}` 的 `chain_analysis ← a-trend-scanner.content.trends[{i}]`（**与原链同节点 id、同路径形态**，只是每项内嵌 `chain_nodes`）；
- `trend_strategy ← j-strategy-type.category`（R3 单一权威）；
- **照抄原链的手工补边**：`t-industry-rank → c-scorer-{tn}`、`a-trend-scanner → c-scorer-{tn}`（Code 节点只注入 edges 直接上游，见 `seed_serenity.rs:920-929`）；
- 生成腿的 prompt 需**同时**满足：`trends[i].chain_nodes` 形态对齐 `a-chain-{tn}` 原输出（供 `strategy-scorer.rhai` 消费）、叙述字段对齐 `a-candidate-mapper` 候选字段（供组装节点消费）。

### 段 F · 候选组装 → 校验 → 落库（复用同 id / 同脚本 / 同契约）

| 节点 id | 类型 | 说明 |
|---|---|---|
| `a-candidate-mapper` | Code(rhai)（**新增** `candidate-assembler.rhai`；**沿用原链同 id，类型由 Agent 变为 Code**） | 把 `j-candidate-pick` 的取舍 + 段 D 的池 + 段 E 的叙述字段组装成**与原 `a-candidate-mapper` 同形**的 `{candidates:[…], summary}`（保证 `stock_code` / `stock_name` / `serenity_score` / `confidence` / `reasons` / `strategy_type` 齐备 —— 这些是落库与 `detail_cache` 直接读取的字段）。**顶层 `summary` 是 `emptyReason` 的唯一来源**（`serenity.rs:1232-1237`），空候选时必须填写 |
| `c-data-verifier` | Code `data-verifier.rhai`（**同 id、同脚本、零改动**） | 只把 `input_mapping.candidates_raw` 指向 `a-candidate-mapper.result`（Code 节点输出在 `result`，**不在 `content`**）；`data-verifier.rhai` 本就对 `candidates_raw` 做字符串解析与围栏剥离兜底（脚本头 `:16-40`），且非 bottleneck 分支与空数组分支均已存在 |
| `c-consistency-check` | Code `consistency-check.rhai`（**同 id、同脚本**） | ⚠ **槽位必须保持 5 个**：脚本显式枚举 5 槽，槽位减少会让 `chain_node_trend4/5` resolve 为 null。快速链沿用 5 槽（不足者由段 E 输出空 `chain_nodes`，与原链 `{"trend_name":null,"chain_nodes":[]}` 的合法空形态一致）；其 `result` 经 `input_mapping` 喂给 `a-candidate-mapper` 作置信度封顶依据，避免成为数据黑洞 |

**落库与前端为何近似零改动**：

1. 提取优先级 `c-data-verifier` → `a-candidate-mapper`（`serenity.rs:1016-1041`）**首选通路命中**（同 id 复用）。非空时 `c-data-verifier.result` 是数组 ⇒ `serenity.rs` 已有的包装逻辑产出 `{"content":"[…]"}` → 提取为 `{"candidates":[…]}`，候选正常。
2. 空候选时 `c-data-verifier` 返回 `[]` ⇒ 被 `:1031-1039` 的 filter 丢弃 ⇒ 回退 `a-candidate-mapper`（Code 节点）—— **此处依赖 R10 的包装增强**，之后 `serenity_extract_from_node` 即可解析出 `{candidates:[], summary:"…"}` ⇒ `emptyReason`（`:1232-1237`）**零改动命中**。
3. trends 通路（`:1186-1191`）读 `a-trend-scanner`（同 id）⇒ **零改动**。
4. 前端 `SerenityScreeningPanel.tsx:268-294` 的节点 id→阶段映射：**沿用同名的节点无需改**；仅需补**纯新增 id**（`c-scanner-brief` / `j-*` / `t-candidate-search` / `c-candidate-pool`），缺映射只影响阶段文案、不影响结果读取。

---

## 4. 落库契约（复用面 100%，仅新增「快速链 = 同一张表同一结构」的保证）

| 项 | 事实 | 依据 |
|---|---|---|
| 表与标识 | `reco_picks`，`style = "serenity"`（快速链**不改标识**，因而历史列表 / 退出信号刷新 / 反馈回路自动一并适用） | `serenity.rs:1336-1351`、`:1499-1501`、`:1603-1606` |
| 直接读取的候选字段 | `stock_code` / `stock_name` / `confidence`（缺省 50）/ `strategy_type`（缺省 `bottleneck`）/ `positionPct`（缺省 5.0）/ `holdingDays`（缺省 20）/ `reasons` | `serenity.rs:1286-1327` |
| 价位 | 落库前抓实时行情，按 `entry = price×0.95/1.05`、`stop = price×0.80`、`target = price×1.30` 计算（行情失败则保持 0） | `serenity.rs:1302-1310` |
| `detail_cache`（前端展示用） | `serenity_score` / `catalysts` / `exit_signals` / `attention_metrics` / `bottleneck_product` / `primary_risk` / `relevance` | `serenity.rs:1363-1376` |
| 全局种子 | `set_serenity_seed` + `set_serenity_candidate_cache` | `serenity.rs:1381-1386` |
| 事件 | `serenity-screening-completed` 的 `candidates` / `trends` / `emptyReason` / `status` / `source`（`source` 按是否传 `themes` 取 `user` / `auto`） | `serenity.rs:1402-1420` |

⇒ **硬要求**：`c-candidate-assembler` 必须**保证上表字段齐备**，否则 `detail_cache` 会取到 null、前端展示残缺，即违反 H3「落库与原链一致」。

---

## 5. 脚本策略：两链共用同一份（零副本 / 零分支）

| 脚本 | 处置 | 说明 |
|---|---|---|
| `strategy-scorer.rhai` | **零改动、零副本** | 快速链只换 `chain_analysis` / `trend_strategy` 的 `input_mapping` 来源（配置动作，非脚本动作） |
| `consistency-check.rhai` | **零改动、零副本** | 5 槽位显式枚举；快速链沿用 5 槽 |
| `data-verifier.rhai` | **零改动、零副本** | 只换 `candidates_raw` 来源；脚本自带多形态解析兜底 |
| `scanner-brief.rhai` / `candidate-pool.rhai` / `candidate-assembler.rhai` | **新增资产** | 原链**没有**对应脚本（原职能全在 Agent prompt 内）⇒ 不构成「同源双版本」，符合 H2 |

> 回归要求（H2）：若为 `emptyReason` 采纳 §3-F 的「甲」方案而增强 `data-verifier.rhai`，必须回归**原工作流**，确认多出的字段不被任何原节点消费、且不改变原 `result` 既有字段。

---

## 6. 改动量汇总

| 层 | 改动 | 量级 |
|---|---|---|
| 后端·命令 | `run_serenity_screening` 增加 `template_id`（1 处硬编码 → 取值）+ 实例名前缀 + `emptyReason`/回退通路的 Code 输出包装增强（§3 的 R10） | 小 |
| 后端·脚本 | 新增 3 个 Rhai 新资产（`scanner-brief` / `candidate-pool` / `candidate-assembler`）；`strategy-scorer` / `consistency-check` / `data-verifier` **零改动** | 中 |
| 工作流模板 | 新增 `serenity-screening-fast`（段 A–F，19 → 约 19 节点，**无新节点类型**）+ 新 seed 函数与调用点（`stock_analysis_setup/mod.rs:771` 旁）；`tool_defs` / `variables` 抽成两链共用的构建函数（保持原种子产物字节不变） | 大（定义量） |
| 前端 | 面板「快速趋势智选」按钮 + `templateId` 透传 + 节点 id→阶段映射**只补纯新增 id**（沿用同名者无需改） | 小 |
| i18n | 按钮 / Tooltip 文案 × 11 语言 | 小 |
| **代码改动总量** | 命令参数 + 3 个新脚本 + 模板定义 + 前端按钮/映射 + i18n；**无新增节点类型、无适配器改造、无脚本副本/版本分叉** | — |

---

## 7. 风险登记

| # | 风险 | 强度 | 应对 |
|---|---|---|---|
| R1 | **CJK 精度**（Jev 官方自陈非最佳）+ `model-jaggedness`：精度随 state 长度劣化。趋势智选的 state **原生就是中文市场数据**，比股票分析链更贴这两条风险 | **高** | 上线前先做**影子比对**：档位一致率 + 位移方向 + 低置信占比**三项同时给出**（只看一致率会掩盖长 state 退化） |
| R2 | 32k 预算：行业排名全量 + 快讯 + 政策新闻会直接爆窗 | **高** | 强依赖 §段 B 的 `scanner-brief.rhai` 裁剪；超预算时按规则降级，不得截断到半句 |
| R3 | `strategy_type` 双写（生成腿与 Jev） | **中** | 单一权威：Jev 独占，生成腿 prompt 去掉该字段（§段 C） |
| R4 | 槽位数改动会破坏共享脚本 | **中** | 快速链沿用 **5 槽**（`consistency-check.rhai` 显式枚举） |
| R5 | 把算术/阈值判断塞进 Jev（它不做算术） | **中** | §段 B 明确划分：算术/阈值归 Rhai，Jev 只做「在已过滤清单里选」 |
| R6 | 生成腿取舍不当 ⇒ 前端大面积空态（`catalysts` / `exit_signals` / 评分等） | **中** | 采纳 §段 E 选项 ①（保留 1 个生成节点） |
| R7 | 快速链的 `a-candidate-mapper` 是 **Code 节点**（无 `content` 字段）⇒ 回退通路 + `emptyReason` 断 | **中** | **已消解**：沿用同 id + §3 的 R10 包装增强（对原链零影响）；空候选时 assembler **必须**写顶层 `summary` |
| R8 | 新模板 seed 的版本门 | **低** | 独立 `TEMPLATE_ID` + 独立 `TEMPLATE_VERSION`（建议从 **1** 起），不复用 53 的语义（`seed_serenity.rs:1411` 的版本门按模板各自计算） |
| R9 | 并发度：段 B/C 多个 Jev 节点 + 5 路评分 | **低** | Jev 单次 70–500ms、输出免费；核对引擎 `max_concurrent`（对齐上游 F6） |
| R10 | `serenity_extract_from_node` 只认 `raw["content"]` **字符串**（`:723-729`），Code 节点输出是 `{"status","result","params"}` **无 `content`**（`code_executor.rs:580-591`）⇒ 回退分支与 `emptyReason` 全断 | **高（实施期实测发现）** | 把 `:1022-1027` 既有的「Code 输出 → 包 `{"content": <result JSON 字符串>}`」逻辑提成辅助函数，**同样作用于 `.or_else` 的 `a-candidate-mapper` 分支**（约 5 行）；原链该节点是 Agent ⇒ 行为零变化 |

**运行时长改善的对比口径**（用于验收）：同 `as_of_date` / 同 `themes` 下对比两条链的 ① 端到端墙钟时长；② **LLM 调用次数与工具往返次数**（这是主因，不是并发度）；③ 落库候选条数与字段完整度（验证没为了快而丢字段）。

---

## 8. 已定（原待拍板 3 项，2026-09-24 用户拍板）

| # | 项 | 结论 | 落实 |
|---|---|---|---|
| 1 | **段 E 生成腿取舍** | **① 保留 1 个合并生成 Agent** | **沿用 id `a-trend-scanner`**（替代原 `a-trend-scanner` + 5×`a-chain-trend1..5`），一次输出 `trends[]` 且每项内嵌 `chain_nodes[]`，并承担 `catalysts` / `exit_signals` / `attention_metrics` / `bottleneck_product` / `primary_risk`；下游脚本零改动、trends 通路零改动 |
| 2 | 候选池来源 | `search_stock` 优先，无结果再 `get_stock_concept_blocks` | 由 `c-candidate-pool.rhai` 判空后置零/降级，节点侧串两个 Tool 并保留 `continue_on_fail` |
| 3 | 新旧链路关系 | **① 两链并存**（新增 `serenity-screening-fast`，两个按钮各自独立） | `serenity-screening` 的 seed 图与边一行不动（H1） |
| 4 | **组装节点 id**（实施期新增的第 4 项决策） | **沿用原链 id `a-candidate-mapper`（类型改为 Code/Rhai）** | 免去 `serenity.rs` 的 `emptyReason` 取值分支；代价是需 §3 的 R10 包装增强（对原链零影响） |

---

## 9. 实施顺序（已按 2026-09-24 用户拍板「一次做完」执行）

单轮完成、末尾统一验证（用户指示：「一次做完整个 §9」）。落地顺序：

1. 抽公共资产：把 `seed_serenity.rs` 的 `tool_defs` / `variables` 构建抽成两链共用的函数（**保持原种子产物逐字节不变**，故不升 `TEMPLATE_VERSION`）。
2. 新增 `seed_serenity_fast.rs`：`serenity-screening-fast` 模板（段 A–F，约 19 节点）+ 独立 `TEMPLATE_ID` / `TEMPLATE_VERSION`（从 1 起）+ `stock_analysis_setup/mod.rs` 调用点。
3. 新增 3 个 Rhai 资产：`scanner-brief.rhai`（段 B 裁剪，**含全部算术/阈值**）、`candidate-pool.rhai`（段 D 客观预筛）、`candidate-assembler.rhai`（段 F 组装，沿用输出 `a-candidate-mapper` 同形契约，空候选必写 `summary`）。
4. `serenity.rs`：`template_id` 参数化（`:889` 硬编码 → 取值）+ 实例名前缀 + **R10 的 Code 输出包装增强**（提辅助函数，作用于回退分支）。
5. 前端：面板「快速趋势智选」按钮 + `templateId` 透传 + `NODE_STAGE_MAP` 只补纯新增 id。
6. i18n：11 语言补按钮 / Tooltip 文案。
7. 统一验证：`cargo fmt --all --check`、`cargo check`（有无他人 WIP 阻塞须分别记录）、`npm run typecheck`、`dprint`。

> ⚠ **并行流冲突面**：`stock_analysis_setup/mod.rs` 与 11 个 `src/i18n/locales/*.json` 可能被他人 WIP 占用；另 `src-tauri/src/commands/stock_workflow/reflection.rs` 现有 5 个他人编译错误（E0425×2 / E0505×2 / E0382），**严禁改动**，主 crate `cargo check` 的失败须与之区分归因。