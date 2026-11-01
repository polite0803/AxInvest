# 兴业银行（601166）决策一致性审计

- **审计日期**：2026-09-13
- **样本记录**：`stock_analyses.id = 3e01fdc5-d788-4074-a554-88c0f90e2ede`（analysis_date = 2026-09-12，created_at = 1789243308401 ms，status = completed）
- **数据来源**：PostgreSQL `axinvest` 库 → `stock_analyses.decision_json` / `decision_reasoning` / `llm_decision_json` / `blackboard_snapshot`（1,666,192 字符）+ `workflow_templates['stock-analysis'] v39`（nodes / edges）+ 工作流源码
- **样本头值**：决策 减持 · 仓位 8.8% · 风险 极高风险 · 信心 44.7% · 一致性 50%

---

## 0. 数据源核对（先确认样本没取错）

| 界面显示                                   | 库中字段                             | 库中值   | 是否一致                                                        |
| ------------------------------------------ | ------------------------------------ | -------- | --------------------------------------------------------------- |
| 决策 减持                                  | `decision_action`                    | 减持     | ✅                                                              |
| 仓位 8.8%                                  | `decision_position_pct`              | 8.8      | ✅                                                              |
| 风险 极高风险                              | `decision_json.riskLevel`            | 极高风险 | ✅                                                              |
| 信心 45%                                   | `decision_json.confidence`           | 44.7     | ✅                                                              |
| 一致性 50%                                 | `debate-convergence.consensus_score` | 50       | ✅（注意：不是 `decision_json.formulaLlmAgreement`，该值为 45） |
| 细节条「决策=减持 置信=44.7 …凯利=0.178…」 | `decision_reasoning`                 | 逐字一致 | ✅                                                              |

> 结论：本次审计用的就是界面展示的这条记录。若另有目标记录，请提供股票代码或时间。

---

## 1. P0 — 风控门的仓位修正被落库链整体丢弃（头号矛盾）

### 现象

同一次分析运行里，**同一条决策出现了两个仓位值**：

| 环节                                          | 仓位     | 依据                                                                                                     |
| --------------------------------------------- | -------- | -------------------------------------------------------------------------------------------------------- |
| `portfolio-mgr`（公式决策）                   | 8.85%    | 凯利公式 `pos=8.8495`                                                                                    |
| `portfolio-risk-gate`（风控门，**链尾终值**） | **0%**   | `risk_gate.originalPositionPct=8.8` → `adjusted=true`，reasons=`R-206 单股仓位 8.8% 超过上限 0%，已下调` |
| `rule-check`（规则检查报告）                  | 0%       | 报告原文「portfolio-risk-gate 最终决策：action=减持，positionPct=0.0（被风控门 R-206 下调至 0）」        |
| `decision-explainer`（决策解释器）            | 0%       | 「风控门 R-206 将凯利建议仓位 8.8% 强制下调至 0%，执行清仓或观望策略」                                   |
| **`stock_analyses`（落库）**                  | **8.8%** | `decision_position_pct = 8.8`，`decision_json.positionPct = 8.8`                                         |
| **前端界面**                                  | **8.8%** | 头部「仓位 8.8%」                                                                                        |

**即：报告说「已清仓至 0%」，界面和数据库说「减持 8.8%」。** 用户只能看到后者。

### 根因

落库取值函数只认 `portfolio-mgr`，完全没有风控门分支：

- `src-tauri/src/commands/stock_workflow/core.rs:884` → `let decision_json = extract_decision_json(&result);`
  - 注释（`core.rs:882-883`）明确写："优先从 **portfolio-mgr** 节点本身提取决策"
- `src-tauri/src/commands/stock_workflow/decision.rs:578-615` → 第一优先级 `wf.results["portfolio-mgr"]`；`:619` 第二优先级 `quality-fallback`；**无 `portfolio-risk-gate` 分支**
- 单测 `extract_decision_json_prefers_portfolio_mgr_result`（`decision.rs:1445`）把这一行为固化

但工作流模板的声明与此相反：

- `store-result.input_var = "portfolio-risk-gate"`（nodes）
- `end-output.output_var = "portfolio-risk-gate"`
- 边：`portfolio-risk-gate → store-result → end-output`

**模板认为终值 = 风控门输出；Rust 落库认为终值 = portfolio-mgr 输出。两条链口径不一致。**

### 连带后果

1. `decision_json` 中**完全没有 `risk_gate` 元数据**（`adjusted` / `reasons` / `originalPositionPct`），前端拿不到"被风控下调过"的事实
2. `rule-check` / `decision-explainer` 走的是 `portfolio-risk-gate`，因此**报告与界面数字长期矛盾**，且报告中有「R-206 已下调至 0%」这类与实际落库不符的表述
3. 同样未生效的还有风控门其他职责：行业暴露上限、持仓数上限、空头强制卖出、组合归一化（本次样本只触发了单股上限分支）

### 触发本次 0% 的规则链（真实语义）

- `RiskTier::Extreme.max_single_stock_pct() = 0.0`（`crates/analysis-engine/src/position_limits.rs:27`）
- `PositionLimits::default().max_single_stock_pct = 20.0`（同文件 `:82`）
- `portfolio_formula.rs:540-549`：`effective_cap = min(20.0, 0.0) = 0.0` → `8.85 > 0` → `R-206` → `final_pct = 0`
- 即 **"极高风险 ⇒ 单股上限 0%"** 是硬编码的（设计意图是"不允许开新仓"），而 `portfolio-mgr` 用的是模板参数 `pos_cap_extreme = 10`。**同一语义存在两套上限：模板 10% vs 硬编码 0%。**

---

## 1.5 P0 —「深度低估 + 极高风险 + 0% 仓位」如何成立（风险分类的行业盲区）

**问**：现价 18.13 元，DCF 中性 44.13 元 / 区间 40.17–49.26 元（上行 143%），为什么结论是「极高风险 + 减持」？

**答**：因为**风险等级完全不看估值** —— 它的 6 个输入里没有任何价格或估值项。判据只有两条硬规则：

| 判据（`portfolio_formula.rs:258-261`，rhai 镜像 `portfolio-mgr.rhai:404-409`） | 兴业银行实测            | 命中     |
| ------------------------------------------------------------------------------ | ----------------------- | -------- |
| ① `负债率 > 85%` **且** `营收增速 < 0`                                         | 91.6% > 85 且 −0.3% < 0 | **★ 是** |
| ② `年化波动率 > 60%` 且 `夏普 < −1.5`                                          | 21.1% / 0.399           | 否       |

而它的其余指标**全部落在低风险档**（`t-risk` 节点 `compute_portfolio_risk` 原始输出）：

