# PLAN-reco-asof-generation —— as-of 候选持久化与代际反思闭环

## 裁定（2026-10-12 → 10-13，用户下）

智能荐股与趋势智选在 as-of（回放）下取到的候选股票**必须落库持久化**，并基于这些
as-of 数据展开反思闭环。

这条裁定**退役** 2026-10-11 那轮的同名守卫（`PLAN` 见记忆 `project-serenity-asof-round`）：
那一轮为防止"历史口径的候选混进实盘历史"，让回放**跳过写库**。跳过 satisfies 不混码，
但不 satisfies 可反思 —— 回放结束后库里没有任何候选行，闭环取不到样本。

实现形态改为：**回放照落库，每行带代际标记**（`as_of_date`，`NULL` = 实盘）。
沿用仓内既有形态（`stock_analyses.as_of_date` / `stock_pipeline_runs.as_of_date`），
不新造概念。

刻意保留的一处跳过：`set_serenity_seed` / `set_serenity_candidate_cache` 是**进程内可变
状态**，没有代际可打，一旦被回放写过就分不回来（会污染下一次实盘建池）⇒ 仍只在 live 写。
这与"落库"不是同一件事，不构成对本裁定的违反。

## 代际列（三张表，全部 additive，走实体声明 + `extras.rs` 对账）

| 表 | 列 | 语义 |
|---|---|---|
| `reco_picks` | `as_of_date` | 该行出自实盘批次还是哪次回放的截止日 |
| `reco_scan_audit` | `as_of_date` | L3 截断留痕同代对齐（回放核查与 live 同口径） |
| `decision_validations` | `as_of_date` | 验证样本的**决策日**（回放不得用入库时刻当决策日） |

## 批次切分与状态

### 批 1 生产侧（本批完成）

- `recommend_stocks` / `recommend_stocks_all_periods`：去掉 `as_of_date.is_none()` 跳过，
  改为总是落库，代际取自 `RecoResponse.as_of_date`
- `persist_reco_picks` / `persist_reco_scan_audit`：加 `as_of_date` 实参
- 趋势智选 `run_serenity_screening_inner`：`pick.insert(db)` 不再被回放跳过，
  ActiveModel 写 `as_of_date: Set(as_of_ctx...)`
- 时间旅行批次链 A：批次直连引擎的 `run_recommendation_scan`，**原本不经命令层落库**，
  现补上落库 + 逐档 L3 留痕（带代际）；`RecommendationScan` 因此新增 `period_audit` 字段
- 铁律 B2 的文档与两处日志文案同步改口（原写"不落 reco_picks"，现在落了）
- `recommender/types.rs` 那条"as-of/replay 同样落库"的文档注释由本批实现兑现

### 批 2 消费侧代际过滤（本批完成）

只读实盘的点，全部显式加 `as_of_date IS NULL`：缓存回读、实盘池分析、趋势智选退出信号、
趋势智选反馈环、回测面板的"最近一次荐股"。

一处特殊：`fetch_reco_prior`（分析链拿智选先验做两链对照）按**同代际**取 —— 回放里的
分析拿回放的候选作先验，实盘拿实盘，否则对照跨代失真。

新增参数（默认口径不变，前端不传即维持实盘）：`list_reco_history` / `reco_ic_stats` /
`list_decision_validations` 的 `generation`。形状校验统一在 `parse_generation_param`，
错格式显式报错，不静默当"不过滤"。

### 批 3 闭环取样（本批完成）

- 回测取样不再排斥回放行；`fetch_and_validate` 的**决策日**改为：回放取 `as_of_date`，
  实盘取 `generated_at` 的日期部分。这是回放样本能否成立的关键 —— 用入库时刻当决策日，
  等于拿决策之后的行情验证一个过去的决策。
- 权重闭环（`reco_loop`）默认**只吃实盘样本**：读侧 SQL 过滤 + 纯函数层同一判据再挡一次
  （只有 SQL 过滤时单测查不到）。配负控测试 `replay_samples_do_not_enter_live_loop`。
  两套计数域不并成一个 IC 分母。

### 批 4 待做（需要拍板或实机）

