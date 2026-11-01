# PLAN：DCF 增长率取数稳健化与主口径判据收口（K1→K4）

> 触发：用户 2026-10-02 报「恒瑞医药（600276）现价 47.20，算法 DCF 估值区间（保守档—乐观档）13.27–16.01 元 −71.9%」。
> 本轮是 DCF 的**第四族**缺陷。前三族（代理锚自我论证 `4e5e011eb` / applicable=false 面板泄漏 `f6eaa2a1f`+`b9aae12ed` / FCF 收益率地板 `valuation.rs` 2026-09-28 重构）修的都是「**锚是假的**」；
> 本例**锚是真的**（最近完整年报 FCF，`is_fallback_anchor=false`），坏的是**增长率**。既有闸口全部放行。

---

## 0. 触发实证（逐数值，全部可复现）

同一次运行（`stock_analyses.id=21cdd00e-f4a4-4780-aaac-4793516938b4`，2026-10-02 16:06）落库的 `dcf.assumptions`：

| 字段 | 实测值 | 含义 |
|---|---|---|
| `fcf_anchor` | 8,273,471,846 元 = **82.73 亿** | 最近完整年报 FCF（OCF−资本开支） |
| `is_fallback_anchor` | **false** | 锚不是代理 ⇒ 前三族的闸口都不适用 |
| `applicable` / `applicability_signals` | **true** / `[]` | 判据 ①~⑤ 零命中 |
| `fcf_per_share` | 1.296984848777471 元 | |
| `total_shares` | 6,379,004,237 股 | 市值 = 47.20 × 63.79 亿 = 3010.9 亿 |
| **`growth`** | **-0.01939002260778** | ← 病灶 |
| `low_growth` / `high_growth` | -2.7146% / -1.1634% | **三档全负，乐观档也假设萎缩** |
| `configured_perpetual_growth` → `perpetual_growth` | 1.3% → **0** | 被符号一致性约束压回（`perpetual_clamped_by_negative_growth=true`） |
| `discount_rate` / `forecast_years` | 7.7% / 5 | |
| `terminal_value_ratio` | 0.681 | |
| `low` / `mid` / `high` | **13.27 / 15.48 / 16.01** | |
| `upsidePct` / `midUpsidePct` | **-71.9%** / -67.2% | |
| 同次输出 `legs` | dcf bearish / reverse_dcf **Impossible** / relative **deep_value(bullish)** / graham -88.5% | 四腿自相矛盾 |
| `f5` 估值因子 | σ=**-0.434**、w=0.21 | 该 -71.9% 确实喂进贝叶斯决策 |

**算式复现**（`two_stage_dcf(1.296985, -0.01939, 0, 0.077, 5, 0.015)`）：预测期现值 4.9382 + 终值现值 10.5378 = **15.476**，终值占比 10.5378/15.476 = **0.681** ⇒ 与落库 `mid=15.48`、`tvr=0.6809667` 逐位吻合。**算术没有错。**

**`growth` 的来源实证**（东财 `ZYZBAjaxNew`，脚本 `.workbuddy/tmp/pg/`）：

| 报告期 | `TOTALOPERATEREVETZ` | 落库 growth |
|---|---|---|
| 2026-06-30（中报，`financials[0]`） | **-1.939002260778** | -0.01939002260778 ✓ 逐位相同 |
| 2026-03-31 | +12.9754 | |
| 2025-12-31（年报） | +13.0243 | |
| 2025-09-30 / 2025-06-30 / 2025-03-31 | +14.8533 / +15.8849 / +20.1429 | |
| 2024-12-31（年报） | +22.6331 | |

⇒ 近 9 期**唯一为负的就是最新这一期中报**，其余 8 期全在 +12.98%~+22.63%；负值来自 2025H1 一次性授权收入造成的极高基数（2026Q1 仍 +12.98%，Q2 单季 -14.5%），同期净利同比仍 +0.34%。
取数代码：`mcp_tools.rs:3464` `growth = latest.revenue_yoy / 100` —— 一个报告期的累计同比，**无平滑、无校验**。

