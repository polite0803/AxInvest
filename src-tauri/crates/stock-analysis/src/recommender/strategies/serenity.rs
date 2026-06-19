//! Serenity 瓶颈分析子策略
//!
//! 不同于其他 5 个策略基于价格/成交量信号扫描，
//! SerenityStrategy 的 seed pool 来自 serenity-screening workflow 的输出，
//! scan_one 从全局缓存读取全量诊断数据（serenity_score / catalysts / exit_signals），
//! 做上下文感知的确定性财务验证。
//!
//! 工作流：workflow 生成候选池 → 持久化 + 全量缓存 → SerenityStrategy 读取 → scan_one 验证

use super::super::strategy::{read_f64, RecoContext, RecommendStrategy};
use crate::recommender::scoring::{calc_confidence, calc_position};
use crate::recommender::types::{Period, RecoPick, Style};
use async_trait::async_trait;
use axagent_astock_data::AStockClient;
use serde_json::Value;
use std::collections::HashMap;

pub struct SerenityStrategy {
    pub period: Period,
}

impl SerenityStrategy {
    /// Serenity 只做中长期（Mid / Long），不做短/超短
    pub const fn mid() -> Self {
        Self {
            period: Period::Mid,
        }
    }
    pub const fn long() -> Self {
        Self {
            period: Period::Long,
        }
    }

