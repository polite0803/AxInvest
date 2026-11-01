# PLAN — 反思与优化闭环六项收口（2026-10-14）

范围来源：本轮「全面分析股票业务反思/优化缺陷」评估后用户按编号点完的六条拍板。
**每条动手前必须回读本节原文**（勿凭记忆另造判据）。

现网读数基线（2026-10-14 取，`node scripts/pg-connect.mjs sql`）：
`stock_reflections` completed=16 / 带 horizon=13；`strategy_performance` `strategy_id='reflection_verdict'`=8 行；
按 `(stock_code, created_at[, horizon])` 可配对的反思行 = **0/8**；
`parameter_suggestions_json` 非空 = 9 行，形态为 `[{"param","current_value","suggested_value","reason"}]`；
`decision_validations`=600 行 / `factor_snapshot` 非空=0；`reflection_lessons`=7（`success_count>0`=0）；
`lesson_applications`=1；`strategy_weight_history` trigger 全为 `reco-loop`（13 行）；
`reco_ic_gate`=shadow、`cost_pct`=0.003、`stop_vol_mult`=1.2、`take_profit_vol_mult`=2、`horizon_prior_kappa`=20、`risk_budget_pct`=1.5；
`workflow_templates`：stock-analysis=156、serenity-screening=60、四档子模板=156。

不在本轮范围（用户未点，勿顺手改）：前端 `apply_param_suggestions` 不读 skipped（缺陷 4）、
`cost_pct` 档内硬编码 0.0015（缺陷 5）、`factor_snapshot` TODO（缺陷 7）、
dao `list_validated_since_date` 漏 as_of 谓词（缺陷 9）、荐股链 √h 与分析链退役口径分裂（缺陷 11）、
`build_hitrate_stats` 全表读（缺陷 12）、子模板不写版本快照与 `workflow_snapshots` 死表（缺陷 13）、
注释「28 项」漂移（缺陷 14）。

---

## 一、参数建议解析统一到单一权威源（缺陷 1+2）

**规格**
1. 权威形态 = 生产形态：条目键为 `param`（参数名）+ `suggested_value`（数值）；兼容既有
   `key`/`value` 与顶层对象写法，但**不得**再出现「认不出就返回默认值」。
2. 名字权威源 = 单一别名表。把 `PARAM_ALIASES` + `to_snake_case` + 名字归一逻辑从
   `src/commands/stock_analysis.rs`（wiring）下移到 `analysis-engine`（implementor），
   wiring 侧改为 `pub use`/调用同一函数，不保留第二份。
3. `try_parse_param_suggestion` 返回 `Option<PortfolioMgrParamSet>` 语义收窄为：
   **至少识别到一个已知参数名才 Some**；识别不到 ⇒ None，且不再被 `unwrap_or(&default)` 洗成默认值。
   `score_param_set` 的 `suggested` 缺省分支保留（那是历史行真无建议的情形），但调用侧要能区分。
4. `calibrate_portfolio_mgr_params` 返回体加诚实计数：`totalReflectionsWithSuggestions` /
   `parsedSuggestions`（真解析出的条数）。`parsedSuggestions=0` 时走「无可解析建议」文案分支。
5. Gap3（`reflection.rs:1050-1108`）方向比较改为：只对**解析成功**的行参与计数；
   注释里「自动触发 WFO」的措辞按事实改（当前实现只记日志，无 cron 接它 ⇒ 本轮不改行为，
   只把文案改成如实描述，触发通路的补齐属缺陷 2 的裁决范围之外，见文末）。

**门禁**
- 引擎单测：用**现网真实串**（`[{"param": "action_watch_threshold", "reason": "...", "current_value": 0.38, "suggested_value": 0.42}]`）
  断言 `watch_threshold == 0.42`，且其余字段仍取默认（证「识别成功」不是靠全量兜底）。
- 负控两条，各自只让对应断言红：① 条目形态对但名字不在别名表 ⇒ `None`；
  ② 数组为空/纯 `[]` ⇒ `None`（不得返回默认值集合）。
- 形态同源自证门（wiring 层，读源码文本）：反思 prompt 里声明的 `params_suggestion` 条目键名
  （`param`/`suggested_value`）必须同时出现在解析器的读取键清单里 —— 防再次长出第二套拼写。
  放 `src/commands/stock_analysis_setup/seed_consistency_tests.rs`（既有读源码门的前例）。

---

## 二、逐档命中率关联键改显式外键（缺陷 6）