```json
{
  "annualizedVolatilityPct": 21.1,
  "debtRatioPct": 91.6,
  "maxDrawdownPct": 8.7,
  "sharpeRatio": 0.399,
  "revenueGrowthYoYPct": -0.3,
  "roeTTMPct": 4.8,
  "peTTM": 5.08
}
```

- 波动率 21.1% < 25（低风险阈值）；最大回撤 8.7% < 30；夏普 0.399 为**正**
- **唯一把这只股送进「极高风险」的是 `负债率 91.6% > 85`**
- 而银行负债科目的主体是**存款** —— 90%+ 是行业固有结构，不是"财务困境"。用工商企业的 85% 阈值套金融业 ⇒ **整个银行业必然全量命中「极高风险」**
- 第二个必要条件 `营收 −0.3%` 也只是**微幅**负增长

### 正算复现（先证明公式能复现存档值）

逐行照抄 `classify_risk` / `compute_risk_bias` / `compute_kelly_position` / action 分档，输入取上述真实画像：

| 环节                                   | 公式结果 | DB 存档                                         | 残差   |
| -------------------------------------- | -------- | ----------------------------------------------- | ------ |
| `classify_risk`                        | 极高风险 | 极高风险 (`riskSource=算法`)                    | ✓      |
| `risk_bias`                            | −0.15    | —                                               | —      |
| `effective_posterior` = 0.5065 − 0.15  | 0.3565   | `f7_free.effective_posterior=0.3589`            | ≤0.003 |
| action 分档（≥0.38 观望 / ≥0.30 减持） | 减持     | `decision_action=减持`                          | ✓      |
| 凯利仓位                               | 8.85%    | `positionPct=8.8` / `kelly_pos_uncapped=8.8495` | 0.05pt |

复现脚本：`output/tmp/replay.mjs`

### 反事实扫描

| 场景                                     | riskLevel  | risk_bias | eff.posterior | action   | 仓位  |
| ---------------------------------------- | ---------- | --------- | ------------- | -------- | ----- |
| **现状**（debt 91.6 / growth −0.3）      | 极高风险   | −0.15     | 0.3565        | **减持** | 8.85% |
| 负债率阈值 85 → 95，或金融业豁免负债率   | **中风险** | 0         | 0.5065        | **持有** | 8.85% |
| 营收增速转正（growth=+1.0）              | **中风险** | 0         | 0.5065        | **持有** | 8.85% |
| 参照：LLM 分类器独立结论（**未被采纳**） | 高风险     | −0.08     | 0.4265        | 观望     | 8.85% |

**边界极脆**：posterior = 0.5065，`risk_bias` 只要**不低于 −0.1265** 就落「观望」档（0.38 线）；当前 −0.15 仅超出 **0.0235**。

### 三个必须澄清的点

1. **8.8% 这个仓位数字与 riskLevel 无关。** 凯利算出 8.85% < 10% 上限，`apply_risk_cap` 形同虚设 —— 中/高/极高风险下凯利都输出 8.85%。**「极高风险」真正的唯一后果是把 action 标签从「持有」压成「减持」。**
2. **「0% 仓位」来自另一条链（风控门 `RiskTier::Extreme → effective_cap=0`），且被落库丢弃** —— 与第 1 节同一根因。界面从未显示过 0%。
3. **估值在这条链上只值一个因子**：`f5_valuation`，权重 0.21、sigma 0.364。它无法对抗 `risk_bias` 对 posterior 的**直接位移**（−0.15 一步跨过 0.38 档线）。「深度低估」的信息量和「极高风险」的否决权，在系统里根本不是同一个量级。

### 附带核实：40.17–49.26 这个"估值"本身的口径

`compute_valuation` 原始输出显示：

- `dcf.assumptions.basis = "当期FCF≤0，改用近5年报正净利均值×0.90归一化锚定（周期底部）"`，`is_fallback_anchor = true`
- low / mid / high = 40.17 / 44.13 / 49.26 **只是同一假设下三组永续增长率参数**的结果（low_perpetual 2.1% / mid 3.0% / high 3.9%），**不是"内在价值区间"**，更不是"理想买入价"
- 同一次输出里还并存**第三个"内在价值"**：`graham.intrinsicValue = 29.14`（格雷厄姆值）
- LLM 层（`value-investor` 节点）把它包装成 `intrinsic_value_range: "40.17-49.26元"`、`ideal_buy_price: "40.17元以下"`

⇒ 用户看到的"估值 40.17–49.26"是**净利归一化 fallback 的敏感性三档**被当成了内在价值区间（本项已在 `AUDIT-stock-business-2026-09-12.md` 估值侧缺陷②登记，此处为其实证）。

### 处置建议

| 优先级 | 动作                                                                                                                            | 性质                 |
| ------ | ------------------------------------------------------------------------------------------------------------------------------- | -------------------- |
| P0     | 风险分类增加**行业维度**：金融业（银行/保险/券商）不参与"负债率"判据，或改用行业分位                                            | 修复方案（需定口径） |
| P0     | `risk_bias` 从 −0.15 单点改为**有上限的惩罚**（≤ 0.1265 才不会跨档），或让 action 分档不因风险档跨线                            | 行为变更             |
| P1     | `classify_risk` 的 `debt>85 && growth<0` 增加**第三条件**（如 ROE<0 或连续两期负增长），避免"微幅负增长 + 行业正常杠杆"直接顶格 | 重校准               |
| P1     | `intrinsic_value_range` / `ideal_buy_price` 与 DCF 三档**语义隔离**（三档 ≠ 区间）                                              | 文案 + 字段语义      |
| P2     | 决策卡展示"风险档来源"（算法 vs LLM）与命中判据，让"极高风险"可解释                                                             | 可观测性             |

---

## 2. 其余矛盾与异常（按严重度）

