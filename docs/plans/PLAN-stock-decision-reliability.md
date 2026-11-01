# PLAN：股票决策可靠性修复（紫金矿业 10-09 误判 + 大盘下行宏观链路 + 反思调度失效）

> 立项时间：2026-10-15（第一轮）；2026-10-16 第二轮增补。触发事件：紫金矿业（601899）2026-10-09 决策给出最高档买入（用户口述"极度买入"，UI 对应「买入/强烈买入」档），四周期中三档买入、主档长期，随后 4 个交易日连续回落。本文用**落库决策 JSON + 实际行情**复盘根因，给出分层修复方案。
>
> **第二轮增补范围（§1.4、R8–R12、对应修复项）**：① 把 9 月下旬以来**整个 A 股走低**（上证 -6.1%）综合进来，审计市场 regime / 宏观数据 / 市场宽度因子链路的共性缺陷；② 查明「按时反思」为什么没有自动激活——结论是候选池分析调度事实上死亡 + 反思建点/到期/重试四层断层共同造成，而非单一开关问题。
>
> 证据来源：PostgreSQL `axinvest.stock_analyses` 三条 live 记录（id 前缀 `4419bd01` = 10-09、`0c8735e7` = 10-10、`7fa10230` = 10-12），`stock_reflections` / `strategy_performance` / `cron_jobs` / `cron_job_history` 全表统计，东方财富前复权日 K（fqt=1，含上证指数 000001）。

## 一、事实链

### 1.1 实际走势（601899，前复权）

| 日期 | 收盘 | 涨跌幅 | 成交额 | 备注 |
|---|---|---|---|---|
| 10-08 | 29.37 | -1.51% | — | 节后首日 |
| **10-09** | **30.80** | **+4.87%** | **约 724 亿（阶段天量）** | **决策日，盘后 19:11 跑出结论** |
| 10-12 | 30.22 | -1.88% | — | 盘中 14:12 复跑 |
| 10-13 | 30.40 | +0.60% | — | |
| 10-14 | 29.98 | -1.38% | — | |
| 10-15 | 29.38 | -2.00% | — | 收盘已低于 10-08 |

决策后 4 个交易日累计约 **-4.6%**；30.8 上方正是 9 月高点 31.77 一线的套牢区，大阳线次日即遇阻。超短档（持有 2 日）、短期档（5 日）当即失效。

### 1.2 系统当时给出了什么（落库原值）

10-09（`decision_json` / `decisionsByHorizon`）：

| 档位 | 动作 | 置信 | 入场门 | 关键腿信号（[-1,1] 中心化口径） |
|---|---|---|---|---|
| ultra_short（2 日） | 买入 | 82.9 | breadth_required **PASS** | momentum +0.50（w46%）、breadth +0.683（w54%，=全市场封板率 84%）；microstructure 腿**未接线** |
| short（5 日） | 持有 | 73.0 | trend_and_flow **FAIL（资金持续性缺数）** | momentum +0.5、breadth +0.683、trend +0.54 |
| mid（28 日） | 买入 | 80.4 | inside_sigma_band PASS（\|z\|≤2） | trend +0.36、**valuation +0.831（w29%）**、expectation +0.595 |
| long（90 日，主档） | 买入 | 84.3 | positive_margin_of_safety **PASS（dcf_margin_20pct）** | **valuation +0.831（w36%）、earnings +0.778（w27%）**、expectation +0.595、macro +0.01 |

- 主结论：`action=买入`，目标价 38.73（+29.4%），止损 21.32（**-28.77%**），凯利原始仓位 40.2%，经风险预算压到 **5.3%**。
- 主链因子：f9 资金流 **+0.83**（大阳线当天主力净流入）、f12 动量 **-0.7**（超买，权重仅 7.5%）、f5 估值 +0.881、f2 共识 -0.1，posterior 0.558。
- **数据质量熔断已触发**：`dqiFuseState=fused`，连续 3 轮未达 A 级，展示置信上限 60（reasoning 原文："⚠️数据质量熔断…置信度上限压至60.0"）。
- 三次决策（10-09/10/12）四个档位 `priorSource` 全部 `neutral_default`、`priorSamples=0`。

10-12 复跑（股价已跌到 30.22）：long 买入 84.3、mid 买入 81.3、ultra **增持** 66.5，主链 f9 已转 **-0.73**、f12 -0.7、posterior 仅 0.51；**LLM trader 明确给「持有」，与公式「买入」方向相反**（`agreementBreakdown.conflictType=opposite_direction`，33 分），最终主结论**仍是买入**。

### 1.3 DCF 当时的输入（`result.t-valuation` 快照）

- FCF 锚 = **444.47 亿（最近完整年报，OCF−资本开支）**，FCF/股 2.157；
- 5 年增速 g=**14.43%**（近 5 年年报营收同比截尾均值，高档 21.6%），折现 7.7%，永续 1.3%；
- DCF 三档 38.67 / 59.23 / 82.89，保守档（low）上行 25.6%；**终值占企业价值 78.1%**；
- PE 自身历史分位 8%、同行分位 9%；反向 DCF 显示"现价隐含 FCF 年复合 **-1%**"（市场定价利润下行），被标 neutral；
- 节点自带使用禁令原文："三档…中性档实测可摆动 4.2 倍…**不得作为目标价/买入价/清仓线等定价依据**"。

### 1.4 大盘背景：9/21–10/15 上证 -6.1% 的下跌波段（第二轮新增）

紫金误判不是孤例，它发生在一段明确的全市场下跌中。上证指数（000001，东财日 K）：

| 日期 | 收盘 | 涨跌幅 | 备注 |
|---|---|---|---|
| 9-21 | 高 3967.68 | — | 波段高点，此后高低点同步下移 |
| 10-08 | 3811.90 | -0.79% | 节后首日 |
| **10-09** | **3813.79** | **+0.05%** | **盘中一度砸到 3754（-1.5%）后收回；紫金同日 +4.87% 属逆势个股异动** |
| 10-12 | 3797.59 | -0.43% | |
| 10-13 | 3801.35 | +0.10% | |
| 10-14 | 3767.85 | -0.88% | |
| 10-15 | 低 3674.82 | **-1.59%** | 波段低点，较高点 **-6.1%** |

系统动作侧的转向明显滞后：全量 live 决策分布到 **10-14** 才转为谨慎（当日 买入 6 vs 持有 9 / 减持 4 / 卖出 1，其中 18 条 `template_version` 为空），10-15 买入仅 1 条。也就是说，在指数已从高点回落约 5%、10-09 当天盘中还出现 -1.5% 急杀的背景下，系统对紫金仍输出四档三买入、long 84.3 的结论。**本节只陈述 K 线与落库动作两类事实，不对下跌做消息面/政策面归因。**

## 二、系统性根因（按对本次误判的因果权重排序）

### R1 估值方法对周期股结构性失效 —— 本次误判的主导项

紫金是铜金资源股，利润随商品价格周期大幅波动。当前链路对它的处理是教科书式"周期顶部价值陷阱"：

1. **FCF 锚取周期高位值**：用最近完整年报 444 亿 FCF 直接做基数（`crates/astock-data/src/valuation.rs:21-49` 二阶段 DCF，无周期中性化处理；全文件无 normalized / 多年平均利润逻辑）。
2. **增速再叠一轮乐观**：g=14.43% 取近 5 年营收增速截尾均值（`mcp_tools.rs:2828` 兜底 `DEFAULT_GROWTH=0.12`），把含商品上行周期的历史增速外推 5 年；终值占比 78%，永续假设轻微变动即可让估值摆动数倍（节点自陈"4.2 倍"）。
3. **PE 分位同向误导**：`valuation_band.rs:96-133` 的 PE band 用 TTM 盈利；周期高位 E 高 → PE 落在历史 8% 分位 → 输出"极度便宜"。DCF 与 PE band 两条估值腿同向 +0.83，在 long 档合计权重约 64%，一个人就决定了方向。
4. **反向证据被无视**：反向 DCF 已反推出市场定价 FCF 年复合 -1%，与正向 DCF 直接矛盾，但只挂 neutral，不推翻主口径。
5. **用途越界**：DCF 节点声明"不得作为目标价/买入价"，long 档却直接拿 DCF low 档当 90 天目标价（38.73）和安全边际入场门（`dcf_margin_20pct`）。**数据源自身的使用边界被下游违反，且无任何代码约束。**

### R2 "长期档"名实不符，估值锚与持有期限错配

`long` 档 `expectedHoldingDays=90`（3 个月），但方向腿 100% 是慢变量（估值/质量/预期/宏观），趋势只作 filter（落库 `trendStrength role=filter, w=0`）。DCF 衡量的是多年内在价值，90 天股价由资金与情绪主导（f9 在 3 天内从 +0.83 翻到 -0.73）。**用无短期预测力的变量对短期价格下方向判断**，再配 -28.8% 的"证伪"止损，等于"看估值买、可以亏 29% 等回归"——这不是 90 天策略，也不是真正的长期策略。

### R3 超短档证据过薄却输出 82.9% 高置信

- 仅两条方向腿：MACD/RSI 技术信号 + **全市场封板率**（breadth 与紫金个股毫无关系；10-09 全市场封板率 84% 给了它 +0.683、54% 权重）。
- 个股微观结构腿（涨停池/龙虎榜/盘口）`microstructure` **未接线**（落库 dataGaps 自陈"缺席是接线状态，不是盘面事实"），缺席没有降置信。
- 入场门 `breadth_required` 同样只验大盘情绪，不验个股。大盘情绪火热时，任何一只股票都能通过超短买入门。

### R4 反馈学习闭环实际失效 —— "可用性不高"被长期固化的根因

系统有完整的命中率/先验/反思设计，但在线决策读到的永远是"零样本中性先验"：

1. `strategy_performance` 仅 8 条 legacy 判定，`template_version` **全部 NULL**；代际筛选规则（`crates/harness/src/holding_period.rs:663` floor=125；`reflection_stats.rs:202-208`）把"代际未知"样本**全部排除** ⇒ 这 8 条不进任何统计。而它们的实际成绩是：平均收益 **-2.82%**、方向胜率 **3/8=37.5%**（低于抛硬币），系统对唯一存在的不利证据完全不可见。
2. 四周期胜负依赖反思 LLM：`stock_reflections` 90 条中 45 pending、约 18 条 failed（多次 **72 万 token 上下文超限**、原始分析缺失成孤儿、黑板快照缺失），逐档结果 `horizon_results_json` 仅 20 行有值。
3. long 档成熟判定要等 90 天 ⇒ 上线以来没有一条成熟 long 样本；`MIN_SAMPLE=5`（`reflection_stats.rs:24`）门槛下，先验只能回退 0.5 中性。
4. 结果：三次决策 `priorSamples=0`，`horizon_prior.rs` 的命中率收缩机制空转，系统**永远没有记忆**，同类错误可无限重复。

### R5 现有安全机制"只降展示数字，不降动作"

1. **DQI 熔断与动作脱钩**：熔断把展示置信压到 60，但选档走 `actionSource=branch_pick`，直接采用分支原始 84.3 出「买入」；`confidenceQualityCap` 只作用于展示口径（落库 `actionConfidenceBasis=84.3 / adjustedConfidence=78.5 / confidence=59` 三个数并存）。
2. **公式与 LLM 反向不降级**：10-12 已有 `opposite_direction`（33 分）、`isContradictory=false`，主链后验仅 0.51、资金/动量双负，仍无任何"反向即强制观望"的路径；后端在 `src/commands/stock_workflow/decision.rs:1867` 只打冲突标签，注释（同文件 :3029）明确仅用于前端显示"建议复核"。
3. **缺数惩罚不一致**：short 门因"资金持续性缺数"正确降级为持有，但同一份缺数在 long（根本不看资金）、mid（`flowPersistence` 缺席）不构成任何阻碍。

