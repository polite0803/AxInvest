# PLAN：四周期（超短/短/中/长）全链路闭环收编

> 状态：active ｜ 日期：2026-09-28 ｜ 对应版本：v2.9.9


日期：2026-09-28 · 状态：**v2 已实施（Phase 0-7 代码全部落地）+ 2026-09-29 §七 收口（预设矩阵裁定、四道机器门、五点对账补齐，**最终门禁衡 §7.5**），未提交** · 上游：本轮静态核查（下列行号均为磁盘实证）

> **v2 执行记录（2026-09-28，Phase 2-7 + Phase 1 返工）**
>
> **Phase 1 返工（v1 → v2）**：撤除「入口锁档」整条通路 —— `run_stock_workflow` 的 `userTimeHorizon`
> 入参与命令层校验、hooks 的 `user_time_horizon` 注入、`seed_variables` 的该变量、portfolio-mgr
> 节点的同名 `input_mapping`、搜索栏的四档 Segmented、store 的 `analysisHorizon` 全部删除；
> `trader.md` 的「本轮持有周期约束」改写为「**持有周期：你不定档**」（trader 自报值降级为仅自述留档，
> 供双视角对比）；sector/sentiment/social-media 三处 `{{user_time_horizon}}` 措辞改为「一次分析
> 同时产出四档决策，报告对四档共同负责」。保留的部分：`decision_horizon_source` 列（值域改为
> `formula`/`model`/`user`，由 `decision.rs::extract_horizon_source` **归一**后入库）、
> `selectedHorizon` 死字段的消除、i18n 错键修复（`stockAnalysis.period.*` 实为 K 线周期段）。
>
> **Phase 2**：`risk-management/SKILL.md` 两档 → 四档（止损 -3/-5/-8/-12、时间止损 2/5/28/90 天，
> 并声明「权威在设置面板/`seed_variables`，SKILL 不参与决策执行」）；trader 档位表加注「公式才是裁定方」；
> 示例 `"mid"` 加「示例值非默认」注。**新增两道门**：
> `expert_prompt_placeholders_all_have_an_injection_source`（占位符须有来源 = 变量表 ∪ input_mapping
> target ∪ 运行期注入，含负控与「运行期注入面扫描自证」）、
> `tunable_params_have_variable_and_rhai_guard`（可调参数三处齐备）。
>
> **Phase 3（HorizonLayer 进主链 + 档位参数化）**：定档段（`days_for`/`time_horizon`/`horizon_source`/
> `expected_holding_days`）**从「── 时间 ──」区上移到凯利仓位之前**，`position_pct = clamp(仓位 × 周期乘数)`
> ⇒ 乘数从「只在前端交叉验证里算」变成主决策真实消费；乘数唯一权威源 = 新增 `Period::position_multiplier`
> （0.6/0.8/1.0/1.2），`evidence_weight.rs` 的 `compute_recommended_position` 改为**委托**该函数
> （原来自己抄一份 0.6/0.8/1.0/1.2 且用 `_ => 1.0` 混兜 mid）。`get_horizon_base_weights` 的 `mid`
> 由 `_` 兜底改为**显式分支**，未知档 WARN + 回落。逐周期止损/止盈**变量化**：新增 8 个可调参数
> `sl_pct_{ultra_short,short,mid,long}` / `tp_pct_*`（默认 3/5/8/12、5/10/18/30），
> 三处齐备（`seed_variables` 定义 + 脚本 `present()` 守卫 + `PORTFOLIO_MGR_TUNABLE_PARAMS` 28→36）
> ⇒ 设置面板新分组 `portfolio_mgr_horizon` + **反思可调清单自动含它**；持有天数**刻意不做成可调**
> （它是按档成熟判定的基准）。`TEMPLATE_VERSION` 96 → 97。
>
> **Phase 4（估值周期闸口）**：f5 融合腿加 `dcf_horizon_decay`(×0.30) / `graham_horizon_decay`(×0.50)，
> 仅 `ultra_short`/`short` 生效，`fusion_wsum` 同步按「可用 × 衰减」归一 ⇒ **中长线是恒等变换**；
> PE/PS/PB 分位带腿不衰减（相对贵贱与持有期近乎无关）；降权**必须**进 `data_gaps` 留痕，
> 与 V77 的「`applicable=false` ⇒ 整腿剔除」明确区分（一个没经济含义、一个有含义但不服务本档）。
>
> ⚠ **2026-10-01 更正（模板 v112）：Phase 4 的**主链**落地形态已撤除。** 它按 `time_horizon` 分支，
> 而该变量定义在脚本**后面的定档段**（`data_gaps` 更在其后）⇒ Rhai 顺序求值下运行期抛
> `Variable not found: time_horizon`（生产实报 line 844），被文件末尾 catch 兜成
> `action="数据缺失"` ⇒ **凡有估值数据的分析全部静默降级**，且两道编译门（语法门 / 纯 compile 门）
> 全绿。该段本身还是**自指环**：主档由「含 f5 的后验」定出，它却要用主档定 f5 的衰减。
> **现形态**：主链恢复 `0.56/0.24/0.20` 的纯可用腿归一（与回归前 `7629f089e^` 逐位相同 ⇒
> 主链零语义漂移）；周期降权由**权威逐档乘数**承担（`horizon_leg_multipliers` → 脚本 `leg_mult`，
> f5 在 ultra_short/short/mid/long = 0.3/0.5/1.2/2.0）；「降权必须留痕」改由 Phase F 段读**同一份
> 乘数**声明进 `data_gaps`（脚本侧不再手抄 0.30/0.50）。新增运行期门
> `portfolio_mgr_runs_with_valuation_evidence_without_degrading`（注入估值两腿真执行整脚本 + 负控），
> 填上「只 compile / 只跑片段」这一格。判据层记录见 `MEMORY-RULES.md` #713。
>
> ⚠ **同日第二处（模板 v113）：同一 catch 兜底把两个缺陷并成了一个症状。** v112 落地后生产
> **新跑的一轮仍报「数据缺失」**，这次是 `line 2853 Variable not found: horizon_prior_json` ——
> Phase C（v102）只做了「hooks 恒注入 + 脚本 `prior_for` **裸读**」，**漏了 portfolio-mgr 的
> `input_mapping` 同名映射**；而脚本能看到的注入名由**本节点映射**单独决定（hooks 写进黑板
> `variables` ≠ 进了脚本 scope，`code_executor` 只按映射注入）⇒ 该缺口被 line 844 的崩溃遮住。
> 已补映射并升版 v113；运行期门同时**收紧**为按「本节点 `input_mapping` ∪ 脚本 `present(x)`」造
> scope（首版把 hooks 的 `Variable.name` 也算来源 ⇒ 恰好在「注入了却漏映射」这一格假绿；
> 收紧后它以**与生产逐字相同**的报文报红，才去修映射）。判据层见 #713 的同日追记。
>
> **Phase 5（经验闭环带周期）**：`reflection_lessons` 与 `stock_reflections` 各新增 `horizon` 列
> （**只改实体声明，DDL 由 `schema_diff::heal_all` 自愈**，无 migration）；`extract_lesson_to_rule`
> 签名增 `horizon`、去重键改 `(stock_code, horizon, lesson_summary)`；读侧 `fetch_stock_lessons` /
> `fetch_rule_lessons` 增 `horizon` 过滤（同档 + NULL 通用，**跨档不进 prompt**）；
> 一次反思 = 一个周期：`run_reflection_workflow` 增 `review_horizon`、`run_reflection_now` 增 `horizon`
> 入口参数（脏值在建反思行前拒绝），内层单股反思显式透传 `primary_horizon`，反思收尾把该档
> **盖章写进 `stock_reflections.horizon`**；`strategy_performance.period` 由裸 `"reflection"` 改为
> `reflection:{horizon}` 并填 `horizon_pnl_json`。原「消除断点⑥（四周期共享同一 reflection 文本）」
> **改判为不需要**：单周期化后该形态不再存在。前端反思面板加「本次复盘哪一档」选择器
> （留空 = 用公式主档，不伪造），重跑沿用该行原档。新行为测试
> `lessons_are_isolated_per_horizon`（真库，一次覆盖「自愈建列 + 同档可读 + 跨档排除 + 两档不合并」）。
>
> **Phase 6**：serenity 落库口径**显式化** —— `Period::Mid` 常量 + `holdingDays` 兜底由裸 20 改为
> 该档权威 28 天（原 `period=mid` 与 `holding_days=20` 在同一行自相矛盾），止损/目标/入场乘数
> 改读**同一份**模板变量 `serenity_stop_mult`/`serenity_target_mult`/`serenity_entry_range`
> （此前工作流路径硬编码 0.80/1.30/±5%，面板调了不生效），并 `info!` 声明档位；
> 面板加 `serenityPanel.tierScopeHint`（× 11 语言）明示「只服务中线，短/超短不适用」；
> `stock_create_trend_screening_cron` **刻意不增 `periods`**（不支持的档不做入口）。
> 策略包按裁定**全删**：3 份 `strategy_pack.rs`（1356 行）+ 18 个 YAML + `StrategyPackSettings.tsx`
> + 7 个命令 + 11 语言 i18n 块；注册表由 `build.rs` 自动重算（`register_commands.rs` 归零，未手改）。
>
> **门禁状态（截至本条）**：绿 ——`cargo fmt --all --check`、`npm run typecheck`、`npm run lint:eslint`(oxlint)、
> `npx dprint check`、`check-line-endings.sh`、`check-i18n-key-parity.mjs`（11 语言 key 集一致）、
> `npm run contracts`（errors=0）、`check:serde`/`check:types`/`check:paired`（0 errors）、
> `audit-inject-coverage --strict`（无新悬空映射，① 仍只有既有 `ts`）、
> `rhai_syntax_check` 8/8、`portfolio_mgr_days_for_rhai` 5/5、`portfolio_mgr_horizon_lock_rhai` 4/4、
> `-p axagent --lib commands::stock_analysis_setup` 37/37、`-p axagent-analysis-engine --lib reflection_stats` 15/15、
> `-p axagent-harness --lib holding_period` 3/3、`evidence_weight` 10/10、
> `-p axagent --lib commands::stock_workflow::reflection` 19/19、Vitest 57 文件 624 用例、
> `playwright test --list` 128 规格可编译。**待补**：全量 `clippy --workspace --all-targets --all-features`
> 与 `cargo test --workspace`（后台在跑）；`check-single-source-facts` 剩 1 条**非我方**红
> （`domain_semantics.rs` → `evidence_citation.rs:198` 指向空行，属并行会话未提交改动）。
>
> **过程中自我抓到的三个缺陷（已修，登记进判据层 #701-#703）**：
> ① 把 `horizon_const` 写成 `fn` ⇒ 生产执行形态（`compile` + `eval_ast_with_scope`）下读不到注入变量，
>    而纯 compile 门全绿；② 按名调用闭包 ⇒ `Function not found`（本仓约定一律 `.call(...)`）；
>    ③ 一次窗口式删改用 `str.index()` 错锚，在 `seed_stock_analysis.rs` 里**复制出 507 行重复节点**、
>    在 `evidence_weight.rs` 里造出**嵌套 `mod tests`** —— 两处均以「与 HEAD 做 difflib opcode 对照 +
>    即时 `cargo check`」定位并按行号还原/重放。另：脚本初稿用了未注册的 `Array.prototype.includes`，
>    改成本仓有先例的 `contains`。
>
> **仍未做**：重启应用真跑四档核对（需你在 `tauri dev` 侧配合）；本轮全部改动**未提交**
> （工作树混有并行会话的估值重构，按你此前决定暂不提交）。

