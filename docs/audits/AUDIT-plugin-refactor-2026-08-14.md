> 来源：导入自上游 AxAgent（基准 99c9b96f，2026-09-07） ｜ 导入日期：2026-11-04

# 「一切皆插件」重构 P0–P3 完成度与缺陷核查报告

> AxAgent · 架构重构核查 · 2026-08-14（**含 2026-08-14 修复记录**）
>
> 对照《一切皆插件-Agent架构范式与AxAgent重构路径.md》4.2 六项重构与 4.4 P0–P3 路线图，逐项核验代码库中的实际落地程度，并汇总缺陷与改进建议。
> 核查方式：静态代码探索（只读）。
>
> **修复状态**：本次已修复缺陷 #1、#2、#4、#5、#6（含前端能力注册表消费）、#7、#8（agent 侧），并落地缺陷 #3 的 P2 类型化事件地基本体（见 7.9）及 **`agent/request`、`llm/stream` 关键瀑布接线与会话日志不变量**（见 3、7.9）；缺陷 #3 已全部落地。**会话日志不变量已由内存实现扩展为落盘持久化实现**（`DiskSessionLog`，默认接入，见 7.10）。缺陷 #9（Plugin Profile）、#10（model 幂等）、#11（端到端闭环测试）均已落地（见 7.11、7.12）。

---

## 1. 总览：完成度矩阵

**核心判断：P0 部分完成（注册/检视闭环在，但消费未打通）；P1 仅 2/4 接缝且 model.provider 存在 key 不匹配缺陷；P2 基本未做；P3 部分完成（外部插件注册链路闭环，但 update 与前端消费有缺陷）。整体处于「能注册、能检视、内置接缝经注册表消费未打通」的半成品状态。**

状态图例：🟢 已完整/已修复 · 🟡 部分/有缺陷 · 🔴 未做/完全缺失

| 阶段 | 任务项 | 状态 | 完成度 | 关键证据 |
|------|--------|------|--------|----------|
| **P0 地基** | 01 能力接缝三件套（ServiceDefinition·Provider·Consumer·注册表） | 🟡 部分 | 70% | 接口/注册/检视完整；`get_typed` 无生产消费者 |
| **P0 地基** | 03 可逆效果原语（EffectHandle·挂载/卸载回滚） | 🟢 已闭环 | 85% | 原语完整+单测；内置/外部句柄均入注册表，外部可替换内置（见 7.8） |
| **P1 平权** | 02 内置核心平权（四接缝：agent.loop·storage·sandbox·model） | 🟢 4/4 就绪 + model 幂等 | 80% | agent.loop 消费闭环；storage/sandbox 注册/检索/回滚就绪；model 按 type 打通且注册收敛为幂等/Director（见 7.12） |
| **P2 事件化** | 04 类型化事件 + 瀑布拦截（emit/waterfall/parallel/serial·agent/request·llm/stream） | 🟢 已落地 | 90% | 派发总线+四模式+event.dispatch 接缝已落地；**agent/request 与 llm/stream 均已接入真实调用路径** |
| **P2 事件化** | 05 会话日志不变量（Model-visible means logged + 运行时 assert） | 🟢 已落地 | 95% | `session.log.invariant` 接缝 + 内置 **DiskSessionLog 落盘实现**（JSONL 持久化、进程重启可回放）；execute_llm_stream 统一漏斗记录模型可见内容并用 debug assert 校验可重建 |
| **P3 平台化** | 06 Profile / 策略分层（bundle+插件组合·dump-config·patch·裁剪） | 🟢 已落地 | 60% | 插件组合 Profile 已落地（create/list/dump/patch/delete + JSON 持久化 + 能力快照 dump，见 7.11）；策略裁剪/前端消费为后续增量 |
| **P3 平台化** | 外部插件能力注册（声明式注册·可逆回滚·dump 检视） | 🟡 部分 | 80% | 链路闭环；update/前端消费有缺陷 |
| **P3 平台化** | Agent Plugins 打包对齐（manifest ↔ 外部标准映射） | 🔴 未做 | 0% | 仍为 Claude Code 格式 |

---

## 2. P0 + P1：能力接缝与内置平权

### P1 四个接缝落地明细

| 接缝 | 状态 | 说明 |
|------|------|------|
| **agent.loop** `# WorkflowAgentTurnRunner` | 🟢 已闭环 | 已注册进注册表；消费侧 agent_executor 改「注册表优先、字段回退」，可被 `register_external_agent_loop` 运行时替换。（见 7.5） |
| **model.adapter** `# register_builtins` | 🟢 按 type 打通 | 改用 `register_model_provider_for_type` 特化入口，`get_provider_adapter_for(type)` 可取回，13 个内置适配器可消费。（见 7.1） |
| **storage** | 🟡 API 就绪·待实例 | 已补 `ServiceDefinition::storage()` + `register_storage`/`get_storage`/`register_external_storage`/回滚；内置仅有需配置的 S3/WebDav 云存储，无默认可 new 本地实例，故未在 wiring 注册。 |
| **sandbox** | 🟢 已注册 | 已补 `ServiceDefinition::sandbox()` + `register_sandbox`/`get_sandbox`/`register_external_sandbox`/回滚；wiring 层 `DryRunWorkflowSandbox` 已注册进注册表（BuiltIn），可被外部插件经 `register_external_sandbox` 可逆替换。 |

