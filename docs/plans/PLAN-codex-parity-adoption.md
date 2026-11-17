# PLAN：codex 后端对齐借鉴项落地

> 状态：active ｜ 日期：2026-09-25 ｜ 对应版本：v2.9.9


- 日期：2026-09-25
- 依据：`AUDIT-codex-parity-gap-2026-09-24.md`（§6 借鉴清单为范围来源，§7–§10 为机制细节来源）
- 对照对象：`openai/codex` 的 `codex-rs`（commit `282cd7b019378746cb87bd91a95d8b4bcae12aa3`）
- 编号：本计划用 **R1–R4**（四阶段）编号，**不复用**源码注释里既有的 `PLAN-codex-parity P0-1/P0-2/P0-3`
  标签 —— 那是上一轮已落地的基线，且其文档在仓库中不存在（见 §0.3）
- 范围：审计 §6 全量十项（明确排除项见 §8）

---

## 0. 动笔前的三处订正（必读）

写计划时逐行读了项目侧代码，发现审计文档有两处结论已被现实推翻、一处文档悬空。**先订正再排期**，
否则会去修一个已经修好的东西。

### 0.1 OnFailure 重试**已经实现** —— 审计 §0.1 与 §6 首行言过其实

审计 §0.1 的论证是「`decide()` 无状态 ⇒ 重试在决策层无法表达 ⇒ OnFailure 是空承诺」。
前半句属实，后半句不成立：**重试回路不在 `decide()`，在 `run_sandboxed`**。

`src-tauri/crates/tools/src/tools/bash.rs:579-586` 已有完整回路 —— 沙箱内非零退出 + `OnFailure` +
用户批准 → `run_direct` 沙箱外重试一次。所以 `src-tauri/crates/harness/src/approval_policy.rs:20-21`
的承诺**基本已兑现**。

真实剩余缺口只有一处，且它被代码自己写明了：

> `src-tauri/crates/tools/src/tools/bash.rs:445-447` —— 「OnFailure 策略（P0-2）：沙箱内非零退出时
> 询问用户，批准后沙箱外重试一次。**v1 不精确判别失败原因是否为沙箱限制**（stderr 语义分析阶段 2 补齐），
> **任何非零退出都触发询问**」

即：缺的不是「重试」，是 **拒绝判据**。任何命令自身报错（`exit 1`）都会被误判为沙箱拒绝并多问一次。
这正是审计 §6 第二行 `is_likely_sandbox_denied` 要解决的事 —— 该项照做，但定位要从「补缺能力」
改为「消误判」。

### 0.2 上下文预算**已部分按模型窗口取** —— 审计 §2 的「常量与真实窗口解耦」只对了一半

- `src-tauri/src/context_manager.rs:62-81` 的 `should_auto_compress` **已经**收
  `model_context_window: Option<u32>`，`:71` 的阈值就是 `窗口 × THRESHOLD_RATIO`；
- `src-tauri/src/context_manager.rs:94-99` 的 `build_context` 同样收该参数，注释明写
  「Sliding window is applied only when `model_context_window` is `Some`」。

**仍硬编码的是「分量预算」**：`src-tauri/src/context_manager.rs:24-37` 的 `token_budget`
模块（`SYSTEM_PROMPT 8000` / `RETRIEVED_MEMORIES 10000` / `SKILLS 5000` / `NUDGES 2000`）
是绝对值，不随窗口伸缩；`HISTORY_RATIO 0.65` 是**比例**（已随窗口伸缩，无需改）。

所以 R2-3 的范围收窄为：把 `token_budget` 的绝对值改为「比例 + 上限」双约束，**不动**已经正确的
阈值逻辑。

### 0.3 `PLAN-codex-parity` 文档不存在，14 处源码引用悬空

`src-tauri/crates/tools/src/approval.rs:3`、`src-tauri/crates/tools/src/win_sandbox.rs:3`、
`src-tauri/crates/harness/src/sandbox_policy.rs:3`、`src-tauri/crates/entities/src/session_events.rs:2`
等 **14 个源码文件**的模块注释引用了 `PLAN-codex-parity P0-1/P0-2/P0-3`，但：

- `docs/plans/` 下 `*codex*` 零命中；
- 全仓 `.md` 搜 `codex-parity` 零命中。

本计划按用户决定用独立新名（`PLAN-codex-parity-adoption.md`）、独立编号，**不继承**那些标签。
悬空引用本身不在本计划范围内，但建议单独处理（补文档 / 改注释为可回溯描述）—— 见 §7 O-6。

---

## 1. 四阶段总览

| 阶段 | 编号 | 项 | 审计依据 | 改动面 | 前置依赖 |
|---|---|---|---|---|---|
| R1 最小闭环 | R1-1 | `is_likely_sandbox_denied` 判据 | §6 第 2 行 / §7.6 | 小（新增 1 模块 + 1 处调用） | 无 |
| | R1-2 | 收窄 R1 的询问条件（消误判） | §0.1 订正后新增 | 小（1 处条件 + 1 处注释） | R1-1 |
| | R1-3 | `decide()` 加 `Attempt` 入参 | §6 第 1 行 | 小（签名 + 单调用点） | 见 §7 O-1 |
| | R1-4 | 沙箱/审批档位写进 system prompt | §6 第 3 行 / §0.2 | 中（预算内 + 模板） | 无 |
| R2 策略与契约 | R2-1 | 批准沉淀为规则 | §6 P1 / §10.3-10.5 | 大（实体 + trait + 匹配器 + 落盘 + 前端） | 阶段启动前勘察（§2.1） |
| | R2-2 | 事件契约枚举化 | §6 P1 / §4.4 | 大（后端枚举 + 生成器 + 前端收窄） | 阶段启动前复核 4 锚点 |
| | R2-3 | 分量预算按窗口取 + 剩余额度工具 | §6 P1 / §0.2 订正 | 中（预算函数 + 新工具） | 工具注册机制勘察 |
| R3 Windows + 审查 | R3-1 | Windows 沙箱补三件 | §6 P2 / §8.1-8.4 | 大（令牌换代 + ACE + 网络） | 见 §7 O-5 |
| | R3-2 | guardian 最小形态 | §6 P2 / §9 | 中（契约 + fail-closed + 预算） | 复用点复核 |
| R4 可回放与传输 | R4-1 | 压缩进事件流 + prompt 版本化 | §6 P3 / §2 | 中（4 处 compact 收敛 + 事件） | 4 处 compact 定位 |
| | R4-2 | `previous_response_id` 增量 + cache key + head/tail 截断 | §6 P3 / §4.3 | 中（上线点 + 少数调用点） | 截断函数定位 |

**阶段划分原则**：R1 单独成阶段，因为它同时满足「改动面最小」与「可独立验证」——三项都落在
`src-tauri/crates/tools/` 与 `src-tauri/src/context_manager.rs` 内，不依赖任何未核实锚点。
R2 之后每阶段都要求在**启动前**先完成一次定点勘察（见各节），避免带着未核实假设开工。

---

## 2. R1：最小闭环（沙箱拒绝 → 有条件重试）

### R1-1 新增 `is_likely_sandbox_denied` 判据

**目标**：把「疑似沙箱拒绝」变成一个可复用、可单测的纯函数判据。

**依据**（codex 侧，已逐行复核）：`sandboxing/src/denial.rs L13-L72` 给出了两级判据，且其注释
自陈这是启发式而非内核级协议（`L5-L9`）。原样照抄即可，**不需要先造结构化拒绝码**。

**改动面**：

| 文件 | 动作 |
|---|---|
| `src-tauri/crates/tools/src/sandbox_denial.rs` | 新增模块：`pub fn is_likely_sandbox_denied(exit_code: i32, stderr: &str) -> bool` |
| `src-tauri/crates/tools/src/lib.rs` | 增加 `pub mod sandbox_denial;` 声明 |

判据映射（codex → 项目）：

| codex 分支 | 项目是否适用 | 理由 |
|---|---|---|
| `sandbox_type == None \|\| exit_code == 0` → false | 改由调用方前置判断 | 项目在 `run_sandboxed` 内已保证是沙箱路径 |
| 退出码快路：**排除** `2` / `126` / `127` → false | **照抄** | `127`=命令未找到、`126`=不可执行，都是「用户命令自身的问题」，不是沙箱 |
| Unix `LinuxSeccomp` 且 `exit_code == 128 + SIGSYS` → true | **不适用** | 项目 Linux 侧走 `unshare` 命名空间（`src-tauri/crates/tools/src/linux_sandbox.rs:3-14`），**不装 seccomp**，不会有 SIGSYS |
| 7 条关键词小写匹配 stderr | **照抄 + 2 条 Windows 扩展** | `operation not permitted` / `permission denied` / `read-only file system` / `seccomp` / `sandbox` / `landlock` / `failed to write file`；**另加 `is denied` + `拒绝访问`** —— 见下方「实施订正」 |

**验收判据**：单测矩阵覆盖 `exit_code ∈ {0, 1, 2, 126, 127}` × stderr 含/不含各关键词；
边界明确锁住「`exit 2` 即使 stderr 含 `permission denied` 也**不**判为沙箱拒绝」（这是 codex 快路
的语义，别改）。

**实施订正（2026-09-25，落地时实测，含一次自我推翻）—— 必须加 Windows 关键词 + 按码页解码，
否则 R1-2 在 Windows 上不是「消误判」而是「删能力」**：codex 的 7 条词全部面向 Unix。本机
（zh-CN）**受限令牌沙箱内**实测：

```
cmd /d /s /c "echo blocked > C:\Windows\axagent_bash_onfailure_probe.txt"
  → exit 1；stderr = GBK 编码的「拒绝访问。」（stdout 为空）
```

即 Windows 沙箱被拒时的文案由操作系统给出、**随系统语言本地化，且走 ANSI/OEM 码页（zh-CN 为
GBK）而非 UTF-8**。这带来两个必须处理的点：

1. **关键词**：实际实现为「codex 7 条 + `is denied`（英文系统 / PowerShell 一族）+ `拒绝访问`
   （简体中文系统的本地化文案）」，并在模块文档里显式标注这是对 codex 的**刻意偏离**及理由。
2. **解码**：`String::from_utf8_lossy` 会把 GBK 字节解成 U+FFFD 乱码 ⇒ 关键词永远匹配不到。
   故新增 `decode_console_text()`：先按 UTF-8 试解（现代 CLI 多为 UTF-8），失败再按本机 ANSI
   码页（`CP_ACP`）解码；顺带修掉了沙箱失败时用户看到 stderr 乱码的问题。

⚠ **订正记录**：本节初稿曾据一次探针写下「沙箱被拒文案是英文 `Access is denied.`」，据此只加了
`is denied` 一条。该结论**与本仓受限令牌沙箱路径的实际文案不符** —— R1-2 的集成测试
（`on_failure_sandbox_denial_asks`）首次运行即失败（断言 `asked==1` 实得 `0`），诊断输出才暴露出
真实 stderr 是 GBK 中文。教训：**「本地敲一条命令看到的报错文案」不能代表受限令牌子进程的文案**，
判据类改动必须以目标路径（沙箱内）的实测输出为准。

**门禁**：`cargo check`；`cargo clippy -p axagent-tools --all-targets -- -D warnings`
（⚠ 必须带 `--all-targets`，否则新写的测试代码不被 lint）。

### R1-2 收窄询问条件（本阶段真正的用户可见收益）

**目标**：把「沙箱内任何非零退出都问用户」收窄为「疑似沙箱拒绝才问」。

**改动面**：

| 文件 | 位置 | 动作 |
|---|---|---|
| `src-tauri/crates/tools/src/tools/bash.rs` | `:483-488` | 条件由 `exit_code != 0` 改为 `exit_code != 0 && is_likely_sandbox_denied(exit_code, stderr)` |
| `src-tauri/crates/tools/src/tools/bash.rs` | `:445-447` | 更新「v1 不精确判别」的注释（改后该声明失效） |

**实施时必须先确认一件事**：`wait_sandbox_result` 返回的 `(result, exit_code)` 里 `result` 是否
携带 stderr 文本（`run_sandboxed` 当前只用它算 `TaskResult`）。若不携带，需要让它一并返回 stderr
—— 这会略微扩大改动面，但**不得**为了省事改成「不传 stderr 只按退出码判」（那等于退化为
codex 快路的反向版，会把 `exit 1` 全判成沙箱拒绝）。

**验收判据**：
- 新增「命令自身报错（`exit 1`，stderr 无关键词）**不再**触发询问」的测试；
- 保留「stderr 含 `operation not permitted` 时**仍然**触发询问」的测试；
- 与 `src-tauri/crates/harness/src/approval_policy.rs:20-21` 的承诺逐字对照：
  承诺是「失败**且疑似沙箱限制**时询问」—— 改后才算真正兑现。

**实施结果（2026-09-25）**：`wait_sandbox_result` 原先只返回 `(ToolResult, i32)`，`ToolResult`
里是**已格式化并截断**的文本，不便于做判据。已改为返回 `(ToolResult, i32, String)`（第三个值是
未截断的 stderr 文本，经 `sandbox_denial::decode_console_text` 按控制台码页解码 —— 原始
`from_utf8_lossy` 会把 Windows 本地化报错解成乱码，见 R1-1「实施订正」），**未**退化为
「只按退出码判」。该解码同时用于展示，沙箱失败时用户看到的 stderr 不再是乱码。

