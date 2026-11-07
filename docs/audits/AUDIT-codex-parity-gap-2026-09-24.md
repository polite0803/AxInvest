# AUDIT：openai/codex 与本项目后端的机制级差距对照

> 状态：active ｜ 日期：2026-09-24 ｜ 对应版本：v2.9.9


- 日期：2026-09-24
- 对象：`openai/codex` 的 `codex-rs`（commit `282cd7b019378746cb87bd91a95d8b4bcae12aa3`，2026-09-24），约 100 个 crate
- 对照方：AxAgent 后端（`src-tauri/`，36 crate 工作区）
- 方法：**实读 codex 源码**（浅克隆 `--filter=blob:none --sparse` 仅取 `codex-rs`），
  项目侧结论来自实读 + grep 复核；凡未逐字核实的项在 §11 显式列出
- 定位说明：codex 为外部仓库，本文其行号统一写作 `路径 Lx-Ly`（**不带冒号**），
  以免被本仓「文档源码引用体检」误判为断链；项目侧引用写完整仓库相对路径
- **增补（同日晚）**：§7–§10 为本轮深挖新增（沙箱系统调用细节 / Windows 服务模型 /
  guardian 预算与契约 / execpolicy 匹配算法），原 §7「未核实项」收敛为 §11。
  本轮由子代理并行读码产出，**承重结论已由本人定点复核**，复核范围见 §7.0。

---

## 0. 先纠上一版的错

### 0.1 项目 `OnFailure` 档是空承诺

`src-tauri/crates/harness/src/approval_policy.rs:20-21` 的文档写着 OnFailure =
「沙箱内先跑；失败且疑似沙箱限制时询问用户批准后沙箱外重试一次」。

但 `src-tauri/crates/tools/src/approval.rs:75-111` 的 `decide()` 入参只有
`(policy, threat, sandbox_active)` —— **没有任何「这是第几次尝试 / 是否已升级」的状态**，
因此「失败后重试」在决策层无法表达。全仓 `decide()` 只有一个调用点
（`src-tauri/crates/tools/src/tools/bash.rs:194`），其分支只做「沙箱内跑」或「直通」，
没有重试回路。

对照 codex：`codex-rs/core/src/guardian/decision.rs L45-L131` 的决策入参包含
`reasons.retry`、`escalated_exec`（是否已升级权限）、`full_access`、`require_guardian`、
`model_requires_review`、`require_synchronous_review`，返回 `Option<ReviewDecision>`，
且注释明确 **"No contributor is never an implicit allow"**（无审查者 ≠ 默认放行，fail-closed）。
其中 `escalated_exec` 由 `sandbox_permissions.requires_escalated_permissions()` 推导
（同文件 L113）—— 「这次是沙箱被拒后的升级尝试」是显式布尔量，而非靠错误字符串猜。

**结论：差距不是「三档 vs 四档」，而是「有状态决策 vs 无状态纯函数」。**
纯函数做不了升级重试，也做不了「同一命令第二次问法不同」。

### 0.2 项目里模型不知道自己在沙箱里

对 `src-tauri/src/context_manager.rs` grep `sandbox|沙箱|approval|审批`：**零命中**。

而 codex 把沙箱/审批档位直接写成 system prompt 模板：
`codex-rs/prompts/src/permissions_instructions.rs L27`（`include_str!` 引入
`templates/permissions/approval_policy/on_request_rule_request_permission.md`），
模板目录 `codex-rs/prompts/templates/permissions/` 下含
`approval_policy/{never,on_request,on_request_rule_request_permission,unless_trusted}.md`
与 `sandbox_mode/{read_only,workspace_write,danger_full_access}.md`。

后果是行为级的：codex 的模型被告知边界后会主动规避越界写路径、主动请求审批；
项目的模型只能盲试，撞拒绝后才在下一轮收到错误。

---

## 1. turn 的形态：会话对象 vs 一次性 RPC

| 维度 | codex（已核实） | AxAgent（已核实） |
|---|---|---|
| turn 归属 | `Session` 持有 `submission_loop`，跑到 `Op::Shutdown`（`core/src/session/mod.rs` L924-L930） | 每 turn 一个 Tauri 命令 `agent_query`（`src-tauri/src/commands/agent/mod.rs:558-680`） |
| turn 内继续收输入 | 支持：`TurnInput::{UserInput, FunctionCallOutput, ResponseItem, InterAgentCommunication}` 统一入队（`core/src/session/input_queue.rs` L22-L34）+ `inject.rs` | 不支持，turn 期间是黑箱 |
| 循环终止条件 | `needs_follow_up = model_needs_follow_up \|\| has_pending_input`、`token_limit_reached`（`core/src/session/turn.rs` L163-L170, L423） | turn 内自循环 |
| 取消语义 | `CancellationToken` 逐层下发至子进程（`core/src/session/turn_suspension.rs` L13-L118） | 运行态表 + 10 分钟超时清理（`src-tauri/src/commands/agent/mod.rs:678-720`） |

**实质差别**：codex 的 `TurnInput` 把「用户新输入 / 工具结果 / 子 agent 消息」放进
**同一条队列** —— 于是 steering、多 agent 通信、工具回填共用一套机制；项目里这三件事
各有各的路径，导致「turn 中途改变意图」与「agent 间通信」都无处安放。
超时兜底 vs 精确取消的差别，在长任务（编译、跑测试）上直接表现为残留子进程。

另有独立**任务层**：`SessionTask` trait（`run`/`abort`/`span_name`）+ regular/compact/
review/user_shell 四种实现（`core/src/tasks/mod.rs` L178-L218）。压缩与 review 是
**一等任务**，可单独取消、单独追踪；项目里压缩只是若干函数，无法被单独取消或观测。

---

## 2. 上下文与预算：模型可查询剩余额度

- codex：`core/src/state/turn_token_usage.rs` L11-L22 按模型聚合 `TokenUsage` 并打点；
  **`get_context_remaining` 是一个工具**（`core/src/tools/handlers/get_context_remaining.rs`），
  模型能主动询问剩余窗口；阈值来自模型信息/配置
  （`core/src/session/context_window.rs` L108-L110 的 `buffered_auto_compact_limit`）。
- 项目：`src-tauri/src/context_manager.rs:24-37` 硬编码
  `SYSTEM_PROMPT 8000 / RETRIEVED_MEMORIES 10000 / SKILLS 5000 / HISTORY_RATIO 0.65` + 70% 阈值。

差别不在「常量 vs 配置」，而在于：**常量与真实模型窗口解耦**（换 200k 窗口模型行为不变），
且**模型对预算无感知**，只能被动被裁 —— codex 把「感知预算」做成了工具，模型可据此自行
决定收敛还是请求压缩。

压缩可回放：codex 把压缩记成 rollout 事件（`RolloutItem::Compacted`），并区分本地 compact
与服务端 `compact_remote_v2` 两条路径；压缩 prompt 本身是版本化文件
（`codex-rs/prompts/src/compact.rs` L1-L2）。项目是 `<!-- context-compressed -->` 锚点 +
并存 4 个 compact 实现，压缩动作不进入可回放事件流 ⇒ 重放时无法还原「当时模型看到的是摘要」。

---

## 3. 工具 / 审批 / 执行：codex 是闭环，项目是点

### 3.1 并行工具调用

- codex：`RwLock` 表达「可并行的工具共享读锁、必须独占的工具取写锁」
  （`core/src/tools/parallel.rs` L132-L206），流结束后按调用顺序回填结果 + 记录
  `result_ready`（同文件 L226-L239）。
- 项目：模型返回的多个 tool call 是**顺序 for 循环**执行
  （`src-tauri/crates/agent/src/provider_adapter.rs:590` →
  `src-tauri/crates/agent/src/action_executor.rs:43-65` 一次只处理一个 Action），无并行。

### 3.2 execpolicy 是三态且策略可自我修订

- codex：`Decision::{Forbidden, Prompt, Allow}` → `ExecApprovalRequirement::{Forbidden,
  NeedsApproval, ...}`（`core/src/exec_policy.rs` L394-L435）；
  `MatchOptions{ resolve_host_executables: true }` 做可执行文件身份解析；
  `derive_requested_execpolicy_amendment_from_prefix_rule`（同文件 L381-L392）
  **把「这次用户批准的前缀」沉淀成策略修订建议**，下次不再问。
- 项目：只有 `src-tauri/crates/kit/src/command_validator.rs` 的黑名单式判定 +
  `decide()` 的威胁分级，**没有「批准后沉淀为规则」的通路** —— 用户每次都要重新批准
  同一条命令。

### 3.3 输出截断策略

- codex：`HeadTailBuffer` 刻意 50% 头 + 50% 尾、中间丢弃并留 `omitted_bytes` 标记
  （`core/src/unified_exec/head_tail_buffer.rs` L5-L19, L45-L48）—— 因为「命令横幅在头、
  最终报错在尾」才是关键信息；`write_stdin` 写入前还过 `stdin_approval`
  （`core/src/unified_exec/process_manager.rs` L883-L969）。