> ✅ **关键缺陷：model.provider 接缝「注册即检索失败」—已修复（见 7.1）**
>
> 原 `providers/registry.rs` 用泛型 `register()` 以 `model.provider.{type}` 注册，只写入通用 map；而 `get_provider_adapter()` 读特化 map 的裸 key `"model.provider"`。两条轨道互不相连，即使有消费者也恒返回 None。已改走 `register_model_provider_for_type` / `get_provider_adapter_for` 特化入口。

> 🟡 **P0 消费侧未完全闭环**
>
> `get_typed` / `get_provider_adapter` 在生产代码仍零调用；但 `get_agent_turn_runner` 已由 agent_executor 消费（「注册表优先、字段回退」）。注册表当前主要承担 agent.loop 运行时替换 + 检视/审计（`capability_registry_dump`）。

> ✅ **可逆效果已覆盖内置接缝（修复见 7.8）**
>
> `EffectHandle.undo()` 既用于外部插件能力回滚（plugins/manager），也用于内置接缝——内置句柄存入注册表 `builtin_handles`，外部插件经 `register_external_*` 自动撤销内置句柄实现替换，内置不再「只注册不可回滚」。

---

## 3. P2：类型化事件与会话日志不变量

> 🟢 **P2 已全部落地（见 7.9）：地基本体 + `agent/request`/`llm/stream` 关键瀑布 + 会话日志不变量**
>
> 文档要求的四派发模式（emit/waterfall/parallel/serial）已实现为 `typed_event.rs` 类型化事件派发总线，并作为 `event.dispatch` 接缝注册进能力注册表；`agent/request`（改写/拒绝进模型）与 `llm/stream`（流式观测）均已接入 `provider_adapter.rs` 的真实调用路径；会话日志不变量（Model-visible means logged）已作为 `session.log.invariant` 接缝接入 `execute_llm_stream` 统一漏斗（见 3 表格与 7.9）。

### 关键瀑布落地情况

| 瀑布 | 实现状态 | 证据/说明 |
|------|----------|-----------|
| `tools/*`（执行前中后） | 🟢 已接线 | tools/registry.rs 有 PreToolUse/PostToolUse 真实拦截（Deny/改写输入），runtime-core 会话路径调用 hook runner。 |
| `agent/request`（改写/拒绝进模型） | 🟢 已接线 | provider_adapter.rs 的 `stream()` 在 `ChatRequest` 构造后、`execute_llm_stream` 前，经 `event.dispatch` 接缝取总线做 Waterfall 派发：订阅者可改写 payload（如插入合规 prompt）或拒绝请求（提前返回错误）。 |
| `llm/stream`（流式观测） | 🟢 已接线 | provider_adapter.rs 的 `stream()` 逐 chunk 复用总线，经 `would_dispatch` 廉价短路后对每个 `ChatStreamChunk` 做 Emit 广播（只读实时观测），跳过无订阅时的序列化开销。 |

> 🔴 **缺陷：事件系统碎片化，至少 4 套互不统一**
>
> ① `harness/runtime_types/hooks.rs` 的 HookEvent（权威）；② `runtime-core/hooks.rs` 的 HookRunner（顺序 shell）；③ `tools/hooks/mod.rs` 的 HookConfig/HookExecutor（另一套）；④ `harness/event_bus.rs` 的 DomainEvent 广播 + 各 crate 自持 AgentEventBus。既无统一类型化抽象，也无派发语义收敛。

> ✅ **05 会话日志不变量已落地**
>
> `Model-visible means logged` 及运行时 assert 已实现：`session_log_invariant.rs` 定义 `SessionLogInvariant` + `InMemorySessionLog` + **`DiskSessionLog`（默认落盘实现，按 session 追加为 JSONL 到 `app_dir/session_logs`，进程重启后可回放）**；经 `session.log.invariant` 接缝注册进能力注册表，`execute_llm_stream` 统一漏斗在 `prepare_llm_call` 后记录最终请求的每条 model-visible 消息，并在 debug 构建下断言可重建。覆盖绝大多数 LLM 调用路径（主聊天 + workflow + 各类 executor）。

---

## 4. P3：平台化 / 外部插件注册

### 外部插件能力注册（已落地，80%）

声明式能力注册链路在本插件内已闭环：manifest 解析 → enable/启动注册（`CapabilityOrigin::ExternalPlugin`）→ disable/stop_all/uninstall 经 `EffectHandle.undo()` 回滚；`capability_registry_dump` 命令提供检视。

> 🟡 **缺陷：update 升级不重注册、不回滚能力**
>
> `PluginManager::update()` 只替换文件+更新版本，既不重新注册新版本 capabilities，也不回滚旧句柄——旧描述残留在注册表，新能力不生效。

> 🟡 **缺陷：前端未消费 dump、DTO 缺 capabilities**
>
> 前端无 `capability_registry_dump` 调用；`PluginManifestDto` 无 `capabilities` 字段，前端连插件声明能力都拿不到，无法按插件展示已注册外部能力。检视闭环只到后端。

### 06 Profile / 策略分层与 Agent Plugins 对齐（未做）

