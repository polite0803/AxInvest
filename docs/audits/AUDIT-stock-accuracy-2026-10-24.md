# 股票业务「准 与 全」四项修复审计 — 2026-10-24

> 状态：active ｜ 日期：2026-10-24 ｜ 对应版本：v2.9.9


针对三条业务线的目标（智能荐股准确荐股且不遗漏 / 股票分析工作流给出准确决策 /
趋势智选准确挑选趋势股），先做源码事实取证，再修**四**个可确证、直接作用于这三个目标
的缺陷；另有若干项属产品语义决策，只登记不擅改，见文末 §5。

> 取证纪律：子代理给出的结论**逐条回源码核对**，其中两条被证伪或直接推翻，见 §6。

---

## 1. 趋势智选 · 缺数据反而拿到「稀缺溢价」（P0，已修）

**位置**：`src-tauri/src/commands/stock_workflow/rhai_bottleneck.rs:357-367`（修复前）

```rust
let supplier_score = if (0.0..=2.0).contains(&raw_supplier) {
    90.0
} else if raw_supplier <= 5.0 {   // ← -1.0 恒真命中这里
    70.0
...
} else {
    50.0                          // ← 作者为「缺失」写的中性兜底，永远不可达
};
```

`map_num(...).unwrap_or(-1.0)` 在字段缺失时返回哨兵 `-1.0`，而 `-1.0 <= 5.0` **恒真**
⇒ 缺 `global_supplier_count` 的产业链环节拿到 **70 分**（供应商稀缺溢价）。

**为什么直接伤害「准确挑选趋势股」**：不可替代性 `irreplaceability` 里 `supplier_score`
占 0.30 权重。同一函数内 `concentration_score` / `cycle_score` 的缺失都走 `else → 50.0`
（中性），supplier 是同族唯一漏网 ⇒ **「没查到」被当成「供应商只有个位数」**，比真实
查到的「8 家供应商」还高 20 分，白得 +6 分 composite（0.30×20），把一批数据最差的标的
顶到榜单前面。

**修复**：显式把负值（缺失）判在前面，与同函数另外两项的中性口径对齐（50 = 中性）。
高分档（≤2 家 ⇒ 90）保留给**真实稀缺**。

**验证**：新增 3 个单测（同文件 `mod tests`）
- `missing_supplier_count_must_not_earn_scarcity_premium`：缺失 ≡ 已知 8 家；且严格劣于真稀缺；严格优于过剩
- `negative_one_sentinel_equals_absent`：显式 `-1` 与字段缺席同分
- `supplier_score_is_monotonic_by_scarcity`：1/3/8/20 家四档严格单调

---

## 2. 智能荐股 · cron 主生产者缺三类种子 ⇒ 漏过的票天天再漏（P0，已修）

**位置**：`src-tauri/src/init/services.rs`（荐股 cron 分支） / `src-tauri/src/commands/stock_analysis.rs`

**取证的链路事实**：
- 00:00 定时荐股是 `reco_picks` 的**主生产者**（pool-scan、漏检核查、回测吃的是它）；
- 候选种子有四路：user（自选∪本机标的）/ mover（当日涨幅榜）/ reflow（回灌）/ miss（漏检反证样本）；
- 修前 cron 只调 `refresh_reflow_seed_db` 一路，**其余三路只在命令层刷新**（用户手点 ▸「刷新」）。

原因写在 `refresh_reflow_seed_db` 自己的注释里：**cron 拿不到 `tauri::State`**，而另外三路
的签名都吃 `&State<'_, AppState>` —— 也就是说 reflow 那次改造已经把同类缺陷修过一次，
但**没有推广到同族另外三个**。

**为什么这是「不遗漏」的致命断口**：`refresh_miss_seed` 的入参正是漏检核查判定
「池子没看见 / 无留痕」的样本。它在 cron 里不刷新 ⇒ 形成闭环上的豁口：

```
漏检核查点名「某妖股没被看见」 → 只写进 miss seed → 仅供手动入口
→ 00:00 主批次用的还是原来那只池子 → 下一轮同一只票再判一次「漏」
```

**修复（结构上杜绝再次分裂）**：
- 抽出 `refresh_user_seed_db` / `refresh_mover_seed_db` / `refresh_miss_seed_db`（与既有的
  `refresh_reflow_seed_db` 同形）；
