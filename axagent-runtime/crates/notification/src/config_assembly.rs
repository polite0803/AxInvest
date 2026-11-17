// SPDX-License-Identifier: AGPL-3.0-only

//! 从用户配置（`AppSettings.notification_channels`）装配出站推送渠道（判据③）。
//!
//! 这是「关一个渠道不动主壳代码路径」的执行层闭环：配置来自 DB 里的
//! `NotificationChannelConfig`，本模块把它翻译成四个 DSA 渠道的具体构造，
//! 并同步到 [`NotificationDispatcher`] 的注册表与路由。**新增/关闭渠道只改配置**，
//! 主壳代码一行不动。
//!
//! 装配幂等：同名渠道覆盖注册（同名即重建），被关掉的渠道注销并从路由剔除。
//! 「先全注销再按配置装配」保证关掉的渠道不会残留在上一次的路由里。

use axagent_harness::types::NotificationChannelConfig;
use axagent_harness::{NotificationChannel, NotificationRoute, RouteConfig};
use std::sync::Arc;

use crate::channels::{
    GotifyChannel, GotifyConfig, NtfyChannel, NtfyConfig, PushPlusChannel, PushPlusConfig,
    ServerChanChannel, ServerChanConfig,
};
use crate::dispatcher::NotificationDispatcher;

/// 把用户配置同步到 dispatcher：注销全部旧渠道 → 装配 enabled 渠道 → 重写报告/告警路由。
///
/// 返回实际装配成功的渠道名（配置有值但缺凭据的渠道会被跳过并记 warn，不伪装就绪）。
pub async fn apply_channel_configs(
    dispatcher: &NotificationDispatcher,
    configs: &[NotificationChannelConfig],
) -> Vec<String> {
    // 幂等起点：清掉所有已注册渠道，避免被关渠道残留在旧路由。
    for name in dispatcher.list_channel_names().await {
        dispatcher.unregister_channel(&name).await;
    }

    let mut active = Vec::new();
    for cfg in configs {
        if !cfg.enabled {
            continue;
        }
        let channel: Option<Arc<dyn NotificationChannel>> = match cfg.id.as_str() {
            "pushplus" if !cfg.token.trim().is_empty() => {
                Some(Arc::new(PushPlusChannel::new(PushPlusConfig {
                    token: cfg.token.clone(),
                    api_url: cfg.api_url.clone(),
                    topic: cfg.topic.clone(),
                })))
            },
            "serverchan" if !cfg.token.trim().is_empty() => {
                Some(Arc::new(ServerChanChannel::new(ServerChanConfig {
                    key: cfg.token.clone(),
                    api_base: cfg.api_url.clone(),
                })))
            },
            "ntfy" if cfg.api_url.as_deref().is_some_and(|s| !s.trim().is_empty()) => {
                Some(Arc::new(NtfyChannel::new(NtfyConfig {
                    server_url: cfg.api_url.clone().unwrap_or_default(),
                    topic: cfg.topic.clone().unwrap_or_default(),
                    token: (!cfg.token.trim().is_empty()).then(|| cfg.token.clone()),
                    default_priority: None,
                })))
            },
            "gotify"
                if !cfg.token.trim().is_empty()
                    && cfg.api_url.as_deref().is_some_and(|s| !s.trim().is_empty()) =>
            {
                Some(Arc::new(GotifyChannel::new(GotifyConfig {
                    server_url: cfg.api_url.clone().unwrap_or_default(),
                    token: cfg.token.clone(),
                    default_priority: None,
                })))
            },
            other => {
                tracing::warn!(
                    target: "axagent_notification::config_assembly",
                    channel = %other,
                    "渠道已启用但缺必需凭据或 id 未知，跳过装配"
                );
                continue;
            },
        };
        if let Some(channel) = channel {
            let name = channel.name().to_string();
            dispatcher.register_channel(channel).await;
            active.push(name);
        }
    }

    // 报告/告警/系统错误三条路由都发往当前 enabled 渠道；无渠道时留空配置
    // （dispatch 侧无 target ⇒ Skipped，与历史"0 渠道不外推"语义一致）。
    for route in
        [NotificationRoute::Report, NotificationRoute::Alert, NotificationRoute::SystemError]
    {
        dispatcher.configure_route(RouteConfig { route, channels: active.clone() }).await;
    }

    active
}

