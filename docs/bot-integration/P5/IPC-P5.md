# IPC-P5 — P5 Tauri IPC 契约（冻结稿）

> 状态：frozen-for-P5 ｜ 日期：2026-11-25 ｜ 基线：`fedf00c17` + 本地三提交（`b0a43360e` / `1c7e75b63` / `afe255c40`）｜ 规则沿用 `IPC.md` / `IPC-P2.md` / `IPC-P2B.md` / `IPC-P3.md` / `IPC-P4.md`：两步注册（命令实现 + `register_commands.rs`）、snake_case 命令名、camelCase DTO、`Result<T, ErrorResponse>`、错误码入 11 语言、`scripts/check-contracts.mjs` 零红。
>
> 前端可据此契约与后端并行开工；任何字段增删须先改本文再改代码。
>
> **本批新增 9 条命令**（P5-b 六条 + P5-c1 三条），**修改 1 处内部契约**（`AssignmentTurnExecutor` 新增带回传用量的方法，ADR-P5-8），**不新增事件**。

## 1. 命令总览（新增 9）

### 1.1 P5-b（费用 / Token 配额）

| 命令 | 类型 | 落点 | 签名（Rust） |
|---|---|---|---|
| `get_bot_cost_quota` | 新增 | `src-tauri/src/commands/bot_cost.rs`（新文件） | `(scope: String, scope_id: String, state) -> Result<BotCostQuotaDto, ErrorResponse>` |
| `set_bot_cost_quota` | 新增 | 同上 | `(input: BotCostQuotaInput, state) -> Result<BotCostQuotaDto, ErrorResponse>` |
| `reset_bot_cost_quota` | 新增 | 同上 | `(scope: String, scope_id: String, state) -> Result<(), ErrorResponse>` |
| `list_bot_usage` | 新增 | 同上 | `(filter: BotUsageFilterDto, state) -> Result<Vec<BotUsageEntryDto>, ErrorResponse>` |
| `get_bot_usage_summary` | 新增 | 同上 | `(scope: String, scope_id: String, window_hours: Option<u32>, state) -> Result<BotUsageSummaryDto, ErrorResponse>` |

（`list_bot_usage` / `get_bot_usage_summary` 共 5 条；另 1 条为下方 C1 的 `import_bot_skill_preset`。）

### 1.2 P5-c1（每 Bot 技能归属 + preset 导入）

| 命令 | 类型 | 落点 | 签名（Rust） |
|---|---|---|---|
| `list_bot_skills` | 新增 | `src-tauri/src/commands/bot_skill.rs`（新文件） | `(bot_id: String, state) -> Result<Vec<BotSkillDto>, ErrorResponse>` |
| `set_bot_skill_enabled` | 新增 | 同上 | `(bot_id: String, skill_name: String, enabled: bool, state) -> Result<BotSkillDto, ErrorResponse>` |
| `import_bot_skill_preset` | 新增 | 同上 | `(bot_id: String, preset: BotSkillPresetDto, state) -> Result<BotSkillPresetResultDto, ErrorResponse>` |

### 1.3 修改（内部契约，非 Tauri 命令）

| 位置 | 变更 | 说明 |
|---|---|---|
| `src-tauri/src/bot_assignment_runner.rs` `AssignmentTurnExecutor` | **新增** `execute_with_usage(&self, assignment, task_text) -> Result<AssignmentTurnOutcome, String>`，**默认实现** = 现有 `execute_with_context` 包一层（usage 为 `None`） | ADR-P5-8：现有 `execute` / `execute_with_context` 签名**不变**；P2-b Fake 执行器零改动 |

**行为细则**：

