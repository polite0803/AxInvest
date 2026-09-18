// SPDX-License-Identifier: AGPL-3.0-only

//! 决策仪表盘报告渲染器
//!
//! 借鉴 daily_stock_analysis 的推送格式，把 harness 层的 DashboardReport /
//! MarketReviewReport / DashboardDigest 渲染为 Markdown / HTML。
//!
//! 7 段式结构：核心结论 / 评分 / 趋势 / 买卖点位 / 风险警报 / 催化因素 / 操作检查清单
//! + 大盘复盘模板（主要指数 / 市场概况 / 板块表现）

use crate::decision_action::{normalize_action, ActionKind};
use axagent_harness::{DashboardDigest, DashboardReport, MarketReviewReport};

// ── 辅助函数 ──

/// 根据动作返回对应 emoji（借鉴 DSA 的 emoji 风格）
fn action_emoji(action: &str) -> &'static str {
    // P1-6(2026-09-14): 改走统一归一化 —— 原判据只认中文，英文值域一律渲染 ❓。
    // ⚠️ 已知收敛损失：dashboard 值域的「强烈买入」与「买入」归一化后同为 ActionKind::Buy，
    //    🚀 与 📈 的区分**丢失**（本 crate 的权威档位没有 strong-buy 这一档）。
    //    要恢复该区分应加独立的「强度」字段，而不是在值域里再加一个档 —— 那正是本次要消灭的形态。
    match normalize_action(action) {
        Some(ActionKind::Buy) => "📈",
        Some(ActionKind::Increase) => "⬆️",
        Some(ActionKind::Hold) => "➡️",
        Some(ActionKind::Wait) => "⏸️",
        Some(ActionKind::Reduce) => "⬇️",
        Some(ActionKind::Sell) => "📉",
        Some(ActionKind::Uncertain) => "🤔",
        Some(ActionKind::Unavailable) | None => "❓",
    }
}

/// 根据趋势返回对应 emoji
fn trend_emoji(trend: &str) -> &'static str {
    match trend {
        "看多" => "🐂",
        "看空" => "🐻",
        "震荡" => "⚖️",
        _ => "❓",
    }
}

/// 根据风险等级返回对应 emoji
fn severity_emoji(severity: &str) -> &'static str {
    match severity {
        "高" => "🔴",
        "中" => "🟡",
        "低" => "🟢",
        _ => "⚪",
    }
}

/// 根据催化方向返回对应 emoji
fn direction_emoji(direction: &str) -> &'static str {
    match direction {
        "利好" => "✨",
        "利空" => "⚠️",
        _ => "➖",
    }
}

/// 格式化可选 f64，None 返回 "—"
fn fmt_opt_f64(v: Option<f64>) -> String {
    v.map(|x| format!("{x:.2}")).unwrap_or_else(|| "—".into())
}

/// 转义 HTML 特殊字符
fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

// ── Markdown 渲染 ──