1. 报表的分母已按代际切开（本批完成：`compute_validation_report` 一次只算一代，
   响应带 `generation` 与 `replay_sample_rows`，代际门里那条豁免已撤）。
   **剩的是 UI 入口**：`list_reco_history` / `reco_ic_stats` / `list_decision_validations` /
   `compute_validation_report` 的 `generation` 入参目前没有任何前端调用方传值。
   - 已真消费的代际路径：`fetch_reco_prior` 在函数内读 `current_as_of()` 自动同代取
     （回放批次里的逐只分析拿回放的智选候选作先验，实盘拿实盘），不需要开关。
   - 默认口径（不传 = 实盘）不变 ⇒ 现网行为正确、前端不破，但"按某次回放的批次查"
     在界面上点不到。补它需要一个代际切换控件（批次卡片或趋势智选历史面板）+ 11 语言文案
     ⇒ 等拍板：要不要做、做在哪一面。
   - 禁止的形态：控件加上却没有任何代码读它（同"面板可调是纸面的"那类缺陷）。
2. 全量门禁：分层下沉之后**此前的全量读数一律作废** —— clippy 全量与 `cargo test --workspace`
   要在下沉后重跑；`cargo test --workspace` 的另一半（bin + integration target）要等应用关闭
   才能重新链接（os error 5）。
3. 实机核对：`npm run tauri dev` 跑一轮回放，验证库里长出带 `as_of_date` 的
   `reco_picks` / `reco_scan_audit` 行，且趋势智选历史面板默认看不到它们。
   ⚠ 三张表的列走启动期声明式对账 ⇒ **不重启不生效**，重启前查库会误判成"没落库"。
4. 批次日期：现网 5 次回放批次候选全为 0（4 次被重启打断 + 1 次选了 2026-05-27，
   而热股榜快照最早只到 2026-09-30）⇒ 有数据的 as_date 是 {09-30, 10-09, 10-12}。
   日线表只有 3 个交易日，反思四档窗口（2/5/28/90）大多会判 pending，属数据累积问题。
5. 提交：与并行会话同树（49 个变更文件里本轮占 20 个），切分等裁定。已核
   `cargo fmt --all` 未改写他人文件（他人文件 mtime 均早于本次 fmt 时刻）。

6. 趋势智选的候选**落库了，但没有任何数值核算入口** —— 读数与三条出路见文末
   「另一条既有断裂」，本批已把该模块的失真自述改成实测事实。**需要拍板**：
   接进每日定时链 / 面板加触发按钮 / 判它按设计不做（要新增"超额收益 vs 行业均值"这个口径，
   不是补一条谓词就能闭环）。

### 本批顺带修掉的两处（与原清单同源，非扩范围）

- **决策回测定时任务从未被播种**：`cron_jobs` 里 8 个任务没有 `decision-backtest`，
  而它是 `decision_validations` 的唯一写入方 ⇒ 该表现网 0 行，荐股权重闭环零样本
  （与 as-of 无关，实盘也一样空转）。已按内置任务形态加进 `seed_stock_automation_crons`
  （07:00，`dry_run` 恒 false），种子测试断言从 4 改 5 并改名。
- **孤儿反思行永不自愈**：`stock_reflections` 有 6 条 pending，指向的 `stock_analyses`
  行已不存在，每轮被计入 `skippedYoung` 后状态仍是 pending ⇒ 连续数月每天 18:00 空转
  （读数：`processed=6 / skippedYoung=6 / resolved=0`）。
  终局化落在 `dao::repo::stock_lesson_queries::retire_orphan_pending_reflections`，
  由**两个**反思入口（`run_batch_reflection` 与 `run_batch_reflection_inner`）在扫 pending
  之前各调一次 —— 与 `reclaim_stale_running` 同一处置时点；循环里撞到只计数不写库，
  下一轮开头会被处置。状态串用既有 `failed: <码>: <人话>` 形态，不新增状态值域。
  读数上新增两个字段：`orphans`（本轮撞到）与 `orphansRetired`（本轮自愈掉）。

## 判据（两道门）

- 新门 `scripts/check-reco-generation.mjs`：R1 读侧必须有代际谓词、R2 写侧调用点近邻必须
  带代际、R3 闭环读侧必须**按代筛**（SQL 谓词或内存切分都算，只把字段拷进 DTO 不算）。
  8 条自证（含 6 组负控 + 空骨架），3 条显式豁免每次打印。
  已双处登记（`ci-check.mjs` + `.github/workflows/ci.yml`）。