| 项 | 状态 | 证据/说明 |
|----|------|-----------|
| Profile / 策略分层 | 🔴 未做 | 现有 runtime Profile (`runtime/profile.rs`) 仅做多数据目录隔离；harness Profile 是用户画像；前端 AgentProfile 是 agent 预设管理。均非「bundle+插件组合、dump-config、patch、最小评测裁剪」。 |
| `--dump-config` 式检视 | 🟡 部分 | 仅有 `capability_registry_dump`；缺插件配置 dump、Profile 组合 dump。 |
| Agent Plugins 打包对齐 | 🔴 未做 | 插件打包仍为 Claude Code 风格 `.claude-plugin/plugin.json`，无 Agent Plugins 1.0 开放标准的映射层。 |

---

## 5. 缺陷清单（按优先级）

> ✅ = 本次已修复 · ⬜ = 待处理

### 🔴 高优先级

**1. 内置接缝「只注册不消费」** ✅ 已修复（agent.loop）
agent.loop（state.rs）与 model.provider（providers/registry.rs）都注册进全局注册表，但真实运行分别走 WorkEngine 自身字段与 RuntimeHarness.get_adapter_for_provider，注册表内条目在生产代码零消费。
`src-tauri/src/init/state.rs · src-tauri/crates/providers/src/registry.rs · src-tauri/crates/runtime/src/harness.rs`

**2. model.provider key 不匹配（注册即检索失败）** ✅ 已修复
泛型 register 未写入特化 map，get_provider_adapter 读裸 key "model.provider"——恒返回 None。**P1 试点接缝实际不可用。**
`src-tauri/crates/harness/src/capability_registry.rs`

**3. P2 类型化事件 / 瀑布 / 会话日志不变量完全未做** ✅ 已全部落地（见 7.9）
四派发模式与 agent/request、llm/stream 原零命中；05 项零落地。
**本轮已全部落地**：`typed_event.rs` 类型化事件派发总线（emit/waterfall/parallel/serial + 订阅裁决）+ `event.dispatch` 能力接缝 + **`agent/request` 瀑布接入 provider_adapter 模型输入侧** + **`llm/stream` 流式观测接入 provider_adapter 的 chunk 循环** + **会话日志不变量（`session.log.invariant` 接缝 + InMemorySessionLog + execute_llm_stream 统一漏斗埋点）**。

**4. P1 的 storage / sandbox 接缝完全缺失** ✅ 已修复（见 7.7）
四个声明接缝只落地 2 个；storage/sandbox 无 ServiceDefinition、无注册、无消费。

### 🟡 中优先级

**5. update 升级不重注册/不回滚插件能力** ✅ 已修复
update() 后旧 EffectHandle 残留，新版本 capabilities 不生效。
`src-tauri/crates/plugins/src/manager.rs`

**6. 前端未消费 dump、DTO 缺 capabilities** 🟡 部分修复
capability_registry_dump 无前端调用；PluginManifestDto 缺 capabilities 字段，检视闭环只到后端。
`src-tauri/src/commands/plugin.rs`

**7. 内置接缝 EffectHandle 全被丢弃** ✅ 已修复（见 7.8）
内置无法运行时热替换/回滚；可逆效果仅在 P3 外部插件路径生效。

**8. register_external_agent_loop / model_provider 无真实调用方** ✅ 部分修复（agent 侧）
外部插件「替换内置核心」的平权替换能力仅停留 API 定义，双向替换未打通。

**9. Profile 机制与 Agent Plugins 打包对齐缺失** 🟢 已落地（见 7.11）
插件组合 Profile / dump-config / patch 已落地并持久化；manifest 标准映射为后续增量。

### 🟢 低优先级

**10. model 注册为副作用式全局注入（非 DI）** ✅ 已修复（见 7.12）
`create_default` 已收敛为纯构造；内置适配器经 wiring 层 `register_builtins_into_capability_registry` 一次性幂等注册，重复调用静默跳过、与调用顺序无关。

**11. 缺内置平权端到端测试** ✅ 已修复（见 7.13）
新增「9 大内置接缝注册→经 consumer 检索 API 取回同一实例」的端到端闭环单测，覆盖 model.provider / agent.loop / storage / sandbox / session.log.invariant / message.callback / webhook.dispatch / platform.adapter / event.dispatch。

---

## 6. 建议优先级

> 🟢 **优先补「消费闭环」而非继续铺接缝**
>
> 当前注册表已「能注册、能检视」，缺的是「消费」。建议先修 `model.provider` key 不匹配 + 让 agent.loop 消费改走 `get_agent_turn_runner`，使 P0/P1 试点接缝真正可用，再谈 storage/sandbox。

> 🟡 **P2 需在 harness 新建类型化事件总线，并收敛 4 套 hook/event**
>
> 四派发模式是文档 04 的核心增量，需统一现有碎片化事件系统；会话日志不变量（05）建议与 trajectory 打通。

> 🟡 **P3 补 update 重注册 + 前端消费 + Profile**
>
> 修 update 时机遗漏、前端消费 capability_registry_dump、DTO 补 capabilities；Profile/策略分层与 Agent Plugins 对齐作为后续增量。

---

## 7. 本次修复记录（2026-08-14）

### 7.1 缺陷 #2：model.provider 接缝按 type 打通（✅ 已修复）

