# PLAN：LLM 决策四档化 + 公式/LLM 逐档对照验证

> 本文件是 `PROPOSAL-stock-decision-four-horizon.md` **阶段 2 的 2.3 / 2.4 两条 LLM 半边**的接管件，
> 并新增「逐档对照验证」阶段。2026-10-16 全链路普查 + 现网取数后落笔。

## 0. 先纠正那份 PROPOSAL 的自述（两处已腐烂）

| 位置 | 它写的 | 代码现状 |
|---|---|---|
| §5「实际落地」段 | `portfolio-mgr.rhai` 的 `horizon_decision()` 闭包四组独立决策 | 该闭包 **2026-10-04 已随 R-11 退役**；现役正门是四路扇出 `pm-h-<档>` 子工作流 + `portfolio-mgr.rhai:3441-3491` 只做装配 |
| §5 改动清单 2.3 | 「已完成」（schema 里有 `decisions` 块） | **一行代码都没接**：见下 §1 第三条 |

## 1. 分层结论（名目 / 执行 / 数据 / 结局）

**四档权威只有一套**：`src-tauri/crates/harness/src/holding_period.rs:31` 的 `enum Period`
（超短 2 日 / 短 5 日 / 中 28 日 / 长 90 日）。`config/llm-output-schemas.json` 与前端 `PeriodKey`
是它的两份门锁定副本，不是并行权威。

### 1.1 名目层：LLM 侧「有档」的地方比想象多

- 分析师层**已经逐档**：23 个 `{base}--{tier}` 实例（`src-tauri/src/commands/stock_analysis_setup/seed_stock_analysis.rs:3296-3331`，条数由播种期自证 `assert_eq!(tiered.len(), 23)`），每档有自己的 VERDICT 契约（`holding_period.rs:56-73` `HorizonVerdictSpec`，档位由**代码盖章**、契约字段清单里刻意不含 `horizon`，见 `:79-87`）。
- `src-tauri/config/llm-output-schemas.json:225-343` 存在一个完整的 `trader_output.decisions.{ultra_short,short,mid,long}` 四档形状（自述「V80 四周期独立决策，与 `decisionsByHorizon` 同维对齐」）。
- `src-tauri/agency_experts/stock-analysis/trader.md:128-136` 明确反向声明：**「持有周期：你不定档」**，LLM 只自报一个 `timeHorizon` 供留档。

### 1.2 执行层：断点在裁决层，不在证据层

链路：逐档分析师 23 实例 → `analyst-brief`（**已按档分组**的文本）→ 辩论（单份）→ `research-mgr`（单份）→ `trader`（单份）→ `portfolio-mgr`（公式四档）→ 仲裁。
- `trader` 是**一个** AgentNode，其逐档输入被硬编码成 mid 代表实例（`seed_stock_analysis.rs:5547-5548`，v152 批注写得很清楚：只挂了 `a-catalyst--mid` / `a-hot-money--mid`）。
- 逐档辩论在种子里被两处标成「后续批次」（`seed_stock_analysis.rs:3748`、`:3827`）⇒ 23 条逐档 VERDICT 在进 trader 之前已被压成一串文本。
- **`decisions` 四档块是纸面契约**：① 它不在 `trader_output.required`（`:78-92` 只列 13 个单值键）；② 该 schema 文件**全仓零运行期消费者**（`grep` 只命中 3 处注释/断言文案），`agent_executor.rs` 六处 `response_format: None` ⇒ 结构化输出根本没下发给模型；③ `trader.md` 从未提到 `decisions`。

### 1.3 数据层：两侧同表不同命（现网实测，2026-10-16）

| 读数 | 结果 |
|---|---|
| `stock_analyses` 有 `horizon_decisions`（公式四档） | 101 行，其中 **100 行正好 4 个键** |
| 有 `llm_decision_json` | 132 行可解析 + **6 行首字符是反引号（非法 JSON）** |
| `llm_decision_json` 含 `timeHorizon` | 131 / 132（单值） |
| `llm_decision_json` 含 `decisions` | **0 行** |
| `strategy_performance`（逐档结局打分线） | **14 行**：`period` 只有 `reflection` 8 / `reflection:short` 3 / `short` 3，超短/中/长 **0 行** |
| `strategy_performance.agreement_score` | **非空 0 行**（三个写点全是 `Set(None)`：`crates/analysis-engine/src/evolution_drift.rs:700`、`:863`、`crates/dao/src/repo/stock_lesson_queries.rs:677`；读点 `get_agreement_score_history` 用 `is_not_null` 筛 ⇒ 该面板结构上恒空） |
| `stock_reflections` 逐档 | 52 / 51 / 33 / 23 + 空档 15（反思**在产出逐档行**，只是多数还没到成熟窗口） |
| `decision_validations` | 765 行（`t_plus_n` 5/20/60 各 255）——这是**荐股池**线，不挂 `stock_analyses.id` |
| `time_travel_batch_runs` | completed 1、interrupted 4 ⇒ 回放喂样本目前极慢 |

### 1.4 一句话根因

> 公式侧把逐档证据做成了**加权数值融合**（`horizon_branch_specs` 的 `legs`），LLM 侧把**同一批逐档证据**在裁决层压回成一段以中期为锚的单值叙事；两者不在同一计数域，所以现有 `compute_decision_agreement` 只能靠 LLM 自报的那一个 `timeHorizon` 去公式侧挑一行来比（`src-tauri/src/commands/stock_workflow/decision.rs:1589-1597`）——**一次比一对，永远不是四对**。

## 2. 可比性前提（必须先立，否则阶段 B 的结论是假的）

公式链的 12 条腿里有 **2 条腿的数值来源就是 LLM 分析师的逐档 VERDICT**（`crates/analysis-engine/src/evidence_weight.rs:441-454` `VERDICT_FACTOR_SOURCES`：`flowPersistence`、`eventCatalyst` 标 `verdict`，其余标 `tierScore` / `sharedTool`；且 `flowPersistence` 是唯一由 LLM 直接给数值并进方向加权的因子，见 `holding_period.rs:150-155`）。

⇒ 两侧**不是独立裁判**。一致率不能读作「两条独立路径互相确认」。
⇒ 处置（待拍板 4）：不剔除、不改算法，而是**在报告里把该档 consensus 腿的权重和算出来当「独立性折扣」**，并另设一列只比「方向动作」这一最粗口径。

## 3. 阶段 A：LLM 决策四档化

### A1 逐档 LLM 决策的产生方式（待拍板 1，本阶段唯一的架构选择）

| 方案 | 做法 | 代价 | 判定 |
|---|---|---|---|
| **甲（推荐先行）** | **单次 trader 调用产出四档**：`trader.md` 增「逐档各给一份决策」契约 + `decisions` 块写进 prompt 要求；解析走 `decisions` | +0 次 LLM 调用；不改图结构（不 bump 图代、不碰播种拓扑 ⇒ 规避 v141 那类首播种炸图）；复用 `analyst-brief` 已有的按档分组证据 | 四份决策出自同一次推理，可能雷同；**「雷同率」本身就是本轮要测的观测量**，测出来再升级 |
| 乙 | 四个 trader 实例（逐档各调一次，可并行 ⇒ 墙钟≈单次） | 每轮分析 +3 次 LLM 调用；改父图（23→27 个 Agent 节点）＋ 图代 ＋ 扇出/边自证；可复用 `agent_executor.rs:887-892` 那条**现为死路**的 `horizon` 契约注入（走 `input_mapping` 路②把它点亮） | 甲暴露「四档雷同」后再接，不预先建设 |
| 丙 | 不新调用，用 23 条逐档 VERDICT 的 `direction` + 权重**确定性合成**四份 LLM 决策 | 0 成本 | **否决**：与公式链共腿，一致率恒高，作验证零信息量；它回答的是「证据层说了什么」不是「决策层怎么选」 |

### A2 落库（拍板 2 = **嵌进 `llm_decision_json`**，推翻我原来的新列推荐）

- 四档结果写进现有 `stock_analyses.llm_decision_json` 的 `decisions.{ultraShort,short,mid,long}`
  （键由 `Period::camel_key()` 现推，与公式侧 `horizon_decisions` 同名同序）。**不加新列。**
