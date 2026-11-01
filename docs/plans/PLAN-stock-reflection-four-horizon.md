# 股票反思工作流四周期化调整实施计划
**目标：** 在四周期股票决策已经落地后，让反思、客观绩效判定、持久化、命中率统计和前端展示按 `ultra_short` / `short` / `mid` / `long` 分别运行，同时保持历史单周期记录可读、可统计、可回放。
**架构：** 保留现有单值字段作为 legacy 主决策与兼容摘要；新增一个统一的四周期反思结果 JSON 契约，记录每个周期的原始决策、实际行情窗口、成熟度、方向判定、目标价/止损价结果和反思结论。`strategy_performance` 继续一条反思一条主记录，并在现有 `horizon_pnl_json` 扩展为四周期结果载体；统计层把四周期 JSON 展开为样本，旧记录没有该 JSON 时回退到现有单值字段。
**技术栈：** Rust/Tauri、SeaORM/SQLite、Rhai 工作流、React/TypeScript、Vitest、Rust 单元测试、Playwright。
---

## 1. 现状与问题边界

当前反思链路已经能消费原分析和行情快照，但仍有以下单周期假设：

1. [reflection.rs](../../src-tauri/src/commands/stock_workflow/reflection.rs) 只从 `decision_time_horizon` 和 `decision_expected_holding_days` 读取一个原始周期。
2. `compute_market_snapshot()` 只计算一个“分析日到最新交易日”的窗口，不能区分四个周期是否已经成熟。
3. `deterministic_was_correct()` 只接收主 `decision_action` 和单一 `net_return_pct`，不能判定四个独立 action。
4. `reflection-comparator.rhai` 只有一份 `direction_match`、`return_category`、`horizon_mismatch`，会把四周期事实压成一个结论。
5. `stock_reflections` 只有单一 `verdict` / `raw_return` / `holding_days` 等字段；`strategy_performance.was_correct` 也只有一个标量。
6. `reflection_stats` 当前按 `stock_analyses.decision_time_horizon` 聚合，无法消费 `decisionsByHorizon` 中每个周期的真实 action、目标价和结果。
7. [ReflectionPanel.tsx](../../src/components/stock-analysis/ReflectionPanel.tsx) 虽已有 `byHorizon` 命中率表，但该 horizon 目前来自旧单值字段，不代表四周期独立结果。

本计划只解决反思链路四周期化，不重新设计四周期决策输出，也不改变 `decision_action` 的单值兼容语义。

## 2. 目标数据契约

新增统一的 `HorizonReflectionResult` JSON 结构，Rust 侧定义权威 DTO，前端从 `@/types` 引用同一语义：

```json
{
  "ultra_short": {
    "status": "mature",
    "expectedHoldingDays": 2,
    "actualHoldingDays": 2,
    "decision": {
      "action": "买入",
      "positionPct": 0.3,
      "targetPrice": 102.5,
      "stopLoss": 97.8,
      "confidence": 72
    },
    "market": {
      "entryPrice": 100.0,
      "exitPrice": 103.1,
      "returnPct": 3.1,
      "alphaPct": 1.4,
      "targetReached": true,
      "stopLossTriggered": false
    },
    "evaluation": {
      "wasCorrect": 1,
      "directionMatch": true,
      "targetHit": true,
      "mismatch": null
    },
    "reflection": {
      "verdict": "correct",
      "whatWentWrong": null,
      "missedSignals": [],
      "fixForFuture": null,
      "lessonSummary": "..."
    }
  },
  "short": null,
  "mid": null,
  "long": null
}
```

契约约束：