### R1-3 `decide()` 加 `Attempt` 入参

**目标**：让决策层能表达「这是沙箱被拒之后的升级尝试」。

**依据**：codex 侧 `core/src/guardian/decision.rs L45-L131` 把 `escalated_exec` 做成显式布尔量
（由 `sandbox_permissions.requires_escalated_permissions()` 推导，同文件 `L113`），而不是靠错误
字符串猜。

**改动面**：

| 文件 | 动作 |
|---|---|
| `src-tauri/crates/tools/src/approval.rs:75-79` | `decide()` 加第四参 `attempt: Attempt` |
| `src-tauri/crates/tools/src/approval.rs` | 新增 `pub enum Attempt { Initial, AfterSandboxDenial }` |
| `src-tauri/crates/tools/src/tools/bash.rs:194` | 唯一生产调用点，传 `Attempt::Initial` |

**⚠ 先看 §7 O-1**：R1-2 落地后重试回路已在 `run_sandboxed` 内闭合，本项的真实增量收益只剩
「重试那一次不走沙箱、直接沙箱外跑」。若不做这个扩展，本项应**降级或砍掉**，只改签名不加语义是
纯粹的形式主义。故本项排在 R1-2 之后，由 O-1 裁定。

**验收判据**（若保留）：`decide()` 的现有 12 个测试全部补 `Attempt::Initial`；新增
`AfterSandboxDenial` 的矩阵测试。

**裁定结果（2026-09-25，R1 实施时）**：**砍掉，本轮不做** —— 按上面 §7 O-1 写明的倾向执行
（R1-2 已闭合重试回路，只改签名不加语义属形式主义）。若后续确需「升级尝试不走沙箱」的语义，
再连同该行为一起做，届时同步补 R1-2 的矩阵测试。

### R1-4 把沙箱/审批档位写进 system prompt

**目标**：让模型知道自己在什么边界内，从而主动规避越界写路径、主动请求审批。

**依据**：审计 §0.2 已核实 codex 把档位写成 system prompt 模板
（`prompts/src/permissions_instructions.rs L27` + `prompts/templates/permissions/` 下
`approval_policy/*.md` 与 `sandbox_mode/*.md`）。项目侧 `src-tauri/src/context_manager.rs` 里
`sandbox|沙箱|approval|审批` **零命中**（审计已核实）。

**改动面**：

| 文件 | 动作 |
|---|---|
| `src-tauri/src/context_manager.rs` | 新增「按档位渲染权限说明段」的纯函数（`render_permission_notice(sandbox_mode, approval_policy) -> String`），并用 `include_str!` 引模板文件 |
| 模板文件（放 `src-tauri/src/prompts/permissions/` 下，与 codex 同构） | `sandbox_mode/{read-only,workspace-write,danger-full-access}.md` + `approval_policy/{never,on-failure,on-request,untrusted}.md` |
| `src-tauri/src/context_manager.rs`（system messages 组装处） | 注入该段，并**计入 `token_budget::SYSTEM_PROMPT` 预算** |

档位来源已存在，无需新增数据通路：`src-tauri/crates/harness/src/types/settings_chat.rs:167-173`
已有 `sandbox_mode` 与 `approval_policy` 两个 settings 字段。

**两个必须先定的点**（阶段内第一步解决，不属开放决策）：
1. **模板语言**：system prompt 是给模型看的，不是 UI 文本 —— 按项目现有 system prompt 的语言惯例
   处理，**不套用「新增 UI 文本必须 11 语言」**那条规则（那条管的是 `src/i18n/locales/`）。
2. **预算冲突**：注入段落会挤占 `SYSTEM_PROMPT 8000` 的额度。若 R2-3 先落地则从窗口比例里出；
   R1 阶段先从 8000 内挤，并在 `:24-37` 注释里写明该额度已被权限段占用。

**验收判据**：单测断言「三档沙箱 × 四档策略」渲染出的段落两两不同，且 `DangerFullAccess`
段的语义与 `src-tauri/crates/harness/src/sandbox_policy.rs:21-57` 的定义一致（不得把
`DangerFullAccess` 描述成仍有写限制）。

**R1 阶段门禁**：`cargo check`；定点 `cargo clippy -p axagent-tools -p axagent-harness --all-targets -- -D warnings`；
`__TAURI_WORKSPACE__=true cargo test -p axagent --lib`（Windows 下必须带该环境变量，否则测试 exe
启动即 `STATUS_ENTRYPOINT_NOT_FOUND`）。

---

## 3. R2：策略与契约

### 3.0 阶段启动前的定点勘察（必做，否则本阶段会带错假设开工）

R2 的三项都依赖我**未逐行复核**的锚点。开工第一步先把下面这些读实：

- **R2-1**：`CommandValidator` 的调用现状、审批链路、持久化落点、分词器能力（结果我已探明，见 3.1，
  但仍需读 `src-tauri/crates/kit/src/command_validator.rs:43-70` 之外的 `classify_segment` 全文）；
- **R2-2**：`scripts/check-contracts.mjs:13-14` 的校验项 G、`src-tauri/schema-gen/src/main.rs:195`
  的 `IPC_DTO_TYPES` 清单（`:287` 为输出落点、`:48` 为 `ipc-types` 子命令）、
  `src/types/generated/ipc.ts:1-5`、`src/lib/invoke.ts:5` 四个锚点；
- **R2-3**：工具注册机制与一个只读工具样板；`src-tauri/crates/agent/src/context_window.rs` 是否
  已提供按模型名查窗口的能力。

### 3.1 R2-1 批准沉淀为规则

**目标**：用户批准一条命令后沉淀为规则，下次同命令不再询问（codex `execpolicy` 的 amendment 语义）。

**已探明的现状（本节均为我逐行/grep 核实）**：

1. `CommandValidator` **没有生产调用点** —— 全仓 grep 只有定义
   （`src-tauri/crates/kit/src/command_validator.rs:20-70`）、模块声明
   （`src-tauri/crates/kit/src/lib.rs:8`）与测试
   （`src-tauri/crates/kit/src/command_validator.rs:362-446`）。
   ⇒ 它是一套**未接线**的白名单校验器，不是当前审批链的一环。
2. **分词器已具备**：`src-tauri/crates/tools/src/bash/parser.rs:66-73` 的
   `parse_command(input) -> Result<ParsedCommand, String>`，`:56-64` 的 Token 枚举已含
   `Pipe` / `And` / `Or` / `Semicolon` / `Background` / `Redirect*`。
   ⇒ codex 的「按连接符切段并逐段复核」在本仓**有现成基础，不需要引入 tree-sitter**。
3. **架构硬约束**：`tools` 是 **hybrid** crate，按 AGENTS.md 铁律**禁止依赖 `entities` / `dao`**。
   ⇒ 规则的读写**必须经 `axagent_harness` trait**，落盘实现放 wiring 层（`src-tauri/src/init/` 或
   `src-tauri/src/commands/`）。
4. **建表落点不是 migration**：`src-tauri/crates/dao/src/migrations/mod.rs:4-23` 明确
   「建表 / 加列 / 加索引：改**实体声明**或 `crate::reconcile::extras` 的声明，**不要新建迁移**」。
   ⇒ 规则表 = `src-tauri/crates/entities/src/` 新增实体 + `crate::reconcile::extras` 声明。
5. **审批钩子的位置**：`ask_user_blocking` 是 trait 方法
   （`src-tauri/crates/harness/src/tool.rs:130`），实现在 wiring 层
   （`src-tauri/src/commands/agent/mod.rs:84`）；bash 侧调用点在
   `src-tauri/crates/tools/src/tools/bash.rs:500`。⇒ 落盘点应放在 **trait 实现附近或批准返回后**，
   而不是 tools crate 内直接落库。

**改动面（四块）**：

| 块 | 文件 | 动作 |
|---|---|---|
| ① 规则存储 | `src-tauri/crates/entities/src/`（新增实体）+ `src-tauri/crates/dao/src/reconcile/extras.rs` 声明 | 规则表：`program`（首 token）、`args_prefix`、`decision`、`created_at`、`source`（哪次批准） |
| ② 能力契约 | `src-tauri/crates/harness/src/`（新增 trait，如 `ApprovalRuleStore`） | 查询/写入/撤销；consumer 只认 trait |
| ③ 匹配器 | `src-tauri/crates/kit/src/`（与 `command_validator.rs` 同层，**纯函数**） | 首 token 精确索引 + 多命中取最严 + 危险包装器黑名单 |
| ④ 接线与 UI | `src-tauri/src/init/`（注入实现）+ 新增 Tauri 命令（列表/撤销）+ 前端 11 语言 | 审批回调落盘；规则列表页 |

**匹配算法（照 §10.3，已在审计中核实）**：首 token 精确索引（`rules_by_program`）→ 多命中取
`Decision::max()`（`Allow < Prompt < Forbidden`，**最严者胜**，依据 codex `execpolicy/src/decision.rs L7-L16`
的 `Ord` 派生顺序）；未命中由调用侧按「沙箱形态 + 审批策略」裁决。

**危险包装器黑名单**：`sudo` / `env` / `nohup` / `timeout` / `bash -c` 这类前缀，**必须看第二个 token**
才算真实程序，且**默认不给沉淀**（否则一条 `sudo rm` 的批准会沉淀出 `sudo` 规则）。
此概念在本仓未见既有实现（`src-tauri/crates/tools/src/permissions/classifier.rs` 与
`src-tauri/crates/tools/src/bash/security.rs` 我只在子代理报告里见过未复核，开工时先确认）。

**模拟写入后复算**：写入规则前，用 `parse_command` 切段、逐段套用新规则集复算**本次全部子命令段**
的决策；若任一子命令段会变宽松（例如 `a && rm -rf /` 的 `a` 被允许后整条变宽松），**拒绝沉淀**。
这是 codex amendment 里最容易被漏掉的一环。

**验收判据**：
- 同一命令第二次不再询问；
- 多命中取最严（`Allow` 与 `Forbidden` 同命中 ⇒ `Forbidden`）；
- 危险包装器前缀**不**产生规则；
- 模拟复算拦截「沉淀后整条命令变宽松」的用例。

**实施结果与验收取证（2026-09-25，事后复核）**：四块改动面全部落地。

| 块 | 落地物 |
|---|---|
| ① 规则存储 | `src-tauri/crates/entities/src/approval_rules.rs`（`approval_rules` 表：`program` / `args_prefix` / `decision` / `source` / `created_at`，主键由 `program + args_prefix` 派生） |
| ② 能力契约 | `src-tauri/crates/harness/src/approval_rules.rs`（`RuleDecision`：`Allow < Prompt < Forbidden`；`ApprovalRule` DTO；`ApprovalRuleStore` trait 的 `list` / `upsert` / `revoke`） |
| ③ 匹配器 | `src-tauri/crates/kit/src/approval_rules.rs`（`select_decision` 首 token 精确索引 + 参数前缀 + 多命中取最严；`is_rewrite_wrapper` 危险包装器黑名单）；逐段切分与复算在 `src-tauri/crates/tools/src/approval_rules.rs`（`segments` / `evaluate` / `safe_to_sediment`） |
| ④ 接线与 UI | 落盘实现 `src-tauri/src/init/approval_rule_store.rs`（`SeaApprovalRuleStore`，`upsert` 按 `rule_id` 幂等）；启动注入 `src-tauri/src/init/state.rs:640-644`；Tauri 命令 `src-tauri/src/commands/approval_rules.rs`；前端 `src/components/settings/AdvancedSettings.tsx:935-1003`；**11 个语言文件全部含 `approvalRules` 系列 key**（zh-CN / zh-TW / en-US / ja / ko / fr / de / es / ru / ar / hi） |

四条验收判据的代码证据（均指向**真实执行路径**，非仅定义）：

| 判据 | 证据 |
|---|---|
| 同一命令第二次不再询问 | `crates/tools/src/tools/bash.rs:196-202` —— 审批前先 `store.list()` 取规则，`RuleVerdict::AllowAll` 直接执行（**跳过** `ask_user_approval`）；批准后执行命令，**退出码 0** 才由 `bash.rs:249` → `sediment_approved_rule`（`bash.rs:443-467`）best-effort 落盘（§7 O-2 裁定 B，二轮回填见下） |
| 多命中取最严 | `crates/kit/src/approval_rules.rs:95` 的 `select_decision` + 同文件单测（含大小写归一与多命中情形） |
| 危险包装器不产生规则 | `crates/tools/src/approval_rules.rs:260-262` 单测：`sudo rm -rf build` / `bash -c 'rm -rf /'` / `env FOO=1 cmd` 均返回 `None` |
| 模拟复算拦截变宽松 | `safe_to_sediment` 把候选规则并入规则集后**重新评估整条命令**；单测 `safe_to_sediment("a && rm -rf /", &[], …)` ⇒ `None`（`crates/tools/src/approval_rules.rs:254`） |

**与计划原文的一处偏离（记档）**：§3.1 表格原要求「规则表 = 新增实体 + `dao/src/reconcile/extras.rs` 声明」。
实测**无需 extras**：该文件的定位（见其模块头 L1-L21）是「SeaORM 实体语法表达不了的 DDL」的例外清单
（FTS5 虚表 / GIN / 复合索引 / CHECK / 生成列等），而 `approval_rules` 的表、列、主键均能由实体表达，
L1 声明即足够 —— 多写一条 extras 反而会被引擎当作需重建的对象。