- ⇒ 由这条选择派生三条硬约束（他批注原话「必须确保四档完整解析并展示」）：
  1. **解析完整性**：`extract_llm_decision_json`（`decision.rs:1360`）现在只做「content 内层解析 + report 展开 + verdict→action + normalize」四步，
     必须再加「`decisions` 四键逐档取齐」一步，并且**四档任一键缺失要显式登记缺席**，不许静默少一档当成「该档没观点」。
  2. **脏 JSON 面被放大**：该列现网已有 **6 / 132 行首字符是反引号**（整列非法 JSON）。嵌列 ⇒ 这 6 行的四档与主决策一起丢，
     所以解析失败必须落「第三态 + 失败原因」而不是回退成「LLM 没表态」。反引号来源要在本轮查明（多是 \`\`\`json 围栏没剥），
     能剥则剥、不能剥则记因——**剥围栏属于解析层修复，不算改语义**。
  3. **展示完整性**：呈现层必须四档齐出（每档一句，缺席档按 §A3 四态各占一句），
     现在 `src/stores/feature/stockAnalysisStore.ts:620,1476-1497` 那条「公式侧缺失 ⇒ 用 LLM 单值 blob 兜 `decision`（四档就此消失）」
     的路径必须一并改，否则嵌列方案的「四档」会在 UI 上被兜底逻辑吃掉。
- `decision_time_horizon` / `decision_horizon_source` **语义不动**（`trader.md:130-136` 那句「LLM 不参与定档」在落库主档这一层依然成立；本轮改的是「LLM 有没有逐档表态」，不是「谁定主档」）。
- 历史行不回填（拍板 6），按代际标记为「上线前样本」；脏 JSON 那 6 行归「解析失败」态留痕。

### A3 缺席语义（呈现层不得造歧义）

四种状态各占一句，不许合并、不许用 0 顶替：
① 该功能上线前的历史行（列本身为 NULL）；② 本轮 trader 失败 / JSON 非法；③ trader 成功但该档没给（模型漏吐）；④ 公式侧该档分支未产出。
输入侧要显式声明来源，两套计数域（公式四档 / LLM 四档）不得并排显示成可比、也不得收敛为同一个数。

### A4 阶段 A 验收判据

1. **生产侧 + 渲染侧两层测试**：store/落库实际写下哪个键（列里出现四个 camel 键）+ 组件出哪句；两侧各做变异，红条数 = 被破坏那一格。
2. **新门（运行门 + 负控，不许只有编译门）**：LLM 四档键集 ⊆ `Period::camel_key()` 全集且等长 4；负控两条——少一档仍绿、把 `ultraShort` 写成 `ultra_short` 仍绿，都必须变红。
3. **门的第一个读数打在未修的树上**（先证它会红）。
4. 现网读数对账：新列在跑一轮真实分析后必须由 0 → 4 键；拿不到就报「未产出」，不得显示成四档全「观望」。
5. `TEMPLATE_VERSION`（`seed_stock_analysis.rs:1433`，现 160）bump + 现网库代际对照（库里高于/低于代码要四态报，不参与差值）。
6. 新增 UI 文案 → 11 语言全量补 key，且插值槽逐语言回读断言。

## 4. 阶段 B：公式 / LLM 逐档对照验证

### B1 一致性层（改现役函数，不新建第二套）

`compute_decision_agreement`（`decision.rs:1573`）从「挑一行比一对」改成**逐档四对 + 主档一对**：
- 沿用既有 `originalAction` 口径（2026-10-10 B 裁定：逐档行取否决前方向值，缺席才回退 `action`）；
- 档位不匹配不再靠「LLM 自报档」消解——两侧都有四档后，`llm_period → 公式挑行` 那段降级路径（`:1589-1597`）改为「自报档仍保留为主档摘要口径，逐档对照另出 `agreementByHorizon`」；
- 维度沿用现有四刀（方向 / 仓位 / 置信 / 风险档），止损带与出场口径作为**第二格**单列（两侧口径不同源：LLM 侧 `trader.md:207-218` 是范围表，公式侧 `sl_pct_for` 是唯一裁定，直接混进同一分会把口径差记成方向分歧）。

### B2 分歧归因层（复用，不新造）

现有 `divergence_attribution`（`src-tauri/src/commands/stock_workflow/hooks.rs:137-232`）已是「驱动项枚举」形态，但服务荐股 crossCheck 且单期。抽成逐档：每档三行——哪条腿反号 / 哪道门改变动作（`entryGate` 四档各不同：`breadth_required` / `trend_and_flow_same_sign` / `inside_sigma_band` / `positive_margin_of_safety`）/ 哪一侧有缺口。

### B3 结局层（真正的「相互验证」，本阶段的落点）

把同一档、同一窗口下的**公式动作**与**LLM 动作**各判一次对错，出 2×2：

| | LLM 对 | LLM 错 |
|---|---|---|
| **公式对** | 一致且都对 | 分歧·仅公式对 |
| **公式错** | 分歧·仅 LLM 对 | 一致但都错 |

- 对错判据**不新建**：直接接 `deterministic_horizon_was_correct`（`crates/analysis-engine/src/reflection_stats.rs:157`），它已按档给 `was_correct`，未成熟返回 None（不写行，不写 0）。
- 落点：`strategy_performance.agreement_score` 从 `Set(None)` 改成**真值写入**（三个写点同批），历史线 `get_agreement_score_history` 因此第一次有电；逐档另出一张对照结果（形态待定：新表 vs 现表加两列 ⇒ 待拍板 5）。
- **统计口径必须先声明样本现实**：现在逐档成熟结局样本是 超短 0 / 短 3 / 中 0 / 长 0，2×2 每格期望 < 1。⇒ 阶段 B 的交付定义是**仪器装好且能读数**，不是「得出谁更准的结论」。窗口未满时面板显示「样本不足」并给出所需 N，禁止显示 0% 或空表。
- 一致性阈值沿用既有下限先例（腿 `MIN_SAMPLE=5`、腿权重 `LEG_MIN_SAMPLES=30`、IC `IC_MIN_SAMPLE=8`），新列若进任何跨代统计，须在 `scripts/check-generation-scope-registry.mjs` 登记生成代下限（现网既有 `HORIZON_BRANCH_GENERATION_FLOOR=125` 的先例）。

### B4 呈现层

`DecisionComparisonPanel` / `CompactDecisionComparison` 现在是「公式四档条 + LLM 标量」的错位并排（`src/components/stock-analysis/dual-view/DecisionComparisonPanel.tsx`）。改成四行 × 两列 + 每行一致率与归因，缺席四态按 §A3 走 i18n。

### B5 门禁连带面（改一处要同批改另一处）

`check-horizon-decision-keys.mjs`（扩到 LLM 侧生产者↔TS↔解析器）、`check-tier-purity.mjs`、`check-analyst-id-shape.mjs`、`check-variable-consumer-registry.mjs`（新列/新变量必须有非声明消费者）、`check-generation-scope-registry.mjs`、`check-i18n-key-parity.mjs`。
终局按 CI 口径三条：`cargo fmt --all --check`、`cargo clippy --workspace --all-targets --all-features -- -D warnings`、`__TAURI_WORKSPACE__=true cargo test --workspace`（`cd src-tauri` 起跑以命中 pinned 工具链）；前端 `npm run ci:check:quick`。

## 5. 拍板记录（2026-10-16，按编号点完 ⇒ 整批授权）

| # | 事项 | 拍板 | 与我的推荐 |
|---|---|---|---|
| 1 | 四档 LLM 决策产生方式 | **甲**：单次 trader 调用产出四档 | 一致 |
| 2 | 落库形态 | **嵌进 `llm_decision_json`** + 「必须确保四档完整解析并展示」 | **推翻我的新列推荐** ⇒ 派生 §A2 三条硬约束 |
| 3 | 方向一致性口径 | **与现有决策一致** = 沿用现役 `originalAction` 优先、缺席回退 `action`（`decision.rs:1605-1608`），不另立新规则 | 一致（表述换成「沿用现役」） |
| 4 | 共腿不独立性 | 标注独立性折扣 + 单列最粗方向一致率 | 一致 |
| 5 | 逐档 2×2 落点 | `strategy_performance` **加列** | 一致 |
| 6 | 历史行 | 不回填，标「上线前样本」 | 一致 |
| 7 | 窗口口径 | **与四档交易日对齐**（2 / 5 / 28 / 90），逐档各绑自己的天数 | 比我的「加两个 N」更明确 |
| 8 | 逐档辩论 + 逐档风险分析 | **本轮做**，并行节点同步进行（原始设计即四档） | **推翻我的「本轮不动」** ⇒ 见 §7 |

## 6. 范围更正与新增硬约束（拍板 2 / 7 / 8 带来的）

### 6.1 逐档辩论的调用数：我上一条给的「+21 次」是错的

原始设计在 `docs/plans/PLAN-four-horizon-workflow-alignment.md:2606-2612`（§五十四 B3）：
**「辩论也按四档」**——档位集合四档全跑（B2 裁定不得选子集），轮数取 **1 轮/档** ⇒ bull + bear = 8 次，
另加逐档收敛 4 次 ⇒ **12 次/轮**；§九 代价账 `:375-377` 写的正是 `4×(1+1+1)=12`。
现状 `DEFAULT_DEBATE_ROUNDS = 2`（`src-tauri/src/commands/stock_analysis_setup/seed_variables.rs:23`）⇒ 4 辩手 + 1 收敛 = **5 次**。
⇒ **净增 +7 次**，不是 +21。我错在拿「四档 × 3 轮」估，且漏了 B3 已把轮数定成 1 轮/档这条既有裁定。

### 6.2 「四档风险分析」有一半已经做过了

- **已按档**：`cls-risk-level-{ultra-short,short,mid,long}` 四节点（`horizon_tier_template.rs:184-215`，v140 搬进子模板），
  输入 = 7 条全局轴 + **2 条本档轴**（`riskWindows.<camel档>.drawdownDepth` / `.windowDays`），输出被该档分支读作 `overall_risk`。
- **仍单值**：父图三视角风险评估 `p-risk-assess` → `risk-agg` / `risk-con` / `risk-neu`
  （`aggressive-/conservative-/neutral-debator`）→ `risk-convergence` → `agg-risk`，其 `context_sources`
  刻意取 mid 代表实例（`seed_stock_analysis.rs:4665-4690`）。
- R-11 §九 9-3 的逐档清单点名的是「该档风险腿」（已落地），**没点三视角评估**。
⇒ 自行裁定：本轮**不动** `p-risk-assess` 三视角（依据＝原始清单未点名 + 它是跨视角分歧度而非档位判断 + 再 +12 次调用）。
但呈现层必须写清「哪一格已四档、哪一格仍单值」。**要覆盖这条只需一句话，回退成本＝改那一格文案。**

### 6.3 顺带查出的现行缺陷：面板声明与数据层相反（不是本轮引入）

`src/components/stock-analysis/HorizonScopeNotice.tsx:23` 仍把「分析师（10 个节点）」列在**「仍共用一份」**里，
`stockAnalysis.horizonScopeAnalyst`（`src/i18n/locales/en-US.json:15494`）现文是
"all four horizons share this single set of analyst outputs"；而 v133（B2-2）起分析师**已按档展开成 23 个实例**
（`seed_stock_analysis.rs:3296-3331`，播种期 `assert_eq!(tiered.len(), 23)` 自证）。
这正是同一组件文档 `:26-28` 自己列为禁物的「声明落后于数据层」。
⇒ 列为 **A0**，与本轮同批改口并配门（判据：`analyst_subset()` 逐档不同 ⇒ `analyst` 那格不得再说共用）；
§五十四 B4 那条「声明与事实一致」的硬条件同时要求 A4 落地后 `debate` 那格随之改口。

### 6.4 窗口口径（拍板 7）的确切落点

- `strategy_performance` 侧**本来就是按档外推**：`evolution_drift.rs:618` 写 `holding_days = period.default_holding_days()`，
  反思侧 `reflection.rs:3079-3083 horizon_window` 用「日线 bar 位移」实现 ⇒ bar 只存在于交易日 ⇒ 单位天然对齐，无需换算。
  四档天数权威值 = `holding_period.rs:294-301` 的 **2 / 5 / 28 / 90**，注释口径是**交易日**（`reflection_stats.rs:128`、`reflection.rs:2346-2348` P1-9）。
- 要改的是荐股线：`backtest_validation.rs:27,68-69,173` 的 `unwrap_or_else(|| vec![5, 20, 60])` ⇒ **由 `Period::default_holding_days()` 现场推导**，
  不得手抄第二份；`fetch_window_series:370-399` 是 bar 切片（`n = min(t_plus_n, len)`），无需日历换算。
- ⚠ 换代代价（必须先声明再动）：现网 `decision_validations` 已有 **765 行**（N=5/20/60 各 255），幂等键是 `(pick_id, t_plus_n)`
  ⇒ 新 N 集与旧行**共存不覆盖**，而 `compute_validation_report` / `reco_ic_stats` / `reco_loop` 三个读点若把新旧 N 混池，
  就是「两套计数域并排显示成可比」。⇒ 按拍板 6 的代际标记口径处理：**新 N 集自代起算，旧行按「上线前窗口」不并入分母**，
  并在 `scripts/check-generation-scope-registry.mjs` 登记（这三个读点已注册为 `statistics` 类，必须带本域代 marker）。

### 6.5 逐档辩论（A4）落地的四条硬约束

1. **必须是 4 组显式节点，不能用扇出**：`AgentNodeConfig` 无实例/矩阵字段（`workflow_types.rs:267`），`WorkflowNode` 无 fanout 变体（`:1366-1418`），
   `SubWorkflowExecutor::map_inputs` 每节点只发一次子执行（`subworkflow_executor.rs:134-139`），`audit_fanouts` 只匹配 `WorkflowNode::SubWorkflow`（`workflow_fanout_audit.rs:246`）。
   ⇒ 照 23 个逐档分析师实例的既有做法（`seed_stock_analysis.rs:3756` 按 `tiered` 循环建节点建边）。
2. **环约束会复制四遍**：破环规则是在三处排除 `value-investor`（debate 边 `:3756`、brief 边 `:3886`、`ab_input` 键 `:3843`，理由 `:3750-3755` / `:1176-1183`）；
   `:3753-3754` 明确记着「把逐档边接回 `debate-bull-bear` 会被 Kahn 检查拒」。⇒ 新边逐条验环，不许整体套循环。
3. **代 bump**：重播种门是 `existing.version >= TEMPLATE_VERSION` 早退（`:2370`，严格大于）；
   **现网已同步在 160**（`workflow_templates` 主模板 + 四个档分支模板全部 160，2026-10-16 实测）⇒ 本轮图改动 bump 到 **161**，
   并照 `debate_rounds_graph_drift`（`:2373-2379`）的形态为「逐档辩论是否真重展开」补一条同型漂移门。
4. **会被打红的既有断言**：`analyst_brief_keys_match_tiered_instances` 锁 21 键（`seed_consistency_tests.rs:4245,4287`）、
   `check-tier-purity.mjs` R2（块内不得点名另一档）、`check-analyst-id-shape.mjs` R1/R2（逐档 id 只能由 `analyst_node_id` 生成，禁止字面量 `base--tier`）、
   `main_chain_risk_grade_is_tier_scoped_in_code_domain` 锁恰好 1 处 `pm_risk_veto`（`:3287,3300`）。
   ⇒ 门扫的是**字面量**，`format!` 拼出来的 id 会让门静默失明（`horizon_tier_template.rs:141-142,175-177` 已写成规矩）。

### 6.6 执行顺序（拍板后定稿）

| 阶段 | 内容 | 依赖 |
|---|---|---|
| **A0** | `HorizonScopeNotice` 的 `analyst` 格改口 + 判据门（§6.3） | 无 |
| **A1** | `trader.md` 四档契约 + `extract_llm_decision_json` 取齐四键 + 缺席四态 + 脏 JSON 剥围栏 | 无 |
| **A2** | 呈现层四档齐出（含改 `stockAnalysisStore.ts:620,1476-1497` 那条吞掉四档的兜底）+ 11 语言 | A1 |
| **A3** | 门：四档键集锁 + 生产侧/渲染侧两层测试 + 负控 + 首读数打在未修树上 | A1/A2 |
| **A4** | 逐档辩论 4 组并行节点 + 破环 + `TEMPLATE_VERSION` 161 + 漂移门（§6.5） | 独立，但 trader 输入面随它改 |
| **B1** | `compute_decision_agreement` 逐档化（四对 + 主档一对，口径按拍板 3） | A1/A2 |
| **B2** | 逐档分歧归因（复用 `divergence_attribution`，`hooks.rs:137-232`） | B1 |
| **B3** | 结局 2×2：`strategy_performance` 加列 + 窗口绑 2/5/28/90 + `agreement_score` 三处写点闭环 + 荐股线 N 集换代 | B1 |
| **B4** | 面板四行×两列 + 一致率 + 「样本不足」口径（现网逐档成熟样本 0/3/0/0） | B1–B3 |
| **B5** | 门禁终局（CI 口径三条 + 前端 quick 全套） | 全部 |

## 7. 执行状态（每阶段收尾回填）

现网基线读数（2026-10-16 取，动图前已重取一次）：`workflow_templates` 主模板 + 四档分支模板**全部 160**（与代码 160 同步）；
`stock_analyses` 138 行有 `llm_decision_json`（132 可解析 / 6 非法）、`horizon_decisions` 101 行（100 行四键齐）；
`strategy_performance` 14 行（超短 0 / 短 3 / 中 0 / 长 0）、`agreement_score` 非空 0 行；`decision_validations` 765 行（5/20/60 各 255）；
`time_travel_batch_runs` completed 1 / interrupted 4。

| 阶段 | 状态 | 门禁读数 | 与本 PLAN 的偏差 |
|---|---|---|---|
| A0 `HorizonScopeNotice` analyst 格改口 + 门 | **完成** | 该门 4/4 绿；`check:i18n:key-parity`／`check:i18n`／`untranslated`／`placeholder`／`check:eol` 全 0；`dprint check` 0；`tsc --noEmit` 0 错；`oxlint` 0 warnings／0 errors。**门的自证**：把 HEAD（改口前）11 份 locale 喂进同一判据 ⇒ **11/11 会红** ⇒ 非恒绿 | 无 |
| A1 四档契约 + 解析取齐 + 缺席四态 + 脏 JSON 剥围栏 | **完成** | `cargo fmt -p axagent --check` 无 diff；`cargo test -p axagent --lib stock_workflow::decision` **50 passed / 0 failed**（第 1 轮 47/1 红＝实现措辞漏成因，已改；新增 12 条含 2 条代际标记负控）；日志正文取结论（后台回执那句 exit 0 不算证据） | 无偏差 |
| A2 呈现层四档齐出 + 11 语言 | **完成（覆盖面缺口见 §11.1）** | 新组件 8 条 + 邻居两条门共 16 条绿；全量前端回归 **154 文件／1954 条全绿**；`tsc` 0 错、`oxlint` 0／0、`dprint check` 0、`check:i18n:key-parity`／`check:i18n`／`untranslated`／`check:eol` 全 0 | 原计划「同批改 store 那条吞四档的兜底」→ **不改**，理由见 §11.2 |
| A3 门与两层测试 + 负控 | **完成**（R5a/R5b/R5c 扩进 `check-horizon-decision-keys.mjs`，未另造门） | `--selftest` 9 条负控全生效 + 正控清；门禁模式 0；**自证**：HEAD（未修）的五份 LLM 源喂进同一判据 ⇒ 6 条红、R5a/R5b/R5c 三格都命中；公式侧 R1-R4 在 HEAD 上仍 0 红（说明本批改动静没碰另一侧）；`node scripts/ci-check.mjs --quick --skip-rust` **REAL_EXIT=0，全部检查通过（87.4s）** | 键名对账门已建；Rust/TS 两层测试已各就位（12 条 / 8 条） |
| A4 逐档辩论 4 组 + 破环 + 代 161 + 漂移门 | **完成**（后端 v161 已落地，§25 读数；前端呈现面 2026-10-21 补齐） | 前端：`tsc --noEmit` 0；定向 vitest **98 条全绿**；全量 vitest **242 文件 / 2601 条全绿**（EXIT=0）；`dprint check` 0（改动文件）；`oxlint` **0 warnings / 0 errors**；`check:i18n:key-parity`（11 语言 16067 键一致）、`check:i18n`、`check:eol` 全通过。**变异自证**：解析层丢档位 ⇒ 3 文件红 / 5 条失败 / 82 绿；渲染层关档位模式 ⇒ 2 红 / 1 绿（历史兼容条仍绿）；文案回退旧句 ⇒ 1 红 / 3 绿。三处变异均已按 SHA-256 校验恢复 | 前端连带面原清单只列了 `AnalysisDebugPanel` / `decisionInputDiagnosis` / `agentOutput`，**漏了 store 的 `debateRounds` 装配、`DebatePanel`、chat 工作流卡片（bridge）与导出报告** —— 本轮一并补齐：`DebateRound.tier` + 三条解析路径按 (tier, round) 归并 + 卡片档位切换 + 缺席档明说 + 导出按档分组 |
| B1 逐档一致性 | **完成（判定内核 + 生产消费者 + 门）** | 新增 8 条测试；`cargo test -p axagent --lib commands::stock_workflow::decision::tests::horizon` **9 passed / 0 failed**（`running 9 tests` 行证明过滤器真打到；第 1 轮 159/1 红＝**夹具侧**缺顶层 `action`，见 §20.1）；`cargo fmt -p axagent --check` 无 diff、`clippy -p axagent --all-targets -D warnings` 0 告警（同批日志 `output/tmp/b2.log`） | 无偏差：`compute_horizon_agreements` 已被 `core.rs` 消费并嵌 `agreementByHorizon`，门 R5d 锁产端 18 键 ↔ TS |
| B2 逐档归因 | **完成（数据层 + 渲染 + 11 语言码表）** | Rust 6 条 + 前端渲染 6 条 + 码表契约 4 条；`vitest` stock-analysis+lib **129 文件／1500 条全绿**、`tsc` 0 错、`oxlint` 0／0、`dprint check` 0、i18n 三道人机门 + EOL 全 0；门 R5e + `--selftest` 全过；**变异自证 4 个破坏样本各只红对应那一条**（`output/tmp/mutate_b2.py`） | 原 §19 的「刻意没做的半件」已补上：驱动项上屏随本批一起做（7 码 × 11 语言 + `none`/`legContribution`），见 §20.2 |
| B3 结局 2×2 + `agreement_score` 写侧闭环 + 荐股线 N 集换代 | **代码完成，终局读数待取**（现网逐档成熟样本 0/3/0/0 ⇒ 交付定义＝仪器能读数，不是出结论） | 待取：`cargo check -p axagent --all-targets`（构建中，日志 `output/tmp/b3-rust.log`）、全量三条、`cargo test --workspace` | 与 §18 的两处偏差见 §20.3：主档分的**实际写入点是 M1 通道**（不是 `record_performance`）；窗口换代**不加窗口集列**，改用 `Period::is_validation_window` 在三读点筛分母 |
| B4 面板 | **完成（数据层 + 三处呈现 + 11 语言，见 §22）** | `cargo test -p axagent-analysis-engine --lib reflection_stats` **41 passed / 0 failed**（含 B4 三条纯函数判据 + 扩写的 camelCase 键锁）；组件 6 条 + **5 个变异各只红对应那一条**（`output/tmp/mutate_b4.py`）；`tsc` 0 错、`oxlint` 0/0、`dprint` 0、i18n 四道门全 0；`vitest src/components/stock-analysis` **62 文件／417 条全绿**；门 **R5f** 双向负控生效。终局 CI 三条：`fmt` 0、`quick --skip-rust` 0、workspace test 全 target 绿（analysis-engine lib 复跑 724/0）；**全量 clippy 唯一红在他人 `evidence_audit.rs` 的 `manual_contains`**，归属与细节见 §23 | 落点与 §18 的差别：**不新增命令**（复用 `reflection_stats` 的既有 IPC，2×2 与命中率同源分母），详见 §22 |
| B5 门禁终局 | **完成（CI 口径三条全绿 + 前端 quick 全套）** | `cargo fmt --all --check` **EXIT=0**；`cargo clippy --workspace --all-targets --all-features -- -D warnings` **Finished，0 告警**（10m42s，无短路 ⇒ 全 workspace 真被 lint 过）；`__TAURI_WORKSPACE__=true cargo test --workspace --no-fail-fast` **146 个 test target 全 `test result: ok`、8599 条 passed、0 failed**（认 `running N` 行数，不认后台回执）；`node scripts/ci-check.mjs --quick --skip-rust` **REAL_EXIT=0（87.5s）**；`tsc` 0 错、`oxlint` 0/0、`check-generation-scope-registry` 自证 16/16 + 正式 0、`check-validation-window-authority` 自证 7/7 + 正式 0 | 两处连带修复见 §20.5（都不是本批引入的缺陷，但不修门禁跑不下去） |

## 14. B1 的口径决定与一张表的抽取（2026-10-16）

### 14.1 逐档一致性只有三维，且两个分数不可并排

拍板 3 说「与现有决策一致」= 沿用现役方向口径（`originalAction` 优先、缺席回退 `action`），这没有歧义。
但把主档那套**五维 90 制**直接套到逐档会造出一个假数：LLM 逐档行按 schema 只有 7 个字段，
其中**没有** `riskLevel`，而**论据清单是一次调用的全局产出**——

- `riskLevel`：公式侧的按档风险来自 `cls-risk-level-<档>`（工具节点，非 LLM）⇒ 这一维逐档**只有一侧有**；
- `evidence_cited`：逐档挂它 = 把同一批引用复制四遍，是伪指标。

⇒ 逐档只比 **方向 / 仓位 / 置信**三维，内部满分 **65**，归一化回百分制；
`dimsUsed` / `dimsMissing` 随产物一起出去，面板必须把「为什么和主档那个数不可比」说给用户。
**不静默补 0**：补 0 会把「这一维只有一侧有」伪装成「两侧差得远」。
同理三种「无从比对」（LLM 契约上线前的记录 / 该档 LLM 没给 / 公式该档分支未产出）产出 `comparable=false` + 成因句，
`total` 保持 `None` —— 0 分是「一致得零分」，缺席不是。

### 14.2 抽取：方向表 / 阶梯 / 三维分函数各只留一份

`action_score_table`、`lean_vs_wait_pair`、`action_note_of`、`position_score_of`、`confidence_score_of`、
`risk_rank` + `risk_level_score_of` 从 `compute_decision_agreement` 内联体上收为模块级函数，
主档与逐档**共吃同一份**。这张表是 P1-6(2026-09-14) 与 2026-10-10 两次纠错的落点，抄第二份必然漂移。
分值与分支条件一字未改；回归证据是既有那批主档测试（`..._lean_vs_wait_is_not_opposite`、
`risk_rank_does_not_read_mid_high_as_mid`、`compute_decision_agreement_aligns_formula_side_to_llm_horizon` 等），
加上新写的 `horizon_agreement_reuses_the_main_action_table`（断同一对方向在主档与逐档里同分同标签）。

⚠ **记下一笔未还的债**：本文件里还剩**第四份**方向表 —— `f7_free_action_score`（自指诊断用）。
它对「单侧缺失」返回 `None` 而不是 15，语义不同（无从判断 f7 影响度），本批**不合并**；
合并会改变 f7 诊断的显示条件，要单独裁。

### 14.3 B1 收尾必须同时做的接线（否则门禁红）

`compute_horizon_agreements` 目前只有测试在读 ⇒ 非 test 构建会报 `dead_code`，全量 `clippy -D warnings` 红。
所以 B1 与下面这些**必须同批**，缺一不得宣称 B1 完成：

1. `core.rs` 把逐档结果嵌进决策产物（键名 camelCase，禁区 13）；
2. DTO / TS 类型 `HorizonAgreement`；
3. 面板一格：四行 × (公式 / LLM / 归一分)，并显式声明「三维 65 制，与主档五维 90 制不可比」+ 独立性折扣 `sharedEvidenceWeightPct`（拍板 4）；
4. 11 语言新键；5. 组件测试（缺席三态各占一句，每个破坏样本只让对应一条红）。

## 15. A4（逐档辩论）隔离执行体的现场存档

子执行体撞到 turn 上限、result 摘要为空 ⇒ 按磁盘现场判活。它的改动**未验证、未合入主树**，
已整份存成 `output/tmp/a4-per-tier-debate.patch`（308 行，4 文件：`holding_period.rs` +37/-8、
`analyst-brief.rhai` 86、`seed_stock_analysis.rs` +66、`seed_variables.rs` +16），
worktree 在 `.qoder/worktrees/agent-general-purpose-d8508181/`（**未提交，勿删**）。

它落到的设计（写在 seed 的文档块里，比我原提示词更细，值得保留）：

1. **brief 从一份变四份** `analyst-brief--{四档}`：各只吃本档实例，键名去档后缀 ⇒ 一份脚本 + 四份映射
   （把同一份合并摘要复制四遍是 §五十三 明令禁止的伪装）。
2. **四组辩论**：每档 `debate-bull-bear--{档}` 容器 + 该档 `bull-rN--{档}` / `bear-rN--{档}` + `debate-convergence--{档}`。
   ⚠ **容器必须按档各一个**：`DebateExecutor` 按 `debater_steps` **串行**驱动子节点
   （`crates/rt-workflow/src/work_engine/executors/debate_executor.rs:125`），
   一个容器装四档就是把 8 次调用串成一条链，直接违背拍板 8 的「并行节点同步进行」。
   —— 这条是我原提示词里没有的事实，属于该批的关键约束。
3. **轮数语义改为「每档几轮」**：`DEFAULT_DEBATE_ROUNDS` 2 → **1**（B3 明文 1 轮/档），
   `DEBATE_ROUNDS_MIGRATION_VERSION` 154 → **161** 同批抬（不抬门等于没改：`merge_variable_values` 无条件保留旧库值）。
4. **下游逐条改接**：`value-investor--{档}` 读本档收敛；`research-mgr` / `trader` / `portfolio-mgr` / `data-quality`
   四个单值消费点改为四档并注（f2 按所选档 switch，与 v128 `overall_risk_*` / v133 `catalyst_level_*` 同形）；
   `p-risk-assess` 三视角按 §6.2 **不动形状**，只把锚从 `bull-rN`/`bear-rN` 改指 mid 代表实例。
5. 新漂移门 `tiered_debate_graph_drift`（版本门答不了「图是否真按四档展开」）。
6. `TEMPLATE_VERSION` 已 bump **160 → 161**（worktree 内）。
7. ⚠ **会改决策数值**：f2 共识因子自 v161 起取「所选档」的收敛分 ⇒ 跨代比较与仓位须按代筛样，不可与 v160 混算。
8. ⚠ **前端连带面**（该批受禁区约束没改，归主树）：按整串 node id 精确匹配的三处
   `src/components/stock-analysis/AnalysisDebugPanel.tsx:51`、`src/lib/decisionInputDiagnosis.ts:66,194`、
   `src/lib/agentOutput.ts:254` 对带档后缀的收敛节点会**静默失配**（症状＝那格显示原始 id / 诊断表少一行）；
   正解是走既有 `analystBaseOf()` 剥后缀，与 v133 那批同一处置。

**尚未完成**（接手时按序补）：`seed_consistency_tests.rs` 未改（21 键锁、debate 展开参数化、容器清单三处必红）、
漂移门未写、破环的逐条 Kahn 验证未见落盘、四份 brief 的映射与 `analyst-brief.rhai` present 集是否同集未自证、
一次编译/门读数都没跑过。⇒ **A4 状态＝设计已定稿 + 约三分之一实现 + 零验证**，不要当成可合入的成品。

## 9. A1 落地面与两处新发现（2026-10-16）

### 9.1 已改的文件（四处，均未提交）

| 文件 | 改了什么 |
|---|---|
| `src-tauri/agency_experts/stock-analysis/trader.md` | 新增「逐档决策：`decisions`（四档都要各给一份）」章节 + 输出示例里补四行逐档样例 + 字段说明补 `decisions` 条。**刻意保留**「持有周期：你不定档」那节（主档裁定权仍在公式），新章节开头明写两者分工，避免被读成「LLM 现在能定档」 |
| `src-tauri/config/llm-output-schemas.json` | `decisions` 块四键 snake→**camel**、每档补 `required`（7 字段）、补各档止损带（2-4／4-7／6-12／10-15，与 trader.md 的表同值）、撤掉**幽灵字段 `confLowerBound`**、`decisions` 加入 `trader_output.required`；并**改写文件顶层 `description`**——原文写「必须通过本 Schema 验证，否则触发 strict_mode 降级」，而实际 `agent_executor` 六处 `response_format: None`、本文件零运行期消费者 ⇒ 那句是文档级谎报，按现状改成「契约文档 + 三处真强制」 |
| `src-tauri/src/commands/stock_workflow/decision.rs` | ① `extract_llm_decision_json` 拆出可测内核 `llm_decision_json_from_trader`（语义不变的三行拆分，目的是让负控拆在守卫本身）；② 新增 `unwrap_json_fence`（只剥完整一层，半截围栏原样交回＝不猜不补）；③ 新增 `normalize_llm_horizon_decisions`：camel/snake/历史 `ultrashort` 三族拼写归一后**只写 camel 落库**，逐档缺 `action` 时按**顶层同一张** `verdict_to_action` 降维（「不确定」仍不压成「观望」），未知档位键**点名不丢弃**，缺席逐档写进**既有** `data_gaps`（两族拼写谁存在就往谁续，不造第二个缺口清单） |
| `src/components/stock-analysis/HorizonScopeNotice.tsx` + `__tests__/HorizonScopeNotice.test.tsx` + 11 份 locale | A0（见 §6.3） |

新写的 10 条 Rust 测试：四档 camel 直通且零缺口／snake+legacy 归一 + 负控「snake 键不得并存」／整档缺席＝**一条汇总**而非四条／未知键 1+4 条／非对象容器／逐档 verdict→action（期望值按权威现算）／「不确定」不得变「观望」／`data_gaps` 续写且不并存两族／围栏四正两负／端到端一格（围栏包装 ⇒ 落库 camel 两档 + 两条点名）。

### 9.2 新发现一：那 6 行脏 JSON 的真实受害面不是 UI，而是**一致性分数**

`src/lib/agentOutput.ts:656 extractLlmField` 走 `parseJsonLoose`，**前端早就自动剥围栏** ⇒ 那 6 行在面板上看不出来。
真正的受害者是 Rust：`compute_decision_agreement`（`decision.rs:1577`）直接 `serde_json::from_str`，无围栏容错 ⇒ 这 6 行的 `llm_json` 解析失败 ⇒ **整条一致性返回 `None`（静默缺席）**。
⇒ 本轮的 `unwrap_json_fence` 修的是**对照层**，不是显示层；A2 阶段做呈现层时**不要**把它当成「UI 已经修好」的依据。

### 9.3 新发现二：TS 侧本就有两族档位键并存，别并排

`src/types/stock-analysis.ts:1243 PeriodKey = "ultra_short" | …`（**snake**，荐股链 `byHorizon` 用它），
而分析链逐档 `HorizonDecision` 走 camel（`decisionsByHorizon`）。
⇒ LLM 四档必须落在 **camel** 族（与公式侧同域，才能逐档对照）；A2 写 TS 类型时**不得**复用 `PeriodKey` 作键类型，
否则第一个坏样本就是「`ultraShort` 读成 `undefined`」那族已付过代价的失效（见 `decision.rs:702-705` 的 v125 换代记录）。

## 10. A2 动手前必须先定的一件事：历史行怎么被认出来（追加裁定 9，附我的默认选择）

拍板 6 说「不回填、标『上线前样本』」，但**拿什么当判据**要到 A1 落地后才看清：

- 直觉选择 `stock_analyses.template_version` 当分界 ⇒ **不成立**：本轮 A1 只改 `trader.md` 与解析器，
  **图没变 ⇒ 不 bump `TEMPLATE_VERSION`**（md 走 `agency_experts` 的 UPSERT 通道，不在图版本里）。
  重启后新跑的分析行 `template_version` 仍是 160，与历史行**同值**，代际判据会恒假。
- 用时间戳 ⇒ 更糟：回放批次会把 `as_of_date` 往前推，一条决策日在 2026-03 的样本可能今天才入库、
  已经带四档 ⇒ 会被判成「上线前样本」，而它其实是上线后产出的。
- 只用「`decisions` 键在不在」⇒ 与「trader 整档没给」**不可区分**，正是拍板 6 要避免的伪样本来源。

**我的选择（默认执行，除非你改）**：由解析器在落库前自打一个契约标记 ——
`normalize_llm_horizon_decisions` 在 blob 顶层写 `"horizonDecisionContract": 1`。四种状态由此机械可判：

| 读侧看到的 | 判成 | 面板说 |
|---|---|---|
| 无 `horizonDecisionContract` 键 | **上线前样本** | 「该记录产出时 LLM 尚未逐档表态」——不猜、不显示四行空表 |
| 有标记 + 某档键缺席 + 该档被 `data_gaps` 点名 | **该档未给出** | 一句缺席（与下两态可区分） |
| 有标记 + 四键齐 | **四档齐备** | 逐行显示 |
| 整列不可解析（含半截围栏） | **解析失败** | 点名失败原因，不进任何分母 |

代价：多一个顶层键（不参与对照计算，只作代际自证）。
若你更希望**零新键**，替代方案是登记 `LLM_HORIZON_DECISION_GENERATION_FLOOR`（harness 常量 +
`check-generation-scope-registry.mjs` 登记），但它必须绑在某个落库列上才可用——而 A1 没加列（拍板 2），
那条路等于回到「加一列」，与拍板 2 冲突。**因此我按上面的默认执行。**

> ⚠ 实施时序：A1 那次 `cargo test` 读数（正被构建锁挡住）**取到之前不要改 `decision.rs`**——
> 主 crate 编译中改源文件会造出半新半旧的读数。这个标记等读数落地后与 A2 同批加。

（时序已按此执行：标记与措辞改动在同一批，只跑一次主 crate 构建。）

## 11. A2 落地后的两处如实声明（都不算已解决）

### 11.1 覆盖面缺口：条只挂在一条渲染分支上

`LlmHorizonDecisionStrip` 现在只挂在 `DecisionBanner.tsx` 的**全量分支**（紧挨阶段2 四周期面板那一段之前）。
两条已知未覆盖路径：
- `HorizonDecisionStrip` 自己的文件头记着同一族问题：工作区壳层下 `DecisionBanner` 走 `!embeddedInWorkspace`
  之外的嵌入分支，四周期区块全被跳过 ⇒ **LLM 四档条在嵌入分支同样不显示**。
- `dual-view/DecisionComparisonPanel.tsx`（完整版双列并排卡片）拿的是 `CompactDecisionShape`（已摊平的字段子集），
  **不含 `llm_decision_json` 原文** ⇒ 要在那里显示四档，得先把原文接进该 view（DTO 侧多带一列 ~9–12KB 文本，
  而这颗卡片在列表里会被反复渲染）。⇒ 归入 B4 一起判，不在此偷偷塞进摘要 DTO。

**所以「四档已展示」这句现在只对全量分支成立**，别按整条产品面读。

### 11.2 原计划要改的 store 兜底：查完决定不改，另立一条真缺陷（追加裁定 10）

PLAN 初稿写「A2 含改 `stockAnalysisStore.ts:1470-1500` 那条吞掉四档的兜底」。查完代码后**这条判断是错的**：
该兜底只替换 `state.decision`（公式形状的对象），而 `state.llmDecisionJson` 是**并列的独立字段**，
新组件直接读后者 ⇒ 四档并没有被兜底吃掉，无需改。已有测试（`stockAnalysisStore.test.ts:326,337`）
锁的就是这个行为，改它属于无故动刀。

但顺着这条路查出一个**独立于本轮**的呈现层缺陷，登记为待裁定：
> 公式侧 `decision_json` 缺失／空壳／解析异常时，兜底把 **LLM 的单值决策当作「公式决策」装进 `decision`**，
> 面板上「公式视角」那一列于是显示的是 LLM 的结论，且来源不作声明。
> 这与 §1.4「两套计数域不得并排显示成可比」、与「拿不到可以、伪装成正常结果不可以」是同一条禁物。

可选处置（我倾向 ①）：① 兜底时给 `decision` 打来源标记，横幅在「公式」列角上一句「本列取自 LLM 兜底，公式侧未落库」；
② 兜底不再填 `decision`，让公式列直接显示缺席；③ 维持现状。
代价：①＝ store + 横幅两处 + 11 语言一句；②＝会有人看到「决策信息缺失」而历史上正是这条被投诉过（见该处注释原话）；③＝缺陷继续存在。

## 12. 两条工具链口径事实（都是「门禁覆盖面」的一部分，抄命令时连参数一起抄）

1. **`dprint` 也管 `*.md`**：本轮改 `trader.md` 后 `npm run format:check` 直接 flag 它（不是我以为的「只 TS/JSON」）。
   ⇒ 改专家 prompt 的同一批必须跑 `npx dprint fmt <md>`，且 fmt 后**要复验门还能读出该文件里的字面量**
   （`--dump` 显示两处 `decisions` 块仍是 4 键，fmt 没把代码样例折掉）。
2. **门脚本自带 CLI main 时必须有 `import.meta.url` 守卫**：`check-horizon-decision-keys.mjs` 原先被 `import` 就会跑门禁并
   `process.exit(0)` ⇒ 我的「拿 HEAD 源文自证」脚本整段被吞，看起来像「跑过了、没报红」，那是**假绿**。
   已补守卫（`pathToFileURL(process.argv[1]).href === import.meta.url`）并复跑门禁 + `--selftest` 两条确认无回归。

## 16. B1 接线批的实际落点（已做完，读数在跑）

1. **嵌入点** = `core.rs:1313` 那一段（全仓 grep 证实生产侧只有这一处调 `compute_decision_agreement`）。
   新增 `compute_horizon_agreements(...)` 调用 + `obj.insert("agreementByHorizon", …)`，
   且刻意插在 `if let Some(ref ab) = agreement_breakdown` **之外**、`obj` 作用域之内 ⇒
   主档那一对算不出来时，逐档的「无从比对」成因仍然报得出来（不然最需要解释的那类记录反而没解释）。
2. **独立性折扣的数据源已换对**：`shared_pct` 改读**该档行自带的 `legs`**（`source.kind=="verdict"` 或 `role=="consensus"`）。
   原先读 `decision_json.horizonBranchJson` —— 那是注入给 Rhai 的工作流变量，不在落库 JSON 里 ⇒ 生产恒 `None`，
   而**测试夹具喂了一份生产不会有的键**，所以测试绿、功能空。测试夹具已同步改成档行内 `legs` 形态。
3. **前端白名单透传这一环差点漏**（新记的长期事实）：`src/lib/agentOutput.ts` 的 `normalizeDecision`
   是**逐字段挑出来重拼**的，不是展开对象 ⇒ 新字段不写进它，面板读到的永远是 `undefined`，
   与当年 `decisionsByHorizon`「类型断了透传、四档从不显示」同一族失效。已补 + 改了
   `agentOutput.test.ts` 的 `expectedDecision`（三处 toEqual 共用同一个期望对象，改一处即可）。
4. 前端已落：`HorizonAgreement` 类型 + `StockDecision.agreementByHorizon` + `CompactDecisionShape` 同名字段 +
   `HorizonAgreementList` 组件（挂在 `DecisionComparisonPanel` 逐档条下方）+ 5 条组件测试 + 4 个新 i18n 键 × 11 语言。
   读数：`tsc` 0 错、`oxlint` 0/0、i18n key-parity 与 placeholder 全 0、
   `vitest src/lib/__tests__ + src/components/stock-analysis/__tests__` **126 文件 / 1459 条全绿**。
5. ⚠ 本轮 Rust 侧自己踩到的一批（都已修，记下来免得再犯）：
   抽出模块级判据函数后 `ActionKind` 不在作用域（原来那份 `use` 在被抽函数体内）；
   测试里 `mid_row(&compute_horizon_agreements(...))` 传临时量 ⇒ 6 处 E0716（返回值借用自临时 Vec），
   必须先 `let rows = …` 再取引用。两次都是 `cargo clippy --all-targets` 才暴露，`cargo check` 那档不够。

## 17. B2 的规格（读完 `divergence_attribution` 之后才知道它不能直接复用）

`hooks.rs:137-236` 那条既有归因读的全是**顶层决策 JSON 的指针**：`/evidence/factors`、
`/effective_params/action_*_threshold`、`posteriorRaw` / `posterior`、`/valuationApplicability/dcfApplicable`、
`/data_gaps`。而逐档行的键域是**另一套**：`legs[]`（带 `factor/owner/role/bias/weight/source/constraint`，
注意是 **`bias` 不是顶层的 `sigma`**）、`absentLegs[]`、`entryGate` / `entryGatePassed` / `gateBasis`、
`tierVetoed` + `originalAction`、`dataGaps[]`。⇒ 两者读的键**互不重叠**，
所以 B2 是**新写一个 `horizon_divergence_attribution`**（读档行），不是给旧函数加参数硬套，
也不是抄第二份方向表。

逐档归因的驱动项（每档最多五句，顺序固定）：

| 驱动项 | 触发判据 | 为什么只有逐档有 |
|---|---|---|
| `tier_risk_veto` | `tierVetoed==true` 且 `originalAction != action` | 这一档的「分歧」其实是自家档内风控改的，不是两个视角反号 —— 主档层面看不见 |
| `entry_gate_failed` | `entryGatePassed==false` | 四档的门是四套（`breadth_required` / `trend_and_flow_same_sign` / `inside_sigma_band` / `positive_margin_of_safety`），只能逐档判 |
| `negative_legs` | `legs[]` 中 `bias < 0 && weight > 0` 按 `bias × weight` 升序取前 3 | 腿集逐档不同（`horizon_branch_specs` 的 `legs`） |
| `absent_legs` | `absentLegs[]` 非空 | 某档整条腿缺席（如短档 flowPersistence 断链那轮的形态） |
| `llm_tier_absent` | 对照行 `comparable==false` 且成因含「LLM 未给出」 | 把「无从比对」与「比出分歧」分开，不然分母会被缺席污染 |

数据缺口沿用档行 `dataGaps[]`（与顶层 `data_gaps` 同一族，不新建键）。

## 18. B3 的列语义决定（一列一个计数域，不给「恒空面板」留第二份真相）

写侧链读完确认：逐档行由 `evolution_drift.rs:599 build_horizon_row_draft` → `upsert_reflection_horizon_performance:646`
双分支（UPDATE 分支刷 8 个字段、INSERT 分支 `agreement_score: Set(None)`，update 分支**根本不碰** `agreement_score`）。
数据来自 `horizon_results_json` 展开的 `DecisionPerformanceSample{horizon, action, was_correct, return_pct, confidence, template_version}`。

⇒ 列的划分（拍板 5 的具体化，**一列只装一个计数域**）：

| 列 | 语义 | 谁写 | 为什么这样分 |
|---|---|---|---|
| `agreement_score`（既有，现网非空 0 行） | **主档**那一对的五维分 | `record_performance` / legacy 通道 | 修掉 `get_agreement_score_history` 的恒空面板；逐档分数**不得**写进它，否则同一列两种尺子，历史线自证失效 |
| `horizon_agreement`（新，`Option<i32>`） | **该档**三维归一分（65 制 → 100） | 逐档 upsert 两分支同批 | 与主档分不同计数域，只能分列 |
| `llm_was_correct`（新，`Option<i32>` 0/1/NULL） | 同一窗口下 **LLM 该档动作**判对否 | 逐档 upsert | NULL = 无从判定（窗口未满 / LLM 该档缺席），**不是 0** |
| `llm_action`（新，`Option<String>`） | LLM 该档动作原文 | 逐档 upsert | 2×2 的每格要能回查到具体记录，否则统计不可复核 |

判据复用现成的 `reflection_stats.rs:157 deterministic_horizon_was_correct(action, return_pct, within_expected_horizon)`，
对 LLM 侧**同函数同窗口同分母**再跑一次（换分母就要重标定，这是既有铁律）；未成熟返回 `None` ⇒ 不写行、不写 0。

⚠ 上游还得把 `llmAction` 送进 `horizon_results_json`（`stock_workflow/reflection.rs` 的逐档结果装配处），
并把 `agreementByHorizon` 里该档的分数带下来。⇒ B3 是三处（reflection 装配 / draft / 两分支写）同批，缺一个就是半条链。
⚠ 2×2 今天**出不来结论**：现网逐档成熟结局样本 超短 0 / 短 3 / 中 0 / 长 0，回放批次只有 1 条 completed。

## 19. B2 落地状态与「数据到位、渲染未上」的明确分界

已完成：`decision.rs::attribute_horizon_divergence`（读档行键域，六类驱动项）+ `core.rs` 嵌 `divergenceByHorizon`
+ TS `HorizonDivergence` + `normalizeDecision` 透传 + `CompactDecisionShape` 字段 + 门 **R5e**（产端 11 键 ↔ TS 11 字段）
+ 6 条 Rust 测试。构建锁在手里，`cargo clippy -p axagent --all-targets -D warnings` 与 `cargo test --lib stock_workflow` 的
终局读数未取到（当前停在 clippy 编译中）。

**刻意没做的半件**：驱动项**没有上屏**。`drivers` 是结构化码表，渲染要 7 个码 × 11 语言的文案；
在没有码表之前把它显示成原始码（`tier_risk_veto` 这种）就是把内部实现抛给用户，
而硬凑一句中文又会与后续 i18n 键打架。⇒ 数据这一层已经通到 store，
**显示随 B4 的 i18n 那一批一起上**（同一批必须含：码表键 7 个、`gateBasis`/`veto`/`negativeLegs` 的摆法、
以及「归因只读不重算」的那句声明）。在此之前不得声称「逐档归因已完成」——只到数据层。

> **本节的「半件」已在同日的 B2 渲染批补上（2026-10-16）**：码表 7 键 + `none`/`legContribution` 已进 11 份 locale，
> 组件与面板接线、呈现测试与码表契约门均已就位 ⇒ 落点与自证读数以 **§20.2** 为准，本节只保留「为什么当时拆两半」的判据。

## 20. B1/B2/B3 收尾批的实际落点（2026-10-16，追加区）

### 20.1 B1 那一次红在夹具侧，不在判据侧

`horizon_agreement_reuses_the_main_action_table` 头两轮各红一次，两次都是**夹具没给生产形态**：

- 第一轮缺 `timeHorizon` ⇒ 主档那一路（`compute_decision_agreement` 的 3a 对齐）退回顶层公式行；
- 第二轮缺**顶层** `action`/`positionPct`/`confidence` ⇒ 主档拿到「单侧缺失 15 分」，
  而逐档那一路读的是 `decisions[mid]`，两侧比的根本不是同一对。

⇒ 生产形态就是「trader 顶层那份单档决策 + A1 新增的 `decisions` 四档」并列，谁也不替换谁；
夹具必须同时给出两套读键位置，否则「两侧同表」这条对照断言是恒真的空话。
读数：`cargo test -p axagent --lib commands::stock_workflow::decision::tests::horizon` ⇒ `running 9 tests` / 9 passed。

### 20.2 B2 渲染批落点（数据层之外的全部改动）

| 层 | 落点 |
|---|---|
| i18n | 11 份 locale 新增 `stockAnalysis.horizonDivergence.{none, legContribution, driver.<7 码>}`（形态照抄既有 `crossCheck.driver.<码>`，不另立一套）；键数 15939/语言，parity 门全绿 |
| 组件 | `HorizonAgreementList` 收 `divergences`，归因块按 **tier 键**（不是数组下标）挂到各自档行内；`drivers.length===0` 说 `none` 那句，**不得**说成两侧一致 |
| 接线 | `DecisionComparisonPanel` 补 `divergences={view.divergenceByHorizon}`（`normalizeDecision` 的透传在上一轮已就位，缺这一行就永不显示） |
| 呈现测试 | `HorizonAgreementList.test.tsx` 新增 6 条（含「归因整列缺席不得说 none」「未译码直出原码」「按下标对齐会挂错档」三条负控） |
| 码表门 | `crossCheckDrivers.test.ts` 新增一个 describe：七码从 `decision.rs` 的 `row.drivers.push(…)` **现场抽取**（不手抄），断 11 语言非空 + 反向死码 + **译文用到的插值槽必须在组件参数袋里** |

变异自证（`output/tmp/mutate_b2.py`）：M1 永不显示 none ⇒ 2 红（该条 + 对齐那条，二者共用同一分支）；
M2 按下标 ⇒ 1 红；M3 参数袋漏 `from` ⇒ 2 红（码表契约 + 七码那条）；M4 `defaultValue` 置空 ⇒ 1 红。基线全绿。

⚠ 两处**抽取器级**的坑（都是红在我自己的测试侧，不是产品）：
`paramBagKeysFromComponent` 用全局 `indexOf("return (")` 切体会切到组件顶部那条 not-produced 早退 ⇒ 空切片、
除手工补的三个键外全漏；t() 替身把嵌套 `legs` 串 JSON.stringify 后引号被转义成 `\"` ⇒ 断 `"bias":"-0.50"` 恒不命中。

