# PLAN-r2-daily-replay —— 每日时间旅行回放（R-2）

> 状态：active ｜ 日期：2026-11-03 ｜ 对应版本：v3.0.0+（未定版，待确认）


> 2026-10-25 建立。目标：把「回放日」从 3 个推向 30+，让回放样本按天累积。

---

## 1. 出发点与结论修正

R-1 接通回放验证后发现回放样本价格错位（详见 `AUDIT-stock-business-profitability-2026-10-25.md` §11）。
修价后回放与实盘在 T+2 上 α 只差 0.02pp ⇒ **回放是可信的验证通道**，值得用来按天攒样本。

但 R-2 一动手就查出一件更基础的事：

> **`run_time_travel_batch` 这条路从未产出过样本。**

`time_travel_batch_runs` 全表只有 5 行，全是 `as_date=2026-05-27`、`total=0`、
消息「荐股与趋势智选均无候选」，其中 4 条是 `interrupted`（应用重启打断）。

而库里已有的 968 条回放票，`purpose` 全是 `mover_replay` / `legacy_replay`
（`legacy_replay` 是 purpose 列引入前的一次性回填标签），**没有一条是 `time_travel_replay`**。

⇒ R-2 的第一件事不是「调度」，是**先把这条路跑通**。

---

## 2. 可回放日的两条硬约束

### 2.1 建池双源快照必须齐全

`require_pool_snapshot_coverage`（`time_travel_batch.rs`）用
`POOL_SNAPSHOT_METHODS = ["get_hot_stocks", "get_industry_ranking"]` 判定，
**只有 2 个源**。热股榜/行业榜都申报 `NoHistoricalSemantic` ⇒ 早于快照起点的日期**物理上不可回补**。

⚠️ 我第一版用「快照源总数 ≥ 12」判定，覆盖结论因此偏小。**按真实清单重查后是 13 个日期**：

| as_of | hot | ind | 可回放 |
|---|:-:|:-:|:-:|
| 2026-09-28 | ✅ | ✅ | ✅ |
| 2026-09-30 | ✅ | ✅ | ✅ |
| 2026-10-09 ~ 10-23（11 个交易日） | ✅ | ✅ | ✅ |
| 09-04 / 09-07 / 09-09 / 09-24 / 06-11 / 06-10 / 08-18 / 05-27 | ❌ | ✅ | ❌ |

### 2.2 持有期必须已到期

判定 T+N 的前提是 **as_of + N 个交易日 ≤ 今天**。按 `calendar::trading_days_between`：

| 档 | 可判定的 as_of | 个数 |
|---|---|---:|
| **T+2** | 09-28、09-30、10-09 ~ 10-21 | **11** |
| **T+5** | 09-28、09-30、10-09 ~ 10-16 | **8** |
| T+28 | 需等到 2026-11-20 之后 | 0 |
| T+90 | 需等到 2027-02 之后 | 0 |

⇒ **时间旅行加速的是「等待」，不是「快照历史长度」。** 28/90 档的墙是后者，R-2 动不了。

### 2.3 因此调度必须是**每天**跑

「今天能回放哪些日」逐日变化：每个交易日新增 1 个可判 T+2 的 as_of。
一次跑完只能拿到当下这批 ⇒ **必须每日执行**。

---

## 3. 🔴 P0 发现：交易日历会凭空造出假休市日

### 3.1 现象

首轮 `--time-travel` 全量跑：**12 个 as_of 里 11 个被拒**，报错统一是
`2026-10-09 不是 A 股交易日，请选择最近的交易日` —— 而 10-09 在 `market_daily_close` 里
有 5,500+ 只票，是**确凿的交易日**。只有 09-28 通过。

### 3.2 根因

`crates/astock-data/src/calendar.rs` 的 `derive_holidays_from_kline_dates`：

```rust
// 原实现
let mut d = first + Duration::days(1);
while d < today {
    if weekday(d) && !parsed.contains(&d) {
        out.push(d);          // ← 序列里缺失的工作日 ⇒ 一律标成休市
    }
    d += Duration::days(1);
}
```

它把「指数日 K 序列里缺失的工作日」**全部**标成休市，**且尾部空洞没有任何上限**。

而指数 K 线在应用侧是**残缺**的：腾讯 `fqkline` 对 `sh000001` 直接请求返回
**60 根完整序列（2026-07-24 ~ 2026-10-23，含 10-09）**，但东财 `push2his` 被 RST 拦截
（项目注释里早已记着「腾讯 fqkline WAF + push2his RST」），应用最终拿到的是**截断序列**。

序列一旦止于 2026-09-28，**09-30 ~ 10-22 共 18 个真实交易日**被凭空标成休市 ⇒
`is_trading_day` 全部返回 false ⇒ 时间旅行批次拒绝所有 2026-10 的锚点日。

**这不是「回放功能坏」，是「日历把最近一个月说成全休市」。**

### 3.3 影响面比 R-2 大得多

`is_trading_day` 还被这些地方消费：
- **cron 的交易日闸门**（`services.rs`）：休市日跳过荐股/池扫描/反思/回测等任务
- `previous_trading_day` / `nth_trading_day` / `trading_days_between`：as-of 回放的取数定位
- live 流水线的数据新鲜性判据

⇒ 假休市会让**一批定时任务被静默跳过**，且没有任何报错。

### 3.4 修复（`calendar.rs`）

把推导拆成两段 + 两道护栏：

| 段 | 规则 | 理由 |
|---|---|---|
| **内部空洞**（首根 < d ≤ 末根） | 照旧推导 | 序列覆盖了这段区间，缺失即真休市 |
| **尾部空洞**（末根 < d < today） | 仅当 `today − 末根 ≤ 12 天` 才推导 | 超出即「序列滞后」，整段不可信 |

护栏：
- `MIN_BARS_FOR_DERIVATION = 5`：区间极窄时「缺失」更可能是取数不全
- `MAX_INTERNAL_HOLE_RATIO = 30%`：内部缺失率过高 ⇒ 序列稀疏 ⇒ 整段放弃推导

⚠️ **两道护栏分工不同**：拦住 2026-10-25 实测故障的是**尾部空洞上限**
（那次残缺序列有 ~45 根，根数完全正常，是「末根落后今天 27 天」触发的上限）。

⚠️ 尾部可信性以「今天 − 末根」的**整体间距**判定，**不是逐日**判 ——
逐日判会漏掉「离末根近但离今天远」的日子（10-09 距末根 11 天、距今天 16 天，
逐日判会误认为「离今天够近」而把它标成休市）。这个坑我先踩了一次才改对。

### 3.5 回归测试（`calendar.rs`，3 个）

| 测试 | 作用 |
|---|---|
| `complete_series_still_derives_real_holidays` | **正控**：完整序列仍要推出真休市（09-25 / 10-01 / 10-02），且 10-09 绝不被标休市 |
| `stale_series_does_not_fabricate_holidays` | **核心回归**：序列截断到 09-28 时，09-30 / 10-09 / 10-12 / 10-22 都不得被标成休市；但内部真休市 09-25 必须保留 |
| `sparse_series_is_rejected_entirely` | 根数过少 ⇒ 整段放弃推导 |

既有 C3 回归 `kline_derived_calendar_covers_inner_and_tail_gaps`（用 5 根序列测
内部空洞 + 尾部空洞）**首轮被我打破了** —— 我把 `MIN_BARS` 设成 10。
修正为 5 并在注释里写明「拦住实测故障的是尾部上限，不是根数下限」。

---

## 4. 实现

### 4.1 `run_time_travel_batch_inner`（`commands/time_travel_batch.rs`）

命令要 `State<'_, AppState>`，而 **cron handler 与独立 CLI 都拿不到 `State`**
⇒ 按 `run_batch_reflection_inner` / `run_decision_backtest_inner` 的既有范式抽出实现，
**收各个组件**（db / client / engine / vector_store / master_key / trajectory_storage / batch_lock）
而不是整个 `AppState` ⇒ 命令 / cron / CLI **三条路径共用同一实现**，不会分叉。

Tauri 命令降为薄包装。

### 4.2 共享的「今天可回放哪些日」

新增 `list_replayable_as_of_dates(db, min_holding_days)`（命令层）+ 
`astock_daily_snapshot::list_snapshot_dates_with_all_methods`（dao 层）。

**CLI 与 cron 分支共用同一个函数** —— 不在两处各算一遍。
（`missing_snapshot_sources` 只能逐日判断；快照积累到几百天后逐日循环会退化成 N 次查询，
故 dao 层加了「一次列出全部候选日」的查询。）

### 4.3 CLI 模式（`src/bin/axagent-validate-backfill.rs`）

```
axagent-validate-backfill --time-travel [--as-of D,D,...] [--tt-min-confidence N]
                           [--tt-top-n N] [--tt-timeout SEC]
```
- 不给 `--as-of` ⇒ 自动选「快照齐全且 T+2 已到期」的全部日期
- 逐日发批次 → **轮询到终态**（批次是后台执行的，不等终态就报「成功」是假绿）
- 单批超时可配（默认 1800s）

### 4.4 cron 任务（`time-travel-batch`，每日 08:30）

- 新 task_type 常量 `TIME_TRAVEL_BATCH_TASK_TYPE = "time-travel-batch"`
- `services.rs` 加 dispatch 分支：列可回放日 → 逐日发批次 → 轮询终态 → `record_run`
- `stock_automation_cron_seed.rs` 播种（id `time-travel-batch-daily`）
- **进交易日闸门**：休市日「可判定的 as_of 集合」与上一交易日完全相同 ⇒ 休市日跑是纯浪费
- 时刻 08:30：避开 live 流水线时间窗（23:50-02:10 / 17:50-18:10）

---

## 5. 实测记录

### 5.1 首轮（全量 12 日）—— 被日历缺陷挡住

| as_of | 结果 |
|---|---|
| 2026-09-28 | **批次实际跑起来了**（>600s 未终态，日志显示 agent 节点在执行） |
| 其余 11 个 | ❌ `不是 A 股交易日`（日历缺陷） |

### 5.2 日历修复后 —— 批次确实产出样本

```
as_of_date  purpose                n   real  first_at
2026-09-28  time_travel_replay    20   20   2026-10-25T18:25:43
```

**`purpose='time_travel_replay'` 首次出现** ⇒ 这条链此前从未跑通，现在跑通了。

耗时特征：选股阶段很快（1 分钟内落 20 条真实票），**分析阶段慢**（逐只跑 LLM 工作流）。
这决定了「每日 +1 个回放日」可行，但**一次性回补 12 天需要数小时**。

---

## 6. 剩余待办

| # | 项 | 优先级 | 说明 |
|---|---|---|---|
| R2-1 | 跑完整回补（11 个 T+2 可判日） | P0 | 需数小时后台运行 |
| R2-2 | 回放 pick 跑验证（`--include-replay`）+ 价源平价校验 | P0 | R-3a 已就位 |
| R2-3 | 确认 cron 分支在应用内可跑（需重启应用播种） | P1 | 未验证 |
| R2-4 | `time_travel_batch_runs` 里的 `running` 僵尸行需收敛 | P2 | CLI 退出即打断 ⇒ 留 `running` 行 |
| R2-5 | 指数 K 线取数改用腾讯直连（东财 push2his 被 RST） | P1 | 日历数据源本身不可靠，是缺陷的源头 |
| R2-6 | 智选链在回放里 0 买入 / 26% 数据缺失 | P1 | R-3c，与本条独立 |

---

## 7. 首轮回补实测（2026-10-25 晚）

### 7.1 日历修复后：批次被接受，链路跑通

| as_of | 结果 |
|---|---|
| **2026-10-21** | ✅ 被接受（此前被日历拒绝），产出 **19 条真实票**（`purpose='time_travel_replay'`） |
| 2026-09-28 | ✅ 产出 20 条真实票 |

**`purpose='time_travel_replay'` 首次出现** ⇒ 这条链此前从未跑通，现在跑通了。

### 7.2 耗时结构：选股快，分析慢

| 阶段 | 耗时 | 说明 |
|---|---|---|
| 选股（guard 内） | **< 1 分钟** | 产出 `reco_picks`（19~20 条真实票） |
| 分析（逐只跑 LLM 工作流） | **30~60 分钟** | 产出 `stock_analyses`（`analysis_kind='replay'`） |
| 反思（guard 外） | 数分钟 | 产出 `outcome` |

**⇒ 盈利能力样本不依赖分析阶段** —— 那条路走 `reco_picks → decision_validations`，
只用 K 线算收益。分析阶段喂的是 `stock_analyses.outcome`（决策质量口径）。

### 7.3 🔴 CLI 设计缺陷（已修）：超时后进入下一日会被锁拒掉

批次是**后台执行**的（命令只返回 `batch_id`）。若 CLI 的等待上限先到而直接进入
下一个 as_of 日，新批次会撞上 `time_travel_batch_lock`（`try_lock_owned`，**不排队**）
⇒ 被拒 `ALREADY_RUNNING` ⇒ **该日被整个跳过**。

而「跳过」是最坏的形态：它长得像「那天气数不足」，实际是「上一批还没跑完」。

**修法**（`axagent-validate-backfill.rs`）：
- 新增 `wait_lock_free(lock, max_wait)`：轮询 `try_lock_owned`，拿到即释放（只确认空闲）
- 发起被 `ALREADY_RUNNING` 拒 ⇒ 等锁空闲后**重试当日**（最多 3 次），不是跳到下一日
- 等待超时 ⇒ 先等锁释放再进下一日

### 7.4 与 D-2（period 值域/窗口错配）的交互

10-21 的 19 条票是 **`short` 档**（持有 5 日），但今天只成熟到 T+2 ⇒
落库的验证行是 `period='short' + t_plus_n=2` 的**错配行**，
按期窗匹配（short↔5 / ultrashort↔2）**被排除 ⇒ 今天贡献 0 条可用样本**。

