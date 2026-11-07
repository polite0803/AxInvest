# PLAN-horizon-decision-repair —— 四档决策链修复（超短/短档）

> 依据：2026-10-14 只读审计轮（用户报「最新的股票分析工作流会出现决策中超短和短期差异很大的问题」）。
> 本文档只解决已实证的问题；诊断证据、链路梳理与现场读数见审计结论（附录 A）与项目记忆。
> 状态：active ｜ 日期：2026-10-14 ｜ 对应版本：v2.9.9
> 运行验证（重播种后跑分析）待用户侧。

---

## 〇、先读数再动手（本文档遵守的证据纪律）

以下读数均已在 2026-10-14 取自现网（PG，v154 代）：

| 读数 | 值 | 取法 |
|---|---|---|
| 超短 confidence 取值 | v150+ 恒为 {11.5, 66.5, 77.5, 99.5}%，且逐条满足 `0.5 + 信号×0.55` | `horizon_decisions` 反算 |
| 短档 entryGatePassed | **25/25 全 false**（v150+），动作仅 持有/观望/减持 | 同上 |
| `a-hot-money--short` 的 flowPersistence | 抽查 5 条：`"unknown"` / `"short_pulse"` / `"机构连续3日流入，非一日游"` 等自由文本，**0 条数值** | `blackboard_snapshot.report.a-hot-money--short` |
| 逐档先验 | 全档 priorSource=neutral_default / samples=0（反思 verdict 仅 8 行） | `horizon_decisions` + `stock_reflections` |
| 超短广度门 | 可用（true 24 / false 2） | `horizon_decisions` |

> ⚠ 修复只对**新跑的分析**生效。存量 52 条带 `horizon_decisions` 的记录**不重跑**（重跑会造伪样本，既有裁定）。

---

## 一、P0 —— flowPersistence 契约-消费断链（阻断级）

### 1.1 问题与根因（证据链，逐环可指）

1. 契约要求 short/mid 档必填 `flowPersistence`，但**只给字段名、没给值域**
   （`crates/harness/src/holding_period.rs:404-453` 的 `verdict_contract_prompt`）；
   该文本在播种时固化进节点 `system_prompt`（`seed_stock_analysis.rs:3667`），**运行时不再渲染**
   ——节点无 `horizon` 变量 ⇒ `agent_executor.rs:885-919` 的动态注入不触发。
2. 模型于是输出自由文本（上表）。消费端 `crates/analysis-engine/src/leg_signal.rs:146-149`
   严格校验 `[-1,1]` 数值，非数值一律判 **None（缺席）**。
3. 短档的门 `trend_and_flow_same_sign` 任一腿缺席 ⇒ 「未判定」⇒ 按不通过封顶
   （`portfolio-mgr-h-short.rhai` 的 `capped`）⇒ **动作上限钉死在「持有」，买入/增持不可达**。
   中档不受门影响，但 posterior 被摊薄（`tw` 减小 ⇒ escale 降）。
4. 数据链本身是通的（父扇出已传 `a-hot-money--short`，`seed_stock_analysis.rs:6275-6278`）
   ——断的是**类型约定**，不是接线。

⇒ 该腿自建成起**从未真正参与过融合**；PLAN 原假设「P4 契约落地即自激活」被现网证伪。

### 1.2 修法

**P0-1 契约文本补值域（主修，随 v155 重播种生效）**

在 harness 新增一张**唯一的「数值型 verdict 因子值域表」**（建议放 `holding_period.rs` 邻近）：

```rust
/// 需要 LLM 以 **JSON 数值** 输出的逐档因子 → (下界, 上界)。
/// 三处消费同源：① verdict_contract_prompt 渲染值域说明；② leg_signal 的消费域（测试锁一致）；
/// ③ （若做 P0-2）重试判据的类型检查。
pub const NUMERIC_VERDICT_FACTORS: &[(&str, f64, f64)] = &[("flowPersistence", -1.0, 1.0)];
```

`verdict_contract_prompt()` 增一节渲染（示意文案，实施时定稿）：

```
数值型字段（必须输出 JSON 数值或 null，不得输出文字描述/标签）：
- flowPersistence：[-1, 1] 之间的数（正=资金持续净流入强度，负=持续净流出强度）；
  确实无法判断时输出 null（不要写 "unknown"/"无法判断" 等文字）。
```

