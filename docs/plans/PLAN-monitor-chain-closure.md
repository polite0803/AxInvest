# PLAN · 股票「监控」链收口（A→B(+C)→E→F→G→D→H）

> 状态：active ｜ 日期：2026-10-14 ｜ 对应版本：v2.9.9


> 立项日期：2026-10-14。来源：同日缺陷评估（会话结论，未另落 AUDIT）。
> 范围裁定：按用户批准的优先级顺序执行 —— **A/B/C 必修 → E 判定质量 → F/G 未接线层 → D/H 假旋钮与呈现**。
> 本文所有 `文件:行号` 取 **2026-10-14 工作树**（脏树，含并行批次 13 个未提交文件）。路径一律写完整仓库相对路径（AGENTS.md「段对齐」规约）。**动手前按符号名重新 grep 定位，不要照抄行号。**

---

## 0. 两句话讲清这次要解决什么

价格告警引擎在跑，**但它的产出没有任何消费者**：事件名两头对不上、触发状态永不回写、通知层从未装配。同时同一只股票只能存活一条告警 —— 现网 56 条告警里，实际进入实时监控的最多 16 条。

本 PLAN 的第一目标是让「告警触发 → 用户看到 → 状态落库」这条主链真的闭合，再谈判定质量与未接线的组合层。

## 1. 现网基线读数（2026-10-14，经 `scripts/pg-connect.mjs sql`）

| 读数 | 值 | 用途 |
|---|---|---|
| `price_alerts` 总行数 | 56 | 影响面 |
| `price_alerts` 去重股票数 | 16 | 按「每股仅一条 config 存活」⇒ 生效上限 16 条，**约 40 条（71%）从未被监控** |
| `is_triggered` 分布 | 全部 0（无一行 1） | 证实触发回写从未发生 |
| 单股最多告警行数 | 6（`601899` / `000400` / `601127` / `600406`） | B 阶段验收要按条数对上 |
| `alert_type` 分布 | `stop_loss` 28 + `take_profit` 28 | `change`/`volume`/`resistance`/`support` 现网 0 条 |
| `portfolio_metrics_daily` | **0 行** | 组合监控「不是算错，是从来没有数据」 |
| `portfolio_holdings` / `trades` | 0 / 0 行 | 同上，G 阶段验收必须先有数据 |
| `watchlist_items` | 1 行 | M5 行情监视当前只覆盖 1 只 |
| `settings` 键 `conditional_orders` | **不存在** | F 阶段退役那套存储无迁移成本 |

**读数带来的两个排序修正**（相对口头评估）：
1. `monitor_emitter.rs` 需要的 `NotificationDispatcher` 在**生产里零构造点**（`register_channel` / `configure_route` 无任何调用方，四个渠道类型只在 `src-tauri/crates/notification/src/lib.rs:38` 被 re-export），且全仓没有 `commands/notification*.rs`、`crates/notification` 不读 `panel_variables`。**通知层是整层未装配，不是接线**。故 B 阶段不含通知，通知另立里程碑（见裁定 7）。
2. G 阶段的组合监控缺的是「数据面为空 + 算法恒零」两层，验收门必须打在真数据上，纯函数层测试不足以证明可用（既有纪律：参数决定结构的批要「纯函数 + 真落库」两层各一条）。

## 2. 提交策略（先看这张表再动手）

并行批次（估值/反思重构）已改未提交 13 个文件。`git add` 是**整文件粒度**（禁用 `-p` 交互），因此凡触碰下列文件的批次**只改不提交**，等那批落地后统一切：

| 本 PLAN 会触碰的文件 | 是否与并行批次同文件 | 该阶段能否独立提交 |
|---|---|---|
| `src-tauri/src/init/services.rs` | 是（其新增落在 `start_cron_scheduler`，本 PLAN 改 `start_realtime_monitor`，hunk 不相交但同文件） | ✖ 只改不提交 |
| `src-tauri/src/commands/stock_analysis.rs` | 是（其 hunk 在 4763 行之后，本 PLAN 改 1702/2221/2910 一带） | ✖ 只改不提交 |
| `src-tauri/src/init/stock_automation_cron_seed.rs` | 是（其改动就在 `build_builtin_jobs`，**同函数真重叠**） | ✖ 只改不提交 |
| `src-tauri/crates/analysis-engine/src/monitor.rs`、`alert_mapping.rs`、`conditional_order.rs`、`risk_inspection.rs`、`portfolio_monitor.rs`、`cross_stock_aggregator.rs` | 否 | ✔ |
| `src-tauri/src/init/monitor_emitter.rs` | 否 | ✔ |
| `src-tauri/crates/astock-data/src/{lib,calendar,realtime_quote}.rs` | 否 | ✔ |
| `src-tauri/crates/harness/src/event_names.rs` | 否 | ✔ |
| `src/components/stock-analysis/PriceAlertPanel.tsx`、`WatchlistPanel.tsx`、`src/i18n/locales/*.json`、`scripts/*.mjs` | 否 | ✔ |

⇒ **A/B/E/H 可以小提交**（它们的 Rust 侧在 analysis-engine + init/monitor_emitter，前端在组件与 locales）；**凡需要改 `services.rs` 装配点的动作，与对应阶段同批只改不提交**，并在提交信息里点名等待的批次。

## 3. 阶段 A —— 告警通路收口

**缺陷**：引擎发字面量 `"stock-monitor-alert"`（`src-tauri/crates/analysis-engine/src/monitor.rs:433`），现役发射器只做 `let _ = self.app.emit(event, payload)`（`src-tauri/src/init/services.rs:3772-3776`）；前端只听 `price-alert-triggered`（`src/components/stock-analysis/PriceAlertPanel.tsx:60`），而发该名的 `TauriMonitorEmitter`（`src-tauri/src/init/monitor_emitter.rs:38`）全仓零构造点。`stock-monitor-alert` 不在 `src/types/generated/events.ts` 白名单内。

**改法（推荐形态，见裁定 1）**：接线既有完整实现，退役重复的三行胶水。
1. `src-tauri/src/init/services.rs` 的 `start_realtime_monitor`：用 `TauriMonitorEmitter::new(app, db, dispatcher)` 取代 `AppMonitorEmitter`；删除 `AppMonitorEmitter`（struct + impl）整体。dispatcher 先传 `Arc<NotificationDispatcher::new()>`（空渠道，通知层在 B2/里程碑 M-NOTIF 装配），**并在 emit 侧如实记录「通知渠道未装配 ⇒ 本轮只走事件 + 落库」**，不得静默。
2. `src-tauri/crates/analysis-engine/src/monitor.rs`：emit 的事件名改用权威枚举，禁止字面量 —— 但 analysis-engine 是 implementor，不能依赖 tauri；正解是**由调用方（wiring 层）传入事件名**：`MonitorEventEmitter::emit` 保持 `&str`，把 `monitor.rs` 里的字面量改成 `crate::monitor::ALERT_EVENT_NAME` 常量并由 `harness::event_names` 侧对齐 —— 具体落点在实施时二选一，但**判据锁死「事件名只有一处权威」**（见门 A-2）。
3. `src-tauri/src/init/monitor_emitter.rs`：删掉「兼容别名」这一层（`monitor_emitter.rs:64-69`）—— 两头都有别名说明中间没人做主，收口后 emit 一次用 `IpcEventName::PriceAlertTriggered.as_str()`。
4. 前端 payload 契约：`PriceAlertPanel.tsx:60` 的泛型是 `PriceAlert`（含 `id/condition/targetPrice/isTriggered`），而后端发的是 `MonitorAlert`（`monitor.rs:77-88`：`stockCode/alertType/alertMessage/currentPrice/changePct/timestamp/suggestedAction`）。当前 handler 忽略 payload 所以不崩，属契约漂移。⇒ 新增 `MonitorAlertPayload` 类型（放 `src/types/`，按禁区 12「前端权威来源」），handler 用 payload 显示「哪只股、哪类阈值、现价」。

**判据**
- 门 A-1（组件测试，运行门）：`PriceAlertPanel` 收到 `price-alert-triggered` 后必须渲染出 payload 里的股票代码与 `alertType` 对应中文；破坏样本两条各红一条（改 payload 键名 / 撤 handler 消费）。
- 门 A-2（脚本，扩既有门优于造新门）：新增 `scripts/check-monitor-event-authority.mjs` —— 扫 `src-tauri/crates/**` + `src-tauri/src/**` 剥注释后的 `app.emit("…")` / `emit(` 实参，断言**股票告警相关事件名只能出现 `IpcEventName::X.as_str()` 或 `as_of` 常量引用**，字面量白名单从 `src-tauri/crates/harness/src/event_names.rs` 的 `as_str()` 实现现场推导（禁手抄清单，且断言推导结果 ≥ 下限）。负控：假别名包成 `Some(..)` 仍要命中。
- 门 A-3（Rust 定点）：`MonitorEventEmitter` 的每个 `impl` 类型必须在非 test 生产代码里有构造点。**数构造点禁用 `grep 'T {'`**（`fn f() -> T {` 的尾巴会命中，前一轮把 5 行虚报成 20 处就是这么来的）⇒ 按「实参位 / 赋值位」形态匹配，并配「全部实现退役 ⇒ 集合为空」的负控。

**验收读数**：`npm run dev` 浏览器模式下 mock 不算数；须 `npm run tauri dev` + 面板建一条 stop_loss 告警（现价下方）+ 等到轮询触发，确认①弹窗出现 ②「已触发」Tab 出现该行 —— 若 B 未开工，②必然失败，**A 阶段收尾时如实报「②归 B」**，不得用①的成功冒充整链闭环。

**执行状态（2026-10-15，阶段 A 代码面完成，终局读数未取全）**
- 已落：`monitor.rs` 发射点改用 `IpcEventName::PriceAlertTriggered.as_str()`；`monitor_emitter.rs` 撤「兼容别名」层（DB 回写与通知改判权威名，零渠道时显式 warn 不外推）；`services.rs` 改构造 `TauriMonitorEmitter` 并整体删除 `AppMonitorEmitter`；前端新增 `MonitorAlertPayload`（`src/types/stock-analysis.ts`，经 barrel 导出）+ `PriceAlertPanel` 上屏「股票名 / 阈值类型 / 现价」；8 个新键补满 11 语言（parity 门 15870/15870 一致、`check_i18n.py --strict` 通过）。
- 已跑的读数：门 `scripts/check-monitor-event-authority.mjs` 自证 21/21、门禁绿；同一套判据打在**未修的 HEAD** 上取过红读数（R1 点名 2 个文件、R2 红、R4 红「前端接口不存在」、R5 红且只点名 `TauriMonitorEmitter`，`AppMonitorEmitter` 被正确认成已装配）；组件测试 3 条 + 三形态变异各逼出预期红条数（M1 红 3、M2 只红名字回退那条、M3 只红未知兜底那条），还原后全绿；`npm run typecheck` 0 错；`stock-analysis` 目录 49 文件 315 条测试全绿；`rustfmt --check` 单文件过（`monitor.rs:4` 的 import 排序 diff 在 HEAD 上就有，非本批造成，未动）。
- 未取的读数：`cargo clippy` 定点（`-p axagent -p axagent-analysis-engine --all-targets --all-features -- -D warnings`）与带 `--all-targets` 的复跑 check —— 首轮 `cargo check -p axagent-analysis-engine -p axagent` 已于 2026-10-15 00:36–01:14 跑完，**EXIT=0、零 error 零 warning**（37m40s，日志 `.workbuddy/tmp/a1-rust-check.log`）；但 `monitor_emitter.rs` 那处 `warn!` 的 rustfmt 换行是在该轮之后改的，故补一轮终局读数。真机核对（上面那段验收）归用户侧。
- 与 PLAN 的两处偏差（都是实施时发现规格不够，不是漏做）：
  1. **门 A-3 落在脚本层而不是 Rust 测试**：判据要「剥注释 + 剥 `#[cfg(test)]` 后扫源文」，Rust 测试没有这个基础设施（源文扫描依赖运行期 CWD），而脚本层已有现成的剥离器可复用 —— 于是 A-2/A-3 合成一扇门，R5 就是原 A-3。剥离器不复制第三份，改为从 `check-panel-var-landing.mjs` import（同时给它补了 `import.meta.url` 运行守卫，守卫前后门与自证读数逐字不变）。
  2. **发射点 payload 改由 `MonitorAlert` 直接序列化**：PLAN 只要求事件名收口，但原来那份手写 `json!` 摊平的副本正是漂移源 —— 阶段 B 给结构体加 `alert_id` 时，副本会静默少一键而两侧都不报错。改成 `serde_json::to_value(alert)` 后，门 R4 才有「逐项对账两侧键集合」的落点。序列化失败走 `tracing::warn!` 且该条仍已进 broadcast channel，不静默丢弃。

## 4. 阶段 B（吸收原 C）—— 触发即终态 + 按条粒度

**为什么原 C 合进 B**：回写触发状态必须能「只摘掉这一条」，而当前监控配置是 `HashMap<股票代码, MonitorConfig>`（`monitor.rs:93`），创建时整体覆盖（`src-tauri/src/commands/stock_analysis.rs:2265-2297`），删除时整只退监（`stock_analysis.rs:2346`）。不改键粒度就写不出正确的回写。顺序不变、条数不减。

**改动清单**
1. `monitor.rs`：`configs` 键由 `stock_code` 改为 `alert_id`；`MonitorConfig` 增 `alert_id: String` 字段（`remove_config(code)` → 拆成 `remove_config_by_id(id)` + 新增 `remove_configs_by_code(code)` 供退监场景）。启动恢复段（`services.rs:3840-3876`）改为一条 `price_alerts` 行 → 一个 config（携带该行 id），并把「同股多条」如实计入日志条数。
2. `stock_analysis.rs` 的 `create_price_alert`：只填对应字段的语义保留，但 `add_config` 按 id 插入不再覆盖；`delete_price_alert` 改为按 id 摘除。
3. `src-tauri/src/init/monitor_emitter.rs` 的 `handle_alert_event`：现按 `(stock_code, alert_type, is_triggered=0)` 命中并**把匹配到的全部行**置 1（`monitor_emitter.rs:100-121`，注释自己承认这个扇出）。改为按 emit payload 携带的 `alertId` 精确定位单行；payload 缺 `alertId` 时**显式 warn 并按旧口径回退**，不得静默当空集。⇒ `MonitorAlert`（`monitor.rs:77-88`）需增 `alert_id`。
4. **重启不再复活已触发告警**：恢复段本身已带 `is_triggered=0` 过滤（`services.rs:3836`），B-1/B-3 生效后自然成立 ⇒ 该条改为门而不是代码（门 B-1）。
5. 每日复盘 `generate_daily_review`（`stock_analysis.rs:2909-2914`）：注释说「当日」但查询无日期条件 ⇒ 补 `triggered_at` 当日窗口（北京日口径，与 E-5 同一 helper），并把「0 条」与「查询失败」两种缺席分句呈现。

**现网读数补两条（2026-10-15，动 B 之前取的）**
- `(stock_code, alert_type)` 有重复的组合 **16 个**（如 `601899` 有 3 行 `stop_loss`）。⇒ 现在那份回写按 `(code, alert_type, is_triggered=0)` 命中会**一次置 1 三行**，比「扇出」注释说的更严重；B-3 改按 `alert_id` 精确命中是必需而非优化。
- 重复行的来源不是手建：分析工作流每次跑都给同一股追加 `price_alerts` 行（`src-tauri/src/commands/stock_workflow/core.rs` 的两处写入），于是行数随分析次数增长。
  ⚠ **这条直接改变 B-1 的代价**：改按条粒度后，轮询是「遍历配置逐只取行情」（`monitor.rs` 的 `for config in configs` + `get_quote`），56 条配置 = 56 次请求/轮，而今天按股票去重只 16 次。⇒ **B 必须与 E-6（批量取数）同批做**，或在 B 里按 `stock_code` 聚合请求、按条求值（后者更省且不依赖批量字段齐备性）。这条不写进 B 的改动清单，就会出现「门全绿而请求量翻 3.5 倍」。

**判据**
- 门 B-1（Rust 真库测试，`dao` 侧允许 `create_test_pool`）：同股建 2 条不同类型告警 ⇒ 两条都进入监控配置；触发其中一条 ⇒ 只有那一行 `is_triggered=1`，另一行仍 0 且仍被监控。夹具要用生产键域真值（股票代码取现网实际存在的 `601899` 形态，不造 `TEST001`）。
- 门 B-2（脚本）：`price_alerts.is_triggered` 必须在生产代码里**既有写侧又有读侧**，且写侧不在 `#[cfg(test)]` 段内（沿用 `check-panel-var-landing.mjs` 的「先剥注释再剥 test mod」口径 —— 测试替自己作证是前一轮实测过的假绿来源）。
- 门 B-3（呈现层组件测试）：「已触发」Tab 空态文案与「监控引擎未装配」文案不得是同一句（三种缺席各占一句）。