### 20.3 B3 落地面与两处偏差（列语义仍按 §18，未动）

| 层 | 落点 |
|---|---|
| 实体 | `entities/strategy_performance.rs` 加 `horizon_agreement: Option<i32>` / `llm_was_correct: Option<i32>` / `llm_action: Option<String>`，并给既有 `agreement_score` 补「计数域=主档五维、逐档分不得写入」的列注释。可空 ⇒ 声明式对账走 `ADD COLUMN`，无需 DEFAULT，也不用写 migration |
| 产端 | `reflection.rs::build_horizon_results_json` 在**每个档对象**上并排写 `llmAction` / `llmWasCorrect` / `horizonAgreement`；对照行取自 `decision_json.agreementByHorizon`（其 `tier` 是 camel ⇒ 经 `Period` 换算，不做字符串拼接）。`llmWasCorrect` 调 `deterministic_horizon_was_correct` **同函数、同 `net_return_pct`、同 `within_expected_horizon`** |
| 样本 | `DecisionPerformanceSample` 加同三字段（`#[serde(default)]`）；`expand_horizon_samples` 从档对象读，legacy 回退路径**从本行自己那三列带回**（成对出现，去别处另取会与落库时那次判定不一致） |
| 写侧 | `HorizonRowDraft` + `build_horizon_row_draft` 透传；`upsert_reflection_horizon_performance` **UPDATE 与 INSERT 两分支同批**（只写 INSERT 会让首写为 NULL 的行永久停在 NULL）；本通道继续不写 `agreement_score` |
| 主档分 | **偏差一**：`agreement_score` 的实际写入点不是 `record_performance`（其唯一调用方是分析回测，入参没有分析行 id 也没有 LLM 侧 ⇒ 恒 None 是它的能力边界，不是遗漏），而是 M1 通道：`ReflectionM1Draft` 加 `agreement_score`，取自被复盘分析的 `decision_json.agreementBreakdown.total`（回退别名 `formulaLlmAgreement`）。⇒ `get_agreement_score_history` 那句 `IS NOT NULL` 从「全仓无人写」变成有源 |
| 窗口集 | **偏差二**：不新增「窗口集代」列，也不回填。权威收到 `harness::holding_period::Period::validation_windows()` / `is_validation_window()`（由 `default_holding_days` 现场推导），写侧默认值与三个跨行池化读点（`compute_validation_report` / `reco_ic_stats` / `reco_loop`）同判据；旧窗口集 `[5,20,60]` 产的 20/60 行留在库里不计数，N=5 两代同义故留在分母。`compute_validation_report` 新增 `excludedPreWindowSetRows` 计数（该响应目前**无前端消费者**，故只到 DTO，不造 UI 假告警） |
| 门 | 新门 `scripts/check-validation-window-authority.mjs`（R1 权威现场推导 / R2 代码无手抄 / R3 三读点同判据 / R4 写侧默认值那一行取权威），`--selftest` 6 绿 0 红；已在 `scripts/ci-check.mjs` 与 `.github/workflows/ci.yml` 同位逐字同源注册 |