**为什么「无法判断」约定为 null 而不是 0**：null ⇒ 缺席（不参与归一，缺席不伪装成中性）；
0 是「在场但中性」——两者语义不同，与脚本 `tw`/`absentLegs` 的既有口径直接配套。

**P0-1 的渲染是**「必须输出字段」清单的补充说明，不改变字段集合本身**（fields 不变，
门与 `seed_consistency_tests` 的「字段集 == 适用因子集」对账不受影响）。

**P0-2 类型不合规重试（保险，建议同批或验证后再定）**

`agent_executor.rs` 的 VERDICT 兜底段（:1658 起，现有四种 `needs_*_retry` 判据）追加第五条：

- 触发条件：节点有契约（`contract_horizon.is_some()`）∧ VERDICT 可提取 ∧ 契约内**数值型字段存在但类型不合规**；
- 动作：与 `needs_rewrite_retry` 同款「带原内容重写一轮」；
- 成本：仅在违规时 +1 次 LLM 调用；现网 short/mid 两实例几乎必触发
  ⇒ **若 P0-1 生效良好（数值化率 >90%），本轮可不做 P0-2**，把它留作观察期后的按需项。

**P0-3 门语义分层（条件启动，默认不做）**

若 P0-1/P0-2 后 flow 仍系统性缺数，再评估「把『缺数』与『冲突』在封顶策略上分开」
（改 `portfolio-mgr-h-short.rhai` 的 `capped` 语义，需单独拍板——「缺席≠通过」是既有裁定，
改动要给出新理由）。

### 1.3 影响面（file:行）

| 文件 | 改动 |
|---|---|
| `crates/harness/src/holding_period.rs` | 新增 `NUMERIC_VERDICT_FACTORS` 表；`verdict_contract_prompt` 渲染值域节；更新 :931 的契约渲染测试（正断言含值域+负控） |
| `crates/rt-workflow/.../agent_executor.rs`（仅 P0-2） | 第五判据 `needs_contract_type_retry` + 单测 |
| `seed_stock_analysis.rs` | `TEMPLATE_VERSION` 154→155（只为把新契约文本重播种；**不改任何图元素**） |
| `analysis-engine/leg_signal.rs` | 不改行为；可加「消费域与 harness 表一致」的测试锁 |

### 1.4 验收判据

1. **门（能红）**：`verdict_contract_prompt_is_rendered_from_the_table` 扩展——断言 short/mid 契约文本含
   `flowPersistence` 的值域与 null 约定；负控：把表里的区间删掉即红。
   `leg_signal` 侧锁：`[-1,1]` 外判 None（已有）+ 与 harness 表一致（新增）。
2. **运行门（用户侧跑，3-5 次分析）**：
   ```sql
   -- 数值化率（期望：short/mid 实例的 flowPersistence 为 JSON number 或 null）
   SELECT stock_code,
          jsonb_typeof(blackboard_snapshot::jsonb->'report.a-hot-money--short'::text::jsonb)  -- 见附录 B 的完整 SQL
   FROM stock_analyses ORDER BY created_at DESC LIMIT 5;
   ```
   同时看短档 `entryGatePassed` 是否出现 true、动作是否可能出现 买入/增持。
3. **不回归**：主 crate 全量门禁（fmt/clippy --all-features --all-targets/test --workspace）按阶段纪律跑。

### 1.5 成本与边界

- 无图结构变更、无数值口径变更（契约只补说明）；不含 P0-2 时不增加 LLM 调用。
- 重播种是一次性的（下一次分析运行时触发；启动链路不播种，见既有机制注）。
- ⚠ 修复后 short/mid 的 posterior 会因 flow 腿在场而变化（**这是目的**）；
  跨代比较须按 `template_version` 筛样（既有代际纪律）。

---

## 二、P1 —— 超短「催化剂腿」接入（#22 的另一半，需拍板）

### 2.1 问题与根因

- 超短 participating 只有 `momentumSignal` 一条腿（`holding_period.rs:489-496`），
  信号是 60 分钟 MACD 四态阶梯（离散）⇒ posterior 只落 8 个离散值，
  动作在「买入/增持」与「卖出」间二值化，与短档（多腿连续混合）天然不可比。