**规格**
1. `strategy_performance` 新增列 `reflection_id: Option<String>`（`crates/entities/src/strategy_performance.rs`）。
   语义：**Gap1 旧格行**指向产出它的那条反思行；`#31` 四档通道继续用 `source_ref`（决策×档），两列不得混义。
2. 写侧：`reflection.rs` Gap1 插入处（现 960-989）设 `reflection_id: Set(Some(analysis_id.clone()))`。
   `analysis_id` 在 `run_reflection_workflow` 内即反思行 id（pending UPDATE 路径为建点 id，
   INSERT 路径为新建 id），已在同作用域可见（Gap3 用 `r.id == analysis_id` 自证）。
3. 读侧：`crates/analysis-engine/src/reflection_stats.rs::build_hitrate_stats`
   改为 ① 优先按 `reflection_id` 精确配对（新建 `ref_by_id` HashMap）；
   ② `reflection_id` 为 NULL 的存量行退回 `(stock_code, created_at, horizon)` 旧键，
   并把退回条数 `tracing::warn!` 出来（不静默）。旧键索引仅在仍有 NULL 行时才需要构建 —— 保持简单：两份索引都建，不做条件跳过（避免第三态）。
4. 列不走 migration（本仓正门 = 实体列属性 + `crates/dao/src/reconcile/extras.rs` 声明式对账）；
   启动期 `additive_only` ⇒ 新列为 ADD COLUMN，存量行 NULL 与第 3 步的退回分支一致。

**门禁**
- 引擎单测（新）：造两行 sp（一行带 `reflection_id`、一行 NULL）+ 两行反思（`created_at` **刻意不同**），
  断言带外键的那行走路径 1（拿到 `horizon_results_json` 展开），NULL 那行走 legacy；
  负控：把外键指向不存在的反思行 ⇒ 落 legacy 而不是 panic/错配。
- `cargo check -p axagent --all-targets` + 定点 clippy（entities/dao/analysis-engine 三 crate）。

---

## 三、教训退役环补 cron（缺陷 3）

**规格**
1. `run_lesson_validation` 增加 `pub const LESSON_VALIDATION_TASK_TYPE`（放在 `reflection.rs`，
   与 `DECISION_BACKTEST_TASK_TYPE` 同形态），cron 分支与播种共用该常量。
2. 种子：`src/init/stock_automation_cron_seed.rs` 增第 8 条内置任务 —— 工作日 21:00（在 18:00 到期反思、
   20:00 进化聚合之后），`LESSON_JOB_ID = "lesson-validation-daily"`，prompt 留空（无参数）；
   文件头表格加一行；「存在即跳过」语义不动。
3. 分派：`src/init/services.rs` 增 `task_type == LESSON_VALIDATION_TASK_TYPE` 分支，调
   `run_lesson_validation(db)`，日志与同类分支同形态（完成/失败两条，勿吞错）。
4. 撤掉 `reflection.rs:1747` 那句**不成立**的注释（"已接入 start_lesson_validation 定时任务"），
   改为指向本 PLAN 落地的真实任务类型。
5. 齐备性门（新，wiring 层读源码）：`build_builtin_jobs()` 产出的每个 `task_type` 常量都必须在
   `src/init/services.rs` 源码里出现为分派判据 ⇒ 防「播种了但没人执行」复发（本轮根因即此）。

**门禁**：种子既有测试 `seed_creates_seven_jobs…` 改名 + 断言条数 8；新任务配置的往返断言补一条；
齐备性门必须自证会红（把某个 task_type 从 services 源码判据里剔除的合成文本要命中）。

---

## 五、荐股 IC 同代筛样（缺陷 8）

**规格**
1. `RecoIcRow` 增 `algorithm_version: Option<i32>`（`crates/analysis-engine/src/recommender/ic.rs`）。
2. 样本装配处（`reco_loop.rs::samples_from_validation_rows` / `to_ic_row`、
   `stock_analysis.rs:4763` 的面板装配）带上来源代际：`decision_validations` 本身无版本列 ⇒
   取来源 pick 的 `reco_picks.reco_version`（已有列，见实体 45 行）作为归属。
   验证行没有 pick 关联列可用时按现有关联路径核实后再定，**不得**用入库时刻代替代际。
3. 聚合处筛样：`aggregate_reco_ic` 只吃 `algorithm_version >= RECO_GENERATION_FLOOR` 的样本；
   NULL = 归属未知 ⇒ 不进分母，并按既有诚实形态报告「样本代际未知 N 条」。
