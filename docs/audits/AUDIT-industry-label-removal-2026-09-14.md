# AUDIT — 判据去行业标签化（601166 争议的第二轮）

- 日期：2026-09-14
- 触发：用户反驳「你将股票按照行业归属来区分分析的思路是错误的，如果每个行业都不同那作为普通股民怎么可能处理」
- 前置：`AUDIT-601166-valuation-decision-2026-09-14.md`（估值 40.17–49.26 vs 观望 0% 的自相矛盾）

---

## 0. 一句话结论

**判据只能锚定「可观测的数据形态」，不能锚定「行业归属」。**

- 数据形态判据观测的是**事实**（这家公司真在亏损吗？真在萎缩吗？它的自由现金流口径成立吗？）
  —— 事实与行业分类无关，因此**天然覆盖全部行业**，不需要枚举任何行业名。
- 行业标签判据观测的是**归类**（它是「金融」所以……）
  —— 一旦行业口径变化（门类级 / 申万一级 / 自定义分类法）判据就整体失效；而且
  「某行业不可能出事」这类断言本身无法成立。

这也直接回答「其它股票怎么办」：**不需要"怎么办"** —— 数据形态判据不是「把银行的特殊规则
抄一遍给保险」，而是从根上不引入行业维度。

---

## 1. 三处按行业下判据的实证

| # | 位置 | 形态 | 性质 | 处置 |
|---|---|---|---|---|
| 1 | `mcp_tools.rs` DCF 适用性判据 | 已按数据形态（3 判据） | ✅ 正确形态 | 保留（本轮已完成） |
| 2 | `portfolio-mgr.rhai` 极高风险「金融业白名单豁免」 | 8 个行业关键词 → 布尔 | ❌ 按行业归属，且**会放掉真困境** | **本报告删除** |
| 3 | `portfolio_monitor.rs::sector_beta` | 行业关键词 → beta 数值 | ⚠️ 行业→数值代理，且**口径落空** | 登记，待裁决（§4） |

### 1.1 DCF 判据（#1，已正确）

三条判据全部只看数据形态，无行业名：

```rust
const LEVERAGE_INAPPLICABLE_PCT: f64 = 80.0;   // ① 杠杆畸高：净利由杠杆驱动
const FCF_NP_DIVERGENCE_MIN: f64 = 0.3;        // ② FCF 与净利背离（含符号相反）
const TERMINAL_RATIO_MAX: f64 = 0.7;           // ③ 负增长且终值独裁
```

覆盖性：银行 / 保险 / 券商 / 地产 / 重资产周期**自动全覆盖**（它们都会命中 ①②③ 之一），
无需写任何 `if 行业 ==`。

### 1.2 金融业白名单（#2，已删除）

```rhai
// 删除前
let financial_keywords = ["金融","银行","保险","证券","信托","多元金融","非银金融","金融服务"];
let sector_is_financial = /* contains 任一 */;
if !sector_is_financial && debt > 85.0 && growth < 0.0 && (roe < 0.0 || growth < -10.0) {
```

两个硬伤（**这才是删除理由，不是"银行股需要特殊照顾"**）：

1. **它宣称「金融企业不可能陷入财务困境」**。一家 `ROE<0` 且在收缩的银行，恰恰是最该顶格的
   标的 —— 白名单会把这种**真困境**一起放掉。这是**漏报**，方向比误报更危险。
2. **它已经失效过一次**：实测 `stock_sector` 取值域是**门类级** —— 601166 的行业值是
   `"金融"` 而**不是** `"银行"`。只列「银行/保险/证券」时全部落空（上一轮真实踩过）。

**替代者**：条件 C（`roe < 0.0 || growth < -10.0`）单独承担。它观测的是事实，与行业名无关。

**对 601166 决策中性**（可正算复核）：`growth=-0.3` 不满足 `<-10`、`roe=4.8` 不满足 `<0`
⇒ C 本身为假 ⇒ 白名单删与不删都得「中风险 / 持有」。

---

## 2. 用户三问的逐条回答（含实证数字）

`RISK_*` 默认值：`VOL_EXTREME=60 / SHARPE_EXTREME=-1.5 / VOL_HIGH=40 / DD_HIGH=45 /
ROE_HIGH=5.0 / DEBT_HIGH=65 / DEBT_EXTREME=85 / VOL_LOW=25 / SHARPE_LOW=0.5 / DD_LOW=30 /
ROE_LOW=8.0 / DEBT_LOW=55 / GROWTH_LOW=3.0`

601166 实测画像（DB `blackboard_snapshot["t-risk"]`，run `3e01fdc5`）：
`vol=21.1 / sharpe=0.399 / dd=8.7 / roe=4.8(半年未年化) / debt=91.6 / growth=-0.3`

### ① 「银行改用 PB-ROE」 → 已做，但做法不是「银行用 PB-ROE」

没有写 `if 银行 then PB-ROE`。做的是**让 DCF 自己承认不适用**（三判据），
然后由 **graham 腿独立承担**估值维度（格雷厄姆公式基于 EPS，对银行不失效）。
逻辑在 `portfolio-mgr.rhai` V77：`dcf_leg_usable` / `graham_leg_usable` 各自独立，
两条腿都不可用时 `f5_weight = 0`（「无法判断」既不加分也不扣分）。

### ② 「强制 growth < 0 时永续增长不得为正」 → 已做

`mcp_tools.rs`：`if growth < 0.0 { perpetual_growth = perpetual_growth.min(0.0) }`，
原配置值存入 `configured_perpetual_growth` 供对账，
`perpetual_clamped_by_negative_growth` 标记。原状态是 `growth=-0.25%` 与
`perpetual_growth=+3%` 并存 —— 模型自相矛盾。

### ③ 「银行资产负债率 91.6% 必然触发高风险必要条件」 → **实证不触发，不需要改**

用户说对了一半，这里给出精确数字：