/// 渲染单只股票决策仪表盘为 Markdown
///
/// 输出格式（借鉴 DSA 推送）：
/// ```markdown
/// # 🎯 贵州茅台(600519) 决策仪表盘
/// 📅 2026-07-16 | 🤖 glm-5.2
///
/// ## 核心结论
/// 📈 买入 | 🐂 看多 | 📊 评分 75/100 | 🎯 置信度 80%
/// 白酒龙头，业绩稳健
///
/// ## 买卖点位
/// - 买入区间: 1680.00 - 1720.00
/// - 目标价: 1900.00
/// - 止损价: 1600.00
/// - 建议仓位: 30%
///
/// ## 🚨 风险警报
/// - 🟡 [技术面] 短期获利盘压力
///
/// ## ✨ 催化因素
/// - ✨ [利好/短期] 中秋旺季需求 (置信度 75%)
///
/// ## ✅ 操作检查清单
/// - [ ] 入场: 确认放量突破
/// ```
pub fn render_dashboard_md(report: &DashboardReport) -> String {
    let mut md = String::with_capacity(2048);

    // 标题
    md.push_str(&format!(
        "# {} {}({}) 决策仪表盘\n\n",
        action_emoji(&report.action),
        report.stock_name,
        report.stock_code
    ));
    md.push_str(&format!("📅 {} | ", report.analysis_date));
    if let Some(model) = &report.llm_model {
        md.push_str(&format!("🤖 {model}"));
    } else {
        md.push_str("🤖 —");
    }
    md.push_str("\n\n");

    // 1. 核心结论
    md.push_str("## 核心结论\n");
    md.push_str(&format!(
        "{} {} | {} {} | 📊 评分 {}/100 | 🎯 置信度 {:.0}%\n",
        action_emoji(&report.action),
        report.action,
        trend_emoji(&report.trend),
        report.trend,
        report.score,
        report.confidence
    ));
    md.push_str(&report.core_conclusion);
    md.push_str("\n\n");

    // 2. 买卖点位
    md.push_str("## 买卖点位\n");
    match (report.buy_point_low, report.buy_point_high) {
        (Some(low), Some(high)) => {
            md.push_str(&format!("- 买入区间: {low:.2} - {high:.2}\n"));
        },
        (Some(low), None) => {
            md.push_str(&format!("- 买入价: {low:.2}\n"));
        },
        _ => {},
    }
    // 2026-09-13: 「交易目标价」与「内在价值」分离标注 —— 前者是 LLM trader 的方向性目标
    // （持有/观望档常为空或等于现价），后者是 `t-valuation` 客观计算的 DCF 三档。
    // 两者曾同名展示为「目标价」⇒ 用户读到「同一工作流结论矛盾」（603466 风语筑实证）。
    md.push_str(&format!("- 交易目标价: {}\n", fmt_opt_f64(report.target_price)));
    md.push_str(&format!("- 止损价: {}\n", fmt_opt_f64(report.stop_loss)));
    if let (Some(low), Some(high)) = (report.intrinsic_value_low, report.intrinsic_value_high) {
        let cp = report.current_price.map(|p| format!("（现价 {p:.2}）")).unwrap_or_default();
        md.push_str(&format!("- 内在价值(DCF): {low:.2} - {high:.2}{cp}\n"));
    }
    md.push_str(&format!("- 建议仓位: {:.0}%\n\n", report.position_pct));

    // 3. 风险警报
    if !report.risk_alerts.is_empty() {
        md.push_str("## 🚨 风险警报\n");
        for alert in &report.risk_alerts {
            let source = alert.source.as_deref().map(|s| format!("[{s}] ")).unwrap_or_default();
            md.push_str(&format!(
                "- {} {}{}\n",
                severity_emoji(&alert.severity),
                source,
                alert.description
            ));
        }
        md.push('\n');
    }

    // 4. 催化因素
    if !report.catalysts.is_empty() {
        md.push_str("## ✨ 催化因素\n");
        for cat in &report.catalysts {
            let timeline = cat.timeline.as_deref().map(|t| format!("/{t}")).unwrap_or_default();
            let conf =
                cat.confidence_score.map(|c| format!(" (置信度 {c:.0}%)")).unwrap_or_default();
            md.push_str(&format!(
                "- {} [{}/{}] {}{}\n",
                direction_emoji(&cat.direction),
                cat.direction,
                timeline.trim_start_matches('/'),
                cat.description,
                conf
            ));
        }
        md.push('\n');
    }

    // 5. 操作检查清单
    if !report.checklist.is_empty() {
        md.push_str("## ✅ 操作检查清单\n");
        for item in &report.checklist {
            let mark = if item.checked { "x" } else { " " };
            md.push_str(&format!("- [{mark}] {}: {}\n", item.category, item.description));
        }
        md.push('\n');
    }

    // 6. 最新动态
    if let Some(news) = &report.latest_news {
        md.push_str("## 📰 最新动态\n");
        md.push_str(news);
        md.push_str("\n\n");
    }

    // 7. 业绩预期
    if let Some(earnings) = &report.earnings_expectation {
        md.push_str("## 💰 业绩预期\n");
        md.push_str(earnings);
        md.push_str("\n\n");
    }

    // 完整性标记
    if !report.integrity_passed {
        md.push_str("⚠️ 报告未通过完整性校验，请核对缺失字段\n");
    }

    md
}

/// 渲染大盘复盘为 Markdown
pub fn render_market_review_md(review: &MarketReviewReport) -> String {
    let mut md = String::with_capacity(1024);

    md.push_str(&format!("# 📊 大盘复盘 {}\n\n", review.review_date));

    // 主要指数
    if !review.indices.is_empty() {
        md.push_str("## 主要指数\n");
        md.push_str("| 指数 | 点位 | 涨跌幅 |\n");
        md.push_str("|------|------|--------|\n");
        for idx in &review.indices {
            let sign = if idx.change_pct >= 0.0 { "+" } else { "" };
            md.push_str(&format!(
                "| {} | {:.2} | {}{:.2}% |\n",
                idx.name, idx.price, sign, idx.change_pct
            ));
        }
        md.push('\n');
    }

    // 市场概况
    md.push_str("## 市场概况\n");
    if let Some(adv) = review.advancers {
        md.push_str(&format!("- 上涨: {adv}"));
        if let Some(dec) = review.decliners {
            md.push_str(&format!(" | 下跌: {dec}"));
        }
        md.push('\n');
    }
    if let Some(lu) = review.limit_up {
        md.push_str(&format!("- 涨停: {lu}"));
        if let Some(ld) = review.limit_down {
            md.push_str(&format!(" | 跌停: {ld}"));
        }
        md.push('\n');
    }
    md.push('\n');

    // 板块表现
    if !review.sector_leaders.is_empty() {
        md.push_str("## 领涨板块\n");
        md.push_str(&review.sector_leaders.join(" / "));
        md.push_str("\n\n");
    }
    if !review.sector_laggards.is_empty() {
        md.push_str("## 领跌板块\n");
        md.push_str(&review.sector_laggards.join(" / "));
        md.push_str("\n\n");
    }

    md
}

