# PLAN — market-sim 在股票业务中的价值定位与兑现路径

> 状态：active ｜ 日期：2026-09-17 ｜ 对应版本：v2.9.7


> 结论先行：**这套仿真是「微观结构实验室」，不是「预测机器」。**
> 它唯一不可替代的能力是**订单簿级的成交成本与压力韧性**。
> 把它用于方向预测或决策硬门，产出的是伪精确；用于执行成本与压力测试，产出的是别处拿不到的信息。

## 零、先分清四条「模拟」线：只有一条该接进工作流

「把模拟仿真接入股票分析工作流」这个要求本身太粗 —— 项目里有**四条互相独立的模拟线**，它们与工作流的关系完全不同，混为一谈会得出相反结论。

| # | 线 | 实现位置 | 与工作流的关系 | 结论 |
|---|---|---|---|---|
| 1 | **K 线统计代理** | `hooks.rs:349-450` → `sim_stability` / `sim_liquidity` / `sim_impact` | **已在链内**，由组合风控门 S-501~503 消费 | 不是仿真，是**命名问题** ⇒ 改名 + 修注释，不新增能力 |
| 2 | **DES 订单簿仿真** | `crates/market-sim`（多 Agent 撮合 + 订单簿 + Oracle 冲击） | **零交集**，入口仅回测页手动面板 | 应接，但形态是**离线快照 → 风控门只读**，不是实时跑 |
| 3 | **蒙特卡洛** | 3a `run_monte_carlo` tool（`tools/src/tools/finance.rs:807`）｜3b `market_sim_run_mc`（MonteCarloPanel） | 3a 挂在「激进评估」agent 的工具表，**由 LLM 自主决定调不调** | 3a 需**确定性化**（改成 CodeNode 才能稳定生效）；3b 需落库 |
| 4 | **纸交易组合** | `paper_portfolios` / `paper_positions`（`v204_paper_portfolio.rs`）+ 9 个命令 + `PaperPortfolioPage` | **零交集**：表已落库、UI 已有、命令已有，但**工作流从不写入** | **本条最优先** —— 它是唯一能让决策链闭环的机制 |

### 为什么第 4 条最优先，而它并不是「仿真」

工作流当前真正的缺口**不是「决策时少了一个仿真数字」，而是「决策做完之后没有任何反馈」**。

- 决策落库到 `stock_analyses` 之后，链路就断了；
- `core.rs:1145-1250` 已经建了「决策 → 可执行触发器」这条边（自动写 `price_alerts` 表 + 注册 `RealtimeMonitor`），但**告警触发只是通知，不回流成评估**；
- 于是评分阈值、`support_rate`、仓位公式全都**无法被证据校准**，只能靠手感 —— 这正是「`support_rate` 修严后阈值需产品标定」一直悬着的根因。

`paper_positions` 的字段（`symbol` / `entry_price` / `entry_date` / `quantity` / `exit_price` / `exit_date` / `status`）**恰好足够承接「按决策虚拟建仓 → 跟踪后市 → 得出命中率」**。

### 接入成本：与已有先例完全同构

`core.rs:1145` 那段 price_alerts 写入就是**现成挂载点**。在同一位置追加一次虚拟建仓，是同一形态的改动：

| 需要的字段 | 工作流里现成来源 |
|---|---|
| `symbol` | `stock_code` |
| `entry_price` | 决策时的 `current_price` |
| `entry_date` | 当日 |
| `quantity` | `position_pct` × 可用资金 |
| `note` | 决策 JSON 的 `action` / `confidence` / 关键理由 |
| `portfolio_id` | **唯一缺口** ⇒ lazy create 一个固定名组合（如「工作流决策跟踪」），幂等 |

写入判据应与 `price_alerts` 一致（**只对含方向性动作的决策建仓**，观望 / 跳过不建），复用 `decision_action::normalize_action`。落库入口是 `analysis-engine::paper_portfolio::add_position`（`AddPositionInput`：`paper_portfolio.rs:64-76`）。

> 表已有（v204）、命令已有（`paper_portfolio_add_position`）、UI 已有 ⇒ **这条是四条线里成本最低、收益周期最短的一条。**

### 顺序结论

```text
第 4 条（闭环写入）    ← 先做：表 / 命令 / UI 全在，只缺一个写入调用
        ↓ 有了跟踪结果之后
第 1 条（改名 + 注释）  ← 顺手做：纯事实修正，零风险
        ↓
第 3 条（MC 确定性化）  ← 需先查真实运行报告有无 MC 数据，避免造无人消费的节点
        ↓
第 2 条（DES 离线快照） ← 最后做：前置改造大（单股 → 组合），且策略基线未跑正
```

**所以对「要不要接入」的直接回答：要接，但先接第 4 条（决策闭环），不是先把 DES 塞进决策链。**
把仿真内核直接接进硬裁决，等于用一个未校准、被 `ref×1.02` 锚定、基线 `Sharpe=-7.14` 的模型去否决真实决策 —— 那是负资产。

## 一、环节定位：唯一正确的位置是「组合风控门」

股票分析链共 18 个节点，但**只有 1 个是组合级的**，其余 17 个都是单股分析。仿真该放进这唯一的一个。

```text
单股分析链（17 节点）                     组合级（1 节点）
──────────────────────────                ────────────────────────────────
数据 → 分析师分组 → 多空辩论 → 风险评估    投资组合经理 → 【组合风控门】 → 尾链
                    ↓                     portfolio-mgr   portfolio-risk-gate
            全部围绕「这一只股票」                            ↑
                                                        仿真应在此
```

### 为什么是这里，而不是别处

| 判据 | 事实（可复核） |
|---|---|
| 它是唯一**看得到整个组合**的环节 | 职责含「行业暴露上限」「持仓数量上限」「**组合总仓位归一化**」——`portfolio-risk-gate.rhai:5-10` |
| 它的**输入已经在手边**，不必新建数据管道 | `holdings_json`（真实持仓，hook 注入 `hooks.rs:461-484`）+ `portfolio_cash`，两者已在 `pm_portfolio_risk_gate(...)` 签名中（`rhai_pm.rs:151-169`） |
| ⚠️ **此条已修正**：消费「假仿真量」的**不是本节点** | `sim_stability`/`sim_liquidity`/`sim_impact` 的消费方是 **`portfolio-mgr.rhai:1812-1868`**（决策产出节点），**不在本节点** —— `portfolio-risk-gate.rhai` 全文零 `sim` 匹配。详见第八节 |
| 它是决策链的**最后一站** | 下游只剩 rule-check / 解释官 / 落库 ⇒ 放在这里影响的是**最终仓位**，而不是中间过程 |

### 关键：形态是「离线预计算 + 只读快照」，不是「实时跑仿真」

风控门是 `execute_directly: true` 的**确定性** CodeNode。在其中实时跑 DES（秒级 + 随机种子）会同时破坏两件事：工作流的可复现性，以及 300-400 次 API 调用下本已紧张的延迟预算。

```text
【离线】收盘后 / 手动触发 ──→ 组合级压力测试（全部持仓进同一个模拟市场）
                                    ↓
                              落库：组合压力快照
                                    ↓
【在线】工作流 → 组合风控门 ──→ 读最近一次快照（读库，毫秒级、确定性）
                                    ↓
                              作为提示变量进 reasoning，不进硬裁决
```

### 前置改造：先补「我这一单」，再谈「单股 → 组合」

> ⚠️ **本节经同日两次修正**。原表述把「单股 → 组合」列为第一前置（过宽）；随后新增的「A 下单量 / 资金规模」**也已被撤回** —— 属同一类错误：**把「已被计算但未命名成字段」的值误判为「缺失」**。**真正的前置只有 B（内核缺口）与 C（接口维度）。**

| 序 | 前置 | 缺什么 | 证据 |
|---|---|---|---|
| ~~A~~ | ~~下单量 / 资金规模~~ | ⚠️ **撤回，此条不成立** —— 见下方「A 撤回说明」 | — |
| **B** | **DES 订单注入入口** | `build_default_agents`（`market_sim.rs:122-187`）只造交易所 / 做市商 / 动量 / 价值 / 噪声五类 Agent，**没有任何「我的订单」Agent**，各自下单量硬编码（500 / 200 / 300 / 50 股）⇒ **它模拟的是一个没有你的市场** | `market_sim.rs:122-187` |
| **C** | 单股 → 组合 | `SimConfig.stock_code`（`config.rs:20`）是单个 `String`；`McSimRequest` 也只吃单股 `stock_code` + `reference_price` | `config.rs:20` / `market_sim.rs:248-257` |

#### ⚠️ A 撤回说明（2026-09-14 同日修正）

**原判断「缺少下单量 / 资金规模」是错的。** 本笔下单金额**代码里已经在算**：

```rust
// portfolio_formula.rs:642
let portfolio_total_value = total_mv + portfolio_cash;               // 分母 = 持仓市值 + 现金
// portfolio_formula.rs:687
let new_position_value = final_pct / 100.0 * portfolio_total_value;  // ← 本笔下单金额
```

- 股数 = `new_position_value / current_price` ⇒ **用现有变量即可算出，无需新建任何字段**。
- `portfolio_holdings` 表 + `holdings_json`（已在风控门手边）**就是**这个分母的数据来源 —— 所以「持仓信息就够了」是对的。

**我错在哪**：对 `total_asset` / `available_cash` 等**字段名**做 grep 得零匹配后，直接判定「缺失」—— 但该值是以**另一个名字**（`portfolio_total_value`）**被算出来**的，只是没被命名成字段。

**真正的问题降级为「精度」，不是「缺失」**：

| 事实 | 位置 | 影响 |
|---|---|---|
| 分母 = `total_mv + portfolio_cash` | `portfolio_formula.rs:642` | 现金项为 0 ⇒ 分母 ≡ 持仓市值 |
| `portfolio_cash` 写死 `0.0` | `hooks.rs:488-492` | **隐含假设「满仓、零现金」** |
| 持仓市值用 `avg_cost` 估而非现价 | `hooks.rs:464`（`mv = shares * avg_cost`） | 市值本身是成本法估值 |

⇒ 若账户**满仓**，两个假设都成立，A 完全不存在；若**有现金**，`position_pct` 会系统性偏大（分母少了现金部分）。这是**标定问题，不是能力缺口**。

**真正要改的是「输出」而非「输入」**：`new_position_value`（`:687`）只被用于 `check_new_position_with_risk` 校验，**函数内用完即弃** —— `PortfolioRiskGateOutput`（`portfolio_formula.rs:510-520`）的字段只有 action / position_pct / risk_level / adjusted / original_action / reasons / concentrationWarning / topHoldingPct / maxSectorPct，**没有本笔金额**。要接仿真，只需给它加一个输出字段（如 `newPositionValue`）。

**B 比 A 更硬**：即使拿到下单量，当前内核也没有地方把它注入订单簿 —— `SimRunResult`（`market_sim.rs:101-118`）只有市场级统计（`total_trades` / `max_queue_depth` / `final_mid_price`），**没有任何「我的成交」字段**（无 `my_fill_price` / `slippage_bps` / `participation_rate`）。

⇒ **修正结论：「执行成本」价值位不是「接线问题」，是内核能力缺口。** 换掉 `hooks.rs` 的 K 线代理也拿不到冲击成本 —— 必须先给内核加 order 注入。

### 关键澄清：「单股」是工作流的**触发维度**，不是**数据维度**

「单股工作流接不了组合级仿真」是个**伪矛盾**。工作流里已经存在一个反例节点：

> **组合风控门**（`portfolio-risk-gate.rhai`）由**单股分析触发**（本次分析某一只），却读取 `holdings_json` —— **看得到全部持仓**。

所以「接入一个吃组合输入的仿真」**不要求工作流变成组合级**。触发是单股、数据是组合，两者本就分离。

| 层 | 触发 | 输入 | 输出 | 要改工作流结构吗 |
|---|---|---|---|---|
| **L1 单股执行成本** | 单股（不变） | 这一只 + 本笔下单量（`new_position_value` ÷ 现价 ⇒ 股数） | 冲击 bps / 参与率 | 不改；**本笔金额已算出，只需透出**（依赖 **B**） |
| **L2 组合流动性压力** | 单股触发 | `holdings_json`（已在风控门手边）+ 本笔 | 建仓后组合压力路径 | 不改，接在风控门旁（依赖 **B + C**） |
| **L3 纸交易闭环** | 单股触发 | 决策方向 / 价格 / 仓位 | 后市命中率 | 不改，复用 `core.rs:1145` 挂载点（**不依赖 B / C**） |

**为什么 L3 不受阻断**：纸交易的 `quantity` 只是**盈亏缩放因子**，取任意基数都不影响命中率（`paper_positions.quantity` 非空仅是建表约束）。而冲击成本对下单量是**非线性**的 ⇒ 仿真必须真量（= `new_position_value / current_price`），纸交易不必。

### 单股 → 组合（C）：为什么仍要做，但排第三

**这恰恰是仿真对「瓶颈掘金」策略最大的失效点**：持仓集中在小盘股，真实风险是**闪崩时一起跌**（相关性冲击）。单股仿真把每只当独立标的跑，永远看不到这个；只有把多个持仓放进**同一个**模拟市场（共享订单簿、共享 Oracle 冲击、Agent 之间互相传递压力），才能得出组合层面的结论。

> **附带发现（独立于仿真的缺陷）**：`portfolio_cash` 在 `hooks.rs:488-490` 是**写死注入 `0.0`** 的（注释自述「暂时注入 0.0，后续可扩展为从账户设置读取」）。组合归一化若以「持仓市值 + 现金」为分母，在真实账户上会系统性算错。登记待修。

## 二、仿真独有的三个价值位

判断标准：**这件事别的模块能不能做**。只有「不能」的才值得投入。

| 价值位 | 为什么只有仿真能做 | 当前兑现状态 |
|---|---|---|
| **① 执行成本（冲击 / 滑点 / 深度）** | K 线只能给你「波动率」，给不出「你这一单打到订单簿上会吃几档」。DES 撮合**理论上**能算冲击 bps 与参与率 | ❌ **不可达（内核缺口，非接线问题）**：不只是 `hooks.rs` 的 `σ×折价+5` 代理问题 —— `build_default_agents` 里根本没有「我的单」，`SimRunResult` 也无「我的成交」字段（见上一节前置 B） |
| **② 情景韧性（闪崩/高波动下仓位会怎样）** | `portfolio_risk.rs` / `position_limits.rs` 是静态统计（波动率、相关性），算不出「冲击 → Agent 连锁反应 → 价格路径」 | ⚠️ **半兑现**：`RobustnessReport` 结构完整，但只在手动 tab、默认不跑压力场景、不落库、无消费方 |
| **③ 策略独立验证（WF 历史 vs DES 前向）** | 历史回测与 Agent-Based 前向模拟是两条**方法论独立**的通道，一致才可信。这是防过拟合的标准手段 | ⚠️ **入口可达，但基线未成立**：`wf_des_integration` 挂在 **QuantLab → `WfDesTab`**（`components/quant/QuantLab.tsx:56`，路由 `/quant-lab`）—— 原判「前端已移除」**有误**，见第八节。且既有唯一基线 `MaCross 5/20: Sharpe=-7.14`，策略层自己没跑正 |

