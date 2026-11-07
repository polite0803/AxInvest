# AxAgent Crate 精确依赖矩阵（实测）

> 状态：active ｜ 日期：2026-10-31 ｜ 对应版本：v3.0.0


> 生成时间：2026-10-31
> 数据来源：逐个读取 `src-tauri/crates/*/Cargo.toml`（41 个）+ `src-tauri/schema-gen/Cargo.toml` + `src-tauri/Cargo.toml` 根包
> 用途：为仓库拆分提供**硬事实**依据，防止凭印象分组

---

## 1. 完整邻接表（仅列 axagent-* 内部依赖）

下表中「依赖」= 正常 `[dependencies]`；「测试依赖」= `[dev-dependencies]`。
`—` 表示无内部依赖。

| crate | 依赖（normal） | 测试依赖（dev） |
|---|---|---|
| `axagent-harness` | — | — |
| `axagent-entities` | harness | — |
| `axagent-plugin-proto` | — | — |
| `axagent-rt-theme` | — | — |
| `axagent-disk-cache` | entities | — |
| `axagent-agent-command-types` | — | — |
| `axagent-dao` | entities, harness | — |
| `axagent-storage` | entities, harness, kit | dao |
| `axagent-migration` | dao, harness, kit | — |
| `axagent-crypto` | harness | — |
| `axagent-credential` | harness | — |
| `axagent-cache` | harness | — |
| `axagent-search` | disk-cache, entities, harness | — |
| `axagent-npm` | harness | — |
| `axagent-document-parser` | harness | — |
| `axagent-rt-webhook` | harness | — |
| `axagent-device` | harness | — |
| `axagent-telemetry` | harness | — |
| `axagent-agent` | harness | — |
| `axagent-orchestrator` | harness | — |
| `axagent-providers` | harness, kit | — |
| `axagent-mcp` | harness | — |
| `axagent-prompt-guard` | harness | — |
| `axagent-trajectory` | entities, harness, kit | dao |
| `axagent-kit` | harness | — |
| `axagent-runtime-core` | harness | — |
| `axagent-tools` | analysis-engine, astock-data, dao, document-parser, harness, kit, mcp, providers, search | — |
| `axagent-rt-messaging` | crypto, harness, kit | — |
| `axagent-rt-workflow` | crypto, dao, harness, kit, providers, telemetry | dao |
| `axagent-runtime` | agent, cache, crypto, dao, disk-cache, document-parser, entities, gateway, harness, kit, mcp, migration, orchestrator, plugins, prompt-guard, providers, rt-messaging, rt-theme, rt-webhook, rt-workflow, runtime-core, search, storage, telemetry, tools, trajectory | search |
| `axagent-quant` | harness | — |
| `axagent-market-sim` | harness | — |
| `axagent-astock-data` | harness | — |
| `axagent-analysis-engine` | astock-data, dao, entities, harness, trajectory | — |
| `axagent-company-runtime` | analysis-engine, entities, harness | dao |
| `axagent-crdt` | harness | — |
| `axagent-gateway` | harness | — |
| `axagent-notification` | harness | — |
| `axagent-plugins` | harness, plugin-proto | — |
| `schema-gen` | harness | — |
| `axagent-mobile` | **axagent（根包）**（`package = "axagent"`，dep key 为 `axagent_lib`） | — |
| `axagent`（根包） | 除 crdt / mobile / plugin-proto 外的全部 crate | agent(tests) |

> **`axagent-mobile → axagent（根包）` 不是「误耦合」，而是 Tauri 移动端的设计要求。**
> 实测：`crates/axagent-mobile/src/lib.rs` 全文仅 `pub use axagent_lib::*;` 一行，
> 它是 Apple/Android 构建所需的 **staticlib 包壳**，作用就是把整个 app lib 暴露给移动端原生层。
> 而根包 `axagent` 并非薄壳 —— `src-tauri/src/commands/` 有 **269 个 .rs**、`lib.rs` 971 行，
> 是完整的应用装配层（wiring）。
> ⇒ **结论：mobile 属于「应用壳层」，必须与 `src-tauri` 同组，不能作为独立子仓库拆分。**
> （若强行独立，它要么反向依赖整个应用壳，要么需要先把应用壳抽成独立库 crate，成本与收益不成正比。）