#[cfg(test)]
mod tests {
    use super::*;
    use axagent_harness::types::NotificationChannelConfig;
    use axagent_harness::{AlertPayload, AlertSeverity, NotificationPolicy};
    use chrono::Utc;

    fn cfg(
        id: &str,
        enabled: bool,
        token: &str,
        api_url: Option<&str>,
    ) -> NotificationChannelConfig {
        NotificationChannelConfig {
            id: id.to_string(),
            enabled,
            token: token.to_string(),
            api_url: api_url.map(str::to_string),
            topic: None,
        }
    }

    /// 判据③：配置层——关掉的渠道不装配、enabled 的才装配；重装配幂等清空旧渠道。
    #[tokio::test]
    async fn apply_channel_configs_tracks_enabled_flag() {
        let dispatcher = NotificationDispatcher::new();
        let cfgs = vec![
            cfg("pushplus", true, "token-1", None),
            cfg("gotify", false, "token-2", Some("https://gotify.example.com")),
        ];
        let active = apply_channel_configs(&dispatcher, &cfgs).await;
        assert_eq!(active, vec!["pushplus".to_string()]);
        assert_eq!(dispatcher.list_channel_names().await, vec!["pushplus".to_string()]);

        // 关掉 pushplus → 装配结果与注册表都清空
        let cfgs_off = vec![cfg("pushplus", false, "token-1", None)];
        let active_off = apply_channel_configs(&dispatcher, &cfgs_off).await;
        assert!(active_off.is_empty(), "关渠道后不应有生效渠道");
        assert!(dispatcher.list_channel_names().await.is_empty(), "关渠道后注册表应清空");
    }

    /// 判据③：dispatch 层——被关渠道（注销 + 路由剔除）不再成为分发目标。
    /// 装配层负责「关掉 = 注销 + 路由剔除」，这里用路由级断言证明该语义真实生效：
    /// 两渠道时 total=2；关掉一个后 total=1（不依赖新增渠道实现，避免污染实现者计数）。
    fn alert(title: &str) -> AlertPayload {
        AlertPayload {
            severity: AlertSeverity::Info,
            title: title.to_string(),
            body: "b".to_string(),
            stock_code: None,
            generated_at: Utc::now(),
        }
    }

    #[tokio::test]
    async fn disabled_channel_receives_no_dispatch() {
        let policy =
            NotificationPolicy { dedup_ttl_seconds: 0, cooldown_seconds: 0, ..Default::default() };
        let dispatcher = NotificationDispatcher::with_policy(policy);
        dispatcher.register_channel(Arc::new(crate::dispatcher::LogChannel::new("a", "a"))).await;
        dispatcher.register_channel(Arc::new(crate::dispatcher::LogChannel::new("b", "b"))).await;
        dispatcher
            .configure_route(RouteConfig {
                route: NotificationRoute::Alert,
                channels: vec!["a".to_string(), "b".to_string()],
            })
            .await;

        let s1 = dispatcher.dispatch_alert(&alert("t1"), Utc::now()).await;
        assert_eq!(s1.total, 2, "两渠道都应成为目标");

        // 关闭 b：注销 + 从路由剔除（与 apply_channel_configs 的关闭路径同构）
        dispatcher.unregister_channel("b").await;
        dispatcher
            .configure_route(RouteConfig {
                route: NotificationRoute::Alert,
                channels: vec!["a".to_string()],
            })
            .await;
        let s2 = dispatcher.dispatch_alert(&alert("t2"), Utc::now()).await;
        assert_eq!(s2.total, 1, "被关渠道不应再成为分发目标");
    }
}