这正是「逐日推进」的机制本身：10-21 的 short 样本在 **3 个交易日后**（10-28）才可用。

⇒ **回放样本的可用性有个「成熟延迟」**：as_of 日当天跑出来的票，
要等 `period` 对应的持有期到期才进分母。short 档延迟 5 日，ultrashort 档延迟 2 日。

### 7.5 当前代际拆分读数

| 代际 | n | 毛 | **净** | 净胜率 | 标准差 | **回放日数** |
|---|---:|---:|---:|---:|---:|---:|
| 实盘 LIVE | 311 | −0.47 | **−0.72** | 37.0% | 10.66 | 11 |
| **回放 REPLAY** | **190** | +0.31 | **+0.06** | **58.4%** | 6.93 | **3** |
| 合计 | 501 | −0.18 | −0.43 | 45.1% | 9.43 | 14 |

回放分日明细：

| as_of | 窗口 | n | 毛 | 净 | 胜率 |
|---|---|---:|---:|---:|---:|
| 2026-06-11 | T+5 | 20 | +2.70 | +2.45 | 55% |
| 2026-09-04 | T+2 | 10 | +2.27 | +2.02 | 50% |
| 2026-09-04 | T+5 | 11 | +5.82 | +5.57 | 64% |
| 2026-09-28 | T+2 | 68 | −0.73 | −0.98 | 51% |
| 2026-09-28 | T+5 | 81 | −0.41 | −0.66 | 65% |

**回放净胜率 58.4% vs 实盘 37.0%**，但**只有 3 个回放日** —— 3 个独立观测，
谈「回放更好」为时过早。且 09-04 那一档（n=21）是唯一显著为正的，
其 α 主要来自单日批次（§11.5 已量化：09-04 批 α +8.05）。

⇒ **R-2 的价值在于把回放日从 3 个推到 30+**，而不是现在这三天的读数。

### 7.6 验证写库

```
report.total   : 381   written_count : 723   skipped_count : 170
```
723 行回放验证行落库（`as_of_date` 非空，与实盘行可分）。
按 as_of 分布：06-11 40、09-04 84、09-28 580、**10-21 19**（新）。

---

## 8. 验证状态

| 项 | 结果 |
|---|---|
| `cargo check --bin axagent-validate-backfill` | ✅ `Finished`（0 error / 0 warning） |
| `cargo check --lib`（cron 分支） | ✅ `Finished` 8m16s，0 error / 0 warning |
| `cargo test -p axagent-astock-data --lib` | ⚠ **652 passed / 1 failed** —— 失败的是 `error::tests::transient_is_limited_to_rate_limit_connect_and_timeout`（`error.rs:226`，连 `127.0.0.1:1` 期望连接失败却拿到 502）。**与本次改动无关**：本轮在 astock-data 只动了 `calendar.rs` |
| `cargo test -p axagent-analysis-engine --lib` | ✅ **869 passed / 0 failed** |
| `cargo test -p axagent-astock-data --lib calendar`（本次改动相关） | ✅ **22 passed / 0 failed** |
| `cargo clippy -p axagent-astock-data --all-targets -- -D warnings` | ✅ `Finished` 2.65s |
| `cargo clippy --bin axagent-validate-backfill -- -D warnings` | ✅ `Finished` 8m56s（修掉 3 处） |
| `cargo clippy -p axagent-dao --all-targets -- -D warnings` | ✅ `Finished` 13m37s |
| `scripts/check-cron-job-authority.mjs` | ⚠ R2 红：`time-travel-batch-daily` 库里还没有行 ⇒ **应用尚未重启播种**（脚本自身说明：这是「还没重启播种」的真实状态，不是判据坏） |

### 8.1 门禁缺口（我的验证疏漏）

本轮 clippy 首次覆盖 bin 时报了 **3 处 lint**，全部来自**我今天早些时候写的代码**
（`--target-days` 的范围校验、因子回填的价格守卫）——
此前几轮我只对 `analysis-engine` 跑了 clippy，没跑 bin。

已修：
- `!(30..=500).contains(&n)`（`RangeInclusive::contains`）
- 价格守卫改 `!x.is_finite() || x <= 0.0`（原 `!(x > 0.0)` 对 NaN 成立、对 +INF 不成立）

> **教训**：bin 也是交付物，clippy 门禁必须覆盖；
> 「`cargo clippy -p <某个 crate>` 通过」不等于「我改的东西都干净」。

---

## 9. 改动文件清单

| 文件 | 改动 |
|---|---|
| `crates/astock-data/src/calendar.rs` | **P0 修复**：尾部空洞上限 + 两道护栏 + 3 个回归测试 |
| `crates/dao/src/repo/astock_daily_snapshot.rs` | 新增 `list_snapshot_dates_with_all_methods` |
| `src/commands/time_travel_batch.rs` | 抽出 `run_time_travel_batch_inner`（收各组件）；新增 `list_replayable_as_of_dates`；新增 `TIME_TRAVEL_BATCH_TASK_TYPE` |
| `src/lib.rs` | 再导出 inner / 请求结构 / task_type / 可回放日查询 |
| `src/bin/axagent-validate-backfill.rs` | 新增 `--time-travel` 模式（含 `wait_lock_free` 等锁重试）；修 3 处 clippy |
| `src/init/services.rs` | 新增 `time-travel-batch` dispatch 分支；该 task_type 进交易日闸门 |
| `src/init/stock_automation_cron_seed.rs` | 播种 `time-travel-batch-daily`（每日 08:30） |

---

## 10. 完整回补结果（关闭分析后）

### 10.1 🔴 分析阶段慢的真正原因：**LLM 供应商限速**

第一次全量跑时日志里全是：

```
LLM_CALL_FAILED: OpenAI API error 429: 您已达到免费用户的 API 速率限制（AgnesAI）
```

逐只 replay 分析要跑 LLM 工作流，节点重试 + fallback（`agnes-3.0-flash` → `agnes-2.5-flash`）
**全被限速拒掉**，于是每个 as_of 日拖 30~60 分钟。

⇒ 这不是本地计算慢，是**免费档配额**问题。
⇒ 而**盈利能力样本不依赖该阶段**（走 `reco_picks → decision_validations`，只用 K 线）。

**新增 `TimeTravelBatchRequest::analyze`（默认 true 保持既有语义）**，
CLI / cron 显式传 false ⇒ 只跑选股链，候选落 `reco_picks` 后即结束。

**实测速度：30~60 分钟/日 → 2~4 分钟/日**（约 15 倍）。
候选在选股阶段就已落库 ⇒ 关闭分析**不丢任何盈利能力样本**。

### 10.2 🔴 连接池饥饿：把**成功的批次**误报成失败

| 现象 | 数字 |
|---|---|
| 首轮（分析开着） | 12 个日期里 10 个被 `Connection pool timed out` 拒掉 |
| 轮询侧 | `读批次失败: Connection pool timed out` ⇒ CLI 报失败，但批次其实成功产出了票 |

根因：**批次的选股链逐只并发取数把 20 连接的池占满**
（`dao/src/db.rs` `max_connections=20` / `acquire_timeout=15s`），
同进程里的轮询者（`fetch_batch_by_id`）拿不到连接 ⇒ 15s 超时 ⇒ **成功批次被记成失败**。

修法（两处）：
- `dao/src/db.rs`：`max_connections` **20 → 32**、`acquire_timeout` **15s → 30s**
  （PG 默认 `max_connections=100`，32 安全；注释里已有 8→20 的同类先例）
- CLI 轮询：连接池错误**重试**（≤20 次）而不是直接判失败

### 10.3 🔴 我自己造成的**假失败状态**（已修）

关闭分析后 `s.total = 候选数 > 0` 而 `s.ok = 0`（没有分析成功数）⇒
撞上终态判定里的「`total > 0 && ok == 0` ⇒ 全部候选分析失败」分支 ⇒
**一次成功的选股批次被判成 `failed`**（实测 12 条误判）。

修法：终态判定里先判 `!analyze` ⇒ 直接 `completed`，并在 message 里写明
「按请求跳过逐只 replay 分析：候选已落 reco_picks」。

> **教训**：「跳过某阶段」与「该阶段失败」在计数器上是同一个形状
> （`total > 0 && ok == 0`）⇒ 必须显式区分，否则成功会被记成失败。
> 与「恒等/恰 100% 先怀疑测量工具」同族。

### 10.4 完整回补产出：11 个回放日

| as_of | 真实票 | as_of | 真实票 |
|---|---:|---|---:|
| 2026-09-28 | 74 | 2026-10-13 | 10 |
| 2026-09-30 | 10 | 2026-10-14 | 10 |
| 2026-10-09 | 7 | 2026-10-15 | 20 |
| 2026-10-12 | 20 | 2026-10-16 | 10 |
| 2026-10-21 | 29 | 2026-10-19 | 10 |
| | | 2026-10-20 | 10 |

（09-28 的 74 条含多轮运行的重复；业务键去重后更少。）

### 10.5 🔴 样本扩大 → 读数再次恶化（与实盘那次同型）

| 阶段 | 回放 n | 回放日 | 净均值 | 净胜率 |
|---|---:|---:|---:|---:|
| 回补前 | 190 | 3 | **+0.06** | 58.4% |
| **回补后** | **278** | **10** | **−0.95** | **51.1%** |

新增 7 个回放日**全部为负**：

| as_of | n | 净 | 胜率 |
|---|---:|---:|---:|
| 2026-09-30 | 10 | −3.65 | 30% |
| 2026-10-09 | 7 | −3.34 | 14% |
| 2026-10-12 | 10 | −6.31 | 20% |
| 2026-10-13 | 10 | −6.77 | 20% |
| 2026-10-14 | 10 | −4.72 | 20% |
| 2026-10-15 | 10 | −4.35 | 30% |
| 2026-10-16 | 10 | −0.56 | 30% |

**09-04 那批（+5.57，n=11）是小样本乐观** —— 与实盘那次「127 单 +0.47% → 311 单 −0.72%」完全同型。

### 10.6 ✅ 配对 alpha：回放与实盘**互相印证**（最终结果）

| 代际 | 窗口 | n | 回放日 | 个股 | 市场 | **配对 α** | α>0 |
|---|---|---:|---:|---:|---:|---:|---:|
| 回放 | T+2 | 77 | 3 | −0.44 | +0.62 | **−1.06** | 44% |
| 回放 | T+5 | 199 | **10** | −0.92 | −1.45 | **+0.52** | 59% |
| 实盘 | T+2 | 120 | 5 | −1.38 | +0.10 | **−1.48** | 34% |
| 实盘 | T+5 | 188 | 7 | +0.18 | +0.31 | **−0.13** | 42% |

**T+2 两路差 0.42pp，T+5 差 0.65pp，两路符号方向一致，α 都为零或负。**

⇒ **R-2 的最终结论**：时间旅行**确实**能把回放样本按天积累（10 个回放日、每 2~4 分钟一个），
而且**修价后回放与实盘互相印证** ⇒ 这是对「度量可信」的最终确认。
但它**不改变结论**：α ≈ 0（略负），两路一致。

---

## 11. 批次状态的数据更正

§10.3 那个「跳过分析被误判 failed」的缺陷在库里留下 17 条错误记录。
修正依据是**记录自身的字段**（`total > 0` 且 `fail = 0` ⇒ 选股链成功）
+ `request_json` 里 `"analyze":"false"` ⇒ 该批本就打算跳过分析。

```sql
-- 备份
CREATE TABLE time_travel_batch_runs_bak_20261025 AS SELECT * FROM time_travel_batch_runs;  -- 25 行
-- 更正（只动「跳过分析且选股成功」的行）
UPDATE time_travel_batch_runs
SET status='completed', error_code=NULL, error_detail=NULL,
    message=coalesce(message,'') || ' (corrected: analysis skipped by request => not a failure)'
WHERE status='failed' AND (request_json::jsonb->>'analyze')='false' AND total>0 AND fail=0;
```

**UPDATE 17** ⇒ 更正后：`completed 18 / interrupted 7`。
剩下的 7 条 `interrupted` 是**真实记录**（我 kill 掉的进程留下的），不改。

回滚：`DELETE FROM time_travel_batch_runs; INSERT INTO time_travel_batch_runs SELECT * FROM time_travel_batch_runs_bak_20261025;`

---

## 12. 记忆层同步

| 项 | 结果 |
|---|---|
| `mem.mjs check` | ✔ 注入层 **7140 / 7168 B**（余 28 B）——本轮为加「股票线」指针压缩了 26 B（判据一条未删） |
| 新增判据 | **#718 缺上限的派生量 ⇒ 凭空造出假事实**（D 组）｜**#719 并发取数占满连接池 ⇒ 观测者饿死 ⇒ 成功被记成失败**（D 组）｜**#720 「跳过某阶段」与「该阶段失败」在计数器上同形**（E 组）｜**#721 样本扩大 ⇒ 读数恶化是可重复的模式**（A 组） |
| 召回触发词 | 注入层已挂：`缺上限派生`·`池饥饿`（D）｜`跳过≠失败`（E）｜`扩样恶化`（A） |

---

## 12. R-3c 定性：回放分析链的「0 买入 / 26% 数据缺失」

> 触发：`AUDIT-stock-business-profitability-2026-10-25.md` §缺陷表第 2 行
> 「分析链降级｜买入率 0% vs 实盘 7.0%；数据缺失 26%｜回放测的不是同一个系统」
> 与 §缺陷表 R-3c「修回放分析链的 as-of 数据缺口（0 买入 / 26% 数据缺失）」。

