# 决策标签 action 语义收敛方案

> 时点：2026-09-14 ｜ 范围：stock-analysis 工作流的决策标签（action）全链路 ｜ 状态：✅ 已完成（2026-09-14 第一轮 + 2026-09-22 第二轮，全部 P0/P1/P2 修复 + i18n 清零 + CI 门禁接线）

## 0. 结论摘要

用户提问「减持/卖出/观望/买入这些概念是不是同义却分成了不同标记」——**是，且不止是命名问题**。

- 7 个前端标签实际只承载 **3 个语义**（看多 / 中性 / 看空）。
- 同一条链路上存在 **6 套互不相同的值域定义**，其中两处会在值域漂移时**静默失效**（fail-open）。
- 已实证：项目自己 prompt 里的示例文本会让 action 解析**方向反转**（3/5 用例不符）。
- 已实证：`TradePanel` 把「观望 / 不确定」当成**买入方向**填进表单，会误导实际下单录入。
- 附带发现：`stockAnalysis` 命名空间有 88 个 key 在 de / fr / es / ru / hi / ko 六种语言中**值仍是中文**（key 存在，检查脚本查不出来）。

---

## 1. 问题清单

### 1.1 P0 — 真缺陷（会误导用户操作或使约束静默失效）

| # | 现象 | 证据（文件:行） | 根因 |
|---|---|---|---|
| P0-1 | 用「观望 / 不确定」决策点「一键录入」，表单方向被填成**买入** | `src/components/stock-analysis/TradePanel.tsx:117` 只拦 `HOLD`/`REDUCE`，漏 `WAIT`/`UNCERTAIN`；`:131` 与 `:105` 均为 `action === SELL ? "sell" : "buy"` | 作者以为「不该买入」= `HOLD｜REDUCE`，漏掉同义的 `WAIT`/`UNCERTAIN` |
| P0-2 | 从自由文本推导 action，**方向反转** | `src/lib/stock-analysis-utils.ts:155-164` + `:111-129`；调用点 `src/stores/feature/stockAnalysisStore.ts:1199`、`:1229` | `parseAction` 用 `raw.includes(label)` **按对象键序**扫描：`买入`(键1) 先于 `看空`(键13)；`减持`(键4) 先于 `看多`(键12)。文本里出现否定词或风险词即被反转 |
| P0-3 | 风控否决 fail-open | `src-tauri/crates/analysis-engine/src/portfolio_formula.rs:396`（经 `src-tauri/src/commands/stock_workflow/rhai_pm.rs:80-83` 注册）只 match 中文 `买入/增持/持有` | 值域不一致：一旦链路上 action 为英文，**极高风险的"禁止持仓"完全不生效**并静默返回原值 |
| P0-4 | 回测策略选错 | `src-tauri/src/commands/stock_analysis.rs:1819-1827` `map_action_to_strategy_id` 无 `"WAIT"` 分支 → 落到 `_ => "watchlist"`；同义的中文「观望」→ `capital` | 同一语义两条入口得到两个策略 |
| P0-5 | 同一「决策缺失」两种语义兜底 | `src-tauri/src/commands/stock_analysis.rs:1486` 用 `unwrap_or_default()` → 空串 → 前端 `parseAction("")` → **观望**；同文件 `:4545` 却兜底为 **`"uncertain"`** | 缺失状态没有唯一哨兵，被两种业务语义吸收 |

#### P0-2 实证（正算复现，非推断）

以 `src-tauri/agency_experts/stock-analysis/trader.md:93` 的**示例 reasoning**（prompt 自身提供）为输入，逐字复用 `parseAction` 逻辑：

| 输入 | 期望 | 实际 | 结果 |
|---|---|---|---|
| trader.md:93 示例（`方向:看多` + `风险点：大股东减持 5%`） | BUY | **REDUCE** | 不符 |
| `方向:看空。……不建议买入。` | SELL | **BUY** | 不符 |
| `方向:中性。……等待更清晰的信号。` | HOLD | **WAIT**（被无关词「等待」先命中） | 不符 |
| `方向:看多。基本面改善，目标价上调。` | BUY | BUY | OK |
| `方向:看空。行业景气度下行，跌破支撑位。` | SELL | SELL | OK |

不符 **3 / 5**。复现脚本：`output/tmp-action-parse-repro.mjs`（输出 `output/tmp-action-parse-repro.txt`）。

### 1.2 P1 — 口径分叉（静默、无告警）

