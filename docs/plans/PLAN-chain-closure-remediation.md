# PLAN：股票业务链路闭环补强（2026-10-20）

来源：一次三链路闭环核查（智能荐股→反思 / 趋势智选→反思 / 荐股与智选→股票分析→反思），
核查结论与全部现网读数记在 `.workbuddy/memory/MEMORY.md` 指向的
`project-chain-closure-audit.md`。本文只写**要做的事、判据、代价**，不重述诊断过程。

## 0. 三条链的当前判定（作为本计划的基线）

| 链 | 判定 | 断在哪 |
|---|---|---|
| 智能荐股 → 反思 | 闭环 | 只断「文字教训不回注荐股打分」（`reflection_lessons` 在 `recommender/` 零读取）|
| 趋势智选 → 反思 | 半闭环 | 后验样本按标签域不进权重；无专责反思任务；报表无 UI 入口；内部因子权重是编译期常量 |
| 荐股/智选 → 分析 → 反思 | 主干闭环，末端断 | 进化学到的权重进不了个股共识（前端硬传 `null`）；结构提案无执行器 |

## 1. 现网读数基线（2026-10-20 取，正门 `node scripts/pg-connect.mjs sql`）

**动任何一条之前要重取一次本表** —— 这些数字直接决定代价，上一轮核查里就有两个旧记载被现网读数推翻。

| 读数 | 值 | 含义 |
|---|---|---|
| `workflow_templates['stock-analysis'].version` | 172 | 与代码同步（代码侧改动后须重取）|
| `reco_ic_gate` | `on` | 荐股数值闭环在现网**已转正**，非 shadow |
| `decision_validations` | 1630 行，全部实盘代 | 智选 354 / bottleneck 66 / capital 576 / value 234 / reversion 208 / trend 192 |
| 其中 serenity 的 `hit_outcome` | 全部超额标签（44/71/103/136）| 一条都不进胜负判定 ⇒ 不进权重闭环 |
| 其中 bottleneck 的 `hit_outcome` | 66 行全 `insufficient` | 智选系格从未有可判样本 |
| `strategy_weight_history` | 35 行，trigger 全 `reco-loop` | 覆盖 capital/reversion/trend/value；**serenity 与 watchlist 零行** |
| `reco_picks` 真实行 live | serenity 81 / bottleneck 11（停在 07-31）/ watchlist 0 | 后两个桶实际不出票 |
| `reflection_lessons` | 162 = 淘汰 125 / 生效 34 / 退役 3 | 教训线信号薄 |
| `lesson_applications` | 15 行，其中 10 行无后验回填 | 「教训被用了没有」当前无法回答 |
| `cron_jobs` | **15 行**，代码只播 11 | 多出 `batchref-due-daily`、`vldec-daily`（详见 A1）|
| `market_mainlines` | 0 行 | 但 `daily-market-events` 每天 18:00 completed（详见 A3）|
| 交易/组合五表 | 全 0 行；`price_alerts` 78 | 组合指标日更每轮如实输出「当前无持仓」⇒ 属上游无入口，非执行器坏 |
| `financial_snapshots` | 43696 行 | 每日估值归档有产物、下游无消费者 |

时间戳单位（避免再踩）：`workflow_executions.created_at` 是**秒**；`cron_job_history.*_at` 与
`cron_jobs.data->>'last_run_at'` 是**毫秒**；`reco_picks.created_at` 是字符串前 10 字符可截日期。

---

## 2. 阶段 A —— 数据卫生与判据诚实（纯代码 + 门，不需要裁定）

### A1　cron 任务齐备性门：检出库里多出来的 active 行

**现状**：库里 15 条任务，代码播种只认 11 个固定 id（`src-tauri/src/init/stock_automation_cron_seed.rs:49-58`）。
多出的两条来源是 2026-09-23 的手工插库脚本 `output/backup-20260923/add-cron.mjs:67,75`
（`INSERT ... WHERE NOT EXISTS` by id），在全仓 git 历史 `--all -S` 零命中。
播种语义是「存在即跳过，不改」（`src-tauri/src/init/stock_automation_cron_seed.rs:27`，保护用户手改的时刻），
`store.remove` 只出现在测试重置路径（同文件 `:81`）与用户显式删除命令 ——
**没有任何一层会回收库里多出来的 active 行，也没有任何一处报告它。**