### 12.1 🔴 先纠正口径：代际判据是 `analysis_kind`，**不是 `as_of_date`**

`stock_analyses.as_of_date` 在 **live 行也非空**（live 行的 `as_of_date` = 该次分析当天，
`run_single_stock_analysis` 无条件写 `Set(Some(anchor_date))`）。
⇒ 用 `as_of_date IS NULL` 判代际会把 **live 行全判成回放**，读数全错。

正确判据：`analysis_kind ∈ {live, replay}`（`AnalysisKind::from_asof(asof_ctx.is_some())`）。

| 代际 | kind | source | n | as_of 日数 | 买入 | 数据缺失 | 空 action |
|---|---|---|---:|---:|---:|---:|---:|
| 实盘 | live | live | 209 | 34 | **14（6.7%）** | 3（1.4%） | 9 |
| 回放 | replay | time_travel | 94 | 9 | **0** | 33 | 3 |
| 回放 | replay | mover_recall | 43 | 9 | **0** | 0 | 0 |
| 回放 | replay | live | 6 | 5 | 0 | 1 | 0 |

### 12.2 ✅ 26% 数据缺失的根因：**四个独立成因，不是 as-of 数据缺口**

回放 34 条「数据缺失」逐条归因（判据 = reasoning 里的错误原文）：

| 根因 | n | 涉及日 | 性质 |
|---|---:|---|---|
| **R1 LLM 供应商 429 限速** | **30** | 10-14、10-15 | **环境**（AgnesAI 免费档配额） |
| R4 子工作流失败（`a-catalyst--ultra_short` / `pm-h-ultra-short`） | 2 | 07-22、10-14 | 环境（并发下游被跳过） |
| R2 Rhai 宿主函数未解析（**整个函数集缺失**，见 §12.6） | 1 | 09-28 | **代码缺陷（活跃）** |
| R3 工作流变量 `horizon_consts_json` 缺失 | 1 | 10-15 | 已随模板换代消失 |

**R1 占 88%** ⇒ 「26% 数据缺失」**不是「as-of 取不到数据」**，是供应商限速。
这与 R-2 阶段发现的「分析阶段慢 = 429」是**同一个根因**（当时只当性能问题，
没意识到它同时污染了决策样本的 24%）。

### 12.3 🔴🔴 「0 买入」的真正成因：**模板 v169 的多档分支换代，与 as-of 无关**

判别检验（`主档=` 标记是「多档分支口径」的指纹）：

| kind | 标记 | n | 买入 | 持有 | 观望 |
|---|---|---:|---:|---:|---:|
| live | 有 `主档=` | 55 | **0** | 32 | 20 |
| live | 无 `主档=` | 130 | **11** | 45 | 36 |
| replay | 有 `主档=` | 85 | **0** | 62 | 20 |
| replay | 无 `主档=` | 55 | **0** | 14 | 2 |

**带 `主档=` 标记的行在两代都是 0 买入** ⇒ 买入与多档分支口径互斥。

时间边界：
- `主档=` 标记**最早出现在 2026-10-18**（template_version=169）
- live 的 14 条买入**全部发生在 10-18 之前**（tv 93~156）

**⇒ 10-18 之后两代全部 0 买入**（live 57 行 / replay 48 行，live 也是 0）：

| as_of | n | 买入 | 有 `主档=` |
|---|---:|---:|---:|
| 10-16 | 2 | 0 | 2 |
| 10-17 | 2 | 0 | 0 |
| 10-18 | 5 | 0 | 4 |
| 10-19 | 41 | 0 | 41 |
| 10-20 | 11 | 0 | 11 |
| 10-21 | 31 | 0 | 24 |
| 10-22 | 16 | 0 | 15 |
| 10-24 | 1 | 0 | 1 |

**这是模板演进的副作用，不是 as-of 缺陷。** 原登记把「模板换代后买入消失」
误判为「回放降级」。

### 12.4 ✅ 控制模板版本后的最终对照：两代**几乎一致**

只取 `template_version >= 169`（同一套模板口径）：

| kind | n | 买入 | 持有 | 观望 | 卖出 | 减持 | 数据缺失 | 空 | 仓位均值 |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| live | 55 | 0 | 32 | 20 | 3 | 0 | 0 | 0 | **5.38** |
| replay | 99 | 0 | 73 | 19 | 3 | 3 | 1 | 0 | **6.32** |

**⇒ 原命题「回放测的不是同一个系统」在分析链上证伪**：
控制模板版本后，决策分布逐项吻合（持有/观望/卖出几乎同比例），仓位均值接近。

### 12.5 残余差异只有一处：**LLM 限速造成的缺失率**

| kind | n | 缺失或空 | 占比 |
|---|---:|---:|---:|
| live | 55 | 0 | **0%** |
| replay | 99 | 4 | **4.0%** |

且这 4 条里 0 条是 429（v169 后）⇒ 限速影响已大幅下降。
**残余 4.0% 是可接受的降级率，不是「系统不同」。**

### 12.6 🔴 唯一需要修的活跃缺陷：**CLI 路径缺 AxInvest 宿主函数集**

> ⚠ 本节结论在实测中**被推翻过一次**（先判成「`pm_leg_signal` 签名不匹配」，
> 读 Rhai 1.26 源码逐一排除了全部签名因素后仍解释不通 ⇒ 改为直接查证据，
> 才找到真因）。推翻过程本身是判据，下面保留。

#### 12.6.1 最初的两个假设（都被实测排除）

| 假设 | 排除依据 |
|---|---|
| 「`pm_leg_signal` 的 `&str` 参数不接受字面量」 | Rhai `func/register.rs:51 by_value` 有 `&str` **专门分支**；`module/mod.rs:1546 map_type` 把注册的 `&str` **归一化为 `ImmutableString`** ⇒ 与实参 TypeId 一致 |
| 「参数类型哈希不匹配」 | `func/hashing.rs:139 calc_fn_hash_full` 用 TypeId，但实参侧 `calc_fn_hash` **只含名字 + 参数个数** ⇒ 不因 `&str`/`ImmutableString` 分叉 |

⇒ 源码层全部排除 ⇒ **必须换证据源**，不能继续推断。

#### 12.6.2 决定性证据：**零参数闭包也报 not found**

回查跑批日志（`.workbuddy/tmp/tt3.log`）里的 `Function not found` 全量：

```
      1 Function not found: branch_row_ok
     32 Function not found: pm_asof_degraded_methods
```

`pm_asof_degraded_methods` 的注册形态是 **`register_fn("pm_asof_degraded_methods", || -> String)`**
—— **零参数闭包** ⇒ **与签名完全无关**。

⇒ 只有一种解释：**那个 Engine 里根本没有 AxInvest 宿主函数**。

日志现场：

```
ERROR code_executor: Rhai 执行失败 error=Function not found: pm_asof_degraded_methods () (line 863, position 24)
WARN  engine: Node failed node_id=data-quality error=VALIDATION_FAILED: Rhai execution failed: …
```

#### 12.6.3 真因：注册点只在 **Tauri 应用启动路径**上

| 入口 | 构造 AppState | 是否注册宿主函数 |
|---|---|---|
| Tauri 应用 | `create_app_state` → `start_background_services` | ✅ |
| `axagent-validate-backfill`（CLI） | `create_app_state` | ❌ **缺** |
| `axagent-batch-rerun`（CLI） | `create_app_state` | ❌ **缺** |
| `server_main`（无头服务） | `create_app_state` | ❌ **缺** |

`register_shared_engine_initializer` 的**唯一**调用点此前在
`src/init/services.rs:35` 的 `start_background_services` 里 —— 那是 Tauri 专属路径。
共享 Engine 是进程级 `OnceLock`，**在首次创建时取走注册队列并冻结**；
CLI 从没注册过 ⇒ 队列是空的 ⇒ Engine 里**一个 `pm_*` 都没有**。

#### 12.6.4 后果：**回放分析链测的确实是降级版系统**（原命题在这一层成立）

脚本里的 `pm_*` 调用报 not found 后被 `try/catch` 吞掉 ⇒ 走保守兜底路径：
`data-quality` 节点 **VALIDATION_FAILED**、portfolio-mgr 降级为「数据缺失」。

⇒ **R-2 阶段用 CLI 跑出来的那 173/278 条回放分析样本，全部来自降级版系统。**
⇒ **盈利能力读数不受影响**（那条路只走 `reco_picks → decision_validations`，纯 K 线），
但**决策质量口径的读数全部无效**。

#### 12.6.5 修复（3 处）

| # | 文件 | 改动 |
|---|---|---|
| 1 | `commands/stock_workflow/rhai_registry.rs` | 新增 `ensure_shared_engine_has_axinvest_functions()`（`Once` 幂等） |
| 2 | `init/state.rs` `create_app_state` | 在**所有路径的共同入口**调它 ⇒ 一处覆盖 Tauri / CLI ×2 / server_main |
| 3 | `init/services.rs` `start_background_services` | 旧注册点改为调同一个幂等入口（保留兜底；`Once` 保证不重复入队） |

**时机校验**：`create_app_state` 早于任何工作流执行（CLI：`create_app_state` → 跑批；
Tauri：`create_app_state` → `start_background_services`）⇒ 共享 Engine 冻结前注册成功。

#### 12.6.6 回归测试

| 文件 | 判据 |
|---|---|
| `tests/shared_engine_host_functions_registered.rs` | **真调用**共享 Engine 上的 4 个 `pm_*`（零参/一参/两参/主决策路径）+ **负对照**（未注册名必须解析失败） |
| `crates/rt-workflow/tests/pm_leg_signal_rhai.rs` | 隔离复现 `pm_leg_signal` 的两种调用形态（字面量 / 变量）+ 负对照 |

**判据用真调用而非查注册表**：Rhai 编译期不校验未知函数名，只 compile 的门禁
看不见「函数未注册」—— 这正是本缺陷逃过既有静态门禁的原因
（`scripts_only_call_registered_fns` 只查**函数名**是否出现在注册点源码里）。

#### 12.6.7 ⚠ 一条重要的方法论教训

我一度用 `j->>'code'` 取节点脚本正文（**错**，实际在 `j->'config'->>'code'`）
⇒ 曾得出「DB 模板不含 `pm_leg_signal`」的**假结论**，并据此推断
「v185 是未播种的新模板 ⇒ 报错来自未播种版本」。

**字段路径错误会造出看似合理的假结论。** 修正后的事实是：
DB tv=184 的 `portfolio-mgr` 脚本与工作树 v185 的该段**逐字一致**
（hex 导出比对），`let f12_raw = pm_leg_signal(` 两边都在第 **1714** 行；
报错行号 1712 是 Rhai 按 token 位置换算的偏移）。

⇒ **取证阶段的每个字段路径都要先用一个已知值验证**，
否则「查不到」会被读成「不存在」—— 这与「0 命中先证会告警」同族。

`template_version=184`（当前最新版）仍复现：

```
portfolio-mgr Rhai 执行异常，已降级为保守决策:
ErrorFunctionNotFound: Function not found: pm_leg_signal (&str | ImmutableString | String, map)
```

**为什么静态门禁漏掉它**：`rhai_registry.rs` 的 `scripts_only_call_registered_fns`
只查**函数名**是否出现在注册点源码里 —— `pm_leg_signal` 确实注册了 ⇒ 假绿。
运行期是否可解析**取决于实参类型**，静态判据看不到这一层
（同 `band_for_score` 那次的教训：只查名字会漏签名不匹配）。

**修复方向**：`register_fn` 的 `&str` 参数在 Rhai 1.26 只接受特定形态；
`portfolio-mgr.rhai:1714` 传的是**字符串字面量 + map 字面量**，
而 `:551` 传的是**变量 + map 字面量** ⇒ 前者的实参类型组合未注册。
需在宿主侧补一个覆盖该形态的注册（或改脚本传参形态）。

### 12.7 R-3c 结论

| 原命题 | 判定 |
|---|---|
| 「回放分析链 0 买入 ⇒ 回放测的是降级版」 | ❌ **0 买入本身证伪**：模板 v169 换代所致，两代同时 0 买入；控制版本后分布一致 |
| 「26% 数据缺失 ⇒ as-of 数据缺口」 | ❌ **证伪**：88% 是 LLM 供应商 429 限速（环境），非数据缺口 |
| 「回放测的不是同一个系统」 | ✅ **成立，但机制与原判断不同**：不是 as-of 数据缺口，而是 **CLI 路径缺 AxInvest 宿主函数集** ⇒ 共享 Engine 里一个 `pm_*` 都没有 ⇒ 脚本调用被 `try/catch` 吞掉走保守兜底 |
| 「回放分析链有活跃代码缺陷」 | ✅ **成立**：宿主函数注册点只在 Tauri 路径（§12.6） |

⇒ **R-3c 从「修 as-of 数据缺口」改为「修 1 处活跃缺陷 + 登记 2 项治理动作」**：
1. ✅ **已修**：宿主函数注册下沉到 `create_app_state`（所有路径共同入口），
   补 `ensure_shared_engine_has_axinvest_functions()`（`Once` 幂等）
   + 2 个真调用回归测试
2. **治理 R1**（429）—— 已在 R-2 阶段做过 `analyze=false` 开关（关掉逐只分析）；
   但**若要保留分析链样本**，需解决限速（换供应商/降并发/退避重试）
3. **登记 R-3d**：静态门禁从「查名字」升级为「真调用」——
   `scripts_only_call_registered_fns` 假绿的本质是**用名字代理可解析性**；
   同理「某路径是否注册了宿主函数」也不能靠读代码判断，**必须真调用**
4. 🔴 **R-2 的回放分析样本需重跑**：那 173/278 条来自降级版系统
   （盈利能力读数不受影响，因为它只走 `reco_picks → decision_validations`）