- 旧门 `scripts/check-asof-node-scope.mjs` 的 R2 判据**改了方向**：`pick.insert(db)` 从
  "前窗口要有跳过守卫"改成"前窗口要有代际写入"；`set_serenity_seed(` 保持要 live 守卫。
  退役的 `ASOF_GUARDS` 常量随之删除。
- 同门新增 **R5**：批次进 guard 前必须校验该 as-of 日有建池快照
  （`require_pool_snapshot_coverage`）。下限 2 = 定义 + 调用点各一处 ⇒ 摘掉调用点只剩定义
  （死代码）会当场红；调用点写进注释也不算。负控由 4 组增至十组（含"代际写入后置"
  "写入挪远""拆掉种子守卫""只剩定义""调用点在注释里"）。
  这条校验把「该日根本不可回放」与「阈值太高没选出票」分成两句 —— 以前批次跑完只留
  一句「均无候选（可能当日数据缺失或阈值过高）」。

## 分层收口（`check-layer-discipline.mjs` 先红 5 处、后绿）

跑 Node 侧门禁时分层棘轮报我 5 处越界（命令层直连库、命令模块互调）。基线**只减不增**，
所以一处都没有靠调基线过关，全部按真下沉处理：

| 越界 | 处置 |
|---|---|
| `persist_reco_picks` / `persist_reco_scan_audit` 在命令层构造 `ActiveModel` | 实体与 INSERT 移入 `dao::repo::reco_pool`（`insert_picks` / `insert_scan_audit`，入参是纯数据 `NewRecoPick` / `NewScanAuditTrim` —— dao 不能收 `RecoPick`，那会与 analysis-engine 成环） |
| 批次与 cron 跨命令模块调这两个函数 | 新建 `commands/_shared_reco_pool.rs` 作三条入口的共用落库口（映射 + 代际透传，不含 `sea_orm::` 字面量），按 `_shared_state` 的先例登记进 `SHARED_INFRA_MODULES` 并写明理由 |
| 反思循环里两条孤儿处置的 `update_many` | 移入 `dao::repo::stock_lesson_queries::retire_orphan_pending_reflections`，两个反思入口各在扫 pending 前调一次 |
| 批次前置校验里的快照查询 | 移入 `dao::repo::astock_daily_snapshot`（`missing_snapshot_sources` / `earliest_snapshot_date`）；**要查哪两源**由 `recommender::pool::POOL_SNAPSHOT_METHODS` 单点导出，dao 与校验都不抄第二份清单 |
| `parse_generation_param` 被 4 个命令模块共用 | 移到 `axagent_astock_data::as_of`（代际语义本属 as-of 域），并在该 crate 补一条形状单测（留空=实盘 / 合法日期透传 / 错格式显式拒绝） |

净效果：`stock_analysis.rs` 少了一处直连库 ⇒ 门提示低于基线，已按它要求跑
`--update-baseline` 下调；分层门现绿。

## 下沉的连带面：主 crate 里那张「清单型门」过期了

`seed_consistency_tests::every_reco_picks_insert_stamps_algorithm_version` 按
`let files = [...]` **硬编码清单**扫 `reco_picks::ActiveModel` 建行点，断言计数 = 2 且每处都盖
`reco_version`。把建行点下沉进 `dao::repo::reco_pool` 后清单里的 `stock_analysis.rs` 不再有建行点
⇒ 计数掉到 1、门红（`预期扫到 2 处，实际 1 —— 有新增写入点未入册`）。

处置：**按门的语义登记新位置**（清单换成 `crates/dao/src/repo/reco_pool.rs` +
`src/commands/stock_workflow/serenity.rs`，计数仍为 2），**不**改成全仓扫描 ——
「移动写入点必须重新登记」是这道门刻意设的摩擦，改成扫描等于拆掉它。
（真要改成扫描另有坑：测试文件自己那行 `line.contains("reco_picks::ActiveModel {")` 含同一记号，
注释行与字符串字面量会被自匹配计入，而 Rust 侧没有现成的剥注释工具。）
动手前用**同一套切分算法**在 node 复算过一遍：`checked=2`、两处 `stamped=true`，
所以随后那 17 分钟的主包重编只是走形式。