- 新增唯一注入点 `refresh_all_reco_seeds_db(db, generation, client) -> RecoSeedCounts`；
- **两条入口（UI 四档 + 00:00 cron）都只调这一个函数** ⇒ 漏一路则两条都漏，
  不可能再退化成「UI 刷了、cron 没刷」；
- 删除四个已无调用方的 `&State` 包装函数（留着会在 `-D warnings` 下报 dead_code）。

**顺带修掉的陈旧残留**：`refresh_mover_seed_db` 在 `n == 0` 时**早退且不调 `set_mover_seed`**
⇒ 上一轮注入的标的无限期留在全局种子里；快照停采 / 板块盲扫全失败时荐股仍在扫一批
已被验证不再强势的旧票。改为与另外三路一致的覆盖语义（空集合也写）。

---

## 3. 股票分析 · 缺财务数据被判「低风险」⇒ 决策系统性偏乐观（P0，已修）

**位置**：`src-tauri/src/commands/portfolio-mgr.rhai:554-597`（修复前）

```rhai
let roe    = if present(risk_roe)            { risk_roe }            else { 100.0 }; // 注释：无数据时不触发 ROE 惩罚
let growth = if present(risk_revenue_growth) { risk_revenue_growth } else { 100.0 };
let debt   = if present(risk_debt_ratio)     { risk_debt_ratio }     else { 0.0 };
let dd     = if present(risk_drawdown)       { risk_drawdown }       else { 0.0 };
...
if vol < RISK_VOL_LOW && sharpe > RISK_SHARPE_LOW && dd < RISK_DD_LOW
   && roe > RISK_ROE_LOW && debt < RISK_DEBT_LOW && growth > RISK_GROWTH_LOW { is_low = true; }
```

四个哨兵的原意是「缺数据**不触发惩罚**」（服务于 588/592/593 三处惩罚类判据，语义正确）。
但 597 行的**低风险**判据是相反方向：它拿同一批哨兵去**证明**「基本面稳健」。于是缺数据时
四条同时自动成立（`roe(100) > 8` ✓、`growth(100) > 3` ✓、`debt(0) < 55` ✓、`dd(0) < 30` ✓），
只要波动率与夏普也在阈值内 ⇒ 直接判**低风险**。

**后果（为什么直接伤害「准确决策」）**：对同一批缺数据标的，惩罚判据被哨兵静音、
正面判据被哨兵点亮 ⇒ **没有任何财务数据的票反被判成最安全的一类** ⇒ 仓位上限
（`POS_CAP_*`）放宽、`f4` 风险惩罚减轻、最终 action 偏乐观。

**与文件自身的既定语义冲突**：同文件 617-619 行明确规定——无法分类时 `风险=中风险、
risk_source="无数据"`。597 行的行为等于在这一处悄悄把「无数据」判成了最高评价。

**修复**：惩罚侧哨兵**一字不改**（保留「缺数据不触发惩罚」）；只给低风险这个正面判定
加一道入参存在性门槛 `low_risk_inputs_present`（sharpe / dd / roe / debt / growth 五个
全部实测存在才允许判低风险），缺任一 ⇒ 回落 `中风险`，与 617-619 的既定兜底一致。

**版本**：`TEMPLATE_VERSION` **177 → 178**（嵌进节点 `code` 的 rhai 改动不升版 = 不落库，
本项目已多次踩过；判定不可查询的生产库见 §5 待办）。

