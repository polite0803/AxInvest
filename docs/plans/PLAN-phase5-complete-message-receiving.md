> 来源：导入自上游 AxAgent（基准 99c9b96f，2026-09-07） ｜ 导入日期：2026-11-04

# Phase 5: 完善消息接收能力

> 依赖：Phase 1-4
> 目标：补齐 6 个只有"发"没有"收"的平台的消息接收能力。

---

## 1. 现状分析

### 1.1 各平台消息接收实现情况

| 平台 | 当前状态 | 目标状态 | 难度 |
|------|---------|---------|------|
| Telegram | ✅ Long Polling getUpdates | ✅ 已完成 | — |
| Discord | ✅ WebSocket Gateway | ✅ 已完成（仅 !ping） | — |
| WeChat | 🔴 仅健康检查轮询 | 🟡 需要公网回调 URL | 高 |
| Feishu | 🔴 仅健康检查轮询 | 🟡 需要公网回调 URL | 高 |
| QQ | 🔴 仅健康检查轮询 | 🟡 需要 WebSocket + 公网 URL | 高 |
| DingTalk | 🔴 仅健康检查轮询 | 🟡 需要公网回调 URL | 高 |
| Slack | 🔴 完全无连接 | 🟢 支持 Socket Mode（无需公网） | 低 |
| WhatsApp | 🔴 完全无连接 | 🔴 需要 Meta Business 验证 | 高 |

### 1.2 分类

- **无需公网即可接收**：Telegram（Long Polling）、Discord（Gateway）、Slack（Socket Mode）
- **需要公网回调 URL**：WeChat、Feishu、QQ、DingTalk、WhatsApp

---

## 2. 修改方案

### 2.1 Telegram — 从固定命令升级为通用消息桥梁

**文件**: `src-tauri/crates/runtime/src/message_gateway/platforms/telegram.rs`

**当前问题**：`handle_telegram_command` 仅响应 `/start /help /ping /status` 四个固定命令。

**修改**：将非命令消息通过 `PlatformMessageCallback` 转发到 PlatformBridge：

```rust
// 修改 polling 循环中的消息处理逻辑
if let Some(ref callback) = PLATFORM_CALLBACK.load() {
    let response = callback.on_message(
        "telegram",
        &user_id,
        username.as_deref(),
        &chat_id.to_string(),
        &text,
    ).await;

    if let Some(reply) = response {
        send_telegram_message_custom(&client, &bot_token, chat_id, &reply).await;
    }
}
```

### 2.2 Discord — 从 !ping 升级为通用消息桥梁

**文件**: `src-tauri/crates/runtime/src/message_gateway/platforms/discord.rs`

**当前问题**：`handle_dispatch` 中 `MESSAGE_CREATE` 仅处理 `!ping`。

**修改**：所有非 Bot 消息转发到 PlatformBridge：

```rust
async fn handle_dispatch(
    event_type: &str,
    data: &serde_json::Value,
    allowed_channels: &Option<Vec<String>>,
) {
    match event_type {
        "MESSAGE_CREATE" => {
            // ... 现有验证逻辑

            // 替换原来的 !ping 处理
            if let Some(ref callback) = get_platform_callback() {
                let response = callback.on_message(
                    "discord",
                    &author_id,
                    Some(&author_username),
                    &channel_id,
                    &content,
                ).await;

                if let Some(reply) = response {
                    send_discord_reply(&channel_id, &reply).await;
                }
            }
        }
        // ...
    }
}

async fn send_discord_reply(channel_id: &str, text: &str) {
    // 通过 REST API /channels/{id}/messages 发送回复
    // 或者复用 webhook URL
}
```

### 2.3 Slack — 实现 Socket Mode 消息接收

Slack 提供 **Socket Mode**，不需要公网 URL，通过 WebSocket 接收事件。这是最易实现的"新平台接收"。

**修改文件**: `src-tauri/crates/runtime/src/message_gateway/platforms/slack.rs`

```rust
use tokio_tungstenite::tungstenite::Message;

// 新增 Socket Mode 连接流程：
// 1. apps.connections.open → 获取 WebSocket URL
// 2. 连接 WebSocket
// 3. 接收 events_api 类型消息
// 4. 分发到 PlatformBridge

struct SlackSocketMode {
    app_token: String,
    ws_url: Option<String>,
}

impl SlackSocketMode {
    async fn connect(&mut self) -> anyhow::Result<()> {
        // POST https://slack.com/api/apps.connections.open
        // Header: Authorization: Bearer {app_token}
        // Response: { ok: true, url: "wss://..." }
        let client = reqwest::Client::new();
        let resp = client
            .post("https://slack.com/api/apps.connections.open")
            .header("Authorization", format!("Bearer {}", self.app_token))
            .send().await?;
        let body: serde_json::Value = resp.json().await?;
        self.ws_url = body["url"].as_str().map(|s| s.to_string());
        Ok(())
    }

    async fn run_loop(&self, callback: &dyn PlatformMessageCallback) {
        let (mut ws, _) = tokio_tungstenite::connect_async(
            self.ws_url.as_ref().unwrap()
        ).await.unwrap();

        while let Some(Ok(Message::Text(text))) = ws.next().await {
            let payload: serde_json::Value = serde_json::from_str(&text).unwrap();

            if payload["type"] == "events_api" {
                let event = &payload["payload"]["event"];
                if event["type"] == "message" && event["bot_id"].is_null() {
                    let user = event["user"].as_str().unwrap_or("");
                    let channel = event["channel"].as_str().unwrap_or("");
                    let text = event["text"].as_str().unwrap_or("");

                    let response = callback.on_message(
                        "slack", user, None, channel, text,
                    ).await;

                    if let Some(reply) = response {
                        // 回发消息（使用现有 send_message 方法）
                        // 发送 ack 包: { "envelope_id": payload["envelope_id"] }
                    }
                }
            }
        }
    }
}
```

