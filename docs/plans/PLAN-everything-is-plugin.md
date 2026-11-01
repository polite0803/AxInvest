# PLAN · 一切皆插件 + 动态插件

> 参照实现：[`dsh-rs`](https://github.com/redoop/dsh-rs)（cordis 内核的 Rust 移植），其
> `crates/dsh-plugin-contract` 已逐字核对（见 §5.1）。
>
> 本仓的「一切皆插件」基座**已存在**（`capability_registry.rs` / `reversible_effect.rs` /
> `typed_event.rs`），本文档不做架构重写，只补三处缺口并修掉两个**已成立**的功能级缺陷。

---

## 1. 目标与达成判据

| 目标 | 判据 |
|---|---|
| **A · 一切皆插件**（组合模型） | 全部可替换能力经同一个 `CapabilityRegistry` 注册；**无任何能力靠硬编码构造函数旁路**；内置与外部实现走同一注册入口 |
| **B · 动态插件**（载入模型） | 插件可**脱离宿主仓库独立编译**、**运行期载入**（默认子进程 worker，见 §1.1），载入后其提供的接缝能**被现有消费方直接取到并使用**，卸载后可逆回滚 |

关键判据：**A 与 B 不是二选一**。B 层插件在注册表里必须与 A 层内置实现**是同一个类型**，
平权来自 provider 的构造方式，而非来自两套存储。

### 1.1 B 层载入主形态（本文档口径，2026-09-24 修正）

**主路径 = 独立二进制 + 长驻 worker 子进程 + 长度前缀 JSON 帧**（§5.0）。
**cdylib + dlopen 降为备选**（仅用于低频热接缝，参照契约见 §5.1）。

> ⚠ **口径说明**：本文档初稿（2026-09-23）按 cdylib 路径撰写；经 §11 实测
> 与 `crates/plugins/src/sandbox.rs` 逐行核对后，**主备已倒置**。
> §5 / §7 / §8 / §9.2 已按新口径重写，与本节一致。
> 结构性结论不受影响：**§4.0 门面 / §4.1 单表 / §4.2 / §4.3 / §3 缺陷 / §10 UI 全部保留** ——
> 因为 §4.0 的 `SeamInvoker` 只约定「JSON 进 JSON 出」，**不约定传输方式**。
> 插件间通信（§15）同样不受主备倒置影响 —— 两种 transport 下机制完全一致。

---

## 2. 现状（已核对事实）

### 2.1 已具备

| 能力 | 位置 | 说明 |
|---|---|---|
| 服务注册表（`provide`） | `src-tauri/crates/harness/src/capability_registry.rs:29` | `ServiceDefinition` 已声明 11 条接缝 |
| 可逆副作用（effect） | `src-tauri/crates/harness/src/reversible_effect.rs:83` | `EffectHandle::undo` / `EffectScope::rollback_all`（LIFO 逆序） |
| Waterfall 拦截 | `src-tauri/crates/harness/src/typed_event.rs:36` | `DispatchMode::{Emit, Waterfall, Parallel, Serial}`，可改写 payload 或拒绝中断 |
| 平权语义（语义层） | `capability_registry.rs:333` | `prepare_registration` 对 `ExternalPlugin` 先撤销同键内置 |
| 声明式外部插件 | `src-tauri/crates/plugins/src/types.rs:176` | `PluginCapabilityDecl`（`seam` 为**自由字符串**，无校验） |
| 插件沙箱（**准入检查，非隔离**） | `src-tauri/crates/plugins/src/sandbox.rs` | 4 项：路径白/黑名单、ENV 白名单、子进程权限、网络（仅告警）。**唯一在 OS 层强制生效的是 ENV**（`apply_env_to_command` 的 `env_clear` + 白名单回填）⇒ 详见 §3.3 |

### 2.2 已声明的 11 条接缝与其契约形态（逐条核对签名）

| 接缝 ID | trait | 签名形态 | 定义位置 |
|---|---|---|---|
| `agent.loop` | `AgentTurnRunner` | `run_turn(req) -> Result<res>` | `harness/src/agent_turn_runner.rs` |
| `workflow.sandbox` | `WorkflowSandbox` | `execute(genome, input) -> Result<res, String>` | `harness/src/workflow_*` |
| `workflow.reflector` | `WorkflowReflector` | `&self, 数据 → Result<数据, String>` | `workflow_reflection.rs:190` |
| `workflow.evolver` | `WorkflowEvolver` | `&self, 数据 → Result<数据, String>` | `workflow_evolution.rs:172` |
| `workflow.optimizer` | `WorkflowOptimizer` | `&self, 数据 → Result<数据, String>` | `workflow_optimization.rs:80` |
| `workflow.business_rule` | `BusinessRuleEvaluator` | `evaluate(node_type, input) -> 裁决` | `business_rules.rs:71` |
| `message.callback` | `PlatformMessageCallback` | `on_message(...) -> Option<String>` | `harness/src/` |
| `webhook.dispatch` | `WebhookDispatch` | `dispatch(event, data) -> DispatchResult` | `harness/src/` |
| `event.dispatch` | `EventDispatchBus` | **本身是总线**（非叶子能力）——即 §15 的 **L1 插件间通信通道** | `typed_event.rs` |
| `session.log.invariant` | `SessionLogInvariant` | `record` / `assert_replayable` | `harness/src/` |
| `platform.adapter.*` | `MessagePlatformAdapter` | `start/stop/send_message`（async 生命周期） | `harness/src/` |

**结论：除 `event.dispatch`（总线）与 `platform.adapter`（async 生命周期）外，
全部是 `数据进 → 数据出` 形态。**

> 注：这两条的**签名形态**与其余不同，但在子进程主路径下**仍可 B 层化**（§5.4 改判）——
> `platform.adapter` 由长驻 worker 承载生命周期，`event.dispatch` 由插件**订阅**（§5.2 的 `subscribe`）。

### 2.3 缺口

| # | 缺口 | 证据 |
|---|---|---|
| **G1** | provider 槽是 `Arc<dyn Any + Send + Sync>` —— **Rust trait object**，按定义不可跨 C-ABI | `capability_registry.rs:253` |
| **G2** | `register_external_*` 共 11 个方法，**生产代码零调用**（仅自身测试） | 全仓 grep 仅命中 `capability_registry.rs` 定义与测试 |
| **G3** | 无动态载入设施：无 `libloading` / `abi_stable` 依赖，无 C-ABI 契约 crate | `src-tauri/**/Cargo.toml` 零命中 |
| **G4** | 接缝未覆盖「一切」：`ToolRegistry` 内置工具、`ProviderRegistry::create_default()`、会话存储、系统提示词组装均硬编码 | `crates/tools/src/registry.rs:133`、`crates/providers/src/registry.rs:22`、`src/init/state.rs:737` |
| **G5** | 装配靠手工顺序：`ServiceDefinition` 无依赖字段，消费方各自 `Option` 判空 ⇒ **注册漏一条只静默拿 `None`** | `src-tauri/src/init/state.rs:737-788` |

---

## 3. 两个已成立的功能级缺陷（根因同一）

### 缺陷 1 · 外部插件的「能力实现」写入即消失（write-only）

`register_plugin_capability`（`capability_registry.rs:376`）最终走**通用** `register`
（`:350`），只写 `inner` 一张表；而所有消费方读的是**特化存储**
（`get_sandbox` 读 `sandboxes` `:539`、`get_platform_adapter` 读 `platform_adapters` `:1011`）。

⇒ 插件声明 `platform.adapter.telegram` 后：`contains()` 为 `true`、
`list_platform_adapters()` **不含 telegram**、`get_platform_adapter("telegram")` 为 `None`。

### 缺陷 2 · 声明与内置接缝同名会静默打死内置实现，且不可恢复

`prepare_registration`（`:333`）对 `ExternalPlugin` 调 `evict_builtin_for_external`（`:325`），
而内置句柄的回滚闭包（`register_rollback` `:1033`）**同时清 `inner` 与特化槽**。
`PluginCapabilityDecl.seam` 是**自由字符串**（`types.rs:176`），无校验。故插件写
`"seam": "workflow.sandbox"` 时：

1. 内置 sandbox 从两张表被撤销
2. 插件只往 `inner` 写一份 descriptor
3. `contains("workflow.sandbox") == true` 但 `get_sandbox() == None` —— **接缝变死**
4. 禁用插件时 `unregister_plugin_capabilities`（`plugins/src/manager.rs:561`）只 undo 外部句柄，
   **内置永不复活**
5. `list_with_details()` 仍报「该接缝由插件 X 提供」⇒ **UI 显示有提供者，运行时该接缝已死**

**共同根因**：`CapabilityOrigin` 参与**存储路由**（决定写哪张表 / 是否 evict）。
这正是「用两套存储模拟平权」的后果。

### 3.3 沙箱的实际强度（2026-09-24 逐行核对 `crates/plugins/src/sandbox.rs`）

**核对结论：`sandbox.rs` 不是隔离机制，是一组「宿主自觉调用的准入检查函数」。**
文件头自述即写明「在 hook/tool/lifecycle 子进程执行前做 capability 检查」——
即**检查在前，插件代码在后**，不存在任何 OS 级强制边界。

逐项核对：

| 项 | 函数 | 实际强度 |
|---|---|---|
| ENV 白名单 | `apply_env_to_command`（L165） | ✅ **唯一真在边界生效**：`command.env_clear()` + 按白名单回填，作用在 `Command` 上 |
| 路径白/黑名单 | `check_path_permission`（L179） | ⚠ **调用方自觉**：纯比较函数，不 hook `open()`。宿主不调即不生效 |
| 子进程权限 | `check_subprocess_permission`（L214） | ⚠ **同上**：仅决定「宿主是否愿意 spawn」，与插件内部行为无关 |
| 网络权限 | `note_network_access`（L228） | ❌ **明确自述无法拦截**，仅 `tracing::warn!`（原注释：需要 seccomp / 网络命名空间） |

**另有两条既存缺陷（与本次决策无关，但影响安全口径）**：

1. **`merged_with`（L283）是并集语义**：`allow_subprocess = self \|\| other`，
   且 `denied_paths` **取交集（更宽松）**。⇒ 多插件聚合后（`HookRunner::from_registry` 路径）
   任一插件声明 `subprocess_execution` 即**对全体放开**，沙箱被稀释到最弱插件水平。
2. **`SandboxConfig::default()` = `permissive()`（L75）**：`allow_subprocess: true`。
   ⇒ 凡未显式接入 manifest 的调用路径**默认放开子进程**，只有 ENV 与敏感路径黑名单还在。

**对 §8 的影响**：初稿称「沙箱建立在子进程边界上，故 dlopen 使其失效」——**该表述不准确**。
准确的表述见 §8（重写后）：**两种形态下，路径/子进程/网络检查的强度完全相同**
（都是宿主自觉检查，都可被绕过），**差别只在 ENV 白名单这一项**。

---

## 4. 设计方案

### 4.0 核心支点：让门面实现原 trait，消费方零改动

每条接缝的 `get_*()` **签名保持不变**（仍返回 `Arc<dyn Xxx>`），但返回对象是一个**门面**，
门面内部按 provider 形态转发：

```rust
/// provider 的两种形态 —— A 层（进程内真实对象）/ B 层（跨边界，JSON 进出）
pub enum ProviderImpl {
    /// A 层：内置实现或编译期插件，直接持有 trait object
    Native(Arc<dyn std::any::Any + Send + Sync>),
    /// B 层：跨边界插件。**不约定传输方式** ——
    /// 默认实现 = 向长驻 worker 子进程发一帧 / 读回一帧（§5.0）
    /// 备选实现 = 调 cdylib 的 C-ABI 函数指针（§5.1）
    Remote(Arc<dyn SeamInvoker>),
}

/// B 层统一调用面 —— **只约定 JSON 进 JSON 出**，传输无关
pub trait SeamInvoker: Send + Sync {
    fn invoke(&self, op: &str, args: serde_json::Value) -> Result<serde_json::Value, String>;
}
```

以 `agent.loop` 为例（其余接缝同构）：

```rust
/// agent.loop 接缝门面 —— 自身实现 AgentTurnRunner，故消费方拿到的仍是 Arc<dyn AgentTurnRunner>
struct TurnRunnerFacade {
    inner: ProviderImpl,
}

#[async_trait]
impl AgentTurnRunner for TurnRunnerFacade {
    async fn run_turn(&self, request: AgentTurnRequest) -> Result<AgentTurnResult> {
        match &self.inner {
            ProviderImpl::Native(n) => {
                let n = n.downcast_ref::<Arc<dyn AgentTurnRunner>>().ok_or(/* TypeMismatch */)?;
                n.run_turn(request).await
            },
            ProviderImpl::Remote(r) => {
                let args = serde_json::to_value(&request).map_err(|e| e.to_string())?;
                let out = r.invoke("run_turn", args)?;
                serde_json::from_value(out).map_err(|e| e.to_string())
            },
        }
    }
}
```

**收益**：B 层插件「看不见地」接进来，`src/init/state.rs` 与全部消费方**无需改动**。

### 4.1 一处存储 + `CapabilityOrigin` 降级

- `CapabilityRegistration.provider` 类型改为 `ProviderImpl`
- **删除路由语义**：`prepare_registration` 不再按 origin 触发 `evict_builtin_for_external`
  （修缺陷 2）；替换改为显式策略 `RegisterPolicy::{Reject, Replace}`（修缺陷 2 的隐式性）
- **单张表**：特化存储（11 个 `HashMap`）全部删除。取回路径为
  `get(id)` → `downcast::<TurnRunnerFacade>()` → `Arc` unsize 协变为 `Arc<dyn AgentTurnRunner>`
  （修缺陷 1；`Arc<T> → Arc<dyn Trait>` 协变可行，这正是当前注释所认为做不到的那点）
- **删除 `register_external_*` 11 个方法**：它们是用两套存储模拟平权，方向反了
- `CapabilityOrigin` **保留但仅供展示**：`list_with_details` / 审计 / UI 用，不参与任何分支判断
- `PluginCapabilityDescriptor` 保留用于**纯声明型**能力（护照 / 发现索引），
  不再冒充「实现」

### 4.2 补 G4：把剩余硬编码接缝化

| 目标 | 现状 | 动作 |
|---|---|---|
| 工具集 | `ToolRegistry::register_all` 硬编码内置工具 | 注册为 `ServiceDefinition::tool_set()`（该接缝**已声明但无注册路径**） |
| LLM provider | `ProviderRegistry::create_default()` 硬注册 10 家 | 各适配器注册为 `model.provider.{name}` 接缝 |
| 会话存储 | 直接依赖 `dao` / `storage` | 注册为 `session.store` 接缝 |
| 系统提示词组装 | 分散在 agent 路径 | 注册为 `system.prompt` 接缝，段（section）可插拔 |

### 4.3 补 G5：`inject` 依赖收敛

- `ServiceDefinition` 增加 `requires: Vec<String>`
- `CapabilityRegistry` 增加 `pending_consumers`；`register()` 成功后按依赖图驱动「重查」
- 消费方从「`Option` 判空」升级为「显式未激活状态」，让**装配顺序错误可见**
  （这是当前最便宜的可靠性提升：漏注册不再是静默 `None`）
- ⚠ **与插件声明的层次区别（勿混用）**：

  | | `ServiceDefinition::requires`（本节） | 插件 `calls`（§5.2 / §15.6） |
  |---|---|---|
  | 层次 | **宿主内部装配**：服务 → 服务 | **插件 → 接缝**：调用声明 |
  | 回答的问题 | 「谁必须先注册」 | 「谁被允许调用」 |
  | 作用时机 | 启动装配期 | 插件载入校验 + 运行期每次调用前 |

  两者是**两个层面**：前者管**顺序**，后者管**权限**。

---

## 5. B 层载入实现（主路径：子进程；备选：cdylib）

### 5.0 主路径：独立二进制 + 长驻 worker

**形态**：插件被编译为一个**独立可执行文件**（非 cdylib），由宿主 spawn 后**长驻**，
双方以 stdin/stdout 上的**长度前缀 JSON 帧**通信。

```
请求帧：[u32 BE payload_len][JSON: { "op": "execute", "args": { … } }]
响应帧：[u32 BE payload_len][JSON: { "kind": "success", "value": { … } }]
                       或 [JSON: { "kind": "error", "message": "…", "code": "SEAM_EXECUTE_FAILED" }]
```

`invoke` 的 `op` / `args` / 返回结构**与 §5.4 的映射表完全一致** ——
换的只是传输层（帧 vs 函数指针），**门面、单表、`get_*()` 签名、消费方全部不动**。

**宿主侧实现**（`SeamInvoker` 的一个具体实现，故与 A 层同表平权）：

```rust
struct WorkerInvoker {
    /// 禁区 8：必须 tokio::sync::Mutex（std guard 跨 await 是 UB）
    stdin: tokio::sync::Mutex<ChildStdin>,
    stdout: tokio::sync::Mutex<BufReader<ChildStdout>>,
    child: tokio::sync::Mutex<Child>,
    plugin_id: String,
}

impl SeamInvoker for WorkerInvoker {
    fn invoke(&self, op: &str, args: serde_json::Value) -> Result<serde_json::Value, String> {
        // ① 编帧 → ② 写 → ③ flush → ④ 读回一帧 → ⑤ 校验 kind
        // 失败语义：worker 崩溃 ⇒ 通道 EOF ⇒ 返回 Err（而非 panic 宿主）—— 崩溃隔离白送
        //
        // ⚠ 这是**简化模型**。插件间通信（§15.4）要求：
        //    ① 帧带 `request_id` 并维护配对表 ② stdin / stdout 各一把锁
        //    ③ **等待响应期间不得持有写锁**（否则重入必然死锁）
    }
}
```

**为何必须长驻**（§11 实测，这是本节的唯一硬约束）：

| 形态 | 单次耗时 | 结论 |
|---|---|---|
| 长驻 worker（`pipe`） | **18.9 µs**（256 B） | 相对 LLM 推理（0.5–30 s）低 4–6 个数量级 ⇒ 开销可忽略 |
| per-call spawn（现状 `hooks.rs:468`） | **72–195 ms** | 50 节点工作流即 **7.5 s 纯进程创建开销** |

⇒ **长驻是主路径成立的前提**，不是优化项。`spawn ≈ spawn_bare`（两轮一致）证明
per-call 耗时**几乎全部是进程创建**，与插件逻辑复杂度无关。

**相对 cdylib 备选的三项净收益**：

1. **`platform.adapter` / `webhook.dispatch` 从 ❌ 转为 ✅** —— 见 §5.4，覆盖面升至 ~11/11
2. **硬前提掉 2 个**：审计不再是唯一防线（进程边界兜底）；panic 隔离基本消失（worker 崩溃不影响宿主）
3. **插件不再需要「零依赖」** —— 可用 `serde` / `tokio` 等常规 crate
   ⇒ **AI 生成带 `serde` 的 Rust，远比手搓 `#[repr(C)]` C-ABI 自然**（对 §12 管线是决定性优势）

**代价（转为工程项，非取舍）**：① 必须长驻 ② 必须实现流式协议 ③ 必须管 worker 生命周期（孤儿进程，见 §13）
④ 大 payload 需分片（`workflow.evolver` 的种群）。

### 5.1 备选路径参照契约（`dsh-plugin-contract` 逐字核对）

> 以下仅用于 **cdylib 备选路径**（仅建议用于低频热接缝）。主路径**不需要任何 C-ABI**。

```rust
#[repr(C)]
pub struct DshPluginExports {
    pub abi_version: u32,
    pub name:  unsafe extern "C" fn(out_buf: *mut u8, cap: usize, out_len: *mut usize),
    pub setup: unsafe extern "C" fn() -> *mut usize,
    pub describe: unsafe extern "C" fn(handle: *mut usize, out_buf: *mut u8, cap: usize, out_len: *mut usize) -> i32,
    pub invoke: unsafe extern "C" fn(
        handle: *mut usize, op: *const u8, op_len: usize,
        args: *const u8, args_len: usize,
        out_buf: *mut u8, cap: usize, out_len: *mut usize) -> i32,
    pub teardown: unsafe extern "C" fn(handle: *mut usize),
}
pub const ABI_VERSION: u32 = 1;
```

**跨边界类型只有三种**：`*mut usize`（不透明句柄）、`*const u8` + len（字节缓冲）、`i32`（状态码）。
**无任何 Rust trait object 过界。** 契约 crate **零依赖**（不链接 cordis / tokio），
理由（其源码注释原文）：

> A cdylib statically links its own copies of every crate it uses, so any framework call made
> from library-side code would panic ("no reactor running") or duplicate state — instead the
> **HOST implements the framework and the PLUGIN only computes**.

### 5.2 本仓新增 `axagent-plugin-proto`（归属：foundation）

主路径下它是**帧协议 crate**（**不是** C-ABI crate）；备选路径的 `#[repr(C)]` 导出表
在同一 crate 的 `cdylib` feature 下提供。按 `AGENTS.md`「crate 角色」标注为
**foundation**（零 `axagent-*` 依赖）。

**主路径（默认 feature）**：

- `AXAGENT_PLUGIN_PROTO_VERSION: u32`
- `write_frame` / `read_frame`（长度前缀 u32 BE）——
  **实现已在 `src-tauri/benches/plugin_ipc_bench.rs` 写好并实测跑通，直接提取即可**
- `FrameRequest { op, args }` / `FrameResponse { kind, value, message, code }`
  （**插件间通信需再加** `request_id` / `call_chain`，以及 `call_seam` op —— 见 §15.4）
- 声明 JSON schema（**比 dsh-rs 更宽** —— 它只有 `tools`，本仓需覆盖 §2.2 的接缝集）：
  ```json
  { "proto_version": 1,
    "subscribe": ["event.dispatch"],
    "calls": ["workflow.sandbox"],
    "capabilities": [{ "seam": "workflow.sandbox", "op": "execute", "version": "1.0" }] }
  ```
  - `subscribe` —— 插件对事件总线是**订阅方**，不是提供方（§5.4 / §15.3）
  - `calls` —— 插件要**调用**的其他接缝，宿主启动时校验（§15.6）
- 结果协议：
  ```json
  { "kind": "success", "value": { } }
  { "kind": "error", "message": "…", "code": "SEAM_EXECUTE_FAILED" }
  ```
  `code` 须复用 `harness/src/error_codes.rs` 既有常量，不得新造字面量（`AGENTS.md` 错误码规范）

**备选路径（`cdylib` feature）**：

- `#[repr(C)] pub struct AxPluginExports { … }`（结构对齐 §5.1）
- `write_out` / `read_in` 辅助函数

### 5.3 宿主侧载入器（共用骨架，两种 transport）

两种形态的**注册 / 卸载流程完全相同**，只有 transport 构造不同：

1. 读插件声明 → 按 `capabilities[].seam` 构造对应门面（§4.0）
2. `register(id, ProviderImpl::Remote(invoker), RegisterPolicy::Reject)` 入表
3. 卸载：`EffectHandle::undo()` → 出表 → 停 worker / `teardown()`（与 cordis fiber effect 同构）

| | 主路径（子进程，默认） | 备选（cdylib，`cdylib` feature） |
|---|---|---|
| transport 构造 | spawn 独立二进制 + 握手（校验协议版本） | `libloading` dlopen → 读 `AxPluginExports` → 校验 `abi_version` → `setup()` |
| 版本校验 | 帧首 `proto_version` | `abi_version` 字段 |
| 声明获取 | worker 首帧返回 `describe` | 调 `describe` 函数指针 |
| 卸载 | 发 shutdown 帧 → 等退出 → 超时 `kill` | `teardown()` |
| 隔离 | 进程边界（崩溃隔离 + ENV 白名单生效） | 无（§3.3 / §8） |

### 5.4 11 条接缝的 `invoke` op 映射

| 接缝 | op | args（进） | value（出） | B 层适配（**子进程主路径口径**） |
|---|---|---|---|---|
| `tool.set` | `exec` | `{name, args}` | `{content}` | ✅ 最成熟（dsh-rs 已验证） |
| `agent.loop` | `run_turn` | `{request}` | `{result}` | ✅ |
| `workflow.sandbox` | `execute` | `{genome, test_input}` | `{validation}` | ✅ |
| `workflow.reflector` | `reflect` | `{execution}` | `{reflection}` | ✅ |
| `workflow.evolver` | `evolve` | `{population}` | `{population}` | ⚠ 种群可能极大，需分片/流式（**唯一的工程项**） |
| `workflow.optimizer` | `suggest` | `{template, reflection}` | `{suggestion}` | ✅ |
| `workflow.business_rule` | `evaluate` | `{node_type, node_input}` | `{verdict}` | ✅ **改判**：长驻实测 **18.9 µs**，原「高频开销」不成立（§11.1） |
| `message.callback` | `on_message` | `{platform, user_id, chat_id, text}` | `{reply}` | ✅ |
| `webhook.dispatch` | `dispatch` | `{event, data}` | `{result}` | ✅ **改判**：worker 有**独立进程与网络出口**，副作用不落在宿主地址空间；凭证经 ENV 白名单不下发 |
| `session.log.invariant` | `record` / `assert` | `{…}` | `{…}` | ✅ **改判**：同 `business_rule`，高频已不成立 |
| `platform.adapter` | `start`/`stop`/`send` | `{…}` | `{…}` | ✅ **改判**：worker **本身就是长驻进程**，async 生命周期在 worker 内自管，宿主只按 op 转发 |
| `event.dispatch` | — | — | — | ⚠ **本身是总线**：插件应**订阅**它（见 §5.2 声明的 `subscribe`），**不应「提供」它** |

**改判依据**：原 ❌/⚠ 评级中，「❌ 长驻 async 生命周期需函数表 + 宿主代跑 async」
是 **cdylib 特有的困难**（同进程内无法隔离长驻状态）；改为子进程后该困难**自然消失** ——
worker 就是一个独立长驻进程，这正是它最擅长的事。
其余 ⚠（高频）由 §11 实测推翻。

---

## 6. 分阶段落地

| 阶段 | 内容 | 依赖 | 风险 |
|---|---|---|---|
| **P0** | §4.3 `inject` 依赖收敛（纯增量） | — | 低 |
| **P1** | §4.1 单表 + origin 降级 + 删 `register_external_*` ⇒ **修缺陷 1 / 2** | — | 中（改注册表内部） |
| **P2** | §4.2 把 `tool.set` / `model.provider` / `session.store` / `system.prompt` 接缝化 | P1 | 中（触及启动装配） |
| **P3** | §5 `axagent-plugin-proto` + worker 载入器 + **L1 事件桥**（把 worker 接上 `EventDispatchBus`，§15.3）⇒ **B 层落地，且插件之间即可互发事件** | P0–P2 | 中（进程生命周期 + 协议边界） |
| **P4** | §15 L2 接缝调用（`call_seam` + `request_id` 配对 + `call_chain` 环检测 + 深度上限，§15.4 / §15.5）⇒ **跨插件同步调用** | P3 | **高**（并发 / 重入 / 死锁） |

**顺序理由**：P0/P1 是前提（P1 顺带修掉两个已成立缺陷），且**与「要不要做 B 层」无关** ——
即使永不做 P3，P1 也该修。P3 只有在 P0–P2 完成后才有意义，否则插件载入进来接不上像样的接缝。

**为何把 L1 放进 P3、把 L2 独立成 P4**：

- L1（事件）**不需要任何新协议** —— `EventDispatchBus` 已存在（§15.3），
  只需在 worker 载入时挂订阅 ⇒ **与 P3 同批做，边际成本近乎为零**
- 而 P3 一旦交付，**跨插件通信的绝大多数需求已经满足**（§15.1：L1 是首选机制）
- L2 引入的是**同步阻塞的跨进程嵌套调用**，并发 / 重入 / 死锁三个问题都集中在这一块（§15.5）
  ⇒ **风险等级为高，必须独立成阶段**，不可与 P3 混做

---

## 7. 验证门禁（按 CI 口径，与 `.github/workflows/ci.yml` 逐字一致）

```
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
__TAURI_WORKSPACE__=true cargo test --workspace
```

附加（本文档引入的新面）：

- **帧协议边界测试**（主路径）：`write_frame` / `read_frame` 往返一致；
  畸形输入必须**返回 Err 而非 panic 或死锁** —— 至少覆盖：
  ① 长度字段与实际字节数不符 ② 长度超出上限 ③ 截断帧（中途 EOF）
  ④ 非法 JSON ⑤ 合法 JSON 但缺 `kind` 字段
- **进程生命周期测试**：worker 崩溃（`abort`）后 `invoke` 返回 Err 而非宿主 panic；
  worker 僵死（不读 stdin）时 shutdown **超时后强杀**，不阻塞宿主退出
- **门面透明性测试**：同一接缝分别以 `Native` / `Remote` 注册，
  断言消费方 `get_*()` 取回后行为一致（`Arc::ptr_eq` 不适用时改为行为等价断言）
- **集成测试须显式门控**（参照 `plugins/src/lib.rs:51` 的
  `require_plugin_subprocess` 模式：缺环境则打印说明后跳过，**不伪造通过**）——
  该模式对主路径尤其自然，因为主路径本就是子进程集成测试
- **插件间通信测试（§15，随 P3 / P4 分批落地）**：
  ① **帧错配回归（最要紧，P4）**：同一 worker 通道上并发 N 个请求，
     且 worker 在 handler 内回调宿主 —— 断言每个响应按 `request_id` **正确配对**。
     这是 §15.4 扩展 1 的验收条件；没有它，重入必然取错响应
  ② **环检测（P4）**：构造 `A→B→C→B` 调用环，断言**被拒绝且不死锁**
     （测试须自带超时，**超时即判失败** —— 否则「死锁」会伪装成「测试挂住」）
  ③ **深度上限（P4）**：**非环**长链超过阈值即拒绝（环检测覆盖不到的情形）
  ④ **未声明即拒（P3）**：插件未在 `calls` 声明却调 `call_seam` ⇒ 拒绝
  ⑤ **L1 事件桥（P3）**：worker 订阅后能收到宿主分发的事件；
     `Waterfall` 模式下 worker 的 `SubscriberVerdict` 能**改写**事件（§15.3）
  ⑥ **卸载后调用（P3）**：B 卸载后 A 调其接缝 ⇒ 返回明确错误（`SEAM_PROVIDER_UNAVAILABLE`）
     而非**挂起等待**（这是 §15.2 第 1 条的验收条件）
- **备选路径专属（`cdylib` feature 下）**：`#[repr(C)]` 布局断言（`size_of` / 字段偏移），
  且断言须随 feature 门控，不得让主路径 CI 依赖 C-ABI
- 缺陷 1 / 2 的回归测试：
  ① 外部插件声明 `workflow.sandbox` 后 `get_sandbox()` 不得为 `None`
  ② 内置注册被替换后，撤销外部句柄须能恢复（或明确拒绝替换）
- §3.3 两条沙箱缺陷的回归测试（独立于 B 层，属既存 bug）：
  ① `merged_with` 的多插件聚合不得把 `denied_paths` 放宽
  ② `default()` 不得默许 `subprocess_execution`

---

## 8. 沙箱语义：从「唯一取舍」降级为「工程项」

> **本节已按 §3.3 的逐行核对结果重写（2026-09-24）。**
> 初稿的前提「sandbox.rs 的权限模型建立在**子进程边界**上，故 dlopen 使其完全失效」
> —— **不成立**。它不是隔离机制，是宿主侧自觉调用的准入检查函数（见 §3.3）。

### 8.1 两种形态的真实差异（只有一项）

| 检查项 | 主路径（子进程） | 备选（cdylib） | 差异 |
|---|---|---|---|
| ENV 白名单 | ✅ **OS 级强制**（`env_clear` + 回填） | ❌ 同进程无 env 边界，无从隔离 | **← 唯一的真实差异** |
| 路径白/黑名单 | ⚠ 宿主自觉调用 | ⚠ 宿主自觉调用 | **相同** |
| 子进程权限 | ⚠ 宿主自觉调用 | ⚠ 宿主自觉调用 | **相同** |
| 网络权限 | ❌ 仅告警（自述无法拦截） | ❌ 仅告警 | **相同** |
| 崩溃隔离 | ✅ worker 崩溃不波及宿主 | ❌ 同地址空间，panic/segfault 拖垮宿主 | 子进程额外获益 |
| 内存篡改 | ✅ 无法直接改宿主内存 | ❌ 可任意读写宿主地址空间 | 子进程额外获益 |

**⇒ 初稿 §8 的「二选一裁决」消失了**：选主路径时，沙箱**不是失效**，而是
**保住了两份它本来就没有全部拥有的东西** —— ENV 强制 + 进程边界。

### 8.2 但必须诚实地说明剩余缺口（两种形态共有）

**`sandbox.rs` 从来不提供「OS 级文件系统与网络约束」** ——
路径检查是纯字符串比较，不 hook `open()`；网络检查自述仅告警。
⇒ **无论选哪种形态，一个恶意插件都能读写白名单外文件、发起任意网络请求。**
这正是 §12 审计清单必须存在的理由，也是「审计不可绕过」为何关键。

### 8.3 选择建议

| | 主路径（子进程） | 备选（cdylib） |
|---|---|---|
| 建议用途 | **默认，全部接缝** | 仅低频热接缝（若确有性能诉求） |
| 信任要求 | 审计为**纵深防御**之一（进程边界兜底） | 审计是**唯一防线**（须签名/哈希白名单，可复用 manifest 既有 `integrity` 字段） |
| 若不选它 | — | 无损失（主路径覆盖 ~11/11 接缝，§5.4） |

**⇒ 结论：不需要「二选一裁决」。默认主路径，cdylib 仅在有明确性能诉求时按接缝逐个启用。**

---

## 9. 能做到什么程度（结论）

### 9.1 「一切皆插件」（组合模型）：**可做到 100%**

纯 A 层工作，无 ABI 约束。前提是补齐 §4.2 的四块硬编码。补齐后全部可替换能力
经同一注册表注册，无特权核心。

### 9.2 「动态插件」（载入模型）：**可覆盖 ~11 / 11 条业务接缝**

按 §1.1 的**子进程主路径**口径（§5.4 逐条改判的汇总）：

| 等级 | 接缝 | 条数 |
|---|---|---|
| ✅ 完全支持 | `tool.set`、`agent.loop`、`workflow.sandbox`、`workflow.reflector`、`workflow.optimizer`、`workflow.business_rule`、`message.callback`、`webhook.dispatch`、`session.log.invariant`、`platform.adapter` | **10** |
| ⚠ 支持但需工程处理 | `workflow.evolver`（大种群需分片/流式） | 1 |
| ⚠ 不是「提供」而是「订阅」 | `event.dispatch`（总线）—— 插件作为**订阅方**完全支持（§5.2 的 `subscribe`） | — |

**改判说明（相对初稿的 9/11）**：

- `platform.adapter` ❌→✅：原判据「长驻 async 生命周期需函数表 + 宿主代跑 async」
  是 **cdylib 特有困难**；worker 本身就是长驻进程，该困难自然消失。
- `webhook.dispatch` ⚠→✅：worker 有独立进程与网络出口，副作用不落在宿主地址空间。
- `workflow.business_rule` / `session.log.invariant` ⚠→✅：原判据「高频开销」被 §11 实测推翻
  （长驻 18.9 µs/次，相对 LLM 推理低 4–6 个数量级）。

**基础设施类接缝仍不适用 B 层**（存储、提示词组装、provider 注册表）——
但理由与初稿不同：**不是因为跨 C-ABI 无意义**（子进程模式同样能送 JSON），
而是它们**属于宿主核心装配路径**，B 层化无收益、且引入进程间时序复杂度（A 层专属）。

### 9.3 结构性不可能（不是工程量问题）

> **「动态插件提供任意 Rust trait 对象」不可能实现。**

Rust 无稳定 ABI，trait vtable 布局不稳定、泛型在宿主侧单态化。**dsh-rs 同样做不到** ——
它只是把动态面限窄到 `tools` 从而绕开了该问题（其 `describe` 里只有 `inject` + `tools`，
**没有** `llm` / `sessions` / `systemPrompt` / `agents`）。它的「一切皆插件」主体
（`dsh-llm` / `dsh-session` / `dsh-tools` / `dsh-core`）是**同 crate 内的编译期模块**。

### 9.4 一个反直觉结论

> **本仓的接缝选型比 dsh-rs 更适合做动态载入。**

dsh-rs 受限于 cordis 的 `Plugin` trait 是**通用**的（"提供任意副作用"），通用 trait 无法过
C-ABI，只好妥协为「只暴露 tools」。而本仓 §2.2 的 11 条接缝**本身就是 `数据进 → 数据出` 形态**，
天然是 `invoke(op, JSON) -> JSON` 的形状（9/11 严格如此，另 2 条在子进程模式下同样可承载）——
故 B 层覆盖面（~11/11）**宽于 dsh-rs 的 1/N**。

**而且本仓可以根本不碰 C-ABI** —— 因为接缝已是 JSON 形状，直接用子进程帧协议即可（§1.1）。
dsh-rs 之所以必须写 `#[repr(C)]` 契约，是因为它把插件当 cdylib 加载；这是它的选择，不是必须。

### 9.5 一句话总计

| 目标 | 达成度 |
|---|---|
| 一切皆插件 | **100%**（做完 P0–P2） |
| 动态插件 | **~11/11 条业务接缝**（做完 P3）；基础设施接缝 A 层专属 |
| 动态插件提供任意 trait object | **0%，且 dsh-rs 亦为 0%** |
| 插件间通信 | **L1 事件零耦合（基座已存在，P3 接入）**；L2 接缝调用需 4 项协议扩展（P4，§15） |
| 沙箱语义 | **主路径保住 ENV 强制 + 进程边界**；两种形态均**无** OS 级文件 / 网络约束（§3.3 / §8） |

---

## 10. 插件 UI：复用既有动态 UI 模块（**不新建协议**）

### 10.1 结论先行

本仓**已有一套完整的动态 UI 子系统**，插件 UI 落在其既有能力范围内，
**不需要新建 `UISchema` 协议、组件白名单或渲染器** —— 新建即为重复造轮子。

既有模块（已核对存在）：

| 层 | 位置 | 作用 |
|---|---|---|
| 前端核心 | `src/lib/dynamicUI/ComponentRegistry.ts` | 组件注册表 |
| | `src/lib/dynamicUI/SchemaValidator.ts` | schema 校验 |
| | `src/lib/dynamicUI/DataBindingEngine.ts` | 数据绑定 |
| | `src/lib/dynamicUI/EventHandlerEngine.ts` | 事件处理 |
| | `src/lib/dynamicUI/ConditionalRenderer.ts` | 条件渲染 |
| | `src/lib/dynamicUI/dataSourceSecurity.ts` | 数据源安全模型 |
| | `src/lib/dynamicUI/nl2ui.ts`、`nl2ui-edit.ts` | **NL → UI 生成** |
| | `src/lib/dynamicUI/useDataSource.ts`、`useDynamicData.ts` | 数据源 hook |
| | `src/lib/dynamicUI/registerBuiltins.ts` | 内置组件注册 |
| 类型 | `src/types/dynamicUI.ts` | `UISchema` / `DynamicComponentType` / `ComponentRegistryEntry` |
| 存储 | `src-tauri/crates/entities/src/dynamic_ui_schemas.rs` | schema 表 |
| | ...`/dynamic_ui_schema_versions.rs` | **版本管理** |
| | ...`/dynamic_ui_pins.rs`、`dynamic_ui_form_data.rs` | 固定项 / 表单数据 |
| 命令 | `src-tauri/src/commands/dynamic_ui.rs` | 命令面 |
| 集成 | `src/pages/DynamicUIManagerPage.tsx`、`src/components/settings/DynamicPagesSettings.tsx`、`e2e/dynamic-ui.spec.ts` | 管理 / 设置 / E2E |

### 10.2 三处已经是插件接入点（零改动可用）

**① `ComponentRegistry` 的 `namespace` 参数 —— 天生为按来源分组设计**

```ts
register(entry, namespace?)        // fullKey = `${namespace}:${entry.type}`
registerBatch(entries, namespace?)
resolve(type, namespace?)          // 命名空间优先，缺失回退全局
```

⇒ 插件按 `namespace = pluginId` 注册即可，**该接口无需修改**。

**② `SchemaValidator` = 「生成 → 审计 → 加载」流程中的现成审计落点**

插件提交的 schema 必须过此校验方可入库。

**③ `dynamic_ui_schema_versions` 表 —— 插件 schema 的版本管理已有**（含回滚语义）。

### 10.3 硬边界：新组件类型**只能是 A 层**（有类型依据）

```ts
// src/types/dynamicUI.ts
export interface ComponentRegistryEntry {
  type: DynamicComponentType;
  component: React.ComponentType<DynamicUIProps>;   // ← React 组件
}
export type DynamicComponentType = "Container" | "Row" | ... | "CodeEditor" | "FilePreview";  // 封闭联合
```

`component` 是 React 组件，`DynamicComponentType` 是 **TS 封闭联合** ⇒ 新增组件类型必须改类型定义
⇒ **只有 A 层（编译期前端代码）能贡献新组件**。故插件（B 层）能力边界为：

| | 插件能做 | 插件不能做 |
|---|---|---|
| 产出 | ✅ `UISchema` 实例（JSON），**组合既有组件词汇** | ❌ 新增 `DynamicComponentType` |
| | ✅ 声明 `onAction` 动作（回流至插件 `invoke`） | ❌ 注册 `React.ComponentType` |
| 数据 | ✅ 经 `useDataSource`（**须过 `dataSourceSecurity`**） | ❌ 绕过数据源安全模型 |

子进程插件正好是「产 JSON」这一侧，**落在既有模块能力范围内**。

### 10.4 两处真实缺口

**G-a · `is_builtin: i32` 是二值，不足以表达来源**

`dynamic_ui_schemas` 表仅有 `is_builtin`（0/1），无法区分「**AI 生成**」与「**插件注入**」——
二者信任级别不同。建议扩展：

```
origin   : builtin | user | ai | plugin
owner_id : String      // origin = plugin 时 = pluginId
```

收益：**卸载插件时可精确撤销其 UI 贡献**，并**与后端 `CapabilityOrigin` 对账**
（呼应 §4.1 —— origin 降级为纯展示元数据后，前端正需要这份元数据）。

**G-b · 插件贡献导航入口无现成机制**

`src/lib/pageRegistry.ts` 自述为「**内置**路由路径的单一真相源」，新增页面需 3 步编译期改动
（`BUILTIN_PAGE_PATH` + `ContentArea` 路由 + `Sidebar` 导航项）。

⇒ 动态 UI **走 `/dynamic-ui` 管理页 + schema 渲染**，不往 `pageRegistry` 加路由（这是对的）。
但若插件需在侧栏加**一级入口**，目前无机制 —— 独立小缺口，设计时须保持
`pageRegistry` 的「单一真相源」原则不被破坏。

### 10.5 落地所需（仅两样）

1. **来源维度**：`origin` + `owner_id`（G-a）⇒ 使插件 schema 可区分、可撤销
2. **action 回流通道**：`DynamicUIProps.onAction` → 新命令 `plugin_ui_action` → 插件进程 `invoke`

组件词汇、渲染、校验、绑定、事件、版本、持久化 —— **全部复用既有**。

---

## 11. 实测数据：三形态 IPC 成本（**修正 §9.2 的估算**）

基准：`src-tauri/benches/plugin_ipc_bench.rs`（已注册 `[[bench]]`，`harness = false`）。
可脱离整包编译直接跑：`rustc --edition 2021 -O src-tauri/benches/plugin_ipc_bench.rs -o /tmp/pipc`

实测（Windows，本机单次采样）：

| 形态 | 256 B 负载 | 1 MB 负载 |
|---|---|---|
| `inproc`（计算下限，无传输） | **0.3 µs** | 889 µs |
| `pipe`（长驻子进程） | **18.9 µs** | 1 490 µs |
| └ 净传输（pipe − inproc） | 18.6 µs | **600 µs** |
| `spawn`（per-call，现状形态） | **179 ms** | 72 ms |
| `spawn_bare`（仅进程创建 + 退出） | 195 ms | 152 ms |

**关键比**：

```
pipe  : inproc  =    63 ×   （绝对值仍可忽略）
spawn : pipe    = ~8000 ×   （~150 ms vs 19 µs）
```

### 11.1 对 §9.2 的改判

**真正的分界线是「长驻 vs per-call」，不是「进程内 vs 进程外」。**

- §9.2 中标注「高频开销」的接缝（`workflow.business_rule`、`session.log.invariant`）：
  长驻子进程实测 **18.9 µs/次**，相对 LLM 推理（0.5–30 s）低 4–6 个数量级 ⇒ **开销可忽略**，
  §9.2 的「⚠ 受限」评级**应予放宽**。
- 唯一真正的性能风险是 **per-call spawn**（现状 `plugins/src/hooks.rs:468` 的形态）：
  Windows 上 **~150 ms/次**，且**方差极大**（72–195 ms）。一个工作流 50 节点各触发一次
  钩子即 **7.5 s 纯进程创建开销**。
- `spawn ≈ spawn_bare`（两轮一致）⇒ per-call 耗时**几乎全部是进程创建**，
  与插件逻辑复杂度无关 —— 纯架构形态问题。

### 11.2 未测项

- **内存**：RSS 探针仅实现 Linux（`/proc/self/statm`），Windows / macOS 未测，
  仍需以任务管理器 / 活动监视器补充。
- 单次采样、单平台。**建议 macOS / Linux 各跑一次上述 `rustc` 命令后再拍板。**

### 11.3 内存方面（估算，待实测确认）

cdylib 亦**自带一份 std / 依赖副本**（dsh-rs 契约源码注释明载 "statically links its own
copies of every crate it uses"），故 cdylib 与子进程的**内存差距不是「0 vs 5MB」而是
「~2MB vs ~5MB」—— 同一数量级**。⇒ **内存不应作为该决策的主导因素。**

### 11.4 长驻收益表补行：命令通道（2026-11-02 追加，任务 #49 落地）

§11 的三形态对比回答的是「一帧往返省多少 µs」；**命令通道**（`manifest.commands[]` 改走长驻 worker，见 §15.8）
的收益不是延迟数字，而是**预算与错码归类从无到有**：

| 命令通道 | spawn-on-call（旧通道，逐字保留） | 长驻 worker + 命令帧（新通道） |
|---|---|---|
| 延迟 | ~150 ms/次（即 §11 表 `spawn` 形态；命令通道**未单独基准**，低频用户动作） | 与 §11 表 `pipe` 形态同量级（同一帧往返） |
| 宿主等待预算 | **无** —— 进程挂死则调用线程跟着挂 | **随帧必填** `timeout_ms`（缺省 `DEFAULT_COMMAND_TIMEOUT_MS = 30_000`）+ 1 s 收帧宽限，到点宿主强制收口 |
| 超时错码归类 | **无** | `WorkerError::Timeout` → `PluginError::CommandTimeout` → 前端 `COMMON_TIMEOUT` / `retryable` |
| 派发口绑定 | **无** | 帧上 `plugin_id` 与连接身份双路校验，不符**写帧之前即拒** |

⇒ 「真正的分界线是长驻 vs per-call」（§11.1）对命令通道同样成立，但换通道的**第一动机是预算与归类**，
不是 µs —— 命令是低频动作，延迟收益按本表口径引用 `pipe` 形态即可，勿引用「8000×」当命令收益。

---

## 12. 运行期编译管线（生成 → 审计 → 编译 → 载入）

> 这是用户场景的正题：**可视化操作或 AI 辅助生成 Rust 源码 → 审计通过 → 动态编译 → 加载**。

### 12.1 五阶段

```
① 生成      可视化编排 / AI 辅助      产出：Rust 源码 + 声明 JSON（§5.2）
② 静态审计   源码级检查               产出：通过 / 拒绝 + 逐条理由（§12.2 清单）
③ 隔离构建   独立 cargo build          产出：可执行产物（§12.3）
④ 装载      spawn worker + 握手        入表（§5.3）
⑤ 回滚      任一步失败即撤销全部副作用   EffectHandle::undo（§4.1，已有）
```

### 12.2 审计：形式合规**可被绕过**（必须先知道的硬事实）

`build.rs` 与 proc-macro **在编译期执行任意代码** —— 它们不在任何「运行期沙箱」范围内，
且执行在**构建机上**、以**宿主用户身份**运行。

⇒ 一个「源码看起来完全合规」的插件，只要含 `build.rs`，就能在编译阶段读凭据、发网络、
改宿主源码。**因此审计清单的第一原则是「禁」，而不是「查」**：

| 禁止项 | 理由 |
|---|---|
| `build.rs` | 编译期任意代码执行，绕过一切源码级检查 |
| 自定义 proc-macro（白名单外） | 同上（proc-macro 本身是编译期代码） |
| `[dependencies]` 中的 git / path 依赖 | 拉入不可审计的外部代码；应限定 registry 白名单 |
| `unsafe` | 除非显式申请 + 人工复核（可选严格档） |
| 任意路径 `std::fs` / `std::process::Command` | **不拦截**（§8.2 已说明无 OS 级约束），但须源码级标注 ⇒ 用于**信任分级**，不是拦截 |

**⇒ 审计只做「形式合规」不够**，必须与 §12.3 的隔离构建配合，
且 §8.2 的诚实结论要一并接受：**审计不是安全边界，是纵深防御的一层。**

### 12.3 隔离构建（离线 + 受限）

- **离线**：`cargo build --offline` —— 依赖必须**预置**（vendor 目录 / 本地 registry 缓存）
- **独立 `--target-dir`**：不污染主仓库 `target/`
- **无网络**：构建进程必须无网络访问（否则 `build.rs` 可外联）
- **编译前置探测**：见 §14.3（四项，缺一不可）
- **产物**：内容寻址，见 §13.1

### 12.4 与既有入口的衔接（**已核对**，2026-09-24）

**核对结论：`plugin_validate_source` 不处理 Rust 源码，也不处理任何内联内容。**

调用链（逐层核对）：

```
plugin_validate_source                    src-tauri/src/commands/plugin.rs:48
→ PluginManager::validate_plugin_source   crates/plugins/src/manager.rs:393
→ parse_install_source(source)            → 得 PluginInstallSource
→ LocalPath ⇒ load_plugin_from_directory  ;  其余 ⇒ 远程拉取
→ load_manifest_from_directory            crates/plugins/src/manager.rs:1347
→ 读 MANIFEST_FILE_NAME 或 SKILL.md
```

`PluginInstallSource`（`crates/plugins/src/types.rs:734`）**只有四个变体**：

```rust
pub enum PluginInstallSource {
    LocalPath { path: PathBuf },
    GitUrl { url: String },
    NpmPackage { name: String, version: Option<String> },
    OpenClaw { package_id: String },
}
```

⇒ 它处理的是**「来源定位符」**（URL / 包名 / 路径），最终读目录里的 **manifest JSON**
（声明式插件）。**Rust 源码形态完全不存在。**

**因此：`plugin_source_audit` 必须新增，不能扩展 `plugin_validate_source`**
（后者是「拉取 + 读 manifest」，前者是「内联源码 → 审计」，输入形态与职责都不同，
不构成 `AGENTS.md` 禁区 12 意义上的重复定义）。

**但「装载」这一环可以直接复用** —— `PluginInstallSource::LocalPath { path }` 已存在，
且 `materialize_source` 对 `LocalPath` 直接返回该路径（`manager.rs:2252`，**不复制**）。
⇒ 只要把「编译产物 + manifest」落到一个目录，即可直接走既有 `plugin_install` 全链路：

```
plugin_source_generate   前端可视化 / AI 产出源码（可复用 nl2ui.ts 的链路形态）
plugin_source_audit      【新增】审计源码 → 报告（§12.2 清单）
plugin_source_build      【新增】隔离构建 → 落盘目录（产物 + manifest）
plugin_install           【既有】LocalPath 变体 ⇒ 复用全部识别/注册/启用逻辑
```

⇒ **P3 的编译管线新增面比预想小：装载环节零新增。**

### 12.5 一个决定性的有利结论

因为主路径是**子进程 + 允许常规依赖**（§5.0 净收益 3），AI 生成的 Rust **可以正常写 `serde`**
⇒ 生成难度**大幅下降**。对比：若走 cdylib + 零依赖，AI 必须手搓
`#[repr(C)]` + 字节缓冲协议，出错率极高。

**⇒ 这条是「AI 辅助生成 Rust 插件」能否实用化的分水岭，也是选子进程主路径的最强理由。**

---

## 13. 持久化与动态卸载

### 13.1 内容寻址产物

```
artifact_hash = H(source || proto_version || target_triple || rustc_version || build_flags)
```

四要素缺一不可：换 rustc 版本 / 换目标平台 / 换构建 flag 都**必须**产生新 hash，
否则会加载到不兼容产物。**hash 即缓存 key —— 命中即跳过编译**（这是编译开销的唯一解法）。

### 13.2 存储布局（三层）

| 层 | 内容 | 位置 |
|---|---|---|
| 元数据 | `plugin_id` / `version` / `artifact_hash` / `origin` / 权限 / 状态 | DB 表（新增 `plugin_artifact`） |
| 产物 | 编译出的可执行文件 | `<appdata>/plugins/<plugin_id>/<hash>/` |
| 源码 | 原始 Rust 源码 + 声明 JSON | 同上 `source/` 子目录 |

**源码必须留存**，理由有二：① 内容寻址的输入**包含源码**，不留存则无法复现 hash；
② 审计规则升级后需要**重审历史插件**（只留二进制就重审不了）。

> 注：新增表须写 migration（`AGENTS.md` 禁区 10），不要直接改 entity。

### 13.3 动态卸载（四个必须处理的点）

| 问题 | 处理 |
|---|---|
| **in-flight 调用** | `drain`：先出表（不再接新调用）→ 等在飞调用完成 → 再停进程。**不可直接 kill**；出表后的新调用须**立即返回明确错误**（`SEAM_PROVIDER_UNAVAILABLE`，§15.2）而非挂起 |
| **孤儿进程** | 宿主崩溃 / 被杀时 worker 会残留 ⇒ ① worker 侧**监听 stdin EOF 即自退**（主防线）② 启动时按 pid 记录扫残留 |
| **停用 vs 删除** | `disable` = 出表 + 停进程（产物保留，可秒级复启）；`uninstall` = 额外删产物目录 + DB 记录 |
| **热替换** | 版本升级：新版本先入表（新 hash、新进程）→ 旧版本 drain → 旧版本出表。**双进程短暂并存**，避免服务中断 |

### 13.4 与既有基座的衔接

| 需求 | 现状 |
|---|---|
| 卸载即回滚 | ✅ `EffectHandle::undo()`（`reversible_effect.rs`，LIFO） |
| 插件启停（前端） | ✅ `pluginStore.enablePlugin` / `disablePlugin`（命令已存在） |
| 产物存储 | ❌ 无 —— 须新增 `plugin_artifact` 表（§13.2） |
| **worker 进程生命周期（drain / 孤儿回收 / 热替换）** | ❌ **完全没有** —— 这是 P3 的主要新增工作量（§5.0 的代价 ③） |

---

## 14. rustc 三对齐与 MSRV

### 14.1 用户自装 rustc 是否必须与项目版本一致？

**答：必须对齐的**不是**「版本号」，而是三个 ABI 相关项**：

| 项 | 是否需要对齐 | 不对齐的后果 |
|---|---|---|
| **target triple** | ✅ 需要（Windows 上 MSVC vs GNU 决定能否链接系统库） | 产物无法启动 |
| **linker** | ✅ 需要（同上，MSVC toolchain vs MinGW） | 链接失败 |
| **panic 策略** | ⚠ **主路径不需要**（进程隔离，`unwind` / `abort` 都由 worker 自己承受）；**cdylib 备选路径必须 `unwind`** | cdylib 路径下 panic 直接拖垮宿主 |
| **rustc 版本号本身** | ❌ **不需要强制一致** —— 见 §14.2 第 1 条 | — |

**⇒ 主路径下「版本对齐」的压力大幅降低**：JSON 帧协议**不含任何 Rust ABI 假设**。
这是一个反直觉但重要的缓解 ——
**进程边界把「ABI 兼容问题」变成了「数据格式兼容问题」。**

### 14.2 务实做法（四条）

1. **不强制版本一致，改为「编译前探测 + 记录进 hash」**：
   `rustc -vV` 取 `rustc_version` / host triple / release，作为 §13.1 的 hash 输入
   ⇒ **不同版本编译出的产物 hash 不同，各自独立缓存** ⇒ 天然不冲突。
2. **本仓已固定工具链**：`src-tauri/rust-toolchain.toml` 声明 `channel = "1.97.0"`
   ⇒ 若用户自装 rustc，**建议（非必须）`rustup toolchain install 1.97.0`** 对齐宿主，
   可减小 `serde` 等依赖版本漂移带来的差异。
3. **MSRV：已实测（2026-09-24）** —— 三个事实：

   | 项 | 值 | 来源 |
   |---|---|---|
   | 声明值 | **无** | `src-tauri/Cargo.toml:51` 的 `[workspace.package]` 只有 `edition = "2024"`，无 `rust-version` |
   | **工程固定值** | **1.97.0** | `src-tauri/rust-toolchain.toml:2` + `.github/workflows/ci.yml` **四处**（L283 / L397 / L616 / L653） |
   | **依赖树实测下限** | **1.95** | `cargo metadata` 扫描 **1203** 包（**807** 个声明了 `rust_version`）取最大值：`sysinfo 0.39.6` → 1.95；次高 `sea-orm 2.0.2` / `sqlx 0.9.0` 系列 → 1.94 |

   另有硬下限 **1.85**（`edition = "2024"` 的引入版本）。

   **⇒ 建议声明 `rust-version = "1.97"`（取工程实践值），而非依赖树下限 1.95。**
   理由：声明 1.95 等于承诺支持 1.95，但 **CI 只在 1.97.0 上验证过**
   （全仓 grep `msrv` 零命中，**无 MSRV job**）⇒ 声明 1.95 属**未被验证的承诺**。

   ⚠ 注意：1.95 是**当前 lock 下**的实测值，依赖升级会推高它 ——
   若真要在插件模板里放宽到下限，应加 CI job 校验，而非一次性写死。
4. **`edition` 沿用宿主**：宿主为 `edition = "2024"`
   ⇒ 生成的插件模板**统一用 `edition = "2024"`**（保持一致，避免 edition 差异引入的编译差异）。

### 14.3 编译前置探测清单（写进 §12.3）

```
① rustc -vV                          → version / host triple / release
② cargo -V
③ rustup target list --installed      → 目标 triple 是否可用
④ 磁盘 / 内存余量                      → 编译是重资源操作，须预检（防 OOM / 磁盘写满）
```

**①–③ 的取值全部进入 §13.1 的 hash 输入** ——
这是「同一份源码在不同环境下产物可区分」的保证，也是 §14.2 第 1 条得以成立的前提。

---

## 15. 两个插件之间如何通信

### 15.1 结论先行

**插件之间不直连，一律经宿主中转（星型拓扑）。三种机制，前两种可用，第三种禁止：**

| 级别 | 机制 | 耦合度 | 用途 | 状态 |
|---|---|---|---|---|
| **L1** | **事件**（emit / subscribe） | **零** —— 双方互不认识 | 通知、扩展点、流水线钩子 | ✅ **基座已存在**（`EventDispatchBus`） |
| **L2** | **接缝调用**（`call_seam`） | **弱** —— 只依赖接缝 ID，不依赖插件 | 请求/响应、能力复用 | ✅ 门面**天然支持**，需 4 项协议扩展（§15.4） |
| **L3** | 直连 P2P（worker ↔ worker） | 强 | — | ❌ **禁止**（§15.2） |

**核心洞察：插件不互相认识，它们通过「接缝」与「事件」相遇。**
A 声明 `requires: ["workflow.sandbox"]`（§4.3），**不关心谁提供** ——
这个「不关心」正是解耦的全部来源。

### 15.2 为什么必须禁止直连（五条，按严重度）

| # | 问题 | 说明 |
|---|---|---|
| 1 | **生命周期耦合** | B 卸载后 A 持有的通道悬空。经宿主中转则可返回**明确错误**（如 `SEAM_PROVIDER_UNAVAILABLE`），A 可优雅降级 |
| 2 | **无法热替换** | B 升级 = 新进程 + 新 pid（§13.3 热替换）。A 若持有旧通道，**必然失效**；宿主中转则由宿主换掉门面背面的 transport |
| 3 | **权限与审计不可控** | 直连绕过宿主的接缝校验、`call_chain` 记录与审计留证 ⇒ 跨插件调用**不可观测** |
| 4 | **组合爆炸** | N 个插件两两直连 = O(N²) 通道，且每新增一个插件都要重新协商 |
| 5 | **与动态性冲突** | 插件是**运行期生成**的（§12），两两约定私有协议意味着**生成器要感知全图** ⇒ 直接摧毁 §12 的可行性 |

### 15.3 L1 事件总线（**基座已存在，无需新建**）

`crates/harness/src/typed_event.rs` 已提供完整机制（逐条核对）：

```
EventDispatchBus     L138   pub fn subscribe(...)              L161
EventSubscriber      L106   pub trait（Send + Sync + 'static）  L106
EventMatcher         L62    any() / category() / kind() / matches()   L71-86
DispatchMode         L36    Emit / Waterfall / Parallel / Serial
SubscriberVerdict    L49    ← 订阅者可返回裁决（即可拦截/改写）
would_dispatch()     L189   pub fn
subscriber_count()   L181   pub fn
```

**⇒ 插件间通信的 L1 通道已经建好**，只需把 B 层 worker 接上去：

```
插件 A（worker）                宿主                        插件 B（worker）
   ├ emit {event, payload} ──►  dispatch(bus)
   │                              └ 按 EventMatcher 匹配 ──► {kind:"event", …} ──► B 处理
   │                              ◄────── B 的 SubscriberVerdict（含 Waterfall 改写）─┘
   └ ◄─ DispatchOutcome ────────┘
```

**关键**：`DispatchMode::Waterfall` + `SubscriberVerdict` 意味着插件**可以改写事件**
—— 这正是 cordis 的 trait 拦截 / `inject` 在消息层的等价物。
⇒ **A 与 B 永不互指，加载顺序无关，任一可独立卸载。**

**建议**：跨插件通信**首选 L1**。只有需要「同步拿到返回值」时才用 L2。

### 15.4 L2 接缝调用（门面天然支持，但需 4 项协议扩展）

**§4.0 的门面设计已经是 L2 的完整答案** —— 因为门面隐藏了 provider 在哪：

```
A（worker）─ call_seam{workflow.sandbox, execute, args} ─► 宿主
                                                             └ get_sandbox() → 门面
                                                                ├ Native(A 层内置)  ⇒ 直接调用
                                                                └ Remote(B 的 worker) ⇒ 转发一帧
```

⇒ **A 调 B 与 A 调内置实现走的是同一条代码路径，零特殊处理。**
这就是 §4.0「让门面实现原 trait」的红利 —— 平权不只对内/外，也对**插件↔插件**。

**但必须补 4 项协议（§5.2 的帧协议目前不够）：**

| # | 扩展 | 为什么必须 |
|---|---|---|
| 1 | **`request_id`** | ⚠ **最要紧**。当前 §5.0 的 `WorkerInvoker` 是「一帧进、一帧出」的同步往返。一旦重入（A 等 B 时 B 又发请求），**同一通道上会帧错配** —— 响应被错误的等待者取走 |
| 2 | **`call_seam { seam, op, args }`** | 宿主代调的入口 op（现有 op 只有各接缝自身语义） |
| 3 | **`call_chain: [plugin_id…]`** | **环检测**（§15.5）—— 没有它就会死锁 |
| 4 | **并发**：`stdin` / `stdout` 各一把锁 + `request_id` 配对 | 不能用一把大锁把通道串行化，否则 A 等待时 B 的任何进出都被阻塞 |

> ⚠ 即 §5.0 给出的 `WorkerInvoker` 是**简化模型**，落地时必须按上表扩展。
> `request_id` + 配对表是支持并发与重入的**前提**，不是优化。

### 15.5 死锁 / 重入（真实风险，必须正面处理）

**风险**：帧调用是**同步阻塞**的（写一帧 → 读回一帧）。故：

```
A 调 X(B)  →  B 在 handler 里调 Y(C)  →  C 又调 X(B)  →  ✖ 循环等待，三方全挂
```

**三条缓解（须同时具备）**：

1. **`call_chain` 环检测（硬性）**：帧携带调用链 `[a, b, c]`，宿主转发前检查目标是否已在链上
   ⇒ 命中即拒绝（`SEAM_CALL_CYCLE_DETECTED`），**不进入等待**
2. **深度上限**：`call_chain` 长度超过阈值（建议 8）即拒绝（`SEAM_CALL_DEPTH_EXCEEDED`）
   ⇒ 兜住环检测未覆盖的**非环长链**
3. **架构建议（最有效）**：**跨插件解耦优先用 L1 事件**，把 L2 保留给**树状依赖**
   （单向下游调用）。链式回环是设计味道，不是协议该扛的事

**另需注意**：worker 内部若自行调用宿主接缝（`call_seam`），
**宿主必须允许「在飞调用中再发起调用」**，这要求 §5.0 的 `stdin` 锁**不可在等待响应期间持有**
—— 即「写一帧 + 释放写锁 + 等配对响应」，这也是扩展 4 的具体落地要求。

### 15.6 权限与审计（与 §12.2 配合）

- 插件调其他接缝须在 **manifest 显式声明**（新增 `calls: ["workflow.sandbox", …]`）
  ⇒ 宿主校验，未声明即拒绝（复用既有 `PluginCapabilityDecl` 形态扩展，勿新造类型）
- 跨插件调用**全部经宿主** ⇒ 宿主可完整记录 `call_chain` ⇒ 与 §12.2 的审计留证天然打通
- ⚠ **诚实提醒**：§3.3 已核实 `sandbox.rs` 无 OS 级文件/网络约束 ⇒
  插件仍可**自行开 socket 绕过宿主**直连另一个插件
  ⇒ **L3 是「架构禁止 + 审计发现」，不是「技术上不可能」**。
  若要硬禁，需 OS 级网络隔离（超出本文档范围，且两形态共有的限制）

### 15.7 与既有代码的衔接

| 需求 | 现状 |
|---|---|
| L1 事件订阅 | ✅ `EventDispatchBus::subscribe` / `EventSubscriber` / `EventMatcher` **全部已有** |
| L1 事件匹配 | ✅ `EventMatcher::{any, category, kind}` 已有 |
| L1 改写/拦截 | ✅ `DispatchMode::Waterfall` + `SubscriberVerdict` 已有 |
| A 层插件间通信（进程内） | ✅ `HookRunner::register_in_process_hook(Arc<dyn PluginHook>)`（`crates/plugins/src/hooks.rs:138`）已存在 |
| L2 门面转发 | ✅ §4.0 门面天然支持（Native / Remote 分支） |
| L2 协议扩展（4 项） | ❌ **新增** —— §15.4 表格 |
| 环检测 / 深度上限 | ❌ **新增** |
| `calls` 声明 + 宿主校验 | ❌ **新增** |
| OS 级禁止插件自行开 socket | ❌ **不存在**（§15.6，两形态共有） |

**⇒ 结论：L1（首选机制）的基座已全部存在，只需接 worker；
L2 的架构支点（门面）也已存在，只需补 4 项协议扩展 + 环检测。**

### 15.8 命令帧通道（`ops::COMMAND`，任务 #49 落地 · 2026-11-02 补录）

**定位**：§15.1–§15.7 谈的是**插件 ↔ 插件**（经宿主中转）；本节补**宿主 → 插件**的命名命令派发通道。
它与 L2 复用同一信封与并发模型（`request_id` 配对、专用读线程、`call_chain` 语义），故不新造协议层。

**协议形态**（`axagent-core/crates/plugin-proto/src/lib.rs`）：

| 项 | 内容 |
|---|---|
| op | `ops::COMMAND = "command"` —— 与 `event` / `ui.action` 同属「宿主 → worker」方向的框架 op；插件不在 manifest 里声明它，worker 的 `serve` 处理器实现它才能承接命令 |
| 类型 | `CommandFrame`（请求：`plugin_id` / `name` / `input` / `timeout_ms`）→ worker 回 `CommandReply`（成功）或 `CommandFault`（失败码 + 消息）；宿主解码面为 `CommandOutcome` 两分支，帧形状校验错误为 `CommandFrameError` |
| 预算 | `DEFAULT_COMMAND_TIMEOUT_MS = 30_000`（随帧下发，调用方可显式覆盖） |
| 错码 | 5 个 `PLUGIN_COMMAND_*`：`UNKNOWN` / `INVALID_ARGS` / `ID_MISMATCH` / `TIMEOUT` / `FAILED` |
| 版本 | **`AXAGENT_PLUGIN_PROTO_VERSION` 未 bump（仍为 1）**。加法式降级判据：旧 worker 对未知 `command` op 按既有约定回 `PLUGIN_UNKNOWN_OP` 错误帧 ⇒ 新宿主解码为 **Fault ⇒ 降级为「失败」而非「误成功」**。该判据由测试 `command_frame_is_additive_no_proto_bump_required` 锁定 |

**三道写帧前防线**（`axagent-plugins/crates/plugins/src/worker.rs`，`WorkerInvoker::execute_command`；
全部落在**写帧之前**，除等待上限外没有任何「先挂起再判错」路径）：

| # | 防线 | 语义 |
|---|---|---|
| ① | drain 闸门 | 停机窗口内拒收新命令帧，与接缝入口同语义 |
| ② | pluginId 宿主绑定 | 帧上 `plugin_id` ≠ 本连接身份（注册表权威键）⇒ `PLUGIN_COMMAND_ID_MISMATCH`，**帧不发出去** —— 「派发口发错插件」在宿主侧掐死，不靠 worker 兜底 |
| ③ | 等待上限 | = 帧预算 `timeout_ms` + `COMMAND_FRAME_GRACE_MS = 1_000` 收帧宽限；到点**撤配对槽位**并报 `WorkerError::Timeout` |

**宽限与迟到帧语义（如实声明）**：宽限给「恰在预算边界算完并回帧」的 worker 留收帧窗口，边界上不误杀；
超时**不等价于取消** —— 对端可能仍在执行，迟到响应帧由读线程按「无等待者」丢弃并留警告日志；
本批**不做跨帧取消协议**。僵死 worker 的 unload：drain 后 shutdown 帧仍走无限 `recv` 等它自己醒
（接缝已被标「可疑」时任务 #53 改为跳过 shutdown 帧直接急停），最终兜底靠 `kill_child`（`Drop` 路径）。

**错误面（前端零新码）**：`PLUGIN_COMMAND_*` 五码**不进前端错误码面**，只保留在 `PluginError` 的 detail 原文。
`src-tauri/src/commands/plugin.rs` 的 `plugin_execute_command` 由纯函数 `plugin_command_error_response` 归类到
既有已译码（`CommandTimeout`→`COMMON_TIMEOUT`/retryable、`NotFound`→`COMMON_INVALID_INPUT`/validation、
`PermissionDenied`→category=permission_denied、其余→`COMMON_INTERNAL` 且协议码留在 detail）——
三个码均已在 `src-tauri/src/commands/error_code.rs` 且 11 语 locale 有键 ⇒ 本批**零 locale 补译**。

**唯一可观察行为翻转（兼容口径）**：manifest 同时声明 `worker` + `commands` 的插件，启用后命令从 spawn
**改走命令帧**（`command` 程序串不再执行；worker 当前没在跑 ⇒ 退回 spawn-on-call，命令不静默消失）。
仓内 bundled 无此类插件 ⇒ 风险面在第三方；兼容判据 = 「未长驻 worker 的插件，旧语义逐字节不变」。

**与 §15.5 / §15.7 的关系**：命令通道即扩展 ①（`request_id` 配对）+ ④（等待期间不持锁）在命令方向的首个完整用户。
⚠ **写侧背压未修**：写帧仍是 `stdin.lock()` + 阻塞写，对端僵死且管道满时调用线程会在**写侧**卡住，
`recv_timeout` 覆盖不到（任务 #56，需 writer 线程/异步写 = 接缝栈重构）——这是 §15.5「等待期间不持任何锁」
原则在写侧的既存缺口，命令与接缝两条通道共有。接缝方向的**收侧**无限等待（D1）在 2026-11-02 复核时
已由任务 #53 闭合：接缝公共入口带 `DEFAULT_SEAM_TIMEOUT_MS = 15_000` 预算，超时置「连接可疑」闸拒绝后续调用。

---

## 附：本文档修订记录

| 日期 | 修订 |
|---|---|
| 2026-09-23 | 初稿（§1–§9），按 **cdylib 路径**撰写 |
| 2026-09-24 | 补 §10（插件 UI 复用既有动态 UI 模块）、§11（三形态 IPC 实测，推翻估量） |
| 2026-09-24 | **主备倒置修正**：§1.1 新增口径说明；§5 重写（§5.0 主路径）；§6/§7/§8/§9.2/§9.4/§9.5 同步；§3.3 新增（sandbox.rs 逐行核对结论，推翻 §8 前提）；补 §12（编译管线 + 审计）、§13（持久化与卸载）、§14（rustc 对齐与 MSRV） |
| 2026-09-24 | **两项待确认已核实**：① §12.4 —— `PluginInstallSource` 四变体无源码形态 ⇒ 审计命令必须新增，但装载可复用 `LocalPath` 全链路；② §14.2 —— MSRV 实测：声明值无 / 工程固定 1.97.0 / 依赖树下限 1.95（`sysinfo 0.39.6`），建议声明 1.97 |
| 2026-09-24 | **补 §15（插件间通信）**：L1 事件 / L2 接缝调用 / L3 直连禁止；核实 `EventDispatchBus` + `EventSubscriber` + `EventMatcher` + `SubscriberVerdict` 基座已存在；提出 4 项协议扩展（`request_id` / `call_seam` / `call_chain` / 并发锁）+ 环检测；§5.0 标注为简化模型，§5.2 补 `calls` 声明 |
| 2026-09-24 | **§15 方案全文档对齐**：§6 拆出 **P4**（L1 并入 P3、L2 独立为高风险阶段）；§7 补**插件间通信测试** 6 项（含帧错配回归、环检测超时判失败）；§4.3 补「`requires`（宿主装配顺序）vs `calls`（插件调用权限）」层次澄清；§9.5 / §13.3 / §2.2 / §1.1 交叉引用同步 |
| 2026-11-02 | **任务 #49/#53 落地补录（纯追加，不动原文）**：新增 §15.8（命令帧通道 `ops::COMMAND`：三道写帧前防线、收帧宽限与迟到帧丢弃、未 bump 版本的加法式降级判据、前端零新码归类、唯一行为翻转的兼容口径）与 §11.4（长驻收益表补「命令通道」一行：收益 = 预算与错码归类从无到有）；§15.8 同时登记未修项（写侧背压 stdin 锁 = 任务 #56；跨帧取消协议不存在；接缝收侧 D1 已由 #53 闭合） |