# AUDIT — 300642 同日两次分析结论不一致 + 估值区间语义 — 2026-09-22

> 触发：用户报告「今天两次透景生命(300642) 分析，一次结论是持有，一次结论是观望，
> 而且巴菲特估值区间从十几块到四十几块，这么大的区间有什么实际意义吗」。
> 本文**不改任何生产代码**，只做取证与归因；需产品语义决策的项单列待裁决（见 §7）。
> 取证数据源：PG `stock_analyses` 两条记录（`8d8644a3…` 11:49:34 / `511abde4…` 11:33:19）。

---

## 1. 结论摘要（先给答案）

| 用户直觉 | 判定 | 依据 |
|---|---|---|
| 「两次结论不一样」 | **成立，但根因不在模型判断** —— 两次**底层方向档都是「观望」**，差异只发生在**展示层派生** | §3.1、§4 |
| 「估值区间十几块到四十几块没有实际意义」 | **成立** —— 该区间是「增长率假设敏感性带」，不是内在价值区间；且它**不是决策用的那条腿**，还与同节点的格雷厄姆结论差 39 倍 | §5 |

三条独立缺陷叠加，共同制造了用户看到的现象：

- **D1（P0，展示层）** `display_action` 把「观望 + 有仓位」渲染成「持有」，
  而「有无仓位」由 3% 试探仓决定，试探仓又由 **LLM trader 的定性方向词**控制
  ⇒ 同一个「观望」被渲染成两个不同标签。
- **D2（P0，公式层）** `compute_graham_value` 的增长率口径与格雷厄姆公式定义不一致
  （传小数、公式要求百分数），且**同一表达式内 `4.4 / bond_yield` 用的是百分数** ——
  同式两套口径。后果：增长溢价项被压 100 倍，倍数由应得的 44.06 掉到 8.856。
- **D3（P1，语义/展示层）** 面板显示的 DCF 三档区间与决策实际消费的估值证据**不是同一条腿**：
  f5 的负值 88% 来自 PE 5 年历史分位腿（占比 20% 权重却给 −0.998），DCF 腿贡献 +0.361（正分）。

---

## 2. 取证方法与复现（模型可信性验证）

按 `scoring-chain-counterfactual-audit` Step 1~2：先逐环节抄公式 → 再用存档输入正算 → 对账。

PG 入口：`node scripts/pg-connect.mjs sql "<SQL>"`（口令经 `~/.axagent` 解密，**不进 argv**）。
`stock_analyses.blackboard_snapshot` 是 **text 列**，需 `::jsonb`；节点输出是**双重编码**
（`content` 是字符串化 JSON），必须 `(…->>'content')::jsonb` 再续路径，否则静默取到 null。

**正算复现结果（误差 ≤ 0.1，达标）**：

| 量 | 公式（文件:行号） | 11:33 存档 | 11:33 正算 | 11:49 存档 | 11:49 正算 |
|---|---|---|---|---|---|
| `f5_signal` | `portfolio-mgr.rhai:739-753, 858` | −0.218 | **−0.2176** | −0.218 | **−0.2176** |
| `bayes_confidence` | `portfolio_formula.rs:462-484` | — | 61.80 | — | 60.90 |
| `disagreement_mod` | `portfolio-mgr.rhai:1691-1693` | — | 0.70 | — | 1.00 |
| `final_confidence` | `portfolio-mgr.rhai:1729` | **43.3** | **43.26** ✅ | **60.9** | **60.90** ✅ |
| `effective_posterior` | `posterior + risk_bias(−0.08)` | 38.2 | 0.382 | 39.1 | 0.391 |

⇒ 复现成立，后续归因可用。

---

## 3. 事实：两次运行逐字段对账

### 3.1 机器轴（后端原生字段）—— **两次一致**

| 字段 | 11:33 | 11:49 |
|---|---|---|
| `action`（方向档，机器轴） | **观望** | **观望** |
| `stock_analyses.decision_action`（落库列） | **观望** | **观望** |
| `posteriorRaw` | 46.2 | 47.1 |
| `posterior`（生效后验 = raw + risk_bias） | 38.2 | 39.1 |
| 档位判定 | ≥0.38 → 观望；<0.48 → 非持有 | 同 |
| `kelly_pos_uncapped` | 0.0 | 0.0 |
| `odds` / `oddsSource` | 1.5 / 波动率fallback | 1.5 / 波动率fallback |
| `riskLevel` / `maxPositionPct` | 高风险 / 35 | 高风险 / 35 |

两档都落在 `[ACTION_WATCH_THRESHOLD(0.38), ACTION_HOLD_THRESHOLD(0.48))`
（`portfolio-mgr.rhai:119-123, 1890-1906`）⇒ **方向判断完全相同**。

### 3.2 展示轴与仓位 —— 差异全在这里

| 字段 | 11:33（显示「观望」） | 11:49（显示「持有」） |
|---|---|---|
| `positionPct` | **0.0** | **3.0** |
| `positionState` | EMPTY | **HOLDING** |
| `decision_trail` | `[]`（空） | `R-207 ACTIVATED 试探仓激活…仓位=3.0%` |
| reasoning 文案 | `决策=观望 置信=43.3 仓位=0.0%` | `决策=持有 置信=60.9 仓位=3.0%` |
| confidence | 43.3 | 60.9 |
| `expectedHoldingDays` / `timeHorizon` | 40 / mid | 5 / short |

### 3.3 输入层：确定性输入**逐位相同**，非确定性输入**全部来自 LLM 节点**

| 输入 | 11:33 | 11:49 | 性质 |
|---|---|---|---|
| DCF `assumptions`（growth/fcf_per_share/折现率） | 0.177819 / 0.981859 / 0.10 | **逐字节相同** | 算法，确定 |
| `dcf.low/mid/high` | 18.08 / 26.39 / 41.51 | **相同** | 算法，确定 |
| `valuation_pe_percentile` | 99.91142604074402 | **相同** | 算法，确定 |
| `current_price` / kline / money_flow / 龙虎榜 / 公告 | 相同 | 相同 | 数据源 |
| `dcf.upsidePct` / `graham.upsidePct` | 22.6 / −96.9 | 相同 | 算法，确定 |
| **f2 consensus σ** | −0.1 | −0.3 | **LLM** |
| **f3 catalyst σ** | −0.155 | +0.125 | **LLM** |
| **f7 trade_signal σ** | −0.362 | 0.0 | **LLM** |
| **f11 pace σ / w** | 0.29 / 0.052 | 0.0 / 0.0 | **LLM** |
| **dqi_score / consensusScore** | 60.6 / 45 | 60.1 / 35 | **LLM** |
| **trader_direction** | **看空** | **中性** | **LLM** |
| **trader_target_price** | 19.2 | **null** | **LLM** |
| **trader_confidence** | 75 | 62 | **LLM** |
| **risk_disagreement** | **40** | **缺失** | **LLM** |

⇒ 「同一天同一标的、确定性输入逐位相同，结论/置信度却不同」= **纯 LLM 采样抖动**，
不是数据问题、不是模板版本问题（两次 `template_version` 均为 75）。

---

## 4. 根因 A：为什么被渲染成两个不同结论

### 4.1 展示层派生规则（直接来源）

`portfolio-mgr.rhai:2298-2315`：

```
let display_action = if final_action != "持有" && final_action != "观望" { final_action }
    else if position_state == "EMPTY" { "观望" } else { "持有" };
```

`position_state` 由 `position_pct <= 0` 决定（`:2288-2296`）。
⇒ **同一个 `final_action = 观望`**：仓位为 0 → 显示「观望」；仓位 > 0 → 显示「持有」。

该派生是 2026-09-21 为修「卡片 Tag 显示观望、点开文本写持有」而加的（`:2298-2308` 注释）。
**它修掉了一个面内的矛盾，制造了跨运行的不一致** —— 按下葫芦浮起瓢。

### 4.2 3% 试探仓的激活守卫，读的是 LLM 的定性词

`portfolio-mgr.rhai:1867-1869`：

```
let probe_position_pct = if !weights_collapsed && untrusted_count == 0
    && !trader_bearish && !trader_dir_bearish
    && posterior >= 0.42 && posterior < 0.50 && odds > 0.001 { 3.0 } else { 0.0 };
```

逐个守卫核对（两次 `weights_collapsed=false`、`untrusted_count=0`、`posterior∈[0.42,0.50)`、
`odds=1.5>0.001` 全部通过）：

| 守卫 | 11:33 | 11:49 | 判定 |
|---|---|---|---|
| `trader_bearish`（targetPrice 低于现价 ≥15%，`:1399-1407`） | target 19.2 / 现价 21.53 → −10.8%，**不命中** | target 缺失 → 不命中 | 非差异源 |
| **`trader_dir_bearish`**（`:1367-1384`） | `trader_direction="看空"` → **true** | `trader_direction="中性"` → **false** | **← 唯一差异源** |

⇒ **LLM 把方向写成「看空」还是「中性」，经「3% 试探仓」这条旁路，
把同一个「观望」渲染成了「观望」或「持有」。**

直接证据：11:33 的 `decision_trail` 为空（既无 ACTIVATED 也无 SKIPPED）⇒ `probe_position_pct==0`；
11:49 的 `decision_trail` 只有一条 `R-207 ACTIVATED 试探仓激活:posterior=0.471∈[0.42,0.50),odds=1.5,仓位=3.0%`。

### 4.3 附带的第二个不稳定：置信度 43.3 vs 60.9

| 步 | 11:33 | 11:49 |
|---|---|---|
| `bayes_confidence` | 61.80 | 60.90 |
| `risk_disagreement` | **40** | **缺失** |
| `disagreement_mod = clamp((100−rd)/100, 0.7, 1.0)` | **0.70** | 1.00 |
| `final_confidence = clamp(bayes×mod, 0, cap(C=75))` | **43.3** | **60.9** |

⇒ 17.6pt 的置信度落差里，**大部分来自 `risk_disagreement` 这个 LLM 产出量的有无**，
而两次的贝叶斯强度几乎相同（61.80 vs 60.90）。
**「方向一致、置信度差 17 个点」同样是展示层的口径抖动，不是证据强度的真实变化。**

---

## 5. 根因 B：估值区间为什么宽、以及它有没有用

### 5.1 这个区间是什么