**实测危害（不是理论）**：`reflection-due-daily` 与 `batchref-due-daily` 同 `task_type=batch-reflection`、
同 `0 18 * * *`，2026-10-19 两条分别在 18:00:34 与 18:01:13 各跑一趟 ⇒ 同一批到期 pending 被两趟消费，
配额 200 与默认 20 各算各的。

**修法（本阶段不删任何库行）**：
1. 新增门 `scripts/check-cron-job-authority.mjs`，判据三条：
   - R1 权威源 = `stock_automation_cron_seed.rs` 里全部 `*_JOB_ID` 常量（现场解析常量值，不许手抄清单）；
     断言常量条数下限 = 11，防止解析退化导致「白名单为空 ⇒ 恒绿」。
   - R2 现网 `cron_jobs` 全表（经 `scripts/pg-connect.mjs` 只读取）里，
     `task_type` 与内置 11 条重叠但 id 不在白名单的 **active** 行 ⇒ 逐条报出（同类型多条 active = 重复调度嫌疑）。
   - R3 `name` 等于 `id` 且 `task_type` 属股票域的行 ⇒ 报「非内置播种来源」（这条是本次查到的形态指纹，
     面板建的任务 `data.name` 就是 id，内置的是中文名）。
   - 门必须**有运行门**：拿现网真表跑一次，并对「把 `batchref-due-daily` 的 id 伪装成白名单内 id」
     造一个坏样本自证会红。库里那两条今天仍在 ⇒ **门今天应当是红的**，红才说明它有电；
     处置动作（删除）归用户在任务中心点（正门 `src/pages/TasksCenterPage/ScheduledJobsTab.tsx:214`）。
   - ⚠ 门要能区分「非交易日跳过」与「真执行」：`vldec-daily` 这类历史行 `success=1` 但输出是
     「非交易日，本轮跳过」，判重复时只看 schedule 与 task_type，不看 success。

**代价**：一个 node 脚本 + 注册进 `scripts/ci-check.mjs` 与 `.github/workflows/ci.yml` 两处同位；零 Rust 改动。

### A2　「部分完成」在历史里被存成失败

**现状（本轮先收窄过一次表述，勿再放大）**：面板**已经**能区分 ——
`src/pages/TasksCenterPage/ScheduledJobsTab.tsx:343-344` 按 `partial` 显示「部分完成」标签，
`src/types/scheduler.ts:61` 有该字段。缺陷在持久化层：
`partial` 只存在于 `cron_jobs.data` 的 `last_result`（`src-tauri/src/commands/scheduled_task.rs:108-109` 注释
自陈「旧记录缺省 false」），而 `cron_job_history` 表**没有 partial 列**（实测列只有
`id/task_id/started_at/completed_at/duration_ms/success/error/output`）。
写侧同时置两位（`src-tauri/src/init/services.rs:2791` `success: outcome.fail == 0 && !outcome.timed_out`，
`:2797` `partial: outcome.timed_out || outcome.truncated`）⇒
**墙钟到点的部分完成在历史表里永久等同失败**，只有「上一次结果」看得清。

**危害面**：任何按 `cron_job_history` 统计的成功率/复盘/门禁都会被压低。实测池扫描 7 轮里 3 轮
`success=0`，其中两轮输出写着「达到单轮墙钟预算提前收尾，属部分完成」；真故障是 10-20 那轮
（行情通道 `ERR_EMPTY_RESPONSE`，候选 20 只入队、分析成功 0）。**两者在库里不可分。**