| #  | 级别 | 现象                                   | 证据                                                                                                                                                                                                                                              | 影响                                                                                                                                                           |
| -- | ---- | -------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 2  | P0   | **`_exitUrgency=60` 对未持仓股票生成** | `portfolio_holdings` 中 601166 记录数为 **0**；`core.rs:1014-1019` 只有「观望」分支检查 `holding.shares > 0`，「卖出/减持」分支**不检查持仓**即返回 90/60                                                                                         | 函数注释写"如果当前股票在持仓中"，实则空仓股也被标"中紧迫减持/退出"                                                                                            |
| 3  | P1   | **quality-gate 双分支同时执行**        | edges 同时存在 `quality-gate →[acceptable] decision-explainer` 与 `quality-gate →[无handle] quality-fallback`；本次 grade=C 应走 acceptable，但 `quality-fallback` 节点**真实调用了 LLM**（usage input_tokens=43409）并输出「持有 / 0% / 高风险」 | 白烧一次 LLM 调用；且 `extract_decision_json` 第二优先级正是 quality-fallback（`decision.rs:619`），一旦 portfolio-mgr 缺 action，就会落库与公式方向相反的结果 |
| 4  | P1   | **数据质量 C 级被当成看多信号**        | `f6_signal = (dqi_score - 50)/50`（`portfolio-mgr.rhai:659-663`）；dqi=61.3 → **f6 = +0.226**，权重 0.15，直接抬高后验                                                                                                                            | 数据质量"及格即加分"；同一份 C 级数据在 `quality-gate` 里被判"可接受"、在 `portfolio-mgr` 里被当作正向因子                                                     |
| 5  | P1   | **四套风险等级并存**                   | `cls-risk-level`(LLM)=高风险 · `trader`(LLM)=中风险 · `portfolio-mgr`(算法)=极高风险 · `quality-fallback`(LLM)=高风险                                                                                                                             | 最终只有算法的"极高风险"生效（决定 f4/f6/仓位上限），而 trader 按"中风险"给了 3.47% 止损                                                                       |
| 6  | P1   | **止损/止盈两套且都落库**              | `decision_json`：stopLossPct=8 / takeProfitPct=18；`llm_decision_json`：stopLoss=17.50(3.47%) / takeProfitPct=10                                                                                                                                  | 前端两个面板数值不同；`rule-check` 自己都指出"止损档位错配"（17.50 属中风险档，极高风险档应为 ≈16.68）                                                         |
| 7  | P1   | **`posterior` 一物两义**               | `decision_json.posterior = 35.7` = (evidence.posterior 0.507 − risk_bias 0.15)×100；而 `reasoning`/`computation_logs` 写"后验=0.51"（未减 risk_bias）；`f7_free.posterior = 0.5089` 又是第三个数                                                  | 前端展示 35.7、日志展示 0.51，无法互校；action 阈值（0.3/0.38/0.48）实际比对的是 0.357                                                                         |
| 8  | P2   | **三套置信度**                         | `confidence=44.7`（界面）· `decisionConfidence=64.3` · `adjustedConfidence=42.5`；`decision-explainer` 又声称"规则检查节点给出 70% 合规性置信度"（`rule-check` 输出中并无此数）                                                                   | 界面 45% 与内部判定用的 64.3 脱节；解释器存在编造数值                                                                                                          |
| 9  | P2   | **方向相反的冲突未标记为矛盾**         | `agreementBreakdown.conflictType = opposite_direction`、`actionOk=false`、`actionScore=0`（公式"减持" vs LLM"持有"）、`total=45`；但 `isContradictory=false`，UI 显示"决策矛盾：中弱"                                                             | 最严重的反向冲突被降级为"部分一致"                                                                                                                             |
| 10 | P2   | **`data_gaps` 在决策层被清空**         | `trader` 输出 8 条缺口（同业 PE/PB/ROE 缺失、可转债摊薄未量化、北向停披、5 个节点占位符…）；`portfolio-mgr`/`decision_json` 的 `data_gaps = []`                                                                                                   | 界面"数据缺口"恒空；且 `agreementBreakdown.dataGapsScore=0` 因"无缺口"而拿不到分，与实际 7 处占位符矛盾                                                        |
| 11 | P2   | **regime-weights 半接线**              | `result.regime-weights.total_weight=0.753`（5 因子）+ `unconsumed_suggestions` 显式列出 catalyst/data_quality/risk/trade_signal 未被消费；`portfolio-mgr` 实际 `totalWeight=1.459`（11 因子），其中 `f9 money_flow=0.063` ≠ regime 建议 `0.084`   | 权重中心化只落实了 4/9；且 `regime-weights.prior=0.5` 与 `portfolio-mgr.prior=0.45(bear)` 不一致                                                               |
| 12 | P2   | **校验节点失败但流程 completed**       | `v-validate = {"results":[{"assertion_type":"exists","passed":false}],"status":"validation_failed","valid":false}`                                                                                                                                | 校验失败不阻断、不告警，界面无提示                                                                                                                             |
| 13 | P2   | **解释器规则编号张冠李戴**             | `decision-explainer.summary` 说「风控门 R-206 下调至 0%」，`rule_trace` 却写「R-204 零仓位修正」；而 `R-204` 在 `portfolio-mgr.rhai:1172/1610` 的语义是**"价格信号无信息量"**，`R-206` 的语义是"单股仓位超上限"，"极高风险禁持仓"实为 **R-200**   | LLM 自行编造规则编号与描述，报告不可作为合规凭据                                                                                                               |
| 14 | P3   | **storage 节点声明 sqlite 后端**       | `storage-result = {backend:"sqlite", collection:"analysis_history", status:"sqlite_insert"}`，而项目生产数据源为 PostgreSQL，真实记录落在 `stock_analyses`                                                                                        | 该节点是否实际写入存疑（疑似空转/双写掩盖）                                                                                                                    |

---

## 3. 结论

- **直接回答"决策有哪些矛盾"**：核心不是模型判断错，而是**同一份决策在链路上被拆成了两个真相** —— 风控门（含 R-206 清零）与落库/前端（8.8%）。
- **本次样本若按链路真实终值执行**：应为 `减持仓位 0%`（即清仓/观望），实际对外呈现的是 `减持 8.8%`，**风险敞口被高估了 8.8pt**，且用户无从得知风控门曾介入。
- 另有 2 条独立缺陷（#2 空仓股退出紧迫度、#3 质量门双分支执行）与 1 组语义污染（#4 数据质量正信号）。

### 建议处置（未改动任何代码）

| 优先级 | 动作                                                                                                                                                  | 说明                                                                                       |
| ------ | ----------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------ |
| P0     | 统一"终值"口径：让 `extract_decision_json` 优先取 `portfolio-risk-gate`（回退 portfolio-mgr）                                                         | **行为变更** —— 会把界面/落库的仓位从 8.8% 变为 0%，需先确认产品语义：风控门是否应对外可见 |
| P0     | 或反向统一：把模板的 `store-result.input_var` / `end-output.output_var` 改回 `portfolio-mgr`，并让 rule-check/decision-explainer 也只读 portfolio-mgr | 二选一，不可两套并存；同样属行为变更                                                       |
| P0     | `_exitUrgency` 增加持仓前置判断（`holding.shares > 0`）                                                                                               | 纯 bug，改动小                                                                             |
| P1     | `R-206` 的单股上限来源统一（模板 `pos_cap_extreme=10` vs `RiskTier::Extreme=0`）                                                                      | 语义决策：0% 是否真要"清零"                                                                |
| P1     | `f6_signal` 改为"数据质量低于阈值才扣分、高于阈值取 0"（避免及格即看多）                                                                              | 需回测校准，属重校准                                                                       |
| P1     | `agreementBreakdown.conflictType=opposite_direction` 时 `isContradictory` 必须为 true                                                                 | 纯逻辑                                                                                     |
| P2     | switch 节点分支互斥性核查（edge 的 handle 语义）                                                                                                      | 影响所有用 switch 的模板                                                                   |
| P2     | 把 `risk_gate` 元数据补齐进 `decision_json`，前端展示"风控已介入"                                                                                     | 可观测性                                                                                   |