---

## 1. 根因分层（四层各自成立，必须四层都修）

### L1 取数层 —— 单点增长率（本例直接原因）
`growth` 与反向 DCF 的 `revenue_yoy`（`mcp_tools.rs:1583`）**都直取 `financials.first()`**。一个坏数据点同时独裁三处：
1. 正向三档增速（-2.71%/-1.94%/-1.16%）⇒「保守档—乐观档」区间内没有任何一档假设增长；
2. `perpetual_clamped_by_negative_growth`（`mcp_tools.rs:3482`）把 `p` 1.3%→0 ⇒ 终值倍数 16.9×→13.0×，mid 再降 15%；
3. 反向 DCF 的期末营收按同一 -1.94% 复合推 ⇒ 隐含末年 FCF 260.61 亿 ÷ 萎缩后营收 > 100% ⇒ 判 `Impossible`。

### L2 模型边界层 —— 正向 DCF 的倍数天花板（增长率修好也不够）
本模型参数（d=7.7%、p≤1.7%、5 年预测期）下，**中性档倍数上限只有 15.0× 锚定 FCF**（g=0 时）。恒瑞市场倍数 = 47.20/1.29698 = **36.4×**。
把 `growth` 换成不同稳健口径重算（同一锚、同一 d/p）：

| growth 口径 | 三档 | upsidePct(保守) | mid/price | high/price |
|---|---|---|---|---|
| 现取单期 -1.94% | 13.27/15.48/16.01 | **-71.9%** | 0.328 | 0.34 |
| 近5年报截尾均值 +4.56% | 18.16/23.65/27.42 | -61.5% | 0.501 | 0.58 |
| 近5年报中位数 +7.26% | 19.45/26.52/32.48 | -58.8% | 0.562 | 0.69 |
| 最新年报 +13.02% | 22.45/33.64/45.91 | -52.4% | 0.713 | 0.97 |

⇒ **即使 K1 把增长率修对，正向 DCF 仍报 -52%~-62%。** 这是结构性的：**PE ≳ 18 的标的必然被本 DCF 报 -40%~-70%**（301269 -97.5%、603353 -94.0%、002371 -82.8%、600276 -71.9% 是同一条边界的样本，不是四个 bug）。
⇒ 所以 `FCF_YIELD_MIN_FOR_DCF_PRIMARY = 1%`（`valuation.rs:428`）这条地板**量纲错了**：它测「锚 ÷ 市值」，恒瑞 2.75% 轻松过关；而它想表达的「当期现金流对现价有无解释力」，实际由「DCF 倍数 ÷ 市场倍数」决定。

### L3 闸口层 —— 前三族的闸口天然覆盖不到本形态
判据 ①~⑤ 全不命中（负债率低、FCF/净利=1.11、净利为正、非代理锚、配置未越界）；前端只屏蔽 `dcfApplicable=false` 与 `anchorIsFallback=true`（`ValueAssessmentPanel.tsx:712,717`）⇒ 本例两者皆 false ⇒ 原样渲染。

### L4 呈现层 —— 标签与实际假设不符
面板写「算法 DCF 估值区间（**保守档—乐观档**）」（i18n key `stockAnalysis.valueAssessment.algorithmRangeBand`），而三档增速全 ≤0 ⇒ 该区间实为「持续衰退带」，自称乐观档属伪装。按既定纪律（结构性缺口不得在 UI 造成歧义），「拿不到」可接受，伪装成正常结果不可接受。

---

## 2. 影响面普查（近 5 日 V74 之后样本）