**对你「瓶颈掘金」策略的直接意义**：pre-takeoff 小盘股的两个真实风险恰好落在 ① 和 ②——
- 流动性差 ⇒ 建仓冲击成本可能吃掉大半预期收益（**①**，决定「这仓位值不值得建」）
- 跌起来无承接 ⇒ 闪崩下的路径依赖与流动性枯竭（**②**，决定「最坏能坏到哪」）

这两个恰好是仿真**能算**、K 线与静态风控**算不出**的。

## 三、四项必须知情的事实缺陷

### 2.1 `survival_rate` 不是「存活率」，是「上涨场景占比」

`crates/market-sim/src/monte_carlo.rs:233-238`：

```rust
let positive = valid_changes.iter().filter(|&&c| c > 0.0).count();
positive as f64 / valid_changes.len() as f64
```

`valid_changes` 是各场景的 `price_change_pct`。所以这个值 = **模拟终价高于参考价的场景数占比**。

**结论：它的数值主要由「你勾了哪些场景」决定，不由这只股票决定。** 默认勾选 normal/bull/bear（`MonteCarloPanel.tsx:28-30`），涨跌在场景定义时就已经定好方向了。

前端标签却是「跨场景存活率」（`MonteCarloPanel.tsx:205`），极易被读成「不亏钱的概率」。**文案与语义不符。**

### 2.2 `consistency_score` 在均值趋零时误报「最一致」

`crates/market-sim/src/monte_carlo.rs:240-252`：

```rust
if mean.abs() > 0.001 { stddev / mean.abs() } else { 0.0 }
```

变异系数在 `mean → 0` 时数学上无定义（分母趋零 ⇒ 应趋于无穷大或不可判定）。代码用 `0.0` 兜底，而 `0.0` 恰好落在前端 `consistencyScore < 0.5` 的**「一致性好」**分支（`MonteCarloPanel.tsx:313-317`）。

**方向搞反了**：各场景涨跌幅均值互相抵消（即最分歧、最无法定论的情形）被判为「高度一致」。

> 这是典型的**恒等边界缺陷**：兜底值恰好等于最优取值。修正方向是让趋零分支返回「不可判定」而非 `0.0`（例如 `f64::INFINITY` + 前端新增"无法判定"分支）。

### 2.3 `BEST_PARAMS`（校准结果）未接到实际命令上

`calibration.rs:16-17` 声称：

> 这些参数已导出为 `BEST_PARAMS` 常量，被 `MarketSimulationTool` 和 `QuantStrategyAgent` 默认使用。

**grep 事实**：`BEST_PARAMS` 全项目仅 3 处引用——`wf_des.rs:260-272`、`market-sim/tests/dag_integration.rs`、`lib.rs:55` 的 re-export。**没有任何 `MarketSimulationTool` 消费它。**

而三个对用户暴露的命令全部使用硬编码字面量：

| 命令 | 做市商价差 | BEST_PARAMS 值 |
|---|---|---|
| `market_sim_run`（MarketSimPanel） | `30` bps（`market_sim.rs:178`） | 35 |
| `market_sim_run_mc`（MonteCarloPanel） | `50` bps（`market_sim.rs:304`） | 35 |
| `market_sim_run_strategy`（QuantSimPanel） | `50` bps（`market_sim.rs:427`） | 35 |
| `wf_des_integration`（无 UI） | **35** ✅ | 35 |

**用户在面板上看到的仿真结果，用的是未校准参数。** 校准工作做了（100 组拉丁超立方 × 5s/路径），但只在一个没有 UI 的命令上生效。

### 2.4 Agent 布局无个股适配，价格中枢被人为锚定

三个命令的 Agent 构造只吃 `reference_price` 一个变量，其余全是常数。最要命的是价值 Agent 的公允价值：

```rust
// market_sim.rs:164（run）与 :316（mc）
ValueAgent::new(..., (reference_price as f64 * 1.02) as i64, ...)
//                ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ 公允价 = 参考价 × 1.02，写死
```

即：仿真世界里「价值投资者」恒定认为标的**值参考价 +2%**，于是价格被持续拉向 +2%。

**因此 `price_change_pct` 的方向性结论是这个人造常数与 Oracle 场景设定的回响，不含市场发现过程。** 不同个股跑同一场景，差异主要来自噪声种子，不来自个股特性。

## 四、会让它变成负资产的四种用法（不建议）

| 用法 | 为什么不行 |
|---|---|
| 用 `price_change_pct` / `best_scenario` 做**方向预测** | 中枢被 `ref×1.02` 锚定 + 场景方向在定义时已定，不构成发现（见 2.4） |
| 把 `survival_rate` 接进 `final_action` / `position_pct` **硬裁决** | ① 它不由个股决定（见 2.1）② 参数未校准、未个股适配（见 2.3/2.4）③ 既有策略基线 `Sharpe=-7.14`，策略层自己没跑正 |
| 用仿真结果做**精确点位** | 波动由种子驱动，路径级精度无意义；宏观正确、微观随机是 ABIDES 类模型的固有属性 |
| 用它替代 K 线做**技术形态判断** | 形态判据需要真实历史序列的连续性，仿真序列不具备该连续性 |

> 注意：既有 `output/market_sim_integration_plan.md` 的 Phase 3 方案（注入 `f12` 模拟因子影响 posterior）**已不宜执行**：
> ① `f12` 槽位已被**动量因子**占用（`portfolio-mgr.rhai:1015-1048`，`get_weight(f_weights, "momentum", ...)`），会撞号；
> ② 该方案让仿真以 5% 权重进入 posterior 融合，属于上述「硬裁决」范畴。该文件位于 `output/`（非文档目录）且 Phase 2 已完成但状态未更新，已可作为历史记录归档。

## 五、兑现路径（按性价比排序）

### P0 — 低成本、立刻让现有资产产生价值

| # | 动作 | 位置 | 收益 |
|---|---|---|---|
| 1 | 修正趋零兜底：`else { 0.0 }` → 返回不可判定，前端加「无法判定」分支 | `monte_carlo.rs:245-249` + `MonteCarloPanel.tsx:313-317` | 消除**方向相反**的误报（2.2），这是当前唯一的逻辑错误 |
| 2 | 默认开启 `flash_crash` / `high_vol` 两场景 | `MonteCarloPanel.tsx:31-32`（`enabled: false` → `true`） | 让面板默认就回答「最坏会怎样」，压力场景才是这个面板的意义所在 |
| 3 | 修正 `calibration.rs:16-17` 注释，或**真的**把三个命令接到 `BEST_PARAMS` | `calibration.rs` / `market_sim.rs` | 注释撒谎是唯一规格错误（2.3）；接上则是让校准工作第一次生效 |

### P1 — 让仿真进入决策链（但只作提示，不作裁决）

| # | 动作 | 说明 |
|---|---|---|
| 4 | `RobustnessReport` 落库 | 现在是纯内存返回，无历史 ⇒ 无法建立触发率/有效性基线 |
| 5 | 在工作流「组合风控门」**之前**注入仿真摘要作为**提示变量**给分析师节点 | 只进 LLM 上下文，**不进** `final_action` / `position_pct`；与现有 `sim_stability` 代理并列，标注数据来源区分 |
| 6 | 恢复 `wf_des_integration` 的入口（哪怕只是调试页） | 这是唯一能验证仿真器可信度的通道；先跑通一个**简单策略**建立可信基线，再谈接入 |

### P2 — 让仿真真正「属于这只股票」（前置依赖，成本高）

| # | 动作 | 说明 |
|---|---|---|
| 7 | 用个股历史数据反标定 Agent 参数 | 至少让 `mm_spread_bps` / `noise_*` / 价值 Agent 公允价由个股波动率与成交额推导，消除 `ref×1.02` 的人造锚 |
| 8 | 用真订单簿仿真替换 `hooks.rs` 的 `sim_impact` 代理 | 这是价值位 ① 的兑现。当前代理为 `σ×折价+5`，`hooks.rs:406-407` 自己承认「绝对水平仍属定性标定」 |

> **顺序依赖**：7 是 8 与 5 的前提。在参数未个股适配前，把仿真接入决策链等于把「标准 A 股」的结论套到具体标的上。

## 六、一句话总结

仿真在当前项目的正确用法是：**回答「这仓位建得起吗、最坏能坏到哪」，而不是「它会涨还是会跌」。**
前者它能给出别处拿不到的数字，后者它只会把人为设定回响给你。

## 七、实施记录（2026-09-14）

本轮实施范围：**P0 三项 + 前端位置迁移与人性化**。P1 / P2 未动。

### 已实施

| # | 项 | 落地位置 | 说明 |
|---|---|---|---|
| 1 | **趋零兜底方向修正** | `market-sim/src/monte_carlo.rs` | `consistency_score: f64` → `Option<f64>`；抽出纯函数 `compute_consistency_score(&[f64]) -> Option<f64>`，均值趋零返回 `None`（原先返回 `0.0`，被下游读成「一致性好」= 方向相反）。补 3 个边界测试钉住：涨跌互抵 ⇒ `None`、场景不足 2 ⇒ `None`、可比情形 = 变异系数且随离散度单调变差 |
| 2 | **压力场景默认开启** | `MonteCarloPanel.tsx` | `flash_crash` / `high_vol` 由 `enabled: false` → `true`，并把默认场景提为模块级常量并注释理由 |
| 3 | **`BEST_PARAMS` 真正接入** | `commands/market_sim.rs` | 三个前端可见命令全部改读校准值：做市商价差 30/50/50 → **35**、挂单量 500 → **634**、噪声激活率 0.3 → **0.27**、噪声幅度 30 → **32**、动量阈值 0.003 → **0.0035**。同时修正 `calibration.rs` 中「被 `MarketSimulationTool` 和 `QuantStrategyAgent` 使用」的失实描述（前者类型不存在、后者未引用） |
| 4 | **前端位置迁移** | `StockAnalysisPage.tsx` / `BacktestPage.tsx` / 新增 `SimulationTabContent.tsx` | 「模拟仿真」由**回测页**（业务层级）迁入**股票分析页**；回测页移除 `simulation` / `mc` / `quant_sim` 三个 tab。迁移理由：仿真属单股分析动作，不属回测验证。<br>⚠ **位置已定稿（当日三次调整）**：迁入时插在 `risk` 之后（仅按「归类」定位，**未判定它在叙事链中的位置**）；同日复核后**先移至决策之后，再由用户裁定落到「仪表盘之后」**。当前序列：`… → value → risk → decision → dashboard → simulation → reflection → evolution`。<br>三处**相互独立**的依据都说「仿真在决策之后」：① `commands/sim-verify.rhai:1`「图上位于决策之后」② `sim_hook.rs::spawn_simulation_after_decision`（调用点 `decision.rs` 决策落库后、`core.rs` 主链同点）③ `simulation.autoTitle`「工作流自动仿真（决策之后）」。<br>落在**仪表盘之后**（而非紧跟决策）的理由：仿真验证是对**整份分析结论**的独立复核，叙事为「看完汇总 → 再做独立验证」，且不打断「决策 → 仪表盘」这条最紧的相邻关系（仪表盘汇总的正是决策产出）。<br>**本行旧文本「插在 `risk` 之后」已属腐烂声明，勿再据此推理** |
| 5 | **前端人性化** | 三个面板 + `SimulationTabContent.tsx` | ① 标的与现价**自动带入**（不再手填代码）② 顶部固定「当前分析标的」条与用途边界说明 ③ 场景预设（快速体检 / 压力测试 / 深度分析）④ 参考价「分」旁回显「元」⑤ 一致性 `null` 渲染为「无法判定」+ 解释 ⑥ 股票代码由 `InputNumber` 改文本输入（原会把 `000001` 读成 `1`）⑦ 涨跌配色改为 A 股约定（红涨绿跌） |

### 未实施（按原优先级保留）

- **P1**：`RobustnessReport` 落库、仿真摘要作提示变量注入风控门、恢复 `wf_des_integration` 入口 —— 均未动。
- **P2**：个股参数反标定（消除 `ref × 1.02` 锚）、真订单簿替换 `hooks.rs` 冲击代理 —— 均未动，仍为价值位 ① 的前置。
- **前置 B**（DES 无订单注入入口）仍未解 —— 因此当前仿真结果**依旧不含「你这一单」**，这一点已写进 UI 文案，避免误读。

### 🔍 顺带发现（**已登记，未改**）：`stockAnalysisPageSchema` 是不跑的第二份 tab 定义

复核 tab 顺序时顺带查到 —— `src/lib/stock-schemas/stock-analysis-page-schema.ts` 自称「StockAnalysisPage 的
**DynamicUI Schema 等价定义**」，其中**独立硬编码了另一套 tab 列表**
（`market/analysts/debate/value/risk/decision/reflection/evolution`），与真实现
（`components/stock-analysis/StockAnalysisPage.tsx::tabs`）已**三处漂移**：

1. **缺 `simulation`** —— 本次迁入的仿真 tab 它完全没有；
2. **缺 `dashboard`**；
3. **`decision` 是已不存在的旧 key** —— 真实现里是 `decision-comparison`，旧 `decision` tab 已改为**顶部全宽 hero**
   （见 `src/components/stock-analysis/StockAnalysisPage.tsx:369` 注释「Decision tab removed — now rendered as full-width hero at top」）。

**它是否在跑**：唯一消费者是 `lib/seed-stock-schemas.ts:13,36`，而 `seedStockSchemas()` 全仓**零调用点**
⇒ 整条链是死的（判据 **I**：N 份实现只 1 份在跑）。

**本轮未改它**的理由：① 它不跑，改它对界面零影响；② 删改死链属架构决策（判据 A「零调用可删吗」），
超出本域收益面。

**风险（留给下轮）**：将来若有人把 `seedStockSchemas()` 接起来（例如做 DynamicUI 迁移），会得到一份
**没有仿真 tab、且带死 key** 的页面；而「等价定义」这个名字会让人**先信它、后查真实现**，排查成本高。
建议接线之前先删掉，或改成从 `tabs` 单一来源派生。

### 本轮改动文件

```text
Rust
  src-tauri/crates/market-sim/src/monte_carlo.rs
  src-tauri/crates/market-sim/src/calibration.rs
  src-tauri/src/commands/market_sim.rs

前端
  src/components/stock-analysis/SimulationTabContent.tsx   (新增)
  src/components/stock-analysis/StockAnalysisPage.tsx
  src/components/stock-analysis/BacktestPage.tsx
  src/components/stock-analysis/MarketSimPanel.tsx
  src/components/stock-analysis/MonteCarloPanel.tsx
  src/components/stock-analysis/QuantSimPanel.tsx
  src/types/market-sim.ts

i18n（11 语言）
  src/i18n/locales/*.json   — 新增 12 个 key + 修正 cross-scenario-survival-rate 语义
```

