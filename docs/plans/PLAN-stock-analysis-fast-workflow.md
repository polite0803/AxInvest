# PLAN — 股票分析「快速分析」（Jev 决策链路）

> 状态：active ｜ 日期：2026-09-23 ｜ 对应版本：v2.9.9


**日期**：2026-09-24
**状态**：**已批准（2026-09-24）**，进入实施；§8 两项按推荐默认执行（`j-target` 取 4 档、新旧链路并存）
**目的**：在股票分析页「执行分析」旁新增「快速分析」按钮，调一条以 Jev 判定为核心的快速工作流；落库、业务后处理、前端读取**全部复用**现有股票分析链路。

## 0. 与既有方案的关系（必读）

上游文档：`PLAN-jev-integration.md`（2026-09-20，v3）、`AUDIT-jev-adoption-stock-workflow-2026-09-20.md`。

| 项 | 既有结论 | 本方案的处理 |
|---|---|---|
| §3.4 裁决「23 个 agent 节点**一个都不能替代**，Jev 只能做影子裁判」 | 该裁决的适用对象是**在原图内替换节点** —— Jev 不生成文本，故原链的 23 个 agent 一个都不能撤 ⇒ **结论正确** | **适用域不同，不构成分歧**：本方案**不动原图**，那 23 个 agent 节点在 `stock-analysis` 里**全部原样保留、照常执行**；新建的 `stock-analysis-fast` 图**本就不含这 23 个节点**，故快速链**不产出中间环节的叙述文本**（分析师 / 辩论 / 风控论述；仅段 F 保留结论解释，属用户原始设计）—— 这不是对原链的取舍，而是新图的**既定设计前提**（用户 2026-09-24 明确：快速链的目的就是快速输出决策）。两图并存、互不影响 ⇒ 上游 §3.4 在其适用域内继续成立 |
| §0 / §1 的接入路线：官方 typed API（`api.typesafe.ai/v1/systemone`）+ 新增 `JevAdapter` + `judge` 方法 | v2/v3 订正稿 | **未按原样落地**。现行实态是 `crates/providers/src/typesafe.rs` 走 OpenRouter 决策端点、按 chat 契约做协议转换，执行器**零改动**；注册见 `crates/providers/src/registry.rs:78`。本方案以**现行实态**为基础 |
| §3.5「若要用 `score`：给配置加 `question_kind: Option<"choice"\|"score">`，**不要新建节点类型**」 | 已立 | **直接沿用**，本方案 §7 的 `score` 需求走此改法 |
| §6 第 2/4 项「`config.model` 复合值机制（provider + model）」 | 已裁决并落地（`crates/dao/src/repo/provider.rs` 的 `split_node_model()`） | **直接沿用** —— 分类器节点的节点级供应商通路已通，无需新增 `provider_id` 字段 |
| §3.4 `rule-check` 不用 Jev | 已立（硬阈值属可计算规则） | 沿用：降为 Code，复用零调用方的 `crates/analysis-engine/src/rules.rs::check()` |
| §5 风险 J1（**CJK 精度**，官方自陈非最佳）、J5（32k + `model-jaggedness`：精度随 state 长度劣化） | 高 / 中 | **升级为本方案首要风险**，见 §6 —— 本方案要在 state 里塞中文原文，正踩这两条 |

### 0.1 三条硬约束（2026-09-24 用户明确）

| # | 约束 | 本方案落实 |
|---|---|---|
| H1 | **不得修改原股票分析工作流的图**（节点 / 边 / 配置） | 新增独立模板 `stock-analysis-fast`（复制原图后改造）；`stock-analysis` 的 seed 节点图与边**一行不动** |
| H2 | **两链脚本必须一致**（2026-09-24 追加）：脚本可改，但**禁止 `*-fast` 副本，禁止版本分支**；改动必须同时服务两条链，且必须回归原工作流 | 同一份 `portfolio-mgr.rhai` / `analyst-brief.rhai` 被两条链按**完全相同的变量契约**调用；差异**只允许存在于节点配置层**（`input_mapping` 指向哪个节点），脚本内部无「快速链专用」分支 |
| H3 | 落库与原工作流**一致**；前端**复用现有前端**；新链路执行时卡片**不显示文本**，但必须**正确解析出结论**（看空 / 看多 / 置信度等） | 见 §3 与 §3.1 |