- `get_bot_cost_quota`：无行返回「不限」默认值（`maxTotalTokens=0`、`maxCostUsd=0`、`windowHours=168`）+ `isDefault=true`、`updatedAt=null`；`scope` 非法 → `BOT_COST_QUOTA_INVALID`；`scope='bot'` 时 bot 不存在 → `BOT_NOT_FOUND`；`scope='department'` 时部门不存在 → `BOT_DEPARTMENT_NOT_FOUND`。
- `set_bot_cost_quota`：全量 upsert；逐字段范围校验 → `BOT_COST_QUOTA_INVALID`（detail 带字段名）；两个上限**至少一个非 0**（全 0 视为「不限」，与读接口的默认值等价 ⇒ 允许但等价于无行，命令层归一为删行语义见下）。
  - **归一化口径**：`maxTotalTokens=0` **且** `maxCostUsd=0` ⇒ 等价于「不限」⇒ 命令层**删除该行**并返回 `isDefault=true` 的合成 DTO（与 P4 `reset_bot_department_quota` 的恢复默认语义同款，用户不需要两个心智模型）。
- `reset_bot_cost_quota`：删行恢复不限；幂等（无行也返回 Ok）。
- `list_bot_usage`：过滤字段全可选；按 `created_at DESC`；`origin` 非法 → `ERR_VALIDATION`（沿用 P3/P4 口径，不新增码）。
- `get_bot_usage_summary`：返回窗口内的四类 token 合计 + 费用合计 + 条目数 + **是否已超限**（`breached: boolean`，按当前 scope 的配额判定）；无配额行 ⇒ `breached=false`。
- `list_bot_skills`：返回该 Bot 的技能列表，**每项标注来源**（`global` / `bot`）与是否同名被 Bot 内优先覆盖（`shadowedGlobal: boolean`）；归属行存在但目录缺失 ⇒ `missing: true`。
- `set_bot_skill_enabled`：只改**本 Bot** 的 `enabled` 标记，不动全局 `skill_states`；归属行不存在 → `BOT_SKILL_NOT_FOUND`。
- `import_bot_skill_preset`：**整包校验通过才落盘**（ADR-P5-6）；白名单外的条目**跳过并回报**（不整包失败）；返回 `{ imported: [...], skipped: [...], presetId }`；preset `version` 不是 `bot-skill-preset.v1` → `BOT_SKILL_PRESET_INVALID`。

## 2. DTO（camelCase）

```ts
// ── P5-b：费用/Token 配额 ──

/** 配额作用域：Bot 自身 / 部门合计。 */
export type BotCostQuotaScope = "bot" | "department";

export interface BotCostQuota {
  scope: BotCostQuotaScope;
  scopeId: string;
  maxTotalTokens: number;    // 0 = 不限
  maxCostUsd: number;        // 0 = 不限
  windowHours: number;       // 默认 168（7 天）
  isDefault: boolean;        // true = 无配额行，值为「不限」
  updatedAt: number | null;
}

export interface BotCostQuotaInput {
  scope: BotCostQuotaScope;
  scopeId: string;
  maxTotalTokens: number;
  maxCostUsd: number;
  windowHours: number;
}

/** 账本行的来源分类。 */
export type BotUsageOrigin = "interactive" | "routine" | "assignment";

/** 计价来源档位（ADR-P5-4）。 */
export type BotPricingSource = "model" | "config" | "heuristic" | "builtin";

export interface BotUsageEntry {
  id: string;
  botId: string;
  departmentId: string | null;      // 写入时快照
  conversationId: string | null;
  executionId: string | null;
  workflowRunId: string | null;
  assignmentId: string | null;
  modelId: string;
  origin: BotUsageOrigin;
  inputTokens: number;
  outputTokens: number;
  cacheCreationTokens: number;
  cacheReadTokens: number;
  totalTokens: number;
  estimatedCostUsd: number;        // 估算值，非账单
  pricingSource: BotPricingSource;
  createdAt: number;
}

export interface BotUsageFilter {
  botId?: string | null;
  departmentId?: string | null;
  origin?: BotUsageOrigin | null;
  workflowRunId?: string | null;
  assignmentId?: string | null;
  since?: number | null;            // epoch millis
  until?: number | null;
}

export interface BotUsageSummary {
  scope: BotCostQuotaScope;
  scopeId: string;
  windowHours: number;
  inputTokens: number;
  outputTokens: number;
  cacheCreationTokens: number;
  cacheReadTokens: number;
  totalTokens: number;
  estimatedCostUsd: number;        // 估算值
  entryCount: number;
  breached: boolean;               // 当前窗口用量是否已达配额上限
  pricingNote: string;             // 固定文案：估算口径说明
}

// ── P5-c1：每 Bot 技能归属 + preset 导入 ──

/** 技能来源：全局库 / 本 Bot 目录。 */
export type BotSkillSource = "global" | "bot";

export interface BotSkill {
  name: string;
  description: string;
  source: BotSkillSource;
  enabled: boolean;                // 本 Bot 维度（全局 skill_states 另算）
  shadowedGlobal: boolean;         // 同名时是否遮蔽了全局技能
  missing: boolean;                // 归属行在、目录缺失
  presetId: string | null;
  updatedAt: number | null;
}

/** preset 导入格式 v1（ADR-P5-6）。 */
export interface BotSkillPreset {
  version: "bot-skill-preset.v1";
  name: string;
  description: string;
  skills: BotSkillPresetItem[];
}

export interface BotSkillPresetItem {
  name: string;
  description: string;
  content: string;                 // SKILL.md 全文（含 frontmatter）
  category?: string | null;
  tags?: string[] | null;
  requiresToolsets?: string[] | null;
}

export interface BotSkillPresetSkipped {
  name: string;
  reason: "not_whitelisted" | "invalid";
  detail: string;
}

export interface BotSkillPresetResult {
  presetId: string;
  imported: string[];              // 成功导入的技能名
  skipped: BotSkillPresetSkipped[];
}
```

