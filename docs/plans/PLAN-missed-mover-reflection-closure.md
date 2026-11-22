# PLAN：补漏未发现的妖股 → 三条链反思优化（智能荐股 / 趋势智选 / 分析工作流）

> 状态：active ｜ 日期：2026-10-21 ｜ 对应版本：v2.9.9


立项 2026-10-20。触发需求：发现→分析→反思主干已闭环，但**没被任何一条链发现的强势股**（妖股）要能被系统性补漏，并用它来优化三条链。
本文件只做两件事：**① 按现网读数给五层结论；② 给分阶段方案，每条修法都写出「哪段代码读它」。**

> 术语边界（本仓已定，勿再混读）：
> - **妖股 = 窗口涨幅达标**，判据唯一来源是模板变量 `mover_gain_ultra_short|short|mid|long`（10/20/30/40%），
>   窗口天数唯一来源是 `Period::default_holding_days()`（`src-tauri/crates/harness/src/holding_period.rs:294`：2/5/28/90）。
>   面板、文案、提示词一律写「窗口涨幅达标」，出现「涨停」即名大于内容（已有门禁锁这条）。
> - 「漏检」是**归因层**名目（`MissLayer`），不是「涨停没抓到」。
> - **智能荐股与趋势智选是两条链**，只在 `reco_picks` 同表共存（智选行 `style="serenity"`）；
>   自选股桶与本轮无关，不得拿来解释「看不到某票」。

---

## 一、现网读数（2026-10-20 单次，PG 正门 `node scripts/pg-connect.mjs sql`）

| 对象 | 读数 | 判读 |
|---|---|---|
| `mover_recall_runs` | **1 行**，status=success，`date_to=2026-10-16`，trigger=scheduler | 核查每周一趟（`mover-recall-weekly`，`0 9 * * 6`），当前滞后 4 天 |
| `mover_recall_events` | **46 行**，全 `pending`；层分布 not_in_pool 25 / other_period 9 / l3_scored_out 6 / unexplained 6 | 66% 的漏检被归到「不在候选池」 |
| 同上 · 档分布 | **ultra_short 39 / short 7 / mid 0 / long 0** | 中、长档窗口需 28/90 采集日，还没到可判定状态 |
| 同上 · 归属 | `analysis_id` 非空 **0**、`reflection_id` 非空 **0** | **漏检补分析一次都没跑过** |
| `mover_recall_analysis_batches` | **0 行** | 同上；该批次只有任务中心手动入口 |
| `stock_reflections` | 195 行；`source` 只有 due 128 / backfill 67（**无 mover_recall**）；`miss_layer` **195 行全 NULL** | 反思侧的权威归因盖章从没执行过（不是判据坏，是没有样本流经它） |
| 同上 · `mover_label` | mover **2** / normal 23 / window_incomplete 111 / no_market_data 25 / NULL 34 | 妖股标签有电，但真妖样本极薄 |
| `market_daily_close` | 5972 行 / **681 只** / 9 个采集日（09-30~10-20）；`amount` 缺失或为 0 **5384 行（90.2%）** | 事件域只有 681 只；强度与流动性判据缺数值 |
| `market_stock_universe` | 681 行（clist 595 + local 86）；`industry` 非空 508；板块 main_sz 618 / main_sh 38 / chinext 17 / star 8 | 覆盖全市场约 12%，且**深主板独大** |
| `market_universe_stats` | 2 行，最新 `coverage_pct=` **11.5%** | 覆盖率门 C1（<50%）现网必然红 |
| `reco_picks` | 11 个 style 共 24,170 行；`serenity` **87 行**（最近 2026-10-20 12:19），荐股侧最近批次 10-20 12:34 | 两条链确实同表同列 |
| `reco_scan_audit` | 7319 行 / 4 个 period | 只有荐股链写（`crates/dao/src/repo/reco_pool.rs:115`），智选不写 |
| `strategy_weight_history` | 35 行，**trigger 全是 reco-loop**，serenity 格 0 行 | 智选从未被闭环写过一行权重 |
| `decision_validations` | hit 394 / partial 254 / miss 490 / false_hit 72 / insufficient 66 + 智选四标签（excess_outperform 44、outperform 71、underperform 103、excess_underperform 136） | 智选标签域被二元判据排除，是**写在注释里的刻意设计** |
| `cron_jobs` | `mover-recall-weekly` 在列；**无** `mover-recall-analysis` | 补分析没有定时入口 |

---

## 二、结论（五层各一句）

- **名目层**：需求里的四件事都已有名目且各有唯一权威 —— 达标判据（`mover_gain_*`）、漏检归因（`MissLayer` 八层）、
  妖股标签（`mover_label_for` 四态）、漏检专项反思（`{{mover_recall_context}}` 三问）。**不需要新建任何同义概念。**
- **数据层**：漏检只能看见「已经枚举到的 681 只」。这是**分母缺陷**，不是判据缺陷；不解决它，后面所有优化都在 12% 的域里自转。
- **执行层**：`漏检 → 补分析 → 反思` 这条链**代码已通、库里 0 次**（补分析只有手动入口），所以「反思优化」目前拿不到任何一条漏检样本。
  另外，漏检信号唯一能自动生效的动作是**降权**（`MoverCellSignal` 只降不升），而漏检的语义是「下次要选到它」，方向相反。
- **呈现层**：漏检面板有明细与逐档窗口未满标注，但**没有链维度**，也没有把「补分析批次 0」如实呈现成「从未发起」。
- **门禁层**：管妖股的三道子门只锁「两处真相 / 文案 / 分母」；`scripts/` 全目录 grep `mover_recall_events|miss_layer|analyze_mover_recall` **零命中**
  ⇒「归因产出被三条链消费」这一格**零门**。

---

## 三、缺陷清单（F-x，按代价从低到高；每条标注归类）

**F-1 闭环一次没跑过｜需实机（重启 + 挂 cron）**
`MOVER_RECALL_ANALYSIS_TASK_TYPE` 自带 `#[allow(dead_code)]`，注释自陈「P2 仅手动发起，暂无消费者」
（`src-tauri/src/commands/mover_recall_analysis.rs:50`）。批次表 0 行、事件 46 条全 pending 是**必然结果**，不是故障。
连带：`stock_reflections.miss_layer` 全 NULL、`source='mover_recall'` 零行。

**F-2 归因把两条链混成一个分母｜纯代码可修**
`load_picks_in_range` 不带 style 过滤（`src-tauri/crates/analysis-engine/src/mover_recall.rs:684`），
命令层 `picks_same_period` 只按 `stock_code + period` 比（`src-tauri/src/commands/mover_recall.rs:145`），
⇒ 智选（`style="serenity"`，现网 87 行）命中会被判成「荐股已推荐」，反之亦然。
**这条直接决定本轮需求能不能成立**：不分链，就无法「分别对智能荐股和趋势智选做反思优化」。

**F-3 智选链对漏检信号两头都不接｜纯代码可修（但触既有裁定）**
① 全部 serenity 文件 grep `mover|market_daily_close` 零命中；② 智选不写 `reco_scan_audit` ⇒ 「算出却被淘汰」对智选永不可判；
③ 智选后验标签 `excess_*` 被 `hit_outcome_to_binary_outcome`（`src-tauri/crates/analysis-engine/src/hit_rate_backtest.rs:411`）判 `None`，
`reco_loop` 直接 `continue` ⇒ 现网 `strategy_weight_history` 智选格 0 行；④ 三力权重 `BOTTLENECK_WEIGHTS`
是编译期常量（`src-tauri/crates/harness/src/domain_ontology.rs:459`），同文件 77 行的表已自标「运行期不可变」。

**F-4 候选宇宙 11.5%，妖股先天进不了事件域｜需拍板（网络代价）**
`market_universe_stats.coverage_pct=11.5`，681 只里 618 只是深主板。漏检事件域 = `market_daily_close` 里有连续窗口日数的票；
没被枚举的票连「事件」都不会生成。既有实测结论不得重犯：东财 `push2 clist` 判废为每日主路（60 页仅 11 页成功、按 IP/指纹阻断），
板块枚举路线判废。可用的是 `boards.rs` 的**编码段盲扫**（现用于当日涨幅榜，4 板块 4 天一轮）。

**F-5 中、长档零事件是窗口未满，不是没有妖股｜需积累**
28/90 采集日未到（现 9 天）。002696 那类「游资短炒却被定档 mid」的票，现有短窗口判据抓不全它的形态。

**F-6 两个归因层永不产出，面板上恒 0｜纯代码可修**
`MissLayer::LateTiming` 从未被 `attribute()` 返回（只有枚举 + `as_str` + i18n 文案）；
`ByDesign` 在生产路径不可达 —— 命令层传的是**档位级** `period_has_active_style`（`mover_recall.rs:706`，四档都至少有一个出票风格 ⇒ 恒 true），
风格级 `matrix_active()` 只有测试在用。恒 0 的两格会被读成「此类问题不存在」。

**F-7 `in_pool` 判据对智选行不可达｜纯代码可修（与 F-2 同批）**
智选落库的 `seed_pool_json` 是**单个候选对象**而非数组（`src-tauri/src/commands/stock_workflow/serenity.rs:1747` 一带，`style` 统一写 `"serenity"`），
`code_in_pool_snapshot` 只解析顶层数组 ⇒ 现网 25 条 `not_in_pool` 里若混入智选批次，其「不在池」结论是拼错证据得来的。
必须先按链分域，再谈扩池。

**F-8 反思拿到的漏检样本，唯一可执行动作是降权｜纯代码可修（走输入面，不碰权重面）**
`MoverCellSignal` 的 penalty 折进 `reco_loop` 的格权重，clamp 到 `[0.05, 1.0]`（只降不升）；
L2/L3 结构提案被 `may_enter_param_grid` 挡出参数网格且**无执行器**（`reflection_tier.rs` 自述「不落进任何执行器」）。
⇒ 妖股漏检的 `fix_for_future` 十有八九是「该扩池 / 该加某类证据腿」，这类结论今天落库即终。