面板「估值结论」里的区间 = `value-investor` 输出的 `intrinsic_value_range`，
其 prompt 明令**直接引用** DCF 区间（`agency_experts/stock-analysis/custom/value-investor.md:69,140`）。

DCF 三档生成（`crates/astock-data/src/mcp_tools.rs:2798-2820`，常量 `:2073-2076, 2083-2092`）：

| 档 | 增长率 | 永续增长率 | 值 |
|---|---|---|---|
| low | `growth × 0.6` = **10.67%** | `p × 0.7` = 2.1% | **18.08** |
| mid | `growth` = **17.78%** | `p` = 3.0% | **26.39** |
| high | `growth × 1.5` = **26.67%** | `p × 1.3` = 3.9% | **41.51** |

现价 21.53；`fcf_per_share = 0.981859`（同一锚），`discount_rate = 0.10`，`forecast_years = 5`。

**上界 / 下界 = 41.51 / 18.08 = 2.30 倍**，全部来自「同一锚 × 三组增长率假设」。
⇒ 它的正确语义是「**增长率假设敏感性带**」（等权、无概率），
**不是「内在价值区间」**；用它读「公司值 18~42 元」是把参数扫描当成了估值精度。
中位 26.39 隐含 +22.6% 低估，而决策端把它当负向证据（见 5.3），**方向相反**。

### 5.2 同一节点内两个模型差 39 倍

| 模型 | 内在价值 | 相对现价 |
|---|---|---|
| DCF `mid` | **26.39** | **+22.6%**（低估） |
| 格雷厄姆 | **0.67** | **−96.9%**（高估） |

格雷厄姆的 0.67 元 / 折合 EPS ≈ 0.0757 元（与同报告 PE 283.92 反算的 21.53/283.92 = 0.0758 一致，
⇒ **EPS 口径与归一化无误**）。问题在**公式的增长率口径**：

`mcp_tools.rs:3036`：`eps * (8.5 + 2.0 * g) * 4.4 / bond_yield`，其中 `g` 传的是**小数**（0.1778）。

- 格雷厄姆公式 `V = EPS × (8.5 + 2g) × 4.4 / Y` 的 **g 是百分数**（17.78）⇒ 增长溢价项应为 35.56；
- 实传小数 ⇒ 溢价项 **0.356**，倍数 8.856 而非 44.06（**差 ≈5 倍**）；
- **同式内 `4.4 / bond_yield` 的 `bond_yield` 是百分数**（4.4 = 4.4%）
  ⇒ 一个表达式里两个「百分数量」用了两套口径，**内部自相矛盾**，属口径缺陷而非建模选择。

**影响量化（按判据要求给出「够不够翻转结论」）**：修正后 g=17.78 → `V = 3.34 元`、`upsidePct ≈ −84.5%`，
`pm_saturate(−84.5, 40)` = −0.679（原 −0.708）⇒ `Δσ = +0.030` ⇒ `Δf5 ≈ +0.0094`
⇒ `Δposterior ≈ 0.0094 × 0.165 / 1.41 ≈ **0.0011**`。
⇒ 本项属 **「恢复正确性」，不是「改变结论」**：修正后格雷厄姆仍然判高估，f5 仍为负。

### 5.3 关键：面板显示的区间**不是决策消费的那条腿**

f5 估值因子由**三腿加权融合**（`portfolio-mgr.rhai:739-753`，权重 0.56 / 0.24 / 0.20）：

| 腿 | 输入（运行时实参） | 映射 | 加权贡献 |
|---|---|---|---|
| DCF | `valuation_dcf_upside = 22.6` | `pm_saturate(22.6, 40)` = **+0.361** | +0.2022 |
| 格雷厄姆 | `valuation_graham_upside = −96.9` | `pm_saturate(−96.9, 40)` = **−0.708** | −0.1699 |
| **PE 5 年历史分位** | `valuation_pe_percentile = 99.911` | `(50−99.911)/50` = **−0.998** | **−0.1996** |

融合 `= (0.2022 − 0.1699 − 0.1996) / 1.0 = −0.16735`，
`× fscore_mult(8≥7 → 1.3) × moat_mult("无" → 1.0)` = **−0.2176** ⇒ 与存档 `f5 σ = −0.218` 一致 ✅

⇒ **f5 的负值主要来自 PE 分位腿（−0.998，近满值）+ 格雷厄姆腿，而 DCF 腿（也就是面板那个区间）贡献的是正分。**
用户「这个区间没有实际意义」的感受有精确的机制支撑：**它既不是决策依据，又与决策方向相反。**

（PE 分位 99.911 本身是有信息量的 —— 透景生命当前 PE 处于自身 5 年分布的第 99.9 百分位，
即历史最贵区间，与叙事一致。问题不在它，在于**面板没显示它**。）

---

## 6. 影响面与风险

| 项 | 影响 | 证据 |
|---|---|---|
| D1 展示标签抖动 | 用户层面**最高**：同一标的同一天给两个不同操作名；不可复盘 | §3.2、§4.1–4.2 |
| D1 连带：置信度抖动 | 17.6pt 落差会被读成「结论可信度变了」，实际是同证据强度 | §4.3 |
| D2 格雷厄姆口径 | 所有高增长标的的内在价值被系统性压低 ~5 倍；`pm_saturate` 饱和后**对最终结论影响可忽略**（Δposterior≈0.0011），但**污染 LLM 论据**（`buffett_verdict` / `risk_flags` 引用该值） | §5.2 |
| D3 展示/决策脱节 | 用户无法从面板看出「为什么估值说低估 22.6% 却不买」 | §5.3 |

**未受影响、无需处置**：DCF 三档计算、PE 分位、t-scoring、风控档位、仓位上限 —— 两次逐位相同。

---

## 7. 分级建议（**清单原文，保留历史留痕** —— 实际处置见 §9，勿据本节判断当前代码状态）

### P0（客观错误，正确答案唯一）

| 编号 | 缺陷 | 位置 | 修法 | 是否翻转结论 |
|---|---|---|---|---|
| P0-1 | `display_action` 由「仓位有无」派生标签，使中性档出现两个名字 | `portfolio-mgr.rhai:2298-2315`（+ 前端 `src/lib/stock-analysis-utils.ts::resolveDisplayAction` 须同步） | 三选一：① 取消派生，展示层独立渲染 `positionState`（回归 V76 两轴正交）；② 改为**稳定**派生（不依赖 LLM 词的旁路）；③ 中性档统一显示「观望」，仓位另列 | 是（且是**本问题唯一必须改**的一处） |
| P0-2 | `compute_graham_value` 增长率口径与公式定义不一致（小数 vs 百分数），且与同式 `4.4/bond_yield` 口径冲突 | `crates/astock-data/src/mcp_tools.rs:3036` | 改为 `(8.5 + 2.0 * g * 100.0)` 或等价百分数口径；同步改 `GrahamAssumptions` 注释与回归测试 | 否（Δposterior≈0.0011；但**必须**同时核对全部消费方与 LLM 论据） |

### P1（策略选择，需你拍板）

| 编号 | 议题 | 备选 |
|---|---|---|
| P1-1 | **DCF 三档的展示语义** —— 现在以「区间」形态出现，会被读成内在价值范围 | ① 改标签为「增长率敏感性带（假设 10.7%–26.7%/永续 2.1%–3.9%）」；② 收窄倍率（如 0.75/1.25）使带宽可控；③ 按概率加权输出单点+区间；④ 只在 `high/low` 落在合理区间时展示 |
| P1-2 | **DCF / 格雷厄姆 / PE 分位三腿的展示口径** —— 面板只显示 DCF，决策主要看 PE 分位 | 把三腿都显示（含各自 σ 与贡献），让「为什么低估 22.6% 却不买」可见 |
| P1-3 | **LLM 因子抖动治理** —— 16 分钟内两次运行，f2/f3/f7/f11 符号不同、trader 方向从「看空」变「中性」 | 固定 `seed` / 降温度 / 对 LLM 因子多次采样取中位 / 落 `runId+promptHash` 以便复盘 |
| P1-4 | `risk_disagreement` 缺失时 `mod=1.0`（**无折价**） | 缺失应取**保守档 0.7** 而非 1.0（现语义：分歧度测不到 ⇒ 更可信，方向可疑） |

### 已否决 / 不建议

- **不建议**用「回调 `pm_saturate` 常数」来压 f5 —— 三腿都未饱和到失效，且 f5 不是本问题的瓶颈
  （本问题在展示层；参考 `AUDIT-dcf-cashflow-anchor-2026-09-21.md` 已量化过同类调参对 action 无效）。
- **不建议**把 PE 分位腿降权 —— 它是本次唯一在「估值」维度上给出**与价格无关**判断的腿（分位 99.9 有经济含义）。

---

## 8. 取证脚本与改动清单

**新增（未进版本库）**：
- `output/tmp/dump300642.mjs`、`output/tmp/probe-dump.mjs` —— 快照对账/探针（只读）
- `output/tmp/q*.txt`、`output/tmp/bb300642.txt` —— SQL 取证输出
- 本文档

**改动清单**：**无生产代码改动**（本次为只读审计）。

**遗留风险**：
1. `_raw.portfolio-mgr.input_params` 里已含全部运行时实参（`valuation_pe_percentile` 等），
   说明「运行时值可对账」已具备 —— 后续审计不必再反解（欠定）。
2. `stock_analyses` 未落 `seed` / `promptHash`，**无法区分**「LLM 抖动」与「prompt 变更」。
   建议 P1-3 一并补。

---

## 9. 修复实施记录（2026-09-22 当晚）

> §7 是当时的建议清单，**保留原文不改**（历史留痕）。本节记录实际落地内容。