| # | 现象 | 证据（文件:行） |
|---|---|---|
| P1-1 | **6 套值域并存**：① 前端 7 值 `BUY/INCREASE/HOLD/REDUCE/SELL/WAIT/UNCERTAIN` ② 中文映射表 15 词 ③ 公式实际产出 6 个中文 ④ LLM prompt 六选一 ⑤ 回测映射（认 `观望\|UNCERTAIN`，无 `WAIT`）⑥ 风控否决（仅中文） | `src/lib/stock-analysis-utils.ts:98-106`、`:111-129`；`src-tauri/src/commands/portfolio-mgr.rhai:1597-1613`；`agency_experts/stock-analysis/trader.md:101`；`stock_analysis.rs:1819-1827`；`portfolio_formula.rs:396` |
| P1-2 | **持有 ⇄ 观望 靠 positionPct 单向互改**：仓位>0 时观望**升级**为持有；仓位≤0 时持有/买入/增持**降级**为观望。两者不是两个语义，是同一状态的两个标签 | `portfolio-mgr.rhai:1619-1623`（升级）、`:1642-1649`（降级记录）、`:1656-1666`（final_action） |
| P1-3 | **verdict 词被映射进 action 空间**：`看多→BUY`、`看空→SELL`、`中性→HOLD`。一旦 LLM 把 verdict 写进 action 字段，`增持`档与`观望`档当场丢失 | `src/lib/stock-analysis-utils.ts:123-125`；约定见 `trader.md:127-131`（「持有 / 观望 → 中性」是**一对二**，映射表却只留 HOLD） |
| P1-4 | 落库**不归一化、无约束**：直接写入决策 JSON 的 `action` 原文，列为 `Option<String>` | `src-tauri/src/commands/stock_workflow/core.rs:1597-1604`；实体 `crates/entities/src/stock_analyses.rs:16` |
| P1-5 | 同一套阈值**两份拷贝**：命名常量支持 `present(action_buy_threshold)` 被工作流参数覆盖，`f7_free_action` 却把 `0.63/0.53/0.48/0.38/0.30` 硬编码了一遍。参数一旦覆盖，两个口径立刻分叉 | 常量 `portfolio-mgr.rhai:119-126` vs 硬编码 `:1310-1315` |
| P1-6 | 消费端硬编码中文穷举，未走统一归一化：`matches!(decision_action.as_str(), "观望" \| "卖出")` | `src-tauri/src/commands/stock_workflow/hooks.rs:129-150`（判定在 `:135`） |

### 1.3 P2 — 一致性 / 卫生

| # | 现象 | 证据 |
|---|---|---|
| P2-1 | **注释声明的区间与实际阶梯不符**：注释写「持有 ← 0.48-0.53」「观望 ← 0.38-0.50」「减持 ← 0.30-0.40」，三处两两重叠（0.48-0.50、0.38-0.40）；实际代码是 `else-if` 阶梯，互斥，重叠带归前序档 | 注释 `portfolio-mgr.rhai:1584-1590` vs 实现 `:1597-1613` |
| P2-2 | **i18n：`stockAnalysis` 有 88 个 key 在 6 种非汉字语言中值仍为中文**（key 存在、`390` 全语言对齐，故「缺 key 检查」全绿）。`actionUncertain` 仅 zh-CN / en-US 正确；zh-TW 为简体「不确定」（应为「不確定」） | 量化见下表；样例 `src/i18n/locales/de.json:14804` |

i18n 量化（脚本 `output/tmp-i18n-audit.mjs`，全 11 语言 `stockAnalysis` 均为 390 key）：

| 语言 | 纯中文值数量 | actionWait | actionUncertain |
|---|---|---|---|
| zh-CN | 248（正常） | 观望 | 不确定 |
| zh-TW | 245（正常） | 觀望 | **不确定**（简体污染） |
| ja | 124（汉字正常） | 様子見 | 不確定 |
| en-US | 4 | Wait | Uncertain |
| **ko** | **88** | 관망 | **不确定** |
| **de** | **88** | Warten | **不确定** |
| **fr** | **88** | Attendre | **不确定** |
| **es** | **88** | Esperar | **不确定** |
| **ru** | **88** | Подождать | **不确定** |
| **hi** | **88** | प्रतीक्षा | **不确定** |
| **ar** | **65** | انتظار | Uncertain（英文，未译） |

六种语言的 88 个未翻译 key **名单完全一致**（`actionUncertain, amplitude, analystConsensus, bearish, bullish, cancelled, contradiction, debtRatio, earningsOnChart, timeHorizon*` 等）→ 判定为**批量添加时只写了 zh-CN / en-US**。

> 注：`scripts/check-hardcoded-i18n.sh` 只检查**代码里的硬编码文案**，不检查 **locale 值是否已翻译**，因此这类问题不会被告警。

---

## 2. 三套收敛方案对比

| 维度 | 方案 A：最小收敛 | 方案 B：拆正交两轴（推荐） | 方案 C：枚举单一权威 |
|---|---|---|---|
| 核心动作 | 保留 7 值，仅**澄清文档语义**为「强度档」；补齐消费端值域缺口；修 P0 全部 | action 只表达**方向强度**（5 档）；新增独立字段 `positionState`（空仓/建仓/持有中/减仓中）表达**持仓状态**；「持有 vs 观望」改为**派生展示**，不再互改 | 在 `axagent-harness` 定义唯一 `DecisionAction` enum；TS 由 `schema-gen` 生成；DB 存 enum + CHECK 约束；全消费端 match enum |
| 消除重叠 | 部分（改名澄清，持有/观望仍互改） | **彻底**（两轴正交，重叠在结构上不可表达） | 彻底 |
| 修 P0-1（TradePanel） | ✅ | ✅（`positionState` 直接给对方向） | ✅ |
| 修 P0-2（文本猜 action） | ✅ 改为读结构化字段，删除 `includes` 兜底 | ✅ | ✅ |
| 修 P0-3/4（fail-open） | ✅ 补英文分支 | ✅ 归一化后单点判定 | ✅ 编译器保证 |
| 修 P0-5（缺失哨兵） | ✅ 统一为显式 `UNAVAILABLE` | ✅ | ✅ |
| 改动面 | 前端 8 文件 + 后端 4 处 | 前端 12 文件 + 后端 6 处 + 1 次 migration | 跨 crate 改造 + 代码生成链路 |
| 历史数据 | 无需迁移 | 需一次映射（旧中文 → 新枚举 + 派生 positionState） | 需 migration + 回填 |
| 风险 | 低 | 中（展示层需回归；已识别 6 个消费组件） | 高（32 crate，且触及生成物链） |
| 回归门禁 | 值域一致性脚本 | 同 A + `positionState` 派生单测 | 同 B + 编译器穷举 |