- **高风险第一条**（`portfolio-mgr.rhai:433`）是 **AND** 结构：
  `(vol>40 || sharpe<0 || dd>45) && (roe<5.0 || debt>65)`
- 右半边：`debt=91.6 > 65` ⇒ **真**（用户说的「必要条件」在这里成立）
- 左半边：`vol=21.1>40` ✗、`sharpe=0.399<0` ✗、`dd=8.7>45` ✗ ⇒ **全假**
- ⇒ AND 整体**假** ⇒ 不判高风险
- 第二条高风险：`vol>35` ✗ ⇒ 假
- 低风险：`sharpe=0.399>0.5` ✗ ⇒ 假
- ⇒ **中风险**（与 DB 存档一致）

结论：**「负债率 91.6% 单独不足以构成高风险」是 AND 结构的正确行为** ——
高杠杆只有在**同时**出现价格层面的恶化证据（高波动 / 负夏普 / 大回撤）时才升级为高风险。
这本身就是数据形态判据在起作用，不需要为了银行再开一条规则。

### ④ ROE 年化（用户未提，但同属「口径」共性问题，本轮一并修）

`roe=4.8` 是**中报半年未年化值**，却参与 `roe < 5.0` 判定；而同一份数据的 PE 用的是
**年化** EPS —— 同一次分析内部两套口径。修法：`annualized_roe()`（与既有
`annualized_eps()` 同构：上年年报 + 本期累计 − 上年同期；缺同期按月份粗年化）。

链路（已核verify）：`annualized_roe()` → `stockRiskProfile.roeTTMPct`
（`mcp_tools.rs:1659`）→ `input_mapping` 的 `risk_roe` → `portfolio-mgr.rhai` 的风险判据。

601166 年化后 ≈9.4–9.8%（正常区间下沿）。**注意：这不会改变 601166 的风险档**
（见 §2③，左半边独立决定），但对「半年 ROE 恰在 5.0 / 8.0 阈值两侧」的其它股票会改变判定
—— 这才是共性问题。

---

## 3. 本轮改动清单

| 文件 | 位置 | 改动 |
|---|---|---|
| `src-tauri/src/commands/portfolio-mgr.rhai` | 404–432 | 删 `financial_keywords` + `sector_is_financial`；极高风险条件改为纯数据形态 |
| `src-tauri/crates/analysis-engine/src/portfolio_formula.rs` | 230–256 / 257–264 / 273–274 / 1299–1324 | 删 `is_financial_sector`；`classify_risk` 去掉 `sector` 形参；删 2 个白名单单测；重写 601166 单测文档为数据形态复算 |
| `src-tauri/src/commands/stock_workflow/rhai_pm.rs` | 40–51 / 58–66 | 死映射 `pm_classify_risk` 同步去掉第 7 参 `sector` |
| `src-tauri/src/commands/stock_analysis_setup/seed_stock_analysis.rs` | 474–480 / 605–611 / 623–625 / 3220–3224 | 注释更正；**删 portfolio-mgr 的 `("stock_sector","stock_sector")` 悬空映射**；`TEMPLATE_VERSION` **50 → 51** |

### 3.1 为什么必须升 `TEMPLATE_VERSION`（含一处自述更正）

⚠️ v50 的注释里我写过「`portfolio-mgr.rhai` 是编译产物、不需升版」—— **这是错的**。

实证：`seed_stock_analysis.rs:3079`

```rust
let pm_code = include_str!("../portfolio-mgr.rhai").to_string();
```

该 rhai 被写进 DB 模板 `portfolio-mgr` **CodeNode 的 `code` 字段**，是模板内容的一部分。
⇒ 改 rhai 公式**必须**升 `TEMPLATE_VERSION`，否则版本门（`existing.version >= TEMPLATE_VERSION`
→ 直接跳过种子）会让 DB 里仍是**旧公式**。「改完不生效」的生成物陷阱。

本轮 50 → 51 同时覆盖：rhai 公式变更 + `input_mapping` 删项。

### 3.2 悬空映射（顺手清理，同属「声明 ≠ 消费」）

`stock_sector` 在 `portfolio-mgr.rhai` 中的**唯一**消费者就是被删的白名单
⇒ `("stock_sector", "stock_sector")` 映射悬空，已删。

⚠️ **工作流变量 `stock_sector` 本身保留**：它仍被 `portfolio-risk-gate.rhai`
的**行业暴露上限检查**消费（`portfolio-risk-gate.rhai:121` → `pm_portfolio_risk_gate`
→ `portfolio_formula::portfolio_risk_gate` 的 `sector_exposures` 比对）。
那是「这一笔交易会让组合的行业集中度超限吗」—— **组合风险度量**，
与「给个股贴行业标签决定它的判据」是**两件不同的事**，不做删除。

---

## 4. 登记待裁决（未改）

### 4.1 `sector_beta`：行业关键词 → beta 数值（第二个同型实例）

`crates/analysis-engine/src/portfolio_monitor.rs:311`

```rust
fn sector_beta(sector: &str) -> f64 {
    // 科技/tech/it/互联网 → 1.3 ; 消费/consumer/食品 → 0.7 ; 银行/金融 → 0.5 ; ...
    // 医药 0.9 / 能源 1.1 / 地产 1.2 / 公用 0.4 / else 1.0
}
```

- **活代码**：`run_all_scenarios` → `run_stress_scenario:270` ←
  `stock_analysis.rs:3220` `dashboard.stress_test`；且 `run_portfolio_stress_test`
  是已注册 Tauri 命令。
