# 交付：P0 / P1 实施结果（2026-10-26 06:40~07:30）

> 状态：done ｜ 日期：2026-10-26 ｜ 对应版本：v2.9.9


> 用户指令：「按照 p0、p1 顺序执行」
> 上游裁决：`AUDIT-stock-business-profitability-2026-10-26.md`
> R-3h 计划：`PLAN-r3h-long-sample-calibration.md`

---

## 0. 一句话

**P0 三项全部落地**（R-3h 核心机制已实跑验证：long 档首次产出 T+90 成熟验证行），
**P1 三项全部落地并加了门禁**；
过程中**推翻了 5 条既有结论**（R-3j / R-3l / R-3e / R-3k / 「T+90 取价错误」），
并**新登记 5 条缺陷**（R-3n ~ R-3r）。

---

## 0.1 ⚠️ 最后一环仍断：反思链（R-3r）

R-3h 的**机制**已跑通（回放早期日 ⇒ long 档当场成熟），但**校准样本仍未增长**：

```
回放 06-10 → long 档决策 → pending 反思行（hindsight = 决策日+90 交易日 = 10-23）
 └─ reflection-due-daily（18:00）消费
    └─ ⚠ 反思链整体不产出：346 条反思只有 2 条 completed
       ├─ 228 条 failed: REFLECTION_OUTCOME_MISSING（07-21~10-23）
       ├─ 5 条 failed: REFLECTION_OUTPUT_INCOMPLETE
       └─ 98 条 pending（28 条已到期未判）
```

⇒ 反思不产出 ⇒ `strategy_performance` 不增长 ⇒ 校准永不达标 ⇒ **顶层买入仍为 0**。
⚠ `REFLECTION_OUTCOME_MISSING` **不在源码里**（全仓 grep 无命中）⇒ 只能实跑拿日志定位。
**⇒ 下一步：`axagent-batch-rerun --reflect-after` 跑一轮全量 pending 并读日志。**

---

## 1. P0

### 1.1 R-3h：0 买入的根因是**校准死循环**（不是「模板换代」）

**死循环链条**（全部实测）：
```
顶层「买入」= 0（206 次分析，tv157~185）
 └─ R-CALIB-CAP：校准未达标 ⇒ 置信压至 60 ⇒ 动作联动降级 买入→持有
      └─ calibrationReady = false（160 条多档里 147 条 = 92%）
           └─ 本档已判定样本 < CALIBRATION_MIN_SAMPLES(30)
                ├─ short 18 / mid 7 / ultra 7 / long 0
 └─ 0 买入 ⇒ 无新决策样本 ⇒ 样本不增长 ⇒ 回到起点
```

**⭐ 关键机制（推翻旧认知）**：档位成熟判定只看「**行情库里决策日之后有没有 N 根 K 线**」，
**不看今天**（`compute_market_snapshots` 取最近 500 根、`exit_idx = entry_idx + expected_days`）
⇒ **回放一个足够早的 as_of 日，long 档当场就能成熟。**

**⭐ 唯一阻塞点**：池快照判据要求**双源齐全**，而实测 7 个历史日**只有 `get_industry_ranking` 一源**
（`get_hot_stocks` 最早到 09-28）⇒ 被全部排除 ⇒ long 档校准样本只能等 wall-clock（2027-01）。

**已实施**：
| # | 文件 | 改动 |
|---|---|---|
| 1 | `crates/dao/src/repo/astock_daily_snapshot.rs` | 新增 `list_snapshot_dates_with_any_method`（**一源**口径）；删除 `list_snapshot_dates_with_all_methods`（无调用方 ⇒ dead_code 会红 clippy） |
| 2 | `src/commands/time_travel_batch.rs` `require_pool_snapshot_coverage` | 返回 `Result<Option<String>, String>`：**两源都缺 ⇒ 拒绝**；缺一源 ⇒ **放行 + 降级说明**；齐全 ⇒ `None` |
| 3 | 同文件 `list_replayable_as_of_dates` | 改用「一源」清单 + **显式排除零源日** |
| 4 | 同文件 `execute_batch` / `run_time_travel_batch_inner` | 新增 `pool_degradation_note` 参数 → **落进批次消息**（少一个真实榜源必须可见） |
| 5 | `src/bin/axagent-validate-backfill.rs` | 帮助文本更新（原文「28/90 档时间旅行加速不了」**已被证伪**） |

