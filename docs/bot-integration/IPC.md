# IPC — Bot 的 Tauri 命令与事件契约

> 状态：active ｜ 日期：2026-11-22 ｜ 对应基线：`11f32873b` ｜ 来源：本地原创 ｜ 范围：`docs/bot-integration/`

## 0. 约定（本仓硬规则）

1. **命令注册两步**：`src-tauri/src/commands/mod.rs` 写 `pub mod bot;` + 函数标注 `#[tauri::command]`；`src-tauri/src/register_commands.rs` 由 `build.rs` 自动生成，**禁止手改**。
2. **参数 camelCase**：Tauri v2 默认 `rename_all = "camelCase"`，前端 `invoke` 必须传 `botId`，不是 `bot_id`。
3. **DTO camelCase**：Rust 结构体字段 snake_case + `#[serde(rename_all = "camelCase")]`；TS 侧消费 camelCase。
4. **错误码**：返回 `Result<T, String>`，但字符串内容必须是 `ErrorResponse` 的 JSON（`commands/error.rs`）；码常量放 `commands/error_code.rs`，并同步 11 语言 `error.<CODE>`。
5. **不新增事件**：P1 复用 `agent-*` / `conversation-*` 既有事件；新增 `IpcEventName` 会牵动 `types/generated/events.ts`，P1 不需要。
6. **IPC 只经 `@/lib/invoke`**：前端禁止直接 `import { invoke } from "@tauri-apps/api/core"`。

## 1. 命令总表

| # | 命令 | 参数 | 返回 | 作用 |
|---|---|---|---|---|
| 1 | `list_bots` | `includeDisabled?: boolean` | `Bot[]` | 侧栏列表；默认只返回 `is_enabled=1` |
| 2 | `get_bot` | `botId: string` | `Bot` | 详情 |
| 3 | `create_bot` | `input: CreateBotInput` | `Bot` | 新建（生成 id、建目录、写 SOUL 镜像） |
| 4 | `update_bot` | `botId: string`, `input: UpdateBotInput` | `Bot` | 局部更新 |
| 5 | `delete_bot` | `botId: string` | `DeleteBotResult` | 删记录、线程置 NULL、保留记忆目录 |
| 6 | `list_bot_threads` | `botId: string`, `includeArchived?: boolean` | `Conversation[]` | 该 Bot 的线程 |
| 7 | `read_bot_soul` | `botId: string` | `BotSoul` | SOUL 正文 + hash + 漂移 |
| 8 | `write_bot_soul` | `botId: string`, `content: string` | `BotSoul` | 写 DB + 原子重写镜像 |
| 9 | `read_bot_memory` | `botId: string` | `BotMemory` | MEMORY.md 索引 + 预算 + 主题清单（只读） |

不在 P1：`write_bot_memory`、记忆写审批、Bot 级审批执行、Bot API key、技能归属。

## 2. Rust 签名

新增 `src-tauri/src/commands/bot.rs`：

