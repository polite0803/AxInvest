# PLAN — 修复「工具可信度」口径与数据质量熔断的附带缺陷

## Context（为什么改）

决策卡上长期出现「决策可信度受限 / 数据质量已连续 N 轮未达 A 级并触发熔断」提示。实测（生产库 `axinvest`）：
`data_quality_observations` 全表只有 4 行、**全部 B 级**，且 `data-quality.rhai` 自身注释自引「全库 46 条最优 composite 79.3」。

- 综合分公式 `报告质量×0.35 + 工具可信度×0.35 + 因子完整度×0.30`，A 级线 `>= 85`（`src-tauri/src/commands/data-quality.rhai:1405-1415`）；
- 实测两轮：中材科技 73.7（报告 72.4 / 工具 52.4 / 因子 100）、许继电气 72.7（76.2 / 45.9 / 100）；
- **A 线 85 高于实测全局上限 79.3 ⇒ 熔断（连续 3 轮未达 A）实际 100% 常开**，提示条退化为恒亮噪音。

工具可信度偏低的机制：`tool_credibility(avg_conf, gap_count) = 自评置信度均值 − 缺口阶梯惩罚`，再减「字段校验每项 3 分（上限 15）」（`data-quality.rhai:645-654`、`1018`、`1185-1189`）。而 `data-quality.rhai:1480` 已自陈：自评低但报告无失败标记时属「**非数据缺口** —— confidence 语义与系统按『数据充分度』消费的口径不一致」。即：**函数自称衡量「上游工具可信度」，实际消费的是分析师对自己结论的不确定度**，量程被压在 ~60，A 级结构性不可达。

目标产出：① 让工具可信度口径与它的语义一致；② 让熔断重新具备信号价值（而不是恒开）；③ 清掉孤儿观测行对连续轮数的污染。

---

## 实证记录（改动前后对比的基线）

| observed_at | grade | 分析 | 工具可信度构成 |
|---|---|---|---|
| 10-09 06:54:05 | B | `stock_analyses` 记录**已删除**（孤儿行） | — |
| 10-09 06:55:31 | B | **孤儿行** | — |
| 10-09 09:33:07 | B | 许继电气(000400) | 自评 61.9 − 缺口 1(10.0) − 校验 2 项(6.0) = 45.9 |
| 10-09 15:34:32 | B | 中材科技(002080) | 自评 61.4 − 校验 3 项(9.0) = 52.4 |

中材科技该轮 `decision_json`：`dqiFuseState="fused"`、`dqiStreak=3`、`confidenceQualityCap=60.0`、`confidence=60.0`。

---

## 步骤 1（你点名的第一件事）工具可信度口径语义修复

**1.1 抽单一分类函数，避免第二份判据。** 在 `data-quality.rhai` 新增
`fn conf_class(c, untrusted, ph_n, asof_ex, raw_ph_n, word_ex)`，返回
`"untrusted" | "missing" | "asof_exempt" | "asof_low" | "wording_exempt" | "tool_gap" | "self_uncertain" | "normal"`；
`diag_for`（`data-quality.rhai:1434-1481`）由 class → 文案，**不再自己串 else-if 链**。

> ⚠ 关键判据（本轮调研纠正）：`ph_e 为空 且 自评 < 50` 是 `diag_for` 末支的**超集** —— 它还会吞掉 (a) as-of 设计性降级（`asof_exempted_hit`，`data-quality.rhai:1456-1457`）与 (b) R2 措辞性缺席豁免（`word_ex`，`data-quality.rhai:1458-1472`）。这三类的共同点是**都不是工具不可靠**（工具按设计返回空 / `count_failed_tool_calls(tool_calls) == 0`，见 `data-quality.rhai:822-841`），故都应剔出工具可信度。

**1.2 工具可信度的分母只计「有证据支撑」的自评。** `valid_count` / `total_conf`（`data-quality.rhai:986-1009`）改为只累加 `conf_class ∈ {"tool_gap", "normal"}` 的节点；剔除 `{"asof_exempt", "asof_low", "wording_exempt", "self_uncertain"}` 四类（`untrusted` / `missing` 仍按原样进 `gap_count`）。

- **如实标注**：剔除低于均值的项必然抬高 `avg_conf`，这是**向上的口径偏移**，不是「中性」。必须在注释与输出里说明，不得写成「不抬不压」。
- **可审计**：把被剔节点与被剔原因并入现有展示通道（`low_confidence_analysts` 为「分析师视角」清单，**保持不动**；新增一句 warnings 说明「N 个自评低但无取数证据的节点未计入工具可信度」）。
- 计数口径：存在性一律用 `placeholder_hits`（`*_ph_e.len() > 0`）；`placeholder_occurrences` 只作面板展示，**不得**引入 `occurrences >= N` 这第二条判据轴。

**1.3 字段校验扣分移出工具可信度。** 删掉 `data-quality.rhai:1185-1189` 的 `-3/项` 扣分，保留 `1823-1828` 的告警。

- 理由：`bull_score + bear_score != 100`、`verdict` 与分值矛盾属**分析师产出形态**缺陷（与 `verdict_field_gaps` 同类，`data-quality.rhai:1376-1389` 已定「只告警不扣分」），扣在「上游工具可信度」上是误归因，且同类两种处置自相矛盾。
- **不得**把它挪进 `report_quality`（换槽位再计一次 = 二次计分）。
- 方向上这仍是**数值放宽**（tc 至多 +15、composite 至多 +5.25），属修口径；且**不足以**让 A 可达（见步骤 2），不得为凑 A 线而打包。

**1.4 预期效果（用于验收）**：中材科技 73.7 → 约 77.5、许继电气 72.7 → 约 75.8，**仍为 B** ⇒ 提示条不会因此消失。

---

## 步骤 2（需你拍板）让熔断重新有意义

只做步骤 1，熔断仍恒开。二选一（我推荐 B）：

**B（推荐）—— 工具可信度改用「工具调用证据」重建量程。**
脚本已有 `tool_calls` 输入与 `count_failed_tool_calls`（`data-quality.rhai:822-833`），改由「失败工具调用占比 + 未获数据维度数 + 占位符节点数」派生，自评退出该口径。这也正是该函数自陈的升级方向（`data-quality.rhai:618`：「P3 升级方向：接入 t-scoring/t-valuation/t-risk 的 credibility 字段后替换此实现」）。
- 属**换量**（修标尺），不动 A 线 85；预期证据干净的轮次 A 重新可达。
- 必须做**历史回放的前后对比**（你要求的「记录前后对比数据」），并防止反向失效（A 变几乎必达 ⇒ 熔断永不触发）。
- 严禁复用 `factor_completeness`（已占 0.30 权重）作为新信号 = 二次计分。

**C —— 熔断判据与 grade 解耦。** `abnormal` 改由脚本新输出的「上游数据信号」派生（`gap_count` / `untrusted` / `upstream_data_gaps`），而非 `grade != "A"`。
- 代价：`abnormal` 的**行语义变更**，须在 `src-tauri/crates/dao/src/repo/data_quality_fuse.rs` 模块头声明规则版本；且工具可信度的语义问题仍然存在。

**都不做** ⇒ 熔断继续恒开，提示条等同永久噪音（`DecisionTrustNotice.tsx` 注释里警告的「恒亮」形态）。

---

## 步骤 3（你点名的第二件事）孤儿观测行

在 `src-tauri/crates/dao/src/repo/data_quality_fuse.rs` 加一个存在性过滤 helper，供 `load_recent_streak`（`:122`）与 `load_fused_intervals`（`:217`）**共用**：`scope = global` 后再按 `analysis_id ∈ stock_analyses` 过滤。

- 选它而不是「删除命令里级联删除」：① **存量 2 行即刻失效**，免迁移；② 覆盖所有删除路径（含未来新增），级联只覆盖已知两条命令（`src-tauri/src/commands/stock_analysis.rs:1401-1411`、`1414-1439`）；③ 只改「行是否参与」，不重算 `abnormal` 归属，符合模块头纪律。
- 必须在模块头注明取舍依据：观测量在分析跑完才落库，因此「未落库的轮」本就不存在，剔除孤儿行不会漏掉真实轮次。

**不做**：`dqi_streak` 在运行开始时读取（不含本轮）不是缺陷（`src-tauri/src/commands/stock_workflow/hooks.rs:452-490`），只在注释里澄清「界面 3 / 库中 4」的差异来源。

---