**放宽后的可回放日**（实测 SQL 模拟）：
| 新增日期 | 源数 | 之后 K 线 | 可判档 |
|---|---:|---:|---|
| **2026-05-27** | 1 | 100 | **T+90** |
| **2026-06-10** | 1 | **90** | **T+90**（边界） |
| 2026-06-11 | 1 | 89 | T+28 |
| 2026-08-18 | 1 | 42 | T+28 |
| 2026-09-04 | 1 | 29 | T+28 |
| 2026-09-07 | 1 | **28** | T+28（边界） |

**⇒ 待实跑验收**（见 §4）。

### 1.2 R-3i：448 行孤儿验证行 —— 已修

**根因**：`reco_picks` 的「同业务键覆盖」历史实现是**先删旧行再插新行** ⇒ 旧 id 消失
⇒ 引用它的 `decision_validations.pick_id` 全部悬空（实测 448 行 / 20.2%）。
⚠ **孤儿行读数系统性偏乐观**（net −0.567 vs 关联行 −1.614）⇒ 混算虚高。

**修复（数据侧四步 + 代码侧一处）**：
| 步 | 动作 | 结果 |
|---|---|---|
| 备份 | `decision_validations_bak_20261026`（2222 行）+ `reco_picks_bak2_20261026`（5213 行） | 可回滚 |
| 1 | 从备份恢复被删但仍被引用的 pick 行 | INSERT 224 |
| 2 | 删「同决策日 + 同收益」的纯重复验证行 | DELETE 50 |
| 3 | 把「不同决策日」的验证行改挂到保留行 | UPDATE 353 + UPDATE 19（补修） |
| 4 | 删除同键多余 pick 行 | DELETE 275 |
| 5 | 加库级唯一索引 `uq_reco_picks_business_key`（`NULLS NOT DISTINCT`） | CREATE INDEX |

**最终状态**（实测）：
| 指标 | 修复前 | 修复后 |
|---|---:|---:|
| `reco_picks` | 5213 | **4957** |
| `decision_validations` | 2222 | **2172** |
| **孤儿行** | **448** | **0** ✅ |
| **业务键重复组** | 194 | **0** ✅ |
| **幂等键重复组** | 281 | **0** ✅ |

**唯一索引防御效果已实测**（事务内探针 + 回滚）：
```
错误: 重复键违反唯一约束"uq_reco_picks_business_key"
DETAIL: 键值"(as_of_date, purpose, period, stock_code, style)=(null, null, long, 000001, capital)" 已经存在
```
⇒ `NULLS NOT DISTINCT` 正确处理了可空列（`(null, null, …)` 被识别为重复）。

### 1.3 R-3k：撤销（是误判，不是缺陷）

第一版判「回放 cron 从未执行 ⇒ 调度器空转」⇒ **误判**：
该 cron 于 **10-25 23:39:50 播种**，`next_run_at` = **10-26 08:30**，而读数时刻是 10-26 06:37
⇒ **首次触发点尚未到达** ⇒ `run_count=0` 是正常状态。
⇒ 缺陷登记撤销；判据入 `MEMORY-RULES.md` #726。

---

## 2. P1

### 2.1 R-3j：期窗错配行 —— **判定为设计行为**（不是缺陷）

**根因**：`decision_validations` 是**多窗口笛卡尔积**表 ——
`t_plus_n_list` 默认 = `Period::validation_windows()` = **[2, 5, 28, 90]**，
每个 pick 跑**全部**窗口 ⇒ `period` 与 `t_plus_n` 必然大量错配（实测 1262 行 / 57%）。

⇒ **不是缺陷**。但**按档统计必须只用匹配窗口**（ultra_short↔2 / short↔5 / mid↔28 / long↔90），
否则宽口径 t=−4.82 vs 严口径 t=−0.92，**读数差一个量级**。
⇒ 已加门禁判据「匹配窗口行数 > 0」（现读数 577）。

### 2.2 R-3l：顶层与档内脱节 —— **判定为设计行为**