> 备份：`output/backup-2026-09-14/i18n-market-sim/`（11 个语言文件改动前快照）

## 八、事实修正与「仿真在工作流中的位置」复核（2026-09-14）

用户追问两件事：① 仿真在分析工作流中是否处于**决策之后**（不阻滞决策）② 是否**自动**接收上游信息运行而非手动触发。复核后发现我此前两处判断有误，先纠正。

### 8.1 两处事实修正

| # | 原表述 | 实际（可复核） |
|---|---|---|
| 1 | 「**组合风控门**（`portfolio-risk-gate`）已经在消费假的仿真量 S-501~503」 | ❌ 错。`portfolio-risk-gate.rhai` **全文零 `sim` 匹配**；S-501~503 实际在 **`portfolio-mgr.rhai:1812-1868`**。`sim_stability`/`sim_liquidity`/`sim_impact` 的 `input_mapping` 同样挂在 `portfolio-mgr` 节点（`seed_stock_analysis.rs:3152-3155`） |
| 2 | 「`wf_des_integration` **前端已移除**（仅剩 i18n 残 key）」 | ❌ 错。它挂在 **QuantLab → `WfDesTab`**（`components/quant/QuantLab.tsx:12,56`；`tabs/WfDesTab.tsx:187` 实际 invoke；路由 `/quant-lab`）。**入口可达**，只是基线未跑正 |

两处错因相同：**用「上次 grep 的印象」代替了复核**。第 1 条是在写第一节时把「组合级节点」与「消费该变量的节点」混为一谈；第 2 条是一次覆盖不全的 grep 就下了「已移除」的结论。

### 8.2 回答问题①：仿真在决策链中的位置

必须分两层 —— 项目里有**两个都叫「仿真」的东西**：

| 层 | 是什么 | 在决策链中的位置 | 会不会阻滞决策 |
|---|---|---|---|
| **A. market-sim 内核**（本次迁移的面板） | `crates/market-sim` 的 DES / MonteCarlo / 策略沙盒 | **不在链上** —— 完全在 DAG 之外，前端手动面板 | ❌ 不阻滞（但它也不产生回流：跑完不落库、工作流不读） |
| **B. `sim_*` 三指标**（K 线代理，名字像仿真） | `hooks.rs:349-450` 算，`portfolio-mgr` 消费 | **在决策链之内**，就在产出 `final_action` / `position_pct` 的那个节点 | ⚠️ **会改决策**：S-501 `< 0.15` 把 action **强改观望**、S-502 `< 0.2` 把仓位**砍半** |

⇒ **与「仿真应在决策之后、不阻滞决策」的期望正好相反**：真正进入决策的是 **B**（代理值，且会改写决策），而 **A**（真仿真）连决策链都没进。

### 8.3 回答问题②：是否自动运行

**不自动。** 三个面板全部需要用户点「运行」按钮。本次做的「自动」只到**表单预填** —— 把当前分析标的与现价填进输入框（省去手输代码），**运行仍是手动**。

已核实：`stock_workflow/` 与 `stock_analysis_setup/` 对 `market_sim|MarketSim|MonteCarlo` **零匹配**；无任何定时任务触发仿真。

### 8.4 该不该自动？建议

| 形态 | 建议 | 理由 |
|---|---|---|
| 决策链**内**实时跑 DES 并参与裁决 | ❌ 不要 | 秒级 + 随机种子会同时破坏工作流可复现性与延迟预算；且前置 B 未解 ⇒ 结果不含「你这一单」 |
| 决策**落库之后**自动跑一次并落库（离线快照） | ⚠️ 可做，但**排在 L3 之后** | 这是「决策之后」的正确形态。但前置 B 未解时它对决策无增量 ⇒ 优先级低于纸交易闭环 |
| 纸交易闭环（决策后自动建虚拟仓 → 跟踪 → 反标阈值） | ✅ **最该先做** | 它**不依赖仿真**、不占延迟预算，且正好补上「决策之后没有任何反馈」这个真正的缺口（第零节第 4 条） |

**一句话**：当前仿真既不自动、也不在决策之后；它与决策链的唯一接触点是 **B（代理值，且在决策之内改决策）**。要把「决策之后的反馈」补上，第一步不是自动化仿真，而是纸交易闭环。

---

## 九、仿真接入工作流：**落点乙**实施记录（2026-09-14）

### 9.1 需求基线（用户原话固化，本轮以此验收）

| 编号 | 用户原话 | 需求 | 状态 |
|---|---|---|---|
| R5 | 「应该在股票分析页内的 tab 而不是现在的股票业务层级」 | 前端展示在分析页 tab | ✅ |
| R6 | 「前端 UI 页应该更加人性化」 | 交互体验 | ✅ |
| R7 | 「处于决策之后（应该不阻滞决策信息）」 | 结果不参与裁决 | ✅ |
| R8 | 「自动接收上游信息运行而不是手动触发」 | 自动运行，无需手点 | ✅ |

### 9.2 落点选择：**乙**（执行在持久化之后，图上仍显示在决策之后）

用户否掉了甲（把 `sim-verify` 串进主链 `decision-explainer → sim-verify → store-result`），理由是**工作流执行时长**。两条约束同时满足：

| 约束 | 要求 | 实现 |
|---|---|---|
| **执行时机** | **持久化之后**，不占工作流执行时长 | `core.rs` 在 `workflow-completed` emit **之后** `tokio::spawn` 触发 |
| **图上位置** | 仍**显示在决策之后** | 模板保留 `sim-verify` 节点，坐标 `(700, 4800)`（对齐 `store-result` 的 `(300, 4800)`） |

### 9.3 两个非直觉的引擎约束（缺一会静默破坏调度）

要让节点「图上可见、但不参与执行」，必须**同时**满足两条 —— 任一单独用都会出问题：

| # | 只做一半的后果 | 依据 |
|---|---|---|
| 1 | **只把节点改成孤立（无入边）** ⇒ 被当成就绪节点**立即执行一次**（入度 0 即就绪） | `rt-workflow/…/engine/dag_store.rs:304-315`（`compute_ready_nodes`） |
| 2 | **只设 `enabled=false` 但保留出边** ⇒ 它永不进 `done_or_skipped`，下游 `store-result` **永久 Pending**（依赖按边计数） | 同文件 `:218-260`（`remaining_deps`） |

⇒ 必须 **`enabled=false` + 无任何边**。两条注释已写进 `seed_stock_analysis.rs` 节点定义处。

### 9.4 复用而非新建（单一实现源）

| 复用对象 | 位置 | 说明 |
|---|---|---|
| 仿真核心 | `market_sim::run_mc_core`（本次从命令函数中抽出） | 前端手动跑与工作流自动跑**共用**，避免两套实现随时间漂移 |
| 决策后触发挂钩 | `stock_workflow/sim_hook.rs`（**单一实现源**） | `core.rs`（完整分析）与 `rerun_decision`（重跑决策）**共用**同一函数，见 9.8 |
| Rhai 宿主函数注册 | `stock_workflow/rhai_pm.rs::register_pm_functions` | 单一权威源；仅在「有人手动打开该节点」时被用到（见 9.9） |
| 结果落点 | `blackboard_snapshot` 的 `sim-verify` 键 | 与 DAG 节点输出同构 ⇒ 前端 store 解析分支**无需区分来源** |
| 触发挂载点 | `core.rs`（与 `price_alerts` 自动创建同一区域） | 「决策落库后做点什么」已有先例 |

### 9.5 关键实现约束（均为已踩过的坑）

| 约束 | 原因 | 应对 |
|---|---|---|
| 仿真循环**必须在 Rust 宿主函数内** | 共享 Rhai 引擎 `set_max_operations(200_000)`；「场景×路径×事件」写进脚本必然超限 | 宿主函数对 Rhai 只计 1 次操作 |
| 宿主函数形参用 `rhai::Dynamic` | ① `input_mapping` 注入的数字被统一转成 f64（`code_executor.rs:133-143`）⇒ 形参写 `i64` 会 Function not found；② Rhai 1.25 对多 `Option<T>` 参数注册有缺陷 | 类型转换放函数体内 |
| CodeNode 有 **30s 硬超时** | `code_executor.rs:205` | 该节点不执行，但宿主函数仍受此约束（如未来改回甲方案） |
| `consistencyScore` 用 `Option<f64>` | 均值趋零时变异系数数学上无定义；用 `0.0` 兜底会被前端读成「高度一致」，**方向相反** | 全链路透传 `null`，UI 渲染「无法判定」 |

### 9.6 改动文件

| 文件 | 改动 |
|---|---|
| `crates/market-sim/src/monte_carlo.rs` | `consistency_score` → `Option<f64>`；抽 `compute_consistency_score` 纯函数；单测覆盖趋零场景 |
| `src/commands/market_sim.rs` | 抽 `run_mc_core`（单一实现源）；新增 `run_mc_preset`（Rhai 入口，按预设场景跑，返回 JSON 字符串） |
| `src/commands/stock_workflow/rhai_pm.rs` | 注册 `sim_run_mc` 宿主函数 |
| `src/commands/sim-verify.rhai` | **新建**：节点脚本（只读上游、不产出决策字段） |
| `src/commands/stock_analysis_setup/seed_stock_analysis.rs` | 加 `sim-verify` 节点（`enabled=false` + 无边）；`TEMPLATE_VERSION` 44 → **46**（45 为脚本入库那次，46 为节点注释/脚本变更后再次升版），再 → **47**（见 §9.14.2 强制重种子） |
| `src/commands/stock_workflow/sim_hook.rs` | **新建**：`spawn_simulation_after_decision` / `run_simulation_after_decision` / `snapshot_reference_price`（含 4 个单测）—— 决策后仿真挂钩的**单一实现源** |
| `src/commands/stock_workflow/mod.rs` | 声明 `pub mod sim_hook;` |
| `src/commands/stock_workflow/core.rs` | 捕获现价 → 决策落库后调用挂钩（原内联块已抽到 `sim_hook`） |
| `src/commands/stock_workflow/decision.rs` | `rerun_decision` 新增 `app: tauri::AppHandle` 形参；返回前调用挂钩（参考价传 `None`）—— 见 9.8 |
| `src/stores/feature/stockAnalysisStore.ts` | `SimulationSnapshot` 类型；`simulation` state（三处）+ 三处填充；监听 `simulation-ready`（含防串台）；`rerunDecision` 请求前清空 `simulation` |
| `src/components/stock-analysis/SimulationTabContent.tsx` | 新增「工作流自动仿真结果」区块（自动展示，无需手点） |
| `src/i18n/locales/*.json`（11 语言） | 新增 19 个 key（18 + `reason_cancelled`），校验 11 文件 key 集合一致 |

### 9.7 与第八节结论的差异说明

第八节 8.4 曾判断「决策落库后自动跑」应排在 L3（纸交易闭环）**之后**。**用户明确要求先做这一项**，故本次实施。

该判断中「**前置 B**（内核无 order 注入入口 ⇒ 结果不含「你这一单」）未解，对决策无增量」这一事实**依然成立** —— 因此本结果被明确定位为**风险视角的补充信息**：UI 已标注不含本笔交易冲击成本，且不写入任何决策字段。

### 9.8 同日追加：重跑一致性修复（`rerun_decision` 也会重跑仿真）

**缺陷**：`rerun_decision`（重跑决策）会改 `action` / `positionPct`，但**不重写 `blackboard_snapshot`**
⇒ 快照里的 `sim-verify` 仍是**上一版决策**的仿真结论。前端会把它与新决策并排展示，
形成「决策已变、补充信息未变」的**静默误导**（无任何时效标记）。

**判据来源（本项目已有先例）**：同一处代码在重跑时已清空 `decisionExplanation`，理由写的是
「旧说明书描述的是上一轮的裁决，重跑后继续展示会与新决策矛盾（同一次决策两个真相源）」
—— 仿真属于**同一类「决策后置产物」**，必须同等处理。

**修法**（两侧同时，缺一不可）：

| 侧 | 改动 | 理由 |
|---|---|---|
| 后端 | 抽出 `stock_workflow/sim_hook.rs`，`core.rs` 与 `rerun_decision` **共用**；`rerun_decision` 新增 `app: tauri::AppHandle` 形参后调用挂钩 | 两处各写一份必然漂移；`AppHandle` 形参与 `#[agent_command]` 共存有大量先例（`backup.rs:59`、`cognitive.rs:556`），前端 invoke **无需新增参数**（Tauri 自动注入） |
| 后端 | 写回改为**无条件覆盖**（失败也写 `simOk:false`） | 只在成功时写回 ⇒ 失败后旧结论继续冒充新结论。失败也必须落一个「不可用」标记 |
| 后端 | 参考价传 `None` ⇒ 回退快照 `t-scoring.result.content.currentPrice` | 该入口**没有新的行情输入**，它重算用的就是快照里那份数据；从快照取价才是同口径 |
| 前端 | `rerunDecision` 在**发请求之前** `simulation: null` | ⚠️ 不能在响应回来后清：后端在响应前后就会 emit `simulation-ready`，后清会把新结果一起抹掉（竞态） |

**「无条件覆盖」的另一个必要动作**：取不到参考价时**不跑仿真但照样写回** `missing_input`
—— 否则「跑不了」会表现成「旧结论还在」，与「跑失败」无法区分。

### 9.9 两条执行路径：`sim-verify.rhai` 在默认路径下不执行（**已知项，非缺陷**）

| | 路径甲（**默认，唯一实际跑的**） | 路径乙（仅手动启用该节点时） |
|---|---|---|
| 触发 | 决策落库后由后端挂钩触发 | DAG `code_executor` 执行节点脚本 |
| 实现 | `sim_hook::spawn_simulation_after_decision` → `run_mc_preset` | `sim-verify.rhai` → 宿主函数 `sim_run_mc` → **同一个** `run_mc_preset` |
| 位置 | `core.rs`（决策落库后）/ `rerun_decision` | 不在链上（节点 `enabled=false` 且无边） |

保留脚本与宿主函数是为了让节点在编辑器里**自带可读、可单独试跑**的定义。
**维护要求**：两条路径的**入参与产物形状**都必须一致 ——

| 项 | 两条路径的共同约定 |
|---|---|
| 场景预设 | `"stress"`（全部 5 场景）；`"base"` 仅 normal/bull/bear，供高频重跑 |
| 价格单位 | **元**（`run_mc_preset` 内部转「分」匹配 `McSimRequest.reference_price`） |
| 成功形状 | `McSimResult` 序列化体（含 `simOk:true` + `scenarioResults` 等） |
| 失败形状 | `ErrorResponse` 序列化体（`{simOk:false, code, category, detail?}`）。宿主函数（`run_mc_preset`）**自己**产出该形状；挂钩侧只补 `simOk:false`，**不做码值映射**。码值定义在 `error_code.rs::stock_sim`，前端由统一翻译层 `t("error.${code}")` 消费 |