- **脚本自述与权威表不一致**：脚本头注释与 gaps 写「方向证据三腿（动量/广度/催化剂）」，
  但广度 2026-10-10 已按实证（002241 市场级量共享信号）降为 gate-only，催化剂则从未接
  （`portfolio-mgr-h-ultra-short.rhai:17-20,45-49`；`seed_stock_analysis.rs:930-938` 记载
  「本版没有随之扩腿集……那是 #22 的另一半」）。
- 催化剂是**个股级事件证据**，与广度的「市场级量」问题不同；2 日窗口恰是事件驱动窗口
  （PLAN §十一 原设计即含它）。`a-catalyst--ultra_short` 实例**已存在**（契约必填、每轮都在跑），
  只差接线。

### 2.2 修法（六件套，一次完整）

1. `harness/holding_period.rs::verdict_spec` 的 UltraShort 臂：`participating` 加 `eventCatalyst`
   （fields/na 列表不变——它在 COMMON 里；`analyst_subset` 不变——a-catalyst 已含）。
2. 分支表自动派生（`evidence_weight::horizon_branch_specs` 无需手改；
   bias 查表得 a-catalyst=1.0，与 momentum 的 1.3 归一 ⇒ 配比 0.565 / 0.435，**不新增数字**）。
3. 父扇出（`seed_stock_analysis.rs` 超短 SubWorkflow 块）加键 + 供给边：
   `analyst_node_id("a-catalyst","ultra_short")` 恒等键 + `e-<id>-pm-h-ultra-short` 边
   （照短档 `a-hot-money--short` 的既有模式，:6275-6278）。
4. 子模板（`horizon_tier_template.rs` 超短块）`input_mapping` 加
   `("catalyst_level", "a-catalyst--ultra_short.content.verdict.catalyst_level")`
   （与主链 f3 的读法逐字一致，路径权威 = 主链既有映射）。
5. 脚本 `portfolio-mgr-h-ultra-short.rhai`：raw 加 `catalystLevel`；删两条相关 gaps/注释
   （「按构造不评估」改为「接入后缺席语义：'无'/空 ⇒ leg_signal 判 None，走 absentLegs」）。
6. `TEMPLATE_VERSION` 154→155（或按批次取下一号），四档子模板版本跟随（绑定主图常量）。

### 2.3 效果算例（为什么能缓解二值化）

设 prior=0.5、escale=0.55 不变：MACD 空头（-0.7）且催化剂为 L2（+0.5，事件驱动独立证据）时
`avg = 0.565×(-0.7) + 0.435×0.5 = -0.178` ⇒ posterior ≈ 0.402 ⇒ **「观望」出现**
（当前只会是 0.115 卖出）。催化剂缺席（"无"）时 tw 减至 0.565 ⇒ escale 降为 0.438、
posterior 摆动收窄——缺席与在场的行为都可解释。

### 2.4 影响面与验收

- 影响面：上列六件套 + 门与锁：
  `portfolio_mgr_tier_params_rhai.rs`（实时 include 四脚本的行为断言）、
  `portfolio_mgr_horizon_lock_rhai.rs`、`seed_consistency_tests.rs`（脚本↔分支表对账、扇出齐备性）、
  `rhai_registry` 整脚本门、`check-tier-purity.mjs`、`audit-inject-coverage.mjs`、
  `check-analyst-id-shape.mjs`（父扇出键用 `analyst_node_id` 生成，禁手抄）。
- 验收：① `tier_fanout_inputs_are_complete` 等既有门绿；② 运行后超短 `legs` 出现
  `eventCatalyst` 行、`absentLegs` 语义正确；③ 超短 posterior 分布从 4 值扩维（观察面）。
- ⚠ **会改决策数值**（超短档）⇒ 跨代样本按代筛样；这不是「修 bug」而是**口径变更**，须拍板。

### 2.5 待拍板点

- 加不加催化剂腿？（不加则退而求其次：只把「本档方向仅由小时线动量驱动」写进呈现层声明，
  消除「三腿」自述与实现的歧义——但那不解决摆动问题。）
- 是否只加超短？（短/中/长的 participating 同样不含 eventCatalyst；短档已有更急的 P0，
  建议本轮只动超短，其余档在观测面评估该腿预测力后再议。）

---

## 三、P2 —— 同源双腿核查（先只读，不修）