**二轮回填（2026-09-25 复核）—— 沉淀时机由「批准即沉淀」改为 §7 O-2 裁定 B 的「批准且执行成功才沉淀」**：
首轮落地的是**选项 A**（`ask_user_approval` 返回 true 后立刻落库，再执行命令），与本计划 §7 裁定表
自相矛盾 —— 裁定写明「批准时只记候选、退出码 0 后才落库」，实现却没有任何执行结果前置。
现改为：AskUser 分支先执行、拿到退出码，再 `sediment_approved_rule(ctx, cmd, &rules, exit_code)`；
`exit_code != 0` 时 `tracing::debug` 后直接返回，不落库。
连带把 `run_direct` / `run_sandboxed` 的返回由 `ToolResult` 改为 `(ToolResult, i32)` —— 直通路径此前
把退出码格式化进文本（`## 退出码: N`）就丢掉了，沉淀判据若去 parse 自有文本会既脆又脏。
取证：`tools::bash::tests::approval_rule_sediment_and_reuse_loop` 新增步骤 3.5，断言
`npm run build` 以退出码 2 沉淀 ⇒ 表内 0 条，随后以 0 沉淀 ⇒ 恰好 1 条。
**与 OnFailure 重试的交互（本轮不改，记档）**：`run_sandboxed` 内的沙箱外重试把重试后的退出码一路上抛，
故「沙箱内被拒 → 批准重试 → 沙箱外成功」这条路径现在**会**沉淀。因规则只免询问、不免沙箱
（`RuleVerdict::AllowAll` 仍在受限子进程内跑），这类命令下次仍会「沙箱内失败 → OnFailure 再问一次」。
沉淀对它是无效的（不是危险的），真要消掉这次询问得让规则携带「曾需沙箱外」的档位信息 —— 属 R2-1 的后续，
不在本轮。
**同步的用户可见文案**：设置页 `settings.approvalRulesHint` 原写「在 Bash 批准后自动记住」，与裁定 B 不符
⇒ 11 种语言全部改为「批准**且成功执行（退出码 0）**后记住」（保序 + LF 落盘，`npx dprint check` 通过）。

### 3.2 R2-2 事件契约枚举化

**目标**：emit / `listen` 双侧由同一枚举驱动，删掉一个事件名后**后端编译失败**（编译期穷举），
而不是靠事后脚本比对。

**依据**：codex 侧 `app-server-protocol/src/protocol/event_mapping.rs L25-L34` 用单一枚举投影，
未知方法返回 `-32601`（`app-server-client/src/remote.rs L363-L405`）；项目现为 emit 字符串 +
`scripts/check-contracts.mjs` 事后校验（审计 §4.4）。

**已探明的现状（子代理定位，**未逐行复核**，开工前必须复核）**：

- `scripts/check-contracts.mjs:13-14` —— 已有校验项 G：「TS 侧 `listen("evt")` 必须有 Rust 侧 `emit("evt")`」；
  ⇒ 这是本项的**存量护栏**，改造时应升级而非推翻。
- `src-tauri/schema-gen/src/main.rs:195` —— 维护 `IPC_DTO_TYPES` 清单，`:287` 输出
  `src/types/generated/ipc.ts`；后者的 `:1-5` 注释说明改 Rust DTO 后需重跑 `schema-gen ipc-types`。
- `src/types/generated/ipc.ts:7-9` —— 该生成文件**当前仅供后端 DTO 同步校验，不在前端运行时直接引用**
  （文件内自陈并附 `oxlint-disable`）。⇒ 让前端 `listen` 消费生成的联合类型，会是这份文件**首次**
  进入前端运行时；若不希望如此，替代方案是生成到独立的新文件（如 `src/types/generated/events.ts`）。
- `src/lib/invoke.ts:5` —— 前端从 `@tauri-apps/api/event` 引入 `listen` 并统一封装。

**改动面（五步，顺序不可换）**：

1. 后端新增事件名枚举（单一来源，带 `as_str()`），放在 harness 或既有事件模块；
2. 让 emit 封装**只接受该枚举**（字符串入口收窄为 `pub(crate)` 或删除）；
3. `schema-gen` 增加「生成事件名联合类型」入口，输出到 `src/types/generated/`;
4. 前端 `listen` 与事件名常量改用生成的联合类型字面量；
5. `scripts/check-contracts.mjs` 的项 G 从「字符串集合比对」升级为「枚举成员集合比对」。

**验收判据**：删掉枚举里一个成员 ⇒ `cargo check` 失败（后端穷举不到）；前端 `npm run typecheck` 对
非法事件名报错；`check-contracts.mjs` 仍绿。

**风险**：第 2 步会一次性触达所有 emit 调用点，是 R2 里最可能产生大 diff 的一项。建议**先做 1+3+4
（并存期），把 2 留到最后**，避免长生命周期的编译红。

**实施订正（2026-09-25，R2-2 落地时，三处对计划原文的偏离）**：

1. **枚举的收录边界不是「后端 emit 全集」，而是「前端 `listen` 全集」**（**79** 个成员 ——
   首稿按 75 收，落地时被前端 typecheck 抓出漏收 5 个，见下方订正 3）。
   实测后端 emit 用到的名字有 100 个，其中 37 个前端**从未监听**（`index-job-*` / `dream-*` /
   `backup` / `webdav` / `stock-monitor-alert` 等）。把后者收进枚举会让「每个成员必须有发射点」
   这条判据对那 37 个天然失守，只得再挂一份白名单 —— 判据于是退化成「维护白名单」，比不收更糟。
   故边界定为：**能被前端 `listen` 的名字必须在枚举里；在枚举里的名字必须有发射引用**
   （6 个历史死链白名单除外，见下）。副作用是改动面从 ~277 处压到 146 处（30 个文件）。
2. **第 2 步「emit 封装只接受该枚举」在本仓无对应物**：全仓没有统一的 emit 封装，
   发射点直接调 Tauri 的 `AppHandle::emit(&str, _)` —— 那是第三方 API，签名不可收窄。
   故本步落为「**契约级事件名在发射点一律写 `IpcEventName::X.as_str()`，不再出现字符串字面量**」，
   并由新项 G2 反向兜住（成员无发射引用即 fail）。
   上游 trait `MonitorEventEmitter::emit(&self, event: &str, _)`
   （`src-tauri/crates/analysis-engine/src/monitor.rs`）**刻意不收窄**：它发的是后端专属名
   `stock-monitor-alert`（不在枚举内），收窄会逼着把这个名字塞进枚举，正好破坏上一条边界。
3. **G1 的正则在「多行泛型 + 字面量」上漏报 —— 本项最值得记录的一次门禁自纠**。
   首稿 `reListen` 的泛型分支写作 `<[^>]*>`，而 `[^>]` 会被载荷类型里的嵌套 `>`
   （如 `Record<string, unknown>`）提前闭合，于是 `listen<{ … Record<string, unknown> … }>("名字", …)`
   整条漏掉。**同一把尺子量了两次**：既定下枚举的收录边界，又自证「全仓 `listen` 全是字面量」——
   于是漏报自洽、无人发现。真正把缺口逼出来的是**另一条独立通路**：前端 typecheck。
   形参收窄为 `IpcEventName` 后，`npm run typecheck` 一次报出 5 处 `TS2345`：

   | 事件名 | 前端监听处 | 后端发射处 |
   |---|---|---|
   | `agent-render-ui` | `src/components/agent/AgentUIRenderer.tsx` | `src/commands/agent/command_bridge.rs`（2 处）+ `src/commands/opc_capability_pack_bridge.rs` + `src/commands/stock_analysis_bridge.rs` |
   | `agent-update-ui` | 同上 | `src/commands/agent/command_bridge.rs`（2 处） |
   | `serenity-screening-completed` | `src/components/stock-analysis/SerenityScreeningPanel.tsx` | `src/commands/stock_workflow/serenity.rs`（2 处） |
   | `workflow-completed` | `src/stores/feature/stockAnalysisStore.ts` + `src/stores/feature/stockWorkflowChatBridge.ts` | `src/commands/stock_workflow/core.rs`（2 处） |

   这 10 处后端发射点**全是字符串字面量**——正因不在枚举里才逃过 codemod（codemod 只替换枚举
   已有的名字）。处置：补 4 个变体进枚举，10 处改枚举投影（其中 2 个文件需新增 import），
   G1 正则的泛型分支改**有界惰性** `[\s\S]{0,800}?`，并在注释里写明这个坑。
   **教训（可复用到任何门禁）**：增量式改造里，「用同一把有缺陷的尺子先定边界、再用它自证边界完整」
   必然自洽；打破自洽只能靠**与尺子无关的第二条通路**。此处若未收窄 `listen` 形参，
   这 5 个名字会永远停留在「前端监听、后端无契约」的静默状态。

**实施结果（2026-09-25）**：

- 新增 `src-tauri/crates/harness/src/event_names.rs`（79 变体，按 8 组分区）：`as_str()` + `ALL` + `Display`；
  `src-tauri/crates/harness/src/lib.rs` 加 `pub mod event_names; pub use event_names::IpcEventName;`。
  模块文档写明收录边界与三条失效模式（删成员 ⇒ 编译失败；非法 `listen` ⇒ typecheck 报错；
  有声明无入边 ⇒ 项 G2 报错）。
- 28 个文件 / 134 处发射点由 codemod 改为 `IpcEventName::X.as_str()`，每文件插入一行
  `use axagent_harness::IpcEventName;`（28 处，无遗漏无重复）。其中 7 处是
  `stock_workflow/core.rs` 的 `let (event_name, payload) = match ...` 元组位置，改造后形态不变、
  仅元组首元素换成枚举投影。**另有本轮补录的 10 处 / 2 个新增 import**（见订正 3），累计 146 处。
- `schema-gen`：新增 `event-names` 入口 → `src/types/generated/events.ts`
  （`IPC_EVENT_NAMES` 数组 + `IpcEventName` 联合类型）。该文件**刻意不写时间戳**（`ipc.ts` 带时间戳是历史遗留），
  以保证「重新生成后内容不变」可作为同步判据；`check` 模式追加生成物同步校验。
- 前端：`src/lib/invoke.ts` 的 `listen` 形参 `event: string` → `event: IpcEventName`；
  `src/types/index.ts` 增 `export type { IpcEventName } from "./generated/events";`（满足禁区 3「类型从 `@/types` 导入」）。
  全仓 `listen` 调用**全是字面量**（无变量传入，含 `await listen(\n "名字", …)` 换行形态），
  故收窄形参本身不引发逐点改造；但**收窄后 typecheck 抓出了 5 个漏收事件名**（见订正 3），
  这是本项真正的收益来源。另有两处前端事件名常量（`DYNAMIC_UI_SCHEMA_UPDATE_EVENT`、`CITE_JUMP_EVENT`）是 `window` 上的
  DOM `CustomEvent`，与 IPC 无关，**不收窄**。
- 清理重复定义（禁区 12）：删除 `src-tauri/crates/harness/src/constants.rs` 的 `event_name` 模块
  （8 个常量全仓零引用，其中 6 个与枚举逐字重复）；`src-tauri/src/commands/cognitive.rs`
  的文件内 `const EVOLUTION_CONSENT_EVENT` 改为 `IpcEventName::EvolutionConsentRequest.as_str()`。
  **后者原先逃过了 codemod**（它只覆盖字面量与 harness 常量路径），正是新项 G2 首跑抓出的唯一缺口。

**新的项 G 判据（四条，都是编译器覆盖不到的边）**：

| 项 | 判据 | 结果 |
|---|---|---|
| G1 | 前端 `listen("X")` 的 X ∈ 枚举成员名 | 首跑 75/75「无未知名」是**假绿**（正则漏报，见订正 3）；修正正则并补 4 个变体后：前端监听 78 个名字，全部 ∈ 79 个成员，无未知名 |
| G2 | 每个枚举成员有 ≥1 处 `IpcEventName::Variant` 引用（排除定义文件本身） | **抓到 1 处真实缺口**（`evolution-consent-request`，文件内常量逃过 codemod）；修后 79/79 |
| G3 | 6 个已知死链白名单自检（已接上必须移出，否则本段对其永久失明） | 通过（6 项确认为零发射） |
| G4 | `src/types/generated/events.ts` 与枚举逐项一致（防生成物腐烂后 typecheck 假绿） | 通过（生成物与枚举逐项一致） |

**验收取证（2026-09-25）**：`node scripts/check-contracts.mjs` 全段 PASS（`errors=0`，G 段摘要
「枚举 79 个成员 / 前端监听 78 个名字 / 已知死链白名单 6 个」）；`npm run typecheck` 干净。
计划原文的验收判据「删掉枚举里一个成员 ⇒ `cargo check` 失败」**已由真实事件代替合成演练**：
本轮编译红恰好两次都是该机制的正向实例 —— ① `as_str` 先写、变体未声明 ⇒ `E0599` ×4；
② 前端 `listen` 收窄 ⇒ `TS2345` ×5。两者都证明「枚举是唯一来源」这层约束真的在拦人，
不再另做「故意删成员」的破坏性演练（需重建整个工作区，收益为零）。