> 失败形状在 **9.11** 做过一次**契约变更**：原先两条路径各自把失败归一化成自造的小写
> `reason` + 中文自由文本 `error`，那等于在 `check-errorcode-alignment.mjs` 管辖的
> 399 个后端码之外**另建一套无人管辖的错误码**。现统一为后端错误码。上表已反映新契约。

> 之所以没有把路径乙做成唯一实现：`code_executor::execute_rhai_directly` 是**私有** `async fn`，
> 只能经完整 `NodeExecutor::execute(ExecutionContext)` 进入；为一次后置计算伪造执行上下文
> 得不偿失，且会给这条旁路引入「引擎构造失败」这个原本不存在的新故障点。

### 9.10 同日再追加：`reason` 值域闭环 + 一处「提前 return」破坏自身契约

**缺陷 1（已修，纯 bug）**：`run_simulation_after_decision` 的原实现在
`spawn_blocking(...).await.ok()` 为 `None` 时**提前 `return`**，跳过了第 3、4 步
（写回快照 + emit）。而这与**同一函数**第 3 步注释写的契约直接矛盾：

> 「『无条件』是刻意的：若仅在成功时写回，`rerun_decision` 之后旧结论会留在快照里冒充新结论。」

即：**自称「无条件覆盖」，实际存在一条跳过覆盖的路径**。后果比「显示错误原因」更重 ——
前端在 `rerunDecision` 里已先清空 `simulation`，此后既收不到写回也收不到 emit，
界面就永久停在「分析完成后自动生成」，**失败与「还没跑」不可区分**（静默无变化）。

修法：三种异常一律**产出带具体 `reason` 的失败值并落入第 3 步**，不再提前 return；
同时把「任务被取消」与「任务 panic」分开 —— `.ok()` 会把两者都压成 `None`，用户拿到
一个查不到原因的失败：

| `spawn_blocking` 结果 | `reason` | 依据 |
|---|---|---|
| `Ok` + 解析成功且含 `simOk` | （正常体，`simOk:true`） | — |
| `Ok` + 其它形状 | `sim_failed` + `error`（原样带出宿主函数 `{"error":...}`） | 与 Rhai 脚本路径同构 |
| `Ok` + JSON 解析失败 | `parse_failed` | — |
| `Err` 且 `is_cancelled()` | `cancelled` | 运行时关闭 |
| `Err` 其它（panic） | `sim_failed` + `error`（异常终止信息） | 与「取消」必须可分 |
| 无参考价（不入 `spawn_blocking`） | `missing_input` | — |

**缺陷 2（已修）**：`reason_cancelled` 在 11 语言里**没有对应键**。前端用
`t(\`stockAnalysis.simulation.reason_${reason}\`)` **动态取键**，i18next 缺键时会把
**原始 key 字符串**渲染到界面（用户看到 `stockAnalysis.simulation.reason_cancelled`）。
两处修：

- 11 语言补 `reason_cancelled`（`output/i18n-patch-reason-cancelled.mjs`：写前自校验
  → 备份 → **文本级插入一行**（不用 JSON 往返，避免破坏 dprint 格式）→ 写后校验 key 集合一致）；
- 前端加 `defaultValue: t("...reason_unknown")` 兜底 —— **根因是前端把后端产出的
  开放值域当成了闭集**，只补当前缺的那个键，下次后端再加值仍会漏原始 key。

**⚠ 登记不擅改（需产品语义决策）**：快照里 `error` 字段是**中文自由文本**
（如 `无可用的参考价：…`、`仿真任务异常终止: …`），而前端 `（{simulation.error}）`
会**原样渲染** ⇒ 日/德/法/俄等语言界面里会出现中文句子。i18n 只覆盖 `reason`
（短码枚举），`error` 是开发者向细节。要彻底解决需把 `error` 从自由文本改成
**错误码 + 11 语言文案**，属契约变更（会同步影响 Rhai 脚本路径），故只登记。

> **✅ 已裁决并实施 —— 见 9.11**（用户 2026-09-14：项目错误码不能是自由文本）。

### 9.11 用户裁决：错误码不得是自由文本（规范落地，含一次契约变更）

**裁决原话**：「项目的错误码不能是自由文本，否则 CI 会失败」。

#### 9.11.1 先核实「CI 会不会失败」（判决性事实）

| 门禁（`ci.yml`） | 改前 | 改后 |
|---|---|---|
| `node scripts/check-errorcode-alignment.mjs`（ci.yml:150） | **绿**（394 码） | **绿**（**399** 码） |
| `node scripts/check-rust-raw-map-err.mjs`（ci.yml:291，禁裸 `map_err`） | 绿 | 绿 |

**结论：改前 CI 其实是绿的** —— 因为自造的 `reason_*` 小写码与中文 `error` 自由文本
**完全落在两道门禁的管辖范围之外**：

- `check-errorcode-alignment.mjs` 只取 `error_code.rs` / `harness/error_codes.rs` 里
  `="XXX_YYY"` 形态的**定义值**（正则 `^[A-Z][A-Z0-9]+(?:_[A-Z0-9]+)+$`）。
  `"missing_input"` 全小写、只一段，**取不到** ⇒ 后端与前端两侧的"码集"里都没有它。
- `check-rust-raw-map-err.mjs` 只匹配 `map_err(|x| x.to_string())` 这一种字面形态，
  仿真失败值是**数据字段**（写进快照的 JSON），不是 `Result::Err`，**根本不进扫描面**。

⇒ 这正是本仓库反复出现的病型：**「门禁绿」不等于「契约对」**。真正在跑的那套
错误码没有任何机器在看。裁决的实质要求是：**把仿真失败纳入同一套错误码契约**，
而不是"改到 CI 变绿"。

#### 9.11.2 违反的规范条款（逐条）

AGENTS.md「后端错误码 i18n 规范（强制）」：

| 条 | 要求 | 改前实际 |
|---|---|---|
| 1 | 禁止把裸中文/英文 `to_string()` 作为用户可见错误回传 | `error` 字段是中文自由文本，且**前端原样渲染** |
| 2 | 码命名 `{CATEGORY}_{SHORT_NAME}` 全大写下划线 | 用的是 `missing_input` 等**全小写**自造码 |
| 3 | 动态参数走 `params`，不拼进 `detail` | 中文整句塞在自由文本里 |
| 5 | 前端消费错误码（`t("error.${code}")`）而非裸字符串 | 前端手搓 ``t(`...reason_${reason}`)`` |
| **6** | **错误码翻译平铺顶层 `error` 段，禁止散落到子段** | `reason_*` 散落在 `stockAnalysis.simulation` **子段** |

#### 9.11.3 实施（5 后端 / 3 前端 / 11 语言 / 1 新测试组）

| 层 | 文件 | 改动 |
|---|---|---|
| 后端 | `commands/error_code.rs` | 新增 `stock_sim` 域 **5 个码**（见下表） |
| 后端 | `commands/market_sim.rs` | 新增 `sim_err_json()`；`run_mc_preset` 4 处失败返回由 `{"error":"…"}` 改为 `ErrorResponse` JSON |
| 后端 | `commands/stock_workflow/sim_hook.rs` | 新增 `err_value()` / `mark_failed()`；6 个失败分支产出错误码；**删除全部中文自由文本** |
| 后端 | `commands/sim-verify.rhai` | 失败分支改用码；宿主失败体**原样透传**（不再改写） |
| 前端 | `stores/feature/stockAnalysisStore.ts` | `SimulationSnapshot`：`reason?`/`error?` → `code?`/`category?`/`detail?` |
| 前端 | `components/stock-analysis/SimulationTabContent.tsx` | 删手搓查表 → **复用既有统一层** `translateBackendError(simulation)` |
| i18n | `locales/*.json` ×11 | 删子段 `reason_*` ×6，顶层 `error` 段加 `STOCK_SIM_*` ×5 |
| 测试 | `sim_hook.rs::tests` | 新增 4 个用例（含**反向断言**：`reason`/`error` 必须不存在；码形态正则与 CI 逐字一致） |

**错误码表**（`error_code.rs::stock_sim`）：

| 常量 | 码值 | `category` | 语义 |
|---|---|---|---|
| `INPUT_MISSING` | `STOCK_SIM_INPUT_MISSING` | `validation` | 入参缺失（股票代码为空），仅 Rhai 脚本路径可达 |
| `NO_REFERENCE_PRICE` | `STOCK_SIM_NO_REFERENCE_PRICE` | `validation` | 实时价与快照现价都取不到 |
| `RESULT_PARSE_FAILED` | `STOCK_SIM_RESULT_PARSE_FAILED` | `unrecoverable` | 宿主返回无法识别 / JSON 解析失败 |
| `EXEC_FAILED` | `STOCK_SIM_EXEC_FAILED` | `unrecoverable` | 内核执行失败（含 panic、序列化失败） |
| `CANCELLED` | `STOCK_SIM_CANCELLED` | `retryable` | 运行时关闭导致任务被取消 |

**为什么新增 5 个码而不是复用 `STOCK_WORKFLOW_INTERNAL`**：规范第 2 条要求
「优先复用」，但这 5 个是**面向用户的原因说明** —— 「仿真验证」区块存在的意义就是
回答「为什么这次没有仿真结论」。全塌成一个通用码的话，11 语言只能显示
「股票分析工作流内部错误」，「缺参考价」与「内核崩了」在界面上不可区分，
用户既不知道该补什么数据、也分不清是数据问题还是程序问题。粒度损失大于复用收益。

**前端为什么零特殊分支**：`ErrorResponse` 的字段名 `{code, category, detail, params}`
**就是** `src/lib/errorI18n.ts` 期望的形状 ⇒ `translateBackendError(simulation)`
直接可用（它已经处理了缺译回退 `detail`、`category` 分支、单/双花括号插值）。

#### 9.11.4 一处**副本**及其维护要求（Rhai 读不到 Rust 常量）

`sim-verify.rhai` 的 3 个脚本侧失败分支必须**硬拷码值**（Rhai 无法引用 Rust 常量，
与 `domain_ontology` 三力权重的处境相同）。脚本已注明副本身份与权威源。
**改动 `stock_sim` 域码值时，`sim-verify.rhai` 必须同步** —— 这是已知的、
未被机器覆盖的耦合点。

#### 9.11.5 迁移过程：脚本自校验拦下一次错误写入（值得留档）

`output/i18n-patch-stock-sim-errors.mjs` 按文本级编辑 11 个 locale。**首轮 10/11 失败**，
原因是我假设「锚点键 `STOCK_SETUP_INTERNAL` 是本段最后一个键」—— **错的**。实测三种形态：

| 形态 | locale | 锚点尾逗号 | 其后 |
|---|---|---|---|
| 锚点确为末键 | zh-CN | 无 | `},` |
| 锚点后有其他键 | ar/de/es/fr/hi/ja/ko/ru | **有** | `"network"` … |
| **键序本身不同** | en-US / zh-TW | 有 | `STOCK_WORKFLOW_INTERNAL`（在 SETUP **之后**） |

首版一律剥掉锚点尾逗号、且让最后一项不带逗号 ⇒ 7 个 locale 的「锚点后那个键」
丢前置逗号、JSON 非法。**写后 `JSON.parse` 校验拦住了全部 10 个，只有 zh-CN 落盘**。
修法：按「锚点原本有没有尾逗号」决定插入块的收尾，并**额外校验锚点后一行的形态**
（无尾逗号 ⇒ 后一行必须是段闭合符）。修后 11/11 通过，zh-CN 被识别为「已迁移」跳过。

> 教训：**多语言文件的结构假设必须实测，不能从源语言推**（键序在 locale 之间不一致）。

#### 9.11.6 顺带发现（**未改**，另行登记）

`check-i18n-untranslated.mjs` 的白名单是 `zh-CN, zh-TW, ja`（含汉字正常）⇒
**混在 `ja` 里的中文不会被拦**。实测存量至少两处：

| 位置 | 现值 | 应为 |
|---|---|---|
| `ja.json` → `error.network` | `网络连接失败`（**中文**） | 日文 |
| `zh-TW.json` → `error.STOCK_SETUP_INTERNAL` | `Stock setup internal error`（**英文**） | 繁中 |

两者都在三道 i18n 门禁的**盲区**里（ja 被白名单豁免；英文不含汉字 ⇒ 那道门禁看不到）。
本属存量、且改译文超出本次范围，故只登记。**建议**：让 `ja` 走"假名混用"而非整体豁免，
或补一道「非源语言不得与 zh-CN 值逐字相同」的检查。

---

### 9.12 同类链登记（**已在 §9.13 修复**）：其余位置仍在产中文自由文本

> **状态更新（同日）**：用户下达「修复连带问题」后，本节登记的 3 条链
> （§9.12.2-A 的 #1/#2/#3）连同 §9.12.4 的字符串嗅探**已全部修复**，
> 实施与验证见 **§9.13**。本节保留为**侦察记录**（判定方法 / 盘点结论 /
> 脚本盲区三部分仍然有效，后续同类审计可直接复用）。

用户裁决的是「错误码不能是自由文本」。本轮只修了**仿真**那一条链（§9.11）。
为免"修一条漏九条"，对全仓做了一次盘点 —— 结论是**另有 3 条链同样会把中文漏到非中文界面**，
但**不在本轮范围**（需产品语义决策），故只登记。

#### 9.12.1 判定方法（三步，缺一步就会误判）

判「自由文本错误」是否构成缺陷，**不能只看后端产出点**。三步：

1. **找产出点**：`grep '"error":'` 全仓 → 本仓 **16 处**产 `"error": <自由文本>`。
2. **找消费端**：产物可能是 ① 命令返回值 ② 事件 emit payload。两者消费路径完全不同：
   - 命令返回 ⇒ 查前端 `invoke("<命令名>")` 调用点；
   - 事件 ⇒ 查前端 `listen("<事件名>")`，**必须拿到事件字符串字面量**（见 §9.12.5 的两个脚本盲区）。
3. **判是否渲染**：消费端若把该字段写进 store 的 UI 字段 / `message.error(...)` / JSX 文本节点
   ⇒ **构成缺陷**；若只读同 payload 的其他字段（如只读 `noteId`）⇒ **不构成**。

> 只做第 1 步会得出"16 处全是缺陷"的错误结论；实测其中多数前端**根本不读**。

#### 9.12.2 盘点结论

**A. 前端确实渲染（==构成同型缺陷，3 条）**

