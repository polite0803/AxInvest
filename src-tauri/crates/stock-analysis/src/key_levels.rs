use sea_orm::{ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, QueryOrder, QuerySelect};
use std::sync::Arc;

use axagent_astock_data::AStockClient;

/// 关键价位快照
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KeyLevelSnapshot {
    pub id: String,
    pub stock_code: String,
    pub stock_name: String,
    pub analysis_id: String,
    /// 分析日期
    pub snapshot_date: String,
    /// 支撑位
    pub support_level: f64,
    /// 压力位
    pub resistance_level: f64,
    /// 建议止损
    pub stop_loss: Option<f64>,
    /// 建议止盈
    pub take_profit: Option<f64>,
    /// 建议入场
    pub entry_price: Option<f64>,
    /// 快照时价格
    pub price_at_snapshot: f64,
    /// 回测命中统计
    pub hit_support_1d: Option<bool>,
    pub hit_resistance_1d: Option<bool>,
    pub hit_stop_loss_1d: Option<bool>,
    pub hit_take_profit_1d: Option<bool>,
    pub hit_support_3d: Option<bool>,
    pub hit_resistance_3d: Option<bool>,
    pub hit_support_5d: Option<bool>,
    pub hit_resistance_5d: Option<bool>,
    pub created_at: i64,
}

/// 关键价位追踪器
pub struct KeyLevelTracker {
    db: Arc<DatabaseConnection>,
    client: Arc<AStockClient>,
}

impl KeyLevelTracker {
    pub fn new(db: Arc<DatabaseConnection>, client: Arc<AStockClient>) -> Self {
        Self { db, client }
    }

    /// 从分析结果中提取关键价位并保存快照
    #[allow(clippy::too_many_arguments)]
    pub async fn capture_snapshot(
        &self,
        analysis_id: &str,
        _stock_code: &str,
        _stock_name: &str,
        support_level: f64,
        resistance_level: f64,
        stop_loss: Option<f64>,
        take_profit: Option<f64>,
        entry_price: Option<f64>,
        current_price: f64,
    ) -> Result<String, String> {
        let id = uuid::Uuid::new_v4().to_string();
        let today = chrono::Utc::now().format("%Y-%m-%d").to_string();

        use axagent_core::entity::stock_analyses;
        let snapshot_json = serde_json::json!({
            "support": support_level,
            "resistance": resistance_level,
            "stop_loss": stop_loss,
            "take_profit": take_profit,
            "entry_price": entry_price,
            "price_at_snapshot": current_price,
            "snapshot_date": today,
        });

        stock_analyses::Entity::update_many()
            .col_expr(
                stock_analyses::Column::BlackboardSnapshot,
                sea_orm::sea_query::Expr::value(snapshot_json.to_string()),
            )
            .filter(stock_analyses::Column::Id.eq(analysis_id))
            .exec(self.db.as_ref())
            .await
            .map_err(|e| e.to_string())?;

        Ok(id)
    }

    /// 回测历史快照命中率
    pub async fn backtest_key_levels(
        &self,
        _lookback_days: u32,
    ) -> Result<KeyLevelBacktestStats, String> {
        use axagent_core::entity::stock_analyses;
        let analyses = stock_analyses::Entity::find()
            .filter(stock_analyses::Column::Status.eq("completed"))
            .filter(stock_analyses::Column::BlackboardSnapshot.is_not_null())
            .order_by_desc(stock_analyses::Column::CreatedAt)
            .limit(Some(100))
            .all(self.db.as_ref())
            .await
            .map_err(|e| e.to_string())?;

        let mut stats = KeyLevelBacktestStats {
            total_snapshots: analyses.len() as u32,
            ..Default::default()
        };

        for analysis in analyses {
            if let Some(snapshot_str) = analysis.blackboard_snapshot.as_deref() {
                if let Ok(snapshot) = serde_json::from_str::<serde_json::Value>(snapshot_str) {
                    let support = snapshot["support"].as_f64().unwrap_or(0.0);
                    let resistance = snapshot["resistance"].as_f64().unwrap_or(0.0);
                    let stop_loss = snapshot["stop_loss"].as_f64();
                    let take_profit = snapshot["take_profit"].as_f64();
                    let snapshot_date = snapshot["snapshot_date"].as_str().unwrap_or("");

                    // 获取快照日期之后的K线
                    if let Ok(klines) = self
                        .client
                        .get_klines(&analysis.stock_code, "daily", 250)
                        .await
                    {
                        let future: Vec<_> = klines
                            .iter()
                            .filter(|k| k.date.as_str() > snapshot_date)
                            .collect();

                        let day1 = future.first();
                        let day3 = future.get(2); // index 2 = 第3天
                        let day5 = future.get(4); // index 4 = 第5天

                        if let Some(k) = day1 {
                            if k.low <= support {
                                stats.support_hit_1d += 1;
                            }
                            if k.high >= resistance {
                                stats.resistance_hit_1d += 1;
                            }
                            if let Some(sl) = stop_loss {
                                if k.low <= sl {
                                    stats.stop_loss_hit_1d += 1;
                                }
                            }
                            if let Some(tp) = take_profit {
                                if k.high >= tp {
                                    stats.take_profit_hit_1d += 1;
                                }
                            }
                        }
                        if let Some(k) = day3 {
                            if k.low <= support {
                                stats.support_hit_3d += 1;
                            }
                            if k.high >= resistance {
                                stats.resistance_hit_3d += 1;
                            }
                        }
                        if let Some(k) = day5 {
                            if k.low <= support {
                                stats.support_hit_5d += 1;
                            }
                            if k.high >= resistance {
                                stats.resistance_hit_5d += 1;
                            }
                        }
                    }
                }
            }
        }

        Ok(stats)
    }
}

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KeyLevelBacktestStats {
    pub total_snapshots: u32,
    pub support_hit_1d: u32,
    pub resistance_hit_1d: u32,
    pub stop_loss_hit_1d: u32,
    pub take_profit_hit_1d: u32,
    pub support_hit_3d: u32,
    pub resistance_hit_3d: u32,
    pub support_hit_5d: u32,
    pub resistance_hit_5d: u32,
}
