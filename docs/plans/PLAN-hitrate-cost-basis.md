# PLAN：命中率链成本口径修复（G4）— 2026-10-25

> 前置裁决：`AUDIT-stock-profitability-verdict-2026-10-25.md` §1.3 与 G4 门槛。
> 目标：把「尺子」改对 —— 在跑任何回放/扩样本之前，先让命中率读数本身可信。
> 本文记录**已实施**的改动、口径裁定与验证状态。

---

## 1. 修的是什么

三处相互放大的缺陷，合起来让所有胜率/均值系统性虚高：

| # | 缺陷 | 位置（符号定位，抗行号漂移） |
|---|---|---|
| ① | 收益**完全不扣成本** | `hit_rate_backtest::compute_price_metrics` 的 `final_return_pct` |
| ② | 判定 `ret > 0` ⇒ **+0.01% 也算赢** | `hit_rate_backtest::compute_hit_outcome`（入参原为毛收益） |
| ③ | **两套胜率口径并存**且无人知道该看哪个 | `ActionStats::hit_rate`（partial 不计输 = 54%）vs `AUDIT` 用的 42.5%（partial 计输） |

同一 crate 里 `backtest.rs` 的另一个回测模块**是扣 0.18% 的**，而喂给闭环权重、
命中率面板、先验校准的是**不扣成本**的那条 —— 「测量工具自己有两种刻度」。

---

## 2. 改动清单

### 2.1 新增 `costs` 模块（成本唯一来源）

`crates/analysis-engine/src/costs.rs`（新文件），并在 `lib.rs` 声明。

| 常量 | 值 | 说明 |
|---|---:|---|
| `COMMISSION_BOTH_SIDES_PCT` | 0.08 | 佣金双边（单边万 4，保守中位） |
| `STAMP_DUTY_PCT` | 0.10 | 卖出印花税（**刻意取回 0.1%**：2023 减半至 0.05%，但政策可回调；且历史口径一直按 0.1% 计，改小会让新旧读数不可比） |
| `SLIPPAGE_PCT` | 0.07 | 滑点双边（散户实操中位，保守侧） |
| `ROUND_TRIP_COST_NO_SLIPPAGE_PCT` | **0.18** | = 佣金 + 印花税。**`backtest.rs` 的历史口径** |
| `DEFAULT_ROUND_TRIP_COST_PCT` | **0.25** | = 三项之和。**命中率链一律用这个** |

工具函数：`net_of_cost(gross, cost)`（纯减法，**不 clamp**）、`net_return_pct(gross)`。

> 为什么 `ROUND_TRIP_COST_NO_SLIPPAGE_PCT` 单独保留而不直接并入 0.25%：
> `backtest.rs` 已落库的 `accuracy` 是按 0.18% 算的，换值会全体漂移，
> 那属于另一次「换版本号」的改动，不并入本次。

### 2.2 `backtest.rs` 去手抄

`const COST_RATE: f64 = 0.0018` ⇒ `crate::costs::ROUND_TRIP_COST_NO_SLIPPAGE_PCT / 100.0`。
**行为完全不变**（0.18/100 = 0.0018，有单测锁），只是来源单点化。

### 2.3 净收益改为**恒可算的方法**（关键设计）

`PriceMetrics::net_return_pct(cost)` / `net_return_pct_default()`、
`PickValidation::net_return_pct()`。

**为什么是方法不是字段** —— 这是本次最重要的结构决策：

- 加字段要改**所有构造点**（`build_pick_validation`、`serenity_validation_to_row`、
  `backtest_validation` 的 DB 行映射，外加 6 处单测字面量）。只要有一处漏填，
  净口径就**静默退回毛口径** —— 正是本次要修的病；
- `final_return_pct` 是**已落库列**，历史行存的是毛收益。方法恒可从毛收益现算
  ⇒ **历史样本自动获得净口径，零 migration、零回填**。

### 2.4 判定改用净收益

`build_pick_validation`：`compute_hit_outcome(&action, m.net_return_pct_default(), ...)`。
`hit_stop_loss` / `hit_target` 仍由价格触发（与成本无关），原样保留。

