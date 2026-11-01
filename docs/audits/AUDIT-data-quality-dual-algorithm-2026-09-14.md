# 数据质量双算法审计与统一（2026-09-14）

> 触发问题：股票分析工作流「决策部分引用的数据质量」为什么等级恒为 **C**，而「分析师节点数据质量检查」等级却恒为 **A**？
> 结论：**两处根本不是同一套算法**。前者是确定性 Rhai 节点的三因子加权；后者是前端弹窗从 100 起扣分的结构校验，且**一个字的报告正文都不读**。本次已删除第二套算法，并把全项目四套 dqi 分档阈值收敛为一份。

---

## 一、两套算法对照（审计基线）

| 维度 | 决策侧（工作流内） | 分析师弹窗侧（前端） |
|---|---|---|
| 实现位置 | `src-tauri/src/commands/data-quality.rhai`（CodeNode） | `src/components/stock-analysis/AnalystDataQualityModal.tsx` 的 `analyzeDataQuality()`（**已删除**） |
| 输入 | 10 份分析师报告**正文** + 9 个数据因子 + 工具可信度 | 仅**单个节点**输出的 JSON 结构 |
| 公式 | `报告均值×0.35 + 工具可信度×0.35 + 因子完整度×100×0.30` | 从 100 起逐项扣分 |
| 等级阈值 | **A≥85 / B≥65 / C≥45 / D≥25** | **A≥90 / B≥70 / C≥50 / D≥30** |
| 报告正文检测 | 有（20 个失败标记词，`placeholder_hits`） | **无** |
| 输出消费者 | `portfolio-mgr` 的 f6 因子、`quality-gate` 路由 | 弹窗 UI + 上报 |

**方法论差异不是「参数不同」，是「测的对象不同」**：前者测「这批证据能不能信」，后者测「这个 JSON 长得规不规范」。

---

## 二、C 与 A 的成因（逐条取证）

### 2.1 决策侧恒 C：`report_quality` 不满量程

`report_quality` 各维度上限合计仅 **80**（字数 40 + 关键词 30 + 一致性奖励 10），导致
`report_quality_avg ≤ 80`，代入加权公式后**综合分天花板被钉在 93**，A 级只剩 8 分余量。
叠加 `tool_credibility` 的多重惩罚（单个分析师 `missing` 即扣该项分），
实际落到 45–65 区间 ⇒ 恒 **C**。

### 2.2 弹窗侧恒 A：空分支不扣分 + 零正文检测

`analyzeDataQuality()` 只需字段齐全、类型正确、数值落在合法区间即可满分；
报告正文写成「数据缺失，无法获取」照样得 90+。⇒ 恒 **A**。

### 2.3 最尖锐后果：同一分数被四套阈值判成不同等级

| 判定点 | 位置 | 阈值 |
|---|---|---|
| ① 权威分级 | `data-quality.rhai` `grade()` | 85 / 65 / 45 / 25 |
| ② 前端弹窗（已删） | `AnalystDataQualityModal.tsx` | 90 / 70 / 50 / 30 |
| ③ 置信度上限 | `portfolio-mgr.rhai` `confidence_quality_cap` | 90 / 75 / 60 |
| ④ 仓位档位 | `portfolio-mgr.rhai` `dqi_a_level` / `dqi_high_quality` | 85 / 65 |

`grade="C"`（45 ≤ score < 65）的标的，在 ③ 里被判成 D 级档位拿 60 上限，在 ④ 里拿不到任何质量加分。
**用户界面上写着「C 级」，引擎内部按「D 级」限流** —— 这正是「UI 显示与决策行为对不上」的根因。

---

## 三、已执行的修复

### 3.1 收敛为单一权威（核心）

- **新增输入映射**：`portfolio-mgr` 的 `input_mapping` 增加 `("dqi_grade", "data-quality.result.grade")`
  （`seed_stock_analysis.rs:3002`）。`dqi_score` 继续传递 —— Rhai 的 f6_signal 与提示文本仍需连续值。