> **Phase 0 落地记录（2026-09-28）**：`Period` 上提至 `crates/harness/src/holding_period.rs`（新增 `holding_days_map()`），`recommender/types.rs` 改 re-export、`reflection_stats::default_expected_holding_days` 改委托、`portfolio-mgr.rhai` 两处手抄天数表删除并改读注入变量 `horizon_days_json`（由 `stock_workflow/hooks.rs` 无条件注入），seed 的 portfolio-mgr `input_mapping` 补同名映射，`TEMPLATE_VERSION` 94→95，e2e 夹具 ultra_short 3→2。
> 已验证：`cargo fmt --all --check` 绿；定点 clippy `-p harness -p analysis-engine -p rt-workflow --all-targets --all-features -D warnings` 绿；`-p axagent` 同口径绿；`cargo test -p axagent-harness --lib holding_period` 2/2；`-p axagent-analysis-engine --lib reflection_stats` 15/15；新增 `crates/rt-workflow/tests/portfolio_mgr_days_for_rhai.rs` 4/4（含缺失注入/未知档位必须失败的负控 + 闭包副本逐字防漂移锁）；`all_rhai_scripts_compile` 8/8；`audit-inject-coverage.mjs --strict` 的 ② 无悬空映射（① 的 `ts` 为既有项，非本轮引入）。
> **未做**：重启应用 + `axagent-batch-rerun` 四档实跑核对（Phase 7）；提交（工作树含他人未提交改动，见下）。