| # | 后端产出 | 前端消费 | 渲染点 | 语言 |
|---|---|---|---|---|
| 1 | `stock_workflow/core.rs:871` `format!("分析超时（超过 {} 秒）")`、`:900` `"分析已被取消"`、`:1581` `e.to_string()` | `stockAnalysisStore.ts:2681` `listen("workflow-error")` | `set({ error: msg })` + `progressMessage: msg` | **中文直通** |
| 2 | `stock_workflow/core.rs:552` 透传 `event.error`（引擎节点错误） | `executionStore.ts:889` `listen("workflow-step-error")` | `upsertPoolItem({ error })` | 取决于引擎 |
| 3 | `commands/search.rs:202` `format!("服务器返回 HTTP {}")`、`:208` `e.to_string()` | `SearchProviderSettings.tsx:264` `invoke("test_search_provider")` | `:274` `message.error(result.error \|\| t(...))` | **中文直通**（i18n 兜底被 `\|\|` 短路） |

**B. 前端有消费者但不读 `error`（不构成缺陷）**

| 后端产出 | 事件 | 前端 |
|---|---|---|
| `sim_hook.rs:220`（`simulation` 字段，**已在本轮修复**） | `simulation-ready` | `stockAnalysisStore.ts:2815` `set({ simulation })` ⇒ 走 `translateBackendError` ✅ |
| `llm_wiki.rs:613` | `wiki-note-indexed` | `IngestPanel.tsx:83` 类型为 `{ noteId: string }`，不读 error |

**C. 前端零消费者（不构成缺陷，`error` 字段纯冗余）**

`agent/mod.rs:1067`（`agent-mcp-load-failed`）、`backup.rs:313` + `init/services.rs:574`（`auto-backup-completed`）、
`platform_integration.rs:413`（`api-server-status`）、`webdav.rs:744`（`webdav-sync-completed`）
—— 上述事件名在前端 `src/`（1092 个 ts/tsx）中 **0 命中**。

**D. 消费端未逐一验证（诚实标注，勿当已排查）**

`workflows/mod.rs:544`、`work_engine.rs:613`、`stock_analysis.rs:6130`、`demand_discovery.rs:207`、
`index_queue.rs:1035`、`serenity.rs:942`（`serenity-screening-step`，前端有 listen）。

#### 9.12.3 为什么本轮不改

1. **是行为变更不是纯 bug**：`workflow-error` 那条链的 `msg` 同时参与
   `isRetryableWorkflowError(effectiveErrorCode, msg)` 的**自动重试判定**，
   以及 `status` / `llmStatus` 的状态机（`stockAnalysisStore.ts:2718-2733`）。
   改成错误码会同时改变重试与状态语义 ⇒ 属产品语义决策。
2. **需要新码 + 11 语言译文**：`分析超时` / `分析已被取消` / 引擎节点错误各自需要稳定码，
   一旦定码就要动 11 个 locale（且要过 `check-errorcode-alignment.mjs`）。
   **码名怎么定（`WORKFLOW_TIMEOUT` 还是 `STOCK_WORKFLOW_TIMEOUT`）是产品决策**。
3. 纪律：同一根因链但需产品语义决策者 **只登记不擅改**（见 MEMORY 纪律 #5）。

#### 9.12.4 另发现一处「自由文本猜结构」（K 组病型）

`stockAnalysisStore.ts:2718`：

```ts
const effectiveErrorCode = errorCode ?? (msg.includes("LLM") ? "LLM_FALLBACK" : "GENERIC_ERROR");
```

后端**绝大多数** `workflow-error` 分支没填 `errorCode`（只有部分带），于是前端用
**字符串嗅探** `msg.includes("LLM")` 反推码值 —— 这正是「自由文本猜结构」。
一旦后端把中文自由文本改成码值，`includes("LLM")` 会**永远为假**，降级分支静默失效。
⇒ 修 §9.12.2-A#1 时必须**同时**处理这一行（这是把 #1 归入"需决策"的第二个理由）。

#### 9.12.5 侦察脚本自身的两个盲区（留档，勿重蹈）

写"后端 emit ↔ 前端 listen"对照脚本时踩了两个坑，**都产生假阴性**：

| 盲区 | 形态 | 后果 | 修法 |
|---|---|---|---|
| 正则不跨行 | `let _ = app_h.emit(\n  "workflow-error",` 事件名在下一行 | `workflow-error` 被判为「后端无 emit」，与我已知事实矛盾 | 匹配 `.emit(` 后**向后扫 300 字符**取首个字符串字面量 |
| 事件名与 payload 是 tuple | `s if … => ("workflow-step-error", json!({…}))` | `workflow-step-error` 仍漏报 | 需额外扫 `(\s*"name"\s*,` 形态 |

> 与 MEMORY 判据 #7「审计脚本自身会撒谎：0 命中 ≠ 没问题」同型。**反向断言救了我**：
> 脚本报 `workflow-error` 无 emit，而我**刚在源码里读到 core.rs:701 就是 emit 它** ⇒ 立刻知道是脚本错。

### 9.13 修复 §9.12 登记的连带链（用户指令：「修复连带问题」）

§9.12 判定的 3 条前端可见链 + 1 处字符串嗅探全部修复。

#### 9.13.1 后端：新增 12 个码（`error_code.rs::stock_workflow` 扩域 + harness `search` 扩域）

`stock_workflow` 原本只有一个 `INTERNAL`。**为什么不复用 `INTERNAL`**：这些码是
**面向用户的原因说明** —— 「超时」可重试、「钩子阻断」要去看数据质量、「DAG 成环」是模板问题。
全塌成一个码，11 语言里全变成「股票分析工作流内部错误」，用户既不知道能不能重试、也不知道该找谁。

| 码 | category | 语义 |
|---|---|---|
| `STOCK_WORKFLOW_TIMEOUT` | retryable | 工作流总超时（含应用关闭的等价路径） |
| `STOCK_WORKFLOW_CANCELLED` | general | 用户取消 |
| `STOCK_WORKFLOW_EXEC_FAILED` | unrecoverable | 执行失败兜底（**前端兜底码也用它**，见 9.13.4） |
| `STOCK_WORKFLOW_STEP_FAILED` | unrecoverable | 单节点失败/超时（`workflow-step-error`） |
| `STOCK_WORKFLOW_HOOK_BLOCKED` | unrecoverable | pre_exec 钩子阻断（数据质量预检不通过等） |
| `STOCK_WORKFLOW_VALIDATION_FAILED` | validation | 输入/输出 schema 校验失败 |
| `STOCK_WORKFLOW_GRAPH_INVALID` | validation | DAG 非法（重复 ID / 依赖缺失 / 成环） |
| `STOCK_WORKFLOW_TARGET_NOT_FOUND` | unrecoverable | 工作流或节点不存在 |
| `STOCK_WORKFLOW_SERIALIZE_FAILED` | unrecoverable | 序列化失败 |
| `STOCK_WORKFLOW_INVALID_STATE` | unrecoverable | 非法状态迁移 |
| `SEARCH_HTTP_ERROR` | retryable / unrecoverable | 连通性测试收到非预期 HTTP 状态 |
| `SEARCH_UNREACHABLE` | retryable | 连通性测试连不上端点 |

**`Err(e)` 分支改为按 `WorkflowError` 变体穷举映射**（`core.rs::workflow_error_code`），
而不是 `e.to_string()` 透传 —— 因为该枚举的 `Display` 有两个变体（`InvalidStateTransition` /
`LifecycleHookFailed`）**本身输出中文**。match **不写 `_ =>` 兜底**：将来 harness 加变体会编译报错，
强迫补映射，否则新变体会静默落到兜底码上（与今天这个缺陷同型）。

#### 9.13.2 前端：三条链改走统一翻译层

| 链 | 改动 |
|---|---|
| `workflow-error` | payload 类型 `{error}` → `{code?, category?, detail?}`；展示文本改 `translateBackendError({code, category, detail})` |
| `workflow-step-error` | 同上；池条目只存**已翻译**文本（在写入点翻译，`AgentPoolPanel` / `ExecutionTimeline` 两处展示自动一致，渲染层零改动） |
| `test_search_provider` | `message.error(translateBackendError(result))`；后端失败体从「`error` 字段塞对象/中文」改为扁平 `{ok:false, code, category, detail, latencyMs}` |

`AgentPoolItem` 新增 `errorCode?`，与 `error` 分工明确：**`error` 负责展示、`errorCode` 负责判定**，
组件不得再用 `error.startsWith(...)` 反推语义。

**兼容保留**：三处都仍读旧字段作**展示兜底**（`code` 缺失时用 `detail`/旧 `error`），
以免前端热更新而后端二进制未重启的窗口期界面空白。旧字段**不参与任何判定**。

#### 9.13.3 两处嗅探删除的**行为等价性**论证（这是本轮最需要证明的一点）

删掉 `msg.includes("LLM")` 与 11 个文本子串嗅探，**是否改变了真实行为**？逐条实证：

| 嗅探 | 位置 | 实证结论 |
|---|---|---|
| `msg.includes("LLM")` → `"LLM_FALLBACK"` | `stockAnalysisStore.ts:2718` | **不可达**。`WorkflowError`（`harness/src/workflow_types.rs:2509`）10 个变体**全部与 LLM 无关**；`stock_workflow/` 目录内含 "LLM" 的字符串只出现在 `decision.rs` 的**节点级** error 与日志里，不经 `workflow-error` 的 `detail`。且 `LLM_FALLBACK` 全仓 `grep` 后端 **0 产出** ⇒ 本就是幽灵码 |
| `lc.includes("timeout"/"503"/"busy"…)` | `isRetryableWorkflowError` | **判据来源错误**。它嗅探的是英文关键词，而后端的 detail 是**中文**（如「分析超时（超过 900 秒）」）。改由后端 `category="retryable"` 判定后，行为**从漏判变为正确判定**，不是等价 —— 这是修复 |

⇒ 删除嗅探**不丢任何真实能力**；第一条是去掉偶然性，第二条是纠正一个本就失效的判据。

`speed`：`LLM_DEGRADED_CODES` 保留为**空集合**（不是删掉机制）。理由写在声明处：
将来后端补「LLM 不可用」就绪探测并产出真码时，只需在此加码，**不必回到字符串嗅探**。

#### 9.13.4 顺带修掉的 4 个真 bug（纯缺陷，非产品决策）

| # | 位置 | 症状 | 修法 |
|---|---|---|---|
| 1 | `commands/search.rs:176` | `json!({ "error": ErrorResponse::new(...) })` ⇒ 前端 `message.error(对象)`，antd 渲染出 **`[object Object]`** | 改扁平 `{code, category}`，前端走翻译层 |
| 2 | `stockAnalysisStore.ts:2718` | 兜底码 `GENERIC_ERROR` 是前端自造、后端不产出 ⇒ `error.${code}` 查询**必然落空**、静默退化为裸串展示 | 兜底改为后端真实存在的 `STOCK_WORKFLOW_EXEC_FAILED` |
| 3 | `browserMock.ts:2618` | mock 返回 `latency_ms`，而真实命令与消费端都是 `latencyMs` ⇒ 浏览器模式下连通性测试恒显示 **`undefinedms`** | 字段名对齐 + 补 `resultCount` |
| 4 | `browserMock.ts:2521` `case "get_gateway_metrics"` | mock 返回 `total_requests`/`avg_latency_ms`/`uptime_seconds` 等 7 个 snake_case 字段，与权威 DTO `harness::types::gateway::GatewayMetrics`（`rename_all="camelCase"`）**零字段重叠** ⇒ 浏览器模式下网关监控六项指标恒 `undefined` | 照抄 DTO 的 11 个 camelCase 字段（值仍 0）。**收尾自查时发现**，详见 9.13.8 |

#### 9.13.5 仍未改（继续登记，勿当已排查）

- **第 4 处嗅探**：`stockAnalysisStore.ts:2238` `error.startsWith("EXECUTION_CANCELLED")`
  （`workflow-step-done` 的 `error` 字段）判「运行级取消」。该码在
  `rt-workflow/src/work_engine/node_executor_trait.rs:68` 是**正式常量**，但
  `StepProgressEvent` 只传 `error: Option<String>`，无 `code` 字段。
  结构化需改 rt-workflow 的**公共事件结构**（跨 crate），影响面大于本轮范围 ⇒ 登记不改。
- **§9.12.2-D** 的 6 处消费端未逐一验证项（`workflows/mod.rs:544` 等）维持原状。
- **`workflow-error` 的 `status`/`llmStatus` 状态机现在只由 `LLM_DEGRADED_CODES` 驱动，
  而该集合为空** ⇒ `llmStatus="placeholder"` 与 `AnalysisProgress.tsx` 的「离线模式」标签
  **当前不可达**。这不是本轮引入的缺陷（嗅探版本同样不可达，见 9.13.3），
  但**真实缺口是**：离线（无 LLM Driver）场景目前没有任何结构化信号上报到前端。
  ⇒ **需产品决策**：是否加后端 LLM 就绪探测 + 一个真实码（如 `STOCK_WORKFLOW_LLM_UNAVAILABLE`）。**未改**。

#### 9.13.6 i18n 迁移（12 码 × 11 语言 = 132 条）

脚本 `output/i18n-patch-workflow-errors.mjs`，沿用 §9.11.5 的三态识别 + 写后自校验。
**锚点形态与 §9.11 那次又不同**（复用了同一套 `anchorHasComma` 逻辑，故一次通过）：

| 形态 | locale | 锚点 `STOCK_WORKFLOW_INTERNAL` |
|---|---|---|
| 有尾逗号，后跟 `STOCK_SETUP_INTERNAL` | ar / de / es / fr / hi / ja / ko / ru / zh-CN | 插入块每项带逗号 |
| 无尾逗号，次行为结构闭合符 `},` | en-US / zh-TW | 给锚点补逗号，插入块末项不带逗号 |

11/11 写入成功。写后校验：`JSON.parse` 通过、差集 `+12 / -0`、非 `error` 段零漂移、行尾一致；
译文质量检查：55+132 条**无一条与 en-US 逐字相同**。

#### 9.13.7 验证矩阵

| 项 | 结果 |
|---|---|
| `check-errorcode-alignment.mjs` | ✔ **411** 码双向对齐、无孤儿（399 + 12） |
| `check-rust-raw-map-err` / `check-i18n-untranslated` / `check-id-validation` | ✔ 全 exit 0 |
| `check-hardcoded-i18n.sh --strict` / `check_i18n.py --strict` | ✔ 全 exit 0 |
| `dprint check`（7 个改动文件 + 11 locale） | ✔ exit 0 |
| `tsc --noEmit` | ✔ exit 0 |
| 11 语言 `error` 段一致性 | ✔ 均 **426** 键 |
| `vitest`（两个 stockAnalysisStore + executionStore） | ✔ **110 passed / 3 files**；后续追加 mock 修复后复跑 2 文件 **63 passed** |
| `cargo fmt -p axagent` | ✔ exit 0 |
| `cargo check -p axagent --all-targets` | ✔ **0 error / 0 warning**（等锁后完成，耗时 14m 59s）—— 见 9.13.9 的证据形态 |