- 项目：常规全量采集 + 截断。

---

## 4. 协议 / 传输 / 配置

### 4.1 feature flag 带生命周期元数据

`Stage::{UnderDevelopment, Experimental{name, menu_description, announcement}, Stable,
Deprecated, Removed}`（`codex-rs/features/src/lib.rs` L56-L88）—— 开关自带
「实验菜单文案 / 是否公告 / 是否废弃」。项目无统一 flag 体系。

### 4.2 AGENTS.md 是有预算的分层拼接

codex（`core/src/agents_md.rs` L1-L18, L43-L95）：

1. 从 cwd 上溯至 project root marker（默认 `.git`），**不越过 root**；
2. 从 root 向下依序拼接，分隔符 `\n\n--- project-doc ---\n\n`（L49）；
3. `project_doc_max_bytes` 递减式消费预算（L68-L95）；
4. `AGENTS.override.md` 作本地覆盖（L45）；
5. **不可信项目直接跳过项目文档**（L64-L66）。

本项目对照：本仓 `AGENTS.md` 自述「按文件名**全量加载**」，并警告「175 条判据 ≈ 61 KB，
全量加载会挤占每次会话的上下文预算」。即：项目已识别该问题，codex 用
「字节预算 + 分层 + override + 不可信跳过」正面解决。

### 4.3 传输层

- codex：WebSocket 优先、失败回落 HTTP 且打点
  （`core/src/client.rs` L650-L669）；重试消费**服务端建议的 `retry_after`**、
  退避上限 60s，`Feature::UnboundedConnectionRetries` 时对连接失败改为「等网络恢复」而非失败
  （`core/src/responses_retry.rs` L52-L151）；WS 请求带 `previous_response_id` 且**只发增量
  items**（`client.rs` L1984-L2011），并带 `store:false`、`include:["reasoning.encrypted_content"]`、
  `prompt_cache_key`（`client.rs` L961-L1008）。
- 项目：`previous_response_id: None` 出现在 11+ 处（`src-tauri/src/memory_extract.rs` ×4、
  `src-tauri/src/indexing.rs`、`src-tauri/crates/agent/src/wiki_compiler.rs` ×3、
  `src-tauri/src/init/agent_turn_adapter.rs`、`src-tauri/crates/agent/src/vision_pipeline.rs`、
  `src-tauri/crates/agent/src/tree_of_thoughts.rs` 等），无 `prompt_cache_key`、无 reasoning
  复用、无 WS 传输、无服务端重试建议消费。

公允说明：项目有 `src-tauri/crates/runtime-core/src/prompt_cache.rs` 做**缓存断点检测**
（预期缓存读取 token vs 实际，<50% 判为异常断点）。这份可观测性不错，但它只能「发现没命中」，
codex 是**用缓存亲和键 + 增量续传保证命中** —— 前者是仪表盘，后者是控制回路。

### 4.4 事件契约的投影方式

- codex：内部 `EventMsg` 单一枚举，到对外协议由 `item_event_to_server_notification`
  投影成 `ServerNotification`
  （`codex-rs/app-server-protocol/src/protocol/event_mapping.rs` L25-L34）；
  app-server 明确**不是**标准 JSON-RPC 2.0（不使用 `jsonrpc` 字段，
  `codex-rs/app-server-protocol/src/rpc.rs` L1-L60），未知方法返回 `-32601`
  （`codex-rs/app-server-client/src/remote.rs` L363-L405）。
- 项目：emit 字符串事件名 + `scripts/check-contracts.mjs:318-388` 事后脚本校验对称性。
  codex 是**编译期 match 穷举**，字符串只存在于最外层传输。

---

## 5. 归因：为什么这套设计能支撑「第一」

不是单点黑科技，而是**七环闭环**，每环都能兜住下一环的失败：

`prompt 告知边界` → `execpolicy/guardian 有状态决策` → `sandbox 内核级约束` →
`拒绝可识别 → 升级重试` → `批准沉淀为策略修订` → `rollout 全程可回放` →
`传输层增量续传 + 可观测/灰度`

项目现状是「每环都实现了判定，但环与环没接上」：

- 有 policy 矩阵，但无重试状态；
- 有沙箱，但模型不知道；
- 有事件流，但无编译期契约；
- 有缓存检测，但无命中保证。

**须补一句限定（本轮深挖后）**：「拒绝可识别」这一环在 codex 里**并非内核级协议，
而是启发式**（退出码快路 + 7 条关键词）。详见 §7.6 —— 它同时**降低了本项目兑现
重试回路的门槛**：不需要先造结构化拒绝码。

---

## 6. 可执行借鉴清单（按性价比排序）

| 优先级 | 动作 | 最小改动面 |
|---|---|---|
| P0 | 给 `decide()` 加「尝试状态」入参（`Attempt::{Initial, AfterSandboxDenial}`），兑现 OnFailure 重试 | `src-tauri/crates/tools/src/approval.rs:75-111` 签名 + `src-tauri/crates/tools/src/tools/bash.rs:194-231` 单调用点；测试矩阵扩一维 |
| P0 | 同上一行配套：新增 `is_likely_sandbox_denied(exit_code, stderr)` 判据（**降本**：codex 自己也是启发式，见 §7.6，无需先造结构化拒绝码） | 照抄 §7.6 的两级判据：exit code 快路（排除 2/126/127）+ 7 关键词小写匹配；放 `src-tauri/crates/tools/src/` 内 |
| P0 | 把当前沙箱/审批档位写进 system prompt | `src-tauri/src/context_manager.rs` 增加按档位渲染的说明段（模板化，勿硬编码文案） |
| P1 | 事件契约枚举化：emit/`listen` 双侧由同一枚举生成 TS 字面量类型 | 复用已有 `src-tauri/schema-gen` + `src/types/generated/ipc.ts` 生成机制，与 `scripts/check-contracts.mjs` 并存过渡 |
| P1 | 预算改为按模型窗口取；新增「剩余额度可查询」工具 | `src-tauri/src/context_manager.rs` 常量降级为默认值；新增工具 |
| P1 | 「批准沉淀为规则」的最小形态（见 §10.5）：首 token 精确索引 + 多命中取最严 + 危险包装器黑名单 + **模拟写入后复算本次全部子命令段** + 落盘去重 | 在 `src-tauri/crates/kit/src/command_validator.rs` 旁新增规则表与匹配器；审批结果回调处追加落盘 |
| P2 | Windows 沙箱补三件（见 §8）：① 令牌从 SAFER 改为 capability SID + `CreateRestrictedToken(DISABLE_MAX_PRIVILEGE\|LUA_TOKEN\|WRITE_RESTRICTED)`；② `WorkspaceWrite` 用工作区 allow ACE；③ 网络用 WFP + Defender 防火墙**双层**（单 WFP 不够） | `src-tauri/crates/tools/src/win_sandbox.rs`；注意 `WRITE_RESTRICTED` 只作用于写检查，读限制须走提权后端 |
| P2 | guardian 的最小形态（见 §9）：一个 `{outcome: allow\|deny}` 必填的 JSON 契约 + fail-closed 失败矩阵 + `max_attempts=3`/90s deadline + 输入侧预算；**不做**会话复用与异步预评分 | 复用 `src-tauri/crates/agent/` 现有 provider 调用；新增 `ext` 之外的轻量审查模块 |
| P3 | 压缩动作进入可回放事件；压缩 prompt 文件版本化 | 落事件 + `include_str!` 模板 |
| P3 | OpenAI 路径启用 `previous_response_id` 增量 + `prompt_cache_key`；bash 输出改 head/tail 对称截断 | 前者改请求构造点；后者改输出截断函数 |

**明确不建议照搬的两项（成本/收益不划算）**：

- **常驻 SYSTEM 服务**（`windows-sandbox-service`，见 §8.6）：它是「免 UAC」的代价对冲，
  但引入常驻特权进程 + 具名管道五重鉴权 + 企业策略否决面 + 复杂卸载回滚。
  本项目若不需要「执行期零 UAC」，不值得付这个复杂度。
- **自带 bubblewrap 二进制**（见 §7.1）：收益是把「依赖用户环境的系统 bwrap」变成可控，
  但代价是要随包分发并在 exec 前做 SHA256 自校验。项目当前 `linux_sandbox.rs` 依赖系统
  unshare，是否升级取决于是否需要支持无 bwrap 的发行版。

---

## 7. 沙箱实现细节：Linux / macOS 到底做了什么

### 7.0 本轮方法与复核范围

本节及 §8–§10 由子代理并行读码产出。**本人定点复核过以下承重文件**（其余定位来自
子代理，未逐行复核，但不作改造依据的结论不在此列）：
`sandboxing/src/denial.rs`、`execpolicy/src/decision.rs`、`ext/guardian-reviewer/src/assessment.rs`、
`ext/guardian-reviewer/src/completion.rs`、`ext/guardian-reviewer/src/lib.rs`、
`linux-sandbox/src/bwrap.rs`、`linux-sandbox/src/landlock.rs`、
`windows-sandbox-rs/src/token.rs`、`execpolicy/src/policy.rs`、`execpolicy/examples/example.codexpolicy`。