**F-9 没有门锁「漏检产出被三条链消费」｜纯代码可修**
`check-reflection-output-consumers.mjs` 的 `ENTRIES` 只有四条（`reco_strategy_weights` / `stock_lessons` /
`serenity_validation_summary` / `historical_weights`），登记制 ⇒ 不登记即不在扫描面。
最省的做法是**扩这张表**，不是造第十一道门。

**F-10 `amount` 缺 90%，强度标签与流动性门都靠它｜需实机（采集侧）**
`market_daily_close` 无 `volume` 列，推定金额只能在采集侧按 `close×volume` 算完再落 `amount`，
并靠 `source` 列区分真值/推定（荐股种子侧现有口径是「`amount=0` 视为未采集并放行」，`crates/dao/src/repo/reco_seed.rs:42`）。

**F-11 核查滞后 4 天｜纯代码可修（时刻调整）**
周六 09:00 一趟 ⇒ 周一~周五的达标行情不进入事件域；与 F-1 合批解决（同一张 cron 种子）。

**F-12 教训注入的档位隔离不严格，跨档漏入｜纯代码可修**
`fetch_same_ticker_completed` / `fetch_recent_completed` 的档条件都是 `horizon = h OR horizon IS NULL`
（`src-tauri/crates/dao/src/repo/stock_lesson_queries.rs:147` 与 `:188`，规则教训同款在 `:218`）⇒ NULL 档行跨档进 prompt。
妖股漏检反思产出的教训多为超短档语境，漏进中线分析是**已知危害**（601127/688378 那次实证）。
补分析样本可注入性已核实通畅：补分析走 `run_analysis_for_candidates` → `run_single_stock_analysis`
→ 收尾写 `blackboard_snapshot`（`src-tauri/src/commands/stock_workflow/core.rs:2266`），而注入按「有没有留依据」筛，不排 synthetic。

---

**F-13 落库的核查轮次没有「读回」入口，UI 上看不到某次执行的结果｜纯代码可修**
2026-10-20 用户在任务中心点「妖股漏检核查（内置）」的立即执行，执行**成功**（`cron_job_history` 落 2 行，
最新行 output=「区间 2026-09-30~2026-10-20 达标事件=77 漏检=65 reachability=50.649 coverage=30.769
unexplained_share=3.077 | run_id=mrr-6eaaeed5-…」，耗时 1937ms；本轮层分布 not_in_pool 38 / l3_scored_out 14 /
other_period 11 / recommended 12 / unexplained 2），但 UI 拿不到明细。三处断点：

- 任务中心只有一枚历史图标开抽屉，抽屉里只渲染 `output` 那一行汇总字符串，**明细没有任何控件**
  （`src/pages/TasksCenterPage/ScheduledJobsTab.tsx:531` 起的 detail 列）。
- 有明细的地方是 回测页 →「策略回测」页签 →「窗口涨幅达标漏检核查」面板，但它挂载时调 `analyze_mover_recall`
  **现场重算**、不带 run 归属（`src/components/stock-analysis/MoverRecallPanel.tsx:74`），
  所以「这次执行的结果」与「面板现在的数字」是两份东西。
- 根因：`mover_recall_runs` / `mover_recall_events` **没有按轮次读回的命令**。已注册的只有
  `analyze_mover_recall` 与 `get_mover_recall_event_review`（逐事件复盘）；runs 表唯一消费者是 20:00 进化聚合的
  `list_runs_since`（`src-tauri/src/commands/evolution_proposal.rs:282`）。⇒ 这就是 F-9「产出没人读」的一个具体实例。

修法（并入阶段 D）：① 新增两条只读命令 `list_mover_recall_runs` / `list_mover_recall_run_events`（按 `run_id` 取已落库明细，
不重算、不新表）；② 面板加「轮次」选择（默认最近一轮），并把「本轮已落库」与「现场重算」两种来源分开成句，不得并排成同一个数；
③ 任务中心历史对 `task_type=mover-recall` 的行把 `run_id` 从字符串里提成结构化字段并可点跳转；
④ 门：`check-reflection-output-consumers.mjs` 的 `ENTRIES` 加 `mover_recall_runs` 一条，
消费点写 `evolution_proposal.rs:282` + 新面板，接线前后各红一次以自证有电。

## 四、方案（阶段 A→D，A/B 是本轮主战场）

### 阶段 A：先让闭环通电，并给归因加「链」维度（纯代码 + 一次实机）

**A-1 把漏检补分析挂上定时任务**
- 做什么：在 `src-tauri/src/init/stock_automation_cron_seed.rs` 播第 12 条内置任务，job id `mover-recall-analysis-daily`（现内置权威 11 条，妖股核查在 `:55`）、
  task_type 用现成的 `MOVER_RECALL_ANALYSIS_TASK_TYPE`（撤掉那行 `#[allow(dead_code)]`）；时刻排在核查之后、反思之前。
- 谁读它：cron 执行分支 `init/services.rs` 里按 task_type 分派到 `run_mover_recall_analysis`（复用现有命令体，不新增 IPC）。
- 必须同时给的安全性：单轮配额上限（默认 top-N 按 `cum_gain_pct` 降序）、与时间旅行批次**共用进程级锁**（现有
  `time_travel_batch_lock`，try_lock 不排队 ⇒ 抢不到就本轮跳过并留痕，不得排队把 live 时间窗堵死）。
- 验收：`mover_recall_analysis_batches` 出现非零行；被认领事件的 `status` 迁移；`stock_analyses.source='mover_recall'`
  且 `anchor_event_id` 非空；`stock_reflections.miss_layer` 首次出现非 NULL 行。
- 门：扩 `scripts/check-cron-job-authority.mjs` 的权威清单（内置任务 id ⇄ 种子代码同源），新增一条即进扫描面；
  task_type 需进 `check-task-type-whitelist.mjs` 与前端任务中心白名单（现有 15 行 cron 的教训：**代码播了要能被门看见**）。

**A-2 归因按链分域（F-2 / F-7）**
- 做什么：把「命中」判据拆成两个集合 —— 荐股链（style ∉ {serenity, bottleneck}）与智选链（归一后 = serenity），
  事件表新增 `chain` 列（值域 `reco|serenity`），一条达标事件按两条链各出一条归因（同票同档两行，`chain` 参与唯一键）。
  `code_in_pool_snapshot` 增加对智选行「单对象」形态的解析分支（按 `chain` 走，不靠猜形状）。
- 谁读它：面板（链切换）、三个率的逐链版本、`MoverCellSignal` 只取 `chain=reco` 的行（智选另有 B-3 的通道，不混）。
- ⚠ 声明层与物理层要分两句报：加列走声明式对账，启动期 `additive_only` **不 DROP**，旧行 `chain` 为 NULL ⇒
  面板必须给「未标链（旧事件）」第三态，不得折进任一链。
- 验收判据：现网 46 条事件重跑核查后，`recommended` 为真的条数**只增不减**（原先由智选冒充荐股的命中被拆出来），
  且两链的 `unexplained` 占比分别可算。

**A-3 处置两个恒 0 的归因层（F-6）**
- `LateTiming`：接进 `attribute()`（窗口起点之前已有同档推荐 ⇒ 判「晚推荐」，不计漏检分母），或退役该枚举值 +
  `as_str` + i18n + 面板格一起删（**退役要连变空的组对象删干净**）。推荐：**接**，因为「晚推荐」与「漏推荐」的优化方向不同。
- `ByDesign`：命令层改传**风格级**判据（事件与格绑定后按 (chain, style, period) 取 `matrix_active`），
  档位级 `period_has_active_style` 保留给「整档无任何风格」的告警。
- 门：在 `scripts/check-reco-horizon-parity.mjs` 追加一段（现有 a–n 之后）：断 `MissLayer` 枚举值 ⇄ `attribute()`
  实际返回集 ⇄ 面板格三处同域，任一层在生产路径不可达即红；配一条「把所有层都判成可达」的负控。

### 阶段 B：趋势智链接入（本轮需求的另一半，A 完成后才有意义）

**B-1 给智选候选域补一条「达标清单」输入**
- 做什么：`seed_serenity.rs` 的 `ts_input_mapping`（现三键：industry_ranking / policy_news / user_themes，
  `src-tauri/src/commands/stock_analysis_setup/seed_serenity.rs:253`）加第四键 `mover_watchlist`，
  值由 live 侧从 `market_daily_close` 的窗口达标清单拼出（as-of 下按既有纪律显式声明「本代无清单」，不开 Fallthrough）。
- 谁读它：智选扫描节点的提示词变量 + 面板变量落点（`check-panel-var-landing.mjs` 的 LANDED 判据会盯它）。
- ⚠ 这是**输入面**扩张，不是权重面 ⇒ 不触碰 Q1「只降不升」与「禁止用 mover 数据提权」两条既有裁定。

**B-2 智选写截断留痕（F-3②）**
- 做什么：智选逐档展开后、最终入选集之外的那批候选，按荐股同 schema 写 `reco_scan_audit`（`chain=serenity` 由 style 值域天然区分）。
- 谁读它：`load_trim_evidence` 现成（键是 (period, code) 的风格集合），写入即让智选侧 `l3_scored_out` 从「永不可判」变可判。
- 代价：每批多 N 行留痕；口径要与荐股侧的 top-N 截断语义对齐，**不得把「内部否决」也写成留痕**（那是另一层，见 F-6 的兜底桶语义）。

