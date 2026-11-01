# 审计：explainer 节点补全 + 前端 profile 映射表补齐（2026-09-14）

> 范围：本轮两个用户裁决项 —— ① `explainer` 节点「补全」（接出口 + 补注册）；
> ② 前端 `PROFILE_NAMES` 「补齐」（真实缺口 22 个，非 7 个 —— 见 §二）。附带修复执行过程中暴露的 3 条既存红灯（均为测试滞后，见 §三）。
> 全部改动均已通过 `tsc` / `dprint` / `oxlint` / `vitest` / i18n 三项 /
> `cargo check --workspace --all-targets`（23m41s，0 error 0 warning）。矩阵见 §四。

---

## 一、explainer 节点补全

### 1.1 断链的两层（此前是「声明的机制 ≠ 运行的机制」的复合形态）

| 层 | 形态 | 事实 |
|---|---|---|
| ① 入边缺失型（零件齐、线没接） | **profile 不存在** | 模板节点 `decision-explainer` 的 `agent_profile_id = "stock-explainer"`，但 `.md` / `EMBEDDED_PROMPTS` / `EXPERT_ROLE_MAP` 三处全无 `explainer` ⇒ `seed_agent_profiles` 不会为它建行 ⇒ `agent_executor` 解析 profile 得 `None` ⇒ **experts 提示词整段跳过（只打一条 WARN，节点仍 completed）** |
| ② 出口无消费者（线接了但没人读） | **产出零消费端** | 节点输出 `{summary, explanation, rule_trace[], risk_comment, confidence_note}` 只被写进 `blackboard_snapshot`，`notify-result` 用固定文案、`store-result`/`end-output` 取 `portfolio-risk-gate`、前端 grep 三字段 0 命中 |

两层都必须修，只修①等于「让说明书从勉强能读变成写得更好」，用户仍看不到。

### 1.2 补注册（4 处，此前缺 3 处）

| # | 位置 | 改动 |
|---|---|---|
| 1 | `agency_experts/stock-analysis/explainer.md` | **新建**。承载「定位 / 三层来源归属判据 / 反幻觉护栏 / 风格要求」 |
| 2 | `mod.rs::EMBEDDED_PROMPTS` | `("explainer", include_str!(...))` |
| 3 | `mod.rs::EXPERT_ROLE_MAP` | `("explainer", "decision-maker")` —— **这一表是 `seed_agent_profiles` 的遍历源**，不在其中就不会建 profile |
| 4 | `mod.rs::PROFILE_TOOLS` | `("explainer", &[])` —— 显式声明「已评估：不需要工具」，与「未配置（NULL）」区分 |

**不重复定义原则**：JSON schema 与「规则追溯码对照表」（R-200~R-210 / R-401/402/405 + 三条护栏）
**仍保留在节点 inline prompt 里**（recency 段，编译期确定、不依赖 DB 种子），`explainer.md`
只写方法论与判据，并明确指路「编号对照表由节点指令给出，以其为唯一权威」——
避免把同一张编号表维护成两份（禁区 12）。

同时 `expert_id_to_display` 补 `"explainer" => "决策解释官"`（profile 行的显示名）。

> 未动 `crates/analysis-engine/src/prompts.rs::EXPERT_IDS`：该常量**零消费者**且其文档已声明
> 「已过期，唯一权威是 `EMBEDDED_PROMPTS`，待整体清理」（它本身已缺 Serenity 4 个 + 简化模板
> 7 个专家共 11 项）。往里加一条只是给死清单续命，无功能收益。

### 1.3 接出口（三层改动，让数据真正到达用户）

| 层 | 文件 | 改动 |
|---|---|---|
| 落库形态 | `crates/analysis-engine/src/blackboard.rs` | `decision-explainer` 加入 `is_structured` 白名单。此前它走 `extract_node_text` 被**压成纯文本**，前端拿不到 `rule_trace` 数组 → 规则追溯码无法逐条渲染 |
| 状态层 | `stores/feature/stockAnalysisStore.ts` | 新增 `decisionExplanation` 字段；**两条路径**都填充：实时 `workflow-completed` 读 `results["decision-explainer"]`、回放 `loadAnalysis` 读 `blackboard_snapshot["decision-explainer"]` |
| 解析层 | `lib/agentOutput.ts` | 新增 `parseDecisionExplanation(raw)` + `DecisionExplanation` 类型。兼容 AgentNode 包装 / 纯 JSON 字符串 / 裸对象三形态；**四个内容字段全空 ⇒ 返回 `null`**（不渲染空壳面板） |
| 视图层 | `components/stock-analysis/DecisionExplanationPanel.tsx` | **新建**。区块默认显示 summary，可展开看 explanation + rule_trace（带 PASS/VETOED/DOWNGRADED 配色）+ risk_comment + confidence_note |
| 挂载 | `components/stock-analysis/DecisionBanner.tsx` | 在决策卡「紧凑指标行」之前挂载（独立模式完整卡片） |