/// 渲染聚合仪表盘（多只股票汇总）为 Markdown
pub fn render_dashboard_digest_md(digest: &DashboardDigest) -> String {
    let mut md = String::with_capacity(2048);

    md.push_str(&format!("# 📋 决策仪表盘汇总 {}\n\n", digest.digest_date));
    md.push_str(&format!(
        "总计 {} 只 | 📈 买入 {} | ➡️ 观望 {} | 📉 卖出 {}\n\n",
        digest.total_count, digest.buy_count, digest.watch_count, digest.sell_count
    ));

    // 大盘复盘（可选）
    if let Some(review) = &digest.market_review {
        md.push_str(&render_market_review_md(review));
        md.push_str("---\n\n");
    }

    // 摘要列表
    if !digest.summaries.is_empty() {
        md.push_str("## 个股概览\n");
        md.push_str("| 代码 | 名称 | 动作 | 评分 | 趋势 | 置信度 |\n");
        md.push_str("|------|------|------|------|------|--------|\n");
        for s in &digest.summaries {
            md.push_str(&format!(
                "| {} | {} | {} {} | {}/100 | {} {} | {:.0}% |\n",
                s.stock_code,
                s.stock_name,
                action_emoji(&s.action),
                s.action,
                s.score,
                trend_emoji(&s.trend),
                s.trend,
                s.confidence
            ));
        }
        md.push('\n');
    }

    md
}

// ── HTML 渲染 ──

/// 渲染单只股票决策仪表盘为 HTML（用于邮件/Web 预览）
pub fn render_dashboard_html(report: &DashboardReport) -> String {
    let mut html = String::with_capacity(4096);

    // 标题
    html.push_str(&format!(
        "<h1>{} {}({}) 决策仪表盘</h1>",
        action_emoji(&report.action),
        html_escape(&report.stock_name),
        html_escape(&report.stock_code)
    ));
    html.push_str(&format!(
        "<p>📅 {} | 🤖 {}</p>",
        html_escape(&report.analysis_date),
        html_escape(report.llm_model.as_deref().unwrap_or("—"))
    ));

    // 1. 核心结论
    let score_color = if report.score >= 60 {
        "#3fb950"
    } else if report.score >= 30 {
        "#d29922"
    } else {
        "#f85149"
    };
    html.push_str("<h2>核心结论</h2>");
    html.push_str(&format!(
        "<p>{} <b>{}</b> | {} <b>{}</b> | 📊 评分 <b style=\"color:{score_color}\">{}/100</b> | 🎯 置信度 {:.0}%</p>",
        action_emoji(&report.action),
        html_escape(&report.action),
        trend_emoji(&report.trend),
        html_escape(&report.trend),
        report.score,
        report.confidence
    ));
    html.push_str(&format!("<p>{}</p>", html_escape(&report.core_conclusion)));

    // 2. 买卖点位
    html.push_str("<h2>买卖点位</h2><ul>");
    match (report.buy_point_low, report.buy_point_high) {
        (Some(low), Some(high)) => {
            html.push_str(&format!("<li>买入区间: <b>{low:.2} - {high:.2}</b></li>"));
        },
        (Some(low), None) => {
            html.push_str(&format!("<li>买入价: <b>{low:.2}</b></li>"));
        },
        _ => {},
    }
    // 2026-09-13: 与 md 渲染同源 —— 「交易目标价」vs「内在价值」语义分离
    html.push_str(&format!("<li>交易目标价: <b>{}</b></li>", fmt_opt_f64(report.target_price)));
    html.push_str(&format!(
        "<li>止损价: <b style=\"color:#f85149\">{}</b></li>",
        fmt_opt_f64(report.stop_loss)
    ));
    if let (Some(low), Some(high)) = (report.intrinsic_value_low, report.intrinsic_value_high) {
        let cp = report.current_price.map(|p| format!("（现价 {p:.2}）")).unwrap_or_default();
        html.push_str(&format!(
            "<li>内在价值(DCF): <b style=\"color:#58a6ff\">{low:.2} - {high:.2}</b>{cp}</li>"
        ));
    }
    html.push_str(&format!("<li>建议仓位: <b>{:.0}%</b></li>", report.position_pct));
    html.push_str("</ul>");

    // 3. 风险警报
    if !report.risk_alerts.is_empty() {
        html.push_str("<h2>🚨 风险警报</h2><ul>");
        for alert in &report.risk_alerts {
            let source = alert
                .source
                .as_deref()
                .map(|s| format!("[{}] ", html_escape(s)))
                .unwrap_or_default();
            html.push_str(&format!(
                "<li>{} {}{}</li>",
                severity_emoji(&alert.severity),
                source,
                html_escape(&alert.description)
            ));
        }
        html.push_str("</ul>");
    }

    // 4. 催化因素
    if !report.catalysts.is_empty() {
        html.push_str("<h2>✨ 催化因素</h2><ul>");
        for cat in &report.catalysts {
            let timeline =
                cat.timeline.as_deref().map(|t| format!("/{}", html_escape(t))).unwrap_or_default();
            let conf =
                cat.confidence_score.map(|c| format!(" (置信度 {c:.0}%)")).unwrap_or_default();
            html.push_str(&format!(
                "<li>{} [{}/{}] {}{}</li>",
                direction_emoji(&cat.direction),
                html_escape(&cat.direction),
                timeline.trim_start_matches('/'),
                html_escape(&cat.description),
                conf
            ));
        }
        html.push_str("</ul>");
    }

    // 5. 操作检查清单
    if !report.checklist.is_empty() {
        html.push_str("<h2>✅ 操作检查清单</h2><ul>");
        for item in &report.checklist {
            let mark = if item.checked { "✓" } else { "○" };
            html.push_str(&format!(
                "<li>{mark} <b>[{}]</b> {}</li>",
                html_escape(&item.category),
                html_escape(&item.description)
            ));
        }
        html.push_str("</ul>");
    }

    // 6. 最新动态
    if let Some(news) = &report.latest_news {
        html.push_str(&format!("<h2>📰 最新动态</h2><p>{}</p>", html_escape(news)));
    }

    // 7. 业绩预期
    if let Some(earnings) = &report.earnings_expectation {
        html.push_str(&format!("<h2>💰 业绩预期</h2><p>{}</p>", html_escape(earnings)));
    }

    // 完整性标记
    if !report.integrity_passed {
        html.push_str("<p style=\"color:#f85149\">⚠️ 报告未通过完整性校验，请核对缺失字段</p>");
    }

    html
}

