# 时间旅行批次复盘（as-of 复用当日流水线）实施计划

## 一、调研结论

目标场景：选定一个历史交易日（如 90 个交易日前），在该日的 as-of 世界中重放当日四段流水线
（荐股 → 逐只分析），再回到 live 世界对这批分析按超短(2)/短(5)/中(28)/长(90)交易日四档做反思。

已确认可直接复用的基础：

1. **当日四段流水线已闭环**（`src/init/stock_automation_cron_seed.rs:63-127`）：
   00:00 荐股 → 01:00 趋势智选 → 02:00 候选池逐只分析 → 18:00 四周期到期反思。
2. **as-of 上下文机制完备**（`crates/astock-data/src/as_of.rs`）：
   - `with_optional_asof(ctx, fut)`（L399-409）RAII 压栈，task_local + 进程级作用域栈双层，
     **跨 `tokio::spawn` 可读**，嵌套/并发/异常展开均不泄漏（2026-09-23 已修复全局残留）。
   - 行情/vendor 调用在 as_of 下自动截断；缓存按 `asof-YYYYMMDD` 与 live 物理分桶。
   - recommender 结果缓存按 `(Period, as_of)` 隔离（`recommender/mod.rs:253-270`）。
3. **分析链原生支持 replay**：`stock_analyses` 有 `analysis_kind(live/replay)` + `as_of_date`
   列（`crates/entities/src/stock_analyses.rs:53-57`）；blackboard 自动写 replay meta。
4. **反思是显式参数驱动**：`run_reflection_workflow`（`commands/stock_workflow/reflection.rs:168`）
   吃 `as_of_date / hindsight_date / review_horizon / horizon_snapshots / reflection_id(幂等)`，
   本就用于「拿 hindsight 日行情复盘 as_of 日决策」。
5. **CronJob 持久化零迁移可加字段**：`cron_jobs` 表只有 `id` + `data`(jsonb) 两列，
   data 是整个 `CronJob` 的 serde 序列化（`dao/src/repo/cron_job_persistence.rs:91-103`）。
   加 `params` 字段只需结构体加列 + `#[serde(default)]`，无 migration。
6. 前端已有回放工作台 `src/components/time-travel/ReplayWorkbench.tsx`（独立页面 +
   `timeAnchorStore` 管理 asOf 日期），作为新入口宿主，不新建页面。

四个必须修的断点：

- **B1 时钟不读 as_of**：`pool_scan.rs:174,197` 硬编码 `chrono::Local::now()`（回看窗口、
  「当日已分析」判据），guard 内仍以真实今天为今天。
- **B2 reco_picks 写表无 live/replay 隔离**：`persist_reco_picks`（`stock_analysis.rs:4527`）
  无 mode 列；pool 全池模式与回测统计都全表扫，replay 荐股落表会污染 live 候选池与 IC 样本。
  **对策（MVP 零迁移）**：replay 批次荐股结果内存传递给分析阶段，不写 `reco_picks`；
  默认跳过趋势智选阶段（其工作流内部直接写主表）。
- **B3 反思不能关在 guard 内**：到期判据是 `hindsight_date <= today`，guard 内 today=as_of、
  分析刚产生 ⇒ 四档全部不到期。**对策**：阶段 1-3 在 guard 内，阶段 4 退出 guard 后执行；
  未到期档写 pending，由现有 18:00 任务未来自动补（零额外机制）。
- **B4 任务不携带业务参数**：cron 触发工作流恒用 `RunOptions::default()`（`services.rs:3033`）。
  **对策**：CronJob 加 `params`；本次先做命令式手动入口，定时化作为后续。

## 二、改动文件与模块

### 后端

- `crates/astock-data/src/calendar.rs`：新增 `nth_trading_day(date, n)`（循环调用
  `previous_trading_day/next_trading_day`，纯函数）与 `trading_days_between`。
- `crates/runtime-core/src/cron_job.rs`：`CronJob` 增加
  `#[serde(default)] pub params: Option<serde_json::Value>`；所有 `CronJob` 构造点
  （`From<CronJobData>` L649、测试辅助）补默认值。
- `src/commands/pool_scan.rs`：
  - `load_pool_selection` 的两个时钟改为 `as_of::current_date_or_now()`（B1）；
  - 抽出/新增支持「显式候选列表」的内部入口，供 replay 批次内存传 picks（B2）；
  - `run_pool_scan` 增加可选 `explicit_candidates: Option<Vec<PoolCandidate>>` 入参，
    有值时跳过 reco_picks 读表，live 路径行为不变。