**语法风险规避**：全仓 `.rhai` 文件**没有任何「多行 `&&` 续行」的先例**，而既有风格是
单行布尔（见 611-612 行）。本项目 `seed_consistency_tests.rs:2046` 自己也写了教训：
「改脚本的人（含 AI）极易在长注释/**多行 `if`** 上写坏却毫无反馈」。故本次刻意不引入
多行布尔——拆成 `quant_inputs_ok` / `fin_inputs_ok` 两个单行中间变量，`is_low` 判据维持
单行写法。

**验证**：生产侧自带的语法门 `portfolio_mgr_rhai_compiles`
（`src/commands/stock_analysis_setup/seed_consistency_tests.rs:2067`）会用
**生产那个引擎工厂**（`rhai_registry::build_stock_rhai_engine` + `PORTFOLIO` 档位）编译
本文件 ⇒ 本次改动由它兜底；该测试是「改 rhai 而不自知」的唯一早期告警。

---

## 4. 智能荐股 · 空批次不留痕 ⇒ 真漏股无法归因（P1，已修）

**位置**：`src-tauri/src/init/services.rs`（修复前 `if !picks.is_empty() { ... }` 整段）

`persist_reco_picks` 与 **`persist_reco_scan_audit`** 被同一个 `if !picks.is_empty()` 包住
⇒ 一轮「扫过、但因硬门/阈值被整批淘汰」的批次在 `reco_scan_audit` 里查不到任何痕迹。

**为什么这掩盖真漏股**：`mover_recall::attribute` 拿不到「本轮确实扫过、票是在哪一档被淘汰」
的证据，只能把票打成 `NotInPool` / `Unexplained`。这两种归因的修复动作**截然相反**：

| 归因 | 含义 | 正确修法 |
|---|---|---|
| `NotInPool` | 票压根没进候选池 | 扩池 / 补种子来源（即 §2 那类） |
| `L3ScoredOut` | 票在池里，是在评分层被主动淘汰 | 改评分判据 / 阈值 |

归因错 ⇒ 改错地方，`L3ScoredOut` 与 `NotInPool` 混淆时尤其致命 —— 前者会让人一直去扩池，
而真相是评分算错了。典型场景：`regime_mult < 1.0` 的追涨腿硬门会把 Trend 整档清空，
这类批次以前就是「跑完了但一片空白」。

**修复**：把 `persist_reco_scan_audit` 循环移出空判，并在日志里带上本批出票数
（`pick==0` 但 `audit>0` 即为「扫过但全被淘汰」的明确信号）。

---

## 5. 已登记、**未擅改**（需产品语义裁决 / 待取证据）

| # | 事项 | 位置 | 为什么不动手 |
|---|---|---|---|
| R-1 | value 策略 PE 门在 long/mid 档砍 PE>35/40 ⇒ 高 PE 成长股整票出局，且**被拦时零留痕** | `recommender/strategies/value.rs:141-166` | 是否放宽属产品取向（用户曾提「PE 过滤器误杀成长股」）；**补留痕**这部分是可直接做的，本次未做 |
| R-2 | `concentration/cycle` 缺失给 50（中性），但**高于**真实差值的 40 ⇒ 仍是「缺数据优于坏数据」 | `rhai_bottleneck.rs:289-307` | 中性值取多少是政策问题；与 §1 的「不可达分支」性质不同，需裁决 |
| R-3 | 趋势智选**全程无 K 线技术判据**（MA/MACD/量比只在智能荐股链） | `recommender/strategies/trend.rs` vs `strategy-scorer.rhai` | 是否引入属产品定义：「趋势股」该按技术面还是产业链瓶颈打分 |
| R-4 | LLM 输出无强制 schema：`required: None`、`action` 无 enum、`SchemaValidator` 是死代码 | `seed_stock_analysis.rs:7585-7597` / `rt-workflow/.../schema_validator.rs` | 启用强制校验会改变失败行为（拒绝 vs 兜底），需裁决 |
| R-5 | 快链 `j-strategy-type` 一个全局标签喂给 5 个 scorer ⇒ 全部走非 bottleneck 分支读不存在的字段 | `seed_serenity_fast.rs:551` | 快链非自动调度链路，影响面待确认 |
| R-6 | `TEMPLATE_VERSION=178` 能否真的落库 | `seed_stock_analysis.rs:1600` | 版本门是 `>=`；**生产库现值未能读取**（项目内无本地 DB 文件）。落库前必查 `SELECT version FROM workflow_templates WHERE id='stock-analysis'`，若现值 ≥178 须再抬高常量 |

## 6. 被推翻的两条二手结论（记录以免复发）

| 被报告的说法 | 核验结果 |
|---|---|
| 「live 模式下 `degraded_styles` 恒为空 ⇒ Trend 硬门删桶无留痕」 | **错**。`recommender/mod.rs:1448-1467` 在 `regime_gated_trend > 0` 时无条件 push Trend，且从 disabled 列表摘除避免文案覆盖 —— 这条链路是通的 |
| 「cron 不刷新任何种子」 | **不完整**。reflow 一路已在 D-3 修好，缺的是 user/mover/miss 三路（本次 §2 修） |

## 7. 验证状态（**当轮**未经编译验证 —— 用户指示「不用编译验证」）

> ⚠ 本节是**当时**的状态。第二轮（§8）已完成编译与定向单测，最终读数见 §8.7；
> §7 保留原文用于对照，不再代表当前状态。

| 项 | 状态 |
|---|---|
| `cargo fmt --check` | ✅ **全仓 0 diff**（我方改动区域已单独格式化；此前残留的 26 处 diff 均落在本次未触碰的行，随并发会话整理后全仓归零） |
| `cargo check` / `cargo test` | ❌ **未跑** —— 按用户指示中止。排队等锁 26 次 / 15 分钟均被**其它会话的 cargo 持锁**拦截（`target/debug/.cargo-build-lock` EPERM）后由用户叫停 |
| 新增单测 3 个（`rhai_bottleneck`） | ❌ **未执行**，语法/语义待跑时确认 |
| `portfolio-mgr.rhai` 语法 | ⚠ **未由语法门确认**；已刻意规避全仓无先例的「多行 `&&` 续行」以降低风险，但**未经 `portfolio_mgr_rhai_compiles` 实测** |

⇒ **本轮 4 项改动属「已落盘未编译」状态**。下次有 cargo 空闲时段请先跑：
`cargo check -p axagent --lib --tests`
`cargo test -p axagent --lib rhai_bottleneck`
`cargo test -p axagent --lib portfolio_mgr_rhai_compiles`

---

## 8. 第二轮（2026-10-24）：趋势智选的「策略类型错配」与评分口径

§7 的 4 项是「已落盘未编译」。本轮继续回源码取证，另查并修复 **6 个直接作用于三条
业务线**的缺陷。与 §1–§4 不同，本轮的 6 项都能用**纯函数/脚本真执行**锁住，不依赖
外部行情或 LLM。

### 8.1 原链 5 个评分器拿不到各自的 `strategy_type`（P0，已修）

**位置**：`src-tauri/src/commands/stock_analysis_setup/seed_serenity.rs` +
`src-tauri/crates/rt-workflow/src/work_engine/executors/mod.rs`

原链把 `a-trend-scanner.content.trends[{i}].strategy_type` 写进 5 个拆解 Agent 与 5 个
评分 CodeNode 的 `input_mapping`。但 `resolve_var_path` 只做 `current.get(part)`，
`get("trends[0]")` 对数组**恒 None**（同一文件 2026-10-24 前无下标支持）。后果：

| 环节 | 实际拿到的 `trend_strategy` | 后果 |
|---|---|---|
| 5 个 `a-chain-trend{i}` | `unit` | 拆解 Agent 无法按政策/业绩/资金/事件/技术分支写 schema，倾向按瓶颈输出 |
| 5 个 `c-scorer-trend{i}` | `unit` ⇒ 脚本默认 `bottleneck` | 非瓶颈趋势按 `chain_nodes` 评分 ⇒ `no_data`，趋势智选只剩瓶颈一条腿 |

此外 `content` 还有四种历史包装（裸 `trends` / ```tool_json``` / `{"report":...}` /
`{"name":...,"arguments":...}`），单靠路径无法统一。

**修复（双保险）**：

1. `resolve_var_path` 新增 `name[idx]` / `[idx]` 下标支持（精确键优先；只认非负十进制；
   越界/非法形态 fail-closed）。带 3 个单测，含 JSON 字符串穿透、越界、非法形态与
   「精确键优先」负控。
2. 原链新增确定性 `c-trend-split` 节点（复用 `trend-split.rhai`），把四种包装统一成
   `trend1..trend5` 恒存在槽位；5 个拆解 Agent 与 5 个评分器改取
   `c-trend-split.result.trend{i}.strategy_type`。`serenity-screening` 模板版本 61 → 62。

### 8.2 快链用一个全局分类喂 5 个策略槽（P0，已修）

**位置**：`src-tauri/src/commands/stock_analysis_setup/seed_serenity_fast.rs`

5 个 `c-scorer-*` 的 `trend_strategy` 全部映射到 `j-strategy-type.category`（一个全局
LLM 分类）。合并腿的 `trends[]` 可能同时含六类趋势，全局分类只会把其中一类套到全部
槽位 ⇒ 其余槽位的 `chain_analysis` 与该策略入参 schema 不匹配，返回 `no_data`；
候选组装端拿不到确定评分，只能退回 LLM 自报的 `confidence`。改为
`c-trend-split.result.trend{i}.strategy_type`，快链模板版本 1 → 2。

### 8.3 候选组装端：首见分 / LLM 自报分冒充分数（P0，已修）

**位置**：`src-tauri/src/commands/candidate-assembler.rhai`

三个独立缺陷：

| # | 旧行为 | 后果 |
|---|---|---|
| a | `score_by_code` 首见即定 | 同一票被多个趋势/策略各算一次时，**先出现的弱分**压住后出现的强分 |
| b | 没有确定评分时直接用 `tconf`（趋势 confidence，可到 90） | LLM 自报分与确定性评分不可比，未算过的票凭自报分排到算过且高分的前面 |
| c | 不输出 `strategy_type` | 下游 `data-verifier` 只能按瓶颈分支处理所有候选 |

**修复**：索引值改为 `{score, strategy_type, reliability}` 并按分数取 **max**；无确定性
评分时 **封顶 54**（落在 `READINESS_BANDS` 的 `weak_signal` 35–54，不丢弃、不冒充潜在
信号）；输出 `strategy_type` 与 `data_reliability`。另加可靠度折价：`insufficient` −10、
`llm_estimated` −5（不改子分的中性取值政策，只在排序上体现不确定性代价）。

### 8.4 `data-verifier` 丢弃非瓶颈候选的全部字段（P0，已修）

**位置**：`src-tauri/src/commands/data-verifier.rhai`

非 bottleneck 分支原先只输出 `stock_code` / `stock_name` / `data_verification` 三个键，
政策/业绩/资金/事件/技术候选的 `serenity_score`、`confidence`、`catalysts`、
`exit_signals` 等在这一步**全部丢失**。且分支只看首条候选的 `strategy_type` ——
快速链一批可能混合多种策略，首条非瓶颈会让整批跳过财务校验（反之则误校验）。

**修复**：分支下沉到逐候选；非瓶颈分支透传全部原始字段；bottleneck 输出补
`strategy_type` / `data_reliability`。

### 8.5 政策/业绩/资金/技术评分的常数与错向（P1，已修）

**位置**：`src-tauri/src/commands/strategy-scorer.rhai`

| 策略 | 旧行为 | 修复 |
|---|---|---|
| policy | `composite = policy_impact*0.40 + 70*0.35`，上限仅 75，同策略所有票同分；注释写「龙头溢价」但未实现 | 行业动量取自 `industry_ranking`（35%）+ 个股理由硬证据（25%）+ 政策力度（40%），龙头命中另 +8；上限 100 |
| earnings | 缺 `expected_beat_pct` 按 0% 超预期罚 25 分；理由里出现「数据/调研/动销」即给 70 分（与方向无关） | 缺预期差给中性 45；过往准确率只认显式 `track_record_score`，否则 50 |
| capital | 缺 `net_inflow` 按 0 流入罚 25 分；缺 `days` 按 1 天给 40 | 缺失给中性 45（缺席不是流出） |
| technical | `sector_aligned: false` 与「字段缺失」同为 50 | false 是负证据给 30，缺失才 50 |

另把 `ind_map` 从 `score_bottleneck` 闭包内提到顶层，六种策略共享同一份行业动量口径；
`chain_analysis` 在公共层统一解包 `{"name":...,"arguments":{...}}`，避免非瓶颈策略
因包装形态直接 `no_data`。

### 8.6 趋势智选落库胜率没吃确定性评分（P0，已修）

**位置**：`src-tauri/src/commands/stock_workflow/serenity.rs`

落库前算「候选评分 → 该档上涨胜率」时读的是 `c["confidence"]`（LLM 自报置信度），
而不是 `c["serenity_score"]`（确定性评分链产物）。这意味着 8.1–8.5 修好的评分
**全部白算**，最终胜率仍由 LLM 自报值决定。

**修复**：抽 `candidate_score_input` 单一优先级实现 —— `serenity_score` 优先，缺失才
回落 `confidence`，再缺省中性 50，统一 clamp 0..100。带单测（含非数值 `serenity_score`
不得冒充 0 的负控）。

### 8.7 R-1 的留痕部分：value 估值门不再静默（行为未改）

**位置**：`src-tauri/crates/analysis-engine/src/recommender/strategies/value.rs`

R-1 登记的是「高 PE 成长股被 value 策略整票砍掉、且被拦时零留痕」。是否放宽属产品
取向（本轮**不改门**），但「被拦时零留痕」是可以直接补的：四个档位的 PE/PB 门在
`return None` 前写 `[value] {code} 档位 {:?} 被 PE/PB 门拦下: 值 > 上限` 的 debug 日志。
这样下一次「为什么某票没进 value 桶」可以从运行日志回答，而不是只能读代码。

### 8.8 验证状态

| 项 | 状态 |
|---|---|
| `cargo fmt --all --check` | ✅ 全仓 0 diff |
| `cargo check -p axagent --lib --tests` | ✅ **通过**（首次 9m 16s；`check-layer-discipline` 修正后的最终 `--lib` 复跑 8m 48s） |
| `rhai_registry::tests` 全量 | ✅ **22 passed / 0 failed**（含新增 5 门）；另 `rhai_bottleneck` **3 passed** |
| `seed_serenity` / 快链版本三态 | ✅ **5 passed**（含 `seed_serenity_tools_all_resolvable`、原链/快链 `version_gate_three_states`） |
| `seed_consistency_tests` 全量 | ✅ **50 passed / 0 failed**（含图结构、逐档分支、`portfolio-mgr` 语法/风险门、模板默认值） |
| `portfolio_mgr_rhai_compiles` | ✅ 通过（`portfolio-mgr.rhai` 语法由生产引擎工厂确认） |
| `serenity_version_gate` | ✅ 3 passed（模板版本 61→62 的三态门） |
| `serenity::candidate_score_input` | ✅ 通过（含非数值 `serenity_score` 负控） |
| `low_risk_requires_observed_fundamentals` | ✅ 通过（正控=低风险、负控=中风险） |
| `resolve_var_path` 下标单测 | ✅ **32 passed / 0 failed**（`axagent-rt-workflow --lib resolve_var_path`，含新增 3 门） |
| `cargo clippy -p axagent --lib --tests --all-features -- -D warnings` | ✅ **通过**（10m 49s，含 `analysis-engine`/`rt-workflow`/`axagent` 全依赖链） |
| `cargo clippy -p axagent-rt-workflow --all-targets --all-features -- -D warnings` | ✅ 通过（2m 31s） |
| `cargo clippy -p axagent-analysis-engine --all-targets -- -D warnings` | ✅ 通过（1m 54s，覆盖 8.7 的 value 日志改动） |
| `node scripts/check-layer-discipline.mjs` | ✅ 通过（直连 DB 68→67；`background_tasks` 按「任务台账基础设施」登记进 `SHARED_INFRA_MODULES`，不是刷基线） |
| `node scripts/check-reco-generation.mjs` | ✅ 通过（R1–R10 全绿） |
| `node scripts/check-reco-horizon-parity.mjs` | ✅ 通过（a–q 全绿） |
| `node scripts/check-ontology-consistency.mjs` | ✅ 通过（D1–D3 为已登记待裁决，不拦） |

**构建环境说明**：受限工作区的 `target/debug/.cargo-build-lock` 无法打开（EPERM），
本轮用 `CARGO_TARGET_DIR=<临时目录>` 完成编译；`src-tauri/build.rs` 的「内容未变不重写」
也消除了 `src/register_commands.rs` 的写权限失败。两者都不改业务语义。

### 8.9 仍然无法在仓内证明的（如实声明，不算已完成）

| # | 事项 | 为什么本轮证明不了 | 下一步 |
|---|---|---|---|
| X-1 | **实际荐股/决策/趋势命中率** | 需要生产库的 `reco_picks` / `stock_analyses` / 行情回放；本机无 DB 文件、无 vendor 凭据 | 在装有生产库的机器上跑 `backtest_reco_strategies` / `hit_rate_backtest` / `serenity_hit_rate_backtest`，按代际分层读 |
| X-2 | **候选宇宙覆盖率** | `market_stock_universe` 现网约 681 只，扩张受东财 `clist` 限流；代码只能如实登记覆盖率，不能变出全市场 | 观察 `market_universe_stats.coverage_pct`，按 tick 累积 |
| X-3 | **R-4 LLM 输出强制 schema** | 启用会改变失败行为（拒绝 vs 兜底），属产品裁决；当前靠脚本容错 | 先在生产回放上量化「强制校验会拒掉多少条有效决策」，再决定 |
| X-4 | **R-6 生产库模板版本** | 仓内读不到生产 DB 的 `workflow_templates.version` | 落库前查 `stock-analysis` / `serenity-screening` / `serenity-screening-fast` 的现值 |

> X-1/X-2 是「准确率」的最终裁判，本轮修的是**让这些读数可信**的前置条件
> （策略类型不再错配、确定性评分真正进入胜率、缺数据不再被当成利好）。
> 不把「编译/单测绿」冒充「命中率达标」——那是两种不同的证据。