- **两点问题**：
  1. 行业名 → 数值，形态上同属「按行业归属」；不过它是**建模代理**（压力测试的 beta 估计），
     不是「豁免/判据」，性质比 §1.2 轻。
  2. **口径落空**：实测 `stock_sector` 取值域（`电子设备 / 电气设备 / 信息技术 / 化石能源 /
     机械设备 / 休闲、生活及专业服务 / 交运设备 / 金融 / 建材`）与该表的键**大面积不匹配** ——
     「电子设备」「电气设备」「信息技术」「交运设备」「建材」全部落到 `else → 1.0`。
     即：**这张表看起来在区分行业，实际基本没生效**（静默退化为中性 1.0）。
- **候选方向**（择一，需产品语义决策，故未擅动）：
  - a) 保持数值不变，只把「桶是否命中」显式暴露（`beta_source: "sector_table" | "default"`），
    让静默退化变成可见；
  - b) 数据形态化：用真实历史 beta（crate 内已有 `compute_beta`，见 `portfolio_monitor.rs:143`），
    缺失时才回退中性 1.0；
  - c) 若坚持保留行业表，**至少补齐取值域** —— 但这是「枚举行业名」，与用户原则相冲突，
    不推荐。

### 4.2 其它已核查、判定**不属于**「按行业下判据」的用法

| 位置 | 用法 | 判定 |
|---|---|---|
| `portfolio_formula.rs::portfolio_risk_gate` | `sector_exposures` 行业暴露上限 | ✅ 组合集中度度量 |
| `scoring.rs::apply_industry_adjustment` | 个股 PE/PB vs **行业中位数** | ✅ 相对估值（PE 本就只能同业比） |
| `data-verifier.rhai:263–277` | PE vs 行业均值 ±2σ | ✅ 相对估值 |
| `concept_index.rs:184–190` | 「银行/保险」→ `industry_bank` 概念 id | ✅ 词汇表 / 同义词表，非判据 |
| `bottleneck-calc.rhai` / `strategy-scorer.rhai` | `industry_ranking` 匹配 | ✅ 数据驱动匹配，非硬编码白名单 |
| `screenshot_diagnosis.rs:210+` | `DEFENSE_KEYWORDS` 名称关键词 | ⚠️ OCR 只能给名称，属接口限制；非行业判据 |

---

## 5. 验证清单（用户重跑 601166 时应看到）

前置：app 重建到 v51（`TEMPLATE_VERSION=51` 触发重新种子）。

| 检查点 | 期望 |
|---|---|
| 模板版本 | DB `workflow_templates.version = 51` |
| 悬空边 | 0（v49 已修 `bear-r3`） |
| `portfolio-mgr` 输入 | 不再含 `stock_sector` |
| DCF | `dcf.assumptions.applicable = false` + `inapplicableReason` 非空 |
| `portfolio-mgr` 输出 | `valuationApplicability.dcfLegUsed = false`，`reasoning` 含剔除原因 |
| `f5` 估值维度 | 由 graham 腿独立承担（`grahamLegUsed = true`） |
| 风险档 | 中风险（数据形态复算，见 §2③） |
| 分析师流 | 截断节点应带 `streamTruncated = true`（不再静默标 completed） |

---

## 6. 复算命令（可自行验证）

`RISK_*` 用默认值、601166 用上表画像，逐条判据手算即可：

- 极高风险 = `debt>85 && growth<0 && (roe<0 || growth<-10)` → 91.6✓ / −0.3✓ / 4.8✗,−0.3✗ → **假**
- 极高风险 = `vol>60 && sharpe<-1.5` → 21.1✗ → 假
- 高风险 = `(vol>40 || sharpe<0 || dd>45) && (roe<5.0 || debt>65)` → 左全✗ → **假**
- 高风险 = `vol>35 && sharpe<0.3 && roe<8 && growth<5` → 21.1✗ → 假
- 低风险 = `vol<25 && sharpe>0.5 && ...` → sharpe 0.399✗ → 假
- ⇒ **中风险**

---

## 7. 裁决结果（2026-09-14 晚）—— `sector_beta` 采用方案 (b)

**用户裁决**：`sector_beta` 改用**真实历史 beta**（缺失才回退 1.0）。

### 7.1 方案对比（当时给出的三选项）

| 选项 | 内容 | 结果 |
|---|---|---|
| (a) | 数值不动，只暴露 `beta_source`，让静默退化可见 | 未选（治标） |
| **(b)** | **改用真实历史 beta（`compute_beta`），缺失才回退 1.0** | **✅ 采纳** |
| (c) | 补齐关键词表 | 未选（= 把同一个病再犯一次） |

⚠️ 实施 (b) 时**保留了 (a) 的核心价值**：退化必须可见。理由见 4.4。

### 7.2 一个必须记下的事实：`compute_beta` 当时是**死函数**

`analysis-engine/src/portfolio_monitor.rs:140` `pub fn compute_beta(...)` 在全仓库
**只有定义、零调用点**（`grep compute_beta` 仅命中定义 + `output/backup-*/` 副本）。
所以「crate 内已有 `compute_beta`」这句话本身是对的，但它**从未被接线** ——
选 (b) 的副作用之一是**复活了一个死函数**。这也是「N 份实现只 1 份在跑」的又一例。

### 7.3 实施内容

| 动作 | 位置 |
|---|---|
| 删除 `fn sector_beta(sector: &str) -> f64`（7 分支行业关键词 → 1.3/0.7/0.5/0.9/1.1/1.2/0.4/else 1.0） | `portfolio_monitor.rs` |
| 新增常量 `DEFAULT_BETA=1.0` / `BETA_LOOKBACK_DAYS=120` / `BETA_MIN_SAMPLES=30` / `BETA_BENCHMARK_CODE="000300"` | 同上 |
| 新增 `BetaEstimate { beta, samples, historical }` + `BetaMap` + `BetaEstimate::fallback()/estimated()` | 同上 |
| 新增 `date_returns(&[KLine]) -> Vec<(String, f64)>`（**按日期**配对） | 同上 |
| 新增 `beta_from_aligned(stock, market) -> Option<(f64, usize)>`（**按日期交集**对齐 + 样本量门） | 同上 |
| 新增 `async estimate_betas(client, positions, lookback_days) -> BetaMap` | 同上 |
| `run_stress_scenario` 增加 `betas: &BetaMap` 形参；β 取自 `betas[code]`，缺失回退并记账 | 同上 |
| `run_all_scenarios` 增加 `betas` 形参 | 同上 |
| `refresh_metrics` 增加 `betas` 形参（+ `#[allow(clippy::too_many_arguments)]`，参数达 8） | 同上 |
| `StressTestResult` 新增 `beta_provenance: BetaProvenance`（**带 `#[serde(default)]`**） | 同上 |
| 3 个调用点接线：`get_portfolio_dashboard` / `refresh_portfolio_metrics` / `run_portfolio_stress_test` | `src/commands/stock_analysis.rs` |
| 前端类型镜像补 `betaProvenance?` | `src/stores/feature/stockAnalysisStore.ts` |