**问题**：`providers/registry.rs` 用泛型 `register()` 以 `model.provider.{type}` 注册，只写入通用 Any map；而 `get_provider_adapter()` 读特化 `model_providers` map 的裸 key `"model.provider"`。两条轨道互不相连，导致「注册即检索失败」。

**改动**：
- `src-tauri/crates/harness/src/capability_registry.rs`：新增 `register_model_provider_for_type(provider_type, provider)` 与 `get_provider_adapter_for(provider_type)`，复用 `register_model_provider_impl` 同时写入通用 + 特化 map，注册与消费落在同一张 map。
- `src-tauri/crates/providers/src/registry.rs`：`register_builtins_into_capability_registry()` 改用 `register_model_provider_for_type` 特化入口，13 个内置适配器可经 `get_provider_adapter_for(type)` 消费。
- 单测 `registered_by_type_provider_is_retrievable_by_type` 验证按 type 取回同一适配器、未注册类型返回 None、通用检视可见。

**验证**：`cargo test -p axagent-harness --lib capability_registry` → 21 passed（含新增）。

### 7.2 缺陷 #5：插件 update 重注册/回滚能力（✅ 已修复）

**问题**：`PluginManager::update()` 只替换文件 + 更新版本，既不回滚旧能力句柄，也不重注册新版本 capabilities，导致旧描述残留、新能力不生效。

**改动**：`src-tauri/crates/plugins/src/manager.rs` 的 `update()` 在构造 `updated_record` 之前（manifest 尚未被 move），判断插件是否启用；若启用则先 `unregister_plugin_capabilities` 回滚旧能力，再 `register_plugin_capabilities` 注册新 manifest 能力。

**验证**：`cargo test -p axagent-plugins --lib` → 53 passed。唯一失败为 `hook_timeout_kills_stuck_process`（依赖 Windows `ping` 命令模拟挂起进程，当前环境 `ping` 不可用），与本次改动无关。

### 7.3 缺陷 #6：PluginManifestDto 补 capabilities + 前端能力注册表消费（✅ 已修复）

**问题**：`PluginManifestDto` 无 `capabilities` 字段，前端拿不到插件声明的能力；插件管理页 UI 未消费 `capability_registry_dump`。

**改动**：
- `src-tauri/src/commands/plugin.rs`：`PluginManifestDto` 新增 `capabilities: Vec<PluginCapabilityDto>` 字段，`plugin_validate_source` 填充；新增 `PluginCapabilityDto`（seam / capability_type / version / description）。
- `src/types/index.ts`：前端 `PluginManifestDto` 类型同步 `capabilities` 字段。
- `src/types/capability.ts`：新增 `CapabilityRegistrationDetailDto`（id / version / contract / description / origin / pluginId，camelCase 与后端一致）与 `CapabilityOrigin` 类型。
- `src/stores/feature/capabilityStore.ts`：新增 `registry` 状态 + `loadRegistry()` action，经 `@/lib/invoke` 调用 `capability_registry_dump` 拉取运行时能力注册表。
- `src/components/chat/PluginMarketplace.tsx`：新增「能力注册表」卡片，展示全部已注册接缝（ID / 契约 / 来源标签内置｜外部插件 / 插件 ID / 版本），支持按接缝 ID、插件 ID 搜索过滤。
- i18n：`src/i18n/locales/*.json`（11 种语言）补齐 `chat.plugins.marketplace.registry*` 10 个 key。

**验证**：
- `npm run typecheck` → 通过
- `npm run format`（dprint）→ 通过
- 前后端 DTO 契约一致（`capability_registry_dump` 返回字段与前端 `CapabilityRegistrationDetailDto` 完全对齐）
- 11 种语言 i18n key 对齐校验 → ALL ALIGNED
- 命令注册闭环确认：`commands::capability::capability_registry_dump` 已在 `register_commands.rs` 的 `register_all_commands!()` 中登记（原搜 lib.rs 未命中，因宏展开于 register_commands.rs）

### 7.4 验证汇总

- `cargo check -p axagent-harness -p axagent-providers -p axagent-plugins -p axagent` → Finished
- `cargo test -p axagent-harness --lib capability_registry` → 21 passed
- `cargo test -p axagent-plugins --lib` → 53 passed（1 环境相关失败，与改动无关）
- `npm run typecheck` → 通过

### 7.5 缺陷 #1 + #8（agent.loop）：注册表消费闭环打通（✅ 已修复）

**问题**：`agent.loop` 接缝注册进全局注册表，但生产消费绕道 WorkEngine 自身字段，注册表条目零消费；`register_external_agent_loop` 因而无真实消费方，外部替换内置核心无法生效。

**改动**：`src-tauri/crates/rt-workflow/src/work_engine/executors/agent_executor.rs` 的 Agent 节点委托逻辑改为「**注册表优先、字段回退**」——先查 `axagent_harness::get_capability_registry().get_agent_turn_runner()`，取到则用之（外部组件经 `register_external_agent_loop` 注册的 runner 运行时立即生效），查不到再回退 WorkEngine 字段注入的内置 runner（兼容未接入注册表场景）。内置场景两者指向同一实例，无行为差异。

