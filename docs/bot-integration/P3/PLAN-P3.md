# PLAN-P3 — P3 分期计划（任务卡 T1'''–T8'''）

> 状态：frozen-for-P3 ｜ 日期：2026-11-23 ｜ 基线：`2e8db65d6` ｜ 体例沿用 `PLAN.md` / `PLAN-P2B.md`：所有命令在 WSL ext4 内执行（`ssh -o BatchMode=yes wsl 'bash -s'`），CI 用 `--grep-invert "@heavy"`。
>
> 责任人代码：**后端** = Bot 运行时后端工程师；**前端** = Bot 前端工程师；**门禁** = Bot 门禁与验证工程师。
>
> Q 列表（§9）以 Q1 用户翻案、Q2..Q8 建议答案冻结；走替代路径时只重排受影响的卡，不换卡片结构。

## 0. 依赖图与并行度

```text
T1''' 两实体 + assignment 加列 + SCHEMA ─┬→ T2''' 步骤校验 + 定义命令 ─┐
                                          ├→ T3''' runner + 完成钩子 ───┼→ T7''' e2e → T8''' 全量门禁
                                          └→ T4''' cron 分支 + 定时命令 ┤
T5''' IPC 审计/事件/TS DTO（契约冻结即可开工）←──────────────────────────┤
T6''' 前端（IPC-P3 §2 冻结后开工，不等 runner）←──────────────────────────┘
```

- 关键路径：**T1''' → T3''' → T7''' → T8'''**（runner 是全链核心）。
- 并行组：T2'''/T3'''/T4''' 均只依赖 T1'''；T5'''/T6''' 在契约冻结后即可开工，不等实现完成。
- 估时（人日，Q1 翻案后重算）：T1''' 2 ｜ T2''' 2 ｜ T3''' 6 ｜ T4''' 1.5 ｜ T5''' 1.5 ｜ T6''' 3 ｜ T7''' 2.5 ｜ T8''' 1 ＝ **19.5 人日**（原 14.5 + 多层委派 +5，落在 ADR-P3-2 反转成本 4–6 人日区间内；区间 18.5–20.5），三人对齐约 **7–8 个工作日**。

## 1. T1''' — 实体/DAO/SCHEMA 登记（MODEL-P3 全量）

- **负责人**：后端
- **落点**：新增 `axagent-core/crates/entities/src/bot_workflows.rs`、`bot_workflow_runs.rs`；`entities/src/lib.rs` 注册；`bot_assignments.rs` 加 `workflow_run_id`/`step_key` 并把头注改为「P3 放开多层 depth≤3，`root_id/parent_id/depth` 为运行期树边」；`root_id`/`parent_id` 加索引；新增 `axagent-infra/crates/dao/src/repo/bot_workflow.rs`、`bot_workflow_run.rs`；`repo/bot_assignment.rs` 加 `list_by_workflow_run` + `create_step_assignment`；`dao/tests/schema_compliance.rs` 清单登记两表 + 两列。
- **测试**：建表幂等（reconcile 重跑零 diff）；`steps_json` round-trip；历史 assignment 两列为 NULL 仍可反序列化；UNIQUE/PK 与索引断言（含 `root_id`/`parent_id`）；链字段 round-trip（depth 1/2/3、root  propagation、parent 链）。
- **验收命令**：`cargo test --manifest-path axagent-infra/Cargo.toml -p axagent-dao --lib bot_ --test schema_compliance`
- **DoD**：SCHEMA 清单零红；正常 Bot/P2-b 单测零回归；无 migration 文件。

## 2. T2''' — 步骤校验 + 定义/查询/删除命令（IPC-P3 §1/§2.1/§2.2/§2.4）

- **负责人**：后端
- **落点**：新增 `src-tauri/src/bot_workflow_steps.rs`（`parse_steps`/`validate_dag`/变量引用解析，纯函数）；`src-tauri/src/commands/bot_workflow.rs`（create/update/list/delete + `BOT_WORKFLOW_*` 错误码 ×7）；`commands/bot_department.rs` 的 `delete_bot_department` 追加转实例归属；`register_commands.rs` 注册。
- **测试**：stepKey 重复/非法、空 steps、dependsOn 未知键、成环、>12 步、非法模板引用 → 对应码；部门编排缺 departmentId → `BOT_WORKFLOW_DEPARTMENT_REQUIRED`；删部门后定义保留且 ownerScope=instance。
- **验收命令**：`cargo test -p axagent --lib bot_workflow && node scripts/check-contracts.mjs`
- **DoD**：定义期校验全覆盖；不写 steps_json 字符串给前端；不改 P2-b 命令。