**口径与自证**（脚本 `.workbuddy/tmp/pg/dcf_sweep5.mjs`，取数 `.workbuddy/tmp/pg/sweep5.txt`）：
段定位由 `position('upsidePct')` 前向取 1700 字符 ⇒ 同一 `dcf` 对象内独立取 `current_price` 与 `low`/`upsidePct`；
**恒等式自证 `price ≟ low/(1+upsidePct/100)` 通过 10/10 条（偏差 <1.5%）**，其中 600276 与后端落库文案（现价 47.20、13.27/15.48/16.01、-71.9%、FCF 收益率 2.75%）逐字吻合。

⚠ **样本量声明（不得当作全量结论）**：`dcf_valuation` 落在 `blackboard_snapshot` 的近 30 天记录 51 行，但 2026-09-28 V74 改版**之前**的存档字段口径不一致（`applicable` 未落、旧版把不可用编码成 `low=0`、`upsidePct` 键不在 `dcf_valuation` 内），强行混算会得到 12/12 现价互斥的假结论（已在 v1 实测复现并废弃）。收口到 V74 之后只剩 **17 行 / 10 个标的**，其中 `applicable=true ∧ 真实锚` **只有 4 只**。故下表是「小样本 + 阈值余量」论证，不是分布估计。

**K2 候选判据命中面**（n=10，`appl=true ∧ 非代理锚` 4 只：600276、301269、600887、600406）：

| 标的 | fcf_yield | high/price | market_mult ÷ high_mult | 现判据 δ(<1%) | 候选 α(<1) |
|---|---|---|---|---|---|
| 600406 国电南瑞 | 5.91% | **2.28** | 0.4 | 留 | 留 |
| 600887 伊利 | 6.58% | **1.35** | 0.7 | 留 | 留 |
| 600276 恒瑞 | 2.75% | **0.34** | 2.9 | **留（漏！）** | 切 |
| 301269 华大九天 | 0.14% | 0.05 | 19.8 | 切 | 切 |

⇒ α（`high/price < 1`）相对 δ 的**净增量只有 600276 一只，附带伤害 0**；健康样本与病样本之间阈值 1.0 落在空档正中（1.35 ↔ 0.34，间隔 4 倍）⇒ 阈值不敏感。
⇒ 不违反「必须给出结论、严禁一路加判据把标的全排除」：α 命中后**让位反向 DCF**（永远有答案的出口），不是判「无结论」。

**K3 命中面**：三档增速全 ≤0 ⇒ 窗口内 1/10（600276）。但 603353（近 5 年报 4/5 年为负，截尾均值仍 -6.90%）实证：**K1 修好后「整带负增长」仍会真实发生** ⇒ K3 不是 K1 的附属，必须独立做。

**K4 命中面**：DCF 看空 ∧ 相对估值看多 ⇒ 窗口内 1/10（600276：DCF -71.9% vs PE 自身历史分位 **7%** ⇒ `deep_value` vs 一致目标价 59.24 元 +25.5%）。低频但危害大（用户看到的就是这个矛盾）。

**K1 统计量选型 —— 中位数与算术均值都被实测否决**：

| 标的 | 单期(现取) | 近 5 年报营收同比 | 五年**累计** | 中位数 | 算术截尾 | **对数截尾（采用）** |
|---|---|---|---|---|---|---|
| 600276 恒瑞 | -1.94% | 13.02, 22.63, 7.26, -17.87, -6.59 | +12% | +7.26% | +4.56% | **+4.23%** |
| 603466 金域 | -0.92% | 20.72, -41.44, 39.75, -42.79, 30.30 | **-26%** | +20.72% ✗ | **+3.19% ✗ 方向错** | **-2.70%** ✓ |
| 603353 和顺 | -6.05% | -8.15, -14.11, -18.04, 1.55, 113.35 | -8% | -8.15% | -6.90% | **-7.13%** ✓ |
| 601166 兴业 | -0.25% | 0.24, 0.66, -5.19, 0.51, 8.91 | +5% | +0.51% | +0.47% | **+0.47%** ✓ |

选型结论（两步，不是一步）：

