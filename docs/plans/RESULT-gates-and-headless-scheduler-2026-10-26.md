# 实施结果：开放档内买入 + 三门可配置　/　cron 剥离为常驻调度（2026-10-26）

> 用户裁决（2026-10-26 15:20）：
> ① 开放档内买入、风控/估值/情绪三门
> ② 将 cron 从 GUI 进程剥离为常驻调度，**按建议顺序**（A 应用常驻 → B 跨进程互斥 → C 独立 daemon → D 三门放开）
>
> 本轮实施：**D（三门 + 顶层采纳）已完成**；**A 已就位（仅差开机自启开关）**；
> **B 已完成（跨进程 leader 租约）**；**C 已完成（无头服务端跑调度器）**；
> 另修复 **R-3y**（回放 cron 与 mover-recall 抢锁 ⇒ 20 日全失败）。

---

## 0. 一句话

> **四个开关已落地为运行期可切换的产品决策**（DB `settings` 表 → 进程内快照 → Rhai 宿主函数），
> **改开关不换代、不重播种、下一轮分析即生效**；
> **调度器已能脱离 GUI 进程运行**（无头服务端路径），并加了**跨进程 leader 租约**防双派发。

---

## 1. D：开放档内买入 + 三门（可配置）

### 1.1 四个开关

| 开关 | `settings` 键 | 判据 | 放开后 |
|---|---|---|---|
| 风控门 | `gate.risk_veto_enabled` | 风险档 高/极高 ⇒ 买入→持有/观望 | 高/极高风险票可开仓（仓位仍按 `pm_risk_cap` 封顶） |
| 情绪门 | `gate.sentiment_enabled` | 全市场封板率 < 0.5 ⇒ 不出买入/增持 | 超短档不再因市场情绪被否决 |
| 估值门 | `gate.valuation_enabled` | DCF upside ≤ −35% 或 PE ≥ 自身 5 年 80 分位 | 长档不再因严重高估被否决 |
| **顶层采纳档内买入** | `gate.tier_buy_enabled` | `R-CALIB-CAP`（校准未达标 ⇒ 置信封顶 ⇒ **动作联动降级**） | 顶层采纳档内已判出的「买入」 |

**默认全关 ⇒ 与引入前逐位一致**（改动不改变既有行为，除非显式打开）。

### 1.2 关键设计决策

| 决策 | 选择 | 理由 |
|---|---|---|
| 开关存哪 | **`settings` 表**（key/value），非模板变量 | 改模板变量会触发 `version+1`（换代快照）⇒ 与 **G1 冻结换代**冲突 |
| 改开关要不要重播种 | **不要** | 开关走**运行期**读取（`pm_gate_enabled` 宿主函数），**不改图结构** ⇒ 不升 `TEMPLATE_VERSION` 的语义负担 |
| 脚本怎么读 | 新增宿主函数 `pm_gate_enabled(name)` | Rhai 宿主函数是**同步闭包**（不能 await DB）⇒ 沿用本仓既有范式（`conditional_order_bridge`）：**表 → 进程内快照** |
| 门名用什么 | **完整键**（`gate.risk_veto_enabled`） | 避免「同名不同义」：短别名（`risk`）在脚本与 DB 两处长得一样却可能指不同的门（MEMORY-RULES #738） |
| 未知门名 | 返回 **false**（不放行） | 脚本里若传错门名，保守方向必须是「仍按原门否决」 |

### 1.3 放开的**精确边界**（逐条显式，不含糊）

| 项 | 行为 |
|---|---|
| 放开**只停止否决**，**不抬信号** | 门从来单向拦；放开后动作仍由评分/后验决定 |
| 「数据缺席」**不是门** | 估值门 `unjudged`（DCF 不适用且 PE 分位缺失）**不因放开而变成「通过」** —— 缺席 ≠ 通过是数据契约 |
| **未识别 action 值域**的保守降级**不是门** | 「待明日观察」+ 极高风险 → 仍降级为观望（门开了也如此） |
| 校准 cap **仍压置信**，只解除**动作联动** | 未校准的置信不作面值（那是事实）；但不再把动作压成持有 |
| **DQI 质量 cap / 熊市门禁 / trader 封顶** | **一律保留**（各有独立判据，不是本次要解的死循环） |
| **P0-5 降杠杆封顶**（超短档，microstructure 未接线） | **保留** ⇒ 超短档即使放开情绪门与风控门，**开仓动作仍被压回「持有」** |

