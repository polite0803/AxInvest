# AUDIT — 601166 估值/决策/分析师产出偏离根因（2026-09-14）

> **触发**：用户重跑 601166（兴业银行）后反馈「现价 18.15 元，估值给 40.17–49.26 元，
> 决策观望 + 持仓 0%」，另报消息面/资金面/情绪面三个分析师产出异常。
>
> **取证入口**：`workflow_executions.id = a5fb7125-c1b7-45f3-852e-06cf36486d70`
> （18:27 运行，`total_time_ms = 561000`，61 节点 = 55 completed + 3 failed + 3 timeout）
> 取证脚本：`output/q-601166-a.mjs` … `output/q-601166-g.mjs`

---

## 0. 先确认：上一轮修复确实生效了

| 检查 | 结果 |
|---|---|
| `bear-r1` 是否有产出 | ✅ 93631ms，3616 字符 ⇒ 悬空边修复 + `max_rounds=1` 落地 |
| B1 子节点级重试是否真触发 | ✅ `a-market-analyst` / `a-sentiment` / `value-investor` 三个节点**首次 504 → 重试成功**（同一 `node_id` 在 `node_executions` 中出现两次） |
| 是否存在 skipped / 断链 | ✅ 无 |

⇒ 「启动失败 / 节点不存在」类故障已闭环。本轮问题在**下游语义层**。

---

## 1. 缺陷 D1（P0）— ROE 未年化：整条看空链建立在一个口径错误上

### 现象

`t-risk` 输出 `stockRiskProfile.roeTTMPct = 4.8`。字段名写着 `TTM`，
实际取值在 `crates/astock-data/src/mcp_tools.rs:1654`：

```rust
let roe_ttm_pct = fin.and_then(|f| f.roe).map(|v| (v * 10.0).round() / 10.0);
//                 ^^^ financials.first() —— 中报口径下这是「年内累计值」（半年）
```

**同一份输出里，`peTTM = 5.09` 用的是年化 EPS。两个口径并列。**

### 核算（601166 / 2026-06-30）

| 项 | 值 | 口径 |
|---|---|---|
| 归母净利 | 411.31 亿 | 半年累计 |
| 净资产 | ≈ 8350 亿（3841 亿市值 ÷ PB 0.46） | 期末 |
| ROE | **4.93%** | 半年 |
| ROE 年化 | **≈ 9.8%** | 年度 |

银行业 ROE 正常区间 10–12%。**9.8% 落在下沿，而非「严重偏离」。**

### 污染面（逐条可在 DB `node_executions` 与源码核对）

| 消费点 | 位置 | 后果 |
|---|---|---|
| `health_score` 分档 | `fundamentals_report.rs:171-181` | `roe >= 5.0 → 10 分`；4.8 只给 **5 分** ⇒ health 33/100 |
| 基本面报告文本 | `fundamentals_report.rs:259` | `ROE = 4.8%` 直出给 LLM |
| 公式风险判定 | `portfolio-mgr.rhai:438` | `is_high` 含 `roe < RISK_ROE_HIGH(5.0)`，4.8 命中 |
| 风险 LLM ×3 | `risk-agg` / `risk-neu` / `risk-con` | **三份报告全部**引用「ROE 4.8% 严重偏离行业均值 10-12%」 |
| 综合风险 | `research-mgr` | 「基本面风险：高（Critical）— ROE 4.8% 严重偏离行业均值」 |
| 辩论收敛 | `debate-convergence.decisive_bear_acks[0]` | 「ROE4.8%属盈利能力根本性恶化信号」 |
| 基本面分析员 | `a-fundamentals.bear_points[0]` | 「ROE严重下滑至4.8%」 |

### 修复（本轮已实施）

新增 `annualized_roe()`（`mcp_tools.rs`），与既有 `annualized_eps` **同构**：

1. 最新为年报 → 直接使用
2. 中报/季报 → `上年年报 ROE + 本期累计 ROE − 上年同期累计 ROE`
3. 缺上年同期 → 按报告期月份数粗年化（`ROE × 12 / months`）
4. 全部不可用 → `None`（调用方**保留原值**，不以猜测值冒充）

两处消费点改为调用它：
- `mcp_tools.rs:1654`（`compute_portfolio_risk` 的 `roeTTMPct`）
- `fundamentals_report.rs` 的 `generate`（覆盖 `compute_ratios` 的单期取值）
  —— 采用「先算后覆盖」而非改签名，避免波及 3 处测试调用点。