**验收读数**：跑一次真触发后 `SELECT is_triggered, count(*) FROM price_alerts GROUP BY 1` 必须出现非零的 1 桶；基线是「全部 0」。

**执行状态（2026-10-15）**
- 已落：配置键 `stock_code` → `price_alerts.id`（`MonitorConfig.alert_id`）、轮询**按股票聚合取数、按条求值**（请求数不随告警行数增长，本批不依赖 E-6）、冷却键改按条目、`MonitorAlert` 带 `alertId` 且发射点已同步；四个构造点全部补齐（`services.rs` 恢复段 / `stock_analysis.rs` 手建 / `stock_workflow/core.rs` 两处自动建），启动日志改为「注入 N 条配置 / 未触发库存 M 行 / 涉及 K 只股票 / 无法映射 U 条」四个数并列。
- 回写改为两条路径：`mark_triggered_by_id`（正门，查不到行就点名）与 `mark_triggered_by_code_and_type`（仅在 payload 缺 `alertId` 时走，并 warn 说明会扇出）。
- **顺手抓到一个运行期硬缺陷（有实证）**：复盘段那句「当日已触发」写的是 `IsTriggered.eq(true)`，而该列在库里是 integer —— PG 实测报「操作符不存在: integer = boolean」，错误又被 `if let Ok` 吞掉，于是那段永远读成「今天没有告警」。已改为 `eq(1_i32)` + 北京日窗口（新增 `astock-data/src/calendar.rs::beijing_day_window_ms`，E-5 与 G-6 都复用它）+ 失败时 warn。窗口口径取自 `beijing_today()`，不再用 UTC 日期。
- 判据：**没有另造第三扇门** —— R6 扩进阶段 A 那扇 `check-monitor-event-authority.mjs`（写侧 `Set(1)` 存在、`eq(0)`/`eq(1)` 两类读侧都在、禁 `eq(true)`），自证 27/27；R4 随 payload 加字段自动收到 9 键。Rust 侧两条行为锁：`analysis-engine` 的 `monitor::tests`（同股两条都存活且各报一次 / 同条冷却 / 同 id 覆盖）与 `src-tauri` 的 `monitor_emitter::tests`（按 id 只改那一行 / 回退口径的扇出留档 / 查不到 id 不误伤）。
- 与 PLAN 的四处偏差（都记在此，不是漏做）：
  1. **门 B-3 撤下、并入 H-4**：「已触发空态」与「监控引擎未装配」两种缺席的呈现本来就是 H-4「缺席三态上屏」的一格，同一条呈现规格不做两遍。
  2. **不加 `remove_configs_by_code`**：PLAN 列了它，但全仓没有退监消费者 —— 新造一个零调用者的方法正是本轮在别处判为缺陷的形态。只留 `remove_config_by_id`。
  3. **E-1 的前半已随循环重构落地**（取数失败必须 warn，不再 `if let Ok` 静默）；失败计数与面板呈现仍归 E-1。
  4. **触发即终态后不给「重新激活」按钮**：面板已有新建表单，重建一条等价；新加一个命令 + 11 语言键的收益不抵面。要真做「原条复活」需另开裁定。
- 新增待裁定 10：**`price_alerts` 行随分析次数无界增长** —— 工作流每次分析都给同一股追加 stop_loss/take_profit 两行（`src-tauri/src/commands/stock_workflow/core.rs` 两处写入），既不更新旧行也不去重。触发即终态生效后，同一股会留下 N-1 条仍活跃的重复行继续监控同一个语义。选项：① 建前按 `(code, alert_type)` 复用未触发行；② 保留追加但给面板一个「清理陈旧重复」动作；③ 维持现状（则监控配置数会一直涨）。**→ 2026-10-15 拍板 ① 并已落地，见 §4.1。**
- 读数状态：`cargo fmt`（repo 工具链）对 `axagent-analysis-engine` 0 diff、主 crate 里本批碰过的文件均不在 diff 名单；`npm run typecheck` 0 错；告警事件组件测试 3/3；门 27/27 自证 + 门禁绿；同一套判据打在未修的 HEAD 上 R1/R2/R4/R5/R6 全红、R3 绿（断链在发射侧）。`cargo test`（两扇新写的 Rust 测试）与 `clippy` 仍在构建锁上排队，读数出来前本阶段不称「完成」。
- **Rust 测试已取的第一半**：`cargo test -p axagent-analysis-engine --lib monitor::` ⇒ `monitor::tests` 三条**逐条 ok**（同股两条都存活且各报一次 / 同条冷却 / 同 id 覆盖），套件读数 `31 passed; 0 failed; 626 filtered out`，`AE_TEST_EXIT=0`。
- **Rust 测试已取的第二半**：主 crate `cargo test -p axagent --lib monitor_emitter::` ⇒ `running 3 tests` → `3 passed; 0 failed; 616 filtered out`（7.77s），`MAIN_TEST_EXIT=0`，三条逐条 ok：按 id 只改那一行 / 回退口径的扇出留档 / 查不到 id 不误伤别的行。走 `create_test_pool` 的临时 SQLite 文件库（与生产同一个 `initialize_schema`），**不碰现网 PG、不需要清理真实数据**。
- ⚠ 上面两半的读数都发生在 `std::sync::Mutex` 那处 clippy 修正**之前**：改的是类型书写路径（`use` 提到模块顶 → 字段写全限定名），不动语义；仍按「改动后的读数才算本批读数」的规矩补跑了一次 `monitor::tests::` 复跑，读数记在下一条。
- ⚠⚠ **读数口径自查（2026-10-15，本批最要紧的一条自我更正）**：有几条 cargo 命令是从仓库根目录带 `--manifest-path src-tauri/Cargo.toml` 跑的，而 rustup 按**当前目录**解析工具链 —— 根目录没有 `rust-toolchain.toml` ⇒ 实际跑的是 stable（clippy 1.96），仓库在 `src-tauri/rust-toolchain.toml` 里 pin 的是 **1.97.0**。实测对照：根目录 `rustup show active-toolchain` = `stable-x86_64-pc-windows-msvc`，`src-tauri` 内 = `1.97.0-x86_64-pc-windows-msvc (overridden)`。两半后果：① 从根目录跑出的 `AE_CLIPPY_EXIT=101` 是**工具链伪红** —— 报点在别人的 `crates/analysis-engine/src/recommender/scoring.rs:67` 的 `nonminimal_bool`，帮助链接写的是 rust-1.96.0，而这条只在 1.96 报、1.97 绿（已记档的既有事实），不是本批代码的问题也不是 CI 口径；② 同一批从根目录跑的测试确实跑通了（测试名逐条 ok、套件计数在内），但那不是 CI 用的编译器。⇒ 已在 `src-tauri/` 目录内用 pin 的工具链重跑三条（两枚 crate 的 `cargo test --lib` 定点 + `cargo clippy --all-targets --all-features -- -D warnings`），终局读数记在下面。
- **B 判据的变异自证已到（2026-10-15，脚本 `.workbuddy/tmp/b-mutants.py`，两趟法 + try/finally 还原）**：M1「配置键退回股票代码」⇒ 实测只红 `same_stock_two_alert_rows_both_survive_and_both_fire`（套件 `30 passed; 1 failed`）；M2「冷却键退回 代码:类型」⇒ 同样只红那一条。两个变异都没有波及另外两条（冷却那条与覆盖那条各自仍绿），说明三条判据分格有效、不是「一改全红」。还原后复跑 `31 passed; 0 failed` 退出码 0，并按标识符回读确认文件现场（`configs.insert(config.alert_id.clone(), …)` 与 `let key = a.alert_id.clone();` 均回到原位、`alert_id` 出现 16 次、`cargo fmt -p` 0 diff、门 6/6 绿）。
- **pin 口径（1.97，从 `src-tauri/` 内发起）的终局测试读数已到**：`axagent-analysis-engine` ⇒ `running 31 tests` / `31 passed; 0 failed`（10m17s），我的三条测试名逐条 `ok`；`axagent` ⇒ `running 3 tests` / `3 passed; 0 failed`（17m08s），三条 `init::monitor_emitter::tests::*` 逐条 `ok`。两步退出码均 0。（`std::sync::Mutex` 那处修正后的复跑同样三条 `ok`；那条从根目录发起的 clippy 作业已被主动撤掉 —— 它的读数按定义不是 CI 口径，留着只会继续抢构建锁。）
- **clippy 在主 crate 首个 error 短路，报点不是本批代码**：`src-tauri/src/commands/backtest_validation.rs:495` 的 `neg_cmp_op_on_partial_ord`（`if !(reco_pick.price > 0.0)`）。三重归因给并行批次：① `git show HEAD:…backtest_validation.rs | grep -c "reco_pick.price > 0.0"` = **0**（HEAD 无此写法）；② 行号 495 落在新增 hunk `@@ -381,0 +426,134 @@` 的 426–559 区间内；③ 该文件 1045 行 → 1223 行。这条属**涉 NaN 的逻辑类**（`!(x>0.0)` 对 NaN 为真、`x<=0.0` 为假，机械改写会变语义）⇒ 不碰，等并行批次收口；本批那一面改用 `-A clippy::neg_cmp_op_on_partial_ord` 的修饰读数补定点证据。`axagent-analysis-engine`（本批 Rust 主战场）在 1.97 下**先于该 error 完成且干净**。：`cargo test -p axagent-analysis-engine --lib monitor::` ⇒ 三条逐条 ok（`add_config_with_same_id_replaces…` / `same_alert_row_is_cooled…` / `same_stock_two_alert_rows_both_survive_and_both_fire`），套件 `31 passed; 0 failed; 626 filtered out`，退出码 0。这一轮仍在 1.96 上跑（该命令发起于发现工具链问题之前）；pin 口径的终局三条（两枚 crate 的 test + `clippy --all-targets --all-features -- -D warnings`）已改从 `src-tauri/` 内发起排队，同时**撤掉了那条从根目录发起的 clippy 作业** —— 它的读数按定义不是 CI 口径，留着只会继续抢构建锁。
- **教训已入 §12 门禁口径**：抄门禁命令要连同**工作目录**一起抄 —— `cd src-tauri` 是这条 pin 生效的一部分，`--manifest-path` 替代不了它。
- **clippy 首轮抓到的是我自己的新代码**：`-D warnings` 报 `use of a disallowed type std::sync::Mutex` —— 测试模块的 `use` 行不被 struct 级的 `#[allow(clippy::disallowed_types)]` 覆盖。修法取同 crate 内 `portfolio_monitor.rs` 的 `MockProvider` 先例形态（字段写全限定路径 `std::sync::Mutex<…>`，不扩模块级豁免），未改语义、未加新 allow。主 crate 三条真库测试与两枚 crate 的 clippy 复跑已重新排队。
- 真机核对（建一条 stop_loss 等它触发 →「已触发」Tab 出现该行）归用户侧；这是 A+B 整链唯一还没被任何自动化读数覆盖的一格。

## 4.1 裁定 10 的执行状态（2026-10-15 拍板 ① 后落地）

| 子项 | 做了什么 | 现场锚点 |
|---|---|---|
| 唯一写库入口 | 新文件 `src-tauri/src/commands/price_alert_upsert.rs`：按 `(stock_code, alert_type, is_triggered=0)` 查回旧行 ⇒ 原地改阈值（**保留 `id`**，所以内存引擎的 `add_config` 天然覆盖而不是留两条在盯同一语义）；查不到才插新行。已触发行是终态，不吸收新单 | `upsert_price_alert()` |
| 类型归一 | `alert_mapping::normalize_alert_type`（6 类标准值 + 老 `above/below/change_up/change_down/volume_spike` 都归到同一类）与 `legacy_condition_for`（写侧兼容列，与读侧 `legacy_condition_to_alert_type` 严格互逆，配 `legacy_condition_round_trips_through_alert_type` 钉住） | `crates/analysis-engine/src/alert_mapping.rs` |
| 配置装配收口 | `config_for_alert()` 取代「面板那条 6 臂 match + 工作流那两处手抄 11 字段字面量」（禁区 12）；未知类型返回 `None` —— **不落「六个阈值全空、占位却不盯任何东西」的配置**，配 `single_threshold_of` 让「有且只有一个字段」这件事可被断言 | 同上 |
| 四处写入点改路 | ① `commands/stock_analysis.rs::create_price_alert`（面板）② `commands/stock_analysis_bridge.rs` 的 `stock_create_price_alert`（agent 工具）③④ `commands/stock_workflow/core.rs` 的止盈行与止损行 —— 后两处原先合用一个本地 `async fn put_auto_alert`，并撤掉 `if let Ok(inserted)` 吞错（插入失败时界面仍显示「分析已完成」，实际一条也没盯上） | core.rs 自动建单块 |
| 退役的死码 | `extract_alerts_from_config`、`alert_to_db_fields`（本批开工前已是全仓零调用，且后者的文档注释声称的接线形态会把「每次触发插一行」引进来）与其 test | alert_mapping.rs |

**判据与读数**
- 门 R11（`scripts/check-monitor-event-authority.mjs`）：新行构造点**全局恰好一处**，且那一处必须同时含 `IsTriggered.eq(...)` 去重谓词（构造与复用分家 ⇒ 删掉复用查询没人会察觉）。只数 struct literal，不数 `= row.into()`（触发回写那条路）也不数 `fn f() -> price_alerts::ActiveModel {` 的返回类型尾巴 —— 后者是 R5 记过的同一陷阱。
- 计数口径按**出现次数**而不是文件数：改造前 `core.rs` 一个文件里就有两处，按文件数会把「4 处」读成「3 处」。
- 自证：门 `--selftest` 在 R11 落地时 **51/51**，随后为裁定 12 补 **R12**（`monitor_status` 两侧键集合逐项相等，含「写成 snake_case 即红」「结构体名抄错＝判据没电即红」两条负控）后 **55/55**；把 HEAD（未修）源文喂进同一条 R11 纯函数判据 ⇒ **红：新行构造点 4 处**（stock_analysis×1、stock_analysis_bridge×1、core×2）；本轮工作树 ⇒ **绿：唯一构造点 `price_alert_upsert.rs`**。脚本 `.workbuddy/tmp/probe-r11-head.mjs`（不落盘、不碰共享树）。
- Rust 测试：`price_alert_upsert::tests` 四条真库（临时 SQLite，走 `create_test_pool`）—— 同股同类型建两次仍一行且阈值取第二次 / 已触发行不被复活也不被改阈值 / 同股不同类型仍两行（第二轮不再涨） / 老拼写 `below` 与新拼写 `stop_loss` 归到同一行。
- **读数（pin 1.97，`src-tauri/` 内发起，2026-10-15 15:2x–15:5x）**：`cargo fmt --check`（`-p axagent -p axagent-analysis-engine -p axagent-astock-data`）**EXIT=0**；定点 `cargo clippy -p axagent-analysis-engine -p axagent-astock-data --all-targets --all-features -- -D warnings` **EXIT=0**；`cargo test -p axagent-analysis-engine --lib` 首轮 **`675 passed; 1 failed`**（就是我那条错前提判据，见上一条），改写后复跑 **`running 676 tests` / `676 passed; 0 failed`，`AE_EXIT=0`**；主 crate `cargo test -p axagent --lib price_alert_upsert::` ⇒ **`running 4 tests` / `4 passed; 0 failed; 619 filtered out`**（8.43s）。`cargo check -p axagent --lib` 唯一一条 warning 是我自己留的死码（`KNOWN_ALERT_TYPES` 零调用 ⇒ 已删，并把只在测试里用的 `alert_types` import 降进 `mod tests`，避免换个新警告）。
- ⚠ 顺带实证禁区 7 的「第二步是编译器做的」：`monitor_status` 命令写完即出现在 `src-tauri/src/register_commands.rs:1279`（该文件 mtime 晚于源文件），**手工去改它才是错的**；而 `price_alert_upsert` 这个新 `pub mod` 因为不含 `#[tauri::command]`，生成物里一条都没多 ⇒ 与「build.rs 把 `::{a,b}` 与 `::*` 混同」那条陷阱无关，登记正常。
- 存量数据：**现网 56 行里那 16 组重复行不自动删** —— 删活库数据是不可逆动作，要单独点头。收口后新建/新分析不再涨行数。
- ⚠ 接线时发现的新缺口 ⇒ **裁定 15**（agent 工具那条路只写库、不进 `RealtimeMonitor`，要重启才生效；与命令路的即时生效不一致）。
- ⚠ **判据第一版按错前提写的（本批实测抓到，读数 `675 passed; 1 failed`）**：我写的 `legacy_condition_round_trips_through_alert_type` 断「6 类 ⇄ 老 `condition` 列严格互逆」，而老列只有 `above`/`below`/`change_up`/`change_down`/`volume_spike` 五个取值 —— **6→5 天生装不下**，`resistance` 与 `take_profit` 都写 `above`。测试当场红在 `resistance → above → take_profit`。⇒ 改判据而不是改语义：独占组（止损/止盈/涨幅/换手）仍断原样读回，共享组（阻力/支撑）改断「与兄弟类同值 + **不得**拿兼容列当去重键」，并把这条约束写进 `legacy_condition_for` 的文档。裁定 10 的去重键一直是 `alert_type`，所以这条错前提只伤测试，没伤行为。
- ⚠ 顺带查实的两处老列泄漏到文案面（不在本批做，记成 **H-8**）：`commands/stock_analysis.rs:2905-2912`（复盘报告把 `alert.condition` 的英文 token `above`/`below` 直接拼进用户可见句子）与 `src/init/services.rs:3876`（同一份 token 进通知文本）。

