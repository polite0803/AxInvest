# PLAN — 三个慢命令异步化（面板不冻等 + 报告落库可回取 + 可取消）

> 状态：active ｜ 日期：2026-10-23 ｜ 对应版本：v2.9.9
> 「批准，开工」。
> 前置：任务中心接入批（run_tracked 内嵌台账 + 第 6 源 backgroundTasks adapter）**在共享树上未提交**，
> 本计划直接在其上演化。两批提交切分见「提交切分」节。

## 0. 背景与根因

三个慢命令已接任务中心纳管（运行可见），但仍是**同步 IPC**：前端 `invoke` 会 await 到
命令返回完整报告——38s（sweep）/ 99-130s（backfill）/ 19-26s（backtest）期间：

1. 面板按钮冻等（用户不知道还剩多久、不能切走）；
2. 前端重载 / 应用重启后报告丢失（结果只在内存里走一趟 IPC，面板刷新即没）；
3. 无法取消。

已核实的事实：

| 事实 | 位置 |
|---|---|
| `run_tracked` 内联形态（同步命令内建行→catch_unwind→终态） | `src-tauri/src/commands/background_tasks.rs:156` 起 |
| 读侧命令 `get_background_task_output(task_id) -> BackgroundTaskInfo` 已存在 | 同文件 ：618 |
| `stop_background_task` 已存在（状态迁移层面） | 同文件 ：640 |
| 事件常量 `IpcEventName::BackgroundTaskCreated/Updated` 已在发射、载荷=裸 id | 同文件 ：571/594 |
| TaskPanel 已有「list + listen created/updated + 5s 轮询兜底」完整样板 | `src/components/chat/TaskPanel.tsx:124-224` |
| `background_tasks` 表**无 result 列**；output 列语义=上一轮裁定的「摘要」 | `crates/entities/src/background_tasks.rs` |
| `RecoStrategyMatrix` 传 `{ group }` 参数**被命令忽略**（签名只有 as_of_date，Tauri 反序列化丢弃未知键）；切正负组时组件在 ：137 本地取 `data.positive/negative` —— **切组是本地重排，不该触发新任务** | `src/components/stock-analysis/RecoStrategyMatrix.tsx:64,137` |
| MoverRecallPanel 的 sweep/backfill 走 `runSampleAction` 包装：assertReport 断言 + failed>0 throw + 成功返回 i18n 插值消息——**这些断言时点要整体后移到「取回落库报告」处** | `src/components/stock-analysis/MoverRecallPanel.tsx:128,139` |
| backtest 有 3 个面板同步消费类型化 `BacktestComparisonResponse` | BacktestPanel / RecommendationPanel / RecoStrategyMatrix |
| 浏览器 mock 与 4 个测试文件 mock `backtest_reco_strategies` 同步返回报告 | `src/lib/browserMock.ts:7009` 等 |

## 1. 修法（架构一句话）

**`run_tracked` 从「内联 await 完再返回」改为「登记后 tokio::spawn，立即返回 task_id」；
完整报告序列化落 `background_tasks.result_json` 新列；前端共用一个 hook
（发任务→订阅既有事件+轮询兜底→终态后取 result_json），5 个消费点改接线。**

产出层（报告落库、任务中心可见）与呈现层（面板渲染）分离：output 列保持摘要语义不动，
result_json 是给消费方的结构化报告，两者各司其职（对齐「结构性缺口不得在 UI 造成歧义」：
面板显示的是 running 行 + 取回的报告，不伪装成同步结果）。

## 2. 退役声明（点明退役什么）