### 7.1 原语选型：不自研 unshare，改为 helper 进程 exec bubblewrap

`linux-sandbox` 是一个独立 helper 二进制（argv0 为 `codex-linux-sandbox`），职责是把策略
**翻译成 bwrap argv 并 exec**，命名空间全部由 bwrap 创建。全盘可写分支的固定 flag 为
`--new-session --die-with-parent --bind / / --dev /dev --bind-try /dev/shm /dev/shm
--unshare-user --unshare-pid --unshare-ipc`，再按模式追加 `--unshare-net`、`--proc /proc`，
最后 `--cap-drop ALL`（`linux-sandbox/src/bwrap.rs L280-L317`）。

其中 `--unshare-user` **即使容器内已是 uid 0 也显式要求**，注释写明原因是「容器内的 root
不必持有 ambient CAP_SYS_ADMIN 也能创建其余 namespace」（同文件 L294-L296）。这是个很实用
的细节：在容器里跑 agent 时不能假设有 CAP_SYS_ADMIN。

PID namespace 下必须有 pid 1 回收僵尸，做法是 exec bwrap 前**无条件插入 `--as-pid-1`**，
让 bwrap 充当 pid 1 去 reap，helper 自身只做信号转发与 `waitpid`
（`linux-sandbox/src/launcher.rs L38-L57`）。

bwrap 二进制的来源是「**系统优先 + 内置兜底**」：先 `find_system_bwrap_in_path()`，用
`bwrap --help` 探测能力（缺 `--perms` 直接弃用），系统不可用才用随包分发的
`codex-resources/bwrap`；内置二进制在 exec 前做 **SHA256 摘要校验**，不符即带专用退出码退出
（`linux-sandbox/src/launcher.rs L126-L205`、`linux-sandbox/src/bundled_bwrap.rs L28-L46`）。

**找不到任何 bwrap 时直接 panic，不存在「无沙箱静默降级」**（`launcher.rs L50-L55`）。
这一点与本项目直接相关：`src-tauri/crates/tools/src/linux_sandbox.rs:12-16` 自述依赖系统
`unshare`，而 codex 用「自带二进制」把这一依赖从用户环境里拿掉了 —— 代价是随包分发 + 自校验。

### 7.2 Landlock 已退居 legacy

结构性事实：**默认文件系统隔离由 bwrap 承担，Landlock 只在 legacy 路径**，且注释明确
「bwrap 失败绝不回落 legacy Landlock」（`linux-sandbox/src/linux_run_main.rs L289-L292`）。

legacy 路径用 ABI `V5` 硬编码 + `CompatLevel::BestEffort` + `AccessFs::from_all(V5)`，
规则形态是**白名单式放开**（`/` 只读、`/dev/null` 读写、各 writable root 读写），
而不是逐条 deny（`linux-sandbox/src/landlock.rs L147-L173`）。降级语义是「部分不支持则降级、
完全不支持才失败」：BestEffort 让 `landlock` crate 与内核协商出子集，随后**显式检查
`status.ruleset == NotEnforced` 并返回 `SandboxErr::LandlockRestrict`**（同文件 L166-L172）。

另有两条硬约束值得记：legacy 模式**不支持「限制读取」**（L80-L86 直接返回
UnsupportedOperation）；legacy 与受限文件系统组合会被 panic 拒绝（L405-L412）。
因此「本项目 `linux_sandbox.rs` 计划接 Landlock」这条路，在 codex 的经验里只到 legacy 级别 ——
真做全后应当考虑 bwrap。

### 7.3 网络封锁：namespace + seccomp 双保险，共三种模式

网络是**双保险**：先 `--unshare-net` 造出无网卡的网络 namespace，再在 namespace **内层**
用 seccomp BPF 兜住 socket 创建，两层不能被单独绕过。seccomp filter 的默认动作是
`Allow`、**命中即返回 `EPERM`**（不是 SIGSYS），目标架构只支持 x86_64 / aarch64
（`linux-sandbox/src/landlock.rs L282-L297`）。

三种模式（`linux-sandbox/src/landlock.rs L201-L280`）：

| 模式 | socket 家族策略 | 其余拦截 |
|---|---|---|
| `Restricted`（完全禁网） | `socket`/`socketpair` **仅当 `arg0 != AF_UNIX` 才拒**（只放行 Unix domain socket，L221-L231） | deny `connect/accept/accept4/bind/listen/getpeername/getsockname/shutdown/sendto/sendmmsg/recvmmsg/getsockopt/setsockopt`（L203-L219） |
| `ProxyRouted`（只允许经代理） | 反之**只放行 AF_INET/AF_INET6**，使沙箱内进程能打到本机 loopback 的代理桥（L233-L269） | 同上 |
| `VmSocketRestricted` | 只拒 `AF_VSOCK`（L270-L279） | 仅此（用于「全盘可写但切断 WSL 宿主桥」） |

三种模式**共同**拦截 `ptrace` / `process_vm_readv` / `process_vm_writev`，以及三个
`io_uring_*`（L190-L199）—— 后者的理由写在注释里：**io_uring 可以在不调用 `socket()` 的情况下
创建 AF_VSOCK socket**（L195-L196），所以必须单独封。

一个很能说明「调过参」的取舍：`recvfrom` **刻意不拦**，注释写明是为了让 `cargo clippy`
这类靠 socketpair + 子进程做子进程管理的工具能跑（L213-L216）。这说明网络封锁粒度是按真实
工具链行为反复调出来的，不是一次写死。

`network-proxy` crate 负责代理本体与策略（socks5 / upstream / mitm / credential_broker /
environment_policy），沙箱侧只消费它给出的 loopback 端口与 Unix socket 白名单
（`linux-sandbox/src/landlock.rs L252-L268`；宿主侧搭桥见 `linux-sandbox/src/proxy_routing.rs L29-L67`）。

### 7.4 挂载层序与「显式遮蔽」

挂载策略有明确的**层序不变量**：默认 `--tmpfs /` 起手（受限读）；全读策略或显式读 `/`
才 `--ro-bind / /`；`--dev /dev` 给最小可写设备；可写根用 `--bind root root` 重新开写；
其下受保护子路径再 `--ro-bind` 复压只读。

关键差异：**不可读路径不是「不加规则」，而是显式遮蔽** —— 目录用
`--perms 000|111 --tmpfs <path> --remount-ro <path>`，单文件用 `--perms 000` + 空文件
bind-data；若存在可写后代则用 `111` 以保留穿越能力（`linux-sandbox/src/bwrap.rs L404-L420`、`L1324-L1366`）。

特权 app-server 的 socket 目录会被单独 `--tmpfs --remount-ro` 遮蔽，并用 `/proc/self/mountinfo`
找出**同一文件系统的全部别名一并遮**（`bwrap.rs L825-L857`、`linux-sandbox/src/daemon_mounts.rs L16-L71`）
—— 防的是「换个挂载点路径绕过遮蔽」。

宿主 fd 传入沙箱走 `--ro-bind-fd <fd> <dest>`；旧版 bwrap 不支持时降级翻译为
`--ro-bind /proc/self/fd/<fd> <dest>` 并追加 `--verify-fd-mount`，让内层按 `(dev, ino)`
复校后立刻关闭，避免可写宿主 fd 漏进沙箱（`linux-sandbox/src/fd_mount.rs L12-L87`、
`linux-sandbox/src/launcher.rs L59-L124`）。

### 7.5 macOS seatbelt

实现是 `sandbox-exec -p <profile> -DKEY=value... -- <cmd>`，且**只信任 `/usr/bin/sandbox-exec`
绝对路径**以防 PATH 注入（`sandboxing/src/seatbelt.rs L55-L59`）。

profile 以 `(deny default)` 起步、全程允许制（`sandboxing/src/seatbelt_base_policy.sbpl L8`）。
文件规则按「可读根 / 可写根」两组动态生成：全盘可写时给 `(allow file-write* (regex "^/"))`
并减掉不可读根，否则逐个 writable root 生成 `(subpath …)`，再用 `require-not` 精确挖掉只读
子路径与受保护元数据名（`.git` / `.codex` 一类），并追加
`deny file-write-unlink (vnode-type DIRECTORY)` 防「重命名祖先搬走受保护后代」
（`seatbelt.rs L507-L611`、`L920-L983`）。

两条细节很能说明细致程度：`deny system-fcntl (fcntl-command 80 110)` 挡
`F_MAKECOMPRESSED` / `F_TRANSFEREXTENTS`（借只读 fd 改内容）；
`deny mach-lookup (xpc-service-name-prefix "")`（L1070-L1088）。网络默认一条不放；开网时
只放 loopback + 代理端口 + DNS 53 与受控 Unix socket，**代理端口推不出来时 fail-closed**
（L316-L378）。