7 条「档内买入 → 顶层持有」逐条查清：
- **4 条**：选档规则正常（选了另一档，该档动作=持有）—— 选档规则 = 「可执行档中取 confidence 最大者」；
- **3 条**：选中的档 action=买入，但被 `R-CALIB-CAP` 降级（校准未达标 ⇒ 置信 75.1 压至 60 ⇒ 动作联动降为持有）。

⇒ **不是缺陷**，且 **R-3h 解决后这 3 条会自然消失**。

### 2.3 R-3e：as-of 非日线尺度 —— **可关闭**

「mid/long 档零真策略票」是**设计行为**：该档位的策略是 **serenity**，
而 `serenity_tiers = [Mid, Long]`（**产品定位**，非遗漏）⇒ mid/long 档有票（serenity 的），
只是没有 reversion/value/trend 那些**属于 short 档尺度**的策略票。
⇒ 已在 `reversion` 3 处 + `scale::fetch` 2 处修过量纲/根数（上一轮）。

### 2.4 R-3f / R-3q：背离检测的硬编码 `14` —— 已修

**根因**：`detect_all_divergences(klines, rsi_period, lookback)` 的两个参数**都在同一 bar 序列上取下标**
⇒ **必须同量纲**。原实现 `lookback` 换了算、`rsi_period` 硬编码 14 根 bar：
| 尺度 | rsi_period（修前） | lookback（修后） | 是否同量纲 |
|---|---:|---:|---|
| daily | 14 根日线 | 14 根日线 | ✅ |
| weekly | **14 根周线**（=98 日） | 3 根周线（=14 日） | ❌ |
| monthly | **14 根月线**（=14 月） | 5 根月线（兜底） | ❌ |

**修法**：与 `lookback` 同源换算（输入同为「日历 14 日」）⇒ **daily 恒等 ⇒ 零回归**。
**新增可执行判据** `divergence_rsi_period_and_lookback_share_one_scale`（3 条断言：daily 恒等 / 粗尺度不得仍为 14 / 同量级）。

### 2.5 R-3p：验证行幂等键过粗 —— 已修

**根因**：写入侧幂等键是 `(pick_id, t_plus_n)` ⇒ 同票同窗口但**不同决策日**的样本被误判成重复而跳过。
**实测**：402 对「不同决策日」的样本被吞（同决策日的纯重复只有 50 对）。

**修法**：幂等键改为 **`(pick_id, t_plus_n, generated_at)`**（含决策日）。

### 2.6 R-3o：唯一约束的并发副作用 —— 已处理

⚠ 加唯一索引后引入**新风险**：live 荐股 cron **没有全局互斥锁**（只有时间旅行批次有）
⇒ 两个进程同时「查到无已有行 → 都 INSERT」⇒ 第二个撞约束 ⇒ 原实现只 `warn!` ⇒ **静默丢一行票**。

**修法**：`insert_picks` 遇唯一约束冲突 ⇒ **重查该键的 id 并改成覆盖**（保留 id）⇒ 不丢行。
（实现用**闭包**构造 `ActiveModel` —— `update`/`insert` 都消费它，值形态做不到「再造一次」。）

---

## 3. 改动文件清单（8 个）

| 文件 | 改动 |
|---|---|
| `src-tauri/crates/dao/src/repo/astock_daily_snapshot.rs` | 新增 `list_snapshot_dates_with_any_method`；删除 `list_snapshot_dates_with_all_methods` |
| `src-tauri/crates/dao/src/repo/reco_pool.rs` | `insert_picks` 闭包构造 + 唯一约束冲突重查覆盖；新增 `is_unique_violation` |
| `src-tauri/src/commands/time_travel_batch.rs` | `require_pool_snapshot_coverage` 放宽 + 返回降级说明；`list_replayable_as_of_dates` 改判据；`execute_batch` / `run_time_travel_batch_inner` 传参；批次消息附加降级说明 |
| `src-tauri/src/commands/backtest_validation.rs` | `write_decision_validations` 幂等键加 `generated_at` |
| `src-tauri/crates/analysis-engine/src/recommender/strategies/reversion.rs` | 背离检测 `rsi_period` 走 `bars_for_daily_span`（R-3f） |
| `src-tauri/crates/analysis-engine/src/recommender/scale.rs` | 新增判据 `divergence_rsi_period_and_lookback_share_one_scale` |
| `src-tauri/src/bin/axagent-validate-backfill.rs` | 帮助文本更新（3 处） |
| `src-tauri/src/init/services.rs` | 注释同步（1 处） |