## 步骤 4 注释订正

`data-quality.rhai:628-630` 断言「移除 good_count 后 A 级重新可达」，与同段自引的「全库 46 条最优 79.3」自相矛盾 —— 改为如实描述「A 线 85 高于实测上限，属结构性不可达，待量程修复后重评」。

---

## 验证

新增门禁（测试模块**追加到文件末尾**，避免 `clippy::items_after_test_module`）：

- `src-tauri/crates/rt-workflow/tests/data_quality_placeholder_gate.rs`（复用既有夹具 `run_quality` / `run_quality_with_tool_calls` / `run_quality_asof`，`:206-283`）：
  - `low_conf_without_evidence_does_not_move_tc`（无标记 c=40 被剔，归因仍落「非数据缺口」）
  - `low_conf_with_marker_still_moves_tc`（**负控**：有失败标记仍入分母）
  - `asof_and_word_exempt_low_conf_are_excluded_from_tc`
  - `validation_penalty_no_longer_scored`（`bull+bear=90` 时 tc 不再被扣，warnings 仍列条目）
- `src-tauri/crates/dao/src/repo/data_quality_fuse.rs` 的 `mod tests`：
  - `orphan_observation_is_ignored_by_streak_and_timeline`（直插一行 `analysis_id` 不在 `stock_analyses` 的行 + 一行正常，断言两个读口都只算后者）

逐阶段定点（AGENTS.md：阶段内不跑全量）：

```
__TAURI_WORKSPACE__=true cargo test -p axagent-rt-workflow --test data_quality_placeholder_gate
__TAURI_WORKSPACE__=true cargo test -p axagent-dao data_quality_fuse
cargo clippy -p axagent-rt-workflow --all-targets -- -D warnings
cargo clippy -p axagent-dao --all-targets -- -D warnings
cargo check -p axagent            # rhai 经 include_str! 嵌入，改脚本后必查
```

全部阶段完成后按 CI 口径跑一次：

```
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
__TAURI_WORKSPACE__=true cargo test --workspace
```

端到端复核：用生产库同样两条分析（中材科技 `239b0b8f…`、许继电气 `b3b54401…`）的快照重跑 `data-quality` 节点，对比步骤 1 记录的前后数值；再查 `data_quality_observations` 的连续轮数是否已排除两条孤儿行。

---

## 执行结果（2026-10-09 收尾，实证记录）

### 真实 Rhai 回放（步骤 2「B」的前后对比）

方法：`stock_analyses.blackboard_snapshot` 的 `_raw.data-quality.input_params`（生产真实输入）原样注入**真实 Rhai 引擎**（复用 `data_quality_placeholder_gate.rs` 的 `build_engine`）重跑 v150 脚本；`rq` / `fc` 取**记录值不变**，只换 `tc` —— 即隔离本次换量的净效果。回放 54 轮（当前脚本时代），0 失败。

- grade 分布：旧 **A 0 / B 36 / C 17 / D 1** → 新 **A 18 / B 21 / C 13 / D 2**。
  A ≈ 1/3 ⇒ **可达但不近乎必达**（熔断仍有判别力），C/D 仍被区分。
- 反向失效核对：23 轮无任何 `tool_calls` 记录（证据源③ 只贡献 0），但「无记录**且** gap/tgap 全 0」的轮次为 **0** ⇒ 不存在只靠③才判得出坏的盲区。
- 点名两条 before/after（同一次回放）：

| 分析 | 旧 tc | 旧综合分 | 新 tc | 新综合分 | 证据 |
|---|---|---|---|---|---|
| 中材科技 2026-10-09 | 52.4 | 73.7 (B) | 100.0 | 90.3 (A) | gap 0 / tgap 0；34 次调用 0 失败 |
| 许继电气 2026-10-09 | 45.9 | 72.7 (B) | 90.0 | 88.2 (A) | gap 1 / tgap 0；31 次调用 0 失败 |

### 孤儿观测行（步骤 3）

`data_quality_observations`（`scope=global`，`observed_at` 倒序、按 `analysis_id` 去重）实测：未过滤连续非 A = **5**，过滤两条孤儿行（`analysis_id` 不在 `stock_analyses`）后 = **3** —— 与界面 `dqi_streak=3` 一致。

### 门禁

- gate `data_quality_placeholder_gate`：**40/40**（含 `clean_round_can_reach_grade_a`、`negative_controls_still_penalize_tool_credibility` 等新口径断言）。
- `cargo fmt --all --check`：EXIT=0。
- 定点 `cargo check -p axagent`、`cargo clippy -p axagent-rt-workflow --all-targets -- -D warnings`：绿。
- workspace 全量 clippy / test：按 CI 口径收尾执行（见提交记录）。

---

## 补充一（2026-10-09 续）数据质量诊断卡片的 5 处展示 / 归因缺陷修复

用户上传两张诊断卡片截图，逐条归因后「一起修」：

| # | 缺陷（截图实证） | 修法 |
|---|---|---|
| ① | `summary` 写出全精度浮点「平均置信度 71.16666666666667」，同卡头部 `avg_confidence` 却显示 71.2 | 新增展示别名 `avg_conf_d = (avg_conf*10).round()/10`，`summary` / warnings / 输出字段统一用它 |
| ② | A/B/C 分支摘要**只报**「8/8 高置信度」，缺失节点（`gap_count>0`）完全不可见 ⇒ 用户无从解释「为什么不是 A」 | A/B/C 分支补 `缺失：… / 低置信：… / 报告含失败标记：…` 与 F/D 分支同构 |
| ③ | 「字段校验问题：2 个」紧挨等级结论，被读成降级原因（自 v149 起该项已不扣分） | 摘要就地标注「（仅提示，不计分）」 |
| ④ | 「缺失因子：催化剂等级」把**中性取值**（分析师判定「无」）与**真缺字段/取数故障**混成一句 | 三态：`!present \|\| ""` → 纯名；`== "无"` → 附「分析师判定『无』，中性取值不计入——非取数故障」。分母口径（与 Rust `compute_factor_completeness` 对齐）**不变** |
| ⑤ | 缺口原因写「节点未输出 VERDICT **或** confidence 字段缺失（可能原因：…）」—— 两种可判定情形混成一句「或」 | `diag_for` 加第 10 参 `v_present`，按 presence 给**确定归因**（「无任何输出」vs「有输出但缺 confidence」）；11 个调用点同步补齐实参 |

**验证（全部绿）**：`data_quality_placeholder_gate` **42/42**（新增 `missing_gap_reason_is_definite_about_presence`、`neutral_catalyst_factor_self_describes_while_true_absence_stays_plain`）；
`seed_consistency_tests::data_quality_rhai_diag_for_arity_matches_all_call_sites` **1/1**（元数守护，自动覆盖新增第 10 参）；
`cargo fmt --all --check`、`cargo clippy -p axagent-rt-workflow --all-targets -- -D warnings` 均通过。

---

## 补充二（2026-10-10）「数据质量封顶 B」根因定位与熔断判据收窄

### 生产库实证（`axinvest`，全部 6 行观测均为 B / `abnormal=1`）

| 分析 | 模板版本 | rq | tc | fc | 综合 | grade | gap_count | 缺失维度 |
|---|---|---|---|---|---|---|---|---|
| 紫金矿业 601899 | 149 | 74.8 | **80.0** | 88.9 | 80.8 | B | 2 | 行业、催化剂 |
| 中材科技 002080 | 148 | 63.8 | 54.3 | 100.0 | 71.3 | B | 0 | — |
| 中材科技 002080 | 148 | 72.4 | 52.4 | 100.0 | 73.7 | B | 0 | — |
| 许继电气 000400 | 145 | 76.2 | 45.9 | 100.0 | 72.7 | B | 1 | 价值评估 |

（表中 `tc=80.0` 只可能来自 v150 新口径 `100 − gap 阶梯`；其余三行 `tc` 是该构建期的旧口径产物。）

### 结论一：A 已由 v150 恢复可达，但核心拖累是「任一维度缺席」

`composite = 0.35·rq + 0.35·tc + 0.30·fc`，在 `rq≈74.8 / fc=88.9` 时 **A 需 `tc ≥ 91.9`**：

- `gap_count=0` ⇒ `tc=100` ⇒ **87.85（A）**；
- `gap_count=1` ⇒ `tc=90` ⇒ 84.35（B，**差 0.65 分**）；
- `gap_count=2` ⇒ `tc=80` ⇒ 80.85（B）。