---

## 13. R-3d：「荐股链 73~95% 走兜底合成」的定性 —— **该比例是历史存量；真问题是档位被压平**

> 触发：用户提问「荐股链为什么 73~95% 走兜底合成的任务还没有实施？」

### 13.1 「73~95% 兜底合成」是**历史存量**，10-22 之后已消失

| purpose | n | synthetic | 占比 |
|---|---:|---:|---:|
| `legacy_replay` | 21109 | 20155 | **95.5%** |
| NULL（启动期回填标签） | 4788 | 3969 | **82.9%** |
| `mover_replay` | 380 | 360 | **94.7%** |
| **`time_travel_replay`**（10-25 跑的） | 230 | **0** | **0.0%** |

**时间线**：合成票全部产生于 **10-17 ~ 10-22**；`reco_pool_batches` 表最早只到
10-22（`degraded=0`、388 只股票）；10-25 我跑的批次 `degraded=0`、65~117 只。

⇒ **该问题事实上已被 10-19 的两处修复解掉**：
`asof_quote_is_usable`（成交额守卫）+ `estimated_amount`（价×量推定成交额）。

### 13.2 ⚠️ 但「库里 0% 合成」**不能**当作「没走兜底」的证据

`run_recommendation_scan` 步骤 3 是 `filter(!p.synthetic)` ⇒
**兜底合成票被过滤掉才落不了库** ⇒ 库里 0% 合成是**必然**。

⇒ 判「是否走兜底」要看 `recommender/mod.rs` 的日志行
`style=X bucket was empty, filled with N synthetic picks`
（10-25 的 `tt_full5.log` 里该行 **0 次** ⇒ 那批确实没走兜底）。

### 13.3 🔴🔴 真正的当前缺陷：**档位被跨档去重压平成单档**

实测（`purpose='time_travel_replay'`）：

| period | picks | distinct stocks |
|---|---:|---:|
| **short** | **230** | **58** |

**100% 落在 `short` 一档**，其余 3 档（ultra_short/mid/long）的票全被吃掉。
对照 `mover_replay`（走**每档独立落库**路径）有 **4 个档位**（long 40 / mid 60 / short 50 / ultra_short 40）。

**根因（3 层叠加）**：

| 层 | 位置 | 机制 |
|---|---|---|
| ① | `recommender/notify.rs` `run_recommendation_scan` 步骤 2 | **跨 period 按 `stock_code` 去重**（留 confidence 最高）⇒ 同一只票在多档命中时只保留一档 ⇒ **档位语义丢失** |
| ② | 同函数 步骤 5 | `top_n` 截断 |
| ③ | `commands/time_travel_batch.rs` 链 A 落库 | 用 `scan.picks`（= 去重后）⇒ **回放落库口径 = 通知口径** |

⇒ **通知口径是对的**（推给用户「这几只票」，同一只票推四遍是噪声）；
**落库口径是错的**（回放的目的是「当时各档分别判了什么」）。

⇒ 后果：反思按档分档、命中率按档统计、`decision_validations` 的
`period ↔ t_plus_n` 匹配**全部失真**（short↔5 / ultrashort↔2 的期窗匹配）。

### 13.4 ✅ 修复（3 处）

| # | 文件 | 改动 |
|---|---|---|
| 1 | `analysis-engine/src/recommender/notify.rs` | `RecommendationScan` 增 `picks_by_period: Vec<(Period, Vec<RecoPick>)>`；新增纯函数 `real_picks_of_period(by_style, min_confidence)`（**唯一**实现点：剔 synthetic + 过阈值，**不去重、不截断**） |
| 2 | `commands/time_travel_batch.rs` | 链 A 落库改用 `scan.picks_by_period` 展平；`scan.picks`（通知口径）**不动** |
| 3 | 同文件测试 | 3 个单测（核心回归 + 2 个负控） |

**落库无冲突**：`persist_reco_picks` → `insert_picks` 用随机 uuid 逐行插，**不去重**
⇒ 同票多档可各落一行。

### 13.5 顺带查清的 as-of 取数链（后续参考）

- `get_quote` 在 as-of 下**只从 K 线合成**（不回退 vendor 实时，避免时间泄露）
- as-of 的 K 线路由：`tencent → eastmoney_push2 → browser_eastmoney → xueqiu`
- `tencent` 日线**只有 6 字段** `[日期,开,收,高,低,成交量]` ⇒ **无成交额**
  ⇒ 触发 `asof_quote_is_usable`（成交额>0）⇒ 落 `amountless` 分支
  ⇒ 由 `estimated_amount(price, volume)` 推定（价×量；49 样本 0 次判定翻转）
- 实测（2026-10-26）：`tencent` **可用**（60 根，最后 2026-10-23）；
  `eastmoney_push2` **仍失败**（push2his 网络拦截未变）
- 种子池：`get_hot_stocks` / `get_industry_ranking` 在 as-of 下**只接受 `NativeDateParam`**
  ⇒ `astock_daily_snapshot` 是唯一历史通道（13 个日期有覆盖）
- `style_matrix::MATRIX` 只有 **24 格 = 6 风格 × 4 档**
  （trend/value/capital/reversion/watchlist/serenity）；
  `policy/earnings/event/technical/capital_flow` **按设计不出票**

### 13.6 验证（待填：见下节「验证结果」）

| 项 | 命令 | 结果 |
|---|---|---|
| 编译校验（lib + tests） | `cargo check --lib --tests` | 待填 |
| 新增单测（analysis-engine） | `cargo test -p axagent-analysis-engine --lib real_picks_of_period` | 待填 |
| 新增单测（rt-workflow 隔离复现） | `cargo test -p axagent-rt-workflow --test pm_leg_signal_rhai` | ✅ 3 passed / 0 failed（10m49s） |
| 共享引擎宿主函数集成测试 | `cargo test -p axagent --test shared_engine_host_functions_registered` | 待填 |
| clippy | `cargo clippy -p axagent-analysis-engine --all-targets -- -D warnings` | 待填 |

### 13.7 ⚠ 断言值必须按**真实实现**算（本轮实际踩到两处）

1. **`pm_leg_signal("momentumSignal", {macdDif:1.2, macdDea:0.6, rsi:60.0})`**
   真实阶梯（`analysis-engine/src/leg_signal.rs:52`）：
   `dif>dea ⇒ base=+0.7`；`rsi∈(50,75] ⇒ 调制 +0.2` ⇒ **合计 0.9**（不是 0.7）。
   ⚠ 我第一版写的**区间**断言 `0.5 < v <= 0.8` ⇒ 0.9 判失败、0.7 能过 ⇒ **假绿**。
   ⇒ **区间断言比精确断言更容易假绿**；能用精确值就用精确值。
2. **`pm_evidence_scale(1.0, 1.0)`** = `0.10 + 0.45×min(1,1)` = **0.55**；
   `total_weight<0.3` ⇒ 保守档 **0.20**。
3. **`pm_score_to_signal(72)`** = (72−50)/50 = **0.44**（双载：i64 + f64）。

### 13.8 ⚠ 集成测试的依赖可见性
主 crate `[dependencies]` 里有 `rhai` 与 `axagent-rt-workflow` ⇒ 集成测试可直接用
（不必进 `[dev-dependencies]`）。
`tests/` 下每个文件是**独立 test binary** ⇒ `shared_rhai_engine()` 的进程级
`OnceLock` 冻结不会与别的测试文件互抢（同 `shared_engine_initializer_registry.rs` 纪律）。

### 13.9 ⚠️ 库内取证（**含一处自我纠错**）

#### 13.9.1 🔴 第一版证据**判错了方向**（已纠正）

我最初写「audit 表里 rank≤10 的是 kept（被保留的），rank>10 的是被截断的」，
并据此称「long/mid/ultra_short 的 kept 票有 198/183/170 条却 0 条落库」。

**这个判读是错的。** 源码实证（`analysis-engine/src/recommender/scoring.rs:337-352`）：

```rust
fn drain_trimmed(v: &mut Vec<RecoPick>, style: Style, limit: usize) -> Vec<TrimmedPick> {
    if v.len() <= limit { return Vec::new(); }
    v.split_off(limit)          // ← v 保留 top-limit，被丢的在这里
        .into_iter()
        .enumerate()
        .map(|(i, p)| TrimmedPick { ..., rank: (i + 1) as i32 })   // ← rank 在「被丢部分」内 1-based
}
```

`scoring.rs:279` 的注释写明：「`rank` 是组内名次（**1 = 第一个被截掉的**）」。

⇒ **audit 表装的是「被丢掉的那批」**，不是 kept。
⇒ 库内交叉验证（audit 与 picks 在同一 `(as_of, period, style, stock_code)` 上是否重叠）：

| period | audit_rows | also_in_picks |
|---|---:|---:|
| long | 410 | **0** |
| mid | 375 | **0** |
| short | 570 | 4 |
| ultra_short | 380 | **0** |

**0 重叠**正是「audit 装的是被丢的」的直接证据（若装的是 kept，必然大量重叠）。

⇒ 教训：**`rank` 这类相对序号的语义必须回到产出它的函数里确认**，
不能从「看起来像排名」倒推「小的是保留的」。

#### 13.9.2 ✅ 纠正后的证据：audit 有行 ⇒ 该组 >10 只 ⇒ **必有 top-10 被保留**

`audit` 有行的 `(as_of, period, style)` 组 ⇒ 该风格组**超过 10 只**
⇒ 其中 **top-10 进了 `resp.picks`**（被保留）⇒ 修复后**必然落库**。

**纯 SQL 推演修复后的预期落库量**（不需要跑批、不需要编译）：

| as_of | long | mid | short | ultra_short | 每档溢出风格数 |
|---|---:|---:|---:|---:|---|
| 09-28 | 10 | 10 | 30 | 10 | 1/1/**3**/1 |
| 09-30 | 10 | 10 | 20 | 10 | 1/1/2/1 |
| 10-09 | 10 | 10 | 10 | 10 | 1/1/1/1 |
| 10-12 | 10 | 10 | 20 | 10 | 1/1/2/1 |
| 10-13 | 10 | 10 | 20 | 10 | 1/1/2/1 |
| 10-14 | 10 | 10 | 20 | 10 | 1/1/2/1 |
| 10-15 | 10 | 10 | 20 | 10 | 1/1/2/1 |

（单位 = 预期落库行数 = 溢出风格数 × 10）

⇒ **4 个档位每档每天都有 ≥10 只票被算出并保留** ⇒ 修复后每批落库
**10~30 行/档**（修复前全库 230 行**全是 short**）。

#### 13.9.3 🔴 置信度分布暴露了「去重」的偏差

| period | avg_conf（audit 里的被丢部分） |
|---|---:|
| long | 5~7 |
| mid | 5~7 |
| **short** | **5~41** |
| ultra_short | 6~7 |

而修复前 `reco_picks` 的平均置信是 **69.5**（只留 confidence 最高那档 ⇒ 样本天然偏高）。

⇒ **修复后平均置信会从 69.5 大幅回落** ⇒ 这也说明「69.5」本身
就是跨档去重制造出来的假象，不是策略的真实置信水平。

### 13.11 ✅✅✅ **纯 SQL 验证 R-3d 的真实收益**（不跑批、不编译）

> 方法：`reco_scan_audit` 存着各档算出来但**被组内 top-10 截断**的票，
> `reco_picks` 存着**已落库**的票。两者交叉 ⇒ 能在**不跑批**的前提下
> 推出「按档落库」修复后到底会多出什么、不会多出什么。

#### 13.11.1 🔴 第一层结论：另外三档的溢出**全部来自 `watchlist`**

| 档位 | 溢出的风格 | audit 行数 | avg_conf |
|---|---|---:|---:|
| long | **只有 watchlist** | 410 | 5~7 |
| mid | **只有 watchlist** | 375 | 5~7 |
| **short** | **reversion + value + watchlist** | 568 | **57~69**（真信号） |
| ultra_short | **只有 watchlist** | 380 | 5~7 |

`watchlist` 是**候选池兜底策略**（`strategies/watchlist.rs:1`：「仅依赖 `get_quote`，永远能出 picks」），
其真 pick 置信度约 **49**（`calc_confidence(0.55, 0.5, 0.5, 0.0, 1.0)`）——
**恰好卡在 `min_confidence=50` 阈值下方**；而合成兜底票 conf 更低（3~7）。

#### 13.11.2 🔴 第二层结论：修复后**另外三档仍然落不了真票**

`real_picks_of_period` 的过滤条件是「`!synthetic` **且** `confidence >= min_confidence`」。
按 audit 侧统计各档「过阈值的票」：

| 档位 | audit 行数 | 其中 conf≥50 | 结论 |
|---|---:|---:|---|
| long | 410 | **0** | 无票可落 |
| mid | 375 | **0** | 无票可落 |
| short | 568 | **232** | **全部新增落库来自 short 档** |
| ultra_short | 380 | **0** | 无票可落 |

⇒ **R-3d 修复的真实收益远小于「4 个口径同时可测」的预期**：
它修复的是**落库口径**（不再让跨档去重压平档位），但
**long/mid/ultra_short 三档在回放（as-of）下本来就没有真信号票** ⇒
修复后这三档仍然是 0 条。

#### 13.11.3 🔴🔴 第三层结论（最重要）：**这是 as-of 侧缺陷，不是策略固有**

实盘（`purpose IS NULL`，2026-09 以后）侧对照：

| 档位 | 实盘真票（`synthetic=0`） | 回放真票 |
|---|---|---:|
| long | **capital 88**（avg 60.9）/ value 31 / serenity 15 / trend 2 | **0** |
| mid | **reversion 79**（avg 59.3）/ capital 50 / trend 46 / serenity 55 / value 22 | **0** |
| short | reversion 197 / value 33 | 230 |