补 4 个测试，含 601166 真实形态 `9.6 + 4.8 − 5.0 = 9.4`，并断言还原值必须越过
`RISK_ROE_HIGH(5.0)`（否则修复对该风险判定无意义，测试应失败而非静默通过）。

---

## 2. 缺陷 D2（P0）— DCF 对银行股结构性失效

### 现场输出

```json
{"low":40.17,"mid":44.13,"high":49.26,"upsidePct":143.1,"available":true,
 "assumptions":{"basis":"当期FCF≤0，改用近5年报正净利均值×0.90归一化锚定（周期底部）",
   "fcf_anchor":73052460000,"fcf_per_share":3.4519,"discount_rate":0.1,
   "perpetual_growth":0.03,"high_perpetual":0.039,"growth":-0.0025,
   "total_shares":21162865013.77,"is_fallback_anchor":true}}
```
```json
"value_signal":"低估"  /  "margin_of_safety":{"level":"充足","pct":58.9}
```

### 复算 `compute_dcf` 的 `dcf_two_stage`（精确复现 mid = 44.13）

`fcf_ps = 3.4519, g = -0.0025, p = 0.03, d = 0.10, forecast_years = 5`

| 段 | 值 | 占比 |
|---|---|---|
| 预测期 FCF 现值合计 | ≈ 12.99 | 29.4% |
| **永续终值现值** | **≈ 31.14** | **70.6%** |

终值 = `3.4082 × 1.03 / (0.10 − 0.03)` = 50.15，再折现 5 年 ⇒ 31.14。

### 三层缺陷

**① 把「净利润」当「自由现金流」**
银行不存在经典 FCF（存款流入即经营现金流入），且净利受资本充足率约束不能自由分配。
fallback 取 `近 5 年报正净利均值 811.7 亿 × 0.90 = 730.5 亿` 作 `fcf_anchor`。

**② assumptions 内部自相矛盾**
`growth = −0.25%`（未来 5 年净利继续缩）与 `perpetual_growth = +3%`（第 6 年起永久增长）
并存。没有任何规则约束二者的方向一致性。

**③ 隐含 PE 与市场定价差 2.8 倍**
`44.13 ÷ 3.107 ≈ 14.2 倍` vs 市场 `PE 5.09`。
对 ROE（年化）9.8%、净利同比 −4.7%、资产负债率 91.6% 的银行给出 14 倍 PE。

### 同类反例（同一模型，同一天）

| 标的 | 股价 | DCF mid | `upsidePct` |
|---|---|---|---|
| 601166 兴业银行 | 18.15 | 44.13 | **+143.1%** |
| 601698 中国卫通 | 23.77 | 2.32 | **−90.8%** |

⇒ 同一套 fallback 逻辑，一只给 +143%、一只给 −90.8%。
**问题不是「太保守」或「太乐观」，而是该公式没有行业适配、终值占比 70%，
对任何标的都会产生极端偏离。**

### 为什么用户会看到「自相矛盾」

`is_fallback_anchor = true` 只让 `portfolio-mgr.rhai:668` 把 f5 的 σ 乘 0.5，
**但 `value_signal: "低估"` 与 `margin_of_safety: "充足 58.9%"` 照原样输出到面板**，
且没有任何「该估值不可信」的标注。用户于是同时看到「低估 143%」与「观望 0%」。

**状态：未修，待产品裁决（见 §5）。**

---

## 3. 缺陷 D3（P0）— 分析师 VERDICT 被截断 ⇒ 结构化数据静默为 `null`

### 铁证：`analyst-brief.input_params`

```json
"a_news_raw": null,
"a_hot_money_raw": null     ← 10 个分析员里仅这两个为 null
```

`analyst-brief` 聚合文本中写出：

```
#### 新闻公告
**数据不可用**（该分析师报告解析失败，无法提取评分）
```

### 截断现场（`node_executions` 原文）

约定格式为「自然语言报告在前 + 末尾 `<!-- VERDICT: {...} -->` 标签」
（`seed_stock_analysis.rs:1629-1631`）。实际产出：