> **Phase 1 落地记录（2026-09-28）**：周期成为分析工作流的**输入**。
> 后端：`run_stock_workflow` 增 `userTimeHorizon`（命令层校验四档 + auto，非法值在建行前拒绝）→
> `RunOptions.input` → `hooks::build_stock_analysis_variables` 恒注入变量 `user_time_horizon`（未选=auto）→
> `trader.md` 新增「本轮持有周期约束」段（`{{user_time_horizon}}` 硬约束）→
> `portfolio-mgr.rhai` 锁主周期并输出 `horizonSource`（**唯一**判定出口，Rust 侧不二次推断）。
> 落库：`stock_analyses` 新增列 `decision_horizon_source`（走实体声明 + schema 自愈，无迁移），
> 三个写入口（工作区 completed / failed 降级、对话直执行 persist）经 `decision.rs::extract_horizon_source` 共用同一读法。
> 前端：store 增 `analysisHorizon`/`setAnalysisHorizon`，搜索栏加 Segmented 四档 + 自动；
> 原先的**死字段 `selectedHorizon` 已消除**（NL 解析结果现真接进入口）；顺带修一个 i18n 错键
> （意图标签读 `stockAnalysis.period.*`，该段实际是 K 线周期 `1m/3m/…` ⇒ 标签此前恒显示回退串）。
> 新增周期 i18n 键 2 个 × 11 语言（`check-i18n-key-parity.mjs`：15304 键全齐）。
> `TEMPLATE_VERSION` 94→96（v95=Phase 0 天数表注入，v96=Phase 1 锁档）。
> 已验证：`cargo fmt --all --check` 绿；**全量** `clippy --workspace --all-targets --all-features -D warnings` 绿；
> 新增 `crates/rt-workflow/tests/portfolio_mgr_horizon_lock_rhai.rs` 5/5（锁档覆盖模型自报 / auto 保留模型值并标 model /
> 后验兜底四档 / 脏值必须失败 + 副本逐字防漂移锁）；`npm run typecheck` 绿；
> `dprint check`（改动的 2 个 TS 文件）绿；行尾检查绿；
> `audit-inject-coverage --strict` 的 ② 无悬空映射（① 仍只有既有的 `ts`）；
> Vitest 股票分析 + lib 全套 31 文件 216 用例全绿。
> ⚠ **过程中修正的自证缺陷**：脚本初版用 `Array.prototype.includes` —— 共享 Rhai 引擎未注册该方法
>   （测试当场报 `Function not found: includes`），生产链只有 `contains` 先例 ⇒ 已改 `contains`。
>   若只跑 `all_rhai_scripts_compile`（纯 compile 门禁）这一条**不会暴露**，因为它只在求值期解析函数。
> **未做**：重启应用真跑四档核对（Phase 7）；提交（工作树混有他人未提交改动，按用户决定暂不提交）。

## 〇、目标与判据

> ⚠ 本节中「**输入有周期**」一环已被 **〇-B（v2）** 取代：周期**不是**分析入口参数，一次分析恒产四档。其余「结论标周期 → 验证按周期 → 经验带周期」三环不变，且 v2 让第三环更硬（教训与档位参数都按档归属）。

四档周期 `ultra_short / short / mid / long`（2 / 5 / 28 / 90 交易日）要在四条链路上做到「**输入有周期 → 结论标周期 → 验证按周期 → 经验带周期**」。

验收总闸（每条不满足即该阶段未完成）：

- **G1 唯一权威源**：全仓只有一份周期枚举 + 一份天数表，其余为引用；
- **G2 显式**：任何结论都能回答「这是给哪个周期的」，用户未选周期时不出现「看似有周期、实为硬编码」的形态；
- **G3 不跨周期污染**：经验/教训/权重回流必须带周期边界，短线的教训不得以无标签形态进入长线 prompt；
- **G4 退化即声明**：某周期在某链路上结构性不支持（如 serenity 不做超短、ultra_short 无独立评分源），必须显式声明而非静默兜底成别的周期。

## 〇-B、v2 主线修订（2026-09-28，用户裁定，取代原「周期作为分析入口参数」的设计）

修订四条（用户三条裁定 + 由其派生的一条咬合约束）：

1. **分析入口不按周期分流**：前端**不**把周期作为参数输出，一次股票分析**恒产四周期决策**
   （既有载体 = `stock_analyses.horizon_decisions` + `portfolio-mgr.rhai` 阶段2）。
   「本轮主周期档」降级为**摘要/标签**，且**不再由 LLM 自报**（见第 2 条）。
2. **四周期决策 100% 本地公式**：档位与动作均由 `portfolio-mgr.rhai` 计算，**LLM 不参与定档**
   —— `trader.content.verdict.timeHorizon` 从决策输入降级为**仅存档展示**
   （`portfolio-mgr.rhai` 不再读 `trader_time_horizon` 定主周期，主周期改由**确定性规则**
   取：沿用既有后验阈值映射 `posterior ≥0.65→long / ≥0.50→mid / ≥0.35→short / else ultra_short`，
   该映射本来就在公式里，删掉 trader 分支即成唯一路径）。
3. **档位参数进入「股票分析设置」并纳入反思优化**：现硬编码在 `portfolio-mgr.rhai` 的
   `sl_pct_for` / `tp_pct_for`（超短 3/5、短线 5/10、中线 8/18、长线 12/30）**变量化**，
   按 `sl_pct_{period}` / `tp_pct_{period}` 命名（与 `StockAnalysisConfigPanel.tsx` 既有的
   `*_ultra_short_*` 逐周期参数同一形态），走「新增可调参数三处齐备」规矩
   （① `seed_variables.rs` 定义；② 脚本 `present(x){x} else {默认}` 守卫；③ 登记
   `PORTFOLIO_MGR_TUNABLE_PARAMS`）⇒ 设置面板自动分组、`reflection.rs:77` 的**可调清单自动含它**
   ⇒ 反思可建议改档位、`apply_param_suggestions` 可回写（既有闭环，不新建通道）。
   ⚠ 参数**按档独立**：每档一套值，反思建议必须点名档位，否则一次建议会污染其余三档
   （这正好与第 4 条互相咬合）。
4. **反思收窄为单周期**：一次反思只对一个周期做深度复盘，**不把四份决策一起塞进 prompt**。
   现状即只喂 `primary_snapshot`（`reflection.rs:1720-1765`），但「本轮周期」来自
   `decision_time_horizon`（模型自报）⇒ v2 改为**显式入参**：`run_reflection_now`
   （`stock_analysis.rs:5208-5216`）与 `run_reflection_workflow` 增 `horizon: Option<Period>`，
   缺省时取第 2 条的确定性主档；批量/定时反思沿用既有 `ReflectionFilter::matches`
   （`reflection.rs:1577-1595`）的按档最近邻筛选，一个任务只跑一档；
   `horizon_results_json` 保留四份**客观判定**（统计层，见 Phase 5），但**反思叙述与教训
   只产本轮那一份** ⇒ 原 Phase 5「消除⑥：四周期共享同一份 reflection」改判为
   **「一次反思 = 一个周期，四周期共享同一文本」这一形态不再存在**（共享的前提被单周期化消解）。

### Phase 1（v1 形态）的返工清单