| 编号 | 处置 | 位置 | 内容 |
|---|---|---|---|
| P0-1 | **已修：取消派生** | `portfolio-mgr.rhai`（删 `display_action` 块，`reasoning` 回归 `final_action`）＋ `src/lib/stock-analysis-utils.ts::resolveDisplayAction`（改为恒等，签名保留）＋ 4 处调用点收敛传参 | 展示档 = 方向档。**根因定性为循环判据**：用本次决策算出的建议仓位反推本次展示名。切断「LLM 措辞 → 试探仓 → positionPct → positionState → 展示档」三级旁路；仓位/持仓状态仍由 `positionState` 独立展示 |
| P0-2 | **已修：量纲** | `crates/astock-data/src/mcp_tools.rs::compute_graham_value` → `(8.5 + 2.0 * g * 100.0)` | 影响面**已量化**（§9.1） |
| P1-1 | **已修：口径标注** | `src/components/stock-analysis/DashboardReportPreview.tsx` ＋ `analysis-engine/src/dashboard_report.rs`（md / html 两处）＋ 11 语言新 key `stockAnalysis.dashboard.intrinsicValueBandNote` | 区间下方明写「三档 = 同一现金流锚 × 增长率假设 ×0.6/×1.0/×1.5（永续 ×0.7/×1.3）的**假设敏感性带**，非概率区间；仅中值为点估计」 |
| P1-4 | **已修：保守档** | `portfolio-mgr.rhai::disagreement_mod` 缺失分支 `1.0 → 0.7` | 修复「数据缺失 ⇒ 置信度反而更高」的倒挂（60.9 > 43.3） |
| P1-2 | **未改** | — | 三腿展示改造属 UI 重构，超出本次范围 |
| P1-3 | **未改** | — | LLM seed / 温度治理属模型调用层策略 |

**同步清理（防注释撒谎）**：8 处写着「展示档由 `(action, positionState, positionPct)` 派生」的
注释已更新 —— `AnalysisHistoryButton` / `HistoricalAnalysisPanel` / `DecisionBanner` /
`StockAnalysisChatIndicator` / `DailyReviewPanel` / `DecisionComparisonPanel`（2 处）/
`ExperimentTrail` / `TradePanel` / `types/stock-analysis.ts`。

### 9.1 P0-2 影响面量化（历史落库反解，**非估算**）

方法：对每个历史样本，由 `graham.intrinsicValue` 与 `assumptions.growth` 反解 EPS
（EPS 在新旧公式两侧相消，故无需 vendor 数据），按新乘数重算 `intrinsicValue` / `upsidePct`，
再过 `pm_saturate(r, 40)`（`k = 40` 由 300642 的 −96.9 → −0.708 反解）得腿信号。
脚本：`output/tmp/graham-impact.mjs`（输入 `output/tmp/r9.txt`）。

| 分组 | 样本数 | 说明 |
|---|---|---|
| `g = 0` ⇒ 乘数 = 1，**零影响** | 21 / 23 | `revenue_yoy` 缺失或 ≤0 占绝大多数 |
| 腿信号**符号翻转** | 2 / 23 | 688114（Δsig = +1.33，−0.601 → +0.729）、300308（Δsig = +1.135，−0.672 → +0.463），均 `g` 顶 30% 上界 |
| 平均 \|Δsig\| | 0.114 | 被 21 个零影响样本拉低 |

⇒ **修复正确且影响可控**：8.7% 样本翻转，且集中在 `g` 顶格的极端高增长标的 ——
这些标的原本被格雷厄姆腿系统性看空到 −60%~−82%，属**误判**（营收增速 182% 却判内在价值比现价低 82%）。
300642 自身 Δf5 = 0.009 ⇒ **不改变其结论**，与 §5.2 的原估算一致。

> **口径注（2026-09-22 复核时补）**：本表数值由**落库的 `intrinsicValue` / `current_price`**
> 反解得出，故 300308 记为 `1265.22` / `+34.5%` / `σ = +0.463`。
> 若改用**精确 EPS 18.47** 直接代入：`18.47 × 68.5 = 1265.20` / `+36.6%` / `σ = +0.478`。
> **两组数同源，仅 round 精度不同**（落库 `iv = 168.08` 反解 EPS = 18.4703）。
> 引用时须注明基准 —— 复核中曾一度把 `1265.22` 误判为算术错误（见 §9.5 第 3 条），
> 该误判本身印证了判据级纪律「同一数值两个结果，先查**判据口径**再怀疑被测对象」。

### 9.2 本次顺带发现、**未修**的缺陷（登记）

| 编号 | 缺陷 | 位置 | 证据 | 建议 |
|---|---|---|---|---|
| D4 | **LLM 节点输出 schema 漂移 + 映射路径只覆盖一种形态 ⇒ 字段静默缺失** | `seed_stock_analysis.rs:3569` / `:3703` 的 `("risk_disagreement", "risk-convergence.content.verdict.disagreement_score")` | 300642 两次：11:33 输出**嵌套** `{verdict:{disagreement_score:40}}` ⇒ 取到 40；11:49 输出**扁平** `{disagreement_score:30}` ⇒ **取不到**。全库 53 条仅 **9 条**有值（`output/tmp/r2.txt`） | 引擎侧给 `input_mapping` 支持多候选路径（回退），或在 `risk-convergence` 后加归一化 CodeNode。**未改**：属引擎语法/模板结构变更，影响全部工作流，须独立评估 |
| D5 | `assumptions.growth` 与 `intrinsicValue` 在同标的两次落库中不自洽 | 待查 | 300308 两次：`g = 0` 与 `g = 0.30`，而 `intrinsicValue` **均为 168.08** | 疑「落库的 assumptions 与计算用的 `g` 不同源」或跨 `template_version` 混读；需按 `template_version` 分组复检 |

> D4 的**消费端**已随 P1-4 一并兜底，但**根因未修** —— 兜底只保证「缺失时不倒挂」，
> 不保证「本该有值时能取到」。

### 9.3 验证

| 门禁 | 命令 | 结果 |
|---|---|---|
| 前端类型 | `tsc --noEmit` | **EXIT=0** |
| 前端格式 | `dprint check` | **EXIT=0** |
| 前端单测 | `vitest run src/lib/__tests__/stockAnalysisAction.test.ts src/lib/__tests__/agentOutput.test.ts` | **94 passed**（43 + 51） |
| 前端单测 | `vitest run src/components/stock-analysis` | **29 files / 160 tests passed** |
| 项目门禁 | `ci-check.mjs --frontend-only` | ⚠️ **分层护栏棘轮未通过 —— 与本次改动无关，见 §9.4**；其余步骤（领域语义 2 项）✓ |
| 项目门禁 | `check-i18n-key-parity.mjs` / `check-input-mapping.mjs` / `check-hardcoded-i18n.sh --strict` / `i18n-scan.mjs` | **全部 EXIT=0** |
| Rust 类型 | `cargo check --workspace --all-targets` | 见下 |

### 9.4 ⚠️ 分层护栏棘轮失败 —— **归因：非本次改动**

`check-layer-discipline.mjs` 报 5 处超基线（`EXIT=1`，会中断 `ci-check` 后续步骤）：

```
commands-no-direct-db     src-tauri/src/commands/backtest_validation.rs   10 > 基线 9
commands-no-sibling-call  src-tauri/src/commands/backtest_validation.rs    5 > 基线 1
commands-no-sibling-call  src-tauri/src/commands/stock_analysis.rs         7 > 基线 5
commands-no-sibling-call  src-tauri/src/commands/stock_analysis_setup/seed_consistency_tests.rs  2 > 基线 0  ← 新越界文件
crate-has-consumer        src-tauri/crates/crdt                            1 > 基线 0  ← 新越界文件
```

**归因判据（不看 git，只看本次编辑记录）**：本次共改动 19 个文件，逐个比对后，
**无一出现在上述越界清单中** —— 上述 4 个文件/目录（`backtest_validation.rs`、
`stock_analysis.rs`、`seed_consistency_tests.rs`、`crates/crdt`）本次均未打开、未修改、未删除。
⇒ 判为**并发会话改动或既有欠账**（本仓常有多会话并行；且这是静态依赖扫描，与本次的
注释/常量改动无关）。

**处置**：按「报告不擅改」纪律**只登记不代改**。若需下调基线（棘轮型门禁的正确做法是
`--update-baseline` 并逐条归因），须先确认那 4 处越界是谁引入的 —— 归因错会把别人的
违规记到本次账上。

⚠️ **本次未跑** `cargo clippy --workspace --all-targets --all-features -- -D warnings` 与
`__TAURI_WORKSPACE__=true cargo test --workspace`（20+ 分钟）。按 `AGENTS.md`「门禁覆盖面」条款，
**提交前必须补**（`cargo check` 不执行 `clippy.toml` 的 `disallowed-types` 等规则，全绿≠合规）。

**测试改动**：`src/lib/__tests__/stockAnalysisAction.test.ts` —— 原「持有/观望派生化」describe
改写为「展示档 = 方向档（不再按仓位派生）」，并新增 AUDIT-300642 回归用例：
**同为「观望」档、仓位 0% vs 3% ⇒ 必须给出同一展示档**（修复前分别是「观望」与「持有」）。

---

### 9.5 ⚠️ 补跑门禁时自查出的**上一轮漏改**（5 条）

上一轮结束时 `cargo check --workspace --all-targets` 是 **EXIT=0**，我据此认为改动完整。
补跑 `cargo test` 前逐字复核，发现 **5 类漏改** —— 它们全部**不可能**被 `cargo check` 发现：

