# P3 实施计划：漏检 → 结构化反思与教训

> 所属总计划：`docs/plans/PLAN-batch-automation-mover-recall.md` §P3（L172-180）、验收 L243-244。
> 前置：P1（核查落库）/ P2（as-of 补分析批次）已完成并通过合并验证。

## Context（为什么做）

P2 产出的妖股漏检补分析行（`stock_analyses.source='mover_recall'`, `synthetic=1`,
`anchor_event_id`）已能自动生成每档 pending 反思行（`run_single_stock_analysis` 成功路径
`core.rs:2208` 统一建点），但反思链路目前：

1. **不区分样本来源**：`stock_reflections` 无 source 列，漏检回放反思与常规到期反思混在一个分母；
2. **没有漏检专项视角**：反思 LLM 不知道"这是一笔窗口涨幅达标但系统漏掉的票、权威卡点层是什么"，
   现有模板只有通用三字段（`what_went_wrong`/`missed_signals`/`fix_for_future`），
   无法产出 plan 要求的固定三问（启动前可见信号 / 卡在哪一层 / 未来判据如何修）；
3. **教训入模无门槛**：`extract_lesson_to_rule`（reflection.rs:1434）仅按长度 10-250 判据
   直接写 `reflection_lessons.status='active'`，"不知道/数据不足"类空洞教训也会注入未来决策
   （decision.rs:3284 只过滤 active）；`reflection_lesson_validator.rs` 现有函数全是**事后**
   置信度统计，没有入模前校验；
4. **前端看不到闭环**：`MoverRecallPanel` 漏检明细行是死表格，点不开补分析报告与反思结论；
5. 事件实体已预留 `reflection_id` 列与 `reflected` 状态（mover_recall_events.rs:60-64），尚无回填方。

目标：漏检样本进入独立 source 分桶的反思队列，带权威归因层完成固定三问，教训先校验后入模，
前端逐行可展开核验。

## 关键复用点（不新造机制）

- pending 入队：`build_pending_reflection_rows` / `PendingReflectionSeed`
  （`src/commands/stock_workflow/reflection.rs:1965/1862`），三通道已统一；
- 反思执行：`run_reflection_workflow`（reflection.rs:168），变量注入用
  `axagent_harness::workflow_types::Variable` push（reflection.rs:548 先例）；
- 反思专家 prompt：`agency_experts/stock-analysis/reflection.md`
  —— 经 `stock_analysis_setup/mod.rs:153` `include_str!` 编译期嵌入并 UPSERT 进 agency_experts，
  每次反思前 `ensure_stock_analysis_experts_seeded`（decision.rs:500）按版本自动升级；
- 入模写口：`extract_lesson_to_rule`（reflection.rs:1434），去重/档位/代际逻辑保持不动；
- 加列：bootstrap additive 自动 ADD COLUMN（dao/reconcile/apply.rs:868），
  照 P2 `stock_analyses.source: String` / `synthetic: i32` 裸字段形态，不写手写 migration；
- 事件状态机：`mover_recall_events.status` 已含 `reflected`、已含 `reflection_id` 列；
- 前端面板：`src/components/stock-analysis/MoverRecallPanel.tsx` misses 表（L240-304）；
- i18n 注入：`.workbuddy/tmp/add_mra_i18n.py` 先例（保序 + LF + ensure_ascii=False）。

## 实施步骤

### P3-1 数据层：stock_reflections 加两列

文件：`src-tauri/crates/entities/src/stock_reflections.rs`

- 加 `pub source: String`（紧跟 `status` 前或 template_version 后），文档注释写明值域
  `due`（常规到期反思：live/chat/回放补建）/ `mover_recall`（妖股漏检回放反思）；
  存量行 bootstrap 加列后落 `''`，**读侧分桶口径 = `source == 'mover_recall'` 一个桶，其余
  （含 `''`）一律 due 桶**，不依赖存量数据回填。