**B-3 智选后验独立进权重闭环（不靠改标签域）**
- 现状根因不是别名匹配、也不是 `hit_outcome_to_binary_outcome` 判错，而是**智选候选没有目标价** ⇒ 二元命中判据不成立（刻意设计）。
- 做什么：另起一趟独立聚合 —— 按 `(chain=serenity, period)` 用超额四标签算「持续为负的档」的降权乘数，
  写 `strategy_weight_history` 用**新 trigger 值 `serenity-loop`**（与分析链的 `cron|manual|rule`、荐股的 `reco-loop` 三个键空间互不冒充），
  读回点接到智选模板变量 `w_supply / w_demand / w_irreplace` 的**覆写层**：`BOTTLENECK_WEIGHTS` 常量继续作默认值，
  变量表有覆写时用覆写值（改常量→改「常量作默认 + 运行期覆写」，同文件 77 行的自标表要一并更新，否则注释与代码打架）。
- 验收：现网智选格 `strategy_weight_history` 从 0 行变为非空，且逐 trigger 分池可查；面板（B1 那批已加的智选后验视图）出现「本轮是否改了权重」的心跳。
- ⚠ 需要拍板（妖-2）：这是**只降不升**口径下的降权，不是提权；若要允许「漏检达标 ⇒ 上调智选权重」，那是在推翻既有裁定，必须显式说。

### 阶段 C：分母与档位（多数要拍板或等里程碑，不在本轮硬做）

**C-1 扩枚举域（F-4）** —— 三个选项见妖-3。推荐「编码段盲扫只负责登记 code 到 `market_stock_universe`，
不负责出事件」，把「建清单」与「出涨幅榜」解耦，覆盖率棘轮随积累上调，面板同时如实写「漏检域=已枚举 N 只，非全市场」。

**C-2 中长档等待期可用化（F-5）** —— 面板已有逐档 `windowOpen` 与 `collected_days`，本轮只补一件事：
把「mid/long 因窗口未满而零事件」与「mid/long 真无达标票」在文案上分成两句（三态，不并成一个 0）。
另外**不做**跨档并集事件口径：达标事实按档生成、归因按档进行，与「四档相互独立、档位不是股票属性」那条裁定一致；
把某档的达标当作别档的证据，正是那条裁定禁止的类别替代。

**C-3 补 `amount` 供数（F-10）** —— 采集侧按 `close×volume` 推定并写 `source` 标签，真值优先；
荐股流动门那条「`amount=0`（未采集）OR ≥1亿」的现有拼写不得改成纯 `≥1亿`（会把加列前的存量整批判死）。

### 阶段 D：门禁与呈现（A/B 落地时同批带上）

**D-1 扩 `check-reflection-output-consumers.mjs` 的 `ENTRIES`**（不造新门）：加
`mover_recall_events.miss_layer`、`stock_reflections.mover_label`、`MoverCellSignal`、（B-3 后）`serenity-loop` 权重四条，
每条写清消费点 `file:符号`；配「豁免带日期、接线后不摘登记即红」的既有形制。

**D-2 链标签齐备门**：扩进 `check-reco-generation.mjs`（现有 R1–R9），断「读 `reco_picks` 做链归属的谓词必须显式带 style 域」，
负控用「去掉 style 过滤」这一格（正是 F-2 的形态）。

**D-3 漏检面板**：链维度切换 + `chain` 未标注旧事件的第三态 + 「补分析批次从未发起」显式成句（0 批次不得显示成「无待补」）。
11 语言同步（中文两语不参与「不得抄 zh-CN」的循环校验）。

**D-4 结构出口最小可用版（F-8）**：把漏检反思产出的「该票应入池」类结论落成一张**可复核的回灌清单**
（复用 `mover_recall_events.status` 加一态 `reflow_pending`，不新建表），下一轮 `refresh_mover_seed`
（`src/commands/stock_analysis.rs:3769` 的 `MOVER_TOP_N` 那处）把它并入种子并标 origin。
转接判据**事前注册**：命中率不降 + 池内覆盖率不降 + 逐档 rank IC 不劣化，闸形制同 `reco_ic_gate`（先 shadow）。

**D-5 核查轮次的读回入口（F-13）** —— 新增 `list_mover_recall_runs` / `list_mover_recall_run_events` 两条只读命令
（按 `run_id` 读已落库明细，不重算、不新表）；漏检面板加「轮次」选择并把「本轮已落库」与「现场重算」两种来源分句；
任务中心历史把 `run_id` 从汇总字符串里提成结构化字段并可点跳转。`ENTRIES` 同批登记 `mover_recall_runs`。
这一条**不依赖任何待裁项**，可先于 A-1 做（2026-10-20 用户实机点「立即执行」后反馈「看不到具体执行结果」即此条）。

---

## 五、待裁定（每条给推荐与依据；被推翻时按「改哪格」执行）

| 编号 | 问题 | 推荐 | 依据 / 代价 | 被推翻时改哪格 |
|---|---|---|---|---|
| **妖-1** | 漏检补分析是否自动挂 cron（每轮最多 N 只 as-of 全链分析） | **挂，配额先小（建议 10/轮）** | 不挂就永远是 0 批次，本轮需求不可验证；代价是 LLM 额度与 as-of 通道打点（push2 族本机速率封禁） | 撤 cron，改回手动：F-1 结论不变，本轮所有验收改为「用户手动发起一次」 |
| **妖-2** | 趋势智选的漏检/后验是否允许**提权** | **不允许，维持只降不升**；改走输入面（B-1 扩清单）+ 留痕（B-2） | Q1 与「禁 mover 提权」是既有裁定；B-3 只做负向降权即可闭环 | 若放开：`reco_loop` 的 `[0.05,1.0]` clamp 与 `ratchet_weight` 的 `new ≤ old` 强制都要退役，并另设防过拟合上限 |
| **妖-3** | 候选宇宙 11.5% 怎么扩 | **编码段盲扫专职化：只登记清单、不出事件**，覆盖率棘轮随积累上调 | clist 与板块枚举两条路已实测判废，不得重探；段扫代价≈每板块 13–50 请求/轮，可分日摊 | 若拒绝网络代价：面板与文档必须长期写「漏检域=已枚举 N 只」，且 C 阶段全部转为呈现层如实化 |
| **妖-4** | 一条事件按两链各出一行（`chain` 参与键）还是加两个布尔列 | **加 `chain` 列、一行一链** | 两个布尔列会把「两链都漏 / 只漏一条」压成并列真值，与「两套计数域不得并排成可比」冲突 | 改双布尔列：D-2 门的谓词判据与面板两格同步改 |
| **妖-5** | `LateTiming` 接线还是退役 | **接线**（判「推荐时点晚于窗口起点」，不计漏检分母） | 与「漏推荐」优化方向不同；`counts_as_miss` 已把它排除在分母外，接上不会污染率 | 退役：枚举 + `as_str` + i18n + 面板格 + 三问措辞一并删净 |
| **妖-6** | D-4 的回灌清单是否算「新增第五套参数/第五个池源」 | **算池源不算参数**，但必须先 shadow 预注册再转 on | 池源现有 5 个（热榜/行业/兜底/用户/mover + 板块盲扫），回灌清单是 mover 的**同源第二口径**，需在 `SeedPoolOrigin` 显式加一个值而不是复用 mover | 若判为不该加：D-4 降级为「只出清单给人看」，不接 `refresh_mover_seed` |
| **妖-7** | 补分析样本（`synthetic=1`）是否进荐股胜率分母 | **不进**（现有裁定：不写 `reco_picks`，天然不在分母） | 保持「补分析是为反思供样，不是为荐股出票」；任何把它算进命中率的做法都是把回放冒充实盘 | 若改：P-g 的四类分治口径要重定义，`check-reco-generation.mjs` 的 R6/R7 同步改 |

---

## 六、执行顺序与不可压缩约束

1. **本轮能做（纯代码 + 定点门）**：A-1、A-2、A-3、B-2、D-1、D-2、D-3、F-12 的档隔离收紧。
2. **需要实机（用户重启应用 + 真跑一轮）**：A-1 的落库验收、B-1 的候选清单入图、F-10 的采集供数。
   播种点在运行前、不在启动链路 ⇒ 改种子必须 bump `TEMPLATE_VERSION` 并重启才可见；本仓主 crate 门禁是批次级（11–19 分钟、占构建锁），Rust 改动要攒批验证，不要改一行编译一次。
3. **需要拍板**：妖-1 … 妖-7。
4. **只能等里程碑（不可压缩）**：mid 档可判定 ≈ 28 采集日（现 9，粗估 11 月中旬）、long ≈ 90 采集日（2027-02）。
   在此之前「中长档漏检优化」的任何数值结论都不成立，只能做 C-2 的如实呈现。

## 七、本文件刻意不做的事

- 不新建妖股判据、不新建窗口天数表、不新建第二张达标清单表（禁区 12：先复用既有唯一权威）。
- 不用「加固提示词」填结构性缺口（B-1 是加输入键，不是改措辞）。
- 不把智选与工作流的候选混成同一个分母（F-2 未修之前，任何「漏检率」数字都不可读）。

## 八、执行状态（2026-10-20 收口时补）

### D-5 已落地（用户批准「就把 D-5 落地」）

改动面 23 个文件：两条只读命令 + dao 两个读函数 + 面板轮次来源 + 任务中心跳转 + 11 语言 + 门登记。

| 交付物 | 落点 |
|---|---|
| 只读命令 | `src-tauri/src/commands/mover_recall.rs` 的 `list_mover_recall_runs` / `get_mover_recall_run_view`（`#[tauri::command]`，注册由 `build.rs` 生成，已实测进产物 `src-tauri/src/register_commands.rs`） |
| 库读原语 | `src-tauri/crates/dao/src/repo/mover_recall_runs.rs` 的 `list_recent_runs` / `fetch_run`（命令层不直连 DB，分层棘轮未抬基线） |
| 当轮载荷还原 | `src-tauri/crates/analysis-engine/src/mover_recall.rs` 新增 `StoredTierRule` / `StoredRates` / `gain_var_by_name` / `rules_from_stored`，并给 `MissLayer`/`MoverEvent`/`LayerRow` 补 `Deserialize` |
| 面板 | `src/components/stock-analysis/MoverRecallPanel.tsx`：轮次下拉 + 四种缺席各自成句（已落库轮次 / 现场重算 / 还没有轮次 / 指定轮次不在库内） |
| 跳转 | `src/pages/TasksCenterPage/ScheduledJobsTab.tsx` 把历史 `output` 里的 `run_id=` 提成链接；`src/components/stock-analysis/BacktestPage.tsx` 认 `?moverRun=` 直接落到「策略回测」页签 |
| 门 | `scripts/check-reflection-output-consumers.mjs` 登记 `mover_recall_runs`（4 个消费面）+ 两条负控 M9/M10 |