| # | 类型 | 漏改内容 | 为何 `cargo check` 全绿 | 处置 |
|---|---|---|---|---|
| 1 | **P0：断言失效** | `mcp_tools.rs` 4 处测试期望值仍写旧量纲 `8.5 + 2.0 * DEFAULT_GROWTH`（**缺 `×100`**）⇒ 期望 `0.39×8.74 = 3.41` vs 生产 `0.39×32.5 = 12.675`，**必然 panic** | `check` **不做断言求值** —— 类型正确即通过（「编译通过七型假阳性」） | 4 处全部改为与生产式**同形**（`* 100.0`），并加注「改一侧必改另一侧」 |
| 2 | **行为变化未登记** | 负增长样本（`g = MIN_GROWTH = −0.30`）：乘数由 `8.5+2×(−0.30) = 7.9` 变为 `8.5+2×(−30) = −51.5` ⇒ 被 `.max(0.0)` 兜为 **0**（修复前 `V = 7.9×EPS`） | 同上 —— 是运行期数值行为 | 断言锁住新行为（`v == 0`）＋ 明确登记：此类标的 `upsidePct` 恒 **−100%**。**本次 23 个样本中 `g < 0` 者为 0**，故未在真实数据上暴露；一旦出现须按 `growth_clamped_lower` 单独降信 |
| 3 | **误判（口径非错误）** | 注释块原写 `1265.22`，我一度判为算术错误（应为 `18.47×68.5 = 1265.195`） | — | **我判错了**：`1265.22` 是用**落库 round 值** `168.08` 反解（`168.08/9.1 = 18.4703`）的结果，与我用精确 EPS 算的 `1265.20` **同源不同精度**。已在注释与 §9.1 双向标注基准。**教训**：同一数值出现两个结果，先查判据口径，再怀疑被测对象 |
| 4 | **撒谎注释 3 处** | ① `mcp_tools.rs` 算式块 `4.4 / 4.136` 与 `168.08` 复算不自洽（`18.47×9.1 = 168.077` 需 `Y = 4.4`）；② `seed_stock_analysis.rs:3760` 手抄旧口径 `EPS 18.47 × (8.5 + 2×0.30) = 168.08`；③ `portfolio-mgr.rhai:671` 旧乘数 `EPS × 9.1` 与 `σ = −0.672`（**符号已翻转**为 `+0.478`） | `check` 不读注释内容 | 全部改写为可复算形式。rhai 注释里**无法复算的相对值**（旧记的 `−0.339`）直接标注「随之作废，勿沿用」，不编造新数 |
| 5 | **自证材料口径矛盾** | `graham.assumptions.formula` 暴露给 LLM：`"EPS × (8.5 + 2g) × 4.4 / bondYield"`，而**同一 JSON 里** `growth = 0.12`（小数）、`bondYield = 4.4`（百分数）⇒ 诱导 LLM 自行重算出错（正是本 bug 的同一陷阱） | `check` 不校验 JSON 语义自洽性 | ① formula 显式写 `2×g%，其中 g% = growth × 100`；② **新增 `multiplier` 字段**直接给乘数，并在注释里令消费端「不要自己从 `growth` 推」。`GrahamAssumptions` 无跨 crate 消费者 ⇒ 加字段安全 |

**教训（写进判据层）**：改一个「量纲/口径」类底层公式时，**消费端清单**必须包含
「**测试断言**」「**注释里的算式**」「**暴露给 LLM 的自证字段**」三类 —— 它们都不受编译器保护。
`cargo check` 只能证明「类型链完整」，**不能证明「口径链一致」**。

**本轮验证**（补跑后）：

| 门禁 | 结果 |
|---|---|
| `cargo test -p axagent-astock-data --lib` | **340 passed / 0 failed**（含 6 个 `graham_*` 逐条 ok） |
| `cargo fmt --all --check` | **EXIT=0** |
| `check-input-mapping.mjs` | **EXIT=0**（多段源路径 35 条、悬空 0、DAG 连通性全 0）|
| `cargo clippy --workspace --all-targets --all-features -- -D warnings` | 见 §9.7 |
| `__TAURI_WORKSPACE__=true cargo test --workspace` | 见 §9.7 |

### 9.6 本轮登记（**D6 已定性，附 DB 证据**；**方案 A 已于当晚实施，见 §10**；⚠️ 本节「代价」判读已作废，见 §11）

| 编号 | 缺陷 | 位置 | **DB 取证（2026-09-22 22:55，PG `axinvest`）** | 处置 |
|---|---|---|---|---|
| **D6** | **估值缺省参数存在第二份分叉副本** ⇒ `DEFAULT_GROWTH = 12%` 在注入路径上永不生效 | `mcp_tools.rs:2063`（`DEFAULT_GROWTH = 0.12` / `MIN_GROWTH = −0.30`）**vs** `stock_analysis.rs:6288-6300`（`ValuationParams::default()` = `default_growth: 0.08` / `min_growth: 0.02`） | `settings` 表共 **96 个键**，匹配 `param\|val\|est\|dcf\|growth\|bond\|graham` 的键 **= 0 个**（唯一 8 条命中全是 `approvalPolicy` / `bubbleStyle` / `s3SyncIntervalMinutes` 等无关键）⇒ `load_valuation_params` 必走 `Ok(None) => ValuationParams::default()` ⇒ 实际注入 **`defaultGrowth: 0.08`** | 已定性；**推荐修法见下，待授权**（改动作用于全部标的，非本工作流局部） |

**D6 完整链路（读代码 + DB 实测，非推断）**：

```text
settings.valuation_params        不存在（DB 实测，96 键中 0 命中）
  → load_valuation_params()      Ok(None) ⇒ ValuationParams::default()        stock_analysis.rs:6306
  → default_growth = 0.08        ← 与 mcp_tools 的 DEFAULT_GROWTH = 0.12 分叉
  → inject_valuation_config_for_tool()  无条件注入 defaultGrowth: 0.08         stock_analysis.rs:6318（唯一调用点 :1828）
  → compute_valuation 解析 valuation_config（优先级最高）
  → cfg.default_growth() 取到 0.08（而非 0.12）                                mcp_tools.rs:2228
     ⇒ 消费点两处：DCF `:2754` 与 graham `:3053`
```

后果：`revenue_yoy` 缺失的标的，其 DCF / graham 缺省增长率**实为 8% 而非 12%**，
而 `mcp_tools.rs:2052` 注释声称「原 8% 偏保守，已升 12%（系统性低估成长股）」
—— **该升级被旁路，注释与实况不符**。同理 `min_growth` 2% vs 常量 −30% 也走的是 2%。

**推荐修法（待授权，二选一）**：

| 方案 | 做法 | 代价 |
|---|---|---|
| **A（推荐）收敛为单一真相源** | 把 `DEFAULT_GROWTH` / `MIN_GROWTH` 提为 `pub const`，令 `ValuationParams::default()` **引用**它们，而非手抄字面量 | 生效值 8% → **12%**、`min_growth` 2% → **−30%** ⇒ **改变所有标的的估值结论**，须先按 §9.1 方法量化影响面<br>⚠️ **该代价判读已作废** —— 变量根本未被消费，「生效值」不会因此改变，见 §11 |
| B 只改注释、承认现状 | 把 `mcp_tools.rs:2052` 的「已升 12%」改为说明「注入路径下 8% 优先生效」 | 零行为变化，但**放弃升级意图**，且 `min_growth` 分叉仍在 |

⚠ **本项不擅自改**：A 是本轮影响面最大的改动（作用于全部股票而非本工作流），
且「缺省增长率取 8% 还是 12%」属**产品判断** —— 与 P0-1/P0-2 那种**有唯一正确答案**的逻辑/量纲错误**不同类**。

> D6 与本轮 P0-2 同族（都是「同一语义两处口径」），但**层次不同**：P0-2 是**同一表达式内**两套量纲（已修），
> D6 是**两个独立配置源**给出不同缺省值。排查同类问题时两层要分别搜。
>
> **另注**：`mcp_tools.rs::from_flat_arguments`（`:2259`，读面板变量 `value_dcf_growth_rate` 等，
> **百分数**口径）是**第三条独立链**，与 D6 无关。三条链优先级见 `:2258`：
> `valuation_config`(object) > 扁平参数 > 模块常量。查「配置不生效」时必须先分清走的是哪条。

### 9.7 门禁记账（**如实区分「已跑」与「未跑」**）

| 门禁 | 命令 | 结果 |
|---|---|---|
| Rust 单测（**实质改动所在 crate**） | `cargo test -p axagent-astock-data --lib` | **340 passed / 0 failed**（6 个 `graham_*` 逐条 ok） |
| Rust 格式 | `cargo fmt --all --check` | **EXIT=0**（CI 口径带 `--all`） |
| 模板映射 | `scripts/check-input-mapping.mjs` | **EXIT=0**（多段源路径 35 条 / 悬空 0 / DAG 连通性全 0） |
| Rust lint | `cargo clippy --workspace --all-targets --all-features -- -D warnings` | ⏳ **排队中**（见 9.7.1） |
| Rust 全量测试 | `__TAURI_WORKSPACE__=true cargo test --workspace` | ❌ **未跑**（缺口分析见 9.7.2） |

#### 9.7.1 全量 clippy 排队中 —— 与并发编译争锁

`cargo clippy --workspace …` 自 22:53 起，日志仅有
`Blocking waiting for file lock on build directory`（1 行 / 54 字节，至今无新行）。
按 `cargo-win-lock-troubleshoot` 的判据分流：

| 判据 | 读数 | 结论 |
|---|---|---|
| 有无 `os error 5` / `[sandbox]` | **均无** | **不是**沙箱、**不是**锁文件损坏 ⇒ 不删锁、不靠 `dangerouslyDisableSandbox` |
| 持锁者是否在干活（CPU 采样：40s 内两次当场读基准） | `rustc` PID 32892：**133.98s → 172.28s（Δ38.3s ≈ 满核）** | **持锁者满核编译中** ⇒ 去等，**不杀** |
| 其余 7 个 cargo 系进程 | Δ CPU **全为 0** | 与我一样在排队 ⇒ 不是「多路抢锁」，是**一条命令展开的进程链** |
| 编到哪一步 | `deps/axagent_lib.d` @ 22:55:24（**依赖图最顶层**） | 主 crate 正在编 ⇒ 其下游依赖均已编过 |

**处置**：按该 skill 的「**若全量口径已经在跑（哪怕还在等锁），就复用它，不要再起一条**」——
保留这条排队，**不并发起第二条 cargo**；同时**暂停一切 `.rs` / `.rhai` 落盘**
（本仓 `include_str!` 会追踪 `.rhai`，落盘即触发对方重建、延长争用）。

#### 9.7.2 `cargo test --workspace` 未跑 —— 覆盖度分析（不是「因为麻烦」）

逐项核对本次改动的**行为影响面**：

| 改动文件 | 改动性质 | 有行为影响？ | 由哪条口径覆盖 |
|---|---|---|---|
| `crates/astock-data/src/mcp_tools.rs` | 公式已在前轮改完；本轮 = **4 处测试断言** + **注释** + **新增 `assumptions.multiplier` 字段** | **有**（新增 JSON 键） | ✅ `-p axagent-astock-data --lib`（340 passed） |
| `src/commands/portfolio-mgr.rhai` | **仅注释** | 无 | — |
| `src/commands/stock_analysis_setup/seed_stock_analysis.rs` | **仅注释**（Rust 源码注释，**非**模板内嵌字符串 ⇒ 不改模板 code） | 无 | — |

补一条**消费端核查**（解释 `multiplier` 新增键为何低风险）：
`GrahamAssumptions` 无跨 crate 消费者；`graham.assumptions` 的消费面仅
① 模板映射（只读 `growthClampedUpper`）② LLM 自证字段（多一个键不影响解析）。