4. `seed_consistency_tests.rs:325-360` 那条依赖锁随之转义：`ic.rs` 出现 `algorithm_version` 后
   锁自动满足 ⇒ 该锁的注释与负控按新语义重写（负控仍拆在真正决定该断言的守卫上）。

**门禁**：IC 单测加「跨代样本不得混入同一格」断言（造两代样本，断言只计当代 + 缺席原因如实）。

---

## 六、定时反思参数可改可回读（缺陷 15）

**规格**
1. 后端新增 `update_validate_decisions_cron(jobId, cronExpression, minConfidenceThreshold, reflectionDepth)`
   （与 `create_/toggle_/delete_validate_decisions_cron` 同模块同 DTO 形态），返回更新后的 job；
   命令只做两步（`#[tauri::command]` + 所属 mod 已声明），注册由 build.rs 生成，勿手改 `register_commands.rs`。
2. 前端 `ReflectionPanel.tsx`：三个控件初值改为**回读** `activeCron` 的实际配置（prompt JSON + schedule），
   回读不到时显示「未配置」而不是造一个像真值的默认数；已有 cron 时编辑 → 调 update 命令；
   失败必须显式报错（不得 `.catch` 吞）。
3. i18n：新增 UI 文案一次性补齐 11 种语言（zh-CN 为源），键放 `stockAnalysis.reflection.*`。

**门禁**：`tsc --noEmit`、`check-i18n-key-parity.mjs`、oxlint、dprint；组件测试锁「回读到旧配置 ⇒ 控件显示该配置」
与「无 cron ⇒ 显示未配置」两种缺席形态。

---

## 四、智选胜率线恢复编译并接消费者（缺陷 10）

**规格**
1. `crates/analysis-engine/src/lib.rs` 补 `pub mod serenity_hit_rate_backtest;`，修正并入后可能已漂移的引用，
   并删除文件头 13-15 的「未参与编译」告警注释。
2. 孤儿模块门（新）：全 workspace 扫 `crates/*/src/**/*.rs`，凡未被同 crate 任何 `mod` 声明可达者即红。
   这是本条的**根因门** —— 文件可以整份存在、自述待裁决，而编译与门禁都不 complaint。
   豁免表显式列出确有理由的例外（`schema-gen` 等非 lib crate 布局者按需），豁免须带日期与理由。
3. 消费者：接进 07:00 决策回测任务（`src/commands/backtest_validation.rs`），
   撤销 172 行对 `style='serenity'` 的无条件排除，改为 serenity 行走本模块判据
   （超额收益 vs 行业均值），非 serenity 行保持原判据。
   **写侧词表边界必须先核实**：`hit_outcome_to_binary_outcome`（`hit_rate_backtest.rs:411-417`）
   只认 `{hit,partial,miss,false_hit}`，serenity 的四档标签若直接落 `hit_outcome` 列，
   下游 `sync_outcomes_to_stock_analyses` 与荐股闭环 `samples_from_validation_rows` 的处置必须逐条点名并跑测试
   （禁区：从严/扩样改动要同批点名现役消费者）。⇒ 实施时先做这一步的引用面普查再落笔。
4. 行业均值取数正门先普查（既有 `industry_chain` / astock-data 通道），不新建重复通道。

**门禁**：孤儿模块门自证（临时把一个已声明模块摘掉 ⇒ 必须红）；
serenity 胜率有真实数值消费者（命令返回体 + 面板一处显示或报表一处聚合），不得只算不落。

---

## 提交与验证纪律

- 与并行会话文件重叠情况：工作树仅 `crates/harness/src/session_log_invariant.rs` 为他人未提交改动 ⇒ 不纳入提交。
- 逐阶段只跑 `cargo check -p <crate> --all-targets` + 定点 clippy；**六项全部落地后**跑一次 CI 口径三条全量。
- JS 侧：`npm run format`、`tsc --noEmit`、`check-i18n-key-parity.mjs`、`check:horizon-parity`、
  `check-reco-generation.mjs`、`check-single-source-facts.mjs`。
- 报「完成」前分三段报：代码 / 门禁读数 / 提交状态；实机核对（`tauri dev`、重启重跑）归用户。

---

## 落地状态（2026-10-15 凌晨）