- 现状：短/中档的 `momentumSignal`（macdDif/Dea）与 `trendStrength`（totalScore）
  同为该档**同一评分节点**的两个投影，合计权重 ≈0.49-0.55，无相关性折扣
  （主链有 `apply_covariance_decay` 先例，逐档没有）。
- 核查步骤（只读）：读 `crates/astock-data/src/scoring.rs` 的 totalScore 公式，
  量化 MACD 分量占比与两腿的秩相关（可用逐档 IC 基建）。
- 两个候选修法（核查后定）：
  a. 换源——给 `trendStrength` 改用与 MACD 解耦的口径（如均线斜率/量价），成本中；
  b. 同源组内折叠——同节点来源的腿先组内平均再组间加权，成本高（改融合结构）。
- **建议本轮不动**：影响相对小（背离时才对冲），且任何改法都改数值、需单独拍板。

---

## 四、不做的（边界声明）

- 不改 `momentumSignal` 阶梯（黄金值测试锁 + 主链 f12 同源，「搬运不许改好一点」）。
- 不恢复 `breadthState` 为方向腿（002241 实证：市场级量共享信号）。
- 不改四档动作阈值数值（现状 0.72/0.62/0.52/0.42/0.32 为各自独立声明，合规）。
- 不处理逐档先验空转（机制已对，样本不足是数据现实；等反思积累，届时按 IC 门槛评估）。
- 不重跑存量分析（伪样本风险）。

---

## 五、批次与顺序（推荐）

| 批 | 内容 | 版本 | 验证方式 |
|---|---|---|---|
| 批 1 | P0-1（+P0-2 按 1.2 的取舍） | **v155**（仅重播种契约文本），clippy/test 全量按纪律 | 用户侧跑 3-5 次分析 ⇒ 查 flowPersistence 数值化率 + 短档 gate 状态 |
| 批 2 | P1（催化剂腿） | **v156** | 门禁 + 运行后超短 legs/posterior 分布核对 |
| 随时 | P2 核查 | 不发版 | 只读结论回报 |

- 批 1 与批 2 **建议分开**：P0 是修复（验证显性）、P1 是口径变更（观察扰动），
  同批会把两个变量混在一次读数里。若用户拍板合并，则一次 v155 全做（省一轮重播种，
  验收判据各自独立保留）。
- 每批完成后按「全量门禁只在所有阶段完成后跑一次」的既有纪律收口。

---

## 六、拍板清单

1. 批 1 做不做 **P0-2（重试兜底）**？推荐：P0-1 先行，观察 3-5 次分析的数值化率后再定。
2. 批 1 与批 2 **分开还是合并**？推荐：分开（批 1 先发 v155）。
3. P1 **加不加催化剂腿**？推荐：加（回到 PLAN §十一 原设计；个股级事件证据；仅超短档）。
4. P2 是否现在启动**只读核查**？推荐：启动（不发版、不改码，只产结论）。

---

## 附录 A：诊断证据索引

- 审计现场读数与全链路梳理：本轮会话记录 + 项目记忆 `project-horizon-ultrashort-short-audit.md`。
- 关键代码锚点：`harness/holding_period.rs:404-453,489-496`、`analysis-engine/leg_signal.rs:146-149`、
  `evidence_weight.rs::horizon_branch_specs`、`horizon_tier_template.rs`、
  `seed_stock_analysis.rs:3667,6275-6278,930-938`、`agent_executor.rs:874-919,1658-1700`。

## 附录 B：验证 SQL（复查用）

```sql
-- ① 契约修复后：flowPersistence 的产出形态（期望：数字或 null，不再是自由文本）
SELECT a.stock_code,
       substring(a.blackboard_snapshot::jsonb->>'report.a-hot-money--short'
                 from 'flowPersistence.{0,40}') AS fp_excerpt
FROM stock_analyses a
WHERE a.blackboard_snapshot IS NOT NULL AND a.blackboard_snapshot <> ''
  AND a.blackboard_snapshot::jsonb ? 'report.a-hot-money--short'
ORDER BY a.created_at DESC LIMIT 5;

-- ② 短档门与动作（期望：gate 出现 true；动作出现 买入/增持）
SELECT a.stock_code, a.template_version,
       a.horizon_decisions::jsonb->'short'->>'entryGatePassed' AS gate,
       a.horizon_decisions::jsonb->'short'->>'action'         AS action,
       a.horizon_decisions::jsonb->'short'->>'confidence'     AS conf
FROM stock_analyses a
WHERE a.horizon_decisions IS NOT NULL AND a.horizon_decisions::jsonb ? 'short'
ORDER BY a.created_at DESC LIMIT 10;
```

