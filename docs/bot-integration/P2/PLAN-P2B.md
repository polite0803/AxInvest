# PLAN-P2B — P2-b 分期计划（任务卡 T1''–T7''）

> 状态：frozen-for-P2-b ｜ 日期：2026-11-23 ｜ 基线：`ee6690c3a` ｜ 规则沿用 P1 `PLAN.md` / P2-a `PLAN-P2.md`：门禁命令见 §7；CI 跑 `--grep-invert "@heavy"`。
>
> 责任人代码：**后端** = Bot 运行时后端工程师；**前端** = Bot 前端工程师；**门禁** = Bot 门禁与验证工程师。
>
> 前提：Q 列表（§8）按建议答案落笔；裁决相反时受影响的卡在卡内标注了替代路径，不影响卡片结构与依赖。

## 0. 依赖图与并行度

```text
T1'' 实体/DAO/SCHEMA 登记 ──┬─→ T3'' 委派执行器（依赖 T1''+T4'' 契约冻结）
T2'' 部门命令层（依赖 T1''）─┤
T4'' IPC 命令层（依赖 T1''）─┘        │
T5'' 前端（契约冻结后即开工）←────────┤
T6'' e2e 收口（依赖 T3''+T5''）        │
T7'' 门禁/契约/i18n 回归（尾随全部）
```

- 关键路径：T1'' → T4'' → T3'' → T6''。
- 并行组：T2'' 与 T4'' 均只依赖 T1''，可同时开工（不同文件）；T5'' 在 T4'' 契约冻结（IPC-P2B §2）后即开工，不等 T3''。
- 估算（人日）：T1'' 1.5 ｜ T2'' 1 ｜ T4'' 1.5 ｜ T3'' 3 ｜ T5'' 2.5 ｜ T6'' 2 ｜ T7'' 1 ｜ 合计 12.5 人日，三人对齐约 4–5 个工作日。

## 1. T1'' — 三实体 + DAO + SCHEMA 登记（MODEL-P2B 全量）

- **目标**：`bot_departments` / `bot_department_members` / `bot_assignments` 三实体落库可读写。
- **负责人**：后端
- **落点**：新增 `axagent-core/crates/entities/src/bot_departments.rs`、`bot_department_members.rs`、`bot_assignments.rs`；`entities/src/lib.rs` 注册（build.rs 自动提取）；新增 `axagent-infra/crates/dao/src/repo/bot_department.rs`、`bot_assignment.rs`（仿 `repo/bot.rs` 风格：实体↔领域类型映射 + list/get/insert/update/delete）；`axagent-infra/crates/dao/tests/schema_compliance.rs` SCHEMA 清单登记三表（MODEL-P2B §4）。
- **测试**：DAO 单测：三表建表幂等（重跑 reconcile 零 diff）；`bot_department_members` UNIQUE 冲突报错；assignment 状态机字段 round-trip。
- **验收命令**：`cargo test -p axagent-dao bot_department bot_assignment schema_compliance`（ext4 内经 WSL）。
- **DoD**：schema_compliance 三表零红；`cargo check` 全 workspace 过。

## 2. T2'' — 部门命令层 + 级联（依赖 T1''；契约 IPC-P2B §1/§2.1）

- **目标**：5 个部门命令 + `delete_bot` 追加级联（清成员行、清 chief、取消在途 assignment——后两项随 T4'' 的 `bot_assignment.rs` 落点一并完成）。
- **负责人**：后端
- **落点**：新增 `src-tauri/src/commands/bot_department.rs`；`src-tauri/src/commands/bot.rs` `delete_bot`（:426）追加扫描；`src-tauri/src/register_commands.rs` 注册；错误码 `BOT_DEPARTMENT_*` ×2 落 `commands/error_code.rs`。
- **测试**：命令层单测：建/改/删部门、add/remove 成员（重复 add → `BOT_DEPARTMENT_MEMBER_EXISTS`）、chief 置空路径（删 Chief Bot 后部门保留 chiefBotId=null）、删除部门后成员 Bot 本体仍在。
- **验收命令**：`cargo test -p axagent --lib bot_department && node scripts/check-contracts.mjs`。
- **DoD**：两步注册齐；11 语言错误码键齐（与 T7'' 分工：译文由门禁补全）；不新增事件。

