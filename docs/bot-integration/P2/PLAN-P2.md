# PLAN-P2 — P2-a 分期计划（任务卡 T1'–T7'）

> 状态：frozen-for-P2-a ｜ 日期：2026-11-23 ｜ 基线：`0f73ec54b` ｜ 规则沿用 P1 `PLAN.md`：门禁命令见 §6；CI 跑 `--grep-invert "@heavy"`。
>
> 责任人代码：**后端** = Bot 运行时后端工程师；**前端** = Bot 前端工程师；**门禁** = Bot 门禁与验证工程师。

## 0. 依赖图与并行度

```text
T1' CronJob 字段+时区 ──┬─→ T2' Bot 例程执行分支 ──→ T4' 前端例程 UI ──→ T6' e2e 收口
T3' 审批解析(独立)  ────┤        │
                        └─→ T5' IPC 命令层（依赖 T1' 结构 + T3' 校验函数）
T7' 门禁/契约回归（尾随全部，最后跑）
```

- 关键路径：T1' → T5' → T2' → T6'；**T3' 与 T1'/T5' 全程并行**（不同 crate，零文件交集）。
- 前端 T4' 可在 T5' 冻结契约（IPC-P2 §2）后即开工，不等 T2'。
- 估算（人日）：T1' 1.5 ｜ T2' 3 ｜ T3' 2 ｜ T5' 1.5 ｜ T4' 2 ｜ T6' 2 ｜ T7' 1 ｜ 合计 13 人日，三人对齐后约 4–5 个工作日。

## 1. T1' — CronJob 加 botId/timezone + 时区化档期计算

- **目标**：`CronJob` 新增两个 serde-default JSON 字段；`calculate_next_run` 支持 IANA 时区（None=Local 现行为）；`record_run` 透传 timezone。
- **负责人**：后端
- **落点**：改 `axagent-runtime/crates/runtime-core/src/cron_job.rs`（字段 + :323 签名 + :723 `record_run`）；全调用点传 `None`（编译器兜底）；`axagent-runtime/crates/runtime-core/Cargo.toml` 如缺 `chrono-tz` 则加 workspace 依赖。
- **测试**：`runtime-core` 单测新增：① 旧 JSON（无新键）反序列化得 None 且档期与改前一致（升级等价性）；② `Some("Asia/Shanghai")` 下 `0 9 * * *` 产出 UTC+8 的 09:00 对应时间戳；③ `Some("Not/AZone")` 构造报错。
- **验收命令**：`cargo test -p axagent-runtime-core cron`（ext4 内经 WSL）。
- **DoD**：三测绿 + 既有 cron 测试零回归 + 不触碰 `MIGRATIONS`/`schema_compliance`。

## 2. T3' — Bot 审批解析纯函数（可与 T1' 并行起步）

- **目标**：实现 ADR-P2-1/P2-4 的 `resolve_bot_approval_level` + `level_supported`，Bot 读自己目录的文件永不问（借 STOCK 只读口径，不新造白名单）。
- **负责人**：后端
- **落点**：新增 `src-tauri/src/bot_approval.rs`（纯函数 + 两张静态表 + 单测同文件）；`src-tauri/src/commands/agent/mod.rs:2109-2137` 接线（Bot 会话 + 会话档位 `default` 时用 Bot 级别，非 default 仍会话覆盖）。
- **测试**：`bot_approval.rs` 单测：五档映射表全量、custom→Prompt、`level_supported` custom=false、会话覆盖双向、例程路径只取 Bot 档。
- **验收命令**：`cargo test -p axagent --lib bot_approval`（若 bin 名不同以 workspace 实名为准，T7' 统一登记）。
- **DoD**：纯函数无 IO；`resolve_effective_permission` 链路保持只降不升；普通会话路径行为不变（无 botId 的会话走原三档）。

## 3. T5' — IPC 命令层（依赖 T1' 结构；契约 = IPC-P2 §2，冻结）

- **目标**：新增 `create_bot_scheduled_task`/`list_bot_scheduled_tasks`；修改 `create_/list_/update_scheduled_task`（botId/timezone 透传 + `Option<Option<T>>` 清除语义）；新错误码 ×2 入 11 语言；两步注册 + 生成 TS 类型。
- **负责人**：后端（错误码 11 语言可拆给门禁 T7' 协办）
- **落点**：`src-tauri/src/commands/bot.rs`（新增 2 命令）、`src-tauri/src/commands/scheduled_task.rs`（:42 校验区加 `validate_timezone`、:252 DTO 加字段、create/list/update 三命令）、`src-tauri/src/error_code.rs`、`src/i18n/locales/*`（11 语言）、`src-tauri/src/lib.rs`（注册）。
- **测试**：命令层单测（botId 不存在→`BOT_NOT_FOUND`；坏时区→新码；`update` 的 `Some(None)` 清除时区后 `next_run_at` 按 Local 重算）；`check-contracts.mjs` A–H 节零红。
- **验收命令**：`node scripts/check-contracts.mjs && cargo check -p axagent`。
- **DoD**：契约与 IPC-P2 §2 逐字一致；11 语言键齐（i18n 门禁绿）；旧调用（不带新参）行为不变。

## 4. T2' — Bot 例程执行分支（依赖 T1'+T5'，关键路径）

