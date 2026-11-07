// SPDX-License-Identifier: AGPL-3.0-only

# PLAN-horizon-scale-alignment —— 四档「持有交易日 → 尺度 → 窗口」全库单源

> 状态：active ｜ 日期：2026-10-18 ｜ 对应版本：v2.9.9


> **2026-10-18 用户裁定**：**长期 = 90 个交易日**，不是「季线」。全库严格对齐。
> 上一轮把长期档的取数需求描述成「60 根季线」是错的 —— 那是把**尺度名**当成了**持有期**。
> 本方案的目标不是「把 min_bars 从 60 调到 6」，而是让**全仓只保留一条推导链**。

---

## 一、唯一口径（本方案的判据，不是偏好）

**规则 R（定阶）**：每根 bar 覆盖的交易日 `d` 必须 ≤ 本档持有期 `h / 3`
⇒ 至少 3 根 bar 才能分辨该持有期内的走势；`h/d < 3` 的尺度对本档**按构造不可用**。

| 档 | `h`（交易日，权威 = `Period::default_holding_days`） | 命中尺度 | `d` | `h/d` |
|---|---|---|---|---|
| `ultra_short` | 2 | `hourly` | 0.25 | 8.0 |
| `short` | 5 | `daily` | 1.0 | 5.0 |
| `mid` | 28 | `weekly` | 5.0 | 5.6 |
| `long` | **90** | `monthly` | 20.0 | 4.5 |

被这条规则排除的（现役阶梯里正在用的两个）：

| 现役组合 | `h/d` | 后果 |
|---|---|---|
| `mid` + `monthly` | 1.4 | 「中线」只由 **1 根月线**决定走势方向 |
| `long` + `quarterly` | **1.5** | 「长线」只由 **1.5 根季线**决定；而季线本体的 `min_bars=60` 又要 **15 年**历史（见 D2） |

`quarterly` 不删（`ScaleProfile` / `aggregate_monthly_to_quarterly` 仍可被手工 `compute_scoring(period="quarterly")` 使用），
但**不再有任何档位落在它上面** ⇒ 档位上永远不会触发「月线聚合」这条重路径。

**唯一推导链**（改任何一环都必须同批改）：

```
Period::default_holding_days()           # harness: 2 / 5 / 28 / 90
  → Period::scale_key()                  # harness: hourly / daily / weekly / monthly
    → ScaleWindowPlan::for_period()      # astock-data: 按 h/d 换算 bar 窗口
      → 取数根数 / 出分下限 / 指标窗口 / 趋势窗口 / 按档风险窗口
```

---

## 二、现状：五处第二份真相（每条都实测过形态）

| # | 位置 | 形态 | 后果 |
|---|---|---|---|
| **D1** | `crates/harness/src/holding_period.rs:330-337` vs `crates/analysis-engine/src/recommender/scale.rs:21-28` | **两套档→尺度阶梯**：分析链 `hourly/weekly/monthly/quarterly`，荐股链 `hourly/daily/weekly/quarterly` | 同一个「短期」在两条链上不是同一尺度，`short`/`mid` 各差一档，且各自都有注释自称「目标架构」 |
| **D2** | `recommender/scale.rs:56-58` + `crates/astock-data/src/scale.rs:55` | 荐股出分下限用 `ScaleProfile::min_bars = 60`（**聚合后**计数） | 季线 = 60 根 = **约 15 年月线**；实测 300567（120 根月线→40 季）、688012（88→29）、301302（53→17）全部整档拒 ⇒ **长期档恒空**。同一概念在分析链是 `ScaleWindowPlan::min_bars` = 6 |
| **D3** | `recommender/strategies/trend.rs:44-87` | `windows_for` 手抄四档窗口，且写死「短=日线 5/10/20、中=周线 4/8/13、长=季线 3/6/8」 | 尺度一改，窗口立刻与口径脱节（它抄的正是 D1 里荐股那份旧阶梯）；这是「换个条数冒充换尺度」的同族 |
| **D4** | `crates/astock-data/src/mcp_tools.rs:2205` | `[("ultraShort",2usize),("short",5),("mid",28),("long",90)]` | 按档风险窗口的第二份天数表；另有 `recommender/scoring.rs:561` 测试里同款 |
| **D5** | `horizon_tier_template.rs:224/352/468/590` + `harness:344-351` | 四档评分节点 `t-scoring-{hour,week,month,quarter}` 与 `const-scoring-period` 字面量绑在旧阶梯上 | 阶梯改了而模板不改 ⇒ 短档仍吃周线、长档仍吃季线（正是 §九十二 那条「死参数」的复现形态） |