---

_附：原始导出物位于 `output/tmp/`（`xy-*.json`、`n-*.txt`、`tpl-nodes.json`、`tpl-edges.json`）_

---

## 4. 复核更正（先纠错，再修）

修复前重跑证据链，**推翻了本报告 §2 的 3 条结论**，特此更正（避免按错误结论改代码）：

| 原编号 | 原结论                                            | 复核结果                                                                                                                                                                                                                                          | 依据                                                                                                                                                                                   |
| ------ | ------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| #3     | "quality-gate **双分支同时执行**"                 | ❌ 错。**只执行了一条分支，而且是错的那条**。`acceptable → decision-explainer` 从未激活；`decision-explainer` 是因为它是 `quality-fallback` 的下游才跑起来的                                                                                      | `quality-gate-result.matched_label = "low-quality"`，而 `actual_value = "C"`。真因见 §5.1：Rhai 无法解析 `_value` 标识符                                                               |
| #9     | "`isContradictory` 应为 true（反向冲突未判矛盾）" | ❌ 错。`isContradictory` 是 **trader 价格方案自洽性**标记（`targetPrice <= stopLoss`，含看空豁免），与 `agreementBreakdown.conflictType`（公式 vs LLM 方向一致性）**是两个不同语义**，本就不该联动。原判据把两者混为一谈                          | `portfolio-mgr.rhai` 的 `isContradictory` 定义 + seed v-35 注释④。真正的问题只是**档位标签偏松**（`total=45` → 文案"部分一致"），属独立项                                              |
| #11    | "regime-weights 半接线（4 因子未被消费）"         | ❌ 错。`factor_weights` 只含 5 个被消费因子 + `total_weight`（仅统计被消费）+ `total_weight_all` + `unconsumed_suggestions`，正是 **P1-F 权重政策契约**（2026-09-11 实施）的按设计形态。`f9=0.063` 也非错：`0.084 × 0.75(f1↔f9 共振衰减) = 0.063` | `regime-weights.rhai:90` `consumed_names`；`portfolio-mgr.rhai:196-216` 契约注释。**唯一真缺陷是 `prior` 字段同名不同义**（0.5 = 市况分类置信度 vs 0.45 = 决策实际先验），已在 §5.6 修 |

---

## 5. 修复实施（2026-09-13，已落地）

方向由用户裁决：**① 落库改取风控门 ② 极高风险上限统一 10% ③ A+C（行业豁免 + 真困境第三条件）④ 全修**。

### 5.1 【P0·真因】expression 型 switch 分支**恒不命中** —— Rhai 拒绝下划线开头的标识符

这是本次最有价值的发现，比"双分支执行"严重得多：**所有用 `_value` 书写的 expression 型 switch 分支，从写下的那一刻起就没生效过**。

**根因（决策性证据）**：`SwitchExecutor` 注入的脚本是 `let _value = <字面量>; <表达式>`，而 Rhai 对 `_value` 直接**解析失败**：

```text
let _value = "C"; _value == "C"   → Err(ErrorParsing(BadInput(MalformedIdentifier("_value")), 1:5))
let _v     = "C"; _v == "C"       → Err(ErrorParsing(BadInput(MalformedIdentifier("_v")),     1:5))
let __v    = "C"; __v == "C"      → Err(ErrorParsing(BadInput(MalformedIdentifier("__v")),    1:5))
let value  = "C"; value == "C"    → Ok(true)
```

（rhai 1.26 实测，脚本见 `crates/rt-workflow/src/work_engine/executors/switch_executor.rs` 的 `normalize_case_expr` 文档与回归单测。）

失败路径只打一条 `tracing::warn!` 就 `continue` ⇒ `found = None` ⇒ **恒回落 `default_case`**。于是：

| 环节                        | 值                                                                                      |
| --------------------------- | --------------------------------------------------------------------------------------- |
| `data-quality.result.grade` | `"C"`                                                                                   |
| case 表达式                 | `_value == "A" \|\| _value == "B" \|\| _value == "C"`                                   |
| `matched_label`             | **`low-quality`（default）** ← 应为 `acceptable`                                        |
| 后果                        | C 级被当低质量 ⇒ 决策尾链走 **conservative `quality-fallback`**（LLM 文案覆盖公式裁决） |

**修复**：

- `switch_executor.rs`：注入变量改用合法名 `value`（`SWITCH_INPUT_VAR`），并新增 `normalize_case_expr()` 把历史 `_value` 写法**规范化**（跳过字符串字面量、要求标识符边界）⇒ DB 里已落库的旧模板无需迁移即恢复；抽出 `build_case_script()` 以便单测直接断言"裸跑脚本能解析"。
- 新增 2 个回归单测：`expression_case_script_evaluates_and_matches`（含 `_value`/`value` 两种写法 × 命中/不命中）、`normalize_case_expr_skips_string_literals_and_non_identifiers`。
- 模板 `quality-gate` 的 case 同步改写为 `value == …`（v40）。

### 5.2 【P0·头号矛盾】落库口径统一到「本次真正走到的链尾决策」

`extract_decision_json` 的候选顺序重排为 **`quality-fallback`（D/F 档）→ `portfolio-risk-gate` → `portfolio-mgr`**，每级都要求含可用 `action` 才认；同时抽出 `unwrap_node_output()` 统一三种历史包装形态（`.result` / `.output` / 裸对象），消除重复。

**⚠️ 顺序为何不能把 gate 放第一**（本条是第一版修复的自我纠错）：

拓扑是 `portfolio-mgr → portfolio-risk-gate → rule-check → quality-gate`，即**风控门在 quality-gate 之前就已产出结果**。而 `quality-gate` 判 D/F（`default_case="low-quality"`）时会路由到 `quality-fallback`，用 AgentNode 的保守决策**替代**公式决策。若把 gate 排到最高优先级，D/F 档下就会拿「已被质量门替代的公式决策」当最终结论 —— **直接推翻 V40 修复**。

正确判据是「本次运行**实际走到的链尾决策节点**是谁」：

| 档位                                 | 实际链尾              | 落库应取             |
| ------------------------------------ | --------------------- | -------------------- |
| A/B/C（quality-gate → `acceptable`） | `portfolio-risk-gate` | **gate**             |
| D/F 或 quality-gate 自身失败         | `quality-fallback`    | **quality-fallback** |
| gate 缺位/失败（历史运行）           | `portfolio-mgr`       | **mgr**              |

正常档下 `quality-fallback` 是 `Skipped`、不写入 `results`（rt-workflow `apply_node_status_update` 仅在 `result=Some` 时写入），因此它不会在正常档误命中。