| 退役项 | 替代 |
|---|---|
| `run_tracked` 内联 catch_unwind 形态（本批上一轮所写） | 拆为 `spawn_tracked`：建行/终态/序列化内部逻辑共用同一套函数，执行体从 inline await 改 tokio::spawn |
| 三命令同步返回报告的 IPC 契约（`Result<BacktestComparisonResponse/…Report, String>`） | 返回 `TaskAccepted { taskId }`；报告改走 result_json 读侧 |
| `RecoStrategyMatrix` 把 `{ group }` 传进 invoke（被命令忽略的死参数） | group 切换只做本地重排（`data.positive/negative`），不触发 invoke |
| run_tracked 三测的同步等待判据 | 判据保留（成功/业务失败/panic 三条），改异步等待终态后断言 |

## 3. 分阶段

### 阶段 A — 后端（result_json 列 + spawn_tracked + 三命令改签名）

1. **新列**：`background_tasks` 实体加 `pub result_json: Option<String>`（Text nullable）。
   走声明式对账正门（实体列属性 + extras.rs，additive_only 启动自动建列），**不写 migration**
   （`MIGRATIONS` 已是 `&[]`，与仓内既有事实一致）。
2. **DTO**：主 crate 的 `BackgroundTaskInfo` 加 `result_json` 字段（serde camelCase → 前端 `resultJson`，
   Option，adapter 无需改——新字段可选）。
3. **`spawn_tracked`**（替代 run_tracked 的对外形态）：
   - 签名：`spawn_tracked<T: Serialize + Send + 'static>(app: &AppHandle, db: &DatabaseConnection,
     title, task_type, description, work: impl Future<Output = Result<T, String>> + Send + 'static,
     summarize: FnOnce(&T) -> String) -> Result<String, String>`（返回 task_id）。
   - 流程：建行 pending + record_task_created → **返回前先 emit `BackgroundTaskCreated`**（面板立刻出行）
     → `tokio::spawn`（内：transition Running + emit Updated → catch_unwind 执行 work →
     成功时 `serde_json::to_string(&T)` 写 result_json + output 存 summarize；失败时 output 存原因；
     panic 落 failed 不留僵尸 → transition 终态 + exit_code → emit Updated）。
   - **summarize 跟随 work 移进 spawn**（它 FnOnce(&T)，与 T 同生命周期，move 进去即可）。
   - run_tracked 内联形态删除（退役表第 1 行）；被 spawn 形态的公共内部函数取代。
4. **三命令改签名**：
   - `backtest_reco_strategies` → `Result<TaskAccepted, String>`；**as-of 语义保持**：包裹体
     仍在 `with_optional_asof` 作用域内，spawn 的工作闭包捕获 ctx（task_local 传播已在仓内
     有先例形态，照 serenity 轮的 node_task_scope 处理——实施时核对 `with_optional_asof`
     的 future 是否 Send，若 ctx 依赖 task_local 且不可跨 spawn，则把 as-of 值作为参数显式
     传入工作闭包并在闭包内自建 task_local 作用域）。
   - `sweep_market_close` / `backfill_market_close_history` → 同样返回 `TaskAccepted`。
   - `TaskAccepted { task_id: String }` + `#[serde(rename_all = "camelCase")]`（禁区 13）。
5. **emit 载荷**：沿用既有裸 id 字符串形态（TaskPanel 已消费此形态，不另立）。
6. **测试**：
   - run_tracked 三测改写为 spawn_tracked 异步版（成功 / 业务失败 / panic；等待终态后断言
     result_json 内容 / output 摘要 / failed 终态 + panic 消息）。
   - 新增一条：**result_json 只在 completed 时非空、failed 时为 None**（防止把摘要当报告消费）。
   - 主 crate clippy 定点 `-p axagent --all-targets -- -D warnings` + entities 那边随主 crate 编译。

门禁：`cargo check -p axagent --all-targets` + 定点 clippy + `__TAURI_WORKSPACE__=true cargo test -p axagent --lib background_tasks`（改 Rust 触发 tauri dev 重启——动手前先确认没有在跑的分析，见记忆条目）。

### 阶段 B — 前端基座（类型 + hook + mock）

1. **类型**（`src/types/` 单一权威）：
   - `TaskAccepted { taskId: string }`；
   - `BackgroundTaskInfo` 补 `resultJson?: string`。
