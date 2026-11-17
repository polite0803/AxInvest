# PLAN-r3h-long-sample-calibration —— 打破「0 买入」死循环（门槛不变，增加 long/mid 样本）

> 状态：active ｜ 日期：2026-11-03 ｜ 对应版本：v3.0.0+（未定版，待确认）


> 2026-10-26 建立。用户裁决：**「门槛不能变化，看看如何增加 long 样本」**。
> 上游：`docs/audits/AUDIT-stock-business-profitability-2026-10-26.md` §2。

---

## 1. 死循环的完整链条（全部有源码/库内实证）

```
顶层「买入」= 0（206 次分析，tv157~185）
  └─ 因为档内 action 被降级：R-CALIB-CAP「置信75.1%被压至60.0%」⇒ 买入→持有
       └─ 因为 calibrationReady = false（160 条多档中 147 条 = 92%）
            └─ 因为本档已判定样本 < CALIBRATION_MIN_SAMPLES(=30)
                 ├─ short  18/30（差 12）
                 ├─ mid     7/30（差 23）
                 ├─ ultra   7/30（差 23）
                 └─ long    0/30（差 30）  ← 结构性
  └─ 顶层 0 买入 ⇒ 没有新的决策样本 ⇒ 样本数不增长 ⇒ 回到起点
```

**用户裁决：门槛（30）不变 ⇒ 必须从「样本供给」侧解。**

---

## 2. 关键机制发现（本轮取证）

### 2.1 校准样本的来源

`strategy_performance`（45 行）⇒ `reflection_stats::by_horizon` ⇒ `calibrationReady`。

两个写入通道：
| 通道 | 来源 | 当前产出 |
|---|---|---|
| 反思物化 | `stock_reflections.horizon_results_json` 中 `status=mature` 的档 | 25 行（`source_ref` 非空） |
| 单条回测 | cron / replay / 离线评测 | 20 行（`source_ref` NULL） |

### 2.2 ⭐ 成熟判定**不看今天**

`compute_market_snapshots`（`src/commands/stock_workflow/reflection.rs:3850`）：
- 取 **最近 500 根**日 K（`REFLECTION_KLINE_COUNT`），**不带 as-of 参数**；
- `exit_idx = min(klines.len()-1, entry_idx + expected_days)`；
- `within_expected_horizon = exit_idx < entry_idx + expected_days`。

⇒ **「档位成熟」= 行情库里决策日之后有 N 根 K 线**，与今天、与 wall-clock **无关**。

⇒ **回放一个足够早的 as_of 日 ⇒ long 档当场成熟。**

### 2.3 ⭐ 行情库足够长

`market_daily_close`：6575 个交易日（1996-10-14 ~ 2026-10-23），6148 只股票。

决策日之后的 K 线根数：
| 决策日 | 之后根数 | 可判窗口 |
|---|---:|---|
| **2026-05-27** | **100** | **T+90 ✅** |
| **2026-06-10** | **90** | **T+90 ✅**（恰好边界） |
| 2026-06-11 | 89 | T+28 ✅ |
| 2026-08-18 | 42 | T+28 ✅ |
| 2026-09-04 | 29 | T+28 ✅ |
| 2026-09-07 | 28 | T+28 ✅（恰好边界） |
| 2026-09-09 | 26 | T+5 only |
| 2026-09-24 | 15 | T+5 only |
| 2026-09-28 及以后 | ≤14 | T+5 only |

### 2.4 ⭐ 唯一的阻塞点：池快照的双源硬约束

`astock_daily_snapshot` 的历史分布：
| 日期 | 源数 |
|---|---:|
| 05-27 / 06-10 / 06-11 / 08-18 / 09-04 / 09-07 / 09-09 | **1**（只有 `get_industry_ranking`） |
| 09-24 | 1 |
| 09-28 起 | 2（+ `get_hot_stocks`） |

旧判据要求**两源齐全** ⇒ 上面 7 个日期**全部不可回放** ⇒ 只能从 09-28 起 ⇒ long/mid 档永远等不到。