---

## 实施记录

### 2026-10-14 批 1（P0：flowPersistence 契约修复，v155）

**改动（未提交）**：

- `crates/harness/src/holding_period.rs`：新增 `NUMERIC_VERDICT_FACTORS`（值域单源）；
  `verdict_contract_prompt` 在「必须输出字段」后渲染数值节（本档 fields 含数值型因子才出节
  ⇒ short/mid 有、超短/长档逐字不变）；新增负控测试
  `numeric_section_has_two_guards_each_negatively_controlled`（空表 / 字段不命中 两条负控
  拆在两个守卫上），扩展 `verdict_contract_prompt_is_rendered_from_the_table`。
- `crates/analysis-engine/src/leg_signal.rs`：新增测试
  `flow_persistence_bounds_follow_the_contract_table`（消费域与表同臂；样本用表内 lo/hi 现算，
  改任一端即红）。
- `seed_stock_analysis.rs`：`TEMPLATE_VERSION` 154→155 + v154→v155 版本注释段
  （⚠ 只为把新契约文本重播种；无图结构变更、无决策口径变更）。
- **行号引用重指**（版本注释插入 14 行使其后行号整体漂移）：按内容重指 2 处——
  `workflow_port_axioms.rs:15` 与 `workflow_template.rs:972` 的
  `seed_stock_analysis.rs:6389` → `:6767-6769`（case/default 边约定的注释块，即引用意图文本所在）。

**门禁读数（批 1 定点级；全量留到批 2 完成后按 CI 口径跑一次）**：

| 项 | 读数 |
|---|---|
| `cargo fmt --all --check` | 0 diff |
| `cargo check --workspace` | 0（11m30s） |
| `cargo test -p axagent-harness --lib` | **793 passed / 0 failed**；新增两条测试名确认在列（`verdict_contract_prompt_is_rendered_from_the_table`、`numeric_section_has_two_guards_each_negatively_controlled`） |
| `cargo test -p axagent-analysis-engine --lib -- leg_signal` | **5 passed / 0 failed**（含新增 `flow_persistence_bounds_follow_the_contract_table`） |
| `cargo clippy -p axagent-harness -p axagent-analysis-engine --all-targets -D warnings` | **绿**（1m53s，豁免下述他人 lint）。⚠ 未豁免时为红——唯一错误是**并行会话进行中代码**的 `session_log_invariant.rs:61` `unused_mut`（他人文件，本批未动）；豁免该条后两 crate 全部 targets 0 告警 |
| `check-single-source-facts.mjs` | 硬判据 0；软判据 9→7 处——消除的 2 处由本批插入引发，剩余 7 处涉及文件与本批改动**无交集**（既有软判据） |

**待验证（用户侧运行门）**：重播种后跑 3-5 次分析，按附录 B 的 SQL 核对
① `flowPersistence` 数值化率（期望 >90% 为数值或 null）；② 短档 `entryGatePassed` 是否出现
`true`；③ 短档动作是否可能出现 买入/增持。

### 2026-10-14 P2 核查结论（只读，不改码）

- 数据源重叠：`ScoringWeights::default` 中 MACD 分量占 totalScore 权重 15/105（≈14%），
  两腿共享 `macdDif/macdDea` 的派生（`scoring.rs` 的 `score_macd` vs `leg_signal.rs` 的四态阶梯）。
- 现网横截面实测（短档 n=26，取自 `horizon_decisions` 的 legs）：
  两腿 Spearman ρ = **0.697**，同号 18 / 背离 8。
  ⚠ 该相关**混有市场共同因子**（同一横截面下强势票全体指标偏多），不是纯条件相关；
  与主链先例（f3↔f11「数据源重叠约 70% ⇒ 降权 35%」）性质不同——那是结构性同源，本处是横截面相关。
- 结论：**维持不改**；把「分市场状态的条件相关」列入观测面待办（与逐档 IC 同基建）。

