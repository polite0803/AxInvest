//! mid/long 档出票门（P0 财报预检 + P1 分析回环否决，2026-10-18）
//!
//! ## 为什么门在出票口而不是建池口
//!
//! `build_seed_pool` 的语义是「候选来源」（热门股 ∪ 行业领涨 ∪ 兜底样本），它**跨四档
//! 共享**：超短/短档按构造不看基本面（兑现周期 > 持有期，`portfolio-mgr-h-ultra-short.rhai`
//! 同一裁定），在池上拦掉会把超短档的动量票一起杀掉。故门挂在 `scan_period` 的
//! **mid/long 出票口**：池照建、短档照跑，只有长线语义的档位才要求「活着且没被判卖出」。
//!
//! ## 两条门（`PLAN` 未立先例，判据来自 002696 案例复盘）
//!
//! - **P0 财报门**：最新一期 `net_profit ≤ 0`（首亏）或最新两期 `profit_yoy` 均为负
//!   （连续两期同比下滑）⇒ 拦。取数走 `client.get_financials`（vendor 路由 + 客户端
//!   缓存 + as-of 披露日截断都是既有能力，本门不重复实现）。
//! - **P1 分析回环门**：最近 [`ANALYSIS_VETO_WINDOW_DAYS`] 天内有一条 `completed` 的
//!   live 分析、其 `horizon_decisions` 对**本档**给出 卖出/减持 ⇒ 拦。数据源是
//!   `stock_analyses` 表既有列（加载在 `reco_loop::load_reco_served_vars_db`，
//!   经 `reco_analysis_vetoes` 模板变量传入），不新建表。
//!
//! ## fail-open 纪律
//!
//! 财报取数失败 / 回环变量缺席 ⇒ **不拦**（宁可放过不可错杀，与 liquidity filter
//! 兜底同族），但必须 `warn` 留痕 —— 静默放行会让「门没生效」无法与「门判通过」区分。

use axagent_astock_data::FinancialReport;

use super::types::{GateBlockedPick, Period, RecoPick};

/// P1 回环否决的有效窗口（天）：更早的分析视为过期 —— 财报/行情会变，
/// 三个月前的「卖出」不该永久封杀一只票。与 `stock_analyses.created_at`（毫秒）比较。
pub const ANALYSIS_VETO_WINDOW_DAYS: i64 = 30;

/// 财报门判据（纯函数）。
///
/// 返回 `None` = 通过；`Some(理由码)` = 拦截。判据次序：**首亏优先于连降**
/// （首亏是更强的事实，理由码留给前端/日志区分两种拦截）。
///
/// 口径说明：`net_profit` 是 vendor 给的净利润字段（归母/全口径以 vendor 为准）；
/// 对「≤0 即拦」这条判据两种口径同向，不构成分歧。`report_date` 按字典序即时间序
/// （`YYYY-MM-DD` 形态）。`net_profit` 缺失（None）⇒ 不作首亏判断，落到连降检查。
pub fn financial_gate_reason(reports: &[FinancialReport]) -> Option<&'static str> {
    if reports.is_empty() {
        return None;
    }
    let mut sorted: Vec<&FinancialReport> = reports.iter().collect();
    sorted.sort_by(|a, b| b.report_date.cmp(&a.report_date));
    let latest = sorted[0];
    if matches!(latest.net_profit, Some(np) if np <= 0.0) {
        return Some("first_loss");
    }
    // 连续两期同比下滑：最新与次新两期 profit_yoy 都存在且都为负。
    // 任一期缺 profit_yoy ⇒ 证据不足，不拦（fail-open）。
    let prev = sorted.get(1).and_then(|r| r.profit_yoy);
    let both_down = matches!((latest.profit_yoy, prev), (Some(a), Some(b)) if a < 0.0 && b < 0.0);
    both_down.then_some("two_period_decline")
}