- 加 `pub miss_layer: Option<String>`：漏检权威归因层（`not_in_pool`/`l3_scored_out`/
  `other_period`/`unexplained`），由系统从事件盖章、LLM 不得自报；NULL = 非漏检反思。
- ActiveModel 构造点补齐（grep `stock_reflections::ActiveModel {` 共 3 处生产 + 1 处测试）：
  - `reflection.rs:249`（手动反思 INSERT 兼容路径）：`source: Set("due".into())`、`miss_layer: Set(None)`
  - `reflection.rs:2012`（`build_pending_reflection_rows_for`）：取 seed
  - `reflection.rs:3882`（测试 `mk_row`）：`"due"` / None
  - 另查 `src/bin/axagent-batch-rerun.rs:406` 附近 reset 是否构造全量 ActiveModel，若是则同补
- `PendingReflectionSeed`（reflection.rs:1862）加 `pub source: &'a str`（**不加** miss_layer，
  建点时拿不到事件层，收尾时盖章）；三构造点传值：
  - `core.rs:2209`：`source`（`run_single_stock_analysis` 现有参数，P2 已加，L1919）
  - `hooks.rs:1178`：`"due"`（对话通道）
  - `bin/axagent-batch-rerun.rs:359`：`"due"`
  - reflection.rs 测试内 seed 构造器（L3943 起 4 处）：`"due"`

### P3-2 入队核验（预计零代码）

P2 批次经 `run_single_stock_analysis` 成功路径已自动建每档 pending 行；
as-of guard 下 hindsight 按交易日外推（reflection.rs:2001），历史日期在批量反思扫表时
（`run_batch_reflection`，reflection.rs:1190，仅按 status=pending 扫、无 source 过滤）立即可处理。
本步仅做代码走读确认，不新增 cron（plan §5.5 矩阵：复用既有 18:00 反思任务）。

### P3-3 漏检固定三问 + 事件回填

文件：`src/commands/stock_workflow/reflection.rs`

1. `run_reflection_workflow` 加载 `original_analysis`（L333-337）之后：
   - 当 `original_analysis.source == "mover_recall"` 且 `anchor_event_id` 非空时，
     `mover_recall_events::Entity::find_by_id` 取事件（拿 `miss_layer`/`window_start`/
     `anchor_date`/`cum_gain_pct`/`period`）；查不到只 warn 不阻断。
2. **无条件**注入新变量 `mover_recall_context`（string，模板将引用，缺注入会 VARIABLE_NOT_FOUND）：
   - 命中事件：一段中文事实块——说明本次为漏检回放、权威卡点层（层码→中文语义）、
     窗口起止与累计涨幅、要求 LLM 回答固定三问（① `missed_signals` 只列 windowStart 当时
     已可见的具体信号并指明出处；② `what_went_wrong` 必须正面回答卡在哪一层并与权威层对照；
     ③ `fix_for_future` 必须是可执行判据修正，禁止"注意风险/谨慎操作"空话）。
   - 未命中：`（本次为常规到期反思，无漏检事件背景，忽略漏检专项三问，按常规复盘即可）`。
3. 收尾 `update_many`（L753）加 `.col_expr(stock_reflections::Column::MissLayer, ...)`；
   `source` 列建点已盖章，**不在 UPDATE 中改写**。
4. `status_text == "completed"` 且事件存在：回填事件
   `reflection_id=本行id`、`status='reflected'`，带 `status.ne("reflected")` 守卫
   （每事件有多档反思行，首个 completed 的档盖章，幂等不覆盖）。

文件：`src-tauri/agency_experts/stock-analysis/reflection.md`

5. 新增一节"漏检回放专项复盘"，引用 `{{mover_recall_context}}`，写明三问与层码对照
   （not_in_pool=入池判据遗漏 / l3_scored_out=L3 评分截断 / other_period=档位错配 /
   unexplained=留痕不足无法归因）；常规到期反思时该节输入为占位句，LLM 自然忽略。
