# AUDIT — 股票分析工作流 Jev 适配度与耗时归因

**日期**：2026-09-20
**范围**：`stock-analysis` 工作流（`seed_stock_analysis.rs`，TEMPLATE_VERSION=55（2026-09-20 由 54 升版，见 §2.4⑦），58 个节点）
**数据源**：本地 PostgreSQL `axinvest` 真库 `workflow_executions`（510 行，2026-07-20 ~ 2026-09-18）
**目的**：回答「哪些节点可换 Jev（TypeSafe AI 决策模型）」+「为什么跑一次这么慢」

---

## 0. 结论摘要

1. **一次股票分析实测均值 660s（11 分钟）**，最高 1084s。慢的构成里 LLM Agent 节点占 **~100%**，所有 Code/Tool/Validation/Notification 节点合计 **< 6s**。
2. **Jev 能砍的是「判定型」节点，实测可省 ~270-290s（45-65%）**，最强标的依次为：`trader`(69s) → 三档风险(70s) → `risk-convergence`(52s) → `rule-check`(43s) → `debate-convergence`(41s)。
3. **但最大单点 `debate-bull-bear`（avg 310.5s，占 37-70%）Jev 吃不掉** —— 它的产物是两位研究员的论证全文，Jev 不生成文本。
4. **两个反直觉判断**：
   - `cls-risk-level` 看起来是 Jev 最佳标的（prompt 就是「只输出一行标签」），但它**既不该换模型、也不该直接删，应下沉 Rhai 并与 `f4_quant_adjust` 一起去重**。它跑的确实是纯 if-else 阈值规则（没有必要用 LLM），但其输出 `risk-level` 喂 `f4_signal`（权重 0.15，实测 26/27 非零）——**直接删会让 26% 样本跨 action 档、且全部向「更看多」= 放松风控**。另有单独缺陷：两侧分类器实测只 37% 一致。详见 §2.4（该节已订正）。
   - `rule-check`(43s) 同理：5 条硬阈值判定，属 `data-quality` 已验证的「LLM→Rhai」改造路径。

---

## 1. 耗时画像（实测，非推算）

### 1.1 总时长分布

```sql
SELECT status, count(*), round(avg(total_time_ms)/1000.0) AS avg_sec,
       round(max(total_time_ms)/1000.0) AS max_sec
FROM workflow_executions WHERE total_time_ms > 0 GROUP BY status;
```

| status | runs | avg | max |
|---|---|---|---|
| partially_completed | 202 | 289s | 7257s |
| **completed** | **105** | **411s** | **1735s**（撞 1800s 兜底） |
| cancelled | 18 | 156s | 505s |
| failed | 2 | 250s | 306s |

股票分析按日（近 14 天，含 `a-market-analyst`）：

| 日期 | runs | avg | max | 辩论段 avg |
|---|---|---|---|---|
| 09-14 | 7 | **660s** | 1084s | 215s |
| 09-13 | 3 | 453s | 489s | 165s |
| 09-12 | 5 | 512s | 836s | 162s |
| 09-11 | 5 | 1034s | 1635s | 388s |
| 09-10 | 7 | 1163s | 1319s | 443s |

### 1.2 单次运行完整分解（442s，`dec22240-07ba-4fe6-96fb-81a463562c95`）

按 `node_executions.started_at` 重建时间线：

| 段 | 墙钟 | 占比 | 决定性节点 |
|---|---|---|---|
| 10 个数据工具（并行） | 0 - 5.6s | — | `t-sector-data` 5.6s |
| 10 个分析师（并发 8） | **64.9s** | 15% | `a-market-analyst` 64.2s |
| `analyst-brief`(Rhai) | 0.006s | — | — |
| **辩论容器（6 辩手串行）** | **163.9s** | **37%** | 42.1+30.2+27.3+17.8+28.6+17.9 |
| `debate-convergence` | 18.3s | 4% | — |
| 三档风险 + value-investor（并行） | **62.1s** | 14% | `risk-con` 62.1s |
| `risk-convergence` | 8.3s | 2% | — |
| `research-mgr` | 26.6s | 6% | — |
| `trader` | 40.6s | 9% | — |
| `portfolio-mgr`+`risk-gate`(Rhai) | 0.09s | — | — |
| `rule-check` | 37.7s | 9% | — |
| `quality-fallback` | 19.6s | 4% | 仅走 fallback 分支 |
| `decision-explainer` | 14.0s | 3% | — |
| 全部 Code/Tool/Validation/Switch/Notification/Storage | **< 6s 合计** | ~0% | — |

> ⚠ 该次为旧模板（3 轮辩论）。v48（2026-09-14）起固定 1 轮 ⇒ 辩论段降至 ~72s。

### 1.3 近 14 天节点耗时排名（avg，仅 >1s）

| # | 节点 | 类型 | runs | avg | max |
|---|---|---|---|---|---|
| 1 | `debate-bull-bear` | debate(容器) | 34 | **310.5s** | 655s |
| 2 | `risk-con` | agent | 33 | 70.6s | 149s |
| 3 | `risk-agg` | agent | 33 | 70.5s | 163s |
| 4 | `trader` | agent | 28 | 69.0s | 241s |
| 5 | `research-mgr` | agent | 29 | 63.2s | 197s |
| 6 | `risk-neu` | agent | 34 | 61.5s | 99s |
| 7 | `bear-r2` / `bull-r3` / `bear-r3` / `bull-r2` | agent | ~33 | 61.8 / 54.5 / 54.6 / 43.7s | — |
| 8 | `a-market-analyst` | agent | 60 | 56.3s | 164s |
| 9 | `value-investor` | agent | 33 | 53.5s | 197s |
| 10 | `risk-convergence` | agent | 33 | 52.2s | 128s |
| 11 | `a-lockup`/`a-catalyst`/`a-research`/`a-sentiment`/`a-news`/`a-hot-money`/`a-fundamentals`/`a-sector`/`a-policy` | agent | 42-50 | 34-51s | — |
| 12 | `rule-check` | agent | 28 | 42.9s | 90s |
| 13 | `debate-convergence` | agent | 32 | 41.2s | 108s |
| 14 | `decision-explainer` | agent | 27 | 31.8s | 102s |

> ⚠ **2026-09-20 复核订正（原注脚的后半句推论不成立）**：`r2/r3` 确实落在近 14 天窗口内，
> 但**全部来自 09-14 当天 v48 生效之前**的运行（含 `bear-r3` 的最后一次 = **09-14 15:59**；
> 09-14 之后无任何一次）。真库实测：DB 模板内 `position('bear-r3' in nodes) = 0`、
> `position('bear-r2' in nodes) = 0` ⇒ **模板已无多轮节点定义**。
> ⇒ 「v48 未完全生效 / 存在多轮残留」是**把历史数据误读为当前状态**；模板侧 `debate_max_rounds`
> 已是硬编码 1（`seed_stock_analysis.rs:1965`）。
> 证据：`output/_probe-s1-rounds.txt`（探针 `output/_probe-s1-rounds.mjs`）。

---

## 2. Jev 适配判定

**唯一判据**：节点输出是否为**类型化答案**。Jev 三原语 —— `Noul`（是/否 + 概率）、`Choice`（≤255 选项 + 每项概率）、`Score`（2-10 档有序量表，可落档间）。**不生成文本**（无自由文本输出）。

### 2.1 ✅ 强适配（Jev 可直接替代 ≥ 80% 的节点职责）

| 节点 | 实测 avg | 当前输出 | Jev 原语 | 可省 |
|---|---|---|---|---|
| `trader` | 69.0s | 纯 JSON：`action`(枚举)/`positionPct`/`confidence`/`targetPrice`/`stopLoss`/`riskLevel`（`seed:3096-3120`），`max_tool_rounds=0` 无工具 | Choice + Score + Score | **~68s** |
| `risk-agg`/`risk-con`/`risk-neu` | 61-71s（并行，墙钟取 max） | 同一份输入 × 三视角 → `position_pct` + 风险评级（`seed:2409-2449`） | Choice + Score | **~70s**（墙钟） |
| `risk-convergence` | 52.2s | `disagreement_score`（单数值）+ verdict（`seed:2503`） | Score | **~52s** |
| `debate-convergence` | 41.2s | `consensus_score`（单数值）+ `aggregate_prediction`（`seed:2142`） | Score | **~41s** |
| `rule-check` | 42.9s | 5 条硬阈值 → `violations`/`corrections`/`force_signals`（`seed:3626-3627`） | Noul ×5 | **~43s** ⚠ 更宜 Rhai |
| `quality-fallback` | ~20s（仅 fallback 分支） | 严格 JSON：`{action, positionPct, confidence:20-40, riskLevel}`（`seed:3713-3716`） | Choice + Score | ~20s |
| `cls-risk-level` | 实测 avg **4.78s**（71 次 completed）／5 次 timeout 各 ~50s | 单标签 `风险等级\|最短理由`，4 类（`seed:2766-2767`） | Choice | ~4.8s，但**别用 Jev**，应下沉 Rhai —— 见 §2.4 |

**合计可省 ~270-290s**（基线 400-660s ⇒ 降幅 45-65%）。

### 2.2 ⚠ 部分适配（混合输出：长文本正文 + 尾部机读标签）

这些节点的**正文是给人和下游看的产物**，Jev 无法替代；但其**结构化结论**可由 Jev 独立产出，用于交叉校验或替代下游解析。

| 节点 | 实测 avg | 为何不能整体替换 |
|---|---|---|
| 10 个分析师 `a-*` | 34-56s | 输出「自然语言报告 + 尾部 `<!-- VERDICT: {...} -->`」（`seed:1708-1717`）。正文是报告主体；且 VERDICT 有实测截断史 |
| `bull-r1`/`bear-r1` | 42-48s | 论证全文，且**辩论语义要求串行**（`seed:2102`） |
| `value-investor` | 53.5s | 护城河/管理层/财务健康长篇论述 + 估值区间 |
| `research-mgr` | 63.2s | 「总体风险评级」（可 Jev）+「主要风险点清单」（不可） |

