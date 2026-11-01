# 估值结论严重偏离现价：参数被 DB 旧值覆盖 + 适用性判据对亏损公司失明

> 报告类型：根因定位 + 修复（本次 = 模板 v74 段；因并发会话已推到 **v75**，落库版本为 75，
> 两批改动同时进 DB —— 见 §7.1 与 §8.9）
> 日期：2026-09-21
> 样本：`688114` 华大智造（`stock_analyses.id = 4fd89367-21b1-41c7-a1a8-1760d02f8ec0`，
> `template_version = 73`，创建 21:00:48，`decision_action = 观望`）
> 关联：`AUDIT-dcf-cashflow-anchor-2026-09-21.md`（前置的字段级死链修复，同日）

## 〇、结论摘要

用户报「华大智造现价 73.11 元，而估值结论 16.33–38.84 元」，并明确指出
**这个问题已经修复过多次**。

**首先排除计算错误**：DCF 的锚、每股值、三档估值、终值占比**全部可逐位复现**，
公式与 TTM 还原均无缺陷（§二）。偏差来自三处独立缺陷叠加：

| 层 | 缺陷 | 性质 | 本次处置 |
|---|---|---|---|
| 1 | **DB 变量旧值覆盖了 2026-09-12 的校准值**（10/3/8 vs 8.5/4/12） | **纯 bug**，且是「修了几次都没用」的直接原因 | ✅ 已修（v74 强制覆写） |
| 2 | 适用性判据 ② 对**净利为负**的公司整段短路 | 设计缺口（判据形态清单不完整） | ✅ 已修（新增判据 ④） |
| 3 | DCF 口径对「投入期/转型期」公司本身不成立 | **方法错配**，需产品裁决 | ⏸ 未处置（登记） |
| 4 | `value_signal` 未纳入 `applicable` | 一致性缺口，需产品裁决 | ⏸ 未处置（登记，§六） |
| — | **层 1 修复方式本身的副作用**：`force` 覆写会清零用户在 Settings 的手动调参 | 产品语义决策 | ⏸ 登记待裁决（§3.5） |

---

## 一、为什么这次与以往不同：修复**暴露**了缺口，而非修复失败

`AUDIT-dcf-cashflow-anchor-2026-09-21.md` 修复的是
`FinancialReport.operating_cash_flow / capital_expenditure / free_cash_flow`
在生产链路上**恒为 `None`** ⇒ `compute_dcf` 只能走「近 5 年报正净利均值 × 0.90」代理锚。

该修复（F1，东财 `xjllbAjaxNew` 现金流量表）**已经生效**，本样本的
`basis = "当期FCF"`、`is_fallback_anchor = false` 即为证据。

但生效的后果是：锚从「历史**正**净利均值」（华大智造含 2022 年新冠检测暴利年，
量级很大）**换成真实 TTM FCF（3.588 亿，相对 304 亿市值极小）**
⇒ 估值从「接近/高于现价」下移到 24.23 元，**看多翻看空**。

⇒ 用户此刻看到的错，是**修复把另一侧的问题照出来了**。
这正是 DETAIL §134.6 Q2（「修好上游反而放大另一腿的权重」）的实例。

---

## 二、复现：公式、TTM 还原、单位全部正确

取值路径（注意 `t-valuation` 在黑板顶层是 **string**，须二次 parse）：

```sql
select ((blackboard_snapshot::jsonb ->> 't-valuation')::jsonb -> 'result' ->> 'content')::jsonb
from stock_analyses where stock_code = '688114';
```

| 量 | 产出值 | 复算 |
|---|---|---|
| `fcf_anchor` | 358,757,150.02 | 2025FY(2.1898−1.9449) + 2026H1(3.0033−0.106) − 2025H1(−2.54−2.094) = **3.588 亿** ✓ |
| `fcf_per_share` | 0.86316 | 3.5876e8 / 415,633,976.2 ✓ |
| `total_mv` | 303.87 亿 | 73.11 × 4.156 亿 ✓ |
| `mid` | 24.23 | 5 年 PV 5.485 + 终值 PV 18.748 = **24.233** ✓ |
| `terminal_value_ratio` | 0.7737 | 18.748 / 24.233 ✓ |
| `owner_earnings_yield_pct` | 1.2 | FCF 收益率 3.588/303.87 = **1.18%** ✓ |

公开财报口径核对（东财 / 巨潮）：

| 期 | OCF | capex | FCF |
|---|---|---|---|
| 2025FY | +2.1898 亿 | −1.9449 亿 | +0.2449 亿 |
| 2026H1 | +3.0033 亿 | −0.106 亿 | +2.897 亿 |
| 2025H1 | −2.54 亿 | −2.094 亿 | −0.446 亿 |

⇒ TTM 还原正确。**结论：不要在公式层继续找问题。**

---

## 三、层 1：DB 变量旧值覆盖校准值（「修了几次都没用」的答案）

### 3.1 事实

`mcp_tools.rs` 与 `seed_variables.rs` 双处的默认值早在 2026-09-12 已校准，
代码注释写明理由（原文）：