6. prompt 版本 bump：实施时定位 mod.rs 中专家档案 UPSERT 的版本/哈希常量（mod.rs:78 注释
   "有则 update"的版本机制；参照 P1/P2 既有 bump 先例），让 `ensure_stock_analysis_experts_seeded`
   把新 .md 推到 DB。若 `seed_reflection_workflow_template`（mod.rs:1537）内联节点 prompt
   （mod.rs:1786 段）也被 reflection-agent 运行时消费，则同步加一行 `{{mover_recall_context}}`
   指引并 bump 模板版本；实施时先确认该字符串归属节点再动。
7. 参照校验脚本 `scripts/check-horizon-weight-parity.mjs` 是否对 reflection.md 有占位符断言，
   必要时同步（grep review_horizon 先例）。

### P3-4 教训入模前校验

文件：`src-tauri/crates/analysis-engine/src/reflection_lesson_validator.rs`

新增纯函数 + 单测：

```rust
/// 入模前校验。Ok(()) = 可写 active；Err(原因列表) = 只留档、不生效。
pub fn validate_lesson_for_admission(summary: &str) -> Result<(), Vec<String>>
```

确定性判据（不靠 LLM）：
1. trim 后字符数 10..=200（与 rhai R-303 截断上限对齐；旧 extract 的 250 上口径统一改 200）；
2. 行动语义：必须命中至少一个行动 token（应/必须/建议/避免/下次/提前/上调/下调/止损/
   加仓/减仓/阈值/确认/等待/剔除/纳入/不得/优先 等）；
3. 空洞拦截：整句仅由"不知道/无法判断/说不清/数据不足/注意风险/谨慎操作/随机/运气"等构成 ⇒ 拒；
4. 纯疑问 / 纯标点 ⇒ 拒。

文件：`src/commands/stock_workflow/reflection.rs`

- `extract_lesson_to_rule` 建行分支调校验：失败时仍 INSERT 留档，
  `status="validation_rejected"`、`confidence=0.0`、
  `rule_pattern=Some(serde_json::json!({"rejectedReasons":[...]}).to_string())`
  （rule_pattern 本为"触发条件"自由文本列，以此承载留档原因；实体注释无需改）；
  去重分支语义不变（已存在规则按原逻辑更新，不因新校验回退已生效规则）。
- 消费侧不改：decision.rs:3284 与 run_lesson_validation（reflection.rs:1564）都只认
  `status="active"`，validation_rejected 天然不注入/不参与统计。
- 不按 source 排除 mover_recall 教训——通过校验即有效业务教训（数据可达性归因分流是 P4-3）。
- 新单测 5-6 例：合格句、过短、纯疑问、空洞句、超长、含行动 token 的否定句。

### P3-5 前端：漏检行展开补分析报告 + 反思结论

后端新命令（加在 `src/commands/mover_recall.rs`，`#[tauri::command]` +
`#[agent_command(domain="finance", safety=Safe, call_mode=StateInput)]`，build.rs 自动注册）：

- `get_mover_recall_event_review(stock_code, anchor_date, period) -> Option<MoverRecallEventReview>`
  - 三键查 `mover_recall_events`（同三键多轮时取 `created_at_ms` 最新一条）；查无 → `Ok(None)`
    （实时面板事件可能从未落库，空态不是错误）；
  - 返回（`#[serde(rename_all="camelCase")]`）：eventId/runId/status/missLayer/windowStart/
    anchorDate/cumGainPct/analysisId/batchId/errorDetail；
  - analysisId 非空时附 `analysis`：stock_analyses model 整体 `serde_json::to_value`
    （不重建 dashboardReport，省 CPU；前端只取日期/状态/decisionJson/horizonDecisions）；
  - `reflections`：按 `original_analysis_id` 查 stock_reflections 全档行，投影字段对齐
    `list_reflections`（stock_analysis.rs:5534）并**新增** `source`/`missLayer`/`verdict`/
    `lessonSummary`/`rawReturn`/`moverLabel`，按 horizon 排序。