**可用形态**：Jev 出 `bull_score`/`bear_score`/`verdict` 等字段，与 LLM 正文的 VERDICT 做一致性校验（`harness/src/consistency_check.rs` 已有该机制）。

### 2.3 ❌ 不适配

- `decision-explainer`（31.8s）：核心产物是「200-300 字说明书 + rule_trace」（`seed:3780-3803`），**文本生成即目的**。
- 全部 Code(Rhai) / Tool / Validation / Switch / Notification / Storage 节点：合计 <6s，已是确定性最优。
- **不得接入**：`crates/prompt-guard/**`（零 LLM 纯正则 4 层，改外部 API = 安全链依赖公网）。

### 2.4 ⚠ 订正：`cls-risk-level` **不可直接删**，应下沉 Rhai（原标「与 `f4_quant_adjust` 一起去重」**已于 2026-09-20 撤回**，理由见 §⑩.4）

> **本节初版结论（「失败无阻断性代价」「输出恒为空」「可零成本删除」）已于 2026-09-20 实测证伪并订正。**
> 初版依据是 `seed:2812-2818` 的**源码自述注释** + 一份仅含 3 次调用的旧审计。真库 76 次实测显示自述与事实不符。

**① 实测运行画像（推翻「输出恒为空」）**

| status | n | avg_ms |
|---|---|---|
| completed | 71 | 4 784 |
| timeout | 5 | 49 983 |

成功率 **93%（71/76）**，平均 4.8s。它**不是**恒失败节点。5 次 timeout 的运行全部落 `partially_completed`（其中 3/5 的 `portfolio-mgr` 仍跑完，161ms）。

**② 它对结论有实质影响 —— 「无阻断性代价」不成立**

`risk-level` 有**两个**消费点：

| # | 消费点 | 位置 | 影响 |
|---|---|---|---|
| 1 | `overall_risk_llm` → `f4_signal` | `portfolio-mgr.rhai:484-492` | 低 +0.30／中 −0.05／高 −0.25／极高 −0.50 |
| 2 | `research-mgr` 上下文 | `seed:2986` `context_sources` | 缺失静默跳过（无阻断，但少一份输入） |

而 `overall_risk` 本身（影响 `risk_bias`／仓位 cap／V43 硬否决）**几乎不受它影响**：实测 25 条 `portfolio-mgr` 输出的 `riskSource`，**24 条「算法」、仅 1 条「LLM回退」**。

⇒ **它的真实职责已退化为「一个 4 档→信号的查表函数」**：名义上的分类结果 96% 被 `portfolio-mgr.rhai:396-449` 的算法路径覆盖弃用，唯一生效用途是喂 `f4_signal` 的 switch 映射。花 4.8s LLM 调用做代码里 6 行 switch 的事。

**③ 直接删的代价（实测，非推算）**

`f4_signal` 实测 **26/27 非零**，权重 **0.15**（13 因子里第 3 大）。删后 `f4_adjusted` 少掉该项，按 `Δposterior = −0.15 × f4_signal / total_weight`（total_weight≈1.459）估算：

| f4_signal | 档位 | n | Δposterior |
|---|---|---|---|
| −0.25 | 高风险 | 19 | **+2.6pp** |
| −0.50 | 极高风险 | 4 | **+5.1pp** |
| +0.30 | 低风险 | 2 | −3.1pp |
| −0.05 | 中风险 | 1 | +0.5pp |

**跨 action 档实证**（阈值 0.38/0.48/0.53/0.63）：**7/27 = 26% 的样本会跨档，且全部向「更看多」**（6 条跨过 0.38：减持/卖出→观望；1 条跨过 0.48→持有）。
⇒ 直接删 = **系统性放松风控**，与「瓶颈掘金 + 风险否决」的方法论方向相反。

**④ 改 Rhai 的隐藏回归：两侧分类器实测只有 37% 一致**

| LLM 判档 | 算法判档 | n |
|---|---|---|
| HIGH | HIGH | 10 |
| HIGH | MID | **7** |
| EXTREME | missing | 3 |
| LOW | MID | **2** |
| HIGH | missing | 2 |
| EXTREME | HIGH | **1** |
| MID | HIGH | **1** |

一致率 **37%（10/27）**，明确分歧 **11 条（41%）**，方向以「LLM 更严」为主（8:3）。两侧输入**同源**（都是 `t-risk.stockRiskProfile`）、阈值**几乎重合**（prompt「波动率>40% 或 夏普<0 或 回撤>45%」 vs `RISK_VOL_HIGH=40` `:131`／`RISK_DD_HIGH=45` `:132`），却给出 41% 不同的档位。
⇒ **照抄算法阈值做 Rhai 化，会有 ~41% 样本的 `f4_signal` 取值改变**，不是零回归。

**⑤ 更深的缺陷：同一批指标被判定三次**

`f4_signal` 的注释（`portfolio-mgr.rhai:480-483`）声称「只读 LLM 分类，与 `overall_risk` 的算法分类**解耦**，避免双重惩罚」，但输入同源 + 阈值重叠 ⇒ **假解耦**。实测叠加：`f4_base=−0.25` 档的 `avg(f4_quant_adjust)=−0.179`，合计 `f4_adjusted≈−0.43`；而 `overall_risk` 又经 `risk_bias` 影响 `effective_posterior`。**同一批 `stockRiskProfile` 指标实际被消费三次**。

**⑥ prompt 本身确实是纯确定性规则**（初版唯一成立的一条，保留）

`seed:2740-2762`：极高险 = 负债率>80% 且 营收增速<0；或 波动率>60% 且 夏普<−1.5。高险 = 波动率>40% 或 夏普<0 或 回撤>45%。并明写「不要计算综合评分，直接按规则判定」⇒ 用 LLM 跑 if-else。同文件先例：`data-quality` 从「LLM Agent 5-10s」改为 Rhai 后 **<10ms**（`seed:2821-2824`）。

**结论（三选一，推荐 ①）**

| 方案 | 动作 | 结论影响 | 收益 |
|---|---|---|---|
| **① 推荐** | Rhai 化 + **单一判定源去重**：新增 `risk-level.rhai` 为唯一分类实现，`portfolio-mgr.rhai:396-449` 与 `f4_quant_adjust` 改为消费它 | 分类只做一次 ⇒ 消除三重消费与 41% 分歧；`f4_signal` 语义保留 | ~4.8s + 消除 timeout/`partially_completed` + 消除双罚 |
| **② 保结论（已实施 · v55）** | Rhai 化，规则 1:1 照抄现 prompt（含 4 档阈值） | ⚠️ **实测不成立「近零回归」**：档位一致率 64.4%、36% 样本 `f4_signal` 改变；但 posterior 位移**均值仅 +0.15pp**（双向抵消，无系统性放松）—— 详见 §2.4⑦ | ~4.8s + 消除 timeout；41% 分歧仍在 |
| ③ 不推荐 | 直接删 | **26% 跨档、全部放松风控** | ~4.8s |

> **附带修掉一个静默降级**：`fallback_label: Some("中风险")`（`seed:2777`）在失败时固定回退中风险 ⇒ `f4_signal = −0.05`（近乎中性），而真实可能是极高风险（应 −0.50）。方案 ①/② 后该路径消失（确定性不会失败）。
>
> ⏩ **2026-09-20 后续订正**：上句只对「LLM 调用失败」成立。Rhai 版仍有**另一条**静默兜底 ——
> 数据缺失（`annualizedVolatilityPct` 缺失 ⇒ `degraded=true`）时退化为「中风险」，同样得到
> `f4_signal = −0.05`。**该支已裁决改为保守档「高风险」**（`risk-level.rhai`，
> `TEMPLATE_VERSION` 59→60）：理由是 `degraded` 对决策路径**不可见** —— `portfolio-mgr.rhai`
> 的 f4 因子只读 `category`、不吃 `degraded`，故中性兜底在决策链上等价于「已判定为中风险」。
> 详见 `PLAN-jev-integration.md` §6 第 3 项。
>
> **不要用 Jev 止痛**：即使换成 Jev（70-500ms），仍是「用外部模型跑 if-else」+ 未解决 41% 分歧与三重消费。Jev 的正确落点是 §2.1 那些**真需要语义判断**的节点。

**⑦ v55 已实施（2026-09-20）—— 方案 ② 落地 + 影子比对实测**

用户裁决选**方案 ②**。实施清单：

| # | 改动 | 位置 | 说明 |
|---|---|---|---|
| 1 | 新增脚本 | `src-tauri/src/commands/risk-level.rhai` | 原 prompt 原文**完整保留在头部注释**作为口径权威源；7 条判据逐条转写 |
| 2 | 节点改型 | `seed_stock_analysis.rs` 的 `cls-risk-level` | `LlmClassifierNode` → `CodeNode`（`include_str!` 嵌入）；`timeout` 60→10s、重试关闭、`continue_on_fail` false→true |
| 3 | 路径穿透 | 同文件 `portfolio-mgr` 的 `input_mapping` | `cls-risk-level.category` → `cls-risk-level.result.category`（CodeNode 输出包在 `result` 里） |
| 4 | 升版 | 同文件 `TEMPLATE_VERSION` **54 → 55** | 节点内容变更 ⇒ 不升版则 DB 模板永久不更新（v50/v54 记过两次同型陷阱） |
| 5 | 清 import | 同文件 use 块 | `LlmClassifierNode`/`LlmClassifierNodeConfig` 已无使用处（否则 clippy `-D warnings` 失败） |
| 6 | 文档订正 | `config/llm-output-schemas.json` | agent 列表移除 `risk-level`；`data-quality` 亦已于 v58 下沉 Rhai，一并订正 |

**影子比对**（真库 59 条可比对样本：`t-risk` 的 7 指标 × `portfolio-mgr` 的 `risk_quant.f4_base` 反推 LLM 实测档）

