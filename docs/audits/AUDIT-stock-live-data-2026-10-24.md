# 股票业务生产库核验（2026-10-24）

> 状态：done ｜ 日期：2026-10-24 ｜ 对应版本：v2.9.9


> 本文回答 `AUDIT-stock-accuracy-2026-10-24.md` §8.9 的 X-1～X-4。
> 数据来源：`scripts/pg-connect.mjs`（口令在 `~/.axagent` 解密，未进 argv/源码），
> 库 `axinvest`，PostgreSQL 18.4。**含两次带备份的生产数据修复**（见 §4.1）；
> 权重表 `strategy_weight_history` / `reco_picks` 未改。

> **⚠ 2026-10-24 更正**：初版 X-1 的 158 单 / 42.4% 被「提前验证」污染，已作废。
> 修复后成熟样本 127 单 / 42.5% / +0.72%；mid、long **没有成熟样本**。
> 详见 §4。

## 0. 结论先行

| 项 | 结论 |
|---|---|
| X-4 模板版本 | 生产库 **177 / 61 / 1**，代码 **179 / 62 / 2** ⇒ 三条模板都是「代码更新」，下次播种会重建；不存在「DB 版本高于代码导致静默跳过」 |
| X-2 候选宇宙 | 清单 6,159，申报 5,925；**最新交易日 5,569 只已交易票全部在清单内（0 缺失）**。590 只无收盘的行是停牌/退市/迁 920 的老码，不是采集漏票 |
| X-3 决策 schema | 328 条 `decision_json`：321 条五个核心字段完整，**7 条（2.1%）缺 action/confidence/positionPct/riskLevel/reasoning**，全是 2026-07 的 `end-output` 空决策；新代码在详情里显式报 `decisionSchemaGaps`，并把 required 写进输出 schema |
| X-1 真实命中率 | 初版 158 单/42.4% **作废**（1,145/1,654 行提前验证）。隔离后成熟样本 **127 单：hit 54（42.5%）、partial 27、miss 45、false_hit 1，均值 +0.72%**；只有 short/ultra 可评估，mid/long 0 条成熟样本 |

## 1. X-4：生产库模板版本

```sql
SELECT id, version FROM workflow_templates
WHERE id IN ('stock-analysis','serenity-screening','serenity-screening-fast');
```

| 模板 | 生产库 | 代码 | 谁更新 | 下次播种 |
|---|---:|---:|---|---|
| stock-analysis | 177 | **179** | 代码 | 重建（`existing.version >= TEMPLATE_VERSION` 不成立） |
| serenity-screening | 61 | **62** | 代码 | 重建 |
| serenity-screening-fast | 1 | **2** | 代码 | 重建 |

结论：X-4 的「读不到生产库」已消除；三条模板都处于「代码待落库」状态，不存在 R-6 的
「DB 更高 ⇒ 永久跳过」。代码侧再把 `stock-analysis` 从 178 抬到 179（R-4 schema required），
落库后本次三项修复会一起生效。

## 2. X-2：候选宇宙与收盘覆盖

### 2.1 实测

| 指标 | 值 |
|---|---:|
| `market_stock_universe` | 6,159 |
| `market_universe_stats.reported_total` | 5,925 |
| 最新收盘 `2026-10-23` 的 distinct 代码 | 5,569 |
| 已交易票中不在清单内的数量 | **0** |
| 最新收盘缺失的清单代码 | 590 |
| 其中前一交易日（10-22）有交易 | **0** |
| 其中从未有过收盘行 | 33 |
| 其中最后交易 < 10-22（停牌/退市/迁码） | 216 |

分市场（清单 / 最新收盘）：

| 板块 | 清单 | 收盘 |
|---|---:|---:|
| main_sh | 1,847 | 1,698 |
| main_sz | 1,647 | 1,494 |
| chinext | 1,454 | 1,408 |
| star | 621 | 616 |
| bj | 590 | 353 |

BJ 的 590 里有 234 个是 83/87 老码（2025 年迁 920 号段）；抽查 `830799 艾融软件`
在腾讯接口只有 09:00 的零成交报价、库内历史 0 行，属**已迁码/停牌**，不是采集失败。

### 2.2 判据修正

`coverage_pct` 旧公式 `universe_size / reported_total` 在清单是申报超集时输出 **103.95%**
（6,159 / 5,925）。覆盖率 >100 不是「覆盖更好」，而是两个集合不同源。已改为
`coverage_pct_of()`：0 分母→0，结果 **clamp 到 100**；带单测（103.95→100、53.15 保留、
0 分母→0）。