- **`portfolio-mgr.rhai` 四处分档改读 grade 字符串**：
  `confidence_quality_cap`（A→99.9 / B→85 / C→75 / D、F→60）、
  `dqi_collapsed`（F）、`dqi_a_level`（A 且 untrusted_count==0）、`dqi_high_quality`（A∪B）。
  局部变量名用 `dqi_grade_str`，避免与上游注入的其他绑定名冲突。
- **展示文案去数值阈值**：两处 `dqi_score=...<25` 改为 `grade=${dqi_grade_str}`（原行 1548、1773）。
- **模板版本门**：`TEMPLATE_VERSION` **41 → 42**（`seed_stock_analysis.rs`），否则已有 DB 不会重新种子化。
- **阈值存量**：全项目 dqi 分档阈值现只剩 `data-quality.rhai` 一份。

### 3.2 修 `report_quality` 满量程

维度重划为：字数 **50**（≥500 字满分）+ 关键词覆盖 **40**（5 组 × 8）+ 占位符扣分 **−15** + 一致性奖励 **10** = 满量程 100。
这消掉了「综合分天花板 93」这一结构性上限。

### 3.3 修「假修复」缺陷

`data-quality.rhai` 的 `validate_and_fix_verdict_schema` 内部构造 `fixed_data` 做「自动归一化修复」，
但唯一调用方 `validate_verdict_schema` **只取 `issues`，`fixed_data` 被直接丢弃**
（铁律 9「声明的机制 ≠ 运行的机制」）。
已重命名并简化为 **`validate_verdict_schema`（纯检测）**，删除 `fixed_data`、`effective_bs` / `effective_bes`
及「保留旧接口以兼容」的包装函数。

### 3.4 修 fail-open

`confidence_quality_cap` 的 `else` 分支（`dqi_score` 缺失）原返回 **99.9** ——
而 `data-quality` 节点本身 `continue_on_fail=true` 允许失败，**失败反而拿到满额置信度上限**。
已改为最保守的 **60**（grade 缺失/非法一律 60），与 `dqi_grade_str == ""` 合并处理。

### 3.5 删掉前端第二套算法

- 新建 `src/lib/dataQualityDiagnosis.ts`：
  - `EXPERT_ID_TO_DQ_ABBR`：10 个 `a-*` 专家 → `data-quality.rhai` 的 diag 缩写（`mk`/`sent`/`news`/`fund`/`pol`/`hm`/`lk`/`res`/`sec`/`cat`）
  - `parseDataQualityReport(raw)`：四类返回 `null`（空/非法 JSON、缺 `grade`、`stale_record===true`、缺 `diagnostics`）
  - `resolveAnalystDiagnosis(expertId, report, expertName?)`：先按缩写映射，再按 `diag.name === expertName` 兜底
- `AnalystDataQualityModal.tsx` **完全重写**（1031 → ~420 行）：
  删除 `analyzeDataQuality()` / `getExpectedVerdictFields()` / `scoreToGrade()`（90/70/50/30）/ `ParsedReport` 巨型接口。
  Props 移除 `parsed`、`report`，改为从 store 读 `dataQualitySummary` 并解析。
  新增诊断表四行：节点状态 / 报告失败标记 / 预期数据来源 / 归因。
  上报指标改为消费权威输出（`dqi_grade` / `dqi_score` / `report_quality_score` / `tool_credibility_score` /
  `factor_completeness_pct` + 本节点 `status` / `confidence` / `placeholder_hits`）。
- `AnalystReportCard.tsx`：两处调用移除内联构造的 `parsed` / `report`（含 VERDICT tag 解析、正则提取
  bull_score/bear_score 的兜底逻辑），共删约 96 行。

### 3.6 注释与文档一致性

| 文件 | 问题 | 处置 |
|---|---|---|
| `portfolio-mgr.rhai` | 注释称 `factor_completeness×0.20`，实际 **0.30**；引用的行号 299 早已失效 | 已更正 |
| `data-quality-inspector.md` | 写「报告质量×0.6 + 工具可信度×0.4」、阈值 90/75/50/25，与 Rhai 的 0.35/0.35/0.30、85/65/45/25 冲突 | 顶部插入「⚠ 口径对照（2026-09-14）」表，声明排障一律以 Rhai 为准 |
| `portfolio-manager.md` | 第 113、214 行称 dqi 来自 `data-quality-inspector.score` | 改为 `data-quality` 节点 `score`，并注明分级权威是 `grade`、**禁止用数值自行分档** |