**测试侧同步改动**：§9.12 判定的嗅探行为有 3 个用例钉死（`feature` 文件），
本章改为**码驱动**的 5 个用例（含 3 条**反向断言**：① detail 含 "LLM" 但无码 ⇒ 仍 `error`；
② 幽灵码 `LLM_FALLBACK` 不再降级；③ 无码旧格式兜底到真码且展示原文本）。

> **另修一处过期断言（非本章引入）**：`src/stores/__tests__/stockAnalysisStore.test.ts:138`
> 断言 `listen` 被调用 **6** 次，实际 **7** 次 —— 第 7 个是 §9.11 同批「落点乙」新增的
> `simulation-ready` 监听。该断言在上一轮就已过时，只是那次未跑这个测试文件。
> 已同步为 7 并补上事件名断言 + 注释（沿用该文件既有的「该改断言而非删监听」判例）。

#### 9.13.8 收尾自查（等锁期间）：又查出一个真 bug + **审计脚本自造缺陷**

`cargo check` 排队期间做了不依赖编译器的静态自查。**第一版脚本本身是错的**：用整文件 `includes`
匹配，4 处命中里 3 处是**我自己刚写的解释性注释**（`// 此前 mock 写的是 latency_ms`、
`// 历史写 msg.includes("LLM")`）。更要记的是：诊断出这一点、把脚本改成「逐行 + 剥注释 + 带行号」
之后，**下一个脚本的残留检查又犯了一遍同一个错**——「残留旧字段? YES」命中的正是刚写的注释。

> 这是判据 7/#77「审计工具自造缺陷」的**第三次复发**，且首次出现「修复注释本身成为未来扫描的
> 永久锚点」这一形态。已沉淀为**判据 #128**（`MEMORY-RULES.md` K 组）：
> ① 审计脚本一律**先剥注释**再匹配，且该能力做成**可复用脚本**而非每次内联；
> ② 修复注释引用旧写法时留变形痕迹；③ 扫描器必须配**正负对照** + 打印读取字节数。

**修正后的可信结论**：

| 命中（逐行、剥注释） | 判定 |
|---|---|
| `stockAnalysisStore.ts:2238` `error.startsWith("EXECUTION_CANCELLED")` | 属 9.13.5 已登记未改项 ✔ |
| `browserMock.ts` `test_proxy` 的 `latency_ms` | ❌ **假阳性**：后端 `desktop.rs:208` 发 `latency_ms`、消费端 `ProxySettings.tsx:30` 读 `latency_ms` ⇒ 三方一致 |
| `browserMock.ts` `get_gateway_metrics` | ✅ **真缺陷** ⇒ 即 9.13.4-4，已修 |

**追加一处启发式扩扫（线索级）**：mock 内共 **82** 个 snake_case key，逐一对后端源码查名，
**仅 `preferred_model` 查无**。追查为**前端 prompt 里声明的 LLM 输出 schema**
（`AgentGeneratorModal.tsx:19`）⇒ LLM 运行时产出、不属后端字段 ⇒ **良性**。
判别力自证：修前该扫描对 `get_gateway_metrics` 会报 **7 个孤儿**，而它确实就是缺陷 ⇒ 扫描器不是恒空。

**追加修复后的复验**：`tsc --noEmit` ✔ exit 0（日志 0 B）｜`dprint check src/lib/browserMock.ts` ✔ exit 0｜
`vitest` 2 文件 ✔ **63 passed**。

#### 9.13.9 `cargo check` 的等待、完成与**证据形态**

`cargo fmt -p axagent` ✔ exit 0。`cargo check -p axagent --all-targets` 因 build 目录被**另一会话**
（`clippy -p axagent-harness -p axagent-dao -p axagent-trajectory` + `check` + `test`）持锁而排队 15 分钟，
最终 ✔ **exit 0，`Finished dev profile [unoptimized + debuginfo] target(s) in 14m 59s`**。

本节刻意**不把「exit 0」当作证据**（那正是记忆纪律 6 / 判据 72 所禁的「编译通过假阳性」）。实际核验方式：

| 判据 | 实测 |
|---|---|
| 日志出现 `Finished` | ✔ 有（39 行 / 3293 B） |
| 日志含 `error[` / `error:` | ✔ **无** |
| 日志含 `warning:` / `warning[` | ✔ **无** |
| 首行 `Blocking waiting for file lock` | ✔ 有 ⇒ 证明这是**本任务的**那次编译，而非读到别人的旧日志 |
| 覆盖 36 个 workspace crate | ✔ 逐 crate 有 `Checking` 行，末行 `Finished` |

**为何这次 `Finished` 可以算数（对照判据 72-①）**：本任务显式带的 `--all-targets` 等价于
`--lib --bins --tests --benches --examples`，`--tests` 会启用 `cfg(test)` ⇒ 测试代码**确实进了编译图**。
仅当不带 `--all-targets` 时，`Finished` 才不证明测试被编译（那才需要 `cargo test --no-run` 作硬证据）。

**仍为「如实登记」而非已验项**：`clippy -D warnings` 与 `cargo test -p axagent --lib` **本轮未跑**
（不属「修复连带问题」的验收面，且会再排一次队）。若需要，另行发起。

---

## 9.14 节点级失败改**结构化码** + 模板强制重种子（2026-09-14 第十轮）

用户指令两条：① 「`stockAnalysisStore.ts:2238` 的 `EXECUTION_CANCELLED` 嗅探改 rt-workflow 公共事件结构」
② 「`TEMPLATE_VERSION` 照常重新种子化模板」。即 §9.13.5 登记的两项未决项**均已裁决执行**。

### 9.14.1 改动一：`StepProgressEvent` 新增 `error_code`（rt-workflow 公共事件结构）

**问题**：`workflow-step-done` 事件只带自由文本 `error`（`NodeError` 的 `Display`，形如
`"CODE: detail"`），消费端要按语义分流只能做子串嗅探 —— 前端实测写的是
`error.startsWith("EXECUTION_CANCELLED")`（`stockAnalysisStore.ts:2238`）。

**为什么是「改公共事件结构」而不是「换一个更精确的子串」**：子串嗅探有三个固有缺陷 ——
① 漏判/误判（detail 里出现同名字样即中招）② 与文案强耦合（后端改措辞即失效）
③ 跨语言失效。且 `error_code` **本就是现成的**：失败的 match 分支里 `err` 就是 `NodeError`，
它已有 `code()` 方法（`node_executor_trait.rs:112`）—— 只是从未被提升为事件字段。

| 文件 | 改动 |
|---|---|
| `crates/harness/src/workflow_types.rs` | `StepProgressEvent` 加 `pub error_code: Option<String>`（紧跟 `error`），doc 写明「`error_code` 负责判定、`error` 负责展示」 |
| `crates/rt-workflow/.../engine/mod.rs` | **7 处**构造点：`failed` ×2 → `Some(err.code().to_string())`；`timeout` → `Some(error_code::TIMEOUT)`；`running`/`completed` ×4 → `None` |
| `crates/rt-workflow/.../executors/agent_executor.rs` | `streaming` → `None` |

> 覆盖性自查（不靠肉眼）：脚本按 `cb(StepProgressEvent {` 定位构造点 ⇒ **8 个，`error_code` 缺失 0 个**；
> 结构体字段逐个打印确认为 8 个含 `error_code`。

### 9.14.2 边界映射：`node_error_code`（为什么**不能**直接透传基座码）

新增 `stock_workflow/core.rs::node_error_code(Option<&str>) -> Option<(&'static str, ErrorCategory)>`。

**不透传的理由（这条最关键）**：`rt-workflow` 的 `error_code` 是**基座值域**，
而 `check-errorcode-alignment.mjs` 只认 `commands/error_code.rs` 与 `harness/src/error_codes.rs`
两份权威源；前端 11 语言的 `error` 段也只登记这两处的码。透传会让前端拿到**无译文**的码，
`translateBackendError` 查表必然落空、静默退化为裸串展示 —— 即 §9.13.4-2 修掉的那种「幽灵码」。

**三个出口，全部有 11 语言译文**：

| 基座码 | 本域码 | `category` |
|---|---|---|
| `EXECUTION_CANCELLED` | **`STOCK_WORKFLOW_STEP_CANCELLED`（新增）** | `general` |
| `TIMEOUT` | `STOCK_WORKFLOW_TIMEOUT`（复用） | `retryable` |
| 其余 / `None` | `STOCK_WORKFLOW_STEP_FAILED`（兜底） | `unrecoverable` |

**为什么新增 `STEP_CANCELLED` 而不复用 `CANCELLED`**：两者落在**不同事件**上 ——
`CANCELLED` 描述整条工作流的终态（`workflow-error`），`STEP_CANCELLED` 描述 DAG 内并发推进期间
逐个节点上报的中间态（`workflow-step-error` / `workflow-step-done`）。合并会让二者无法区分。

**返回 `Option` 是刻意的（本轮自己踩到的坑）**：第一版让 `None` 也归到 `STEP_FAILED`，
于是 `running` / `completed` 事件会带着一个「步骤失败」码流向消费端 ——
**契约形状对、语义错**，正是本项目最怕的一类缺陷。已改为 `None` 返回 `None`，
`workflow-step-done` 的 `errorCode` 序列化为 `null`，前端判据相应写成
`typeof errorCode === "string"`（而不是 `!== undefined` —— `null !== undefined` 为真）。

### 9.14.3 改动二：前端删嗅探（判据 **126** 的第二次应用）

```ts
// 前（子串嗅探）
const isRunLevelCancel = status === "failed"
  && typeof error === "string" && error.startsWith("EXECUTION_CANCELLED");
// 后（结构化集合，与既有 RETRYABLE_WORKFLOW_CODES 同型）
const RUN_LEVEL_CANCEL_STEP_CODES = new Set(["STOCK_WORKFLOW_STEP_CANCELLED"]);
const isRunLevelCancel = status === "failed"
  && typeof errorCode === "string" && RUN_LEVEL_CANCEL_STEP_CODES.has(errorCode);
```

**可达性论证（判据 126 要求，先证再改）**：该嗅探**是可达的**（`check_cancellation_or_pause`
在硬取消与「暂停期间检测到取消」两处产出该码，经 `Ok(Err(err))` 分支流入事件），
故这是**真行为变更**而非等价重构 —— 差别在于：旧写法要求 `error` 恰好以该码**开头**
（`NodeError::Display` 为 `"{code}: {detail}"`，开头确实成立，故旧写法「碰巧能用」），
但它同时依赖「detail 里不会先出现同名字样」这一未约定前提。新写法该前提消失。

### 9.14.4 改动三：`TEMPLATE_VERSION` 46 → 47（强制重种子）

**本版无模板内容变更**，是一次纯版本推进。理由是版本门的**静默失效形态**：
`sim-verify.rhai` 的文本在 v46 声明之后**仍被再次修改**（实测 mtime：脚本 06:07 > 本文件 04:25），
而该脚本经 `include_str!("../sim-verify.rhai")`（本文件 L3684）嵌入节点体 ⇒
若 v46 的种子恰好跑在这两次写之间，**DB 里存的是旧脚本文本**，而
`existing.version >= TEMPLATE_VERSION` 会让它此后**永远不再更新** ——
典型的「仓库对、DB 错、且没有任何提示」。

⚠️ **代价（已与用户确认接受）**：会覆盖 DB 中该模板 `nodes`/`edges` 的任何手工调整；
仅 `variables` 的自定义值在升级前被保留（见 `old_variables`）。

### 9.14.5 验证矩阵

| 项 | 结果 |
|---|---|
| `check-errorcode-alignment.mjs` | ✔ **412** 码双向对齐、无孤儿（411 + 1） |
| `check-rust-raw-map-err` / `check-i18n-untranslated` / `check-id-validation` | ✔ 全 exit 0 |
| `check-hardcoded-i18n.sh --strict` / `check_i18n.py --strict` | ✔ 全 exit 0 |
| 11 语言 `error` 段 | ✔ 均 **427** 键（426 + 1），11/11 键集一致 |
| `dprint check`（2 源文件 + 11 locale） | ✔ exit 0 |
| `tsc --noEmit` | ✔ exit 0（0 B 日志） |
| `vitest`（3 文件） | ✔ **115 passed**（110 + 5 新用例） |
| 剥注释静态自查（`output/selfcheck-stepcancel.txt`） | ✔ 生产代码 0 命中；唯一 `latency_ms` 系已证良性的 `test_proxy` |
| `cargo fmt -p axagent -p axagent-harness -p axagent-rt-workflow` | 见 §9.14.6 |
| `cargo check -p axagent --all-targets` | 见 §9.14.6 |
| `cargo check -p axagent-harness -p axagent-rt-workflow --all-targets` | 见 §9.14.6 |

**测试侧新增 5 个用例**（`src/stores/feature/__tests__/stockAnalysisStore.test.ts` 的 `5.1` 组）：
码为 `STEP_CANCELLED` ⇒ 不计入 failedNodes｜**反向断言**：文本含取消字样但无码 ⇒ 仍算节点失败
（钉死「嗅探已移除」）｜码为 `STEP_FAILED` ⇒ 计入且保留原始文本｜
**边界**：`errorCode: null` ⇒ 仍算真实失败｜该节点随后 completed ⇒ 标记摘除。

### 9.14.6 编译验收（**已实测**）

日志：`output/cargo-stepcancel-summary.txt`（exit code）｜`cargo-check-stepcancel.log`｜`cargo-check-stepcancel-deps.log`。
判据同 §9.13.9：须同时满足「含 `Finished`」「无 `error`」「无 `warning`」「出现 `Blocking waiting`
（证明是本任务那次，不是串到别的会话的编译）」。

| 命令 | exit | `Finished` | error | warning | `Blocking waiting` |
|---|---|---|---|---|---|
| `cargo fmt -p axagent -p axagent-harness -p axagent-rt-workflow` | **0** | — | — | — | — |
| `cargo check -p axagent --all-targets` | **0** | ✔ `dev … in 13m 25s` | **0** | **0** | ×1 |
| `cargo check -p axagent-harness -p axagent-rt-workflow --all-targets` | **0** | ✔ `dev … in 4m 27s` | **0** | **0** | ×1 |

`summary.txt` 全文 = `FMT_EXIT=0` / `CHECK_AXAGENT_EXIT=0` / `CHECK_DEPS_EXIT=0` / `DONE`（4 行齐 ⇒ 三条命令都真跑完，不是中途被掐）。
`--all-targets` ≡ `--lib --bins --tests --benches --examples`，故 `Finished` 覆盖 `cfg(test)`（首行 35 个 `Checking` 亦与全量一致）。

> ✅ 结论：§9.14 的改动（`StepProgressEvent.error_code` + 边界映射 + 前端删嗅探 + 模板 v47）**编译通过，0 error / 0 warning**。

### 9.14.7 三项登记（**已于 §9.15 全部处理**）