- `status` 只能为 `mature`、`immature`、`unavailable`、`legacy`；未成熟或无行情时 `wasCorrect` 必须为 `null`，不得写成错误。
- `expectedHoldingDays` 优先使用该周期决策自己的 `expectedHoldingDays`，缺失时使用明确的兼容默认值：`ultra_short=2`、`short=5`、`mid=28`、`long=90`。
- `actualHoldingDays` 使用分析日之后的交易日跨度，不使用自然日近似。
- `decision` 从 `stock_analyses.horizon_decisions` 读取；旧记录没有该列内容时仅构造一个 `legacy` 主周期结果。
- `targetHit` 必须按 action 方向计算：买入/增持使用价格达到目标价，卖出/减持使用价格跌至目标价；持有/观望不判定目标命中。
- `wasCorrect` 只由行情事实和周期 action 确定性计算，禁止使用 LLM 的 `verdict` 作为胜负依据。
- 四周期反思结论独立保存；顶层旧 `verdict`、`raw_return`、`alpha_return`、`holding_days` 继续保存主周期结果，供旧 UI、旧脚本和旧记录读取。

## 3. 文件范围与职责

### 后端

- `src-tauri/src/commands/stock_workflow/reflection.rs`
  - 把单一 `MarketSnapshot` 抽象为按 horizon 计算的快照集合。
  - 提取四周期决策、计算成熟度、确定性判定和旧记录回退。
  - 以四周期变量注入反思工作流，并把四周期结果写入 `stock_reflections` 与 `strategy_performance`。
- `src-tauri/src/commands/reflection-comparator.rhai`
  - 输入改为 `horizon_decisions` 与 `horizon_market_facts`。
  - 输出 `by_horizon`，每个周期独立包含方向、收益分类、目标价、时间匹配和数据可用性。
  - 顶层旧字段继续输出主周期摘要，避免旧模板崩溃。
- `src-tauri/src/commands/stock_workflow/reflection_stats.rs`
  - 将四周期结果展开成统计样本。
  - 对四周期结果按 `action`、`horizon` 聚合；无四周期结果时回退旧 join 逻辑。
- `src-tauri/crates/analysis-engine/src/reflection_stats.rs`
  - 扩展 `DecisionPerformanceSample`、`HitrateStats` 和纯函数聚合逻辑，支持 immature/unavailable 排除和 horizon 级 target/return 统计。
- `src-tauri/crates/entities/src/stock_reflections.rs`
  - 增加 `horizon_results_json`（可空），保留旧反思列。
- `src-tauri/crates/entities/src/strategy_performance.rs`
  - 保留既有 `horizon_pnl_json`，明确其 JSON schema；如实现需要分离反思结果，则增加可空 `horizon_results_json`，不得复用 `was_correct` 表达四个结果。
- 现有数据库 migration 注册位置与 migration 文件目录
  - 为新增列写幂等 migration：旧数据库补列，新数据库一次建全；禁止只改 entity。
- `src-tauri/src/commands/stock_workflow/hooks.rs`
  - 检查反思 pending 行创建时是否需要写入四周期决策快照或版本标识，确保后续决策更新不会改变历史反思对象。
- `src-tauri/src/init/services.rs`
  - 调整批量反思调度配置，使按周期到期检查与四周期成熟度一致；不得继续用一个 `expected_days` 阻止其它周期结算。

### 前端

- `src/types/stock-analysis.ts`
  - 增加 `HorizonReflectionResult`、`HorizonReflectionResults`、`HorizonReflectionStatus`、`HorizonHitrateGroup` 类型。
  - `HitrateStats` 增加按周期的成熟样本数、方向命中、目标命中和平均收益字段，同时保持旧字段可选回退。
- `src/components/stock-analysis/ReflectionPanel.tsx`
  - 反思历史行显示四周期状态：已成熟、期中观察、行情不可用、旧记录。
  - 命中率面板按周期展示方向命中率、目标价命中率、平均净收益、平均 alpha 和样本数。
  - 单条反思详情按 Tab 展示四周期输入事实与结论，缺失周期显示“无该周期结果”，不能显示成 0%。
- `src/i18n/locales/*.json`（11 种语言）
  - 新增状态、指标和错误提示 key；zh-CN 先定稿，再同步其余 10 种语言。
- `src/lib/__tests__/agentOutput.test.ts` 或新增反思 DTO 测试文件
  - 覆盖四周期结果透传、旧字段回退、`null`/`undefined`/空对象。

## 4. 分阶段实施任务

### 阶段 A：冻结契约与测试夹具