## 5. 阶段 E —— 判定质量与新鲜度

| 子项 | 缺陷锚点 | 改法 |
|---|---|---|
| E-1 取数失败不再静默 | `monitor.rs:237` `if let Ok(quote)` | 改为 match：失败计入本轮 `failed_codes`，`warn` 带股票与错误类别；连续 N 轮全失败时输出一条「监控实际未在取数」的 error。**呈现层归 H**（面板要能看见「本轮 x/y 只取数失败」） |
| E-2 涨跌停 / ST 语义 | `check_alerts`（`monitor.rs:286-411`）从不读 `limit_up/limit_down/is_st`；而 sina 臂这些字段是 `None`/`false`（`src-tauri/crates/astock-data/src/sina.rs:386-398`） | `MonitorAlert` 增 `limit_state` **四态**（涨停/跌停/未知/无），未知不得写成「未封板」；呈现句子按裁定 6 决定标注还是抑制 |
| E-3 穿越边沿 vs 单边比较 | `monitor.rs:308/329/349` 全是 `>=`/`<=` ⇒ 价格在阈值一侧停留期间每过一个冷却窗口重复报 | 记「上次符号位」，只在 假→真 边沿报；首次观察已越界按裁定 3 处理 |
| E-4 旧价续命（真 bug） | `src-tauri/crates/astock-data/src/lib.rs:974-977`：L2 命中后用 `now + 3600` 回写 L1，而写入是一律双写 L2（`lib.rs:1013`）⇒ 30 秒 TTL 的报价可在 L1 活一小时 | `disk_cache::get` 增返回 `(value, expires_at)` 的取法（`src-tauri/crates/astock-data/src/disk_cache.rs:165` 目前只返 value），L2 命中按**剩余 TTL** 回写，剩余 ≤0 视为未命中 |
| E-5 时段闸门 | 闸门只在告警循环（`monitor.rs:233`）；行情监视器无闸门 ⇒ 凌晨仍每 2s/10s 逐只发请求。边界写法：`calendar.rs:280` `hour==11 && minute<=30`、`calendar.rs:291` 15:00 仅 `minute==0` | 抽 `trading_session_of(now)` 返回枚举（盘前/盘中/午间休市/收盘后/非交易日），告警循环与 watcher 共用；节假日表（`calendar.rs:49-81`）补年份覆盖并加「表过期 ⇒ 红」的判据 |
| E-6 逐只请求 vs 批量 | `monitor.rs:236-239` 每只一次 `get_quote`；腾讯批量口径已实现（`src-tauri/crates/astock-data/src/vendors/tencent.rs:409`，80 码/请求）但唯一调用者是收盘归档；`batch_get_quotes`（`src-tauri/crates/astock-data/src/batch.rs:121`）零实现 | **先验证批量返回字段齐备**（`turnover_rate`/`limit_up`/`is_st` 是否都在），齐备才把轮询切批量；不齐备则本项挂为「等上游」，不得为了让数字好看而砍字段 |

**判据**：门 E-1（`disk_cache` 单元测试：写入 TTL 30s ⇒ L2 命中后 L1 过期时间不得超过原始到期点；破坏样本 = 把回写改成固定 3600 必须红）；门 E-2（Rust 测试：`limit_state` 四态各有样本，含「vendor 返 None ⇒ 未知」而非「无」）；门 E-3（`check_alerts` 行为测试：价格持续越界只报一次，回落再越界才第二次报）。E-5 补 `calendar` 的会话边界表驱动测试（含 11:30/13:00/15:00 三个临界分钟）。

