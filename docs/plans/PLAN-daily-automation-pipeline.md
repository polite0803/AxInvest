# PLAN: 每日自动化闭环四任务（荐股 / 趋势智选 / 候选池分析 / 四周期到期反思）

> 状态：active ｜ 日期：2026-09-30 ｜ 对应版本：v2.9.9


## 用户诉求（原文口径）

1. 每个交易日 **00:00** 智能荐股 → 产生真实候选股票池
2. 每个交易日 **01:00** 趋势智选 → 产生候选股票池
3. 每个交易日 **02:00** 对候选池中**尚未分析**的股票依次分析；判据统一为「当日没分析过就分析」——第一次进池的必然分析；与以前重复的，只要**当日还没分析**也自然会分析（即没排到/失败的次日自然补上）；当日已分析过的不重复（2026-09-30 三轮澄清定稿）
4. 每个交易日 **18:00** 遍历历史分析记录，筛选「已超过超短/短/中/长各自持有期且未反思」的记录做对应反思

## 现状盘点（已调查实证）

| 环节 | 执行内核 | cron 分支 | CRUD 命令 | 缺什么 |
|---|---|---|---|---|
| 智能荐股 | `recommender::run_recommendation_scan`（四档共享池扫描→逐档落 `reco_picks`） | `services.rs:2240` `stock-recommendation` | `create_recommendation_cron` | 无自动建任务；不判交易日 |
| 趋势智选 | `serenity-screening` / `-fast` 工作流，产物落 `reco_picks`（style='serenity'，`serenity.rs:1318`） | `services.rs:2947` `workflow_id` 兜底分支 | `create_trend_screening_cron`（`recommendation_cron.rs:247`，默认 `0 16 * * 1-5`） | 无自动建任务 |
| 候选池逐只分析 | `run_pool_scan`（`pool_scan.rs:172`，读近 N 天 reco_picks 含 serenity，串行 `run_single_stock_analysis`） | `services.rs:2556` `pool-scan` | `create_pool_scan_cron`（默认 `0 17 * * *`） | 无自动建任务；不判交易日；**无「尚未分析」筛选**（按 confidence 截断 max_stocks=10，未分析的积压被静默挡掉） |
| 四周期到期反思 | `run_batch_reflection_inner` + `BatchReflectionConfig{period, dueOnly, maxCount}`（`reflection.rs:1648`，due_only=按 2/5/28/90 天间隔判到期） | `services.rs:2674` `batch-reflection` | 有 | 无自动建任务；另有 6h 兜底服务不限周期（保留不动） |

**关键前置缺陷（P0）**：`crates/runtime/src/cron/scheduler.rs:47` `should_run_now` 用 `chrono::Utc::now()` 匹配 cron 字段 ⇒ 写 `0 0 * * *` 实际在**北京时间 08:00** 跑。前端所有默认表达式（`0 16 * * 1-5`、`0 17 * * *`）都是按北京时间写的，即**现存定时任务全部晚 8 小时触发**。本项目仅 Windows 桌面单机、用户在东八区 ⇒ 修调度器为本地时区匹配是唯一能让「00:00 = 00:00」成立的口径（备选：种子表达式按 UTC-8 反写，但会和前端展示/用户手写语义永久分裂，否决）。

## 方案

### A. 调度器时区收口
- `scheduler.rs`：`should_run_now` 改按 `chrono::Local` 匹配（机器本地时区；东八区机器即北京时间）。
- 补单测：同一表达式在 Local 与 UTC 下命中小时不同，锁死语义。
- ⚠ 影响面：所有既有 CronJob 的触发点整体前移 8h 对齐到用户书写意图——这是纠偏，不是回归。

### B. 交易日闸门
- 在四个 executor 分支（`stock-recommendation` / `workflow_id`（仅 trend-screening 标记的）/ `pool-scan` / `batch-reflection`）入口加 `astock_data::calendar::is_trading_day(本地今日)`，非交易日记 info 日志后 skip（不产生 reco_picks 空批次、不落库为失败）。
- 18:00 反思分支同样加闸门（休市日无新行情，skip 无害；到期记录次日交易日会补上，pending 队列不丢）。
- 闸门放 executor 分支而非 scheduler 层：scheduler 是通用设施，不该懂 A 股日历。
- 日历兜底：`calendar.rs` 硬编码表已含 2026 全年（含中秋）；远程拉取失败不阻塞。