⇒ **任何 ≥1 个维度缺席即基本锁死 B**。同轮 `fc` 恒 88.9%（分母 9 恒缺 1，即「催化剂等级」）另白丢 3.33 分。

### 结论二：缺席的两维不是「数据源不好」，而是**产出形态故障**

从 `_raw` 实证（紫金矿业 `4419bd01…`）：

- **行业分析师** `a-sector--long`：`content` 仅为 3 行工具调用请求文本（`{"function":"get_stock_concept_blocks",…}`），`tool_calls_made=[]`、`usage.output_tokens=63` ⇒ 该节点 agent 循环未完成，**从未产出 `report`/`verdict`**。故 `sec_report` / `sec_verdict` 与四个逐档键**全为 None**。
- **催化剂分析师** `a-catalyst--mid`：`content` 是**纯 markdown**（无 `{report, verdict}` JSON 包裹）⇒ 代表实例（mid 优先）解析不到 verdict。而 `--long/--short/--ultra_short` 三档**都有**合法 JSON 与 `catalyst_level` ⇒ `cat_verdict`（代表）与 f3 因子 `catalyst_level` 同源为空 ⇒ **「催化剂缺失」与「缺失因子：催化剂等级」是同一处故障的两个投影**。

### 结论三：熔断判据把「良好」的 B 也当异常 ⇒ 提示恒亮

`data-quality.rhai` 定档 A≥85 / B≥65，`B` 明确定义为「良好」；而 `dao::repo::data_quality_fuse::grade_is_abnormal` 原判据 `grade != "A"`（按 #8「B 级跨样本熔断」原话）⇒ **B 也累计「异常」**。只要 A 不易达成，连续 3 轮 B 即弹「决策可信度受限 / 本轮置信度上限按最保守档收紧」，把**本来可接受的 B 轮**的决策置信度压到最低档。用户 2026-10-10 判定为误报。

**本次动作（用户已授权「实在无法拿 A 就消除该提示」）**：

- `grade_is_abnormal` 收窄为 **`C` / `D` / `F` / 缺失（NULL·空串）才异常**；`A`、`B` 均为可接受档。
- 未识别字母 / 未 trim 的空白仍按异常（保守方向不变）；`C/D/F` 与「节点没跑成」照旧熔断 ⇒ **判别力恢复**，不是放宽证据面。
- **不回填历史行**：`abnormal` 列按模块纪律「只在写侧判一次」；而 `load_recent_streak` 数的是**自最新一轮往回的前缀**，下一轮（按新判据落库）即打断前缀 ⇒ 存量观测无需迁移，读数自行恢复。
  - ⚠ 该结论**已被补充四推翻**（漏考虑 `load_fused_intervals`），历史行已于当日回填。
- 同步更新：模块头与函数注释、`core.rs` 的两条断言、`data-quality.rhai` / `seed_stock_analysis.rs`（LLM 侧措辞）、`stock-analysis.ts` 字段注释、**11 语言** `stockAnalysis.trustNotice.dqiFuse` 文案（「未达 A 级」→「降档（C 级及以下）」）。

### 遗留（未完成，属独立子系统）

`a-sector--long`（只输出工具请求）与 `a-catalyst--mid`（Text 模式无 JSON 包裹）是**分析师节点产出形态**缺陷。修好它们（尤其使 `catalyst_level` 可达）预计把紫金矿业该轮从 B(80.8) 抬到 **A(87.7)**：`0.35×74.8 + 0.35×90 + 0.30×100 = 87.68`（gap 2→1、fc 88.9→100）。这是「真正拿到 A」的下一步。

### 门禁

- `cargo fmt --all --check`：EXIT=0。
- `cargo test -p axagent-dao data_quality_fuse`、`cargo test -p axagent dqi_observations_are_written_only_by_the_full_chain`：见提交记录。
- `npx dprint check`（改动的 locales + `stock-analysis.ts`）：EXIT=0。
- `npx vitest run DecisionTrustNotice.dqiFuse.test.tsx`：**8/8**。

## 补充三（2026-10-10）「A 级真正可达」的产端修复：VERDICT 标签畸形容错

### 根因（比补充二「Text 模式无 JSON 包裹」更精确）

补充二把 `a-catalyst--mid` 归为「纯 markdown 无 JSON 包裹」，**不完整**。逐字节核对
`_raw.a-catalyst--mid.content` 尾部（紫金矿业 `4419bd01…`）后确认：标签**存在且以
`<!-- VERDICT: ` 起、`-->` 止**，但**标签内 JSON 非法**。

`extract_verdict_tag` 只在 `serde_json::from_str` 通过时才返回 `Some`，因此整条
「标签 → 重构 `{report,verdict}` → 写 `params.<node>`」链全部落空 ⇒ content 保持纯文本
⇒ `a-catalyst--mid.content.verdict.catalyst_level` 解析失败。

40 快照全量扫描：**9 个 text+VERDICT 节点，标签 JSON 合法率 0/9**。形态三类：

| 形态 | 节点（实证） | 标签 JSON 报错 |
|---|---|---|
| **对象提前闭合 + 追加顶层字段** | `a-catalyst--mid`、`a-research--long` | `Extra data at char 801 / 554` |
| 双层转义（`{\"…\"}`） | `risk-convergence` | `Expecting property name…` |
| 空标签 / 截断串 | `research-mgr`、`a-news`、`a-hot-money`、`a-fundamentals--long` | `Expecting value` / `Unterminated string` |

`a-catalyst--mid` 的具体形态（模型在扁平判决对象写到一半就补 `}`，随后又追加两字段）：

```text
{"verdict":"偏多",…,"supplyShock":"…"}, "notApplicable":[…], "qualified":{…}}
                                       ^ 多写的 `}`
```

### 修复（产端解析容错，单点；`agent_executor.rs`）

- 新增 `drop_premature_close_braces`：字符串感知 + 括号深度判据，删除**最外层**
  （`depth==1`）后跟 `, "key":` 的多余闭合 `}`。合法 JSON 最外层不可能出现 `}, "key":`；
  对象数组内 `}` 深度=2 天然不命中；字符串值里的 `}, "x":` 因字符串感知不被误删。
- 新增 `repair_verdict_json`：依次尝试「原文 / 去提前闭合 / 再叠 `repair_json`」，
  任一候选解析为 object 即返回。在 `extract_verdict_tag` 主路径接入。
- 追加字段 `notApplicable` / `qualified` 被**合并进同一对象**而非丢弃（契约字段不丢）。
- 修 2/9（恰好覆盖紫金该轮受影响的 `a-catalyst--mid` + `a-research--long`）；
  其余 7 个异构形态（双转义 / 空标签 / 截断）按「改动尽量小」暂不扩面。

### 前后对比数据（生产库离线回放，紫金矿业该轮）

| 指标 | 修复前 | 修复后（回放） |
|---|---|---|
| `a-catalyst--mid.content` 是否合法 JSON | 否 | **是**（重构为 `{report,verdict}`） |
| `params.a-catalyst--mid` 是否写入 | 否 | **是** |
| `content.verdict.catalyst_level` | 不可达 | **`"L2业绩拐点级"`** |
| 数据质量缺口数 `gap_count` | 2 | 1（`a-sector--long` 仍缺，见「遗留」） |
| `fc`（因子完整度） | 88.9 | 100 |
| `composite`（`0.35·rq + 0.35·tc + 0.30·fc`） | 80.8（B） | **87.68（A）** |

> 算式：`0.35×74.8 + 0.35×90 + 0.30×100 = 87.68`（`rq≈74.8` 不变，`tc=100−10×1=90`）。

### 验证（本补充）

- 新增单测（`work_engine::executors::agent_executor::verdict_extract_tests`）：
  - `test_extract_verdict_tag_premature_close_then_appended_fields`（mid 形态，含 `notApplicable`/`qualified` 不丢）
  - `test_drop_premature_close_braces_keeps_valid_nesting`（对象数组 / 字符串内 `}, "x":` 回归保护）
- `cargo test -p axagent-rt-workflow --lib verdict_extract_tests`：**21/21**（含既有 8 条截断/畸形用例）。
- `cargo clippy -p axagent-rt-workflow --all-targets -- -D warnings`：EXIT=0。
- `cargo fmt --all --check`：EXIT=0。
- 生产库离线回放：mid 标签修复后 `catalyst_level` 解析为 `L2业绩拐点级`（见上表）。