### 3.7 i18n

11 个语言包各新增 **18 个 key**（锚点 `dataQualityNoScore`）：`dqDimensionReport`、`dqDimensionTool`、
`dqDimensionFactor`、`dqScoreSource`、`dqNodeDiagnosis`、`dqNoNodeDiagnosis`、`dqFieldNodeStatus`、
`dqFieldPlaceholder`、`dqFieldExpected`、`dqFieldGapReason`、`dqStatusNormal` / `Low` / `Missing` / `Untrusted`、
`dqPlaceholderNone`、`dqPlaceholderCount`、`dqGapReasonNone`、`dqDirectionConflict`。

---

## 四、验证

| 项 | 命令 | 结果 |
|---|---|---|
| TS 类型 | `npm run typecheck`（tsc --noEmit） | **exitcode=0** |
| 格式 | `dprint check`（3 个改动 TS 文件） | **exitcode=0** |
| Lint | `oxlint --max-warnings=0`（同上） | **0 warnings, 0 errors** |
| i18n | Python 逐文件 JSON 解析 + key 存在性断言 | 11/11 合法，18 key 全在 |
| Rust | `cargo check --workspace`（第一轮） | **exitcode=0**（`Finished dev profile in 12m 15s`，含 `axagent` / `axagent-runtime` / `axagent-mobile` 重编） |
| Rust 格式 | `cargo fmt -- --check` | **exitcode=0 / 0 处差异**（修平 8 文件 28 处既有差异后） |
| Rust | `cargo check --workspace`（第二轮·注销注册后） | **exitcode=0**（`Finished dev profile in 23m 34s`，含 `axagent` 主 crate + `axagent-mobile` 重编，**0 error / 0 warning**） |
| 运行期行为 | 需一次真实工作流跑批对照 `grade` 与 `confidence_quality_cap` | **未做**（建议在 601166 或 600031 上复跑） |

> Rust 侧备注：**两轮 `cargo check --workspace` 均已 exitcode=0**。第二轮改动涉及
> `stock_analysis_setup/mod.rs`（新增 `RETIRED_EXPERT_IDS` + 两处 `delete_by_id` 调用）、
> `analysis-engine/src/prompts.rs`（`EXPERT_IDS` 删项）、`data-quality.rhai` 与 `seed_stock_analysis.rs`（注释）；
> 其中 `EMBEDDED_PROMPTS` 集合变化触发了 `axagent` 重编 —— 已跑完并**零 error / 零 warning**
> （耗时 23m 34s，同期另有 4 个并发 cargo 会话争用构建锁）。
> 运行期行为对照仍缺，建议在 601166 或 600031 复跑一次真实工作流，确认
> ① 专家列表不再出现 `data-quality-inspector`；② `grade` 与 `confidence_quality_cap` 口径一致。

---

## 五、顺带发现与裁决执行

### 5.1 `agency_experts/` 下存在 `include_str!` 白名单之外的文件 —— 判「零引用 ⇒ 可删」必须核对**运行期目录扫描的深度**

比对结果：目录下 42 个 `.md`，`mod.rs` 用 `include_str!` 显式引用 39 个，差集 3 个：

| 文件 | 是否可达 | 说明 |
|---|---|---|
| `portfolio-manager.md` | **可达** | 位于 `agency_experts/<子目录>/` 一层内，可被 `import_experts_impl` 目录扫描导入 |
| `policy-chain.md` | **可达** | 同上，属可导入专家，但未接入任何模板 |
| `custom/example-trend-follower.md` | 不可达 | 位于**二层**子目录；`import_experts_impl`（`agency_expert.rs:332-356`）只下钻一层 |