## 3. T3'' — 委派执行器（依赖 T1''+T4'' 契约冻结；ADR-P2B-2/3/5）

- **目标**：dispatch 后台执行 + consult 工具双路径，复用 P2-a 例程分支三积木。
- **负责人**：后端
- **落点**：新增 `src-tauri/src/bot_assignment_runner.rs`（执行器：并行门常量、`tokio::spawn`、超时 300s、状态机迁移与事件发射）；复用 `src-tauri/src/bot_context.rs`、`bot_approval.rs`、`init/services.rs` 例程分支同款 `agent_query` 等价路径（抽出共享函数，例程分支改为调用同一函数——**行为不变重构**，单测锁旧行为）；新增 tools 层 consult 工具（`axagent-runtime/crates/tools/src/tools/bot_delegate.rs`，runner 注入照抄 `multi_agent.rs` 模式，**只注册给非 assignee 轮**，ADR-P2B-5）。
- **测试**：① dispatch 全链：queued→running→completed，result_text 落回执线程；② assignee ask 档 + 写操作 → failed/`ERR_PERMISSION`，errorDetail 可读；③ 并行门：第 5 单 request → `BOT_DELEGATE_BUSY`；④ 超时 300s 注入短时钟 → failed/`BOT_DELEGATE_TIMEOUT`；⑤ assignee 轮工具表无 `delegate_to_bot`（深度=1 防环）；⑥ cancel：queued/running 可取消、终态报错。
- **验收命令**：`cargo test -p axagent --test bot_assignment && cargo test -p axagent-tools bot_delegate`。
- **DoD**：例程分支行为逐字节不变（既有 `bot_routine` 测试零回归）；无新调度器/会话池；`bot-assignment-updated` 事件三态齐发。

## 4. T4'' — IPC 命令层（依赖 T1''；契约 = IPC-P2B 冻结稿）

- **目标**：`create/list/cancel_bot_assignment` 三命令 + 错误码 ×4 + 事件枚举登记 + `bot-ipc-audit.mjs` P2-b 段。
- **负责人**：后端
- **落点**：新增 `src-tauri/src/commands/bot_assignment.rs`；`commands/error_code.rs`（bot 段追加 4 码）；`axagent-core/crates/harness/src/event_names.rs` 加 `BotAssignmentUpdated`；`src-tauri/src/lib.rs`/`register_commands.rs` 注册；`scripts/lib/bot-ipc-audit.mjs` 扩展；TS 生成物核对。
- **测试**：命令层单测（同 Bot 委派 → `BOT_DELEGATE_SELF`；坏 assignee → `BOT_NOT_FOUND`；`mode=consult` → `ERR_VALIDATION`）；`check-contracts.mjs` A–H 零红。
- **验收命令**：`node scripts/check-contracts.mjs && cargo check -p axagent`。
- **DoD**：契约与 IPC-P2B 逐字一致；事件枚举与生成 TS 类型零 diff。

## 5. T5'' — 前端（依赖 T4'' 契约冻结，可早于 T3'' 开工）

- **负责人**：前端
- **目标**：① 部门管理：BotsPage 侧栏按部门分组展示（保留 section 兜底分组）、部门创建/编辑弹窗（含 Chief 下拉）、成员增删；② 委派：Bot 详情页「委派」区（发起 dispatch 表单 + Assignment 列表 + 状态徽章 + 取消按钮），`bot-assignment-updated` 监听实时刷新；③ 回执线程可见：completed 结果以普通 assistant 消息出现在发起线程（后端已落，前端零改动，仅 e2e 验证）。
- **落点**：`src/pages/BotsPage.tsx`（分组）、新增 `src/components/bot/BotDepartmentModal.tsx`、`src/components/bot/BotAssignmentsPanel.tsx`、`src/stores/feature/botStore.ts`（部门/委派 slice）、`src/types`（DTO）；i18n 键 `bot.department.*`（约 12）、`bot.assignment.*`（约 14）、错误码 ×6，入 11 语言。
- **测试**：vitest：store 过滤/分组投影；组件测试：部门表单校验（空名）、Assignment 状态徽章映射、取消按钮终态禁用。
- **验收命令**：`npm run typecheck && npx vitest run --grep-invert "@heavy"`。
- **DoD**：11 语言齐；无部门 Bot 在「未分组」组可见不丢；与「AI 办公室」（fleet）文案区分明确（ADR-P2B-1）。