`compute_hit_outcome` 的**入参语义**改为净收益，签名/函数体不变（`> 0` 仍是
「实盘赚钱」，变的是零点的基准）。文档补齐「顺序即优先级」——原文档写的顺序
（`ret>0` 先于 `stop_loss`）与函数体实际顺序（`stop_loss` 优先）**不符**，一并修正。

### 2.5 落库行加口径标记

`build_pick_validation` 的 `data_source` 追加 `+net_cost`（`net_cost_data_source`，**幂等**，
可与既有 `+premature_window` 共存）。

原因：同一行里 `final_return_pct` 是**毛**、`hit_outcome` 是**净**判，
不标记就无法区分「毛判定的历史行」与「净判定的新行」。

### 2.6 `ActionStats` 补齐三个口径

| 字段 | 口径 | partial 归属 |
|---|---|---|
| `hit_rate` | **legacy** = hit/(hit+miss+false_hit) | 不进分母（读数偏高） |
| `strict_win_rate` + `strict_denom` | hit/(hit+miss+false_hit+**partial**) | **计输**（与 AUDIT 42.5% 同算法） |
| `net_avg_return_pct` / `net_win_rate` + `net_win_denom` | 逐条扣 0.25% 后求均值 / 净 > 0 占比 | 由净收益符号定 |

`net_win_denom` 单列出来，是为了让 0 分母不被展示成「0% 胜率」——
「一条没赢」与「没有样本」含义相反。

净口径**不依赖落库的 `hit_outcome`**（历史行是毛判的），而是从 `final_return_pct`
现算 ⇒ 历史样本一并进净统计，新旧同口径。

---

## 3. 口径裁定：三套「胜负」刻意不同（不是漂移）

| 出口 | 回答的问题 | partial（-5%~0 微亏） |
|---|---|---|
| `hit_outcome_to_binary_outcome` → `stock_analyses.outcome` | 方向判对了吗 | **win**（微亏仍算看对） |
| `ActionStats::strict_win_rate` | 按这判断会不会亏 | **输** |
| `ActionStats::net_win_rate` | 扣成本后赚没赚 | 由净收益符号定 |

把第一行改成 partial→loss 会让「方向对了、成本吃掉利润」被记为决策失败，
污染的是**决策质量**归因；后两者只关心**钱**。已在该函数文档里写成对照表防误改。

---

## 4. 验证状态

| 项 | 状态 |
|---|---|
| `cargo fmt --all --check` | ✅ 0 diff |
| `cargo check -p axagent-analysis-engine` | ✅ 通过（25.14s） |
| `cargo test -p axagent-analysis-engine --lib` | ✅ **859 passed / 0 failed** |
| `cargo clippy -p axagent-analysis-engine --all-targets -- -D warnings` | ✅ 通过（31.76s） |
| 定向 `--lib hit_rate` | ✅ **43 passed / 0 failed**（含核心负控） |
| `+net_cost` 是否真落库 | ✅ 已核：`build_pick_validation` 加标记 → `backtest_validation.rs:438` 构建 → `:638` `Set(v.data_source.clone())` 写库，中途无覆盖 |

> **首轮测试有 1 个失败（我自己的新测试）**，值得记录：`no_net_samples_yield_zero_denominator_not_zero_win_rate`
> 首版断言 `strict_denom == 0`，实测 1。
> 原因是它构造了**现实中不可能**的状态（`hit_outcome="hit"` 但 `final_return_pct=None`）——
> `hit` 只可能由"有收益且为正"派生。修真：改用 `insufficient`。
> 并**因此补了两条断言**：`strict_denom`（吃 `hit_outcome`）与 `net_win_denom`（吃
> `final_return_pct`）**不同源**，正常数据同源、残缺行会分叉，不得假定恒等 ——
> 这条差异已写进 `ActionStats` 的字段文档。

新增单测（`hit_rate_backtest::tests`，追加在 mod 末尾）：

1. `tiny_gross_gain_is_not_a_hit_after_cost` —— **核心负控**：毛 +0.10% 扣成本后为负，
   不得判 hit；同时断言「若传毛收益会被判 hit」证明该断言有区分力