- **目标**：executor handler 增加 `job.bot_id` 分支（ADR-P2-2）：构造 Bot 上下文 → 按 Bot 档非交互跑一轮 → `record_run`/历史/一次性 Disabled。
- **负责人**：后端
- **落点**：`src-tauri/src/init/services.rs`（:2332 handler 内、:4433 兜底前插分支）；复用 `src-tauri/src/bot_context.rs`、`agent_turn_adapter.rs` headless 模式；`src-tauri/src/commands/bot.rs` `delete_bot` 追加「删该 Bot 例程」（命令层 `store.list()` 过滤 botId，GAP G-10）。
- **测试**：新增集成测试（`#[ignore]/@heavy` 标注者除外）：① Bot 例程到点触发并落 history；② Bot 级 `ask` 档下含写工具的轮被自动拒绝且 history.error 可读；③ `delete_bot` 后该 Bot 例程消失、非 Bot 例程不受影响。
- **验收命令**：`cargo test -p axagent --test bot_routine`（新测试文件）+ 手工：创建 1 分钟后到点的 Bot 例程，观察任务中心历史与通知。
- **DoD**：非 Bot 例程行为逐字节不变（跑既有 scheduled_task 回归）；无独立调度器/会话池新增。

## 5. T4' — 前端：Bot 例程 UI + 通知 botId 维度（依赖 T5' 契约冻结）

- **负责人**：前端
- **目标**：① Bot 详情页加「例程」区：列表（复用任务中心 cronAdapter 的行模型）、创建/编辑表单（cron + 时区下拉，预设宿主 Local）、暂停/删除；② `Notification.botId` 写入与 NotificationBell「按 Bot」过滤、BotsPage 未读角标。
- **落点**：`src/pages/BotsPage/`（例程区与角标）、`src/lib/notification.ts`（类型+过滤 helper）、`src/components/layout/NotificationBell.tsx`（过滤）、`src/stores/feature/agentStore.ts`（agent-done 写 botId）、任务中心 `cron.adapter.ts` 带 botId 徽章；i18n 键 `bot.routine.*`、`notification.filterByBot` 入 11 语言。
- **测试**：vitest：notification helper 过滤/角标计数；组件测试例程表单校验（坏 cron/坏时区报错文案走 i18n）。
- **验收命令**：`npm run typecheck && npx vitest run --grep-invert "@heavy"`。
- **DoD**：11 语言齐；旧通知（无 botId）在「全部」视图可见不丢；不新增 IpcEventName。

## 6. T6' — e2e 收口（依赖 T2'+T4'）

- **负责人**：门禁
- **目标**：一条纵向 e2e：建 Bot（edits 档）→ 建例程（Asia/Shanghai，1 分钟后）→ 到点自动跑 → 任务中心见成功历史 → 通知中心出现带 Bot 徽章的未读 → 删 Bot 后例程与角标消失。另加 ask 档负路径（写操作被拒、history.error 落地）。
- **落点**：`e2e/bot-routine.spec.ts`（沿用 P1 e2e 脚手架）。
- **验收命令**：`npx playwright test e2e/bot-routine.spec.ts`。
- **DoD**：正负两路径绿；CI `--grep-invert "@heavy"` 下不拖慢。

## 7. T7' — 门禁/契约/i18n 全量回归（尾随全部）

- **负责人**：门禁
- **目标**：`check-contracts.mjs`、i18n 11 语言键齐、schema compliance 零 diff、错误码 TS 生成核对；登记 T3'/T2' 新测试命令进 `ACCEPTANCE-P2`；核对 GAP-P2 §7 两条未验证项的实测结论补表。
- **落点**：`docs/bot-integration/P2/ACCEPTANCE-P2.md`（新增，登记进 `docs/INDEX.md`）。
- **验收命令**：`node scripts/check-contracts.mjs && npx vitest run --grep-invert "@heavy" && npx playwright test --grep-invert "@heavy"`。
- **DoD**：全绿 + 已知红与 P1 `ACCEPTANCE.md` 清单一致无新增。

## §. P2-b 范围与依赖（一段）

P2-b = **团队/部门**（`teams` 表 + `fleets↔bots` 归属、Chief 角色与权限半径、共享模板白名单）+ **多 Bot 委派运行时**（Assignment 合同：委派消息结构、跨 Bot 线程引用、结果回传与归属、委派深度/环检测）。依赖：P1（Bot 实体/线程归属/SOUL 注入）与 P2-a 的审批解析（委派源的「外部不升权」规则要落在 ADR-P2-1 的函数上）；不依赖 P2-a 的例程与通知。风险集中点：Assignment 运行时不能复用单 Bot 会话的 `agent_query` 简单包装，需要 P2-b 内先出运行时 ADR。

## §. P2-c 清单（一段）

P2-c = ① 每 Bot 技能与预设导入（技能目录按 botId 归属 + OMB preset 导入格式）；② Bot API key（对外调用 AxAgent 的鉴权与速率面）；③ 记忆写入与维护（Bot 对自身 MEMORY 的写入授权、记忆编辑器、变更日志、归档与脱敏）。依赖 P1 的 BotWorkspace 与 P2-a 的审批档（记忆写入属何种档的裁决随批给出），无运行时改造。

## §. 开放问题（≤3，请裁决；建议答案已按此落笔）

| # | 问题 | 建议 |
|---|---|---|
| Q1 | OMB `auto` 映射到 AxAgent 哪档：`WorkspaceWrite`（守工作区边界）还是 `DangerFullAccess`（真放手）？ | **WorkspaceWrite**（ADR-P2-4）：与 OMB「auto 仍守 Bot 边界」更贴，改一行可切 |
| Q2 | 前端未开时的例程通知丢失，P2-a 接受为已知限制？ | **接受**（ADR-P2-3），P2-b 再评估补发；不留假接口 |
| Q3 | Bot 例程命令走「复用 scheduled_task + bot 便捷层」还是全新 bot 域命令族？ | **前者**（IPC-P2 §1）：零新引擎约束下命令面最小，裁决相反只动 §2.1/§2.2 位置 |
