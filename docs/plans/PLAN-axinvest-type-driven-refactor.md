# AxInvest 类型驱动设计重构方案

> **状态**: 实施中
> **创建**: 2026-08-19
> **对齐**: 上游 `TYPE_DRIVEN_DESIGN.md` 和 `REFACTOR_PLAN.md`

## 背景

上游 AxAgent 已完成类型驱动设计重构（阶段 A-D），核心框架层已实现：
- 魔法字符串 → 强类型枚举
- JSON 隐式协议 → 结构化 DTO
- `.unwrap()` → 结构化错误处理

AxInvest fork 独有的 crate（analysis-engine、stock-analysis 前端、market-sim 等）仍存在同类问题，需同步改造。

---

## P0：魔法字符串消除（高优先级）

### 涉及模块
- `analysis-engine` crate（Rust 后端）
- `stock-analysis` 前端组件（TypeScript）

### 任务清单

#### P0-1：定义价值投资枚举

**新文件**: `analysis-engine/src/types.rs`

| 枚举 | 变体 | 替换的魔法字符串 |
|------|------|-----------------|
| `MosLevel` | `Adequate`, `Moderate`, `Insufficient`, `None` | "充足", "适中", "不足", "无" |
| `FScoreLevel` | `Excellent`, `Good`, `Fair`, `Weak` | "优秀(7-9)", "良好(5-6)", "一般(3-4)", "弱(0-2)" |
| `MoatLevel` | `Wide`, `Narrow`, `None` | "宽阔", "狭窄", "无" |
| `ValueSignal` | `Undervalued`, `SlightlyUndervalued`, `Fair`, `SlightlyOvervalued`, `Overvalued` | "低估", "合理偏低", "合理", "偏高", "高估" |
| `MosJudgment` | `Adequate`, `Moderate`, `Insufficient`, `None` | mos_judgment 字段的隐式枚举 |

#### P0-2：定义市场状态枚举

| 枚举 | 变体 | 替换的魔法字符串 |
|------|------|-----------------|
| `MarketRegimeType` | `Bull`, `Bear`, `Sideways` | "bull", "bear", "sideways" |
| `VolatilityLevel` | `High`, `Low`, `Normal` | "high", "low", "normal" |

#### P0-3：改造文件列表

| 文件 | 改造内容 |
|------|---------|
| `value_investing.rs` | `ValueMetrics.mos_level/ f_score_level / moat_level / value_signal` 从 `String` → 枚举 |
| `value.rs` | `IntrinsicValue.mos_judgment`, `FScore.grade`, `MoatAssessment.moat_type`, `ValueAssessment.value_judgment / buffett_verdict` 从 `String` → 枚举 |
| `market_regime.rs` | `MarketRegime.regime / volatility` 从 `String` → 枚举 |
| `intent_parser.rs` | `action_type`, `time_horizon` 从 `String` → 枚举（P1） |

---

## P1：前端类型安全（中优先级）

### 涉及模块
- `src/components/stock-analysis/` 所有组件
- `src/stores/feature/stockAnalysisStore.ts`

### 任务清单

| 任务 | 文件 | 改造内容 |
|------|------|---------|
| 定义联合类型常量 | `stockAnalysisStore.ts` | `DecisionAction`, `AnalystStatus`, `StepIdPrefix` 等 |
| 替换硬编码比较 | `DecisionBanner.tsx` | `decision.action === "BUY"` → 使用枚举 |
| 替换硬编码比较 | `AnalystReportGrid.tsx` | `status: "pending" \| "failed" \| "done"` → 联合类型 |
| 替换硬编码比较 | `stockAnalysisStore.ts` | `stepId.startsWith("a-")` 等 → 常量 |
| 替换硬编码比较 | `StockSearchBar.tsx` | `HORIZON_LABELS` → 类型化映射 |

---

## P2：Panic 风险消除（低优先级，随日常开发完成）

| 文件 | 改造内容 |
|------|---------|
| `market-sim/kernel.rs` | `.unwrap()` → `ok_or_else()` + 错误日志 |
| `analysis-engine/serenity.rs` | `detail["key"].as_f64().unwrap_or(0.0)` → 结构化解析 |
| `astock-data/vendors/*` | `item["key"].as_str().unwrap_or("")` → DTO 映射 |

---

## 实施原则

1. **向后兼容**: 枚举序列化时使用中文标签（`#[serde(rename = "充足")]`），确保与现有 JSON 协议兼容
2. **渐进式改造**: 先定义枚举类型，再逐步替换字段，避免大爆炸式改动
3. **测试优先**: 每个枚举改造后运行现有测试确保行为不变
4. **文档同步**: 枚举定义处添加中文文档注释说明业务含义

---

## 验证标准

- [ ] `cargo check` 通过（零错误）
- [ ] `cargo test` 通过（analysis-engine 所有测试）
- [ ] `cargo clippy -- -D warnings` 零警告
- [ ] 前端 `npm run typecheck` 通过
- [ ] 现有 API 响应 JSON 格式不变（向后兼容）