补充（本轮**不**改，登记）：`strategies/capital.rs:57-58` 的两条窗口经 `bars_for_daily_span` 换算后各自 `.max(5)`，
在 `weekly/monthly` 上会双双落成 5 ⇒ `vol_ratio ≡ 1.0 < 1.5` ⇒ capital 在中/长档**按构造永不出票**。
属风格自家窗口下限问题，与档位尺度解耦，需产品裁定，另开。

---

## 三、目标窗口表（`ScaleWindowPlan::for_period` 换算结果）

短带 = `ceil(h/d)`（下限 2），长带 = `3 × 短带`，`min_bars = 长带`，`fetch_limit = 2 × 长带`。

| 档 | 尺度 | 短带 | 长带 | RSI | MACD | BOLL | 量能 | `min_bars` | `fetch` | 旧读数（对照） |
|---|---|---|---|---|---|---|---|---|---|---|
| 超短 | hourly | 8 | 24 | 6 | 5/10/4 | 8 | 2 | 24 | 48 | 不变（本来就是 hourly） |
| 短 | **daily** | 5 | 15 | 4 | 3/7/2 | 5 | 2 | 15 | 30 | 旧 weekly：2 / 6 / min 6 / fetch 12 |
| 中 | **weekly** | 6 | 18 | 4 | 4/8/3 | 6 | 2 | 18 | 36 | 旧 monthly：2 / 6 / min 6 / fetch 12 |
| 长 | **monthly** | 5 | 15 | 4 | 3/7/2 | 5 | 2 | **15** | 30 | 旧 quarterly：2 / 6 / min 6 / fetch 12 + 聚合前 36 根月线 |

**长期档可用性**（本轮要解决的空档）：`min_bars = 15` 根月线 ≈ 15 个月历史 —— 300567（120 根）、
688012（88）、301302（53）全部过线；仅 688806（4 根）仍拒，那是次新股本身没有 15 个月历史，属**该拒**。

**趋势窗口推导**（`trend.rs`，替换手抄表）：

```
anchor = plan.long_bars            # = 3 × 短带，覆盖 3 倍持有期
slow   = max(2, 2/3 × anchor)
fast   = max(2, 1/3 × anchor)
high_window = amount_window = plan.short_bars
```

| 档 | fast/slow/anchor | 高点/量能 | 旧读数 |
|---|---|---|---|
| 超短 | 8 / 16 / 24 | 8 / 8 | 8 / 16 / 40，8 / 20 |
| 短 | 5 / 10 / 15 | 5 / 5 | 5 / 10 / 20，20 / 20 |
| 中 | 6 / 12 / 18 | 6 / 6 | 4 / 8 / 13，8 / 13 |
| 长 | 5 / 10 / 15 | 5 / 5 | 3 / 6 / 8，4 / 8 |

---

## 四、实施分片

### P0 权威：改 `Period::scale_key` / `scoring_node_id`
文件：`crates/harness/src/holding_period.rs`

- `scale_key()`：`Long => "quarterly"` → `"monthly"`；`Mid => "monthly"` → `"weekly"`；`Short => "weekly"` → `"daily"`。
- `scoring_node_id()`：`hour` / **`day`**（新）/ `week` / `month`；`t-scoring-quarter` 退役。
- 文档注释补「规则 R」，并写明**为什么不是季线**（90/60 = 1.5 根，按构造不可分辨持有期）。
- 测试 `scale_key_and_scoring_node_are_one_table_per_tier` 的期望表同步。