**修法**：`cron_job_history` 实体加 `partial` 列（走既有声明式对账：实体列属性 +
`src-tauri/crates/dao/src/reconcile/extras.rs`，**不写 migration** —— 本仓 `MIGRATIONS` 已清空），
写侧落库带上，读侧 DTO 从历史行取真值而不是缺省 false。
⚠ 启动期 `additive_only` ⇒ 新列在存量库自动补，不碰既有行；报「已生效」要有现网列存在读数。

**判据**：一条 Rust 测试锁「`timed_out=true` 时 `partial` 必须为真且历史行落库后回读仍为真」；
配负控：拆掉 `services.rs:2797` 的 `timed_out` 项后该测试必须红。呈现层已合格，不动。

### A3　板块主线：把落库从「模型自愿调工具」改成确定性节点

**现状**：生产者齐全（`src-tauri/crates/analysis-engine/src/market_mainline.rs:335` 的
`batch_upsert_mainlines`、命令 `market_mainline_batch_upsert`、MCP 工具
`src-tauri/crates/tools/src/tools/market_mainline.rs:141`）；取数不落在被封的 push2 族；
**触发也真在跑** —— 模板 `daily-market-events`（库 v1）每天 18:00 执行，近 11 次 completed，
最近 2026-10-19，收尾节点输出里有完整 `mainlines` JSON。
断点是图里只有 `trigger → synthesize-mainlines(agent) → end` 三个节点，**没有持久化节点**，
落库全靠提示词要求模型自己调工具（`src-tauri/src/commands/stock_analysis_setup/seed_daily_market_events.rs:113`
注册工具、`:165` 提示词要求、`max_tool_rounds 8`）。
用「工具名在 agent 节点输出中的出现次数」取证：近 8 次全为 **0 次**，只有 09-06(35)/09-09(3)/10-10(1) 碰过。
⇒ 表 0 行。**这是结构性缺口，禁止用「往提示词里再加一句务必调用」来填。**

**修法**：
1. 在 `seed_daily_market_events.rs` 的图里，`synthesize-mainlines` 与 `end` 之间加一个
   `Tool` 节点（节点类型现成：`src-tauri/crates/harness/src/workflow_types.rs:714` 的 `ToolNode`、
   枚举变体 `:1415`），入参绑上游 agent 输出的 JSON，调 `market_mainline_batch_upsert`
   （`archive_missing=true` 沿用现提示词里的语义）。
2. `TEMPLATE_VERSION` 从 1 bump（同文件 `:32`）。⚠ 播种门是 `existing.version >= TEMPLATE_VERSION`
   先 return（`:243`）——**不 bump 就永不重播种**，这是辩论轮数那一轮踩过的同一个坑。
3. 加一条**落库层判据**：跑通一次该工作流（或喂一段已知 JSON 走同一 upsert 函数）后断
   `market_mainlines` 行数 > 0 且 `mainline_date` 等于批次日；负控拆掉 Tool 节点的 input 绑定，必须红。
4. 读侧本阶段**不做**（归 D6）。

**代价**：改一张种子图 + 版本号 + 一条落库判据；不动引擎。
风险：该模板的 agent 节点输出形态不稳（实测 10-10 输出是一整段 python 代码块）⇒
Tool 节点必须对「解析不出契约 JSON」显式失败并留痕，**不得静默产出 0 行**（缺席要让界面看得见，
不能又变成一张空表）。

---

## 3. 阶段 B —— 呈现层补口（智选后验看得见）

### B1　智选超额命中报表进界面

**现状**：07:00 决策回测里智选是独立一趟
（`src-tauri/src/commands/backtest_validation.rs:463-565` 的 `run_serenity_backtest_pass`，
报表结构 `SerenityHitRateReport`，判据 `src-tauri/crates/analysis-engine/src/serenity_hit_rate_backtest.rs`，
该模块已在编译清单 `src-tauri/crates/analysis-engine/src/lib.rs:93`、命令已注册
`src-tauri/src/register_commands.rs:122`），但**前端没有任何地方 invoke `run_decision_backtest`**（按两个可能的
名字 grep 全 `src/` 零命中），报表只进 tracing 日志。现网已有 354 行智选后验样本躺着没人看。