### R6 同步/滞后指标被当作开仓领先信号（追涨结构）

10-09 的全部看多技术读数——季线评分 83 多头排列、MACD 零上、f9 资金 +0.83、封板率 84%——都是 **+4.87% 放量大阳线收盘后的同步读数**。盘后据此出"买入"，次日在阻力位下方接力即被套。f12 超买 -0.7 是唯一的反向领先信号，权重仅 7.5%。mid 档 σ 带门 `|z|≤2` 覆盖 95% 的常态波动，时机过滤几乎不生效（10-12 时 bandZ=-0.97 仍 PASS）。

### R7 后验概率未经校准直接进凯利

分支后验（0.843）被直接当作胜率进 Kelly（原始 k=0.385、档位仓位 40.2%）。模型从未做可靠性校准（reliability curve / Brier），唯一存在的 8 个历史样本显示真实方向胜率仅 37.5%。风险预算把最终仓位压到 5.3% 避免了大亏，但**动作结论本身的错误没有被任何一层拦截**。

### R8 市场 regime 单指数、滞后，且无熊市硬门禁（第二轮新增）

- **判据本身滞后**：`crates/analysis-engine/src/market_regime.rs:30-132` 仅用沪深300近 60 日 K 的 MA60 / MA20 / 5 日斜率三条件判牛/熊/震荡。60 日均线对 -6.1%、持续约 3 周的下跌波段反应迟缓——指数跌破 MA20 后还要等 MA60 条件，regime 大概率在 10-14/10-15 才翻 bear，与"系统动作 10-14 才转向谨慎"的观测互相印证。
- **bear 只改先验、拦不住动作**：regime 对结论的影响仅是先验与因子权重（bull .55 / sideways .50 / **bear .45**）。后验 = 先验 + 证据加权，0.45 的先验在 valuation +0.83 这类强腿面前被轻易翻回 0.558，**下跌波段照样出 long 买入**。系统没有任何"熊市禁新开仓/降总仓位"的市场级硬门禁。
- **静默降级把"不知道"变成"中性"**：regime 注入失败时 hook 只给 `unknown` 让 LLM 自行判断（`src/commands/stock_workflow/hooks.rs:610-642`），而 `src/commands/regime-weights.rhai:57-58` 把 sideways 与 unknown 一律映射成 neutral。取数失败与"判定震荡"在下游不可区分，安全语义被悄悄抹平。
- 与本次误判的关系：10-09 大盘处于已确立的下跌波段中段、当天盘中还有急杀，而宏观/市场层没有任何一个机制能否决紫金的开仓动作。R5 是"个股安全机制不联动动作"，R8 是**市场级安全机制整体缺位**。

### R9 市场级量（封板率）为个股背书，ultra 档仍是方向腿（第二轮新增）

- 封板率（涨停家数封板比例，解析自 `crates/astock-data/src/vendors/ths.rs:664` `parse_limit_up_breadth`）是**全市场共享的一个数**：同一天所有个股拿到完全相同的 breadth 信号。`crates/harness/src/holding_period.rs:490-494` 注释自承认它是市场级量。它对"紫金这只票次日涨不涨"没有个股区分度，却在 10-09 以 84% 封板率给紫金 ultra 腿 +0.683、54% 权重。
- 10-09 的反讽在于：当天大盘盘中 -1.5% 急杀、全市场风险偏好实际在退潮，但早盘/前日残留的高封板率仍作为强看多信号喂给了每一只被分析的股票——**市场级情绪指标在拐点处最容易系统性误导，且误导是全市场同时发生的**。
- 修过一半：short 档自 2026-10-10 起已把封板率降为背景过滤（`crates/analysis-engine/src/evidence_weight.rs:1260`），但 **ultra 档仍把它当方向腿**（与 R3 联动：ultra 一共只有两条腿，它占 54%）。这是同一缺陷在档位间修了一半的状态。

### R10 宏观腿数据滞后半年却保留 18% 名义权重（第二轮新增）

- long 档 macroRegime 腿占约 18% 权重，但信号源是**月度低频宏观序列**：10-09 决策黑板（`result.t-macro-data`）中 PMI 50.1（腿信号仅 +0.01）、CPI/PPI 还是 8 月值、**GDP 停在 2026 Q1（4/20 发布，滞后约半年）**。该约束自带 `macroPartialSeriesOnly` 标注，等于自认"数据残缺、近乎无效"，却仍保留完整名义权重。
- 后果有两层：① 无效腿稀释有效腿的边际贡献（权重预算被占用）；② 更危险的是**它偶尔会非中性**——一旦某月 PMI 小幅回暖给出 +0.2 信号，就会在大盘实际下行时给 long 档添一个"宏观向好"的伪证据，而它衡量的东西与"未来 90 天股价"关系极弱。
- 取数失败同样走 unknown 静默降级（同 R8 的 hook 路径），缺数据与"宏观中性"不可区分。宏观腿与 regime 腿共同构成了系统的"大势判断层"，而这一层在本次下跌中既迟钝又无力。

### R11 候选池分析（pool-scan）调度事实上死亡 —— "反思没按时激活"的供给侧根因（第二轮新增）

反思的原料是逐只分析落库时写入的 pending 行，而原料供给端的 cron 任务在桌面端长期跑不完且不留任何痕迹。PG + 代码双向钉死：

1. **现象（cron_jobs 实存值）**：`pool-scan-daily`（schedule `0 2 * * *`）`run_count=2`，最后记录停在 2026-10-10 02:00 的「非交易日，本轮跳过」，`next_run_at=0`；10-12 至 10-15 `cron_job_history` **零行**（成功失败都没有）。对照组全部健康：`decision-backtest-daily` 10-15 07:01 正常跑（written=165）、`stock-reco-daily` / `trend-screening-daily` 10-15 05:18 重启补发、`reflection-due-daily` 10-14 18:13 也跑过——**调度器本身活着，唯独这个任务消失**。
2. **next_run_at=0 是重启恢复语义**：`crates/runtime-core/src/cron_job.rs:367-381` `resumed_next_run` 发现档期已过期即置 `Some(0)`，设计上"错过的档期补发一次"。它**不是**交易日闸门造成的——闸门分支（`src/init/services.rs:2241-2263`）跳过后会走 `record_run` 正常推进档期。
3. **死亡机制是四个设计的复合**：
   - 内置配置 `{lookbackDays:0, maxStocks:0}` = **全池、不截断**（`src/commands/pool_scan.rs:68-70`）；四档 × topN 长期累积后单轮候选可达数百只；
   - `analyze_candidate_queue`（`pool_scan.rs:412-469`）**串行**逐只跑完整 LLM 分析链，单只数分钟，整轮可达数小时；
   - `record_run` 只在全部跑完后调用一次（`services.rs:2790`），spawn 外层**没有超时**（`config.timeout_seconds=300` 无人消费）；
   - 桌面端每天关 app / 重启 ⇒ 长任务中途被杀 ⇒ 无成功行、无失败行、`next_run_at` 停在 0；调度循环的在途去重表（`crates/runtime/src/cron/scheduler.rs:47,77,125-130`）在本进程内还会持续压制重复派发，直到下次重启再从头再来。
4. 全池积压下"本轮在关机前跑完"是小概率事件，于是任务**事实上永久不产出**，而任务面板只显示一条 10-10 的"跳过"，运维侧完全不可见。候选池→逐只分析环节断流，反思队列自然无米下炊。

### R12 反思覆盖的四层独立断层 —— "按时反思"即使任务活着也产不出有效复盘（第二轮新增）

1. **建点门槛**：`reviewable_horizons` 只为方向动作（Buy/Increase/Sell/Reduce）建反思点（`src/commands/stock_workflow/reflection.rs:2141` 附近），持有/观望永不复盘；且统一门槛 `DEFAULT_REFLECTION_MIN_CONFIDENCE=70`（`reflection.rs:2091` 附近），低分决策也永不复盘。
2. **历史无点**：反思点随 10-14 的 v154/v156 模板才批量铺开（当天 46 行）；此前 live 分析在 `stock_reflections` 几乎零行（10/01–10/13 仅 10-11 有 2 行）。紫金三条分析（10-09/10-10/10-12）**至今没有任何反思行**——正是"误判后系统毫无自我反馈"的直接原因。
3. **到期口径用自然日**：live 通道按自然日算 hindsight（`reflection.rs:2252-2256` 附近），ultra 10-16、short 10-19、mid 11-11、**long 落到 2027-01-12（90 自然日）**。当前 45 条 pending 全部未到期，`reflection-due-daily` 10-14 跑时 `totalPending=0`、12ms 结束——任务"成功"了，但是空转。T+N 复盘应按交易日 bar（T+2/T+5/T+28/T+90 交易日），自然日口径让长档样本成熟额外推迟约 40%，且长假后失真。
4. **失败即终局**：约 16 条 failed 永久失败、无死信重试——4 条 LLM 上下文超限（727019/730002/629477/562181 tokens 撞 524288 窗口；根因已于 10-14 提交 c247e03e2 的三层上下文预算修复，但**旧失败行不会自动重跑**）、5 条 `ANALYSIS_SNAPSHOT_MISSING`、6 条 7 月孤儿；running 仅靠 `STALE_RUNNING_HOURS=6`（`reflection.rs:2462` 附近）回收。
5. **兜底轮造成"反思在跑"的假象**：10-14 以来实际产出 resolved 的是 `start_batch_reflection` 的**每 6 小时系统兜底轮**（`src/init/services.rs:4032` 注释；走 `record_auxiliary_run`，`crates/runtime-core/src/cron_job.rs:653-692`，只落历史不推进档期），不是 `reflection-due-daily` 档期任务。兜底轮消费的是同一批 pending，到期口径不变，因此它无法纠正第 3 条的空转，只掩盖了"档期任务无事可做"的可见性。
6. 与 R4 的关系：R4 讲"学习闭环读不到样本"（先验侧），R11/R12 讲"样本为什么生产不出来"（供给侧），三者合起来解释了用户感知的"整个反思机制名存实亡"。

## 三、修复方案

原则：先止血（让错误结论发不出来），再治本（让系统长记忆、信号有检验），最后体系化。每步小改、可独立验证，不做整体重构。

### P0 止血（建议 1–2 天）

