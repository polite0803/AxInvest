//! Serenity 瓶颈分析子策略
//!
//! 不同于其他 5 个策略基于价格/成交量信号扫描，
//! SerenityStrategy 的 seed pool 来自 serenity-screening workflow 的输出，
//! scan_one 只做确定性财务/估值信号验证。
//!
//! 工作流：workflow 生成候选池 → 持久化 → SerenityStrategy 读取 → scan_one 验证

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
        // 1. 获取财务数据验证护城河
        let financials = client.get_financials(code).await.ok()?;
        // 取出最新一期财务数据（数组倒序，新→旧，所以 first 是最新）
        let latest = financials.first()?;

        // 2. 毛利率 > 50%（技术壁垒/品牌溢价）
        let gross_margin = read_f64(vars, "serenity_min_gross_margin", 50.0);
        let gm = latest.gross_margin.unwrap_or(0.0);
        if gm < gross_margin {
            return None;
        }

        // 3. 负债率 < 60%（避免融资稀释风险）；数据缺失时直接过滤掉
        let max_debt = read_f64(vars, "serenity_max_debt_ratio", 60.0);
        let dr = latest.debt_ratio?;
        if dr > max_debt {
            return None;
        }

        // 4. 营收增速（同比）> 10%（成长性验证），用 revenue_yoy
        let min_rev_growth = read_f64(vars, "serenity_min_revenue_growth", 10.0);
        let rev_growth = latest.revenue_yoy.unwrap_or(0.0);
        if rev_growth < min_rev_growth {
            return None;
        }

        // 5. 获取实时行情用于价格/入场计算
        let quote = client.get_quote(code).await.ok()?;
        let price = quote.price;

        let target_mult = read_f64(vars, "serenity_target_mult", 1.30);
        let stop_mult = read_f64(vars, "serenity_stop_mult", 0.80);
        let entry_range = read_f64(vars, "serenity_entry_range", 0.05);

        // 如果有 EPS，用目标 P/E 倍数做估值锚定（默认 25x 成长股）
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

        // 置信度计算：用护城河指标 + 实际行情数据
        let conf_quality = (gm / 100.0).min(1.0); // 毛利率越高越可信
        let conf_growth = (rev_growth / 50.0).min(1.0); // 增速越高，但上限 50%
                                                        // 用实际行情数据替代硬编码值
        let liquidity = (quote.turnover_rate / 5.0).clamp(0.1, 1.0); // 换手率/5 ≈ 流动性
        let price_momentum = (quote.change_pct.abs() / 10.0).clamp(0.1, 1.0); // 涨跌幅/10 ≈ 动量
        let turnover_anomaly = 1.0; // 仍硬编码：无历史换手率做对比基准
        let conf = calc_confidence(
            conf_quality * 0.6 + conf_growth * 0.4, // consistency
            conf_quality,                           // signal_strength
            liquidity,                              // 从实际换手率计算
            price_momentum,                         // 从实际涨跌幅计算
            turnover_anomaly,
        );
        let position = calc_position(base_position, conf, self.period);

        let reasons = vec![
            format!("确定性财务验证通过"),
            format!("毛利率 {:.1}% > {}% 门槛", gm, gross_margin),
            format!("营收同比增速 {:.1}% > {}% 门槛", rev_growth, min_rev_growth),
            format!("负债率 {:.1}% < {}% 上限", dr, max_debt),
        ];

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
            risk_notes: vec![
                "瓶颈环节可能因技术变革或竞争格局变化而失效".to_string(),
                "建议作为投资组合的弹性增强部分，而非全部".to_string(),
            ],
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