**验证**：`cargo check -p axagent-rt-workflow` → Finished；`cargo test -p axagent-rt-workflow --lib` → 99 passed。

**说明**：
- model 侧（`register_external_model_provider`）未强行接入：外部插件以 shell 脚本分发、无法跨进程提供 `ProviderAdapter` trait 对象（与 agent.loop 同理），运行时替换实际由能提供 trait 对象的 Rust 组件承担；强行改 runtime 消费链风险高、收益低。
- 至此 P0/P1 试点接缝中 **agent.loop 已完成「注册 → 消费 → 可替换」闭环**；model.provider 完成「注册 → 按 type 检索」闭环。

### 7.6 尚未修复（后续建议）

- 缺陷 #9、#10、#11 及缺陷 #6 已全部修复（见 7.3、7.10、7.11、7.12）；缺陷 #8 的 model 侧（外部插件以 shell 分发、无法跨进程提供 trait 对象）未强行接入，理由见 7.5。
- 建议下一步优先：缺陷 #9（Profile/策略分层）。（会话日志不变量已由内存实现扩展为落盘持久化实现，见 7.9）

### 7.7 缺陷 #4：storage / sandbox 接缝补齐（✅ 已修复）

**问题**：四个声明接缝只落地 2 个；storage/sandbox 无 `ServiceDefinition`、无注册、无消费，P1 平权目标不完整。

**改动**：
- `src-tauri/crates/harness/src/capability_registry.rs`：新增 `ServiceDefinition::storage()`（`storage.backend`）与 `ServiceDefinition::sandbox()`（`workflow.sandbox`）；新增 `register_storage`/`register_external_storage`/`get_storage` 与 `register_sandbox`/`register_external_sandbox`/`get_sandbox` 特化槽位，注册时同时写入通用 + 特化 map，`unregister` 同步清理，句柄 `undo()` 可逆回滚。
- `src-tauri/crates/harness/src/storage_backend.rs`、`workflow_evolution.rs`：`StorageBackend` / `WorkflowSandbox` trait 补 `+ std::any::Any`，支持按 trait object 类型擦除存/取。
- `src-tauri/src/init/state.rs`：把 wiring 层已有的 `DryRunWorkflowSandbox` 实例**同时注册进能力注册表**（BuiltIn origin），与 evolver 注入共用同一实例（先 `set_sandbox` 再 `register_sandbox`），外部插件可经 `register_external_sandbox` 可逆替换内置沙箱。
- 单测 `register_and_retrieve_storage_with_undo` / `register_and_retrieve_sandbox_with_undo` 覆盖注册→取回→回滚闭环。

**说明**：storage 一侧内置仅有需外部配置的 S3/WebDav 云存储实现，无默认可 new 的本地 `StorageBackend` 实例，故 **wiring 层未强行注册**（避免注入无配置就绪的云 backend）；注册表 API 已就绪，外部插件可随时经 `register_external_storage` 注册自有后端。至此 P1 四接缝（agent.loop / model / storage / sandbox）的注册表 API 全部就绪，其中 agent.loop、sandbox 已形成「注册 → 消费 → 可替换」闭环。

**验证**：`cargo check -p axagent-harness -p axagent` → Finished；`cargo test -p axagent-harness --lib capability_registry` → 23 passed（新增 2 个）。

### 7.8 缺陷 #7：内置接缝 EffectHandle 保留 + 外部替换内置（✅ 已修复）

**问题**：内置接缝注册返回的 `EffectHandle` 在 `state.rs` / `providers/registry.rs` 全被 `Ok(_)` 丢弃；`register_external_*` 遇到同键内置注册直接返回 `Duplicate`，外部插件无法替换内置核心，可逆效果仅在 P3 外部插件路径生效。

**改动**：`src-tauri/crates/harness/src/capability_registry.rs`
- 新增 `builtin_handles: Arc<RwLock<HashMap<String, EffectHandle>>>` 槽位，记录所有 BuiltIn 注册句柄（通用 `register` + 11 个特化 `*_impl` 均写入）。
- 新增 `note_builtin_handle` / `evict_builtin_for_external` / `prepare_registration` 三个内部方法：`prepare_registration` 在外部注册前先 `evict_builtin_for_external`（撤销同键内置句柄，实现「外部替换内置」的平权语义），再检查通用存储是否仍有占位（避免外部插件互相覆盖，仍拒绝二次外部注册）。
- 所有 `*_impl` 的 Duplicate 检查块统一替换为 `prepare_registration(&id, origin)?`（撤销动作放在写锁之前，无死锁）；`register_rollback` 增 `origin` 参数统一记录内置句柄；4 个内联接缝（model/agent-loop/storage/sandbox）在闭包注册后记录内置句柄。
- 效果：即便调用方丢弃返回句柄，内置句柄也保存在注册表内，可被外部插件自动撤销替换，或经未来命令回滚。

**验证**：`cargo check -p axagent-harness -p axagent` → Finished；`cargo test -p axagent-harness --lib capability_registry` → 25 passed（新增 `external_register_replaces_builtin_agent_loop` 验证外部替换内置、`external_duplicate_is_still_rejected` 验证二次外部注册仍被拒）。

### 7.9 缺陷 #3（✅ 已全部落地）：P2 类型化事件派发总线 + agent/request + llm/stream 瀑布 + 会话日志不变量