- `src/commands/time_travel_batch.rs`（**新模块**）：批次编排，详见步骤 M2/M3。
- `src/commands/mod.rs`：`pub mod time_travel_batch;`（逐字模块声明，不用花括号 re-export，
  规避 build.rs 命令注册器限制）。
- `src/commands/error_code.rs` 或 `crates/harness/src/error_codes.rs`：新增错误码
  `BATCH_INVALID_PARAMS`、`BATCH_ASOF_NO_DATA`（日期非交易日 / 清单为空 / 行情不足）。
- `src/app_state.rs`：新增批次互斥锁
  `pub time_travel_batch_lock: Arc<tokio::sync::Mutex<()>>`（init/state.rs 初始化）。

### 前端

- `src/types/`：ScheduledTask/批次相关类型（若 barrel 已有 scheduledTask 类型则就地扩展
  `params?`）。
- `src/components/time-travel/ReplayWorkbench.tsx`：新增「历史批次复盘」卡片
  （日期模式：固定日期 / 相对 N 交易日前；阶段与档位勾选；发起按钮；进度与结果区）。
- 进度复用现有事件通道（批次内逐只分析本身会 emit stock t0 队列/rerun 事件）；
  批次级阶段进度用新事件 `time-travel-batch-progress`。
- `src/i18n/locales/*.json` × 11：新 UI 键 + `error.BATCH_*` 两键（zh-CN 为源）。

### 不改动

- 四个 live cron 及其种子；`reco_picks` 表结构（MVP 不加 mode 列）；
- 反思 pending 机制与 18:00 到期任务；趋势智选在 replay 批次中默认跳过。

## 三、实施步骤（依赖顺序）

### M1 基础设施：params + 交易日算术 + 骨架命令

1. calendar 新增 `nth_trading_day` / `trading_days_between` + 单测（周末/节假日/跨年长区间）。
2. CronJob 加 `params`（serde default），编译修复全部构造点；DTO/CreateTaskInput 透传 params。
3. 新建 `time_travel_batch.rs`：
   - 入参 `TimeTravelBatchRequest { as_of: 日期固定或{tradingDaysAgo}, periods: 四档勾选,
     min_confidence, top_n, run_reco: bool(默认true), run_serenity: bool(默认false),
     reflect: bool(默认true) }`；
   - 命令 `run_time_travel_batch(state, request) -> batch_id`；
   - 参数校验：as_of 必须是交易日（`is_trading_day`）、不晚于今天、不早于行情可得下限；
   - 获取 `time_travel_batch_lock`（拿不到 ⇒ 报「已有批次运行中」），随后 `tokio::spawn`
     后台执行，命令立即返回 batch_id。
4. 批次状态：MVP 用进程内 `Mutex<HashMap<batch_id, BatchRunState>>`（阶段/总数/完成/失败清单/
   四档结果摘要）+ 查询命令 `get_time_travel_batch(batch_id)`；**不建表**（M5 再持久化）。

### M2 阶段 1-3 在 as_of guard 内重放（B1/B2）

5. pool_scan as_of 化 + 显式候选入口（见上），live 路径回归测试。
6. 编排体：
   ```text
   batch_id 生成 → 状态 RUNNING
   with_optional_asof(Some(ctx=as_of), async {
     [阶段1] run_recommendation_scan(...) 得 picks: Vec<RecoPick>
            （不调 persist_reco_picks；内存持有）
     [阶段2] 默认跳过 serenity
     [阶段3] run_pool_scan(explicit_candidates = picks 映射的 PoolCandidate)
            → 逐只 run_single_stock_analysis（replay 上下文自动落 analysis_kind/as_of_date）
            收集本次新建的 stock_analyses id 列表
   })
   ```
7. guard 边界与 live 隔离：进入批次前检查当前北京时间不在 live 流水线窗口
   （23:50-02:10、17:50-18:10）则拒绝并提示；批次内所有 spawn 均在 guard future 内派生，
   靠 as_of 作用域栈继承；批次结束（含异常）guard drop 自动清理。

### M3 阶段 4 退出 guard 的四档反思（B3）