| v1 已落地 | v2 处置 |
|---|---|
| `run_stock_workflow` 的 `userTimeHorizon` 入参 + 命令层校验 + hooks 注入 `user_time_horizon` | **删**（周期不是分析入口参数） |
| `trader.md`「本轮持有周期约束」段（`{{user_time_horizon}}`） | **删**，改写为「周期由系统公式判定，你自报的 `timeHorizon` 仅作记录、不参与定档」 |
| `portfolio-mgr.rhai` 的 `horizon_user_locked` 锁档段 + `horizonSource` 输出 | **删锁档**；`horizonSource` 值域改为 `model`（旧数据）/ `formula`（v2 主档），来源仍由公式单点判定 |
| `seed_variables.rs` 的 `user_time_horizon` 变量 + seed `input_mapping` 同名映射 | **删**（不留无消费者的注册项） |
| `sector/sentiment/social-media-analyst.md` 里的 `{{user_time_horizon}}` 措辞 | **改**为「本轮分析同时产出四档决策，你的报告对四档共同负责」的措辞，不引用已删变量 |
| `stock_analyses.decision_horizon_source` 列 + `extract_horizon_source` 三写入口 | **保留但改语义**（`model`/`formula`），它是「主档怎么来的」唯一留痕，Phase 5 单周期反思的缺档依据 |
| 搜索栏 Segmented 四档 + `analysisHorizon` store 状态 | **删选择器**；store 字段移除；`selectedHorizon` 死字段的处置改为「NL 解析出的周期**只作展示标签**」（保留 i18n 错键修复） |
| Phase 0 全部（`Period` 权威源、`horizon_days_json` 注入、`days_for` throw、e2e 夹具 3→2） | **保留**（与 v2 正交，且是第 3 条参数化的前置） |
| i18n `periodAuto` / `analysisHorizonHint` × 11 语言 | `analysisHorizonHint` **删**（无入口）；`periodAuto` 若展示层不再需要「自动」档一并删，改完用 parity 门验零残留 |

> v2 下「闭环」的定义随之收敛为：**分析恒产四档 → 每档独立客观验证 → 反思逐档产出教训与档位参数建议 → 建议回写该档设置**。

## 一、现状断点（核查结论）

| # | 断点 | 证据 |
|---|---|---|
| ① | 分析工作流**无周期入参**：`run_stock_workflow` 参数不含 horizon；NL 解析出的周期写进 `selectedHorizon` 后**全仓零消费** | `src-tauri/src/commands/stock_workflow/core.rs:194-212`、`src/stores/feature/stockAnalysisStore.ts:1129-1148`、`src/components/stock-analysis/StockSearchBar.tsx:136`（全项目唯一出现处，已复核） |
| ② | 主周期由 trader LLM **自报**，prompt 无模板变量、示例写死 `"mid"` | `stock_workflow/decision.rs:607-622`、`agency_experts/stock-analysis/trader.md:75,107,166-173` |
| ③ | 证据权重 HorizonLayer 已实现但**不进主决策**（仅前端交叉验证旁路），且 `mid` 落 `_` 默认档 | `crates/analysis-engine/src/evidence_weight.rs:245-315,621-626,640`、`src-tauri/src/commands/stock_analysis.rs:5894-5900`、`src/lib/stock-analysis-utils.ts:102` |
| ④ | 估值（DCF/格雷厄姆）与周期**完全无关、无短周期警示** | `crates/astock-data/src/mcp_tools.rs:2686`（`ideal_buy_price = dcf_mid × 0.70`）、`src-tauri/src/commands/portfolio-mgr.rhai:638-659,811-836` |
| ⑤ | 反思**判定/统计分周期，经验不分周期**：`reflection_lessons` 无周期列（已复核：该文件 grep `horizon\|period` 零命中），去重键与消费入口均无 horizon | `crates/entities/src/reflection_lessons.rs:8-36`、`stock_workflow/reflection.rs:1264-1273,1298-1311`、`stock_workflow/core.rs:2211-2244,2311-2359` |
| ⑥ | 反思 LLM 文本**四周期共享同一份**，只看主周期快照 | `stock_workflow/reflection.rs:1720-1765,2861-2871,2938-2948` |
| ⑦ | serenity 链无周期入口，落库**硬编码 `mid` / 20 天 / 止损 ×0.80** | `stock_workflow/serenity.rs:1350-1355,1361,1368,1384`、`commands/recommendation_cron.rs:247-279` |
| ⑧ | 策略包 YAML 为**死数据**：`StrategyPackRegistry/template_vars_for/load_packs_from_dir` 零调用者（已复核），无「应用策略包」命令 | `crates/analysis-engine/src/strategy_pack.rs:195-239`、`commands/strategy_pack.rs:34-191`、`src-tauri/src/register_commands.rs:1348-1354` |
| ⑨ | 天数表/周期枚举**多份副本** | `recommender/types.rs:101-108`、`analysis-engine/reflection_stats.rs:96-104`、`portfolio-mgr.rhai:2517`、各策略 `period_defaults`；第二套枚举 `crates/harness/src/strategy_pack.rs:84-108`（`rename_all="lowercase"` 会产出 `ultrashort`） |
| ⑩ | ultra_short 是**退化档**：复用日线单后验、置信下限 0.35、无独立评分源 | `portfolio-mgr.rhai:2601-2626,2630-2634` |
| ⑪ | 风控 SKILL 只有两档（-5% 短线 / -8% 中长线），与四档表不一致且实际链路不消费 | `agency_experts/skills/risk-management/SKILL.md:50-63` |

已成立的部分（不动，仅作复用参照）：荐股链四档从入口到定时全通（`commands/stock_analysis.rs:4419-4439`、`recommender/mod.rs:545-576,627-658`、`recommender/scoring.rs:128-159`、`recommendation_cron.rs:30-34`）；四周期独立快照与成熟判定（`reflection.rs:2290-2300`）；命中率按周期分组各带 MIN_SAMPLE=5（`analysis-engine/reflection_stats.rs:20,315-320`）；分析师权重按 (analyst, horizon) 回流（`backtest_feedback.rs:122-131`）；前端四周期 Tabs 先例（`RecommendationPanel.tsx:333-337,379-384`）。

## 二、设计总则

**周期三态**（贯穿所有链路的统一语义）：

- `user` —— 用户显式选定的周期，是**硬约束**：下游只产该档决策为主结论，其余三档降级为「参考」；
- `auto` —— 未选（缺省）：维持现状由 trader 自报，但结论必须带 `horizon_source: "model"` 标记，面板明示「周期由模型判定」；
- `fixed` —— 结构性单档链路（serenity）：必须显式声明它只服务哪几档，不得静默写 `mid`。

**权威源单点**（G1）：`crates/analysis-engine/src/recommender/types.rs` 的 `Period`（枚举 + `ALL` + `default_holding_days` + `factor` + `nearest_for_holding_days`）为唯一源；Rust 侧其余表改为引用，Rhai 侧由 Rust 注入常量而非脚本内硬编码。

## 三、实施阶段

### Phase 0 权威源单点化（低风险，先做）

- 删 `crates/harness/src/strategy_pack.rs:84-108` 的 `StrategyPackPeriod` 副本，改用 `Period`（harness 是 foundation，**不得反向依赖 analysis-engine** ⇒ 落地方式：把 `Period` 上提到 harness，analysis-engine 与 strategy_pack 均 `pub use` 引用；这是 AGENTS.md「权威定义在 harness」的既有惯例）；
- `analysis-engine/reflection_stats.rs:96-104` 的 `default_expected_holding_days` 改为委托 `Period::default_holding_days`（`reflection.rs:1543-1544` 已明令禁止重复定义，本阶段落实）；
- `portfolio-mgr.rhai:2517` 的天数表改由 Rust 注入：`stock_workflow/hooks.rs` 新增变量 `horizon_days_json`（object），`seed_stock_analysis.rs` 的 `portfolio-mgr` 节点 `input_mapping` 加同名映射（照抄 `:4404 ("screening_source","screening_source")` 形态）；Rhai 侧 `_=>5` 兜底分支删除，缺失即报错（缺数 ≠ 默认，避免静默错档）；
- **bump** `seed_stock_analysis.rs:518 TEMPLATE_VERSION`（94 → 95）。