1. **中位数不行**：它只保证「位置居中」，与累计增长脱钩 —— 金域五年营收**净缩 26%**，中位数却给 +20.72%。
2. **算术截尾均值也不行，而且错得更根本**：它给金域 **+3.19%（方向都反了）**。`growth` 在 `two_stage_dcf` 里是**复利乘数**（`fcf × (1+g)^year`），其无偏估计必须是**几何**平均；算术均值恒 ≥ 几何均值（Jensen 不等式，波动越大偏差越大）⇒ 用算术口径会把萎缩公司写成增长公司，属**方向性错误**而非精度问题。

⇒ 采用**对数收益率截尾均值**（去极值版的几何平均）。真萎缩的和顺仍给 -7.13%（**不会被洗白**），兴业 +0.47%（平台期），恒瑞 +4.23%。

> 本节是实施中改的：PLAN 初稿写的是「算术截尾均值」，金域那条序列把它纠正过来了。


## 3. 方案

### K1 增长率取数稳健化

**改点**：`src-tauri/crates/astock-data/src/mcp_tools.rs`

1. `compute_dcf` 的 `growth` 改为**近 5 个年报期营收同比的对数收益率截尾均值**
   （升序 → 去最高/最低各一个 → 取均值 → `exp_m1`）。样本 < 3 时退回全样本几何均值；
   年报期一条都没有有效同比时退回最新单期；单期也缺时退回 `default_growth`。
   **每条分支都过 `clamp(min_growth, max_growth)`** —— P0-F 方案 A 的档位单调不变量不能破。
2. 取数封成纯函数 `robust_annual_growth`，与 `latest_annual_fcf` / `ttm_net_profit` 并列
   （可单测、可复用，禁区 12）。
3. **反向 DCF 的同源收口有两处，不是一处**：
   - `revenue_yoy`：`financials[0].revenue_yoy`（单期）→ `a.growth`。
     否则同一份输出里正向按 +4.23% 折现、反向按 -1.94% 推期末营收，两个口径互相打架，
     而反向的可行性判据会被同一个坏数据点污染。
   - `revenue_0`：`financials[0].revenue`（**中报 = 半年营收**）→ 年报营收
     （新增 `latest_annual_revenue`，与年报 FCF 锚取**同一条报告期**）。
     恒瑞实证：分母用半年营收 154.56 亿时，隐含末年 FCF 260.61 亿 ÷ 复合后的期末营收
     得 >100% ⇒ 判 `Impossible`（一句**假的**「物理不可能」）；换年报 316.29 亿后落到
     Strained 量级（紧张但可能）—— 这才是有信息量的结论。
4. `DcfAssumptions` 新增**三个**披露字段（都**不进** `applicability_signals` ——
   那里的 `is_empty()` 直接决定 `applicable`，塞进去会把数值可用的腿整条误杀）：
   - `growth_basis: String` —— 取数口径自述（含回落原因）；
   - `growth_is_outlier: bool` —— 单期与稳健值背离 >10pp，**或**跨零且至少一侧脱离 1pp 近零带；
   - `growth_band_all_negative: bool` —— `high_growth ≤ 0`，K3 消费。
5. **不新增用户可配参数**。判据 ⑤ 的教训写明：`pct()` 守卫下的扁平参数能把 `p` 配进
   发散区 ⇒ 新参数若无独立含义就不要开这个口子。

**测试**（追加到 `mcp_tools.rs` 的 `valuation_tests` mod 末尾 —— 不是文件末尾那个
`consensus_gap_tests`，那里没有 `compute_dcf` 与 `report()` helper；本人第一次就加错并编译失败）：
恒瑞形态（对数口径 0.042287 手算锁定，容差 1e-3 且报错文案点明「若得 0.0456=算术、
0.0726=中位数、-0.0194=未改」）、和顺形态（真萎缩仍为负 + `growth_is_outlier=false`）、
金域形态（反向锁：不得等于中位数 20.72% 也不得等于算术截尾 3.19%）、三条回落分支各自自述 basis。