## 3. T3''' — 编排 runner（复用 P2-b 执行器 + 完成钩子）

- **负责人**：后端
- **落点**：新增 `src-tauri/src/bot_workflow_runner.rs`（拓扑排序、就绪集、并发门、总超时、子树终态等待、失败传播、回执 append、context 更新、run 事件）；**行为不变重构** `bot_assignment_runner.rs`：抽 `run_assignment_inner(..., completion: &dyn Fn(&BotAssignment))`，旧 `run_assignment` 传原有 event sink + 空钩子，既有 `bot_assignment` e2e 零回归；`commands/bot_assignment.rs` 暴露 `create_step_assignment`（复用 `create_inner`，不调用 scheduler）并接受内部链字段；`bot_routine_runner.rs` 的 `build_turn_tool_registry` 改按 depth 放行/禁用 `delegate_to_bot`；`run_bot_non_interactive_turn` 增 task-local `AssignmentContext`；`bot_delegate.rs` 工具 schema 加 `mode`、内部字段由 task-local 注入（详见 §3.1）。
- **测试**：① 三步骤链 A→B→C：依赖顺序正确、B 收到 A 的输出替换、三条回执同线程且 parent=anchor；② 两独立步骤并发 ≤ maxConcurrency；③ 成环定义被 T2 拒绝，运行时按 root 链 visited 集再挡一次；④ `onFailure=abort` 取消在途 + run.failed 继承步骤错误码；⑤ `onFailure=continue` 跳过下游、独立分支完成、skipped 列表正确；⑥ 总超时 → `BOT_WORKFLOW_TIMEOUT`；⑦ 部门成员变更后触发 → `BOT_WORKFLOW_MEMBER_CHANGED`；⑧ cancel → 步骤 assignment 及其整棵子树 cancelled；⑨ 链测试见 §3.1 ⑩–⑰。
- **验收命令**：`cargo test --manifest-path src-tauri/Cargo.toml --test bot_workflow --test bot_assignment --test bot_routine --test bot_delegation_chain`
- **DoD**：无新调度器/会话池/模型调用路径；P2-b `bot_assignment` 4 测试与 `bot_routine` 4 测试仍全绿；run 事件三态齐发。

## 3.1 多层委派子卡（T3''' 内展开，逐条落实 ADR-P3-2 ①–⑤）

| # | 反转清单项 | 实现落点与判据 |
|---|---|---|
| ① | 工具注册按深度放行 | `build_turn_tool_registry(round_kind, depth)`：根/例程轮保留工具；assignee 轮 `depth<3` 出示 `delegate_to_bot`，`depth=3` 不注册；`Agent`/`RemoteTrigger` 恒禁。深度 1 起算（根派发=1），子=parent+1；测试 ⑩：`depth=1/2` 有工具、`depth=3` 无工具（替换 P2-b 旧断言） |
| ② | parent/depth/root 写入 | `create_assignment` 接受内部 `parent_assignment_id`/`root_assignment_id`，子行 `depth=parent+1`、`parent_id=parent.id`、`root_id=parent.root_id`；根仍自链；`workflow_run_id`/`step_key` 沿树继承。测试 ⑪：三层链 root 恒同、parent 逐层指向、`depth=1..3` |
| ③ | visited 链 + 深度门 | 派发前按 `root_id` 查祖先 Bot 集合：命中 → `BOT_DELEGATE_CYCLE`（新增码）；`depth+1>3` → `BOT_DELEGATE_MAX_DEPTH`（新增码）；直接自派仍 `BOT_DELEGATE_SELF`。静态 DAG 环仍用 `BOT_WORKFLOW_CYCLE`；测试 ⑫：A→B→A 拒绝、depth=3 再委派拒绝、两者 error_code 落审计行 |
| ④ | 逐层预算 | `per(root_id, depth)` 在途 ≤2；P2-b 4/8 与 `BOT_DELEGATE_BUSY` 保持；编排 `maxConcurrency` 为独立预算，二者叠加。链总超时 300s 不逐层重置。测试 ⑬：同层第 3 单拒绝、第 9 单全局拒绝、子链超时整链 failed |
| ⑤ | 回执按 root 汇总 | 子孙落同一线程、parent=根 anchor、前缀 `[L{depth} · {stepKey} · {botName}]`；`list_bot_assignments(rootId)` 供前端组链；事件/通知复用；run 等整棵子树终态。测试 ⑭：根 completed 但子在途时 run 仍 running；⑮：`abort`/`cancel` 覆盖子树；⑯：链事件与通知逐条发；⑰：子链 error_code 继承到 run |

