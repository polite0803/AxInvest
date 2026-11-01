//! mid/long 档的财报**标注**（P0，2026-10-18 立 / 2026-10-20 退成标注 / 2026-11-01 撤掉 P1）
//!
//! ## 为什么挂在出票口而不是建池口
//!
//! `build_seed_pool` 的语义是「候选来源」（热门股 ∪ 行业领涨 ∪ 两源都没给票时才并入的
//! 兜底样本）。池**跨四档共享**：超短/短档按构造不看基本面（兑现周期 > 持有期，
//! `portfolio-mgr-h-ultra-short.rhai` 同一裁定），在池上拦掉会把超短档的动量票一起杀掉。
//! 故标注挂在 `scan_period` 的 **mid/long 出票口**：池照建、短档照跑。
//!
//! ## P1 分析回环否决已整体删除（2026-11-01 架构裁定）
//!
//! 原 P1：最近 30 天内有一条 `completed` 的 live 分析、其 `horizon_decisions` 对本档给出
//! 卖出/减持 ⇒ 拦掉该票。删它不是因为不管用，而是因为**它让荐股不再是独立决策**：留弃由
//! 另一条链（LLM 辩论 + 终审）对同一只票的结论决定 ⇒ 面板上的「系统推荐」实际是「分析说买
//! 就在场、分析说卖就消失」，胜率与 IC 也跟着变成同源读数。同批删除的还有
//! `watchlist::apply_analysis_tier`（按本档结论把占位票升级成真实票）与趋势智选候选注入荐股池。
//!
//! 裁定口径：**荐股的候选来源、留弃与分数只能来自市场数据**。
//!
//! ## P0 保留，但只是标注（不是门）
//!
//! 最新一期 `net_profit ≤ 0`（首亏）或最新两期 `profit_yoy` 均为负 ⇒ 往该 pick 的
//! `risk_notes` 写一条「财报预检：…（仅提示）」。取数走 `client.get_financials`
//! （vendor 路由 + 缓存 + as-of 披露日截断是既有能力）⇒ **数据源是财务数据本身，不是另一条
//! 链的结论**，所以不违背上面的口径。
//!
//! 为什么只是标注（2026-10-20 自决，判据至今成立）：① 立门的唯一动机案例（002696）本身证伪
//! 了它——那只票在判「亏损股不该做中线」后约 +65%；② 零后验支持：门上线至今没有任何读数显示
//! 首亏股在中线/长线前向收益更差；③ 类别型硬否决必须有后验读数支撑，否则只能当标注。若日后
//! 确有显著劣化，恢复硬拦的根据是读数，不是个案。
//!
//! ## fail-open 纪律
//!
//! 财报取数失败 ⇒ 跳过标注并 `warn` 留痕：静默会让「没取到」与「没问题」无法区分。

use axagent_astock_data::FinancialReport;

use super::types::{Period, RecoPick};

/// 财报预检判据（纯函数）。
///
/// 返回 `None` = 无风险；`Some(理由码)` = 命中风险。**只用于标注**（写进
/// `risk_notes`），不再作为否决 —— 为什么见模块文档「P0 为什么从硬否决退成标注」。
///
/// 判据次序：**首亏优先于连降**（首亏是更强的事实，理由码留给前端/日志区分两种形态）。
///
/// 口径说明：`net_profit` 是 vendor 给的净利润字段（归母/全口径以 vendor 为准）；
/// 对「≤0 即命中」这条判据两种口径同向，不构成分歧。`report_date` 按字典序即时间序
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

/// 出票口 glue：mid/long 档给命中财报预检的票写一条风险**标注**（不改留弃、不改分数）。
///
/// 只处理**非 synthetic** picks：合成 pick 是「无信号占位」而不是推荐，给它写财报提示没有
/// 意义。同 code 多风格命中时财报只取一次（调用内去重）。取数失败 ⇒ warn 并跳过标注。
///
/// 2026-11-01：原来这里还有一道 P1 分析回环否决（就地移除被分析判卖出/减持的票），
/// 已按「荐股只允许市场数据」的裁定整体删除 ⇒ 本函数不再返回任何拦截留痕。
pub async fn annotate_financial_risk(
    client: &axagent_astock_data::AStockClient,
    period: Period,
    picks: &mut [RecoPick],
) {
    if !matches!(period, Period::Mid | Period::Long) {
        return;
    }
    // 同 code 财报判据缓存：一个 code 可能同时在多个风格桶出票，vendor 只问一次。
    let mut fin_cache: std::collections::HashMap<String, Option<&'static str>> =
        std::collections::HashMap::new();
    for p in picks.iter_mut() {
        if p.synthetic {
            continue;
        }
        // 财报预检：**只标注**，不改变留弃（模块文档：类别型硬否决必须有后验读数支撑）。
        let fin_reason = match fin_cache.get(&p.stock_code) {
            Some(cached) => *cached,
            None => {
                let reason = match client.get_financials(&p.stock_code).await {
                    Ok(reports) => financial_gate_reason(&reports),
                    Err(e) => {
                        tracing::warn!(
                            "[reco_gate] {} 财报取数失败（跳过标注，不影响出票）: {e}",
                            p.stock_code
                        );
                        None
                    },
                };
                fin_cache.insert(p.stock_code.clone(), reason);
                reason
            },
        };
        let Some(code) = fin_reason else {
            continue;
        };
        let label = match code {
            "first_loss" => "最新一期首亏",
            "two_period_decline" => "连续两期同比下滑",
            other => other,
        };
        tracing::debug!(
            "[reco_gate] 财报预检标注 {} {} ({:?} {}): {}",
            p.stock_code,
            p.stock_name,
            p.style,
            period.as_str(),
            code
        );
        p.risk_notes.push(format!("财报预检：{label}（仅提示，本门不剔除）"));
    }
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
}