### K2 主口径判据改倍数
**改点**：`src-tauri/crates/astock-data/src/valuation.rs`
1. 新增常量 `DCF_PRIMARY_MIN_HIGH_COVERAGE: f64 = 1.0`，文档写明弹性来源（本模型倍数上限 1/(d−p)≈15.7×，市场倍数中位 59× ⇒ 覆盖率 <1 时正向折现对现价无解释力）。
2. `ConclusionInputs` 增 `dcf_high_to_price: Option<f64>`（调用方在 `mcp_tools.rs:1604` 传 `high/current_price`）；
3. `dcf_ok = applicable ∧ upside 存在 ∧ (fcf_yield ≥ 1% ∨ high/price ≥ 1.0)` ⇒ 两条地板**取并集**（保留 δ 已取得的收益，不回收）；
4. `not_applicable` 文案按**实际失败的那条**分别措辞，不得笼统写"锚不成立"。

**测试**：600276 真实数值（fcf_yield=0.0275、high=16.01、price=47.2）⇒ 修复后 `primary_method="reverse_dcf"`；600887（h/p=1.35）、600406（h/p=2.28）⇒ 仍 `"dcf"`（锁"不扩大打击面"）；301269 保持既有 `reverse_dcf` 行为不变。

### K3 全带负增长的呈现纠偏
1. `DcfAssumptions` 增 `growth_band_all_negative: bool`（`high_growth ≤ 0`）。
2. 透出链：`seed_stock_analysis.rs` 的 `input_mapping`（参照 L4583/L4596 两条 `valuation_dcf_*`）→ `rhai_registry.rs` 默认常量（参照 L878-879）→ `portfolio-mgr.rhai` 的 `valuationApplicability`（L3256-3269）。
   ⚠ **必 bump `TEMPLATE_VERSION`**（改种子不 bump ⇒ 存量库不重建，实测过多次）。
3. 前端 `ValueAssessmentPanel.tsx`：命中时在区间**之前**标注，且把 `algorithmRangeBand` 标签换成负增长专用标签（不解析 LLM 文案，走结构化布尔 —— 与 I1 闸口同纪律）。
4. i18n：**11 种语言全补**（`src/i18n/locales/*.json`，Edit 时保序 + 强制 LF），过 `check-i18n-key-parity.mjs`。
5. 测试补进 `ValueAssessmentPanel.render.test.tsx`（正/反两向：命中必现、未命中不得现）。

### K4 两腿矛盾并列披露

`build_conclusion`（`valuation.rs`）新增 `ValuationConclusion.conflicts`，并在**所有**主口径分支
把冲突句并入 `headline` 末尾（`headline` 才是下游 Agent 与面板实际消费的那一句，
单独放字段里等于没说）。

⚠ **覆盖面在实施中被实测纠正过一次**：冲突判据最初只认 `dcf` 腿 × `relative` 腿。
但 K2 生效后，高倍数标的的 DCF 腿会被摘掉 —— 恒瑞修完 K1+K2 后 `primaryMethod=reverse_dcf`、
`dcf` 不在 `legs` 里 ⇒ 冲突分支**反而不再触发**，「PE 处自身四年 7% 分位」与
「现价隐含需 21.6% 年复合」又只剩一句，**等于没修**。
⇒ 判据改为取**现金流腿** = `dcf` **或** `reverse_dcf`（stance ≠ neutral）。
这条由测试 `conflict_still_fires_when_dcf_leg_is_dropped_by_coverage_gate` 锁住
（它先断言 `dcf` 腿确实被摘，再断言冲突仍报 —— 否则测试会退化成空跑）。

文案**不仲裁谁对**：只并排两条证据，并点明两者口径不可通约 ——
现金流折现腿答「绝对定价需要多激进的假设」，相对估值答「与自身历史/同行比在哪个分位」，
**相对自己便宜并不等于绝对定价不需要增长**，两者可以同时为真。