**与 PLAN 的偏差（逐条列名）**

1. 第二条命令命名为 `get_mover_recall_run_view`，不是草案里的 `list_mover_recall_run_events`。理由：面板要的是**与实时核查同形**的整轮载荷（`MoverRecallView` 摊平 + 轮次元信息），拆成「明细列表 + 面板自己拼」会出现两份计数域自己各拼一遍汇总数字。
2. 载荷还原读 `misses_json`（那一轮的全集），不读 `mover_recall_events` 表。理由：事件表只存漏检行、不含命中行，用它拼 `rates` 会得到与落库时不同的分母；面板要的「那一次的结果」以 `*_json` 快照为准。事件表继续只服务补分析认领。
3. 本轮**没有**给 `mover_recall_events` 加链（`chain`）列，也**没有**动 A-1/A-2 —— 那些仍待裁定。

**读回时不重算的口径保护**（写进代码注释与单测）：`misses_json` 为 NULL 是落库侧「该轮零漏检」的唯一合法表示 ⇒ 还原成空集；其余任何 JSON 解不出来一律 `Err`，面板走 `failed` 支。`rates` 里的 `null` 还原成 `NaN`（「算不出来」≠ 0%）。

### 一处刻意保留的差异

明细行数：实时核查在薄命令里截到 200 条（`analyze_mover_recall` 原有行为），按轮次读回**不截**
（落库快照本来就是全集）。两条路的汇总数字都取 `rates.events / rates.misses`（真实总数），
面板不显示「行数」这类会被截断影响的计数 ⇒ 不出现两套可比数字。若以后面板改成显示行数，
这一处必须同步。

### 本轮新查出的两条（不改代码，只登记）

**F-14 任务中心历史列的「有 output」分支此前没有测试**：`ScheduledJobsTab.test.tsx` 的 mock 只导出了 `showBackendError`，缺 `translateBackendError` ⇒ 任何 `output` 非 null 的历史行都会让详情列整棵树抛错。本轮加跳转测试时才第一次跑到这条分支（已补 mock）。含义：凡「后端会回一堆文本」的列，测试用 null 行就能把它遮住。

**F-15 「立即执行」与定时触发在落库里不可区分**：`mover_recall_runs.trigger_source` 对手动触发也写 `scheduler`。实证两轮：`mrr-d5cb333a` 落库 2026-10-17 09:04（该任务排程就是周六 09:00 ⇒ 定时），`mrr-6eaaeed5` 落库 2026-10-20 18:35（周二，用户在任务中心点的）⇒ 同为 `scheduler`。本轮面板因此**只报落库时刻、不报触发方式**（不假装知道）。修法并入 A-1：dispatch 把手动入口的来源写成 `manual`，前端再补标签 + 11 语言。

### 本轮门禁读数

**已取读数（2026-10-20 收口，源状态 = 本次最终状态）**

- Rust：`cargo check -p axagent --all-targets` EXIT=0（17m50s）；`cargo fmt -p axagent -p axagent-analysis-engine -p axagent-dao -- --check` 首读 EXIT=1（两处都是本轮新写的换行选择），`cargo fmt -p` 后复检 EXIT=0；
  `cargo clippy -p axagent-analysis-engine -p axagent-dao --all-targets -- -D warnings` 0；`cargo clippy -p axagent --all-targets -- -D warnings` 0（8m15s）。
- 测试（定点）：引擎 `--lib mover_recall` **16 passed**（含本轮新增 3 条）；主 crate `__TAURI_WORKSPACE__=true cargo test -p axagent --lib commands::mover_recall` **5 passed**（本轮新增，700 条被过滤器挡在外面，不是全量）。
- 前端：`npm run typecheck` 0；`npm run lint:eslint`（oxlint）0 warnings 0 errors；`npx dprint check` 全仓无输出；`npm run test:run` **239 文件 / 2576 条全过**（含本轮新增 6 条面板来源用例 + 1 条任务中心跳转用例 + 1 条负控）。
- 门：`check-reflection-output-consumers.mjs --selftest` **11 例全过**（含新增 M9/M10），正式跑「登记 5 条产物 → 消费面，豁免 1 条」；
  `check-reco-horizon-parity` a–n 全 OK（含 k 阈值单源 / l 禁「涨停」/ m 标签单源）；`check-i18n-key-parity` 11 语言一致、`check_i18n.py` 全通过；`check:eol` PASS；
  另 11 条股票业务门（reco-generation / cron-job-authority / orphan-modules / horizon 三件 / builtin-vars-keys / snapshot-whitelist / tier-purity / analyst-id-shape）全 OK；
  `check-single-source-facts` 默认模式 EXIT=3 = 只有「疑似腐烂」软判据 9 处（无一指向本轮改动的文件），CI 口径 `--ci` 为 0。

**未取的读数（不许被当成已收口）**

- `cargo clippy --workspace --all-targets --all-features` 与 `cargo test --workspace` 两条 CI 全量命令未跑 —— 按 AGENTS 的逐阶段口径，这两条属于**提交前那一次**；本轮只改不提交。
- dao 的两个新读函数（`list_recent_runs` / `fetch_run`）没有直连库的集成测试：它们是单表读取，行为由命令层的 `parse_run_payload` 定点测试与面板测试间接覆盖。
- **实机核对归用户**：重启应用后在任务中心点一次「立即执行」，再到「回测 → 策略回测 → 窗口涨幅达标漏检核查」确认默认落在那一轮、来源行显示落库时刻、点历史里的「在漏检面板查看该轮」能跳过来。

### A-1 已落地（2026-10-20 稍晚，同一会话内继续推进）

按 **妖-1 的推荐**做：挂上定时任务、单轮配额 10。**代价与节奏**：现网 111 条待补事件 ⇒ 每天 10 条约 11 个交易日清完积压；每天一轮 as-of 历史行情链（push2 族本机速率触发 RST），与时间旅行批次共用同一把全局锁（抢不到就本轮失败并留原因，不排队）。

| 交付物 | 落点 |
|---|---|
| 第 12 条内置任务 `mover-recall-analysis-daily`（每日 06:00） | `src-tauri/src/init/stock_automation_cron_seed.rs`（job id 常量 + 任务体 + 文档表 + 三条测试锚点 11→12） |
| 发起入口手动/定时共用一份判据 | `src-tauri/src/commands/mover_recall_analysis.rs`：命令体抽成私有 `launch(...)`，新增 `pub(crate) start_for_scheduler(...)`；`MOVER_RECALL_ANALYSIS_TASK_TYPE` 撤掉 `#[allow(dead_code)]`，新增 `CRON_BATCH_LIMIT=10` 与 `cron_default/to_json/from_json` |
| 调度分支 | `src-tauri/src/init/services.rs`（排在通用 workflow 兜底分支之前；**不**进 2244 那道交易日闸门 `matches!` 白名单 —— 重放锚定历史窗口起点，与今天是否交易日无关） |
| 三处白名单同步 | `scheduled_task.rs::BUILTIN_TASK_TYPES`、前端 `taskGrouping.ts` 的 `INVEST_TASK_TYPES` 与 `BUILTIN_EXECUTABLE_TYPES`（由 `check-task-type-whitelist.mjs` 逐值对表，已实测绿）；`browserMock.ts` 同形补一条任务 |
| 门自证下限 | `check-cron-job-authority.mjs` 的 `MIN_SEED_IDS` 11→12 |
| 测试 | 种子侧：条数锚点 + 新任务的时刻/配额/描述往返断言；命令侧：新增 `cron_config_tests` 三条（prompt 往返、空串=缺省而脏值=显式 Err、**只认 camelCase** —— snake 拼写会静默落回缺省，正是本批要消灭的形态） |

**偏差（逐条列名）**

1. **cron 的任务历史只写「已发起 + batchId」，不等批次跑完。** 理由：批次跑在独立多线程 runtime（as-of 链要 32MB 栈），要等它就得把 `execute_batch` 的运行时归属一起重构。逐票成败仍在任务中心「漏检补分析」来源里看；把「已发起」写成「已完成」就是伪装（该判据已写进分支注释）。
2. **配额显式写进 `cron_jobs.prompt`，不靠 `from_json` 的缺省。** 理由：任务行是用户可改的对象，数字必须看得见；种子语义是「存在即跳过」，改过之后重启不被回滚。
3. 时刻取 06:00（不是草案里未定的空槽）：在 04:00 中断补跑之后、07:00 决策回测之前，当日新产的补分析行还能赶上 18:00 的到期反思。

**这条红已解除（2026-10-20 22:47 之后）**：`node scripts/check-cron-job-authority.mjs` 此前报
`R2 内置任务 mover-recall-analysis-daily 在库里没有行 ⇒ 该链未被自动化`。
播种发生在应用启动时 ⇒ 用户重启一次即解除；现读数为「权威 12 条 / 现网 14 行 active / 无孤儿 ⇒ ✅ 通过」
（那两条手工插入的孤儿任务也已被删掉，R4 同批转绿）。

