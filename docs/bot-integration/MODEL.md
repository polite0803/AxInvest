# MODEL — Bot 数据模型与迁移策略

> 状态：active ｜ 日期：2026-11-22 ｜ 对应基线：`11f32873b` ｜ 来源：本地原创 ｜ 范围：`docs/bot-integration/`

## 1. 总览

```text
bots (新表)
  id ──────────────┐
  soul / soul_hash │
  workdir          │        conversations (既有表，加一列)
  approval_mode    │          id
  model_id         └───────►  bot_id  (NULL = 未归属，存量行为不变)
  agent_profile_id ──────► agent_profiles (既有，语义不变：专家/角色模板)
  section / sort_order
  created_at / updated_at

文件系统
  ~/.axagent/bots/<botId>/
    SOUL.md        ← 镜像（DB 真相源）
    MEMORY.md      → 真相源（注入用）
    memory/*.md    → 真相源（按需读取）
```

设计原则：**Bot 是主人，profile 是模板，conversation 是线程，session 是运行时**。四者各管一层，不互相冒充。

## 2. `bots` 表

### 2.1 字段

| 列 | SQLite 类型 | 空 | 默认 | 说明 |
|---|---|---|---|---|
| `id` | TEXT | 否 | — | 主键；UUID v4，创建时生成 |
| `name` | TEXT | 否 | — | 显示名，1–64 字符，去首尾空白后非空 |
| `title` | TEXT | 是 | NULL | 副标题/岗位（如「投研助手」） |
| `description` | TEXT | 是 | NULL | 一句话描述，≤ 500 字符 |
| `soul` | TEXT | 否 | `''` | SOUL 正文（Markdown），≤ 64 KB |
| `soul_hash` | TEXT | 否 | `''` | `sha256(soul)` 十六进制；用于镜像漂移检测 |
| `soul_drift` | INTEGER | 否 | 0 | 0=一致，1=磁盘镜像与 DB 不一致 |
| `icon` | TEXT | 否 | `'🤖'` | emoji 头像；P1 不做图片上传 |
| `color` | TEXT | 是 | NULL | 侧栏色标（`#rrggbb`） |
| `section` | TEXT | 否 | `'general'` | 侧栏分组键；P1 只用 `general` |
| `model_id` | TEXT | 是 | NULL | 默认模型；NULL = 跟随全局默认 |
| `provider_id` | TEXT | 是 | NULL | 默认提供商；与 `model_id` 同生同灭 |
| `effort` | TEXT | 是 | NULL | 推理档（`low|medium|high|max`），NULL = 引擎默认 |
| `workdir` | TEXT | 是 | NULL | Bot 工作目录；NULL = `~/.axagent/bots/<id>` |
| `approval_mode` | TEXT | 否 | `'full'` | `ask|edits|auto|full|custom`；P1 只存不执行（ADR-007） |
| `agent_profile_id` | TEXT | 是 | NULL | 可选：关联的角色/工具模板 |
| `is_enabled` | INTEGER | 否 | 1 | 0 = 归档，不在侧栏显示 |
| `sort_order` | INTEGER | 否 | 0 | 侧栏排序，小在前 |
| `created_at` | BIGINT | 否 | — | Unix 毫秒 |
| `updated_at` | BIGINT | 否 | — | Unix 毫秒 |

索引：`idx_bots_section`（`section`）、`idx_bots_sort_order`（`is_enabled, sort_order`）。命名沿用引擎的 `idx-<table>-<cols>` 归一化规则，实体上写 `#[sea_orm(indexed)]` 即可。

### 2.2 实体代码骨架

新增 `axagent-core/crates/entities/src/bots.rs`：