| 项 | 代码落点 | 已取读数 | 待取 |
|---|---|---|---|
| 一 参数建议解析单一权威源 | engine `portfolio_formula`（别名表 + 生产形态 + `parse_param_suggestion_entries`）；wiring 只留调用方；Gap3 改逐参数方向；calibrate 诚实计数 | 6 条引擎单测（含两负控）+ 形态同源门（含两负控）已写；`dprint`/`tsc`/`oxlint` 绿 | `cargo check` 两侧、引擎测试跑通、clippy |
| 二 命中率显式外键 | `strategy_performance.reflection_id` + Gap1 盖章 + `match_sp_rows_to_reflections` | 4 条 join 单测（外键命中 / 存量回退计数 / 悬空外键 / 档区分）已写 | 同上 + 启动期 extras 对账真 ADD COLUMN（实机） |
| 三 教训退役 cron | 第 8 条内置任务 + services 分派 + `LESSON_VALIDATION_TASK_TYPE` + 撤掉「已接入 start_lesson_validation」假注释 | 齐备性门 + 种子 8 条断言已写 | 种子测试跑通（需 cargo） |
| 五 荐股 IC 代际筛 | `RecoIcRow.algorithm_version` + 下限筛 + 两类排除分列；面板与闭环共用 `load_pick_versions`；登记表新增 statistics 条目 | `check-reco-generation` 绿（新豁免已打印）、登记表 17 项一致、i18n 两套门绿 | cargo 侧 |
| 六 cron 可改可回读 | `update_validate_decisions_cron` + `CronJobResponse.prompt` 回传 + 面板回读三态 + 保存按钮 | **4 条组件测试实跑全绿**；11 语言 5 键 parity 绿、untranslated 绿、key-parity 15870 键一致 | cargo 侧（命令注册由 build.rs 生成，须确认无 missing-command 类报错） |
| 四 智选胜率线 | lib.rs 收编 + 独立一趟（不进通用命中率报表）+ `not_persisted` 桶 + 报表进任务中心摘要；孤儿模块新门已接 `ci-check.mjs` / `ci.yml` / `package.json` | 新门首跑 121→11→0（两轮修解析器假阳后自证仍会上报 serenity 类案例）；模块 13 条单测（含两负控）已写 | cargo 侧 |

**被什么挡住**：无（构建锁曾被子会话占用约 1 小时，其间只跑不依赖 cargo 的门）。

**已取到的 Rust 读数（2026-10-15）**：`cargo check -p axagent --all-targets` → **EXIT=0**；
`cargo test -p axagent-analysis-engine --lib` → **657 passed / 0 failed**（含本轮新增的 10 条：
参数解析 5 条、命中率 join 4 条、智选 2 条负控桶）；`cargo check -p axagent-analysis-engine --all-targets` → 0 error。
在跑：`cargo fmt --all --check` + 定点 clippy（engine/entities + 主 crate）+ 主 crate `--lib` 测试（`/tmp/round3.log`）。

**未提交**：共享树纪律 —— 子会话正在写 `alert_mapping.rs` / `monitor.rs` / `evidence_weight.rs` /
`calendar.rs` / `stock_workflow/core.rs` / `monitor_emitter.rs` / `PriceAlertPanel.tsx` /
`types/stock-analysis.ts` / `session_log_invariant.rs` / `check-contracts.mjs` / `check-panel-var-landing.mjs`
并长出 `check-monitor-event-authority.mjs` 等新文件；`src/register_commands.rs` 是构建生成物，
两侧命令混在同一份里 ⇒ 提交切分待那批落地后统一做。

**实施期对 PLAN 原规格的两处偏离**（都因现网读数）：
1. §四.3 原写「serenity 行走本模块判据（超额收益 vs 行业均值）」——实测 `reco_picks.pick_data`
   里没有 `serenity_score` / `bottleneck_composite` / `data_reliability` 三键（75 行全缺），
   故报表把未持久化维度单列 `not_persisted` 桶、因子 IC 只吃有评分的行；基准取不到的行**不判**
   （旧 `unwrap_or(0.0)` 会把绝对收益冒充超额，已改成 `?` 并配负控）。
2. §四.3 原写「撤销对 style='serenity' 的排除」——实际保持通用查询的排除不动，
   智选走**独立一趟**：它的标签域与通用命中率报表的分母不兼容，混进去会把智选显示成「命中率 0%」。

## 三轮门禁读数（2026-10-15）