**推荐：A + B 分两步走，C 作为长期方向。**

理由：
1. P0 全部是**行为缺陷**（不是命名问题），无论选哪个方案都必须修 —— A 的成本仅为 B 的 ~30%，可作为立即止血。
2. 「持有/观望靠 positionPct 互改」是**结构性重叠**（P1-2），改名治不了；只有 B 的拆轴能在结构上让重叠不可表达。
3. C 的收益（编译器穷举）依赖 `axagent-harness` 作为唯一权威，而当前值域权威散落 6 处，直接上 C 会把「值域定义错误」也一起固化。应先 B 收敛值域，再 C 固化。

---

## 3. 分阶段执行清单

### 阶段 1 — 止血（可独立交付，不依赖方案裁决）

| # | 改动点 | 具体动作 | 验证 |
|---|---|---|---|
| 1 | `TradePanel.tsx:105,117,131` | 抽出 `actionToDirection(action)`：仅 `BUY/INCREASE` → `buy`，仅 `SELL/REDUCE` → `sell`，其余（`HOLD/WAIT/UNCERTAIN`）**不填方向**并禁用提交 | 单测覆盖 7 值 × 期望方向 |
| 2 | `stock-analysis-utils.ts:155-164` | 删除 `includes` 扫描兜底；未识别返回**显式** `UNCERTAIN`（不再退回 `WAIT`）；`STOCK_ACTION_LABELS` 移除 verdict 词（`看多`/`看空`/`中性`）并由 `store:1068/1099` 改读结构化 `verdict` 字段 | 复用 `output/tmp-action-parse-repro.mjs` 的 5 用例，要求 5/5 |
| 3 | `portfolio_formula.rs:396` | 值域归一化后再判定（接受中英文）；未识别值**不得**静默返回原值，改为返回 `(action, false, "未识别值域")` 并落日志 | 新增 Rust 单测：中文/英文/未识别 各 1 例 |
| 4 | `stock_analysis.rs:1819-1827` | 补 `"WAIT"` 分支（与中文「观望」同归 `capital`） | 单测：`观望`/`WAIT`/`UNCERTAIN` 三者结果一致 |
| 5 | `stock_analysis.rs:1486,4545` | 统一缺失哨兵为同一显式值（建议 `UNAVAILABLE`），前端单独渲染「数据缺失」而非观望 | 单测 + 前端渲染断言 |

### 阶段 2 — 收敛值域（方案 B 主体）

| # | 改动点 | 具体动作 |
|---|---|---|
| 6 | 新类型定义 | `DecisionAction` = `BUY_STRONG / BUY / HOLD / TRIM / SELL_STRONG`（+ `UNAVAILABLE`）；`PositionState` = `EMPTY / OPENING / HOLDING / TRIMMING` |
| 7 | `portfolio-mgr.rhai:1597-1613` | `base_action` 阶梯映射到新 5 档；`:1619-1623` / `:1642-1666` 的「升级/降级」改为**只设置 `positionState`**，不再改写 action |
| 8 | `portfolio-mgr.rhai:1310-1315` | 硬编码阈值改为复用 `:119-126` 常量（消除 P1-5） |
| 9 | `src-tauri/src/commands/stock_workflow/core.rs:1597-1604` | 落库前归一化；`stock_analyses` 加 `decision_position_state` 列（migration，含回填） |
| 10 | 前端展示 | 「观望 / 持有」由 `(action == HOLD, positionState)` 派生，删除互改 |
| 11 | `hooks.rs:129-150` | 硬编码中文改为新枚举判定 |
| 12 | 受影响的 6 个前端消费组件 | `DecisionHeroBar` / `DecisionBanner` / `DecisionComparisonPanel` / `CompactDecisionComparison` / `AnalysisHistoryButton` / `ExperimentSidebar` 按新枚举适配，i18n key 同步 11 语言 |

### 阶段 3 — 补 i18n（可与阶段 2 并行）

| # | 改动点 | 具体动作 |
|---|---|---|
| 13 | `src/i18n/locales/{de,fr,es,ru,hi,ko}.json` | 88 个 `stockAnalysis.*` 中文值翻译；`ar` 补 65 个；`zh-TW` 修 `actionUncertain` 简体；`ar` 的 `actionUncertain` 由英文译出 |
| 14 | 新增检查脚本 | `scripts/check-i18n-untranslated.mjs`：对非汉字语言断言「值不含纯 CJK」，作为 CI 门禁（现 `check-hardcoded-i18n.sh` 覆盖不到） |

---

## 4. 防回归门禁建议

| 门禁 | 检查内容 | 为什么必要 |
|---|---|---|
| 值域一致性脚本 | 扫描全项目 action 字面量（`.ts/.tsx/.rs/.rhai/.md`），断言值域集合 ⊆ 权威定义，且**每处消费端都覆盖全部枚举值** | 本次 6 套值域的根因是「各自 match」，无单点；`apply_risk_veto` 的 fail-open 与 `map_action_to_strategy_id` 缺 `WAIT` 都逃过了现有测试 |
| Rust enum + `#[non_exhaustive]` | 消费端必须 match 穷举；新增档位由编译器报警 | 铁律「加字段靠编译器穷举」——当前全链路是字符串，加值域不会报错 |
| i18n 未翻译检查 | 见阶段 3 第 14 项 | 「key 存在」≠「已翻译」 |
| 文本猜 action 的负向测试 | 用例必须含否定句、风险词（如「大股东减持」）、无关词（「等待」） | 本次实证 3/5 不符，且**含项目自己 prompt 的示例文本** |