| 常量注释 | 校准 |
|---|---|
| 「永续增长率 `PERPETUAL_GROWTH = 4%`：接近长期名义 GDP 增速（**原 3% 偏低**）」 | 3 → 4 |
| 「折现率 `DISCOUNT_RATE = 8.5%`：无风险利率 2.5% + 6% 风险溢价（**原 10% 偏高**）」 | 10 → 8.5 |
| 「默认增长率 `DEFAULT_GROWTH = 12%`：优秀公司平均增速（**原 8% 偏保守，系统性低估成长股**）」 | 8 → 12 |

但 DB 实测（`workflow_templates` where `name='A股多维度分析'`）：

| 变量 | DB 实际 | 校准值 | 方向 |
|---|---|---|---|
| `value_dcf_discount_rate` | **10** | 8.5 | 压低估值 |
| `value_dcf_perpetual_rate` | **3** | 4 | 压低估值 |
| `value_dcf_growth_rate` | **8** | 12 | 压低估值 |

**三个值一个不差，全是校准前的原值。** 三个方向**一致**压低估值 ——
即那次校准的目标（「消除系统性低估成长股」）**从未生效**。

### 3.2 机制

`stock_analysis_setup/mod.rs::merge_variable_values` 的语义是
**「新定义 + 无条件保留旧值」**（`mod.rs:1222-1223` 注释原文）。
其自身文档（`:1334-1336`）已经写明这个陷阱：

> 这对「用户自己调过的参数」是正确的，但对**语义发生变化的变量**是错的：
> DB 里那个旧值本身就是本次要修掉的东西，不覆写就等于没改。
> 首个用例（v48，2026-09-14）：`debate_rounds` 由 3 固定为 1。
> 只改 `seed_variables.rs` 的默认值无效，因为 DB 里的 3 会在升级时覆盖回来。

⇒ **改代码常量与种子默认值，对「参数类」变量是假修复**（与判据「改生成物 = 假修复」同族）。
项目已有 `force_variable_value` 专治此症，但**这三个 DCF 参数从未被 force 过**。

### 3.3 量化

| 参数组合 | `low` / `mid` / `high` | `upsidePct`（现价 73.11） |
|---|---|---|
| DB 现值 10% / 3%（本次前） | 16.33 / **24.23** / 38.84 | −66.9% |
| 校准值 8.5% / 4%（本次后） | 22.39 / **37.27** / 71.14 | −49.0% |

⇒ `mid` **+53.8%**；`high` 71.14 已贴住现价 73.11。

### 3.4 修复（v74）

在变量合并之后、建图之前强制覆写（法同 `debate_rounds`）：

```rust
let variables_val = force_variable_value(
    &variables_val,
    "value_dcf_discount_rate",
    serde_json::json!(super::seed_variables::DEFAULT_DCF_DISCOUNT_RATE_PCT),
);
// perpetual / growth 同法
```

取值一律引用 `seed_variables` 的常量（新增三个 `DEFAULT_DCF_*_PCT` 单一权威源），
**不在调用处手抄数值** —— 避免「同名散成多套值」。
`TEMPLATE_VERSION` 73 → **74**（版本门是 `>=`，不升版整个种子流程被跳过；
该常量后被并发会话推进到 75，见 §7.1）。

### 3.5 ⚠️ 副作用登记：`force` 覆写会清掉用户的手动调参（**待裁决，未擅改**）

「强制覆写」是本次达成修复的**唯一**手段（合并语义决定默认值无效），
但它有一个真实代价，**必须先说清再让用户知情**：

| 项 | 事实 |
|---|---|
| 这三个参数**是否对用户可见** | **是** —— `seed_stock_analysis/mod.rs:1270-1272` 有 `("value_dcfDiscountRate", "value_dcf_discount_rate")` 等三条扁平映射，前端 Settings 面板可改 |
| 是否在反思 LLM 的可调清单里 | **否** —— 不在 `PORTFOLIO_MGR_TUNABLE_PARAMS`（28 项，只含 `risk_*` / `action_*` / `pos_*` / `regime_*` / `cost_pct` / `trader_cap_min_weight`）⇒ 冲突面**不涉及** LLM 自动调参建议 |
| 代价 | 此后**每次 `TEMPLATE_VERSION` 升版重种**都会把这三个值重置为常量值。用户若在 Settings 里调成（例如）折现率 7.0，下次升级即被回退到 8.5 |

> **⚠ 订正（2026-09-23，v77 施工时核对）**：上行的「每次升版都会重置」**与实际实现不符**。
> 代码的门是 `previous_version < DCF_MIGRATION_VERSION`（`seed_stock_analysis.rs:4786`），
> 而 `DCF_MIGRATION_VERSION` 是**固定常量** ⇒ 一旦 DB 版本升到该值，此后升版**不再**覆写。
> 即**门是一次性的**（实现当时的注释 `:4765-4769` 已写明该设计意图：若无条件执行，才会
> 「每次升版都打回默认」）。
> ⇒ 本行**高估了代价**，影响 §3.5 待裁决项的取舍依据。**结论方向不变（仍应登记为产品语义决策），
> 但代价量级须按此修正为「仅从旧版本升级到该版本这一次」。**
> 另注：v77（2026-09-23）再次使用无条件 `force`，理由与 DB 实测见
> `AUDIT-valuation-model-scientific-soundness-2026-09-23.md` §7.2。