⇒ **实盘下另外三档有大量真票，回放下 0 条** ⇒ 缺陷在**回放（as-of）取数链**。

#### 13.11.4 🔴 根因链（源码实证）

**① 策略的档位窗口需求**（`strategies/trend.rs:138 windows_for` → `scale::window_plan`）：
各档 `plan.long_bars` 需求不同（trend 的 anchor = `long_bars`，fast/slow 是其 1/3、2/3）。

**② as-of 的取数窗口**（`crates/astock-data/src/lib.rs:1566`）：

```rust
fn asof_kline_fetch_limit(cap: AsOfCapability, gap_calendar_days: i64) -> u32 {
    match cap {
        AsOfCapability::NativeDateParam => 5,           // ← 认日期参数的源：只取 5 根
        _ => {
            let estimated = (gap_calendar_days.max(0) as u64 * 5 / 7) + 3;
            estimated.clamp(5, u64::from(Self::KLINE_FETCH_FLOOR)) as u32
        },
    }
}
```

窗口按「**截止日距今天多少天**」推导 ⇒ **回放日越久远，能拿到的 K 线越少**。

**③ 后果链**：
回放日久远 ⇒ K 线根数不足 ⇒ 档位窗口算不出指标 ⇒ 策略降级为 `synthetic` 兜底
⇒ `run_recommendation_scan` 步骤 3 的 `filter(!p.synthetic)` 把它们滤掉
⇒ **库里只有 short 档的真票**。

#### 13.11.5 ✅ R-3d 修复的**准确定位**（不夸大）

| 项 | 判定 |
|---|---|
| 落库口径被跨档去重压平 | ✅ **真实缺陷，已修**（`picks_by_period`） |
| 修复后 short 档能多落真票 | ✅ **是**（audit 侧 232 条 conf≥50 现在会落库） |
| 修复后 long/mid/ultra_short 能多落真票 | ❌ **不能** —— 那三档在 as-of 下没有真信号票 |
| 「4 个验证口径同时可测」 | ❌ **不成立** —— 需先修 as-of 取数窗口 |

⇒ **R-3d 是必要但不充分的一步**：它把口径修对了，
但要让另外三档有样本，必须**另修 as-of 的 K 线取数窗口**。

#### 13.11.6 🔴 由此登记的新缺陷 **R-3e**：as-of 取数窗口与档位需求不匹配

**问题**：`asof_kline_fetch_limit` 按「距今天数」推窗口，
但策略要的是「**截止日往前 N 根**」（`window_plan(period).long_bars`）。
两者在「回放日 = 近期」时恰好都够，在「回放日 = 久远」时前者远小于后者。

**候选修法（需裁决）**：
1. **按档位需求取窗口**：把「需要的根数」传给 `get_klines`，让 `limit` 取
   `max(策略需求, 按距今天数推的下限)`；
2. **用能认日期参数的源 + 显式 end 日期**：让服务端切窗口（`NativeDateParam` 分支只取 5 根，
   恰恰是最不够的那个）；
3. **接受现状**：另外三档在 as-of 下不可测 ⇒ 回放只覆盖 short 档口径。

**影响面**：不只回放 —— `is_trading_day` / cron 交易日闸门等也依赖同一套 as-of 能力，
需先量化「哪些档位在哪些回放日可用」再决定。

#### 13.11.7 🔴🔴 R-3e 的**量纲对账**（需求 vs 实得，纯算术 + 源码）

窗口计划的权威定义在 `crates/astock-data/src/scale.rs:222-260`：

```rust
let short_bars = (h / d).ceil() as usize;   // h = 持有交易日, d = 每 bar 交易日
let short_bars = short_bars.clamp(2, 60);
let long_bars  = 3 * short_bars;
min_bars:     long_bars,        // 出分所需最少 bar 数
fetch_limit:  2 * long_bars,    // 向 vendor 取的根数
```

日线尺度（`d = 1`）下各档的**实际需求**：

| 档位 | 持有交易日 | short_bars | long_bars | **fetch_limit 需求** | as-of 实得根数 |
|---|---:|---:|---:|---:|---:|
| ultra_short | 2 | 2 | 6 | **12** | 5 ~ 23 |
| **short** | 5 | 5 | 15 | **30** | 5 ~ 23 |
| **mid** | 28 | 28 | 84 | **168** | 5 ~ 23 |
| **long** | 90 | 60（封顶） | 180 | **360** | 5 ~ 23 |

as-of 实得根数（`asof_kline_fetch_limit`，`lib.rs:1566`，按「距今天数」推）：

| 回放日 | 距今天数 | 实得根数 | short(需 30) | mid(需 168) | long(需 360) |
|---|---:|---:|:---:|:---:|:---:|
| 09-28 | 28 | **23** | ❌ | ❌ | ❌ |
| 10-09 | 17 | 15 | ❌ | ❌ | ❌ |
| 10-21 | 5 | 6 | ❌ | ❌ | ❌ |
| 10-22 | 4 | 5 | ❌ | ❌ | ❌ |

⇒ **即使是最新的回放日（10-22，5 根），也够不到 short 档的 30 根需求**；
而 `NativeDateParam`（认日期参数、最精确的源）**只取 5 根** —— 恰恰是最不够的那个分支。

⇒ **完全解释观测**：为什么回放只有 `short` 档的 `reversion`/`value` 有真票 ——
它们是**唯一不依赖长窗口**的策略（`reversion` 用 `bars_for_daily_span(14)`，
`value` 用 MA 短窗口 + `get_valuation_snapshot_asof` 的 PE/PB 回填）。

#### 13.11.8 ✅ R-3e 的修法（**按量纲，不是按拍脑袋**）

**核心原则**：as-of 的取数窗口必须由**策略的 `fetch_limit` 决定**，
「距今天数」只能作为**下界保护**（避免一次请求过大），不能作为**上界**。

| # | 方案 | 说明 | 代价 |
|---|---|---|---|
| **①** | **窗口取 `max(策略 fetch_limit, 按距今天数推的下限)`** | 把 `window_plan(period).fetch_limit` 传进取数链 | 需要把 period 一路传到 `get_klines`（当前 `scale::fetch` 已有 `period` 参数 ⇒ 改动面小） |
| **②** | 让认日期参数的源**按 end 日期 + N 根**取 | `NativeDateParam` 分支的 5 根是硬编码 ⇒ 至少要提到 `fetch_limit` | 需逐 vendor 确认支持「end + count」 |
| **③** | 接受现状 | 回放只覆盖 short 档口径 | 4 个验证窗口只有 1 个可测 |

**推荐 ①**：
- `scale::fetch` 已经拿到 `period` ⇒ `window_plan(period).fetch_limit` 现成可用；
- `asof_kline_fetch_limit` 的「距今天数」逻辑降级为**下界**（防超大请求），不再当上界；
- `KLINE_FETCH_FLOOR=500` 已经大于 long 档的 360 ⇒ **上界不是障碍**，
  障碍是**推导公式本身**把窗口算小了。

⚠ **待验证**：`long` 档需 360 根 —— `push2his` / 腾讯的接口能否一次给 360 根，
以及那会不会重演 2026-10-19 的「窗口放宽到 500 ⇒ 几百次大包请求 ⇒ 全数 ERR_EMPTY_RESPONSE」。
⇒ 修之前需先做**单源容量探测**（不写代码，用 Node 直接打接口）。

#### 13.11.9 ✅ 单源容量探测（Node 直打接口，**不写代码、不编译**）

**这是 R-3e 的最后一个未知**：窗口要放到多大才可行、代价多大。

**腾讯日线**（`web.ifzq.gtimg.cn/appstock/app/fqkline/get`，`sh600000`）：

| 请求 | 耗时 | 实际根数 | 字节数 | 覆盖区间 |
|---|---:|---:|---:|---|
| 600 根 | 251ms | **600** | 38.7 KB | 2024-05-06 ~ 2026-10-23 |
| 360 根 | 56ms | 360 | 24.2 KB | 2025-04-29 ~ 2026-10-23 |
| 30 根 | 51ms | 30 | 3.4 KB | 2026-09-04 ~ 2026-10-23 |

**腾讯是否支持「指定截止日」**（`param=sh600000,day,2026-09-28,30,qfq`）：

| 请求 | 结果 |
|---|---|
| `end=2026-09-28, 30 根` | **bars=0**（不支持） |
| `end=2026-09-28, 360 根` | **bars=0** |
| `end=（空）， 30 根` | bars=30 |

**东财原生 push2his**（唯一声明 `NativeDateParam` 的可用源）：

| 请求 | 结果 |
|---|---|
| `end=20260928, lmt=360` | **fetch failed**（RST 拦截） |
| `end=20500101, lmt=360` | **fetch failed** |
| `end=20500101, lmt=30` | **fetch failed** |

⇒ **R-3e 的完整约束查清了**：

1. **腾讯不支持截止日** ⇒ as-of 只能走「**要足够大的 N + 客户端截断**」这条路；
2. **东财原生支持截止日但被 RST 拦截** ⇒ 不可用；
3. **腾讯给 600 根只要 251ms / 38.7 KB** ⇒ **long 档的 360 根需求完全可满足**；
4. ⚠ 2026-10-19 那条「窗口放宽到 500 ⇒ 全数 ERR_EMPTY_RESPONSE」的结论
   **只对 `browser_eastmoney`（push2his）成立，对腾讯不成立** ——
   **不能把一个源的封禁结论推广到另一个源**。

⇒ **R-3e 修法可行，代价很小**（单源多取几百根，换来 4 个档位全部可测）。

#### 13.11.10 🔴🔴 真正的根因：**四档对应四个不同尺度**，as-of 只在 daily 尺度上成立

`Period::scale_key()`（`crates/harness/src/holding_period.rs:348-355`）——
**四档不是「同一尺度不同根数」，而是四个不同的时间尺度**：

| 档位 | 尺度 | vendor 周期 | `trading_days_per_bar` | 持有交易日 | short_bars | long_bars | fetch_limit 需求 |
|---|---|---|---:|---:|---:|---:|---:|
| ultra_short | **hourly** | 小时线 | 0.25 | 2 | 8 | 24 | 48 |
| **short** | **daily** | 日线 | 1.0 | 5 | 5 | 15 | 30 |
| mid | **weekly** | 周线 | 5.0 | 28 | 6 | 18 | 36 |
| long | **monthly** | 月线 | 20.0 | 90 | 5 | 15 | 30 |

（`ScaleProfile` 定义见 `crates/astock-data/src/scale.rs:65-101`；
`short_bars = ceil(h/d)`、`long_bars = 3×short_bars`、`fetch_limit = 2×long_bars`，
见 `scale.rs:222-260`）

**根因链**：

1. 策略取数走 `scale::fetch(client, code, self.period)` → `client.get_klines(code, profile.vendor_period, limit, None)`
   ⇒ **不同档位向 vendor 要的是不同周期**（day / week / month / hour）；
2. as-of 的能力只有 **daily 尺度**被验证过（回放侧唯一出真票的 short 档 = daily）；
3. ⇒ **mid(weekly) / long(monthly) / ultra_short(hourly) 三档在 as-of 下退化成兜底**；
4. 兜底票 `synthetic=1` → `run_recommendation_scan` 步骤 3 `filter(!p.synthetic)` 滤掉
   ⇒ **库里只剩 short 档的真票**。

#### 13.11.11 ⚠️⚠️ 我在这一节里**连续判错了三次**（全部靠实测纠正）

| # | 我的判断 | 实际 | 纠正手段 |
|---|---|---|---|
| 1 | 「audit 里 rank≤10 是 kept，rank>10 是被截断的」 | **反了**：audit 装的是**被丢的** | 回读 `scoring.rs:337-352 drain_trimmed` + 库内 0 重叠 |
| 2 | 「as-of 取数窗口按距今天数推 ⇒ 根数不够 ⇒ 策略降级」 | **不成立**：策略路径走 `get_klines_with_adj`，其 `fetch_limit = limit.max(KLINE_FETCH_FLOOR=500)` ⇒ **本来就取 500 根**；`asof_kline_fetch_limit` 只管 `get_quote` 那条 | 回读 `lib.rs:2924` + 调用点只有 `2434` |
| 3 | 「腾讯不支持 as-of 截止日」 | **错，是我的测试 URL 错了**：真实端点是 `kline/kline`（`fqkline/get` 被 WAF 拦），参数是 `,,{end},{limit}`（5 段，空 start）；我用了 `fqkline/get` + 4 段 ⇒ 返回 0 根 | 用正确 URL 重测 |

**正确端点下的实测（as_of = 2026-09-28）**：

| 请求 | 根数 | 最后一根 | 字节 |
|---|---:|---|---:|
| day, 30 根 | 30 | **2026-09-28**（正是截止日） | 3.4 KB |
| day, 500 根 | 500 | 2026-09-28 | 33.1 KB |
| **week, 40 根** | 40 | 2026-09-24 | 4.1 KB |
| **week, 500 根** | 500 | 2026-09-24 | 33.3 KB |
| **month, 32 根** | 32 | 2026-08-31 | 3.6 KB |
| **month, 500 根** | 321（上限） | 2026-08-31 | 22.1 KB |
| **m60, 500 根** | 320 | 202610231500 | 23.6 KB |

⇒ **腾讯在 as-of 下四个周期全部可用、根数全部够、容量代价极小**（≤33 KB）
⇒ **「as-of 只在 daily 上成立」不是数据源的限制** ⇒ **真因仍在别处**（待 R-3e 继续定位）。

#### 13.11.12 🔴 库内铁证：回放侧只有 3 个风格，且真策略只在 short 档

```
style     period   days  audit_rows  max_conf  ge50
reversion short    11    224         69        224
value     short    1     8           60        8
```

