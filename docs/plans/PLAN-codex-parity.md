> 来源：导入自上游 AxAgent（基准 99c9b96f，2026-09-07） ｜ 导入日期：2026-11-04

# PLAN-codex-parity — Agent 框架对标 Codex 补齐方案

> 依据：2026-09-03 对 openai/codex（codex-rs）与本项目 `src-tauri/crates` 的双端调研。
> 所有"现状"均经代码实证（文件 + 行号）。方案遵循裁剪优先、禁止重复定义（权威 DTO 归 `axagent-harness`）、UI Settings 可配置（feature flag）三项原则。

---

## 总览：分期与优先级

| 期 | 项目 | 对标 Codex 能力 | 新建代码量预估 |
|----|------|----------------|--------------|
| P0-1 | Windows 真沙箱（Restricted Token + JobObject） | Windows 沙箱 + sandbox mode 枚举 | ~600 行 |
| P0-2 | ApprovalPolicy 策略化审批（含 on-failure 升级） | approval policy 四档 | ~400 行 |
| P0-3 | 跨进程 Resume（session_events 事件流） | rollout 文件 + resume | ~500 行 |
| P0-4 | Headless CLI（crates/exec） | codex exec | ~450 行 |
| P1-1 | TokenBudget + 自动 compact 接线 | /compact 自动压缩 | ~250 行 |
| P1-2 | ApplyPatchTool（原子多文件 patch） | apply_patch 一等工具 | ~500 行 |
| P2-1 | MCP serve（把 agent 暴露为 MCP server） | MCP 双向 | ~300 行 |

依赖关系：P0-2 依赖 P0-1 的 SandboxMode 枚举；P0-4 依赖 P0-2/P0-3（CLI 要能传 policy、断点续跑）；P1-1 依赖 P0-3 的事件表（compact 记事件）。P1-2、P2-1 独立可并行。

---

## P0-1 Windows 真沙箱

### 现状（实证）
- `crates/runtime-core/src/sandbox.rs:341` `build_windows_sandbox_command` 仅**声明式**返回 `WindowsSandboxCommand { program: "cmd.exe /c", integrity_level: "Low", use_app_container: false }` —— 无任何实际隔离，是个幽灵开关。
- `crates/tools/src/job_object.rs:114` `assign_job(&Child)` 只做 KILL_ON_JOB_CLOSE 进程生命周期管理，唯一调用点 `tools/bash.rs`。
- macOS 已有 sandbox-exec + Seatbelt（`sandbox.rs:371`），Linux 是 unshare namespace（`sandbox.rs:291`，无 seccomp、无网络封锁）。

### 方案
1. **统一沙箱模式枚举**（定义在 `crates/harness/src/`，权威 DTO 层）：
   ```rust
   pub enum SandboxMode { ReadOnly, WorkspaceWrite, DangerFullAccess }
   pub struct SandboxPolicy { mode: SandboxMode, workspace_cwd: PathBuf, network_access: bool }
   ```
   macOS/Linux/Windows 三分支消费同一枚举，替换 `sandbox.rs` 现有的 `SandboxStatus` 判定输入。
2. **Windows 隔离实现**（新文件 `crates/tools/src/win_sandbox.rs`，`windows` crate 已在依赖树）：
   - `CreateRestrictedToken`：去掉 Builtin\Administrators 等高权 SID、加 deny-only SID；
   - `SetTokenIntegrityLevel` → Low（写不了用户文件，除非显式授权路径）；
   - 用 `CreateProcessAsUserW` 启动子进程，替换 bash.rs 现在直接 spawn 的路径；JobObject 保留叠加（管生命周期，职责不冲突）；
   - ReadOnly / WorkspaceWrite 差异用 DACL：WorkspaceWrite 时对 workspace_cwd 加 `ACCESS_MASK::GENERIC_WRITE` 的显式 allow ACE（临时目录同路径处理）。
   - **不做**：WFP 网络过滤（太重）。网络限制分两步走——阶段 1 靠 Low integrity（默认阻断多数网络位置）+ 文档声明；阶段 2 评估 `network_access=false` 时改走本地代理出口（gateway 已有鉴权层可复用）。
