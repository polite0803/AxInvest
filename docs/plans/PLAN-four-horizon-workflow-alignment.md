# PLAN：四周期工作流对齐 —— 十项需求差距分析与实现方案

> 状态：active ｜ 日期：2026-11-03 ｜ 对应版本：v3.0.0+（未定版，待确认）
> 范围声明：本仓「四周期」跨**三条互不消费的链** —— 分析决策链（`src-tauri/src/commands/portfolio-mgr.rhai`）、
> 荐股链（`src-tauri/crates/analysis-engine/src/recommender/`）、趋势智选链
> （`src-tauri/src/commands/stock_workflow/serenity.rs`）。本文十项需求**只对分析决策链**立方案；
> 触及另两条链处单独标注，且沿用既有撞车边界（serenity 并发会话未提交区不动）。

## 差距定级口径

| 级别 | 含义 |
|---|---|
| ✅ 已对齐 | 现状即满足，勿重复建 |
| ◐ 形态不符 | 机制在，但产出/接线形态与需求不同（改动量小） |
| ✖ 结构性缺失 | 仓内没有对应设施，需新建（含引擎能力） |
| ⚔ 前提冲突 | 需求与既有裁定/物理约束互斥，须拍板后才能选路 |

---

## 一、差距分析（按十项需求）

### 需求①：技术分析师内部四周期切片，不拆节点；核查数据源是否满足四周期

| 子项 | 定级 | 现状（实证） |
|---|---|---|
| 分析师节点数 | ◐ | 10 个分析师，每类 1 个：`src-tauri/src/commands/stock_analysis_setup/seed_stock_analysis.rs:2460-2471`（`a-market-analyst` 为技术面）。**不拆节点这条已满足**。 |
| 分析师 prompt 有无周期维度 | ✖ | 10 份里**只有 2 份**提到档位：`src-tauri/agency_experts/stock-analysis/sector-analyst.md:21-23`、`sentiment-analyst.md:41-43`。**`market-analyst.md` 零周期字样** ⇒ 技术分析师被要求给一个跨周期统一结论。 |
| 四档技术评分是否进分析师 | ✖ | 5 个 `compute_scoring` 节点（`:3715`、`:3824-3867`）出边**只到代码/公式层**；四档 `totalScore_*` 仅 `:4517-4520` 流向 `portfolio-mgr`。`a-market-analyst` 唯一上游是原始 K 线（`:2685-2689` ↔ `:2506` `get_stock_kline`）⇒ **分析师看不到任何评分，更看不到分档评分**。 |
| 数据源满足度 | ⚔ | 逐档核查见下表。**技术腿齐，另两腿有硬缺**。 |

**数据源四周期满足度**（这是需求①后半的答案，也是需求⑦的输入）

| 周期 | 已有 | 硬缺 | 冒充点（最危险） |
|---|---|---|---|
| 超短 2d | 60 分钟 K 线，5 个 vendor 原生支持（`crates/astock-data/src/scale.rs:51-61`、`vendors/eastmoney.rs:1788`、`tencent.rs:363`、`sina.rs:242`、`xueqiu.rs:132`、`mootdx.rs:184`） | **实时逐笔/L2 全仓零命中**（`harness/src/market_data.rs:200-201` 自述 WebSocket 推 tick 是"架构占位，尚未实现"）；股吧情绪**只有 1 行当下聚合、无时序**（`vendors/guba.rs:256-321`）；换手率有字段但**未进工作流变量**（`astock-data/src/types.rs:323`，`seed_stock_analysis.rs` 内 `turnover` 零命中）；连板数**零字段**（涨停池解析未取连板 `vendors/ths.rs:305-338`） | — |
| 短 5d | 日线 + 周线 | 龙虎榜无机构/游资标记（`types.rs:106-114` 仅 7 字段，游资靠 prompt 猜 `agency_experts/stock-analysis/hot-money-tracker.md:46-49`） | `vendors/ths.rs:140-157` 用**涨停池拼成龙虎榜**、金额恒 0、日期空串，且被排在路由首位（`astock-data/src/lib.rs:303`）⇒ 伪记录无标注 |
| 中 28d | 月线 | 舆情时序仅有 news_archive（可用，见需求③） | **财报只有 `report_date`、无披露日**（`harness/src/market_data.rs:87-99`）⇒ as-of 截断按报告期（`astock-data/src/lib.rs:1150-1160`）会拿到尚未披露的财报 = **前视泄露且不记降级** |
| 长 90d | 月线本地聚合出季度（`scale.rs`，vendor 无稳定季线） | **宏观历史序列是纯 mock**（`astock-data/src/macro_data.rs:10` 自述占位、`:138-151` warn+mock、`:159` source 标 `mock(占位实现)`，且**不在工作流工具表**）；**一致预期所有 vendor 全 Fallthrough**（`lib.rs:4140-4192`） | as-of 下用「截止日前最近财报 trailing EPS」**充当一致预期并标 `is_estimated:false`**（`lib.rs:4162-4180`）⇒ 伪装成实测值 |

> **本项结论**：「技术分析师内部按四周期切片」这件事**代码层可做（改 prompt + 加 4 条入边），但数据层做不到均匀**——
> 超短缺 tick/情绪时序，长档缺宏观与一致预期。诚实形态是：逐档**声明适用性**，
> 不适用因子作废而非乘系数（与 `docs/plans/PLAN-multi-horizon-decision-science.md` Phase A 的裁定一致）。

### 需求②：四周期隔离与并行，每路内正反方对抗，周期之间不串味

| 子项 | 定级 | 现状（实证） |
|---|---|---|
| 引擎能否并行执行同层节点 | ✅ | 自研 DAG，就绪批次用 `JoinSet::spawn` 并行 await：`src-tauri/crates/rt-workflow/src/work_engine/engine/mod.rs:3001-3038,3546,3694`。 |
| 「按档展开成 4 路」能否动态表达 | ✖ | **不支持 fan-out**：全 crate 无动态建节点痕迹；`Parallel` 节点**只是元数据、不派发子节点**（`executors/parallel_executor.rs:5-27`，`wait_for_all`/`branch_timeout_ms` 未被调度消费 `engine/mod.rs:2692-2702`）；`Loop` 节点逐轮逐步**全串行**（`executors/loop_executor.rs:233,266-270`）。⇒ 四路必须**静态展开成 4 份**或用子图。 |
| 子工作流 | ✅ | `SubWorkflow` 节点经回调递归 `run_workflow`，带 `parent_execution_id` 与取消句柄（`executors/subworkflow_executor.rs:3-48,68-74`）⇒ **落地路径存在**。 |
| 对抗现状 | ◐ | 辩论 = 6 个独立 AgentNode 串边（`bull-r1..r3`+`bear-r1..r3`，`seed_stock_analysis.rs:2979-3109`）+ 1 个 `debate-convergence`（`:3113-3157`）；**单份、不分档**。风险辩论三节点独立一条（`:3492-3509`→`:3650`→`:3681`）。辩手已被剥成纯决策节点（`tools=vec![]`、`max_tool_rounds=Some(0)`，`:3034-3042`）。 |
| 「周期不串味」 | ⚔ | 现状是**反面**：`risk_bias` 由单一 `overall_risk` 出一个标量、四档共用（`portfolio-mgr.rhai:1800-1801,1872`）；教训注入代理的是「上一条分析的**主档**」一个档（`src-tauri/src/commands/stock_workflow/hooks.rs:314-338`，自述原因：增强钩子跑在 portfolio-mgr 之前、那一刻还没后验）⇒ 四套 lessons 常年只有三套不回灌。 |
| 批量场景下四路并行 | ✖ | 池扫描**按 stock_code 去重只留一个 period**（`src-tauri/src/commands/pool_scan.rs:107-132`）⇒ 批量分析现在就是一票一档。 |

### 需求③：时间旅行 —— 同一路径、按时间戳取数、关联网关插件防偷看未来

| 子项 | 定级 | 现状（实证） |
|---|---|---|
| Live 与回放同一路径、仅参数不同 | ✅ | 入参 `as_of_date`，命令入口 `src-tauri/src/commands/stock_workflow/core.rs:202`，`AS_OF.scope` 包裹**同一个** `run_stock_workflow_inner`（`core.rs:215-246`）；模板 id 缺省即完整链（`seed_stock_analysis.rs:6145`、`core.rs:420`）；批量重跑也是它（`src-tauri/src/bin/axagent-batch-rerun.rs:590`）⇒ **这条已满足，勿新建通路**。 |
| 截断生效层 | ✅ | 路由层 truncate + vendor `*_with_asof`（`astock-data/src/lib.rs:1079-1380`、`vendors/eastmoney.rs:2292-4454`），能力申报 `astock-data/src/as_of_capability.rs:21-39`，降级留痕 `astock-data/src/as_of.rs:314-339`。 |
| 「关闭联网搜索与插件权限」 | ⚔ | **全仓不存在 provider 级联网开关**：`grep web_search|enable_search|tool_choice|plugins` 在 `crates/providers/` 零命中；`ChatRequest` 无搜索/插件位（`crates/harness/src/types/settings_chat.rs:410-438`）。现有隔离是**结构性的**（节点工具只取模板白名单 `executors/agent_executor.rs:243,1094`；股票工具清单无联网工具 `seed_stock_analysis.rs:1814-2277`）。⚠ 两点未闭合：① `strict_tool_permissions()` 的 `forbidden_tools` 为**空**（`src-tauri/src/commands/stock_workflow/mod.rs:37-42`）；② 自动注入 web_search 只发生在 chat 流（`src-tauri/src/commands/conversations/streaming.rs:36-48,1162`），时间旅行不经过它。⇒ 若供应商**服务端默认开搜索，本仓无关闭手段**（未实证）。 |
| 时间泄露的真实来源 | ✖ | **不是模型偷看网络，而是取数层 3 处冒充**：①财报只有 `report_date`、**无披露日**（见需求①表）；②as-of 下用最近财报 trailing EPS **充当一致预期却标 `is_estimated:false`**（`lib.rs:4162-4180`）；③ths 用涨停池拼**伪龙虎榜**（`vendors/ths.rs:140-157`，且被排在路由首位 `lib.rs:303`）。另 1 处竞态：`as_of_ctx_or_degrade` 拿不到 ctx 时**退化为不截断**（`lib.rs:1027-1046`）。 ⚠ **勿当缺陷领活的两处**：行业归属/概念板块走 `Fallthrough`（`lib.rs:3930-3938,4272-4285`）是既有裁定正解（「慢变量 ⇒ `Fallthrough` 容忍当日值」，「真·当日榜单 ⇒ `NoHistoricalSemantic`」；T10/T12 修的是**申报倒挂**，不是这个分档本身）。 |
| 模型参数化知识这条泄露渠道 | ⚔ | **无法用调用层隔离**。模型权重本身含未来信息（它对某只票此后发生什么有先验）。需求③「隔离完全靠调用层控制」这句在**取数层成立、在先验层不成立**。⇒ 直接影响「时间旅行样本是否算有效分析样本」的定义，见 §三 Q6。 |

### 需求④：版本绑定 + Rhai 全量快照 + 按版本号调取历史做单条归因

| 子项 | 定级 | 现状（实证） |
|---|---|---|
| 结论记录版本号 | ✅ | `stock_analyses` 有 `template_version`/`template_id`/`as_of_date`/`blackboard_snapshot`（`src-tauri/crates/entities/src/stock_analyses.rs:51,57,94,113`；后两个行号已按 §五十九 退役两列后的顺移订正），写入 `core.rs:1159-1166,1498-1504`、`decision.rs:3213`；读数端已有版本漂移标注（`decision.rs:3295-3311`）。**这条已满足**。 |
| Rhai 算法+参数全量入库 | ✅ | `workflow_template_versions` 存 nodes/edges/variables（`src-tauri/crates/dao/src/repo/workflow_template.rs:344-347`）；Rhai 脚本作为 CodeNode 内容随 nodes 一起存 ⇒ 「天然版本快照」判断正确。 |
| 三要素齐不齐（版本号+行情时间戳+因子快照） | ◐ | 版本号 ✅、行情时间戳 ✅（`as_of_date`）、**因子快照半** —— 无 `factor_snapshot`/`params_snapshot` 列，因子值只以 `blackboard_snapshot` 形式存在（`analysis-engine/src/blackboard.rs:3,28`）；`model_version` 与 `data_snapshot_id` 已按「零设施」裁定**退役**（2026-10-05 §五十九：退役前现网 `stock_analyses` 80 行两列全 NULL、读侧零消费）。 |
| 反思按版本号调取历史 | ✖ | **零处**按版本筛样：`stock_reflections` 无版本列（`entities/src/stock_reflections.rs:7-70`），反思行仍写 `template_version: None`（`stock_workflow/reflection.rs:3439`）；批量反思按 status/hindsight 选样（`reflection.rs:1142,2204`）；IC/命中率/权重无版本过滤（`recommender/ic.rs`、`hit_rate_backtest.rs`、`weight_decay.rs:77`）。`workflow_template_versions` 存在但**从未与 `stock_analyses.template_version` 联查** ⇒ 「按版本号取回当时的工作流+结果」这条链是断的。 |
| 跨代混池是否已有防护 | ◐ | 只有一个口径水印 `snrAnchorDays`（写侧 `reflection.rs:3211` ← `portfolio-mgr.rhai:2963`），**读取处仅 1 处**（`analysis-engine/src/reflection_stats.rs:406-413` 只挡 rank IC 分母）。未覆盖：`direction_hit_rate`/`target_hit_rate`/`avg_raw_return_pct`/`avg_alpha_pct`（`:449-453`）、顶层聚合（`:470-521`）、以及**下游真正生效的消费者** `horizon_prior.rs:60-146` → `hooks.rs:442` 每次分析注入。荐股链更彻底：`recommender/ic.rs:166-168` 窗口内全样本配对、零水印，其 IC **直接自动降权**（`reco_loop.rs:347-360`）⇒ **污染就在写回路径上**。 |

### 需求⑤：反思闭环 —— 单条、输入含同因子同周期错例、输出定量修正、由规则节点执行

| 子项 | 定级 | 现状（实证） |
|---|---|---|
| 单条 vs 批量 | ◐ | 一行=一股一档（`reflection.rs:1860-1888` 逐档建 pending）✓；但消费侧是**批量队列** `run_batch_reflection`（`reflection.rs:1087,2116`，默认 max_count=20，循环 `:1118/:2193`）。已有「按档窗口」裁定（J 轮 Q5）。 |
| 输入=当前版本参数+同因子同周期错例 | ✖ | 输入含本样本硬数字（`reflection.rs:387-413,453-466,481,574,601`）+ `stock_lessons`（`:419-432`），但**无版本**、**无因子维度**：教训检索只按 code+档+时间（`crates/dao/src/repo/stock_lesson_queries.rs:33,44,66,86`）；`similar_cases` 存在但**只在分析链**且无档无因子（`stock_workflow/core.rs:2157`：code+loss+90 天）；反思链的向量库**只写索引不检索**（`reflection.rs:886-905`）；`reflection_lessons` **无因子列**（`entities/src/reflection_lessons.rs:11-45`）。 |
| 输出定量修正 | ◐ | 已有 `{param,current_value,suggested_value,reason}` 三元组（`stock_analysis_setup/mod.rs:1716-1723`），但 param 名被限死在 28 项 catalog，解析只回 `PortfolioMgrParamSet` 少数字段（`analysis-engine/src/portfolio_formula.rs:946`）；**无「哪个因子×哪档×from→to」结构化列**。 |
| 规则节点/Rhai 执行写回 | ✖ | 分析决策链**完全没接**：`reflection.rs:868-875` 代码明写「只记录不实际执行」；`evolution_drift.rs:176 recalc_and_persist` 只写 history，而 `stock_workflow/**` 对 `load_current_weights`/`strategy_weight_history` **零引用**。荐股链自动生效（`reco_loop.rs:470-480,512-552`，出厂 shadow `seed_variables.rs:813-821`）但 `source_reflection_id` **恒 None**（`:474`）⇒ 不可溯源到某次反思。人工通路有：`apply_param_suggestions`（`stock_analysis.rs:5842,5969-5975`）+ 前端按钮（`src/components/stock-analysis/ReflectionPanel.tsx:1094`）。 |
| 可调参数是否真接上 | ⚔ | 五点对账**纸面化实证**：`debate_rounds` 面板项存在（`src/components/settings/StockAnalysisConfigPanel.tsx:29,825`）但保存只让 dao `version+1`，而重种门是 `DB version >= TEMPLATE_VERSION(=122)` ⇒ **面板调轮数实际不重展图**。同类风险见 `seed_variables.rs:2533-2552` 自述「四处断链」。门禁缺位：`scripts/audit-inject-coverage.mjs` 头注自称"审计工具不是 CI 门禁"，`scripts/check-horizon-weight-parity.mjs` **不在** `scripts/ci-check.mjs`。 |

### 需求⑥：规则节点 —— 跨周期仲裁 + B 级节点处置（不破坏静态图）

| 子项 | 定级 | 现状（实证） |
|---|---|---|
| 跨周期仲裁 | ⚔ | **现状是有意不做**：`portfolio-mgr.rhai:1932-1934` 注释明文「这里**不做**四档 switch」，主档只是后验阈值指针；四档并列输出 `decisionsByHorizon`（`:3169`）→ 平行 JSON 落库（`entities/src/stock_analyses.rs:41-50`、写侧 `core.rs:1101-1134`）。`pool_scan.rs:109-144` 的去重**只决定先分析谁**，不产生跨档结论；`hooks.rs:131-232` 的 `divergence_attribution` **纯解释不回灌**。`arbit|仲裁` 在股票域零命中。⇒ 新增仲裁与此前裁定冲突，见 §三 Q3。 |
| 按信源质量裁决 | ◐ | 现成锚点是**孤儿配置**：`src-tauri/config/weight-tables.json:53-63` 定义了 `source_credibility` 表，**全仓零消费者**（grep 无命中）。接上它即「按信源质量裁决」的最省实现。 |
| B 级处置（跨周期连续异常→降权/熔断） | ✖ | 现有 A–F 分级是**本轮单次**评分（`src-tauri/src/commands/data-quality.rhai:1411-1414`）→ 同一模板 Switch 门禁按 `data-quality.result.grade` 路由到 `quality-fallback`（`seed_stock_analysis.rs:5134-5174,5178-5235`）。**跨样本累计状态机不存在**：内存态计数器重启即丢（`stock_self_evolution.rs:48,285-316`）；持久计数只到 lesson 粒度（`lesson_applications.rs:26-46`）；`vendor_health.rs:32-33` 是取数层不属结论层。「熔断」现有语义=预算闸（`scheduler/gate.rs:27-51`）。 |
| 降权设施可复用 | ◐ | `weight_decay.rs:78-134`（Beta 平滑 + log 饱和 + EWMA，下限 0.05，**只降不升**）——消费者目前只有荐股线（`reco_loop.rs:250-261`、`recommender/mod.rs:178`）。 |
| 用判断/代码节点、不破坏静态图 | ✅ | 节点枚举 32 种（`crates/harness/src/workflow_types.rs:1356-1405`）。Condition 能路由（`:459-476` 支持 `judge_by_llm`+`routing_prompt`+`confidence_threshold`）、Switch 能多 case（`:867-885`）、Code 能读任意多入边（`:721-734`，实证 `analyst-brief` 一次映射 10 个分析师 `seed_stock_analysis.rs:2803-2830`）。成熟范式现成：`opc_workflows/seed_content_media.rs:1514-1525,1883-1894`、`seed_production.rs:199-231,371-386`、构造器封装 `seed_domain_helpers.rs:186-233`（判断节点必须配 `EdgeType::ConditionTrue/False` 出边 `:180-185`）。 |

### 需求⑦：数据源按四周期补齐 + 知识库退为后台备查

| 子项 | 定级 | 现状（实证） |
|---|---|---|
| 实时逐笔 | ✖ | 零通路（见需求①表）。 |
| 龙虎榜 | ◐ | 席位级明细有（`vendors/eastmoney.rs:2440-2496` `RPT_BILLBOARD_DAILYDETAILSBUY/SELL`），日频，已进工作流（`seed_stock_analysis.rs:3795-3810`，出边 `:4982,:4987`）；缺机构/游资标记字段。全市场版**尚未纳入每日快照**（`astock-data/src/daily_snapshot.rs:51` 注明待补）。 |
| 舆情流 | ◐ | 新闻是**可回放时间流**（三级降级 vendor 回溯→`news_archive`→汇总，`astock-data/src/lib.rs:2763-2850,3397-3475`），且已有词典法情绪分实体（`types.rs:52-59`、填充 `lib.rs:2721-2733`）；股吧只有当下单行快照（`vendors/guba.rs:256-321`）。 |
| 宏观历史序列 | ✖ | 纯 mock、无历史序列、**未进工作流工具表**（`astock-data/src/macro_data.rs:10,138-151,159-249`；只有 UI 命令 `src-tauri/src/commands/stock_analysis.rs:1697-1698`）。 |
| 知识库退为后台备查 | ✅⚠ | **无对象可摘**：行业走实时工具（`seed_stock_analysis.rs:2551` `get_industry_ranking`、`:2245`、`:1977`）非 KB；公司静态资料零通路；董监高在 `astock-data/` **零命中**，只存在于后台知识图谱（`stock_analysis_setup/knowledge.rs:2641`、`harness/src/knowledge_graph.rs:532`），唯一出口是板块选股（`analysis-engine/src/knowledge_loader.rs:22`、`stock_workflow/misc.rs:116`）不进单股决策流。单股链唯一 RAG 挂载是反思记忆（`stock_analysis_setup/mod.rs:1771` `rag_source_ids=["memory:stock_reflections"]`），分析节点 `rag_source_ids` 恒空（`seed_stock_analysis.rs:2398`）。⇒ 需求要的状态**已是现状**，要做的只是**显式声明**，防止有人误建。⚠ 一处未兑现自述：`seed_serenity.rs:365` 称「行业信息改由知识库 RAG 提供」但 `:973` `rag_source_ids: vec![]`。 |
| 取数质量优先于事后降权 | ◐ | 现状双轨且**偏向事后**；「live 不留痕」是**既有设计而非遗漏**（`DegradationEntry` 本身按 as-of 语义建模）：显式失败面齐（`as_of_capability.rs:21-39`、`as_of.rs:314-339` 三档严重度 `harness/src/as_of.rs:142-150`、结构化缺席 `mcp_tools.rs:1091-1104`）；但同时——**live 模式 `record_degradation` 直接 return**（`astock-data/src/as_of.rs:324-327`）⇒ 降级留痕只存在于回放；`astock-data/src/lib.rs` 内 `Ok(vec![])` 静默空 **46 处**。 |

### 需求⑧：耗时从 12+ 分钟压到 5-6 分钟

**先纠正分解**（用户给的是「分析师 2min、对抗 4min+、分析 3min、估值与决策剩余」）：

| 环节 | 实测 | 证据 |
|---|---|---|
| 全链 | 均值 660s、最高 1084s，LLM 占 ~100%；近三次 17.3/20.9/21.1 min | `docs/audits/AUDIT-jev-adoption-stock-workflow-2026-09-20.md:12,29-49`、`.workbuddy/memory/2026-09-11.md:36-45` |
| 分析师 | 4.2 min（并发 8，10 个分析师分 2 批；`a-lockup` 单节点 248.9s 拖尾） | `seed_variables.rs:125-131`、`decision.rs:1824-1827`、`.workbuddy/memory/2026-09-11.md:38` |
| 对抗 | 6 辩手串行 6.1 min + convergence 1.8 min + 风险 4 并行 2.5 min（debate 容器 avg 310.5s） | `seed_stock_analysis.rs:2979-3109`、`AUDIT…:74-94` |
| **估值/算法节点** | **全链 Code+Tool 合计 <6s**（`t-valuation`/`t-scoring`/`t-risk`/`t-valuation-band` 均纯算法 ToolNode，无 LLM） | `seed_stock_analysis.rs:3712-3725,3776-3791` |
| 剩余大头 | **串行尾部 6.3 min**（`decision-explainer`、`quality-fallback` 每次必触发 ~40s、`rule-check`） | `.workbuddy/memory/2026-09-11.md:36-45`、`AUDIT…:69-77` |

⇒ 「估值与决策」不是剩余主因，**尾部 LLM 串行才是**；把估值异步化收益近零。

各手段可行性：

| 手段 | 定级 | 关键事实 |
|---|---|---|
| 对抗压单轮 | ◐ | 轮数是模板变量（`seed_variables.rs:102-116`），但**只有建图时生效**，需升 `TEMPLATE_VERSION` 强制重种（`seed_stock_analysis.rs:1643` `resolve_debate_rounds(old_variables)`）；1 轮 = `bull-r1`+`bear-r1`+`convergence` = **3 次 LLM**（现 7 次）。风险辩论链与轮数无关、**不会跟着减**（`:3492-3509,3650,3681`）。 |
| 同分析师多轮合并为一次 | ✅ | `max_tool_rounds=2`（`a-catalyst=3`，`seed_stock_analysis.rs:2698-2702`）⇒ 每分析师最坏 3–4 次远端调用（`agent_executor.rs:1016,1443-1446`，strict 模式 VERDICT 兜底再加 1 次 `:1601-1640`）；而**上游 ToolNode 数据已经走 `context_sources` 进来了**（`:2685-2689`）⇒ 轮询属重复取数。同模板内「一次成型」先例 11 处（`tools=vec![]`+`max_tool_rounds=Some(0)`：`:3042,3080,3144,3206,3600,3692,4371,4438,5108,5209,5263`）。**改动最小、收益最大的一条。** |
| 提并发 | ◐ | 模板变量 `max_concurrent` 种子 8 < 10（`seed_variables.rs:125-131` 注释「3→8；DB 存量 v7 实际值仍为 3」），面板兜底 12（`StockAnalysisConfigPanel.tsx:30`）⇒ **DB 现值未实证**。双重卡口：按类型限流只过滤本批（`engine/mod.rs:3012-3021`）后 `.take(options.max_concurrent)` 再砍一刀（`:3022`）⇒ tool 的 10 也救不了。**⚠ 待核**：两处证据互斥 —— `join_next()` 排空整批后才重算 ready（`:3694`，另一 agent 据此判"硬栅栏"）vs 「节点完成即补调新就绪节点」（`:4400-4444`）。必须读码定论，它决定提并发能否消长尾。 |
| 调思考步数/换模型 | ✖（需接线） | provider 侧能力齐（`crates/providers/src/openai.rs:653-705`、`anthropic.rs:403-407`、`gemini.rs:430-452`、builder `crates/agent/src/provider_adapter.rs:194`），**工作流侧 5 处硬编码 `thinking_budget: None`**（`agent_executor.rs:1095-1097,1774-1776,2174-2176,2639-2641,2849-2851`）；`RunOptions.model_id/provider_id`（`engine/mod.rs:216-217`）在 `stock_workflow/core.rs:937-959` **未设** ⇒ 目前无法按节点换模型。股票模板统一 `temperature 0.3 / max_tokens 32768`（`seed_stock_analysis.rs:2389-2391`）。⚠ 网关侧有 4096 输出钳制 + **prefill 是延迟主因**（`risk-agg` input 115717 tokens，`.workbuddy/memory/2026-09-11.md:47`）⇒ 抬 max_tokens 无效，**压输入比压输出有效**。 |
| 估值异步 | ⚔ | 实测 <6s，**收益近零，建议不做**；且引擎**不支持 fire-and-forget**（就绪判定要求入边上游全部终态，fail-closed 级联跳过 `engine/mod.rs:2832-2907`、`dag_store.rs:143-160`；唯一松弛是 `continue_on_fail` `dag_store.rs:126-152`）。真要做需新增「可选边」语义，或照引擎钩子层 `tokio::spawn` 形态（`engine/mod.rs:5360-5367,1473`）在 post-exec 外挂。 |
| 四周期并行 + 多票批量并行 | ⚔ | 三处批量入口**全串行**：`axagent-batch-rerun.rs:537-540` 注释「刻意串行：高并发触发 LLM 供应商限流降级」；`pool_scan.rs:308,313` 循环内直接 await 且按 code 去重只留一档（`:107-132`）；`init/services.rs:2612,2653` 只 spawn 单任务。约束：`total_timeout_secs` 默认 1800s 每次 run 独立（`decision.rs:1804,1834-1837`）、DB 池 `max_connections(20)`（`crates/dao/src/db.rs:229`）、429 退避（`crates/agent/src/recovery_strategies.rs:100-128`）、**每模型节流 `request_delay_ms` 在工作流路径不生效**（`harness/src/types/provider_model.rs:326` → 只作用 `provider_adapter.rs:474`）。⇒ 加线程池前要先有速率预算，否则把「刻意串行」的原因原样踩一遍。 |

**「四路并行 + 单轮对抗」调用数账**（决定 5-6 分钟目标是否自洽）：
现在对抗层 LLM 调用 = 7 次/票（6 辩手 + convergence），风险层另 5 次。
若四路各跑单轮 = 4×2 辩手 + 4×convergence = **12 次**（+风险四路则 20 次）。
⇒ 除非同时做「分析师合并为一次」（每分析师省 1–3 次 ×10 人）与「风险辩论合并」，
**四路并行本身是增耗时的**，它靠并行度换墙钟、不换 token 量。这条账必须在拍板前摆明。

### 需求⑨：妖股识别（规则初筛→人工确认打标→反思读错例→调超短资金/情绪权重）

| 子项 | 定级 | 现状（实证） |
|---|---|---|
| 从历史样本捞候选 | ◐ | 引擎已有：`mover_recall.rs` 按窗口累计复利涨幅 ≥ 逐档阈值（出厂 10/20/30/40，`:35-45,64-82,96-105`；窗口天数取 `Period::default_holding_days()` 单源 `:516-557`；阈值变量 `seed_variables.rs:830-857`），候选域是**全市场历史收盘**（`market_daily_close` + `market_stock_universe` `:326-346,473-494`）。差距：①**现算不落库**（`commands/mover_recall.rs:69-211` 返回 `MoverRecallView`，misses 截 200，无 `mover_event` 实体）；②明文**禁止涨停语义**（`mover_recall.rs:9-13`）⇒ 妖股判据不能塞进它，必须独立。 |
| 三条初筛判据 | ✖ | 涨幅：有（绝对累计，非偏离）；**涨幅偏离**无独立字段（只在龙虎榜 `EXPLANATION` 文本 `vendors/eastmoney.rs:2495`；注意 `astock-data/src/scoring.rs:16,44` 的 `deviation` 是**均线乖离**，不是涨幅偏离）；**换手率**有字段未成判据（`types.rs:323`、`scale.rs:172`、批量列 `batch.rs:156`，唯一算分处是关注度权重 25% `mcp_tools.rs:4580-4648`；历史选股器不吃 `screen_stocks` —— `stock_analysis.rs:2734-2756` 只喂自选/热股）；**连板数零字段**（`vendors/ths.rs:305-338` 解析未取；前端仅估算 `src/components/stock-analysis/LimitUpPanel.tsx:80-81`）。 |
| 打「妖股」标签 | ✖ | **全仓无股票标签设施**：entities 里 `tag|label|category|is_` 命中者全属 agent_profiles/workflow_template/notes/memory_items，`reco_picks`/`stock_analyses`/`watchlist_items`/`market_stock_universe` **一个都没有**（`entities/src/reco_picks.rs:12-39`、`stock_analyses.rs:7-145`、`watchlist_items.rs:6-16`）；`stock_tags` 零命中。唯一可复用的持久槽位是 `watchlist_items.notes`，**且已被当 JSON 口袋存 `group`**（`stock_analysis.rs:1377-1434`；写它的 UI 只改分组 `WatchlistPanel.tsx:88,272,355`）。前端 `Tag` 组件在股票面板**全是展示用**、无 invoke 写入（`AnalystReportCard.tsx:488-499`、`SerenityCandidateCard.tsx:172-182`）。「人工确认」既有通路只有交易意图改状态（⚠ 2026-11-13 订正：原引用指向宿主命令面 8 条（47–69 行），已随 `PLAN-shell-plus-plugins.md` §160 候选 B 整批退役，宿主文件只剩 39 行调度垫片；改状态的业务权威是 `axagent-quant/crates/analysis-engine/src/trade_intent.rs:282-299` 的 `approve_intent` / `reject_intent`）。 |
| 反思读标签错例 | ✖ | 反思输入无标签维度（见需求⑤），前端也无「按标签筛样本/看错例」入口——所有过滤只有 code/status/date/horizon（`ReflectionPanel.tsx:92-94,419-436,596-601`、`MoverRecallPanel.tsx:75-77,202-290`、`HistoricalAnalysisPanel.tsx:139-142`）。 |
| 调高超短资金/情绪权重 | ✅ 有落点 | 现成注入通路：`hooks.rs:391-410` 的 `horizon_leg_weights_json`（档 × 分析师乘数表），以及 `evidence_weight::DECISION_LEG_ANALYST`。⇒ 这项不需要新建机制，需要的是「谁来写这张表」——见需求⑤写回缺口。 |

### 需求⑩：五条设计原则

| 原则 | 定级 | 现状 |
|---|---|---|
| 1 版本冻结、旧版封存 | ◐ | 版本递增有（`TEMPLATE_VERSION=122`，`seed_stock_analysis.rs:765`），但**没有"封存/隔离"语义**：历史版本样本一律进当前统计池（无 `is_current`/`archived`，见需求④）。 |
| 2 数据隔离（只传截止时点） | ◐ | 取数层成立，冒充 4 处未闭合；模型先验无法隔离（需求③）。 |
| 3 反思只吃同版本样本、历史版当错题本 | ✖ | 零版本过滤 + 无对照组概念。最近似只有 `reco_ic_gate=shadow`（`reco_loop.rs:512-540`）与 as-of A/B（`:400-410`）。 |
| 4 串行主干、旁支异步 | ◐ | 主干确为 DAG 依赖驱动 ✅；旁支异步**引擎不支持**（需求⑧）。 |
| 5 因子增删即升版、不硬改老版 | ◐ | 升版机制在；但「旧样本归档」没有（同原则 1）。 |

---

## 二、方案要点（阶段划分，每阶段可独立验收）

> 排序原则：**先保正确性（时间旅行不能带未来信息），再保归因性（版本可筛），再自动量（拓扑改造），最后保吞吐（性能）**。
> 顺序颠倒的代价：在带前视泄露的数据上做批量刷样本 + 反思学修正 = 把错误固化成权重。

| 阶段 | 内容 | 关键改动点 |
|---|---|---|
| **P0 前置测量**（不写码） | ①逐节点耗时实测（`node_executions.execution_time_ms`，字段见 `engine/mod.rs:6008-6023`）；②核 `max_concurrent` DB 现值；③**读码定论「批栅栏 vs 完成补位」**（`engine/mod.rs:3694` vs `:4400-4444`）；④60 分钟本机是否回包；⑤真库 `strategy_performance` 逐档样本量；⑥SubWorkflow 子执行**是否继承 `as_of` 上下文**（`as_of.rs:36-204` 是 task_local + 进程栈，子执行若 `spawn` 则 task_local 丢 ⇒ 直接决定 P3 选路） | `scripts/pg-connect.mjs sql`（正门；坑见文档 `PLAN` 附注） |
| **P1 时间旅行正确性** | 财报**披露日**（加列或按法定披露截止日保守推断）；as-of 一致预期 `is_estimated:true`+记降级；`get_dragon_tiger` 失败改结构化 `available:false`（不再 `Ok(vec![])`）；ths 伪龙虎榜从路由首位摘除或标注；`as_of_ctx_or_degrade` 竞态改 fail-closed；是否给 live 也留痕列为 **Q10**（现在 `as_of.rs:324-327` 直接 return，而 `DegradationEntry` 按 as-of 语义建模、改它要动模型）；vendor 尺度静默兜底 `_ => "101"` 删除；`forbidden_tools` 回放模式显式禁联网类 | `harness/src/market_data.rs:87-99`、`astock-data/src/lib.rs:1150-1160,4162-4180,3194-3201,3930-3938,4272-4285,1027-1046,303`、`astock-data/src/as_of.rs:324-327`、`vendors/{eastmoney.rs:1792,international.rs:264,baidu_stock.rs:166}`、`stock_workflow/mod.rs:37-42` |
| **P2 版本冻结与错题本** | `stock_reflections` 加 `template_version` 并填真值；所有统计（命中率/IC/prior/weight_decay/hit_rate_backtest）**按版本同代筛样**，把 `snrAnchorDays` 的窄水印升级为「版本即分母」；新 dao：按 `template_version` 联查 `workflow_template_versions` 取当时 nodes/edges/variables；`stock_analyses` 补 `factor_snapshot` + 填 `model_version` | `entities/src/stock_reflections.rs`、`reflection.rs:3442,1143,2206`、`reflection_stats.rs:406-413,449-453,470-521`、`horizon_prior.rs:60-146`、`recommender/ic.rs:166-168`、`weight_decay.rs:77`、`dao/src/repo/workflow_template.rs:344-347` |
| **P3 分析师四周期切片** | 技术分析师加 4 条入边（`totalScore_{ultra_short,short,mid,long}`）；`market-analyst.md` prompt 加逐档 VERDICT 字段（防「只有标签没正文」，沿用 I 轮收口的字段家族校验）；其余分析师按 §三 Q2 裁定分「切片组」与「适用性声明组」；`analyst-brief`（`:2803-2848`）输出带档结构 | `seed_stock_analysis.rs:2460-2471,2685-2689,4517-4520`、`src-tauri/agency_experts/stock-analysis/market-analyst.md` |
| **P4 四路并行与按档对抗** | 依 P0-⑥ 选路（SubWorkflow ×4 或静态展开 ×4），每路 = 该档对抗 + 该档风险；教训注入改「按当前路档位」（拆掉 `hooks.rs:314-338` 的主档代理）；`risk_bias` 改逐档；新门锁「档内不得读他档变量」 | 新增/改 `seed_stock_analysis.rs` 图 + `portfolio-mgr.rhai:1800-1801` + `hooks.rs` |
| **P5 规则节点（仲裁 + B 级处置）** | 跨周期仲裁 **CodeNode**（吃 4 路结论 + 信源质量），信源质量直接**接上孤儿表** `weight-tables.json:53-63` 的 `source_credibility`；B 级处置 = 新建 (维度×档) 连续异常持久计数（照 `lesson_applications.rs:26-46` 形态）+ 复用 `weight_decay.rs:78-134` 降权 + 照现有 grade Switch（`:5134-5174`）做熔断路由 | 新 CodeNode + `seed_domain_helpers.rs:186-233` 构造器 |
| **P6 反思定量修正闭环** | 输入侧按 (因子, 档, 版本, 标签) 检索错例（`stock_lesson_queries.rs` 加因子/版本维度；反思链向量库补检索）；输出侧三元组扩成「因子×档×from→to」结构化列；**执行侧规则节点/Rhai 写回 `horizon_leg_weights_json`**（`hooks.rs:391-410`），出厂 shadow；`source_reflection_id` 填真值（`reco_loop.rs:474`）；把五点对账门**真进 CI**（`audit-inject-coverage.mjs`、`check-horizon-weight-parity.mjs` 入 `scripts/ci-check.mjs`） | `reflection.rs:868-875,1953,387-466`、`stock_analysis_setup/mod.rs:1716-1723`、`portfolio_formula.rs:966`、`seed_variables.rs:2533-2552` |
| **P7 妖股识别** | 独立判据（不复用 mover，其明文禁涨停语义）：连板数解析补齐（`vendors/ths.rs:305-338`）+ 涨幅偏离新指标 + 换手暴增入工作流变量；初筛结果**落库**成新实体；新 `stock_labels` 表（不复用 `watchlist_items.notes`，已被 group 占用）+ 人工确认 UI + 反思按标签过滤 | `mover_recall.rs`（并列而非扩展）、新 entities、`ReflectionPanel.tsx` 过滤项 |
| **P8 吞吐** | 顺序即收益：①`max_tool_rounds`→0/1 + `tools=[]`（照 11 处先例）；②对抗 1 轮（**必须升 TEMPLATE_VERSION 重种**）；③风险辩论与主辩论合并；④尾部 LLM（`decision-explainer`/`quality-fallback`/`rule-check`）移出关键路径或改按需触发；⑤prompt 输入瘦身（`risk-agg` 115717 tokens 是延迟主因）；⑥`thinking_budget`/节点级模型接线（5 处硬编码 None）；⑦多票批量并行（带速率预算，先修工作流路径不生效的 `request_delay_ms`） | `seed_stock_analysis.rs:2698-2709,2389-2391`、`agent_executor.rs:1095-1097` 等、`engine/mod.rs:216-217`、`pool_scan.rs:107-132,308`、`axagent-batch-rerun.rs:537-540` |

**明确不做**：估值异步（实测 <6s，收益近零，且需为它扩引擎边语义）；知识库"退为备查"的摘除动作（无对象，只补显式声明）；逐笔/L2 通路（本仓无 vendor 支撑，属独立数据采购，不在本轮）。

---

## 三、需拍板（按推荐值给，认可即照此实施）

| # | 议题 | 选项与推荐 |
|---|---|---|
| Q1 | 四路并行落地形态 | **(a) SubWorkflow ×4**（父图 4 节点同批就绪即并行，档间天然变量隔离，图不膨胀）**[推荐，前提=P0-⑥ 证明子执行继承 as_of]**；(b) 静态展开 4 份辩手容器（无 spawn 风险、调试直观，节点数 +18）。 |
| Q2 | 除技术分析师外，其余 9 个是否切片 | **推荐分层**：技术/资金/情绪/催化剂 4 个逐档切片；基本面/估值/政策/研报/解禁/行业 6 个改为**逐档适用性声明**（不适用即作废，不乘系数）。 |
| Q3 | 仲裁节点 vs 既有「不做四档 switch」裁定 | **推荐**：仲裁独立成新字段 `arbitration`，四档并列输出与按档复盘不动；仲裁只回答"资金投向哪档"，不产综合报告（符合需求⑥"不汇总"）。 |
| Q4 | 反思定量修正是否自动写回分析决策链 | **推荐先 shadow**（照荐股链 `reco_ic_gate=shadow` 出厂形态），同代样本 ≥ 门槛后由你放行 on。 |
| Q5 | 对抗压单轮是否连风险辩论一起减 | **推荐一起**（否则风险层 5 次调用不动，单轮收益被吃掉一半）。 |
| Q6 | 时间旅行样本的"有效性"定义 | 模型先验无法隔离 ⇒ **推荐把目标降格为「同版本内的相对对照/错题本」，不宣称"绝对纯净样本"**；并加输出侧检测（prompt 禁提截止日后事件 + 检出未来日期/事件即标废样）。若你要求"绝对纯净"，则需求③的批量刷样本目标本身不成立，需重设。 |
| Q7 | 财报披露日取不到时的处理 | **推荐**：按报告期 + 法定披露截止日保守推断，标注 `disclosureSource: reported\|estimated`；长档财报因此滞后是可接受代价，前视不可接受。 |
| Q8 | 「四路并行」与「5-6 分钟」互斥时的优先级 | 四路单轮使对抗层 LLM 调用 7→12 次。**推荐**：P4 与 P8-①②③④ **同一批落地**，P0 先把墙钟预算算完再动图；不接受只加并行不砍调用数。 |
| Q9 | 妖股标签落库形态 | **推荐**新建 `stock_labels` 表（`watchlist_items.notes` 已被 `group` 占用，塞进去是第三套语义）。 |
| Q10 | live 模式是否也记取数降级 | **推荐记**（需求⑦「取数质量优先于事后降权」若无 live 读数就无法验收），但 `DegradationEntry` 现在按 as-of 语义建模（`astock-data/src/as_of.rs:324-327` live 直接 return）⇒ 要动模型，成本 = 新增 `mode: live\|asof` 字段 + 面板分档；若你判定 live 不需要，P1 里这条撤掉。 |

---

## 四、附：本轮普查得到的两条既有缺陷（不属十项需求，但会影响其验收）

1. **荐股链 IC 无版本/水印过滤且直接自动降权**（`recommender/ic.rs:166-168` → `reco_loop.rs:347-360`）⇒ 跨代混池正在写回权重。修它属 P2 范围，但优先级应视为 P0（正确性缺陷，非改进项）。
2. **面板可调项「纸面化」的实例已被抓到**：`debate_rounds` 保存后不重展图（版本门 `DB version >= TEMPLATE_VERSION` 跳过重种）。这正是 `AGENTS.md` 五点对账纪律要防的形态，而**该纪律目前没有真门禁**（两个校验脚本都不在 `scripts/ci-check.mjs`）。建议 P6 一并收口。

---

## 五、拍板结果（2026-10-03 已批，按推荐值整批通过）

| # | 定案 |
|---|---|
| Q1 | 四路并行 = **SubWorkflow ×4**（前提 P0-⑥：子执行必须继承 `as_of`，否则回落 (b) 静态展开） |
| Q2 | 分析师分层：技术/资金/情绪/催化剂 **逐档切片**；基本面/估值/政策/研报/解禁/行业 **逐档适用性声明**（不适用即作废，不乘系数） |
| Q3 | 仲裁 = 新增 `arbitration` 字段（只答「资金投向哪档」），四档并列输出与按档复盘**不动** |
| Q4 | 反思定量修正**先 shadow**（照荐股链 `reco_ic_gate` 出厂形态），同代样本达门槛后由用户放行 on |
| Q5 | 对抗压单轮**连风险辩论一起减** |
| Q6 | 时间旅行样本定性为「**同版本内的相对对照 / 错题本**」，不宣称绝对纯净；补输出侧未来信息检测 |
| Q7 | 财报披露日取不到 ⇒ 按报告期 + 法定披露截止日**保守推断**，标 `disclosureSource: reported\|estimated` |
| Q8 | P4（四路并行）与 P8-①②③④（压调用数）**同批落地**，P0 先算完墙钟预算 |
| Q9 | 新建 `stock_labels` 表，不复用 `watchlist_items.notes` |
| Q10 | live 模式**也记**取数降级 ⇒ 给 `DegradationEntry` 加 `mode`，面板按 mode 分档 |

**执行序**：P0（测量）→ P1（时间旅行正确性）→ P2（版本冻结与错题本）→ P3（分析师切片）→ P4（四路并行）→
P5（规则节点）→ P6（反思定量闭环）→ P7（妖股识别）→ P8（吞吐）。
按 `AGENTS.md`：阶段内定点 `cargo clippy -p <crate> --all-targets`，**全部阶段完成后**跑一次全量三门禁。

## 六、P0 读码定论（2026-10-03，两处推翻初判）

**P0-③「批栅栏 vs 完成补位」= 有补位，不是栅栏。** `engine/mod.rs:4400-4444` 是
`inter-batch early scheduling`：每个节点跑完立刻重算 ready 并补 spawn，上限由
`active_nodes.len() >= options.max_concurrent` 控制。⇒ **提 `max_concurrent` 就能直接消 10 分析师分 2 批的排队**，
不需要改调度器。初判「墙钟 = max(8)+max(2)」作废。

**P0-⑥ as-of 跨 spawn 的传播路径 = 靠进程级全局栈，不靠 task_local。** 链路是：
`stock_workflow/core.rs:907-921` spawn 前 `current_as_of()` 捕获 → spawn 内 `with_optional_asof` 重进作用域
（该函数**同时**设 task_local 与全局栈，`astock-data/src/as_of.rs:91-100`）→ 但每个节点又被
`join_set.spawn` 成新任务 ⇒ 节点内 task_local **不可见**，`current_as_of()` 回落到**全局栈栈顶**。

- 对 Q1 的影响：SubWorkflow 子执行（`spawn_blocking` + thread-local runtime，`subworkflow_executor.rs:32-35`）
  **确实能拿到截止日**（走全局兜底）⇒ Q1 选路 (a) 成立，无需回落 (b)。
- **但带出一条新的正确性缺陷（登记为 P1-8，且是 P8-⑦ 的前置）**：as-of 读取实际是**进程级单值**。
  一旦按 P8-⑦ 做「多票批量并行」且各票 as-of 不同，或批量回放与用户手动 live 分析并发，
  两条 run 会互相看见对方的截止日 ⇒ 正是 `as_of.rs:64-70` 记录的 2026-09-23 事故同类
  （那次是全局残留把批量反思锁进回放模式，一条 `strategy_performance` 都没写）。
  ⇒ 修法：把 `as_of` 存进 `ExecutionState`，在**每个** spawn 出的节点任务内部重进 `AS_OF.scope(...)`，
  使节点读取以 task_local 为准；全局栈退为「显式非任务路径」的兜底而不是主通道。
  **P8-⑦ 未在此之前落地不得开工。**

## 七、P0 实测读数（2026-10-03，六项全回，其中三处改写本文前面的判断）

### P0-① 逐节点耗时（真库 `workflow_executions.node_executions`，最近 20 次 stock-analysis）
总时长 384k–712k ms（**6.4–11.9 min**），节点耗时之和 1.07M–1.74M ms（并行放大 2.5–3×）。
排名（avg / max / calls）：`debate-bull-bear` **174s / 257s / 20**（单节点占总时长 ~1/3）、
`a-sentiment` 73s、`a-market-analyst` 67.6s / **189s max / 23 次**、`a-sector` 66s、`a-catalyst` 66s、
`a-fundamentals` 64s、`a-hot-money` 51s、`a-research` 49s / 23 次、`risk-neu` 49s、`risk-con` 41s、
`trader` 41s、`risk-agg` 40s、`rule-check` 35s / **169s max**；取数类 5.6–19.4s；
门控/规则类（`p-analysts`/`t-risk`/`pace-calc`/`quality-gate`/`store-result`）**< 100ms**
⇒ **实证「估值与算法节点不是耗时来源」**，P8 里「估值异步」确认为不做。

- ⚠ **20 次全部 `partially_completed`**（不是 completed）⇒ 主链路尾部有节点稳定不达标，需单独归因（新增 P8-0）。
- ⚠ `calls > 20` = 重试痕迹：`a-market-analyst` 23、`a-research` 23、`t-pledge-data` 22、
  `a-catalyst`/`a-hot-money`/`t-catalyst-data`/`t-sector-data` 21 ⇒ 重试本身在贡献墙钟（`max_retries=3`）。
- ⚠ `t-scoring-hour` / `t-scoring-quarter` 只有 **14 次**（其余节点 20）⇒
  **60 分钟与季度评分在 30% 的运行里没出数**（`scoreSource=daily_fallback` 的真实发生率）。

### P0-② 模板变量现值（真库 `workflow_templates.variables`，id=`stock-analysis`，version=122）
**`max_concurrent = 10`**（不是种子默认 8）⇒ **本文 §需求⑧「10 个分析师分 2 批」的判断作废**：
10 分析师正好一批，排队不来自这个上限。`debate_rounds=3`、`horizon_prior_kappa=20`、`risk_budget_pct=1.5`、
**波动率倍数实名是 `stop_vol_mult=1.2` / `take_profit_vol_mult=2.0`**（`pm_stop_vol_k1`/`pm_take_profit_vol_k2` 不存在）；
超时实名是 `agent_timeout_secs=300` / `tool_timeout_secs=30`（**`total_timeout_secs` 这个变量在本库不存在**）；
`reco_*` 一组与 `mc_default_volatility=0.3`、`pos_max_total=10`、`risk_vol_low/high/extreme=25/40/60`。

### P0-④ 60 分钟 K 线本机可达性 → **可用但降级**
| 源 | 结论 | 关键证据 |
|---|---|---|
| **新浪** | **可达且够样本（唯一正门）** | `CN_MarketData.getKLineData?scale=60&datalen=320` 稳定 200，320/320 根，日 4 根；`datalen=2000` 可回到 2024-09 ⇒ **as-of 靠「取足量 + 客户端截断」可行** |
| 腾讯 | **代码里的 URL 是坏的** | `vendors/tencent.rs:372` 拼 `web.ifzq.gtimg.cn/appstock/app/kline/mkline` ⇒ HTTP 301 → `web3.ifzq.gtimg.cn`，该域名**无 A 记录（ENOTFOUND）**，reqwest 跟 301 必失败；同 path 换 `ifzq.gtimg.cn` 直接 200 / 320 根（深度上限 482 根，start/end 位对分钟线无效）⇒ 新增 **P1-9 修 host** |
| 东财 | **连接级不可达** | `push2his.../klt=60` 29 次里 26 次 ECONNRESET/socket hang up；放行的一次只 124 根、`end=20260901` 44 根 ⇒ **跌破 `min_bars=60` 直接拒出分** |
| 雪球 | 需会话态，无法裸探 | `xq_get` 必带 `xq_a_token`，无 token 回 400/400016（端点本身可达） |
| mootdx | 通道到得了、拿不到数据 | 24 台 8 台 TCP 通、setup 握手过，但 0x052d 只回 2 字节桩包；另 `parse_bars:500` 对 category=7 **只格式化到日 ⇒ 丢时:分** |

⇒ 新增 **P1-10**：小时线路由首选改新浪；`browser_eastmoney.rs:226-231` 的 klt 映射**没有 `"60"` 分支**
⇒ 落 `_ => "101"` = webview 兜底会**静默把 60 分钟换成日线**（正是 P1-7 那条静默兜底的确证站点之一）。

### P0-⑤ 逐档先验样本量 → **实测为 0（把「未实证」变实证）**
`strategy_performance` 全表 **8 行**，`period` 只有 `reflection` 一个值，`horizon_pnl_json` **8/8 为空**
⇒ **逐档样本量 = 0** ⇒ Phase C 的经验贝叶斯收缩当前**恒等价于 `pooled`/`neutral_default`**。
这不是「等样本积累」的问题：`horizon_pnl_json` 全空说明**逐档盈亏根本没写进去** ⇒ P2 必须先修写入侧，
否则先验永远拿不到 n_h。（同时 `recommender/ic.rs` 的配对也在这块空数据上算。）

### P0-补充：需求④「版本号已记录」需下调为 ◐
真库 `stock_analyses` 79 行：`template_version` **NULL 44 行 / 非 NULL 35 行**，19 个 distinct 值（65–120），
而 **created_at 最新那行仍是 NULL** ⇒ 列与写点都在，但**现网并没有真的在记** ⇒ P2 要把「每条分析必带版本号」
做成门禁（缺版即红），否则「按版本筛样」拿不到东西。
另 `stock_reflections` **根本没有 `template_version` 列**（只 `model_version`，19/19 NULL），
`horizon` **19/19 全 NULL** —— 复核结论（同日补测）：`max(created_at)=2026-09-28`，而「一次反思 = 一个周期」的盖章写入是 09-28 起、逐档建行是 09-30 起 ⇒ **这 19 行全部早于该特性**，属历史遗留而非「写了没生效」。真实状态是**该特性落地后库里还没有一条反思样本** ⇒ 需要跑一次反思才有读数，不能据 NULL 判它坏。
`workflow_template_versions` 里 stock-analysis 快照最大 version=**120** < 现值 122 ⇒
**「按版本号调取当时工作流」对 v121/v122 无快照可取**，P2 要补「存版」这一步本身。
`as_of_date` 侧：79/79 行**都带 as_of_date**（含 74 行 `analysis_kind=live`）⇒
**「`as_of_date IS NULL` 判 live」这个判据在本库不成立**，任何按它分池的统计都是错的。

### P0-补充：趋势智选逐档落库已实证
`reco_picks` 最近一批（2026-10-02）6 行、`period` = **{mid, long}**、同 3 只代码 ⇒
Q2「逐档各落一行」在库里成立，且档位集合与 `style_matrix` 的 {mid,long} 一致。

## 八、P1 落地记录（2026-10-03）

| # | 内容 | 改动位置 | 判据/回归锁 |
|---|---|---|---|
| P1-1 | 财报**披露日**判据（Q7）：`FinancialReport` 加 `disclosure_date`；`statutory_disclosure_deadline()` 按 A 股四类期末推法定截止日（12-31 → **次年** 04-30）；`effective_disclosure_date()` 返回 `(日期, reported\|estimated)`。as-of 截断从「报告期 ≤ 截止日」改为「**披露日** ≤ 截止日」，推断值要求**严格已过**、自证公告日当天即算；非期末报告期 ⇒ 丢弃并记 `Failure` 级降级 | `crates/harness/src/market_data.rs`（字段+两方法）、`crates/astock-data/src/lib.rs::truncate_financials_by_asof`、12 处 vendor/test 构造点 | harness `financial_disclosure_tests` 4 条（含跨年与非期末不推断）；astock-data 新增 5 条，其中 `truncate_financials_drops_reports_not_yet_disclosed_at_asof` **先断言旧判据会留它**，故新判据不成立时必红 |
| P1-1b | 基本面报告露出「何时可见」：`FundamentalsReport` 加 `disclosure_date`/`disclosure_source` + markdown 一行 `披露可见性：…（按法定披露截止日推断）` | `crates/astock-data/src/fundamentals_report.rs` | 由 P1-1 的派生函数单源供给（不另算一遍） |
| P1-2 | as-of 下用最近财报 trailing EPS 充当一致预期时，`is_estimated` 由 **`false` 改为 `true`** + `estimate_source=as_of_proxy_trailing_eps_from_latest_report` + 记一条 `NoData` 降级 | `crates/astock-data/src/lib.rs::get_consensus_eps` | — |
| P1-3 | 龙虎榜**真故障不再洗成空数组**：closure 里「源答完、答案零行」改用 `DataError::NotFound`，外层据此分流 ⇒ 零行 ⇒ `Ok(vec![])`（真缺席），其余错误 ⇒ **上抛 + 记 `Failure` 降级**（此前两者同为 `Ok(vec![])`，「接口挂了」读起来和「没上榜」一模一样） | `crates/astock-data/src/lib.rs::get_dragon_tiger` + 新 `dragon_tiger_absent_is_legit_answer` | — |
| P1-4 | 摘除**拼出来的伪龙虎榜**：`market_dragon_tiger` 路由首位 `ths` 移除（它用涨停池拼，金额恒 0、日期空串）；ths 侧实现改为显式 `VendorError` 并删掉拼装代码（不留 `#[allow]`、不留死函数） | `crates/astock-data/src/lib.rs`（路由）、`vendors/ths.rs` | — |
| P1-5 | `as_of_ctx_or_degrade` 竞态由 **fail-open 改 fail-closed**：14 处 `truncate_*_by_asof` 拿不到上下文时改为**丢弃数据**（原为原样返回未截断数据 = 把截止日之后的行情交给分析师）。原注释「不截断是更安全的失败方向」的论证被逐字纠正 —— 它把 live 的正确行为搬到了回放里 | `crates/astock-data/src/lib.rs`（14 站点 + 函数文档） | — |
| P1-6 | **live 也记降级**（Q10）：`DegradationEntry` 加 `mode`（`asOf`\|`live`），`record_degradation_kind` 不再对 live 早退。同时把豁免面与面板按 mode 关死 —— `global_degraded_methods_since_baseline()` 只认回放条目；`get_asof_degradation_log/count` 走新的 `global_degradations_for_mode("asOf")`。⇒ 实盘降级有读数，但**不会**冒充回放降级、也不改变 live 决策打分（`pm_asof_degraded_methods` 本就 live 恒 `[]`） | `crates/harness/src/as_of.rs`、`crates/astock-data/src/as_of.rs`、`src/commands/stock_analysis.rs` | — |
| P1-7 | **删掉全部静默周期兜底**：5 处 `_ => "101"`/`_ => "day"` 改为显式失败并点名白名单（eastmoney live+as-of 两处、international、baidu、xueqiu 由闭包返 `&str` 改 `Option`）；`browser_eastmoney` 另补上原本**缺失的 15/30/60 分支**（此前小时线经 webview 兜底会静默变日线而 `scoreSource` 仍标 `tier_native`） | `vendors/{eastmoney,international,baidu_stock,xueqiu,browser_eastmoney}.rs` | `vendors/xueqiu.rs` 的 `None` 分支带 warn；显式失败交路由换源 |
| P1-8 | **as-of 按节点任务传播**：引擎新增 `NodeTaskScopeHook`（在每次 `join_set.spawn` 处把节点 future 包一层），业务层用它把本轮 `as_of` 重新进入每个节点任务。探针实证 `tokio::task_local` 无同步 `enter` ⇒ 只能包 future，已把结论写进注释。从此节点读截止日以 task_local 为准，**进程级全局栈不再是主通道** ⇒ 这是「多票批量并行回放」（P8-⑦）的安全前提 | `crates/rt-workflow/src/work_engine/engine/{mod.rs,node_state.rs}`、`src/commands/stock_workflow/core.rs` | 钩子注册入口 `node_task_scope_hook()` 让调用方无需命名 `NodeResult`（不把引擎内部类型漏成业务层依赖） |
| P1-9 | 腾讯**分钟 K 的 host 与 param 两处都是坏的**：①`web.ifzq.gtimg.cn` 上 `kline/mkline` 会 301 到 `web3.ifzq.gtimg.cn`（**无 A 记录**）⇒ 分钟线恒失败，改 `ifzq.gtimg.cn`（日 K 不重定向，不能一起改）；②实测 `param=sh600519,m60,320` 回 **HTTP 200 但零根**（320 被当成 start 位吃掉）⇒ 补回空槽 `,,320`。第二处比第一处更阴：不报错、不降级，只让分析师写「该股没有分钟数据」 | `vendors/tencent.rs`（`kline_host()` + `kline_route()`） | `minute_kline_host_is_the_non_redirecting_domain`（host 回归锁）；`fwd_and_hfq_route_to_plain_kline_endpoint` 的分钟段断言改为 `,,320` |
| P1-10 | 小时线路由：普查确认 2026-10-01 已把 `sina` 补进 `klines` 序列表，P0-④ 实测新浪是唯一「够 320 根 + 可深历史」的正门 ⇒ **无需改路由**，只需把腾讯修好（P1-9）。原计划的「路由首选改新浪」据实测撤销 | — | — |
| P1-11 | 需求③「关联网关插件」的可看守版：`providers/` 里**不存在**搜索开关（实测零命中），隔离是结构性的 ⇒ 把这条性质做成门：股票模板声明的工具 ∩ registry 的 `ToolCategory::Network` = 空集，命中即红并点名 | `src/commands/stock_analysis_setup/seed_consistency_tests.rs::assert_no_network_tools` | `network_tool_gate_fires_on_webfetch_and_stays_clean_on_real_seeds` 含**负控**（往声明文本塞一个真实联网工具名必须被抽出命中）+ 「registry 里确实存在 Network 类工具」的防空判据断言 |

**顺带纠正本文前面的一处判断**：§需求⑧ 说「10 个分析师分 2 批」不成立 —— 真库 `max_concurrent=10`（见 §七 P0-②），分析师是一批跑完。

## 九、R-11 架构定档（2026-10-03 用户裁定，**推翻 09-29 拍板③**）

> 决策路径不得再是「一个算法 + 四套参数」；必须**从分析师起**，四周期各自取该档数据、
> 走该档算法分支，得出四档不同的决策。

### 9-1 这条裁定实际要退役的现有机制（「四套参数」在代码里的三处藏身处）

| 被退役物 | 位置 | 为什么它就是「一套算法四套参数」 |
|---|---|---|
| `leg_mult(h, leg)` / `horizon_leg_multipliers()`（逐档分析师**乘数**表，Phase A） | `crates/analysis-engine/src/evidence_weight.rs`、`src/commands/portfolio-mgr.rhai:2897-2902` | 四档共用同一批 f2–f13 信号值与同一张基线权重表，只换个乘数 ⇒ 档间差异是**配比**不是**算法** |
| `pm_snr_confidence(heff, days, 28)` 的 √(h/28) 单调变换（Phase D-2 的「长线更值得」） | `crates/harness/src/indicators.rs:383`、`portfolio-mgr.rhai:2914,2921` | 四档置信 = 同一 `heff` 的单调函数 ⇒ 「四个结论」可由一个数推出，这正是被判错的形态 |
| 单份 `risk_bias` / 单份 `σ_daily` / 单份价带规则 `k·σ·√h` | `portfolio-mgr.rhai:1800,1977,2936` | 风险与退出规则跨档同构，只是 √h 缩放；超短与长的**收益分布假设不同**（见 9-2），不该同式 |
| 10 份 VERDICT 汇总成单份 `analyst-brief` + 单链 `debate-convergence` | `seed_stock_analysis.rs:2803-2848`、`:3113-3157` | 分析师跨档共享结论，对抗也只对抗一次 ⇒ 档间不可能有独立判据 |
| `prior_for(h)`（逐档经验贝叶斯先验，Phase C） | `crates/analysis-engine/src/horizon_prior.rs` | **不退役**，但降级为「分支内先验」，不再充当档间唯一差异来源 |
| `sharesPosteriorWith`（Phase F 的同 posterior 互标） | `portfolio-mgr.rhai:2971` + 门 `portfolio_mgr_same_source_facts` | 语义**反转**：新形态下它应恒为空，一旦出现互标即说明某分支又塌回共享 ⇒ 从「显式缺席」改为**红判据** |

### 9-2 四档算法分支的目标形态（每档因子集 / 判据 / 退出规则各自成立，且带方法学出处）

| 档 | 该档数据（尺度） | 算法分支主干 | 退出规则 | 方法学出处 | 该档**作废**的因子（显式声明，不乘系数） |
|---|---|---|---|---|---|
| 超短 2d | 60 分钟 K（新浪正门）+ 龙虎榜席位 + 热榜/情绪 + 换手异动 | 微观结构 + 短窗动量延续：放量滞涨/量价背离、封单与连板语义、资金份额 | **时间止损**（到期即出）+ 移动止盈 | 短窗动量（Jegadeesh–Titman）、信息份额（Hasbrouck）；A 股涨停制度使价格发现跨日未完 | 估值、DCF、一致预期、ROE 质量（兑现周期 > 持有期 ⇒ 证据与判定错配） |
| 短 5d | 日线 + 周线 + 5 日累计资金流 + 催化剂日历 + 解禁/减持供给 | 事件驱动波段：持有期内事件的概率 × 幅度，叠加趋势强度（MA 斜率/ADX） | 事件兑现即出 + 固定止损 | 事件研究法（event study） | 长期均值回归项、行业景气周期 |
| 中 28d | 周线 + 月线 + 财报窗口 + 一致预期修正 | 趋势 × 中期反转混合：回归项按半衰期起权，估值带宽给边界 | 对称价带 `k·σ√h` 在此档**成立**（近正态） | 中期反转（3–12 月，Jegadeesh 1990）本档取其下沿 | 分钟级微观结构（信息衰减快于持有期） |
| 长 90d | 月线/季线聚合 + 财报多期 + 宏观 + 行业景气 | 基本面质量 + 安全边际：FCF/ROE-TTM 趋势、DCF 上行空间、估值分位；**技术腿只作入场时机过滤，不参与方向合成** | 目标价 + 基本面证伪出，不设对称止盈 | Graham 安全边际、FCF 贴现 | 短窗动量、换手异动、龙虎榜资金 |

共享面只允许有两类：**算子层**（`astock-data::scale` / `scoring` / `harness::indicators`）与**数据通道**（每票取一次、逐档切片）。
**不得共享**：因子集、权重表、阈值标定口径、价带与退出规则、证据完整度分母、置信变换、action 阶梯常量。

### 9-3 图形态（P4 由此改写）

```
取数层（共享一次，按 P1 正确性口径） ──┬─ SubWorkflow: horizon-ultra-short ─┐
                                      ├─ SubWorkflow: horizon-short      ─┤→ P5 仲裁 CodeNode
                                      ├─ SubWorkflow: horizon-mid        ─┤   （只答「资金投向哪档」）
                                      └─ SubWorkflow: horizon-long       ─┘
每个子图 = 该档分析师子集（prompt 逐档）→ 该档尺度评分 → 该档正反方单轮对抗 + 收敛 → 该档风险腿 → **该档决策 Rhai 脚本**
```

- 4 份子决策脚本各自入库、各自 `template_version` ⇒ 反思按 **(档, 版本)** 筛同代样本（P2 口径随之加一档维度）。
- 子图间不串味由结构保证：父图不再向子图传任何「档无关的结论」，只传原始数据切片。
- 引擎侧依赖：SubWorkflow 已支持；`as_of` 跨任务传播已由 **P1-8** 打通（否则 4 路子执行会共用进程级截止日）。

### 9-4 代价账（为什么 R-11 与 P8-① 必须同批）

| 项 | 现状 | 四分支后 |
|---|---|---|
| 分析师 LLM 调用 | 10 人 × 最坏 3 轮 ≈ **20–30 次** | 19 人次（子集去重后）× **1 轮** = **19 次** |
| 对抗 | 6 辩手 + 1 收敛 = 7 | 4 × (1+1+1) = **12** |
| 风险 | 5 | 4 |
| 尾部（估值/解释/质检） | 6–8 | 6–8（不变，异步化另计） |
| 合计 | ~38–50 | **~41** |

⇒ **只有把分析师轮数压到 1（P8-①）四分支才负担得起**；否则 4 分支 ≈ 当前 1.6–2 倍调用。
墙钟：四路并行 ⇒ 关键路径 ≈ 单档（约 200s）+ 共享尾部（约 90s）≈ **5 分钟**，与需求⑧目标自洽。
并发前置：`max_concurrent` 需从 10 提到 ≥20（4 子图 × 5 分析师同时在途），但受 provider 配额与
DB 池 `max_connections(20)`（`crates/dao/src/db.rs:229`）约束 ⇒ P8-⑦ 的速率预算先算后调。
真库实测已给出警示：近 20 次执行里 `a-market-analyst`/`a-research` 因 **LLM 网关超时** failed 各 3 次、
`t-pledge-data`/`t-catalyst-data`/`t-sector-data` 节点 timeout 共 4 次 ⇒ `partially_completed` 全 20 次的成因是
**基础设施超时 + fail-closed 级联跳过**，不是四周期设计缺陷；但它意味着分支化会把这类失败**乘 4**，
所以逐档子图必须自带「该档某节点超时 ⇒ 该档显式降级、不影响另三档」的容错，而不是让一档塌了整链空。

### 9-5 P3 / P4 / P5 按 R-11 重述

- **P3′（分析师按档分叉）**：不是「给技术分析师加 4 条入边」，而是**每档子图挂自己的分析师子集与 prompt**；
  `market-analyst.md` 等按档拆成带档语义的版本；VERDICT 字段家族逐档定义（超短档不要求估值字段，长档不要求封单字段）。
  新增门：逐档 VERDICT 字段集与「该档作废因子清单」一致，缺字段 ⇒ 显式缺席而非漏吐（接 I 轮收口形态）。
- **P4′**：4 个 SubWorkflow + 4 份决策脚本 + 结构隔离；退役 9-1 前三行；`sharesPosteriorWith` 转红判据。
- **P5′**：仲裁 CodeNode 吃 4 路**结论 + 该路证据质量**（接孤儿表 `source_credibility`），输出 `arbitration`；
  逐档容错（单档失败 ⇒ 该档 `horizonStatus=degraded` 并进 data_gaps，不级联跳过另三档）。

> ⚠ 本裁定使 §五 的 Q1/Q2 从「推荐值」升级为「已定方向」，并使 §需求① 的结论
> 「诚实上限是逐档声明不适用」改为「**逐档独立算法 + 该档作废因子显式声明**」——后者是前者的超集。

## 十、P2 落地记录（2026-10-03）+ 两处自我更正

### 10-1 已落地

| 项 | 内容 | 位置 |
|---|---|---|
| P2-A | 荐股链算法归属**开始可追溯**：`reco_picks` 加 `reco_version` 列（缺列由 `dao/src/reconcile/schema_diff` 声明式补齐，本仓不走 migration）；新增唯一源常量 `recommender::RECO_ALGORITHM_VERSION`（文档写明「因子增删/判据口径/档位矩阵变动 ⇒ +1，只调参数值不升版」）；**两处建行点**（手动/定时荐股 `stock_analysis.rs`、趋势智选工作流链 `serenity.rs`）全部盖章，两链共用同一常量（分版会造出「同一张表两种归属」） | `crates/entities/src/reco_picks.rs`、`crates/analysis-engine/src/recommender/mod.rs`、`src/commands/stock_analysis.rs`、`src/commands/stock_workflow/serenity.rs` |
| P2-B | 写侧结构门 `every_reco_picks_insert_stamps_algorithm_version`：扫到 2 处建行点且每处必须含 `reco_version:`；**计数不符即红**（新增第三个写入点而未入册会被抓住 —— 这正是 `seed_node_constructors_are_not_duplicated` 那族事故的形态） | `src/commands/stock_analysis_setup/seed_consistency_tests.rs` |
| P2-C | 闸门依赖锁 `ic_gate_can_only_turn_on_after_version_screening_exists`：`RecoIcRow` 尚未带 `algorithm_version` ⇒ seed 里一旦出现 `reco_ic_gate="on"` 即红。附负控 `ic_gate_dependency_lock_detects_the_bad_pairing`（人造「seed=on」文本必须被识别，`shadow` 必须不识别） | 同上 |
| P2-D | 更正 §四 定级：需求④「结论记录版本号」经复核**写入路径是完整的**（详见 10-2） | — |

### 10-2 两处更正（我自己前面说过头的地方）

1. **「现网并没有真的在记版本号」—— 说错了。** 真库复核：78 条 `completed` 行里缺版本的都是 2026-09-20 之前的存量；
   9-20 之后只有 **1** 条缺，而那一条 `status='running'`（在途分析，尚未走到回写）。
   三个 `template_version: Set(None)` 是 **running 占位行**的显式初始化，真实版本由完成路径用
   `col_expr` 回写（主路径与降级分支都回写，见 `stock_workflow/core.rs:1161-1166`）。
   ⇒ 需求④ 回到 ✅（但「按版本筛历史工作流」的联查仍缺，那是 P2-E 未做项）。
2. **「荐股链跨代混池正在污染写回」—— 当前并未激活。** 真库复核：`stock-analysis` 模板变量里**根本没有**
   `reco_ic_gate` 这一项 ⇒ 代码走缺省 `shadow`（`reco_loop.rs:530-537`：非 `on` 直接返回、不覆盖权重）。
   所以它是「**离激活只差一个变量**」的正确性缺陷，而不是正在发生的污染 ⇒ 定级从 P0 降为
   「P2-C 依赖锁 + 翻闸前必须补 IC 筛样」。原 §四/§六 的表述按此修正。

### 10-3 P2 未做项（诚实边界，别当成已完成）

- **P2-E `IC 同代筛样`（读侧）**：需要给 `RecoIcRow` 加 `algorithm_version` 并改样本装载链
  （`reco_loop.rs:97-123` 的行来自一条跨表查询，字段集要先扩 SELECT 再传递）。**未做**，
  由 P2-C 的依赖锁保证它做完之前闸翻不到 on。
- **P2-F 按版本联查历史工作流**（`workflow_template_versions` ↔ `stock_analyses.template_version`）与
  `axagent-batch-rerun --template-version`：**未做**。且实测发现该快照表对 stock-analysis 只到 **v120**
  （现值 122）⇒ 先得让「存版」这件事完整，联查才有东西可查。
- **P2-G `factor_snapshot` / `model_version` 真值**：`model_version` 改判为**退役**（2026-10-05，§五十九），
  不再补真值；`factor_snapshot` **未做**。

### 10-4 P3′ 按 R-11 的具体分析师分叉（待实施，逐档子集的因子作废声明同步落文档）

| 档 | 子工作流挂载的分析师 | 显式作废（不进该档证据、不乘系数） |
|---|---|---|
| 超短 | `a-market-analyst`(60min) / `a-hot-money` / `a-sentiment` / `a-catalyst` | `a-fundamentals` `a-research` `a-policy` `a-sector` `a-lockup`（兑现周期 > 持有期） |
| 短 | `a-market-analyst`(日/周) / `a-hot-money` / `a-catalyst` / `a-news` / `a-lockup` | `a-research`(一致预期修正以季为单位) `a-policy`(政策传导 > 5 日) |
| 中 | `a-market-analyst`(周/月) / `a-fundamentals` / `a-research` / `a-sentiment` / `a-policy` | `a-lockup`(供给冲击在 28 日内已被趋势吸收，降为风险提示不参与方向) |
| 长 | `a-fundamentals` / `a-research` / `a-policy` / `a-sector` / `a-catalyst` | 分钟级微观结构（信息衰减快于持有期）；技术腿**只作入场时机过滤** |

超短与长两档的**数据现实**必须在分支里显式声明，不得由近似数据顶替。以下三条原写于 P0，
**2026-10-03 已按 P9 实装结果更正**（原文的「零字段 / mock / 全 Fallthrough」都已不成立）：

| 原表述（已作废） | 现状 | 仍缺什么 |
|---|---|---|
| 超短「连板数零字段」 | P9-4 同花顺涨停池给到连板数 + 涨停天数 + 封单额/量 + 炸板次数 + 板型，且**按日可回溯** | 逐笔 / L2 撮合明细（P9-6 待探）⇒ `microstructure` 仍标 `partialSource` |
| 长档「宏观历史序列是 mock」 | P9-1 真源东财 datacenter，五条序列（CPI/PPI/PMI/非制造业 PMI/GDP），按**发布日**裁 | M1/M2/社融/新增贷款/LPR/Shibor/MLF/外储 等仍是结构体字段无供应 ⇒ 缺席子集标 `macroSourceUnavailable` |
| 「一致预期全 vendor Fallthrough ⇒ trailing EPS 代理」 | P9-3 官方 `RPT_WEB_RESPREDICT` 的 `YEAR_MARK="E"` 聚合行，代理已删 | 该行**没有日期列** ⇒ `NoHistoricalSemantic`，回放要等 P9-5 归档 |

另外两项 P0 结论**未变**：股吧情绪时序只有当日快照（`hot_rank` 现在同平台真填，但仍是当日榜、
且 as-of 刻意不取），逐笔/L2 仍零通路。

## 十一、P4′ 实施清单（R-11 落地用，可直接照此改图）

### 11-1 新增资产

| 资产 | 内容 |
|---|---|
| 4 个子模板 | `stock-analysis-h-ultra-short` / `-h-short` / `-h-mid` / `-h-long`，各自 `TEMPLATE_VERSION` 从 **1** 起（与主模板版本**互不相干**，反思按 `(子模板 id, version)` 筛同代） |
| 4 份决策脚本 | `portfolio-mgr-h-ultra-short.rhai` … `-long.rhai`。每份**只**吃本档证据；`build.rs` 会自动收集 `#[tauri::command]`，但 Rhai 脚本要走 `include_str!` + seed 注入 ⇒ 同步扩 `seed_stock_analysis.rs` 的脚本常量与 `portfolio_mgr_same_source_rhai.rs` 的逐字副本门 |
| ~~4 套分析师 prompt~~ **已作废（2026-10-03，见 §二十三）**：不派生四份 md，改为「1 套专家 md + 执行期按 `Period::verdict_contract_prompt()` 注入逐档契约」。派生四份就是造四份与权威表漂移的清单源；注入方案下「不拆节点」仍成立（同一 expert，各分支实例注入各自档位）。**子模板因此必须注入 `horizon`** —— 这是 4j 段的唯一触发条件 |
| 父图 4 节点 | `WorkflowNode::SubWorkflow`，`output_var` 分别 `h_ultra_short` / `h_short` / `h_mid` / `h_long`；`input_mapping` 只允许映射**数据类**上游（`kline_*`、`fin_*`、`mf_*`、`news_*`、`dt_*`、`quote`），**禁止**映射任何档无关结论 |

### 11-2 边（父图）

```
t-market-data / t-financials-data / t-money-flow / t-news / t-dragon-tiger-data / t-lockup-data …
        └──(全部为 ToolNode 取数，一次)──→  h-ultra-short   h-short   h-mid   h-long
                                                        └───────┴───────┴────────┘
                                                                    ↓
                                                        arbiter (CodeNode, P5′)
                                                                    ↓
                                                    store-result / notify-result
```
四个 SubWorkflow 同层就绪 ⇒ 引擎现有 `JoinSet` 天然并行（P0-③ 实证有 inter-batch early scheduling，无需改调度器）。

### 11-3 退役清单（**同一批**删，不留兼容 shim）

| 退役 | 位置 | 替代 |
|---|---|---|
| `leg_mult` / `horizon_leg_multipliers` / `DECISION_LEG_ANALYST` 的乘数语义 | `analysis-engine/src/evidence_weight.rs`、4 份新 rhai 不再出现 `leg_mult.call` | 每档自己的腿集合与权重（分支内声明） |
| `pm_snr_confidence` 的 √(h/锚) 变换作为档间唯一差异 | `harness/src/indicators.rs`、`portfolio-mgr.rhai:2914,2921` | 各档自己的置信推导（超短可用事件命中口径，长档用安全边际带宽） |
| `horizon_decision` 单闭包产四档 | `portfolio-mgr.rhai:2893-2989` | 4 份脚本各自本体 |
| `risk_bias` 跨档同值 | `portfolio-mgr.rhai:1800,1872,2910` | 逐档风险腿（子图内 `t-risk-h*`） |
| 单一 `analyst-brief` + `debate-convergence` | `seed_stock_analysis.rs:2803-2848`、`:3113-3157` | 子图内逐档 brief/debate/convergence |
| `sharesPosteriorWith` 作为「显式缺席」 | 同上 + 前端 `DecisionBanner.tsx:1524-1537` | **语义反转**：非空即门红（说明某分支塌回共享） |
| `totalScore_{ultra_short,short,mid,long}` 的四入边映射 | `seed_stock_analysis.rs:4517-4520` | 每档子图只取**自己尺度**的评分节点 |

### 11-4 版本与门禁

- 主模板 `TEMPLATE_VERSION` 122 → **123**（图结构变更必须重种；实证：门跳过重种的判据是 `DB version >= TEMPLATE_VERSION`）。
- 4 个子模板各自的版本变量进 `seed_variables.rs` 时**按五点对账**逐条走完（本轮已把 `audit-inject-coverage.mjs --strict` 与 `check-horizon-weight-parity.mjs` 提进 CI，`ci-check.mjs` 与 `ci.yml` 双侧同步 ⇒ 漏一处现在会红）。
- 新门（P4′ 自带，命名建议 `check-horizon-branch-parity.mjs`，`--selftest` 负样本齐）：
  a. 四份 rhai 决策脚本**不得**出现 `leg_mult`/`snr_confidence`/`horizon_decision` 字样（出现即说明又走回共享算法）；
  b. 每个子模板的 `input_mapping` 不得引用其它档的输出键（串味判据）；
  c. 逐档 VERDICT 字段集 == 该档声明的适用因子集（缺字段 ⇒ 显式缺席，不许静默）；
  d. `sharesPosteriorWith` 全档为空（11-3 最后一行的反转）；
  e. 四档 action 阶梯常量各自独立、且不得由「同一常量 ±偏移」派生。
  f. **每个子模板必须注入 `horizon`**（`context.variables` 里有该键，或节点 `input_mapping` 的
     target_key 为 `horizon`）。缺它则 4j 段静默不注入 ⇒ 契约不生效、分析师照旧只出通用 6 键，
     而现场看起来像「模型不守约」—— 这是本步最隐蔽的失效形态，必须有门。
  ⚠ 判据 c 的字段集来源是 `axagent_harness::Period::verdict_spec()` / `verdict_factors()`，
     门内**不得**再写一份清单（否则门与表互相指认，谁错都看不出来）。
- 引擎依赖：Q1 选 SubWorkflow ⇒ **P1-8 的 `NodeTaskScopeHook` 是硬前提**（已落）。子执行跑在
  `spawn_blocking` + thread-local runtime（`subworkflow_executor.rs:32-35`），钩子只在**父图节点任务**边界生效，
  子图内部的节点任务由子执行自己的 `run_workflow` 走同一条 spawn 路径 ⇒ 需在 P4′ 实跑时验证截止日确实穿透到孙子任务
  （验证方法：子图里放一个取 K 线的节点，回放时断言其末根日期 ≤ 截止日）。

### 11-5 已知的失败放大（必须同时处理，否则四分支比现在更脆）

真库实测近 20 次执行：`a-market-analyst` / `a-research` 因 **LLM 网关超时**各失败 3 次、
`t-pledge-data`/`t-catalyst-data`/`t-sector-data` 节点 timeout 4 次，全部 20 次 `partially_completed`。
分支化后同一抖动的作用面从 1 变 4 ⇒ P5′ 的逐档容错是**功能项而非优化项**：
单档子图失败 ⇒ 该档 `horizonStatus="degraded"` + 一条 data_gaps，其余三档照常出；
仲裁节点在缺席档存在时**不得**把它们当低置信参与裁决（缺席 ≠ 看空）。

## 十二、P3′ 逐档 VERDICT 字段集规格（prompt 与门禁的共用权威表）

> 规则：**字段存在 = 该档确实算了这件事**；字段缺席 = 结构性不适用，必须由 prompt 显式写「不适用」而非省略，
> 否则就退回 I 轮那个「只有标签没正文」的形态。因此下表同时是 prompt 模板、Rhai 解析、
> `check-horizon-branch-parity.mjs` 的判据 c（字段集 == 适用因子集）三方的唯一源。

| 字段 | 超短 2d | 短 5d | 中 28d | 长 90d | 缺席时的显式文案键 |
|---|---|---|---|---|---|
| `direction`（方向） | ✅ | ✅ | ✅ | ✅ | — |
| `confidence`（0-1 胜率，**各档自己算**，不得由共同 heff 变换而来） | ✅ | ✅ | ✅ | ✅ | — |
| `horizon`（档位标签，代码盖章非模型自报） | ✅ | ✅ | ✅ | ✅ | — |
| `momentumSignal`（短窗动量/量价背离） | ✅ | ✅ | ⛔ 不适用 | ⛔ 不适用 | `notApplicableMomentum` |
| `microstructure`（封单/连板语义） | ✅ 连板数 + 涨停天数 + 封单额/量 + 炸板次数 + 板型（P9-4 同花顺涨停池，**可按日回溯**）；⚠ 仍无逐笔/L2 撮合明细 ⇒ 仍标 `partialSource`（缺的是撮合级，不是连板级） | ⛔ | ⛔ | ⛔ | `notApplicableMicrostructure` |
| `eventCatalyst`（持有期内事件） | ✅ | ✅ | ✅（窗口内财报/政策） | ✅（只取改变基本面的） | — |
| `flowPersistence`（资金持续性，≥5 日累计） | ⛔ | ✅ | ✅ | ⛔（长期由预期替代） | `notApplicableFlow` |
| `trendStrength`（周/月线趋势） | ⛔ | ✅ | ✅ | ✅（只作入场时机过滤，**不参与方向**） | `trendFilterOnly` |
| `valuationBand`（估值分位/带宽） | ⛔ | ⛔ | ✅ | ✅ | `notApplicableValuation` |
| `earningsQuality`（ROE-TTM 趋势 / FCF） | ⛔ | ⛔ | ⚠ 部分（兑现跨季） | ✅ | `notApplicableQuality` |
| `expectationRevision`（一致预期修正） | ⛔ | ⛔ | ✅ | ✅ | `notApplicableExpectation`（真库已换官方 `RPT_WEB_RESPREDICT` 的 `YEAR_MARK="E"` 行，P1-2 那套 trailing EPS 代理**已删**；该行无日期列 ⇒ `NoHistoricalSemantic` ⇒ 回放须靠 P9-5 归档，归档起始日之前标缺席） |
| `macroRegime`（宏观序列） | ⛔ | ⛔ | ⛔ | ⚠ **部分可得**（P9-1 真源：CPI/PPI/PMI/非制造业 PMI/GDP 五条，且按**发布日**而非统计期裁）；货币社融类 M1/M2/社融/新增贷款/LPR/Shibor/MLF/外储 **仍缺席** ⇒ 缺席子集标 `macroSourceUnavailable` | `macroSourceUnavailable`（仅指缺席子集） |
| `supplyShock`（解禁/减持供给） | ⛔ | ✅ | ⚠ 降为风险提示 | ⛔ | `supplyAsRiskNoteOnly` |
| `exitRule`（`time_stop` / `k_sigma_band` / `target_and_falsified`） | `time_stop` | `time_stop`+固定止损 | `k_sigma_band` | `target_and_falsified` | — |
| `stopSource` / `priorSource` / `positionSource`（退化点名，R-11 后仍必填） | ✅ | ✅ | ✅ | ✅ | 见 §需求⑤/⑥ |

### 12-1 这张表要消灭的三种伪装

1. **把「没数据」写成「算了但分低」**：`microstructure` / `macroRegime` 两行是真库缺口（逐笔零通路、宏观 mock），
   必须走 `partialSource` / `SourceUnavailable`，不得用 60 分钟 K 顶替逐笔、不得用行业排名顶替宏观序列。
2. **把「不适用」写成「跨档共享值」**：`trendStrength` 在长档**只作入场过滤**——若代码里它仍进方向加权，
   就是 R-11 要退役的形态，判据 a（四份脚本不得出现 `leg_mult`/`snr_confidence`）会红。
3. **把「同源」写成「巧合一致」**：判据 d（`sharesPosteriorWith` 恒空）+ 各档独立置信推导，
   使「四档同分」只剩「真的收敛」这一种解释。

### 12-2 依赖与顺序

- 本节只依赖 P4′ 的子图划分；prompt 与门 a–e 一起落，缺任一条判据 ⇒ 该档不出结论（不静默补全）。
- i18n：上表 `notApplicable*` / `*Unavailable` 共 9 个键 × 11 语言 = 99 条文案，属**独立小轮**，
  且需先定「非中英语言由谁定稿」——本轮已明确拒绝用机翻占位（理由见 §十 与执行记录）。

## 十三、P9 数据源补齐（用户裁定 2026-10-03：既不按缺席收口、也不用近似顶替 ⇒ **补真实源**）

> 本节全部结论来自本机实测回包（探测脚本 `.workbuddy/tmp/probe/sources*.mjs`、`macro_names.mjs`、`macro_sort.mjs`），
> **未回包的候选一律写「不存在」，不写进实现**。这是 §十二 那张逐档字段表的替代品：那里标 `Unavailable` 的行，
> 按本节改为「接真源」或「换源再探」。

### 13-1 已证实可用的新通道

| 指标 | 端点（实测 200） | 字段 | 历史跨度 | **可否回放** |
|---|---|---|---|---|
| CPI | `datacenter-web.eastmoney.com/api/data/v1/get?reportName=RPT_ECONOMY_CPI` | `REPORT_DATE` `TIME` `NATIONAL_SAME`(同比) `NATIONAL_BASE` `NATIONAL_SEQUENTIAL`(环比) `NATIONAL_ACCUMULATE` `CITY_*` `RURAL_*` | 2006→ **2026-08** | ✅ 可（按发布日裁） |
| PPI | 同上 `RPT_ECONOMY_PPI` | `REPORT_DATE` `BASE` `BASE_SAME` `BASE_ACCUMULATE` | 2006→ | ✅ 可 |
| PMI | `RPT_ECONOMY_PMI` | `MAKE_INDEX`(制造业) `MAKE_SAME` `NMAKE_INDEX`(非制造业) | 2021→（样本所见） | ✅ 可 |
| GDP | `RPT_ECONOMY_GDP` | `DOMESTICL_PRODUCT_BASE` `FIRST/SECOND/THIRD_PRODUCT_BASE` `SUM_SAME` `*_SAME` | 2016→ | ✅ 可 |
| 一致预期（评级+EPS） | `RPT_WEB_RESPREDICT` `filter=(SECURITY_CODE%3D%22600519%22)` | `RATING_ORG_NUM` `RATING_BUY/ADD/NEUTRAL/REDUCE/SALE_NUM` `YEAR1..4` `EPS1..4` `DEC_AIMPRICEMIN/MAX` `INDUSTRY_BOARD` `CONCEPTINDEX_BOARD` `MARKET_BOARD` | 仅**当前值** | ⚠ 不可回放（见 13-3） |
| 涨停池（连板/封单/炸板/换手） | `push2ex.eastmoney.com/getTopicZTPool?...&date=YYYYMMDD` | `lbc`(**连板次数**) `hs`(换手) `fund`(封板资金) `zbc`(炸板次数) `fbt`/`lbt`(首末封板时间) `zdp` `amount` `ltsz` `c`/`n`/`p`/`m` | **仅当日**：20260930 回 50 条；20260105 / 20250630 / 20240930 / 20240102 **全部 `pool=0`** | ⚠ 不可回放（见 13-3） |

**实现注意（全是实测踩出来的，别再靠记忆写）**：
- 排序列实名是 **`REPORT_DATE`（带下划线）**。写 `REPORTDATE` 回 `code=9501 排序列不存在`；
  **不带排序时返回顺序是乱序**（实测 2013、2021、2009）⇒ 取「最新一期」必须显式 `sortColumns=REPORT_DATE&sortTypes=-1`。
- 涨停池 `sort` 必须是白名单值：`sort=fbt:asc` 有数据，**`sort=qty:desc` 回 `pool=0`**（不是「那天没涨停」，是参数非法）。
- `push2.eastmoney.com/api/qt/stock/details/get`（逐笔）形状是 `{rc,data:{details:[...csv...]}}`；
  实测**一次 200、一次 `UND_ERR_SOCKET`** ⇒ 与 push2 族累积/速率触发拒连同族，**不得当作稳定源**。

### 13-2 宏观必须按**发布日**裁，不是按统计期

`REPORT_DATE` 是统计期，CPI/PPI 实际在**次月 9–12 日**发布。按 `REPORT_DATE ≤ 截止日` 裁，
会把尚未发布的数字放进回放 ⇒ 与我刚在 P1-1 给财报修掉的前视**同一族**。
现场证据：今天 2026-10-03，CPI 最新只到 **2026-08**（9 月值尚未发布）。
⇒ 落地形态：`macro_available_at(indicator, report_date)` 单一函数 + 保守规则（CPI/PPI = 次月 15 日、
PMI = 月末 +2 日、GDP = 季后次月 20 日），来源标 `estimated_release_rule`（不得谎称公告日实测），
回放按 `available_at ≤ 截止日` 过滤，与 `FinancialReport::effective_disclosure_date` 同构。

### 13-3 两条硬上限（物理限制，不以任何声明或近似绕过）

一致预期与涨停池**只有当前值**。要让它们参与时间旅行，只能从现在开始每日归档（本仓已有
`astock-data/src/daily_snapshot.rs` + `news_archive` 的积累正门形态），**过去的样本永久不可回收**。
⇒ 影响需求③的账面：超短档「批量刷当前版有效样本」的上限 = 归档起始日之后；
在此之前，超短分支只能用 60 分钟 K（新浪，可回放）+ 已归档部分。
必须在 UI 与文档里写清「归档起始日」，否则旧回放会被读成「那几天没有连板」。

### 13-4 待办分解（P9，先于 P3′ 的对应档）

| # | 内容 | 验收 |
|---|---|---|
| P9-1 | 宏观四指标真抓取（替换 `macro_data.rs` 的 mock 与 `tracing::warn`），含 `available_at` 规则与 as-of 过滤；命令 `macro_data_snapshot` 的消费者不再拿 mock | 零网络解析测试（真实响应切片夹具）+ `available_at` 规则表测试 + 「乱序输入必须倒序」测试 |
| P9-2 | 宏观进工作流：新工具 `get_macro_indicators`（`mcp_tools.rs` schema+dispatch）、新节点 `t-macro-data`、能力申报 `NativeDateParam`、工具入 analyst context | 五点对账 + `audit-inject-coverage --strict` 绿 |
| P9-3 | 一致预期真通道接入（替换「全 vendor Fallthrough ⇒ trailing EPS 代理」），as-of 下**显式失败**而非代理 | 路由测试 + 代理路径删除（不留 `is_estimated` 双轨） |
| P9-4 | 涨停池接入（连板/封单/炸板/换手四字段），live 可用 | 解析测试（`lbc`/`hs`/`fund`/`zbc`）+ `sort` 非法值回空的负控 |
| P9-5 | 每日归档 `limit_up_pool` + `consensus_eps`（扩 `daily_snapshot.rs`），并落「归档起始日」读数 | 快照写入/读取测试 + UI 点名归档起始日 |
| P9-6 | 逐笔：换源再探（新浪/腾讯内核/webview 兜底），探到之前**不写实现** | 探测记录 |
| P9-7 | M2 / 利率 / 社融 / 固投 / 进出口 / 工业增加值：**18 个候选报表名全部「配置不存在」** ⇒ 需另找源（统计局 `easyquery`、央行），探到前同样不写 | 探测记录 |

> P3′ 依赖关系更新：长档的 `macroRegime` 与 `expectationRevision` 由「Unavailable」改为 **P9-1/P9-3 实装**；
> 超短档的 `microstructure` 由「无逐笔」改为 **P9-4 连板/封单 + P9-6 逐笔待探**。
> §十二 表中标 ⚠ 的行按本节替换，别再引用旧结论。

## 十四、P9-1 落地记录（2026-10-03，宏观真源）

**替掉了 mock**：`crates/astock-data/src/macro_data.rs` 原 `snapshot()` 返回 8 条硬编码假数 +
`tracing::warn` 自称 mock，现改为并发取 `RPT_ECONOMY_CPI / _PPI / _PMI / _GDP` 四条真实历史序列。
唯一消费方 `macro_data_snapshot` 命令（`src/commands/stock_analysis.rs:1698`）**签名不变**，
从「拿到假数」直接变成「拿到真数」。

**判据与实现要点**：
- `macro_available_at(indicator, report_date)` 保守可得日：CPI/PPI 次月 15 日、PMI 月末+2 天、GDP 次月 20 日；
  未知指标返回 `None` 不推断。`source` 串里写死「(发布日按规则推断)」⇒ 下游读得出这是规则不是公告日。
- **回放按可得日裁，不按统计期**（`pick_published`）⇒ 与 P1-1 财报披露日同构，防的是同一族前视。
- **CPI 的 `NATIONAL_SEQUENTIAL` 被丢弃**：实测 2013 行 = 1.0103、2026 行 = 0.4，同一列跨年换口径，
  填进 `mom` 会造出「环比 +101%」这种假读数（有测试锁住 `mom == None`）。
- PMI 一行同时产两条（制造业 / 非制造业），避免「非制造业没有源」的误读。
- 排序列必须 `REPORT_DATE` 且**显式倒序**；不带排序时响应乱序（实测 2013/2021/2009）。

**验收**：
- 零网络单测 6 条（可得日规则表含跨年与 2 月闰判断、**防前视主证**「10-03 时 09 期不可取、须回退 08 期，
  10-16 之后才允许取 09」、来源自证推断、环比被丢弃、PMI 双条、乱序输入仍选对期）。
- **live 冒烟 1 条**（`#[ignore]`，命令写在文档注释里）实测通过，真接口真数据：
  `cpi period=2026年08月份 value=0.8 available_at=2026-09-15`、`ppi 2026年08月份 value=3.8`、
  PMI/GDP 各取到已公布期 ⇒ 断言「取到的期次不得晚于今日可得日」在真实响应上成立。
- M2 / LPR / 社融 / 固投 / 进出口 / 工业增加值：**对应报表名实测「配置不存在」**，这些字段保持 `None`，
  等 P9-7 找到统计局/央行接口再接，不用邻近值顶。

**边界（为什么 P9-2 不在本轮做）**：把宏观接进决策流要动工具 schema、dispatch、种子节点、
`as_of_capability` 申报和 `TEMPLATE_VERSION` 重种 ⇒ 其验收必须**重启应用 + 实跑一次分析**看节点是否真出数，
这在当前会话里无法自证。按本仓纪律，不做「写了但没跑过 seed」的半成品。

## 十五、P9-3 落地记录（2026-10-03，一致预期真通道）

**先纠正我自己两句**：① 「一致预期所有 vendor 全 Fallthrough ⇒ 无真通道」不准确 —— eastmoney 早就有实现
（`reportapi/report/list` 手聚 20 条研报的 `predictThisYearEps` 取均值），只是权威弱；② 官方口径
`RPT_WEB_RESPREDICT` 才是强通道。所以这次是**升级 + 修语义**，不是从零接。

**实现**（`vendors/eastmoney.rs`）：
- 新增 `fetch_consensus_official()`（inherent impl 内）+ 自由函数 `parse_consensus_forward()`（可零网络测）。
- **语义陷阱已锁**：官方行 `YEAR1=2025 / YEAR_MARK1="A"` 是**已披露实际值**，`YEAR2=2026 / "E"` 才是预测
  ⇒ 只取第一个 mark=`E` 的年。按字段顺序取 `EPS1` 会把实际值当预期，既语义错配又前视。
- 目标价取 `DEC_AIMPRICEMIN/MAX` **中值**（1430/2030 ⇒ 1730），不拿上沿当合理估值；
  `rating_avg` 保持 `None`（官方行只给评级**分布**，不编造单一评级名）。
- 调用顺序：官方口径优先，取不到才退回原研报均值 —— **两条都是真数据**，不存在「缺数时编一个」的分支。
- 能力申报 `get_consensus_eps`：`Fallthrough` → **`NoHistoricalSemantic`**，理由写进代码：它是点时序列
  且官方行**没有任何日期列**（实测），容忍当日值等于把未来信息喂进回放。
  ⚠ 与 T12 的教训对照过：`get_concept_blocks` 当年改成 NoHistoricalSemantic 会让白名单跳过唯一活源而整维拿空，
  这里不重复那个错误 —— 路由层拿不到命中会 `record_degradation` 留痕后返回 `None`，不是静默空。

**删掉的近似顶替**：`lib.rs` 里「as-of 下用最近财报 trailing EPS 充当一致预期」整段删除（连带 P1-2 的
`is_estimated:true` + `estimate_source=as_of_proxy_trailing_eps...` 双轨）。判据换成：回放拿不到就**明确拿不到**
（记降级），可回放的历史一致预期等 P9-5 每日归档积累 —— 这正是用户裁定的「不拿近似值顶替」。

**验收**：单元测试 6 条过（含 A/E 陷阱、无预测年返回 `None`、能力申报断言）；
**live 冒烟真跑通 vendor 通道**：`year=2026 eps=67.249 target=1730 orgs=43`。

## 十六、P9-4 落地记录（2026-10-03，涨停池）+ 一处自造的失效判据

**实测否掉了「按日期取历史」这条路**：`push2ex/getTopicZTPool` 的响应带 `qdate`（实际生效日），
而请求 10-09 / 10-03 / 09-25 / 08-01 / 2025-06-30 时 **`qdate` 一律是 20260930**，
同时 `tc` 却随请求日期变（09-24→52、09-23→51、09-10→35、09-09→48、09-25→0）——09-25 是交易日却回 0。
⇒ 该参数不可信，历史维度**只能靠每日归档**（P9-5）。

**落地**：
- DTO `LimitUpPoolEntry` / `LimitUpPool`（`types.rs`）：连板数 `limit_up_streak`、换手 `turnover_rate`、
  封板资金 `seal_amount`、炸板次数 `break_count`、首末封板时间（由 `HHMMSS` 整数转 `HH:MM:SS`）、成交额、流通市值。
  ⇒ 妖股三条判据里的两条（连板、换手）从此有真字段；涨幅偏离另计（P7）。
- `LimitUpPool::is_for_requested_date()` + `format_pool_qdate()`：
  **`qdate != 请求日 ⇒ 判「该日数据不可得」，绝不判「当天没有涨停」** —— 这是本次拿到的判别器，
  也是我之前担心的「拿不到 vs 没有」混淆的唯一机械解法。
- 能力申报 `get_limit_up_pool = NoHistoricalSemantic`；vendor trait 给默认实现（其他源回 `Ok(None)`）。
- 未接进工作流（与 P9-2 同一批 seed 改动，验收要重启应用实跑），也**未**接 mcp 工具面。

**自造的失效判据（live 冒烟当场抓到）**：`pool_date` 我原本写成 `{y}-{m}-{d}` 无补零 ⇒
实测打印 `2026-9-30`，与任何 ISO 日期都不等 ⇒ `is_for_requested_date()` 会**恒假**，
整条判据形同不存在，而且它「测试全绿」——因为冒烟里我只断言了不等。
修法：`format!("{y:04}-{m:02}-{d:02}")` + 两条测试锁补零（含 `20260105 → 2026-01-05`、异常值回空串不编造）。
**教训**：判据断言「不等」时，必须同时有一条断言「相等」成立，否则恒假也能全绿。

## 十七、P9-5 每日归档设计（时效项，每晚一天就多一天永久缺失）

**归档对象（三类，都是「只有当下值」的真源）**：涨停池（`qdate` 自证归属）、官方一致预期
（`RPT_WEB_RESPREDICT`，行内无日期列）、股吧人气榜（`emappdata/stockrank/getAllCurrentList`，`sc/rk/rc/hisRc`）。

**落点**：沿用既有正门 `astock-data/src/daily_snapshot.rs`（已承载质押/个股情绪快照；
`get_market_dragon_tiger` 在 `:51` 已注明「待补」，本次一并补）。

**表形态（按本仓纪律：列走实体 + `dao/src/reconcile/schema_diff` 声明式补齐，不写 migration）**：
每类一张日频表，主键 `(trade_date, stock_code)`，另存 `fetched_at` 与 `source_qdate`。
`source_qdate` 是**归属自证字段**：写入时若 `source_qdate != trade_date` ⇒ 不写这一行（而不是写一行今天的冒充那天），
否则归档本身就成了造假源。这条与 P9-4 的 `is_for_requested_date()` 同一条判据。

**读侧（回放）**：`*_with_asof` 优先查归档表按 `trade_date <= 截止日` 取最近一期；
无归档 ⇒ `record_degradation(StructuralGap)` 并写明「归档自 {最早 trade_date} 起，该日在之前」，
UI 必须显示**归档起始日**（否则用户会把「那几天没数据」读成「那几天没涨停 / 没机构预测」）。

**触发**：并入既有 `init/stock_automation_cron_seed.rs` 的四任务表（现有 02:00 口径是「尚未分析」状态判据），
新增一个收盘后任务；采集失败按 vendor 连续失败降级（`vendor_health.rs`）处理，不静默。

**验收要点（三条，缺一不可）**：
1. 写入路径在 `source_qdate != trade_date` 时**拒写**并有测试（这是防自造假的那一刀）。
2. 读侧在无归档时返回缺席 + 留痕，且降级文案带归档起始日 —— 断言「不会返回空数组当成答案」。
3. 连续跑两个交易日能拿到两行不同 `trade_date` 的记录（真实数据自证，不靠 mock）。

> 与 R-11 的关系：超短分支的 `microstructure`（连板/封单/换手）与长分支的 `expectationRevision`
> 的**可回放性完全取决于本项**。归档没跑满窗口之前，这两档的回放样本只能标「该维度归档前不可得」，
> 这是物理边界，不是实现进度。

## 十八、P9-4 更正：涨停池**能**按日直取（§十六 的结论作废）

**错因先记下来**：§十六 判「历史维度只能靠每日归档」，是因为我直接新接了东财 `push2ex/getTopicZTPool`，
而没有先普查仓内既有取数实现 —— `src-tauri/crates/astock-data/src/vendors/ths.rs:285` 的 `get_hot_stocks()`
早就在打同花顺 `dataapi/limit_up/limit_up_pool`（同一业务名目，只是没解全字段）。
用户据此立规（已入记忆：新增数据源前先按业务名目普查既有取数实现，判「扩字段 > 换申报 > 新建」）。
后果不是「多写一份代码」这么简单：**重复实现选中的是能力更差的那条**（push2ex 不采纳 date，ths 采纳）。

**ths `limit_up_pool` 实测（2026-10-03，探针 `.workbuddy/tmp/probe_ths_pool{2,3,4}.mjs`）**：

1. `date` 参数**生效**：`data.date` 逐日等于请求日（验过 20260930/29/28/25/24、20260105、20251231、20250930、20250918）。
2. 覆盖左界落在 **2025-09-16 ～ 09-18 之间**（≈ 回溯 12.5 个月；是否随时间滚动**未实证**）。越界回
   `status_code=-1 / status_msg="date参数不合法"`。
3. 非交易日（20260925 中秋、20251001–1008 国庆）回 `status_code=0` + `page.total=0` + 回显该日。
   ⇒ 「该日不可得 / 该日确无涨停 / 当下」三态天然可分，不需要额外自证字段。
4. `limit` 必须 ≤ 200（`limit=500` 直接被拒）；`page.{total,count}` 可分页（实测 107 行 = 6 页×20，末页 7 行，拼接顺序与单页一致）。
5. `high_days_value` = **(连板数 << 16) | 涨停天数** —— 5 个交易日 320 行零反例。
   反例校核：「3天2板」= 131075 = (2<<16)|3；按「天数在高位」读会把它错当成 4 连板，方向刚好反。
6. `first/last_limit_up_time` 是**字符串形式的 Unix 秒**（最早 09:25:00 CST = 集合竞价一字板），不是东财那种 `HHMMSS` 整数。
7. `reason_type` 的分隔符是 `+`（`"玉米种业+转基因玉米+业绩增长"`）；按逗号拆会得到一整串。
8. `open_num`（炸板次数）约 2/3 的行是 `null`，`limit_up_suc_rate` 偶发 `null` ⇒ 字段留 `Option`，不折成 0。
9. ⚠ 陷阱：`data.trade_status` 反映的是**当下**时钟状态（请求历史交易日照样回 `non_trading_day`），不得当作请求日的属性。
10. `limit_up_count.today` = `{num, history_num, rate, open_num}` ⇒ 涨停家数 / 触板次数 / 封板率 / 炸板家数，
    这是超短档情绪面的原生成份（`yesterday` 段刻意不取：回放要的就是「按日取」，把两日塞进一行等于重新制造歧义）。

**改了什么**：
- **撤**：`vendors/eastmoney.rs` 的 `get_limit_up_pool`（push2ex）、`format_pool_qdate`、`limit_up_pool_tests`
  整模与能力申报项；`types.rs` 的 `hhmmss_to_clock`（唯一使用者就是被撤的那条）。
- **建**：`vendors/ths.rs` 实现 `get_limit_up_pool` —— 与 `get_hot_stocks` **同一 URL**（后者语义与字段不动），
  带 `status_code` 闸（非 0 一律 Err，不退化成空池）、分页收齐校验（`total != 实收` ⇒ Err，不出半池）、
  回显日期不等 ⇒ Err（防「接口哪天不改了就 silently 拿今日池顶替历史」）。
- **DTO**：`LimitUpPoolEntry` 换成 ths 词汇表（连板数 + **涨停天数**两个轴、封单额/量、炸板次数、首末封板时间、
  板型、是否回封、流通市值、逻辑标签），除 code/name 外全 `Option`；`LimitUpPool` 加 `breadth`。
- **申报**：`ths.get_limit_up_pool = NativeDateParam`（本仓目前唯一按日可回溯的涨停通道）。

**对 §十七 的影响**：涨停池**退出归档对象**。归档清单收敛为两项 + 一项补漏：
官方一致预期、股吧人气榜，以及 `daily_snapshot.rs:51` 注明「待补」的 `get_market_dragon_tiger`。
（超短档 `microstructure` 的回放能力因此不再受归档起始日约束。）

**普查同时浮出的呈现层缺陷（下一单元处理，先记事实）**：
`src/components/stock-analysis/LimitUpPanel.tsx:81` 用 `boardCount = Math.max(1, Math.round(changePct / 10))`
**以涨幅反推连板数** —— 实测 09-30 的 600825「7天7板」（涨幅 9.9894%）算出来是 `boardCount = 1`，
而 `:217` 的渲染条件是 `boardCount > 1` ⇒ 七连板在面板上**连标签都不出现**；
妖股判据的核心数字是猜的；这正是裁定 3 禁止的「用近似数据顶替」。同一函数另有两处形态问题：
`:56-84` 对前 30 只票逐只 `invoke("get_stock_quote")`（池子里已带 `latest` / `turnover_rate`，N+1 取数），
`load()` 与 `useEffect()` 是同一段逻辑写了两遍。⇒ 下一单元：出命令 + 换呈现 + 删估算，一并收掉。

**消费侧接线（同日，P9-4 的后一半）**：库层通道建好≠面板吃到。四处已接：
1. `crates/astock-data/src/lib.rs` 路由表新增 `limit_up_pool: vec!["ths"]` + 客户端方法
   `get_limit_up_pool(requested_date)`。as-of 分支**不查快照缓存**（有原生 date 参数），
   并把「调用方没给日期」兜成**截止日**而非 `None` —— 传 `None` 会让今天的池当成回放那天的答案。
   取不到时回 `Ok(None)` + `StructuralGap/Failure` 留痕，**不回空池**。
2. `src/commands/stock_analysis.rs` 出 `get_limit_up_pool(date?)` 命令（build.rs 自动注册，实测主 crate 编译通过）。
3. `src/components/stock-analysis/LimitUpPanel.tsx` 重写：一次 `get_limit_up_pool` 取代
   「`get_hot_stocks` + 前 30 只逐只 `get_stock_quote` + 按涨幅反推连板数」三段式。
   删掉的近似是 `boardCount = Math.max(1, Math.round(changePct / 10))` —— 实测 09-30 的 600825
   是「7天7板」，该式算出 1，而渲染条件 `boardCount > 1` 又让它**连标签都不显示**；
   现在显示接口给的 `limitUpStreak`/`limitUpDays`（`3天2板` 与 `2连板` 分得开）+ 炸板次数 + 板型 + 封单。
   表头新增一行盘面情绪（`涨停 52 家 · 封板率 81.3% · 炸板 12`）—— 这是超短档的情绪面原生数据。
   `load()` 与 `useEffect()` 的同一段逻辑两份 ⇒ 合并成 `useCallback` 一处。
4. `vendorCheck.ts` 的 `limitup` gate 从 `["ths","baidu_stock","iwencai"]` 收窄为 `["ths"]`：
   那两个源不提供涨停池，ANY-of 语义下会让「只启了 baidu」的用户看到 gate=ok 而面板空。

**顺带收的一处重复**（禁区 12）：`DragonTigerPanel.tsx` 与 `IndustryRankingPanel.tsx` 各有一份**逐字相同**的
`function fmtYi`（已 diff 证实 IDENTICAL）⇒ 提到 `src/lib/format.ts::formatYi`，涨停池面板直接用第三份复用。
新日志/UI 文案 4 个 key（`poolSummary` / `boardDays` / `breakTimes` / `sealAmount`）已补齐 11 种语言，
三道 i18n 门（key 奇偶一致 / 占位符使用 / `check_i18n.py`）全绿。

**未接（诚实边界）**：`mcp_tools.rs` 的工具面与三份专家 md 的 `data_sources` 仍未挂涨停池
⇒ 分析师链现在**读不到**这些字段（面板读到了）。这与 P9-2 宏观节点属同一批 seed 改动，
验收同样要重启应用实跑 + `TEMPLATE_VERSION` bump，刻意不拆成两次。
另一项普查产物已开单：`get_hot_stocks` 的名目（「热门股榜」）与它真实供给（涨停池）冲突，
`trend-scanner.md:37` 早在 Phase 0 就因该原因把它摘掉，而 `social-media-analyst.md:6`、
`sector-analyst.md:6` 还挂着 —— 属名目收编，不在本轮顺手改语义。

## 十九、工具面注册（P9-2 + P9-4 共用，先于 P4′ 的图改动）

`crates/astock-data/src/mcp_tools.rs` 三处各加两项（schema 清单 / 「不需要 stock_code」白名单 / dispatch）：

| 工具名 | 参数 | 说明 |
|---|---|---|
| `get_limit_up_pool` | `date?` | 与 Tauri 命令同名同 DTO，dispatch 直接调 `client.get_limit_up_pool` |
| `macro_data_snapshot` | 无 | 与既有命令同名；复用 `MacroDataClient::snapshot()`，**不复制口径**（该函数内部已按 as-of 截止日收窄，实测 `on_or_before = current_asof.unwrap_or(today_cst)`） |

dispatch 里 `date` 做空串归一（`trim` 后 `""` ⇒ `None`）：LLM 常把可选 string 传成空串，
透传会让 vendor 打一次 `date=` 的必失败请求 —— 与 `mcp_tools.rs:901` 那条 `stock_code` 空值预检同一动机。

**刻意留到 P4′ 一起做（不是遗漏）**：把这两个工具挂进专家 `data_sources` 表
（`stock_analysis_setup/mod.rs:486` 一带）与工作流静态图的取数节点（`seed_stock_analysis.rs:2503` 的
`tool_assignments` 位置参数表），都需要 bump `TEMPLATE_VERSION` 并重种子。P4′ 本来就要改图（四路并行 +
逐档决策），**合并成一次 bump、一次重启核对**，比拆两次少一次全量验证，也避免「模板 v123 只加了工具、
v124 又改图」的版本噪音。在此之前：工具在 registry 里可达，但没有任何专家被授权调用它 —— 状态是
「通道 + 面板已通，分析师链未通」，与 P9 前半段的验收口径一致。

## 二十、#19 名目收编：「热门股榜」原来不是热度榜（含一处主动造假）

**普查出的四个供应方，没有一个给的是热度榜**（这是用户立规「新增数据源前先普查既有取数实现」的直接产物）：

| 位置 | 实际打的接口 / 做法 | 真实语义 |
|---|---|---|
| `vendors/ths.rs::get_hot_stocks` | `dataapi/limit_up/limit_up_pool?page=1&limit=20` | **涨停池前 20 行**（与 `get_limit_up_pool` 同接口同名目） |
| `vendors/iwencai.rs::get_hot_stocks` | 自然语言问「今日涨幅前20的股票」 | 涨幅榜 |
| `vendors/neodata.rs::get_hot_stocks` | 问「今日A股热门股票排名」后**从模型生成文本里按行刮** | 不是数据源 |
| `vendors/baidu_stock.rs::get_hot_stocks` | opendata `resource_id=5359&type=hot`（该域已 301 失效，且早已不在路由里） | 死通道 |

**主动造假一处**：`lib.rs` 的「修复 M-HOT-1」拿 `get_hot_stocks()` 的**数组下标**填
`SocialSentiment.hot_rank`，而该字段（`types.rs:393`）的文档口径是「热度排名（**平台内**）」。
guba 自己恒回 `None`。净效果：**「某票在涨停池里排第几」被当成「它在股吧排第几」序列化进分析师上下文**。
读侧普查：`hot_rank` 无任何 Rust/TS 读取点，唯一去处是 LLM prompt ⇒ 假数直接进判定。

**真源实测（2026-10-03，探针 `.workbuddy/tmp/probe_hot_list.mjs` / `probe_guba_rank_depth.mjs`）**：
1. 同花顺真热股榜 `dq.10jqka.com.cn/fuyao/hot_list_data/out/hot_list/v1/stock`：`data.stock_list` 恒 **100 行**，
   字段 `order`(名次,1 起) / `rate`(**字符串**热度值) / `rise_and_fall` / `tag.concept_tag`；
   `type=hour` 与 `type=day` 热度值差三个数量级（41772 vs 13212763）⇒ **只可同期比**；
   `date` 参数**不被采纳**（20260930 / 20260105 回显完全一致）⇒ 能力申报维持 `NoHistoricalSemantic`。
2. ⚠ `rise_and_fall` **已经是百分比原值**：601127 的 0.2777 对腾讯行情 0.28%、新股 001246 的
   206.5944 对 206.59%。再乘 100 会把新股首日涨幅写成 20659%。冒烟里用「max|涨幅| ≤ 500 且
   存在 |涨幅| > 1」两头锁住这个尺度（与宏观 `NATIONAL_SEQUENTIAL` 那次口径错同族）。
3. 东财股吧人气榜 `emappdata/stockrank/getAllCurrentList`（POST）：`data` 形如 `{"sc":"SH601127","rk":1,...}`，
   **只到前 100** —— `pageNo>=2` 或 `pageSize>100` 都回 `code=0` + 空数组（「参数被拒」伪装成「无数据」，
   与涨停池 `limit>200` 同族陷阱）⇒ `pageSize` 锁 100、首轮回空判接口异常、榜外 ⇒ `None`（不是第 101 名）。
4. 两源榜首互证：都是 601127 赛力斯。

**落地**：
- `ths.rs::get_hot_stocks` 换端点为 fuyao hot_list（涨停池那行注释保留，写明「为什么不再打那个 URL」）。
- `HotStock` 扩 `rank` / `hot_value` 两字段（**扩展而非重定义**，禁区 12）；`turnover_rate`/`sector`
  因热股榜不提供而恒 `None`（不折 0）。
- 删掉 iwencai / neodata / baidu 三处 `get_hot_stocks` override 及其能力申报与测试条目，
  `routing.hot_stocks` 收成 `["ths"]` —— 路由里留近似榜 = 允许假数据顶上真名目。
- 删 `lib.rs` 的假填充（原地留注释说明为什么不再跨源借位）；名次改由 `vendors/guba.rs::guba_hot_rank`
  **同平台**回填，且 `as_of` 回放**刻意不取**（人气榜只有当下语义，回填即时间泄露；缺席至少不说谎）。
  提取成自由函数 `pick_guba_rank` 以便零网络测「榜外 ⇒ None」。
- 呈现层 `HotStocksPanel`：删掉一列**从来不存在的字段**（`price` 列，后端 DTO 无此字段 ⇒ 恒 `-`）
  与热股榜不提供的换手列，换成名次 + 热度值；`load()` 与 `useEffect` 的 Promise 链两份重复合并成一处。
  gate 从 `screener`（六源 ANY-of）改成本面板自己的 `hotstocks: ["ths"]`。
- 文案与工具描述对齐（`mcp_tools` 里那句「同花顺强势股」改成热度榜口径并写明「涨停要查 `get_limit_up_pool`」）；
  `trend-scanner.md` 那句**理由失效但结论保留**的更正；`panels.rank` / `panels.hotValue` 补齐 11 语言。

**验收（已跑）**：`axagent-astock-data` 526 单测 + clippy `-D warnings` 0 + fmt 0；
热股榜 live 冒烟打印 `100 行 榜首=601127/赛力斯 涨幅=0.28% 热度=44453`；
ScreenerPage vitest 7/7；tsc / oxlint / dprint / 三道 i18n 门全绿。主 crate 门在跑。

## 二十二、P3′+P4′ 合并成一批（P3′ 单独落会造成「两套分」）

**为什么要合并**：分析师一旦改成逐档出结论，下游四个 `t-scoring-hour/week/month/quarter`
（`seed_stock_analysis.rs:3825/3836/3847/3858`）仍读**单个顶层 verdict** ⇒ 同一次分析里同时存在
「模型逐档给的」与「打分节点合成的一套」两种分。这正是缺陷 D2 的形态（四条入口算多套分），
也违反 §12-2 自述的「prompt 与门 a–e 一起落」。⇒ P3′（分析师逐档）与 P4′（子图四路 + 逐档决策脚本）
按一批实施，共用一次 `TEMPLATE_VERSION` bump 与一次重启核对。

**本批第一件已落（纯增量，不动任何消费者）**：逐档 VERDICT 契约的机器可读唯一源
`crates/harness/src/holding_period.rs` 的 `Period::verdict_spec()` + `HorizonVerdictSpec`。

放 harness 的理由与 `default_holding_days` 同一条：它同时被 ① 专家 prompt（人写）、
② Rhai 解析与数据质量判定、③ CI 门 三方消费，放任何一侧都会造出第二份权威（禁区 12）。

**三类「没有」被拆成三种语义**（原表把它们混成「缺席」一列，是这次落表时暴露出来的缺陷）：

| 类别 | 含义 | 键形态 |
|---|---|---|
| `fields` | 该档确实算了 | — |
| `not_applicable` | 按构造不适用 ⇒ **必须写显式文案**，不许省略 | `notApplicableXxx` |
| `qualified` | 只作过滤 / 只有部分来源 ⇒ 不进方向加权，且必须带标记 | 具体标记键 |

⚠ 由此发现 §十二 的「缺席时的显式文案键」一列**每行只给了一个键**，而同一因子在不同档的缺席
理由并不相同：`trendStrength` 超短是「不适用」、长档是「只作入场过滤」；`macroRegime` 短/中是
「不适用」、长档是「只有五条真序列」。⇒ 新增三个缺席键 `notApplicableTrend` /
`notApplicableMacro` / `notApplicableSupply`，与既有 `trendFilterOnly`、
`supplyAsRiskNoteOnly`、以及新记的 `microstructurePartialSource`（超短无撮合级）、
`qualityRealizationBeyondHorizon`（中档盈利兑现跨季）、`macroPartialSeriesOnly`（长档宏观仅五条）
分工明确。**这六个缺席键 + 三个条件键的 11 语言文案仍未落**（§十二 12-2 已列为独立小轮）。

**逐档黄金值（门里钉成字面量，改动必须是有意）**：参与因子 超短 2 / 短 4 / 中 6 / 长 5；
出场口径 `time_stop` / `time_stop+fixed_stop` / `k_sigma_band` / `target_and_falsified`；
`horizon` 明确**不进**模型必填清单（它由代码盖章，模型自报会把「它以为的档」当事实）。

**验收**：`cargo test -p axagent-harness holding_period` 5 passed（3 旧 + `verdict_spec_partitions_factors_and_differs_across_tiers`
+ `verdict_spec_golden_values`）、clippy `-D warnings` 0、fmt 0。
⚠ **未经突变验证**：「故意改坏表再看门是否报红」这一步被安全分类器拦下（它拒绝任何故意破坏源码的动作），
所以门的判别力目前**按构造成立**——黄金值是对字面量集合逐档断言、跨档差异断言要求
`shapes.windows(2).any(|w| w[0]!=w[1])`，把一套表抄四遍会直接红。不做绕过，如实标注。

**本批剩余（按依赖序）**：① `market-analyst.md` 等专家 prompt 的逐档契约段（含 §10-4 的显式作废声明）；
② `:5530` 的 `VERDICT_MACHINE_FIELDS` 从全局 6 键改成逐档白名单
（现在是 `VERDICT_SPEC_NOT_EXHAUSTIVE` 自承认「角色特有字段无门」）；③ 四路子图 + 四份 Rhai 决策脚本
（§十一 清单，含退役清单 `leg_mult`/`snr_confidence`/√(h/28) 同批删）；④ 缺席/条件文案键 11 语言；
⑤ `TEMPLATE_VERSION` bump + 重启实跑。

## 二十三、逐档契约改成「按表注入」，不落手抄（P3′ 第一步已落）

**为什么不写进各专家 md**：md 里手抄一份逐档字段清单，就会与 `verdict_spec()` 漂移 ——
而这张表同时是 prompt、Rhai 校验、CI 门三方的唯一源。所以契约**在执行期由代码渲染注入**，
md 里只留一个指针段。改表即改 prompt，结构上不存在「md 写七项、门按八项判」的中间态。

**落点**：

1. `crates/harness/src/holding_period.rs`
   - `Period::verdict_contract_prompt()` 渲染本档契约（必填 / 条件 / 不适用三类 + `exitRule` 口径）；
   - `qualified_reason()` 给每个条件键一句可指到实测缺口的理由（超短「连板有真字段、逐笔零通路」、
     长档「技术腿只作入场过滤」「宏观仅五条真序列」…）；
   - `Period::verdict_factors()` 暴露因子全集 —— 门读它，不自带第二份名单；
   - `Period::label_zh()` 档位中文标签（注入文案与面板共用）。
2. `crates/rt-workflow/src/work_engine/executors/agent_executor.rs` 新增 **4j 段**
   （既有 4h=领域尾部约束、4i=TaskScene，未挤占编号）：取到档位即把契约 push 成 prompt 尾段。
   档位取值**两条路都认**，P4′ 再定死用哪条：
   ① `context.variables["horizon"]`（分支子工作流直接写变量）；
   ② 节点 `input_mapping` 里 `target_key == "horizon"` 的映射（按源路径解析）。
   取到但不是四档之一 ⇒ `warn`（静默不注入会让「契约没生效」在现场长得像「模型不守约」，
   与 4g 段那条「悬空 input_mapping」同一判据）。
3. `agency_experts/stock-analysis/market-analyst.md` 只加**指针段**「逐档契约」，明确
   「本文件刻意不复制清单」+ 没有注入时按通用格式输出。

**现网影响：零。** 现状分析师无档运行 ⇒ 两条取值路都拿不到 `horizon` ⇒ 不注入，
prompt 逐字不变。这一步因此**可以独立于 P4′ 落地**而不造成「分析师出四档、打分只读一档」的两套分
—— §二十二 说的合并批次，被这个触发条件拆成了可分段落地的两步。

**新门（第 46 道）**：`seed_consistency_tests.rs::expert_prompts_do_not_hand_copy_per_horizon_factor_lists`
—— 任何专家 md 出现逐档因子名即红。三条自证齐备：① 扫描面 `>= 20` 个 md（目录改名会当场红，
不会静默空跑）；② **负控**：把一份合成手抄文本喂给判据函数，必须命中 3 项
（不靠改生产码来证明门会红 —— 上次那个「故意改坏表」的动作被安全分类器合理拦下，
正确姿势是让判据函数可被坏样本直接调用）；③ 反向负控：通用 6 键（`confidence`/`bull_score`…）
**不该**命中，证明本门只拦逐档因子、不拦跨档核心。

**契约原文怎么读**：`cargo test -p axagent-harness verdict_contract_prompt_dump -- --ignored --nocapture`
会把四档契约原文打出来（人工审阅用，`#[ignore]`）。

**本步验收**：harness `holding_period` 6 passed + 1 ignored dump；`axagent --lib` 种子门
**46 passed**（45→46）；rt-workflow `cargo check --all-targets` 通过；三 crate fmt/clippy 在跑。

**这批剩余**：② `agent_executor.rs:4502 VERDICT_MACHINE_FIELDS` 逐档化 —— 现在截断修复的验收
只看通用 6 键，而注入契约后**本档必填因子缺失也该判修复失败**（否则半截 JSON 被闭合成
「只有 6 键」的空壳，正是 4502 那段注释记载的 688498 事故形态的逐档版）；
③ P4′ 图（4 路 SubWorkflow + 4 份决策 Rhai + 退役清单）；④ 六个缺席键 + 三个条件键的 11 语言文案；
⑤ `TEMPLATE_VERSION` 122→123 + 重启实跑。


## 二十四、三套 id 空间的收编（用户裁定 A，2026-10-03 落地）

### 24-1 缺陷实况（三族，逐族都给可复现证据）

| 族 | 形态 | 为什么长期无人察觉 |
|---|---|---|
| ① **幽灵 id** | `evidence_weight.rs` 的四档权重表里并存着 `a-market`/`a-technical`/`capital`/`macro`/`fundamental`/`sentiment` **六个图里不存在的名字**（真实节点见 `seed_stock_analysis.rs` 的 `analysts` 数组：`a-market-analyst`「技术面分析：K线形态、MACD/RSI、支撑阻力位」等） | 后端按**裸节点 id 精确查表**，落空即退 `default=1.0`；前端手抄表抄的是**同一批幽灵名** ⇒ 原判据①②「两侧逐字段一致」在自选名之间互相印证，永远绿 |
| ② **前缀未剥** | 黑板键是 `report.{id}`（`crates/analysis-engine/src/blackboard.rs`），权重表按裸 id 索引 ⇒ 即便 id 写对，查表也**全员落空** | 与①同族：两侧一致地错，没有跨侧参照物 |
| ③ **归因按「看起来像」** | 桥表 `DECISION_LEG_ANALYST` 把 `f3`（变量 `catalyst_level`，路径 `a-catalyst.content.verdict.catalyst_level`）桥到 `a-news`；把 `f10`（变量 `lockup_bundle`，来自 `t-lockup-data` 的解禁+增减持+大宗+股东户数）桥到 `a-hot-money`（那是 `f9` 的 `money_flow` 域） | id 对不上 ⇒ 乘数恒 1.0 ⇒ **写错归因没有行为后果**，也就没有任何现象会暴露它 |

⚠ 第③族的因果次序要说清：**它是在①②修好之后才变成真问题的**。id 统一后归因会真的改变乘数，
所以订正归因不是洁癖，而是统一 id 的必要前置 —— 否则「修好查表」等于「把错归因激活」。

### 24-2 已落地（全部静态可验，不动图、不动模板版本）

1. **权威 id 清单**：`evidence_weight.rs::EVIDENCE_ANALYST_IDS`（12 个：10 个分析师 + `value-investor` +
   `research-mgr`），`ANALYST_IDS` 保留为等价历史名。清单里**每一个 id 都逐个在 seed 里搜到过**
   —— 我写第一版时自己塞了一个 `a-technical-scan`，被这道逐名核对当场否掉；这条方法登记在此，
   因为「新增权威清单的人造出一个幽灵名」正是本文件记载的缺陷的复发形态。
2. **查表入口归一**：`analyst_key()` 剥 `report.` 前缀，且**查不到时 warn**（不静默退 1.0）。
   归一放在入口一处，不让各消费端各自 `replace`。
3. **`classify_role` 改精确 12 名**，后缀推断只作兜底且兜底时 warn。
4. **四档权重表重排**为 12 个真节点 id；四个从未被赋权的分析师（`a-catalyst`/`a-lockup`/`a-policy`/
   `a-research`）显式写 `1.0` —— 「该档没声明过偏置」必须是**写出来的 1.0**，不是查表落空的 1.0。
   本轮**没有为它们发明新数值**（数值口径要 P4′ 的逐档因子集来定，见 24-4）。
5. **前端离线兜底表**（`src/lib/stock-analysis-utils.ts` 的 `ANALYST_TIME_HORIZON_WEIGHT`）同步为
   同 12 名同数值；`getAnalystWeight` 也剥 `^report\.`。
6. **桥表归因订正**：`f3 → a-catalyst`、`f10 → a-lockup`；`f2 → research-mgr`、`f13 → a-sector`
   保留，但按**证据域唯一属主**口径写明理由（腿的信号多由算法/工具节点产出，LLM 分析师并不
   直接产出该变量 ⇒ 只能按域属主桥，无属主即 `None`；`hooks.rs` 关于 f13↔a-sector 的既有论证不变）。
7. **`UNBRIDGED_ANALYST_IDS` 显式登记**（`a-fundamentals`/`a-research`/`a-policy`/`a-news`）+
   闭合测试 `bridged_plus_unbridged_covers_every_analyst`（有腿 ∪ 无腿 = 权威清单，且不重叠）。
   这一条是本轮唯一「把缺陷变成结论」的动作：**没腿**今后是写在源码里的声明，而不是查表落空的结果。

### 24-3 门（`scripts/check-horizon-weight-parity.mjs`，判据 ⑦⑧）

- **⑦ 三处键集合 = 权威清单**：后端每档、前端每档都严格等于 `EVIDENCE_ANALYST_IDS`，
  且清单里每个 id 必须在 seed 中以 `"id"` 形态出现（防清单自己腐烂成新的幽灵名）。
- **⑧ 闭合**：有腿 ∪ 无腿 = 清单、两边不重叠、桥表不得引用清单外的人。
- 自检从 15 条扩到 **28 条**，其中 ⑦⑧ 各带负控 + 若干正控；正控全部用**内存里的合成坏样本**
  （把幽灵名塞回前端表 / 后端整档少一个 id / seed 里把 `a-lockup` 改名 / 清单掺一个 `a-phantom`
  ⇒ 精确报 9 处），不改生产码、不改磁盘文件 —— 上一轮「故意改坏表证门会红」被安全分类器合理拦下，
  正确姿势是让判据函数可被坏样本直接调用。
- 现网读数：`比对面：4 档 × 后端合计 48 键 | 桥表 12 腿 | 权威分析师清单 12 个 | 无腿登记 4 个`。

### 24-4 行为变化（必须重启实跑核对，静态门覆盖不到）

id 统一 ⇒ 逐档乘数**从恒 1.0 变成真的作用**，以下四档差异是本轮起才存在的：

| 档 | 起效的关键乘数 | 预期现象 |
|---|---|---|
| 超短 | `a-market-analyst` 1.3、`a-hot-money` 2.0、`a-fundamentals` **0.3**、`value-investor` **0.3** | 估值/基本面腿被压低；此前这两腿恒按 1.0 参与 ⇒ 超短与短线结论会拉开 |
| 短 | 技术 1.5 / 资金 1.5 / 情绪 1.3 / 基本面 0.6 | 同上，弱一档 |
| 中 | 基本面 1.2、技术 1.0 | 与主链口径最接近（回归基线） |
| 长 | `value-investor` 2.0、`a-fundamentals` 1.5、技术 0.6、资金 0.5 | 估值腿主导 |

**核对点**：同一只票重跑四档，`weightsSource` 不得再出现 `fallback_unity`；`f10` 筹码面的逐档乘数
现在跟着 `a-lockup` 走（恒 1.0，因为 `a-lockup` 四档都登记 1.0 —— 这条腿本轮**行为不变**，
是刻意留白等 P4′ 定档口径，不是漏改）。

### 24-5 本轮刻意不做

- **不给四个无腿分析师发明档位数值**：数值要由 P4′ 的九因子集（`Period::verdict_spec` 的
  `VERDICT_FACTORS`）决定，它们的承接关系已在 `UNBRIDGED_ANALYST_IDS` 的文档注释里逐名写明
  （`a-fundamentals → earningsQuality`、`a-research → expectationRevision`、`a-policy → macroRegime`、
  `a-news` **刻意无腿**：公告方向通道已由 `a-catalyst` 的 `f3` 承载，同域双桥就是重复计数）。
- **不改图、不 bump 模板版本**（仍与 P4′ 同批 122→123）。
- 顺带记两处**别的**手抄清单缺陷：`src/lib/decisionInputDiagnosis.ts` 的 `NODE_SPECS` 把 `f7`
  权重抄成 0.15（权威 `f7_default = 0.10`，已订正），且整表缺 `f12`/`f13` 两条腿 —— 已在文件头
  写明「16 个是节点数、不是决策腿全覆盖」的覆盖面边界，收编成独立事项 **#21**（与 P4′ 的
  「腿表由后端注入渲染」是同一次改动，不在这里加第二份手抄）。
- `decision.rs` 的 `expert_mapping` 只覆盖 12 个分析师里的 7 个（面板不完整），本轮**仍不修**，
  避免与 #21 的清单收编混成两批。

### 24-6 本步验收状态

- `cargo test -p axagent-analysis-engine evidence` ⇒ **29 passed**（含新闭合测试）
- `node scripts/check-horizon-weight-parity.mjs` ⇒ EXIT=0；`--selftest` ⇒ 28/28 PASS
- `node --check` / `npx dprint check`（三处前端脚本）/ `npm run typecheck` ⇒ 全绿
- 定点 `cargo clippy -p axagent-analysis-engine -p axagent-harness -p axagent-rt-workflow
  --all-targets --all-features -- -D warnings` ⇒ **EXIT=0**（2m20s 编完，零 warning）；
  `rustfmt --check crates/analysis-engine/src/evidence_weight.rs` ⇒ 干净（注释对齐由 rustfmt 定形）
- `cargo test -p axagent-analysis-engine` ⇒ **593 passed / 0 failed**；`-p axagent-harness` ⇒ 42 + 6 passed
  （另 2 + 6 ignored，都是需联网/人工审阅的）


## 二十五、P4′ 第一步：分析师挂载集合改「推导」，不手抄（2026-10-03）

### 25-1 为什么第一步不是改图

P4′ 的清单（§十一）第一行是「4 个子模板」，但**子模板挂哪些分析师**这件事在此前只存在于
本节 §10-4 的手抄表里，而**每档参与哪些因子**已在 `Period::verdict_spec()` 定档（§十二）。
两份清单同时存在必然漂移，且漂移的表现不是文字错乱而是**死输入**：
实测 §10-4 的**中档与长档都没挂 `value-investor`**，而两档的必填因子都含 `valuationBand`
⇒ 照那张表建子图，估值腿在子图里永远拿不到产出方，面板还会显示「估值因子已参与」。
⇒ 先把两份清单合成一份（属主表 + 推导），再动图。

### 25-2 唯一表：因子 → 证据属主（`crates/harness/src/holding_period.rs`）

`VERDICT_FACTOR_OWNERS`（11 个逐档因子）+ `COMMON_FACTOR_OWNERS`（跨档共用的 `eventCatalyst` → `a-catalyst`），
`Period::analyst_subset()` = 该档（必填 ∪ 条件）因子属主 ∪ 共用因子属主，去重排序。

本轮**新增两个因子**，都指到实测数据出处，不是理念补位：

| 因子 | 属主 | 数据出处 | 参与档 |
|---|---|---|---|
| `breadthState`（涨停家数 / 触板数 / 封板率 / 炸板率） | `a-sentiment` | P9-4 同花顺涨停池的 `LimitUpBreadth`（**可按日回溯**） | 超短、短（中长不适用：情绪广度是短窗量） |
| `sectorRotation`（行业景气与轮动） | `a-sector` | `get_industry_ranking` —— **live 有真源，回放结构性不可得**（BK 历史源普查已穷尽、push2* 族按累积量/速率触发 RST） | 长档，且**只作条件字段**（标记 `sectorRotationAsOfGap`，不进方向加权） |

`microstructure`（封单/连板结构，板级）属主定为 `a-hot-money`，与 `breadthState`（家数/封板率）
分属两人的划界理由：**封单额与炸板次数是资金行为，涨停家数与连板高度是情绪状态** ——
同一份涨停池响应里的两组字段，不该由一个分析师同时代言（否则逐档权重把两者当一个域）。

### 25-3 推导结果与 §10-4 的四处差异（谁为准：推导）

| 档 | 推导出的挂载集合 | 与 §10-4 的差异 |
|---|---|---|
| 超短 | `a-catalyst` `a-hot-money` `a-market-analyst` `a-sentiment` | **逐字相同**（这张表唯一猜对的一档） |
| 短 | `a-catalyst` `a-hot-money` `a-lockup` `a-market-analyst` `a-sentiment` | 少 `a-news`（与 `a-catalyst` 同读公告域 ⇒ 双挂就是重复计数，与 §二十四 的 `UNBRIDGED_ANALYST_IDS` 同一裁定）；多 `a-sentiment`（`breadthState`） |
| 中 | `a-catalyst` `a-fundamentals` `a-hot-money` `a-lockup` `a-market-analyst` `a-research` `value-investor` | 多 `value-investor`（补上死输入）、`a-hot-money`、`a-lockup`；**少 `a-sentiment`、`a-policy`** —— 中档既无 `breadthState`（短窗量）也无 `macroRegime`（§十二 判中档不适用），挂着只会白烧一次 LLM 调用并伪装成证据在场 |
| 长 | `a-catalyst` `a-fundamentals` `a-market-analyst` `a-policy` `a-research` `a-sector` `value-investor` | 多 `value-investor`、`a-market-analyst`（`trendStrength` 在长档是条件字段 = 入场时机过滤，R-11 明文不得进方向加权） |

**两个「有节点、无因子」的分析师在任何档都不挂**：`a-news`（同域去重）、`research-mgr`（裁决/合成层）。
这条由测试 `analyst_subset_is_derived_from_factor_owners` 双向锁死：既不许挂无因子的分析师，
也不许有因子的分析师不在挂载集合里（`owns_something` 那半条）。

### 25-4 验收读数

- harness：`cargo test -p axagent-harness holding_period` ⇒ **7 passed / 0 failed**（含新测试），
  另 1 ignored（`verdict_contract_prompt_dump` 人工审阅）；lib 全量 42 passed。
- 黄金值改动是**有意的**：逐档参与因子从 2/4/6/5 变成 **3/5/6/6**（新增两因子所致），
  `verdict_spec_golden_values` 与 `not_applicable` 计数同步订正，未放宽任何断言。
- 跨 crate 闭合：`evidence_weight.rs::factor_owners_are_all_registered_analysts`（新）⇒ 属主表推出的
  每个挂载分析师都必须在 `EVIDENCE_ANALYST_IDS` 里 —— 两张表分处两个 crate，编译器不给提示。
- 反手抄门无需改：`hand_copied_horizon_factors` 读 `verdict_factors()` 动态取清单，
  新增因子自动进扫描面（这正是当初把门写成读表而不是写死名单的原因）。

### 25-5 下一步（仍在 P4′ 批内，顺序不可换）

1. **P4′-b**：4 份逐档决策 Rhai（`portfolio-mgr-h-*.rhai`）—— 只吃本档挂载分析师的 verdict 因子，
   置信推导与 `exit_rule` 同族（超短 `time_stop` ⇒ 事件命中口径；中 `k_sigma_band`；长 `target_and_falsified` ⇒ 安全边际带宽），
   **不得**出现 `leg_mult` / `pm_snr_confidence` / `horizon_decision`（门 a）。
2. **P4′-c**：4 个子模板（各自版本从 1 起）+ 父图 4 个 SubWorkflow 节点 + arbiter CodeNode；
   子模板**必须注入 `horizon`**（门 f，否则 4j 段契约静默不生效）。
3. **退役**（与 1/2 同批，不留 shim）：`leg_mult`/`horizon_leg_multipliers` 的乘数语义、
   √(h/28) 档间唯一差异、跨档同值 `risk_bias`、`totalScore_*` 四入边映射、`sharesPosteriorWith` 语义反转。
4. `TEMPLATE_VERSION` 122→123 一次 bump + 用户重启实跑（本批唯一必须由人做的验收）。
5. 遗留小轮：缺席/条件文案键从 9 个变 **11 个**（新增 `notApplicableBreadth`/`notApplicableSector`，
   条件键 +1 `sectorRotationAsOfGap`）⇒ 11 语言文案仍按「有消费者才落」等 P4′-c 的面板接线。


## 二十六、P4′-b 落地：四份逐档决策脚本 + 腿信号单一实现（2026-10-03）

### 26-1 这一批新增的东西

| 资产 | 位置 | 作用 |
|---|---|---|
| 分支表 | `analysis-engine/src/evidence_weight.rs::horizon_branch_specs()` | 逐档输出 `legs / analysts / entryGate / confidenceMethod / exitRule / days / positionMultiplier / volLookbackDays / degraded` —— **数值全部派生**自 §二十四 的权重表与 §二十五 的因子属主表，本函数不新增一个数 |
| 腿信号 | `analysis-engine/src/leg_signal.rs`（12 个因子一处实现） | 因子原始量 → `[-1,1]`；`None` = 缺席（**不是 0**）。`momentumSignal`/`eventCatalyst` 是从 `portfolio-mgr.rhai` 的 f12/f3 阶梯**原样搬运**，黄金值测试逐情形锁住 |
| 带位 | `harness/src/indicators.rs::band_z` + 宿主函数 `pm_band_z` | `(末价 − SMA) / σ_价`；停盘/一字板 σ=0 ⇒ `None`（不得读成「正居中」） |
| 四份分支脚本 | `src/commands/portfolio-mgr-h-{ultra-short,short,mid,long}.rhai` | 各档自己的融合、门、动作阶梯与出场口径 |
| 新门（第 47 道） | `seed_consistency_tests.rs::horizon_branch_rhai_scripts_compile_and_are_genuinely_forked` | 禁词（§11-3 退役形态）+ 正向断言（必须读 `branch_json`、必须调 `pm_leg_signal`、必须自报档位）+ 两两不同 + **生产同配置编译**（`RhaiSandboxLimits::PORTFOLIO`） |

### 26-2 四档到底哪里不同（不是四套参数）

| 档 | 方向腿 | 非方向腿 | 门 | 出场 |
|---|---|---|---|---|
| 超短 | 动量 / 情绪广度（**本表原列「催化剂」是错的**，权威表 `verdict_spec::UltraShort.participating` 不含 `eventCatalyst` ⇒ 以权威表为准，见 §三十四 与 §六十二(2)；该缺席自 v130 起由本档 `data_gaps` 点名，不再只活在脚本注释里） | 封单结构 `partial`（只到板级、无撮合级） | `breadth_required`：封板率 < 0.5 ⇒ 封顶持有 | `time_stop`（无目标价止盈） |
| 短 | 动量 / 广度 / 资金持续性 / 趋势 / 供给 | — | `trend_and_flow_same_sign` **且**在场方向腿 ≥ 2 条 | `time_stop+fixed_stop` |
| 中 | 资金持续性 / 趋势 / 估值带 / 预期修正 | 盈利质量 `partial`、解禁 `riskNote`（不进方向） | `inside_sigma_band`：`\|z\| ≤ 2` 才建仓，带外向上封顶持有 | `k_sigma_band` |
| 长 | 估值带 / 盈利质量 / 预期修正 / 宏观 | 技术腿 `filter`（只作入场时机）、行业轮动 `partial`（背景） | `positive_margin_of_safety`：DCF 上行 ≥ 30%（Graham 外部标准）**或** PE 在自身 5 年下四分位 | `target_and_falsified`（止盈=估值目标；止损=三条机械证伪判据） |

置信口径四档一致地**不做跨期开方**：持有期已经进了价带（`pm_vol_move_pct(…, days)` 内部含 √days），
再乘 `√(h/28)` 就是给同一个时间假设计两次利息 —— 这是 §11-3 退役 `pm_snr_confidence` 的理由，
不是「换个写法」。

### 26-3 两处我自己收回的东西（写下来防复发）

1. **长档的「PE 中位代理目标价」**：初稿写 `(50 − pePercentile)/5` 充当止盈目标。分母 5 是拍的，
   且把近似数据当结论 —— 直接违反用户裁定③（既不按缺席收口、也不用近似顶替）。
   现改为：DCF 不适用 ⇒ `takeProfitSource = "no_target"` ⇒ 赔率无定义 ⇒ **不出仓位**，
   并让面板点名。银行/保险那一批在长档因此拿不到仓位建议，这是**数据现实的出口**，不是缺陷。
2. **禁词门自打一枪**：门第一版拿整份文本比 `leg_mult`，结果被我自己的说明性注释
   （「⚠ 禁止出现 `leg_mult`…」）判红。修法不是删注释（注释正是读者需要的），而是
   **剥注释后再查代码**，并补一条自证「剥完仍 > 40 行」—— 否则剥离函数把代码也吃掉时，
   本门会静默变成恒真。（与本仓「扫到 0 条的门等于没有门」同族，这次是「剥到 0 行的剥」。）

### 26-4 现网影响：零

`horizon_branch_specs()` **已注入**为工作流变量 `horizon_branch_json`（`stock_workflow/hooks.rs`，
与 `horizon_leg_weights_json` 同段、恒注入不 throw），但**没有任何脚本读它**：
四份分支脚本还**没进图**（子模板与父图是 P4′-c），主链继续走它自己的乘数表
⇒ 本批全部改动对当前运行不产生任何行为变化（变量在场但无人消费，是「接线前一步」而不是缺陷）。
`portfolio-mgr.rhai` 的 f3/f12 内联阶梯也**暂未改调 `pm_leg_signal`**：那是 §11-3 退役动作的一部分，
与图改动同批做（改主脚本 = 必须重种 = 必须重启核对，不该拆成两次）。

### 26-5 验收读数（本批实跑）

- `cargo test -p axagent-analysis-engine --lib` ⇒ **600 passed / 0 failed**（含 leg_signal 4 条、
  分支表 1 条、闭合与属主 3 条）；`-p axagent-harness --lib` ⇒ **778 passed / 0 failed**（含 `band_z`）。
- 定点 `cargo clippy -p axagent-analysis-engine -p axagent-harness -p axagent-rt-workflow
  --all-targets --all-features -- -D warnings` ⇒ **EXIT=0**（首轮报 3 处 `cmp_owned` +
  1 处 `if_same_then_else`，均已按「不为过 lint 改语义」修：`Value` 比较改字面量、
  动量阶梯两个同体分支合成 `||`，NaN 逐情形等价已核）。
- `cargo fmt --all --check` ⇒ **EXIT=0**。
- 新门 `horizon_branch_rhai_scripts_compile_and_are_genuinely_forked` ⇒ **passed**
  （四份脚本在生产同配置下编译通过；首轮的 FAILED 是 §26-3 那条自打，属门正常起效）。
- ⚠ 一次 `cargo test -p axagent-analysis-engine` 报 `599 passed; 1 failed`，复跑 600/600 ——
  **未定位到具体是哪条**（日志被 tail 截断），按本仓已知 flaky 形态（live-network / CPU 挤爆）待定；
  不写成「已排除」。

### 26-6 P4′ 剩余（下一批，仍是同一批 bump）

1. **P4′-c 接线**（注入已完成，剩子模板与图）：4 个子模板
   （`stock-analysis-h-*`，各自版本从 1 起）挂本档分析师子集（= `analyst_subset()` 的推导结果）
   + 本档尺度的评分节点 + 对应的 `sharedTool` 通道（涨停池 / 估值带 / 一致预期 / 宏观快照 /
   解禁包 / 行业排名 —— 其中**涨停池与宏观快照目前只有工具注册、无取数节点**，本批要补节点）；
   父图 4 个 `SubWorkflow` + arbiter CodeNode（P5 仲裁规则）。
2. **§11-3 退役**：主脚本 f3/f12 改调 `pm_leg_signal`、`leg_mult`/`horizon_leg_multipliers`
   的乘数语义与 `horizon_decision` 单闭包删除、`totalScore_*` 四入边映射改指各档子图、
   `sharesPosteriorWith` 语义反转成红判据。
3. **门 b–f**：串味判据（子模板不得引用他档输出键）、字段集 == `verdict_spec()`、
   `sharesPosteriorWith` 恒空、四档 action 常量互相独立、**每个子模板必须注入 `horizon`**。
4. `TEMPLATE_VERSION` 122→123（一次 bump）+ 用户重启实跑；缺席/条件文案键 11 个 × 11 语言在
   面板接线后落。


## 二十七、P4′-c 第 0 片：宏观与涨停池进分析师前置取数（2026-10-03，bump 123）

**这一片是 P9-2 的正解**，也是分支表里 `macroRegime` / `breadthState` / `microstructure`
三条腿的**数据侧补齐**：工具早就注册（§十九），但没有任何节点替分析师去取，也没有被授权 ⇒
腿在表里声明参与、实际永远拿不到数。

照 `t-pledge-data`（v72）的先例，一个「分析师额外前置供给」必须**四处成对**，少任一处都是假修复：

| 触点 | 位置 | 漏掉的后果（都是本仓实测过的） |
|---|---|---|
| ToolDef | `seed_stock_analysis.rs` 的 `td_*` + `tool_def_map` | `config.tools` 由 `filter_map` 生成 ⇒ **静默丢工具**，界面显示「已授权」而 LLM 手里没有 |
| 节点 | `tool_node_noarg(MACRO_TOOL_ID / LIMITUP_TOOL_ID, …)` | 没人取数 ⇒ 分析师只能在 prompt 里写「该维度数据缺失」 |
| 边 | `trigger → 节点 → 消费者` 三段 | 只有 `context_sources` 没有边 ⇒ 节点不等它完成，取到空变量（时序竞态，比恒缺更隐蔽） |
| 消费声明 | `context_sources`（本轮改成「额外前置供给」表） | 数据到了黑板但不在该分析师的清单里 ⇒ prompt 里没有它（600887 `f474ec9b` 实证） |
| 授权 | `PROFILE_TOOLS` 三处 + 三份专家 md 的 `data_sources` | LLM 侧无法按同口径重取；md 侧 `md_prompt_tool_refs_are_resolvable` 会红 |

两处刻意的设计决定，写下来防被「顺手改回去」：

1. **两个 ToolDef 都 `no_params()`**，不给模型 `date` 入参。回放里「哪一天算已知的最新一天」
   只有取数层知道（宏观按保守可得日裁、涨停池按截止日回落）；把日期做成模型可填的参数
   = 给前视开门。与 `horizon` 同纪律：**口径由代码盖章，不进模型可自报的字段集**。
2. **涨停池同时给 a-sentiment 与 a-hot-money，不算重复计数**：`breadthState` 取的是
   `LimitUpBreadth`（家数/触板/封板率/炸板数），`microstructure` 取的是行内
   封单额/量 + 连板结构 + 炸板次数 —— 同一份响应的两组字段，两个因子的属主本就不同
   （§二十五 的属主表）。

**版本**：`TEMPLATE_VERSION` 122→**123**，变更说明按本仓惯例逐条写进常量上方的 doc。

**验收（实跑）**：`cargo fmt --manifest-path src-tauri/Cargo.toml --all` 通过；
`__TAURI_WORKSPACE__=true cargo test -p axagent --lib seed_consistency` ⇒ **31 passed / 0 failed**
（含 `tool_node_positional_tool_names` 系两道扫描门、`profile_tools_all_resolvable`、
`seed_stock_analysis_tools_all_resolvable`、`md_prompt_tool_refs_are_resolvable`）；
另跑了一处**结构自证**（17 条断言逐触点核到磁盘文本，不信「编译过 = 改动在」）。

**仍待人工核对**：重启应用重播种后，看一次真实分析里 a-policy 的 prompt 是否出现宏观段、
a-sentiment / a-hot-money 是否出现涨停池段（这一步只有 `tauri dev` 能证）。


## 二十八、分支脚本的**运行门**：编译绿≠跑得通（2026-10-03，抓出三个真缺陷）

新增门 `seed_consistency_tests::horizon_branch_rhai_scripts_execute_end_to_end`：
用**生产同一个引擎工厂**（`rhai_registry::build_stock_rhai_engine(PORTFOLIO)`）、
**生产同一个转换器**（`rt-workflow::expression_engine::rhai_eval::value_to_dynamic`，本轮提为 `pub`）、
**真实分支表**（`horizon_branch_specs()`）把四份脚本各跑两态：

- ① 输入齐备 ⇒ 动作在六档词表内、`legs` 非空、置信在 0-100、输出 `horizon` 与脚本档位一致；
- ② 只给分支表、其余全缺 ⇒ `absentLegs` 必须点名、`positionPct ≤ 0`、`dataGaps` 必须留痕。

判据形态照 `code_executor` 的 V57：**先把脚本里 `present(x)` 出现的名字全铺 `unit`**，
否则测的是「变量不存在」而不是「变量未注入」。

### 28-1 第一跑就连抓三个（都是编译门查不出的）

| # | 现象 | 为什么编译门看不见 |
|---|---|---|
| 1 | `Function not found: present (f64)` —— 四份脚本都没定义 `present`，它一直是 `portfolio-mgr.rhai:22` 的**脚本自带 `fn`** | Rhai 编译期不解析函数名；宿主引擎也没注册 `present` |
| 2 | `branch_json["volLookbackDays"]` 取到 unit —— 我在脚本里读这个键，但**分支表从来没写过**（设计时说好要放，落码时漏了） | 这是「声明了要读的注入键不存在」，Rhai 索引缺失键不报错 |
| 3 | 修 ② 时暴露的写法风险：`pm_vol_move_pct(Array, i64, i64)` 收到 f64（JSON 数字一律落 f64） | 函数解析按**实参类型**匹配，签名不匹配是运行时 `Function not found` |

对应处置：① 四份脚本各自补 `fn present(x)`（与主链同规；**不**改成宿主注册 —— 本仓有
`pm_*` 静默未注册的前例，脚本自带 1 行守卫比依赖注册面稳）；② 分支表补
`VOL_LOOKBACK_DAYS: i64 = 20` 常量并注入 `volLookbackDays`；③ 脚本侧加 `band_days`/`band_lookback`
双守卫：只有 `type_of(...) == "f64"` 才 `.to_int()` 并调用宿主函数，否则价带按「不可得」走
（绝不对 unit 调 `to_int()` —— 崩得比缺席难看）。

### 28-2 顺带新增的反漂移门

`main_chain_vol_lookback_matches_injected_const`：主链 `portfolio-mgr.rhai` 仍保留自己的
`let VOL_LOOKBACK_DAYS = 20;`（退役与图改动同批做），而分支表已注入同名口径 ⇒
两处数字若各自漂移，得到的是「短档用 20 日带、超短用 30 日带」这种**两侧各自自洽、跨档互相打架**
的形态，谁都不报错。本门逐字比对两侧。

### 28-3 一次自造事故（记法，不是记成绩）

写 §28 的两条门时，我把 `open(p,"a")` 与 `write(全文 + 新段)` 混用，**整份 2947 行的
`seed_consistency_tests.rs` 被重复了一遍**（变成 5914 行），编译症状是文件中部出现
`//!` 内层文档注释报 `E0753`。第一反应差点归因成「并行会话往中间插了内容」。
定位办法：查「文件首行标记的第二次出现」并截断，只存在于重复段的 drift 门补回，
再用「标记出现次数==1」+「新门名计数==1」两条自证收尾。
⇒ 规则：**追加只写增量**；要重写全文才用 `"w"` 且一次性写完。

### 28-4 验收

`__TAURI_WORKSPACE__=true cargo test -p axagent --lib seed_consistency` ⇒ **33 passed / 0 failed**
（31 → 33：运行门 + 反漂移门）。运行门首轮 FAILED 的那次不是噪音 —— 它就是上面三个缺陷的现场，
修完才绿。定点 clippy（`axagent` / `harness` / `analysis-engine` / `rt-workflow`，
`--all-targets --all-features -- -D warnings`）与 `cargo fmt --all --check` 的读数见本节末补记。

**还差什么才算「四路能上生产」**：脚本已经能跑，但**没有节点跑它** ——
P4′-c 第 1 片（父图 4 个节点/子模板 + arbiter 产出 `decisionsByHorizon`、§11-3 退役）
才是把它接进真实运行的一步。

### 28-5 补记：钉工具链后的最终读数 + 一条与本批无关的旧账

本批全部静态门用**pin 住的 1.97.0**（`src-tauri/rust-toolchain.toml`）跑，退出码单独取，
不用管道末位的 `$?`：

- `rustup run 1.97.0 cargo clippy -p axagent --all-targets --all-features -- -D warnings` ⇒ **EXIT=0**
- `… -p axagent-harness -p axagent-analysis-engine -p axagent-rt-workflow …` ⇒ **EXIT=0**（4m21s）
- `… cargo fmt --all --check` ⇒ **EXIT=0**
- `__TAURI_WORKSPACE__=true … cargo test -p axagent --lib seed_consistency` ⇒ **33 passed / 0 failed**

**为什么会红过一次（值得记的机制，不是本批的债）**：把 cargo 改成从仓库根 `--manifest-path` 跑之后，
rustup **不读子目录的 `rust-toolchain.toml`** ⇒ 实际用 default stable 1.96.1，
它对 HEAD 里早已存在的一行报 `nonminimal_bool`
（`crates/analysis-engine/src/recommender/scoring.rs:67`，`git status` 显示该文件本轮未改动，
最后提交是 51ddfec17）。识别方法是看报错 URL 里的版本段（`rust-1.96.0` vs `1.97.0`）。
⇒ 两条规则：① 固定工作目录时必须 `rustup run <pin> cargo --manifest-path …` 显式带 pin；
② **工具链版本本身就是门禁覆盖面**，换版本等于换一个门 —— 判「某行不合规」之前先确认是哪个 clippy 在判。
本轮另一个真红是自己造的：`///` 列表项的续行没缩进 ⇒ `doc_lazy_continuation`
（AGENTS.md 已记过同类事故：改注释也会红），按建议缩进即修。


## 二十九、P5 仲裁的算法内核先落地（可比口径 + 容错分列，2026-10-03）

节点还没接，但**口径与判据先固定**并全部跑通，避免接线时边写边改语义。

### 29-1 排名口径：每日对数资本增长率

`portfolio_formula::kelly_growth_rate(posterior, odds, position_pct, holding_days) -> Option<f64>`：
`g = [p·ln(1+f·b) + (1−p)·ln(1−f)] / days`，`f = positionPct/100`（Kelly 1956 的增长率，除以持有天数
= 每日期望对数增长）。宿主函数 `pm_kelly_growth` 注册在 `rhai_pm.rs`，Rhai 侧无 `ln` 所以只能在这算。

**为什么不能用「哪档仓位大 / 哪档置信高」**：8% 仓锁 2 天与 30% 仓锁 90 天不可直接比。
单测 `growth_rate_normalises_by_holding_period` 就把这条钉成断言：同 p 同 b 下
超短（8%/2 日）**必须**胜过长线（30%/90 日）—— 若哪天有人把排名改回比仓位，这条立刻红。

### 29-2 缺席与不可比必须分列（P5′ 的核心那条）

`portfolio-mgr-arbiter.rhai` 的三态：`ranked` / `ineligibleTiers`（有结论但零仓位、赔率非正、
持有 0 天、缺仲裁要的字段）/ `absentTiers`（该路节点压根没出东西）。
**两类都不得进比较，也都不记 0 分** —— 把缺席读成 0 增长 = 「缺席 = 看空」，正是 §十一 11-5 的靶子。
全不可比 ⇒ `primaryTier = ""` + `reason` 点名「不做硬选」。
越界概率 / NaN / `f=1`（`ln(0)`）在 Rust 侧一律 `None`，**负增长率仍返回 `Some`**
（"算出来不划算" 与 "拿不到数" 是两件事，`absence_is_none_and_negative_edge_stays_some` 锁住）。

职责边界照用户裁定 Q3：只输出新字段 `arbitration`（`scope = "capital_allocation_only"`），
`decisionsByHorizon` 与按档复盘**一字不动**。

### 29-3 这条门顺手抓出的是**我自己测试里的三个缺陷**（不是脚本的）

`arbiter_rhai_executes_and_never_scores_absence_as_bearish` 首轮 FAILED，三处都修在测试侧：

1. 闭包签名 `&[(&str, Value)]` 与实参 `&Value` 不匹配（编译期即红）；
2. **断言用的键名与脚本输出契约不符**（`absent` vs `absentTiers`），而 `list_len` 辅助函数
   对不存在的键**静默返回 0** ⇒ 「契约名拼错」与「确实没有缺席档」在断言里长得一样。
   ⇒ 已改成缺键直接 panic 并写明原因。这是本仓「弱断言」家族的新一例，
   **辅助取值函数默认值不能与"空集合"同形**；
3. 第③条用例期望值写错：我只注入两档却断 `absent == 1`（另两档当然也缺席）。
   改成「四档全注入、只有长线零仓位」的干净形态，这样"零仓位被错算成缺席"才会被抓到 ——
   脚本行为自始至终是对的。

### 29-4 读数与**尚未做**的部分

`__TAURI_WORKSPACE__=true rustup run 1.97.0 cargo test -p axagent --lib seed_consistency` ⇒ **34 passed / 0 failed**；
`kelly_growth` 两条 Rust 单测过。定点 clippy / `fmt --all --check` 读数见本节末补记。

P5 的另一半**没做**，别当已完：`source_credibility`（孤儿表 `weight-tables.json:53-63`）接入仲裁、
以及 B 级节点处置（(维度×档) 连续异常持久计数 + `weight_decay` 降权 + grade Switch 熔断路由）
都要等 arbiter 真进图之后才有载体。arbiter 目前也**仍无节点调用**。


## 三十、接枝前的接线契约：每条腿的数从哪个节点拿（2026-10-03）

四份分支脚本 + arbiter 都「能跑但没节点调用」。接线时最容易犯的错**不是写错路径，而是猜路径**：
`input_mapping` 写一个不存在的点路径，rt-workflow 会**安静地补 unit** ⇒ 该腿恒缺席，
而面板只看到「证据不足」。所以先把每条腿的供给源逐个定性，⚠ 项在实测确认前**一律不写进映射**。

| 因子 | 属主（= 唯一供给方） | 脚本读的变量 | `input_mapping` 源 | 定性 |
|---|---|---|---|---|
| `trendStrength` | a-market-analyst | `tier_score` | `t-scoring-{hour,week,month,quarter}.result.content.totalScore` | ✔ 主链现有四条映射在用 |
| `momentumSignal` | a-market-analyst | `macd_dif`/`macd_dea`/`rsi_14` | 同上节点的 indicators | ⚠ 主链读得到这三个名字，但**按档换源节点后**要实测 indicators 在 `.content` 下第几层 |
| `eventCatalyst` | a-catalyst | `catalyst_level` | `a-catalyst.content.verdict.catalyst_level` | ✔ 主链在用 |
| `flowPersistence` | a-hot-money | `flow_persistence` | `a-hot-money.content.verdict.flowPersistence` | ✖ 依赖逐档契约（子模板必须注入 `horizon`，见 §二十五 门 f）⇒ 未接前该腿**恒缺席，脚本会点名** |
| `breadthState` | a-sentiment | `seal_rate` | `t-limitup-pool.result.content` →（`safe_parse`）`breadth.*` | ⚠ 内层键名须按 `LimitUpBreadth` 的 serde 实际输出核对（禁猜） |
| `microstructure` | a-hot-money | `pool_in_pool`/`pool_break_count` | 同一份池的 `entries[]` 里按 `stock_code` 命中本行 | ⚠ 需要「按代码取本行」这一步，落在节点侧还是脚本侧**先定再写** |
| `valuationBand` | value-investor | `pe_percentile` | `t-valuation-band.result.content.metricPe.currentPercentile` | ✔ 主链在用 |
| `earningsQuality` | a-fundamentals | `f_score` | `t-valuation.result.content.fScore.score` | ✔ |
| `expectationRevision` | a-research | `consensus_eps`/`latest_eps` | `t-consensus-data`（**本片刚补的节点**）+ 最近实际 EPS | ◐ 节点已补；`latest_eps` 源未定 ⇒ 未定前该腿缺席 |
| `macroRegime` | a-policy | `pmi` | `t-macro-data.result.content` →（parse）series | ⚠ 同上，内层形状实测为准 |
| `supplyShock` | a-lockup | `lockup_float_ratio` | `t-lockup-data.result.content` →（parse）解禁市值 ÷ 流通市值 | ⚠ 比值若工具不直接给，**不得用现算近似冒充**（裁定③） |
| `sectorRotation` | a-sector | `industry_rank_percentile` | `t-sector-data.result.content` | ⚠ 长档只作背景（条件字段），接不上也只是背景缺失 |

### 30-1 三条接线纪律

1. ⚠ 项一律**先实测再写映射**：读一次真实节点输出（`pg-connect.mjs` 查 `node_executions`
   的 variables，或临时日志），确认键名与层级后再落 `input_mapping`；
2. 接不上就让腿缺席 —— 四份脚本与 arbiter 都已把「缺席」实现成 `absentLegs` / `absentTiers`
   并留 `dataGaps`，**不需要**任何兜底假数据；
3. 每接完一条腿就跑 `cargo test -p axagent --lib horizon_branch_rhai_scripts_execute` 与
   arbiter 那条门，并重启看该腿是否从 `absentLegs` 消失 —— 消失才算接通，
   映射写了但路径错在门里是看不出来的（这就是 §28 抓到的第三类缺陷形态）。

### 30-2 本片顺手补的一处结构性缺口（已并入 v123）

`get_stock_consensus_eps` 的 ToolDef 与 `PROFILE_TOOLS` 授权**早就有**（还授给了 research-analyst
与 fundamentals-analyst），但**全图没有任何取数节点** ⇒ 一致预期只能等 LLM 自己想起来调。
新增 `t-consensus-data` → a-research（trigger 扇入 + 出边 + `context_sources` 三处成对，
照 §二十七 的四触点清单）。这是「声明参与、实际恒缺」那条判据在**接枝之前**就抓到的一例。

### 30-3 我试做过一个「接枝前置」机械门，**没通过自证就删了**（别当成已存在的门）

想把它写成 `scripts/check-branch-input-coverage.mjs`：扫五份脚本的**裸读变量**，凡不在
「可注入名全集」（`seed_variables.rs` ∪ `hooks.rs` 注入 ∪ 现有映射键）里就红。第一版输出 320 处
「缺来源」，含 `branch_json`/`prior`/`overall_risk` 这些**确实有来源**的名字，且把 `action` 报成
`actio`、`clamp` 报成 `clam` ⇒ 判据本身失效，不是脚本有 320 个缺陷。两处根因：

1. 正则用了尾向断言 `([a-z_][A-Za-z0-9_]{2,})(?!\s*[:=(])`，被**回溯**截掉最后一个字符
   （`action:` 整体匹配失败后，引擎回退成 `actio` + 下一字符 `n` 通过断言）—— 标识符抽取必须
   带右词边界 `\b` 并配左逆视 `(?<![\w.])`；
2. 「可注入名全集」的解析面不全：hooks 里跨行写的 `name:\n "x".into()` 没被抽到。

⇒ 已删该文件（未进 CI、未被引用），并记此条防止下一次有人把同类草稿当成品。
**结论不变**：§30-1 表里的 ✔/⚠ 定性目前依据是「主链现有映射逐条读到源码」，
要把它变成机械门，得先把抽取器写对（词边界 + 全集解析面），再按本仓规矩配 `--selftest` 负控。

## 三十一、接枝的可粘贴形状（本轮不再改 seed，避免半落地）

我选了「先接 ✔ 四条」，但**不在这一轮动手**：Seed 图改动要 ~200 行内联 CodeNode，而我的上下文已不足以承受「写完 → 编译报错 → 修到一半」的风险，那会留下不可编译的种子。把形状先钉死在这里，下一步照抄即可。

### 31-1 先改四份脚本的一处依赖（接枝的前置，零现网影响）

脚本现在写 `let prior_val = prior;` —— 而 `prior` 在主链是**脚本内算出来的**（由 `market_regime_prior` + `regime_prior_bull/sideways/bear` 推），不是工作流变量。接线时既不能注入一个不存在的名字（裸读 ⇒ `Variable not found`），也不能各抄一份先验推导（第二份口径）。改成「先验只认 `horizon_prior_json`，两条路都拿不到就拒绝出结论」：

```rhai
let hp_row = if type_of(horizon_prior_json) == "map" { horizon_prior_json["<TIER>"] } else { () };
let prior_ok = type_of(hp_row) == "map" && type_of(hp_row["prior"]) != "()";
if !prior_ok {
    gaps.push("本档先验不可得（horizon_prior_json 缺失或缺该行）=> 拒绝输出结论：不注入猜测先验，也不在脚本里重推一份");
}
let prior_val = if prior_ok { hp_row["prior"] } else { () };
```

随后 `posterior` 一行加 `prior_ok` 前置，false 时走脚本已有的「无在场方向腿 => 观望 + 0 仓」出口。

### 31-2 一个分支节点的形状（照 data-quality / portfolio-mgr 的同款内联写法）

```rust
nodes.push(WorkflowNode::Code(CodeNode {
    base: WorkflowNodeBase {
        id: "pm-h-ultra-short".into(),
        title: "超短分支决策".into(),
        description: Some("只吃本档证据的决策分支（R-11）".into()),
        position: Position { x: 840.0, y: 4200.0 },
        retry: RetryConfig::default(),
        timeout: Some(10),
        enabled: true,
        parent_id: None,
        compensation: None,
        continue_on_fail: true,
    },
    config: CodeNodeConfig {
        language: "rhai".into(),
        code: include_str!("../portfolio-mgr-h-ultra-short.rhai").to_string(),
        output_var: "h_ultra_short".into(),
        tool_name: None,
        execute_directly: true,
        input_mapping: std::collections::HashMap::from([
            ("branch_json", "horizon_branch_json.ultra_short"),
            ("tier_score", "t-scoring-hour.result.content.totalScore"),
            ("catalyst_level", "a-catalyst.content.verdict.catalyst_level"),
            ("macd_dif", "t-scoring-hour.result.content.macdDif"),
            ("macd_dea", "t-scoring-hour.result.content.macdDea"),
            ("rsi_14", "t-scoring-hour.result.content.rsi14"),
            ("horizon_prior_json", "horizon_prior_json"),
            ("overall_risk", "risk-agg.result.stockRiskProfile.riskLevel"),
        ]),
    },
}));
edges.push(edge("e-pm-h-ultra-short", "a-catalyst", "pm-h-ultra-short"));
```

### 31-3 抄之前必须先实测的三处（不许照抄本文件）

1. `t-scoring-hour.result.content.macdDif|macdDea|rsi14` 的**真实键名与层级** —— 主链的
   `macd_dif` 等是从哪个节点/哪一层取的，要用一次真实运行的 `node_executions.variables` 核
   （§30 纪律 1：宁可不接、让腿缺席，也不猜路径）；
2. `risk-agg` 输出的风险等级路径（主链映射里叫什么就照什么，别新造）；
3. `horizon_branch_json` 是否真的能以点路径穿透（它由 `hooks.rs` 以 object 注入；
   `resolve_var_path` 对 map 的逐段解析在生产上只验证过 `t-*.result.content.*` 形态）。

### 31-4 接完必须跑的三条

- `cargo test -p axagent --lib seed_consistency`（34 道，含五份脚本的运行门）；
- `node scripts/audit-inject-coverage.mjs --strict` —— 该脚本目前**只覆盖 portfolio-mgr 一个节点**，
  新分支节点要纳入它的判据面，否则「映射写了但路径不存在」无人报（§30 纪律 3）；
- 重启实跑，看 `node_executions` 里 `pm-h-ultra-short` 的 `absentLegs` 是否随接线逐条缩短
  —— **这是唯一能证明「接通了」的读数**，映射写了不代表到账。

## 三十二、P4′-c 第一片：一个分支节点入图（2026-10-04，形状探针）

**做了什么**：`pm-h-ultra-short`（CodeNode，`output_var = h_ultra_short`，探针期 `continue_on_fail = true`）
接入种子图，配 5 条供给边。其余三档与 arbiter **不在本片**。

**为什么先只接一个**：§三十一 的草稿是「按写法抄」，而本片的价值恰恰在**测出草稿错了**。
实测结论（全部从现网映射 / Rust 源逐字读出，非推断）：

| 问题 | §三十一 草稿 | 实测正解 | 依据 |
|---|---|---|---|
| 指标在哪层 | `t-scoring-hour.result.indicators.*` | `.result.content.indicators.{macdDif,macdDea,rsi14}` | 本文件 :4942–4944 现网映射 |
| `overall_risk` 能否注入 | 直接映射同名变量 | **不存在这个可注入变量**；改 `cls-risk-level.result.category` | 主链只有 `overall_risk_llm`（:4668），且 :492 `overall_risk = overall_risk_llm` ⇒ **同一词表串**，不是近似 |
| 多层点号能否穿透 | 假设可以 | **可以**，且**终值不 auto_parse**（保留原样给 rhai） | `executors/mod.rs:113-171` |
| 涨停池内层键名 | 未接（待测） | `content.breadth.{sealRate,limitUpCount,breakCount}` | `astock-data::types::LimitUpBreadth` + `#[serde(rename_all="camelCase")]`；`breadth=None` ⇒ 导航失败 ⇒ 注入自动补 unit ⇒ 脚本走缺席分支 |

**本片仍未接**（按纪律 1「宁可不接、让腿诚实缺席」）：
- `pool_in_pool`：语义是「本代码 ∈ `entries[]`」，点号路径表达不了数组包含 ⇒ 不硬凑；
- `flow_persistence`：分析师侧还没有逐档契约，不会吐出该 verdict 字段；
- 宏观内层形状：属 a-policy，不在超短腿集合内。

**刻意没抄的东西**：`pe_percentile`（超短无 `valuationBand` 腿，写进来就是白注入 + 一条多余的 `t-valuation-band` 供给边，正是本仓反复出现的「映射声明了没人读的入边」形态）。

**节点构造形态**：`input_mapping: [ ("k","v"), … ].into_iter().map(|(k,v)| (k.to_string(), v.to_string())).collect()`
（种子内 11 处同形；`HashMap::from([(&str,&str)])` **不编译**，本片第一次就踩了）。

**同批修掉的运行时红**：`seed_consistency_tests.rs:2959` 对 `&Dynamic` 调 `as_string()` ⇒ `E0507 cannot move out`
（`as_string` 消费 self）。改 `g.clone().try_cast::<String>().is_some_and(...)`。
这是「**编译门才是第一道门**」的又一次实证：§三十一 写的那段断言在落地前从未过过编译。

## 三十三、其余三档的入参路径实测（接第二片之前必须做完的测量）

**测量姿势本身是一条教训**：第一次普查 `input_mapping` 用的是「同一行内 `("k", "v")`」正则，
漏掉了种子里 **12 处跨行书写**的映射对（`(\n "k",\n "v",\n)`），差点据此判成
「`valuation_dcf_applicable` 生产里根本没注入」。补用「按 key 名单独 grep」才拿到真值
（`seed_stock_analysis.rs:4729-4732`）。⇒ **计数型结论必须两种姿势各跑一遍对上才算数。**

| 入参 | 可注入路径（逐字出处） | 结论 |
|---|---|---|
| `pe_percentile` | `t-valuation-band.result.content.metricPe.currentPercentile` | ✔ 可接。`ValuationBand`/`MetricBand` 均带 `#[serde(rename_all="camelCase")]`（`valuation_band.rs:19/31`），`current_percentile: Option<f64>` ⇒ `None` 时导航失败 ⇒ 注入补 unit ⇒ 诚实缺席 |
| `valuation_dcf_applicable` | `t-valuation.result.content.dcf.assumptions.applicable` | ✔ 可接，主链已在用（:4730） |
| `valuation_dcf_inapplicable_reason` | `t-valuation.result.content.dcf.assumptions.inapplicable_reason` | ✔ 可接（留痕用） |
| `pmi` | `t-macro-data.result.content.pmiManufacturing.value` | ✔ 可接（`MacroDataSnapshot`/`MacroDataPoint` 皆 camelCase，`value: f64`） |
| `consensus_eps` | `t-consensus-data.result.content.consensusEps` | ⚠ **可接但必须带闸门**：`ConsensusEPS.is_estimated`（`types.rs:310`）文档明文「下游不得用估算值做超预期/不及预期判定」⇒ 同批注入 `consensus_estimated`，脚本内 `is_estimated==true` ⇒ 该腿按缺席，不吃这个数 |
| `lockup_float_ratio` | — | ✖ **没有标量路径**：`LockupSchedule.unlockRatio` 是**逐条解禁事件**（`types.rs:121-129`），腿要的是「近窗最大/合计占比」= 数组归约，点号表达不了 ⇒ 正解是注入 `lockup_bundle` 串、在脚本内 parse+归约（主链对同一份 bundle 已是这个形态） |
| `pool_in_pool` | — | ✖ 同上：要判断本代码 ∈ `entries[]` ⇒ 注入涨停池串后在脚本内判 |
| `latest_eps` | — | ✖ **图上没有财报节点**：`get_stock_financials` 只在 `tool_def_map`/`PROFILE_TOOLS`（:1848/:2306），由 a-fundamentals 自行调用 ⇒ 无确定性节点可指。要么补一个 `tool_node`（与 §二十七 给 consensus/macro/limitup 补节点同形，**不是新增取数实现**，只是把已有工具前置供给），要么该腿按缺席 |
| `flow_persistence` | — | ✖ 属主 a-hot-money 的 verdict 里还没有这个字段（要先落逐档契约）⇒ 缺席 |
| `industry_rank_percentile` | — | ✖ 行业排名通道的 as-of 结构性缺口未闭（见既有记载）⇒ 缺席，不用个股涨幅近似 |
| `stop_vol_mult` / `take_profit_vol_mult` | 同名工作流变量（`seed_variables.rs:2834/2843`，登记于面板可调清单 :91-92） | ⚠ 分支节点**必须**做恒等映射：探针片没接 ⇒ 脚本 `else { 1.2 }` 的默认值把「面板调过」静默吃掉了 —— 这不是缺席而是**假装是面板值**，属下一片必修 |

## 三十四、接线测出的权威表漂移：超短的「催化剂腿」（2026-10-04，第二片第一动作）

接探针节点时按「这条映射被谁消费」逐条回查，测出**三处不同步**，都不是文字问题：

1. **`limit_up_count` 是白注入**：`breadthState` 腿的信号只读 `sealRate`（`leg_signal.rs:127`），
   四份脚本全文没有任何一处读 `limit_up_count` ⇒ 已从节点映射与注释中删除。
2. **`catalyst_level` 在超档拿不到消费方**（本片的真发现）：
   - `PLAN §十一` 的逐档分支表写「超短 = 动量 / 情绪广度 / **催化剂**」；
   - 但 `Period::UltraShort` 的 `participating = [momentumSignal, breadthState]`、
     `qualified = [microstructure]`（`holding_period.rs:344-348`），而 `VERDICT_FACTORS`
     共 11 项里**根本没有 `eventCatalyst`**（:93-105）⇒ 分支表不会产生催化剂腿；
   - 同一层的 `evidence_weight::get_horizon_base_weights("ultra_short")` 却给
     `a-catalyst` 登记了权重 1.0（:298）⇒ **分析师被计权、但没有腿**，那个权重是惰性的；
   - `leg_signal::SUPPORTED_FACTORS`（12 项，含 `eventCatalyst`，:154-167）与 `VERDICT_FACTORS`
     （11 项）两个「因子全集」本来就差这一项，而 `supported_factors_equal_the_branch_source_table`
     只锁 `SUPPORTED_FACTORS ↔ VERDICT_FACTOR_SOURCES` 的**条数**，锁不住「某档实际用不用它」。
   ⇒ 本片先把 `catalyst_level` 映射与 a-catalyst 供给边**摘掉**（不留惰性接线），
   下一片第一动作是把 `eventCatalyst` 正式纳入超短契约，牵动面按序：
   `VERDICT_FACTORS` 11→12 + `na_key` + `VERDICT_FACTOR_OWNERS` + `VERDICT_FACTOR_SOURCES`
   + `Period::UltraShort.plain` + 逐档字段计数测试（现 pin 3/5/6/6）+ `analyst_subset` 推导
   + 契约 prompt 渲染 + `check-horizon-weight-parity.mjs` 的权威表可落地判据 ⇒ 然后把映射与边接回。
   **判据要跟着改**：加一条「`get_horizon_base_weights` 里非惰性的分析师，必须在该档 `analyst_subset`
   里有条腿」的反惰性门 —— 这次是手工回查发现的，不该指望人。
3. **面板乘数在分支里被默认值吃掉**：`stop_vol_mult` / `take_profit_vol_mult` 是登记在面板可调清单
   （`seed_stock_analysis.rs:91-92`，变量定义 `seed_variables.rs:2834/2843`）的工作流变量，
   而分支脚本写的是 `if present(...) else { 1.2 }`。探针片若不映射，面板用户改到 1.5 时分支仍按 1.2 算
   —— 这不是「缺席」（缺席要留痕），是**拿默认值假装是面板值**。⇒ 下一片补恒等映射
   `("stop_vol_mult","stop_vol_mult")` / `("take_profit_vol_mult","take_profit_vol_mult")`（主链同一形态）。

**检索姿势更正（自犯）**：第一次普查用的是「同一行内 `("k","v")`」正则，漏掉种子里 12 处跨行书写，
差点误判 `valuation_dcf_applicable` 未注入。已在 §三十三 更正；记忆条目 4aa 同批落。

## 三十五、运行门的 scope 必须是「节点那一份」—— 否则它测的是另一个程序（2026-10-04）

**发现方式**：把分支节点接进图之后，用 `audit-inject-coverage.mjs` 的 ② 倒查（当日刚修的
「仅注释提及不算消费」）抓到 `limit_up_count` 白注入；顺着「脚本读的名字 vs 节点映射的键」
逐条对账，发现另外三个名字 `catalyst_level` / `flow_persistence` / `pool_in_pool`
**脚本在读、节点没映射**，而 §三十二 那批运行门全绿。

**根因是门的 scope 造得太宽**（不是代码错，是检法错）：

- 生产语义（实测读码）：`code_executor.rs:184` 只把 **`input_mapping` 的 key** 推进 scope；
  `resolve_var_path` 返回 `None` 或 `Null` 才落成 `Dynamic::UNIT`（:191）。
  ⇒ 「映射过但取不到值」= unit = 可被 `present()` 判缺席；
  **但「压根没映射的名字」= `Variable not found` = 运行期错误 ⇒ 整节点失败**（本仓
  `Variable not found: horizon_prior_json` 的同型事故就记在 `rhai_registry.rs:826`）。
- 门的 scope 来自测试自己手抄的 24 项常量清单，比生产节点的 11 项宽 ⇒ 三个越界读取在门里
  是 unit（顺利跑通并正确退化成观望），在生产里会把整个分支节点打死。
  **门测的是「另一个程序」，而它的绿被当成「脚本跑得通」。**

**收口**：新增门 `branch_node_scope_is_exactly_the_seed_mapping` —— scope 的**唯一来源换成种子**
（解析该节点 `input_mapping` 区块的 target 键集合，剥注释后按字符串字面量奇偶交替取键，
不用正则：rustfmt 会把长名元组拆行，本文件另一处门为此踩过三次），再用生产同一个引擎工厂 +
同一个沙箱档位跑两态。三条断言：① 齐备态跑通且形状合规；② 退化态观望 + 不产仓位；
③ **负控**：剥掉一个脚本真读的名字（`seal_rate`）必须**运行失败且点名它** —— 否则本门对该档
是空的（「红必须来自 scope」由这条自证，而不是靠我声称）。
配套的强约束：齐备态的键若在本门值表里登记不到 ⇒ 红（防止新接一个键后「齐备态」悄悄缺料）。

**本门首跑红过一次，红因是门自己**（实施时更正，原文保留）：报
`Variable not found: seal_rate (line 88)` —— 听着像节点漏接线，其实节点接了；
真因是解析函数**先找闭合 `]` 再剥注释**，被区块上方注释里的 `#[serde(rename_all="…")]`
（自带 `]`）提前截断 ⇒ 少收两条映射 ⇒ 门把自己的解析缺陷说成了被测代码的缺陷。
⇒ 已改成「先剥注释、再定位闭合括号」，并在解析函数里加三条**结构自证**：
① 键集合必须含 `branch_json`（分支节点都必接，缺 ⇒ 区块被截断）；
② 每个键必须是小写标识符（抽到注释残渣/属性片段当场红）；③ 键数下限 8。
⇒ 通则：**任何「从源码抽清单」的判据，必须能自证「抽全了」**，否则它的红与绿都不可解释。

**覆盖面限定（写进门注释，不自夸）**：Rhai 的变量查找发生在运行时，只有**执行到的路径**上的
越界会报；未走到的分支仍会漏。静态穷查要词法器，而那条门已因自证失败被删（§三十 30-3），
这里不重犯 —— 本门只掐掉「测试 scope ⊋ 生产 scope」这一个假绿来源。

**代码侧连带修正**（同批）：
- 探针节点删 `limit_up_count`（无腿消费）、`catalyst_level`（§三十四的权威表漂移，接了也是惰性）、
  a-catalyst 供给边；补 `("stop_vol_mult", "stop_vol_mult")` 恒等映射（面板可调乘数不接就是
  **拿脚本默认值冒充面板值**，与「缺席」不同类，必须显式）。
- 超短脚本的 `raw` 只剩 6 个键，与节点映射严格一一对应；并在头部把「已接 / 未接且不读」分开列。
- `microstructure` 腿的缺席原因**单列进 dataGaps**：`absentLegs` 只能说「这条腿没数」，
  分不清「今天没有涨停」与「成员判定没接线」——后者是接线状态，被读成盘面事实就是呈现层歧义。
- `code_only`（剥注释）从单个测试内部提到模块级，两门共用一份（铁律 12）。
- 门禁工具侧：`audit-inject-coverage.mjs` 的 ② 改在**剥注释后的代码域**上倒查，
  `--selftest` 加正控⑧（仅注释提及 ⇒ 报悬空）与负控⑧b（注释 + 代码都有 ⇒ 不报），
  并新增 `--include-marker` / `--push-marker` 让同一个审计能指向**任意** CodeNode（此前锚点写死在 portfolio-mgr）。

## 三十六、下一片接线前必须先钉的三件事（2026-10-04 实测，非推断）

1. **`lockup_float_ratio` 有量纲陷阱**（接它之前不钉住就是必然 bug）：
   - 来源侧 `LockupSchedule.unlockRatio` 是**百分数** —— 前端按 `(…).toFixed(1)}%` 渲染
     （`src/components/stock-analysis/EventCalendarPanel.tsx:56/194/263`），
     主链脚本的注释也写「unlockRatio 表示解禁股占总股本比例」（`portfolio-mgr.rhai:1270`）；
   - 消费侧 `leg_signal` 的 `supplyShock` 锚是**小数** 0.10 = 满强度（`leg_signal.rs:122`，
     `-cl(r / 0.10)`，另有 `lockupFloatRatio: 0.05 ⇒ −0.5` / `0.20 ⇒ −1.0` 两条单测钉住）。
   ⇒ 直接喂会把「解禁 10%」当成「解禁 1000%」⇒ 该腿**恒打满负分**，而两侧各自都自洽。
   归约处必须 `/100`，并且门要合成一条 `unlockRatio: 10.0` 的 bundle 断言腿信号恰为 −1.0
   （断言取的是**换算后的值**，不是「解析成功」）。
   ⚠ 另一条同源风险在取数层：`baidu_stock.rs:450` 用 `unwrap_or(0.0)` —— 「厂商没给这一列」与
   「解禁占比 0」在数据上同形。本片的腿不消费它时无害，接上之后 0 会被读成「无供给冲击 ⇒ 中性偏正」。

2. **`leg_signal.rs:126` 的注释声明了一个不存在的消费方**：原文写「涨停家数不进信号，
   只做 `breadth_required` 门的输入」，而那道门只读 `seal_rate`（`portfolio-mgr-h-ultra-short.rhai:133`）。
   ⇒ 更正注释而不是新造判据：把「家数下限」加进门是**新判据**，不在已批准的 §十一 范围内。
   这条也正是 §三十五 里 `limit_up_count` 被摘掉的依据 —— 「注释说它有角色」不等于「代码消费它」。

3. **赔率（`odds`）的形状要说清，别让它冒充标的信息**：超短/短/中三档的止盈止损都按
   `k·σ_daily·√days` 推导 ⇒ `odds = tp/stop = k_tp/k_stop`，σ 与 √days **在比值里被约掉**，
   与标的无关（超短取 `tp = 1×σ√d` ⇒ `odds = 1/k_stop`）。
   这不是缺陷 —— 波动带本身就是出场规则，三阶障碍法的赔率确实由带几何给定；
   但它意味着**这三档的仓位只由 posterior 决定**，面板调的是带的形状而不是「这笔交易的赔率」。
   长档相反：`tp` 取 DCF 上行空间（%），与 σ 位移**量纲不同** ⇒ `odds` 真随标的变，
   也正是 §裁定③「没有目标价 ⇒ 赔率无定义 ⇒ 不出仓位」只适用于长档的原因。
   ⇒ 要求：契约文案与面板 tooltip 不得把三档的 `odds` 写成「本标的赔率」，
   应写成「由出场带给定的固定赔率（k_tp : k_stop）」。
   **现状定性（不是已发生的缺陷）**：全仓检索 `odds`/`oddsSource` 只有类型声明
   （`src/types/stock-analysis.ts:233-235`），**没有任何组件消费它**；面板当前显示的「Kelly 赔率」
   走的是另一个字段 `payoff_ratio`（`src/components/stock-analysis/utils.ts:81`）。
   ⇒ 这条是「接分支输出到面板时**必须带上**的标注要求」，本片不改前端。

## 三十七、第二片可直接照做的接线清单（全部路径已实测，无需再猜）

**新增的生产侧硬规则**（§三十五 的推论，写在这里以免下一片又靠记忆）：
脚本 `present(X)` / 裸读 `X` 的每一个名字，**必须出现在该节点的 `input_mapping` 键集合里**。
「映射了但路径取不到」= `Dynamic::UNIT` = 诚实缺席（`code_executor.rs:191`）；
「没映射」= `Variable not found` = **整节点失败**。两者在面板上长得一样，在运行时完全不同。

| 档 | 节点 | 映射（target ← source） |
|---|---|---|
| 超短 | `pm-h-ultra-short` | **已落**：branch_json / horizon_prior_json / tier_score(`t-scoring-hour.result.content.totalScore`) / macd_dif / macd_dea / rsi_14（同节点 `.content.indicators.*`）/ overall_risk(`cls-risk-level.result.category`) / kline_bars(`t-scoring.result.content.kline_json`) / stop_vol_mult(恒等) / seal_rate / pool_break_count(`t-limitup-pool.result.content.breadth.{sealRate,breakCount}`) |
| 短 | `pm-h-short` | 同上，把 `t-scoring-hour` 换成 `t-scoring-week`；加 `take_profit_vol_mult`(恒等)；腿集含 `supplyShock` ⇒ 加 `lockup_bundle ← t-lockup-data.result.content`（**串**，脚本内 parse） |
| 中 | `pm-h-mid` | `t-scoring-month` + `pe_percentile ← t-valuation-band.result.content.metricPe.currentPercentile` + `f_score ← t-valuation.result.content.fScore.score` + `consensus_eps ← t-consensus-data.result.content.consensusEps` + `consensus_estimated ← …content.isEstimated` + `lockup_bundle` + `flow_persistence ← a-hot-money.content.verdict.flowPersistence`（今日恒 unit ⇒ 腿缺席，P4 契约落地即自激活） |
| 长 | `pm-h-long` | `t-scoring-quarter` + `pe_percentile` + `f_score` + `consensus_eps` + `consensus_estimated` + `valuation_dcf_applicable ← t-valuation.result.content.dcf.assumptions.applicable` + `valuation_dcf_upside ← …dcf.upsidePct` + `pmi ← t-macro-data.result.content.pmiManufacturing.value` + `flow_persistence` |
| 仲裁 | `pm-arbiter` | `r_ultra_short ← pm-h-ultra-short.result` … 四路（**节点 id + `.result`**，与 `("pm_result","portfolio-mgr.result")` 同形；引擎同时按 node_id 与 output_var 写键，见 `rt-workflow/.../engine/mod.rs:1307-1309`）；出边四条 `e-<分支>-pm-arbiter` |

**四条待办，按依赖排序**：

1. **`catalystLevel` 从四份脚本的 `raw` 里删掉**（今日没有任何一档的腿集含 `eventCatalyst`；
   留着就是「脚本准备消费、表永远不会调用」的惰性形态）。与任务 #22（把 `eventCatalyst`
   正式纳入超短契约，并复核 §十一 给短档列的「催化剂日历」是否也该成腿）**同批**再一起加回。
2. **超短的 `microstructure` 要真能评估**：注入 `limitup_pool_json ← t-limitup-pool.result.content`
   （终值是 JSON **串**，`resolve_var_path` 对终值不 auto_parse）+ `stock_code`（工作流变量，恒等映射），
   脚本内 `json_parse` 后判 `entries[].stockCode == stock_code` ⇒ 得到 `inLimitUpPool`。
   这样 `absentLegs` 里的 microstructure 才会随池子数据真的缩短。
3. **`lockup_float_ratio` 的量纲**：来源是**百分数**、腿锚是**小数** ⇒ 归约处 `/100`，
   并给门合成一条 `unlockRatio: 10.0` 的 bundle 断言腿信号恰为 −1.0（§三十六 第 1 条）。
4. **`expectationRevision`（中/长参与腿）现在没有实际数据源**：`consensusEps` 有了（P9-3），
   但「最近实际 EPS」在图上无节点 —— `get_stock_financials` 是**既有工具、缺节点**
   （只出现在 `tool_def_map`/`PROFILE_TOOLS`）。补法与 §二十七 给 consensus/macro/limitup
   补节点同形（不是新取数实现）：`tool_node` + trigger 入边 + 消费声明；
   脚本侧从 `content`（`Vec<FinancialReport>` 的串）取**最新一期非 null 的 `eps`**。
   ⚠ 这条与 as-of 语义耦合：`FinancialReport` 的到期口径已由 T 轮按申报/披露日收口，
   取「最新一期」必须沿用工具返回的顺序，不要在脚本里自己按日期重排。

**不做的事**（避免范围漂移）：不在本片动 §11-3 的退役、不加第五个可调参数、
不把 `odds` 的带几何性质改成别的口径（§三十六 第 3 条只要求**文案说清**）。

## 三十八、第二片落地时的两个自我否决（2026-10-04）

1. **`format!` 拼出来的映射路径 = 把判据饿死**（写完循环后当场否掉，改成三块显式节点）：
   第二片最初写成一个 `for (tier, node_id, score_node, extra) in branch_seeds` 循环，
   路径全部 `format!("{score_node}.result.content.totalScore")` 现拼。看着是去重，实际造成两处**静默失效**：
   - 刚建的 `branch_node_scope_is_exactly_the_seed_mapping` 靠**读字面量**拿每档键集合 ⇒ 区块里
     根本没有 `input_mapping: [` 形态（映射是 `Vec` 现建的），门会对这三档**无从下手**；
   - 源侧路径检法（`check-input-mapping.mjs` 一类，按字符串形状判「第 2 段是不是 content/result」）
     同样看不见拼出来的串 ⇒ 路径写错只能等运行期。
   ⇒ 与本仓「工具名必须写字面量，`tool_node_positional_tool_names` 才查得到」是同一条理由。
   **这里重复的是节点声明（种子内本来就有上百个），不是类型/函数定义 ⇒ 不触禁区 12。**
   判据：**去重若让某条门失去读取面，就不是去重而是拆门。**

2. **改 Rust 的时机**：上一条重构发生在一次批级编译**进行中**（`bbzkm9e94`），
   那次运行的读数因此作废（不能拿来当「第二片过了」的证据），改完后重跑一次才算。
   这条是本仓既知陷阱（「主 crate 门禁是批次级，跑着时别改 Rust」），我又犯了一次 ——
   记进回报而不是悄悄当成没发生。

**仲裁节点也纳入同一条门**（不是复制一份判据）：`mapping_keys()` 增加 `required_key` + `min_keys`
两个入参，分支节点传 `("branch_json", 8)`、仲裁节点传 `("r_ultra_short", 4)`；
仲裁另加两条断言：键集合排序后必须恰为 `r_{ultra_short,short,mid,long}`（接线形状锁），
以及四路全 `unit` 时 `primaryTier == ""`（缺席不硬选）+ 剥掉一路必须红且点名。

## 三十九、第二片读数 + 一条门被删掉的交代（2026-10-04）

**已实证**（pinned 1.97.0，退出码逐条单独取）：种子门 **34 passed / 0 failed**（`gate9.log` 正文），
`cargo fmt --all` 通过；`check-horizon-weight-parity.mjs` 主门 + `--selftest` 全绿；
`audit-inject-coverage.mjs` 对四个分支节点与仲裁节点 ①②③ 三段皆空；
另用一次性脚本以**同一种解析姿势**独立复算五个节点的键集合 = 11 / 12 / 15 / 16 / 4（与门的读数一致）。
clippy 在删门之后重跑中（`clip9`）。

**门数 35 → 34 的原因（主动交代，不是悄悄少一条）**：
删掉 `horizon_branch_rhai_scripts_execute_end_to_end`。它的 scope 是测试里手抄的 24 项 `VARS`，
而它存在的全部理由（「跑通而不只是编译过」）已被 `branch_node_scope_is_exactly_the_seed_mapping`
以**生产同源 scope** 覆盖；两条并存时，旧条会因清单不同步而**双向说谎** ——
本次实锤是假红：中/长档新增的 `consensus_estimated` 不在它的 `VARS` 里，于是报
`Variable not found: consensus_estimated`，而生产节点明明接了这个键。
⇒ 保留「scope 来自生产」那一条，把旧条的独有断言逐条迁入：动作六档词表 / 自报档位 /
齐备态必须有腿 / **置信 ∈ [0,100]** / 退化态观望 / **不产出仓位** / **`absentLegs` 必须点名** /
**`dataGaps` 必须留痕** / **退化原因必须点名「先验不可得」** / 负控必须红且点名。
（加粗三条是旧条独有、新条原先没有的，已补。）

**第二片新入图的节点**（全部 `continue_on_fail = true`，输出暂无下游消费 ⇒ 现网结论仍由 `portfolio-mgr` 出）：
`pm-h-ultra-short` / `pm-h-short` / `pm-h-mid` / `pm-h-long` / `pm-arbiter`，并同步 v123 文档（库内仍是 122 ⇒ 不另开版本号）。

**下一步（第三片，§11-3 退役 + 门 b–f）**：主链 f3/f12 改调 `pm_leg_signal`、
删 `leg_mult`/`horizon_leg_multipliers` 的乘数语义、删 `pm_snr_confidence` 的 √(h/28) 折算、
拆掉单一 `horizon_decision` 闭包与 `totalScore_*` 四条入边、`sharesPosteriorWith` 反转成红判据；
每份子模板必须注入 `horizon`（门 b）；`audit-inject-coverage` 扩到这五个节点并进 CI 的 selftest 面。

## 四十、§11-3 退役的真实面积（只读普查 + 顺序更正，2026-10-04）

派了一次**只读**普查（子代理，禁改文件），结论改变第三片的顺序：**不能先删再补，必须先换生产者再删**。

**为什么**：`decisionsByHorizon` 不是「只有闭包在产、没人消费」的死形状 —— 它**今天就有下游**：
`src/lib/agentOutput.ts:402-414`、`src/lib/stock-analysis-utils.ts:1349,1407`、
`src/components/stock-analysis/DecisionBanner.tsx:682-696,1524-1527`、
Rust 读侧 `stock_workflow/decision.rs:730` + `core.rs:1107,1433,2052` + `hooks.rs:1056` +
实体列 `entities/src/stock_analyses.rs:50` + 解析器 `reflection_stats.rs:92`。
⇒ 删掉 `horizon_decision` 闭包 = 现网四档输出直接消失 ⇒ 面板与反思链一起受损。
正解顺序：**先把 `decisionsByHorizon` 的生产者换成四路分支节点的输出（注入进 `portfolio-mgr`），
再删闭包与它的四个附属形态**。

**六项退役对象的定义点与消费点（普查读数，逐条带行号）**：

| 对象 | 定义 | 代码侧消费（注释命中已排除） | 会被改红的测试 |
|---|---|---|---|
| `leg_mult` / `horizon_leg_multipliers` / `horizon_leg_weights_json` | `portfolio-mgr.rhai:2844`（闭包）+ `evidence_weight.rs:412` + `hooks.rs:398-413` 注入 + 种子 :4897 | 脚本 :2867/:2899/:3067；探针断言 `rhai_registry.rs:800`（**键从映射里消失就 panic**） | `evidence_weight.rs:1612/1631/1643/1664`、`portfolio_mgr_tier_params_rhai.rs:109/:122`（逐字锁 `LEG_MULT_FN`）、`:189`、`:204`、`StockAnalysisConfigPanel.test.tsx:67` |
| `pm_snr_confidence` / `snr_confidence` / `SNR_ANCHOR*` | `indicators.rs:411` + 注册 `rhai_pm.rs:261` + `portfolio-mgr.rhai:2026` + `reflection_stats.rs:34` | 脚本 :2914/:2921/:2963；`reflection.rs:3211`、`serenity.rs:1516`、`recommender/mod.rs:915`、`recommender/scoring.rs:75` | `indicators.rs:802/813/828`、`reflection_stats.rs:1347/1385/1429-1438/1451`、**逐字锁** `portfolio_mgr_horizon_lock_rhai.rs:141` |
| `horizon_decision` / `decisionsByHorizon` / `sharesPosteriorWith` | 闭包 `portfolio-mgr.rhai:2893`，调用 :3000/:3013-3017，输出 :3169 | 见上「Rust 读侧 + TS 读侧」整行 | `portfolio_mgr_tier_params_rhai.rs:151/:173`、`portfolio_mgr_same_source_rhai.rs:126/143`、`stockAnalysisHorizon.test.ts:82/100-107`、`agentOutput.test.ts:70` |
| `totalScore_{ultra_short,short,mid,long}` | 只在种子 :4636-4639 注入 | 脚本 :3000/:3015-3017（TS 零消费） | `portfolio_mgr_tier_params_rhai.rs:165-167`（**反向锁**：字面量抄本不得回来）、`audit-inject-coverage.mjs:495-523` 的合成夹具 |
| `risk_bias` 跨档同值 | `portfolio_formula.rs:413` + `rhai_pm.rs:78`，唯一计算点 `portfolio-mgr.rhai:1800` | :1801/:1804/:2910/:2923-2928/:2947 | `portfolio_mgr_horizon_lock_rhai.rs:68/148/178`（读 `effective_posterior`）；分支脚本**不含** risk_bias（只用 `overall_risk` 进 `pm_kelly_position`） |
| `horizon_consts_json` | 产地 `holding_period.rs:247 decision_consts_map` → `hooks.rs:376-388` → 种子 :4892 | 闭包 `horizon_const` :1914/:1918/:1924 → `days_for`/`sl_pct_for`/`tp_pct_for` | `holding_period.rs:459`、`portfolio_mgr_days_for_rhai.rs:79/103/110/119`、`portfolio_mgr_horizon_lock_rhai.rs:92` |

> **就地更正（3b-β 落地后，同日）**：本表里点到的 `portfolio_mgr_same_source_rhai.rs` **已删除** ——
> 它锁的是「逐档乘数与主链同源」，乘数通道整体删掉后判据本身不再成立。独有覆盖的搬家位置：
> 「技术腿随周期衰减 / 估值腿随周期增长」→ `crates/analysis-engine/src/evidence_weight.rs` 的
> `technical_bias_decays_and_valuation_bias_grows_with_horizon`；「腿集合与属主闭合」→
> `scripts/check-horizon-weight-parity.mjs`（判据措辞已改成「属主唯一 + 集合闭合」）。
> 按「删 + 交代」的规矩记在这里，不给后来者留空引用。

**禁词门的自 trap 检查**（这条必须先确认，否则退役动作会被自己的说明文字判红）：
门 a 的 BANNED 清单 `seed_consistency_tests.rs:2713-2717` 作用在 `code_only()` **剥注释之后**的文本上
（`seed_consistency_tests.rs:2899`），且有一条「剥完仍 > 40 行」的负控 :2731 ⇒
四份分支脚本头部那些「⚠ 禁止出现 `leg_mult`…」的说明**不会**判红。已实测成立（本轮多次绿）。

**普查未定论的四条（不当作已知）**：① `config/llm-output-schemas.json:226` 的描述文案是否被 schema-parity
测试锁住（未跑该文件的检索）；② `portfolio_mgr_veto_rhai.rs` 是否断言 `risk_bias`（文件未开）；
③ DB 里历史模板快照仍带旧名（`output/` 被排除）；④ 种子版本号的位置（子代理按
`WORKFLOW_TEMPLATE_VERSION|template_version` 没搜到 —— 真名是 `TEMPLATE_VERSION`，已由主仓读侧确认）。

**切分与本轮已完成的部分**：
- **3a（部分落地）**：**f3 催化剂阶梯已改调唯一实现**（`portfolio-mgr.rhai:390`，已验证 ——
  rtw 五个真跑脚本的目标 26 passed / analysis-engine 黄金值 4 passed / 主 crate 种子门 34 passed）；
  **f12 动量阶梯尚未搬**（等价性已核对完：主链无 `clamp`、多一道 `type_of(...)=="f64"` 守卫，
  其余四态阶梯 + RSI 调制逐情形与 `leg_signal.rs:57-72` 相同 ⇒ 用「None ⇒ 0.0」桥接即逐情形等价，
  而原权重门 `if f12_signal != 0.0` 在缺席路径上行为不变）。等当前 clippy 批次结束后同批落，
  落完必须重跑 rtw + ae + 种子门（**改动前的绿不作数**）。
  ⚠ 本条原文曾把 3a 写成「两处都已改」，实态以本段为准。等价性逐情形：催化剂的空串/`无`/词表外标签
  原本落 `else { 0.0 }`，`leg_signal` 返回 `None` ⇒ 脚本侧统一映射成 `0.0`；动量是纯搬运
  （`leg_signal.rs:57-59` 的 `||` 合并形态与其黄金值单测 `momentum_ladder_matches_the_rhai_it_was_ported_from`
  就是按搬运前的实测值钉的）。零现网口径变化。
- **3b（下一批）**：`decisionsByHorizon` 生产者换成四路分支输出 ⇒ 然后才删
  `leg_mult`/`horizon_leg_multipliers` 乘数语义、`pm_snr_confidence` 的 √(h/28)、`horizon_decision` 闭包、
  `totalScore_*` 四条入边；同时把上表「会被改红的测试」逐条改成**红判据**（`sharesPosteriorWith` 反转、
  `LEG_MULT_FN` 逐字锁改成禁词锁）。
- **3c**：门 b–f（含「每份子模板必须注入 `horizon`」）、`audit-inject-coverage` 纳入这五个节点、v123 收口。

## 四十一、3b 的前置契约：两种输出形状的对账（先定键，再动生产者）

普查确认 `decisionsByHorizon` 的行形状与分支脚本的输出形状**已经分叉**，而前端按行形状读
（`src/lib/agentOutput.ts:402-414`、`src/lib/stock-analysis-utils.ts:1349,1407`、
`src/components/stock-analysis/DecisionBanner.tsx:682-696,1524-1527`）。
⇒ 3b 换生产者之前必须先把这张对账表定下来，否则「换心脏」会同时改掉面板的字段可见性，
而那是两个不同的决定。

| 键 | 主链闭包（`portfolio-mgr.rhai:2953-2975`） | 分支节点（`portfolio-mgr-h-*.rhai` 输出段） | 3b 处置 |
|---|---|---|---|
| `action` / `verdict` / `positionPct` / `confidence` / `posterior` | ✔ | ✔ | 保留，同键同量纲（都是 0-100 与 0-10 粒度） |
| `scoreSource` | ✔ `tier_native`/`daily_fallback` | ✔ 同两值 | 保留（这是「结构性缺席」的机读字段） |
| `stopSource` / `positionSource` | ✔（含 `risk_budget` 三态） | ✔（`kelly_x_position_multiplier` 等新值） | **值域要并集**：面板文案与 `StockAnalysisConfigPanel.test.tsx` 的白名单要同时收新旧 |
| `stopLossPct` / `takeProfitPct` / `odds` / `expectedHoldingDays` | 隐含在价带里（无独立键） | ✔ 独立键 | 3b 起成为**权威**（仲裁节点就读这几个） |
| `confidenceRiskAdjusted` / `posteriorEffective` | ✔ | ✖ | 退役（R-11 之后不再有「同一后验 × 跨期折算」）⇒ 前端两处消费同批删 |
| `snrAnchorDays` | ✔ | ✖ | 退役（√(h/28) 已判为退役形态，门 a 已禁词） |
| `weightsSource` | ✔ `table`/`fallback_unity` | ✖（改为 `legs`/`absentLegs` 自证） | 退役 ⇒ 换成 `legsSource: "horizon_branch_specs"` |
| `sharesPosteriorWith` | ✔（同源注脚） | ✖ | **反转成红判据**：逐档算法已真正不同 ⇒ 「两档同 posterior」不再是「复制粘贴」的证据；`portfolio_mgr_same_source_rhai.rs:126/143` 的锁随本轮翻向（同分不再要求注脚；**缺分支输出却仍出四行**才要求报错） |
| `confLowerBound` | ✔（超短硬写 0.35） | ✖ | 退役到分支表：本档下限属于逐档判据，不该由主链逐档手写 |
| `horizon` / `confidenceMethod` / `exitRule` / `entryGate` / `entryGatePassed` / `legs` / `absentLegs` / `dataGaps` / `priorSource` / `priorSamples` | ✖ | ✔ | 3b 起入 DTO ⇒ 面板新增「本档凭什么这么算」的自证区（这是 R-11 的可观测面） |

**必须由用户拍板的一条（3b 的唯一开口，其余都可自行裁定）**：
分支节点带 `continue_on_fail = true`，四路里某一路运行失败时，那一档在 `decisionsByHorizon` 里怎么办？

- **A（建议）**：该档**显式缺席** —— `decisionsByHorizon` 里不出现这一行，同时 `data_gaps` 出一条
  「<档名>分支未产出（节点失败）⇒ 本档无独立结论」。代价：面板可能出现「只有三档」，
  但这是真实状态，且与既有裁定「结构性缺口不得在 UI 造成歧义」一致。
- **B**：保留该行并写 `action: "无结论"` —— 形状稳定（永远四行），但要求前端新增一种行语义，
  改动面更大，且容易把「节点挂了」和「算完认为无方向」混在同一种显示里。
- 现网今天的行为是「永远四行、且四行同算法」——那正是 R-11 要废除的，所以**不存在零变化的选项**。

**用户已拍板（2026-10-04）：选 A（该档显式缺席）** —— 我先前按两条既有裁定的合取自行选了 A 并写出依据，提问核对时用户点了同一个选项，故本节从「自行裁定」升为**已批口径**。
判据不是偏好，而是两条既有裁定的合取：
① 「结构性缺口不得在 UI 造成歧义」——「拿不到」可接受、伪装成正常结果不可接受；
B 方案保留行 + `action:"无结论"`，视觉上仍是「这一档有结论」，需要新增一种行语义才不歧义，
   等于把「节点失败」的归因负担压到展示层；
② R-11 的立论本身就是「四档必须有各自真实的算法」——那么「某档的算法这一轮没跑出东西」
   就是**该档无结论**，而不是「该档结论 = 无结论」。
⇒ 落地形态：`decisionsByHorizon` 缺该键 + `data_gaps` 出一条
`"<档名>分支未产出（节点失败）⇒ 本档无独立结论"`；
配一条门：四路里任何一路缺失时，`data_gaps` 必须有点名，且**不得**在 `decisionsByHorizon` 里
出现凭空补出来的行（红判据方向与 `sharesPosteriorWith` 的反转同一批做）。
回退成本：只改 `portfolio-mgr.rhai` 的装配段与一条门，不动分支脚本、不动节点图。

**批下来的执行分解（3b 是不可切小的单批，理由写在下面）**：
- 3b-1/2 合并 = 主脚本装配段换生产者 + 删 `horizon_decision`/`leg_mult`/`snr`/`prior_for`(若仅闭包在用)
  + 删 `totalScore_*` 四条映射 + 六把 Rust 测试锁翻向 + TS 类型/组件 + 11 语言文案。
  **不可分批的机制**：换生产者与「删掉旧生产者」之间若插一次提交，中间态会是
  「闭包还在产、且某档缺席时闭包补一份」= 我们刚判定为伪装的那种形状；
  反过来先删闭包则面板直接空四行。⇒ 必须同批。
- 3b 之前先落 #25 的整段求值门：但全脚本求值需要 ~92 个入参的夹具，成本远超一次改动；
  替代方案（也是本仓既有风格）= **给新的装配段做片段级黄金门 + 逐字漂移锁**
  （`pm.contains(片段副本.trim())` 那条判据），既钉住「缺席 ⇒ 不产生该键 + data_gaps 点名」，
  又不用养一份 90 键夹具。#25 的全量求值仍作为独立选项保留，不在 3b 里顺手做。

**3b 的其余步骤（不含上面的拍板也可先行）**：把 `portfolio-mgr` 里读 `totalScore_*` 的四条入边、
`leg_mult`/`horizon_leg_weights_json` 的乘数语义、`pm_snr_confidence` 的 √(h/28) 折算、
`horizon_decision` 闭包一并退役；上表右列标「退役」的六个键同时从 TS 类型与两处组件里删；
`portfolio_mgr_tier_params_rhai.rs:122/:189/:204` 三条逐字锁翻成禁词锁，
`portfolio_mgr_horizon_lock_rhai.rs:141` 的 `snrAnchorDays` 锁翻成「不得出现」。

## 四十二、3b 的逐文件执行清单（已把面积测完，下一轮照做，勿再普查）

**已确认的现成事实**（决定顺序，不必重查）：
- TS 契约 `src/types/stock-analysis.ts:249-253` 的 `DecisionsByHorizon` 四个键都是
  `HorizonDecision | null | undefined` ⇒ **「某档缺席」在类型层已经可表达**，
  §四十一 选 A 不需要新造 UI 语义（但我自行裁定里要求的 `data_gaps` 点名仍然必须做 ——
  类型允许缺席 ≠ 缺席可以不解释）。
- `HorizonDecision` 的**必填**键含 `targetPrice: number | null` 与 `stopLoss: number | null`
  （绝对价，不是百分比），而分支节点只出 `stopLossPct`/`takeProfitPct` ⇒ 装配段必须自己用
  `current_price` 把绝对价算回来（闭包原本就是这么写的，`portfolio-mgr.rhai:2953-2954`）。
- `src/lib/agentOutput.ts:402-414` 只做 camel/snake 双兼容透传 ⇒ 它不读逐字段，改形状不影响它；
  真正逐字段渲染的是 `DecisionBanner.tsx:682-696,1524-1527`。

**执行顺序（每步都带「谁会变红」，按此顺序做不会出现半落地）**：

| 步 | 文件 | 动作 | 会变红的既有锁 |
|---|---|---|---|
| 1 | `seed_stock_analysis.rs`（portfolio-mgr 节点） | 加四条入边 `pm-h-* → portfolio-mgr` + 四条 `input_mapping`：`h_ultra_short ← pm-h-ultra-short.result` 等（与 arbiter 同形）；**同时**删 `("totalScore_ultra_short"…)` 四条映射（:4636-4639）与 `("horizon_leg_weights_json", …)`（:4897） | `rhai_registry.rs:800`（键从映射消失即 panic）、`audit-inject-coverage.mjs:495-523` 合成夹具、`portfolio_mgr_tier_params_rhai.rs:165-167`（反向锁本就禁止字面量，安全） |
| 2 | `portfolio-mgr.rhai` | 用装配函数替换 `horizon_decision.call(...)` 四处：读 `h_*` ⇒ 缺键则**该档不出现**且 `data_gaps.push("<档名>分支未产出（节点失败）⇒ 本档无独立结论")`；有键则补算 `targetPrice`/`stopLoss`，删 `confidenceRiskAdjusted`/`posteriorEffective`/`snrAnchorDays`/`weightsSource`/`confLowerBound`/`sharesPosteriorWith` 六键，收 `horizon`/`confidenceMethod`/`exitRule`/`entryGate`/`entryGatePassed`/`legs`/`absentLegs`/`dataGaps` 八个自证键 | `portfolio_mgr_tier_params_rhai.rs:151-153`（`{f}.call("{h}")` 逐档锁）、`:173`（`horizon_decision.call("ultra_short"` 锁）、`:189/:204`（`leg_mult`/`weightsSource` 逐字锁）、`portfolio_mgr_horizon_lock_rhai.rs:141`（`snrAnchorDays`）、`portfolio_mgr_same_source_rhai.rs:126/143`（`sharesPosteriorWith`） |
| 3 | 同一文件 | 删除退役本体：`horizon_decision` 闭包（:2893-2995 段）、`leg_mult`（:2844）、`prior_for` 若仅闭包在用（**先 grep 再删**：主链单档决策也在读 `prior` 时不能删）、`SNR_ANCHOR_DAYS`（:2026）与其调用 | 上表全部 + `defaults_match_the_eight_seed_variables`（若档位函数被牵连） |
| 4 | `evidence_weight.rs` | 退役 `horizon_leg_multipliers`（:412）与其 4 条单测（:1612/1631/1643/1664）；`horizon_branch_specs` 里 `bias`/`weight` 的来源改为**逐档腿集自身**（不再乘乘数） | `check-horizon-weight-parity.mjs` 的 ⑦⑧ 判据（前端乘数表与桥表比对 ⇒ 前端 `ANALYST_TIME_HORIZON_WEIGHT` 同批退役或改读分支表） |
| 5 | `hooks.rs` | 停投 `horizon_leg_weights_json`（:398-413）；`horizon_consts_json` 保留到 3 步确认主链不再用 `days_for/sl_pct_for` 之后再退役（:376-388） | `rhai_registry.rs:860-873` 的两处测试 scope |
| 6 | `src/types/stock-analysis.ts` + `DecisionBanner.tsx:682-696,1524-1527` | 删六个退役键的类型与渲染；新增八个自证键的可选类型；缺席档的渲染分支（不画空 Tab，画「该档本轮无独立结论」） | `src/lib/__tests__/stockAnalysisHorizon.test.ts:82/100-107`、`agentOutput.test.ts:70`、`StockAnalysisConfigPanel.test.tsx:62/67`（白名单含 `weightsSource` 值域） |
| 7 | i18n | 缺席档文案 + 新自证字段的标签，**11 种语言**同批补键（裁定②：必须是本语言资源，不得占位） | `check-i18n-key-parity.mjs` + `check_i18n.py` |
| 8 | 门 b–f + `audit-inject-coverage` 纳入五个节点 + v123 收口 | 门 b：每份子模板必须注入 `horizon`；门 c–f 见 §十一 原表 | — |

**⚠ 三条会静默骗人的地方（做每一步时单独验，不要只跑门）**：
1. 步 2 装配段若写成「缺键就 `#{}` 占位」，等价于把缺席伪装成空结论 —— 必须是**不产生该键**；
   断言方向：注入三路 ⇒ `decisionsByHorizon` 恰好三个键 + `data_gaps` 有一条点名（正向 + 负向各一例）。
2. 步 3 的 `prior_for` 删除前必须 grep 主链其余用法；`sharesPosteriorWith` 的**填充环**（:3036+）
   与它的读取端要同批删，留一半会得到「字段永远为空」的新惰性形状。
3. 步 6 的前端删键必须与后端同批 —— 分两批会让面板读到 `undefined` 并静默画 0
   （本仓 `null` 与 0 同形的那族缺陷，见裁定「结构性缺口不得造成歧义」）。

## 四十三、主脚本从来没有「整段求值」的门 —— 3b 之前必须先补（2026-10-04 实测发现）

**发现方式**：给 3a 的 f3/f12 移植找回归证据时，去 rtw 的五个 `portfolio_mgr_*` 目标里看它们怎么跑，
读到的是 `const PRIMARY_TIER_SRC: &str = r#"…"#` + `HORIZON_CONST_CLOSURE`（`portfolio_mgr_horizon_lock_rhai.rs:18-52`）
——**逐字副本片段**，不是真脚本。⇒ 五目标 + 主 crate 的 `portfolio_mgr_rhai_compiles` 加起来，
覆盖的是「片段行为」与「全文可编译」，**没有任何一条把 `portfolio-mgr.rhai` 整段求值过**。

**先更正本段首稿的一处错判（保留原文，按本仓「写错要在文档里自证」的规矩）**：
我原本在此断言「未注册宿主函数这一族没有门」——**错**。`stock_workflow/rhai_registry.rs` 的 tests
早就有这条判据（`PRODUCTION_SCRIPTS` × 剥注释后解析 `register_fn("…")` 字面量，还带 `band_for_score`
这条「前缀不是判据全部」的反例）。真缺的不是判据，而是**它的扫描面**：本轮新增的五条脚本不在清单里。
⇒ 已把 `PRODUCTION_SCRIPTS` 从 17 扩到 22（四条 `portfolio-mgr-h-*` + `portfolio-mgr-arbiter`）。
教训同族：设计新门前先 grep 既有判据（我对数据源做过这步、对判据没做，于是既差点造重复门、
又把「扫描面漏了」报成「一片空白」）。

**「整段求值」这一格其实也早有门 —— 本段的第二处错判（同批更正）**：
`stock_workflow/rhai_registry.rs:1167` 的 `portfolio_mgr_runs_with_valuation_evidence_without_degrading`
就是「整脚本真执行」门（`run_portfolio_mgr` 按 `code_executor` 同一规则造 scope，
断言三层：没落 catch 兜底 / `decisionsByHorizon` 四档齐备且 `weightsSource=="table"` /
估值降权只在真被下调的档留痕，末尾还带「裸引用未定义名字必红」的负控）。
它自己的文档表格里就写着「`portfolio_mgr_*_rhai.rs` 各门只跑抽出片段」——我读到那行就据此下了
「整格空白」的结论，**没去搜提出这行的人补上的那道门**。
⇒ 教训（比 4j 更尖锐的一格）：**「空白」类断言的检索面不能按「测试通常放在哪」预设**；
本仓的门同时存在于 `crates/*/tests/`、`src/commands/**/seed_consistency_tests.rs`
**和 `src/commands/stock_workflow/rhai_registry.rs` 的 `#[cfg(test)] mod tests`**（第三个最容易漏，
因为它的名字不像门）。
⇒ 于是 #25 的剩余内容从「新建门」降级为「**随 3b 翻这道既有门的 fixture 与断言**」：
`horizon_leg_weights_json` 从 fixture 里撤、② 的 `weightsSource=="table"` 断言改成
「装配段的四行来自四路分支输出」、③ 的 `weightAdjustments` 两条改成恒空且反向锁
`data_gaps` 里不得出现乘数降权那类串。
- 3a 刚把两处阶梯改成 `pm_leg_signal` —— 正是新增了一个跨语言调用点；
- 3b 要删 `horizon_decision`/`leg_mult`/`pm_snr_confidence` 并在主脚本里新增「装配四路分支输出」，
  改动面比 3a 大一个数量级。⇒ 在动手术前先有一条「整段跑通 + 关键键有 golden」的门，
  否则 3b 的红/绿无法区分「我改错了」与「本来就没测到」。

**门的设计（与 §三十五 那条分支门同构，复用同两个生产件，不新写第三份）**：
1. **scope 的唯一来源 = 种子**：解析 `include_str!("../portfolio-mgr.rhai")` 所在节点的
   `input_mapping` 键集合（直接复用 `seed_consistency_tests.rs` 的模块级 `code_only()` +
   `mapping_keys()`；把 `mapping_keys` 的 `required_key` 传 `totalScore`、`min_keys` 传 40 ——
   主链节点现有 52 个 target，低于下限就是区块截断）。
   ⚠ **写到这里就被自己纠正了一次**：主链节点的映射不是 `input_mapping: [ … ]` 数组字面量，而是
   `input_mapping: { let mut m: Vec<(&str, &str)> = vec![ … ]; m.extend(PORTFOLIO_MGR_TUNABLE_PARAMS
   .iter().map(|n| (*n, *n))); m.into_iter()… }` 的**块表达式**（`seed_stock_analysis.rs:4617-4645`）
   ⇒ §三十九 那套 `mapping_keys` 只认数组形态，直接复用会 `unwrap` panic 或**静默少收 40 个可调参数名**。
   先扩解析器：认 `input_mapping: {` ⇒ 取 `vec![` 到匹配的 `]` ⇒ 再并上 `m.extend(<常量>…)` 指向的
   `PORTFOLIO_MGR_TUNABLE_PARAMS` 清单（该常量本身有 `declaredCount` 自证先例可抄），
   并断言「键集合大小 == 手写对数 + 常量声明数」两路相加对得上，缺一即红。
   ⇒ 这样「脚本读到没映射的名字」在生产语义下必红（本门首跑就是一次真实验，见下）。
2. **引擎与沙箱 = 生产同一份**：`rhai_registry::build_stock_rhai_engine(RhaiSandboxLimits::PORTFOLIO)`，
   值转换 = `rhai_eval::value_to_dynamic`（已为复用提公开）。
3. **求值到出参**：断言返回 map 里至少含 `action`（六档词表）、`decisionsByHorizon` 的四档键、
   `confidence` ∈ [0,100]、`reasoning` 非空且含 `decision_degraded` 哨兵的反例不触发。
4. **f3/f12 的 golden 就近钉在这里**（而不是再抄一份片段）：两组输入各跑一次 ——
   催化剂 `L2业绩拐点级` + MACD 零上多头 + RSI 60 ⇒ 期望与搬运前实测一致；
   把 `macd_dif` 改成 unit ⇒ f12 权重归零（缺席冒充 0 的伪装方向）。
5. **负控**：从键集合里剥掉一个主脚本真读的名字（`catalyst_level`）⇒ 必须**运行失败且点名**，
   证明本门测的确实是节点 scope。

**顺带闭合的旧账**：`horizon_leg_weights_json` 的探针断言在 `rhai_registry.rs:800`（键从映射消失即 panic），
那是「测试自己声明的映射」而非真节点 ⇒ 本门上线后它可退役（避免两条不同源的 scope 判据并存，同 §三十九 那条教训）。

## 四十四、3b-α 落地读数 + 两条「注释致盲」的假红（2026-10-04）

**这一片改的是什么**：`decisionsByHorizon` 的生产者从主链闭包换成四路分支节点的**装配**。
测试侧随之翻锁，翻出两条**同族**缺陷，都值得单独记：

### ① 禁词门按裸文本判定 ⇒ 被自己的退役说明注释打死

首版把「退役形态不得回归」写成 `!pm.contains("leg_mult")` 一类的裸文本判定，实跑必红 ——
主链脚本里留着**六段**退役说明注释（`:861`、`:2054`、`:2816`、`:2829`、`:2870`、`:2875`）逐字
提到 `leg_mult` / `horizon_decision` / `pm_snr_confidence` / `sharesPosteriorWith`。这些注释是给
后来者解释「这里为什么不再有乘数」的，删不得；判据查的应该是**代码**。

处置（两种粒度，各按判据本意选）：
- **全文级**（退役字段 + 退役算法名）→ 搬进主 crate 的
  `seed_consistency_tests::main_script_keeps_retired_per_tier_algebra_out_of_code`，
  复用同文件的模块级 `code_only()`（四份分支脚本的禁词门共用**同一份**剥离实现，铁律 12）。
  该门带三条自证：剥离后仍含四段真实代码（扫描面没被吃掉）、禁词写成注释**不**命中（证明
  查的不是全文）、把禁词写进代码**必**命中（证明不是恒真断言）。
- **区段级**（装配区不得**调用**逐档函数）→ 留在 rt-workflow，但禁词一律带调用标点
  （`leg_mult(` / `sl_pct_for.call(`），判据本来说的就是「不得调用」，带括号既更准也免疫注释；
  另附变异点负控（往装配区插一行退役调用，同一套锚点必须抓到）。
- rt-workflow 拿不到 `code_only()`（它在主 crate，方向不能反），也不该复制第二份剥离器
  ⇒ 全文级那条**只能**住在主 crate。这条边界本身值得记住：**门的存放位置由它能用哪些
  基础设施决定，不是由「测试习惯放哪」决定**（对照 §四十三 那条「空白类断言的检索面」）。

### ② 另一条同类假红：逐档 stopSource 的副本锁

`portfolio_mgr_tier_params_rhai.rs::stop_and_take_profit_are_volatility_derived_with_labeled_fallback`
原本断「每档与主档**都**输出 `stopSource`」，逐档那一半的对象是闭包 ⇒ 换心脏后必红。
翻向为：主档那份仍在（正锁）＋ 主链**不得**再逐档写（禁词 + 变异点负控）＋ 四份分支脚本
**必须**各自输出 `stopSource`（否则「翻向」等于把判据删了，两头都没锁）。

### 本轮实测读数（批次 bcsp7rfg7，2026-10-04 07:56–08:18）

| 门 | 读数 |
|---|---|
| `cargo fmt --all --check` | 绿（FMT_OK） |
| `-p axagent --lib` | **528 通过 / 2 红**，`LIB_EXIT=101` |
| `-p axagent-rt-workflow` | **1 红**（上面那条 stopSource），其余 8 个测试二进制全绿，`RTW_EXIT=101` |
| `-p axagent-rt-workflow` clippy（`--all-targets --all-features -D warnings`） | `CLP_RTW_EXIT=0` |

两条 lib 红的定性：
1. `rhai_registry::every_injected_var_read_bare_has_a_mapping:1225` ——
   报的是**负控**：「摘掉 `horizon_prior_json` 的映射后判据没报出来，实际 `[]`」。
   真因不是判据坏，是**负控的样本失效了**：3b 之后主链不再裸读该名字（逐档先验改由四个分支
   节点自己映射），所以摘掉映射不再构成「裸读无映射」⇒ 正断言（`bad.is_empty()`）依旧是绿的，
   只有那条「必须能报出 horizon_prior_json」的负控在指一个已经不存在的事故形态。
   ⇒ 判据翻向时**连负控样本一起换**，换一个仍满足三条件的名字（hooks 注入 + 主脚本裸读 +
   本节点同名映射且在 seed 里唯一）。已换 `horizon_consts_json`。
   顺带发现主节点仍映射 `horizon_prior_json` 而脚本零消费 ⇒ 按 §三十九 的同一判据（惰性接线
   比未接更坏）撤掉该映射，权威源与四个分支节点的映射不动。
2. `serenity::serenity_extract_tests::extract_recovers_candidates_from_strict_mode_report`
   —— **HEAD 自带**（`serenity.rs` 与 `kit/utils.rs` 均无本地改动；单跑隔离复现
   `serenity.rs:2333` panic）。与本片无关，登记为任务 #26；产品含义按该测试自己的注释：
   趋势智选可能又在产出 0 候选。**不在 3b 中途动它**（混合源读数会毁掉本片的证据链）。

### ③ 换心脏时被丢掉、本轮补回的一条顶层声明

旧闭包段有一条「某档技术腿退化为日线 ⇒ 往顶层 `data_gaps` 推一条」的声明（Phase F 产物），
装配段首版**没有**搬它 ⇒ 逐档退化只剩档内 `scoreSource`，而「数据缺口 N 项」横幅读的是顶层
条数 ⇒ 结构性缺席在呈现层重新变成歧义。已按批准口径补回（装配循环内，`scoreSource == "daily_fallback"`
⇒ 点名该档），并在 `decisions_by_horizon_assembles_and_never_recomputes` 里钉成正反两条锁
（正：装配区必须有这条 push；反：`gap_note` 必须在装配段之后求值，否则文案计数与数组长度不符）。

## 四十五、3b-β 的面积（乘数通道整体删除，前后端同批）

普查已完成，**照此执行即可，勿再普查**：

| 面 | 位置 | 动作 |
|---|---|---|
| 权威表 | `evidence_weight.rs` 的 `horizon_leg_multipliers()` + 四个测试 | 删函数；「技术腿随周期衰减 / 估值腿随周期增长」搬到 `get_horizon_base_weights` 上（判据换对象要搬家，不能随表一起删） |
| 注入 | `hooks.rs` 的 `horizon_leg_weights_json` 恒注入段 | 删段 |
| 脚本 | `portfolio-mgr.rhai` 的 `weight_adjustments` 声明 + 两处输出（成功路径 + 异常兜底） | 删 |
| 种子 | 主节点 `("horizon_leg_weights_json", …)` 映射与四处版本注释 | 映射删；v99/v112/v114 是**史料**，只在段末追加退役说明，不改写历史 |
| 门 | `rhai_registry` 整脚本门 ③ | 从「必须恒空」翻成「键不得存在」 |
| TS 类型 | `src/types/stock-analysis.ts` 的 `WeightAdjustment` + `StockDecision.weightAdjustments` + `HorizonDecision` 的 `weightsSource`/`snrAnchorDays`/`sharesPosteriorWith`/`confLowerBound`/`posteriorEffective`/`oddsSource`/`confidenceRiskAdjusted` | 删（逐档那七个是**幽灵声明**：分支脚本从不产出） |
| 解析/透传 | `agentOutput.ts` 的 `isWeightAdjustment` + 解析 + 输出字段；`stockAnalysisStore.ts` 重跑路径同款 | 删 |
| 渲染 | `DecisionBanner.tsx` 三块脚注（confLowerBound / sharesPosteriorWith / weightAdjustments）+ 因此不再使用的 `HORIZON_CAMEL_TO_SNAKE` 导入 | 删 |
| 自述 | `DecisionTrustNotice.tsx` 头注释、`stockAnalysisHorizon.test.ts` 的 `horizonSuffix` 理由段 | 改（判据主语换成现役消费方） |
| i18n | `confLowerBoundHint` / `horizonSharesPosteriorHint` / `horizonLegDownweight` × 11 语言 | 删（33 行；键序与 LF 不动，逐文件 JSON 解析自证） |
| Node 门 | `check-horizon-weight-parity.mjs` 表格与 ⑦/⑧ 的「乘数命中不到腿」措辞、报错文案 | 改文案（判据本身仍然有效：属主唯一 + 集合闭合） |
| vitest | `DecisionBanner.test.tsx` / `DecisionTrustNotice.test.tsx` / `agentOutput.test.ts` / `stockAnalysisHorizon.test.ts` / `StockAnalysisConfigPanel.test.tsx` 白名单 | 用例对象已不存在 ⇒ 翻成现役判据（逐档腿清单透传、缺档不出 Tab），不是简单删 |

**为什么 3b-β 不可拆成两次提交**：通道的一端留在树上就会出现「后端仍产出、前端无人声明」
或反之的形状 —— 两者都是本仓禁的「DTO 与消费方不同批」（禁区 13）。

## 四十六、3b-β 落地读数：整脚本门第一次跑就抓住自己人的断裂（2026-10-04）

**做了什么**：按 §四十五 清单删掉逐档乘数通道（前后端同批），并把两条会被丢弃的旧判据**搬家**：

| 旧判据 | 新产地 |
|---|---|
| 「逐档 `stopSource` 必须由主链闭包输出」（对象已退役） | `portfolio_mgr_tier_params_rhai::stop_and_take_profit_…` 翻成三段：主档必须有 + 主链不得逐档写（带调用标点禁词 + 装配区变异点负控）+ **四份分支脚本必须各自输出** |
| 「乘数表里技术腿衰减 / 估值腿增长」+「元约束腿恒 1.0」 | `evidence_weight::technical_bias_decays_and_valuation_bias_grows_with_horizon`（搬到 `get_horizon_base_weights` 上）；元约束那半条**不重复实现**，指向既有的 `DECISION_LEG_ANALYST` 属主 `None` + 闭合测试 + Node 门判据 ⑧ |
| `weightAdjustments` 「必须恒空」 | `rhai_registry` 整脚本门 ③ 翻成「**键不得存在**」 |
| 负控样本 `horizon_prior_json` | 换 `horizon_consts_json`（三条件齐备且 seed 内唯一），见 §四十四 ①-1 |

**批次 G 读数**（batch b5mgem8nn，08:56–09:06）：`RTW_EXIT=0`、`AE_EXIT=0`、
`CLP_EXIT=0`（axagent + rt-workflow + analysis-engine 三 crate `--all-targets --all-features -D warnings`），
`LIB_EXIT=101` = **529 通过 / 1 红**，且那条红就是 §四十四 定性的 HEAD 自带智选红
（`serenity.rs` 与 `kit/utils.rs` 均无本地改动 ⇒ 与本片无关，任务 #26）。

**本片最有价值的一条实证**：β 删声明后**漏删一处输出引用**，整脚本门首跑即以
`Variable not found: weight_adjustments`（line 2977）把主脚本打回 catch 兜底 ——
这正是 §四十三 那条「编译门查不出未注册变量、片段门查不出整段断裂」的原始动机。
`cargo check` / `clippy` / `portfolio-mgr.rhai` 语法门在当时那状态下**全绿**。

**批次 G 之后的两处改动**（读数属批次 I，见本节末）：
1. **分层棘轮红**：`check-layer-discipline.mjs` 报 `commands-no-sibling-call`
   `seed_consistency_tests.rs 8 > 基线 2`。真因是**我**四处门各自写全路径
   `crate::commands::stock_workflow::rhai_registry::{build_stock_rhai_engine, RhaiSandboxLimits}` ——
   该棘轮按「规则 + 文件」计**行**数，同一个依赖被记成 8 个坑。
   ⇒ 修法不是抬基线（抬基线=承认挖深），而是模块级 `use` 单点导入 + 四处调用改短名：
   8 → 1（低于基线，棘轮提示下调）。**通则：棘轮按行计数时，重复的全路径不是「风格问题」而是「负债计数」**，
   单点导入既是风格也是债务下调；反过来，把一次依赖拆成多处全路径写会伪装成「挖深」。
2. **既有红（非本片造成，登记为任务 #28）**：`check-single-source-facts.mjs --ci` 硬拦 1 处 ——
   `commands/stock_workflow/core.rs:802` 引用 `engine/mod.rs:3144`，该行现为 `continue;`。
   两侧文件均无本地改动 ⇒ HEAD 自带；它会**中断 ci-check 的后续步骤**，所以本地跑全量 node 门前必须先修。

**前端侧读数**（同批）：`tsc --noEmit` 干净；`oxlint --max-warnings=0` 0 warning 0 error（1283 文件）；
`npm run test:run` **190 文件 / 2118 测试全绿**；`dprint check` 无差异；行尾检查两条规则全 PASS；
`check-i18n-key-parity.mjs`（11 语言 key 集合一致）、`check_i18n.py`、
`check-i18n-placeholder-usage.mjs`、`check-horizon-weight-parity.mjs`（含 `--selftest`）、
`audit-inject-coverage.mjs`（①③ 空、② 只剩 `screening_source` 一条 HEAD 既有悬空）全部通过。

## 四十七、批次 I 读数 + 三条 HEAD 自带红的清单（2026-10-04 收口）

**批次 I**（batch bdm21uaz9，09:14–09:32，验证 §四十六 之后那两处改动）：

| 门 | 读数 |
|---|---|
| `-p axagent --all-targets --all-features -- -D warnings`（含 test target） | `CLP_AX_EXIT=0` |
| `-p axagent --lib` | `LIB_EXIT=101` = **529 通过 / 1 红**，唯一红 = 下表 ① |

⇒ 3b-α/β 的代码与门**全部落定**：Rust 侧除三条与本片无关的既有红外，无一红。

### HEAD 自带红清单（逐条已证「非本片造成」，未批不动）

| # | 红 | 证据 | 为什么不修 |
|---|---|---|---|
| ① | `serenity::serenity_extract_tests::extract_recovers_candidates_from_strict_mode_report`（`serenity.rs:2333` panic「report 解包后应回收候选」） | `serenity.rs` 与 `kit/utils.rs` 零本地改动；隔离单跑复现（任务 #26） | 产品含义是**趋势智选可能又在产出 0 候选**，属独立归因轮；在 3b 中途动它会毁掉本片的源状态 |
| ② | `check-single-source-facts.mjs --ci` 硬拦 1 处：`core.rs:802` 引用 `engine/mod.rs:3144`，该行现为 `continue;` | 两侧文件零本地改动（任务 #28） | 修法唯一但需先确认那个事件由**哪个** `StepProgressEvent` 构造点发出（同文件有 7 处），挑同号行 = 把一条门换成另一条假引用 |
| ③ | `i18n-scan.mjs --strict` 报 2 条「new」：`format.ts:102/103` 的「亿 / 万」硬编码 | `format.ts` 与 `.i18n-allowlist.json` 零本地改动，该文件最后由 **HEAD 提交 d12cac6ec** 改动（任务 #29） | 二选一是产品决定（补 11 语言键 vs 进豁免清单），不在本片范围 |

⚠ ②③ 都会**中断 `ci-check.mjs` 的 fail-fast 链**（②在中途即停）。⇒ 本片验证期间把 ci-check 的其余步骤**逐条单跑**补齐了：
`check-reco-horizon-parity` / `audit-inject-coverage --strict` / `check-id-validation` /
`check-errorcode-alignment` / `check-sql-dialect` / `check-i18n-untranslated` /
`check-rust-raw-map-err` / `check-horizon-weight-parity`（+selftest）/ `dprint check --incremental=false`
/ `tsc --noEmit` / `vitest run`（190 文件 2118 测试）/ 行尾两条规则 —— **全部通过**。

### 现在欠的只有两件人做的事

1. **重启应用重播种**（库内 `stock-analysis` 仍是 **122**，模板已 v123）—— 由用户执行；
   播种后按 §三十七 的路径表逐节点核对 `pm-h-*` 四路是否真的出数（未播种期间四档按缺席呈现是**预期**）。
2. **手工提交** —— 本片按约定不代提交。⚠ 提交须包含 6 个新增未跟踪文件：
   `crates/analysis-engine/src/leg_signal.rs` + 四份 `src/commands/portfolio-mgr-h-*.rhai`
   + `src/commands/portfolio-mgr-arbiter.rhai`（漏任何一个 = 门与脚本不同代）。
   建议按 §四十二 的批次切分（α/β 可各自成一条，但 β **不可只提前端**）。

## 四十八、000710 实跑暴露的「同屏两个超短线」矛盾 + 三条新裁定（2026-10-04）

### 实测事实（`stock_analyses.id = a817a0e6-1282-462d-a08d-a05e718f3f54`，live，2026-10-04 10:11，template_version=123）

播种与接线**都已生效**（`workflow_templates(id=stock-analysis).version=123`，四路 `pm-h-*` 与
`pm-arbiter` 节点在 `nodes` 里、`e-pm-h-*-portfolio-mgr` 边在 `edges` 里），四档也**全部产出**：

| 层 | action | confidence | positionPct | odds | entryGate / 判定 | exitRule | confidenceMethod | 腿 / 缺腿 | priorSource / 样本 |
|---|---|---|---|---|---|---|---|---|---|
| 主链（日线口径） | **减持** | 38.3（风险调整后 44.1） | 0.0 | — | — | — | — | — | — |
| 超短分支 | **买入** | 87.8 | 0.0 | 0.0 | `breadth_required` / **通过** | `time_stop` | `edge_no_time_scaling` | 2 / 1 | pooled / **0** |
| 短线分支 | 持有 | 77.8 | 0.0 | 0.0 | `trend_and_flow_same_sign` / 未通过 | `time_stop+fixed_stop` | `dual_confirmation` | 3 / 2 | pooled / 0 |
| 中线分支 | 持有 | 56.8 | 0.0 | 0.0 | `inside_sigma_band` / 未通过 | `k_sigma_band` | `sigma_band_position` | 2 / 4 | pooled / 0 |
| 长线分支 | 减持 | 33.1 | 0.0 | 0.0 | `positive_margin_of_safety` / `unjudged` | `target_and_falsified` | `margin_of_safety` | 3 / 3 | pooled / 0 |

主链 `timeHorizon = ultra_short`（`decision_horizon_source = formula`），而**同一档**的分支结论是买入
⇒ 卡片顶部「超短线 · 减持」与四档摘要「超短线: 买入」并存。

### 三条裁定（用户 2026-10-04 拍板）

- **Q1 = C**：**定档改由四档结论选出**，主档 action 取所选档的结论；主链日线口径退为归因/证据列。
- **Q2 = B**：**保留方向动作**，但 UI 必须并列声明「无可执行计划（止损/止盈不可得）」——
  不把「买入 + 仓位 0%」悄悄降级成观望。
- **Q3 = A**：**逐档证据全量上屏**（在场腿/缺腿、门与判定依据、出场口径、置信方法、先验来源与样本数）。

### C 的设计约束（写代码前必须先对齐的四件事）

1. **顺序必须重排**。当前主链是「先定档（`:1937`）→ 凯利与风险预算（`:2040-2066`）→ action 阶梯与
   风险否决（`:2392-2403`）→ 各类降级/翻转（`:2443-2563`）」，而四档输出在 `:2835` 之后才装配。
   选档要成为定档的输入 ⇒ **装配段必须前移到定档之前**（或定档与主 action 的计算整体后移）。
   ⚠ `time_horizon` 目前被下游消费：`risk_budget_position(time_horizon)`、`odds_for.call(...)`、
   `days_for`、`horizon_price_map`、主档 `stopSource` ⇒ 换档后这些**必须用新档重算**，
   否则得到「档位是分支选的、仓位与价位还是主链旧档算的」的第二次同屏矛盾。
2. **门槛只许降级，不许升级**。选档之后仍要叠加主链的 `pm_risk_veto` / 技术否决 / 模拟盘闸门 /
   组合仓位上限 / 风险预算 —— 与既有裁定「反思建议只降不升」同族：叠加层只能把 action 往保守方向改、
   把仓位往下压，**不得**把观望改成买入。
3. **`decision_horizon_source` 要新增取值**（现恒 `formula`）。建议 `branch_pick`（值域权威在
   `harness`，不是脚本内字符串），并同步落库列注释与读侧；否则「谁定的档」这个问题又变成不可答。
4. **历史代际**：`decision_action` / `decision_time_horizon` 的语义换代 ⇒ 老行是旧口径。
   读侧与统计（IC、反思、错题本）必须按 `template_version` 筛样（列已有），
   不得把两代混在一个样本池里算命中率。反思的「默认复盘那一档」随新定档机制走（一次反思一档不变）。

### 阶段切分（逐阶段推进，每阶段自带门禁）

| 阶段 | 内容 | 门禁 |
|---|---|---|
| **1（呈现层，零决策语义变更）** | Q2-B + Q3-A：每档面板补「在场腿/缺腿 + 门与 gateBasis + exitRule + confidenceMethod + priorSource/priorSamples」；`odds==0 \|\| positionPct==0` 时并列声明「方向成立但无可执行计划」；新增 i18n 键 × 11 语言 | `tsc` / `oxlint` / `vitest`（新增两条正反用例：全证据档渲染、无可执行计划档必须成句）/ `check-i18n-key-parity` / `check_i18n.py` / `check-i18n-placeholder-usage` / `i18n-scan --strict` |
| **2（选档与顺序重排）** | 装配段前移；新增 `pick_primary_tier()`（判据：先筛可执行档 `positionPct>0`，取 `confidence` 最大；**全不可执行 ⇒ 仍取 confidence 最大档**，由阶段 1 的「无可执行计划」声明兜住）；`time_horizon` / `horizon_source` 改由它产出 | 整脚本门新增断言：主档 `action == decisionsByHorizon[所选档].action`（叠加门槛前）+ `horizonSource == "branch_pick"`；`portfolio_mgr_horizon_lock_rhai` 的「LLM 不得影响定档」两条锁继续成立 |
| **3（门槛叠加与仓位/价位重算）** | 用新档的 odds/days 重算 `position_pct`、`horizon_price_map`、主档 `stopSource`；veto/风险预算只降不升 | 锁「升级方向」的反例（合成坏样本必须红）+ `risk_budget` 与 `fallback_kelly_x_mult` 两分支来源标注仍可反解 |
| **4（落库与读侧代际）** | `decision_horizon_source` 值域扩 `branch_pick`（harness 权威）+ 实体列注释；IC/反思/错题本按 `template_version` 筛样 | 既有 `check-domain-single-source` / 值域闭合门 + 新锁「值域含 branch_pick 且脚本产出同一值」 |
| **5（重放核对）** | 重启重播种后对 000710 与另一只票各重跑一次，逐条核对：主档与所选档一致、四档证据上屏、无可执行计划成句 | 人工核对清单（本文件记读数） |

⚠ 阶段 2/3 会**改变决策数值**（action/仓位/定档），按本仓规矩：一旦落地即需重放核对，
且历史行的对比要分代 —— 不是「修个显示」级别的改动。

### 阶段 1 已落地（2026-10-04，纯前端，不需重播种）

**改了什么**：`DecisionBanner` 的逐档详情区新增四组呈现 + 一条缺席声明：
① 入场门（`entryGate` token + 三态：**通过 / 未通过 / 未判定**，`gateBasis=="unjudged"` 走第三态
——把「算不出」读成「未通过」也是伪装成结论）；② 出场口径 `exitRule` + 置信口径 `confidenceMethod`
+ 赔率 `odds` + 证据完整度 `evidenceScale`；③ 先验 `priorSource` + `priorSamples`，**样本为 0 时单列成句**
（「本档无历史样本，先验借自全档合并基准」）；④ 在场腿（factor/role/weight/signal）与缺腿分列 + 档内 `dataGaps`；
⑤ Q2-B：方向族（买/卖族）且 `positionPct<=0 && odds<=0` ⇒ 并列声明「方向成立，但本档无可执行计划」，
**观望/持有档不声明**（0 仓位对它们是常态，声明它就是把常态说成缺陷）。

**刻意不做的一件事**：`entryGate` / `exitRule` / `confidenceMethod` 的**值**按 Rust token 原样显示
（mono 字体），不翻成中文。翻一份就得在前端维护第二套值域，而值域权威是
`evidence_weight::horizon_branch_specs` —— 与「四档显示名不得手抄天数」同一条判据。
若以后要中文标签，必须先把值域导出成 DTO 再渲染，不在这里抄。

**读数**：`tsc --noEmit` 干净；`oxlint --max-warnings=0` 0/0；`dprint fmt` 无差异；
`vitest run` **190 文件 / 2120 测试全绿**（新增 2 条：逐档证据上屏、无可执行计划正反两半）；
i18n 五门（key 对齐 / 未翻译 / 占位符 / `check_i18n.py --strict` / 硬编码扫描）与行尾检查全绿；
其余 14 条 node 门（分层棘轮 / 本体 / 能力域 / input_mapping / 荐股口径 / 权重一致性 /
注入覆盖 strict / ID 边界 / 错误码 / 裸 map_err 等）全绿。
`i18n-scan --strict` 仍报 2 条 —— 逐条核过**仍是** `format.ts` 那对既有红（任务 #29），与本片无关。

**新增 i18n 键 14 个 × 11 语言**（插在 `stockAnalysis.horizonScoreFallbackHint` 之后，
落盘前逐文件断言 round-trip 等价 ⇒ 除新增键外零漂移）：
`horizonNoExecutablePlan` / `horizonEntryGateLabel` / `horizonGatePassed` / `horizonGateUnjudged` /
`horizonGateFailed` / `horizonExitRuleLabel` / `horizonConfidenceMethodLabel` / `horizonOddsLabel` /
`horizonEvidenceScaleLabel` / `horizonPriorLabel`（`{{source}}`+`{{n}}`）/ `horizonPriorNoSamples` /
`horizonLegsLabel` / `horizonAbsentLegsLabel` / `horizonTierGapsLabel`（三个 `{{count}}`）。

## 四十九、阶段 2+3 落地，与一条被生产读数证伪的自我假设（2026-10-04）

### 先记一件错事：我上一轮的根因判断是**错的**

四档 `stopLossPct` / `takeProfitPct` / `odds` / `positionPct` 全为 0 这件事，我上一轮给出的
根因是「`kline_bars` 注入末端是 JSON 字符串，分支脚本没有 `safe_parse` ⇒ `bars_ok` 恒假」。
本轮动手前先去现网取了一次**实际注入值**（`workflow_executions.node_executions[]` 里
`pm-h-*` 节点 `output.input_params.kline_bars`）：

```
kj_type = array   n = 120
first_bar = {"close":10.35,"date":"2026-04-09",...}   last_bar = {"close":10.87,"date":"2026-09-30",...}
```

⇒ 假设被**整条证伪**：注入的确实是数组，且是能算出 σ 的数组。若照那条假设去补 `safe_parse`，
改完仍然是全 0，而且会留下一段「已经修过了」的错误记载。判据：**修法落笔前，根因必须有一条
现网读数值支撑**，不接受「代码看起来会这样」。

### 真根因：同一份 JSON 数字，两个载体两种类型

`src-tauri/crates/rt-workflow/src/work_engine/executors/code_executor.rs` 的注入分两条路：

| 位置 | 走哪段 | 结果类型 |
|---|---|---|
| **顶层**标量（`input_mapping` 直接给到一个数字） | `code_executor.rs` 的 `Some(Value::Number(n))` 分支，先 `as_f64` | Rhai `f64` |
| **嵌在 map / array 里**的数字 | 兜底分支 `json_value_to_dynamic`（`crates/harness/src/rhai_engine.rs`，先 `as_i64` 保整数） | Rhai **`i64`** |

分支表 `horizon_branch_specs()` 的 `days`（2/5/28/90）与 `volLookbackDays`（20）都是**整数**，
而它们恰好嵌在 map 里 ⇒ 脚本拿到的是 `"i64"`。四份分支脚本原来的判据是：

```
let band_days = if spec_ok && type_of(branch_json["days"]) == "f64" { ... } else { 0 };
```

只认 `"f64"` ⇒ `band_days` / `band_lookback` 恒 0 ⇒ `pm_vol_move_pct` / `pm_band_z` **整段跳过**
⇒ 价带、赔率、凯利、仓位全 0；更糟的是 `stopSource` 的写法是
`if exit_move > 0.0 { "vol_band" } else { "fallback_pct" }` ⇒ 它把「判据把自己拒了」标成
「走了固定百分比兜底」，而那组百分比**从未参与计算** —— 这就是「缺席被伪装成一种口径」。

一句话病灶：**`type_of(x) == "f64"` 不是「它是不是数」的判据**，它只是「它恰好不是整数」的判据。
仓内已有正确先例（`src-tauri/src/commands/analyst-brief.rhai`、`candidate-assembler.rhai`、
`data-quality.rhai:61` 的口径注），四份分支脚本没照抄。

普查到的同形态面积（**本轮只修 horizon 链**，其余按未批不扩范围处理）：
19 个 `.rhai` 文件含 `== "f64"`，共 100+ 处；其中「两型都收」（`|| type_of(x) == "i64"`）的
是安全形态，「只认 f64 且 else 退成 0/缺席」的才是同一缺陷。已逐一列出，待单独一轮处理。

### 改了什么（三处，都在 v124 一片里）

1. **四份分支脚本**新增 `spec_int` 归一闭包（收 `f64` / `i64` / `int`，非数一律 0），
   `band_days` / `band_lookback` 改走它。用 `.call(...)` 调用（见下方门禁事故）。
2. **主链选档换代**（裁定 Q1=C）：`branch_tiers` 行清单只有一份，选档段与装配段共用；
   判据「可执行优先（`positionPct>0`）→ 置信最大」，全不可执行仍取方向最强档；
   `horizon_source` 扩两值 `branch_pick` / `formula_no_branch`（权威表
   `crates/harness/src/holding_period.rs` 的 `HORIZON_SOURCES`）；主 action 的分支覆盖点
   在 `pm_risk_veto` **之前**（锁死顺序：风控只准往保守方向改）。阶段 3「用新档重算」由
   **定档早于凯利/价位段**这一条顺序实现 —— `sl_pct_for` / `stop_source_for` /
   `horizon_position_mult` 都以 `time_horizon` 为入参，改档即改数。
3. **门补齐**（这条最要紧，因为它一直在绿）：分支脚本运行门的夹具原来是**单调等差**价格序列，
   日收益率标准差 1e-5 量级 ⇒ 四舍五入两位后价带本就为 0，「齐备态能算出价带」这条断言
   即使写了也没有区分力。现改成有离散度的锯齿序列，并新增三条断言：`stopLossPct>0`、
   `takeProfitPct>0`、`odds>0` + `stopSource != "fallback_pct"`；再加一条**前提自证** ——
   注入的 `days` / `volLookbackDays` 必须是 i64（哪天产出端改成浮点，这条红，提醒换样本而不是删断言）。

### 一条门禁把自己人的新代码抓了两次

`rhai_registry` 的两条门各自抓到一处本轮新写的缺陷，都不是靠人眼：

- `every_production_script_calls_closures_via_method_call` 报
  `portfolio-mgr: branch_row_ok` —— 我把闭包写成 `branch_row_ok(t.row)` 按名调用。Rhai 的按名
  调用**不查作用域里的 FnPtr**，运行期 `Function not found` ⇒ 整脚本落 catch ⇒ 四档装配全丢。
  同批的 `portfolio_mgr_runs_with_valuation_evidence_without_degrading` 报出同一处的
  `ErrorFunctionNotFound @ line 1965`，两条门互为正反向证据。
- 这条判据在 2026-10-04 之前只是「经验」，现在是**有生产事故形态背书的一次实测**。

### 仍存在的两数风险（未批，不动）

换代后主档指针指向某一档分支，但**主档的数值**（`position_pct` 走风险预算、
`stopLossPct` 走日线 σ 阶梯）仍由主链公式产出，而面板的逐档 Tab 显示该档分支自己的
`positionPct` / `stopLossPct`。两套算法对同一档名给两个数 ⇒ 与用户本轮抱怨的
「同屏两个超短线」是同一族缺陷，只是这次差在数值层而不是档名层。价带恒 0 时看不出来
（两边都是 0），修好之后必然显形。**要么主档数值直接取所选档分支的输出、主链退为归因与风控上限，
要么明确「主档是聚合摘要、逐档是明细」并禁止两处并列** —— 这是产品级选择，已提给用户。

## 五十、①「主档数值直接取所选档」的设计与退役面（2026-10-04 批准）

**依据（两条既有裁定合推出唯一解，不是新偏好）**：裁定 C「主档由四档结论选出」+ R-11
「不得一个算法 + 四套参数」⇒ 档既然是分支选的，该档的数也必须是分支算的。方案 ②（并列展示 +
标注「两处口径不同」）被排除的理由与当初判废 `sharesPosteriorWith` 注脚同一条：**给伪装贴标签
不等于去掉伪装**（见 §四十二 与 `seed_consistency_tests` 的门 ③ 反向锁）。

**四处改动（同一批，不拆开）**：

| # | 位置 | 改法 |
|---|---|---|
| 1 | `portfolio-mgr.rhai` 仓位段 | `branch_picked` 时主档仓位取 `picked_row["positionPct"]`，主链风险预算退为**上限**（`min`，只降不升）；`positionSource` 加第三值，三值齐备可反解「谁被用上」 |
| 2 | 同文件主档出场字段 | `stopLossPct` / `takeProfitPct` / `odds` / `expectedHoldingDays` / `targetPrice` / `stopLoss` 取所选档行；`formula_no_branch` 那代仍走主链公式（兜底可以，但必须自报身份） |
| 3 | 同文件 `horizonPriceMap` | **同类项**：它现在还是「主链用同一公式对四档各算一遍」的第二套逐档价位 —— 与刚被判废的形状一模一样，且与逐档 Tab 并列上屏（`src/components/stock-analysis/DecisionBanner.tsx:677`）。改由 `decisionsByHorizon` **投影**（在装配循环内逐行搬运，不重算），缺席档不出键 ⇒ 全链只剩一个逐档数值源。**顺带修掉一个既有缺陷**：映射原按 snake 出键而展示层按 camel 读（`agentOutput` 只做类型断言不转键）⇒ 四键里只有 `ultraShort` 拼写错开，表现为「超短档价位行从来不显示、另三档正常」，而库里四键齐备（现网实证：`mid / long / short / ultra_short`）。现在产端改 camel、读端两族都收（存量行仍能显示四行），断言见门 ⑦ 与 `normalizeHorizonPriceMap` 的 vitest |
| 4 | 门 | 整脚本门加断言：主档数值 == 所选档行（叠加风控后 ≤ 上限）；`horizonPriceMap[主档]` 与该行逐字段相等；`positionSource` 三值反向锁；`portfolio_mgr_horizon_lock_rhai` 的仓位口径逐字锁随第 1 条翻向 |

**退役面（必须当场点明，否则下一次「调档不动」又要从头查）**：`sl_pct_for` / `tp_pct_for` /
`stop_source_for` 不再参与主档与价位映射输出，只保留两处用途 —— σ 不可得时的兜底、
风险预算上限的分母。**推论**：设置面板那八项 `sl_pct_*` / `tp_pct_*` 调档参数
**只对兜底路径生效**；这条要同时写进面板说明文案与 `seed_stock_analysis.rs` 的 v125 注，
否则「面板改了、四档 Tab 不动」就是本轮刚修掉的那类同屏矛盾的下一次复发。
`PORTFOLIO_MGR_TUNABLE_PARAMS` 那八项**不删**（兜底路径与反思建议仍需要入口），但语义变了：
从「主决策档位」变成「退化档的保守档位」。

**回退成本**：单片可逆；模板 bump 到 v125，重播种即回 v124 行为。

**同批要顺手收掉的两处陈旧文档**（都是本轮改动造成的，不是历史包袱）：
`src-tauri/src/commands/stock_workflow/decision.rs` 的 `extract_decisions_by_horizon` 文档
仍写键为 `ultra_short`（实为 camelCase，现网已核）且仍提 `confLowerBound`（本轮已随幽灵字段删除）；
`src/types/stock-analysis.ts` 的 `HorizonPriceMap` 注释要随第 3 条改成「分支输出的投影」。

**落地时的两处偏差（与上表不同，以代码为准）**：
① 第 3 行的键拼写：计划写「沿用 snake」，实际改成 **camel** —— 因为落地前用现网读数核到了
   「超短档价位行从不显示」的既有丢键缺陷（snake 产端 × camel 读端，只有 `ultraShort` 一族错开）。
   产端转 camel 与 `decisionsByHorizon` 同族，读端 `normalizeHorizonPriceMap` 两族都收以保存量行。
② `branch_row_ok` 的自证字段从一条变两条（`confidenceMethod` + `stopSource`）：主档止损标签
   既然改为「取被采用那条算法自己的话」，一行说不出止损来历就不能被当主档用。
   连带装配段的缺席文案同步点名两个字段。

### 阶段 5 重放核对清单（①落地后必须逐条看，缺一条就是没核）

合成门只锁得住方向，锁不住「真数据下会不会被后面的封顶清零」——S5 实测：整脚本门里主档仓位
是 0，而 mid 行是 12（`positionSource` 仍标 `branch_position_capped`）。原因是主档在选出之后
还要过若干同样只做减法的关卡（证据不足 ⇒ 空仓、风险等级上限、V59 试探仓）。所以重放时要看的
是**这四条一起**，不是只看「仓位非 0」：

| # | 看什么 | 期望 | 不期望说明什么 |
|---|---|---|---|
| 1 | 主档 `timeHorizon` / `horizonSource` | 与四档里被选中的那档一致，来源 `branch_pick` | 选档没生效（或该轮四路全缺席 ⇒ 必须显式 `formula_no_branch`） |
| 2 | 主档 `positionPct` vs 所选档 `positionPct` | 相等，或**更小**且 `positionSource=branch_position_capped` | 更大 = 两处口径互相加成（① 要消灭的形态） |
| 3 | 主档 `stopLossPct` / `takeProfitPct` / `stopSource` | 与所选档行逐字段一致（空仓时整体为 0 / null） | 不一致 = 主链又在代算；空仓却带价位 = 把不下注伪装成等执行 |
| 4 | `horizonPriceMap` 与逐档 Tab | 同档同名同数；缺席档不出键；**四行都在**（含超短） | 少一行 = 键拼写或投影又漂移（S5 前是超短那行静默丢失） |

另附一条与本轮同族的观察项：000710 那行的历史读数是「主档 减持 / 0% 仓位 / 置信 38%」而
超短分支「买入 87.8」——重放后主档应改为**取自被选中的那一行**，方向与仓位都不该再来自
日线口径的后验阶梯。

## 五十一、阶段 4 剩余：读侧按代筛样（先写侧后读侧，2026-10-04 普查）

### 为什么顺序要反过来

原计划是「IC / 反思 / 错题本 / 回放按 `template_version` 分代筛样」。动手前量了一次现网分布：

```
template_version  n   latest
NULL             44   2026-10-02 18:11:44
93 / 79 / 118 / 91 / 88 / 117 / 119 …  2~4 条不等
```

且 `select ... where template_version is null group by analysis_kind` 的聚合口径下，**44 条 NULL 全是 `analysis_kind = live`**（`horizon_decisions` 非空的条数 = 0）。⇒ 结论：`template_version IS NULL` **不等于**「列引入前的老记录」，它有相当一部分是**近期由旁路写入**的现役行。

先修读侧会得到一个假的分母：按 `>= 125` 筛样时，这些「未知代但其实是现役算法」的行被整批排除，指标大面积变成「样本不足」，而真因是写侧不落版本号。所以顺序是 **① 写侧定性 → ② 读侧筛样 → ③ 门**。

### ① 写侧：先分名目，不要一律补值

写入点共 9 处（`grep template_version` 覆盖 Sea-ORM 表达式 + 裸 `Expr::value`）：

| 位置 | 现值 | 定性 |
|---|---|---|
| `src-tauri/src/commands/stock_workflow/core.rs:595` | `Set(None)`（建行初值） | **中间态**：同函数的 :1165 / :1504 会补写 ⇒ 不是缺陷 |
| `src-tauri/src/commands/stock_workflow/core.rs:2056` | `Set(None)`（`run_single_stock_analysis` 的 "running" 占位行） | **就是这里**：建行留 NULL 正确，但本通道的**落库 update 从不补写** ⇒ 现网 44 条 NULL 全来自它（本函数不经 `run_stock_workflow_inner`，原注释那句「由主路径写回」是错的）。2026-10-04 已补 `:2072` 的 `template_version: Set(Some(loaded.version))` |
| `src-tauri/src/commands/stock_workflow/hooks.rs:1065` | `Set(None)` | **不是缺陷**（同日核对更正）：那是对话直执行的**建行**，`analysis_kind = "chat"`、不经 `workflow_templates` ⇒ 无版本可言；该行注释已陈明「有快照但无版本」是这类记录的正常形态 |
| `src-tauri/crates/analysis-engine/src/trade_intent.rs:243/599/672/742/827` | `Set(None)` ×5 | 交易意图行，**不是分析结论** ⇒ 正确形态是「保持 NULL + 读侧按名目排除」，而不是给它套一个分析模板版本号（那是把两个名目焊成一个） |

判据：**只有「会被读侧当决策样本」的行才必须带代**；不属于该名目的行宁可 NULL + 显式排除，也不要造一个版本号出来。这条与禁区 12（不许把不同名目焊成同一套）和「结构性缺口不得伪装成结论」是同一条。

### ② 读侧：沿用既有水印机制，不新造一套

`crates/analysis-engine/src/reflection_stats.rs` 已经有一套**样本自带判定口径水印**的做法：`DecisionPerformanceSample.snr_anchor_days`（注释原文：「两代混在一档里会打乱池化秩 ⇒ 引擎按此位决定是否进 IC 分母」）。分代筛样应当是**加一个同类水印字段**，而不是在各调用方各写一份 SQL 过滤：

- 样本新增 `template_version: Option<i32>`，取自已 JOIN 到的分析行（`nearest_analysis_dimensions` 返回的就是整行，字段现成）；
- 新增常量（放 `holding_period.rs` 与值域同处）：`HORIZON_BRANCH_GENERATION_FLOOR = 125`；
- 各指标**自己声明**起算代，并把「因代际被排除的样本数」随结果回传，走既有 `ic_status` 缺席通道新增两个状态：`generation_pre_floor`（旧代样本，算法不可比）、`generation_unknown`（写侧未落版本，见 §五十一-①）—— 两者文案必须分开，合成一句「样本不足」就是把两种完全不同的处置（等攒样本 / 去补写侧）并成一条。

跨行聚合入口清单（普查结果，全部**当前无代际过滤**）：`reflection_stats.rs:662 build_hitrate_stats`（下游三处：`horizon_prior.rs:163`、`hooks.rs:428`、`src/commands/stock_workflow/reflection_stats.rs:21`）、`stock_analysis.rs:1959 backtest_all_history`、`src-tauri/src/commands/stock_workflow/core.rs:2173 fetch_similar_cases`（90 天 loss 错题本）、`stock_analysis.rs:4708 reco_ic_stats` + `recommender/ic.rs:140`、`backtest_validation.rs:469`、`monthly_report.rs:52`、`backtest.rs:417/428`、`evolution_drift.rs:156`→`evolution_optimizer.rs:133`、`key_levels.rs:110`、`trade_review.rs:86`、`stock_workflow/misc.rs:114`、`reflection.rs:840/1451-1530`。
已带部分代际判据的：`dao/src/repo/stock_lesson_queries.rs:114`（只筛档名）、`decision.rs:3340`（复算时比对版本漂移）、`src/bin/axagent-batch-rerun.rs:490`（`TemplateVersion.is_null()`）。

### ③ 门：新增读者必须登记

清单由单一权威渲染并配反手抄门（同 `PORTFOLIO_MGR_TUNABLE_PARAMS` 的做法）：把「跨行聚合入口」做成一份 Rust 常量清单，配一条门断言**每个登记项的函数体都调用了同一个筛样 helper**，并加一条自证（把 helper 调用从某个登记项里拿掉必须红）。否则今天列的 16 处只是本文件里的一张表，下一个人加读者时依旧无声漏掉。

### 必须先说清的后果（不是 bug，是诚实代价）

现网总样本约 60 行，按 `>= 125` 筛后**多数指标的可用样本会掉到 0~个位数**，面板会大面积显示「样本不足 / 待攒样本」而不是一个命中率数字。这正是分代的目的（不再拿两代混池的数当指标），但视觉上像「指标变差了」⇒ 需要在面板上显示分母与被排除原因，否则就是换了一种方式误导。

## 五十二、#33 普查：其余 .rhai 的「f64 单型数值判据」（2026-10-04，只读普查，未改生产）

### 口径先钉死（两条注入通道，不是「JSON 数字一律 f64」）

| 通道 | 代码位置 | 整数的落点 |
|---|---|---|
| **顶层标量** number 参数 | `src-tauri/crates/rt-workflow/src/work_engine/executors/code_executor.rs:194` —— `Value::Number` 先 `as_f64` | f64 |
| **其余一切**：顶层 map/array 参数的**每一层**、`json_parse()` 的产物 | `src-tauri/crates/harness/src/rhai_engine.rs:156` —— `as_i64` 优先，真小数才落 f64 | **i64** |

⇒ `type_of(x) == "f64"` 只在「顶层标量」这一种形态下等价于「x 是个数」。拿它当数值判据读嵌套值，就是把整数判成缺席（本轮 P0 的成因，见 §四十九）。
辅助事实：64-bit 构建下 `type_of` 对整数返回 `"i64"`，**`"int"` 恒不成立**（仓内先例注释 `src-tauri/src/commands/data-quality.rhai:61`；仓内 8 个脚本写成 `"i64" || "int"` 成对兜桥）。

### 分布读数（26 个 .rhai，机械计数）

- `type_of(...) == "f64"` 出现 **89** 处，其中同行带 `"i64"` 兜桥的 **14** 处；
- **未走桥、且操作数是嵌套读取**的 f64 单型守卫 **22** 处；
- **`== "int"` 且同行无 `"i64"`**（纯死分支）**4** 处。

主脚本已有正确桥 `num_of`（`src-tauri/src/commands/portfolio-mgr.rhai:75-79`），注释即写明「旧代码只判 f64 会漏读 i64（静默归零）」⇒ 22 处里同一文件还有 5 处没走它，是同一条已认识到的规则没铺开。

### 22 + 4 处逐条定性（关键是「现网能不能命中」）

| 站点 | 操作数来源 | 生产者数值形态 | 现网后果 |
|---|---|---|---|
| `portfolio-mgr.rhai:1251/1275/1296/1328/1329`（筹码面：`t["shares"]` / `l["unlockRatio"]` / `b["discount_pct"]` / `e["buyAmount"]` / `e["sellAmount"]`） | `lb = safe_parse(lockup_bundle)` ⇒ 嵌套 | `get_stock_lockup_bundle` 的 Rust 字段**全声明 f64**（`src-tauri/crates/astock-data/src/types.rs:112/125/126/218/424/441`），serde 出 `1200000.0` ⇒ 注入回 f64 | **暂不咬人**。安全来自「生产者恰好是 f64」，不是判据成立 —— 股数/金额这类天然整数字段一旦被改成 `i64`/`u64`，筹码面三条腿当场静默归零，且没有任何门会红 |
| `pace-calc.rhai:157/159`（`inner["confidence"]`） | `llm_events` = `a-catalyst.content`，**LLM 手写 JSON**（`seed_stock_analysis.rs` 的 input_mapping 实接） | 整数极常见（`85`、`1`） | **真命中**：既不是 f64 也不是 `"int"` ⇒ 落 `0.5` 兜底 ⇒ 把「模型给了 85%」呈现成「50% 中性」，属用近似值顶替，不是缺席声明 |
| `pace-calc.rhai:235/249-250/271-272/290-291/303-304`（`evt[...]` / `item[...]`） | 混合：脚本自己 push 的事件（值面是 f64 字面量/除法结果）+ 来自 `llm_events` 的原始 item | 前者 f64 ⇒ 成立；后者同上一行 | LLM 侧同样可命中，形态与上一行相同 |
| `pace-calc.rhai:325/333`（`p_history` 数组元素的 `"int"` 死分支） | 顶层数组参数 ⇒ 元素按嵌套口径 | **`("p_history", "")` 未接线**（`seed_stock_analysis.rs:5528`，注释自陈「暂未接入」）⇒ 整段 320-343 现网不执行 | 无现网后果，但它是「死代码里再叠一个死分支」——将来接线时 mean/std 会漏掉全部整数项 |
| `consistency-check.rhai:69`（`safe_f64` 的 `"int"` 死分支） | `obj[key]`，obj 来自其它 Rhai 节点输出的 map | 现网读的两个键（`bottleneck_composite` / `supply_rigidity_score`）由 `bottleneck-calc.rhai:126/138` 的 `clamp(...)` 产出 ⇒ f64 | **暂不咬人**，同样靠生产者是 `clamp` 返回值；脚本若改成整数字面量即落 `-1.0` 哨兵（「无数据」） |
| `data-verifier.rhai:180/196/212/229/241/253/266` | `fin.*` 嵌套字段 | 写法是 `if f64 {…} else { x.to_float() }` ⇒ i64 走 else 被正确转换 | **形态正确**，不计入缺陷（但同形态对 `()`/string 会抛运行期错，靠上游 present 守卫） |

### 结论

除本轮分支表那处（v124/v125 已修）之外，**没有发现新的现网「整数被判缺席」事故**；但整类判据的成立性目前是**寄生在生产者的字段类型上**的，不是判据本身成立。唯二有现网暴露的是 `pace-calc` 读 LLM 置信度那一族（整数 ⇒ 静默 0.5）。

### 待拍（编号，不动生产即不产生数值变化）

① 把 4 处 `"int"` 死分支改成 `"i64"`（或直接走 `num_of`）：纯正确性收益、现网恒不触发 ⇒ 零行为变化；
② 把「LLM 手写 JSON 的嵌套数值读取」列为必须走桥的面（`pace-calc` 10 处），改成 `num_of` 形态 —— **会改决策数值**（整数置信度从 0.5 变回真值），需带模板 bump 与整脚本门负控；
③ 新增一条机械门锁住这条规则：`.rhai` 里出现「同行只有 `== "int"`」或「嵌套读取的 f64 单型守卫」即红，**存量挂基线、新增即红**（同 `check-single-source-facts.mjs` 的 baseline 做法），否则这张表只是本文件里的一次性读数；
④ `("p_history", "")` 一族「空串映射 ⇒ 整段脚本死代码」要不要单列一条清单门 —— 与本轮四周期主线无关，先记不领。

### §五十二 附（同日执行轮）：①②③ 已落地，表内行号是**修复前**读数

批准「开工并一起批」后，①②③ 与 §五十一-① 同批落地（模板 v125 → **v126**）：

| 事项 | 落点 | 读数 |
|---|---|---|
| ①② 数值桥下沉 | `crates/harness/src/rhai_engine.rs` 注册 `num_of(x)` / `num_of(x, dflt)` 两个元数；删掉 `portfolio-mgr` / `risk-level` / `trader-proxy` 三份脚本内实现，`consistency-check.safe_f64` 改走桥 | harness 新增 `num_of_bridges_nested_integers_and_keeps_absence_explicit`（**已跑，读数在此**：
`cargo test -p axagent-harness --lib` 780 passed / 0 failed，其中该条单独跑 `1 passed`）。
它顺带钉住一条此前只能推断的事实：**Rhai 1.26 不把 i64 自动升格进 f64 形参** ——
探针 `clamp(m["p"], 0.0, 5.0)`（`m` 来自 `json_parse`，`p` 是整数 ⇒ i64）返回 `Err`
（`Function not found`），断言 `is_err()` 为真。
⇒ 严重性比「静默读成缺席」还高一档：v125 主链那句裸 `clamp(picked_row["positionPct"], …)`
一旦遇到整数形态的分支输出，不是读成 0，而是**整脚本抛错被 catch 兜成「数据缺失」**；
③ 把分支行读取过桥正是消掉这一格（同时也就消掉「缺席伪装」那一格）。） |
| ② 嵌层守卫收编 | `pace-calc` 10 处 + 2 处死 `"int"` 求和分支、`portfolio-mgr` 筹码面 5 处、`consistency-check:69` | 唯一有现网数值影响的是**整数置信度不再落 0.5**；其余站点生产者本就是 f64 ⇒ 读数不变，改掉的是「换个人把字段改成整数就静默归零」这条寄生 |
| ③ 机械门 | 新增 `scripts/check-rhai-numeric-typing.mjs`（a 嵌层必桥 / b `"int"` 死分支 / c 桥唯一），零基线，接 `ci-check.mjs` + `ci.yml` | 修复前对真树红 **22 处**（与人工普查逐条同集），修复后绿；`--selftest` 正负对照 6 条全过 |
| §五十一-① 写侧落代际 | `core.rs` 的 `run_single_stock_analysis` 落库 update 补 `template_version: Set(Some(loaded.version))`；建行处那句「由 `run_stock_workflow_inner` 写回」的注释**是错的**（本函数不经它），已改为陈明实情 | 新增 `src-tauri/tests/stock_workflow_write_side.rs`：正断言 + 摘行负控；同时反向锁 `trade_intent.rs` 五处**继续 NULL**（交易意图不是分析结论，不得焊上分析代际 —— 禁区 12） |

`hooks.rs:1065` 经核对**不是缺陷**：那是对话直执行的建行（`analysis_kind = "chat"`），注释已陈明该通道不经 `workflow_templates` ⇒ 无版本可言；现网 44 条 NULL 全是 `live`，来源就是上面这一处。
PLAN 上方 §五十二 表内的 `pace-calc.rhai:157` 等行号保留为**普查时点**读数（该文件本轮被改，行号已下移）。

### 执行轮读数（2026-10-04，同批）

| 验证项 | 读数 |
|---|---|
| `cargo test -p axagent-harness --lib` | **780 passed / 0 failed**（含新增 `num_of_*` 条；单跑该条 `1 passed`） |
| `cargo test -p axagent --lib stock_workflow::rhai_registry` | **12 passed / 0 failed**，其中新增 `pace_calc_integer_confidence_is_not_read_as_absent ... ok`；同批 `every_host_fn_used_by_production_scripts_is_registered` 与 `every_production_script_calls_closures_via_method_call` 仍绿（⇒ 全局 `num_of` 在生产 Engine 里解析得到，且没有引入按名调用） |
| `cargo fmt --all --check` | **0**（全工作集，不只是本包） |
| `check-rhai-numeric-typing.mjs` | 修复前对真树 **红 22 处**（a 15 / b 4 / c 3，与人工普查逐条同集）⇒ 修复后 **绿**，扫描面 68/15/2 非零；`--selftest` 6 条正负对照全过 |
| `audit-inject-coverage.mjs --strict` | **0**（新局部名 `picked_days` 等被正确归为「脚本内自给」） |
| `check-single-source-facts.mjs --ci` | 从「我造成 2 处客观错」收到只剩**已知 HEAD 红 #28**（`engine/mod.rs:3144`）；`mod.rs` 里 `risk-level.rhai:111-113` 因删 2 行指到空行，已改 `113-114`，本文件 §五十一 三处 core.rs 行号同步对齐 |
| `src-tauri/tests/stock_workflow_write_side.rs` | **仍未跑成**（不是判据红）：`cargo test -p axagent --test <名字>` 与 `--workspace` **都**要重链 `target/debug/axagent.exe`，被运行中的应用占住 ⇒ `拒绝访问 (os error 5)`，测试进程没起。⚠ 我一度以为 `--test` 选目标能避开 —— 那次其实是 `--no-run` 被自己的 `timeout 400` 杀掉（`EXIT=124`、rustc `143`，尾部还印 `could not compile axagent (lib)`，看着像编译错误、实为 SIGTERM），根本没走到链接步 ⇒ 对锁**没有否证力**。只能关应用或交 CI 收这一步。判据本身已用同一口径在盘上复核（正断言 / 建行 NULL / 摘行负控 / trade_intent 五处无代，全 true） |
| `cargo test -p axagent --lib`（全量） | **532 passed / 1 failed** —— 唯一那条是已知 HEAD 红 #26（`src-tauri/src/commands/stock_workflow/serenity.rs:2333` report 解包回收候选），`git status` 证 `serenity.rs` 与 `crates/kit/src/utils.rs` 零本地改动 ⇒ 非本批造成；比上一轮 531/1 多出的那一条正是本批新增的 pace-calc 求值门 |
| `cargo test -p axagent-rt-workflow --no-fail-fast` | **20 个测试目标全绿**（含被我重同步的 `portfolio_mgr_horizon_lock_rhai` 9/9）；补跑 `clippy -p axagent-rt-workflow --all-targets --all-features -D warnings` **0**。顺带修掉该门自己的夹具盲区：片段测试原来用裸 `Engine::new()` —— `clamp` 是 Rhai 自带而 `num_of` **不是** ⇒ 已改调 `harness::register_common_functions`，与生产同一函数集 |
| `cargo test --workspace --exclude axagent --no-fail-fast` | **EXIT=0，136 个套件 ok / 0 failed**。⚠ 第一次不带 `--no-fail-fast` 的同名命令只跑到 **10 个套件**就停在 analysis-engine 的一条时序 flaky（`recommender::tests::cache_live_and_replay_are_isolated`，隔离复跑 1 passed、全量复跑亦绿）⇒ **`--workspace` 默认在首个失败二进制就停**，报「全量」必须显式带 `--no-fail-fast` 并附套件计数 |

## 五十三、000710 重跑（v126 生效后）暴露的「主档与所选档不同源」+ 呈现层档位普查（2026-10-04）

用户报：历史行（图1）显示「持有 · 超短线 · 分支选档 · 仓位 20.3% · 置信 47%」并带四档 chips
（超短线:买入 / 短线:持有 / 中期:持有 / 长期:减持），而决策面板（图2）显示
「#1 决策=持有 置信=46.9 仓位=20.3% … 视角部分一致:44分(维度:act=10 pos=…)」。
用户归因：「UI 从分析师、辩论师、风险分析师到决策都没按四周期显示」。

### ① 读数（PG `stock_analyses.id=2b0743ba-889e-42f0-b437-573bd5cde6f0`，`template_version=126`）

主档：`action=持有` `positionPct=20.3` `confidence=46.9` `timeHorizon=ultra_short`
`horizonSource=branch_pick` `positionSource=branch_position_capped` `stopSource=vol_band`
`sl=7.38` `tp=6.15` `days=2`。四档分支行：

| 档 | action | 仓位 | 置信 | sl | tp | days |
|---|---|---|---|---|---|---|
| ultraShort | **买入** | 21.0 | **87.8** | 7.38 | 6.15 | 2 |
| short | 持有 | 25.7 | 77.8 | 11.67 | 19.45 | 5 |
| mid | 持有 | 15.4 | 56.8 | 27.62 | 46.04 | 28 |
| long | 减持 | 0.0 | 33.1 | 49.52 | 0.0 | 90 |

选档判据成立（可执行档里置信最大 = ultraShort 87.8）；`reasoning` 原文含
`⚠️高风险风控否决:买入→持有(高风险禁止加仓)` 与 `⚠️高风险:仓位≤35.0%`。

### ② 三层结论（先分层，再谈改法）

- **数据层**：四档分支行与主档字段**都在库里且互不矛盾** —— 主档 action 是分支结论被后置风控**单向降级**的结果，
  仓位是 `min(分支, 风险预算)`，sl/tp/days 逐字段等于 ultraShort 行。
- **执行层（真缺陷，UI 改不掉）**：v125 批准 ① 只把**仓位/止损/止盈/天数/stopSource** 搬到所选档，
  **`action` 与 `confidence` 没有跟着走**：
  - `action` 被主链后置规则改写，但**没有 `actionSource` 标签**可反解（`positionSource` 有四值、`stopSource` 有，
    action 只能从 `reasoning` 的自由文本里读）⇒ 违反本轮自己立的规矩「谁被用上必须能从输出反解」；
  - `confidence=46.9` 来自主链 `final_confidence_pure`（先验/后验口径），与所选档的 87.8 **不同源且无标签** ⇒
    同屏两个置信度是**结构性**的，不是渲染错。
  - 附带：`reasoning` 里 `赔率=0.0(波动率fallback)` 与分支实际 tp/sl 不同源 —— 主链归因列仍是旧口径文本。
- **呈现层（用户点名的主因，成立）**：只有 2 个组件消费四档数据，其余全是单视角：

| 组件 | 档位感知 | 消费主档 reasoning/decisionJson |
|---|---|---|
| `DecisionBanner.tsx` | ✓（83 处） | ✓ |
| `DecisionTrustNotice.tsx` | ✓ | — |
| `AnalystReportCard.tsx` | ✗ | ✓ 5 处 |
| `DebatePanel.tsx` | ✗ | ✓ 4 处 |
| `DecisionExplanationPanel.tsx` | ✗ | ✓ |
| `ExitRecommendationPanel.tsx` | ✗ | ✓ |
| `ValueAssessmentPanel.tsx` | ✗（3 处仅文案） | ✓ 5 处 |
| `WhatIfBacktest.tsx` | ✗ | ✓ 7 处 |
| `StockAnalysisPage.tsx` | ✗（2 处仅传参） | ✓ |
| `dual-view/CompactDecisionComparison.tsx`、`dual-view/DecisionComparisonPanel.tsx` | ✗ | ✓ |
| `utils.ts` | ✗ | ✓ |

⇒ 图2 的「视角部分一致:44分」是**多分析师视角一致性**摘要，与周期轴正交；它和图1 的四档 chips 同屏时，
没有任何一处说明「哪一个是主档、主档来自哪一档、被谁改写过」。

### ③ 待拍（编号；① 是产出层、②③ 是呈现层，顺序建议 ①→②→③）

① **主档可反解收口**（改 `portfolio-mgr.rhai` + 值域三载体 + 门，**换代 v127**）：
   新增 `actionSource`（建议值域 `branch_pick` / `risk_veto_downgrade` / `probe_upgrade` / `main_chain`）与
   `confidenceSource`（`main_chain_posterior` / `branch_row`）；并把「主档 confidence 是否改取所选档」作为
   一个显式选项 —— 我倾向**取所选档**（与 ① 已批的仓位/出场同源原则一致），但这条会改主档数值，需单独点头。
② **决策面板按档组织**（`DecisionBanner` 已有档轴，缺的是把「主档 = 所选档 + 后置改写」这条来历**成句显示**：
   档名 + `actionSource` + 与分支原值的差异，如「超短线分支：买入 → 风控否决降为持有」）；
   同时把 `DecisionExplanationPanel` / `ExitRecommendationPanel` 的 reasoning 文本口径与分支实际值对齐（② 的附带项）。
③ **分析师 / 辩论 / 风险 / What-If 四组面板的档位化**（面积最大：每处都要决定是「按档 Tab 切换」还是
   「按档分组并列」，且新增文案要 11 语言真译 + 三种缺席各占一句）⇒ 建议单独开一轮，不与 ①② 混提。

### ④ 呈现层 B 的前置事实（2026-10-04 普查，改变了 ③ 的性质）

图上节点的档位维度实测（`seed_stock_analysis.rs` 全部节点 id）：

| 环节 | 节点 | 是否按档 |
|---|---|---|
| 取数 | `t-*-data`（14 个） | ✗ 一份共享 |
| 分析师 | `a-fundamentals` / `a-catalyst` / `a-news` / `a-policy` / `a-sector` / `a-sentiment` / `a-hot-money` / `a-lockup` / `a-research` / `a-market-analyst`（10 个） | ✗ **全部单份，无档位后缀** |
| 辩论 | 在各 `a-*` 内部（`debate_rounds`），无独立节点 | ✗ 单份 |
| 风险 | `t-risk` + `cls-risk-level` | ✗ 单份（四路分支都读同一个 `cls-risk-level.result.category`）—— **【v128 已改】** 一根轴按档：四个 `cls-risk-level-{tier}` 各读本档窗口回撤深度；波动率/夏普/基本面三项仍共用 60 日全局值。见 §五十四 B1 执行记录 |
| 评分 | `t-scoring` + **`t-scoring-hour` / `-week` / `-month` / `-quarter`** | ✓ 四档各一份 |
| 决策 | **`pm-h-ultra-short` / `-short` / `-mid` / `-long`** | ✓ 四份 |
| 仲裁 | `pm-arbiter` | 单份（按 Q3 裁定只管「钱往哪走」） |

⇒ **R-11 原设计是「4 个 SubWorkflow 各挂该档分析师子集 / prompt / 评分 / 单轮对抗 / 风险 / 独立决策」，
现状只落了「评分 + 决策」两格。** 分析师、辩论、风险这三组**在数据层就没有四份**——
所以 ③ 里「把分析师/辩论/风险面板按四周期显示」若照字面做，只能把同一份内容复制四遍，
那正是本轮反复判废的形态（伪装成按档）。

### ⑤ 因此 ③ 拆成两个决定（需拍板）

甲 **呈现层只做「有四档数据的地方按档」**：评分四份、决策四份、价位/出场四份按档并列；
   分析师 / 辩论 / 风险三组改为**显式声明共用**（一句「本轮四档共用同一份分析师与风险输入」，
   按「三种缺席各占一句」的规矩成句，不复制四遍）。⇒ 纯前端 + i18n，零图改动，可即刻做。
乙 **图形态补齐**：每档跑自己的分析师子集与风险分类（R-11 原意）。代价账必须先算：
   分析师调用 ×4（现 10 个节点单轮 10 次 → 最坏 40 次/轮，含对抗更多）；
   裁剪版可行 —— 超短档只挂 `a-hot-money` / `a-catalyst` / `a-news` / `a-sentiment`，
   长线只挂 `a-fundamentals` / `a-research` / `a-policy` / `a-lockup` ⇒ 调用数不升反可能降。
   但这是图 + prompt + 反思绑定 + 门禁的独立一轮，不应与甲混提。

建议顺序：①（已做）→ 甲 → 乙（单独拍）。

### ⑥ ②（= 新编号 ①甲）落地读数（2026-10-04，模板 v126 → **v127**）

| 面 | 改动 | 读数 |
|---|---|---|
| 产端 | `portfolio-mgr.rhai`：新增 `action_source`（七值，最后改写者优先）与 `confidence_source`（两值）；主档 `confidence` 改为「所选档 × 档位无关的视角分歧/质量约束」；`kelly_note` 下移到出场段之后，赔率按**实际采用的出场对**（`adopted_exit_pair`）重算；`reasoning` 的置信度改用同源值并标口径 | 主 crate lib **532 passed / 1 failed**（唯一红 = 已知 HEAD #26），其中 `portfolio_mgr_runs_with_valuation_evidence_without_degrading` 与 `pace_calc_integer_confidence_is_not_read_as_absent` 均 ok；rt-workflow `--no-fail-fast` **exit 0** |
| 整脚本门 ⑦ | 新增四条：值域外/缺失即红、「标签说直取 ⇒ action 必须等于该档 action」、`confidenceSource` 两值、`branch_row` 时主档置信度**不得高于**该档自报值 | 同上（断言在本批新增后立即跑绿） |
| 展示层值域 | `stock-analysis-utils.ts` 新增 `ACTION_SOURCES` / `CONFIDENCE_SOURCES` + `actionSourceLabelKey` / `confidenceSourceLabelKey`（未命中返回 null，不给旧记录编来历）+ `readDecisionProvenance`（三形态） | 新门 `stockAnalysisHorizon.test.ts` **21 passed**，含**变异负控**（往产端塞一个值域外标签 `invented_label`，门必须只报它） |
| 归一化白名单 | `agentOutput.normalizeDecision` 显式带上两个新字段 | 不带上就会重演 2026-09-11 那次「字段一直发、界面从未显示」（`weightsCollapsed` 同型） |
| 成句显示 | `DecisionBanner`（工作区嵌入版与主标题行**共用**一个 `provenanceLine` 定义）、`HistoricalAnalysisPanel`、`AnalysisHistoryButton` 三处；`main_chain_posterior` 时才追加置信口径点名 | `DecisionBanner.test.tsx` **15 passed**（三格：降级成句 / 主链格点名 / v127 之前整行不渲染） |
| i18n | 11 语言 × 11 键（9 标签 + 2 模板），逐语言真译 | `check-i18n-key-parity` 11 文件一致、`check-i18n-placeholder-usage` 干净；测试里另锁「非中文语言不得抄 zh-CN」与两条模板的插值集合逐语言一致 |
| 全量 | `npm run test:run` **190 files / 2135 tests 全绿**；`typecheck` 0；`oxlint --max-warnings=0` 0；`dprint check` 0；`check-rhai-numeric-typing` 绿（扫描面 68/15/2） | clippy / `cargo fmt --all --check` 在跑 |

**③（新编号 甲/乙）未动**：分析师 / 辩论 / 风险三组面板**在数据层就没有四份**（见 ④ 的节点普查），
按 ③ 字面做只能复制同一份内容四遍 ⇒ 已停在 ⑤ 的拍板问题上，不在本轮偷偷实现。

## 五十四、乙：分析师 / 辩论 / 风险按档补齐（2026-10-05 批准「甲+乙」后的分阶段计划）

### 成本账的事实（不估，全取自仓内）

| 环节 | 现状 | 类型 | 按档化的边际成本 |
|---|---|---|---|
| 分析师 | 10 个 `a-*` 节点（`seed_stock_analysis.rs` 的 `analysts` 数组） | LLM AgentNode | 每次调用都算钱 ⇒ 取决于子集矩阵（B2） |
| 辩论 | `DEFAULT_DEBATE_ROUNDS = 3` ⇒ 6 个辩手节点（bull-rN/bear-rN） | LLM AgentNode | 最贵的一格（B3） |
| 风险分类 | `cls-risk-level` —— **v55 起是 CodeNode（`risk-level.rhai`），不是 LLM** | Rhai | **零 LLM 成本**，可即刻按档（B1） |
| 风险数据 | `t-risk`（ToolNode） | 取数 | 保持共享一份 —— R-11 裁定「取数层共享一次」，不得按档重拉 |
| 评分 / 决策 / 出场 | `t-scoring-{hour,week,month,quarter}` + `pm-h-*` | 已是四份 | 无需动 |

⇒ 顺序按「成本从零到高、且后者依赖前者」排：**B1 风险 → B2 分析师子集 → B3 辩论 → B4 呈现层切换 + 撤声明**。

### B1 风险按档（本轮先做，零 LLM 成本）

1. 图上把 `cls-risk-level` 展开成四份：`cls-risk-level-ultra-short` / `-short` / `-mid` / `-long`，
   每份的 `input_mapping` 读**同档的输入**（波动率窗口、回撤窗口、`kline_bars` 区间按档裁剪），
   输出 `cls-risk-level-<tier>.result.category`；
2. 四路分支脚本 `portfolio-mgr-h-*.rhai` 改读自己那一格（现在四路都读同一个 `result.category`）；
3. 主链的 `pm_risk_veto` / 仓位上限仍用主档那一格 —— 但**主档 = 所选档**，所以它自然跟着变；
4. 换代 **v128**（会改决策数值：某档风险等级可能与旧的全局等级不同 ⇒ 降级判定不同）；
5. 门：① 图门断言四份节点齐备且各自 `input_mapping` 不含全局 `cls-risk-level`；
   ② 分支脚本门断言每档读的是自己那一格（负控：改回读全局必须红）；
   ③ `check-input-mapping.mjs` 覆盖新节点。

### B2 分析师子集（需要一张矩阵，先给默认值）

按「该档的判断依赖哪些信息」定，而不是按「谁闲着」：

| 档 | 子集（默认提案） | 理由 |
|---|---|---|
| 超短线 | `a-hot-money` `a-sentiment` `a-news` `a-catalyst` | 两日窗口内起作用的是资金、情绪、突发与叙事 |
| 短线 | 超短四 + `a-market-analyst` `a-lockup` | 加入技术结构与解禁/减持这类周内供给冲击 |
| 中期 | `a-market-analyst` `a-fundamentals` `a-sector` `a-research` `a-policy` | 周—月尺度靠景气度、估值与政策 |
| 长期 | `a-fundamentals` `a-research` `a-policy` `a-lockup` `a-market-analyst` | 季—年尺度看商业模式与制度变化 |

⇒ 调用数 4 + 6 + 5 + 5 = **20 次/轮**（现状 10 次）。

**裁定（2026-10-05，用户）**：不能只给候选主档（置信最高的两档）跑子集，**全部四档都要跑**。
⇒ 曾提的「只跑候选两档 ≈ 12–13 次」这条省法**作废**，不再作为备选项出现在后续讨论里。
成本翻倍是这条裁定的**已知且被接受**的代价，不是待发现的缺陷。

由这条裁定顺带定下的原则（后续档位相关的取舍一律照此判）：**按档化的对象是「哪些档」，
不是「哪几档值得」**——挑子集跑会让未跑的那些档在数据层就没有结论，呈现层只能靠兜底或沉默，
而「四档并列且互斥」的展示已经承诺了四档各自成立（§五十三 ① 的成因正是这种承诺与实现的错位）。

⇒ 同一条原则回落到 B3：**辩论也按四档**，「只给候选两档各 1 轮」的备选项一并作废
（注意：裁定管的是**档位集合**，轮数仍是独立旋钮 —— B3 取 1 轮/档 = 8 次，见下）。

### B3 辩论按档

现状 3 轮 × 多空 = 6 次；全按档 × 3 轮 = 24 次。档位集合按 B2 裁定＝**四档全跑**，
轮数取 **1 轮/档**（bull + bear ⇒ 8 次）。

「只给候选两档各 1 轮」（4 次）这条省法已随 B2 裁定作废，不再作为备选。
轮数若日后要调，必须是「四档同轮数」（否则又造出数据层的档位不等），且要换代并配门。

### B4 呈现层切换与撤声明（硬条件）

甲 落地的 `HorizonScopeNotice` 三格是**对现状的诚实声明**，不是永久装饰：
B1 落地后 `risk` 那格必须删（风险已按档），B2/B3 落地后 `analyst` / `debate` 两格依次删除。
⇒ 配一条门：声明的 scope 与其数据层事实一致（节点是否按档），**不许面板挂着「四档共用」而图已经按档**——
那与反向的伪装同样有害（读者会以为按档数据是复制品）。

### 未做的余项

`WhatIfBacktest` / `dual-view` 两面板的按档化不在 B1–B4 任一格里（它们消费的是主档决策与回放），
待 B1 落地后按同一「声明与事实一致」的原则重查，不预先改动。

### B1 的设计更正（2026-10-05，动手前查到 ⇒ 原写法是伪装）

上面 B1 第 1 步写「把 `cls-risk-level` 展开成四份节点」—— **照做会得到四份完全相同的结论**。
实证：该节点 `input_mapping` 的八个输入全部来自**同一个** `t-risk` 的输出
（`risk_volatility` ← `t-risk.result.content.stockRiskProfile.annualizedVolatilityPct`、
`risk_drawdown` ← `.maxDrawdownPct`、`risk_sharpe` / `risk_roe` / `risk_gross_margin` /
`risk_debt_ratio` / `risk_revenue_growth` 同源，见 `seed_stock_analysis.rs:4238-4256`）。
输入不分档 ⇒ 复制节点只是把一份判断印四遍，比现状更坏（读者会以为拿到了四个独立风险等级）。

**更正后的 B1（三步，仍是零 LLM 成本）**：
1. **取数层扩窗，不扩调用**：`t-risk` 的 `stockRiskProfile` 增加**按窗口**的波动率与最大回撤
   （2 / 5 / 28 / 90 日，一次计算四组输出）。基本面三项（ROE / 毛利率 / 负债率 / 增速）与窗口无关，保持单份。
   —— 这与 R-11「取数层共享一次」不冲突：多算的是同一份 K 线上的四个窗口，不是四次网络取数。
2. `risk-level.rhai` 增加一个 `window` 入参，波动率/回撤改读该窗口那一组；图上展开成
   `cls-risk-level-{ultra-short,short,mid,long}`，每份只读自己那一组窗口值。
3. 四路分支脚本改读 `cls-risk-level-<tier>.result.category`（现在四路读同一个全局格），
   主链 `pm_risk_veto` 用**所选档**那一格 ⇒ 换代 v128（某档风险等级可能与旧全局等级不同，降级判定随之变）。

门：① 图门断言四份节点各自映射里**不含**全局 `annualizedVolatilityPct` / `maxDrawdownPct`（负控：改回读全局必须红）；
② 分支脚本门锁「每档读自己那一格」；③ `check-input-mapping.mjs` 覆盖新节点；
④ 甲 的 `HorizonScopeNotice` 里 `risk` 那格随 B1 落地**同步删除**（B4 的「声明与事实一致」原则，先于 B2/B3 触发）。


### B1 的第二条更正 + 执行记录（2026-10-05）

**上面「更正后的 B1」第 2 步仍有一处统计错误**：它写「`risk-level.rhai` 增加一个 `window` 入参，
波动率/回撤改读该窗口那一组」—— 照做会把**绝对阈值判据跨窗口复用**，而这三条轴的窗口性质不同：

| 轴 | 是否窗口无关的同一量 | 拿 60 日校准的绝对阈值判别的窗口 |
|---|---|---|
| 年化波动率 | 是（同一 estimand，只是样本少 ⇒ 噪声大） | 可用，但 2 日窗口只有 2 个自由度 |
| 夏普 | 同上 | 同上 |
| **最大回撤** | **否 —— 随窗口单调增长，是两个不同的量** | **恒不命中 ⇒ 按档反而放松** |

实测（真库 131 份 `t-scoring` 日线样本，同一 σ 口径，SQL 见下）各窗口回撤中位数：

```
w(交易日)   2      5      28     60     90
dd 中位%   0.9    2.7    17.3   32.9   37.7
depth p90  1.62   1.38   1.42   1.50   1.49
depth p50  0.28   0.48   0.94   1.06   1.03
vol 中位%  33.7   48.6   55.3   57.4   62.0
```

⇒ 原判据「60 日回撤 >45% 算量化高风险」换到 2 日窗口等于要求两天跌 45%（样本里最大 2 日回撤
远不到），那一档的这条判据**恒为假** ⇒ 结论系统比全局档**更松**，与按档目的相反。

**采用的判据（本版唯一新增的一条轴）**：`depth_w = 本窗口峰谷回撤 ÷ (σ_w × √w)`，
其中 **σ_w 是本窗口自己的日收益样本标准差**（`harness::indicators::realized_vol_pct`，全仓唯一 σ 实现）。

> **这里我自己踩了一次、靠实测打回**：首版分母用的是 60 日**全局** σ（顺手照了止损那条
> 「固定 lookback 的 σ × √持有期」的既有口径）。按出厂定义重测后立刻不成立：
>
> ```
> depth 分位（分母=60 日全局 σ，n=131）      2      5     28     60     90
>   p25 / p50 / p75                       0.00/0.21/0.48  0.20/0.36/0.67  0.65/0.84/1.00  0.85/1.06/1.23  0.77/1.00/1.29
>   p90 / max                              0.89/1.49       0.92/1.36       1.30/1.82       1.50/1.56       1.48/2.05
> ```
>
> 2 日窗口的**历史最大值只有 1.49** ⇒ 任何 ≥1.5 的单条线在超短/短档恒不命中 ——
> 那正是我这一轮开头用来否决「绝对阈值跨窗口复用」的同一个失败模式，只是换了尺度。
> 换成 σ_w 后 p90 回到 1.62/1.38/1.42/1.50/1.49（±0.12）才真正对齐。
> 判据：**「按档」的有效性必须由出厂公式自己的分布来验，不能用另一份口径的测量代验。**

阈值 **1.58** 的推导：60 日窗口上典型位移 σ_60 ≈ 28.5%（该样本集中位年化波动 57.4% ⇒ σ_daily 3.68%），
45% ÷ 28.5% ≈ 1.58；1.58 这条线在各窗口的命中面（σ_w 口径实测）约 **6%–11%**，
即每档都在「自身尾部之外」这一同一含义上生效 ⇒ 四档共用一条线，**不需要为每档另配一组参数**（R-11）。

**只升不降**：该臂只加在「高风险-A」的**或**侧（量化维度更容易判高），LOW / MID / EXTREME 各臂
一字未动 ⇒ 任一档结论只会 ≥ 旧全局档。按档不能成为放松风险的通道，与本仓
「action 后置规则单向降级」「荐股反思只降不升」是同一条纪律。

**落地形态（v128）**：
- `crates/harness/src/indicators.rs` 新增 `max_drawdown_fraction`（本仓第五份 peak-trough 的话就说不过去了：
  原 `astock-data/mcp_tools.rs` 的内联循环与 `analysis-engine/risk.rs::peak_trough_drawdown` 现都改为调它）；
  同文件按行号被 `PLAN §…:1737` 引用的三条测试行号已顺移（766/777/792 → 802/813/828）。
- `astock-data/mcp_tools.rs::compute_portfolio_risk`：K 线 60 根 → **120 根**（长档 90 日窗口要 91 根），
  新增 `stockRiskProfile.riskWindows.{ultraShort,short,mid,long}`，每格
  `windowDays / windowBars / maxDrawdownPct / drawdownDepth`；三个**全局**量化字段样本仍截最后 60 根
  （`GLOBAL_LOOKBACK`）⇒ 取数变宽 ≠ 口径变宽；次新股样本不足 ⇒ 两个数值落 `null` 不硬算。
- `risk-level.rhai`：新增两条**可选**输入 + 深度臂 + 输出 `axisScope`（按档边界的机器读面）。
- 种子：四个 `cls-risk-level-{ultra-short,short,mid,long}` 节点（各读自己那一格）、
  四路分支 `overall_risk` 改读本档节点、主链新增四个 `overall_risk_{snake}` 键并在选档后取更严者
  作为 `pm_risk_veto` 实参（留痕 **R-211**，已登记进 `portfolio-mgr` 的规则追溯码对照表）。
  **全局 `cls-risk-level` 保留且不注入按档两键** ⇒ 它的结论与 v127 逐位相同
  （research-mgr 的 `context_sources` 与主链 `LLM回退` 要的是 60 日整票档）。

**本版没有按档的（诚实边界，`axisScope` 自证）**：波动率 / 夏普 = 60 日全局；
基本面四项 = 最新财报（与窗口无关）；**f4 因子与 `risk_bias` 仍用全局档** —— 那两处在选档
之前执行（脚本顺序即判据），把它们也改成按档需要把选档段上移，属结构改动，未在本批做。

**门清单的三处更正**（相对上面写的四条）：
- ①「断言四份映射里**不含**全局 `annualizedVolatilityPct` / `maxDrawdownPct`」—— 按最终设计这条写反了：
  七条全局键**必须**仍指全局。改为锁「两条按档键必须指 `riskWindows.<本档>`」，
  负控是「把 mid 节点的深度改指 long 那一格 ⇒ 必须红」（`seeded_template_carries_horizon_scoped_risk_nodes`）。
- ③「`check-input-mapping.mjs` 覆盖新节点」—— **不成立**：该门的 `MAKERS` 只扫 `make_*` 构造调用，
  而本模板用 `WorkflowNode::Code(CodeNode { … })` 结构体字面量建节点，从来不在它的扫描面内
  （CI 里它连文件参数都没带，只扫 `seed_content_media.rs`）。⇒ 新增覆盖面由 ① 的 Rust 门承担，
  不改那个 .mjs（扩它的 MAKERS 会顺带把整个模板拉进一条从未跑过的门，属另一轮）。
- ④「`HorizonScopeNotice` 的 `risk` 那格随 B1 **删除**」—— 不删，**改写**：本版只有一根轴按档，
  删掉声明就等于把「波动率/夏普/基本面仍是全局」这半边抹掉。11 语言文案已改写为
  「按档的是回撤深度 + 仍是 60 日全局的三条轴」，并由
  `HorizonScopeNotice.test.tsx` 的同一谓词双向锁（写窄=回退成「四档共用」红，写宽=不点名全局三轴也红）。

**测量 SQL（可复跑，n=131）**：
```sql
with s0 as (select e.id, (x->'output'->'result'->>'content')::jsonb #> '{kline_json}' as kj
  from workflow_executions e, jsonb_array_elements(e.node_executions::jsonb) x
  where x->>'node_id'='t-scoring'
    and jsonb_array_length((x->'output'->'result'->>'content')::jsonb #> '{kline_json}') >= 91
  order by e.created_at desc limit 150),
src as (select id, kj, jsonb_array_length(kj) as len from s0),
wn(win) as (values (2),(5),(28),(60),(90)),
bars as (select s.id, w.win, (b.value->>'close')::float8 as c, b.o
  from src s cross join wn w, jsonb_array_elements(s.kj) with ordinality as b(value,o)
  where b.o > s.len - w.win - 1 and (b.value->>'close')::float8 > 0),
rets as (select id, win, c, lag(c) over (partition by id,win order by o) as pc,
  max(c) over (partition by id,win order by o rows between unbounded preceding and current row) as peak,
  row_number() over (partition by id,win order by o) as rn from bars),
per as (select id, win, stddev_samp(case when pc is not null then c/pc-1 end) as sigma,
  max(case when rn>=2 and peak>0 then (peak-c)/peak end) as maxdd from rets group by id, win),
d as (select win, maxdd/(sigma*sqrt(win)) as depth, sigma*sqrt(244.0)*100.0 as vol_annual,
  maxdd*100.0 as dd_pct from per where sigma>0 and maxdd is not null)
select win, count(*), percentile_cont(0.5) within group (order by depth),
  percentile_cont(0.9) within group (order by depth),
  percentile_cont(0.5) within group (order by vol_annual),
  percentile_cont(0.5) within group (order by dd_pct) from d group by win order by win;
```
上面这条即**出厂口径**（分母 `sigma` = 本窗口自己的 σ）。测「分母=60 日全局 σ」那一版只需
再加一个 CTE `g as (select id, sigma as sigma60 from per where win = 60)` 并把 `d` 里的
`sigma` 换成 `g.sigma60`，输出多取 `percentile_cont(0.95)` 与 `max(depth)` 两列 —— 两组读数
都在本文上面的表里，任何人可复跑推翻这条阈值（结论可证伪 = 本仓对参数的一贯要求）。


### B1 的第三条更正（写侧否决的**归因**问题，2026-10-05，代码已落地后自查发现）

上面 ④ 原写成「主链否决取『主链自算档』与**所选档节点档**的更严者」。这一句在实现后是**有问题的**：
主链的 `overall_risk` 来自 `portfolio-mgr.rhai:429-485` 的 **V54 算法分类**，而 `cls-risk-level-*`
来自 **prompt 规则分类** —— 种子自己就写着二者「**并非同一套阈值**（实测 6 处不同，见
risk-level.rhai 头部）」（`seed_stock_analysis.rs:4939-4942`）。把两个**不同规则体系**的档位直接
取 max ⇒ 每次否决入参的变化**无法归因**：它可能来自本批新增的深度臂，也可能只是两套阈值的固有分歧
（v55 影子比对实测档位一致率只有 64.4%）。本仓的纪律是「会改决策数值的改动必须可归因」，
所以这条不允许就这么上。

**改法（已采用）**：按档的增量必须**只在同一套规则体系内部**度量 —— 即拿「本档节点档」和
「**全局节点档**（同一个 prompt 规则体系、不注入按档输入）」比：

```
本档节点档 严格于 全局节点档  ⇒  差值只可能来自本档回撤深度臂（其余七条输入两节点完全相同）
                              ⇒  才把否决入参加严到本档节点档
否决入参 = max(主链算法档, 上面那个「纯按档增量」)
```

⇒ 两套分类器**永不直接互相顶替**，新增的那一次加严可逐条归因到深度臂（留痕 R-211 的文案
同时写明「比全局节点档更严 ⇒ 差值来自本档回撤深度」）。四路分支节点侧不受影响：它们本来就
读 prompt 规则体系的 `category`，只是从全局格换成本档格 ⇒ 同体系、纯增量。

测试面同步补一条**归因负控**：`overall_risk_mid` 比 `overall_risk_llm` 更严、但主链算法档本来就
更松时，只有「本档 > 全局节点档」才允许加严 —— 只注入本档格而不给全局节点档时必须**不加严**
（否则等于把「两套阈值的分歧」当成「按档的结论」发布）。


## 五十五、B4 的正确顺序（2026-10-05，写在 B1 落地之后，避免下一步先扩无证面）

B4 剩下的不是「把面板加四个 Tab」，而是**让每档的风险档在落库里可读出来**。现状与顺序：

1. **现在四档的风险档不在 `horizonDecisions` 里**。分支脚本 `portfolio-mgr-h-*.rhai` 收到
   `overall_risk`（v128 起＝本档节点档），但它只把它喂给 `pm_kelly_position`，**不回写到输出行**
   ⇒ 呈现层拿不到「这一档的风险档」，除非去读节点输出（历史行没有节点输出，只有落库 JSON）。
2. 所以 B4 的产出层是：**分支行增加 `overallRisk`（+ `riskWindowDays` / `riskDrawdownDepth`）**，
   再让逐档 Tab 显示。这与 v125「主档数值与逐档 Tab 同档两数」是同一形态（值取本档、来源随字段透出）。
3. ⚠ **顺序硬约束：先补 #34 的键集门，再加新键。** `decision.rs:734-738` 自己写着
   「该清单与四份分支脚本返回段**靠人工对齐**，仓内没有锁它的门」，且已有漂移实证
   （读侧 `reflection_stats.rs:49-69` 还留着 TS 已删的 `conf_lower_bound`）。
   在没有键集门的情况下往分支行加三个键，等于**把无证面再扩大一圈** ——
   将来 TS/落库/读侧三处任一漏改，症状是「那一档的风险行不显示」，而不是任何报错
   （本仓已为「snake 产端 × camel 读端 ⇒ 超短价位行从不显示」付过一次代价）。
4. 值域三载体同批（禁区 13 那条）：Rhai 产端键名 → TS `HorizonDecision` → 11 语言显示名。
   `riskWindowDays` 是**嵌层整数** ⇒ 注入落 i64，TS 侧按 `number` 读、Rhai 侧任何算术都要走 `num_of` 桥
   （v126 实测：Rhai 不把 i64 升格进 f64 形参 ⇒ 裸 `clamp` 会让整脚本降级）。
5. 撤声明的判据（B4 的「硬条件」）：某格的数据层事实变了才能改文案，且**改窄也要门**——
   `HorizonScopeNotice.test.tsx` 现在锁的是「risk 提到回撤深度 + 点名三条共用轴」；
   B2/B3 落地后 analyst/debate 两格改成按档说法时，同一谓词的形式要各自新建（不许复用一条串）。


## 五十六、B2 开工前的矩阵更正与分阶段（2026-10-05）

### ① 矩阵不能用手抄的那张 —— 成本基数从 20 变成 23

§五十四 B2 那张「超短 4 / 短 6 / 中 5 / 长 5 ⇒ 20 次」是我手写的。仓内**已有唯一权威**：
`Period::analyst_subset()`（`harness/src/holding_period.rs:437`），它由
「该档参与因子（必填 ∪ 条件）的属主 ∪ 共用因子属主」推导，且 :109-113 明写着
**手抄清单必然漂移**（历史事故：手抄的中/长档都漏挂 `value-investor`，而两档
`valuationBand` 都是必填 ⇒ 那条腿在子图里永远拿不到产出方）。所以 B2 只能按推导表建节点。

按推导实际算出的四档子集（节点 id 与 `VERDICT_FACTOR_OWNERS`/`COMMON_FACTOR_OWNERS` 逐字读出）：

| 档 | 子集（derived） | 数 |
|---|---|---|
| 超短 | a-market-analyst、a-sentiment、a-hot-money、a-catalyst | 4 |
| 短线 | a-market-analyst、a-sentiment、a-hot-money、a-lockup、a-catalyst | 5 |
| 中线 | a-market-analyst、a-hot-money、a-lockup、a-fundamentals、a-research、**value-investor**、a-catalyst | 7 |
| 长线 | a-market-analyst、a-fundamentals、a-research、a-policy、a-sector、**value-investor**、a-catalyst | 7 |

⇒ **合计 23 次/轮**（现状 10 ⇒ 2.3×），不是 20。三处与我手写表的实质分歧：
1. `a-news` 四档都不挂 —— 这是**既有裁定**（:115-117：公告方向通道已由 `a-catalyst` 的
   `eventCatalyst` 承载，同域双挂 = 重复计数），我的表把它列进超短是错的；
2. `value-investor` 中/长必须挂 —— 它不是那 10 个 `a-*` 之一，而是独立的价值评估 agent，
   我手写表里写的是 `a-fundamentals`/`a-research`，会漏掉必填的 `valuationBand` 属主；
3. 短线的 `supplyShock`（a-lockup）来自因子表，但那条腿的数据面本身还是坏的（任务 #23：
   来源语义是「占当日总解禁」而非占总股本）⇒ **B2 会让 a-lockup 在短线多跑一次调用，
   而那条腿仍会因 `latestEps`/`lockupFloatRatio` 缺席而 absent**。这不算浪费吗？算。
   所以阶段 1 之前先把 #23/#24 的口径拍掉，或者接受「先有产出的分析师、腿暂时不吃到」。

用户裁定是「四档都要跑子集」（不是「只跑候选两档」），2.3× 的成本量级与裁定同向；
但**基数从 20 改到 23 是我此前报给拍板的数字有误**，先更正再建节点。

### ② 分阶段（每阶段单独可验、单独可回滚）

| 阶段 | 内容 | 改数值？ | 换代 | 主要风险 |
|---|---|---|---|---|
| **B2-0** | 把「腿 ↔ 属主 ↔ 脚本取数键」三条表对起来：给 `pm_leg_signal` 的 factor→var 面补门（每条腿必须在脚本 `raw` 里有来源键，否则该腿恒 absent），已知缺席者进基线并逐条点名 | 否（只暴露既有缺席） | 否 | 门初值不为零 ⇒ 必须按基线门写 |
| **B2-1** | 图上按 derived 子集建 23 个档位分析师节点（id `{base}-{tierKebab}`；expert 复用同一份 md；`system_prompt` 追加档位段：档名 `Period::label_zh()` + 该档 VERDICT 契约 + 持有天数）；每档 `t-*-data` 仍共享一份（R-11 取数层不重拉） | 是（分支读到的 verdict 字段将来自本档分析师） | **v129** | 23 次调用是现网时延与费用的直接变化；`value-investor` 也在子集内 ⇒ 覆写链（v-value）要按档处理 |
| **B2-2** | 消费面 rewire：`debate-*` / `analyst-brief` / `data-quality` 的 `*_tool_calls` / `research-mgr` 的 `context_sources` / 前端 `AnalystReportGrid` 与 evidence 权重表，全部改为**按档分组**读 | 是（f1..f13 与各分析师 confidence 的来源变化） | v130 | 这一步与 **B3 辩论按档**天然重叠 ⇒ 顺序上把 B3 并进 B2-2 更省一次换代 |
| **B2-3** | 撤 `analyst` 那格共用声明（B4 的判据一致性），逐档 Tab 显示该档的分析师清单 | 否 | v131 | 必须先有 #34 的键集门（§五十五 第 3 条） |

### ③ B2-0 是真门而不是形式：它现在就该红

分支脚本的腿信号取自脚本内手写的 `raw`（例：`portfolio-mgr-h-mid.rhai:61-71` 只列了
`flowPersistence` / `consensusEps` / `pePercentile` / `fScore` 四个键），而腿清单由 Rust 表
注入 ⇒ **两边没有任何锁**。已知后果就在任务清单里：中线 `expectationRevision` 恒 absent
（缺 `latestEps`，#24）、短线 `supplyShock` 恒 absent（量纲/窗口，#23）。
⇒ 门的形式：对每档，`legs[].factor` 必须能在该脚本的 `raw` 键集里取到；取不到的进基线名单
并要求点名理由（缺席可以，静默不行）。这条门同时是 B2-1 的接线面（新节点的 verdict
路径就按 `factor → owner → 节点 id` 生成）。


### ④ B2-1 的实际面积（2026-10-05 普查，只读 agent 全仓扫）

结论：**分析师节点 id 被 12 处按完整字面量精确匹配**，改 id 全部**静默失效**（不报错、只是那一格永远空）：
种子内 :2920 `extra_supplies` match、:2931 `market_regime` 注入、:2937 `max_tool_rounds`、
:3042-3051 analyst-brief 的 `a_*_raw` 短名桥、:4616-4695 data-quality 的 10×4 组映射、
:5307 portfolio-mgr `catalyst_level`/`untrusted_*`、:5583/:5636 `flow_persistence ← a-hot-money…`、
:5808 `a-catalyst → portfolio-mgr` 边、:5881 pace-calc `llm_events ← a-catalyst.content`、
:6545-6558 内嵌 Rhai `ctx.results["a-market-analyst"]`（断则**静默退 50.0 中性分**）、
`stock_workflow/decision.rs:3379-3395 expert_mapping`（断则仪表盘分析师栏直接空）、
`astock-data/src/quality.rs:137-155` 必采清单（断则 `run_quality_gate` **放水到 B 级**）。
另有 3 道门会**响亮红**（是好事，先跑它们）：`scripts/check-horizon-weight-parity.mjs:123-165` 三处键集严格相等、
`seed_consistency_tests.rs:1908-1935`（它按文本抽 `let analysts = [`，**每行恰 3 个字面量** ⇒ 把数组改成循环生成当场失效）、
`seed_stock_analysis.rs:9683-9730` 成对断言。前端 4 张手抄表（`AnalystReportGrid.tsx:18-27`、
`AnalysisProgress.tsx:19-28`、`stockWorkflowChatBridge.ts:13-22`、`dataQualityDiagnosis.ts:25-34`）
与 11 语言 × 3 套 key（`analyst.a-*` / `analyst.<去前缀>` / `phase.a-*`）也要同批。
库内读数：`analyst_feedbacks` 0 行（没有历史权重按 id 存），历史 `node_executions` 196/199 行含旧 id
⇒ **旧记录不可迁移，但也不阻塞改造**（新代 v129 起按代筛样，属 #12/#31 那条线）。

**关键设计决定（这条决定形状，而不是决定改几处）**：不要用「把后缀写死在每个 id 字面量里」的做法
—— 那是把 12 个隐式契约换成 12 个新的手抄点，下一轮再加档还得再改 12 处。正解是**两条唯一函数**：

```
analyst_node_id(base: &str, tier: &str) -> String   // 产端：a-fundamentals + mid → a-fundamentals--mid
analyst_base_of(node_id: &str) -> Option<&str>       // 读端：剥档位后缀，得到 base
```

上面 12 处**全部**改为经由这两条函数（产端由 `analysts × analyst_subset()` 生成，读端按 base 匹配），
并配一条门：任何按 `a-` 字面量精确匹配的地方都必须走 helper（可静态扫：源码里出现的 `"a-xxx"`
字面量必须 ∈ 白名单，白名单=base 域 + helper 自身）。
⇒ 消费者一侧的键名域（`a_*_raw`、data-quality 的 40 键、前端 4 张表的行 key）**保持不变**，
变的是节点 id 的**生成与归属**，静默失效面从 12 降到 0。

**落地顺序（缺一格就出现死端窗口）**：
1. 先加 helper + 那条「字面量必须走 helper」的门（此时 helper 是恒等映射，图不变 ⇒ 门先绿）；
2. 把 12 处逐个改经由 helper（每改一处跑一次门，绿了再下一处）；
3. 再建 23 个档位节点（`analysts × analyst_subset()`），**同批**把 `analyst_subset` 从惰性注入变成
   真消费（`pm-h-*.rhai` 读 `branch_json["analysts"]`，并把 verdict 字段路径按 `{owner node id}` 生成）；
4. 前端 4 张表 + 11×3 套 i18n key 同批（否则面板四档格恒空 = 第 3 步白做）；
5. v129 换代 + `--dump` 类读数核对（重跑一只票，确认四档各拿到自己的分析师报告）。

⇒ 这是一个**批次**，不能拆成「先建节点、下次再接消费者」。B3（辩论按档）与第 3/4 步同层，
建议并入同一批以省一次换代。


## 五十七、三条裁定（2026-10-05 用户拍定）

### ① #45「f4 / risk_bias 也按档」= 要做

形态：**把选档段上移到风险分类之前**（现在 `overall_risk` 在 :429-485，选档在 :1973，
`risk_bias` 在 :1798、f4 在 :498/:513）。选档只依赖四路分支输入（`branch_tiers` +
`branch_row_ok` + `horizon_const`），**不依赖 posterior**（只有「四路全缺席时按后验阈值定档」那条
else 分支依赖，上移时把该 else 留在原位、选档成功与否两段分别处理即可）。
⇒ 一次改动同时让 f4 / risk_bias / kelly / cap 全部按档，不再有「半按档」的自相矛盾。
⚠ 会同时改三处数值（f4 因子、后验、action 阶梯），**必须换代 v131** 并跑既有逐字锁门
（`PRIMARY_TIER_SRC` / `POSITION_SOURCE_SRC` 会比到漂移——它们一个是原文、一个是剥注释骨架，两侧口径不同）。
顺序上放在 B2-1 之后（分析师按档会改分支输入，先稳定输入再动装配顺序，避免一次改两件事没法归因）。

### ② #10 妖股判据：阈值**已有权威表，不要新建第二份**

用户给的「超短 ≥10% / 短 ≥20% / 中 ≥30% / 长 ≥40%」与
`analysis-engine/src/mover_recall.rs:41-44` 的
`("mover_gain_ultra_short", 10.0) / ("mover_gain_short", 20.0) / ("mover_gain_mid", 30.0) / ("mover_gain_long", 40.0)`
**逐字相同** —— 那批变量已在面板可调、且 `TierRule.gain_pct` + `window_days = Period::default_holding_days()`
都已就位（窗口涨幅达标漏检那一轮的成果）。⇒ 妖股判定**复用同一张表**，新增的只有三件事：
1. **归约口径**：反思时按「分析时价格 → 反思时价格」的区间涨幅判，而不是「本档持有期内的累计涨幅」
   （`window_cum_gain_pct` 判的是后者）。两者含义不同：前者是「这只票在这次分析的持有期真涨了多少」，
   后者是「窗口内哪几天涨够了」。实现点应在反思链（`stock_workflow/reflection.rs` 的快照段已有
   `period_high/period_low/max_drawdown_pct` 与 `expectedHoldingDays`，价格两端都拿得到）。
2. **落库形态**：标签是**按档**的 ⇒ 一条分析可产生「超短妖、长不妖」两种结论，
   所以它挂在**逐档反思行**上而不是分析主行（与「反思每档一行」那条既有裁定同形）。
3. **不伪装缺席**：持有期未满（反思时价格还没走到档期末）⇒ 该档妖股位**留空并注明未满**，
   不得用当前价当到期价算一个数（那是「近似数据顶替」，用户裁定③禁的形态）。
⚠ 禁区 12：不得再写一份 `if gain >= 10.0` 的字面量 —— 从 `TierRule` / 面板变量读。

### ③ #47 非中文 locale **接受改数制**
即允许「亿/万」在 en/de/ru/… 下变成 B/M 等 SI 紧凑计数（会改现网显示）。
实现约束：zh-CN / zh-TW / ja 三条**逐字不变**（现口径：1e8→2 位小数「亿」、1e4→0 位小数「万」），
所以要做成「按语言的 (除数, 单位, 位数) 表」而不是 `Intl compact`（实测 Intl 在 zh 下给
`1234.57万` / `1亿`，与现网 `1235万` / `1.00亿` 不一致）；表要有门（三语言逐字锁 + 其余语言
键集齐备），且 `scripts/.i18n-allowlist.json` 里 format.ts 那条**按行号豁免要同批撤掉**
（否则豁免会替新表挡掉未来的漏译）。


### B2-1 实施细化（helper 落位与分隔符，2026-10-05 定）

两条 helper **两种语言各一份**，且都必须紧邻各自的档位权威表（同一文件里才拦得住手抄）：

| 面 | 位置 | 形态 |
|---|---|---|
| Rust | `crates/harness/src/holding_period.rs`（`analyst_subset()` 旁边） | `pub fn analyst_node_id(base: &str, tier_snake: &str) -> String` / `pub fn analyst_base_of(node_id: &str) -> Option<&str>` |
| TS | `src/lib/stock-analysis-utils.ts`（`HORIZON_T_SUFFIX` 旁边） | `analystBaseOf(nodeId): string`（剥后缀）/ `analystNodeId(base, tier): string` |

分隔符定 **`--`**（`a-market-analyst--mid`）而不是单横线：base id 自身含单横线
（`a-market-analyst`、`a-hot-money`），单横线拼接后「剥到哪一段」没有唯一解；`--` 在 base 域里不出现，
`strip_suffix` 唯一可逆。⚠ 同一条门要顺手锁「base 域里禁止出现 `--`」，否则这个前提会在未来某个新分析师身上失效。

读端一律 `analyst_base_of(id)` 再查表 ⇒ **消费者侧键名域零变化**：
data-quality 的 40 个键、`analyst-brief.rhai` 的 `a_*_raw`、前端 4 张表的行 key、
11 语言 × 3 套 i18n key（`analyst.a-*` / `analyst.<去前缀>` / `phase.a-*`）**都不新增条目**
—— 这是选 helper 而不是选「后缀写死 12 处」的直接回报。
唯一必须新增的是「档位维度」本身：报告卡要按档分组显示（前端读 `(base, tier)` 二元组），
以及 `astock-data/src/quality.rs` 的必采清单要接受带后缀的 id（MCP 那条链的 id 由 LLM 传入，
剥不到 base 就落 `_` 分支 ⇒ 放水 B 级，是普查点名的静默失效之一）。

⇒ 因此 **B2-1 不动 `let analysts = [...]` 的字面量形态**（它是 base 域的唯一声明，也是
`seed_consistency_tests.rs:1908-1935` 那道门的抽取面：它按文本抽这个数组、要求每行恰 3 个字面量，
改成循环生成会**当场把门的抽取面打空** —— 门不会红，只会瞎）。
按档展开写在它之后的独立循环里：`for (base, title, expert) in analysts { for tier in Period::ALL { if 子集含 base { 建 `analyst_node_id(base,tier)` } } }`，
子集判定直接调 `Period::analyst_subset()`（Rust 侧同 crate 可见），不在种子内重写一份矩阵。


## 五十八、接续执行单（2026-10-05 定序 3→2→1，可直接照做不必重新普查）

⚠ 本节的意义是把三件事的**定位与验证面**固化下来：接手者不必重跑普查、不必重读 §五十六/§五十七。
每条都带「先跑什么」，因为这三件都会碰 `include_str!` 资产 ⇒ 阶段内只做定点门，全量留到最后一次。

### 第 3 件（小项收口，零/低数值影响）

1. **#46 `compute_attention_score_impl` 墙钟锚点**
   位置：`src-tauri/crates/astock-data/src/mcp_tools.rs:4639`，损坏点在 :4656-4657
   （`Utc::now() - 90d` 当「近 90 天研报覆盖」的窗口锚）。上游 `get_research_reports` 已按 as-of
   截断 ⇒ 分母仍是今天 ⇒ 回放恒判「零覆盖」。性质是**锚点错**，不泄露未来数据。
   改法：窗口锚改用既有 as-of 通道（同文件既有工具的 `asof_probe` 口径，别新开一份），无 as-of 时才回落 now。
   验证：`cargo test -p axagent-astock-data`；并补一条**同 `as_of_date` 两次读数必须相同**的回归
   （这条才是判据，编译门检不出锚点错）。
2. **#5 两条零设施列的退役**（`model_version`、`data_snapshot_id`）—— **已落地，见 §五十九**。
   原计划里「`extras.rs` 登记待删」这条**是错的**：`extras.rs` 只声明实体表达不了的 DDL（L2），
   没有「待删列」这一层；孤儿豁免规则（`ORPHAN_EXEMPT`）是**表级**的，也管不到列。
   实际机制是「删实体列声明 ⇒ 引擎在存量库上判出 `DropColumn` 孤儿，而启动期 `bootstrap_options()`
   是 `additive_only = true` + `max_destructive_per_run = 0` ⇒ 只登记不执行」，详见 §五十九 的处置口径。
   反思表侧缺口（`stock_reflections` 没有 `template_version` 列）**不在本件里**：那是 #12/#31 的筛样裁定，别顺手加列。

### 第 2 件（#45：f4 / risk_bias 按档 ⇒ 选档段上移，v131）

要动的顺序：把 `portfolio-mgr.rhai` 的 **选档段**（`horizon_const` :1911、`branch_tiers` :1950、
选档循环 :1962-1972、`branch_picked` :1973）整体上移到 **风险分类之前**（:411 之前），
`horizon_source` 的 else 分支（四路全缺席时按 `effective_posterior` 阈值定档 :1979）留在原位分段处理
—— 因为 posterior 依赖 f4、f4 又依赖风险档，这条 else **不能**参与被上移的那一段。
之后 `overall_risk` 直接按所选档的节点档取（B1 的 `risk_for_veto` 就地升级成 `overall_risk` 本身，
`axisScope` 那套同体系比较的归因纪律照用），f4（:498/:513）、`risk_bias`（:1798）、kelly（:2088）、
cap 与 `pm_risk_veto` 一起按档。
⚠ 三条已知的门必须先红后绿：`portfolio_mgr_horizon_lock_rhai.rs` 的 `PRIMARY_TIER_SRC` /
`POSITION_SOURCE_SRC` 两条逐字锁（**两侧口径不同**：一条比原文、一条比剥注释骨架）、
`rhai_registry.rs` 的整段求值门（合成 scope 里逐档数值会变）。换代 **v131**，
理由写进 `seed_stock_analysis.rs` 的版本块：本版起 f4/后验/action 阶梯随所选档而变。
### 第 1 件（B2-1 续：12 处改经由 + 字面量白名单门）

已落地（本轮，带读数）：Rust 侧 `ANALYST_TIER_SEP` / `analyst_node_id` / `analyst_base_of`
（`holding_period.rs`，doctest 2 passed）+ TS 侧 `analystNodeId` / `analystBaseOf`
（`stock-analysis-utils.ts`，typecheck / dprint 0）。**目前无人调用**，所以 B2 不算闭环。
待做（顺序即依赖）：① 种子内 6 处 `match *id` / 硬编码 `context_sources` / analyst-brief 短名桥
改读 base（helper 已就位，键名域零变化）；② 门：`"a-*"` 字面量白名单（白名单=base 域 + helper 自身 +
测试夹具），**初值必须为 0 违规**否则它就是给现状背书；③ 非种子读端三处
`decision.rs:3379 expert_mapping`、`astock-data/quality.rs:137-155` 必采清单（剥不到 base 会放水 B 级）、
`evidence_weight.rs:280-345/915-945` 的 HashMap 键（miss 退 1.0 是静默放大器）；
④ 前端 4 张表按 `(base, tier)` 二元组分组；⑤ 建 23 节点 + `pm-h-*.rhai` 真消费 `branch_json["analysts"]`
（终结惰性接线）；⑥ v129。
门侧会响亮红的三处先跑：`check-horizon-weight-parity.mjs`、`seed_consistency_tests.rs:1908-1935`
（**别把 `let analysts = [...]` 改成循环生成**，它是那道门的文本抽取面）、`seed_stock_analysis.rs:9683-9730`。

## 五十九、#5 零设施列退役：`model_version` ×2 + `data_snapshot_id`（2026-10-05 落地）

### 判据与裁定依据

退役的**不是「一个没人读的列」，而是「一个从没被赋过值的概念」**。三条实测支撑（缺一条就不该删，而该去补）：

1. **写侧**：全仓 **20 个写入点**（`model_version` 11 + `data_snapshot_id` 9，按 HEAD 逐文件 `grep -c` 计数）
   **逐字**都是 `Set(None)` / `None`，没有一处赋实值（`core.rs` 两建行点、
   `hooks.rs` chat 通道、`reflection.rs` 反思建行 + 一处测试 Model 字面量、`trade_intent.rs` 五个建行点
   + 测试建表 SQL）。原 `axagent-batch-rerun.rs` 的自陈注释「全仓只有 `Set(None)` 写入点、无实值赋值」
   是这条的既有记载，本次连注释一起清掉。
2. **读侧**：`grep` 覆盖 `.rs` / `.rhai` / `.ts` / `.tsx` / `.mjs` / locales / seed 变量表 ⇒
   对 `model_version`、`data_snapshot_id`、`modelVersion`、`dataSnapshotId` **零消费**（`.rs→.md`
   里剩下的 `image_gen.rs` 局部同名变量、`VersionPagination.tsx` 的 `modelVersions` 是无关物）。
3. **现网**（`scripts/pg-connect.mjs` 正门，退役前读数）：
   `stock_analyses` total=**80** / `count(model_version)`=**0** / `count(data_snapshot_id)`=**0**；
   `stock_reflections` total=**19** / `count(model_version)`=**0** ⇒ 删列**不丢任何数据**。

`data_snapshot_id` 的名目本意是「关联 L2 disk-cache 快照」（列注释原话），而 L2 侧的键是
`indexing_triggers.rs` 的 `snapshot_id`（root 路径稳定哈希），**两边从未接上**，属设计性缺席而非缺陷。

### 范围的一处主动扩大（按既有裁定合推，非静默）

`stock_reflections.model_version` 同批退役。依据：它与 `stock_analyses.model_version` 是**同一个概念**
（列注释同为「所用 LLM 的版本标识」）、同一套零设施证据（写侧 `Set(None)`、现网 19/19 NULL），
只退一半会留下「同一字段一张表有声明、一张表没有」的半简半繁形态；
而 §10-3 P2-G 的「填真值」路线已被本次裁定取代（`PLAN-analysis-data-repair.md` D7 已写状态订正）。
**不在本次范围**：`stock_reflections` 缺 `template_version` 列 —— 那是 #12/#31 的筛样裁定，本次没顺手加列。

### 处置口径：删声明 ≠ 删物理列（这一条是本节的主要价值）

本仓不走 migration（`MIGRATIONS` 已清空），正门是声明式对账。§五十八 原写「删实体列属性 +
`extras.rs` 登记待删」**是错的**，两处都错：

- `reconcile/extras.rs` 是 **L2 例外清单**，只承载实体表达不了的 DDL（FTS 虚表 / GIN / 复合索引 /
  CHECK / 生成列 / 函数），没有「待删列」这一层；
- 里面的 `ORPHAN_EXEMPT` 是**表级**所有权声明（`Prefix` 族 / `Exact` 个体），管不到列。

真实机制（读码 + `apply.rs::bootstrap_options` 实证）：删掉实体列声明 ⇒ 引擎 diff 在**存量库**上判出
`DropColumn` 孤儿，而启动期 `bootstrap_schema` 走 `additive_only = true` + `max_destructive_per_run = 0`
⇒ **只登记、不执行**（闸 4 还要求导出证据，产出端在 `reconcile/evidence.rs`，且只做 PG）。
⇒ 所以本次改完的效果是：**新库（含 `create_test_pool`）不再有这两列**，存量 PG/SQLite 库的物理列
**仍在**，`p5_engine_takeover -- --diff` 与 `db_config` 的结构状态会一直把它们报成孤儿。
要真删需走离线路径（导出证据 + 非 additive_only 的 `apply::cycle`）——**那是破坏性动作，
2026-10-05 用户点头后已执行**（见下方「物理层收口」）。
（现网两列全 NULL，导出的证据文件会是 0 行 JSONL，恰好就是「本来没数据」的自证。）

### 物理层收口（2026-10-05 执行，含执行前后的实测）

声明层退役后，存量 PG 库里的物理列**已在用户授权后删除**。走的是**限定作用域的 SQL** 而不是
`p4_apply_probe <url> --execute --export-evidence`，理由是如实的：那条引擎路径会执行
**整个 plan**，而当前共享工作树里还有他人的实体改动未提交 ⇒ plan 里除这三列外还会有别的
破坏性项，一次 apply 会把没授权的东西一起做掉。引擎路径的价值（graveyard 证据 + `_ax_schema_audit`
留痕）在这里换不成「只删被点头的那三样」，所以选了精确范围。

删前三道前置实测（`scripts/pg-connect.mjs` 正门）：
1. **无依赖**：`pg_indexes` 里没有任何索引定义提到这三列；`information_schema.columns` 的
   `is_generated` 三行全 `NEVER`；`pg_depend` 查到的 depend 对象只有同表**其它列**的索引
   （`relkind='i'`、`deptype='a'`），无视图/物化视图/规则/触发器依赖。
2. **零数据**：`stock_analyses` 80 行 / `count(model_version)`=0 / `count(data_snapshot_id)`=0；
   `stock_reflections` 19 行 / `count(model_version)`=0。
3. **列定义快照**（可复原的最低成本依据）：三列均 `text`、`is_nullable=YES`、无 default、无约束。

执行：单事务三条 `ALTER TABLE … DROP COLUMN`（`BEGIN; … ; COMMIT`，输出四个语句回执）。
复验：`information_schema` 里这三个 (表,列) **零行**；`count(*)` 仍是 **80 / 19**（未动行数）；
`stock_analyses.template_version` 非空 **37**、`as_of_date` 非空 **80**（与删前同量级，未被牵连）。
⚠ 如实标注一处口径缺口：`stock_reflections.horizon` 的**删前**读数本轮没单独取（现在 0 非空），
只能引 §P0-补充 的既有实测「19/19 全 NULL，那 19 行早于逐档盖章特性」为旁证 —— 删另一列
不可能把这一列写成 NULL，但「没取删前读数」这句话要留在纸上，不拿推断冒充实测。

⇒ 现在**声明层与物理层一致**，`p5_engine_takeover -- --diff` 不再报这三条 `DropColumn` 孤儿。
⚠ 反面约束（给未来）：**从当前树之前构建的旧二进制**若被启动，其 INSERT 仍会点名
`model_version` / `data_snapshot_id` ⇒ 报「列不存在」。当前工作树已无这些写入点（本节改动清单），
所以只要跑的是本树构建出来的版本就安全；回滚到退役前的提交而不加回列，会直接写库失败。

### 改动清单（20 个写入点 + 3 处实体声明 + 2 行测试建表 SQL + 1 条自陈注释）

| 文件 | 改动 |
|---|---|
| `crates/entities/src/stock_analyses.rs` | 删 `model_version`、`data_snapshot_id` 两列及其文档注释 |
| `crates/entities/src/stock_reflections.rs` | 删 `model_version` |
| `src/commands/stock_workflow/core.rs` | 两建行点各删 2 行（running 占位行 + 快速/自动化建行点） |
| `src/commands/stock_workflow/hooks.rs` | chat 通道删 2 行 |
| `src/commands/stock_workflow/reflection.rs` | 反思建行 ×2、测试 Model 字面量 ×2（共 4 行） |
| `crates/analysis-engine/src/trade_intent.rs` | 五个建行点 `Set(None)` ×10 + 测试 `CREATE TABLE` 里两列 |
| `src/bin/axagent-batch-rerun.rs` | 删掉「`model_version` 恒 NULL ⇒ 无需处理」那条自陈注释（随列退役） |

同批的文档侧：`PLAN-four-horizon-workflow-alignment.md` §四 需求④ 行、§10-3 P2-G、§五十八 第 3 件
条目改为指向本节；`PLAN-analysis-data-repair.md` D7 加状态订正、§(c) 那条括号随之改写。
被本次顺移影响、已按当前磁盘读数订正的行号引用：`stock_analyses.rs:51,57,94,113`、
`stock_reflections.rs:7-70`、`reflection.rs:3439`、`reflection.rs:1142,2204`。

### 读数

- `cargo check -p axagent-entities -p axagent-analysis-engine -p axagent --all-targets` ⇒ **EXIT=0**，
  12m33s，输出含 `Checking axagent-entities` / `Checking axagent-analysis-engine` /
  `Compiling axagent`（按本仓口径：没有这几行就可能是粗 mtime 复用旧二进制，绿了也不作数）。
- `node scripts/check-single-source-facts.mjs --ci` ⇒ **EXIT=0**，
  LOCATED 460 / NONLOC 25 / EXTERNAL 43 / **BROKEN·越界·空行·控制流 0**；
  软判据「疑似腐烂」17 处**逐条筛过目标文件**，无一指向本次改动的 6 个文件
  （`.rs` 侧唯一指向被改文件的引用是 `reflection_stats.rs:719 → stock_reflections.rs:14`，落在删除点之前，未移位）。
- fmt / 定点 clippy / 受影响 crate 测试 / 主 crate lib 测试（三条都是本轮实测，非推断）：
  · `cargo fmt --all --check` ⇒ **FMT=0**；
  · `cargo clippy -p axagent-astock-data -p axagent-entities -p axagent-analysis-engine --all-targets
    --all-features -- -D warnings` ⇒ **CLIPPY_A=0**（第一轮在此短路于 `mcp_tools.rs` 的 `len_zero`，
    见本节末「顺带补验」那条；修后重跑才拿到 0，**首轮报的 1 个 error 不能用来推定其余包干净**）；
  · `cargo clippy -p axagent --lib --bins --all-features -- -D warnings` ⇒ **CLIPPY_B=0**；
  · `cargo test -p axagent-entities -p axagent-analysis-engine --no-fail-fast` ⇒ **599 passed / 0 failed**
    + entities 1 passed（`trade_intent.rs` 那 5 个建行点与测试建表 SQL 删列后仍绿，是本批最硬的证据）；
  · `__TAURI_WORKSPACE__=true cargo test -p axagent --lib --no-fail-fast` ⇒ **534 passed / 1 failed**，
    唯一那条是既有红 #26
    （`commands::stock_workflow::serenity::serenity_extract_tests::extract_recovers_candidates_from_strict_mode_report`），
    与本批改动无关（列退役不触及该路径；同名测试在 §五十三 之前就是 1 failed）。
    ⚠ 取数姿势的自纠：上一轮我在 `cargo test … | grep …` 后接 `echo "TEST=$?"`，拿到的是 **grep 的退出码**
    而不是 cargo 的 ⇒ 那行 `TEST_LIB=0` 是假绿。本轮把 `$?` 换成 `${PIPESTATUS[0]}` 并直接读 `test result` 行。

⚠ 本节**不需要** bump `TEMPLATE_VERSION`：没碰任何被 `include_str!` 的资产（`.rhai` / `agency_experts/*.md` 均零改动）。

### 顺带补验：B1（v128）「全局三量逐位不变」这两条当时只靠读码断言，本轮才取到证据

定点 clippy 一跑就短路在 `axagent-astock-data`（我 B1 改动所在），把那条违规清掉之后，才顺手把 B1 的
等价性主张补成实测 —— 记在这里，免得它继续停在「看起来会这样」：

1. **K 线尾切方向**（决定 `GLOBAL_LOOKBACK=60` 切出来的是不是同一批 60 根）：
   `get_klines_with_adj` 的缓存命中路径 `crates/astock-data/src/lib.rs:2942` 就是
   `let start = klines.len().saturating_sub(limit); klines[start..]` ⇒ **升序、尾部=最新**是既有口径。
   本轮把 fetch 从 60 提到 120、再取尾 60 根喂全局三量 ⇒ 落到的仍是同批 60 根，
   `ann_vol_pct` / `max_dd_pct` / `sharpe` 与退役前逐位一致（不是「近似一致」）。
2. **退化样本（2 根 K 线 ⇒ 1 个收益率）**：`harness::indicators::stddev_sample` 对 `n < 2` 直接
   `return 0.0`（`indicators.rs:183-185`），HEAD 是把长度 1 的切片照样喂进去 ⇒ 两边同得 `0.0`。
   本轮新加的 `returns.len() >= 2` 守卫**没改变任何数值**，只是把那条隐含路径写成显式。

⚠ 门禁形态的一条实录：`cargo clippy -p <若干包> -- -D warnings` 的 `-D warnings` 会落到
**每一个被编译的 crate**（不只是 `-p` 选中的那些），所以它在 `axagent-astock-data` 首个 error 即停，
其后的包**从未被检查** —— 不能由「只报了 1 个 error」推定其余干净（本仓已记过同型教训）。
那条 error（`mcp_tools.rs` 的 `returns.len() >= 1` ⇒ `clippy::len_zero`）是**我 B1 改写时引入的**，
HEAD 同文件零命中（`git show HEAD:… | grep -c`），已按 `!returns.is_empty()` 修：
对切片二者严格等价，且 `returns` 由 `windows(2)` 产出、无 NaN 分支，属机械建议可直接套用的那类。

## 六十、#45（v129）：选档段上移 ⇒ 主链风险口径整条按档（2026-10-05 落地）

### 为什么这次上移**不再**是自指环（这是 §五十八 那条设计的前提，必须先说清）

:830 那段 2026-10-01 的根因记录判死的是「用 `time_horizon` 决定 f5 腿权重」，它错在两点：
前向引用 + **循环**（`time_horizon` 由 `effective_posterior` 定，而 posterior 含那条腿）。
Q1=C（v124）之后这条环**断了一半**：有分支结论时 `time_horizon` 来自**四路分支的行**（注入的输入），
与主链 posterior 无因果关系 ⇒ 把它挪到风险分类之前不成环。环只可能从**兜底路径**回来，
所以兜底那条（`effective_posterior` 阈值映射）**留在原位不动**：

| 段 | 位置 | 为什么在这里 |
|---|---|---|
| 选档（`branch_row_ok` / `branch_tiers` / 选档循环 / `branch_picked`） | **上移**到风险分类之前 | 它只读注入的四行分支结论，不读主链任何中间量 |
| 按档收紧（`risk_tier` / `depth_upgrade` / `risk_tier_applied` / `risk_llm_eff`） | 新增，紧贴算法分类之后、`risk_rank_val` 之前 | 收紧的对象是 `overall_risk`，必须在它的**第一个消费者**之前 |
| `days_for` / `time_horizon` 的阈值兜底 / 乘数 / 期望天数 | **留在原位** | 兜底要读 `effective_posterior`，提前就是前向引用 + 成环 |

⇒ 于是 f4 惩罚强度（`risk_rank_val`）、f4_signal 的 prompt 口径、`risk_bias`、仓位 cap、
`pm_risk_veto` **全部自动跟着走**，一处定义、零新增参数（R-11：不是「一个算法 + 四套参数」）。
四路全缺席（存量库未重播种）⇒ `risk_tier_applied` 恒假 ⇒ 与 v128 逐位相同。

### 两条轴都必须单向（本版我自己写错的一处，如实记）

`overall_risk` 那条沿用 v128 的只升不降。但 `risk_llm_eff`（f4_signal 读的 prompt 分类）**首版写成
`if tier_llm_usable`** —— 只要本档有值就采用。那会让「本档比全局**更松**」时 f4 拿到一个更漂亮的
证据（低风险 ⇒ +0.30），等于**从证据轴把风险放松**，而 `overall_risk` 那条却守住了单向。
两条轴共用同一个 `depth_upgrade` 后才是完整的纪律 ⇒ 改成 `if depth_upgrade { risk_tier } else { overall_risk_llm }`。

⚠ 这条是自查抓到的，不是门抓到的；而它顺带暴露了负控自身的形态错误：负控首版只拆**第二道**
比较（本档 vs 算法档），结果 mutant 与原判据同结果 —— 因为第一道 `depth_upgrade` 先把它挡住了
（本档「低风险」 vs 全局「中风险」根本不同向）。⇒ **负控必须拆在「真正决定该断言的那道守卫」上**，
拆错位置的负控是假绿的一部分。现在两条比较一起拆（`RISK_TIER_SRC` 的 mutant），B 情形才真的翻车。

### 改动清单

| 文件 | 改动 |
|---|---|
| `src/commands/portfolio-mgr.rhai` | 选档段整体上移（字节不动，43 行）；风险分类后新增按档收紧段；`f4_signal` 改读 `risk_llm_eff`；**退役** `risk_for_veto` / `two_strings` / 原 `tier_risk` 四路 switch；`pm_risk_veto` 回到 `overall_risk`；R-211 条件改 `risk_tier_applied`、文案扩到「f4/risk_bias/仓位 cap/否决同源」、并新增 `overall_risk_algo`（收紧前留值） |
| `seed_stock_analysis.rs` | `TEMPLATE_VERSION` **128 → 129** + 版本块；规则追溯码表 R-211 文案改「主链风险档按所选档收紧」 |
| `crates/rt-workflow/tests/portfolio_mgr_horizon_lock_rhai.rs` | `PRIMARY_TIER_SRC` 一拆为二（`TIER_PICK_SRC` + 定档段），求值门按新顺序拼接；新增 `RISK_TIER_SRC`（剥注释骨架）与 `RISK_RANK_FN`；新增反向锁⑤「选档必须早于风险分类」+ 其搬运负控；新增 `v129_risk_tier_tightens_both_axes_one_way_only`（A–E 五情形）与 `v129_one_way_guard_is_what_stops_the_looser_tier`（两条守卫一起拆的负控） |
| `src/commands/stock_workflow/rhai_registry.rs` | 整脚本门改名 `v129_…_scopes_the_whole_main_chain_one_way_only`；**源码锁翻向**（必须恰有一处 `pm_risk_veto(final_action, overall_risk)`、`risk_for_veto` 全脚本零出现、必须有 `overall_risk = risk_tier;`） |
| 呈现层 | `HorizonScopeNotice.tsx` 文档与 11 语言 `horizonScopeRisk` 各追加一句「f4 证据 / 风险偏置 / 仓位上限 / 否决亦按所选档，只升不降」；谓词加两条必需词 + **负控 3**（v128 那句当时正确的文案现在必须被拒）；`RiskMatrix.tsx` 注释同步 |

### 编号更正

§五十七/§五十八 把本项记为 **v131**、B2-1 记为 v129，那是按「B2 先做」写的；用户随后定序 3→2→1，
本项先落地 ⇒ 取下一个连续号 **v129**，B2-1 顺延 **v130**，#45 的 v131 号作废。版本号的判据是
「连续、单调、每号对应一次重播种」，不是「按计划表里的编号」。

### 读数

- `cargo test -p axagent-rt-workflow --test portfolio_mgr_horizon_lock_rhai` ⇒ **11 passed / 0 failed**
  （含新增两门与逐字锁的三条负控；中途两次真红都是门自己的错：Rhai 不认三元 `? :`、
  以及 `format!` 里插入的 Rhai 源码引号未转义 ⇒ Rust 字符串提前结束）。
- vitest `HorizonScopeNotice.test.tsx` ⇒ **4 passed**。
- node 门 ⇒ `check-horizon-weight-parity` / `check-rhai-numeric-typing` / `check-horizon-decision-keys`
  / `check-input-mapping` / `audit-inject-coverage` / `check-i18n-key-parity` / `check-i18n-untranslated`
  **全部 0**；`npm run typecheck` **0**；`dprint fmt` **0**。
- `check-single-source-facts --ci` ⇒ **EXIT=0，BROKEN 0**；软判据 18 处逐条筛过，
  无一指向 `.rhai` 侧被改的段落（`.rs` 里没有按行号引用 `portfolio-mgr.rhai` 的条目）。
- 主 crate `--lib` 全套 + `clippy` + `cargo test -p axagent-rt-workflow --all-targets`（fmt 后全部复跑）：
  · `cargo test -p axagent --lib --no-fail-fast` ⇒ **535 passed / 1 failed**（EXIT=101），
    唯一那条是既有红 **#26**（`serenity_extract_tests::extract_recovers_candidates_from_strict_mode_report`）。
    中间一轮读数曾是 533/2 —— 第二条红正是本版自己的门（禁词没剥注释那条，见下），已修。
  · 新增门 `main_chain_risk_grade_is_tier_scoped_in_code_domain` 与改名后的
    `v129_tier_risk_scopes_the_whole_main_chain_one_way_only` 在此轮**双双通过**。
  · `cargo clippy -p axagent-rt-workflow --all-targets --all-features -- -D warnings`
    ⇒ 首轮 **101**（`needless_borrow`：我给 `push_constant(format!(…))` 多加一个 `&`），修后 **0**；
    `cargo test -p axagent-rt-workflow --all-targets --no-fail-fast` ⇒ **0**（lock 门 **11 passed**）。
  · `cargo fmt --all --check` ⇒ 首轮 **1**（我写的测试有 4 处折行不合 rustfmt），
    `cargo fmt --all` 落盘后 **0**；rustfmt 不动 raw string 内容 ⇒ 逐字副本无恙，
    仍复跑一次 lock 门确认（**11 passed**）。

### 一处「新写的门自己违反了已记录的纪律」（值得留在 PLAN 里）

我先在 `rhai_registry.rs` 写 `!src.contains("risk_for_veto")`，首轮实测**红在退役注释上** ——
散文里提到退役变量名是合法的。本仓早已记过「禁词门要剥注释再查」，这次是**新门没照办**。
修法不是就地补一份剥注释的小函数（那会在主 crate 造出第二份剥离实现，铁律 12），
而是按「门的存放位置由可用基础设施决定」把代码域禁词搬到 `seed_consistency_tests.rs`
（`code_only()` 的正主所在），并在那里补「剥离没吃掉代码」的自证：注入一行
`let risk_for_veto = overall_risk;` 后判据必须抓到。`rhai_registry` 侧只留 veto 调用的结构锁
+ 指向新门的注释 —— 判据只有一位主人。

⇒ 新门锁四件事：代码域零 `risk_for_veto`、`pm_risk_veto(final_action, overall_risk)` 恰一处、
必须有 `overall_risk = risk_tier;`、且**覆盖早于 `risk_rank_val`**（晚一步 f4 惩罚强度就读不到
本档 —— 「顺序即判据」的代码域版）。

⚠ 本版**会改决策数值**且比 v128 更宽（v128 只动否决，本版动 f4→posterior→action→Kelly→cap 整条）。
仍未按档的三条轴保持不变：波动率 / 夏普（60 日全局）、基本面四项（最新财报）。
核对需要**重启应用重播种**（v129 是图外脚本正文换代，`TEMPLATE_VERSION` 门会触发重种）。

## 六十一、B2-1 第一片：分析师 id **形态门**先落地（2026-10-05，不改图、不升版）

顺序上先给「待修面」装上尺子，再动生产代码 —— 建 23 个带档节点之后再去追裸匹配，
就已经是**静默失配**（查不到不报错，权重退 1.0、必采清单放水）而不是可测缺陷了。

新增 `scripts/check-analyst-id-shape.mjs`（已进 `scripts/ci-check.mjs`、`package.json`
的 `check:analyst-id-shape[:selftest]`、`.github/workflows/ci.yml`，命令逐字同源）：

| 规则 | 判据 | 现状 |
|---|---|---|
| R1 | 完整 `"a-<x>"` 字面量必须 ∈ **由种子分析师表推导**的权威 base 集（10 个，不在门里手抄第二份） | 1 条带日期豁免（见下） |
| R2 | `<base>--<tier>` 形态只能由 helper 产出，字面量里不得出现 | 0 违规（B2 前恒真，它拦的是「提前手抄」） |
| R3 | `(==/!=) "a-…"`（含 `== Some("a-…")`）与 `match` 分支 `"a-…" =>` 的**比较点**计数，锁基线**只许降** | **基线 27** = seed 17 + `evidence_weight.rs` 10 |

⚠ 基线是**首读打在未修的树上**的现状快照（本仓新门规矩），不是「门坏了」；清零后把常量改成 0，
它自动变硬拦。R3 只量**比较点**：种子里 `vec!["a-fundamentals".into(), …]` 这类**列表字面量**
与权威表同文件、R1 也看不出 ⇒ 归 B2-2 建节点那一批同批改，届时由 `check-input-mapping.mjs`
的 DAG 面兜住。这句话写进门头，是因为「门绿」不等于「待修面已清」。

**门第一轮就找到一个真缺陷**（不是我的改动造成的，是既有状态）：
`evidence_citation.rs` 的展示名映射里写着 `"a-technical" | "market-analyst" =>` 与
`"a-macro" | "policy-analyst" =>` —— `a-technical` / `a-macro` **不在权威 base 表**（表里是
`a-market-analyst` / `a-policy`），即「遗留 id + 专家短名」两套拼法共存于同一个 match。
处置：**带日期豁免**（`KNOWN_FINDINGS`，每次运行都打印「不是已修」+ 归属任务 #44），
不当违规放过、也不静默 allowlist；删除这两臂要先普查 citation.analystId 的产端
（`backtest_feedback.rs` 的测试夹具也用 `a-technical`，那是另一条链的 id 空间）。

同批把 `src/stores/feature/__tests__/stockAnalysisStore.test.ts` 的夹具 id 从臆造的
`a-tech-analyst` / `a-market` 换成真 base（`a-market-analyst` / `a-sentiment`）——
夹具用自己的假名目时，门与测试都摸不到生产键域（本仓「运行门 scope 要与生产同源」那条）。

自证（`--selftest`，6 条全 PASS）里有一条专门值：**「剥了 base 却仍比整串字面量」的假改造**
（`base == Some("a-news")`）必须仍被 R3 计入。首版漏掉 `Some(` 前缀 ⇒ 门会把最典型的 Rust
写法放过，正是这条负控把它逼出来的；另一条负控数到 2 而不是 1，暴露我直接把原文喂给 `scan`
而没过 `codeOnly`（禁词不剥注释就会红在散文上，同 §六十 那格的教训）。

读数：`check-analyst-id-shape.mjs` **0**、`--selftest` **0**、邻居门
`check-horizon-decision-keys.mjs` **0**、`dprint format:check` **0**、
`vitest stockAnalysisStore.test.ts` **63 passed**。
本片**不改图、不改脚本正文 ⇒ 不升 `TEMPLATE_VERSION`**；v130 留给 B2-2 那一批（23 节点 +
`pm-h-*.rhai` 真消费 + 前端 4 表 + 11×3 i18n）。

## 六十二、B2-1 接线 + #22 作废可见化（v130）+ 一轮清单对账（2026-10-05）

### (1) B2-1：归一接进生产点，但门的 R3 判据我**退回笨口径**

接线（身份等价，今天没有带档 id ⇒ 数值零变化）：`evidence_weight.rs::classify_role`、种子的
额外前置供给表 / `market_regime` 注入 / `max_tool_rounds` / analyst-brief 短名桥，
全部先过 `analyst_base_of(*id).unwrap_or(*id)`。买到的是「带档 id（`a-x--mid`）出现时不静默失配」——
`_ => &[]` 与后缀推断那类兜底不报错，只少一段 prompt，是本项目最难查的缺陷形态。

R3 的「是否已归一」判据我连写三版，**三版都错**，记在这里免得再走一遍：
① 命中点上方 6 行内有调用 ⇒ 长 match 块（十个臂）把已接线代码继续算成违规；
② 按**行**正则认绑定 ⇒ 扛不住 `cargo fmt` 折行（实测读数从 9 弹回 13）；
③ 按**语句** `split(';')` 认绑定 ⇒ 检不出绑定，退化成全量。
最终退回**字面量比较点全量计数**，基线如实取现值 27，并在 `scan()` 里写明「接线不降数字，
它买的是另一件事」；自测那条控制同步改成断言「归一写法照样计入」，免得下一个人以为豁免已实现。
**会把进度读没的判据比笨判据更坏** —— 与既有纪律同源。豁免判据（比较主语必须是
`analyst_base_of` 绑定的变量）留到 B2-2 重写比较点时一起立。

门首跑抓到的既有缺陷不是我的改动：`evidence_citation.rs` 的展示名映射里
`"a-technical" | "market-analyst"`、`"a-macro" | "policy-analyst"` —— 前两个 id **不在**权威 base 表
（表里是 `a-market-analyst`/`a-policy`）= 遗留 id 与专家短名共存于同一 match。走 `KNOWN_FINDINGS`
**带日期豁免**（每次运行打印「不是已修」+ 归属 #44），不静默 allowlist；删臂要先普查
`citation.analystId` 的产端（`backtest_feedback.rs` 夹具的 `a-technical` 是另一条链的 id 空间）。

### (2) #22 第一步：注释级作废 → 机器可见作废（**v130**）

`portfolio-mgr-h-ultra-short.rhai` 新增一条 `data_gaps`：「催化剂腿本档按构造不作废评估：
权威分支表 `verdict_spec.UltraShort` 不含 `eventCatalyst`」，并把头部注释改口指向它。
判据：**注释不进 `data_gaps`，呈现层就只见「没有结论」不见「为什么没有」**，
与「结构性缺口不得在呈现层造成歧义」那条裁定相反（注释级作废不算作废）。
`.rhai` 是 `include_str!` 正文 ⇒ `TEMPLATE_VERSION` 129→130，版本块写明
「本版只买到缺席可解释，没买到一致性被检查」。
⚠ 本版**没有**做两件容易被误当成做掉的事：腿集仍不含 `eventCatalyst`（扩腿集会改该档数值）、
「被计权 ⟺ 有该档的腿」这条门仍不存在（`a-catalyst` 至今逐档计权，
`evidence_weight.rs:303/317/332/346`）⇒ #22 保持未闭环。
编号更正：v130 原记给 B2-1/B2-2，实际按落地顺序发号 ⇒ **#45=v129、#22 第一步=v130、
B2-2（建 23 个带档分析师节点）顺延 v131**。

### (3) 清单对账（两组只读核查 + 关键条我自己抽验）

新闭环 **#21**（决策腿清单由权威注入）：实测链 `evidence_weight.rs:490,583` → `hooks.rs:399,408`
→ 种子逐档映射 `:5519/5605/5655/5713` → `portfolio-mgr-h-long.rhai:45` 读 `branch_json["legs"]`
→ 主链整行透传 → `DecisionBanner.tsx:1648-1675`；主链无残留硬编码腿清单。
范围被改准：**#13**（联查数据面已成立 —— 非空 `template_version` 的 21 个 distinct 值全部命中快照，
剩 `--template-version` 参数与「当前版无快照」）、**#31**（值域那半边已做，剩读侧完全不按代筛）、
**#23**（(c) as-of 已闭，剩短档没接 lockup 数据 + 量纲两套）、**#22/#24/#5/#12** 各钉到单点。
两条现网读数：`reco_picks` **1213 行 / `reco_version` 盖章 0 行** ⇒ #12 的筛样即便做完也无物可筛，
先要跑一次荐股循环；live 模板 version=129 且写库正常 ⇒ v129 已重播种，#45 的实机核对前置满足，
也说明那三列的物理 DROP 没打在活着的写路径上。

### (4) 顺带：`extract_json_from_llm_response` 的围栏信息串缺陷（#26 的第一段根因）

`crates/kit/src/utils.rs` 的通用 ``` 围栏分支不剥**信息串** ⇒ `` ```tool_json `` 的 `tool_json`
粘在 JSON 前面，`from_str` / `repair_json` / `extract_named_arrays` / 文本兜底**四层必败**，
正是「候选完整躺在 report 里、趋势智选产出 0」的形态（上一分支只专门处理 ```json，
所以走到这里的标记必然是别的名字）。新增 3 条 kit 测试（含「裸围栏不许把首行当标记吃掉」的反向控制）
⇒ `cargo test -p axagent-kit --lib` **112 passed**、clippy `--all-targets --all-features` **0**。
⚠ #26 **未闭环**：`extract_recovers_candidates_from_strict_mode_report` 仍红 —— 该夹具是**故意损坏**的
JSON，要求回收损坏点之后的候选，而第四层 `try_extract_candidates_from_text` 喂的是**外层 content**
（内层引号已转义成 `\"candidates\"`）而非解包后的 report 文本 ⇒ 改动会动生产提取路径，
需独立一轮并加「完好 report 不重复计数」的负控，本轮没有草率改。

> **本段那句根因是假设，已被 §六十四 的实测否证**：解包层其实已经在跑（`serenity_strict_mode_report`
> 拿到未转义 report 并喂进四层链），层 4 的逐对象提取也**确实回收到了候选**；真正把候选压成 0 的是
> 层 4 自身的分支顺序（先判 `has_summary` 就返回空候选 + 硬编码理由）。围栏信息串那条修的是真缺陷，
> 但它不是本红的因。留原文不删，是为了记下「先取读数再落笔」这条又一次被自己验证的过程。


## 六十三、#47：按语言配数制表（2026-10-05 已执行，规格与实测缺陷同记于此）

裁定来源：用户 2026-10-05「非中文 locale 接受改数制」；Intl compact 已实测排除
（zh-CN 下 `1234.57万`/`1亿` 与现函数 `1235万`/`1.00亿` 不一致 ⇒ 换过去会改现网读数）。

**硬约束**：`zh-CN`/`zh-TW` 两个 locale 的输出必须**逐字不变**（现网行情读数依赖它）。

做法（关键是「换算因子跟单位绑定」，所以是数制表不是词条表）：
1. `src/lib/format.ts` 内新增数制表（**TS 侧只有 unitKey，不写任何汉字** ⇒ 撤 allowlist 条目）：
   `type ScaleTier = { divisor: number; digits: number; unitKey: string }`，
   按 locale 分组：`万/亿族`（zh-CN、zh-TW、ja、ko：`[{1e8,2,"unitYi"},{1e4,0,"unitWan"}]`，
   中文两支的 divisor/digits 与今天完全相同 ⇒ 读数不变）、
   `短 scale 族`（en-US、de、es、fr、ru、ar：`[{1e9,2,"unitBillion"},{1e6,2,"unitMillion"}]`）、
   `印度族`（hi：`[{1e7,2,"unitCrore"},{1e5,2,"unitLakh"}]`，**不许**把 करोड़ 当 1e6 硬塞——
   那是数制错误不是翻译问题）。未知 locale 回落短 scale。
2. `formatYi(v)` 改名保留兼容签名或新增 `formatCompactCount(v, locale)`：**只改函数体**，
   两处调用点（`DragonTigerPanel`、`IndustryRankingPanel`）同批传 locale。
3. locales：11 个文件各加 `stockAnalysis.unitYi|unitWan|unitBillion|unitMillion|unitCrore|unitLakh`
   六键，值必须是**该语言真字**（禁占位符，这是本仓 locales 铁律）；跑 `check-i18n-key-parity` +
   `check-i18n-untranslated`（后者会拦「非 CJK locale 含汉字」⇒ 正好是这活的验收门）。
4. 同批删 `scripts/.i18n-allowlist.json` 里 `src/lib/format.ts:102,103` 那条（**豁免与实现必须同批**，
   否则豁免先撤会红、实现先落则没人记得撤）。
5. 加两条锁：zh-CN 读数逐字不变的快照测试 + 「非中文 locale 不得出现 `亿|万` 且必须出现该语言单位」
   的负控（否则数制表配错族会静默出中文单位）。

### 执行结果与一条当场抓到的缺陷

按上面规格落地：`format.ts` 的数制表 + 三族（万字族 zh-CN/zh-TW/ja/ko、短 scale 族 en-US/de/es/fr/ru/ar、
印度族 hi）+ 11 语言 6 键真译单位 + **同批撤** `scripts/.i18n-allowlist.json` 的 `format.ts:102,103`
（434→433 条；撤后 `i18n-scan --strict` 仍 0 ⇒ 是「真满足」不是「绕过」）。
新增 `src/lib/__tests__/format.test.ts` 5 条锁，测试按生产 `loadLocale` 同路径挂**真语言包**
（用 mock 替身会让断言只在比键名，测不到数制）。

⚠ 测试当场抓到我上一版写出的真缺陷：i18next 对未登记 locale 会回退到 `fallbackLng` 的**单位词**，
而数制族按 zh 算 ⇒ 输出 `1.23 hundred million` —— 因子是 1e8、单位是 1e9 的词，
**错三到四个数量级而数字看着仍像话**，正是这一族最容易悄悄漏过的形态（与「只换词不换因子」互为镜像）。
修法是把「族」和「该族单位语种」**绑成一个对象**成对回落（`WAN_SCALE = { tiers, unitLng: "zh-CN" }`），
不再把 raw locale 直接交给 i18next。快照锁现在双向覆盖：既拦中文读数被改，也拦跨族错配。

读数：vitest（stock-analysis 组件 + lib 全量）**1243 passed / 101 files**、`i18n-scan --strict` 0、
i18n 两门 0、typecheck 0、oxlint 0、dprint 0、行尾 0。
仍待实机：切 en/de/hi 看龙虎榜 / 行业排名 / 涨停封单三处金额单位与数量级；zh 两支应与改前逐字一致。

## 六十四、#26 归因与修法：第四层兜底把已回收的候选压成 0（2026-10-05）

### (1) 先取读数，不猜

上一轮只把这条红登记为「解包层还没接线」，**这个假设被实测否证了**。临时诊断测试逐层打印
（夹具 = `strict_mode_envelope()`，即 §六十(4) 那个故意损坏的 report）：

| 层 | 产出 |
|---|---|
| 解包 `serenity_strict_mode_report` | **成功**，拿到未转义的 report 文本 |
| 层 0 `from_str(extracted)` | false（`extract_json_from_llm_response` 已剥 `tool_json` 信息串 ⇒ 拿到裸 JSON） |
| 层 1 `repair_json` | 括号数 balanced 但**结构错位**（多余 `}` 把 `arguments` 提前关掉 ⇒ 仍不可解析） |
| 层 2/3 `extract_named_arrays` | None（`"candidates":` 后裸计 `[]` 深度在损坏点就崩） |
| 层 4 `try_extract_candidates_from_text(report)` | **Some((2 个候选, has_summary=true))** —— 逐对象提取本来就回收成功了 |
| `serenity_extract_from_content` 实际返回 | `{"candidates":[],"summary":"上游数据不足，无法识别有效候选标的"}` |

⇒ 缺陷不在解包层，而在**层 4 的分支顺序**：

```rust
if let Some((found, has_summary)) = try_extract_candidates_from_text(content) {
    if has_summary { return json!({"candidates": [], "summary": "上游数据不足…"}); }   // ← 丢弃 found
    return json!({"candidates": found});
}
```

两条独立错叠在一起：

1. **契约与分支矛盾**。`try_extract_candidates_from_text` 的两条路径都以「挖到候选」为成功条件
   （`extract_candidates_one_by_one` 空则 None；尝试 2 末尾 `if found.is_empty() { None }`），
   ⇒ `Some` 恒等于**非空**。于是 `has_summary` 分支唯一能触发的时机恰恰是「候选已回收」那次。
   它注释里想服务的场景（「0 候选 + summary 字段」）**从来不曾到达**。
2. **`has_summary` 的窗口算错**。`summary_pos = find("\"summary\"") - 500`，
   而判定读的是 `text[summary_pos .. summary_pos+200]` = 键位置**之前 300–500 字节**的窗口，
   任何 JSON 那段里都有 `": "` ⇒ 只要文本里出现过 `"summary"` 这个键名就判真。
   它衡量的不是「模型给了 summary」，是「前面随便有点什么」。

产品后果比 §六十(4) 那条围栏缺陷更普遍：**任何**走到层 4 的轮次（前三层全败的破损 JSON），
只要输出里带 `summary` 键，就被压成 0 候选，并且界面拿到一句**模型自己没说过的**
「上游数据不足，无法识别有效候选标的」。按「结构性缺口不得在 UI 造成歧义」这条，
这既是产出层丢数据，也是呈现层编造缺席原因 —— 而真正的「模型拒绝出票」场景
（`{"candidates": [], "summary": "…"}`，语法完好）在层 0 就带着**模型原文**返回，不经兜底。

### (2) 修法：删掉那条分支，而不是给它补判空

- `try_extract_candidates_from_text` 签名从 `Option<(Vec, bool)>` 收成 `Option<Vec>`，
  doc 里把「`Some` ⇒ 非空」写成契约；`has_summary` 那套窗口计算整体删除（不是修窗口，
  是它服务的分支本就不该存在 —— 两处一起死，不留半个假判据）。
- 层 4 改为「兜底命中即采用 `found`」，并注释记录为什么这里**没有** summary 分支。
- 顺带删掉 `run_serenity_screening` 兜底策略 2 里冗余的 `if !found.is_empty()`（同一契约）。
- **解包层同族的一面**：`serenity_extract_from_node` 原来的命中判据只认候选
  （`serenity_has_candidates(&inner)`），于是 report 里装的是「模型拒绝出票」的完好输出
  （`candidates: []` + 真理由）时，inner 整个被丢弃、退回 `first = Null` ⇒ 界面拿到
  「无候选、也无原因」。判据改为「候选 **或** 非空 summary」：**一面是凭空造原因，
  一面是把真原因扔掉**，两头都得堵。
- 不动生产提取链的层序，不加新层。

### (3) 三条负控

- `extract_does_not_double_count_well_formed_report`：report **语法完好**的信封 ⇒ 候选数恰为 2
  （不是 4）、顺序不变、`summary` 取模型原文。锁「解包层 + 四层链叠加不会重复计数」。
- `declined_round_keeps_model_own_reason_and_empty_candidates`：模型真拒绝出票 ⇒ 空候选 +
  **它自己的**理由原样保留，并显式断言不等于那句被删掉的硬编码串。锁「修 bug 不许顺手把
  反幻觉展示路径改成编造」。
- `extract_preserves_model_reason_from_wrapped_decline`：信封里的 report 是**完好**的
  「拒绝出票」输出 ⇒ 空候选 + 模型真理由必须活着返回（旧判据下这条退回 Null，改前是红的）。
  三条一起把「回收候选」与「保留缺席原因」这两个方向相反的失败面都钉住。
- 测试 18 的**名字与注释此前在说谎**（`extract_keeps_summary_when_report_unwrap_finds_nothing`
  断言的却是 `got.is_null()`）—— 按它真正锁住的行为改名
  `extract_falls_back_to_first_when_unwrap_yields_nothing`，注释同步改口。

（读数见 (4)。）

### (4) 读数与状态

| 门 | 读数 |
|---|---|
| `cargo test -p axagent --lib`（全量，一次冷编译 + 一次带自证断言的复跑） | **539 passed / 0 failed**（`EXIT=0`；改前该轮是 535 passed / 1 failed —— 本批次里主 crate 第一次全绿） |
| 五条信封/兜底锁 | `extract_recovers_candidates_from_strict_mode_report`（改前唯一红条）**ok**；`content_only_extract_returns_null_on_strict_mode_envelope`（改前形态自证，必须仍绿）**ok**；三条新负控/锁 **ok** |
| `rustfmt --edition 2024 --check src/commands/stock_workflow/serenity.rs` | 0 diff |
| `cargo clippy -p axagent --all-targets --all-features -- -D warnings`（本轮源状态） | **2 条 error，全部不在 `serenity.rs`** —— 都是 B2-1 接线行的 `explicit_auto_deref`（`analyst_base_of(*id)` → `(id)`），已修；`serenity.rs` 在该轮零发现 |

⚠ 三条锁的分工要说清，别统称「改前是红的」：只有层 4 那条（① 的 `extract_recovers…`）在改前**确实**红；
`extract_does_not_double_count_well_formed_report` 与 `declined_round_keeps_model_own_reason…`
改前也绿 —— 它们是**回归护栏**（锁「修复不顺手把重复计数/反幻觉展示改坏」）。
唯一锁「解包命中判据」的新条 `extract_preserves_model_reason_from_wrapped_decline` 没有跑过变异，
改为在测试内**自证前提**：`assert!(!serenity_has_candidates(&inner))` —— 夹具必须落在
「候选空但有理由」那一侧，否则旧判据根本不会丢它，这条锁就是空跑。

**仍待实机**：跑一轮趋势智选，看 ① 候选数不再恒 0；② 空候选时界面拿到的是模型原文而不是「上游数据不足」。

## 六十五、#24：`expectationRevision` 的分母接到年报 EPS（v131）

任务原写「补 `t-financials` 节点」，**裁定改为扩字段**，理由三条：
① `compute_valuation` 已经调 `client.get_financials(code)`，新建节点 = 同一端点再取一次；
② 分支子模板已经依赖 `t-valuation` 且有可用的 `result.content.*` 点路径；
③ 本仓既有取向是「扩字段 > 换申报 > 新建」，且 #11（吞吐）方向相反于加节点。

**口径（这才是本条的实质，不是接个键就算）**：`expectationRevision = (cons − act)/act`，
`cons` 是**财年**一致预期（`ConsensusEPS.year` 就是财年），所以 `act` 必须也是年度值。
取「最近一个**已披露年报**的 `eps`」，并且：

- **不做年化外推**。只有季报/半年报时返回 `null` —— 拿累计值年化就是近似数据顶替（裁定③）。
  旧缺陷形态：一季报 ≈ 全年 1/4 ⇒ `(cons−act)/act` 恒大 ⇒ 「预期上调」被系统性造出来。
- **跳过 `estimated` 行**（板块均值/估值 fallback 不是真实财报，不能当分母）。
- **亏损值原样交出**：「`act ≤ 0` 时比值无经济含义」的判据已经在 `leg_signal` 那一层，
  这里再判一次就成了互相掩盖。
- 年报识别复用既有口径：`report_period_key()`（兼容 `"2025-12-31"` 与 `"2025-12-31 00:00:00"`）。
  我第一版写的是 `ends_with("-12-31")` —— **会静默漏掉带时间戳的那批行**，被既有 helper 的
  注释当场否证；按报告期键比较而不是按原始串、也不依赖 vendor 行序。
- as-of 免费得到：入参已由 `truncate_financials_by_asof` 按**披露日**（P1-1）截断，
  所以「回放里用未来年报当分母」这条路不存在。

**缺席要分两种**（这是 UI 歧义问题，不是日志问题）：`basis` 在而 `value` 不在 =
该标的截止日前没有已披露年报（属性）；两者都不在 = `t-valuation` 整体没跑成（它还要行情，接线/取数）。
两档脚本各自成句，并把 v130 那句**无条件**的「本轮不可评估：缺财报节点」退役。

**已知耦合（认账不掩盖）**：估值节点因行情失败 ⇒ EPS 一并缺席，哪怕财报可得。这是选择扩字段
换来的代价，判据是「`basis` 也导航失败」而不是猜。

门：`latest_annual_eps` 5 条单测（顺序无关 / 带时间戳 / 只有中期 ⇒ 缺席 / 跳过 estimated / 负值保留）
+ `latest_eps_payload` 1 条（缺席输出 `null` 而不是 0，并点名 basis）；
整段求值门补 `value_for` 两个键 + **①′ 新断言**「中/长档齐备态必须真的有 `expectationRevision` 腿」
（这条在改前是红的 —— 分母没数据面时该腿恒缺席）；节点侧 `audit-inject-coverage --strict` 无悬空形参。

### 读数（2026-10-05，本轮源状态）

| 门 | 读数 |
|---|---|
| `cargo test -p axagent-astock-data --lib` | **535 passed / 0 failed / 7 ignored**（含 6 条新增：`latest_annual_eps` 5 + `latest_eps_payload` 1） |
| `cargo test -p axagent --lib` | **539 passed / 0 failed**（①′「中/长档齐备态必须有 expectationRevision 腿」在这轮首次为真） |
| `cargo test -p axagent-rt-workflow --test portfolio_mgr_tier_params_rhai --test portfolio_mgr_horizon_lock_rhai` | **11 passed** + **5 passed**，`--no-fail-fast` exit 0 |
| `rustfmt --edition 2024 --check`（serenity / seed_stock_analysis / seed_consistency_tests / mcp_tools 我新增的行） | 0 diff。⚠ `mcp_tools.rs` 里**另有 3 处 HEAD 既有** fmt 漂移（`oe_yield` 的多行 if-else + 两个 test mod 的 `use super::{…}` 排序），不是本轮造成，也不由本轮代改（本地 rustfmt 1.9.0-stable，仓里 `rust-toolchain.toml` 钉 1.97.0 —— 版本差即判据面差，见 [[verification-and-commit-hygiene]]「工具链版本也是门禁覆盖面」） |
| `cargo clippy -p axagent-astock-data / -p axagent --all-targets --all-features -- -D warnings` | **均 0**（`ASD_CLIPPY=0`、`MAIN_CLIPPY=0`；日志有 `Checking axagent-astock-data` 与 `Compiling axagent` 两行 ⇒ 真的查过） |
| 节点门 6 条 | `audit-inject-coverage --strict` / `check-input-mapping` / `check-rhai-numeric-typing` / `check-horizon-decision-keys` / `check-horizon-weight-parity` / `check-analyst-id-shape` **全部 EXIT=0**；inject 门无悬空形参，唯一悬空映射是既有的 `screening_source`（非本轮） |

**待实机**（`tauri dev` 重启后自动重播种 v131）：跑一只**有年报**的票，看决策面板的腿明细里出现 `expectationRevision`；
再跑一只**截止日前没有年报**的票（如次年 1 月的当年新股），看 `dataGaps` 说的是
「截止日前没有已披露年报（basis=…）」而不是那句已退役的无条件文案，且短/超短档不因此多出该腿。

## 六十六、#23 归因：`unlockRatio` 不是任何「占比」，而主链解禁臂已经是常数（2026-10-05）

本轮只读 + 现网取数，**没有动任何 Rust**（树保持在 v131 的可编译状态）。结论比原任务描述严重：
#23 不是「腿没接线」，而是**产端把一个非占比的数放进了名为 ratio 的字段里，主链按占比用它已经常年化**。

### (1) 同一个字段，四处各有各的说法（逐条 file:line）

| 处 | 说法 / 用法 | 位置 |
|---|---|---|
| 产端（唯一在用的 vendor） | `unlock_ratio = 单股东解禁股数 ÷ 当日解禁总数 × 100 × 100`，注释自陈「表示该股东解禁占当日总解禁的比例，非占总股本比例」 | `crates/astock-data/src/vendors/eastmoney.rs:2596-2602` |
| 类型声明 | `pub unlock_ratio: f64` —— **非 `Option`** ⇒ 「拿不到」只能落成 `0.0`（与 [[project-margin-designified-absence]] 同一族） | `crates/astock-data/src/types.rs:126` |
| 路由 | `lockup: vec!["eastmoney"]` —— baidu 那份实现（透传 `unlockRatio`）**根本不在链上** ⇒ 现网读数的语义只由上面那行公式决定 | `crates/astock-data/src/lib.rs:277` |
| 主链消费 | 注释写「unlockRatio 表示解禁股**占总股本**比例」，把数组内**全部行**（不筛日期）求和后与 `5.0` 比，>20 即钳到 −0.35 | `src/commands/portfolio-mgr.rhai:1363-1373` |
| 前端 | 直接 `(value).toFixed(1)%` 当百分数渲染 | `src/components/stock-analysis/EventCalendarPanel.tsx:56/194/263` |
| 腿判据 | `supplyShock` 要的是「未来解禁规模 ÷ **流通市值**」的**小数**，0.10 满强度 | `crates/analysis-engine/src/leg_signal.rs:120-122` |

⇒ §三十六 当年给的处方「归约处必须 `/100`」**不够**：问题不是差 100 倍，是分子分母都不是那两个东西。
按它改完仍然是一个有名字的错数。

### (2) 现网读数（`workflow_executions.node_executions`，154 次含解禁的执行）

| 量 | 读数 |
|---|---|
| 提取到的 `unlockRatio` 样本数 | **50641** |
| 分布 | min 0.0 / 中位 **4.21** / 均值 16.15 / max **100.0**；`>100` 的样本 **0 条** |
| 每次执行的**求和**（主链那个算法） | 152/154 条执行 **> 5.0**，且 **152/154 > 20.0** ⇒ `clamp(-sum/20, -0.35, 0)` 取到边界；min 0.0、max **21719.2** |

结论两条，都不靠代码推断：
① 「解禁压力」臂在 **98.7%**（152/154）的轮次里**恒等于 −0.35** ⇒ 乘权重 0.25 后是**常数偏置**，
   零信息量 —— 与本仓已两次的「因子方差为 0」病同源（`mcp_tools.rs` 里 total_mv 1e8 那次留过实证注）。
② 求和量级到 21719，而字段被当百分数渲染/被当「占比」判定 ⇒ 三处读法互不相同且都错。

### (3) 已定的修法（下一批执行，不再调查）

分母与日历**都在**，所以这不是结构性缺口，是可以闭合的：
1. **取数层算真占比**（沿 [[project-codex-parity-adoption]] 里 B1 的「取数层扩窗，不复制节点」形态）：
   在 `get_lockup_bundle` 的 payload 里新增 `supplyShock` 块 =
   `Σ(解禁股数 × 现价) ÷ 流通市值`，窗口 = 从**锚点日**起的**后 N 个交易日**（`astock-data::calendar`
   已有 `is_trading_day`；N 取 `harness::holding_period` 的权威天数 2/5/28/90，**不新增调参面**）；
   锚点 = as-of 上下文日期（回放不得用墙钟 ⇒ 与 #46 同一条判据）。
   分母不可得（`circulating_mv` 或 `price` 缺）⇒ 该块输出 `ratio: null` + `basis: "no_float_market_cap"`，
   **不输出 0**（`types.rs:126` 那种非 Option 字段把「拿不到」压成 0 的形态，本轮不再复制一次）。
2. **`unlock_ratio` 语义收口**：产端要么给出真占比、要么把该字段改名为 `intraday_share_pct` 并
   保留百分数（只有一处含义）。默认取**后者 + 新增真占比块**（改类型会牵动前端与 bundle 文案，
   改名是更小的爆炸半径）；同时把 `types.rs` 的 `unlock_ratio` 改 `Option<f64>` 以显式表达缺席。
3. **主链去常数化**：`portfolio-mgr.rhai` 的解禁臂改为消费第 1 步的块（窗口内、小数占比），
   阈值语义由「>5% 才罚」变成 `leg_signal` 同一条线性到 10% 满强度 ⇒ 方差恢复。
4. **接线**：short/long 档（该腿参与方向）与 mid 档（riskNote）各注入本档那一个窗口值，
   替代现在两句无条件 gap；`check-horizon-weight-parity` / 整段求值门各加一条
   「齐备态 supplyShock 必须在场且信号在值域内」+ 一条 `ratio: null ⇒ 腿缺席` 的负控。
5. 版本：动 `.rhai` 正文与节点 `input_mapping` ⇒ **v132**（B2-2 再顺延 v133）。

⚠ 本批**会改决策数值**（筹码面臂从常数变成真值 ⇒ `chip_sig`、f 因子权重合成、进而 action/仓位），
   所以必须与「按代筛样」（#31/#12）一起说清：v131 及以前的现网样本里该臂是常数，跨代不可比。

### (4) 本轮插入造成的行号漂移（手工重定位；facts 门扫不到 docs）

本轮在 `serenity.rs`（净 +2 行）与 `mcp_tools.rs`（+~100 行）里有插入，按 [[verification-and-commit-hygiene]]
「行号门不扫 gitignore 的 docs ⇒ 删/移行要按内容手工重定位」逐条复验：

| PLAN 里的引用 | 复验结果 |
|---|---|
| `serenity.rs:1516`（SNR 折算调用点，§表内） | **仍然准确** —— 现 1514-1518 就是同一调用的 `tier_days` / `SNR_ANCHOR` 实参块 |
| `serenity.rs:2333`（#26 那条 panic 的行，§十六/§表内三处） | 现在 `assert` 落在 2334（2333 是上一行的 `let got = …`）。**不改**：那是 2026-10-03 那次读数的原样记录，历史读数不能倒着修 |
| `mcp_tools.rs:4580-4648`（「唯一算分处 = 关注度权重 25%」） | 已漂移。按内容重定位：`compute_attention_score_impl` 的权重块现约 **:5155-5220**（`optimize_attention_weights_impl` 的缺省权重块另在 :5113-5122，不是同一段） |
| `mcp_tools.rs:4639` 与「损坏点 :4656-4657」（#46 墙钟锚点） | 该缺陷**已修**（#46 关单）：现 :4639 是库存周转的 `days_latest`，而 `Duration::days(90)` 在本文件已不存在；:5219 的注释自证「不再自己取 `Utc::now().date_naive()`」⇒ 原行号是改前位置，保留作历史 |

⇒ 结论一句：本 PLAN 里**带行号的句子分两类**——「当时读到」的历史读数（保留原行号）与
「现在还在这么写」的现状声明（必须按内容重定位）。上表已逐条标注是哪一类，后续批次若再插入，照此复验。

#### 源码侧的行引用（同一次复验的新读数，`check-single-source-facts.mjs` 报 BROKEN 0 但**不等于准确**）

本文件全仓只有 10 处按行号引用被我改过的四个文件，逐条列出现状（HEAD 里 `seed_stock_analysis.rs:4939`
那段在本批之后已移到别处，现 :4939 是 `context_sources` 的赋值行 —— 即**引用方自己的说明文字已指错**）：

| 引用方 | 引用的行 | 判定 |
|---|---|---|
| `crates/astock-data/src/scale.rs:8` | `seed_stock_analysis.rs:4202` | 需按内容重定位（本批在该文件 :942 处 +23 行） |
| `crates/harness/src/workflow_port_axioms.rs:15/:126` | 同上 `:3655` / `:4974` | 同上 |
| `src/commands/workflow_template.rs:964` | `:3655` | 同上 |
| `src/commands/stock_analysis_setup/seed_consistency_tests.rs:541` | `:2490` | 同上 |
| `seed_stock_analysis.rs:892`（v128 的注释） | `:4939-4942` | **实测已指错**（现 :4939 是 `context_sources` 赋值），按内容重定位为「V54 算法档阈值 6 处不同 / 影子比对 64.4% 那段」 |
| `crates/harness/src/domain_ontology.rs:50/:599` | `serenity.rs:1344` / `:1268` | 本批 `serenity.rs` 在 :660-830 净增约 +2 行 ⇒ 需按内容复验 |
| `portfolio-mgr.rhai:209/:246`、`rhai_syntax_check.rs:641` | `mcp_tools.rs:1254` | **不受影响**（:1254 早于本批两处插入） |

⇒ 判据本身要说清：facts 门的 `locate()` 允许「附近窗口内找到引号值」，所以 **+23 行的位移可以整批通过而不报**——
它验的是「这行附近还有那句话」，不是「行号仍指向同一构造」。这类门对本场景是**弱判据**，
别把它的 0 当「行号都准」。两条后续处置（下一批的 chore，不混进决策批）：
① 上表 6 处改成语义锚（引用「那段阈值不同的原文」而不是 `:4939`）；
② 若要机械保证，就得让门按「引用行的**整行内容哈希**」对账 —— 但那要求引用方把被引内容抄进注释，
   与本仓「反手抄」那条相冲，故默认选 ①。

## 六十七、#23 执行批（v132）：取数层真占比 + 主链常数臂去常数化 + 字段语义收口

按 §六十六(3) 定下的五步执行，五步全落地。

### (1) 落地内容

1. **取数层**（`crates/astock-data/src/lib.rs`）：`get_lockup_bundle` 新增 `supply_shock` 块。
   三个纯函数（`trading_day_deadline` / `unlock_shares_in_window` / `supply_shock_block`）+ 一个调用点。
   - 窗口 = 从**锚点日（不含）**往后数该档 `default_holding_days()` 个**交易日**
     （2/5/28/90 全部来自 `harness::holding_period`，本批零新增调参面）。
     真数交易日而不是 `n×7/5` 折自然日 —— 长假前一次「未来 2 日」会盖住 4–5 个交易日。
     带迭代上限做硬保护（日历整体异常时宁可截断窗口，也不许卡住一次取数）。
   - 锚点 = `as_of::current_date_or_now()`（北京时区、回放取截止日）。与 #46 同一条判据：
     窗口边界绝不用 `Utc::now()`，否则回放里「数据按截止日、窗口按墙钟」。
   - 分母 = `StockQuote.circulating_mv`（**流通市值**，不是总市值 —— 键名与块内 `denominator` 都写死了这件事）。
     行情失败 / 分母非严格为正 ⇒ `available=false` + `unavailableReason`，**不输出 ratio: 0**。
   - quote 失败**不拖垮整包**（解禁/增减持/大宗/户数仍返回）。
2. **字段语义收口**：`LockupSchedule.unlock_ratio: f64` → **`intraday_share_pct: Option<f64>`**，
   并删掉多余的 ×100（旧式 `shares/lift_all ×100×100`：0.21% 的日内份额被写成 21，前端再打上 `%`）。
   `baidu_stock.rs` 那份实现的 `unlockRatio` 分母**未经证实** ⇒ 显式 `None` 并写明
   「接进路由前必须先核实语义」，不把来路不明的数灌进已有明确语义的字段。
3. **主链去常数化**（`portfolio-mgr.rhai` 筹码面第 ② 臂）：改消费新块；档位取 `picked_tier`，
   无选档回落 `mid`（本段标题原写的「未来 30 天」正是 mid 的 28 交易日 ⇒ 不是新参数）；
   取值口径 `clamp(r / 0.10, 0.0, 1.0)` —— **与 `leg_signal` 同一条线性**；
   刻意不用 `pm_saturate`（它是有理饱和 `r/(|r|+k)`，10% 只给 0.5，会让同一个量在
   分支腿与主链腿长成两种读法，正是本批在修的那类病）。
   深度上界沿用旧 `0.35 × 0.25`：本批只改量纲与窗口，**不顺带重标定贡献深度**。
4. **接线**：short（方向腿）与 mid（riskNote）各注入 `lockup_float_ratio` + `lockup_supply_reason`
   两条映射并各加一条上游边；两档脚本把 `lockupFloatRatio` 并进 `raw`，
   v131 之前那两句**无条件** gap 退役成两种缺席各一句（「拿不到分母」≠「该窗没有解禁」≠「节点没跑成」）。
   超短/长档腿集里没有 `supplyShock` ⇒ 不注入（注入即悬空映射，`audit-inject-coverage` 会报）。
5. **呈现层**：`EventCalendarPanel` 三处同批改读 `intradaySharePct`，
   并新增 `lockupDetail()` —— 份额不可得时**只写名目不写百分数**，
   不再用 `Number(x ?? 0).toFixed(1)%` 把「不知道」渲染成「0.0%」（那读起来就是「没有解禁量」）。
   旧名不留双读兼容（面板是实时 invoke，无历史快照依赖，盘过消费者才敢删）。

### (2) 门与两条自抓到的缺陷

| 门 | 读数 |
|---|---|
| `cargo test -p axagent-astock-data --lib` | **539 passed / 0 failed / 7 ignored**（含 `supply_shock_tests` 4 条） |
| `cargo test -p axagent --lib` | **539 passed / 0 failed**（①″「short/mid 齐备态必须有 supplyShock 腿」在这轮首次为真） |
| `cargo test -p axagent-rt-workflow`（6 个脚本锁目标，`--no-fail-fast`） | **12** + **32** + **5** + **2** + **9** passed，exit 0；新锁 `v129…` 那批不受影响 |
| `cargo clippy -p axagent-astock-data / -p axagent --all-targets --all-features -- -D warnings` | **均 0** |
| 节点门 6 条（含 `--strict`）+ i18n 三门 + 行尾门 + `npm run typecheck` + oxlint + dprint | **全部 EXIT=0**；`check-input-mapping` 零违规零基线残留；inject 门无新增悬空（唯一悬空是既有 `screening_source`） |

**自抓到两条，都不是测试先给我**：
- **测试文件结构错**：`portfolio_mgr_horizon_lock_rhai.rs` 是**顶层测试**（没有 `mod` 包裹），
  我用「剥掉末尾一个右括号再插」的写法把新测试塞进了上一个 `fn` 的函数体里
  ⇒ `cargo test` 立刻 `unexpected closing delimiter`。判据：**剥右括号这类插入法必须先确认文件末尾那个右括号属于谁**
  （serenity.rs 有 mod 包裹所以同样写法是对的，两个文件的正确答案相反）。
- **heredoc 又吃了一次续行**：Rhai 断言消息里的 `\` 续行被归成一串空格。
  与 §六十三(4)/[[feedback-edit-cjk-anchor-hazards]] 补记五同源 ⇒ 反斜杠一律走 `chr(92)` 或干脆写单行。
- **clippy 独有的 NaN 缺陷**：分母判据我写了 `!(price > 0.0)`（`neg_cmp_op_on_partial_ord` 报红），
  而机械改成 `price <= 0.0` **会把 NaN 判成有效分母** —— 与 AGENTS.md 那条
  「`manual_range_contains` 对 NaN 不等价」同一个坑。终写法：
  `matches!(v.partial_cmp(&0.0), Some(Ordering::Greater))`，NaN ⇒ `None` ⇒ 走缺席分支；
  并补一条 NaN 断言（`quote(NAN, Some(NAN))` ⇒ `available=false` + 无 windows）。

### (3) 现网对照与实机核对

改前基线（§六十六(2)）：154 次执行、50641 个 `unlockRatio` 样本，逐次求和 **152/154 > 20** ⇒ 该臂恒 −0.35。
待实机（`tauri dev` 重启重播种 v132 后）：
① 挑一只**近窗有大额解禁**的票，看面板筹码面 f10 的解禁项是否随占比连续变化（不再恒满值）；
② 挑一只**窗口内无解禁**的票，看 short 档 `supplyShock` 信号是否恰为 0（有值的中性，而不是缺席）；
③ 断网/行情失败一次，看 `dataGaps` 是否写「缺分母 reason=quote_failed」而不是「没有解禁」；
④ 事件日历里解禁行的百分数是否落在 0–100（旧形态可见到 5000.0%）。
## 六十八、#13 落地 + 五个开放项的清单对账（2026-10-05）

### (1) #13：批量重跑的「代际联查」不再靠人肉查库

`src-tauri/src/bin/axagent-batch-rerun.rs` 新增 `--template-version <N>`（按代筛样）、
`--snapshot-report`（只报告即退出）与一段完整性报告。报告把三件事**分开印**，合成一句就会误导：

| 数 | 含义 | 现网实测（2026-10-05） |
|---|---|---|
| ① `TEMPLATE_VERSION` | 代码里的代 ⇒ 重播种后才生效的图 | 132（本批） |
| ② `workflow_templates.version` | 库里已播种到的代 ⇒ **本次重跑实际用的图** | 129 |
| ③ `workflow_template_versions` | 该代有无快照 ⇒ 没有就**无法按代回查**图定义 | stock-analysis 76 份，覆盖 0..128 |

⇒ 结构性结论：**最新一代恒无快照**是机制使然（快照在升级前把**旧**版本另存一行），不是故障；
且历史上并非每代都存过（76 / 129）。所以「按代筛样」与「按代回查」是两件事，CLI 现在分别说清。
另加一条硬约束：`--only-stale`（IS NULL）与 `--template-version`（= N）同时给是恒空集 ⇒
**直接报错**，不让「没有要重跑的」被当成正常结果（假绿的一种）。`TEMPLATE_VERSION` 由 `pub(crate)`
提到 `pub` 并在 `lib.rs` 再导出 —— bin 是独立 crate，读不到又不能抄第二份数字。
门：`cargo check --bins` 通过、主 crate lib **539/0**、`clippy --all-targets`（含 bin）**0**、
三文件 rustfmt 0 diff。⚠ 未替你跑 `--snapshot-report`：它会连真库并走启动期 schema 对账（写路径），
属未获授权的共享状态动作；表内三个数是用同口径 SQL 直接读出来的。

### (2) 清单对账：五项里两项其实已闭、一项与另一项同源、两项收窄

派了只读核查再自己复核关键条（不采信二手结论 —— 第一条就被我改回了）：

| 项 | 结论 | 关键证据 |
|---|---|---|
| #9 P6 反思定量修正闭环 | **已完成** | `hooks.rs:415-445` 反思统计→经验贝叶斯→注入 `horizon_prior_json`；κ 是可调变量 `seed_variables.rs:2853`；建议回写走 `stock_analysis.rs:5860/6003`；门 `ci-check.mjs:230-231` + `seed_consistency_tests.rs:3271` |
| #6 P3 分析师四周期切片 | **与 #44 同源 ⇒ 删除并入**，不重复计数 | `holding_period.rs:476-496` 的分档 id 机制全仓只有 doc-test 在调；种子自陈「今天没有带档 id」（`seed_stock_analysis.rs:3003-3006`） |
| #8 P5 规则节点 | 收窄成一条：B 级跨样本熔断无实体 | 核查报告说「arbiter 无下游」像缺陷，但 `seed_stock_analysis.rs:788` 原文是 **Q3 拍板边界**（只出投向）⇒ 记成待做等于把已决边界写成债，已改口 |
| #18 P9-6/7 探测 | 探测**已穷尽**，剩换渠道裁定 | `macro_data.rs:123-128`：只有 CPI/PPI/PMI/GDP 四条真序列，18 个候选 reportName 一律「报表配置不存在」+ 排序列名坑；逐笔全仓无实现、结论登记在 `holding_period.rs:163/358` |
| #20 P9-5 每日归档 | 定时通路已有、中毒缺陷**已修**；剩覆盖与形态 | `services.rs:3614-3629`→`stock_analysis.rs:4236`；`daily_snapshot.rs:108-122` 的 `usable()` 已在读写与判重唯一出口过滤 `"[]"`；剩 `:36` 只有热股榜、`:51` 龙虎榜显式未纳入、归档仍是 DiskCache 而非表 |

⇒ 清单从 33/46 变成 **35/45**（+2 真闭环、-1 删除合并）。开放项现在 10 个：
#5 #7 #10 #11 #12 #31 #44 + 三个「等裁定/等窗口」的 #8 #18 #20。
## 七十、#44 的前置片：citation 显示名收编 + 门的「登记面」规则（2026-10-05）

（章节号按落地顺序：#69 留给 B2-2 主批。本片属 #44，不动 `.rhai`/md 资产 ⇒ **不发版**，v133 仍归 B2-2。）

### (1) 抓到一条当下生效的缺陷，不是 B2-2 的准备活

`evidence_citation::analyst_display_name` 的 match 表里**缺五个现役 base**：
`a-lockup` / `a-catalyst` / `a-research` / `a-policy` / `a-market-analyst` ⇒ 这些分析师的证据引用
在面板「来源分析师」一格显示的是**原始节点 id**。而 `check-analyst-id-shape.mjs` 的带日期豁免
只盯**表外名字**（`a-technical`/`a-macro`），所以「表内没登记」这件事门完全看不见、还全绿。

⇒ 判据补法（把豁免换成判据）：新增 Rust 测试 `every_authority_analyst_has_a_display_name`，
遍历 `evidence_weight::EVIDENCE_ANALYST_IDS` 全部 12 个 id，断言 ① 有显示名（不落 `_ => id`）、
② 带档形态与裸 id 解析结果一致。判据住在**权威清单所在的仓内**，不再靠 JS 门猜文件名域。
另把 `analyst_display_name` 改成先 `analyst_base_of` 归一（与 `classify_role` 同一条接线）——
带档 id 一出现就不退回原始 id。红→绿读数：`a-lockup--ultra_short` 改前得 `"a-lockup--ultra_short"`。

### (2) 门反噬了一次自己的修复，按域收口而不是抬基线

补那 5 个显示名臂之后 R3 从 27 涨到 **32** —— 因为登记 base 字面量**本来就必须写在这个 match 里**。
门反过来惩罚补登记，就是它把判据指错了对象。处理：
① R3 加「登记面」规则，识别方式**按文件内容**（该文件真有 `fn analyst_display_name` 才豁免），
   不按文件名清单 ⇒ 函数改名/搬走则豁免自动失效；② 两侧自测（有登记面不计 / 改名必须计）；
③ `--dump` 里那一行标 `[登记面不计R3]` 并打印原始计数，不静默。

⚠ **改口一条**：我此前把「R3 基线清零」写成 #44 的待做项，那是错的 —— 那 27 处里 17 处早已走
`base_id` 中介，而 R3 是笨的全量计数器（「是否已归一」的判据试过三版都会把进度读没，见门头注）。
所以 R3 的语义是**上限约束**（不许再新增裸比较），不是进度条；门的收尾文案已按这个说法改。

### (3) 本批自己的行号漂移，facts 门当场抓到（BROKEN 0 → 1 → 0）

我在 `evidence_citation.rs` 前段插入 ~17 行 ⇒ `domain_semantics.rs` 的量纲 evidence 指向了空行，
`check-single-source-facts.mjs` 报 **BROKEN/越界/空行 1**（这正是「删/插几行会让别处按行号写的引用静默失准」）：

| 引用处 | 改前指向 | 现在 |
|---|---|---|
| `domain_semantics.rs` match_confidence 的 evidence | `evidence_citation.rs:176 / :207` | 重指 `:58`（值域声明）与 `:260`（`Ratio01::new` 构造点） |
| `workflow_port_axioms.rs` quality-gate 语义分支 | `seed_stock_analysis.rs:4974 / :4921` | 重指 `:6101` 并把第二个锚改成**内容描述**（case label 在其 branches 内），少留一个会烂的行号 |
| `seed_consistency_tests.rs` 辩论轮数派生 | `seed_stock_analysis.rs:2490` | `:3313`（`for round in 0..debate_max_rounds`） |

### (4) 读数

| 门 | 读数 |
|---|---|
| `cargo test -p axagent-analysis-engine --lib` | **601 passed / 0 failed**（+2：带档解析、12/12 覆盖） |
| `cargo test -p axagent --lib` | **539 passed / 0 failed** |
| `cargo clippy -p axagent-analysis-engine / -p axagent --all-targets --all-features -- -D warnings` | **均 0** |
| `check-analyst-id-shape.mjs` + `--selftest` | R3 27/27（登记面 5 处已标注不计）、自测全绿（含新两侧控制） |
| `check-single-source-facts.mjs` | BROKEN/越界/空行 **0**，软判据回到既有基线 **17** |
| rustfmt（改动的 3 个 rs 文件） | 0 diff |

⚠ 我自己写测试时踩了本门的 R2：夹具里手抄了 `"a-sentiment--mid"` 等三条带档 id ⇒ 门红。
正解不是加豁免，而是**改用产端 `analyst_node_id()` 生成** ⇒ 一条测试同时锁「产端拼出的形状 == 读端能剥的形状」。
另一条：未登记 id 的样例改成不带 `a-` 前缀（`ghost-analyst`），否则要为一条测试开豁免 = 给门打洞。

## 七十一、#5 落地：错题本按算法代际归属 + `factor_snapshot` 判为**不新建列**（2026-10-05）

（编号说明：按落地顺序发号，本片是 #5 而非 B2-2 ⇒ 追加在 七十 之后取 **七十一**；
`#69` 那个号仍留给 B2-2 主批（v133），不要因为它比本片小就以为它先落地。
本片不动 `.rhai` / md 资产 ⇒ **不发版**，`TEMPLATE_VERSION` 保持 132。）

### (1) 普查先把「加列」这条判掉了 —— 数据早就存着，只是不在那张表里

按「新增数据源/新列前先普查既有实现」的规矩取三次读数，结论是 **`factor_snapshot` 不该建**：

| 事实 | 出处 |
|---|---|
| `blackboard_snapshot` 里除节点文本外**另存两份结构化值**：`params.<nodeId>`（AgentNode `.content` 解析后的 JSON）与 `result.<nodeId>`（CodeNode `.result`） | `crates/analysis-engine/src/blackboard.rs:136-162` |
| 两条分析通道都会写它（不是只有业务封装路径） | `stock_workflow/core.rs:1109`、`core.rs:1447` |
| 复算器的输入**本来就从这个存档里取**，注释即写明「取自 DB `blackboard_snapshot["t-risk"]` 的真实存档值」 | `analysis-engine/src/portfolio_formula.rs:1337` |
| 呈现与追溯侧同样在消费它 | `key_levels.rs:76,123`、`evidence_citation.rs:145-176` |

⇒ PLAN 需求④ 那格「三要素齐不齐」当时写 ◐ 的理由是**没找到名为 `factor_snapshot` 的列**；
现在按 substance 重判：版本号（`template_version`）+ 行情时间戳（`as_of_date`）+ 因子/参数存档
（`blackboard_snapshot` 的 `params.*` / `result.*`）**三件都在同一行上**，再开一列就是把同一份
因子值抄第二遍 —— 违禁区 12（不得重复定义/重复存储），且两份会随节点改名各自漂移。
**所以 #5 的因子半不写代码，只在这节留下判据**；真正的余项是下面 (2)。

⚠ 顺带记一条**不是**缺口的东西，免得下一次又把它当活领走：`workflow_template_versions` 里
「最新一代恒无行」是设计（存的是升级前的**旧**代），当时的图在 `workflow_templates` 主表，
`dao/src/repo/workflow_template.rs:329-347` 的 `get_template_by_version` 第一步读的就是主表。
⇒ 这条订正同时改掉了 #13 在 `bin/axagent-batch-rerun.rs` 报告里的一句**假缺席**，见 (4)。

### (2) 真缺口：错题本两表**一个版本列都没有**

现网/声明层读数（grep 实证，非推断）：

- `entities/src/stock_reflections.rs` 全文 `version` **零命中** ⇒ 反思行无法归代；
- `entities/src/reflection_lessons.rs` 有 `horizon`（〇-B v2 第 4 条引入）但无代际；
- 注入链 `core.rs::fetch_stock_lessons` → `dao/repo/stock_lesson_queries.rs` 只按
  `code + 档 + 时间` 筛 ⇒ 一代算法的错题被原样喂给另一代，事后**无法归因也无法筛除**。

这与 `horizon` 引入前的跨周期污染是**同一类缺陷换了维度**，且更隐蔽：换代后旧教训
仍然「同股同档」，按档门对它完全无感。

落地形态（五处，全部照 `horizon` 那条既有纪律的形状抄，不另造口径）：

| 层 | 改动 | 关键语义 |
|---|---|---|
| 声明 | `stock_reflections.template_version` + `reflection_lessons.template_version`，均 `Option<i32>` | 正门=实体列属性 + 声明式对账（`MIGRATIONS=&[]`、启动期 `additive_only`）⇒ **没有**手写 migration；存量行保持 NULL |
| 写侧盖章 | `PendingReflectionSeed.analysis_template_version` → `build_pending_reflection_rows_for` 一处盖（三建点共用）；`extract_lesson_to_rule` 建行即盖 | 值 = **被复盘那条决策**的代际（`stock_analyses.template_version`，权威不变），抄一次是溯源戳不是第二份权威 |
| 三个建点 | `core.rs` 传 `Some(loaded.version)`；`hooks.rs`（chat）传 `None`；`bin/axagent-batch-rerun.rs` 传 `rec.template_version` | 重放补建归到**那条分析自己的代**，不是重放时跑的图 |
| 读侧筛样 | `generation_eq_or_null` + 三个 `fetch_*` 增 `generation: Option<i32>`；`count_foreign_generation_*` 两条计数 | `Some(g)` ⇒ 同代 + NULL 进 prompt，异代排除；`None` ⇒ 不过滤（与 `horizon` 的 `Option` 语义逐字同族） |
| 呈现不留歧义 | 注入文本每组带「另有 N 条来自其它算法代际，未注入」 | 空结果可能是「真没教训」也可能是「教训全在别的代」，**含义相反**；计数查询失败写「条数查询失败」而**不**折算成 0 |

三处刻意没做，都是判过的：
- 不改 `hooks.rs` chat 通道那句 `template_version: Set(None)` —— A4 已裁定那是该链路的**正常形态**
  （对话直执行不经过 `workflow_templates`），顺手"修"它等于伪造一个不存在的公式版本；
- 手动反思的 INSERT 路径同样 NULL，但**成因不同**（`original_analysis_id` 是空串 ⇒ 连被复盘的
  决策都没有）⇒ 注释里两种 NULL 分开写，不许合并成一句「旧数据」；
- `lesson` 命中同文去重时**不改写**代际：这列记「最早由哪一代产出」，改写就变成「最近被引用」，
  后者该由引用计数层（`lesson_applications`）回答。

### (3) 门

| 门 | 锁什么 | 负控 |
|---|---|---|
| `pending_rows_carry_the_reflected_analyses_generation`（`reflection.rs`） | 建点即 Set、四档同代 | seed 给 `None` ⇒ 必须落 NULL，不许冒充某代 |
| `lessons_are_isolated_per_generation`（`reflection.rs`，真库 SQLite） | 两表新列可插、同代+NULL 进、异代被排除、**两条声明句各在位**（断 `matches(..).count()==2`） | `generation=None` ⇒ 异代行**必须在**，且不得出现「来自其它算法代际」——否则上面那三条「不含」可能只是数据没插进去 |

（4) 读数与阻塞项见下。

## 七十三、#31 读侧按代筛样：统计/先验/错题本三处按**起算代际**收口（2026-10-05，不发版）

（编号说明：本片是 #31 的读侧，落地在 §七十/§七十一 之后 ⇒ 追加取 七十三。`#69` 仍留给 B2-2 主批。
未动 `.rhai` / md 资产 ⇒ **不发版**，`TEMPLATE_VERSION` 保持 132。）

### (1) 我先写错了一版规则，按 §五十一 已批规格改回：**下限，不是等号**

第一版我实现成「`template_version == 当前代`」，四个调用点还专门去库里解析当前代（dao 加了
`current_template_version`）。改回时把那些改动**全部撤掉**（含 dao helper —— 它随即变成零消费）。
理由是本文件 §五十一-② 已经批过判据：

> 新增常量（放 `holding_period.rs` 与值域同处）：`HORIZON_BRANCH_GENERATION_FLOOR = 125`

等号有个致命副作用 —— 本仓 v125→v132 只用了两周，**每次换代分母都会清零**，
指标长期停在「样本不足」，而真因是筛法本身。下限的语义是「125 起算同一套判据」。
落点：`crates/harness/src/holding_period.rs` 文件末尾（测试模块之前，避免行号漂移影响
`HORIZON_SOURCES:190` 等既有引用）。

### (2) 一条规则，两处**故意不同**的 NULL 处置

| 面 | 规则 | NULL（代际未知） |
|---|---|---|
| 错题本注入（`stock_lesson_queries` → `fetch_stock_lessons`） | `>= floor` **或** NULL 进 prompt | **保留**，但注入文本声明「代际未知」 |
| 统计分母（`reflection_stats::build_hitrate_stats`） | `>= floor` 才进分母 | **排除**，并单独计数如实报出 |

同一列在两处处置不同是**有意的**：prompt 容得下一条带标注的、来路不明的文本教训；
分母容不下未知代（那正是「两代判据的加权平均」本身）。两处都写死在注释里，防后来者"统一"掉。

### (3) 落地清单

| 层 | 改动 |
|---|---|
| 判据常量 | `harness::holding_period::HORIZON_BRANCH_GENERATION_FLOOR = 125`（**不读库**：它是判据参数，不是运行时状态 ⇒ 四个调用点天然同源，不存在「先验按一代筛、教训按另一代筛」） |
| 统计 | `DecisionPerformanceSample.template_version: Option<i32>`（值取反思行建点盖章，退化到分析行时间窗匹配）；`filter_by_generation_floor`（纯函数）+ 两类计数；`HitrateStats` 增 `generationFloor` / `excludedPreFloorGeneration` / `excludedUnknownGeneration` |
| 注入 | `generation_floor_or_null`（`gte OR NULL`，与档的 `eq OR NULL` 同族但**比较符不同**，故单独命名）；`fetch_*` 三函数 + 两个计数函数改为 floor；注入文本每组带「另有 N 条早于起算代（第 125 代），未注入」；**两组都空**时也出这句话（否则会退成「暂无历史反思」＝把结构性缺口读成空集） |
| 呈现 | `HitrateStats` 三个新键进 IPC（camelCase 门同步扩）；`ReflectionPanel` 常显一句「起算代际：第 125 代（已排除更早代的 N 条、代际未知的 M 条）」，11 语言各 1 键 |

### (4) 门（三条，都带负控）

| 门 | 锁什么 | 负控 |
|---|---|---|
| `generation_floor_keeps_at_or_after_and_counts_the_rest_separately`（engine 纯函数打表） | 夹具放 `floor-1 / floor / floor+4` 三个值 ⇒ **等号实现当场红**；两类排除分开计数 | 全部旧代 ⇒ 分母 0 且排除计数 == 原样本数 |
| `lessons_are_isolated_per_generation`（真库 SQLite，改写） | 两张表：起算代及之后 + NULL 进、旧代出；**两句声明各在位**（`matches(..).count()==2`） | 把 floor 降到旧代之下 ⇒ 旧代行必须重现，且不得声称筛过 |
| `hitrate_dto_serializes_camel_case_for_ipc`（扩键） | 三个新键的 camelCase 形状 | （原反向锁保留） |

⚠ 提醒：`expand_horizon_samples` 与 `fetch_stock_lessons` 改签名后，**旧调用点会在
`cargo check -p axagent --all-targets` 里静默漏过**（那只覆盖主 crate 自己的 target）。
改库 crate 签名必须再单独跑 `-p <lib> --all-targets` —— 本轮就是靠这条才捞到 9 处测试调用。

### (5) 读数

| 门 | 读数 |
|---|---|
| `cargo test -p axagent-analysis-engine --lib --no-fail-fast` | **602 passed / 0 failed**（+1 本批门；另 `cache_live_and_replay_are_isolated` 单跑必绿、全量偶发红 ⇒ 记入 flaky 清单，非本批引入） |
| `cargo test -p axagent --lib --no-fail-fast` | **541 passed / 0 failed**（+2 本批门） |
| `cargo check -p axagent --all-targets` / `-p axagent-analysis-engine --all-targets` | 0 / 0 |
| clippy（harness · analysis-engine · dao · 主 crate，`--all-targets --all-features -D warnings`） | **均 0** |
| 前端 | dprint 0 · i18n 键平齐 0 · 占位符 0 · 行尾 0 · oxlint 0 · `tsc --noEmit` 0 |

### (6) #31 只剩什么（别按整条领活）

已做 = 本节的统计入口（`build_hitrate_stats` → 三处下游：hooks 先验注入 / 统计面板 / 荐股先验）。

未做 = §五十一-③ 的**登记门**与其余入口，逐条仍在表里：
`evolution_drift.rs:156`→`evolution_optimizer.rs:133`（**weight_decay / strategy_performance 无代际列**，
要新增列 + 写侧盖章 + 闸门）、`stock_analysis.rs` 的 `backtest_all_history`、
`core.rs` 的 `fetch_similar_cases`（90 天 loss 错题本）、`reco_ic_stats` + `recommender/ic.rs`
（= #12，且现网 0 条盖章 ⇒ 跑一次才有样本）、`backtest_validation.rs`、`monthly_report.rs`、
`backtest.rs`、`key_levels.rs`、`trade_review.rs`、`stock_workflow/misc.rs`、`reflection.rs` 两处。
做法照 §五十一-③：把入口清单做成 Rust 常量 + 门断言每个登记项都调同一 helper，并配「拿掉某个调用必须红」的自证。

## 七十四、#20① 收口：一致预期接入每日归档 + 「申报 NHS ⇒ 必须归档」登记门（2026-10-05）

（编号承接 §七十三；仍未动 `.rhai`/md 资产 ⇒ **不发版**。）

### (1) 一致预期：抓到的是「申报了通道、通道却没接」

| 事实 | 出处 |
|---|---|
| eastmoney 自 P9-3 起把 `get_consensus_eps` 申报 `NoHistoricalSemantic`（弃用板块常数估算代理） | `vendors/eastmoney.rs:4054-4060` |
| 但读取侧白名单 `SNAPSHOT_METHODS` **没有它** ⇒ `try_stock_daily_snapshot` 的 `contains` 恒 false ⇒ as-of 恒 miss | `daily_snapshot.rs` 该常量、`lib.rs:1944-1952` |
| 而降级文案还写着「等每日归档（P9-5）」——归档**从来不会采它** | `lib.rs` `get_consensus_eps` 的 as-of 分支 |

落地三处，**必须同批**（只加一边等于没接）：
① `SNAPSHOT_METHODS` + `PER_STOCK_METHODS` 同时登记 `get_consensus_eps`；
② `get_consensus_eps` 的 as-of 分支照资金流/两融形态**先查个股快照、再探 vendor 通道**，并订正过时注释（「所有 vendor 均为 Fallthrough」已不成立）；
③ 采集侧 `run_daily_snapshot_sweep` 加 `"get_consensus_eps"` 臂（存内层 `ConsensusEPS`，读取侧按 `Option<ConsensusEPS>` 反序列化）。

### (2) 龙虎榜：**判为不该进这个白名单**（所以没动）

个股/全市场龙虎榜条目**自带日期** ⇒ eastmoney 走 `Fallthrough` + `truncate_dragon_tiger_by_asof`（按截止日截断），
**不是** `NoHistoricalSemantic` ⇒ 读侧根本不会查快照，归档对它无用。要改善得给它一条原生/截断 as-of 通道 ——
那是**语义裁定**，不是「加进白名单」。已写进 `daily_snapshot.rs` 的「仍未纳入（各自有独立理由）」段。

### (3) 新门：`scripts/check-snapshot-whitelist.mjs`

- **R1**（硬）：`PER_STOCK_METHODS ⊆ SNAPSHOT_METHODS` —— 只加一边等于没接。
- **R2**（硬）：每个 vendor 申报 NHS 的方法必须在 `SNAPSHOT_METHODS` 里；例外逐条进 `EXEMPT`
  （**带日期 + 理由，每次运行都打印**，失效即红），当前恰两条：
  `get_holder_count`（eastmoney 申报但有意不归档，计划改申报 NativeDateParam）、
  `get_peers`（iwencai 申报但**走不到** —— 调用点 as-of 放行列表只含 `NativeDateParam`，
  NHS 分支在路由层就被筛掉；同维度 eastmoney 有原生通道）。
- **R3**：`--selftest` 用合成文本证明识别器真读 match 臂（正样本 2 命中 + **全 Fallthrough 负控为空**）。
- 扫描面：逐 vendor **按 `=>` 切臂**，只有右值出现该 enum 的臂才算。
  ⚠ 本门第一版用 `grep -B6` 邻域法抽名字，把相邻臂的 `get_market_dragon_tiger`/`get_quote`
  混成了 NHS ⇒ **判据不可信、当场弃用**（没拿它建门）；这条教训写进了脚本头注。
- 首跑读数：白名单 14 项（个股级 6）、申报 NHS 共 9 个方法、0 违规、2 条豁免。
- **双处登记**：`scripts/ci-check.mjs`（--selftest + 正式两条 step）与 `.github/workflows/ci.yml`
  （同名 step，命令行**逐字同源**）。

### (4) 读数

| 门 | 读数 |
|---|---|
| `cargo test -p axagent-astock-data --lib` | **539 passed / 0 failed**（7 ignored） |
| 主 crate lib / clippy（astock + 主 crate） / 两个 `cargo check` | 541/0 · 0 · 0 · 0 |
| `check-snapshot-whitelist.mjs` + `--selftest` | 0 / 0（首跑即 0 违规） |
| `ci-check.mjs --skip-rust` 全量 | **全部检查通过（165s）** |
| `ci-check.mjs` 完整跑 | 本门两条 step 均 **✓**；随后卡在 `cargo fmt 格式化检查` ✗ —— **他人未提交的 `tencent.rs` 漂移**（`use` 排序，本地 rustfmt 1.9.0 vs 钉 1.97 的老问题），该文件本轮我从未打开 ⇒ 不代改 |

### (5) #20 只剩

① 龙虎榜的 as-of 语义裁定（给原生通道 or 申报 NHS 后两侧接线）；
② 归档要不要从 DiskCache JSON 升格为历史表（设计裁定，影响能否按 SQL 联查/回放）。

## 七十五、#31 权重线接入按代筛样：`strategy_performance` 加代际列 + 窗口按起算代 + 排除量上报（2026-10-05）

（承接 §七十三；未动 `.rhai`/md ⇒ **不发版**。）

### (1) 为什么这条不能靠「读侧加个谓词」了事

`strategy_performance` 此前**没有代际列** —— 而权重线的分母正是「同 (策略, 档) 的历史样本」，
跨代混池算出的胜率是两套判据的加权平均，且它直接决定下一轮策略权重。所以三件一起做：

| 层 | 改动 |
|---|---|
| 声明 | `entities/strategy_performance.rs` 加 `template_version: Option<i32>`（声明式对账，无 migration；NULL = 未知，**不进分母**） |
| 写侧 | 反思链写 sp 行时盖章 = **被复盘分析的代际**（`reflected_generation`，与反思行同源同值） |
| 读侧 | `load_performance_window` 内部按 `HORIZON_BRANCH_GENERATION_FLOOR` **下限**筛（与统计侧同一常量、同一条规则） |

### (2) 一处**我自己差点犯的错**：给生产 API 盖了编造的章

第一版我把 `record_performance`（cron / replay / 离线评测共用的**生产**写入口）里的盖章
写成「夹具默认起算代」⇒ 那是把「不知道」写成「就是这一代」，正是本仓一路在禁的形态。
改成**参数化**：`record_performance(..., template_version: Option<i32>)`，唯一调用点
（`backtest_analysis`）如实传 `None` 并在注释里写明为什么拿不到（入参只有 `analysis_date`，
没有 `analysis_id` ⇒ 同日多条分析时归到哪一代是**猜**）。代价是这些样本不进权重分母 ——
但会被仪表盘计入「代际未知 N 条被排除」，**可见、不静默**；要真接上得先把 `analysis_id`
透传进该命令参数（已登记在 #31 剩余项）。

### (3) 呈现：分母变小必须能解释

`EvolutionDriftDashboard` 增 `generationFloor` + `excludedPreFloorGeneration` +
`excludedUnknownGeneration`；面板加一句口径声明。**起算代随载荷下发**（`generationFloor`），
不在前端手抄 `125` —— 常量只有一处权威，面板只负责显示。

### (4) 门

`performance_window_filters_pre_floor_and_unknown_generations`（真库 SQLite）：
夹具四值 `floor-1 / floor / floor+4 / NULL` ⇒ 断言「恰在起算代」与「起算代之后」都留在窗口
（**等号实现会误杀后者**）、旧代与未知都不进，且两类排除计数恰为 (1,1)。
另：`EvolutionDriftDashboard` 的 camelCase 序列化自测里同步补两个键。

### (5) 读数

| 门 | 读数 |
|---|---|
| `cargo test -p axagent-analysis-engine --lib` | **603 passed / 0 failed**（+1 本批门） |
| `cargo test -p axagent --lib` | **541 passed / 0 failed** |
| `cargo check`（引擎 / 主 crate）· clippy（引擎+entities+dao / 主 crate） | 0 / 0 · 0 / 0 |
| 前端 | dprint 0 · i18n 键平齐 0 · 占位符 0 · 行尾 0 · oxlint 0 · tsc 0 |
| `vitest run` 全量 | **2146 passed / 1 failed** —— 失败是 `McpServerSettings` 的 5s **超时**，当时主 crate clippy 正在抢 CPU（记忆里的「CPU 挤爆 vitest」同型）；**单跑该文件 1 passed** ⇒ 判 flaky，不追判据 |

### (6) #31 只剩

**登记门**（入门口清单常量 + 「每个登记项都调同一 helper」+ 拿掉调用必须红的自证）+
其余 **9 个入口**：`backtest_all_history`、`core.rs::fetch_similar_cases`、
`backtest_validation.rs`、`monthly_report.rs`、`backtest.rs`、`key_levels.rs`、
`trade_review.rs`、`stock_workflow/misc.rs`、`reflection.rs`（:840 / :1451-1530）。
（原 11 项里 `evolution_drift → evolution_optimizer` 与统计入口本批/上批已做；reco IC 归 #12。）

## 七十六、#31 剩余 9 个入口的**分类普查**：统一门会给一半入口筛错（2026-10-05，只读，未改码）

§五十一-③ 当时写的是「把跨行聚合入口做成一份清单，断言**每个登记项都调了同一个 helper**」。
本轮把 9 个入口逐个读了一遍 —— 这个形状**是错的**：那张清单混了三种名目，统一套 `>= floor`
会让「呈现层」与「单条取数」两类别无端丢行。分类如下：

| 入口 | 实态 | 判定 |
|---|---|---|
| `backtest.rs:410` 自适应评分权重 | 「从所有已完成分析取平均得分分布」，样本 < 10 退回默认 | **统计** ⇒ 该筛（分母=分析条数） |
| `key_levels.rs:104` 关键位回测 | 取有 `blackboard_snapshot` 的已完成分析算价位命中 | **统计** ⇒ 该筛 |
| `core.rs::fetch_similar_cases` | 同股近 3 个月**失败案例** → 进 prompt | **注入语料**（与错题本同族）⇒ 按 floor 筛 + 声明被筛条数 |
| `stock_analysis.rs::backtest_all_history` | 批量回测历史分析，**只返回统计、不写 sp 行**（本轮回读确认：`backtest.rs`/`backtest_validation.rs` 均无 `strategy_performance` 写入） | **统计** ⇒ 该筛；且它是**独立回测口径**，与闭环命中率不同套 |
| `reflection.rs` 两处 | 一处是 `strategy_performance` 的**唯一写入口**（已盖章，:809-836）；另一处是反思收尾段 | 写侧，不是读侧 ⇒ 无需再筛 |
| `monthly_report.rs:45` | 月度报告：按 `analysis_date` 区间列该月分析 | **呈现** ⇒ 筛了会藏数据；正解是**标注**代际而非过滤 |
| `trade_review.rs:80` | 取卖出前最近一条已完成分析当「事前预测」对照 | **单条取数** ⇒ 筛掉旧代会让它取不到数、反而制造缺席；正解是标注 |
| `misc.rs:108` | 决策↔实际结果对账列表（观测/回放用） | **呈现** ⇒ 同上，标注而非过滤 |
| `backtest_validation.rs:469` | （行号漂移，本轮未定位到具体读取点） | **待定位**后再判 |

⇒ 修正后的门形（**下一步**）：登记表按 `统计 / 注入 / 呈现 / 单条` 四类分列，
只对前两类断言「函数体里出现允许的筛选 helper」；后两类断言「**登记了理由**」并在每次运行时打印。
一条**不允许**的形态也顺手锁死：把 floor 筛套到呈现/单条两类上（本轮的判据就是为什么）。

⚠ 另记一条**已排除的假警报**：`reflection.rs:834` 曾被本轮普查读成「第二处未盖章的 sp 写入」——
回读全文后确认那是**同一个字面量的尾部**（`agreement_score` 行），不是第二个写入口。
`grep` 只给行号、不给归属，这类判断必须回读上下文。

## 七十七、#31 收口：统计/注入两类迁移 + 按四类分列的登记门（2026-10-05，不发版）

### (1) 迁移（§七十六 分类里「该筛」的那些）

| 入口 | 改动 |
|---|---|
| `backtest.rs::optimize_weights` | 自适应评分权重按起算代下限筛；**计数与取数用同一条件**（只筛取数会出现「count 说够 10 条、实际只剩 3 条」的假足量） |
| `key_levels.rs::backtest_key_levels_with_config` | 关键位命中率按起算代筛（分母=快照数） |
| `stock_analysis.rs::backtest_all_history` | 批量回测统计按起算代筛 |
| `core.rs::fetch_similar_cases` | 注入语料：`>= floor OR NULL`（NULL 保留但声明）+「另有 N 条早于起算代…未纳入」；计数失败写「条数查询失败」而不折算 0 |

### (2) 门首跑抓到**我自己前面两处工作的形状问题**（都已改）

1. `fetch_stock_lessons` 当时把 `generation_floor: i32` 当**参数**收（调用方传常量）⇒ 函数体里看不到常量，
   登记门无从断言「这个入口真的按代筛了」。已改为**函数内部解析**（与 `load_performance_window` 同形）：
   判据常量不该由调用方透传 —— 透过一层，判据与调用点就脱钩了。
2. `stock_lesson_queries::generation_floor_or_null` 是**比较符 helper**（常量由调用方给），没有聚合入口语义
   ⇒ 断言它引用常量是假要求。已从登记表移除（它被两个 fetch 覆盖）。

### (3) 新门：`scripts/check-generation-scope-registry.mjs`

四类分列 + 两向断言，**不是**「都调同一个 helper」：

| class | 断言 | 现有条目 |
|---|---|---|
| `statistics` | 函数体**必须**引用 `HORIZON_BRANCH_GENERATION_FLOOR` | `build_hitrate_stats`、`load_performance_window`、`optimize_weights`、`backtest_key_levels_with_config`、`backtest_all_history` |
| `injection` | 同上 | `fetch_similar_cases`、`fetch_stock_lessons` |
| `presentation` | 函数体**不得**引用（筛了藏数据） | `generate_monthly_report`、`query_decision_backtest` |
| `single_lookup` | 函数体**不得**引用（筛了制造缺席） | `get_trade_review` |
| `pending` | 不得假装已筛 + **每次打印理由** | `compute_validation_report`（源表 `decision_validations` 无代际列） |

- 找不到函数 ⇒ 红（登记表不许腐烂）。
- `--selftest`：**9 条对照**，含「统计类缺常量必红」「呈现/单条/pending 带常量必红」「配平只取本函数体」。
- 双处登记：`scripts/ci-check.mjs` + `.github/workflows/ci.yml`（命令行逐字同源）。
- ⚠ **本文件不出现任何正则与转义**：第一版用 `new RegExp` 拼模式，而文件是 heredoc 落盘的 ⇒
  双反斜杠被双层归一吃掉（`\s`→`s`）⇒ 正则恒不匹配、自证与首跑**同时全红**。改纯 `indexOf` 后无转义面
  （HEAD 注释已把这条写进去，防下一个人再引入正则）。

首跑读数：登记 11 项（statistics 5 · injection 2 · presentation 2 · single_lookup 1 · pending 1），0 违规。

## 七十八、B2-2/B3 原子批的**接手清单**（2026-10-05，交给新会话逐项执行）

> **为什么单独成节**：本批是「改图 + 改消费者 + 改前端 + 换代」的**原子批**（§五十六 明写
> 「不能拆成先建节点、下次再接消费者」），一次会话做完整批最稳。本节把每项写成**可按顺序勾选**的
> 步骤，每项都在树内留下可核对的读数 —— 新会话照此执行即可，不必再做普查。
> 我（上一会话）此后不再同时改这些文件，避免两份半成品打架。

### 0. 开工前（只读，5 分钟）
1. 读 §五十四 / §五十六（规格与面积）；读 `crates/harness/src/holding_period.rs` 的
   `Period::ALL` / `analyst_subset()` / `label_zh()` / `ANALYST_TIER_SEP` / `analyst_node_id` / `analyst_base_of`。
2. 跑一次基线读数并存档：`node scripts/check-analyst-id-shape.mjs`（应 R3=27/27 绿）、
   `cargo check -p axagent --all-targets`（应 0）、`__TAURI_WORKSPACE__=true cargo test -p axagent --lib`（应 541/0）。

### 1. 节点生成（seed：`src-tauri/src/commands/stock_analysis_setup/seed_stock_analysis.rs`）
| # | 步骤 | 锚点 |
|---|---|---|
| 1.1 | `analysts` 基表与 `tool_assignments` **同时**摘掉 `a-news` / `t-news-data`（既有裁定：公告通道由 `a-catalyst` 承载） | `:2720-2731`、`:2773-2830` |
| 1.2 | 生成逐档清单 `tiered`：`Period::ALL × analyst_subset()`，id 走 `analyst_node_id`；**`value-investor` 特判**（不在基表 ⇒ 中/长各给一个，标题/专家单列）；**自证**：每个子集 base 必须能在表里找到标题/专家 | 紧接 1.1 |
| 1.3 | 分支/边按档展开：每个 tool 节点仍只一份，`e-{tool}-{tiered_id}` 与 `Branch` 对该 base 的每一档各一条；额外供给（pledge/macro/limitup/consensus）**边也要按档**各一条 | `:2843-2889`、`:2924`、`:2956`、`:2975`、`:3011-3020` |
| 1.4 | 节点循环改遍历 `tiered`（`context_sources`/`max_tool_rounds`/`market_regime` 已按 `base_id` 查表 ⇒ 改后天然对）；布局按 4 列 × N 行重排，避免压住 `p-analysts` 容器 | `:2979-3057` |
| 1.5 | `value-investor` 节点按档生成 + `value-verify` 覆写链按档（`output_var` 同名覆写是本链的机制）+ `e-value-investor-research-mgr` 与 `research-mgr` 的 `context_sources` 按档 | `:3503`、`:3715-3777`、`:4886`、`:4948` |

### 2. 消费者（同批，否则「跑了没人读」）
| # | 步骤 | 锚点 |
|---|---|---|
| 2.1 | `analyst-brief` 的 `input_mapping` 改**分档键**（`a_fundamentals_raw--mid` 形态，键由 `analyst_base_of` + 档位拼出）；**重写** `analyst-brief.rhai`：Rhai 不能枚举 scope 变量 ⇒ **显式列 23 条**，标签带档位。保持「单 brief + 单条 `analyst-brief → debate-bull-bear` 边」（逐档辩论留给后续批次） | seed `:3123-3200`、`src-tauri/src/commands/analyst-brief.rhai`（74 行） |
| 2.2 | `data-quality` 的 10×4 映射按「属主 base 的逐档节点 id」生成 | seed `:4697-4714` |
| 2.3 | `portfolio-mgr` 的 `catalyst_level` / `untrusted_*` / `flow_persistence ← a-hot-money` 路径按档生成 | seed `:5307`、`:5583`、`:5636`、`:5808`、`:5881`、`:6545-6558`（内嵌 Rhai `ctx.results["a-market-analyst"]`，断则静默退 50.0 中性分） |
| 2.4 | `pm-h-*.rhai` 读 `branch_json["analysts"]`（本档子集）+ verdict 路径按 `{属主节点 id}.content.verdict` 生成；不在本档子集的腿维持「缺席并留痕」 | `src-tauri/src/commands/portfolio-mgr-h-{short,mid,long}.rhai` |
| 2.5 | 前端 4 张手抄表按 base 归组、行 key 兼容带档 id；**11 语言 × 3 套 key**（`analyst.a-*` / `analyst.<去前缀>` / `phase.a-*`）同批补齐 | `AnalystReportGrid.tsx:18-27`、`AnalysisProgress.tsx:19-28`、`lib/stockWorkflowChatBridge.ts:13-22`、`lib/dataQualityDiagnosis.ts:25-34` |

### 3. 门（同批改，不许删）
- `seed_consistency_tests.rs:1908-1935` 按**文本**抽 `let analysts = [` 且要求每行恰 3 个字面量 ⇒
  1.2 之后它必失效：**改成断言「由 `analyst_subset()` 推导」**（这是判据升级，不是豁免）。
- `scripts/check-horizon-weight-parity.mjs:123-165`（三处键集严格相等）、`scripts/check-input-mapping.mjs`、
  `check-analyst-id-shape.mjs`（R3 只许降）。

### 4. 换代与收尾
1. `TEMPLATE_VERSION` **132 → 133**（本批动 `.rhai` 与 md ⇒ 必须升版；把「改了哪几件」写进常量上方注释）。
2. 全套门**只在最后跑一次**：`cargo fmt --all --check`、`cargo clippy --workspace --all-targets --all-features -- -D warnings`、
   `__TAURI_WORKSPACE__=true cargo test --workspace`、前端 `npm run format` / `npm run lint:eslint` /
   `npm run typecheck` / `check-i18n-key-parity.mjs` / `check-i18n-placeholder-usage.mjs`。
   ⚠ 仓库里有**他人未提交改动**（如 `vendors/tencent.rs` 的 fmt 漂移）⇒ 全量 fmt/clippy 可能报非本批的红，**报出来、不代改**。
3. 实机核对（需重启应用）：重跑一只票，逐档 Tab 各显示**本档**分析师报告；`--dump` 类读数对照 id。

### 5. 交付格式（新会话照此汇报）
每条：`文件:行` + 一句「改了什么/为什么」；门读数**读日志原文**（`test result:` / 退出码），未跑就写「未跑」；
**没做完的部分要写清会让界面出现什么**（是结构性缺席还是漏接线）。不 commit（由用户手动提交）。

## 七十九、**未闭环任务总接手清单**（2026-10-05；新会话照此推进到全清单闭环）

> 用法：按 A→B→C 顺序做。A 组不需要任何人拍板，可连续执行；B 组做到时把问题摆出来等一句裁定；
> C 组是外部条件（PG 服务重启）解锁后才可核对。每条都给了 `文件:行` / 命令 / 判据，**不需要重新普查**。
> 权威任务表：本会话会话内任务清单（#7 #8 #10 #11 #12 #18 #20 #44 #48 #49 十条开放）。

### A 组：不需拍板，可直接做

**A1 = #44（B2-2/B3 原子批）** —— 清单见 **§七十八**（12 步 + 3 道同批升级的门 + v133 换代）。
⚠ 先做 A1：其余条目多数与它同文件（seed / pm-h-*.rhai），避免并行改冲突。

**A2 = #49（`compute_validation_report` 的代际筛样）**
1. 看现状：`node scripts/check-generation-scope-registry.mjs --dump`（它的 `pending` 行即本条）。
2. 二选一：① `crates/entities/src/decision_validations.rs` 加 `template_version: Option<i32>`（声明式对账，无 migration）
   + 写侧盖章（先 grep 写入口，看能否从 pick/analysis 上溯代际）；② 改为 join `reco_picks`/`stock_analyses` 取版本列。
3. 说完事把登记门里该条从 `pending` 改成 `statistics`（门会自动开始断言它引用 `HORIZON_BRANCH_GENERATION_FLOOR`）。
4. 门：`cargo check -p axagent --all-targets`、`node scripts/check-generation-scope-registry.mjs`。

**A3 = #7 的 ② 与 ④（并行门与跨档读门）**
- ②「四路同批起跑」现状是**错峰重叠**（四个评分节点自身串成链 `seed_stock_analysis.rs:4102-4109`）⇒
  要么按实际形态写算子/注释（承认错峰），要么改 `t-scoring-{hour,week,month,quarter}` 的拓扑让四路可同批。
  **判据**：新门断言「四路分支各自只依赖本档评分节点」（不让某档意外等别档）。
- ④「档内不得读他档变量」门不存在（`seed_consistency_tests.rs:3072` 判的是「注入面==映射面」）⇒
  新门：扫 `pm-h-{short,mid,long}.rhai` + `portfolio-mgr-h-*.rhai`，禁止出现他档变量名（`totalScore_<别的档>` 等）。

**A4 = #10 P7 妖股识别（标签与判定）**
- 阈值已裁：复用 `analysis-engine/src/mover_recall.rs:41-44` 的 `mover_gain_{ultra_short,short,mid,long}`（**不得写第二份字面量**）。
- 三件：① 归约口径 = 「分析时价格 → 反思时价格」（`window_cum_gain_pct` 是档内累计，**不是同一个量**）；
  ② 标签**按档**挂在逐档反思行（与「反思每档一行」同裁定）；③ 持有期未满 ⇒ 妖股位**留空并注明未满**（不得拿当前价冒充到期价）。
- 待拍（做到这里时问一句）：标签列落 `stock_reflections` 还是 `strategy_performance`；面板要不要按标签过滤。

**A5 = #8 的 B 级持久计数/熔断**
- 新实体 `(维度×档)` 连续异常持久计数（照 `lesson_applications.rs:26-46` 的形态）+ 复用
  `weight_decay.rs` 的降权 + 照现有 grade Switch（`seed_stock_analysis.rs:5134-5174`）做熔断路由。
- **图有改动 ⇒ 与 A1 的 v133 同版或顺延 v134**（按落地顺序发号，别插队）。

### B 组：需要用户一句话裁定（做到时停下问）

| 条 | 要裁什么 | 现在的事实 |
|---|---|---|
| #7 ① | 「四路并行」的落地形态：改图（SubWorkflow×4）还是改 PLAN 文字（承认静态展开×4） | 全图 `SubWorkflow` **零实例**，现形态是静态展开；PLAN Q1 当时写的是子工作流 |
| #10 | 标签列落哪张表 + 面板是否按标签过滤 | 两处都可行，取决于面板交互 |
| #18 | 宏观序列换渠道（现只有 CPI/PPI/PMI/GDP 四条真序列，18 个候选 reportName 已证伪） | 需决定「换哪个源/放弃哪个维度」 |
| #20 | ① 龙虎榜要不要原生 as-of 通道；② 归档要不要从 DiskCache JSON **升格为历史表** | ①② 都是设计裁定；归档现状见 `daily_snapshot.rs` |
| #11 | 三项吞吐改造的落点：`batch.rs` 接哪条路径、`request_delay_ms` 在哪层生效、尾部 LLM/提示词瘦身 | 均需先有 P0 耗时读数（被 C1 挡）|

### C 组：等外部条件

**C1 = #48（现网 PostgreSQL 冷页读全挂）** —— 需用户批准重启 `postgresql-x64-18`：
1. 重启后先探活：`node scripts/pg-connect.mjs sql "select count(*) from workflow_executions"`；
2. 仍复现 ⇒ `postgresql.conf` 显式 `io_method='sync'` 再重启一次；
3. 恢复后解锁的核对：v129–v133 各批的实机核对、#12/#31 的按代筛样现网读数、A3/A5 的耗时读数。

**C2 = #12（荐股 IC 读侧同代筛样）**：前置 = 跑一次荐股循环（现网 `reco_picks` 1213 行而 `reco_version` 盖章 **0 行**）；
之后给 `RecoIcRow` 加 `algorithm_version`、装载链（`reco_loop.rs:97-123`）按代筛，形态复用
`dao/repo/stock_lesson_queries.rs` 的 `generation_floor_or_null` + 计数声明。

### 收尾（每次做完一条就做）
- 按证据改**任务条目的字段**（状态/标题/描述），不写散文总结；
- 汇报三件：`文件:行` 改动清单 → 门读数（读日志原文）→ 未完成部分会让界面出现什么；
- **不 commit**（用户手动提交）。

## 八十、#49 落地：决策验证报告按**荐股域**起算代筛样 + 登记门按域分常量、剥注释断言（2026-10-06，不发版）

### (1) 先否证 PLAN 给的选项 ①（加列 + 盖章）

§七十九 A2 给的两条路里，①「给 `crates/entities/src/decision_validations.rs` 加 `template_version` 再上溯代际」**不成立**：

- `decision_validations.pick_id` 只能 join 到 `reco_picks.id`；而 **pick 不引用** `stock_analyses`，也不引用 `workflow_executions`
  —— 「从 pick/analysis 上溯」里的 analysis 一支在本表上**根本没有连接**（`sync_outcomes_to_stock_analyses` 用
  `stock_code + 日期` 近似匹配，那是启发式，不是能盖章的外键）。
- 所以加列只会多一个**没人能填**的 NULL 列（新写入也一样填不出工作流版本 —— 荐股链不跑模板）。

⇒ 取选项 ②：join `reco_picks.reco_version`。写侧已有盖章（`stock_analysis.rs:4446`、`stock_workflow/serenity.rs:1645`），
且被 `src-tauri/src/commands/stock_analysis_setup/seed_consistency_tests.rs` 的「每个 `reco_picks::ActiveModel` 必带 `reco_version`」门守着。

### (2) 一个新常量：域不对，整数比大小就是假判据

`src-tauri/crates/analysis-engine/src/recommender/mod.rs:36` ⇒ `pub const RECO_GENERATION_FLOOR: i32 = 1;`

为什么不能沿用工作流域的 125：荐股链不跑工作流模板（出处见同文件 `RECO_ALGORITHM_VERSION` 的注释），
`reco_version` 的值域是 1.. ⇒ 拿 125 去 `>=` 比**恒假**（所有样本被判成「早于起算代」，报告永远空、
且给出的理由还是错的）；反过来拿 1 去筛 `template_version` ⇒ **恒真**。两条都是「门看着在筛、其实没筛」。

- **下限，不是等号**（§五十一-② 同一条规则）：升 `RECO_ALGORITHM_VERSION` 不自动抬这里，抬了等于每次换代把窗口清零。
- `NULL` 处置 = **排除**，与错题本注入语料的「NULL 保留」（`dao/src/repo/stock_lesson_queries.rs` 的
  `generation_floor_or_null`）**刻意相反**：教训跨代仍有参考价值，命中率/IC 分母跨代不可比。

### (3) 两个聚合入口同批 —— 否则一个名目两套分母

- 唯一判据：`src-tauri/src/commands/backtest_validation.rs:105` `screen_by_reco_generation(versions, floor)`
  ⇒ `(进分母的下标, 早于起算代, 代际未知)`，`debug_assert` 三类构成全集（漏一类＝静默丢样本）。
- 两处消费：`:289` `run_decision_backtest_inner`（cron 自动触发那条）与 `:637` `compute_validation_report`（命令那条）。
- **写侧不筛**（`:259` 注释）：旧代/代际未知的 pick 照样落 T+N 结果 —— 在写侧筛会让
  `stock_analyses.outcome` 回写链断在存量数据上。原始事实与统计分母是两回事。
- `compute_validation_report` 的返回类型由 `HitRateReport` 改为 `ValidationReportResponse`（`:539`）：
  多带 `generationFloor` / `excludedPreFloorGeneration` / `excludedUnknownGeneration` / `totalRows`
  —— 分母变小必须能解释（形态同 §七十五 权重窗口的两个排除计数）。
  前端与生成契约**零消费方**（`src/` 全仓搜 `compute_validation_report`、`HitRateReport` 均 0 命中），
  所以这次改返回类型没有同步面；新结构按禁区 13 走 `#[serde(rename_all = "camelCase")]`。
- `src-tauri/src/init/services.rs:2941`：cron 的运行产出摘要追加 `代际筛样[floor=… 早于起算代=… 代际未知=…]`，
  让「本轮 0 条进分母」在任务记录里自证，而不是只留一个空报告。

### (4) 门的升级：登记门按**域**给常量，并剥掉注释再断言

`scripts/check-generation-scope-registry.mjs`：

- 条目新增可选 `marker`（缺省 = 工作流域常量）；两条荐股域入口登记为 `statistics` + `marker: RECO_GENERATION_FLOOR`。
  `pending` 桶机制保留，当前 0 项。`--dump` 打印每条实际断言的常量名。
- 断言面从「函数体原文」改为「**剥掉行注释**后的函数体」：原写法在注释里抄一遍常量名就算「已筛」，
  而这正是本门要拦的「假装筛过」。自证相应加两条对照：只在注释里写 ⇒ 必须红；剥注释 ⇒ 不得吃掉同一行的代码。
  「剥法没吃真代码」的现场见证 = 现网 12 项跑完仍全绿（吃了就会当场红，不会静默）。
- 跨域负控：统计类夹具只引用**别域**的起算代常量 ⇒ 必须红（否则换域常量等于没门）。

### (5) 读数

- `check-generation-scope-registry.mjs --selftest` ⇒ `✅ 自证通过（16 条对照，含两方向负控）`
- `check-generation-scope-registry.mjs` ⇒ `扫描：登记 16 项（statistics 7 · injection 2 · presentation 3 · single_lookup 2 · pending 2）` + `✅`，
  并逐条打印两条 pending（见 (6) 末：#12 的两个统计读者仍未筛）。
- `check-tier-purity.mjs`（A3 的门，本批一并接进两处 CI）⇒ 首读打在**未改的树**上 = 0 串档；
  `--selftest` 8 条对照，另加四个现场变异（脚本读 `--mid` / 两档共用评分源 / 只在注释里提他档 / 块数变 3）行为全对。
- 同批复跑 12 道 node 门全绿：`check-contracts`（并把 [H] 棘轮按实测从 263 收到 **262**）、
  `check-analyst-id-shape`（R3 **26/27**）、`check-horizon-weight-parity`、`check-horizon-decision-keys`、
  `check-rhai-numeric-typing`（27 个 .rhai）、`check-input-mapping`、`check-reco-horizon-parity`、
  `check-snapshot-whitelist`、`check-ontology-consistency`、`check-domain-single-source`、`check-layer-discipline`、
  `check-domain-semantics --strict` EXIT=0。
- `cargo fmt --all --check` ⇒ 0 diff（只格式化本批文件，未触及他人文件）。
- `check-single-source-facts.mjs --ci` ⇒ EXIT=0、`BROKEN 0`；但本批在 `recommender/mod.rs` 前段插 16 行
  ⇒ `domain_semantics` 的三条 `file:行` 证据指针漂进软判据桶 ⇒ **按内容重指**（先例是 §七十 同一条）：
  `:1005→:1043`（`p.confidence = … clamp(0.0, 100.0) as u8`）、`:1006→:1008`（`p.position_pct = … clamp(0.0, 95.0)`）、
  `:313|:722→:346|:280`（`cfg.min_confidence` 赋值 | 字段声明）。重指后该条从软判据桶消失。
- Rust 三条读数（`cargo check -p axagent --all-targets` / `__TAURI_WORKSPACE__=true cargo test -p axagent --lib commands::backtest_validation` /
  `cargo clippy -p axagent --all-targets -- -D warnings`）见本会话汇报。

### (6) 现网后果（不说清就会被读成「没数据」）

2026-10-05 的现网读数：`reco_picks` 1213 行而 `reco_version` 盖章 **0 行**（§七十九 C2 同一事实）。
⇒ 筛样后**两个聚合入口的分母都是 0**，且响应体/运行记录会写明「早于起算代 0 条 / 代际未知 1213 条 × T+N」。
这不是回归，是把「样本不可比」如实说出口；解锁条件 = 跑一次荐股循环让新 pick 带章（#12 的前置，C 组）。
新鲜读数被 #48（PG 冷页读全挂）挡着，恢复后需复核一次。

界面影响面：`compute_validation_report` / `run_decision_backtest` 在前端**零调用方**
（只有 `src/components/settings/ScheduledAnalysisTab.tsx` 建 cron），所以本批的可见面只有定时任务产出摘要，
分析面板无回归项；#49 的代码面到此闭环，剩余实机核对并入 A1.13 那次重启。

**登记面因此补全（这条比筛样本身更容易漏）**：同一张 `decision_validations` 还有**两个统计读者**没筛 ——
`reco_ic_stats`（逐档 rank IC 面板）与 `load_reco_loop_samples`（**自动降权的写回路径**）。它们属 #12（C 组，
前置=跑一次荐股循环），本轮按 `pending` 登记并每次打印。若不登记，#49 把唯一的 pending 条转成 statistics 之后，
`pending` 桶会**看着是空的** ⇒ 下一轮容易误读成「全表都筛过了」。另把同表两个非统计读者也登记成
`presentation`（`list_decision_validations` 分页列表）与 `single_lookup`（`sync_outcomes_to_stock_analyses`
逐条回写 outcome），这两类断言的是「**不得**引用任一起算代常量」——筛它们是藏行/把旧代样本说成未验证。

## 八十一、A3（#7 ②④）+ A4（#10 妖股标签）落地（2026-10-06，**不发版**）

### (1) A3 的新门 `scripts/check-tier-purity.mjs`：三条判据，扫描面按域限定

只扫四个 `pm-h-<档>` 种子块 + 四份 `portfolio-mgr-h-<档>.rhai`。跨档消费者（`pm-arbiter` /
`portfolio-mgr`）**根本不在面内** ⇒ 不需要豁免表（这比「全仓扫 + 一串豁免」强：豁免表会随时间烂掉）。

- **R1** 每档 `tier_score` 的来源节点必须存在、互不相同，且都来自现场收集的 `t-scoring-*` 节点
  —— 历史真实缺陷是长档复用月线评分 ⇒ mid/long 方向永远一样（种子里那段原注释自己承认过）。
- **R2** 逐档单元内不得点名他档：`t-scoring-<别档>` / `cls-risk-level-<别档>` /
  `windows.<别档>` / `--<别档>` 分析师实例 / `horizon_branch_json.<别档>` 五形一起查。
- **R3** 评分节点之间的依赖必须逐条登记（`DECLARED_SCORING_STAGGER`，:167；现场收集 :173）。

两条实现坑（都造成过当场假绿/假红，别再踩）：

1. **点号必须算 token 断符**。第一版没放 `.` ⇒ `horizon_branch_json.ultra_short` 是一个整 token，
   尾部 `_short` 被归属成 `short` ⇒ 超短档自己被自己的键判成串档。
2. **归属取最长命中**。`ultra_short` 含 `short`、`cls-risk-level-ultra-short` 也含 `-short` ⇒
   按子串命中会把超短档判成短档。规则：`token === 档名` 或以 `_`/`-` + 档名 结尾，多命中取档名最长者。

首读打在**未改的树**上：0 串档、四档评分源 = hour/week/month/quarter 各不相同。另做六个现场变异
（脚本读 `--mid` / 两档共用评分源 / 只在注释里提他档 / 块数变 3 / 偷偷加一条错峰边 /
撤一条已声明的边），方向全对。双处接线：`scripts/ci-check.mjs` 与 `.github/workflows/ci.yml`
各两条 step（selftest + 实跑），命令逐字同源。

### (2) A3-② 选了「承认错峰」而不是改拓扑

`seed_stock_analysis.rs:4316` 那段注释改成实际形态：**四路不是同批就绪**，而是
`t-scoring → hour/week`、`week → month`、`month → quarter` 的错峰链。不改并行的两条依据写在注释里：
① 季线在 `compute_scoring` 内由月线聚合而来，让 quarter 等 month 跑完等于复用同一份月线抓取，
改并发就是把同一段历史再拉一遍；② 本机对东财 `push2*` 是**累积/速率触发** RST，五个评分节点同批
就绪会把已知风险推到峰值，换来的只是墙钟。真正的口径判据（R1/R2）与这条链的声明（R3）都由门守。

**没动**的部分（不在 A3 范围内，别混读）：#7 的 ①「四路并行的落地形态（SubWorkflow×4 vs 静态展开×4）」
仍是 B 组待拍；#11 的吞吐三项仍等 P0 耗时读数。

### (3) A4（#10 妖股标签）—— 用户两点拍板已生效

拍板：**标签落 `stock_reflections`**（逐档反思行），**面板只显示标签列、不加过滤**。

- 归约口径 = **分析价 → 反思价的原始涨跌幅** = `MarketSnapshot::price_change_pct`
  （入场 = 分析日之后首个交易日开盘价，退出 = 反思时点最新收盘）。明确否掉两个近邻量：
  `window_cum_gain_pct` 是档内逐日收盘累计（不是同一个量）；`net_return_pct` 扣了双边成本
  （阈值是给涨幅定的，扣费会把 39.8% 判成非妖股）。
- 阈值单源 = 面板变量 `mover_gain_{ultra_short,short,mid,long}`，经
  `mover_recall::tier_rules_from_vars` 取；窗口天数取该档快照自带的
  `within_expected_horizon` ⇒ 零新增按档参数，也没有第二份字面量。
- 标签值域（**NULL 不复用**，每种缺席各占一句）：`mover` / `normal` / `window_incomplete` /
  `no_market_data` / `rule_unavailable` / NULL=未复盘或本列引入前的存量行。
  「未满」与「判据不可用」都**不得**写成 `normal` —— 那是把没判定说成判定过。

落点（行号为落地后现值）：

| 处 | 位置 |
|---|---|
| 判据纯函数 | `src-tauri/crates/analysis-engine/src/mover_recall.rs:118`（`mover_label_for`）+ 门测 :866 / 阈值防漂移锁 :891 |
| 列声明（声明式对账，无 migration） | `src-tauri/crates/entities/src/stock_reflections.rs:59` |
| 消费端（读 served vars + 盖章） | `src-tauri/src/commands/stock_workflow/reflection.rs:330`（`compute_mover_label`）、UPDATE 写列 :806、两处 INSERT 留 NULL :264 / :1990 |
| 出参 | `src-tauri/src/commands/stock_analysis.rs:5528`（`list_reflections` 的 `moverLabel`） |
| 呈现分层 | `src/lib/stock-analysis-utils.ts:1971`（`moverLabelPresentation`，与 `horizonIcAbsenceKey` 同族） |
| 面板列 | `src/components/stock-analysis/ReflectionPanel.tsx:160`（cell）/ :893（列，插在「复盘档」与「结果」之间） |
| 导出 | 同文件 `exportReflections` 的 CSV 追加 `horizon` + `moverLabel` 两列 —— 标签是**按档**的结论，导出只带标签不带档就没法解释（顺带补上此前导出就缺的 `horizon`） |
| i18n | 11 语言 × 7 键（`stockAnalysis.reflection.colMover/moverYes/moverNo/moverWindowIncomplete/moverNoMarketData/moverRuleUnavailable/moverNotReflected`），插在 `colHorizon` 之后保序、LF 落盘。列名取「**妖股标签（本档）**」（各语言沿用本文件已有的 `colHorizon` 档位名词，不生造术语）—— 因为展开行里四档并列显示客观判定，列名不带档限定就会被读成「四档共用一个标签」 |

### (4) 新增/升级的门

- `check-reco-horizon-parity.mjs` 加 **门 m「妖股标签消费端单源」**（:503）：消费端必须出现
  `mover_label_for` / `tier_rules_from_vars` / `price_change_pct` 三个标识，且**行内不得拿
  出厂阈值 10/20/30/40 直接比较**（数字按边界匹配 —— 子串匹配会把 `120.0` 报成 `20.0`，
  一条自己造红的门不算门）。门 l 的扫描面同时补上 `reflection.rs`，两侧（生产 step 与 selftest 正控）
  建面一起改，避免「门在 CI 里比自测宽」。实跑 a–m 全 OK，扫描面 l=19 / m=12。
- vitest：`src/lib/__tests__/stockAnalysisHorizon.test.ts` 追加 6 条（四态各自成句、
  三种「无从判定」不得映射成「未达标」的负控、NULL≠未达标、未登记值 → `unknown` 不回退文案、
  **反手抄**：从 `mover_label_for` 现场解析产端字面量并要求 TS 全覆盖、11 语言真译）。

### (5) 现网后果与剩余

- **列是新的 ⇒ 存量 19 行反思记录标签为 NULL**，面板那些行显示「未复盘」句。这如实反映「本列引入前
  没判过」，不是缺陷；要出真值需重跑反思（`run_reflection_now` 或每日批量反思），且**必须重启应用**
  让声明式对账把新列加上（启动期 `additive_only` ⇒ 只加列不删列）。
- 标签口径与荐股链的 `reco_version` 代际**无关**（它描述单只票单档的实际涨幅，不是统计分母），
  所以不接 `RECO_GENERATION_FLOOR`；若将来妖股标签进统计分母，按 §八十 的分域规则另立判据。
- A4 未做（如实记）：**达标核查面板（MoverRecallPanel）不动**，妖股标签只在反思面板逐档行显示；
  「妖股率」这类聚合统计不在本条范围（PLAN P7 原文只要识别与标注）。
- A3/A4 都**不需要 bump `TEMPLATE_VERSION`**：A3 只改注释，A4 只加实体列与反思收尾逻辑，
  生成的图与 `include_str!` 资产一字未动。

## 八十二、A4 的 Rust 批次读数 + **A1 漏项暴露并修复**（2026-10-06，不发版）

### (1) A4 批次读数（日志原文，一条命令一个退出码，不串 `&&`）

| 门 | 读数 |
|---|---|
| `cargo check -p axagent --all-targets` | `Finished dev profile … in 11m 26s` ⇒ `CHECK_EXIT=0` |
| `cargo test -p axagent-analysis-engine --lib mover_recall` | `test result: ok. 12 passed; 0 failed; … 593 filtered out` ⇒ `AE_TEST_EXIT=0` |
| `cargo test -p axagent --lib stock_workflow::reflection` | `test result: ok. 21 passed; 0 failed; … 523 filtered out` ⇒ `REFL_TEST_EXIT=0` |
| `cargo clippy -p axagent --all-targets -- -D warnings` | `Finished` ⇒ `CLIPPY_MAIN_EXIT=0` |
| `cargo clippy -p axagent-analysis-engine -p axagent-entities --all-targets -D warnings` | ⇒ `CLIPPY_AE_EXIT=0` |
| `cargo fmt --all --check` | 0 diff |
| `cargo test -p axagent --lib stock_analysis_setup` | **`52 passed; 3 failed` ⇒ 红，见 (2)** |

### (2) 同一条命令顺手跑出的 **A1 真缺陷**（不是测试过期）

上一轮（A1/v133）的门禁面只跑了新写的 `seed_consistency_tests` 与 rt-workflow 分列门，**没跑整个
`stock_analysis_setup` 模块**；这次补跑浮出三条红，逐条定性后只有一条是测试期望问题：

- **真缺陷**：快速链派生里我手工补了一条 `t-news-data → analyst-brief` 出边（v133 自建新闻节点那处）。
  `repair_orphan_nodes` 只处理**入度为 0** 的节点 ⇒ 这条边把 `analyst-brief` 的入度变成 1 ⇒
  孤儿修复**整体跳过**它 ⇒ `FAST_BRIEF_INPUTS` 的 22 路里 21 路「`input_mapping` 里有、图上没有边」。
  后果不是报错，正是该函数文档自己点名的形态：**「跑得很快、全是空」**——快速链简报只等到新闻，
  其余 21 路按声明读取却没有调度保证，简报静默残缺。
  修法：删掉手工出边（`news_data` 本来就写在 `FAST_BRIEF_INPUTS` 里 ⇒ 修复步骤会补出
  **同一条、同一个 id** 的边），并把「派生里手工补边要先看它会不会关掉下游自动接线的前置条件」写进注释。
- 测试期望两处：① `pledge` 供给边按裸 base id 断言 ⇒ 改为按 `analyst_base_of` **从图现算**逐档实例
  期望集，每个实例都要求 `context_sources` 与供给边（漏一个档也红）；② `val_*` 槽在快速链
  刻意不重指向（本链无价值评估节点 ⇒ 恒 Null、该维度如实 missing）⇒ 改成显式豁免，
  并**反向断言**「派生图里若出现 `value-investor*` 则该豁免前提已失效，必须回到 ⑦ 重指向」。

### (3) 复跑读数（修完三条红之后）

- `cargo test -p axagent --lib stock_analysis_setup` ⇒ `test result: ok. 55 passed; 0 failed; … 489 filtered out`
  ⇒ `SEED_TEST_EXIT=0`（原来是 52 passed / **3 failed**；+3 即修复的三条，模块内其余测试一字未动）
- `cargo clippy -p axagent --all-targets -- -D warnings` ⇒ `CLIPPY_EXIT=0`
- `cargo fmt --all --check` ⇒ 0 diff
- 16 道 node 门复跑全绿；`check-analyst-id-shape` 的 R3 顺带从 26 降到 **25**（基线仍是 27，
  下降来自删掉又一处裸 `"a-fundamentals"` 字面量，不是抬基线）
- ⚠ 自付的一个额外编译周期（如实记）：新写的「按图现算期望集」闭包**引用了声明在其后的变量**
  ⇒ `E0425 cannot find value all_agent_ids`。Rust 闭包只能捕获**先前**已存在的绑定，
  这类错误 30 秒通读即可免，而代价是主 crate 测试编译一整轮（12–15 分钟）。⇒ 判据：
  **主 crate 批次发出前，先自查顺序/借用/所有权，再赌编译器。**

### (4) 换代判定：这一片**不需要** bump `TEMPLATE_VERSION`

快速链种子化走的是**内容门禁**（`same_graph && same_variables ⇒ skip`，否则重建），而源图
`stock-analysis` 的 nodes/edges 一字未动（H1 测试就是逐字比对这个）⇒ 修复会在下次启动自动重建
快速链行。`TEMPLATE_VERSION` 仍是 133；v134 留给 A5（它要改图与 `.rhai`）。

### (5) 判据沉淀（已同步进 `.qoder-cn` 记忆 4bh）

改**产端结构**（图 / schema / 键名域）之后，验证面要按「谁读它」枚举整个模块
（`--lib <module>`，而不是只跑本轮新写的那条测试名）+ 所有按文本/结构读产端的 node 门。
一条命令的成本，远低于「以为绿了两轮」的代价。

## 八十三、A5 = #8 数据质量**跨轮**熔断落地 = **v134**（2026-10-06）

### (0) 动手前的两条普查结论（它们改变了方案形状）

1. **按档计数没有数据源**。`data-quality.rhai` 只输出**一个全局** grade（其 `:1419-1423` 明文禁止
   派生 per-node 字母档），而 v133 把 23 个逐档分析师实例**在进本节点之前**就折叠成代表实例
   （`seed_stock_analysis.rs:4906-4912`，mid 优先）。⇒ 按档熔断要么先拆折叠（等于重开 A1 已批的取舍），
   要么无数据可做。⇒ 问用户并获批：**计数按全局 grade**；实体 `horizon` 列如实落 `NULL`
   （`NULL` 的含义是"本表引入时不存在按档 grade"，不是"四档同值"）。
2. **三条生效面会不会互相吞没**。先证再写：路由面把落库结论**整条**换掉
   （`decision.rs:913-934` 链尾选择器优先级 0 = `quality-fallback`），于是"置信度上限收紧"看着会被旁路。
   实证它另有消费点：公式侧走 `decision.rs:879 extract_formula_decision_json`（只认
   `portfolio-risk-gate`/`portfolio-mgr`），与链尾是**两条独立入库的口径**，面板 `formulaAction`
   与一致性诊断读的是公式侧 ⇒ 两条不重叠。⇒ 通则（已进记忆 4bj）：**每条生效面动手前写一行
   "谁最终消费这个数"**，写不出来的那条不要做。

### (1) 持久层（表结构走声明式正门，不走 migration）

- 新实体 `crates/entities/src/data_quality_observations.rs`（追加式，逐轮一行）：
  `scope` / `analysis_id` / `observed_at` 三列带 `#[sea_orm(indexed)]` ⇒ `reconcile/expected.rs:741`
  自动派生 `idx-{表}-{列}`，**不需要**在 `extras.rs` 登记（那里只放多列索引/部分索引/CHECK/FTS）。
  注册点 `entities/src/lib.rs:186`。
- 新 dao 模块 `crates/dao/src/repo/data_quality_fuse.rs`（+ `repo.rs:20` 声明）：
  `grade_is_abnormal(None) == true`（「拿不到」≠「没问题」）、判据**只在写侧落一次**成 `abnormal` 列、
  写侧按 `analysis_id` 幂等 + 读侧按同 id 只认最近一行（存量重复行不会把连续计数虚增，
  `legacy_duplicate_rows_are_deduped_on_read_not_dropped` 直插两行专门测读侧那条分支）、
  `DqiStreak::EMPTY.fused() == false` 且 `observations` 单独成字段（`0/0` 说的是"无观测"，不是"很好"）、
  `DQI_FUSE_STREAK = 3` 留在 Rust 常量而不是面板变量（面板可调要满五点对账，先例 `stop_vol_mult`
  占 7+3+3+2+4 处；本判据无调参需求）。

### (2) 三条生效面

- **① 路由**：新 SwitchNode `dqi-fuse-gate`（`seed_stock_analysis.rs:6400-6445`），
  `input_var = dqi_fuse_flag`，`cases = [fused]`，`default_case = "ok"`，`continue_on_fail = true`；
  插在 `quality-gate` 的 acceptable 出口与 `decision-explainer` 之间（`:6594` 改目标 +
  `:6604`/`:6613` 两条新边 `ok→decision-explainer` / `fused→quality-fallback`），
  `FAST_REQUIRED_NODE_IDS` 增该 id（`:7532`）。
  ⚠ **两条出边都必须带 `source_handle` 且必须声明 `default_case`**：读 `dag_store.rs` 实证 ——
  未选中的 switch 控制边不计入依赖（`:288-301`）、`Skipped` 与 `Completed` 同样算前驱满足（`:143-148`）、
  而**无 default_case 的 switch 对无 handle 的边恒放行**（`:117-121`）⇒ 少任一条就变成同一轮两套真相。
  同批改 `quality-fallback` 的 system_prompt：它现在有**两条**进入原因（本轮 D/F、跨轮熔断），
  旧文案只声明前者 ⇒ 措辞会变成假话（并加一条"不得把熔断说成本轮 D/F"）。
- **② 置信度上限**（`portfolio-mgr.rhai:2297-2345`）：`cap_floor` 先定义、被 grade 兜底档与熔断封顶
  **两处引用**（脚本里"最保守一档"只剩一个字面量，不新增常数）；`dqi_fuse_state` 三态
  （`fused` / `ok` / `unobserved`）；reasoning 三态各成一句；输出四个新键
  `dqiFuseState`/`dqiStreak`/`dqiObservations`/`confidenceQualityCap`（最后一个是为了能反解
  "是被封顶压的还是本来就低"）。注入的是**已判定结果**，脚本只做 `>= 1`。
- **③ 权重历史**（`recommender/reco_loop.rs`）：新增第四条外乘线 `dqi_penalty` ——
  本格窗口内样本按**产出时刻**（`RecoLoopSample.predicted_at_ms`，取 `generated_at` 不是验证时刻）
  归因到熔断时间线，`(clear+1)/(clear+fused+1)` 的 Laplace 转化率，`DQI_MIN_SAMPLES = 3`，
  只降不升、`None` 完全不参与；时间线由 dao 侧 `fused_intervals` / `load_fused_intervals` / `fused_at`
  给出，`fused_at` 是**三态**（早于首条观测或表为空 ⇒ `None`，不冒充清白）。
  留痕 `rationale` 同批补两条外乘线（"漏检线×/熔断线×"）—— 不写的话「窗口样本一样、权重却更低」
  在审计侧无从反解。

### (3) 写侧 / 读侧接线

- 写侧 `commands/stock_workflow/core.rs:36`（新函数）+ 两处 `bb_snapshot` 之后（`:1144` / `:1483`）。
  等级取自 blackboard 快照的 `result.data-quality`（新纯函数
  `crates/analysis-engine/src/blackboard.rs:278 data_quality_grade`，两种存储形态都读：对象与
  二次编码字符串；读不出 ⇒ `None` 不猜）。解析/落库失败 ⇒ 打日志不记行，**不把已完成的分析判成失败**。
- 读侧注入 `commands/stock_workflow/hooks.rs:452-490`：三个数分开写入变量池（同名覆盖，缺则追加）。
  **取数失败按"未熔断"处理并留痕** —— 熔断是收紧动作，拿一次查询故障当触发条件会把整条链按最坏处理，
  那是把基础设施抖动转嫁给用户决策；真正的不确定性由 `unobserved` 在界面上单独成句承担。

### (4) 面板可见 + 前端契约

- `DecisionTrustNotice.tsx`：`fused` 点亮警示条（`unobserved` **不**点亮卡，那是设施没跑过，
  每卡都亮就是噪声；它只在 reasoning 里成句）。banner 与 tag 两种形态都带。
- `agentOutput.ts` 的 `normalizeDecision` 是**白名单式重建** ⇒ 四个键显式透传 + 新增 `numberField`
  helper（缺字段 ⇒ `undefined` 而不是塌成 0；`observations` 的 0 有独立语义）。
- `types/stock-analysis.ts` 加四个字段；11 语言新增 `stockAnalysis.trustNotice.dqiFuse`
  （i18next 插值是 `{{streak}}`，写成单花括号不报错、只显示原文）。
- 落库侧无需改：`pick_chain_tail_decision` 与 `stockAnalysisStore.ts:208` 早就认 `quality-fallback`，
  熔断支复用既有节点 ⇒ 不产生第二份保守路径。

### (5) 换代判定：**v134**

改图（新节点 + 三条边重接）+ 节点级 `input_mapping` + `portfolio-mgr.rhai` ⇒ 全是播种进 DB 的模板内容。
`FAST_TEMPLATE_VERSION` 不抬（快速链重建判据是内容比对 `same_graph && same_variables`，不是版本号）。
版本块见 `seed_stock_analysis.rs:1021-1040`。
③（`reco_loop` 那条线）**不属模板内容** ⇒ 不占版本号，与本版共用同一份 dao 判据。

### (6) 读数（每条命令一个退出码，读日志原文，不串 `&&`）

| 门 | 读数 |
|---|---|
| `cargo check -p axagent --all-targets` | `CHECK_EXIT=0` |
| `cargo test -p axagent --lib stock_analysis_setup` | `test result: ok. 57 passed; 0 failed; … 490 filtered out` ⇒ `SETUP_TEST_EXIT=0`（含本轮新增 2 条：`dqi_fuse_gate_is_wired_on_both_chains` / `portfolio_mgr_receives_dqi_fuse_inputs_on_both_chains`） |
| `cargo test -p axagent --lib commands::stock_workflow` | `test result: ok. 100 passed; 0 failed; … 447 filtered out` ⇒ `WORKFLOW_TEST_EXIT=0`（含新增 `dqi_observations_are_written_only_by_the_full_chain`；修前是 `98 passed; 1 failed`，见 (7)） |
| `cargo clippy -p axagent --all-targets --all-features -- -D warnings` | `CLIPPY_EXIT=0` |
| `cargo clippy -p axagent-dao -p axagent-analysis-engine -p axagent-entities --all-targets --all-features -- -D warnings` | `CLIPPY_LIBS_EXIT=0`（首跑 `101`：本模块两条 —— `unnecessary_map_or` ⇒ 改 `is_none_or`；`collapsible-if` ⇒ `fused_at` 改 `intervals.first()?` 形态。两条都在**我新写的代码**里，不是他人漂移） |
| `cargo test -p axagent-dao --lib` | `test result: ok. 331 passed; 0 failed` ⇒ `DAO_TEST_EXIT=0` |
| `cargo test -p axagent-analysis-engine --lib` | `test result: ok. 612 passed; 0 failed` ⇒ `AE_LIB_EXIT=0` |
| `cargo fmt --all --check` | `FMT_EXIT=0`（首跑有 7 个文件的 diff，**全部是本轮我改的文件** ⇒ 用 `cargo fmt --all` 落地后复查为 0；他人文件一处未动） |
| `npm run typecheck` | `TSC_EXIT=0` |
| `npm run lint:eslint`（oxlint） | `Found 0 warnings and 0 errors` ⇒ `OXLINT_EXIT=0` |
| `npx vitest run`（全量 194 文件） | 首跑 `1 failed | 2160 passed` ⇒ 见 (7) 第 3 条；修后 `Test Files 194 passed` |
| `node scripts/ci-check.mjs --skip-rust` | `CISCHECK_EXIT=0` |
| 定点 node 门 | `audit-inject-coverage --strict` 0 / `check-generation-scope-registry` 0（16 项登记，`pending` 两条照常每次打印）/ `check-tier-purity` 0 / `check-rhai-numeric-typing` 0（27 脚本）/ `check-input-mapping` 0 / `check-single-source-facts --ci` 0 / `check-contracts` 0 / `check-analyst-id-shape` 0 / `check-i18n-untranslated` 0 |

### (7) 批次跑出来、但**不属于本轮新代码**的三条（逐条定性 + 处置）

1. **`pace-calc` 的注入面测试是 A1/v133 的漏同步**（`rhai_registry.rs::pace_calc_integer_confidence_is_not_read_as_absent`）：
   脚本 `pace-calc.rhai:136-140` 已改成从 `llm_events_ultra_short/short/mid/long` 四键里取第一个有值的实例，
   而测试仍断言 seed 里有 `("llm_events",`。⇒ 同步测试清单为四键，并把夹具注到**第一顺位**
   `llm_events_ultra_short`（注到脚本内部那个归一后的局部名 `llm_events` 会被脚本的 `let` 覆盖成恒空）。
   ⚠ 这是 4bh 的**第二次复发**：A1 那轮跑了 `--lib stock_analysis_setup` 但没跑 `--lib commands::stock_workflow`。
2. **快速链的 grade 是结构性恒等 F**（写钩子的作用域缺陷，本轮实现时抓到）。`data-quality.rhai` 的 score
   有 35%+35% 来自分析师报告质量 / 工具可信度，而快速链按既定设计不建那 10 个 Agent 节点
   ⇒ `score ≈ factor_completeness × 30 ≈ 23` ⇒ **恒 F**（派生器 ③ 因此摘掉 `portfolio-mgr` 的
   `dqi_score`/`dqi_grade` 并换掉 `quality-gate` 判据）。若照原方案把每一轮都记进全局观测表，
   三轮之后整条链永久熔断，且界面理由是假的。⇒ 写钩子按 `template_id` 拦掉快速链
   （`core.rs:47` 加形参 + `FAST_TEMPLATE_ID` 早退），新增测试
   `dqi_observations_are_written_only_by_the_full_chain` 同时锁「坏快照不记行」「NULL grade 计入异常」。
3. **参数面板的反断链门抓到我的三条同名映射**（`StockAnalysisConfigPanel.test.tsx`，
   「portfolio-mgr input_mapping 的同名映射必须在变量表中有定义（或为运行时注入）」）：
   三数由 `hooks.rs` 运行时注入、既不在 seed 变量表也不该进面板 ⇒ 加入该测试的
   `RUNTIME_INJECTED` 白名单并写明权威源与「若改可调要按五点对账且删掉 dao 那份常量」。
   这道门正是 2026-09-11「配置项空接线」的检法，它按预期红了新加的键，属**门有效**而不是门坏。

### (8) 未完成部分与界面后果（如实分三段）

- **代码与门禁**：A5 三条生效面 + 持久层 + 前端契约全部落地，上表读数全绿；**未提交**（按指令）。
- **现网行为**：`data_quality_observations` 是新表，启动期 `additive_only` 会建表；表初始为空
  ⇒ `dqiFuseState = "unobserved"`、`dqi_fuse_flag = 0` ⇒ **现网存量结论逐位不变**。
  熔断要等完整链跑满 3 轮异常才可能触发；在此之前界面上只会出现 reasoning 里那句
  「数据质量跨轮观测缺失（观测表 0 条），熔断判定未参与」。
- **仍被挡的核对**：v133（A1）与 v134（A5）的重播种 + 实机核对都要重启应用，
  且被 **C1/#48（PG 冷页读全挂）** 挡着 ⇒ 无法证明「库里真的播种了新节点」。
  ③ 权重线还需要 `decision_validations` 里有**熔断之后产出**的样本才会有非 `None` 的乘数，
  在那之前 `dqiPenalty` 字段在 JSON 里缺席（`skip_serializing_if`），闭环面板不会显示该列 —— 与 mover 线一样。
- **已知边界（不是缺陷，是口径）**：① 计数是**全局**的（现网没有按档 grade）；
  ② ③ 的归因按**时刻**（pick 表没有 analysis 溯源，这是 #12/#44 那条既有缺口）；
  ③ `unobserved` 不点亮决策卡（避免每卡噪声），只在 reasoning 成句。

## 八十四、B 组四条裁定（2026-10-06 用户拍定）+ 动手前的三条**硬前置**

### (0) 裁定原文（逐条按用户选项记录，不要凭记忆复述）

| 条 | 裁定 | 备注 |
|---|---|---|
| #7 ① | **真改图：换成 SubWorkflow×4** | 不采纳「承认静态展开」那条最省事选项 |
| #18 | **换官方源（国家统计局 / 央行公开接口）** | 不采纳「收口径只声明四条」 |
| #20 ① | **做原生 as-of 读侧**（龙虎榜） | 推翻此前「不该走每日归档」的 Fallthrough 判定 |
| #20 ② | **每日归档从 DiskCache JSON 升格为历史表** | 不保留文件形态 |

### (1) 普查推翻了「B 组都是纯代码活」这个前提 —— 三条硬前置

**前置一（挡 #7①，是引擎级真缺陷，不是改图工作量）**
`crates/rt-workflow/src/work_engine/executors/subworkflow_executor.rs` 是**完整实现且在生产被用**
（认知编排器图有 3 个真模板 SubWorkflow 节点：`cognitive_router_init.rs:430/491/561`），
所以「全图零实例」是我此前的**误判**（零实例只对股票模板成立，`stock_workflow/reflection.rs:318` 记着 v2 主动退役过）。
但普查读出三条硬事实，决定不能直接改图：

1. **子图失败被父图吞成成功**：`engine/mod.rs:5139-5151` 的 `run_workflow` 失败仍返回 `Ok(status=Failed)`，
   而两处 sub 回调（`engine/mod.rs:2703-2710`、`3507-3516`）只 `.map_err` **不查 status**，
   失败时 `output.unwrap_or({})` 当成功返回 ⇒ 父节点的 `continue_on_fail` 与下游对「子图整段失败」完全无感。
   四档分支若走这条路，一档子图失败就会落回一个**空对象**，界面按「该档无结论」渲染 —— 这是
   「结构性缺口伪装成正常结果」的引擎级复发口，比股票链里任何一个缺陷都大。
2. **执行器路径零引擎级测试**：`tests/execution_finalization.rs:78` 跑的是普通图 + `parent_execution_id`
   收尾捷径，**没经过 SubWorkflow 执行器**；`executors/mod.rs:313-357` 只测 `unwrap_end_envelope`。
3. **缺任一映射键即整节点硬错**：`subworkflow_executor.rs:95-100`（`map_inputs` 从父变量池按点路径取值，
   取不到就 `SUBWORKFLOW_FAILED`）。⇒ 四档扇出要求**每档映射键在父池齐备**；
   而子图拿的是**快照**（`engine/mod.rs:2670-2687`），子图内部节点输出**不回父池**，
   只由父引擎按 `node_id` + `output_var` 双键写回（`engine/mod.rs:1717-1721`）。

⇒ 执行顺序被这条前置改写：**先补 1（失败传播）+ 2（引擎级测试），再谈把四档换成 SubWorkflow×4**。
另外 `SubWorkflowConfig`（`workflow_types.rs:744-752`）的 `is_async` / `cache_enabled` / `cache_ttl_secs`
是**声明未消费**的死配置（`subworkflow_executor.rs:54-55`），改图前要先决定是补实现还是退役字段
（按禁区 12/记忆「退役形状前先盘下游消费者」）。
父 DAG 对 SubWorkflow 是**不透明叶子**（`dag_store.rs` 零特殊处理）⇒ 子图内部节点不参与父入度/就绪计算，
四档的「错峰起跑」判据（`check-tier-purity.mjs` R1/R3 与 A3 §八十一）在改图后**语义会失效**，
必须同步换判据而不是让门继续绿。

**前置二（挡 #18，可行性未证）**
全仓 `stats.gov.cn` / `pbc.gov.cn` **零命中**；唯一相关是 `astock-data/src/macro_data.rs:9` 的注释
**声称**「国家统计局/央行（通过 neodata/eastmoney 代理）」—— 那是文档谎言面，实际端点是
`datacenter-web.eastmoney.com`（`:333-337`）。⇒ 换官方源不是改配置，是**新增一个数据源**：
先要实测两件事（本机可达性、按 reportName/指标码能否取到**带真发布日**的历史序列），
再决定做成 vendor（登记面 8 处：`lib.rs:145-185 VendorRouting` + `:388` 装配 + `default_routing:216-340`、
`register_vendor:728-775`、`gate.rs:14-25 CAPACITIES`、`vendors/mod.rs:509-522`、
`seed_variables.rs:450-548`、`recommender/pool.rs:197-211 KNOWN_VENDORS`、
`DataVendorsTab.tsx:140-160`，加 `as_of_capability.rs:22-39` 申报与快照白名单）。
⚠ 现网 as-of 语义是**推断发布日**（`macro_available_at:155-189`：cpi/ppi 次月 15、pmi 月末+2、gdp 次月 20，
并把这个推断写进 `source:197` 供界面点名）⇒ 若官方源给得出真发布日，`pick_published:278-292` 的判据与
`macroPartialSeriesOnly` 文案（`holding_period.rs:177`，**中文硬编码在 Rust 不在 i18n**）要同批改。
消费面只有一条键：`pmi` ← `t-macro-data.result.content.pmiManufacturing.value`（`seed:6081-6082`，长档），
加 `leg_signal.rs:119` 的 PMI 荣枯线；**Rhai 脚本零消费**（全 `*.rhai` 对 macro/cpi/gdp 无命中）。

**前置三（挡 #20②，改的是形态不是加字段）**
现归档是**独立 DiskCache 实例 + 单文件** `~/.axagent/astock_daily_snapshot.json`
（`init/state.rs:1616-1618`），全量重写 tmp+rename（`disk_cache.rs:246-254`），
上限 10_000 条 + 64MB 字节预算、超预算 LRU 淘汰（`disk_cache.rs:22/33/129-148`）。
读取侧按名目分类（这一分决定「哪些必须换、哪些不该换」）：
- 类①「as-of 回放取数替换」**13 点**（`astock-data/src/lib.rs:1876` 通用 NHS 臂 + 个股级
  `2580/3106/3609/3779/3893/3985/4216/4322` + keyword `3360` + 全市场专用 `4598/4763/4968/5408`）
  ⇒ 升格后这 13 点全换 SQL 读端，外加三个 helper `try_daily_snapshot:1909` / `try_stock_daily_snapshot:1944` / `:1971`。
- 类②「跨日聚合统计」**零现读者** —— DiskCache 根本没有 prefix-scan API ⇒ **升格表带来的新能力**，
  不是回归面。这条决定验收口径：升格之后要能真做一次跨日聚合，否则升格只是换个存储。
- 类③「呈现/判重」2 点：缺席三态文案 `snapshot_absence_clause:1924-1937`、判重闸门
  `has_daily_snapshot:1957` + `daily_snapshot_enabled:1965`。
⇒ 现成同类正门照抄：`entities/src/market_daily_close.rs:17-38`（`id=code@date`）+
`analysis-engine/src/market_close_store.rs` + `init/services.rs:3701-3760` 的 sweep ——
那个文件头十二行就明写「不用 DiskCache 承载本表，窗口聚合必须是 SQL」，正是 #20② 要的形。
⚠ 表结构正门之外有**四张登记面必须同批**：`reconcile/extras.rs` 的 `MULTI_COL_INDEXES/GIN_INDEXES/
PARTIAL_INDEXES/CHECK_CONSTRAINTS`（`:1242-1244`、`:743`）、`extras.rs:1268-1330` 的**冻结计数**
（`EXPECT_MULTI=35` / `EXPECT_CHECKS=3` / `EXPECT_PARTIAL=6` / `EXPECT_NON_MAIN_DB_TABLES=8` /
`l2_total_is_sixty_nine`）⇒ 加条目必须显式改数；`expected.rs:549-612` 的「每个注册实体都要成表」；
`safety.rs:418-432` 闸 3 `BaseShrank`。`dao/build.rs:25-49` 扫 `entities/src/lib.rs` 的 `pub mod`
生成实体清单 ⇒ **不需要**手工表名数组，也不需要 migration。
另外 `scripts/check-snapshot-whitelist.mjs` 的判据要**重指**：白名单语义由「DiskCache key」变「表 method 域」，
而它现在只扫 `vendors/*.rs` 的 `fn asof_capability` match 右值（`:112-122`）⇒ `macro_data.rs` 不在扫描面上
（它不是 vendor），这条绕过口在 #18 做成 vendor 后会自动消失，但**不能靠"自动消失"来闭嘴**，要显式扩面。

### (2) 建议执行顺序（按「前置最少 + 复发风险最高」排）

1. **B-1 = #7① 的前置片**：子图失败传播（`engine/mod.rs:2703-2710`/`3507-3516` 查 status 并上抛）
   + 第一条真正过执行器的引擎级测试。**不改股票图**，因此不发版；它同时修好认知编排器那 3 个现役节点。
2. **B-2 = #7① 本体**：新建四档子模板 + `SubWorkflowConfig` 死配置裁定 + 扇出映射键齐备性门
   + A3 那道 `check-tier-purity` 判据换形 ⇒ 发版（顺延 v135）。
3. **B-3 = #20②**：归档升格历史表（照 `market_daily_close` 形；13 个读点 + 4 个写点 + 4 张登记面 + 门重指），
   并**必须**产出类②的一次跨日聚合，否则不算闭环。
4. **B-4 = #20①**：龙虎榜原生 as-of 读侧（与 B-3 同一张表框架，放它后面省一次框架设计）。
5. **B-5 = #18**：官方源可行性实测 → 决定是否做成 vendor → 真发布日判据替换推断规则。

⚠ 共同的现网核对前提仍是 **C1/#48（PG 冷页读全挂）**：B-2 改图要重播种、B-3/B-4 改存储要回查历史行，
两者都需要库里读得出真数。**代码可以先行，验收必须等重启。**

## 八十五、B-1 落地：子工作流失败不再被父图吞成成功（2026-10-06，**不发版**）

§八十四 前置一的第 1、2 条已补。**这一步不改股票图** ⇒ 模板内容一字未动 ⇒ 不占 `TEMPLATE_VERSION`
（判据同 §八十二(4)：播种走内容比对，`same_graph && same_variables ⇒ skip`）。

### (1) 改动

- `crates/rt-workflow/src/work_engine/engine/mod.rs:398` 新增私有函数 `sub_workflow_output_or_fail(child, sub_workflow_id, child_execution_id)`：
  只有 `status == Completed` **且** `output` 有终值才返回值，其余一律 `Err`，错误文本点名子模板 id 与子执行 id。
  两个从严判定点写在函数文档里：`PartiallyCompleted` 的含义是**有节点停在 Pending**（死锁兜底路径），
  子图声明的输出键可能压根没写 ⇒ 不做「部分可用」的乐观假设；`Completed` 但 `output = None`
  回 `{}` 就是第二次伪装 ⇒ 同样失败。
- 两处调用点改走该函数（过去都是 `result.output.unwrap_or(json!({}))`，只 `.map_err` 不看 status）：
  `engine/mod.rs:2755`（引擎内联子图路径）与 `engine/mod.rs:3563`（`sub_workflow_callback` 递归路径）。
- 测试两条（`engine/mod.rs` 尾部 `mod tests`）：
  `sub_workflow_result_is_usable_only_when_completed_with_output`（五种非 Completed 终态逐个红 +
  Completed 无终值红 + Completed 有终值绿）、
  `failed_sub_workflow_output_is_not_accepted_even_when_present`（反向锁：防有人把判据改成「有 output 就算成功」）。

### (2) 影响面（如实声明，不等用户来问）

现役**认知编排器**图里有 3 个真 SubWorkflow 节点（`cognitive_router_init.rs:430/491/561`），
它们此前享受的是「子图失败 ⇒ 静默空对象」。本改动把它们换成「子图失败 ⇒ 父节点显式失败」。
这是**行为变更不是纯加固**——但它消除的是"错了看不出来"。普查同时纠正了我此前的一条误判：
「全图 SubWorkflow 零实例」只对**股票模板**成立（`stock_workflow/reflection.rs:318` 记着 v2 主动退役过它），
执行器本身一直在生产路径上跑。

回归读数（见 (3)）：rt-workflow `--all-targets --no-fail-fast` 共 **16 个测试二进制全绿**，
没有任何一条现有测试依赖旧的空对象成功语义。

### (3) 读数

| 门 | 读数 |
|---|---|
| `cargo test -p axagent-rt-workflow --lib` | `test result: ok. 294 passed; 0 failed` ⇒ `RTW_LIB_TEST_EXIT=0`（含本轮新增 2 条） |
| `cargo test -p axagent-rt-workflow --all-targets --no-fail-fast` | **16 个测试二进制全部 `ok`、0 failed** ⇒ `RTW_ALL_TEST_EXIT=0`（没有任何现有测试依赖旧的「空对象成功」语义） |
| `cargo clippy -p axagent-rt-workflow --all-targets --all-features -- -D warnings` | `RTW_CLIPPY_EXIT=0` |
| `cargo test -p axagent --lib stock_analysis_setup` | `test result: ok. 57 passed; 0 failed` ⇒ `SETUP_TEST_EXIT=0` |
| `cargo test -p axagent --lib commands::stock_workflow` | `test result: ok. 100 passed; 0 failed` ⇒ `WORKFLOW_TEST_EXIT=0` |
| `cargo clippy --workspace --all-targets --all-features -- -D warnings`（CI 口径） | `CLIPPY_WS_EXIT=0` |
| `cargo fmt --all --check` | 漂移计数 `0` |

### (4) 剩余（#7① 本体，即 B-2）

前置一的第 3 条与「四档换 SubWorkflow」本身**还没做**，且 B-1 不足以让它可执行：
1. `SubWorkflowConfig`（`harness/src/workflow_types.rs:744-752`）的 `is_async` / `cache_enabled` /
   `cache_ttl_secs` 是声明未消费的**死配置**（`subworkflow_executor.rs:54-55`）⇒ 改图前先裁：
   补实现 or 退役字段（按「退役形状前先盘下游消费者」走）。
2. 子图拿的是**变量快照**（`engine/mod.rs:2723` 起 `variables: Some(...)`），子图内部输出**不回父池**，
   只按 `node_id` + `output_var` 双键写回（`engine/mod.rs:1765` 写 `node_id`，`1354-1356` 注明 `update_node_status_for_execution` 同时写两个 key）⇒ 四档各自的
   23 个逐档分析师实例、`horizon_branch_json`、`dqi_*` 等**全部要显式列进子模板的 input 映射**，
   缺一键即整节点硬错（`subworkflow_executor.rs:95-100`）。⇒ 必须配一道「扇出映射键齐备性」门，
   而不是靠人抄清单。
3. 父 DAG 对 SubWorkflow 是**不透明叶子**（`dag_store.rs` 零特殊处理）⇒ A3 那道
   `check-tier-purity.mjs` 的 R1/R3（四档 `tier_score` 来源互不相同、错峰声明）在改图后**语义失效**，
   要同步换判据；不换就是「门继续绿而它锁的东西已经不在了」。
4. 子执行是独立 `execution_id` 且被前端过滤（`workEngineStore.ts:378`）⇒ 进度面板看不到子图内部节点，
   四档分支的运行状态会**从界面上消失**。这条要么补观测（把子执行的 step 事件转发到父执行），
   要么在改图前就明确否决「四档 SubWorkflow 化」。
5. 需要**新建四档子模板行**（`sub_workflow_id` 必须是库里存在的模板 id，
   `subworkflow_executor.rs` 现读现建）⇒ 属播种内容 ⇒ 发版顺延 **v135**，且实机核对被 C1/#48 挡。

## 八十六、B-2a 执行规格：子执行观测转发（用户 2026-10-06 裁定「先补观测，再改图」）

**这一步不改股票图 ⇒ 不发版**（判据同 §八十二(4)、§八十五）。

### (0) 普查读出的三条现成事实（决定了方案形状，别再重复推导）

1. **子执行的事件其实已经在往父界面发**：引擎在构造子 RunOptions 时把**父的 progress 回调原样传下去**
   （`engine/mod.rs:2741-2743` 与 `3546-3549`，`opts.with_progress_callback(cb)`），
   所以子图每个节点的 step 事件都到了前端 —— **不是没有数据，是被前端按 id 丢了**。
2. **丢弃点有三处，都是同一条 id 守卫**：`src/stores/feature/workEngineStore.ts:310`（step 事件）、
   `:333`（状态/完成）、`:378`（终态载荷）——`payload.execution_id !== executionId ⇒ return`。
3. **引擎在启动时就已知子执行 id 并且有登记表**：`execution_state.rs:276` 的
   `child_executions: Arc<Mutex<...>>`（`:324`/`:364` 构造），`subworkflow_executor.rs:301` 传入 tracker，
   `:308` 生成 `child_execution_id`；节点完成后还把 `_child_execution_id` 写进**节点输出**
   （`subworkflow_executor.rs:352`）⇒ 事后归并可行，但**live 期需要引擎在载荷里带上归属信息**。

### (1) 设计（按「不新建 IPC 命令、不动持久化、不产生 node_id 碰撞」三条约束选形）

1. **载荷加两个字段而不是改 id**：step / status / 终态三类事件在**子执行**上多发
   `parent_execution_id`（`RunOptions.parent_execution_id` 已经在传，`engine/mod.rs:2733`/`3520`）
   与 `sub_workflow_node_id`（父图里那个 SubWorkflow 节点的 id，回调闭包处可得）。
   ⚠ **不要**把子节点记录直接平铺进父的 `node_records`：父子模板的节点 id 可以同名
   （四档子模板就是同一张模板跑四次 ⇒ `trigger`/`end` 全撞），平铺会互相覆盖 ——
   这一条是本规格里最容易做错的地方，做错了表现为"看起来有数据但四档只有一档"。
2. **前端三处守卫改成语义等价的两句**：
   `payload.execution_id === 本执行` ⇒ 原路径不变；
   否则若 `payload.parent_execution_id === 本执行` ⇒ 写进新增的
   `childNodeRecords: Record<childExecutionId | subWorkflowNodeId, NodeRecord[]>`，
   按 SubWorkflow 节点 id 归组（live 期用 `sub_workflow_node_id`，事后 `_child_execution_id` 只作校验）。
3. **呈现**：`WorkflowProgressPanel` / `AgentPoolPanel`（`src/components/chat/`，现对
   `sub_workflow` **零匹配**）在 SubWorkflow 节点行下渲染一个可折叠的子节点列表；
   新 key 走 i18n **11 语言**（`check-i18n-key-parity.mjs` + `check_i18n.py` 两道正门，别散落到子段）。
   缺席口径照既有纪律：拿不到 `sub_workflow_node_id` 的旧事件**不得**被归到"第一个 SubWorkflow 节点"，
   要么单独成一句"子执行归属未知"，要么不显示 —— 混归就是把猜测伪装成事实。
4. **反向锁**（必须有，否则这批改动能静默失效）：
   ① 引擎侧单测：子执行的 step 载荷含 `parent_execution_id` 且含父节点 id；
   ② store 单测：**同 id 的父事件与子事件各来一条**，父记录与子分组记录互不覆盖（这条就是防 node_id 碰撞的）；
   ③ store 单测：既不属于本执行、也没有本执行父 id 的事件**必须仍被丢弃**（防把守卫改成"全收"）。

### (2) 判定闭环的条件

- 四档静态展开的现状下也能验：给一个**临时**带 SubWorkflow 节点的最小图跑一次，看子节点出现在面板里
  （不要等 B-2b 才有测试载体）。
- 全部门禁按 CI 口径复跑：`cargo fmt --all --check`、`cargo clippy --workspace --all-targets --all-features -- -D warnings`、
  `__TAURI_WORKSPACE__=true cargo test --workspace --no-fail-fast`（要报**套件数**，不带 `--no-fail-fast` 的全量可能只是前缀）、
  `npm run typecheck`、`npm run lint:eslint`、`npx vitest run`、`node scripts/ci-check.mjs --skip-rust`。
- 实机核对（`npm run tauri dev` 跑一次带子工作流的执行）被 **C1/#48** 挡不挡：**不挡** ——
  这条走的是内存事件 + 前端 store，不读冷页；但仍需一次真运行才算"界面看得到"。

### (3) 完成 B-2a 之后才轮到 B-2b（改图本体）

B-2b 的剩余前置见 §八十五(4)：死配置裁定、扇出键齐备性门、`check-tier-purity` 判据换形、
新建四档子模板行 ⇒ **v135** + 重播种实机核对（被 #48 挡）。
**顺序不可调换**：先改图再补观测，会让四档在一段时间里既无面板可见性又无静态展开可回退。

### (4) 落地时要照抄的先例与已数好的工作量（本轮只读普查所得）

- **同类缺陷的先例**：`engine/mod.rs:6322-6348` 的 V58/V60/2026-09-08 三段修复，治的就是
  「Debate 容器内部辩手子节点不产生任何 StepProgressEvent ⇒ 前端整段卡在等待中」。
  它给出的两个做法要沿用：① 用**真实 workflow execution_id**（不是 `node_{uuid}` 那种 per-node id）；
  ② 容器内部事件的 `total_nodes/completed_nodes` 填 **0**，因为前端对 0 有守卫（保留原进度百分比），
  否则子图计数会污染父图进度条。**这一条是本批最容易做错的第二个地方**（第一个是 node_id 碰撞）。
- **要改的构造点已数清**：`StepProgressEvent` 字面量构造共 **9 处**
  （`engine/mod.rs` 7 处 + `executors/agent_executor.rs` 1 处 + `harness/src/workflow_types.rs` 1 处默认构造）
  ⇒ 加字段是**编译期强制**的，不会静默漏（这正是把归属做成一等字段而不是塞进 `output` 的理由）。
- **DTO 归属字段的选型**：`harness/src/workflow_types.rs:2029` 的 `StepProgressEvent`
  加**一个** `Option<子执行归属>`（含父 execution_id + 发起它的父节点 id），
  `#[serde(skip_serializing_if = "Option::is_none")]` ⇒ 旧消费端零影响；
  父节点 id 从 `RunOptions` 新增字段带下来（两处启动点 `engine/mod.rs:2755`/`3563` 的闭包里有 node 上下文）。
- **前端三处守卫的行号**：`src/stores/feature/workEngineStore.ts:310`（step）、`:333`（状态/完成）、
  `:378`（终态载荷）。三处都要改，**只改一处**会得到"进度条动了但面板没有子节点"这类半生效形态。
- **面板侧现状**：`WorkflowProgressPanel` / `AgentPoolPanel` 对 `sub_workflow` **零匹配** ⇒ 需要新增渲染分支 + i18n。
- ⚠⚠ **本条下面那句「包装回调盖章」的方案已被实测否证，正确形状写在 (5)，动手前读 (5)。**
- **形状的修正（2026-10-06 收尾时想清楚的一条，省掉一半工作量）**：
  归属**不要在 9 个构造点各填一次**，而是在**两处子执行启动点包装 progress 回调**时统一盖章 ——
  全文件只有两处 `opts.with_progress_callback(cb)`（子执行启动点，一处在引擎内联子图路径、
  一处在 `sub_workflow_callback` 递归路径，2026-10-06 实测在 `:2747` 与 `:3550` 附近，
  **按内容定位而不是抄行号**）改成先 `stamp(cb, parent_execution_id, parent_node_id)` 再装入
  （`parent_execution_id` 在同处的 `RunOptions` 里已有；父节点 id 在同一闭包可得，动手时先确认它叫什么）。这样做的三个理由：
  ① 事件构造点一处不用改，DTO 只加**一个** `Option` 字段；
  ② 子图**内部**的容器事件（loop / debate 体内节点，`build_loop_body_dispatch` /
     `build_debate_body_dispatch` 那 3 个构造点）也自动带上归属 —— 按构造点填会把这部分漏掉，
     而漏掉的形态正是 2026-09-08 那次「辩论期间前端整段卡住」的同款；
  ③ 盖章只可能发生在子执行上 ⇒ 顶层执行的 `origin` 恒 `None`，不存在"忘记填"的静默失效面。
  签名已取到（`engine/mod.rs:218`）：`ProgressCallback = Arc<dyn Fn(StepProgressEvent) ->
  Pin<Box<dyn Future<Output = ()> + Send>> + Send + Sync>` ⇒ 包装闭包按值收事件、
  填 `sub_workflow_origin` 后转调内层即可，不需要额外装箱层；
  该类型定义在 rt-workflow 而 `StepProgressEvent` 在 harness（`engine/mod.rs:215` 再导出）
  ⇒ 新字段加在 harness 侧，盖章函数放 rt-workflow 侧，别放反。
- **本批可拆成两半先落一半**：后端（DTO + 两处盖章 + Rust 单测）是**纯增量**，
  前端未消费时界面零变化、也不会回归 ⇒ 可以先落并单独跑门禁，前端半批随后接。
  （**本轮结束时这一半也尚未开工** —— 已核验的是 B-1，不是 B-2a；不要把上面的规格当成已完成改动。）
  拆的判据是「新字段在被消费前不改变任何现有行为」，不是为了少跑门。

### (5) 第二次修正（2026-10-06 动手后量出的真实形状 —— **覆盖 (4) 的「包装盖章」方案**）

我把 (4) 的方案落到代码后**判定它不成立并撤回**，工作树回到 §八十五 的 B-1 状态
（`git diff HEAD` 对 `harness/src/workflow_types.rs` 与 `subworkflow_executor.rs` 均 **0 行**；
`cargo test -p axagent-rt-workflow --lib` 294/0；`cargo fmt --all --check` 漂移 0）。三条否证事实：

1. **回调签名里没有父节点 id**：`SubWorkflowCallback = Fn(sub_workflow_id, parent_execution_id, input)`
   （`subworkflow_executor.rs:47`）。归组键必须是父节点 id（(4) 的串档论证成立），
   所以**签名要加第 4 个参数**由 executor 从 `sub_node.base.id` 传入 —— 不是"回调自己取"能解决的。
2. **「补发一条子执行已启动事件」有不可消除的乱序**：子执行 id 在回调内部才生成，而
   `tokio::task::spawn_blocking` 是**立即启动** ⇒ 归属事件可能排在子执行的头几条节点事件之后。
   修它的代价（前端缓冲孤儿事件 + 超时丢弃 + 重试期多个子执行）比让归属**随每条事件携带**更大。
   ⇒ 形状改成：`RunOptions` 带 `sub_workflow_origin` → 落到 `ExecutionState` → 事件构造点从 `ctx` 读。
3. **构造点不止 8 个**：`ExecutionState` 另有 **7 处测试字面量构造点**（`dispatcher.rs:527`、
   `execution_state.rs:527`、`code_executor.rs:691/918`、`condition_executor.rs:420/495`、
   `llm_classifier_executor.rs:604`），加字段是编译器强制全改的。
   ⇒ 本批真实规模 ≈ **20 处构造点** + commands 层新事件名 + 前端三守卫 + 面板渲染 + 11 语言 + 三侧测试。

**给下一轮的形状**：按 (5)2 做（归属随事件走），别再走 (4) 的盖章方案；
`ExecutionState` 上只需新增**父节点 id** 一项 —— 它的 `parent_execution_id` 本来就有
（`execution_state.rs:285`），事件里由两者合成 `SubWorkflowOrigin`。

### (6) 第三次读数（2026-10-06 动手前的第二次普查 —— **(5) 的两条数字是错的，此处按实测改口**）

按「根因落笔前先取一次现网读数」重量 (5)，两条被否证：

1. **「`ExecutionState` 另有 7 处测试字面量构造点」不成立**。那 7 行是
   `fn make_test_exec_state() -> ExecutionState {` 这类**函数签名**（`grep "ExecutionState {"`
   把函数体的左括号一起命中了）。真实**结构体字面量全仓只有 1 处**
   （`llm_classifier_executor.rs:609`，且它写的是 `..ExecutionState::new(...)`），
   其余一律走 `ExecutionState::new()` ⇒ 加一个字段 = 声明 + `new()` + `from_snapshot()` 三行，
   编译器把该改的地方**全点出来**（这是加字段相对加参数的真实优势）。
   ⇒ (5)3 的「≈20 处构造点」作废；本批真实规模：harness 1 个新类型 + 8 个 `StepProgressEvent`
   字面量 + rt-workflow 侧 RunOptions/ExecutionState/回调 4 参 + 前端 3 守卫 + 面板 + 11 语言。
   **判据：`grep 'TypeName {'` 命中数≠构造点数**，函数签名尾巴的 `{` 会冒充；
   要数构造点得看 `= TypeName {` / `TypeName \{\s*$` 或干脆数编译器报的 E0063。
2. **`(5)` 末尾那句「`ExecutionState` 的 `parent_execution_id` 本来就有 ⇒ 只需新增父节点 id」也错**，
   错在一个字段被重载：主循环给**每节点**的 `exec_ctx` 赋的是
   `exec_ctx.parent_execution_id = Some(execution_id.clone())`（`engine/mod.rs:3418`，2026-10-06 按内容重指；注释自己写明
   这是为了"让容器执行器找到 `execution_workflows`"，因为 per-node 的 `execution_id` 是 `node_{uuid}`）。
   ⇒ 同名字段在**真实 ExecutionState**（`:2181` 从 options 取，= 真父）与**per-node exec_ctx**
   （`:3418`，= 自己）里是**两个意思**。若按 (5) 原计划"两者合成 origin"，子执行的每条事件都会把
   **自己的 id 说成父 id** ⇒ 前端把子节点归到不存在的父，正是本条要防的伪装。
   ⇒ 形状改成：新增**专用**字段 `sub_workflow_origin: Option<SubWorkflowOrigin>`（父执行 id + 父节点 id
   两项都在里面），**不复用** `parent_execution_id`；容器内节点记录那条既有歧义（`:6592` 把 ctx 的
   重载值写进 `NodeExecutionRecord.parent_execution_id`）**不在本批顺手改**，另计（见 (7)）。

顺带一条前端读数（同一处普查撞见的既有缺口，本批一并收）：`workflow:state-changed` 的
`node_records` 里前端类型声明了 `parent_execution_id?`（`workEngineStore.ts:372`、
`src/types/index.ts:1734`），而后端发射器（`src/commands/work_engine.rs:531-545`）**从不序列化该键**
⇒ `parentExecutionId` 恒 `null`，"字段声明有、生产者不发"＝界面拿不到又不说为什么。
另：DebugPanel **已有**事后子执行抽屉（`:1774` 读 `output._child_execution_id` 再取详情），
所以 B-2a 缺的只有 **live 期**（子执行跑的时候父面板看不到里面），不是"完全没有子执行视图"。

### (7) 本批范围边界（防止顺手改出不属于本批的数值变化）

- 只做「子执行事件可归属 + 父面板 live 期渲染子节点」两件事；
- **不**改 `NodeExecutionRecord.parent_execution_id` 在容器路径上的既有取值（`(6)2` 末段）：
  它会影响已持久化记录的读法，属另一条缺陷，按对账另立条目；
- **不**碰股票分析链的 `stock_workflow/core.rs` 转发器（那条链现在按设计是静态展开×4，
  子执行只出现在 B-2b 之后）。

## 八十七、B-5 第一步实测（2026-10-06，本机网络，只做 GET 探活）

用户裁定 #18 = 换官方源。动手前的第一条判据是「这台机器能不能真拿到官方数据、拿不拿得到**真发布日**」
（现网 `macro_data.rs` 的 as-of 是靠 `macro_available_at` 规则**推断**发布日，并把"推断"这件事写进
`source` 字段供界面点名 —— 换源只有换来真发布日才算收紧，否则只是换了一个同样要推断的上游）。

### (1) 已取得的读数

| 目标 | 结果 |
|---|---|
| `https://data.stats.gov.cn/easyquery.htm?m=QueryData&dbcode=hgyd&…`（统计局月度同比数据 API，带浏览器 UA） | **`403`，1.40s，响应体 312B** ⇒ 域可达但**数据接口按反爬拒绝裸 GET** |
| `https://www.stats.gov.cn/` | `200`，0.52s，141 KB ⇒ 主站不被本机阻断（与 `push2*` 那种累积/速率触发 RST 的形态不同） |
| `https://www.pbc.gov.cn/` | `200`，0.37s，141 KB |
| `http://data.www.pbc.gov.cn/sdgds/` | **DNS 解析失败**（我按猜的子域拼的 URL，实测否证；央行真实数据子域待查） |

### (2) 第二轮（用户 2026-10-06「点头」授权后补做）—— 未证已关掉，且**否证现网规则**

1. **数据 API 这条路是死的**：带上主站返回的 4 行 cookie + `referer: data.stats.gov.cn/easyquery.htm`
   + 浏览器 UA 重试同一 URL，仍回 **`403 … reason:UrlACL`**（响应里带 eventID 与本机出口 IP）
   ⇒ 这是 **URL 级策略拒绝**，不是缺会话、不是缺 UA。⇒ 换源**不能**做成「把端点从 eastmoney 换成 easyquery」。
2. **真发布日拿得到，且不需要推断**：`/sj/zxfb/`（最新发布列表页，HTTP 200 / 73 013 B）
   每条目的 URL 内嵌发布日（`t20260909_1965263.html`），标题给期次 ⇒
   「期次 2026-08 ⇒ 发布 2026-09-09」是**读出来的事实**，不是日历规则。
3. **数值也从可达页面拿得到**：那条 CPI 公报详情页 HTTP 200 / 245 888 B，正文含「同比上涨0.8%」，
   另有 **2 张 HTML `<table>`**（分类指数）；页内**无内嵌 JSON 变量**（`var x = {` 计数 0）
   ⇒ 取数形态是 **HTML/表格解析**，不是 JSON 接口。
4. **实测出的真实发布日 vs 现网推断**（同一列表页抽样）：

| 序列 | 期次 | 官方真发布日 | 现网 `macro_available_at` 推断 | 偏差 |
|---|---|---|---|---|
| CPI | 2026-08 | **2026-09-09** | 次月 15 日 = 09-15 | 推晚 **6** 天 |
| PPI | 2026-08 | **2026-09-09** | 次月 15 日 = 09-15 | 推晚 **6** 天 |
| PMI | 2026-08 | **2026-08-31** | 月末 + 2 = 09-02 | 推晚 **2** 天 |
| PMI | 2026-09 | **2026-09-30** | 月末 + 2 = 10-02 | 推晚 **2** 天 |

⇒ 现网推断的**方向是保守**（把可得日推晚），不构成前视，但会在「真实已发布 → 推断未发布」这段窗口里
**人为制造缺席**（GDP 那条「次月 20 日」本轮未取到样本，仍属未证）。

### (3) 由实测能定的三件事

1. 路线**只剩一条**：数据 API 被 UrlACL 挡死 ⇒ 「换个端点」不成立；能做的只有
   **列表页（期次 + 真发布日）+ 公报详情页（数值）的 HTML 解析**。代价是新增解析依赖与新的失败模式
   （改版、表格列序、限速），收益是 `macro_available_at` 的推断规则**可整段退役**，
   并消掉回放里那 2–6 天的人为缺席。

2. 无论选哪条，登记面按 §八十四 前置二末段：做成 vendor 是 8 处 + 快照白名单 + `as_of_capability` 申报；
   不换 vendor 只换端点，则 `MACRO_SERIES` / `fetch_series` / `parse_macro_rows` / `MacroDataSnapshot`
   三处形态必须同批（否则字段错位是静默的）。
3. 消费面窄到只有一条键 `pmi`（`seed_stock_analysis.rs:6081-6082` 长档 + `leg_signal.rs:119`），
   Rhai 零消费 ⇒ 换源的**决策影响半径小**，但界面后果也小 —— 别把它当高收益项排到 B-3 前面。

### (4) 排序结论（写下来，防下一轮又把它当前置）

#18 实测后**成本上调、收益半径未变**（消费面仍只有 `pmi` 一键，`*.rhai` 零消费），
而 B-3（归档升格历史表）是 13 个读点的公共底座 ⇒ **B-3 排前、#18 排后**。
且 #18 开工时按 (3)1 的路线做，**不要再试 `easyquery.htm`**（已实测 URL 级拒绝，两轮读数在 (1)(2)）。

## 八十八、B-2a 落地：子执行进度不再从父面板消失（2026-10-06，**不发版**）

用户裁定顺序「先补观测，再改图」⇒ 本批只做观测，股票图一字未动 ⇒ **不 bump `TEMPLATE_VERSION`**
（判据同 §八十二(4)：生成的图与 `include_str!` 资产没变，快速链走内容门禁也不会重建）。

### (1) 落地形状与 §八十六(5)/(6) 的差

1. **归属不合成、直接带两项**。(5) 末尾说「`ExecutionState` 只需新增父节点 id，事件里由
   `parent_execution_id` + 它合成」—— (6)2 已证 `parent_execution_id` 在 per-node `exec_ctx`
   上是「本执行 id」，拿它合成会把子执行自己的 id 说成父 id。⇒ 改成新增**专用**字段
   `sub_workflow_origin: Option<SubWorkflowOrigin>`（两项都在里面），从 `RunOptions` 一路带到
   真实 `ExecutionState`、per-node `exec_ctx` 与快照往返。
2. **父执行 id 不取回调参数**。`subworkflow_executor` 传的 `parent_execution_id` 实为
   `context.execution_id` = `node_{uuid}`（per-node ctx 的既有形态）⇒ 引擎在两处启动点改用自己
   作用域里的真实 `execution_id`（局部 `sub_origin_parent_eid`），回调新增的那一参只补
   「父节点 id」。既有 `parent_execution_id` 参数**不动**：它唯一的消费者是
   `is_sub_workflow = options.parent_execution_id.is_some()`（`engine/mod.rs:4826`），值不影响判定，
   本批也不假装修好它。
3. 回调签名因此 3 参 → 4 参（`sub_workflow_id, parent_execution_id, parent_node_id, input`），
   第 3 参只有执行器知道（`sub_node.base.id`）⇒ 这就是 (5)1 的结论，与「包装回调盖章」无关。
4. (6)1 的「构造点数被 `grep 'TypeName {'` 虚报」在本批兑现：`ExecutionState` 只加了
   声明 + `new()` + `from_snapshot()` + 快照结构 + `to_snapshot()` **五处**，测试侧零改动
   （那 7 行确实是 `fn … -> ExecutionState {` 签名）。

### (2) 改动清单（`文件:行` 全部为落地后实测现值）

| 层 | 位置 | 改了什么 |
|---|---|---|
| DTO | `crates/harness/src/workflow_types.rs:2020`（`SubWorkflowOrigin`，带 `rename_all = "camelCase"`）、`:2053`（`StepProgressEvent.sub_workflow_origin`，`default + skip_serializing_if`） | 新类型 + 新可选字段。顶层执行恒 `None` ⇒ 线上载荷连键都不出现，旧消费端零影响 |
| 引擎·选项 | `engine/mod.rs:180`（`RunOptions.sub_workflow_origin`）、`:279`（`Default`） | |
| 引擎·传递 | `:2194`（真实 state 拷入）、`:2372`（取一次存局部）、`:3422` 与 `:4690`（两处 per-node `exec_ctx` 都赋） | 少赋一处就是半生效形态 |
| 引擎·启动点 | `:2682`/`:3477`（`sub_origin_parent_eid` 取父真实 id）、`:2690`/`:3488`（回调第 4 形参）、`:2767`/`:3589`（`sub_workflow_origin: Some(…)`）、`:2709`/`:3510`（每次调用再 clone 一份进 `spawn_blocking`，否则 `Fn` 退化 `FnOnce`） | 两处子执行启动点同批 |
| 引擎·事件 | 主循环 4 条读局部（`:3326` running / `:3955` completed / `:4318` failed / `:4541` timeout）、容器 dispatch 3 条读 `ctx`（`:6580`/`:6663`/`:6727`）、`executors/agent_executor.rs:1403`（streaming 读 `context`） | **8 个构造点全覆盖** —— 漏一个就是「该节点的事件没有归属」 |
| 执行器 | `executors/subworkflow_executor.rs:53`（签名 + 注释）、`:80`（`RetryParams.parent_node_id`）、`:136`（cb 传 4 参）、`:312`（`parent_node_id: sub_node.base.id.clone()`） | 唯一能给出父节点 id 的地方 |
| 状态 | `execution_state.rs:297`（字段 + 那段「同名字段两种含义」警示）、`:324`（`new()`）、`:409`（`from_snapshot`）、`:494`（快照结构，`#[serde(default)]` 兼容旧快照 JSON）、`:514`（`to_snapshot`） | 快照往返必须带，否则「暂停恢复后子节点又消失」 |
| 其它签名 | `src/commands/cognitive.rs:1208`（系统能力回调补第 4 形参 `_parent_node_id`） | 类型共用，编译器点的名；系统能力不产生子执行 ⇒ 如实忽略 |
| commands | `src/commands/work_engine.rs:509`（取 origin）+ `:526`（node-status 载荷）+ `:566`（state-changed 载荷，取 `full_state.sub_workflow_origin`） | `workflow:execution-completed` **不改** —— 子执行不走那条，见 (5)1 |
| 前端 store | `src/stores/feature/workEngineStore.ts:34`/`:55`/`:75`（三个纯函数：归属判定 / 增量合并 / 全量覆盖）、`:101`（`subFlow` 状态字段）、`:146` 与 `:377`（初始 + `resetDebug` 清空）、`:398`（node-status 监听）与 `:482`（state-changed 监听） | 判定抽成纯函数由两条监听器**共用**；父执行分支逐字不变 |
| 类型 | `src/types/index.ts:1754`（`SubWorkflowOrigin`）、`:1826`（`SubWorkflowProgress`） | 禁区 13：Rust snake_case + 线上 camelCase + TS camelCase |
| 面板 | `src/components/workflow/DebugPanel.tsx:565`（`openSubExecution` 提出来给两条路共用）、`:679`（`subFlow` 选择器）、`:1818`/`:1826`（SubWorkflow 节点详情里的实时段 + 缺席句） | 「查看子执行」原来只在节点**完成后**出现（靠 `output._child_execution_id`）⇒ live 期也能点了 |
| i18n | 11 语言 × 2 键（`debugPanel.subFlowLive` 带 `{{count}}`、`debugPanel.subFlowPending`），插在 `subWorkflow` 之后保序、LF 落盘 | |
| 测试 | Rust：`subworkflow_executor.rs:405` 起新 `mod tests`（2 条）+ `execution_state.rs:664`（快照往返 1 条）；TS：`src/stores/feature/__tests__/workEngineStore.subFlow.test.ts` 16 条（6 归属判定 / 5 桶 / 3 生产端反手抄 / 2 键 × 11 语言真译） | 变异自证见 (3) |

### (3) 门读数与变异自证

#### (3.1) Rust 侧（一条命令一个退出码，不串 `&&`）

| 命令 | 读数（日志原文） |
|---|---|
| `cargo check -p axagent --all-targets` | `Finished dev profile … in 9m 15s` ⇒ `MAIN_CHECK_EXIT=0`，`^error` 计数 **0** |
| `__TAURI_WORKSPACE__=true cargo test -p axagent --lib` | `test result: ok. 547 passed; 0 failed; …` ⇒ `MAIN_LIB_EXIT=0` |
| `cargo clippy -p axagent --all-targets -- -D warnings` | `Finished dev profile … in 9m 29s` ⇒ `MAIN_CLIPPY_EXIT=0`，`^error` 计数 **0** |
| `cargo test -p axagent-rt-workflow --all-targets --no-fail-fast` | **16 个测试二进制全绿**（`RTW_TEST_EXIT=0`）；lib 套件 `297 passed; 0 failed`（= 改前 294 + 本批 3 条新测试） |
| `cargo clippy -p axagent-harness -p axagent-rt-workflow --all-targets --all-features -- -D warnings` | `Finished … in 1m 47s` ⇒ `TWO_CLIPPY_EXIT=0` |
| `cargo fmt --all --check` | `FMT_ALL_EXIT=0`，`^Diff in` 计数 **0** |

新测试三条：`subworkflow_executor` 两条（`callback_receives_id_of_launching_node_and_parent_execution`、
`two_sub_workflow_nodes_on_same_template_get_distinct_parent_node_ids`）+ `execution_state` 一条
（`sub_workflow_origin_survives_snapshot_round_trip`）。第一条用「回调把收到的参数原样塞进子执行输出」
的替身 ⇒ 断的是**实际传了什么**，不需要 `std::sync::Mutex` 记录调用（那会踩 `disallowed_types`，
且记录式替身查不出"传了但传错"）。

#### (3.2) 前端 / node 侧

| 命令 | 读数 |
|---|---|
| `npm run typecheck` | `EXIT=0` |
| `npm run lint:eslint`（oxlint） | `Found 0 warnings and 0 errors.` ⇒ `EXIT=0` |
| `npx dprint check` | `EXIT=0` |
| `bash scripts/check-line-endings.sh` | `All line endings check passed.` ⇒ `EXIT=0` |
| `npx vitest run` | `Test Files 195 passed (195)` / `Tests 2177 passed (2177)` ⇒ `EXIT=0` |
| `node scripts/ci-check.mjs --skip-rust` | `全部检查通过! (耗时 169.1s)` ⇒ `EXIT=0`（含档位权重 / HorizonDecision 键集 / 分析师 id 形态 / tier-purity / i18n key 对齐 / dprint / tsc / vitest 全套） |
| `python scripts/check_i18n.py` | `🎉 全部通过` ⇒ `EXIT=0` |

#### (3.3) 变异自证（两条，都验过"会红"再改回）

| 变异 | 结果 |
|---|---|
| 去掉 `SubWorkflowOrigin` 的 `#[serde(rename_all = "camelCase")]` | `× DTO 走 camelCase ⇒ 推导出消费端应读的键名，并要求 store 源码逐个用到` ⇒ `1 failed \| 15 passed` ✔ 红 |
| 去掉 `work_engine.rs` node-status 载荷里的那一条 `"sub_workflow_origin": origin,` | `× commands 层两条载荷都发 sub_workflow_origin` ⇒ `1 failed \| 15 passed` ✔ 红 |

两条都改回后 `Tests 16 passed (16)`。**为什么挑这两条**：它们是本批唯一「两侧靠拼写对齐」的面
（Rust DTO ↔ 前端 store），编译门查不出线上拼写漂移 ⇒ 只能靠反手抄 + 变异证它有没有电。
另有一条自抓到：`store 必须读 parentExecutionId` 的初版用裸名断言 ⇒ `parentExecutionId`
在 `node_records` 映射处**早就存在**，那条断言恒真；改成断 `origin.parentExecutionId`
（取值形态）才有电。

#### (3.4) 全量口径的归属

`clippy --workspace --all-features` 与 `test --workspace` 按 AGENTS.md 的分层策略
**留到 B 组全部阶段收尾时跑一次**（本批是阶段内）。上一次全量读数＝工作集 136 套件全绿（§七十九 批）。

### (4) 界面后果（三段）

- **代码面**：闭环 —— 父子同名节点不互相覆盖（按子执行 id 分桶），归属随每条事件 ⇒ 无乱序面。
- **现网行为**：**当前生产图里没有任何 SubWorkflow 节点**（股票主链是静态展开×4；全图
  `SubWorkflow` 零实例 —— §七十九 B 组 #7① 同一条事实）⇒ 本批在现网**一个像素都不变**。
  它是 B-2b 的**前置**，不是 B-2b 的收益，别按「四档面板好了」来验收。
- **界面会多出什么**：编辑器调试面板里被标成 `subWorkflow` 的节点（认知编排器的 `system_*`
  除外 —— 它不产生子执行）在运行时多一段「子执行实时进度（N 个）」，每行 = 一次子执行
  （重试就有多行，互不覆盖）+ 它的 `节点 id → 状态` 标签 + 「查看子执行」入口。
  跑起来了但事件还没到的那一瞬显示「子执行事件尚未到达」，**不**显示成「没有子节点」
  —— 那是把「还没到」说成「没有」。
- **仍待实机核对**：需要一张真带 SubWorkflow 节点的图跑一次。#48（PG 冷页读全挂）挡的是
  「从库里翻历史执行」那部分；本条走内存事件 + `get_workflow_execution_status` 命中
  `self.executions`，理论上不依赖冷页 —— 但**未实测**，故按未闭环记。

### (5) 本批刻意没做的三件（防止下一轮当成已完成）

1. `workflow:execution-completed` 那处守卫**没改**：子执行不 emit 那条（只有
   `debug_run_workflow` 用父 id emit）⇒ 改它是空改。§八十六(4) 的「三处都要改」实测后
   收窄成**两处**。
2. `NodeExecutionRecord.parent_execution_id` 在容器路径上被写成「本执行 id」（读的就是那个
   重载字段）⇒ **未动**，另立条目；不顺手修，因为它会改已持久化记录的读法。
3. 前端 `node_records` 声明了 `parent_execution_id` 而后端发射器不序列化（§八十六(6) 末段
   既有缺口）⇒ 本批**仍**不补：补了会把 (2) 的容器路径错值直接暴露到界面（父执行 ID 标签
   显示成子执行自己的 id）。要么先修 (5)2，要么保持缺席。
4. **同一条普查顺手量出的既有缺陷（另立条目，不在本批修）**：
   `src/stores/shared/backendStatusStore.ts:153` 也监听 `workflow:node-status-changed`，
   但它**完全没有 execution_id 守卫**，且任务 id = `wf-node-${node_id}` ⇒
   「全局后台任务列表」里父子同名节点、以及**并发两次执行**的同名节点一直在互相覆盖。
   两个后果：① §八十六(0) 那句「子执行事件不是没有、是被前端按 id 丢了」只对
   `workEngineStore` 成立，对这个 store **不成立**（它从不筛 ⇒ 子执行节点早就混在列表里，
   只是混得没有依据）；② 修它＝给列表加 execution 作用域 ⇒ **会改变可见条目**
   （原本混进来的会被分桶或消失），属呈现层变更 ⇒ 与 B-2b 一起裁，不在本批顺手改。

## 八十九、B-2b 前置 1 落地：`is_async` 与执行器 cache 旋钮退役（2026-10-06，**不发版**）

§八十五(4) 的前置 1「改图前先裁死配置」，用户拍 **退役字段 + 撤面板开关**。
这一步只删不参与股票图生成 ⇒ 模板内容一字未动 ⇒ **不占 `TEMPLATE_VERSION`**（判据同 §八十二(4)）。

### (1) 动手前的实测读数（决定"是死配置"而不是"未接线的功能"）

- `SubWorkflowNodeConfig.is_async`：Rust 侧**零读取**（`grep '\.is_async'` 在
  `src-tauri/crates/rt-workflow` 与 `src-tauri/src` 内 0 命中，只剩声明与字面量构造）。
- 但它**在界面上是个开关**：`SubWorkflowPropertyPanel.tsx` 渲染了一个 `Switch`
  绑 `config.isAsync`，用户勾了不产生任何行为差异 ⇒ 属「配置项空接线」那一族
  （2026-09-11 那道门的形态，只是那道门只管 `portfolio-mgr` 的 input_mapping，管不到节点属性面板）。
- 执行器自己的 `SubWorkflowExecutorConfig.cache_enabled` / `cache_ttl_secs`：
  只在 `Default` 里赋 `true`/`300`，全仓零读取 ⇒ 同批一起退役。
  `timeout_secs` / `max_retries` **有电**（`self.config.timeout_secs`、`self.config.max_retries` 都读），不动。

### (2) 退役面（10 处，两侧同批）

| 侧 | 位置 | 动作 |
|---|---|---|
| Rust DTO | `crates/harness/src/workflow_types.rs:744` | 删字段，原位留一条退役注释（写明「Rust 侧零读取而面板有开关」+ 旧模板兼容结论） |
| Rust 字面量 | `src/commands/workflow_ai/helpers.rs`（AI 兜底 config）、`src/init/cognitive_router_init.rs`、`crates/rt-workflow/.../subworkflow_executor.rs`（本轮新测试夹具） | 各删一行 ⇒ 编译器点的名，不会漏 |
| AI 提示词 | `helpers.rs` 的 `subWorkflow` schema 段 | 删 `"is_async": true \| false` 一行。**这条比删字段更重要**：提示词留着，模型就会继续吐这个键，而吐出来的键从此没人读——比原来更静默 |
| Rust 执行器配置 | `subworkflow_executor.rs:56` | 删 `cache_enabled` / `cache_ttl_secs` 两字段 + `Default` 对应两项 |
| TS 类型 | `src/components/workflow/types/workflow.types.ts:323` | 删 `isAsync: boolean`（必填字段 ⇒ 三处默认值必须同批改，否则 tsc 红） |
| TS 默认值 | `WorkflowEditor.tsx` 两处、`SubWorkflowPropertyPanel.tsx` 一处 fallback | 各删一行 |
| TS 面板 | `SubWorkflowPropertyPanel.tsx` | 删整个开关 `<div>` 块 + 随之失效的 `Switch` import |
| i18n | 11 语言 | 删 `workflow.props.asyncExecution`（该键唯一消费点就是被删的开关 ⇒ 留着就是 11 份死文案；parity 门重跑一致） |

兼容结论（这是删字段唯一真正的风险面）：库里已存在的模板 JSON 带 `"isAsync": false` ⇒
serde 默认忽略未知字段，**读取不受影响**；`SubWorkflowNodeConfig` 上也没有 `deny_unknown_fields`。
声明层没有列可退役（不是 DB 列），故不适用「声明层 vs 物理层两句报」那条。

### (3) 门读数

| 门 | 读数 |
|---|---|
| `cargo check -p axagent --all-targets` | `Finished dev profile … in 9m 19s` ⇒ `MAIN_CHECK2_EXIT=0` |
| `__TAURI_WORKSPACE__=true cargo test -p axagent --lib` | `test result: ok. 547 passed; 0 failed … finished in 62.31s` ⇒ `MAIN_LIB2_EXIT=0`（与退役前**同一条数** ⇒ 没有任何测试依赖 `is_async`） |
| `cargo test -p axagent-rt-workflow --lib` | `297 passed; 0 failed` ⇒ `RTW_LIB2_EXIT=0` |
| `cargo fmt -p axagent-harness -p axagent-rt-workflow` + `cargo fmt --all --check` | 0 漂移 |
| `npx dprint check` / `check-line-endings.sh` / `npm run typecheck` / `npm run lint:eslint` | 全 `EXIT=0`（oxlint `Found 0 warnings and 0 errors.`） |
| `node scripts/check-i18n-key-parity.mjs` | `结论: 11 个语言文件的 key 路径集合一致` + 正控「查不存在路径必报不存在」✓ |
| `python scripts/check_i18n.py` | `🎉 全部通过` ⇒ `EXIT=0`（删键后无"代码引用缺定义"） |
| `cargo clippy -p axagent -p axagent-rt-workflow -p axagent-harness --all-targets --all-features -- -D warnings` | `Finished dev profile … in 9m 40s` ⇒ `CLIPPY3_EXIT=0`，`^error` 计数 **0** |
| `npx vitest run` | `Test Files 195 passed (195)` / `Tests 2177 passed (2177)` ⇒ `VITEST3_EXIT=0` |
| `node scripts/ci-check.mjs --skip-rust` | `全部检查通过! (耗时 170.6s)` ⇒ `CI3_EXIT=0` |

（vitest 与 node 门是等 clippy 让出 CPU 之后才跑的 —— 同机并发会造 vitest 超时假红，见门禁 flaky 清单。）

### (4) 界面后果

- 属性面板的 SubWorkflow 节点少一个**本来就没电**的开关「异步执行」—— 用户看到的
  是"一个骗人的控件消失"，不是"一个功能被砍"。11 语言的该文案同步删除。
- 其余无可观察变化：`is_async` 从来没被读过，删它不可能改变执行行为。
- 未做实机核对：与 #30 同批看（都要重启应用）。

### (5) 下一条（B-2b 前置 2）

扇出键齐备性门 —— 子模板只拿**变量快照**，四档那 23 个逐档实例 + `horizon_branch_json` +
`dqi_*` 全部要显式列进子模板 `input_mapping`，缺一键即整节点硬错
（`subworkflow_executor.rs` 的映射解析路径）。判据要在**改图之前**建好，
否则 v135 改完才发现少键，报错点是运行时而不是 CI。

## 九十、B-2b 前置 2 落地：扇出键齐备性判据（2026-10-06，**不发版**）

§八十五(4) 前置 2。这一步不动模板内容 ⇒ 不占 `TEMPLATE_VERSION`。

### (1) 判据从三条代码事实里读出来（不是按命名直觉定的）

1. **子执行只拿一份变量快照**：引擎在两处启动点把父扇出 `input_mapping` 的 **target 键**
   转成 `Variable` 列表传进子 `RunOptions`；子图内部输出**不回父池**。
2. **子图侧解析器是严格私有副本**：`subworkflow_executor.rs:390` 的 `resolve_var_path`
   用 `current.get(part)?` 逐段走，**不做 JSON 字符串穿透** ⇒ 引用一个没传进来的名字就是
   `Variable 'x' not found` 硬错。而 `check-input-mapping.mjs:24-27` 自己记着
   「SubWorkflow / Storage / Validation / LlmClassifier 这四类节点的输入映射**暂不在扫描范围内**」
   ⇒ 这一族此前没有任何门。
3. **节点结果同时写在两个键上**：`engine/mod.rs:1791-1796` 的
   `results.insert(node_id)` + `results.insert(output_var)` ⇒ 下游用 **node_id 形态**（`l1_llm`）
   引用上游是合法的。这条直接决定「什么算自产」的定义，见 (3)。

### (2) 落点形状：Rust 测，而不是又一道 node 门

- 为什么不写成 `.mjs`：判据要的是「子图每个节点读了哪些名字 / 写了哪些名字」，而种子是用
  `data_transformer_node(...)` / `condition_node(...)` 这类**辅助函数 + 位置参数**造的
  ⇒ 文本抽取要按 maker 逐个对齐下标（`check-input-mapping.mjs` 就为 `strings[6]` 错过一次，
  注释里记着那条史）。改成 Rust 测后节点是 `Vec<WorkflowNode>` **类型化**值，`match` 到 config
  字段直接取 ⇒ 下标脆弱性归零，覆盖面由编译器给。
- 判据函数 `audit_fanouts(graphs, templates)` **只产事实不判红**，断言都在测试侧 ⇒
  负控可以拿**同一个函数**验同一套公式（另写一遍比较公式的负控证不到真判据）。
- 位置：`src/init/cognitive_router_init.rs` 末尾新增 `#[cfg(test)] mod tests`
  （存量三张子模板的构造器就在同一文件里，类型化抽取零跨模块成本）。

### (3) 首读即假红，反证就在同一个文件里

门第一次跑在**未改的存量**上就报：子模板 `cognitive_l1_domain_router` 读取的入参 `l1_llm`
不在父 `input_mapping` 的 target 键里。定性：`l1_llm` 是**节点 id** 而不是变量名
—— (1)3 的双键写入让这种引用合法，而本文件 `:49-52` 的修复史注释**自己就写着**
「normalize 表达式改引用上游 node_id key（l1_llm / l1_fallback）」，反证一直在手上我没读。
⇒ 缺陷在抽取面（`produced` 只收了 `output_var`），不在现网。
修法：`produced` 收 `c.base_id()` ∪ 各节点 `output_var`；另补 `root_of_shapes` 形态自证
（点分 / 表达式 / `0.5` 不得当成引用）。
⇒ 沉淀进判据层 **4bt**：写「引用 X 的名字必须有人提供」这类齐备性判据之前，先取
**键的写入面**读数（谁往池子里插键、插几个），别按命名直觉定义「什么算已定义」。

### (4) 第二处漏检：只走主图会漏一整层嵌套扇出

改完假红后用 `--nocapture` 打出「`system_* 排除 0 个`」—— 可 L3 子模板里明明有 2 个。
根因：审计只遍历主图，而那两个扇出（`l3_rar_search`→`system_rar_retriever`、
`graph_router`→`system_workflow_graph_router`）在 `build_l3_router_nodes()` **内部**
⇒ 「子模板内部再扇出」这一整层当时完全没被查，而这正是 B-2b 之后可能出现的形态。
修法：`audit_fanouts` 改收**全部图**（`graphs_to_audit` = 主图 + 每个已登记子模板），
缺口记录带「所在图」字段；`excluded_system` 随之从 0 变成真实计数 2。
⇒ 判据：门自己报出的「排除 / 缺席计数」若与普查读数不一致，那是**作用域漏洞**而不是「没问题」。

### (5) 覆盖面与盲区（写清楚，别让绿被读成全覆盖）

- 声明式字段：`input_mapping` 两侧、`context_sources`、`input_var`、`categories_var`、
  `Condition::var_path`、`ValidationAssertion::expression` 的**根名**。
- **不**覆盖：LLM 提示词与 Rhai 表达式体里的自由变量名。这一族运行期不报错、只拿到 `unit`
  （`seed_consistency_tests.rs` 同一结论）⇒ 本门不假装能查它，登记为已知盲区。
- 引擎自动注入名不算缺键，且**直接取 `executors/mod.rs:91-94` 的常量**而不是抄字面量 ——
  我第一版抄成了 `workflow_model_id` / `workflow_provider_id`（真名是 `__workflow_model__` /
  `__workflow_provider_id__`），虽未致错但属错表，已改成常量；同时删掉一条因此必然恒真的
  「名表对账」测试（自己造的名字自己查＝没有电）。
- `node_var_io` 未覆盖的节点类型必须为空 ⇒ B-2b 带进新节点形却不补 match 臂就会红，
  这是防「门悄悄失去覆盖面」的棘轮。

### (6) 断言清单与读数

| 断言 | 内容 | 读数 |
|---|---|---|
| 无缺口 | 每个扇出的 `needs − produced − injected − provided` 为空 | 在 `sub_workflow_fanout_inputs_are_complete` 内 ⇒ ok |
| 无未登记 | 非 `system_*` 扇出都必须有登记的子模板数据源 | 同上 ⇒ ok（2 个 `system_*` 走排除路径） |
| 扇出面非空 | 抽取器不再认得 `sub_workflow_node` 时本门不得静默绿 | 同上 ⇒ ok（实数 3） |
| 双向一致 | 登记子模板集合 == 实际扇出集合（撤扇出要同步撤表） | 同上 ⇒ ok |
| 无盲区 | `node_var_io` 未覆盖类型集为空 | 同上 ⇒ ok |
| 负控 | 抽掉 `user_input` ⇒ **同一个** `audit_fanouts` 必须报出缺口 | `fanout_missing_target_key_is_caught` ⇒ ok |
| 形态自证 | `root_of` 对点分 / 表达式 / 数字三种输入的行为 | `root_of_shapes` ⇒ ok |

作用域读数（`--nocapture` 原文）：

    审计 4 张图；图内扇出 3 个（cognitive_l1_domain_router,cognitive_l2_cluster_router,cognitive_l3_capability_router）；system_* 能力回调排除 2 个

其余门禁：

- `cargo test -p axagent --lib init::cognitive_router_init` ⇒ `3 passed; 0 failed; 547 filtered out`，`EXIT=0`
- `__TAURI_WORKSPACE__=true cargo test -p axagent --lib`（全量）⇒ `550 passed; 0 failed`（= 547 + 本批 3）⇒ `LIB4_EXIT=0`
- `cargo fmt --all --check` ⇒ `FMT4_EXIT=0`，`^Diff in` 计数 0
- `cargo clippy -p axagent --all-targets --all-features -- -D warnings` ⇒ 见 (6.1)

### (6.1) 修 lint 后的复跑

`clippy` 首跑 **红一次**：`iter_overeager_cloned`（我先 `.cloned()` 再 `.filter()`）
⇒ 改成 `.filter(..).cloned()` 后复跑：

- `cargo clippy -p axagent --all-targets --all-features -- -D warnings`
  ⇒ `Finished dev profile … in 6m 39s`，`CLIPPY5=0`
- `cargo test -p axagent --lib init::cognitive_router_init` ⇒ `3 passed; 0 failed; 547 filtered out`
- `cargo fmt --all --check` ⇒ `FMT5_EXIT=0`，`^Diff in` 计数 0
- `__TAURI_WORKSPACE__=true cargo test -p axagent --lib`（全量复跑）⇒
  `test result: ok. 550 passed; 0 failed … finished in 60.96s` ⇒ `LIB5_EXIT=0`

### (7) 对 B-2b 的直接意义

四档子模板落地时：① 必须在判据的数据源表里登记 4 条子模板，否则 `unregistered` 红；
② 任何一个没被父扇出 target 覆盖的键会让门红，**报错点在 CI 而不是运行期**；
③ 四档子模板里若出现新的节点类型，match 臂必须同步长，否则盲区棘轮红；
④ 若四档子模板内部再套扇出（本轮已因只走主图漏检过一次），`graphs_to_audit` 会把那一层一起扫进去。

## 九十一、B-2b 本体执行规格：四档分支 SubWorkflow 化（**v135**，未开工）

粒度裁定（2026-10-06 用户拍）：**整段搬 —— 每档评分 + 风险 + 决策进子模板**，主图只留 4 个 SubWorkflow 节点。

### (0) 先纠正我给的选项措辞（它按字面不可执行）

我在提问里写的选项是「每档把 `t-scoring-<档>` → `cls-risk-level-<档>` → `pm-h-<档>` 整段搬进子模板」。
普查后的真实边界：

- `pm-h-ultra-short` 的 `input_mapping`（`seed_stock_analysis.rs:5789-5850`）**同时**引用
  本档节点（`t-scoring-hour.result.content.totalScore` 等 4 项、`cls-risk-level-ultra-short.result.category`）
  与**共享**节点（`t-scoring.result.content.kline_json`、`t-limitup-pool.result.content.breadth.{sealRate,breakCount}`）
  与运行时注入变量（`horizon_branch_json.ultra_short`、`horizon_prior_json`、面板 `stop_vol_mult`）。
- ⇒ 能搬的只有**本档那三个节点**；`t-scoring` / `t-risk` / `t-limitup-pool` 是四路共用的上游，
  搬进任一子模板就会让另外三路各拉一遍同样的数据（且违反「一次分析产四档」的既定设计）。
- 回环已验：`horizon_branch_json` 由 `stock_workflow/hooks.rs:391-404` **运行时注入**
  （权威源 `evidence_weight::horizon_branch_specs`），**不是** `portfolio-mgr` 的产出
  ⇒ 「portfolio-mgr 在父、pm-h 在子、pm-h 又读 portfolio-mgr 输出」这个担心不成立。
- 但方向反过来是真的：`portfolio-mgr` 的 `input_mapping` 含四路
  `("h_ultra_short", "pm-h-ultra-short.result")` 等（`:5295-5298`）⇒ **扇出必须在 `portfolio-mgr` 上游**，
  四个 SubWorkflow 节点的输出要以 `pm-h-<档>.result` 的等价键回到父池。
  ⚠ 这一条是本批最容易做错的地方：子执行写回父池的键是父图那个 SubWorkflow 节点的
  `node_id` + `output_var` 双键（`engine/mod.rs:1791-1796`），而 `portfolio-mgr` 现在读的是
  `pm-h-ultra-short.result` ⇒ **父节点 id 必须继续叫 `pm-h-<档>`**（或同步改 `portfolio-mgr` 的 4 个映射键），
  否则 `map_inputs` 取不到就是四档全塌。选「父节点沿用 `pm-h-<档>` id」以免动 `portfolio-mgr` 的映射面。

⇒ 本批的**可执行形状**：子模板 `<档>` = { `t-scoring-<周期>`, `cls-risk-level-<档>`, `pm-h-<档>` }
  三个节点 + 它们之间的边；主图 = 删这三个节点，加一个 id 仍为 `pm-h-<档>` 的 SubWorkflow 节点
  （`sub_workflow_id` = 新档模板），其 `input_mapping` 覆盖子模板全部外部读取。

### (1) 现状清单（改图前的实测底数）

| 档 | 本档节点（要搬） | 现在这些节点的入边 |
|---|---|---|
| ultra_short | `t-scoring-hour`、`cls-risk-level-ultra-short`、`pm-h-ultra-short` | `e-t-scoring-t-scoring-hour`、`e-t-risk-cls-risk-ultra-short`、`e-{src}-pm-h-ultra-short` |
| short | `t-scoring-week`、`cls-risk-level-short`、`pm-h-short` | 同上形（`-week` / `-short`）+ `e-{hm_short}-pm-h-short` |
| mid | `t-scoring-month`、`cls-risk-level-mid`、`pm-h-mid` | 同上 + `e-{hm_mid}-pm-h-mid` |
| long | `t-scoring-quarter`、`cls-risk-level-long`、`pm-h-long` | 同上 |

- 错峰链 `t-scoring → hour/week`、`week → month`、`month → quarter`（`:4392-4399`，A3 已裁定「承认错峰」并登记在
  `check-tier-purity.mjs` 的 `DECLARED_SCORING_STAGGER`）⇒ **搬进四个独立子模板后这条链在物理上断开**：
  quarter 不再能等 month，四路各自从零拉月线 ⇒ 这是 §八十一(2) 那条「季线由月线聚合而来、
  改并发就是把同一段历史再拉一遍」的**代价重新出现**，必须在动手前决定是否接受（见 (5) 决策点 3）。
- 每档 `pm-h-*` 的 `input_mapping` 现有 10–20 项；子模板化后父侧要**显式列全**（§九十 的门会在 CI 拦，
  这是它建在这里的第一次真实用途）。

### (2) 子模板的形状（照认知编排器的既有形）

- 认知编排器的 L1/L2/L3 子模板**没有 trigger 节点**（首节点直接是 `data_transformer_node`）⇒ 档子模板同形：
  三节点 + 边，不塞 trigger。
- 播种：主图与档模板分开的版本门。现成先例＝`REFLECTION_TEMPLATE_VERSION`（`stock_analysis_setup/mod.rs:1972-2065`，
  独立常量 + `existing.version >=  ⇒  skip`）⇒ 档模板新增常量
  `HORIZON_TIER_TEMPLATE_VERSION`，取值与 `TEMPLATE_VERSION` **同步升到 135**（四张模板内容随主图一起变）。
- 命名：`stock-horizon-<档>`（与节点 id 前缀一致，`check-tier-purity` 的归属判据可直接复用）。

### (2.5) 一处被种子文件自己否证的形状（先记，别照着 (3) 旧版动手）

我原本打算「先改形状再改接线」：把每档三节点抽成 `fn horizon_tier_nodes(tier)`，主图与档模板共用它，
这样中间态可验证。**该形状被本文件自己的注释否证**：

- `:4604-4607`：`audit-inject-coverage.mjs` 是按 `input_mapping: [` **块内字面量**去配对脚本的
  `present()` 面的 ⇒ 「七个全局字段在每个节点里逐字重写而不是抽公共数组」，
  且 `:5249` 早已为同一理由**拒绝过循环**。
- ⇒ 抽共享 builder ＝ 把那道门的读取面拆掉（判据层「去重若让检法失去字面量读取面＝拆门」同族）。
- 但**复制两份逐字定义**也不是答案：那会在一段时间里让主图与档模板各有一份同 id 节点，
  两链演进时分叉（本文件里 `portfolio-mgr-h-*.rhai` 的教训就是靠 `check-input-mapping` 钉字面量，
  不是靠两份定义）。
- ⇒ 正解：**逐字定义只写一份（在档模板 builder 里），同时把 `audit-inject-coverage.mjs` 的扫描面
  从"主图节点块"扩到"档模板节点块"**。门跟着搬，而不是把定义留两份或把字面量收进循环。

### (3) 执行顺序（2026-10-06 按 (2.5) 改过一遍，不可调换）

0. 先在审计模块加一个纯函数 `external_reads(children) -> BTreeSet<String>`（`audit_fanouts` 内部
   那份"needs"计算的唯一出口，别再抄第二份公式）。
1. 写四张档模板的 builder（`horizon_tier_template_nodes(tier)`，**逐字**定义那 12 个节点），
   **先不接播种**：只加一条诊断测试，让它对四张模板各调一次 `external_reads` 并打印 ⇒
   得到"父侧每档必须显式传哪些键"的机器答案（不靠人抄）。
2. 同一个批内改主图：删 12 个节点与其边、加 4 个 SubWorkflow 节点（id 仍为 `pm-h-<档>`，
   见 (0) 的双键写回理由），按步骤 1 的答案填 `input_mapping`。
2.5 接播种：4 张档模板行 + `HORIZON_TIER_TEMPLATE_VERSION` 与 `TEMPLATE_VERSION = 135` 同步。
3. 门跟改：`audit-inject-coverage.mjs` 扩扫描面（(2.5)）、`check-tier-purity.mjs` 换形（(6)）、
   `seed_consistency_tests` 的节点计数/入度/H1 面、扇出审计登记四张模板（必须绿：无缺口/双向一致）。
4. 全套门禁（CI 口径三条 + node 门 + vitest）。

⚠ 步骤 1 与 2/3 之间**不存在可发布的中间态**（档模板有定义而主图仍静态展开 ⇒ 同 id 节点两份定义）。
所以本批的"最小可验证单元"是 0–3 全做完，不要中途去跑全量门禁自欺。


### (4) 会被本批打到的既有面（逐条列，防"门没红但语义变了"）

- `seed_consistency_tests.rs`：注入面==映射面、节点计数、`FAST_REQUIRED_NODE_IDS`、四档块的边断言。
- 快速链派生（`derive` + `repair_orphan_nodes`）：SubWorkflow 节点在快速链里的可达性/入度；
  快速链**不建**分析师节点，但档子模板的输入键含分析师逐档实例 ⇒ 快速链要么不扇出（保留静态展开）
  要么扇出映射里那些键恒缺席 —— 这是**决策点 1**，见 (5)。
- `check-input-mapping.mjs`：它的 MAKERS 不认 SubWorkflow 映射（正是 §九十 记的那条自陈）⇒
  本批不扩它，判据由 Rust 审计承担，避免两套口径。
- `check-horizon-weight-parity` / `check-horizon-decision-keys`：`pm-h-*.rhai` 的**脚本内容不变**，
  变的是它挂在哪个模板里 ⇒ 若门按"文件在不在种子 nodes/edges 里"判定，需要复核扫描面（决策点 2）。
- 面板与出参：主图节点数变化会改 DebugPanel 的节点计数展示，但四档结论的读端是 `pm-h-<档>.result`
  等价键 ⇒ (0) 已锁定父节点 id 不改名，界面无面。

### (5) 三个决策点（1 与 3 已自行拍定并给依据；2 留给动手时按读数定）

1. **快速链要不要跟着扇出** ⇒ **自行拍定：不跟**（按两条既有裁定合推）：
   ① 派生器 ③ 段自己写着「快速链不建那 10 个 Agent 节点 ⇒ 分析师报告质量 / 工具可信度两项恒 0，
   故摘掉 `dqi_grade` 映射」⇒ 档子模板的输入键（逐档分析师实例等）在快速链里**恒缺席**；
   ② 「结构性缺口不得在 UI 造成歧义」⇒ 扇出去只得到 4 个"输入残缺的子执行"，
   等于把「本链没有这些证据」伪装成「这一档算不出来」。
   代价 / 退役面：两链档位结构**不再同形**（完整链 4 个扇出、快速链仍是 4 个代码节点）⇒
   `FAST_REQUIRED_NODE_IDS`、H1 逐字比对、§九十 审计的 `graphs` 面都要按"哪条链有扇出"分开登记。
   回退成本：日后要让快速链也扇出，得先给它补一份"缺席即显式声明"的子模板输入表（不是改开关）。
2. 若四档移进子模板后 `check-horizon-weight-parity` 等"按种子文本判"的门失去读取面 ⇒ 是**扩门的扫描面**
   （读四张档模板）还是**把判据搬到子模板 builder 上**？倾向前者（扩清单优先于造新门）。
3. 错峰链断开（(1) 末段）是否接受：接受＝四档真并行但季线重复拉月线；不接受＝档子模板里保留
   "只读父侧已算好的月线结果"，即**quarter 不进子模板**（四档不对称）。倾向：接受断开，
   因为它正是 #7① 要的效果（用户选了"真改图"而不是"承认静态展开"），代价是多一次 HTTP 拉取，
   但 `t-scoring` 共享节点仍在父图，quarter 需要的月线序列可由父侧传入 ⇒ **不断开也能并行**：
   把 `t-scoring-month.result` 作为扇出输入传给 quarter 的子模板即可（代价 = 父图等 month 完成才扇出，
   又回到错峰）。
   ⇒ **自行拍定：先按"四张子模板各自算自己的季/月线"实现（真并行）**，把"父侧传入月线"留作
   **读数之后的优化分支** —— 它改的是运行时长而不是结论，而运行时长读数（每档 HTTP 拉取次数与耗时）
   被 #48 挡着，现在猜没有证据。动手时在 (3) 之后补一条计数断言：四张子模板各自 `t-scoring-*`
   的输入键里**不得**出现兄弟档的评分节点（＝R2 的跨模板版）。

### (6) `check-tier-purity.mjs` 换形（前置 3，与本批同批落）

- R1（四档 `tier_score` 来源互不相同）：改图后 `tier_score` 来自**父扇出映射**而不是主图里的
  `t-scoring-<档>` ⇒ 判据要换成「四个扇出节点各自的映射里，`tier_score` 的源路径互不相同」；
- R2（逐档块禁点名他档）：扫描面从主图扩到**四张档模板**（`stock-horizon-<档>` 的 builder +
  `portfolio-mgr-h-<档>.rhai` 不变），并新增一条：子模板 `sub_workflow_id` 必须等于本档模板；
- R3（错峰边逐条登记）：`DECLARED_SCORING_STAGGER` 在主图里归零（四路互不等）⇒ 登记表要么改成
  「四张子模板各自不依赖兄弟模板」，要么显式退役并说明原因（不能留着让它绿着空转）。
- 门必须**同批**改：先改图后改门 ⇒ 中间 CI 绿但判据已失效；先改门后改图 ⇒ 门要同时认两形。
  采取"门先扩成双形可认，本批末尾收紧到只认新形"的两步写法（同 §八十 的 `pending` 桶纪律）。

### (7) 验收边界（现在就说清，别到收尾才发现）

CI 面可自证：编译 / fmt / clippy / seed_consistency / 扇出审计 / tier-purity / 全套 node 门 / vitest。
**不能自证的**：v135 重播种 + 实机跑一次四档扇出（四档在面板里各自一条子执行进度 —— 那正是 B-2a 铺好的
呈现面）—— 被 **#48（PG 冷页读全挂）** 挡 ⇒ 与 #13/#30 同一批核对。⚠ 本批**不 commit**（按指令）。

## 九十二、B-2b 步骤 1 落地：四张档子模板的逐字定义 + needs 机器读数（**不发版**，2026-10-06）

### (1) 落地面

| 位置 | 内容 |
|---|---|
| `commands/stock_analysis_setup/horizon_tier_template.rs:34` | `horizon_tier_template_nodes(Period) -> (Vec<WorkflowNode>, Vec<WorkflowEdge>)`，按 `Period` **四臂穷举**（编译器保证加第五档时这里必须跟着长） |
| 同文件 `:101 / :225 / :351 / :486` | 四档各自的 `t-scoring-* → cls-risk-level-* → pm-h-* → end`，**逐字**写（`input_mapping: [` 块内全是字面量，不 `format!`） |
| 同文件 `:71 / :52` | 两个共用形状：评分工具节点按 `tool_node` 闭包的**同一字段形状**构造（`retry.max_retries=2`／`timeout=None`／`description="获取数据: compute_scoring"`），终值节点 `end` 的 `output_var` 只能写**平键** `h_<档>`（`end_executor.rs:45` 是 `variables.get(var)`，不支持点分路径） |
| 同文件 `:633 / :657 / :698` | `sibling_id_hits`（判据本体）+ 正断言 + **负控** `cross_tier_scoring_reference_is_caught` |
| `stock_analysis_setup/mod.rs:23-24` | `#[cfg(test)] mod horizon_tier_template;` —— 步骤 1 刻意**不接播种**；先给生产可见性会撞 `dead_code`（`-D warnings`），步骤 2 去掉这个 gate |
| `commands/stock_analysis_setup/seed_stock_analysis.rs:9599` | `fn direct_edge` → `pub(crate) fn`（复用既有边构造，不在新模块里抄第二份 `WorkflowEdge` 形状） |

### (2) needs 读数（= 父侧每档必须显式传的键，机器算的，不许人抄）

```
ultra_short ⇒  8：horizon_branch_json、horizon_prior_json、hourly、stock_code、stop_vol_mult、t-limitup-pool、t-risk、t-scoring
short       ⇒ 11：a-hot-money--short、horizon_branch_json、horizon_prior_json、stock_code、stop_vol_mult、t-limitup-pool、t-lockup-data、t-risk、t-scoring、take_profit_vol_mult、weekly
mid         ⇒ 13：a-hot-money--mid、horizon_branch_json、horizon_prior_json、monthly、stock_code、stop_vol_mult、t-consensus-data、t-lockup-data、t-risk、t-scoring、t-valuation、t-valuation-band、take_profit_vol_mult
long        ⇒ 11：horizon_branch_json、horizon_prior_json、quarterly、stock_code、stop_vol_mult、t-consensus-data、t-macro-data、t-risk、t-scoring、t-valuation、t-valuation-band
```

### (3) 首读是**假答案**：`root_of` 把带连字符的节点 id 判成「不是标识符」

第一次打印出来的 needs 是 `result`、`content`、`horizon_branch_json`、`hourly`……`t-risk` / `t-scoring-hour` /
`a-hot-money--short` 一个都不在。根因在门自己身上：`root_of` 的分词表不含 `-`，
`t-risk.result.content.…` 整段被当成**一个** token，`ident()` 判定失败 ⇒ 函数**继续往后找**下一个合法
token ⇒ 取到 `result`。后果是双向的：needs 既**漏**真上游（改图时会漏传键 ⇒ 运行期红），又**多**假键
（父侧根本没法传）。认知编排器那 3 个扇出之所以从没报出来，是因为它的变量名全是 snake（`l1_result.hit`）——
**这道门第一次被拿去打带连字符 id 的图，才暴露扫描面自身的错**。

修法：拆成两个口径，各自限定适用范围，并互写反向锁（`workflow_fanout_audit.rs:38/45`，测试 `:273`）：

- `path_root()`：声明式变量路径（`input_mapping` 的值、`context_sources`、`Condition::var_path`、
  `End`/`DataTransformer` 的键）⇒ 取第一个 `.` 之前的整段，**连字符是名字的一部分**。
- `root_of()`：只留给表达式（`ValidationAssertion::expression`）⇒ 表达式里 `-` 是减号，不能沿用上者。
- 反向锁一条：`root_of("t-risk.result.content") == "result"` **故意断言成错答案** —— 它钉的是
  「两条函数不得互相顶替」，而不是「这个结果是想要的」。
- 扇出 `provided` 侧同步改走 `path_root`（映射**键**是变量名，也可能带连字符）。

连带修掉一条**恒真断言**：判据二首版拿 `Period::ALL` 索引去对一张手抄的
`[(&str, &[&str]); 4]` 禁用表，而当时的 needs 里根本没有节点 id ⇒ 断言永真却绿着。改成
(a) 兄弟档节点 id 由 `horizon_tier_template_nodes(其他档)` **现算**（不做第二份权威），
(b) 先做差集剔掉四张模板共用的 `end`，(c) 补负控：把超短模板的 `tier_score` 改成
`t-scoring-week.result.content.totalScore` ⇒ 必须报出 `t-scoring-week`（`:698`，实测红⇒绿成立）。

### (4) 真读数里量出一个既有缺陷：四档评分的 `period` 从来没生效（**要拍板，见 (6)**）

needs 每一档都多出一个不可能的键：`hourly` / `weekly` / `monthly` / `quarterly`。顺着它查下去：

1. `tool_node` 闭包（`seed_stock_analysis.rs:1965-1967`）把 extra 元组按
   **「工具参数名 ← 变量名」**写进 `input_mapping`；`tool_executor.rs:75-80` 对每个 value 调
   `resolve_var_path`，取不到 ⇒ `unwrap_or(Value::Null)`（**没有**「原样当字面量」的兜底，
   `executors/mod.rs:113-171` 两分支都只查 `variables`）。
2. 全仓不存在名为 `hourly`/`weekly`/`monthly`/`quarterly` 的工作流变量
   （`seed_variables.rs` 里只有 `kline_period`/`boll_period` 这类**别的**参数；hooks 注入侧也没有）。
3. `compute_scoring` 读 `arguments["period"].as_str().unwrap_or("daily")`（`astock-data/src/mcp_tools.rs:1279`）。

⇒ 现网 `t-scoring-hour/week/month/quarter` 四个节点取的都是**日线**，`tier_score`/`macd_dif`/`macd_dea`/
`rsi_14` 四档恒等。而 `seed_stock_analysis.rs:4395-4398` 的注释写着「长档不再复用月线评分……旧做法的后果是
mid/long 两档的评分输入恒等 ⇒ 两档方向永远一样」—— 该修复**从未生效**，属本仓已登记的
「配置项空接线 / 声明与真实契约不符」族（同族史料见 `:4268-4279` 那条 V79：错了也没有任何报错）。
普查了全部 `tool_node` 调用点：带 extra 的只有 4 处（`algo_tools` 里 `VALUATION_FLAT_ARGS` 的三条指向真面板变量，
**有电**）⇒ 缺陷面就是这四条，且正好是 B-2b 要搬的那四个节点。

⚠ 证据等级要说清：以上是从代码路径**确定**推出的（三步都有 `file:line`），但「现网四档 tier_score 逐位相同」
这条直接读数要查库 ⇒ 仍被 #48 挡。步骤 3 我打算补一道门（ToolNode 的 `input_mapping` **值**必须在
「种子变量 ∪ 本图节点 id/output_var」里有源），这样这类死参数以后 CI 直接红，不依赖读数。

### (5) 「接通」会立刻撞上什么（不是免费的）

`scale.rs` 的各尺度 `min_bars = 60`，且是**聚合后**计数：日/周/月/季各自的含义是
60 交易日 / 60 周（≈1.2 年）/ 60 月（**5 年**）/ 60 季（**15 年**）。今天因为恒走 daily，
`fetch_limit=120` 日线 + `min_bars=60` ⇒ 绝大多数票够；一旦真按尺度取数，
2021 年后上市的票**中档**、2011 年后上市的票**长档**会走
`mcp_tools.rs:1298-1305` 的「样本不足 ⇒ 拒绝出分」显式失败，再经 B-1 的
`sub_workflow_output_or_fail` 上抛成**节点失败**（四档分支 `continue_on_fail=true`，主链不会整体塌，
但该档结论缺席）。也就是说：接通 = 同时改「取数口径」和「多少票会缺一档结论」，不是把一根线接上而已。

### (6) 停下问用户的决策（B 组内，且会改决策数值）

- **甲：本片就接通**（子模板头部加常量节点产出 `scoring_period`，评分节点改指它）⇒ 四档真按各自尺度取数，
  同时必须先定 `min_bars`/`fetch_limit` 口径（否则 (5) 的失败面直接在现网出现）。
- **乙：搬图时摘掉这四条死映射**（现网结论**逐位不变**，但不再谎报「本节点按周线取数」），
  把尺度层接通另开一片，前置 = 先拿到「各尺度样本够不够」的可读数据（被 #48 挡着的那类）。
- **丙：接通但同步下调 min_bars / 改由父侧传日线本地聚合**（动 `scale.rs` 的出分判据，牵面最大）。

三条都能让 B-2b 继续（needs 里那条假键要么被传真值、要么整条消失、要么换成真变量）。
按「遇到 B 组停下问我」的指令，**已停在这里等拍板**；步骤 2/3 不动。

### (7) 本批门禁读数

| 命令 | 读数 |
|---|---|
| `__TAURI_WORKSPACE__=true cargo test -p axagent --lib` | `test result: ok. 553 passed; 0 failed`（= 改前 550 + 本批 3 条） ⇒ `LIB9_EXIT=0` |
| `cargo test -p axagent --lib horizon_tier_template -- --nocapture` | 2 passed / 0 failed，needs 见 (2) ⇒ 负控 `cross_tier_scoring_reference_is_caught ... ok` |
| `rustfmt --edition 2024 --check`（两个改动文件） | 0 diff |
| `cargo clippy -p axagent --all-targets -- -D warnings` | `Finished dev profile … in 7m 43s` ⇒ `CLIPPY8_EXIT=0`，`^error` 计数 **0** |

⚠ 本批**未接播种**（模板行、`HORIZON_TIER_TEMPLATE_VERSION`、`TEMPLATE_VERSION = 135` 都在步骤 2/2.5），
⇒ **不发版**、现网逐位不变。`audit-inject-coverage.mjs` / `check-tier-purity.mjs` 的扫描面**还没**扩到本文件 ——
那是步骤 3，且必须在改图的同一批里做（§九十一(6) 的两步写法）。本批**不 commit**（按指令）。

## 九十三、丙 的落地前置：尺度层出分口径（2026-10-06 用户拍「丙：接通 + 同步改出分口径」）

§九十二(6) 三选一，用户拍 **丙**。本节把「丙」拆成**能证伪的规格**，并先记三个刚取到的读数 ——
它们决定了丙 不能只是「把线接上」。

### (1) 三个读数（都是现场取的，不是推断）

1. **指标窗口是「bar 根数」而不是「日历跨度」**：`indicators.rs:61-77` 的
   `IndicatorConfig::default()` 写 `ma_periods=[5,10,20,60]`、`macd=12/26/9`、`rsi=[6,12,14,24]`、
   `boll_period=20`；`indicators.rs:150-157` 按 **period 数值**把结果塞进固定字段
   （`60 => ma60 = val`）⇒ 同一组窗口挂在月线/季线上，含义自动膨胀
   （月线 MA60 = **5 年**、季线 MA60 = **15 年**）。
2. **配置通道是半接线**：`compute_indicators_with_config` 存在，但**全仓生产端只有默认路径**
   （`:344-346` 传 `None`；唯一带 config 的调用在 `:511` 的测试里）。且 `TechnicalIndicators`
   的字段名是 `ma5/ma10/ma20/ma60`、`rsi6/12/14/24` —— 传自定义 `ma_periods=[3,6]`
   不会改变输出字段**含义**（没有 3/6 这两个字段）⇒ 「能配」不等于「配了有用」，
   直接走这条路会得到新的空接线族。
3. **本地聚合只有季线一条**：`scale.rs:150 aggregate_monthly_to_quarterly`，
   周/月线仍是向 vendor 各取各的（`fetch_limit` 120 根），`min_bars=60` 对四个尺度
   分别等于 60 交易日 / 60 周(≈1.2 年) / 60 月(**5 年**) / 60 季(**15 年**)。

### (2) 为什么「下调 min_bars」单独不成

`mcp_tools.rs:1296-1305` 的地板是刻意的：拿 3 根 bar 去算 MA60 会产出一个「看起来正常」的假分数。
把 `min_bars` 从 60 调到比如 12 就能让多数票出分 —— 但那时 `ma60` 字段算的是
「12 根月线的收盘均值」，而消费端（`ScoringEngine`、分析师 prompt、面板文案）按
**60 周期趋势**读它 ⇒ 这是把「拿不到」换成「拿到一个名字撒谎的数」，属本仓最禁止的形态
（§九十二(4) 那条 V79 史料同一个族）。**所以丙 的正确落法必须同时动「窗口定义」**，
只动 `min_bars` 的那一支判为不可用。

### (3) 规格：窗口按**日历跨度**定义，bar 数由尺度换算

- 唯一权威 = `scale.rs` 已有的 `trading_days_per_bar`（`0.25 / 1 / 5 / 20 / 60`）
  \+ `harness::holding_period::Period::default_holding_days`（`2 / 5 / 28 / 90`）。两者都是既有常量，
  **不新增可调参数**（不触「新增可调参数按五点对账」）。
- 换算式：某尺度上的窗口根数 `n_s = round(c · h / d_s)`，`c` 是该窗口相对持有期的倍数。
  现有日线口径给出的锚点：`h=daily` 链上 `ma60` 对应 60 交易日 ⇒ 对**长档**（h=90 交易日）
  在**季线**（d_s=60）上，若要保持同一日历跨度（60 个交易日）则 `n = 60/60 = 1` 根 bar —— 无意义；
  要保持「趋势窗口 = 3 倍持有期」则 `n = 3·90/60 = 4.5 → 5` 根季线。**这就是必须显式选的东西**：
  季线上的 MA60 与长档的 90 日持有期不可能同时成立，二者取一个就要在字段名与 prompt 里改口。
- 因此本片的**第一个交付物不是接线，而是字段契约**：给每档尺度一套
  `{短/中/长趋势窗口}` 的**命名与序列化契约**（新字段或带 `window_days` 标注的对象），
  同步 `verdict_spec`/分析师 prompt/面板 4 表/11 语言 —— 牵动面与本仓「新增数据源五步」同量级，
  **不是 B-2b 的子项**。
- 结论可证伪的验收：改完后 (a) 四档 `tier_score` 在同一票上**不再恒等**（可由 `scale.rs` 的
  聚合单测 + 一条「同票四尺度取数根数互不相同」的门自证）；(b) 历史不足的新票**点名缺席**而不是出假分。

### (4) 与 B-2b 的先后（不能倒）

`needs` 里那条 `hourly/weekly/monthly/quarterly` 假键，在丙 下会变成「本档尺度常量」，
但**传什么值、够不够样本**由 (3) 的窗口契约决定 ⇒

1. 先做 (3) 的尺度契约片（含 `scale.rs` 换算、字段契约、四尺度取数、11 语言）——**独立一片，会改决策数值**；
2. 再落 §九十二(3) 的步骤 2/2.5（改图 + 播种 + v135），此时 `needs` 才是最终清单；
3. 步骤 3 的门跟改与 §九十二(4) 末那条新门（`ToolNode` 的 `input_mapping` **值**必须在
   「种子变量 ∪ 本图节点 id/output_var」里有源）同批落 —— 它正是让这类死参数以后 CI 直接红的检法。

⇒ **B-2b 步骤 2/3 现在停在这里**（不等实机，等的是口径设计）。本轮先做**与 (3) 无关**的一片：
§九十一(6) 要求的「门先扩成双形可认」——`check-tier-purity.mjs` 与 `audit-inject-coverage.mjs`
扩到能读 `horizon_tier_template.rs`，这样改图那天不会出现「图改了、门还只看主图 ⇒ CI 绿而判据已失效」。

## 九十四、B-2b 前置 3 第一步落地：`check-tier-purity.mjs` 扩成双形可认 + 过渡期逐字一致门（**不发版**，2026-10-06）

拍板结果（§九十二(6)）：**丙** ⇒ 尺度层另立一片（§九十三，任务 #39），#36 改图被它挡着。

### (1) 落地面（`scripts/check-tier-purity.mjs`）

| 位置 | 内容 |
|---|---|
| `:39-41` | 新增 `BUILDER_REL = "src-tauri/src/commands/stock_analysis_setup/horizon_tier_template.rs"` —— 扫描面从「只看主图」变成主图 + 档模板 |
| `:48-79` | `matchBrace` **通用化**：原来只配 `{}`，而 `mappingPairs` 要配 `input_mapping: [` 的 `[]` ⇒ 首版直接返回错位置（自己写的辅助函数缺一种括号） |
| `:86-113` | `branchBlocksIn()`：按 `CodeNode {` 的**花括号平衡**切块。替掉旧右边界「到下一个 `nodes.push(` 为止」—— 那是主图专用启发式，档模板里分支节点后面跟的是 `tier_end_node(…)`（同一个 `vec![…]`），旧法会一路滑到文件末尾，把**后面三档的字面量一起吃进同一个块** ⇒ R2 假红 |
| `:116-137` | `mappingPairs()`：按本门自己的口径取键值对（**先剥行注释**再配对，与 §八十 的登记面纪律同族） |
| `:144-160` | `branchBlocks(seedText, builderText)`：两侧合并、**主图优先**（过渡期不重复计档） |
| `:163-186` | 新判据 R4 `transitionDrift()`：同 id 块两侧都存在 ⇒ 映射面必须逐字一致；任一侧取不到对 ⇒ 直接报「一致性判据失去对象」（不许退化成少比几项） |
| `:381-389` | `checkAll(seedText, harnessText, scriptsByText, builderText = "")` —— builder 放**第四参且缺省空**，既有 10 条自证对照不用改口径（避免为兼容写出第二套判据） |
| `:531-585` | 自证夹具改成**生产同形**（`WorkflowNode::Code(CodeNode { … })`）；`fourBlocks(tail)` 造两种尾巴 |
| `:610-644` | 新增 4 条对照：两侧一致必须绿 / 漂移必须红 / 主图那份撤掉后扫描面不塌 / 切块不吃进兄弟档 |

### (2) 两个副产品（都不是我预期的）

1. **两侧现网逐字相符，由门自证**：`pm-h-ultra-short` 11 条、`pm-h-short` 12 条、`pm-h-mid` 17 条、
   `pm-h-long` 17 条映射，两侧集合完全相同、`transitionDrift` 为空。⇒ §九十二 说的「逐字搬」这件事
   不再依赖我盯 diff 的次数。
2. **我本轮新写的 `文件:行` 里有一处形状违规**：`workflow_fanout_audit.rs:5` 写的是
   `crates/rt-workflow/.../subworkflow_executor.rs:390` —— `.../` 属 AGENTS.md「段对齐」明令禁止的
   **段内截断**；另有 4 条裸 basename 落进 facts 门的 `NONLOC（无法定位因而未检查）` 桶，
   还有 1 条把 seed 文件的行区间写成「本文件」。全部改成唯一后缀形态
   （`work_engine/executors/end_executor.rs:45`、`work_engine/engine/output_builder.rs:41-45`、
   `commands/stock_analysis_setup/seed_stock_analysis.rs:5859-5862` 等）并复跑。
   ⇒ 判据补一条：**写 `file:行` 时，落在 NONLOC 桶就等于没写过**；裸 basename 要在门读数里确认它 LOCATED。

### (3) 门禁读数

| 命令 | 读数 |
|---|---|
| `node scripts/check-tier-purity.mjs --selftest` | `✅ 自证通过（14 条对照…）` ⇒ 0 |
| `node scripts/check-tier-purity.mjs --dump` | 四档块全部「读自 主图」，评分源 = hour/week/month/quarter，错峰现场 = 声明 ⇒ `EXIT=0` |
| `audit-inject-coverage` / `check-analyst-id-shape` / `check-horizon-weight-parity` / `check-input-mapping` / `check-horizon-decision-keys` / `check-generation-scope-registry` / `check-rhai-numeric-typing` | 逐条 `EXIT=0`（含既有豁免与 pending 提示，非新增红） |
| `check-single-source-facts` | `EXIT=3` —— 软判据「疑似腐烂 6 处」，逐条核对**都不落在本轮改动的文件**（`src/init/services.rs:1972/975`、`decision.rs:289`、`mcp_tools.rs:1254`、`strategy-scorer.rhai:212`、`chokepoint-identifier.md`），且改完引用后本轮引用已从 NONLOC 移出 |
| `__TAURI_WORKSPACE__=true cargo test -p axagent --lib` | `553 passed; 0 failed`（§九十二(7)，本轮引用未动代码） |
| `rustfmt --edition 2024 --check`（两个改动 .rs） | 0 diff |
| `cargo clippy -p axagent --all-targets -- -D warnings` | `Finished dev profile … in 7m 57s` ⇒ `CLIPPY9_EXIT=0`（纯注释/引用形态改动也复跑：`doc_lazy_continuation` 只管注释） |

### (4) 界面后果

零 —— 仍未接播种、未 bump 版本。但这一步的必要性是界面层的：若不先扩面就改图，
四档分支的 R1/R2 会**继续只看主图**，而主图那四个块已被删 ⇒ 门在零个块上"通过"，
CI 全绿、界面上四档照常各自显示一行，串档与漏传键都不再有人检。

### (5) 顺序（现在唯一确定的一条）

`#39 丙 前置片（尺度口径 + 指标字段契约）` → `#36 步骤 2 改图 + 播种 v135` →
`#37 步骤 3 门跟改（audit-inject-coverage 扩面、seed_consistency 节点计数/入度、扇出登记四张模板、
§九十二(4) 末那条「ToolNode 的 input_mapping 值必须有源」新门）` → `#33 第二步（R3 归零后退役或改判、
R1 换成按扇出映射判、收紧到只认新形）`。

## 九十五、#39 片 1 落地：档 ↔ 尺度 ↔ 评分节点 的权威进 harness + R5 门（**不发版**，2026-10-07）

### (1) 动手前的一次普查：这张映射在代码里**不存在**

`grep "weekly\|monthly\|quarterly\|scale" crates/harness/src/holding_period.rs` ⇒ **0 命中**；
全仓只有两处 `Scale::Quarterly` 特判（`analysis-engine/recommender/scale.rs:48`、
`astock-data/src/mcp_tools.rs:1293`）。也就是说「超短=小时线 / 短=周线 / 中=月线 / 长=季线」
（阶段 2 的拍板）**只活在种子注释**和「哪档读哪个 `t-scoring-*`」的字面量里。

⇒ 这把 §九十二 那条缺陷的**检不出来**解释清楚了：`period` 参数没到账之后，四档读的是同一个尺度，
但各处字面量互相印证，没有第三方可对 —— 权威缺失时，一致性门只能证明「抄得齐」，不能证明「抄得对」。

### (2) 落点

| 位置 | 内容 |
|---|---|
| `crates/harness/src/holding_period.rs:236-253` | `Period::scale_key()`（`hourly/weekly/monthly/quarterly`）与 `Period::scoring_node_id()`（`t-scoring-hour/week/month/quarter`）—— 同一个决定的两种拼写，**必须同臂改** |
| 同文件 `:245-250` 的分层说明 | 尺度的日历跨度 `trading_days_per_bar` 与窗口换算**留在 `astock-data::scale`**：那里才有 `d_s`；把 `0.25/1/5/20/60` 抄进 harness = 第二份表（禁区 12） |
| 同文件 `:919-950` | 测试 `scale_key_and_scoring_node_are_one_table_per_tier`：四臂逐档期望写死（改一臂就红）+ 四档尺度互不相同 + 四档节点互不相同 + 前缀约定 |
| `scripts/check-tier-purity.mjs:120-143` | 新解析器 `armsOfPeriodFn(harnessText, fn)`：按 `Period::X => "值"` 现推四臂，档名由变体名 camel→snake 推得 |
| 同文件 R5 段 | **新判据 R5**：每档 `tier_score` 来源 ≡ 权威 `scoring_node_id`；权威取不到四臂 ⇒ 直接判红（「判据不存在≠绿」）；两套推导（camel→snake 与 `as_str` 右值）互验，不一致即点名 |
| 同文件自证 | 夹具 harness stub 补 `scoring_node_id` 四臂；新增 3 条对照（权威改了种子没跟 / 取不到四臂 / 变体名漂了）⇒ 自证 14→17 条 |
| `seed_stock_analysis.rs:5841-5843` | 引用**按内容重指**：原写 `harness::holding_period:344-348`，实际那段（`Period::UltraShort => (&["momentumSignal","breadthState"], …)`）现已在 `:397-398`。⚠ 这条引用**在我插入之前就已经漂了约 20 行**（上一轮别人加的显示名 match 等），本轮的 `+32 行`只是又推了一次 ⇒ 印证既有纪律：前段插行必须当场跑 facts 门并按内容重指，而不能指望行号自己稳 |

### (3) R5 与既有 R1 的分工（不要混读）

R1 判「四路来源**互不相同**」，它**看不出对不对档** —— 把短档与长档的来源互换，R1 仍绿。
R5 判「这一档的来源 == 权威指定的那一个」。两条都在，串档与互换才都拦得住。

### (4) R5 首跑读数：绿（要说清它意味着什么）

现网种子四档来源 = hour / week / month / quarter，与新权威逐档一致 ⇒ **R5 没有抓到既有缺陷**。
⇒ §九十二 那条缺陷**不在"接错档"**（接的是对的节点），而在"节点里那个区分尺度的参数没到账"。
别把 R5 读成"已经把 period 那件事修好了" —— 它只是让「以后接错档」变得可见。

### (5) 片 2 的推导（窗口口径，尚未落）

倍数**不从零发明**，锚在日线既有口径自身的倍数关系（`indicators.rs:61-77`）：
`MA20 ≈ 1 个自然月`、`MA60 = 3 × MA20`、`MACD slow 26 ≈ 1.87 × RSI14`、`signal 9 ≈ 0.35 × slow`。
于是同一档尺度上按**日历跨度**取（`d_s = trading_days_per_bar`，`h = default_holding_days`）：

- 短带 `= max(2, ceil(h / d_s))`；长带 `= ceil(3h / d_s)`；RSI 周期 `= 短带`；
  MACD `= (短带, 长带, max(2, round(0.35 × 长带)))`；
- `min_bars = 长带 + 1`（最后一个 bar 要有前一根才谈得上"穿过"）；
  `fetch_limit = 4 × 长带`（留余量；季线仍由月线聚合 ⇒ 聚合前取 `4 × 长带 × (60/20)` 根月线）；

| 档 | h | d_s | 短带 | 长带 | RSI | MACD(f,s,sg) | min_bars | fetch |
|---|---|---|---|---|---|---|---|---|
| ultra_short | 2 | 0.25（小时） | 8 | 24 | 8 | 8/24/8 | 25 | 96 根小时线 |
| short | 5 | 5（周） | 2 | 3 | 2 | 2/3/2 | 4 | 12 根周线 |
| mid | 28 | 20（月） | 2 | 5 | 2 | 2/5/2 | 6 | 20 根月线 |
| long | 90 | 60（季） | 2 | 5 | 2 | 2/5/2 | 6 | 20 根季线（=60 根月线聚合） |

⚠ 中/长两档的**根数**同为 (2,5)，但**尺度不同** ⇒ 取的是不同的 bar 序列、数值必然不同。
「四档互不相同」要按尺度看，不能按这几个整数看 —— 这正是 R5 用节点 id 而不是用窗口数当判据的原因。

⚠ 上表**只覆盖 MA / MACD / RSI 三族**。`TechnicalIndicators` 里还有一样多的其它窗口，
片 2 必须逐族给同样的锚与换算，不许留一半：
`boll_period`（日线锚 20 = MA20 同族 ⇒ 与短带同值）、`bias_ma5`（锚 5 ⇒ `max(2, ceil(5/d_s))`）、
`volume_lookback`（日线锚 5 ≈ 1 周 ⇒ 同上）、`support/resistance` 的分形窗口、
以及 `rsi_overbought/oversold` 这类**阈值**（不是窗口，尺度变了阈值不该跟着缩 —— 它衡量的是
相对强弱百分位，与 bar 长度无关；这条要在片 2 明确写"不动"，否则会被顺手一起改）。
⇒ 判据：片 2 交付时逐字段列一张「日线锚 → 该尺度取值 → 是否随尺度变」的表，
缺任何一族都算未闭环（"名字带数字的字段"就是 §九十三(1) 那条撒谎面）。

**字段契约（片 2 的另一半）**：`compute_scoring` 的 payload 随分输出
`windows { scale, holdingDays, trendShortBars, trendLongBars, rsiPeriod, macd, minBars }`；
四档分支（`pm-h-*.rhai` 与 `check-horizon-decision-keys`）改读**带 windows 的那份**，
而 `ma5/ma10/ma20/ma60` **原样保留给日线链**（那是它自己的真值，不改名）——
于是不会再出现「月线链上的 `ma60` 被下游当 60 日读」。同步面：`verdict_spec` / 分析师 prompt /
面板 4 表 / 11 语言（§九十三(3) 已记它等于「新增数据源五步」的量级）。

### (6) 本片门禁读数

`cargo test -p axagent-harness --lib` ⇒ **783 passed / 0 failed**（含新测试，
`holding_period::tests::scale_key_and_scoring_node_are_one_table_per_tier ... ok`）｜
`node scripts/check-tier-purity.mjs --selftest` ⇒ 17 条全过｜现网 `--dump` ⇒ `EXIT=0`（见 (4)）｜
`__TAURI_WORKSPACE__=true cargo test -p axagent --lib` ⇒ **553 passed / 0 failed**（harness 是依赖，
改了它必须复跑读它的门）｜`cargo clippy -p axagent-harness -p axagent --all-targets -- -D warnings`
⇒ `Finished … in 9m 08s`，`CLIPPY10_EXIT=0`｜`rustfmt --check`（holding_period.rs）0 diff｜
facts 门：`LOCATED 206→207`、`NONLOC 303→300`、`BROKEN 0`，`疑似腐烂` 仍是既有 **6** 处
（`holding_period` 在报告里 0 命中 ⇒ (2) 那条重指后引用可定位）。

### (7) 界面后果

片 1 **零** —— 没有任何生产代码消费这两个新函数（只有 harness 自己的测试与 node 门读它）。
片 2/3 落地后会改数值：四档的 `tier_score` / MACD / RSI 首次真的按各自尺度算 ⇒
面板四档技术分行、`f4`/风险按档、选档（arbiter）都会动 ⇒ 需要换代 + 实机核对（仍被 #48 挡）。

## 九十六、丙 片 2 第一步落地：尺度窗口计划（`ScaleWindowPlan`）+ 与本文件旧结论的分歧裁定（**不发版**，2026-10-07）

### (0) 先接住一条**与本方向相反**的既有结论（不静默覆盖）

`crates/astock-data/src/scale.rs` 头部（Phase B 写的）有一条：

> ⚠ **指标窗口不需要按尺度改写**：`TechnicalIndicators` 的 MA/RSI/MACD 窗口以 **bar** 为单位，
> 周线 MA20 天然 = 20 周 ≈ 100 交易日。真正的尺度缺陷在 ① 静默降级、② 评分分段阈值、③ 缺尺度。

这条在「bar 数含义」上没错，也确实被落实了一半 —— 读数：`compute_scoring` **已经在调**
`ScoreBands::scaled_for(&profile)`（`mcp_tools.rs:1308-1310`，√d 缩放，注释自陈「日线 f=1.0 ⇒ 零回归」），
并且 payload 已经输出 `"period": profile.period`。**缺的是另一半：窗口本身仍是 bar 数固定值**
（`compute_indicators(code, &klines)` 走 `IndicatorConfig::default()`，全仓无人传自定义 config）。

它的推理漏在哪：把「窗口以 bar 为单位」当成了「窗口与持有期无关」。后果可算：
`min_bars = 60` 在季线上是 **15 年**历史，而长档回答的是 90 交易日的持有问题；
60 期均线的响应时间 ≫ 持有期 ⇒ 多数票上恒不穿越 ⇒ `trendScore` 在中/长档退化成常数。
⇒ **裁定**：Phase B 那条的 ①②③ 全部保留且已生效，只把「指标窗口不需要按尺度改写」这半句
**就地标注为被本节取代**（代码注释里写明取代关系与理由，不删原文，避免下一个人重走一遍）。

### (1) 落点（`crates/astock-data/src/scale.rs`）

| 位置 | 内容 |
|---|---|
| 头部 ⚠ 段（`:13-22`） | 原文保留 + 追加「本条只被证伪了一半…取代关系记 §九十六」 |
| `:145-196` | `ScaleWindowPlan`（12 个字段 + `parent_fetch_limit`）与比值来源说明 |
| `:200-203` | `fn scaled(short, ratio, floor)` —— 各族共用的钳位（窗口下限 2，防退化成常数） |
| `:205-241` | `ScaleWindowPlan::for_period(Period)`：尺度与持有期**都不在此重述**，取 `Period::scale_key()` / `default_holding_days()`（§九十五 的权威）+ `ScaleProfile::resolve`（不认即 panic，绝不静默回退 daily） |
| `:243-266` | `daily_default()`：日线不属于任何一档 ⇒ 走常数；它的值就是既有默认的逐字复现（零回归锚，有测试对账） |
| 测试 `:437-471`、`:473-507`、`:509-527` | 六条，见 (3) |

### (2) 换算与比值（锚在日线既有口径**自身的**比例）

取日线「中带 = MA20」为 `short`，其余族：`long = 3×short`（60=3×20）、`rsi = 0.7×short`（14/20）、
`macd_fast = 0.6×short`（12/20）、`macd_slow = 1.3×short`（26/20）、`macd_signal = 0.75×fast`（9/12）、
`boll = short`（20/20）、`volume_lookback = 0.25×short`（5/20）、`min_bars = long`（60）、
`fetch = 2×long`（120）。⇒ `short = max(2, ceil(h / d_s))`（钳到 ≤60），代式在 `short=20` 时
**逐字复现日线默认** ⇒ 「档位用比值、日线用常数」不是两套真相，是同一比值在两个尺度上各代一次。

实际读数（`cargo test -p axagent-astock-data --lib horizon_window_plan_dump -- --ignored --nocapture`）：

```
档=ultra_short scale=hourly    h=2  短带=8 长带=24 rsi=6 macd=5/10/4 boll=8 vol=2 min=24 fetch=48  聚合前=None
档=short       scale=weekly    h=5  短带=2 长带=6  rsi=2 macd=2/3/2  boll=2 vol=2 min=6  fetch=12  聚合前=None
档=mid         scale=monthly   h=28 短带=2 长带=6  rsi=2 macd=2/3/2  boll=2 vol=2 min=6  fetch=12  聚合前=None
档=long        scale=quarterly h=90 短带=2 长带=6  rsi=2 macd=2/3/2  boll=2 vol=2 min=6  fetch=12  聚合前=Some(36)
档=daily(零回归锚)              短带=20 长带=60 rsi=14 macd=12/26/9 boll=20 vol=5 min=60 fetch=120
```

⚠ **如实记两点，别当成已解决**：

1. 短/中/长三档的**整数完全相同**（2/6/…），差别只在尺度（2 周 / 2 月 / 2 季 = 10 / 40 / 120 交易日）。
   这是代式的必然（三档的 `h/d_s` 都落在 1~1.5 之间被下限抬到 2）。所以测试**刻意不断言
   「四档整数互不相同」**（那条恒假），分层判据交给 R5（节点 id ≡ 本档尺度节点）与
   「中带日历跨度 ≥ 持有期」这条可算断言。若将来要让整数也分层，得引入第二条轴（例如中带 = 2h），
   那会偏离「中带 ≈ 一个持有期」的锚 —— 属另一次裁定，不在本片偷偷做。
2. `fetch` 从现值（周/月 120、季 240 月线、小时 320）**降到** 12/12/36/48。副作用是好的方向：
   一次分析的评分取数量下降一个数量级 ⇒ 已知「东财 push2* 累积/速率触发 RST」的风险面随之下降。
   但这是**取数行为变更**，片 2 第二步要用真序列验一遍「12 根 bar 够不够 `compute_indicators`
   出全部字段（含支撑/压力位的近期窗口）」，不够就调比值而不是不验。

### (3) 六条测试各自钉什么

- `daily_default_reproduces_existing_constants`：对账 `IndicatorConfig::default()` 与
  `ScaleProfile::daily()` 的 `min_bars`/`fetch_limit` ⇒ 零回归锚，任一改动即红；
- `tier_scale_keys_are_resolvable_by_this_table`：`Period::scale_key()` 的字面量必须被尺度表白名单认
  （不认即 panic）⇒ **这就是 §九十二 那条死参数的 CI 版**：写错串不再静默归 daily；
- `tier_window_plans_cover_holding_period_and_are_admissible`：中带日历跨度 ≥ 持有期、
  长带 = 3×中带、`macd_slow > macd_fast`、`min_bars ≤ fetch_limit`（否则恒失败）、四族窗口 ≥ 2；
- `quarterly_plan_carries_parent_fetch_and_others_do_not`：季线必须带「聚合前月线根数 = 3×fetch」，
  其余尺度必须为 `None`；
- `insufficient_history_fails_by_naming_scale_and_counts`：样本不足时诊断要**点名尺度与两个根数**；
- `tier_plans_are_not_copies_of_the_daily_default`（负控）：非日线档不得落回日线尺度。

### (4) 本轮两次自身失误（要记，别只记成果）

1. **把读数写在运行之前**：我在真跑之前就写了「539 passed」——实际那次运行里**根本没有新测试**
   （文件 386 行、只有类型没有测试），539 是既有套件数。⇒ 判据仍是老那条：**只报日志里有的**，
   而且新测试要按**测试名**在日志里点名确认它跑过（`grep "^test scale::"`），套件数不是证据。
2. **Edit 吃掉闭合符号**：追加 dump 测试时 `old_string` 以 `);` 开头结尾、`new_string` 少带了那段
   ⇒ `assert_ne!` 与三层右括号被静默删除，`rustfmt` 直接报 `mismatched closing delimiter`。
   同族教训已在记忆里有（「new_string 少带同一行＝静默删邻行」），这次是**结构符号**版本：
   改完除了按标识符计数回读，还要让**解析器**过一遍（rustfmt 单文件秒级即可抓到）。

### (5) 门禁读数

`cargo test -p axagent-astock-data --lib` ⇒ **545 passed / 0 failed; 8 ignored**（= 改前 539 + 本批 6），
`AE_TEST4_EXIT=0`；dump 测试单跑 `1 passed`；`cargo clippy -p axagent-astock-data --all-targets -- -D warnings`
⇒ `Finished … in 53.21s`，`AE_CLIPPY_EXIT=0`；`rustfmt --edition 2024 --check` 0 diff。
**现网零变化**：`ScaleWindowPlan` 目前**没有任何生产调用方**（下一步进 `compute_scoring` 才有）。

### (6) 本片剩下的两步（顺序固定）

- **片 2 第二步**：把 plan 接进 `compute_indicators_with_config`（构造 `IndicatorConfig`）+
  payload 输出 `windows {…}`；`ma5/ma10/ma20/ma60` 原样留给日线链，档位链新增不撒谎的结构
  （因为 `indicators.rs:150-157` 是按 period **数值**塞固定字段，自定义窗口数不会落进 `maNN` ⇒
  必须新增槽位，不能假装配置通道可用）。同批验 (2) 的「12 根够不够出全部字段」。
- **片 3（#41）**：`("period", …)` 接到真值 + `pm-h-*.rhai`/面板/prompt/11 语言改读带 windows 的那份
  ⇒ 换代 + 全套门禁 + 实机核对（#48）。

## 九十七、片 2 第二步的**形状被实测否证**：命名槽按数值填 ⇒ 必须先加中性槽（2026-10-07，不发版）

§九十六(6) 给的第二步形状是「plan 塞进 `IndicatorConfig` + payload 加 `windows`」。
动手第一步就把这条否证了，读数如下。

### (1) 实测：非命名槽周期会被**静默丢弃**

`indicators.rs:146-157` 的写法是 `for &period in &cfg.ma_periods { … match period { 5 => ma5=val, 10 =>,
20 =>, 60 =>, _ => {} } }` —— 四槽按**周期数值**认领。传档位换算出的 `[8,24]` ⇒ **两条都落进 `_ => {}`**
⇒ 四槽全部停在初值 `latest_close` ⇒ `ma_alignment`（比四槽大小）恒判「缠绕/交叉」。
RSI 同族且更隐蔽：`rsi(&closes, period).unwrap_or(50.0)` ⇒ 样本不够/周期不认时**伪装成「中性」**。

落成锁（`indicators.rs` 测试 `off_ladder_periods_are_silently_dropped_by_named_slots`）：
70 根每根 +0.1 的上升序列，`ma_periods=[8,24]`、`rsi_periods=[10]` ⇒
断言四槽 `== latest_close`、四个 RSI 槽 `== 50.0`、`ma_alignment == "缠绕/交叉"`；
**对照组**用默认周期跑同一序列 ⇒ `latest - ma20 > 0.5`、`latest - ma60 > 1.5`、`rsi14 > 50.0`
（没有对照组，主断言就只是「数据不够」的同义反复）。这条**锁的是当前危险行为**，
2b 落中性槽后要改判成「中性槽被填 + 命名槽未被拿去冒充档位窗口」，不许为了绿而删它。

### (2) 2b 的契约因此改掉

- 新增**与尺度无关的中性槽**（趋势短/长带 + MACD + 动量 + 每槽实际 bar 数 `bars`），
  `ma5/ma10/ma20/ma60` 继续只服务日线链（日线不属于任何一档，见 §九十六(1) 的取代说明）；
- 粗尺度上「四条 MA 带」本来就分辨不出来（`ceil(h/d)` 在周/月/季都落到 2~5 根）⇒
  形态判据 `ma_alignment` 在档位链上必须按**实际带数**重写，不能照抄四槽的大小比较。

### (3) 一处我自己提了又撤的形状

中途加过 `ScaleWindowPlan::named_slot_ma_periods()`（只有 `daily` 返回 `Some([5,10,20,60])`），
意图是"给命名槽一个合法出口"。它等于**给静默丢弃留一条正规通道**，且当前没有任何消费者 ⇒
撤掉；事实改由 (1) 那条锁承担。⇒ 判据：契约不清时不要用「只在一个分支返回 Some」的 API 把
问题藏进类型签名里。

### (4) 本轮两次小失误

1. 对照组容差贴着边界：每根 +0.1 ⇒ MA20 恰比收盘低 0.95，我写 `< latest - 1.0` ⇒ 差 0.05 报红。
   合成序列上的数值断言要用**差值 + 合理边界**，别写贴着边界的绝对阈值。
2. `clippy::field_reassign_with_default` 首红（`let mut cfg = Default::default(); cfg.x = …`）⇒
   改 struct-update 语法一次成型。

### (5) 读数与状态

`cargo test -p axagent-astock-data --lib` ⇒ **546 passed / 0 failed; 8 ignored**（= §九十六 那轮 545 + 本条锁），
新测试按名字确认 `…off_ladder_periods_are_silently_dropped_by_named_slots ... ok`｜
`cargo clippy -p axagent-astock-data --all-targets -- -D warnings` ⇒ **0**｜`rustfmt --check` ⇒ **0**。

`scale.rs` 本轮**未新增公共 API**（撤掉了 (3) 那个），`ScaleWindowPlan` 仍无生产调用方 ⇒ **现网零变化**。
2b（中性槽 + payload `windows`）与片 3（接线 + 换代）都还没做，#36 改图继续被挡。

## 九十八、2b 已落 + 接线前的口径分叉（4 带 ladder 与 2 带新评分路径）（2026-10-07，不发版）

### (1) 2b 落地面

| 位置 | 内容 |
|---|---|
| `indicators.rs:43-66` | 两个 additive 字段：`windows: Option<IndicatorWindows>`（回显**实际使用**的周期）与 `scale_trend: Option<ScaleTrend>`（两带事实），都 `skip_serializing_if` |
| `indicators.rs:68-99` | `IndicatorWindows`（`from_cfg` 只做回显，不重新解释） |
| `indicators.rs:101-118` | `ScaleTrend { fast_bars, slow_bars, fast, slow, diff_pct: Option, fast_slope: Option }` —— **只出数字不出标签**（分类留给有 `√d` 上下文的一侧，再造阈值就是第三套口径） |
| `indicators.rs:390-420` | `scale_trend_of()`：取 `ma_periods` 排序去重后的**最后两根带**；日线默认 ⇒ `(20,60)`（与 `ma20/ma60` 同一对 ⇒ 两代口径同构），档位 `[8,24]` ⇒ `(8,24)` |
| 缺席四条 | 少于两根可用带 / 带数重复 / 样本不够算慢带 / 空输入 ⇒ 一律 `None`（不给 0、不给「缠绕」）；空输入时 `windows` 仍回显 |

测试（读数：`cargo test -p axagent-astock-data --lib` ⇒ **549 passed / 0 failed; 8 ignored**，
`AE_CLIPPY9_EXIT=0`，`rustfmt` 0 diff）：
`windows_echoes_the_config_actually_used`（默认路径逐字等于 `IndicatorConfig::default()`）、
`scale_trend_absent_instead_of_fake_when_bands_unusable`（四条缺席）、
`new_fields_are_additive_and_camel_case`（13 个旧键一个不少 + 新键 camelCase），
以及 §九十七 那条锁**就地改判**（同一份 `[8,24]` 输入：命名槽仍被丢弃，但 `windows`/`scaleTrend` 有真值）。

### (2) 为什么不能顺着「加完中性槽」直接接线：实测有 5/7 个评分分量走命名槽

`scoring.rs:171-183` + `map_signal` 的取值来源逐条核对：

| 分量 | 取值 | 命名槽被丢弃时 |
|---|---|---|
| trend | `ma_alignment`（比 `ma5/ma10/ma20/ma60` 四槽大小） | 四槽全等于收盘价 ⇒ 恒「缠绕/交叉」⇒ 恒 12 分 |
| deviation | `bias_ma5`（由 `ma5` 推） | 恒 0 ⇒ **恒最高档 20 分**（比缺席更糟，是伪装成"贴均线走强"） |
| rsi | `rsi6` | 恒 50 ⇒ 恒中间档 |
| support | `support_levels`（由 `ma5/ma10/ma20/ma60` 组） | 全等于收盘价 ⇒ 支撑=压力=现价 ⇒ `score_support` 恒命中容差 |
| signal 映射 | `ma_alignment` | 恒「中性」 |
| macd / volume / boll | 由 `cfg` 的周期**直接**算 | 不受影响 ✓ |

⇒ 档位链若只填 `[短带, 长带]` 两根带，四个命名槽仍然空 ⇒ 七个分量里五个静默退化。
**这不是"再加一个字段"能绕过的**，是填槽方式（按数值 vs 按索引）与带数（2 vs 4）的口径选择。

### (3) 两条可执行形状与各自的真实代价

- **甲：按索引填命名槽 + 带数补足到四条**（`ma_periods` 升成 ladder `[base, 2b, 4b, 长带]`，
  日线 base=5 ⇒ `[5,10,20,60]` 逐字不变 ⇒ 日线零回归可证）。
  代价是**硬下限**：`base ≥ 2`（`MA1` 等于收盘价本身，喂给 `bias_ma5` 就是恒 0 的另一种伪装）
  ⇒ 中带 ≥ 4 ⇒ 长带 = 3×中带 ≥ 12 根粗尺度 bar ⇒ **季线 12 根 = 12 年、月线 12 根 = 1 年**；
  长档的出分门槛从「15 年」降到「12 年」，但仍然把绝大多数 A 股判成样本不足。
  收益：评分七个分量**一行都不用改**就拿到真值；payload 键不变 ⇒ 历史行、TS 类型、面板、
  提示词的键名全部不动；撒谎风险由同 payload 的 `windows` 声明 + 一条新门（档尺度脚本读 `maNN`
  必须同批读 `windows`）承担。回退成本：填槽改回 `match by value` 即回退。
- **乙：两带 + 新写一条档位评分路径**（`score_scale_aware`，trend 用 `scaleTrend` 的差幅与斜率、
  deviation 用「收盘对快带」的偏离、支撑压力用两带本身），命名槽完全不参与。
  收益：不为了凑四槽把出分门槛抬到 12 根 ⇒ 长档在多数票上**能出分**；字段名不再兼任两种意思。
  代价：`ScoringEngine` 多一条公开路径 + 档位评分的七个分量要逐条重新给依据（尤其
  `score_support` 的容差与 `map_signal` 的阈值），并且这些改动**直接落在决策数值上**；
  `pm-h-*.rhai`/面板/prompt/11 语言要跟着改读新 payload ⇒ 面比甲大一档，且需要实机核对才能验收。

### (4) 一句边界声明

两条形成本身都能自证，但「长档要不要因为口径而常态性无结论」是产品后果而不是实现细节 ⇒ 交用户一句裁定。
**#36 改图不依赖这条**：扇出的 needs 只多一条「本档尺度常量」，与甲/乙无关，可与本裁定并行。

## 九十九、乙 已落：`score_scale_aware` + `scaleMomentum` 中性槽（2026-10-07，**不发版、现网零变化**）

用户拍 **乙**（两带 + 新写档位评分路径），并要求 **#36 不得切成半成品**（整批做）。

| 位置 | 内容 |
|---|---|
| `indicators.rs:62-70` | `ScaleValue { period, value }`（周期与值同处一处，不靠字段名暗示周期） |
| `indicators.rs:51-60` | `scale_momentum: Option<ScaleValue>` —— 取 `rsi_periods` 里**最快**那个周期，角色逐一对应命名槽 `rsi6`；样本不够 ⇒ `None`（不拿 50 冒充读数） |
| `indicators.rs:417-424` | `scale_momentum_of()` + 两个构造点接线（早退分支 `None`） |
| `scoring.rs:222-285` | `ScoringEngine::score_scale_aware(ind, price, weights, profile)`：五个退化分量（trend / deviation / rsi / support / signal 映射）改读中性槽与两带；macd/volume/boll 本来就按 `cfg` 周期直接算 ⇒ 照旧 |
| `scoring.rs:516-559` | 测试 `score_scale_aware_uses_neutral_slots_not_named_slots` |

三条要点：

1. **不新增常数**：张开阈值 = `deviation_band_1 × √d`（季线 √60≈7.75 ⇒ 1% 变 7.75%）；
   分档值沿用既有 30/20/12/0；「空头但在收敛」不另造分值、落回「缠绕」档
   ⇒ 档位路径与日线路径共用同一张刻度表；`map_signal` 需要的形态标签由**同一个** trend 分档反推
   ⇒ 结构与措辞不可能各说一套。
2. **退化被正面钉住**：夹具就是「命名槽全空、只有中性槽有值」的档尺度输入 ⇒ 断言 多头 > 缠绕 > 空头，
   并**同时**断言 legacy `score()` 在同一输入下认不出张开（`legacy == flat` 且 `!= bull`）——
   「乙确实修掉了退化」不靠自述。
3. **`scaleTrend` 缺席那支在生产上不可达**（`min_bars` 已在取数侧拦住样本不足）；保留它只为
   「有人绕过入口」时退回命名口径并在 `windows` 里如实标出，不给空分也不给假分。

一次自纠（记下来免得重犯）：测试先插进了 `impl ScoringEngine`（我拿 `/// 基本面调整` 那条文档当锚点，
它在 impl 块内而不是 tests 模块），搬进 `mod tests` 并补 `use crate::indicators::{ScaleTrend, ScaleValue}`
后才编译。⇒ 锚点必须选在**目标作用域内**的唯一文本，不能只看"这段文字只出现一次"。

读数：`cargo test -p axagent-astock-data --lib` ⇒ **550 passed / 0 failed; 8 ignored**（
`scoring::tests::score_scale_aware_uses_neutral_slots_not_named_slots ... ok`）｜
`cargo clippy -p axagent-astock-data --all-targets -- -D warnings` ⇒ **0**｜
`rustfmt --edition 2024 --check`（scale/indicators/scoring 三文件）⇒ **0**。

**接下来两条各自不可切，按序做**：
- **#36**：主图删 12 节点与它们的边 + 加 4 个 id 仍为 `pm-h-<档>` 的 SubWorkflow 扇出（`input_mapping`
  按 §九十二(2) 的 needs 逐条填全）+ 播种 4 张档模板与 `HORIZON_TIER_TEMPLATE_VERSION` +
  `TEMPLATE_VERSION = 135` + 门跟改（`audit-inject-coverage` 扩面、`seed_consistency` 节点计数/入度/H1、
  扇出登记无缺口且双向一致、§九十一(6) 第二步：R3 归零后改判或退役、R1 换成按扇出映射判）。
- **#41**：`compute_scoring` 收 `horizon` ⇒ 用 plan 的窗口/`fetch_limit`/`min_bars` 并走
  `score_scale_aware`；`pm-h-*.rhai`、分析师 prompt、面板 4 表、11 语言改读
  `windows` / `scaleTrend` / `scaleMomentum`；补 §九十二(4) 末那条新门
  （ToolNode `input_mapping` 的值必须在「种子变量 ∪ 本图节点 id/output_var」里有源）；换代 + 实机核对（#48 挡）。

## 一○○、#36 的可机械执行清单（本轮不执行：预算不足以一次做完并验完，而本批不可切）

2026-10-07 决定：不在本会话开刀。原因写清楚，免得下一个人以为"当时有什么挡住了"——
本批需要约 15 次编辑 + 主 crate lib(8min)/clippy(9min)/node 门/vitest 四轮读数，
剩余预算不够做完并验完；而 §九十一(3) 与用户裁定都禁止切成半成品（删了主图节点而扇出/播种/门没跟上
＝ CI 绿、四档分支实际不存在）。⇒ 这里把唯一还需要"想"的两件事算完，落地时只剩搬运。

### (1) 子模板要先加一个尺度常量节点，needs 才成立

按丙，评分工具节点的 `period` 必须来自**变量**（`tool_executor.rs:75-80` 无字面量兜底），
而父池里没有名为 `hourly/weekly/monthly/quarterly` 的变量（§九十二(4) 的根因）⇒
不新增种子变量（面板可调参数要满五点对账，且尺度不该可调，`seed_variables.rs:2767` 已记这条裁定），
改成**每张档子模板自己产常量**：在 `t-scoring-*` 之前插一个 `DataTransformer` 节点：

- id `const-scoring-period`、`input_var: ""`、`expression: "\"hourly\""`（各档换成本档 `Period::scale_key()` 的字面量，
  **逐字四份**，理由同 §九十一(2.5)）、`output_var: "scoring_period"`；
- 评分节点的 extra 由 `("period","hourly")` 改成 `("period","scoring_period")`；
- 于是 `scoring_period` 落在子模板自产集里 ⇒ needs 少掉 `hourly/weekly/monthly/quarterly` 这一条。

### (2) 改完后的 needs 期望值（扇出门必须逐条命中，父侧映射**全是恒等键**）

扇出的 target 键必须等于子模板读取的根名（`map_inputs` 以 target 键作子快照的变量名），
而子模板读的就是 `t-risk` / `t-scoring` 这类节点 id 与工作流变量名 ⇒ **每一行都是 `("X","X")` 恒等映射**，
下表即四张扇出节点的完整 `input_mapping`（8/11/13/11 减去常量条）：

| 档 | 父扇出 `input_mapping`（键=值，逐字写进种子） |
|---|---|
| ultra_short | `stock_code`、`t-risk`、`t-scoring`、`t-limitup-pool`、`horizon_branch_json`、`horizon_prior_json`、`stop_vol_mult` |
| short | 上述除 `t-risk` 外全部 + `t-lockup-data`、`a-hot-money--short`、`take_profit_vol_mult`（即 `stock_code`、`t-risk`、`t-scoring`、`t-limitup-pool`、`t-lockup-data`、`a-hot-money--short`、`horizon_branch_json`、`horizon_prior_json`、`stop_vol_mult`、`take_profit_vol_mult`） |
| mid | short 的十条 + `t-valuation`、`t-valuation-band`、`t-consensus-data`，并把 `a-hot-money--short` 换成 `a-hot-money--mid` |
| long | `stock_code`、`t-risk`、`t-scoring`、`t-valuation`、`t-valuation-band`、`t-consensus-data`、`t-macro-data`、`horizon_branch_json`、`horizon_prior_json`、`stop_vol_mult` |

⚠ 三条落地时的硬校验，别靠这份表自证：
① `audit_fanouts` 必须**报零缺口**（它会用改完的子模板现算 needs，键名与上表不一致就红）；
② `registered` 与 `fanouts` 双向一致 ⇒ 四张模板都要进 `seeded_child_templates()` 的数据源表；
③ 扇出节点的 `output_var` 保持 `h_<档>`、节点 id 保持 `pm-h-<档>` ⇒ `portfolio-mgr` 的
   `pm-h-<档>.result` 四个映射键**一个字都不用改**（§九十一(0) 的双键写回理由）。

### (3) 删除与新增的边界（行号是今天的读数，动手时按内容重找）

- 删：`seed_stock_analysis.rs:4348-4399`（四个 `t-scoring-*` 及其三条错峰边——错峰声明在主图归零，
  同批改 `check-tier-purity` 的 R3，§九十一(6) 第二步）＋ `:4611-4825`（四个 `cls-risk-level-*` 块与四条
  `e-t-risk-*` 边）＋ `:5787-5849`、`:5875-5945`、`:5947-6031`、`:6033-6104`（四个 `pm-h-*` 块与供给边）；
  `:5764-5766` 那个 `for src in [pm-h-*] ⇒ portfolio-mgr` 的四条供给边**保留**（源节点 id 不变）。
- 增：同位置放四个 `WorkflowNode::SubWorkflow`，id `pm-h-<档>`、`sub_workflow_id = stock-horizon-<档>`、
  `output_var = h_<档>`、`input_mapping` 见 (2)；每个扇出节点的入边沿用今天 `pm-h-*` 的上游边
  （`t-scoring`/`t-risk`/…/`a-hot-money--<档>`/`debate-*` 等）——**父图等谁，就等谁的节点**，
  因为数据现在由父池传入。
- 播种：`HORIZON_TIER_TEMPLATE_VERSION = 135` + `TEMPLATE_VERSION = 135`，四行模板 `is_preset=true`、
  无 trigger/无快照历史（快照只服务主链代际回查），`hooks_config` 留空（子执行继承父的注入）。

## 一○一、#36 落地（B-2b 步骤 2 + 2.5）= **v135**：四档分支 SubWorkflow 化（2026-10-07）

用户裁定沿用：#7① 真改图 SubWorkflow×4 ｜ 粒度「整段搬」｜ period 死参数按丙「接通 + 同步改出分口径」｜
#36 不得切成半成品。**全程不 commit**（树内有他人未提交改动）。

### (1) 一处对 §九十一(0) 形状的**偏离**（动手时普查出来的，不是选择）

§九十一(0) 的可搬运边界把每档三个节点（`t-scoring-<周期>` / `cls-risk-level-<档>` / `pm-h-<档>`）
都算进子模板。普查后 **`cls-risk-level-<档>` 不能进子模板**，因为父图有四个真实消费者：

| 消费点 | 位置 |
|---|---|
| `portfolio-mgr` 的 `overall_risk_{ultra_short,short,mid,long}` 四条映射 | `seed_stock_analysis.rs`（v128 B1 段，四条 `("overall_risk_…", "cls-risk-level-….result.category")`） |
| 其消费脚本 `tier_risk_raise` | `portfolio-mgr.rhai`（四键都进 `present()` 守卫） |
| 四条供给边 `cls-risk-level-<档> → portfolio-mgr` | 同文件的 v128 边段 |
| 前端阶段映射表（四个 id 各占一格） | `src/stores/feature/stockAnalysisStore.ts` 的 NODE_CLASS_TABLE |

而子执行**只**把父扇出节点的 `node_id` + `output_var` 双键写回父池
（`crates/rt-workflow/src/work_engine/engine/mod.rs:1791-1796`），End 之外的一切子图变量都是私有的
⇒ 风险节点一进子模板，那四条映射就指向不存在的路径 ⇒ `present()` 恒假 ⇒ **v128 B1 的按档风险收紧
整条静默退役**（本仓登记的「配置项空接线 / 声明与真实契约不符」族）。
把它带回父侧需要改四份分支脚本的输出 + 主链读端，而 §九十一(4) 明写本批「`pm-h-*.rhai` 脚本内容不变」。

⇒ **本批形状**：子模板 = `const-scoring-period` → `t-scoring-<周期>` → `pm-h-<档>` → `end`（四节点）；
`cls-risk-level-<档>` 仍逐档展开在**父图**，并经扇出以恒等键传进子快照。
判据跟着换载体而没有退役：「分支读本档那一格」搬到
`horizon_tier_template.rs:1057` 的 `risk_cell_problems`（负控 `branch_reading_global_risk_cell_is_caught`），
父侧那半（扇出是否传了本档风险节点）仍在 `mod.rs` 的 `check_horizon_scoped_risk` 里，
其负控 ② 已改成「扇出只传全局那个 ⇒ 必须红」。
要恢复原裁定（风险节点也进子模板）的前置 = 先给 `h_<档>` 加一个携带风险档的出口键，
那属 #41（改四份分支脚本 + 主链读端 + 面板）。

### (2) needs 的机器读数（改图后重跑，不抄表）

`cargo test -p axagent --lib horizon_tier_template -- --nocapture` 的 `[B-2b needs]` 行：

```
[B-2b needs] ultra_short ⇒ 7 个外部键：cls-risk-level-ultra-short、horizon_branch_json、horizon_prior_json、stock_code、stop_vol_mult、t-limitup-pool、t-scoring
[B-2b needs] short ⇒ 10 个外部键：a-hot-money--short、cls-risk-level-short、horizon_branch_json、horizon_prior_json、stock_code、stop_vol_mult、t-limitup-pool、t-lockup-data、t-scoring、take_profit_vol_mult
[B-2b needs] mid ⇒ 12 个外部键：a-hot-money--mid、cls-risk-level-mid、horizon_branch_json、horizon_prior_json、stock_code、stop_vol_mult、t-consensus-data、t-lockup-data、t-scoring、t-valuation、t-valuation-band、take_profit_vol_mult
[B-2b needs] long ⇒ 10 个外部键：cls-risk-level-long、horizon_branch_json、horizon_prior_json、stock_code、stop_vol_mult、t-consensus-data、t-macro-data、t-scoring、t-valuation、t-valuation-band
[B-2b shape] 每张 ⇒ 节点 4 个（const-scoring-period、t-scoring-<档>、pm-h-<档>、end），边 3 条
审计 5 张图；图内扇出 4 个（stock-horizon-long,stock-horizon-mid,stock-horizon-short,stock-horizon-ultra-short）；system_* 排除 0 个
```

四张模板各四节点，父扇出的 `input_mapping` 与之**逐条相等**且全是恒等键（`("X","X")`）：
ultra_short 7 ／ short 10 ／ mid 12 ／ long 10。与 §一○○(2) 的表相比只差一处：
`t-risk` 换成 `cls-risk-level-<档>`（见 (1)），并少掉 `hourly/weekly/monthly/quarterly` 那条假键
（尺度常量由子模板自产，见 (3)）。

### (3) 尺度常量节点 = 丙 的「接通」在本批落地的形态

`horizon_tier_template.rs:99` `tier_scoring_period_const_node()`：
DataTransformer，`input_var: ""`、`expression: "\"hourly\""`（逐字四份）、`output_var: "scoring_period"`。
评分节点的 extra 从 `("period","hourly")` 变成映射 `("period","scoring_period")`。
判据两条：`scoring_period_constant_matches_scale_key_authority`（常量必须等于权威
`Period::scale_key()`）+ 负控 `wrong_scale_constant_is_caught`（改成 `"weekly"` 必红）。

**现网数值影响（如实）**：
· 四档评分从「恒日线」变成各按本档尺度取数 —— 这是 §九十二(4) 那条死参数的修复，也是丙 的代价；
· `scale.rs` 的 `min_bars=60` 在月线=5 年、季线=15 年 ⇒ 样本不足的票该档评分节点显式失败；
· 出分口径的正式改造（`ScaleWindowPlan` 的窗口 / `fetch_limit` / `score_scale_aware`）在 **#41**，
  所以本批是「按尺度取数 + 仍用日线 ladder 出分」的中间态，不宣称已完成。

### (4) 落地面（每行都是断言）

| 位置 | 内容 |
|---|---|
| `seed_stock_analysis.rs:1643` | `TEMPLATE_VERSION` 134 → **135** + v135 六条变更说明（含两条数值影响） |
| `seed_stock_analysis.rs`（原 `:4348-4403`） | 删四个 `t-scoring-*` `tool_node` + 三条错峰边 + 四条**无人消费**的「→ portfolio-mgr」边，换成一段说明「搬去哪、谁还在管串档」 |
| `seed_stock_analysis.rs`（原 `:5787-6105`） | 四个 `pm-h-<档>` CodeNode 块 → 四个 `SubWorkflowNode`（id / `output_var` 不变），供给边按「父图谁被引用」逐条重算（去掉 `t-scoring-<档>`，其余沿用） |
| `seed_stock_analysis.rs` 快速链 | `FAST_REQUIRED_NODE_IDS` 摘掉四个评分节点；`FAST_BRIEF_INPUTS` 四路 `algo_scoring_*` 改指**日线** `t-scoring` ⇒ 快速链出参逐位不变（决策点 1「不跟」） |
| `horizon_tier_template.rs` | 四张模板改为「常量 → 评分 → 分支 → 终值」四节点；删掉 10-06 版本里的 `cls-risk-level-*` 块（回归父图） |
| `horizon_tier_template.rs:762` | `HORIZON_TIER_TEMPLATE_VERSION` **绑到** `seed_stock_analysis::TEMPLATE_VERSION`（不是抄 135）⇒ 扇出键集与模板内容不可能分代 |
| `horizon_tier_template.rs:785` | `seed_horizon_tier_templates()`：四行 `is_preset=true`、`is_public=false`、`visibility=SystemOnly`、无 trigger、`hooks_config=None`、走 `upsert_workflow_template`（该函数不写快照表 ⇒ 「无快照历史」由实现保证） |
| `stock_analysis_setup/mod.rs:24` | 去掉 `#[cfg(test)] mod horizon_tier_template;` 的 gate ⇒ builder + 播种转生产可见 |
| `stock_analysis_setup/mod.rs:858` | 主图/快速链之后调 `seed_horizon_tier_templates`（缺一行 = 该档显式失败，日志按非致命但点名） |
| `seed_consistency_tests.rs` | `branch_node_scope_is_exactly_the_seed_mapping` 抽映射的载体由主图种子换成 `include_str!("horizon_tier_template.rs")`（判据本体不变） |
| `mod.rs` 末段新增 `horizon_tier_fanout_tests` | 三条：键齐备正对照（零缺口 / `registered == fanouts` / 无盲区 / 扇出指向本档模板 / 四行版本落库）+ 抽键负控 + 形状锁（四节点） |

### (5) 门跟改（四条，全部与改图同批）

| 门 | 改法 |
|---|---|
| `scripts/audit-inject-coverage.mjs` | 扫描面扩到档模板 builder：新增 `--tier <档>` / `--tier all`（档名由权威 `Period::as_str` 现推，锚在脚本内部拼 —— Windows 的 cmd.exe 会吃掉命令行里的 `"`，而 include 锚必须含 `"`）；`--selftest` 加两条 ★护栏：四档扫描面非零、四档 target 面**两两不同** |
| `.github/workflows/ci.yml` + `scripts/ci-check.mjs` | 各加一条 step `--tier all --strict`（两处逐字同源） |
| `scripts/check-tier-purity.mjs` | §九十一(6) **第二步**：收紧到只认新形（`branchBlocks` 只读 builder）；新增 **R6**「主图 `pm-h-*` 的 CodeNode 块必须归零 + 四个 `pm-h-<档>` 的构造器必须是 SubWorkflow」；**R3 改判**为「主图与档模板都不得出现档间评分依赖」，`DECLARED_SCORING_STAGGER` 清空 = 登记「四路互不等」这个决定；`transitionDrift`（过渡期逐字一致）随之退役 —— 它锁的对象已不存在，退役理由写在函数位置的原地注释里；自证 17 条 |
| `mod.rs` / `horizon_tier_template.rs` 的 Rust 门 | 扇出键齐备性（`workflow_fanout_audit` 首次拿去打真图）+ 尺度常量与权威一致 + 分支读本档那一格（换载体） |

扇出登记与 `audit_fanouts` 的双向一致：四张模板都进 `seeded_child_templates` 等价的数据源
（本批是「从 DB 读回四行」而不是直接调 builder —— 判据要证的是落库那四行与主图扇出对得上）。

### (6) 语义变化（不是回归，但界面看得见）

1. **缺席形态变了**：`map_inputs` 严格 ⇒ 父侧任一被引用节点没产出（工具失败 / 分析师实例失败）
   会让**整档**子执行失败，而不再是「该腿注入 unit、脚本按缺席处理」。
   主链不塌（四个扇出各自 `continue_on_fail=true`），但分支脚本 `dataGaps` 里
   「该节点整段没跑成」那一类文案在新形状下不再可达 ⇒ 缺席句子只剩「取数层给的原因」+
   「整档失败」两族，#41 之后要按新形状重核对面板与 i18n 文案。
2. **面板呈现**：四档各自一条**子执行**进度（B-2a 铺好的呈现面）—— 父节点行仍是 `pm-h-<档>`，
   子图内 `const-scoring-period` / `t-scoring-*` / `pm-h-<档>` / `end` 的事件按子执行 id 分桶。
   这条**不能由 CI 自证**（#48 挡），与 #13/#30 同批实机核对。
3. 主图节点数 **−4**（删掉四个 `t-scoring-*`；四个 `pm-h-<档>` 是同名换构造器，不是增删），
   DebugPanel 的节点计数与画布会跟着少四格；边数净减 **12** 条
   = 评分节点的 4 条入边（其中 `week→month`、`month→quarter` 是真档间错峰）
   + 4 条无人消费的「`t-scoring-<档>` → portfolio-mgr」
   + 四档各自那条「`t-scoring-<档>` → `pm-h-<档>`」（评分现在在子图里，父扇出只等日线 `t-scoring`）。
   净增 0：每个扇出的入边沿用原 `pm-h-<档>` 的其余上游（`t-scoring` / `t-risk` 间接 /
   `cls-risk-level-<档>` / `t-limitup-pool` / `t-lockup-data` / 本档游资实例 / 估值 / 一致预期 / 宏观）。

### (7) 门禁读数（只记日志里有的）

| 命令 | 读数 |
|---|---|
| `node scripts/check-tier-purity.mjs --selftest` | ✅ 自证通过（17 条对照，含「最长命中」「剥注释」「判据不存在」「旧形残留必须红」四类负控） |
| `node scripts/check-tier-purity.mjs --dump` | EXIT=0；现场档间评分依赖 = 无｜声明 = []；主图残留旧形 = 无；四档评分源 = hour/week/month/quarter 各一 |
| `node scripts/audit-inject-coverage.mjs --selftest` | SELFTEST PASS（含两条新 ★护栏：四档扫描面非零、四档 target 面**两两不同**） |
| `node scripts/audit-inject-coverage.mjs --tier all --strict` | EXIT=0（四档 ①②③ 全空） |
| 其余 node 门（input-mapping / horizon-weight-parity / reco-horizon-parity / analyst-id-shape / rhai-numeric-typing / generation-scope-registry / horizon-decision-keys / contracts / domain-semantics / layer-discipline / ontology / domain-single-source / office-scene-align / decl-evidence --ci / port-axiom-wiring / snapshot-whitelist / id-validation / errorcode-alignment / sql-dialect / rust-raw-map-err / single-source-facts --ci） | **全 EXIT=0**；`check-layer-discipline` 报 `commands-no-direct-db 656 处（基线 680）✅ 通过`（本批新文件已从 1 处降到 0：签名里的 `sea_orm::DatabaseConnection` 改走 `axagent_dao::db::DatabaseConnection`）；`check-single-source-facts --ci` 分类 LOCATED 208 / BROKEN-越界-空行 0 |
| `__TAURI_WORKSPACE__=true cargo test -p axagent --lib horizon_tier -- --nocapture` | `test result: ok. 9 passed; 0 failed; 551 filtered out` ⇒ `T1_EXIT=0`（needs / shape 见 (2)，审计行见 (2) 末） |
| `__TAURI_WORKSPACE__=true cargo test -p axagent --lib stock_analysis_setup::` | `test result: ok. 66 passed; 0 failed; 494 filtered out` ⇒ `T2_EXIT=0`（含 `tier_fanout_inputs_are_complete` / `tier_fanout_missing_key_is_caught` / `seeded_template_carries_horizon_scoped_risk_nodes` / 快速链派生全套 / 版本门三态） |
| `cargo fmt --all --check` | EXIT=1 —— **两处 diff 全在 `crates/astock-data/src/indicators.rs`（他人未提交文件，本批没动它、也没替他人格式化）**；本批四个 Rust 文件用 `rustfmt +1.97.0 --edition 2024 --check` 单独验 ⇒ 0 diff |
| `cargo clippy --workspace --all-targets --all-features -- -D warnings` | `Finished dev profile in 6m 45s` ⇒ `CLIPPY_EXIT=0`，`^error` 计数 0 |
| `__TAURI_WORKSPACE__=true cargo test --workspace --no-fail-fast` | 144 个测试二进制里 **2 条红**（其余全绿）：主 crate lib `560 passed; 0 failed`、`astock-data` lib `550 passed; 0 failed; 8 ignored`。两条红都**不在本批面上**，且各自取到了独立读数 ↓ |
| 红 1：`stock_workflow_write_side::single_stock_channel_w_generation_at_persist` | 报 `run_single_stock_analysis 之后应还有别的顶层函数`。现场读数：`core.rs` 的顶层函数序为 `:248 / :426 / :1868 / :1892`，**`:1892` 就是最后一个** ⇒ 该门的「窗口终点」定位不到。`core.rs` 在他人未提交清单里，本批一行未动 |
| 红 2：`analysis-engine::recommender::tests::cache_invalidate_works` | 全量跑红（`assert!(cache_get(Period::Short).is_none())`），**单跑绿**：`cargo test -p axagent-analysis-engine --lib recommender::tests::cache_invalidate_works` ⇒ `1 passed; 0 failed; 611 filtered out` ⇒ 共享全局缓存的并行互扰（登记为 flaky，不属本批） |
| `npx vitest run` | 首跑 `1 failed \| 2176 passed` ⇒ 那条红**是本批的**：`StockAnalysisConfigPanel.test.tsx` 的「同名映射必须在变量表或运行时注入白名单里」抽到 `("horizon_branch_json","horizon_branch_json")` —— v135 前它只以 `<变量>.<档>` 子路径出现，故不在同名抽取面里；扇出必须按恒等键整张传 ⇒ 首次成为同名映射。修法 = 把它补进 `RUNTIME_INJECTED`（与 `horizon_prior_json` 同源，注释写明生产点 `hooks.rs:391-404`），**不是**放宽判据。改后重跑 ⇒ `195 passed (195)` / `2177 passed (2177)` ⇒ `VITEST_EXIT=0` |
| `npm run typecheck` + `npx dprint check --incremental=false` | 均 EXIT=0 |

⚠ `cargo check -p axagent --lib --tests` 先跑过一次：`Finished dev profile in 15m 39s` / `EXIT=0`，
那轮的唯一 error 是我自己写的两处（`await` 不在 async 闭包里、`hm_short`/`hm_mid` 移动后再借用），
都已修；修完后重跑的读数就是上表 `horizon_tier` / `stock_analysis_setup::` 两行（9 passed 与 66 passed）。

### (8) 界面后果（现在能确定的 / 还确定的）

**能确定（不依赖运行）**：主图画布少四个 `t-scoring-*` 节点、`pm-h-<档>` 从 Code 变成子工作流节点
（标题跟着改成「…分支决策（子工作流）」）；四档的**结论字段与读端逐字未动** ⇒
面板上四格的方向 / 仓位 / 置信度不会因为本批而换形态。

**要跑一次才知道的（被 #48 挡）**：
· v135 重播种（主图 + 四张档模板行）；
· 四档各自一条子执行进度（B-2a 铺的呈现面），子图里四行的事件按子执行 id 分桶；
· 中/长档在样本不足的票上会出现**整档显式失败**（不是「该腿缺席」）⇒
  那一格在界面上是「失败」而不是「算不出来」，与 (6) 的缺席形态变化是同一件事；
· 任一被引用的父侧上游失败 ⇒ 整档失败 ⇒ `dataGaps` 里那条「该节点整段没跑成」文案不再可达，
  #41 之后要按新形状重核对面板与 11 语言文案。
**本批不 commit**（按指令；树内有他人未提交改动）。


## 一○二、#41 第一片落地：ToolNode 参数**必须有源**的门（2026-10-08，**不发版、零数值影响**）

§九十二(4) 末承诺的那条门补上了 —— 「这类死参数以后 CI 直接红，不依赖读数」。
它是 #41 里唯一可独立落地的一片（不动取数口径、不动脚本、不动面板 ⇒ 现网逐位不变），
先落它，#41 剩下的两片就有网接着。

### (1) 落地面

| 位置 | 内容 |
|---|---|
| `src-tauri/src/workflow_fanout_audit.rs:134` | 把 `external_reads` 内的「自产集」抽成 `produced_names()`（两条判据共用同一份公式，不抄第二份），新增 `external_reads_for_kind(nodes, kind, blind)` ⇒ 按节点种类切读取面 |
| `commands/stock_analysis_setup/mod.rs` 的 `horizon_tier_fanout_tests` | 新增判据 ⑥：`tool_node_argument_sources_exist`（主图 + 四张档模板）+ 两侧负控 `dead_tool_argument_is_caught_in_tier_template` / `dead_tool_argument_is_caught_in_main_graph` + `runtime_injected_names()` |

判据：每个 ToolNode 的 `input_mapping` **值**的根名，必须落在
「本图产出（节点 id ∪ `output_var`）」∪「本图外部可源」∪「引擎注入四常量」里。
外部可源：主图 = 落库行的种子变量表；档模板 = 父扇出传进来的键。
**为什么单列一条而不是复用 `audit_fanouts` 的 needs**：工具**参数名**不是工作流变量，
它根本不出现在「子模板要读什么、父得传什么」那张面上 —— §九十二(4) 那条 `("period","hourly")`
就是靠这个缝活下来的（四档评分恒等日线，`check` / `clippy` / 全套门全绿）。

### (2) 两次首跑读数教我的事（都是门自己的口径缺陷，不是图的）

1. **把子图面的断言搬到整图面**：首版写 `assert!(blind.is_empty())`，主图立刻报
   `{"aggregator","debate","notification","parallel","storage","switch"}` 六个未覆盖种类。
   那六个是本模块文档里早已声明的盲区，与「按 `tool` 切开的读取面」无关。
   改判成：**本门载体种类（`tool`）必须被覆盖**，其余种类如实打印，并写明误差方向是
   **多报**（那些种类写出的变量没进自产集 ⇒ 被当成无源）而不是漏报 —— 少报才会让门绿着空转。
2. **`stock_code` 被误报成无源**：它不在模板 `variables` 列里，由 hooks 在每次 run 前 push
   （`hooks.rs:354` 的表 + 多处 `Variable { name: … }`）。修法**不是**在 Rust 里手写一份白名单
   （那会与 `StockAnalysisConfigPanel.test.tsx` 的 `RUNTIME_INJECTED` 成两份各自漂移的副本，
   正是本仓「清单要有单一权威」要拦的形态），而是**从注入点的构造现场扫出来**：
   `runtime_injected_names()` 在 `hooks.rs` / `core.rs` 上推
   `name: "<x>".into()` 与 `("<x>", json!(…))` 两种形态，并自证「推到的数量 > 10 且含 `stock_code`」。
   顺带封住漂移方向：hooks 哪天不再注入某个名字 ⇒ 它自动离开集合 ⇒ 还在用它当工具参数的图立刻红。

### (3) 读数

| 命令 | 读数 |
|---|---|
| `__TAURI_WORKSPACE__=true cargo test -p axagent --lib horizon_tier -- --nocapture` | 首跑 **10 passed / 2 failed**（两条红的都是新判据自己：盲区断言搬到整图面 + `stock_code` 被判无源）⇒ 修完重跑 **12 passed / 0 failed** ⇒ `K1_EXIT=0`；按测试名点名确认跑过：`tool_node_argument_sources_exist ... ok`、`dead_tool_argument_is_caught_in_tier_template ... ok`、`dead_tool_argument_is_caught_in_main_graph ... ok` |
| `__TAURI_WORKSPACE__=true cargo test -p axagent --lib stock_analysis_setup::` | **69 passed / 0 failed**（= #36 落库时的 66 + 本批 3）⇒ `K2_EXIT=0` |
| `__TAURI_WORKSPACE__=true cargo test -p axagent --lib` | **563 passed / 0 failed**（改前 560）⇒ `L_EXIT=0` |
| `cargo clippy -p axagent --all-targets --all-features -- -D warnings` | `CLIPPY_EXIT=0`（改动前后各跑一次，均 0） |
| `cargo fmt --all --check` | EXIT=1 —— 两处 diff 仍全在他人未提交的 `crates/astock-data/src/indicators.rs`；本批两个文件 `rustfmt +1.97.0 --check` 0 diff |
| node 门 12 条（tier-purity 自证 + 实跑、`audit-inject-coverage` 自证 + `--tier all --strict`、analyst-id-shape、generation-scope-registry、layer-discipline、rust-raw-map-err、single-source-facts --ci、contracts、input-mapping、horizon-weight-parity） | **全 EXIT=0** |

**现网零变化**：本批只加判据与一个按节点种类切开的取值函数，未动取数、未动脚本、未动面板 ⇒ 不发版、
`TEMPLATE_VERSION` 不抬。

### (4) #41 剩下的两片（顺序固定，两片各自不可切）

- **片 A（取数 + 出分口径）**：`mcp_tools.rs::compute_scoring` 的 `("period")` 一路改用
  `ScaleWindowPlan::for_period(...)` 的 `fetch_limit` / `min_bars`（季线还要 `parent_fetch_limit`），
  指标改走 `compute_indicators_with_config` 传 plan 派生的 `IndicatorConfig`（含中性槽），
  出分改走 `ScoringEngine::score_scale_aware`，payload 输出 `windows{…}` / `scaleTrend` / `scaleMomentum`。
  **一个待定的形状**：`ScaleWindowPlan::for_period` 收的是 `Period`，而工具现在只拿得到 `period`
  尺度串 ⇒ 要么给档模板再加一个 `const-scoring-horizon` 常量节点并多传一个工具参数（= 图再变、再换代），
  要么在 `scale.rs` 里加 `Period` ↔ 尺度的**反查**并由它判「一尺度一档」（四档尺度互不相同，可反查）。
  我倾向后者：不新增种子形状，且反查表与 `Period::scale_key` 同源 ⇒ 加第五档时编译器会逼着两处一起长。
- **片 B（消费侧改读）**：四份 `portfolio-mgr-h-*.rhai`、分析师 prompt、面板四表、11 语言改读
  `windows` / `scaleTrend` / `scaleMomentum`；换代 v136；实机核对被 #48 挡。
  A 与 B 不可分：只做 A 会让面板继续按日线名目展示按尺度算出来的数（声明与事实分叉）。


## 一○三、#41 片 A + 片 B 落地 = **v136**：按档尺度真接管取数/出分，读端停止读「按数值认领的命名槽」（2026-10-08）

### (0) §一○二(4) 那条待定形状：拍**反查**，不加第二个常量节点

`crates/harness/src/holding_period.rs:281` 新增 `Period::from_scale_key(scale) -> Option<Period>`，
与正查 `scale_key()`（同文件 :246）**同臂同表** ⇒ 加第五档时编译器逼着两处一起长。

- 为什么不加 `const-scoring-horizon` 节点：那会让图再变一次、`TEMPLATE_VERSION` 再抬一次，
  而换代的原因本该是**数值变了**（本批数值确实变了，见 (3)），不该是「为了传一个字符串」。
  反查路径下 `scoring_period` 一个常量就够了，工具参数面不扩。
- 反查成立的前提是**四档尺度互不相同**，这条由 `scale_key_and_scoring_node_are_one_table_per_tier`
  （同文件 :935）钉住；负控：`daily` → `None`（日线不属于任何档 ⇒ 主链零回归的钉子），
  `""` / `"weekl"` / `"Monthly"` / `"quarter"` 全 `None`（**一一而非就近**，拼错不许吸附）。

### (1) 片 A：接管三件事 + 一条出分路径

| 位置 | 内容 |
|---|---|
| `src-tauri/crates/astock-data/src/mcp_tools.rs:1292` | `plan = Period::from_scale_key(period_arg).map(ScaleWindowPlan::for_period)`；`plan == None` ⇔ 日线，走既有常数 |
| 同文件 :1294-1297 | ① 向 vendor 取的根数 = `parent_fetch_limit.unwrap_or(fetch_limit)`（季线取的是**聚合前**的月线根数）；② 出分下限 `min_bars` = 计划的 `long_bars` |
| 同文件 :1298 + :1327 | ③ 指标窗口：`IndicatorConfig::from(&plan)` 交给 `compute_indicators_with_config`（日线传 `None` ⇒ 与 `compute_indicators` 逐字同路，见 `crates/astock-data/src/indicators.rs:507`） |
| 同文件 :1337 | ④ 出分：按档尺度改走 `ScoringEngine::score_scale_aware`（§九十九那条），日线继续 `score_with_bands` + `ScoreBands::scaled_for` ⇒ **逐位不变** |
| `crates/astock-data/src/indicators.rs:172` | 新增 `impl From<&ScaleWindowPlan> for IndicatorConfig` —— **只搬窗口，不搬阈值**（§九十六(2) 裁定：RSI 是 0-100 有界统计量，与 bar 的日历跨度无关）；`rsi_periods` 只给**一个**周期，让 `windows.rsiPeriods` 与 `scaleMomentum{period,value}` 说同一件事，而不是留三个停在 50.0 的假命名槽 |

### (2) 片 B：五处读端改名，键名不再谎报周期

| 位置 | 内容 |
|---|---|
| `commands/stock_analysis_setup/horizon_tier_template.rs:186/258/334/421` | 四张档模板的 `("rsi_14", "…indicators.rsi14")` → `("rsi_value", "…indicators.scaleMomentum.value")` |
| `commands/portfolio-mgr-h-{ultra-short,short,mid,long}.rhai` 的 `raw` 表（:99/:76/:101/:79） | payload 键 `rsi14:` → `rsi:`，值取本档 `rsi_value`；超短那份头部把「旧键为什么作废」写了 4 行 |
| `src-tauri/crates/analysis-engine/src/leg_signal.rs:72` | `momentumSignal` 的调制项 `num(in_, "rsi14")` → `num(in_, "rsi")`（+ 三处测试夹具同批） |
| `commands/portfolio-mgr.rhai:1518` | 主链（日线）payload 键也改叫 `rsi`，**值仍是 `rsi_14` ← `indicators.rsi14`** —— 日线上这个名字是真的，改的只是与档侧共用同一个中性键名 |
| `commands/stock_analysis_setup/seed_consistency_tests.rs`（`value_for` 的 `"rsi_14"` → `"rsi_value"`） | 一致性测试跟着读端改名，否则锁的是旧字面量 |

**为什么这不是 cosmetic**：命名槽 `rsi6/rsi12/rsi14/rsi24` 是**按数值认领**的（§九十七实测「七个分量里五个走命名槽」），
档位换算出的周期几乎不落进 `{6,12,14,24}` ⇒ 那些槽停在初值 **50.0**，而 50.0 恰好命中 `momentumSignal`
的「健康 +0.2」区间 ⇒ **每一档都被无谓抬高 0.2，且抬它的数是初值不是测量值**。这是「伪装成正常结果」，不是「拿不到」。

### (3) 接管之后四档的真实数值（按 (1) 的公式现算，不是抄 §九十六 的表）

| 档 | 尺度 | `ceil(h/d)`→`short` | `long=3×short` | MA 窗口 | RSI | 出分下限 | 取数（旧 → 新） |
|---|---|---|---|---|---|---|---|
| 超短 | hourly | `2/0.25`=8 | 24 | [2, 4, 8, 24] | 6 | 24 根 | 320 → **48** 根小时线 |
| 短 | weekly | `5/5`=1→钳位 2 | 6 | [2, 6] | 2 | 6 根 | 120 → **12** 根周线 |
| 中 | monthly | `28/20`=2 | 6 | [2, 6] | 2 | 6 根 | 120 → **12** 根月线 |
| 长 | quarterly | `90/60`=2 | 6 | [2, 6] | 2 | 6 根 | 240 → **36** 根月线（聚合出 12 根季线） |

⇒ §九十二(5) 那条「月/季档要 60 根才出分（= 5 年 / 15 年历史）」的失败面被这里收敛掉了：
长档回答 90 交易日的问题，只需 12 根季线。代价写清：**短/中/长三档的窗口根数由 `clamp(2,60)` 的下限决定，
不由持有期决定**，三档粒度的差别只剩 bar 的日历跨度（2 周 / 2 月 / 2 季）—— 这条不是本批新发现的，
`crates/astock-data/src/scale.rs:443-447` 那段注释与 `tier_window_plans_cover_holding_period_and_are_admissible`
（:449）已经认了它，判据刻意只断「中带盖得住持有期 + `min_bars ≤ fetch_limit`」而**不**断「四档根数互异」。

### (4) 本轮两条实测（都是**门/工具的口径**，不是图的）

1. **断言「四档 MA 窗口两两不同」恒假**。新测试首跑报
   `Mid 的窗口与别的档重复 ⇒ 两档评分输入恒等：[[2, 4, 8, 24], [1, 2, 6]]`。
   我先以为是缺陷，读数证明是**前提写错**：短/中/长按 (3) 就是同一个整数。
   修法不是放宽断言，而是把判据换成 **(scale_key, ma_periods) 配对唯一**，
   并**另外**钉两条可见性断言 —— `assert_eq!(mid.ma, long.ma)`（“相同是设计使然，变了就要连注释一起重写”）
   与 `assert_ne!(mid.scale_key, long.scale_key)`（“两档尺度真相同才是缺陷”）。见 `crates/astock-data/src/indicators.rs:873`。
   顺带补了 MA 下限 `.max(2)`：粗尺度 `short_bars=2` 时 `2/4` 取到 1，MA1 = 当根收盘价冒充「一条带」，
   会让 `windows` 里出现一个不是均线的数。
2. **单文件 `rustfmt` 与 `cargo fmt` 不是同一口径**。我用
   `rustfmt +1.97.0 --edition 2024 <file>` 自检报 0 diff，而 `cargo fmt --all --check` 报 **5 处**（3 处在我改的 `mcp_tools.rs`）——
   因为 `src-tauri/rustfmt.toml` 的 `use_small_heuristics = "Max"` 之类要经 cargo 才生效。
   ⇒ 定稿口径只能是 `cargo fmt --all --check`；单文件预检快但**不能当门禁**。
   ⚠ 副作用要说：`cargo fmt -p axagent-astock-data` 顺带把 `crates/astock-data/src/indicators.rs` 里**他人未提交**的两处夹具也格式化了
   （本批不 commit，提交时这两处的归属要在 commit message 里点明，别让人以为是自己改的内容）。

### (5) 读数（只报日志里有的）

| 命令 | 读数 |
|---|---|
| `__TAURI_WORKSPACE__=true cargo test -p axagent-astock-data --lib` | **552 passed / 0 failed**（含新增 `plan_derived_config_reproduces_daily_numbers`、`plan_derived_configs_are_per_tier_and_match_actual_windows`） |
| `__TAURI_WORKSPACE__=true cargo test -p axagent-harness --lib` | **783 passed / 0 failed**（含反查负控那批断言） |
| `__TAURI_WORKSPACE__=true cargo test -p axagent-analysis-engine --lib` | **612 passed / 0 failed**（`momentumSignal` 阶梯黄金值**未改** ⇒ 改名前后逐值等价） |
| `__TAURI_WORKSPACE__=true cargo test -p axagent --lib stock_analysis_setup::` | **69 passed / 0 failed** |
| `cargo clippy -p axagent-astock-data/-p axagent-harness --all-targets -D warnings` | 各 `EXIT=0` |
| `cargo clippy --workspace --all-targets --all-features -- -D warnings` | `EXIT=0`（9m10s，**在 fmt 之前**跑的） |
| node 门（tier-purity 自证 + 实跑、`audit-inject-coverage --tier all --strict` 与 `--strict`、analyst-id-shape、input-mapping、horizon-weight-parity、rhai-numeric-typing、generation-scope-registry、layer-discipline、rust-raw-map-err、single-source-facts --ci、contracts） | **全 EXIT=0**（在读端改名之后跑，不是改名之前） |
| `cargo fmt --all --check` | `EXIT=0`（用 `cargo fmt -p axagent-astock-data` 收敛后） |
| `cargo test --workspace --no-fail-fast` / 全量 clippy / vitest 复跑 | 见本节末「(7) 收尾补记」（写本节时全量链仍在编译主 crate） |

### (6) 现网会看到什么

- **中/长档的样本门槛大幅下降** ⇒ 以前整档出不了分的小票现在能出分；出分口径是 `score_scale_aware`（两带 + 中性槽）。
- 某档历史仍不足 `min_bars` ⇒ 该档**整段子执行显式失败**（v135 的严格 `map_inputs` + B-1），界面上是「失败」而不是「该腿缺席」；
  这是 §一○一(6) 已经点过的形状，本批没改它。
- `scaleMomentum` 样本不够 ⇒ 缺席（`skip_serializing_if`）⇒ `rsi_value` 不 present ⇒ 动量腿进 `absent_legs` 并在 `dataGaps` 留痕。
  **不再**是「50.0 冒充健康」。
- 面板**仍不显示**每档实际用的窗口/尺度 ⇒ 读者看不出「四档各自的 MA 是几根」。这不是回归，是 #41 剩下的呈现片：
  四表 + 11 语言 + `normalizeDecision` 白名单透传（`indicators.windows/scaleTrend/scaleMomentum` 目前无人读，前端零引用已核）。
- 实机核对（v136 重播种、四档子执行行、逐档数值、`scoreSource`）被 **#48** 挡。

### (7) 收尾补记

**全量 `cargo test --workspace --no-fail-fast`**：144 个测试二进制、**8250 passed / 1 failed**。
唯一那条红是 `src-tauri/tests/stock_workflow_write_side.rs` 的 `single_stock_channel_writes_generation_at_persist`，
**与本片无关**，且我做了机械证明而不是推定：

- 该门的 `single_stock_window()` 用「本函数之后还有别的顶层 `pub fn` / `pub async fn`」当窗口终点（:34-38 的 `.expect`）；
- 把**同一段算法**分别跑在 `git show HEAD:...core.rs` 与工作树 `core.rs` 上，两份都返回
  「之后没有别的顶层 pub 函数 ⇒ 窗口终点定不了」；
- `git log --diff-filter=A` 指认该文件由 **`507a4af69`**（#31 之前那批）引入，而**在那个提交上探针同样红**
  ⇒ 这是一道**从落地那天就没绿过**的门（当时批次门跑的是 `--lib`，集成测试目录不在覆盖面里 —— 与铁律「新门要过编译+跑通」同源）。
- 修法形状（**另批**，不混进本片）：窗口终点改成「本函数自己的收尾 `}`（列 0）」——
  rustfmt 对顶层 item 保证这一点，且天然不吞后面的 `mod tests`（吞了就会让负控在测试夹具的字面量上误绿）。

**顺带流量出的第二个零设施**（不在 #41 范围内，需要裁定，故只登记）：
`seed_variables.rs:899-948` 声明了 8 个「技术指标周期」可调变量（`macd_fast` / `macd_slow` / `macd_signal` /
`boll_period` / `boll_stddev` / `volume_lookback` / `volume_surge_ratio` / `volume_shrink_ratio`），
面板 `src/components/settings/StockAnalysisConfigPanel.tsx:581-594` 把它们渲染成一个可编辑分组（标题就叫 technical_indicators），
但**全仓零消费者**：没有任何节点把它们写进 `input_mapping`，`compute_scoring` 也不从 `arguments` 读这些名字
（读数：8 个名字在 `src-tauri/src` + `crates` 里除声明处以外命中 0 次）。⇒ 属本仓登记的「配置项空接线」族，
且**裁定要连着本片一起看**：一旦把它们接进 `compute_scoring`，就会与 `ScaleWindowPlan` 派生的窗口成两套权威
（档侧的计划赢还是面板赢，必须先定）。我这道新门查不到它 —— 它查的是「映射值必须有源」，反方向（「声明的变量必须被消费」）没有门。

**读数补记**（全部改动落定后按 CI 口径跑，同一棵树）：

| 命令 | 读数 |
|---|---|
| `__TAURI_WORKSPACE__=true cargo test --workspace --no-fail-fast` | `WS_EXIT=101`：**8250 passed / 1 failed**、144 个测试二进制；唯一红 = 上面那条既存门 |
| `cargo clippy --workspace --all-targets --all-features -- -D warnings` | `CL_EXIT=0`，日志里 `^warning` / `^error` 各 **0** 条（8m43s） |
| `cargo fmt --all --check` | `FMT_EXIT=0` |
| `npm run test:run` | `VITEST_EXIT=0`，**195 files / 2177 passed**（91.99s） |
| node 门复跑（`audit-inject-coverage --tier all --strict` / `--strict` / `check-tier-purity --selftest` / `check-tier-purity` / `check-single-source-facts --ci`） | **各 EXIT=0** |

⇒ 本批**唯一**没绿的是那条既存门的窗口终点（修法见 §一○四，已按裁定 1「改」落地）。


## 一○四、既存门 `stock_workflow_write_side` 的窗口定位器修好（裁定 1「改」，2026-10-08，**不发版**）

`src-tauri/tests/stock_workflow_write_side.rs:33-50`：窗口终点从「本函数之后还有别的顶层 `pub fn`」
改成「**本函数自己的列 0 收尾 `}`**」（rustfmt 对顶层 item 保证），并加一条越界自检 —— 窗口里再出现
`\npub async fn ` / `\npub fn ` 就红（吞掉兄弟 item 就是拿别人的字面量当本函数的落库形态）。

**修的时候又被这条门自己量出一个伪装**（旧判据根本走不到这一步，所以它从没暴露）：判据原本是
`win.contains("template_version: Set(Some(loaded.version))")`，而 `core.rs:2177` 有一条注释
逐字抄了同一个表达式（**没有尾逗号**）⇒ 负控「摘掉那一行」之后判据仍然为真。
改成只认**带尾逗号的写入语句**（常量 `PERSIST_WRITE`，:52-58），并锁「本函数里这样的写入点恰好 1 处」
（多一个 update 分支漏写代际 = 又一类「有结论、无代际」，先红着让人来数）。
⇒ 属本仓已登记的「注释里抄一遍常量名原本算已筛」那一族，这次是在 **Rust 侧的门**里复现，不是 node 门。

两条定位器自证（同文件末尾 :106-134）：① 本函数是文件最后一个顶层 item 时窗口照样定得了
（现网形态，也正是旧判据从没绿过的原因）；② 摘掉收尾 `}` ⇒ 必须 panic（负控拆在真正决定该断言的那道守卫上）。

读数：`__TAURI_WORKSPACE__=true cargo test -p axagent --test stock_workflow_write_side` →
**4 passed / 0 failed**（改前 1 failed）｜
`cargo clippy -p axagent --test stock_workflow_write_side --all-features -- -D warnings` → `CLIPPY_EXIT=0`（8m50s）｜
`cargo fmt -p axagent` 无 diff。


## 一○五、裁定 2「先加设施再接线」= **v137**：面板「技术指标」8 个变量从空接线变成真生效（2026-10-08）

### (0) 本批量到的缺陷与两域裁定

这 8 个变量此前**只有两处**：`seed_variables.rs:899-948` 的声明 + 面板一个可编辑分组；
`input_mapping` 里**零命中**（8 个名字在 `src-tauri/src` + `crates` 除声明处命中 0 次）⇒ 面板改值对评分零影响，
属本仓登记的「配置项空接线」族（同族先例：`value_dcf_*` 的 C2 路径 Z、`is_async` 的 B-2b 前置 1）。

裁定是「**先加设施再接线**」，而接线时新增了一条边界，两侧都会坏：
**漏接** ⇒ 回到空接线；**多接** ⇒ 把「几根 bar」接到四档 ⇒ 四档窗口重新焊成同一份（= 片 A 刚拆掉的缺陷）。
⇒ 拆成两域，且**两域的依据都是既有裁定而不是新偏好**：

| 域 | 成员 | 作用面 | 依据 |
|---|---|---|---|
| 窗口域 | `macd_fast` `macd_slow` `macd_signal` `boll_period` `volume_lookback` | **只日线链** `t-scoring` | 档侧「几根 bar」= `ScaleWindowPlan`（§九十六），面板再给就是第二权威 |
| 阈值域 | `boll_stddev` `volume_surge_ratio` `volume_shrink_ratio` | **全链**（日线 + 四档） | §九十六(2) 已裁「阈值不随尺度缩」（与 bar 日历跨度无关的有界量/比值） |

### (1) 设施（Rust，`crates/astock-data/src/mcp_tools.rs`）

| 位置 | 内容 |
|---|---|
| :3108-3127 | `INDICATOR_BAR_ARGS`（5）/`INDICATOR_RATIO_ARGS`（3）两张参数表；参数名统一 `ind_` 前缀、**不等于**变量名（同 `VALUATION_FLAT_ARGS` 那条「巧合会掩盖映射写错」的理由）。⚠ **落点（写进 `IndicatorConfig` 哪个字段）与键名在同一张表里**（`fn(&mut IndicatorConfig, usize)` 指针）：表在别处再 match 一遍就是两份权威 —— 加第六个键而忘加分支 ⇒ 那个参数被静默忽略 ⇒ 又是一次「面板调了没生效」；同表则「表里有」与「落得了地」是同一件事，生产路径上也就不需要 `unreachable!` |
| :3129-3170 | `panel_bar_arg` / `panel_ratio_arg`：缺失 ⇒ 不覆盖；非数值 / 非整数 / `<2` / 越界 / NaN ⇒ **显式失败**，**绝不回落默认值**（回落就是「调了没生效」换形态复发）。⚠ `Null` 也算「给了但坏」：`tool_executor.rs:75-78` 对解析不到的变量写的是 `Value::Null`，所以「用户在面板里把值清空」会走到报错分支而不是退回默认值 |
| :3171-3230 | `effective_indicator_config`：基准 = 日线 `default()` / 按档 `from(plan)`，再叠面板值；**档侧收到窗口域参数即报错**（点名参数与 `ScaleWindowPlan`）；两条结构判据 `macd_slow > macd_fast`、`volume_surge_ratio > volume_shrink_ratio`（倒置会「算得出数」但信号语义反了） |
| :1309 / :1336 | `compute_scoring` 换成这一条装配路径；日线由 `None` 改传 `Some(default)` —— `compute_indicators_with_config(.., None)` 内部本就 `unwrap_or(&default)`（`indicators.rs:221-222`）⇒ 逐字同路 |
| :460-471 | `compute_scoring` 的 `inputSchema` 补 8 条属性，**描述里写明各自作用面**（提示词/schema 层不留第二份真相） |
| :7408 | `mod panel_indicator_config_tests`（五条测试，见 (5)） |

### (2) 接线（种子，图内容变 ⇒ 换代 137）

- 主图 `t-scoring` 的 `extra` 从 `&[]` 变成 `INDICATOR_DAILY_FLAT_ARGS[..]`（8 条，`seed_stock_analysis.rs:4309-4327`）；
- 四张档模板的评分节点只接**阈值 3 条**，逐字写、不循环生成（`horizon_tier_template.rs:126-145`，理由同 §九十一(2.5)：按字面量配对的那些门读的就是这里的字面量）；
- 四条父扇出各补三条**同名身份映射**（档内工具节点读的是**子池**里的键；少一条不是「那条不生效」而是**整档子执行失败**，`subworkflow_executor.rs:97-105`；四块分别在 `seed_stock_analysis.rs:5887` / `5922` / `5976` / `6035` 附近）；
- `TEMPLATE_VERSION = 138`（`seed_stock_analysis.rs:1121`，v137 与 v138 两段变更史在 :1085-1118）；档模板版本常量绑在主版本上自动跟。

### (3) 新门 `scripts/check-indicator-config-scope.mjs`（接进 `ci-check.mjs` + `ci.yml` 各两条）

| 判据 | 内容 |
|---|---|
| P1 | 面板 `b("<名>", <默认值>,` 的手抄默认值 ≡ `IndicatorConfig::default()` 同名字段 ⇒ 「接线」在默认态**不改现网数值**；两份抽取任一取不到字段 ⇒ 报「判据没电」而不是「没有违规」 |
| P2 | 域归属**双向**：窗口域 5 条必须出现在主图、**不得**出现在档模板、**不得**被扇出身份映射带进子图；阈值域 3 条必须两边都在，且扇出身份映射**恰好 4 条** |
| P3 | 8 个变量名各自至少有一个 `("ind_<名>", "<名>")` 配对 ⇒ 「声明了但没人读」复发即红 |

自证 5 条负控：档模板塞窗口参数 / 面板默认值改一个 / 四条扇出少一条 / 主图摘掉一条窗口接入 /
Rust `impl Default` 整块取不到（「判据没电」一类）。
⚠ 扫描面是这 8 个**具名**变量（定向门）—— 「所有声明变量都得有消费者」的全域反向判据会把
「由 Rust 在播种期直接读走的变量」（`debate_max_rounds` / `kline_limit` 等数十处）误报成无源，故不做。

### (4) 面板呈现（`src/components/settings/StockAnalysisConfigPanel.tsx`）

旧的一组 8 项拆成两组：`technical_indicators_daily`（5 项，标题「仅日线链」）与
`technical_indicators_shared`（3 项，标题「全链生效」），各带一条 `note`（**必须放 vars 之后** ——
label 与 vars 之间插任何东西都会让整组不被该文件的分组解析器识别，2026-09-29 实测踩过）。
4 个新 i18n 键 × 11 语言（`group.indicatorsDailyWindows` / `group.indicatorsThresholds` /
`note.indicatorsDailyWindows` / `note.indicatorsThresholds`），旧键 `group.indicators` 已从 11 份里删除
（留着就是「面板不再渲染、翻译表还在维护」的第二份权威）。
vitest 侧新增一条：两组成员逐字对账 + note 键挂上 + 11 语言齐备 + 非拉丁语系整串仍 ASCII 即红 + 旧键残留即红。

### (5) 默认态零数值变化的两侧锁

| 侧 | 锁 |
|---|---|
| 代码 | `mcp_tools::panel_indicator_config_tests::empty_args_reproduce_the_pre_wiring_configs_bit_identically`（日线 ≡ `default()` 逐字段；四档 ≡ 纯 `from(plan)`） |
| 面板 | 上面 P1（面板手抄的 8 个默认值 ≡ Rust 默认） |

### (6) 读数

| 命令 | 读数 |
|---|---|
| `__TAURI_WORKSPACE__=true cargo test -p axagent-astock-data --lib panel_indicator_config` | **5 passed / 0 failed**（`empty_args…bit_identically` / `window_domain_applies_to_daily_only_and_fails_loudly_on_tiers` / `threshold_domain_applies_to_every_tier` / `bad_panel_values_fail_instead_of_falling_back` / `inverted_structure_fails`） |
| `__TAURI_WORKSPACE__=true cargo test -p axagent-astock-data --lib` | **557 passed / 0 failed / 8 ignored**（改前 552 ⇒ 本批 +5） |
| `__TAURI_WORKSPACE__=true cargo test -p axagent --lib stock_analysis_setup::` | **69 passed / 0 failed**（含裁定 3 那条被拦下后补好的 `branch_node_scope_is_exactly_the_seed_mapping`） |
| `cargo clippy -p axagent-astock-data --all-targets --all-features -- -D warnings` | **EXIT=0**（修完 `type_complexity` 之后） |
| `node scripts/check-indicator-config-scope.mjs` / `--selftest` / `--dump` | **EXIT=0 / 自证 5 条负控全红得住 / dump 显示 arg 配对各 1、阈值身份 4 条、窗口身份 0 条** |
| `npm run typecheck` | 0 ｜ `npx dprint check`（本批 4 个前端文件）0 diff |
| `npx vitest run StockAnalysisConfigPanel.test.tsx` | **15 passed / 0 failed**（含本批新增那条两域对账） |
| `npx vitest run DecisionBanner.test.tsx` | **16 passed / 0 failed**（含裁定 3 的渲染门） |
| i18n 三条门（`check-i18n-key-parity.mjs` / `check_i18n.py` / `check-i18n-placeholder-usage.mjs` / `check-i18n-untranslated.mjs`） | 全 **EXIT=0**；`check_i18n.py` 在我先写横幅后补 locale 的中间态**确实报过** `代码引用但 zh-CN 缺失: stockAnalysis.horizonScoringWindows` ⇒ 门有电 |

### (7) 本批自己踩到的一条流程缺陷（写下来是为了不再踩）

我在改完 `astock-data` 后只跑了 `cargo check` + 该 crate 的测试，**没有在阶段内跑定点 clippy**，
把 lint 交给末尾的全量兜底。全量因此报出 2 条 `clippy::type_complexity`
（新写的两张参数表把 `fn(&mut IndicatorConfig, usize)` 直接摊在 const 类型里）⇒
修法是按 clippy 的建议抽 `type IndicatorBarArg` / `IndicatorRatioArg` 别名，而不是加 `#[allow]`。
AGENTS.md 那条「阶段内用 `cargo clippy -p <crate> --all-targets -- -D warnings` 收敛」是**省时间的**：
全量 clippy 一轮 7–9 分钟，而定点 13 秒；更糟的是全量遇首个 error 会短路其后所有 crate。
⇒ 判据：**动了哪个 crate，就在该 crate 的阶段末尾跑一次定点 clippy**，全量只在最后兜底。



## 一○六、裁定 3「让用户看出各档实际几根」= **v138**：逐档决策行带上本档实际用的指标窗口（2026-10-08）

### (0) 这条要解决的不是「加一列」

v136 之后四档各自按尺度取数、按该档窗口计划出指标，但界面**只有分数没有几根 bar** ⇒
「这一档算得粗」与「这一档观点不同」在界面上同形。更具体的是 §一○三(3) 那条实测：
短/中/长三档的 MA 根数**按公式本就是同一组**（`[2,6]`），差别只在 bar 的日历跨度（2 周 / 2 月 / 2 季）
⇒ 不把**根数与尺度并列**印出来，读者与面板都无法区分「两档恒等」与「两档同数不同跨」。
这条属 AGENTS 的「结构性缺口不得在呈现层造成歧义」：缺口本来就存在（粒度粗），藏起来才是缺陷。

### (1) 产出层（图内容变 ⇒ 换代 138）

| 位置 | 内容 |
|---|---|
| `commands/stock_analysis_setup/horizon_tier_template.rs` 四张模板的分支 `input_mapping`（:204 / :279 / :358 / :450） | 各加两条：`("scoring_windows", "t-scoring-<周期>.result.content.indicators.windows")` + `("scoring_scale", "…result.content.period")`。**读的是子图内本档评分节点** ⇒ 父扇出一条都不用加（`audit_fanouts` 的 needs 不变，新增的三个键是阈值域那批，见 §一○五(2)） |
| `commands/portfolio-mgr-h-{ultra-short,short,mid,long}.rhai` 输出段 | `scoringWindows: if present(scoring_windows) { scoring_windows } else { () }` + `scoringScale: …`；四份头部「已接」清单同步点名（超短那份另写两条为什么） |
| 产端形态 | `indicators.windows` 是 `IndicatorWindows::from_cfg` 的**回显**（实际吃进去的那组），不是「想要的配置」⇒ 面板印的就是算过的那组 |

**读侧为什么不用改 Rust**：`analysis-engine::reflection_stats::HorizonDecision` 没有
`deny_unknown_fields` ⇒ 两个新键在反思读侧被自然忽略（那里不需要窗口，加字段反而是无消费者的声明）。

### (2) 呈现层

| 位置 | 内容 |
|---|---|
| `src/types/stock-analysis.ts` | 新增 `HorizonScoringWindows` 接口（7 个字段，与产端 serde camelCase 同名）+ `HorizonDecision.scoringWindows?` / `scoringScale?`，注释写清**缺席语义 = 该行早于 v138** |
| `src/lib/agentOutput.ts` | **不改** —— 逐档行是整体透传（`decisionsByHorizon` 走 cast 不重建，:411-413），新键天然活着；只有顶层字段才需要进白名单 |
| `src/components/stock-analysis/DecisionBanner.tsx` | 逐档 Tab 内、紧跟 `daily_fallback` 注脚之后，一条 `t("stockAnalysis.horizonScoringWindows", {scale,ma,rsi,macd,boll,vol})`；**只在有回显时渲染**（旧代行不出现该行，不补 0、不渲染空表） |
| 11 语言 | 新键 `stockAnalysis.horizonScoringWindows`，六个插值占位符（`{{scale}} {{ma}} {{rsi}} {{macd}} {{boll}} {{vol}}`）逐语言齐备 |

### (3) 锁

- `scripts/check-horizon-decision-keys.mjs`：`--dump` 实测四档产端键集 25/27/27/31 里**都含** `scoringWindows scoringScale`，
  TS 侧 37 条里含 `scoringWindows? scoringScale?` ⇒ R1（产端有 TS 未声明）与 R4（TS 声明了没人产）两头都有电，
  不是「门没看见」。
- `scripts/audit-inject-coverage.mjs --tier all --strict`：四档新读名都必须在同档映射块里有源（EXIT=0，见 (5)）。
- vitest `DecisionBanner.test.tsx`：断言形态用现成的 `t` 替身（插值对象渲染成 `key|k=v,…`）⇒
  直接锁「传进去的是**这一档**的 monthly / [2,6] / 2 / 2/3/2」，并锁旧代行整行不渲染。
- `scripts/check-tier-purity.mjs`：新映射只点名本档评分节点 ⇒ R2/R5 不受影响（EXIT=0）。
- **既有齐备态门自己把它拦下了**（这条是本批最有价值的读数）：`cargo test -p axagent --lib stock_analysis_setup::`
  首跑 **68 passed / 1 failed**，红的正是
  `seed_consistency_tests::branch_node_scope_is_exactly_the_seed_mapping`，报文：
  `档 ultra_short 有映射键在本门的值表里登记不到：["scoring_windows", "scoring_scale"] ⇒ 「齐备态」其实缺料，先补值表再改门`。
  那道门会**用真实节点 scope 跑四份分支脚本**，逐键要求「齐备态有料」⇒ 新加的映射若不登记，
  它测的就不是齐备输入。处置按报文说的走「补值表」而不是改门：
  ① `value_for` 登记 `scoring_windows`（对象，字段名 = `IndicatorWindows` 的 serde camelCase 输出）；
  ② `scoring_scale` 走 `run_with` 里的**逐档**分支（值 = 本档权威 `Period::scale_key()`，不是固定串）；
  ③ 齐备态新增两条断言：回写的尺度必须 ≡ 本档权威尺度（脚本硬编码某档 ⇒ 另三档红），
  且 `scoringWindows.maPeriods` 必须非空（Rhai 不能枚举 map ⇒ 键名写错就是**静默缺席**，
  所以必须按形状断，不能只看跑不跑，§四十九 那条限制）。


### (4) 界面现在是什么

四档 Tab 各多一行「本档技术腿窗口：monthly · MA 2/6 · RSI 2 · MACD 2/3/2 · 布林 2 · 量能 2」，
尺度与根数并列 ⇒ 短/中/长「同根数不同跨度」变成可读事实而不是隐藏推断。
存量行（v138 之前落库的历史决策）没有该字段 ⇒ 那一行**不出现**，界面不会因此显示 0 或空表；
这需要实机核对（与 #48 同批）。

### (5) 读数（本批）

| 命令 | 读数 |
|---|---|
| `__TAURI_WORKSPACE__=true cargo test -p axagent --lib stock_analysis_setup::` | **69 passed / 0 failed**（先 68/1 被齐备态门拦下 ⇒ 补值表后转绿，见 (3)） |
| `node scripts/check-horizon-decision-keys.mjs` + `--selftest` + `--dump` | **EXIT=0**；`--dump` 点名四档产端键集 25/27/27/31 各含 `scoringWindows scoringScale`，TS 37 条含两个可选键 ⇒ 覆盖度是实测出来的，不是推定 |
| `node scripts/audit-inject-coverage.mjs --tier all --strict` | **EXIT=0**（四档新读名都有映射来源） |
| `node scripts/check-tier-purity.mjs` / `--selftest` / `check-rhai-numeric-typing` / `check-analyst-id-shape` / `check-generation-scope-registry` / `check-horizon-weight-parity` / `check-contracts` / `check-layer-discipline` / `check-rust-raw-map-err` / `check-single-source-facts --ci` | **各 EXIT=0** |
| `npx vitest run DecisionBanner.test.tsx` | **16 passed / 0 failed**（含本批新增的逐档渲染门：旧代行整行不出现、有回显时参数逐字对账） |
| `npm run typecheck` / `dprint check`（4 个前端文件） | 0 / 0 diff |
| i18n 四条门 | 全 0（11 语言键集一致、占位符干净、无新增未翻译） |
| `cargo test --workspace --no-fail-fast` / `cargo clippy --workspace --all-targets --all-features` / `cargo fmt --all --check` / 全量 vitest | 见本节末「(6) 收尾」 |

### (6) 收尾（三条 CI 口径 + 全量 vitest，同一棵树）

| 命令 | 读数 |
|---|---|
| `cargo clippy --workspace --all-targets --all-features -- -D warnings` | **CL_EXIT=0** |
| `cargo fmt --all --check` | **FMT_EXIT=0** |
| `npm run test:run` | **VT_EXIT=0**，**195 files / 2179 passed**（89.99s，比改前 +2 = 本批两条新前端门） |
| `__TAURI_WORKSPACE__=true cargo test --workspace --no-fail-fast` | **WS_EXIT=0**，144 个测试二进制、**8258 passed / 0 failed** |

⚠ 一条我自己的操作缺陷（不是代码缺陷，但会影响读数可信度）：第一次把这三条串起来跑时，
我在链中途 `cd` 到了仓库根目录，而 `cargo test --workspace` 需要在 `src-tauri/` 下 ⇒
该 stage 直接 `error: could not find Cargo.toml`、`WS_EXIT=101`，**一条测试都没跑**。
上面那行 8258/0 是回 `src-tauri/` 重跑的。⇒ 判据同「门禁命令的覆盖面本身就是门禁的一部分」：
**退出码非 0 时先读日志第一行确认它到底有没有开始干活**，别把「没跑」读成「跑挂了」。

⇒ 本批（裁定 1 + 裁定 2 + 裁定 3）落地后 **CI 口径三条全绿**，没有剩余红灯。


## 一○七、**全清单对账**（2026-10-08，按证据逐条改状态；§七十九 那张表的当前读数）

> §七十九 是 2026-10-05 的接手快照 ⇒ **不删不改**，本节只按现场读数重述状态。
> 每条都给可复核的证据（`文件:行` 或门读数），不给散文结论。

| 条 | 状态 | 证据 |
|---|---|---|
| A1 = #44（B2-2/B3，v133） | **已完成并提交** | `507a4af69`「portfolio-mgr 按档拆分」；23 个逐档分析师实例 + `analyst_node_id` 双语言镜像 |
| A2 = #49（验证报告代际筛样） | **已完成** | 走 join `reco_picks.reco_version` + `RECO_GENERATION_FLOOR=1`（分域），写侧不筛，登记门里该条已在 `statistics` 桶 |
| A3 = #7 ②（错峰 vs 改拓扑） | **已被事实消解，无需再拍** | B-2b 本体把四个评分节点搬进各自子模板 ⇒ 父图不存在评分链；`scripts/check-tier-purity.mjs:174` 的 `DECLARED_SCORING_STAGGER = []` 就是「四路无评分错峰」的登记，R3 改判成「主图与档模板都不得有档间评分边」 |
| A3 = #7 ④（档内不读他档门） | **已存在** | `check-tier-purity.mjs` 的 R2（他档 token 扫描，含「最长命中」「剥注释」两类负控） |
| A4 = #10（妖股标签） | **已完成**（含原「待拍」那一问的自行裁定） | 列落 `crates/entities/src/stock_reflections.rs:44`（**按档**挂逐档反思行）；唯一判据 `crates/analysis-engine/src/mover_recall.rs:44` 复用 `DEFAULT_GAIN_THRESHOLDS`（没写第二份字面量）；DTO `src/commands/stock_analysis.rs:5528`；面板列 `src/components/stock-analysis/ReflectionPanel.tsx:60`；四态测试 `mover_recall.rs:863` |
| A5 = #8（数据质量跨轮熔断，v134） | **代码已完成**，实机核对等 C1 | 新实体 `data_quality_observations` + `dao/repo/data_quality_fuse.rs` + Switch `dqi-fuse-gate` + `DecisionTrustNotice` |
| B #7 ① | **已实施**（真改图 SubWorkflow×4） | §一○一（v135/136），四张 `stock-horizon-<档>` 模板已播种路径与版本门齐 |
| B #18（宏观换官方源） | **已拍板、未实施** | 现场 grep `stats.gov.cn` / 新源客户端 ⇒ 零命中；`macro_data.rs` 仍只有 CPI/PPI/PMI/GDP 四条真序列 |
| B #20 ①②（龙虎榜原生 as-of / 归档升格历史表） | **已拍板、未实施** | 同上，`daily_snapshot.rs` 仍是 DiskCache JSON 形态，无新表 |
| B #11（三项吞吐改造） | **未开始**（前置是 C1 的耗时读数） | 无落点代码 |
| C1 = #48（PG 冷页读挂） | **未解锁** ⇒ 连带 **v129–v138 全部实机核对未做** | 需批准重启 `postgresql-x64-18`；正门 `node scripts/pg-connect.mjs sql "select count(*) from workflow_executions"` |
| C2 = #12（荐股 IC 按代筛样） | **前置未满足** | 登记门 `--dump` 现场读数：仍剩 2 项 `pending` = `reco_ic_stats`（`src/commands/stock_analysis.rs`）与 `load_reco_loop_samples`（`analysis-engine/src/recommender/mod.rs`）⇒ 读侧没筛。上次现网读数是 `reco_version` 盖章 0 行（需重启 PG 后**重取**再判，别拿旧数当现状） |
| #41 门（§九十二(4) 那条 ToolNode 参数必须有源） | **已完成** | `horizon_tier_fanout_tests::tool_node_argument_sources_exist` + 两侧负控 |
| #41 片 A / 片 B（v136） | **已完成** | §一○三 |
| #41 呈现片（v138） | **部分完成 —— 范围比 §一○二(4) 原文窄** | 已做：逐档 `scoringWindows` + `scoringScale` 一行（尺度与根数并列）；**未做**：`scaleTrend{fastBars,slowBars,fast,slow,diffPct,fastSlope}` 与 `scaleMomentum{period,value}` 的**数值四表**（现在只有 RSI 那一个数进决策行）。原文里「分析师 prompt」那一支经核查是**空集**（md 资产与 Rust 提示词串里无 RSI/命名槽引用） |
| §一○一(1) 记的粒度偏离（风险节点留父图） | **未收口，且不在任何 # 项里** | 四份 `portfolio-mgr-h-*.rhai` 里没有「本档风险档」出口键（grep `riskTier`/`risk_tier` 零命中）⇒ 要按原裁定做成三节点形状，前置仍是「先给 `h_<档>` 加携带风险档的出口键 + 主链读端」 |

> 本表是 2026-10-08 当时的快照，**后两行的「未做」已被同日晚些的 §一○八（v139 呈现层数值上屏）与 §一○九（v140 三节点形状收口 + `riskCategory`）闭合**；
> 原文保留不改，避免把快照改成事后叙述。全域对账的当前读数见 §一一二 末尾的三桶。

### 本轮新量出、需要一句话裁定的两条

1. **全域反向门要不要建**：`check-indicator-config-scope.mjs` 只对**这 8 个具名变量**断言「声明必须有消费者」。
   「任何种子变量都必须有消费者」的全域版**没建** —— 它会把「由 Rust 在播种期直接读走」的那批
   （`debate_max_rounds` / `kline_limit` 等数十处、形态不一）误报成无源。
   可选形状：登记制（变量 → 消费种类 + 定位串，门按内容验证，照 `check-generation-scope-registry.mjs` 的形态）。
2. **面板窗口域要不要也作用到档侧**：现在的裁定是「档侧窗口归 `ScaleWindowPlan`，接到档上工具显式拒绝」。
   若产品希望面板能整体压过计划（例如统一调 MA 根数），就得先定「计划派生 vs 面板值」的次序，
   而不是让两处各自生效一半（那会重演四档恒等）。另：日线窗口组的说明文案目前只写了
   「放量倍数必须大于缩量倍数」这一条结构约束，**没写** `macd_slow > macd_fast` 同样会报错 —— 要不要补进 11 语言文案。


## 一○八、**v139**：呈现层补齐 —— 逐档两带与动量的**数值**上屏（2026-10-08）

### (0) 为什么这一条还欠着

§一○二(4) 给片 B 的原文是「四份 `portfolio-mgr-h-*.rhai`、分析师 prompt、面板四表、11 语言改读
**windows / scaleTrend / scaleMomentum**」三支。v138 只落到第一支（根数 + 尺度）
⇒ 界面能看到「这一档按几根算」，但看不到「这一档两带差多少、动量具体多少」。
§一○七 对账时把它记成「**部分完成 —— 范围比原文窄**」而不是「已完成」，本节补那一段。
原文里的「分析师 prompt」那一支经核查是**空集**（md 资产与 Rust 提示词串不含 RSI/命名槽引用），
不是漏做 —— 空集也要写出来，否则下一个人会以为还有一处待改。

### (1) 产出层（图内容变 ⇒ 换代 139）

| 位置 | 内容 |
|---|---|
| `horizon_tier_template.rs` 四张模板分支 `input_mapping` | 各加两条：`("scale_trend", "t-scoring-<周期>.result.content.indicators.scaleTrend")` + `("scale_momentum", "…indicators.scaleMomentum")`；仍**读子图内本档节点** ⇒ 父扇出不加键 |
| 四份 `portfolio-mgr-h-*.rhai` 输出段 | `scoringTrend: if present(scale_trend) { scale_trend } else { () }` + `scoringMomentum: …`；四份头部「已接」清单点名（超短那份写两条为什么） |
| `seed_consistency_tests.rs` 齐备态夹具 | 两个新键在 `run_with` 里按 **`tier_idx` 给不同数值**（退化态给 UNIT）；新增三条**数值**断言：慢带根数、两带差、动量值必须等于本档那份 ⇒ 脚本硬编码或串档时另三档当场红 |
| 取值器 `dyn_num` | 同时收 i64 与 f64 —— 本条锁的是「逐档透传」而不是数值类型（类型由 `check-rhai-numeric-typing.mjs` 与 v138 的 windows 断言各自覆盖）；**放宽是有意的，注释里写清**，免得被读成「哪儿都能松」 |

### (2) 呈现层

- `src/types/stock-analysis.ts`：新增 `HorizonScaleTrend`（`diffPct` / `fastSlope` 类型是 `number | null`）
  与 `HorizonScaleMomentum`，`HorizonDecision` 加 `scoringTrend?` / `scoringMomentum?`；
- `DecisionBanner.tsx`：逐档 Tab 里紧跟窗口那一行再印两行；
  **`null` 印「—」而不是 0** —— 产端 `Option<f64>` 不跳过序列化 ⇒ 缺席落成 JSON `null`，
  慢带非正是「算不出」而不是「差 0%」（AGENTS「结构性缺口不得在呈现层造成歧义」）；
- 11 语言新增 `stockAnalysis.horizonScaleTrend`（6 个占位符）与 `horizonScaleMomentum`（2 个）。

### (3) 验证与读数

| 命令 | 读数 |
|---|---|
| `npm run typecheck` | **EXIT=0** |
| `npx vitest run DecisionBanner.test.tsx` | **16 passed / 0 failed** —— 含新加的「mid 的 `diffPct`/`fastSlope` 为 null ⇒ 渲染 `—`」与「点 short 档 ⇒ 数值跟着换（`9.75` / `2.63` / `51.5`）」 |
| `node scripts/check-horizon-decision-keys.mjs` + `--dump` | **EXIT=0**；dump 里四档产端键集与 TS 声明都出现 `scoringTrend scoringMomentum`（覆盖度是打出来的，不是推定） |
| `audit-inject-coverage --tier all --strict` / `check-tier-purity` / `check-rhai-numeric-typing` / `check-indicator-config-scope` / `check-input-mapping` / `facts --ci` / `line-endings` | **各 EXIT=0** |
| i18n 四条门 + 逐语言回读断言 | 全 0；两键齐备、6+2 占位符完整、非拉丁语系含本语言字符、**没有任何语言与 en-US 逐字相同**、无汉字串扰、无 CRLF |
| `cargo test -p axagent --lib stock_analysis_setup::` / CI 口径三条 / 全量 vitest | 见 (4) |

⚠ 一条**我自己写的验证判据被否证**：locale 回读初版用「串里必须出现非 ASCII」当「已本地化」的代理，
de/es/fr 的动量短句 legitimately 全是 ASCII ⇒ 三条假红。换成可检的形态：非拉丁语系要求含本语言字符
（ja 假名 / ko 谚文 / ru 西里尔 / ar / hi / zh），拉丁语系改为**「不得与 en-US 逐字相同」**——
「有没有非 ASCII」判不了译文质量，「是不是抄的英文」判得了。

### (4) 读数补记

| 命令 | 读数 |
|---|---|
| `__TAURI_WORKSPACE__=true cargo test -p axagent --lib stock_analysis_setup::` | **69 passed / 0 failed**（`SETUP_EXIT=0`） |
| `cargo fmt --all` → `--check` | 无 diff |

⚠ 首跑是**编译失败**而不是测试红：`error[E0599] no method named as_f64 found for &rhai::Dynamic`
—— 我在断言里把 `serde_json::Value` 的 API 当成 Rhai 的用了。改成
`v.clone().try_cast::<f64>().or_else(|| v.clone().try_cast::<i64>().map(|i| i as f64))`。
⇒ 跨层写取值器时先核对被调类型的 impl，别把两套 Value API 互换。


## 一○九、**v140**：三节点形状收口 —— 风险节点搬进子模板 + `riskCategory` 出口键（2026-10-08）

### (0) 补的是哪一条偏离

§一○一(1) 登记的偏离：粒度裁定要「评分 + 按档风险 + 分支决策」三段都进子模板，v135 只搬了两段，
`cls-risk-level-<档>` 留在父图。留下的原因是普查到 `portfolio-mgr` 有**四个父侧读面**
（`overall_risk_{ultra_short,short,mid,long}`），而子执行只把扇出节点的 `node_id`/`output_var`
**双键**写回父池（`work_engine/engine/mod.rs:1791-1796`）⇒ 直接搬走那四条映射就指向不存在的路径
⇒ `present()` 恒假 ⇒ **v128 B1 的按档风险收紧整条静默退役**（「配置项空接线」族）。
⇒ 收口的前提正是「先给 `h_<档>` 加一个携带风险档的出口键」，本批就是它。

### (1) 四处一起改（缺一处就是那次静默退役的复现）

| # | 位置 | 内容 |
|---|---|---|
| ① | `horizon_tier_template.rs` | 新增 `tier_risk_node`，四张模板各插一个节点 ⇒ 形状 **4→5 节点 / 3→4 边**；九条输入映射与父图旧形态**逐字相同**（含 `riskWindows.<camel档>` 两键，四份逐字、不 `format!` 拼，理由同 §九十一(2.5)）；子图内加边 `cls-risk-level-<档> → pm-h-<档>` |
| ② | `seed_stock_analysis.rs` | 四条扇出的身份键 `cls-risk-level-<档>` → **`t-risk`**（风险节点吃的仍是父侧 `t-risk` 产出）+ 供给边同步；**删**四个 CodeNode（`-213 行`）与四条 `cls-risk-level-<档> → portfolio-mgr` 边 |
| ③ | 四份 `portfolio-mgr-h-*.rhai` | `riskCategory: if present(overall_risk) { overall_risk } else { () },` ⇒ 双键写回把风险档带回父池 |
| ④ | 主链读面 + TS | 四条 `overall_risk_<档>` 源路径 → `pm-h-<档>.result.riskCategory`；`HorizonDecision.riskCategory?`（keys 门 R1 要求产端与声明同批） |

全局节点 `cls-risk-level` 与其供给边**不动**（research-mgr 上下文与主链回退分支要的是 60 日整票档），
门第 ⑦ 条继续锁「不得给它注入按档两键」。

### (2) 「语义逐位不变」的两句证明

- **数值**：同一份 `risk-level.rhai` + 同样九条输入路径 ⇒ 按档风险档不变；
- **失败面**：子模板里风险节点 `continue_on_fail = false`。父图旧形态是 `true`，但它失败时产出
  不进父池 ⇒ 扇出严格 `map_inputs` 同样判**整档子执行失败**。照抄 `true` 进子图的话，失败就退化成
  「分支少读一个 `overall_risk`」⇒ **趁搬图把「该档失败」放宽成「该档少一腿」**。
  要真放宽须另立裁定并点名 `portfolio-mgr.rhai::tier_risk_raise` 这个消费者。

### (3) 门跟改：`check_horizon_scoped_risk` 换形 + 五条负控

判据 ①②③ 从「父图 JSON 的节点」搬到 **typed 子模板节点**（`config.input_mapping` / `config.code` / 子图边）；
④⑤⑦ 留父图；⑥ 新增 **absence 断言**：父图不得再有任何 `cls-risk-level-<档>` 节点或以它为端点的边
（留着就是两份权威，父图那份照样跑出一个没人读的风险档）。

⚠ 关键的一处是签名加了 `child_of: impl Fn(Period) -> (Vec<WorkflowNode>, Vec<WorkflowEdge>)`：
判据面搬到子模板之后，老负控只能改父图 JSON ⇒ **打不到判据** —— 那不是「门还红着」而是
「门换了个姿势假绿」。改成注入式后五条各打一条独立失效面：
① mid 子模板读 long 那一格；①′ 抽子图内「风险 → 分支」边；② 撤扇出的 `t-risk` 键；
③ 撤父侧 `t-risk → pm-h-mid` 边；④ 主链读端改回旧路径。

`tier_template_shape_is_the_declared_four_nodes` → `..._five_nodes`，风险节点 id 由 `Period::as_str()` 现推。

### (4) 界面与运行期

- 面板**无可视变化**（`riskCategory` 不进面板，只进主链的风险收紧）；
- 进度面板里四个风险节点的事件现在带子执行来源（B-2a 的接缝），
  `src/stores/feature/stockAnalysisStore.ts:3009-3012` 的阶段名单按 **id** 匹配 ⇒ 预期仍落阶段 4，
  **这条必须实机核对**（与 #48 同批）——「少一格」在那份名单的注释里就是本仓登记的静默失效形态。
- 换代到 v140 ⇒ 重启重播种后核对四档风险档与主链收紧逐位不变。

### (5) 读数

| 命令 | 读数 |
|---|---|
| node 门八条（tier-purity / `audit-inject-coverage --tier all --strict` / `check-horizon-decision-keys` / input-mapping / analyst-id-shape / indicator-config-scope / horizon-weight-parity / `facts --ci`） | **各 EXIT=0**（图搬完之后跑）；keys 门 `--dump` 实证 `riskCategory` 出现 **5** 次（四档产端 + TS） |
| `npm run typecheck` | EXIT=0 |
| `npx vitest run DecisionBanner.test.tsx StockAnalysisConfigPanel.test.tsx` | **31 passed / 0 failed** |
| `__TAURI_WORKSPACE__=true cargo test -p axagent --lib stock_analysis_setup::` | **69 passed / 0 failed**（`SETUP_EXIT=0`）—— 含换形后的 `check_horizon_scoped_risk` 与**五条**负控（原三条） |
| `__TAURI_WORKSPACE__=true cargo test -p axagent --lib horizon_tier -- --nocapture` | **12 passed / 0 failed**；`--nocapture` 现场读数：四张模板均 **节点 5 / 边 4**（`const-scoring-period → t-scoring-<周期> → cls-risk-level-<档> → pm-h-<档> → end`），扇出 needs 已由 `cls-risk-level-<档>` 换成 **`t-risk`**，键数 10 / 13 / 15 / 13（v137 的 +3 阈值与 v140 的 ±0 都在里面） |
| CI 口径三条 + 全量 vitest | 见 (6) |

### (6) 读数补记

| 命令 | 读数 |
|---|---|
| `cargo clippy --workspace --all-targets --all-features -- -D warnings` | **CL_EXIT=0**（第一次跑报 1 条 `clippy::useless_conversion` —— 我给已是 `String` 的 `format!` 结果又加了 `.into()`，按建议删掉而不是加 `#[allow]`） |
| `cargo fmt --all --check` | **FMT_EXIT=0** |
| `npm run test:run` | **VT_EXIT=0**，**195 files / 2179 passed** |
| `__TAURI_WORKSPACE__=true cargo test --workspace --no-fail-fast` | **144 个二进制 / 8257 passed / 1 failed**；唯一红 = `analysis-engine::recommender::tests::cache_live_and_replay_are_isolated`，**单条 1/0 绿、整只 `--lib` 612/0 绿** ⇒ 跨二进制并跑才现形的全局荐股缓存 flaky（与 `cache_invalidate_works` 同族，已进清单）；本批没碰荐股缓存键 |

⚠ 一次环境型红，别当代码缺陷：全量里
`axagent-tools --lib win_sandbox::tests::network_block_status_is_truthful` 报红
（`crates/tools/src/win_sandbox.rs:1512` 断言「`Unavailable` 档下网络**确实放行**」——
那是已登记的已知限制，机器上此刻真有别的东西断了沙箱进程网络 ⇒ 断言方向反了）。
**单跑 1 passed / 0 failed** ⇒ 与并行/环境态相关，已进「门禁已知 flaky 测试」清单；
处置是不改断言方向、只点名。

## 一一○、#18（B-5）前置实测完成：官方 API 不可达，但**真发布日可达**（2026-10-08，只读、未动码）

§八十四 给 B-5 定的前置是「先实测两件事（本机可达性、按 reportName/指标码能否取到**带真发布日**的历史序列），
再决定做成 vendor」。今天重跑的读数列在下表（`curl --max-time 20`，全部现场取，不引早间旧数）。

### (1) 读数

| 请求 | 结果 |
|---|---|
| `data.stats.gov.cn/easyquery.htm?m=QueryData&dbcode=hgyf&rowcode=zb&colcode=sj&wds=[]&kwcode=[]&dfcode=csnd` | **403**（`0.277s`） |
| 同上 + `User-Agent: Mozilla/5.0…` + `X-Requested-With: XMLHttpRequest` + `Referer: https://data.stats.gov.cn/` | **403** ⇒ 不是缺请求头 |
| `data.stats.gov.cn/api/` | 404 |
| `data.stats.gov.cn/dbyjs.htm`（数据一览表/元数据） | 404 |
| `data.stats.gov.cn/` | 302 |
| `www.stats.gov.cn/sj/zxfb/`（最新发布列表） | **200**，73,013 字节 |
| `www.stats.gov.cn/sj/zxfb/index_2.html` / `index_5.html` | **200**（73,111 字节），条目发布日跨 **2023-02-17 → 2026-05-18** |

列表页原文样本（逐字，一条 CPI）：

```html
<a class="fl pc_1600" href="./202609/t20260909_1965263.html" target="_blank" title='2026年8月份居民消费价格同比上涨0.8%'
```

⇒ 文件名里的 `t20260909` 就是**真发布日 2026-09-09**，`title` 同时带**参照月（2026 年 8 月）**与头条值；
分页可达、无需鉴权、无需 JS ⇒ 「带真发布日的历史序列」这一问的答案是**能拿到**，但**拿的通道不是 easyquery**。
（`grep -c 居民消费价格` 在该页 = 6 处，两条 DOM 变体各一份。）

### (2) 裁定（B-5 换形，不是收工）

1. ❌ 原方案的「换成官方**数值**序列」（`easyquery.htm` + `reportName`/指标码）⇒ 判为**等外部条件**：
   403 是鉴权/WAF 型，需要统计局开放平台的注册凭据，本机无凭据。
   **不做**「拿推断值装成官方源」的伪接线（那是本仓登记的「伪装成正常结果」形态）。
2. ✅ 但 #18 的实质诉求（现网 as-of 用的是**推断发布日**：cpi/ppi 次月 15、pmi 月末+2、gdp 次月 20）
   有官方替代来源 ⇒ 改做成**发布日历侧信道**：`指标 × 参照月 → 真发布日`，
   只替换 `macro_available_at` 的**发布日**，不替换数值。
3. ⚠ **不做成 vendor**（这是对本节「再决定做成 vendor」的回答）：vendor 化要动 8 处登记面
   （`lib.rs` 的 `VendorRouting`/装配/`default_routing`/`register_vendor`/`gate.rs CAPACITIES`/
   `vendors/mod.rs`/`seed_variables.rs`/`recommender/pool.rs KNOWN_VENDORS` + `DataVendorsTab.tsx`），
   而它的语义是「一条完整取数通道」。日历只给现有 `t-macro-data` 的 as-of 判据补一个字段 ⇒
   真做成 vendor 会把「**数值仍来自东财、发布日来自 NBS**」这件事藏进第二个 vendor 的名字里，
   界面反而更难点名来源。⇒ 落点选 `crates/astock-data/src/macro_data.rs` 内新增日历函数 +
   在 `source:` 里标注 basis（那条现在就在写推断口径，同一段）。

### (3) 下一批的执行规格（本节不实施，避免「helper 进树无人调用」那种假闭环）

1. 抓取：`sj/zxfb/index_{n}.html` 分页（`index_5` 已跨 3 年 ⇒ 覆盖 CPI/PPI/PMI/GDP 两年历史约 5~8 页）；
2. 解析判据：文件名 `t(\d{8})_\d+\.html` 的 8 位日期 = 发布日；
   标题 `(\d{4})年(\d{1,2})月份(居民消费价格|工业生产者出厂价格|制造业采购经理指数|国内生产总值)`
   定指标与参照月 ⇒ 与 `SNAPSHOT_METHODS` 无关，与 `pmi` 那条消费键（`seed` 的长档 + `leg_signal.rs` 荣枯线）对齐；
3. 失败面：日历取不到 ⇒ **回落现有推断规则并在 `source:` 点名「发布日为推断值」**，
   两种 basis 不得静默混用（这条与 `macro_available_at` 现有「把推断写进 source 供界面点名」是同形）；
4. 同批改：`pick_published` 的判据 + `macroPartialSeriesOnly` 文案
   （`holding_period.rs:177`，**中文硬编码在 Rust 不在 i18n** —— 本身是待收编项，改它时一并搬）；
5. 测试形态：真网络测试属已知 flaky 族 ⇒ 判据跑在**离线夹具**上（本次抓到的 73 KB 列表页就是夹具源），
   网络侧只留「取不到即回落并标 basis」的负控。

## 一一一、待拍板 2 已落：日线窗口组文案补上 MACD 结构约束（2026-10-08，**不发版**）

### (1) 改动

11 语言 `stockAnalysis.settings.note.indicatorsDailyWindows` 各追加一句，内容是 Rust 侧已经生效、
但面板没说的那条结构判据（`mcp_tools.rs` 的 `cfg.macd_slow <= cfg.macd_fast ⇒ Err`）。
原文只在阈值组写了「放量倍数必须大于缩量倍数」，窗口组没写慢线/快线这条 ⇒ 用户改相等值会撞到
一个界面上没预告的报错。

zh-CN 追加原文：`MACD 慢线必须大于快线，否则工具显式报错——两者相等或倒置会让金叉/死叉反向。`
（其余 10 语同义，逐语言追加长度 41~170 字，由脚本断言「原文命中恰 1 次 + 回读末尾匹配 + 无零宽字符」。）

### (2) 读数

| 命令 | 读数 |
|---|---|
| `node .workbuddy/tmp/append-macd-note.mjs` | 11 语全部 `✅ … 追加 N 字，回读通过` |
| `npx dprint check src/i18n/locales/*.json` | **DPRINT_EXIT=0** |
| `node scripts/check-i18n-key-parity.mjs` | 11 个语言文件 key 路径集合一致 |
| `node scripts/check-i18n-placeholder-usage.mjs` | **PH_EXIT=0**，「占位符使用干净」 |
| `node scripts/check-i18n-untranslated.mjs` | **UNTR_EXIT=0**，无新增未翻译值 |
| `PYTHONIOENCODING=utf-8 python scripts/check_i18n.py` | **PY_EXIT=0**，「全部通过」 |
| `npx vitest run src/components/settings/StockAnalysisConfigPanel.test.tsx` | **15 passed / 0 failed** |

### (3) 该问的另一半闭合为「次序已定，不改」

「面板窗口值要不要压过 `ScaleWindowPlan`」⇒ **不改**。现判据是「档侧窗口由持有期推导，接到档侧工具显式报错」，
它正是 v135 之后「四档 `tier_score` 来源互不相同」这条（`check-tier-purity` R5）的前提；
改成「面板整体压过计划」等于允许用户把四档重新焊回同一份窗口 —— 就是 §一○五 记的那次回归的成因。
⇒ 待拍板 2 的两个问点分别落为：**文案补齐（已做）**、**权威次序＝计划派生 > 面板值，面板只作用日线链（维持）**。

## 一一二、待拍板 1 已落：全域「声明必须有引用面」门建成（**不发版**，2026-10-08）

### (1) 先推翻我自己的预测

§一○七 写的是「全域反向判据会把『由 Rust 在播种期直接读走的变量』误报成无源，那类消费点数十处且形态不一」。
实测：**判据不成立** ⇒ 按「非声明面引用面」口径（剥注释后的 `.rs` 引号形态 snake/camel + `.rhai` 裸标识符；
剔除 `seed_variables.rs` 与 `StockAnalysisConfigPanel.tsx` 两份声明面、再剔 `mod.rs` 的 `const RENAME_MAP` 别名块），
**374 个声明变量里只有 36 条零引用面**，且逐条可判真伪。⇒ 结论从「不建，怕误报」翻成「**建，面积可控**」。

### (2) 新门

`scripts/check-variable-consumer-registry.mjs`（登记制，照 `check-generation-scope-registry.mjs` 的形）：

- **G1** 每个声明变量：引用面 ≥1，或在 `EXEMPT` 登记 ⇒ 新空接线当场红；
- **G2** 登记项**已**有引用面 ⇒ 红（接线完成必须摘掉豁免，否则门失去真实面积读数）；
- **G3** 登记项已不在声明面 ⇒ 红（变量删了、豁免还留着＝假账）；
- **G4** `status ∈ {intentional, pending}`，`intentional` 必须给 `cite`（书面裁定出处）；
- **G5** 声明数 < 200 或扫描文件数 < 1500 ⇒ 判**红并退出码 2**（「判据没电」≠「全绿」）。
- ⚠ 这是**必要条件**判据：「有引用面」只证明该名字在非声明面被写出来过，不证明它真被读进执行路径
  ⇒ 本门是**面积上限**门；逐字两侧对账仍由定向门 `check-indicator-config-scope.mjs` 负责。

### (3) 读数（现场取，非计划）

| 命令 | 读数 |
|---|---|
| `node scripts/check-variable-consumer-registry.mjs` | **EXIT=0**：`声明 374 ｜ 零引用面 36（已登记 36）｜ 其中面板仍渲染 34 条`，`intentional 1 条、pending 35 条`（10.5s） |
| 同前 `--selftest` | **负控 9/9 通过**，ST_EXIT=0（G1×1、G2×1、G3×1、G4×2、G5×2 + 剥注释器/别名表两判据） |
| `node scripts/check-line-endings.mjs` | **EOL_EXIT=0** PASS |
| 接线 | `scripts/ci-check.mjs`（指标组门之后，两条 step）+ `.github/workflows/ci.yml`（新增 `Check variable consumer registry` step）两侧逐字同源 |

首跑的两条**自错**（记下来，属老毛病）：负控的期望串我按「`G1 brand_new_dead_var`」这种简称抄，真实文案是
「`G1 新增空接线：变量 brand_new_dead_var …`」；比较又写成 `got.includes(needle)` —— 对**数组**是整元素相等，不是子串
⇒ 七条负控全被判「没红」。改成 `got.some((p) => p.includes(needle))` 并逐条对齐真实文案后才 9/9。

### (4) 36 条里最重的发现（**这批的新信息，不是新代码**）

**其中 34 条仍被设置面板渲染成可编辑控件** —— `pos_max_single_pct` / `pos_max_total` / `pos_max_sector_pct`、
`risk_hhi_medium` / `risk_divers_medium` / `risk_divers_high`、`val_pe_low|high` / `val_pb_low|high`、
`val_{ultra_short,short,long}_kline_limit`、`rule_auto_stop_loss_pct` / `rule_bias_limit_pct` /
`rule_bear_low_score` / `rule_volume_signal_block`、`value_moat_threshold` / `value_safety_margin`、
`rev_divergence_lookback`、`news_limit`、`ref_*_code` ×7、`monitor_*` ×3、
以及 RSI 两组：`signal_rsi_*`(30/70) 与 `rule_rsi_*`(20/80) —— 而真正生效的阈值在 Rust 常量里
（`scoring.rs` 的 `ScoringBands` 四带 ladder、`indicators.rs` 的 `IndicatorConfig` 80/20）。
⇒ 「用户改了没人读」是**成族**（34 个控件）而不是个例，与 `is_async`/`cache_*`/面板技术指标 8 项同一族。

`kline_period` 是唯一 `intentional`：v115 那批的书面裁定写在 `seed_stock_analysis.rs` 的 t-market-data 接线注释里
（接上后用户设 weekly/monthly，`kline_limit=250` 就变成 250 根周线）。

### (5) 未做（归入「需拍板」桶，不是遗漏）

逐条**接**还是**删**没做：接上会改决策数值（仓位上限、集中度/分散度判据、PE·PB 修正阈值、RSI 阈值、取数条数），
且 RSI 同名两组值不同 ⇒ 只接一组等于把歧义换个位置。需要一条产品裁定：
**（a）整族接线**（面板值权威，定 ladder 与面板值的次序，等价于把 §一一一 的裁定反过来一档）/
**（b）整族退役**（删声明 + 删面板控件 + 删 11 语言描述，Rust 常量为唯一权威）/
**（c）分族**：仓位/风控/估值接，`ref_*_code`×7 与 `monitor_*`×3 退役。

## 一一三、B-3（#20②）架构裁定：走**端口 + 适配**，不给 astock-data 加数据库依赖（2026-10-08）

§八十四 给 B-3 留了一个没答的问题：归档升格成历史表之后，**读取侧在 `astock-data`，而 `astock-data` 依赖里
没有 entities/dao/sea-orm**（只有 harness + redb + moka + parking_lot）。当时列了两条路：
①给 `astock-data` 加数据库依赖；②定一个 trait 再改异步。**裁定走②形，但 trait 不住 harness** ——
现场有一条一模一样的先例，不必新造分层：

- `axagent-core/crates/harness/src/news_archive.rs:19` `pub trait NewsArchiveSink: Send + Sync`（**async 方法**：`upsert` / `search_asof`）——**2026-11-03 B1（`#100`）起定义已上收 harness**：原定义在 `crates/astock-data/src/lib.rs:91`（写本条时记作 `:85`，当时已漂），现该处改为 `pub use axagent_harness::news_archive::NewsArchiveSink;`（`lib.rs:85`）⇒ 本条「由 astock-data 自己声明」的结论作废；「端口不进实现层、实现留 wiring」的原则不变（harness 是 foundation，仍无 entities/dao 依赖）。
  由 `axagent-harness` 声明（B1 前为 astock-data 自己声明），实现落在 `src-tauri/src/init/news_archive_sink.rs:105`（wiring 层，看得见 dao/entities），
  注入点 `AStockClient::with_news_archive_sink(sink: Arc<dyn NewsArchiveSink>)`（`lib.rs:922`）。
- ⇒ B-3 的正解是**同形复制**：端口在 astock-data（下层声明需求），适配器在 `src/init/`（上层用 sea-orm 实现）。
  **禁止**路①：①会让 `astock-data` 反向依赖 `entities/dao`，而 `analysis-engine → astock-data` 已经存在，
  `dao` 那一侧又依赖 entities ⇒ 直接把 implementor 层拧成环（禁区 1）。

### (1) 目标形状（全部照 `market_daily_close` 那份形抄，它文件头十二行就写着「不用 DiskCache 承载本表」）

| 面 | 落点 | 规格 |
|---|---|---|
| 端口 | `crates/astock-data/src/daily_snapshot.rs` | `#[async_trait] pub trait DailySnapshotStore: Send + Sync`：`get(method,date)` / `get_stock(method,code,date)` / `get_keyword(method,keyword,date)` / `put(method,scope,date,json)` / `contains(method,scope,date)` / `count_by_date_range(method,from,to)`（最后这条是 DiskCache **结构上给不出**的跨日聚合，正是升格的唯一正当理由）；`scope: Option<&str>` 统一现有三种 key 形态（全市场 / 个股 / keyword），空结果过滤 `usable()` 的语义**搬进适配器**，不留在调用方 |
| 实体 | `crates/entities/src/astock_daily_snapshot.rs` | `id = "{method}@{scope:-}@{date}"`（主键即天然去重，重复采集 upsert 覆盖，同 `market_daily_close.rs:20-22`）；列 `method` / `scope` / `snapshot_date` / `payload` / `collected_at`；`method`、`snapshot_date` 各 `#[sea_orm(indexed)]`，跨日聚合要 `(method, snapshot_date)` **多列索引** ⇒ 进 `extras.rs` 的 `MULTI_COL_INDEXES` |
| 注册 | `crates/dao/src/reconcile/extras.rs` | 加多列索引条目 ⇒ **必须同批改冻结计数**（`EXPECT_MULTI: usize = 35`（`:1294`）与 `l2_total_is_sixty_nine`（`:1340`）），还有 `EXPECT_CHECKS` / `EXPECT_PARTIAL` 若加了 CHECK；`expected.rs` 的「每个注册实体都要成表」自动覆盖（`dao/build.rs` 扫 `entities/src/lib.rs` 的 `pub mod`），**不需要**手工表名数组、**不需要** migration |
| 适配器 | `src-tauri/src/init/daily_snapshot_store.rs`（新） | `impl DailySnapshotStore for …`，读写 sea-orm；`count_by_date_range` 走 SQL `find().filter(method).filter(date between).count()` |
| 装配 | `src-tauri/src/init/state.rs:1617` | 现在这行是 `client.with_daily_snapshot_cache(app_dir.join("astock_daily_snapshot.json"))` ⇒ 换成 `with_daily_snapshot_store(Arc::new(…))`，并把旧 `with_daily_snapshot_cache` 与 `DailySnapshotCache` 的 DiskCache 实例**退役**（不留双写：两条真相正是本仓「配置项空接线/两套权威」族） |
| 读侧 | `crates/astock-data/src/lib.rs` 的 **14 个点**（§八十四 写「13」是漏数了 `get_index_quotes:5408` 那一臂；按名目枚举见下表，实测计数优先） | 已实测：14 个点**全部在 `pub async fn` 内**（`try_vendor_with_asof:1843`、`get_social_sentiment:2570`、`get_money_flow:3099`、`search_stock:3316`、`get_margin_data:3596`、`get_pledge_data:3772`、`get_north_bound_holding:3884`、`get_sector_info:3976`、`get_consensus_eps:4204`、`get_concept_blocks:4309`、`get_hot_stocks:4590`、`get_industry_ranking:4755`、`get_cls_flash:4959`、`get_index_quotes:5394`）⇒ 只加 `.await`，**不产生同步函数被改异步的涟漪**；`has_daily_snapshot` / `daily_snapshot_enabled` / `set_daily_snapshot` 是 `pub`，调用方在 `src/commands/stock_analysis.rs:4396` 与 `src/init/services.rs:3625/3651`（两处都已在 async 上下文） |
| 门 | `scripts/check-snapshot-whitelist.mjs` | **不需要重指**：`SNAPSHOT_METHODS` / `PER_STOCK_METHODS` 两张清单仍在同一文件同一位置，R1/R2 的扫描面不变；只需把注释里「DiskCache key」的措辞改成「表主键域」，并加 **R4**：若实体侧加了 `method` 的 CHECK 允许集，白名单每项必须落在该集内（否则采了进不了表） |

### (2) 验收口径（这条决定「升格」是不是只是换个存储）

⚠ §八十四 要求「升格之后要能真做一次跨日聚合」。现网**没有任何决策侧消费者**，
而给决策加新输入＝改图＝换代 ＝ 把 B-3 变成 #41 那种量级的批。⇒ 本批的跨日聚合消费者**定为呈现/诊断侧**：

1. 用 `count_by_date_range` 出一条真实读数：**回放锚点前 N 个交易日内，某 method 有快照的天数 / N**；
2. 该读数进**缺席三态文案**（现 `snapshot_absence_clause` 那条），把「这一天没有快照」与
   「整段窗口都没采到」分开说 —— 后者是采集侧断了，前者是数据本身没有；
3. 不新增工作流节点、不改任何决策数值 ⇒ **不发版**（`TEMPLATE_VERSION` 不动，判据同 §八十二(4)）。

### (3) 现网前置（没解除也能写码，但验收会假绿）

- **#48（PG 冷页读挂）**仍未解锁：B-3 要回查历史行，必须等那条 elevated 重启
  （`Restart-Service postgresql-x64-18`，`io_method='sync'` 已 staged 在 `E:/Data/PostgreSQL/postgresql.auto.conf`）；
- 存量 `~/.axagent/astock_daily_snapshot.json` 里的历史快照要不要**回填进表**：裁定**要**（否则升格当天回放覆盖率
  从「有」跳到「空」，那是把旧数据判没了），回填走一次性命令 + 幂等 upsert，回填完把旧文件改名保留（不删，留回退）。

## 一一四、当前三桶（2026-10-08 收尾对账，给 §一○七 那句「见三桶」落地）

### 能做（纯代码，不依赖外部条件）

1. **B-3 = #20②**（归档升格历史表）—— 架构裁定已在 §一一三，逐面规格到位；本日晚些已派隔离 worktree 子执行体在跑。
2. **B-4 = #20①**（龙虎榜原生 as-of 读侧）—— 排在 B-3 之后，共用同一张表框架，省一次框架设计。
3. **#18 发布日历侧信道**（§一一○(3) 已给五条执行规格，判据可跑在离线夹具上）。
4. **36 条空接线的逐条处置**（§一一二(4) 那张清单）—— 前置是下面「需拍板」第 1 条。

### 需拍板（判据不机械，给选项与代价）

1. **36 条接还是删**：（a）整族接线（面板值权威，需同时定「面板值 vs Rust 常量/ladder」次序，等于把 §一一一 的裁定反过来一档，且会改决策数值）/
   （b）整族退役（删声明 + 删 34 个面板控件 + 删 11 语言描述，Rust 常量为唯一权威，零数值影响）/
   （c）分族：仓位·风控·估值接，`ref_*_code`×7 与 `monitor_*`×3 退役。
   ⚠ RSI 有 `signal_rsi_*`(30/70) 与 `rule_rsi_*`(20/80) 两组同名不同值 ⇒ 只接一组＝把歧义换个位置。
2. **`holding_period.rs:177` 的 `macroPartialSeriesOnly` 中文硬编码**要不要搬 i18n（禁区 2；它已在 §一一○(3)4 被列为同批改对象，
   但搬动会牵 11 语言与那条文案自己的判据）。

### 等外部 / 等用户动作（代码可先行，验收必须等）

1. **#48**：需要一次**提权**重启 `Restart-Service postgresql-x64-18`（`io_method='sync'` 已 staged 在 `E:/Data/PostgreSQL/postgresql.auto.conf`）。
   它挡着 v129–v140 的全部实机核对、B-3 的历史行回查，以及所有现网读数。
2. **#12**：需要授权跑一次现网荐股循环给 `reco_picks.reco_version` 盖章（上次读数是 1213 行全 NULL ⇒ 筛样做完也无物可筛），
   然后重取读数并摘掉登记门里那两条 `pending`（`reco_ic_stats` / `load_reco_loop_samples`）。
3. **#11**：三项吞吐改造需要 #48 解除后的耗时读数。
4. **#18 官方数值序列**：需要统计局开放平台凭据 —— 已裁定**不走这条路**（§一一○(2)1），留档防止再猜。

## 一一五、36 条空接线的拍板 = **（a）整族接线**（2026-10-08 用户一字「接」）

### (1) 拍板读法

「接」在 §一一四 的三个选项里唯一排除的是「退役」⇒ 选 **(a) 整族接线**，面板值成为权威。
(c) 那条「分族」是 (a) 的子集（它只把 `ref_*`×7 与 `monitor_*`×3 判成退役），既然拍了「接」，
这十项也一并接，**除非**普查证明它根本没有执行路径（见下 (4)）。

### (2) RSI 两组同名不同值的冲突：实测证明**不是冲突**，是阶梯的内外两带

拍板前我把四带常数与两组默认值逐字对了一遍（现场读数）：

| 来源 | 值 |
|---|---|
| `crates/astock-data/src/scoring.rs:104-109` 的 `ScoringBands` | `rsi_oversold_deep=20` / `rsi_oversold=30` / `rsi_overbought=70` / `rsi_overbought_high=80` |
| `crates/astock-data/src/indicators.rs:152-153` 的 `IndicatorConfig` | `rsi_overbought=80` / `rsi_oversold=20` |
| 变量 `signal_rsi_oversold` / `signal_rsi_overbought` | **30 / 70** |
| 变量 `rule_rsi_oversold` / `rule_rsi_overbought` | **20 / 80** |

⇒ 两组各自精确对应**不同的一档**：`signal_rsi_*` = 内带（`rsi_oversold`/`rsi_overbought`），
`rule_rsi_*` = 外带（`rsi_oversold_deep`/`rsi_overbought_high`，同一对数也正是 `IndicatorConfig` 的那两个字段）。
所以 §一一二(5) 担心的「接一组等于把歧义换个位置」**不成立** —— 两组都接、各挂各的带即可，
且**面板默认值 == 现 Rust 常量 ⇒ 接线后现网数值逐位不变**（与 v137 那批同一条安全判据）。

### (3) 接的三条硬规矩（照 v137 那批的形，不新造）

1. **默认值对账**：接之前先证「面板默认 == Rust 现值」，不等的那几条要单独点名（它们会变数值，属改动决策）；
2. **一张表里同时放键与 setter**（`INDICATOR_BAR_ARGS` 形）⇒ 「加了第六个键忘了加分支」不可能静默丢参；
3. **门跟着收**：每接一条，登记门 `check-variable-consumer-registry.mjs` 的 **G2 会当场红**（登记项已有引用面却不摘豁免＝面积读数造假）
   ⇒ 摘掉该条豁免即是这批的完成计数，**门自己会数还剩几条没接**。

### (4) 前置：落点普查已在跑

36 条里每条的「现值实际由什么决定」（Rust 常量 / 字面量 / ToolDef `default` / 提示词里写死的数 /
Rhai 常数 / 节点映射里被硬编码的字面值）**必须逐条实测后再动手** —— 本仓最隐蔽的失效就是「两套权威」，
凭印象接会把「没人读」换成「两个人读、值不一样」。普查表（含「根本没有执行路径」的点名）回来后，
本批按「现网数值不变的先接、会变的单独一批并带 `TEMPLATE_VERSION` 讨论」切分。

## 一一六、落点普查回来了：「接」只对 14 条成立，另 22 条**根本没有执行路径**（2026-10-08）

普查由只读子执行体跑，回来后**我逐条复核了其中四条关键断言**（`rev_*` 键名、pb/pe 现值、
`start_with_config` 调用者、`RuleEngine` 引用面），下面的结论都按复核后的读数写。

### (1) 当场抓到并修掉的一条真 bug（数值不变）

`crates/analysis-engine/src/recommender/strategies/reversion.rs:62` 读的是 `rev_divergence_lookback_days`、
`:93` 读的是 `rev_rsi_period_days`（两处**修改前**在 `:58` / `:89`，本次加了四行判据注释 ⇒ 行号整体下移），而 `seed_variables.rs:2218` / `:1207` 声明的是
`rev_divergence_lookback`(14) 与 `rev_rsi_period`(6) ⇒ **读取恒不命中、永远走兜底**；
兜底值又恰好等于默认值（14/6）⇒ 这条缺陷**从不现形**，面板那两格改了没有任何东西读。

- 修法：读侧改成声明名（两行），并在 `:55` 上方写下判据注释。
- 数值影响：**零**（默认 == 原兜底 ⇒ 除非用户曾在面板改过，那时「生效」正是本批目的）。
- 族价值：这是空接线里最隐蔽的一型 —— **正向 G1 判据看不见它**（读侧真的存在，只是名字多/少一个后缀）。

### (2) 因此门加了 R6（反向判据），并把它自己的首跑读数打在未修的树上

`scripts/check-variable-consumer-registry.mjs` 新增 **R6：读取侧点名的键必须真在声明表里**，
形状与 G1 对称（新违规红、登记项一旦有了声明必须摘登记、`intentional` 必须给 cite、抽取面有下限）。

| 读数 | 值 |
|---|---|
| 门禁 | **EXIT=0**：`声明 374 ｜ 零引用面 35（已登记 35）｜ 面板仍渲染 33 条`（修完 rev_divergence_lookback 后 36→35、34→33） |
| R6 首跑 | `读取面键 93 个，其中不在声明表 28 个（全部已登记：是）` |
| 负控 | **11/11**（原 9 条 + R6 两条：「读一个没人声明的名 ⇒ 点名键与文件」「登记项有了声明却不摘 ⇒ 红」） |
| `cargo fmt --check --all` | **FMT=0** |
| `cargo clippy -p axagent-analysis-engine --all-targets --all-features -- -D warnings` | **CLIPPY=0** |

28 条登记分两族，**严重度不同，别混报**：
- 2 条 `intentional`：`trend_ma_tolerance_daily` / `trend_high_tolerance_daily`
  —— `trend.rs:171` 的注释就是判据（日基准刻意由 Rust 常量给，各档按 √d 推，「消灭每张策略各自手抄一档阈值」）；
- 26 条 `pending`，全部集中在 `backtest_strategy.rs` 的 `detect_*`（`trend_*_min_kline_len`、`value_*_deviation`、
  `technical_max_rsi` 一族）⇒ 这些**面板本来就没有控件**，所以它们是「压根不可调」而不是「改了没人读」，
  与 (b) 桶那 33 个控件不是一回事。

### (3) 「接」可执行的 14 条 —— 按数值影响切两批

- **A 批（现网数值逐位不变，9 条）**：`signal_rsi_oversold/overbought`（落 `scoring.rs:105/:108` 内带）、
  `rule_rsi_oversold/overbought`（落 `indicators.rs:152-153` 外带）、`pos_max_single_pct`/`pos_max_total`/
  `pos_max_sector_pct`（`position_limits.rs:108` 构造侧改按 vars）、`val_pe_low`/`val_pe_high`（`scoring.rs:318/:320`）、
  `monitor_poll_interval_secs`（把 `services.rs:3500` 的 `start()` 换成已有正门 `monitor.rs:252` 的 `start_with_config`）。
- **B 批（会改决策数值，7 条，须单独一批并讨论换代）**：`val_pb_low` 1.5→1.0、`val_pb_high` 5.0→6.0、
  `value_moat_threshold` 70/40 双档→60、`value_safety_margin` 30%→20%、`risk_divers_medium` 5→4、
  `monitor_change_pct`/`monitor_turnover` 从无全局阈值→5/10。
- ⚠ 必须同批的第二权威：`val_pe_*` 在回放链还有自己的一份 —— `src/commands/stock_analysis.rs:659-662`
  那句注释「简化版：PE<20 视为低估，PE>40 视为高估」连同代码是**第二权威（20/40）**，
  只改 `scoring.rs` 会留下两套权威；`value_safety_margin` 同理（`mcp_tools.rs:2976` 的 `REQUIRED_MARGIN_OF_SAFETY=0.30`
  与 `portfolio-mgr-h-long.rhai:161` 的「DCF 上行 < 30%」）。

### (4) 22 条**接不了**：没有执行路径（需要第二拍）

| 族 | 条数 | 证据 | 只有这两条路 |
|---|---|---|---|
| `RuleEngine` 整条链零调用者 | 6（`rule_auto_stop_loss_pct`/`rule_bias_limit_pct`/`rule_bear_low_score`/`rule_volume_signal_block` + 走它的 `rule_rsi_*` 两条） | `analysis-engine/src/rules.rs:19` 的 `impl RuleEngine` / `:27` 的 `pub fn check` —— 全仓搜 `.check(` 带 rule 的调用点**零命中**，常量在 `decision.rs:165-181` | 重启这条链（＝新决策输入）/ 退役 |
| `t-baseline-*` 节点早在 v17 删除 | 7（`ref_*_code`） | `seed_serenity.rs:32`、`:1505`；`seed_variables.rs:162` 那句「t-baseline-* 使用」是**假账** | 只能退役（现役对标另有码表 `industry_chain.rs:577` 等） |
| 现役判据另有形 | 4（`risk_divers_medium/high` 判据是**持仓只数** `portfolio_monitor.rs:210/:212`；`risk_hhi_medium` 连比较点都不存在；`monitor_change_pct/turnover` 阈值住在 `price_alerts` 行级 `alert_mapping.rs:70/:73`） | 同左 | 改成面板口径（推翻现役判据）/ 退役 |
| 与尺度层冲突 | 3（`val_ultra_short/short/long_kline_limit`） | 取数根数现由 `scale.rs:66-108` 的 `fetch_limit` 推；同型 `rev_kline_limit`/`rev_min_kline_len` 已有摘除裁定（`reversion.rs:38-40`） | **不可接**（接了就是 #41 那批「档位各抄一份日线口径」的回潮）/ 退役 |
| 已裁定不接 | 1（`kline_period`） | `seed_stock_analysis.rs:3075` | 维持 |

⇒ 「接」这条拍板对这 22 条不是可执行动作。要真消灭它们造成的界面歧义，只剩两条路：
**造执行路径**（等于新增决策输入，需逐条定义判据）或**退役**（删声明 + 删面板控件 + 删 11 语言描述，零数值影响）。
我的建议是退役，理由是这 22 条的现役判据**都另有权威**，把面板接上去会出现「同一个量两处权威」——
那比现在更坏；而 B-2b 前置 1 那批（`is_async`/`cache_*`）已经立过同一形的处置先例。等你点头就按退役做。














## 一一七、退役 22 条已落地（2026-10-08，**不发版**）

「点头」＝批准 §一一六(4) 建议的退役。五处同批改完；先跑演练、断言过了才落盘。

### (1) 改动面

| 面 | 改了什么 | 读数 |
|---|---|---|
| 声明 | `seed_variables.rs` 删 22 个 `Variable` 块 | 声明数 **374 ⇒ 352** |
| 面板 | `StockAnalysisConfigPanel.tsx` 删 **19** 个 `b()` 控件声明 + **19** 条分组条目 + **整组删两个组**（`tool:"rules"` 组 6 条全退役、`tool:"refCodes"` 组 7 条全退役） | 演练逐组打印 `rules 删 6 ⇒ 剩 0【整组删除】`、`refCodes 删 7 ⇒ 剩 0【整组删除】`、`strategy_data 删 3 ⇒ 剩 10`、`tool_executor 删 1 ⇒ 剩 4`、`monitor 删 2 ⇒ 剩 1` |
| 别名 | `stock_analysis_setup/mod.rs` 的 `const RENAME_MAP` 删 9 条 | 留着＝指向不存在键的别名（旧 camel 值无处可迁） |
| i18n | 11 语言各删 19 个 `stockAnalysis.configDescriptions.<camel>` 键（另 3 条本来就没建键） | 每份 configDescriptions **191 ⇒ 172** 键，11 语 key 集合一致 |
| 门 | `check-variable-consumer-registry.mjs` 摘掉这 22 条 `EXEMPT`；G4 负控改成**合成条目** | 负控仍 **11/11** |

> 不发版的判据同 §八十二(4)：`same_graph && same_variables ⇒ skip` —— 图一字未动，变量表变了会走
> `merge_variable_values`（字段定义以新模板为准）⇒ 存量库里那 22 个键随下一次播种消失，**不需要**占 `TEMPLATE_VERSION`。
> 与 B-2b 前置 1 退役 `is_async`/`cache_*` 同形。

### (2) 为什么「整组删除」不是可选动作

分组是 `vars: resolve([...])`，而 `resolve` 对取不到变量的名字做 `filter(Boolean)`（`StockAnalysisConfigPanel.tsx:379`）⇒
只删条目不删组，界面就留下**有标题、没内容**的组 —— 那正是本批要消灭的「结构性缺口在 UI 造成歧义」。
`rules`（6 条全退役）与 `refCodes`（7 条全退役）两组因此必须连对象一起删。

### (3) G4 负控必须换前提样本（老毛病又差点犯）

原负控借 `kline_period` 造「intentional 却没 cite」的场景 ⇒ 它退役后该负控会**恒不命中**（假绿）。
改成合成条目 `[{ name: names[0], status: "intentional" }]` ⇒ 判据不动、样本自己生成（同「被锁符号退役要换前提样本、不删判据」）。

### (4) 我报错的两处口径，在这里更正

1. 上一轮报的「面板仍渲染 **33** 条」是按**分组条目**算的**上限**：其中 3 条（`val_ultra_short/short/long_kline_limit`）
   根本没有本地 `b()` 兜底声明，只靠后端变量表渲染。门里 `panelExposes` 同口径 ⇒ 待把输出改成「渲染面（上限）」，
   并分列「有本地兜底声明」的条数。
2. i18n 第一遍用 `corpus.includes(camelKey)` 判引用，把 `KLineChart.tsx:85` 的 **store 字段** `klinePeriod`
   误当成对 `configDescriptions.klinePeriod` 的引用 ⇒ 留了 1 个孤儿键。第二遍改按**完整调用形态**
   `configDescriptions.<key>` 判 ⇒ 补删，读数 `仍被引用 0`。
   ⇒ 判据：**「有没有引用」要按调用形态判，按标识符子串判必然假阳。**

### (5) 剩下的 13 条是「待接」，不是「待删」

门读数：`声明 352 ｜ 零引用面 13（已登记 13）｜ pending 13 条`（36 − 退役 22 − 已接 1 = 13）。

- **可接、且现网数值不变（9 条）**：`signal_rsi_oversold`/`signal_rsi_overbought`（落 `scoring.rs:105/:108` 内带）、
  `pos_max_single_pct`/`pos_max_total`/`pos_max_sector_pct`（`position_limits.rs:108` 的 `Default` 就是 20/10/40）、
  `val_pe_low`/`val_pe_high`（`scoring.rs:318/:320`，**必须同批**改 `commands/stock_analysis.rs:659-662` 那份 20/40）、
  `monitor_poll_interval_secs`（`services.rs:3500` 现调 `start()`，正门是 `monitor.rs:252` 的 `start_with_config`，零调用者）、
  `news_limit`（入口在 `mcp_tools.rs:1094` 的 `unwrap_or(30).min(100)`，但 `:4877/:5004/:5150` 三处调用点各自写死 ⇒ 要一并收）。
- **接了会改决策数值（4 条）**：`val_pb_low` 1.5→1.0、`val_pb_high` 5.0→6.0、`value_moat_threshold` 70/40 双档→60、
  `value_safety_margin` 30%→20% ⇒ 单独一批并讨论换代。

⚠ 一条我上轮说错的地方在这里更正：A 批原本含 `rule_rsi_oversold/overbought`（说它们落 `IndicatorConfig` 外带）。
普查后它们**唯一的声明消费路径是死的**（`RuleConfig`/`RuleEngine`，`rules.rs:19` 的 impl 与 `:27` 的 `pub fn check` 全仓零调用者）；
把它们接到 `IndicatorConfig` 不是「补接线」而是**新决策** —— 同一对数字会同时驱动指标信号，而 `IndicatorConfig`(80/20) 与
`ScoringBands`(deep 20 / high 80) 本就是两处活权威 ⇒ 已按退役处理。将来要给 RSI 阈值开面板，先定「压哪一处权威」，
那属于 §一一四 的「需拍板」，不是 §一一七 的收尾。

## 一一八、v142「修了等于没修」的两条现网读数，与本轮的换代修法（2026-10-09）

> 用户现场两次截图同样报错（第一次 `data-quality ← a-market-analyst`，第二次
> `t-dragon-tiger-data ← a-hot-money`），我上一轮只做了**定点**剔除**且没有升版**。
> 本节只记读数与判据，不写「已完成」——实机验收（启动 + 真跑一次分析）仍在下面第 4 条等。

1. **库里那一版到底坏在哪**（`pg-connect.mjs` 只读，按 `jsonb_array_elements` 交叉查边端点是否落在节点集）：
   - `workflow_templates` 中 `id='stock-analysis'` 当前 `version=142`，悬空边**恰好一条**：
     `e-t-dragon-tiger-data-hot-money`（source `t-dragon-tiger-data` 在，target `a-hot-money` 不在节点集）；
   - 四张 `stock-horizon-<档>` 与 `stock-analysis-fast` 的悬空边读数均为 **0** 行
     ⇒ 阻断点只在父图，不是四档子模板；
   - ⇒ 症状与截图逐字对得上，不是「缓存旧」也不是「用户没重启」。

2. **为什么 v142 的修复在用户机器上一次都没跑**：版本门是
   `existing.version >= TEMPLATE_VERSION ⇒ return Ok(())`（本轮实测位置 `seed_stock_analysis.rs:2063` 的版本门段，
   该文件自己的注释 :237 / :266 早已写明「改图必须升版」）。代码升到 142、库里也已播种成 142，
   而 142 那次播种**写进库的就是带悬空边的图** ⇒ 此后每一次启动都被门静默跳过。
   判据：**改图内容的批必须同时改版本号**，这不是可选的收尾动作，而是那一批的一部分。

3. **本轮改法（三代：142 → 143）**：
   - `seed_stock_analysis.rs` 新增模块级 `prune_dangling_analyst_edges(present, edges, migrated_analyst_ids)`，
     在主图序列化前对**每一条边**判一次：两端齐 ⇒ 留；缺的那端**确为** B-2b 迁进档子模板的分析师 base id ⇒ 剔除并
     `warn` 出清单；其余悬空边 ⇒ `Err` 拒绝播种（否则将来新写错的边会被兜底静默吃掉）。
     定点那套（本轮实测位置 `:4992` 的 `dq_present`）保留，它管的是 data-quality 的**十维映射**，与边不是同一格失效面。
   - `TEMPLATE_VERSION` 142 → **143**（`HORIZON_TIER_TEMPLATE_VERSION` 按别名同升）。
   - 两道新门（`stock_analysis_setup/mod.rs` 的 `version_gate_tests`）：
     `seeded_main_graph_has_no_dangling_edges`（整图口径，按每一条边断言，并带「主图确实不该再有
     `a-hot-money` 节点」的前提样本，防空集恒真）；
     `dangling_edge_guard_prunes_analysts_and_rejects_others`（兜底函数**两条分支**各一条 + 两条负控：
     非分析师悬空边必须 `Err`、`a-hot-mony` 这种「前缀像分析师但不在清单」也必须 `Err`，
     锁住判据是白名单而不是前缀匹配；再加 source 侧悬空一条，覆盖 `contains(source)` 那一半分支）。

4. **实机验收（本轮已做一半，剩「跑一次分析」）**：用带 v143 的新二进制**直接起进程**（复用已在跑的 vite 1420），
   启动日志 312 行、`ERROR`/`panic` 计数 **0**，`[panel_variables] 变量表快照已装入：352 条（模板 stock-analysis）`，
   但**没有出现任何 `v142 → v143` 的播种行** —— 这不是缺陷，而是我此前的表述错了：
   `ensure_stock_analysis_experts_seeded` 的调用点是 `stock_workflow/decision.rs:500`（`load_and_inject_template`
   的**运行前幂等兜底**）与 `serenity.rs:955`，**应用启动链路里根本没有 stock_analysis_setup 的种子调用**
   （同一条注释就写着它存在的理由：「代码 v13-v15 期间 DB 停在 v12，V59 修复静默失效三天」）。
   ⇒ 更正：界面文案与 §一一九 那句「启动时重播种」都已按「下一次分析运行时」改写（11 语言同批）。
   **仍未完成**：真跑一次分析 ⇒ 期望依次出现「更新股票分析工作流模板 v142 → v143」与
   「已剔除指向档子模板分析师节点的悬空依赖边: ["e-t-dragon-tiger-data-hot-money（…）"]」，
   然后库里 `stock-analysis` 的悬空边读数应回到 **0** 行，用户点「开始分析」应出结论。
   在此之前，§七十九 以降所有「实机核对」条目继续挂账。
   ⚠ 我没有用 `axagent-batch-rerun` 代跑这一次：它会真花 LLM 配额并往库里写一行分析记录，属未授权范围。

## 一一九、历史分析列表补「图代」列 + 代码/库内版本对照（2026-10-09，用户点名）

用户诉求原文：**「我的工作量列表中没有显示工作流版本，这不符合版本的频繁更新条件」**。
这条不是美化 —— 本仓 `TEMPLATE_VERSION` 近期从 129 推到 143，落差六代**全靠人肉查库**才发现（§一一八 第 2 条就是它的一次实付）。

- **现网读数**（改前先取，不拿「代码看起来会这样」当证据）：
  `stock_analyses` 83 行，其中 `template_version` 非空 **37** 行，`max=127 / min=65`
  ⇒ 列本身早就在写（写侧由已提交的 `tests/stock_workflow_write_side.rs` 锁住），
  缺的是**读侧与呈现**：`list_stock_analyses` 的 `select_only()` 没带这一列，界面自然看不见。
- **后端**：`list_stock_analyses` 拆出 `query_stock_analysis_items(db, limit, offset)`（命令体要 `State`，
  返回体形状否则无法单测），SELECT 补 `TemplateVersion`；新增只读命令
  `get_stock_template_version_status` → `{ codeVersion, dbVersion }`，`codeVersion` 取自
  `seed_stock_analysis::TEMPLATE_VERSION`（**唯一权威**，与 `axagent-batch-rerun` 读同一个数字，不得复制常量）。
- **前端**（`AnalysisHistoryButton.tsx`）：逐行一枚 `v<代>` 标签；弹窗顶部一行代际对照。
  对照刻意做**四态**而不是「一致 / 不一致」两态：
  `matched` / `behind`（差 N 代，**下一次分析运行时**重播种 —— 种子化点在 `decision.rs:500` 的运行前兜底，
  不在启动链路）/ **`ahead`**（库里高于代码 ⇒ 版本门跳过重播种，
  代码里的图改动不生效 —— 就是 §一一八 那一族的形状）/ `notSeeded`（库里没这张图，
  `dbVersion=null` 不得参与差值计算）。命令取失败时**整行不显示**，不假装「一致」。
- **i18n**：`stockAnalysis.templateVersion.{matched,behind,ahead,notSeeded,unknown}` 五条 × 11 语言。
  落盘前用脚本自带校验：每条按用途必须含 `{{db}}`/`{{code}}`/`{{gap}}`，且摘掉合法槽后**不得再有余括号**
  —— 该检查当场抓到 fr 的 `{{gap}`（少一个右花括号）两处，i18next 对残缺槽是原样显示、parity 门也查不出。
- **门读数**（本轮实测，非计划）：`check-i18n-key-parity.mjs` ⇒ 11 个语言文件 key 路径集合一致（各 15436 条）；
  `check-i18n-placeholder-usage.mjs` ⇒ 结论「占位符使用干净」；`check_i18n.py` ⇒ 全部通过；
  `npm run typecheck` ⇒ 无输出（绿）；新增 `AnalysisHistoryButton.templateVersion.test.tsx`
  ⇒ **5 passed / 5**；`check-contracts.mjs` ⇒ errors=0 warnings=0 PASS；`check-line-endings.mjs` ⇒ PASS；
  `check-single-source-facts.mjs --ci` ⇒ 本轮一度**硬拦 2 处**：我在种子文件里插了 54 行（实测按同一句注释的
  行号差算出：`seed_stock_analysis.rs` 的「原问题：bull_tools 含 td_quote…」从 3654 移到 3708），
  把另外两个文件按行号写的引用（`crates/harness/src/workflow_port_axioms.rs:15`、
  `src/commands/workflow_template.rs:972` 都指向 `seed_stock_analysis.rs:3655`）顶到了空行上 ⇒
  已按**内容**重新定位到讲解该约定的 `:6354`（顺带更正：那两处引用在 HEAD 上就已语义腐烂，
  门只查「空行/越界」不查语义，所以一直绿）⇒ 现 BROKEN/越界/空行/控制流 **0**。

## 一二〇、「工作流列表里看不到版本」这句我第一轮读窄了（2026-10-09，用户第二次发作）

用户原话是「我的**工作量**列表中没有显示工作流版本」（语音/输入误差），我第一轮只按
「历史分析列表 = `AnalysisHistoryButton`」这一种读法落地（§一一九）。他随后一句
「版本号也修不好」证明还有第二种读法没覆盖：**工作流模板列表本身**（业务模板页 + 系统模板页）。
两种读法现在都落上了。

- **现场读数（为什么这不是「没做」而是「做在另一条链上」）**：
  `crates/harness/src/workflow_types.rs:2720` 的 `WorkflowTemplateResponse` **一直带** `pub version: i32`，
  `src/components/workflow/types/workflow.types.ts:856` 的 TS 侧也**一直声明** `version: number`
  ⇒ 数据从后端一路送到前端，**唯独两个列表控件从不渲染它**（`TemplateList.tsx` 标题行只渲染
  icon/名称/预设标签/只读标签，`SystemTemplateList.tsx` 同形）。所以这不是取数缺口，是纯呈现缺口。
- **落地**：两个列表的标题行各加一枚 `v<代>` 标签（`{`v${template.version}`}`）。
  不加新 i18n key —— `v143` 这种形态在带「版本历史」入口（`TemplateList.tsx` 的 `versionHistory` 菜单）
  的列表里无需第二句说明；文案越少，11 语言的失效面越小。
- **新门**：`src/components/workflow/__tests__/templateListVersion.test.tsx` ⇒ **4 passed / 4**。
  它按**四层**各锁一处（后端 DTO 字段 → TS 必填声明（显式断言**不得**写成 `version?:`，
  可选声明会让控件静默显示 `vundefined`）→ 两个列表各自真的渲染），
  并带一条「摘掉控件后上一条断言会红」的自检。
- **本轮其余读数**：`npm run typecheck` 无输出；`dprint fmt` 改了 1 个文件后 `check` 干净；
  `oxlint src/components/workflow/Templates/` ⇒ 0 warnings 0 errors。
- **一条我**没有**顺手做的、必须说清的边界**：**执行历史里根本没有「哪代图跑的」这一列**
  （`crates/entities/src/workflow_executions.rs` 的字段只有 `id/workflow_id/status/input_params/
  output_result/node_executions/total_time_ms/execution_state_json/paused_at/created_at/updated_at`）。
  要做「每次执行显示其图代」得先加列（实体列属性 + `dao/reconcile/extras.rs` 声明式对账 + 写侧），
  那是新的一批，不在「把版本显示出来」这句的范围里 —— 我没有擅自扩，等你点头再开。

## 一二一、v144 断两条回边（Cycle detected）+ v145 清 brief 的死注入（2026-10-09）

用户第三次截图：`{"code":"STOCK_WORKFLOW_INTERNAL", ... "detail":"创建工作流失败: Cycle detected in workflow"}`。
**悬空边报错确实消失了**（同一张截图证明 v143 已落库），换它的是引擎校验顺序里的**下一道**：
重复 id → 悬空边 → **Kahn 环检测**。我 v143 只补了第二道的门，没补第三道 ⇒ 「修一条露一条」。

1. **先取读数再动手**（不跑分析也能验，姿势记在 `pg-query-gotchas.md`）：把他库里那份 v143 的
   `nodes` / `edges` 两列 dump 到文件，本地跑 Kahn ⇒ **45 个节点**留在强连通分量，实测两条环：
   - `debate-bull-bear → bull-r1 → bear-r1 → bull-r2 → bear-r2 → bull-r3 → bear-r3 → debate-convergence → value-investor--mid → debate-bull-bear`
   - `analyst-brief → debate-bull-bear → … → debate-convergence → value-investor--mid → analyst-brief`
   闭合边就是 `e-value-investor--mid-debate` 与 `e-value-investor--mid-brief`。
2. **根因（v133 遗留，不是 v143 引入）**：`value-investor` 在权威档位子集里（中/长档 `valuationBand`
   属主，harness 明写漏了就等于漏必填因子），v133 把「分析师→辩论」「分析师→brief」两个循环
   从 10 个 base id 换成 23 个**带档实例**时把它一起卷了进去；而它在主图是辩论**下游**节点
   （同文件 `seed_stock_analysis.rs:3868` 起就给它挂 `e-debate-{vi}` / `e-convergence-{vi}` 两条**入**边）。
   **判据本来就在仓里**：`:3326` 那个循环早就写了 `*b != VALUE_INVESTOR_ID` —— 缺的不是知识，
   是同一个判据在四处各自手写、其中三处漏了。
3. **v144**：两处循环（`e-{tid}-debate`、`e-{tid}-brief`）各加 `!= VALUE_INVESTOR_ID`；换代 143 → 144；
   补第三道门 `version_gate_tests::seeded_main_graph_is_acyclic`（对**落库后**的 nodes/edges 跑 Kahn，
   失败时直接打印残留节点集）。它的非恒真见证就是上面那条 45 节点的现网读数 —— 同一判据打在未修的图上。
4. **v145（用户一句「查出来的缺陷为什么不修改？」把我推回去的那批）**：v144 只断了**边**，
   `analyst-brief` 的 `input_mapping` 里那两把 `value_investor_raw__mid/long` 键还在 ⇒
   brief 声明要读一个图上看永远没跑的变量，Rhai 的 `present()` 恒假，
   「摘要里没有价值评估那一段」完全静默。同批做四件：
   ① `ab_input` 加同一个排除（23 → 21 把键）；② `analyst-brief.rhai` 删那两条 `present()`
   （脚本实测 `^if present(` = 21），段头与「23 条」注释同步改口；
   ③ 键集锁 `analyst_brief_keys_match_tiered_instances` 期望值 23 → **21**，且它的排除
   改用**提到模块级**的 `pub(crate) const VALUE_INVESTOR_ID`（原来它是 `seed_stock_analysis_workflow_template`
   函数内的局部常量，锁够不着 ⇒ 只能在测试里抄字面量，那正是「两处不同步还都绿」的形状）；
   ④ 换代 144 → **145**（脚本内容属于节点 `code` 字段 ⇒ 图内容变）。
5. **门读数（实测）**：`cargo fmt --all` ⇒ `FMT=0`；
   `cargo test -p axagent --lib stock_analysis_setup::` ⇒ **73 passed / 0 failed**（72→73 是新增的环门），
   其中 `seeded_main_graph_is_acyclic ... ok`、`seeded_main_graph_has_no_dangling_edges ... ok`、
   `analyst_brief_keys_match_tiered_instances ... ok`。
   过程中编译器还替我抓到一个自错：环门里 `.enumerate().map(|(id, i)| …)` 把元组顺序写反
   （`HashMap<String, usize>` 建不出来）⇒ 编译期即红，改完才绿。
6. **`check-single-source-facts` 的新形态教训**：这两批在 `seed_stock_analysis.rs` 里净增了 30+ 行，
   外部两处按行号写的引用（`crates/harness/src/workflow_port_axioms.rs:15`、
   `src/commands/workflow_template.rs:972` → `:6354`）**门是绿的**（`BROKEN/越界/空行/控制流 0`），
   因为门只验「那一行非空且不是裸控制流」—— 实际 `:6354` 已经指到 `quality-fallback` 那段无关代码，
   真正的目标注释移到了 `:6389`。⇒ 已按**内容**重定位到 6389。
   判据补一条：**门的「0 BROKEN」不等于引用还指着它当初指的那件事**，自己插过行的批次要按内容回看一次。
7. **仍未闭环**：v145 要下一次分析运行才落库（播种点在 `decision.rs:500`）；
   全量 clippy / `cargo test --workspace --no-fail-fast` 正在跑（`/tmp/g8.log`、`/tmp/g9.log`）。

## 一二二、B 裁定落地 = **v151**：风控否决**下到四档分支**，四档 Tag 与主档不再同屏互斥（2026-10-10）

**用户实证（兆易创新 603986 / 2026-10-10）**：历史记录主档结论「观望」（中期 · 分支选档，仓位 7.4% ·
置信 58%，reasoning 写明「主档取自中期分支『买入』，被风控否决降级降为『观望』」），
而同屏四档 Tag 却是「超短线: 增持 / 短线: 减持 / 中期: 买入 / 长期: 买入」——
**中期 Tag 说「买入」、主档说「观望」，同屏互斥，且无人能解释谁改写了谁**。

1. **根因**：四档**做了**风险分析，但只把**仓位**按本档风险档压低，**行动档（action）本身未否决**。
   否决只在主链发生一次 ⇒ 四档 Tag 是分支 **pre-veto** 的原始方向，主档是 **post-veto** 的最终结论。
   两者本就不是一回事，被并排展示却没有任何字段区分 ⇒ 看着像两套结论打架。
2. **三方案与用户裁定**：A = 维持现状（仅加说明）；B = **把否决下到四个周期分支各自的 action**
   （否决后的值即该档结论）；C = 仅呈现层标注「Tag 是原始方向、主档已否决」。
   用户原话：**「B —— 我需要的是最符合客观需求，而不是所谓的难度最低」**，C 已被明确否决
   （呈现层标注治不了「分支产出的结论本身含 pre-veto 的原始值」这个产端缺陷）。
3. **落地（v151）**：
   - 四份 `portfolio-mgr-h-{ultra-short,short,mid,long}.rhai` 在**各自动作阶梯之后**加「档内风控否决」：
     `action = pm_risk_veto(action, <本档 overall_risk>)`（本档风险档取自 `cls-risk-level-<档>`）⇒
     否决后的值即**该档结论**，四档 Tag 与主档天然一致。
   - 新增 `originalAction` / `tierVetoed` 两字段：保住**否决前的方向信息**（下游反思 / 冲突判定要读方向），
     并让「何以降级」可反解 —— 否则成句会退化成「…『观望』被风控降为『观望』」。
   - 主链否决**保留为兜底 + 加严**（`pm_risk_veto` 幂等 ⇒ 档内已否决时天然 no-op；仅当主链算法风险档
     比本档更严时额外收紧），新增 `actionSource` 第九值 **`tier_risk_veto`**（「最后改写者优先」，
     按档排名落在 `branch_pick` 之前一格）；reasoning 区分「本档风控否决(档):X→Y」与主链那句。
     ⚠ 刻意**没有**采纳「仅当 `originalAction == action` 时才由主链否决」——那是**回归**，
      会关掉「主链算法档比本档更严仍要收紧」的兜底保护。
   - 冲突判定（`decision.rs` 的 `compute_decision_agreement`）与来历读取
     （`stock-analysis-utils.ts` 的 `readDecisionProvenance`）改读 `originalAction`：
     LLM 侧 action 天然 pre-veto，比**原始方向**才公平，否则「本方风控政策」会被记成「两视角方向不一致」。
4. **三载体同步**：`rhai_registry` 整脚本门 ⑦ 的 `ACTION_SOURCES`（8→9）；
   `src/lib/stock-analysis-utils.ts` 的 `ACTION_SOURCES` / `DOWNGRADE_ACTION_SOURCES` + 标签函数；
   11 语言 `stockAnalysis.actionSourceTierRiskVeto`（zh-CN 源）。
5. **版本**：`TEMPLATE_VERSION` 150 → **151**（四份 `h-*.rhai` 与 `portfolio-mgr.rhai` 均经 `include_str!`
   嵌入节点 `code` 字段；`HORIZON_TIER_TEMPLATE_VERSION` 编译期绑 `TEMPLATE_VERSION`，无需各自 bump）。
6. **仍不**改的地方（诚实边界）：`reflection.rs` 的 `directional(&d.action)` 仍是 post-veto ⇒ 被否决的档
   会落出方向反思统计。经权衡保持现状（最终建议是观望则无方向可复盘），不在本轮改。
   `portfolio-mgr-arbiter.rhai` 的 `action` 仅作 `ranked` 透传、选档用 growth/positionPct ⇒ 不受影响。