**与先例 v48 `debate_rounds` 的语义差别（重要）**：v48 覆写的是
`debate_max_rounds` —— 一个**从用户设置派生出来的值**（`resolve_debate_rounds`），
故它是「回写用户意图」，不破坏调参；而本次覆写的是**常量**，会覆盖用户意图。
⇒ 两者同形但**不同语义**，不可据先例推定本次无副作用。

**可选替代（更保守，本次未采用）**：条件式覆写 —— 仅当存量值 ∈ `{10, 3, 8}`
（校准前原值）时才改写。它在本机产出**完全相同**的结果（DB 现值恰为 10/3/8），
同时保留用户调参；代价是逻辑多一个分支、且若用户恰好「故意」设成 8/3/10 则会被改掉。
**属产品语义决策，登记待裁决，不擅自扩大改动面。**

---

## 四、层 2：适用性判据对「净利为负」失明

### 4.1 缺陷

`mcp_tools.rs` 判据 ② 的全部逻辑挂在：

```rust
if let Some(np) = latest.net_profit.filter(|v| *v > 0.0) {
    match direct_fcf { /* 符号相反 / 量级脱钩 0.3 */ }
}
```

两条形态（净利 > 0 而 FCF ≤ 0；`0 < FCF/净利 < 0.3`）**都预设净利为正** ⇒
**公司净利为负时整段短路**。

688114：`pe = −144.08`、ROE −3.0%、净利 −2.22 亿 ⇒ 判据 ② 不执行；
判据 ①（负债率 29% < 80%）不触发；判据 ③（growth +18.91% > 0）不触发
⇒ **`applicable = true`、`applicability_signals = []`**。

### 4.2 反向不公（同日对照，决定性）

| 标的 | 净利 | FCF/净利 | 命中判据 | `applicable` |
|---|---|---|---|---|
| 300308 中际旭创 | **+204 亿** | 0.14 | ② 量级脱钩 | `false` ✅ 拦住 |
| 688114 华大智造 | **−2.22 亿** | ② 短路 | 无 | `true` ❌ 放行 |

**差别只在净利符号。** 净利为正但现金流差的公司被拦下，
**真亏损**（更该质疑「拿当期 FCF 折现」前提）的反被放行。

### 4.3 后果链

`applicable = true` + `is_fallback_anchor = false`
⇒ `portfolio-mgr.rhai` 的 `dcf_anchor_decay = 1.0` ⇒ **DCF 腿全额权重进 f5**；
同时 `value-investor.md:146-152` 的硬不可用只有
① 持续亏损（FCF≤0 **且**近 5 年无正净利）② `applicable == false`，
本例两条都不命中 ⇒ prompt 指示**「直接引用」**
⇒ `intrinsic_value_range = "16.33-38.84元"`、`margin_of_safety = −201.7`、
`buffett_verdict = 【减持】`，并逐字进 `risk_flags`
（**与 DETAIL §134「假诊断文案被引用放大」同型**）。

### 4.4 修复：新增判据 ④

形态（**两条并列，缺一不可**）：

- 当期净利 ≤ 0 —— 公司尚未证明其商业模式能产生利润；
- 锚定 FCF / 市值 < `FCF_YIELD_INAPPLICABLE_MIN`（**3%**）—— 当期现金流不具定价意义。

⚠️ **不能**简化成「净利 ≤ 0 即不适用」：那会误伤「一次性减值致亏、
但经营现金流充沛」的公司（FCF 收益率 20% 时 DCF 完全成立），
也会与 V74「周期底部用历史正净利归一化锚定」的设计**直接冲突**。

阈值依据：正常经营企业 FCF 收益率 3–8%；折现率 10% 意味着
「零增长 + 零再投资」的理论收益率下限约 10%。取 3% 已属宽松，
只拦「相对市值小到不具备定价意义」的极端形态（688114 的 1.18%，市值/FCF ≈ 85 倍）。

同时把此前**传入但未使用**的 `compute_dcf` 形参 `_current_price` 启用为 `current_price`
（判据 ④ 需要它以计算市值）—— 零接口变更。

缺数不命中（`net_profit == None` / 市值非正），与判据 ② 同口径：缺数据 ≠ 模型不成立。

### 4.5 修复后的传导（已验证链路存在）

| 环节 | 位置 | 效果 |
|---|---|---|
| f5 因子 | `portfolio-mgr.rhai:636` | `applicable=false` ⇒ **整腿剔除 DCF**，由 graham 腿独立承担 |
| 诊断文案 | `portfolio-mgr.rhai:2255-2257` | `inapplicable_reason` 拼进说明 |
| 前端 | `portfolio-mgr.rhai:2423-2426` | 输出 `dcfApplicable` + `reason`（Q1(c) 已接线的 Alert） |
| LLM | `value-investor.md:152` | 硬不可用 ② 命中 ⇒ `intrinsic_value_range` / `margin_of_safety` 填 **`null`**，`ideal_buy_price` 写「无算法估值锚」 |

⇒ **`16.33-38.84` 与 `−201.7%` 不再流入结论**，且**无需改 prompt**（条件 ② 复用既有条款）。

---

## 五、层 3：方法错配（未处置，登记）

FCF 收益率 1.18% 的公司，**任何** DCF 都会判「贵」：静态上限
`3.588 / (0.10 − 0.03) = 51 亿` ⇒ 12.33 元/股。