⇒ **行为相关改动全部落在 `axagent-astock-data`**，已由定点口径覆盖；主 crate 两处是**纯注释**，
不改变运行期行为。
⚠ **如实记账**：`--workspace` 口径**未跑**，故「主 crate 测试面」本轮**无独立读数**，缺口待出锁后补。

### 9.8 生效前提（**已取证**：DB 仍为修复前状态）

| 事实 | 读数 | 来源 |
|---|---|---|
| DB `workflow_templates.id='stock-analysis'` 的 version | **75** | PG `SELECT version FROM workflow_templates WHERE id='stock-analysis'` |
| 代码 `TEMPLATE_VERSION` | **76** | `seed_stock_analysis.rs:246` |
| DB 模板内是否仍含 `display_action`（P0-1 修复前标志） | **命中** | 对 `nodes` 字段正则扫描（368,901 字符） |
| DB 模板内是否仍含 `gate_display_action`（第二份副本） | **命中** | 同上 |
| DB 模板内 `disagreement_mod … else { 0.7 }`（P1-4 修复后形态） | **未命中** | 同上 |

⇒ **DB 确认仍是修复前状态**；`existing.version(75) < TEMPLATE_VERSION(76)` ⇒
**重启应用即触发重种子**，本轮全部后端改动（含 P0-1 / P1-4 / 模板内嵌 rhai）**届时生效**。
（`portfolio-mgr.rhai` 的最后一轮注释微调发生在升版到 76 **之后**，因 v76 尚未落库，
该注释同样会被 v76 带上 ⇒ **无需再升版**。注释不影响行为，故也不为它单独升版触发全量重种子。）

---

## 10. 方案 A 实施（用户指令：「§9.6 按照 A 进行修改」）

实施于 2026-09-22 深夜。**实际分叉是 4 个字段，§9.6 只登记了 2 个** —— 补登见 §10.1。

### 10.1 §9.6 漏登的两个分叉字段（本轮回溯补齐）

| 字段 | `astock-data` 常量 | `ValuationParams::default()`（修复前） | 分叉 |
|---|---|---|---|
| `perpetual_growth` | 0.04 | **0.03** | ✅ **§9.6 漏登** |
| `discount_rate` | 0.085 | **0.10** | ✅ **§9.6 漏登** |
| `default_growth` | 0.12 | 0.08 | ✅ 已登 |
| `min_growth` | −0.30 | 0.02 | ✅ 已登 |
| `max_growth` | 0.30 | 0.30 | 一致 |
| `forecast_years` | 5 | 5 | 一致 |
| `bond_yield` | 4.4（内联字面量） | 4.4 | 一致（但两处手抄） |

⇒ **7 个字段中 4 个分叉**。§9.6 只搜了「增长率」两个字段，漏了折现率与永续增长率 ——
**同一表达式内的分叉会一起暴露，而「同一结构体的多字段分叉」必须逐字段穷举**
（漏登的直接后果：只修 2 个字段的话，`discount_rate` 仍会让两链分叉）。

### 10.2 改动清单（5 处）

| # | 文件 | 改动 |
|---|---|---|
| 1 | `crates/astock-data/src/mcp_tools.rs` | 6 个常量提为 `pub const`（新增 `DEFAULT_BOND_YIELD`，消掉 `unwrap_or(4.4)` 裸字面量）；文档注释登记**全部 4 处消费端** |
| 2 | `commands/stock_analysis.rs` | `ValuationParams::default()` 7 字段全部改为 `m::<常量>` **派生**；字段 JSDoc 改为「缺省值派生自常量，见 `Default` 实现」（**不再写具体数字** —— 写了必然腐烂） |
| 3 | `.../stock_analysis_setup/seed_variables.rs` | `DEFAULT_DCF_*_PCT` 改为 `axagent_astock_data::mcp_tools::<常量> * 100.0` **派生**（原为手抄 12.0/4.0/8.5） |
| 4 | `src/components/settings/StockAnalysisConfigPanel.tsx` | `DEFAULT_VALUATION_PARAMS` 对齐 `0.04/0.085/0.12/−0.30`；前端引用不到 Rust ⇒ 保留手抄 + 门禁守卫 |
| 5 | **新增** `scripts/check-valuation-defaults-parity.mjs` + 接入 `scripts/ci-check.mjs`、`.github/workflows/ci.yml`、`package.json` | 等式门禁（4 处载体逐字段相等 + 派生形态检查），含 `--selftest` |

**新增的等式测试**（主 crate）：`valuation_defaults_tests::valuation_defaults_are_single_sourced`
与 `seed_pct_defaults_round_trip_to_truth_source`（后者锁 **IEEE754 往返**：
`0.085 × 100.0` 必须**恰好**等于 `8.5`，否则模板变量会落库成 `8.499999999999998`）。

### 10.3 门禁验证（**含负控**）

| 项 | 结果 |
|---|---|
| `check-valuation-defaults-parity.mjs` 正式门禁 | **EXIT=0**（7 字段全 ✓） |
| 同上 `--selftest`（5 条正负对照） | **5/5 通过**：正样本无错；前端漂移 / 主 crate 手抄 / seed 手抄 / 真相源写法变更 **四类负样本全部报红** |
| 独立负控（临时把前端改成 `0.13` 再跑） | **EXIT=1**，精确定位到 `DEFAULT_GROWTH / defaultGrowth 不相等`，且**文件逐字节恢复**（`restored=true`） |

> **为什么要做独立负控**：门禁自己报「通过」不构成证据 —— 一个永远返回 0 的脚本也是「通过」。
> 必须证明它在**被测对象真的错**时会红。

### 10.4 生效前提（**无需重启既有行为不变**）

本次 5 处改动中，1/3/4 都是**值不变的类型收敛**（`12.0` → `DEFAULT_GROWTH * 100.0`，同值）；
2 是**值变化**（旧 0.08/0.03/0.10/0.02 → 新 0.12/0.04/0.085/−0.30），但**只作用于前端直调链**。

⇒ **工作流侧的行为不因本次改动而变化**（理由见 §10.5）。前端改动（4）编译期生效。

---

## 11. ⚠️ 更正：v74「强制校正」的连带效应判断是**错的**

§9.9 之前（本报告早期版本 §9.6 讨论与我的口头汇报）曾称：
> 「v74 的 `force_variable_value` 会在 v76 重种子时首次执行 ⇒ DB 变量从 `10/3/8` 改为 `8.5/4/12`
> ⇒ **全部标的的 DCF 都会变**（688114：`dcf.mid` 24.23 → 37.27，+53.8%）」

**该判断错误。** 三处证据（全部为 PG 实测）：

| # | 证据 | 读数 |
|---|---|---|
| 1 | **历史版本快照**（`workflow_template_versions`，v64–v73 共 6 版）的 `value_dcf_*` | **全部是 `8 / 3 / 10`** ⇒ v74 的 force **从未执行**（若执行过，v74 之后的值应为 `12 / 4 / 8.5`） |
| 2 | 该表**最新版本 = 73**，而 `workflow_templates.version = 75` | 中间两版（74/75）无快照 ⇒ 印证「版本门 `existing.version >= TEMPLATE_VERSION ⇒ return`」把种子整个跳过 |
| 3 | 运行产物落库的**实际生效参数**（`dcf.assumptions.configured_perpetual_growth` / `discount_rate`） | tpl=65/71/73/75 全部为 **`0.04 / 0.085`**（= **模块常量**，非变量值 `3/10`）⇒ **变量未被消费** |

**⇒ 双重结论**：
1. **v74 的 force 未执行**（版本门饿死）；
2. **即使执行，也不改变工作流结论** —— 因为 `value_dcf_*` 变量**根本没被消费**（证据 3）。

⇒ **重启应用不会导致 DCF 结论变化**（工作流一直用的是模块常量 = 校准后值）。
**§9.6 里「代价：生效值 8% → 12% ⇒ 改变所有标的的估值结论」这行判读作废**（我当时以为变量生效）。

**根因教训**：我把「代码里写了 force」当成「值会变」，没有验证**消费端是否读取该值** ——
这正是本仓反复出现的「只改定义端、漏消费端」的镜像形态：**改了源头，但没人消费**。

---

## 12. 本轮新登记 **D8**：模板变量 `value_dcf_*`「偶发」生效（**根因已定位并修复，见 §13**；本节保留当时的登记原文）

### 12.1 事实

同一标的、同一变量表（快照证明 v64–v75 恒为 `8/3/10`），落库的**实际生效参数**却分两组：

| code | tpl | 时间 | `configured_perpetual_growth` | `discount_rate` | 判定 |
|---|---|---|---|---|---|
| 300308 | 65 | 09-21 05:37 | 0.04 | 0.085 | 变量**未**生效（走常量） |
| **300308** | **70** | **09-21 16:10** | **0.03** | **0.1** | **变量生效**（= 变量值 3/10） |
| 300179 | 71 | 09-21 18:22 | 0.04 | 0.085 | 变量**未**生效 |
| 688114 | 73 | 09-21 21:00 | 0.04 | 0.085 | 变量**未**生效 |
| 300642 | 75 | 09-22 11:33 / 11:49 | 0.04 | 0.085 | 变量**未**生效 |

⇒ 变量只在 **tpl=70 那一次**被消费。**面板上的 DCF 三个参数绝大多数时候是死配置**
（用户改它不生效），偶尔生效 —— 比「恒定不生效」更难查。
（历史实证：09-13 的 601166 落库**含**工具入参 `{"dcf_discount_rate":8.5,"dcf_growth_rate":12,…}`
且为**数字**类型；09-21/09-22 的所有运行落库**都不含**这三个 key。）

### 12.2 为什么本轮**不改**

1. **根因未定位**。已排除：映射表本身（DB v75 模板里 `dcf_growth_rate: value_dcf_growth_rate`
   三条映射**都在**）、变量缺失（三个变量都在表里且 `type=number`）、
   注入端类型（09-13 落库证明曾是数字）。
   尚未查清的是**运行期 `context.variables` 的填充**（`engine/mod.rs:2061` 与 `:5817` 两处
   写入路径 + 运行时 override）。