**`rerunDecision` 显式置空该字段**：该路径只重跑 `portfolio-mgr`、不重跑 explainer，
旧说明书描述的是上一轮裁决 —— 继续展示会与新决策矛盾（同一次决策两个真相源）。

### 1.4 加门禁（把这次的教训固化成检查）

| 门禁 | 位置 | 检什么 |
|---|---|---|
| 编译期三表一致性 | `seed_consistency_tests.rs::expert_registration_tables_are_consistent` | `EMBEDDED_PROMPTS` ↔ `EXPERT_ROLE_MAP` key 集合相等；无重复 id；`PROFILE_TOOLS` 无孤儿 |
| 编译期「显式登记」 | `seed_consistency_tests.rs::profile_tools_have_no_orphans` | 有专家但无工具行 ⇒ 报错（要求显式 `&[]`，不允许 NULL 含糊过去） |
| 运行期越界告警 | `seed_stock_analysis.rs`（写库前） | 遍历模板全部 `agent_profile_id`，对不在 `EXPERT_ROLE_MAP` 的引用打 `tracing::warn` —— 这类「模板声明了但没注册」的断链此前**全程静默** |
| 前端 i18n 完整性 | `components/settings/__tests__/agentProfileMaps.test.ts` | 五张表 key 集合两两相等 + 每个 i18n key 在 11 语言包存在 |

### 1.5 顺带修掉的 5 个「有专家、无工具行」

一致性测试一上就抓出：`debate-convergence` / `risk-convergence` / `reflection` /
`social-media-analyst` / `volume-price-analyst` 在 `EMBEDDED_PROMPTS` 里但不在 `PROFILE_TOOLS`，
`recommended_tools` 落 **NULL（「未配置」，与「确认为空」无法区分）**。

处理：前三个是收敛/复盘节点（输入由上下文注入，确实不需要工具）→ 显式 `&[]`；
后两个**当前未接入任何模板**（仅专家库候选）→ 也显式 `&[]`，但在注释里标注
「接入模板前必须先按数据源确定白名单，不要沿用本行」，把这个待决问题显式留在代码里。

---

## 二、前端 PROFILE_NAMES 补齐

### 2.1 真实缺口是 22 个，不是 7 个

上一轮的表述「缺 7 个」来自不完整的口径。以**后端唯一权威** `EXPERT_ROLE_MAP`（38 个）为准，
前端 5 张平行表当时各只有 16 个 key ⇒ 真实缺口 **22 个**；加上本轮新增的 `explainer`，
终态应为 **39 个**。

### 2.2 为什么必须补齐（失败形态）

`AgentProfileList.loadAll()` 在拿不到模板节点时走「离线回退」分支，用
`Object.keys(PROFILE_NAMES)` **驱动渲染** —— 少一个 key 就**凭空少一行**；
`PROFILE_ROLE_IDS` / `PROFILE_ROLES` 缺 key 时静默兜底成 `""` / `"-"`
（正是铁律 73 的「幽灵型 / 拼接型」两种断裂）。

### 2.3 改动

- 5 张表各补齐至 39 条：`PROFILE_NAMES` / `PROFILE_ROLES` / `PROFILE_ROLE_IDS` / `PROFILE_NAME_KEYS` / `PROFILE_ROLE_KEYS`
- 抽出到独立模块 `components/settings/agentProfileMaps.ts`（数据与视图分离，使单测可直接断言）
- i18n：11 个语言包 × 23 个 `stockAnalysis.analystRoles.*` 新 key（**全 11 语言真实翻译**，
  不是英文占位 —— 缺 key 时 i18next 的 `fallbackLng` 会兜到 en-US，再无就把原始 key 串显示到界面）
- `PROFILE_ROLE_KEYS` 无需新 key：新增 23 条的 role 全部落在既有 5 类
  （`analyst` / `debateResearcher` / `riskAssessor` / `decisionMaker` / `trader`）

---