但市场定价显然不基于当期 FCF —— 华大智造 2026H1 营收 +18.91%、
多组学 +44.91%、药厂及药检领域 +114.94%，定价包含成长期权与平台价值。

判据 ④ 拦下的正是这种「当期现金流不具定价意义」的形态，
使结论**不再被当作证据**；但「这类公司该用什么口径估值」（PS / 管线价值 / EV-EBITDA /
标准化长期盈利）**属于产品决策**，本次不擅改。

另注：`terminal_value_ratio = 0.774` ⇒ 77.4% 的估值来自永续假设，
而预测期只有 `forecast_years = 5` 年 —— 对成长股而言预测期偏短，同属口径问题。

---

## 六、层 4：`value_signal` 未纳入 `applicable`（未处置，登记）

`mcp_tools.rs:1466`：

```rust
let valuation_unavailable = dcf_mid.is_none() && graham_value.is_none();
```

只判「数值是否可用」，**不判 `applicable`**。故 688114 在修复后：

- LLM 层：硬不可用 ⇒ `intrinsic_value_range = null`、「无算法估值锚」✅
- 算法层：`valuation_unavailable = false`（三档有值）⇒
  `value_signal = value_signal_of(mos_pct, …)` ⇒ **仍输出「高估」** ❌

消费点两处：`ValueAssessmentPanel.tsx:473`（展示 Tag「综合判断: 高估」）、
`seed_stock_analysis.rs:2621`（prompt 里作为「算法锚」供 LLM 引用）。

⇒ 会出现「算法说高估、LLM 说无估值锚」的**新不一致**。

**未修原因**：`applicable = false` 时 `value_signal` 该显示什么有多个合理答案
（「无法估值」/「高估（模型不适用）」/ 保留原值 + 前端警示），
属**产品语义决策**，登记待裁决。**不擅自扩大改动面。**

---

## 七、改动清单

| # | 文件 | 改动 |
|---|---|---|
| 1 | `crates/astock-data/src/mcp_tools.rs` | `compute_dcf` 启用 `current_price` 形参（原 `_current_price`） |
| 2 | 同上 | 新增常量 `FCF_YIELD_INAPPLICABLE_MIN = 0.03` + 取值依据文档 |
| 3 | 同上 | 新增判据 ④（净利 ≤ 0 且 FCF 收益率 < 3% ⇒ 判不适用） |
| 4 | 同上 | 同步模块顶部判据清单（3 条 → 4 条）与 `applicable` 字段文档 |
| 5 | 同上 | 新增 2 个测试：正例（复刻 688114 四量）+ **反向对照**（净利为负但 FCF 收益率 20% 不得判不适用） |
| 6 | `commands/stock_analysis_setup/seed_variables.rs` | 新增 3 个 `DEFAULT_DCF_*_PCT` 单一权威源常量；三个变量定义改为引用常量 |
| 7 | `commands/stock_analysis_setup/seed_stock_analysis.rs` | `TEMPLATE_VERSION` 73 → **74** + v74 版本注释；变量合并后 `force_variable_value` ×3 |

### 7.1 ⚠️ 版本号已被并发会话推进到 75（本次改动**未失效**）

落笔后复查发现该常量现为 **75**，并多出一段 v75 注释
（`portfolio-mgr.rhai` PE 三态分流 / 配合上游三家 vendor 放开 `filter(|v| *v > 0.0)`）——
**这不是本次改动**，是并发会话在飞的独立改动（与 §8.2 报红的 `scoring.rs`
属同一批：它改的正是「负 PE 被抹成 None」的上游侧）。本次 v74 注释与三次
`force_variable_value`（`:4641-4655`）**均完好共存**。

**DB 侧实测**：`version` 仍为 **73** ⇒ `73 >= 75` 为假 ⇒ **版本门开着**，重种时会执行本次覆写
（详见 §8.8.2）。⚠️ 但重种**会同时带上 v75** ⇒ 复跑 688114 时**输出变化不可单独归因于本次修复**，
须按字段比对（详见 §8.9）。

**关键核对（防「版本门跳过」导致修复静默失效）**：版本门是
`existing.version >= TEMPLATE_VERSION ⇒ return`，若 DB 已被推到 75 则种子整体跳过、
覆写永不执行。实测：

```sql
select id, version from workflow_templates where id='stock-analysis';
-- stock-analysis | 73
```

⇒ DB = **73** < 常量 **75** ⇒ **门是开的**，下次触发种子即会重种并执行覆写。

**生效前置**：必须重启应用（或任何触发 `seed_stock_analysis_workflow_template` 的路径）
才会重种并执行覆写；否则 DB 里仍是 v73 + 旧参数。

---

## 八、验证

### 8.1 格式门

| 门禁 | 结果 |
|---|---|
| `cargo fmt --manifest-path src-tauri/Cargo.toml -- --check` | **EXIT=0** ✅ |

（`cargo fmt` 会读 `rustfmt.toml`；直接用 `rustfmt --edition 2024 --check` 不读配置 ⇒ 假红，见判据 #630。）

### 8.2 clippy：首跑报红，但**经取证不属于本次改动**

`cargo clippy -p axagent-astock-data --all-targets -- -D warnings` ⇒ **EXIT=101**，
2 处 error **全部**落在 `crates/astock-data/src/scoring.rs:185 / :192`：