8. guard 外对 M2 得到的每条分析：
   - 四档目标 hindsight = `nth_trading_day(as_of, 2/5/28/90)`；
   - `hindsight <= 今天`：`compute_market_snapshots` 一次性拉四档窗口行情，
     调 `run_reflection_workflow(as_of, hindsight, review_horizon=该档, snapshots)` 立即复盘；
   - `hindsight > 今天`（未到期，多见于 mid/long）：写 pending 行（沿用现有入队函数），
     返回摘要中标记 `pending_horizons`，由 18:00 任务未来补；
   - 同股同档 reflection_id 幂等，重复跑不产生重复行。
9. 批次汇总：各档已反思数/未到期数/失败数，成功/失败股票清单，写状态 Completed/Partial。

### M4 前端入口 + i18n + 错误码

10. ReplayWorkbench 加卡片（日期模式、勾选、发起、轮询 `get_time_travel_batch` 展示
    阶段进度/逐只状态/四档摘要/失败清单/未到期提示）。
11. 11 语言 i18n；错误码翻译；DTO camelCase 对齐；`@/lib/invoke`。

### M5（后续，不在本次 MVP）

批次持久化表、断点续跑、重启恢复、定时化（带 params 的一次性/周期 CronJob 走同一编排体）。

## 四、依赖与注意事项

- 分层：编排放 wiring 层 `src/commands/`，可依赖任意 implementor；不得把新逻辑塞进
  runtime-core（consumer）。as_of/calendar 属 astock-data（implementor），M1 的
  `nth_trading_day` 放该 crate。
- CronJob::new 现有调用点多，加字段后用编译错误逐个补 `params: None`；
  `From<CronJobData>` 也要补。
- as_of 全局栈对 scope 外 spawn 可见 ⇒ 批次窗口互斥是硬约束（M2 第 7 点）。
- 荐股 `generated_at` 在 replay 中不写表，批次内候选 → 分析的关联改用「批次内内存列表」；
  stock_analyses 行靠 `as_of_date + analysis_kind=replay + created_at≈批次时刻` 可追溯，
  M5 再上正式 batch_id 列。
- 反思阶段**严禁**在 guard 内运行（拿不到未来 K 线）；这是本计划最易错点，code review 重点。
- build.rs 命令注册：新命令只需 `pub mod` + `#[tauri::command]`，勿手改生成文件。

## 五、验证

- `cargo test -p axagent-astock-data`（日历新函数）、
  `-p axagent-runtime-core`（CronJob params 序列化兼容：旧 JSON 缺 params 能反序列化）；
  pool_scan as_of 判定单测（构造 as_of 上下文验证「当日」= as_of 而非今天）。
- `cargo check` 阶段内增量；全部完成后
  `cargo fmt --all --check` +
  `cargo clippy --workspace --all-targets --all-features -- -D warnings` +
  `__TAURI_WORKSPACE__=true cargo test --workspace`（Windows 口径）。
- 前端：`npm run typecheck`、`npm run lint:eslint`、`npm run format`、ReplayWorkbench 测试更新。
- 手工冒烟（交易日数据）：选一个 mid 档已到期、long 档未到期的历史日期（如今日 2026-10-11
  往前约 60 交易日）：验证 ① reco_picks 无新增 live 污染行 ② stock_analyses 新增 replay 行
  且 as_of_date 正确 ③ 已到期档落 stock_reflections、long 档进 pending ④ live 荐股缓存不串档
  ⑤ 批次进行中再发起第二次被锁拒绝。

## 六、风险

| 风险 | 对策 |
|---|---|
| as_of 全局栈泄漏导致整进程锁进回放（族坑已发生两次） | 只用 `with_optional_asof` RAII；窗口互斥；冒烟后跑一次 live 荐股验证未串档 |
| replay 荐股误写 reco_picks 污染 IC 样本 | 内存传递，code review 核对编排体不出现 persist_reco_picks；冒烟 SQL 核验行数 |
| guard 内反思拿不到未来行情，四档全空 | 结构上反思调用在 guard 之外；单测/冒烟双重确认 |
| 长批次（数十只 × LLM）中途应用退出，状态丢失 | MVP 接受（状态标 Aborted 的策略：重启后检测不到即无残留，分析行靠幂等可重跑）；M5 持久化 |
| 历史日期行情/清单缺失 | 启动前预检（as_of 当日 K 线可得性），失败快速报错 BATCH_ASOF_NO_DATA，不跑半截 |
| 并发 live 流水线串扰 | 时间窗拒绝 + 批次锁；M5 再升级为与 cron 的统一调度锁 |