---

## 5. 待裁决 / 未取证

| 项 | 说明 |
|---|---|
| 方案选择 | A / B / A+B / C 的**剩余部分**属产品语义与改造成本权衡 —— 见 §7「未实施项」 |
| 「买入/增持、卖出/减持」是否合并 | 二者当前仅靠 posterior 强度分档，与「建仓 vs 加仓」「清仓 vs 减仓」的命名含义无关。**须先决策语义目标**，再定 5 档命名 |
| DB 实际值域未取证 | 本地未找到 `axagent.db`（`init/database.rs:65` 默认路径，生产走 PostgreSQL）。`stock_analyses.decision_action` 的**真实历史值分布**未统计 → P0-4 的实际命中率未量化。建议在可用库上执行一次 `SELECT decision_action, COUNT(*) GROUP BY 1` |
| 2 个临时脚本 | `output/tmp-action-parse-repro.mjs` / `output/tmp-i18n-audit.mjs` 及对应 `.txt` 为取证产物，可随时删除 |

---

## 6. 证据索引

**前端**
- `src/lib/stock-analysis-utils.ts:98-106`（7 值枚举）、`:111-129`（15 词映射）、`:155-164`（parseAction）、`:191-207`（配色）、`:215-258`（Tag 样式）、`:260-278`（i18n key）
- `src/lib/agentOutput.ts:300`（normalizeDecision 内调用 parseAction）
- `src/stores/feature/stockAnalysisStore.ts:1199`、`:1229`（从 reasoning 猜 action）、`:1460`
- `src/components/stock-analysis/TradePanel.tsx:105,117,131`、`DecisionBanner.tsx`、`dual-view/DecisionComparisonPanel.tsx`、`dual-view/CompactDecisionComparison.tsx`、`AnalysisHistoryButton.tsx`、`ExperimentSidebar.tsx`、`stock-workspace/DecisionHeroBar.tsx`

**后端**
- `src-tauri/src/commands/portfolio-mgr.rhai:119-126`（阈值常量）、`:1310-1315`（硬编码副本）、`:1584-1590`（重叠区间注释）、`:1597-1613`（base_action 阶梯）、`:1619-1623`（观望→持有）、`:1642-1649`、`:1656-1666`（final_action）
- `src-tauri/src/commands/stock_analysis.rs:1486`、`:1819-1827`（回测映射）、`:1926`、`:4489`、`:4545`、`:5914`
- `src-tauri/src/commands/stock_workflow/core.rs:1597-1604`（落库）、`hooks.rs:129-150`、`rhai_pm.rs:80-83`（veto 注册）
- `src-tauri/crates/analysis-engine/src/portfolio_formula.rs:396`（apply_risk_veto）
- `src-tauri/crates/entities/src/stock_analyses.rs:16`（`decision_action: Option<String>`）

**Prompt / 规范**
- `src-tauri/agency_experts/stock-analysis/trader.md:93`（示例 reasoning）、`:101`（六选一）、`:134-138`（verdict 表）、`:140-144`（targetPrice 约束）

---

## 7. 实施记录（2026-09-14，按 P0 → P1 → P2 顺序落地）

### 7.1 已修复

| # | 修复内容 | 落地形态 |
|---|---|---|
| P0-1 | 「一键录入」方向填错 | `actionToDirection()` 单一判据；无方向时 `message.warning` 并拒绝提交。`TradePanel` + `DecisionBanner` 两个入口同源 |
| P0-2 | 自由文本猜 action 方向反转 | `parseAction` 改**严格全等**匹配（删 `includes` 扫描）；`STOCK_ACTION_LABELS` 移除 verdict 词；新增 `parseDirectionFromText`（只认 `方向:`/`verdict:`/`stance:` 显式标记）；`store:1068/1099` 改读结构化字段。新增 19 个前端单测（含 trader.md:93 自带示例的负向回归） |
| P0-3 | `apply_risk_veto` fail-open | 先 `normalize_action` 再判定；未识别值在高/极高风险下显式降级并留归因 |
| P0-4 | 回测策略映射缺 `WAIT` | 改走 `normalize_action` **穷举** match；`Unavailable`/未识别单列 `watchlist`（区别于「观望是结论」） |
| P0-5 | 缺失哨兵两种形态 | 统一为 `ACTION_UNAVAILABLE`。**共 4 处**：`extract_evidence_citations` 的空串、`get_latest_analysis_for_stock` 的硬编码 `"uncertain"`、`get_latest_analyses_for_stocks` 批量构造点的硬编码 `"uncertain"`、`backtest_history` 的 `unwrap_or("持有")` |
| P1-1 | 6 套值域并存 | 新建 `analysis-engine/src/decision_action.rs` 为**唯一权威**（`ActionKind` 八档 + `normalize_action`/`verdict_to_action`/`is_unavailable`），含 5 个单测 |
| P1-2 | 持有 ⇄ 观望靠仓位互改 | 新增 `PositionState` 轴：rhai 输出 `positionState`，DB 加 `decision_position_state` 列（migration v228），前端 `resolveDisplayAction` 单点派生。见 §7.2 关于「互改本身是否移除」的说明 |
| P1-3 | verdict 词混进 action 空间 | 映射表移除 verdict 词；`trader.md` 补「字段边界」与「六档不得合并」约束 |
| P1-4 | 落库不归一化 | 新增 `normalize_action_for_storage` 作为**唯一写入口**，两个落库点共用 |
| P1-5 | 阈值两份拷贝 | `f7_free_action` 改用 `ACTION_*_THRESHOLD` 常量（支持 `input_mapping` 覆盖） |
| P1-6 | 消费端硬编码中文穷举 | **共 17 处**收敛（多处为实施中才扫出，见 §7.3）：`hooks.rs`、`core.rs`（落库 + 告警门）、`decision.rs`（一致性评分 ×2 块）、`backtest.rs`（`was_correct`）、`dashboard_report.rs` ×3、`rules.rs`、`trade_intent.rs`、`conditional_order_bridge.rs`、`portfolio_formula.rs` ×2（R-200/R-210）、`stock_analysis.rs`、前端 `getActionColor`、`getActionTagStyle`、`dual-view` ×2（配色 + 差异判据）、`ExperimentSidebar`（跨值域比较） |
| P2-1 | 注释与实现不符 | `portfolio-mgr.rhai` 重叠区间注释改为说明「互斥 else-if 阶梯 + 重叠带归靠前档」 |
| P2-2 | i18n 值未翻译 | 顶层 `stockAnalysis` 已清零（见 §7.2 的量级更正）；新增 CI 门禁 `scripts/check-i18n-untranslated.mjs`（含基线棘轮 + 内建判据自检） |