2. `strict_win_rate_counts_partial_as_loss` —— legacy 0.5 vs strict 1/3，并断言两者不相等
3. `net_win_rate_uses_cost_adjusted_threshold` —— 净胜率 1/4 < 毛口径 2/4
4. `no_net_samples_yield_zero_denominator_not_zero_win_rate` —— 0 分母 ≠ 0% 胜率
5. `strict_and_net_denominators_diverge_on_incomplete_rows` —— 两分母不同源的负控
6. `net_cost_marker_is_idempotent` —— 标记幂等 + 与 `+premature_window` 共存
7. `costs::tests` 4 个（聚合自洽、与 0.0018 等价、不 clamp、零成本负控）

---

## 5. 明确未做（边界）

| # | 事项 | 为什么不做 |
|---|---|---|
| N-1 | 落库 `final_return_pct` 改存净收益 | 会破坏历史可比性；毛/净并存的正确解法是「毛落库 + 净恒时现算」，已实现 |
| N-2 | 回填历史行的 `hit_outcome` | 无法区分「当时按毛判」与「现在按净判」；净统计已绕开该列，回填无收益 |
| N-3 | `backtest.rs` 切到含滑点 0.25% | 会让已落库 accuracy 全体漂移，须单独换版本号 |
| N-4 | 前端展示新字段 | 前端无 `ActionStats` TS 类型（已核），新增字段不会破坏；UI 接入属独立任务 |
| N-5 | 涨停不可成交过滤、跌停不可卖出 | 属回放/回测的执行摩擦，不是命中率尺子本身；见 `AUDIT-replay-sample-feasibility-2026-10-25.md` §6.4 |
| N-6 | `serenity_hit_rate_backtest` 的 `excess_*` 标签域 | 该通道的 `hit_outcome` 是另一套标签，本就排除在 `compute_hit_rate_report` 之外（`_ => insufficient`）；其净收益经 `PickValidation::net_return_pct()` 已自动生效 |

---

## 5.1 已登记、未改：第四处口径（先验校准链）

`reflection_stats::deterministic_horizon_was_correct` 的 `was_correct` 取
`return_pct` 的**符号**（毛口径，扣成本前）→ `build_hitrate_stats`
→ `direction_hit_rate` → `horizon_prior` 收缩 → 分支后验 → **Kelly 仓位**。

即：**先验校准用的仍是「方向准确率（毛）」，不是「盈利概率（净）」**。

为什么不并入本次：

- 它判的是「方向对不对」，与 §3 表第一行同类 —— 微亏（毛 +0.10%）确实"看对了方向"，
  `was_correct=1` 在**决策质量归因**上是成立的；
- 但它被拿去当**胜率**喂 Kelly，而 Kelly 要的是「赢的钱与输的钱的比值下的赢的概率」。
  用方向准确率代替盈利概率，就是 `PLAN-stock-decision-reliability.md` R7
  「后验概率未经校准直接进凯利」的一部分。
- 改它会让**生产仓位**变化，属产品语义决策（门槛、κ、先验收缩策略都要一起定），
  不是单纯的口径修正。

⇒ 本次只把「读数」改对；**先验→仓位这条链仍是毛口径**，等裁决。
在此之前，界面上的 `net_win_rate` 与决策实际使用的先验**不可互相印证** —— 这一点必须已知。

---

## 6. 验证阻塞记录

本轮 `cargo check` 首次失败：`failed to open target/debug/.cargo-build-lock`（os error 5）。
排查结论：**不是僵死锁** —— `Get-CimInstance` 查到另一个会话正在跑
`cargo check -p axagent --lib`（创建于 03:34:29，rustc 在编主 crate）。

按「不抢他人锁」的处置：改用不依赖 `target` 的 `cargo fmt` 先收格式，编译验证等锁释放。
**未使用** `CARGO_TARGET_DIR` 临时目录兜底，因为那会触发全量重编
（含 `aws-lc-sys`，实测 10 分钟级），在只改了一个小 crate 的情况下不划算。

---

## 7. 下一步

尺子改对之后，才能谈量长度：

1. ~~跑单测~~ ✅ 已全绿（§4）；
2. **用新读数实测重算 127 单** ✅ 已完成，见 §8；
3. 之后才进入回放方案（`AUDIT-replay-sample-feasibility-2026-10-25.md` §6 的顺序）。

---

## 8. 实测重算（2026-10-25，生产库 `axinvest`）

### 8.1 先复现基线（证明筛选口径与审计一致，不是另算一份）