> H2 可直接落地：`portfolio-mgr` 的 5 个 `trader_*` 变量**全部由节点 `input_mapping` 注入**（`seed_stock_analysis.rs:3868-3886`，路径统一为 `trader.content.verdict.*`），脚本只认变量名、不认来源 ⇒ 换来源是**配置动作而非脚本动作**。快速链让组装节点 `output_var` 仍叫 `trader` 且输出同形，**`input_mapping` 与脚本一律零改动**（详见 §4）。

## 1. 入口与启动路径

| 项 | 设计 | 依据 |
|---|---|---|
| 按钮位置 | `src/components/stock-analysis/StockSearchBar.tsx:174` 现有「执行分析」旁新增「快速分析」（同款 `isRunning` 联动；Tooltip 说明「Jev 秒级决策，不含长篇论述」） | 现有主按钮即此处的 `stockAnalysis.startAnalysis` |
| 前端动作 | `src/stores/feature/stockAnalysisStore.ts` 新增 `startFastAnalysis(stockCode)`，复用 `startAnalysis` 的参数构造，多传 `templateId: "stock-analysis-fast"` | 现有调用为 `invoke("run_stock_workflow", runArgs)`（同文件 :1073-1076） |
| 后端最小改动 | `run_single_stock_analysis` / `run_stock_workflow` 增加可选参数 `template_id`（默认 `"stock-analysis"`），把 `src-tauri/src/commands/stock_workflow/core.rs:1832` 的硬编码 `"stock-analysis"` 改为取值 | 该处 `load_and_inject_template(db, code, name, "stock-analysis")` 现为硬编码，**不参数化则两条工作流进不去** |
| 落库 | 完全复用：走业务封装路径 ⇒ 自动落 `stock_analyses`（+ `stock_reflections` 反思占位、`price_alerts` 告警、dashboard） | 见 §3 |
| i18n | 11 种语言补 `stockAnalysis.fastAnalysis` / `fastAnalysisHint` / `fastAnalysisRunning` | 项目规范：新增 UI 文本必须 11 语言齐全 |

## 2. 逐节点清单

> **口径声明（全章适用）**：本章所有节点都是 `stock-analysis-fast` 这张**新图里新建**的。本方案**不改造原图**，因此不存在「下掉 / 替代 / 保留某项」这类动作 —— 凡出现的原链节点名（`a-*` / `bull-r1` / `value-investor` 等）**仅作职能对照**，用来说明「新图用什么承接了同一职能」；那些节点在 `stock-analysis` 里照常存在、照常执行（H1）。
>
> **id 策略**：数据 / 算法 / 落库节点**沿用原链同名 id**（`t-scoring`、`portfolio-mgr`、`portfolio-risk-gate`、`quality-fallback`、`data-quality` 等）⇒ 落库三级取值（§3）与前端按 id 读取（§3.1）天然对齐；Jev 判定节点用新名 `j-*`。模板之间 id 相互隔离，同名不构成覆盖。

### 段 A · 取数与简报（复用现有 ToolNode / 算法资产，零新增节点类型）

| 节点 id | 类型 | 输入 | 输出 / 变量 | 资产来源 |
|---|---|---|---|---|
| `t-market-data` … `t-catalyst-data`（10）+ `t-pledge-data` + `t-index-quotes` + `t-institutional-visits` + `t-dragon-tiger-data` | ToolNode | `stock_code` | `{node_id}.result.content` | 复用现有数据源 ToolNode 定义 |
| `analyst-brief` | Code(rhai) | 上述数据源 + `raw-data-aggregated` | `.result`（含原文裁剪段） | **两链共用同一份脚本**（H2：无副本、无分支）：脚本**增强**为总是产出原文裁剪段（新闻 / 政策 / 研报 / 催化，各 ≈4k 字符预算，时间倒序 + 标题优先）—— 原链不消费该字段即忽略，两链同输入同输出，**行为零分叉** |
| `t-scoring` → `t-valuation` → `t-valuation-band` → `t-risk` → `t-scoring-week` → `t-scoring-month` | ToolNode·算法 | `stock_code` / `stock_codes` | 同现有 | 复用现有算法 ToolNode 与其依赖顺序（无环） |
| `data-quality` | Code(rhai) | 各数据源 | `.result` | 前置门禁；不足 ⇒ `quality-gate` ⇒ `quality-fallback` 短路 |
| `raw-data` | ToolNode | 13 个数据源 | `raw-data-aggregated` | 复用（供报告导出） |
| `t-valuation` 的 `value_dcf_*` | 设置面板变量 | — | — | 复用现有 3 个扁平参数通路 |