### 7.6 「沙箱拒绝」的识别：codex 也是启发式（本轮最重要的一条纠偏）

`sandboxing/src/denial.rs L5-L9` 的注释自陈：**没有完全确定的方式**判断命令是否因沙箱失败
（用户 zshrc 里的错误也可能让命令失败）。判据是两级：

1. **退出码快路**（`denial.rs L13-L42`）：`SandboxType::None` 或 `exit_code == 0` 直接 false；
   `[2, 126, 127]` 被**显式排除**（shell 内建误用 / 权限拒绝 / 命令未找到，都属非沙箱语义）；
   Unix 下只有 `exit_code == 128 + SIGSYS` **且** backend 是 `LinuxSeccomp` 才判 true。
2. **关键词兜底**（`denial.rs L45-L72`）：在 stderr / stdout / aggregated 三处小写匹配
   7 个词 —— `operation not permitted`、`permission denied`、`read-only file system`、
   `seccomp`、`sandbox`、`landlock`、`failed to write file`，命中即归因。

这解释了 §7.3 里 seccomp 统一返回 **EPERM 而非 SIGSYS** 的代价：EPERM 只能靠 errno 文本
被认出来，`128+SIGSYS` 那条快路实际多数时候走不到。

**结论：「拒绝可识别 → 升级重试」这条环在 codex 里是启发式成立的，不是内核级协议。**
对本项目的意义是双向的：

- **降本**：兑现 `OnFailure` 重试**不需要**先在沙箱侧引入结构化拒绝码，用「退出码 + 关键词表」
  即可达到 codex 同等水平；
- **接受同等误报面**：普通 `EPERM` / 权限错误会被误判为沙箱拒绝 —— codex 也接受这一点。

真正结构化的产物只有两条：`FileSystemSandboxViolation{backend, reason, path, output_snippet}`
（reason 六值：OperationNotPermitted / PermissionDenied / ReadOnlyFileSystem / PolicyDenied /
FailedToWriteFile / SignalSyscall）与 `NetworkSandboxViolation`（来自 managed proxy 的
`BlockedRequest`，带 host / protocol / decision / timestamp）（`sandboxing/src/violation.rs L46-L114`）。
**这两条是「已确认违规」的上报面，不是「是否违规」的判据面** —— 判据仍是启发式。

### 7.7 crate 边界：`sandboxing` 不是纯 DTO，`process-hardening` 管的是别的事

`codex-sandboxing` 是「策略 → 命令行变换 + 违规判定 + 进程启动」的中心层：持
`SandboxManager` / `SandboxType`、各平台 argv 变换（bwrap / seatbelt / windows）、
`denial.rs` / `violation.rs`、`spawn.rs`（`sandboxing/src/lib.rs L1-L54`）。
`core/src/sandboxing/mod.rs` 只是**适配层**：保留 exec 侧元数据（`ExecRequest` / `ExecOptions`），
把 `SandboxExecRequest` 转回 `ExecRequest`，并**注入 `CODEX_SANDBOX_NETWORK_DISABLED` /
`CODEX_SANDBOX=seatbelt` 等环境变量**（`core/src/sandboxing/mod.rs L1-L8`、`L117-L202`）。

最后这点值得单独标出：**沙箱档位不只写进 system prompt，还以环境变量形式注入子进程** ——
本项目目前两者都没有（见 §0.2）。

`process-hardening` 做的是**进程自身硬化，与隔离无关**：Linux `prctl(PR_SET_DUMPABLE, 0)` 禁
同用户 ptrace、`RLIMIT_CORE=0` 关 core dump、清 `LD_*`；macOS `ptrace(PD_DENY_ATTACH)`、
`RLIMIT_CORE=0`、清 `DYLD_*`（`process-hardening/src/lib.rs L44-L61`、`L83-L100`）。
它**不设 `NO_NEW_PRIVS`、不隐藏 `/proc`** —— `PR_SET_NO_NEW_PRIVS` 在
`linux-sandbox/src/landlock.rs L129-L136` 单独设置，且**只在需要 seccomp 或 legacy Landlock
时才设**（因为会让依赖 setuid 的 bwrap 部署失效，见 L66-L74）。这正是 codex 把 seccomp 拆到
bwrap 之后作「内层 stage」的原因：**先让 bwrap 完成 setuid 相关挂载，再收紧**。

---

## 8. Windows 沙箱与服务模型

### 8.1 令牌：不是 SAFER，是 capability SID + 受限令牌

非提权路径用 `CreateRestrictedToken`，标志固定为
`DISABLE_MAX_PRIVILEGE | LUA_TOKEN | WRITE_RESTRICTED`（`windows-sandbox-rs/src/token.rs L500`）
—— **不是 SAFER、不是 AppContainer**。restricting SID 不是凭空造，而是**自造 capability SID**
（落盘 `codex_home/cap_sid`），entries 顺序被注释锁定为「Capabilities… → ExtraRestricting… →
Logon → Everyone」（`token.rs L42-L112`、`L481-L498`）。

`WRITE_RESTRICTED` 是本模块的全局关键不变量：**限制 SID 只参与写访问检查**，
所以 capability SID 上的 deny-**read** ACE 对这种令牌**根本不生效**。codex 因此把
「读限制」硬性划归提权后端（`windows-sandbox-rs/src/lib.rs L716-L725`、
`sandboxing/src/windows.rs L96-L118`）。提权路径才真的**创建两个独立本地账户**
`CodexSandboxOffline` / `CodexSandboxOnline` + 本地组，随机密码经 DPAPI 落
`codex_home/.sandbox-secrets`（`windows-sandbox-rs/src/setup.rs L52-L72`、
`src/setup_provisioning/sandbox_users.rs L66-L110`）。

### 8.2 写保护方向：给工作区加 allow，不是给用户 Profile 加 deny

这点**容易误判**：`compute_allow_paths_for_permissions` 从 writable roots 生成 allow / deny
两个集合（`windows-sandbox-rs/src/allow.rs L14-L42`），而 deny 只用于**工作区内的只读子路径**
（`.git`）与 `cwd/.codex`、`cwd/.agents` 的 deny-write（`src/workspace_acl.rs L23-L30`）。

