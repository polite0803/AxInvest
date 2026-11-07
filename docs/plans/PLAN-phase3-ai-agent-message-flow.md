> 来源：导入自上游 AxAgent（基准 99c9b96f，2026-09-07） ｜ 导入日期：2026-11-04

# Phase 3: 接入 AI Agent 消息流

> 依赖：Phase 1（统一数据模型）+ Phase 2（适配器已启动）
> 目标：将平台消息桥接到 AI 对话系统，实现「用户消息 → AI 处理 → 回复回传」的完整闭环。

---

## 1. 现状分析

### 1.1 当前消息处理断点

目前 `PlatformMessageHandler.handle()` 的实现（[platform_integration.rs:L216-L232](../src-tauri/crates/trajectory/src/platform_integration.rs#L216-L232)）仅返回固定回声：

```rust
// TelegramHandler::handle
PlatformMessage::Telegram(tg_msg) => {
    let response = format!("Received: {}", tg_msg.text);
    Some(OutgoingMessage { ... })
}

// DiscordHandler::handle
PlatformMessage::Discord(dc_msg) => {
    let response = format!("Received: {}", dc_msg.content);
    Some(OutgoingMessage { ... })
}
```

完全没有与 AI 对话系统建立联系。

### 1.2 目标状态

```
[Telegram/Discord/...]
       │
       ▼ 消息到达
[PlatformAdapter 接收] → [PlatformManager 转发]
                              │
                              ▼
                         [PlatformBridge]
                              │
                    ┌─────────┼─────────┐
                    ▼                   ▼
              [SessionRouter]     [ConversationSystem]
              (会话管理)           (AI 生成回复)
                    │                   │
                    └─────────┬─────────┘
                              ▼
                        [发回消息]
                              │
                              ▼
                       [平台用户收到]
```

---

## 2. 修改方案

### 2.1 创建 PlatformBridge — 核心桥接层

**新建文件**: `src-tauri/crates/runtime/src/message_gateway/platform_bridge.rs`

```rust
use std::sync::Arc;
use tokio::sync::RwLock;
use sea_orm::DatabaseConnection;

use axagent_core::platform_config::PlatformConfig;
use crate::message_gateway::platform_manager::PlatformManager;
use crate::message_gateway::session_router::{RoutedSession, SessionRouter};

pub struct PlatformBridge {
    db: DatabaseConnection,
    platform_manager: Arc<PlatformManager>,
}

impl PlatformBridge {
    pub fn new(db: DatabaseConnection, platform_manager: Arc<PlatformManager>) -> Self {
        Self { db, platform_manager }
    }

    /// 核心方法：处理来自平台的消息，生成 AI 回复并发送回去
    pub async fn process_incoming_message(
        &self,
        platform: &str,
        user_id: &str,
        username: Option<&str>,
        chat_id: &str,
        text: &str,
    ) -> anyhow::Result<Option<String>> {
        // 1. 解析/创建会话
        let session = {
            let mut router = self.platform_manager.get_session_router_mut().await;
            router.resolve_or_create(platform, user_id, username.map(|s| s.to_string()))
                .clone()
        };

        // 2. 查找或创建 AI 对话（复用已有 conversation）
        let conversation_id = self.find_or_create_conversation(
            platform, user_id, &session
        ).await?;

        // 3. 将用户消息发送到 AI 系统，等待回复
        let assistant_reply = self.send_to_agent_and_wait(
            &conversation_id,
            text,
        ).await?;

        // 4. 将 AI 回复发送回平台
        if let Some(adapter) = self.platform_manager.get_adapter(platform).await {
            let config = self.load_config().await;
            adapter.send_message(&config, chat_id, &assistant_reply, None).await?;
        }

        // 5. 关联 Agent 会话
        {
            let mut router = self.platform_manager.get_session_router_mut().await;
            router.link_agent_session(platform, user_id, &conversation_id);
        }

        Ok(Some(assistant_reply))
    }

    /// 为平台用户查找或创建 conversation
    async fn find_or_create_conversation(
        &self,
        platform: &str,
        user_id: &str,
        session: &RoutedSession,
    ) -> anyhow::Result<String> {
        // 如果已有 agent_session_id，直接复用
        if let Some(ref agent_session_id) = session.agent_session_id {
            return Ok(agent_session_id.clone());
        }

        // 否则创建新 conversation
        let conv = axagent_core::repo::conversation::create_conversation(
            &self.db,
            &format!("[{}] Chat with {}", platform, session.username.as_deref().unwrap_or(user_id)),
            "127.0.0.1", // 本地 endpoint
            platform,     // 用 platform 作为 app_id 标识来源
            None,
        ).await?;

        Ok(conv.id)
    }

    /// 发送消息到 AI Agent 并阻塞等待回复
    async fn send_to_agent_and_wait(
        &self,
        conversation_id: &str,
        text: &str,
    ) -> anyhow::Result<String> {
        // ★ 关键：这里需要同步调用 conversations 系统的 send_message
        // 由于当前架构是流式事件驱动，需要特殊处理：
        //
        // 方案 A：直接调用 core::repo 层，构建消息并同步等待 LLM 非流式响应
        // 方案 B：通过 Tauri 事件总线，前端已经监听 chat-stream-chunk
        //
        // Phase 3 推荐方案 A（无需前端参与）

        // 1. 将用户消息写入数据库
        let user_msg = axagent_core::repo::message::create_message(
            &self.db,
            conversation_id,
            "user",
            text,
            &[],
        ).await?;

        // 2. 构建 ChatRequest 并调用 Provider
        // (需要获取 conversation 的 provider/model 配置)
        let conversation = axagent_core::repo::conversation::get_conversation(
            &self.db, conversation_id
        ).await?;

        // 3. 调用 LLM（非流式），获取完整回复
        // ... 直接调用 ProviderAdapter::chat()
        // 具体实现依赖 conversation 的 provider_id / model_id

        // 4. 将 assistant 回复写入数据库
        let assistant_msg = axagent_core::repo::message::create_message(
            &self.db,
            conversation_id,
            "assistant",
            &reply_text,
            &[],
        ).await?;

        Ok(reply_text)
    }

    async fn load_config(&self) -> PlatformConfig {
        axagent_core::repo::platform_config::get_platform_config(&self.db).await
    }
}
```

> **注意**：步骤 3（调用 LLM）的具体实现需要复用 `conversations.rs` 中的 `send_message_to_llm` 相关函数。可能需要将那段逻辑提取为 `core` crate 级别可复用函数。

### 2.2 提取 LLM 调用为可复用接口

**新建/修改文件**: `src-tauri/crates/core/src/repo/message_sender.rs`

从 `src-tauri/src/commands/conversations.rs` 中提取核心 LLM 调用逻辑：

```rust
use sea_orm::DatabaseConnection;
use crate::types::*;

/// 非流式调用 LLM，返回完整回复文本
pub async fn send_message_sync(
    db: &DatabaseConnection,
    conversation_id: &str,
    user_message: &str,
    master_key: &[u8; 32],
) -> Result<String, AxAgentError> {
    // 1. 加载 conversation 配置
    // 2. 加载 provider + key + model
    // 3. 构建消息历史
    // 4. 调用 adapter.chat() 非流式模式
    // 5. 返回 content
}
```

这个函数本质上是 `consume_stream` 的同步简化版。如果提取成本太高，可以临时使用 tokio channel 通过 `agent_query` 间接调用。

### 2.3 修改 PlatformManager — 支持消息回调注册

**文件**: `src-tauri/crates/runtime/src/message_gateway/platform_manager.rs`

在 Discord / Telegram 等适配器接收到消息时，需要回调 PlatformBridge：

```rust
impl PlatformManager {
    /// 设置消息到达回调（由 PlatformBridge 注册）
    pub async fn set_message_callback(
        &self,
        callback: Box<dyn PlatformMessageCallback>,
    ) {
        // 存储回调，适配器收到消息时调用
    }
}

#[async_trait::async_trait]
pub trait PlatformMessageCallback: Send + Sync {
    async fn on_message(
        &self,
        platform: &str,
        user_id: &str,
        username: Option<&str>,
        chat_id: &str,
        text: &str,
    ) -> Option<String>;
}
```

### 2.4 修改适配器：将收到的消息通过回调转发

以 Discord 为例（[discord.rs:L238-L290](../src-tauri/crates/runtime/src/message_gateway/platforms/discord.rs#L238-L290)）：

修改 `handle_dispatch` 中的消息处理逻辑：

```rust
// 旧代码：仅处理 !ping
if content == "!ping" { /* ... */ }

// 新代码：通过回调转发到 AI
if let Some(ref callback) = get_message_callback() {
    let response = callback.on_message(
        "discord",
        &author_id,
        Some(&author_username),
        &channel_id,
        &content,
    ).await;

    if let Some(reply) = response {
        // 通过 Discord webhook/webSocket 发送回复
        send_discord_reply(&channel_id, &reply).await;
    }
}
```

类似地修改 Telegram、WeChat、Feishu、QQ、DingTalk 的消息入口。

### 2.5 AppState 集成

**文件**: `src-tauri/src/app_state.rs`

```rust
pub platform_bridge:
    Arc<axagent_runtime::message_gateway::platform_bridge::PlatformBridge>,
```

**文件**: `src-tauri/src/init/state.rs`

```rust
let platform_bridge = Arc::new(
    axagent_runtime::message_gateway::platform_bridge::PlatformBridge::new(
        sea_db.clone(),
        platform_manager.clone(),
    )
);

// 注册回调
platform_manager.set_message_callback(Box::new(platform_bridge.clone())).await;
```

---

## 3. 替代方案

如果不想抽取 LLM 调用逻辑，可以使用 **Tauri 事件总线** 走前端中转：

```
[PlatformAdapter] → emit("platform-message") → [前端监听]
                                                     ↓
                                            invoke("process_platform_message")
                                                     ↓
                                         [Agent 生成回复，emit response]
                                                     ↓
                                            [前端调用 send_message 回发]
```

**缺点**：需要前端在线才能处理消息，不够健壮。

**推荐**：Phase 3 优先走方案 A（后端直接调用 LLM），因为消息渠道的本质是异步后台处理。

---

## 4. 实施步骤汇总

| # | 操作 | 文件 | 类型 |
|---|------|------|------|
| 1 | 创建 PlatformBridge | `runtime/.../message_gateway/platform_bridge.rs` | **新建** |
| 2 | 定义 PlatformMessageCallback trait | `runtime/.../message_gateway/platform_manager.rs` | 修改 |
| 3 | 提取 LLM 同步调用函数 | `core/src/repo/message_sender.rs` | **新建** (或复用) |
| 4 | 修改 DiscordAdapter 消息转发 | `runtime/.../platforms/discord.rs` | 修改 |
| 5 | 修改 TelegramAdapter 消息转发 | `runtime/.../platforms/telegram.rs` | 修改 |
| 6 | AppState 添加 platform_bridge | `src/app_state.rs` | 修改 |
| 7 | 初始化并注册回调 | `init/state.rs` | 修改 |
| 8 | conversation 创建时指定 platform 来源 | repo 层拓展 | 可选 |

---

## 5. 不做的事项

- ❌ 不新增前端 UI 功能（Phase 4）
- ❌ 不完善 6 个平台的消息接收能力（Phase 5）
- ❌ 不实现多轮对话上下文管理（后续迭代）