// ── 从决策工作流输出转换 ──

/// 从决策工作流输出 + 分析上下文构建 DashboardReport
///
/// `decision_json` 是 portfolio-mgr 节点输出的 JSON，包含：
/// - action: 决策动作
/// - positionPct: 仓位百分比
/// - confidence: 置信度
/// - reasoning: 决策理由
/// - targetPrice: 目标价（可选）
/// - stopLoss: 止损价（可选）
/// - timeHorizon: 时间维度（可选）
///
/// `score_json` 是评分节点输出的 JSON，包含：
/// - total: 综合评分
/// - trendScore / signal 等
///
/// `stock_code` / `stock_name` / `analysis_date` 来自分析记录元数据。
///
/// `analyst_reports` 是各专家节点的报告文本（key 为 expert_id），用于提取风险警报和催化因素。
///
/// `valuation_json` 是 `t-valuation` 节点输出的**已解包并 parse** 的对象
/// （`{current_price, dcf:{low,mid,high,...}, graham:{...}, ...}`），用于填充
/// **估值语义**的 `intrinsic_value_*` —— 与交易语义的 `target_price` 严格区分
/// （2026-09-13：两者同名展示导致用户读到「同一工作流结论矛盾」，603466 实证）。
/// 传 `None` 时这四个字段保持 `None`，UI 侧按「无估值数据」渲染。
pub fn build_dashboard_report_from_workflow(
    decision_json: &serde_json::Value,
    score_json: &serde_json::Value,
    stock_code: &str,
    stock_name: &str,
    analysis_date: &str,
    analyst_reports: &std::collections::HashMap<String, String>,
    valuation_json: Option<&serde_json::Value>,
) -> DashboardReport {
    use chrono::Utc;

    // P1-6(2026-09-14): 原 `unwrap_or("持有")` 把「决策缺失」吸收成「持有」——缺失不是结论。
    //   改走统一归一化：识别到的档位落规范中文，缺失 / 未识别一律落显式哨兵「数据缺失」，
    //   由展示层渲染「数据缺失」而非任何操作建议。
    let action = decision_json
        .get("action")
        .and_then(|v| v.as_str())
        .and_then(normalize_action)
        .unwrap_or(ActionKind::Unavailable)
        .as_storage_cn()
        .to_string();
    let position_pct = decision_json.get("positionPct").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let confidence = decision_json.get("confidence").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let reasoning =
        decision_json.get("reasoning").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let target_price = decision_json.get("targetPrice").and_then(|v| v.as_f64());
    let stop_loss = decision_json.get("stopLoss").and_then(|v| v.as_f64());

    // ── 估值语义字段（来自 t-valuation 的客观计算，与交易价位严格区分）──
    // 603466 实证：仪表盘「目标价」= 13.27（LLM 自填、等于现价），而估值区间是 4.44–5.57
    // ⇒ 用户读到「同一工作流结论矛盾」。根因是两套语义共用了一个词，故在此显式并列供 UI 分栏。
    let val_dcf = valuation_json.and_then(|v| v.get("dcf"));
    let intrinsic_value_low = val_dcf.and_then(|d| d.get("low")).and_then(|v| v.as_f64());
    let intrinsic_value_high = val_dcf.and_then(|d| d.get("high")).and_then(|v| v.as_f64());
    let intrinsic_value_mid = val_dcf.and_then(|d| d.get("mid")).and_then(|v| v.as_f64());
    let current_price =
        valuation_json.and_then(|v| v.get("current_price")).and_then(|v| v.as_f64());

    let score = score_json.get("total").and_then(|v| v.as_u64()).map(|v| v as u32).unwrap_or(0);
    let trend = score_json
        .get("signal")
        .and_then(|v| v.as_str())
        .map(|s| match s {
            "strong_buy" | "buy" | "hold" => "看多",
            "strong_sell" | "sell" => "看空",
            _ => "震荡",
        })
        .unwrap_or("震荡")
        .to_string();

    // 从专家报告中提取风险警报和催化因素
    let risk_alerts = extract_risk_alerts(analyst_reports);
    let catalysts = extract_catalysts(analyst_reports);

    // 从 reasoning 提取核心结论（取第一句或截断到 100 字符）
    let core_conclusion = if reasoning.is_empty() {
        format!("{action} {stock_name}，置信度 {confidence:.0}%")
    } else {
        // 取第一句（句号/问号/感叹号分割）
        let first_sentence =
            reasoning.split(['。', '！', '？', '.', '!', '?']).next().unwrap_or(&reasoning).trim();
        let conclusion = if first_sentence.is_empty() {
            &reasoning[..reasoning.chars().count().min(120)]
        } else {
            first_sentence
        };
        conclusion.to_string()
    };

    // 构建操作检查清单（根据动作生成默认清单）
    let checklist = build_default_checklist(&action, target_price.is_some(), stop_loss.is_some());

    let mut report = DashboardReport {
        stock_code: stock_code.to_string(),
        stock_name: stock_name.to_string(),
        analysis_date: analysis_date.to_string(),
        generated_at: Utc::now(),
        core_conclusion,
        action,
        score,
        trend,
        confidence,
        buy_point_low: None, // 由调用方根据 K 线支撑位填充
        buy_point_high: None,
        target_price,
        stop_loss,
        position_pct,
        intrinsic_value_low,
        intrinsic_value_high,
        intrinsic_value_mid,
        current_price,
        risk_alerts,
        catalysts,
        checklist,
        latest_news: analyst_reports.get("news-analyst").cloned(),
        earnings_expectation: analyst_reports.get("fundamentals-analyst").cloned(),
        llm_model: None, // 由调用方填充
        integrity_passed: false,
    };

    // 完整性校验 + 占位符补全
    let missing = axagent_harness::validate_dashboard_report(&report);
    if missing.is_empty() {
        report.integrity_passed = true;
    } else {
        tracing::warn!("[dashboard_report] 报告完整性校验失败，缺失字段: {:?}", missing);
        axagent_harness::fill_missing_with_placeholders(&mut report);
    }

    report
}