```
error: manual `!RangeInclusive::contains` implementation
   --> crates\astock-data\src\scoring.rs:185:19
185 |         } else if pe > 50.0 || pe < 0.0 {
```

**归属判定（按 `concurrent-session-failure-attribution` 的三项证据）**：

| 证据 | 实测 | 含义 |
|---|---|---|
| 报错文件是否在本次改动清单 | **否**（本次只动 `mcp_tools.rs` / `seed_variables.rs` / `seed_stock_analysis.rs`） | 非本次引入 |
| `scoring.rs` mtime | **13:54:22** | **晚于**本次全部改动（13:21:15 / 13:24:50 / 13:25:38） |
| 该文件内容 | 注释含「2026-09-21 **放开负 PE** 之后才必须显式守卫」「资不抵债，2026-09-21 新增」 | 另一会话正在做的独立改动 |
| 进程表 | 5 组并发 cargo 共用同一 `src-tauri/target`（含 `clippy --workspace`、`test -p … axagent-astock-data …`） | 确有多会话在飞 |

⇒ **结论：该 2 error 属并发会话的在飞改动，不代改、只登记**（其改的正是
`filter(|v| *v > 0.0)` 负 PE 那族问题的上游侧，与本次 §九.2 同源，方向不冲突）。

**已知限制（当时必须如实记）**：clippy 遇首个 error 即短路 ⇒
**本次改动尚未被 clippy 检查**，不能由「只有 N 个 error」推定干净（判据 #72/#633）。

> ✅ **已解除（2026-09-21 晚间）**：两项后续 clippy 均为**独立完整运行**且 **EXIT=0**
> （`-p axagent-astock-data` 与 `-p axagent` 各一次，见 §8.7）。
> 且 §8.2 这两处红**已被并发会话自行修复** —— 归因判定得到事后验证，见 §8.7.2。

### 8.3 门禁被并发会话阻塞（环境事实，附**更正后的归因**）

用只读方式绕开（压制那条 lint，不改任何文件）：

```
cargo clippy -p axagent-astock-data --all-targets -- -D warnings -A clippy::manual_range_contains
```

该命令**长时间停在 `Blocking waiting for file lock on build directory`**（11 分钟以上）。

⇒ **已复跑完成，门禁全绿**（实测结果见 §8.7）。复跑发生在车队排空之后，
三条命令合并成**一条串行脚本**执行（避免本会话的两个 job 在同一 target dir 互踩 fingerprint、
把车队放大成 4 组）。

**⚠️ 归因更正（初判「死锁/被长期持有的锁」是错的）**：

| 初判 | 实测更正 |
|---|---|
| 锁被一个长期进程独占、等下去无意义 | **锁在推进**：`target\debug\build\aws-lc-sys-*\build-script-main.exe`（PID 26720）存活，并带**两个活跃 `cl.exe`** 在编 C ⇒ 有真实进度 |

真相是**车队效应（convoy）**，三个乘数叠加：

1. **`jobs = 1`** —— `~/.cargo/config.toml` 设了 `[build] jobs = 1` ⇒ 全程单线程编译；
2. **多会话共用同一 target dir** —— 实测同时有 3 组 `clippy --workspace --all-targets -- -D warnings`
   （21:31 / 21:55 / 22:14）、2 组 `check --workspace`、1 组 `test -p … astock-data …` 在排队；
   每轮换手都可能让 `aws-lc-sys`（已知 10+ 分钟 choke 点）重编一次 ⇒ 排队被放大到小时级；
3. 另有会话设了 `CARGO_TARGET_DIR=target-v79` 试图绕开，但该目录**尚冷**
   （无 `libaxagent_astock_data*.rlib`）⇒ 绕道无益。

**评估过的绕锁方案与否决理由**：`CARGO_TARGET_DIR` 指向新目录可完全绕锁，但
`~/.cargo/config.toml` **未接 `RUSTC_WRAPPER=sccache`**（`C:/Users/polit/.cargo/bin/sccache.exe`
存在但未接线）⇒ 冷目录意味着连 `aws-lc-sys` 在内全链从零重编，代价高于等待，故否决。

> 📌 **文档漂移（登记，非本次范围）**：`AGENTS.md` 称「`Cargo.toml` 已配置
> `[profile.dev.build-override]` 优化第三方依赖编译，**sccache 缓存 50G**」，
> 但用户级 `~/.cargo/config.toml` 里既无 `RUSTC_WRAPPER` 也无 `CARGO_TARGET_DIR`，
> 且该文件把 `jobs` 压到 **1** ⇒ 「sccache 加速」在生产路径上**不存在**，
> 实际是**单线程**编译。属判据 J（多入口文档必有一份撒谎：数值先腐烂）。

### 8.4 耗时的真实原因（供后续排期参考）

首次 clippy 实测 **51m15s**。中途用「间隔 30s 测两次 `target/debug/build/*/out` 最新 mtime」
确认为**正常编译而非卡死**（`aws-lc-sys` 的 C/汇编阶段产出 `.o`，日志不新增行属正常形态）。

⚠️ 判卡死**不数 `rustc.exe`**：build script 进程名是 `build-script-main.exe`，
C 编译阶段另有 `cl.exe` —— 只按 `rustc` 计数会得到 0 并误判卡死。

