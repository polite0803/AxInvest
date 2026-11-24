# GAP-P4 — P4 差距分析（部门运行配额 / needs_input 待答）

> 状态：frozen-for-P4 ｜ 日期：2026-11-25 ｜ 基线：`416094880`（P3 收口 + Q1 多层修订）｜ 前置：`GAP.md`、`GAP-P2.md`、`GAP-P2B.md`、`GAP-P3.md`、`PLAN-P4.md`
>
> 方法沿用 P1–P3：A（已有可复用）/ B（需改造或接线）/ C（缺失需新建），逐条给「P4 做 / 不做 + 理由」。
> 来源：`PLAN-P4.md` §0.2 两条遗留项原文（`PLAN-P3.md:136-137`）与 §0.4 只读实测；本轮未重新联网复核 OMB 原文，引用沿用 P1/P2 已落盘抽取。

## 1. 需求与现状映射

| 需求 | 现状证据 | 判定 |
|---|---|---|
| 交互式提问能力 | `axagent-runtime/crates/tools/src/tools/todo_write.rs:113` `AskUserQuestionTool`，默认注册 | **A 复用**，不新造提问工具 |
| 提问桥与超时 | `src-tauri/src/commands/agent/mod.rs:71-171` `AppAskUserBridge::ask_user_blocking`（oneshot + 300s） | **B 接线**：阻塞前落库 |
| 待答可见性 | 仅在内存 map；`agentStore.ts:755-760` 切会话即清 | **C 新建**：持久化请求表 |
| 非交互轮提问 | `bot_routine_runner.rs:269-281` prompter 空表、无桥 | **不做**（D4=A）；表结构预留列 |
| P3 运行预算 | `bot_workflow_runner.rs:31-35`（2/900/12）、`bot_assignment_runner.rs:33-35`（300/每层 2）全为常量 | **B 接线**：配额覆盖层，min 生效 |
| 部门维度策略 | `bot_departments` 无任何配额列/表 | **C 新建**：`bot_department_quotas` |
| 错误码 / i18n / 审计机制 | P3 12 码、485 码基线、`bot-ipc-audit.mjs` P3 段 | **A 复用**机制，新增 5 码 |

## 2. 逐条 A/B/C 判定

| # | 差距 | 现状 | 判定 | P4 做 / 不做 | 理由 |
|---|---|---|---|---|---|
| G-Q1 | 部门运行配额存储 | 无表无列 | **C** | **做**：新表 `bot_department_quotas`，`UNIQUE(department_id)` | 塞 `metadata_json` 失去类型校验/索引/审计；加列让部门记录职责混杂（MODEL-P4 §3） |
| G-Q2 | 配额生效点 | runner 常量直读 | **B** | **做**：run 创建时解析一次 `QuotaLimits`，经 `AssignmentContext` 下发 | 热路径不重复查库（ADR-P4-8） |
| G-Q3 | 配额与默认值关系 | 无覆盖层 | **B** | **做**：`min(定义值, 部门配额)`，**只收紧不放松** | P3 默认是安全基线，配额不能放大 |
| G-Q4 | 部门同时在跑编排数 | 无上限概念 | **C** | **做**：`max_concurrent_runs`（1..8，默认 2） | 编制配额常与总并发一起管；同一表零额外成本 |
| G-Q5 | 配额查看/编辑 UI | 无 | **C** | **做**：部门弹窗新增配额页签，6 字段 + 恢复默认 | 无 UI 的配额等于不可用 |
| G-Q6 | 费用 / Token 配额 | 无 Bot 维度用量账本 | **不做** | **不做**（另立 P5） | 需新增记账链路，8–12 人日，与运行配额正交 |
| G-Q7 | 提问工具 | `AskUserQuestionTool` 已存在且默认注册 | **A** | **复用** | 不新造工具、不改工具名 |
| G-Q8 | 待答持久化 | 仅内存 oneshot + map | **C** | **做**：新表 `bot_input_requests`，四态 | 刷新/切换后必须可见、可答、可过期 |
| G-Q9 | 答复投递 | `agent_respond_ask`（`commands/agent/mod.rs:3078`） | **B** | **做**：签名不变，命中 P4 行时额外转 `answered` | 通用聊天路径零变化，防回归 |
| G-Q10 | 超时与重启 | 300s 超时存在；无清扫 | **B** | **做**：`expires_at` + 启动 `expire_stale` | 防「永远待答」幽灵卡（R3） |
| G-Q11 | needs_input 进状态机 | assignment 五态、run 六态 | **不做** | **不做**（D6=A） | 独立请求表足以表达，改状态机会牵动 P2-b/P3 全部断言 |
| G-Q12 | 无人值守 needs_input | 例程/委派/编排 `prompter=None` | **不做** | **不做**（D4=A）；表留 `origin`/`workflow_run_id`/`assignment_id` 列 | 无人值守无回答者；扩面 +3–4 人日，留作后续裁决 |
| G-Q13 | 待答数 / 提问卡 | 通用 `AskUserCard` 只在活轮 | **C** | **做**：Bot 线程持久卡 + `/bots` 待答数徽章 | 同 `askId` 隐藏通用卡，避免双卡 |
| G-Q14 | 错误码 / i18n / 审计 | P3 机制完备 | **A** | **复用**：新增 5 码，先 locale 后代码 | 不新建错误通道 |
| G-Q15 | 断点续跑 | 无 | **不做** | **不做** | P4-b 只保证可见/可答/可过期，不恢复被中断的模型轮 |

## 3. P4 做 / 不做清单

**做（P4-a 配额）**：部门配额表与 DAO；配额解析器与两 runner 接入；保存定义与触发 run 两处校验；3 命令 + 2 码；部门弹窗配额页签；Rust 测试 + e2e + 台账。

**做（P4-b 待答）**：待答请求表与 DAO；交互桥落库/终态/事件；启动清扫；3 命令 + 改 1 命令 + 3 码 + 1 事件；持久提问卡 + 待答数；Rust 测试 + e2e + 台账。

**不做**：费用/Token 配额（P5）；无人值守 needs_input（D4=A）；assignment/run 新增状态（D6=A）；断点续跑；自动事故线程与重试；Bot 群聊；新调度器/会话池/模型调用路径；P2-c 三件事。

## 4. 复用边界（实现线禁止重造）

- 提问工具：只用 `AskUserQuestionTool`；不复制工具、不改提示词契约。
- 提问桥：只改 `AppAskUserBridge` 的持久化与上下文注入；oneshot 等待机制与 300s 超时语义不动。
- 执行器：配额只在 `bot_workflow_runner` / `bot_assignment_runner` 的常量读取处做覆盖；不新建 runner。
- 状态机：`bot_assignments` 五态、`bot_workflow_runs` 状态零变更。
- 错误/i18n/审计：沿用 `error_code.rs`、11 语言 locale、`bot-ipc-audit.mjs` 既有结构。