### 1.4 代码落点

| 文件 | 改动 |
|---|---|
| `crates/analysis-engine/src/decision_gates.rs`（**新建**） | 单一真相源：四门状态位图 + DB 刷新 + 持久化 + 测试隔离锁 |
| `crates/analysis-engine/src/portfolio_formula.rs` | `apply_risk_veto` 读全局开关；新增 `apply_risk_veto_with_gate`（显式参数版）；4 条新判据测试 |
| `src/commands/stock_workflow/rhai_pm.rs` | 注册宿主函数 `pm_gate_enabled` |
| `src/commands/portfolio-mgr.rhai` | 读四门；`calibration_cap_binds_action` 区分「压置信」与「压动作」；cap 归因改用实际生效值 |
| `src/commands/portfolio-mgr-h-long.rhai` | 估值门可放开（**unjudged 不放行**）+ `valuationGateOpen` 字段 |
| `src/commands/portfolio-mgr-h-ultra-short.rhai` | 情绪门可放开 + `sentimentGateOpen` 字段 |
| `src/commands/decision_gates.rs`（**新建**） | 3 个 Tauri 命令：读 / 单写 / 批量写 |
| `src/init/state.rs` | 启动期从 `settings` 加载四门（**所有路径共同入口**：Tauri / CLI ×2 / server_main） |
| `src/components/settings/DecisionGatesPanel.tsx`（**新建**） | 设置面板（独立 tab，不混进模板变量面板） |
| `src/i18n/locales/*.json`（11 语言） | `stockAnalysis.decisionGates.*`（14 key）+ `stockAnalysis.settings.tab.gates` |

### 1.5 模板版本

`TEMPLATE_VERSION` **187 → 188**（Rhai 脚本内容变了 ⇒ 必须升版触发重播种，否则存量库跑的还是旧脚本）。

**⚠ 与 G1「冻结换代」的关系**：v188 是**一次性**的（脚本改动必须重播种才能生效），
之后开关的调整**不再需要升版** ⇒ G1 的冻结目标**仍然成立**。

---

## 2. A：应用常驻

**核查结论：机制早已存在，只差两个开关。**

| 组件 | 状态 |
|---|---|
| `tauri_plugin_autostart` | ✅ 已注册（`init/plugins.rs`）+ capabilities 授权 + 前端 `GeneralSettings.tsx` / `AppInitializer.tsx` 已接线 |
| `close_to_tray`（关窗隐藏到托盘，**进程继续跑 ⇒ 调度器继续 tick**） | ✅ 后端 `set_close_to_tray` + `AtomicBool` + `on_window_event` 全在；前端 `minimizeToTray` 已接线 |
| 托盘 | ✅ `tray_enabled` 已接线（`start_background_services` 按设置装载） |
| **生产库现值** | `trayEnabled=true`、`minimizeToTray=true`、**`autoStart=false`** |

**⇒ 关窗后应用**已经**留在托盘、调度器继续跑。A 方案只剩「开机自启」一项开关**（`autoStart=false` ⇒ 开机不自动启动 ⇒ 早上不开应用就没有调度）。

**⚠ A 方案不是本次改代码的重点**（机制齐备），但**它暴露了 C 的必要性**：A 只解决「应用开着的时候」，C 解决「应用没开的时候」。

---

## 3. B：跨进程 leader 租约（R-3aa）

### 3.1 为什么是「租约表」而不是 PG advisory lock

首选 `pg_try_advisory_lock`（会话级、进程退出自动释放，语义最干净），**但本仓不可行**：

- `DatabaseConnection` 是**连接池**（`db.rs` 里 `max_connections` 有上界）
  ⇒ 会话级 advisory lock 绑在池中**某个连接**上，下一轮 tick 用的可能是另一个连接 ⇒ **锁语义失效**
- 事务级 `pg_try_advisory_xact_lock` 随事务结束即释放 ⇒ **不构成互斥**

⇒ 改用**带过期时间的租约行**（双方言、自动过期、**不依赖任何「进程退出时清理」逻辑**）。

