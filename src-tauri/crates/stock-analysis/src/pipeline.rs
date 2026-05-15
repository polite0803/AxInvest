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
            // 优先提供已计算的技术指标（预计算），K线数据作为补充
            if let Some(indicators) = bb.get_state("raw.indicators") {
                let _ = write!(ctx, "## 预计算技术指标\n{indicators}\n\n");
            }
            if let Some(klines) = bb.get_state("raw.klines") {
                let _ = write!(ctx, "## 原始K线数据（最近120日）\n{klines}\n");
            }
            if let Some(score) = bb.get_state("raw.objective_score") {
                let _ = write!(ctx, "## 客观评分参考\n{score}\n");
            }
        },
        "fundamentals-analyst" => {
            if let Some(fin) = bb.get_state("raw.financials") {
                let _ = write!(ctx, "## 财务数据\n{fin}\n");
            }
            // 提供客观评分和估值参考
            if let Some(score) = bb.get_state("raw.objective_score") {
                let _ = write!(ctx, "## 技术面客观评分（参考）\n{score}\n");
            }
            if let Some(quote) = bb.get_state("raw.quote") {
                let _ = write!(ctx, "## 实时行情（PE/PB等估值参考）\n{quote}\n");
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
            // 预计算技术指标和评分（供下游阶段使用）
            if let Some(indicators) = bb.get_state("raw.indicators") {
                let _ = write!(ctx, "## 技术指标\n{indicators}\n\n");
            }
            if let Some(score) = bb.get_state("raw.objective_score") {
                let _ = write!(ctx, "## 客观评分\n{score}\n\n");
            }
            // 规则检查结果
            if let Some(violations) = bb.get_state("rule_check.violations") {
                let _ = write!(ctx, "## 严进规则违规\n{violations}\n\n");
            }
            if let Some(corrections) = bb.get_state("rule_check.corrections") {
                let _ = write!(ctx, "## 规则修正建议\n{corrections}\n\n");
            }
            if let Some(force) = bb.get_state("rule_check.force_signal") {
                let _ = write!(ctx, "## 强制信号\n{force}\n\n");
            }
            // 所有分析师报告
            for field in &[
                "report.market-analyst",
                "report.sentiment-analyst",
                "report.news-analyst",
                "report.fundamentals-analyst",
                "report.policy-analyst",
                "report.hot-money-tracker",
                "report.lockup-watcher",
                "report.bull-researcher",
                "report.bear-researcher",
                "report.research-manager",
                "report.trader",
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

/// 导出黑板的完整快照为 JSON（供持久化和历史回看）
pub async fn export_blackboard_snapshot(
    blackboard: &Arc<RwLock<SharedBlackboard>>,
) -> String {
    let bb = blackboard.read().await;
    let mut snapshot = serde_json::Map::new();
    for (key, value) in &bb.shared_state {
        snapshot.insert(key.clone(), serde_json::Value::String(value.clone()));
    }
    serde_json::to_string(&snapshot).unwrap_or_default()
}