2. **影响面被「巧合」掩盖**：校准后的变量值（8.5/4/12）与模块常量（0.085/0.04/0.12）**同值**
   ⇒ 即便修好链路，工作流结论**也不变**（这是 v74 校准时刻意对齐的副作用，
   但它同时让 D8 变成「静默」缺陷 —— 不修也看不出，修了也看不出）。
3. **修法涉及运行期变量注入语义**（作用于全部工作流，不止本模板）。

⇒ 登记待独立评估。**判据建议**：先查「为什么只有 tpl=70 那次带上了扁平参数」——
若能在落库的 `blackboard_snapshot` 里找到那次的 `arguments` 与其它次的差异，根因即现。

---

## 13. D8 根因定位与修复（2026-09-22 深夜）

### 13.1 结论

**D8 不是「偶发生效」，是「四条入口路径各有各的参数来源」。**

面板上的三个 DCF 参数（`value_dcf_growth_rate` / `_perpetual_rate` / `_discount_rate`）
在 **4 条执行路径中只有 1 条**会读到它；而**那一条读到的还是被 v74 漏掉的陈旧值**。
其余 3 条压根不加载模板变量 ⇒ tool 节点 `input_mapping` 解析不到 ⇒ 静默回退模块常量。

| 路径 | 入口 | `opts.variables` 来源 | `dcf.assumptions` 实际值 |
|---|---|---|---|
| **工作区**（UI 单股分析/重跑） | `run_stock_workflow` → `run_stock_workflow_inner`（`stock_workflow/core.rs:843` `opts.variables = template_vars`） | **模板变量 360 项** | `0.08 / 0.03 / 0.10` ← **陈旧变量值** |
| **批量单股**（池扫描/自选股扫描/管道子调用） | `run_single_stock_analysis`（同文件 `:1814` `variables: None`） | **无** | `0.12 / 0.04 / 0.085` ← 模块常量 |
| **对话直执行** | `commands/workflows/mod.rs::workflow_execute`（原 `:200-227`） | **无**（需会话带 `workflow_template_id`，实测为 NULL） | 模块常量 |
| **管道** | `stock_pipeline/core.rs`（原只传 6 个管道自带变量） | **仅管道变量** | 模块常量 |

⇒ 用户视角的「**改了有时管用有时不管用**」= 6 条落库样本里只有 1 条走了工作区路径。

### 13.2 证据链（每条都可复算）

| # | 取证 | 读数 | 排除了什么 |
|---|---|---|---|
| E1 | `workflow_templates.variables`（v75） | 360 项；key 组合**唯一**（`description,is_secret,name,value,var_type`）、结构与 `harness::workflow_types::Variable` 完全兼容；`value_dcf_*` = **8 / 3 / 10** | 排除「`.ok()` 静默解析失败」——当前数据能正常反序列化 |
| E2 | `workflow_template_versions` v70/v71/v73 | 三版**逐字节同长**（41429）、`value_dcf_*` 恒 8/3/10 | 排除「模板数据在 v70→v71 之间被改写」 |
| E3 | 节点 diff v70 vs v71 | `t-valuation.config.input_mapping` **语义完全一致**，唯一差异是 JSON **key 顺序** | 排除「映射表漂移」 |
| E4 | `tool_executions` 表 | `compute_valuation` **0 条**（整表 0 行） | 排除「用逐次入参取证」这条路（见表下 D10） |
| E5 | `conversations.workflow_template_id`（6 条分析对应的会话） | **全为 NULL** | 证明对话路径的模板变量加载**从未激活** |
| E6 | `stock_analyses` 落库 dcf 参数（6 条） | 5 条 = `0.04/0.085`（常量）；1 条 = `0.03/0.1`（**300308 @tpl=70，`parent_analysis_id` 非空 = 工作区重跑**） | 值集合与「路径决定来源」假说**完全自洽** |

> ⚠ **同值巧合（必须记下，否则会误判）**：模板变量 `8/3/10 ÷100 = 0.08/0.03/0.10`，
> 与**修前**的 `ValuationParams::default()`（`default_growth 0.08` / `perpetual 0.03` / `discount 0.10`）
> **逐字段相同**。所以单看 E6 那一行的数值**无法区分**「模板变量生效」与「注入 `ValuationParams` 生效」。
> 判据只能靠**路径可达性**：工作流 tool 节点走 `crates/tools/src/tools/astock_data.rs:110`
> 的 `execute_mcp_tool`，**不做任何 `valuation_config` 注入**（注入只存在于 Tauri 命令
> `execute_stock_mcp_tool`，`stock_analysis.rs:1828`）⇒ 该路径上唯一可能来源就是扁平参数 = 模板变量。✓

### 13.3 修复（4 处，全部落地）

| # | 文件 | 改动 |
|---|---|---|
| F1 | `stock_workflow/core.rs`（`run_single_stock_analysis`） | `variables: None` → `variables: loaded.variables.clone()`（模板变量此前只用于取并发/超时，没进上下文）|
| F2 | `commands/workflows/mod.rs`（`workflow_execute`） | 模板变量加载从「经 `conversation_id` 反查 `conversations.workflow_template_id`」改为**按本命令已持有的 `workflow_id` 直查**；前端显式变量仍按 name 覆盖 |
| F3 | `stock_pipeline/core.rs` | 模板变量作**基座**，`build_pipeline_variables` 的计算值按 name 覆盖同名项（此前只传 6 个管道变量）|
| F4 | `stock_workflow/decision.rs` + `stock_pipeline/core.rs` | `.ok()` 静默降级 → 解析失败 `tracing::warn!`（**保留降级、消除静默**，与同文件 `hooks_config` 同语义）|
| F5 | `init/state.rs`（`RunWorkflow` 工具执行器） | 同型入口：**LLM 经 `RunWorkflow` 工具按 id 跑工作流**时只传 `input`、不装变量 ⇒ 一并按 `workflow_id` 直查装载 |

#### 13.3.1 同型入口穷举（`grep "RunOptions {"` 全仓 9 处逐处核对）

| 构造点 | `variables` | 判定 |
|---|---|---|
| `stock_workflow/core.rs:816`（工作区） | `opts.variables = template_vars` | ✅ 本就正确 |
| `stock_workflow/core.rs:1782`（批量） | 原 `None` | 🔴 → **F1** |
| `commands/workflows/mod.rs`（对话） | 原依赖会话标记 | 🔴 → **F2** |
| `stock_pipeline/core.rs` | 原只有 6 个管道变量 | 🔴 → **F3** |
| `init/state.rs:1441`（**RunWorkflow 工具执行器**） | 无 | 🔴 → **F5** |
| `stock_workflow/serenity.rs:964` | `loaded.variables`（**已含模板变量**，`serenity.rs:868`） | ✅ 无缺陷 |
| `demand_discovery.rs:682` / `opc_capability_pack_actions.rs:2458` | `Some(variables)` | ✅ 已传 |
| `plan.rs:1249`（计划执行器） | 无 | ⚪ 不适用（该模板无面板参数）|
| `stock_workflow/reflection.rs:426` | `Some(variables)` | ✅ 已传 |

⇒ 结论：同型缺陷共 **4 处**（F1/F2/F3/F5），已全部修复；其余 5 处经核对无缺陷。
**这一栏是必须做的** —— 只修「报告里点名的入口」会漏掉 3 处同型（本次实测）。

F4 是这类缺陷的共同放大器：`variables = None` 与「解析失败」在行为上**完全不可区分**，
这正是 D8 能潜伏至今的原因。

### 13.4 影响面（**不止 DCF**）

F1–F3 让 **360 个模板变量**在原本不生效的 3 条路径上全体生效。高影响项：

- `vendor_*` 开关 —— 此前**批量扫描/对话路径忽略面板开关**，会调用用户已关闭的数据源；
- `rule_*` 阈值（RSI/乖离/量能/止损）、`kline_limit` / `news_limit`；
- `agent_temperature`、`debate_rounds`、`value_moat_threshold` / `value_fscore_buy` / `value_safety_margin`；
- `max_concurrent` / `tool_timeout_secs`：批量路径**原本就**通过 `resolve_runtime_options` 读模板变量，行为不变。

⇒ 判据：修复后「面板 = 唯一入口」，四条路径取值必然一致。

### 13.5 结论是否改变？**不变**（这点很重要）

> ⚠️ **本节第 2 条的「生效值」判断已被 §13.10 推翻**（当时基于「力会被执行」的假设）。
> 保留原文以便留痕，**当前有效结论见 §13.10**。

- **修复前**：多数样本（批量/对话/管道）= 模块常量 `12/4/8.5`；少数（工作区）= 陈旧变量 `8/3/10`。
- ~~**修复后 + v76 重种子**：DB 变量被 v74 的 `force_variable_value` 强制校正为 `12/4/8.5`
  （实测该 force **无条件执行**，只要种子体跑；DB 75 < 代码 76 ⇒ 重启即跑）⇒ 四条路径全部 = `12/4/8.5`。~~
  → **实测不成立**：变量落库后端有一段「逐名保留旧值」的覆盖（`:5043`），把 force 写的值又打回
  `8/3/10` ⇒ **重种后变量仍是 `8/3/10`**（见 §13.10）。
- ⇒ 与今天**多数样本**一致，**不改变任何既有结论**；变的是「少数路径的陈旧值被消掉」+「面板从此真的能用」。
  （「少数路径陈旧值被消掉」的表述仍成立 —— 只是方向反过来：**统一到 `8/3/10`**，不是 `12/4/8.5`。）

### 13.6 对 §12 的两处更正

1. §12 写「校准后的变量值（8.5/4/12）与模块常量同值」—— **错**。变量实际是 **8/3/10**，
   与常量 `12/4/8.5` **不同值**。§12 这句话是基于当时未取到的读数推断的。
2. §12 写「根因未定位 … 修法涉及运行期变量注入语义（作用于全部工作流）」—— 前半句已由本节闭合；
   后半句仍然成立：F1–F3 **确实**作用于全部工作流（凡是从这三条入口起的模板），
   但改动都在**调用侧的变量装载**，未触碰引擎的注入语义（`engine/mod.rs` 一行未改）⇒ 风险可控。

### 13.7 新登记（未修，需独立评估）