**不做**的事：不做「矛盾即降级」，不因此把该腿置 `applicable=false`
（那会把决策证据整条丢掉，且谁对没有任何判据）。


### K5 荐股链（趋势智选）同源收口（2026-10-02 经用户批准并入本轮）

**语义裁定（自裁，依据在被改代码里）**：`serenity.rs` 的 `rev_growth` 同时充当
① 排除门槛（`< serenity_min_revenue_growth`=5% 出局）与 ② **高 PE 的豁免依据**
（`:101` 增速 ≥ `serenity_growth_exempt_pct`=50% 就跳过「估值过高」排除）。
第 ② 条决定它问的是**可持续增长**——用单期尖峰去豁免高 PE，等于在标的最贵的时候给它最宽的放行。
⇒ 复用 `robust_annual_growth`（与正向 DCF 同一份实现，不写第二遍，禁区 12）。

**改点**（`crates/analysis-engine/src/recommender/strategies/serenity.rs`）：
1. `latest.revenue_yoy.unwrap_or(0.0)` → `robust_annual_growth(&financials) × 100`，
   年报样本缺失时回落单期；
2. **缺数与零增长分列**：原 `unwrap_or(0.0)` 让「没取到增速」与「增长为零」同形，
   缺数标的被「增速过低」误排且给出错理由。现缺数**仍排除**（筛选强度不变），
   但日志理由改为「取不到增速」；
3. 排除门与 PE 豁免门的日志都带上 `growth_basis`（豁免依据必须可追溯）。

**双向后果**（这是本轮唯一一处**同时放宽与收紧**选池的改动）：
高基数导致单期下滑的（恒瑞这类）被救回；周期顶点单期暴涨撑起高 PE 的被收回豁免 ⇒ 候选池可能变小。

**影响面：未实测。** 原拟从 `reco_picks` 拉当前 serenity 候选池逐只比对口径翻转，
两次查询均报 `could not read blocks 0..0 in file "base/53691/55944": Invalid argument`
（同库另一会话正在写入）。⇒ 名单待补；若实测后认为收紧过头，可只保留「缺数分列」而回退口径切换。

## 4. 推进顺序与验收

| 阶段 | 内容 | 定点验证 |
|---|---|---|
| ① | K1（取数 + 反向同源 + 2 字段 + 4 组测试） | `cargo check` + `cargo clippy -p axagent-astock-data --all-targets -- -D warnings` + `cargo test -p ... valuation` |
| ② | K2（倍数判据 + 3 组测试） | 同① |
| ③ | K4（headline 并列 + conflicts） | 同① |
| ④ | K3（Rust→种子→Rhai→前端→11 语言） | 后端同①；前端 `npm run typecheck` + `npm run test:run` + `npm run lint:eslint`(=oxlint) + `npm run format` + `check:i18n` |
| ⑤ | 全量门禁（一次，CI 口径逐字） | `cargo fmt --all --check`；`cargo clippy --workspace --all-targets --all-features -- -D warnings`；`__TAURI_WORKSPACE__=true cargo test --workspace` |
| ⑥ | 实机核对 | 重启 `tauri dev` 重跑 600276：期望 `primaryMethod=reverse_dcf`、growth>0、区间标签带负增长声明、决策卡不再以 -71.9% 定调 |

**①阶段后的中间态（须向用户如实报告，不得称"已修好"）**：K1 单独生效后恒瑞 upsidePct 从 -71.9% 变为约 -61.5%（仍负）—— 这是 L2 结构边界，不是回归。

---

## 5. 附带发现（不在本计划范围，仅登记）