1. **写 Rust DTO 和纯函数的失败测试**
   - Files: `src-tauri/crates/analysis-engine/src/reflection_stats.rs`、`src-tauri/src/commands/stock_workflow/reflection.rs` 的测试模块。
   - 覆盖：四周期成熟/未成熟/无行情、买入与卖出方向、目标价方向、空 horizon、旧单值记录。
   - Expected red checks：`cargo test -p axagent-analysis-engine reflection_stats` 与定向 reflection 测试至少出现未实现的类型或断言失败。

2. **定义四周期 JSON 版本与字段语义**
   - Files: `src-tauri/crates/analysis-engine/src/reflection_stats.rs`、`src-tauri/crates/entities/src/stock_reflections.rs`、`src-tauri/crates/entities/src/strategy_performance.rs`。
   - 使用 `#[serde(rename_all = "camelCase")]`，Rust 字段保持 snake_case。
   - 明确 `schemaVersion`，建议初始值 `1`；解析器对未知字段忽略，对缺失字段按状态回退。

3. **先补数据库 migration 和 fixture**
   - Files: 现有 migration 注册文件、新 migration 文件、反思/分析引擎测试夹具。
   - 为 `stock_reflections.horizon_results_json` 增加 nullable 列；确认 `strategy_performance.horizon_pnl_json` 既有列的 fixture 已覆盖。
   - 测试 migration 在空库、旧库和重复执行三种场景下均成功。

### 阶段 B：实现四周期事实计算

4. **提取每周期决策并固定默认持有期**
   - File: `src-tauri/src/commands/stock_workflow/reflection.rs`。
   - 读取 `horizon_decisions` 的 camelCase/snake_case 两种形态。
   - 对每个 key 解析 action、expectedHoldingDays、targetPrice、stopLoss、confidence；解析失败只将该周期标记为 `unavailable`，不能阻断其它周期。

5. **把单快照计算改为四周期快照计算**
   - File: `src-tauri/src/commands/stock_workflow/reflection.rs`。
   - 复用一次 K 线读取，按四个目标交易日索引生成窗口，避免每周期重复请求。
   - 目标日尚无足够交易数据时生成 `immature`；行情源失败时生成 `unavailable`；已到目标日才生成 `mature`。
   - 每个周期独立计算 entry/exit、net return、alpha、最大回撤、target progress、target reached、stop loss triggered。

6. **实现四周期确定性判定**
   - File: `src-tauri/src/commands/stock_workflow/reflection.rs`。
   - 将 `deterministic_was_correct()` 拆为接收 `HorizonDecision + HorizonMarketSnapshot` 的纯函数。
   - `mature` 才允许返回 `Some(0|1)`；`immature`、`unavailable`、中性 action、非法收益统一返回 `None`。
   - 增加 NaN、Infinity、负价格、目标价缺失、卖出方向等边界测试。

### 阶段 C：接入反思工作流与持久化

7. **扩展 comparator 的输入输出**
   - File: `src-tauri/src/commands/reflection-comparator.rhai`。
   - 新增 `horizon_decisions`、`horizon_market_facts`、`horizon_statuses` 输入。
   - 将现有分类函数按 horizon 逐项执行，输出 `by_horizon`，顶层保留主周期兼容摘要。
   - 明确 `immature` 只能输出观察性描述，不能输出“决策错误”。

8. **扩展 reflection prompt**
   - File: `src-tauri/agency_experts/stock-analysis/reflection.md`。
   - 输入说明改为“四周期事实表”，要求逐周期输出 `mature/immature/unavailable` 的不同结论。
   - `reflection` 增加 `by_horizon`，每个成熟周期必须有 verdict、错因、被忽视信号、改进建议；期中观察只允许记录待观察项。
   - 仍禁止输出新的交易决策，禁止把某一周期结论冒充全局结论。

9. **写入四周期反思结果并保留 legacy 字段**
   - File: `src-tauri/src/commands/stock_workflow/reflection.rs`。
   - `stock_reflections.horizon_results_json` 保存原始四周期结果和 schemaVersion。
   - 顶层旧列从“主周期”确定性投影；没有四周期数据的旧调用方沿用原路径。
   - `strategy_performance.horizon_pnl_json` 写入每周期绩效事实；`was_correct` 只保存主周期兼容值，不能将四个值压成一个新的伪总值。
   - 反思失败时仍保留 pending/running/failed 状态和可诊断错误。