原「字符串集合比对」的 `emitted` / `mentioned` / `unresolved` 三级逻辑**整体删除**：改造后发射点
不再有字符串字面量（G2 反向保证），旧判据会退化成「`emitted` 恒空 ⇒ 全部 `listen` 判 missing」的恒红，
留着只会误导。

### 3.3 R2-3 分量预算按窗口取 + 剩余额度工具

**目标**：`token_budget` 的绝对值随模型窗口伸缩；模型可主动查询剩余额度。

**依据**：codex 侧 `core/src/tools/handlers/get_context_remaining.rs`（**剩余额度是一个工具**）+ 
`core/src/session/context_window.rs L108-L110` 的 `buffered_auto_compact_limit`。

**范围（已按 §0.2 订正）**：

| 动作 | 文件 |
|---|---|
| `token_budget` 绝对值改为「比例 + 上限」双约束（`min(ratio × window, cap)`），现有常量降级为 cap 默认值 ⇒ 行为兼容 | `src-tauri/src/context_manager.rs:24-37` |
| 新增「查询剩余额度」工具 | 待勘察（工具注册机制 + 只读工具样板） |
| 让工具与 `should_auto_compress` **共用同一个供值函数** | `src-tauri/src/context_manager.rs:62-81` |

**⚠ 不要动**：`THRESHOLD_RATIO`（`:19-20`）与 `should_auto_compress` 的阈值逻辑 —— 已正确。

**验收判据**：换 200k 窗口模型时分量预算随动；工具返回的剩余额度与压缩阈值**口径一致**（同一函数
供值，杜绝两处口径漂移 —— 这是 codex 把 `buffered_auto_compact_limit` 单一化的用意）。

**实施结果（2026-09-25）**：

- 新增 `src-tauri/crates/harness/src/context_budget.rs` —— foundation 层的**唯一供值入口**：
  `REFERENCE_WINDOW = 200_000`、`AUTO_COMPACT_THRESHOLD_RATIO = 0.70`、`HISTORY_RATIO = 0.65`
  与五个 `*_CAP`（8000 / 800 / 10000 / 5000 / 2000）。`budgets_for(window)` 按
  `min(cap / REFERENCE_WINDOW × window, cap)` 给出五分量，`ContextBudgets` 暴露
  `fixed_overhead()` / `history_budget()` / `auto_compact_threshold()`。末尾 7 个单测锁定浮点取值
  （200k / 32k / 1M 三档 + 0 窗口 + 单调性）。
- `src-tauri/src/context_manager.rs`：**删除**本地 `THRESHOLD_RATIO`（禁区 12 —— 与 harness 常量重复定义）；
  `should_auto_compress` 与 `build_context_with_query` 均改为调用 `budgets_for(...)`，本文件不再自行乘比例；
  `token_budget` 六个常量改为引用 harness 的 `*_CAP` / `HISTORY_RATIO`（同名同值，外部 6 处消费方**零改动**）。
  末尾 3 个单测：逐 token 锁定压缩翻转点与 `budgets_for` 阈值同源、未知窗口（`None`）永不压缩、
  小窗口历史额度严格更少。
- 新增只读工具 `ContextRemaining`（`src-tauri/crates/tools/src/tools/context.rs`，注册在
  `crates/tools/src/tools/mod.rs` 的「监控和上下文」段）：输出窗口 / 压缩阈值（标注 70%）/ 已用 /
  距阈值剩余 / 五分量预算 / 历史额度公式；无窗口时明写「模型上下文窗口未知…不要假设一个窗口值来做规划」。
- 窗口注入通道：`ToolContext.extra[context_keys::CONTEXT_WINDOW]`，两处 wiring 各注入一次 ——
  `src-tauri/src/commands/agent/mod.rs`（`agent_query` 内，`with_execution_context` 之后）与
  `src-tauri/src/init/agent_turn_adapter.rs`（`WorkflowAgentTurnRunner::run_turn` 内，模型解析之后）。
  取值口径与 `src-tauri/src/commands/conversations/streaming.rs:1419` 的 `model_context_window`
  **逐字一致**（均取模型记录的 `max_tokens`），避免两处各自解析窗口产出不同预算。

**订正 1（不改三处消费点）**：`conversations/streaming.rs`（RAG 注入三段）、
`commands/agent/skill_execution.rs:721,929`（技能索引）、`commands/context_breakdown.rs:108-153`（展示）
**保持 cap 语义不变**。理由：这三处处在「尚不知窗口」的位置 —— regenerate 两处解析窗口在 RAG 注入**之后**；
若为它们引入 `axagent_kit::model_knowledge` 查表，会与 DB `max_tokens` 形成**新的口径漂移**，
与本项「单一供值」的目标相悖。计划原文已写「现有常量降级为 cap 默认值 ⇒ 行为兼容」，cap 正是规范
指定的 fallback，故这三处**本就不该改**。

**订正 2（用量注入缺口，如实记档）**：`ContextRemaining` 的「已用 tokens」目前**拿不到** ——
`axagent-harness` 的 `ToolExecutor` trait（`runtime_types/conversation.rs`）只有 `execute` /
`execute_batch`，无「回灌本轮用量」的钩子；`axagent-tools` 是 hybrid crate，读不到 agent session 状态；
工具注册表的 `extra` 是一次性设定后 move 进执行器的。故该字段当前诚实返回「未知（本轮未注入用量）」，
不编造数值。补齐需要在 harness 侧新开一个供值 trait（本轮不做，避免把 wiring 的实现细节漏进工具层）。

**前后对比数据（订正 3）**：以窗口为自变量，`fixed_overhead()` / `history_budget()` / `auto_compact_threshold()`：

| 窗口 | fixed_overhead | history_budget | 阈值 | 说明 |
|---|---|---|---|---|
| 32 000（旧行为的痛点档） | 25 800 → **4 128** | 4 030 → **18 116** | 22 400 | 旧绝对值常量几乎吃掉整个窗口，历史只剩 4k；改后历史可用额 ×4.5 |
| 200 000（标定基准） | 25 800 → **25 800** | 113 230 → **113 230** | 140 000 | **逐位不变 ⇒ 对 200k 模型行为完全兼容** |
| 1 000 000 | 25 800 | 633 230 | 700 000 | 五分量全部触顶（= cap），历史占比 0.65 |

**订正 4（分量上限与压缩阈值的代数关系，本轮不修）**：`fixed_overhead + history_budget
= 9 030 + 0.65 × window`，压缩阈值 `= 0.70 × window`，两者相交于 `window ≈ 180 600`。
**窗口大于该值时，分量预算之和低于压缩阈值**（200k 档：139 030 < 140 000）—— 即「若历史严格按预算裁剪，
`should_auto_compress` 永不触发」，实际触发依赖「检查用的是**未裁剪**的实际用量」（
`conversations/streaming.rs` 在裁剪前调用）。该关系**非本轮引入**（旧代码把绝对值常量喂进同一个 0.70 阈值，
相交点相同），本轮只是把它算清了。是否重排三者代数（属 R4-1 压缩进事件流的范畴）留待 R4，本轮**记档不修**。

---

## 4. R3：Windows 沙箱 + LLM 审查闸门

### 4.1 R3-1 Windows 沙箱补三件

**现状（已核实）**：

- `src-tauri/crates/tools/src/win_sandbox.rs:211-246` 用
  `SaferCreateLevel(SAFER_LEVELID_NORMALUSER)`（`:216`）+ `SaferComputeTokenFromLevel`（`:233`）；
- `src-tauri/crates/tools/src/win_sandbox.rs:159` 起的 `spawn_sandboxed` 按 mode 分支，
  **只实现了 `ReadOnly`**，`WorkspaceWrite` 在 `:133` 直接返回 Err；
- 子进程启动在 `src-tauri/crates/tools/src/win_sandbox.rs:320`（重建白名单环境变量 + 私有 Desktop
  + `CreateProcessAsUserW`）。

**三件改造（对标审计 §8.1-8.4）**：

| # | 动作 | 依据 |
|---|---|---|
| ① | 令牌换代：SAFER → `CreateRestrictedToken(DISABLE_MAX_PRIVILEGE \| LUA_TOKEN \| WRITE_RESTRICTED)` + capability SID | codex `windows-sandbox-rs/src/token.rs L462-L529`（已逐行复核） |
| ② | `WorkspaceWrite` 用**工作区 allow ACE**（给工作区授予写权限），而非给用户 Profile 加 deny | 审计 §8.2 |
| ③ | 网络封锁：WFP **+** Defender 防火墙**双层**（单 WFP 不够） | 审计 §8.4 |

**必须写进代码注释的两条硬约束**（否则后人会重犯）：

1. `WRITE_RESTRICTED` **只作用于写检查** ⇒ 在该令牌上挂 deny-**read** ACE 是**无效**的。读限制只能
   靠「工作区外不授予读权限」的 allow-only 模型实现 —— 这同时解释了为什么 `src-tauri/crates/tools/src/win_sandbox.rs:36`
   与 `:133` 自述的「用户 Profile 写保护」走不通：**不是档位不够，是 SAFER 与 restricting SID 的机制不同**。
2. `attributes=0`、ACE 的排列顺序（capability → extra-restricting → logon → Everyone）有语义，
   照 codex 的顺序抄，别自作聪明重排。

**门禁特殊性**：Windows 令牌 API 无法在本机跑单测。本项门禁只能是
`cargo clippy -p axagent-tools --all-targets -- -D warnings` + 一份**手工验收清单**
（越界写被拒 / 工作区内写成功 / 网络被断 / 读越界行为符合预期）。见 §7 O-5。

**实施结果 / 验收取证（2026-09-25，全部本机实测）**：

| # | 落地情况 | 对应取证（测试名） |
|---|---|---|
| ① | 已完成：`CreateRestrictedToken(DISABLE_MAX_PRIVILEGE｜LUA_TOKEN｜WRITE_RESTRICTED)` + restricting「capability → Logon → Everyone」；补 `set_default_dacl`（见下第 2 条） | `sandboxed_echo_works` / `sandboxed_can_read_system_dir` / `sandboxed_cannot_write_user_profile` |
| ② | 已完成：`WorkspaceWrite` 走工作区 allow ACE（`grant_workspace_write`），`spawn_sandboxed` 不再对 `WorkspaceWrite` 返回 Err | `workspace_write_inside_allowed`（含子目录 ACE 继承）/ `workspace_write_outside_denied` / `read_only_cannot_write_workspace` |
| ③ | 部分完成：WFP 四层 block 过滤（`ALE_AUTH_CONNECT_V4/V6` + `ALE_RESOURCE_ASSIGNMENT_V4/V6`）按 capability SID 匹配，**两档 SID 各挂一遍**；**Defender 层未做**（理由见下） | `network_block_status_is_truthful`（`Enforced`/`BestEffort` 必须真断网；`Unavailable` 如实放行） |

`win_sandbox::tests` = **8 passed / 0 failed / 1 ignored**（ignored 是既有的环境驱动探针
`sandbox_env_probe`，非本项产物）；`tools::bash::tests::bash_sandboxed_path_end_to_end` 亦绿。

**计划未覆盖、实施中实测定位的四个硬事实（必须留档，否则会重犯）**：

1. **`CreateProcessAsUserW` 返回 `5 ERROR_ACCESS_DENIED` 的根因**：`CreateRestrictedToken`
   派生句柄的**已授予访问权继承自基础令牌句柄**。基础句柄漏 `TOKEN_ASSIGN_PRIMARY(0x0001)`
   ⇒ 新句柄同样没有 ⇒ spawn 访问检查直接失败。与 flags、restricting 列表内容、`lpDesktop`
   **全无关**（8 档探针矩阵全是 5；`safer` 档在同环境下全绿，作阳性对照）。
2. **子进程 `0xC0000142`（`STATUS_DLL_INIT_FAILED`）的根因**：受限令牌的双遍访问检查要求
   **写**在第二遍（只认 restricting 列表 SID 的 allow ACE）也通过，而派生令牌的**默认 DACL
   继承自基础令牌**（只含 `SYSTEM`/`Administrators`/user SID，**不含任何 restricting SID**）
   ⇒ 子进程初始化期对自己新建对象的写被拒。**修法 = 照 codex 补 `set_default_dacl(logon_sid)`**。
   探针实测：flags `0x0/0x4/0x7/0xD` × restricting 列表「空 / 三选组合」**全部 `0xC0000142`**，
   唯一把退出码变成 `0x0` 的变量就是这一步；私有桌面与 `WinSta0\Default` 均 `0x0` ⇒ 与桌面无关。
3. **两档必须用不同的 capability SID**（新增 `CapKind`）：否则 `grant_workspace_write` 打下的
   allow ACE 会**遗留**给之后的 `ReadOnly` 会话（ACE 幂等且设计上不清理），`ReadOnly` 就能写
   工作区 —— `read_only_cannot_write_workspace` 实测复现。codex `cap.rs` 的
   `CapSids { workspace, readonly }` 是同一原因。WFP 过滤器随之改为 8 条 key
   （索引 = 档位 × 4 + 层），一次把两档都封上。
4. **`S-1-15-*` 段进不了 restricting 列表**：`CreateRestrictedToken` 返回
   `ERROR_INVALID_PARAMETER(87)`，手工拼的与 `DeriveCapabilitySidsFromName` 派生的都是 87；
   必须是 `S-1-5-21-*` 形态（codex 同款，也已写进模块文档「原理」1）。