### C. 任务种子化（幂等，照 `seed_opc_cron.rs` 的按固定 ID upsert 模式）
新增 `src-tauri/src/commands/seed_stock_automation_cron.rs`，在 `init/state.rs` 现有 OPC 种子化旁调用。固定 ID、**存在即跳过不改**（用户手动改过周期/阈值后重启不得被回滚）：

| job id | task_type | 表达式 | 配置（prompt JSON） |
|---|---|---|---|
| `stock-reco-daily` | `stock-recommendation` | `0 0 * * *` | `{periods:[ultra_short,short,mid,long], minConfidence:50, topN:5}` |
| `trend-screening-daily` | `trend-screening` + `workflow_id=serenity-screening` | `0 1 * * *` | 空（executor workflow_id 兜底分支） |
| `pool-scan-daily` | `pool-scan` | `0 2 * * *` | 见 C2（全池 + 当日未分析判据），非现默认 |
| `reflection-due-daily` | `batch-reflection` | `0 18 * * *` | `{period:null, dueOnly:true, maxCount:200}` —— 一条任务覆盖四档。✅ 已核验并修复配额消耗缺陷：`run_batch_reflection_inner` 的 `take(max_count)` 发生在 filter **之前**，未到期的老 pending 会耗光配额、饿死当日到期记录 ⇒ 已把到期筛选（`hindsight_date <= 今日`，东八区口径同 `matches()`）下推到扫 pending 的 SQL。注意循环内 `ReflectionFilter::matches` **仍**判 `due_only`（reflection.rs:1693），SQL 下推是**补位**（保配额）而非替代闸口，两处口径一致 |

- 「产生真实候选股票池」的落点：四档扫描本身建 seed 池（热股榜∪行业龙头）→ 逐档 picks 落 `reco_picks`，即候选池持久化形态；`synthetic` 标记 + `SeedPoolOrigin` 诚实化机制已有，不动。
- 00:00/01:00/02:00 顺序依赖：串行于同一晚的三个时点天然满足；若荐股/趋势智选超时未落库，当晚池缺这批，但「尚未分析」判据保证**不丢**——缺的候选次日仍在队列（见 C2 边界声明）。

### C2. pool-scan 筛选内核改造：「尚未分析」判据（三轮澄清定稿）

`load_pool_candidates`（`pool_scan.rs:84`）改为四步筛，cron 与手动入口共用同一函数（保持既有「两入口口径一致」约定）：

1. **池 = 全部 `reco_picks`**（`synthetic=0`、`pick_data` 非空、confidence ≥ `minConfidence`），**去掉 generated_at 下界**——「没排到的次日自然补上」不靠窗口顺延，而由「尚未分析」判据天然覆盖：任何一天进池、至今没分析过的股票，每晚都在队列里。
2. **排除「当日已分析」**：对第 1 层结果逐 code 查 `stock_analyses`，**当日**已有 live 分析记录的跳过；历史某天分析过但当日没分析的**照常分析**（= 第一次进池必分析、重复出现的按当日判）。
   - ⚠ **判据字段选型（实施前核验项）**：`stock_analyses.analysis_date` 在批量路径写的是 **UTC 日**（`core.rs:1858`），与本地日的边界在 08:00——02:00（本地）跑的任务其 UTC 日还是「昨天」，直接用 `analysis_date = 今天` 判会错配；须核验表内是否有毫秒时间戳列可作「>= 本地今日 00:00」比较，没有则以 `analysis_date ∈ {UTC今日, UTC昨日}` 两值并集判据，并补「本地 02:00 写入 ⇒ 同轮再查判为已分析」的单测锁住边界。
   - 排除 replay / ab_test 记录（`analysis_kind != 'live'`），历史回放的分析不算「当日已分析」。