### 段 B · Jev 结论（13 个 `llmClassifier` 并发一组；新图不建分析师 LLM 节点）

`state` 组成：`analyst-brief.result` + 6 个算法节点输出 + 新闻/政策/研报/催化**原文裁剪段**。

| 节点 id | `input_var` | `categories` | `output_var` | 职能对照（仅参照，原图节点不动） |
|---|---|---|---|---|
| `j-market` | `analyst-brief.result` | 支持 / 中性 / 反对 | 同名 | 原链 `a-market` |
| `j-sentiment` | 同上 | 支持 / 中性 / 反对 | 同名 | 原链 `a-sentiment` |
| `j-fundamentals` | 同上 | 支持 / 中性 / 反对 | 同名 | 原链 `a-fundamentals` |
| `j-hotmoney` | 同上 | 支持 / 中性 / 反对 | 同名 | 原链 `a-hotmoney` |
| `j-lockup` | 同上 | 支持 / 中性 / 反对 | 同名 | 原链 `a-lockup` |
| `j-sector` | 同上 | 支持 / 中性 / 反对 | 同名 | 原链 `a-sector` |
| `j-news` | 原文段-新闻 | 利好 / 中性 / 利空 | 同名 | 原链 `a-news`（新图直接喂原文，无对应 LLM 节点） |
| `j-policy` | 原文段-政策 | 利好 / 中性 / 利空 | 同名 | 同上 |
| `j-research` | 原文段-研报 | 利好 / 中性 / 利空 | 同名 | 同上 |
| `j-catalyst` | 原文段-催化 | 利好 / 中性 / 利空 | 同名 | 同上 |
| `j-direction` | 段 B 全部输出 + 算法输出 | 看多 / 中性 / 看空 | 同名 | 原链 10 分析师的汇总结论 |
| `j-conviction` | 同上 | 强 / 中 / 弱 | 同名 | 原链结论强度 |
| `j-divergence` | 同上 | 一致 / 分歧 / 严重分歧 | 同名 | 原链分析师间一致性 |

**原文裁剪预算**（32k 上下文，state 约占一半）：指标类 ≈4k 字符；新闻 / 政策 / 研报 / 催化各 ≈4k 字符，按时间倒序 + 标题优先截断。裁剪逻辑落在 `analyst-brief.rhai`。

### 段 C · Jev 对抗辩论（新图不建 `bull-r1` / `bear-r1` / `debate-convergence`）

| 节点 id | `categories` | `output_var` | 消费方 |
|---|---|---|---|
| `j-bull-strength` | 强 / 中 / 弱 | 同名 | 辩论面板（改指向） |
| `j-bear-strength` | 强 / 中 / 弱 | 同名 | 同上 |
| `j-winner` | 多方 / 空方 / 平局 | 同名 | 方向参考 |
| `j-disagreement` | 低 / 中 / 高 | 同名 | `portfolio-mgr` 的置信度修正 |

> 修既有缺陷：现 `disagreement_score` 实测 53 次落库仅 9 次取到值（其余走 `portfolio-mgr.rhai` 的 0.7 下界）；Jev 的 `category` 必然有值 ⇒ 修正从「偶发生效」变「每票生效」。

### 段 D · Jev 风险 + 估值（新图不建 `risk-agg` / `risk-con` / `risk-neu` / `risk-convergence` / `value-investor`）