**但「缺一源」≠「池子空」**：
1. 缺的那源在 as-of 下**仍走 vendor 历史通道**（THS 热榜回溯 `hot_list/history/v1/rank`），
   取不到返回**空榜**（`Ok(vec![])`，不是失败）；
   ⚠ 实测该接口只有最近几天有数据（10-22 ✅ / 09-28 ❌ ⇒ `-20001 No data error`）；
2. 建池 = `get_hot_stocks(30) ∪ get_industry_ranking 龙头(20) ∪ FALLBACK_STOCKS`，
   **FALLBACK 是静态大盘股列表**（茅台/五粮液/宁德时代…）⇒ **与日期无关 ⇒ 无前视偏差**。

⇒ **放宽判据 = 「至少一源齐全」，两源都缺仍排除**（那时池子 100% 是 FALLBACK ⇒ 不是「当时的候选池」）。

### 2.5 ⚠⚠⚠ 放宽的**代价**（2026-10-26 实测，**必须如实记录，不得当副作用略过**）

「快照行存在」≠「快照里有可用成分」。实测**有效成分数**（`get_industry_ranking` 看 `leaderCode`、
`get_hot_stocks` 看 `stockCode`）：

| 快照日 | 源 | 行数 | **有效成分** |
|---|---|---:|---:|
| 05-27 ~ 09-24（8 个日） | industry | 18~31 | **0**（`leaderCode` 全为 null，只有行业名+涨跌） |
| **09-28** | hot_stocks | 19 | **19** |
| 09-30 起 | 两个源 | 20~100 | 全量 |

⇒ **09-28 之前的快照虽存在，但一个股票成分都没有。**

**⇒ 放宽只买到「能跑」，买不到「池是当时的候选池」**：
那些日期的池 **100% 是静态 `FALLBACK_STOCKS`**。

**但这个取舍可接受，因为两件事被刻意分开**：

| 用途 | 依赖什么 | 放宽是否解锁 |
|---|---|---|
| **档位成熟**（long/mid 能不能有 T+90/T+28 成熟样本） | **只看 K 线**（决策日之后有没有 N 根） | ✅ **解锁**（这正是 R-3h 要的） |
| **选股能力**（池里的票是否代表当时候选） | 池成分 | ❌ **仍不成立** |

⇒ 这些日期的样本**只用于「档位动作 → 胜率」映射**，**不得**当作「系统选股能力」的证据。
⇒ 批次消息的降级说明必须把这句话说给用户看（已实施）。

**⇒ 顺带修掉一处假绿**：`pool.rs` 的 `industry_succeeded` 判据原是 `!industries.is_empty()`
（**行业列表非空**）⇒ 在「`leaderCode` 全 null」的快照上为 true
⇒ 「仅靠 FALLBACK_STOCKS 兜底」的告警**不触发** ⇒ 池 100% 静态兜底却看起来正常。
已改为 **`taken > 0`**（实际取到的股票数）。

---

## 3. 已实施的改动（2026-10-26）

| # | 文件 | 改动 |
|---|---|---|
| 1 | `crates/dao/src/repo/astock_daily_snapshot.rs` | 新增 `list_snapshot_dates_with_any_method`（**一源**口径）；删除 `list_snapshot_dates_with_all_methods`（**全部源**口径，已无调用方 ⇒ 留着会 dead_code 红 clippy） |
| 2 | `src/commands/time_travel_batch.rs` `require_pool_snapshot_coverage` | 返回 `Result<Option<String>, String>`：两源都缺 ⇒ **拒绝**；缺一源 ⇒ **放行 + 降级说明**（`Some(note)`）；齐全 ⇒ `None` |
| 3 | 同文件 `list_replayable_as_of_dates` | 改用「一源」清单 + **显式排除零源日** |
| 4 | 同文件 `execute_batch` / `run_time_travel_batch_inner` | 新增 `pool_degradation_note` 参数 → **落进批次消息**（不静默：少了一个真实榜源这件事必须可见） |
| 5 | `src/bin/axagent-validate-backfill.rs` | 帮助文本更新（原文「28/90 档时间旅行加速不了」**已被证伪**） |