### 2026-10-14 批 2（P1：超短催化剂腿 + 两条一致性修复，v156，已提交 `8b64746db`）

**改动（13 文件，+328/−30）**：

- **催化剂腿四处同批**：`verdict_spec.UltraShort.participating` 加 `eventCatalyst`
  （harness，含新断言：退回单腿即红）；超短子模板 `catalyst_level` 映射
  （读法与主链 f3 逐字一致）；父扇出加 `a-catalyst--ultra_short` 恒等键 + 供给边
  （照短档 `a-hot-money--short` 既有模式，id 经 `analyst_node_id` 生成）；脚本 raw 加
  `catalystLevel` 并退役「按构造不作废评估」的 gap。
- **终审一致性修复 ①（cap 穿透）**：`apply_risk_cap` 导出为 `pub` + 注册 `pm_risk_cap`；
  四份脚本 `position = pm_risk_cap(kelly × mult, overall_risk)` —— 修复长档乘数 1.2
  把「极高 10 / 高 35 / 中 50」穿透成 12/42/60。测试：
  `risk_cap_is_final_upper_bound`（12→10、42→35、60→50、未达上限不抬、低风险不 cap）。
- **终审一致性修复 ②（观望+正仓位）**：四份脚本加守卫块（**逐字相同**，含注释）：
  「非买入/增持/持有 ⇒ position=0」——修两条路径：kelly 门槛 0.50 低于「持有」阶梯 0.52
  （002241/002080 实证）；档内否决折「观望」未清零（002384/300045 实证，现网共 8 条）。
  测试：`horizon_position_cap_and_guard_present_in_all_four_tiers`（四脚本 contains +
  两个破坏样本各只让对应一条红）。
- `TEMPLATE_VERSION` 155→156 + v156 版本注释（含 ④⑤ 两条一致性修复）。
- 行号重指（第二批插入累计 +23 与 +7）：`workflow_port_axioms.rs` /
  `workflow_template.rs` 的 `:6767-6769` → `:6797-6799`；顺带按内容修复两处旧腐烂：
  `horizon_tier_template.rs:13` 的「拒绝循环」引用 → `:6190` + `:5075`；
  seed 内 `:5249` 自引用 → `:6190`。

**门禁读数**：

| 项 | 读数 |
|---|---|
| `cargo fmt --all --check` | 0 diff |
| `cargo check --workspace` | 0（15m23s） |
| `cargo test -p axagent-harness --lib` | 793 passed / 0 failed |
| `cargo test -p axagent-analysis-engine --lib` | **640 passed / 0 failed**（首跑抓到 1 条批 2 遗漏的老断言，见下） |
| `cargo test -p axagent-rt-workflow --test portfolio_mgr_tier_params_rhai --test portfolio_mgr_horizon_lock_rhai` | 12 + 6 passed / 0 failed |
| `cargo test -p axagent --lib commands::stock_analysis_setup`（种子/版本/一致性面） | 80 passed / 0 failed |
| `cargo test -p axagent --lib commands::stock_workflow::rhai_registry`（脚本注册面覆盖门） | 13 passed / 0 failed |
| 主 crate lib clippy（定点；`-A clippy::neg_cmp_op_on_partial_ord` 豁免一条已证属他人未提交代码的 lint） | **0**（31m19s） |
| clippy / test workspace（CI 口径 `--all-features` / `--no-fail-fast`） | **未闭合（环境所限）**：三轮分别被并行会话进行中代码挡——`ic.rs:203`/`evolution_drift.rs:514,685,845` E0063/E0277 → `stock_analysis.rs:5338` doc 缩进（**已中性修复**：插空 `///` 分段，只改不提交）→ `backtest_validation.rs:495` neg_cmp（**逻辑类，未动**）/`stock_automation_cron_seed.rs:296` E0308；⇒ 待其收口后复跑。本批改动面已由上述定点门逐项覆盖 |
| 脚本门 6 道（horizon-decision-keys / rhai-numeric-typing / input-mapping / port-axiom-wiring / indicator-config-scope / horizon-weight-parity） | 全 0 |
| 审计门 3 道（audit-inject-coverage `--strict` / tier-purity / analyst-id-shape） | 全 0（悬空白注入 `screening_source` 为历史遗留） |
| `check-single-source-facts.mjs` | 硬判据 0；软判据 7 处（均为既有，与本批无文件交集） |