新 Rust 测试三条：`holding_period::validation_windows_derive_from_tier_days_and_reject_the_old_set`
（正断言只锁「与四档天数同序列」+ 条数，**不把 2/5/28/90 再抄一遍**；负控锁 20/60/0/-3/91）、
`evolution_drift` 既有 draft 测试补 B3 六条断言（有键透传 / 缺键 None 而非 0 / 观望判 None 不判错）。

### 20.4 本批尚未取到的读数（不许当作已完成）

> **10-16 收尾更新**：下面第 1、2 条已在 **§20.5** 取到并全绿；第 3–5 条仍是事实（样本要积累、面板要重启、
> 本批未提交、A4 派给隔离执行体）。本节保留原文是为了记下「当时哪些读数还不存在」，不要拿它当现状。

1. `cargo check -p axagent --all-targets` + 主 crate 全量三条 —— 构建中，日志 `output/tmp/b3-rust.log`；
2. `npm run ci:check:quick` 未跑；
3. 现网 `strategy_performance` 只有 14 行、逐档成熟样本 超短 0 / 短 3 / 中 0 / 长 0 ⇒ **2×2 今天出不来结论**，
   本批交付的是「仪器能读数」；面板那格要等样本积累 + 重启重跑反思；
4. 存量行不会自动获得新三列的值（不回填，按拍板 6）；
5. 模板代际：本批未改图 ⇒ 未 bump `TEMPLATE_VERSION`（A4 的 161 仍未开工）。