### 7.4 三个关键设计决定（都不是随手写的）

**① 为什么 `beta_provenance` 必须带 `#[serde(default)]`**

`get_dashboard` 从 `portfolio_metrics_daily.stress_test_json` **反序列化**历史快照：

```rust
let stress: StressTestBundle = m.stress_test_json.as_deref()
    .and_then(|s| serde_json::from_str(s).ok())      // ← .ok() 吞掉一切错误
    .unwrap_or_default();                            // ← 失败即"整个压测数据为空"
```

新字段若无默认值，**所有历史快照都会反序列化失败**，而 `.ok()` 把错误吞掉 ⇒
面板上的压测区**整体静默消失**，且日志里什么都没有。这是本项目最典型的一类故障。

**② 为什么基准不复权、个股前复权**

- 基准（沪深300）：指数无复权概念，与 `backtest.rs:346` / `market_regime.rs` 一致 ⇒ `None`
- 个股：未复权价在**除权日**会凭空产生一段负收益，直接污染回归斜率 ⇒
  `Some(AdjType::Forward)`（项目里算收益的标准做法，见 `backtest.rs:117`）

该口径选择由**测试替身钉死**（`MockProvider` 记录每次 `get_klines` 的 `adj_type`），
否则它只是两行裸参数，日后被改动没有任何东西会拦。

**③ 为什么保留「行业集中度惩罚」（β × 1.2）**

它与「按行业归属给个股下判据」**语义不同**：观测量是「组合在该行业的**暴露占比**」
这一**组合级事实**（越集中，下跌时踩踏越重：流动性折价 + 相关性坍缩），
不改变任何个股的独立判断，且占比不超 40% 时完全不生效。
（同类合理用法还有 `portfolio_formula.rs::portfolio_risk_gate` 的 `sector_exposures`、
`scoring.rs::apply_industry_adjustment` 的相对估值。）

### 7.5 顺带修掉的一个持锁跨 await 隐患

`get_portfolio_dashboard` / `run_portfolio_stress_test` 原先在

```rust
let engine = state.trading_engine.read().await;
let positions = engine.get_positions().await?;
```

之后**一直持有读锁**到函数结束。新增的 `estimate_betas` 会发 N+1 次网络取数（数秒），
若在其后 await ⇒ **握着 `trading_engine` 读锁等网络**，写入方会被阻塞数秒。
已改为作用域收紧：

```rust
let positions = {
    let engine = state.trading_engine.read().await;
    engine.get_positions().await?
};
```

**④ 落库快照与实时面板必须同源**

`refresh_metrics` 原先自己调 `run_all_scenarios(positions, &sector)`（拿不到 client）。
若只改面板不改它，则**快照恒用回退 1.0、面板用真实 beta** ⇒ 按 `as_of_date`
做「时间旅行」对比时，等于在比两套不同的方法。故 `betas` 由调用方传入，
两个入口共用同一份估计。

### 7.6 测试

新增 / 重写（`portfolio_monitor.rs` 的 `mod tests`）：

| 测试 | 钉住什么 |
|---|---|
| `date_returns_drops_pairs_involving_nonpositive_close` | 0 价 bar 的两侧配对都丢弃；收益挂在**后一根** bar 的日期 |
| `beta_from_aligned_recovers_known_slope` | `stock = 1.5 × market` ⇒ 斜率**精确** 1.5（无噪声回归无偏） |
| `beta_from_aligned_aligns_by_date_not_by_position` | market 40 日 / stock 缺首日且尾部少 3 日 ⇒ 重叠恰 36（尾部对齐会得 37） |
| `beta_from_aligned_rejects_unestimable_input` | ① 市场方差 0 ② 样本 < 30 ⇒ 都判不可估计 |
| `stress_scenario_fallback_is_neutral_and_visible` | **负控**：无估计 ⇒ 恰 -10%，且 `fallback=1` + `fallbackCodes` 可见 |
| `stress_scenario_no_longer_uses_sector_keyword_table` | **反向断言**：旧口径 `sector_beta("金融")=0.5` ⇒ -5%，断言已**不再**是 -5% |
| `estimate_betas_uses_real_regression_and_correct_adj_type` | β≈2.0 + `samples` = 59 + **基准 `None` / 个股 `Forward`** 口径 |
| `estimate_betas_falls_back_when_benchmark_unavailable` | 基准拉不到 ⇒ 每个持仓仍**有条目**（不是缺键），`historical=false` |
| `estimate_betas_falls_back_when_stock_klines_fail` | 个股拉不到 ⇒ 单独回退，不被基准缺失的早退分支吞掉 |
| `estimate_betas_empty_positions_is_empty_without_any_fetch` | 无持仓 ⇒ 零取数（不白跑网络） |
| 删除 `sector_beta_buckets` | 该表已不存在 |

### 7.7 行为变更（对用户可见）