- 与模板四处声明（`store-result.input_var`、`end-output.output_var`、`rule-check` 与 `decision-explainer` 的 `contextSources`）终于一致。
- `risk_gate` 元数据（`adjusted` / `originalPositionPct` / `reasons`）随之进入 `decision_json`。
  ⚠️ **更正**：写到这里时的"前端可展示风控已介入"是**展望不是现状** —— 全仓 grep `src/` 确认
  `risk_gate` / `riskGate` **前端零消费者**。数据现已就位（风险门是否介入、原始仓位是多少都能读到），
  但**要真正让用户看到"风控曾介入"，还需新增前端消费点**（列入 §6 待办）。
- 新增 3 个单测：`extract_decision_json_prefers_portfolio_risk_gate_over_mgr`、`extract_decision_json_falls_back_to_mgr_when_gate_unusable`、`extract_decision_json_prefers_quality_fallback_over_gate`（专门锁 D/F 档顺序）。

### 5.3 【P1·纠正】空仓股不再生成退出紧迫度（⚠️ 原判 P0 有误，已降级并更正）

`core.rs` 的 `_exitUrgency` 三个分支统一加 `has_holding`（`holding.shares > 0`）前置判断。此前仅「观望」分支检查持仓，「卖出/减持」不检查 ⇒ 601166（`portfolio_holdings` 0 行）被标"中紧迫减持 60"。

**⚠️ 自我纠正**：原稿写「用户会以为自己持有该股」是**未经验证的因果推断**。全仓 grep（后端 / 前端 `src/` / 类型 / e2e）后确认：

| 事实                                   | 依据                                                                                                                                                                                |
| -------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `_exitUrgency` **只写不读**            | 全仓仅 1 个写入点（`core.rs:1035`），零消费者                                                                                                                                       |
| 前端展示的"退出紧迫度"来自**另一条链** | `SerenityCandidateCard.tsx` 读 `exitSignals.overall_exit_urgency`，生产者为 `astock-data::mcp_tools`（Serenity 专用，值为 `no_urgency/watch/caution/exit_now`），与本字段无任何关系 |

⇒ 本次改动**不是修一个用户可见的 bug**，而是防止未来消费者读到与事实相反的值。**降级为 P1，并新增待裁决项**：给它接上消费者，或连同计算一起删除。

### 5.4 【P0】「极高风险」仓位上限统一 10% + 修掉 High 档死分支

`position_limits.rs`：

| 档位      | 旧值 | 新值     | 说明                                                                                                                                                                                     |
| --------- | ---- | -------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `Extreme` | 0.0  | **10.0** | 与模板参数 `pos_cap_extreme=10`、`apply_risk_cap(极高)=10` 对齐。旧值会把「减持 8.85%」直接清零成「清仓」——极高风险的真实语义是**禁开新仓**（`forbid_new_position`）＋限仓，不是强制清仓 |
| `High`    | 35.0 | **20.0** | 旧值 35 恒被 `min(PositionLimits=20)` 覆盖，是**永不生效的死分支**                                                                                                                       |

同时在 `RiskTier::max_single_stock_pct` 上写明**语义约束：本表只能收紧、不能放宽**（唯一消费方式是 `min(全局上限, 档位上限)`），并更正 `check_new_position_with_risk` 的错误注释（原写"高风险 35% 覆盖 max_single_stock_pct"，`min` 语义下不可能向上覆盖）。

### 5.5 【P0】风险分类的行业盲区（A+C）

- **A 金融业豁免**：新增 `portfolio_formula::is_financial_sector()`，并给 `classify_risk` 增加第 7 参 `sector`；`pm_classify_risk` 同步加参（Dynamic，非字符串/空串→`None` 按非金融处理，保守）。
- **C 真困境第三条件**：`d > 85 && g < 0` 追加 `&& (roe < 0 || growth < -10)`，防单一指标顶格。
- **运行时同步**：`portfolio-mgr.rhai` 的分类镜像（生产实际路径）同样落地 A+C；模板 `portfolio-mgr.input_mapping` 新增 `stock_sector ← stock_sector`（`hooks.rs:485` 已注入该工作流变量）。
- **对本次样本的影响（⚠️ 不要照抄"持有 8.85%"这个数）**：风险档位修正把 `risk_bias` 从 **−0.15 抬到 0**（+0.15），方向是**上移** action；但 §5.6 的 f6「只惩罚」修正同时**下移** posterior（601166 的 `dqi=61.3>50`，旧公式给出 +0.226 正信号 × 权重 0.15 ≈ **+0.034**，新公式为 0）。
  ⇒ 两者**符号相反**，净效果不能靠手算断言。参照位：旧 `effective_posterior = 0.3565`（DB 复现），档线为「减持 <0.38 ≤ 观望 <0.48 ≤ 持有」。
  ⇒ 净增量若 > +0.1235 落「持有」，落在 [0.38,0.48) 则是「观望」（并按 R-207 触发**试探仓**，因 posterior 可能落 [0.42,0.50)）。
  **最终档位必须在 v40 重种子后实跑一次确认** —— 本报告不预判（铁律 4：反事实结论必须先正算复现，而不是推演）。

#### ⚠️ 5.5.1 关键词表的自我纠错：真实行业值是 `"金融"`，不是 `"银行"`

第一版把关键词写成「银行/保险/证券/信托/多元金融/非银金融/金融服务」——**实测全部落空**。用 DB 全库历史扫描 `stock_sector` 的真实取值域：

```
电子设备(x8) / 电气设备(x4) / 信息技术(x4) / 化石能源(x4) / 机械设备(x4)
休闲、生活及专业服务(x2) / 交运设备(x2) / 金融(x2) / 建材(x2)
```

即该字段是 **门类粒度**（来源 `astock-data::get_sector_info` 的 `industry` 字段，经 `hooks.rs:485` 注入），**601166 的值是 `"金融"`**。修正：关键词表加入 `"金融"`（`contains` 语义下同时覆盖非银金融/多元金融/金融服务），先细词后粗词并列。

#### 5.5.2 反事实正算（铁律 4：先证明公式能复现存档值）

用 DB 存档的 `blackboard_snapshot["t-risk"].result.content.stockRiskProfile` 真实值：

| 指标                      | 实测值 |
| ------------------------- | ------ |
| `annualizedVolatilityPct` | 21.1   |
| `debtRatioPct`            | 91.6   |
| `maxDrawdownPct`          | 8.7    |
| `roeTTMPct`               | 4.8    |
| `revenueGrowthYoYPct`     | −0.3   |
| `sharpeRatio`             | 0.399  |

阈值取 `portfolio-mgr.rhai` 的 `RISK_*` 默认值（85 / 60 / −1.5 / 40 / 45 / 5 / 65）：