/// P1 回环否决判据（纯函数）。
///
/// `veto_var` 形态（由 `reco_loop::load_analysis_veto_var` 产出，经模板变量注入）：
/// `[{"code":"002696","mid":"减持","long":"卖出"}, ...]` —— **只含有否决档的条目**，
/// 且同一 code 只保留最新一条分析。判据：`period.as_str()` 对应动作 ∈ {卖出, 减持}。
///
/// 语义边界：该 code 有条目但本档无否决动作（键缺席或非卖出/减持）⇒ **不拦并停止
/// 匹配**（最新分析已表态：本档没判卖出）—— 不许继续向后扫旧条目，那会拿旧账翻案。
pub fn analysis_veto_reason(
    veto_var: Option<&serde_json::Value>,
    code: &str,
    period: Period,
) -> Option<String> {
    let arr = veto_var?.as_array()?;
    for entry in arr {
        let entry_code = entry.get("code").and_then(|v| v.as_str())?;
        if entry_code != code {
            continue;
        }
        let action = entry.get(period.as_str()).and_then(|v| v.as_str())?;
        if action == "卖出" || action == "减持" {
            return Some(format!("analysis_veto_{}:{}", period.as_str(), action));
        }
        return None;
    }
    None
}

/// 出票口 glue：就地移除被拦的 pick，返回拦截留痕。
///
/// 只拦**非 synthetic** picks（合成 pick 是「无信号占位」，不是推荐 —— 拦它没有意义，
/// 且占位消失会让空桶兜底语义失效）。同 code 多风格命中时财报只取一次（调用内去重）。
/// 财报取数失败 ⇒ warn + fail-open（该票不拦）。
///
/// ⚠ `veto_var` 的回放守卫在调用方（`scan_period`）：as-of 回放必须传 `None`
/// （回测语义 =「当时的判定」，不带今天的分析否决）。本函数不感知 as-of 上下文
/// —— 守卫放这里会让「time_travel 独立入口」漏防（多入口一处守 = 迟早漂移）。
pub async fn apply_exit_gates(
    client: &axagent_astock_data::AStockClient,
    period: Period,
    picks: &mut Vec<RecoPick>,
    veto_var: Option<&serde_json::Value>,
) -> Vec<GateBlockedPick> {
    if !matches!(period, Period::Mid | Period::Long) {
        return Vec::new();
    }
    // 同 code 财报门判据缓存：一个 code 可能同时在多个风格桶出票，vendor 只问一次。
    let mut fin_cache: std::collections::HashMap<String, Option<&'static str>> =
        std::collections::HashMap::new();
    let mut blocked = Vec::new();
    let mut kept = Vec::with_capacity(picks.len());
    for p in picks.drain(..) {
        if p.synthetic {
            kept.push(p);
            continue;
        }
        let mut reasons: Vec<String> = Vec::new();
        let fin_reason = match fin_cache.get(&p.stock_code) {
            Some(cached) => *cached,
            None => {
                let reason = match client.get_financials(&p.stock_code).await {
                    Ok(reports) => financial_gate_reason(&reports),
                    Err(e) => {
                        tracing::warn!(
                            "[reco_gate] {} 财报取数失败（fail-open 不拦）: {e}",
                            p.stock_code
                        );
                        None
                    },
                };
                fin_cache.insert(p.stock_code.clone(), reason);
                reason
            },
        };
        if let Some(r) = fin_reason {
            reasons.push(r.to_string());
        }
        if let Some(r) = analysis_veto_reason(veto_var, &p.stock_code, period) {
            reasons.push(r);
        }
        if reasons.is_empty() {
            kept.push(p);
        } else {
            tracing::info!(
                "[reco_gate] 拦截 {} {} ({:?} {}): {:?}",
                p.stock_code,
                p.stock_name,
                p.style,
                period.as_str(),
                reasons
            );
            blocked.push(GateBlockedPick {
                stock_code: p.stock_code,
                stock_name: p.stock_name,
                style: p.style,
                confidence: p.confidence as i32,
                reasons,
            });
        }
    }
    *picks = kept;
    blocked
}