---

## 4. 实跑验收结果（2026-10-26 08:51~09:10）

### 4.1 已达成 ✅

| # | 判据 | 结果 |
|---|---|---|
| 1 | 06-10 批次**不因池快照被拒** | ✅ `status=completed`，消息含降级说明 |
| 2 | 批次消息含降级说明 | ✅「2026-06-10 缺建池快照之一（get_hot_stocks）⇒ 该源在回放中为空榜…候选池由另一源 + 静态兜底名单构成；⚠ 该日样本只用于「档位动作→胜率」映射，**不得**当作选股能力的证据」 |
| 3 | `reco_picks` 新增 `as_of_date=2026-06-10` 的 **long 档**票 | ✅ **5 条**（style=serenity，conf 58.8）＋ mid 5 条（conf 56.0）＝ 共 10 条 |
| 4 | `decision_validations` 出现 `period=long AND t_plus_n=90` | ✅ **5 行**（**全库首次**；另 mid 5 行 ⇒ T+90 共 10 行、T+28 共 48 行） |
| 5 | T+90 的 exit 价可信 | ✅ 交叉验证：与库内「第 90 根 K 线收盘」偏差 −0.15% ~ +2.59%（复权口径差异） |

### 4.2 ❌ 未达成：校准样本仍不增长（**下一环的缺陷**）

`strategy_performance` 逐档已判定样本 **未变**（short 18 / mid 7 / ultra 7 / long 0）。

**根因链**（全部实测）：
```
回放 06-10 → 产生 long 档决策 → 建 pending 反思行
   └─ 但 pending 行的 hindsight_date = 决策日 + 90 交易日 = 2026-10-23
      └─ 到期后由 reflection-due-daily（18:00）消费
         └─ ⚠ 反思链整体失败：346 条反思里只有 2 条 completed
            ├─ 228 条 failed: REFLECTION_OUTCOME_MISSING（07-21 ~ 10-23）
            ├─ 5 条 failed: REFLECTION_OUTPUT_INCOMPLETE
            └─ 98 条 pending（其中 28 条已到期未判）
```

⇒ **反思链是死循环的最后一环**：它不产出 ⇒ `strategy_performance` 不增长 ⇒
校准永不达标 ⇒ 置信永久封顶 ⇒ 顶层 0 买入。

**⚠ 已排除的解释**（避免下轮重复查）：
| 假设 | 实测 | 判定 |
|---|---|---|
| pending 被 `max_count=200` 截断 | pending 池仅 98 条；06-10 的 4 条在 due 队列里排名 **8~11** | ❌ 排除 |
| `hindsight_date` 未到期 | 06-10 的 4 条 hindsight=**2026-10-23** ≤ 今天 | ❌ 排除 |
| 原分析缺失 | 4 条的 `original_analysis_id` **都存在**（`analysis_kind=replay`） | ❌ 排除 |
| 档位筛选不匹配 | 反思行 `horizon=long`；cron 的 `period=None` ⇒ 不过滤 | ❌ 排除 |
| K 线窗口不足 | 4 只票从 06-11 起各 **90 根**（= 需要值，边界够） | ❌ 排除 |
| `min_confidence` 门槛 | cron 的 `min_confidence` 未设（默认 0） | ❌ 排除 |

⇒ **下一步必须实跑一轮反思拿日志**（`axagent-batch-rerun --reflect-after` 会跑全量 pending），
定位 `REFLECTION_OUTCOME_MISSING` 的真实产出点 —— 该状态串**不在源码里**（grep 全仓无命中），
说明它**不是编译期常量**，而是运行时从某处透传上来的错误文本 ⇒ 必须看日志。

### 4.3 ⚠⚠ T+90 的读数不可作「盈利能力」结论

| as_of | period | t_plus_n | rows | avg_net | net_win |
|---|---|---:|---:|---:|---:|
| 2026-06-10 | long | 90 | 5 | **−14.92** | 40.0% |
| 2026-06-10 | mid | 28 | 5 | −11.75 | 0.0% |