验收：`grep` 全仓天数硬编码只剩 `Period::default_holding_days` 一处 + Rhai 消费点；模板重建后既有分析可正常跑（用 `src-tauri/src/bin/axagent-batch-rerun.rs` 回放一只票）。

### Phase 1 分析工作流周期入口打通（核心）

> **本节为 v1 形态，已被 〇-B（v2）取代**：v1 代码已落地（见文首落地记录），返工按 〇-B 末尾的「Phase 1（v1 形态）的返工清单」执行；本节保留作为沿革与行号索引。

- 后端：`run_stock_workflow` 增参 `time_horizon: Option<Period>`、`horizon_source: Option<String>`（`stock_workflow/core.rs:194-212` 与共用实现体 `:375-393`）；写入 `RunOptions.input`（`:944-950`）并经 `hooks.rs:706-722` 的 `input_str` 读出，push 为 `user_time_horizon` 变量；
- 前端：`stockAnalysisStore.ts:1023-1026,1129-1148` 增 `timeHorizon` 入参；`StockSearchBar.tsx:119-136` 的 NL 解析结果**接到真实入口**（消除断点①的死字段），并在搜索区加四档 + 「自动」选择器（复用 `RecommendationPanel.tsx` Tabs 形态，i18n 复用 `stockAnalysis.periodUltraShort…` `zh-CN.json:15299-15302`，新增 `auto/自动` 一档需 11 语言齐补）；
- 约束传导：`trader.md` 加 `{{user_time_horizon}}` 模板变量（变量注册进 `seed_variables.rs:50 build_template_variables`，默认值 `"auto"`），声明「选定周期时 `timeHorizon` 必须等于该值」；`decision.rs:607-622` 在 `horizon_source == "user"` 时**覆盖** LLM 自报值；
- ⚠ 大括号转义：`trader.md` 走 `format!`/`render_prompt`（`prompt_template.rs:193`），示例 JSON 的 `{}` 必须 `{{}}` 转义——本仓已有两次烧穿事故（commit `771d734fc`）。
- 落库：`stock_analyses` 增 `decision_horizon_source` 列（走实体声明 + `schema_diff::heal_all` 自愈，**不新建迁移**，见 `crates/dao/src/migrations/mod.rs:21`；旧行 NULL ⇒ 读侧按 `"model"` 解释）。

验收：选定「超短线」跑一只票 ⇒ `decision_time_horizon='ultra_short'` 且 `horizon_source='user'`；选「自动」⇒ 仍由模型判且面板显示「模型判定」；`e2e/stock-reflection-four-horizon.spec.ts` 不回归。

### Phase 2 专家 prompt 与风控档位对齐四档

> **v2 影响**：本节里「向 trader 注入 `{{user_time_horizon}}` 硬约束」这条**作废**（〇-B 第 1/2 条：周期不是入口参数、LLM 不定档）；trader 侧改为显式声明**「持有周期由系统公式判定，你自报的 `timeHorizon` 仅作记录，不参与定档」**，sector/sentiment/social-media 的周期措辞改为「本次分析同时产出四档决策，你的报告对四档共同负责」。
> **仍然成立并已落地的部分**：风控 SKILL 的两档 → 四档对齐、trader 档位表加「公式才是权威」注、trader 示例 JSON 的 `"mid"` 加「示例值非默认」注。**追加要求**：SKILL 里那四个数字与变量化后的
> `sl_pct_{period}` 默认值同源，故 SKILL 必须写「默认值见设置面板 / `seed_variables`」而非把数字再抄一遍。

- `trader.md:75` 写死的 `"timeHorizon": "mid"` 示例改为变量化或中性化；`:166-173` 止损表与 `portfolio-mgr.rhai:2515-2519` 档位表核对，不一致以 Rhai 表为准并在 prompt 注明「档位由后端裁定，模型只给方向与理由」；
- `sector-analyst.md:20`、`sentiment-analyst.md:41`、`social-media-analyst.md` 等硬编码「短线」措辞改为随周期语境（最小做法：注入 `{{user_time_horizon}}` 一句「本次分析服务周期」，不做逐分析师文案重写）；
- `skills/risk-management/SKILL.md:50-63` 两档 → 四档，数值对齐 `portfolio-mgr.rhai` 的 `sl_pct_for`；
- **改 md 即重启 UPSERT 入库**（既有机制，无需额外迁移）。

验收：五类专家 prompt 渲染产物 grep 无残留写死周期；同一票 `auto` 与 `user=long` 两次运行，trader 输出的 `timeHorizon` 分别为「模型自选」与「long」。

### Phase 3 证据权重 HorizonLayer 进主链

- 时机：analysts 输入只有分析跑完才有 ⇒ HorizonLayer 的主链消费点是 **portfolio-mgr（决策合成节点）**，不是前置；
- 做法：在 `stock_workflow/hooks.rs` 决策后置钩子或 portfolio-mgr 之前的 Rust 侧调用 `compute_evidence_weights`（`evidence_weight.rs:640`），把 `recommended_position_pct` / `hold_gate` push 成变量注入 Rhai（先例：`hooks.rs:259-268 serenity_context`、`reflection.rs:496-529` 的 object 变量注入）；
- 补齐 `evidence_weight.rs:245-315` 的 `mid` 档显式分支（当前落 `_` 默认档 = 静默兜底，违反 G4）；`:621-626` 周期仓位乘数与 `recommender/scoring.rs:128-159` 的 `period.factor()` 语义对齐，两套乘数关系必须写清是「同一件事的两个实现」还是「不同链路口径」，若前者则合并、若后者则各自注明作用域；
- 前端镜像表 `src/lib/stock-analysis-utils.ts:1118-1185` 的 ultra_short 缺 `a-technical` 键补齐（不再靠 `:1193-1196` 后缀模糊匹配兜）。

验收：同一份 analysts 输出，四个周期算出四个不同的 `recommended_position_pct`，且该值真实进入最终决策仓位（而非仅交叉验证展示）。

> **v2 追加（〇-B 第 3 条）—— 周期档位参数化，与本节同一落点（portfolio-mgr）**：
> - `sl_pct_for` / `tp_pct_for` 的手抄表改为「可调变量 + 表内默认」双层形态：
>   `sl_pct_ultra_short` / `sl_pct_short` / `sl_pct_mid` / `sl_pct_long`（`tp_pct_*` 同），
>   每档在 `seed_variables.rs` 注册、脚本里 `present(x) ? x : 默认`、并登记进
>   `PORTFOLIO_MGR_TUNABLE_PARAMS`（28 → 36 项）⇒ 设置面板与**反思可调清单**同时自动生效；
> - `days_for` 的**档位语义**仍锁 `Period::default_holding_days`（Phase 0 权威源），
>   **不**做成自由可调数字 —— 它是逐档成熟判定的基准，被改一次就废掉全部按档统计；
> - 反思建议必须**点名档位**（`sl_pct_mid` 而非 `sl_pct`）⇒ 单周期反思（〇-B 第 4 条）
>   只碰本轮那一档的参数，杜绝一次建议污染其余三档。