## 实机核对清单（重启后逐项看）

1. 启动日志有 `[stock-automation-cron] 已创建内置任务 decision-backtest-daily`；
   三张表的 `as_of_date` 列也由启动期对账补上 —— **不重启查库会误判成"没落库"**。
2. 挑一个有快照的日期（2026-09-30 / 10-09 / 10-12）跑批次，日志应逐字出现
   `[time-travel-batch] <日期> 链 A 落库 reco_picks: N/M 行（as_of_date=<日期>）`
   和 `[time-travel-batch] <日期> 链B 趋势智选 K 只逐档候选（已落 reco_picks，代际 as_of_date=<日期>）`
   （这两条是源码里的 `tracing::info!` 原文，不是转述）；
   改挑 2026-05-27 应被当场拒绝并给出最早可回放日期。
3. 查库 `select as_of_date, count(*) from reco_picks group by 1` ⇒ 应出现非空那一档；
   趋势智选历史与荐股历史**默认看不到**这些行。
4. 决策回测跑一轮（07:00 定时或手动）：`select count(*) from decision_validations` 应 > 0，
   且新行的 `as_of_date` 与所属 pick 一致。
5. 反思：6 条孤儿行应变成 `failed: ORIGINAL_ANALYSIS_MISSING: …` 且不再被重扫，
   `reflection-due-daily` 输出里的 `orphans` 首轮应为 6、后续轮为 0。

## 门禁读数（截至本批最后一步）

| 门 | 读数 | 是否覆盖最终源码 |
|---|---|---|
| `cargo check -p axagent-analysis-engine --all-targets` | 绿，4m11s | ✅ |
| `cargo check -p axagent-astock-data / -p axagent-dao --all-targets` | astock-data RC=0（1m12s）；**dao 首红 1 处**：`.count(db)` 缺 `PaginatorTrait`（sea-orm 2.x 该方法是 trait 方法，`cargo check` 报「not an iterator」），另有 `EntityTrait` 未用告警 ⇒ 修后 RC=0（4m21s，含等他人构建锁） | ✅ 下沉后的新 dao 模块 |
| `cargo check -p axagent --lib` | 绿，21m50s（修掉 4 处 `Expr` 未导入）→ 复跑 19m25s → **下沉后复跑 8m36s 绿**（唯一告警 `reco_scan_audit` 未用 = 下沉残留，已删） | ✅ 覆盖下沉后源码 |
| `cargo fmt --all --check` | 0 diff | ✅ |
| `clippy --workspace --all-targets --all-features -- -D warnings` | **下沉后重跑 RC=0、零告警，18m54s**（其中约 25 分钟在等并行会话的构建锁） | ✅ 覆盖最终源码 |
| `cargo test --workspace --exclude axagent --no-fail-fast` | 下沉后重跑：**96 个测试二进制 / 5795 passed / 0 failed / 45 ignored**，1 个目标红（`-p axagent-agent --lib`）—— **零测试输出**、退出码 `0xc0000139 STATUS_ENTRYPOINT_NOT_FOUND`，即漏 `__TAURI_WORKSPACE__=true` 的已知本地形态（该 crate 本批零改动：`git status --porcelain -- src-tauri/crates/agent` 空） | ⚠ 属**我漏抄了 CI 口径的 env 前缀**（AGENTS.md「常用命令」明写）⇒ 带 `__TAURI_WORKSPACE__=true` 重跑：
  `-p axagent-agent --lib` = **1940 passed / 0 failed，RC=0**（同一棵树，证实是环境形态） | ✅ 已证 |