**A-1 门禁读数（2026-10-20，源状态 = 本批最终状态）**：`cargo check -p axagent --all-targets` 0（8m22s）；
`cargo fmt -p` 三包 `--check` 0；`__TAURI_WORKSPACE__=true cargo test -p axagent --lib init::stock_automation_cron_seed`
**5 passed**（含条数锚点 12、executor 处置对表、手动可执行对表、配置往返）；
同库 `commands::mover_recall` **8 passed**（D-5 的 5 条 + 本轮 `cron_config_tests` 3 条）；
`cargo clippy -p axagent --all-targets -- -D warnings` **0**；
前端 `tsc` 0、oxlint 0、dprint 全仓无输出、`TasksCenterPage` 全套 **96 passed**；
`check-task-type-whitelist.mjs` ✅ 三处一致；`check-cron-job-authority.mjs --selftest` **8 例全过**、
正式跑只剩上面那条预期 R2。积压现网读数：`mover_recall_events` 待补 **111 条**（全 pending，无 excluded/failed）。

**F-15 仍未修**（本轮只让补分析自动跑，没动核查轮次的触发来源）：`services.rs` 里
`persist_mover_recall_run(&db, &view, "scheduler")` 是硬编码，手动「立即执行」与定时共用同一 dispatch，
区分两者要把触发来源作为参数穿进 executor 签名（跨 `runtime-core` 的 CronJobStore/dispatch），
不属于「顺手改一行」的量级 ⇒ 留在 A-2 之后单独一批。

### A-2 已落地（2026-10-20 更晚，同一会话内继续推进）

一次核查现在产出**两条链各一份归因**：`analyze_mover_recall` 返回 `Vec<MoverRecallView>`（reco / serenity），
每条链各自算三个率、各自落一行 `mover_recall_runs` + 每事件一行 `mover_recall_events`，`chain` 列随两者一起写。

| 交付物 | 落点 |
|---|---|
| 链词汇（唯一权威） | `src-tauri/crates/analysis-engine/src/mover_recall.rs:66` 的 `enum RecoChain` + `of_style`（:85）+ `CHAIN_UNLABELED = "unlabeled"`（:96）；`as_str` 产 `reco\|serenity`，与前端 i18n 键域由 parity 门 o 段对表 |
| 列（声明层） | `src-tauri/crates/entities/src/mover_recall_runs.rs:49`、`.../mover_recall_events.rs:53` 各加 `pub chain: Option<String>`（走声明式对账，启动期 `additive_only` 不 DROP ⇒ 旧行为 NULL） |
| 逐链视图 | `src-tauri/src/commands/mover_recall.rs:162` `struct SharedFacts` + :170 纯函数 `build_chain_view(chain, f)`；取数一趟、判链两遍（不因链重复打行情）；persist 在 :340 / :1108 两处写 `chain` |
| 调度摘要 | `src-tauri/src/init/services.rs:3314` 起：逐链 persist 后把两行汇总拼成 `[reco] … / [serenity] …` 写进任务历史 `output` ⇒ 面板与任务中心不再各拼一遍计数 |
| 补分析认领按三键 | `src-tauri/crates/dao/src/repo/mover_recall_events.rs`：`CHAIN_ROWS_PER_EVENT=2`（:20）、`event_key`（:27，`(run_id, code, period)` 之外的第三键域是链）、`dedupe_by_event_key`（:33）、`expand_to_siblings`（:42）；`claim_events/finish_event/stamp_reflected` 全部走姊妹扩散，`list_candidate_events` 先按 `limit×2` 取再去重截回配额 |
| 权重闭环只吃荐股链 | `mover_recall.rs::load_mover_cell_signal` 过滤 `RecoChain::of_style(&p.style) == RecoChain::Reco` ⇒ 智选票不再进 `MoverCellSignal` 的分母（B-3 另起一趟，不混） |
| 进化聚合避开智选链 | `src-tauri/src/commands/evolution_proposal.rs` 把 `chain='serenity'` 的 run 计入 `serenity_runs_skipped` 并 `tracing::info!` 留痕（不静默丢） |
| 面板 | `src/components/stock-analysis/MoverRecallPanel.tsx`：链下拉 + 四种缺席各自成句（`mover-recall-runs-empty` / `-run-gone` / `-source` / `-chain` / `-chain-unlabeled` / `-chain-select` / `-run-select`） |
| 门 | `scripts/check-reco-horizon-parity.mjs` 新增 o 段 `checkChainVocabulary`：`RecoChain::as_str` 的字面量 ⇄ 前端 `chainName` 的 i18n 键 ⇄ 落库列值域三处同域，配 4 条自证（含「把链值改成第二套真相必须红」）；`check-reflection-output-consumers.mjs` 的 ENTRIES 补链维度消费面 |

**与 PLAN 原文的偏差（逐条列名）**

1. **`bottleneck` 判给荐股链，不判给智选链**（原文写「荐股链 style ∉ {serenity, bottleneck}」，那会把 bottleneck 格同时排除在两链之外）。
   依据：`bottleneck` 是**荐股矩阵里的一个格子**（`normalize_style_key` 把它归一成 serenity 只为取矩阵格服务），
   它的产出方是 00:00 荐股扫描，不是 01:00 趋势智选工作流 ⇒ 归智选会让两条链互相顶替。
   该判据由 `chain_of_style_is_not_alias_normalized` 单测钉住（刻意与别名归一对撞），o 段门再钉词汇表。
2. **「唯一键」落在 dao 层，不新增 DB 约束**：现网 `mover_recall_events` 只有 `id` 主键与非唯一索引
   （实测 `pg_indexes`：pkey + batch_id/status/analysis_id/run_id 四条普通索引），本来就不存在
   `(run_id, code, period)` 唯一索引。⇒ 三键去重与姊妹扩散写在 `dedupe_by_event_key` / `expand_to_siblings`，
   加 DB 唯一索引会因旧行 `chain IS NULL` 与新行同键而需要数据回填，属于另一批（且 `additive_only` 不 DROP，删不掉）。
3. **智选链的 `scored_out` 强制为空**（原文未提）：智选没有 `reco_scan_audit` 留痕（那是 B-2 的活），
   若不显式置空，`attribute()` 会把荐股链的截断证据算到智选头上 ⇒ `l3_scored_out` 在智选链永不可判是**当前事实**，
   用空集 + 面板缺席声明表达，不用「猜」。
4. **A-2 自己引出的一个账本缺陷，已在同批修掉**：姊妹扩散按三键**跨轮**找行，而
   `finish_event` 原先没有状态筛 ⇒ 新一轮批次会把旧轮已 `reflected` 的同三键行降回 `analyzed`
   （`reflection_id` 却留在行上，补分析面板按 status 的计数也会无因移动）。修法：新增
   `FINISHABLE_STATUSES = [running, pending, failed]` 并在 `finish_event` 上加 `Column::Status.is_in(...)`。
   `claim_events` 本来就有候选态守卫（`CANDIDATE_STATUSES`）⇒ 只有终态回写这一格缺电。
   dao 侧配了一条新门 `finishable_guard_partitions_the_declared_status_domain`：状态值域**从实体文档现场取词**
   （`/// - \`xxx\`：` 每行第一个反引号词，锚 6 条）而不是抄清单，并带一条「删掉声明一行 ⇒ 取值必须少一个」的负控。
5. 未做 A-3（`LateTiming` / `ByDesign` 两个恒 0 层）与 parity 门的 a–n 之后的「层可达性」段 —— 那是一条独立判据，
   本轮只补了链词汇段。

**验收判据仍待实机**（代码侧无法自证）：现网 46 条事件重启后重跑一次核查 ⇒
`mover_recall_runs` 每轮出现 2 行（chain=reco / serenity），两链的 `unexplained_share` 分别可算，
且 `recommended` 为真的条数只增不减（原先由智选冒充荐股的命中被拆出来）。

### A-2 的现网读数（2026-10-20 22:47 那一轮，用户重启后应用自己跑的 ⇒ 验收已成立）

一轮核查在库里长出**两行** `mover_recall_runs`（同 `date_from=2026-09-30 / date_to=2026-10-20`，同 `universe_size=681`）：

| chain | 达标事件 | 漏检 | reachability | coverage | unexplained_share | 漏检层分布 |
|---|---|---|---|---|---|---|
| `reco` | 77 | 65 | 50.65 | 30.77 | 3.08 | not_in_pool 38 / l3_scored_out 14 / other_period 11 / unexplained 2 |
| `serenity` | 77 | 77 | 45.45 | **0.00** | 45.45 | not_in_pool 42 / unexplained 35 |
| （旧混池轮 18:35，chain 为 NULL） | 77 | 65 | 50.65 | 30.77 | 3.08 | 与 reco 行逐层同数 |

三点结论：

1. **荐股链的账一分未动**（65 条漏检、四层分布与旧混池轮完全一致）⇒ 「拆链不会把荐股已有的命中让给智选」这条
   反向安全判据成立；旧轮的 50.65/30.77/3.08 本来就全是荐股链的数，智选在里面没有独立账本。
2. **智选链第一次有独立读数，且读数是「0 命中」**：77 条达标事件全部是它的漏检，42 条判「扫过但不在池」、
   35 条判「本窗无智选批次 = 不知道」。这不是拆链拆出来的新缺陷，而是混池时被荐股的覆盖率顶替掉的真实面貌。
   35 条「不知道」的成因可查：`reco_picks` 里 `style='serenity'` 全库仅 87 行，其中 **81 行是回放代**
   （`as_of_date` 非空，按 2026-10-13 代际裁定不得进 live 归因），**live 只有 6 行、日期全是 2026-10-20**
   ⇒ 智选定时链刚开始产出实盘行，历史窗口基本无批次可判（属阶段 C 的分母问题，不是 A-2 的判据问题）。
3. **判链方向的实证依据**：写方 `src-tauri/src/commands/stock_workflow/serenity.rs:1747` 是
   `style: Set("serenity")` ⇒ 趋势智选工作流的产物确实带 `serenity`；而 `bottleneck` 的 live 行有 1 940 条
   （10-17~10-20）——那是 00:00 荐股矩阵的格子，与 `of_style("bottleneck") = Reco` 一致。
   上一条偏差 1 由「代码推断」升级为「现网实证」。