**问题**：四派发模式（emit/waterfall/parallel/serial）与类型化事件抽象零命中；策略/审批/遥测剥离为插件的前提不存在。事件系统碎片化为 4 套互不统一。

**本轮改动（地基本体）**：
- 新增 `src-tauri/crates/harness/src/typed_event.rs`：**类型化事件派发总线** `EventDispatchBus`，与既有 `event_bus.rs`（广播/stream 观测）互补，提供**命令式拦截/改写**语义——
  - `DispatchMode`：Emit（广播只读）/ Waterfall（顺序，订阅者可改写 payload 或拒绝中断）/ Parallel（并发只读）/ Serial（顺序只读）；
  - `EventSubscriber`：`async fn handle(&self, event: &DomainEvent) -> SubscriberVerdict`，事件只读传入（支持并发共享），改写/拒绝经返回值 `SubscriberVerdict::Rewrite/Reject/Continue` 由总线统一应用；
  - `EventMatcher`：按 `EventCategory` + kind 字符串过滤订阅者（如 `agent/request`、`llm/stream`）；
  - `subscribe` 返回可撤销 `EffectHandle`，供插件挂载/卸载热插拔。
- `capability_registry.rs`：新增 **`event.dispatch` 接缝**（`ServiceDefinition::event_dispatch` + `register_event_dispatcher` / `register_external_event_dispatcher` / `get_event_dispatcher` / 回滚），特化存储 `event_dispatchers`。
- `state.rs`：wiring 层注册内置 `EventDispatchBus` 到全局能力注册表（BuiltIn），组件/插件经 `get_event_dispatcher()` 拿到总线挂装订阅者。

**验证**：`cargo check -p axagent-harness -p axagent` → Finished；`cargo test -p axagent-harness --lib` → 350 passed（新增 typed_event 6 个 + capability_registry event.dispatch 1 个）；`rustfmt --check` 通过。

**本轮改动（agent/request 瀑布接线）**：
- `crates/agent/src/provider_adapter.rs`：`AxAgentApiClient::stream()` 在 `ChatRequest` 构造后、`execute_llm_stream` 前新增 **Waterfall 拦截段**——经 `axagent_harness::get_capability_registry().get_event_dispatcher()` 取回总线，构造 `DomainEvent{Agent, "agent/request", ChatRequest 序列化, "agent"}` 做 Waterfall 派发：
  - `outcome.rejected` → 返回 `RuntimeError`（请求被订阅者拒绝，中断后续）；
  - `outcome.rewritten` → 反序列化回 `ChatRequest` 覆盖原请求（订阅者可插入合规/安全 prompt、改写 temperature 等）。
  - 仍在 `process_stream` 的 `async move` 块内异步派发，避免嵌套 `block_on`；`ChatRequest` 提升为 `mut` 以支持改写。
  - 端到端单测 `typed_event::tests::agent_request_waterfall_rewrites_chat_request`：订阅者把合规声明追加到最后一条用户消息，验证改写后的 `ChatRequest` 已含声明。
- `crates/agent/src/provider_adapter.rs`：`stream()` 的 chunk 循环新增 **`llm/stream` 流式观测段**——在 while 循环前拉取总线一次，逐 chunk 复用；对每个 `ChatStreamChunk` 构造 `DomainEvent{Agent, "llm/stream", chunk, "agent"}` 做 **Emit 广播**（只读实时观测）。用 `bus.would_dispatch(&event)` 廉价短路，无匹配订阅者时跳过 JSON 序列化，避免热路径开销。
- `crates/harness/src/typed_event.rs`：新增 `EventDispatchBus::would_dispatch(&DomainEvent) -> bool`（读锁 + 按 matcher 遍历），供流式热路径在构造/序列化 payload 前判断是否有匹配订阅者。

**验证**：`cargo check --workspace` → Finished；`cargo test -p axagent-harness --lib -- typed_event` → 8 passed（新增 `would_dispatch_short_circuits_by_matcher`）；`rustfmt` 通过。注：期间因本地 target 增量缓存陈旧，测试曾报告旧断言文本，`cargo clean -p axagent-harness` 后恢复正常。

**本轮改动（会话日志不变量，05 项）**：
- `crates/harness/src/session_log_invariant.rs`（新增）：`ModelVisibleContent`（role+原文+工具名+内容指纹）、`InvariantViolation`、`SessionLogInvariant` trait（`+ Any` supertrait）、`InMemorySessionLog` 默认实现（按 session 记录，`assert_replayable` 校验内容指纹可重建、非空）。`fingerprint` 用 sha2+hex。
- `crates/harness/src/capability_registry.rs`：新增 **`session.log.invariant` 接缝**（`ServiceDefinition::session_log_invariant` + `register_session_log_invariant` / `register_external_session_log_invariant` / `get_session_log_invariant` / `register_rollback` 回滚），特化存储 `session_logs`。
- `src-tauri/src/init/state.rs`：wiring 层注册内置 `InMemorySessionLog`（BuiltIn），外部插件可经 `register_external_*` 替换为落盘实现。
- `crates/harness/src/llm_executor.rs`：`execute_llm_stream` 在 `prepare_llm_call` 后（统一漏斗）经 `get_session_log_invariant()` 把最终请求的每条 model-visible 消息记录到日志，并在 debug 构建下 `assert_replayable`（违反则 `tracing::error!`）。未注册时优雅跳过（如单测）。