`cargo fmt --all --check` → **0**；`cargo clippy -p axagent-analysis-engine -p axagent-entities --all-targets -- -D warnings` → **0**；
`cargo clippy -p axagent --all-targets -- -D warnings` → **101**（1 条，本轮我自己写的）；
`cargo test -p axagent --lib` → **616 passed / 3 failed**。三条红的归属与修法：

| 红 | 归属 | 根因 | 修法 |
|---|---|---|---|
| clippy `neg_cmp_op_on_partial_ord`（智选趟里的入场价合法性判断） | 本轮智选那一趟 | 取反比较作用于 `f64`；本意是「NaN 也算非法」 | 改 `!price.is_finite() || price <= 0.0`，注释保留「不是收益 0」的语义 |
| 新门 `suggestion_entry_key_shape_is_identical_across_prompt_and_parser` | 本轮 §一 的门 | 读 md 用了 `../agency_experts/...`，而 `CARGO_MANIFEST_DIR` 已是 `src-tauri` ⇒ 路径不存在 | 去掉 `../`（同文件既有门即用 `join("agency_experts")` 的写法） |
| 新门 `every_builtin_task_type_is_handled_by_the_executor` | 本轮 §三 的门 | 扫描面含本测试自身源码：`".with_task_type("` 字面量 + 文档注释各命中一次 ⇒ 8 条报成 10 条 | seed 侧扫描面先剥注释行、再截到 `#[cfg(test)]` 之前（「判据读的必须是生产代码」本身入注释） |
| 既有测试 `lessons_are_isolated_per_generation` | 上一批（依据过滤，已随 `b4aa1b355` 入库） | 注入侧按「被复盘的分析有没有落黑板快照」筛除，而该夹具只插反思行、不插分析行 ⇒ 新代那条被当成「依据不可得」筛掉，按代际隔离的断言根本走不到 | 夹具补两条带 `blackboard_snapshot` 的 `stock_analyses` 行（`a-r-new` / `a-r-old`），并把这条前置写进注释 |

> 三条新门在写的时候都过了「会红」的自证，但**「门自身跑不起来」与「门恒真」是两类故障**：
> 前两条属于前者（路径不存在 / 自指数），只有跑一次 `--lib` 才暴露。⇒ 新门的读数必须打在真测试上，
> 不能只靠 `cargo check`。

**取读数的口径**：T2 那一轮我在编译中途改过 `reflection.rs` ⇒ 该轮单条读数只当定位用，
最终以一轮无改动的全量 `cargo test -p axagent --lib` 为准。

## 终局门禁读数（2026-10-15）

CI 口径三条按实机状态取到：

| 门 | 读数 |
|---|---|
| `cargo fmt --all --check` | **0** |
| `cargo clippy --workspace --all-targets --all-features -- -D warnings` | **0**（整仓，含子会话同期改动的 `alert_mapping.rs` / `monitor.rs` / `evidence_weight.rs` / `calendar.rs` / `core.rs` / `session_log_invariant.rs`） |
| `cargo test --workspace --lib --no-fail-fast` | **0** —— 42 个套件、**7923 passed / 0 failed**；分项：主 crate lib **619/0**、`analysis-engine` **657/0**、`dao` **339/0**、`entities` 1/0 |
| 节点侧七道门 | 全 0：`check-orphan-modules` / `check-reco-generation` / `check-generation-scope-registry` / `check-i18n-key-parity` / `check-single-source-facts --ci` / `check-contracts` / `check-horizon-weight-parity` |

> ⚠ **口径差**：`cargo test --workspace`（含 bin 与 `tests/` 集成目标）**没取到** ——
> `target/debug/axagent.exe` 正被运行中的应用占用（`os error 5 拒绝访问`），cargo 无法替换该二进制。
> 处置是**不动那个进程**，改用 `--lib` 口径并把这一格如实标为「未跑」，
> 而不是让「全量」这个词盖过实际覆盖面（禁区：门的覆盖面本身是门禁的一部分）。

**本轮新增/修复的测试在 619 条里的位置**：`suggestion_entry_key_shape_is_identical_across_prompt_and_parser`
（§一 形态同源）、`every_builtin_task_type_is_handled_by_the_executor`（§三 齐备性）、
`lessons_are_isolated_per_generation`（上一批依据筛除留下的夹具缺口，已补两条带快照的分析行）。

**前端四道（同一轮，CPU 空出后跑）**：`tsc --noEmit` **0**、`dprint check` **0**、`oxlint` **0**、
`vitest run` **2403 passed / 219 files**（含本轮 4 条 cron 回读组件测试）。