### 7.2 本计划**自身**的两处测量/口径缺陷（实施中才发现，须更正）

1. **P2-2 的 88 个 key 是「只有顶层」的统计口径 ⇒ 严重低估。**
   原统计 `keys.filter(k => hasCJK(t[k]))` 对**对象值**返回 false，因此整个**嵌套命名空间**
   （`stockAnalysis.risk.*` / `stockAnalysis.period.*` / `stockAnalysis.dualView.*` 等）从未被扫描。
   按递归口径重新量化：**8 种语言合计 13228 项**未翻译值（de/es/fr 各 1829、ru 1831、hi 1839、
   ko 1830、ar 1751、**en-US 490**）。其中 `en-US` 有 490 项中文值尤其严重 —— 它是英文 fallback。
   本次只清了 **107 个顶层 key × 7 语言 + en-US 的 4 项**具名缺陷（`calibrate` / `gradeSuffix` /
   `rerunDecision` / `rerunDecisionHint` 在 en-US 中值仍是中文），其余按**棘轮基线**冻结为「不得新增」。
2. **P1-2「互改」本版只做了「显式化」，未做「移除」。**
   `action` 的取值仍保持既有互改结果（行为兼容），仅把原本靠 `positionPct` 反推的隐式关系
   显式化为 `positionState`，并让展示层单点派生。**移除互改是行为变更**（会连带改变
   `apply_risk_veto` / `inject_reco_crosscheck` / 告警创建门 的判定对象），须单独裁决后实施。

### 7.3 实施中新发现（原计划未列，均已一并修复）

| 现象 | 证据 | 性质 |
|---|---|---|
| 一致性评分的 `is_buy/is_hold/...` 判据只认中文 | `decision.rs:1330-1350`（**第二个**评分块，原计划只记了第一个） | P1-6 同类；英文值域下落 `(Some,Some) => 0.0` 被判「完全对立」 |
| 声明分值与分支不互斥（不可达分支） | 同上：`(持有\|观望) vs 不确定 = 3` 吃掉 `观望 vs 不确定 = 6` | P1-6 同类；两处同病 |
| `getActionColor` / `getActionTagStyle` **不解析**原串 | `stock-analysis-utils.ts:375,400` | 传中文/小写值即无配色；同文件 `getActionTKey` 却是解析的，三者判据不一致 |
| `dual-view` 差异判据按**字形**比较 | `DecisionComparisonPanel.tsx:55`、`CompactDecisionComparison.tsx:100` | 「公式=BUY vs LLM=买入」被判成分歧 ⇒ 假警报 |
| `ExperimentSidebar` 跨值域比较 | `ExperimentSidebar.tsx:128` | 本地仿真 `"buy"` vs 后端 `"BUY"` ⇒ 「参数已修改」指示器永久常亮 |
| `trade_intent.rs` 半归一化 | `src-tauri/crates/analysis-engine/src/trade_intent.rs:171` | 判据含 `hold`/`watch` 却漏 `WAIT` 变体与「不确定」 |
| `QuickBacktestSample.decision_action` 由收益**反推** | `stock_analysis.rs:5932` | 用结果当「决策」的 look-ahead 标签，且「卖出/持有」本身是两概念合一 |
| `rt-workflow` 无 action 值域消费点 | grep 全 crate 仅测试夹具命中 | 无需改动（记录以避免下次重复排查） |
| `astock-data::quality.rs` / `sentiment.rs` 的中文词表 | `quality.rs:113-117`、`sentiment.rs:77-132` | **不是** action 值域 —— 是「报告正文/情绪」关键词表，故意中文化，**不改** |
| `trade_import.rs` 多语言别名表 | `trade_import.rs:84-85` | 成交记录导入的多语言枚举，**不改** |
| `check_i18n.py` 未被 CI 调用 | `.github/workflows/ci.yml` 无 `check:i18n` | 又一处「声明的机制 ≠ 运行的机制」；本次未动，仅记录 |

### 7.4 第二轮收敛（同日，审计口径扩大后新扫出 7 处）

**为什么会有第二轮**：§7.3 的 P1-6 消费端是用**窄 pattern** grep 出来的（只搜 `is_buy` / `is_sell`
这类自定义辅助函数名）。改用**宽 pattern**（直接搜 action 中文字面量 `"买入"|"卖出"|…`）复查后，
又扫出 7 处同类消费端。**这不是「新出现的 bug」，是上一轮审计方法本身的缺口** ——
按函数名搜索必然漏掉「不封装辅助函数、直接内联 `matches!(action, "买入")`」的写法。