| 编号 | 缺陷 | 位置 | 证据 | 为何不擅改 |
|---|---|---|---|---|
| ~~**D9**~~ | ~~**面板 DCF 三参数会在每次模板升版被静默重置**~~ ⚠️ **本行已作废，并入 D11（§13.11）** —— 实测「升版从不改变变量值」，与本节当时的推断相反 | `seed_stock_analysis.rs:4663-4677`（`force_variable_value` ×3） | 该 force **无条件**执行，但被 `:5043` 覆盖 ⇒ **不是「被重置」，而是「从不生效」**（见 §13.10）| — |
| **D10** | 工作流 tool 节点的调用**不落** `tool_executions` ⇒ 逐次入参无取证通道 | `crates/dao/src/repo/tool_execution.rs` 的 `create_tool_execution` 仅被 `conversations/streaming.rs:606`（对话 MCP）与 `tools/src/recorder.rs:40` 调用 | 全表 **0 行**；本次 D8 因此只能绕道 `blackboard_snapshot` + 代码路径可达性取证 | 属可观测性缺口（补上要接 recorder 到工作流 tool executor），优先级低于 D8/D9 |

### 13.8 门禁

| 门禁命令 | 结果 |
|---|---|
| `cargo fmt --all --check` | **EXIT=0** |
| `cargo check --workspace --all-targets` | 首跑 **EXIT=101**（F5 引入 `E0507`，见 §13.8.1）→ 修复后并入 clippy 口径重验 |
| `cargo clippy --workspace --all-targets --all-features -- -D warnings` | 待 §14 补读数（**不预填**）|
| `__TAURI_WORKSPACE__=true cargo test --workspace` | 见 §14 |

`clippy` 全量口径**同时覆盖类型检查**（clippy 先做类型检查再跑 lint）⇒ 修复 `E0507` 后
不再单独重跑 `cargo check`，直接以 clippy 结果作为「类型 + lint 合并判定」，避免为同一产物类
多付一轮主 crate 编译（`check` 与 `clippy` 同产 `.rmeta`，属同一产物类）。

#### 13.8.1 F5 首跑编译失败（E0507）与修复

**症状**（`output/tmp/cargo-check-d8b.log`，`CARGO_D8B_EXIT=101`，13m20s 后失败）：

```
error[E0507]: cannot move out of `db_for_rw_tool`, a captured variable in an `Fn` closure
    --> src\init\state.rs:1421:26
1416 |             move |workflow_id: String, ...| -> RunFuture {
1419 |                   |______________________________- captured by this `Fn` closure
1421 |                   Box::pin(async move { ^^^^^^^^^^ `db_for_rw_tool` is moved here
     = help: `Fn` and `FnMut` closures require captured values to be able to be consumed
             multiple times, but `FnOnce` closures may consume them only once
```

**根因**：`set_workflow_executor` 注册的是 `Fn` 闭包（执行器可被多次调用）。F5 把 `db` 句柄
捕获在闭包**外**，再让内部 `async move` 块直接 consume 它 ⇒ 只有 `FnOnce` 才会允许。

**修复**（`state.rs:1421-1423`）：在**闭包体内**先 clone、把 clone 出的副本 move 进 async 块：

```rust
let engine = engine_for_run.clone();
// 本闭包是 `Fn`（可多次调用）⇒ 不能把捕获的 `db_for_rw_tool` move 进
// `async move` 块，每次调用 clone 一份（E0507 的编译器建议）
let db = db_for_rw_tool.clone();
Box::pin(async move { /* … 用 &db … */ })
```

**这条值得记（跨项目通用）**：凡「向全局注册一个执行器 / 回调」的接线点（`set_*_executor`、
`set_*_handler`），宿主闭包类型是 `Fn` 而非 `FnOnce` —— **闭包外捕获的任何非 `Copy` 资源
（DB 句柄、`Arc` 之外的连接、buffer）都不能直接 move 进内层异步块**，必须在闭包体内 clone。
这是新增接线点时的首个高频编译错误，且**在 `cargo check` 单 crate 定点跑时常常不会暴露**
（本次就是全量 check 才报）。

### 13.9 生效前提（与 §9.8 一致）

1. **重启应用** → 触发 `TEMPLATE_VERSION=76` 重种子 ⇒ **四条路径取值一致**（D8 修复的意义所在）；
2. ⚠️ **但变量值不会变成 `12/4/8.5`** —— 重种后 `value_dcf_*` 仍是 `8/3/10`（见 §13.10）。
   面板改动**会**即时落库并生效（不再区分入口），这是本次修复真正交付的能力。

### 13.10 ⚠️ 重大更正：`force_variable_value` 是**假修复**（D8 收尾阶段实测发现）

**推翻 §13.5 第 2 条与 §13.9 第 1 条的原判断。**

#### 13.10.1 触发发现的事实

对 DB 做只读实测（`output/tmp/db-probe-d8verify.cjs`）：

| 读数 | 值 |
|---|---|
| `workflow_templates` 活行（id=`stock-analysis`） | `version=75`，`variables` 360 项 |
| `value_dcf_growth_rate` / `_perpetual_rate` / `_discount_rate` | **`8` / `3` / `10`**（= 校准**前**的旧值）|
| 代码侧目标值（`astock-data::mcp_tools`） | `DEFAULT_GROWTH=0.12`、`PERPETUAL_GROWTH=0.04`、`DISCOUNT_RATE=0.085` ⇒ ×100 = **`12 / 4 / 8.5`** |

**推论（不依赖读代码即可成立）**：活行 `version=75` 是**种子自己写的** ⇒ 种子必然成功跑过
（且不止一次，从 70 一路到 75）⇒ 若 `force_variable_value` 有效，值早该是 `12/4/8.5`
⇒ **实测是 `8/3/10` ⇒ force 的效果在某处被抹掉**。

#### 13.10.2 被谁抹掉：`variables` 落库端的「逐名保留旧值」

`seed_stock_analysis.rs` 的落库**不是一个 UPDATE，而是 DELETE + INSERT**：

```rust
// :4985  删除旧行（`let _ =` 吞掉错误）
let _ = workflow_template::Entity::delete_by_id(TEMPLATE_ID).exec(db).await;
//    ↑ 前一次 grep 用 `delete(db)` 模式漏了这里的 `.exec(db)` 写法 ⇒ 差点得出
//      「没有 delete」的错误结论（教训：grep 模式不够宽 ⇒ 0 命中 ≠ 不存在）

// :5043  写新行时，variables 的 value 列**逐名用旧值覆盖回去**
variables: Set(if let Some(ref ov) = old_variables {
    // 升级时保留用户自定义的变量值
    let new_vars = serde_json::from_str(&variables_val)…;   // = 已 force 过的新值
    for nv in &mut final_vars {
        if let Some(old_v) = old_parsed.iter().find(|o| 同名) {
            nv.insert("value", old_val.clone());            // ← 打回 DB 旧值
        }
    }
    …
} else { Some(variables_val.clone()) }),
```

**执行顺序**（`old_variables` 在 `:1079`、DELETE 之前从 `existing.variables` 捕获）：
`:4641` `merge_variable_values`（保留旧值）→ `:4654`/`:4663` **force（写新值）** →
**`:5043` 用旧值覆盖（抹掉 force）** ⇒ **最终落库 = DB 旧值**。

⇒ **净语义：升版时 `variables` 的每个 `value` 一律不变。** 这解释了为什么「DCF 参数 2026-09-12 校准」
到 2026-09-22 仍未生效 —— 也解释了为什么 DB 恒为 `8/3/10`。

#### 13.10.3 对原注释的更正

`seed_variables.rs:28-35` 的诊断写「**只改默认值不够，必须经 `force_variable_value` 覆写存量**」，
并归因于 `merge_variable_values` 的「无条件保留旧值」。

**归因只对了一半**：`merge_variable_values`（`:4641`）确实保留旧值，但**真正致命的是 `:5043`** ——
它位于 force **之后**，把 force 刚写的新值再覆盖回旧的。⇒ 注释里「实测 688114 `dcf.mid`
24.23 → 37.27（+53.8%）」的推算**在生产路径上不成立**（那是按「force 生效」假设算的）。

#### 13.10.4 影响面（**比 D8 更广**）

凡「靠 `force_variable_value` 把存量变量值迁移到新默认值」的修复**全部失效**。当前已知两处：

| force 调用 | 目的 | 实际 |
|---|---|---|
| `debate_rounds`（`:4654`） | 变量表与建图轮数同源 | **无害** —— 建图走 `resolve_debate_rounds(old_variables)`（`:2281`）也用旧值 ⇒ 两侧同源，覆盖前后同值 |
| DCF 三参数（`:4663-4677`，v74） | 校正存量陈旧值 | **完全落空** —— 这正是 `8/3/10` 仍在的原因 |

⇒ 判据：**`force_variable_value` 只能影响「同一次种子内、落库前」的其他消费方**（如建图）；
**它无法影响最终落库值**。把它当「存量迁移工具」是错误用法。

#### 13.10.5 当前可达的正确操作（无需改码）

面板已是唯一入口（§13.3）⇒ **用户在面板上把三个 DCF 参数显式改成 `12 / 4 / 8.5` 一次即可** ——
之后任何升版都会「逐名保留旧值」把它保住（`:5043` 的语义反过来成了保护用户设置）。

#### 13.10.6 修法论证：`:5043` 的覆盖是**冗余实现**，可安全删除

原先我把这件事判为「产品语义、需裁决」。**再核一遍后不成立** —— 分三步论证：

**第一步：目标是既定决策，不是待选项。** `seed_variables.rs:30` 明写「这组值 2026-09-12 就校准过
（原 10% / 3% / 8% → 8.5% / 4% / 12%）」⇒ 「DCF 参数应取 `12/4/8.5`」**早已是团队决定**，
只是实现从未生效。故「要不要迁到 `12/4/8.5`」不必再问。

**第二步：`:5043` 的覆盖与 `merge_variable_values` 语义完全重复 ⇒ 删除它行为等价。**

| | `merge_variable_values`（`:4641` 调用） | `:5043` 落库构造点 |
|---|---|---|
| 基线 | 新变量表（`build_template_variables`）| `variables_val`（= merge 的输出）|
| 动作 | 逐名查旧表，命中则 `v["value"] = 旧值` | 逐名查旧表，命中则 `insert("value", 旧值)` |
| 别名映射 `RENAME_MAP` | **有** | 无 |

逐情形对账（N = 新变量表，O = 旧变量表）：