**检法教训**：`src-tauri/src/commands/agency_expert.rs:315` 的 `import_experts_impl` 从**用户传入的路径**
（`ExpertSelector.tsx:135`）扫描 `.md` 并 UPSERT 到 `agency_experts` 表。
判「零引用 ⇒ 可删」时，传递闭包**必须包含「运行期目录扫描」这条边，并核对扫描深度**
（铁律 24「无引用 ≠ 可删」的新形态）。

> **同日后续（裁决）**：初判曾据此**放弃删除** `portfolio-manager.md`，只修其过期引用。
> 但它与 `policy-chain.md` 只是「用户手动传路径时才可达」的**潜在**可导入素材、并非代码依赖，
> 用户最终裁决**归档删除**二者（副本存 `output/backup-2026-09-14/archived-experts/`）。
> 两次结论不冲突：判定当时「不可删」是正确的，删除是**知情的产品决策**，
> 而非「确认无引用之后的清理」—— 这条界线值得记住。

### 5.2 `cargo fmt --check` 在 8 个**未被本次改动触及**的文件上失败（28 处）—— 已按裁决修平

`cargo fmt -- --check` 退出码 1，共 **28 处**差异，全部与本次改动无关
（本次唯一的 `.rs` 改动 `seed_stock_analysis.rs` **0 处命中**）：

| 文件 | 处数 |
|---|---|
| `src/commands/stock_workflow/reflection.rs` | 9 |
| `src/commands/stock_analysis.rs` | 6 |
| `src/commands/stock_workflow/decision.rs` | 6 |
| `src/commands/pool_scan.rs` | 3 |
| `crates/analysis-engine/src/recommender/mod.rs`（`pub use` 排序） | 1 |
| `src/commands/stock_analysis_setup/mod.rs` | 1 |
| `src/commands/stock_workflow/hooks.rs` | 1 |
| `src/commands/recommendation_cron.rs` | 1 |

差异形态均为 rustfmt 的既有风格（`is_empty()` 单行 `if/else` 展开、`format!` 换行位置、
`pub use` 字母序）。**AGENTS.md 声明 `cargo fmt` 是 CI 强制项 ⇒ 修平前 CI 的 fmt 门禁为红。**

**处置（裁决：全仓修平）**：执行 `cargo fmt` 后复检 `cargo fmt -- --check`
**exitcode 0 / 0 处差异**，fmt 门禁恢复绿灯。改动前基线备份于
`output/backup-2026-09-14/fmt-baseline/`。

> 本节初稿曾记为「9 文件 / 26 处」—— 那是正则解析失败的产物
> （`Diff in <path>:<line>:` 的路径段被并入文件名统计，按文件计数遂虚高）。
> 准确值以本节表格为准：**8 文件 / 28 处**。

### 5.3 裁决结果与执行（2026-09-14）

| 裁决项 | 裁决 | 执行结果 |
|---|---|---|
| fmt 既有差异（8 文件 / 28 处） | **全仓修平** | 复检 0 差异，CI fmt 门禁转绿 |
| `data-quality-inspector` 的注册 | **移除注册** | 三处注册点全清 + DB 残留定向清理（见 5.4） |
| `policy-chain.md` / `portfolio-manager.md` | **归档删除** | 已删除，副本存 `output/backup-2026-09-14/archived-experts/` |
| `AgentProfileList.tsx` 的 5 处 `stock-portfolio-manager` | **清掉** | 五个平行 map 各删 1 处，key 集合收敛为 16（见 5.5） |
| 5.5 顺手发现的 `agency-${pid}` 拼接断链 | **顺手修**（纯 bug，且有正对照） | 4 处改为 `agency-stock-analysis-${pid.replace(/^stock-/, "")}`（见 5.5） |

### 5.4 移除 `data-quality-inspector` 注册：**只删三个数组落不了地，还有 DB 残留**

三处注册点（均在 `stock_analysis_setup/mod.rs`）已全部清除：`EMBEDDED_PROMPTS`（`include_str!` 定义）、
`EXPERT_ROLE_MAP`（`("data-quality-inspector", "stock-analyst")`）、
`PROFILE_TOOLS`（`("data-quality-inspector", &["search_stock"])`）。

