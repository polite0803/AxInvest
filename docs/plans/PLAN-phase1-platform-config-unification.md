> 来源：导入自上游 AxAgent（基准 99c9b96f，2026-09-07） ｜ 导入日期：2026-11-04

# Phase 1: 消息渠道统一数据模型

> 目标：消除 `PlatformConfig` 双重定义，将统一结构体下沉到 `core` crate，统一前后端数据模型，建立配置持久化机制。

---

## 1. 现状分析

### 1.1 双重 PlatformConfig 问题

| 版本 | 文件 | 覆盖平台 | 是否被引用 |
|------|------|----------|------------|
| **A: message_gateway** | `crates/runtime/src/message_gateway/platform_config.rs` | 8 平台完整版 | ❌ 无 |
| **B: trajectory** | `crates/trajectory/src/platform_integration.rs` | 仅 Telegram/Discord | ✅ Tauri commands |

### 1.2 关键问题

- A 版本结构完整但从未被实例化使用，属于死代码
- B 版本在 Tauri command 层被实际使用，但字段不完整
- 配置仅存储在内存 `RwLock` 中，重启丢失
- **循环依赖**：`trajectory` 不能直接依赖 `runtime`（`runtime` 依赖 `trajectory`）

### 1.3 Crate 依赖关系

```
runtime ──> trajectory ──> core
                │
                └── core
```

结论：**统一结构体必须放在 `core` crate**，`runtime` 和 `trajectory` 各自删除本地定义，统一引用 `core`。

### 1.4 前端类型

- [types/platform.ts](../src/types/platform.ts) 仅覆盖 Telegram、Discord、API Server
- [platformStore.ts](../src/stores/feature/platformStore.ts) 默认值和 API 调用皆对应旧版 3 平台

---

## 2. 修改方案

### 2.1 统一 PlatformConfig 结构体

**决策：在 `core` crate 中新建 `platform_config.rs`，作为唯一数据源，覆盖全部 8 个平台。**

最终 `PlatformConfig` 覆盖以下平台：

| 平台 | 启用开关 | Token/凭据1 | Token/凭据2 | 可选配置 |
|------|---------|------------|------------|---------|
| Telegram | `telegram_enabled` | `telegram_bot_token` | `telegram_allowed_users` | `telegram_webhook_url`, `telegram_webhook_secret` |
| Discord | `discord_enabled` | `discord_bot_token` | `discord_allowed_channels` | `discord_webhook_url` |
| Slack | `slack_enabled` | `slack_bot_token` | `slack_signing_secret` | `slack_workspace_id` |
| WhatsApp | `whatsapp_enabled` | `whatsapp_phone_number_id` | `whatsapp_access_token` | `whatsapp_business_account_id` |
| WeChat | `wechat_enabled` | `wechat_app_id` | `wechat_app_secret` | `wechat_token`, `wechat_encoding_aes_key`, `wechat_original_id` |
| Feishu | `feishu_enabled` | `feishu_app_id` | `feishu_app_secret` | `feishu_verification_token`, `feishu_encrypt_key` |
| QQ | `qq_enabled` | `qq_bot_app_id` | `qq_bot_token` | `qq_bot_secret` |
| DingTalk | `dingtalk_enabled` | `dingtalk_app_key` | `dingtalk_app_secret` | `dingtalk_robot_code` |

通用配置：

| 字段 | 类型 | 默认值 | 说明 |
|------|------|--------|------|
| `api_server_enabled` | `bool` | `false` | REST API Server |
| `api_server_port` | `Option<u16>` | `None` | API Server 端口 |
| `auto_sync_messages` | `bool` | `true` | 自动同步消息 |
| `max_history_per_session` | `usize` | `100` | 单会话最大历史 |

### 2.2 文件变更清单

#### 步骤 1: 在 `core` crate 中创建统一的 PlatformConfig

**新建文件**: `src-tauri/crates/core/src/platform_config.rs`

以 message_gateway A 版本为基础，将完整 `PlatformConfig` 定义及其 `Default`、`validate()` 实现放到 `core` crate 中。同时补充各平台子配置 struct（如 `TelegramConfig`, `DiscordConfig` 等）和 `platforms()` builder 方法。

#### 步骤 2: 删除 `runtime::message_gateway::platform_config` 中的本地定义

**文件**: `src-tauri/crates/runtime/src/message_gateway/platform_config.rs`