登记原文保留如下，作为「登记理由本身可能是错的」的实证材料 —— 其中 **2/3 的登记理由被 §9.15.0 推翻**：

| # | 项 | 登记时写的「为什么不动」 | 后续核验 |
|---|---|---|---|
| 1 | `src-tauri/src/commands/stock_workflow/serenity.rs:929` 的 `serenity-screening-step` 未透传 `error_code` | 该事件**无消费端**读取取消语义 ⇒ 加了是「写进产物没人消费」 | ❌ **理由不成立**：有界面消费端且**直接渲染中文自由文本** |
| 2 | `stock_pipeline/core.rs:162` 的 `pipeline_step` 文案仍用 `format!` | 它是面向运行日志的中文串（不经 `translateBackendError`），不属错误码契约面 | ❌ **理由不成立**：经 `pipeline-step` emit、由 `PipelinePage` 渲染 |
| 3 | 前端 `stockAnalysisStore.ts:1007` 的 `errorCode: "DATA_QUALITY_INSUFFICIENT"` | 幽灵码（后端 0 产出；根因 `skipped` 响应不发 `code`）；字段无消费端 ⇒ 潜伏缺陷；修法 1 行 | ✔ 理由成立（确无消费端），但**根因侧该修**，本轮一并修 |

---

## 9.15 清理三项登记（2026-09-14 第十一轮，用户指令：「继续仍未改的项目」）

### 9.15.0 先证「登记理由」是否成立（本节最重要的方法）

三项的登记理由都建立在**「有没有消费端」**这个判断上，而登记时我只 grep 了后端。补上消费端核验后，**两项的理由当场被推翻**：

| 项 | 实测消费端 | 实际后果 |
|---|---|---|
| 1 | `SerenityScreeningPanel.tsx:1221`（折叠摘要 `truncateText(s.error, 60)`）／`:1282`（展开态 `<div className="…text-red-500">{s.error}</div>`） | `s.error` = `NodeError::Display` = `"EXECUTION_CANCELLED: 节点执行已取消"` ⇒ **中文自由文本直接漏到界面**，非中文用户也看到中文 |
| 2 | `PipelinePage.tsx:218-225`（`ev.detail` 直接渲染） | `detail` = `"<nodeId>: 执行中"` ⇒ **中文硬编码漏到界面**，且同一处混着英文 `failed` |
| 3 | 全仓无组件读 `stockAnalysisStore.errorCode`（仅 store 赋值 + 测试） | 确为潜伏缺陷 —— 但**根因**（后端不发 `code`）是活动的，前端只是「在没有码时自己编了一个」 |

> 判据沉淀：**「无消费端」这个结论必须先扫前端再下**。登记者与消费者常在不同层
> （Rust emit ↔ TSX render），只在 `src-tauri/` 里 grep 不出来的消费端真实存在。
> 这已是**同一项目内第二次**因「只扫单侧」而误判可达性（第一次见判据 #126）。

### 9.15.1 项 1：`serenity-screening-step` 补 `errorCode`

后端（`stock_workflow/serenity.rs`）在事件 payload 里补一个字段：

```rust
"errorCode": super::core::node_error_code(event.error_code.as_deref()).map(|(c, _)| c),
```

配套把 `core.rs::node_error_code` 的可见性从私有放开为 **`pub(super)`**（即对整个
`stock_workflow` 模块可见，`serenity.rs` 以 `super::core::node_error_code` 引用）——
**刻意复用而非再写一份映射**：同一张「基座码 → 本域码」表若散成两份，迟早只改一份
（判据「N 份实现只 1 份在跑」）。前端侧：

- `serenityStore.ts` 的 `StepLog` 增 `errorCode?: string | null`（与 `stocksAnalysisStore` 同形）
- `SerenityScreeningPanel.tsx` 的 `listen<…>` 泛型增 `errorCode?: string | null`，并透传进 `addStep`

> 只做**透传**，不改渲染。把 `s.error` 改走 `translateBackendError` 会**丢弃 detail**
> （该函数命中码时只返回本地化文案，见 `lib/errorI18n.ts`）⇒ 属产品文案决策，另记
> §9.15.7。本轮不擅自降级信息量。

### 9.15.2 项 2：进度载荷结构化（后端不再拼中文）

`stock_pipeline/core.rs` 三处改动：

1. **新增载荷结构体** `PipelineStepInfo { node_id: String, status: String }`（`stock_pipeline/core.rs:134`）——
   刻意用结构体而非继续传两个 `&str`：`&str, &str` 的第二个参数在历史实现里被塞进
   「拼好的中文」，正是问题源头；换成结构体后产出端**没有地方放人类可读文案**。
2. **回调签名**：`Option<Arc<dyn Fn(&str, &str) + Send + Sync>>`
   → `Option<Arc<dyn Fn(&str, PipelineStepInfo) + Send + Sync>>`（L151）。
3. **载荷**：删除 `format!("{}: 执行中", …)` / `"完成"` / `"{s}"` 三个分支，改为原样透传结构化值
   （L179-186）。**注意**：给 rt-workflow 的 `ProgressCallback` 传的仍是
   `StepProgressEvent`（该类型由 rt-workflow 定义，不可改），本改动只发生在本 crate 自己的
   中间回调上 —— 即**没有**为此再加一次跨 crate 契约变更。

前端 `types/pipeline.ts::PipelineStepEvent` 增 `nodeId?` / `status?`，`detail` 标
**`@deprecated`**（保留作旧载荷兜底）；`PipelinePage.tsx` 用**显式映射表**把状态换成 key：

```ts
// PipelinePage.tsx:25 注释即写明：必须显式映射，不要用 t(`pipeline.step${status}`) 拼 key
const STEP_STATUS_KEY: Record<string, string> = {
  running: "pipeline.stepRunning",   completed: "pipeline.stepCompleted",
  failed:  "pipeline.stepFailed",    timeout:   "pipeline.stepTimeout",
};
```

```tsx
const statusKey = rawStatus ? STEP_STATUS_KEY[rawStatus] : undefined;
const statusText = statusKey ? t(statusKey, { defaultValue: rawStatus })
                             : (rawStatus || (ev.detail ?? ""));
```

即：**文案由前端 i18n 组装，后端只给语义**；三轮兜底（映射命中 → 原样状态值 → 旧 `detail`）
保证未知状态与旧载荷都不会渲染成空白。**刻意不做 `t(\`pipeline.step${status}\`)` 拼接** ——
拼接型 key 一旦后端多出状态就恒 miss（`error.${code}` 那条幽灵码是同一病型）。

> 顺带修正一处**文档腐化**：该函数 doc 原写「供 Tauri 命令**和 cron** 调用」，但全仓
> grep 显示**只有 1 个调用者**（Tauri 命令）；且 `init/services.rs:2405` 与
> `pool_scan.rs:14` 都写明该模板 `trigger = Manual`、**永不自动调度** ⇒ doc 里的
> 「和 cron」是假的，已删除。

### 9.15.3 项 3：`skipped` 响应补 `code`，并**撤回我新加的码**

`stock_workflow/core.rs` 的 `QualityPrecheckResult::Insufficient` 分支补 `"code"`。

**码的选择上我先做错了一次**：第一直觉是新加 `STOCK_WORKFLOW_INSUFFICIENT_DATA`
（12→13 码 + 11 语言译文都已落盘）。改完复核时读到 `error_code.rs:295` 里
`HOOK_BLOCKED` 的**既有文档**：

> 前置检查未通过，分析已中止（**如数据质量预检不通过**）

⇒ 该场景**本就是 `HOOK_BLOCKED` 的覆盖范围**，属**漏接线**而非缺码；且 `HOOK_BLOCKED`
有真实产出点（`core.rs:78` ← `LifecycleHookFailed`），不是幽灵码。同一语义散成两个码
是典型的**一名多义**病型。故：

1. 从 11 个 locale 的备份**回滚**（`output/backup-2026-09-14/*.before-insufficient-data+…`），
   逐文件断言备份里不含该键、回滚后 `error` 段回到 **427**、`pipeline` 段回到 **13**；
2. patcher spec 只保留 pipeline job，重新落盘；
3. 前端 `stockAnalysisStore.ts` 改读后端码并**不再编造**：

```ts
errorCode: (result.code as string | undefined) ?? null,
```

（`null` = 明示「后端没给码」，而不是塞一个自己造的码 —— 同判据 #129 的值域纪律。）

> 门禁印证：撤回后 `check-errorcode-alignment.mjs` 报 **412 码双向对齐、无孤儿**；
> 若保留新码而不被任何 emit 点引用，这里会直接红。**门禁是这个判断的第二把锁。**

### 9.15.4 i18n：只新增 `pipeline` 段 4 键（`error` 段不新增）

| 段 | 键 | 变化 |
|---|---|---|
| `error` | — | **426 → 427 → 427**（v10 的 `STEP_CANCELLED` 保留，本轮回滚掉 `INSUFFICIENT_DATA`） |
| `pipeline` | `stepRunning` / `stepCompleted` / `stepFailed` / `stepTimeout` | **13 → 17**（11 语言一致） |

迁移工具**参数化后可复用**：`output/i18n-patch-keys.mjs`（不再是一次性脚本），三态识别
（已迁移/部分迁移/未迁移）＋**段内锚点定位**（不是全文件子串）＋锚点尾逗号形态断言＋
写前备份（逐语言 `.before-<tag>`）＋写后 `JSON.parse` + 键数 + 双向差集 + 非目标段零漂移。
幂等性已实测：连续 `--apply` 两次键数不变、无重复插入。

### 9.15.5 验证矩阵

| 项 | 结果 |
|---|---|
| `check-errorcode-alignment.mjs` | ✔ **412** 码双向对齐、无孤儿（撤回新码后仍对齐） |
| `check-rust-raw-map-err` / `check-i18n-untranslated` / `check-id-validation` | ✔ 全 exit 0（`id-validation` 有 **38 条 WARN 级**存量违规、不阻塞提交；与本轮改动无关，未新增） |
| `check-hardcoded-i18n.sh --strict` / `check_i18n.py --strict` | ✔ 全 exit 0 |
| 11 语言段键数 | ✔ `error` **427** × 11｜`pipeline` **17** × 11，键集一致 |
| `dprint check`（6 源文件 + 3 locale 抽查） | ✔ exit 0 |
| `tsc --noEmit`（**含本轮新增测试**） | ✔ exit 0（0 B 日志） |
| `vitest`（4 文件） | ✔ **118 passed**（`feature/stockAnalysisStore` 60 + `stores/stockAnalysisStore` 10 + `errorI18n` + `stock-analysis` + `ci.workflow`） |
| `cargo fmt -p axagent` | ✔ `FMT_EXIT=0` |
| `cargo check -p axagent --all-targets` | ✔ 见 §9.15.6 |

**前端新增 2 个回归用例**（`feature/__tests__/stockAnalysisStore.test.ts` 第 6 组）：
后端 `skipped` 带 `code` ⇒ `errorCode` 取后端码且**断言 `not.toBe("DATA_QUALITY_INSUFFICIENT")`**
（钉死幽灵码已删）｜后端不带 `code` ⇒ `errorCode` 为 **`null`**（钉死「不编造码」）。

### 9.15.6 编译验收（**已实测**）

日志：`output/cargo-threeitems-summary.txt` ＋ `output/cargo-check-threeitems.log`。判据同 §9.13.9。

| 命令 | exit | `Finished` | error | warning | `Blocking waiting` |
|---|---|---|---|---|---|
| `cargo fmt -p axagent` | **0** | — | — | — | — |
| `cargo check -p axagent --all-targets` | **0** | ✔ `dev … in 17m 56s` | **0** | **0** | ×1 |

`summary.txt` 全文 = `CHECK_AXAGENT_EXIT=0` / `DONE`（齐 ⇒ 真跑完）。

> ✅ 结论：§9.15 的三项清理（`serenity` 透传码 + 管道载荷结构化 + `skipped` 补码）
> **编译通过，0 error / 0 warning**。

### 9.15.7 顺带修正与仍存的登记

**顺带修掉（纯缺陷，顺手改）**：

| 项 | 内容 |
|---|---|
| 文档腐化 | `run_stock_pipeline_inner` doc 的「和 cron 调用」为假（唯一调用者是 Tauri 命令；模板 `trigger = Manual` ⇒ 永不自动调度），已删 |
| 注释腐化 | `stockAnalysisStore.ts:1010` 我上一轮写的注释仍写「后端发 `STOCK_WORKFLOW_INSUFFICIENT_DATA`」，与 §9.15.3 的最终决定（复用 `HOOK_BLOCKED`）矛盾 —— 属**自证性注释污染**，已按 `HOOK_BLOCKED` 更正 |
| 幽灵码 | 前端自造的 `DATA_QUALITY_INSUFFICIENT` 已删；现存字样仅在**注释与测试反向断言**中（有意保留） |

**仍登记未改**：

| # | 项 | 为什么不动 |
|---|---|---|
| 1 | ~~`SerenityScreeningPanel` 直接渲染 `s.error`（`NodeError::Display`，含中文）~~ | **已于 §9.16 处理**（不再需要「产品裁决」—— 见 §9.16.0） |
| 2 | ~~`StockAnalysisConfigPanel` 里 `error.${code}` 的展示兜底~~ | **该登记行不成立，已撤回**：该文件的错误路径只有 `t("stockAnalysis.settings.*")` 与 `console.error` 两类，**全仓无 `error.${code}` 兜底**（`grep 'error\.\$\{'` 的命中仅 `errorI18n.ts` 自身与其文档）。属我登记时未核实的臆断 |

---

## 9.16 失败文本的本地化展示（2026-09-14 第十二轮，用户指令：「继续仍登记未改」）

### 9.16.0 入口：登记项与「产品裁决」的化解

§9.15.7 项 1 登记的是：`SerenityScreeningPanel` 把 `s.error`（`NodeError::Display` 自由文本，
含中文）**直接渲染**。当时判为「产品裁决」的理由是：改走 `translateBackendError` 会**丢 detail**，
于是要在「本地化但不含原因」与「含原因但可能非本地化」之间二选一。

**这个二选一是伪命题** —— 两者可以并存，因为**折叠态与展开态的职责不同**：

| 位置 | 职责 | 采用 |
|---|---|---|
| 折叠态单行摘要 | 一眼看懂「为什么失败」 | **本地化译文**（`error.${code}`，11 语言） |
| 展开态详情 | 排查用，承载具体原因（LLM 报错正文 / IO 详情） | **保留后端原文**（无对应译文） |

于是无需裁决即可同时拿到「本地化」与「不丢原因」。**登记时把「做不到」误判为「需要裁决」**
是本轮的第一条方法教训。

### 9.16.1 先量形态：为什么不能从前端「剥前缀」

改展示前必须先回答：码能不能从原文里解析出来？实测**不能**，两条独立理由：