---

## 2. 被依赖次数（入度）排名

| crate | 被依赖次数 | 说明 |
|---|---|---|
| `axagent-harness` | **32** | 绝对核心契约，所有拆分方案的中枢 |
| `axagent-entities` | 8 | 数据模型基座 |
| `axagent-kit` | 7 | computer-use 实现层，被 infra/runtime/ai 同时依赖 |
| `axagent-dao` | 6 | 数据库访问 |
| `axagent-plugin-proto` | 1 | 仅 plugins 依赖 |
| `axagent-disk-cache` | 2 | search、runtime |
| `axagent-trajectory` | 1 | 仅 analysis-engine |
| `axagent-analysis-engine` | 2 | tools、company-runtime |

> `axagent-kit` 被 infra（storage/migration）、runtime（rt-*）、ai（providers/trajectory）同时依赖，
> 是**跨层共享实现**。分组时把它归入任何单一层都会制造跨层边，必须单独抽为共享层或明确归入 core 之上的「公共实现层」。

---

## 3. 循环依赖校验

### 3.1 包级环检测

对全部 normal + dev 边做拓扑排序：**无环**（与仓库可编译的事实一致）。

dev 依赖引入的边均不产生新环：

| consumer | dev 目标 | 是否已有 normal 路径 |
|---|---|---|
| root | agent(testing) | 是 |
| company-runtime | dao | 是（→analysis-engine→dao） |
| rt-workflow | dao | 是 |
| runtime | search | 是 |
| storage | dao | 否，但 dao→entities/harness，无回边 |
| trajectory | dao | 否，但 dao→entities/harness，无回边 |

### 3.2 分组级环检测（针对原 Plan A）

原 Plan A 的分组在**包级**无环，但存在两类缺陷：

**缺陷 1：漏分组 5 个 crate**

原 Plan A 只登记了 37 个 crate（5+12+5+7+5+2+1），以下 5 个**未分配**：

| 未分配 crate | 实际依赖 | 建议归属 |
|---|---|---|
| `axagent-agent-command-types` | 无 | core（契约类型） |
| `axagent-agent-macro` | agent-command-types | core（编译期宏，随契约走） |
| `axagent-crdt` | harness | runtime 或独立（协作编辑） |
| `axagent-gateway` | harness | runtime（API 网关） |
| `axagent-notification` | harness | runtime（通知） |

**缺陷 2：层间依赖描述不完整**

原 Plan A 声称 `runtime = core + infra`，实测 runtime 还依赖 **ai / quant / plugins**；
原 Plan A 未提及 `infra → ai`（migration→kit、storage→kit）。

实测分组级依赖（修正后）：

```
core        → （无出边）
plugins     → core
ai          → core
infra       → core, ai          ← 原方案遗漏 infra→ai
quant       → core, infra, ai
runtime     → core, infra, ai, quant, plugins   ← 原方案遗漏 ai/quant/plugins
mobile      → 根包（= 全部）     ← 最大障碍
```

该修正后的分组图**仍然无环**（`ai` 不依赖 `infra/quant`，`quant` 不依赖 `runtime`，`runtime` 不被任何人依赖）。

---

## 4. 结论

1. **Plan A 的拆分在拓扑上可行**（无环），但必须补上 5 个未分组 crate，并修正层间依赖描述。
2. **`axagent-mobile` 属于应用壳层，应与 `src-tauri` 同组**，不拆为独立子仓库（见 §1 说明）。
   附带优化：桌面/CI 的 `--workspace` 会把 mobile 的 staticlib 也钉一遍（对桌面无用），可在 CI 中将其排除以省一次全 app 归档。
3. 推荐把 `axagent-kit` 从「ai 层」提升为**公共实现层**（与 infra 平级或置于 core 之上），因为它被 infra/runtime/ai 三方依赖，归入任何单一层都会制造跨层边。
4. 物理拆仓库前，应先做「**逻辑拆分**」（拆多 workspace + 边界校验脚本），以最小风险暴露所有边界问题。