| 判据             | 计算                                                                               | 结论                                                   |
| ---------------- | ---------------------------------------------------------------------------------- | ------------------------------------------------------ |
| **旧（v39）**    | `91.6>85 && −0.3<0`                                                                | **极高风险** ✅ 与 DB 存档 `riskLevel="极高风险"` 一致 |
| 新 A（金融豁免） | sector `"金融"` → 命中                                                             | 负债率子句整体排除                                     |
| 新 C（真困境）   | `roe 4.8<0`? 否；`growth<−10`? 否（仅 −0.3）                                       | C 不成立                                               |
| **新（v40）**    | `is_extreme`=false；`is_high`=false（21.1<40）；`is_low`=false（sharpe 0.399<0.5） | **中风险**                                             |

⇒ A 与 C **各自独立**排除极高风险（601166 靠 C 单独成立即可翻转），双条件交叉覆盖。新增 5 个单测，其中 `risk_classify_bank_profile_is_not_extreme` 的输入**全部取自上述实测值**，且 sector 用实测的 `"金融"` 而非猜测的 `"银行"`。

### 5.6 【P1/P2】其余一致性缺陷

| #         | 缺陷                                                                                                                                                                                                                                                                                                                    | 修法                                                                                                                                                                                                                                                                                                                                                                         |
| --------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| #4        | `f6=(dqi-50)/50` 让 C 级数据（61.3）贡献 **+0.226 看多信号**                                                                                                                                                                                                                                                            | 改为**只惩罚**：`dqi≥50 → 0`（够用即中性）、`25≤dqi<50 → 线性负分`、`dqi<25 → 0`（保留 V66 中性化）。数据质量度量证据**可靠度**，不是标的方向，不该产生正向 posterior 贡献                                                                                                                                                                                                   |
| #13       | `decision-explainer` 规则对照表过期：R-204 写作「零仓位修正」（实为「价格信号无信息量」）、R-200 写作「高风险风控否决」（实为「极高风险禁止持仓」），列出的 **R-403/R-404 无任何生产者**（全仓唯一出现处是 `divergence-log.rs:248` 的分类 `match` 分支 —— 消费端列了、产出端从未发出），且完全漏掉风控门的 R-206～R-210 | 重写为**按模块分组的权威表**（公式层 / 风控门层 / 技术面），并明确指示：R-200/201/206/207 两层各有含义、以 `reasons`/`reasoning` 文本判定归属；R-403/R-404 不得写入；未在上下文出现的规则**不得**写进 `rule_trace`。逐条核验：R-200~R-210 与 R-401/402/405 **均有真实生产者**（`portfolio_formula.rs:684/724/739`、`portfolio-mgr.rhai:1070/1074/1079`），R-403/R-404 **无** |
| #10       | `agreementBreakdown.dataGapsScore` 长期恒 0                                                                                                                                                                                                                                                                             | `compute_decision_agreement` 的 data_gaps 评分重写：双方都空 → **满分**（原 `union==0 → 1.0` 分支是永不可达的死守卫，真落到这里反而是 `unwrap_or(0.0)` 得 0 分，**惩罚了一致**）；仅一方为空 → **中性 5 分**（两侧词表本就不同：公式侧=节点级输入缺失，trader 侧=分析师自报缺口，Jaccard 恒 0 属口径错配，不是分歧）；双方非空 → Jaccard×10                                  |
| #7        | `posterior` 一物三义                                                                                                                                                                                                                                                                                                    | `bayes_note` 同时标注 `后验=<原始>` 与 `生效后验=<含 risk_bias>`；字段注释更正（本字段 = **生效后验**，此前误写"原始上涨概率"）；新增 `posteriorRaw` 供归因对照                                                                                                                                                                                                              |
| #11(部分) | `regime-weights.prior=0.5` 与 `portfolio-mgr.prior=0.45` 同名不同义                                                                                                                                                                                                                                                     | 该值实为 `market_regime` 分类器**置信度**（模板映射 `market_regime_prior ← market_regime.confidence`），字段更名 `prior → regime_confidence`。前端仅消费 `factor_weights`（`decisionInputDiagnosis.ts:269`），改名无破坏                                                                                                                                                     |

### 5.7 模板版本

`TEMPLATE_VERSION 39 → 40`（seed 内已写明本版变更与"会改变决策数值"的警告）。涉及：`quality-gate` case 表达式、`decision-explainer` 规则表、`portfolio-mgr` 新增 `stock_sector` 映射。`.rhai` 均以 `include_str!` 内联，需升版才会重新落库。

### 5.8 本批改动的**行为影响**（需要知晓）

| 影响面                                 | 变化                                                                                                                                     |
| -------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------- |
| **所有 C 级数据标的**                  | 不再被误判为低质量 ⇒ **不再走 `quality-fallback`**。此前这些标的的落库决策其实是 LLM 兜底文案，本批之后回归公式链（受风控门约束）        |
| **所有 dqi>50 的标的**                 | `f6` 从正贡献变为 0 ⇒ posterior **普遍小幅下移**                                                                                         |
| **`dqi` 在 25–50 的标的**              | 不变（仍是负分）                                                                                                                         |
| **金融业标的**                         | 不再因负债率恒判极高风险 ⇒ `risk_bias` 归零（+0.15）、action 档位上移                                                                    |
| **两处改动方向相反**                   | 风险档位修正**上移** action（+0.15），f6 修正**下移** posterior（601166 ≈ −0.034）⇒ **净档位必须实跑确认**，不能手算断言（见 §5.5 末段） |
| **`risk_gate` 触发调整的运行**         | 界面/DB 仓位现在会反映风控门的修正（此前被丢弃）                                                                                         |
| **`formulaLlmAgreement`**              | data_gaps 维度不再恒 0 ⇒ `total` 上移 ⇒ `adjustedConfidence` 随之变化；60/40 档位文案可能改档                                            |
| **含 switch 节点的所有模板（含 OPC）** | expression 型 case 从"恒不命中"变为真实按表达式路由 —— **这是本次影响面最大的一条**，建议对 OPC 各行业模板的 switch 做一次回归           |

---

## 6. 修复后仍未处置（需裁决 / 另开一轮）