### 8.5 DB 物证：层 1 元凶的落库形态（实测）

```sql
select v->>'name' as name, v->>'value' as value
from workflow_templates t, jsonb_array_elements(t.variables::jsonb) v
where t.id='stock-analysis'
  and v->>'name' in ('value_dcf_growth_rate','value_dcf_perpetual_rate',
                     'value_dcf_discount_rate','debate_rounds')
order by 1;
```

```
debate_rounds             3
value_dcf_discount_rate   10      ← 应 8.5（校准前的原值）
value_dcf_growth_rate     8       ← 应 12 （校准前的原值）
value_dcf_perpetual_rate  3       ← 应 4  （校准前的原值）
```

⇒ 三值**逐一**等于 2026-09-12 校准前的原值，与 §3.2 的「`merge_variable_values`
无条件保留旧值」机制**完全吻合**，非巧合。
（`debate_rounds = 3` 不属缺陷：v48 起它由 `resolve_debate_rounds` 从用户设置派生后
覆写，3 是用户配置的反映。）

### 8.6 零回归排查：判据 ④ 会不会打破既有测试（逐条核对）

新增判据会改变 `applicable`，故逐一核对测试模块里**全部 6 处**
`applicable` 正向断言与 1 处**精确计数**断言（计数断言最脆）：

| 位置 | 断言 | 数据形态 | 是否受判据 ④ 影响 |
|---|---|---|---|
| `:4023` | `a.applicable` | 净利 +100 亿 | 否（净利 > 0） |
| `:4546` | `signals.len() == **2**` | 净利 **+411.31 亿** | **否**（④ 不触发 ⇒ 计数仍为 2） |
| `:4587-4588` | `a.applicable` + `signals.is_empty()` | 净利 +700 亿 | 否 |
| `:4615` | `a.applicable` | 净利 +100 亿 | 否 |
| `:4623` | `a.applicable`（FCF 缺失态） | 净利 +100 亿 | 否 |
| `:4657` | `a.applicable`（默认折现率） | 净利 **+10 亿** | 否 |

判据 ④ 的前置是 `net_profit ≤ 0`，上述全部样本净利为正（或走 `:4147` 的持续亏损
**提前 `return None`** 路径，根本不产出快照）⇒ **零回归**。

**并核实两个阈值常量不会被误触**：`LEVERAGE_INAPPLICABLE_PCT = 80.0`
（本次用例给 29.0 / 30.0）、`TERMINAL_RATIO_MAX = 0.7` 且判据 ③ 需 `growth < 0`
（本次用例给 +18.91% / +8.0%）⇒ 判据 ①③ 均不触发，测试断言的**唯一**信号必是 ④。

**取数链核对**：新用例用中报日期 `2026-06-30`，而既有同路径用例（`:4157`）用年报日期。
`ttm_fcf` 对中报先试 TTM 还原、缺段则落 `latest.free_cash_flow`（`:2421`）——
新用例只给 `free_cash_flow` 而**不给** `operating_cash_flow` ⇒ 还原不出 ⇒ 直供兜底命中，
与既有通过用例 `:3957`（同一「只给 `free_cash_flow` 的中报」形态）**同构** ⇒ 取数链无风险。

### 8.7 复跑结果（**已实测，全绿**）

三条命令合并为一条串行脚本（各日志落 `output/tmp-gate-*.log`）：

```
cargo clippy -p axagent-astock-data --all-targets -- -D warnings -A clippy::manual_range_contains
cargo test   -p axagent-astock-data --lib mcp_tools
cargo clippy -p axagent --all-targets -- -D warnings     # 覆盖 seed 两个文件
```

| # | 命令 | 结果 | 耗时 |
|---|---|---|---|
| 1 | `cargo fmt --all --check` | **EXIT=0** ✅ | 秒级 |
| 2 | `clippy -p axagent-astock-data --all-targets -- -D warnings -A clippy::manual_range_contains` | **EXIT=0** ✅ | 52m37s |
| 3 | `__TAURI_WORKSPACE__=true cargo test -p axagent-astock-data --lib mcp_tools` | **EXIT=0** ✅ **36 passed / 0 failed**（303 filtered） | 20m59s |
| 4 | `clippy -p axagent --all-targets -- -D warnings`（**无豁免**） | **EXIT=0** ✅ | 35m27s |

两个新测试**均实跑通过**，在 36 个里可见：

```
test mcp_tools::valuation_tests::dcf_inapplicable_for_loss_making_with_tiny_fcf_yield ... ok
test mcp_tools::valuation_tests::dcf_applicable_for_loss_making_with_healthy_fcf_yield ... ok
```

⇒ §8.2 那条「clippy 遇首 error 即短路 ⇒ 本次改动尚未被 lint 检查」的**已知限制已解除**：
后续两次 clippy 均为独立完整运行，EXIT=0。

#### 8.7.1 判据 ④ 的区分力（不靠变异，靠构造）

两测试**只差 FCF 量级与市值**，净利同为负，结论一 `!applicable`、一 `applicable`：

