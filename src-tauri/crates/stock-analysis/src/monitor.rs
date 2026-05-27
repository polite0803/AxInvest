use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tokio::time::{interval, Duration};

use axagent_astock_data::{AStockClient, StockQuote};
use tauri::Emitter;

/// 监控条件
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct MonitorConfig {
    pub stock_code: String,
    pub stock_name: String,
    /// 止损价（跌破告警）
    pub stop_loss: Option<f64>,
    /// 止盈价（突破告警）
    pub take_profit: Option<f64>,
    /// 突破压力位告警
    pub resistance_break: Option<f64>,
    /// 跌破支撑位告警
    pub support_break: Option<f64>,
    /// 涨跌幅超过N%告警
    pub change_pct_alert: Option<f64>,
    /// 成交量异常（换手率>N告警）
    pub turnover_rate_alert: Option<f64>,
    /// 是否启用
    pub enabled: bool,
}

/// 告警事件
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MonitorAlert {
    pub stock_code: String,
    pub stock_name: String,
    /// "stop_loss" | "take_profit" | "resistance" | "support" | "change" | "volume"
    pub alert_type: String,
    pub alert_message: String,
    pub current_price: f64,
    pub change_pct: f64,
    pub timestamp: String,
    /// 操作建议（如 "考虑减仓50%，现价低于止损"）
    pub suggested_action: Option<String>,
}

/// 实时监控引擎
pub struct RealtimeMonitor {
    client: Arc<AStockClient>,
    configs: RwLock<HashMap<String, MonitorConfig>>,
    alert_tx: tokio::sync::broadcast::Sender<MonitorAlert>,
    running: RwLock<bool>,
    app_handle: RwLock<Option<tauri::AppHandle>>,
    last_alerts: RwLock<HashMap<String, i64>>,
}

impl RealtimeMonitor {
    pub fn new(client: Arc<AStockClient>) -> Self {
        let (alert_tx, _) = tokio::sync::broadcast::channel(128);
        Self {
            client,
            configs: RwLock::new(HashMap::new()),
            alert_tx,
            running: RwLock::new(false),
            app_handle: RwLock::new(None),
            last_alerts: RwLock::new(HashMap::new()),
        }
    }

    /// 设置 Tauri AppHandle 以桥接告警到前端
    pub async fn set_app_handle(&self, handle: tauri::AppHandle) {
        *self.app_handle.write().await = Some(handle);
    }

    pub fn subscribe(&self) -> tokio::sync::broadcast::Receiver<MonitorAlert> {
        self.alert_tx.subscribe()
    }

    /// 添加监控标的
    pub async fn add_config(&self, config: MonitorConfig) {
        let mut configs = self.configs.write().await;
        configs.insert(config.stock_code.clone(), config.clone());
        let _ = Self::persist_configs(&configs);
    }

    /// 移除监控标的
    pub async fn remove_config(&self, stock_code: &str) {
        let mut configs = self.configs.write().await;
        configs.remove(stock_code);
        let _ = Self::persist_configs(&configs);
    }

    fn persist_configs(configs: &HashMap<String, MonitorConfig>) -> Result<(), String> {
        let json = serde_json::to_string(&configs.values().collect::<Vec<_>>())
            .map_err(|e| e.to_string())?;
        axagent_core::repo::settings::set_setting_sync("monitor_configs", &json)
    }

    pub fn load_configs_from_db() -> Vec<MonitorConfig> {
        axagent_core::repo::settings::get_setting_sync("monitor_configs")
            .ok()
            .flatten()
            .and_then(|json| serde_json::from_str::<Vec<MonitorConfig>>(&json).ok())
            .unwrap_or_default()
    }

    /// 获取所有监控配置
    pub async fn list_configs(&self) -> Vec<MonitorConfig> {
        let configs = self.configs.read().await;
        configs.values().cloned().collect()
    }

    /// 启动监控循环（每30秒轮询一次）
    pub async fn start(&self) {
        {
            let mut running = self.running.write().await;
            if *running {
                return;
            }
            *running = true;
        }

        let mut ticker = interval(Duration::from_secs(30));
        loop {
            ticker.tick().await;
            {
                let running = self.running.read().await;
                if !*running {
                    break;
                }
            }

            let configs = {
                let c = self.configs.read().await;
                c.values().cloned().collect::<Vec<_>>()
            };

            for config in configs {
                if !config.enabled {
                    continue;
                }
                // 非交易时段跳过
                if !axagent_astock_data::calendar::is_trading_time() {
                    continue;
                }

                if let Ok(quote) = self.client.get_quote(&config.stock_code).await {
                    self.check_alerts(&config, &quote).await;
                }
            }
        }
    }

    pub async fn stop(&self) {
        let mut running = self.running.write().await;
        *running = false;
    }