覆盖率门 `scripts/check-universe-coverage.mjs` 用真实读数跑出的唯一红条是
C6：5,569 / 5,925 = 94.0% < 95%。这条红条的正确解释是 **356 只申报代码当日没有成交**
（0 只在前一交易日有成交、33 只从未成交），不是「荐股看不见 356 只已交易票」。
「已交易票 0 缺失」已单独取证。门本身保留红条作为「停牌占比异常」的观察，不把它
伪造成绿色。

## 3. X-3：决策 JSON 的必填形状

### 3.1 生产审计

```sql
SELECT count(*),
 count(*) FILTER (WHERE NOT (decision_json::jsonb ? 'action')),
 count(*) FILTER (WHERE NOT (decision_json::jsonb ? 'confidence')),
 count(*) FILTER (WHERE NOT (decision_json::jsonb ? 'positionPct')),
 count(*) FILTER (WHERE NOT (decision_json::jsonb ? 'riskLevel')),
 count(*) FILTER (WHERE NOT (decision_json::jsonb ? 'reasoning'))
FROM stock_analyses WHERE decision_json IS NOT NULL AND decision_json <> '';
```

| 总数 | 缺 action | 缺 confidence | 缺 positionPct | 缺 riskLevel | 缺 reasoning |
|---:|---:|---:|---:|---:|---:|
| 328 | 7 | 7 | 7 | 7 | 7 |

7 条全部是 `2026-07-22 ~ 07-24` 的 `end-output` 空决策（`{"node_id":"end-output","output":null,"status":"terminated"}`
或只有 `adjustedConfidence/agreementBreakdown`），但 `status='completed'`。
这证明 R-4 不是理论风险：**2.1% 的已完成分析没有可读决策**。

### 3.2 修复（非破坏）

1. `seed_stock_analysis` 的 output_schema `required` 由 `None` 改为
   `action / confidence / positionPct / riskLevel / reasoning`；`targetPrice` / `stopLoss`
   对持有/观望本就可空，不列必填。`TEMPLATE_VERSION` 178→179。
2. `decision::decision_schema_gaps()`：解析已落库 `decision_json`，返回缺失/空串/越界
   字段；`get_stock_analysis` 在响应里附 `decisionSchemaGaps`（空缺口不写键）。
   不拒绝、不改库 —— 让「这条决策不完整」在详情/审计里显式可见。
3. 单测：完整决策 0 缺口；空串/101/-1 全报；`None`/空白/坏 JSON 分别报
   `decision_json` / `decision_json:unparseable`。

## 3.5 反思链路：completed 但没有实际结果/verdict

生产 `stock_reflections` 325 行：

| status | n | actual_outcome='' | verdict=NULL/'' |
|---|---:|---:|---:|
| completed | 230 | **228** | **222** |
| pending | 85 | 85 | — |
| running | 2 | 2 | — |
| failed | 8 | 8 | — |

**228/230 条 completed 反思没有实际走势**。根因：
`build_pending_reflection_rows_for` 建行时 `actual_outcome=''`；
`run_reflection_workflow` 的 UPDATE 路径从未回写 `ActualOutcome` 列（只写 status/
decision_json/lesson 等），只有手动 INSERT 分支写。于是分档定时反思全部“盲反思”。

**222 条 verdict 为空**，根因有三层：
1. reflection-validator 不拆 `{"report":"{\"verdict\":...}"}` 包装 ⇒ 字段取不到；
2. 取不到时 validator 默认 `verdict="partial"`，而落库路径又没跑通，最终 NULL；
3. validator 用 `lesson_full.chars().len()`，Rhai 的 `chars()` 没有 `len()` ⇒ 运行时
   `ErrorFunctionNotFound("len (range)")`，校验器只要遇到非空 lesson 就崩；
4. 完成状态只看 reflection-agent 节点 `Completed`，不看输出是否有 verdict ⇒
   「节点跑完但没产出」也盖 completed。

修复：
- validator 先拆 `report`；缺 verdict 输出空串 + `status=incomplete`，不再默认 partial；
- 去掉 `chars().len()`，改 `lesson_full.len()` + 逐字符截断；
- 完成路径优先读 `reflection-validated`，`has_verdict=false` 时写
  `failed: REFLECTION_OUTPUT_INCOMPLETE`，不再盖 completed；
- 完成 UPDATE 补 `ActualOutcome` 列；行情不可用且无人工结果 ⇒
  `SkippedNoMarketData` 保持 pending，不盲反思；
- 回归测试直接在生产同配置 Rhai 引擎里执行 `reflection_validator.rhai`（report 包装 +
  缺 verdict 两态）。

历史数据处理（2026-10-24，带 `stock_reflections_backup_20261024` 备份）：
- 212 条从 `decision_json->report` 回填 verdict / lesson_summary / what_went_wrong 等；
- 228 条 `completed` 但 `actual_outcome=''` 统一改为
  `failed: REFLECTION_OUTCOME_MISSING`（不再算有效反思）；