| `cargo test --workspace`（CI 口径全量，含 bin / integration 目标） | **仍没跑成**：应用运行中持有 `target/debug/axagent.exe`，重新链接被拒（os error 5）⇒ 要关应用补跑 | ⚠ 关应用后补 |
| `cargo test -p axagent --lib --no-fail-fast` | 带 `__TAURI_WORKSPACE__=true` 重跑：首跑 **595 passed / 1 FAILED** ⇒ 那条红是**真门生效**：`every_reco_picks_insert_stamps_algorithm_version` 报「预期 2 处建行点，实际 1」，因我把建行点下沉后它的硬编码清单过期。按门语义登记新位置（见「下沉的连带面」）后复跑 = **596 passed / 0 failed，RC=0**（92.41s） | ✅ 覆盖最终源码 |
| `cargo clippy -p axagent --all-targets --all-features -- -D warnings` | 登记修正后的定点复跑：**RC=0**，15m56s | ✅ 覆盖最终源码 |
| `check-single-source-facts.mjs --ci` | EXIT=0（硬判据 BROKEN/越界/空行 0；软判据 5 条落在他批文件） | ✅ 覆盖下沉后源码 |
| `check-layer-discipline.mjs` | 5 处棘轮违规全部靠下沉消除，基线**只下调过一次**，结论 ✅ 通过 | ✅ |
| `check-reco-generation.mjs` | 自测 8 条全过；现网 0 违规、命中 14/6/5、豁免 3 条 | ✅ |
| `check-asof-node-scope.mjs` | 自测十组负控全过；现网 R1=2 / R2=2 / R4=6/6 / R5=2 绿 | ✅ |
| `check-contracts.mjs` | PASS；`[H]` 静默丢弃棘轮 262 未上升（新写的终局落库用 `if let Err` + warn，不是 `let _ =`） | ✅ |
| `check-line-endings` | PASS（新脚本 LF 干净；`scripts/*.mjs` 不在 dprint 覆盖面内，与既有门脚本一致） | ✅ 本轮复跑 PASS: No violations |
| `npm run typecheck`（tsc --noEmit） | **0 条错误**（此前那 2 条「事件名不在联合类型」随他批 `events.ts` 重生成已消失） | ✅ |
| `npm run lint:eslint`（oxlint，CI 口径） | RC=0，1332 文件 / 96 规则 / 0 warning | ✅ |
| `npm run test:run`（vitest） | **2372 passed（213 个文件全过），RC=0**，100s | ✅ |
| `npx dprint check` | RC=20，**唯一一条红是他批未提交的 `debate-convergence.md` 表格对齐**（见「归属边界」） | ⚠ 不代他批 fmt |
| `check-generation-scope-registry.mjs` | EXIT=0，16 项登记一致；2 项 `reco_version` pending 是 #12 既有债（见「两个代际域」） | ✅ |

## 收尾时查出的另一条既有断裂（不属本批引入）

`crates/analysis-engine/src/serenity_hit_rate_backtest.rs` 有完整的智选胜率/T+N 命中实现，
但 `src-tauri/src` 里 **零调用点** ⇒ 「逻辑完整、没人触发」，形态同 2026-09-20 那轮的
`run_decision_backtest`（当时也是零调用方，靠补 cron 分支接上）。

后果与本裁定的交叉：趋势智选在回放口径下目前只有两条真实消费 ——
批次内合并去重后进逐只 replay 分析、以及先验层 `fetch_reco_prior` 按同代取；
数值表现核算（胜率线）压根没有入口，所以「给它的反馈环加 `generation` 参数」是假闭合
（加了也没人跑）。要做的是先定这条线接在哪个入口（面板手动触发 / 内置定时任务），
再谈代际口径。

**2026-10-13 补三个读数（把这条断裂的量级钉准）**：
1. 该模块**连编译都不参与** —— `analysis-engine/src/lib.rs` 只声明了 `hit_rate_backtest`，
   没声明 `serenity_hit_rate_backtest`；它文件头自称的接线 crate `crates/stock-analysis`
   在仓内已不存在（`git ls-files` 只剩 analysis-engine 这一份）。⇒ 它是死代码，不是"待接线"。
2. `run_decision_backtest_inner` 取样带 `Style.ne("serenity")`（2026-08-01 的 P1 修复，理由是
   serenity 的 `pick_data.price` 可能为 0、`seed_pool_json` 格式不同）⇒ 通用验证链**按设计**不收
   趋势智选样本，本批写入的回放代际 serenity 行同样落在过滤之外。
3. 趋势智选现役的兑现路径是 `refresh_serenity_exit_signals` + 反馈环（按当下 live 报价算浮盈），
   本批给它们加了 `AsOfDate.is_null()` 守卫 ⇒ 回放的 serenity 候选目前**两边都不进**：
   落了库、留了痕，但没有数值消费者。