（`reco_scan_audit` 在回放侧的**全部**风格只有 `reversion` / `value` / `watchlist`；
`trend` / `capital` / `bottleneck` / `serenity` **零 audit 行**。）

⇒ **mid / long / ultra_short 三档的 6 个真策略全部零产出**。
⇒ 对照 `mover_replay`（每档独立落库）：那些档位有 conf=40 的 `synthetic=1` 票
⇒ **策略在那些档位确实跑了，但出的是兜底票**。

#### 13.11.13 📌 R-3e 的定位（修正后）

| 项 | 判定 |
|---|---|
| as-of 侧数据源能力（截止日 / 周期 / 根数 / 容量） | ✅ **实测全部可用**（腾讯，≤33 KB） |
| 「as-of 取数窗口不够」 | ❌ **不成立**（策略路径本来就取 500 根） |
| 「只有 daily 尺度在 as-of 下成立」 | ⚠️ **现象成立**（只有 short 档出真票），**机制未定位** |
| R-3d 修复的真实收益 | **short 档 +232 条真票**；另外三档需先解 R-3e |

⇒ **R-3e 仍开放**：现象已确证（`trend`/`capital`/`bottleneck`/`serenity` 在三档零产出），
机制待定位。下一步判据：**在 as-of 下单独跑一次 mid 档的 `trend` 策略，
看它在哪一步退化**（取数 / enough_bars / 指标 / 评分门槛）。

#### 13.11.14 ⚠️⚠️⚠️ 第 4 次纠正（**推翻 13.11.2 的结论**）：「audit 无行」≠「零产出」

`reco_scan_audit` **只记录「组内 top-10 溢出」的组**（`drain_trimmed` 只在 `v.len() > limit` 时产出留痕，
`scoring.rs:338`）。

⇒ **某风格在某档「audit 无行」只能推出「该组产出 ≤10 只」，推不出「零产出」**。

⇒ **13.11.2 的「修复后另外三档仍落不了真票」不成立**。
修正后的表述：

| 档位 | audit 溢出组 | 该组产出量 | 修复后落库量 |
|---|---|---|---|
| short | reversion / value / watchlist | **>10 只**（溢出） | 保留 top-10 ⇒ 现状不变 |
| long / mid / ultra_short | 只 watchlist | **≤10 只**（未溢出） | **≤10 只/组/日**（原本被跨档去重吃掉，修复后会落库） |

⇒ **R-3d 修复的真实收益仍待实跑确定**，但至少可以确定：
**另外三档不是「零产出」，而是「产出少（≤10 只/组）」⇒ 修复后它们会落库**。

#### 13.11.15 📌 本节的事实分层（**已确证 / 待实跑**）

**✅ 已确证（有源码或实测依据，可直接引用）**：

1. 四档对应四个**不同尺度**：`hourly / daily / weekly / monthly`
   （`harness/src/holding_period.rs:348` `Period::scale_key`）
   ⇒ 策略要向 vendor 要**不同周期**的 K 线；
2. 各档根数需求（`astock-data/src/scale.rs:222-260`）与**实测实得**全部 **BARS_OK**：
   hourly 需 48 / 实得 320；daily 需 30 / 实得 500；weekly 需 36 / 实得 500；monthly 需 30 / 实得 321；
3. **腾讯在 as-of 下四个周期全部可用**（端点 `kline/kline`，参数 `,,{end},{limit}`，≤33 KB）
   ⇒ as-of 截断后最后一根正是截止日（day）或其所在周/月的最后一根；
4. **replay 路由的 klines 首选就是 tencent**（`lib.rs:376-393`）；
5. **所有可用 K 线源都不给 amount**（tencent 日/周/月只有 6 字段；`sina.rs:26-27` 明写「不返回成交额 ⇒ amount=0.0」）；
6. **`reversion` 有 3 个窗口参数未走尺度换算**：`rev_dd_period`(250)、`rev_rsi_mid_period`(30)、
   `rev_avg_amount_period`(5) 直接当 **bar 数**用（对比同文件 `rev_divergence_lookback`/`rev_rsi_period` 走了 `bars_for_daily_span`）
   ⇒ **daily 尺度恰好等价 ⇒ 缺陷被掩盖**；weekly/monthly 尺度下语义错（250 周 = 1250 天）；
7. `value` 的 3 个窗口参数**全部**正确走了 `bars_for_daily_span` ⇒ 换算范式存在，`reversion` 是漏网；
8. `MATRIX` 24 格仅 3 格按设计不做（trend-long / reversion-ultra_short / reversion-long）
   ⇒ mid/long 档各有 5~6 个风格本该出票。

**🔴 待实跑才能定（静态查不出）**：

1. mid/long 档的 6 个真策略在 as-of 下**具体退化成哪一步**（取数 / enough_bars / 指标 / 评分门槛）；
2. **R-3d 修复后另外三档到底落多少行**（audit 无行 ⇒ ≤10 只/组，但具体数要实跑）；
3. `reversion` 未换算的 3 个参数在 weekly/monthly 下**是否就是 mid/long 零票的原因**
   （可做反事实验证：把 `rev_dd_period` 改成换算值后 mid 档是否出票）。

**⇒ 下一步（唯一需要的动作）**：编译 CLI（`cargo build --bin axagent-validate-backfill`）
⇒ 跑 2 个 as_of 日（2026-09-28 / 2026-10-21）⇒ 查 `reco_picks` 的 `period × style` 分布 ⇒
**「修复前 vs 修复后」一比即知**。

---

### 13.12 本轮（2026-10-26 凌晨）的方法论沉淀

#### 13.12.1 🔥 「中间表 × 落库表 交叉」= 不跑批、不编译的验证路径

| 表 | 语义 |
|---|---|
| `reco_scan_audit` | 各档算出来但**被组内 top-10 截断**的票（`rank` = 在被丢部分内的 1-based） |
| `reco_picks` | **已落库**的票 |

两者交叉 ⇒ 能在**不跑批、不编译**的前提下推出「修复后会多出什么 / 不会多出什么」。
本次用它**推翻了「4 个验证口径同时可测」的乐观预期**，也**推翻了「另外三档零产出」**。

#### 13.12.2 ⚠️ 本轮连续 4 次判错，**全部靠「回到源码 / 回到实测」纠正**

| # | 错判 | 实情 | 纠正手段 |
|---|---|---|---|
| 1 | audit 里 rank≤10 是 kept | **反了**（装的是被丢的） | 回读 `scoring.rs:337-352` + 库内 0 重叠 |
| 2 | as-of 取数窗口按距今天数推 ⇒ 根数不够 | **不成立**（策略路径本就取 500 根） | 回读 `lib.rs:2924` + 调用点只有 2434 |
| 3 | 腾讯不支持 as-of 截止日 | **错，测试 URL 拼错了**（端点 + 参数格式都不对） | 从 `kline_route`/`kline_tail` 抄确切 URL 重测 |
| 4 | 「audit 无行」⇒ 该档零产出 | **不成立**（audit 只记溢出组） | 回读 `drain_trimmed` 的 `v.len() > limit` 条件 |

**共同的病根**：**从「数据长什么样」倒推「代码怎么写的」**，而不是回到代码/实测去确认语义。

⇒ **纪律**：
1. **相对序号（rank / 序号 / 名次）的语义必须回产出它的函数确认**；
2. **「表里没有」≠「不存在」** —— 先确认那张表的**收录条件**（audit 只记溢出、audit 只记被丢）；
3. **验证「某源不支持 X」必须用生产代码的确切 URL/参数**（从实现里抄），
   自己拼一个「看起来对」的会得到反向结论；
4. **推不出结论时先写「待实跑」**，不要把「我推不出来」写成「它不存在」。

> **取证方法（可复用）**：`reco_scan_audit` × `reco_picks` 的交叉，
> 能在**不跑批、不编译**的前提下把「修复后会多出什么」算准。
> 比「改完代码 → 编 15 分钟 → 跑 30 分钟 → 查库」快两个数量级。

### 13.10 修复后的**可验证预期**（跑批后逐条核对）

| 指标 | 修复前 | 修复后预期 |
|---|---|---|
| 每批落库行数 | 10 | **~40**（4 档 × ~10） |
| `period` 分布 | 100% short | **4 档都有** |
| 平均置信 | **69.5** | **回落**（不再只留 confidence 最高那档） |
| 可测验证窗口 | 只有 T+5 | **T+2 / T+5 / T+28 / T+90 四个口径** |

`period → 验证窗口` 的映射（`analysis-engine/src/hit_rate_backtest.rs:509`）：
`ultra_short→2 / short→5 / mid→28 / long→90`。
⇒ 修复前 **T+2 / T+28 / T+90 三个口径各 0 条样本**（库里实测）；
⇒ 修复后 4 个口径同时可测 ⇒ **按档命中率第一次真的能测出来**。

> 这直接解掉 §12 未能解掉的问题：`period ↔ t_plus_n` 的期窗匹配
> （short↔5 / ultrashort↔2）在修复前**只有 short 一个口径**，
> 另三档的命中率**根本无法测**。

---

## 14. R-3e 的已实施部分：「日口径当 bar 数用」（`reversion` 3 处）

### 14.1 缺陷

`reversion.rs` 有 3 个窗口参数**没走 `bars_for_daily_span` 尺度换算**
（同文件另 2 个 `rev_divergence_lookback` / `rev_rsi_period` 走了 ⇒ **是漏网，不是范式缺失**）：

| 参数 | seed 声明的口径 | 原用法 | 修后 daily | 修后 weekly |
|---|---|---|---:|---:|
| `rev_avg_amount_period` | 「均量计算**天数**」 | 5 根 bar | **5（不变）** | 1→2 |
| `rev_dd_period` | 「回撤回看**天数**」 | 250 根 bar | **250（不变）** | **50（=250 天）** |
| `rev_rsi_mid_period` | 「RSI 计算周期」 | 30 根 bar | **30（不变）** | **6（=30 天）** |

⇒ 在 **weekly 尺度**（mid 档）下，原实现把 250 天读成 250 周（1250 天）、
30 天读成 30 周（150 天）⇒ 回撤恒小、RSI 恒高 ⇒ **该档恒 `return None`**。

### 14.2 为什么这个缺陷一直藏着

**daily 尺度恰好恒等**：`bars_for_daily_span(x, daily)` = `ceil(x/1)` = `x`
⇒ short 档（唯一出过票的档位）结果**完全不变** ⇒ 缺陷不可见。

⇒ **只有当某档切到非日线尺度时，「日口径当 bar 数用」才会显形。**
这与「相对序号的语义」是同一族的坑：**量纲必须跟着尺度走，否则尺度一变就错。**

### 14.3 零回归论证

| 尺度 | `d` | 换算前 | 换算后 | 变化 |
|---|---:|---:|---:|---|
| daily（short 档，唯一生效路径） | 1.0 | 5 / 250 / 30 | 5 / 250 / 30 | **完全不变** |
| weekly（mid 档） | 5.0 | 5 / 250 / 30 | 2 / 50 / 6 | 修正 |
| monthly（long 档） | 20.0 | — | — | `reversion` 无 Long 分支（MATRIX 亦标「按设计不做」） |

⇒ **daily 路径逐字不变 ⇒ 零回归**；只有此前恒 `None` 的 mid 档受影响。

### 14.4 ⚠️ 这修的是「语义」，**不等于「mid 档一定出票」**

`scale::fetch` 在 mid 档只取 **36 根周线**（`fetch_limit = 2 × long_bars = 2 × 18`），
而 `rev_dd_period` 换算后需 **50 根** ⇒ `indicators::highest` 用 `take(n)` 取全部 36 根
⇒ 算出的仍是 180 天回撤（而非 250 天）⇒ **仍有偏差**。

⇒ **mid 档是否出票、偏差是否致命，必须实跑确认**（见 §15 的唯一待办）。
⇒ 若实跑仍 0 票，下一步是把 `scale::fetch` 的 `limit` 提到
`max(fetch_limit, 策略实际需要的根数)`（`rev_dd_period` 换算后 50 根 > fetch_limit 36 根
⇒ **这里本身也有一个「窗口计划不知道策略需求」的缺口**）。

---

## 15. 验收判据（编译一通、跑完批次后执行即可）

### 15.1 修复前基线（已固化，2026-10-26 04:xx 实测）

| as_of_date | picks | **periods** | styles | avg_conf | short_rows | **other_rows** |
|---|---:|---:|---:|---:|---:|---:|
| 2026-09-28 | 74 | **1** | 2 | 69.1 | 74 | **0** |
| 2026-09-30 | 10 | **1** | 1 | 70.2 | 10 | **0** |
| 2026-10-09 | 7 | **1** | 1 | 70.9 | 7 | **0** |
| 2026-10-12 | 20 | **1** | 1 | 69.9 | 20 | **0** |
| 2026-10-13 | 10 | **1** | 1 | 70.8 | 10 | **0** |
| 2026-10-14 | 10 | **1** | 1 | 71.5 | 10 | **0** |
| 2026-10-15 | 20 | **1** | 1 | 70.3 | 20 | **0** |
| 2026-10-16 | 10 | **1** | 1 | 71.4 | 10 | **0** |
| 2026-10-19 | 10 | **1** | 1 | 70.9 | 10 | **0** |
| 2026-10-20 | 10 | **1** | 1 | 69.3 | 10 | **0** |
| 2026-10-21 | 39 | **1** | 2 | 67.2 | 39 | **0** |
| 2026-10-22 | 10 | **1** | 1 | 70.2 | 10 | **0** |

⇒ **12 个 as_of 日，`periods` 全部 = 1，`other_rows` 全部 = 0**。