「用户自己目录不被写」靠的是**身份差异（沙箱令牌 ≠ 用户令牌）+ world-writable 审计**：
候选依次为 CWD 一级子项、TEMP/TMP、`USERPROFILE`、`PUBLIC`、PATH、`C:\`、`C:\Windows`；
限额 `MAX_ITEMS_PER_DIR=1000` / `AUDIT_TIME_LIMIT_SECS=2` / `MAX_CHECKED_LIMIT=50000`；
跳过 reparse point 与 `/windows/installer` 等噪声目录；命中即打 capability **deny ACE**
（`windows-sandbox-rs/src/audit.rs L28-L75`）。

敏感目录（`.ssh` / `.aws` / `.kube` 等，见 `USERPROFILE_ROOT_EXCLUSIONS`）走 **deny-read ACL**，
且**同时按词法路径与 canonical 目标打 ACE**，缺失路径先 `create_dir_all` 再打（防沙箱自己
造路径再读），任一失败回滚本次已加 ACE（`src/deny_read_acl.rs L18-L90`）。

对照本项目 `src-tauri/crates/tools/src/win_sandbox.rs:26-40` 自述「用户自己 Profile 的写入
不受保护」：codex 解决它的手段**不是 SAFER 那一档**，而是「真造一个不同身份的账户」或
「用 capability SID 做 deny ACE 审计」。这解释了为什么项目侧 SAFER 路线在「Profile 写保护」
上走到死胡同 —— 见 §6 的 P2 条目。

### 8.3 reparse point 防护

独立一层：用 `NtCreateFile` + `OBJ_DONT_REPARSE` 打开目录，命中 junction / symlink 即
`STATUS_REPARSE_POINT_ENCOUNTERED = 0xC000_050B` 失败
（`windows-sandbox-rs/src/no_reparse_dir.rs L1-L96`）。防的是「用链接把受限路径重定向到
任意目标」这一整类绕过。本项目 `win_sandbox.rs` 无对应机制。

### 8.4 网络：三层叠加，且不是 block-all

1. **WFP**：持久化 provider / sublayer / filter（固定 GUID + `FWPM_FILTER_FLAG_PERSISTENT`），
   条件恒为 `FWPM_CONDITION_ALE_USER_ID`（按沙箱账户 SID 匹配）+ 协议 / 远端端口，挂在
   `ALE_AUTH_CONNECT_V4/V6` 与 `ALE_RESOURCE_ASSIGNMENT_V4/V6` 层，实际只阻断 ICMP、
   DNS 53/853、SMB 139/445 共 12 条（`windows-sandbox-rs/src/wfp.rs L67-L100`、
   `src/wfp/filter_specs.rs L25-L124`）。
2. **Defender 防火墙 COM**（`INetFwPolicy2` / `INetFwRule3`）：对 offline 账户做
   **全量出 / 入站阻断 + loopback 例外**，规则名前缀 `codex_sandbox_offline_*`，
   loopback 串 `127.0.0.0/8,::/127`（`src/setup_provisioning/firewall.rs L33-L60`）。
3. **代理出口 allow-list**：真正的出网只允许 loopback 到专用代理端口。

结论：**default 出网是被 Defender 防火墙规则（而非 WFP）兜住的** —— 因为 WFP 侧主动省略了
NAME_RESOLUTION_CACHE filter。本项目 `win_sandbox.rs:38-40` 自述「网络封锁未做，阶段 2 通过
WFP 或代理出口补齐」，而 codex 的实际做法是「WFP + Defender 防火墙 + 代理白名单」**三件套**，
单靠 WFP 并不够。

### 8.5 启动与 elevated 通道

进程启动用 `CreateProcessAsUserW` + `STARTUPINFOEXW` / `EXTENDED_STARTUPINFO_PRESENT` +
`CREATE_UNICODE_ENVIRONMENT`，并持 `JobObject` 与私有 desktop
（`windows-sandbox-rs/src/process.rs L29-L49`）。本项目 `win_sandbox.rs` 同样用 JobObject +
私有 Window Station / Desktop（自述 conhost 在默认桌面会输出堆垃圾）—— **这一块项目侧已与
codex 同构**。

elevated 路径**不在客户端进程里直接提权**：父进程通过帧协议
`Message::{SpawnRequest, SpawnReady, Output, Stdin, Exit, Error, Terminate}` 与 runner 进程
通信，`SpawnRequest` 携带 cap SIDs、代理限制 SID、私有桌面名
（`src/elevated/ipc_framed.rs L39-L129`）；runner 以 `CREATE_NO_WINDOW | CREATE_SUSPENDED`
起 helper，把服务端已 pin 的目录句柄 `DuplicateHandle` 注入其句柄表，最后 `NtResumeProcess`，
并强调「未完全初始化的 helper 绝不能在父进程放弃目录保护后运行」
（`src/setup_launch.rs L25-L71`、`src/elevated_impl.rs L65-L129`）。取消走 pipe 发 `Terminate`
+ 50ms 轮询。

### 8.6 `windows-sandbox-service`：为免 UAC 而引入的常驻 SYSTEM 服务

是**常驻 SCM 服务**：`StartServiceCtrlDispatcherW` + `SERVICE_WIN32_OWN_PROCESS`，接受
STOP / SHUTDOWN / SESSIONCHANGE，事件日志 ID 从 1000 到 3004 分段
（`windows-sandbox-service/src/service.rs L69-L106`）。身份 **LocalSystem**（setup lock 的 SDDL
为 `D:P(A;;GA;;;SY)(A;;GA;;;BA)`，`windows-sandbox-rs/src/setup_mutex.rs L28-L60`）。

**明确回答「免 UAC 还是每次提权」**：客户端把 provisioning 请求发到具名管道，由 SYSTEM 服务
直接跑提权 setup helper；**仅当服务不可用**（未打包 / 无服务 / 策略解析失败 / 超时
`PROVISIONING_TIMEOUT = 120s`）才回落到客户端自己的 UAC helper，此时
`SetupErrorCode::OrchestratorHelperLaunchCanceled` 正是「用户取消了 UAC」
（`windows-sandbox-rs/src/provisioning_client.rs L101-L149`、`src/setup_error.rs L11-L118`）。
**代价 = UAC 弹窗从「每次执行都可能出现」变成「安装期一次」。**

**IPC 鉴权**是这套设计里最重的一环：管道用
`FILE_FLAG_FIRST_PIPE_INSTANCE | PIPE_REJECT_REMOTE_CLIENTS`，SDDL 为
`D:P(D;;GA;;;<sandbox_sid>)(A;;GA;;;SY)(A;;GA;;;BA)(A;;0x0012019b;;;IU)`
（IU 的 `0x0012019b` 是标准用户读 / 写 / 读属性的子集），单实例、请求上限 4096 字节、
空闲 5s 超时（`windows-sandbox-service/src/ipc.rs L42-L254`、`src/ipc/listener.rs L29-L62`）。

服务端**不只认 SDDL**，还做五重校验：取 setup lock 并比对本地组当前 SID 与本实例绑定 SID
（不一致返 `SANDBOX_GROUP_CHANGED`，防 SID 代际漂移导致 stale deny 误拒）；
`ImpersonateNamedPipeClient` + `OpenThreadToken` 后校验进程 token 用户 == 被冒充用户；
`process_package_family` 必须与自身 MSIX 包族一致；`TokenSessionId` 必须非 0；
`CheckTokenMembership` 拒绝沙箱组成员（`src/ipc/authentication.rs L40-L222`、
`src/package_identity.rs L45-L111`）。**这才是「防任意本地进程调用」的实际做法**：
即便能连上管道，非同一 MSIX 包身份的进程也拿不到 provisioning。

Provisioning 幂等靠三点：`SetupRuntime::Registered` 直接返回；
`setup_is_complete_with_settings` 为真直接返回；helper 必须定位到同目录的
`codex-windows-sandbox-setup.exe` 且**拒绝非普通文件或带 `FILE_ATTRIBUTE_REPARSE_POINT`**
（防被链接替换）（`windows-sandbox-service/src/provisioning.rs L62-L142`）。

回滚：卸载先取 setup lock → 禁用沙箱账户 → 捕获 retained logons（最多留 2 个 token，
用 `TokenStatistics.AuthenticationId` 防登录会话被回收）→ 停相关进程 → 删
`.sandbox` / `.sandbox-secrets` / `.sandbox-bin`，失败则恢复账户 flags
（`windows-sandbox-rs/src/uninstall_windows.rs L38-L120`、
`src/uninstall_windows/retained_logons.rs L19-L70`）。安装记录刻意存
`HKLM\SOFTWARE\OpenAI\Codex\WindowsSandboxService` 下的 `ProvisionedInstallation`（JSON），
注释说明放服务键**之外**是因为包更新会替换服务键（`windows-sandbox-rs/src/installation_record.rs`）。

另一处代价：企业 managed requirements 可以**否决**请求设置（服务内起单线程 tokio runtime，
`ImpersonateLoggedOnUser` 后加载 managed config layer stack 校验）
（`windows-sandbox-service/src/machine_policy.rs L20-L80`）。

### 8.7 `mxc-sandbox`：第三种后端，且默认关闭

不是上层抽象，而是与 `windows-sandbox-rs`（非提权受限令牌）、Linux 后端**并列的第三种平台
后端**，枚举里以 `SandboxType::WindowsMxc` 独立存在，违规上报侧也有
`SandboxViolationBackend::WindowsMxc`（`sandboxing/src/violation.rs L46-L66`）。

它自身**不构造令牌、不改 ACL、不开沙箱账户、不提权** —— README 明确写「never invokes MXC's
AppContainer dispatcher, edits host ACLs, creates sandbox users, runs setup, or requests
elevation」（`mxc-sandbox/README.md L5-L27`）。它只做**策略翻译**：把 `PermissionProfile`
映射成 Microsoft MXC/PSEC 的 `ContainerPolicy{readwrite_paths, readonly_paths, denied_paths,
network_egress/ingress, ui, base_process_ui}`，并显式设 `fallback.allow_dacl_mutation = false`，
即**永不退回 ACL 兜底**（`mxc-sandbox/src/policy.rs L72-L313`）。执行路径是把命令经当前 codex
exe 重入 `BaseContainerRunner::spawn(..., StdioMode::Inherit)`，参数经环境变量分块传递以规避
命令行长度限制（`mxc-sandbox/src/native.rs L19-L43`、`mxc-sandbox/src/lib.rs L24-L119`）。

默认关闭（`features.prefer_mxc` 默认 off）；一旦用户显式配 `windows.sandbox = "mxc"`，
语义是 **strict** —— 失败**不回落**到另两个后端；有 `denied_paths` 时必须先探测到该 Windows
构建支持 deny paths，否则直接报「this Windows build cannot enforce native MXC deny paths」
（`mxc-sandbox/src/native.rs L19-L43`）。

**为什么值得记**：它演示了「依赖平台原生容器运行时」这条路线 —— 代价是绑定 MXC/PSEC 可用性，
收益是不必自己维护令牌 / ACL 的一大摊。

### 8.8 与 Linux 侧的一致性：折扣与硬拒都是显式的

三档是**同一套 `PermissionProfile`**，Windows 映射为两种令牌模式
`WindowsSandboxTokenMode::{ReadOnlyCapability, WritableRootsCapability}`，因此 ReadOnly /
WorkspaceWrite 在语义上是**等价落地**而非另立体系
（`windows-sandbox-rs/src/resolved_permissions.rs L40-L61`）。

但折扣是显式声明的：`permission_profile_supports_windows_restricted_token_sandbox` 仅在
`!has_full_disk_write_access()` 时为真；split filesystem read restriction 在非提权后端不可强制，
代码直接报「windows unelevated restricted-token sandbox cannot enforce split filesystem read
restrictions directly; refusing to run unsandboxed」（`sandboxing/src/windows.rs L96-L118`）
—— 即**宁可拒绝运行也不降级为无沙箱**，与本项目「不静默降级」的取向一致。

平台差异兜底：MXC 侧因「MXC volume-root grants are nonrecursive」，显式把卷根一级子目录
快照成递归授权（`mxc-sandbox/src/policy.rs L72-L313`）；`/tmp` 与 `TMPDIR` 在 Windows 无对应
语义，被显式剔除后改用环境里的 `TEMP` / `TMP`（`windows-sandbox-rs/src/resolved_permissions.rs L40-L61`）。

---

## 9. guardian：预算参数与分类器契约

### 9.1 形态：不是函数，是一条审批闸门

工具调用在拿到 `ReviewDecision` 前不能执行，唯一入口是 `decide_approval`
（`core/src/guardian/decision.rs L45-L131`）。投递形态有三种：有外部取消令牌时走
`spawn_approval_decision` —— **专门开一个名为 `codex-approval-review` 的 OS 线程**，用
`runtime.block_on` 跑审查再经 `oneshot` 回传（`decision.rs L17-L42`）；网络审批走
`tokio::spawn`；其余直接 `await`。即：**阻塞审批路径，但把 LLM 往返从 tokio 工作线程上剥离**。

真正会落到 LLM 的条件是 `AskForApproval::OnRequest | Granular(_)` **且**
`ApprovalsReviewer::AutoReview`（`ext/guardian-reviewer/src/routing.rs L52-L60`）。返回 `None`
表示「交回既有用户审批流程」，**不是隐式放行**（`decision.rs L44`）。

### 9.2 分类器契约（最重要）

结构 `GuardianAssessment`：`risk_level` / `user_authorization` / `outcome` / `rationale`
（`ext/guardian-reviewer/src/assessment.rs L12-L18`）。JSON schema 设
`additionalProperties: false`，但 **`required` 只有 `["outcome"]`**（`assessment.rs L80-L103`）。

容错解析 `parse_guardian_assessment`（`assessment.rs L24-L65`）：先严格 `serde_json`；
失败则「取首个 `{` 到末个 `}`」再试一次（容忍模型把 JSON 包在散文里）；仍失败即 bail。
缺省字段有**确定性兜底**：risk 由 outcome 反推（Allow→Low、Deny→High），authorization→Unknown，
rationale→固定串。注意 payload 用 `Option` 承接，与 schema 的 `required` 刻意错开 —— 这是
**有意留出的容错面**，不是疏漏。

契约文本与 schema 放在同一文件相邻位置，注释要求二者同步演进
（`guardian_output_schema()` / `guardian_output_contract_prompt()`，`assessment.rs L75-L118`）。

### 9.3 失败矩阵：fail-closed

- `ext/guardian-reviewer/src/completion.rs L111-L143`：`StaleAuthorization` /
  `InputBudgetExceeded` / `PromptBuild` / `Session` / `Parse` 一律 `ReviewDecision::denied(...)`
  + status `Denied` + `FailedClosed`。
- 超时 → `ReviewDecision::TimedOut`（L85-L98）；取消 → `Abort`（L99-L110）。
- **唯一降级成「问用户」的路径**：`InputBudgetExceeded && !require_guardian` → `decision: None`
  （L70-L83）。
- 模型「拒答」（返回空载荷）被当作 Session 错误 → 同样 fail-closed（`outcome.rs L101-L105`）。
- 重试：`MAX_REVIEW_ATTEMPTS = 3`（`ext/guardian-reviewer/src/lib.rs L40`），只重试 Parse、
  StaleAuthorization 以及可恢复的 Session（限流 / 过载 / 5xx / 408 / 429）
  （`retry.rs L94-L137`），退避 `200ms × 2^(n-1) × jitter(0.9–1.1)` 且封顶在 deadline
  （`retry.rs L74-L79`）；总 deadline `REVIEW_TIMEOUT = 90s`（`lib.rs L41`）。

### 9.4 预算参数表

| 参数 | 值 | 约束对象 | 定位 |
|---|---|---|---|
| `MAX_REVIEW_ATTEMPTS` | 3 | 单次审查最大尝试 | `ext/guardian-reviewer/src/lib.rs L40` |
| `REVIEW_TIMEOUT` | 90s | 整个审查操作 deadline | `ext/guardian-reviewer/src/lib.rs L41` |
| 退避 | `200ms × 2^(n-1)` × jitter 0.9–1.1 | 重试间隔 | `ext/guardian-reviewer/src/retry.rs L74-L76` |
| `INPUT_TOKEN_MARGIN` | 256 | 输入上限预留余量 | `core/src/guardian/request_budget.rs L18` |
| `DEFAULT_MAX_INPUT_TOKENS` | 128_000 | 目录无权威窗口时的保守输入上限 | `guardian-context/src/budget.rs L56` |
| `IMAGE_TOKEN_RESERVATION` | 10_000 | 单图 token 预留 | `guardian-context/src/budget.rs L54` |
| entry_limits | message 5_000 / tool 1_000 / node_repl_output 1_000 | 单条转写条目上限 | `guardian-context/src/profile.rs L65-L69` |
| retention | message 20_000 / tool 10_000 / max_recent_non_user_entries 40 | 转写聚合上限 | `guardian-context/src/profile.rs L71-L75` |
| `MIN_RECENT_TOOL_ENTRIES` | 5 | 无论多紧都保留的最近工具证据条数 | `guardian-context/src/profile.rs L25` |
| `GUARDIAN_MAX_TOOL_ENTRY_TOKENS` | 1_000 | 同步档工具条目上限 | `core/src/guardian/mod.rs L70-L73` |
| `GUARDIAN_MAX_ROOT_MESSAGE_TOKENS` | 900 | 根授权消息截断 | `core/src/guardian/mod.rs L74` |
| `GUARDIAN_MAX_NODE_REPL_TOOL_RESULT_TOKENS` | 6_000 | Node REPL 证据上限 | `core/src/guardian/mod.rs L75` |
| `GUARDIAN_MAX_APPROVAL_REASON_TOKENS` | 512 | 审批理由截断 | `core/src/guardian/prompt.rs L41` |
| `MAX_GUARDIAN_ENVIRONMENT_ID_BYTES` | 256 | 环境 ID 上限（超限直接报错） | `core/src/guardian/prompt.rs L43` |
| `GUARDIAN_MAX_IMAGE_ITEM_TOKENS` | 10_000 | 是否接纳图片的条件之一 | `core/src/guardian/review_session.rs L102` |
| `MAX_PREVIOUS_REVIEWS` | 8；片段 1_000 / body 800 / 关联 100 / action 350 / rationale 250 | 历史审查记录注入上限 | `guardian-context/src/reviews.rs L20, L104-L107` |
| `MAX_ITEMS_PER_KIND` / `MAX_BYTES_PER_KIND` | 128 / 4 MiB | 历史按类别的条数与字节上限 | `guardian-context/src/history.rs L21, L24` |
| `MAX_TRANSCRIPT_IMAGES` | 4 张 / 8 MiB | 转写图片上限 | `guardian-context/src/images.rs L18-L19` |
| `MAX_RENDERED_BYTES` | 32_000 | Node REPL 渲染上限 | `guardian-context/src/node_repl.rs L19` |
| 证据段上限 | instruction 900 / answer 900 / trusted_skill 768 / trusted_tool 512 | 各证据段 | `guardian-context/src/retained_instructions.rs L20` 等 |
| `MAX_ASYNC_PERMISSION_BYTES` | 3_000 | 异步权限证据上限，超限回退同步审查 | `guardian-context/src/permissions.rs L70, L90-L95` |
| 熔断阈值 | 连续 3 / 近 10（窗口 50）；Cyber 模型 1 / 1 | 拒绝熔断 | `ext/guardian-reviewer/src/circuit_breaker.rs L3-L7` |
| `GUARDIAN_PLUGIN_ATTRIBUTION_TIMEOUT` | 5s | 插件归因超时 | `core/src/guardian/review.rs L48` |
| `MAX_RECORD_BYTES` | 4 MiB | 失败审查反馈记录上限 | `ext/guardian-reviewer/src/feedback.rs L14` |

**关键判读**：这些预算约束的是**「喂给审查模型的证据量」，不是审查模型自己的输出量**。
输入侧有两道闸：`check_pending`（开工前可行性预检，`core/src/guardian/input_budget.rs L32-L66`）
与 `finalize`（组装完成后按真实开销选证据，L68-L194），末端还有 `check_prompt`
（`core/src/guardian/request_budget.rs L59-L90`）。**没有独立的「会话总 token 预算」** ——
审查会话的总量边界就是审查者模型自己的 context window（`effective_input_token_limit`，
`guardian-context/src/budget.rs L60-L78`），超限先 compaction，再超限才判 `InputBudgetExceeded`。

### 9.5 输入构造与模型选择

审查模型看到的是：**被压缩的转写**（用户消息与人工审批条目全保留，其余按 retention 上限
回填最近条目）+ **待审动作的精确 JSON**（`format_guardian_action_pretty`）+ **权限档证据**
（denied paths / globs）+ 可信用户输入 + 根授权消息 + 可选 Node REPL 证据
（`core/src/guardian/prompt.rs L82-L221`）。

它**用独立 system prompt**：`base_instructions` 被整体替换为
`GuardianPolicyInstructions::new(租户策略, 额外策略, 模板, 输出契约)`，provenance 标 `Custom`
（`core/src/guardian/reviewer_config.rs L39-L48`）。模型选择**偏小偏省**：优先 provider 声明的
`approval_review_preferred_model()`，命中即用该模型**并强制 Low reasoning effort**；
未命中则回退父模型、同样优先 Low（`ext/guardian-reviewer/src/model.rs L18-L75`）。

审查者自身被钉在**只读沙箱 + `approval_policy = Never`**，工具白名单只有
`exec_command / write_stdin / view_image / exec / wait`
（`ext/guardian-reviewer/src/settings.rs L31-L52, L82`）。这是一个漂亮的递归自洽：
**审查者自己不能再申请审批**。

### 9.6 coverage 与缓存

动作归类为 `GuardianScope{ComputerUse, Shell, FileChanges, Mcp, Network, Permissions}`
（`protocol/src/openai_models/guardian.rs L116-L123`；映射 `core/src/guardian/coverage.rs L8-L18`）。
必须过审查的前提即 §9.1 的 policy + reviewer 组合。**可跳过**有两条明确判据：
① `full_access` → 直接 `Approved`，不调 LLM（`routing.rs L79-L84`）；
② 扩展的异步评分已给出 `Allow` 且 `!require_fresh_review` → `cached_approval`
（`routing.rs L113-L115, L159-L186`）。

而 `require_fresh_review` 在「同步审查要求 / 模型要求审查但异步未开 / 已取消 / 是重试 /
是提权 exec」时被**强制置真**（`routing.rs L102-L106`）⇒ **缓存只对「首次、未提权、非重试」
的调用有效**。

### 9.7 会话生命周期与成本防护

审查会话**复用而非一次一建**：`ReviewerPool` 持有一个 trunk 会话；trunk 空闲时新审批
**追加到同一 Guardian 对话**以保持 prompt-cache 稳定，trunk 忙则从最后一次已提交的 rollout
做**临时 fork**，避免并行审批互相阻塞（`core/src/guardian/review.rs L175-L188`）。是否复用由
`GuardianReviewSessionReuseKey`（模型 / 权限 / 指令 / cwd / environment）判定，变了就重建
（`core/src/guardian/review_session.rs L176-L271`）。

结论回流为一条 `ReviewDecision`：放行 `Approved`；拒绝
`denied(render_guardian_rejection(rationale, rejection_instructions))`
（`completion.rs L173-L183`）—— 即**拒绝是「带指令的拒绝」而不是裸串**，理由同时展示给用户与
主模型，并经 `GuardianAssessmentEvent` / `GuardianWarning` 上报（`review.rs L114-L128`）。

成本三防护：单操作层 `MAX_REVIEW_ATTEMPTS=3` + `REVIEW_TIMEOUT=90s` + 输入预算；
会话层 `prompt_cache_key = "guardian:{parent_thread_id}"` 复用前缀缓存
（`review_session.rs L273-L285`）；拒绝熔断（`reporting.rs L146-L164`、
`circuit_breaker.rs L53-L84`）—— 注意**熔断限的是「拒绝太多导致 turn 空转」，不是审查调用次数**。

**没有「同一命令已批准就不再问」的按命令记忆**：`decision.rs L1-L2` 明确「no outcome is
stored by tool-call ID」。免重问靠的是异步评分缓存（§9.6），不是命令级记忆。这与 §10.5 的
execpolicy amendment 是**两套不同机制**：execpolicy 沉淀的是**命令行前缀规则**，
guardian 缓存的是**动作等价性**。

---

## 10. execpolicy：规则表示与匹配算法

### 10.1 规则表示：Starlark 方言

规则文件是 `.rules` 文本（**Starlark 方言**，非 JSON、非自定义行式语法）。加载器只扫描每个
配置层目录下的 `<config_folder>/rules/`，**扩展名必须恰为 `rules`**
（`core/src/exec_policy.rs L54-L56, L1121-L1171`），多层按低→高优先级依次 parse 并叠加
（`core/src/exec_policy.rs L662-L716`）。

解析是 `AstModule::parse`（`Dialect::Extended` + 开 `enable_f_strings`）后 `eval_module`
—— 规则**不是静态扫描出来的，而是在求值期由注入的 builtins 副作用写入 builder**
（`execpolicy/src/parser.rs L57-L79`）。三个 builtin：`prefix_rule` / `network_rule` /
`host_executable`（`parser.rs L347-L472`）。

真实样例（`execpolicy/examples/example.codexpolicy`）：

```starlark
prefix_rule(
    pattern = ["git", "reset", "--hard"],
    decision = "forbidden",
    justification = "destructive operation",
    match     = [["git", "reset", "--hard"]],
    not_match = [["git", "reset", "--keep"], "git reset --merge"],
)
```

语法要点：`pattern` 是**有序 token 列表**，首元素是索引键（程序名）且可为列表表示别名，
**别名只在首 token 展开成多条规则**；其余位置写 `["a","b"]` 表示「该位多选一」→
`PatternToken::Alts`（`execpolicy/src/rule.rs L16-L35`、`parser.rs L184-L217`）。
`decision` 默认 `"allow"`（`parser.rs L357-L360`）；`justification` 可选、可为任意 decision
服务、但**禁止全空白**（`parser.rs L362-L368`）。

**不支持正则、不支持通配**：网络规则显式拒绝 `*`（`rule.rs L200-L204`），命令侧只有
「逐 token 全等 + 别名」。`match` / `not_match` 是**加载期自测**（相当于把单测写进规则文件），
字符串形式用 `shlex::split` 切词（`parser.rs L297-L309`、`rule.rs L246-L305`）。索引结构为
`rules_by_program: MultiMap<String, RuleRef>`（`rule.rs L110-L115`）。

### 10.2 命令解析：tree-sitter-bash，但只在特定包装下启动

核心是 **tree-sitter-bash**（`shell-command/src/bash.rs L13-L20`），但**只在顶层是
`bash|zsh|sh -c/-lc <script>` 包装时才启动**：`extract_bash_command` 要求该三元组且 flag ∈
{`-c`, `-lc`}（`bash.rs L106-L119`），入口在 `commands_for_exec_policy_for_platform`
（`core/src/exec_policy.rs L876-L904`）。

进入后 `try_parse_word_only_commands_sequence` 用**白名单拒绝式**枚举 AST：命名节点只允许
program / list / pipeline / command / command_name / word / string / string_content /
raw_string / number / concatenation，标点只允许 `&& || ; | " '`（`bash.rs L36-L80`）。
因此各构造的处理是：

