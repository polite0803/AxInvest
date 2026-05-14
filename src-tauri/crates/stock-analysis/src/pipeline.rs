use std::sync::Arc;
use tokio::sync::RwLock;

use crate::decision::AnalysisEvent;
use axagent_agent::shared_blackboard::SharedBlackboard;

/// 构建分析师的数据上下文（从 Blackboard 中提取相关数据）
pub async fn build_analyst_context(
    expert_id: &str,
    blackboard: &Arc<RwLock<SharedBlackboard>>,
) -> String {
    let bb = blackboard.read().await;

    let mut ctx = String::new();
    ctx.push_str(&format!("你是 {}\n\n", expert_id));
    ctx.push_str("以下是你需要分析的原始数据：\n\n");

    match expert_id {
        "market-analyst" => {
            if let Some(klines) = bb.get_state("raw.klines") {
                ctx.push_str(&format!("## K线数据\n{}\n", klines));
            }
        },
        "fundamentals-analyst" => {
            if let Some(fin) = bb.get_state("raw.financials") {
                ctx.push_str(&format!("## 财务数据\n{}\n", fin));
            }
        },
        "news-analyst" | "sentiment-analyst" | "policy-analyst" => {
            if let Some(news) = bb.get_state("raw.news") {
                ctx.push_str(&format!("## 新闻数据\n{}\n", news));
            }
        },
        "hot-money-tracker" => {
            if let Some(mf) = bb.get_state("raw.money_flow") {
                ctx.push_str(&format!("## 资金流向\n{}\n", mf));
            }
            if let Some(dt) = bb.get_state("raw.dragon_tiger") {
                ctx.push_str(&format!("## 龙虎榜\n{}\n", dt));
            }
        },
        "lockup-watcher" => {
            if let Some(lockup) = bb.get_state("raw.lockup") {
                ctx.push_str(&format!("## 限售解禁\n{}\n", lockup));
            }
        },
        _ => {
            // 辩论/风控/决策角色读取所有报告
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
                    ctx.push_str(&format!("\n---\n{}\n", val));
                }
            }
        },
    }

    ctx
}

/// 写报告到 Blackboard
pub async fn write_report(
    expert_id: &str,
    report: &str,
    blackboard: &Arc<RwLock<SharedBlackboard>>,
    events: &tokio::sync::broadcast::Sender<AnalysisEvent>,
) {
    {
        let mut bb = blackboard.write().await;
        bb.set_state(&format!("report.{}", expert_id), report);
    }
    events
        .send(AnalysisEvent::AnalystReport {
            expert_id: expert_id.to_string(),
            report_text: report.to_string(),
        })
        .ok();
}