### Phase 4 估值口径的周期闸口

- DCF 本身是长期现金流折现口径，短周期下不应作为「理想买入价」的等价依据（`mcp_tools.rs:2686` 的 `ideal_buy_price = dcf_mid × 0.70` 恒用 mid 档）；
- 做法：估值结论按周期降权并显式声明——`horizon ∈ {ultra_short, short}` 时，DCF/graham 腿在 panel 与决策理由中标注「长期估值口径，仅供 X 以上周期参考」，不参与短周期主结论（`portfolio-mgr.rhai:811-836` 的 0.7/0.3 融合权重按周期分支）；
- 与已修的 DCF 适用性闸（`PLAN-dcf-fallback-anchor-applicability.md`）叠加，不冲突：那个闸管「模型成不成立」，本闸管「结论服务哪个周期」。

验收：给一只 DCF 适用的高估值票跑 `ultra_short` ⇒ 面板仍显示 DCF 价，但理想买入价/上行空间带周期限定语，且短周期主结论不被 DCF 驱动。

### Phase 5 反思经验闭环的周期维度（最重）

- 表：`reflection_lessons` 增 `horizon` 列（实体加字段即自愈）+ `stock_workflow/reflection.rs:1264-1273` 去重键改 `(stock_code, horizon, lesson_summary)`、`:1298-1311` 写入填值；`extract_lesson_to_rule` 签名增 `horizon: Option<Period>`（NULL = 周期无关，读侧两种解释都要有）；
> **v2 前置（〇-B 第 4 条）**：一次反思 = 一个周期。`run_reflection_now`（`stock_analysis.rs:5208`）
> 与 `run_reflection_workflow` 增 `horizon: Option<Period>` 入参，缺省取公式主档（不再读 LLM 自报值）；
> 原「消除断点⑥（四周期共享同一份 reflection_json）」**改判为不需要**——单周期化后「共享」形态不存在，
> 落库时把该次反思的 `horizon` 写进 `horizon_results_json` 对应 key，其余三档仍由**客观判定**独立产出。

- 消费：`core.rs:2211 fetch_stock_lessons(stock_code, db)` 增 horizon 过滤——同档优先、跨档不进 prompt（G3）；`:2232-2244` 的跨 ticker 近 7 天注入同样带档；`:2311-2359 fetch_rule_lessons` 置信度榜按 (horizon) 分档取；
- 产出：反思 LLM schema（`stock_analysis_setup/mod.rs:1697-1715`）增 `lesson_horizon` 字段（要求模型判定该教训属哪档、可多档），或由 `reflection-comparator.rhai:301-363` 已有的 per-horizon 判定结果**确定性派生**（优先后者，不依赖模型自觉）；
- 消除断点⑥：`reflection.rs:2861-2871,2938-2948` 四周期共享同一份 `reflection_json` 改为逐周期写入各自的判定段（至少把 comparator 的 per-horizon 结论落到对应 key，文本正文可共享但必须标 `for_horizon`）；
- `strategy_performance` 一次反思只写 1 行 `period="reflection"`、`horizon_pnl_json: Set(None)`（`reflection.rs:706-721`）改为逐周期 N 行或填 `horizon_pnl_json`；
- **bump** `mod.rs:1894 REFLECTION_TEMPLATE_VERSION`（2 → 3）。

验收：同股先跑一次 `ultra_short` 反思再跑 `long` 分析 ⇒ 注入的 `【规则化教训】` 只含 long 档与 NULL 档，不含 ultra_short；四周期 Tab 的反思详情不再显示同一份文本。

### Phase 6 serenity 周期口径显式化 + 策略包接线或删

- serenity：`strategies/serenity.rs:22-29` 明确「只做中长线」是**产品定位**还是**实现偷懒**？若是定位 ⇒ `stock_workflow/serenity.rs:1361,1384` 的硬编码 `"mid"` 改为常量声明 + 面板明示「趋势智选仅服务中线」，`holdingDays` 由 `Period::default_holding_days` 推导而非 `unwrap_or(20)`（20 ≠ 28，是口径矛盾）；若可支持四档 ⇒ `run_serenity_screening`（`:883-890`）与 `stock_create_trend_screening_cron`（入参 `TrendCreateRequest`，`recommendation_cron.rs:221`；核心 `create_trend_screening_cron_core` `:292`）增 `periods: Vec<Period>`（照抄荐股 cron `:30-34` 形态），前端 `SerenityScreeningPanel.tsx:970-980` 传参 + 四档 Tab；止损/目标按档分档（当前 `:184-197` Mid/Long 共用一组）；
- **bump** `seed_serenity.rs:104`（53 → 54）、`seed_serenity_fast.rs:117`；
- 策略包：6 个周期 YAML（`agency_experts/stock-analysis/strategy_packs/*.yaml`）二选一——(a) 接线：新增 `apply_strategy_pack` 命令，把 `template_vars_for`（`strategy_pack.rs:195-239`）产出的变量经 `mod.rs:1386 force_variable_value` 写入模板变量并 bump 版本；(b) 删除：连同 `StrategyPackRegistry` 死代码一起摘除，设置面板只保留现有逐周期参数（`StockAnalysisConfigPanel.tsx:584-606,669-673` 已经在做同一件事）。**倾向 (b)**：它想表达的 per-period 参数已被硬编码取代（`strategies/trend.rs:3-5,17-18` 注释自述），接线等于复活一条已废弃口径。

验收：趋势智选产出的每条 pick 的 period 与其止损/持有天数自洽；策略包面板不再出现「可编辑但永不生效」的控件。

### Phase 7 门禁与回归

- 阶段内：`cargo check` + 定点 `cargo clippy -p <crate> --all-targets -- -D warnings`；
- 全部阶段完成后按 CI 口径一次跑全量：`cargo fmt --all --check`、`cargo clippy --workspace --all-targets --all-features -- -D warnings`、`__TAURI_WORKSPACE__=true cargo test --workspace`、`npm run lint:eslint`、`npm run typecheck`、`npm run format`、i18n 齐备性（`check-i18n-key-parity.mjs` + `check_i18n.py`）、`scripts/check-single-source-facts.mjs`；
- 新增/变更检法：在 `.workbuddy/memory/MEMORY-RULES.md` 追加判据（「周期硬编码兜底」类缺陷的检出法）；
- 真实核对：`tauri dev` + `axagent-batch-rerun` 对同一票跑四档，逐档看结论是否真的不同（**只有静态绿不能算完成**）。
- 附带收编（已复核）：`e2e/stock-reflection-four-horizon.spec.ts:40,91` 夹具把 ultra_short 的 `expectedHoldingDays` 写成 **3**，而权威表是 **2**（`recommender/types.rs:101-108`）——该 e2e 正在锁一个错误天数，Phase 0 落地后一并对齐（纯 UI 展示数据，不影响计算口径，但留着会误导后续夹具）。