**⚠ 关键陷阱**：`seed_agency_experts` / `seed_agent_profiles` 只有 **UPSERT**（有则 `update`、
无则 `insert`），**从不删除** ⇒ 已装环境的 DB 里 `agency_experts.id =
"agency-stock-analysis-data-quality-inspector"` 与 `agent_profiles.id = "stock-data-quality-inspector"`
两行会**残留**；而前端按 `source_dir = "stock-analysis"` 聚合专家列表 ⇒ 症状仍是
「**界面里还能选，但它已不参与工作流**」，与裁决初衷正相反。这是「声明层改了、数据层没跟上」，
属铁律 9 的「出口不严型」。

修法：新增退役清单 + 定向删除（幂等；失败仅 `tracing::warn`，不阻断启动）：

```rust
/// 已注销的专家 id。…（完整说明见源码注释）
const RETIRED_EXPERT_IDS: &[&str] = &["data-quality-inspector"];
```

```rust
for retired in RETIRED_EXPERT_IDS {
    // seed 专用命名空间 —— 避免误删用户手动导入的专家
    let agency_id = format!("agency-stock-analysis-{retired}");
    if let Err(e) = agency_experts::Entity::delete_by_id(&agency_id).exec(db).await { /* warn */ }
}
```

> 为什么不用「删掉所有不在 `EMBEDDED_PROMPTS` 里的 stock-analysis 行」？那会误删用户手动导入的专家。
> `agency-stock-analysis-` 与 `stock-` 是 seed 专用前缀，**锁定命名空间**才是安全的精确解。

**附带同步的 4 处**：
- `crates/analysis-engine/src/prompts.rs`：`EXPERT_IDS` 移除该项。该常量注释原写「**22 个专家 ID
  （对应 22 个 .md 文件）**」而实列 25 项、又不含 Serenity 链条与新增专家；且全项目 grep 确认
  **只有定义处命中（零消费者）**，与 `EMBEDDED_PROMPTS` 重复维护（违反 AGENTS.md 禁区 12）⇒
  改标注为「已过期且零消费者」。
- `data-quality.rhai:519`：原注释引「data-quality-inspector.md 职责第 3 条要求…」⇒ 改为声明该专家
  已注销注册、其 `.md` 仅作方法论文档保留。
- `stock_analysis_setup/mod.rs:364`：原注释「提升到 14 以容纳 11 个 a-* + value-investor +
  data-quality-inspector + catalyst-analyst + social-media-analyst + volume-price-analyst（共 14 个）」
  —— **数字本身就不自洽**（实列 16）⇒ 改为不含具体数字的描述。
- `seed_stock_analysis.rs:2967`：原注释「公式逻辑（与 portfolio-manager prompt 保持一致）」指向已删的
  md ⇒ 改指真实实现 `portfolio-mgr.rhai`。

### 5.5 清掉前端孤儿 profile 映射 → **牵出两个更大的问题**

**执行**：`src/components/settings/AgentProfileList.tsx` 删除 5 处 `stock-portfolio-manager`
（`PROFILE_NAMES` / `PROFILE_ROLES` / `PROFILE_ROLE_IDS` / `PROFILE_NAME_KEYS` / `PROFILE_ROLE_KEYS` 各 1 处）。

**先修正一个轻率判定**：它**不是**「按 pid 查表查不到」的死键 —— 查不到有 `?? pid` 兜底，不会显示错。
真正的问题是 **L232 / L330 的离线回退用 `Object.keys(PROFILE_NAMES)` 驱动渲染**
⇒ 这个孤儿键会**凭空生成一行幽灵节点**（17 → 16 行）。
判「多一条键无害」之前，必须先确认该 map 是否被 `keys` 驱动。

**删除依据**（`stock-analysis` 模板 profile id 全集实测 **23 个**，其中无 `stock-portfolio-manager`）：
- 主图 Agent 节点由闭包生成：`agent_profile_id = Some(format!("stock-{expert_id}"))`
  （`seed_stock_analysis.rs:1318`）
- `portfolio-mgr` 是 **CodeNode（Rhai）**，非 Agent 节点 ⇒ 无 profile id，与本表无关
- 后端 `EMBEDDED_PROMPTS` / `EXPERT_ROLE_MAP` / `PROFILE_TOOLS` 三处注册也均无该 id
- **验证**：五表 key 集合**完全一致**（各 16 个，双向差集均为空）