- `|`、`&&`、`;` → **拆成多个子命令逐段判定**（`bash.rs L331-L341` 有明文测试断言）；
- 重定向 `>`、后台 `&`、子 shell `( )`、命令替换 `$( )` / 反引号、变量展开 `$HOME`、
  heredoc、环境变量前缀 `FOO=bar cmd` → **一律拒绝整棵解析**
  （`bash.rs L404-L421, L491-L499`），此时**不做拆分，整条 argv 原样当一条命令**
  （`core/src/exec_policy.rs L900-L903` 的兜底 `vec![command.to_vec()]`）。

**这条语义很关键**：拒绝解析 ≠ 放行 ≠ 逐段猜，而是「**退化为保守的单命令匹配**」——
整条串作为一个 token 序列去比 prefix 规则，几乎必然不命中，从而落到 §10.3 的默认裁决。
对写规则的人是安全方向。

包装器只解 `bash -c` 这一层；`env` / `sudo` / `time` **在已读路径中未见解包逻辑**
（`sudo`、`env` 只出现在「禁止沉淀为规则」的黑名单里，`core/src/exec_policy.rs L57-L146`），
会以包装器为首 token 参与匹配。Windows 另有 PowerShell 分支
（`core/src/exec_policy.rs L889-L898`）。

### 10.3 匹配算法与决策优先级