| 级别 | 项                                                                                                | 说明                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                    |
| ---- | ------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| P1   | **规则编号跨模块冲突**                                                                            | `R-200`/`R-201`/`R-206`/`R-207` 在「公式决策层 portfolio-mgr」与「风控门 portfolio_risk_gate」**各自有不同语义**（同名不同义，铁律 41/25）。本轮只把对照表写准并让解释官以 `reasons` 判定归属，**未重编号**。彻底修需把风控门迁到独立号段（如 `R-3xx`），并同步 `divergence-log.rs` 的映射                                                                                                                                                                                                                                                                                                                                                              |
| P1   | **`pm_classify_risk` 是死映射**（双向实证）                                                       | 注册进引擎但**全仓零调用**：① 仓库内所有 `*.rhai` 无该标识符（含 7 个 `.worktrees` 副本）；② DB `workflow_templates` 全表扫描 `nodes`/`tool_defs` 为 **0 条**。正对照 `pm_evidence_scale` 命中 1 条（stock-analysis），证明检索有效。同一手法另发现 **`pm_covariance_decay` 同样零调用**。⇒ 生产运行时的风险分类走 `portfolio-mgr.rhai` 内联镜像（阈值取面板参数 `RISK_*`），Rust `classify_risk` 是**硬编码 V54 值的参考实现** ⇒ **改它不会改变任何一次运行的结论**（假修复陷阱）。本轮已在 `rhai_pm.rs` 注册处加警示注释并互相加注释互指；**未删**（它是风险判据唯一的语义单测载体）。待裁决：删除去重 / 把 rhai 改调它（统一真相源但会失去面板调参） |
| P2   | `R-401~R-405` 在 `divergence-log.rs:248` 被整体映射为 `technicals`，但 `R-200~R-204` 没有对应分支 | 分类器映射表不完整（新号段加入时会静默落到默认分支）；另 `divergence_logs.rs` 文档写「R-200~R-204 / R-401~R-405」与实际号段（R-200~R-210）不符                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                          |
| P2   | ~~`quality-fallback` 仍是落库第二优先级~~ → **已由 v40 解决**                                     | 新优先级把 `quality-fallback` 提到**第一位**（D/F 档链尾），gate 次之。剩一个 P3：LLM 兜底文案的来源未在 `decision_json` 里显式标记（建议加 `decisionSource: "formula" \| "risk_gate" \| "llm_fallback"`）                                                                                                                                                                                                                                                                                                                                                                                                                                              |
| P2   | **`_exitUrgency` 是只写不读的死字段**                                                             | `core.rs:1035` 是唯一写入点，零消费者；前端"退出紧迫度"来自另一条链（`astock-data::mcp_tools` 的 `overall_exit_urgency`）。本轮加了 `has_holding` 守卫，但**不是**修用户可见 bug（见 §5.3 更正）。**待裁决**：接消费者（把紧迫度真正展示）或连同计算一起删                                                                                                                                                                                                                                                                                                                                                                                              |
| P2   | **`risk_gate` 元数据已进 `decision_json`，但前端零消费者**                                        | 要让用户看到"风控曾介入、原始仓位是多少"，需新增前端消费点（`src/` 内 `risk_gate`/`riskGate` grep 为 0）                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                |
| P2   | 反向冲突的档位标签偏松                                                                            | `conflictType=opposite_direction`、`actionScore=0`、`total=45` ⇒ 文案"部分一致（中弱）"，低估了方向反转。属阈值/文案标定，非 `isContradictory` 问题（见 §4 #9）                                                                                                                                                                                                                                                                                                                                                                                                                                                                                         |
| P2   | `v-validate` 失败不阻断                                                                           | `status=validation_failed, valid=false` 时流程仍 `completed` 且无用户可见告警                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                           |
| P2   | `storage-result` 声明 `backend:"sqlite"`                                                          | 生产为 PostgreSQL；该节点是否为空转/双写未验证                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                          |
| P2   | 止损/止盈两套（#6）                                                                               | `decision_json` 8%/18% vs `llm_decision_json` 17.50(3.47%)/10%；需定"哪一套是权威"，属产品语义                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                          |
| P2   | 估值侧                                                                                            | `intrinsic_value_range`/`ideal_buy_price` 与 DCF 三档语义隔离（三档 ≠ 区间）；同输出还有第三个内在价值 `graham`                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                         |

---

## 7. 本轮自我纠错（第一版修复自身引入/暴露的缺陷）

修复过程中有 5 处必须自我纠错。记录下来，因为它们都是**同一类形态**：_验证动作本身发现了修复动作的错_。

### 7.1 落库优先级：`portfolio-risk-gate` 不能排第一（推翻第一版）

第一版把 gate 放在最高优先级。复核拓扑时发现 `portfolio-mgr → portfolio-risk-gate → rule-check → quality-gate`，**风控门在 quality-gate 之前就已产出**，而 quality-gate 判 D/F 时会用 `quality-fallback` 的保守决策替代公式决策 ⇒ gate 排第一会把「已被替代的公式决策」重新抬成结论，**推翻 V40 修复**。已改为 `quality-fallback → gate → mgr`，并新增 `extract_decision_json_prefers_quality_fallback_over_gate` 锁定。

> 教训：判「谁是链尾」不能只看模板声明（模板四处都声明 gate），要看**当前运行实际走通的那条路径**。声明是静态的，路由是动态的。

### 7.2 行业关键词表：真实值是 `"金融"`，我写的 `"银行"` 一个都不命中

见 §5.5.1。这是本轮最值得记的一条 —— **修复本身是"死代码"**：A 豁免写对了逻辑，却因为关键词表与真实取值域不匹配而**从不生效**。而它不会报错、不会告警，只是静默不触发（铁律 6「死映射恒空不报错」的又一变体）。

> 教训：涉及**外部数据取值域**的判据，必须先用真实数据枚举取值域，再写匹配规则；不能用"我以为是"的分类名。本例 `assert!(is_financial_sector(Some("银行")))` 这种自造输入的测试**全绿也说明不了任何事**。

### 7.3 断言与实现冲突：`x._value` 该不该改写

`normalize_case_expr` 的边界判定把 `.` 视为非标识符字符，于是 `x._value → x.value`。第一版断言要求「不改写」，实测冲突。判断依据：Rhai 的字段访问与裸标识符**共用同一套标识符规则**，`x._value` 同样解析失败 ⇒ 改写是严格更优（保留原样只会留下必然求值失败的表达式）。**改断言、保留实现**。

### 7.4 `pm_classify_risk` 是死映射（连带发现 `pm_covariance_decay`）

见 §6。触发点是核对「我改的 Rust `classify_risk` 到底生不生效」—— 结论是**不生效**，运行时走 rhai 内联镜像。已在注册处加警示注释，避免后来者踩「改了没反应」的假修复陷阱。

> 教训：改任何 `analysis-engine` 的 Rust 函数前，先 grep「谁调用它」+「那个调用点有没有被消费」两跳。本项目存在「Rust 实现 ← rhai 函数注册 ← rhai 脚本调用」三层，只查前两层会得出错误结论。

### 7.5 长期红测的定性：`..._falls_back_to_pm_wrapper_when_result_missing`

按现行契约重写为 `extract_decision_json_hardens_completed_pm_without_action`。定性依据（无 git）：worktree `v297` / `pre992947df` 中**该测试存在而 `has_usable_action` 尚不存在** ⇒ 失败早于本次改动，是 V71（2026-09-11）引入 `has_usable_action` 后**未同步的历史遗留**。