### 3.2 机制

| 动作 | 实现 |
|---|---|
| 抢租约 | `UPSERT` 条件更新：**只有「无有效租约」或「已过期」才接管** ⇒ 两个进程同时抢，DB 行级串行化收敛到**恰好一个** holder |
| 续租 | 每 30s 更新 `expires_at`（TTL 90s ⇒ 允许连续 2 次心跳失败） |
| 释放 | 进程正常退出时删行 |
| **崩溃** | 不再续租 ⇒ **租约到期自动失效** ⇒ 别的进程接管（**无需清理逻辑**） |

### 3.3 调度器侧接线

`runtime` crate **仍零 DB 依赖**（它靠 `with_leader_check(closure)` 注入判定，不感知租约是什么）：

```
每轮 tick → leader_check() → false ⇒ 本轮不派发任何任务（继续 tick，租约可能回来）
```

**⚠ 必须每轮实时判定**（不是启动时判一次）：租约会被别人抢走 ⇒ 那时必须**立即停止派发**。

### 3.4 降级纪律

| 情况 | 行为 |
|---|---|
| 抢租约**失败**（表不存在 / DB 不可达） | **降级为「照常派发」**（旧行为）+ 记 ERROR。⚠ 若因抢租约失败就静默不派发，等于「租约功能一坏，全站定时任务全停」⇒ 故障放大 |
| 续租**读失败**（DB 抖动） | **保持现状，继续**（不立刻停派发）。一次网络抖动把全站定时任务停掉是故障放大；租约本身有 TTL 兜底 |
| 续租**确认失败**（被接管） | 立即标记非 leader ⇒ **停止派发** |

---

## 4. C：调度器脱离 GUI 进程

### 4.1 实施

| 改动 | 说明 |
|---|---|
| `start_cron_scheduler(app: Option<&AppHandle>, state)` | `AppHandle` 变可选 ⇒ 桌面 / 无头共用一份实现 |
| 新增 `start_cron_scheduler_headless(state)` | 无头入口 |
| `server_main.rs` | 调用 `start_cron_scheduler_headless(&state)` ⇒ **无头服务端跑调度器** |
| `send_desktop_notification_opt(app: Option<AppHandle>, …)` | 新增内部可复用版；`None` ⇒ 降级为日志（结果仍落库）。⚠ **不改** `send_desktop_notification` 签名（它是 `#[tauri::command]`，改了破坏前端契约） |
| 中断分析重跑分支 | 无头路径**跳过**（它靠 AppHandle 反查 managed state，Tauri 专属）+ 记日志 |

### 4.2 无头路径的**两处显式降级**（不静默）

| 降级 | 理由 |
|---|---|
| 荐股桌面通知 → 日志 | 无头路径没有通知通道；**结果仍落库**，用户在应用里能看到 |
| 中断分析重跑 → 跳过 | 依赖 AppHandle 反查 managed state；该任务本质依赖 Tauri 窗口/事件通道，无头服务常驻时桌面侧通常也在跑 |

### 4.3 C 的实际效果

**「应用一关，长任务就腰斩」有了结构性解法**：

| 证据（改造前） | 值 |
|---|---|
| `mover_recall_analysis_batches` 的 `interrupted` | **连续 3 次**（10-23 / 10-24 / 10-25） |
| 消息 | 「应用重启：补分析批次运行中进程退出，as-of 封闭世界无法跨进程续跑」 |

⇒ 调度器宿主不再必须是 GUI 进程 ⇒ **长任务不再随窗口关闭而作废**。

**⚠ 落地前提（尚未做）**：Windows 上让 `axagent-server` 真正常驻（计划任务 / NSSM 服务）。
仓库**零服务化设施**（无 `windows-service` 依赖、无计划任务定义）⇒ 这一步是**运维配置**，不是代码改动。
本轮已把**代码侧的全部前提**备好（`cargo build --features server --bin axagent-server` 可编译 + 调度器可无头启动）。

---

## 5. 附带修复：R-3y（回放 cron 20 日全失败）

### 5.1 根因（实测闭环）