**与本计划原文的两处偏差（如实记录，不静默）**：

- **③ 的 Defender 防火墙层未做**。本机实测（非提权标准用户）：`FwpmEngineOpen0` **成功**，而
  `FwpmSubLayerAdd0`（含退到内建 universal 子层后的）`FwpmFilterAdd0` 均返回
  `ERROR_ACCESS_DENIED(5)` ⇒ **WFP 对象安装需要提权**。按本计划 §8「不建本地账户、不装常驻
  SYSTEM 服务」的排除项，非提权运行时网络**不会**被阻断；处理方式是按 `NetworkBlock` 三档
  如实上报（`Unavailable{reason}` + `warn`），**不假装已断网、也不据此拒绝启动**。
- 本节原「门禁特殊性」判断**已被实测推翻**：9 项集成测试能在本机真实 spawn 子进程并断言
  退出码与文件系统效果，故 §7 O-5「三件各自必须配可实测集成测试」的裁定成立且已执行。

### 4.2 R3-2 guardian 最小形态

**目标**：一个 LLM 审查闸门 —— 对高危动作做二次判定，fail-closed。

**契约（已逐行复核 codex 侧）**：`ext/guardian-reviewer/src/assessment.rs L80-L103` ——
JSON schema `additionalProperties: false`、**`required: ["outcome"]` 只必填 `outcome`**；其余字段
（`risk_level` / `user_authorization` / `rationale`）用 `Option` 承接，与 schema 刻意错开。
解析策略：严格 `serde_json` → 失败则取首个 `{` 到末个 `}` 再试 → 仍失败即 bail。

**改进项**：项目侧**已有**结构化输出能力（`src-tauri/crates/providers/src/structured_output.rs` 与
`src-tauri/crates/providers/src/openai.rs:80-88` 的 `response_format`），优先用它而不是照抄 codex 的
「截取花括号重试」暴力解析。

**fail-closed 矩阵（已逐行复核 codex 侧 `ext/guardian-reviewer/src/completion.rs L111-L143`）**：

| 情形 | 项目应取的决策 |
|---|---|
| 解析失败 | **拒绝**（不重试解析，除上述一次括号截取） |
| 超时 / 重试耗尽 | **拒绝** |
| 输入超预算且非强制审查 | 返回 `None` ⇒ **问用户**（这是**唯一**的「问用户」路径） |
| 无审查者可用 | **绝不隐式放行**（codex 同文件注释：`No contributor is never an implicit allow`） |

**预算**：`max_attempts = 3`、`deadline = 90s`（codex `ext/guardian-reviewer/src/lib.rs L40-L41`，已复核）。

**复用点（子代理定位，未复核 —— 开工前先读实，并遵守 AGENTS.md 禁区 12「禁止重复定义」）**：

- `src-tauri/crates/agent/src/llm_bridge.rs:155-213`（provider 桥接：`ProviderLlmBridge` 在 `:155`、
  `new_with_fallback` 在 `:189`、`with_cache` 在 `:211`）
- `src-tauri/crates/agent/src/retry_policy.rs`（重试封装；同构模块也存在于
  `src-tauri/crates/runtime-core/src/retry_policy.rs` —— **先确认用哪一个，不要再造第三个**）
- **候选近义枚举**：`src-tauri/crates/agent/src/guardrails.rs:40-60` 的
  `GuardrailDecision { Allow, Warn, Block, Halt }`、以及
  `src-tauri/crates/agent/src/self_verifier.rs`、`src-tauri/crates/agent/src/goal_evaluator.rs`、
  `src-tauri/crates/agent/src/pre_validator.rs`、`src-tauri/crates/agent/src/fact_checker.rs`
  —— 这些模块名都暗示已存在「LLM 二次判定」能力。**开工第一件事是判断能否改造复用；
  只有在确认都不可复用后才新建模块。**

**明确不做**：会话复用、异步预评分、证据缓存（审计 §6 已限定）。

**验收判据**：解析失败 → 决策为「拒绝/问用户」而**非**放行（fail-closed 单测）；3 次耗尽后
**不再**调用 provider（用调用计数断言）；无审查者时行为与「拒绝」一致。

#### 实施结果 / 验收取证（2026-09-25）

**复用点复核结论**（开工第一件事，结论与计划原文的差异已记在下方「偏离」）：

| 候选 | 复核结论 | 处置 |
|---|---|---|
| `crates/agent/src/retry_policy.rs` | agent 侧**权威**实现（`with_retry` + `AgentRetryPolicy` + 退避委托 `backon`） | **复用**，不新建第三套 |
| `crates/runtime-core/src/retry_policy.rs` | 只有 4 行，是 `pub use axagent_harness::retry_policy::*` 的**兼容垫片** | 不用（它没有 `with_retry`） |
| `crates/agent/src/llm_bridge.rs` `ProviderLlmBridge::call_llm_structured` | 现成的低温度（0.2）+ 4096 token 结构化调用，已含 fallback 与缓存 | **复用**为审查者适配器 |
| `crates/agent/src/guardrails.rs` `GuardrailDecision{Allow,Warn,Block,Halt}` | 语义是**工具调用循环失败计数**阈值（`Warn`/`Halt` 与审批无关） | **不可复用**（非同一判定域） |
| `self_verifier` / `pre_validator` / `fact_checker` / `goal_evaluator` | 均为「输出正确性 / 事实性 / 目标达成」判定，**不承载审批闸门语义**、也不 fail-closed | **不可复用** |
| `harness::AccessDecision{Allow,Deny,RequireConfirmation}` | 与 codex 的 `allow` / `deny` / `None`(问用户) **一一对应** | **复用为判定结果类型** |

**产物**：新增 `src-tauri/crates/agent/src/guardian.rs`（唯一新文件）+ `crates/agent/src/lib.rs` 一行
`pub mod guardian;`。无第二份契约、无第二套重试。

**落地清单**：`MAX_REVIEW_ATTEMPTS=3` / `REVIEW_TIMEOUT=90s` / 退避基数 `200ms`（= codex
`200ms × 2^(n-1)`）/ `INPUT_TOKEN_MARGIN=256` / `DEFAULT_MAX_INPUT_TOKENS=128_000`；
`guardian_output_schema()`（`additionalProperties:false` + `required:["outcome"]`）与
`guardian_output_contract_prompt()` 相邻放置，**同步性由单测机械锁住**（计划只要求注释约定，
这里做成了可执行断言）；`parse_guardian_assessment` 严格 → 首个 `{` 到末个 `}` 兜底 → 报错；
输入总量闸先行，超限绝不喂模型。

**与计划的偏离（4 处，均为收窄而非放大权限）**：

1. **判定结果复用 harness `AccessDecision`**，未新建 guardian 专用决策枚举 —— 直接满足
   AGENTS.md 禁区 12；`Allow`/`Deny`/`RequireConfirmation` 与 fail-closed 矩阵三档同构。
2. **`risk_level` / `user_authorization` 保留原始串（`Option<String>`）而非枚举** —— audit §9.2 只
   核实了「缺省值」与「由 outcome 反推」，取值全集未逐字核实，**不臆造枚举值**；本模块本就不消费它们。
3. **只做输入总量闸，未做逐字段截断**（codex 的审批理由 512 tokens / 单条转写条目上限等）——
   最小形态下逐字段裁剪属证据裁剪细节，已在模块文档「明确不做」写明。
4. **重试集合偏离**（计划原文说「优先复用 retry_policy」，落地时发现语义差异）：`with_retry` 的重试
   集合由 `ErrorClassifier` 决定（`Transient`/`Unknown` 重试，`Unrecoverable` 立即停），而非 codex 的
   「只重试 Parse / StaleAuthorization / 可恢复 Session」清单。差异方向仍是 fail-closed（不该重试的
   **提前** `Deny`），已在模块文档「复用与偏离」写明。

**验收取证**：

```
cargo test -p axagent-agent --lib guardian
  → running 12 tests … test result: ok. 12 passed; 0 failed; 0 ignored
cargo clippy -p axagent-agent --all-targets -- -D warnings
  → Finished dev profile（0 warning）
```

三条验收判据逐条对应测试：`parse_failure_is_denied_not_allowed`（解析失败 → `Deny`）；
`attempts_exhausted_then_provider_not_called_again`（断言 `calls == MAX_REVIEW_ATTEMPTS` == 3，
不多一次）；`no_reviewer_is_denied`（无审查者 → `Deny`）。另覆盖：超时 `Deny` 且不再发起调用、
`outcome` 双值映射、超预算「非强制 → 问用户 / 强制 → 拒绝」并断言**未调用模型**、schema 与契约文本同步。

⚠ **实施记录（一处实测教训）**：`attempts_exhausted_…` 首跑实测 `calls == 2`（期望 3）。根因是测试
把 `cfg.timeout` 设为 500ms，而该值同时充当「每次退避上限」与「整次审查 deadline」—— 两次退避
（200~400ms + 400~800ms）即可撞上 500ms 外层 `tokio::time::timeout`，把循环掐在第 2 次。生产档位
（90s）下 3 次尝试不可能逼近 deadline，故这**只影响测试构造**；修法是把该测试的 deadline 放宽到 3s
并在注释里写明理由，**未**改生产语义。

⚠ **未做（有意留出）**：本模块**尚无生产调用点**。落地时确认了分层障碍：审批闸门所在
`crates/tools` 是 **hybrid**，不能依赖 `agent`（consumer）；且 `harness::ToolContext` **没有 LLM 句柄**
（只有 `ask_user_bridge` / `approval_rule_store` 等桥接）。因此接线必须落在 wiring 层（`src/init` /
`src/commands`）并新增一个 LLM 桥接字段 —— 这是独立改动面，未包含在「最小形态」内。本节交付的是
**可被 wiring 层直接调用的闸门 + fail-closed 矩阵 + 预算**，与审计 §6 P2「新增 `ext` 之外的轻量审查模块」
的措辞一致。

---

## 5. R4：可回放与传输

### 5.1 R4-1 压缩动作进入可回放事件 + 压缩 prompt 版本化

**现状（部分已核实）**：

- 可回放基础设施**已存在**：`src-tauri/crates/entities/src/session_events.rs:1-28`（`session_id` /
  递增 `seq` / `event_type` / JSON payload，用于跨进程 Resume）；
- 压缩锚点常量在 `src-tauri/src/context_manager.rs:40`（`COMPRESSION_MARKER = "<!-- context-compressed -->"`，已核实）；
- 压缩相关的裁剪逻辑在 `src-tauri/src/context_manager.rs:181`（`build_context`）、
  `:111`（`build_context_with_query`）、`:187`（`sliding_window`）、`:355`（`prune_by_relevance`）
  —— 行号已核实，函数体未逐行读。

**4 个 compact 实现已定位**（我按文件名核实存在，内容未读）：

| 文件 | 备注 |
|---|---|
| `src-tauri/crates/runtime-core/src/compact.rs` | |
| `src-tauri/crates/runtime-core/src/microcompact.rs` | |
| `src-tauri/crates/runtime-core/src/reactive_compact.rs` | |
| `src-tauri/crates/runtime-core/src/session_memory_compact.rs` | |
| （另）`src-tauri/crates/runtime-core/src/snip.rs` | 疑似截断型，需确认是否算第 5 个 |

同目录还有 `src-tauri/crates/runtime-core/src/compact_thresholds.rs` 与
`src-tauri/crates/runtime-core/src/compact_warning.rs`（存在性已核实）。

**改动面**：

| 动作 | 文件 |
|---|---|
| 压缩完成时向 session_events 追加 `event_type = "context_compacted"`，payload 记 summary 文本 + 被覆盖的消息区间 | 4 处 compact 实现的收敛点（**收敛为 1 个后再落事件**，否则要改 4 遍） |
| 压缩 prompt 用 `include_str!` 版本化文件 | 参照审计 §2 的 codex 做法（`prompts/src/compact.rs L1-L2`） |

**验收判据**：重放一条含压缩的会话，能还原「当时模型看到的是摘要」；`session_events` 中压缩事件的
`seq` 单调。

**范围问题见 §7 O-4**：把 4 处收敛为 1 处属范围外，但不收敛就要改 4 遍 —— 建议顺手做。

#### 实施结果 / 验收取证（2026-09-25）

**开工勘察（三处缺口，均已查明）**：

| 缺口 | 结论 |
|---|---|
| 压缩 prompt 落点 | `crates/kit/src/prompts.rs` 的编译期内联 registry（zh-CN 与 en-US 各一份，`_ => ""` 回退），是 **git 跟踪的源码**，非运行时资源 |
| `conversation.rs` 内的压缩判定点 | `maybe_auto_compact`（轮次分支 + 阈值分支）+ `compact()`（手动）；三处现统一收敛到新助手 `apply_compaction` |
| 「4 处 compact」是否 = 4 份重复实现 | **不是**。真重复只有 harness 与 runtime-core 各一份 `compact_session`；`microcompact`（结果去重）/ `snip`（单条 ToolResult 截断）/ `reactive_compact`（触发器包装）/ `session_memory_compact`（结构化记忆策略）**用途不同，不合并** |