10. **改造批量反思调度**
    - Files: `src-tauri/src/commands/stock_workflow/reflection.rs`、`src-tauri/src/init/services.rs`。
    - 一条 pending 反思允许部分周期成熟：成熟周期结算，未成熟周期保留 `immature`，不重复生成已完成周期的 LLM 反思。
    - 只有四周期均已成熟或用户明确结束观察时，才把整条反思标记为最终 completed；否则使用可识别的 partial/observing 状态。
    - 每次重跑保持幂等，不能重复插入 `strategy_performance` 或重复应用 lesson。

### 阶段 D：统计与前端

11. **按四周期展开命中率统计**
    - Files: `src-tauri/src/commands/stock_workflow/reflection_stats.rs`、`src-tauri/crates/analysis-engine/src/reflection_stats.rs`。
    - 将每个 `mature` horizon 展开为一条 `DecisionPerformanceSample`；`immature` / `unavailable` 不进方向命中率分母。
    - 样本门槛继续使用 `MIN_SAMPLE=5`，但按 horizon 独立应用；全局汇总只作为附加信息，不替代分周期统计。
    - 旧记录没有 `horizon_results_json` 时，通过 `original_analysis_id` 使用 legacy 主周期字段，归类为 `legacy`，并在 DTO 里带数据源标识。

12. **扩展前端类型与反思面板**
    - Files: `src/types/stock-analysis.ts`、`src/components/stock-analysis/ReflectionPanel.tsx`。
    - 统计表每行显示 horizon、成熟样本、方向命中率、目标价命中率、平均净收益、平均 alpha；`null` 明确显示样本不足/暂无数据。
    - 反思详情使用既有四周期文案和 Tab 模式；不新增嵌套卡片，不把空周期渲染为 0 值。
    - 保留现有按 action 的表格和旧记录查看能力。

13. **补齐 11 语言文案**
    - Files: `src/i18n/locales/ar.json`、`de.json`、`en-US.json`、`es.json`、`fr.json`、`hi.json`、`ja.json`、`ko.json`、`ru.json`、`zh-CN.json`、`zh-TW.json`。
    - 增加四周期反思状态、成熟度、目标价命中、期中观察、legacy 数据源和加载失败文案。
    - 用现有 locale 保序工具更新，强制 LF；运行 locale key 检查。

### 阶段 E：验收与回归

14. **运行纯函数和引擎定向测试**
    - Commands:
      ```powershell
      Set-Location D:\OneManager\AxInvest\src-tauri
      $env:__TAURI_WORKSPACE__ = "true"
      cargo test -p axagent-analysis-engine reflection_stats
      cargo test -p axagent-analysis-engine reflection
      ```
    - Expected: 四周期成熟度、方向判定、目标价方向、旧记录回退测试全部通过。

15. **运行前端定向测试与类型检查**
    - Commands:
      ```powershell
      Set-Location D:\OneManager\AxInvest
      npx vitest run src/lib/__tests__/agentOutput.test.ts --reporter=dot
      npm run typecheck -- --pretty false
      npm run lint:eslint
      ```
    - Expected: 四周期透传、旧数据兼容、React 类型和 oxlint 全部通过。

16. **运行工作流/浏览器验收**
    - Files: 新增或修改 `e2e/knowledge.spec.ts` 之外的股票分析专用 E2E 文件，建议命名 `e2e/stock-reflection-four-horizon.spec.ts`。
    - 覆盖：四周期全部成熟、部分成熟、行情失败、旧记录、反思失败重试、刷新后结果保持。
    - Assertions：UI 不把缺失周期显示成“正确/错误”，不把 `null` 显示成 0%，且按周期切换后 action、收益、结论互不串线。
    - Command:
      ```powershell
      Set-Location D:\OneManager\AxInvest
      npm run test:e2e -- e2e/stock-reflection-four-horizon.spec.ts
      ```