/// 从专家报告中提取风险警报
///
/// 扫描 risk 类专家报告（policy-analyst / hot-money-tracker / lockup-watcher），
/// 按关键词识别风险等级。
fn extract_risk_alerts(
    analyst_reports: &std::collections::HashMap<String, String>,
) -> Vec<axagent_harness::RiskAlert> {
    let mut alerts = Vec::new();
    let risk_experts = [
        ("policy-analyst", "政策面"),
        ("hot-money-tracker", "资金面"),
        ("lockup-watcher", "解禁面"),
    ];

    for (expert_id, source) in &risk_experts {
        if let Some(report) = analyst_reports.get(*expert_id) {
            // 简单关键词匹配：高/中/低风险
            let severity = if report.contains("高风险") || report.contains("重大风险") {
                "高"
            } else if report.contains("风险") || report.contains("警惕") || report.contains("注意")
            {
                "中"
            } else {
                continue; // 无风险关键词则跳过
            };

            // 提取包含"风险"的句子作为描述。
            // 切分符含真实换行（报告 unwrap 后是 markdown 多行文本），跳过标题行，
            // 避免整段 markdown 表格被当成一个「句子」塞进描述（2026-09-11 光库实证）
            let description = report
                .split(['。', '！', '？', '\n'])
                .map(|s| s.trim())
                .find(|s| {
                    (s.contains("风险") || s.contains("警惕") || s.contains("注意"))
                        && !s.starts_with('#')
                        && s.chars().count() >= 6
                })
                .map(|s| s.trim().to_string())
                .unwrap_or_else(|| format!("{source}存在风险"));

            if !description.is_empty() {
                alerts.push(axagent_harness::RiskAlert {
                    description,
                    severity: severity.to_string(),
                    source: Some(source.to_string()),
                });
            }
        }
    }

    alerts
}