```rust
// SPDX-License-Identifier: AGPL-3.0-only

use crate::AppState;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, State};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Bot {
    pub id: String,
    pub name: String,
    pub title: Option<String>,
    pub description: Option<String>,
    pub soul: String,
    pub soul_hash: String,
    pub soul_drift: bool,
    pub icon: String,
    pub color: Option<String>,
    pub section: String,
    pub model_id: Option<String>,
    pub provider_id: Option<String>,
    pub effort: Option<String>,
    pub workdir: Option<String>,
    pub approval_mode: String,
    pub agent_profile_id: Option<String>,
    pub is_enabled: bool,
    pub sort_order: i32,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateBotInput {
    pub name: String,
    pub title: Option<String>,
    pub description: Option<String>,
    pub soul: Option<String>,
    pub icon: Option<String>,
    pub color: Option<String>,
    pub section: Option<String>,
    pub model_id: Option<String>,
    pub provider_id: Option<String>,
    pub effort: Option<String>,
    pub workdir: Option<String>,
    pub approval_mode: Option<String>,
    pub agent_profile_id: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateBotInput {
    pub name: Option<String>,
    pub title: Option<Option<String>>,
    pub description: Option<Option<String>>,
    pub soul: Option<String>,
    pub icon: Option<String>,
    pub color: Option<Option<String>>,
    pub section: Option<String>,
    pub model_id: Option<Option<String>>,
    pub provider_id: Option<Option<String>>,
    pub effort: Option<Option<String>>,
    pub workdir: Option<Option<String>>,
    pub approval_mode: Option<String>,
    pub agent_profile_id: Option<Option<String>>,
    pub is_enabled: Option<bool>,
    pub sort_order: Option<i32>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BotSoul {
    pub content: String,
    pub hash: String,
    pub drift: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryBudget {
    pub max_lines: i64,
    pub max_bytes: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryTopic {
    pub relative_path: String,
    pub category: String,
    pub title: String,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BotMemory {
    pub index_content: String,
    pub line_count: i64,
    pub byte_count: i64,
    pub budget: MemoryBudget,
    pub topics: Vec<MemoryTopic>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeleteBotResult {
    pub detached_threads: i64,
    pub memory_path: String,
}

#[tauri::command]
pub async fn list_bots(
    state: State<'_, AppState>,
    include_disabled: Option<bool>,
) -> Result<Vec<Bot>, String>;

#[tauri::command]
pub async fn get_bot(state: State<'_, AppState>, bot_id: String) -> Result<Bot, String>;

#[tauri::command]
pub async fn create_bot(
    app: AppHandle,
    state: State<'_, AppState>,
    input: CreateBotInput,
) -> Result<Bot, String>;

#[tauri::command]
pub async fn update_bot(
    app: AppHandle,
    state: State<'_, AppState>,
    bot_id: String,
    input: UpdateBotInput,
) -> Result<Bot, String>;

#[tauri::command]
pub async fn delete_bot(
    state: State<'_, AppState>,
    bot_id: String,
) -> Result<DeleteBotResult, String>;

#[tauri::command]
pub async fn list_bot_threads(
    state: State<'_, AppState>,
    bot_id: String,
    include_archived: Option<bool>,
) -> Result<Vec<axagent_harness::Conversation>, String>;

#[tauri::command]
pub async fn read_bot_soul(
    state: State<'_, AppState>,
    bot_id: String,
) -> Result<BotSoul, String>;

#[tauri::command]
pub async fn write_bot_soul(
    app: AppHandle,
    state: State<'_, AppState>,
    bot_id: String,
    content: String,
) -> Result<BotSoul, String>;

#[tauri::command]
pub async fn read_bot_memory(
    state: State<'_, AppState>,
    bot_id: String,
) -> Result<BotMemory, String>;
```

实现注意：

- 命令层（wiring）可以依赖 `axagent-dao` 与 `axagent-agent`；**不得**在命令里直接写 SeaORM 查询（本仓 `check:layers` 约束）。
- `app` 参数用于可选事件；P1 不新增事件，但保留参数便于 P2。
- `list_bot_threads` 直接返回既有 `Conversation` DTO，不新建线程 DTO（禁止重复定义）。

## 3. TypeScript 类型（`src/types/bot.ts`，经 `src/types/index.ts` 导出）

```ts
export type ApprovalMode = "ask" | "edits" | "auto" | "full" | "custom";

export interface Bot {
  id: string;
  name: string;
  title: string | null;
  description: string | null;
  soul: string;
  soulHash: string;
  soulDrift: boolean;
  icon: string;
  color: string | null;
  section: string;
  modelId: string | null;
  providerId: string | null;
  effort: string | null;
  workdir: string | null;
  approvalMode: ApprovalMode;
  agentProfileId: string | null;
  isEnabled: boolean;
  sortOrder: number;
  createdAt: number;
  updatedAt: number;
}

export interface CreateBotInput {
  name: string;
  title?: string | null;
  description?: string | null;
  soul?: string;
  icon?: string;
  color?: string | null;
  section?: string;
  modelId?: string | null;
  providerId?: string | null;
  effort?: string | null;
  workdir?: string | null;
  approvalMode?: ApprovalMode;
  agentProfileId?: string | null;
}

export interface UpdateBotInput extends Partial<CreateBotInput> {
  isEnabled?: boolean;
  sortOrder?: number;
}

export interface BotSoul {
  content: string;
  hash: string;
  drift: boolean;
}

export interface MemoryBudget {
  maxLines: number;
  maxBytes: number;
}

export interface MemoryTopic {
  relativePath: string;
  category: string;
  title: string;
  updatedAt: number;
}

export interface BotMemory {
  indexContent: string;
  lineCount: number;
  byteCount: number;
  budget: MemoryBudget;
  topics: MemoryTopic[];
}

export interface DeleteBotResult {
  detachedThreads: number;
  memoryPath: string;
}
```