### P1 窗口计划：抽 `for_scale_and_holding_days`
文件：`crates/astock-data/src/scale.rs`

- 把 `for_period` 的换算体抽成 `pub fn for_scale_and_holding_days(profile, holding_days)`，
  `for_period` 只负责取权威后转调。理由：`quarterly` 不再被任何档命中，
  若不抽出来，`parent_fetch_limit` 那条分支会**变成无人可测的死代码**。
- 测试：`quarterly_plan_carries_parent_fetch_and_others_do_not` 改为
  「四档都不落 quarterly ⇒ `parent_fetch_limit` 全 None」+「直接给 quarterly 剖面仍算出 3×fetch」。

### P2 荐股尺度层：删掉 D1 那份阶梯 + 接 P1
文件：`crates/analysis-engine/src/recommender/scale.rs`

- `scale_name()` 保留为**转发**（`period.scale_key()`），不再自带四臂表。
- `profile_for()` 走 `period.scale_key()`。
- `fetch()` 的取数根数改由窗口计划给：`plan.parent_fetch_limit.unwrap_or(plan.fetch_limit)`。
- `enough_bars(bars, period)` 改签名 → `bars.len() >= ScaleWindowPlan::for_period(period).min_bars`。
  **不再读 `ScaleProfile::min_bars`**（它仍是「尺度自身的粗下限」，只服务无档的手工调用）。

### P3 荐股策略：窗口改推导 + 5 处调用点
文件：`recommender/strategies/{trend,capital,value,reversion,watchlist}.rs`

- `enough_bars(&klines, &profile)` → `enough_bars(&klines, self.period)`（5 处）。
- `trend.rs::windows_for` 改为按上表推导；`require_high_breakout` / `require_macd` 两处**语义开关**保留逐档显式（它们不是窗口）。
- 注释里「超短=60 分钟、短=日线、中=周线、长=季度」改为指向权威。

### P4 模板与节点名（需重种子）
文件：`commands/stock_analysis_setup/horizon_tier_template.rs`

| 档 | 旧节点 id / 标题 / 常量 | 新 |
|---|---|---|
| 超短 | `t-scoring-hour` / 60 分钟 / `"hourly"` | 不变 |
| 短 | `t-scoring-week` / 周线 / `"weekly"` | `t-scoring-day` / 日线 / `"daily"` |
| 中 | `t-scoring-month` / 月线 / `"monthly"` | `t-scoring-week` / 周线 / `"weekly"` |
| 长 | `t-scoring-quarter` / 季线 / `"quarterly"` | `t-scoring-month` / 月线 / `"monthly"` |

- 改名必须**先占位再落名**（`quarter→@A`、`month→@B`、`week→@C`，再 `@C→day`、`@B→week`、`@A→month`），
  否则 `short` 的 `t-scoring-week` 会与 `mid` 的目标名撞车。
- 边 id 内含节点 id（`e-t-scoring-week-pm-h-short` / `e-const-scoring-period-t-scoring-week`），同批替换。
- `TEMPLATE_VERSION` 165 → **166**（`HORIZON_TIER_TEMPLATE_VERSION` 自动跟随）。

### P5 天数第二份表
文件：`crates/astock-data/src/mcp_tools.rs:2205`
`[("ultraShort",2),…]` → 由 `Period::ALL` 现推（`p.camel_key()` + `p.default_holding_days()`），注释改「权威 = harness」。

### P6 门禁
- `scripts/check-reco-horizon-parity.mjs` 新增 **门 n**（字母已排到 m，故取 n；**不是**门 g ——
  门 g 是「矩阵理由码 11 语言」）：`recommender/scale.rs` 内出现 `Period::X => "…"` 四臂尺度表
  ⇒ 红（阶梯只允许在 harness 一处）；含正负控进 `--selftest`。
- `check-tier-purity.mjs` 的 R5 读 harness 自动跟随，但四个 `t-scoring-*` id 是**现场收集** ⇒ 需确认收集面仍为 4 个（`hour/day/week/month`）。
- 其余门（`check-input-mapping` / `audit-inject-coverage` / `seed_consistency_tests`）按字面量读，同批对账。

