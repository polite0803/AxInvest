> 来源：导入自上游 AxAgent（基准 99c9b96f，2026-09-07） ｜ 导入日期：2026-11-04

# Harness 架构全面审查报告

**审查日期**: 2026-06-04
**审查范围**: 全部 20 个 Rust crate
**审查目标**: 识别所有不符合 Harness 架构原则的做法

---

## Harness 架构核心原则（重申）

| 原则 | 说明 |
|------|------|
| **纯接口层** | `crates/harness/` 只包含 trait 接口定义和纯数据 DTO，零业务逻辑 |
| **依赖倒置** | 组件 → harness ← 实现。组件只依赖 harness trait，不依赖具体实现 crate |
| **最小依赖** | harness 及其依赖的 core 应保持最小、最轻量 |
| **DI 容器** | `RuntimeHarness` 作为唯一依赖注入点 |

---

## 审查结果总览

| 级别 | 数量 | 概要 |
|------|------|------|
| **P0 — 架构根本性违规** | 2 | core 是臃肿大杂烩 / harness 包含业务逻辑 |
| **P1 — 依赖倒置违规** | 3 | rt-workflow/runtime-core 直接耦合具体 crate / 缺少关键 harness trait |
| **P2 — 次要违规** | 5 | RuntimeHarness 硬编码/agent 耦合 trajectory/plugins 耦合 npm/webhook 耦合 messaging/trajectory 不参与架构 |
| **总计** | **10** | 见下文逐项分析 |

---

## P0 — 架构根本性违规

### #1 [P0] `axagent-core` 是臃肿的大杂烩（不是轻量类型层）

**严重程度**: ⚠️ 最高

**现状**: `crates/core/src/` 下 80+ 个源文件，涵盖大量实现代码：

```
rag.rs, rag_pipeline.rs       — RAG 实现
mcp_client.rs, mcp_*.rs       — MCP 客户端
browser_automation.rs         — 浏览器自动化
computer_control.rs           — 计算机控制
screen_capture.rs, screen_vision.rs — 屏幕捕获
sandbox_runner.rs             — 沙箱执行
hybrid_search.rs, reranker.rs — 搜索/排序
storage_*.rs                  — 存储管理
memory_forgetting.rs, recall_pipeline.rs — 记忆系统
ui_automation.rs              — UI 自动化
sync_conflict.rs              — 同步
cloud_*.rs                    — 云服务
```

**依赖问题**: `Cargo.toml` 直接依赖重型库：

| 依赖 | 说明 |
|------|------|
| `candle-core`, `candle-nn`, `candle-transformers` | ML 推理框架 |
| `tokenizers` | 分词器 |
| `reqwest` (features: stream, multipart, cookies) | HTTP 客户端 |
| `rusqlite` + `sqlite-vec` + `sea-orm` | 数据库 |
| `keyring` | 系统密钥链 |
| `enigo` | 键盘鼠标模拟 |
| `xcap` (Windows) | 截图 |
| `rmcp` | MCP 协议客户端 |

**连锁影响**: `axagent-harness` 直接依赖 `axagent-core` → 所有依赖 harness 的 crate（agent、gateway、runtime-core、rt-messaging、rt-workflow 等）都**间接拖入全部重型依赖**。这意味着：

```
任何引用 harness trait 的 crate
  → axagent-harness
    → axagent-core (80+ 源文件)
      → candle-core, tokenizers, keyring, reqwest, rusqlite, enigo, ...
```

**预期行为**: `axagent-core` 应只包含：
- `types.rs` — 共享类型/DTO
- `error.rs` — 错误定义
- `constants.rs` — 常量
- `persistence.rs` — persistence DTO（可选）

所有重型实现应拆分到独立 crate 中（如 `axagent-rag`、`axagent-mcp`、`axagent-computer-control` 等）。

**修复建议**:
1. 将所有实现代码从 `core` 迁移到独立的 `axagent-*` crate
2. `core` 只保留纯类型/错误/常量
3. 在 `harness` 中为这些新 crate 定义 trait
4. `harness` 依赖精简后的 `core`

---

### #2 [P0] `harness/provider.rs` 包含业务逻辑实现

**严重程度**: ⚠️ 高 — 直接违背"零业务逻辑"承诺

**文件**: `src-tauri/crates/harness/src/provider.rs`（第 236-330 行）

**违规代码**（trait `ProviderAdapter` 定义之外的 7 个自由函数）：

```rust
pub fn default_version_for_type(...) -> &'static str { ... }       // 版本路由逻辑
pub fn resolve_base_url(...) -> String { ... }                     // URL 解析逻辑
pub fn resolve_base_url_for_type(...) -> String { ... }            // URL 解析逻辑
pub fn resolve_chat_url(...) -> String { ... }                     // URL 构建逻辑
fn resolve_base_url_inner(...) -> String { ... }                   // 私有实现函数
fn has_version_suffix(url: &str) -> bool { ... }                  // 私有辅助函数
fn find_common_version_prefix(...) -> Option<&str> { ... }        // 私有辅助函数
```