2. **共用 hook** `useTrackedTask`（`src/hooks/`）：
   - `start(invokeCall)` → 存 taskId、置 running；
   - 订阅 `background-task:updated`（+created 兜底）——收到本 taskId 且状态终态时
     `invoke<BackgroundTaskInfo>("get_background_task_output", { taskId })` 取行，从
     `resultJson` 反序列化出报告，置 result / error；
   - **轮询兜底**（事件丢失时 5s 轮询任务行，照 TaskPanel 形态）；
   - 组件卸载不取消任务（任务照跑、报告照落库），只是停止监听——重载后由
     「历史行仍在」兜底（这就是好处②的落点）。
   - hook 单测（vitest）：终态取报告 / failed 转 error / 事件丢失走轮询兜底 三条。
3. **browserMock**：`backtest_reco_strategies` / `sweep_market_close` / `backfill_market_close_history`
   三个 mock 改 task 形态（返回 TaskAccepted，`get_background_task_output` 对新 id 返回
   终态行含 resultJson）——浏览器模式全绿是本阶段门。

门禁：`npm run typecheck` + 定点 vitest（hook 测试）+ `npm run format`。

### 阶段 C — 5 个消费点改造 + 4 个测试文件

| 消费点 | 改造 |
|---|---|
| `RecoStrategyMatrix.tsx` | 挂载时发任务；**group 切换不再 invoke**（本地重排，退役表第 3 行）；externalData 分支保留 |
| `BacktestPanel.tsx:74` | 同步 report 改 hook 取回后渲染 |
| `RecommendationPanel.tsx:132` | 同上 |
| `MoverRecallPanel.tsx:128,139` | `runSampleAction` 包装改两段式：发任务（立即返回）+ 报告取回后做 assertReport / failed>0 throw / i18n 插值消息（断言时点后移，判据不减） |
| `src/lib/browserMock.ts:7009` | 随阶段 B |
| 4 个测试文件 mock | BacktestPanel.strategyToken / MoverRecallPanel.sampleActions / RecommendationPanel.horizonPresentation / ScreenerToWorkspace.integration——mock 改 task 形态（vi.mock 的 invoke 返回 TaskAccepted + 终态行） |

呈现层判据（照「门禁与退役批的判据写法」）：每个面板组件测试断言
①发起后按钮立即解冻（不 await 报告）；②报告取回后正文与改造前同一夹具等价；
③failed 时错误句上屏。破坏样本各配变异。

门禁：tsc + 定点 vitest（5 组件 + 4 测试文件）+ dprint。

### 阶段 D — 收口

- 全量门禁三件套（fmt --all --check / clippy --workspace --all-targets --all-features /
  `__TAURI_WORKSPACE__=true cargo test --workspace`）+ 前端 tsc / vitest 全量 / oxlint。
- PLAN 本节写「执行状态」块。
- **提交切分**（共享树）：
  - 批 1（先行可独立提交）：任务中心接入批（run_tracked 内联形态 + adapter + locale）——
    但 run_tracked 内联会被本计划退役 ⇒ **更合理：两批合一提交**，或批 1 只含
    adapter + types + locale（不含 run_tracked）。实施到收口时按树况定，到时列明。
  - `Cargo.lock` 的 ctutils 0.4.2→0.4.5 属并行会话，勿顺手提交。

## 4. 明确不做（本期）

- backtest 的 group 正负对比拆两次任务：现报告一次返回两组，拆分无收益。
- IPC 之外的 engine 内部耗时优化（批次数、限速）——不属本缺陷。

## 5. 待拍板项（已拍板，2026-10-23）

1. **取消能力一期做不做**：**做** ⇒ 新增阶段 E（见下）。
2. **提交切分**：用户已手动提交；本计划改动从干净树起步。

### 阶段 E — 取消能力（一期做，2026-10-23 拍板）