/// 仅供主 crate 集成测试使用（决策仪表盘风险描述提取回归）
#[doc(hidden)]
pub fn __extract_risk_alerts_for_test(
    analyst_reports: &std::collections::HashMap<String, String>,
) -> Vec<axagent_harness::RiskAlert> {
    extract_risk_alerts(analyst_reports)
}

/// 从专家报告中提取催化因素
///
/// 扫描 news-analyst / fundamentals-analyst / market-analyst 报告，
/// 按关键词识别利好/利空催化。
fn extract_catalysts(
    analyst_reports: &std::collections::HashMap<String, String>,
) -> Vec<axagent_harness::Catalyst> {
    let mut catalysts = Vec::new();
    let catalyst_experts = [
        ("news-analyst", "消息面"),
        ("market-analyst", "市场面"),
        ("fundamentals-analyst", "基本面"),
    ];

    for (expert_id, source) in &catalyst_experts {
        if let Some(report) = analyst_reports.get(*expert_id) {
            // 识别利好/利空
            let (direction, keyword) = if report.contains("利好")
                || report.contains("增长")
                || report.contains("突破")
            {
                ("利好", "利好")
            } else if report.contains("利空") || report.contains("下滑") || report.contains("亏损")
            {
                ("利空", "利空")
            } else {
                continue;
            };

            // 提取包含关键词的句子作为描述（切分符含真实换行，跳过标题行，
            // 与 extract_risk_alerts 同规则；2026-09-11 光库实证）
            let description = report
                .split(['。', '！', '？', '\n'])
                .map(|s| s.trim())
                .find(|s| {
                    (s.contains(keyword) || s.contains("增长") || s.contains("下滑"))
                        && !s.starts_with('#')
                        && s.chars().count() >= 6
                })
                .map(|s| s.trim().to_string())
                .unwrap_or_else(|| format!("{source}{direction}"));

            if !description.is_empty() {
                catalysts.push(axagent_harness::Catalyst {
                    description,
                    direction: direction.to_string(),
                    timeline: Some("短期".to_string()),
                    confidence_score: None,
                });
            }
        }
    }

    catalysts
}

/// 根据动作构建默认操作检查清单
fn build_default_checklist(
    action: &str,
    has_target: bool,
    has_stop_loss: bool,
) -> Vec<axagent_harness::ChecklistItem> {
    let mut items = Vec::new();

    // P1-6(2026-09-14): 改走统一归一化 —— 原判据只认中文，一旦链路上是英文 token
    //   或 dashboard 值域短语，is_buy / is_sell 双 false ⇒ 入场/出场检查清单**整段不生成**，
    //   报告缺项且毫无告警。未识别 / 缺失时双 false 是正确行为（没有方向就不该给方向性清单）。
    let kind = normalize_action(action);
    let is_buy = kind.is_some_and(ActionKind::implies_buy);
    let is_sell = matches!(kind, Some(ActionKind::Reduce | ActionKind::Sell));

    if is_buy {
        items.push(axagent_harness::ChecklistItem {
            description: "确认放量突破关键阻力位".into(),
            checked: false,
            category: "入场".into(),
        });
        if has_target {
            items.push(axagent_harness::ChecklistItem {
                description: "目标价已设定".into(),
                checked: true,
                category: "止盈".into(),
            });
        }
        if has_stop_loss {
            items.push(axagent_harness::ChecklistItem {
                description: "止损价已设定".into(),
                checked: true,
                category: "止损".into(),
            });
        }
        items.push(axagent_harness::ChecklistItem {
            description: "分批建仓，避免一次性满仓".into(),
            checked: false,
            category: "入场".into(),
        });
    } else if is_sell {
        items.push(axagent_harness::ChecklistItem {
            description: "确认破位下跌，及时止损".into(),
            checked: false,
            category: "止损".into(),
        });
        items.push(axagent_harness::ChecklistItem {
            description: "分批减仓，避免一次性清仓".into(),
            checked: false,
            category: "减仓".into(),
        });
    } else {
        // 持有
        items.push(axagent_harness::ChecklistItem {
            description: "保持观望，等待信号明确".into(),
            checked: false,
            category: "入场".into(),
        });
    }

    items
}