| 来源 | 形态 | 后果 |
|---|---|---|
| `NodeError::Io`（`node_executor_trait.rs:89`） | `#[error(transparent)]` ⇒ 原文是 `std::io::Error` 的 Display，**不带 `CODE: ` 前缀** | 前端 `split(":")` 剥前缀会在该变体上失配 |
| `WorkflowError::{InvalidStateTransition, LifecycleHookFailed}`（`workflow_types.rs:2563`/`:2565`） | `Display` 本身**输出中文**（`"非法状态迁移: …"` / `"生命周期钩子 … 阻断执行: …"`） | 连「原文是技术英文」这个前提都不成立 |

结论：**码必须由后端结构化给出，前端只消费**。这与 §9.14 的做法一致。

### 9.16.2 改动 1：翻译层新入口 `translateFailureText`

`src/lib/errorI18n.ts` 新增：

```ts
export function translateFailureText(
  error: string | null | undefined,
  code: string | null | undefined,
): string {
  const raw = error ?? "";
  if (!raw || typeof code !== "string" || !ERROR_CODE_RE.test(code)) {
    return raw;
  }
  return translateBackendError({ code, detail: raw });
}
```

三条路径（均有测试钉死）：有码且 locale 收录 ⇒ 译文；有合法码但未收录 ⇒ 回退 `detail`（= 原文）；
无码 / 非法码 / 原文空 ⇒ 原文（零回归）。

**为何空原文要短路**：`translateBackendError` 在码**未被收录**时会走 `parsed.detail || parsed.raw`，
而**对象入参的 `raw` 是 `JSON.stringify({...})`** ⇒ 界面会显示 `{"code":"X"}`。
尖角的精确条件是「合法码 + locale 未收录 + detail 空」三者同时成立（见 §9.16.5）。

命名从 `translateNodeError` 改为 `translateFailureText`：本轮起**工作流级**失败通道也用它，
`Node` 会造成误导（一名多义是本项目已数次栽过的坑）。

### 9.16.3 改动 2：节点级展示（`SerenityScreeningPanel`）

1. 新增 `failureText = translateFailureText(s.error, s.errorCode)`；
2. 折叠摘要 `truncateText(s.error, 60)` → `truncateText(failureText, 60)`；
3. 展开态**保持原样**（渲染 `s.error` 原文）—— 刻意不在展开块重复主文案，
   否则摘要与展开块显示同一句话（这是本轮的第二条自我纠错，见 §9.16.6）。

### 9.16.4 改动 3：工作流级通道补码（`serenity-screening-completed`）

实证发现**第三条**自由文本漏出通道，且比登记项更严重（**两层中文叠加**）：

```
WorkflowError::Display（2/10 变体自带中文）
  → src-tauri/src/commands/stock_workflow/serenity.rs:1403  format!("Serenity 筛选工作流失败: {e}")
    → emit "serenity-screening-completed" { error }
      → SerenityScreeningPanel.tsx:376  store.setError(p.error)
        → 组件 L1357  {error}  ← 直接渲染
```

修法**不新增码**：`stock_workflow/core.rs` 早有 `workflow_error_code`，**穷举 `WorkflowError` 全部 10 个变体**
（含 `LifecycleHookFailed → HOOK_BLOCKED`），且第 59-61 行的注释已写明「不写 `_ =>` 兜底」。
只需把它的可见性放开给兄弟模块（`pub(super)`），`serenity.rs` 便能复用**同一份**映射：

```rust
let (code, _category) = super::core::workflow_error_code(&e);
// emit 时补 "code": code（`error` 保留原文作技术详情）
```

前端 `store.setError(translateFailureText(p.error, p.code) || i18n.t("serenityPanel.errorUnknown"))`。

> 顺带修掉 `core.rs` 该函数 doc 里一个**腐化行号**（写 `workflow_types.rs:2533`，实际中文在 `:2563`/`:2565`）。

### 9.16.5 一次被负向对照推翻的假设（本轮最值钱的一条）

我给 `translateFailureText` 写的第一版 doc 断言：「对象入参 + 空 `detail` ⇒ 退化成 JSON 串」。
为**实证**而非读码断言，写了「行为记录」用例：

```ts
const text = translateBackendError({ code: "TOOL_NOT_FOUND", detail: "" });
expect(text).toContain('"code"');   // ← 第一次跑 6ms 就红了
```

实测返回 **`"工具未找到"`** —— 因为码**被 locale 收录**时 L156-158 直接返回译文、根本不看 `detail`。
即我的判据**条件写宽了**，尖角只在「码**未**收录」时才触发。

这与判据 #128 是同一型错误（**判据不锚定被测对象自身形态**）。修正后 doc 与用例都写成精确条件，
并加了负向对照（同码 + 非空 detail ⇒ 正常回退原文）。

> 价值：这次是**测试推翻了我的注释**。若只写正向用例（「有码 ⇒ 译文」全绿），
> 这条错误条件会作为「已实测」留在文档里，下次误导别人。

### 9.16.6 一次自我纠错：展开块的冗余渲染

第一版把展开块写成「主文案 + 原文详情」两行，用例断言无码时只有 1 处文本 ⇒ 实测 **2 处**。
查明后发现「2 处」不是 bug 而是**我的设计会引入真冗余**：折叠摘要在展开时**仍然显示**，
展开块再渲染一次主文案就是同一句话出现两遍。

改为「摘要承担主文案、展开块只留原文」后，用例语义也更清晰：
有码时译文**恰好 1 处**（钉死不重复）+ 原文可查；无码时摘要与详情同源共 2 处（**改动前既有行为**）。

### 9.16.7 验证矩阵

| 项 | 结果 |
|---|---|
| `dprint check`（5 文件） | ✔ exit 0 |
| `tsc --noEmit` | ✔ exit 0（0 B 日志） |
| `vitest`（`errorI18n` + 新增组件测试） | ✔ **30 passed**（23 + 7） |
| `vitest`（既有受波及：`stockAnalysisStore` / `stockWorkflowChatBridge`） | ✔ 85 passed（3 文件合计） |
| `check-errorcode-alignment.mjs` | ✔ **412** 码双向对齐、无孤儿（未新增码，故数字不变） |
| `check-rust-raw-map-err` / `check-i18n-untranslated` / `check-id-validation` | ✔ exit 0（`id-validation` 38 条**存量** WARN、不阻塞） |
| `check-hardcoded-i18n.sh --strict` / `check_i18n.py --strict` | ✔ exit 0 |
| 改名残留核验（改后 grep） | ✔ 5 文件旧名 0 命中 |
| i18n key | ✔ **未新增任何 key**（复用既有 12 个 `stock_workflow` 域码的译文） |

### 9.16.8 编译验收（**已实测**）

| 判据 | 实测 |
|---|---|
| `cargo fmt -p axagent` | ✔ `FMT_EXIT=0` |
| `cargo check -p axagent --all-targets` | ✔ `CHECK_AXAGENT_EXIT=0` / `DONE` |
| 含 `Finished` | ✔ ``Finished `dev` profile [unoptimized + debuginfo] target(s) in 12m 11s`` |
| 无 `error` | ✔ **0** 命中 |
| 无 `warning` | ✔ **0** 命中 |
| `Blocking waiting for file lock` | ✔ ×1（证明是本任务那次） |

> ✅ 结论：§9.16 的三处改动（`translateFailureText` 新入口 + 节点级展示 + 工作流级通道补码）
> **编译通过，0 error / 0 warning**。
>
> 本次落点全部在**前端 + 命令层**（`stock_workflow/core.rs` 放开可见性 + doc 行号、
> `stock_workflow/serenity.rs` 的 emit 补 `code`、前端 5 文件），
> **未触碰跨 crate 公共结构** ⇒ 不像 §9.14 那样需要另跑 harness / rt-workflow 的 `--all-targets`。

### 9.16.9 方法教训（本轮新增 3 条）

| # | 教训 | 判据 |
|---|---|---|
| 1 | 「做不到」常被误判为「需要裁决」—— 先问「能不能两者都给」 | 登记理由须写清**为何二选一不可避免** |
| 2 | 判据条件写宽（对象入参 + 空 detail）会被负向对照当场推翻 | #128 的又一实例 |
| 3 | 同一文件的多个 `Edit` **必须串行**：同一条消息里并行两个 Edit 会各自读到旧内容、后写者覆盖先写者 | 纪律 2（同文件 Edit 串行 + **改后 grep 验证**）—— 本轮违反了「改后验证」，靠测试报错才发现 |

### 9.16.10 落库部分失败通道（第五条同型；**已修 · 第十四轮**）

清查「还有没有别处把自由文本直接渲染」时，在同一面板内又发现一条 ——
`partial_failure` 分支：

```ts
// SerenityScreeningPanel.tsx:421-423
if (p.status === "partial_failure" && typeof p.persistenceError === "string") {
  store.setError(p.persistenceError);        // → 组件 L1357 {error} 直接渲染
}
```

`persistenceError` 的产出端是 `serenity.rs` 的

```rust
persistence_detail = format!("写入 {} 失败: {}", code, e);   // 中文硬编码 + DB 错误原文
```

**当初登记的三个顾虑 — 本轮逐条处置**：

1. 「需判断新增 vs 复用」⇒ 判定为**新增** `STOCK_WORKFLOW_PERSIST_FAILED`，**不**复用 `INTERNAL`。
   判据不是「语义够不够宽」（`INTERNAL` 的 doc 本已覆盖「DB 写入」），而是**消费端行动相反**：
   `INTERNAL` 描述「工作流自身的基础设施操作失败」（`backtest_validation.rs:110`、`quant_backtest.rs:189`
   的既有用法）⇒ 用户应**重试**；本码描述「筛选成功、候选有效、只是这次没进历史」⇒ 重试只会
   **白烧一次 API 额度**（Serenity 单轮 300+ 次调用），正确动作是照用候选。合并会让「内部错误」
   同时指代两件行动相反的事（判据 K：一名多义）。也未复用 `SERIALIZE_FAILED`（那是序列化，非 DB 写入），
   `harness` 的 `STORAGE_WRITE_FILE_FAILED` 是文件存储域，跨域复用会成第二个「一名多义」。
2. 「结构上要拆三元组」⇒ 已拆，并**顺带修掉一处信息丢失**：原 `format!` 是**覆盖赋值**，
   多只落库失败只留最后一只 ⇒ 现改为「首个失败记 `params` + `detail`，逐只 `tracing::warn!` 保留全量原因」
   （日志侧本来就有全量，界面侧给首个即可，不必为多只失败造聚合文案）。
3. 「需 11 语言迁移」⇒ 一次性脚本完成；`check-errorcode-alignment.mjs` 由 412 → **413** 码，对齐无孤儿。

**实现（第十四轮）**：

| 层 | 改动 |
|---|---|
| 码 | `error_code.rs::stock_workflow::PERSIST_FAILED` = `STOCK_WORKFLOW_PERSIST_FAILED`（+1 码，doc 写明与 `INTERNAL` 的分界及「不是 `SERIALIZE_FAILED` 同类」） |
| 产出 | `serenity.rs`：`persistence_detail` 去掉中文前缀、只存 DB 原文；新增 `persistence_stock_code`（params）；payload 增 `persistenceCode` / `persistenceStockCode` |
| 传输 | `partial_failure` 事件三者同发；`persistenceError` 语义由「中文整句」收窄为「技术详情」（**向后兼容**：无码旧载荷时前端回退原文，与迁移前逐字一致） |
| 展示 | `SerenityScreeningPanel`：主文案 `translateFailureText(p.persistenceError, p.persistenceCode)`；详情行 = `stockCode: dbError`（**有码才拼**，否则旧载荷会把代码渲染两遍） |
| 容器 | `serenityStore` 新增 `errorDetail` 字段（主文案 / 技术详情的分工写进字段 doc）；`setError(e, detail = null)` **缺省即清空**（否则上一次的 DB 原文串台），`reset()` 走 `initialState` 自动对齐 |
| 渲染 | 顶层错误块加详情行（`errorDetail !== error` 才渲染 ⇒ 与主文案同串时不重复）；`console.warn` 改打**结构化字段**（非中文环境查日志同样可读） |

**测试**：`SerenityScreeningPanel.nodeFailure.test.tsx` 7 → **11 例**，新增 4 例覆盖本通道 ——
有码（主文案译文 + 详情补「哪只」）｜有码缺 `persistenceStockCode`（详情退化为纯 DB 原文，**无** `: ` 前缀）
｜旧载荷（原文整句 + 不重复，`getAllByText(...)` 长度 = 1）｜候选不受影响（落库失败只提示，候选照常渲染）。

> **至此 PLAN 内「已登记未改」项清零**（§9.15.7 项 1 已于第十三轮修、项 2 已撤回；§9.16.10 本轮修）。

**验收实测（第十四轮）**：

| 项 | 结果 |
|---|---|
| `cargo check -p axagent --all-targets` | `Finished dev profile [unoptimized + debuginfo] target(s) in 13m 01s`｜**0 error / 0 warning**｜`--all-targets` 保证测试一并编译（判据 #72：不带 target 的 `Finished` 不证明测试被编译） |
| 证据时序 | `error_code.rs` 08:13:27、`serenity.rs` 08:13:46，**均早于** 08:16 启动的编译 ⇒ 验收有效 |
| `check-errorcode-alignment.mjs` | ✔ **413** 码（412 → +1），对齐无孤儿 |
| `check-rust-raw-map-err.mjs` / `check-hardcoded-i18n.sh --strict` | ✔ exit 0（新增中文**注释**未被 Rule 1 误判） |
| `tsc --noEmit` / `dprint check` | 均 exit 0（`dprint` 曾报 3 文件，其中 `WikiGraphPage.tsx` / `GraphView.tsx` 属**非本轮**的既存格式漂移，已一并 `fmt`） |
| 组件测试 | **11 passed**（7 → +4，全部新增用例覆盖 `partial_failure` 通道） |
| 全量 vitest | 跑三次：**2 failed → 1 failed → 0 failed**（149 文件 / **1706 passed**，第三次为空载复跑）。失败点恒在 `DemandDiscoveryPage.test.tsx`（OPC 域）：**单独跑 11 passed**；且该文件 `vi.mock("react-i18next")` 把 `t` 替换为 key 透传 ⇒ **根本不加载真实 locale**，与本轮改动零关联；其 `userEvent` 多步交互用例单跑即 34s，并发下超 15s 超时 ⇒ 判为**既存并发超时 flaky**，非本轮引入 |

> 已顺带排除两条**看似**同型的通道（**不是**缺陷，勿改）：
> - `PipelinePage.tsx:64/78` 的 `t("chat.workflow.pauseFailed") + ": " + msg` —— 已有本地化主文案，
>   `msg` 是技术详情，属「本地化文案 + 详情」的合法形态；
> - `SerenityScreeningPanel` 的 `emptyReason` —— 它是 **LLM 生成的内容**（「为什么没有候选」），
>   不是错误信息，原文展示正是用户要看的东西。