| 场景 | 旧 | 新 |
|---|---|---|
| `sector_name` 命中关键词表（如「科技」） | β=1.3（写死） | β=该股对沪深300的 120 日回归；无数据 ⇒ 1.0 |
| `sector_name` 未命中（实测**大多数**，门类级口径） | β=1.0（**静默**） | β=真实估计；无数据 ⇒ 1.0 **且记账可见** |
| `StressTestResult.note` | "…sector_beta × 大盘跌幅" | "…β_i（120 日真实历史 beta，对沪深300回归）× 大盘跌幅"；有回退时追加"N/M 只无足够历史样本，β 回退中性 1" |

⇒ **压测数值会变**（尤其原先落在 `else 1.0` 的标的，现在会用真实 beta）。这是预期内的口径纠正。

---

## 8. 本轮验证与一次自我纠错（2026-09-14 深夜）

### 8.1 验证矩阵（全部为实测）

| 步骤 | 命令 | 结果 |
|---|---|---|
| STEP2 | `cargo test --test rhai_syntax_check` | ✔ `EXIT=0` 3/3（`all_rhai_scripts_compile` / `bottleneck_calc_v9_compiles` / `portfolio_mgr_has_no_hold_wait_mutual_rewrite`） |
| STEP3 | `cargo test -p axagent-astock-data --lib valuation_tests` | ✔ `EXIT=0` 27/27（**两条链独立复跑**，结论一致） |
| A | `cargo test -p axagent-analysis-engine --lib portfolio_monitor` | ⚠ 28 条：27 通过 / **1 失败**（见 8.2；已修，待复跑） |
| 前端 | `node node_modules/typescript/bin/tsc --noEmit` | ✔ `EXIT=0` |
| fmt | `cargo fmt` | ✔ `EXIT=0` |
| STEP4 | `cargo test -p axagent --lib stock_analysis_setup` | ⏳ 编译中（主 crate 体量） |
| STEP5 | `cargo clippy --all-targets -- -D warnings` | ⏳ 排队 |

> ⚠ **时效性陷阱（本轮实际踩到）**：STEP1（4 个 crate `--all-targets`，`Finished in 16m 20s`）
> **早于**本轮对 `analysis-engine/portfolio_monitor.rs`（14:27:07）与 `commands/stock_analysis.rs`
> （14:26:38）的改动 ⇒ STEP1 的 `EXIT=0` **不能**用作本轮改动的证据。有效证据是 STEP4/STEP5
> （在改动之后启动，会重编到新版本）。**「先跑的检查」不覆盖「后做的改动」**，这是纪律 6 的又一变体。

### 8.2 一条失败测试的失败与修复（判据 **#174** 出处）

新测试 `beta_from_aligned_aligns_by_date_not_by_position` 首跑 **FAILED**：

```
panicked at crates\analysis-engine\src\portfolio_monitor.rs:1233:63:
36 ≥ 30 应可估计
```

- **现象**：断言「重叠 36 ≥ 30 应可估计」失败 —— 看起来像**对齐逻辑写错了**。
- **真因**：测试数据用了**常数序列**（market 恒 0.002 / stock 恒 0.003）⇒ 市场收益**方差为 0**
  ⇒ 走进 `compute_beta` 的「不可估计」分支返回 `None`。**是数据构造错，不是被测逻辑错。**
- **修法**（两件事，第二件更重要）：
  1. 换成带波动的序列（`sin(i × 0.41) × 0.01`），令 stock = market × 1.5 ⇒ beta 期望**精确 1.5**；
  2. 加**错位对照**：把同一组 stock 收益挂到 market 尾部 36 日的日期上（时间轴整体后移 3 天），
     断言斜率**显著偏离 1.5**（实测 ≈ 0.50）。⇒ 本条测试**自证区分力**：若这组数据对
     「按日期对齐」与「按位置尾对齐」给出相同结果，测试自己会失败告警。
- **沉淀** → 铁律 **#174**（组 E）：*测试数据须自证区分力*。

> 这是**纪律 8**（统计量恒等或异常 ⇒ 先怀疑测量工具）的新变体：
> 这里的「测量工具」是**测试数据**；它退化后伪装成被测逻辑的缺陷。

### 8.3 顺带修掉的文档缺陷

本报告在追加 §4 时产生了**重复章节号**（"4. 登记待裁决" 与 "4. 裁决结果" 并存）
⇒ 已把后者重编为 **§7（7.1–7.7）**。纯编辑事故，非内容问题。

### 8.4 本轮**不在**改动范围的项（登记）

- **行业集中度惩罚（`> 40% × 1.2`）保留**。它是**组合级事实**（组合整体的行业暴露度是否超阈值），
  与「按行业给**个股**下判据」（已删除的 `sector_beta` 关键词查表）语义不同：
  前者对任何行业一视同仁地度量「是否集中」，后者则给了「科技股天生 β=1.3」这种**与个股无关**的先验。
- 记忆注入层的引用卫生修复（悬空指针 / 歧义指针 / 条数三套值）→ `MEMORY-DETAIL.md` **§58**。

---

## 9. ⚠ 本轮发现的一个**可见性缺口**（登记，未擅自改）

**事实**：`BetaProvenance.fallback_codes`（**具体哪几只**持仓的 β 回退了）已计算、已序列化、
已在前端类型里镜像（`src/stores/feature/stockAnalysisStore.ts:312–318`），但**零消费端** ——
全前端 grep `betaProvenance|fallbackCodes` 只有 **3 处命中，全在类型定义块内**，
**没有任何 UI 渲染它**。

**用户实际看到的是**：`note` 文案里的**计数**（"3/12 只无足够历史样本，β 回退中性 1"），
**看不到是哪 3 只** ⇒ 无法核查「这几只的压测跌幅是否可信」。

**为什么不当场改**：展示形态属**产品决策**，且两个方案成本差很大：