**n=5，且池 100% 是静态 FALLBACK_STOCKS** ⇒ 这 5 只票**不代表任何选股能力**
⇒ **只作「该档动作 → 胜率」映射的机制验证**，**不得**读作盈利能力证据。

### 4.4 口径澄清（本轮的一次自我误判，勿重犯）

⚠ 我一度怀疑「T+90 的 exit 价取错了窗口」（记录 91.60 vs 我算的 09-27 的 110.40）。
**错在我自己**：我用「全市场交易日历」数第 90 根，却按「06-11 起」的**日历日**推算；
而库内 06-11 → 10-23 **就是 90 个交易日**（中途无休市）⇒ 第 90 根 = **10-23**（close 91.73）⇒
**代码是对的**，偏差仅 −0.15%（复权口径）。
⇒ **教训**：数「第 N 根」必须用**该序列自身**的行数，不能用外部日历推算 ——
与「量纲跟尺度」「同量被两套判定用不同口径消费」同族。

---

## 5. 预期效果与残余风险

### 5.1 预期
- **long 档校准样本从 0 → 可累积**（每次回放 06-10 产出一批 long 档成熟样本）；
- 累积到 30 条 ⇒ `calibrationReady=true` ⇒ 置信不再封顶 ⇒ **long 档恢复出买入信号**；
- mid 档同理（09-07 / 09-04 / 08-18 / 06-11 四个可判日）。

### 5.2 ⚠ 残余风险（必须如实记录，不得静默）

| 风险 | 说明 | 缓解 |
|---|---|---|
| **R1 池成分不同构** | 06-10 的池 = industry 龙头(20) + FALLBACK(约80)，**无热股榜**；与「当时真实候选池」不同 | 已在批次消息留痕；校准样本只统计**档位动作 → 胜率**的映射，不直接当选股能力证据 |
| **R2 长期档样本天然稀少** | long 档每次回放只产 3~4 条（serenity 链）；30 条需多次回放 | 回放可重复跑；且 long 档实盘票也在累积（10-02 起 191 条） |
| **R3 mid/long 档只有 serenity** | `serenity_tiers = [Mid, Long]`（产品定位）⇒ mid/long 的「真策略票」只有 serenity | 非缺陷；但 serenity 的 `period` 是**链内档位**，其 `tier_period` 来自 `serenity_tiers` ⇒ 口径一致 |
| **R4 门槛不变 ⇒ 达标仍需时间** | 30 条样本要跑够批次 | 每日 cron + 手动回放 06-10/09-07 两条路径并行 |

---

## 6. 与其它缺陷的关系

| 编号 | 状态 |
|---|---|
| **R-3h**（本文） | 🔄 实施中 —— 门槛不变，从样本供给侧解 |
| **R-3i**（448 行孤儿） | ✅ 已修（数据侧恢复 224 行 → 去重合并 → 孤儿归零；新增门禁 `scripts/check-decision-validation-integrity.mjs`） |
| **R-3j**（期窗错配行） | ✅ **判定为设计行为**（`decision_validations` 是多窗口笛卡尔积表）；按档统计须只用匹配窗口（门禁已加判据） |
| **R-3l**（顶层与档内脱节） | ✅ **判定为设计行为**：4 条是选档规则正常（选了另一档）、3 条是 `R-CALIB-CAP` 校准未达标 ⇒ 动作联动降级。**R-3h 解决后这 3 条会自然消失** |
| **R-3f**（硬编码 14） | ✅ 已修（`reversion.rs` 背离检测的 `rsi_period` 走 `bars_for_daily_span`，daily 恒等 ⇒ 零回归；新增可执行判据 `divergence_rsi_period_and_lookback_share_one_scale`） |
| **R-3e**（as-of 非日线尺度） | ✅ **可关闭**：mid/long 档的「零真策略票」是设计行为（该档策略 = serenity）；已在 `reversion` 3 处 + `scale::fetch` 2 处修过量纲/根数 |
| **R-3k**（cron 空转） | ✅ **撤销**（播种 10-25 23:39，`next_run_at`=10-26 08:30，首次触发未到，非缺陷） |