```
15:03:17 mover-recall-analysis-daily 发起 batch c6fac351（running）
        ⇒ 持 state.time_travel_batch_lock（try_lock_owned，进程级）
15:05:17 time-travel-batch-daily 触发 ⇒ 20 个可回放日逐个 run_time_travel_batch_inner
        ⇒ 全部 Err(ALREADY_RUNNING) ⇒ fail_days = 20 ⇒「20 个回放日失败」
```

同锁证据：`mover_recall_analysis.rs:367` 与 `time_travel_batch.rs:731` 用**同一把** `state.time_travel_batch_lock`。

### 5.2 两个缺陷

| # | 缺陷 | 修法 |
|---|---|---|
| ① | **「锁被占」被计为失败** ⇒ 任务中心显示「20 个回放日失败」（实为「本轮没排上」） | `ALREADY_RUNNING` 单独归为 **skip**（新增 `skip_days`），`success` 不受其影响；全被占用时记 `skipped=true` |
| ② | **失败不留批次行** ⇒ 除 `cron_job_history.output` 外无处可查 | skip 计数进入汇总文案（「成功 N / 失败 N / 并发占用跳过 N」） |

### 5.3 未改的失败分类

`live 窗` / `快照覆盖` 等**真失败**仍如实计入 `fail_days`（它们不会被重试解决）⇒ **失败率口径不被"调度拥挤"污染**。

---

## 6. 验证

| 门 | 结果 |
|---|---|
| `cargo check -p axagent --lib` | ✅ 0 error / 0 warning |
| `cargo check -p axagent --lib --features server --bin axagent-server` | ✅ 0 error / 0 warning |
| `cargo test -p axagent-analysis-engine --lib` | ✅ **887 passed / 0 failed**（含 5 条新判据测试） |
| `cargo test -p axagent --lib` | 首跑 **787 passed / 2 failed**（2 个是**既有** R-3v）⇒ 修 R-3v 后复跑 **789 passed / 0 failed** |
| `tsc --noEmit` | ✅ 0 error |
| `dprint check` | ✅ 通过 |
| `check-i18n-key-parity.mjs` | ✅ 11 语言 16243 条 key 集合一致 |
| `check-i18n-untranslated.mjs` | ✅ 新增未翻译 **0**（门禁抓到 2 项韩/阿语残留汉字，已修） |
| `check-hardcoded-i18n.sh` | ✅ 全绿 |
| `check-i18n-placeholder-usage.mjs` | ✅ 干净 |

### 6.1 一处测试断言的自我更正

新写的 `risk_veto_gate_closed_is_byte_identical_to_legacy_behavior` 首版断言「门关时 note 为空」⇒ **红**。
查证后确认是**断言写错、不是代码错**：旧行为在**否决生效**时 note 本就有内容（「高风险风控否决：禁止加仓」），
「note 为空」只成立于「未触发否决」的分支。⇒ 改为比对**三元组**（动作 / 是否降级 / note 逐字一致）。

### 6.2 🔴 一处**死锁**（我自己引入的，已修）

首版用 `RwLock<()>` 做「测试期间冻结门状态」⇒ **写锁内跑 Rhai 脚本** ⇒ 脚本同步调读侧（`is_open`）
⇒ **同线程写锁 + 读锁 ⇒ 死锁**（`RwLock` 写锁不可重入）。

**症状极具误导性**：测试二进制跑 **26 分钟、CPU 仅 207s**（典型「锁等待」而非计算）⇒ 看起来像「测试慢」，
实则必须**杀进程**才能继续。

⇒ **修法**：改用**原子计数器**做冻结标志（无锁 ⇒ 不可能死锁；可嵌套）。
⇒ **新增判据测试** `freeze_makes_reads_report_closed_and_is_nestable` 锁死该行为（防回归）。
⇒ **判据入记忆（RULES #742）**：判「测试卡住」先看 **CPU 时间** —— CPU 远低于墙钟 ⇒ **在等锁**。
（与「cargo 看起来卡死≠卡死，看产物 mtime 是否推进」同族，但**对象是测试进程**。）

### 6.3 测试隔离（防幽灵红）

门状态是**进程级全局**，而种子一致性测试**真的跑 Rhai 脚本** ⇒ 若另一个测试此刻开门，那些
「门把『算不出』当成了『通过』」的断言会**随机变红**（红因与被测代码无关）。

