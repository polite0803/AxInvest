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