**修法**：在回测视图（`src/components/stock-analysis/BacktestView` 所在链）加一格，按「四态」显示：
未跑过 / 本轮样本不足 / 有命中率 / 通道失败，**禁止**把「库里没有智选行」与「智选不参与闭环」
混成同一个 0%（这是既有偏好：结构性缺口不得在 UI 造成歧义）。
i18n 须同批补满 11 语言并跑 `check-i18n-key-parity` + 占位符自检。

⚠ **文件重叠**：`src/components/stock-analysis/SerenityScreeningPanel.tsx` 与全部 11 份
`src/i18n/locales/*.json` 此刻正被并行批次改（工作区 14 个 M/?? 文件即该批）。
本条**不得落在这两个文件上**：承载点选回测视图一侧，locale 键的追加要等并行批落地后再做，
否则两边互相覆盖。

### B2　`refresh_serenity_feedback` 名实相符

**现状**：命令描述写「跟踪推荐表现、验证催化剂、调优权重」
（`src-tauri/src/commands/stock_workflow/serenity.rs:2025`），实现体 `do_feedback_loop`
（同文件 `:2040` 起，全文无 `ActiveModel`/`insert`/`update`）**全程零写库**，只返回 JSON 给面板
（面板调用点 `src/components/stock-analysis/SerenityScreeningPanel.tsx:1339`）⇒ 按下面板按钮算完即丢，
但文案让人以为调优已生效。

**修法（二选一，默认取①）**：
① 诚实化：命令描述与面板文案改成「按当前 live 报价查看候选浮盈」，不提「调优权重」。零风险。
② 真闭环：让它把结果落库并产出可回注的东西 —— 但那要连 D2 一起裁（智选要不要自己的权重线），
   本阶段不擅自扩到这里。

---

## 4. 阶段 C —— 接通「产出 → 被消费」（末端那一段）

### C1　进化权重进个股共识：把死参从后端装载，而不是等前端传

**现状**：证据权重请求体留了 `historical_weights` 入参
（`src-tauri/crates/analysis-engine/src/evidence_weight.rs:202`），Rust 侧除测试外**无人填值**；
生产唯一注入口是前端可选参数，而调用点 `src/stores/feature/stockAnalysisStore.ts:2961` **硬传 `null`**
（该函数全仓只有这一个生产调用方）。⇒ 进化学到的权重永远进不了个股共识。
注意荐股侧是通的：`reco_strategy_weights` 经 `load_reco_served_vars_db` 生效（闸 `on`）。

**修法**：在后端命令里从权威源装载（`strategy_weight_history` 当前生效行），
前端不再传该参 —— 这是「接通通路」而不是「换个硬编码值」。
**不新增任何可调参数**：不引入开关、不加阈值常量，装载失败即按「无历史权重」显式走现有兜底路径，
并在响应里带缺席声明，不得静默当空表。

**判据**：两层各一条 —— 纯函数层断「给定权重行 ⇒ 计算出的证据权重与之相关」；
生产层断「命令返回的报告里权重来源标签不是 `absent`」。两侧各配变异，红条数 = 被破坏那一格。

### C2　新增门：反思/进化产物必须有生产消费者

**现状**：没有任何脚本锁这一面。`scripts/check-orphan-modules.mjs:19` 自陈「门绿 ≠ 全仓无死码：
本门只判有没有被编译，不判编译了但没人调」⇒ C1 那种「函数在、参数恒 null」的形态全部查不到。

**修法**：新增 `scripts/check-reflection-output-consumers.mjs`，对一张**产物 → 消费点**清单逐条断言
「消费点在代码域里存在且传的不是字面量 null/空表」。清单初始就三条（不多不少，避免造第二个注册表）：
`historical_weights`、`reco_strategy_weights`、`{{stock_lessons}}`。
「是否被消费」只看代码域不看注释（既有纪律）；每条要写出**一行消费点的 `file:line`**，写不出来就是这条不成立。

---

## 5. 待裁定项（等点，不自行开工）