- Rust DTO 落 `axagent-core/crates/harness/src/types/bot.rs`；TS 镜像落 `src/types/bot.ts`；审计脚本按字段白名单核对（P3/P4 同款）。
- 账本行**不向前端暴露 JSON 字符串**：四类 token 已是数字列，前端不做 parse。

## 3. 事件

**本批不新增事件。**

| 事件 | 状态 |
|---|---|
| `bot-usage-updated` | **不新增** —— 用量是低频数据，页面打开时主动拉取即可；新增事件等于给高频账本写加一条广播通道，收益不抵复杂度（同 P4「配额不新增事件、保存后前端主动重取」的口径） |
| `bot-skill-updated` | **不新增** —— 同上，技能列表在页签打开时拉取 |

## 4. 错误码（新增 4）

| 码 | zh-CN 源文案 | 触发点 |
|---|---|---|
| `BOT_COST_QUOTA_INVALID` | Bot 费用配额取值不合法 | `scope` 非法 / 上限为负 / `windowHours` 越界；detail 带字段名 |
| `BOT_COST_QUOTA_EXCEEDED` | 超出费用或 Token 配额 | 新建 run / 新建轮前检查：窗口内用量 ≥ 上限；detail 带 scope、当前用量、上限 |
| `BOT_SKILL_NOT_FOUND` | 未找到该 Bot 的技能归属 | `set_bot_skill_enabled` 作用于不存在的归属行 |
| `BOT_SKILL_PRESET_INVALID` | 技能预设格式不合法 | preset `version` 不支持 / 必填字段缺失 / frontmatter 不可解析 |

**沿用不新增**：`BOT_NOT_FOUND`、`BOT_DEPARTMENT_NOT_FOUND`、`ERR_VALIDATION`（`list_bot_usage` 的 `origin` 非法）、`BOT_QUOTA_INVALID` / `BOT_QUOTA_EXCEEDED`（**P4 运行配额专用，本批不混用** —— 费用维度用 `BOT_COST_QUOTA_*`，两者语义不同：一个是运行上限，一个是用量上限）。

门禁基线：P4 收口实测 **504 码**（11 语言各 504，`src/i18n/locales/*.json` 的 `error` 段实测）；P5 后期望 **508**（以 `check-errorcode-alignment.mjs` 实测为准，不写死）。

## 5. 契约审计扩展（`scripts/lib/bot-ipc-audit.mjs`）

新增 `P5_COMMANDS` 段（仿 `P3_COMMANDS` / `P4_QUOTA_COMMANDS`）：

