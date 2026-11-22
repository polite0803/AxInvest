# 智能体运行时

> 状态：active ｜ 日期：2026-11-04 ｜ 对应版本：v3.0.0 ｜ 功能点：agent-runtime

## 定位

这条链路把「一次对话 / 一次工作流节点」落到可执行动作：会话持有上下文与事件，工具注册表把工具统一成可校验、可审计、可审批的执行单元，编排器决定哪些工具能并行；上下文超预算时由压缩链把历史折叠成摘要；知识库、记忆、技能与 MCP 作为可装载能力接入同一套工具面。

**边界**：本文件只写平台侧的会话 / 工具 / 审批 / 压缩 / 检索 / 工作流 / 装载机制，**不含供应商适配**。模型流式协议、`tool_calls` 命名归一、采样参数、计费与多 Key 轮询属于 `provider-models`。

## 当前行为

### 1. 智能体引擎与会话

`SessionManager` 是会话生命周期的唯一持有者：创建 / 恢复会话、把一轮 `RolloutItem` 落成消息、维护 token 账本、在阈值处触发自动压缩（`axagent-ai/crates/agent/src/session_manager.rs:237`、`:318`）。会话事件以 `SessionEventType` 记录，压缩完成时发 `Compacted`，payload 带 `covered_range` / `summary_preview` / `tokens_before` / `tokens_after`（`axagent-core/crates/harness/src/session_events.rs:37`、`:79-113`）；`summary_preview` 有字符上限，避免事件本身把日志撑大（`:125`）。

会话可从 DB 事件回放恢复：`list_session_ids` 在内存缺失时回退数据库，`agent_resume_from_events` 提供跨进程恢复诊断（`CHANGELOG.md:460`、`:472`）。`runtime-core` 另有一份 token 账本与压缩事件对齐层，保证「会话侧压缩」与「运行时侧计量」看到同一组数（`axagent-runtime/crates/runtime-core/src/session_token_ledger.rs:2-15`）。

（对应版本：v2.9.9（跨进程恢复诊断，`CHANGELOG.md:460`、`:472`）；v3.0.0（压缩与记忆链 70cac4b0a，会话 compact 收敛 1faa79c68 / a75e1c26b））

### 2. 工具系统：注册 → 校验 → 执行

注册表按工具名 + 别名索引，禁用列表独立于工具自身的 `is_enabled()`，`find()` 先查禁用再查别名（`axagent-runtime/crates/tools/src/registry.rs:206`、`:217`）。三类动态装载共用同一注册表：技能工具 `register_skill_tool`（`axagent-runtime/crates/tools/src/registry.rs:1222`）、运行时工具 `register_runtime_tool`（同文件 `:1231`）、MCP 工具 `register_mcp_tool`（同文件 `:1358`）。

`execute()` 是唯一执行入口，顺序固定：启用检查 → 限流（独立错误码 `rateLimited`，不与普通执行失败混用）→ 输入脱敏 → `permission_policy` 授权 → `PreToolUse` Hooks → 网络 / Shell 沙箱检查（`axagent-runtime/crates/tools/src/registry.rs:1409-1474`）。任一前置闸门拒绝都不会进入工具实现；限流单独分码是为了让上层能区分「稍后重试」与「这个调用本身不合法」（`:1417-1428`）。

**并行编排**：`OrchestratorConfig::default()` 为 `max_concurrency = 10`、`timeout_secs = 120`、`max_retries = 3`、`retry_delay_ms = 1000`（`axagent-runtime/crates/tools/src/orchestration.rs:61-68`）。`execute()` 先按 `is_concurrency_safe()` 把调用分成「可并发」与「串行」两堆（`:80-123`），可并发批用 `Semaphore` 控上限（`:166-179`），串行批逐个执行。`partition_ordered()` 则把**连续**的并发安全工具归为同一批、非安全工具各自独占批，保留原始调用顺序（`:127-164`）。

**结果回填**：并发批按 `handles` 入队顺序 `await`，因此同批结果顺序与请求顺序一致；串行批的结果统一追加在并发批之后（`:80-123`）。调用方应以 `ToolCallResponse.id` 为键回填，不能假定「整轮结果数组 = 原始调用顺序」。并发能力是否启用还受 feature flag `TOOL_CONCURRENCY` 控制（`axagent-runtime/crates/runtime-core/src/feature_flags.rs:233`）。