### 20.5 B5 实跑数与两处连带修复（2026-10-16 收尾）

终局读数：`cargo fmt --all --check` = 0；`cargo clippy --workspace --all-targets --all-features -- -D warnings` = `Finished`、0 告警（10m42s）；
`__TAURI_WORKSPACE__=true cargo test --workspace --no-fail-fast` = **146 个 test target 全 `ok`，8599 passed / 0 failed**；
`node scripts/ci-check.mjs --quick --skip-rust` = 0（87.5s）。日志正文取结论（`output/tmp/b5b.log`、`output/tmp/b5-quick.log`）。

**连带修复一（他人批次遗留，HEAD 上就编译不过）**：`crates/rt-workflow/tests/rhai_syntax_check.rs` 里有
**两个同名** `block_body_after`（两个批次各加一份、都已进 master）⇒ `E0428`，
使 `clippy --workspace --all-targets` 与 `cargo test --workspace` 在该 test target 上直接编译失败 —— 也就是
**全仓终局门禁被挡住**。处置按「格式/命名类可最小中性修」档：把后面那份改名 `block_body_after_decl` 并同步它两处
调用点，两份实现语义一字未动，**只改不提交**。（分档依据：逻辑类不碰，这里不涉判据只撞名字。）

**连带修复二（我自己新建的门失明）**：`check-validation-window-authority.mjs` 的 R4 原先按**单行**匹配
`t_plus_n_list.unwrap_or_else(`，而 `cargo fmt` 把这条链式调用折成三行 ⇒ 门报「找不到默认值语句」，**源码是对的、
检法读不到**。改成按**语句窗口**读（`let t_plus_n_list` 起往下取 4 行拼一起再判），并补 M6 负控
「默认值语句被搬走必须红，不许静默过关」——自证由 6 条增至 7 条，全绿。
这是既有教训的同型复发：**rustfmt 折行 ⇒ 任何按行的判据都要回读相邻行**。

A4 已派给隔离 worktree 子执行体（带 §6.5/§15 全规格 + 禁止清单 + 「holding_period.rs 必须严格 additive」），
主树不因其运行而改动。

## 21. A4 第二轮执行体的磁盘现场与接手清单（2026-10-16，追加区）

第二轮隔离执行体（`.qoder/worktrees/agent-general-purpose-3a3c5d4b/`）**撞到 150 turn 上限中止**，
它的 result 摘要不可信 ⇒ 一切按磁盘现场判。现状＝**实现推进到约八成、验证仍是零**。

磁盘现场（`git diff --stat`）：`seed_stock_analysis.rs` +493/-… （60 个 hunk）、`analyst-brief.rhai` 86、
`holding_period.rs` 45、`seed_variables.rs` 16。整份已冻结成 **`output/tmp/a4-per-tier-debate-v2.patch`（933 行）**，
且 `git apply --check` 打到**当前主树（含本批 B3 未提交改动）= EXIT 0** ⇒ 与 `validation_windows()` 那两处
不重叠，将来合并不会撞车（这条实证值得留着，别下次再试一遍）。

已落地的关键形态（逐条对着 §15 验收过，都在源码里读得到）：

- `TEMPLATE_VERSION` **160 → 161**、`DEBATE_ROUNDS_MIGRATION_VERSION` **154 → 161**（同批抬，门才有效）、
  `DEFAULT_DEBATE_ROUNDS` **2 → 1**（每档一轮）。
- **每档一个容器**：`debate-bull-bear--{档}` + `bull-rN--{档}` / `bear-rN--{档}` + `debate-convergence--{档}`，
  并带 `prune_dangling` 兜底与「装饰节点/Decorative Container」的按档列坐标注释；
  brief 同样按档 `analyst-brief--{档}`。
- 带档后缀的 id **统一走 `holding_period::tier_node_id(base, p.as_str())`**（全仓唯一实现），
  不是 `format!` 现拼 ⇒ `check-analyst-id-shape.mjs` 的字面量扫描面还在。

仍然欠的三件（**没有一件可以跳过，否则 A4 就是「图改了但没人证明图对」**）：

1. `seed_consistency_tests.rs` **一行没改**（该文件不在 diff 里）⇒ 21 键锁、辩论展开参数化、容器清单
   三处既没按新形态改期望、也**没跑过**——现在把它们跑起来大概率是红的（红在哪一侧要先判，别直接放宽判据）。
2. 漂移门 `tiered_debate_graph_drift` **只存在于注释里**（`seed_stock_analysis.rs:1453` 那段设计自述），
   实现零 ⇒ 「版本门答不了图是否真按四档展开」这条还没被钉住。
3. **一次编译/门读数都没取过**：该 worktree 有自己的 `target/`，冷编一次 30–60 分钟；
   所以接手时**先把 patch 打到主树**（增量编译 10–20 分钟）再取读数，别在 worktree 里冷编。

⚠ 合并前必须向用户点明的后果：v161 会**重播种整图**（启动前的播种点，不在启动链路 ⇒ 要重启应用 + 重跑分析），
且 f2 共识因子自 v161 起取「所选档」的收敛分 ⇒ **决策数值会变**，跨代比较必须按代筛样，不可与 v160 混算。

## 22. B4（逐档结局 2×2 面板）的落地面（2026-10-16，追加区）

**不新增命令、不新增取数**：2×2 挂在既有 `reflection_stats` 的 IPC 上（`HitrateStats.horizonConfusion`），
与命中率、失败模式看板共用**同一批筛完代的样本**（`build_hitrate_stats` 里 `filter_by_generation_floor`
之后那一步）。若另起一个命令查询，两处会各有一份「按代筛样」，同一面板上就是两种口径。

| 层 | 落点 |
|---|---|
| 纯函数 | `reflection_stats.rs::build_horizon_confusion(samples) -> Vec<HorizonConfusionRow>`（engine，可单测；判定不重算，`was_correct`/`llm_was_correct` 都是产端在同一份档快照上算好的） |
| DTO | `HitrateStats.horizonConfusion`；行结构 `HorizonConfusionRow` 带 `#[serde(rename_all="camelCase")]`；`compute_reflection_stats`（纯函数）里给空 vec，由取数侧按**筛完代的样本**填 |
| TS | `src/types/stock-analysis.ts` 的 `HorizonConfusionRow` + `HitrateStats.horizonConfusion`（`varyByHorizon` 那类旧写法都没有，直接镜像 Rust） |
| 组件 | `src/components/stock-analysis/HorizonConfusionBoard.tsx`（命名导出；档名走 `HORIZON_T_SUFFIX` → `t(timeHorizonXxx)`，**不直出产端中文名**） |
| 接线 | `ReflectionPanel` 失败模式看板之后新增一张卡（`testId="horizon-confusion-board"`） |
| i18n | 11 份 locale 各 7 键（`horizonConfusionTitle/NotProduced/Insufficient/Cells/Undecided/AvgAgreement/ScopeNote`），两趟法脚本 + 隐形字符/半槽断言 |
| 门 | `check-horizon-decision-keys.mjs` 扩 **R5f**：Rust `HorizonConfusionRow` 字段（按 serde camelCase 折算）↔ TS 字段集，双向各一条负控 |

三条口径写在结构体文档与组件注释里，都是「不写就会被读成别的意思」的那类：

1. `llmUndecided` **不进任何一格**：LLM 缺席 / 窗口未满 / 观望持有这类**非方向动作**都不构成
   「LLM 判错」；折进 `bothWrong`/`formulaOnly` 会让 LLM 胜率凭空变差。
2. 四格只比**方向对错**，不含收益高低；且与上方 `byHorizon` 命中率**不同分母**（那里只统计公式侧）。
3. 「样本不足」复用 engine 既有 `MIN_SAMPLE`（=5），**不新建门槛**；不足时四格只是原始计数，
   面板同屏出「样本不足（已判 N 条）」那句 —— 门槛值不落前端硬编码，避免第二份参数。

**今天的真实读数**：现网逐档成熟样本 超短 0 / 短 3 / 中 0 / 长 0 ⇒ 四档全 `sufficient=false`，
面板只会显示「样本不足」+ 原始计数。这是**仪器就位**，不是结论；要出结论得等样本积累。

## 23. B4 终局读数与归属（2026-10-17 凌晨）

本轮 CI 口径三条 + 前端 quick（日志 `output/tmp/b4-final.log`、复跑 `output/tmp/b4-ae3.log`、`b4-ae-clippy.log`）：

| 面 | 读数 | 归属 |
|---|---|---|
| `cargo fmt --all --check` | **0** | 全仓 |
| `cargo clippy --workspace --all-targets --all-features -D warnings` | **红在 `crates/analysis-engine/src/evidence_audit.rs:375,408`**（`clippy::manual_contains`，两处 `participating_set.iter().any(...)`） | **不是本批**：该文件是并行会话当时正在写的新文件（两轮之间行号 367→375 在动）⇒ 按归属纪律不动它，也不为它改判据 |
| `cargo test --workspace --no-fail-fast` | 全 workspace 141 个 test target；其中 analysis-engine lib **724 passed / 0 failed**（复跑）；另有一次读到 726/3 红 + 一批 `E0463 can't find crate` 的连锁（本仓已知形态：某 target 编译失败后伪装成一片 E0463），**两轮之间那 3 条与 E0463 均自行消失**（并行会话在改自己的文件） | 3 条瞬时红＝他人面；本批的 `reflection_stats` 相关全绿 |
| `node scripts/ci-check.mjs --quick --skip-rust` | **EXIT=0**（88s 级） | 全仓前端 + 脚本门 |
| 定点（我这半） | ae lib `reflection_stats` 过滤 **41/41**；`vitest src/components/stock-analysis` **62 文件 / 417 条**；组件 **6/6** + 5 变异各只红一条；`tsc` 0 错；`oxlint` 0/0；`dprint` 0；i18n 四道门 + parity 全 0；门 R5f 双向负控 + 正式 0 | 本批 |

⚠ **收尾判读的两条纪律（本轮又各踩一次）**：
1. **门禁命令别把输出管进 `grep -E "^…"`**：本轮 `cargo test --workspace 2>&1 | grep -E "^(running|test result|…)"`
   把失败**点名**（`---- name stdout ----`）整段滤掉了，只剩 `failures:` 两行 ⇒ 只能重跑一次才拿到归属证据。
   要截断就截到文件再本地 grep，别在管道里丢证据。
2. **并行会话在同一棵树上跑，读数就要带 mtime 判据**：判「这轮读数算不算」先看被编译的 Rust 文件 mtime
   是否落在编译窗口内（本轮 clippy 两轮之间行号就漂了）。

## 24. A4 收尾：并入主树后的逐层修复与读数（2026-10-17 凌晨）

patch 打到干净基（用户已提交 `b2e5a6b7a`/`0c8a9302e`/`6bf545b1a`）后，**逐层**暴露并修掉五类断裂
（种子套件九轮收敛：**25 红 → 8 红 → 1 红 → 87/87 全绿**，日志 `output/tmp/a4-test{1..9}.log`）：

| 层 | 断裂 | 后果 | 修法 |
|---|---|---|---|
| 建边层 | 5 条边仍指裸 id（`bear-rN` 与 `debate-convergence → data-quality/portfolio-mgr/research-mgr/trader`） | 种子**自身**的悬空边守卫拒绝播种 ⇒ 25 条「先播种再断言」的测试连锁红 | t-scoring 接**代表档 mid**；四消费点**四档并注**（逐档边循环） |
| 变量路径层 | 8 处 `input_mapping`/`context_sources` 裸引用（风险官 Kelly / data-quality / research-mgr / trader / portfolio-mgr 的 f2） | **不建边 ⇒ 守卫抓不到**，运行期静默取 `()`（五处一起降级，零报错） | 统一取 mid 代表档收敛节点（与文件内既有「代表实例 mid 优先」规则同口径）；id 走 `tier_id` 现场拼 |
| 解析层 | 2 处「按纯数字后缀解析辩手轮号」（生产 `debate_rounds_graph_drift` + 测试 helper） | 带档 id 被当成非辩手 ⇒ 生产门**每次启动恒报漂移**（重建整图、抹编辑器手改）；helper 恒 0 | 先按 `TIER_ID_SEP` 截轮号段再 parse |
| 作用域层 | 一处绑定误插进 `vec![]` 宏、一次落错同形块 | 编译错（`no rules expected let` / E0425） | 按行号手术挪到块首；自证「绑定 5 处、`let rep_conv =` 计数」 |
| 快速链层 | `FAST_REQUIRED_NODE_IDS` 仍要裸 `analyst-brief`；H1 判据断言源图含裸 id | 派生报「源图缺少必备节点」⇒ 8 条快速链测试红 | 照 `t-news-data` 先例**自建**裸 id 节点（`fast_analyst_brief_node`，id/output_var 不变，脚本与映射由 ④ 步覆写）；H1 判据改为「**四份逐档 brief 都仍是原脚本**」 |

**三件欠账（§21 列的）全部落地**：
1. 漂移门 `tiered_debate_graph_drift`（四档 × 四类：摘要/容器/首轮辩手/收敛，缺任一即报）开在
   `debate_rounds_graph_drift` **同一个建图求值点** + 5 条测试；另有真播种断言
   `seeded_graph_expands_debate_families_per_tier`（16 个节点真在库里）。
2. `analyst_brief_keys_match_tiered_instances` 按 v161 重写：脚本 9 条 `present()` == 四档子集短名**并集**
   + 「四档键集不全同」（防「一份摘要复制四遍」的 §五十三 伪装）。
3. **破环**：既有 `seeded_main_graph_is_acyclic` 就是整图 Kahn —— 四组新边的环验证由它承担，
   本轮 87/87 全绿即已覆盖（未另造逐边脚本：整图门是同一判据的更强形式）。

**读数**：种子套件 `cargo test -p axagent --lib stock_analysis_setup` ⇒ `running 87 tests` / **87 passed**；
`check-analyst-id-shape`（R3 25/27 上限约束）/ `check-tier-purity`（四档纯度 + 旧形归零）/ `check-horizon-decision-keys`
（R1–R5f）三门全绿。全量 CI 三条与前端 quick 见 §25。

**待拍板（本轮我的选择，需你确认或改口）**：f2 / Kelly / research-mgr / trader / data-quality 五处单值消费点
**统一取 mid 代表档的收敛分**，而非 §15 原稿的「f2 按所选档 switch」。按档 switch 要往 `portfolio-mgr.rhai`
的决策数学里加一层选择（并牵动 `f2` 的语义：长档决策该看哪一档的辩论共识），属新增语义 ⇒ 未在无裁定下顺手做。