```rust
// SPDX-License-Identifier: AGPL-3.0-only

use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};

/// Bot —— 身份 + SOUL + 工作区 + 记忆 + 线程归属的宿主记录。
/// 与 `agent_profiles`（专家/角色模板）职责不同：Bot 是主人，profile 是模板。
#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[sea_orm(table_name = "bots")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: String,
    pub name: String,
    pub title: Option<String>,
    pub description: Option<String>,
    #[sea_orm(default_value = "")]
    pub soul: String,
    #[sea_orm(default_value = "")]
    pub soul_hash: String,
    #[sea_orm(default_value = 0)]
    pub soul_drift: i32,
    #[sea_orm(default_value = "🤖")]
    pub icon: String,
    pub color: Option<String>,
    #[sea_orm(default_value = "general")]
    pub section: String,
    pub model_id: Option<String>,
    pub provider_id: Option<String>,
    pub effort: Option<String>,
    pub workdir: Option<String>,
    #[sea_orm(default_value = "full")]
    pub approval_mode: String,
    pub agent_profile_id: Option<String>,
    #[sea_orm(default_value = 1)]
    pub is_enabled: i32,
    #[sea_orm(default_value = 0)]
    pub sort_order: i32,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
```

并在 `axagent-core/crates/entities/src/lib.rs` 增加：

```rust
pub mod bots;
```

> `dao/build.rs` 会读盘生成实体注册表，**无需**第二处登记；不需要 migration。

### 2.3 `conversations` 增量

`axagent-core/crates/entities/src/conversations.rs` 增加：

```rust
/// Bot 归属；NULL = 未归属（存量会话与「普通对话」均为此）。
#[sea_orm(indexed)]
pub bot_id: Option<String>,
```

- 不改 `agent_profile_id`。
- 不改任何既有列的默认值或可空性。
- 存量行 `bot_id` 自动为 NULL，行为完全不变。

## 3. `schema_compliance` 登记（强制）

`axagent-infra/crates/dao/tests/schema_compliance.rs:15` 的 `SCHEMA` 加两项：

1. 新表 `bots` 的全部 20 列（类型按上面的表）。
2. `conversations` 条目追加 `("bot_id", "TEXT")`。

不登记的后果不是编译错，而是该测试**覆盖不到**新列（静默缺口），与既有 `SCHEMA` 的设计意图相悖。

## 4. DAO 层

新增 `axagent-infra/crates/dao/src/repo/bot.rs`（implementor 层，允许依赖 `entities`）：

```rust
pub async fn list_bots(db: &DatabaseConnection, include_disabled: bool) -> Result<Vec<Bot>>;
pub async fn get_bot(db: &DatabaseConnection, id: &str) -> Result<Bot>;
pub async fn create_bot(db: &DatabaseConnection, input: &CreateBot) -> Result<Bot>;
pub async fn update_bot(db: &DatabaseConnection, id: &str, patch: &UpdateBotPatch) -> Result<Bot>;
pub async fn delete_bot(db: &DatabaseConnection, id: &str) -> Result<()>; // 同时把 conversations.bot_id 置 NULL
pub async fn list_bot_threads(
    db: &DatabaseConnection,
    bot_id: &str,
    include_archived: bool,
) -> Result<Vec<Conversation>>;
pub async fn set_soul(
    db: &DatabaseConnection,
    id: &str,
    soul: &str,
    hash: &str,
) -> Result<Bot>;
```

要点：

- `create_bot` 生成 UUID、`created_at/updated_at`，校验后插入；`name` 冲突**不禁止**（Bot 允许重名，靠 id 区分）。
- `update_bot` 只写 patch 中出现的字段（`Option<Option<T>>` 表达「不改」与「置空」的区别），最后统一 `updated_at`。
- `delete_bot` 在一个事务里删行 + 清 `conversations.bot_id`。
- `list_bot_threads` 复用 `conversation.rs` 的排序（`updated_at DESC`），过滤 `is_archived = 0`（除非显式包含）。

## 5. 文件系统模型

新增 `axagent-ai/crates/agent/src/bot_workspace.rs`（与 `personality.rs` / `project_memory.rs` 同层，负责 FS；不依赖 DB）：

```rust
pub struct BotWorkspace { root: PathBuf }

impl BotWorkspace {
    pub fn new(root: impl Into<PathBuf>) -> Self;
    pub fn root(&self) -> &Path;
    pub fn soul_path(&self) -> PathBuf;          // root/SOUL.md
    pub fn ensure(&self) -> Result<(), String>;  // mkdir 0700
    pub fn read_soul(&self) -> Result<String, String>;
    pub fn write_soul_atomic(&self, content: &str) -> Result<(), String>;
    pub fn memory(&self) -> ProjectMemory;        // ProjectMemory::new(root)
    pub fn soul_sha256(content: &str) -> String;  // 统一 hash 算法
}
```

