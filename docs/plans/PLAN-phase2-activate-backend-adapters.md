> 来源：导入自上游 AxAgent（基准 99c9b96f，2026-09-07） ｜ 导入日期：2026-11-04

# Phase 2: 激活后端适配器

> 依赖：Phase 1（统一数据模型 + 持久化）
> 目标：将 8 个平台适配器接入生命周期管理，按配置按需启动/停止，激活 SessionRouter。

---

## 1. 现状分析

### 1.1 已实现但未激活的适配器

8 个适配器在 `crates/runtime/src/message_gateway/platforms/` 下均完整实现了 `PlatformAdapter` trait：

| 适配器 | 文件 | 连接方式 | 状态 |
|--------|------|---------|------|
| `TelegramAdapter` | [platforms/telegram.rs](../src-tauri/crates/runtime/src/message_gateway/platforms/telegram.rs) | Long Polling | ✅ 实现完整 |
| `DiscordAdapter` | [platforms/discord.rs](../src-tauri/crates/runtime/src/message_gateway/platforms/discord.rs) | WebSocket Gateway | ✅ 实现完整 |
| `WeChatAdapter` | [platforms/wechat.rs](../src-tauri/crates/runtime/src/message_gateway/platforms/wechat.rs) | 健康检查轮询 | ⚠️ 无消息接收 |
| `FeishuAdapter` | [platforms/feishu.rs](../src-tauri/crates/runtime/src/message_gateway/platforms/feishu.rs) | 健康检查轮询 | ⚠️ 无消息接收 |
| `QQAdapter` | [platforms/qq.rs](../src-tauri/crates/runtime/src/message_gateway/platforms/qq.rs) | 健康检查轮询 | ⚠️ 无消息接收 |
| `DingtalkAdapter` | [platforms/dingtalk.rs](../src-tauri/crates/runtime/src/message_gateway/platforms/dingtalk.rs) | 健康检查轮询 | ⚠️ 无消息接收 |
| `SlackAdapter` | [platforms/slack.rs](../src-tauri/crates/runtime/src/message_gateway/platforms/slack.rs) | 无长连接 | ⚠️ 仅发送 |
| `WhatsAppAdapter` | [platforms/whatsapp.rs](../src-tauri/crates/runtime/src/message_gateway/platforms/whatsapp.rs) | 无长连接 | ⚠️ 仅发送 |

**核心问题**：`init/services.rs` 中无任何初始化代码，应用启动后这些适配器从未被创建。

### 1.2 SessionRouter 未使用

[message_gateway/session_router.rs](../src-tauri/crates/runtime/src/message_gateway/session_router.rs) 提供了：
- `resolve_or_create()` — 按平台+用户管理会话
- `link_agent_session()` — 关联 AI Agent 会话
- `list_active_sessions()` — 活跃会话列表
- `clear_expired_sessions()` — 过期清理

但**从未被任何代码实例化**。

---

## 2. 修改方案

### 2.1 创建 PlatformManager — 统一管理全部适配器

**新建文件**: `src-tauri/crates/runtime/src/message_gateway/platform_manager.rs`