⇒ `decision_gates` 加**原子冻结计数器**：冻结期间读侧一律返回 false ⇒ 脚本无论怎么跑都读到「门关」。

### 6.4 ✅ 关闭 R-3v（既有失败，本轮一并修）

主 crate 测试首跑：**787 passed / 2 failed**，两个失败是**本轮之前就存在**的 R-3v
（`init::stock_automation_cron_seed` 断言内置任务数 13，实得 14）。

**R-3v 的前置条件是「改断言前须独立验证 14 个 task_type 各自在 services.rs 有处置点」** —— 本轮已完成该验证：

| 判据 | 结果 |
|---|---|
| 逐条扫描 14 个 `with_task_type` 实参 | **14/14 在 `services.rs` 有处置点（0 MISS）** |
| 结论 | 断言数字是**纯过期**，不是「有任务没有执行分支」 |
| 处置 | 断言 13 → **14**（3 处）+ 更新演进注释（含本次验证结论） |

⇒ 该测试同时守两半：「加了种子任务却没改锚」会红 + 「忘了在 services.rs 加执行分支」会红 ⇒ **只修一处会让另一半静默通过**。

### 6.5 新增行为判据测试：`decision_gate_switch_reaches_rhai_scripts`

**这条门锁的是「开关真的接到了脚本」**，不是「函数存在」：

| 断言 | 证明 |
|---|---|
| 在**生产同构引擎**（`build_stock_rhai_engine`）上用脚本 `fn _gate_probe(n) { pm_gate_enabled(n) }` 调用宿主函数 | 宿主函数**已注册进共享引擎**（否则 `Function not found` —— R-3d 同族：函数集缺失 ⇒ 整个脚本降级） |
| 冻结态（`with_gates_disabled`）下四个门都报「关」 | 默认行为 = 与引入前一致 |
| 只开风控门 ⇒ **只有它**变 true | 四门**互相独立** |
| 未知门名 ⇒ false | 判错方向**偏保守**（不放行） |

⚠ 编译期修正两处：① 闭包形参须是 `&'static str`（Rhai 对 `&str` 入参要求 `'static`；门键 `Gate::key()` 正好是 `&'static str`）；② 拆分测试函数时留下的重复 import（外层已无用）⇒ 删。
另修正一处**探测写法**：Rhai 的 `fn` 之后**不接受 `_` 开头的名字**（`_` 是丢弃占位符 ⇒ `ParseError(FnMissingName)`）⇒ 改用 `eval` 直接调宿主函数（也更贴近脚本真实调用形态）。

**测试结果**：`decision_gate_switch_reaches_rhai_scripts ... ok`（1 passed / 0 failed）⇒ **门开关确实接到了 Rhai 脚本**。

### 6.6 ⚠ 端到端验证（顶层买入恢复）**未完成** —— 环境阻塞，如实登记

| 环节 | 状态 |
|---|---|
| v188 重播种 | ✅ **已完成**（实测 `workflow_templates.version` 187 → **188**）⇒ 新 Rhai 脚本已落库 |
| 四门加载 | ✅ 生产库四键均为 `true`；启动期从 `settings` 读入进程内快照 |
| 门开关接到脚本 | ✅ **行为判据测试通过**（§6.5） |
| **顶层「买入」实际恢复** | ❌ **未观察到** |

**未观察到的原因（环境，非代码）**：
1. **并发会话持续占用 cargo 构建锁** ⇒ 本轮多次验证被「Blocking waiting for file lock」拖到 20~27 分钟/次；
2. **并发会话反复重启应用** ⇒ 重播种后产出的分析**全部 `interrupted`**（6 次：18:16 / 18:20 / 18:47 …），
   **没有一次跑到产出 `horizon_decisions`** ⇒ 顶层动作无从读。

**⇒ 该项验证的结论只能是「机制已就位、端到端待环境安静后复核」**，不能声称「顶层买入已恢复」。
**⇒ 复核方法（一条命令，环境安静后跑）**：
```sql
-- 期望：重播种后新分析里出现 decision_action='买入'
select count(*) filter (where decision_action = (chr(0x4E70)||chr(0x5165))) as top_buys,
       count(*) as total
from stock_analyses where created_at >= <重播种时刻>;
```