**结构性根因（本轮一并修掉）**：`decision_action` 原先放在 `analysis-engine`，而
`harness` / `market-sim` / `notification` / `astock-data` **都不依赖** analysis-engine
（实测 `harness` 内部零依赖，其余四个都依赖 harness）。⇒ 这些 crate 的消费端**永远够不到**
唯一归一化器，只能各自 `match` 中文字面量。**靠逐个 grep 消费端治不了这个根**。

修法（按 AGENTS.md 规则 12「共享数据模型/契约的权威定义在 `axagent-harness`」）：

1. `analysis-engine/src/decision_action.rs` → **上移至 `harness/src/decision_action.rs`**；
2. `harness/lib.rs` 新增 `pub mod decision_action;` + 7 个符号 `pub use`；
3. `analysis-engine/lib.rs` 改为 `pub use axagent_harness::decision_action;` 转发 ⇒
   `crate::decision_action::*` 与 `axagent_analysis_engine::decision_action::*` **两个旧路径不变，消费端零改动**；
4. 新增 `ACTION_ALIASES` 共享别名集 —— 供「输入是 LLM 自由文本」的消费端做宽松匹配
   （形如「买入（分批建仓）」严格归一化会返 `None`），别名集合必须来自权威模块，不得再各写一份。

| # | 现象 | 证据 | 性质 |
|---|---|---|---|
| 1 | `harness` 的「买入信号应有目标价」校验只认 3 个中文词 | `harness/dashboard_report.rs:272` | 英文 `BUY`/`INCREASE` 不匹配 ⇒ **校验被静默跳过**（fail-open，与 P0-3 同族） |
| 2 | `market-sim` 策略派发只匹配中文 | `src-tauri/crates/market-sim/src/agent/strategy.rs:151` | 英文 `BUY` 落 `_` 只打印一条 warn 后**什么都不做**，而模拟仍报执行成功（**静默 no-op**）。现对 8 档穷举，第 5–8 档单列告警（不确定/缺失 ≠ 判断为观望） |
| 3 | `backtest.rs` 同一文件**第三处**中文字面量分类点 | `backtest.rs:438, 450` | 英文值域全被计成「持有」⇒ `buy_r` 恒偏低 ⇒ 下方权重调整静默走错分支 |
| 4 | `node_quality` 的 action 校验白名单仅 8 个字面量 | `node_quality.rs:434` | 「观望 / 增持 / 减持 / wait / trim」等**合法**取值被误判「action 值异常」并扣 10 分（**假阳性惩罚**，让质量分系统性偏低）。改为 `normalize_action(a).is_some()` ∨ 命中 `ACTION_ALIASES`（保留对冗长 LLM 输出的容错） |
| 5 | 报告 action 配色只认 4 个中文词 | `src-tauri/crates/analysis-engine/src/report.rs:205` | 英文值域落 `_` ⇒ 看多/看空**都被渲染成中性黄**（与前端 `getActionColor` 同类） |
| 6 | 退出信号「分析建议卖出」只认中文 | `exit_recommend.rs:221` | 英文 `SELL`/`REDUCE` **不触发**退出信号（漏报，fail-open 方向） |
| 7 | 买入前二次确认只认中文 | `trading.rs:204` | 英文 `SELL` 不触发「分析建议卖出而非买入，请二次确认」（漏报） |

**本轮明确「不改」（含理由，避免下次重复排查）**：

| 对象 | 位置 | 不改理由 |
|---|---|---|
| 报告 markdown 正文关键词打分 | `stock_analysis_round.rs:272-278` | 输入是**整篇报告文档**而非 action token，与 `quality.rs`/`sentiment.rs` 同族。它缺 `持有/观望/hold/wait` 属「文档启发式广度」问题；改它会直接改变节点质量分（**行为变更**）⇒ 按铁律 7 只记录 |
| 空 action 被填成「持有」 | `harness/dashboard_report.rs:300` | 同一「把缺失吸收成结论」家族，但它是**有文档的占位策略**（注释自陈借鉴 DSA 报告完整性占位），「缺失时填哪个值」属**产品语义决策** ⇒ 只记录不擅改 |

### 7.5 验证（2026-09-14）

| 门禁 | 结果 |
|---|---|
| `cargo check --workspace --all-targets` | **0 error / 0 warning** |
| `cargo test -p axagent-harness --lib` | **600 passed / 0 failed**（含本轮新增 3 个回归测试） |
| `cargo fmt --check` | **EXIT 0** |
| `cargo clippy --workspace --all-targets --all-features` | 本轮提交前运行（CI 门禁） |
| 前端 `npx tsc --noEmit` | **EXIT 0** |
| 前端 `vitest run stockAnalysisAction.test.ts` | **19 / 19 passed** |
| 前端 `dprint fmt` | 已格式化 7 个文件 |
| `node scripts/check-i18n-untranslated.mjs` | **EXIT 0**（8 种非 CJK 语言 13228 项基线**零新增**；内建判据自检 10 例 + 4 组对照通过） |

**本轮新增回归测试（钉住两处已修的 fail-open，防再被静默改回）**：

1. `harness::dashboard_report::tests::test_buy_signal_in_english_also_requires_target_price`
   —— 正对照：`BUY`/`buy`/`INCREASE`/`increase`/`买入`/`增持`/`强烈买入` 7 个形态都必须在缺目标价时报缺失；