    async fn check_alerts(&self, config: &MonitorConfig, quote: &StockQuote) {
        let mut alerts = Vec::new();
        let now = chrono::Utc::now().format("%Y-%m-%d %H:%M:%S").to_string();

        // 止损检查
        if let Some(stop) = config.stop_loss {
            if quote.price <= stop {
                let suggested =
                    Some(format!("建议: 考虑减仓50%，现价{:.2}低于止损{:.2}", quote.price, stop));
                alerts.push(MonitorAlert {
                    stock_code: config.stock_code.clone(),
                    stock_name: config.stock_name.clone(),
                    alert_type: "stop_loss".into(),
                    alert_message: format!(
                        "跌破止损价 {}: 现价 {:.2} <= 止损 {:.2}",
                        config.stock_name, quote.price, stop
                    ),
                    current_price: quote.price,
                    change_pct: quote.change_pct,
                    timestamp: now.clone(),
                    suggested_action: suggested,
                });
            }
        }

        // 止盈检查
        if let Some(tp) = config.take_profit {
            if quote.price >= tp {
                let suggested =
                    Some(format!("建议: 考虑卖出50%锁利，现价{:.2}≥止盈{:.2}", quote.price, tp));
                alerts.push(MonitorAlert {
                    stock_code: config.stock_code.clone(),
                    stock_name: config.stock_name.clone(),
                    alert_type: "take_profit".into(),
                    alert_message: format!(
                        "突破止盈价 {}: 现价 {:.2} >= 止盈 {:.2}",
                        config.stock_name, quote.price, tp
                    ),
                    current_price: quote.price,
                    change_pct: quote.change_pct,
                    timestamp: now.clone(),
                    suggested_action: suggested,
                });
            }
        }

        // 压力位突破
        if let Some(res) = config.resistance_break {
            if quote.price >= res {
                let suggested = Some(format!("建议: 压力位突破{:.2}，关注持续性和量能配合", res));
                alerts.push(MonitorAlert {
                    stock_code: config.stock_code.clone(),
                    stock_name: config.stock_name.clone(),
                    alert_type: "resistance".into(),
                    alert_message: format!(
                        "突破压力位 {}: 现价 {:.2} >= 压力 {:.2}",
                        config.stock_name, quote.price, res
                    ),
                    current_price: quote.price,
                    change_pct: quote.change_pct,
                    timestamp: now.clone(),
                    suggested_action: suggested,
                });
            }
        }

        // 支撑位跌破
        if let Some(sup) = config.support_break {
            if quote.price <= sup {
                let suggested =
                    Some(format!("建议: 支撑位{:.2}跌破，考虑减仓或设立更严格止损", sup));
                alerts.push(MonitorAlert {
                    stock_code: config.stock_code.clone(),
                    stock_name: config.stock_name.clone(),
                    alert_type: "support".into(),
                    alert_message: format!(
                        "跌破支撑位 {}: 现价 {:.2} <= 支撑 {:.2}",
                        config.stock_name, quote.price, sup
                    ),
                    current_price: quote.price,
                    change_pct: quote.change_pct,
                    timestamp: now.clone(),
                    suggested_action: suggested,
                });
            }
        }

        // 涨跌幅异常
        if let Some(pct) = config.change_pct_alert {
            if quote.change_pct.abs() >= pct {
                let dir = if quote.change_pct > 0.0 { "涨" } else { "跌" };
                let suggested = Some(format!(
                    "建议: 异常{}幅{:.2}%，关注消息面和资金流向",
                    dir, quote.change_pct
                ));
                alerts.push(MonitorAlert {
                    stock_code: config.stock_code.clone(),
                    stock_name: config.stock_name.clone(),
                    alert_type: "change".into(),
                    alert_message: format!(
                        "异常{}幅 {}: {:.2}%",
                        dir, config.stock_name, quote.change_pct
                    ),
                    current_price: quote.price,
                    change_pct: quote.change_pct,
                    timestamp: now.clone(),
                    suggested_action: suggested,
                });
            }
        }

        // 换手率异常
        if let Some(ratio) = config.turnover_rate_alert {
            if quote.turnover_rate >= ratio {
                let suggested =
                    Some(format!("建议: 换手率异常{:.2}%，关注主力进出痕迹", quote.turnover_rate));
                alerts.push(MonitorAlert {
                    stock_code: config.stock_code.clone(),
                    stock_name: config.stock_name.clone(),
                    alert_type: "volume".into(),
                    alert_message: format!(
                        "换手率异常 {}: {:.2}% > {:.2}%",
                        config.stock_name, quote.turnover_rate, ratio
                    ),
                    current_price: quote.price,
                    change_pct: quote.change_pct,
                    timestamp: now.clone(),
                    suggested_action: suggested,
                });
            }
        }

        let now_ts = chrono::Utc::now().timestamp();
        let cooldown_secs: i64 = 300;
        let mut last = self.last_alerts.write().await;
        alerts.retain(|a| {
            let key = format!("{}:{}", a.stock_code, a.alert_type);
            if let Some(&last_ts) = last.get(&key) {
                if now_ts - last_ts < cooldown_secs {
                    return false;
                }
            }
            last.insert(key, now_ts);
            true
        });

        // 发送告警 — 内部 broadcast channel + Tauri 前端事件桥接
        let app_handle = self.app_handle.read().await.clone();
        for alert in &alerts {
            let _ = self.alert_tx.send(alert.clone());
            if let Some(ref app) = app_handle {
                let _ = app.emit(
                    "stock-monitor-alert",
                    serde_json::json!({
                        "stockCode": alert.stock_code,
                        "stockName": alert.stock_name,
                        "alertType": alert.alert_type,
                        "alertMessage": alert.alert_message,
                        "currentPrice": alert.current_price,
                        "changePct": alert.change_pct,
                        "suggestedAction": alert.suggested_action,
                        "timestamp": alert.timestamp,
                    }),
                );
            }
        }
    }
}