```rust
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

use axagent_core::platform_config::PlatformConfig;
use crate::message_gateway::platforms::*;
use crate::message_gateway::session_router::SessionRouter;

pub struct PlatformManager {
    adapters: RwLock<HashMap<String, Arc<dyn PlatformAdapter>>>,
    session_router: RwLock<SessionRouter>,
    running_adapters: RwLock<Vec<String>>,
}

impl PlatformManager {
    pub fn new() -> Self {
        let mut adapters: HashMap<String, Arc<dyn PlatformAdapter>> = HashMap::new();

        adapters.insert("telegram".to_string(), Arc::new(telegram::TelegramAdapter::new()));
        adapters.insert("discord".to_string(),  Arc::new(discord::DiscordAdapter::new()));
        adapters.insert("wechat".to_string(),   Arc::new(wechat::WeChatAdapter::new()));
        adapters.insert("feishu".to_string(),   Arc::new(feishu::FeishuAdapter::new()));
        adapters.insert("qq".to_string(),       Arc::new(qq::QQAdapter::new()));
        adapters.insert("dingtalk".to_string(), Arc::new(dingtalk::DingtalkAdapter::new()));
        adapters.insert("slack".to_string(),    Arc::new(slack::SlackAdapter::new()));
        adapters.insert("whatsapp".to_string(), Arc::new(whatsapp::WhatsAppAdapter::new()));

        Self {
            adapters: RwLock::new(adapters),
            session_router: RwLock::new(SessionRouter::new()),
            running_adapters: RwLock::new(Vec::new()),
        }
    }

    /// 根据配置启动/停止各平台适配器
    pub async fn reconcile(&self, config: &PlatformConfig) -> anyhow::Result<PlatformReconcileReport> {
        let adapters = self.adapters.read().await;
        let mut running = self.running_adapters.write().await;
        let mut report = PlatformReconcileReport::default();

        for (name, adapter) in adapters.iter() {
            if adapter.is_enabled(config) {
                if !running.contains(name) {
                    match adapter.start(config).await {
                        Ok(()) => {
                            running.push(name.clone());
                            report.started.push(name.clone());
                            tracing::info!("[PlatformManager] {} started", name);
                        }
                        Err(e) => {
                            report.errors.push((name.clone(), e.to_string()));
                            tracing::error!("[PlatformManager] {} start failed: {}", name, e);
                        }
                    }
                }
            } else {
                if running.contains(name) {
                    match adapter.stop().await {
                        Ok(()) => {
                            running.retain(|n| n != name);
                            report.stopped.push(name.clone());
                            tracing::info!("[PlatformManager] {} stopped", name);
                        }
                        Err(e) => {
                            report.errors.push((name.clone(), e.to_string()));
                            tracing::error!("[PlatformManager] {} stop failed: {}", name, e);
                        }
                    }
                }
            }
        }

        Ok(report)
    }

    /// 执行全量停止
    pub async fn stop_all(&self) -> anyhow::Result<()> {
        let adapters = self.adapters.read().await;
        let mut running = self.running_adapters.write().await;

        for name in running.iter() {
            if let Some(adapter) = adapters.get(name) {
                let _ = adapter.stop().await;
            }
        }
        running.clear();
        Ok(())
    }

    /// 获取所有启用的适配器
    pub async fn get_adapter(&self, name: &str) -> Option<Arc<dyn PlatformAdapter>> {
        let adapters = self.adapters.read().await;
        adapters.get(name).cloned()
    }

    pub async fn get_running_adapters(&self) -> Vec<String> {
        self.running_adapters.read().await.clone()
    }

    pub async fn get_session_router(&self) -> RwLockReadGuard<'_, SessionRouter> {
        self.session_router.read().await
    }

    pub async fn get_session_router_mut(&self) -> RwLockWriteGuard<'_, SessionRouter> {
        self.session_router.write().await
    }
}

#[derive(Debug, Clone, Default)]
pub struct PlatformReconcileReport {
    pub started: Vec<String>,
    pub stopped: Vec<String>,
    pub errors: Vec<(String, String)>,
}
```

在 `message_gateway/platforms/mod.rs` 中添加：`pub mod platform_manager;`

### 2.2 集成到 AppState

**文件**: `src-tauri/src/app_state.rs`

在 `AppState` 中添加：
```rust
pub platform_manager:
    Arc<axagent_runtime::message_gateway::platform_manager::PlatformManager>,
```

### 2.3 初始化 PlatformManager

**文件**: `src-tauri/src/init/state.rs`

```rust
let platform_manager = Arc::new(
    axagent_runtime::message_gateway::platform_manager::PlatformManager::new()
);

// 加到 AppState 构造中
AppState {
    // ... 现有字段
    platform_manager,
    // ...
}
```

### 2.4 启动时执行初次 reconcile

**文件**: `src-tauri/src/init/services.rs`

在 `start_background_services` 中添加：

```rust
fn start_platform_adapters(state: &AppState) {
    let platform_manager = state.platform_manager.clone();
    let db = state.sea_db.clone();

    tauri::async_runtime::spawn(async move {
        let config = axagent_core::repo::platform_config::get_platform_config(&db).await;
        match platform_manager.reconcile(&config).await {
            Ok(report) => {
                if !report.started.is_empty() {
                    tracing::info!("Platform adapters started: {:?}", report.started);
                }
                if !report.errors.is_empty() {
                    for (name, err) in &report.errors {
                        tracing::error!("Platform adapter {} error: {}", name, err);
                    }
                }
            }
            Err(e) => tracing::error!("Platform reconcile failed: {}", e),
        }
    });
}

// 在 start_background_services 中调用
pub fn start_background_services(...) {
    // ... 现有服务
    start_platform_adapters(state);
}
```