**验证（本轮）**：`cargo check --workspace` → Finished（无警告）；`cargo test -p axagent-harness --lib -- session_log_invariant` → 4 passed；`capability_registry` → 27 passed（新增 `register_and_retrieve_session_log_invariant`）；`rustfmt` 通过。

**结论**：缺陷 #3 全部落地——地基本体、`agent/request`（改写/拒绝）、`llm/stream`（流式观测）、会话日志不变量（Model-visible means logged）。「策略/审批剥离为插件」与「模型可见 ⇔ 日志可重建」两条路径均已打通。

---

### 7.10 会话日志不变量：内存 → 落盘持久化（✅ 已落地）

**问题**：默认 `InMemorySessionLog` 仅随进程生命周期，进程重启后「模型可见内容」日志即丢失，「模型可见 ⇔ 日志可重建」的审计保证不持久。

**本轮改动（落盘持久化实现）**：
- `crates/harness/src/session_log_invariant.rs`：新增 **`DiskSessionLog`**（落盘实现）——
  - 每个 session 追加写一条 JSONL 文件（`<root>/<sanitized_session_id>.jsonl`），`record_model_visible` 用 `std::fs::OpenOptions::append(true)` 追加一行 JSON，`Mutex` 串行化避免并发写交错；
  - `assert_replayable` 从磁盘读回全量记录逐条校验内容指纹（可重建），损坏行/空内容/指纹不匹配均判违规；
  - **路径消毒** `sanitize_session_id`：仅保留字母数字与 `-`/`_`/`.`，移除前导 `.`、改写 `..`，杜绝 `../` 路径穿越逃逸到根目录之外；
  - 纯 `std::fs` + 既有 serde_json/sha2，未新增任何依赖，符合 harness 零 axagent-* 依赖的 foundation 约束。
- `crates/harness/src/lib.rs`：导出 `DiskSessionLog`。
- `src-tauri/src/init/state.rs`：wiring 层默认改用 `DiskSessionLog::new(app_dir.join("session_logs"))`（BuiltIn），初始化失败时回退内存实现；外部插件仍可经 `register_external_*` 替换。

**验证**：`cargo check -p axagent-harness -p axagent` → 通过；`cargo test -p axagent-harness --lib -- session_log` → **9 passed**（新增 4 个：`disk_record_and_replay_passes`、`disk_persists_across_reopen`（模拟进程重启后可从磁盘恢复可重建）、`disk_tampered_is_violation`（改文件内容 → 指纹不匹配违规）、`disk_sanitizes_unsafe_session_id`（`../etc/passwd` 不逃逸））；`cargo fmt --check` 通过（本次改动文件）。

**结论**：会话日志不变量具备持久审计能力——落盘后进程重启仍可回放「模型所见」，供事后审计与合规追溯。后续可选：llm/stream 观测覆盖非 execute_llm_stream 的调用路径、日志文件滚动/清理策略。

---

### 7.11 缺陷 #9（✅ 已落地）：Profile / 策略分层——插件组合 Profile + dump-config + patch

**问题**：`--dump-config` 缺插件配置与 Profile 组合的 dump；无「agent 预设 + 插件组合」的命名打包机制，无法把一组插件固化为可复用、可检视、可 patch 的策略单元。

**本轮改动**：
- `src-tauri/src/commands/plugin_profile.rs`（新增）：插件组合 Profile 命令集——
  - `plugin_profile_create`：选 agent 预设 + 插件列表打包为命名 Profile（名称唯一性校验）；
  - `plugin_profile_list` / `plugin_profile_delete`：列出 / 删除组合；
  - `plugin_profile_patch`：patch 覆盖——增删插件、改 name/description/agent_profile_id、按插件切换启用状态；
  - `plugin_profile_dump`：dump-config——导出 Profile 完整配置 + 其插件在能力注册表声明的能力接缝快照（`CapabilityRegistrationDetailDto`：id/version/contract/description/origin/plugin_id）。
- 持久化：`app_data_dir/plugin_profiles.json`（`PluginProfileStore` 惰性全局单例，`BTreeMap` + JSON 序列化，进程重启后自动恢复），无需 DB migration，与运行时 Profile / UserProfile / AgentProfile 概念正交。
- 错误码：`error_code.rs::plugin_profile` 新增 `NOT_FOUND` / `DUPLICATE_NAME` / `IO_FAILED`，并同步 11 种语言 `error` 段翻译（zh-CN 为源）。
- 注册：`commands/mod.rs` 声明 `plugin_profile` 模块 + `register_commands.rs` 注册 5 个 Tauri 命令。
- 单测：`PluginProfileStore` 的 CRUD、跨实例重建持久化、空文件加载——验证重启后组合不丢。

**验证**：`cargo check -p axagent` 通过；`cargo test -p axagent --lib commands::plugin_profile::tests` → **2 passed**（`store_crud_and_persists_across_reopen`、`store_new_loads_empty_when_no_file`）。