### 仍未完成（遗留不变）

`a-sector--long`（agent 循环只发出文本式工具请求即终止，`tool_calls_made=[]`）仍使
`gap_count=1`。按上表，gap=1 时仍达 **A(87.68)**；若要追求 gap=0，需单独修该节点的
工具调用解析（`needs_inline_tool_parsing` / `parse_inline_tool_calls` 对
`{"function":…,"parameter":…}` 形态的识别），属另一子系统、风险更高，暂缓。

## 补充四（2026-10-10 续）熔断横幅「已连续 4 轮降档（C 级及以下）」误报

### 现象与用户判读

用户看到「⚠️ 决策可信度受限：数据质量已连续 4 轮降档（C 级及以下）并触发熔断」，据此判断「质量已跌到 C 级以下」。

### 实测（生产库 `axinvest`）

- `data_quality_observations` 共 7 行，**grade 全为 B**；其中 5 行健在（2 行孤儿）、6 行 `abnormal=1`。
- 最近 4 轮的冻结快照：

| 分析 | 快照 grade/score | 冻结 `dqiStreak` | 冻结 `dqiFuseState` |
|---|---|---|---|
| 我爱我家（今日） | B / 81.8 | 4.0 | fused |
| 紫金矿业 | B / 80.8 | 3.0 | fused |
| 中材科技 `ce684296` | B / 71.3 | 2.0 | ok |
| 中材科技 `239b0b8f` | B / 73.7 | 3.0 | fused |

⇒ **四轮全是 B（良好）**，无一轮是 C。

### 根因（两个独立因素叠加）

1. **存量行按旧判据落库**：`dqiStreak` 是**本轮开始运行时**从库中读到的前缀（不含本轮）。我爱我家那轮开始时，库里 4 条旧行仍是 `grade != "A"` 判据的产物 ⇒ `abnormal=1` ⇒ 前缀 = 4 ⇒ `fused`。该轮**自身**的观测行已按新判据落成 `abnormal=0`，但**冻结在快照里的 4 不会重算**。
2. **文案把「判据」写成了「事实」**：横幅括注「（C 级及以下）」本意是解释什么算「降档」，却被读成「本轮质量是 C 级及以下」。

### 修复

- **回填历史行**（6 行）：`set abnormal = 0 where abnormal <> 0 and upper(trim(coalesce(grade,''))) in ('A','B')`。原值全为 1，可逆。回填后整表 `abnormal` 全 0 ⇒ 真实 `streak = 0 / fused = false`。
  - ⚠ **推翻补充二「不回填历史行」的结论**：那条理由只覆盖 `load_recent_streak`（下轮自愈），**漏了同模块 `load_fused_intervals`** —— 它读**全量**观测导出熔断时间线（闭环权重历史降权），旧 B 行的 `abnormal=1` 会合成**假熔断段**且**不会**随新行自愈。`data_quality_fuse.rs` 模块头已同步订正。
- **文案**（11 语言）：`stockAnalysis.trustNotice.dqiFuse` 去掉「（C 级及以下）」括注、改为中性「降档」——判据仍准确，但不再暗示当前质量等级。zh-CN 净变化（vs HEAD）：`未达 A 级` → `降档`。

### 验证

- `cargo fmt --all --check`：EXIT=0。
- `cargo clippy -p axagent-dao --all-targets -- -D warnings`：EXIT=0。
- `npx dprint check "src/i18n/locales/**/*.json"`：EXIT=0；11 文件 `json.load` 通过、无 CRLF。
- 回填前/后：`abnormal=1` 行数 6 → 0；读侧 `streak` 4 → 0。

### 说明（刻意未改）

- **已冻结的旧快照仍显示 `dqiStreak=4`**：那是该轮运行时的真实读数（当时库确实如此），属历史审计值，**不回写快照**；用户重跑一轮即得干净结果（新读数 = 0）。
- LLM 侧措辞（`seed_stock_analysis.rs:6504`）与 TS 字段注释（`stock-analysis.ts:470,477`）里的「（C 级及以下）」**保留** —— 那些是**判据定义**，语义正确。

---

## 补充五（2026-10-10 续）「决策信息缺失」占位卡：前端 LLM 兜底缺两个入口

### 现象

用户报告决策卡显示「决策信息缺失 / 工作流已结束，但 portfolio-mgr 节点未输出可解析的决策（LLM 输出可能被截断或 JSON 残缺）」，
证据引用审计面板同步显示「不适用（本轮决策未产出）…**支撑率在此情形下没有定义，故不显示为 0%**」。

### 先排除「运行中误报」（与本次现象无关）

- `DecisionBanner` 与 `EvidenceCitationPanel` **只在 `status === "completed"` 挂载**（`StockAnalysisPage.tsx:548`），运行中走进度条分支（`:584`）；store 已按后端 status 区分（`stockAnalysisStore.ts:1371`）。
- 当时生产库**唯一**的 `running` 记录（紫光国微 `002049`，`3556a759…`，`created_at` = 当日 12:11:21）根本不渲染这两块 ⇒ 不是「运行中被误报」。

### 实测（生产库 `axinvest`，86 条 `stock_analyses`）

- 85 条 `completed`，其中 **8 条**（`2026-07-22 ~ 07-25`）`decision_json` 是 end-output 节点包装：

  ```json
  {"node_id":"end-output","output":null,"source":"portfolio-mgr","status":"terminated"}
  ```

  （其中 6 条另带 V50 注入的 `adjustedConfidence / agreementBreakdown / formulaLlmAgreement`。）
  `output` 为 `null` 且 `status="terminated"` ⇒ **公式侧（portfolio-mgr）确实未产出**；`decision_action` 全为 NULL ⇒ 后端
  `extract_evidence_citations` 兜底成 `UNAVAILABLE` 哨兵 —— 与用户截图里的 `action=UNAVAILABLE` **逐字吻合**。
- 该 8 条中 **7 条 `llm_decision_json` 存有完整决策**（典型：招商银行 `600036` / 07-25，`decision_json` 为 **NULL**，
  但 LLM 侧有 `action=买入 / confidence=68 / positionPct=12 / targetPrice…`）。

### 根因（前端单点，不是数据真缺失）

`stockAnalysisStore.loadAnalysis` 的 LLM 兜底原先只挂在「`decisionJson` 非空但解析成空壳」这一支：

| 入口 | 原行为 |
|---|---|
| ① `decisionJson` 解析成空壳 | 走兜底 |
| ② `decisionJson` 解析抛异常（截断 JSON） | **停在「决策缺失」** |
| ③ `decisionJson` 整列为 NULL | **整块被 `if (record.decisionJson)` 跳过 ⇒ 完全不兜底** |

⇒ `600036` 这类记录明明有可用 LLM 决策，UI 仍显示「决策信息缺失」。
另：hint 文案「（LLM 输出可能被截断或 JSON 残缺）」是**无据猜测** —— 实测证据是 `status="terminated"`（工作流在该节点被终止），并非 LLM 输出畸变。

### 修复

- **A（核心）** `stockAnalysisStore.ts`：把兜底抽成单点 `fillDecisionFromLlm()`，①②③ 三个入口共用；顺带删掉逐字重复的 30 行旧实现。
- **B（文案）** 11 语言 `stockAnalysis.decisionMissingHint` 去掉无据猜测，改为据实中性表述（公式侧缺失 + 可仅重跑决策 / 全量重跑）。

### 验证

- `npm run typecheck`：EXIT=0。
- 相关 4 套件（`stockAnalysisStore` ×2 / `DecisionBanner` / `agentOutput`）：**82/82**；新增 3 例回归（NULL 由 LLM 侧填充 / 截断 JSON 回退 / 两侧皆空负控）。
- `npm run test:run` 全量：**2213/2214** —— 唯一失败为 `DemandDiscoveryPage.test.tsx` 的 15s 超时（并发负载 flaky，单独跑 3.1s 通过），与本次改动无关。
- `npx dprint check`：EXIT=0；11 语言 locale JSON 合法、CRLF=0。

### 说明（刻意未改）

- 工商银行 `601398`（07-23）LLM 侧被守卫判 `__untrusted` + `verdict_missing`，两侧皆无决策 ⇒ 占位卡**仍应出现**；B 只是不再猜因。
- `EvidenceCitationPanel` 的 not-applicable 分支（含「支撑率不显示为 0%」）本身准确，属设计行为，保留。
- 未回写历史行的 `decision_json` / `llm_decision_json`（历史审计值）；修复只在**前端读取侧**，不改库。