| 方案 | 改动面 | 成本 | 风险 |
|---|---|---|---|
| **A. 追加进 `note`** | 后端 1 处 `format!` + 截断策略（> N 只用「…等」） | **低** —— `note` 已在 `PortfolioMonitorPanel.tsx:334` 与 `RiskRoomMiniBar.tsx:300` 渲染，无需改 UI、无需 i18n | `note` 是中文硬编码自由文本（**既有形态，非本轮引入**）；列表长时可读性下降 |
| **B. 前端独立列表** | 两个消费面各加一段 + 新增 i18n key **× 11 语言** | 中高 | 两处消费面都要改，且要与面板既有布局协调 |

**建议**：先做 **A**（成本最低、立刻可核查）；若后续要做「点代码跳到该股详情」，再升级为 B。

> 附注：`note` 的中文硬编码是**既有形态**（本轮改动前就存在），不在本轮范围内。
> 若要与 §9.16 已确立的「失败结构化码 + 模板 + 前端本地化」口径统一，应作为**独立一轮**处理
> （涉及 `TEMPLATE_VERSION` 或 DTO 变更，不宜与本次口径纠正混做）。

---

## 10. 实测验证终态（2026-09-15 凌晨，全部取证落盘）

### 10.1 五级验证链

| 步骤 | 命令 | 结果 | 覆盖了本轮什么 |
|---|---|---|---|
| STEP1 | `cargo check -p {astock-data,rt-workflow,analysis-engine,tools,company-runtime,runtime} --all-targets` | ✔ `EXIT=0`（`Finished in 16m 20s`） | ⚠ **时效性无效** —— 见 10.2 |
| STEP2 | `cargo test --test rhai_syntax_check` | ✔ `EXIT=0` 3/3 | V77 DCF 门语法合法；`portfolio_mgr_has_no_hold_wait_mutual_rewrite` 守护成立 |
| STEP3 | `cargo test -p axagent-astock-data --lib valuation_tests` | ✔ `EXIT=0` 27/27 | DCF 适用性门 + ROE/Graham/永续增长全套判据 |
| STEP4 | `cargo test -p axagent --lib stock_analysis_setup` | ✔ `EXIT=0` 9/9 | 模板播种一致性（工具可解析、debater 轮次参数化、round 字面量扫描） |
| **BETA** | `cargo test -p axagent-analysis-engine --lib portfolio_monitor` | ✔ `EXIT=0` **28/28** | **本轮 (b) 全部新增/重写测试** |
| STEP5 | `cargo clippy --workspace --all-targets -- -D warnings` | ⏳ 主 crate 编译中 | CI 硬门禁 |

旁证（与 CLI 环境坑规避相关）：
- 前端 `tsc --noEmit` → ✔ `EXIT=0`（**必须直调 `node_modules/typescript/bin/tsc`**，`npm run typecheck` 在 shim 下报 `/usr/bin/env: 'bash': No such file or directory`）
- `cargo fmt --check` → ✔ `EXIT=0` 零差异
- `./node_modules/dprint/dprint.exe check` → ✔ `EXIT=0`（注意 `dprint.json` **未排除** `docs/` 与 `.workbuddy/` 的 markdown ⇒ 本次改的报告/记忆文件也过了格式门禁）

### 10.2 ⚠ 一条差点被误用的「证据」：STEP1 是假阳性

STEP1 于 **14:24** 结束，而 `portfolio_monitor.rs` 改动落在 **14:27:07**、`commands/stock_analysis.rs` 落在 **14:26:38** —— **均在其后**。
「日志里有 `Finished` ⇒ 本轮代码编译通过」这个推论**不成立**。有效证据只能是 STEP4（重编了 `axagent-analysis-engine`）与 BETA（重编并运行新测试）。

> 判据沉淀：**验证步骤与改动的先后顺序本身就是证据的一部分**；跨轮次复用的编译日志必先核对 mtime 再引用。这是纪律 6（「编译通过」三型假阳性）的**第四型**：*时序型假阳性* —— 不是日志骗人，是**引用的日志早于被测对象**。

### 10.3 新增/重写测试（analysis-engine，28 条中本轮占 10 新 + 4 重写）

| 测试 | 守护的判据 | 类型 |
|---|---|---|
| `date_returns_drops_pairs_involving_nonpositive_close` | 0 价 bar 参与的**两侧配对都丢**；收益挂在后一根 bar 日期 | 边界 |
| `beta_from_aligned_recovers_known_slope` | 严格线性 stock=1.5×market ⇒ beta **精确 1.5** | 正确性 |
| `beta_from_aligned_aligns_by_date_not_by_position` | 40∩36=36 样本；**含错位对照自证区分力** | **反向断言** |
| `beta_from_aligned_rejects_unestimable_input` | 市场方差 0 / 样本 29 ⇒ `None`（不硬算） | 负控 |
| `stress_scenario_fallback_is_neutral_and_visible` | 全回退时恰 `-10%`（严格中性）且 provenance 可见 | 负控 |
| `stress_scenario_no_longer_uses_sector_keyword_table` | **不再是** -5%（旧行业查表值）⇒ 表已死 | **反向断言** |
| `estimate_betas_uses_real_regression_and_correct_adj_type` | β≈2.0、`samples=59`、基准 `None` / 个股 `Forward` | 取证（替身记录实参） |
| `estimate_betas_falls_back_when_benchmark_unavailable` | 基准取数失败 ⇒ **每个持仓仍有回退条目**（不缺键） | 降级 |
| `estimate_betas_falls_back_when_stock_klines_fail` | 个股取数失败 ⇒ 该标的单独回退 | 降级 |
| `estimate_betas_empty_positions_is_empty_without_any_fetch` | 空持仓 ⇒ **零网络调用** | 负控 |