- 219 条 `reflection_lessons` 中 38 条 `active` 来自这些盲反思 ⇒ 统一
  `retired: REFLECTION_OUTCOME_MISSING`（带 `reflection_lessons_backup_20261024`）；
- 处理完 `completed` 只剩 2 条（都有实际走势），活跃教训只剩 1 条，不再出现
  “完成但盲反思”的统计与“盲教训继续进 RAG”。

## 4. X-1：真实命中率——初版读数被「提前验证」污染

> **更正（2026-10-24 12:xx）**：初版写的「158 单、42.4%、-0.28%」**不成立**。
> 复核发现 `decision_validations` 里 1,145/1,654 行是在窗口**没走完**时写下的：
> `fetch_window_series` 用 `min(t_plus_n, 可用K线)`，于是 T+90 只有 26 根 K 线时
> 也写成 T+90；同时 `build_pick_validation` 把 `generated_at` 写成回测跑批时刻，
> 1,276 行的日期与真实 pick 不符。初版表格把这两种行当成了真实 T+N 结果。

### 4.1 数据修复与真实样本

已做（生产库，带备份）：
- `decision_validations_backup_20261024`：改日期前全表备份；
- `decision_validations_backup_20261024_premature`：隔离提前行前全表备份；
- 按 `reco_picks.generated_at` 回填 1,276 行真实生成时间；
- 按 `market_daily_close` 实际交易日数复核，**1,145 行** `t_plus_n` 窗口不足 ⇒
  `hit_outcome='insufficient'`、价格/收益字段置 NULL、`data_source += '+premature_window'`。

隔离后，期窗匹配且成熟的买入样本只剩 **127 单**：

| 样本 | hit | partial | miss | false_hit | 均值收益 | strict 胜率 |
|---:|---:|---:|---:|---:|---:|---:|
| 127 | 54 | 27 | 45 | 1 | **+0.72%** | **42.5%** |

按格拆开（只有 short / ultra_short 有成熟样本）：

| 格 | 成熟样本 | strict 胜率 | 均值收益 |
|---|---:|---:|---:|
| capital_short | 40 | 47.5% | **+3.41%** |
| value_short | 29 | 55.2% | **+0.69%** |
| trend_short | 12 | 25.0% | -1.68% |
| capital_ultrashort | 30 | 36.7% | -0.89% |
| value_ultrashort | 10 | 40.0% | -0.89% |
| trend_ultrashort | 6 | 16.7% | -1.57% |

**mid / long 全部 0 条成熟样本**。初版写的 `capital_long -2.56`、`capital_mid -4.04`、
`reversion_mid -2.69`、`trend_mid -4.86`、`reversion_short -0.83` 全部来自提前行，
不能作为策略结论。

### 4.2 两个独立根因

**根因 A（更严重）：回测把未成熟窗口写成 T+N。**
`fetch_window_series` 的 `min(t_plus_n, valid_klines.len())` 让「只有 26 根」的 T+90
行带上了 26 日收益；`build_pick_validation` 又把 `generated_at` 写成 `now()`。
命中率、闭环、反思全部拿这批错标行当证据。

**根因 B：partial 被闭环计 win。**
在**成熟**的 `capital_short` 40 单里：19 hit + 5 partial + 15 miss + 1 false_hit。
旧闭环把 partial 计 win ⇒ 60% 胜率；strict 只有 47.5%。`reversion_short` 的 22 条
T+5 全是提前行（0 条成熟），旧权重 0.906 没有一条成熟样本支撑。

**根因 C：无样本格的先验是 1.0。**
旧 `reset_prior` 若清空 prior，缺失键会落回 `BASELINE_WEIGHT=1.0` ⇒ 没有成熟样本的
`reversion_short` / `trend_mid` 反而拿满权重。这是「越没证据越敢推」的结构性错误。

### 4.3 修复

1. `fetch_window_series`：`valid_klines.len() < t_plus_n` 直接报错，**不再用短窗口冒充 T+N**。
2. `fetch_and_validate`：把 `validation.generated_at` 覆盖回真实 pick 生成时刻。
3. 回测命令对外报告改 `compute_period_matched_report`（此前这里仍池化 2/5/28/90）。
4. `RecoLoopSample.win` 只认 `hit`；partial 保留为有效样本但不计 win。
5. 语义版本 `RECO_LOOP_LOGIC_VERSION=2`：重置时全部矩阵格先给**中性 0.5**
   （`RECO_LOOP_RESET_NEUTRAL_WEIGHT`），不是清空成 1.0；有成熟样本的格再由胜率/IC
   校准，无样本的格不再满权重。
