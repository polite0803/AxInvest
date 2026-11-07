> 来源：导入自上游 AxAgent（基准 99c9b96f，2026-09-07） ｜ 导入日期：2026-11-04

# AxAgent 全面功能缺陷审计报告（更新版）

> **审计日期**: 2026-07-04
> **当前版本**: 2.8.0 (Cargo workspace, 30 crates + 1 schema-gen)
> **审计范围**: 前端(React/TS, 754+文件) + 后端(Rust/Tauri, 30 crates) + CI/构建 + i18n + 架构合规 + 安全性能
> **审计方法**: 六路并行 Agent 审计 + 去重合并
> **对比基准**: 2026-07-02 审计报告（78个缺陷）

---

## 执行摘要

本次全链路审计发现 **139 个功能缺陷**（含上次审计残留和新增），按严重程度分布：

| 严重程度 | 数量 | 关键影响 |
|----------|------|----------|
| 🔴 CRITICAL | 12 | 架构报告虚假修复、运行时panic、CI永远失败、数据静默丢失 |
| 🟠 HIGH | 31 | i18n结构性不一致、类型安全绕过、Mutex中毒panic、重连缺失 |
| 🟡 MEDIUM | 52 | 竞态条件、超大组件、CSP放宽、依赖版本不一致 |
| 🟢 LOW | 44 | 死代码、文档过时、代码异味、测试覆盖不足 |

### 🚨 核心发现

1. **架构审计报告虚假** — `architecture_audit_report.md` 声称 5 项 P0/P2 已修复，经逐文件验证 **0 项落地**
2. **i18n 结构性崩溃** — 10 种语言各缺 203-237 键，76 个键源码使用但 en-US 未定义，占位符 36 处不一致
3. **类型安全侵蚀** — 工作流组件中 120+ 处 `as any` / `as unknown as` 绕过 TypeScript
4. **serde_json 静默丢数据** — 20+ 处 `.to_string().ok()` 序列化失败无日志无降级
5. **生产日志泄露** — 100+ 处 console.log/info 在生产代码中未移除

---

## 与上次审计（2026-07-02）对比

| 状态 | 数量 | 说明 |
|------|------|------|
| ✅ 已修复 | 2 | C-5 CI macOS apt-get → brew（初版报告即已纠正）；部分硬编码中文已 i18n 化 |
| ⚠️ 部分修复 | 3 | C-8 i18n 键数从缺 258 降至缺 203-237；C-1/C-2 Mutex unwrap 部分替换 |
| ❌ 未修复 | 73 | 大部分缺陷仍然存在 |
| 🆕 新增 | 64 | 本次审计新发现的缺陷 |

---

## 🔴 CRITICAL 级缺陷 (12)

### C-NEW-1. [审计诚信] architecture_audit_report.md 修复记录与代码实际状态严重不符

**这是本次审计最严重的问题。**

审计报告声称的 8 项修复，经逐文件 `grep` 验证：

| 修复事项 | 报告声称状态 | 代码实际状态 |
|---------|:-----------:|:-----------:|
| P0: 从 Cargo.toml 移除 4 项直接依赖 | ✅ 已修复 | ❌ **未修复** — 4 行完整保留 |
| P0: src/ 下 637 处 import 批量替换 | ✅ 已修复 | ❌ **0 处替换** — 61 文件仍直接 import |
| P0: `lib.rs` 添加 `pub mod domain;` | ✅ 已修复 | ❌ **未添加** — domain.rs 是死代码 |
| P2: `runtime-core` 改依赖 `kit` | ✅ 已修复 | ❌ **仍依赖 core** |
| P2: `gateway` 移除 `kit` 依赖 | ✅ 已修复 | ❌ **仍依赖 kit** |
| P2: `RuntimeHarness` 添加 `SessionTracer` | ✅ 已修复 | ❌ **未添加** |
| P2: `rt-dashboard` 添加 harness 依赖 | ✅ 已修复 | ❌ **未添加** |
| P1: `i18n.rs` 从 core 移到 harness | ✅ 已修复 | ❌ **copied 而非 moved** — 双份代码 |

**文件**: `architecture_audit_report.md` 第 356-395 行「修复记录」章节
**影响**: 后续决策依赖虚假基线，技术债真实状态被掩盖
**根因**: `domain.rs` (4 行 pub use) 已创建但 `lib.rs` 缺少 `pub mod domain;` 声明

### C-NEW-2. [数据完整性] 20+ 处 serde_json::to_string().ok() 序列化失败静默返回 None

覆盖存储层、工作流引擎、流式工具调用、DAO 层。序列化失败（通常表示数据结构不兼容）会导致数据库字段存 NULL 且无任何错误信号。