1. **600000 浦发** 落库 `upsidePct = +1434.3%`、`mid/price = 19.6` —— 三档数值超出量程的**正向**侧未见封顶（既有遮蔽只处理了部分路径）。该样本 `applicable=false` 已被拦，但数值仍落库并流入 `dcf.mid` 存档。
2. ~~反向 DCF 的 `revenue_0` 取 `financials.first().revenue`（中报=半年营收）与 `forecast_years` 复合口径不匹配~~ **【已修，并入 K1 第 3 点】** 原例：分母用 2026H1 营收 154.56 亿、分子用年报 FCF 锚反解出的隐含末年 FCF 260.61 亿 ⇒ 占比 >100% ⇒ 输出「自由现金流超过全部营收，物理不可能」这句**假诊断**。现与年报锚取同一报告期的营收（316.29 亿），占比落 Strained 带（紧张但可能）。
3. `financial_snapshots` 表的 `revenue_yoy` / `profit_yoy` / `roe` / `debt_ratio` 列对 600276 近 8 日**全为空**（仅 pe/pb/ps 有值）⇒ 若后续要用该表做取数旁证，先补列写入。

---

4. **【新发现，未在本计划范围，待裁定】荐股链（趋势智选 `serenity`）有第二套单期增速取数，且后果更硬。**
   `src-tauri/crates/analysis-engine/src/recommender/strategies/serenity.rs:81`
   `let rev_growth = latest.revenue_yoy.unwrap_or(0.0);` —— 同样取**最新单期**同比，但它驱动的是**排除门**：
   - `:80-84` 营收增速 < `serenity_min_revenue_growth`(默认 5%) 且 `serenity_score < 85` ⇒ **整只股票被踢出候选池**；
   - `:99-103` PE 超上限且增速 < `serenity_growth_exempt_pct`(50%) ⇒ 再排一次；
   - `:248` `conf_growth = rev_growth / 50` ⇒ 置信度按同一个数线性缩放。

   ⇒ 恒瑞这一期（2026H1 = **-1.94%**）在荐股链上会直接落进「增速过低排除」，
   比分析链「估值区间偏低」的后果更硬（不是数值难看，是**根本不进池**）。
   另有一处独立缺陷：`unwrap_or(0.0)` 把「**没取到数**」当成「零增长」⇒ 缺数标的会被增速门误排，
   违反「结构性缺口不得在 UI/决策造成歧义」（缺数应显式缺席，不是填 0）。

   与 K1 的关系：**同一取数病灶，不同链**（本计划只覆盖分析决策链）。
   若要把 K1 的 `robust_annual_growth` 复用到荐股链，需先决定两件事：
   ① 荐股链的增速语义是「当期动量」还是「可持续增长」——若是前者，稳健多年均值反而会抹掉它要的信号；
   ② 缺数态必须从 `unwrap_or(0.0)` 改成显式缺席（否则任何口径切换都还在误排）。
   ⇒ **不在本轮擅自扩范围**，等裁定。

---

## 6. 实施状态（2026-10-02 收尾）

| 阶段 | 代码 | 定点门禁 | 全量门禁 | 实机核对 |
|---|---|---|---|---|
| K1 增长率同源取数 | 已落地（在 `51ddfec17`） | ✔ | ✔ | 未跑 |
| K2 双地板主口径 | 已落地（在 `51ddfec17`） | ✔ | ✔ | 未跑 |
| K3 全带负增长披露 | 已落地（在 `51ddfec17`，含 v122） | ✔ | ✔ | 未跑（**必须重启才播种**） |
| K4 冲突并列披露 | 初版在 `51ddfec17`；**覆盖面纠正在工作树** | ✔ | ✔ | 未跑 |
| K5 荐股链同源收口 | 工作树，**未提交** | ✔ | ✔ | 未跑 |

### 门禁实测读数（定稿后复跑，不是中途读数）

