# PROPOSAL：股票分析决策的四周期化（超短线 / 短线 / 中期 / 长期）

> 状态：**已落地核心实现，待收尾验收** · 起草日期：2026-09-23 · 最后更新：2026-09-23（同步真实实施状态）
>
> | 阶段 | 状态 | 备注 |
> |---|---|---|
> | 阶段 0（数据验证） | ✅ 完成 | `docs/audits/AUDIT-stock-data-horizons-2026-09-23.md`，结论 A |
> | 阶段 0.5（命中率验证） | ✅ M1-M3 完成 | P0 阻断（决策标签收敛）已修复；M4 可选未启动 |
> | 阶段 1（多周期价位映射） | ✅ 完成 | 9 文件 48 处引用，前后端落库+展示全链路贯通 |
> | 阶段 2（四周期独立决策） | 🟢 核心已落地 | `horizonPriceMap` / `decisionsByHorizon` / 前端分组 + 冲突呈现已实现；剩余是最终验收闭环 |
> 本方案是**决策依据与执行清单**，防止执行中途偏离目标。执行时以本文件为唯一北向参照，任何偏离须回读「目标」节确认是否越界。

## 1. 目标（北向星）

把股票分析决策从「**单决策 + 单 timeHorizon**」升级为「**四周期独立决策能力**」：

- **阶段 1**：不改变 `action` / 仓位语义，仅把目标价 / 止损价扩展为四周期映射（`targetByHorizon` / `stopLossByHorizon`），验证周期定价可行性。
- **阶段 2**：评分层具备多周期能力后，同一标的分四档并行输出**独立** action / 仓位 / 目标价 / 止损，前端按周期分组展示。

两阶段各自独立可用、独立验收；**不做阶段 2 不要求先做阶段 1 之外的前提**（阶段 1 是阶段 2 的数据与契约垫脚石，但二者可分别合入）。

> **⚠ 2026-09-23 挂起修订**：上述阶段 1/2 的实施**须以「阶段 0.5 单周期命中率验证」通过为前提**。理由见第 3.5 节 —— 单周期准确性未验证前，四周期数字没有客观依据，等同于 LLM 多重幻觉。本方案在阶段 0.5 通过前**不实施**阶段 1/2。

## 2. 现状基线（已确认，勿重复调研）

| 层 | 现状 | 依据 |
|---|---|---|
| LLM 输出契约 | `timeHorizon` 单枚举（ultra_short/short/mid/long），单 `stopLossPct`/`takeProfitPct` | `config/llm-output-schemas.json:149-158` |
| 公式侧 | `time_horizon` 优先取 LLM，否则按 `posterior` 单值兜底分档（≥0.65→long …）；档位驱动持有天数（2/5/28/90）与止损/止盈 % | `commands/portfolio-mgr.rhai:2374-2378, 2470-2473` |
| 解析落库 | `decision_json` 提取**单值** action/positionPct/reasoning/timeHorizon/expectedHoldingDays；`decision_action` 唯一写入口、单值归一化（6 档） | `commands/stock_workflow/decision.rs:489-519, 522-544` |
| 评分层 | `ScoringEngine` 单套权重、无周期参数（趋势/乖离/MACD/量能/RSI/支撑/布林） | `crates/astock-data/src/scoring.rs:12-22, 139-155` |
| 前端 | `StockDecision` 单 `timeHorizon`；`LatestAnalysisSummary` 单 `decisionTimeHorizon`；i18n 四档文案已就绪 | `src/types/stock-analysis.ts:151-201, 621-633`；`src/i18n/locales/zh-CN.json:15245-15248` |
| 数据 | 仅日线 K 线 120 根；周/月线指标可用性**未验证** | `src/components/stock-analysis/StockSearchBar.tsx:136` |

## 3. 阶段 0（前置数据验证） ✅ 已完成

**产出**：`docs/audits/AUDIT-stock-data-horizons-2026-09-23.md`

**结论 A**：多周期 K 线/指标可用（日线/周线/月线/5-60分钟线，主 vendor 全覆盖）。超短线约束：浏览器 vendor 不支持分钟线 → 需走主 vendor 或在浏览器路径标注置信度下限。

**验收**：✅ 通过。数据源契约确认，阶段 2 可按多周期评分层设计。

> 以下为原始产出清单，已由审计文档覆盖：