| 用例 | 净利 | FCF | 市值 | FCF 收益率 | 期望 |
|---|---|---|---|---|---|
| `…tiny_fcf_yield`（复刻 688114 四量）| −2.2214 亿 | 3.5876 亿 | 303.87 亿 | **1.18%** | `!applicable` ✅ |
| `…healthy_fcf_yield`（反向对照）| −1.0 亿 | 20.0 亿 | 100.0 亿 | **20.0%** | `applicable` ✅ |

这比「删代码看测试红」更强：它同时固定了 688114 的**真值形态**与一个**必须放行的对照形态**
（判据 E「测试数据自证区分力」）。

#### 8.7.2 §8.2 那条并发红已自行消失（**未代改的判断被证实**）

`scoring.rs:183-193` 的 `manual_range_contains` 已被**并发会话自己修复**：
`pe > 50.0 || pe < 0.0` → `!(0.0..=50.0).contains(&pe) && !pe.is_nan()`，
并补写理由「`RangeInclusive::contains` 对 NaN 恒 `false` ⇒ 会改语义，故须显式补 `!pe.is_nan()`」。

⇒ 我此前判「属并发在飞改动、**不代改**」是**正确的**：红在我方零接触的情况下自然消解。
（§8.3 的 `-A` 豁免因此成为**历史措施**，保留记录以示当时的归因依据。）

**不要**改 `scoring.rs`（属并发会话在飞改动）。

#### 8.7.3 「撤掉 `-A` 的无豁免复跑」为何**不单独跑**（有意决定，非遗漏）

第 2 条 clippy 带了 `-A clippy::manual_range_contains`。事后该红已被属主修复（§8.7.2），
技术上可撤 `-A` 重跑，**但决定不跑**，三条理由：

1. **标志一变 ⇒ 整条依赖链指纹失效** —— 实测撤 `-A` 后 clippy 标志全集变化，
   等价于再付一次全链重检（与第 2 条同为 ~50 min 量级）。
2. **该结论已由他方覆盖** —— 并发会话在跑的 `cargo clippy --workspace --all-targets -- -D warnings`
   使用**完全相同的标志**，其 workspace 面**必然包含** `axagent-astock-data`
   ⇒ 无豁免结论由它给出；我重复跑只增加抢锁。
3. **代价不对称，且落在用户身上** —— `~/.cargo/config.toml` 的 `jobs = 1` 使构建**严格串行**，
   而实测此刻用户正排着一条 `cargo run`（应用启动）在**同一锁队列**里，
   那正是触发重种、产出**端到端验证**的命令。多排一条 50 分钟任务 = **直接推迟修复生效**。
   ⇒ **优先级判定：让应用启动 > 跑一条冗余门禁。**

**豁免范围自证（关键）**：`-A` 只关闭 `manual_range_contains` **一条** lint，其余仍 `-D warnings`。
本次新增代码中**不存在**该形态 —— 判据 ④ 只有三类**单侧**比较：
`market_cap > 0.0`、`fcf_yield < FCF_YIELD_INAPPLICABLE_MIN`、
`latest.net_profit.filter(|v| *v <= 0.0)`，**无 `A && B` 式手写区间**
⇒ 该豁免**不可能掩盖本次改动的任何问题**。

---

### 8.8 库身份确证 与「落库前置」状态（2026-09-22 00:2x 实测）

#### 8.8.1 我查的库 = 应用重种写的库（必须自证，否则整篇结论可能落在错的库上）

本条是**判据 R（判据工具自证）**级别的前置：若审计读的库与应用实际写的库不是同一个，
则 §五 层 1 的全部结论都不成立。逐项核对：

| 来源 | 证据 | 值 |
|---|---|---|
| 应用运行时配置 | `C:\Users\polit\.axagent\db_config.json`（`init/database.rs:62-71` 读 `{app_dir}/db_config.json`）| `db_type=postgres`、`pg_host=localhost`、`pg_port=5432`、**`pg_database=axinvest`**、`pg_user=postgres` |
| 审计所用连接 | `select current_database(), current_user, inet_server_addr(), inet_server_port()` | **`axinvest`** / `postgres` / `localhost` / `5432` |
| 交叉验证（更早实测）| 同一库内 `workflow_templates.version` 与运行产物 `blackboard_snapshot` 的 `template_version` **同值**（62 ↔ 62）| 同源 |

⇒ **同一个库，确证。** 且 SQLite 兜底路径（`init/database.rs:65`，`db_config.json` 不存在时才走）
**未生效** ⇒ 不存在「重种写 SQLite、我查 PG」的错配。

#### 8.8.2 落库前置**尚未发生**（修复代码就绪 ≠ 已生效）

| 探针 | 实测 | 判读 |
|---|---|---|
| `workflow_templates['stock-analysis'].version` | **73** | 常量已是 **75**，`73 >= 75` 为假 ⇒ **版本门开着**，重种**一定会执行** |
| `value_dcf_discount_rate` | **10** | 仍为校准前旧值 |
| `value_dcf_perpetual_rate` | **3** | 同上 |
| `value_dcf_growth_rate` | **8** | 同上 |

**原因已查明（非修复失败）**：进程表中那条
`cargo run --no-default-features --features notification,computer-use`
（PID 18792 / 7344，00:18:35 起）**仍堵在同一 `target` 目录的锁队列里、尚未真正启动**
⇒ 种子代码没被执行过。（旁证：21:58 那条同命令的 `cargo run` 同样未启动成功 ——
否则 DB 早该是 74。）