17. **执行仓库 CI 级门禁**
    - Commands:
      ```powershell
      Set-Location D:\OneManager\AxInvest\src-tauri
      cargo fmt --all --check
      cargo clippy --workspace --all-targets --all-features -- -D warnings
      $env:__TAURI_WORKSPACE__ = "true"
      cargo test --workspace

      Set-Location D:\OneManager\AxInvest
      npm run typecheck -- --pretty false
      npm run lint:eslint
      node scripts/check-contracts.mjs
      ```
    - Expected: 所有命令退出码为 0；不得只依赖局部测试宣称完成。

## 5. 验收标准

- 四个 horizon 都能独立得到 `mature` / `immature` / `unavailable` 状态，缺失一个不阻断其它三个。
- 已成熟周期的 `wasCorrect`、目标价命中和收益均由行情事实确定性计算；LLM verdict 只能解释，不能决定胜负。
- 期中观察不进入方向命中率分母，不会污染胜率；数据不可用不被伪造为 0% 或错误。
- `stock_reflections` 和 `strategy_performance` 可保存四周期结果，旧列和旧记录仍可读取。
- `reflection_stats` 按 horizon 独立执行 `MIN_SAMPLE=5`，前端能区分样本不足、无数据和 legacy。
- 批量反思幂等：重复调度不重复插入绩效、不重复提取 lesson、不重复调用已完成周期。
- 11 种语言 key 完整，TypeScript、Rust 格式、Clippy、工作区测试、契约脚本和股票反思 E2E 均通过。

## 6. 不做事项

- 不修改 `decision_action` 的单值语义，不把四个 action 写入该列。
- 不用 LLM verdict 直接覆盖 `was_correct`。
- 不把四周期简单平均成一个“总命中率”作为唯一指标。
- 不为未成熟周期制造收益、目标命中或错误判定。
- 不删除历史字段、历史反思记录或现有 legacy fallback。
- 不在本计划中重新设计评分引擎、决策 UI 或交易执行流程。

## 7. 实施顺序与提交边界

建议按以下独立提交边界推进，每个边界都能单独验证：

1. `test`：四周期 DTO、快照成熟度和确定性判定失败测试。
2. `feat`：migration、实体字段和四周期结果契约。
3. `feat`：四周期行情快照与 comparator。
4. `feat`：反思工作流持久化、幂等和批量调度。
5. `feat`：统计 DTO、前端面板和 11 语言文案。
6. `test`：Rust/前端/E2E/契约门禁全量验收。

每一步完成后先运行对应的定向命令，再进入下一步；不要在全量门禁失败时用局部通过结果替代结论。

## 8. 可直接执行的批次计划

### 批次 0：建立基线与冻结兼容边界

- **前置条件：** 当前四周期决策 proposal 的 Rust、前端和契约门禁保持绿色。
- **动作：** 记录 `stock_analyses`、`stock_reflections`、`strategy_performance` 的现有列结构；保存一组含四周期决策的 fixture、一组只有旧单值决策的 fixture，以及一组缺行情 fixture。
- **产出：** 三类基线 fixture、旧字段回退规则、四周期 JSON schemaVersion=1。
- **验证：** 先运行现有 reflection_stats 与 agentOutput 测试，确认基线在未改代码前通过。
- **放行条件：** 能明确区分“旧记录兼容失败”和“四周期新增逻辑失败”；否则不得进入数据库改造。

### 批次 1：先完成纯函数和数据契约

- **修改范围：** entities、analysis-engine DTO/聚合、reflection.rs 中的解析和成熟度纯函数。
- **动作顺序：** 先写红测，再实现四周期 key 解析、默认持有期、成熟度状态和方向/目标价判定；暂不接真实工作流。
- **验证命令：**
  ```powershell
  Set-Location D:\OneManager\AxInvest\src-tauri
  $env:__TAURI_WORKSPACE__ = "true"
  cargo test -p axagent-analysis-engine reflection_stats
  cargo test -p axagent --lib commands::stock_workflow::reflection
  ```
- **放行条件：** 四周期逐项判定、NaN/Infinity、卖出目标价、期中观察、旧记录回退全部通过；任何一个周期异常不能阻断其它周期的测试样本。