- 删除 `PlatformConfig` 结构体和 `Default` / `validate` 实现
- 改为 `pub use axagent_core::platform_config::PlatformConfig;` 重新导出
- 这样 `runtime` 内的 platform adapters 可以继续从 `crate::message_gateway::platform_config::PlatformConfig` 引用

#### 步骤 3: 删除 `trajectory::PlatformConfig` 重复定义

**文件**: `src-tauri/crates/trajectory/src/platform_integration.rs`

- 删除本地的 `PlatformConfig` 结构体定义（第13-35行）和其 `Default` 实现
- 替换为 `use axagent_core::platform_config::PlatformConfig;`

> `trajectory` 已依赖 `axagent-core`（[Cargo.toml:L28](../src-tauri/crates/trajectory/Cargo.toml#L28)），无需新增依赖。

#### 步骤 4: 更新 `core::lib.rs` 导出

**文件**: `src-tauri/crates/core/src/lib.rs`

添加 `pub mod platform_config;`

#### 步骤 5: 更新 Tauri commands 中的引用

**文件**: `src-tauri/src/commands/platform_integration.rs`

修改 import 路径：
```rust
// 旧：use axagent_trajectory::{..., PlatformConfig, ...};
// 新：
use axagent_core::platform_config::PlatformConfig;
use axagent_trajectory::{DiscordMessage, MessagePlatform, OutgoingMessage, PlatformMessage,
                         PlatformSession, TelegramMessage};
```

#### 步骤 6: 创建 PlatformConfig Repository（持久化）

**新建文件**: `src-tauri/crates/core/src/repo/platform_config.rs`

复用现有 `settings` 表，通过 key `"platform_config"` 存储完整序列化 JSON：

```rust
use sea_orm::DatabaseConnection;
use crate::platform_config::PlatformConfig;
use crate::repo::settings;

pub async fn get_platform_config(db: &DatabaseConnection) -> PlatformConfig {
    settings::get_setting(db, "platform_config")
        .await
        .ok()
        .flatten()
        .and_then(|v| serde_json::from_str(&v).ok())
        .unwrap_or_default()
}

pub async fn save_platform_config(
    db: &DatabaseConnection,
    config: &PlatformConfig,
) -> crate::error::Result<()> {
    let json = serde_json::to_string(config)
        .map_err(|e| crate::error::AxAgentError::Serialize(e.to_string()))?;
    settings::set_setting(db, "platform_config", &json).await
}
```

同时添加 `pub mod platform_config;` 到 `core/src/repo/mod.rs` 的导出列表中。

#### 步骤 7: 更新 `PlatformIntegrationService`

**文件**: `src-tauri/crates/trajectory/src/platform_integration.rs`

```rust
use axagent_core::platform_config::PlatformConfig;

pub struct PlatformIntegrationService {
    config: RwLock<PlatformConfig>,
    sessions: RwLock<HashMap<String, PlatformSession>>,
    message_handlers: Vec<Box<dyn PlatformMessageHandler>>,
}
```

添加 `with_config` 构造函数：
```rust
impl PlatformIntegrationService {
    pub fn with_config(config: PlatformConfig) -> Self {
        Self {
            config: RwLock::new(config),
            sessions: RwLock::new(HashMap::new()),
            message_handlers: Vec::new(),
        }
    }
}
```

#### 步骤 8: 更新 Tauri Command 初始化（加载持久化配置）

**文件**: `src-tauri/src/init/state.rs`

```rust
let platform_config = block_on(
    axagent_core::repo::platform_config::get_platform_config(&sea_db)
);
let platform_integration_service = Arc::new(tokio::sync::RwLock::new(
    axagent_trajectory::PlatformIntegrationService::with_config(platform_config),
));
```

#### 步骤 9: 更新 Tauri commands 配置写回

**文件**: `src-tauri/src/commands/platform_integration.rs`

修改 `update_platform_config` 在更新内存的同时写入数据库：

```rust
#[tauri::command]
pub async fn update_platform_config(
    state: State<'_, AppState>,
    config: PlatformConfig,
) -> Result<(), String> {
    // 持久化到数据库
    axagent_core::repo::platform_config::save_platform_config(&state.sea_db, &config)
        .await
        .map_err(|e| e.to_string())?;
    // 更新内存
    let service = state.platform_integration_service.write().await;
    service.update_config(config).await;
    Ok(())
}
```

#### 步骤 10: 统一前端类型

**文件**: `src/types/platform.ts`

扩展 `PlatformConfig` 为覆盖全部 8 平台 + 通用字段的完整接口（字段名与 Rust 结构体 `snake_case` 一一对应）。

额外添加前端辅助类型：

```typescript
export interface PlatformMeta {
  name: string;
  label: string;
  icon: string;
  enabledKey: keyof PlatformConfig;
}

export const ALL_PLATFORMS: PlatformMeta[] = [
  { name: "telegram",  label: "Telegram",  icon: "✈️", enabledKey: "telegram_enabled"  },
  { name: "discord",   label: "Discord",   icon: "💬", enabledKey: "discord_enabled"   },
  { name: "slack",     label: "Slack",     icon: "💼", enabledKey: "slack_enabled"     },
  { name: "whatsapp",  label: "WhatsApp",  icon: "📱", enabledKey: "whatsapp_enabled"  },
  { name: "wechat",    label: "WeChat",    icon: "💚", enabledKey: "wechat_enabled"    },
  { name: "feishu",    label: "Feishu",    icon: "🐦", enabledKey: "feishu_enabled"    },
  { name: "qq",        label: "QQ",        icon: "🐧", enabledKey: "qq_enabled"        },
  { name: "dingtalk",  label: "DingTalk",  icon: "🔷", enabledKey: "dingtalk_enabled"  },
  { name: "api_server",label: "API Server",icon: "🔌", enabledKey: "api_server_enabled"},
];
```

#### 步骤 11: 更新前端 Store

**文件**: `src/stores/feature/platformStore.ts`

- `defaultConfig` 补齐 6 个缺失平台字段（全部默认 `false` / `null`）
- `loadStatuses()` 使用 `ALL_PLATFORMS` 辅助常量遍历生成 9 个平台状态

#### 步骤 12: 更新前端 UI 组件

**文件**: `src/components/settings/PlatformStatusCard.tsx`
- `platformMeta` 替换为导入 `ALL_PLATFORMS`

**文件**: `src/components/settings/GatewayConfigPanel.tsx`
- 为 6 个新增平台添加配置表单区域（Switch 开关 + Token 输入框）

---

## 3. 依赖关系（修正后）

```
core (PlatformConfig + repo)
  ↑                ↑
  │                │
trajectory      runtime::message_gateway::platform_config
(使用 core)     (重新导出 core)
  ↑                │
  │                └──────────┐
  │                           │
commands                    platform adapters
(使用 core + trajectory)    (通过 re-export 使用)
```

无循环依赖，`core` 是共用的基础层。

---

## 4. 实施步骤汇总

| # | 操作 | 文件 | 类型 |
|---|------|------|------|
| 1 | 创建统一 `PlatformConfig` | `core/src/platform_config.rs` | **新建** |
| 2 | `core::lib.rs` 添加导出 | `core/src/lib.rs` | 修改 |
| 3 | `runtime` 重新导出 core 定义 | `runtime/src/message_gateway/platform_config.rs` | 修改 |
| 4 | 删除 `trajectory` 本地定义 | `trajectory/src/platform_integration.rs` | 修改 |
| 5 | 更新 commands import | `commands/platform_integration.rs` | 修改 |
| 6 | 创建 repo 持久化层 | `core/src/repo/platform_config.rs` | **新建** |
| 7 | 添加 repo 模块导出 | 查找 core repo 导出文件 | 修改 |
| 8 | `PlatformIntegrationService` 加 `with_config` | `trajectory/src/platform_integration.rs` | 修改 |
| 9 | 初始化加载持久化配置 | `init/state.rs` | 修改 |
| 10 | 命令层配置写回 DB | `commands/platform_integration.rs` | 修改 |
| 11 | 统一前端类型 + 辅助常量 | `src/types/platform.ts` | 修改 |
| 12 | 更新前端 Store | `src/stores/feature/platformStore.ts` | 修改 |
| 13 | 更新前端组件 | `PlatformStatusCard.tsx`, `GatewayConfigPanel.tsx` | 修改 |

---

## 5. 不做的事项

- ❌ 不实际启动平台适配器（Phase 2）
- ❌ 不修改 SessionRouter（Phase 2）
- ❌ 不接入 AI Agent 消息流（Phase 3）
- ❌ 不在 SettingsPage 中添加消息渠道入口（Phase 4）
- ❌ 不实现各平台消息接收回调（Phase 5）
