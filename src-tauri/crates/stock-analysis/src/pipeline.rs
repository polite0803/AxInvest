use std::fmt::Write;
use std::sync::Arc;
use tokio::sync::RwLock;

use crate::decision::AnalysisEvent;
use axagent_agent::shared_blackboard::SharedBlackboard;

pub async fn build_analyst_context(
    expert_id: &str,
    blackboard: &Arc<RwLock<SharedBlackboard>>,
) -> String {
    let bb = blackboard.read().await;

    let mut ctx = String::new();
    let _ = write!(ctx, "你是 {expert_id}\n\n");
    ctx.push_str("以下是你需要分析的原始数据：\n\n");

    match expert_id {
        "market-analyst" => {
            if let Some(klines) = bb.get_state("raw.klines") {
                let _ = write!(ctx, "## K线数据\n{klines}\n");
            }
        },
        "fundamentals-analyst" => {
            if let Some(fin) = bb.get_state("raw.financials") {
                let _ = write!(ctx, "## 财务数据\n{fin}\n");
            }
        },
        "news-analyst" | "sentiment-analyst" | "policy-analyst" => {
            if let Some(news) = bb.get_state("raw.news") {
                let _ = write!(ctx, "## 新闻数据\n{news}\n");
            }
        },
        "hot-money-tracker" => {
            if let Some(mf) = bb.get_state("raw.money_flow") {
                let _ = write!(ctx, "## 资金流向\n{mf}\n");
            }
            if let Some(dt) = bb.get_state("raw.dragon_tiger") {
                let _ = write!(ctx, "## 龙虎榜\n{dt}\n");
            }
        },
        "lockup-watcher" => {
            if let Some(lockup) = bb.get_state("raw.lockup") {
                let _ = write!(ctx, "## 限售解禁\n{lockup}\n");
            }
        },
        _ => {
            for field in &[
                "report.market-analyst",
                "report.sentiment-analyst",
                "report.news-analyst",
                "report.fundamentals-analyst",
                "report.policy-analyst",
                "report.hot-money-tracker",
                "report.lockup-watcher",
            ] {
                if let Some(val) = bb.get_state(field) {
                    let _ = write!(ctx, "\n---\n{val}\n");
                }
            }
        },
    }

    ctx
}

pub async fn write_report(
    expert_id: &str,
    report: &str,
    blackboard: &Arc<RwLock<SharedBlackboard>>,
    events: &tokio::sync::broadcast::Sender<AnalysisEvent>,
) {
    {
        let mut bb = blackboard.write().await;
        bb.set_state(&format!("report.{expert_id}"), report);
    }
    if let Err(e) = events.send(AnalysisEvent::AnalystReport {
        expert_id: expert_id.to_string(),
        report_text: report.to_string(),
    }) {
        tracing::warn!("发送分析事件(AnalystReport)失败: {e}");
    }
}