## 注释面与裁定对齐（2026-10-13 收尾扫描）

裁定把「回放不落 `reco_picks`」推翻成「落库 + 打代际」后，按**旧裁定自己的关键词**
（`不落库|不落主表|铁律（B2|回放荐股`）全仓扫注释，浮出 3 处仍在替旧裁定说话，逐处按现实现改写：

| 位置 | 旧注释说的话 | 现实现 |
|---|---|---|
| `commands/stock_workflow/serenity.rs`（`SerenityScreeningArgs` 文档） | 「`as_of_ctx=Some` 尾部天然跳过 `reco_picks` 落库与全局种子同步」 | 落库**照写**并带代际；只有全局种子同步仍限 live（进程内单槽、无代际维度） |
| `commands/pool_scan.rs`（`run_analysis_for_candidates` 文档） | 把「不回读主表」的正当性挂在「回放荐股不落主表」铁律（B2）上 | 该函数不读主表是因为**候选队列由批次内存选择直接传入**（输入封闭），与落库与否无关 |
| `commands/time_travel_batch.rs`（`serenity_replay` 文档） | 「`as_of_ctx=Some` 自动跳过 reco_picks 落库……回放不污染 live 候选池（铁律 B2）」 | 同上：落库带代际，种子同步仍 live-only |

另：`recommender/types.rs` 那条「as-of/replay 同样落库，保证回放核查与 live 同一口径」由本批实现兑现
（L3 截断留痕 `reco_scan_audit` 现在也带代际），用户点名的这处不一致已闭合。

## 刻意延后的一项（不是遗漏）

三张表的 `as_of_date` **不建索引**。理由：`reconcile::expected::build` 从实体派生期望 schema，
新增可空文本列启动期会自动 `ADD COLUMN`（additive_only），无需 `extras.rs` 条目；而索引必须进
`extras.rs` 的 `INDEXES` 并同步 L2 冻结计数测试（`l2_item_counts_are_frozen`）。
现网读数：`decision_validations` 本批前 **0 行**、`reco_picks` 千级 ⇒ 顺序扫描零成本，
收益无法实测 ⇒ 不给它建无证据的索引。行数上万后再补，补的时候连带 bump 冻结计数。

## 「代际」在本仓有两个域，别混读

| 域 | 载体 | 谁锁 | 现网状态 |
|---|---|---|---|
| **算法起算代** | `reco_picks.reco_version`（整数，`RECO_GENERATION_FLOOR`） | 既有门 `check-generation-scope-registry.mjs`（16 项登记，presentation/single_lookup 反向断言「不得误筛」） | 仍有 2 项 pending 未筛：`reco_ic_stats`、`load_reco_loop_samples` |
| **实盘 / 回放批次** | 三张表的 `as_of_date`（`NULL`=实盘） | 本批新门 `check-reco-generation.mjs`（R1 读侧谓词 / R2 写侧标记 / R3 闭环分域） | 0 违规、3 条带理由豁免 |

⇒ 同一张 `decision_validations` 上两个域**同时存在且互相正交**：一条回放样本既有
`as_of_date=2026-09-30` 又有 `reco_version=N`。本批只动前者；后者那 2 项 pending 是
#12 的既有债，别把它当本批遗漏，也别顺手用本批的门去锁它（门的判据词不同，会互相当遮羞布）。

## 本批门禁读数的归属边界（避免被当成我的红）

- `npx dprint check` **RC=20，唯一一条红**在 `src-tauri/agency_experts/stock-analysis/debate-convergence.md`
  （markdown 表格列宽对齐）。该文件 `git status` 为 `M` = **他批未提交**（辩论轮数那一轮），
  本批从未触碰 ⇒ 不代它跑 `dprint fmt`（会把别人的草稿按我的口径重写）。
- `cargo test` 里两个目标红（`-p axagent-agent --lib`、`-p axagent --lib`）是**漏
  `__TAURI_WORKSPACE__=true` 的本地环境形态**（零测试输出 + `0xc0000139 STATUS_ENTRYPOINT_NOT_FOUND`），
  不是断言红；`src-tauri/crates/agent` 本批零改动。带该 env 前缀重跑见上表。