（对应版本：v2.9.9（数据工具接入统一注册表，`CHANGELOG.md:475`）；v3.0.0（注册表硬化 11aa58772）；并发编排在 v2.8.3 前已存在，`2560ff868` 仅测试修正）

### 3. 审批与沙箱档位

审批策略是四档枚举，序列化值为 `untrusted` / `on-failure` / `on-request` / `never`（`axagent-core/crates/harness/src/approval_policy.rs:17-61`）。全局策略与沙箱策略都以静态槽位注入，构建 `ToolContext` 时回退读取：实例显式设置优先，否则用 Settings 写入的全局值（`axagent-runtime/crates/tools/src/registry.rs` 顶部 `GLOBAL_SANDBOX_POLICY` / `GLOBAL_APPROVAL_POLICY`）。审批规则存储通过 harness trait 注入，默认未注入即「不查规则、不沉淀」（同文件 `GLOBAL_APPROVAL_RULE_STORE`；规则实体见 `axagent-core/crates/harness/src/approval_rules.rs:70-92`）。

沙箱不是「执行容器」，而是**准入检查**：`sandbox.config()` 的 `network_enabled` 与 `check_command()` 在工具执行前判定网络 / Shell 是否放行（`axagent-runtime/crates/tools/src/registry.rs:1467-1474`）。真正需要人工中断的节点走工作流审批：`approval_executor` 把挂起状态写入 `workflow_approvals` 表，`resume_token = "approval-{node_id}-{...}"` 用于恢复，返回 `NodeControl::Suspend`（`axagent-runtime/crates/rt-workflow/src/work_engine/executors/approval_executor.rs:16-95`）。

（对应版本：v2.9.9（ApprovalPolicy 策略化审批，`CHANGELOG.md:458`）；工作流审批节点 v2.9.0（796bdfa11），类型硬化 v2.9.6（473042419））

### 4. 上下文预算与压缩

自动压缩阈值是 `AUTO_COMPACTION_TOKEN_THRESHOLD = 100_000`；`SessionManager` 在装配时把该常量写入 runtime 的 auto-compaction 阈值（`axagent-ai/crates/agent/src/session_manager.rs:39`、`:923`）。压缩配置默认值与入口在 `axagent-ai/crates/agent/src/session_manager.rs:779-781`（`CompactionConfig::default()` / `should_compact` / `compact_session`），`runtime-core` 的 `compact.rs` 承载具体压缩策略与摘要实现；压缩结果以 `SessionEventType::Compacted` 对外发事件（`axagent-ai/crates/agent/src/session_manager.rs:892`、`:926`）。

压缩分两层：turn 级触发（pre-turn / post-turn auto-compaction，`axagent-ai/crates/agent/src/session_manager.rs:740-742`）与 reactive 兜底（流中触发，失败时用更激进的配置再压一次，`axagent-runtime/crates/runtime-core/src/reactive_compact.rs:73-111`）。`microcompact` 另有一条只裁剪消息体的轻量路径（`axagent-runtime/crates/runtime-core/src/microcompact.rs`）。

**压缩 prompt 版本化**：当前代码里没有独立的 `prompt_version` 常量；压缩提示词随 `compact` / `session_memory_compact` 的代码版本走，版本证据只能落到提交（70cac4b0a / 1faa79c68），不能从运行时字段读出。

（对应版本：v3.0.0；自动压缩 70cac4b0a，`CHANGELOG.md:461`；阈值语义修正见 v2.8.6，`CHANGELOG.md:1671`）

### 5. 知识库 / RAG 与记忆

RAG 入口是 `RAGPipeline`：混合检索（`HybridSearchOptions`）→ 阈值过滤 → top_k 截断 → rerank（cross-encoder，缺配置时降级规则 reranker）→ Self-RAG 相关性判定（`axagent-infra/crates/search/src/rag_pipeline.rs:17-43`、`:97-122`、`:162-168`）。候选池是 `top_k * 3`，所以「先筛后截断」不会把合格项挤掉；返回项保留 `chunk_index` 供引用追溯（`:90-96`，返回结构的 `chunk_index` 字段）。知识源同步 / 入队链在 `src-tauri/src/commands/knowledge.rs`（导入结果用 `skipped_count` / `skipped` 记录被跳过的条目）。