**执行状态（2026-10-15）**
- E-1（部分）：轮询里的 `if let Ok(quote)` 改成 match，取数失败逐只 warn 并点名「本轮该股票未求值」。**PLAN 里的另一半（连续多轮全失败 ⇒ 一条聚合 error）刻意没做**：计数器在本轮没有任何消费者，正是阶段 D 正在退役的那类假旋钮 ⇒ 挪到 H，与面板「本轮 x/y 只取数失败」同批落（该处代码注释里已写明挪去 H，不留「归阶段 E」的误导说法）。
- E-2：`LimitState` 四态（涨停/跌停/未封板/**未知**）+ 纯函数 `limit_state_of`（`monitor.rs:115`）——两侧 `limit_up/limit_down` 都为 `None` 只能是「未知」，不得读成「未封板」（sina / xueqiu 那两臂的真实形态）。**偏差**：没有按 PLAN 给 `MonitorAlert` 加 `limit_state` 字段，改成把状态**内联进 `alert_message`**（`monitor.rs:504-507`）。理由：裁定 6 只要「标注不抑制」，而 payload 的 9 键契约在阶段 A 已随门 R4 冻结并同步到前端 `MonitorAlertPayload` + 11 语言；H 阶段若要按态着色，再补字段并同批改门 R4 与 TS 类型。
- E-3：新增 `alert_states`（键 `alert_id:alert_type`），`check_alerts` 只在 假→真 边沿报；首次观察即越界按裁定 3 报一次并在文案里写明「（进入监控时已越界）」；状态为**该配置求过的每一类**都更新（否则「回落 → 再越界」取不到边沿）；`remove_config_by_id` 同步回收状态键（否则 `alert_states` 只涨不落）。
- E-4：`DiskCache::get_with_expiry` 交回条目**自己的绝对到期点**，L2 命中回填 L1 走 `AStockClient::l1_expiry_from_l2`（`lib.rs:967`）——沿用原到期点，`0`（永不过期哨兵）原样交回。**分母已核对**：L1 的 `expires_at` 来自 `chrono::Utc::now().timestamp()`（`lib.rs:1028`），L2 的来自 `SystemTime…as_secs()`（`disk_cache.rs:152`）——**两侧都是 unix 秒**，所以「沿用同一个数」是正确的，不需要换算成剩余 TTL；PLAN 原文写的「按剩余 TTL 回写」与此形式等价（L1 读的本来就是绝对到期点），「剩余 ≤0 视为未命中」由 `get_with_expiry` 与 `get` 共用的那条过期判据承担（两套判据不并存）。
- E-5：`TradingSession` 枚举 + 纯函数 `trading_session_of`（`calendar.rs:299`），告警循环与行情监视器共用；边界改**左闭右开**，修掉旧 `is_trading_time` 的两种整点判法（11:30 整点算开市、15:00–15:59 整段算开市）。本项另外收掉了三处同族缺陷（都是「时段/日期判据被各写一遍」）：
  1. `next_trading_time_desc` 原来自带一套时刻比较，是第三份判据 ⇒ 改为 `current_trading_session().status_label()`；纯方法 + `every_session_bucket_has_its_own_label` 锁五档各有一句。**可见文案变化**：非交易日原会落到兜底分支，周六 16:00 会说「已收盘」（那天根本没开过盘），现在说「非交易日」。前端只透传这个串（`StockAnalysisPage.tsx:187` 取 `status`），没有任何按旧串分支的代码。
  2. `is_lunch_break` 在 1 改掉之后全仓零引用（含测试、含并行批次的工作树现场，grep 只剩它自己的声明）⇒ 退役，不留同一判据的第二张脸。
  3. `get_market_status`（`src-tauri/src/commands/stock_analysis.rs`）把 **UTC** 的 `now()` 格式化成字符串再解析回来问交易日 ⇒ 北京 00:00–07:59 问的是「昨天」，开市日凌晨会报「非交易日」；改用 `calendar::beijing_today()`。**注意**：该命令返回的三个键里前端只消费 `status`（`isTradingDay`/`isTradingTime` 零消费者），这条归 H 一起判（要么让面板真用它们，要么撤键）。
  - ⚠ watcher 加闸门前的消费路径核查（「枚举全部消费路径」）：`WatchlistPanel` 有两条取数路 —— 初始快照走命令 `get_stock_quote`（`WatchlistPanel.tsx:171`，不受闸门影响），增量走 `stock-quote-update` 事件（watcher 首次轮询 `prev=None` 时也 emit，`realtime_quote.rs:222`）。闸门只关掉增量那条 ⇒ 收盘后打开面板看到的是初始快照而非空白，无回归。
  - **撤下的判据**：PLAN 要求「节假日表过期 ⇒ 红」。不做。`HARDCODED_HOLIDAYS` 只覆盖 2025-2026 是**设计如此**（`calendar.rs` 缺陷 A 的记录：2027 起改依赖远程 365 天滚动 + K 线正向自证，硬编码表本来就是兜底），给它加一条「年份不够就红」的门等于把设计判成缺陷。表内条目的对错由既有 `is_trading_day` 那组测试 + `REMOTE_TRADING_DAYS` 自证机制管。
- E-6：**本批不切批量取数**（新增裁定 11，见 §11）。核对结论：批量返回字段**齐备**（`batch.rs` 与单只路共用 `parse_quote`，`turnover_rate`/`limit_up`/`is_st` 都在），但切过去要付三样代价 —— ① 现路是 6 vendor 逐级降级，批量口径只有腾讯一家，等于用降级链换请求数；② 要在 `MarketDataProvider` trait 上加 `batch_get_quotes` 并给 6 个 impl 各写一臂（其中三家根本没有批量接口，只能写成「逐只回退」，那是假批量）；③ as-of 模式下批量路返回 `Ok(vec[])`，回放批次会静默拿到空行情。请求数在本批已不是问题（阶段 B 已按股票聚合：56 条告警 / 16 只股票 ⇒ 每轮 16 次，不随告警行数增长）。⇒ 判为「等上游（多 vendor 批量能力）」，不为了数字好看砍降级链。

**判据与变异自证（脚本 `.workbuddy/tmp/e-mutants.py`，一次一个破坏样本，apply 前先把原文件字节存 baseline、revert 后按字节校验还原；日志 `.workbuddy/tmp/e-mutants.log` 与 `.workbuddy/tmp/e-final.log`）**

| 破坏样本 | 拆的是哪道守卫 | 实测红条 | 该轮套件读数 |
|---|---|---|---|
| M1 调用点绕过 helper，写回 `now + 3600` | `cache_get` 的 L2 分支 | **只有** `l2_hit_backfills_l1_with_the_original_deadline`（纯函数那条仍 ok ⇒ 端到端那条才是调用点的电） | `1 passed; 1 failed; 607 filtered` |
| M2 helper 把到期点 `+1` | `l1_expiry_from_l2` 本体 | `l1_backfill_keeps_a_finite_expiry_verbatim` + 端到端那条（同因连带） | `0 passed; 2 failed` |
| M3 `Some(true) => false` 改回 `true` | `check_alerts` 的边沿 retain | **只有** `alert_fires_on_crossing_not_while_staying_beyond` | `34 passed; 1 failed; 629 filtered` |
| M4 `Unknown` 塌回 `NotLimited` | `limit_state_of` 的双 `None` 臂 | `limit_state_has_four_distinct_outcomes` + `limit_state_is_annotated_without_suppressing_the_alert`（两条都读这同一个臂，同因） | `33 passed; 2 failed` |
| M5 盘前文案塌进「交易中」 | `TradingSession::status_label` 的 PreOpen 臂 | **只有** `every_session_bucket_has_its_own_label`（边界表那条仍 ok ⇒ 反塌缩断言管的是文案不是分档） | `15 passed; 1 failed; 593 filtered` |
| M6 午休右界 `13*60` 改 `13*60+1` | `trading_session_of` 的午休臂 | **只有** `trading_session_boundaries_are_half_open` | `15 passed; 1 failed; 593 filtered` |

六个样本的还原全部 `verified_identical=yes`，两轮收尾都 `OUTSTANDING=none`；`cargo fmt -p` 退出码 0。M3/M4 那两轮用的过滤器 `monitor::tests` 是子串匹配，同时把 `portfolio_monitor::tests` 也放进来跑了（35 条），所以「只红一条」的对照面比预期更宽 —— 别人的 28 条也没被我的变异波及。

**终局读数（pin 1.97，全部从 `src-tauri/` 内发起；日志 `.workbuddy/tmp/e-final.log`）**：`cargo test -p axagent-astock-data --lib` ⇒ `600 passed; 0 failed; 9 ignored`（`TEST_ADATA_EXIT=0`，含本轮新写的 `calendar::tests::trading_session_boundaries_are_half_open`、`calendar::tests::every_session_bucket_has_its_own_label`、`disk_cache::tests::get_with_expiry_returns_the_entrys_own_deadline`、`l2_readthrough_tests::*` 四条，逐条 `ok` 已在日志内核对）；`cargo test -p axagent-analysis-engine --lib` ⇒ `664 passed; 0 failed`（`TEST_AE_EXIT=0`，`monitor::tests` 七条逐条 `ok`）；`cargo clippy -p axagent-astock-data -p axagent-analysis-engine --all-targets --all-features -- -D warnings` ⇒ `CLIPPY_EXIT=0`；`cargo fmt --check -p` 两枚 crate ⇒ `FMT_EXIT=0`；主 crate `cargo check -p axagent --lib` ⇒ `MAIN_CHECK_EXIT=0`（8m08s，覆盖 `stock_analysis.rs` 的北京日修正与 `is_lunch_break` 退役在所有下游的编译面）。
- 本轮**没跑**的三样，如实记：① 全量 `cargo clippy --workspace` / `cargo test --workspace` / `cargo fmt --all --check` —— 按 §12 口径留到所有阶段收口那一次，且当前仍会被并行批次的 `backtest_validation.rs:495` 短路；② `npm run typecheck` 与 vitest —— 阶段 E 一行前端代码没改（`status_label` 的串只在后端产出，前端透传），改的是 Rust；③ 告警事件契约门 `check-monitor-event-authority.mjs` 在 E 编码后复跑过一次：自证 27/27、R1–R6 全绿（E 没动 payload 键集，这一次是确认没顺手碰坏）。
- 真机核对新增一格（归用户）：市场状态串在**非交易时段**打开应用要显示「非交易日 / 已收盘 / 距开盘」中的正确一档（本轮把第三份判据副本消掉了，周六显示「已收盘」那个假读数应已消失）。

## 6. 阶段 F —— 条件单 / 风控巡检（接或退役，见裁定 5）

现状：`start_risk_inspection_service`（`src-tauri/crates/analysis-engine/src/risk_inspection.rs:30`）零调用者 ⇒ 30 分钟过期轮与 5 分钟热加载从未跑；行情回调只转发事件不调 `evaluate_quote_against_conditions`；两套存储互不相通（内存引擎 vs `settings` 键 `conditional_orders`，**现网该键不存在**）；`conditional_order_evaluate` 恒返 `Ok(vec![])` 且注释自称「仅做框架示例」（`src-tauri/src/commands/stock_analysis.rs:1702-1714`），却已注册。

**推荐形态**（退役输入面按六层检索：声明 / 构造点 / TS 类型 / 面板控件 / i18n / 提示词）：
1. 引擎侧真接线：行情回调 → `evaluate_quote_against_conditions` → 产 `MonitorAlert` 同型告警走 A/B 那条通路（不再另造第二条通知）。
2. 持久化收口：`settings` 那套退役；条件单落独立表（走本仓正门：实体列属性 + `src-tauri/src/init/extras.rs` 声明式对账，**不写 migration**，注意启动期 `additive_only` ⇒ 退役旧形状要分「声明层 / 物理层」两句报）。
3. 已知实现缺陷同批修：`active_start`/`active_end` 必须**同时** `Some` 才生效而所有构造点传 `None/None`（`conditional_order.rs:147`）⇒ 死分支；`"HH:MM"` 字符串比较 + `Local::now()` 与 `bridge` 侧 `Utc` 的 `now_ms` 混用（`conditional_order.rs:149-152`）；除 `prev_close` 未防 0（`conditional_order.rs:168/176/184`）⇒ NaN 使所有比较为假、静默不触发；`#[serde(skip)]` 的触发计数被 `load_from_db` 归零且永不跨日滚动（`conditional_order.rs:87/91`）。
4. `ConditionType::Custom { .. } => false`（`conditional_order.rs:187`，注释「Rhai 脚本暂未接入」）⇒ 要么接 Rhai 要么在面板如实声明「自定义条件未实现」。
5. 假命令 `conditional_order_evaluate`：**退役**（命令 + 前端零调用 + `register_commands.rs` 由 build.rs 自动收敛）。

**判据**：门 F-1（纯函数 + 真落库两层，参数决定结构的批必须两层各一条）；门 F-2（反豁免形态当完成计数器：扫 `conditional_order.rs` 的 `None/None` 构造点，接线批归零才算完，门绿 ≠ 待修面已清要写进脚本注释）。

**执行状态（2026-10-15）**
- 已落（按裁定 5 推荐项「引擎 + 新表、退役 settings 那套」执行）：
  1. **新表 `conditional_orders` = 定义的唯一权威**（`crates/entities/src/conditional_orders.rs` + `entities/src/lib.rs` 的 `pub mod`）。
  2. 引擎重写为**纯判据**：`QuoteSignal` 由调用方把北京时/北京日/UTC ms 交进来（引擎不再读墙上时钟），结论改三态 `Trigger | Indeterminate | Suppressed`；时间窗**任一侧有值即生效**、缺侧补到当日边界、格式非法或终点早于起点 ⇒ 判为「无法判定」而不是「今天不生效」；前收盘/现价无效 ⇒ `Indeterminate`（旧形态除法得 NaN 让所有比较为假、静默判成「未满足」）；单日上限按北京日滚动（`note_triggered` + `trigger_date` 成对读）；换手率 ≤0/非有限视为拿不到 ⇒ `Indeterminate`（`StockQuote.turnover_rate` 是裸 f64，缺数据时是 0.0，直接 `Some(0.0)` 会把「数据源没给」读成「没到阈值」）。
  3. **价格类条件改带用户阈值**：`ConditionType::StopLoss { price }` 等四臂收绝对价。原实现根本不收用户给的数，而是拿 `prev_close × 固定 7%/15%/5%` 冒充 —— 「跌破我设的止损价」实际算的是「较前收盘跌 7%」。`MACross`（要日 K）与 `Custom`（要 Rhai）整体退役，创建口按名如实拒绝并说明未实现，不留恒假分支。
  4. 桥接层重写：读写新表；**退役「从 `stock_analyses` 的推理文本用中文关键词反读重建定义」**（那是把触发结果当成规则来源的语义倒置，且阈值/上限/冷却全被硬编码成 3%/10%/3 次/30 分）；触发计数加在引擎那一份并写回表（旧形态回写调用方 clone，引擎恒 0 ⇒ 冷却与上限从未生效）；列表不再 `format!("{:?}")` 漏 Rust Debug。
  5. 命令面：`commands/conditional_order.rs` 五条真命令（create / list / set_enabled / delete / manual_evaluate），`create` 落库（旧 `create_conditional_order` 形参 `_state` 未用 ⇒ 只返回 id 的 no-op，任何一次 list 都被 `load_from_db` 清空）；DTO 拆 `conditionValue` / `actionValue`（旧形态一个字段被换手率阈值与买入股数争用）；`disable` 不再改 `stock_analyses.status`（与分析查询争用同一列）；`manual_evaluate` 去掉「周末 + 手写时段 + |涨跌|<0.5%」三重门，返回结构化报告（`codesRequested` / `codesQuoteFailed` / `triggered` / `triggeredToday` / `session`），默认评估对象改成**有启用条件单的股票**（旧写法取 `portfolio_holdings`，现网 0 行 ⇒ 恒返 0）。
  6. 退役：`stock_analysis.rs` 三条 settings 命令（含恒返空的 `conditional_order_evaluate`）＋ `risk_inspection.rs` 整文件 ＋ `settings` 键 `conditional_orders`（现网 0 行，零迁移）。
  7. 接线：`init/services.rs` 启动时 `refresh` 装载定义；行情回调在同一条路里评估 → 触发则计数落表 + 非「仅通知」生成交易意图 + 发 `conditional-order-triggered`。事件名进 `IpcEventName` 权威枚举（三处 + 生成物重取）。前端有真实监听者（`PriceAlertPanel` 一条 toast + 组件测试 5 条），不留没人听的事件。
  8. **F-8 条件单界面（用户拍板裁定 13 = ①「要界面」后补做）**：新面板 `src/components/stock-analysis/ConditionalOrderPanel.tsx`，挂在 `WatchlistPage` 底部全宽一行，与价格预警并列不合表（两个实体）。含建单表单（六类条件下拉、阈值按类别换单位、四类动作、冷却/单日上限/生效时段）、列表（代码/名称/后端条件句/动作句/今日触发/最后触发/启用开关/删除）、启停与删除。四态在界面上各自成句：**读失败**（红条带原因，且不同时显示「暂无」）/ **表里真没有**（空态）/ **有行但读不回为规则**（黄条带计数与逐行原因）/ **正常列表**；标题栏两个数各用自己的文案 —— 「在评估 N 条」（启用且可读回）与「表内共 M 条（含已停用）」，不并排成可比的两个数也不收敛成一个数。建单参数按 camelCase 整包 `req` 传（Tauri 参数校验在 IPC 层就拒 snake_case）。
- 与 PLAN 的六处偏差（都记在此，不是漏做）：
  1. **PLAN 写的正门路径不存在**：`src-tauri/src/init/extras.rs` → 实际在 `crates/dao/src/reconcile/extras.rs`，且**平表不需要在那里声明**（`price_alerts` 在整个 reconcile 目录零命中）。新表只需实体文件 + `pub mod`，`build.rs` 生成的 `entity_registry!` 自动收录，启动期声明式对账建表。
  2. **上面那条「门 F-2 扫 `None/None` 构造点」作废**：时间窗改成任一侧有值即生效之后，`None/None` 是合法的「全天」，拿它当计数器会把正确写法判成待修面。换成 R10「**退役生产者计数为 0**」——同一反豁免思路，计数对象换成被退役的六只手（反读重建函数、settings 键读写、意图行当定义、`risk_inspection`、假命令）。替换理由写进了脚本注释，六类各有回潮负控。
  3. PLAN 只点名 3 条命令，实际有**两个并行命令族**（`commands/conditional_order.rs` 4 条 + `stock_analysis.rs` 3 条）。前者改造为真 CRUD，后者整体退役。
  4. **门 F-1 的落库层写成「一条串行端到端」而不是多条**：桥接层的引擎是进程级 `static`，两个 `#[tokio::test]` 并发时会互相 `refresh()` 覆盖规则集 ⇒ 断言变随机。拆成独立测试需要先把引擎挂进 `AppState`（形态同 `stock_monitor`），本批未做，已记在此。
  5. PLAN 未提的一处真缺陷：**价格类条件根本不收用户阈值**（见上第 3 点），不改这一条则「接线」接出来仍是假规则。
  6. `risk_inspection` 退役带来一个副作用：pending 交易意图的**自动**过期随之下线（30 分钟那条循环是它唯一的自动入口），能力本身仍在命令 `expire_old_trade_intents`（agent 可调）。要不要把自动轮接回 ⇒ 新增裁定 14。
- 新增待裁定 13：**条件单要不要 UI 入口** —— 普查实测：`src/` 下 `invoke` 调用 0 处、store 0、TS 类型 0（本批新增的是 payload 类型）、i18n 键 0（本批为触发提示补了 11 语言各 1 键）。也就是说建单目前只能走 agent 工具。选项：① 本 PLAN 内加面板（建单表单 + 列表 + 启停，≈ 一个独立批次）；② 归阶段 D 与「假旋钮收口」一并接（**推荐**，D 正在处理的就是「控件有、没人读」）；③ 保持 agent-only 并在面板如实写明「条件单由 AI 助手登记」。
- 新增待裁定 14：**pending 交易意图的自动过期**要不要接回 cron（随 `risk_inspection` 退役下线；推荐归阶段 G 的 cron 批统一挂，不在 F 里另造第二条轮询）。
  → **2026-10-15 用户拍板：先处理 14 再进 G ⇒ 取 ①**：接回自动过期，落点在 **G-3**（与组合指标日更同一条 cron 批，不另造第二条轮询）。要挂的任务：`expire_old_intents(72h)` —— 现役只有命令 `expire_old_trade_intents` 能触发它，自动入口随 `risk_inspection.rs` 退役消失。
- 判据与读数（pin 1.97，cargo 全部从 `src-tauri/` 内发起；日志 `.workbuddy/tmp/f-round3.log` / `f-round4.log` / `f-final.log` / `f-mf2fix.log`）：
  - 跨层契约门 `check-monitor-event-authority.mjs` 从六条扩到**十条**（R7 条件单发射点用权威枚举且 Rust 侧无字面量 / R8 两头同名且已进生成物 / R9 payload 九键逐项相等 / R10 六类退役生产者计数为 0），自证 **45/45**；对真源文跑出 **R1–R10 全绿**。过程中这条门自己被抓了一次：R7 初版把权威文件里 `IpcEventName::ConditionalOrderTriggered => "conditional-order-triggered"` 那行当成「抄第二份字面量」⇒ **假红**，改为按「哪个文件持有这条映射」现场推导豁免面（不写死文件名，文件搬迁不该让门失效），并补两条对照用例（权威文件内那行 ⇒ 绿；别处再抄 ⇒ 红）。
  - 代码生成这一步是被门逼出来的：R8 在 `schema-gen` 重跑之前**确实红**（生成物里没有该事件名），跑完才绿 ⇒ 这条判据对「生成物腐烂」有电。
  - `check-contracts.mjs`：项 H 棘轮 **261 → 259**（退役的两处 `let _ = load_from_db(..)`：`risk_inspection.rs:62` 的 5 分钟热加载、`commands/conditional_order.rs:112` 列表命令里的同名丢弃，两处都是「装载失败仍返回成功」）；`errors=0 warnings=0`。
  - `cargo test -p axagent-analysis-engine --lib` ⇒ **671 passed / 0 failed**（含本批新写的 8 条引擎判据 + 2 条桥接层判据）；`cargo clippy -p axagent-entities -p axagent-analysis-engine --all-targets --all-features -- -D warnings` ⇒ 0；`cargo fmt -p` 两枚 crate ⇒ 0；主 crate `cargo check -p axagent --lib` ⇒ 0，且 `register_commands.rs` 由 build.rs 自动收敛成 5 条新命令（三条 settings 命令与 `disable_conditional_order` 已从注册表消失）。
  - 前端：`npm run typecheck` ⇒ 0 错；`check-i18n-key-parity.mjs` ⇒ 11 个语言文件键路径集合一致（新增 `stockAnalysis.conditional.triggeredDetail` 各 1 键，逐语言回读断言三个插值槽齐）；`check-line-endings` ⇒ PASS。
  - 破坏样本六个（`.workbuddy/tmp/f-mutants.py`，一次一个、apply 前存字节 baseline、revert 后按字节校验，收尾 `OUTSTANDING=none`）：MF1 换手率拿不到塌成「未满足」⇒ 只红三态那条；MF3 单日计数不看归属日 ⇒ 只红滚动那条；MF4 `note_triggered` 的跨日清零失效 ⇒ 红纯函数那条 + 落库端到端那条（同因）；MF5 触发不写回引擎 ⇒ 只红落库端到端；MF6 换手率 0.0 当 0% ⇒ 只红映射那条。**MF2 是绿的，而它该红**：样本把「只给起点时右界补到 24:00」收成「补到起点自身」，测试却只探了 09:00（窗外）与 09:30（恰是起点那一分钟）⇒ 探不到这一格。按「破坏样本打不绿的测试就是缺格的测试」处理：补 11:00 与 23:00 两点必须放行、以及「只给终点时凌晨也放行」，按「破坏样本打不绿的测试就是缺格的测试」处理：补 11:00 与 23:00 两点必须放行、以及「只给终点时凌晨也放行」，再复跑 MF2 取红。
  - 补强后的复跑（`.workbuddy/tmp/f-mf2fix.log`）：基线仍 **671 passed / 0 failed**，MF2 复跑 ⇒ `window_accepts_one_sided_and_absent_bounds` **FAILED**（`10 passed; 1 failed; 660 filtered`，退出码 101），还原 `verified_identical=yes`、`OUTSTANDING=none`，clippy 仍 0。⇒ 六个样本现在全部会红该红的那一条。
  - 前端判据：`npx vitest run` 两个面板事件测试 ⇒ `PriceAlertPanel.alertEvent.test.tsx` 3/3 + `PriceAlertPanel.conditionTrigger.test.tsx` 5/5（**8 passed**；后者含一条反证：价格告警收到 payload 时**必须**重取列表，用来证明「条件单不重取」那条不是恒真）；`npx dprint check` 对本批改过的 5 个 ts/mjs/json 文件 ⇒ 无差异。
  - F-8 判据与读数：`ConditionalOrderPanel.test.tsx` **10 条全绿**（列表来自命令而非前端拼串 / 读失败不显示「暂无」/ 真空态才显示「暂无」/ 读不回的单独成句带原因与计数 / 两个计数域各用自己的键且实测为 1 与 2 / 建单参数 camelCase 包 `req` 且阈值为数值 / 停用落 `enabled` 列且参数名 `orderId` / 删除点击当刻不落刀 / 阈值单位随类别换 / 认识之外的类别落「未知」）；三枚面板事件测试合跑 **18 passed**（本批 10 + 阶段 A 的 3 + 阶段 F 的 5，证明给面板加第二个监听器没把告警那条碰坏）。i18n 新增 26 键 × 11 语言，脚本三类自证：槽位齐备、`{{`/`}}` 配对、**非汉字语言混入 CJK** —— 正是最后这条抓出三处真错（hi「प्रतिदिन上限」、ko「감仓」、ko「{{count}} 件」），混写不会被 parity 门或缺译门发现；逐文件与落盘前备份比对 delta=28 行；`check-i18n-key-parity` 11 文件键集合一致；`check-line-endings` PASS。`npm run typecheck` 0（顺带修掉 antd 6 两处 API 误用：`Alert` 无 `size` 属性、`message` 已废弃 ⇒ 改 `title`）；`oxlint src --max-warnings=0` ⇒ 0 warnings 0 errors；`dprint check` 对本批 5 个 ts/tsx 无差异；`check-contracts` errors=0 warnings=0（项 E 认到 `ConditionalOrderListView` / `ConditionalOrderItem` / `UnreadableConditionalOrder` 三个 DTO 同名导出）；门 R1–R10 仍全绿。F-8 只动前端，Rust 侧读数以 §6 上面那批为准。全量前端套件 ⇒ `2417 passed | 1 failed`，唯一红的是 `ScreenerToWorkspace.integration.test.tsx`（该用例真连 PG，日志带 `deadlock detected` + `STOCK_WORKFLOW_PERSIST_FAILED`），单跑同一文件 `5 passed` ⇒ 判为环境偶发（并行会话撞库），已记进「门禁已知 flaky 测试」第 29 条，不动断言。`npm run check:facts` ⇒ LOCATED 530 / NONLOC 0 / BROKEN 0（本批删掉的 64 + 105 行没有让任何按 `文件:行` 写的引用失准）；`check:i18n:placeholder` ⇒ 占位符使用干净。
  - 还没跑的：全量三条（按 §12 留到全部阶段收口）；`cargo test -p axagent --lib`（主 crate 那 619 条，本批只改了 `init/services.rs` 与两个 commands 文件，且 `cargo check -p axagent --lib` 已 0 —— 落库端到端走的是 `create_test_pool` 的临时 SQLite，不碰现网 PG）；真机核对归用户，且**必须重启应用**才建得出 `conditional_orders`（本仓正门是启动期声明式对账）。

## 7. 阶段 G —— 组合级监控

| 子项 | 现状锚点 | 动作 |
|---|---|---|
| G-1 T+0 自动重跑 | `TZeroConfig.enabled` 默认 false（`monitor.rs:40`）；命令已注册、store 有 `fetchT0Config`/`setT0Config`（`src/stores/feature/stockAnalysisStore.ts:2248/2272`）**但零组件调用** | 见裁定 4（接面板 vs 退役这条腿）—— 接的话要按「新增可调参数五点对账」+ 11 语言 i18n + 成本声明 |
| G-2 组合级信号 | `portfolio-signal`（`monitor.rs:523`）不在 `IpcEventName`；`subscribe_portfolio_signals`（`monitor.rs:169`）零调用者；聚合器已注入（`services.rs:3793-3800`）但产出没人收 | 事件名进权威表 + 生成 `events.ts` + 前端订阅并落到「监控」视图一条呈现；`subscribe_portfolio_signals` 若无消费者则**删除**而非留空 |
| G-3 组合指标日更 | 内置 8 条定时任务无一条是组合指标（`src-tauri/src/init/stock_automation_cron_seed.rs:90-202`）；实体注释却写「每日 EOD 写入一行 … 手动或 cron 触发」（`src-tauri/crates/entities/src/portfolio_metrics_daily.rs:4-7`） | 加第 9 条任务（收盘后、走交易日闸门）；**与并行批次同函数真重叠 ⇒ 只改不提交**；注释按现况改口径 |
| G-4 恒零两列 | `cash_pct`/`max_drawdown_pct` 写死 0.0（`src-tauri/crates/analysis-engine/src/portfolio_monitor.rs:598-599`，注释称「由 refresh_metrics 补算」，而 `refresh_metrics` 在 `:678/:681` 原样写回 0）；`compute_max_drawdown_pct`（`:121`）生产零调用 | 接进 refresh 真算；`cash_pct` 需要现金来源 —— **现网 `portfolio_holdings` 0 行，先定数据来源再接**（裁定 8） |
| G-5 读取路径空壳 | `portfolio_monitor.rs:743` 集中度阈值传 0 ⇒ `n<3` 分支永不命中；`:744-746` 硬编码 `risk_level:"—"`、`diversification_score:0`、`positions: vec![]`；`:566` `_limits` 收下不用 ⇒ 面板无视仓位限制 | 要么补算要么把该行从面板撤下（撤展示位比留一个恒零数更符合「不得在 UI 造成歧义」） |
| G-6 缺价当 0 / 时区 / 复权 | `market_value.unwrap_or(0.0)`（`:172/:264/:578`）参与所有分母；`Utc::now()` 盖北京交易日戳（`:651/:847`）⇒ 早 8 点前落到前一天；相关性用不复权 K 线（`:829`）而 β 用前复权（`:482`），且按尾部位置对齐（`:856-857`）—— 正是它自己在 `:401-403` 点出的缺陷 | 缺价 ⇒ 显式「无价」并从分母剔除（不折算 0）；统一走 E-5 的北京会话 helper；相关性改按日期对齐 + 与前复权同口径 |

**判据**：门 G-1（真库：跑一次刷新 ⇒ `portfolio_metrics_daily` 出现 1 行，且 `cash_pct`/`max_drawdown_pct` **非零**（现网基线 0 行 0 值）；夹具时间戳要落在生产查询窗内）；门 G-2（事件名权威表覆盖 `portfolio-signal`，且前端有对应 `listen` 调用点）；门 G-3（cron 清单现取含新任务 id，且断言条数下限，不手抄数字）。

## 7.1 阶段 G 的执行状态（2026-10-15 开工，G-1/G-2/G-3/G-5 已落地）

**开工前重取的现网读数**：`portfolio_holdings` 0 行 / `portfolio_metrics_daily` 0 行 / `trades` 0 行，`price_alerts` 仍 56 行。⇒ 「补算出来的数对不对」在本机无法用真仓验证，只能靠带夹具的 Rust 测试；面板真机核对需要用户先建持仓。

| 子项 | 做了什么 | 现场锚点 |
|---|---|---|
| G-1 | 缺价不再当 0 参与**两条真有缺陷的**口径：① 盈亏百分比的分母改成「与分子同源」（只累加拿到 `unrealized_pnl` 的那几只的成本）；② 风险等级 / 分散度收的 `n` 从「表里行数」改成「已定价只数」；缺价只数新增 `PortfolioDashboard.unpriced_positions` 并进面板独占一句 | `compute_dashboard` / `count_unpriced` / `PortfolioMonitorPanel.tsx` |
| G-1（订正一处我写错的判据依据） | `market_value.unwrap_or(0.0)` 参与**市值分母**这一处**原先不是缺陷**：往分母加 0 不改变任何一只的占比。集中度函数改成显式 `filter_map` 只是让口径由构造说明，数值不变 ⇒ 文档注释按此改写，不再声称修掉了「假集中」 | `compute_concentration` 头注 |
| G-2 | 两处快照日期从 `Utc::now()` 改 `calendar::beijing_today()`（北京早上 8 点前不再落到前一天）；相关性从「不复权收盘价水平 + 取较短者尾部」改成「**前复权日收益 + 按日期交集对齐**」，新增纯函数 `aligned_pearson` 与 `beta_from_aligned` 同一条对齐规则；`eprintln!` 改 `tracing::warn!`；写库失败不再被 `is_ok()` 吞 | `refresh_metrics` / `refresh_correlation` / `aligned_pearson` |
| G-3 | `max_drawdown_pct` 接进 `refresh_metrics` 真算（`drawdown_from_history`：90 日回看窗 + 剔除 ≤0 的历史行 + 严格早于本日 + 不足两点给 0）；`compute_max_drawdown_pct` 的参数名由 `equity_curve_pct` 改 `equity_curve`（它收的是水平值序列，旧名会诱导调用方先除以 100） | `portfolio_monitor.rs` 持久化层 |
| G-5 | 读取路径不再交常量：`portfolio_metrics_daily` 新增两个可空列 `position_count` / `unpriced_count`（声明式对账自动补列），`get_dashboard` 用「已存集中度 + 行业暴露 + 落库只数」复原 `risk_level` / `diversification_score` / `concentration_warning`，原先硬编码的「阈值传 0 ⇒ `n<3` 那一档永不命中」一并消掉；老行 `position_count IS NULL` ⇒ 保持 `"—"` 与 0，不硬造数 | `entities/src/portfolio_metrics_daily.rs` / `get_dashboard` |
| G-4 | 实体注释按现况改口：`cash_pct` 写明「无来源、写入侧恒 0」，`max_drawdown_pct` 写明口径；三处 `(0-1)` 的单位注释订正为百分数（代码一直给 0-100） | 同上 |
| `_limits` | **退役参数**而不是补一行判据：`compute_dashboard` / `refresh_metrics` 不再收 `PositionLimits`（面板本来就没有「仓位限制」这一行，传进来从不读；三条上限的现役消费者在风控门与分析命令里，与这条链无关）。唯一调用点 `stock_analysis.rs` 同步改 | `compute_dashboard` 上方注释 |

**判据与读数（pin 1.97、`src-tauri/` 内发起）**
- `cargo test -p axagent-analysis-engine --lib portfolio_monitor::` ⇒ **34 passed / 0 failed**（含本轮新增 6 条）；`cargo check -p axagent --lib` **EXIT=0**；R11/R12/R13 三条门均绿（R13 新加：`PortfolioDashboard` 两侧 19 键逐项相等）。
- 变异自证（`.workbuddy/tmp/mut-g1.py`，每次只改一处 + sha256 回写核对）：M1 盈亏分母退回全成本 ⇒ 只红 `unpriced_position_excluded_from_pnl_denominator`；M2 只数退回行数 ⇒ 只红 `diversification_uses_priced_count_not_row_count`；M3 相关退回按位置取 ⇒ 只红 `aligned_pearson_pairs_by_date_not_by_tail_position`；M6 刷新不补算回撤 ⇒ 只红 `refresh_metrics_persists_nonzero_drawdown_and_position_counts`；基线 34/34。
- ⚠ **两条回撤判据的第一版没有电**（这是本轮最值得记的一次自证失败）：我把「市值为 0 的历史行」放在序列**开头**并断言结果为 0.0 ⇒ 「去掉 ≤0 过滤」与「`lt` 退成 `lte`」两个变异体都照样绿 —— 0 在低点不构成回撤，递增序列本来就没有回撤。改成把干扰行放在**峰与谷之间**（12000 → 0 → 11000，本次 8800）后，不过滤就读成 100%、把今天那行算进来就读成 75%，两个变异体才各自红。复跑读数记在下一条。
- 前端：`PortfolioMonitorPanel` 缺价句 4 条组件测试绿（含「没有缺价时不得常驻一句 0」）；`tsc` 只剩 `portfolio-signal` 一条未生成的事件名（G-6 进行中，非回归）；`dprint` / `oxlint` 干净。
- 面板真机核对：需要用户先建 `portfolio_holdings`（现网 0 行），否则刷新出来的组合快照本身就该是空。

### 7.1.1 G-cron 段（2026-10-15 收尾批）

**两条新内置任务**（`init/stock_automation_cron_seed.rs` 第 9/10 条，共 10 条）：

| 任务 | id（幂等键） | task_type | 时刻 | 闸门 | 产物 |
|---|---|---|---|---|---|
| 组合指标日更 | `portfolio-metrics-daily` | `portfolio-metrics` | `0 16 * * *`（收盘后、18:00 反思前） | **进**交易日闸门（产物就是当日收盘净值行，休市日跑=重复样本塞进 90 日回撤窗） | `portfolio_metrics_daily` + `portfolio_correlation_snapshot` |
| 交易意图过期 | `intent-expiry-daily` | `intent-expiry` | `30 7 * * *`（07:00 回测后、09:30 开盘前） | **不进**闸门（只改状态列，不读当日行情） | `stock_analyses.trade_intent_status` = expired（超过 72h 未审核） |

- **G-7 · 单一通路**：三步链（β → 指标落库 → 相关性落库）原先只写在 `refresh_portfolio_metrics` 命令体里 ⇒ 抽成引擎层 `portfolio_monitor::refresh_snapshot(db, client, positions, as_of)`，命令面与 cron 面共用；`refresh_portfolio_for_scheduler`（命令模块）只做「读持仓 → 空仓返 `None`」。空仓**不落全 0 行**，记 `skipped`（0 行会进回撤窗、也会让面板显示 0 市值）。
- **G-8 · 顺手检出的第五个假旋钮**：面板键 `risk_correlation_lookback_days`（种子 60 / 面板 60）此前**零读取者** —— 刷新链吃的是调用方写的裸 `60`。现接进引擎 `correlation_lookback_days_in()`（纯函数判区间 20..=250，落点回落值 = 接线前现值 60 ⇒ **默认逐位不变**），并**按该门自己的口径**把它加进 `check-panel-var-landing.mjs` 的 `LANDED` 清单（13 → 14 条，面积自证同步）。落点处同时把 `60` 提成 `CORRELATION_LOOKBACK_DAYS`（原来那个裸字面量在 `stock_analysis.rs`，两个生产者谁都不知道对方写了几）。
- **G-9 · `positionsSnapshotted` 名实不符**：`refresh_metrics` 第二条恒返 `1`（= 写了几行），而命令面/前端把它叫 `positionsSnapshotted` ⇒ 8 只持仓会显示成 1。改成返**覆盖的持仓只数**，命令面 JSON 键不变（`asOfDate` 语义也不变，仍是 `Option<&str>`→`null`）。
- **`task_type` 三处清单对齐（查出第三处漏登记）**：`lesson-validation` 既不在前端 `INVEST_TASK_TYPES`（任务中心归不进「投资」组、删除确认缺警示）也不在后端 `scheduled_task.rs::BUILTIN_TASK_TYPES`（「立即执行」报 `MANUAL_RUN_NOT_SUPPORTED`，而到点自动执行照常 ⇒ 只有手动触发才露头）。本批把三条（含新增两条）补齐三处，并加**齐备性判据 ②**：`build_builtin_jobs()` 的真实 `task_type` 必须逐个落在后端白名单里（拿真值对表，不解析文本）。
- **P4 面积门里降了一格 min（有据）**：`check-panel-var-landing.mjs` 对 `stock_analysis.rs` 的 `PositionLimits::panel_effective()` 要求由 3 改 2 —— 第三处是阶段 G-5 撤掉的**死参数透传**（`compute_dashboard` 从不读 limits），留下的两处才是真消费者（风控门 + `get_portfolio_position_limits` 命令）。
- **为什么这两条任务不进交易日闸门/进闸门**：写进了两条分支的注释，同时闸门 `matches!` 列表加了 `PORTFOLIO_METRICS_TASK_TYPE`（常量引用，与 `TREND_SCREENING_TASK_TYPE` 同形）；`intent-expiry` 刻意不进。

**判据与读数（本批新增 4 条）**
- `every_builtin_job_is_manually_executable`（Rust，真值对表 + 负控）；`correlation_lookback_window_adopts_panel_value_only_within_range`（纯函数四格：区间内/过小/过长/非数值/键缺席）；`refresh_snapshot_runs_beta_metrics_and_correlation_in_one_pass`（真库：返回值 + **产物表真有几行** 两层）；seed 侧 `seed_creates_all_builtin_jobs_and_is_idempotent`（8→10）+ `job_configs_roundtrip_through_executor_parsers` 补两条时刻/空 prompt/时长常量对齐断言。
- 门 `check-panel-var-landing.mjs`：G-8 那条**靠一次性取证脚本**验证（`.workbuddy/tmp/verify-g8-landing.mjs`，6/6）：正控绿 + 自证 needle 计数 = 2 + 三个负控（调用点退回裸 60 ⇒ 只红 P2「只有 1 处」；回落值 60→45 ⇒ 只红 P1「落点回落 45」；键改名 ⇒ 只红 P2「键没出现在落点」）+ P4（清单删一条 ⇒ 报「应为 14 条」）。
  ⚠ **为什么不能直接 `--selftest`**：`portfolio-mgr-h-long.rhai` 被**另一个批次**的未提交改动（`PLAN-stock-decision-reliability` P0-4「估值门语义反转 2026-10-16」）摘掉了 `pm_required_margin_pct()` 调用 ⇒ 门正控第 2 条（跨载体）当场红、`selftest()` 首步 bail。取证脚本把那份 rhai **只在内存里**换回 HEAD 版（不落盘、不碰他人文件）后，负控才验得到。
- 前端：`taskGrouping.test.ts` **89/89**（TasksCenterPage 全目录 8 文件）；新增两行 `canExecuteNow` 断言在补白名单**之前先跑成红**（坏样本自证），补上后转绿。
- 未取到的读数（**归批次收口，见 §12 阻断**）：engine 的 `cargo test --lib`、主 crate `check`/`clippy`、全量三条 —— 被并行批次的编译错误挡在 `axagent-astock-data`。

### 7.1.2 G-1 / 裁定 4（T+0 自动重跑接面板）——2026-10-15 落地

**取证结论**：这条链路**后端全通** —— 引擎 `maybe_trigger_t0` 按 `TZeroConfig`（阈值 + 最小间隔去重）判，
`services.rs` 注入了后端重跑 callback（P1-1），前端 store 里挂着 `stock-monitor-t0-rerun-requested`
的事件 listener，事件名早已在权威表与生成物里。**缺的只有一处 UI 入口**：`fetchT0Config` /
`setT0Config` 零组件调用者 ⇒ 用户看见的是「功能不存在」。

| 做了什么 | 落点 |
|---|---|
| 价格告警面板加一条 T+0 设置行：开关（**默认关**，与引擎 `TZeroConfig::default().enabled=false` 同口径）+ 开启后展开三个阈值/间隔输入 | `PriceAlertPanel.tsx` |
| **成本句上屏常显**（不塞 tooltip）：一次触发 = 重跑整条分析工作流（成本≈一次完整分析）；同一标的按最小间隔去重 | 同文件 + 6 键 × 11 语言 |
| 保存失败**回滚乐观态并上抛**：store 的 `setT0Config` 此前只 `console.error`（面板会显示「已开启」而后端没收到） | `stockAnalysisStore.ts` |

**判据与读数**：组件测试 4 条（默认关 + 成本句上屏；开启载荷是合并后的完整配置；保存失败回滚 + toast；改阈值只发改动那格）；
变异自证 4/4（`.workbuddy/tmp/mut-t0-panel.py`：M1 不回滚⇒只红「保存失败」、M2 撤成本句⇒只红「默认关」、
M3 阈值不按 enabled 门控⇒只红「默认关」、M4 只发增量⇒红两条载荷断言且不碰另外两条）。

## 8. 阶段 D —— 假旋钮收口（核心是「接通通路」而不是换个硬编码值）

现状：`monitor_alert_cooldown_secs` / `monitor_min_severity` / `monitor_channels` 只有播种（`src-tauri/src/commands/stock_analysis_setup/seed_variables.rs:635/642/649`）和面板展示（`src/components/settings/StockAnalysisConfigPanel.tsx:90-92`），Rust 侧零读取者；既有的落地门 `scripts/check-panel-var-landing.mjs` 清单里**不含这三条**（只有 `monitor_poll_interval_secs`，`scripts/check-panel-var-landing.mjs:154`）⇒ 门不会红。热更入口 `set_alert_cooldown_secs` / `set_poll_interval_secs`（`monitor.rs:262/272`）零调用者，且轮询间隔在循环前一次性取走（`monitor.rs:212`），`src-tauri/src/init/services.rs:3909` 明写「告警冷却本批刻意不接」。

**改法**
1. **扩既有门优于造新门**：把三条键加进 `check-panel-var-landing.mjs` 的 `LANDED` 清单（13 条 → 16 条），按该门自己的口径配 `calls` 最小处数 + 负控，并同步 P4 面积自证的条数（由 `cases.length` 现算，不手抄）。
   ⚠ **动 D 之前先读这条 2026-10-14 的实测更正**：面积门 `check-variable-consumer-registry.mjs` 现网读数是「声明 352 ｜ 零引用面 0（已登记 0）｜ 其中面板仍渲染 0 条」——**它并没有把这三条报成空接线**，因为它把 `src-tauri/src/commands/stock_analysis_setup/mod.rs` 里那张 camelCase→snake 键名映射表的命中也算作了引用面。所以 D 的依据不是「没有门」，而是「门把映射行算成了消费者」；扩落地门时**同时要给面积门补一条口径**：命中面要排除纯键名映射表，否则新接的键仍会被它判成「已引用」。
2. 面板保存 ⇒ 变量生效的**通路**：在股票配置保存命令里回调 `state.stock_monitor` 的 setter；冷却与轮询都要即时生效。
3. `monitor.rs:212` 的取间隔挪进循环（每轮重读，或改 `tokio::time::sleep` + 主动重建 `Interval`）—— 否则 setter 接了也不生效。
4. `monitor_min_severity` / `monitor_channels` 归通知腿（M-NOTIF）消费 —— 在装配前，**面板这两格必须显式声明「当前无消费者」**，不能继续显示成可调（裁定 7）。
5. ~~顺手修注释腐烂：`services.rs` 那句「`start_with_config` 全仓零调用者」已过期~~ —— **已在阶段 A 顺手订正**（该行现为「正门是 `start_with_config`，本段末尾就是它的唯一调用点」）。

**判据**：门 D-1 = 上面那条扩清单后的落地门；门 D-2（Rust 测试）= 调 setter 后 `monitor` 下一轮实际用新值（破坏样本：把间隔读取留在循环外必须红 —— 这一条就是本轮缺陷的形态锁）。

### 8.1 阶段 D 的执行状态（2026-10-15 落地）

**接了两条、如实标注了三格、拒接一条**：

| 对象 | 处置 | 落点/依据 |
|---|---|---|
| `monitor_poll_interval_secs` | **接通**：间隔改**每轮重读**（ticker 变化时用 `interval_at` 重建，避免新 ticker 的首个 tick 把等待吃掉），`.max(1)` 守卫（0 会让 `tokio::time::interval` panic） | `monitor.rs` 的 `start()` |
| `monitor_alert_cooldown_secs` | **接通**：消费点（每轮求值时读）早已活，缺的是「面板值 ⇒ 引擎」这一环 | 同上的回灌方法 |
| 保存设置 ⇒ 生效 | **同一个回灌方法**同时服务启动装配与保存通路（`update_workflow_template` 的 `id == "stock-analysis"` 分支）；两处各写一份「读值+校验+覆盖」必然分叉（A 批就分过一次：间隔接了、冷却没接，校验范围还写在装配点里） | `monitor.rs::apply_panel_runtime_vars` / `services.rs` / `workflow_template.rs` |
| `keylevel_lookback_days`（裁定 18） | **拒接**（详见 §11 裁定 19：它的消费者**不存在** —— 检测器没实现、`store_key_levels` 零调用者、回测那个 `lookback_days` 参数是 `_lookback_days`），改为在面板描述里如实标注 | 11 语言描述后缀 |
| `monitor_min_severity` / `monitor_channels` | 归 M-NOTIF（裁定 7）：在面板描述里如实标注「当前无消费者」，不摆成可调 | 同上 |

**判据与读数**
- 落地门 `check-panel-var-landing.mjs` 扩到 **15 条**（+`monitor_alert_cooldown_secs`），两条监控键的 `calls` 各带「引擎回灌方法 + 启动装配 + 保存通路」三个 needle；`monitor_poll_interval_secs` 的键读取点从装配点**挪到引擎**（两处共用同一方法后，键名不再在 wiring 层出现第二次）。
- 新门 **R17**（并进 `check-monitor-event-authority.mjs`）：`start()` 的 `loop {` 之后必须仍能读到 `poll_interval_secs` —— 这条锁的是**形态**（落地门数调用处数看不出「读一次就定型」）。自证 71/71（含两种合法写法的正控 + 原形态负控 + 锚点失效负控）。
- Rust 单测 `apply_runtime_vars_adopts_valid_and_rejects_invalid`（纯函数版）：合法采纳 / 间隔 0 拒绝 / 越上界拒绝 + 边界 3600 采纳 / 冷却为负拒绝 + 0 合法 / 键缺席一律不动，五格各一条。走纯函数版是因为快照是进程内全局（并行用例互踩）。
- 取证脚本 `.workbuddy/tmp/verify-g8-landing.mjs` 扩到 **7/7**（新增两条 D 批负控：撤保存通路 ⇒ 两条监控键同时报「没接」；引擎回灌方法改名 ⇒ 报「写了没被调用」）。

## 9. 阶段 H —— 呈现层与类型对齐

1. `PriceAlertPanel.tsx` 三处 `catch { /* 静默 */ }`（`:35/:90/:97`）⇒ 建/删失败用户无感。按 AGENTS.md 后端错误码规范改成解析 `code` + `t("error.${code}", params)`，**并补齐 11 种语言**（zh-CN 为源，`error.MONITOR_*` 若需新码则同步 `commands/error_code.rs`）。
2. 阈值单位分列：一格 `target_price` 同时装价格、涨跌幅 % 和换手率 %（`stock_analysis.rs:2236` 直接把 `target_price` 当 `threshold`），列表统一按价格渲染 ⇒ 涨幅告警的 5 显示成「5.00」。按 `condition_type`（`price`/`change_pct`/`turnover_rate`，`src-tauri/crates/analysis-engine/src/alert_mapping.rs:32-36`）渲染单位。
3. `src/components/stock-analysis/PriceAlertPanel.tsx:14` 的 `isTriggered: boolean` 对后端 i32（`src-tauri/crates/entities/src/price_alerts.rs:32`）⇒ 类型改 `0 | 1` 或 `number`，并检查所有真值判断。
4. 缺席三态上屏：本轮取数失败数（E-1 产出）、监控引擎未装配（`services.rs` 注入失败路径）、非交易时段停摆（E-5 产出）—— 三种缺席各占一句，不得并排成可比的两个计数。
5. 两套组合面板（`PortfolioDashboard` vs `PortfolioMonitorPanel`）与 `MonitorView.tsx:14`「阶段 4 会整合为统一 dashboard」⇒ 要么整合要么撤注释；整合属结构调整，单独提子项 H-5 等拍板（裁定 9）。
6. `get_market_status`（`src-tauri/src/commands/stock_analysis.rs`）返回三键，前端只消费 `status`（`StockAnalysisPage.tsx:187`），`isTradingDay` / `isTradingTime` **零消费者** ⇒ 要么让面板真用它们，要么按退役六层撤键。同一件事的两套表达（bool + 中文分档串）并存在一个返回体里，正是 H-4 要消掉的那类歧义。（E-5 已把它的日期基准从 UTC 改回北京日，见 §5。）
7. 复盘报告的 `market_status`（`src-tauri/crates/analysis-engine/src/review.rs:101-109`）仍是「交易日 + `is_trading_time`」的二值判法 ⇒ 午休时段跑复盘会把当天写成「已收盘」。改法与 `next_trading_time_desc` 完全相同（接 `TradingSession::status_label`），**但它写进的是复盘文本**（新旧行会文案不一致）⇒ 待拍板，并与 H-4 的三态一并看。
8. 老 `condition` 列的英文 token 直接进用户可见文案（裁定 10 接线时查实）：`src-tauri/src/commands/stock_analysis.rs` 的每日复盘段把 `alert.condition` 原样拼进「above触发: 价格≥…」，`src-tauri/src/init/services.rs` 的通知文本同一形态。改法 = 走 `alertType` + 既有六类 i18n 键（`stockAnalysis.alert.type*`），不再读兼容列；⚠ `services.rs` 与主命令文件都在并行批次的重叠清单里 ⇒ **只改不提交**。

**判据**：呈现层按既有纪律做组件测试 —— i18next 替身回 key 原文 ⇒ 三态按 key 断言、插值断到 `count`；每个破坏样本**只**让对应那一条红（三条全红＝一条门）；且「生产侧 store 实际写哪个键 + 渲染侧出哪句」两层各测、两侧各做变异。

## 9.1 裁定 12 与 H-4 的执行状态（2026-10-15 拍板 ① 后落地）

**为什么计数器与面板格必须同批**：E-1 当时只落了逐只 `tracing::warn!`，聚合那一半刻意没做 —— 计数器没有消费者就是又一个假旋钮（阶段 D 正在收口的那类缺陷）。本批把两端一起接上。

| 层 | 做了什么 | 现场锚点 |
|---|---|---|
| 引擎计数 | `PollRoundStats{codes_total, codes_failed, consecutive_all_failed_rounds, finished_at_ms}`；转移逻辑抽成纯函数 `next_all_failed_streak(prev, total, failed)`，轮末记账抽成 `record_poll_round()` —— `start()` 是个不返回的 loop，判据打不进循环体，只能打在方法上 | `crates/analysis-engine/src/monitor.rs` |
| 三档语义 | 整轮全失败 ⇒ +1；任一成功 ⇒ 归零；**`codes_total==0` ⇒ 归零**（没有启用中的告警就没有「失败的取数」，留着旧连击会把「没单」演成「通道坏了」） | 同上 + `all_failed_streak_has_three_distinct_buckets` |
| 到档日志 | 连击恰满 `ALL_FAILED_STREAK_LOG_AT=3` 时打**一条** `error`（不每轮重复；计数本身继续累加并暴露在 `monitor_status` 里，日志不承担计数） | `record_poll_round()` |
| 时段契约 | `TradingSession::code()` 给六档稳定码。**不用** `status_label()`（中文串端给前端 ⇒ 11 语言里 9 种显示中文）也**不用** `format!("{:?}")`（把 Debug 形态当契约，改变体名就静默改掉前端键）；配 `session_codes_are_distinct_and_stable` 断六码两两不同 | `crates/astock-data/src/calendar.rs` |
| 命令面 | `monitor_status` → `MonitorStatusDto`（`rename_all="camelCase"`，九键）。未装配时**早退**返回 `assembled=false` 且计数全 0，由 `assembled` 告诉前端「这些 0 没有含义」 | `commands/stock_analysis.rs` |
| 呈现层 | `monitorAbsenceSentences()`（纯函数）+ 面板渲染：`readFailed` / `notAssembled` / `pausedBySession` / `fetchFailed` / `allFailedRounds` / `firstRoundPending` —— **一条一句，不相加成一个数**；未装配独占、计数句全部闭嘴；读失败不塌成未装配（一次 IPC 故障 ≠ 结构性缺失）；命令回空报「后端未返回读数」不静默 | `PriceAlertPanel.tsx` |
| i18n | `stockAnalysis.monitor.*` 13 键 × 11 语言（新建该段）。批量脚本带四道自证：逐键断插值槽齐备、`{{`/`}}` 配对、非汉字语言不得混入 CJK 汉字、落盘后除本段外逐字段零改动 | `.workbuddy/tmp/add_monitor_status_i18n.py` |

**判据与读数**
- 组件测试两层各一条链：纯函数层 10 条（含「齐备态一条都不出」「空配置不得立失败句」「连击满 1 轮不重复立句」）+ 渲染层 7 条（i18next 替身回 key 原文 ⇒ 按 key 断言、插值断到 `"failed":3` / `"rounds":4` / `"reason":"ipc down"`）。合计 **17 passed**，与既有两条面板测试一起跑 **32/32**。
- 变异自证 5 个（`.workbuddy/tmp/mut-z12-panel.py`，每次只改一处 + 按 sha256 回写核对）：阈值 2→1 红 **1** 条；撤掉未装配独占 红 **2** 条（纯函数 + 渲染各一）；读失败塌成未装配 红 **3** 条；分母改成「有股票就算失败」红 **4** 条；未知档位码兜成 `morning` 红 **1** 条。基线（未变异）**0 failed / 17**。⇒ 每个破坏样本只红在该当那一格，没有「一改全红」。
- ⚠ 自证脚本自身踩到一条工具链事实：vitest 在被管道接走时**仍然发 ANSI 颜色序列**，第一版解析 `Tests\s+(\d+) failed` 匹配不上 ⇒ 五个变异体被整批误报成「没红」。剥掉转义序列后读数才是真的；**判据的解析器也会是缺陷源**（已记入门禁纪律）。
- 前端其余门：`npm run typecheck` 0 错；`dprint check`（本批 4 个 TS/JSON 面）EXIT=0；`oxlint` 0 warnings / 0 errors；`check-i18n-key-parity` 11 语言键集合一致；`check-i18n-placeholder-usage` R1–R4 全 0 命中。
- Rust 侧读数（pin 1.97、`src-tauri/` 内发起）：`cargo fmt --check` 三个包 **EXIT=0**；`cargo clippy -p axagent-analysis-engine -p axagent-astock-data --all-targets --all-features -- -D warnings` **EXIT=0**（`record_poll_round` / `next_all_failed_streak` / `TradingSession::code()` 连同其 test 都在这一面的覆盖里）；`cargo test -p axagent-analysis-engine --lib` **`676 passed; 0 failed`**（含新增的 `all_failed_streak_has_three_distinct_buckets` 与 `record_poll_round_exposes_counters_and_resets_on_recovery`）。⚠ **`session_codes_are_distinct_and_stable` 不在这个读数里** —— 它住在 `axagent-astock-data`，AE 的 `--lib` 覆盖不到（`-p X --all-targets` 不覆盖依赖 crate 的 test target，同一批已踩过一次）⇒ 该条的读数由 `-p axagent-astock-data --lib calendar::` 单独取，未到之前本条不算闭合。主 crate `clippy --all-targets` 与全量三条仍在排队/归批次收口，读数到达前不称「全绿」。
- **本轮没做的 H 面**：H-1（三处 `catch { /* 静默 */ }` 改错误码）、H-2（阈值单位按 `condition_type` 分列）、H-3（`isTriggered: boolean` 对后端 i32）、H-5（两套组合面板整合，裁定 9）—— 仍归阶段 H 后续批次。
- ⚠ `finished_at_ms` 是「最后一次**真取数**的时刻」：非交易时段整轮跳过时不更新。面板的时段停摆句走 `tradingNow` 现算，不靠这个时间戳推断。

### 9.2 阶段 H 的执行状态（2026-10-15 落地；H-5 按裁定 9 只做文案订正）

| 子项 | 做了什么 | 判据/落点 |
|---|---|---|
| H-1 | 建单/删单/读列表三处静默 catch 全部改成 `translateBackendError(e)` + `message.error`（走既有错误码翻译层，不新造 helper）。此前建单失败「以为建上了」、删单失败「以为删了」、读列表失败与「没有告警」同屏 | 组件测试「建单失败必须报出来」（断言 toast 含后端 detail） |
| H-2 | 类型列改读**权威 `alertType`**（老行按兼容列回落，回落表与后端 `legacy_condition_to_alert_type` 同序）；方向由类型判（`stop_loss`/`support` ⇒ ≤）；阈值列按 `conditionType` 分列单位（price 原样、另两类带 `%`）；建单表单那格随所选条件给 `%` 后缀 | 纯函数 3 条 + 渲染 3 条 |
| H-3 | `isTriggered: boolean` → `0 \| 1`（后端是 `i32`）；**顺带消掉一处重复**：挂载时那趟「取列表 + 按 isTriggered 分流」与 `loadAlerts` 逐字重复 ⇒ 合并成一次调用（判据只打在其中一个上的风险一并消掉） | 组件测试按 tab 计数断言 |
| H-7 | 复盘 `market_status` 从「交易日 + `is_trading_time()`」二值判法改走 `TradingSession::status_label()`（午休不再被写成「已收盘」）；**这一格是写进中文复盘正文**，所以用中文标签（与 IPC 契约那套「只给稳定码」不同口径，注释已写明） | `review.rs` |
| H-8 | 复盘段不再拼老 `condition` 列（英文 token + 6→5 有损），改 `alert_type` + 新增的 `alert_type_label_zh` / `is_below_alert`；阈值优先读新列、单位按 `condition_type` 分列。⚠ **PLAN 原写的第二处（`services.rs` 通知文本）实测不存在**：那里的 `&alert.condition` 是**恢复装配**传给 `db_model_to_config_field` 的（非用户可见文案）⇒ 已订正记录 | `alert_mapping.rs` + `stock_analysis.rs` |
| 裁定 9 | `MonitorView` 头注撤掉「阶段 4 会整合为统一 dashboard」（指向不存在的计划）⇒ 改成如实描述：两块刻意不合，各自实体与刷新路，整合归下一轮 | 同文件 |
| H-6 | `get_market_status` 撤掉 `isTradingDay` / `isTradingTime` 两个**零消费者**键（Rust/TS/rhai/测试四面查过），只留前端在用的 `status`；时段要结构化读数的地方走 `monitor_status` 的会话档位码，不在这里再开一个中文口径 | `stock_analysis.rs` |

**判据与读数**：新增组件测试文件 `PriceAlertPanel.alertRow.test.tsx` **6/6**；变异自证 4/4（`.workbuddy/tmp/mut-h-alertrow.py`：M1 类型列退回英文 token、M2 阈值丢单位、M3 活跃筛选按 boolean 判、M4 建单退回静默 —— 各只红该红那条；M3 因分流是三条渲染断言共同上游而红 3 条，已在脚本里写明这条期望并限定「失败只许落在本文件」）。

## 10. 判据汇总（新增 9 条 + 扩 1 条既有门；裁定 10/12 收口时补 4 条，见 R11 与 Z-12-*）

| 编号 | 形态 | 阶段 | 一句话 |
|---|---|---|---|
| A-1 | 组件测试 | A | 告警事件 payload 的字段必须渲染出来 |
| A-2 | 脚本（剥注释） | A | 告警事件名只有一处权威，禁止字面量 |
| A-3 | Rust 定点 | A | 每个 `MonitorEventEmitter` 实现必须有生产构造点 |
| B-1 | Rust 真库 | B | 同股多条互不顶掉、按条回写 |
| B-2 | 脚本 | B | `is_triggered` 写侧在读、且不在 test 段 |
| B-3 | 组件测试 | B | 空态与「未装配」不得同句 |
| E-1..E-3 | Rust 单元 | E | L2 命中沿用条目自己的到期点（纯函数 + 端到端各一条）/ 限停四态 / 边沿触发 |
| E-5 | 表驱动单元 | E | 会话边界左闭右开（含 11:30/13:00/15:00 三个临界分钟）+ 五档各有自己的一句（反塌缩断言） |
| F-1 | Rust 纯函数 + 端到端 | F | 三态判据/时间窗/计数滚动各锁一格；落库层一条串行端到端（引擎是进程级 static，见 §6 偏差 4） |
| F-2′ | 脚本反计数（并进门 R10） | F | 六类退役生产者必须为 0（原「`None/None` 计数」作废，见 §6 偏差 2） |
| F-3 | 脚本（并进门 R7–R9） | F | 条件单事件名两头一致 + payload 九键逐项相等 |
| F-4 | 组件测试 5 条 | F | 触发三项进文案 + 「不重取告警列表」及其反证条 |
| R11（并进门 `check-monitor-event-authority.mjs`） | 脚本（剥注释剥 test） | 裁定 10 | `price_alerts` 新行构造点**按出现次数**恰好一处，且与 `IsTriggered` 去重谓词同文件 |
| Z-12-1 | Rust 纯函数 + 引擎 | 裁定 12 | 连击三档语义各锁一格（全失败 +1 / 任一成功 0 / `total=0` 也 0）+ 轮末读数落引擎、恢复即归零 |
| Z-12-2 | 表驱动单元 | 裁定 12/H-6 | 六个时段档位码两两不同（IPC 契约不随变体名或中文文案漂移） |
| Z-12-3 | 组件测试 17 条（纯函数层 + 渲染层） | H-4 | 六句缺席各占一格、未装配独占、读失败不塌成未装配；五个变异体分别只红 1/2/3/4/1 条 |
| R12（并进门 `check-monitor-event-authority.mjs`） | 脚本（两侧键集合逐项相等） | 裁定 12 | `MonitorStatusDto` 九字段（snake）与 TS `MonitorStatus`（camel）逐项相等 —— 抽不到一侧即判红，不许由「没抽到」推成「对齐」 |
| G-1..G-3 | Rust 真库 / 脚本 | G | 刷新落一行且两列非零 / 事件名在权威表 / cron 现取含新任务 |
| G-7 | Rust 真库 | G | `refresh_snapshot` 一次跑完三步（β 取数 + 指标表 1 行 + 相关性表真落行），返回值与产物两层各断一次 |
| G-8 | 脚本（**扩既有门** `check-panel-var-landing.mjs`，清单 13→14） | G | `risk_correlation_lookback_days` 三面默认值对账 + 消费点 ≥2（定义 + 真调用）；调用点退回裸数字即红 |
| G-9 | Rust 真库 | G | `refresh_metrics` 第二条 = **覆盖的持仓只数**（2 只返 2）；键名与值不再各说各话 |
| G-10 | Rust 真值对表 | G | `build_builtin_jobs()` 的每个 `task_type` 都在后端「立即执行」白名单里 + 负控；配套 `seed_creates_all_builtin_jobs_and_is_idempotent`（10 条）与 roundtrip 时刻断言 |
| G-11 | 组件测试 4 条 | G | T+0 面板开关：默认关 + 成本句上屏 / 开启载荷是合并配置 / 保存失败回滚 + toast / 改阈值只发改动格；四变异各红该红那条 |
| R17（并进门 `check-monitor-event-authority.mjs`） | 脚本（形态锁） | D | `start()` 的 `loop {` 之后必须仍能读到 `poll_interval_secs`（读一次就定型＝面板改了没反应）；自证含两种合法写法正控 + 原形态负控 |
| D-3 | Rust 单元（纯函数版） | D | 回灌两个运行期参数：合法采纳 / 0 拒绝 / 越上界拒绝 + 边界采纳 / 负冷却拒绝 + 0 合法 / 键缺席不动 |
| I（新门 `check-task-type-whitelist.mjs`，进 CI） | 脚本（跨语言三处对账） | 裁定 17 | 后端「立即执行」白名单 ≡ 前端可执行集，且每个值都能被任务中心三组接走；五个坏样本自证 |
| H-1..H-3 | 组件测试 6 条 + 4 变异 | H | 建单失败要报出来 / 类型与阈值按权威字段渲染 / `isTriggered` 按 0-1 数字分流 |
| D-1 | **扩既有门** `check-panel-var-landing.mjs` | D | 三条监控键纳入落地清单 |
| D-2 | Rust 单元 | D | setter 生效于下一轮（形态锁） |

门的存放位置由可用基础设施决定：全文级禁字面量门只能住在有 `code_only()` 剥离能力的那一层（脚本层），Rust 侧只放行为锁。**每条新门的第一个读数打在未修的树上**（先证明它会红），撤门自证要拆在真正决定该断言的那道守卫上。

## 11. 待拍板（按编号点完即可开工）

1. **裁定 1 · 发射器归属**：接线 `TauriMonitorEmitter` + 退役 `AppMonitorEmitter`（推荐，完整实现已在库里，且避免重复定义）／ 或把三项职责搬进 `AppMonitorEmitter` 并删死文件。
2. **裁定 2 · 告警生命周期**：边沿触发 + 触发即终态（`is_triggered=1`，面板给「重新激活」）（推荐）／ 或持续重复提醒（保留冷却窗口，不置终态）。
3. **裁定 3 · 重启后首次已越界**：报一次并标注「进入监控时已越界」（推荐）／ 或等下一个穿越边沿（风险：止损可能长时间静默）。
4. **裁定 4 · T+0 自动重跑**：接面板并默认关（需 11 语言 i18n + 成本声明：一次重跑 = 一整轮股票分析）／ 退役这条腿（含 `get_t0_config`/`set_t0_config` 两命令 + store 两方法，按六层检索退役）。
    → **2026-10-15 按推荐 ① 落地**：链路后端全通（引擎判定 + callback + 前端 listener），只缺 UI ⇒ 接面板，默认关 + 成本句上屏常显。见 §7.1.2。
5. **裁定 5 · 条件单**：收口到引擎 + 新表、退役 `settings` 那套（现网该键 0 行 ⇒ 零迁移成本，推荐）／ 或整层退役不接。
6. **裁定 6 · 涨跌停 / ST**：加标注不抑制告警（推荐，避免把「拿不到」变「没触发」）／ 或封板日抑制突破类告警。
7. **裁定 7 · 通知腿**：本 PLAN **不含**通知装配，另立里程碑 M-NOTIF，期间 `monitor_channels` / `monitor_min_severity` 面板格显示「当前无消费者」（推荐）／ 或在本 PLAN 内一并装配（工作量：渠道配置来源 + 命令 + 面板 + 11 语言，≈ 一个独立批次）。
8. **裁定 8 · `cash_pct` 数据来源**：现网 `portfolio_holdings` / `trades` 均 0 行，现金这一项从哪来（设置格 / 手填 / 该列退役）？
    → **2026-10-15 取完证据后重开**：仓里有四处「像现金但不是现金」的东西，逐条按「schema 有 / 生产链上有人读」两栏分：
    ① `fund_transfers` 表（银证出入金，`entities/src/fund_transfers.rs:7-23`）—— **Rust 侧零消费者**：`commands` 里没有 `list_fund_transfers` / `record_fund_transfer` 两条命令，而前端 `FundPanel.tsx:47/73` 已经在调它们（见新增裁定 16）。要走这条路 = 先补命令面 + 面板，≈ 独立批次。
    ② 工作流变量 `portfolio_cash`（`commands/stock_workflow/hooks.rs:783-790` 注 `json!(0.0)`，原注释自己写「暂时注入 0.0，后续可扩展为从账户设置读取」）—— 消费方倒是齐全（`rhai_pm.rs:161/178` → `portfolio_formula.rs:591/683` 把 `total_mv + portfolio_cash` 当组合总值）。**源头是常量**，接它等于把同一个 0 换个地方写。
    ③ 面板键 `pos_min_cash_pct`：只在 `StockAnalysisConfigPanel.tsx:84` 注释与 locales 里活着，`seed_variables.rs` 没播、Rust 没读 ⇒ 死键，不是来源。
    ④ `quant_runs.initial_cash` —— 回测假设资金，语义不是账户余额，不能挪用。
    ⇒ 三选一（推荐 ③）：① 新增「现金」设置格（要按「新增可调参数五点对账」+ 11 语言，且 §13 写着「不给监控加新可调参数」）；② 先补 `fund_transfers` 两条命令再按出入金汇总（跨批次）；③ **`cash_pct` 与 `max_drawdown_pct` 两列的「恒 0」如实改口径** —— 实体注释与 `compute_dashboard` 的注释不再写「由 refresh_metrics 补算」，`max_drawdown_pct` 走 ② 真算（历史净值序列现成，不依赖现金来源），`cash_pct` 在拿到来源之前**不进面板**（实测 `PortfolioMonitorPanel.tsx` 本来就没渲染这两格 ⇒ 撤的是「注释里的承诺」而不是展示位）。
9. **裁定 9 · 两套组合面板**：合并成一个入口（结构改动，建议下一轮）／ 本 PLAN 只做撤展示位与文案订正（推荐）。
    → **2026-10-15 按推荐落地**：`MonitorView` 头注不再承诺「阶段 4 整合」，改为如实描述两块为何不合（两个实体、两条刷新路）。整合仍归下一轮。
10. **裁定 10 · `price_alerts` 行随分析无界增长**（阶段 B 实施中发现，详见 §4.1 执行状态）：① 建前按 `(股票代码, 告警类型)` 复用未触发行（推荐，配置数与请求数都不再涨）；② 保留追加 + 面板给「清理陈旧重复」动作；③ 维持现状。
    → **2026-10-15 拍板 ①，已落地**：见 §4.1。
11. **裁定 11 · 监控取数是否切批量**（阶段 E-6 核对后判为「等上游」，详见 §5 执行状态）：① 维持逐只 + 按股票聚合（推荐，本轮已把 56 条压成 16 次/轮，且保住 6 vendor 降级链与 as-of 语义）；② 现在就切腾讯批量（拿到 80 码/请求，代价：失去降级链 + 给 `MarketDataProvider` 加一臂三家用逐只回退的假批量 + 回放批次拿空行情）；③ 先给批量补降级与 as-of 能力再切（≈ 一个独立批次，属 astock-data 侧）。
    → **2026-10-15 拍板 ①（无代码改动）**：请求数已不随告警行数增长，切批量换来的是「三家用逐只回退的假批量」+ 降级链从 6 家缩到 1 家 + as-of 模式静默拿空行情。判为「等上游」（属 astock-data 侧独立批次），本 PLAN 不做。
12. **裁定 12 · 「监控实际未在取数」这一读数要不要有**（阶段 E-1 后半挪出来的）：① 与 H 的面板格同批做「本轮 x/y 只取数失败」+ 连续全失败聚合告警（推荐，计数器一落地就有消费者）；② 只要日志（现状：逐只 warn，没有聚合面，长期全挂要靠人翻日志）；③ 不做。
    → **2026-10-15 拍板 ①，已落地**：见 §9.1。
13. **裁定 13 · 条件单要不要 UI 入口**（阶段 F 实施中实测：前端 invoke 0 处、store 0、i18n 键 0 ⇒ 建单目前只能走 agent 工具）：① 本 PLAN 内加面板（建单 + 列表 + 启停，≈ 一个独立批次）；② 归阶段 D 与「假旋钮收口」一并接（推荐）；③ 保持 agent-only，面板写明「条件单由 AI 助手登记」。
    → **2026-10-15 用户拍板 ①（要界面）**，已随 F-8 落地：`src/components/stock-analysis/ConditionalOrderPanel.tsx` + 挂在 `WatchlistPage` 底部（全宽一行，与价格预警并列不合表）。见 §6 执行状态 F-8 条。
14. **裁定 14 · pending 交易意图的自动过期要不要接回**（随 `risk_inspection` 退役下线，能力仍在命令 `expire_old_trade_intents`）：① 归阶段 G 的 cron 批统一挂（推荐，不另造第二条轮询）；② 保持手动/agent 触发；③ 连同意图审核一起判为待退役。
    → **2026-10-15 拍板 ①**：归 §7 阶段 G 的 cron 批（与组合指标日更同批，见 G-3）。
15. **裁定 15 · agent 工具建的告警不进监控引擎**（裁定 10 接线时实测发现）：`stock_create_price_alert` 走 `commands/stock_analysis_bridge.rs` 的 `dispatch_stock_command`，该 handler 只拿得到 `db`（签名在 `create_stock_handler` / `execute_stock_command` 两层，均无 `AppState`）⇒ 写库后不调 `add_config`，**要等下次启动装载才生效**；而面板那条路（`create_price_alert` 命令）是即时生效的。同一个「建告警」在两条路上有两种生效时刻，属于本 PLAN 要消掉的那类歧义。选项：① 把 `Option<Arc<RealtimeMonitor>>` 沿 `create_stock_handler` 两层透传下去，与命令路共用 `config_for_alert`（≈ 小改，动的都是既有签名）；② 保持现状，但在 agent 工具的返回 JSON 里如实写明「本条要下次启动才生效」（呈现层不歧义的最低要求）；③ 判为待退役（agent 不建告警）。
    → **2026-10-15 落地（改法比 ① 更省）**：不必沿 `create_stock_handler`/`execute_stock_command`
    两层加参数 —— 那一层的 `AppHandle` 已能取到 `AppState`，在**调用时**用
    `app_handle.try_state::<AppState>()` 取 monitor 即可（注册时捕获会踩「装配时机」：注册顺序
    与监控注入顺序没有保证）。落库 + 进引擎抽成 `price_alert_upsert::upsert_price_alert_and_wire`
    （**两条路共用一份实现**），返回体多一个 `monitorWired: bool` —— 拿不到引擎时如实说「本轮未进引擎」。
    用 `try_state` 而非 `state`：后者在未 manage 时 panic，而缺状态本来就是合法的「未装配」。
16. **裁定 16 · 资金流水这条腿整条没接**（阶段 G 取证时查实，2026-10-15）：`src/components/stock-analysis/FundPanel.tsx` 调三条命令 —— `list_fund_transfers`（`:47`）、`get_fund_summary`（`:48`）、`record_fund_transfer`（`:73`），而全 `src-tauri/` 下**三条都没有 `#[tauri::command]`**（按名 grep 零命中）；更要紧的是该组件**全仓零挂载点**（`grep FundPanel` 只命中它自己那个文件）。⇒ 现状是「一个没被渲染的组件 + 三条不存在的命令 + 一张有 schema 却零 Rust 消费者的 `fund_transfers` 表（`entities/src/fund_transfers.rs:7-23`）+ 一整套仍计入 parity 的 `stockAnalysis.fundPanel.*` 键」。这与裁定 8 是同一件事的两头：现金来源要么走这张表，要么整条退役。选项：① 补三条命令 + 落库 + 挂载 + 与 `cash_pct` 同源汇总（≈ 独立批次）；② **整条退役**（删 `FundPanel.tsx` + 清 11 语言的 `fundPanel` 段 + 撤实体声明里的 `fund_transfers`？**不撤** —— 声明撤了不等于物理列被删（`additive_only`），且撤表要单独拍）；③ 保持现状，只在 PLAN 里把它记成「已知未接线」，不占本轮工时。

    → **2026-10-15 用户点 ③「按推荐实施」⇒ 取 ②（退干净）**：删 `FundPanel.tsx`（零挂载点）
    + 11 语言的 `stockAnalysis.fundPanel.*` **18 键**；`fund_transfers` 实体声明**保留**
    （声明撤了不等于物理列被删，且撤表要单独拍）。三条不存在的命令从未有过实现，无需退役。
17. **裁定 17 · `task_type` 三处清单没有跨语言门**（阶段 G-cron 实施中发现，2026-10-15）：同一个 `task_type` 要同时登记在 ① `commands/constants.rs`（权威常量）/ 各业务模块 ② `commands/scheduled_task.rs::BUILTIN_TASK_TYPES`（「立即执行」白名单）③ 前端 `taskGrouping.ts` 的两个 `Set`（分组 + 可执行）。本批把 `lesson-validation` 漏登的两处补齐，并给「② ⊇ 种子任务」加了 Rust 真值对表（G-10）；但 **③ 与 ② 之间没有任何门**，下次再漏就是同一种静默（任务中心置灰/归错组，后端照跑）。选项：① 扩 `check-contracts.mjs` 加一条「解析前端两个 Set ⊇ 后端 `BUILTIN_TASK_TYPES`」的对账（≈ 20 行，与既有命令/DTO 对账同层，推荐）；② 把这两份清单交给 `schema-gen` 生成（像 `events.ts` 那样出 `taskTypes.ts`，代价：多一次构建步骤 + 前端要改成 import）；③ 维持手工，只靠 G-10 守后端那一半。
    → **2026-10-15 落地为独立 CI 门**（比 ① 更进一步）：`scripts/check-task-type-whitelist.mjs`（I1 双向相等 / I2 归组 / I3 逐字来源 + 五个坏样本自证），注册进 `ci.yml` 与 `ci-check.mjs` 同位同源。**为什么没放进 `check-contracts.mjs`**：那份脚本当前不在 CI 的 35 道门里（只 `npm run contracts` 手动跑）⇒ 写在那里等于没牙齿；原段落已移出并留指针注释，不留两份实现。
18. **裁定 18 · `keylevel_lookback_days` 是同类假旋钮的兄弟**（G-8 顺手查实）：它与 `risk_correlation_lookback_days` 同批播种（`seed_variables.rs:613`）、面板同样可编辑（`StockAnalysisConfigPanel.tsx:153`），而 `key_levels.rs` 的取数链**同样不吃它**（裸字面量）。G-8 只接了「相关性」那一条（在监控链上）；选项：① 同批接掉（关键价位链，≈ 小改 + 进 LANDED 清单 ⇒ 15 条）；② 归阶段 D「假旋钮收口」批统一处理（推荐，与 `monitor_alert_cooldown_secs` 那三条同性质）；③ 撤键（面板与种子一起摘）。
    → **2026-10-15 按 ② 处理，但深查后判为「不可接通」**：见新增裁定 19。
19. **裁定 19 · 关键价位链的消费者不存在（新挂，2026-10-15 阶段 D 取证）**：`keylevel_lookback_days` 想接的「检测回看窗口」在仓里**没有实现** ——
    · 唯一的窗口参数在 `KeyLevelTracker::backtest_key_levels_with_config`，而它是 **`_lookback_days`（收下不用）**；该回测真正用的是 `cfg.query_limit`（快照条数）与 `cfg.klines_count`（500 根 K 线），并按每行快照日期取**之后的** K 线；
    · 写快照的那条路 `store_key_levels` **全仓零调用者**（`blackboard_snapshot["key_levels"]` 只有它一处写点、回测一处读点）⇒ 命中率回测读的是一个「没人写的键」，读数恒 0/空；
    · 因此「接通」只能是接一个没人读的值（正是阶段 D 要消灭的形态）⇒ 本轮按「不得在 UI 造成歧义」改为**如实标注**（面板描述加「当前无消费者」后缀 × 11 语言）。
    选项（下一轮）：① 补真检测器（关键价位从 K 线算出来 → 写快照）+ 让回看窗口走面板键，同时把回测的 `_lookback_days` 变成真参数；② **整链退役**（`KeyLevelTracker` + `backtest_key_levels` 命令 + 前端调用 + 面板那一格 + 11 语言描述），并把 `_lookback_days` 参数删掉；③ 只撤面板那一格，引擎层的死方法留到下次重构。**推荐 ②**（与裁定 16 同一条线：没被渲染/没人读的东西要么接通要么退干净）。
    → **2026-10-15 用户点「按建议实施」⇒ 取 ② 整链退役，已落地**（七层逐层改）：
    L1 删 `crates/analysis-engine/src/key_levels.rs` 整文件 + `lib.rs` 的 `pub mod`；
    L2 删命令 `backtest_key_levels`（含 import 与 Key Levels 段注释）并订正核心工作流里那条陈旧注释；
    L3 删面板三行 `b()` **与整个 keylevels 组对象**（组变空必须连组一起删）；
    L4 删种子三个 `Variable` 与段注释；L5 删 11 语言三个 `configDescriptions.*` + `settings.group.keylevels`（该键只被这个组用）；
    L6 删两处测试引用；L7 删 `check-generation-scope-registry.mjs` 里指向该文件的登记（**不删会让 CI 门自己红**）。
    ⚠ 残留只有 `src/register_commands.rs` 的一行 —— 它是 `build.rs` 的**生成物**，下次构建自动消失，禁手改。

## 12. 门禁执行口径

- 阶段内：`cargo check` + 定点 `cargo clippy -p <crate> --all-targets -- -D warnings`；前端 `npm run typecheck` + 定点 vitest。
- **cargo 的工作目录是门禁的一部分**：所有 cargo 命令必须在 `src-tauri/` 内跑。仓库的 1.97.0 由 `src-tauri/rust-toolchain.toml` pin，rustup 按**当前目录**解析 —— 在仓库根目录加 `--manifest-path src-tauri/Cargo.toml` 会退回 stable(1.96)，lint 面与编译器都和 CI 不一致（本批实测：同一条 `nonminimal_bool` 只在 1.96 报，从根目录跑出来的 clippy 红是伪红）。
- **全部阶段完成后**跑一次全量三条（与 `.github/workflows/ci.yml` 逐字一致）：`cargo fmt --all --check` / `cargo clippy --workspace --all-targets --all-features -- -D warnings` / `__TAURI_WORKSPACE__=true cargo test --workspace --no-fail-fast`。
- 主 crate 门禁是批次级（10–25 分钟、3.5GB×2）；**跑着的时候不要改 Rust**。`beacon`: 被 `include_str!` 的资产都算对象，改 md/.rhai 同样会造半新半旧读数。
- 全仓 `ci:check:quick` 可能卡在并行批次 `portfolio_formula.rs` 的 rustfmt 漂移 ⇒ 验自己的门时逐条单跑或 `--skip-rust`，并如实标注「跳过了哪条、为什么」。
- GitHub Actions 自 2026-09-07 起不为 master push 创建 run ⇒ 「CI 绿」无证明力，收口只认本地三条 CI 口径读数。
- 每阶段收尾报三段：改了哪些文件 / 跑了哪些门（带读数）/ 没跑什么、为什么。**没跑完不叫完成。**

### 12.1 本批收口时的实测阻断（2026-10-15，共享树）

**（22:0x 更新）** 上一轮挡路的 `axagent-astock-data` 编译错误（P0-4 批次的未提交半成品）**已被对方修好**：`cargo check -p axagent-astock-data --lib` **EXIT=0** ⇒ 本 PLAN 的 Rust 读数重新可跑。跑的时候注意两点：主 crate 是批次级（10–25 分钟）；**可能被并行批次的构建锁挡住**（实测 `Blocking waiting for file lock on build directory` —— 先看 `tasklist` 里有没有别人的 rustc 占着，**别抢锁、别循环重试**）。

仍在的**非本批**红（已定归属，不修）：

- `check-panel-var-landing.mjs` 正控第 2 条：`portfolio-mgr-h-long.rhai` 的 `pm_required_margin_pct()` 调用被该批次 P0-4 的语义反转摘掉（文件内注明「2026-10-16 语义反转」），跨载体那一环当场断 ⇒ 门红在**它的改动**上。**本批新增/改动的条目全部通过**（G-8 与 D 批共 7 条判据由 `.workbuddy/tmp/verify-g8-landing.mjs` 单独取证 7/7；脚本把那份 rhai 只在内存里换回 HEAD 版，不落盘）。
- `cargo test -p axagent-analysis-engine --lib`（22:3x）：**687 passed / 1 failed**。失败的是
  `evidence_weight::tests::technical_bias_decays_and_valuation_bias_grows_with_horizon`
  （`evidence_weight.rs:1717`，「估值属主 value-investor 未随周期单调变强」）—— 该断言 **HEAD 上就存在**
  （`git show HEAD:` 可查到原句），是 P0-3 批次改 `get_horizon_base_weights`（其 hunk `@@ -334 +334,6 @@`）
  之后被打破的 ⇒ **非本批**。本批新增的四条判据在同一次运行里全部 `ok`
  （`chinese_label_and_direction_...` / `apply_runtime_vars_adopts_...` / `correlation_lookback_window_...` /
  `refresh_snapshot_runs_...`）。
- `src-tauri/crates/astock-data/src/mcp_tools.rs:4285` 有一条 `dead_code: fn compute_dcf is never used`
  警告（同批次未提交改动留下）—— 警告不影响本地测试，但 `clippy -D warnings` 会把它升级成 error。
- `npm run check:facts`：3 条「引用指向空行」—— 2 条在 `harness/src/rhai_ast_cache.rs` 指向 `stock_analysis.rs:199`（**HEAD 上该行就是空行** ⇒ 旧腐烂，非本批移位所致，已逐行对证）；1 条在 `harness/src/domain_semantics.rs` 指向 `evidence_weight.rs:762`（由该批次 `:609` 处 +2 行插入造成）。

已自愈的红：`check-contracts.mjs` 的 `[A] retry_failed_reflections 未注册`（对方构建后生成物刷新即恢复）；`cargo fmt` 在本批改动文件上的残留。

## 13. 明确不做

- 不给监控加新可调参数（现有 4 条键先接通，接通通路优于新造旋钮）。
- 不引入新的行情渠道；只修既有渠道的静默降级与新鲜度。
- 不在本 PLAN 内重构 `MonitorView` 的布局整合（裁定 9 归下一轮）。
- 不动 `.worktrees/` `output/` 副本目录（既有纪律：副本不删，改门的口径）。

## 14. 读数汇总（2026-10-15 深夜批：G-cron + G-1 + D + H）

| 面 | 读数 | 备注 |
|---|---|---|
| 前端 typecheck | `npm run typecheck` **0 错** | 含 `isTriggered: 0\|1` 与三处新 helper |
| 组件/单元测试 | `PriceAlertPanel.t0` **4/4**；`PriceAlertPanel.alertRow` **6/6**；三个面板文件合跑 **27/27**；`src/components/stock-analysis/__tests__` **361/361**；`src/stores/__tests__` + `TasksCenterPage` **257/257** | 每条新断言都有对应变异体自证（4/4 + 4/4） |
| 落地门 `check-panel-var-landing.mjs` | 清单 15 条；本批 7 条判据单独取证 **7/7** | 现网仍报 2 条**他人**红（rhai 语义反转），见 §12.1 |
| 告警/监控门 `check-monitor-event-authority.mjs` | R1–R17 全绿；`--selftest` **71/71** | R17 为新增（轮询间隔形态锁） |
| 新门 `check-task-type-whitelist.mjs` | 门绿；`--selftest` **5/5**；已注册进 `ci.yml` + `ci-check.mjs` | 裁定 17 |
| i18n | key parity 一致；placeholder 干净；untranslated **无新增**；locales dprint 干净、纯 LF | 本批动过 11 语言的 6+3+3 处 |
| 契约门 `check-contracts.mjs` | errors=0（他人那条已随生成物刷新自愈） | 该脚本不在 CI，仅手动 |
| `check:facts` | 3 条旧腐烂（2 条 HEAD 即空行 + 1 条他人 +2 行插入），非本批 | 见 §12.1 |
| Rust fmt | `cargo fmt -p axagent -p axagent-analysis-engine -- --check` **EXIT=0** | 含本批全部改动 |
| Rust · engine `--lib` | **687 passed / 1 failed**（22:3x）；失败那条 `evidence_weight` 单调性是**他人**未提交改动打破的 HEAD 既有测试（见 §12.1）；本批四条新判据全 `ok` | 判据：`chinese_label...` / `apply_runtime_vars...` / `correlation_lookback_window...` / `refresh_snapshot_runs...` |
| Rust · astock-data `--lib calendar::` | **全绿**（含 `session_codes_are_distinct_and_stable`、`trading_session_boundaries_are_half_open`、`every_session_bucket_has_its_own_label`） | §9.1 里「未取到之前本条不算闭合」的那条 Z-12-2 **就此闭合** |
| Rust · engine `--lib`（keylevel 退役后复跑） | **687 passed / 1 failed** —— 与退役前**逐条同数**（`key_levels.rs` 本身零测试），失败仍是他人那条 `evidence_weight` 单调性 | 用一个独立的 `CARGO_TARGET_DIR` 绕开卡住的构建锁跑完（见下） |
| Rust · 主 crate `--lib` + clippy | **被别人挡住（已复现到底）**：`src/commands/pool_scan.rs:832/843`（E0063 `PoolScanOutcome` 缺 `pending_remaining`/`timed_out` —— 另一批次结构体加了字段、测试夹具没跟）⇒ `could not compile axagent (lib test) due to 2 previous errors` | 解锁后即可跑；本批自身文件已过 `cargo fmt --check` |：被并行批次的构建锁挡住（对方 rustc 5.8GB 在编，实测 `Blocking waiting for file lock`） | 解锁后自动续跑（后台链：`d-main-test.log`），随后补 `clippy -p axagent --all-targets` 与 CI 三条 |

## 15. 存量数据清理（2026-10-15，用户点「删」）

**对象**：`price_alerts` 里裁定 10 之前留下的重复未触发行。

**判据与他的写入规则同源**（这是这次能放心删的原因）：`price_alert_upsert` 复用未触发行时的选择是
`(stock_code, alert_type, is_triggered=0)` 中 `ORDER BY created_at DESC` 的第一条 ⇒ 清理就是沿用这条谓词，
**保留每组最新那条，删其余**（`row_number() OVER (PARTITION BY stock_code, alert_type ORDER BY created_at DESC, id DESC) > 1`）。
不去猜「哪条更对」—— 同组各行的阈值实测同值，且引擎重启后按新表装载。

| 时点 | total | 未触发 | 已触发 | 重复组 |
|---|---|---|---|---|
| 清理前 | 56 | 44 | 12 | 12 组（3×7 + 2×5） |
| 清理后 | **37** | **25** | 12 | **0** |

- `DELETE 19`（与「重复组多出的行数」逐组相加一致）。
- 已触发的 12 行是**终态历史**（裁定 2 边沿触发 + 触发即终态），照旧保留；同一股多条已触发行是正常的跨周期记录。
- **回滚快照**：清理前 56 行全字段导出在 `.workbuddy/tmp/price_alerts-before.tsv`（本地临时目录，不入库不入版本库）。
- 清理时应用未运行（`tasklist` 无 `axagent.exe`）⇒ 不存在「内存引擎还握着已删 id」的窗口；下次启动按新表装载。

> **构建锁卡住的绕行（2026-10-15 实测）**：`tasklist` 里有 2–5 个 `cargo.exe` 却**没有一个 `rustc.exe`**、
> 自己的 cargo 一直 `Blocking waiting for file lock` ⇒ 那是**锁被空占**（对方卡在别处），等待无尽头。
> 不抢锁、不杀对方进程：`CARGO_TARGET_DIR=<临时目录>` 另起一份（依赖全量重编 ≈ 十几分钟），跑完即删（本轮回收 2.7G）。
