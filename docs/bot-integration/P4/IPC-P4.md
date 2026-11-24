# IPC-P4 — P4 Tauri IPC 契约（冻结稿）

> 状态：frozen-for-P4 ｜ 日期：2026-11-25 ｜ 基线：`416094880` ｜ 规则沿用 `IPC.md` / `IPC-P2.md` / `IPC-P2B.md` / `IPC-P3.md`：两步注册（命令实现 + `register_commands.rs`）、snake_case 命令名、camelCase DTO、`Result<T, ErrorResponse>`、错误码入 11 语言、`scripts/check-contracts.mjs` 零红。
>
> 前端可据此契约与后端并行开工；任何字段增删须先改本文再改代码。

## 1. 命令总览（新增 6，修改 1）

| 命令 | 类型 | 落点 | 签名（Rust） |
|---|---|---|---|
| `get_bot_department_quota` | 新增 | `src-tauri/src/commands/bot_quota.rs`（新文件） | `(department_id: String, state) -> Result<BotDepartmentQuotaDto, ErrorResponse>` |
| `set_bot_department_quota` | 新增 | 同上 | `(input: BotDepartmentQuotaInput, state) -> Result<BotDepartmentQuotaDto, ErrorResponse>` |
| `reset_bot_department_quota` | 新增 | 同上 | `(department_id: String, state) -> Result<(), ErrorResponse>` |
| `list_bot_input_requests` | 新增 | `src-tauri/src/commands/bot_input.rs`（新文件） | `(filter: BotInputRequestFilterDto, state) -> Result<Vec<BotInputRequestDto>, ErrorResponse>` |
| `answer_bot_input_request` | 新增 | 同上 | `(request_id: String, answer: String, state) -> Result<BotInputRequestDto, ErrorResponse>` |
| `cancel_bot_input_request` | 新增 | 同上 | `(request_id: String, state) -> Result<BotInputRequestDto, ErrorResponse>` |
| `agent_respond_ask` | **修改（签名不变）** | `src-tauri/src/commands/agent/mod.rs:3078` | `(ask_id: String, answer: String, state) -> Result<(), String>`（保持） |

**行为细则**：
- `get_bot_department_quota`：无行返回 P3 默认值 + `isDefault=true`、`updatedAt=null`；部门不存在 → `BOT_DEPARTMENT_NOT_FOUND`。
- `set_bot_department_quota`：全量 upsert；逐字段范围校验 → `BOT_QUOTA_INVALID`（detail 带字段名）；部门不存在 → `BOT_DEPARTMENT_NOT_FOUND`；成功回读行。
- `reset_bot_department_quota`：删行恢复默认；幂等（无行也返回 Ok）。
- `list_bot_input_requests`：四个过滤字段全可选；`status` 非法 → `ERR_VALIDATION`（沿用 P3 口径，不新增码）；按 `created_at DESC`。
- `answer_bot_input_request`：只接受 `pending`；去空白后为空 → `BOT_INPUT_ANSWER_EMPTY`；非 pending → `BOT_INPUT_REQUEST_NOT_PENDING`；不存在 → `BOT_INPUT_REQUEST_NOT_FOUND`；成功投递 oneshot 并转 `answered` + 事件。
- `cancel_bot_input_request`：只接受 `pending`；唤醒等待方为“已取消”；其余同上。
- `agent_respond_ask`：**签名与返回类型不变**；若 `ask_id` 命中 P4 行则额外转 `answered` + 事件，否则行为逐字节等于现状（通用聊天零变化）。

## 2. DTO（camelCase）

```ts
interface BotDepartmentQuotaDto {
  departmentId: string;
  maxConcurrency: number;        // 1..4, 默认 2
  totalTimeoutSecs: number;      // 60..3600, 默认 900
  maxSteps: number;              // 1..12, 默认 12
  maxInFlightPerDepth: number;   // 1..4, 默认 2
  stepTimeoutSecs: number;       // 30..900, 默认 300
  maxConcurrentRuns: number;     // 1..8, 默认 2
  isDefault: boolean;            // true = 无配额行，值为 P3 默认
  updatedAt: number | null;
}

interface BotDepartmentQuotaInput {
  departmentId: string;
  maxConcurrency: number;
  totalTimeoutSecs: number;
  maxSteps: number;
  maxInFlightPerDepth: number;
  stepTimeoutSecs: number;
  maxConcurrentRuns: number;
}

interface BotInputRequestFilterDto {
  conversationId?: string | null;
  botId?: string | null;
  status?: 'pending' | 'answered' | 'expired' | 'cancelled' | null;
  workflowRunId?: string | null;
}

interface BotInputRequestDto {
  id: string;
  askId: string;
  conversationId: string;
  botId: string | null;
  workflowRunId: string | null;
  assignmentId: string | null;
  origin: 'person' | 'routine' | 'workflow';
  question: string;
  options: string[];
  status: 'pending' | 'answered' | 'expired' | 'cancelled';
  answer: string | null;
  answeredAt: number | null;
  expiresAt: number;
  createdAt: number;
  updatedAt: number;
}
```