记忆是**文件化**的，不是向量库：`ProjectMemory` 维护 `MEMORY.md` 索引 + 分类目录，索引行数与单文件大小都有上限；`scan_relevant_files` 只取前 `MEMORY_RELEVANT_FILES_LIMIT` 个候选（`axagent-ai/crates/agent/src/project_memory.rs:190-241`、`:328-334`）。写入超限不是截断后静默保存，而是报错让调用方决定（`:258-295`）。经验条目另有 `tier` / `importance` / `decay_rate` 字段，导出时按这些字段序列化（同文件的经验导出路径）。

（对应版本：v2.9.9（RAG 管线与知识源同步，`CHANGELOG.md:557`）；记忆链 v3.0.0（70cac4b0a）；检索层更早见 v2.9.4（`CHANGELOG.md:884`））

### 6. 工作流引擎

工作流引擎在 `rt-workflow`：`work_engine/engine` 负责 DAG/FSM 调度、节点状态、guard 求值与 trace；`work_engine/executors` 是按节点类型拆的执行器（LLM、工具、条件、循环、并行、聚合、审批、子工作流、存储、HTTP、通知等），统一走 `node_executor_trait`（`axagent-runtime/crates/rt-workflow/src/work_engine/executors/mod.rs`、`.../node_executor.rs`）。

三类关键行为：

- **LLM 节点**：模型解析与类型门由 `llm_resolve` 统一处理，调用 `resolve_model_type` 决定生成 / 嵌入 / 阻断（`axagent-runtime/crates/rt-workflow/src/work_engine/executors/llm_resolve.rs:135-144`）。
- **审批节点**：挂起 → 写 `workflow_approvals` → 用 `resume_token` 恢复（`axagent-runtime/crates/rt-workflow/src/work_engine/executors/approval_executor.rs:16-95`）。
- **宿主受理**：`workflow.run` 提供异步受理、owner 隔离、总超时与进度事件（`35db96406`）。

`runtime-core` 只提供运行时抽象与共享类型（会话、压缩、usage、权限、feature flag），不实现具体节点执行器——这是 consumer / hybrid 的分层边界（`axagent-runtime/crates/runtime-core/src/lib.rs`）。

（对应版本：v2.9.0（人工审批节点 796bdfa11）；v2.9.6（类型驱动硬化 473042419）；v3.0.0+（unreleased，`workflow.run` 35db96406））

### 7. 技能与 MCP 装载

技能与 MCP 都不新造执行面，而是注册成普通工具：技能走 `register_skill_tool`（handler 为 `Fn(&str) -> Result<String, ToolError>`，`axagent-runtime/crates/tools/src/registry.rs:1222`），MCP 走 `register_mcp_tool`（配置由 `McpManager` / `McpServerConfig` / `McpToolConfig` 承载，同文件 `:1358`、`:1369`）。运行时动态注册的工具通过 `register_runtime_tool` 登记（同文件 `:1231`），卸载时按后进先出清理副作用（同文件 `Disposer` 的 LIFO 语义）。

MCP 服务端接入会话层后，`agent_status` / `agent_cancel` 从「占位」变成真实操作；`HarnessAgentAdapter` 注入 `SessionManager`，`execute` 返回真实 `session_id`（`CHANGELOG.md:464`、`:467`）。

（对应版本：v2.9.9（`CHANGELOG.md:464`、`:467`）；v3.0.0（审批规则体系 + 注册表硬化 11aa58772，`CHANGELOG.md:140`））

## 关键代码入口

- 会话 / 压缩：
  - `axagent-ai/crates/agent/src/session_manager.rs:237`（`SessionManager`）、`:318`（impl）、`:779-781`（压缩配置与入口）、`:923-926`（阈值与 `Compacted`）
  - `axagent-core/crates/harness/src/session_events.rs:37`（`Compacted`）、`:79-113`（payload）
  - `axagent-runtime/crates/runtime-core/src/compact.rs`、`axagent-runtime/crates/runtime-core/src/reactive_compact.rs:73-111`、`axagent-runtime/crates/runtime-core/src/microcompact.rs`
