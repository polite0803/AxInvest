# I18N-KEYS-P2B — P2-b 权威 i18n 键名清单（供门禁提前备 locale）

> 状态：active ｜ 日期：2026-11-23 ｜ 基线：`ee6690c3a` ｜ 规则沿用 `I18N-KEYS-P2.md`：zh-CN 为源，11 语言齐更（zh-CN/en/en-GB/es/fr/de/ja/ko/pt-BR/ru/…以现库清单为准）；落盘顺序「先 locale 后代码」防 errorcode-alignment 孤儿码红。

## 1. 后端错误码 ×6（`error` 段，IPC-P2B §4 冻结）

| 键 | zh-CN 源文案 | 备注 |
|---|---|---|
| `BOT_DEPARTMENT_NOT_FOUND` | 未找到 Bot 部门 | T2'' |
| `BOT_DEPARTMENT_MEMBER_EXISTS` | 该 Bot 已归属其他部门 | T2'' |
| `BOT_ASSIGNMENT_NOT_FOUND` | 未找到委派记录 | T4'' |
| `BOT_DELEGATE_SELF` | Bot 不能委派给自己 | T4'' |
| `BOT_DELEGATE_BUSY` | 委派并发已达上限 | T4'' |
| `BOT_DELEGATE_TIMEOUT` | 委派执行超时 | T4'' |

共 **6 码 × 11 语言 = 66 条译文**。

## 2. 前端键（T5''）

### 2.1 `bot.department.*`（12 键）

`title`（部门）、`untitled`（未分组）、`create`（新建部门）、`edit`（编辑部门）、`name`（部门名称）、`nameRequired`（请输入部门名称）、`description`（描述）、`chief`（Chief）、`chiefPlaceholder`（选择 Chief Bot）、`chiefEmpty`（暂无 Chief）、`addMember`（加入部门）、`removeMember`（移出部门）。

### 2.2 `bot.assignment.*`（14 键）

`title`（委派）、`create`（发起委派）、`assignee`（执行 Bot）、`task`（任务内容）、`taskRequired`（请输入任务内容）、`modeDispatch`（派工）、`send`（发起）、`cancel`（取消）、`statusQueued`（排队中）、`statusRunning`（执行中）、`statusCompleted`（已完成）、`statusFailed`（失败）、`statusCancelled`（已取消）、`empty`（暂无委派记录）。

### 2.3 与「AI 办公室」的区分文案（ADR-P2B-1）

`bot.department.hint`：部门是 Bot 的常设组织归属；AI 办公室（fleet）是临时协作场景——1 键 ×11 语言。

**前端合计 27 键 × 11 语言 = 297 条译文；前后端总计 363 条。**

## 3. 落盘顺序（门禁执行口径）

1. 先补 11 语言 locale（错误码 `error` 段 + 上述 UI 键）→ `check-contracts.mjs` C 节自检；
2. 再合错误码常量（T2''/T4''）→ B 节核对码 ⊆ locale；
3. 前端组件（T5''）最后引键 → C 节零多余/缺失。

## 4. DoD

`check-contracts.mjs` B/C 节零红；11 语言 locale diff 中 P2-b 键一一对应（无孤儿键、无缺失键）。