3. **池内去重**：同一 code 多条 pick（四档各一排、荐股与趋势智选撞车）只分析一次，取 confidence 最高的一条带 `period` 入队（沿用现有 seen-set 逻辑）。
4. **依次全量，不截断**：`max_stocks=0` 语义改为「不限」（现 `create_pool_scan_cron` 拒绝 0，放开）；种子任务 `minConfidence:50`，其余按第 1 层全池。现默认 10 会把「排在 10 名之后、尚未分析」的候选静默挡掉，与诉求直接冲突。
5. **失败可见性**：`PoolScanOutcome` 增加 `skipped_analyzed_today` 计数与逐只 skip 明细；空池、全跳过、截断三种缺席各报各的，不混成一个读数。

**边界声明（全池口径的三个已知后果，均已实证，须让用户知情）**：

- `reco_picks` **无任何自动清理**（全仓唯一删除入口 = 前端手动 `batch_delete_reco_history`，`reco_history.rs:244`）⇒ 表只增不减，「全池」= 全部历史积压。**上线首晚**会把库里所有从没分析过的历史候选一次性排入（按 confidence 降序逐只跑），首轮耗时长、消耗模型配额——这是诉求「第一次出现就必然分析」的直接推论，不是缺陷；接受则无需另加窗口，想限速可临时用 `maxStocks>0`。
- `minConfidence` 仍是**池成员资格**判据：历史低置信候选（< 阈值）永远不进队，即「未分析」不等于「必分析」，过滤是设计而非疏漏（种子任务取 50，与荐股档出厂先验一致；现默认 60 会挡掉大量超短档 pick）。
- 失败重试：某只本轮 `run_single_stock_analysis` 失败 ⇒ 当日无分析记录 ⇒ 次晚自动再进队，**无限重试**直到成功或该 pick 被手动删出池；无「永久失败」标记（如需要另说，本计划不做）。

### D. 前端呈现
- 四条种子任务在现有定时任务列表（`ScheduledAnalysisTab.tsx`）按 task_type 自动可见，名称带「（内置）」后缀；**不新建页面、不加新 invoke**。
- 不做「一键关闭全部内置任务」之类超范围功能；用户要停用可在现有列表逐条暂停。

