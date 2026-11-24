# I18N-KEYS-P4 — P4 权威 i18n 键名清单（供门禁提前备 locale）

> 状态：frozen-for-P4 ｜ 日期：2026-11-25 ｜ 基线：`416094880` ｜ 规则沿用 `I18N-KEYS-P2/P2B/P3`：zh-CN 为源文案，11 语言齐更；落盘顺序「先 locale 后代码」防 errorcode-alignment 孤儿码红。
>
> **11 语言以现库实测为准**：`src/i18n/locales/` 下 `ar.json`、`de.json`、`en-US.json`、`es.json`、`fr.json`、`hi.json`、`ja.json`、`ko.json`、`ru.json`、`zh-CN.json`、`zh-TW.json`。

## 1. 后端错误码 ×5（`error` 段，IPC-P4 §4 冻结）

| 键 | zh-CN 源文案 | 卡 |
|---|---|---|
| `BOT_QUOTA_INVALID` | Bot 部门配额取值不合法 | A3 |
| `BOT_QUOTA_EXCEEDED` | 超出部门运行配额 | A2/A3 |
| `BOT_INPUT_REQUEST_NOT_FOUND` | 未找到待答请求 | B3 |
| `BOT_INPUT_REQUEST_NOT_PENDING` | 待答请求已回答或已失效 | B3 |
| `BOT_INPUT_ANSWER_EMPTY` | 答复内容不能为空 | B3 |

5 码 × 11 语言 = **55 条译文**；P3 收口基线 485 码，P4 后期望 **490 码**（以脚本实测输出为准）。

## 2. 前端键（A4 / B4）

### 2.1 `bot.quota.*`（20 键，部门配额面板）

| 键 | zh-CN 源文案 | 用途 |
|---|---|---|
| `title` | 运行配额 | 面板标题 |
| `subtitle` | 配额只会收紧默认限制，不会放宽 | 副标题（强制“只收紧”语义） |
| `inheritHint` | 未设置时使用默认限制 | 空态说明 |
| `maxConcurrency` | 单编排并发上限 | 字段标签 |
| `maxConcurrencyHint` | 默认 2，可设 1–4 | 字段提示 |
| `totalTimeout` | 单编排总超时（秒） | 字段标签 |
| `totalTimeoutHint` | 默认 900，可设 60–3600 | 字段提示 |
| `maxSteps` | 单编排步数上限 | 字段标签 |
| `maxStepsHint` | 默认 12，最多 12 | 字段提示 |
| `perDepth` | 委派链每层在途上限 | 字段标签 |
| `perDepthHint` | 默认 2，可设 1–4 | 字段提示 |
| `stepTimeout` | 单步超时（秒） | 字段标签 |
| `stepTimeoutHint` | 默认 300，可设 30–900 | 字段提示 |
| `maxRuns` | 部门同时运行编排数 | 字段标签 |
| `maxRunsHint` | 默认 2，可设 1–8 | 字段提示 |
| `save` | 保存配额 | 主按钮 |
| `reset` | 恢复默认 | 次按钮 |
| `resetConfirm` | 恢复默认会删除该部门的配额设置，确定继续？ | 二次确认 |
| `invalidRange` | 取值超出允许范围 | 前端预校验 |
| `saved` | 配额已保存 | 成功提示 |

### 2.2 `bot.input.*`（14 键，持久提问卡与待答数）

| 键 | zh-CN 源文案 | 用途 |
|---|---|---|
| `pendingTitle` | 等待你的回答 | 卡片标题 |
| `pendingBadge` | 待答 | 侧栏徽章/计数 |
| `question` | 问题 | 问题标签 |
| `placeholder` | 输入你的回答 | 输入框占位 |
| `submit` | 提交回答 | 主按钮 |
| `cancel` | 取消 | 次按钮 |
| `cancelConfirm` | 取消后该问题将不再等待回答，确定继续？ | 二次确认 |
| `expiresIn` | 剩余 {time} | 倒计时（`{time}` 由前端格式化） |
| `expired` | 该问题已过期 | 过期态标题 |
| `expiredHint` | 应用重启或超时后不可再作答 | 过期态说明（强制语义：不可恢复） |
| `answered` | 已回答 | 终态 |
| `cancelled` | 已取消 | 终态 |
| `answerFailed` | 提交失败，请重试 | 错误提示 |
| `empty` | 暂无待答问题 | 空态 |

**前端合计 34 键 × 11 语言 = 374 条译文；前后端总计 429 条。**

## 3. 落盘顺序（门禁执行口径）

1. 先补 11 语言 locale：`error` 段 5 码 + `bot.quota.*` 20 键 + `bot.input.*` 14 键；
2. 再把 5 码常量写进 `src-tauri/src/commands/error_code.rs`（`check-errorcode-alignment.mjs` B 节：码 ⊆ locale 且无孤儿）；
3. 把 34 个 UI 键加入 `scripts/check-bot-i18n-keys.mjs` 的 `BOT_I18N_KEYS` 必需集合（P3 后为 151，P4 后期望 185）；
4. 前端组件（A4/B4）最后引键 → `check:i18n` / `check:i18n:key-parity` 零缺失、零多余。

## 4. 文案约束

- `bot.quota.subtitle` / `inheritHint` 必须明示「只收紧，不会放宽 P3 默认值」，不得暗示配额可放大并发或超时上限。
- `bot.quota.resetConfirm` 必须说明会删除该部门配置；不得写成「清空全部配额」而让人误以为影响其他部门。
- `bot.input.expiredHint` 必须说明「应用重启或超时后不可再作答」，不得暗示可恢复被中断的模型轮（ADR-P4-5）。
- `BOT_QUOTA_EXCEEDED` 不得译成通用「校验失败」；必须能被用户理解为「超出部门配额」。
- `BOT_INPUT_REQUEST_NOT_PENDING` 不得译成「未找到」；必须表达「已回答或已失效」。
- 待答卡的 `{time}` 占位符由前端格式化（分:秒），后端不拼文案。

## 5. DoD

`node scripts/check-errorcode-alignment.mjs`、`node scripts/check-bot-i18n-keys.mjs`、`npm run check:i18n`、`npm run check:i18n:key-parity` 四项零红；11 语言 diff 中 P4 键一一对应，无孤儿键、无缺失键、无空值。