## 6. T6'' — e2e 收口（依赖 T3''+T5''）

- **负责人**：门禁
- **目标**：一条纵向 e2e：建两 Bot（A edits 档 / B ask 档）→ 建部门（Chief=A，成员 A+B）→ person 向 A 发起 dispatch 委派 B → 列表见 queued→running→completed → 发起线程出现 B 的回执 → B 负路径：委派写操作 → failed 徽章 + errorDetail → 取消路径：cancel 在途单 → 删 Bot B：在途单 cancelled、部门保留 Chief、B 从成员消失。
- **落点**：`e2e/bot-delegation.spec.ts`（沿用 P1/P2-a e2e 脚手架）。
- **验收命令**：`npx playwright test e2e/bot-delegation.spec.ts`。
- **DoD**：正负两路径绿；CI `--grep-invert "@heavy"` 不拖慢。

## 7. T7'' — 门禁/契约/i18n 全量回归（尾随全部）

- **负责人**：门禁
- **目标**：`check-contracts.mjs`（含新 P2-b 审计段）、i18n 11 语言键齐、错误码 TS 生成核对、schema_compliance 零 diff、事件枚举一致性；登记 T2''–T6'' 新测试命令进 `ACCEPTANCE-P2B.md`（新增文件，登记进 `docs/INDEX.md`）；核对 GAP-P2B §5 与实测差异补表。
- **落点**：`docs/bot-integration/P2/ACCEPTANCE-P2B.md`（新增）。
- **验收命令**：`node scripts/check-contracts.mjs && npx vitest run --grep-invert "@heavy" && npx playwright test --grep-invert "@heavy"`。
- **DoD**：全绿 + 已知红与 P1/P2-a `ACCEPTANCE` 清单一致无新增。

## 8. Q 列表（待用户确认；其余按建议答案执行）

| # | 问题 | 建议答案 | 落笔状态 | 裁决相反时 |
|---|---|---|---|---|
| Q1 | 委派深度：P2-b 只允许一层（assignee 不得再委派），字段先落？ | **是**：一层 + 防环零成本，P3 解禁只改工具注册条件 | 已按此落 ADR-P2B-5 | P3 计划改为树形状态机；P2-b 卡片不变 |
| Q2 | 一个 Bot 同时只属一个部门？ | **是**：UNIQUE(department_id, bot_id) 在 member 表，多对多只需去约束 | 已按此落 MODEL-P2B §2.2 | 去 UNIQUE + 前端多选，无 schema 结构变化 |
| Q3 | consult 回执是否也落回执线程？ | **否**：结果在 requester 轮对话历史自然可见，Assignment 行留审计 | 已按此落 ADR-P2B-4 | 后端 append 一处 + 测试一条 |
| Q4 | 部门删除级联：清成员归属、部门随 Bot 删除而保留（Chief 置空）？ | **是**：组织常设、个体可删，反向级联破坏他人关系 | 已按此落 ADR-P2B-6 | 改 delete 命令分支 + 一条单测 |
| Q5 | 并行/超时常量（单 requester 4 / 全局 8 / 300s）？ | **按左值起步**，常量集中在 runner 头部 | 已按此落 IPC-P2B §3 | 改常量即可，契约不动 |
| Q6 | 例程（cron Bot 轮）内可否发起 dispatch 委派？ | **可**：`origin='routine'`，走同一并行门；这是「Bot 例程统筹多 Bot」的第一块拼图 | 已按此落 MODEL-P2B §3/ADR-P2B-5 | 禁掉则例程轮工具表同样排除 delegate 工具（改动更小） |
