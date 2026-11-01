# AUDIT — Metrics 定义关系核查（`MetricType` 是否真重复？）

- 审计日期：2026-09-20
- 审计类型：只读，未改动任何代码
- 审计对象：`agent` / `quant` / `telemetry` 三处 metrics 口径（登记为「可能重复需裁定」）

---

## 一、结论速览

**任务前提需要纠正**：任务描述「agent / quant / telemetry 各有一份 `MetricType`」不成立。

全项目 grep `MetricType`（且排除测试），**名为 `MetricType` 的 enum 实际只有两份**：

| # | 定义文件:行号 | crate | 变体数 | 语义层 |
|---|---|---|---|---|
| 1 | `crates/agent/src/metrics.rs:10` | axagent-agent | 4 | 性能打点类型 |
| 2 | `crates/analysis-engine/src/opc/analytics.rs:8` | axagent-analysis-engine | 9 | 业务 KPI 类型（stock-analysis 域） |

**quant 与 telemetry 没有 `MetricType` 这个类型**：

- `crates/quant/src/metrics.rs:28` 定义的是 `MetricsReport`（**struct**，回测绩效指标聚合 DTO），并非 enum `MetricType`。
- `crates/telemetry/src/metrics.rs`、`crates/telemetry/src/collector.rs` 定义的是 `CostMetrics` / `TraceMetrics` / `AppMetrics` / `WorkflowMetrics` / `MetricsCollector` 等，均**无 `MetricType`**。

因此「裁定三份能否合并」实际要裁定的是**两份**同名 `MetricType`（agent vs analysis-engine/opc），而 quant / telemetry 是命名空间相近、但类型形态与语义完全不同的 DTO/采集器，不与 `MetricType` 冲突。

---

## 二、三处定义完整字段/变体对比

### 2.1 第 1 份 — `crates/agent/src/metrics.rs:10`（性能打点类型）

```rust
pub enum MetricType { Counter, Gauge, Histogram, Timing }
```

- **形态**：enum，`#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]`，实现 `Display`。
- **序列化**：有 serde，**无 `rename_all`** → 序列化为 PascalCase（`"Counter"` / `"Gauge"` / `"Histogram"` / `"Timing"`）。
- **用途**：作为 `MetricsCollector`（同文件 `:96`）内存聚合打点的**类型标记**，挂在 `MetricValue::metric_type`（`metrics.rs:32`），区分 counter/gauge/timing 三种聚合语义。
- **消费范围**：仅 `agent/src/metrics.rs` 内部（构造 `MetricValue`）及自身测试。无跨 crate 消费、无 re-export。

### 2.2 第 2 份 — `crates/analysis-engine/src/opc/analytics.rs:8`（业务 KPI 类型）

```rust
pub enum MetricType { Counter, Count, Gauge, Histogram, Rate, Ratio, Currency, Percentage, Boolean }
```

- **形态**：enum，`#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]`，实现 `Display` + `FromStr`（`analytics.rs:37`）。
- **序列化**：有 serde，**无 `rename_all`** → 序列化为 PascalCase。
- **用途**：作为 `KpiDefinition::metric_type`（`analytics.rs:61`）的业务口径，表示 KPI 数值该按什么语义解读（`Count` 计数、`Currency` 金额、`Percentage` 百分比、`Ratio` 比率、`Rate` 速率、`Boolean` 布尔）。
- doc 头注释**明示归属**：「── 分析指标类型（stock-analysis 域）」（`analytics.rs:5`）。
- **消费范围**：仅 `analysis-engine/opc` 内部 —— `capability_pack_config.rs` 大量以 `super::analytics::MetricType::*` 构造各域包的 `KpiDefinition`（`capability_pack_config.rs:92,98,104,...` 共 30+ 处）。无跨 crate 消费、无 re-export。

### 2.3 第 3 份 — 对齐任务中的「quant / telemetry」

任务假设有第三份，实际查到的相关类型如下（**形态与语义均不同于前两份，不是 `MetricType` 的重复**）：

| 文件:行号 | 类型 | 形态 | 用途 | serde |
|---|---|---|---|---|
| `crates/quant/src/metrics.rs:28` | `MetricsReport` | struct（21 字段，如 `total_return` / `sharpe` / `max_drawdown_pct` 等） | 回测绩效指标**聚合 DTO** | `rename_all = "camelCase"` |
| `crates/telemetry/src/collector.rs:7` | `MetricsCollector` | struct | Prometheus 工作流/LLM/RAG 打点采集器 | 无 serde（运行时采集） |
| `crates/telemetry/src/metrics.rs:9,57,83,113,161,208` | `CostMetrics` / `TraceMetrics` / `SpanMetrics` / `AggregatedMetrics` / `AppMetrics` / `WorkflowMetrics` | struct | 遥测/成本/追踪 DTO | `rename_all = "camelCase"` |

> 另发现 `crates/runtime/src/gateway_metrics.rs:31` 有一份同样功能的**性能打点 enum** `MetricKind { Counter, Gauge, Histogram, Summary }`（命名不同、非 `MetricType`，供 gateway 采集器使用）——它才是与 agent 份语义同层的那一份，但因命名不同未被登记为「重复」。