---

## 五、验证清单

读数栏口径：`✅` = 本轮实跑通过并记下读数；`⏳` = 因 `target` 目录被外部并发 cargo
进程独占（`error: failed to open .cargo-build-lock / os error 5`）尚未跑，**不得**当作通过。

| 项 | 判据 | 读数 |
|---|---|---|
| `cargo test -p axagent-harness` | 阶梯黄金表测试过 | ✅ 734 passed / 0 failed（`scale_key_and_scoring_node_are_one_table_per_tier` 等） |
| `cargo test -p axagent-astock-data --lib` | 窗口计划 + `horizon_window_plan_dump` 输出与 §三 表逐格一致 | ✅ 612 passed / 0 failed；`--ignored --nocapture` dump 逐格 = §三（超短 8/24、短 5/15、中 6/18、长 5/15；日线零回归锚 20/60 不变） |
| `cargo test -p axagent-analysis-engine --lib` | 尺度层/策略测试过 | ✅ 807 passed / 0 failed（含 `enough_bars_uses_tier_min_bars_not_the_scale_floor`） |
| `node scripts/check-tier-purity.mjs --selftest && node scripts/check-tier-purity.mjs` | R5 四档来源 = hour/day/week/month | ✅ 自证 17 条；实跑 R5 通过；`--dump` 四档评分源 = `t-scoring-hour/day/week/month` |
| `node scripts/check-reco-horizon-parity.mjs --selftest && node scripts/check-reco-horizon-parity.mjs` | 门 n 双控通过 | ✅ 自证 42 条 PASS（含门 n 两负控一正控）；实跑 14 门全 `OK` |
| `node scripts/check-indicator-config-scope.mjs --selftest` | 夹具与模板字面量同源 | ✅ 自证 5 负控 + 1 正控通过 |
| `node scripts/check-input-mapping.mjs` | 子模板映射零悬空 | ✅ `RESULT: OK`（入度0/不可达/死端全 0） |
| `node scripts/audit-inject-coverage.mjs` | 被引用变量全部有来源 | ✅ 差集为空（`screening_source` 悬空为既有登记项，非本轮） |
| `node scripts/check-horizon-{label,weight,decision-keys}.mjs` | 档名/权重/决策键三面一致 | ✅ 三连全绿 |
| `node scripts/check-variable-consumer-registry.mjs` | 声明↔引用对账 | ✅ 349 声明 / 零引用面 0 |
| 前端 `tsc --noEmit` | `src/types/stock-analysis.ts` 注释改动不破类型 | ✅ 退出码 0 |
| `cargo clippy -p axagent-analysis-engine -p axagent-astock-data -p axagent-harness --all-targets -- -D warnings` | 0 警告 | ✅ 2m34s 全 `Finished`，零警告 |
| `cargo test -p axagent --lib` | 模板形状 / 负控 / 常量对账测试过 | ✅ **696 passed / 0 failed**；⚠ 必须带 `__TAURI_WORKSPACE__=true`（首跑漏带 ⇒ 0xC0000139 假崩，`MEMORY-DETAIL.md` §87）；另修负控 A 一处漏改名（断言仍找旧串 `monthlyx`，注入已改 `midx`） |
| 重种子后实测 | 智能荐股四档同批返回，`long` 档 `raw_picks > 0` | ⏳ 待重种子（实机动作，属下轮） |

---

## 六、本轮不做（登记，不混做）

1. **价格带（entry/stop/target）仍按百分比**：尺度归一属 Phase R-D（波动率风控），与窗口无关。
2. **`capital` 中/长档窗口下限（`.max(5)` 使两窗相同）**：见 §二 补充，需产品裁定。
3. **前端「档级空态」与 `absence_reason`**：属展示层，另开（原 `PLAN-serenity-horizon-adaptation.md` G4）。
4. **历史数据回填**：旧分析按 v165 图产出，不重算；`TEMPLATE_VERSION` 升代后新分析才吃新窗口。