---

## 补充六（2026-10-10 续）「决策缺失」的**真实故障轮**：归因订正（甲）+ 容错补齐（乙）

### 为什么补充五不够

补充五修的是**前端兜底**。用户随后在应用新构建后仍看到占位卡（`action` 由 `UNAVAILABLE` 变为「数据缺失」），
复查确认那是**另一条真实故障记录**（不是同一现象的残留），前端兜底覆盖不到它 —— 因为该轮的公式侧与
LLM 侧**都真的没有决策**（`llm_decision_json` 为空）。

### 实证（生产库 `axinvest`）

- 记录：紫光国微 `002049` / `2026-10-10` / `stock_analyses.id = b697487a-7d2d-472b-9915-4cdacd7fc405`
  / `template_version = 149` / `decision_action = '数据缺失'`（**全库唯一**）。
- 关联运行：`workflow_executions.id = 3a2e73a7-3d50-4a6c-8d09-be027ea8bc14`（`workflow_663fe429…`）
  / `status = partially_completed` / `total_time_ms = 247000`。
- **三个辅助取数源同时超时**（`node_executions` 逐次尝试）：

  | 取数节点 | 尝试结果 | 消费它的分析师（`extra_supplies`） |
  |---|---|---|
  | `t-limitup-pool` | `timeout` ×3 | `a-sentiment` / `a-hot-money` |
  | `t-pledge-data` | `timeout` ×3 | `a-lockup` / `a-fundamentals` |
  | `t-consensus-data` | `timeout` ×3 | `a-research` |
  | `t-macro-data` | `timeout` → 重试 `completed` | `a-policy`（侥幸跑通） |

- 该轮 `output_result` 的键里 **没有** `portfolio-mgr` / `a-sentiment--*` / `a-hot-money--*` /
  `a-fundamentals--*` / `a-lockup--*` / `a-research--*` —— 这些节点**从未执行**；
  只有 `a-market-analyst--*` / `a-catalyst--*`（无辅助供给，不被阻塞）与 `a-policy--long` 跑完。

### 根因（两处，互相独立）

1. **归因错（甲）**：`decision.rs` 的兜底 hint 只有三分支 ——
   `Skipped` / `Completed` / **`else` 一律当「Rhai 运行失败」**。而本轮的 `nodeStatus` 是 **`Pending`**，
   语义是「该节点**从未被调度**」，与 Rhai 毫无关系。落库的 `diagnostics` 因此同屏两套归因：
   `reasoning` / `rootFailureNode` 说「根因 `t-limitup-pool` 超时」，`hint` 却让用户去查 `present()`/除零。
   附带第二条：`reasoning` 的「N 个节点被级联跳过」把 `skipped_nodes` **全量**计入，而该清单含
   `disabled`（本轮唯一被跳过的正是未启用的 `sim-verify`）⇒ 文案谎称「**含 portfolio-mgr**」，
   而 `portfolio-mgr` 当时是 `Pending`。
2. **容错缺（乙）**：引擎的容错判据在**下游节点**上 ——
   `compute_ready_nodes` 的 `continue_on_fail_map` 按 **target** 查表（`rt-workflow/.../dag_store.rs`：
   上游 `Failed` 且 target 为 true 才不计该依赖边）。链尾的 `debate-bull-bear`（v7）/
   `research-mgr` / `trader` / `portfolio-mgr` **早就是** `true`，唯独**分析师**从未打开 ⇒
   辅助取数一超时，分析师永不就绪，整条决策链停摆。

### 修复

- **甲（归因订正，`stock_workflow/decision.rs`）**：
  - `hint` 改 `String`，`PlaceholderContext::collect` 前移；`else` 拆为
    `Failed`（保留 Rhai 文案）/ **其余非终态**（用 `ctx.root_failure` 给出「从未执行 + 上游 X 失败」的准确归因）。
  - `reasoning` 的「级联跳过」计数只认故障传播型跳过（`upstream_failed` / `upstream_skipped`），
    「（含 portfolio-mgr）」按实际成员判定（不再凭空宣称）。`diagnostics.skippedNodeCount` 仍为全量（原始清单）。
  - 新增回归测试 `extract_decision_json_pending_pm_blames_upstream_not_rhai`（Pending + 上游 `timeout` +
    唯一跳过为 `disabled` ⇒ hint 不得含「Rhai」、必须点名上游；reasoning 不得含「级联跳过」）。
- **乙（容错补齐，`stock_analysis_setup/seed_stock_analysis.rs`）**：分析师循环里，
  `extra_supplies` 非空的 6 个 base（`a-sentiment` / `a-hot-money` / `a-fundamentals` / `a-lockup` /
  `a-policy` / `a-research`）的全部逐档实例设 `continue_on_fail = true`。
  **`TEMPLATE_VERSION` 149 → 150**（图内容变更，不升版存量库被版本门挡住 ⇒ 修复不生效；
  `HORIZON_TIER_TEMPLATE_VERSION` 直接绑 `TEMPLATE_VERSION`，自动跟随）。
  - ⚠ 如实标注代价：node 级开关同时覆盖各分析师的**主**取数工具（`t-sentiment-data` 等）。
    这是有意的放宽 —— 分析师 prompt 第 2/4 条本就要求「诚实标记缺失并评估影响」，出降级报告远好于整轮无决策。
  - ⚠ 写在**取数节点自身**是**空操作**（引擎按 target 查表）—— 这一点是本轮定位中反复确认的关键判据。

### 预期的行为变化

- 辅助取数（涨停池 / 质押 / 一致预期 / 宏观）超时 ⇒ 对应分析师**照常执行**、在报告里标注该维度缺失；
  `debate-bull-bear` 之后按既有降级话术继续 ⇒ **本轮仍产出决策**（而非 `数据缺失`）。
- 真出现「所有分析师都被卡」的极端情形时，占位卡的 `hint` 现在指向**真正的根因节点**，不再误导去查 Rhai。
- 存量库需**重播种**（升版触发）后新图才落库；已生成的旧记录不回写。

---

## 补充七（2026-10-28）「9 个降级」是跨轮熔断读数；文案与判据三处不一致收口

### 现象与用户判读

用户：「最新的股票分析结果出现 9 个降级」→ 追问「为什么会出现决策不可信」。

实测（生产库 `axinvest`，记录 `bdb58aea…` = 002812 / `analysis_kind=live` / 2026-10-28 05:47 / 模板 v189）：

- 本轮 data-quality：grade **A**、score 86.9、`degraded_count=0`、`gap_count=0`、10/10 `status=normal`；
- `decision_json`：`weightsCollapsed=false`、`collapseReason=none`、`dataGaps=[]`，但 **`dqiFuseState="fused"`、`dqiStreak=9`**；
- reasoning 原文：`⚠️数据质量熔断(连续9.0轮未达A级),置信度上限压至60.0`。

⇒ 「9」**不是本轮的 9 个降级**，是 `dao::repo::data_quality_fuse::load_recent_streak` 的**跨轮**计数；点亮「决策可信度受限」的是 `DecisionTrustNotice` 的 `fused` 分支（该分支与本轮 grade 无关，属设计）。

### 那 9 轮是谁（按 `observed_at` 倒序、剔孤儿、同 `analysis_id` 去重 —— 即 dao 口径）

| # | observed_at | grade | 股票 | 来历 |
|---|---|---|---|---|
| 1 | 10-28 03:42 | D | 000001 | live（10 位分析师全缺位） |
| 2 | 10-27 23:50 | C | 001359 | replay / time_travel |
| 3–8 | 10-27 08:14→12:17 | D | 002343 / 001216 / 003006 / 600802 / 600267 / 002141 | replay / **mover_recall**（as-of 2026-06-11，每条都是「10 个缺失，报告 0.0」） |
| 9 | 10-27 08:10 | D | 000001 | live |
| — | 10-26 20:59 | **B** | 603466 | live ⇒ 正常档，前缀在此断开 |

计数器**不分票、不分 live/回放**（`DQI_SCOPE_GLOBAL` 注释明写「一轮分析一条，不区分票、不区分档」）⇒ 9 行里 7 行来自补分析回放批次。熔断后 `cap_floor=60` 压顶：本轮 A 档阶梯上限 99.9 被压到 60.0（同一条 reasoning 里还有第二处独立封顶也压到 60）。

### 三处不一致与修法