**P2-b 继承修订清单（实现线必改，不改历史台账）**：`src-tauri/src/bot_routine_runner.rs:141` 工具表函数签名与 `p2_bot_routine_tests::assignee_round_tool_table_excludes_delegate` 断言（改为 `delegate_tool_respects_depth_cap`）；`PLAN-P2B.md` T3'' 测试⑤/DoD 的「一层不回归」表述由 ADR-P3-2 superseded；`IPC-P2B.md` §4 六码保留、P3 加两码；`src-tauri/tests/bot_assignment.rs` 四测试不动，新增 `src-tauri/tests/bot_delegation_chain.rs`；`e2e/bot-delegation.spec.ts` 两例不动，新增链 e2e；`ACCEPTANCE-P3.md` 登记「P2-b 一层断言自 P3 起 superseded」。完整表见 `ADR-P3.md` ADR-P3-2「继承修订清单」。

## 4. T4''' — cron 触发分支 + 定时命令

- **负责人**：后端
- **落点**：`src-tauri/src/init/services.rs` 在 `if job.bot_id.is_some()` 之前加 `task_type=="bot_workflow"` 分支（从 `job.params["workflowId"]` 取 id，cron 触发时新建持久线程：`department` 编排用 Chief 作 conversation.bot_id，否则 NULL）；`commands/scheduled_task.rs` 加 `create_bot_workflow_schedule` 薄封装 + `BOT_WORKFLOW_TASK_TYPE` 常量；`record_run`/`finish` 复用既有 `TaskRunResult` 路径。
- **测试**：合法/非法 cron 与 timezone；`task_type` 与 `bot_id` 互斥（两者并存以 task_type 为准并 warn）；一次性 run 完成置 Disabled（复用 G-7）；定时触发产生的 run.origin='routine' 且有持久线程；多层补充：cron 触发的步骤根 depth=1，步骤内子链从 2 起算且 run 等整棵子树终态，不因根步骤提前 completed 而写 history 成功。
- **验收命令**：`cargo test -p axagent --lib bot_workflow_schedule && cargo check -p axagent`
- **DoD**：cron 调度器文件零结构性改动（只加分支）；既有 bot_routine 例程行为不变。

## 5. T5''' — IPC 审计/事件枚举/生成物（契约落地）

- **负责人**：后端（门禁复核）
- **落点**：`axagent-core/crates/harness/src/event_names.rs` 加 `BotWorkflowRunUpdated`；`register_commands.rs` 两步注册齐；`scripts/lib/bot-ipc-audit.mjs` 加 P3 段（IPC-P3 §5，含 `list_bot_assignments(rootId)` 与工具 schema 公开字段核对）；TS DTO 生成物核对；12 码写进 `error_code.rs`（编排 10 + 多层 2）；locale 由门禁按 `I18N-KEYS-P3.md` 先落。
- **测试**：`check-contracts.mjs` A–H 零红；错误码 485（以实测为准）↔ 11 语言对齐；事件枚举 `as_str()` = `bot-workflow-run-updated`；工具 schema 不含 `rootId/parentId/depth`。
- **验收命令**：`node scripts/check-contracts.mjs && node scripts/check-errorcode-alignment.mjs && npm run typecheck`
- **DoD**：契约与 `IPC-P3.md` 逐字一致；无孤儿码/无缺失键。

## 6. T6''' — 前端（编排编辑器 + 运行面板 + 定时入口）