| 文件 | 行号 | 数据内容 |
|------|------|---------|
| `src-tauri/src/commands/agent/mod.rs` | 1552, 2581 | 工具调用块、技能依赖 |
| `src-tauri/src/commands/workflow_template.rs` | 31,37,41,46,51 | trigger_config, input_schema, output_schema, error_config, tool_defs |
| `src-tauri/src/commands/conversations/streaming.rs` | 236 | 流式工具调用 |
| `src-tauri/src/commands/skill_decomposition.rs` | 471 | 原始来源 |
| `src-tauri/crates/rt-workflow/src/work_engine/engine/mod.rs` | 2802,3038,3086,3087 | 工作流状态 |
| `src-tauri/crates/dao/src/repo/workflow_template.rs` | 125,128,130,132,283,289,293,298 | 工作流模板字段 |
| `src-tauri/crates/dao/src/repo/prompt_template.rs` | 129 | 标签 JSON |
| `src-tauri/crates/trajectory/src/storage.rs` | 155,159 | 检查点上下文/结果 |
| `src-tauri/crates/runtime-core/src/hooks.rs` | 726 | 更新输入 |
| `src-tauri/crates/kit/src/workflow_version.rs` | 100,104 | 版本比较 |

**修复**: 添加 `.map_err(|e| { tracing::error!(%e, "序列化失败"); e }).ok()` 或使用 `unwrap_or_default()`

### C-NEW-3. [运行时] Mutex 中毒直接 panic — 无降级

- **文件**: `src-tauri/crates/agent/src/feedback_orchestrator.rs:161,200` — `self.seen_traces.lock().unwrap()` 裸 unwrap
- **文件**: `src-tauri/crates/agent/src/llm_bridge.rs:69,80` — `self.inner.lock().unwrap()`
- **文件**: `src-tauri/crates/agent/src/provider_fallback_20260701_010125_664.rs:195-267` — 6 处 `.lock().unwrap()`。**注意**: 文件名含时间戳，疑似自动备份/快照残留

### 由上期残留的 CRITICAL：

**C-1.** [生产] 工作流引擎 `audit_recorder` Mutex 中毒 → panic
- **文件**: `crates/rt-workflow/src/work_engine/engine/mod.rs:3024`
- **状态**: ❌ 未修复

**C-2.** [生产] `pause_signal` 不受保护的 `unwrap()`
- **文件**: `crates/rt-workflow/src/work_engine/engine/mod.rs:1763`
- **状态**: ❌ 未修复

**C-3.** [生产] Rhai 值转换 5 处不安全 `unwrap()`
- **文件**: `crates/rt-workflow/src/expression_engine/rhai_eval.rs:52-73`
- **状态**: ❌ 未修复

**C-4.** [前端] BalanceBadge 调用不存在于 providerStore 的方法
- **文件**: `src/components/settings/BalanceBadge.tsx:18`
- **状态**: ❌ 未修复

**C-6.** [CI] `pr-ci.yml` 集成测试不执行
- **文件**: `.github/workflows/pr-ci.yml`
- **状态**: ❌ 未修复

**C-7.** [代码] `crates/billing/` 孤立 crate
- **文件**: `src-tauri/crates/billing/`
- **状态**: ❌ 待确认（本次未复查此目录）

**C-8.** [前端] 其他语言各缺 203-237 个 i18n 键
- **状态**: ⚠️ 部分改善 — 上次缺 258 键，本次缺 203-237 键，但仍有结构性不一致问题（见 H-NEW-1）

### 新增 CRITICAL：

**C-NEW-4.** [CI] `test-build.yml` prerelease 覆盖 draft → 意外公开发布
- **文件**: `.github/workflows/test-build.yml:64`
- **问题**: `releaseDraft: true` 与 `prerelease: true` 同时设置，prerelease 会覆盖 draft，导致测试构建**立即公开发布**而非仅创建草稿
- **修复**: 移除 `prerelease: true` 或改为条件设置

**C-NEW-5.** [CI] cargo-audit 安全漏洞不阻止 CI 通过
- **文件**: `.github/workflows/pr-ci.yml:220-232`
- **问题**: `continue-on-error: true` 使安全漏洞静默降级，CI 不会变红
- **修复**: 移除 `continue-on-error` 或仅在确认安全的特定漏洞上设置 `--ignore`

**C-NEW-6.** [工具] `crates/tools/src/tools/skill.rs` 三阶层级裸 unwrap 对用户文档无防护
- **文件**: `src-tauri/crates/tools/src/tools/skill.rs:2179-2187`
- **代码**: `doc.as_object_mut().unwrap().as_object_mut().unwrap()...` — 三级级联，用户传入任意 JSON 都可能导致 panic
- **修复**: 使用 `match` / `if let` / `?` 逐层检查

---

## 🟠 HIGH 级缺陷 (31)

### 架构违规（新增 + 残留）

**H-NEW-1.** [架构] 61 个文件绕过 runtime 直接依赖实现 crate