**口径：批间协作式取消，不逐 await 点**。130s 的 backfill 天然有 ~12 批 + 限速间隔，
批间检查足够及时；贯穿每个取数 await 点不必要且改动面失控。

1. **旗标传递**：`spawn_tracked` 建 `Arc<AtomicBool>` 取消旗标，存入模块级注册表
   （`tokio::sync::RwLock<HashMap<String, Arc<AtomicBool>>>`，遵守异步锁禁区 8），
   task 终态时从注册表摘除。
2. **engine 侧**：三条链的批循环签名加取消检查（实施时取改动面小者：`&AtomicBool` 参数
   或 `is_cancelled: impl Fn() -> bool` 闭包）：
   - `backfill_close_history_for_codes`：批间 + 限速间隔前检查，命中即返回已完成批次的
     部分报告——报告加 `cancelled: bool` 字段（HistoryBackfillReport）；
   - `sweep_market_close` 的 engine 批循环：同口径，CloseSweepReport 同加 `cancelled`；
   - `backtest_reco_strategies` 的正负样本循环：逐票间检查。
3. **stop 命令接线**：`stop_background_task` 命中本类任务（注册表有旗标）时置旗标返回，
   不直接改库——终态由 spawn 体观察到旗标后写 `stopped` + emit；注册表无旗标（历史行/
   非 spawn 任务）走既有状态迁移路径不变。
4. **取消语义**：取消≠失败——终态 `stopped`、报告落 result_json（部分完成，带
   `cancelled: true`）、output 摘要注明「用户取消，已完成 X/Y 批」。
5. **判据**：spawn_tracked 取消测（置旗标→终态 stopped + result_json 带 cancelled）；
   engine 层批间检查测（第一批后置旗标→不跑第二批）；stop 命令注册表命中/不命中两条。
6. **前端**：hook 暴露 `cancel()`；面板任务进行中给取消入口（MoverRecallPanel 的
   进行中提示 + 任务中心行既有 stop 按钮自动可用）。

## 6. 风险与边界

- **with_optional_asof × spawn（已解除）**：`as_of.rs` 的 task_local 不跨 spawn，但有进程级
  作用域栈回退。正确路线是**把 as-of 作用域建在 spawn 出去的任务内部**（`ctx` 以值显式 move
  进工作闭包，闭包内 `with_optional_asof(ctx, inner)`）——task_local 与全局栈都随任务走、
  退出即清，不依赖调用方 task 的残留。已按此实现，四 crate `cargo check` 绿（1m59s）。
- `backfill_market_close_history` 99-130s 期间 spawn 的任务与 cron 触发的同一 engine
  实现并发：幂等写 0 行不算失败（既有裁定），报告里 no_new 单独成句——已有形态，不动。
- 面板旧用户习惯变化：按钮从「转圈 130s」变「立即返回 + 任务中心看进度 + 完成后正文出现」，
  MoverRecallPanel 的成功消息（codes/rows/noNew 插值）在报告取回后才弹出——时点后移要在
  组件测试里锁住（阶段 C 判据③）。

## 7. 执行状态（2026-10-24 开工中）

### 已完成

- **阶段 A（后端）**：
  - `background_tasks` 实体加 `result_json: Option<String>`（声明式对账正门，不写 migration）；
    `BackgroundTaskInfo` 同步加字段；dao 的 `spawn_task` 构造点补 `Set(None)`。
  - `run_tracked` 内联形态**已退役** → `spawn_tracked`（`commands/background_tasks.rs`）：
    建行 → `record_task_created` → 注册取消旗标 → emit created → `tokio::spawn`（Running →
    catch_unwind 执行 → 写 output 摘要 + result_json → 终态 + emit updated → 摘除旗标）→
    立即返回 `TaskAccepted { taskId }`。测试可走 `spawn_tracked_impl(None, …)`（无 AppHandle）。
  - 三命令改返回 `TaskAccepted`：`sweep_market_close` / `backfill_market_close_history`
    （`market_close_sweep.rs`，加 `app_handle` 参数）/ `backtest_reco_strategies`
    （`stock_analysis.rs`，**as-of 作用域移进 spawn 任务内部**，`ctx` 值显式传入）。
  - 四 crate `cargo check` 绿（1m59s）。