| # | 改动 | 位置 | 验收 |
|---|---|---|---|
| P0-1 | **熔断联动动作**：DQI 质量 cap 生效时，动作阶梯使用 `min(分支置信, cap)`；cap=60 时全档最高只出「持有/观望」，禁止买入/增持 | `stock_workflow` branch_pick 选档处 + 四档 rhai 动作判定 | 单测：cap=60、branchConf=84.3 ⇒ action≠买入 |
| P0-2 | **双视角反向硬降级**：`conflictType=opposite_direction` 时强制 action=观望（或取两者中更保守者），banner 显红并禁止通知推送"买入" | `stock_workflow/decision.rs:1867` 附近 | 用 10-12 快照回放 ⇒ 结论变观望 |
| P0-3 | **DCF 用途边界落地**：`dcf_target`/`dcf_margin` 门仅允许在真正长期档（≥180 天）使用；90 天 long 档要么持有期改为 ≥180 天并改名为长期，要么移除估值方向腿（降为背景），mid 档估值腿权重 29%→≤10% | `evidence_weight.rs:487-519` 分支表 + rhai 门 | 重跑 10-09 快照：long/mid 不再仅凭 DCF 出门 |
| P0-4 | **周期股估值开关**：行业 ∈ 有色/煤炭/钢铁/石化/航运/养殖/券商 时：① DCF FCF 锚改用近 5 年 FCF 中位数（周期中性化）而非最近年报；② PE band 信号置空（不进方向）；③ 安全边际门阈值 20%→35%；④ 反向 DCF 隐含 g<0 时禁止"合理偏低"结论 | `astock-data/src/valuation.rs`、`valuation_band.rs`、行业分类映射 | 紫金 10-09 重放：valuation 腿不再 +0.83 |
| P0-5 | **超短档降杠杆**：microstructure 个股腿未接线期间，ultra_short 最高只出「持有」；封板率腿降为纯门（市场情绪差时否决，情绪好时不加分），与 R9 一并修复；breadth 门增加"个股近 5 日涨停/强势股"前提 | `evidence_weight.rs:292-365` ultra 列 + `portfolio-mgr-h-ultra-short.rhai` | 紫金类非涨停大盘股 10-09 ⇒ ultra≠买入 |
| P0-6 | **救活 pool-scan 调度**（R11）：① 单轮加硬上限（默认每轮 ≤20 只，剩余靠"当日未分析"判据次日续跑，废弃全池一轮跑完语义）；② 外层 `tokio::time::timeout` 加**单轮墙钟预算**（入 PoolScanConfig，按 20 只 × 单只预算估，默认约 30–40 分钟；现 `config.timeout_seconds=300` 与批量任务不匹配，不复用它），超时按"部分完成"如实 record_run（success=false/partial + 已完成明细），杜绝零行；③ 断点天然存在——每只完成即落 `stock_analyses`，重启/次日靠"当日已分析"判据（`pool_scan.rs:261-272`）续跑，无需另建检查点表；④ 启动恢复遇到 `next_run_at=0` 时，若今日已有成功批次则直接重排次日、不重复派发 | `src/commands/pool_scan.rs:68-70,393-469`、`src/init/services.rs:2704-2807`、`crates/runtime-core/src/cron_job.rs:367-381` 恢复分支 | 构造 50 只候选 + 60s 预算 ⇒ 历史出现部分完成行而非零行；重启后下一轮续跑剩余且不重复分析 |
| P0-7 | **熊市市场级硬门禁**（R8）：regime=bear 时动作层强制——禁止新开仓（买入/增持只允许对已持仓加仓的保守子集或直接禁止）、总风险预算下调一档；regime=unknown 取数失败时按"未判定=不加仓"，不得当 neutral；门禁作用于动作而非展示（避免重蹈 R5） | `market_regime.rs:30-132` 判定 + `hooks.rs:610-642` 注入 + branch_pick/风险预算处 | 单测：bear + 分支买入 84.3 ⇒ 动作≤持有；unknown 不产出买入 |
| P0-8 | **failed 反思死信重试**（R12-4）：给 failed:* 行增加可重跑状态与重试命令（复用既有 pending 消费通道）；4 条上下文超限行在三层预算修复（c247e03e2）之后可安全重试，优先重跑；重试次数/last_error 落库可观测 | `stock_workflow/reflection.rs` 失败落库处 + 批量反思入口 | 4 条超限行重跑后 status=completed；重复执行幂等 |

### P1 治本（建议两周内）

| # | 改动 | 要点 |
|---|---|---|
| P1-1 | **修复先验样本供给** | ① 写 `strategy_performance` 时补 `template_version`，8 条 legacy 回填（或显式标代际）；② 胜负判定增加**纯确定性 M1 路径**：到期按后复权收盘价直接算 return/was_correct，不依赖反思 LLM；③ 各档按自身持有天数（2/5/28/90）独立到期判定，ultra/short 先快速积累样本；④ 先验取数对 M1 样本与 LLM 反思样本分开计数 |
| P1-2 | **反思 LLM 瘦身** | 上下文只传决策 JSON + 逐档腿信号 + 区间行情摘要（目标 <30k token），禁止把整块黑板快照塞 prompt；失败行可重试/补判，消灭 72 万 token 超限 |
| P1-3 | **概率校准** | M1 样本 ≥30 后，按档位做置信度→实际胜率的可靠性校准（isotonic/Platt），后验进 Kelly 前过校准映射；置信标签改为"该置信档历史胜率 xx%"；样本不足时置信上限 60 |
| P1-4 | **追涨防护** | 当日涨幅 >4% 且成交额 >90 日 90 分位（10-09 两个条件都满足）时：① 禁止盘后直接出开仓动作，改为"次日回踩确认"；② 增加阻力位（前高/跳空缺口/套牢密集区）否决项；③ f12 超买在 ultra/short 档升权（7.5%→≥15%），σ 带门从 \|z\|≤2 收紧到 \|z\|≤1 |
| P1-5 | **缺数统一语义** | 任何档位方向腿缺席：该腿信号按 0（中性）计权但同步扣减 evidence_scale 并在置信上体现；关键门缺数一律按"未判定=不加仓"（沿用 short 档已验证的正确做法），消除档间不一致 |
| P1-6 | **为历史 live 分析补建反思点**（R12-2） | 用离线 bin `src/bin/axagent-batch-rerun.rs` 既有的 `build_pending_reflection_rows_for` 通道，为 10 月（v154 前）历史 live 分析批量补建 pending；紫金三条（10-09/10-10/10-12）优先，作为修复后第一批真实验证样本；补建行带独立 source 标记便于回溯 |
| P1-7 | **ultra 档封板率降纯背景门**（R9） | 对齐 short 档 2026-10-10 的改法（`evidence_weight.rs:1260` 同款）：市场情绪差时可否决，情绪好时不加分；ultra 方向腿在个股 microstructure 接线前以动量/技术腿为主，证据条数不足时置信上限同步压低（与 P0-5 配套） |
| P1-8 | **宏观腿时效门禁**（R10） | 给宏观序列加发布日期与最大滞后阈值：GDP 滞后超 1 季度、CPI/PPI/PMI 超 1 个月时该腿权重置零并在黑板显式标注 `stale`，不得保留 18% 名义权重；`macroPartialSeriesOnly` 约束触发时权重同样置零；取数失败与"中性"在落库字段上可区分（配合 R8 的 unknown 语义） |
| P1-9 | **反思到期改交易日口径 + 扩大复盘覆盖**（R12-1、R12-3） | hindsight 到期改用交易日 bar（复用 `axagent_astock_data::calendar`，T+2/T+5/T+28/T+90 交易日），与决策回测 T+N 口径统一；持有/观望动作按抽样（如每档每日抽 20%）建反思点，置信<70 的错误方向不再被门槛整体隐藏；存量 45 条 pending 的 hindsight_date 按新口径重算 |
| P1-10 | **调度可观测性**（R11） | 任务在途时长/派发后未 record_run 的时长进入任务面板：next_run_at=0 超过 N 小时未推进即标异常；批量任务结束态区分 success / partial / failed / skipped；`record_auxiliary_run` 兜底轮在 UI 上与档期触发显式区分，避免"6h 兜底轮在跑"掩盖档期任务空转 |

### P2 体系化（后续迭代）

- **信号上线前必须过预测力检验**：复用 `hit_rate_backtest` / 回测骨架，每个因子腿按档位跑历史 rank IC 与方向胜率；IC 不显著（含置信区间跨 0）的权重置零；估值腿需按持有期限分别验证（大概率证实"估值对 2–28 日无预测力"）。
- **失败模式结构化归因**：把本次识别的"周期顶部 DCF 乐观 / 大阳线追涨 / 大盘情绪替个股背书 / 反向 DCF 被无视"写成反思模板枚举，`miss_layer` 统计上看板，月度复盘。
- **Banner 信息对称**：展示动作时同屏显示依据档位、cap 原因、双视角冲突、该档历史命中率与样本数，避免单个"买入"词汇造成强误导。
- **长期档正名**：要么做真长期（≥180 天、接受大止损、估值驱动），要么把 90 天档改名为"季度"并以资金/趋势腿为主，估值只作风险过滤。
  ⚠ **后一路（改名为「季度」）已于 2026-10-16 当日晚回退、判为错误**：前提「90 天 = 一个季度」把**交易日**当成了自然日。
  实测 90 交易日 = 18 周 ≈ 4.2 自然月 ≈ 1.5 个「尺度季度」（`astock-data/src/scale.rs` 定义 1 季 ≈ 60 交易日/bar）
  ⇒「quarterly」是**评分尺度名（bar 粒度）**、不是持有期名，用它会造出「季度 90 天」这类自相矛盾文案。
  取证与全量清单见 `AUDIT-horizon-naming-2026-10-16.md`。真长期（≥180 天）这条**仍未做**，见 P2 其它项。
- **regime 多指数 + 市场宽度融合**（R8）：沪深300 单一指数扩为沪深300/中证全指/上证 50 多指数确认，并引入市场宽度（涨跌家数比、新高新低差、站上 MA20 占比、封板率的**变化率**而非水平值）；60 日均线体系之外加入波段识别（高低点同步下移，可用现有波段口径计算），把"确认熊市"的滞后从数周压到数日。
- **小样本先验改收缩而非整批丢弃**（R4/R12）：`strategy_performance` 样本量在 FLOOR=125 之下时，用 Beta 收缩先验（向 0.5 收缩强度随样本量变化）而非把代际未知样本整批排除，让 3/8=37.5% 这类早期不利证据至少部分可见，同时避免小样本过拟合。

## 四、本次个案处置建议

1. 10-09/10-12 三条「买入」结论在修复上线前应视为**不可采信**；短期档（5 日）已到期亏损，建议在系统标记该批结论失效，避免推送/策略继续引用。
2. 紫金个案本身：短线已跌破决策价 30.80 与 10-08 平台，技术面走弱；中长期争议点是铜金价格周期位置，不应由当前未中性化的 DCF 给出 38.73 目标价，修复后重跑再评估。
3. 紫金三条 live 分析（`4419bd01`/`0c8735e7`/`7fa10230`）当前在 `stock_reflections` 零行（R12-2）。建议在 P1-6 补建通道上线后**第一批补建 pending**：ultra/short 已到期、mid 临近，可立即验证反思链端到端是否打通，并为"大盘下行期误判"留下首个完整复盘样本。
4. 运维侧即时处置（不改代码即可做）：在定时任务面板对 `pool-scan-daily` 做一次手动触发验证存活；`next_run_at=0` 可通过"停用再启用"或保存一次任务配置强制重算档期；4 条上下文超限 failed 行在 P0-8 上线前先人工标记，避免污染统计。

## 五、附：关键证据索引