2. `harness::dashboard_report::tests::test_neutral_signal_does_not_require_target_price`
   —— 负对照：`HOLD`/`WAIT`/`持有`/`观望`/`UNCERTAIN`/`不确定`/`UNAVAILABLE` 7 个中性/哨兵档位**不得**被要求目标价（否则凭空造缺失项）；
3. `harness::decision_action::tests::别名集覆盖全部可识别取值`
   —— 正对照：`normalize_action` 能识别的每个值，其小写形态必须出现在 `ACTION_ALIASES` 中；
   负对照：无关文本（`待明日观察`）不得命中任何别名。

**v228 新列初始化点全量核查（避免「修一处、再编译 25 分钟发现下一处」）**：
自建扫描器枚举全仓库 `stock_analyses::{Model, ActiveModel}` 初始化点 —— 共 **13 处**，
其中 10 处显式写 `decision_position_state`、3 处靠 `..Default::default()` 补齐（豁免）、
**缺字段且无补齐：0 处**。
（注：扫描器首版未考虑 `..Default::default()`，报了 3 个假阳性 —— 又一例「审计脚本自身会撒谎」，
判据修正后才得到可信结论。）




**i18n**
- `src/i18n/locales/{de,fr,es,ru,hi,ko,ar,zh-TW}.json`（`stockAnalysis` 命名空间）

---

## 8. 第二轮实施（2026-09-14 续）

承接 §7.2 遗留的「未做项」，本轮完成三项。

### 8.1 移除后端「观望 ⇄ 持有」互改

§7.2 第 2 条记录：P1-2 只做了「显式化」，`action` 仍保持互改结果。本轮移除。

**互改实为双向**（§7.2 只记了降级向）：

| 方向 | 位置（`portfolio-mgr.rhai`） | 原行为 | 处置 |
|---|---|---|---|
| 升级向 | 试探仓块 | `base_action = "持有"`（观望 → 持有） | 删该赋值，保留 `position_pct` 赋值 |
| 降级向 | `final_action` 首分支 | `position_pct<=0` 且 base ∈ {买入,增持,持有} ⇒ `"观望"` | 删该分支 |
| 反向耦合① | 仓位对齐段 | `final_action=="观望" && position_pct>0` ⇒ `position_pct=0` | 删（不删则①使试探仓被静默清零） |
| 反向耦合② | trail | `R-204`「零仓位修正」 | 删（编号与 L1206 的「价格信号无信息量」**撞号**） |
| 措辞 | `contradiction_note` | `零仓位修正:{base}→{final}(仓位0%,不可持股)` | 改中性「action修正」，并加 `!risk_downgraded` 免与风控 note 重复 |

**保留**（非「观望⇄持有」互改，属独立语义）：`apply_risk_veto` 的高/极高风险降级（禁止加仓/禁止持仓）；
`partial_low_confidence` / `!trader_data_valid` 时「买入/增持 → 持有」（降级，非中性档互改）。

**语义变化与影响面**（行为变更）：

| 影响 | 说明 |
|---|---|
| 落库 `action` 保真 | 空仓看多的记录由「观望」变为「买入/增持」；两轴可表达「方向看多 + EMPTY」 |
| 价格告警 | 该类记录原被 `is_no_action`(Wait) 过滤 ⇒ 现**会创建**告警（语义更正确：看多应设触发器） |
| 回测 `was_correct` | 判定对象随之改变（「买入 + 空仓」不再按窄幅震荡判对） |
| 展示层 | `resolveDisplayAction` 职责不变（非中性档原样、HOLD/WAIT 按持仓状态派生）⇒ **UI 显示结果不变**，变的是落库保真度 |

`TEMPLATE_VERSION` **43 → 44**（rhai 经 `include_str!` 编译期内嵌，不改版本 DB 不会重种子）。

### 8.2 后端 DTO 下发 `decisionPositionState`

| DTO | 位置 | 改动 |
|---|---|---|
| `StockAnalysisListItem` | `stock_analysis.rs:1159` | 加 `decision_position_state`，并补 `select_only` 列（`FromQueryResult` 缺列会运行时报错） |
| `LatestAnalysisSummary` | `stock_analysis.rs:4504` | 加字段 + 2 处构造点（单数 / 批量） |
| `crossCheck` 快照 | `stock_workflow/hooks.rs` | 写入 `decisionPositionState` |
| 顺手修 | 批量构造点 | `unwrap_or_else(\|\| "uncertain")` → `ACTION_UNAVAILABLE` —— **P0-5 遗漏同族**（单数版已改、复数版漏改，构成「同一缺失两种语义」） |

前端 8 处消费点同步（此前刻意留的「第二个实参」位置换成真实字段）：类型层 3 处（`AnalysisSummary` / `LatestAnalysisSummary` / `RecoCrossCheck`）、
`AnalysisHistoryButton`、`HistoricalAnalysisPanel`、`RecommendationPanel`、`dual-view` 两组件、`StockAnalysisPage`。
两条路径统一为**优先 `decisionPositionState`，仅 `null`（v228 之前的历史行）才退回 `positionPct`**。

### 8.3 i18n 存量翻译：13228 → 0

§7.2 第 1 条更正后的量级（8 语言 13228 项）本轮**全部清零**。

去重后发现需翻译的 **(语言, 值) 对**仅 11465 个（值 p50 = 4 字符），故可批量机器翻译；
关键是给出**术语对照表**约束 —— 「观望 ≠ 持有」必须在所有语言保持区分
（初测无术语表时，8 语言里有 3 种把二者译成同一个词）。