## 四、风险与注意事项

- **命令注册两步**：新增 `apply_strategy_pack` 等命令只改 `commands/mod.rs` + `#[tauri::command]`，**禁手改 `src/register_commands.rs`**；`commands/mod.rs` 对含命令的子模块只能 `pub use sub::*;` 或逐条列出（AGENTS.md 禁区 7）。
- **IPC 参数 camelCase**：前端传 `timeHorizon`，不是 `time_horizon`（禁区 13）。
- **schema JSON 键名**：`llm-output-schemas.json:148-151,226-245` 是 camelCase（`timeHorizon`/`decisionsByHorizon`），实体列是 snake_case，两侧靠 serde 注解桥接，改一侧必须查另一侧消费点。
- **i18n 11 语言**：新增周期标签/限定语必须 11 种语言齐补（禁区 2），locale 文件落盘必须 LF（禁区 14）。
- **模板版本 bump 是硬要求**：改种子必 bump（项目记忆），四个模板版本互相独立，别漏。
- **Phase 0 的 `Period` 上提 harness** 是跨 crate 改动，会牵动 analysis-engine / harness / entities 三处，务必先跑 `cargo tree` 确认不引循环依赖（harness 零 axagent-* 依赖）。
- **Phase 1 与 Phase 3 有顺序依赖**：先有 `user_time_horizon` 变量，HorizonLayer 才有确定的输入周期；Phase 5 依赖 Phase 1 的 `horizon_source` 列。

## 五、建议实施顺序与工作量

Phase 0 → 1 → 2 → 3 → 4 → 5 → 6 → 7。0/2/4 较小（各约半天），1/3 中等，5 最重（表 + 产出 + 消费三处），6 取决于策略包选型。

## 六、待拍板项（请点单）

1. **没选周期时的缺省语义**：维持「auto = 模型自报 + 面板明示」，还是改为强制要求用户选档（无 auto）？建议前者（兼容存量数据与定时任务）。
2. **用户选定周期时，其余三档决策是否继续产出**：建议继续产但降级为「参考」（现有 `horizon_price_map`/`horizon_decisions` 四套结构不动），只把主结论钉在选定档。
3. **serenity 定位**：结构性只做中长线（则只需显式声明，Phase 6 前半走轻量路线），还是补四档能力（需给超短/短线做止损分档）？
4. **策略包 YAML**：接线复活，还是连同死代码一起删除？（本 PLAN 倾向删除）
5. **ultra_short 退化档（断点⑩）**：本轮是否补 60 分钟级评分源（工作量大、牵动数据采集），还是仅在面板声明「超短线复用日线后验、置信下限 0.35」？

## 七、裁定与收口记录（2026-09-29，Phase 7）

### 7.1 待拍板项 4 的裁定：人工策划的预设矩阵**不接入**，随包废弃

被删的 18 个 YAML 就是一张人工策划的参数预设矩阵（档位分布 ultra_short 4 / short 9 / mid 2 / long 3）。不接的判据：

- 它想表达的字段（`period` + 权重/仓位/风控预设）与本 PLAN 已落地的权威源**逐项重复**：周期枚举与天数/乘数在
  `src-tauri/crates/harness/src/holding_period.rs`，证据权重在 `analysis-engine/src/evidence_weight.rs::get_horizon_base_weights`，
  分档止损/止盈在 8 个模板变量 `sl_pct_*` / `tp_pct_*`（已进设置面板、且已在反思可调参数清单里）。
- 接线等于**复活一条已废弃口径**（`strategies/trend.rs` 注释自述其参数已被硬编码取代），并新造第二条定档/给参通路
  —— 与 G1「唯一权威源」、G2「显式」直接冲突；而它原本就是「可编辑但永不生效」的面板（判据 #702）。
- 接入还要新增「应用策略包」写模板变量的命令 + UI，**收益面为 0**：同一参数已有唯一一处可调、可被反思优化。

结论：**不接入，随包废弃**，与 3 份 `strategy_pack.rs`、`StrategyPackSettings.tsx`、7 个命令、11 语言 i18n 块一并清除。
需要回溯时从引入提交 `21e73fe58`（2026-07-16）取，**不在本仓另存副本**。

### 7.2 把「自我造成的缺陷」升级为**机器门**（散文判据不可执行 = 没有门）

- `src/commands/stock_analysis_setup/seed_consistency_tests.rs::seed_node_constructors_are_not_duplicated`
  —— 正则扫 `seed_stock_analysis.rs` 全文，任何 `id: "xxx".into(),` 出现 >1 次即红（重复块后者静默覆盖前者）。
  测试内自带**负控**：在内存里把 pm 构造块复刻一份，必须报出 `portfolio-mgr == 2`，否则门是空转的。
  预演实测：事故形态 ⇒ `{'portfolio-mgr': 2}`，干净树 ⇒ `{}`。
- `crates/rt-workflow/tests/rhai_syntax_check.rs::fn_bodies_do_not_read_injected_workflow_vars`
  —— 锁 `portfolio-mgr.rhai` / `portfolio-risk-gate.rhai`：不得出现 `fn horizon_const`；必须保留闭包定义；
  调用点 `by_name == 0` 且 `by_call >= 2`（本仓 Rhai 按名调用闭包恒 `Function not found`）。

**这两道门自己第一次跑就红了**，红因是我把 Python 习惯带进 Rust：`str.index()`（Rhai/Rust 无此方法）
与把 `"` 直接放进 `r"..."`（正则串被提前终止 ⇒ 16 个连锁编译错误）。
比这更值得记的一条：后台任务回执 **exit code 0**，而日志正文是 `error: could not compile` ——
再次实证「回执退出码不能当判据，必须读日志正文」。

已验证：`cargo check -p axagent --all-targets` 绿；
`__TAURI_WORKSPACE__=true cargo test -p axagent --lib seed_node_constructors_are_not_duplicated` 1/1；
`__TAURI_WORKSPACE__=true cargo test -p axagent-rt-workflow --test rhai_syntax_check` 9/9。

### 7.3 收口第二轮（2026-09-29，按用户「按建议值收口」）

自查发现本轮 v97 留下两处**未闭合的口径**，均已修：

1. **四档独立决策的档位数值接回唯一通路**（`TEMPLATE_VERSION` **97→98**）
   —— `decisionsByHorizon` 的 short/mid/long 原先传字面量（5/10/5、8/18/28、12/30/90）、
   超短线块硬写 3/5，于是面板调档与反思建议只作用于主决策与 `horizonPriceMap`，
   四档 Tab 不动 ⇒ 同一份 JSON 里档位%与价位反解打架。现全部改经
   `sl_pct_for.call(h)` / `tp_pct_for.call(h)` / `days_for.call(h)`（`portfolio-mgr.rhai`），
   并订正该文件里「档位数值只出现一遍」这句**当时并不成立**的自述。
   新门 `crates/rt-workflow/tests/portfolio_mgr_tier_params_rhai.rs`（4/4）：逐字副本防漂移 +
   默认值/调档生效的行为测试 + **块窗口静态锁**（`decisions_by_horizon` 段内三张函数必须逐档出现、
   字面量形态禁止回来）。门的判别力用 HEAD 的真实旧形态自证：旧文件在该锁下报 **9/9 缺项**
   且反向锁命中 ⇒ 门不是空转。