- **负责人**：前端
- **落点**：`src/pages/BotsPage.tsx` 增「编排」页签（或并列路由 `src/pages/BotWorkflowsPage.tsx`，二选一由前端按现有路由体例定，须 `npm run check:routes` 绿）；新增 `src/components/bot/BotWorkflowEditor.tsx`（步骤卡片、依赖勾选、Bot 下拉、onFailure、预算）、`src/components/bot/BotWorkflowRunsPanel.tsx`（状态徽章、进度、outputs 展开、取消按钮）、`src/components/bot/BotDelegationChainPanel.tsx`（按 `rootId` 过滤的 L1/L2/L3 链视图，复用 P2-b Assignment 列表组件）；`src/stores/feature/botStore.ts` 增 workflow slice；DTO 类型；i18n 键按 `I18N-KEYS-P3.md`（含 `bot.assignment.depth`/`chain` 2 键）。
- **测试**：vitest：DAG 前端预校验与后端错误码映射、waiting/running 徽章、终态取消按钮禁用、部门筛选含 instance 编排；`origin='workflow'` assignment 类型扩展不破坏 P2-b 列表；多层补充：depth 1/2/3 徽章、`rootId` 过滤链顺序、cycle/maxDepth 错误码映射、父 completed 子在途时 run 仍显示 running。
- **验收命令**：`npm run typecheck && node scripts/vitest-run.mjs run --grep-invert "@heavy"`
- **DoD**：11 语言齐；无编排/无 Bot 空态不丢；不新增通知组件。

## 7. T7''' — e2e 纵向收口

- **负责人**：门禁
- **落点**：`e2e/bot-workflow.spec.ts` + `e2e/bot-delegation-chain.spec.ts`（沿用 P1/P2-b 脚手架，P2-b 原 `bot-delegation.spec.ts` 两例不动）。
- **场景**：建 3 Bot（A/B/C）→ 建部门含 A/B → 建编排：A 汇总（无依赖）→ B 审稿（依赖 A，读 A 输出）→ C 归档（依赖 B）；手工运行 → 列表见 queued→running→completed，单线程出现三条带步骤前缀回执；负路径一：改 B 为非成员后触发 → MEMBER_CHANGED；负路径二：B ask 档写操作 → run.failed + 继承 ERR_PERMISSION，C 被跳过；取消路径：在途 cancel → 步骤 assignment 一并 cancelled；定时路径：建 schedule（cron 下一分钟）→ run.origin=routine + history 落一条。
- **场景（多层）**：A(depth1) 内派 B(depth2)、B 内派 C(depth3) → 三条回执同线程且带 L 前缀、root 相同、run 等 C 终态；C 再委派 → `BOT_DELEGATE_MAX_DEPTH`；B 委派回 A → `BOT_DELEGATE_CYCLE`；同层第 3 单 → `BOT_DELEGATE_BUSY`。
- **验收命令**：`npx playwright test e2e/bot-workflow.spec.ts e2e/bot-delegation-chain.spec.ts --project=chromium --grep-invert "@heavy" --timeout=120000 --reporter=list`
- **DoD**：正负路径绿；CI 不加 @heavy 负担；P2-b 原两例不改仍绿。

## 8. T8''' — 全量门禁与验收台账

- **负责人**：门禁
- **落点**：新增 `docs/bot-integration/P3/ACCEPTANCE-P3.md`（实现后补实测），登记进 `docs/INDEX.md`。
- **验收命令（逐条记 exit 与摘要）**：

```bash
# P2-b 基线回归（15 条台账，错误路径已替换为实测正确路径）
npm run typecheck
node scripts/vitest-run.mjs run
node scripts/check-contracts.mjs
node scripts/check-errorcode-alignment.mjs
node scripts/check-bot-i18n-keys.mjs
npm run check:layers
cargo test --manifest-path axagent-ai/Cargo.toml -p axagent-agent
cargo test --manifest-path axagent-infra/Cargo.toml -p axagent-dao
cargo test --manifest-path axagent-infra/Cargo.toml -p axagent-dao --lib bot_
cargo test --manifest-path axagent-infra/Cargo.toml -p axagent-dao --lib reconcile
cargo test --manifest-path axagent-infra/Cargo.toml -p axagent-dao --lib counts::
cargo test --manifest-path axagent-infra/Cargo.toml -p axagent-dao --test schema_compliance
cargo test --manifest-path src-tauri/Cargo.toml --test bot_assignment --test bot_routine
cargo test --manifest-path axagent-runtime/Cargo.toml -p axagent-tools bot_delegate
npm run check:i18n
npm run check:i18n:key-parity
npm run check:routes
npm run check:serde
npx playwright test e2e/bot-delegation.spec.ts --project=chromium --grep-invert "@heavy" --timeout=120000 --reporter=list
# P3 新增门禁
cargo test --manifest-path src-tauri/Cargo.toml --test bot_workflow --test bot_assignment --test bot_routine --test bot_delegation_chain
npx playwright test e2e/bot-workflow.spec.ts e2e/bot-delegation-chain.spec.ts --project=chromium --grep-invert "@heavy" --timeout=120000 --reporter=list
```