> 教训：一条长期红测会让整个 `cargo test` 退出码非 0，掩盖新引入的失败。`cargo test <filter>` 按名字过滤跑，能绕开它 —— 但这样做会**失去发现"新失败被老失败掩盖"的能力**。红测必须被定性（改实现 or 改断言），不能挂着。

### 7.6 「落地后可展示」被我当成了「已展示」—— 输出侧消费点没穷举

写 §5.2/§5.3 时，我给两处改动附了**未经验证的因果推断**：

| 我写的                                        | 核实结果                                                                                                                 |
| --------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------ |
| `_exitUrgency` 会导致「前端提示有仓位待退出」 | ❌ 该字段**只写不读**；前端展示的紧迫度来自 `astock-data::mcp_tools` 的另一条链（`overall_exit_urgency`，Serenity 专用） |
| `risk_gate` 元数据「前端可展示风控已介入」    | ❌ 前端 `risk_gate`/`riskGate` 零消费者 —— 数据就位，但**没人读**                                                        |

两处都已回改（§5.3 降级并更正、§5.2 加"展望不是现状"标注），并各自列入 §6 待裁决。

> 教训：**「字段进了产物」≠「有人消费」**。写"某字段会导致某现象"之前，必须 grep 到那个现象真正的读取点；grep 不到就只能写"数据已就位，尚无消费者"。
> 这与本轮 §7.4（死映射）是**同一条铁律的两端**：_生产者存在_ 和 _消费者存在_ 是两件独立的事，任何一侧缺失，改动都不产生运行时效果。

---

## 8. 本批验证

| 门禁                                                                                                     | 范围                                       | 结果                                                       |
| -------------------------------------------------------------------------------------------------------- | ------------------------------------------ | ---------------------------------------------------------- |
| `cargo fmt --all -- --check`                                                                             | 全仓                                       | ✅ 无 diff                                                 |
| `cargo clippy -p axagent-analysis-engine -p axagent-rt-workflow -p axagent --all-targets -- -D warnings` | 3 包                                       | ✅ `Finished`，**0 warning / 0 error**                     |
| `cargo test -p axagent-rt-workflow --lib switch_executor`                                                | 新增回归                                   | ✅ **2/2**（含此前失败的断言）                             |
| `cargo test -p axagent-rt-workflow --test rhai_syntax_check`                                             | 3 个改动过的 `.rhai`                       | ✅ **2/2**                                                 |
| `cargo test -p axagent-analysis-engine --lib`                                                            | 全 crate                                   | ✅ **432 passed / 0 failed**（含新增 5 项）                |
| `cargo test -p axagent --lib stock_workflow::decision`                                                   | 落库口径                                   | ✅ **24 passed / 0 failed**（含新增 3 项 + 重写 1 项红测） |
| DB 正算复现                                                                                              | 601166 档案值                              | ✅ 旧判据复现「极高风险」，与 DB 一致                      |
| DB 取值域实测                                                                                            | `stock_sector`                             | ✅ 9 个真实值全枚举，修正关键词表                          |
| DB 模板扫描                                                                                              | `pm_classify_risk` / `pm_covariance_decay` | ✅ 0 命中（正对照 `pm_evidence_scale` 1 命中）             |
| DB 产物形状实测                                                                                          | `quality-fallback` / `t-risk`              | ✅ 扁平 `{content,model}` / 双重编码，3 次运行全为 C 级    |
| 全仓 grep                                                                                                | `_exitUrgency` / `risk_gate` 消费者        | ✅ 均 0 命中 ⇒ §7.6 更正成立                               |

---

## 9. 回到你最初的问题：「现价 18.13、估值 40.17–49.26、却是极高风险 + 0% 仓位」是怎么成立的

这条结论是**四层机制叠加**出来的，每一层都独立看都"合理"，串起来就成了自相矛盾：

| # | 层            | 机制                                                                                                                                      | 为什么看起来不合理                                                                                           |
| - | ------------- | ----------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------ |
| 1 | **风险分类**  | `debt 91.6>85 && growth −0.3<0` 命中硬规则 ⇒ 极高风险。该硬规则**不看估值**，也不认行业 —— 银行负债主体是存款，负债率 90%+ 是行业固有结构 | 「便宜」这个理由是**估值链**给的，而风险分类只看 6 项量化指标，两者根本没有交集                              |
| 2 | **先验/后验** | 极高风险 ⇒ `risk_bias = −0.15` **直接位移** `effective_posterior`（0.5065 → 0.3565），一步跨过「减持」档线（0.38）                        | 估值链的全部结论（DCF/Graham）最终只进 1 个因子 `f5`（权重 0.21），而 `risk_bias` 是**加法直移**，量级不对等 |
| 3 | **风控门**    | 极高风险 ⇒ `RiskTier::Extreme` 仓位上限 0 ⇒ 风控门把仓位清 0，并写 `R-206` 理由                                                           | 「极高风险」在此又变成一个**仓位**规则，同一档位在三处各有一套上限（0 / 10 / 10）                            |
| 4 | **落库**      | 落库链只认 `portfolio-mgr` ⇒ 风控门的 0% **被整体丢弃**，DB/界面拿到的是公式层的 8.85%                                                    | 报告说「已下调至 0%」、界面说 8.85% —— 同一次运行两套真相                                                    |

**四层里三处是缺陷**（1 的行业盲区、3 的三套上限、4 的丢弃），只有 2 是设计意图（风险厌恶直接降 posterior）。
诊断卡住的地方在于：这条链上「估值」和「风险」根本不共享输入，**所以问"便宜为何还极高风险"这个问题，在两条链各自的坐标系里都没有答案** —— 必须先把两条链的输入集合列出来，才能看到它们不相交。

### 修复后的预期（待实跑确认）

| 项          | 修复前（DB 存档）                                          | v40 之后                                              |
| ----------- | ---------------------------------------------------------- | ----------------------------------------------------- |
| 质量门路由  | `grade="C"` 却 `matched_label="low-quality"` ⇒ 走 LLM 兜底 | C ⇒ `acceptable` ⇒ 回归公式链                         |
| 风险档位    | 极高风险（负债率硬规则）                                   | **中风险**（行业 `"金融"` 豁免 + 真困境第三条件）     |
| `risk_bias` | −0.15                                                      | 0                                                     |
| action      | 减持                                                       | 上移（**持有 或 观望+试探仓**，取决于 f6 修正的净额） |
| 仓位        | 风控门清 0%，但被丢弃 ⇒ 界面显示 8.85%                     | 风控门不再清零；落库取风控门值 ⇒ 报告与界面一致       |
| 退出紧迫度  | 空仓却标"中紧迫减持 60"                                    | 不再生成（该字段当前无人读，见 §5.3）                 |