| 节点 id | `input_var` | `categories` | 原语 | 对应资产 |
|---|---|---|---|---|
| `j-risk-level` | 段 B/C 输出 + `t-risk` | 低 / 中 / 高 / 极高 | choice | 承接原链三档风险评估；`risk-level.rhai` 另作交叉校验（两链共用同一份） |
| `j-risk-score` | 同上 | 高 / 中 / 低 | choice | 风险强度 |
| `j-pledge-risk` | `t-pledge-data.result` | 是 / 否 / 数据不足 | noul | 质押超警戒线 |
| `j-lockup-risk` | `t-lockup-data.result` | 是 / 否 / 数据不足 | noul | 解禁临近 |
| `j-goodwill-risk` | `t-fundamentals-data.result` | 是 / 否 / 数据不足 | noul | 商誉占比 |
| `j-valuation-signal` | `t-valuation.result` + `t-valuation-band.result` | 低估 / 合理 / 高估 / 无法估值 | choice | `value-investor` 判断内核 |
| `j-margin-of-safety` | 同上 | 强 / 中 / 弱 | choice | 同上 |
| `j-moat` | 同上 | 宽 / 窄 / 无 | choice | 同上 |
| `j-dcf-evidence-usable` | `t-valuation.result.content.dcf.assumptions.*` | 可用 / 不可用 | noul | **把现 ~40 行 prompt 硬判据（`applicable` / `is_fallback_anchor` / `fcf_data_missing`）改成 criteria** |

本段同样新建这些节点：`t-valuation` / `t-valuation-band` / `t-risk`（f5 十条注入的唯一来源）、`portfolio-risk-gate`（一票否决，优先级高于 `portfolio-mgr`）；`risk-level.rhai` 作为两链共用资产。

### 段 E · Jev 决策 → 落库三节点契约

**不设 `j-position` / `j-action`**：`portfolio-mgr.rhai:966-977` 明确注释**不消费 `trader.positionPct`**（避免仓位自指）与 **不消费 `trader.action`**（避免 action 自指）；仓位由 `portfolio-mgr.rhai:1775` 的 `pm_kelly_position(effective_posterior, odds, COST_PCT, overall_risk)` 算出，action 由 `position_pct` 阶梯反推（同文件 :2034-2049）。

`trader` 对决策的真实贡献只有两处 —— `f7_signal`（同文件 :972-1055）与 `odds`（同文件 :1037-1049）：

| 节点 id | `categories` | 注入到（`input_mapping` 键） | 影响 |
|---|---|---|---|
| `j-direction` | 看多 / 中性 / 看空 | `trader_direction` | f7 方向 + odds 兜底判据 |
| `j-confidence` | 高 / 中 / 低 | `trader_confidence` | f7 幅度（**顺带修 V75 伪二值缺陷**：注入端期望 0-100 而 LLM 给 0-1，实测 σ 恒 ±0.6、f7 被降权至 0.10；Jev 输出 0-1，改映射时统一单位即恢复幅度） |
| `j-risk-level` | 低 / 中 / 高 / 极高 | `trader_risk_level` | f7 的 `risk_mod` |
| `j-target` / `j-stop` | 相对现价档位（+5% / +10% / +20% / +30%） | 经 `trader-proxy` 换算为绝对价后仍走 `trader_target_price` / `trader_stop_loss`（见 §4） | odds |

**落库三节点 id 必须沿用同名**（`src-tauri/src/commands/stock_workflow/decision.rs` 的 `extract_decision_json` 按节点 id 三级取值）：`quality-fallback`（优先）> `portfolio-risk-gate`（`portfolio-mgr` 的超集）> `portfolio-mgr`（公式层）。缺任一项 ⇒ 落库走兜底分支生成**占位决策**。

### 段 F · Agent 解释（新图唯一的 LLM 节点）

`decision-explainer`（沿用同名 id 新建），`context_sources` 补入段 B–E 全部 Jev 输出 + `analyst-brief`。**「从原文提炼要点」并入本节点** —— 需要论据清单 / 多空论述时由它生成，零额外关键路径耗时。

## 3. 落库契约（复用面 100%）