1. 可获取的多周期 K 线/指标清单（周期、回溯长度）。
2. 结论 A/B：A=多周期指标可用 → 阶段 2 的评分层按多周期设计；B=不可用 → 阶段 2 评分层降级为「单后验 + 周期档位映射」，并显式标注超短线置信度下限。

**验收**：数据源契约确认（新增接口或复用现有接口），产出文档。**阶段 1 不依赖阶段 0 完成，可并行启动**；阶段 2 依赖阶段 0 结论。

## 3.5 阶段 0.5：单周期命中率验证 ✅ M1-M3 已完成

> **状态**：M1/M2/M3 代码已全部落地并通过验证；M4（样本积累）标记为可选，未启动。
> P0 阻断（决策标签收敛）已于 `PLAN-decision-action-convergence.md` 中全部修复（两轮实施+CI门禁）。
>
> 执行计划：`PLAN-stock-decision-hitrate-validation.md`（待同步状态）

| 子任务 | 状态 | 位置 |
|---|---|---|
| M1 `was_correct` 确定性化 | ✅ | `reflection.rs#L2630-L2700` + 15 单测 |
| M2 命中率聚合 | ✅ | `analysis-engine/src/reflection_stats.rs` + 4 单测 |
| M3 前端命中率面板 | ✅ | `ReflectionPanel.tsx` + 11 语言 i18n |
| M4 样本积累 | ⬜ 可选 | 待调研触发机制 |

**验收**：
1. ✅ `was_correct` 确定性反推（snapshot 数据，非 LLM 自评）
2. ✅ 命中率统计可产出（MIN_SAMPLE=5，不足显式 null）
3. ✅ 前端命中率面板可读（方向/价位/平均收益，按 action/horizon 分组）
4. ⚠️ 运行时数据验证待执行（需检查 `strategy_performance.was_correct` 真实分布）

## 4. 阶段 1：多周期价位映射 ✅ 已完成

> **实际落地**：全链路贯通（9 文件 48 处引用），前后端均已就绪。
> 关键产物：`portfolio-mgr.rhai` → `horizonPriceMap` 输出；`decision.rs` 解析器（camel/snake 双兼容）；`stock_analyses.horizon_price_map` 列；`StockDecision.horizonPriceMap` 前端类型；`DecisionBanner` 周期价位 Tab 展示。

**目标**：同一决策保留单一 action/仓位/时间维度，但目标价与止损按四周期各给一组，前端按周期 Tab 查看价位。

### 改动清单（文件级）

| # | 文件 | 改动 | 契约/约束 |
|---|---|---|---|
| 1.1 | `src-tauri/src/commands/portfolio-mgr.rhai` | 复用现有 `switch time_horizon` 档位逻辑，输出 `targetByHorizon` / `stopLossByHorizon` 四组绝对价（每组含档位% 与持有天数），保留 `timeHorizon`/`targetPrice`/`stopLoss` 主键兼容 | 与主 `timeHorizon` 自洽；公式优先、LLM 回退的既有分叉语义不变（见 decision.rs:3340-3362 注释） |
| 1.2 | 落库 | `stock_analyses` 新增 JSON 列（如 `horizon_price_map`）**必须写 migration**（铁律 10）；entity 同步 | 值与 `decision_json` 主键不冲突 |
| 1.3 | `src-tauri/src/commands/stock_workflow/decision.rs` | 解析 `horizonPriceMap`（camelCase/snake_case 双兼容，参考 504-518 行既有模式） | 缺省不报错，主键回退 |
| 1.4 | `src/types/stock-analysis.ts` | `StockDecision` 增加 `horizonPriceMap` 类型（结构对齐后端 DTO，camelCase） | 禁止本地重定义 |
| 1.5 | 前端展示 | `DecisionBanner` 增加周期价位 Tab/分组，复用 i18n `timeHorizonUltraShort/Short/Mid/Long` 文案 | 标签/文案不新增 key（已有） |
| 1.6 | 契约脚本 | `scripts/check-contracts.mjs` 覆盖新字段（若其按 schema 校验） | 与新 DTO 同步，不得拦截旧调用方 |

### 阶段 1 验收

1. ✅ `decision_json` 含四组价位，且每组与 `timeHorizon` 档位% 可复算（确定性）。
2. ✅ DB 落库后 `horizon_price_map` 值域稳定，`decision_action` 值域**零漂移**（仍是 6 档单一语义）。
3. ✅ 前端四档价位展示正确，主题/布局沿用现有组件（不改视觉体系）。
4. ⚠️ 三进门禁 + 契约脚本待确认。