- **DoD**：全绿；`npm run check:serde` 存量红（P1 起 `vector_store.rs`）保持不新增；与 P2-b `ACCEPTANCE` 的已知红清单逐条比对无新增。

## 9. Q 列表（Q1 用户裁决，Q2..Q8 建议答案；替代路径同步给出）

| # | 问题 | 冻结口径（Q1 用户裁决，Q2..Q8 建议答案） | 裁决相反时的替代路径（影响卡） |
|---|---|---|---|
| Q1 | P2-b 的「委派恒一层」是否放开？ | **用户已拍板翻案：放开多层 depth≤3**（ADR-P3-2），五条反转清单全部落 T3'''/T4'''/T6'''/T7'''，+5 人日入计划 | 回退一层案：删工具深度放行与 parent/depth 写入两处改动，两码保留不用（≤1 人日）；P2-b 旧 DoD 恢复生效 |
| Q2 | 编排归属部门还是实例？ | **实例所有 + 部门筛选/成员校验**（ADR-P3-3）；删部门转实例保留 | 部门唯一所有：ownerScope 固定 department + 列表强过滤 + 删部门级联删定义/run（T2'''/T6'''，+1 人日） |
| Q3 | 编排与 P2-a 例程是否合并？ | **不合并**：新 `task_type` 分支并列，零新调度器（ADR-P3-5） | 合并进 bot_routine：扩 CronJob 多 Bot 字段 + 改 P2-a 分支内部（T4'''，+2 人日，回归面大） |
| Q4 | 多 Bot 产物落同一线程还是每步独立线程？ | **同一线程 + 锚点 parent 分组**（ADR-P3-4） | 每步独立线程：run 加 threadIds + 前端跨线程跳转（T3'''/T6'''，+2 人日） |
| Q5 | 步骤引用 Bot 被删/禁用时怎么处理？ | **保留定义，运行失败** `BOT_WORKFLOW_STEP_BOT_MISSING`（MODEL-P3 §5） | 删除时自动从定义移除步骤（改 `delete_bot` + 一条单测；会静默改变编排语义，不建议） |
| Q6 | 步骤失败默认策略？ | **`abort`**，可配 `continue`；run 终态一律 failed（ADR-P3-6） | 默认 continue：改默认常量 + e2e 断言，安全边界变弱，不建议 |
| Q7 | 预算常量（编排并发 2 / 总超时 900s / 12 步 / 链每层 2 / 单步与链总 300s）？ | **按左值起步**，编排常量在 `bot_workflow_runner.rs`、链常量在 `bot_assignment_runner.rs` 头部；两套独立叠加 | 改常量即可，IPC/MODEL 不变；若要按部门配额则需新表，另立 P4 |
| Q8 | OMB `waiting/needs_input` 全量落地？ | **只做 run 级 waiting**（依赖未就绪）；不做 needs_input（无人值守） | 补 needs_input 需要交互入口与超时答应口径（T3'''/T6'''，+3 人日，建议 P4） |

## 10. 分期收口判定

P3 收口需同时满足：① T1'''–T8''' 全绿且提交链完整，多层链 e2e 覆盖 depth 1→2→3 / cycle / maxDepth / per-layer busy / 子树等待 / 整链取消；② 一条 e2e 覆盖顺序/依赖/变量传递/单线程回执/失败跳过/取消/cron 触发；③ P2-b 既有四组回归（`bot_assignment`/`bot_routine`/i18n/契约）零新增红；④ `ACCEPTANCE-P3.md` 记入 INDEX；⑤ P2-b 一层断言已按 ADR-P3-2「继承修订清单」改写，无遗留「一层不回归」表述；⑥ 未改实现代码之外的任何 P2 规格口径（若改动，须先在本文留修订记录）。