---

## 三、依赖方向结论

- `axagent-agent`（`agent/Cargo.toml`）是 **consumer 层**，仅依赖 `axagent-harness`；不依赖 analysis-engine。
- `axagent-analysis-engine`（`analysis-engine/Cargo.toml`）依赖 `astock-data / dao / entities / harness / trajectory`；**不依赖 agent**。
- 两份 `MetricType` 分属互不依赖的 crate，**没有 pub use 复用关系**，各自是领域内唯一权威。
- 因此不存在「一份权威 + 一份按需子集/同义」的现有结构——它们现在是**两棵独立叶子**上的同名类型。

---

## 四、是否真重复？

**判定：不是真重复，属不同领域层，不可合并。**

理由（对应每份的语义差异）：

1. **语义层不同**：agent 份 = **运行性能打点**的聚合类型（counter/gauge/histogram/timing）；analysis-engine 份 = **业务 KPI 数值口径**（count/currency/percentage/ratio/rate/boolean）。前者描述「怎么聚合」，后者描述「数值含义」，是两套不相交的概念。
2. **变体集合不可并**：两份交集仅 `Counter/Gauge/Histogram`。agent 独有的 `Timing` 是打点计时专用；analysis-engine 独有的 6 个变体（`Count/Rate/Ratio/Currency/Percentage/Boolean`）是业务口径专用。如果合一成一个 13 变体 enum，那会让打点侧出现毫无意义的 `Currency`/`Boolean`，又让 KPI 侧背上打点概念的 `Timing`——语义被污染。
3. **依赖方向封死**：合并成单一权威（挂 harness 或任一 crate）会破坏 consumer 层唯一 harness 依赖的约束，且 driver 的上下文不一致（同文件 `KpiAvailability` 等 KPI 类型本就属于 analysis-engine 域，不宜上提）。

**Necessary 之间的务实选择**：保持现状即可，倾向**不合并**，仅建议**可选的重命名消歧**（见 §6.3）以降低「登记为重复」的维护噪音。真正同语义可考虑简单正交的是 agent 的 `MetricType` 与 runtime 的 `MetricKind`（均可合并成一个「性能打点类型」），但那属于另一组、且 runtime/agent 已分层清晰，收益有限，本审计不推荐动。

---

## 五、合并方案（若决策机构坚持统一）

**不推荐**，仅记录假如一定要消除两种同名 `MetricType` 的替代路径：

- **以 analysis-engine 份为权威**（它变体更多、消费更广、有 `FromStr` 配置反解析），把 agent 份改 `pub use`。
  - **阻塞点**：`agent` 是 consumer 层，仅允许依赖 `harness`，不能依赖 `analysis-engine`；analysis-engine 是下游，design 上不应被上游 agent 引用。二者之间无合法通道做 `pub use`，除非把该 enum 上提到 `harness`（foundation 层）。
- **上提到 harness**：把 `MetricType`（扩大为两集合并集）放进 `axagent-harness`，agent 与 analysis-engine 各自 `pub use axagent_harness::MetricType`。
  - **反对**：会引入上文变体集语义污染，且 `owner` 信息（KpiDefinition 等业务类型）仍留在 analysis-engine，拆分到 harness 只是把被导出的 enum 上移，无法带走业务上下文。

结论：**两条硬合并路径都得不偿失。** 建议维持现状。

---

## 六、落地建议（可执行，但不改代码）

### 6.1 登记口径更新
在计划/登记表中把该项从「三处 MetricType 待裁定」改为「**两份同名 enum，不同语义层，裁定不合并**」，并附本报告文件名引用。

### 6.2 不做任何代码改动
现状下两份 `MetricType` 各自自洽、无跨 crate 依赖、无重复逻辑，无技术债风险。

### 6.3 （可选，纯命名消歧，降低误判噪音）
若想避免未来审计再次把它误判为重复，可低成本做**其中一份重命名**（不引入依赖）：

- 方案 A（推荐）：`analysis-engine/opc/analytics.rs:8` 的 `MetricType` → 语义化更名为 `KpiType` 或 `KpiMetricType`，保留在 analysis-engine 域（改动面：`analytics.rs` + `capability_pack_config.rs`，约 35 处同 `super::analytics::MetricType::*` 引用）。
- 方案 B：agent 份 `MetricType` → `MetricKind`（与 runtime 侧命名一致），改动面仅 `agent/src/metrics.rs`。

此二撤点仅为降低「同名不同义」的审计噪音，**不改变功能、不引入依赖**；非必需，谨慎起见由决策者拍板。

---

## 参考文件清单

- `crates/agent/src/metrics.rs:10`
- `crates/analysis-engine/src/opc/analytics.rs:8`
- `crates/analysis-engine/src/opc/capability_pack_config.rs`
- `crates/quant/src/metrics.rs:28`
- `crates/telemetry/src/collector.rs:7`、`crates/telemetry/src/metrics.rs`
- `crates/runtime/src/gateway_metrics.rs:31`（`MetricKind`）