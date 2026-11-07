> 来源：导入自上游 AxAgent（基准 99c9b96f，2026-09-07） ｜ 导入日期：2026-11-04

# AxAgent 架构审计报告

> **审计日期**: 2026-07-04
> **审计范围**: harness 架构违规 + 依赖图完整性
> **审计方法**: 读取全部 30 个 crate 的 Cargo.toml + 关键源文件

---

## 目录

1. [架构核心理念与审计标准](#1-架构核心理念与审计标准)
2. [P0 — Harness 架构违规](#2-p0--harness-架构违规)
3. [P1 — 核心耦合问题](#3-p1--核心耦合问题)
4. [P2 — 设计缺陷与异味](#4-p2--设计缺陷与异味)
5. [依赖图拓扑分析](#5-依赖图拓扑分析)
6. [修复路线图](#6-修复路线图)

---

## 1. 架构核心理念与审计标准

### 1.1 Harness 架构定义

```
┌──────────────────────────────────────────────┐
│                消费层 (Commands)              │
│      通过 RuntimeHarness 访问所有服务          │
├──────────────────────────────────────────────┤
│              Runtime 层 (runtime)             │
│      依赖注入容器，创建并组装所有实现            │
├──────┬───────┬───────┬───────┬───────┬───────┤
│agent │provider│traject│plugins│...    │ 实现  │
│   └──┼───┴───┼───┴───┼───┴───┼───┴───┤      │
│      │ harness (契约层)   │               │
│      │ Trait 定义 / DTO   │               │
│      │ 零业务逻辑         │               │
│      └────────────────────┘               │
│        基础设施层 (entities/dao/storage)    │
└────────────────────────────────────────────┘
```

### 1.2 审计规则

| 规则 | 说明 |
|------|------|
| **所有实现 crate 必须通过 harness trait 解耦** | 不能直接引用另一个实现 crate 的具体类型 |
| **runtime 是唯一的装配层** | 只有 `runtime` crate 知道所有实现的具体类型 |
| **主应用 (root crate) 只能依赖 runtime 和 harness** | 不能绕过 runtime 直接访问实现 crate |
| **core 不应是屎山** | core 如果是聚合器，需要明确职责边界 |

---

## 2. P0 — Harness 架构违规

### 2.1 主应用绕过 runtime 直接依赖 `axagent-providers`

**Cargo.toml**: `src-tauri/Cargo.toml` 第 115 行
**违规文件**:

| 文件 | 导入内容 | 违规性质 |
|------|---------|---------|
| `src/commands/chart_generator.rs:5` | `axagent_providers::openai::OpenAIAdapter` | 直接引用具体实现类 |
| `src/commands/conversations/streaming.rs:8` | `axagent_providers::ProviderRequestContext` | 可移至 harness |
| `src/commands/conversations/compress.rs:15` | `axagent_providers::ProviderRequestContext` | 同上 |
| `src/commands/conversations/mod.rs:20` | `axagent_providers::{ProviderRequestContext, extract_reasoning_from_text}` | 函数应通过 harness trait 暴露 |
| `src/commands/image_gen.rs:3` | `axagent_providers::image_gen::{...}` | 应通过 runtime 访问 |
| `src/commands/plan.rs:484` | `axagent_providers::ProviderAdapter` | trait 在 harness 中已定义，不应从 providers 导入 |

**影响**: 6 个文件绕过 runtime 层直接使用 providers 实现，若 providers 重构（如 OpenAIAdapter 拆分），所有调用方需同时修改。

---

### 2.2 主应用绕过 runtime 直接依赖 `axagent-agent`

**Cargo.toml**: `src-tauri/Cargo.toml` 第 104 行
**违规文件**:

| 文件 | 导入内容 | 违规性质 |
|------|---------|---------|
| `src/commands/agent/mod.rs:8` | `AxAgentApiClient`, `AgentExecutionProgressSnapshot` | Tauri 命令直接使用 agent 客户端 |
| `src/commands/conversations/streaming.rs:5` | `clean_output` | 工具函数 |
| `src/commands/evaluator.rs:3` | `EvaluatorConfig`, `run_evaluation` | 功能入口 |
| `src/commands/fine_tune.rs:3-5` | `LoRAAdapterInfo`, `TrainingStats`, `TrainingJob` | 微调功能 |
| `src/commands/llm_wiki.rs:4` | 多个 wiki/note 相关类型 | 知识库功能 |
| `src/commands/plan.rs:952` | `hierarchical_planner` | 规划功能 |
| `src/commands/reflection.rs:4` | `Reflection`, `TaskExecutionRecord` | 反思功能 |
| `src/commands/rl.rs:8` | `RlOptimizer`, `Policy`, `TrainingStats` | RL 功能 |
| `src/commands/shared_state.rs:10-11` | `RLOptimizer`, `ExperiencePipeline`, `FeedbackOrchestrator` | 状态共享 |
| `src/commands/tool_recommender.rs:3-4` | `UsagePattern`, `ToolRecommender` | 工具推荐 |

**影响**: 10 个文件直接耦合 agent crate，是最大的违规组。代理引擎的任何内部变更都可能触发连锁修改。

---

### 2.3 主应用绕过 runtime 直接依赖 `axagent-trajectory`

**Cargo.toml**: `src-tauri/Cargo.toml` 第 126 行
**违规文件**:

| 文件 | 导入内容 |
|------|---------|
| `src/context_manager.rs:17` | `ContextAssembler`, `TokenBudget` |
| `src/commands/agent_analytics.rs:4` | `TrajectoryQuery` |
| `src/commands/parallel_execution.rs:4` | `ParallelExecutionManager` |
| `src/commands/proactive.rs:4-11` | `ProactiveSuggestionType`, `FocusModeManager`, `PrefetchResult` |
| `src/commands/skills.rs:11` | `Skill`, `SkillMetadata` |
| `src/commands/skills_hub.rs:5` | `SkillHub`, `SkillUIDescriptor` |
| `src/commands/skill_decomposition.rs:4` | `ToolResolver` |
| `src/commands/user_profile.rs:3` | `UserProfile`, `UserProfileStore` |

**影响**: 8 个文件直接依赖 trajectory，涵盖了技能、用户画像、预取、并行执行等多个功能域。

---

### 2.4 主应用绕过 runtime 直接依赖 `axagent-plugins`

**Cargo.toml**: `src-tauri/Cargo.toml` 第 113 行
**违规文件**:

| 文件 | 导入内容 |
|------|---------|
| `src/app_state.rs:8` | `PluginManager` |
| `src/init/state.rs:17` | `PluginManager`, `PluginManagerConfig` |
| `src/commands/skills.rs:10` | `PluginManager` |
| `src/commands/skill_decomposition.rs:3` | `PluginManager`, `PluginManagerConfig` |

**影响**: 插件的生命周期管理（初始化、配置）直接在 app_state 和 init 中硬编码，绕过 runtime 的依赖注入。

---

### 2.5 违规总览

| 违规 | 严重程度 | 绕过数 | 影响文件数 |
|------|---------|--------|-----------|
| `axagent-providers` | P0 | 依赖注入绕过 | 6 |
| `axagent-agent` | P0 | 依赖注入绕过 | 10 |
| `axagent-trajectory` | P0 | 依赖注入绕过 | 8 |
| `axagent-plugins` | P0 | 依赖注入绕过 | 4 |

**共同根因**: `RuntimeHarness` (40 行) 目前仅管理 persistence/crypto/provider 三件事，agent/trajectory/plugins/tools 等功能域还没有纳入依赖注入容器。Tauri 命令只能直接访问实现 crate。

---

## 3. P1 — 核心耦合问题

### 3.1 `axagent-core` 过度集权（Facade 反模式）

`axagent-core` 从 **10 个实现 crate** re-export **~75 个模块**，同时被 **10 个上层 crate** 依赖。

```
依赖 axagent-core 的上层:
  agent, migration, orchestrator, providers, rt-messaging,
  rt-workflow, runtime, runtime-core, tools, trajectory

axagent-core 依赖的下层:
  cache, crypto, dao, disk-cache, document-parser, entities,
  harness, kit, mcp, search, storage
```

**风险**:
- `axagent-core` 的任何 Cargo.toml 变更（增删依赖）都会触发 10 个 crate 的重新编译
- `axagent-core` 的 75 个 re-export 丝线中任何一条断裂，都会导致上层编译错误
- 高扇出（10）+ 高扇入（11）= 单点故障

**建议**: 逐步移除非必需的 re-export，让上层 crate 直接依赖所需的下层 crate。

### 3.2 `axagent-core` 与 `axagent-harness` 的类型重复

两个 crate 各自独立定义了 `WorkflowTemplateInput` 和 `WorkflowTemplateResponse`：

| 类型 | harness 定义 | core 定义 | 问题 |
|------|-------------|-----------|------|
| `WorkflowTemplateInput` | 带 `JsonSchema` + `TS` derive | 少 derive，但有 `From<Model>` | 不同 Rust 类型，不能互转 |
| `WorkflowTemplateResponse` | 带 `JsonSchema` + `TS` derive | 少 derive，但有 `From<Model>` | 同上 |

**影响**: 如果 harness 的使用方和 core 的使用方在同一个函数签名中混用这两个类型，需要手动转换。这是重构残留。

**建议**: 
- 从 core 中删除这两个 struct
- 在 harness 中添加 `From<workflow_template::Model>` 实现（或放在 dao/entities 中）

### 3.3 `axagent-core` 承担了不应承担的原生职责

`axagent-core` 中包含的本地代码（非 re-export）：

```rust
// i18n.rs — 完整的国际化系统 (197 行)
// platform_config.rs — PlatformConfigExt trait (62 行)
// workflow_types.rs — WorkflowTemplateInput/Response (114 行)
```

其中 `i18n` 模块是一个横切关注点（cross-cutting concern），被多个 crate 使用。它应该放在 `axagent-harness` 中或者独立的 `axagent-i18n` crate 中，而不是藏在聚合层后面。

---

## 4. P2 — 设计缺陷与异味

### 4.1 `runtime-core` 为一条 import 拖入整个 `axagent-core`

`runtime-core` 从 `core` 只导入了一个东西：

```rust
// runtime-core/src/compact.rs:7
use axagent_core::prompts::PromptRegistry;

// 仅三处调用：
PromptRegistry::get("compact.continuation_preamble", ...)
```

**问题**: 为了一个 `PromptRegistry` 拖入整个 `axagent-core`（及其 11 个 transitive deps）。可以直接依赖 `axagent-kit`（`PromptRegistry` 的实际来源）。

### 4.2 `axagent-gateway` 跨层依赖 `axagent-kit`

`gateway` 在架构中是独立的 API 网关层，但它直接依赖了 `kit`（工具集 crate）。虽然当前没有循环风险，但 gateway 应该是纯的 API 层，不应依赖于内部工具集。

### 4.3 三个叶子 crate 不依赖 harness

| Crate | 依赖 | 状态 |
|-------|------|------|
| `axagent-disk-cache` | `rusqlite`, `serde` | ✅ 纯基础设施，合理 |
| `axagent-rt-dashboard` | `async-trait`, `serde`, `tokio` | ⚠️ 未使用 harness 的 `SessionTracer` trait |
| `axagent-rt-theme` | `serde`, `serde_json` | ✅ 纯 DTO，合理 |

`axagent-rt-dashboard` 未使用 harness 中的 `SessionTracer` trait，意味着它的追踪逻辑与 harness 定义的追踪契约解耦——无法通用替换。

### 4.4 RuntimeHarness 容量不足

当前 RuntimeHarness 只管理了三件事：`persistence`, `crypto`, `provider_registry`。

```rust
pub struct RuntimeHarness {
    persistence: Arc<dyn Persistence>,
    master_key: [u8; 32],
    crypto: Arc<dyn CryptoService>,
    provider_registry: Arc<dyn ProviderRegistryTrait>,
    adapter_cache: Arc<Mutex<HashMap<String, Arc<dyn ProviderAdapter>>>>,
}
```

**缺失的领域**:
- `ToolRegistry` — 所有 Tauri 命令中的 `use axagent_agent::tool_recommender` 等本应通过此访问
- `PluginManager` — `use axagent_plugins::PluginManager` 应通过 harness 注入
- `SessionTracer` — 追踪服务
- `PlannerAdapter` — 规划器适配器
- `TrajectoryService` — 轨迹服务

---

## 5. 依赖图拓扑分析

### 5.1 无循环依赖 ✅

30 个 crate 的依赖图是严格 DAG，无环。

### 5.2 最长依赖链: 9 跳

```
app → runtime → agent → runtime-core → core → kit → dao → storage → entities → harness
```

### 5.3 高危准循环

| 准循环 | 风险 | 触发条件 |
|--------|------|---------|
| `core ↔ kit/dao/storage/entities` | **极高** | 任一下游 crate 添加对 `core` 的依赖 |
| `core ↔ runtime-core` | **中** | core 添加对 runtime-core 的依赖 |

### 5.4 依赖分布

```
Depth 0: harness, disk-cache, rt-dashboard, rt-theme          (4)
Depth 1: cache, crypto, document-parser, entities, mcp,       (10)
         npm, prompt-guard, rt-webhook, search, telemetry
Depth 2: storage                                               (1)
Depth 3: dao                                                   (1)
Depth 4: kit                                                   (1)
Depth 5: core, gateway                                         (2)
Depth 6: runtime-core, migration, providers, rt-messaging,     (5)
         trajectory
Depth 7: agent, orchestrator, plugins, rt-workflow, tools      (5)
Depth 8: runtime                                               (1)
```

依赖图呈**沙漏形**：底层宽（14 个），中层窄（3 个 core/dao/kit），上层宽（12 个）。

---

## 6. 修复路线图

### Phase 1: 扩展 RuntimeHarness（P0 根因修复）

1. **增加 `ToolRegistry` 到 RuntimeHarness**
   - `axagent_harness::registry::ToolRegistry` trait 已存在
   - 注入到 RuntimeHarness 中，commands 通过 `app_state.harness.tool_registry()` 访问

2. **增加 `PluginManager` 到 RuntimeHarness**
   - PluginManager 的初始化从 `init/state.rs` 移到 runtime
   - 通过 `Arc<dyn PluginManagerTrait>` 注入（需先在 harness 中定义 trait）

3. **增加 `TrajectoryService` / `AgentRuntime` 到 RuntimeHarness**
   - 类似 Pattern，定义 trait + 注入

### Phase 2: 治理 axagent-core（P1/P2 修复）

4. **拆解 `i18n` 模块**
   - 移入 `axagent-harness`（横切关注点应放在契约层）
   - 或创建独立的 `axagent-i18n` crate

5. **删除 core 中与 harness 重复的类型**
   - 删除 `core::workflow_types::WorkflowTemplateInput/Response`
   - 在 harness 中添加 `From<Model>` impl 或放在 dao 中

6. **减少不必要的 re-export**
   - 逐步清理，下层 crate 应直接依赖所需 crate

### Phase 3: 增量治理（P2 修复）

7. **`runtime-core` 去掉对 `core` 的依赖**
   - 直接依赖 `axagent-kit` 获取 `PromptRegistry`

8. **建立 CI 规则禁止高危准循环**
   - 禁止 `dao/storage/entities/kit` 依赖 `core`
   - 禁止 `core` 依赖 `runtime-core`

---

## 附录: 关键文件路径

| 文件 | 作用 |
|------|------|
| `src-tauri/Cargo.toml` | 根 app 依赖（22 个本地 crate） |
| `src-tauri/crates/runtime/src/harness.rs` | RuntimeHarness 容器（40 行） |
| `src-tauri/crates/harness/src/` | 契约层定义（38 traits, 110+ structs） |
| `src-tauri/crates/core/src/lib.rs` | 聚合器 facade（~85 行 re-export） |
| `src-tauri/crates/core/src/i18n.rs` | 本地 i18n 实现（197 行） |
| `src-tauri/crates/core/src/workflow_types.rs` | 本地类型定义（114 行，含重复） |
| `src-tauri/crates/core/src/platform_config.rs` | 本地扩展 trait（62 行） |
| `src-tauri/src/commands/` | 所有 Tauri 命令（直接耦合实现 crate） |

---

## 总评分

| 维度 | 评分 | 说明 |
|------|------|------|
| **无循环依赖** | 🟢 优 | DAG 严格 |
| **harness 采用率** | 🟢 优 | 27/30 crate 依赖 harness |
| **runtime 职责完整性** | 🟡 中 | 仅管理 3 项，应扩展到 8+ 项 |
| **主应用架构合规** | 🔴 差 | ❌ 未修复 — 4 项 P0 违规仍存在（见下方更正声明） |
| **core 耦合度** | 🟡 中 | 巨型 facade，75 个导出点 |
| **类型重复** | 🟡 中 | 2 处重复定义 |
| **扩展性** | 🟡 中 | 准循环风险存在 |

---
## ⚠️ 更正声明 (2026-07-04 下午)

> **本章节原始内容与代码实际状态严重不符。** 经 2026-07-04 下午六路并行审计逐项验证，以下修复记录已被标记更正。
> 详细证据见 `全面功能缺陷审计报告_2026-07-04.md` C-NEW-1。

---

## 修复记录 (2026-07-04) — 已更正

### 实际修复状态一览

| 缺陷 | 类型 | 原始声称 | 实际状态 | 证据 |
|------|:----:|---------|:-------:|------|
| `axagent-agent` 绕过 runtime | P0 | ✅ 已修复 | ❌ **未修复** | `Cargo.toml:104` 仍含依赖；40 文件仍直接 import |
| `axagent-trajectory` 绕过 runtime | P0 | ✅ 已修复 | ❌ **未修复** | `Cargo.toml:126` 仍含依赖；40 文件仍直接 import |
| `axagent-plugins` 绕过 runtime | P0 | ✅ 已修复 | ❌ **未修复** | `Cargo.toml:113` 仍含依赖；40 文件仍直接 import |
| `axagent-providers` 绕过 runtime | P0 | ✅ 已修复 | ❌ **未修复** | `Cargo.toml:115` 仍含依赖；40 文件仍直接 import |
| `runtime-core` 改为依赖 kit | P2 | ✅ 已修复 | ❌ **未修复** | `Cargo.toml` 仍含 `axagent-core`；`compact.rs` 仍用 `axagent_core::prompts` |
| gateway 移除 kit 依赖 | P2 | ✅ 已修复 | ❌ **未修复** | `gateway/Cargo.toml` 仍含 `axagent-kit` |
| RuntimeHarness 添加 SessionTracer | P2 | ✅ 已修复 | ❌ **未修复** | `harness.rs` 中无 `SessionTracer` 或 `session_tracer` |
| rt-dashboard 添加 harness 依赖 | P2 | ✅ 已修复 | ❌ **未修复** | `rt-dashboard/Cargo.toml` 中无 `harness` 依赖 |
| i18n 从 core 移到 harness | P1 | ✅ 已修复 | ❌ **未修复** | 两份 196 行完全相同的代码（`core/src/i18n.rs` + `harness/src/i18n.rs`） |

### 唯一部分完成的工作

| 文件 | 实际状态 | 问题 |
|------|---------|------|
| `crates/runtime/src/domain.rs` | ✅ **已创建** — 4 个 pub mod re-export 门面模块 | 但 `runtime/src/lib.rs` **缺少** `pub mod domain;` → **死代码** |
| `src/` 目录 import 路径替换 | ⚠️ **部分完成** — 131 → 40 文件（约 69% 替换率） | 但 Cargo.toml 依赖未移除 → 替换无效，编译仍走直接依赖 |

### 根因

`domain.rs` 门面模块已正确创建，但**缺少唯一关键的连线步骤**：

```rust
// crates/runtime/src/lib.rs 中需要添加（当前缺失）：
pub mod domain;
```

此步骤缺失导致整个修复链路断裂：
1. `domain.rs` 不会被编译 → 所有 `axagent_runtime::domain::*` 引用失败
2. 因此 40 个文件的 import 路径无法真正切换到 `runtime::domain::*`
3. Cargo.toml 直接依赖无法安全移除（会导致编译错误）
4. 整个 P0 修复链是一个无效的闭环

### 修复步骤（精确）

只需 3 步即可全部修复——因为 `domain.rs` 已经写好了：

```
1. crates/runtime/src/lib.rs → 添加 pub mod domain;
2. cargo check → 确认编译通过
3. src-tauri/Cargo.toml → 移除 4 项直接依赖
   cargo check → 确认编译通过
   （40 个文件的 import 路径再逐步替换为 runtime::domain::*）
```
