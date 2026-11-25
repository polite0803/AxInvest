# I18N-KEYS-P5 — P5 权威 i18n 键名清单（供门禁提前备 locale）

> 状态：frozen-for-P5 ｜ 日期：2026-11-25 ｜ 基线：`fedf00c17` + 本地三提交（`b0a43360e` / `1c7e75b63` / `afe255c40`）｜ 规则沿用 `I18N-KEYS-P2/P2B/P3/P4`：zh-CN 为源文案，11 语言齐更；落盘顺序「先 locale 后代码」防 errorcode-alignment 孤儿码红。
>
> **11 语言以现库实测为准**：`src/i18n/locales/` 下 `ar.json`、`de.json`、`en-US.json`、`es.json`、`fr.json`、`hi.json`、`ja.json`、`ko.json`、`ru.json`、`zh-CN.json`、`zh-TW.json`。
>
> **现库实测基线**：11 个 locale 的 `error` 段各 **504** 个键；`scripts/check-bot-i18n-keys.mjs` 的 `BOT_I18N_KEYS` **185** 条。本批新增 **4 码 + 32 UI 键** ⇒ 收口后期望 **508 码 / 217 个必需 UI 键**（以脚本实测输出为准，不写死）。

## 1. 后端错误码 ×4（`error` 段，IPC-P5 §4 冻结）

| 键 | zh-CN 源文案 | 卡 |
|---|---|---|
| `BOT_COST_QUOTA_INVALID` | Bot 费用配额取值不合法 | B3 |
| `BOT_COST_QUOTA_EXCEEDED` | 超出费用或 Token 配额 | B2/B5 |
| `BOT_SKILL_NOT_FOUND` | 未找到该 Bot 的技能归属 | C1-3 |
| `BOT_SKILL_PRESET_INVALID` | 技能预设格式不合法 | C1-4 |

4 码 × 11 语言 = **44 条译文**。

**不得与 P4 运行配额码混用**：`BOT_QUOTA_INVALID` / `BOT_QUOTA_EXCEEDED` 是**运行上限**（并发/超时/步数）；`BOT_COST_QUOTA_*` 是**用量上限**（token/费用）。两者语义不同、触发点不同、文案不同。

## 2. 前端键

### 2.1 `bot.usage.*`（17 键，用量与费用）

| 键 | zh-CN 源文案 | 用途 |
|---|---|---|
| `title` | 用量与费用 | 页签标题 |
| `subtitle` | 最近的 Token 用量与估算费用 | 页签副标题 |
| `estimated` | 估算 | 费用列的「估算」标识（**强制**，ADR-P5-4） |
| `estimatedHint` | 按本地价格表估算，与服务商账单可能存在差异 | 「估算」的悬浮说明 |
| `pricingSource` | 计价来源 | 计价来源列标签 |
| `inputTokens` | 输入 Token | 字段标签 |
| `outputTokens` | 输出 Token | 字段标签 |
| `cacheCreationTokens` | 缓存写入 Token | 字段标签 |
| `cacheReadTokens` | 缓存读取 Token | 字段标签 |
| `totalTokens` | Token 合计 | 汇总行标签 |
| `totalCost` | 估算费用 | 汇总行标签 |
| `quotaTitle` | 费用配额 | 配额区标题 |
| `quotaHint` | 超限后只拦截新建的运行，已启动的会跑完 | 熔断语义说明（**强制**，ADR-P5-3） |
| `breached` | 已超限 | 超限状态徽标 |
| `window` | 统计窗口 | 窗口字段标签 |
| `empty` | 暂无用量记录 | 空态 |
| `refresh` | 刷新 | 刷新按钮 |

### 2.2 `bot.skills.*`（15 键，每 Bot 技能）

| 键 | zh-CN 源文案 | 用途 |
|---|---|---|
| `title` | 技能 | 页签标题 |
| `subtitle` | 该 Bot 可用的技能；全局技能对所有 Bot 可见 | 页签副标题 |
| `globalSource` | 全局 | 来源标签（全局库） |
| `botSource` | 本 Bot | 来源标签（Bot 目录） |
| `shadowedGlobal` | 同名时优先使用本 Bot 的技能 | 同名遮蔽说明（ADR-P5-5） |
| `missing` | 技能文件缺失 | 归属行在、目录缺失（ADR-P5-5 的一致性口径） |
| `enabled` | 启用 | 本 Bot 启用状态 |
| `disabled` | 已停用 | 本 Bot 停用状态 |
| `empty` | 该 Bot 尚未配置技能 | 空态 |
| `importPreset` | 导入预设 | 导入按钮 |
| `presetName` | 预设名称 | preset 名称字段 |
| `presetImported` | 已导入 {count} 个技能 | 导入成功提示（`{count}` 由前端格式化） |
| `presetSkipped` | 跳过 {count} 个技能 | 跳过提示（白名单过滤，ADR-P5-6） |
| `presetInvalid` | 预设格式不合法 | 格式错误提示 |
| `whitelist` | 白名单 | 白名单说明 |

**前端合计 32 键 × 11 语言 = 352 条译文；前后端总计 396 条。**

## 3. 落盘顺序（门禁执行口径）

1. 先补 11 语言 locale：`error` 段 4 码 + `bot.usage.*` 17 键 + `bot.skills.*` 15 键；
2. 再把 4 码常量写进 `src-tauri/src/commands/error_code.rs` 的 `pub mod bot`（`check-errorcode-alignment.mjs` B 节：码 ⊆ locale 且无孤儿）；
3. 把 32 个 UI 键加入 `scripts/check-bot-i18n-keys.mjs` 的 `BOT_I18N_KEYS` 必需集合（P4 后为 185，P5 后期望 217）；
4. 前端组件（B5 / C1-5）最后引键 → `check:i18n` / `check:i18n:key-parity` 零缺失、零多余。

## 4. 文案约束

- `bot.usage.estimated` 必须出现且与费用数字**相邻**；费用列**不得**单独出现而没有「估算」标识。**禁止**把费用列写成「账单」「已花费」「余额」这类对账语义（ADR-P5-4）。
- `bot.usage.quotaHint` 必须写明「只拦截新建的运行，已启动的会跑完」—— 用户必须能理解熔断的滞后性（ADR-P5-3）。
- `bot.usage.breached` 必须表达「已超限」而非「额度不足」；超限是**已达上限**，不是「不够用」。
- `bot.skills.shadowedGlobal` 必须表达「同名时优先用本 Bot 的」，不能暗示全局技能被删除（ADR-P5-5）。
- `bot.skills.missing` 必须表达「文件缺失」而非「未安装」—— 归属声明在、目录不在，是**不一致**不是没装。
- `BOT_COST_QUOTA_EXCEEDED` 不得译成通用「配额不足」；必须能被用户理解为「已达用量上限，新运行被拦截」。
- `BOT_COST_QUOTA_INVALID` 不得译成「校验失败」；必须表达「费用配额取值不合法」。
- `BOT_SKILL_PRESET_INVALID` 必须表达「预设格式不合法」，不能译成「导入失败」（导入失败还可能是磁盘错误，两种原因要能区分）。
- `{count}` 占位符由前端格式化（数字转字符串），后端不拼文案。

## 5. DoD

`node scripts/check-errorcode-alignment.mjs`、`node scripts/check-bot-i18n-keys.mjs`、`npm run check:i18n`、`npm run check:i18n:key-parity` 四项零红；11 语言 diff 中 P5 键一一对应，无孤儿键、无缺失键、无空值。