**结论**：Profile/策略分层的地基（打包 + 检视 + 裁剪的实际操作原语）已打通，前端可基于 `plugin_profile_*` 提供「插件组合」管理 UI。后续增量：策略裁剪（按 profile 过滤能力接缝）、manifest ↔ 外部标准（Claude Code / Agent SDK）映射。

---

### 7.12 缺陷 #10（✅ 已修复）：model 副作用式全局注入收敛为幂等 / Director 注册

**问题**：`ProviderRegistry::create_default()` 既构造本地注册表，又隐式调用 `register_builtins_into_capability_registry()` 把 13 个内置适配器重复写入全局能力注册表。`create_default` 在 `llm_bridge` / `compress` / `conversations::mod` / `indexing` / `state` 等 8 处被调用，每次重复注册，靠 `Duplicate + warn` 容错，注册结果依赖调用顺序，属副作用式全局注入（非 DI）。

**本轮改动**：
- `crates/providers/src/registry.rs`：
  - `create_default()` 收敛为**纯构造**——只填充本地注册表（抽出私有 `register_all_builtins()`），移除隐式全局注册副作用；
  - `register_builtins_into_capability_registry()` 成为**显式 Director 入口**，由 wiring 层调用一次；幂等（同键内置重复注册静默跳过），重复调用安全、与调用顺序无关。
- `crates/harness/src/capability_registry.rs`：
  - `register_model_provider_impl` 新增 `idempotent` 参数；`register_model_provider_for_type`（内置按类型路径）传 `true`，同键且同为 BuiltIn 时静默跳过并返回空句柄（撤销亦为空操作），不再产生 `Duplicate` 告警；
  - `register_model_provider` / `register_external_model_provider`（默认单键接缝）保持非幂等，兼容既有 `duplicate_registration_is_rejected` 语义。
- `src-tauri/src/init/state.rs`：wiring 层在 `create_default()` 后显式调用 `register_builtins_into_capability_registry()`（Director 注册统一收敛于此）。

**验证**：`cargo check -p axagent` 通过；`cargo test -p axagent-harness --lib capability_registry` → **29 passed**（新增 2 个：`builtin_by_type_reregistration_is_idempotent`（同键内置重复注册静默跳过、能力条目不增、撤销空句柄不移除首次注册）、`builtin_by_type_does_not_override_external`（外部先占同键 → 内置按类型注册报 Duplicate，不覆盖外部，保持平权语义））；`cargo test -p axagent-providers --test registry_tests` → **7 passed**；rustfmt 通过。

**结论**：内置适配器注册从「分散副作用 + 告警容错」收敛为「wiring 层单点 Director 幂等注册」，与调用顺序解耦。

---

### 7.13 缺陷 #11（✅ 已修复）：补充内置接缝端到端闭环测试

**问题**：既有单测只覆盖「注册/回滚 API 正确性」（如 Duplicate、undo 移除），没有「内置接缝经注册表被真实 consumer 取回同一对象」的闭环测试，因此「注册即检索失败」类缺陷（如 model.provider key 不匹配）未被单测捕获。

**本轮改动**：`crates/harness/src/capability_registry.rs` 新增端到端闭环单测 `builtin_seams_end_to_end_register_and_consumable`——把 9 大内置接缝注册进本地注册表后，逐个经对应 consumer 检索 API 取回，并用 `Arc::ptr_eq` 断言**取回的就是注册的那个实例**（注册与消费落在同一存储），证明链路闭环：
- `model.provider.{type}` → `get_provider_adapter_for(type)`（缺陷 #10 配套）
- `agent.loop` → `get_agent_turn_runner`
- `storage` → `get_storage`
- `sandbox` → `get_sandbox`
- `session.log.invariant` → `get_session_log_invariant`（用真实 `InMemorySessionLog`）
- `message.callback` → `get_message_callback`
- `webhook.dispatch` → `get_webhook_dispatch`
- `platform.adapter` → `get_platform_adapter`
- `event.dispatch` → `get_event_dispatcher`（用真实 `EventDispatchBus`）

**验证**：`cargo test -p axagent-harness --lib capability_registry` → **30 passed**（新增 1 个端到端闭环测试）；rustfmt 通过。

**结论**：内置核心平权链路现在有端到端闭环护栏——任何一处「注册键」与「消费键」不一致，测试即红，可防止缺陷 #8（model.provider 注册即检索失败）类回归。

---

## 核查依据

- 《一切皆插件-Agent架构范式与AxAgent重构路径.md》4.2 六项重构 + 4.4 P0–P3 路线图 + 4.5 P3 落地进展 — `d:\OneManager\AxAgent\docs\一切皆插件-Agent架构范式与AxAgent重构路径.md`
- harness 能力注册表与可逆效果实现 — `src-tauri\crates\harness\src\capability_registry.rs · reversible_effect.rs`
- 插件系统外部能力注册与 manager 实现 — `src-tauri\crates\plugins\src\manager.rs · types.rs`
- wiring 层注入与命令注册 — `src-tauri\src\init\state.rs · src-tauri\src\commands\capability.rs · register_commands.rs`
- 事件/hooks 现状 — `harness\src\event_bus.rs · runtime-core\src\hooks.rs · tools\src\hooks\mod.rs · harness\src\tool_service.rs`

> 本报告由只读代码探索生成；完成度百分比为按任务关键子项的定性评估，非精确度量。