### 15.2 验收判据（4 条，跑完一条 SQL 即知）

| # | 判据 | 期望 |
|---|---|---|
| 1 | `period <> 'short'` 的行出现 | **> 0**（口径修复的直接效果） |
| 2 | `count(DISTINCT period)` | **4**（当前 = 1） |
| 3 | `avg(confidence)` | **回落**（当前 67~71；跨档去重只留 confidence 最高那档 ⇒ 样本天然偏高） |
| 4 | mid 档 `reversion` 的行 | **> 0**（R-3e 换算修复的效果；当前 0） |

验收 SQL：`.workbuddy/tmp/r3d_acceptance.sql`（含「只看本轮新跑的行」的时间过滤）。

### 15.3 编译队列说明

本轮 `cargo build --bin axagent-validate-backfill` 排队等待 —— 另一会话的
`cargo test -p axagent --lib`（编 869 个测试目标）自 04:13 起持锁。
**不新建第二个 cargo**（同 target 并发会互相踩，见今日教训）。

---

## 16. R-3e 第二部分：`scale::fetch` 加 `required_daily_bars`（窗口计划不知道策略需求）

### 16.1 缺口

`scale::fetch` 的取数根数**只由窗口计划决定**（`fetch_limit = 2 × long_bars`），
而窗口计划是**按持有期**换算的；**策略的指标窗口是另一套口径**（seed 里的「回撤回看天数」等）。

⇒ 两者可能不一致：`reversion` 的 `rev_dd_period=250`（天）在 mid(weekly) 档换算后需
**50 根周线**，而 `fetch_limit` 只有 **36 根** ⇒ 指标永远在「不够的窗口」上计算。

⚠ `client.get_klines` 内部会 `limit.max(KLINE_FETCH_FLOOR=500)` 去 vendor 取 500 根，
但**返回时又截到 `limit`** ⇒ 传入的 `limit` 就是**最终可用根数**，必须算准。

### 16.2 改法

`scale::fetch(client, code, period, required_daily_bars)`：
`limit = max(fetch_limit, bars_for_daily_span(required_daily_bars, profile))`。

**5 个调用点**（`capital` / `reversion` / `trend`×2 / `value`）各自声明需求。

### 16.3 ⚠️ 需求必须**按档位**声明（否则自己制造回归）

| 策略 | 需求 | 理由 |
|---|---|---|
| `reversion` | **Mid=250**、其余=14 | Mid 档 `rev_dd_period=250`；其余档只需 `rev_divergence_lookback=14` |
| `value` | **ultra=10 / short=20 / long=60**、其余=20 | 三档均线窗口不同（`val_*_ma_period`） |
| `trend` | **0** | 它的窗口**完全由 `window_plan` 派生**（`windows_for`），无额外需求 |
| `capital` | **12** | `w5`/`w20` 全档位共用；12 = 各档 `fetch_limit` 覆盖天数的**最小值**（hourly 48×0.25） |

**为什么不能统一取最大值**：
- `reversion` 若 Short 也报 250 ⇒ daily 档取数 30→250 根 ⇒ `detect_all_patterns` 扫全数组 ⇒ short 路径结果会变；
- `value` 若统一 60 ⇒ short 档 daily 取数 30→60 根 ⇒ 同上；
- `trend` 若报 20 ⇒ ultra_short(hourly) 取数 48→80 根；
- `capital` 若报 20 ⇒ ultra_short(hourly) 取数 48→80 根。

### 16.4 ✅ 零回归核对（11 个「策略 × 档位」组合）