| 实现 crate | 违规文件数 | 关键文件 |
|-----------|-----------|---------|
| `axagent-agent` | 19 | app_state.rs, init/state.rs, commands/agent/mod.rs, commands/conversations/* 等 |
| `axagent-trajectory` | 21 | app_state.rs, lib.rs, context_manager.rs, init/services.rs, init/state.rs 等 |
| `axagent-providers` | 13 | indexing.rs, init/state.rs, commands/agent/mod.rs, llm_wiki.rs 等 |
| `axagent-plugins` | 8 | app_state.rs, init/state.rs, commands/skills.rs 等 |

**根因**: `domain.rs` 文件已创建但 `lib.rs` 缺少 `pub mod domain;`

**H-NEW-2.** [架构] i18n.rs 在 core 和 harness 中重复存在
- **文件**: `src-tauri/crates/core/src/i18n.rs` (197行) + `src-tauri/crates/harness/src/i18n.rs` (197行)
- **问题**: 两个完全相同的模块，一个应该被删除。如果两个独立演进，会产生分歧

### i18n 结构性崩溃（新增）

**H-NEW-3.** [i18n] 76 个翻译 key 在源代码中使用但 en-US.json 中不存在

这些 key 在前端代码中用 `t('xxx')` 调用，但没有在任何语言文件中定义，运行时返回原始 key 字符串。部分示例：

- `expertCategory.` (尾部含点号，疑似代码 bug) — ExpertSelector.tsx:561,872,986 等 5 处
- `workflow.aiPanel.workflowParsed` — AIPanel.tsx:234
- `workflow.importExport.yamlExportFailed` — WorkflowEditor.tsx:1235
- `workflow.mergeNode.branches` — MergeNode.tsx:25
- `workflow.props.aggregAll` — AggregatorPropertyPanel.tsx:49
- `timeTravel.modeSwitch` — ModeSwitch.tsx:18
- 其余约 70 个主要在 `workflow.props.*`, `workflow.validation.*` 命名空间

**H-NEW-4.** [i18n] zh-CN 占位符 36 处与 en-US 不一致

| 类型 | 数量 | 示例 |
|------|------|------|
| 缺失占位符 | 30 | `chat.workflow.attempts` 缺 `{count}`, `workflow.confirmDeleteTemplate` 缺 `{name}` |
| 多余占位符 | 6 | `workflow.aiAssist.loop.continueHint` 多了 `{items}` |

**H-NEW-5.** [i18n] 非 en-US 文件存在 en-US 中不存在的键族（结构不一致）
- `components_devtools_RLTrainingConfig.*` (~5键)
- `components_devtools_RLTrainingMonitor.*` (~8键)
- `components_devtools_RLTrainingPanel.*` (~5键)
- `components_trace_BottleneckAnalyzer.*` (~3键)

这些键在 ja/ko/fr/de/es/ru/ar/hi/zh-TW 中存在但 en-US 中缺失，且键名为中文（如 `学习率`、`批次大小`），值标记为 `[待实现]`。

**H-NEW-6.** [i18n] `src/i18n/locales/.npm-cache/` 混入源代码树
- **文件**: `src/i18n/locales/.npm-cache/` — npm 缓存目录，含多个 debug 日志文件

### 前端代码质量

**H-NEW-7.** [前端] 120+ 处 `as any` / `as unknown as` 类型安全绕过

| 密集区域 | 数量 | 说明 |
|---------|------|------|
| `workflowEditorStore.ts` | 18 | 节点更新、配置清理、诊断修复、模板加载 |
| `DebugPanel.tsx` | 8 | 运行时属性访问绕过类型系统 |
| `WorkflowEditor.tsx` | 10 | 4 处 as unknown as + 6 处 any 参数 |
| `Nodes/*.tsx` (30+文件) | ~30 | `_data as unknown as *NodeData` 统一模式 |
| `PropertyPanels/*.tsx` (15+文件) | ~15 | `node as unknown as *Node` |
| `LeftPanel.tsx` | 3 | KeyboardEvent 强制转 MouseEvent |
| `AIPanel.tsx` | 2 | nodes/edges 选择器强制转换 |
| `workflowLayout.ts` | 4 | (n as any).title/config/kind |
| 其他位置 | ~10 | RLTrainingConfig, BalanceBadge, BenchmarkRunner 等 |

**H-NEW-8.** [前端] 4 个 .bak 备份文件残留在版本控制中

- `src/pages/DashboardPage.tsx.bak`
- `src/components/dynamicUI/data/Dashboard.tsx.bak`
- `src/components/workflow/AIPanel/NLParseResultView.tsx.bak`
- `src/components/workflow/Diagnostic/DiagnosticDrawer.tsx.bak`

**H-NEW-9.** [前端] 6 个超大文件（>1500行）严重影响维护性

| 文件 | 行数 | 问题 |
|------|------|------|
| `InputArea.tsx` | 3312 | 22 处 eslint-disable，极度复杂 |
| `workflowEditorStore.ts` | 2802 | Store 过大，混杂 UI 状态和业务逻辑 |
| `WorkflowEditor.tsx` | 2657 | 16 处 eslint-disable + 6 处 any |
| `ChatSidebar.tsx` | 2384 | 会话列表管理过于集中 |
| `conversationStoreSend.ts` | 1785 | 发送逻辑过于集中 |
| `ChatViewMessages.tsx` | ~1600 | 渲染逻辑应拆分 |

### 后端错误处理

**H-NEW-10.** [后端] `serde_json::to_string().ok()` 序列化失败静默吞错 — 见 C-NEW-2（同源问题，严重程度升至 CRITICAL）

**H-NEW-11.** [后端] 非初始化代码中的裸 expect() — 不变量假设脆弱

- `src-tauri/crates/agent/src/react_engine.rs:1202,1220,1242,1311,1346,1402`
- `src-tauri/crates/agent/src/hierarchical_planner.rs:85`
- `src-tauri/crates/agent/src/reflector.rs:217`
- `src-tauri/crates/agent/src/shared_blackboard.rs:140`
- `src-tauri/crates/agent/src/tree_of_thoughts.rs:422`
- `src-tauri/crates/agent/src/wiki_compiler.rs:591,598`
- `src-tauri/crates/agent/src/ingest_pipeline.rs:733`

**H-NEW-12.** [后端] 工作流引擎 Mutex expect("mutex poisoned") — 直接 panic 而非降级

- `src-tauri/crates/rt-workflow/src/work_engine/engine/mod.rs:557,567,584,593,602,609`

### 由上期残留的 HIGH：

**H1.** `std::sync::RwLock` 全局状态 — **状态变更**: 经审计验证，3 处 `std::sync::RwLock` 均有 SAFETY 注释且设计合理，**降级为低风险**
**H2.** FileRoleRegistry `std::sync::RwLock` — **同上，降级**
**H3.** `src/init/services.rs` expect() panic — **未复查**
**H4.** ReAct 循环 `consecutive_failures` 重置逻辑漏洞 — **状态**: ❌ 未修复
**H5.** SubWorkflow spawn 失败无追踪 — **状态**: ❌ 未修复
**H6.** 硬编码中文文本 — **状态**: ⚠️ 部分改善，仍有新增发现
**H7.** `key={index}` — **状态**: ❌ 未修复（仍有 14 处）
**H8.** `AntdList.tsx` 死桩代码 — **状态**: ❌ 未修复
**H9.** conversationStore 模块级可变状态 — **状态**: ❌ 未修复
**H10.** useVoiceChat WebSocket worklet 竞态 — **状态**: ❌ 未修复
**H11.** useUpdateChecker 非 Tauri 环境崩溃 — **状态**: ❌ 未修复
**H12.** 47+ 处 `as unknown as` — **状态**: ❌ 未修复，且新增到 120+ 处
**H13.** Webhook 无并发限制 — **状态**: ❌ 未修复
**H14.** WebSocket 断开无自动重连 — **状态**: ❌ 未修复
**H15.** `majority.unwrap()` — **状态**: ❌ 未修复
**H16.** 字面量 Header unwrap — **状态**: ❌ 未修复
**H17.** CHANGELOG 缺失 v2.7.0/v2.8.0 — **状态**: ❌ 未修复

### 安全与性能新增：

**H-NEW-13.** [安全] 100+ 处 console.log/info 在生产代码中未移除

主要集中点：
- `src/lib/invoke.ts` — 7 处（慢调用、错误详情）
- `src/stores/domain/conversationStoreSend.ts` — 5 处（消息再生细节）
- `src/stores/domain/streamStore.ts` — 2 处（取消流、会话模式）
- `src/hooks/useKeyboardShortcuts.ts` — 快捷键命中
- `src/components/workflow/WorkflowEditor.tsx` — 4 处（操作细节）

**H-NEW-14.** [安全] 50+ 处 localStorage 直接操作，缺乏抽象层

关键位置：notificationCenter, inputArea (模型选择), modelSelector, acpSettings, expertStore, agentPanelStore, searchStore, workflowEditorStore, WorkflowEditor 面板状态

**H-NEW-15.** [安全] ACP WebSocket SDK 无重连逻辑
- **文件**: `src/sdk/index.ts:196-245` — `connectWebSocket()` 的 `onclose`/`onerror` 仅设置 `done = true`

---

## 🟡 MEDIUM 级缺陷 (52)

### 前端 (15)

| # | 文件 | 问题 |
|---|------|------|
| M-NEW-1 | `src/stores/feature/workflowEditorStore.ts:2737-2766` | 拖拽/缩放时密集同步写 localStorage |
| M-NEW-2 | `src/stores/shared/agentPanelStore.ts:83,91,99` | 调整大小时密集写 localStorage |
| M-NEW-3 | `src/stores/feature/skillExtensionStore.ts:499-510` | `_pollingTimer` 模块级 setInterval，多处使用时可能重复轮询 |
| M-NEW-4 | `src/components/chat/WorkflowProgressPanel.tsx:703` | pollTimer 依赖数组可能不完整 |
| M-NEW-5 | `src/lib/preloadChatRenderers.ts:40-41` | `import("...").catch(() => {})` 空 catch 吞资源加载失败 |
| M-NEW-6 | `src/pages/DashboardPage.tsx:151-152` | 空 catch 吞 fetchGatewayMetrics/fetchProviders 失败 |
| M-NEW-7 | `src/components/chat/AskUserCard.tsx:13-14` | `askId` 和 `conversationId` props 声明但未使用 |
| M-NEW-8 | `src/components/decomposition/*` (4文件) | 使用 `../../types` 相对路径而非 `@/types` |
| M-NEW-9 | `src/components/workflow/Panels/PropertyPanels/*` (18文件) | 使用 `../../Hooks` 和 `../../types` 相对路径 |
| M-NEW-10 | `src/stores/feature/workEngineStore.ts:5` | `import { invoke } from "../../lib/invoke"` 而非 `@/lib/invoke` |
| M-NEW-11 | `src/stores/domain/conversationStore.ts:238-302` | `usePreferenceStore.getState()` + `useMultiModelStore.getState()` 跨 store 直接调用 |
| M-NEW-12 | `src/components/chat/ChatViewMessages.tsx:1026-1061` | `renderUserAvatar` 既 useCallback 又 useMemo 双重内存化 |
| M-NEW-13 | `src/components/chat/InputArea.tsx:224` | 注释掉的废弃代码残留（`// const [modelRoutingOpen...]`） |
| M-NEW-14 | `src/stores/domain/conversationStore.ts:1394` | `partial as unknown as Parameters<typeof setState>[0]` |
| M-NEW-15 | `src/stores/feature/expertStore.ts:59` | Agent profile 强制类型转换 |

### 后端 (14)

| # | 文件 | 问题 |
|---|------|------|
| M-NEW-16 | `src-tauri/src/commands/error_mapping.rs` | 在 commands/ 但未在 mod.rs 声明 — **死代码** |
| M-NEW-17 | `src-tauri/src/commands/constants.rs` | 同上 — **死代码** |
| M-NEW-18 | `src-tauri/crates/agent/src/provider_fallback_20260701_010125_664.rs` | 文件名含时间戳，疑似自动快照残留 |
| M-NEW-19 | `src-tauri/crates/trajectory/src/**` (25文件) | 100+ 处 `#[allow(dead_code)]` — 大量功能未集成 |
| M-NEW-20 | `src-tauri/crates/tools/src/tools/bash.rs:213,333` | `需修复(Windows)` — cmd 孤儿子进程，需 Job Objects |
| M-NEW-21 | `src-tauri/crates/rt-workflow/src/work_engine/executors/switch_executor.rs:133` | `待实现: 集成 LLM 调用` — Switch LLM 路由未实现 |
| M-NEW-22 | `src-tauri/src/commands/tracer.rs:584` | `待实现(Phase3): 触发技能进化评估` |
| M-NEW-23 | `src-tauri/src/commands/scheduled_task.rs:24` | `待实现(C5): 待 AppState 引入 active_operator` |
| M-NEW-24 | `src-tauri/src/commands/workflow_ai_apply.rs:107` | `segments.split_last().unwrap()` — 空切片 panic |
| M-NEW-25 | `src-tauri/crates/rt-messaging/src/message_gateway/platforms/*.rs` (slack/discord/qq) | `_ => {}` 静默忽略未预期的消息平台事件 |
| M-NEW-26 | `src-tauri/crates/providers/src/*.rs` (anthropic/openai/hermes/multi_mode) | `_ => {}` 静默忽略 LLM 响应新变体 |
| M-NEW-27 | `src-tauri/crates/agent/src/session_manager.rs:1199` | `_ => {}` 静默忽略会话管理状态 |
| M-NEW-28 | `src-tauri/src/commands/image_gen.rs` | 绕过 runtime 直接使用 provider image_gen 模块 |
| M-NEW-29 | `src-tauri/crates/tools/src/tools/skill.rs:2554` | `parts.next().unwrap()` 双次链式 unwrap |

### 由前期残留的 MEDIUM：

**M1-M9, M10-M18, M19-M27** — 大部分未修复，详见 2026-07-02 报告

### CI/构建新增 (11)

| # | 文件 | 问题 |
|---|------|------|
| M-NEW-30 | `.github/workflows/release.yml:186` | `npm install` 而非 `npm ci`（依赖漂移风险） |
| M-NEW-31 | `.github/workflows/release.yml:442` | Android NDK `r27d` 已废弃 |
| M-NEW-32 | `.github/workflows/release.yml:449` | JDK 23 (Zulu) 已 EOL |
| M-NEW-33 | `.github/workflows/pr-ci.yml:110` | Protoc 版本与 release.yml 不一致 |
| M-NEW-34 | `.github/workflows/pr-ci.yml:205` | E2E 排除 Agent/Cache/Conversation/Gateway |
| M-NEW-35 | `package.json:128` | `oxlint` 已安装但未集成到任何脚本 |
| M-NEW-36 | `package.json:31` | `test:integration` 实际运行所有测试而非仅集成测试 |
| M-NEW-37 | `package.json:57-58` | `@dprint/toml` 和 `@dprint/markdown` 在 dependencies 而非 devDependencies |
| M-NEW-38 | `src-tauri/tauri.conf.json:15` | CSP `connect-src` 允许 `localhost:*` + 硬编码外部 API，`img-src` 含通配符 `https:` |
| M-NEW-39 | `src-tauri/build.rs:8-9` | `cfg!(target_os)` 在 build.rs 中评估主机平台而非目标平台（交叉编译 bug） |
| M-NEW-40 | 项目根目录 | 缺少 `rust-toolchain.toml` — Rust 版本未固定 |

### 安全新增 (4)

| # | 文件 | 问题 |
|---|------|------|
| M-NEW-41 | `src/sdk/sandboxTemplate.ts:48` | Skill 沙箱 CSP 使用 `script-src 'unsafe-inline'` |
| M-NEW-42 | `src/components/chat/ArtifactPreview/ReactPreview.tsx:25` | React 预览 CSP 含 `unsafe-eval` + unpkg.com 外部脚本 |
| M-NEW-43 | `src/lib/codeExecutor.ts:128-131` | Pyodide `exec()` 执行用户 Python 代码 |
| M-NEW-44 | `.github/workflows/release.yml:475` | Keystore 在 CI runner 上明文写入文件系统 |

### 前端路径别名 (4)

| # | 文件 | 问题 |
|---|------|------|
| M-NEW-45 | `src/components/workflow/Templates/*.tsx` (3文件) | 使用 `../types` 而非 `@/components/workflow/types` |
| M-NEW-46 | `src/components/workflow/Diagnostic/DiagnosticDrawer.tsx` | 使用 `../types/workflow.types` 相对路径 |
| M-NEW-47 | `src/components/workflow/AIPanel/AIPanel.tsx` | 使用 `../dndState` + `../types/workflow.types` |
| M-NEW-48 | `src/components/workflow/StatusBar/EditorStatusBar.tsx` | 使用 `../types` 而非 `@/...` 别名 |

---

## 🟢 LOW 级缺陷 (44)

### 前端 (10)

| # | 文件 | 问题 |
|---|------|------|
| L-NEW-1 | `src/components/chat/CredibilityBadge.tsx:36,42` | `key={i}` 2 处 |
| L-NEW-2 | `src/components/chat/VoiceCall.tsx:60` | `key={i}` |
| L-NEW-3 | `src/components/wiki/WikiDetailPanel.tsx:532` | `key={i}` |
| L-NEW-4 | `src/components/wiki/BacklinkPanel.tsx:28` | `key={i}` |
| L-NEW-5 | `src/components/chat/EvolutionPanel.tsx:166` | `key={i}` |
| L-NEW-6 | `src/components/trace/BottleneckAnalyzer.tsx:69` | `key={i}` |
| L-NEW-7 | `src/components/chat/CategoryManagerModal.tsx:162` | `key={i}` |
| L-NEW-8 | `src/components/devtools/RLTrainingMonitor.tsx:34,38` | `key={i}` SVG 元素 |
| L-NEW-9 | `src/components/dynamicUI/VisualEditor.tsx:652` | `key={idx}` 动态 style 条目 |
| L-NEW-10 | `src/components/dynamicUI/SchemaTreeEditor.tsx:349` | `key={idx}` prop 条目 |

### 后端 (12)

| # | 文件 | 问题 |
|---|------|------|
| L-NEW-11 | `src-tauri/crates/tools/src/global_state.rs:19` | `std::sync::RwLock` — **安全**，有 SAFETY 注释且异步上下文正确 |
| L-NEW-12 | `src-tauri/crates/storage/src/storage_paths.rs:4` | `std::sync::RwLock` — **安全**，有 if let Ok() 中毒恢复 |
| L-NEW-13 | `src-tauri/crates/rt-workflow/src/agent_roles.rs:13` | `std::sync::RwLock` — **可接受**，锁在 await 之后获取 |
| L-NEW-14 | `src-tauri/src/commands/agent/payloads.rs:72,101,151,167,318` | `#[allow(dead_code)]` 5 处 |
| L-NEW-15 | `src-tauri/src/state/agent.rs:16,31` | `#[allow(dead_code)]` 2 处 |
| L-NEW-16 | `src-tauri/src/state/task.rs:16,36` | `#[allow(dead_code)]` 2 处 |
| L-NEW-17 | `src-tauri/src/state/memory.rs:11,37` | `#[allow(dead_code)]` 2 处 |
| L-NEW-18 | `src-tauri/src/state/gateway.rs:15,21` | `#[allow(dead_code)]` 2 处 |
| L-NEW-19 | `src-tauri/src/state/skill.rs:13,50,127,135` | `#[allow(dead_code)]` 4 处 |
| L-NEW-20 | `src-tauri/src/state/infra.rs:15,25` | `#[allow(dead_code)]` 2 处 |
| L-NEW-21 | `src-tauri/crates/providers/src/openai.rs:304-458` | `#[allow(dead_code)]` 9 处 |
| L-NEW-22 | `src-tauri/crates/harness/src/workflow_types.rs:1260` | 注释"预留供 SwarmNodeConfig 后续使用" |

### 构建/配置 (10)

| # | 文件 | 问题 |
|---|------|------|
| L-NEW-23 | `typedoc.json:23` | `treatWarningsAsErrors: false` — CI 中警告不阻塞 |
| L-NEW-24 | `tsconfig.json:7` | `skipLibCheck: true` — 忽略第三方类型错误 |
| L-NEW-25 | `vite.config.ts:113` | CSS 仍用 postcss 而非 lightningcss |
| L-NEW-26 | `package.json:144-146` | `@antv/infographic` 在 optionalDependencies 不合理 |
| L-NEW-27 | `eslint.config.js:45-49` | 关闭 Nodes 目录 no-unused-vars 缺少注释说明 |
| L-NEW-28 | `eslint.config.js:51-56` | ChatViewMessages 关闭两项 hook 规则缺少注释 |
| L-NEW-29 | `src-tauri/tauri.conf.json:55` | updater 禁用但配置保留 pubkey+endpoints |
| L-NEW-30 | `.github/workflows/pr-ci.yml:92-95` | 磁盘清理失败静默忽略（但后续可能因磁盘满失败） |
| L-NEW-31 | `.github/workflows/pr-ci.yml:131-140` | test-unit 创建 4GB swap，可能不够 |
| L-NEW-32 | `.github/workflows/deploy-website.yml:39` | `gh api --silent` 标志有效性待验证 |

### 依赖治理 (4)

| # | 文件 | 问题 |
|---|------|------|
| L-NEW-33 | `src-tauri/crates/plugins/Cargo.toml` | `serde` 未使用 `workspace = true` |
| L-NEW-34 | `src-tauri/crates/runtime/Cargo.toml` | `serde` + `tokio` + `tracing` 未使用 `workspace = true` |
| L-NEW-35 | `src-tauri/crates/trajectory/Cargo.toml` | `serde` + `tokio` + `tracing` 未使用 `workspace = true` |
| L-NEW-36 | `src-tauri/crates/telemetry/Cargo.toml` | `serde` + `tokio` + `tracing` 未使用 `workspace = true` |

### 文档 (4)

| # | 文件 | 问题 |
|---|------|------|
| L-NEW-37 | `AGENTS.md:54` | 引用过时版本 2.5.0 |
| L-NEW-38 | `index.html` | `<html lang="en">` 应改为 `zh-CN`（默认语言） |
| L-NEW-39 | `CHANGELOG.md` | 缺失 v2.7.0 和 v2.8.0（H17 残留） |
| L-NEW-40 | `architecture_audit_report.md:356-395` | 修复记录与代码状态不符（C-NEW-1 根因文件） |

### 测试 (2)

| # | 文件 | 问题 |
|---|------|------|
| L-NEW-41 | `src/lib/__tests__/workflowValidate.test.ts` | 源文件可能已移动 |
| L-NEW-42 | `src/components/settings/__tests__/McpServerSettings.test.tsx:131` | 测试中硬编码 API Key `"secret"` |

### 前端 store 使用 (2)

| # | 文件 | 问题 |
|---|------|------|
| L-NEW-43 | `src/stores/domain/compressStore.ts:23,63` | 直接跨 store 调用 getState |
| L-NEW-44 | `src/stores/feature/decompositionStore.ts:82` | `console.error` 中文日志 |

---

## 按功能域分布（旧+新合并）

| 功能域 | CRITICAL | HIGH | MEDIUM | LOW | 合计 |
|--------|----------|------|--------|-----|------|
| 工作流引擎 (rt-workflow) | 3 | 4 | 4 | 2 | **13** |
| 前端 (React/TS) | 2 | 10 | 18 | 14 | **44** |
| 后端核心 (services/state) | 2 | 5 | 12 | 12 | **31** |
| Agent 引擎 | 0 | 3 | 2 | 1 | **6** |
| 架构合规 | 1 | 2 | 0 | 4 | **7** |
| CI/CD/构建 | 2 | 1 | 8 | 10 | **21** |
| i18n | 0 | 4 | 0 | 0 | **4** |
| 安全 | 0 | 3 | 4 | 1 | **8** |
| 配置/Docs | 0 | 1 | 2 | 4 | **7** |

---

## 优先修复路线（修订版）

### Phase 0 — Stop the Lie（今天，1 项）
> 修复架构审计报告的虚假修复记录

1. **C-NEW-1**: 修正 `architecture_audit_report.md` 修复记录章节，标注实际未修复

### Phase 1 — Stop the Bleeding（本周，11 CRITICAL）

2. **C-NEW-2**: `serde_json::to_string().ok()` 全局添加 error 日志（20+ 处）
3. **C-NEW-3**: `feedback_orchestrator.rs` + `llm_bridge.rs` Mutex 添加中毒恢复
4. **C-NEW-6**: `tools/skill.rs` 三阶层级 unwrap 改为安全模式匹配
5. **C-1/C-2/C-3**: 工作流引擎 unwrap 全面替换
6. **C-4**: BalanceBadge `fetchBalance` 修复
7. **C-NEW-4**: test-build.yml prerelease 覆盖修复
8. **C-NEW-5**: CI cargo-audit `continue-on-error` 移除或加白名单
9. **C-6**: CI 集成测试补充执行
10. **C-7**: 清理 `crates/billing/` 孤立目录

### Phase 2 — i18n & 类型安全（两周，31 HIGH）

11. **H-NEW-3**: 76 个缺失翻译 key 补全到 en-US.json
12. **H-NEW-4**: zh-CN 占位符一致性修复（36 处）
13. **H-NEW-5**: 非 en-US 额外键族清理（同步或删除）
14. **H-NEW-6**: 清理 `.npm-cache` 目录 + 添加 .gitignore
15. **C-8**: 运行 i18n 同步脚本 + CI 强校验
16. **H-NEW-1**: Harness 架构修复：lib.rs 添加 `pub mod domain;` + 批量替换 import
17. **H-NEW-2**: i18n.rs 去重（保留 harness 版本，删除 core 版本）
18. **H-NEW-13**: 生产代码 console 日志添加条件编译移除
19. **H-NEW-14**: localStorage 统一通过存储抽象层
20. **H4/H5/H13/H14**: 工作流引擎重连和追踪修复

### Phase 3 — Clean House（本月，52 MEDIUM）

21. **M-NEW-1/M-NEW-2**: 密集 localStorage 写入添加防抖
22. **M-NEW-16/M-NEW-17**: 死代码 error_mapping.rs/constants.rs 清理
23. **M-NEW-19**: trajectory `#[allow(dead_code)]` 审计清理
24. **M-NEW-20**: Windows 孤儿进程 Job Objects 实现
25. **M-NEW-21**: Switch executor LLM 路由集成
26. 超大组件拆分（InputArea → 多个子组件）
27. 依赖版本统一（serde/tokio/tracing → workspace = true）
28. CSP 加固 + rust-toolchain.toml 添加
29. build.rs 交叉编译 bug 修复

### Phase 4 — Polish（下月，44 LOW）

30. 文档版本更新 + 代码异味清理
31. .bak 文件 + .npm-cache 清理
32. oxlint 集成 + typedoc 配置优化
33. eslint 规则注释补全

---

## 审计工具与方法论

| 审计通道 | Agent | 审计范围 | 发现数 |
|---------|-------|---------|--------|
| 通道 1 | Explore | 前端 React/TS 代码质量 | 44 |
| 通道 2 | Explore | 后端 Rust 代码质量 | 31 |
| 通道 3 | Explore | CI/构建/配置 | 21 |
| 通道 4 | Explore | i18n 国际化完整性 | 4（结构性问题，每类含数十子项） |
| 通道 5 | Explore | 架构依赖合规 | 13 |
| 通道 6 | Explore | 安全与性能 | 16 |

**数据来源**: 1200+ 文件直接审读 + grep 全局搜索 + 配置文件交叉验证 + Cargo.toml / package.json 依赖图分析
**审计时长**: ~10 分钟并行执行
**上期对比**: 与 2026-07-02 报告逐项对比，标注修复/残留/新增状态

---

> **警示**: `architecture_audit_report.md` 的修复记录不可信，后续决策应基于本次审计的实际代码状态。
>
> **维护**: 本次审计结果已记录到 `.workbuddy/memory/2026-07-04.md` 和 `全面功能缺陷审计报告_2026-07-04.md`