2. **历史记录不再只显示一条无归属的 Action**
   —— 后端 `StockAnalysisListItem` 补 `decision_time_horizon` + `decision_horizon_source` 两列
   （仍走实体声明 + schema 自愈，无迁移）；两处历史入口（`HistoricalAnalysisPanel.tsx`、
   `AnalysisHistoryButton.tsx`）行内并列：主档标签 +（公式定档 / 模型自报）来源徽标，
   缺列的老记录显式显示「未知周期」而非回退成某档；四档 Action 从 `decisionJson.decisionsByHorizon`
   解析后逐档出小 Tag（无该结构则**整行不渲染**，不伪造）。
   显示名沿用既有键 `stockAnalysis.timeHorizon*`（与 `DecisionBanner` 同一批，未另起一套）；
   新增键 2 个 × 11 语言（`horizonSourceFormula` / `horizonSourceModel`），parity 15264 键一致。
   helper 收敛在 `src/lib/stock-analysis-utils.ts`（`HORIZON_T_SUFFIX` / `horizonSourceLabelKey` /
   `readHorizonActions`），两个组件共用；前端新增 `stockAnalysisHorizon.test.ts` 5 用例（含坏 JSON、
   snake_case 兼容、部分档缺失）。

**仍未闭合（已查明、未批不改）**：`stockAnalysis.reflection.horizonUltraShort` 在 11 语言里写的是
「超短线 (1-3 天)」，而 `Period` 权威持有天数=2（Phase 0 已把夹具 3→2）⇒ 反思面板与档位口径同屏打架。
属文案订正（4 键 × 11 语言），等拍板。

**过程中再次自伤并已修复**：用 python 在 `AnalysisHistoryButton.tsx` 插块时，
`str.replace` 的目标串（24 空格 + `</div>`）是更深层那行（30 空格 + `</div>`）的**尾子串**
⇒ 插到了错误位置并留下悬空 `);`。按行号断言区间还原后，`git diff` 复核确认只剩 4 个预期 hunk。
教训与 [[feedback-edit-cjk-anchor-hazards]] 第 2 条同族：**缩进前缀会互相吞并，锚点必须整行唯一起手**。

### 7.4 收口第二轮的附带发现：新登记参数漏了三个「契约面」（全量 Vitest 抓出）

`npx vitest run` 首轮 3 红，全在 `src/components/settings/StockAnalysisConfigPanel.test.tsx`，
**红的是我 Phase 1 登记的 8 项档位参数**——三处都不是功能缺陷，而是契约面没收口
（`cargo check` / `all_rhai_scripts_compile` / `typecheck` 对这三处全绿）：

| 派生点 | 缺了什么 | 修法 |
|---|---|---|
| 运行时注入白名单 | `horizon_consts_json` 由 `hooks.rs` 无条件注入、不是模板变量，却不在 `RUNTIME_INJECTED` ⇒ 报「同名映射指向不存在的变量」 | 入白名单并写明注入点与「缺它即 throw」 |
| 面板分组可解析性 | 我把注释写在分组的 `label:` 与 `vars: resolve([...])` **之间**，而门按「label 的下一行就是 vars」匹配 ⇒ 整组不被识别，表现为「没有 UI 入口」 | 注释挪到 `tool:` 之前，并在面板里留警示注释说明这条解析约定 |
| 生效快照观测面 | `portfolio-mgr.rhai` 的 `effective_params` 未输出这 8 项 ⇒ 反思看得到档位效果、看不到档位本身取值 | 补 8 行输出 `*_v`（本次实际生效值，含缺省兜底） |

⇒ 判据：**新增可调参数按五点对账** —— seed 变量 / `input_mapping` / 面板 `b()` 声明 / 面板分组 `resolve([...])` /
`effective_params` 输出。已登记为判据 **#705**（撞号复查：`^[0-9]{3,4}\. ` 全集无重复）。

### 7.5 最终门禁表（2026-09-29，覆盖本 PLAN 前文所有「待补」表述）

| 门禁 | 口径 | 结果 |
|---|---|---|
| `cargo fmt --all --check` | 全量 | **EXIT=0**（首跑 `FMT_EXIT=1`，6 处 diff 全在我新写的两个测试文件 ⇒ 单文件 `rustfmt --config-path rustfmt.toml` 修正后复验 0 diff） |
| `cargo clippy --workspace --all-targets --all-features -- -D warnings` | 全量 | **EXIT=0** |
| `__TAURI_WORKSPACE__=true cargo test --workspace` | 全量 | **EXIT=0**，143 条 `test result:` 行**全部 ok**，0 FAILED |
| `npx vitest run` | 全量 | **182 文件 / 2056 用例全绿**（首轮曾 3 红，见 §7.4，修好后复跑全绿） |
| `npm run typecheck` / `npx oxlint src --max-warnings=0` / `npx dprint check`（改动文件） | | 全绿 |
| `bash scripts/check-line-endings.sh` | | 全绿 |
| `node scripts/check-i18n-key-parity.mjs` | 11 语言 | key 集合一致（15264 条） |
| `npm run contracts` / `check:types` / `check:paired` / `check:valuation-parity` | | errors=0 / 0 errors / 通过 / 5 处载体一致 |
| `cargo test -p axagent-rt-workflow`（4 个 portfolio-mgr Rhai 测试二进制） | 定点 | 5 + 4 + 4 + 9 全绿；格式化后复跑 `portfolio_mgr_tier_params_rhai` 4/4 |
| `cargo test -p axagent --lib commands::stock_analysis_setup` | 定点 | 38/38；格式化后 `seed_node_constructors_are_not_duplicated` 1/1 |
| `node scripts/audit-inject-coverage.mjs --strict` | | ② 无悬空映射 ✅；① 报 `ts` —— **审计脚本自身的假阳性**（`ts` 是 `horizon_decision` 闭包形参，其三个实参 `totalScore_short/mid/long` 确有 input_mapping，`seed_stock_analysis.rs:4200-4202`），修法待批 |
| `node scripts/check-single-source-facts.mjs` | | 1 条既存红：`domain_semantics.rs` → `evidence_citation.rs:198` 指空行（并行会话未提交文件，非我方） |
| `check:serde` | | 红在 `commands/analyst_feedback.rs` —— 已实证 **HEAD 既存**（该文件无未提交改动、HEAD 同样 0 处 `serde(rename_all)`、CI 不跑此脚本） |
| `npx playwright test --list` | | 128 规格可编译 |

**仍待你侧**：重启应用 + `axagent-batch-rerun` 四档实跑核对（Phase 7 末环）；提交切分。本轮**全部未提交**。