| 变体 | 一致率 | posterior 位移均值 | 位移 ≥2pp |
|---|---|---|---|
| **严格照抄 prompt（已采用）** | **64.4%**（38/59） | **+0.15pp** | 21/59 |
| 特异性优先 + 兜底上档（**未采纳**，见下） | 69.5%（41/59） | +0.01pp | 18/59 |

混淆矩阵（行 = LLM 实测档，列 = 新 Rhai 规则档）：

| LLM ＼ Rhai | 低风险 | 中风险 | 高风险 | 极高风险 |
|---|---|---|---|---|
| 低风险 | 0 | 2 | 0 | 0 |
| 中风险 | 0 | 0 | 1 | 0 |
| 高风险 | 0 | 9 | **32** | 5 |
| 极高风险 | 0 | 1 | 3 | **6** |

**核心判读：位移均值仅 +0.15pp（双向抵消，最大上抬 +4.63pp / 最大下压 −3.60pp）⇒ 不存在系统性放松或收紧。**

**⚠️ 必须记录的预期偏差**：本轮此前把方案 ② 描述为「近零回归」——**实测不成立**，36% 样本的 `f4_signal` 取值会变。分歧归因：

1. **主因是原 LLM 判定自身不一致，而非本脚本偏离规则**（决定性实证）：`debt=91.6 / growth=−0.3` 被判「高风险」，而 `debt=92.2 / growth=+8.3` 被判「极高风险」—— 后者负债率更高、营收增长为正，档位反而更严。同族另一条以 `vol=22.7 / dd=9.5 / sharpe=0.403`（量化健康）仅因 `debt=92.2` 被判极高风险。
   ⇒ **追求 100% 复现 LLM 等于拟合噪声**；换成确定性实现本身就消除了这部分随机性，属改造目的而非回归。
2. **LLM 会因单点极端值独立上档**，不严格遵循 prompt 的「A 且 B」合取：9 条「高→中」中 `vol=86.6%`、`vol=97.3%`、`debt=90.4%` 这类单点极端值让 LLM 直接判高风险，而字面规则要求量化与基本面**同时**满足。
3. **prompt 的「低风险」档 7 项全为 AND，实测零命中**（原 LLM 判出的 2 条低风险用的是更宽标准）。未改门槛以保持与 prompt 字面一致 ⇒ 该档在新实现下更稀有。

**未采纳的对齐增强（仅登记，未施工）**：把「中风险以基本面无硬伤为前提，前提不成立则上档高风险」显式化 —— 实测可把一致率 64.4% → 69.5%（分歧 21→18 条）。不采纳理由：该逻辑**超出 prompt 字面**（属推断 LLM 的隐含规则），与方案 ② 的「1:1 照抄」承诺冲突；且会重新引入「用推断规则替代声明口径」这类难审计的形态。