| 情形 | merge 结果 | `:5043` 结果 | 等价？ |
|---|---|---|---|
| 名字 ∈ N∩O | 旧值 | 旧值 | ✅ |
| 名字仅 ∈ N（新增变量）| 新默认值 | 新默认值（旧表查不到 ⇒ 不覆盖）| ✅ |
| 名字仅 ∈ O（已删变量）| 不出现 | 不出现（只遍历 `variables_val`）| ✅ |
| 旧名经 `RENAME_MAP` 改名 | **能迁移** | **不能迁移**（用原生名查）| ⚠ merge 更强 |

⇒ 唯一差异在最后一行，且是 `:5043` **更弱**。⇒ 删除 `:5043` **不丢失任何能力**，
唯一净效果是 **force 不再被抹掉**。

**第三步：必须同时给 force 加「一次性」门，否则会变成每次升版重置。**
单删 `:5043` ⇒ force 生效 ⇒ v76 校正一次 ✓；但 v77 升版时 force 会再次把用户
在面板调过的值打回 `12/4/8.5` ✗（这正是被作废的 D9 所担心的行为）。
⇒ 加门：**仅当旧版本 < `DCF_MIGRATION_VERSION`（= 76）时执行**，此后不再执行。

⇒ 结论：**修法 = ① 删除 `:5043` 的覆盖分支 + ② 给 DCF 三处 force 包一次性门。**
两步都在调用侧，不触碰引擎；② 保证「校正一次、此后尊重用户设置」，与 v74 的原意一致。

### 13.11 新登记 D11（并入 D9）

| 编号 | 缺陷 | 位置 | 证据 | 处置 |
|---|---|---|---|---|
| **D11** | **`force_variable_value` 的校正效果被落库端「逐名保留旧值」整段覆盖 ⇒ 一切「用 force 迁移存量变量值」的修复都是假修复** | `seed_stock_analysis.rs:5043`（覆盖）vs `:4663-4677`（force） | DB 活行 `version=75`（种子跑过多次）却仍是 `8/3/10`，与代码目标 `12/4/8.5` 不符 ⇒ force 从未反映到落库 | **已定位可修**：`:5043` 与 `merge_variable_values` 语义完全重复（§13.10.6 逐情形对账）⇒ 删除行为等价；配 force 一次性门 ⇒ 校正一次且不覆盖用户设置。**修复见 §13.12** |

**与 D9 的关系**：D9 原描述为「面板参数**每次升版被静默重置**」—— 经本节实测，**该描述不成立**
（实际是「**升版从不改变变量值**」）⇒ **D9 作废并入 D11**。
D11 的处置**不需裁决**：目标值 `12/4/8.5` 是 2026-09-12 的既定校准决定，`:5043` 是冗余实现
（§13.10.6）⇒ 采用「① 删冗余覆盖 + ② force 一次性门」，落地记录见 §13.12。

### 13.12 D11 修复实施（2026-09-22 深夜）

**改动 4 处（全在调用侧，引擎一行未改）**：

| # | 位置 | 改动 |
|---|---|---|
| G1 | `seed_stock_analysis.rs:1000-1003` | 新增 `previous_version: Option<i32>`（照抄既有 `old_variables` 的「函数体外声明以延长生命周期」模式，`None` = 首次创建）|
| G2 | 同文件 `:1082-1085` | `previous_version = Some(existing.version)`；**并更正**原注释「用 UPDATE 替代 DELETE」—— 实现是 DELETE + INSERT |
| G3 | 同文件 `:4656-4700` | DCF 三处 force 包进 `if previous_version.is_none_or(\|v\| v < DCF_MIGRATION_VERSION /* =76 */)`，并写清「为何必须一次性」「为何门锚在旧版本」两条理由 |
| G4 | 同文件落库处 `:5063-5069` | **删除**「再用 `old_variables` 逐名覆盖 `value`」的冗余分支 ⇒ 简化为 `variables: Set(Some(variables_val))` |

**修后语义（三步自洽）**：

1. **v76 重种**：`previous_version = 75 < 76` ⇒ force 执行 ⇒ `value_dcf_*` = `12/4/8.5`
   ⇒ **2026-09-12 的校准首次真正生效**（此前一直是假修复）；
2. 用户在面板改这三个参数 ⇒ 落库为新值；
3. **此后任何升版**：`previous_version >= 76` ⇒ force 不执行，`variables` 由 `merge_variable_values`
   逐名保留 ⇒ **用户设置被保住** ✓（即不再有 D9 所描述的那种行为）。

**回归风险与对账**：

- `old_variables` 仍被 `resolve_debate_rounds`（`:2287`）与 `merge_variable_values`（`:4647`）使用
  ⇒ **不会变成 unused**（`grep` 已证）；
- `debate_rounds` 的 force（`:4659`）**刻意不加门**：它写的是「从旧变量解析出的轮数」（同源），
  属同值覆写，加不加门行为一致 ⇒ 不加门可减少一次判断；
- G4 删除的等价性由 §13.10.6 的逐情形对账保证（唯一差异在 `RENAME_MAP` 别名迁移上，而 merge 更强）。

### 13.13 门禁与并发会话污染（必须分开记）

| 门禁命令 | 结果 |
|---|---|
| `cargo fmt --all --check`（D8 修复后，23:47 左右）| **EXIT=0** |
| `cargo fmt --all --check`（D11 修复后，23:55）| **EXIT=1**，但 diff 的 3 个文件**全部不在本次改动清单内** |
| `cargo clippy --workspace --all-targets --all-features -- -D warnings` | 见 §14 |
| `__TAURI_WORKSPACE__=true cargo test --workspace` | 见 §14 |

**⚠️ 并发会话污染（归因，不是我的改动）**：同一台机器上同时有**至少两个其它 cargo 会话**
在操作同一个 `target/`（进程表取证，只按 `CommandLine` 判归属）：

| PID | 命令 | 归属 |
|---|---|---|
| 25232 / 36704 | `cargo run --no-default-features --features notification,computer-use` | 外部会话 |
| 25448 | `cargo check --workspace --all-targets --all-features` | 外部会话 |
| 17252 / 2128 | `cargo clippy --workspace --all-targets --all-features -- -D warnings` | **本次（我）** |

⇒ 三条后果，必须分开记，**不可相互归因**：

1. **我的 cargo 命令会长时间「Blocking waiting for file lock」** —— 实测排队 10 分钟以上，
   **排队里可能一条都不是自己的**（判归属只看 `CommandLine`）；
2. **`cargo fmt --check` 由绿转红的 3 个文件**（`analysis-engine/src/stock_adaptive_engine.rs:259`、
   `analysis-engine/src/stock_self_evolution.rs:239`、`entities/src/stock_evolution_history.rs:34`）
   是**并发会话正在编辑的特征** —— 8 分钟前同一命令 EXIT=0，期间我只改了
   `seed_stock_analysis.rs` 与 `init/state.rs`（两者均**格式合规**，不在 diff 内）
   ⇒ 按纪律**只登记、不代改**（改别人的半成品会造成更大混乱）；
3. **clippy 结果可能掺入他人中间态** —— 本轮 clippy 有相当一段时间在 `Checking axagent-analysis-engine`，
   而该 crate 正是上述被并发编辑的目录 ⇒ 若 clippy 在该 crate 报错，**须先归因再定责**。

#### 13.13.1 首轮 clippy 的 6 个 error：归因与修复（**2 个本次造成 / 4 个既有欠账**）

首轮（日志 `output/tmp/clippy-d8-full.log`，覆盖 F1–F5 + E0507 修复，**不含 D11**）：
`CLIPPY_D8_EXIT=101`，`error: could not compile axagent (lib) due to 6 previous errors`。

> ⚠️ **先记一条方法教训**：我曾据「clippy 还在 `Checking axagent-runtime`」判断「主 crate 尚未开始编译
> ⇒ 此刻改源码安全」。**该判断是错的** —— 主 crate 产物的 `.d` 文件 mtime 是 **23:52:37**，
> 而 D11 改动在 **23:56** ⇒ 那一轮 clippy **不含 D11**。
> ⇒ **判据：要判断「编译是否包含我的改动」，不能看日志末行停在哪，要看产物的 mtime**
> （`ls -lt target/debug/deps/axagent_lib-*.d`）与改动时刻的先后。

| # | 位置 | lint | 归因 | 处置 |
|---|---|---|---|---|
| 1 | `commands/workflows/mod.rs:100` | `unused_mut` | **本次造成**：F2 重写函数体后，参数 `variables` 由「被 mutate」变为**只读**（`if let Some(ref front_vars) = variables`）⇒ `mut` 多余 | 去掉 `mut` ✅ |
| 2 | `init/state.rs:1450-1451` | `field_reassign_with_default` | **本次造成**：F5 写成 `RunOptions::default()` 后跟 `opts.input = input` | 改结构体初始化器 `RunOptions { input, ..Default::default() }`；`mut` 保留（下方命中变量时仍整体重绑 `opts`）✅ |
| 3–6 | `seed_stock_analysis.rs:242-245` | `doc_lazy_continuation` ×4 | **既有欠账**（不在本次改动范围：内容是 `positionState` / `必须升版` 的说明，属 v76 升版注释）—— 编号列表 `1./2./3.` 之后紧跟**非缩进** `///` 行，被 clippy 判为「列表项续行缺缩进」 | 按 clippy 第一条建议**加空 `///` 作段落分隔**（语义中性：一个字未改，只补分隔）✅ |

**⇒ 两条可复用判据**：

1. **改函数体 ⇒ 顺手复查参数/局部变量的 `mut`**：把「写入」改成「只读」后，`mut` 会立刻变成
   `unused_mut`（而它**只在 clippy 下暴露**，`cargo check` 不管）；
2. **`Default::default()` 后接字段赋值 = `field_reassign_with_default`** —— 一律写成结构体初始化器
   （`Foo { 字段, ..Default::default() }`）。

**⚠️「首轮不完整」的又一实例**：`doc_lazy_continuation` 在本仓**不止一处**
（2026-09-21 已在 `stock_workflow/decision.rs:1265` 记过同族），但 clippy **遇首个 error 即短路**
⇒ 每修完一批才能暴露下一批 ⇒ **必须全量重跑到干净**，不可由「只剩 N 个 error」推定其余干净。