**问题**: 这些是**具体的实现逻辑**（URL 拼接、版本探测、字符串处理），不是接口定义。它们在 harness 中意味着：
- harness 不是"纯接口"层
- 这些逻辑无法被替换/mock
- 增加了 harness 的测试负担

**修复建议**:
1. 将这些函数移到 `axagent-providers`（实现层）作为公共工具函数
2. 或者在 `ProviderAdapter` trait 上提供默认方法
3. 或者在新的 `axagent-harness-utils` 辅助 crate 中提供

---

## P1 — 依赖倒置违规

### #3 [P1] `rt-workflow` 直接依赖具体实现 crate

**文件**: `crates/rt-workflow/Cargo.toml`（第 22-23 行）
```toml
axagent-tools = { path = "../tools" }   # rhai_engine
axagent-agent = { path = "../agent" }   # hierarchical_planner
```

**代码违规**（5 处具体引用）：

| 文件 | 行 | 使用的具体类型 | 问题 |
|------|-----|--------------|------|
| `engine.rs` | 238 | `axagent_tools::rhai_engine::RhaiScriptCache` | 结构体字段直接使用具体类型 |
| `engine.rs` | 332 | `axagent_tools::rhai_engine::compile_from_tool_defs()` | 直接调用具体函数 |
| `engine.rs` | 983 | `axagent_tools::rhai_engine::ToolFn` | 闭包内使用具体类型 |
| `engine.rs` | 999 | `axagent_tools::rhai_engine::execute_rhai_ast()` | 直接调用具体函数 |
| `agent_executor.rs` | 652 | `axagent_agent::hierarchical_planner::{HierarchicalPlanner, Plan, ...}` | 直接导入 6 个具体类型/函数 |

**预期行为**: `rt-workflow` 应通过 harness trait 使用 rhai 引擎和规划器：
- 新增 `RhaiEngineAdapter` trait 到 harness（定义编译/执行接口）
- 新增 `PlannerAdapter` trait 到 harness（定义规划/重规划接口）

---

### #4 [P1] `runtime-core` 直接依赖具体实现 crate

**文件**: `crates/runtime-core/Cargo.toml`（第 21-22 行）
```toml
axagent-telemetry = { path = "../telemetry" }
axagent-prompt-guard = { path = "../prompt-guard" }
```

**代码违规**（2 处具体引用）：

| 文件 | 行 | 使用的具体类型/函数 | 问题 |
|------|-----|-------------------|------|
| `conversation.rs` | 8, 203, 312, 1022+ | `axagent_telemetry::SessionTracer` | 结构体字段 + 运行时调用 |
| `session.rs` | 253-254 | `axagent_prompt_guard::GuardConfig::default()` + `PromptGuardPipeline::new()` | 直接实例化安全管线 |

**预期行为**:
- 新增 `SessionTracer` trait 到 harness（定义 `record(event, attrs)` 方法）
- `runtime-core` 通过 `Option<Arc<dyn SessionTracer>>` 注入
- 新增 `PromptGuard` trait 到 harness（定义 `process(input) -> Result` 方法）
- `runtime-core` 通过 `Option<Arc<dyn PromptGuard>>` 注入

---

### #5 [P1] 缺少关键 harness trait（架构缺口）

当前 harness 只定义了 4 个核心 trait：

| 已有 Trait | 实现方 |
|-----------|--------|
| `ProviderAdapter` | `axagent-providers` |
| `Tool` | `axagent-tools` |
| `ToolRegistry` | `axagent-tools::registry` |
| `ProviderRegistry` | `axagent-providers::registry` |
| `Persistence` | re-export from `axagent-core::persistence` |

**缺少的 trait**（导致上述 #3/#4/#7/#8/#9 违规的根因）：

| 缺少的 Trait | 被谁需要 | 谁应实现 |
|-------------|---------|---------|
| `SessionTracer` | `runtime-core` | `axagent-telemetry` |
| `PromptGuard` | `runtime-core`, `agent` | `axagent-prompt-guard` |
| `RhaiEngineAdapter` | `rt-workflow` | `axagent-tools::rhai_engine` |
| `PlannerAdapter` | `rt-workflow` | `axagent-agent::hierarchical_planner` |
| `NpmRegistryService` | `plugins` | `axagent-npm` |
| `MessageGateway` (可选) | `rt-webhook` | `axagent-rt-messaging` |

---

## P2 — 次要违规

### #6 [P2] `RuntimeHarness::new()` 硬编码具体 ProviderRegistry

**文件**: `crates/runtime/src/harness.rs`（第 44-46 行）
```rust
pub fn new(deps: HarnessDeps) -> Self {
    let concrete_registry =
        Arc::new(axagent_providers::registry::ProviderRegistry::create_default());
```