## 三、顺带修复的既存红灯（3 条，均为测试滞后，非行为变更）

| 测试 | 现象 | 判定 | 处理 |
|---|---|---|---|
| `src/lib/__tests__/agentOutput.test.ts` | 3 个用例失败（`positionState: null` 缺失） | P1-2(2026-09-14) 给 `StockDecision` 加 `positionState` 时未更新夹具 | 补夹具字段 |
| `src/types/__tests__/stock-analysis.test.ts` | `parseAction("unknown")` 期望 WAIT 实得 UNCERTAIN | P1-1(2026-09-14) 已明确「解析失败 = UNCERTAIN ≠ 观望」，实现有文档注释背书，**测试断言过期** | 改断言 + 补注释说明语义依据 |
| `src/components/settings/StockAnalysisConfigPanel.test.tsx` | `actual_market_text` 后端已种子化但面板缺失 | 该变量属于**反思复盘工作流**的运行时注入（`reflection.rs` 写入价格事实），扫描源 `mod.rs` 同时承载 reflection 模板定义故被带入；既有白名单已有 3 个同族变量，漏了它 | 加入 `runtimeInjected` 白名单 + 注释说明判定依据 |

> `src/__tests__/App.d2.test.tsx` 在全量并发下偶发失败、单独运行通过 —— **并发抖动**，非缺陷，未改。

---

## 四、验证矩阵（全绿）

| 项 | 命令 | 结果 |
|---|---|---|
| 类型检查 | `npx tsc --noEmit` | **exit 0**（无输出） |
| 格式 | `npx dprint check` | **0 差异** |
| Lint | `npx oxlint src --max-warnings=0` | **0 warnings, 0 errors**（1227 文件） |
| 前端测试 | `node scripts/vitest-run.mjs run` | **145 passed / 1 skipped，1651 passed / 2 skipped，0 failed** |
| i18n 完整性 | `python scripts/check_i18n.py --strict` | **全部通过**（11 locale 语法 + zh-CN 无空值 + 代码引用 key 全部有定义） |
| i18n 未翻译 | `node scripts/check-i18n-untranslated.mjs` | **新增 0 项**（11 语言均与基线一致） |
| Rust | `cargo check --workspace --all-targets` | **exit 0 · 0 error / 0 warning**（`Finished dev profile` in 23m41s；含 `--all-targets` ⇒ 新增的 `seed_consistency_tests` 已一并编译验证） |

**未做的验证（须诚实标注）**：没有跑真实工作流做端到端对照。
`explanation` / `rule_trace` 能否正确落进 `blackboard_snapshot` 并渲染，
需在 601166 / 600031 上复跑一次完整分析确认（单测只覆盖解析层与映射层）。

---

## 五、待裁决 / 后续建议

1. **`EXPERT_IDS` 死清单**：`crates/analysis-engine/src/prompts.rs` 的 `EXPERT_IDS` 零消费者、
   已缺 11 个专家、文档自称待清理 —— 建议整体删除（收敛到唯一咽喉 `EMBEDDED_PROMPTS`）。
2. **`social-media-analyst` / `volume-price-analyst`**：注册在专家库但零模板引用，
   工具白名单暂为 `&[]`。若要接入，需先确定其数据源工具。
3. **`App.d2.test.tsx` 并发抖动**：单独运行稳定通过，全量并发下偶发失败，建议排查
   跨测试的模块级状态泄漏。
4. **运行期对照**：建议在真实股票上复跑一次，核对决策卡上「决策依据说明书」区块的
   内容与 DB `blackboard_snapshot["decision-explainer"]` 一致。

---

## 六、方法沉淀

本轮识别出的判据「**建行遍历源 vs 填充源**」已沉淀为技能 `rust-consistency-audit` 的
**方法二十一** → `references/method-21-registration-traversal-source.md`。
`SKILL.md` 三处已同步（frontmatter `description` 症状列表 / ①症状分流表 / ③方法全文索引），
计数由 20 个方法更新为 **21 个**。

核心口诀：
> **别问「哪张表里有这个名字」，要问「这一行是谁 `insert` 出来的」。**
> 建行源缺一行 ⇒ 实体不存在（消费侧多为 `warn + continue` ⇒ **静默降级**）；
> 填充源缺一行 ⇒ 该列落 `NULL`。**两者症状不同、修法不同、等级不同。**

与相邻方法的分工：方法四查**入口**、方法十九查**出口**、方法二十查**名字**，
方法二十一查**这一行是谁建出来的**。