| 节点 | 尾部 | 截断点 |
|---|---|---|
| `a-news` | `...<!-- VERDICT: {"verdict": "偏空", "bull_score": 35, "bear_score": 65, "bull_points": ["中` | 断在 `"中` |
| `a-hot-money` | `...<!-- VERDICT: {"verdict": "中性", ..., "bear_points": ["大单持续` | 断在 `"大单持续` |

### 关键排除

- **不是 token 上限**：节点 `max_tokens = Some(32768)`，而 `a-hot-money` 只用了 513 output tokens。
- `a-news` 的 `usage = {input_tokens: 0, output_tokens: 0}` —— **usage 未回填 = 流式未正常结束**，
  耗时 73630ms 为全节点最长（期间有 504）。
- 节点状态仍标 `completed`（fail-open）⇒ 半截输出被当作成功。

⇒ 用户所说的「消息面没有文本输出」「资金面分析完成但未返回结构化内容」，就是这两条。
（`a-news` 实际有 980 字符正文，只是 VERDICT 不完整。）

**状态：未修。**

---

## 4. 缺陷 D4 / D5（P1）

### D4 — `cls-risk-level` 三次全 timeout，`fallback_label` 从未生效

| 尝试 | 耗时 | 结果 |
|---|---|---|
| 1 | 64927ms | timeout |
| 2 | 60005ms | timeout |
| 3 | 60017ms | timeout |

三次输出恒为空。prompt 注释记录 v9 曾因「34.95s > 30s」把 timeout 30→60s
⇒ **现在 60s 也不够了**。

`fallback_label: "中风险"` 是**解析失败**兜底，**不覆盖节点级 timeout 路径**。
下游 `risk-agg` / `risk-neu` / `risk-con` 仍全部 `completed`（未因缺 `risk-level` 报错）
⇒ 静默无产出。

### D5 — 情绪面数据管道缺口（用户反馈属实）

`t-sentiment-data` 只调 `get_social_sentiment`（股吧：`postCount 80` / `bullRatio 0.72`）。

| 维度 | 状态 | 判定 |
|---|---|---|
| 融资余额 / 杠杆 | ❌ 未接 | **管道缺失** |
| 北向资金 | ❌ 未接 | **管道缺失** |
| 期权 PCR | ❌ 不适用 | ✅ 分析员判断正确（银行股无活跃期权） |
| 龙虎榜 | ⚠️ 有数据但为 2007 / 2013 / 2024 老记录 | 该股近期未上榜，分析员如实报「缺失」是**正确行为** |

⇒ 问题在**数据管道**（未暴露融资/北向工具），不在分析员。

---

## 5. 待产品裁决

| # | 议题 | 我的建议 |
|---|---|---|
| **Q1** | **金融业（高杠杆）是否应禁用 DCF / 格雷厄姆？** | 建议**降级为参考值**：`debtRatioPct >= 80` 时 DCF 标记 `not_applicable`，`value_signal` 与 `margin_of_safety` 同步标注「不适用」；银行改用 **PB-ROE 相对估值**。依据：银行 FCF 无定义、净利受资本充足率约束、高杠杆是经营本质而非风险信号。 |
| **Q2** | **`perpetual_growth` 与 `growth` 的方向一致性怎么约束？** | 建议强制 `perpetual_growth <= max(growth, 保守上限)`；`growth < 0` 时永续增长不得为正。这是**纯逻辑矛盾**，与行业无关。 |
| **Q3** | **风险阈值 `risk_debt_high = 65` / `risk_debt_extreme = 85` 对金融业是否需要行业分档？** | 需要。银行资产负债率 91.6% 必然触发「极高/高风险」必要条件，而该值对银行属正常。建议按 `sector_is_financial` 分档（该变量已在 `portfolio-mgr.rhai:428` 存在）。 |
| **Q4** | **VERDICT 截断时应当如何处理？** | 建议自动重试整节点（当前重试只覆盖 504 类错误，不覆盖「返回了但标签不完整」）；至少让 `analyst-brief` 对解析失败**显式标记 degraded** 并纳入 `data_gaps`，不再静默置 `null`。 |

---

## 6. 本轮改动清单

| 文件 | 改动 |
|---|---|
| `crates/astock-data/src/mcp_tools.rs` | 新增 `pub(crate) fn annualized_roe()`；`:1654` 改为调用它；`valuation_tests` 新增 4 个测试 |
| `crates/astock-data/src/fundamentals_report.rs` | `generate()` 用 `annualized_roe()` 覆盖 `ratios.roe`（先算后覆盖，零签名变更） |