### A-3 已落地（2026-10-21 凌晨，同会话续：处置两个恒不可达的归因层）

`LateTiming` 与 `ByDesign` 此前**生产路径永远产不出来**，而枚举、`as_str`、面板格、11 语言全都齐备
—— 看上去是八层归因体系，实际只落五层。A-3 把两层接回判据，并给「层可达性」补了必要条件门 + 充分条件测试。

| 交付物 | 落点 |
|---|---|
| 逐层遍历面 | `src-tauri/crates/analysis-engine/src/mover_recall.rs` 的 `MissLayer::ALL: [MissLayer; 8]`（门 p 拿枚举对这张表 ⇒ 加层不登记即红） |
| 链级档位判据 | 同文件 `chain_period_has_active_style(chain, period)` + `SERENITY_MATRIX_KEY`：智选链只看矩阵 `serenity` 那一行，荐股链沿用 `period_has_active_style` |
| 两条新证据 | `Evidence.chain_tier_active`（原 `matrix_active` 改名）与 `Evidence.picked_before_window` |
| 归因优先级 | `attribute(ev)` 去掉恒真的 `layer_period_ok` 形参；顺序改为 Recommended → **LateTiming** → ByDesign → PoolDegraded → 池三态 → OtherPeriod → L3ScoredOut → Unexplained |
| 命令层 | `src-tauri/src/commands/mover_recall.rs:180` 取链级 `tier_active`、:217 算 `picked_before_window`（同档、`generated_at < window_start`） |
| 门 | `scripts/check-reco-horizon-parity.mjs` 新增 **p 段** `checkMissLayerVocabulary`（枚举 ⇄ as_str ⇄ ALL ⇄ attribute 使用面 ⇄ 11 语言 `moverRecall.layer.*` 五处同域，含「面板自列层清单 = 第二套真相」与「i18n 多出退役层键」两侧），配 8 条夹具用例 |
| 测试 | 引擎：`attribution_priority_is_mechanical` 重写 + 新增 `every_miss_layer_is_reachable_from_attribute`（逐层夹具 + 两条优先级负控）+ `chain_tier_activity_is_scoped_per_chain`；命令层：新增 `pick_before_window_is_late_timing_not_a_miss`、`serenity_short_tiers_are_by_design_while_reco_still_judges_pool` |

**与 PLAN 原文的偏差（逐条列名）**

1. **ByDesign 落成「链级档位判据」，不是「(chain, style, period) 风格级」**：个股事件不绑定单一风格
   （达标事件只有 票 × 档 × 窗），拿风格级判据就得先给事件造一个风格归属，那是拼出来的证据。
   链级是原文意图的最小可判形态：智选链只看 `serenity` 行 ⇒ 超短/短判「按设计」，荐股链那一支与档位级
   **逐字同义**（由 `chain_tier_activity_is_scoped_per_chain` 的同义断言钉住 ⇒ 拆链不改荐股口径）。
2. **「档位级判据保留给整档无任何风格的告警」没有落地为告警通道**：现矩阵四档每一档都至少有
   一个出票风格 ⇒ 该告警恒不触发；按禁区 12「不造没有消费者的通道」，它由荐股链那一支的 ByDesign 直接给出
   （若未来整档退役所有风格，两条链都会判 by_design，面板 layers 里可见，不需要新告警位）。
3. **删掉 `attribute` 的 `layer_period_ok` 形参**：两个调用点此前都传字面量 `true`（恒真 = 没电的第二根线），
   保留它会让读者以为 ByDesign 有两个输入。
4. **LateTiming 排在 ByDesign 之前**（原文未定优先级）：同档在窗口之前出过票 ⇒ 链已经在做这一档，
   「按设计不做这档」的说法当场不成立。顺序由 `every_miss_layer_is_reachable_from_attribute` 的两条负控锁住。
5. **`reco_trim_evidence_is_not_borrowed_by_serenity_chain` 的夹具从超短档迁到 mid 档**：超短档现在会被
   ByDesign 短路 ⇒ 夹具走不到「留痕层」，那条判据会退化成恒过。mid 档序列（28 交易日、每日 +2% ⇒ 复利 ≈74%）
   只有 mid 满窗 ⇒ 恰一条事件，两条链都活着判据。

**下游影响（不是缺陷，是想要的效果）**：智选链在超短/短档不再产生漏检行 ⇒
`mover_recall_events` 里 `chain='serenity'` 的行数会下降，补分析队列不会再收到「智选本就不该出的格」的票；
`rates.coverage` 在整档按设计时算不出来（NaN → JSON null → 面板「样本不足」句），不会显示成 0%。

### 阶段 A 收口门禁读数（2026-10-21 02:1x，源状态 = 本批最终状态）

- `cargo fmt --all --check` **0**（首读 1 = 我新写的测试夹具三行超宽，`cargo fmt -p axagent -p axagent-analysis-engine -p axagent-dao` 后转绿；
  全程未跑裸 `cargo fmt --all` 写盘 ⇒ 没碰他人文件）。
- `cargo clippy --workspace --all-targets --all-features -- -D warnings` **0**（8m29s）。
  中间一轮 `EXIT=101`：`clippy::type_complexity` 指在 `facts_with(closes: Vec<(&str,&str,Vec<(&str,f64)>)>)` ⇒ 抽 `type Series<'a>` 后转绿。
- `__TAURI_WORKSPACE__=true cargo test --workspace --no-fail-fast` **147 套件 / 0 failed**（`TEST_EXIT=0`）。
  ⚠ 配对关系要说清：这一轮取在 `Series` 别名之前；别名之后重跑了**被改那个 crate 的那个模块**
  （`cargo test -p axagent --lib commands::mover_recall` ⇒ 13 passed）。别名是纯类型命名、零语义 ⇒
  不重跑全量 test，但读数分段标注，不混着当同一次。
- 上一轮（A-2 那批）同一条命令 147 套件里**恰 1 条红**：`axagent-astock-data` 的
  `news_limit_landing_tests::panel_value_reaches_every_news_fetch_site`（`面板 50 ⇒ left: 30`），
  单跑该 exe `2 passed` ⇒ 进程内共享表被并发用例压回缺省的隔离性 flaky；
  工作树 ≡ HEAD、该文件最后由 `bc5253675`（他人 10-19 批）触碰 ⇒ 不碰不修，本轮它自己绿了。
- 引擎定点：`cargo test -p axagent-analysis-engine --lib mover_recall` **20 passed**（A-3 新增/重写 3 条在内）。
- 前端（本轮 A-3 零改动，沿用 A-2 那批的读数）：`tsc` 0、oxlint 0、`dprint check` 无输出、
  全量 vitest **240 文件 / 2580 条全过**、四道 i18n 门（key-parity / untranslated / placeholder / check_i18n.py）全过、EOL PASS。
- node 门：`check-reco-horizon-parity` **a–p 全 OK**（p 段扫描面 195），`--selftest` 全过（含 p 段 8 例夹具）；
  `check-reflection-output-consumers` ✅（登记 5 条产物）、`check-cron-job-authority` ✅（权威 12 条 / 现网 14 行 active / 无孤儿）、
  `check-task-type-whitelist` ✅ 三处一致、`check-reco-generation` ✅、`check-single-source-facts --ci` **0**
  （默认模式的 9 条「疑似腐烂」软判据桶无一指向本批文件，PLAN 里两处 `file:行` 已按 fmt 后真实行号重定位：180 / 217）。
- `check-universe-coverage` 仍红 2 条：`coveragePct=11.5`（universe 681 / 申报 5924）与
  `C2 枚举域偏斜 main_sz 90.7%`。这是**阶段 C 的分母裁定**，不是本批判据缺陷，也不在 CI 正式跑（要从本机 PG 取数）。

---

## 九、阶段 B/C/D 执行状态（2026-10-21，用户「继续阶段B、C、D」＝按第五节推荐整批放行）

### B-1 已落地（输入面：智选拿得到「窗口达标清单」）

| 交付物 | 落点 |
|---|---|
| 取数与格式化 | `crates/analysis-engine/src/mover_recall.rs` 的 `mover_watchlist_digest(db, limit)` + 纯函数 `format_mover_watchlist`；最多回看最长窗口那么多天 ⇒ 代价与一次核查同阶 |
| 注入点 | `src-tauri/src/commands/stock_workflow/serenity.rs` 运行前覆盖 `mover_watchlist` 变量：**live** 取最新收盘日达标票；**as-of** 注入 `MOVER_WATCHLIST_ASOF_DECLARATION`（显式声明本代无清单，不开 Fallthrough —— 回放读 live 清单＝前视）；取数失败也注入「取数失败」声明，**不静默成空串** |
| 模板 | `seed_serenity.rs`：`ts_input_mapping` 第 4 键 + 变量注册 + `a-trend-scanner` 提示词一段（「只供优先核对，不构成入选理由」）；`TEMPLATE_VERSION` 60→**61**（生产与测试镜像两处同抬） |
| 测试 | `watchlist_digest_is_a_sorted_deduped_declaration`（空清单成句且点名判据来源 / 降序 / limit / 同票多档只留一条 / 只认当日锚点） |

⚠ **要重启才生效**：播种点在运行前，`TEMPLATE_VERSION` 已抬 ⇒ 重启后模板重建才会带上第 4 键；重启前该变量不在图里（注入值不会报错，只是没人读）。

### B-2 已证伪 ⇒ 退役该目标（不是没做，是原文前提不成立）