**合并后果（需在重播种前点明）**：`TEMPLATE_VERSION = 161` ⇒ 下一次分析前的播种点会**重展整图**
（`DEBATE_ROUNDS_MIGRATION_VERSION` 同批抬到 161，一次性迁移门把 `debate_rounds` 归到新默认 1）；
四档辩论各 1 轮（净 +7 次 LLM 调用）；f2 源改代表档后**决策数值会变** ⇒ 跨代比较必须按代筛样，不可与 v160 混算。

## 25. A4 终局读数（2026-10-17 凌晨收口）

| 面 | 读数 | 说明 |
|---|---|---|
| `cargo fmt --all --check` | **0** | |
| `cargo clippy --workspace --all-targets --all-features -- -D warnings` | **`Finished` / 0 告警**（8m33s） | 含 patch 与本轮全部新代码（漂移门/自建 brief/快速链改动/测试） |
| 种子套件 `cargo test -p axagent --lib stock_analysis_setup` | **87/87** | 九轮收敛（25→8→1→0）见 §24 |
| `data_quality_placeholder_gate`（rt-workflow 集成门） | **42/42** | 它按**字面元组**扫 input_mapping ⇒ 把映射改写成表达式形态会被判「无来源」（见下） |
| `cargo test --workspace --lib --no-fail-fast` | **42 targets 全 ok / 8087 passed / 0 failed** | CI 那条 `--workspace` 的**等价替代**，不含各 crate 的 `tests/*` 集成目标 |
| `node scripts/ci-check.mjs --quick --skip-rust` | **全部检查通过** | 含前端注入覆盖审计（它同样按字面元组扫） |
| 三个定向门 | `check-analyst-id-shape` / `check-tier-purity` / `check-horizon-decision-keys` 全绿 | |

⚠ **CI 那条 `cargo test --workspace` 本轮未取到**：`failed to remove file target/debug/axagent.exe —
拒绝访问 (os error 5)` ⇒ 运行中的应用锁住 exe，构建中止（0 条 test result）。这是环境态不是代码态
（clippy 与 fmt 都在同一次运行里编译过全部源码并全绿）。**待关掉应用后补跑那一行**即为终局。

⚠ **本轮我自己的一个返工（记下来防复发）**：为躲 `check-analyst-id-shape` 的「手抄带档 id」，
我把 5 处映射路径改成 `rep_conv_path.as_str()` 表达式形态 —— 结果被两道**按字面元组扫描**的门
（rt-workflow 的 `EXTERNAL_VARS` 对账、前端注入覆盖审计）判成「该变量无注入来源」。
树内既有 `"a-catalyst--mid.content.verdict.catalyst_level"` 这类**路径**字面量（v133 先例）⇒
id-shape 门只拦「整串就是节点 id」的字面量，**不拦路径里的 `--档`**。判据：**改门之前先看门扫什么形态**
（按整串 id 还是按 `(字面 key, 字面 path)` 元组），别凭名字想象。

## 26. 现网失效根因收口（2026-10-17 午后，按**对象级**读数重做归因）

### 26.1 先撤回一条错判

§25 之后我对「分析决策跑不出来」给的归因是 **LLM 提供商 429**。那条对 09:07 那批成立，
对**用户点名的这条记录不成立**。按用户要求改查对象级现场（301302 最新一行
`8706c500-52c5-4ca2-9e9d-63d1d72c9c0c` / 执行 `079f73cd-eb1b-4d31-973e-51d3dcff0137`）：

| 读数 | 事实 |
|---|---|
| 该执行节点总数 | 87 个，其中**只有 `t-sentiment-data` 失败**（重试 3 次：+0s / +1.25s / +2.5s） |
| 失败原因 | `Vendor error from guba: 股吧 HTML 中未找到 article_list 数据, stock_code=301302` |
| 该轮 LLM 侧 | **全部成功**：四档分析师、四档 `analyst-brief--档`、四组辩论（bull/bear-r1）、四份 `debate-convergence--档`、`risk-*` 三视角、`research-mgr`、`trader` 全 completed |
| LLM 四档产物 | `llm_decision_json` 12690 字节，四键齐全：ultraShort=观望 / short=减持 / mid=观望 / long=卖出 |
| 公式侧 | `portfolio-mgr` **在 node_executions 里根本不存在**（状态 Pending）⇒ `agreementByHorizon` 四档全部 `formulaAction: null` |
| 面板表现 | 决策=「数据缺失」哨兵、证据引用审计「不适用（本轮决策未产出）」 |

⇒ **结论：这一条不是 429，是单个数据维度取数失败把终审掐死了。**

### 26.2 两个独立成因，各自带证据

**成因①（我的改动造成，量级最大）**：`stock-analysis-rerun` 补跑批次**没有真正串行**。
`rerun_one` 等的是**来源行 id**，而回放路径不改来源行（同日无回放行 ⇒ 新建 UUID 版本，
`stock_workflow/core.rs:591`），于是 `wait_analysis_settled` 第一次读就判「非 running」立即返回
⇒ 整批毫秒级 tick 完。库内实证：`workflow_executions` 在 09:07:04–09:07:47 之间出现
**59 个父 DAG**（input 形态 `{"analysis_id":…,"as_of_date":"2026-10-15"…}`，as_of 逐行不同 ⇒
就是补跑取件而非单日期全池回放）；`cron_job_history` 里 09:04–09:14 只有其它任务 ⇒
发起方是启动钩子。**35 条中断行 × 3 轮 = 105 次执行**（算术对上），其中 71 条连股吧都正常、
纯粹死于 429，**0 条产出决策**。

**成因②（先于我的改动存在，但被①放大成必然）**：`t-sentiment-data → raw-data → portfolio-mgr`
这条硬依赖。逐版快照核对：该两条边在 `stock-analysis_v86 / v154 / v160` **都存在**，
不是我 A4 引入的；但 10-16 及之前**没有任何一次** `t-sentiment-data` 失败
（按天统计 `runs_sent_failed`：10-17 之前全为 0），所以缺陷今天才第一次现形。
而 `portfolio-mgr` 自己那格 `continue_on_fail: true` 在这里完全不生效 ——
引擎只对 **Failed** 上游放开容错，卡在 Pending 的 `raw-data` 不是 Failed。

股吧本身没坏：本机 `curl` 对同一 URL、三种 UA（含我们那串截短 UA）全部 200 且
`article_list=` / `var other_list` / `"re":[` 三个锚点齐全 ⇒ 解析判据仍然成立，
失败是**被 09:07 那批并发打到限流**（与 `push2*` 同一族「累积/速率触发」）。

### 26.3 本轮落地的三处修复（只改不提交）

| # | 落点 | 改动 | 判据/负控 |
|---|---|---|---|
| F1 | `commands/stock_analysis_rerun.rs` | 等待目标改为 `run_stock_workflow_inner` **返回体里的 `analysisId`**（纯函数 `settle_target_id`），缺键/空串退回来源行 | 3 条：正向锁「等真正落库那条」、负控「缺键不得返回空串」、接线锁 `rerun_one` 内不得再出现 `wait_analysis_settled(db, &row.id` |
| F1b | `crates/dao/src/repo/stock_analysis_rerun_queue.rs` | 新增 `resolved_source_ids`：**计数与取件共用同一谓词**，已有子版本（completed 或仍 running）的来源行退出批次 | `#[tokio::test]` 建真库（`create_test_pool`）断言取件集合 + 「剩余计数 == 取件条数」；摘掉排除即红 |
| F2 | `seed_stock_analysis.rs` | `raw-data.continue_on_fail` false → **true**，`TEMPLATE_VERSION` 162 → **163** | 安全性实证：`raw-data` 的聚合产物 `raw-data-aggregated` 在 v162 图里**零消费者**（按节点 JSON 全文查，除它自己以外无任何节点引用）⇒ 这条例外只是调度前置，不进出路数值；同图 44/97 节点早已开着这格 |

F2 **不改决策数值** ⇒ 不引入新代际，`HORIZON_BRANCH_GENERATION_FLOOR` 不动。
另清掉一处会腐烂的引用：快速链 `jev_aggregator_node` 的文档原文写「与源图 `agg-risk` /
`raw-data` 同」，源图那格已翻 ⇒ 改为只引 `agg-risk` 并点名别再引 `raw-data`。

### 26.4 运行时读数（正门）

12:38 应用被看门狗按新码重启后，补跑**只起了 1 条**（`acf6a107…` / 执行 `workflow_faba6f2c…`，
stock 603466、as_of 2026-10-14，status running），而不是 09:07 那样 43 秒 59 条 ⇒ F1 的串行
恢复在现网取到了读数。**这条落库后要验**：`horizon_decisions`（公式四档，v158 实测 6.9–7.2 KB
有内容）与 `llm_decision_json.decisions`（四键）同时非空、`agreementByHorizon[*].formulaAction`
不再是 null。

### 26.5 新增待裁定（本轮刻意没顺手做）

- **P-a 风险支同型硬阻断**：`agg-risk`（aggregator）`continue_on_fail: false`，其上游
  `risk-agg / risk-con / risk-neu` 与 `risk-convergence` 也全是 `false` ⇒ 任一风险视角 LLM 失败
  会以**完全相同的机制**再掐一次终审。F2 只解了数据支那一半。要不要把「决策链上的聚合节点必须容错」
  写成一条枚举全部 Aggregator 的门，请拍板。
- **P-b 逐档链的容错语义**：`debate-convergence--{档}` 为 `false`，被失败的 `bull-r1--{档}` 阻断 ⇒
  放开它等于「该档辩论没辩成也要出一份收敛」。这会让四档里出现**结构性的该档缺席**，
  属语义变更（LLM 侧已有 `horizonDecisionContract` 四态可表达），需单独裁定。
- **P-c 429 的 provider 侧退避**：工作流 agent 节点走 `llm_executor::execute_llm_stream`，
  **不经** `RetryPolicy::default_llm()`（那条只被 react_engine / init 消费），也不解析
  `Retry-After`；`request_delay_ms` 在 `rt-workflow` / `stock_workflow` 下**零消费点**（声明未接线）。
  本轮靠 F1 消除突发并发，但单条分析约 30 次调用的配额余量属产品口径，待裁。

## 27. P-a 落地 + 把「行不收口」的归因更正（2026-10-17 13:0x）

### 27.1 上一节的一处归因说错了，这里改口

§26 结尾我把 35 条 `interrupted` 存量说成「09:07 那批被应用重启打断留下的」。
现场读数不支持这个说法，真机制是**补跑批次自我繁殖**：

- 同一行 `d3a74ec8`（603466）在 12:38:21 / 12:45:35 / 12:51:41 被连跑 **3 次**，
  每次间隔仅 2 秒 —— 即上一次落库刚写完 `completed`，下一个驱动立刻把它改回 `running`。
- 驱动不止一条：启动钩子 `start_interrupted_analysis_rerun`（`init/services.rs`）
  与内置任务/系统兜底轮走的 cron executor **住在同一进程**，而
  `EXTERNAL_RERUN_LOCK_FILE` 那把锁只防离线 bin，防不到同进程的两次 spawn。
- 每条驱动都在起点载入**同一份** 35 行清单、按同一 `(analysis_date, stock_code)` 顺序前进 ⇒
  lockstep：整批被重跑 3 遍，且每一遍的终态都被下一遍覆盖成 `running`；
  下次启动回收再把 `running` 订正成 `interrupted` ⇒ backlog 永不收敛。

⇒ §26 里「persist 没跑」的怀疑是错的：persist 跑了、写对了，是**被后面的重跑盖掉的**。

### 27.2 本轮追加的两处修复

| # | 落点 | 改动 | 依据读数 |
|---|---|---|---|
| F3 | `stock_analysis_rerun.rs` | `run_rerun` 开头取 `RERUN_SINGLE_FLIGHT`（`OnceLock<tokio::sync::Mutex>` + `try_lock`），拿不到就整批让路；让路时 `pending_at_start` 与 `still_pending` 填同一个数 ⇒ `is_success()` 为假，不会把「什么都没做」记成成功。另在**每条发起前**重取状态，不在本轮可重试集合内的直接跳过 | 挡「同进程重复批」与「清单过期」两种竞态，两者不是同一种 |
| F4 | `seed_stock_analysis.rs` + `mod.rs` | `agg-risk.continue_on_fail` false → **true**；`TEMPLATE_VERSION` 163 → **164**；新增门 `zero_consumer_aggregators_are_tolerant`（按播种落库产物枚举**全部**聚合器，零消费者必须容错，并断言现场 ≥2 个防空集恒真） | 现图全文查：除自身外没有任何节点引用 `agg-risk`/`risk-aggregated`（0 命中），`risk-convergence` 的 `context_sources` 与 `input_mapping` **都是空** ⇒ 与 `raw-data` 同型，放开不进出路数值 |

### 27.3 还剩什么（按能不能纯代码推进分档）

**纯代码可做、已排期**
- **缺席声明（本轮最大的一笔欠账）**：F2/F4 放开连坐之后，「这一路取数失败」在面板上
  **不再有任何声明**（`diagnostics` 只在决策=哨兵时才构造，见 `decision.rs:1247`）。
  现网读数：`data-quality` 的 `input_mapping` 只有 **29 键**，其中只吃 **3 路** `t-*`
  （`t-hotmoney-data` / `t-scoring` / `t-catalyst-data`），另外 9 路取数它根本看不见 ⇒
  已有的 `upstream_data_gaps` 只覆盖 DCF 族。
  实施面四处联动，缺一处就是假声明：① 种子 `input_mapping` 补 12 路（**必须写字面元组**，
  改不成 `format!` 拼装）；② `data-quality.rhai` 的 `upstream_data_gaps` 按 `present()` 逐路枚举；
  ③ 两道按字面元组扫描的门同步（`rt-workflow/tests/data_quality_placeholder_gate.rs`
  的 `EXTERNAL_VARS` 手抄清单 + 前端注入覆盖审计）；④ 11 语言文案。
  ⚠ 处置纪律沿用脚本内已写明的「只告警不扣分，不进 score / grade / factor_completeness」。

**要运行时读数**
- 四档对照至今**没有一个「两侧同时有键」的样本**：公式侧 `horizon_decisions` 正常（6.9–7.2 KB），
  LLM 侧四键在 301302 那次也齐，但唯一一次跑完的 603466 被 §27.1 的 lockstep 盖掉。
  v164 重播种 + 单飞生效后需真跑一条并读 `agreementByHorizon[*].formulaAction` 非 null。

**要拍板（未动）**
- P-b：`debate-convergence--{档}` 被失败的 `bull-r1--{档}` 阻断 ⇒ 放开等于「该档没辩成也出收敛」，
  是语义变更。
- P-c：429 的 provider 侧退避（工作流 agent 节点不经 `RetryPolicy::default_llm`、不解析
  `Retry-After`；`request_delay_ms` 在工作流路径零消费点）。
- 单轮条数：`RerunConfig::default().max_per_run = 0`（不限）⇒ 串行后 35 条 ≈ 4 小时 + 每条约 30 次
  LLM 调用；`pool-scan` 的先例是 20。要不要给补跑也设上限，等一句话。

## 28. 缺席声明落地（2026-10-17 午后）—— 改的是既有通道，不是新字段

### 28.1 一处自我纠正：差点造出第二套同义字段

第一版把失败节点写成 `decision_json.diagnostics.toleratedFailures`。查渲染面时发现
`data_gaps` **早就两头都通**：`portfolio-mgr.rhai` 写、`agentOutput.ts:403` 解析成
`decision.dataGaps`、`DecisionBanner.tsx:1043` 触发 `DecisionTrustNotice` 渲染并计数，
且该组件的文档注释把语义写明为「**只装本该拿到的数据没拿到**」——正是本轮要声明的东西。
另立一套 = 禁区 12 的重复定义，且前端要改三个文件 + 11 语言。⇒ 改回并入 `data_gaps`，
呈现层与文案**零改动**。

条目形态跟既有字形对齐：`节点本轮执行失败(t-sentiment-data)：<错误摘要 ≤180 字符>`
（既有条目形如 `资金流向(t-hotmoney-data)`，节点 id 一律放在括号里）。

### 28.2 落地点