    async fn scan_one(
        &self,
        client: &AStockClient,
        code: &str,
        name: &str,
        sector: Option<String>,
        vars: &HashMap<String, Value>,
    ) -> Option<RecoPick> {
        // 0. 读取 Serenity 全量诊断数据（由 workflow 填入缓存）
        let detail = super::super::get_serenity_candidate_detail(code);
        let serenity_score = detail
            .as_ref()
            .and_then(|d| d["serenity_score"].as_f64())
            .unwrap_or(0.0);
        let catalysts = detail
            .as_ref()
            .and_then(|d| d["catalysts"].as_array())
            .cloned()
            .unwrap_or_default();
        let exit_signals = detail
            .as_ref()
            .and_then(|d| d["exit_signals"].as_object())
            .cloned()
            .unwrap_or_default();
        let _attention_metrics = detail
            .as_ref()
            .and_then(|d| d["attention_metrics"].as_object())
            .cloned()
            .unwrap_or_default();

        // 1. 获取财务数据验证护城河
        let financials = client.get_financials(code).await.ok()?;
        let latest = financials.first()?;

        // 2. 毛利率校验（上下文感知：高 serenity_score 可豁免部分硬门槛）
        let base_gross_margin = read_f64(vars, "serenity_min_gross_margin", 50.0);
        // 评分 >= 80 时门槛降为 40%，>= 90 时降为 35%
        let gross_margin = if serenity_score >= 90.0 {
            base_gross_margin.min(35.0)
        } else if serenity_score >= 80.0 {
            base_gross_margin.min(40.0)
        } else {
            base_gross_margin
        };
        let gm = latest.gross_margin.unwrap_or(0.0);
        if gm < gross_margin {
            return None;
        }

        // 3. 负债率校验（同样上下文感知）
        let base_max_debt = read_f64(vars, "serenity_max_debt_ratio", 60.0);
        let max_debt = if serenity_score >= 85.0 {
            base_max_debt.max(70.0) // 高评分候选放宽到 70%
        } else {
            base_max_debt
        };
        let dr = latest.debt_ratio?;
        if dr > max_debt {
            return None;
        }

        // 4. 营收增速校验（上下文感知）
        let base_min_rev_growth = read_f64(vars, "serenity_min_revenue_growth", 10.0);
        let min_rev_growth = if serenity_score >= 85.0 {
            base_min_rev_growth.min(5.0) // 高评分候选放宽到 5%
        } else {
            base_min_rev_growth
        };
        let rev_growth = latest.revenue_yoy.unwrap_or(0.0);
        if rev_growth < min_rev_growth {
            return None;
        }

        // === 价值捕获深层验证 ===

        // 5a. ROIC 近似（ROE > 15% 为强信号）
        let roe = latest.roe.unwrap_or(0.0);
        let roe_ok = roe > 15.0;

        // 5b. 经营现金流/净利润比 > 0.7
        let ocf_ratio_ok = if let (Some(op_cf), Some(net_profit)) =
            (latest.operating_cash_flow, latest.net_profit)
        {
            if net_profit.abs() > 0.0 {
                (op_cf / net_profit).abs() > 0.7
            } else {
                true
            }
        } else {
            true
        };

        // 5c. 毛利率趋势
        let margin_trend_ok = if financials.len() >= 2 {
            let prev_gm = financials[1].gross_margin.unwrap_or(0.0);
            prev_gm > 0.0 && gm >= prev_gm * 0.9
        } else {
            true
        };

        // 6. 行情数据
        let quote = client.get_quote(code).await.ok()?;
        let price = quote.price;

        let target_mult = read_f64(vars, "serenity_target_mult", 1.30);
        let stop_mult = read_f64(vars, "serenity_stop_mult", 0.80);
        let entry_range = read_f64(vars, "serenity_entry_range", 0.05);

        let target_price = if let Some(eps) = latest.eps {
            let target_pe = read_f64(vars, "serenity_target_pe", 25.0);
            (eps * target_pe).max(price * target_mult)
        } else {
            price * target_mult
        };
        let stop_loss = price * stop_mult;
        let entry_low = price * (1.0 - entry_range);
        let entry_high = price * (1.0 + entry_range);
        let base_position = read_f64(vars, "serenity_base_position", 10.0);

        // 置信度计算（含 workflow 诊断因子）
        let conf_quality = (gm / 100.0).min(1.0);
        let conf_growth = (rev_growth / 50.0).min(1.0);
        let conf_roe = if roe_ok { 1.0 } else { 0.6 };
        let conf_ocf = if ocf_ratio_ok { 1.0 } else { 0.7 };
        let conf_margin_trend = if margin_trend_ok { 1.0 } else { 0.7 };
        // workflow 评分因子：0-100 归一化到 0.5-1.0
        let conf_workflow = 0.5 + (serenity_score / 200.0).min(0.5);
        // 催化剂加分：每个催化剂 confidence >= 70 加 0.05
        let conf_catalyst = (0.05
            * catalysts
                .iter()
                .filter(|c| c["confidence"].as_f64().unwrap_or(0.0) >= 70.0)
                .count() as f64)
            .min(0.15);

        let consistency = conf_quality * 0.25
            + conf_growth * 0.20
            + conf_roe * 0.15
            + conf_ocf * 0.10
            + conf_margin_trend * 0.10
            + conf_workflow * 0.15
            + conf_catalyst * 0.05;
        let signal_strength = conf_quality.min(conf_growth).min(conf_roe);

        let liquidity = (quote.turnover_rate / 5.0).clamp(0.1, 1.0);
        let price_momentum = (quote.change_pct.abs() / 10.0).clamp(0.1, 1.0);
        let turnover_anomaly = 1.0;
        let conf = calc_confidence(
            consistency,
            signal_strength,
            liquidity,
            price_momentum,
            turnover_anomaly,
        );
        let position = calc_position(base_position, conf, self.period);

        // 构建 reasons（含 workflow 诊断信息）
        let mut reasons = vec![
            format!("确定性财务验证通过"),
            format!("毛利率 {:.1}% > {}% 门槛", gm, gross_margin),
            format!("营收同比增速 {:.1}% > {}% 门槛", rev_growth, min_rev_growth),
            format!("负债率 {:.1}% < {}% 上限", dr, max_debt),
        ];
        if serenity_score > 0.0 {
            reasons.push(format!("瓶颈分析评分: {:.0}/100", serenity_score));
        }
        if roe_ok {
            reasons.push(format!("ROE {:.1}% > 15% 显示资本回报效率高", roe));
        }
        if margin_trend_ok && financials.len() >= 2 {
            let prev_gm = financials[1].gross_margin.unwrap_or(0.0);
            reasons.push(format!("毛利率趋势稳定：从 {:.1}% → {:.1}%", prev_gm, gm));
        }
        // 催化剂摘要
        for cat in catalysts.iter().take(2) {
            let cat_desc = cat["description"].as_str().unwrap_or("");
            let cat_tf = cat["expected_timeframe"].as_str().unwrap_or("");
            if !cat_desc.is_empty() {
                reasons.push(format!("催化剂: {} ({})", cat_desc, cat_tf));
            }
        }

        // 风险提示（含退出信号感知）
        let mut risk_notes = vec![
            "瓶颈环节可能因技术变革或竞争格局变化而失效".to_string(),
            "建议作为投资组合的弹性增强部分，而非全部".to_string(),
        ];
        // workflow 诊断的主风险
        if let Some(primary_risk) = detail.as_ref().and_then(|d| d["primary_risk"].as_str()) {
            if !primary_risk.is_empty() {
                risk_notes.push(format!("工作流诊断风险: {}", primary_risk));
            }
        }
        // 退出信号
        if let Some(urgency) = exit_signals
            .get("overall_exit_urgency")
            .and_then(|v| v.as_str())
        {
            if urgency == "caution" {
                risk_notes.push("⚠ 退出信号：6-12月内关注退出条件".to_string());
            } else if urgency == "watch" {
                risk_notes.push("退出信号：12月以上关注技术替代风险".to_string());
            }
        }
        // 技术替代风险
        if let Some(tech_risk) = exit_signals
            .get("technology_disruption_risk")
            .and_then(|v| v.as_str())
        {
            if !tech_risk.is_empty() && !tech_risk.eq_ignore_ascii_case("null") {
                risk_notes.push(format!("技术替代风险: {}", tech_risk));
            }
        }
        // 毛利率下降
        if !margin_trend_ok && financials.len() >= 2 {
            let prev_gm = financials[1].gross_margin.unwrap_or(0.0);
            risk_notes.push(format!(
                "⚠ 毛利率从 {:.1}% 下降至 {:.1}%，关注技术替代或竞争加剧风险",
                prev_gm, gm
            ));
        }
        // 高负债率
        if dr > 50.0 {
            risk_notes.push("负债率偏高，关注再融资或稀释风险".to_string());
        }

        Some(RecoPick {
            stock_code: code.into(),
            stock_name: name.into(),
            sector,
            style: Style::Serenity,
            period: self.period,
            price,
            entry_low,
            entry_high,
            stop_loss,
            target_price,
            position_pct: position,
            holding_days: self.period.default_holding_days(),
            confidence: conf,
            reasons,
            risk_notes,
            secondary_styles: vec![],
            synthetic: false,
        })
    }
}

#[async_trait]
impl RecommendStrategy for SerenityStrategy {
    fn id(&self) -> &'static str {
        match self.period {
            Period::Mid => "serenity_mid",
            Period::Long => "serenity_long",
            _ => unreachable!("Serenity 只支持 Mid/Long，不应被调用其他周期"),
        }
    }
    fn style(&self) -> Style {
        Style::Serenity
    }
    fn period(&self) -> Period {
        self.period
    }
    fn required_vendors(&self) -> &'static [&'static str] {
        &["eastmoney", "ths", "akshare"]
    }

    async fn scan(&self, ctx: &RecoContext<'_>) -> Result<Vec<RecoPick>, String> {
        let mut picks = Vec::new();
        for (code, name, sector) in ctx.seed {
            let _g = ctx.per_code_locks.lock_for(code).await;
            if let Some(p) = self
                .scan_one(ctx.client, code, name, sector.clone(), ctx.vars)
                .await
            {
                picks.push(p);
            }
        }
        Ok(picks)
    }
}