- `cargo test -p axagent-astock-data -p axagent-analysis-engine --lib`：**1089 passed / 0 failed**（499 + 590），含本轮新增 9 条
- `cargo clippy -p axagent-astock-data -p axagent-analysis-engine --all-targets -- -D warnings`：**EXIT = 0，0 error**
- `cargo fmt --all --check`：**EXIT = 0**。⚠ 更正：本轮早期我报过「剩 2 处红属他人未提交」，那两处（`mcp_tools.rs:1500` 的 `oe_yield`、`:6854` 的 `use super::{}` 顺序）已随 `51ddfec17`/`b07c1bbf1` 消掉 ⇒ 当时的归属判断只对当时那份快照成立
- 前端：`typecheck` 无错、`vitest` 全量 **2119 条**（唯一红是 `DemandDiscoveryPage` 的 `Test timed out`，隔离重跑 1 passed，当时 2 个 `cargo.exe` 在编译）、`oxlint` 0 警告、`check-i18n-key-parity` 11 文件键集一致、`audit-inject-coverage.mjs` ①② 差集为空

### 主 crate `-p axagent --lib`：518 passed / 2 failed，两条红均已逐一取证，**都不属本轮**

1. `seed_consistency_tests::data_quality_rhai_diag_for_arity_matches_all_call_sites` ——
   全量里红、**隔离重跑 ok** ⇒ 不可复现的读数不进结论。取证同时发现 `seed_consistency_tests.rs`
   正被另一条会话改写（该文件凭空出现在 `git diff --name-only` 里，我从未编辑过它）。
2. `stock_workflow::serenity::extract_recovers_candidates_from_strict_mode_report` ——
   隔离重跑**仍 FAILED**（稳定红，非 flaky）；`git diff HEAD -- <该文件>` **无差异**，
   其最后一次提交正是 `51ddfec17` ⇒ 属 **HEAD 既存红**（随那条会话进仓，与被测代码同提交），
   与本仓已登记的「HEAD 上带既存红会短路 CI 口径」同型。**未代修**——要不要顺手补由你定。
   ⚠ 它与 K5 改的是**两个不同文件**（主 crate `commands/stock_workflow/serenity.rs` 是节点输出解析，
   我改的是 `crates/analysis-engine/.../strategies/serenity.rs` 策略取数），且该测试用固定信封文本、
   不跑策略 ⇒ 无因果路径。

### 提交事件与未提交面

K1~K4 已被另一条会话扫进 `51ddfec17`「置信度口径统一与数据质量缺席分列（模板 v118-v122）」
（`mcp_tools.rs +462`、`valuation.rs +295`、`seed_stock_analysis.rs +96` 含我写的 v122 登记、
`portfolio-mgr.rhai +44`、11 个 locale），**提交信息未提 DCF 增长率**。未为拆归属改写历史。
工作树当前未提交：K4 覆盖面纠正 + `robust_annual_growth` pub 化 + K5（`valuation.rs` /
`mcp_tools.rs` / `analysis-engine .../serenity.rs`），另有一个他人 WIP 文件混在同树。

### 仍缺的两件事

1. **实机核对**：重启 `tauri dev`（v122 才会重播种）重跑 600276。对账基准见
   `.workbuddy/tmp/pg/expected_600276.txt` —— 该基准的复刻实现已用**修复前落库值双向自证**
   （正向复现 `13.27/15.48/16.01` + `tvr=0.680967`；反向复现 `隐含 26% / 260.61 亿 / Impossible`），
   不是拟合出来的预期。
2. **K5 影响面名单**：`reco_picks` 两次读均报 `could not read blocks 0..0 … Invalid argument`
   （同库另一会话在写）⇒ 只有方向、没有逐只清单。若要补，等库空闲时按候选池逐只比对口径翻转。

### 更正记录（保留原文，不涂改）

- §2/§3 的统计量：初稿「算术截尾均值」→ 实施中更正为「**对数**收益率截尾均值」（金域反例，见 §2）
- §3 K4 初判据「`dcf × relative`」→ 实施中更正为「`(dcf | reverse_dcf) × relative`」（K2 摘腿致分支空转）
- 本节早期「fmt 剩 2 处红属他人未提交」→ 已随两条提交消掉，现值 EXIT=0