#[cfg(test)]
mod tests {
    use super::*;
    use axagent_harness::{Catalyst, ChecklistItem, IndexQuote, RiskAlert, StockSummary};
    use chrono::Utc;

    fn make_report() -> DashboardReport {
        DashboardReport {
            stock_code: "600519".into(),
            stock_name: "贵州茅台".into(),
            analysis_date: "2026-07-16".into(),
            generated_at: Utc::now(),
            core_conclusion: "白酒龙头，业绩稳健".into(),
            action: "买入".into(),
            score: 75,
            trend: "看多".into(),
            confidence: 80.0,
            buy_point_low: Some(1680.0),
            buy_point_high: Some(1720.0),
            target_price: Some(1900.0),
            stop_loss: Some(1600.0),
            position_pct: 30.0,
            intrinsic_value_low: None,
            intrinsic_value_high: None,
            intrinsic_value_mid: None,
            current_price: None,
            risk_alerts: vec![RiskAlert {
                description: "短期获利盘压力".into(),
                severity: "中".into(),
                source: Some("技术面".into()),
            }],
            catalysts: vec![Catalyst {
                description: "中秋旺季需求".into(),
                direction: "利好".into(),
                timeline: Some("短期".into()),
                confidence_score: Some(75.0),
            }],
            checklist: vec![ChecklistItem {
                description: "确认放量突破".into(),
                checked: false,
                category: "入场".into(),
            }],
            latest_news: Some("贵州茅台发布半年报".into()),
            earnings_expectation: Some("2026H1 营收同比+15%".into()),
            llm_model: Some("glm-5.2".into()),
            integrity_passed: true,
        }
    }

    #[test]
    fn test_render_md_contains_all_sections() {
        let report = make_report();
        let md = render_dashboard_md(&report);
        assert!(md.contains("决策仪表盘"));
        assert!(md.contains("核心结论"));
        assert!(md.contains("买卖点位"));
        assert!(md.contains("风险警报"));
        assert!(md.contains("催化因素"));
        assert!(md.contains("操作检查清单"));
        assert!(md.contains("最新动态"));
        assert!(md.contains("业绩预期"));
        assert!(md.contains("买入区间: 1680.00 - 1720.00"));
        assert!(md.contains("目标价: 1900.00"));
        assert!(md.contains("止损价: 1600.00"));
        assert!(md.contains("建议仓位: 30%"));
    }

    #[test]
    fn test_render_md_empty_sections_omitted() {
        let mut report = make_report();
        report.risk_alerts.clear();
        report.catalysts.clear();
        report.checklist.clear();
        report.latest_news = None;
        report.earnings_expectation = None;
        let md = render_dashboard_md(&report);
        assert!(!md.contains("风险警报"));
        assert!(!md.contains("催化因素"));
        assert!(!md.contains("操作检查清单"));
        assert!(!md.contains("最新动态"));
        assert!(!md.contains("业绩预期"));
    }

    #[test]
    fn test_render_md_integrity_warning() {
        let mut report = make_report();
        report.integrity_passed = false;
        let md = render_dashboard_md(&report);
        assert!(md.contains("报告未通过完整性校验"));
    }

    #[test]
    fn test_render_html_escapes_special_chars() {
        let mut report = make_report();
        report.core_conclusion = "<script>alert(1)</script>".into();
        let html = render_dashboard_html(&report);
        assert!(!html.contains("<script>alert(1)</script>"));
        assert!(html.contains("&lt;script&gt;"));
    }

    #[test]
    fn test_render_market_review_md() {
        let review = MarketReviewReport {
            review_date: "2026-07-16".into(),
            generated_at: Utc::now(),
            indices: vec![
                IndexQuote { name: "上证指数".into(), price: 3200.0, change_pct: 0.5 },
                IndexQuote { name: "深证成指".into(), price: 10500.0, change_pct: -0.3 },
            ],
            advancers: Some(2500),
            decliners: Some(2300),
            limit_up: Some(45),
            limit_down: Some(8),
            sector_leaders: vec!["AI".into(), "白酒".into()],
            sector_laggards: vec!["地产".into()],
            llm_model: None,
        };
        let md = render_market_review_md(&review);
        assert!(md.contains("大盘复盘"));
        assert!(md.contains("上证指数"));
        assert!(md.contains("+0.50%"));
        assert!(md.contains("-0.30%"));
        assert!(md.contains("上涨: 2500"));
        assert!(md.contains("领涨板块"));
        assert!(md.contains("AI / 白酒"));
    }