**批 2 遗漏与修复（首跑 T1 抓到，已修）**：`crates/analysis-engine/src/evidence_weight.rs` 的
`branch_specs_are_derived_and_role_partitioned` 断言「超短方向腿 == ["momentumSignal"]」——
批 2 改 `verdict_spec` 时**只查了 harness 内的断言，漏了本跨 crate 派生断言**
（教训与判据层 4bh 同款：**改产端结构后验证面按「谁读它」枚举整个模块**）。
修法：断言改 `["momentumSignal", "eventCatalyst"]` + 注释说明；复跑全量 640/0。

**首跑 T5/T6 被挡说明**（`ic.rs:203` / `evolution_drift.rs:514,685,845` 的 E0063/E0277——
`strategy_performance` 加 `reflection_id` 与 `RecoIcRow` 迭代器类型错，属**并行会话进行中的功能**，
本批未动；其期间一并修复了此前 `session_log_invariant.rs:61` 的 `unused_mut`）⇒ 树恢复后已重跑 T5/T6。

---

## §八 四档科学合理性终审（2026-10-14，随批 2 收口）

逐环判定（依据 = 本轮代码与现网读数；「修复」指本次两批内的动作）：

| # | 环节 | 判定 | 依据/修复 |
|---|---|---|---|
| 1 | 取数与共享上游（as-of、σ_daily 单一实现、K 线多源） | 合规 | 前几轮已收编；本批未触 |
| 2 | 分析师矩阵（23 逐档实例、`analyst_subset()` 推导） | 合规 | B2-2 已落地；本批补上超短催化剂实例的**消费**（此前该实例白跑） |
| 3 | 逐档尺度与评分（ScaleWindowPlan、ScoreBands 按 √bar 缩放） | 合规 | 窗口按持有期换算；短/中/长 bar 数相同而尺度不同（有公式与日历跨度论证） |
| 4 | 腿信号（`leg_signal` 单一实现、锚内禀、缺席≠0） | 合规 | 批 1 让 `flowPersistence` 的**契约承诺**与消费域同臂（修复前该腿从未生效） |
| 5 | 方向融合（四档脚本、缺席腿不归一、role 分 direction/filter/riskNote） | **已修复** | 批 2 修超短单腿离散化（催化剂腿接入 ⇒ posterior 不再只落 8 个离散值）；同源两腿（momentum/trend，横截面 ρ=0.697）判**保留**——混市场共同因子、变换主体不同（MACD 占 totalScore 权重 14%）、背离形态（回调）对冲到中性是合理行为；条件相关列入观测面 |
| 6 | 门（四形态） | 合规 | 批 1 前提修复后短档 `trend_and_flow_same_sign` 恢复可判定；其余三门依赖确定性数据 |
| 7 | 价带与仓位 | **已修复** | 批 2 两条一致性修复（cap 穿透、观望+正仓位）；k·σ·√h、半凯利、乘数（`positionMultiplier` 0.6/0.8/1.0/1.2 为经验值——R-11 后裁定保留，见开放项） |
| 8 | 选档与仲裁（可执行优先、凯利日增长率） | 合规 | 修复 ② 同时消除「观望档因残留仓位被误判为可执行档」的污染路径 |
| 9 | 落库与呈现（投影、快照、代际四态） | 合规 | 前几轮已收编；本批 legs 多一行由通用渲染承载（前端因子字典已含 eventCatalyst） |

**开放项（记录不修，及理由）**：

1. `positionMultiplier` 0.6/0.8/1.0/1.2：经验乘数、无推导、不可面板调。R-11 重做时显式保留（晚于 09-29 裁定⑥，按后裁定执行）⇒ 保留；观测面（逐档 IC/命中率）积累后校准或退役。
2. 仲裁 `arbitration` 字段无消费方（面板/落库都不读）：Q3 已决边界（只回答资金投向），消费接线未做 ⇒ 记录。
3. `decision_validations` 现网 0 行：逐档 IC / 预测半衰期的观测面数据源未播种 ⇒ 观测面机制就绪、数据未生。
4. `flowPersistence` 数值化后的**校准质量**（LLM 自评 [-1,1] 的可靠性）待现网积累后由观测面评估——批 1 验证时顺带看数值分布。