6. 负期望硬停：`n≥10` 且期窗均收益 ≤-1% ⇒ 权重 0。它只对**成熟样本**生效 ——
   数据修复后 mid/long 没有成熟样本，不会被误停。

### 4.4 修复后的真实结论

- 当前仅有 short / ultra_short 可评估：`capital_short` 是唯一强正期望格
  （40 单、+3.41%、47.5%）；`value_short` 弱正（+0.69%）；其余 short/ultra 为负。
- mid / long **没有成熟样本**，不能说它们「已亏损」，也不能继续用旧权重 0.535/0.168
  当证据；重置后它们应回到中性 0.5，等 T+28/T+90 真正走完再评。
- `reco_min_confidence=50` 与「权重是乘数」仍冲突：中性 0.5 × base 70 ≈ 35 < 50，
  多数格会被置信度门挡掉。阈值应降到 15–20，或把无样本格先验压到 0.3 以下；
  属产品裁决，本轮未改生产设置。

### 4.5 若只保留两个正期望格（仅作重排，不是承诺）

成熟样本里 `capital_short` + `value_short` 合计 69 单，strict 命中 35（50.7%），
均值 **+2.27%**。这是同批历史样本的重排；mid/long 没有成熟样本，不能外推。

### 4.6 趋势智选（同样剔除提前行）

`style='serenity'` 的 378 条验证里，剔除提前行后只剩 **98 条成熟样本**：
mid 80 条（excess_outperform 13、excess_underperform 14、outperform 28、underperform 25），
long 18 条（1 / 4 / 7 / 6）。整体均收益约 **-1.54%**，不是初版写的 -13.35%；
`excess_underperform` 子集均值 -10.5% 仍是最差的一组。
「为什么追高会亏」仍需用 `reco_scan_audit` + K 线特征归因，不在本轮拍脑袋改选股公式。

## 5. 验证状态

| 项 | 状态 |
|---|---|
| 生产库查询 | ✅ 已完成（上述全部读数） |
| 生产数据修复 | ✅ `generated_at` 回填 1,276 行；提前窗口隔离 1,145 行；两张备份表 |
| `cargo fmt --all --check` | ✅ 0 diff |
| `cargo check -p axagent-analysis-engine` | ✅ 通过 |
| `cargo test -p axagent-analysis-engine --lib reco_loop::tests` | ✅ **31 passed / 0 failed**（含中性 0.5 先验断言） |
| `cargo test -p axagent --lib reflection_output_shape_tests` | ✅ **3 passed / 0 failed**（report 包装/缺 verdict/真执行 validator） |
| `cargo test -p axagent --lib backtest_validation::tests` | ✅ **4 passed / 0 failed** |
| `cargo test -p axagent-analysis-engine --lib hit_rate_backtest::tests` | ✅ **33 passed / 0 failed**（含期窗匹配正/负控） |
| `cargo test -p axagent-analysis-engine --lib market_close_store::tests` | ✅ **8 passed / 0 failed**（含 coverage clamp） |
| `cargo test -p axagent-analysis-engine --lib decision_quality::tests` | ✅ **3 passed / 0 failed**（决策形状；函数已放引擎层，避免命令横向互调） |
| `node scripts/check-layer-discipline.mjs` | ❌ `stock_analysis.rs` 直连 DB 71 > 基线 67：来自会话期间新增的 `workflow_param_changes` 参数台账函数（4 处直连），不在本轮股票链改动内；未擅自改基线 |
| `cargo check -p axagent --lib --tests` | ✅ 通过（10m 33s，最终源码） |
| `cargo clippy -p axagent-analysis-engine --all-targets -- -D warnings` | ✅ 通过（26s） |
| `cargo clippy -p axagent --lib --tests --all-features -- -D warnings` | ✅ 通过（8m 10s；顺带修复 `PendingEvent` 缺 `Clone` 的 all-features 编译错） |
| `cargo test -p axagent --lib seed_consistency_tests` | ✅ **50 passed / 0 failed**（版本 179 后） |

## 6. 仍未做（明确边界）

- **没有改生产权重表**：代码会在下一次闭环重算时 reset 到中性 0.5；
  `strategy_weight_history` / `reco_picks` 未写。也就是说，**在 cron 真正跑之前，
  生产仍在用旧的 0.906/0.068 权重**。
- **没有重跑生产回测**：本轮只修复/隔离了存量 `decision_validations`；没有重新拉 K 线
  为 1,145 条提前行补算。mid/long 因此仍是 0 条成熟样本，不能用旧权重当结论。
- **7 条空决策未回填**：2026-07 的历史行；代码已能报 `decisionSchemaGaps`。
- **停牌/迁码清单未清理**：`market_stock_universe` 的 234 个老 BJ 码仍在；已证明
  「已交易票 0 缺失」，清理属数据治理，不影响当前召回。