原文假设「智选逐档展开后有 top-N 截断，被淘汰的那批要写 `reco_scan_audit`」。**实测不成立**：
`serenity.rs::run_serenity_screening_inner` 的候选来自 `c-data-verifier`/`a-candidate-mapper`，提示词直接要求 LLM
输出 3–5 个最终候选（`seed_serenity.rs` 的 mapper prompt），随后**全部逐档写入 `reco_picks`** ⇒ 内存里不存在
「进入评分但被淘汰」的候选，也就没有可写的留痕。
实体文档（`crates/entities/src/reco_scan_audit.rs` 头部）明确禁止把「非候选/算过」压成「被淘汰」——硬写就是伪造证据。
⇒ **结论：智选链的 `l3_scored_out` 是按设计不可判，不是未实现**；A-2 已让它显式为空集、A-3 让 ByDesign 先短路掉
超短/短档，现存判据 `reco_trim_evidence_is_not_borrowed_by_serenity_chain` 继续钉住「智选不得借荐股留痕」。

### B-3 未落地，附结构性发现（堵住了一条会变成假接线的路）

原推荐「按 (serenity, period) 的降权乘数 → 覆写 `w_supply/w_demand/w_irreplace`」**构造性无效**：
`bottleneck_composite = s·ws + d·wd + i·wi`（`src/commands/stock_workflow/rhai_bottleneck.rs:387`，**不做归一化**），
三个权重同乘一个 m ⇒ 整批候选的 composite 同比例缩放，`max_score` 的 argmax 不变 ⇒ **入选项一个都不会变**。
而仓内没有「按力」的标签流（`decision_validations` 只记整票超额），所以差异化的三力降权也没有证据支撑。
⇒ 真正能咬住的杠杆只有两类：**入选阈值**或**逐档配额**，两者都要新参数/新口径，属第五节级别的拍板项。
**本轮不做**（不造「面板能调、代码不读」的假接线）；后验聚合本身要等 `decision_validations` 里
`style='serenity'` 的样本量起来（现网智选 live 只有 6 行），在此之前任何权重结论都是噪声。

### C-2 已落地（逐档三态，不再把「本档无达标」与「本档有达标」混成一个 0）

- 后端：`MoverRecallView.events_by_tier`（`period -> events`，live 恒有值）+ 新列 `mover_recall_runs.events_by_tier_json`
  （旧轮次 NULL ⇒ 面板第三态「本轮未记录逐档计数」，**不显示 0**）。
- 面板：每档三态 —— 未满（`windowOpen`）/ 已满且 0 事件（`tierNoEvents`）/ 已满且有事件（`tierEvents` 给条数）/
  旧轮次（`tierCountMissing`）；全局句拆三句：全满 ⇒ 结论、全未满 ⇒ 尚不可判定、**部分满 ⇒ 点名未满档**（`emptyNotYetTiers`）。
- 11 语言同批补齐；面板套件 **26 条**（新增三态 3 条 + 批次三态 2 条）。

### C-3 已落地（成交额推定，带可分辨来源）

`crates/analysis-engine/src/market_close_store.rs::close_row` 新增 `volume` 形参：源不给成交额（0/非有限）时用
`close × volume` 推定（解析层已把「手」换成「股」、「万」换成「元」⇒ 乘积就是元），`source` 打
`tencent_batch+amount_est` 后缀；两者都没有 ⇒ 仍是 0 =「未采集」。**荐股流动门的 `amount=0 OR ≥1亿` 拼写一字未改**。
测试 5 条（真值优先 / 推定 / 两缺 / NaN 量不得算出 NaN 额 / 停牌仍不落行）。

### D-1 已落地（不造新门，只扩登记表）

`check-reflection-output-consumers.mjs` 的 `ENTRIES` 5→**8** 条：新增 `mover_recall_events.miss_layer`
（消费者：漏检反思提示词 + 面板逐行）、`stock_reflections.mover_label`（消费者：反思面板 + 20:00 进化聚合）、
`MoverCellSignal`（消费者：`compute_loop_cell_weights` 逐格 penalty）。`--selftest` 11 例通过；正式跑「登记 8 条产物 → 消费面，豁免 1 条」。

### D-2 已落地，但**落在 parity 门 q 段**而不是 generation 门 R10

理由：这条判据的对象是**链域切片**，与 o/p 两段同族；`check-reco-generation` 管的是代际（as_of），
混进去会让 R1–R9 的语义漂移。q 段判据：漏检链里凡是读 `reco_picks`（`Entity`/`Column` 形态）的函数，
必须给出 style 域之一 —— `Column::Style` / `RecoChain::of_style` / 共享取数 `load_picks_in_range`（同文件须真有切片）；
另钉 `build_chain_view` 按链切、`load_mover_cell_signal` 只吃 `RecoChain::Reco`、前端不得自己按 style 判链。
首读（未修前树）：q 段红 2 条（`analyze_mover_recall` / `pick_generated_in_range` 被误判为「裸读」）；
把触发面收窄到 `reco_picks::Entity|Column` 后转绿（扫描面 4）。4 条负控（删切片 / 权重吃错链 / 新增裸读 / 前端自判链）。

### D-3 已落地

链切换、未标链第三态、轮次来源四态在 A-2/D-5 已完成；本轮补上「**补分析批次从未发起**」显式成句：
面板取 `list_mover_recall_analysis_batches(limit=1)`，三态 —— `batchesNever`（0 批，明说漏检不会被补分析）/
`batchesLatest`（最近一批状态）/ `batchesUnknown`（读不到 ≠ 从未发起）。11 语言同批。

### D-4 / C-1 本轮未做（给理由，不假关）

- **D-4（回灌清单）**：妖-6 已定「算池源不算参数，但必须先 shadow 预注册」⇒ 需要 `SeedPoolOrigin` 新增一格 +
  `reflow_pending` 状态 + 事前判据（命中率不降/覆盖率不降/rank IC 不劣化）。
  ⚠ **原推迟理由已作废（用户 2026-10-21 追问后更正）**：写成「等样本」是**循环论证** —— 清单不存在 ⇒ 永远不会有样本。
  正确的切分是两段：**清单半段**（反思的结构化 `pool_reflow` 结论 → 事件 `reflow_pending` + `reflow_note` → 只读清单命令
  + 面板可复核 → 这是样本的来源）现在就能做；**转正半段**（并入 `refresh_mover_seed`、标 `SeedPoolOrigin::reflow`）
  才需要事前判据，且判据必须 **fail-closed**：样本不足 ⇒ 保持 shadow（不因「没数据」而自动放行）。
  清单半段改动面 8 处（reflection.md + 内联 prompt 两处 + `reflection_validator.rhai` + 反思落库时按 `anchor_event_id` 盖章
  + 实体加 `reflow_note` + dao 两函数 + 只读命令 + 面板/11 语言 + 反思模板版本号）。
- **C-1（编码段盲扫）**：妖-3 的推荐方案是「只登记 code、不出事件」，但仓内没有段扫基础设施
  （`crates/scanner` 不存在该能力），且它是网络面（每板块 13–50 请求/轮）——需要你确认是否允许这部分网络代价，
  以及跑在哪个时刻（当前 live 时间窗 23:50–02:10 / 17:50–18:10 是禁用的）。在此之前面板与文档继续如实写
  「漏检域 = 已枚举 N 只（现 681 / 申报 5924 ≈ 11.5%）」。

### 阶段 B/C/D 收口门禁读数（2026-10-21 03:5x，源状态 = 本批最终状态）

- `cargo fmt --all --check` **0**（未跑裸 `cargo fmt --all` 写盘）。
- `cargo clippy --workspace --all-targets --all-features -- -D warnings` **0**（9m05s）。
- 定点测试：引擎 `--lib mover_recall` **21 passed**（B-1 新增 1 条；首跑 20/21 红是**测试自身的计数写法**
  —— 夹具名字里含代码（`股002696`），数裸子串把一行数成两次 ⇒ 改按行首匹配）；
  引擎 `--lib market_close` **6 passed**（C-3 四条新判据）；主 crate `--lib commands::mover_recall` **13 passed**；
  主 crate `--lib seed_serenity` **5 passed**（版本门镜像 61 与图重建）。
- `cargo check -p axagent --all-targets` **0**（8m42s，作为本轮第一道类型闸）。
- 前端：`tsc` 0、oxlint 0、`dprint check` 无输出（首读 1 个测试文件未格式化，`dprint fmt` 单文件后转绿）、
  四道 i18n 门全过、EOL PASS；全量 vitest **240 文件 / 2585 条全过**（A 阶段 2580 + 本轮 5 条）。
- node 门：`check-reco-horizon-parity` **a–q 全 OK**（q 段扫描面 4），`--selftest` 全过（q 段 5 例）；
  `check-reflection-output-consumers` 正式跑「登记 **8** 条产物 → 消费面，豁免 1 条」、`--selftest` 11 例；
  `check-reco-generation` / `check-cron-job-authority` / `check-task-type-whitelist` 正式跑与自证全绿。
- `cargo test --workspace --no-fail-fast`：**147 套件 / 1 failed**（`TEST_EXIT=101`）。
  唯一红条 `win_sandbox::tests::network_block_status_is_truthful`（`crates/tools/src/win_sandbox.rs:1445`）：
  `Unavailable` 档断言「网络确实没被断」，实测回环连接失败 ⇒ 判成「以为已隔离」。
  **归属与本批无关**：`git status` 对该文件为空（≡ HEAD），最后触碰是 `fdf976c0a`（2026-09-25 他人批）；
  单跑该 exe（`axagent_tools-*.exe network_block_status_is_truthful`）**1 passed** ⇒ 并行/环境 flake。
  本批自己的四个定点套件（21/6/13/5 passed）全绿。

### C-1 已落地（编码段盲扫：只登记清单、不出事件）