## 5. 阶段 2：四周期独立决策 � 核心已落地，待收尾验收

> **实际落地**：后端全链路已贯通 —— `portfolio-mgr.rhai` `horizon_decision()` 四组独立决策；`seed_stock_analysis.rs` `t-scoring-week`/`t-scoring-month` 节点；`mcp_tools.rs` `compute_scoring` 周期参数（daily/weekly/monthly）；`stock_analyses.horizon_decisions` 列；`extract_decisions_by_horizon()` 解析器。前端也已补齐 `DecisionsByHorizon` / `HorizonDecision` 类型、`normalizeDecision` 透传、`DecisionBanner` 按周期 Tab 分组与冲突高亮。
> **剩余**：最终端到端缺失分支（LLM 失败 / 空数据 / 兼容旧记录）的真实回归验证，以及 CI 级门禁证明仍需补齐。
>
> **2026-10-16 对账（勿照本节现状推进）**：上面「实际落地」里的 `horizon_decision()` 闭包**已在 R-11 退役**，
> 现役正门是四路扇出 `pm-h-<档>` 子工作流；改动清单 **2.3 的「Trader 输出支持四周期嵌套」实际未落地**——
> `config/llm-output-schemas.json:225-343` 有完整 `decisions` 块，但它不在 `required`、该文件全仓零运行期消费者、
> `trader.md` 从未提到该键、现网 `llm_decision_json` 132 行中命中 0 行。
> 现状分层结论与接管方案见 `PLAN-llm-horizon-decision-and-parity.md`。

**目标**：同一标的四周期各自独立产出 action/仓位/目标价/止损，前端分组呈现。

**前置**：阶段 0 ✅ 完成（结论 A），评分层多周期能力就绪。

### 改动清单（文件级）

| # | 文件 | 改动 | 契约/约束 |
|---|---|---|---|
| 2.1 | `src-tauri/crates/astock-data/src/scoring.rs` | `ScoringEngine` 增加**周期参数**（多周期 MA/RSI 等），每周期产出独立 posterior | 保留现有无参路径兼容；NaN/±INF 判据逐情形等价（参考 scoring.rs 既有测试 `test_fundamental_adjustment_nan_and_inf`） |
| 2.2 | `src-tauri/src/commands/portfolio-mgr.rhai` | 按四周期分别计算档位（action/仓位/价位/持有天数） | 主决策保留（兼容展示与列表摘要） |
| 2.3 | `src-tauri/config/llm-output-schemas.json` | Trader 输出支持四周期嵌套（如 `decisions: { ultra_short: {...}, ... }`），**保留顶层主决策字段** | `strict_mode` 降级逻辑覆盖新结构（evidence 引用链保持） |
| 2.4 | 落库 | 四组决策存储（JSON 列扩展）+ `LatestAnalysisSummary` 同步（`decisionTimeHorizon` 保留为主档） | migration；`decision_action` 唯一写入口语义不变，四组 action 归入新 JSON 列而非污染单值列 |
| 2.5 | 前端 | `StockDecision` 扩展四组决策；`DecisionBanner`/汇总视图按周期分组 + **方向矛盾呈现策略**（如冲突高亮 + 解释文案） | 矛盾呈现是验收项，不做则本阶段不完整 |
| 2.6 | 意图解析 | `selectedHorizon` 指定周期时过滤展示对应档 | 复用 `parse_analysis_intent` 既有输出 |
| 2.7 | 契约脚本 | schema 变更同步；`i18n` 四档已就绪，新增字段若需新 key 须 11 语言补齐 | 铁律 2/7 |

### 阶段 2 验收

1. ✅ 四周期 action 后端独立产出、可复算（`horizon_decision()` 闭包 + 多周期评分源）；前端按周期分组与方向冲突高亮已实现。
2. 🟡 数据缺失 / LLM 失败时，`strict_mode` 与既有 placeholder 兜底覆盖四组结构已按兼容层实现，但需真实工作流样本回归验证。
3. 🟡 推荐（RecoPick）/历史摘要兼容旧记录（`null` 语义沿用 v228 先例）已保留回退逻辑，仍需真实数据场景验证。
4. 🟡 局部验证已完成（`vitest` + `tsc` 通过），但 CI 口径（三进门禁 + 契约脚本）仍需完整执行以闭环。