| 层 | 位置 | 内容 |
|---|---|---|
| 生产 | `decision.rs::tolerated_failure_notes` | 从 `Workflow.node_states` 筛 `Failed`，按 node id 排序（`HashMap` 不排序会让同一轮两次落库逐字节不同 ⇒ flaky）；错误串按**字符**截断 180（按字节切中文会切出非法 UTF-8） |
| 生产 | `decision.rs::with_tolerated_failures` | 接在 `extract_decision_json` 的链尾成功分支；**判重按节点 id 是否已被脚本报过**（同节点堆两条会被数成「2 处缺失」）；无失败时原串逐字节不动，不凭空造 `data_gaps` |
| 生产判据 | 3 条 | 「失败节点必须进 data_gaps 且只列 Failed」「无失败不得写该键」「同一节点不重复登记」 |
| 呈现 | `DecisionTrustNotice.test.tsx` 新增 1 条 | 后端那种形态真能被渲染出来（钉呈现，不只是钉生产侧写了字符串） |
| 边界 | 函数文档内注明 | 对话直执行链的 `HookOutcome`（`workflow_lifecycle.rs:65`）只有 `status`/`results`、**没有节点状态** ⇒ 那条链无法在此声明；补齐属 harness DTO 变更，未列入本轮（宁可少一条链有声明，也不造恒空的假声明） |

### 28.3 读数

| 面 | 读数 |
|---|---|
| `cargo fmt -p axagent` / `--check` | **0** |
| `cargo test -p axagent --lib stock_analysis`（种子 + 补跑，含负控版新门） | **107 passed / 0 failed** |
| 新门自带负控 | 把 `agg-risk` 容错位改回 `false` ⇒ 同一个纯函数只点名它，且聚合器集合不变 |
| dao 取件门变异周期 | 绿(2/2) → 摘掉 `resolved_source_ids` 排除 ⇒ **红 1 passed/1 failed** → 还原 ⇒ 绿(2/2) |
| `npx vitest DecisionTrustNotice` | **14 passed**（含新增那条） |
| dprint | 该文件已格式化并 check 通过 |
| `cargo test -p axagent --lib stock_workflow::decision` | 见下一节补记（构建锁排队中） |

## 29. 三条全量门禁的逐条读数（2026-10-17 15:0x）

| CI 那条 | 读数 | 说明 |
|---|---|---|
| `cargo fmt --all --check` | 第一次 **EXIT=1**（两处都在 `stock_workflow/decision.rs:1064`/`:1092`，是**我的**）→ `cargo fmt --all` 后 **EXIT=0** | ⚠ 自我纠正：我在改完条目形态（`data_gaps` 那版）之后**没有重跑格式化检查**，拿的是改之前的「fmt 0」读数。不是覆盖面问题，是「落笔后没重取读数」。 |
| `cargo clippy --workspace --all-targets --all-features -- -D warnings` | **EXIT=0** | 全 workspace lint 干净（此前卡在他人那条 NaN 型 lint 上，现已不阻塞） |
| `cargo test --workspace` | **未取得**：`error: failed to remove file …target\debug\axagent.exe — 拒绝访问 (os error 5)` ⇒ 构建在链接主二进制前就中止，`0` 条 `test result` | 用 `cargo test --workspace --lib --no-fail-fast` 代跑（不重链 bin ⇒ 不吃 exe 锁）。这是**更窄的读数**（不含各 crate `tests/` 集成目标），不能顶替 CI 那一行。 |

改动面确认：`git status` 共 11 项（9 个 Rust 文件 + 1 个新建 dao 文件 + 1 个 tsx 测试），
`cargo fmt --all` **没有**波及任何他人文件。

## 30. P-b / P-c / #8 落地（2026-17 午后，`TEMPLATE_VERSION` → **165**）

### 30.1 P-b：把「逐点补容错」换成一条图级规则

三次复发是同一条链：`raw-data`（v163）→ `agg-risk`（v164）→ 四档辩论链（v165）。
现网实证（09:07 批量 58 次运行）：8 个辩手节点全 `failed`，而 `debate-convergence--档`
与 `value-investor--档` 在那些运行里**一次都没出现**＝被 Pending 卡住从未执行。

引擎语义决定了「只开收敛那一格」不够：`continue_on_fail` 查的是**下游**节点，且只对
**Failed** 上游放行 ⇒ 链上留一格 `false`，它一被卡成 Pending，下游所有容错都救不了终审。
所以本批不再逐点写：`seed_stock_analysis.rs::harden_decision_chain` 在种子末尾做一次遍历
（`portfolio-mgr` 的传递祖先 − 唯一上游是 `trigger` 的 12 个取数节点 ⇒ 实测 **24 格**），
一次写 `continue_on_fail`，**不动边/id/映射/脚本 ⇒ 决策数值与代际不变**。
实现拆成三个函数，BFS 只有一份（`ancestors_of` → `decision_chain_single_points` → mutator），
门与种子共用，禁止各写一遍。

缺席不静默的配套：Failed 节点由 v163 的落库侧声明（`decision.rs::tolerated_failure_notes`
写进既有 `data_gaps`），加上辩手/分析师 system_prompt 原则 2/4 的「诚实标注缺失」要求。

门 `no_single_point_blocks_portfolio_mgr`（种子测试模块，复用 `seed_both` / `parse_nodes` /
`parse_edges`）三段：违规格必须为空、祖先数量 > 40（防空集恒真）、负控把
`debate-convergence--mid` 改回 `false` 后**同一个纯函数**必须只点名它。

### 30.2 #8：执行行收口（两端各一处）

- 引擎：`complete_execution` 原来要求「内存 `executions` map 里还有这次执行」才写库，
  未命中返回 `NotFound` 而调用点 `.ok()` 吞掉 ⇒ 现网 85 条幽灵 `running`。
  现在内存缺失也写终态（`node_executions` 只活在内存，按 repo 的「None 不动列」留原值、不伪造）；
  调用点改成 `tracing::error!`。集成测试 `complete_execution_persists_terminal_status_when_memory_state_is_gone`。
- 启动：`restore.rs::reclaim_stale_workflow_executions`（值用 `interrupted`，不用 `cancelled`——
  那在本表是「用户主动取消」的专有意义）；⚠ **本表时间列是秒**，分析表是毫秒，
  判据测试的最后一段就是钉这个单位（把 cutoff 传成毫秒会连正在跑的新行一起误标）。
- 呈现：`interrupted` 进 `workflowStatusI18n` + 11 语言（复用各语言已有「已中断」译文，不新造）；
  回放面板加一条断言「走译文键且不再转圈」。
- 测试竞态：同文件两个用例都要**进程级**注册替身仓储（全局替换），并行时互相覆盖 ⇒
  加 `REGISTRY_GUARD`（`tokio::sync::Mutex`）串行化。首轮红就是这么来的，不是实现错。

### 30.3 P-c：本批只做「有效的那一格」

工作流路径**唯一真正生效的退避**是节点级 `RetryConfig`（`RetryPolicy::default_llm()` 只被
react_engine/init 消费；模型级 `request_delay_ms` 长在 `AxAgentApiClient::stream`，工作流不经那里
⇒ 全仓零消费点）。而该配置自己的注释写着「429 限流可持续 30s+」，退避却只覆盖 3+6+12=**21 秒**
⇒ 每次都在窗口内放弃。本批 `max_retries` 3 → **5**（3+6+12+24+48=93 秒）。

刻意**没做**：把节流搬进统一入口 `execute_llm_stream`。它是终态位置，但要同时删掉客户端那处
（否则双倍 sleep）并改 5 个调用点，属另一次带验证的改动 ⇒ 不与本批混做。
同样没做：全进程 LLM 在飞上限（嵌套等待有死锁风险，见 §29 与 `services.rs` 的 `max_concurrent_by_type`）。

### 30.4 条数上限（按推荐默认落地，写明依据与回退）

启动钩子 `start_interrupted_analysis_rerun` 的单轮上限设成 **3**
（`services.rs::STARTUP_RERUN_MAX_PER_ROUND`；定时巡检仍不限）。
依据＝现网实测：36 行待补、每行约 7 分钟，而 `tauri dev` 看门狗每次存盘重启都会把在跑那行
打断回 `interrupted` ⇒ 两小时只补完 1 行，其余原地不动而配额照烧。
**回退**：常量改回 `0` 即完全恢复改前语义（`RerunConfig.max_per_run == 0` = 不限），无其它连带面。

### 30.5 待补读数

`--lib stock_analysis`（含 v165 门）、`--lib scheduler::restore`、rt-workflow `execution_finalization`
两例、`cargo fmt --all --check`、`clippy -p axagent -p axagent-rt-workflow -p axagent-dao --all-targets`。
CI 字面 `cargo test --workspace` 在 `tauri dev` 运行期物理上跑不了（exe 锁），已用 `--workspace --lib` 代跑并注明差异。

## 31. v165 批次终局读数 + 「35 条中断分析消失」的归因（2026-10-17 18:4x）

### 31.1 §30.5 那五条待补读数的现状

| 读数 | 结果 | 备注 |
|---|---|---|
| `cargo test -p axagent --lib stock_analysis` | **109 passed / 0 failed，`SA_EXIT=0`** | 含本轮两道新门：`no_single_point_blocks_portfolio_mgr`、`zero_consumer_aggregators_are_tolerant` |
| `cargo test -p axagent --lib scheduler::restore` | **1 passed / 0 failed，`RESTORE_EXIT=0`** | 秒/毫秒单位判据（末段毫秒负控） |
| `cargo test -p axagent-rt-workflow --test execution_finalization` | **2 passed / 0 failed，`EXIT=0`** | 首轮红来自进程级替身仓储被并行用例互相覆盖，已由 `REGISTRY_GUARD` 串行化（§30.2），不是实现错 |
| `cargo fmt --all --check` | **EXIT=0**（带 `--all`，覆盖 `crates/` 那 36 个成员） | |
| `cargo clippy -p axagent -p axagent-rt-workflow -p axagent-dao --all-targets -- -D warnings` | **未取得** — 日志正文停在 `Blocking waiting for file lock on build directory`，即构建目录被另一路 cargo 占着，clippy 还没进编译阶段 | 读数文件 `.workbuddy/tmp/v165-clippy.log`；按纪律不拿「后台任务回执 exit 0」当结论 |

本轮编译错三轮全部在**我新写的判据代码**里（不是实现）：`ancestors_of` 缺 `'a` 生命周期（连带 5 处 E0425）、测试模块的显式导入清单没加两个新符号、`restore.rs` 测试里用闭包返回 async block 触发「lifetime may not live long enough」⇒ 改局部 `async fn put`。另修 `unused import: WorkflowNode` 与两处 `doc_lazy_continuation`。

### 31.2 「35 个中断的分析消失了」：不是删除，是被 30 行窗口顶出视野

18:45 现网读数（正门 `scripts/pg-connect.mjs`）：

- `stock_analyses` = **completed 190 / interrupted 35 / running 1** ⇒ 35 条**一条没少**。
- 35 条 interrupted 全部 `parent_analysis_id IS NULL`（都是原始行，不是重跑产生的子行）。
- **比「最新一条 interrupted」更新的行有 42 条**；而历史列表 `query_stock_analysis_items` 的口径是 `ORDER BY created_at DESC`，前端三处调用点把 `limit` 写死成 **30 / 50 / 20** ⇒ 35 条**全部落在窗口之外**，界面上就是「消失」。
- 成因是两条的合成：启动回收**只改 `status`、不改 `created_at`** ⇒ 被回收的行永远是「旧行」；补跑按原 `analysis_date` 落**新行**，新行 `created_at` 更大 ⇒ 每补一轮，没补完的那批就再往下沉一层。这是**呈现层窗口**问题，不是回收把数据弄丢了。
- `workflow_executions` = **running 87 条，且 87 条全部早于 1 小时**（newest running 约 2.6 小时前），`interrupted` **0 条** ⇒ §30.2 的执行表回收**尚未跑过**（不是「跑了没效果」）：`Get-Process` 里没有主程序进程、`tauri dev` 未运行，而这条回收只挂在启动钩子上。
- 互斥门没有挡路：`~/.axagent/` 下**不存在** `batch-rerun.lock` ⇒ 下次启动不会跳过回收。
- 用与实现逐字同谓词的 SELECT 预估下次启动命中面：**执行表 87 条、分析表 1 条**（那条 running 是 `d3a74ec8-…@2026-10-17 09:05`，进程被打断时遗留）。

⇒ **#8 的两端修复（引擎无条件写终态 + 启动回收）代码与判据到位，现网效果要有读数必须应用重启一次**。我不把「判据绿」写成「87 条已回收」。

### 31.3 由此浮出的新待裁项（本批未动手）

要让界面能定位到「未产出结论」的行，三选一：

| 选项 | 做法 | 代价 |
|---|---|---|
| 甲 | 历史入口加「只看未产出结论」筛选，后端 `list_stock_analyses` 加 `status` 谓词 | 要动命令参数 + 11 语言文案；能直达那 35 条 |
| 乙 | 打通 `offset`（后端参数已支持，前端从没传过）做分页 | 只改前端；226 行要翻 8 页 |
| 丙 | 列表不动，靠任务中心 `stock-analysis-rerun` 的运行摘要计数（`RerunOutcome::render()` 已经写进 `TaskRunResult.output`，实测文案「起始 N 条待处理 → 收敛 M 条，剩余 K 条」） | 零改动；但只能看到**一个数**，看不到是哪 35 条 |

推荐 **甲 + 乙同时做**：丙 的计数通道已经活着，缺的恰恰是「哪几条」；按「结构性缺口不得在 UI 造成歧义」这条纪律，该给的是可定位，而不是一个更大的数字。

### 31.4 补跑链的现网装配读数（回答「那 35 条到底会不会被补完」）

`cron_jobs` 里这一行是活的（键值表，列为 `id / data`）：

```
id = stock-analysis-rerun-daily
schedule = 0 4 * * *            ← 每天 04:00
prompt   = {"maxPerRun":0,"maxRounds":3}   ← 定时巡检**不限条数**（与启动钩子的 3 条是两格）
status   = active   run_count = 0   last_run_at = null   next_run_at = 2026-10-18 04:00
```

⇒ 结论口径要分两句说，不能合成一句：

- **启动钩子**：单轮 3 条（`STARTUP_RERUN_MAX_PER_ROUND`，§30.4），只解决「刚发现就补」。
- **夜间巡检**：`maxPerRun = 0` = 不限，`run_rerun` 内**逐条 `await` 落定再起下一条**（本轮修的就是这条），
  35 条 × 约 7 分钟 ≈ 4 小时 ⇒ 只要 04:00 那趟不被 `tauri dev` 存盘重启打断，这批会在夜里排空。
- `run_count = 0` 是如实读数：**这趟还没跑过**，所以「35 条会收敛」目前是**按判据推算**，不是实测。

### 31.5 前端侧本轮改动的门禁读数（与 Rust 门禁分列）

| 门 | 读数 |
|---|---|
| `npm run typecheck`（tsc --noEmit） | **EXIT=0** |
| `npx dprint check` | **EXIT=0** |
| `npm run lint:eslint`（oxlint，1359 文件 / 96 规则） | **0 warnings / 0 errors** |
| `node scripts/check-i18n-key-parity.mjs` | **EXIT=0**，11 语言 key 集合一致（各 15950 条），正控有效 |
| `python scripts/check_i18n.py` | **EXIT=0**，代码引用的 key 全部有定义 |
| 定点 `vitest run`（`DecisionTrustNotice` + `ExecutionReplayPanel`） | **2 files / 20 tests passed** |

## 32. 定点 clippy 的真实读数：一条红，红在我自己的新代码（2026-10-17 晚）

§31.1 那条「仍在编译」已被真读数取代。全量跑完后日志正文是：

```
error: items after a test module
    --> src\commands\stock_analysis_setup\seed_stock_analysis.rs:10159:1
11296 | use axagent_harness::workflow_types::{WorkflowEdge, WorkflowNode};
11302 | pub(crate) fn ancestors_of<'a>(…)
11324 | pub(crate) fn decision_chain_single_points(…)
CLIPPY_EXIT=101
```

> 上面代码块里的行号是**搬迁前**的现场读数，现已不成立；本文件定位一律按符号名（`ancestors_of`、`fast_workflow_derivation_tests`）取，不按行号。