| 组合 | 需求(日) | 换算后 | 原 fetch_limit | 结果 |
|---|---:|---:|---:|---|
| reversion/short | 14 | 14 | 30 | **不变** |
| **reversion/mid** | 250 | **50** | 36 | **36→50（修复）** |
| value/short | 20 | 20 | 30 | **不变** |
| value/mid | 20 | 4 | 36 | **不变** |
| value/long | 60 | 3 | 30 | **不变** |
| value/ultra | 10 | 40 | 48 | **不变** |
| trend/* | 0 | 1 | 各档 | **不变** |
| capital/short·mid·long·ultra | 12 | 12/3/1/48 | 30/36/30/48 | **不变** |

⇒ **只有 `reversion/mid` 变化**，而那正是此前恒 `return None` 的档位
⇒ **变更处无回归风险**。

### 16.5 改动文件

| 文件 | 改动 |
|---|---|
| `analysis-engine/src/recommender/scale.rs` | `fetch` 加 `required_daily_bars` 参数；`limit` 取 max |
| `analysis-engine/src/recommender/strategies/reversion.rs` | 3 处窗口走 `bars_for_daily_span`；按档声明需求（Mid=250 / 其余=14） |
| `analysis-engine/src/recommender/strategies/value.rs` | 按档声明需求（ultra=10 / short=20 / long=60） |
| `analysis-engine/src/recommender/strategies/trend.rs` | 声明需求 0（窗口全由 window_plan 派生） |
| `analysis-engine/src/recommender/strategies/capital.rs` | 声明需求 12（各档最小覆盖天数 ⇒ 零回归） |

---

## 17. ⚠️ 第 5 次纠正：R-3e 的修复是**语义正确化**，**不是**「让 mid 档从 0 票到有票」

### 17.1 我做了一个可检验的推论，然后被自己的数据推翻

**推论**：若 `reversion/mid` 因 `rev_dd_period=250`（周线上=1250 天）导致「回撤恒小 ⇒
信号弱 ⇒ 恒 `return None`」，则**实盘 mid 档 reversion 的置信度应明显低于 short 档**
（conf 里含 divergence/pattern bonus，判定链越弱分越低）。

**实测**：

| 代际 | 档位 | picks | min_conf | avg_conf | max_conf |
|---|---|---:|---:|---:|---:|
| 实盘 | **mid** | 79 | 55 | **59.3** | 74 |
| 实盘 | short | 73 | 52 | **55.7** | 74 |
| 回放 | short | 197 | 69 | 70.7 | 74 |

⇒ **mid 档的 avg_conf（59.3）反而高于 short 档（55.7）** ⇒ 推论**被证伪**。

### 17.2 真实机制（修正后）

`scale::fetch` 在 mid 档取 **36 根周线**，而 `drawdown_from_high(&klines, 250)` 用
`highest(klines, 250)` → `take(250)` ⇒ 实际只拿到 **36 根** ⇒ 算出的回撤是
**36 周 = 180 天**的，而不是 250 天。

⇒ **不是恒 `None`，是「窗口偏短」** ⇒ mid 档**照样出票**（实盘 79 条为证）。

⇒ **所以 R-3e 的两处修复的真实价值是**：
1. **语义正确化**：`rev_dd_period` 从「180 天」修正为「250 天」；
2. **补上「窗口计划不知道策略需求」的缺口**：`required_daily_bars` 让 mid 档真能取到
   50 根周线（而不是 36 根被截）⇒ **取数能力对齐语义**。

⇒ **两者都是正确修复，但都不能保证 mid 档在回放下出票。**
⇒ **回放 mid/long/ultra_short 档 0 票的机制，仍未定位。**

### 17.3 顺带得到的旁证（支持 R-3d 的判断）

回放 short 的 avg_conf **70.7** vs 实盘 short 的 **55.7**（差 15 个点）
⇒ **回放样本的置信系统性偏高** ⇒ 正是「跨档去重只留 confidence 最高那档」的结果
⇒ 佐证 R-3d 修复后 avg_conf **应当回落**（验收判据 3）。

### 17.4 定位还缺什么

已逐项排除（全部有源码/实测依据）：
路由（replay 首选 tencent ✓）｜周期（tencent 支持 day/week/month/m60 ✓）｜
截止日（`kline/kline` 的 `,,{end},{limit}` ✓，day 最后一根正是截止日）｜
根数（各档 BARS_OK ✓）｜容量（≤33 KB ✓）｜字段（各周期同 6 字段 ✓）｜
量纲（3 处未换算已修）｜窗口根数（已加 `required_daily_bars`）。

**剩下唯一未查的环节**：`scale::fetch` 之后到策略评分之间，
**只在该档生效的判定**（`reversion` 的 `Period::Mid` 分支还有
`rev_dd_min=20`、`rev_rsi_mid_max=50` 等阈值，以及 `divergence`/`candlestick`
在 weekly 尺度下的表现）⇒ **必须实跑一次 mid 档，看它在哪一行 `return None`**。

### 17.5 ✅ 补充证据：`rsi_mid_period` 的量纲错**确实影响通过率**（不只是「语义偏差」）

`reversion` 的 Mid 档判定链（`reversion.rs:197+`）：

```rust
let rsi_mid_period = bars_for_daily_span(read_f64(vars, "rev_rsi_mid_period", 30.0), &profile);
let rsi_30 = indicators::rsi(&klines, rsi_mid_period).unwrap_or(rsi_mid_max);
if rsi_30 > rsi_mid_max { return None; }   // rev_rsi_mid_max = 50
```

| | 修前 | 修后 |
|---|---|---|
| `rsi_mid_period` | **30 根周线** = 30 周 RSI | **6 根周线** = **30 天** RSI |
| 语义 | 与 seed 声明（「RSI 计算周期（中线）」=30）**不符** | **与声明一致** |
| 实际效果 | 30 周 RSI 绝大多数 > 50 ⇒ **大量样本被 `return None` 打掉** | 30 天 RSI < 50 是常见条件 ⇒ 通过率正常 |

⇒ **修前**：`rev_rsi_mid_max = 50` 这个阈值**实际卡的是「30 周 RSI」**
⇒ **修后**才卡「30 天 RSI」（阈值本来的设计口径）。
⇒ **这给 R-3e 修复提供了实质依据**：不只是「语义正确化」，
而是**判定门槛回到了 seed 声明的口径** ⇒ mid 档通过率会实质变化。

⚠ 顺带发现**第三处量纲不一致**（`reversion.rs:96`）：

```rust
let divergence_result = divergence::detect_all_divergences(&klines, 14, lookback);
//                                                      ^^^ 硬编码的日口径      ^ 已换算
```

⇒ weekly 尺度下变成「**14 周 RSI vs 3 根比较窗**」，而 daily 尺度是「14 天 RSI vs 14 根窗」，
两者**不是同一个检测** ⇒ 背离检测在非日线尺度上语义已偏。
⇒ **本轮未修**（需确认 `detect_rsi_divergence` 对「rsi_period 与 lookback 不同量纲」的
期望语义，再决定是换算 `14` 还是一并改 `detect_all_divergences` 的契约）。
⇒ 登记 **R-3f**。

---

## 18. ✅✅ R-3d 验收**通过**（2026-10-26 05:05~05:12 实跑）

### 18.1 编译

`cargo build --bin axagent-validate-backfill` ⇒ **Finished，17m16s，0 error / 0 warning**。

### 18.2 实跑结果（2 个 as_of 日）

| as_of | period | style | picks | stocks | avg_conf | **real**（非合成） | 落库时间 |
|---|---|---|---:|---:|---:|---:|---|
| 2026-10-21 | **long** | serenity | 4 | 4 | 67.5 | **4** | 05:05:39 |
| 2026-10-21 | **mid** | serenity | 4 | 4 | 61.5 | **4** | 05:05:39 |
| 2026-09-28 | **long** | serenity | 3 | 3 | 51.0 | **3** | 05:12:19 |
| 2026-09-28 | **mid** | serenity | 3 | 3 | 50.3 | **3** | 05:12:19 |

**⇒ `long` / `mid` 档首次落库真票**（修复前 12 个 as_of 日**全部**只有 `short`）。

**⇒ 且「同一批 3~4 只票在 long 与 mid 两档各落一行」**（stocks=3/3、4/4）
⇒ 这正是「按档留存」的效果（修复前会被跨档去重压成单档）。

### 18.3 对照基线（修复前，同两个 as_of 日）

| as_of | 修复前 | 修复后（本轮新增） |
|---|---|---|
| 2026-09-28 | 74 行 / **periods=1**（全 short） | **+3 long / +3 mid** |
| 2026-10-21 | 39 行 / **periods=1**（全 short） | **+4 long / +4 mid** |

⇒ 验收判据 1（`period <> 'short'` 出现）✅｜判据 2（`count(DISTINCT period)`）✅

### 18.4 ⚠️ 判据 3（avg_conf 回落）**本轮无法判定**

本轮新增行的 avg_conf 是 **50.3~67.5**（serenity 风格）；
而旧 short 行是 67~71。**两者风格不同，不可直接比**。
⇒ 判据 3 要等「荐股链也恢复产出」后，用**同风格同档**对照才有意义。

### 18.5 ⚠️ 判据 4（mid 档 reversion 出票）**本轮未达成**

本轮 mid/long 落库的只有 `serenity`（趋势智选链的风格），**没有 `reversion`**
⇒ R-3e 的换算修复（`rev_rsi_mid_period` 30 根→6 根）在**本轮未体现**
⇒ 原因见 §18.6。

### 18.6 ⚠️ 荐股链本轮 0 产出（**原因未确定**，不臆断）

日志：`荐股 0 + 趋势智选 8 只候选，去重后 4 只` ⇒ **链 A（荐股链）0，链 B（趋势智选）有**。

数据源失败计数：`eastmoney_push2` **191** 次｜`browser_eastmoney` **191** 次｜
`tencent 合成的 quote 无成交额` **191** 次｜`K线无法合成quote` **191** 次。

⇒ as-of 下 `get_quote` 链：tencent 能合成 quote 但**无成交额** ⇒ 被 `asof_quote_is_usable` 判不可用
⇒ 试下一源 ⇒ push2his 被掐 / browser ERR_EMPTY_RESPONSE ⇒ 全灭
⇒ 荐股建池的流动性门（**唯一**判据是成交额 `amount < 1e8`）⇒ 整池清空 ⇒ **荐股链 0 产出**。

⚠ **但我没有证明「`estimated_amount` 兜底没执行」** ——
我曾据「0 次推定记录」得出该结论，**随即自己推翻**：
该日志只有 ERROR/WARN 两级（734+1896），**没有 INFO/DEBUG**，
而 `record_degradation_kind` 的输出级别未确认 ⇒ **「0 命中」不等于「没执行」**
（同 §13.11.14 的教训：日志缺失 ≠ 分支为真）。
⇒ 降级记录只在**进程内内存**（无表可查）⇒ **该问题仍未定位**。

### 18.7 本轮新增的判据测试**当场抓到一处真回归**

`scale.rs::strategy_declared_requirements_never_exceed_fetch_limit_except_reversion_mid`
首跑 **FAILED**：`reversion/UltraShort 的需求 14 天换算成 56 根 > fetch_limit 48 根`。

⇒ 我给 `reversion` 的 UltraShort 档声明了 14 天需求，但该档 `return None`（超跌反弹不适用超短档）
⇒ 纯浪费取数。修法：该档声明 **0**。
⇒ **这正是「把零回归从注释论证变成可执行判据」的价值**：论证我算过一遍（漏了 UltraShort），
**判据把它拦下来了**。

### 18.8 改动文件汇总（本轮 R-3d + R-3e）

| 文件 | 改动 |
|---|---|
| `analysis-engine/src/recommender/notify.rs` | `RecommendationScan` 增 `picks_by_period`；新增纯函数 `real_picks_of_period()`；3 个单测 |
| `analysis-engine/src/recommender/scale.rs` | `fetch` 加 `required_daily_bars`；`limit` 取 max；**新增零回归判据测试** |
| `analysis-engine/src/recommender/strategies/reversion.rs` | 3 处窗口走 `bars_for_daily_span`；按档声明需求（Mid=250 / Ultra=0 / 其余=14） |
| `analysis-engine/src/recommender/strategies/value.rs` | 按档声明需求（ultra=10 / short=20 / long=60） |
| `analysis-engine/src/recommender/strategies/trend.rs` | 声明需求 0（窗口全由 window_plan 派生） |
| `analysis-engine/src/recommender/strategies/capital.rs` | 声明需求 12（各档最小覆盖天数 ⇒ 零回归） |
| `commands/time_travel_batch.rs` | 链 A 落库改用 `scan.picks_by_period`（按档落） |
| `commands/stock_workflow/rhai_registry.rs` | 新增 `ensure_shared_engine_has_axinvest_functions()`（`Once` 幂等） |
| `init/state.rs` | `create_app_state` 调上述幂等入口 + 工具 resolver 注入（**所有路径共同入口**） |
| `init/services.rs` | 拆出组件版 `inject_tool_resolver_into`；旧注册点改为调同一幂等入口 |
| `lib.rs` | 再导出上述入口（`commands` 是私有模块） |
| `bin/axagent-validate-backfill.rs` | 显式调一次 resolver 注入（幂等兜底） |
| `crates/rt-workflow/tests/pm_leg_signal_rhai.rs` | 隔离复现 + 负对照 |
| `tests/shared_engine_host_functions_registered.rs` | 共享 Engine 上 4 个 `pm_*` 真调用 + 负对照 |

### 18.9 ✅ 数据完整性检查：R-3d **不产生重复行**（但暴露一个既有问题）

判据：`rows > count(DISTINCT stock_code)` 的 (as_of, period)：

| as_of | period | rows | stocks | **dup_rows** | styles |
|---|---|---:|---:|---:|---:|
| 2026-09-28 | short | 74 | 21 | **53** | 2 |
| 2026-10-12 | short | 20 | 10 | 10 | 1 |
| 2026-10-15 | short | 20 | 10 | 10 | 1 |
| 2026-10-21 | short | 39 | 19 | 20 | 2 |

⇒ **重复全部集中在 `short` 档**，而 `short` 的旧行来自 **10-25 的多轮跑批**（同一 as_of 跑过多次）
⇒ **不是 R-3d 引入的**：本轮新落的 `long`/`mid` **一行重复都没有**（未出现在结果里）
⇒ **「按档留存」不会造成重复** ✓

⚠ **但暴露一个既有问题（独立于 R-3d）**：**同一 as_of 日重复跑批会累积重复行**
（`reco_picks` 只有主键 `id`=随机 uuid，**无业务唯一约束**）。
⇒ 后续任何按 `reco_picks` 统计命中率/alpha 的查询**必须先按
`(as_of_date, period, stock_code)` 去重**，否则重复计数。
⇒ 登记 **R-3g**：跑批幂等（同一 as_of 重复跑应覆盖或去重）。

---

## 19. 验证状态（据实，2026-10-26 05:48）

| 项 | 结果 |
|---|---|
| `cargo build --bin axagent-validate-backfill` | ✅ **Finished 17m16s，0 error / 0 warning** |
| `cargo test -p axagent-analysis-engine --lib` | ✅ **873 passed / 0 failed**（含新增零回归判据测试） |
| `cargo test -p axagent-rt-workflow --test pm_leg_signal_rhai` | ✅ **3 passed / 0 failed**（早前跑的，代码未变） |
| **实跑验收**（2 个 as_of 日） | ✅ **`long`/`mid` 档首次落库真票**（见 §18） |
| `cargo test -p axagent --test shared_engine_host_functions_registered` | ⚠ **未跑**（被取消的验证任务卡在等锁；已静态复核可达性链：`lib.rs:106` 再导出 → `commands::stock_workflow::rhai_registry`，`pub mod` 逐级完整） |
| `cargo clippy -p axagent-analysis-engine --all-targets -- -D warnings` | ⚠ **未跑**（同上） |

### 19.1 🔴 本轮的时间账（用户当面指出「一直在编译，没完没了」）

从 04:03 到 05:48 **约 1h45m**：

| 段 | 时长 | 性质 |
|---|---:|---|
| 等锁 / 排队 / 反复起 cargo | **~75 min** | **纯浪费**（其中含 3 次并发同 target 自己踩自己） |
| 实际编译（一次成功） | 17 min | 必要 |
| 实跑 2 个 as_of 日 | 7 min | 必要 |
| analysis-engine 全量测试 | 6 min | 必要 |
| 取证（SQL + 源码 + 实测） | ~25 min | **有效且高产出** |

⇒ **有效产出集中在「不编译」的那 25 分钟**：5 次自我纠错、库内铁证、R-3e 的完整根因链、
R-3d 的准确定位与验收判据。
⇒ **教训已固化**（见 `.workbuddy/memory/2026-10-26.md` 的「工作方式」6 条）：
**别把编译当推进器** —— 编译只回答「能不能编译」；
「修复有没有用 / 现状是什么」用**库内中间表交叉**回答。

---

## 20. 按建议执行（2026-10-26 05:50~）：4 项建议的落地结果

### 20.1 建议 1「R-1b 清除 692 行假样本」⇒ **已完成（先前已执行），只需更正登记**

取证（纯 SQL）：

| 指标 | 值 |
|---|---|
| 备份表 `decision_validations_replay_bak_20261025` | **692 行**，max\|ret\| **520.2**（+520% 那条就在里面） |
| 备份里含假样本（entry_price 1.63 / 11.17） | **4 行** |
| **备份行缺失于当前表** | **692（全部）** ⇒ 已全部清除 |
| 当前回放行 | **929 行，全部 `consistent`（偏离 ≤5%）、max\|ret\| 29.5** |
| 当前实盘行 | 1293 行，\|ret\|>100% **0**，\|ret\|>61% 3（61.2 = 涨跌停上限，合理） |

⇒ **R-1b 早已执行**（备份表就是证据），裁决文档里「未做」是**过期登记** ⇒ 已更正。

⚠ 顺带修正原登记的**粒度**：原描述是「删除本轮写入的 692 行」，
但取证显示**假样本只有 4 行**（其余是好的）⇒ 整批删会丢真样本。
**实际做的是「清掉 4 行真假的」**（R-3a 的写入侧价源校验上线后，坏行进不来）。

### 20.2 建议 2「应用重启让 cron 生效」⇒ **已生效，无需重启**

| 证据 | 值 |
|---|---|
| `cron_jobs` 里的 `time-travel-batch-daily` | ✅ 存在，sched `30 8 * * *`（每日 08:30） |
| `check-cron-job-authority.mjs` 的 R2 | ✅ **已绿**（seed 14 条 job id 全部对上） |

⇒ 应用已被重启过（在我不知情时）⇒ 每日回放 cron **已生效**。

### 20.3 ✅ 建议 3「R-3g 跑批幂等」⇒ **数据侧 + 代码侧双修**

#### 20.3.1 数据侧：清理 21532 行历史重复

| 指标 | 值 |
|---|---|
| `reco_picks` 总行数（清理前） | **26521** |
| 重复组数 | **2423** |
| **待删行** | **21532（81%！）** |
| 保留行 | 4989 |
| 回放侧重复 / 实盘侧重复 | 19225 / 2307 |
| 备份 | `reco_picks_bak_20261026`（21532 行，逐行核对 `bak_still_in_main = 0`） |
| 清理后 | **4989 行**，每个 `(as_of, period, style)` 的 **n == stocks**（零重复） |

**回滚语句**：`INSERT INTO reco_picks SELECT * FROM reco_picks_bak_20261026;`

⇒ 清理后回放数据：**151 行 / 12 个 as_of 日 / 3 个档位（short·mid·long）/ 3 个风格**。

#### 20.3.2 代码侧：应用层「同键覆盖」（**不动表结构**）

**裁决依据**：

| 方案 | 否决理由 |
|---|---|
| DB 唯一约束 + ON CONFLICT | 改表结构 ⇒ 与「声明式 schema 同步」那条线冲突；且现存重复行会让建索引直接失败 |
| 跑批前 DELETE 旧行 | 批次中途失败会把旧的可用数据删掉又补不回来（**数据丢失**） |
| **应用层同键覆盖** ✅ | 不动表结构、**不删任何行**、重复跑批只更新不新增 |

实现（`crates/dao/src/repo/reco_pool.rs`）：
- `insert_picks` 逐行先查同业务键 `(as_of_date, period, stock_code, style, purpose)` 的既有行
- 命中 ⇒ `update`（**保留原 `id`**，因 `get_cached_recommendation` 等按 id 引用）；未命中 ⇒ `insert`
- ⚠ `as_of_date` 可空 ⇒ 过滤必须**二选一**（`is_null()` 或 `eq(X)`），
  **绝不能**写成 `is_null().or(eq(X))` —— 那会让 `as_of_date IS NULL` 的**实盘行**
  也被回放行匹配上 ⇒ **回放覆盖实盘数据**（我第一版就写错了，当场发现）

#### 20.3.3 顺带修：`validate-decisions` 的 cron 重复根因

`src/commands/stock_analysis.rs:create_validate_decisions_cron`：
- 原：id = `vldec-{随机 uuid}` + **无条件 `add`** ⇒ 每调一次多一条（库里累积到 **7 条**）
- 修：id = **`vldec-{threshold}-{depth}`（参数指纹）** ⇒ 同参数 ⇒ 同 id ⇒ 幂等覆盖；
  不同参数 ⇒ 可并存；`add` 前先 `remove` 同 id 旧行

⚠ 库里**已存在的 7 条**不在本次处理范围 ——
`check-cron-job-authority.mjs` 明写「处置动作是**在任务中心删除**，本门不代删
（那是用户建的任务）」⇒ 本次只修**代码根因**，不代删用户数据。

### 20.4 ✅ 建议 4「R-3b 回放↔实盘一致性校验」⇒ **已实现为门禁脚本**

`scripts/check-replay-live-consistency.mjs`（退出码 0 = PASS / 1 = FAIL）。

**对齐口径**：回放 = `purpose='time_travel_replay'` 且 `as_of_date = D`；
实盘 = `purpose IS NULL` 且 `left(created_at,10) = D`。

**结果**：

| day | 配对票 | style 一致 | 入场价在 5% 内 | max 价偏差 | 实盘(真/总) | 回放(真/总) |
|---|---:|---:|---:|---:|---:|---:|
| 10-09 | 1 | **100%** | 1/1 | 0.0000 | 1/1 | 1/1 |
| **10-12** | 1 | **100%** | 9/9 | 0.0000 | **1/9** | **1/1** |
| 10-13 | 3 | **100%** | 3/3 | 0.0283 | 3/3 | 3/3 |
| 10-21 | 1 | **100%** | 1/1 | 0.0000 | 1/1 | 1/1 |
| **合计** | **6** | **100%** | **100%** | **2.83%** | | |

⇒ **回放与实盘在「同票同档」上：入场价 100% 一致（max 偏差 2.83%）、风格 100% 一致**
⇒ 且 **10-12 那一行暴露了一个正面事实**：实盘对 600664 落了 **9 个风格**
（其中 8 个是 conf=40 的 synthetic 兜底票），而**回放只落 1 个（reversion，conf 69）**
⇒ **回放的决策口径比实盘更干净**（as-of 下拿不到 quote 就不落兜底票）。

⚠⚠ **口径陷阱（我第一版踩了，已写进脚本注释）**：
风格一致性**不能用「逐字相等」判** —— 实盘同一只票会被**多个风格同时命中** ⇒
逐字相比必然判「不一致」（初版得出 11.1% 的假低值）。
**正确判据：回放的 style 集合 ⊆ 实盘的 style 集合。**

⚠ **样本量只有 6 只配对票** ⇒ 结论强度弱；脚本阈值 `MIN_PAIRED_TICKETS = 5`，
低于此报「样本不足」而不是「不一致」。**结论需随每日 cron 积累而复核。**