筛选：`inferred_action='buy'` + `style<>'serenity'` + `as_of_date IS NULL`（实盘代）
+ `data_source NOT LIKE '%premature_window%'` + 期窗匹配（`ultrashort@2` / `short@5` /
`mid@28` / `long@90`）。

| n | hit | partial | miss | false_hit | 毛均值 | strict 胜率 |
|---:|---:|---:|---:|---:|---:|---:|
| 127 | 54 | 27 | 45 | 1 | +0.719% | 42.5% |

**与 `AUDIT-stock-live-data-2026-10-24.md` §4.1 逐位一致**（含 +0.72% 与 42.5%），
说明下面的对比是同一分母上的「同一批样本、两把尺子」。

### 8.2 新旧口径对比

`new_strict` = 按新判定重算的 strict 胜率（止损优先，`net > 0` 或触及目标才 hit）；
`net_win` = 净收益 > 0 的占比（纯收益口径，不看止损标签）。

| 格 | n | 毛均值 | **净均值** | 旧 strict | **新 strict** | **净胜率** |
|---|---:|---:|---:|---:|---:|---:|
| capital_short | 40 | +3.41 | **+3.16** | 47.5 | 47.5 | 50.0 |
| value_short | 29 | +0.69 | **+0.44** | 55.2 | 55.2 | 55.2 |
| trend_short | 12 | −1.68 | **−1.93** | 25.0 | 25.0 | 25.0 |
| capital_ultrashort | 30 | −0.89 | **−1.14** | 36.7 | 36.7 | 36.7 |
| value_ultrashort | 10 | −0.89 | **−1.14** | 40.0 | **20.0** | **10.0** |
| trend_ultrashort | 6 | −1.57 | **−1.82** | 16.7 | 16.7 | 16.7 |
| **合计** | **127** | **+0.72** | **+0.47** | **42.5** | **40.9** | **40.9** |

### 8.3 三个值得单独说的结果

**① 合计：42.5% → 40.9%，均值 +0.72% → +0.47%。**
下降幅度（−1.6pp / −0.25pp）与 §7 预估一致。掉出 `hit` 的是 2 单——毛收益落在
(0, 0.25%] 的微利样本，实盘是净亏。

**② `value_ultrashort` 从 40% 崩到 10% —— 本次最大的单个发现。**

n=10 里 4 单原本记 hit，扣成本后只有 **1 单**净赚。原因是其中 3 单毛收益落在
(0, 0.25%]：`hit_target` 触发记了命中，但期末净收益为负。
**超短持有期对成本的敏感度是短线的 5 倍以上**（T+2 vs T+5，成本占比 ∝ 换手频率），
这与成本理论的预期一致，但此前完全没有被度量到。

⇒ 结论修正：`value_ultrashort` 不是「40% 胜率的弱格」，而是**接近抛硬币以下的负期望格**
（10% 净胜率，−1.14% 净均值）。它在旧口径下被严重高估。

**③ `capital_short` 出现 `net_win(50.0) > new_strict(47.5)` —— 两种口径的分叉是真实的。**

差 1 单：该单**触及止损**（`hit_stop_loss=1` ⇒ 判 miss），但 T+N 收盘价回升，
期末净赚 > 0.25%。

这不是 bug，而是两个问题的答案本来就不同：

- `new_strict` 回答「按这套规则交易，该不该止损离场」→ 该单算输；
- `net_win` 回答「期末结账赚没赚」→ 该单算赚。

### 8.4 明细取证：判定与收益用了**两套不同的交易假设**

这是本轮最重要的结构性发现。抽两条极端样本：

| 股票 | 毛收益 | 净收益 | 止损 | 目标 | 旧判 | 新判 | 问题 |
|---|---:|---:|---:|---:|---|---|---|
| 300274（value_ultrashort） | **−0.024%** | −0.274% | 0 | **1** | hit | **hit** | 期末亏钱，因盘中触及目标价仍记命中 |
| 002851（capital_short） | **+16.862%** | +16.612% | **1** | 1 | **miss** | **miss** | 全程大牛，因中途破位被判失败 |

规则 `hit_target == 1 || ret > 0.0` 与 `hit_stop_loss == 1 → miss` 隐含了
「**触及目标就卖、触及止损就卖**」的交易假设；但 `final_return_pct` 用的是
**T+N 收盘价**，隐含「**一路持有不看盘**」。

