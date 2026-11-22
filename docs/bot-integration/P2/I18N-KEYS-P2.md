# P2-a 权威 i18n 键名清单（门禁协办，供前端 T4' / 后端 T5' 共同依据）

> 状态：frozen-for-P2-a ｜ 日期：2026-11-23 ｜ 基线：`b93d4b7c4` ｜ 作者：Bot 门禁与验证工程师（T5'/T7' 协办）
>
> 用法：前端 T4' 与后端 T5' **不得自行命名 i18n 键**；直接按本清单落盘。
> 后端落 `error_code.rs` 时，把 §2 的 2 个错误码**同时**写进 11 语言（`error.*` 段），
> 否则 `check-errorcode-alignment.mjs`（双向：后端码 ↔ 前端 error 段，11 语言集合一致）会红。
> 前端 T4' 落 §1 键后，必须把键名追加进 `scripts/check-bot-i18n-keys.mjs` 的 `BOT_I18N_KEYS`
> （P2-a 规定由**前端 T4' 随组件一并登记**，与代码同 commit，防止「键齐但无人引用」孤儿）。

## 0. 现有门禁判定面（已实测核对，P2-a 落盘时遵守）

| 门禁 | 与本次改动的关系 |
|---|---|
| `check-errorcode-alignment.mjs` | 双向：后端码必须在 11 语言 `error.*` 段有文案；前端大写码必须在后端有定义。**2 个新错误码必须与 `error_code.rs` 同 commit 落 11 语言**，单独落 locale = 红（孤儿码）。 |
| `check-i18n-key-parity.mjs` | 11 语言叶子路径集合全等。新键必须 11 语言齐。 |
| `check_i18n.py --strict` | zh-CN 空值即红；orphan（代码未引用）在 strict 下也判非零——**键与调用点应同 commit**。 |
| `check-i18n-untranslated.mjs` | de/fr/es/ru/ar/hi/ko 值不得含 CJK 汉字；ja 也**不得含简体字形**（ja 在 CJK 白名单但样例 `様子見` 会被正则命中——ja 译文须用日本汉字/假名）。 |
| `check-i18n-placeholder-usage.mjs` | 含 `{{x}}` 的值，调用点必须真插值；值不得回吐键路径。§1 带 `{{count}}`/`{{bot}}`/`{{tz}}` 的键，组件里用 `t(key, {count})` 形式。 |
| `check-bot-i18n-keys.mjs` | 必需 key 集合门禁（带 selftest）；新键登记进 `BOT_I18N_KEYS` 后 selftest 自动覆盖新键。 |

## 1. 前端 T4' 新增键（`bots.*` 与 `notification.*` 段，纯前端，可即落盘）

### 1.1 Bot 详情页「例程」区（`bots.*` 段）

| 键 | zh-CN | en-US | 用途 |
|---|---|---|---|
| `bots.tabRoutines` | 例程 | Routines | Bot 详情页 Tab 名 |
| `bots.routineAdd` | 新建例程 | New routine | 创建按钮 |
| `bots.routineName` | 例程名称 | Routine name | 表单：名称 |
| `bots.routineNameRequired` | 例程名称不能为空 | Routine name is required | 校验 |
| `bots.routinePrompt` | 例程提示词 | Routine prompt | 表单：prompt |
| `bots.routinePromptRequired` | 例程提示词不能为空 | Routine prompt is required | 校验 |
| `bots.routineCron` | Cron 表达式 | Cron expression | 表单：cron 输入框标签 |
| `bots.routineCronHint` | 5 段 cron，不支持每分钟 | 5-field cron; every-minute is not supported | 表单 hint |
| `bots.routineCronInvalid` | Cron 表达式无效（必须为 5 段，且不能为每分钟） | Invalid cron expression (must be 5 fields, not every-minute) | 坏 cron 报错文案（与 §2.1 后端码同源语义） |
| `bots.routineTimezone` | 时区 | Time zone | 表单：时区下拉标签 |
| `bots.routineTimezoneLocal` | 跟随系统时区 | Follow system time zone | 下拉「宿主 Local」预设项（= 后端 `null` 语义） |
| `bots.routineTimezoneInvalid` | 未知的 IANA 时区 | Unknown IANA time zone | 坏时区报错文案（与 §2.2 后端码同源语义） |
| `bots.routineEmpty` | 还没有例程 | No routines yet | 空态标题 |
| `bots.routineEmptyDescription` | 例程到点会自动以本 Bot 的身份与审批级别运行 | Routines run automatically as this bot, with its approval level, when due | 空态描述 |
| `bots.routineNextRun` | 下次运行 | Next run | 列表列 |
| `bots.routineLastRun` | 上次运行 | Last run | 列表列（数据复用任务中心历史） |
| `bots.routinePause` | 暂停 | Pause | 行操作 |
| `bots.routineResume` | 恢复 | Resume | 行操作 |
| `bots.routineDelete` | 删除例程 | Delete routine | 行操作 |
| `bots.routineDeleteConfirm` | 删除这条例程？删除后到点不再运行。 | Delete this routine? It will no longer run when due. | 删除确认 |
| `bots.routineSaveFailed` | 例程保存失败 | Failed to save routine | 通用失败（后端码文案经 `error.*` 展示时此键不参与） |
| `bots.routineBadge` | 例程 | Routine | 任务中心 cronAdapter 的 botId 徽章（复用 bots.tabRoutines 词根，避免两种译法） |