### 批次 2：数据库迁移与持久化闭环

- **修改范围：** migration、stock_reflections entity、strategy_performance entity、reflection.rs 写入路径和测试夹具。
- **动作顺序：** 先新增 nullable 列和 migration，再写入 `horizon_results_json`；最后实现旧顶层字段从主周期投影。
- **验证命令：**
  ```powershell
  Set-Location D:\OneManager\AxInvest\src-tauri
  cargo test -p axagent-analysis-engine trade_intent
  cargo test -p axagent --lib commands::stock_workflow::reflection
  ```
- **放行条件：** 空库、旧库、重复执行 migration 均可用；反思失败不丢 pending 状态；重复批处理不重复写绩效和 lesson。

### 批次 3：行情快照和 Rhai comparator

- **修改范围：** `reflection.rs`、`reflection-comparator.rhai`、`reflection.md`。
- **动作顺序：** 一次读取 K 线，按四个交易日窗口生成事实；再把四周期事实注入 comparator；最后更新 prompt 的逐周期输出约束。
- **验证重点：**
  - 数据只够 ultra_short 时，其他周期必须是 `immature`，不能被当作负样本。
  - 行情失败时必须是 `unavailable`，不能用 0% 代替。
  - comparator 顶层旧字段仍有主周期摘要，旧模板变量不报 `VARIABLE_NOT_FOUND`。
- **验证命令：**
  ```powershell
  Set-Location D:\OneManager\AxInvest\src-tauri
  cargo test -p axagent --lib commands::stock_workflow::reflection
  cargo test -p axagent --test '*rhai*'
  ```
- **放行条件：** Rhai 语法检查和反思纯函数测试通过，且四周期结果可从同一输入 fixture 确定性重放。

### 批次 4：统计 API 与前端反思面板

- **修改范围：** `reflection_stats.rs`、`stock-analysis.ts`、`ReflectionPanel.tsx`、11 个 locale 文件。
- **动作顺序：** 先扩展后端 DTO 和旧数据回退，再扩展 TS 类型，最后接入面板 Tab/表格；UI 不直接解析未经类型约束的 JSON 字符串。
- **验证命令：**
  ```powershell
  Set-Location D:\OneManager\AxInvest
  npx vitest run src/lib/__tests__/agentOutput.test.ts --reporter=dot
  npm run typecheck -- --pretty false
  npm run lint:eslint
  ```
- **放行条件：** UI 能区分 `mature`、`immature`、`unavailable`、`legacy`；null 不显示为 0；按 horizon 切换不会串用其它周期 action 或收益。

### 批次 5：真实工作流与全量门禁

- **修改范围：** E2E fixture、股票反思 E2E、必要的契约脚本断言。
- **动作：** 先用固定 fixture 验证四周期全成熟、部分成熟、旧记录和行情失败，再执行真实工作流；真实工作流只用于验证接线，不用随机 LLM 输出作为确定性断言。
- **验证命令：**
  ```powershell
  Set-Location D:\OneManager\AxInvest
  npm run test:e2e -- e2e/stock-reflection-four-horizon.spec.ts
  node scripts/check-contracts.mjs

  Set-Location D:\OneManager\AxInvest\src-tauri
  cargo fmt --all --check
  cargo clippy --workspace --all-targets --all-features -- -D warnings
  $env:__TAURI_WORKSPACE__ = "true"
  cargo test --workspace
  ```
- **放行条件：** 上述命令全部退出码为 0，且没有因“样本不足”或“行情不可用”生成错误命中率。

### 批次间停止规则

- 批次 1 的契约或纯函数失败：停止，不进入 migration。
- 批次 2 出现列缺失、重复写入或旧记录读取失败：停止，不接入 UI。
- 批次 3 出现 Rhai 变量缺失或 0% 假行情：停止，不接入统计。
- 批次 4 出现 i18n key 缺失、空周期误报或 TypeScript 错误：停止，不跑真实 E2E。
- 批次 5 任一 CI 门禁失败：只能修复对应批次，修复后重新执行该批次及其后的全部门禁，不能用局部测试替代全量结论。