Slack Socket Mode 的优势：
- **无需公网 URL**：WebSocket 长连接由客户端发起
- **Token 类型**：使用 App-Level Token（`xapp-`），在 Slack App → Basic Information → App-Level Tokens 中创建，勾选 `connections:write`
- **配置字段**：需新增 `slack_app_token` 到 `PlatformConfig`

### 2.4 需要公网 URL 的平台 — 方案设计

WeChat/Feishu/QQ/DingTalk 都需要一个公网可达的回调 URL。提供两种方案：

**方案 A：本地 Gateway 端口暴露**
- 利用项目的 Gateway Server（已有 HTTP 服务）
- 添加 `/webhook/wechat`、`/webhook/feishu` 等路由
- 用户通过 ngrok/frp/cloudflare tunnel 将端口暴露到公网

**方案 B：平台内使用 Long Polling**
- QQ：使用 `GET /v2/groups/{group_openid}/messages` 轮询（需确认 API 可用性）
- DingTalk：使用 `GET /v1.0/im/conversations/{cid}/messages` 轮询
- Feishu/WeChat：不支持轮询，必须回调

**推荐实施策略**：
1. 优先实现 Slack（无需公网）
2. QQ 和 DingTalk 尝试 Long Polling 实现
3. WeChat/Feishu 在 Phase 5 中仅记录方案，待后续有条件时实现

### 2.5 PlatformConfig 扩展

新增 Slack Socket Mode 需要的字段：

**文件**: `src-tauri/crates/core/src/platform_config.rs`

```rust
pub struct PlatformConfig {
    // ... 现有字段
    pub slack_enabled: bool,
    pub slack_bot_token: Option<String>,
    pub slack_signing_secret: Option<String>,
    pub slack_workspace_id: Option<String>,
    pub slack_app_token: Option<String>,  // ← 新增：Socket Mode App-Level Token
    // ...
}
```

### 2.6 Webhook 路由注册（Gateway 集成）

为将来的公网回调需求预留 Gateway 路由：

**文件**: `src-tauri/crates/gateway/src/routes.rs`

```rust
// 预留路由（Phase 5 不实现完整逻辑，仅注册路由框架）
Router::new()
    .route("/webhook/telegram", post(handle_telegram_webhook))
    .route("/webhook/wechat", get(handle_wechat_verify).post(handle_wechat_callback))
    .route("/webhook/feishu", post(handle_feishu_callback))
    .route("/webhook/dingtalk", post(handle_dingtalk_callback))
    .route("/webhook/qq", post(handle_qq_callback))
```

每个 handler 的实现：解析平台回调 → 提取消息 → 调用 PlatformBridge。

> **注意**：这些路由仅在 Gateway 启动且用户配置了公网暴露时才有实际作用。如果 Gateway 未启动或仅绑定 127.0.0.1，回调将无法到达。

---

## 3. 实施步骤汇总

| # | 操作 | 文件 | 类型 | 优先级 |
|---|------|------|------|--------|
| 1 | Telegram 升级为通用桥梁 | `platforms/telegram.rs` | 修改 | P0 |
| 2 | Discord 升级为通用桥梁 | `platforms/discord.rs` | 修改 | P0 |
| 3 | Slack Socket Mode 实现 | `platforms/slack.rs` | 重写 | P1 |
| 4 | PlatformConfig 添加 `slack_app_token` | `core/src/platform_config.rs` | 修改 | P1 |
| 5 | Slack Socket Mode 连接循环 | `platforms/slack.rs` | 重写 | P1 |
| 6 | QQ Long Polling 尝试 | `platforms/qq.rs` | 修改 | P2 |
| 7 | DingTalk Long Polling 尝试 | `platforms/dingtalk.rs` | 修改 | P2 |
| 8 | Gateway Webhook 路由预留 | `gateway/src/routes.rs` | 修改 | P2 |
| 9 | 前端 Slack app_token 字段 | `GatewayConfigPanel.tsx`, `types/platform.ts` | 修改 | P2 |

---

## 4. 不做的事项（留给未来迭代）

- ❌ WeChat 公网回调实际实现（需要公网域名 + ICP 备案）
- ❌ Feishu 事件订阅实际实现（需要公网域名）
- ❌ WhatsApp Business API 回调实现（需要 Meta Business 验证 + 公网域名）
- ❌ 各平台消息格式富文本支持（Markdown→平台特定格式转换）