#[cfg(test)]
mod tests {
    use super::*;

    fn report(date: &str, net_profit: Option<f64>, profit_yoy: Option<f64>) -> FinancialReport {
        serde_json::from_value(serde_json::json!({
            "stockCode": "002696",
            "reportDate": date,
            "netProfit": net_profit,
            "profitYoy": profit_yoy,
        }))
        .expect("FinancialReport 可从 JSON 构造（camelCase rename）")
    }

    #[test]
    fn first_loss_blocks() {
        // 002696 2026H1 实况形态：最新期亏损（此前盈利）。
        let reports = [
            report("2025-12-31", Some(1.2e8), Some(-3.0)),
            report("2026-06-30", Some(-8.253e7), Some(-800.0)),
        ];
        assert_eq!(financial_gate_reason(&reports), Some("first_loss"));
    }

    #[test]
    fn two_period_decline_blocks() {
        let reports = [
            report("2025-12-31", Some(2.0e8), Some(-10.0)),
            report("2026-06-30", Some(1.5e8), Some(-12.0)),
        ];
        assert_eq!(financial_gate_reason(&reports), Some("two_period_decline"));
    }

    #[test]
    fn healthy_profits_pass() {
        let reports = [
            report("2025-12-31", Some(1.0e8), Some(5.0)),
            report("2026-06-30", Some(1.2e8), Some(8.0)),
        ];
        assert_eq!(financial_gate_reason(&reports), None);
    }

    #[test]
    fn missing_fields_fail_open() {
        // net_profit / profit_yoy 双缺 ⇒ 证据不足不拦。
        let reports = [report("2026-06-30", None, None), report("2025-12-31", None, None)];
        assert_eq!(financial_gate_reason(&reports), None);
        // 空表同理。
        assert_eq!(financial_gate_reason(&[]), None);
        // 只有一期且盈利 ⇒ 无「连续」证据。
        let single = [report("2026-06-30", Some(1.0e8), Some(-1.0))];
        assert_eq!(financial_gate_reason(&single), None);
    }

    #[test]
    fn report_date_ordering_is_descending() {
        // 乱序输入：排序后最新期是 2026-06-30（亏损）⇒ 仍判首亏。
        let reports = [
            report("2025-12-31", Some(1.2e8), Some(1.0)),
            report("2026-06-30", Some(-1.0e7), Some(-100.0)),
            report("2025-06-30", Some(1.0e8), Some(2.0)),
        ];
        assert_eq!(financial_gate_reason(&reports), Some("first_loss"));
    }

    #[test]
    fn veto_matches_only_own_tier() {
        let var = serde_json::json!([{ "code": "002696", "mid": "减持", "long": "卖出" }]);
        assert_eq!(
            analysis_veto_reason(Some(&var), "002696", Period::Long),
            Some("analysis_veto_long:卖出".to_string())
        );
        assert_eq!(
            analysis_veto_reason(Some(&var), "002696", Period::Mid),
            Some("analysis_veto_mid:减持".to_string())
        );
        // 本档无否决 ⇒ 不拦（且不回扫其它条目）。
        assert_eq!(analysis_veto_reason(Some(&var), "002696", Period::Short), None);
        // 无记录 code ⇒ 不拦。
        assert_eq!(analysis_veto_reason(Some(&var), "600519", Period::Long), None);
        // 变量缺席 ⇒ 不拦。
        assert_eq!(analysis_veto_reason(None, "002696", Period::Long), None);
    }

    #[test]
    fn veto_ignores_non_exit_actions() {
        // 最新分析判「观望/买入」⇒ 不得拿旧账拦。
        let var = serde_json::json!([{ "code": "002696", "long": "观望" }]);
        assert_eq!(analysis_veto_reason(Some(&var), "002696", Period::Long), None);
    }
}