    #[test]
    fn test_render_digest_md() {
        let digest = DashboardDigest {
            digest_date: "2026-07-16".into(),
            generated_at: Utc::now(),
            total_count: 3,
            buy_count: 1,
            watch_count: 1,
            sell_count: 1,
            summaries: vec![StockSummary {
                stock_code: "600519".into(),
                stock_name: "贵州茅台".into(),
                action: "买入".into(),
                score: 75,
                trend: "看多".into(),
                confidence: 80.0,
            }],
            market_review: None,
        };
        let md = render_dashboard_digest_md(&digest);
        assert!(md.contains("决策仪表盘汇总"));
        assert!(md.contains("总计 3 只"));
        assert!(md.contains("买入 1"));
        assert!(md.contains("贵州茅台"));
    }

    #[test]
    fn test_render_md_with_optional_fields_none() {
        let mut report = make_report();
        report.target_price = None;
        report.stop_loss = None;
        report.buy_point_low = None;
        report.buy_point_high = None;
        report.llm_model = None;
        let md = render_dashboard_md(&report);
        assert!(md.contains("目标价: —"));
        assert!(md.contains("止损价: —"));
        assert!(md.contains("🤖 —"));
        assert!(!md.contains("买入区间"));
    }

    #[test]
    fn test_build_dashboard_report_from_workflow_buy_signal() {
        let decision = serde_json::json!({
            "action": "买入",
            "positionPct": 30.0,
            "confidence": 80.0,
            "reasoning": "业绩超预期，估值合理。短期获利盘压力需关注。",
            "targetPrice": 1900.0,
            "stopLoss": 1600.0,
        });
        let score = serde_json::json!({
            "total": 75,
            "signal": "buy",
        });
        let mut reports = std::collections::HashMap::new();
        reports.insert("news-analyst".into(), "公司发布利好公告，业绩增长超预期。".into());
        reports.insert("policy-analyst".into(), "行业政策存在风险，需警惕。".into());

        let report = build_dashboard_report_from_workflow(
            &decision,
            &score,
            "600519",
            "贵州茅台",
            "2026-07-16",
            &reports,
            None,
        );

        assert_eq!(report.stock_code, "600519");
        assert_eq!(report.action, "买入");
        assert_eq!(report.score, 75);
        assert_eq!(report.trend, "看多");
        assert!((report.confidence - 80.0).abs() < 0.01);
        assert!((report.position_pct - 30.0).abs() < 0.01);
        assert_eq!(report.target_price, Some(1900.0));
        assert_eq!(report.stop_loss, Some(1600.0));
        assert!(report.integrity_passed);
        // 核心结论取第一句
        assert!(report.core_conclusion.contains("业绩超预期"));
        // 检查清单应有入场项
        assert!(report.checklist.iter().any(|c| c.category == "入场"));
        // 风险警报应从 policy-analyst 提取
        assert!(!report.risk_alerts.is_empty());
        // 催化因素应从 news-analyst 提取
        assert!(!report.catalysts.is_empty());
    }

    #[test]
    fn test_build_dashboard_report_from_workflow_hold_signal() {
        let decision = serde_json::json!({
            "action": "持有",
            "positionPct": 0.0,
            "confidence": 50.0,
            "reasoning": "趋势不明朗，建议观望。",
        });
        let score = serde_json::json!({
            "total": 45,
            "signal": "watch",
        });
        let reports = std::collections::HashMap::new();

        let report = build_dashboard_report_from_workflow(
            &decision,
            &score,
            "000001",
            "平安银行",
            "2026-07-16",
            &reports,
            None,
        );

        assert_eq!(report.action, "持有");
        assert_eq!(report.trend, "震荡");
        // 持有信号应有观望检查清单
        assert!(report.checklist.iter().any(|c| c.description.contains("观望")));
        // 无专家报告时风险警报和催化因素为空
        assert!(report.risk_alerts.is_empty());
        assert!(report.catalysts.is_empty());
    }

    #[test]
    fn test_build_dashboard_report_integrity_filled_when_missing() {
        // 缺少 targetPrice 但 action=买入 → 完整性校验失败 → 占位符补全
        let decision = serde_json::json!({
            "action": "买入",
            "positionPct": 20.0,
            "confidence": 70.0,
            "reasoning": "看好长期价值。",
            "stopLoss": 100.0,
            // 缺少 targetPrice
        });
        let score = serde_json::json!({"total": 60, "signal": "buy"});
        let reports = std::collections::HashMap::new();

        let report = build_dashboard_report_from_workflow(
            &decision,
            &score,
            "600519",
            "贵州茅台",
            "2026-07-16",
            &reports,
            None,
        );

        // 完整性校验失败但占位符补全后 integrity_passed=true
        assert!(report.integrity_passed);
        assert!(report.target_price.is_none()); // 占位符不填充数值
    }
}