| 编号 | 事项 | 我的建议与理由 | 被推翻时改哪格 |
|---|---|---|---|
| D1 | 荐股要不要吃文字教训（`reflection_lessons` 目前在 `recommender/` 零读取）| **先不接**。现网教训 162 条仅 34 条生效，且应用记账 15 行里 10 行无后验回填 ⇒ 接通等于给一条薄信号开新通路。建议顺序：先补后验回填覆盖率，再谈接通 | 若点「接」，做法照分析否决表（`reco_analysis_vetoes`）形态加一条读取，不新建表 |
| D2 | 智选要不要一条自己的权重线（把超额标签折成档位调整）| 需要你判。样本已就位（354 行），代价低于上一轮估计；但它要新定一套「超额 → 权重」的判据，属新方法论，不是接线 | 若点「不做」，则 B1 的格子里必须写明「智选按设计不参与自学习」，不许留空白格 |
| D3 | `vldec-daily` 怎么处置 | 它是 06:00 扫全 pending、不带到期判据，实测每轮 `skippedYoung == totalPending`、`resolved 0` ⇒ 每天白跑一趟（10-16「幼龄不再盲反思」修复的必然结果）。建议删，或改成带 `dueOnly` | 若点「保留」，须在任务中心如实显示其 0 产出，不伪装成正常轮次 |
| D4 | 结构提案（L2/L3）执行器 | 判给里程碑。现在 `evolution_proposal.rs:10-11` 自陈只落 `pending_approval`、不执行任何变异，做执行器等于引入图级自动改写 | 不排期则保留现状 + 在审批抽屉文案写明「结构类提案仅留档」 |
| D5 | 持仓来源入口（交易/组合链上游）| 交易五表全 0，组合指标每轮如实报「无持仓」⇒ 这条链不是断在中段而是还没有入口。要做得先定「持仓从哪来」（手工录入 / 模拟盘 / 导入） | 属新产品决策，本计划不排 |
| D6 | 板块主线的读侧 | A3 只把「写得进去」修好；`market_mainline_*` 命令现前端零调用者、只读 MCP 工具委托的执行器注册已被移除。要不要 UI / 要不要进荐股打分，等 A3 有真实样本后判 | 若点「进打分」，要先查荐股侧是否已有板块因子，避免第五套参数 |

---

## 6. 门禁与验收口径

阶段内逐条收口用定点命令；**全部阶段完成后**才跑一次全量（三条与 CI 逐字一致）：

```
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
__TAURI_WORKSPACE__=true cargo test --workspace
```

- Rust 侧改动（A2/A3/C1）在 `cd src-tauri` 下跑定点 `cargo clippy -p <crate> --all-targets -- -D warnings`；
  从仓库根用 `--manifest-path` 会退回旧工具链产生伪红。
- 新增 node 门（A1/A2 呈现/C2）必须**既有编译门也有运行门**：真的跑一次拿到读数，并对每条判据造一个坏样本
  自证会红；两道新门都要在 `scripts/ci-check.mjs` 与 `.github/workflows/ci.yml` 两处同位注册。
- 前端（B1/B2/C1）跑 `npm run typecheck`、`npm run lint:eslint`（走 oxlint，不是 ESLint）、
  相关组件测试、`npm run format`；改到 locales 时逐语言回读并断言插值槽完整。
- 本仓工作树与并行会话共享：动手前重取一次 `git status`，与并行批文件真重叠则**只改不提交**。

## 7. 执行状态