- 决策落库：`stock_analyses` id `4419bd01-…`（10-09）、`7fa10230-…`（10-12），字段 `decision_json` / `blackboard_snapshot`。
- 后验融合：`src/commands/portfolio-mgr-h-ultra-short.rhai:163`、`portfolio-mgr-h-mid.rhai`、`portfolio-mgr-h-long.rhai`（`posterior = prior + avg * evidence_scale`，信号口径契约见 `crates/analysis-engine/src/leg_signal.rs:44`，为 [-1,1] 中心化，公式本身自洽，问题在信号源）。
- 证据缩放：`crates/analysis-engine/src/portfolio_formula.rs` `compute_evidence_scale`（全腿在场上限 0.55）。
- 四档权重/桥表：`crates/analysis-engine/src/evidence_weight.rs:292-365,367-405,487-519`。
- 先验：`crates/analysis-engine/src/horizon_prior.rs:1-23,59-95`；代际 floor：`crates/harness/src/holding_period.rs:663`；样本门槛与三表取数：`crates/analysis-engine/src/reflection_stats.rs:24,202-208,739-751`。
- DCF：`crates/astock-data/src/valuation.rs:21-49,124-220`；参数常量 `crates/astock-data/src/mcp_tools.rs:2807,2817,2828`；PE band `crates/astock-data/src/valuation_band.rs:96-133`。
- 冲突标签：`src/commands/stock_workflow/decision.rs:1867,3029`；落盘 `core.rs:1140-1216`。

第二轮（大盘宏观链路 + 反思调度，2026-10-16）：

- 大盘数据：东方财富上证指数 000001 日 K（fqt=1）；动作分布取自 `stock_analyses` live 行按日聚合（10-14 买入 6 / 持有 9 / 减持 4 / 卖出 1，18 行 `template_version` 为空）。
- regime 判定：`crates/analysis-engine/src/market_regime.rs:30-132`（沪深300 60 日 K + MA60/MA20/5 日斜率；prior bull .55 / sideways .50 / bear .45）；注入与失败降级 `src/commands/stock_workflow/hooks.rs:610-642`；权重映射 `src/commands/regime-weights.rhai:57-58`。
- 封板率链路：解析 `crates/astock-data/src/vendors/ths.rs:664`；市场级量自承认 `crates/harness/src/holding_period.rs:490-494`；short 档已降背景 `crates/analysis-engine/src/evidence_weight.rs:1260`；10-09 实喂值 +0.683 / w54%（见 `4419bd01` 落库腿信号）。
- 宏观数据时效：`4419bd01` 黑板 `result.t-macro-data`（PMI 50.1、CPI/PPI 为 8 月值、GDP 停 2026 Q1、约束 `macroPartialSeriesOnly`）；long 档 macroRegime 权重见四档桥表 `evidence_weight.rs:487-519`。
- 调度三件套：档期口径 `crates/runtime-core/src/cron_job.rs:291-381`（`calculate_next_run` / `resumed_next_run` 过期置 0）、记账推进 `cron_job.rs:621-651`（record_run）、兜底轮专用 `cron_job.rs:653-692`（record_auxiliary_run）；派发循环与在途去重 `crates/runtime/src/cron/scheduler.rs:39-130`。
- executor 接线：交易日闸门 `src/init/services.rs:2219-2265`（skip 也 record_run + return）；pool-scan 分支 `src/init/services.rs:2695-2807`（record_run 仅在分支末尾 :2790）；6h 兜底轮注释 `src/init/services.rs:4032`；内置任务种子「存在即跳过」`src/init/stock_automation_cron_seed.rs:55-73`（排除种子覆盖嫌疑）。
- pool-scan 执行体：默认全池配置 `src/commands/pool_scan.rs:62-71`；串行队列 `src/commands/pool_scan.rs:393-469`；「当日已分析」断点判据 `src/commands/pool_scan.rs:240-272`。
- 反思链路：建点动作白名单 `src/commands/stock_workflow/reflection.rs:2141` 附近；置信门槛常量 `reflection.rs:2091` 附近（DEFAULT_REFLECTION_MIN_CONFIDENCE=70）；自然日到期 `reflection.rs:2252-2256` 附近；陈旧 running 回收 `reflection.rs:2462` 附近（STALE_RUNNING_HOURS=6）；补建通道 `src/bin/axagent-batch-rerun.rs`（build_pending_reflection_rows_for）。
- DB 实证（PostgreSQL axinvest，2026-10-16 查询）：`cron_jobs` pool-scan-daily run_count=2 / next_run_at=0 / last_result=「非交易日（2026-10-10），本轮跳过」；对照 decision-backtest-daily 10-15 07:01 written=165、reflection-due-daily 10-14 18:13 totalPending=0、reco/trend 10-15 05:18 重启补发；`cron_job_history` pool-scan 10-12 起零行；`stock_reflections` 45 pending 全未到期、约 16 failed（4 条 token 超限 / 5 条快照缺失 / 6 条 7 月孤儿）、紫金三条 live 零反思行；`strategy_performance` 8 行 template_version 全 NULL（胜率 3/8、均收益 -2.82%）。

## 六、执行状态（2026-10-16 收口轮）

### P0 止血 —— 代码 8/8 已落地（cfb0f60fe + a4782e9d6 升版 v157，已提交）

- P0-1 … P0-5、P0-7：代码 + 档位级测试已入库（见各文件注释引用）；**运行验证待新决策产出**（现网 175 条分析的 decision_json 中 R-REGIME-GATE / R-DQI-CAP / preConflictAction 均为 0 条，全部产生于修复前）。
- P0-6：①②③ 已落地（max_per_run=20 / 40 分钟墙钟 / 部分完成 render）；④「读今日成功批次直接重排」未实现 —— 防重复由「当日已分析」判据兜住（补发只会空跑一次）。**运行验证：重启后 run_count 仍 2、next_run_at 仍 0，补发未发生（待观察）**。
- P0-8：命令已入库；本日已执行重试（UPDATE 5：4 条 token 超限 + 1 条 LLM 500，结构性终局 11 条不动）——5 条已置回 pending，**待下一轮反思消费后闭环**（尚未 completed）。

### P1 治本

- P1-1：①写侧补 template_version ✅；legacy 8 条以「代际未知桶显式计数」达成（**回填不可行**——strategy_performance 无 analysis_id 列，无可靠关联）；②M1 判定已确定性化 ✅（was_correct 由行情快照反推），但物化仍挂反思完成 —— 「到期即物化」解耦评估后延后；③各档独立到期判定 ✅；④M1/LLM 分计数 = ② 的前置，随解耦一并延后。
- P1-2：✅ 三层预算 12 万 → 3.6 万字符；新增「非保护节点优先淘汰」（硬引用保护名单 8 节点）+ 占位保留 `__untrusted` 哨兵；4 条新测试。
- P1-4：✅ 主链 R-SURGE-GATE（>4% + 90 分位 ⇒ 开仓压持有）+ R-RESISTANCE-GATE（前高 5% 带）+ R-F12-OVERBOUGHT（RSI>75 且动量负 ⇒ f12 权重下限 0.15）+ mid 档 σ 带 |z|≤2→≤1；测试：surge_state 4 单测 + 主链 2 集成测试。
- P1-6：✅ 已执行 —— 31 条补建（紫金三条 6 行 pending，source=backfill，交易日口径 hindsight）；注：紫金 ultra 档在落库 horizon_decisions 中缺席（装配口径 A），该档无行可建。
- P1-8：✅ pm_macro_stale_reason（GDP>3 月 / CPI·PPI·PMI>1 月 ⇒ 腿按缺席）+ 映射 5 键 + h-long 门 + 5 单测；齐备态夹具同步。
- P1-9：✅ 交易日口径统一（live 同 as_of）+ 持有/观望 20% 确定性抽样 + 存量 42 条重算（幂等复跑 0 改写）。
- P1-10：✅ 前端（partial 徽章 / next_run 停滞标记 / 兜底轮 trigger=system）+ 后端（TaskRunResult.partial，33 构造点补全 + DTO）。

### P2 体系化

- P2-c：✅（依据档位标签上 live banner + 「该档命中率」全链：后端 hitRate → 四脚本 priorHitRate → 前端渲染）。
- P2-d：❌ **改名为「季度」已于 2026-10-16 当日晚回退**（原记为「✅ 长期档正名『季度』」）。理由与回退面：
  - **前提错误（单位）**：`Period::default_holding_days()` 的单位是**交易日**（`holding_period.rs:309` 明示），
    90 交易日 = 18 周 ≈ **4.2 自然月 ≈ 1.5 个季度**；而 `Scale::Quarterly` 定义 **1 季 ≈ 60 交易日/bar**
    （`astock-data/src/scale.rs:49,106`）⇒ 90 ≠ 60。`quarterly` 是**尺度名（取数/评分粒度）**，
    把它当**持有期档名**，等于把「持有 90 交易日」读成「持有 1 个季度」。
  - **事故形态**：① `stockAnalysis.scheduledAnalysis.reflectionScanHint` 出现同句互斥的
    「超短线 2 天 / 短线 5 天 / 中线 28 天 / **季度 90 天**」；② 后端 `HorizonAgreement.tierLabel`
    （`decision.rs:1926` 取 `Period::label_zh()`）与 `dataGaps`（`portfolio-mgr.rhai:3435` 的 `cn_tier`）
    显示「长线」，前端 i18n 显示「季度」⇒ **同屏两名**。
  - **回退面**：11 语言 × **8 套档名键族**（264 处；同时消掉 ar/hi/ja 等语言的**未翻译英文占位**，
    如 `Short | Short | Mid`）+ 3 类文案（33 处，并**去掉手抄天数**）+ 前端 3 个测试文件断言
    + `intent_parser` 补「季度」为**输入别名**（注明非展示名）。
    ⚠ 这「8 套」**只统一了值、定义仍是 8 份**；其中 7 套已于同日第二批**整体删除**（见下「收敛」）。
  - **权威同步**：`Period::label_zh()` 第 1 档由「超短」改为「**超短线**」，使 Rust 权威 ≡
    Rhai `cn_tier` ≡ i18n（**当时** 8 键族）≡ 导出 `labelMap` **逐字一致**（此前四处三种拼法）。
  - **新增门（首版判据）**：`scripts/check-horizon-label-parity.mjs`（判据 A 族间逐字一致 / B 禁尺度词 /
    C 与 `label_zh()` 逐字对账；含 5 正控 + 负控的自检，已注册进 `scripts/ci-check.mjs` 与 `.github/workflows/ci.yml`）；
    并**恢复** `check-reco-horizon-parity.mjs` 门 i 的严格性 —— 它当时被放宽成认「长线|季度」两种写法
    （**改判据去适配改名**），现只认「长线」且**显式禁「季度」**（新增独立负控证明该判据具区分力）。
    ⚠ 判据集已于同日第二批改为 **B / C / D / E / F**（A 随收敛撤销，因已无「族间」可比）。
  - **收敛（同日第二批；用户裁定「只统一了值、未做收敛任务」）**：把 **7 套**冗余族**整体删除**
    （28 键 × 11 语言 + 2 个被删空的壳；每语言 diff = **33 删 / 1 增**，那 1 增是 `zoomHint` **逗号归正**，
    合计 363 删 / 11 增；28 条路径已回灌删除前备份自证**确实曾存在** ⇒ 判据非空过）。
    消费端 **6 处**改走新增的单一存取器 `horizonLabelKey()`（`src/lib/stock-analysis-utils.ts`）；
    `InvestTaskFields.tsx` 两张只差键前缀的同形表**并成一张**。
    门禁随之由「族间一致 A」换成「**唯一族 + 禁回流 E**」（`RETIRED_FAMILIES` 28 条路径 + 2 壳须全部不存在）
    + 「**四档互异 F**」，自检 **8 正控 + 1 负控**，正式跑**扫描点 1433 → 67**。
    `check-horizon-weight-parity.mjs` 判据 ④ 的读取路径同步迁移（不迁即恒定 `undefined` ⇒ 假红/假绿）。
  - **收敛连带修复（超出本项范围，已修）**：`check-i18n-key-parity.mjs`（2026-09-21 加）与
    `check-i18n-placeholder-usage.mjs`（2026-10-01 加）**只进了 CI、没进本地镜像**，而
    `src/__tests__/ci.workflow.test.ts` 的「同步护栏」当时只守 2026-09-14 那三道 ⇒ 近一个月「CI 有、本地无」
    而**无任何断言红**。已补进 `scripts/ci-check.mjs` 并把护栏扩到五道（命令行与 `ci.yml` 逐字同源，
    阴性对照自证断言非空转）。**其中 ④ key-parity 正是证明「33 键 × 11 语言删得对称」的那道门。**
  - **取证**：`AUDIT-horizon-naming-2026-10-16.md`（§十 = 方案 A 落点；**§十一 = 收敛批次**）。
