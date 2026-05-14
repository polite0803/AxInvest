use std::sync::Arc;
use tokio::sync::RwLock;

use axagent_agent::shared_blackboard::SharedBlackboard;
use axagent_astock_data::{AStockClient, StockRawData};

use crate::decision::{AnalysisConfig, AnalysisEvent, StockDecision};

/// 股票分析编排器 — 5 阶段执行
pub struct StockAnalysisOrchestrator;

impl StockAnalysisOrchestrator {
    /// 运行完整的 5 阶段分析
    pub async fn run(
        _sessions: &axagent_agent::session_manager::SessionManager,
        data_client: &AStockClient,
        blackboard: Arc<RwLock<SharedBlackboard>>,
        stock_code: String,
        stock_name: String,
        date: String,
        config: AnalysisConfig,
        _provider_id: String,
        _conversation_id: String,
        events: tokio::sync::broadcast::Sender<AnalysisEvent>,
    ) -> Result<StockDecision, String> {
        // 写入基本元数据
        {
            let mut bb = blackboard.write().await;
            bb.set_state("stock_code", &stock_code);
            bb.set_state("stock_name", &stock_name);
            bb.set_state("analysis_date", &date);
        }

        if let Err(e) = events.send(AnalysisEvent::Started {
            stock_code: stock_code.clone(),
            stock_name: stock_name.clone(),
            date: date.clone(),
        }) {
            tracing::warn!("发送分析事件(Started)失败: {}", e);
        }

        // ── 阶段 1: 数据加载 ──
        let _raw = Self::phase_1_load_data(data_client, &stock_code, &config, &blackboard, &events)
            .await
            .map_err(|e| {
                if let Err(send_err) = events.send(AnalysisEvent::Error {
                    stage: "data_loading".into(),
                    message: e.clone(),
                }) {
                    tracing::warn!("发送分析事件(Error)失败: {}", send_err);
                }
                e
            })?;

        // ── 阶段 2-5 占位 — 将在后续 tasks 中通过 SessionManager::run_turn_with_tools 集成 Agent 执行 ──

        if let Err(e) = events.send(AnalysisEvent::Decision(StockDecision {
            action: "持有".to_string(),
            position_pct: 0.0,
            target_price: None,
            stop_loss: None,
            reasoning: "全部分析阶段完成，待 Agent LLM 集成后输出真实决策".to_string(),
            risk_level: "中".to_string(),
            confidence: 0.0,
        })) {
            tracing::warn!("发送分析事件(Decision)失败: {}", e);
        }

        Ok(StockDecision {
            action: "持有".to_string(),
            position_pct: 0.0,
            target_price: None,
            stop_loss: None,
            reasoning: "全部分析阶段完成".to_string(),
            risk_level: "中".to_string(),
            confidence: 0.0,
        })
    }

    async fn phase_1_load_data(
        data_client: &AStockClient,
        stock_code: &str,
        config: &AnalysisConfig,
        blackboard: &Arc<RwLock<SharedBlackboard>>,
        events: &tokio::sync::broadcast::Sender<AnalysisEvent>,
    ) -> Result<StockRawData, String> {
        let raw = data_client
            .fetch_all(stock_code, &config.kline_period, config.kline_limit, config.news_limit)
            .await
            .map_err(|e| format!("数据获取失败: {}", e))?;

        let klines_json = serde_json::to_string(&raw.klines).unwrap_or_default();
        let financials_json = serde_json::to_string(&raw.financials).unwrap_or_default();
        let news_json = serde_json::to_string(&raw.news).unwrap_or_default();
        let money_flow_json = raw
            .money_flow
            .as_ref()
            .map(|m| serde_json::to_string(m).unwrap_or_default())
            .unwrap_or_default();
        let dragon_tiger_json = serde_json::to_string(&raw.dragon_tiger).unwrap_or_default();
        let lockup_json = serde_json::to_string(&raw.lockup).unwrap_or_default();

        {
            let mut bb = blackboard.write().await;
            bb.set_state("raw.klines", &klines_json);
            bb.set_state("raw.financials", &financials_json);
            bb.set_state("raw.news", &news_json);
            bb.set_state("raw.money_flow", &money_flow_json);
            bb.set_state("raw.dragon_tiger", &dragon_tiger_json);
            bb.set_state("raw.lockup", &lockup_json);
        }

        if let Err(e) = events.send(AnalysisEvent::DataLoaded {
            kline_count: raw.klines.len(),
            news_count: raw.news.len(),
        }) {
            tracing::warn!("发送分析事件(DataLoaded)失败: {}", e);
        }

        Ok(raw)
    }
}