- 默认根：`dirs::home_dir()/.axagent/bots/<botId>`；若 `bots.workdir` 非空，则工作目录用它，但 **SOUL/MEMORY 始终在 `~/.axagent/bots/<id>/`**（工作目录可指向用户项目，记忆不能跟着项目跑）。
- 原子写：写 `SOUL.md.tmp` → `fs::rename`；Unix 上 `set_permissions(0o600)`。
- 漂移判定：命令层读 `soul_hash`，再读镜像算 hash；不等则返回 `drift: true`，不自动覆盖任何一侧。

## 6. 兼容性与迁移

| 场景 | 行为 |
|---|---|
| 存量库首次启动（无 `bots` 表） | 引擎 `CreateTable` 建表；无数据写入 |
| 存量库首次启动（`conversations` 无 `bot_id`） | 引擎 `AddColumn` 加可空列；存量行 NULL |
| 存量会话打开 | `bot_id IS NULL` ⇒ 不注入 bot-soul/bot-memory，走原 persona 路径 |
| 旧 `~/.axagent/personalities/` | 不迁移、不删除；P2 提供导入 |
| 降级到无 Bot 的旧版本 | `bots` 表成为孤儿表，旧版本引擎会判孤儿并 DROP（本仓引擎行为）。**发布说明必须写「降级会丢 Bot 记录」**；P2 评估是否加保护 |
| 备份/恢复 | Bot 记录随 SQLite 备份；记忆目录需单独纳入备份清单（P2） |

**不做数据回填**：不把任何 `agent_profiles` 自动变成 Bot。用户新建即新建。

## 7. 校验规则（命令层统一执行）

| 字段 | 规则 | 错误码 |
|---|---|---|
| `name` | trim 后 1–64 字符 | `BOT_NAME_INVALID` |
| `title` | ≤ 120 字符 | `BOT_FIELD_TOO_LONG` |
| `description` | ≤ 500 字符 | `BOT_FIELD_TOO_LONG` |
| `soul` | ≤ 64 KB（UTF-8 字节） | `BOT_SOUL_TOO_LARGE` |
| `approval_mode` | ∈ `ask|edits|auto|full|custom` | `BOT_APPROVAL_MODE_INVALID` |
| `model_id` / `provider_id` | 同时为空或同时非空 | `BOT_MODEL_PAIR_INVALID` |
| `workdir` | 若存在必须是已存在的绝对目录 | `BOT_WORKDIR_INVALID` |
| `icon` | 1 个 grapheme，≤ 8 字节 | `BOT_ICON_INVALID` |

服务端校验是权威；前端同规则做即时提示，但不作为唯一防线。

## 8. 删除语义（与 ADR-009 一致）

```text
delete_bot(id):
  BEGIN
    DELETE FROM bots WHERE id = ?
    UPDATE conversations SET bot_id = NULL WHERE bot_id = ?
  COMMIT
  保留 ~/.axagent/bots/<id>/（SOUL.md / MEMORY.md / memory/）
```

不级联删消息、不删 session、不删记忆目录。

## 9. 与既有模型的关系（防重复定义）

| 既有对象 | 关系 | 禁止 |
|---|---|---|
| `agent_profiles` | Bot 可选引用，作为角色/工具模板 | 不得把 Bot 字段加进 profile；不得让 Bot 列表读 profile 表 |
| `agent_roles` | 可被 profile 引用，间接服务 Bot | 不得让 Bot 直接持有 role 字段 |
| `conversations` | Bot 的线程 | 不得用 `agent_profile_id` 冒充 `bot_id` |
| `agent_sessions` | 会话的运行时状态（cwd、permission_mode） | 不得把 Bot 配置复制进每个 session |
| `fleets` / `fleet_members` | P2 的组织层 | P1 不建第二套团队表 |
| `memory_namespaces` | 现有 DB 向量记忆 | 不得把 MEMORY.md 内容再灌进命名空间（两套真相源） |