- P2-f：✅ 代际未知样本改「收缩纳入」（`filter_by_generation_floor` 的 None 不再排除；字段更名 `unknown_generation_samples`，**仅决策链**——荐股链与 evolution 权重重算链的同名字段经排查确认语义未变、保持原名与方法）；两处面板文案同步（hitrateGenerationScope 改「按收缩纳入」；evolutionDrift 保持原样）。
- P2-a / P2-b / P2-e：❌ 未动（后续迭代；a 依赖样本积累(现网 completed 33 条)、b 需失败模式分类裁定、e 需多指数/宽度数据源普查）。

### P2-e 预备（2026-10-16，数据面普查完成）

- **现役判定**：仅 `000300`（`hooks.rs` 的 `get_klines("000300","daily",60)` → `classify_regime`：MA20/MA60/5 日斜率三分支，市场级量）。
- **多指数取数就绪度**：`to_em_secid` 支持**直通 secid**（`1.000985` / `1.000016` / `1.000300` 原样通过）⇒ 多指数只需 ① hooks 换取数代码 ② `is_benchmark_code` 白名单扩「复权短路」判定。⚠ 本机 curl 对 push2his 域不通（000/连接层），**可取性以应用内实测为准**（应用现役路径稳定取 000300）。
- **宽度数据缺口**：现成仅「涨停/跌停家数 + 封板率」（ths 盘面情绪）；**涨跌家数比 / 新高新低 / 站上 MA20 占比无现成通道**（东财全市场扫描有 push2 速率触发 RST 的既有约束）；「封板率**变化率**」还缺历史序列载体（无每日快照表），暂不可派生。
- **判据草案（待裁定后再上线）**：每指数 bear 票 = `classify_regime`=bear，或**波段下移**（Dow 判据：近 60 根内最近两组局部高点与低点同步下移——弱信号，置信取 0.6）；三指数 ≥2 bear 票 ⇒ bear、≥2 bull ⇒ bull、否则 sideways。⚠ 判据会改**全链先验**与 **P0-7 门禁的触发面** ⇒ 按 P2-a「信号先过预测力检验」的原则，先裁定后实施。

### 运行验证与门禁（本日实测）

- 批 1（反思链）`cargo check --bin` 13m43s 全绿；批 2 全量 `cargo check --workspace --all-targets` 首发暴露 3 处编译错（TaskRunResultDto 两处漏补 + Option 的 `?` 误用）→ 修复后主 crate `--all-targets` check 13m42s **全绿**。
- `axagent-analysis-engine` 库测试 **688/688 绿**（含 P2-f 的 filter 语义与 IPC 键名同步）。
- 主 crate 新增测试 **7/7 绿**：`macro_stale_reason_tests` 5 条（P1-8）+ `portfolio_mgr_surge_gate_caps_open_on_big_up_bar` / `portfolio_mgr_f12_overbought_weight_floor_activates`（P1-4 主链门）；`axagent-harness` 的 `surge_state` 4 条单测同批绿。
  - 修复过程两处（首跑暴露、均已修）：① R-F12-OVERBOUGHT 首版把 `decision_trail.push` 写在其定义行之前 ⇒ 前向引用落 catch（复现 R-200~R-204 的历史教训）→ 整段迁移到追溯链定义之后；② surge 测试夹具缺 DQI 良好形态 ⇒ 买入先被 R-DQI-CAP 压成持有、本门看不到开仓动作 → 夹具补 `dqi_grade=A` 等四项。
- `axagent-harness` 库测试 799 过 / **2 红**：`recent_strength_uses_only_last_lookback_window` 与 `recent_strength_refuses_degenerate_inputs` —— 经与 HEAD 逐字比对，**该实现与两条测试在 HEAD 与本工作树完全相同**（测试与实现互矛盾的既存基线红，非本批引入；影响面 = `pm_recent_strength` 的门输入口径，待裁）。
- 前端：`tsc --noEmit` 0 错；dprint 定向 check 0；i18n parity 门 11 语言一致（含新增 5 键与两处文案更新）；受影响 vitest（TasksCenter 31 + DecisionBanner 19 + 档位名 19+12 + 反思/进化 15）全绿（4 处文案断言随 P2-d 同步 + 2 处测试名）。
  - ⚠ **该读数已被 2026-10-16 下午的回退覆盖**：P2-d 的「档位名 19+12」断言随后按回退事实再改一次（第 4 档 `季度 → 长线`、第 2/3 档 `短期/中期 → 短线/中线`），见下文「P2-d 回退批」。
- `cargo fmt --all --check` 0 处（先定点修了 3 文件的 4 处格式漂移）。
- 定点 clippy（`-p axagent -p axagent-harness -p axagent-analysis-engine -p axagent-runtime-core --all-targets -- -D warnings`）**全绿**（首跑 1 处 lint：`byte_char_slices` @ reflection.rs 抽样函数 → 已修，复跑 19m34s 通过）。
- 模板升版 **v157 → v158**（P1-4/P1-8/P2-c 的 .rhai 与映射变更；重启应用后播种生效）。
- 执行侧：反思重算 42 条（幂等复跑 0 改写）；历史补建 31 条（紫金三条 6 行）；死信重试 5 条置回 pending（**尚未被消费**，待下一轮反思）。
- **未提交**：本批改动与并行会话的未提交改动同树共存（共享工作树纪律：只改不提交，待统一切分）。

### 偏差与新增待裁定（追加编号）

16. P0-6④「读今日成功批次重排次日」→ **结案（不实施字面版）**：防重已由「当日已分析」判据达成，补发一次的代价 = 候选查询 + 全跳过，秒级空跑；字面实现的查库穿透（store→history）收益不抵复杂度。
17. P0-7 的 `regime-weights.rhai` unknown→neutral 权重映射 → **结案（按既有裁定保留）**：动作层硬门禁已兜（unknown=不加仓），且 risk_budget 段有明文裁定「unknown 不二次惩罚（取数失败不应二次惩罚已有持仓的风险计算）」——权重层保持 neutral 与该裁定一致。
18. P1-4③ 按主链全局超买条件实现 → **结案**：主链权重无「档」上下文；分支档的 momentumSignal 归一化占比本就 ≥15%（ultra≈56%、short≈28%），分支侧无需再升。
19. P1-6 补建空名单文案 → **已修**（bin：空名单时改打印「无可建复盘档（四档全为中性且未抽中…）」并计 false，不再打印误导性的空「补建 pending 」）。
20. 紫金 ultra 落库「缺失」→ **误报已更正 + 真缺陷已修**：`decisionsByHorizon` 的生产键是 **camelCase**（`ultraShort`，全库 100/100，snake 0 条）；旧 `reviewable_horizons` 只取 snake 键 ⇒ **ultra 档在逐档路径系统性 miss**（现网 ultra 反思行仅 6 条、且全部来自「主档恰为 ultra」的兜底路径，逐档路径一条都没有）——已改双键取（`as_str().or_else(camel_key())`）+ 加防回归测试（camel 夹具）。
21. harness 2 条红测试 → **已裁并修**：以实现为准 —— ① 脏端点对**整对跳过**（0 不是有效前收）⇒ days=0；② lookback=3 交易日 = **4 个收盘点** ⇒ cum 基点 11.2（原期望 days=3 与基点 11.1 自身互斥）。两条测试的旧期望与实现矛盾且不自洽，已按实现订正期望 + 注释写明判据。
22. 89d33856「空决策」→ **根因查明**：v157 脚本引用**未定义变量 `regime_is_bear`**（HEAD 实查：仅一行使用、无定义）⇒ 工作流落 catch、`action="数据缺失"`。该行已由并行会话的 `regime_gate_state` 订正覆盖并随 **v159** 播种生效。附：全库「数据缺失」仅 4 条（10-10 12:35 / 10-12 02:15 / 10-12 15:10 / 10-16 02:44，tv=149/152/152/157）——后三条各属不同代际的独立成因，量小，不逐条深挖；⚠ 其中 `474c1750`（紫金 7-22 的 as_of 重放）正是 P0-8 置回行 `bcd4cb44` 的原分析 ⇒ 该反思行的原料是坏决策，重试后若产出样本需按脏数据对待。
23. **v158 同版本号不会重播种**（版本门 `existing.version >= TEMPLATE_VERSION ⇒ return`）⇒ 已升 **v159**：v158 于 04:30 被播种（含当时工作树快照），但此后又修了 R-F12-OVERBOUGHT 前向引用（该快照内的首版会让 RSI>75 且动量转负的分析落 catch）与 P1-1②/P2-f 收尾——必须再升一版才能播种。**本次重启后应播种到 159 才算包含了全部修复**。

## 七、P1-3 校准封顶 + P2-a 腿级预测力（2026-10-16·第三会话）

### P1-3「概率校准」—— 落地的是**可得性判据**那一半

现网读数（2026-10-16 上午 `strategy_performance`）：14 行 —— v79 4 / v89 2 / 代际未知 8；
按起算代 ≥125 筛后**本代 0 行**。计划里「M1 样本 ≥30 后做 isotonic/Platt 映射」这一半
物理上不可执行（任何 30 条的可靠性曲线都是过拟合阶梯），故本批只落**另一半**：

- 判据单一来源：`reflection_stats::CALIBRATION_MIN_SAMPLES = 30`（与 `MIN_SAMPLE`/`IC_MIN_SAMPLE`
  同族，**不是**面板变量 —— 样本门槛在本仓一贯是引擎常量，面板变量留给决策量纲）。
- 逐档结论由 Rust 算：`horizon_prior_map` 每档新增 `calibrationReady`，判据取 **group 自身**
  的 `samples`，**不取** `est.samples`（后者在样本不足时借自全档合并基准 ⇒ 别档样本会掩盖本档不足）。
- 四份 `portfolio-mgr-h-*.rhai` 透传 `calibrationReady`；主链按「脚本只读结论、不比阈值」的
  DQI 纪律决定是否封顶。
- 主链折进**既有 cap 单一权威**：`confidence_cap = min(质量 cap, 校准 cap)`，未达标取本文件
  已有的最低一级 `cap_floor`（不新增常数）。P0-1 的动作联动读的就是这个终值 ⇒ 校准不足**同时压动作**，
  不重蹈 R5「只降展示数字」。追溯链按实际收紧的那条 cap 分别记 `R-CALIB-CAP` / `R-DQI-CAP`；
  reasoning 追加一句「本档历史判定样本未达校准门槛…」（展示侧唯一归因载体）。