1. **文案比判据说得严**。判据自 2026-10-10 起 `A` 与 `B` 都是正常档（`grade_is_abnormal` 只把 C/D/F/无等级算异常，见本文件补充二/四），但仍有 5 处写「拿不到 A 级 / 未达 A 级」：`portfolio-mgr.rhai` 的 reasoning 两句、`hooks.rs` 的注入变量描述与段头注释、`seed_stock_analysis.rs` 熔断门节点描述（LLM 可见）、`DecisionTrustNotice.tsx` 的组件注释、`data_quality_placeholder_gate.rs` 的历史段。另 11 语言 `stockAnalysis.weightCollapseDqi` 带括注 `dqi<25`，而代码里根本没有该阈值（`dqi_collapsed = dqi_grade_str == "F"`）。
   → 统一改「降档（C/D/F 或无等级）」；删 11 语言的 `dqi<25` 括注（每份一行）；顺带把 reasoning 里 `连续9.0轮` 的 f64 直插改 `to_int()`。
2. **「连续 N 轮」跨过空档却不披露**。`alive_analysis_ids` 把「所属分析已删除」的观测行剔出前缀，模块头却声称「剔除它不会把某条真实轮次算少」——**不成立**：本次 15 条孤儿行里的 `10-27 14:29:51`（B 级、正常档）本可把前缀断在 **2**，剔除后读出 **9**；反向（被剔的都是异常轮）则低估。
   → 判据口径**不动**（剔除仍是判据），只补披露：`DqiStreak` 加 `orphans_skipped` / `consecutive_abnormal_incl_orphans`，聚合逻辑抽成纯函数 `streak_from_rows`（跨度下界取**第一个存活正常档**，不是最后一条被数的异常轮——取后者会漏报「正常档孤儿夹在两条存活异常轮之间」那种形态）；`hooks.rs` 注入 `dqi_orphans` / `dqi_streak_incl_orphans`；reasoning 追加「其间 N 轮观测因所属分析已删除未参与(计入则为 M 轮)」；`portfolio-mgr` 的 `input_mapping` 加两条同名映射 ⇒ **`TEMPLATE_VERSION` 189→190**。
3. **摘要下溢成负数**。`data-quality.rhai` 的 `low_count = n_int - good_count - gap_count`：`n_int` 只数**有 verdict** 的分析师，`gap_count` 却把无 verdict / untrusted 者一并计入 ⇒ 有人缺席时减法为负。实证：当日 8 条 D 级轮的摘要写「10 个缺失，**-9 个低置信**」。同一文件输出段（`status_*_count` 处）早已写明「不用那种减法，会下溢成负数」，减法却留在摘要侧。
   → `low_count = low_confidence_analysts.len()`，与紧随其后的点名清单同源。

### 现网读数（修复后自愈，不回写历史）

2026-10-28 06:57 补分析批次落了一条 D 级观测，其前一条是 05:53 的 A（正常档）⇒ 现读 `consecutive_abnormal=1`、`fused=false`。
**冻结在 05:47 那轮快照里的 `9` 不回写**（沿用补充四裁定：那是该轮运行时的真实读数，属历史审计值）。

### 刻意未改 / 待裁

- **呈现层不带空档披露**：`trustNotice.dqiFuse` 标签仍只显示 `dqiStreak`。要在标签上并列须把 `dqiOrphans` 打通 DTO + 11 语言；且「轮数」与「被剔除行数」不同轴，并排显示容易诱导用户相减 ⇒ 先只进 reasoning 长文。**待裁**。
- **判据本身是否该计入孤儿行**（甲：维持剔除 + 披露，本批已落；乙：孤儿行参与前缀 ⇒ 本例 streak 自动变 2、熔断解除，但让"已删除的分析"继续决定现网置信上限）。本批刻意选甲，**待裁**。
- 15 条存量孤儿观测行**不删**：读侧本就不参与 ⇒ 删除不改变任何读数，属历史审计值。
- `docs/plans/PLAN-stock-decision-reliability.md` 里「连续 3 轮未达 A 级」是当日实测记录（快照），按「不回写历史快照」裁定保留。
- 那 8 条 D 级轮的**上游成因**（分析师层整批没产出、报告 0.0）不在本批范围，属独立的产出形态子系统（见补充二/三「遗留」）。

### 验证（前端与静态门，本轮实测）

- `npm run typecheck` `EXIT=0`；`npm run lint:eslint`（oxlint）`EXIT=0`（0 warnings / 0 errors，1418 文件）。
- `npx dprint check "src/i18n/locales/*.json" + 改动的 tsx` `EXIT=0`。
- `check-i18n-key-parity`：11 份 key 路径集合一致（各 16256 条）；`check-i18n-untranslated` `EXIT=0`；`check-i18n-placeholder-usage` `EXIT=0`；`python scripts/check_i18n.py` 全部通过；`check-errorcode-alignment`：后端 448 码 ↔ 11 语言对齐一致。
- `check-input-mapping` / `check-panel-var-landing` / `check-builtin-vars-keys` / `check-replay-live-consistency` 四条 `EXIT=0`。
- `npx vitest run` 分析组件全域：**596/596**（83 文件；首跑在并发编译负载下 1 条超时红，重跑全绿）。
- `cargo test -p axagent --lib dqi`：**5 passed / 0 failed**（`MAIN_TEST_EXIT=0`）——含新增
  `portfolio_mgr_dqi_fuse_note_discloses_the_skipped_gap`（钉「连续 9 轮降档 + 其间 4 轮…计入则为 2 轮」
  真出现在 reasoning 里，并负控 `orphans=0` 时那句披露不得凭空出现）、
  `portfolio_mgr_receives_dqi_fuse_inputs_on_both_chains`（两条链的 portfolio-mgr 都带 5 个同名映射）、
  `dqi_fuse_gate_is_wired_on_both_chains`、`dqi_observations_are_written_only_by_the_full_chain`。
- `cargo test -p axagent-rt-workflow --test data_quality_placeholder_gate`：首轮 **49 passed / 0 failed**
  （改动前形态）；新增 `summary_low_count_never_underflows_when_most_analysts_are_absent` 后重跑见下。
  该用例本身即负控：旧式 `n_int - good_count - gap_count` 在本夹具下得 `1-1-9=-9`，必红。
- `cargo fmt --all --check`：`EXIT=1`，仅 2 处 diff 且分属
  `crates/astock-data/src/vendors/tencent.rs`（他人未提交文件，本批未碰）与本批新用例的换行偏好（已按
  rustfmt 建议改正）；本批四个 Rust 文件经 `cargo fmt -p` 后无残留 diff。
- dao 侧 `data_quality_fuse`（含 4 条纯函数口径 + 1 条端到端落库用例）与 `clippy -p axagent-dao
  --all-targets`：见本节末尾补记。clippy 首轮抓到本批新写的 3 条 lint（两处
  `unnecessary_sort_by` + 一处 `manual_contains`），已全部改正后复跑。

---

## 补充八（2026-10-28 续）第四处不一致：解密失败被报成「不支持的数据供应商」

### 现象与用户判读

追问补充七里那 9 条 D 级轮的成因。用户随后批准「继续修第四处（解密失败报错）」。

### 实测（生产库 `axinvest` + 节点级现场）

- 记录 `bdb58aea…`（002812 / live / A 级）快照 `report.a-*`=21、`_raw.a-*`=21、`params.a-*`=21；
  而 9 条 D 级轮**三个键数都是 0** ⇒ 分析师层不是解析失败，是 LLM 调用整批没发出。
- 执行 `664e4fd0…`（回放 002343）：284 条节点尝试中 **235 次失败、涉及 55 个节点，全部是 `a-*`**，
  每个 6 次重试，错误逐字 `UNSUPPORTED_PROVIDER: API key decryption failed: Crypto error: Decryption failed: aead::Error`；
  同批 `completed` 的 49 个节点全是 code/tool 节点（`analyst-brief--*` / `portfolio-mgr` / `data-quality` 等）。
- live 同因：执行 `9916375f…`（000001 / 02:41）232 次同类失败、`status=partially_completed`。
- 已排除的两条猜测：① 环境变量覆盖主密钥（进程内 / User / Machine 三处均未设置）；
  ② 多密钥轮询撞上坏行（默认 provider DeepSeek 只有 1 行密钥，`get_active_key` 在 `len==1` 时直接返回）。
  仍缺的读数：`[LLM-RESOLVE] 选中 provider`（含 provider_id / key_id）—— 库侧 `gateway_usage` 为 **0 行**、
  快照 `_meta` 不记 provider ⇒ 定不到失败那次解析到哪一家，属应用侧日志。