**验证**：`__TAURI_WORKSPACE__=true cargo check -p axagent-astock-data --all-targets`
（待锁；`--all-targets` 必带，否则 `#[cfg(test)]` 不编译 = 三型假阳性）

---

## 7. 接线取证链 —— 为什么 V77 的 DCF 门**真的会生效**

「改了但没接线」是最高发的假修复。逐链取证（全部为源码事实，非推断）：

| # | 环节 | 结论 | 证据 |
|---|---|---|---|
| L1 | 后端是否真的产出该字段 | ✅ | `mcp_tools.rs:2648-2653` 构造 `DcfAssumptions { applicable, inapplicable_reason, … }`；`:1539-1552` 别名块 `dcf.assumptions` 与 `:1511` 的 `dcf_valuation.assumptions` 同源同值 |
| L2 | 字段名是 snake 还是 camel | ✅ **snake（关键）** | `mcp_tools.rs:2352` `#[derive(Debug, Clone, serde::Serialize)]` —— **无 `rename_all`** ⇒ 原样输出 `inapplicable_reason`。若按项目 DTO 惯例误写 `inapplicableReason` 会**静默取不到**。既有先例同为此形态：`seed_stock_analysis.rs:3184` 用的是 `assumptions.is_fallback_anchor` |
| L3 | 模板有没有把该路径接进来 | ✅ | `seed_stock_analysis.rs:3195-3203` 新增两条 `input_mapping` |
| L4 | 值能进到 Rhai 作用域吗 | ✅ | `stock_analysis.rs:171-173` `push_json_to_scope` 显式处理 `Value::Bool` ⇒ `push_constant(key, *b)`。**若该分支缺失，`false` 会落空 ⇒ `present()` 为假 ⇒ 门永不触发** |
| L5 | `present(false)` 是否为真 | ✅ | `portfolio-mgr.rhai:22` `fn present(x) { type_of(x) != "()" }` ⇒ `type_of(false) == "bool"` ≠ `"()"` ⇒ **true** |
| L6 | 变量缺失时会不会抛异常 | ✅ 向后兼容 | `code_executor.rs` V57 的 `extract_present_vars()` 把 `present(...)` 里引用的外部变量补 unit 默认值（单测 `extract_local_vars_portfolio_mgr_scenario` 正是本文件场景）⇒ 旧模板（v50，无该映射）下 `present()` 为假 ⇒ `dcf_leg_usable = present(upside)` ⇒ **行为与改动前完全一致**，零破坏 |

### 7.1 反向断言：谁在消费（防「写进产物 ≠ 有人消费」）

| 消费点 | 行 | 用法 |
|---|---|---|
| `dcf_leg_usable` 定义 | `portfolio-mgr.rhai:607` | `present(upside) && !(present(applicable) && applicable == false)` |
| `dcf_sig` | `:621` | `if dcf_leg_usable { pm_saturate(…) } else { 0.0 }` |
| `fusion_wsum` | `:646` | 按可用腿归一，单腿不缩水 |
| `fused_sig` | `:651` | 分子分母同源用 `*_leg_usable` |
| `f5_fallback_decay` | `:691` | 加 `dcf_leg_usable &&` 前置 —— 否则会用「DCF 的 FCF 锚定缺陷」去惩罚与 FCF 无关的 graham 信号 |
| `valuation_note` → `reasoning` | `:1968` / `:2039` | 让「估值说低估、决策说不买」之间有可读因果 |
| `valuationApplicability` | `:2105-2110` | 结构化输出 `dcfApplicable / dcfLegUsed / grahamLegUsed / reason` |

### 7.2 一个刻意保留的双消费者事实（不是漂移）

`portfolio-mgr.rhai` 有**两条** `include_str!` 消费路径：

- `seed_stock_analysis.rs:3087` → 写进 DB 模板 CodeNode.code（DAG 运行期）
- `stock_analysis.rs:265` → What-If 回测

同一份文件 ⇒ 两条路径自动一致。**但这也意味着改 rhai 必须升 `TEMPLATE_VERSION`**（见 `AUDIT-industry-label-removal-2026-09-14.md` 的自述更正：v50 注释里写的「rhai 是编译产物、不需升版」是错的）。