### E. 验证门禁（全阶段完成后一次）
- `cargo fmt --all --check`
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`
- `__TAURI_WORKSPACE__=true cargo test --workspace`
- 定点自证：种子化幂等测试（跑两次不重复建）；`should_run_now` 时区单测；交易日闸门的节假日/周末/工作日三例；C2 的「当日已分析跳过 / 历史分析过但当日未分析照常 / 池内重复去重取最高档」三判据单测 + UTC 日边界单测。
- 实机：`npm run tauri dev` 挂起过零点观察四任务按序触发（或临时把表达式改成近时点自证后改回）。

## 阶段切分（逐阶段等「开工」指令）

- **阶段 1**：A（调度器时区）+ B（交易日闸门）——地基纠偏
- **阶段 2**：C2（pool-scan「尚未分析」判据改造，含 analysis_date 时区核验项）+ C（种子化四任务）+ maxCount 核验/逐档回退方案
- **阶段 3**：D 核对前端列表可读性 + E 全量门禁 + 实机核对

## 边界声明（不做 / 已知限制）
- 应用未运行时不会跑（autostart 插件已开机拉起，关机期间任务顺延至下次启动后由调度器按当前分钟判，不补跑历史时点）。
- 不新建表、不动 schema；`stock-recommendation` 分支现不调 `recommend_stocks_multi` 的 preseed 管道，与「历史推荐滚动构建池」是另一条已规划路径，不在本计划。
- 命中率回填（decision-backtest，T+5/20/60）与次日 T+1 回填是既有/另一议题能力，本计划四任务不含，如要并入请单说。
- 时区修正只覆盖「CronJob 轮询体系」；rt-workflow 的 TriggerManager 自带 cron crate（支持 timezone 字段），本次不动。

## 实施状态（2026-09-30 落地）

**已改文件**：
- `crates/runtime/src/cron/scheduler.rs` — 时区改 Local + 同分钟去重（`minute_key` 守卫）+ 2 条新单测
- `src/init/services.rs` — executor 入口新增交易日闸门（6 个 task_type，skip 也 `record_run` 显示「非交易日，跳过」）
- `src/commands/pool_scan.rs` — 新增 `PoolSelection` / 纯函数 `select_candidates` / `load_pool_selection`，全池 + 当日未分析 + 不截断 + skip 明细；默认改 `{0,0,50}`；`maxStocks=0`/`lookbackDays=0` 放开；5 条新单测
- `src/commands/stock_workflow/reflection.rs` — 到期筛选下推到扫 pending 的 SQL（修 `take(max_count)` 在 filter 之前消耗配额的缺陷）
- `src/commands/seed_stock_automation_cron.rs`（新）+ `commands/mod.rs` 声明 + `init/state.rs` 启动调用；3 条单测（建 4 条且幂等 / 不回滚用户改动 / 配置能被 executor 解析器反序列化）

**新代码的顺序约束（不是既存缺陷，是本轮引入 skip 后才出现的坑）**：旧实现没有「已分析」概念，边遍历边按 `max_stocks` 截断无害；一旦加入 skip，**排除必须发生在截断之前**，否则配额被当日已分析的标的消耗、未分析积压反而进不了队列。本轮把逻辑抽成纯函数 `select_candidates` 后由单测锁住该顺序。

**内存优化**：全池查询改为 `select_only` 只取 4 列，不再把每行的 `pick_data` / `seed_pool_json` 大 JSON 载入内存。

**未做（超出计划）**：未给历史低置信候选加「永久失败」标记；未动 6h 兜底反思服务；未加 OS 级独立定时。

**门禁状态（2026-09-30 分段汇报）**：
- ✅ 已实证绿：`cargo fmt --all --check`；行尾门禁（25571 文本文件无 CRLF）；runtime `clippy -p axagent-runtime --all-targets -- -D warnings` EXIT=0；runtime 调度器单测 14/14（含机器无关时区锁、同分钟去重键两条新用例）；**主 crate 全量 lib 测试 505 passed / 0 failed**（本轮新增 10 条逐条确认执行：pool_scan 7 + 种子化 3）。
- ⚠ 全量 `-D warnings` 当前 3 条红，逐条比对 HEAD 后**均非本轮文件**：`analyst_feedback.rs:13`（该行 HEAD 里已存在）、`stock_analysis.rs:4478`、`stock_analysis.rs:4705`（不在 HEAD ⇒ 并行会话 WIP）。
- ⏳ 未跑：`cargo clippy --workspace` / `cargo test --workspace` 兜底。理由（非偷懒）：clippy 遇首个 error 即短路，本树必在 `axagent` crate 因上述他人 lint 停下，而本轮改动只落在 `runtime`（已定点 `-D warnings` 绿）与 `axagent`（已定点 + 全量 lib 测试绿），其余 crate 未被触及 ⇒ 现在跑全量的增量信息为零。待并行会话那两条清掉后再跑一次即可。
- ⏳ 未做：`npm run tauri dev` 实机零点核对。种子化只在**启动时**写库，故下次启动后应能在定时任务列表看到四条「（内置）」任务（智能荐股 00:00 / 趋势智选 01:00 / 候选池分析 02:00 / 到期反思 18:00）。

**工作树归属声明**：本轮 5 个改动文件（`scheduler.rs`、`services.rs`、`pool_scan.rs`、`reflection.rs`、`seed_stock_automation_cron.rs` + `mod.rs`/`state.rs` 接线）与并行会话的未提交 WIP 同树；中途一度看到的 `hooks.rs:34`、`reflection.rs:1858` 两条告警在其后续编辑中自行消失，**未代改任何他人 hunk** ⇒ 按共享树规矩**只改不提交**，待那批落地后统一切分。

**自写单测抓出的两处本轮自伤（已修，留作实证记录）**：
1. 种子化把固定 id 传给了 `CronJob::new` 的首参——该参数是 **name**，`id` 由 uuid 现生成 ⇒ 幂等完全失效、每次启动长出重复任务。跑两次不重复建的用例当场红。修法：显式赋 `job.id`（同 `seed_opc_cron.rs`）。
2. 同分钟去重键原本只细到**小时** ⇒ `*/15 * * * *` 这类分钟步进任务会被从每小时 4 次压成 1 次。修法：键细到分钟，并把该用例改成不依赖真实时钟的构造（同一分钟起点 +59s 必同键、+60s 必变键）。
