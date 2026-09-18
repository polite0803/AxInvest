use crate::decision_action::{normalize_action, ActionKind};
use std::collections::HashMap;

/// 修复 M-DS-3: 包装 `serde_json::from_str` 的失败路径，
/// 在解析失败时通过 `tracing::warn!` 记录原始 JSON 片段，便于排查上游数据质量问题。
/// 同时仍返回 `Default::default()` 维持降级行为（不破坏既有调用方）。
fn parse_or_warn<T: serde::de::DeserializeOwned + Default>(label: &str, json: &str) -> T {
    match serde_json::from_str::<T>(json) {
        Ok(v) => v,
        Err(e) => {
            let preview: String = json.chars().take(120).collect();
            tracing::warn!(
                "[stock-analysis] report.rs JSON 解析失败 label={} err={} preview={:?}",
                label,
                e,
                preview
            );
            T::default()
        },
    }
}

/// Generate an HTML visualization report from all analysis data
#[allow(clippy::too_many_arguments)]
pub fn generate_html_report(
    stock_code: &str,
    stock_name: &str,
    analysis_date: &str,
    quote_json: &str,
    indicators: &axagent_astock_data::indicators::TechnicalIndicators,
    score_json: &str,
    analyst_reports: &HashMap<String, String>,
    decision_json: &str,
    quality_summary: &str,
    rule_check_result: &str,
    value_assessment_json: &str,
    block_trades_json: &str,
    institutional_visits_json: &str,
    index_quotes_json: &str,
) -> String {
    let quote: serde_json::Value = parse_or_warn("quote", quote_json);
    let price = quote["price"].as_f64().unwrap_or(0.0);
    let change_pct = quote["changePct"].as_f64().unwrap_or(0.0);
    let score: serde_json::Value = parse_or_warn("score", score_json);
    let decision: serde_json::Value = parse_or_warn("decision", decision_json);
    let value: serde_json::Value = parse_or_warn("value_assessment", value_assessment_json);
    // 2026-09-12 修（P0-H）：value-investor 的输出有**两代 schema**，原实现只认旧的一代，
    // 且键名/类型也不对，导致报告里三张卡恒为 `-` / `0.0%` / `0/9 · 0/100`：
    //   · 旧（扁平）：`{ "buffett_verdict": "...", "margin_of_safety": -100.0, "f_score": 5, ... }`
    //   · 新（嵌套）：`{ "report": "...", "verdict": { ...同样字段... } }` ← 当前 v35 实测形态
    //   · 键名：是 `margin_of_safety`（新形态为**字符串** `"-90.8%"`），**不是** `margin_of_safety_pct`
    // 现改为：先取 `verdict` 子对象 → 非空则用，再回落顶层；百分数字符串做容错解析。
    let vroot = value.get("verdict").filter(|v| v.is_object());
    let get = |key: &str| -> serde_json::Value {
        vroot
            .and_then(|r| r.get(key))
            .filter(|v| !v.is_null())
            .cloned()
            .unwrap_or_else(|| value.get(key).cloned().unwrap_or(serde_json::Value::Null))
    };
    let as_pct = |v: &serde_json::Value| -> Option<f64> {
        v.as_f64().or_else(|| {
            v.as_str().and_then(|s| s.trim().trim_end_matches('%').trim().parse::<f64>().ok())
        })
    };
    let buffett_verdict =
        get("buffett_verdict").as_str().map(str::to_string).unwrap_or_else(|| "-".to_string());
    let margin_of_safety = as_pct(&get("margin_of_safety"))
        .or_else(|| as_pct(&get("margin_of_safety_pct")))
        .unwrap_or(0.0);
    let f_score = get("f_score").as_f64().unwrap_or(0.0);
    let moat = get("moat_score").as_f64().unwrap_or(0.0);

    // P0-H L3（2026-09-12）：字段名原为 `date` / `buyer` / `seller`，而
    // `astock_data::types::BlockTrade` 带 `#[serde(rename_all = "camelCase")]`，
    // 实际序列化键是 `tradeDate` / `buyerDept` / `sellerDept`
    // ⇒ 三个字段恒为 `-`（日期/买方/卖方三列全空，只有价格和数量能出数）。
    // 这是「字段名 ↔ 结构体」不匹配，与 §7.7 的层级/类型不匹配同族。
    let block_trades: Vec<serde_json::Value> = parse_or_warn("block_trades", block_trades_json);
    let block_trades_section = if block_trades.is_empty() {
        String::new()
    } else {
        let mut rows = String::new();
        for bt in block_trades.iter().take(5) {
            let date = bt["tradeDate"].as_str().unwrap_or("-");
            let price = bt["price"].as_f64().unwrap_or(0.0);
            let vol = bt["volume"].as_f64().unwrap_or(0.0);
            let buyer = bt["buyerDept"].as_str().unwrap_or("-");
            let seller = bt["sellerDept"].as_str().unwrap_or("-");
            rows.push_str(&format!(
                "<tr><td>{date}</td><td>{price:.2}</td><td>{vol:.0}</td><td>{buyer}</td><td>{seller}</td></tr>"
            ));
        }
        format!(
            "<h3 style=\"margin:16px 0 8px\">大宗交易</h3><table style=\"width:100%;font-size:12px;border-collapse:collapse\"><tr style=\"color:#8b949e\"><th>日期</th><th>价格</th><th>数量</th><th>买方</th><th>卖方</th></tr>{rows}</table>"
        )
    };

    // P0-H L3（2026-09-12）：字段名原为 `date` / `org_count` / `content`，
    // 而 `astock_data::types::InstitutionalVisit`（camelCase 序列化）实际是
    // `visitDate` / `institutionCount` / `mainContent` ⇒ 三列全空（恒 `-` / `0` / `-`）。
    let visits: Vec<serde_json::Value> =
        parse_or_warn("institutional_visits", institutional_visits_json);
    let institutional_visits_section = if visits.is_empty() {
        String::new()
    } else {
        let mut rows = String::new();
        for v in visits.iter().take(5) {
            let date = v["visitDate"].as_str().unwrap_or("-");
            let orgs = v["institutionCount"].as_u64().unwrap_or(0);
            let content = v["mainContent"].as_str().unwrap_or("-");
            let short: String = content.chars().take(60).collect();
            rows.push_str(&format!("<tr><td>{date}</td><td>{orgs}</td><td>{short}</td></tr>"));
        }
        format!(
            "<h3 style=\"margin:16px 0 8px\">机构调研</h3><table style=\"width:100%;font-size:12px;border-collapse:collapse\"><tr style=\"color:#8b949e\"><th>日期</th><th>机构数</th><th>内容</th></tr>{rows}</table>"
        )
    };

    let index_quotes: Vec<serde_json::Value> = parse_or_warn("index_quotes", index_quotes_json);
    let index_quotes_section = if index_quotes.is_empty() {
        String::new()
    } else {
        let mut rows = String::new();
        for idx in index_quotes.iter() {
            let name = idx["name"].as_str().unwrap_or("-");
            let price = idx["price"].as_f64().unwrap_or(0.0);
            let pct = idx["changePct"].as_f64().unwrap_or(0.0);
            let color = if pct >= 0.0 { "#3fb950" } else { "#f85149" };
            let sign = if pct >= 0.0 { "+" } else { "" };
            rows.push_str(&format!(
                "<tr><td>{name}</td><td>{price:.2}</td><td style=\"color:{color}\">{sign}{pct:.2}%</td></tr>"
            ));
        }
        format!(
            "<h3 style=\"margin:16px 0 8px\">大盘指数</h3><table style=\"width:100%;font-size:12px;border-collapse:collapse\"><tr style=\"color:#8b949e\"><th>指数</th><th>点位</th><th>涨跌幅</th></tr>{rows}</table>"
        )
    };

    // P0-H L3（2026-09-12）：**删除「同行业可比公司」与「期权PCR」两个板块**。
    //
    // 决策依据（实测，非推测）：
    //   · 期权 PCR —— `eastmoney::get_option_pcr` 对**非 ETF 代码硬编码 `return Ok(None)`**
    //     （实现注释：「个股期权 PCR 数据无稳定公开 API，且多数个股无场内期权」），
    //     而股票分析标的恒为个股 ⇒ 该板块**在任何个股上都不可能出数**，属代码级死路径。
    //   · 同行对比 —— `eastmoney::get_peers` 取的是「精准**概念**板块」而非行业：
    //     实测 603353（和顺石油，加油站零售）返回的「同行」是高压快充 / 存储芯片 /
    //     先进封装 / 半导体概念（因其 2026-03-19 收购奎芯科技的公告被打上半导体标签）
    //     ⇒ 数据源可用但**语义错误**，渲染出来会主动误导用户。
    //     修它属独立的「数据源语义」缺陷（已登记），不在本次报告导出修复范围内。
    //
    // 取舍原则：**宁可不出，不可出错** —— 显示错误「同行」比留空更危险；
    // 且不留「该数据源未接入」的半吊子占位（那只是把空换成另一种空）。
    // 若后续修好 `get_peers` 语义，再连同参数与板块一并恢复。

    // Build analyst report HTML
    let mut analyst_html = String::new();
    for expert_id in &[
        "market-analyst",
        "sentiment-analyst",
        "news-analyst",
        "fundamentals-analyst",
        "policy-analyst",
        "hot-money-tracker",
        "lockup-watcher",
    ] {
        if let Some(report) = analyst_reports.get(*expert_id) {
            let name = get_analyst_display_name(expert_id);
            let escaped = report
                .replace('&', "&amp;")
                .replace('<', "&lt;")
                .replace('>', "&gt;")
                .replace('\n', "<br>");
            analyst_html.push_str(&format!(
                r#"<div class='report-card'><h3>{}</h3><p>{}</p></div>"#,
                name, escaped
            ));
        }
    }

    let total = score["total"].as_u64().unwrap_or(0);
    let trend = score["trendScore"].as_u64().unwrap_or(0);
    let deviation = score["deviationScore"].as_u64().unwrap_or(0);
    let macd_s = score["macdScore"].as_u64().unwrap_or(0);
    let volume_s = score["volumeScore"].as_u64().unwrap_or(0);
    let rsi_s = score["rsiScore"].as_u64().unwrap_or(0);
    let support_s = score["supportScore"].as_u64().unwrap_or(0);
    let signal = score["signal"].as_str().unwrap_or("-");
    let action = decision["action"].as_str().unwrap_or("-");
    let position = decision["positionPct"].as_f64().unwrap_or(0.0);
    let reasoning = decision["reasoning"].as_str().unwrap_or("");

    let price_color = if change_pct >= 0.0 {
        "#3fb950"
    } else {
        "#f85149"
    };
    let change_sign = if change_pct >= 0.0 { "+" } else { "" };
    let score_color = if total >= 60 {
        "#3fb950"
    } else if total >= 30 {
        "#d29922"
    } else {
        "#f85149"
    };
    // P1-6(2026-09-14): 原先只认 4 个中文字面量 —— 英文值域落 _ 档，看多/看空全部
    // 被渲染成中性黄（与前端 getActionColor 同一类缺陷）。
    let action_color = match normalize_action(action) {
        Some(ActionKind::Buy | ActionKind::Increase) => "#3fb950",
        Some(ActionKind::Reduce | ActionKind::Sell) => "#f85149",
        _ => "#d29922",
    };
    let ma5 = indicators.ma5;
    let ma10 = indicators.ma10;
    let ma20 = indicators.ma20;
    let ma60 = indicators.ma60;
    let dif = indicators.macd_dif;
    let dea = indicators.macd_dea;
    let bar = indicators.macd_bar;
    let macd_signal = &indicators.macd_signal;
    let rsi6 = indicators.rsi6;
    let rsi12 = indicators.rsi12;
    let rsi24 = indicators.rsi24;
    let vol_ratio = indicators.volume_ratio;

    format!(
        r###"<!DOCTYPE html>
<html lang="zh-CN">
<head>
<meta charset="UTF-8">
<meta name="viewport" content="width=device-width, initial-scale=1.0">
<title>股票分析报告 - {stock_name}({stock_code})</title>
<style>
*{{margin:0;padding:0;box-sizing:border-box}}
body{{font-family:-apple-system,BlinkMacSystemFont,"Segoe UI",Roboto,sans-serif;background:#0f1117;color:#e1e4e8;padding:20px;max-width:1000px;margin:0 auto}}
.header{{text-align:center;padding:30px 0;border-bottom:1px solid #30363d;margin-bottom:20px}}
.header h1{{font-size:28px;margin-bottom:8px}}
.header .sub{{color:#8b949e;font-size:14px}}
.price-row{{display:flex;gap:20px;justify-content:center;align-items:baseline;margin:16px 0}}
.price{{font-size:36px;font-weight:bold;color:{price_color}}}
.change{{font-size:20px;color:{price_color}}}
.grid{{display:grid;grid-template-columns:repeat(3,1fr);gap:12px;margin:16px 0}}
.card{{background:#161b22;border:1px solid #30363d;border-radius:8px;padding:16px}}
.card h3{{font-size:12px;color:#8b949e;text-transform:uppercase;margin-bottom:8px}}
.card .value{{font-size:20px;font-weight:bold}}
.score-bar{{height:8px;border-radius:4px;background:#21262d;margin-top:8px}}
.score-fill{{height:100%;border-radius:4px;transition:width .3s}}
.score-grid{{display:grid;grid-template-columns:repeat(3,1fr);gap:8px;margin:8px 0}}
.score-item{{background:#161b22;padding:10px;border-radius:6px;text-align:center}}
.score-item .label{{font-size:11px;color:#8b949e}}
.score-item .val{{font-size:18px;font-weight:bold}}
.reports{{margin:20px 0}}
.report-card{{background:#161b22;border:1px solid #30363d;border-radius:8px;padding:16px;margin-bottom:10px}}
.report-card h3{{font-size:14px;margin-bottom:8px;color:#58a6ff}}
.report-card p{{font-size:13px;line-height:1.6;color:#c9d1d9}}
.decision{{background:linear-gradient(135deg,#1a2332,#0f1a25);border:2px solid {action_color};border-radius:12px;padding:24px;margin:20px 0;text-align:center}}
.decision h2{{font-size:22px;margin-bottom:8px}}
.decision .action{{font-size:32px;font-weight:bold;color:{action_color};margin:8px 0}}
.footer{{text-align:center;color:#8b949e;font-size:12px;margin-top:30px;padding-top:20px;border-top:1px solid #30363d}}
</style>
</head>
<body>
<div class="header">
  <h1>{stock_name} ({stock_code})</h1>
  <div class="sub">分析日期: {analysis_date}</div>
</div>
<div class="price-row">
  <span class="price">¥{price:.2}</span>
  <span class="change">{change_sign}{change_pct:.2}%</span>
</div>
<div class="grid">
  <div class="card">
    <h3>综合评分</h3>
    <div class="value" style="color:{score_color}">{total}/100</div>
    <div class="score-bar"><div class="score-fill" style="width:{total}%;background:{score_color}"></div></div>
  </div>
  <div class="card">
    <h3>技术信号</h3>
    <div class="value" style="font-size:16px">{signal}</div>
    <div style="font-size:12px;color:#8b949e;margin-top:4px">{indicators_ma_alignment}</div>
  </div>
  <div class="card">
    <h3>投资建议</h3>
    <div class="value" style="color:{action_color}">{action}</div>
    <div style="font-size:12px;color:#8b949e;margin-top:4px">仓位 {position}%</div>
  </div>
</div>

<h3 style="margin:16px 0 8px">评分明细</h3>
<div class="score-grid">
  <div class="score-item"><div class="label">趋势</div><div class="val">{trend}/30</div></div>
  <div class="score-item"><div class="label">乖离率</div><div class="val">{deviation}/20</div></div>
  <div class="score-item"><div class="label">MACD</div><div class="val">{macd_s}/15</div></div>
  <div class="score-item"><div class="label">量能</div><div class="val">{volume_s}/15</div></div>
  <div class="score-item"><div class="label">RSI</div><div class="val">{rsi_s}/10</div></div>
  <div class="score-item"><div class="label">支撑</div><div class="val">{support_s}/10</div></div>
</div>

<h3 style="margin:16px 0 8px">技术指标</h3>
<div class="grid">
  <div class="card"><h3>均线</h3><div style="font-size:13px">MA5: {ma5:.2}<br>MA10: {ma10:.2}<br>MA20: {ma20:.2}<br>MA60: {ma60:.2}</div></div>
  <div class="card"><h3>MACD</h3><div style="font-size:13px">DIF: {dif:.2}<br>DEA: {dea:.2}<br>柱: {bar:.2}<br>{macd_signal}</div></div>
  <div class="card"><h3>RSI &amp; 量能</h3><div style="font-size:13px">RSI6: {rsi6:.0}<br>RSI12: {rsi12:.0}<br>RSI24: {rsi24:.0}<br>量比: {vol_ratio:.1}x</div></div>
</div>

<h3 style="margin:16px 0 8px">价值评估</h3>
<div class="grid">
  <div class="card"><h3>巴菲特判定</h3><div class="value" style="font-size:16px">{buffett_verdict}</div></div>
  <div class="card"><h3>安全边际</h3><div class="value">{margin_of_safety:.1}%</div></div>
  <div class="card"><h3>F-Score / 护城河</h3><div class="value">{f_score:.0}/9 · {moat:.0}/100</div></div>
</div>

{block_trades_section}

{institutional_visits_section}

{index_quotes_section}

<div class="decision">
  <h2>最终决策</h2>
  <div class="action">{action}</div>
  <div style="font-size:14px;margin-top:8px">{reasoning}</div>
</div>

<h3 style="margin:16px 0 8px">分析师报告</h3>
<div class="reports">{analyst_html}</div>

<div style="margin:12px 0;padding:12px;background:#161b22;border-radius:8px;font-size:12px;color:#8b949e">
  <strong>质量评估:</strong> {quality_summary}<br>
  <strong>规则检查:</strong> {rule_check_result}
</div>

<div class="footer">
  <p>⚠️ 本报告由 AxInvest AI 分析引擎生成，仅供参考，不构成投资建议。</p>
  <p>股市有风险，投资需谨慎。Generated at {analysis_date}</p>
</div>
</body>
</html>"###,
        indicators_ma_alignment = indicators.ma_alignment,
    )
}

fn get_analyst_display_name(id: &str) -> &str {
    match id {
        "market-analyst" => "市场技术分析师",
        "sentiment-analyst" => "情绪面分析师",
        "news-analyst" => "消息面分析师",
        "fundamentals-analyst" => "基本面分析师",
        "policy-analyst" => "政策面分析师",
        "hot-money-tracker" => "资金面追踪者",
        "lockup-watcher" => "筹码面观察者",
        _ => id,
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// P0-H 回归测试（2026-09-12 补）
//
// 本文件此前**零单测**，是 P0-H 三类缺陷（信封形态 / 字段层级键名类型 / camelCase 字段名）
// 全都静默退化的结构性原因 —— 没有任何断言守着这些卡片的取值路径。
//
// 三个用例分别锁死三代不匹配：
//   ① 新一代嵌套 schema 必须出真值（否则三卡恒 `-` / `0.0%` / `0/9 · 0/100`）
//   ② 旧一代扁平 schema 不能因支持新 schema 而回退失效
//   ③ 三个市场板块的 camelCase 字段名（原实现读 snake_case，每一列恒 `-` / `0`）
//
// 断言刻意用**格式化后的最终呈现串**（`"7/9"`、`"-12.5%"`），而不是中间的 `Value` ——
// 铁律「修 bug ≠ 修结论：验证必须核对最终决策字段」的同构应用。
// ─────────────────────────────────────────────────────────────────────────────
#[cfg(test)]
mod p0h_report_regression {
    use super::*;
    use axagent_astock_data::indicators::TechnicalIndicators;

    /// 三代不匹配全部命中的输入：嵌套层级 + 字符串百分数 + 真实键名。
    fn render(value_json: &str, block: &str, visits: &str, idx: &str) -> String {
        generate_html_report(
            "603353",
            "和顺石油",
            "2026-09-12",
            r#"{"price":39.25,"changePct":-1.18}"#,
            &TechnicalIndicators::default(),
            "{}",
            &HashMap::new(),
            "{}",
            "",
            "",
            value_json,
            block,
            visits,
            idx,
        )
    }

    #[test]
    fn reads_nested_value_schema_with_string_pct() {
        let html = render(
            r#"{"report":"分析正文","verdict":{"buffett_verdict":"买入","margin_of_safety":"-12.5%","f_score":7,"moat_score":80}}"#,
            "[]",
            "[]",
            "[]",
        );
        // 正向断言必须**锚定到卡片自身的 DOM 形态**（`<h3>标题</h3><div class="value">值</div>`）。
        // 两种错误锚法（本例首跑两次踩坑：`{buffett_verdict}` 与 `{signal}` 两张卡 DOM 完全同形）：
        //   · 只写 `html.contains("买入")` —— 会被报告别处的字面量蒙过去 ⇒ **假通过**
        //   · 只写 `!html.contains(">-</div>")` / `style="font-size:16px">-</div>` ——
        //     会命中其它**本就为空**的卡片（报头 `{signal}` 卡同形）⇒ **假失败**
        assert!(
            html.contains(
                r#"<h3>巴菲特判定</h3><div class="value" style="font-size:16px">买入</div>"#
            ),
            "巴菲特判定未从 verdict 子对象取到值"
        );
        assert!(
            html.contains(r#"<h3>安全边际</h3><div class="value">-12.5%</div>"#),
            "安全边际未解析字符串百分数"
        );
        // 护城河卡含 U+00B7 分隔符，故拆成「锚卡片前缀 + 校验第二段」两步，
        // 避免测试代码里手抄分隔符（抄错就变成又一次假失败）。
        assert!(
            html.contains(r#"<h3>F-Score / 护城河</h3><div class="value">7/9"#),
            "F-Score 未取到值（卡片仍为空）"
        );
        assert!(html.contains("80/100"), "护城河未取到值");
    }

    #[test]
    fn stays_compatible_with_flat_value_schema() {
        let html = render(
            r#"{"buffett_verdict":"观望","margin_of_safety":8.5,"f_score":5,"moat_score":60}"#,
            "[]",
            "[]",
            "[]",
        );
        assert!(
            html.contains(
                r#"<h3>巴菲特判定</h3><div class="value" style="font-size:16px">观望</div>"#
            ),
            "旧扁平 schema 的 buffett_verdict 回退失效"
        );
        assert!(
            html.contains(r#"<h3>安全边际</h3><div class="value">8.5%</div>"#),
            "旧扁平 schema 的数值型 margin_of_safety 回退失效"
        );
        assert!(
            html.contains(r#"<h3>F-Score / 护城河</h3><div class="value">5/9"#),
            "旧扁平 schema 的 f_score 回退失效"
        );
        assert!(html.contains("60/100"), "旧扁平 schema 的 moat_score 回退失效");
    }

    #[test]
    fn reads_camel_case_market_sections() {
        let html = render(
            "{}",
            r#"[{"tradeDate":"2026-08-14","price":31.61,"volume":157900.0,"buyerDept":"国泰海通总部","sellerDept":"机构专用"}]"#,
            r#"[{"visitDate":"2026-09-09","institutionCount":5,"mainContent":"请问您如何看待行业未来的发展前景"}]"#,
            r#"[{"code":"000001","name":"上证指数","price":3888.11,"changePct":-1.18}]"#,
        );
        assert!(html.contains("2026-08-14"), "大宗交易未读取 tradeDate（原读 date）");
        assert!(html.contains("国泰海通总部"), "大宗交易未读取 buyerDept（原读 buyer）");
        assert!(html.contains("机构专用"), "大宗交易未读取 sellerDept（原读 seller）");
        assert!(html.contains("2026-09-09"), "机构调研未读取 visitDate（原读 date）");
        assert!(html.contains("上证指数"), "指数未读取 name");
        assert!(html.contains("3888.11"), "指数未读取 price");
        // 已删除的两个板块不得再渲染
        assert!(!html.contains("同行业可比公司"), "已删除的同行板块仍在渲染");
        assert!(!html.contains("期权PCR"), "已删除的期权PCR板块仍在渲染");
    }
}