匹配是**左前缀逐 token 全等**：`PrefixPattern::matches_prefix` 先要求 `cmd[0] == first`
且 `cmd.len() >= pattern_len`，再对 `rest` 逐位 `matches`，任一位不等即整体不命中
（`execpolicy/src/rule.rs L46-L59`）。索引也按首 token 精确取桶
`rules_by_program.get_vec(cmd[0])`（`execpolicy/src/policy.rs L334-L342`）⇒ **首 token 写错
一位就完全不命中**；参数不匹配的后果是「该规则不命中」而非「降级命中」。

判定顺序（**全程无任何数值优先级字段**）：

1. 用 `cmd[0]` 精确取桶，逐条 `rule.matches`（`policy.rs L334-L342`）；
2. 精确命中**非空** → 结果定案，**不再**做可执行文件回退（`or_else` 只在空时触发，
   `policy.rs L311-L320`）；
3. 精确命中为空且 `resolve_host_executables = true` → basename 回退（受 `host_executable`
   白名单约束，见 §10.4）；
4. 仍为空 → 调外部 `heuristics_fallback` 生成一条 `HeuristicsRuleMatch`（`policy.rs L322-L331`）；
5. 多条命中时，`Evaluation::from_matches` 取全部命中的 `Decision` 的 **`max()`**；而
   `Decision` 的 `Ord` 由枚举声明顺序决定 = `Allow < Prompt < Forbidden`
   （`policy.rs L402-L411`、`execpolicy/src/decision.rs L7-L16`）⇒ **最严者胜**，既不是
   「首个命中」也不是「最具体优先」。prefix 长度只用于**文案**（取最长命中前缀渲染理由，
   `core/src/exec_policy.rs L1058-L1096`）。