- 工具 / 编排：
  - `axagent-runtime/crates/tools/src/registry.rs:1409-1474`（统一执行入口）
  - `axagent-runtime/crates/tools/src/orchestration.rs:61-68`（默认配置）、`:80-123`（分区执行）、`:127-164`（有序分批）
- 审批 / 沙箱：
  - `axagent-core/crates/harness/src/approval_policy.rs:17-61`
  - `axagent-core/crates/harness/src/approval_rules.rs:70-92`
  - `axagent-runtime/crates/rt-workflow/src/work_engine/executors/approval_executor.rs:16-95`
- 检索 / 记忆：
  - `axagent-infra/crates/search/src/rag_pipeline.rs:17-43`、`:97-122`
  - `axagent-ai/crates/agent/src/project_memory.rs:190-241`、`:328-334`
- 工作流：
  - `axagent-runtime/crates/rt-workflow/src/work_engine/executors/llm_resolve.rs:135-144`
  - `axagent-runtime/crates/rt-workflow/src/work_engine/engine/mod.rs`
  - `axagent-runtime/crates/rt-workflow/src/work_engine/node_executor.rs`

## 配置与开关

- 工具并发 / 重试：`max_concurrency = 10`、`timeout_secs = 120`、`max_retries = 3`、`retry_delay_ms = 1000`（`axagent-runtime/crates/tools/src/orchestration.rs:61-68`）；并发总开关 `TOOL_CONCURRENCY`（`axagent-runtime/crates/runtime-core/src/feature_flags.rs:233`）。
- 自动压缩阈值 `100_000` tokens（`axagent-ai/crates/agent/src/session_manager.rs:39`、`:923`）。
- 审批档位：`untrusted` / `on-failure` / `on-request` / `never`（`axagent-core/crates/harness/src/approval_policy.rs:48-61`）。
- 沙箱：`sandbox_mode` 由 Settings 写入全局槽位，`network_enabled` 与 `check_command` 是实际准入判据（`axagent-runtime/crates/tools/src/registry.rs` 顶部与 `:1467-1474`）。
- RAG：`top_k`、`rerank.candidate_k`、`min_score` 由调用方传入；语义是「先阈值筛，再截断 top_k」（`axagent-infra/crates/search/src/rag_pipeline.rs:90-122`）。
- 记忆：`MEMORY_INDEX_MAX_LINES`、`MEMORY_FILE_SIZE_LIMIT`、`MEMORY_RELEVANT_FILES_LIMIT`（`axagent-ai/crates/agent/src/project_memory.rs:145`、`:258`、`:334`）。

## 已知限制与坑

1. **`sandbox_permissions` / `escalated_exec` 在本仓代码里查不到**：Rust 与 TS 全量检索均无这两个标识。当前实现是「`ApprovalPolicy` 四档 + 全局沙箱准入检查 + 工作流审批节点」；若产品文档仍写这两个名字，属于**命名漂移**，以本文件所列代码为准（待确认）。
2. **`execute()` 不是全局严格顺序**：并发批结果先全部返回，串行批结果追加在后（`axagent-runtime/crates/tools/src/orchestration.rs:80-123`）。要保序必须用 `partition_ordered` 或按 `id` 回填；直接按下标取结果会把「先并发后串行」误读成原始调用序。
3. **并发安全是工具自述的**：`is_concurrency_safe()` 由各工具实现声明；声明错误的工具会被并发执行。新增有共享状态的工具必须返回 `false`（`axagent-runtime/crates/tools/src/registry.rs:concurrency_safe_tools`）。
4. **审批未注入 ≠ 全拒绝**：`GLOBAL_APPROVAL_RULE_STORE` 未注入时只是「不查规则、不沉淀」，不会把策略升级成一律拒绝；`GuardianBridge` 未注入时闸门不启用（`axagent-runtime/crates/tools/src/registry.rs` 顶部注释）。
5. **压缩阈值是常量不是面板变量**：`AUTO_COMPACTION_TOKEN_THRESHOLD` 写死在代码里（`axagent-ai/crates/agent/src/session_manager.rs:39`）；要调必须改代码并走版本门，不能指望设置面板。
6. **压缩 prompt 没有运行时版本号**：无法从 `Compacted` payload 判断「用的是哪版提示词」；只能靠提交追溯（70cac4b0a / 1faa79c68）。需要灰度时得自己加字段。
7. **RAG 的 Self-RAG 会整批过滤**：`judge_chunks` 返回的过滤结果可能为空；调用方必须处理「检索到但全部不相关」的空结果，不能当成「索引为空」（`axagent-infra/crates/search/src/rag_pipeline.rs:168-202`）。
8. **记忆写入超限直接报错**：`save_topic_file` 超过 `MEMORY_FILE_SIZE_LIMIT` 返回错误而不是截断（`axagent-ai/crates/agent/src/project_memory.rs:292-295`）；上层若忽略该错误会得到「看似保存成功但内容没落盘」。
9. **工作流审批依赖恢复令牌**：`resume_token` 由节点 id 拼接，重复挂起同一节点会产生新令牌；恢复方必须用挂起时返回的那一个（`axagent-runtime/crates/rt-workflow/src/work_engine/executors/approval_executor.rs:67`）。