3. **Linux 补网络封锁**：unshare 基础上接 `seccomp` 过滤 `socket/connect`（`network_access=false` 时），~80 行，顺手补齐对标 Landlock+seccomp。
4. **Settings UI**：新增 `sandbox_mode`（下拉：ReadOnly/WorkspaceWrite/DangerFullAccess）+ `windows_sandbox_enabled` 开关；i18n 11 语言。
5. 命令执行统一改走 `sandbox.rs` 新入口 `run_sandboxed(policy, command)`，bash.rs 删掉平台 if 分支。

### 验收
- 测试：workspace 内可写、workspace 外写入被拒、Low integrity 进程 token 校验（`__TAURI_WORKSPACE__=true cargo test`）。
- DangerFullAccess 行为与现状一致（回归保证）。

### 风险
Restricted Token ACL 细节多，微软文档陈旧——先写 spike 验证 `CreateRestrictedToken` + DACL 组合，再落库。

### Spike 结论（2026-09-03，已定型 ✅）
`crates/tools/src/win_sandbox.rs` 落地并 3 测试 × 3 轮全绿。实测后方案与原计划有如下偏离：

| 原计划 | 实测定型 | 原因 |
|--------|----------|------|
| `CreateRestrictedToken` 去高权 SID + deny-only | **SAFER NormalUser**（`SaferCreateLevel` + `SaferComputeTokenFromLevel`，单层） | deny-only 用户 SID 层：①置于 SAFER 前被令牌派生重建丢失；②置于后 `CreateProcessAsUserW` 报 1314（本机标准用户令牌不持有 `SeAssignPrimaryTokenPrivilege`，whoami /priv 实证）。CRT 派生配方实测 0xC0000142/0xC0000022，排除 |
| `SetTokenIntegrityLevel` Low | SAFER NormalUser（等价 `runas /trustlevel:0x20000`） | Low integrity + restricted 组合排查复杂，SAFER 标准配方稳定；系统目录写入实测被拒 |
| WorkspaceWrite 用 DACL 对 workspace_cwd 加 allow ACE | 阶段 1 仅 ReadOnly；Profile 写保护留阶段 2 AppContainer/工作区 ACE | SAFER NormalUser 保留用户对自己 Profile 的写权限（实测），v1 边界如实界定为「系统目录受保护」 |

另两条硬性教训（已写入 win_sandbox.rs 模块文档）：
1. **`CreateProcessAsUserW` 的 cmdline `Vec<u16>` 必须追加 null 终止符**（`.chain(once(0))`）——collect 后容量==长度，堆残留旧字符串拼进子进程命令行，cmd `/s` 模式下产生随机「找不到路径」+ 回显垃圾字节；
2. **环境块必须按变量名大小写不敏感字母序排序并去重**（`Path`/`PATH` 变体并存会出问题）；
3. **私有 Window Station/Desktop 是必需项**：默认桌面 `WinSta0\Default` 的 DACL 不含受限令牌 check SID，conhost 初始化半途失败 → 输出堆垃圾。

状态：P0-1b ✅（spike 定型）；P0-1c（统一入口 + Settings UI + i18n + Linux seccomp）待做。

### P0-1c 完成记录（2026-09-03，✅）