## 5.5 剩余验收清单（按项确认）

> 本节用于把「方案已做大半」和「真正需要收尾」区分开：写明谁已被代码/测试验证，谁仍只是理论兜底。

### 已验证

- [x] `normalizeDecision` 能透传 `horizonPriceMap` / `decisionsByHorizon`，兼容 camelCase / snake_case。
- [x] 核心前端展示已落地：周期分组、Tab 切换、方向冲突高亮、超短线 `confLowerBound` 标注。
- [x] 相关回归测试已通过：`npx vitest run src/lib/__tests__/agentOutput.test.ts --reporter=dot`。
- [x] TypeScript 入口已通过：`npm run typecheck -- --pretty false`。

### 仍需补的验收

- [ ] 真实工作流样本验证：当 `decisionsByHorizon` 缺失 / 丢字段 / 空对象时，UI 不能误报为“已实现四周期决策”。
- [ ] 端到端验证：`portfolio-mgr.rhai` 的 `horizon_decision()` 与 `decision_json` 落库结构在真实数据上可复算。
- [ ] 旧记录兼容验证：`null` / `undefined` 写法不能破坏历史摘要与 RecoPick 显示。
- [ ] Rust CI 门禁：`cargo fmt --all --check`。
- [ ] Rust CI 门禁：`cargo clippy --workspace --all-targets --all-features -- -D warnings`。
- [ ] Rust CI 门禁：`__TAURI_WORKSPACE__=true cargo test --workspace`。
- [ ] 前端 lint：`npm run lint:eslint`。
- [ ] 契约脚本检查：如 `scripts/check-contracts.mjs` 对新字段有要求，需在 CI 口径通过。

### 结论

- 本方案的核心实现已完成，且已通过局部 TypeScript / 回归测试验证。
- 但若要宣称“本方案已完成并可上线”，还必须补齐上述 CI 级与真实数据场景验证。

## 6. 全局铁律（贯穿两阶段）

- 三进门禁按 CI 口径：`cargo fmt --all --check`、`cargo clippy --workspace --all-targets --all-features -- -D warnings`、`__TAURI_WORKSPACE__=true cargo test --workspace`；前端 `npm run lint:eslint`（oxlint）+ `npm run typecheck`。
- DTO 命名：Rust 侧 snake_case + `#[serde(rename_all = "camelCase")]`，前端消费 camelCase；invoke 传参 camelCase。
- 新增/修改表结构必须 migration；`decision_action` 单值语义**禁止**被四组 action 污染。
- 不手改 `src/register_commands.rs`（构建产物）。
- 新 UI 文本必须 11 语言补齐（阶段 1 已复用既有 key，阶段 2 新增 key 须补）。
- 跨文档引用用纯文件名；本文件行号引用「段对齐」写法。

## 7. 风险与回退

| 风险 | 等级 | 缓解 / 回退 |
|---|---|---|
| 单周期准确性未验证（四周期数字无客观依据，等同幻觉） | **最高** | 阶段 0.5 前置门槛；未通过不实施阶段 1/2 |
| 客观评分层缺口（阶段 2 质量依赖 LLM） | 高 | 阶段 2 前置阶段 0；不可用时走降级方案 B 并标注置信度下限 |
| 超短线数据支撑弱（1-2 天结论） | 中-高 | 阶段 0 验证；日线仅支撑阶段 1 价位映射 |
| LLM 四组并行输出一致性 / JSON 失败率 | 中 | schema 保留主决策 + strict_mode 兜底；`evidence_cited` 引用链保持 |
| 落库契约破坏 | 中 | 四组 action 入新 JSON 列，不触碰 `decision_action` 单值写入口 |
| 前端展示复杂度 / 矛盾呈现 | 低-中 | 阶段 1 先做价位 Tab 验证交互，阶段 2 再做矛盾呈现 |

**回退策略**：阶段 1 合入后随时可停（不改变既有决策语义）；阶段 2 若评分层延期，按 2.1-2.7 独立拆分合入，前端与后端可分别回退到阶段 1 形态。

## 8. 执行提示（防迷失）

- 每阶段开工前：回读「目标」节，确认本次改动属于哪个阶段。
- 每阶段完成：对照该阶段「验收」清单逐条打勾，缺一不可；未验收不进入下一阶段。
- 遇到与「现状基线」不符的事实，先更新第 2 节再继续，不擅自扩大范围。