> 注意：T4' 列表行模型复用任务中心 cronAdapter；如需沿用 `tasksCenter.*` 既有键请直接复用，**不要**为本功能再造同义键。上表 `routineNextRun/routineLastRun` 若实测可复用 `tasksCenter.nextRun/lastRun` 则**不落盘**（以组件实现为准，宁缺勿重）。

### 1.2 通知（`notification.*` 段）

| 键 | zh-CN | en-US |
|---|---|---|
| `notification.filterByBot` | 按 Bot 筛选 | Filter by bot |
| `notification.allBots` | 全部 Bot | All bots |
| `notification.viewThread` | 查看线程 | View thread |
| `notification.unreadCount` | {{count}} 条未读 | {{count}} unread |

### 1.3 审批生效级别（`bots.*` 段，ADR-P2-4 落地后 UI 文案）

| 键 | zh-CN | en-US |
|---|---|---|
| `bots.approvalEffective` | 生效级别 | Effective level |
| `bots.approvalEffectiveHint` | 保存后立即生效：会话档位「默认」时使用本级别，非「默认」的会话仍按会话档位执行。 | Takes effect on save: used when a session's level is Default; sessions with an explicit level keep it. |

### 1.4 BotsPage 未读角标

| 键 | zh-CN | en-US |
|---|---|---|
| `bots.unreadBadge` | {{count}} | {{count}} |

## 2. 后端 T5' 错误码 ×2（与 `error_code.rs` 同 commit 落 11 语言）

落点：`src-tauri/src/commands/error_code.rs` + `src/i18n/locales/*` 的 `error.*` 段（**按字母序插入**，保持现有 479 键的排序惯例）。

| 码 | zh-CN | en-US | 备注 |
|---|---|---|---|
| `BOT_SCHEDULE_CRON_INVALID` | 无效的 cron 表达式（必须为 5 段，且不能为每分钟） | Invalid cron expression (must be 5 fields, not every-minute) | 包 `validate_cron_expression` 失败文案（IPC-P2 §2.1，避免前端解析自由字符串） |
| `BOT_SCHEDULE_TIMEZONE_INVALID` | 未知的 IANA 时区 | Unknown IANA time zone | `chrono_tz::Tz::from_str` 失败（IPC-P2 §2.1） |

**全部 11 语言译文**（后端直接照抄；`en-GB` 不在 locales 清单，实为 11 文件：zh-CN/zh-TW/en-US/ja/ko/de/fr/es/ru/ar/hi，与 IPC-P2 §3 所述「以 P1 清单为准」核对一致）：

| 键 | zh-CN | zh-TW | en-US | ja | ko | de | fr | es | ru | ar | hi |
|---|---|---|---|---|---|---||---|---|---|---|
| `BOT_SCHEDULE_CRON_INVALID` | 无效的 cron 表达式（必须为 5 段，且不能为每分钟） | 無效的 cron 運算式（必須為 5 段，且不能為每分鐘） | Invalid cron expression (must be 5 fields, not every-minute) | 無効な cron 式です（5 フィールド必須、毎分は指定できません） | 잘못된 cron 식입니다(5개 필드 필수, 매 분은 지원하지 않음) | Ungültiger Cron-Ausdruck (5 Felder erforderlich; jede Minute nicht erlaubt) | Expression cron invalide (5 champs requis, chaque minute non accepté) | Expresión cron no válida (debe tener 5 campos y no puede ser cada minuto) | Недопустимое выражение cron (должно быть 5 полей, ежеминутные не поддерживаются) | تعبير cron غير صالح (يجب أن يكون 5 حقول، ولا يُسمح بكل دقيقة) | अमान्य cron अभिव्यक्ति (5 फ़ील्ड आवश्यक हैं, और हर मिनट समर्थित नहीं है) |
| `BOT_SCHEDULE_TIMEZONE_INVALID` | 未知的 IANA 时区 | 未知的 IANA 時區 | Unknown IANA time zone | 不明な IANA タイムゾーン | 알 수 없는 IANA 시간대 | Unbekannte IANA-Zeitzone | Fuseau horaire IANA inconnu | Zona horaria de IANA desconocida | Неизвестный часовой пояс IANA | منطقة زمنية IANA غير معروفة | अज्ञात IANA समय क्षेत्र |

## 3. 落盘顺序（防门禁红的硬顺序）

1. 后端 T5'：`error_code.rs` 加 2 码 + 11 语言 `error.*` 同 commit → `check-errorcode-alignment` / `check-i18n-key-parity` 绿。
2. 前端 T4'：组件 + §1 键 11 语言同 commit，并把 §1 键追加进 `BOT_I18N_KEYS` → `check-bot-i18n-keys` / `check_i18n.py --strict`（调用点同 commit）绿。
3. 任一侧提前单落 locale 都会把对应门禁打红——这是顺序约束，不是风格建议。