1. **8 条新命令**的 `pub async fn` 定义 + `register_commands.rs` 两步注册（P5-b 五条 + P5-c1 三条）；
2. **DTO 字段一致**（Rust + camelCase TS）：`BotCostQuota` / `BotCostQuotaInput` / `BotUsageEntry` / `BotUsageFilter` / `BotUsageSummary` / `BotSkill` / `BotSkillPreset` / `BotSkillPresetResult`；
3. **4 个新码 × 11 语言**（`error` 段）；
4. **`AssignmentTurnExecutor::execute_with_usage` 存在且 `execute` / `execute_with_context` 签名未变**（防误改 P2-b Fake 执行器契约）；
5. **`bot_skill_bindings` / `bot_cost_quotas` / `bot_usage_ledger` 三表在 `schema_compliance.rs` 的 SCHEMA 清单里**（防漏登记）；
6. **`usage_pricing` 四类分价被账本写入点引用**（防有人退回只算 input/output 的旧口径）；
7. **`bot-skill-preset.v1` 版本串在 Rust 与 TS 两侧一致**。

`check-contracts.mjs --selftest=bot` 至少加 4 条单点破坏自检：删命令、漏注册、DTO 缺字段、删 `execute_with_usage`。

`scripts/check-contracts.mjs` 的 sources 段需加 `p5:` 块（`botCostRs` / `botSkillRs` / `assignmentRunnerRs` / `usagePricingRs`），仿 `p3:` / `p4:` 的写法。

## 6. i18n 键

见 `I18N-KEYS-P5.md`：4 码 + `bot.usage.*` 17 键 + `bot.skills.*` 15 键 = 36 项 × 11 语言 = 396 条译文。落盘顺序固定「先 locale → 再 `error_code.rs` → 再 `BOT_I18N_KEYS` 必需集合 → 最后组件引键」。

## 7. 门禁命令原文（WSL ext4，与 PLAN-P5 §9.2 同源）

```bash
# 数据层
cargo test --manifest-path axagent-infra/Cargo.toml -p axagent-dao --lib bot_
cargo test --manifest-path axagent-infra/Cargo.toml -p axagent-dao --test schema_compliance

# P5 新增 Rust 测试
cargo test --manifest-path src-tauri/Cargo.toml --test bot_cost --test bot_skill

# P1–P4 回归（不新增红）
cargo test --manifest-path src-tauri/Cargo.toml --test bot_workflow --test bot_assignment --test bot_routine --test bot_delegation_chain --test bot_quota --test bot_input
cargo test --manifest-path axagent-runtime/Cargo.toml -p axagent-tools bot_delegate

# 契约与 i18n
npm run typecheck
node scripts/check-contracts.mjs --selftest=bot
node scripts/check-contracts.mjs
node scripts/check-errorcode-alignment.mjs
node scripts/check-bot-i18n-keys.mjs
npm run check:i18n
npm run check:i18n:key-parity
npm run check:routes
npm run check:layers
npm run check:serde   # 存量红（vector_store.rs）保持，不新增

# 格式
cargo fmt --manifest-path axagent-infra/Cargo.toml --all -- --check
cargo fmt --manifest-path src-tauri/Cargo.toml --all -- --check
npx dprint check

# 前端单测与 e2e
node scripts/vitest-run.mjs run
npx playwright test e2e/bot-cost.spec.ts e2e/bot-skills.spec.ts --project=chromium --grep-invert "@heavy" --timeout=120000 --reporter=list
npx playwright test e2e/bot-quota.spec.ts e2e/bot-needs-input.spec.ts --project=chromium --grep-invert "@heavy" --timeout=120000 --reporter=list
```

## 8. DoD

- `check-contracts.mjs`（含 selftest）零红，P5 段 8 命令 / 4 码 / 8 DTO 全覆盖；
- `check-errorcode-alignment.mjs` 码 ⊆ 11 语言且无孤儿；
- `check-bot-i18n-keys.mjs` 必需集合含 P5 32 个 UI 键（185 → 217）；
- 前端仅引本文字段，无自造字段、无 snake_case 泄漏；
- `AssignmentTurnExecutor` 的 `execute` / `execute_with_context` 签名**逐字节未变**（P2-b Fake 执行器零改动即证）。