- 主链兜底路径（四路分支全缺席）按未达标处理 ⇒ fail-closed。
- 待办（本版刻意不做）：`calibrationReady=true`  today 只是**解除封顶**，不是已完成校准映射。
  isotonic/Platt 待本代 M1 样本 ≥30/档 再实施；映射未上线前，面板上的「该档命中率」（P2-c）
  是本档可信度的唯一对外表述。

代际副作用已核：反思建点不读置信数值（读的是行上自带的 `min_confidence_threshold` 常量），
故封顶不会饿死样本供给；rank IC 的预测侧取 `horizon_results.decision.confidence`（**未压 cap**
的逐档原始值），校准输入不被封顶污染。

### P2-a「信号上线前必须过预测力检验」—— 先有读数，不动权重

普查结论：既有 rank IC 是**档级**（该档最终置信 vs 该档净收益），全仓**没有**腿级判据；
`factor_analysis.rs` 的 `is_significant(|IR|>0.5)` 零消费者且属另一套词汇（因子注册表），
`hit_rate_backtest::compute_factor_ic` 样本不足时写 0.0 但其注释自证「消费方是权重调整、只消费数值」
—— 按既有裁定保留，不改判据口径。

新增 `analysis-engine::leg_predictiveness`（不与前二者并行为第三套：数值核复用 `harness::spearman_rank_ic`，
筛样复用 `reflection_stats::generation_bucket`）：

- 观测 = 已完成反思行的四周期结果（该档到期收益/胜负）× 被复盘分析行 `horizon_decisions` 的
  `legs[].{factor,signal,weight}`；逐档键 **snake/camel 双取**（追加项 20 的实证）。
- 每个 (档 × 腿) 分格给 `rank_ic` + **Fisher-z 95% 区间** + `direction_hit_rate` + `status`，
  四态分开：`ok` / `insufficient_sample`（n<30）/ `ic_undefined`（|ρ|=1 或秩退化）/
  `ci_spans_zero`（有数不显著）。`usable` 仅 `ok` 为真。
- 方向胜率分母 = 腿与结果**都给出方向**的观测（signal=0 或收益=0 两侧都不进 —— 否则持平会被记成看空命中）。
- 消费方 = 离线 bin `axagent-batch-rerun --leg-ic-report`（只读）。**不接面板**：月频读数接面板
  就要补 11 语言文案并常驻一格「样本不足」空卡。
- ⚠ 裁定：本批**不做**「IC 不显著 ⇒ 权重置零」。两个理由：① 现网 (档 × 腿) 分格全在 n<30，
  据此置零等于用噪声改四档权重体系；② 逐档乘数通道 `horizon_leg_weights_json` 已随 R-11 退役，
  重开注入通道 = 第五套参数，须单独拍板。等本代样本累积后再裁。

### 本批门禁读数

- `cargo test -p axagent-analysis-engine --lib`：`calibration_ready_follows_own_tier_samples_not_the_borrowed_prior`
  **1 passed / 0 failed**（688 filtered，上午读数）。
- 主链首跑（P1-3 落完后 `--lib portfolio_mgr`）：**3 红** —— ① `portfolio_mgr_runs_with_valuation_evidence…`
  与 ② `…surge_gate…` 因基线夹具按生产真值给 `calibrationReady=false` 而被 60 封顶改写前提
  （mid 增持→持有、开仓动作看不到）；③ 本批新门的正控断错量纲（拿展示置信断绝对值，而它还要乘
  视角分歧 mod）。已按夹具文档的既有口径处置：**键集合必须真、值不必与生产同** ⇒ 基线夹具改
  `calibrationReady=true`（未校准一侧由新门单独锁），正控改成两侧**严格大小**比较。
- 待取：主 crate `--lib` 全量（含 `branch_node_scope_is_exactly_the_seed_mapping`，四份分支脚本返回段加了键）
  与 `cargo check --bin axagent-batch-rerun`。
- **10-16 午后终局读数（三段）**：
  ① 已取 —— 引擎侧 `calibration_ready_…` 门 1 passed；主链 `--lib portfolio_mgr` 11 passed / 3 failed，
     三条红的归因与处置见上（两条夹具前提 + 一条本门正控量纲），已按处置改完。
  ② 被什么挡 —— 改完无法复跑：`cargo check -p axagent-analysis-engine --all-targets` 现报
     **2 × E0502**，位置全在并行会话当日新建的未跟踪文件
     `crates/analysis-engine/src/failure_mode.rs:170` 与 `:177`（`rejected` 可变借用与不可变借用冲突）。
     引擎 crate 编译不过 ⇒ 主 crate `--lib` 与 `--bin` 的复跑读数取不到。该文件属 P2-b 那一批，
     按纪律不改其逻辑，等其解除后补跑。
  ③ 待取 —— 引擎绿后依次：`cargo test -p axagent-analysis-engine --lib`、
     `cargo test -p axagent --lib portfolio_mgr`、`cargo test -p axagent --lib branch_node_scope`、
     `cargo fmt -p axagent-analysis-engine -p axagent -- --check`、定点 clippy。
- **P2-a 本会话产物已撤回**：本会话按同一目标写了 `leg_predictiveness.rs`（含 6 条单测），
  随后发现并行会话正在实现 `leg_ic.rs` + `failure_mode.rs`（两者均为本次新建的未跟踪文件）⇒
  两份并存就是禁区 12 的重复定义。已把本会话那份**移出源码树**（`.workbuddy/tmp/leg_predictiveness.withdrawn.rs.txt`，
  未删除，含 Fisher-z 区间与四态 status 的判据设计，可作 P2-a 那批的对照材料），
  lib.rs 声明、bin 的 `--leg-ic-report` 旗标与 `generation_bucket` 重构一并回退。
  ⚠ `reflection_stats::generation_bucket` 的抽公共重构**暂留**（并行批的 `leg_ic.rs` 可能引用）；
  若最终无人使用，会表现为一次 `dead_code` 警告，届时删它而不是留着。


### 剩余项现状（本批对账）

- **P2-b 失败模式结构化归因**：**并行会话在制**（`crates/analysis-engine/src/failure_mode.rs` 为本会话
  新建且未跟踪，含受控词表 5 值 + `catalog_for_prompt` + `build_failure_mode_board`，并声明与
  `miss_layer` 的分界）。本会话不重复实现、不改它的文件；看板接线与 `miss_layer` 跨期统计归那一批。
- **P2-e regime 多指数 + 宽度融合**：**用户 2026-10-16 批准判据草案**（≥2 bear 票 ⇒ 熊、≥2 bull 票 ⇒ 牛、
  否则震荡；波段票 = 道氏相邻两组高低点同步移动，弱信号置信 0.6）。批准后由**并行会话实现**
  （本会话不再造第二份）：`market_regime.rs` 新增 `swing_structure` / `index_vote` /
  `classify_regime_multi` / `REGIME_INDEX_SECIDS`（沪深300 `1.000300`、中证全指 `1.000985`、
  上证50 `1.000016`，用显式 secid 而非裸码），`hooks.rs:612-645` 三指数并发取数后投票；
  复权短路白名单同步扩到含 secid 形态的 6 个字面 —— 正是本会话上午那条「对指数发起分红查询并被
  本地复权算歪」的同族风险。市场宽度按普查结论**未实现**（涨跌家数比 / 新高新低 / 站上 MA20 占比
  无现成通道，封板率变化率缺每日快照载体），未以近似值冒充。
  ⚠ 本会话验收发现两条**待裁**（属并行批的在制文件，且该 crate 当时编译不过 ⇒ 未擅改其逻辑）：
  ① 三只指数**都拿到 K 线但每只不足 `VOTE_MIN_BARS`=20 根**时，`classify_regime_multi` 返回
     `Sideways`（置信 0.3），而 hooks 只在「全空」时才落 `unknown` ⇒ 「拿到了但无从判定」会以中性方向
     进 P0-7 门禁，与既有裁定「未判定 = 不加仓」相反（测试 `multi_vote_without_any_data_is_low_confidence`
     目前把这一行为锁成了期望）。
  ② `regime` 值域里没有 Unknown 变体（unknown 只是 hooks 注入的字符串）⇒ 建议把「有效票数为 0」
     在引擎层就判成 unknown，别在注入层再判一次（否则同一判据有两份载体）。
  待实测：多指数可取性仍只能应用内验（本机 curl 对 push2his 域不通）。


## 八、P1-5 中间态判据锁 + 两处编译解阻（2026-10-16 上午·第二并行会话，编号续上）

### P1-5「缺数统一语义」—— 判据锁（本轮增量）
四档脚本自 v133/v156 起已统一为：缺席腿不进 `tw`（分母 `dir_total`）⇒ `evidenceScale` 随证据量收缩、
posterior 经 `avg*escale` 向先验收缩；关键门缺数 ⇒「未判定 ≠ 通过」。但既有 scope 运行门只锁**两极**
（全齐备 ① / 全退化 ②），中间态（只缺一条腿/一个门输入）无判据 —— 谁把缺席腿改回计权、或把门改成
缺数放行，此前没有任何一条会红。本轮在 `branch_node_scope_is_exactly_the_seed_mapping` 增加
「单腿剥离态 / 门输入剥离态」（`run_with` 第三参 `masked`：键仍在 scope、值换 unit，
与生产「映射到了但源没产出」同形）：

- **单腿剥离**（ultra `catalyst_level`→`eventCatalyst`、short/mid `flow_persistence`、long `pmi`→`macroRegime`）：
  断言 `evidenceScale` 严格下降 + 该腿出现在 `absentLegs`；
- **门输入剥离**（ultra `seal_rate`、short `flow_persistence`、mid `kline_bars`、long `valuation_dcf_upside`+`pe_percentile`）：
  断言 `entryGatePassed == false`；ultra 另按 `gateBasis == "breadth_missing"` 归因
  （其夹具个股腿恒 `stock_not_strong`，仅凭 flag 分不出「缺数被识别」还是「绕过 breadth 检查」）。
- 负控拆在同一守卫上：缺席仍计 tw ⇒ 断言①红；门放行缺数 ⇒ 断言②红。每档都要在两张表里登记，
  漏一行即 panic 点名（防「漏一档 = 静默少测」）。

**读数（2026-10-16 上午）**：`branch_node_scope_is_exactly_the_seed_mapping` **1 passed / 0 failed**
（648 filtered）；`-p axagent --lib` 编译绿；主 crate `cargo fmt --check` 0 处。

### 两处编译解阻（并行批次 P1-1 物化在制品的中间态，非本批引入）1. `persist_m1_sample_if_absent` 的 `horizon_pnl_json` 键写成裸 ident `primary_horizon`
   （该名不在函数作用域 ⇒ E0425，整树编译不过）——已按同文件既有形态改为显式
   `serde_json::Map` + 动态键 `inputs.primary_horizon`（键=复盘档名，与字段文档同形）。
2. 同函数 `.count(db)` 缺 `PaginatorTrait`（sea-orm 2.x 报「not an iterator」）——fn 级 import
   已由本批作者自行补上（3827-3828）；第二会话先行加的文件级重复导入已回退，避免 `unused_imports`。


## 九、P2 三项收口轮（2026-10-16 午后·第一会话，编号续上）

### P2-e 多指数 + 波段识别 —— 已实施（含一处取数标的的否证）