虽然 `with_registry()` 方法提供了注入能力，但 `new()` 作为默认构造器直接创建了具体实现，耦合了 runtime 到 axagent-providers。当需要测试或替换 ProviderRegistry 时，必须显式使用 `with_registry()`，增加了使用心智负担。

**修复建议**: `new()` 也应通过参数接收 ProviderRegistry（或使用工厂 trait）。

---

### #7 [P2] `agent` 直接依赖 trajectory 和 prompt-guard

**文件**: `crates/agent/Cargo.toml`
```toml
axagent-trajectory = { path = "../trajectory" }
axagent-prompt-guard = { path = "../prompt-guard" }
```

**代码违规**:

| 文件 | 使用的具体类型 |
|------|--------------|
| `llm_bridge.rs` | `axagent_trajectory::{LlmJudge, LlmToolProvider, ...}` 等 12+ 类型 |
| `trajectory_recorder.rs` | `axagent_trajectory::{Trajectory, TrajectoryStep, ...}` |
| `session_manager.rs` | `axagent_prompt_guard::{GuardConfig, PromptGuardPipeline}` |

**说明**: agent 对 `runtime-core` 的依赖相对可接受（runtime-core 是共享类型/工具层），但 trajectory（进化学习）和 prompt-guard（安全）应通过 harness trait 抽象。

---

### #8 [P2] `plugins` 直接依赖 `axagent-npm`

**文件**: `crates/plugins/Cargo.toml` + `src/lib.rs`
```rust
use axagent_npm::NpmRegistry;              // 第 19 行
let (name, version) = NpmRegistry::parse_package_spec(source);  // 第 2910 行
let registry = NpmRegistry::new();                               // 第 2970 行
```

**说明**: plugins crate 直接使用 `NpmRegistry` 的具体类型进行 npm 包解析和下载。应通过 `NpmRegistryService` trait 抽象。

---

### #9 [P2] `rt-webhook` 直接依赖 `rt-messaging`

**文件**: `crates/rt-webhook/Cargo.toml`（第 20 行）
```toml
axagent-rt-messaging = { path = "../rt-messaging" }
```

**说明**: webhook crate 直接依赖消息网关的具体实现。如果将来需要切换消息实现，必须修改 webhook 代码。应通过 harness trait 解耦。

---

### #10 [P2] `trajectory` 完全不参与 harness 架构

**文件**: `crates/trajectory/Cargo.toml`
```toml
# 不依赖 axagent-harness
axagent-core = { path = "../core" }   # 唯一 workspace 内部依赖
```

**说明**: trajectory crate 是一个独立王国——不依赖 harness，不实现任何 harness trait。它依赖 heavyweight 的 `axagent-core`（含 candle-core 等 ML 依赖），但自身又完全不参与依赖倒置。其他 crate（如 agent）只能直接硬编码依赖它。

---

## 依赖关系图（当前现状）

```
                     ┌─────────────────────────────┐
                     │       axagent-core           │
                     │  (80+ 文件, candle, reqwest) │
                     └──────────┬──────────────────┘
                                │ depends on
                                ▼
              ┌─────────────────────────────────────┐
              │          axagent-harness             │
              │  (trait + 业务逻辑函数)               │
              └──────┬──────────────┬───────────────┘
                     │              │
         ┌───────────┤      ┌───────┴───────────┐
         ▼           ▼      ▼                   ▼
   axagent-agent  gateway  runtime-core   rt-messaging
   ┌────┬────┐             ┌────┬─────┐     rt-workflow (违规!)
   │    │    │             │    │     │       ┌────┬────┐
   ▼    ▼    ▼             ▼    ▼     ▼       │    │    │
trajectory prompt-core  telemetry prompt-guard tools agent
(违规!)  (违规!)        (违规!)  (违规!)       (违规!)(违规!)
```

**说明**: 虚线框表示直接依赖具体实现的违规关系。理想的依赖图应该是星形放射——所有组件通过 harness 核心 trait 连接，不直接依赖彼此。

---

## 修复优先级建议

| 优先级 | 违规编号 | 修复工作量 | 影响范围 |
|--------|---------|-----------|---------|
| **P0 优先** | #1 core 拆分 | 大（需要拆分 80+ 文件） | 全局编译时间、依赖树 |
| **P0 优先** | #2 harness 清理 | 小（移动 7 个函数） | 无破坏性 |
| **P1 跟进** | #5 新增 trait | 中（定义 4-6 个新 trait） | 需逐个改造实现方和使用方 |
| **P1 跟进** | #3 + #4 + #7 + #8 + #9 | 大（改造 5 个 crate） | 需 #5 完成后逐步执行 |
| **P2 低优** | #6 + #10 | 小 | 局部优化 |

**推荐路线**: #2 → #5 → #1 → #3/#4 → #7/#8/#9 → #6/#10