**新增文件（3 个）**：
| 文件 | 用途 |
|---|---|
| `scripts/check-decision-validation-integrity.mjs` | 门禁：孤儿=0 + 匹配窗口>0 + 业务键零重复 + 幂等键零重复 |
| `docs/plans/PLAN-r3h-long-sample-calibration.md` | R-3h 实施计划（含机制、证据、验收判据、残余风险） |
| `AUDIT-stock-business-profitability-2026-10-26.md` | 重答版裁决（10-26） |

**改动脚本（1 个）**：`scripts/ci-check.mjs`（挂入新门禁）。

---

## 4. 验证结果

| 项 | 结果 |
|---|---|
| `cargo check -p axagent-dao --lib` | ✅ Finished 51s，0 error / 0 warning |
| `cargo check --lib`（主 crate） | ✅ Finished 12m03s，0 error / 0 warning |
| `cargo test -p axagent-analysis-engine --lib` | ✅ **874 passed / 0 failed**（873 + 新增 1） |
| `cargo check --lib --bins` | ✅ Finished 9m38s，0 error / 0 warning |
| `cargo test -p axagent-dao --lib` | ✅ **359 passed / 0 failed** |
| `cargo clippy -p axagent-dao --all-targets -- -D warnings` | ✅ Finished 50s，0 warning |
| `cargo clippy -p axagent-analysis-engine --all-targets -- -D warnings` | ✅ Finished 1m07s，0 warning |
| `cargo clippy --lib --bins -- -D warnings`（主 crate） | ✅ Finished 8m45s，0 warning（首轮抓到 1 处 `push_str` 单字符字面量 ⇒ 已改 `push`） |
| `node scripts/check-decision-validation-integrity.mjs` | ✅ PASS（孤儿 0 / 匹配窗口 577 / 业务键重复 0 / 幂等键重复 0） |
| 唯一索引防御（事务探针 + 回滚） | ✅ 正确拒绝同键插入，探针 0 残留 |

---

## 5. 待办（下一步）

1. **🔴 R-3r：实跑一轮反思拿日志**，定位 `REFLECTION_OUTCOME_MISSING` 的运行时来源
   ```bash
   cargo run --bin axagent-batch-rerun -- --reflect-after --reflect-max 30
   ```
   ⚠ 已排除 6 个假设（见 PLAN-r3h §4.2）⇒ 不要再重复查那 6 条。
   ⚠ 反思链不产出 ⇒ 校准样本不增长 ⇒ **顶层买入仍为 0** ⇒ R-3h 死循环未闭合。
2. **跑 R-3h 的实跑验收**（已部分完成，见 PLAN §4）：06-10 ✅ / 09-07 待跑
3. **观察 08:30 首次自动触发**是否落库（回放票 `as_of_date` 是否新增 10-23）
4. **G1：冻结换代**（模板 6 天换 16 版是样本成熟的敌人）
5. 校准样本累积到 30 条/档 ⇒ `calibrationReady=true` ⇒ **验证顶层是否恢复买入**（R-3h 的最终验收）

---

## 6. 残余风险（如实登记，不得静默）

| # | 风险 | 说明 |
|---|---|---|
| 1 | **池成分不同构** | 06-10 的池 = industry 龙头(20) + FALLBACK(约80)，**无热股榜**；与「当时真实候选池」不同 ⇒ 已在批次消息留痕；校准样本只统计「档位动作→胜率」的映射，不直接当选股能力证据 |
| 2 | **long 档样本仍需累积** | 每次回放 06-10 只产 3~4 条 long 票；30 条需多次回放（可重复跑） |
| 3 | **实盘 α 仍为负** | 本轮修的是「形态层」（能不能开仓）与「样本层」（能不能判定），**没有改变盈利能力读数**：实盘 T+2 的 α=−1.85（t=−2.50）仍显著为负 ⇒ **修好之后系统会开始交易一个负 alpha 的策略** ⇒ G1（冻结换代）+ 预注册格仍必须做 |
| 4 | 唯一索引的并发路径 | live 荐股 cron 无互斥锁 ⇒ 已用「撞约束 ⇒ 重查覆盖」兜住，但**未做并发实测**（需真并发跑批验证） |