| 项 | 事实 | 依据 |
|---|---|---|
| 落库触发 | 走业务封装路径，`input` 带 `analysis_id` ⇒ 由业务路径自行落库；`StockAnalysisPersistHook` 此时跳过 | `src-tauri/src/commands/stock_workflow/hooks.rs:780-784` |
| 决策提取 | 按节点 id 三级优先（`quality-fallback` > `portfolio-risk-gate` > `portfolio-mgr`），再抽 `action` / `position_pct` / `reasoning` / `time_horizon` / `expected_holding_days` / 四周期价位与独立决策 | `src-tauri/src/commands/stock_workflow/decision.rs` 的 `extract_decision_json` / `extract_decision_fields` / `extract_horizon_price_map` / `extract_decisions_by_horizon` |
| 落表 | `stock_analyses` + `stock_reflections` + `price_alerts` | `src-tauri/crates/entities/src/stock_analyses.rs` |
| 前端读取 | 按表读，与工作流内部用 Jev 还是 LLM **无关** | `src/components/stock-analysis/AnalysisHistoryButton.tsx` 等 |
| 模板身份 | 业务封装路径硬绑定模板 id，故需 §1 的参数化 | `src-tauri/src/commands/stock_workflow/core.rs:1832` |

> **既有隐患（与 Jev 无关，但会被本方案放大）**：两条落库链口径不等价 —— `src-tauri/src/commands/stock_workflow/hooks.rs:743-757` 的 `extract_decision_from_results` **只认 `portfolio-mgr`**，而 `decision.rs` 的版本认三级（风控门优先，是超集）。同一只股票走对话直执行 vs 业务封装，落库仓位可能差 8pt 级（V71 实证 601166：8.85% vs 风控门值）。违反项目铁律「同一语义不得被两条链按不同口径消费」。建议一并统一（hooks 版改调 `decision::extract_decision_json`）。

### 3.1 前端复用与结论解析（H3）

**关键事实**：前端对决策/解释的解析**集中在一层**（`src/lib/agentOutput.ts`），不是散落在各卡片 ⇒ 「复用现有前端 + 正确解析结论」= 在这一层加一个入口，卡片组件基本不动。

| 现有导出 | 对新链路的行为 | 动作 |
|---|---|---|
| `normalizeDecision` / `extractDecision` / `tryParseDecision` | 读 `portfolio-mgr` 输出或落库 JSON，**与节点内部用 Jev 还是 LLM 无关**（§3 事实） | **零改动** —— 看空 / 看多 / 仓位 / 价位照常解析 |
| `extractValuationApplicability` | 读 `t-valuation` 输出（算法节点，两链共用） | 零改动 |
| `parseDecisionExplanation` | 读 `decision-explainer` 文本（段 F 仍产文本） | 零改动 |
| `extractContent` | Jev 节点输出为 `{category, confidence, model, provider, node_id}`（无 `content` / 文本字段） | **空态降级**：卡片不渲染空文本区（即 H3 的「不再显示文本」）；实施时需实测确认不抛错、不落假占位 |
| **新增** `extractJevJudgment` | — | 读 `{category, confidence}` 归一为「方向 + 置信度」徽章；新增徽章标签必须 **11 语言齐全**（项目规范） |

> 按**节点 id 硬读**的面板（如辩论 / 风险面板指向 `bull-r1` / `risk-convergence`）在快速链下会空 —— 处理见 §6 F4。

## 4. 脚本策略：两链共用同一份（零副本 / 零分支）

**事实前提**：`portfolio-mgr` 节点的 5 个 `trader_*` 全部走 `input_mapping` 注入（`seed_stock_analysis.rs:3868-3886`），脚本只按变量名读取（`present(trader_direction)` 等），**不关心来源是 LLM agent 还是 Jev** ⇒ 换来源是**配置动作**，不需要动脚本。

| 变量 | 脚本期望（行号） | 原链注入路径 | 快速链注入路径 |
|---|---|---|---|
| `trader_direction` | string，`f7_signal` 首判据（:980） | `trader.content.verdict.verdict` | **同路径**（由组装节点提供） |
| `trader_confidence` | f64，>1 自动按 0-100 归一（:995-1002） | `trader.content.verdict.confidence` | 同路径 |
| `trader_risk_level` | string，`risk_mod`（:1019） | `trader.content.verdict.riskLevel` | 同路径 |
| `trader_target_price` / `trader_stop_loss` | f64，决定 `odds` ⇒ 凯利仓位（:1037-1049、:1702-1714、:1775） | `trader.content.verdict.targetPrice` / `.stopLoss` | 同路径 |
| `trader_evidence_count` | **必须 array**：脚本判 `type_of(...) == "array"` 后取 `.len()`（:1006-1008）；非数组 ⇒ `evidence_mod = 0.4` | `trader.content.verdict.evidence_cited` | 同路径 |