#### ⚠ 新发现 A · 前端拼错 id 前缀 ⇒ `expertMap` 恒 miss（**影响全部 23 行，不止被删的那 1 行**）

| 侧 | 表达式 | 实测值（`expert_id = value-investor`） |
|---|---|---|
| 后端 seed | `format!("agency-stock-analysis-{expert_id}")`（`stock_analysis_setup/mod.rs:784`） | `agency-stock-analysis-value-investor` |
| 后端导入器 | `format!("agency-{}-{}", dir_name, file_stem)`（`agency_expert.rs:399`，`dir_name="stock-analysis"`） | 同上 ✓ |
| **前端** | `` `agency-${pid}` ``（`AgentProfileList.tsx` 4 处：L196 / 232 / 293 / 330） | `agency-stock-value-investor` ✗ **少 `analysis-` 段** |

`expertMap` 的 key = `list_agency_experts` 返回的 **DB 原始 id**（`agency_expert.rs:498` `id: m.id`）
⇒ `expertMap[row.expertId]` **恒 `undefined`** ⇒ 专家名走 `t(PROFILE_NAME_KEYS[...] ?? expertName)` 兜底、
prompt 区显示「无 prompt」（L453 / 460 / 492）。

已排除的其它写入点：`workflow_template.rs:1782`（n8n 导入，`source_dir="n8n-import"`）、
`opc_setup/*`（OPC 域，另一套命名）、`capability.rs:582` 与 `agency_expert.rs:805`（均为 update，沿用已有 id）。

#### ⚠ 新发现 B · 7 个反向缺口（后端有、前端 `PROFILE_NAMES` 无）⇒ 显示原始英文 id

- `stock-catalyst-analyst` —— 后端 `analysts` 数组实测 **10 个**，前端只列 9 个（漏了 `a-catalyst`）
- `stock-value-investor` / `stock-rule-checker` / `stock-quality-fallback` / `stock-explainer` /
  `stock-debate-convergence` / `stock-risk-convergence`
- 另：i18n `stockAnalysis.analystRoles` 缺 `catalystAnalyst`

> 反向（幽灵向）差集已清空 = 0。**只算一个方向是常见错误** —— 只看到「前端比后端少」就去补，
> 会漏掉同时存在的「前端比后端多」。

#### ✅ 已修 · 新发现 A（拿到正对照后判定为确凿 bug，顺手修掉）

**决定性正对照**：同一项目另一个组件用的正是正确前缀 ——
`ExpertPromptList.tsx:40` `r.id.startsWith("agency-stock-analysis-")`、
`:108` `expert.id.replace("agency-stock-analysis-", "")`；
而 `agency-stock-[非a]` 在**全项目（含 `.worktrees/`）0 命中**
⇒ 历史上**从未存在** `agency-stock-xxx` 格式 ⇒ 这是**确凿的纯 bug**
（铁律 19「同一语义两套口径」的形态），不是设计选择。

```ts
// 4 处统一（L196 / L232 / L293 / L330）
expertId: `agency-stock-analysis-${pid.replace(/^stock-/, "")}`,
```

拼接等价性断言（5 个 pid 取值全部 `OK`）：`stock-value-investor` → `agency-stock-analysis-value-investor` ✓

#### ⚠ 新发现 C · `explainer` 节点三处缺注册 ⇒ **静默降级**（只 WARN，不失败）

| 层 | 状态 |
|---|---|
| md 文件 | `agency_experts/stock-analysis/explainer.md` **不存在**（目录 38 个 md 中无它） |
| `EMBEDDED_PROMPTS` | 38 条，**无 `explainer`** |
| `EXPERT_ROLE_MAP` | 38 条，与上表**一一对应**，同样**无 `explainer`** |
| 模板节点 | **存在**：`decision-explainer`，`agent_profile_id = "stock-explainer"`（`seed_stock_analysis.rs:3550`）⇒ **指向不存在的 profile** |