**产物 1 —— 真重复那一对收敛为 1 处**：`crates/runtime-core/src/compact.rs` 的 `compact_session`
改为委托 `axagent_harness::compact_session::compact_session`，自身只保留 runtime-core 专属的
PreCompact / PostCompact hook；`format_compact_summary` / `get_compact_continuation_message`
改为 re-export harness 权威实现。该文件共**删除 21 个函数定义**：

- 2 个 pub 函数 → 转为 re-export（`format_compact_summary` / `get_compact_continuation_message`）；
- 3 个 prompt 取词助手（`compact_continuation_preamble` / `compact_recent_messages_note` /
  `compact_direct_resume_instruction`）+ 3 个仅服务上述函数的助手（`extract_tag_block` /
  `strip_tag_block` / `collapse_blank_lines`）；
- 13 个摘要/压缩助手（`summarize_messages` / `merge_compact_summaries` / `summarize_block` /
  `collect_recent_role_summaries` / `infer_pending_work` / `collect_key_files` / `infer_current_work` /
  `has_interesting_extension` / `extract_file_candidates` / `truncate_summary` /
  `extract_existing_compacted_summary` / `extract_summary_highlights` / `extract_summary_timeline`）。

⚠ 这 13 个助手在 runtime-core 侧**只被测试引用**（生产路径已全部在 harness 里用它们），故**不可只删不迁**：
把原本只存在于 runtime-core 的 3 个测试（`truncates_long_blocks_in_summary` /
`extracts_key_files_from_message_content` / `infers_pending_work_from_recent_messages`）**移植**到
`crates/harness/src/compact_session.rs` 末尾新建的 `tests` mod（该文件此前**零测试覆盖**）。

**产物 2 —— 压缩动作进入可回放事件**：

- 事件类型**复用既有** `SessionEventType::Compacted`（`as_str() == "compacted"`），**未**按计划原文
  新建 `"context_compacted"` —— 该枚举全仓**从无生产侧 emit**，正是给压缩预留的槽位；新建同义类型
  违反 AGENTS.md 禁区 12；
- payload 构造器统一在 harness（`build_compacted_event_payload`），两条路径共用，回放侧不必按调用方
  分叉解析；
- 两条生产压缩路径**都**落事件：`crates/agent/src/session_manager.rs`（turn 前压缩，直发）；
  `crates/runtime-core/src/conversation.rs`（阈值/轮次 `maybe_auto_compact` + 手动 `compact()`）
  经**新增**的 `ConversationRuntimeHost::set_session_event_sink`（默认空实现）由 `SessionManager`
  在 `run_turn` 前把自己持有的 sink 透传注入 ⇒ **wiring 层零新增接线点**（`src/init/state.rs` 既有
  注入点不变）。

⚠ **实施记录（一处实测教训，推翻了「逐行等价」的假设）**：收敛后首跑
`cargo test -p axagent-runtime-core --lib compact::tests` 报 `keeps_previous_compacted_context_when_compacting_again`
失败。根因：harness 侧 `extract_existing_compacted_summary` **缺少 runtime-core 版里的空串守卫** ——
prompt 取空串（`NoopPromptProvider`）时分隔符退化为 `"\n\n"` / `"\n"`，把既有摘要截成第一行
`"Summary:"`，二次压缩丢失「Previously compacted context」。已把守卫补进 harness 权威实现，并加回归
测试 `extract_existing_summary_keeps_text_when_prompt_strings_are_empty`。**教训：跨 crate 收敛不能只
肉眼比函数体，必须逐条跑测试** —— 两版差异不在主流程，而在一个「看起来等价」的字符串切分细节上。

**偏离项（3 处，逐条记档）**：

1. **事件类型复用 `Compacted`**，不新建 `context_compacted`（理由见上，禁区 12）。
2. **`coveredRange` 是近似值**：`CompactionResult` 不携带原会话索引，现按「原会话开头 `removed` 条
   被摘要取代」记 `(0, removed)`。精确区间需给 harness `CompactionResult` 增字段（跨 crate 变更），
   本阶段不做；事件仍足以表达「此处发生过压缩、覆盖了多少条」。
3. **payload 记有界预览而非全文**：计划原文说「payload 记 summary 文本」，实作沿用既有
   `SessionEventPayload` 契约的字段名 `summary_preview`，**最多 2000 字符**
   （常量 `COMPACT_SUMMARY_PREVIEW_CHARS`）。若「逐字还原摘要」成为硬需求，把上限放开即可（构造器
   已收敛在一处）。

**明确不做：压缩 prompt 的 `include_str!` 文件化（记档偏离，附理由）**：

计划 §5.1 改动面原写「压缩 prompt 用 `include_str!` 版本化文件」。**不做**，理由三条：

1. 目标**已满足**：三个 key 定义在 `crates/kit/src/prompts.rs` 的 registry —— git 跟踪的源码，历史可
   逐 commit diff、可按版本回溯。`include_str!` 只换载体，不改变「谁在什么版本下看到什么 prompt」。
2. 决定「重放能还原当时模型看到的摘要」的是**事件 payload**（本轮已落：摘要 + 覆盖区间 + 前后 token），
   与 prompt 载体形式解耦 ⇒ 验收判据不依赖它。
3. 迁移**有实证风险而收益为零**：`crates/kit/src/**` **未**被 `dprint.json` 的 excludes 排除，而 dprint
   装了 markdown 插件 ⇒ 若用 `.md` 载体，格式化会改写空白（含结尾空行），而 `continuation_preamble`
   结尾的 `\n\n` **有语义**（分隔 preamble 与摘要）；当前**无任何测试**断言该文本 ⇒ 漂移不会被检出。
   若后续确需文件化（例如为该 prompt 派生缓存键），应与 R4-2 的 `prompt_cache_key` 一并做，并补
   「渲染文本逐字节等价」的测试。

**验收取证（本机实测）**：

```
cargo check -p axagent-harness -p axagent-runtime-core -p axagent-agent --all-targets
  → Finished（0 warning）
cargo fmt -p axagent-harness -p axagent-runtime-core -p axagent-agent
  → 通过
cargo clippy -p axagent-harness -p axagent-runtime-core -p axagent-agent --all-targets -- -D warnings
  → Finished（0 warning）
__TAURI_WORKSPACE__=true cargo test -p axagent-harness --lib compact_session
  → 4 passed; 0 failed
__TAURI_WORKSPACE__=true cargo test -p axagent-runtime-core --lib compact::tests
  → 38 passed; 0 failed
```

**验收判据对照**：①「重放能还原当时模型看到的是摘要」—— `Compacted` 事件带 `summaryPreview` +
`coveredRange` + `tokensBefore/After`，且**确有生产侧 emit**（此前全仓零 emit）；
`compacted_payload_shape_roundtrips` / `compacted_payload_truncates_preview_by_chars_not_bytes`
锁住 payload 形状与「按字符而非字节」截断。②「`seq` 单调」—— `seq` 由实体层 `session_events` 递增
分配，本轮**未改**该机制、注入的仍是既有 `DbSessionEventSink`，故单调性沿用现状（未新增写入端）。

### 5.2 R4-2 `previous_response_id` 增量 + `prompt_cache_key` + head/tail 截断

**现状（grep 全量核实，30 处命中）**：

- `src-tauri/crates/providers/src/openai_responses.rs:75` —— DTO 字段定义（`Option<String>`）；
- `src-tauri/crates/providers/src/openai_responses.rs:471` —— **真正的线上赋值点**
  （`previous_response_id: request.previous_response_id.clone()`）；
- 其余 **28 处调用侧全部硬写 `None`**，分布在
  `src-tauri/crates/agent/src/`（`deep_research.rs` / `ingest_pipeline.rs` / `wiki_compiler.rs` ×3 /
  `vision_pipeline.rs` / `tree_of_thoughts.rs` ×3 / `evaluator/runner.rs` ×2 / `llm_bridge.rs` /
  `self_verifier.rs` / `react_engine.rs` / `research_agent.rs` / `query_engine.rs` /
  `provider_adapter.rs`）、`src-tauri/src/`（`memory_extract.rs` ×4 / `indexing.rs` /
  `init/agent_turn_adapter.rs`）、以及 `benches/` 与 `tests/`。

**⇒ 最小改动面不是改 28 处**，而是在 `:471` 一线接入会话级状态，并且**只改「对话型」调用点**：

| 调用点 | 是否改 | 理由 |
|---|---|---|
| `src-tauri/src/init/agent_turn_adapter.rs:107` | **改** | 主对话 turn，增量收益最大 |
| `src-tauri/crates/agent/src/provider_adapter.rs:486` | **改** | agent 循环 |
| `src-tauri/src/memory_extract.rs` ×4 / `src-tauri/src/indexing.rs` | **不改** | 无状态批处理，传上轮 id 反而错 |
| `wiki_compiler.rs` / `tree_of_thoughts.rs` / `ingest_pipeline.rs` 等 | **不改** | 各自独立的多轮推导，未共享会话 |

**`prompt_cache_key`**：DTO 当前**无此字段** ⇒ 需在请求 DTO（`openai_responses.rs:75` 附近）与上线点
（`:471`）各加一处。键的派生规则待定 —— 见 §7 O-3。
对接既有可观测性：`src-tauri/crates/runtime-core/src/prompt_cache.rs`（缓存断点检测，存在性已核实）
应改为消费新的 key，使「发现没命中」升级为「保证命中」。**（实施时裁定「不接入」，理由见下方
实施结果节的「明确不做」段。）**

**head/tail 截断**：审计 §3.3 指出 codex `core/src/unified_exec/head_tail_buffer.rs L5-L19, L45-L48`
刻意 50% 头 + 50% 尾 + `omitted_bytes` 标记。**项目侧截断函数尚未定位** ⇒ 阶段启动前先勘察
（在 `src-tauri/crates/tools/src/` 与 `src-tauri/crates/harness/src/util_fns.rs` 附近找；
`src-tauri/src/context_manager.rs:16` 已 import `truncate_to_char_boundary`，是线索之一）。

**验收判据**：同一会话连续两轮，第二轮请求带上一轮的 `previous_response_id` 且 items **只含增量**；
截断函数单测覆盖「头尾各半 + 中间 `omitted_bytes` 标记」；批处理型调用点断言仍传 `None`。

**实施结果 / 验收取证（2026-09-25）**：

| 文件 | 动作 | 内容 |
|---|---|---|
| `src-tauri/crates/harness/src/provider_continuation.rs` | **新增** | 会话级续写状态的权威契约：`ContinuationState { response_id, covered_messages }` + `ContinuationStore`（进程级 `global()`）+ `can_continue()`（水位须**非零且严格落后**于当前历史）。模块级 `#![allow(clippy::disallowed_types)]`，理由：状态表同时被同步调用点与异步 provider 访问，临界区内只有 HashMap 读写、绝不跨 await |
| `src-tauri/crates/harness/src/util_fns.rs` | 改 | `truncate_head_tail(s, max)`（头尾各半 + `[omitted_bytes=N]`，多字节边界安全）与 `short_sha256`（sha256 前 8 字节，缓存键用；`DefaultHasher` 不保证跨版本稳定，不可作缓存键） |
| `src-tauri/crates/providers/src/openai_responses.rs` | 改 | ① `ResponsesRequest` 加 `prompt_cache_key`，`build_request` 派生 `"{conversation}:{short_sha256(instructions)}"`（`instructions` = 系统消息 ∪ extra instructions ⇒ 系统提示一变即失效）；② `chat` / `chat_stream` 在调用点未给 id 时回退 `ctx.previous_response_id`（该字段的**首个生产消费点**）；③ `response.completed` 处 `note_response_id` 登记服务端 response id |
| `src-tauri/crates/agent/src/provider_adapter.rs` | 改 | 纯函数 `resolve_continuation(conversation, history_len) -> (Option<String>, usize)`：链可用 ⇒ `(Some(id), 水位)` 且只发增量；失效 ⇒ `(None, 0)` **并清除**陈旧状态；未绑定会话 ⇒ 恒 `(None, 0)`。`stream()` 按返回值裁剪并注入 id；`process_stream` 成功后 `take_response_id` + `record(id, history_len)` 配对落库 |
| `src-tauri/src/commands/agent/mod.rs` | 改 | `build_streaming_api_client` 内 `ctx.conversation = Some(stream_conv_id)`（主对话路径构造 ApiClient 的收口点） |
| `src-tauri/src/init/agent_turn_adapter.rs` | 改 | `ctx.conversation = Some(session_id.clone())` —— 会话 id 在 `resolve_provider` **之后**才拿到，故在其构造 `AxAgentApiClient` 之前补写 |
| `src-tauri/crates/runtime/src/bash.rs` | 改 | `truncate_output` 委托 `truncate_head_tail`（16 KiB 预算不变） |
| `src-tauri/crates/tools/src/tools/bash.rs` | 改 | `truncate_lossy` 委托 `truncate_head_tail`（stdout 500 KB / stderr 250 KB 预算不变） |