⇒ **做法**：快速链新增一个 Code(rhai) 组装节点 `trader-proxy`，`output_var` **仍叫 `trader`**、输出**同形**对象 ⇒ `portfolio-mgr` 的 `input_mapping` 与 `portfolio-mgr.rhai` **双双零改动**，且两条链调用的是同一份脚本、同一套路径。

| 组装项 | 来源 | 说明 |
|---|---|---|
| `verdict.verdict` | `j-direction.category` | 看多 / 中性 / 看空 ⇒ f7 方向 |
| `verdict.confidence` | `j-confidence` 的 Jev `confidence`（0-1） | 单位与脚本设计一致（脚本另兼容 0-100） |
| `verdict.riskLevel` | `j-risk-level.category` | ⇒ `risk_mod` |
| `verdict.targetPrice` / `.stopLoss` | **算法算，不用 LLM**：`target = currentPrice × (1 + dcf.upsidePct/100)`（`t-valuation` 现成）、`stop = currentPrice × (1 - kσ)`（σ 取 `t-risk`）；`j-target` / `j-stop` 的档位结果用于**夹逼/校验** | 绝对价 Jev 给不出（官方明确不做算术）⇒ 这是本方案唯一不可省的自研计算 |
| `verdict.evidence_cited` | **数组**：段 A–D 实际取到数的数据源/因子清单（可由 `data-quality` 的覆盖度结果派生） | 必须保持数组类型，否则 `evidence_mod` 静默退化为 0.4 |

**其余脚本零改动、零副本**：`risk-level.rhai` / `portfolio-risk-gate.rhai` / `data-quality.rhai` / `quality-fallback` 原样共用；`analyst-brief.rhai` 仅**增强**（总是产出原文裁剪段，两链同输入同输出，见段 A）。

> ⚠ 唯一新增脚本是 `trader-proxy` —— 它是**新资产**，不是任何原脚本的新版本：原链的 `trader` 由 LLM agent 节点产出，本就**没有**对应脚本，故不构成「同源双版本」。
> ⚠ 回归要求（H2）：`analyst-brief.rhai` 的增强必须回归原工作流，确认多出的原文裁剪段不被任何原节点消费、且不改变原 `result` 既有字段。

## 5. 改动量汇总

| 层 | 改动 | 量级 |
|---|---|---|
| 后端·命令 | `template_id` 参数化（1 处硬编码 → 取值） | 小 |
| 后端·脚本 | `analyst-brief.rhai` **增强**（总是产出原文裁剪段，两链共用）+ **新增** `trader-proxy.rhai`（组装 `trader` 同形对象，§4）+ 统一两条落库链口径（§3 备注）；**`portfolio-mgr.rhai` 零改动、无副本、无版本分支** | 中 |
| 工作流模板 | 新增 `stock-analysis-fast`（段 A–F），复用全部数据源/算法/rhai 资产 | 大（定义量，无新节点类型） |
| 前端 | 1 按钮 + store 方法 + 解析层新增 `extractJevJudgment`（§3.1）；**仅**「新图不建该职能」的面板需改读 `j-*` 或隐藏（§6 F4）—— 沿用同名 id 的节点前端零改动 | 小 |
| i18n | 3 key × 11 语言 | 小 |
| **代码改动总量** | 仅命令参数 + 脚本增强 1 处 / 新增 1 个 + 前端按钮/面板 + i18n | **无新增节点类型、无适配器改造、无脚本副本/版本分叉**（Jev 由 `llmClassifier` 的 `config.model` 复合值指向，现单问题形态足够） |

## 6. 风险登记（首要两条沿用上游 §5）