**落库后预期态**：`version = 75` 且三参数 = **8.5 / 4 / 12**。

**⇒ 修复优先级判定（本轮的关键决策）**：`~/.cargo/config.toml` 的 `jobs = 1` 使构建**严格串行**，
而这条 `cargo run` 正是触发重种、产出**端到端验证**的唯一命令
⇒ 已**主动终止**我自己排队的冗余门禁（§8.7.3），**把锁让给它**。排队中的复核让位于让修复生效。

### 8.9 ⚠️ 重种混淆项：v74（本次）与 v75（并发会话）**会一次性同时落库**

`TEMPLATE_VERSION` 现为 **75**，其注释块（`seed_stock_analysis.rs:220-230`）不是本次改动 ——
属**另一会话**为**同一个 688114** 做的修复。⇒ 下次重种会把**两批改动同时**写进 DB：

| 批次 | 内容 | 修复的 688114 症状 |
|---|---|---|
| **本次（v74 段）** | DCF 三参数强制覆写（8.5/4/12）+ DCF 适用性判据 ④ | 估值区间 `16.33–38.84` 偏低、DCF 对亏损公司强行出数 |
| **并发会话（v75）** | `portfolio-mgr.rhai` 的 `risk_pe` **三态分流**（`!present` 真缺失 / `== 0.0` 占位 / `< 0.0` 亏损） | `pe_ttm = −144.08` **有值却被报成「PE 数据缺失」** ⇒ 连带 `data_gaps` 维度打 0 分、UI 把观望说成「数据不足导致的被动降级，非看空判断」（**方向说反**） |

**⇒ 复跑 688114 时的归因纪律**：两批改动同时生效 ⇒ **输出变化不可单独归因于本次修复**。
要隔离，须**按字段比对**而非只看最终 action：

1. `t-valuation`（黑板顶层 string）里的三档区间 —— 只受**本次**影响（v75 不动它）；
2. `risk_pe` 的三态与 `data_gaps` 是否还含「PE数据(t-risk)」—— 只受 **v75** 影响；
3. `value-investor` 是否改填 `null` + 「无算法估值锚」—— 受**本次**影响。

**这也解释了用户「多次修复这错误」的观感**：同一个标的被两个独立会话从两个不同角度修，
单看最终结论会以为「改了几次都没用」，实际是**两条腿各自还没落库**。

#### 8.9.1 同族根因：本仓第 **4** 例确认（判据层升级依据）

v75 注释里的 `pe_ttm = −144.08` 被报成「缺失」，与本次层 2 的「净利 ≤ 0 短路」是
**同一形态**：**用 `<= 0.0` / `> 0.0` 过滤，把「负」这一有信息量的状态塌缩成「无」**。

⇒ 该家族在本仓的**已确认实例升至 4 个**（见 §九.2），且**其中 2 个是同一家公司同一周内
被两个独立会话各发现一次** ⇒ 这**不是偶发**，是**系统性写法缺陷**。
建议（登记，不擅改）：把「`filter(|v| *v > 0.0)` 形态」做成 lint/门禁扫描项，
而非继续靠人工在每次故障后逐个发现。

---

## 九、可复用判据

1. **「改默认值」对「存量配置」是假修复** —— 凡「代码常量 / 种子默认值 / DB 存量值」
   三处并存的配置项，改前两处**不会改变生产行为**。判据：拿 DB 现值与代码校准值比对，
   若恰好等于「校准前的历史值」⇒ 校准从未生效。
   检法：`select v->>'name', v->>'value' from workflow_templates t, jsonb_array_elements(t.variables::jsonb) v`。
   ⚠️ 只改默认值而忘记 `force_variable_value`，是**静默无效**。
2. **`filter(|v| *v > 0.0)` 把「负」这一有信息量的状态退化成「无」** —— 本仓已确认 **4 例**
   （① vendor 侧负 PE 被抹成 `None` ⇒ 报「缺数据」；② 本节判据 ② 净利为负短路；
   ③ `normalized_annual_profit` 的过滤，属有意设计；④ **并发会话 v75** 的
   `risk_pe` 把 `< 0.0` 与 `!present` 合流 ⇒ `pe_ttm = −144.08` 被报成「PE 数据缺失」，
   连带 `data_gaps` 维度打 0 分、UI 把观望说成「数据不足导致的被动降级」）。
   **其中 ② 与 ④ 是同一家公司（688114）同一周内被两个独立会话各发现一次**
   ⇒ 属**系统性写法缺陷**，不是偶发。
   检法：见到 `> 0.0` / `<= 0.0` 过滤，先问「被过滤掉的那一侧是不是独立状态、下游是否需要知道」。
   **建议（登记）**：把该形态做成 lint / 门禁扫描项，替代「每次故障后人工逐个发现」。
3. **判据的「形态清单」要按**被测对象的符号**穷举** —— 判据 ② 只覆盖「净利 > 0」，
   于是最该拦的「净利 ≤ 0」成了盲区。凡判据内出现符号前置条件，必须检查反面。
4. **修复上游会暴露下游缺口** —— 字段死链修好后，被它掩盖的「模型适用性」问题
   立刻显形（看多翻看空）。修复前后必须重跑同一标的并对比结论方向，
   不能只看「字段有值了」。