| 项 | 实现 | 文件 |
|----|------|------|
| Settings 字段 | `sandbox_mode: String`（serde default = `"danger-full-access"`，零回归；JSON 键值存储免迁移） | harness/types/settings_chat.rs |
| 策略解析 | `SandboxMode::from_mode_str` / `SandboxPolicy::from_mode_str`（未识别值回退 DangerFullAccess）+ 单测 | harness/sandbox_policy.rs |
| 全局注入 | `GLOBAL_SANDBOX_POLICY` 静态（parking_lot RwLock）+ `set_/global_sandbox_policy()`；ToolContext 构建处实例级策略优先、回退全局 → 所有 `UnifiedToolRegistry::new()` 站点（含 per-request）自动生效 | tools/registry.rs |
| 注入点 | 启动初始化 + `save_settings`（改完立即生效，无需重启） | init/state.rs、commands/settings.rs |
| Linux 沙箱 | `unshare --user --map-root-user --mount --ipc --pid --uts --fork [--net]`（network_access=false 时 `--net` 断网）+ env 白名单 + kill_on_drop RAII；unshare 缺失显式报错不降级。**边界**：v1 文件系统写保护未实现（阶段 2 Landlock 补齐） | tools/linux_sandbox.rs（新） |
| bash.rs 统一 | 双平台 `SandboxWait` trait 折叠等待/超时/格式化为一份 `wait_sandbox_result` | tools/tools/bash.rs |
| Settings UI | AgentBehaviorSection 新增 sandboxMode Select（三档）+ 提示文案；11 语言 i18n（`advancedSettings.sandboxMode/sandboxModeHint/sandboxReadOnly/sandboxWorkspaceWrite/sandboxDangerFullAccess`） | AdvancedSettings.tsx、locales/*.json ×11、types/index.ts、settingsStore.ts |

验证：cargo check 零警告、clippy（harness/tools/axagent）`-D warnings` 零警告、harness 单测 5/5、tools bash 回归 6/6、tsc 通过、dprint 通过、i18n 11 语言全覆盖。

遗留（非本项引入）：settingsStore.ts L260-336 有 10 处既有 saveSettings 中文调试日志触发 i18n Rule 1 检查（console.*，非 UI 字符串），待后续清理。

---

## P0-2 ApprovalPolicy 策略化审批

### 现状（实证）
- 分类已有两层但无决策层：`bash/security.rs:137` `SecurityAnalyzer::analyze` → `SecurityResult { Safe, Dangerous, Blocked }`（L298）；`permissions/classifier.rs` `HeuristicClassifier::classify_bash` → `RiskLevel`，在 `tools/bash.rs:93/143` 做拦截。
- 审批只有一条 PlanCard 桥：`coordinator.rs:546` 发 `AgentEventType::PlanReadyForApproval` → oneshot `Sender<bool>` 存入 `app_state.rs:240` `agent_plan_approvals` → await。`tools/plan.rs:76` 注释自认"告知性 stub"。

### 方案
1. harness 新增：
   ```rust
   pub enum ApprovalPolicy { Untrusted, OnFailure, OnRequest, Never }
   ```
2. 决策函数（新文件 `crates/tools/src/approval.rs`）：
   ```
   analyze(command) -> SecurityResult   // 复用现有两层分类，不重写
   decide(policy, result, sandbox_mode) -> ApprovalDecision { SandboxRun, AskUser(reason), BlockOutside }
   ```
   - `Untrusted`：非 Safe 一律 AskUser；
   - `OnFailure`：沙箱内先跑，失败且失败原因是沙箱限制（写权限/网络）→ AskUser → 批准后**沙箱外重试一次**（这是 codex 的核心体验，现有桥完全能承载）；
   - `OnRequest`：Dangerous → AskUser；
   - `Never`：只在沙箱内跑，失败即失败。
3. 执行点：`tools/bash.rs` 在 L93/143 分类后插入 `decide()`；AskUser 复用 `agent_plan_approvals` oneshot 桥，但审批载荷从 `bool` 扩为 `{ approved: bool, allow_outside_sandbox: bool }`（oneshot 类型改 `AgentApprovalAnswer`，coordinator.rs:728 `approve_plan` 同步改）。
4. 前端：`PlanCard.tsx` / `PlanApprovalModal.tsx` 增加命令详情（命令文本 + 触发原因 + 沙箱限制说明）+ "仅此一次允许沙箱外执行"选项；`planStore.ts:116` `approvePlan()` 传新参数。Settings 增加 `approval_policy` 选择（默认 OnRequest），存 preference store。
5. `AgentEventType::PlanReadyForApproval` 事件 payload 扩展字段，前端按需消费（老字段保留，避免破坏）。

### 验收
- 单测覆盖 4 policy × 3 SecurityResult 决策矩阵；
- 现有 PlanCard 审批链路回归通过。

### P0-2 完成记录（2026-09-03，✅）

**实际定型 vs 原方案**（决策矩阵为准，与 Codex 语义对齐）：

| | Dangerous（含 Security Blocked / suggest_deny） | Suspicious（Warning 或 Medium/High） | Safe |
|---|---|---|---|
| **Untrusted** | 硬拒 | AskUser | 有沙箱→跑 / 无沙箱→AskUser |
| **OnFailure** | 硬拒 | 沙箱→跑 / 无沙箱→AskUser | 沙箱→跑 / 无沙箱→直跑 |
| **OnRequest**（默认） | 硬拒 | AskUser | 沙箱→跑 / 无沙箱→直跑 |
| **Never** | 硬拒 | 直跑 | 直跑 |

关键点：**危险操作任何策略下都硬拒**（`rm -rf /` 不因 Never 放行）；AskUser 复用现有 `ask_user_bridge`（前端 agent-ask-user UI 零改动），批准后按沙箱可用性分派。

**交付清单**：
- `crates/harness/src/approval_policy.rs`（新）：`ApprovalPolicy` enum（kebab-case serde）+ `from_policy_str`（未识别回退 OnRequest）+ 解析/序列化测试
- `crates/tools/src/approval.rs`（新）：`ThreatLevel`（两层分类归并 `merge_threat`）+ `ApprovalDecision` + `decide()` 纯函数 + 24 组合矩阵单测
- `crates/harness/src/tool.rs`：`ToolContext.approval_policy: Option<Arc<ApprovalPolicy>>`
- `crates/tools/src/registry.rs`：`GLOBAL_APPROVAL_POLICY` 静态 + set/global 函数对 + `UnifiedToolRegistry.approval_policy` 字段/setter + ToolContext 构建 `实例级 or_else 全局`
- `crates/tools/src/tools/bash.rs`：分类段改收集式 → `merge_threat` → `decide()` 分派；新 helper `run_direct`（原直通段提取）、`ask_user_approval`（无桥保守拒绝）；`run_sandboxed` 支持 OnFailure 沙箱内失败非零退出 → AskUser → 批准后 `run_direct` 沙箱外重试一次
- Settings 贯通：`AppSettings.approval_policy`（默认 "on-request"，serde default 免迁移）+ init/state.rs 启动注入 + save_settings 即时生效；前端 `types/index.ts` + `settingsStore.ts` 默认值 + `AdvancedSettings.tsx` approvalPolicy Select（四档）
- i18n：11 语言 `advancedSettings.approvalPolicy/Hint/Untrusted/OnFailure/OnRequest/Never`（无点 key，避开 i18next keySeparator 坑）

**验证**：`cargo check` harness+tools ✅；`cargo clippy -p harness -p tools -p axagent --all-targets --all-features -D warnings` ✅ 零警告；`cargo fmt` ✅；`tsc --noEmit` ✅；dprint ✅；tools approval 矩阵 11/11 ✅、tools::bash 回归 6/6 ✅（`bash_sandboxed_path_end_to_end` 在 Never 策略下通过）、harness approval_policy 解析 3/3 ✅。

**与原方案的偏离**：
1. 审批载荷未扩 `AgentApprovalAnswer`（{approved, allow_outside_sandbox}）——AskUser 走 ask_user_bridge 的问答选项（「批准执行」/「拒绝」），PlanCard/planStore 前端零改动，阶段 2 再做 PlanCard 命令详情展示。
2. OnFailure 的"失败原因是沙箱限制"语义分析暂未实现——当前以 `exit_code != 0` 粗判（stderr 语义分析留阶段 2）。
3. decide 的枚举命名为 `ThreatLevel{Safe,Suspicious,Dangerous}` + `ApprovalDecision{RunInsideSandbox,RunOutside,AskUser,Deny}`，与方案草稿命名不同但语义一致。

**遗留（阶段 2）**：PlanCard 审批 UI 展示命令文本+触发原因+沙箱限制说明；stderr 语义化失败归因；`settingsStore.ts` L260-336 十处既有中文调试日志 i18n 违规（非本次引入）。

---

## P0-3 跨进程 Resume（session_events）

### 现状（实证）
- `coordinator.rs:33` `AgentStatus { Idle, Initializing, Running, WaitingForConfirmation, Paused, Completed, Failed }`；`resume()`（L837）只校验 Paused 后调 `impl_guard.resume()`——**进程退出即丢执行态**。
- 持久化粒度：`trajectory/src/storage.rs:86-117` `trajectories` 表每条轨迹一行（session_id/topic/summary/outcome），无逐 turn 事件流；`AgentSessionRepository`（`harness/src/agent_session_repo.rs:16`）只存会话元数据。

### 方案
1. **migration 新增 `session_events` 表**（不引入 JSONL rollout 双持久化——SQLite 已是权威存储，加文件违反裁剪原则）：
   ```
   session_events: id PK, session_id, seq, event_type, payload(JSON), created_at
   ```
   事件类型：`TurnStarted / Message / ToolCall / ToolResult / Compacted / TurnEnded / Interrupted`。
2. 写入点：react_engine 主循环（`react_engine.rs:694` while 循环）在 `process_state()`（L1124）各分支发事件，经 `SessionManager` 落表；实现 `SessionEventSink` trait（harness 定义，agent 依赖 trait）。
3. **恢复入口**：新命令 `agent_resume_from_events(session_id)`：
   - 读事件流重放构建 `ThoughtChain`（ToolCall/ToolResult 对可恢复 observation，未配对的 ToolCall 视为 Interrupted）；
   - `ContextWindow::from_chain`（`context_window.rs:51`）重建上下文；
   - coordinator 从 `Resuming` 状态直接进主循环下一轮。未完结 session（有 TurnStarted 无 TurnEnded）在前端会话列表标"可恢复"。
4. 前端：会话列表加恢复入口；`Interrupted` 的工具调用恢复时提示用户（可以选择重发或跳过）。
5. 清理：session 正常完成后按现有 retention 策略归并事件（防表膨胀），只保留最近 N 个未完结 session 的全量事件。

### 验收
- 集成测试：跑到一半 kill 进程 → 重启 → resume → 上下文含已完成步骤的 observation。

---

## P0-4 Headless CLI（crates/exec）

### 现状（实证）
- `src-tauri/src/main.rs` 仅 Tauri 入口；全仓无 clap/CLI bin。约 1115 个 Tauri 命令全经前端 IPC。
- 关键前提待验证：`crates/agent` 是否干净无 tauri 依赖（库 crate，大概率干净；若不干净，先解耦——把 tauri 触达点挪到 src-tauri 命令层，这本身就是架构收益）。

### 方案
1. 新 crate `crates/exec`（members 加一行），依赖 agent + tools + providers + trajectory + harness，**不依赖 tauri**：
   ```
   axagent-exec "任务描述"
     --sandbox workspace-write --approval on-failure
     --cwd <dir> --provider deepseek --json    # JSON Lines 输出 AgentEventType
     --resume <session_id>                      # 对接 P0-3
   ```
2. 组装路径：复用 `SessionManager::run_turn_with_tools`（`session_manager.rs:609`）+ `ProviderRegistry`；审批在 CLI 下退化实现——`AskUser` 输出到 stderr 等 stdin 确认（`--approval never` 时直接拒绝）。
3. JSON 输出复用 `AgentEventType` 的 serde 序列化（harness 权威 DTO，不新建事件类型）。
4. 与桌面版共存：同一 SQLite（路径参数 `--db`），同一 settings 语义。CI 可直接用（`npm run test:e2e` 之外新增 smoke 脚本调 axagent-exec）。

### 验收
- `axagent-exec "列出 TODO" --sandbox read-only --json` 端到端跑通并输出事件流；
- `--resume` 能续上 P0-3 测试中断的会话。

---

## P1-1 TokenBudget + 自动 compact

### 现状（实证）
- `trajectory/src/compactor.rs:181` `SessionCompactor::compress(&self, messages: &[MessageRecord]) -> CompressionResult` 是**纯函数，生产零调用**（仅 lib.rs:93 导出）——现成的轮子没装上车。
- 上下文只在 think/plan/reflect/synthesize 前重建窗口（react_engine.rs L411/438/477/506 `ContextWindow::from_chain`），无预算判定。

### 方案
1. harness 新增 `TokenBudget { max_context, reserved_output, compact_threshold_pct }`；估算器起步用 chars/4（中文按 chars/2 修正），预留 `Tokenizer` trait 后续换真 tokenizer。
2. 接线点：react_engine 主循环每轮 observation 回填后（L1222-1228 附近）检查预算 → 超阈值调 `SessionCompactor::compress` 压缩早期步骤 → 压缩结果替换 `ThoughtChain` 对应段 + 发 `Compacted` 事件（进 P0-3 事件表）。
3. Settings：`auto_compact_enabled`（默认开）+ 阈值滑条；关闭时行为与现状一致。

### 验收
- 单测：构造超长 chain，验证 compact 触发、事件落表、压缩后仍能正常续跑。

---

## P1-2 ApplyPatchTool

### 现状（实证）
- 文件工具逐个读写，无原子多文件修改语义；`shadow_fs.rs` 已有（可做快照回滚，现在是权限模拟用途）。

### 方案
1. 新工具 `crates/tools/src/tools/apply_patch.rs`：
   - 输入：git-style unified diff（生态兼容，不发明 codex 的 V4A 格式——diff 解析库 `similar` 已在依赖树的概率高，若无则手写 ~200 行）；
   - 执行：apply 前对涉及文件做内存快照 → 逐文件应用 → 任一失败全部回滚（原子性）；
   - 返回结构化结果（应用/跳过/失败文件清单）。
2. 注册进 registry；**不加入** `concurrency_safe_tools()`（写工具不并发）。
3. 系统 prompt 补一段 apply_patch 使用指引（对应 codex 的 `apply_patch_tool_instructions.md` 收敛做法）。
4. 与审批联动：apply_patch 走 P0-2 `decide()`，WorkspaceWrite 沙箱内自动执行。
5. i18n 11 语言 + Vitest 前端展示（ToolNode diff 渲染可复用现有文件工具卡片）。

---

## P2-1 MCP serve

### 现状（实证）
- `crates/mcp/src/mcp_client.rs:16` `McpPeer = rmcp::service::Peer<RoleClient>`，传输层 stdio（`TokioChildProcess` L617）+ StreamableHttp（L9）——rmcp 的 Peer 抽象天然双向，serve 端是同一 SDK 的另一半。

### 方案
1. `crates/mcp` 新增 `server.rs`：rmcp `serve_server` + stdio transport，暴露三个 tool：
   - `agent_run(prompt, session_id?)` → 创建会话跑一轮（复用 SessionManager）；
   - `agent_status(session_id)` / `agent_cancel(session_id)`。
2. 入口命令 `axagent-mcp-serve`（挂 crates/exec 的 bin，或独立小 bin），使 Claude Desktop / 其他 MCP 宿主可直接把 AxAgent 当工具调用。
3. 复用 `global_mcp_pool()` 的健康监控语义给 server 侧加就绪检查。

---

## 执行顺序与验收门禁

```
P0-1 spike（Windows restricted token 验证，1 次提交）
  → P0-1 落地 + P0-2（决策矩阵共用一批测试）
  → P0-3 → P0-4（依赖前两者）
  → P1-1 → P1-2 → P2-1
```

每期验收门禁（AGENTS.md 铁律）：
- `cargo check` 秒级零错 → `cargo clippy --workspace --all-targets --all-features -D warnings` 零警告（禁止 `#[allow(dead_code)]`，无用代码直接删）
- `cargo fmt` + `npm run format`（dprint）通过
- `__TAURI_WORKSPACE__=true cargo test` 相关模块全绿
- 新增 UI 文本 11 语言 key 全覆盖；新增 Tauri 命令同步 mod.rs + 宏（build.rs 自动生成，改完重编译即可）
- 迁移一律写 migration，禁止直接改 entity

## 风险表

| 风险 | 影响 | 缓解 |
|------|------|------|
| Windows restricted token + DACL 组合行为不可预期 | P0-1 卡壳 | 先 spike 单独验证，不行降级为 AppContainer LPAC 或声明"Windows 建议在容器内跑 exec" |
| crates/agent 隐含 tauri 依赖 | P0-4 解耦工作量放大 | 第一步用 grep 验证；解耦本身列为独立收益提交 |
| compact 接入改变 prompt 质量 | P1-1 回归 | feature flag 默认开但保留 `auto_compact_enabled=false` 逃生门；对瓶颈掘金工作流跑回归 |
| session_events 表膨胀 | P0-3 存储 | retention 归并策略在 migration 同期实现 |
| apply_patch 与现有 file 工具双轨 | 工具面冗余 | apply_patch 只补"原子多文件"场景，单文件小编辑仍走现有 write/edit，系统 prompt 明确分工，不删旧工具 |