- 现役判据只有 `000300` 一根且经 `to_em_secid` 按首位数字推市场位 ⇒ 拼成深市 `0.000300`。
  **先取读数再下结论**：新增 live 探针 `astock-data/src/lib.rs::index_secid_live_probe`
  （`#[ignore]`，跑法见函数注释）实测裸码 4179.51 / 显式 `1.000300` 4178.89 /
  `1.000985`=5313.13 / `1.000016`=2771.98 ⇒ 都是千点量级指数序列，**「一直在算一只股票」的怀疑被否证**；
  但裸码能拿到指数是 vendor 兜底的侥幸，且 `is_benchmark_code` 的复权短路按字面匹配 ⇒ 已全部改显式 secid。
- 引擎侧（`analysis-engine/src/market_regime.rs`）新增纯函数层：
  `swing_structure`（分形拐点，翼 ±2；高点或低点不足两组 ⇒ `None`，**缺席 ≠ 未下移**）、
  `index_vote`（均线判出方向用均线；均线判震荡时由波段结构补票，置信 `SWING_VOTE_CONFIDENCE=0.6`）、
  `classify_regime_multi`（≥2 bear ⇒ 熊、≥2 bull ⇒ 牛、否则震荡；有效票 1 ⇒ 沿用方向但乘
  `SINGLE_VOTE_PENALTY=0.7`；0 票 ⇒ 描述写「大盘确认不可用」）；波动率取**中位等级**（序数不取均值）。
  判据对称（`multi_vote_bulls_symmetrically`），不是只补空票。
- 清单单一出处 `REGIME_INDEX_SECIDS`（沪深300 / 中证全指 / 上证50）；`astock-data::is_benchmark_code`
  同步扩 secid 形态 5 项，否则会对指数发起分红查询并被本地复权算歪。
- 取数侧 `stock_workflow/hooks.rs` 三指数 `join_all` 并发，失败的那只以空数组进 series ⇒
  描述里点名「缺席指数：X」；三只全失败才落 `unknown`（与修复前的单指数行为同口径，不新增缺席形态）。
- 测试 11 条（`market_regime.rs`）：波段四态 + 补票正控（夹具自证均线仍是震荡，否则测不到滞后压缩）
  + 一票不足 + 对称牛市 + 缺席点名 + 单票降级乘数 + 零数据低置信。
- ⚠ 副作用声明（PLAN 偏差 25）：本改动**会提前 P0-7 熊市门禁的触发面**（均线确认需数周，
  波段同步下移数日即成立）。`market_regime` 的 JSON 形状未变 ⇒ 无需模板升版，`.rhai` 侧零改动。
- 市场宽度（涨跌家数比 / 新高新低 / 站上 MA20 占比 / 封板率变化率）**未实施**：
  现成通道只有单日涨停快照、无历史序列载体（第七节数据面普查结论），本批不造伪通道。

### P2-b 失败模式结构化归因 —— 已实施（词表 → 模板 → 落库 → 聚合 → 面板全链）

- 词表单一权威源：`analysis-engine/src/failure_mode.rs` 的 `FailureMode` 5 值
  （`dcf_optimistic_at_cycle_top` / `chase_big_up_bar` / `market_regime_endorsement` /
  `reverse_dcf_ignored` / `other`），`catalog_for_prompt()` 渲染模板清单 ⇒ 模板不抄值（反手抄漂移）；
  `parse_failure_modes` 白名单化并保留被丢弃 token；`build_failure_mode_board` 聚合。
- 接线五处（缺一即静默失效，逐处点名）：① 反思节点**内联 system_prompt**（`stock_analysis_setup/mod.rs`）
  加 `{{failure_mode_catalog}}` 与 `\"failure_modes\"` 字段；② 专家档案 `agency_experts/stock-analysis/reflection.md`
  加词表节 + 两处 JSON 示例补该字段；③ 该节点 `input_mapping` 登记；④ 模板变量表登记
  （`Variable{name:"failure_mode_catalog"}`）；⑤ 运行时注入（`stock_workflow/reflection.rs`
  与 `mover_recall_context` 同一批无条件注入，取 `catalog_for_prompt()`）。
- 模板升版 `REFLECTION_TEMPLATE_VERSION 4 → 5`（不升版则库里保留 v4 行、本次全部改动静默失效）。
- 落库：实体 `stock_reflections.failure_modes: Option<String>`（JSON 数组文本）；
  收尾 `update_many` 盖章；NULL=模板未产出，`[]`=产出但全被白名单丢弃（两态在库里可分）。
  非法 token 只进 `tracing::warn`（含 reflection id），不进本列。
- 库结构：已对现网执行 `ALTER TABLE stock_reflections ADD COLUMN IF NOT EXISTS failure_modes text`
  （与启动期声明式对账的产物逐字相同；提前做是为了不让重启窗口期的反思 cron 因缺列整链失败，
  参见 `entities/stock_reflections.rs` 里 `source` 列的连坐实证）。
- 聚合与 IPC：**零新增 invoke** —— `HitrateStats` 加 `failure_modes: FailureModeBoard`，
  取数复用 `build_hitrate_stats` 已经读进内存的 completed 反思行（不额外查库）；
  面板展开行的单值走 `dao::reflection_review_json` 的 `failureModes` 投影。
- 呈现层：`ReflectionPanel.tsx` 抽出 `FailureModesSummary`（三种缺席各占一句：未标注 /
  词表外自报 / 无一条命中），命中格的译名走**显式映射表**，词表外的值直出原串不伪造译名；
  组件测试 5 条（每个破坏样本只让对应那一句红）；i18n 15 键 × 11 语言（parity 门绿）。
- 计数域边界：本看板与 `miss_layer`（漏检链机械归因）**不是同一套账**，独立成卡不并排比较。

### P2-a 腿级预测力 —— 读数层被回滚后由本会话补成完整闭环（取数→算→存→裁决）

- 撞车与回滚的现场读数：本会话先写了 `leg_ic.rs`（装配 + 统计 + 覆盖），发现
  `axagent-batch-rerun` 已有 `--leg-ic-report` 并 import 第三会话 10:17 落地的
  `analysis-engine::leg_predictiveness` ⇒ 按禁区 12 **删除重复实现**（`leg_ic.rs` 已不存在）。
  随后再次取读数：`leg_predictiveness.rs` **文件消失且 git 从未跟踪**，bin 里 `leg-ic` 相关
  参数/分支/实现**全部为空**（`grep -c "leg-ic" = 0`）⇒ 那一批把整个读数层回滚了，
  PLAN 第七节的记载与当前树不一致（属该批的现场差异，本会话不替它改写章节）。
- 于是把 PLAN 原文要求的闭环补齐，实现住在 `analysis-engine/src/leg_weight_overlay.rs`：
  - **判据与第七节逐字同口径**（四态 `ok`/`insufficient_sample`/`ic_undefined`/`ci_spans_zero`、
    门槛 `LEG_MIN_SAMPLES=30`、Fisher-z 95%、`|rho|=1` 归入秩退化臂、方向胜率分母=双方都给方向的观测）；
  - 装配 `leg_samples_from_db_pair`：档位键 snake/camel **双取**（偏差 #20），该档无后验收益 ⇒ 整条不配（不造伪观测）；
  - 取数 `dao::stock_lesson_queries::leg_ic_pairs`（completed ∧ 有逐档结果 ∧ 原始分析带决策 JSON；
    下沉理由就是 `commands-no-direct-db` 棘轮）；
  - **写侧**：反思成功收尾 `reflection.rs::refresh_leg_ic_report` ⇒ 每产出一条配对就重算并写
    `settings`（键 `leg_ic_report`，含 `generatedAtMs`）；刷新失败只降级为「仍按先验权重」，不影响反思落库；
  - **读侧**：`hooks.rs` 在 `horizon_branch_specs()` 之后、注入 `horizon_branch_json` 之前应用
    `apply_leg_weight_overlay`。
  - **裁决层**：五种状态里只有 `ci_spans_zero` 作废权重（反 fail-open）；置零后把该档**其余方向腿重新归一到合计 1**
    （不归一＝把「一条腿没用」放大成「全档证据不足」）；全档方向腿都不显著 ⇒ 不重归一、不退等权；
    每腿必落 `icStatus`（缺标注会被呈现层读成已验证）。
  - 与被 R-11 退役的 `horizon_leg_weights_json` **无关**：本层改的是现役分支表里每条腿自己的 `weight`，
    不新增变量、不新增参数 ⇒ 第七节「重开注入通道=第五套参数」这条顾虑对本路径不成立。
- 测试 10 条（`leg_weight_overlay_tests`）：状态机有序判据、双键族配对、胜率分母排除持平、
  现网形态（n<30 全不足 ⇒ 分支表逐字不变 + 报告零置零）、置零后归一、全档不显著不退等权、
  无对应格标 `untested`、camel 档键仍命中、报告 IPC 键 camelCase 往返。
- **今天不会真的改权重（实算读数，非推断）**：以引擎同口径（Spearman 平均秩 + Fisher-z 95% +
  n≥30 + 无后验收益不配）对现网跑一遍 —— `37` 行 completed 且带逐档结果可配对 ⇒ **126 条观测 /
  16 个 (档 × 腿) 格 / 全部 `insufficient_sample` / 判「无预测力」`0` 格**（复算脚本
  `output/tmp/leg_ic_sim.mjs`，一次性模拟、不入仓、不建门）。
  ⚠ 先前按 `decisionsByHorizon` 展开算出的「19 格 / max 31」是**未配后验收益**的口径，两者不可混用。面板未接（尊重第七节「月频读数不常驻空卡」的裁定），
  生效证据是反思收尾的 `[leg_ic] 报告刷新：…` 日志与 `settings.leg_ic_report` 本体。
- 收敛路径（偏差 26）：若 `leg_predictiveness` 重新落地，本模块的「装配 + 统计」两段应删除、
  改由它映射 cells，只保留覆盖层 —— 不允许两套腿级判据并存。

### 偏差与新增待裁定（追加编号）

24. P2-e 的**市场宽度四量**（涨跌家数比 / 新高新低差 / 站上 MA20 占比 / 封板率变化率）无通道亦无历史序列载体
    ⇒ 本批只落「多指数 + 波段」两票源，宽度部分显式声明未实施（不造伪通道）。
25. P2-e 会**提前** P0-7 熊市门禁的触发面（这是该项目的目的，不是副作用失控）⇒ 下一次实测需核对
    「门禁触发日 vs 均线确认日」的差值，若出现单日噪声触发（同步下移但幅度极小）需回来收紧波段补票的幅度门槛。
26. P2-a 裁决层已接线闭环（写侧=反思收尾刷新 settings，读侧=分支表注入）。若 `leg_predictiveness` 重新落地，
    必须删掉本模块的装配与统计两段、只留覆盖层 —— 两套腿级判据并存即禁区 12；若裁定「永不做腿级置零」，则整模块连同读侧一起撤。

## 九、P2-e 批准后的验收与门禁读数（2026-10-16·第三会话）

- **P2-e 判据已批准并由并行批实现**（详见 §七「剩余项现状」的 P2-e 条：三指数 secid 直通、
  ≥2 票成势、波段弱票 0.6、缺席不进票但点名、宽度按普查结论未实现）。
  本会话的两个待裁点（有效票为 0 时返回震荡而非 unknown；unknown 无引擎层变体）**仍未改**，
  属并行批在制文件的逻辑，等其批次收口后再裁。
- 引擎 crate 全量 `cargo test -p axagent-analysis-engine --lib`：**714 passed / 0 failed**
  （含本会话 `calibration_ready_follows_own_tier_samples_not_the_borrowed_prior`
  与并行批 `market_regime::tests::*` 16 条 P2-e 判据门，全绿）。
  此前挡路的两条 E0502（`failure_mode.rs:170/177`）已由其作者修掉。