### 根因

`crates/rt-workflow/src/work_engine/executors/llm_resolve.rs` 把 `decrypt_key` 的失败套在
`resolve_model_for_node` 之后同一个 `UNSUPPORTED_PROVIDER` 码上；同一条故障在缓存路径
`executors/agent_executor.rs` 里用的却是语义正确的 `API_KEY_DECRYPT_FAILED`。⇒ 一条故障两个码，
归因随「有没有命中缓存」而变，界面把「密钥解不开」显示成「不支持的数据供应商」。

### 修复（三件）

1. `llm_resolve.rs` 解密站点改码 `API_KEY_DECRYPT_FAILED`（与缓存路径同码）。
2. **重试语义随码改变，是有意的取舍**：`API_KEY_DECRYPT_FAILED` 在 `node_executor_trait.rs`
   的 `NON_RETRYABLE` 清单内，`UNSUPPORTED_PROVIDER` 不在。改码后这类节点**一次即定**，
   不再空跑六次（实测那次批次因此多花 235 次尝试）。依据：master key 与密文都不随重试改变。
   ⚠ 代价与未做的更优解：多密钥供应商若只有一行坏，旧行为靠轮询"意外"转到好行；正解是
   key 级 failover（仓内既有 `report_key_failure` 形态），属另一格改动，本批没做。
3. `data-quality.rhai` 摘要尾句改成分支共用的 `advice` 三态：无缺席 ⇒ 空串；**十维全缺** ⇒
   「分析师层本轮整批无产出…上游取数与模型侧调用（供应商/密钥）皆有可能，请按各分析师节点的
   错误码定位」；部分缺席 ⇒ 原句「建议检查对应分析师的上游工具数据源」。
   顺带补上 A/B/C 分支**根本没有这一句**的不对称（与 2026-10-09 补「缺失/低置信」同型问题）。

### 验证

新增三条用例（实测读数）：

- `data_quality_placeholder_gate.rs`：**51 passed / 0 failed**（`DQ_GATE_EXIT=0`，原 49 + 本批 2）。
  `summary_advice_stops_blaming_data_source_when_every_analyst_is_absent` 钉「十维全缺 ⇒ 说两种可能」
  与「只缺九位 ⇒ 仍指数据源」两格；`summary_low_count_uses_the_named_list_not_a_subtraction`
  因 `low_count` 只出现在 D/F 分支、而本文件夹具把因子完整度固定在 90（综合分最低只到 C，实测 48.0）
  构造不出 D ⇒ 改为源文扫描 + 内存负控（把旧式减法塞回同一行，判据必须恰红一条）。
- `llm_resolve.rs` 两条：`api_key_decrypt_failure_stays_non_retryable` **ok**（钉住"本码不可重试"
  这一后果 + 对照 `UNSUPPORTED_PROVIDER` 仍可重试）；`decrypt_failure_keeps_the_api_key_decrypt_code`
  第一版**假红**（600 字符窗口正好截在 `error_code::API_KEY_DECRY` 上，且窗口再宽会吞掉下一站
  adapter 缺失那处本应保留的 `UNSUPPORTED_PROVIDER`）⇒ 判据改成「解密站点上先出现哪个码」，
  配「把码换回旧值必须翻红」的负控；复跑读数见本节末。
- dao：`data_quality_fuse` **15 passed / 0 failed**；`clippy -p axagent-dao --all-targets` `EXIT=0`；
  `clippy -p axagent-rt-workflow --all-targets` `EXIT=0`（含他人未提交文件，一并干净）。
- `cargo fmt --all --check`：`EXIT=1`，唯一 diff 在 `crates/astock-data/src/vendors/tencent.rs`
  （他人未提交文件，本批未碰）。

---

## 补充九（2026-10-28 续）三项裁定落地：判据计入孤儿轮 + 披露上决策卡 + key 级 failover

用户在补充七/八的待裁项上一次点完三项，本批全部落地。**模板 `TEMPLATE_VERSION` 191→192**
（退役一条注入变量 + 新增一条输出字段，都是图内容变更，不升版存量库读不到）。

### 甲：熔断判据改为**计入**孤儿观测轮

- `dao::repo::data_quality_fuse::streak_from_rows`：前缀按**全部观测行**数（孤儿行也参与断开），
  `observations` / `deduped` 同步跨两个集合；原 `orphans_skipped` +
  `consecutive_abnormal_incl_orphans` 两个字段合并成一个披露项 `orphans_counted`
  （"这 N 轮里有几轮的所属分析已删除"），只数**前缀之内** —— N 之外的行不在这句话里。
- 两个读口必须同口径：`load_fused_intervals`（闭环权重降权线）同步去掉存活过滤，
  否则决策卡与降权线各说一套。`alive_analysis_ids()` 保留，但只用于**打标记**不再过滤。
- `dqi_streak_incl_orphans` 退役（新口径下差值恒 0，留着就是第二份载体）：
  `hooks.rs` 注入数组 5→4、种子 `input_mapping` 与键清单测试、前端白名单同步。
- 测试：`orphan_observation_is_ignored_by_streak_and_timeline` 改名
  `..._counts_in_streak_and_timeline` 并翻转期望（observations 4→6、streak 3→5、
  熔断段起点 5_000→3_000）；四条纯函数用例改为"计入 + 披露"两组方向；
  端到端落库那条把孤儿行从正常档改成异常档，才真的同时钉住「计入」与「披露」。
  夹具里 `grade="B" + abnormal=1` 的自相矛盾一并改掉（判据只读布尔，但夹具该与生产同形）。

### 乙：披露上决策卡标签

`portfolio-mgr.rhai` 新增输出 `dqiOrphans`，reasoning 那句改为
「…(连续N轮降档),置信度上限压至60.0(其中M轮的所属分析已删除,记录不在库里)」；
前端三处接线：`types/stock-analysis.ts` 加 `dqiOrphans?`、`agentOutput.ts` 白名单透传
（camelCase 与 snake_case 都认）、`DecisionTrustNotice` 在 `fused` 分支紧跟轮数那句成第二句。
新增 11 语言键 `stockAnalysis.trustNotice.dqiFuseOrphans`（`{{count}}` 插值，与既有
`gapReason` 同形、不建复数档）。测试两层：渲染侧三条（有孤儿轮出那句 / 0 不出 / 未熔断不出），
生产侧一条（normalizeDecision 白名单不漏字段，含 0 与缺席可区分）；
既有「三载体反向锁」两处的键清单从四个补到五个。

### 丙：多密钥供应商的 key 级 failover

`llm_resolve.rs` 解密失败不再"一行定死"：以 `prov.keys.len()` 为上限，逐把调
`provider_repository().get_active_key(&prov.id)`（dao 侧轮询游标每次前进一格）再解，
全部解不开才回 `API_KEY_DECRYPT_FAILED`，错误文本带上"已轮换 N 把 key"。
依据：一行坏密文是**行级**故障（实测 Agnes 4 把 key、其中一把 `last_error='Validation failed'`
且从未验证通过），而它配合那个不可重试码，等于一行坏 key 让整条链的分析师节点全部拿不到模型
（实测 55 个节点全灭、每个重试 6 次）。
接线用源文判据 `failover_wiring_gaps(src)` 钉三条（上限挂在 key 行数 / 解的是 candidate /
失败后真的赋回下一把），三个负控各只红对应那一格。
⚠ **运行证据仍缺**：现网 DeepSeek 只有 1 把 key（触发不了轮换），Agnes 那把坏 key 属用户凭证、
本批没动；行为级测试要给 `resolve_provider_and_adapter` 注入假 ProviderRepository，
而它走的是进程级全局（`harness::repositories::set_provider_repository`），并行测试下互污。

### 现网读数（本批改动前后对照）

- 本批改动前的新序列（observed_at 倒序，含存活标记）：07:55 D(t) / 06:57 D(t) / 05:53 A(t) /
  03:42 D(t) / 23:50 C(t) / **14:29:51 B(f)** / 14:29:39 C(f) / 14:00:53 C(f) / 14:00:21 C(f) …