⇒ **判定按 A 模型，收益按 B 模型，两者从未对齐。**

- 方向一（乐观）：300274 盘中摸到目标价 → 记命中，但期末是亏的；
- 方向二（悲观）：002851 中途破位 → 记失败，但期末 +16.9% 是它贡献的。

两方向同时存在，**净效应不可加总** —— 所以不能靠"有对有错正好抵消"来自我安慰。

### 8.5 对照实验：若真按规则执行止损

把 `hit_stop_loss=1` 的样本收益改为**止损价成交**（`(stop_loss − entry) / entry`），
其余仍用 T+N 收盘价：

| 格 | n | 触及止损 | 持有到 T+N | 止损执行 | 止损执行净额 |
|---|---:|---:|---:|---:|---:|
| capital_short | 40 | 15 | +3.41 | +3.32 | **+3.07** |
| value_short | 29 | 4 | +0.69 | +0.44 | **+0.19** |
| trend_short | 12 | 2 | −1.68 | −1.71 | −1.96 |
| capital_ultrashort | 30 | 19 | −0.89 | **−0.64** | −0.89 |
| value_ultrashort | 10 | 3 | −0.89 | **−1.07** | −1.32 |
| trend_ultrashort | 6 | 2 | −1.57 | −1.26 | −1.51 |
| **合计** | **127** | **45** | **+0.72** | **+0.69** | **+0.44** |

三个结论：

1. **合计几乎不变（+0.72 → +0.69）** —— 两个方向基本抵消。所以「止损有害/有益」在这批
   样本上**都给不出结论**，别拿它当论据。
2. **`capital_ultrashort` 改善 0.25pp**（−0.89 → −0.64）：19/30 触及止损，止损确实截住了
   尾部大亏。
3. **`value_ultrashort` 反而恶化 0.18pp**（−0.89 → −1.07）：典型是 `000582`
   期末仅 −0.089%，但期间触及止损 ⇒ 按止损价成交变成 −8% 级亏损。
   **止损不是免费的，它把「破位后回升」的单变成真亏。**

⇒ 这条直接决定一件事：**光把止损写进规则没用，得同时决定「止损后是否允许再入场」**。
当前样本里的 45 单止损，有多少是假突破、有多少是真跌，没算过 —— 登记为 N-9。

### 8.6 与盈利性裁决的交叉验证

`AUDIT-stock-profitability-verdict-2026-10-25.md` 的核心结论是「无证据表明能赚」。
本次实测**把它进一步收紧**：

- 全样本净胜率 **40.9%**（不是 42.5%）—— 距 50% 已有 9 个百分点；
- 唯一强正格 `capital_short`：净胜率 50.0%、净均值 **+3.07 ~ +3.16%**，n=40，其中
  **15/40 触及止损**。标准误 ≈ 7.9% ⇒ **仍与抛硬币不可区分**；
- 6 个格里 4 个净均值为负，且扣成本后全部更差；
- 收益高度依赖尾部：`002851` 一单 +16.9%，占 `capital_short` 总收益的 12%。

⇒ 结论不变，但**「不能赚」的证据变强了**，而不是变弱。

---

## 9. 新增登记（本轮发现）

| # | 事项 | 为什么不本轮做 |
|---|---|---|
| N-7 | 触及止损的样本，收益应改为**止损价成交**而非 T+N 收盘价 | 改的是收益计算模型，不是口径；且须同时定「止损后是否可再入场」（N-9）。§8.5 已给出对照数字，但**未并入生产** |
| N-8 | `net_win` 与 `new_strict` 在「止损后回本」样本上分叉 | **不是缺陷**，两者回答不同问题（§8.3③）。但面板文案须区分，否则会被当成矛盾数据 |
| N-9 | 45 单止损里「假突破回升」与「真跌」各占多少 | 需要止损后 N 日的 K 线回放（是否创新高），本轮只做了收益替换，没做路径分析 |
| N-10 | **判定模型与收益模型不一致**（触及目标/止损的判定 vs T+N 收盘价收益） | 这是本轮最重要的结构发现（§8.4）。**已裁决 → 采用方案②「触发价成交」**，见 `PLAN-validation-fill-model.md`；该文含 127 单双模型实测对比（合计净 +0.47% → **−0.31%**）、3 条配套规则与实施清单。**代码未改** |