**关键设计：两段式状态写入（为何 provider 只登记 id、水位由调用点写）**。`AssistantEvent` 与
`ChatStreamChunk` 都**不透出** response id，而 `ChatStreamChunk` 有 34 处构造点（12 个文件）——
给它加字段属大面积改动，违反「改动尽量小」。故拆成两段：provider 在 `response.completed` 只
`note_response_id`（**它看不到未裁剪的完整历史，算不出水位**），调用点 `AxAgentApiClient` 读水位、
裁增量，成功后 `take_response_id` + `record(id, 历史条数)` 一次配对写入。水位语义 = **不含系统提示**
的本地历史条数；系统提示每轮完整重发（切片只作用于 `request.messages`，不碰 `all_conv_messages`
的 System 前缀，否则会因「重发系统前缀」使水位漂移成死循环）。

**并发假设**：同一会话同一时刻只有一个 turn 在跑（`pending_response_id` 按会话 id 单槽）；
同轮重试会让水位与历史齐平 ⇒ 被 `can_continue` 判为失效并清除，退回整段重发。

**明确不做：接入 `runtime-core/src/prompt_cache.rs`（记档偏离，附理由）**。计划原文要求该模块
「改为消费新 key」。**不做**，理由两条：① **分层倒置** —— `prompt_cache` 挂在 `AppState`，是缓存
**断点检测 / 遥测**结构（生产消费仅 `commands/agent_advanced.rs` 的状态读取与 `cache_guard.rs`），
位于 consumer crate（`runtime-core`）；新的 key 在 provider 侧派生，要让它消费就得把 provider 层
的派生规则上推给 consumer，正好违反 harness 依赖铁律。② **目标已由 ① 达成**：请求侧 `prompt_cache_key`
已上线，「命中」由服务端缓存保证；`prompt_cache.rs` 的职责是「事后发现断点」，与新键**互补而非重复**。
若后续确需联动（例如把命中率回灌到断点面板），应经 harness trait 由 wiring 层桥接，不在此轮。

**验收取证（本机实测）**：

```
cargo check -p axagent-providers -p axagent-agent -p axagent-runtime -p axagent-tools --all-targets
  → Finished（0 error）
cargo check -p axagent --all-targets
  → Finished（0 error）
cargo clippy -p axagent-harness -p axagent-providers -p axagent-agent -p axagent-runtime -p axagent-tools
             --all-targets -- -D warnings
  → Finished（0 warning；本轮顺带修掉 R3-1 遗留的 13 处 win_sandbox clippy：hex 分组 / doc 续行缩进 /
    `field_reassign_with_default` ×2 / `useless_vec`）
cargo clippy -p axagent --all-targets -- -D warnings
  → Finished（0 warning）
__TAURI_WORKSPACE__=true cargo test -p axagent-harness --lib -- util_fns provider_continuation
  → 18 passed; 0 failed
__TAURI_WORKSPACE__=true cargo test -p axagent-providers --lib -- openai_responses
  → 6 passed; 0 failed
__TAURI_WORKSPACE__=true cargo test -p axagent-agent --lib -- provider_adapter
  → 25 passed; 0 failed
__TAURI_WORKSPACE__=true cargo test -p axagent-runtime --lib -- truncation
  → 4 passed; 0 failed
__TAURI_WORKSPACE__=true cargo test -p axagent-tools --lib -- bash
  → 24 passed; 0 failed
```

**验收判据对照**：①「第二轮带上一轮 id 且只含增量」—— `resolve_continuation_reuses_chain_when_watermark_behind_history`
断言 `(Some("resp_1"), 2)`（发第 2 条起的增量），`resolve_continuation_drops_stale_chain_and_clears_it`
锁住「水位齐平 / 超前 ⇒ `(None, 0)` 且状态被清除」；`resolve_continuation_without_conversation_is_always_fresh`
锁住批处理隔离（既有链也不被消费、且不误清他人状态）。②「截断头尾各半 + `omitted_bytes`」——
`head_tail_keeps_both_ends_and_marks_omitted_bytes` / `head_tail_cjk_boundary_safe`（harness）、
`truncate_keeps_both_head_and_tail` / `truncate_cjk_boundary_safe`（tools）、
`long_output_truncated_keeping_both_ends` / `one_over_boundary_truncated`（runtime）覆盖。
③「批处理型调用点仍传 `None`」—— 未绑定会话路径不读也不写状态表，`memory_extract.rs` ×4 /
`indexing.rs` 等 28 处调用点**一行未改**（`ctx.conversation` 仍为 `None`）。
④ `prompt_cache_key` 三方对齐 —— `prompt_cache_key_derives_from_conversation_and_system_prompt`
一条测试同时锁住「同会话同系统提示稳定同键 / 跨会话不撞 / 系统提示变更即失效 / 未绑定会话无键」。

---

## 6. 门禁与验证

**逐阶段完成时**（不要跑全量）：

```
cargo check
cargo clippy -p <受影响 crate> --all-targets -- -D warnings
__TAURI_WORKSPACE__=true cargo test -p axagent --lib <模块过滤>
```

⚠ `--all-targets` 不可省 —— 否则 test / bench target 不被 lint，而新写的测试代码正是高发区。
⚠ 内联 `#[cfg(test)] mod tests` 一律**追加到文件末尾**，插在函数后面会触发
`clippy::items_after_test_module`，该 lint 同样只在 clippy 下暴露。

**全部阶段完成后**（与 CI 逐字一致，只跑一次）：

```
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
__TAURI_WORKSPACE__=true cargo test --workspace
```

⚠ `cargo fmt --check` **不带 `--all` 只检查根 package**，`crates/*` 全不被检查 ⇒ 必须带 `--all`。
⚠ `clippy` **不带 `--all-features`** 只 lint 默认 feature 组合 ⇒ fixture-gated 路径不被检查。

**前端**（每阶段涉及前端时）：

```
npm run typecheck
npm run test:run
npm run format          # dprint
npm run lint:eslint     # 实际是 oxlint；不要用 lint:eslint:legacy
```

**横向约束（每个涉及 DTO / 文本 / 错误码的改动都要过）**：

- **DTO 字段命名**：后端 Rust 字段保持 snake_case + `#[serde(rename_all = "camelCase")]`；
  前端 `src/types/` 侧消费 camelCase。**新增 invoke 调用后跑一次 IPC 参数名一致性扫描**
  （命令参数同样按 camelCase 校验，传 snake_case 会报 `missing required key`）。
- **后端错误码**：新增用户可见错误必须用 `src-tauri/src/commands/error_code.rs` 或
  `crates/harness/src/error_codes.rs` 的常量，并**同步补齐 11 种语言**的 `error.${CODE}` 翻译。
- **新增 UI 文本**：必须补齐 `src/i18n/locales/` 下**全部 11 种语言**。
- **i18n locale 落盘**：保序 + **LF**（这几个文件历史上是 CRLF，出错后排查成本高）。
- **行尾**：所有文本文件 LF；`*.bat` / `*.cmd` / `*.ps1` 例外必须 CRLF。

---

## 7. 开放决策（需要裁定，不要默认推进）

| 编号 | 决策点 | 选项 / 我的倾向 |
|---|---|---|
| **O-1** | R1-3（`decide()` 加 `Attempt`）在 R1-2 落地后是否还有价值 | 倾向：**先不做**。R1-2 已闭合重试回路，`Attempt` 只剩「重试那一次直接走沙箱外」的增量。若不为「用户先拒绝一次再改主意」这类扩展铺路，只改签名不加语义是形式主义 |
| **O-2** | 规则沉淀时机 | 选项 A「批准即沉淀」（codex 语义）；选项 B「批准且执行成功才沉淀」。倾向 **B** —— 不易沉淀出「批准了但命令本来就写错」的垃圾规则 |
| **O-3** | `prompt_cache_key` 的派生规则 | 需三方对齐：单一会话内稳定、跨会话不撞、与系统提示变更同步失效。倾向「会话 id + 系统提示哈希」 |
| **O-4** | R4-1 是否顺带把 4 处 compact 收敛为 1 处 | 倾向 **做**。不收敛就要把「落压缩事件」改 4 遍，且审计已把「并存 4 个实现」列为问题 |
| **O-5** | R3-1（Windows 沙箱）是否纳入本轮 | 本机无法验证令牌 API，门禁只有 clippy + 手工清单。若你手边没有可实测的 Windows 环境，建议**后置到 R3 的末尾或单独一轮** |
| **O-6** | 14 处 `PLAN-codex-parity` 悬空引用怎么处理 | 选项 A 补一份历史计划文档；选项 B 把注释改成可回溯描述（去掉不可解析的文档名）。倾向 **B**（成本低、不留考古负担） |

**裁定结果（2026-09-25，R2 起步时按「最优选择」逐条定稿；O-1 已在 R1 落地时裁定）**：

| 编号 | 裁定 | 依据 / 补充 |
|---|---|---|
| **O-1** | **砍掉**（不做） | 已于 R1 落地时裁定，见 §R1-3「裁定结果」 |
| **O-2** | **B：批准且执行成功才沉淀** | 沉淀的语义是「这条命令**被证明可用**」，不是「用户点过一次同意」。实现方式：批准时只记待沉淀候选（program + args_prefix），命令以退出码 0 结束后才经 harness trait 落库；失败/被拒一律丢弃。代价是多一个「候选」中间态，收益是杜绝 `sudo rm` 式垃圾规则。**已按此落地（2026-09-25 二轮回填，见 §3.1）** —— 未引入「候选」中间态：候选计算（`safe_to_sediment`）本就是纯函数，改为执行后带退出码调用即可，无需先存后取 |
| **O-3** | **会话 id + 系统提示哈希** | 单一会话内稳定（同会话多轮请求命中同一缓存键）、跨会话不撞（id 不同）、系统提示变更即时失效（哈希变）。哈希用仓内既有摘要工具，不新增依赖。**已按此落地**（`harness::util_fns::short_sha256`，R4-2，见 §5.2 实施结果节） |
| **O-4** | **做（收敛 4 处 compact 为 1 处）** | 不收敛就要把「落压缩事件」改 4 遍，且审计已把「并存 4 个实现」列为问题；收敛是 R4-1 的前置，不是可选优化 |
| **O-5** | **纳入本轮，排在 R3 末尾** | 计划原倾向「无实测环境则后置」的前提不成立 —— 本机即 Windows，且 `win_sandbox.rs` 现有集成测试（`sandboxed_cannot_write_system_dir` 等）可真实跑通。故三件各自必须配可实测的集成测试，不允许只靠 clippy 过门禁 |
| **O-6** | **B：改成可回溯描述** | 去掉不可解析的文档名，改为描述性注释（写明「为什么这么写」而非「依据哪份已不存在文档」），不留考古负担。**已按此落地（2026-09-25 二轮回填）**：§0.3 记的 14 处已膨胀到 37 处（R2-1 一族新写代码沿用了旧标签写法，且文档名漏写 `-adoption`），全部处理完毕 —— 23 处 `PLAN-codex-parity P0-x` 与 3 处裸 `（P0-x）` 去掉不可解析的标签（保留原描述性文字，`linux_sandbox.rs` 的「见 PLAN-codex-parity P0-1」改为直接陈述边界），11 处 `PLAN-codex-parity R2-1` 补全为可解析的 `PLAN-codex-parity-adoption R2-1`。清理**只动注释、不动行号**（逐处同行替换），避免 §9 与 `.workbuddy` 记忆层按行号引用成批腐烂。**范围界定**：`rar_router.rs` / `marketplace_scanner.rs` / `scanner_common.rs` 里的 `（P0-n）` 属另案（RAR / 需求扫描）标签，与本计划无关，**未动** |

---

## 8. 不做的事（明确排除）

- **常驻 SYSTEM 服务**（`windows-sandbox-service`）：它是「免 UAC」的代价对冲，换来的常驻特权进程 +
  具名管道五重鉴权 + 企业策略否决面 + 复杂卸载回滚，成本不划算。（审计 §6 已排除）
- **自带 bubblewrap 二进制**：收益是摆脱「依赖用户环境的系统 bwrap」，代价是随包分发 + exec 前
  SHA256 自校验。项目当前 Linux 侧走系统 `unshare`，是否升级取决于是否需要支持无 bwrap 的发行版。
- **guardian 的会话复用 / 异步预评分 / 证据缓存**：审计 §6 已限定为「最小形态」。
- **自研 tree-sitter-bash**：`src-tauri/crates/tools/src/bash/parser.rs:66-73` 的 `parse_command` 已够
  支撑 R2-1 的切段需求；`src-tauri/crates/kit/src/shell_parser.rs:11-15` 的 tree-sitter TODO
  不属本轮。

---

## 9. 引用可信度分级

**已由我逐行 / grep 核实**（可直接作为改造依据）：

`src-tauri/crates/tools/src/approval.rs`、`src-tauri/crates/tools/src/tools/bash.rs`（`:150-235` /
`:425-491`）、`src-tauri/crates/harness/src/approval_policy.rs`、
`src-tauri/src/context_manager.rs:15-100`、`src-tauri/crates/kit/src/command_validator.rs:1-70`、
`src-tauri/crates/kit/src/lib.rs:8`、`src-tauri/crates/tools/src/bash/parser.rs:55-95`、
`src-tauri/crates/harness/src/tool.rs:130`、`src-tauri/src/commands/agent/mod.rs:84`、
`src-tauri/crates/dao/src/migrations/mod.rs:1-45`、`previous_response_id` 全量 30 处命中、
以及 §3.1 列出的两侧三档 / SAFER 现状。

**仅子代理定位 —— 行号我已校正，内容未逐行读**（**开工前必须定点复核，不得直接作为改造依据**）：