**未命中时 execpolicy 自己不表态**：无 fallback 时返回空 `matchedRules`、不输出 decision
（`execpolicy/src/execpolicycheck.rs L45-L51`）。真正的默认裁决在调用侧
`render_decision_for_unmatched_command_for_platform`（`core/src/exec_policy.rs L770-L855`）：
命中危险命令特征（或 Windows 沙箱未启用却有托管 FS 限制）→ `Prompt`，`AskForApproval::Never`
时升为 `Forbidden`；否则 `Never`→`Allow`、`UnlessTrusted`→`Prompt`、`OnRequest`/`Granular`
看沙箱是否受限（受限且未申请越权则 `Allow`，申请越权则 `Prompt`）。⇒ **default 不是简单
allow，而是「沙箱形态 + 审批策略」的函数**。

### 10.4 可执行文件身份解析

`resolve_host_executables: true` **不做 PATH 查找、不做 shebang 解析**。实现
`match_host_executable_rules`（`policy.rs L344-L371`）：先要求该 token 本身已是绝对路径
（`AbsolutePathBuf::try_from`，失败即放弃回退 ⇒ **相对路径 `./git` 拿不到 basename 回退**），
取 basename 作查找键（Windows 会剥 `.exe/.cmd/.bat/.com` 并小写化，
`execpolicy/src/executable_name.rs L4-L29`），把首 token 换成 basename 后套用同名规则，
并在结果里附 `resolved_program`。

防绕过三道：① 绝对路径的精确规则先匹配且**优先**，basename 回退不能覆盖它
（`policy.rs L311-L320`）；② 若策略里声明了 `host_executable(name="git", paths=[...])`，
则白名单以外的绝对路径**禁止**借 basename 规则，`paths = []` 表示彻底禁止回退
（`policy.rs L354-L361`）；③ 调用侧对「不认识的程序」只把可执行文件自身作为一条命令评估，
**内层参数不能反过来给可执行文件背书**（`core/src/exec_policy/executable_identity.rs L43-L53`）。

符号链接归一化**未核实**（execpolicy 内未见实现）。

### 10.5 自我修订（amendment）

产物是 `ExecPolicyAmendment{command: Vec<String>}`，**只表示「允许以此 prefix 开头」**
（`protocol/src/approvals.rs L34-L54`）。生成语义在
`derive_requested_execpolicy_amendment_from_prefix_rule`（`core/src/exec_policy.rs L958-L997`）：

1. 入参 `prefix_rule` 由模型在工具调用里给出 ⇒ 空则丢弃；
2. 与 `BANNED_PREFIX_SUGGESTIONS`（`bash -c`、`python`、`rm`、`sudo`、`env` 等约 90 条，
   `core/src/exec_policy.rs L57-L146`）逐条全等比对 ⇒ 命中则丢弃；
3. **若已有任何 policy 规则命中则丢弃**（避免与新规则冲突）；
4. 最后做**模拟写入校验**：把该规则加进策略副本后逐段复算，**必须使本次所有子命令段都
   判定为 `Allow`**，否则不给建议（`core/src/exec_policy.rs L999-L1020`）。

该建议只在 `Decision::Prompt` 分支挂到审批请求上（`core/src/exec_policy.rs L426-L437`）；
当模型不遵守 prefix 规则时整体禁用（`AllowPrefixRules::IgnoreForCyberModel` 会从生效策略里
剔除全部 Allow 前缀规则，`core/src/exec_policy/model_policy.rs L27-L55`）。

**用户可见**：TUI 把它渲染成明确选项「Yes, and don't ask again for commands that start with
`X`」（`tui/src/bottom_pane/approval_overlay.rs L850-L869`），多行前缀直接不展示该选项。

批准后落盘：路径固定为 `$CODEX_HOME/rules/default.rules`（`core/src/exec_policy.rs L867-L869`），
`spawn_blocking` 调 `blocking_append_allow_prefix_rule`，token 经 `serde_json` 序列化后拼成
`prefix_rule(pattern=[...], decision="allow")` 单行，**加锁 + 全文去重**（完全相同行已存在
则跳过）（`execpolicy/src/amend.rs L65-L81, L147-L193`），随后同步更新内存策略并经 `ArcSwap`
发布（`core/src/exec_policy.rs L464-L512`）。另有一次性迁移会**删除**历史遗留的危险 allow 规则
（`execpolicy/src/sandbox_migration.rs L10-L29`）。

### 10.6 测试锁住的边界语义

| 用例 | 锁住的语义 |
|---|---|
| `basic_match`（`execpolicy/tests/basic.rs L142-L167`） | 省略 `decision` 即 `allow` —— 锁住 DSL 默认值本身（**默认放行**，规则文件写错 = 放行） |
| `strictest_decision_wins_across_matches`（L556-L594） | `git`(prompt) 与 `git commit`(forbidden) 同时命中 → forbidden，锁住「多命中按最严合成」 |
| `match_and_not_match_examples_are_enforced`（L510-L554） | `git --config color.status=always status` 必须**不**命中 `["git","status"]` —— 锁住「从首位起连续比对，非子序列」，防插一个 flag 绕过规则 |
| `only_first_token_alias_expands…`（L375-L444）/ `tail_aliases_are_not_cartesian_expanded`（L446-L508） | 别名只在首 token 展开，尾部别名是「单位多选一」—— 锁住首 token 索引模型，防规则数笛卡尔积爆炸 |
| `heuristics_match_is_returned_when_no_policy_matches`（L647-L663） | 未命中返回 `HeuristicsRuleMatch` 而非空、更非默认 allow —— 锁住「默认裁决权归调用方」的接口契约 |
| `host_executable_resolution_*`（L830-L859, L861-L894, L928-L962） | 空 allowlist 禁 basename 回退、白名单外绝对路径不得借 basename 规则、绝对路径精确规则优先 —— 锁住「同名不同文件不能互相背书」 |
| `append_allow_prefix_rule_dedupes_existing_rule`（L85-L101） | 同一 prefix 重复批准只落一行 —— 锁住 amendment 幂等性 |
| `justification_cannot_be_empty`（L230-L247） | 空白 justification 视为解析错误 —— 锁住「理由要么真实要么不写」 |

---

## 11. 剩余未核实项（边界声明）

原 §7 列的 4 项已在本轮全部核实并展开为 §7（沙箱系统调用）/ §8（Windows 与服务模型）/
§9（guardian 预算与契约）/ §10（execpolicy 匹配算法）。剩余未核实：

- **两侧三档是否逐条等价**：只能从 Windows 侧的显式声明反推（§8.8），未做 Linux / Windows
  交叉比对。
- **`windows-sandbox-service` 的安装落点**：MSIX manifest 里的服务注册方式与 `CreateServiceW`
  调用点在仓库内未核实（只有 crate 名与 `Cargo.lock` 佐证）。
- **guardian `pool.rs` 的并发度 / 池大小常量**、`execution.rs` 的事件等待细节、
  `review_session.rs` fork 快照的回滚时机未逐行读完。
- **`execpolicy` 的符号链接归一化**未见实现，`host_executable` 是否间接处理未核实。
- **`mxc-sandbox/src/windows.rs`**（平台读根枚举）未逐行读。
- 以下文件正文未读：`windows-sandbox-rs/src/spawn_prep.rs`、`windows-sandbox-service/src/ipc/home.rs`、
  `windows-sandbox-rs/src/uninstall_windows/{principals,processes,firewall}.rs`、
  `windows-sandbox-rs/src/setup_provisioning.rs` 主流程全文、`machine_policy.rs` L80 之后。

**路径纠错**：本文早期版本的 §7 写过 `codex-rs/crates/mxc-sandbox/` —— 错误。codex 的各 crate
**直接位于 `codex-rs/` 下，没有 `crates/` 中间层**，正确写法是 `mxc-sandbox/`（相对 `codex-rs` 根）。

本文所有「已核实」结论可回溯到对应 `路径 Lx-Ly`；凡标「推测 / 未核实 / 未逐行复核」处
不得作为改造依据。