- 顺手给 `list_reflections`（stock_analysis.rs:5508）加可选参数
  `original_analysis_id: Option<String>`（camelCase 自动）+ 输出补 source/missLayer/verdict/
  lessonSummary，供复盘页同源过滤（增量向后兼容；面板本身走上面的一把梭命令）。

前端：

- `src/types/stock-analysis.ts`：加 `MoverRecallEventReview` /
  `MoverRecallReviewReflection`（camelCase，与后端 DTO 一一对应）。
- `src/components/stock-analysis/MoverRecallPanel.tsx`：
  - misses 表新增一列（展开按钮，antd 不用新依赖，用原生 ▸/▾ 字符 + Button type=link size=small），
    点击 toggle 该行展开态（key 用 stockCode+anchorDate+period）；
  - 展开区懒加载上述命令（一次/行，缓存到本地 state map），四种态各自成句：
    ① `null`=实时核查事件未入库（注明面板是实时视图、落库由周六核查）；
    ② excluded/pending/running=状态 Tag + 说明；③ failed=错误详情；
    ④ analyzed=补分析摘要 + 逐档反思卡（verdict/missLayer/三问文本/missedSignals 数组/
    lessonSummary/moverLabel；无反思行时提示等待反思任务）。
  - 浮层 ref 规则不涉及（非 Popover children）。
- `src/lib/browserMock.ts`：加 `get_mover_recall_event_review` mock（默认 null）；
  `list_reflections` mock 形状补新字段可选。
- 新组件测试：展开→null 空态、展开→analyzed 渲染三问（mock invoke 两次返回）。
- i18n 11 语言：新增 `stockAnalysis.moverRecall.review.*`
  （toggle/title/notTracked/statusPending/statusRunning/statusExcluded/statusFailed/
  noReflection/analysisHeading/reflectionHeading/verdict/missedSignals/fixForFuture/
  whatWentWrong/layerHint/errorDetail 等），用 Python 脚本注入，跑 locale parity 校验。

### P3-6 合并验证（全部改完后一次性跑）

- `cargo check`（src-tauri 下）
- 定点：`__TAURI_WORKSPACE__=true cargo clippy -p axagent -p axagent-entities -p axagent-dao -p axagent-analysis-engine --all-targets -- -D warnings`
- 定点测试：`cargo test -p axagent-analysis-engine reflection_lesson_validator`、
  主 crate `cargo test -p axagent --lib -- reflection mover_recall`
- `cargo fmt -p axagent -p axagent-entities -p axagent-dao -p axagent-analysis-engine`（+ --check 复核）
- 前端：`npm run typecheck`、`npx vitest run src/components/stock-analysis src/pages/TasksCenterPage`、
  `npx dprint fmt --incremental=false <本次改动文件>`、locale parity 脚本
- 全量三命令（fmt --all --check / clippy --workspace --all-features / test --workspace）
  仍留到 P1-P4 全部完成后一次。

## 验收对照（plan L243-244）

- [ ] 漏检反思行在 `stock_reflections` 按 `source='mover_recall'` 可查（miss_layer 同步盖章）；
- [ ] 反思文本含三问产出，missed_signals 为启动前可见信号、what_went_wrong 与权威层对照；
- [ ] validator 拒绝的教训可在 reflection_lessons 以 validation_rejected 留档但不注入决策；
- [ ] MoverRecallPanel 漏检行可展开看到补分析报告与逐档反思结论；
- [ ] 反思完成后 mover_recall_events 回填 reflected + reflection_id；
- [ ] 常规 due 反思行为与输出逐字不变（变量占位句兜底）。

## 不做（边界）

- 不新增 cron / 不改五条既有定时任务语义；
- 不把命中率统计页面大改（source 列可供 P3 统计侧/后续面板分桶，本阶段只保证口径可查，
  前端统计分桶可视化若 plan 未点名不扩）；
- 不做 P4 的数据可达性归因分流、进化审批、装饰器删除；
- 不碰 git 在途的无关 M 文件。