- 主 crate 上一轮全量：**648 passed / 2 failed** —— 本会话 P1-3 的三条红（追涨门、估值证据门、
  新加的校准封顶门）**全部复绿**；剩两条红的归因见下。
- 红 ① `init::stock_automation_cron_seed::tests::every_builtin_job_is_manually_executable`
  —— **判据自身缺一条腿**，不是生产缺陷：趋势智选内置任务带 `workflow_id`
  （`stock_automation_cron_seed.rs:116`），手动执行走 `execute_scheduled_task` 的**分支 1**，
  根本不读 `BUILTIN_TASK_TYPES`；而该门只查白名单 ⇒ 把可用任务判成不可用。
  已按生产分支顺序重写为「有 workflow_id **或** task_type 在白名单」，负控拆在同一谓词上
  （两条路都没有 ⇒ 拦下；补 workflow_id ⇒ 放行）。**没有**为了让旧判据变绿把 `trend-screening`
  塞进白名单 —— executor 无该分支（`recommendation_cron.rs:201` 明文），塞进去等于宣称有，门从此失效。
- 红 ② `reflection::…::missing_horizon_decisions_yield_single_primary_row` ——
  **本会话上一条判断错了**（我据「现树无条件建 1 行」推它已被并行批修好，复跑仍红）。真实根因：
  该用例第三段负控写的是「四档全中性 ⇒ 退回主档单行」，而 P1-9 已批裁定是「中性档按 20% 抽样建点，
  未抽中就是空表」（`reviewable_horizons` 的 `if !parsed.is_empty() { return out; }` 与 bin 的空名单
  文案同口径）—— **测试期望与裁定互相矛盾**，红的是测试侧。已把期望改成按同一条 `sampled_in` 现推
  的集合（并加一条「四档不可能全命中」的反恒真断言），不再写死条数把成败交给 token 哈希运气。
- 修掉的一处呈现层会说谎的注释：`portfolio-mgr.rhai` 仍写「hooks 拉沪深300 失败时注入 unknown」
  与「bear 市况（沪深300 空头排列）」，确认面已是三指数 ⇒ 改成三指数表述，并把历史实测标注成
  「P2-e 前的单指数时代」（不把旧证据改写）。`.rhai` 是被 include_str 进模板的，但库里最高仍是
  158、v160 尚未播种 ⇒ 该注释随 v160 一起生效，不需额外升版。
- `cargo fmt -p axagent -- --check`：本会话新增/改动处（`rhai_registry.rs`、
  `stock_automation_cron_seed.rs`）已按 rustfmt 自身形态修正；`hooks.rs:407/422/657` 三处
  属并行批 P2-e 区域的漂移，未擅动他人代码。

### 本批门禁与运行读数（三段，2026-10-16 午后）

**① 已取（可归因到本批）**

- `cargo fmt --all --check` **EXIT=0**（在 `src-tauri/` 下跑 ⇒ rustup 按 `rust-toolchain.toml` 用 pinned **1.97.0**，
  与 CI 第 475 行命令逐字一致）。为此先把本批 5 个独占文件按 1.97 格式化，另把 `horizon_prior.rs:278/289`
  两处**纯格式**（注释列对齐、`assert_eq!` 折行）对齐到 1.97 期望 —— 属他人 hunk 的中性格式修，只改不提交。
  ⚠ 1.97 rustfmt 改了 `use` 排序规则，对既有文件（如 `backtest.rs`、`blackboard.rs`）会提出与树上写法不同的排序；
  本轮全量 fmt 恰好只剩我处理过的这 2 处 ⇒ 说明这类漂移是**逐文件按需**出现的，不能靠一次包级格式化解决。
- `cargo check -p axagent --all-targets` **EXIT=0**（20m51s，含 bin 与 test target）。
- `cargo test -p axagent-analysis-engine --lib` **718 passed / 0 failed**
  （本批新增：`failure_mode` 8 条、`leg_weight_overlay` 10 条、`market_regime` 波段与投票 11 条）。
- 前端：`npx tsc --noEmit` **0 错**；`dprint check`（我改的 3 个文件）**0**；
  vitest `ReflectionPanel.*` 3 文件 **15 passed**（其中 `failureModes` 新门 5 条）。
- 契约门（绝对路径逐条单跑，`ci-check` 是串行短路的）：i18n key parity **11 语言 15922 键一致**；
  未译值 **0**；占位符使用 **0** 违规；`check-input-mapping.mjs` **零违规零基线残留**
  （本批新增的 `failure_mode_catalog` 映射键与变量表就是它覆盖的面）。
- 分层棘轮 `check-layer-discipline.mjs`：本批引入的 `commands-no-direct-db` 那条**已消**——
  M1 样本的查重与写入下沉到 `dao::stock_lesson_queries`（`has_reflection_m1_sample` /
  `insert_reflection_m1_sample`），命令层不再出现实体/ORM 字面量，**基线未上调**；
  腿预测力取数也直接落在 dao（`leg_ic_pairs`）。脚本另提示「1 个文件已低于基线 ⇒ `--update-baseline` 下调」，
  本批未代跑（属基线维护动作，需单独授权）。
- 运行读数（现网 PG，经 `scripts/pg-connect.mjs`）：`workflow_templates.stock-analysis` **version=159 已播种**；
  `stock_reflections` 分布 completed 46 / pending（long 43、mid 26、short 35、ultra_short 16）/ running 1；
  新列 `failure_modes` 已在库（`ADD COLUMN IF NOT EXISTS`，与声明式对账产物同形）；
  腿预测力实算 = **126 条配对观测 / 16 格 / 全部 `insufficient_sample` / 可作废 0 格**。

**② 被什么挡住（读数未取到的部分，点名归属）**

- `cargo clippy -p axagent-analysis-engine -p axagent-dao -p axagent-entities -p axagent-harness
  --all-targets -- -D warnings` **EXIT=0（2m01s）**。上一轮那 2 条红的处置：
  `failure_mode.rs:196` `unnecessary_sort_by` 是我的 ⇒ 改 `sort_by_key(Reverse(count))`（仍稳定，同数保持词表序）；
  `reflection_stats.rs` 的 `doc_lazy_continuation`（他人 hunk，`git show HEAD` 无该串）**没有代修，
  它在那两轮之间被并行批自己修掉了** ⇒ 印证「他人 lint 先归『等那批』」这条处置。
- 主 crate `cargo clippy -p axagent --all-targets -- -D warnings` **只剩 1 条红**：
  `src/commands/stock_analysis_setup/seed_consistency_tests.rs:3829` `manual_contains`
  （`masked.iter().any(|m| *m == name)` ⇒ 建议 `contains`），落在他人新增 hunk
  `@@ -3816,0 +3825,8 @@`（P1-5 单腿剥离态那条通道）内 ⇒ **未代修**。
  - ⚠ 本会话在这条门前先撞过一次自己的语法错：`reflection.rs:5147` 的补丁留下**尾随多余引号**，
    使 `r#"…"#` **未闭合**、把其后整段源码吞成字符串 ⇒ 报出来的是 `unknown start of token: \u{ff1a}`
    （全角冒号）这类**看着像中文标点问题**的错。定位靠逐行 `repr()` 数引号（`…""#",`＝5 个引号）。
    ⇒ 判据：非 ASCII 标点被报成语法 token 时，先怀疑上游字符串未闭合，不怀疑标点。
- `cargo test -p axagent --lib`：**649 passed / 1 failed**，唯一红条
  `commands::files_page::tests::test_open_attachment_accepts_existing_file`
  （`files_page.rs:742` 期望 Ok 得 `FILE_NOT_FOUND`）⇒ 并发互删型 flaky，该文件 `git status` 干净、
  非本批面（已登记进项目记忆「门禁已知 flaky 测试」第 30 条）。
- `cargo clippy --workspace --all-targets --all-features` 与 `cargo test --workspace`、全量 vitest **未跑**：
  前两条撞在同一把构建锁与批次级成本上；全量 vitest 与 Rust 门禁并发会被 CPU 挤成超时（既有判据），
  留到 Rust 门跑完再取。分层棘轮仍红 1 处：`commands-no-sibling-call src/commands/stock_analysis.rs 10 > 基线 9`，
  新增 hunk 是 `use crate::commands::stock_workflow::{SnapshotGap, take_primary_snapshot};`（非本批）。

**③ 需要做的运行验证（应用侧，本批无法自证）**

0. **模板版本对照（现网实测，四态里的「库里低于代码」两侧）**：
   `workflow_templates` 里 `stock-reflection` **库里 4 / 代码 5** ⇒ 本批 P2-b 的注入尚未生效；
   `stock-analysis` **库里 159 / 代码 160**（160 是并行批升的）⇒ 下次启动会一并播到 160。
   两句都不是缺陷，但**不重启就等于 P2-b 没有上线**（同 v157/v158 那次「升版没播种就不生效」）。
1. **重启应用** ⇒ 播种后 `stock-reflection` 应为 5，才有 `{{failure_mode_catalog}}` 与 `failure_modes` 字段。
2. 跑一轮分析 + 反思 ⇒ 核三件事：
   ① 日志出现 `[leg_ic] 报告刷新：配对 N 行 / 观测 N 条 / 分格 N 个 / 判「无预测力」0 格`，
      `settings` 里出现 `leg_ic_report`；
   ② 反思面板「失败模式分布」卡：现网应为**已标注 0 / 未标注 46**（这是诚实的空读数，不是链路故障）；
      新产出的反思行才会开始写 `failure_modes`；
   ③ 新分析的大盘描述应为「多指数确认（有效 3 票：熊 x / 牛 y / 震荡 z…）⇒ …」，
      且某只指数取数失败时点名「缺席指数：…」——单指数时代没有这两句。
3. 上一批遗留的呈现项一并对：`partial`/`stale` 徽章、依据档位标签、该档命中率、~~长期档「季度」文案~~（**已随 2026-10-16 的 P2-d 回退消解**：第 4 档统一为「长线」，且原 8 套冗余键族当日第二批已**删除 7 套、只留唯一族** + `Period::label_zh()` 逐字一致，门禁 `check-horizon-label-parity.mjs` 以「唯一族 + 禁回流」守之）。

### 追加读数（同日晚·第三会话收口）

- 主 crate 全量 `--lib`：**650 passed / 0 failed**（232.71s）—— 含 P1-3 的校准封顶门、
  重写后的任务中心「立即执行」齐备门、改口径后的抽样负控。
- `cargo fmt -p axagent -- --check`：**0 处**；`npm run typecheck`：**0 错**。
- `cargo clippy -p axagent-analysis-engine --all-targets -- -D warnings`：**0 告警**（2m12s）。
  首跑报出的 2 条里，一条是**本会话引入的真缺陷**：把 `generation_bucket` 插在
  `filter_by_generation_floor` 的文档块与函数之间 ⇒ 那份函数文档被接去描述我的 enum，
  函数从此裸奔（clippy 只报 `doc_lazy_continuation`，但它指的是错位而不是缩进）。
  已整段搬到文档块之前并回读顺序确认。另一条 `failure_mode.rs:196 sort_by_key`
  在其作者手中于两轮之间修掉，本会话未改他人文件。
- 主 crate clippy 与引擎全量测试复跑读数：见下一条编辑（本轮跑完补）。