- Rust DTO 落 `axagent-core/crates/harness/src/types/bot.rs`；TS 镜像落 `src/types/bot.ts`；审计脚本按字段白名单核对（P3 同款）。
- `options_json` 在命令层 parse 成 `options: string[]`，前端永不接触 JSON 字符串。

## 3. 事件

| 事件 | payload | 触发点 |
|---|---|---|
| `bot-input-request-updated`（新增） | `BotInputRequestDto` | 落库 `pending`、答复 `answered`、超时/清扫 `expired`、取消 `cancelled` 各一次 |
| `agent-ask-user`（既有） | 原 payload | **保持原样**；交互桥继续发，保证通用聊天与旧卡片不回归 |
| 配额 | **不新增** | 保存后前端主动重取；配额不参与实时刷新 |

- `IpcEventName` 枚举落 `axagent-core/crates/harness/src/event_names.rs`（P3 段之后），`as_str() = "bot-input-request-updated"`；枚举完整性测试的变体清单同步追加。

## 4. 错误码（新增 5，沿用 4）

| 码 | zh-CN 源文案 | 触发点 |
|---|---|---|
| `BOT_QUOTA_INVALID` | Bot 部门配额取值不合法 | 任一字段越界/非整数；detail 带字段名 |
| `BOT_QUOTA_EXCEEDED` | 超出部门运行配额 | 保存/更新定义或触发 run 时 `定义值 > 部门配额`；detail 带字段名、定义值与上限 |
| `BOT_INPUT_REQUEST_NOT_FOUND` | 未找到待答请求 | 答复/取消不存在的 id |
| `BOT_INPUT_REQUEST_NOT_PENDING` | 待答请求已回答或已失效 | 已是 answered/expired/cancelled，或已无活 sender |
| `BOT_INPUT_ANSWER_EMPTY` | 答复内容不能为空 | 去空白后为空 |

**沿用不新增**：`AGENT_INPUT_WAIT_REPLY_TIMEOUT`（超时，`commands/error_code.rs` 既有）、`AGENT_INPUT_CHANNEL_CLOSED`、`BOT_DEPARTMENT_NOT_FOUND`、`BOT_NOT_FOUND`、`ERR_VALIDATION`、`ERR_PERMISSION`。

门禁基线：P3 收口实测 485 码；P4 后期望 **490**（以 `check-errorcode-alignment.mjs` 实测为准，不写死）。

## 5. 契约审计扩展（`scripts/lib/bot-ipc-audit.mjs`）

新增 `P4_COMMANDS` 段（仿 `P3_COMMANDS`）：

1. 6 条新命令的 `pub async fn` 定义 + `register_commands.rs` 两步注册；
2. `BotDepartmentQuota` / `BotInputRequest` 的 Rust/TS 字段一致（camelCase）；
3. `bot-input-request-updated` 进入 `IpcEventName` 与枚举完整性清单；
4. 5 个新码 × 11 语言（`error` 段）；
5. `BotInputRequestDto.origin` 与 `status` 值域；
6. `agent_respond_ask` 仍存在且签名未变（防误删通用路径）。

`check-contracts.mjs --selftest=bot` 至少加 4 条单点破坏自检：删命令、漏注册、DTO 缺字段、事件漏枚举。

## 6. i18n 键

见 `I18N-KEYS-P4.md`：5 码 + `bot.quota.*` 20 键 + `bot.input.*` 14 键 = 39 项 × 11 语言 = 429 条译文。落盘顺序固定「先 locale → 再 error_code.rs → 再 `BOT_I18N_KEYS` 必需集合 → 最后组件引键」。

## 7. 门禁命令原文（WSL ext4，与 PLAN-P4 §9.2 同源）

```bash
# 数据层
cargo test --manifest-path axagent-infra/Cargo.toml -p axagent-dao --lib bot_
cargo test --manifest-path axagent-infra/Cargo.toml -p axagent-dao --test schema_compliance

# P4 新增 Rust 测试
cargo test --manifest-path src-tauri/Cargo.toml --test bot_quota --test bot_input

# P1–P3 回归（不新增红）
cargo test --manifest-path src-tauri/Cargo.toml --test bot_workflow --test bot_assignment --test bot_routine --test bot_delegation_chain
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

# 前端单测与 e2e
node scripts/vitest-run.mjs run --grep-invert "@heavy"
npx playwright test e2e/bot-quota.spec.ts e2e/bot-needs-input.spec.ts --project=chromium --grep-invert "@heavy" --timeout=120000 --reporter=list
npx playwright test e2e/bot-workflow.spec.ts e2e/bot-delegation-chain.spec.ts --project=chromium --grep-invert "@heavy" --timeout=120000 --reporter=list
```

## 8. DoD

- `check-contracts.mjs`（含 selftest）零红，P4 段 6 命令 / 1 事件 / 5 码 / 2 DTO 全覆盖；
- `check-errorcode-alignment.mjs` 码 ⊆ 11 语言且无孤儿；
- `check-bot-i18n-keys.mjs` 必需集合含 P4 34 个 UI 键；
- 前端仅引本文字段，无自造字段、无 snake_case 泄漏。