运行期后果（`crates/rt-workflow/src/work_engine/executors/agent_executor.rs:605`）：
`Ok(None)` 分支**只打 `tracing::warn!`**（"Expert 不存在（expert_id 无效或已删除），expert 提示词不会生效"）
**并跳过**，节点**不失败**（该节点另有 `continue_on_fail = true`）⇒ 靠 inline `system_prompt` 硬撑，
丢失「决策解释专家」方法论。属**静默降级**，不是断链。

**B（7 个反向缺口）的危害已因修 A 而收窄**：`expertMap` 现在能命中 ⇒ 其中 6 个（md 有注册）
显示正常中文名；**仅 `explainer` 真正缺兜底**。但离线回退仍只生成 16 行（真实 23 个）⇒ **少 7 行**。

**待裁决**：B / C 的补注册（C 需新写专家 prompt 内容）。

**本轮验证**：`dprint check` **EXIT=0** ｜ `oxlint --max-warnings=0` **0 warning / 0 error** ｜
`npx tsc --noEmit` **EXIT=0**（A 修复后复跑）
**备份**：`output/backup-2026-09-14/orphan-profile-map/`（删映射前）、
`output/backup-2026-09-14/expert-id-fix/`（改 id 前）


---

## 六、改动文件清单

**Rust 侧**
- `src-tauri/src/commands/data-quality.rhai`（`report_quality` 满量程；`validate_and_fix_verdict_schema` → `validate_verdict_schema` 纯检测）
- `src-tauri/src/commands/portfolio-mgr.rhai`（四处分档改读 `grade`；fail-open 99.9→60；两处展示文案；注释 0.20→0.30）
- `src-tauri/src/commands/stock_analysis_setup/seed_stock_analysis.rs`（新增 `dqi_grade` 映射；`TEMPLATE_VERSION` 41→42；变更说明注释）
- `src-tauri/src/commands/stock_analysis_setup/mod.rs`（新增 `RETIRED_EXPERT_IDS` + 两处定向删除；移除三处注册；修 `max_concurrent` 过期注释）
- `src-tauri/crates/analysis-engine/src/prompts.rs`（`EXPERT_IDS` 移除该项 + 注释标注「已过期且零消费者」）

**前端**
- `src/lib/dataQualityDiagnosis.ts`（新建）
- `src/components/stock-analysis/AnalystDataQualityModal.tsx`（完全重写）
- `src/components/stock-analysis/AnalystReportCard.tsx`（两处调用精简）
- `src/components/settings/AgentProfileList.tsx`（删除 5 处 `stock-portfolio-manager` 孤儿映射，五表 key 集合收敛为 16）
- `src/i18n/locales/*.json`（11 文件 × 18 key）

**专家 prompt / 文档**
- `src-tauri/agency_experts/stock-analysis/data-quality-inspector.md`（口径对照表；**文件保留**，仅注销注册）
- `portfolio-manager.md` / `policy-chain.md`（**已归档删除**，副本见下方「备份 / 归档」）

**注册 / 注销机制**
- `src-tauri/src/commands/stock_analysis_setup/mod.rs`：新增 `RETIRED_EXPERT_IDS` + 两处定向删除；移除 `data-quality-inspector` 的三处注册（`EMBEDDED_PROMPTS` / `EXPERT_ROLE_MAP` / `PROFILE_TOOLS`）；修正 `max_concurrent` 过期注释
- `src-tauri/crates/analysis-engine/src/prompts.rs`：`EXPERT_IDS` 移除该项；注释改标注「已过期且零消费者」
- `src-tauri/src/commands/stock_analysis_setup/seed_stock_analysis.rs:2967`：悬空引用改指 `portfolio-mgr.rhai`

**备份 / 归档**（均在 `output/backup-2026-09-14/`）
- `dqi-grade-unify/` —— 第一轮 dqi 改动基线
- `fmt-baseline/` —— 8 个 fmt 目标文件改动前快照
- `registration-removal/` —— `mod.rs` / `prompts.rs` / `data-quality.rhai` 改动前快照
- `archived-experts/` —— `portfolio-manager.md` / `policy-chain.md` 归档副本

