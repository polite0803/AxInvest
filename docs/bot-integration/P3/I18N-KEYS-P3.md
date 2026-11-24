# I18N-KEYS-P3 — P3 权威 i18n 键名清单（供门禁提前备 locale）

> 状态：frozen-for-P3 ｜ 日期：2026-11-23 ｜ 基线：`2e8db65d6` ｜ 规则沿用 `I18N-KEYS-P2/P2B`：zh-CN 为源文案，11 语言齐更；落盘顺序「先 locale 后代码」防 errorcode-alignment 孤儿码红。
>
> **11 语言以现库实测为准**：`src/i18n/locales/` 下 `ar.json`、`de.json`、`en-US.json`、`es.json`、`fr.json`、`hi.json`、`ja.json`、`ko.json`、`ru.json`、`zh-CN.json`、`zh-TW.json`。P2 文档里举例的 `en`/`en-GB`/`pt-BR` 不是本仓库实际文件名，P3 不沿用。

## 1. 后端错误码 ×12（`error` 段，IPC-P3 §4 冻结）

| 键 | zh-CN 源文案 | 卡 |
|---|---|---|
| `BOT_WORKFLOW_NOT_FOUND` | 未找到 Bot 编排 | T2''' |
| `BOT_WORKFLOW_STEP_INVALID` | 编排步骤定义不合法 | T2''' |
| `BOT_WORKFLOW_CYCLE` | 编排步骤存在循环依赖 | T2''' |
| `BOT_WORKFLOW_DEPARTMENT_REQUIRED` | 部门编排必须指定部门 | T2''' |
| `BOT_WORKFLOW_MEMBER_CHANGED` | 编排步骤 Bot 已不属于该部门 | T3''' |
| `BOT_WORKFLOW_RUN_NOT_FOUND` | 未找到编排运行记录 | T2''' |
| `BOT_WORKFLOW_RUN_BUSY` | 编排存在未结束的运行 | T2''' |
| `BOT_WORKFLOW_STEP_BOT_MISSING` | 编排步骤引用的 Bot 不存在或已禁用 | T3''' |
| `BOT_WORKFLOW_TIMEOUT` | 编排运行超时 | T3''' |
| `BOT_WORKFLOW_CANCEL_INVALID` | 编排运行已结束，无法取消 | T2''' |
| `BOT_DELEGATE_CYCLE` | 委派链出现循环 | T3''' |
| `BOT_DELEGATE_MAX_DEPTH` | 委派链深度已达上限 | T3''' |

12 码 × 11 语言 = **132 条译文**（编排 10 + 多层委派 2）；P2-b 收口基线 473 码，P3 后期望 **485 码**（以脚本实测输出为准）。

## 2. 前端键（T6'''）

### 2.1 `bot.workflow.*`（28 键，定义编辑器）

`title`（编排）、`new`（新建编排）、`edit`（编辑编排）、`empty`（暂无编排）、`emptyDescription`（创建编排后可让多个 Bot 按顺序协作）、`createFirst`（创建第一个编排）、`name`（编排名）、`nameRequired`（请输入编排名）、`description`（描述）、`ownerScope`（归属范围）、`ownerInstance`（实例级）、`ownerDepartment`（部门级）、`department`（部门）、`departmentPlaceholder`（选择部门）、`steps`（步骤）、`addStep`（添加步骤）、`removeStep`（移除步骤）、`stepKey`（步骤标识）、`assignee`（执行 Bot）、`prompt`（任务提示词）、`dependsOn`（依赖步骤）、`timeoutSecs`（单步超时（秒））、`onFailure`（失败策略）、`onFailureAbort`（中止后续步骤）、`onFailureContinue`（跳过下游继续）、`maxConcurrency`（并发上限）、`totalTimeoutSecs`（总超时（秒））、`save`（保存编排）。

### 2.2 `bot.workflowRun.*`（14 键，运行面板）

`title`（运行记录）、`run`（运行）、`empty`（暂无运行记录）、`statusQueued`（排队中）、`statusRunning`（执行中）、`statusWaiting`（等待依赖）、`statusCompleted`（已完成）、`statusFailed`（失败）、`statusCancelled`（已取消）、`progress`（进度）、`outputs`（步骤输出）、`skipped`（已跳过）、`cancel`（取消运行）、`viewThread`（查看回执线程）。

### 2.3 `bot.workflowSchedule.*`（5 键，定时入口）

`title`（定时触发）、`create`（创建定时编排）、`cron`（Cron 表达式）、`timezone`（时区）、`hint`（编排定时与单 Bot 例程互不冲突）。

### 2.4 `bot.assignment.*` 补键（多层委派链，T6'''）

`depth`（层级）、`chain`（委派链）——2 键，用于链视图的 L1/L2/L3 徽章与按 rootId 分组标题；既有 P2-b `bot.assignment.*` 14 键不动。

**前端合计 49 键 × 11 语言 = 539 条译文；前后端总计 671 条。**

## 3. 落盘顺序（门禁执行口径）

1. 先补 11 语言 locale：`error` 段 12 码 + `bot.workflow*`/`bot.workflowRun*`/`bot.workflowSchedule*` 47 键 + `bot.assignment.depth`/`bot.assignment.chain` 2 键；
2. 再把 12 码常量写进 `src-tauri/src/commands/error_code.rs`（`check-errorcode-alignment.mjs` B 节：码 ⊆ locale 且无孤儿）；
3. 把 49 个 UI 键加入 `scripts/check-bot-i18n-keys.mjs` 的 `BOT_I18N_KEYS` 必需集合（防 11 语言一起漏掉仍假绿；P1/P2 既有键保持不动，预计从 102 增至 151）；
4. 前端组件（T6'''）最后引键 → `check:i18n` / `check:i18n:key-parity` 零缺失、零多余。

## 4. 文案约束

- `bot.workflow.ownerScope` 的「实例级/部门级」必须与 ADR-P3-3 一致：部门级只是筛选与成员校验，不是资产所有权；帮助文案不得暗示「删部门会删编排」。
- `bot.workflowRun.statusWaiting` 译为「等待依赖」，不得译成「等待用户输入」（P3 无 needs_input，ADR-P3-6）。
- 步骤回执前缀由后端拼 `[步骤 {key} · {botName}] `，前端不另加方括号，避免双前缀。
- 多层链回执前缀 `[L{depth} · {stepKey} · {botName}]` 由后端拼装，前端只渲染 `bot.assignment.depth` 徽章，不重复拼 L 前缀。
- `BOT_DELEGATE_CYCLE` / `BOT_DELEGATE_MAX_DEPTH` 是链门错误码，不得译成编排 DAG 的「步骤循环」文案（那是 `BOT_WORKFLOW_CYCLE`）。

## 5. DoD

`node scripts/check-errorcode-alignment.mjs`、`node scripts/check-bot-i18n-keys.mjs`、`npm run check:i18n`、`npm run check:i18n:key-parity` 四项零红；11 语言 diff 中 P3 键一一对应，无孤儿键、无缺失键、无空值。