- 阶段 A：A1 **已落地并实测**；A2 **代码落地**（待 Rust 编译与测试读数）；A3 **代码落地**（待编译读数）
  - A1 `scripts/check-cron-job-authority.mjs`：自检 8 例全过；连库首跑 **红 3 条**（R3 重复调度 1 条 +
    R4 孤儿行 2 条 = `batchref-due-daily`、`vldec-daily`），与手工核查逐条一致。
    设计即如此：**门今天应当红**，删除动作归用户在任务中心点。已注册 `ci-check.mjs` + `ci.yml` 两处同位
    （R2–R4 需本机 PG，CI 上取不到读数时明印「未检查」，不报绿）。
  - A2 落点：`entities/cron_job_history.rs`（加 `partial` 列，带 `default_value = 0`）、
    `harness/cron_persistence.rs`（`CronHistoryRecord.partial`）、
    `dao/repo/cron_job_persistence.rs`（写读两处 + 新增两条 `#[tokio::test]` 往返/负控）、
    `runtime-core/cron_job.rs`（`record_run` / `record_auxiliary_run` / `ExecutionRecord`）、
    前端 `types/scheduler.ts` + `ScheduledJobsTab.tsx` 历史列（复用既有 `tasksCenter.lastRun.partial` 键，
    **未新增 i18n**）。
    ⚠ 不需要手写 ALTER：`cron_job_history` 虽由 `ensure_tables` 运行时自举，但它的实体在
    `entity_modules!`（`crates/dao/build.rs` 从 `entities/src/lib.rs` 现场生成）里 ⇒
    `schema_diff::heal_all` 会按实体补列，双方言（`NOT NULL DEFAULT 0` 后缀由 `default_value` 推出）。
  - A3 落点：`seed_daily_market_events.rs`（`TEMPLATE_VERSION` 1→2、Agent 工具清单摘掉 batch_upsert、
    提示词改为「只输出契约 JSON，落库归下游节点」、新增 `const-mainline-archive` + `persist-mainlines`
    ToolNode 与四条边）；判据在 `seed_consistency_tests.rs` 新增两条测试（形状 + 七个破坏样本，
    含「绕过持久化节点」「入参路径写错」「版本没抬」各红自己那一格）。
    顺带修了 `stock_analysis_setup/mod.rs` 里那条会因本次改动失准的注释（它写着该工具在 `agent_tools` 内）。
    运行期前提已逐条读实现确认：`expression: "true"` 经 `dynamic_to_json` 的 `is::<bool>()` 分支
    得到 JSON `true`；`resolve_var_path` 会穿透 Agent 输出里 `content` 那段 JSON 字符串，
    故 `synthesize-mainlines.content.mainlines` 这一族路径成立；ToolNode 的权限闸来自 FSM
    （`fsm_executor::current_allowed_tools`）而非 agent 工具清单 ⇒ 从 `agent_tools` 摘掉该工具不会误伤新节点。
    首跑 2 红均判在我这边（假阳性 + 测试误用 `getBy`），修后待复跑读数。
- 阶段 B：B2 **已落地（范围收窄）** —— 现网面板只展示胜率/浮盈等只读统计，文案未说谎，
  误导只在 `refresh_serenity_feedback` 的命令描述上（写「调优权重」但实现零写库），故只改 Rust 侧描述，
  **不动** locales 与面板（避免与并行批抢文件）。
  B1 **已落地**：新只读命令 `serenity_validation_summary`
  （`src-tauri/src/commands/backtest_validation.rs`，只按已存 `hit_outcome` 标签聚合，不重新判定）+
  新面板 `src/components/stock-analysis/SerenityValidationPanel.tsx`（挂在 `BacktestPage` 的策略页签）+
  11 语言文案（顶层 `backtest.serenityValidation`，12 键，含 `{{error}}` 失败槽）+
  组件测试 6 例（三种缺席各占一句、失败不伪装成空、回放态真传 `generation`）——
  面板测试 6/6 绿、`BacktestPage.test.tsx` 4/4 未回归、oxlint 0。
  并在 C2 门里登记了这条新接线（M8 负控：面板拆掉调用即红）。
  详见 §8 偏差②（少一个不落库的数）。
- 阶段 C：C2 **已落地并实测**（自检 8 例全过；真实树绿 + 打印 1 条带日期豁免；已注册两处同位）。
  C1 **前提不成立，未实施** —— 见下方偏差①。
- 待裁定：D3 的处置动作（删库行）仍在用户侧；其余 D1/D2/D4/D5/D6 未拍。

## 8. 与本文的偏差