`scripts/check-contracts.mjs:13-14`、`src-tauri/schema-gen/src/main.rs:195`、
`src/lib/invoke.ts:5`、`src-tauri/crates/agent/src/llm_bridge.rs:155`（`:189` / `:211` 为其两个构造入口）、
`src-tauri/src/commands/conversations/compress.rs:100`（`:201` 为 `ChatRequest` 构造点，即
`previous_response_id: None` 所在）、`src-tauri/crates/tools/src/win_sandbox.rs:211-246`（`:216` / `:233`）。

**连行号都未校正**（子代理报告后我未做定位）：`src-tauri/crates/providers/src/openai.rs:80-88`、
`src-tauri/crates/agent/src/retry_policy.rs`、`src-tauri/crates/agent/src/guardrails.rs:40-60`、
`src-tauri/crates/runtime-core/src/prompt_cache.rs`、
`src-tauri/crates/tools/src/permissions/classifier.rs`、`src-tauri/crates/tools/src/bash/security.rs`。

**仅确认「文件存在」、内容未读**：§5.1 表格中的 4 处 compact 实现与
`compact_thresholds.rs` / `compact_warning.rs` / `snip.rs`。

> 凡本文标「未复核 / 待勘察 / 推测」处，**不得作为改造依据**；对应的阶段开工第一步即为复核。

---

## 附：与审计文档 §6 的编号映射

| 审计 §6 优先级 | 本文编号 |
|---|---|
| P0（`Attempt` 入参） | R1-3（见 O-1） |
| P0（`is_likely_sandbox_denied`） | R1-1 + **R1-2（订正后新增的收尾项）** |
| P0（档位写进 system prompt） | R1-4 |
| P1（事件契约枚举化） | R2-2 |
| P1（预算 + 剩余额度工具） | R2-3（范围按 §0.2 收窄） |
| P1（批准沉淀为规则） | R2-1 |
| P2（Windows 沙箱三件） | R3-1 |
| P2（guardian 最小形态） | R3-2 |
| P3（压缩进事件流） | R4-1 |
| P3（`previous_response_id` + cache key + head/tail） | R4-2 |
---

## 10. 二轮回填：把「记档不做」的四项收口（2026-09-26）

首轮把四项以「有意留出」记档收尾。本轮按 1 → 3 → 2 → 4 的优先级逐条做实。

### 10.1 R3-2 guardian 接入生产审批路径（首轮：零调用点的死代码 → 现在真的生效）

接线障碍如首轮所料：审批在 hybrid crate `tools`，审查者在 consumer crate `agent`，`tools` 不得依赖
`agent`。收口方式是**只在 harness 落一个 trait**，实现留在 wiring：

| 层 | 落地物 |
|---|---|
| 契约 | `crates/harness/src/tool_access.rs` 新增 `GuardianBridge`（`review(payload, reason) -> AccessDecision`）；`ToolContext.guardian_bridge` 字段；`lib.rs` re-export |
| 透传 | `crates/tools/src/registry.rs` 全局槽 `set_global_guardian_bridge` / `global_guardian_bridge`（与沙箱/审批/规则存储同一模式），构建 `ToolContext` 时注入 |
| 消费 | `crates/tools/src/tools/bash.rs` AskUser 分支：`Allow` ⇒ 免询问执行、`Deny` ⇒ 硬拒且不再打扰用户、`RequireConfirmation` ⇒ 用闸门的理由转人工 |
| 实现 | `src-tauri/src/init/guardian_bridge.rs`（新增）：`AppGuardianBridge` 包 `ProviderLlmBridge`（后者已 `impl GuardianReviewer`），`refresh_global_guardian_bridge` 负责装载/换人/停用 |
| 装配点 | 启动 `init/services.rs::start_background_services` 与 `commands/settings.rs::save_settings` 各调一次刷新 ⇒ 开关保存即生效，无需重启 |
| 开关 | `settings.guardian_review_enabled`，**默认 false**；前端 types / settingsStore / AdvancedSettings 开关 + 11 语言 `advancedSettings.guardianReview{,Hint}` |

三条刻意的设计判断（不是疏漏）：

1. **未注入闸门 = 闸门不启用**，仍走「问用户」。fail-closed 的正确射程是「配了审查者却给不出结论
   ⇒ 拒」，而不是「没配审查者 ⇒ 全世界都拒」—— 后者会让一个未配好的 provider 瘫痪全部 Bash 审批。
   同理：开关开着但 `build_llm_bridge_from_db` 取不到 provider 时，本模块把全局槽置 `None` 并
   `warn`，不装一个「必然拒绝」的桥。
2. **闸门放行不沉淀规则**。沉淀的语义是「这个人批准过这类命令」；没人被问过，就没有可记住的批准。
   故 `bash.rs` 引入 `human_required` 标志，只有真问过用户且拿到批准、且退出码 0，才落规则。
   测试 `guardian_allow_runs_without_asking_and_does_not_sediment` 锁住这条。
3. **不提供「独立审查模型」配置**。审查者 = 首个启用 provider 的首个启用模型（codex 用独立 reviewer
   模型）。要拆开得先有「按用途选 provider」的通路，属独立改动面，与首轮 §4.2「明确不做」同级留出。

验收（`__TAURI_WORKSPACE__=true cargo test -p axagent-tools --lib tools::bash` ⇒ 16 passed / 0 failed）：
`guardian_allow_runs_without_asking_and_does_not_sediment` / `guardian_deny_blocks_without_asking`
/ `guardian_require_confirmation_falls_back_to_user` / `absent_guardian_keeps_asking_user` 四条各锁一档。

### 10.2 R4-1 压缩事件的消费侧

首轮只做到「有生产侧 emit」，读侧未证。复核发现**读侧其实早已存在**：
`commands/agent/mod.rs` 的 `agent_resume_from_events` 会按 `seq` 读全事件流，并已为压缩预留
`"compacted" => "compacted_no_ended"` 判据。真缺口是它被 Tauri `State` 绑死、无法被测试触达，
于是「能读回来」只是一句断言。

收口：把主体抽成可测内核 `resume_report_from_events(db, conversation_id)`，命令退化为薄壳；
新增端到端测试 `init/state.rs::compaction_replay_tests::compacted_event_round_trips_from_sink_into_resume_report`
—— 用真实 `DbSessionEventSink` 写 TurnStarted + Compacted，再从诊断读取原路读回，断言
`seq` 递增、`summaryPreview` 逐字还原、`coveredRange` / `tokensBefore/After` 齐全、
`reason == "compacted_no_ended"`。

**为什么值得单独写一条端到端**：`session_events` 曾长期「表建好了却永远写不进一行」（SQL 插值缺陷
被 `tracing::warn` 吞掉，见 `init/state.rs` 的 `emit` 文档）。只有写入端单测恰好漏得过这类缺陷 ——
它锁得住 payload 形状，锁不住「落库 → 读出」。

仍未做（如实记录）：该命令**没有前端调用方**，即压缩时间线在 UI 上不可见。要不要开这块面板属产品决策，
不在「对齐借鉴项」范围内。

### 10.3 R2-3 订正 2：`ContextRemaining` 报真实已用 tokens

`context_keys::CONTEXT_USED_TOKENS` 这个键首轮就已定义、工具侧也已消费，**只是没人写入** ——
一条悬空契约。本轮补上写入端：`commands/agent/mod.rs` 在注入窗口之后，取本会话最近一条有计量的
assistant 消息的 `prompt_tokens` 注入。

**口径必须说清**（工具的「已用」行与键的文档都写了）：这是**上一轮请求实际发出的上下文占用**，
不是本轮实时值 —— 本轮用量在工具被调用的时刻还不存在。选它而非重新估算，是因为 auto-compact
阈值检查用的同样是「未裁剪的实际用量」（`conversations/streaming.rs`），两边同量纲才谈得上「同源」。

偏离与边界：
- 首轮设想的「harness 新开一个用量回灌 trait」没做 —— 该口径不需要中途回灌，一次读库即得，
  省掉一个跨 crate 契约。
- **工作流 turn（`init/agent_turn_adapter.rs`）不注入用量**：那条路径没有对应的消息读取通路。
  工具在-workflow 下仍如实报「未知」，不编造。

新增 `tools/context.rs` 三条测试（该文件此前零测试）：窗口未知 ⇒ 明说未知不猜；只给窗口 ⇒ 只报预算；
窗口 + 用量 ⇒ 「距阈值剩余」必须由 `budgets_for(window).auto_compact_threshold()` 算出（同源判据）。

### 10.4 R4-1 偏离 2：`coveredRange` 从近似改真实

`CompactionResult` 增 `covered_range: (usize, usize)`。权威实现本来就知道真实边界
（`compacted_prefix_len` = 既有摘要前缀、`keep_from` = tool-use/result 边界回退后的折叠终点），
只是没往外传。两条生产 emit 点（`agent/session_manager.rs`、`runtime-core/conversation.rs`
的 `apply_compaction` 与手动 `compact()`）随之改传真实区间，`conversation.rs` 里那段
「近似值 + 需增字段」的注释删除。

两处如实标注的残余不精确：
- **区间不保证逐条被折叠**：可移除范围 >10 条时启用重要性评分，区间内高分消息会被额外保留，
  故 `end − start >= removed_message_count`。这是「消费了哪一段上下文」的诚实表达；
  当成逐条删除清单就会对不上数。已写进字段文档。
- **会话记忆压缩路径**（`session_memory_compact::to_compaction_result`）保留 `(0, removed)` 近似：
  该策略的保留集本就不是连续后缀，与它摘要文案 "summarize the earlier portion" 同口径。

判据 `runtime-core/compact.rs::covered_range_reports_the_real_folded_window_not_a_zero_based_guess`：
首次压缩 `(0, 2)`，二次压缩必须是 `(1, 3)` —— 旧口径在二次时会把既有摘要前缀算进去而整体错位一格。

### 10.5 本轮门禁（全量，逐条取证）

四条 CI 口径命令 + 前端全套，全部以**独占日志路径**跑（见下方教训）：

| 门禁 | 结果 |
|---|---|
| `cargo fmt --all --check` | `EXIT=0` |
| `cargo clippy --workspace --all-targets --all-features -- -D warnings` | `EXIT=0`，9m40s，日志 3957 字节、`^error`/`^warning` 计数 0（已推进到 `axagent` 主 crate 与 `axagent-mobile`，非中途短路） |
| `__TAURI_WORKSPACE__=true cargo test --workspace` | `EXIT=0`，**150 个 target 全 `test result: ok`**，`test result: FAILED` / `failures:` 零命中 |
| 前端 | `typecheck` 0 · `vitest` 2029 passed · `oxlint --max-warnings=0` 0 · `dprint check` 0 · 行尾检查 PASS · `check_i18n.py` 全部通过 · `check-contracts.mjs` errors=0 |

本轮新增/改写的 10 条测试均在全量测试中逐条 `ok`：`guardian_*` ×4（tools）、
`compacted_event_round_trips_from_sink_into_resume_report`（主 crate）、
`covered_range_reports_the_real_folded_window_not_a_zero_based_guess`（runtime-core）、
`context_remaining_tests` ×3（tools）、`approval_rule_sediment_and_reuse_loop`（含 3.5 步）。

⚠ **本轮两次自纠，都值得进判据**：

1. **共用日志路径 ⇒ 门禁读数会说谎**。先后两次 `cargo clippy` 都重定向到 `/tmp/clippy_full.log`，
   两份输出交叉写入同一文件：日志尾部留着「修 `ColumnTrait` 之前那次」的 `3 previous errors`，
   而 `CLIPPY=0` 来自后一次 —— 同一个文件同时给出「绿」与「红」，任何一方都无法单独被采信。
   复跑改用带时间戳的独占路径 `/tmp/clippy_clean_<ts>.log`，并**读磁盘核验**（字节数 +
   `^error|^warning` 计数 + 是否推进到最后一个 crate），不只看任务回显。
2. **进程探活方式本身会造假绿**：`ps -W | grep -c "cargo|rustc"` 少了 `-E`，把「或」当字面量，
   **恒为 0** ⇒ 我据此判「后台任务已死」，于是叠加启动了 4 个 cargo 任务。后果不是并发失败，而是
   ① 互抢构建锁（单次 clippy 从 10 分钟拖到 19 分钟）② 上面那条日志交叉污染
   ③ CPU 被挤占，把前端 `App.d2` 测试的 10s 超时打爆（隔离重跑 1.2s 即过）。
   判据：**探活命令的匹配模式必须实测其命中面**，`grep -c` 返回 0 既可能是「没有」，也可能是「没匹配上」。
3. **定点门禁的 `-p` 清单要跟着改动面走**。改完第 3、4 项后我先只跑了
   `cargo check -p axagent-harness -p axagent-runtime-core -p axagent-agent -p axagent-tools` 就判绿，
   漏了主 crate —— 而第 3 项那句「读上一轮用量」的查询恰在主 crate，缺 `sea_orm::ColumnTrait` 导入，
   `cargo clippy --workspace` 立刻报 3 个 `E0599`。与 §6 同一条铁律：**门禁命令的覆盖面本身就是门禁的一部分**，
   改到哪个 crate，清单就要覆盖到哪个 crate；按动词抄命令，不按直觉估改动面。