## 相关文档

| 文档 | 状态 | 版本 | 关联 |
|---|---|---|---|
| `PLAN-codex-parity-adoption.md` | active | v2.9.9 | Codex 对齐项的审批规则 / Guardian 闸门 / 会话续写的上游依据 |
| `PLAN-repo-split.md` | active | v3.0.0+（未定版，待确认） | 8 个 workspace 根与路径迁移；本文件所有 `axagent-*` 路径均按拆分后目录书写 |
| `PLAN-plugin-gap-closure.md` | active | v3.0.0+（未定版，待确认） | 插件 / MCP / 技能装载的缺口收编；与第 7 节同一装载面 |
| `PLAN-unity-p3-llm-classifier-and-approval-gate.md` | 待确认 | — | LLM 分类器 + ApprovalGate 真实阻断的端到端计划；与第 3 节审批链路相关 |
| `PLAN-batch3-multiagent-workflow.md` | 待确认 | — | 多智能体协作 + DAG 工作流鲁棒性；与第 6 节工作流引擎相关 |
| `PLAN-knowledge-source-ingest.md` | 已完成（2026-08-01） | v1.1 | 知识源持续增长入口；与第 5 节知识库同步相关 |

## 版本变更记录

| 版本 | 日期 | 该功能点在本版本改了什么 | 证据（提交短哈希或 CHANGELOG 版本节） |
|---|---|---|---|
| v2.8.3 | 2026-07-11 | 工具并发编排已存在；本版仅修正 `truncate_string` 测试上限 | `2560ff868` |
| v2.9.0 | 2026-08-08 | 工作流人工审批节点落地（HomeRail 借鉴） | `796bdfa11`；`CHANGELOG.md:1092` |
| v2.9.6 | 2026-08-20 | 审批执行器类型驱动硬化（消灭魔法字符串） | `473042419` |
| v2.9.9 | 2026-09-23 | ApprovalPolicy 策略化审批 + TokenBudget 自动压缩 | `CHANGELOG.md:458`、`:461` |
| v2.9.9 | 2026-09-23 | MCP 服务端接入 SessionManager；`execute` 返回真实 `session_id`；会话 id 列表 DB 回退 | `CHANGELOG.md:464`、`:467`、`:472` |
| v2.9.9 | 2026-09-23 | 数据工具接入统一注册表 + 金融域路由优先级提升 | `CHANGELOG.md:475` |
| v3.0.0 | 2026-09-25 | 审批规则体系 + Guardian 审查闸门 + 注册表硬化 | `11aa58772`；`CHANGELOG.md:140` |
| v3.0.0 | 2026-10-30 | 会话压缩 / 会话记忆链收敛 | `70cac4b0a` |
| v3.0.0 | 2026-09-26 | 会话 compact 收敛 + 插件命名命令前端面 | `1faa79c68` |
| unreleased | 2026-11-01 | 代码按 8 个 workspace 根拆分（路径迁移，行为不变） | `8dfaeb4db` |
| unreleased | 2026-11-02 | `workflow.run` 宿主服务：异步受理 + owner 隔离 + 总超时 + 进度事件 | `35db96406` |
| unreleased | 2026-11-03 | 5 个公用件上收 harness（registry 相关依赖收敛） | `8d802bc6f` |