- 新判据现读 `consecutive_abnormal = 2`（05:53 那条 A 断开）、`orphans_counted = 0` ⇒ 未熔断。
- 那条 9 只存在于 `05:47` 那轮的冻结快照里（不回写，沿用补充四裁定）。
- ⚠ 分析师凭证问题仍在产出新 D 级轮（06:57、07:55 各一条）⇒ 再落一条 D 就会重新触发熔断，
  这是丙与"缺 [LLM-RESOLVE] 读数"那一格要解的事，不是本批的计数口径。
---

## 补充十：本批入库归属 + 门禁读数台账（2026-10-28 午）

### 入库归属（不是我提交的）

工作树现在只剩 `src-tauri/src/commands/stock_analysis_setup/mod.rs` 带我的改动，其余本批文件
均已等于 `HEAD` ⇒ **先前取到的门禁读数对入库内容有效**：

| 本批内容 | 落点提交 |
|---|---|
| 甲（孤儿观测计入判据）+ 乙（空档上进决策卡标签）+ 丙（key 级 failover）+ 11 语言 + 前端 4 文件 | `c6494fb8b` |
| 第四处（`src-tauri/src/commands/data-quality.rhai` 的 `low_count` 与摘要归因 + `src-tauri/crates/rt-workflow/tests/data_quality_placeholder_gate.rs` 两条新判据） | `ff9c46f39`（并行会话的大提交，把我当时在工作树里的改动一起带走） |
| 补充九的文档段 | `c6494fb8b`（+59 行） |

### 门禁读数台账（真实退出码逐条落 `.workbuddy/tmp/`）

| 门 | 读数 | 日志 |
|---|---|---|
| `cargo test -p axagent-dao dqi` | 16 passed / 0 failed，`DAO_TEST_R2_EXIT=0` | `dqi-v192-rust2.log` |
| `cargo test -p axagent-rt-workflow --lib`（llm_resolve 三条） | 3 passed，`LLMRESOLVE_R2_EXIT=0` | 同上 |
| `cargo test -p axagent-rt-workflow --test data_quality_placeholder_gate` | 51 passed，`DQGATE_R2_EXIT=0` | 同上 |
| `cargo clippy -p axagent-dao --all-targets` / `-p axagent-rt-workflow --all-targets` | 0 告警 | 同上 |
| `cargo fmt -p axagent --check` / `cargo fmt --all --check` | 均 0（`FMT_EXIT=0` / `FMT_ALL_R3_EXIT=0`，后者覆盖 36 个 crate） | `fmt-mod.log`、`fmt-all-r3.log` |
| 前端 | vitest 4 files / 98 passed（含 `DecisionTrustNotice.dqiFuse` 12 条、`agentOutput` 56 条）、`tsc --noEmit` 0、`dprint check` 0 | `v192-*.log` |
| i18n 三门 | 11 语言 key 集合一致（16257 条）、未翻译 0、占位符干净、错误码 448 ↔ 11 语言对齐 | `v192-parity/untrans/ph.log`、`gate-check-errorcode-alignment.log` |
| 行号引用门 | `--selftest` 0（两类判据都能红）、`--ci` 0 | `facts-ci.log` |
| **收口全量** `cargo clippy --workspace --all-targets --all-features -- -D warnings` | `WS_CLIPPY_EXIT=0`（11m 47s） | `ws-gates.log` |
| **收口全量** `cargo test --workspace` | `WS_TEST_EXIT=1`，但**断言层零红**：109 个测试目标 9049 passed / 0 failed；唯一失败是 `Doc-tests axagent_lib` 编译阶段 5×`E0463` | 同上 |
| 上述 E0463 的判定实验 `cargo test -p axagent --doc` 单跑 | `DOC_R2_EXIT=0`（11:38:34→11:54:40，4+1 条 doctest 全 ignored）⇒ **并发构建瞬态，不是缺陷** | `doc-r2.log` |
| 主 crate `cargo test -p axagent --lib dqi` | **5 passed / 0 failed**（794 条被过滤），`MAINDQI_R4_EXIT=0` | `dqi-main-r4.log` |

⚠ 07:00 那次 `check-single-source-facts` 报的两条客观红（`crates/disk-cache/src/lib.rs` 指空行、
`crates/rt-workflow/src/work_engine/executors/condition_executor.rs` 指空行）**不是本批引入的**，
已由 `38841b5e5` 修掉；本轮重取即为 0。

### 主 crate lib 测试为什么到现在才取到：HEAD 自身编译不过

`ff9c46f39` 把 v191 的 `context_slim_defaults_on_and_legacy_db_is_forced` 写进了
`mod horizon_tier_fanout_tests`（`src-tauri/src/commands/stock_analysis_setup/mod.rs:4205` 起），
但该用例调用的 `seed` / `force_version` 定义在**兄弟**测试模块 `mod version_gate_tests`
（同一文件 2797 起）里，默认模块私有 ⇒ `lib` 的 **test target 编不过**，6 条 `E0425`
（4×`seed`、2×`force_version`）。`cargo check` / `cargo clippy`（不带 `--all-targets`）/
`cargo build` 都看不见它 —— 正是 AGENTS.md「必须 `--all-targets`，test target 不被 lint/编译」那一格的实证。

本地最小修（**复用而非另抄第二份**，按禁区 12）：

- `mod.rs:2814` `async fn seed` → `pub(super) async fn seed`
- `mod.rs:2848` `async fn force_version` → `pub(super) async fn force_version`
- `mod.rs:4214` 新增 `use super::version_gate_tests::{force_version, seed};`

两处模板 id 同值已核（`version_gate_tests::TEMPLATE_ID = "stock-analysis"`、
`SOURCE_TEMPLATE_ID = "stock-analysis"` @ `src-tauri/src/commands/stock_analysis_setup/seed_stock_analysis.rs:8525`）
⇒ 跨模块复用不改语义。本批裁定是**只改不提交**，回退：
`git checkout -- src-tauri/src/commands/stock_analysis_setup/mod.rs`。

### 仍待取 / 仍待运行验证

1. ~~主 crate `--lib dqi`~~ **已取**：上面三条修完即 `5 passed / 0 failed`，逐条为
   `portfolio_mgr_dqi_fuse_note_discloses_orphan_rounds`（乙：熔断句 + `dqiOrphans=4`，负控 `orphans=0` 那半句一个字都不出）、
   `portfolio_mgr_receives_dqi_fuse_inputs_on_both_chains`、`dqi_fuse_gate_is_wired_on_both_chains`、
   `dqi_observations_are_written_only_by_the_full_chain`、`portfolio_mgr_dqi_cap_forces_action_ceiling`。
2. ~~主 crate 定点 clippy~~ **已取**：`cargo clippy -p axagent --all-targets --all-features -- -D warnings`
   ⇒ `MAINCLIPPY_EXIT=0`（`Finished dev profile in 12m 59s`，无 warning 无 error，日志 `main-clippy.log`）。
   ⚠ 这一步此前一直被 1 里那 6 条 `E0425` 挡在编译前 —— **不带 `--all-targets` 的 `cargo check` / clippy 看不见 test target**，
   正是 AGENTS.md 那条铁律的又一次实证。
3. ~~收口一次全量~~ **已取**（见台账最后三行）：全量 clippy `WS_CLIPPY_EXIT=0`；全量 test 断言层
   9049/0 全绿，只有 `Doc-tests axagent_lib` 报 5×`E0463 can't find crate`。
   那 5 条报的行是**函数体/模块顶的普通 `use`**（`src-tauri/src/init/services.rs:499`、
   `src-tauri/src/init/trigger_recovery.rs:18`、`src-tauri/src/commands/_shared_state.rs:14`、
   `src-tauri/src/commands/agent/mod.rs:29`、`src-tauri/src/commands/conversations/mod.rs:2621`），
   不是文档示例 ⇒ 属 rustdoc 依赖解析，而 `--extern` 里那五个 rlib 现在都在、mtime 11:13–11:30
   正落在那次跑的中间 ⇒ 并行会话在同一 `target/` 重建换文件。**单跑 `--doc` 复验为 0** ⇒ 瞬态定性成立。
   ⚠ 判据层收获：共享工作树里 `--workspace` 的 doctest 阶段**本身会被并发构建打成假红**，
   看到 `E0463` 先查 rlib 是否存在 + mtime 落点，再决定要不要动代码。
3. 运行验证：重启应用 ⇒ v192 重播种 ⇒ 真跑一条分析，看决策卡标签那句空档披露与
   `[LLM-RESOLVE] 该 key 解密失败 ⇒ 换该供应商的下一把` 是否成句/成行。