| # | 风险 | 强度 | 应对 |
|---|---|---|---|
| F1 | **CJK 精度**（上游 J1，官方自陈非最佳）+ **`model-jaggedness`**：精度随 state 增长劣化 | **高** | 段 B 的 `j-news` / `j-policy` / `j-research` / `j-catalyst` 直接吃中文原文 ⇒ **上线前必须先在自有语料上做影子比对**（档位一致率 + 位移方向 + 低置信占比三项同时给出，只看一致率会掩盖长 state 退化） |
| F2 | 32k 预算（上游 J5）：4 类原文 + 指标 + prompt 共享同一预算 | **中** | 严格按 §2 段 B 的裁剪预算；原文超预算时按规则降级而非截断到半句 |
| F3 | 原文取舍靠规则，做不到「读完 50 条挑关键 3 条」 | 中 | 提炼能力挪到段 F（`decision-explainer`） |
| F4 | **前端按节点 id 读、而新图没有该职能的面板**：`valueAssessments`（`value-investor`）、`rule_check.*`（`rule-check`）、辩论面板（`bull-r1`/`bear-r1`/`debate-convergence`）、`AnalystReportGrid`（`a-*`） | **中** | ① **沿用同名 id 的节点**（`t-*` / `portfolio-*` / `data-quality` 等，见 §2 id 策略）⇒ 前端**零改动**；② 仅上述「新图不建该职能」的面板需处理：改读对应 `j-*` 节点，或在新链下隐藏该面板 |
| F5 | `odds` 精度取决于**算法腿**：`target = 现价 × (1 + dcf.upsidePct/100)` 与 `stop = 现价 × (1 - kσ)` 的系数取值；`j-target` / `j-stop` 档位退为**夹逼校验**（§4） | 中 | 系数先用行业惯例值，落库后按赔率分布与实际兑现度回测校准；档位夹逼的触发需留日志以便复盘 |
| F6 | 段 B 共 13 个并发 `llmClassifier` | 低 | 核对 `src-tauri/src/commands/stock_workflow/core.rs` 的 `max_concurrent`；不够则分批，仍远快于 LLM |
| F7 | `confidence_threshold` 口径（上游 J2：`confidence` ≠ `max(probabilities)`） | 中 | 若启用阈值，必须按自有数据重标定后再放开 |

## 7. 前期可选改动（可独立先做，与本方案解耦）

1. **修 V75 伪二值缺陷**（f7 从恒 ±0.6 恢复幅度）—— 对决策的影响可能大于整个 Jev 化。
2. **统一两条落库链口径**（§3 备注）。
3. **`rule-check` 降为 Code**，复用 `crates/analysis-engine/src/rules.rs::check()`（返回同形 violations/corrections/force_signals，源码侧当前零调用方）。
4. 若要用 `score` 原语（段 C 的「分歧程度」、段 D 的「量级」）：按上游 §3.5 给配置加 `question_kind: Option<"choice"|"score">`，**不新建节点类型**；适配器侧需按上游 §0 补 `score`（本仓 `crates/providers/src/typesafe.rs` 当前只暴露 choice / noul）。

## 8. 待拍板（2 项）

> **已定前提**（用户 2026-09-24）：快速链的目的就是**快速输出决策**。新图**不建**原链那些分析师 / 辩论 / 风控 LLM 节点 —— 它们只存在于原图、照常运行（见 §2 口径声明），新图用 Jev 判定节点承接同一职能。这是设计前提，非待议项。

> **实施口径**（用户 2026-09-24「按这个方案来」）：上述两项均按**推荐默认**执行 —— `j-target` / `j-stop` 取 **4 档**；新旧链路**并存**（不动 `stock-analysis`）。实施顺序见 §9。

| # | 项 | 选项 |
|---|---|---|
| 1 | `j-target` / `j-stop` 档位 | 4 档（+5/+10/+20/+30%）还是更细 —— **已不直接决定赔率**（绝对价由算法腿算出，§4），档位退为**夹逼校验**边界：粒度越细校验越紧、越可能把算法价夹偏 ⇒ 建议先粗后细 |
| 2 | 新旧链路关系 | ① 并存（新增 `stock-analysis-fast` 模板，两按钮各自独立，推荐）② 就地替换 `stock-analysis` |

## 9. 建议实施顺序

1. §7 第 1、2 项（独立收益，先做）
2. §1 的 `template_id` 参数化 + 前端按钮 + 空模板 ⇒ 打通入口并确认落库正常
3. 段 A + 段 E（复用面最大）⇒ 验证落库决策字段与旧链路可比
4. 段 B / C / D 逐段替换 Jev，逐段对比落库结果
5. 段 F 与前端面板改指向（F4）