**首跑失败的那条**：`beta_from_aligned_aligns_by_date_not_by_position` 报 `panicked: 36 ≥ 30 应可估计`。
真因是**测试数据用了常数序列** ⇒ 市场收益方差为 0 ⇒ 落入「不可估计」返回 `None`。**实现无误，数据错**。
修法：① 换带波动序列；② **加错位对照**（同一组收益贴到日期后移 3 天的位置，斜率必须偏离 1.5）⇒ 该测试现在能自证「错误实现会给出不同结果」。沉淀为铁律 **#174**（组 E）。

### 10.4 最终交付物状态

| 文件 | 状态 |
|---|---|
| `crates/analysis-engine/src/portfolio_monitor.rs` | 删行业查表 + `BetaEstimate`/`BetaMap`/`estimate_betas`/`date_returns`/`beta_from_aligned` + DTO 兼容字段 + 10 新测试 |
| `src/commands/stock_analysis.rs` | 3 个调用点接线 + 2 处锁作用域收紧（`drop(engine)` / 作用域块） |
| `src/stores/feature/stockAnalysisStore.ts` | `betaProvenance?` 可选镜像（后端 `#[serde(default)]`） |
| `.workbuddy/memory/{MEMORY,MEMORY-RULES,MEMORY-DETAIL,2026-09-14}.md` | 铁律 #172/#173/#174；注入层引用腐烂全修（0 悬空）；预算余量 182 → 362 B |
| `docs/audits/AUDIT-industry-label-removal-2026-09-14.md` | 本文（§0–§10） |
| `output/check-mem-refs.mjs` | 新增记忆引用校验器（裸 `§N` / 文档名 / 铁律编号三类） |
| 技能 `datashape-vs-label-judgment/SKILL.md` | §4.8「替代实现的三类守护测试」 |

### 10.5 STEP5 clippy 首跑失败 —— 测试替身撞上 `disallowed_types`（已修）

首跑 `STEP5_EXIT=101`，2 个 error，**全在新增的测试替身里**：

```
error: use of a disallowed type `std::sync::Mutex`
  --> crates\analysis-engine\src\portfolio_monitor.rs:1287:15   (结构体字段类型)
  --> crates\analysis-engine\src\portfolio_monitor.rs:1292:34   (构造表达式 `Self { seen: ... }`)
error: could not compile `axagent-analysis-engine` (lib test) due to 2 previous errors
```

**真因**：`clippy.toml:10-16` 的 `disallowed-types` 列了 5 个 std 锁类型（`RwLock` / `Mutex` / `RwLockReadGuard` / `RwLockWriteGuard` / `MutexGuard`），`Cargo.toml:96` 配 `disallowed_types = "deny"` —— 这是铁律 #8（异步锁必须 `tokio::sync`）的 lint 化。我的 `MockProvider` 用 `std::sync::Mutex` 记录每次 `get_klines` 的 `(code, adj_type)`，目的是钉住「基准 `None` / 个股 `Forward`」这一复权口径。

**关键认知**：`cargo check` 与 `cargo test` **完全不跑这条规则** —— 它们全程绿。拦住它的只有 clippy，且**只有加了 `--all-targets` 才覆盖 test target**。
⇒ **「编译通过 + 测试通过 ≠ 合规」**；新写的测试代码本身也是门禁对象。

**修法：按项目自己的规约修，不改语义。** `clippy.toml:6-8` 明写合法例外 ——
「不跨 await 的同步临界区需 `#[allow(clippy::disallowed_types)]` + SAFETY 注释」，全仓已有 7 处先例（含 `crates/harness/src/test_support.rs:13`）。
故给 `struct MockProvider` 与 `impl MockProvider` 各加一条 `#[allow]` + 按模板写 SAFETY 注释（`seen` 的 lock guard 是语句级临时量，函数内无 `.await` 点）。

> **反面做法**：为了过门禁把 `std::sync::Mutex` 换成 `tokio::sync::Mutex` —— 那是**改语义**（测试替身并不需要异步锁的可重入语义），且会引出「`adj_for` 是同步函数」的连锁改造。判据：**先 grep 该 lint 的既有豁免先例，有就照抄形态**。

| 检查 | 结果 |
|---|---|
| `cargo fmt --check` | ✔ `EXIT=0` |
| `cargo clippy -p axagent-analysis-engine --all-targets -- -D warnings` | ✔ `EXIT=0`（2m02s） |
| `cargo clippy --workspace --all-targets -- -D warnings` | ⏳ 全量重跑中 |

⚠ **连带判据**：clippy 遇到首个 error 会**提前终止**，后续 crate **从未被检查** ⇒ **不能**由「只有 2 个 error」推定主 crate `axagent`（含 `stock_analysis.rs` 的 3 个调用点）干净。必须全量重跑背书。这与纪律 6 ①（日志不分 target）是**两个不同**的坑：① 是「覆盖率不足」，这条是「失败即短路」。

### 10.6 ⚠ 第 10.1 节 STEP1 的「证据」实为**时序型假阳性**（纪律 6 的第四型）

STEP1 于 **14:24** `Finished`，而 `portfolio_monitor.rs` 改动落在 **14:27:07**、`commands/stock_analysis.rs` 落在 **14:26:38** —— **均在其后**。

「日志里有 `Finished` ⇒ 本轮代码编译通过」**不成立**。前几轮我一直在用「引用早于被测对象的编译日志」当有效证据，这是新的一型：
**不是日志撒谎，是引用的日志早于被测对象。**
有效证据只能是 STEP4（重编了 `analysis-engine`）、BETA（重编并运行新测试）、STEP5/AE（重编并 lint 新代码）。

沉淀 **铁律 #175（组 L）**，并挂进注入层纪律 6 的 `④` 与 L 组召回行。
**判据**：跨轮次复用任何编译/测试日志前，先比对日志 mtime 与源文件 mtime；早于改动的一律作废。

---

## 11. 全量 clippy 第二轮：**第二个、独立的 error**（清掉后才是真门禁）

### 11.1 现象

修完 §10.5 的 `disallowed_types` 后重跑，`-p axagent-analysis-engine --all-targets -- -D warnings` 立刻 `EXIT=0`（2m02s）。但**全量**那一轮：

