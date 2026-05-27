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
            if let Some(score) = bb.get_state("raw.objective_score") {
                let _ = write!(ctx, "## 技术面客观评分（参考）\n{score}\n");
            }
            if let Some(quote) = bb.get_state("raw.quote") {
                let _ = write!(ctx, "## 实时行情（PE/PB等估值参考）\n{quote}\n");
            }
            if let Some(dividends) = bb.get_state("raw.dividend_records") {
                let _ = write!(ctx, "## 分红记录\n{dividends}\n");
            }
            if let Some(vm) = bb.get_state("raw.value_metrics") {
                let _ = write!(ctx, "## 价值投资指标（系统预计算）\n{vm}\n");
            }
            if let Some(reports) = bb.get_state("raw.research_reports") {
                let _ = write!(ctx, "## 研报列表\n{reports}\n");
            }
            if let Some(eps) = bb.get_state("raw.consensus_eps") {
                let _ = write!(ctx, "## 机构一致预期EPS\n{eps}\n");
            }
        },
        "news-analyst" | "sentiment-analyst" => {
            if let Some(news) = bb.get_state("raw.news") {
                let _ = write!(ctx, "## 新闻数据\n{news}\n");
            }
            if let Some(cls) = bb.get_state("market.cls_flash") {
                let _ = write!(ctx, "## 财联社快讯\n{cls}\n");
            }
        },
        "policy-analyst" => {
            if let Some(news) = bb.get_state("raw.news") {
                let _ = write!(ctx, "## 新闻数据\n{news}\n");
            }
            if let Some(sector) = bb.get_state("raw.sector_info") {
                let _ = write!(ctx, "## 行业分类\n{sector}\n");
            }
            if let Some(hot) = bb.get_state("market.hot_stocks") {
                let _ = write!(ctx, "## 同花顺强势股\n{hot}\n");
            }
            if let Some(industry) = bb.get_state("market.industry_ranking") {
                let _ = write!(ctx, "## 行业横向排名\n{industry}\n");
            }
        },
        "hot-money-tracker" => {
            if let Some(mf) = bb.get_state("raw.money_flow") {
                let _ = write!(ctx, "## 资金流向\n{mf}\n");
            }
            if let Some(dt) = bb.get_state("raw.dragon_tiger") {
                let _ = write!(ctx, "## 龙虎榜\n{dt}\n");
            }
            if let Some(margin) = bb.get_state("raw.margin_data") {
                let _ = write!(ctx, "## 融资融券数据\n{margin}\n");
            }
            if let Some(nb) = bb.get_state("raw.north_bound") {
                let _ = write!(ctx, "## 北向资金持仓\n{nb}\n");
            }
            if let Some(nbf) = bb.get_state("market.north_bound_flow") {
                let _ = write!(ctx, "## 北向资金分钟级流向\n{nbf}\n");
            }
            if let Some(mdt) = bb.get_state("market.market_dragon_tiger") {
                let _ = write!(ctx, "## 全市场龙虎榜\n{mdt}\n");
            }
        },
        "lockup-watcher" => {
            if let Some(lockup) = bb.get_state("raw.lockup") {
                let _ = write!(ctx, "## 限售解禁\n{lockup}\n");
            }
            if let Some(trades) = bb.get_state("raw.shareholder_trades") {
                let _ = write!(ctx, "## 股东增减持记录\n{trades}\n");
            }
        },
        "value-investor" => {
            if let Some(fin) = bb.get_state("raw.financials") {
                let _ = write!(ctx, "## 财务数据\n{fin}\n");
            }
            if let Some(quote) = bb.get_state("raw.quote") {
                let _ = write!(ctx, "## 行情\n{quote}\n");
            }
            if let Some(va) = bb.get_state("value.assessment") {
                let _ = write!(ctx, "## 量化价值评估\n{va}\n");
            }
            if let Some(eps) = bb.get_state("raw.consensus_eps") {
                let _ = write!(ctx, "## 机构一致预期EPS\n{eps}\n");
            }
            if let Some(reports) = bb.get_state("raw.research_reports") {
                let _ = write!(ctx, "## 研报列表\n{reports}\n");
            }
        },
        "research-analyst" => {
            if let Some(reports) = bb.get_state("raw.research_reports") {
                let _ = write!(ctx, "## 研报列表\n{reports}\n");
            }
            if let Some(eps) = bb.get_state("raw.consensus_eps") {
                let _ = write!(ctx, "## 机构一致预期EPS\n{eps}\n");
            }
            if let Some(quote) = bb.get_state("raw.quote") {
                let _ = write!(ctx, "## 实时行情\n{quote}\n");
            }
        },
        "sector-analyst" => {
            if let Some(hot) = bb.get_state("market.hot_stocks") {
                let _ = write!(ctx, "## 同花顺强势股\n{hot}\n");
            }
            if let Some(industry) = bb.get_state("market.industry_ranking") {
                let _ = write!(ctx, "## 行业横向排名\n{industry}\n");
            }
            if let Some(concept) = bb.get_state("raw.concept_blocks") {
                let _ = write!(ctx, "## 概念板块归属\n{concept}\n");
            }
            if let Some(sector) = bb.get_state("raw.sector_info") {
                let _ = write!(ctx, "## 行业分类\n{sector}\n");
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
                "report.research-analyst",
                "report.sector-analyst",
                "report.value-investor",
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

    if expert_id == "portfolio-manager" {
        if let Some(score) = bb.get_state("raw.objective_score") {
            let _ = write!(ctx, "\n--- 客观评分 ---\n{}\n", safe_truncate(score, 800));
        }
        if let Some(value) = bb.get_state("value.assessment") {
            let _ = write!(ctx, "\n--- 价值评估 ---\n{}\n", safe_truncate(value, 800));
        }
        if let Some(quality) = bb.get_state("data_quality_summary") {
            let _ = write!(ctx, "\n--- 数据质量 ---\n{}\n", quality);
        }
        if let Some(rule_result) = bb.get_state("rule_check.result") {
            let _ = write!(ctx, "\n--- 规则检查 ---\n{}\n", rule_result);
        }
        if let Some(rule_corrections) = bb.get_state("rule_check.corrections") {
            let _ = write!(ctx, "\n--- 规则修正建议 ---\n{}\n", rule_corrections);
        }
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
pub async fn export_blackboard_snapshot(blackboard: &Arc<RwLock<SharedBlackboard>>) -> String {
    let bb = blackboard.read().await;
    let mut snapshot = serde_json::Map::new();
    for (key, value) in &bb.shared_state {
        snapshot.insert(key.clone(), serde_json::Value::String(value.clone()));
    }
    serde_json::to_string(&snapshot).unwrap_or_default()
}

fn safe_truncate(s: &str, max_len: usize) -> &str {
    if s.len() <= max_len {
        return s;
    }
    let mut end = max_len;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}