---

## 7. 已知限制（如实登记）

| # | 限制 | 影响 |
|---|---|---|
| 1 | **超短档开仓仍被 P0-5 封顶**（microstructure 腿未接线 ⇒ 开仓动作压回「持有」） | 放开情绪门+风控门后，超短档**仍出不了买入** |
| 2 | **熊市门禁（P0-7）保留**（`bear`/`unknown` ⇒ 买入/增持降持有） | 熊市里顶层仍不出买入 |
| 3 | **校准 cap 仍压置信**（未校准的置信不作面值） | 放开 `tier_buy` 后动作可恢复，但**展示置信仍是封顶值** |
| 4 | **四档校准样本仍 < 30**（short 19 / mid 9 / long 4） | `calibrationReady` 仍恒假 ⇒ 需靠每日积累自然增长 |
| 5 | **C 的 Windows 服务化未做**（计划任务 / NSSM） | 代码侧前提已备好；**不配服务化 ⇒ 无头服务端不会自动常驻** |
| 6 | **D 的效果需跑新分析才可见** | 存量分析结果不会回溯改写；顶层买入数要从**下一次分析**开始观察 |
| 7 | 三门放开**只让 α 可测，不让 α 变正** | 实测 ultrashort(T+2) 净 −1.628 / 胜率 31.7% ⇒ **放开后可能亏得更多**。这是产品决策的风险，已在设置面板文案里显式写明 |
| 8 | **v188 重播种需应用启动一次** | ✅ **已完成**（实测版本 187→188）。⚠ 但重播种后产出的分析**全部被并发会话的重启打断**（6 次 `interrupted`）⇒ **顶层买入是否实际恢复尚未观察到**（见 §6.6） |

---

## 8. 改动文件清单（17 个）

### 新建（4）
```
src-tauri/crates/analysis-engine/src/decision_gates.rs
src-tauri/crates/entities/src/cron_leader_lease.rs
src-tauri/crates/dao/src/repo/cron_leader_lease.rs
src-tauri/src/commands/decision_gates.rs
src/components/settings/DecisionGatesPanel.tsx
```
### 修改（Rust，8）
```
src-tauri/crates/analysis-engine/src/lib.rs                                  （模块声明）
src-tauri/crates/analysis-engine/src/portfolio_formula.rs                   （风控门开关 + 4 条测试）
src-tauri/src/commands/stock_workflow/rhai_pm.rs                           （pm_gate_enabled）
src-tauri/src/commands/portfolio-mgr.rhai                                   （主链四门 + 校准 cap 动作联动）
src-tauri/src/commands/portfolio-mgr-h-long.rhai                           （估值门）
src-tauri/src/commands/portfolio-mgr-h-ultra-short.rhai                    （情绪门）
src-tauri/src/commands/desktop.rs                                          （通知 Option 版）
src-tauri/src/commands/mod.rs                                             （模块声明）
src-tauri/src/register_commands.rs                                         （命令注册）
src-tauri/src/init/services.rs                                            （R-3y + leader 租约 + app Option + 无头入口）
src-tauri/src/init/state.rs                                               （启动期加载四门）
src-tauri/src/server_main.rs                                              （无头调度器）
src-tauri/crates/runtime/src/cron/scheduler.rs                             （leader_check 门）
src-tauri/src/commands/stock_analysis_setup/seed_stock_analysis.rs        （TEMPLATE_VERSION 188）
src-tauri/src/commands/stock_analysis_setup/seed_consistency_tests.rs      （测试隔离 + 拆分内层函数）
src-tauri/crates/dao/src/repo.rs                                           （模块声明）
src-tauri/crates/entities/src/lib.rs                                      （模块声明）
```
### 修改（前端，3）
```
src/components/settings/DecisionGatesPanel.tsx        （新建）
src/components/settings/StockAnalysisSettings.tsx      （新增 gates tab）
src/i18n/locales/*.json  × 11                          （decisionGates 段 + tab.gates）
```
### 辅助（非交付物）
```
.workbuddy/tmp/add_gates_i18n.mjs / add_gates_tab_i18n.mjs / runsql.cjs / *.sql
```