> `UpdateBotInput` 的可空字段在 TS 用 `null` 表达「显式置空」，`undefined` 表达「不改」；与 Rust 的 `Option<Option<T>>` 对齐。前端组装请求时必须**省略**未修改的键，不能传 `undefined` 占位（Tauri 会按缺失处理，行为一致）。

## 4. 调用示例（前端）

```ts
import { invoke } from "@/lib/invoke";

const bots = await invoke<Bot[]>("list_bots", { includeDisabled: false });
const bot = await invoke<Bot>("create_bot", {
  input: { name: "投研助手", soul: "# 你是投研助手\n..." },
});
const threads = await invoke<Conversation[]>("list_bot_threads", {
  botId: bot.id,
  includeArchived: false,
});
const soul = await invoke<BotSoul>("write_bot_soul", {
  botId: bot.id,
  content: nextSoul,
});
```

## 5. 错误码

新增到 `src-tauri/src/commands/error_code.rs` 的新模块 `pub mod bot`：

| 码 | 触发 | 前端 key |
|---|---|---|
| `BOT_NOT_FOUND` | id 不存在 | `error.BOT_NOT_FOUND` |
| `BOT_NAME_INVALID` | 名称空/超长/仅空白 | `error.BOT_NAME_INVALID` |
| `BOT_FIELD_TOO_LONG` | title/description 超长 | `error.BOT_FIELD_TOO_LONG` |
| `BOT_SOUL_TOO_LARGE` | SOUL > 64 KB | `error.BOT_SOUL_TOO_LARGE` |
| `BOT_APPROVAL_MODE_INVALID` | 不在枚举内 | `error.BOT_APPROVAL_MODE_INVALID` |
| `BOT_MODEL_PAIR_INVALID` | model/provider 只给一个 | `error.BOT_MODEL_PAIR_INVALID` |
| `BOT_WORKDIR_INVALID` | 目录不存在/非绝对路径 | `error.BOT_WORKDIR_INVALID` |
| `BOT_ICON_INVALID` | emoji 超长 | `error.BOT_ICON_INVALID` |
| `BOT_DELETE_FAILED` | 事务失败 | `error.BOT_DELETE_FAILED` |
| `BOT_MEMORY_READ_FAILED` | 读取 MEMORY.md 失败 | `error.BOT_MEMORY_READ_FAILED` |
| `BOT_SOUL_WRITE_FAILED` | 镜像写盘失败 | `error.BOT_SOUL_WRITE_FAILED` |

契约：`scripts/check-errorcode-alignment.mjs` 会校验「后端码值 ⊆ 11 语言 `error` 段」；缺一种语言即红。

## 6. 事件

**P1 不新增事件。** 理由与替代：

- Bot 列表/详情由命令返回值驱动，前端在 mutation 后本地更新 store，无需广播。
- SOUL/MEMORY 是单窗口编辑，写后返回新值即可。
- 线程变化复用既有 `conversation-title-updated`、`agent-*` 事件。

若 P2 需要多窗口同步，再新增 `bot-updated` / `bot-deleted`，并同步 `IpcEventName`、`types/generated/events.ts` 与 `contracts --only=g`。

## 7. 注册与生成物检查清单

| 步骤 | 位置 | 校验 |
|---|---|---|
| 1. 声明模块 | `src-tauri/src/commands/mod.rs` 加 `pub mod bot;` | `check-orphan-modules.mjs` |
| 2. 标注命令 | 每个 `pub async fn` 前 `#[tauri::command]` | `build.rs` 解析 |
| 3. 生成注册 | `src-tauri/src/register_commands.rs`（自动） | `contracts --only=a` |
| 4. 分层 | 命令不得直连 `sea_orm` / `entities` | `npm run check:layers` |
| 5. IPC 类型快照 | `axagent-plugins` 下 `cargo run -q -p schema-gen -- check` | CI `:660` |
| 6. 错误码对齐 | `error_code.rs` ↔ 11 语言 | `check-errorcode-alignment.mjs` |
| 7. 前端参数名 | `invoke("...", { botId })` camelCase | `.workbuddy/tmp/scan_ipc_args.py`（本地） |

## 8. 与 `list_conversations` 的关系

P1 **不改** `list_conversations` 的签名。理由：

- 它当前只接收 `state`，返回全部会话；前端已有调用方与测试。
- 改签名会让 `conversations` 命令的既有契约与 e2e 一并变动，收益仅为少一个命令。
- `list_bot_threads` 是纯新增，语义明确，便于单独测试与门禁。

P2 若需要「一次取全部会话并带 Bot 归属」，再给 `list_conversations` 加可选 `botId`，届时保持向后兼容。