| 语言 | 替换 | 语言 | 替换 |
|---|---|---|---|
| en-US | 490 | ru | 1828 |
| ko | 1827 | hi | 1836 |
| de | 1826 | ar | 1748 |
| fr | 1826 | **合计** | **13207** |
| es | 1826 | | |

**过程中处理的非平凡问题**：

1. **混合值**（已部分翻译、残留中文，如 `[SkillExtension] … 冲突：技能 …`）—— 指令要求「保留已译部分、只译中文片段」；
2. **译文仍含中文 54 条**——分三类，**不是同一问题**：① 机器漏译（括号内中文、示例人名、公司名）⇒ 精修 23 条 + 公司名统一为公开英文名 14 条；② **有意保留**（语言自称、需用户原样输入的命令串）⇒ 走门禁豁免；③ 垃圾数据（见 §8.4）⇒ 删除；
3. **门禁新增豁免机制** `scripts/i18n-untranslated-allowlist.json`（正则匹配**值**，每条须写 `reason`），并加**第 5 组自检对照**：豁免必须**既生效又不过宽**（断言「买入」「风险收敛」不被豁免）—— 否则豁免会变成静默削弱门禁的后门；
4. **棘轮收紧为 0**：基线由 13228 项改为 **0 项**（零容忍），CI 默认模式即任何新增中文值都转红。

### 8.4 本轮新发现

| 现象 | 证据 | 性质 |
|---|---|---|
| 33 处垃圾键：`"components_skill_Ski"` 的值被写成**完整 key 路径** | 11 个 locale 各 3 处；key 恰为 20 字符（截断痕迹） | i18n 抽取脚本 bug；`components_skill*` 在 `src/` **零引用**；已删除 |
| `ja` / `zh-TW` 不被本门禁覆盖 | 门禁 `CJK_LOCALES` 白名单 | 已知边界（门禁自陈）。初筛 ja ≈1.1k / zh-TW ≈1.1k 项含简体专用字（≈7.5%），**但初筛工具本身有误报**（`数`/`号`/`通` 等简繁日共用字污染），精确数需专门审计；已确认存在实例：zh-TW `根据自然语言描述生成的全能助手`、ja `默认权限` |
| 「审计脚本自身会撒谎」再现 | 删除垃圾键时首版断言用 `JSON.stringify(obj).includes(键名)`，而值 `components_skill_SkillABTestResults.…` 恰以该键名**为前缀** ⇒ 断言恒真 | 子串判据把前缀当命中；改为递归比对**键**后才可信 |
| **`check_i18n.py` 从不被 CI 调用** | `.github/workflows/ci.yml` 无 `python3` 步骤 | 又一处「声明的机制 ≠ 运行的机制」。它与未翻译门禁**互补**（查 key vs 查值），缺一都逃逸。已接线 `python3 scripts/check_i18n.py --strict` |
| **`check_i18n.py` 非 strict 模式退出码恒为 0** | 源码 `L183-204`：`missing`/`errors`/`empty` 为真时只有 `if strict: sys.exit(1)`，否则仅打印 ⚠️ | 门禁**退出码撒谎** ⇒ 任何以退出码判定的自动化形同虚设。且脚本 docstring 自称「退出码 1 = 有问题」与实现矛盾。已修 docstring 并把 `package.json` 的 `check:i18n` 改为带 `--strict`（本地口径对齐 CI） |
| **本地 CI 镜像 `scripts/ci-check.mjs` 一条 i18n 门禁都没有** | 该文件头自陈「必须与 ci.yml 对应步骤**逐字同源**，否则本地绿、CI 红」，实测只有 domain/layer 两条 | 本地 `ci-check` 全绿 ⇒ 冒充 CI 绿。已补三条 i18n 门禁，**走 node 直调**（不用 `.sh` 包装，Windows 下 `bash` 未必在 PATH）；python 那条先探测再跑，缺环境则**显式打「跳过 ≠ 通过」** |
| **三道 i18n 门禁此前无测试守卫** | 删任一步骤/降级为 `\|\| true` 都不会有测试转红 | 按 `ci.workflow.test.ts` 既有宗旨补 4 个守卫用例，并做**负对照**：注入「去 `--strict`」+「加 `\|\| true`」⇒ 恰好 2 例转红；还原后 12/12 绿 |

### 8.5 验证

| 门禁 | 结果 |
|---|---|
| `cargo check --workspace --all-targets` | **0 error / 0 warning** |
| `cargo test -p axagent-rt-workflow --test rhai_syntax_check` | **3 passed / 0 failed**，含新增 `portfolio_mgr_has_no_hold_wait_mutual_rewrite`（钉住互改不得被重新引入） |
| `cargo clippy --workspace --all-targets --all-features` | **0 error / 0 warning** |
| `npx tsc --noEmit` | **EXIT 0** |
| `npx dprint check` | **EXIT 0** |
| `npx oxlint --max-warnings=0 src` | **0 warning / 0 error** |
| `node scripts/check-i18n-untranslated.mjs` | **EXIT 0**：8 语言**未翻译 0 项**，豁免 7 项 |
| `python3 scripts/check_i18n.py --strict` | **EXIT 0**（11 语言 key 全对齐；9664 个 `t()` key 全有定义） |
| `node scripts/ci-check.mjs --frontend-only` | **9/9 全绿**（4 静态门禁 + 3 i18n + dprint + tsc） |
| `npx vitest run src/__tests__/ci.workflow.test.ts` | **12/12 通过**（8 原有 + 4 新增 i18n 守卫），且负对照已验证能转红 |
| locale 结构完整性校验 | 11 个文件：key 无新增、仅删除 33 处垃圾键；顶层 key 集合与顺序不变 |