- **阶段 E（取消）**：
  - 新模块 `axagent_astock_data::task_cancel`（`CancelToken` + task_local + `is_cancelled()`，
    **刻意不做全局回退**：全局旗标会让一个任务的取消误伤并发任务；三链 engine 循环均在
    当前任务内顺序执行、不内部 spawn）。自带 3 条单测（作用域外恒 false / 置位可见 / 嵌套隔离）。
  - 三条链批间检查：`backfill_close_history_for_codes`（票间）、`run_close_sweep`（批间）、
    `run_group` 的 K 线取数循环（票间）；`CloseSweepReport` / `BackfillStats` /
    `BacktestComparisonResponse` 加 `cancelled: bool`。
  - `stop_background_task`：注册表命中 ⇒ 只置旗标（终态由 spawn 体写 stopped，避免两处写终态）；
    未命中 ⇒ 走既有状态迁移。取消≠失败：终态 stopped、部分报告照落 result_json、
    output 以「用户取消（已完成部分）：」开头。
  - `spawn_tracked` 五条测试：成功（output 摘要 + result_json）/ 业务失败（无 result_json）/
    panic（failed 非僵尸）/ 取消（stopped + 部分报告 cancelled=true）/ 未注册的 stop 不报错。
- **阶段 B（前端基座）**：`TaskAccepted` + `BackgroundTaskInfo.resultJson` 类型；
  `useTrackedTask` hook（事件订阅 + 2s 轮询兜底 + 终态取 result_json + 过期任务归属丢弃 +
  cancel）；browserMock 三命令改任务形态（三份报告搬进 result_json，新增
  `get_background_task_output` / `stop_background_task` mock）；hook 4 条单测绿。
- **阶段 C（消费点）**：5 处改造 + 4 个测试文件 mock 改任务形态；
  `RecoStrategyMatrix` 切正/负组**不再发任务**（死参数 `group` 退役，本地重排）；
  取消后的部分报告在矩阵与 BacktestPanel 上带「已取消：仅含已完成部分」徽标。
  i18n：`tasksCenter.tracked.*` 3 键、`stockAnalysis.moverRecall.task{Running,Cancel,Cancelled}`
  3 键、`stockAnalysis.backtest.cancelled` 1 键，11 语言两趟写 + 回读逐字校验。
- **读数**：tsc 0；定点 vitest 11 文件 / 156 条全绿（含任务中心 40 条 adapter 回归）；
  dprint 本轮文件全过；oxlint 0。

### 未完成 / 待取读数

- 主 crate `cargo check -p axagent --all-targets`：**被并行会话的构建锁挡住**
  （日志只有 `Blocking waiting for file lock`；共享树有 astock-data vendors / rt-workflow 的他人改动）。
- 定点 clippy（`-p axagent` / `-p axagent-astock-data` / `-p axagent-analysis-engine`）未跑。
- `__TAURI_WORKSPACE__=true cargo test -p axagent --lib spawn_tracked` 未跑。
- 全量门禁（fmt --all / clippy --workspace / test --workspace / 全量 vitest）未收口。
- 已知测试边界：engine 层「批间置旗标后不跑第二批」**没有独立 engine 级测试**
  （`run_close_sweep` / `backfill_close_history_for_codes` 依赖真实 AStockClient，无可注入替身）；
  现由 `task_cancel` 单测 + `spawn_tracked` 取消测（同一 `is_cancelled()` 调用）覆盖接线，
  engine 内的三行检查属代码审查面。若要 engine 级判据，需先给 engine 注入 provider 抽象。
- 本批未提交（共享树，等全量门禁与用户裁定）。