① **C1 不做**（不是没做完，是判据前提被证伪）。本文原写「由后端从 `strategy_weight_history` 装载」。
   读实现后确认键域不同：`evidence_weight` 的消费点按 **`analyst_id`** 查
   （`evidence_weight.rs:1011-1014` `hw.get(&analyst.analyst_id)`），
   而 `strategy_weight_history` / `weight_decay::compute_adjusted_weights` 的键域是
   **(style_id, period)**（`weight_decay.rs:58`）。两者不相交 ⇒ 照原方案接通只会让每条查找恒落 1.0，
   把「已接通」演给门禁看，比现在的 `null` 更坏。另普查过候选权威源：
   `analyst_feedbacks` 存的是节点质量反馈（`node_evolution.rs` 产出 prompt 纠正，无数值权重）、
   `backtest_feedback` 的参与记录由前端传入 ⇒ **本仓没有「按分析师的历史表现」权威源**。
   要真做，先得定义并实现那个源（含样本窗口与证伪口径），属新的一轮，不在本批擅自扩。
   已落 C2 的 R4 机制替这件事看守：该条目的豁免一旦被接通而未撤销，门即红。

② **B1 的产物比本文少一个数**：本文写「把报表接到界面」，实接的是**按已存标签计数的只读汇总**
   （行数 / 去重候选 / 已判定 / 跑赢 / 命中率 / 标签分布 / 最近验证日 / `inWeightLoop=false`）。
   **平均超额收益不落库**（只出现在回测当轮 tracing 里），所以新 DTO 里没有那个字段，
   面板用一句 `excessNote` 如实声明。要显示它得先给写侧加列，那属于本文之外的另一件事。
   新命令 `serenity_validation_summary` 刻意**只聚合已存 `hit_outcome`、不重新判定**——
   判定权属于 `serenity_hit_rate_backtest`，前端或命令里再判一次就是第二套判据。

③ 本文 §3 的 B1 注意事项（「承载点不得落在 `SerenityScreeningPanel.tsx`」）已兑现：
   新组件 `src/components/stock-analysis/SerenityValidationPanel.tsx` 挂在
   `src/components/stock-analysis/BacktestPage.tsx` 的「策略」页签内，未新增页签标签（免造新键）。

④ 本批自查出的四处自踩（都已修，记录以免下次重犯）：
   · A3 形状门首跑假阳性：用全文 `contains("name: \"x\"")` 判「幽灵工具」，而
     `tool_name: "x"` 的尾巴就含这个子串 ⇒ 把自己新加的持久化节点判成违规。改为按行首判。
   · A3 门的既有姊妹判据 `seed_tool_def_names` 把 `tool_name:` 归进 rhai 桶，
     我从 `agent_tools` 摘掉 ToolDef 会让既有可解析门**少一格覆盖面** ⇒ 不硬扩那张桶，
     改由新测试直接对 `tool_name` 查 `resolvable_tool_names(&[])`（registry ∪ MCP）。
   · B1 组件测试缺席断言误用 `getByTestId`（不存在即抛错）⇒ 改 `queryByTestId`。
   · B1 文案 `stateFetchFailed` 起初**没有 `{{error}}` 槽**，把失败原因丢了；补槽时按
     「写后逐语言回读并断言槽位成对」自检（缺译门与 parity 门都查不出半槽）。

## 9. 新增待裁定（本批执行中浮出，追加区）

- **D7**：`market_mainlines` 有了确定性写入口之后，读侧仍无人消费（`market_mainline_*` 命令零前端调用、
  `dojo_list_market_mainlines` 委托的执行器注册已移除）。要 UI 还是要进荐股打分，等首批真实样本落库后再判。
- **D8**：`cron_job_history` 的 `skipped`（非交易日跳过）与 `partial` 同构——也只活在
  `cron_jobs.data.last_result`，历史表同样没有该列。本批只补了 `partial`（因为它把「部分完成」
  误判成失败，会压低统计），`skipped` 只影响「跳过 vs 0ms」的显示，未擅自扩。要一起补请说一声。