| 交付物 | 落点 |
|---|---|
| 段定义与分片 | `crates/analysis-engine/src/universe_scan.rs`：`CODE_SEGMENTS`（深主板/创业板/沪主板/科创板/北交所，含 605；**不含** 430 新三板与 399 指数段）、`codes_in_segment`（补 6 位）、`segments_for_shard`、`shard_for_day`（按 UTC 日轮转） |
| 非股票过滤 | `is_plausible_stock_quote`：6 位数字 + 价格/昨收有限且 >0 + 名字非空且不含「指数/ETF/基金/债/购/沽」+ `detect_market_type ∈ {main_sz, main_sh, chinext, star, bj}` |
| 登记（只登记） | `market_close_store::upsert_universe_registered(... origin, confirm=false)` ⇒ `last_confirmed_at` 保持 0（**不冒充东财全量确认**，面板「确认 N 只」不被灌水）、行业列不覆写；**不写 `market_daily_close`、不产漏检事件** |
| 入口 | `src-tauri/src/commands/universe_scan.rs` 的 `run_universe_code_scan`（手动）+ `run_universe_scan_inner`（手动与 cron 共用；live 时间窗拒绝只写这一处） |
| 定时 | 第 13 条内置任务 `universe-code-scan-daily`（每日 05:00）+ `services.rs` 调度分支（**不进**交易日闸门）；三处 task_type 白名单 + `browserMock` + `MIN_SEED_IDS` 12→13 |
| 面板文案 | 无需改：`moverRecall.universe` 已是「枚举域 {{size}} 只，其中 {{confirmed}} 只经东财全量确认」，`boundary` 已是「此处之前未采集、不按规则推断」——段扫新增的行只抬高 `size`，不冒充 `confirmed` |

**判据与代价**：分片「无重叠、全覆盖」由单测钉（1..=8 片都验）；非股票 9 条负控（指数/ETF/转债/零价/NaN/空名/非 6 位/新三板/B 股）；
每天只扫 **1/3 段**（6 段 ⇒ 每天 2 段 ≈ 4 000 个代码 ≈ 80 次批量请求），三天覆盖全表。
⚠ **`check-cron-job-authority` 现在会红**：`R2 内置任务 universe-code-scan-daily 在库里没有行` —— 播种在应用启动时，重启一次即转绿（与 A-1 同形，别改判据放宽）。

### D-4 进行中（2026-10-21，用户要求两阶段一并实施）

**已落（底层，可编译验证）**
- 实体 `mover_recall_events.reflow_note`（`Option<String>`，声明式对账，旧行 NULL）—— 与 `error_detail` 分开：
  一个是失败原因，一个是「本应入池」的依据。
- dao：`mark_reflow_pending(event_id, note, now_ms)`（**只**从 `analyzed`/`reflected` 盖章，`pending`/`running`
  还没被分析、`excluded` 是池降级，都不当回灌依据）、`list_reflow_candidates(limit)`（只读清单，最新在前）。

**未落（同一批的剩余面，按此顺序）**
1. 反思契约：`reflection.md` + `seed_stock_analysis` 内联 prompt **两处**追加 `pool_reflow`（仅漏检专项要求，
   `{should_include, reason}`）；`reflection_validator.rhai` 归一化并输出该字段；反思模板版本号 +1。
2. 反思落库：解析 `pool_reflow`；当 `mover_event` 存在且 `should_include=true` ⇒ `mark_reflow_pending`（失败只记日志，
   不影响反思行落库）。
3. 只读命令 `list_mover_reflow_candidates`（清单 + 转正就绪度）+ 面板一行（三态：无候选 / 有候选未达样本 / 可转正）+ 11 语言。
4. 转正半段：`SeedPoolOrigin::reflow` 新值 + `refresh_mover_seed` 读取清单并只在判据 `Pass` 时并入、并入后状态转 `reflow_merged`；
   判据纯函数 **fail-closed**：`Insufficient`（样本 < 门槛）⇒ 保持 shadow，**不因没数据而放行**。
5. 门：`check-reflection-output-consumers` 登记 `reflow_pending`（生产者:反思落库；消费者:`refresh_mover_seed` + 面板）；
   Rust 单测覆盖「pending/running 不得盖章」「样本不足不得转正」。

**D-4 两段已接通（2026-10-21 第二轮）**

- 契约：`reflection.md` + 内联 prompt **双处**新增 `pool_reflow`（仅漏检专项要求）；`reflection_validator.rhai`
  归一化（`should_include` 非 bool ⇒ 不表态；`reason` 截 200 字符）并输出；反思模板 `REFLECTION_TEMPLATE_VERSION` 6→**7**。
- 落库：`reflection.rs::pool_reflow_of` 三形态容错（顶层/一层嵌套/缺省=不表态）；判定 true 且有 `anchor_event_id`
  ⇒ `mark_reflow_pending`（dao 只从 `analyzed`/`reflected` 盖章；0 行时 warn「状态不符」，不冒充已入清单）。
- 清单：`list_mover_reflow_candidates` 返回候选 + `settled/required/ready`；面板三态
  （无候选 / 样本不足=shadow / 可转正），11 语言。组件测试 28 条（含 G/H 两条新态）。
- 转正：`SeedPoolOrigin.reflow` 与 `mover` **分列计数**；`refresh_reflow_seed` 在扫描前写进程内快照
  （覆盖语义，空也写）。并入需**同时**满足：① 人工开关 `settings.mover_reflow_merge = "on"`（默认 off）；
  ② `settled >= REFLOW_MIN_SETTLED(30)`。任一不满足 ⇒ 写空 + 如实日志（fail-closed：样本不足是**拒绝**，不是放行）。
- 分工声明：三项质量指标（命中率不降 / 池内覆盖率不降 / 逐档 rank IC 不劣化）是**打开开关前的人工核对条件**，
  代码只守样本下限 —— 刻意的，避免把「没有数据」的指标算成通过。
- 不新增 `reflow_merged` 状态：快照是覆盖语义、`merge_extra_seed` 按 code 去重 ⇒ 重复并入无害；
  `reflow_pending` 就作为清单态（实体状态机仍是 6 态，dao 的值域门不用改）。

### 提交与收口状态（2026-10-21 07:0x）

- **A-1/A-2/C-1 已由用户侧提交**：`733289d2e 🔧 feat: 漏检链分链归因 + 补分析批次 + 编码段盲扫扩域（A-1/A-2/C-1）`
  （另有 `9b9e05cd5` 是依赖补丁）。⇒ 「未提交 50+ 文件」的说法已作废，当前未提交面 = **D-4 一批（14 个文件）**。
- D-4 定点：引擎 22 passed（含 `reflow_ready_is_a_hard_floor`）、dao 3 passed、主 crate `commands::mover_recall` 13 passed、
  `pool_reflow` 1 passed；`cargo check -p axagent --all-targets` **0**。
- 前端：tsc 0、oxlint 0、dprint 0（1 个文件自动格式化后）、四道 i18n 门全过、EOL PASS、vitest **240 文件 / 2587 条**全过。
- node 门：`check-reflection-output-consumers` **登记 9 条**（新增 `reflow_pending` 条目）+ 自检 11 例；
  `check-cron-job-authority` ✅（现网 15 行 active，含 `universe-code-scan-daily`，无孤儿）；
  `check-reco-horizon-parity` a–q 全 OK。
- 全量：`fmt --all --check` **0**、`clippy --workspace --all-targets --all-features` **0**；
  `cargo test --workspace --no-fail-fast` **147 套件 / 0 failed**（`TEST_EXIT=0`）。
  ⇒ 阶段 A/B/C/D 全部收口，D-4 的 14 个文件待提交。

### 「立即执行」看不到结果 —— 已修（2026-10-21 实机反馈第二轮）

**根因**（不是数据没写，是 UI 拿不到）：`execute_scheduled_task` 的内置任务分支只调
`scheduler.trigger_now(job)` 就返回「已触发，请稍后刷新」，前端也只弹一个 toast
（`tasksCenter.scheduled.executedOk`）。核查结果确实异步落进了 `cron_job_history` 与
`mover_recall_runs`，但点按钮的人当场看不到任何东西。

**修法**
1. 新增 `commands/mover_recall.rs::run_mover_recall_check(db, trigger_source)`：算两链视图 → 逐链落库 → 拼摘要，
   **cron 与手动共用**；`trigger_source` 手动写 `manual`、定时写 `scheduler`（F-15 这条链闭合）。
2. `execute_scheduled_task` 加**分支 1.5**：`task_type == mover-recall` 时**等待**该函数（一次库内扫描，秒级），
   把摘要直接返回并 `record_run` 落史；其余内置任务仍走 `trigger_now`（长任务没法等）。
3. `TaskRunResultDto` 新增 `awaited: bool`（`#[serde(default)]`）：`true` = `output` 是这一轮的结论；
   `false` = 只是 ack。前端据此二选一：**awaited ⇒ Modal 摊开 output（含 run_id）**，否则仍弹「已触发」。
   `last_result` 回读映射填 `awaited=true`（落库的 last_result 本来就是跑完的结果）。
4. 11 语言新增 `tasksCenter.scheduled.resultTitle`。

**测试**：`ScheduledJobsTab.test.tsx` 新增两条 —— 「awaited+output ⇒ 展示含 run_id 的结论」与
「awaited=false ⇒ ack 不得被当成结果弹出来」；任务中心套件 **96 → 98** 条（8 文件）。

⚠ 界面名目澄清：这个入口在**「计划任务」页**（组标签是「投资 / 自定义 / OPC」），不是另有一个「任务中心 › 投资任务组」。

**门禁读数（本修复批）**：`fmt --all --check` 0；`clippy --workspace --all-targets --all-features` 0；
`cargo test --workspace --lib --no-fail-fast` **42 套件 / 0 红**。
⚠ 平口径的 `cargo test --workspace` 这次**跑不了**：应用正在运行 ⇒ 链接阶段删不掉 `target\debugxagent.exe`
（`os error 5 拒绝访问`）。这是环境占用，不是代码错；关掉应用后可补跑那条完整口径。
前端：tsc 0、oxlint 0、dprint 0、四道 i18n 门、EOL 全过；vitest **240 文件 / 2589 条**全过。
顺手修了一个真实的定时器泄漏：`executeNow` 的 800ms 延迟刷新此前卸载后仍会发 IPC（把「15s 自动刷新」
那条用例的调用计数从 2 顶成 3）⇒ 现在用 ref 保存并在卸载时清理。