**根因不是「测试模块插错位置」，而是「生产函数追加到了文件物理末尾」**。该文件在
`mod fast_workflow_derivation_tests` 之前本来就写着一条警告注释「本测试模块必须留在文件末尾」，
我按「新代码追加到文件末尾」的既有纪律往里加，结果把三个**生产**函数加到了它**后面** ⇒
`clippy::items_after_test_module` 当场判红。这条 lint 只在 clippy 下暴露，
`cargo check` 与 `cargo test` 全绿（本轮就是 109/0 + 1/0 全绿却 clippy `EXIT=101`）。

**修法（纯位置搬迁，零语义改动）**：把 `use` + `ancestors_of` + `decision_chain_single_points`
+ `harden_decision_chain` 整块移到该文件**第一个 `#[cfg(test)]` 之前**（现位于
`seed_stock_analysis_fast_workflow_template` 的收尾之后），测试模块重新成为文件最后一个条目。搬迁用两趟法脚本
（先算再写），落盘前断言三件事：块内必须含那三个符号、块内不得含 `#[cfg(test)]`、
搬完后 `定义行 < #[cfg(test)] 行 < mod 行`。`wc -l` 读数 `11394 → 11395 → 11394`
（搬迁时多插的那个空行由 `cargo fmt -p axagent` 收掉），`cargo fmt -p axagent --check` **EXIT=0**。

**判据（写进纪律，别再犯第二次）**：「新代码追加到文件末尾」这条只对**测试模块**成立；
一个文件里只要已经存在测试模块，**新增生产条目必须插到它之前**。同一条 lint 的另一半后果是
它会把该文件后续所有条目一并判死 ⇒ 搬迁后必须重跑 clippy，不能只跑 `cargo test`。
重跑读数（clippy 三条 crate + 两组定点 lib 测试）在 `.workbuddy/tmp/v165-final3.log`。

## 33. 第 9 项落地：模型级「请求延时」接进统一入口（P-c 的终态）

### 33.1 动手前的现场读数（三处各写一半）

设置页 `settings.requestDelayMs` 的文案是「**每次调用 API 前**等待的毫秒数」，控件带上下界
（`ProviderDetail.tsx` 的 `min={0} max={60000}`）。可是同一个 `ModelParamOverrides::request_delay_ms`
在全仓有三种互不一致的读法：

| 读法 | 位置 | 后果 |
|---|---|---|
| 每次 `stream()` 无条件睡 | `crates/agent/src/provider_adapter.rs`（`block_in_place` + `thread::sleep`） | 只覆盖 agent 运行时那条链 |
| 只在 `iteration > 0` 睡 | `src/commands/conversations/streaming.rs` | 首轮请求根本不节流，与文案不符 |
| **完全不读** | 工作流 10 个 `LlmCallConfig` 构造点 | 面板那句话对股票分析链是**假的** |

⇒ 终态：值仍由调用方按「本次要打哪个模型」取，**sleep 只做在一处** ——
`harness::llm_executor::apply_request_delay`，且放在缓存命中短路**之后**（命中的那趟没有 API 请求，
限流无从谈起，不该被拖慢）。

### 33.2 改动清单

1. **消费端**：`LlmCallConfig` 加 `request_delay_ms`；`execute_llm` / `execute_llm_stream` 各调用一次
   `apply_request_delay`。`execute_llm` 里睡在重试序列**之前**（后续 attempt 由 `RetryPolicy` 的退避负责）。
2. **查表端**：新增 `request_delay_ms_for_model(prov, model_id)`，与既有 `resolve_model_type`
   **同一查找形态**（按 `model_id` 在 `prov.models` 定位），不重定义第二套（禁区 12）。
   顺带给 `ModelParamOverrides` 补 `Default` derive —— 该结构体字段**全是 `Option`**，
   `Default` 的语义正好是「一格 override 都没配」。
3. **删掉两处各睡一次**：`AxAgentApiClient` 的字段与 sleep 整块删除，builder 方法名
   `with_request_delay_ms` 保留、改为写进 `llm_config`；`streaming.rs` 的 `iteration > 0` sleep 删除。
   ⚠ **字段被删之后，「只有一个地方 sleep」由编译器保证** —— 这比再加一条 grep 门更硬，
   所以本批**没有**新建「全仓只许一处 sleep」的脚本门。
4. **顺序坑**：`commands/agent/mod.rs` 里延时的赋值挪到语义缓存分支**之后**。那个分支用
   `..Default::default()` **整份替换** `llm_config` ⇒ 写在它前面会被静默抹掉，症状是
   「开了语义缓存就不节流」，不报错、不掉日志。
5. **工作流 10 个构造点全部接上**：`agent_executor.rs` 6 处（主模型 2、回退路径 2、
   `execute_plan_mode`、`try_fallback_stream`）+ `condition_executor` / `llm_classifier_executor` /
   `switch_executor` / `llm_executor` 各 1。回退那四处取**回退模型自己**的延时（`fb_prov` / `fb_model`），
   不沿用主模型的。
6. **刻意未扩**：网关 `crates/gateway/src/handlers/chat.rs` 与 `src/commands/workflow_ai/chat.rs`
   传的是各自构造的 `llm_config`，新字段取默认 `None` ⇒ 行为与改前逐字一致。要扩也得先决定
   「外部 OpenAI 兼容流量该不该被本机某个模型的延时拖住」，那是产品口径，不在本批。

### 33.3 判据（三条，各带负控）

| 判据 | 落点 | 钉住什么 |
|---|---|---|
| `request_delay_tests` 2 条 | `crates/harness/src/llm_executor.rs` | 配 40ms 必须真睡够；`None` 与 `Some(0)` **不得**睡（把判据写成无条件 sleep ⇒ 第二条红） |
| `request_delay_lookup_tests` 3 条 | `crates/harness/src/types/provider_model.rs` | 命中取值；**邻居模型的值不得借用**（`find` 换成 `first()` ⇒ 红）；模型未登记 ⇒ `None`，不许为「查不到」编一个默认延时 |
| 新门 `llm_throttle_wired` 2 条 | `crates/rt-workflow/tests/llm_throttle_wired.rs` | 扫 `executors/` 每个构造点必须带 `request_delay_ms`；下限断言 `>= 10` 防空集恒真；探测器自证 5 个样本（裸 `default()`、漏字段字面量、`use` 行、类型标注、注释行） |

两侧分工写死：harness 那两条钉**消费端真在睡**，新门钉**调用端把值送到了**——摘掉任一边只红对应那一条。

### 33.4 本批踩到的四处（都不是实现错，但都值得记）

- **3× E0308**：判据的辅助函数写成 `overrides(Option<u64>)`，我调用时传了裸整数 `700`。
  改成 `overrides(u64)` 内部包 `Some` —— 值表保持生产形态，签名短到不易误用。
- **2× `clippy::needless_borrow`**：`condition_executor` 那处的 `prov` / `model` 本来就是
  `&ProviderConfig` / `&str` 形参，我照抄了别处的 `&prov, &model`。⚠ clippy 短路：
  `axagent-rt-workflow` 报红时**主 crate 从未被 lint** ⇒ 修完必须再跑一轮才谈得上四 crate 全绿。
- **一次 Edit 手滑**：`provider_model.rs` 的 `old_string` 多带一行、`new_string` 少带同一行，
  静默删掉 `ModelType::Voice => write!(f, "voice")` 一格（会变成非穷尽 match）。
  改完立刻 `git diff` 该文件才发现并补回 —— 「改整行必须给整行全文，改完按 diff 回读」。
- **一次 Read 与磁盘不一致**：`Read` 渲染出的注释行是 `(default/OpenAI) or`，磁盘上其实是
  `(default) or` ⇒ 以 Read 输出当锚点写 `old_string` 直接 0 occurrences。锚点一律回到
  `grep`/`sed` 取的磁盘字面量。

## 34. 现网运行时读数（应用 18:50 起过一次，两批第一次拿到实测证据）

20:40 取，正门 `scripts/pg-connect.mjs`。这一节把 §31.2 / §31.4 / §30.1 三处「按判据推算」换成实测。

### 34.1 种子与回收都真的跑了

- `workflow_templates(id='stock-analysis').version` = **165** ⇒ v165 的种子（图级容错规则 + `max_retries: 5`）
  已落库，不只是代码里。
- `workflow_executions`：completed 640 / partially_completed 490 / cancelled 168 / **interrupted 87** /
  failed 16，`running` **0 条**（18:45 读数是 running 87、interrupted 0）。87 条的 `updated_at`
  **全部 = 10-17 18:50** ⇒ #8 的启动回收在现网生效。
  顺带实测掉一个我以为要补的格：这些行 `paused_at` 残留 **0** 条 ⇒ 回收不必额外清 `paused_at`
  （这是量出来的，不是推的）。
- `stock_analyses`：completed **191** / interrupted **35** / running **0**，总行数 226 未变 ⇒ 没删行。
  18:45 那条幽灵 running（`d3a74ec8`，603466）现况 = **completed、`updated_at` 18:58、
  `data_gaps` 空、四档键齐** ⇒ 「18:50 回收 → 启动补跑（单轮上限 3）→ 落库收口」整条链在现网跑通一次。

### 34.2 P-b 的图级规则在**落库产物**上独立复算（不等 Rust 判据）

用一条递归 CTE 在库里的真实图上重算，排除规则与 `decision_chain_single_points` 逐字对齐
（入度 0 的根跳过；唯一上游是 `trigger` 的取数格跳过）：

| 读数 | 值 |
|---|---|
| `portfolio-mgr` 传递祖先 | **84** |
| 按规则剔除：入度 0 的根 | 2（`trigger` + `t-dragon-tiger-data`） |
| 按规则剔除：唯一上游是 `trigger` 的取数格 | **15** |
| **真违规格** | **0** |

- ⚠ 与 §30.1 记的「12 个取数节点」差 3。判据断言的是「违规格为空」，不锁这个集合的条数 ⇒ 判据照样成立，
  但**两个读数不同源**；下次动这块前先按现码重取，别把 §30.1 的数字当权威。
- `t-dragon-tiger-data` 入度 0 是**设计**，不是悬空缺陷：种子代码里 v4 就写明「不挂 `bear-r3 → t-dragon-tiger-data`
  入边，否则 `a-hot-money → 辩论链 → bear-r3 → t-dragon-tiger-data → a-hot-money` 成环」，数据依赖只由出边保证。
  引擎就绪判据正是「入度为 0」（`crates/rt-workflow/src/work_engine/engine/dag_store.rs:126` 的注释 +
  `engine/mod.rs` 的 in_degree 计算）⇒ 它作为第二根照常起跑。
- 另一格 `sim-verify`（code 节点，入度 0 且出度 0）是全孤立，不卡任何链路；本批未动（要不要删属另行裁定）。

### 34.3 「公式 ↔ LLM 逐档对照」到底有几条样本（这是最初需求的验收读数）

逐档对照是**分析落库时**算好、写进 `decision_json.agreementByHorizon` 的（`stock_workflow/core.rs`
的 B1/B2 段），不是等反思那趟。库里现况：

| 读数 | 值 |
|---|---|
| completed 行 | 191 |
| 四档 LLM 键齐（`horizon_decisions` 有 `ultraShort/short/mid/long`） | **102** |
| 其中 `decision_json` 里带 `decisionsByHorizon` 的 | **102（全带）** |
| 写了 `agreementByHorizon` 的行 | **35**（共 140 格） |
| 其中 `comparable=true` 的格 | **8** ＝ **2 行 × 4 档** |
| 这 2 行的 `template_version` | **164**（15:17）与 **165**（18:58） |
| 另外 100 行有 LLM 键但没 `agreementByHorizon` | 是**更旧代次**的产物（≤158，多数 `updated_at` 10-16 07:30） |
| 140 格里 `absence` 文案分布 | 124「LLM 逐档契约上线前的记录」+ 8「公式侧该档分支未产出」+ 8（无 absence＝可比格） |

⇒ 三句话结论：**通道是活的**（v164/v165 两条各自算出 4 档可比格）；**样本数=2 行 / 8 格**，
不是「零」也不再是「一条都没有」；那 100 行缺对照**不是新缺陷**，而是「不回填、标上线前样本」
那条已批裁定的直接体现（它们的 `template_version` 就是证据）。
要涨样本只有两条路：新跑分析（每跑一条就多 4 格），或让 35 条中断行走完补跑。

## 35. 节流批（§33 那件事）的终局读数 + 一条现网新读数

### 35.1 门禁：四步逐 crate 全绿（日志正文，非后台回执）

| 步 | 命令 | 读数 |
|---|---|---|
| 1 | `cargo fmt -p harness -p agent -p rt-workflow -p axagent` 写入 + `--check` | **0 / 0** |
| 2 | `cargo test -p axagent-harness --lib request_delay` | **5 passed / 0 failed**（2 条消费端行为 + 3 条查表，含「不得借用邻居模型的延时」负控） |
| 3 | `cargo test -p axagent-rt-workflow --test llm_throttle_wired` | **2 passed / 0 failed**（构造点全带字段 + 探测器 5 个假样本自证） |
| 4 | `cargo clippy -p axagent-rt-workflow --all-targets -D warnings` | **EXIT=0**（首轮为 2× `needless_borrow`，修在 `condition_executor.rs`） |
| 5 | `cargo clippy -p axagent-harness -p axagent-agent --all-targets -D warnings` | **EXIT=0** |
| 6 | `cargo fmt --all --check`（全 workspace 口径） | **EXIT=0** |
| 7 | `cargo clippy -p axagent --all-targets -D warnings`（主 crate） | **EXIT=0** |

⚠ 第 4 步报红时**主 crate 从未被 lint**（clippy 遇首个 error 短路）⇒ 第 7 步必须在那之后单独再跑，
本表就是按这个顺序取的两轮。

### 35.2 一条行为变更要如实记账（不是回归，是口径统一）

聊天路径旧写法只在 `iteration > 0` 睡 ⇒ **首轮请求不节流**；接到统一入口后变成「每次请求前都睡」，
与设置页文案「每次调用 API 前等待的毫秒数」一致，代价是**首 token 延迟增加 `request_delay_ms`**。
控件本来就有上界 `max={60000}`（`ProviderDetail.tsx:2702`）⇒ 最坏每次多等 60 秒，不会失控。
刻意没扩的两条路径（网关 OpenAI 兼容入口、`workflow_ai`）仍取默认 `None`，行为与改前逐字一致。

### 35.3 待补：CI 字面 `cargo test --workspace`

此前一直被 exe 锁挡住（主程序在跑）。20:40 起 `Get-Process` 已无主程序进程，本次按
`--workspace --no-fail-fast` 跑（**比 CI 那条更强**：CI 的 fail-fast 只报首个失败二进制，
`--no-fail-fast` 会把全部失败列出；两者对「全绿」的判定等价）。

**实测读数（21:1x 取，正文来自 `.workbuddy/tmp/workspace-test-final.log`）**：
`test result:` 共 **147** 行，**147 行全部 `ok`**；合计 **8641 passed / 0 failed / 51 ignored**，
`TEST_EXIT=0`。日志正文里 **没有** 任何 `test result: FAILED`、`failures:` 或 `error[E...]` 块
⇒ 环境型红一条都没出现（不必动用 flaky 归桶）。这一条同时是**两批**（v165 图级容错/执行行收口 +
节流接线）的终局全量读数，不再是 `--lib` 的窄口径。

### 35.4 顺带取到的一条现网读数（关系到「对照样本什么时候涨」）

35 条 `interrupted` 行的构成：`analysis_date` = 10-15 **16 条** / 10-14 **18 条** / 10-11 **1 条**，
`template_version` **全为 NULL**（没跑完 ⇒ 没记代次），35 行覆盖 **31 只标的**（同一标的可有多行，`load_rerun_candidates` 里已按 `(analysis_date, stock_code)` 同键去重）。
⇒ 夜间巡检补跑的是**历史 as-of 回放**（10-11~10-15），不是当日分析；
它们的超短档（T+2）已接近到期、短档（T+5）约 10-19~10-21 陆续到期 ⇒ **补完之后逐档对照样本会随反思链涨起来**，
这比新跑一条当日分析拿到档位结论更快。
另：`template_version` 为 NULL 属已处理形态（界面按「未记录」陈述，不参与差值计算）。