```
error: items after a test module
  --> src\commands\stock_analysis_setup\mod.rs:1311:1
     = note: `-D clippy::items-after-test-module` implied by `-D warnings`
error: could not compile `axagent` (lib test) due to 1 previous error
CLIPPY_WS_EXIT=101
```

### 11.2 为什么第一次没看见 —— 这是「门禁覆盖有洞」而非「偶发」

首跑的 clippy 命令是 `-p axagent-astock-data -p axagent-analysis-engine -p axagent-rt-workflow -p axagent --all-targets -- -D warnings`，**四个 crate 全列了**，看起来覆盖完整。

但 clippy 在 `analysis-engine` 的 `disallowed_types` 上失败后**短路**（`error: could not compile ... waiting for other jobs to finish`），`axagent (lib test)` **从未被 lint**。于是：

| 轮次 | analysis-engine | axagent (lib test) |
|---|---|---|
| 首跑（4 crate） | ✗ 2 error（`disallowed_types`） | **未检查** |
| 定点（仅 analysis-engine） | ✓ `EXIT=0` | **未检查** |
| 全量（`--workspace`） | ✓ 缓存命中 | ✗ 1 error（`items_after_test_module`） |

**判据**：「`-p` 把相关 crate 都列了」≠「都检查了」。只要任一个失败，**其后的 target 全部未检查**；因此**修完一处必须重跑全量**，不能凭「上次命令里带了 `-p axagent`」推定它干净。此实证已同步进 `AGENTS.md` 的「Rust 验证门禁」新节（判据 ②）。

### 11.3 归属：这是**本轮对话早前轮次**引入的，不是历史遗留

不碰 git，用备份比对定性：

| 证据 | 事实 |
|---|---|
| `output/backup-2026-09-03/stock_analysis_setup/mod.rs`（2073 行） | **不含** `force_variable_value` 函数，**不含** `force_variable_value_tests` |
| 当前文件（2300 行） | 两者都在 |

⇒ `force_variable_value` + 其测试是 **09-03 之后、v48 模板版本时期**加入的。当时 test mod 紧跟在 `force_variable_value` 函数之后（L1310），**而该位置之后还有 915 行非空生产代码**（`seed_reflection_workflow_template`、`EventTriggeredTemplateSpec`、`seed_event_triggered_decision_template`、`seed_auto_position_plan_template`、`seed_auto_stop_loss_review_template`）。

### 11.4 修法：块移动，不动生产代码

clippy 给两种等价建议：① 把 test mod 之后 items 移到 test mod 之前；② 把 test mod 移到那些 items 之后。选 ② —— 移动 **40 行测试** vs 移动 **915 行生产代码**，风险差两个数量级。

```text
移动前：L1288-1308 force_variable_value 函数
        L1310-1349 #[cfg(test)] mod force_variable_value_tests { … }
        L1361-2300 五个生产项   ← clippy 报的就是这里
移动后：L1288-1308 force_variable_value 函数
        L1361-2258 五个生产项
        L2260-2299 #[cfg(test)] mod force_variable_value_tests { … }   ← 文件末尾
```

`use super::force_variable_value;` 的解析路径**不变**（`super` 仍是 `stock_analysis_setup` 自身）⇒ 测试可编译性不受影响。已用 `cargo test -p axagent --lib stock_analysis_setup` 复验（见 §11.5）。

### 11.5 自查设计的一次纠错：**我的第一条判据是个必然误报的假阳性**

脚本首跑被自己的断言拦下、**拒绝写入**（脚本工作正常，判据错了）：

```
❗去空白后文本不一致 ⇒ 不只是移动，字符被改动
   去空白字节 原文=67814 新文=67814        ← 长度一字节不差
```

判据写的是「全文去空白后逐字节相等」。**这条对「移动」类操作注定为假** —— 移动必然改变「按顺序拼接」的结果，与是否改动字符无关。长度相同已经说明没增删字符，只是顺序变了。

改成锚定**被测对象自身形态**（纪律 2）的三条：

| 判据 | 含义 | 实测 |
|---|---|---|
| `strip(块)` 前后相等 | 移走的那 40 行**一字未改** | ✓ |
| `strip(其余)` 前后相等 | 其余 2260 行**一字未改** | ✓ |
| 字符多重集（排序后）相等 | 全文没有凭空增删字符 | ✓ |

比多重集更强的两条（块/其余各自不变）合起来即证明「原封不动的两块换了拼接顺序」。

### 11.6 验证

| 项 | 结果 |
|---|---|
| 文件字节 | `112147 → 112147`（一字节不差） |
| 行数 | `2300 → 2300` |
| test mod 新位置 | `L2260–2299`，其后非空行 **0** |
| `cargo fmt --check` | **`EXIT=0`** 零差异 |
| `cargo test -p axagent --lib stock_analysis_setup` | 见 `verify5.log` |
| `cargo clippy --workspace --all-targets -- -D warnings` | 见 `verify5.log`（本轮最终门禁） |
| 备份 | `output/backup-2026-09-14-clippy/mod.rs.bak`（112147 B，与改前逐字节相同） |

### 11.7 沉淀

- **规约**：新增内联 `#[cfg(test)] mod xxx { }` **必须追加到文件末尾**，不要插在「刚写的那个函数」后面。Rust 惯例如此，且 clippy 有 `items_after_test_module` 强制；而该 lint **只在 clippy 下暴露**（`cargo check` / `cargo test` 全绿也照样红），意味着错误会潜伏到提交时才炸。
- 类型归属：这是纪律 6「编译通过三型假阳性」的**又一实证**，且与 `#175`（时序型）不同 —— 本案是**空间型/覆盖型**：日志是真的、结果是真的，但**覆盖范围**没到被测对象。判据 ② 已入 `AGENTS.md`。