**已知未实现项（数据不可得，改造前后一致 ⇒ 属等价照抄）**：prompt 的「极高风险① 标的为 ST/*ST/退市股」与「低风险③ 无 ST/退市风险、无重大负面公告」两条判据**无法实现** —— `stockRiskProfile`（`astock-data/src/mcp_tools.rs` 的 `compute_portfolio_risk` 输出）只含 7 个数值字段，无 ST 标记、无公告数据；`is_st` 仅存在于上游 `Quote`（`astock-data/src/types.rs`）且未被透出。原 LLM 版同样拿不到该信息。**启用需在该工具补 `isST` 字段 —— 属改口径，待批。**

**复现**：`output/rhai-shadow-compare.mjs`（脚本）+ `output/rhai-shadow-report.txt`（全量分歧明细）。规则正确性可独立复核：脚本内 7 条判据与 `risk-level.rhai` 头部注释的 prompt 原文逐条对应。

**⑧ v55 复核（2026-09-20 第二轮）：真实 Rhai 引擎实跑 + 3 处缺陷修复**

影子比对（⑦）当时是**用 JS 复刻规则**得出的，并未真正执行 `.rhai`。本轮补上「真实引擎执行」，并因此查出 3 处缺陷。

**方法**：新增独立执行台 `output/rhai-harness/`（单独 `target` 目录，**不抢项目 `target` 锁**），逐字复刻 `shared_rhai_engine()` 的 `Engine::new()` + 六项沙箱上限 + `register_common_functions` + `json_value_to_dynamic`/`dynamic_to_json_value` + `extract_present_vars`/`extract_local_vars`，对真库 63 条样本实跑。报告：`output/harness-report-PREFIX.txt`（改前）/ `harness-report-FIXED.txt`（改后）。

**引擎能力面（探针实测，非查文档）**：裸 `Engine::new()` **确有** StringBasicPackage/ArrayBasicPackage —— 字符串 `+` / `+=`、数组索引与 `len`、`for i in 0..n`、`type_of`、`to_float`、map 字面量全部可用；且 `dynamic_to_json_value` **不会**把 `f64` 截断成整数（`1.5` 原样保留）。
⇒ 订正一条流传的笼统说法：历史坑（`archive/2026-08.md:956` 的 data-quality.rhai 全败）是 `String::replace` 不在默认包内，**不是**「没有 StringPackage」。判据应是「脚本是否用了 `replace`/`split` 这类完整 StringPackage 函数」，而不是笼统怀疑整个字符串能力面。

**输入类型契约（生产库实证，曾差点误判）**：`t-risk.output.result.content` 是**字符串**（JSON 文本），`resolve_var_path` 穿透中间段时自动 `from_str` 解析它，故 `...content.stockRiskProfile.<field>` 最终是 **JSON `number`**（7 个字段逐个 `jsonb_typeof` 确认）。
⚠️ 陷阱：先前用 `->>`（text 抽取）导出的样本里 7 个值**全是带引号的字符串**。若信了这份导出就会得出「脚本 7 个 `has_*` 全 false」的错误结论。**核对类型契约必须用 `jsonb_typeof()`，不能用 `->>` 的文本形态。**

**修复 3 处缺陷（对 59 条样本的分类结论零影响：改前改后同为 64.4% / +0.15pp）**

| # | 缺陷 | 改前实测症状 | 修复 |
|---|---|---|---|
| 1 | **7 个输入均未出现于 `present(...)`** ⇒ `code_executor` 的 V57 扫描不为其自动补 unit | 生产库 63 样本上**故意摘掉** `risk_roe` 的映射：**10/10 全部报错** `Variable not found: risk_roe (line 132)` —— 整条节点硬失败，下游 `overall_risk_llm` 取不到值 | 7 个输入一律 `present(X) && has(X)`；同一负向测试变为 **10/10 成功** |
| 2 | **哨兵值泄漏进输出** | `metrics.gross_margin_pct`（缺失）输出 `0.0`，会被读成「毛利率就是 0%」 | 新增 `maybe(ok, v)` ⇒ 缺失落 `null` |
| 3 | `has()` 注释谎称兼容「字符串化数字」（代码并不支持） | 注释与代码不符，误导后人 | 注释改写为「字符串**不算**可用数值，走 degraded 兜底」 |

**反事实量化（回答「类型契约一旦漂移会怎样」）**：把同样 63 条样本的数值**以字符串形态**注入（模拟上游协议漂移），一致率从 64.4% 跌到 **1.7%**、**58/59 塌缩为「中风险」**、posterior 位移均值由 +0.15pp 变为 **+2.27pp（系统性放松风控）**。
⇒ 这是**静默失效**（不报错、不告警），故脚本输出的 `degraded` 与 `warnings` 是这个契约的可见性依赖，不可删。

**新增仓内回归测试 4 条**（`crates/rt-workflow/.../code_executor.rs`，刻意走 `execute_rhai_directly` / `CodeExecutor::execute` **真实链路**而非只 `engine.eval`）：① 正常路径 + 哨兵不泄漏；② 波动率缺失 ⇒ degraded 兜底不报错；③ 漏配单个 input_mapping ⇒ 不硬失败（V57 回归）；④ `execute()` 的 `result` 包装与 `output_var="risk-level"` 契约。
另注：新增内联测试**必须留在文件末尾**（`clippy::items_after_test_module` 只在 clippy 下暴露）。

**前端契约已核实（非推断）**：`src/lib/decisionInputDiagnosis.ts` 的 `extractContentField()` **确会**解包 CodeNode 的 `result`（该函数 `:93` 注释即写明），故 `case "cls-risk-level"` 的 `getPath(content, "category")` 无需改动；仅订正该文件 `:59` 的 `role`，由 `"LLM 风险分类"` 改为 `"确定性风险分类"`（否则面板在叙述上失实）。

---

## 3. Jev 治不了的部分（结构性缺陷）

即使 §2.1 全部落地，耗时也只降到 ~130-190s。剩余瓶颈不在模型速度：

| # | 缺陷 | 位置 | 影响 | 状态（2026-09-20 回源码复核） |
|---|---|---|---|---|
| **S1** | **辩论容器 310.5s（max 655s）是最大单点** | `seed:1923-1954` | 产物是论证全文，Jev 无法替代 | ⚠ **后半句已订正**：v48 固定 1 轮后该段降至 ~72s；「DB 中仍有 r2/r3」是**历史数据误读**（见 §1.3 订正），模板内 `bear-r2/r3` 已不存在 ⇒ 不存在「未完全生效」 |
| **S2** | ~~容器体子节点无 retry / timeout 接线~~ | `rt-workflow/.../engine/mod.rs` | ~~模板声明的 `maxRetries`/`timeout` 失效；一次 504 抖动即终结整链~~ | ✅ **已修（非本次审计施工）**：`dispatch_container_body_with_retry`（`:6166`）在位 —— `node.base_timeout()` → 回退 `DEFAULT_CONTAINER_BODY_TIMEOUT=300s`、超时折算 `TIMEOUT`（与普通失败同一套重试判定）、只取**节点级** `retry`、`is_non_retryable_error` 短路、复用 `compute_backoff`。真实调用点 `:5915`(debate) / `:6433`(swarm)，另有 `debate_executor.rs:107` / `swarm_executor.rs:89` 注释交叉印证「重试粒度是子节点、不是整个容器」。<br>原判据 `:6273` 已过时 —— 现在 `:6271` 的裸 `dispatch()` 只是 `execution_workflows` **未命中时的 fallback 分支** |
| **S3** | ~~header timeout 声明值 vs 实际值差 20 倍~~ | `providers/src/openai.rs:998` | ~~实际 15s 判死~~ | ✅ **已修（2026-09-08 / 09-10，非本次审计施工）**：现为**随体积缩放** `10 + body_size/(64KB) × 5`、**封顶 60s**。且两个超时**作用域本就不同**（响应头阶段 vs body 分片），「20 倍差」部分是误读 ⇒ 见下方订正 |
| **S4** | **并发上限 8 < 10 个分析师** | `stock_workflow/decision.rs:1589` | 分析师段排两批，墙钟被最慢节点放大 | ❌ **未改**（2026-09-20 实测仍为 `DEFAULT_MAX_CONCURRENT = 8`）。注：模板变量 `max_concurrent` 可覆盖，clamp 1–32（`:1614-1617`） |
| **S5** | **容器耗时记录不可信** | `p-analysts` 记录 0.004s（装饰容器立即 Completed） | 按 `node_executions` 判断「哪个容器慢」会得出错误结论，必须看子节点 | ℹ 观测性结论（方法学提醒，非待办项） |

> **S2/S3 订正依据**：
> - S3 的演进在 `openai.rs:987-997` 的注释里有完整记录：2026-09-08 加响应头超时（上游网关 hang 住实证挂 82s 才返回）；**2026-09-09/10 实测两次反例** —— 固定 10s 会杀大 prompt 请求（`portfolio-mgr` 308KB / `candidate-mapper` 149KB / `data-quality` 142KB 的 prefill 排队远超 10s），固定 30s 会把小请求（`a-news`）推过 `step_timeout(120s)` 触发整节点重跑 ⇒ 改为随体积缩放。
> - ⚠ 本表原写「`:998` = 15s」，与当前代码不符；**`engine/mod.rs:6164` 的文档注释同样仍写「15s」**（也已过时，其**意图**「本函数只补重试机会、不抬阈值」仍成立）——「注释里的数字会腐烂」的又一实例。
> - ⇒ **S2/S3 属「审计报告初版之后、被其它会话/既有提交修复」**：本报告是快照，引用行号与结论**已被后续改动刷新**，引用前须回源码复验。

---

## 4. 建议实施顺序

| 阶段 | 动作 | 预期收益 | 原风险 | 状态（2026-09-20 回源码复核） |
|---|---|---|---|---|
| **①** | `cls-risk-level` **下沉 Rhai**（新建 `risk-level.rhai`，方案 ② 口径）—— **未直接删** | 4.8s/轮 + 消除 timeout 与 `partially_completed` | — | ⏳ **代码已完成（v55）／DB 未落库**：版本门是 `>=`，常量 55 < DB 现值 58 ⇒ **运行期 100% 不生效**（§⑨）；已升 `TEMPLATE_VERSION` → **59**，**待跑一次工作流后生效**（§⑩.3）。实测档位一致率 **64.4%**、位移均值 **+0.15pp**（§2.4⑦ + §2.4⑧ 真实引擎实跑复核）⇒ 非「近零回归」，但无系统性方向偏移。<br>~~去重阶段（改 `portfolio-mgr.rhai:396-449`，即方案 ①）~~ **已撤回** —— 见下条 |
| **①** | `f4_signal` / `f4_quant_adjust` / `overall_risk` **三重消费同一批 `stockRiskProfile` 指标**（§2.4⑤） | 消除双罚与 41% 判档分歧 | 中（改核心脚本，需产品语义决策） | 📌 **登记中，未修**。⚠ **订正**：**「去重」这个修法本身已被撤回**（§⑩.4 —— 三层是有意解耦，去重会同时改掉「双重惩罚防护」与「LLM 缺失时独立承担」两条设计）；但「同一批指标被消费三次」这个**事实仍然成立**，只是**正确解法未定** ⇒ 需产品语义裁决 |
| **①** | `rule-check` → Rhai CodeNode（照 `data-quality` 先例） | ~43s | 低 | ❌ **未做，且「零成本」的定级不成立**：仍是 AgentNode（`seed:3664`）；需照 v55 流程先做**影子比对 + 契约保住**（prompt 是硬性阈值规则；`:3677-3687` 已做过「移除全部工具 + `max_tool_rounds=0`」⇒ 纯决策节点）⇒ 属**独立立项**（§⑩.7） |
| **② 接 Jev（先影子）** | ⚠ **目标节点已订正**：原写 `trader` / `risk-*` / `*-convergence`（它们是产出**叙述文本**的 agent 节点，而 **JEV 不生成文本**）⇒ 改为「对其**同一 state** 提 typed 问题、只比对不驱动」的**影子裁判**；**试点首选 4 个 `cognitive_*` 路由**（英文标签，规避 JEV 的 CJK 弱项） | 验证中文金融语料效果 | 中（效果未验证） | ❌ **未开始**（§⑪ D3）。**接入方案 v2 已成文** → `PLAN-jev-integration.md`（接口形态已由官方文档确认；含 3 处 v1 方向性订正、真库取证、8 项风险登记） |
| **③ 修结构性缺陷** | ~~S2 容器体接线 retry/timeout；S3 header timeout 与声明对齐~~ | 消除偶发整链失败 | 中（引擎层改动） | ✅ **S2/S3 已修（非本次审计施工，见 §3 订正）**；**余下真实待办 = S4 并发上限**（`DEFAULT_MAX_CONCURRENT` 仍为 8）⇒ 与 S1（订正后只是耗时事实）、S5（方法学提醒）不同，S4 是唯一仍待动的结构性项 |
| **④ 放开驱动** | 影子比对达标后，Jev 驱动决策，保留本地 fallback | ~270s | 中 | ❌ **未开始**（依赖 ②） |

**前置约束**：Jev 早期访问无 SLA，64k/请求上限；所有调用点**必须保留本地 fallback**（`condition_executor` 已有 `heuristic_fallback` 可参考）。且 Jev **无世界知识**——只认传入的 `state`，依赖上游把上下文结构化（本项目 workflow 变量已具备）。

---

## 5. 复现方法与数据陷阱

### 5.1 连接真库（唯一正确入口，口令不进 argv）

```bash
node scripts/pg-connect.mjs sql "<SQL>"
```

### 5.2 核心查询

```sql
-- 近 14 天节点耗时排名
SELECT node->>'node_type', node->>'node_id', count(*),
       round(avg((node->>'execution_time_ms')::bigint)/1000.0,1) AS avg_s,
       round(max((node->>'execution_time_ms')::bigint)/1000.0) AS max_s
FROM workflow_executions w, LATERAL jsonb_array_elements(w.node_executions::jsonb) AS node
WHERE w.node_executions LIKE '[%' AND w.node_executions LIKE '%a-market-analyst%'
  AND w.created_at > extract(epoch from now())-14*86400
  AND (node->>'execution_time_ms')::bigint > 1000
GROUP BY 1,2 ORDER BY avg_s DESC;
```

### 5.3 ⚠ 三个必须知道的坑

1. **`workflow_executions.node_executions` 是 `text` 列** —— 必须 `::jsonb` 转换，且加 `LIKE '[%'` 过滤非 JSON 值，否则静默失败/报错。
2. **同库两种时间基准**：`workflow_executions.created_at` 是**秒**级 epoch，而 `node_executions[].started_at` 是**毫秒**级。混用会得到 1970 年的日期（本次实测已踩）。
3. **`l1_llm`（认知编排 L1 兜底）的 988 次 failed 不可当常态读** —— 全部集中在 2026-08-28 单日，错误为 `UNSUPPORTED_PROVIDER: ... http://127.0.0.1:8080/v1/chat/completions`（当时本地 provider 未启动）。正常时单次 avg ~2.5s。**单日集中 ⇒ 环境故障，不是失败率**。

---

## 6. 附：本次未覆盖

- `a-candidate-mapper`(avg 112.9s)、`a-chain-trend1~4`(65-75s)、`a-trend-scanner`(50.4s) 属**其它工作流**（瓶颈掘金链），不在本次范围，但同为「LLM 长文本生成」，Jev 适配度判定与 §2.2 相同。
- 前端耗时可视化目前只有 `ExecutionReplayPanel` / `DebugPanel` 的表格 + Timeline，**无甘特图**；`TraceRecorder`（`engine/trace_recorder.rs`）与 `audit_recorder`（`engine/mod.rs:889` 恒为 None）两条时间线机制在生产中**从未接线** ⇒ `slowest_duration_ms` 等统计永不产出。

### 6.1 v55 复核（2026-09-20 第二轮）遗留的未覆盖项

- ✅ **`degraded` / `warnings` 已接上消费方**（2026-09-20 第三轮 —— **本条已闭环**，施工见 §⑩.3）
  原状（第二轮实证）：Rust 侧全仓仅 `seed_stock_analysis.rs:3223` 取 `cls-risk-level.result.category`；
  TS 侧 `decisionInputDiagnosis.ts` 只读 `category`，`stockWorkflowChatBridge.ts:251` 只读节点状态
  ⇒ 脚本的降级契约（`degraded` / `warnings` / `metrics` 不写哨兵值）**零消费方**，
  只能事后查 `node_executions` —— 属「设计成可见、实际不可见」，比静默失败更难发现。
- **门禁**：
  - ✅ `cargo fmt -p axagent-rt-workflow -- --check` → EXIT=0（首次为 1，新增测试代码有 3 处未格式化，已 `cargo fmt` 修正）；
  - ✅ `cargo check -p axagent-rt-workflow --all-targets` → EXIT=0，`Finished in 3m 54s`；
  - ✅ `cargo clippy -p axagent-rt-workflow --all-targets -- -D warnings` → EXIT=0；
  - ✅ `cargo test -p axagent-rt-workflow --lib` → **EXIT=0**，`test result: ok. 269 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out`（全量、无过滤），耗时 1.10s（`Finished in 8m 16s`）。
    本轮新增 4 条**逐条通过**：`risk_level_direct_path_category_and_no_sentinel_leak` / `risk_level_missing_volatility_degrades_instead_of_failing` / `risk_level_missing_mapping_entry_does_not_hard_fail` / `risk_level_execute_wraps_result_and_declares_output_var`。
  - ℹ️ 该测试起初被**外部会话**阻塞：报 `Blocking waiting for file lock on build directory`，持锁者是 `cargo test --workspace`（PID 10420 / 26524，01:23:01 启动，**非本会话所发**）。按纪律未杀进程、未抢锁，排队等待至其释放（约 12 分钟）。
  - ℹ️ `cargo fmt` 修正发生在 clippy **首跑中途**，其结论可能对应旧快照 ⇒ 已对最终内容**重跑** clippy → `CLIPPY2_EXIT=0`（`Finished in 10m 58s`）。首次 `cargo check`（3m54s）通过的是未格式化版本，但 fmt 只改空白、语义不变。
  - ⚠️ `cargo check -p axagent`（12m45s，零 error/warning）**不含 rt-workflow**（被判 fresh 未重编）⇒ 不能拿它当 rt-workflow 的判据。

**复核证据文件**：`output/harness-report-PREFIX.txt`（改前）/ `harness-report-FIXED.txt`（改后）/ `cargo-rtw-v55.log` / `cargo-rtw-final-v55.log`；执行台源码 `output/rhai-harness/`。

**⑨ v55 落库核验（2026-09-20 第三轮）：发现「升版升得不够高」，方案 ② 当时 100% 空转 —— 已修**

⑧ 复核全绿后，收尾时补查「升到 55 是否真落库」（本项目 v50/v54 记过同型陷阱）。真库结果：

| 项 | 值 |
|---|---|
| DB `workflow_templates.version`（`id='stock-analysis'`） | **58**（`updated_at` = 2026-09-18 06:04:22） |
| 源码 `TEMPLATE_VERSION` | **55**（`seed_stock_analysis.rs:703`） |
| 版本门 | `:738` `} else if existing.version >= TEMPLATE_VERSION { return Ok(()); }` |

⇒ `58 >= 55` 成立 ⇒ **种子永久跳过** ⇒ DB 里 `cls-risk-level` **仍是旧 `LlmClassifierNode`**，
`risk-level.rhai` 未写入 ⇒ **方案 ② 在运行期 100% 不生效**。
而 `cargo check` / `clippy` / `fmt` / `test` **全绿** —— 它们只回答「代码能否编译」，**不回答「版本门会不会开门」**。

**修复**：`TEMPLATE_VERSION` 55 → **59**（严格 > 58），并在原处写明判据与核验 SQL。

⚠️ **判据订正**：升版**不是「比上一版 +1」**（注释里 v52→v53→v54→v55 的序列极易误导），
而是**严格大于 DB 现值** —— 因为门是 `>=` 而非 `==`。
⚠️ DB 的 58 从何而来**未查明**：`updated_at` = 09-18 06:04 **早于**注释里 v53/v54 的 09-19 ⇒ **与注释序列自相矛盾**。
可能来自并发会话 / 带更高常量的构建 / 模板保存动作（该表有 `is_editable`）。**来源不影响处置**，
但证明：**不能靠源码注释的版本序列推断 DB 现值。**
⚠️ 副作用同前：升版会**覆盖 DB 中对该模板 nodes/edges 的手工调整**（`variables` 的自定义值保留）。

**通用教训**：「改了文件」→「编译过」→「测试过」→「**落库生效**」是**四道独立的门**，前三道全绿证明不了第四道。
凡涉及**版本门 / feature flag / 迁移开关**的改动，收尾必须查一次**判据源的真实现值**。

---

**⑩ 第三轮收尾（2026-09-20）：卡住的不是一批而是三批 + 触发路径订正 + 守门测试 + §2.4 主张撤回**

⑨ 只查到「常量 55 / DB 58 ⇒ 门恒关」。本轮把取证推进一层，**结论扩大 3 倍**。

**1) 快照表取证法（新，可复用）**

查 `workflow_template_versions`（**复数** —— 权威是实体 `crates/entities/src/workflow_template_version.rs:7` 的
`#[sea_orm(table_name = "workflow_template_versions")]`；⚠ 我**先按单数查**得到「关系不存在」，
差点得出「快照表不存在 ⇒ 种子写快照必失败」的完全错误结论，靠跨 schema 模糊搜才纠回）。

写法语义：`{id}_v{N}` 快照 = 「发现 DB 现值 = N、准备升到 N+1」时写的**旧版留档**
⇒ **末条快照的 `created_at` = 最后一次成功落库时刻**。实测：

| 快照 | version | created_at |
|---|---|---|
| `stock-analysis_v51` | 51 | 09-18 01:04:04 |
| `stock-analysis_v52` … `v57` | 52…57 | **09-18 06:03:38 → 06:04:22（44 秒内连升 6 版）** |
| （此后无记录） | — | — |

⇒ 写 `v57` 快照那次把 DB 变成 **58**，此后**一次都没有**。

**2) 三条独立判据逐批确证未落库（全部真库实测）**

| 批次 | 判据 | 实测 |
|---|---|---|
| ~~**v53**~~ | `position('consensus_eps_is_estimated' in nodes)` | **0 —— 但该判据作废（见下方订正）**：0 是**结构性**的，与落库无关 |
| **v54** | `portfolio-mgr` 的 `input_mapping` 是否仍含被删 6 键 | **仍在**（`trader_action` / `trader_data_gaps` / `trader_position_pct` / `trader_stop_loss_pct` / `trader_take_profit_pct`） |
| **v55** | `cls-risk-level` 的 `x->>'type'` | **`llmClassifier`**（旧 prompt 原文完整在场；DB 的 `code` 节点仅 7 个，无它） |

> ⚠️ **2026-09-20 订正（v53 整行作废）**：原写「v53 形态最隐蔽 —— 拒判分支已生效但工具入参声明
> 未落库 ⇒ 半截状态」。**这个判断是错的**。回源码取证（`output/_probe-v53-anchor.txt`）：
> `detect_earnings_surprise` **全仓无任何节点硬接线** ——
> ① `tool_def_map`（`seed:1424-1481`）不含它；② 节点 `config.tools` 由 `PROFILE_TOOLS` 的**名字**
> 经 `seed:1781` 的 `filter_map(|tn| tool_def_map.get(tn).cloned())` 解析，**查不到即静默丢弃**；
> ③ 定义它的 `td_earnings` 只被 `_bull_tools`（`seed:2036`，下划线 = 刻意未使用）引用。
> 而代码自己在 `tools/src/tools/finance.rs:1332-1343` 写明：「全仓**无** rhai / 节点硬接线调用它
> ⇒ A2 在消费端只是一道**软防线**；**产出端不再造数才是硬解**」，且产出端已收敛
> 「触发前提**当前已消失**：全链只剩 `false`」。
> ⇒ **v53 不是一个「待落库」批次**：它在模板 DB 上**没有可见落点**，`0` 恒成立。
> 其有效部分（产出端不再造 `is_estimated=true`）**已随二进制生效**。
> ⇒ 连带作废：本节 9) 的「补上 v53 覆盖缺口」与 §⑪ B2 把 v53 与 v54/v55 并列为「三批待落库」。

**3) ⚠ 订正 ⑨ 的触发路径（⑨ 写错了）**

⑨ 写「待用户**启动应用**触发重种子」—— **错**。`ensure_stock_analysis_experts_seeded` 的调用点
**全在业务路径**，init 链里没有：

| 调用点 | 上下文 |
|---|---|
| `stock_workflow/decision.rs:397` | `load_and_inject_template` —— 每次运行股票分析前（**主路径**） |
| `stock_workflow/decision.rs:418` | 模板节点为空时的兜底重种子 |
| `stock_workflow/serenity.rs:857` | Serenity 扫描链路 |

而 `load_and_inject_template` 又被 `core.rs:583` / `core.rs:1762` / `reflection.rs:178` / `serenity.rs:862` 调用。
⇒ **正确表述：跑一次股票分析（或反思 / Serenity 扫描）即会落库；启动应用不会。**
（`decision.rs:393-396` 注释亦记录过同族历史事故：「代码 v13-v15 期间 DB 停在 v12，V59 修复静默失效三天」。）

**4) ★ 撤回 §2.4 的「与 `f4_quant_adjust` 一起去重」主张（自我纠错）**

我上一轮把去重列为「方案 ①」。本轮读设计注释后发现**该提议本身是错的** ——
`portfolio-mgr.rhai:487-490` 原文：

> P0-2 修复: f4_signal 只读 LLM 分类（与 overall_risk 的算法分类解耦）
> overall_risk 已基于量化指标计算，f4_quant_adjust 又用同样的指标做修正
> **为避免双重惩罚**，f4_signal 改为只读 LLM 的独立风险判断
> 当 LLM 分类缺失（算法兜底场景）时 f4_signal=0.0，由 f4_quant_adjust 独立承担

⇒ 三层各有分工、**有意解耦**：

| 层 | 数据源 | 作用 |
|---|---|---|
| `overall_risk`（`:403-459`） | 算法（vol/dd/sharpe/roe/debt/growth） | 展示 + `risk_rank_val` |
| `f4_signal`（`:491-499`） | **只读** `overall_risk_llm`（= `cls-risk-level` 的 category） | 风险因子的 LLM 腿 |
| `f4_quant_adjust`（`:505+`） | 量化指标（带成长风格豁免） | 风险因子的量化腿；LLM 缺失时独立承担 |

去重会同时改掉「双重惩罚防护」与「LLM 缺失时的独立承担」⇒ **撤回**。

**5) 顺带纠掉一次「死分支」误报**

`portfolio-mgr.rhai:462` 的 `if !risk_classified && present(overall_risk_llm)` 看似死代码
（`:448-458` 的 if-else **之后**无条件置 `risk_classified = true`）。
但 grep 全部赋值点后确认：初始化在 `:399`，`:458` 的赋值**在 `if present(risk_volatility) && >0 {`（`:403` 开块）块内**
⇒ 波动率缺失时块不进入、卫兵仍 false ⇒ **分支可达**（作者注释：真库 25 条 completed 命中 1 条）。
**只看尾部那几行就会误报。**

**6) 本轮施工**

- `seed_stock_analysis.rs` 顶部新增 `pub(crate) const TEMPLATE_VERSION: i32 = 59;`
  （原为函数体内 `const`，测试引用不到）；函数内原处留指针注释 + 三批未落库的取证结论。
- `stock_analysis_setup/mod.rs` 末尾新增 `#[cfg(test)] mod version_gate_tests`（**3 条**，
  见 `:2414` / `:2457` / `:2526`）：
  ① `version_gate_seeds_then_upgrades_then_skips` —— 门的三态语义
  （空库建 / 旧版升级 / **新版跳过且不把 version 拉低**）；
  ② `seeded_template_carries_rhai_risk_level_node` —— 版本号对 ≠ 内容对：断言
  `cls-risk-level.type == "code"`、`code` 含 `matched_rules`、**`config.prompt` 为空**，
  且 `portfolio-mgr.overall_risk_llm == cls-risk-level.result.category`（CodeNode 多一层包装）。
  ⚠ **初版写的是「`code` **不残留**旧 prompt 原文」——方向错**，会在 v59 上**必红**
  （该串由 `risk-level.rhai:115` 头部注释**有意保留**作口径权威源，详见本节 9）；
  已改锚到旧节点**专属字段** `config.prompt`。
  ③ `seeded_template_carries_v53_and_v54_changes` —— 把核验脚本的 ⑥/⑦ 判据**同步进 CI**
  （v53 的 `consensus_eps_is_estimated` 入参 / v54 的 6 条悬空 `input_mapping` 已消失），
  理由：seed 文件上万行、常改，**内容回归是静默的**（版本号对 ≠ 内容对）。
- **不碰生产库**：用 `axagent_dao::db::create_test_pool()`（`crates/dao/src/db.rs:744`），
  它走**与生产同一个** `initialize_schema` 建表链，落 `temp_dir` 唯一 SQLite 文件 ⇒ 可进 CI。
  这也规避了真跑生产库的实质风险：种子是 **`delete_by_id` + `insert` 两步**
  （`:4459-4460` / `:4550`），中途失败会让 `stock-analysis` 模板**丢失**。
  （`:800` 注释的「用 UPDATE 替代 DELETE」指的是**版本快照**那段，不是主写入 —— 两处勿混读。）
- 判据全部从 `TEMPLATE_VERSION` **现取**，不写死数字 ⇒ 改常量不会让测试腐烂。
- **给用户的只读核验脚本**：`output/verify-v59-seed.mjs`（跑过一次工作流后执行，逐项 ✅/✘，
  覆盖 version / 节点类型 / 脚本内容 / 源路径 / v54 六键 / 快照）。

**7) 仍待裁决（四项，均为「已登记」而非「我该擅自改」）**

| 子项 | 状态 | 出处 |
|---|---|---|
| `rule-check` 下沉 Rhai | **同 v55 形态但需立项**：它是 AgentNode（`seed_stock_analysis.rs:3664`），prompt 是硬性阈值规则（RSI/乖离率/止损/放量下跌/空头排列），且 `:3677-3687` 已做过「移除全部工具 + `max_tool_rounds=0`」优化 ⇒ 纯决策节点。下沉需先影子比对 + 契约保住（照 v55 流程），**不是顺手活** | `seed_stock_analysis.rs:3660-3697` |
| `isST` 补入 `compute_portfolio_risk` | **属改口径，待批**（原 LLM 版同样拿不到该数据 ⇒ 不启用**不算退化**） | `risk-level.rhai:69-74` / `:213` / `:236` |
| `risk_source="LLM回退"` 字面量统一 | **待统一项**（会展示到 UI ⇒ 产品文案决策；该分支真库 25 条 completed 仅命中 1 条） | `portfolio-mgr.rhai:392-394` |
| 方案 ① 去重 | **已撤回**（见本节 4） | — |

**8) 门禁（v59 + 守门测试）**

| 门禁 | 结果 |
|---|---|
| `cargo fmt -p axagent -- --check` | 首次 EXIT=1（新增测试的 1 处 `use` 折行）→ 改回单行（`:2366`）⇒ 复验 **EXIT=0** ✅，且 `cargo fmt --all -- --check` 亦 **EXIT=0** ✅（`output/_fmt-check-4.txt`） |
| `cargo test -p axagent --lib version_gate`（3 条） | ⚠ **首跑 = 旧快照，已作废**：`TEST_EXIT=101`，`running 2 tests`（新增的第 3 条不在二进制里），`panicked at src\commands\stock_analysis_setup\mod.rs:2484` 消息 = 「不得残留旧 LlmClassifierNode 的 prompt 原文」——**这句话在修正后的源码里已不存在**（现行 `:2494` 断言 `config.prompt` 为空）⇒ 二进制编自旧源码，**判定依据充分**，与本节 6) 的预测（「原样交付会在 v59 上必红」）逐字吻合。旧快照的编译早在 18:28:33 启动、早于 18:28:56 的修正 ⇒ 重跑中（日志 `output/cargo-v59-rerun.log`） |
| `cargo clippy -p axagent --all-targets -- -D warnings` | 首跑 `CLIPPY_EXIT=-1`（进程被外部终止，非编译错误；同批 shim 亦失效）⇒ 无有效读数，重跑中。⚠ 纪律：clippy **遇首错短路** ⇒ 不可由「只有 N 个 error」推定其余干净；修完必须**全量重跑**。另 `#[cfg(test)] mod` 必须留在文件**末尾**，否则触发 `clippy::items_after_test_module`（该 lint 只在 clippy 下暴露；本模组结束于文件末行 `:2568` ✅） |
| 生产库落库核验 | `output/verify-v59-seed.mjs` —— 已实跑负对照（见本节 9），判定 7 项在 v58 上全数报警、退出码 1 |
| 前端（第 1 项接线，§6.1） | `dprint fmt` / `dprint check` ✅、`tsc --noEmit` ✅、`vitest` **7 passed (7)** ✅ |

**证据文件**：`output/_probe-version-history.mjs`（快照历史）/ `_probe-verify-v53-v54-v55.mjs`（三批判据）/
`_probe-v54-tooldDefs.mjs` / `verify-v59-seed.mjs`（核验）/ `cargo-v59-test.log`（门禁）。

**9) 核验脚本自证 —— 抓出「交付工具」自己撒谎的两处缺陷（2026-09-20 第三轮收尾追加）**

`output/verify-v59-seed.mjs` 是上一轮交付的核验工具，但**从未被执行过**。
一个没跑过的核验脚本只是一段「声称能核验」的代码。本轮拿它跑**负对照**
（生产库仍在 v58 = 未落库态），立刻暴露两处**自身**缺陷：

| # | 缺陷 | 后果 | 修法 |
|---|---|---|---|
| 1 | ③b「`config.code` 不含旧 prompt 原文」 | **空洞通过** —— 该报警的状态下永远报 ✔ | 改锚到旧节点**专属字段** `config.prompt` |
| 2 | ~~**漏验 v53**（只验 v54/v55 却宣称「三批」）~~ | **此「修法」本身是错的（第三次修正已推翻）** —— v53 **没有 DB 落点**，`> 0` 恒不成立 ⇒ 补上的是条**永久假红** | 已改为**不计分的 ○ 项**（见本节 11） |

**定锚依据（两面都实测过才敢定锚）**：旧 prompt 在 v58 落 `config.prompt`（**1220 字符**，
dump 见 `output/_probe-cls-node.txt`）；在 v59 落 `config.code` —— 由 `risk-level.rhai:115`
的头部注释带入，而 `:111-113` 明写「**完整保留作为本脚本的口径权威来源**」。
⇒ 写成「`config.code` 不得含」：旧态空洞通过、新态假红；写成「整节点不得含」：
会毁掉「口径权威源」这条设计。**唯一正确的判别面 = `config.prompt` 是否为空。**

⚠ **同一个错误在守门测试里也有一份**：`stock_analysis_setup/mod.rs` 的
`assert!(!code.contains("你是专业风险分析师"), …)` ⇒ 原样交付会在 v59 上**必红**
（一个永久假红的门禁比没有门禁更糟：它会诱导人去改**正确的**实现）。
已改为断言 `config.prompt` 为空，并加注释说明为何不能反向写。

**负对照读数**（`output/_verify-negctl.txt`）：判定 7 项、**7/7 未通过**、退出码 **1**；
其中 ④ 报「仍有 1220 字符」—— 这正是修后断言**真有判别力**的直接证据（修前该项是 ✔）。

**判定三态化**：`true` 通过 / `false` 失败 / `null` 不适用。v58 下
「快照 `stock-analysis_v58` 已写入」应报 ○（尚未升级 ⇒ 不适用）；若按失败计，
则**正控**（v59 已落库态）会被这一项拖成假红。

**正控覆盖（「✔ 路径是否可达」逐条有来源 —— 负对照只证明会报警，证不了会通过）**：

| 判据 | 正控来源 | 状态 |
|---|---|---|
| ① version=59 | `version_gate_tests::version_gate_seeds_then_upgrades_then_skips`（空库建后取现） | ✅ **实测 `ok`** |
| ② type=`code` / ③ `matched_rules` / ④ `config.prompt` 空 / ⑤ `.result` 路径 | `seeded_template_carries_rhai_risk_level_node`（同一 `seed()` ⇒ 同一条写入路径） | ✅ **实测 `ok`** |
| ⑥ 6 条悬空键已消失 | `seeded_template_carries_v53_and_v54_changes` 的 v54 段 | ⏳ **未跑到** —— 该测试在 `:2537` 的 **v53 断言**处即失败，其后的 v54 断言**从未执行**（这正是「一条断言挡在后面」的典型） |
| ~~⑦ v53 入参已落库~~ | — | ❌ **判据本身错**：无 DB 落点，恒为 0（已改 ○，见本节 11） |
| ⑧ 快照 id = `stock-analysis_v58` | **回源码核实**：`seed_stock_analysis.rs:801` = `format!("{}_v{}", TEMPLATE_ID, existing.version)` ⇒ 取**升级前**现值 ⇒ 形态正确 | ✅ 已核（此项原先无正控，是本轮补的） |

⚠ ⑧ 的推导依赖「恰好从 58 跳到 59」；若中途有别的会话先把 DB 写到 59，快照名会是 `_v59`
⇒ 本项**假红**（脚本内已写明此边界与替代查法）。这正是「反向断言须锚定被测对象自身形态」（#588）的
正向版本：**正向断言也必须锚定被测对象的真实形态，否则会假红**。

**附带事故**：`output/_probe-cls-node.mjs` 首版把连接串当**位置参数**传给 psql
（又叠加 SQL 以换行开头 ⇒ psql 把 `\n` 当元命令而报错），失败时 `execSync` 的
异常消息**含完整明文口令**并落盘（`output/_probe-cls-node.out`，已 scrub 覆盖）。

判据 **#588 / #589 / #590**；证据 §116。

**10) 重种子对用户数据的副作用（预检结论，回答「会不会丢我改过的东西」）**

| 对象 | 重种子后 | 依据 |
|---|---|---|
| `variables` 的**自定义值** | ✅ **按 `name` 保留旧 `value`**；新变量取新默认值 | `seed_stock_analysis.rs:837` 取 `old_variables` → `:4179-4184` + `:4556-4574` 合并 |
| 模板的 `nodes` / `edges` **手工调整** | ❌ **必然被覆盖**（设计如此，非本次引入） | `:590` 注释 / `:4498` `delete_by_id` + `:4588` `insert` |
| 模板 `input_schema` / `output_schema` | ❌ 随种子重写 | `:4554-4555` |

⇒ 升级前若手工改过该模板的节点，请先自行备份；变量面板里的自定义值会被保留。

**11) 门禁重跑读数 + **v53 判据被整个推翻**（第三次修正，2026-09-20）**

**11.1 重跑读数**（`output/cargo-v59-rerun.log`）——`running 3 tests` ⇒ 确认跑的是**修正后**源码：

| 测试 | 结果 |
|---|---|
| `seeded_template_carries_rhai_risk_level_node` | ✅ **ok** —— ④「`config.prompt` 为空」这条修正后的断言在 v59 上**真的通过**（负对照下曾报 1220 字符） |
| `version_gate_seeds_then_upgrades_then_skips` | ✅ ok |
| `seeded_template_carries_v53_and_v54_changes` | ❌ **FAILED at `:2537`**（**v53 正向断言**），⇒ 其后的 v54 断言**从未执行** |
| `cargo clippy -p axagent --all-targets -- -D warnings` | ✅ **`CLIPPY_EXIT=0`**（测试代码亦通过 lint） |

**11.2 ⚠ 源码在门禁运行期间被**另一个会话**改过**：`mod.rs` mtime = `19:28:51Z`，而本会话最后一次编辑是
`18:32:40Z` ⇒ 该文件在 19:28 被改，内容是把 v53 的**正向**断言翻成**反向**（理由引
`AUDIT-codebase-review-roadmap-2026-09-19.md` §7.3「A2 接线核查」的产品决策「不接」）。
⇒ 上表的判定**属于一个已不存在的源码**，最终门禁须在现态上重跑。

**11.3 v53 判据被推翻（不是「修好」）** —— 证据链（`output/_probe-v53-anchor.txt`）：

| 环节 | 事实 |
|---|---|
| 工具名引用 | `detect_earnings_surprise` 全仓仅 3 处：定义 `seed:1267` + 两行注释。**无任何节点引用** |
| 解析路径 | 节点 `config.tools` 由 `PROFILE_TOOLS` 的**名字**经 `seed:1781` 的 `filter_map(\|tn\| tool_def_map.get(tn).cloned())` 解析 ⇒ **名字查不到即静默丢弃、无告警** |
| 是否在映射表 | `tool_def_map`（`seed:1424-1481`，37 键）**不含它** |
| 定义是否被用 | `td_earnings` 只被 `_bull_tools`（`seed:2036`，**下划线 = 刻意未使用**）引用 |
| 代码自陈 | `tools/src/tools/finance.rs:1332-1343`：「全仓**无** rhai / 节点硬接线调用它 ⇒ A2 在消费端只是一道**软防线**；**产出端不再造数才是硬解**」＋「触发前提（存在 `is_estimated=true` 的生产者）**当前已消失**：全链只剩 `false`」 |

⇒ **v53 在模板 DB 上没有可见落点**，`position(... in nodes) = 0` 是**结构性**的，与版本门无关。
该批的有效部分（产出端不再造数）**已随二进制生效**。⇒ 本节 2) 的 v53 行、9) 的「补 v53 判据」、
§⑪ B2 的「三批待落库」**均已作废**。

**11.4 已动作 / 未动作（边界）**

- ✅ **已改 `output/verify-v59-seed.mjs`**：⑦ 从「`> 0` 才通过」改为**不计分的 ○ 项**（附完整理由），
  头部与结论文案同步订正 —— 否则你跑完工作流会看到一条**永远无法消除的假红**，并据此去改正确的实现。
- ⛔ **未改 `mod.rs` / `seed_stock_analysis.rs`**：另一会话正在改这两个文件，重复施工会互相覆盖。
  但有两处**已登记、待你裁决**：
  1. **现态那条反向断言其实很弱**：因 `:1781` 的 `filter_map` 会静默丢弃解析不到的名字，
     「有节点误挂该工具」这一场景**不会**让该串进入 `nodes` ⇒ 断言**在该场景下也照样通过**。
     它只在「既加了 `tool_def_map` 条目、又挂了节点」的**完全接线**态才会触发。
     （另：该断言的依据成色与措辞偏差，见 11.5 —— 它把「**推荐**」写成了「已定案」）
  2. **`seed:666-673` 的 v53 注释前提为假**：它写「ToolDef 的入参表与 description 同样是 DB 模板
     内容的一部分」——对**这个**工具不成立（无接线路径）。上升版要求仍成立（v54/v55 共用同一次升版），
     但该段推理会误导后人以为「升版后该入参就落库了」。
- ℹ️ **另查一桩、结果无缺陷**：`filter_map` 静默丢弃本可能是个活缺陷（`PROFILE_TOOLS` 有名字
  解析不到就会被无声扔掉），但**已有覆盖** —— `seed_consistency_tests.rs:600` 的
  `profile_tools_all_resolvable` 守的正是这个面，且带两条自证（解析空间 ≥50 / 扫描面 ≥100）。
  ⇒ **不重复立项**（教训：报「新缺陷」前先查是否已有守卫）。

**11.5 最终门禁（现态源码，`output/cargo-v59-rerun2.log`）—— 全绿，但有一处必须读对**

| 门禁 | 读数 |
|---|---|
| `cargo test -p axagent --lib version_gate`（3 条） | **`running 3 tests` → `3 passed; 0 failed; 0 ignored; 420 filtered out`**（2.14s）→ **`TEST_EXIT=0`** |
| `cargo clippy -p axagent --all-targets -- -D warnings` | **`CLIPPY_EXIT=0`** |

三条各自的结论：`seeded_template_carries_v53_and_v54_changes ... ok`、
`seeded_template_carries_rhai_risk_level_node ... ok`、`version_gate_seeds_then_upgrades_then_skips ... ok`。
`mod.rs` 的 mtime 自 `19:28:51Z` 起**未再变** ⇒ 本表确实跑在现态源码上（`running 3 tests` 是三态指纹之一）。

⚠ **v53 那条由 ❌ 转 ✅ 不是「修好了」** —— 它是 11.2 那次并发翻转的直接结果：现态
`mod.rs:2540-2544` 已改为反向断言 `!raw.contains("consensus_eps_is_estimated")`，而该态
**今天必然通过**（11.3 已证该串结构性进不了 `nodes`）。⇒ 本表的绿**不能**解读为「v53 已落库」，
它只证明：**修正后的 `config.prompt` 断言（②③④⑤）与 v54 判据在 v59 上真的通过** + clippy 干净。

⚠ **翻转所引依据的成色（已回源码复验，不采信注释自述）**

| 检查项 | 结论 |
|---|---|
| §7.3 是否存在 | ✅ 存在：`AUDIT-codebase-review-roadmap-2026-09-19.md:604`（待办表 `:640`、专项块 `:676-688`） |
| 结论是否「不接」 | ✅ 是 —— 推荐「不接（无需动作 —— 现状即结论）」 |
| 措辞是否「已定案」 | ❌ **不是**。原文是「**➡️ 我的推荐：不接**」（`:683`）＋「**要让它真正可达需产品决策**」（`:682`）；`:945` 亦列为「产品决策（要不要给某分析师挂上）· 推荐不接」 |

⇒ 方向与源码事实**一致**（该工具确无接线，`:676-681` 的核查与我 11.3 的独立取证逐条吻合），
但现态注释里「**已定案**：产品决策『不接』」**把一个「推荐」写成了「已裁决」**。后果不是缺陷而是
**定性偏差**：那条反向断言等于**把一条尚未裁决的产品推荐固化成了 CI 门禁** —— 若你日后决定接线，
它会红并要求一次**有意的翻转**（这本身可接受，是棘轮式防回流）。⇒ 建议把注释措辞收敛为
「维持推荐（待产品决策）」，并保留「若接线则本断言须同步翻转」的提示。

**证据文件**：`_probe-v53-anchor.mjs|txt`（v53 锚点取证）、`_probe-silent-tool-drop.mjs|txt`（静默丢弃排查）、
`cargo-v59-rerun2.log`（**最终门禁 = 现态源码，全绿**）、`cargo-v59-rerun.log`（重跑，旧态）、
`cargo-v59-test.log`（首跑 = 更旧的快照）。

---

## ⑪ 非 JEV 条目「完成状态」总表（2026-09-20 回源码 / 真库逐项复核）

> 本节回答「除了 Jev 替代，报告发现的其它问题完成到哪一步」。
> **每条都回源码或真库复验过**，不采信报告自身的旧表述 —— 本轮即查出 §1.3 / §3 共 **3 处已过时**（见 E 组）。

### A. ✅ 已闭环

| # | 问题 | 出处 | 闭环证据（实测） |
|---|---|---|---|
| A1 | `degraded` / `warnings` **零消费方**（降级契约「设计成可见、实际不可见」） | §6.1 | 已接入 `src/lib/decisionInputDiagnosis.ts:195-208` 的 `case "cls-risk-level"`，写进 `note`（`确定性降级（N 项输入不可用）`）；新增 7 条测试（含「降级不进 `status` 计数」的反向自证）⇒ `vitest 7 passed (7)` |
| A2 | `risk-level.rhai` 的 **3 处缺陷**：① V57 扫描不补 unit ⇒ 漏配即**硬失败**；② 哨兵值泄漏进输出；③ `has()` 注释谎称兼容字符串化数字 | §2.4⑧ | 源码在位：7 个 `present(X) && has(X)`（`:167-173`）、`maybe()` 转 `null`（`:298-304`）、注释已改写（`:82-83`）。4 条回归测试在 `work_engine/executors/code_executor.rs:686 / 709 / 734 / 755`，曾全量 `269 passed; 0 failed` |
| A3 | 前端 `role` 失实（写「LLM 风险分类」） | §2.4⑧ | `decisionInputDiagnosis.ts:62` 已是 `"确定性风险分类"` |
| A4 | `v53` 的 **产出端收敛**（源不再伪造 `is_estimated`） | §⑩.2 / §⑩.11 | `crates/tools/src/tools/finance.rs:693` 读 `consensus_eps_is_estimated`；`astock-data/src/lib.rs:3557-3561` 声明「本函数自此**不再是** `is_estimated=true` 的 producer」⇒ **已随二进制生效**。⚠ 原写「只有工具入参声明未落库」**不成立**：该工具全仓无节点硬接线，声明**没有**落库路径（§⑩.11）。⚠ **且不得写作「A2 已闭合」** —— `AUDIT-codebase-review-roadmap-2026-09-19.md:681` 明令「消费者侧拒判属防守了一条没人走的路……**不可据此宣称 A2 闭合**」；闭环的只是**产出端**，消费端拒判在工作流内**仍不可达** |
| A5 | §2.4 的 **3 处错误结论**（「输出恒为空」「可零成本删除」「与 `f4_quant_adjust` 去重」） | §2.4 / §2.4⑦ | 前两条已于第二轮实测证伪并订正；「去重」已于 §⑩.4 **撤回**（三层为有意解耦） |

### B. ⏳ 代码已施工 / 待落库或待门禁

| # | 项 | 状态 | 生效条件 |
|---|---|---|---|
| B1 | 方案 ② 的 **v55**（`cls-risk-level` 下沉 Rhai） | 代码完成，**DB 未落库**（常量 55 < DB 现值 58） | 已升 `TEMPLATE_VERSION` → **59**；**跑一次股票分析**（⚠ 启动应用无效，§⑩.3）+ `node output/verify-v59-seed.mjs` |
| B2 | **v54 / v55 两批**（同困于版本门） | 同上 —— **两批一起待落库** | 同上；判定用 §⑩.2 的两条仍有效的判据。⚠ **v53 已从本组移除**（它不是「待落库」而是「在模板 DB 上无落点」，见 §⑩.11） |
| B3 | `cargo test -p axagent --lib version_gate`（3 条）+ `clippy -p axagent --all-targets` | ✅ **现态源码全绿**（`output/cargo-v59-rerun2.log`）：`running 3 tests` → **`3 passed; 0 failed`**、`TEST_EXIT=0`、`CLIPPY_EXIT=0`。⚠ **但 v53 那条由 ❌ 转 ✅ 不是「修好」**，是另一会话把断言翻成反向（§⑩.11.5）⇒ **不可读作「v53 已落库」** | 见 §⑩.8 / §⑩.11.5 |

### C. 📌 已登记 / 待裁决（未施工，均属「不该擅自改」）

| # | 项 | 为何不擅改 | 现场 |
|---|---|---|---|
| C1 | `rule-check` **下沉 Rhai** | prompt 是硬性阈值规则，须照 v55 走**影子比对 + 契约保住**；核心风险 = 契约（`violations`/`corrections`/`force_signals`） | `seed:3664`（仍是 AgentNode）；`:3677-3687` 已做「移除全部工具 + `max_tool_rounds=0`」⇒ 纯决策节点 |
| C2 | `isST` **补入** `compute_portfolio_risk` | **改口径**；原 LLM 版同样拿不到该数据 ⇒ 不启用**不算退化** | `risk-level.rhai:69-74` 自述「改造前后一致地不生效」；`astock-data/src/mcp_tools.rs:1570` 的 `stockRiskProfile` 只含 7 个数值字段 |
| C3 | `risk_source="LLM回退"` **字面量统一** | 会**展示到 UI** ⇒ 产品文案决策（注释自承「兜底者不再是 LLM」）；已核实 **TS 侧零消费方**（`src/` 内 `riskSource`/`risk_source` 零命中）⇒ 无 i18n 牵连 | `portfolio-mgr.rhai:392-394`；展示点在 `seed:4155` |
| C4 | **同一批 `stockRiskProfile` 被消费三次**（§2.4⑤） | 修法已撤回、**正确解法未定** ⇒ 需产品语义裁决 | `portfolio-mgr.rhai:403-459` / `:491-499` / `:505+` |

### D. ❌ 未动（本轮只核实，未施工）

| # | 项 | 实测状态 |
|---|---|---|
| D1 | **S4 并发上限 8 < 10 个分析师** | `stock_workflow/decision.rs:1589` 仍为 `DEFAULT_MAX_CONCURRENT = 8`（可用模板变量 `max_concurrent` 覆盖，clamp 1–32，`:1614-1617`） |
| D2 | **前端无甘特图** + 两条时间线未接线 | `src/` 内 `gantt` **零命中**；`audit_recorder` 全 `work_engine` 只有 `:889` 的 `None` 与 `:5642` 的读取、**无任何注入点**（「恒为 None」成立）；`TraceRecorder::new` 只出现在 `integration_test.rs` 与自身单测 ⇒ 生产**未接线** ⇒ `slowest_duration_ms` 永不产出 |
| D3 | **Jev 接入**（影子模式 → 放开驱动） | **代码未动**（§4 的 ②③④ 阶段均未动）。**方案 v2 已定稿** → `PLAN-jev-integration.md`：接口形态经官方文档确认为**专有 typed API**（`POST /v1/systemone`、三原语、无流式、无采样参数）；已排出 P0–P4 与 8 项风险，**待裁决 4 项**（§6）—— ⏩ **2026-09-20 后续：那 4 项已全部裁决并落地**（`PLAN-jev-integration.md` §6 裁决表；其中第 1/3 项已回改本仓代码，第 2/4 项为纯后端解析机制）。故本行「代码未动」仅描述当时点，**勿再据此推断现状** |

### E. ⚠ 本轮订正（报告曾过时 / 自相矛盾）

| # | 报告原表述 | 实测 |
|---|---|---|
| E1 | §1.3 注脚：「v48 固定 1 轮未完全生效 / 存在多轮残留」 | **不成立** —— DB 模板内 `position('bear-r3' in nodes)=0`、`position('bear-r2' in nodes)=0`；近 14 天的 r2/r3 全来自 **09-14 当天 v48 之前**的历史运行（末次 `bear-r3` = 09-14 15:59）。证据 `output/_probe-s1-rounds.txt` |
| E2 | §3 **S2**：「容器体子节点无 retry / timeout 接线」（`:6273` 裸 `dispatch()`） | **已修（非本次审计施工）** —— `dispatch_container_body_with_retry`（`:6166`）在位且被真实调用（`:5915` debate / `:6433` swarm）；裸 dispatch 只是 `execution_workflows` 未命中的 **fallback 分支** |
| E3 | §3 **S3**：「header timeout 15s vs 声明 300s，差 20 倍」 | **已修（2026-09-08 / 09-10）** —— 现为**随体积缩放** `10 + body_size/64KB × 5`、封顶 60s（`providers/src/openai.rs:998`）；且两个超时**作用域本就不同**（响应头阶段 vs body 分片），「20 倍差」部分是误读。⚠ `engine/mod.rs:6164` 的注释原写「15s」亦已过时 —— **本轮已就地改准**（注释比报告更危险：报告是快照，注释会被后人当真理），`cargo fmt -p axagent-rt-workflow -- --check` = **EXIT=0** |
| E4 | §⑩.6：「断言 `code` **不残留**旧 prompt 原文」 | **方向错** —— 该串由 `risk-level.rhai:115` 头部注释**有意保留**作口径权威源 ⇒ 原断言会在 v59 上**必红**；已改锚到旧节点专属字段 `config.prompt` 为空（本节 9 已记录） |

> **判据沉淀**：E1–E3 的共性是「**快照类报告的行号与否定性结论会被后续改动刷新**」——
> 引用审计报告里的**行号**或**否定性结论**（「无 X」「未接线」「差 N 倍」）之前，**必须回源码复验**；
> 相对稳定的是「设计与结构意图」类结论（如 S1 的「辩论容器是最大单点」、S5 的「容器耗时记录不可信」）。