### 2.5 创建 Tauri commands（管理适配器生命周期）

**文件**: `src-tauri/src/commands/platform_integration.rs`

添加命令：

```rust
/// 获取所有平台适配器的运行状态
#[tauri::command]
pub async fn get_platform_statuses(
    state: State<'_, AppState>,
) -> Result<Vec<PlatformAdapterStatus>, String> {
    let config = {
        let service = state.platform_integration_service.read().await;
        service.get_config().await
    };
    let running = state.platform_manager.get_running_adapters().await;
    let adapters = state.platform_manager.get_adapters_read().await;

    let mut statuses = Vec::new();
    for (name, adapter) in adapters.iter() {
        statuses.push(PlatformAdapterStatus {
            name: name.clone(),
            enabled: adapter.is_enabled(&config),
            connected: running.contains(name) && adapter.is_connected().await,
            last_activity: None, // 后续可扩展
            active_sessions: state.platform_manager
                .get_session_router().await
                .list_active_sessions().iter()
                .filter(|s| s.platform == *name)
                .count() as i32,
        });
    }
    // 加上 API Server
    statuses.push(PlatformAdapterStatus {
        name: "api_server".to_string(),
        enabled: config.api_server_enabled,
        connected: false, // Phase 3 实现
        last_activity: None,
        active_sessions: 0,
    });
    Ok(statuses)
}

/// 重新应用配置（启动/停止适配器）
#[tauri::command]
pub async fn reconcile_platforms(
    state: State<'_, AppState>,
) -> Result<PlatformReconcileReport, String> {
    let config = {
        let service = state.platform_integration_service.read().await;
        service.get_config().await
    };
    state.platform_manager.reconcile(&config).await.map_err(|e| e.to_string())
}
```

### 2.6 配置更新时自动 reconcile

**文件**: `src-tauri/src/commands/platform_integration.rs`

当 `update_platform_config` 被调用时，在持久化之后自动调用 reconcile：

```rust
#[tauri::command]
pub async fn update_platform_config(
    state: State<'_, AppState>,
    config: PlatformConfig,
) -> Result<PlatformReconcileReport, String> {
    // 持久化
    axagent_core::repo::platform_config::save_platform_config(&state.sea_db, &config)
        .await.map_err(|e| e.to_string())?;
    // 更新内存
    {
        let service = state.platform_integration_service.write().await;
        service.update_config(config.clone()).await;
    }
    // 按新配置调和适配器状态
    state.platform_manager.reconcile(&config).await.map_err(|e| e.to_string())
}
```

### 2.7 前端新增类型

**文件**: `src/types/platform.ts`

```typescript
export interface PlatformAdapterStatus {
  name: string;
  enabled: boolean;
  connected: boolean;
  last_activity: number | null;
  active_sessions: number;
}

export interface PlatformReconcileReport {
  started: string[];
  stopped: string[];
  errors: [string, string][];
}
```

### 2.8 前端 Store 调整

**文件**: `src/stores/feature/platformStore.ts`

- `loadStatuses()` 改为调用 `get_platform_statuses` 命令
- 新增 `reconcile()` 方法调用 `reconcile_platforms`
- `saveConfig()` 的返回值类型改为 `PlatformReconcileReport`，保存后展示启动/停止情况

---

## 3. 实施步骤汇总

| # | 操作 | 文件 | 类型 |
|---|------|------|------|
| 1 | 创建 PlatformManager | `runtime/.../message_gateway/platform_manager.rs` | **新建** |
| 2 | 添加模块声明 | `runtime/.../message_gateway/platforms/mod.rs` | 修改 |
| 3 | AppState 添加 platform_manager | `src/app_state.rs` | 修改 |
| 4 | 初始化 PlatformManager | `init/state.rs` | 修改 |
| 5 | 启动时执行 reconcile | `init/services.rs` | 修改 |
| 6 | 新增状态查询命令 | `commands/platform_integration.rs` | 修改 |
| 7 | update_platform_config 自动 reconcile | `commands/platform_integration.rs` | 修改 |
| 8 | 前端类型新增 | `src/types/platform.ts` | 修改 |
| 9 | 前端 Store 对接新命令 | `src/stores/feature/platformStore.ts` | 修改 |

---

## 4. 不做的事项

- ❌ 不实现消息内容的 AI 处理和回调路由（Phase 3）
- ❌ 不完善各平台的消息接收能力（Phase 5）
- ❌ 不添加前端的配置入口页面（Phase 4）
