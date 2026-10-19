//! 候选池兜底策略：仅依赖 `get_quote`，永远能出 picks
//!
//! 当 4 个主策略（trend/value/capital/reversion）因网络/数据不可用全部返回空时，
//! 这个策略保证面板至少展示 seed pool 的"系统初筛"列表，附带基于现价的合成
//! 入场/止损/目标位（按周期调整振幅）。
//!
//! 不做任何技术信号判定，所以"confidence"和"position"都给得保守（低 0.5 / 0.4 base），
//! 真实仓位由用户在账户页面自己分配。
//!
//! ⚠ 本桶的出票默认标 `synthetic: true`（入场位是现价乘数，不是信号推导），**只有**被
//! 分析定档命中且动作允许的那几条会升级为真实票 —— 见 [`apply_analysis_tier`]。

use super::super::strategy::{read_f64, PerCodeLocks, RecoContext, RecommendStrategy};
use crate::recommender::pool::SeedItem;
use crate::recommender::scoring::{calc_confidence, calc_position};
use crate::recommender::types::{Period, RecoPick, Style};
use async_trait::async_trait;
use axagent_harness::market_data::MarketDataProvider;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;

pub struct WatchlistStrategy {
    pub period: Period,
}

impl WatchlistStrategy {
    pub const fn ultra_short() -> Self {
        Self { period: Period::UltraShort }
    }
    pub const fn short() -> Self {
        Self { period: Period::Short }
    }
    pub const fn mid() -> Self {
        Self { period: Period::Mid }
    }
    pub const fn long() -> Self {
        Self { period: Period::Long }
    }

    async fn scan_one(
        &self,
        client: &dyn MarketDataProvider,
        code: &str,
        name: &str,
        sector: Option<String>,
        vars: &HashMap<String, Value>,
    ) -> Option<RecoPick> {
        // 只 fetch 实时 quote，不依赖 K 线 / 财务 / 资金流，确保尽量总有数据
        let quote = client.get_quote(code).await.ok()?;
        if quote.is_st {
            return None;
        }
        if quote.price <= 0.0 {
            return None;
        }
        let price = quote.price;

        // 按周期给不同的振幅
        let (entry_low, entry_high, stop_loss, target_price, base_position, reason) =
            match self.period {
                Period::UltraShort => {
                    let el = read_f64(vars, "wl_ultra_short_entry_low", 0.998);
                    let eh = read_f64(vars, "wl_ultra_short_entry_high", 1.005);
                    let sl = read_f64(vars, "wl_ultra_short_stop", 0.98);
                    let tg = read_f64(vars, "wl_ultra_short_target", 1.03);
                    let bp = read_f64(vars, "wl_ultra_short_base_pos", 2.0);
                    (
                        price * el,
                        price * eh,
                        price * sl,
                        price * tg,
                        bp,
                        "候选池初筛（超短线：-0.2% 进场，+3% 目标）",
                    )
                },
                Period::Short => {
                    let el = read_f64(vars, "wl_short_entry_low", 0.99);
                    let eh = read_f64(vars, "wl_short_entry_high", 1.01);
                    let sl = read_f64(vars, "wl_short_stop", 0.96);
                    let tg = read_f64(vars, "wl_short_target", 1.06);
                    let bp = read_f64(vars, "wl_short_base_pos", 4.0);
                    (
                        price * el,
                        price * eh,
                        price * sl,
                        price * tg,
                        bp,
                        "候选池初筛（短线：日内-1% 进场，+6% 目标）",
                    )
                },
                Period::Mid => {
                    let el = read_f64(vars, "wl_mid_entry_low", 0.97);
                    let eh = read_f64(vars, "wl_mid_entry_high", 1.03);
                    let sl = read_f64(vars, "wl_mid_stop", 0.92);
                    let tg = read_f64(vars, "wl_mid_target", 1.15);
                    let bp = read_f64(vars, "wl_mid_base_pos", 6.0);
                    (
                        price * el,
                        price * eh,
                        price * sl,
                        price * tg,
                        bp,
                        "候选池初筛（中线：-3% 进场，+15% 目标）",
                    )
                },
                Period::Long => {
                    let el = read_f64(vars, "wl_long_entry_low", 0.95);
                    let eh = read_f64(vars, "wl_long_entry_high", 1.05);
                    let sl = read_f64(vars, "wl_long_stop", 0.88);
                    let tg = read_f64(vars, "wl_long_target", 1.25);
                    let bp = read_f64(vars, "wl_long_base_pos", 8.0);
                    (
                        price * el,
                        price * eh,
                        price * sl,
                        price * tg,
                        bp,
                        "候选池初筛（长线：-5% 进场，+25% 目标）",
                    )
                },
            };

        // 信心度低（0.55），因为没有技术信号支撑
        let conf = calc_confidence(
            read_f64(vars, "wl_conf_consistency", 0.55),
            read_f64(vars, "wl_conf_signal", 0.5),
            read_f64(vars, "wl_conf_direction", 0.5),
            read_f64(vars, "wl_conf_market", 0.0),
            read_f64(vars, "wl_conf_base", 1.0),
        );
        // P1-12: Watchlist 兜底也不是永远输出——当置信度低于阈值时跳过
        let min_conf = read_f64(vars, "wl_min_confidence", 30.0) as u8;
        if conf < min_conf {
            return None;
        }
        let position = calc_position(base_position, conf);

        Some(RecoPick {
            stock_code: code.into(),
            stock_name: name.into(),
            sector,
            style: Style::Watchlist,
            period: self.period,
            price,
            entry_low,
            entry_high,
            stop_loss,
            target_price,
            position_pct: position,
            holding_days: self.period.default_holding_days(),
            confidence: conf,
            reasons: vec![reason.to_string()],
            risk_notes: vec![
                "初筛列表，无技术信号，请结合其他风格确认".to_string(),
                "若数据源恢复可重新拉取获取更准确入场区间".to_string(),
            ],
            secondary_styles: vec![],
            confidence_percentile: None,
            prior_source: None,
            prior_samples: None,
            evidence_score: None,
            stop_source: None,
            position_source: None,

            synthetic: true,
        })
    }
}

#[async_trait]
impl RecommendStrategy for WatchlistStrategy {
    fn id(&self) -> &'static str {
        match self.period {
            Period::UltraShort => "watchlist_ultra_short",
            Period::Short => "watchlist_short",
            Period::Mid => "watchlist_mid",
            Period::Long => "watchlist_long",
        }
    }
    fn style(&self) -> Style {
        Style::Watchlist
    }
    fn period(&self) -> Period {
        self.period
    }
    fn required_vendors(&self) -> &'static [&'static str] {
        // 任意实时 quote vendor 即可
        &["tencent", "eastmoney", "mootdx"]
    }

    async fn scan(&self, ctx: &RecoContext<'_>) -> Result<Vec<RecoPick>, String> {
        let mut picks = Vec::new();
        // Watchlist 仅作为 fallback，扫描前 30 只即可满足 10 个 pick 上限
        // 限制扫描范围减少冗余 quote API 调用，降低 vendor 压力。
        //
        // 但「前 30」必须**先放有定档的票**：用户标的并入后池子约 150 只，
        // 若只按池序截断，真正被分析定档过的那几只可能排在 30 名之外 ——
        // 于是「按档归位」这条接线对它们永久不生效（2026-10-19 实测：
        // 002696 在池、有 as_of 定档，但 watchlist 根本没扫到它）。
        let tiers_var = ctx.vars.get("reco_analysis_tiers");
        let tier_codes = analysis_tier_codes(tiers_var);
        let ordered = order_scan_codes(ctx.seed, &tier_codes, 30);
        for (code, name, sector) in ordered {
            let _g = ctx.per_code_locks.lock_for(code).await;
            if let Some(p) = self.scan_one(ctx.client, code, name, sector.clone(), ctx.vars).await {
                picks.push(p);
            }
        }
        Ok(apply_analysis_tier(self.period, picks, tiers_var))
    }
}

/// 扫描队列：**有定档的票排最前**，其余按池序补足 `cap`（去重、保序）。
fn order_scan_codes<'a>(
    seed: &'a [SeedItem],
    tier_codes: &std::collections::HashSet<&str>,
    cap: usize,
) -> Vec<&'a SeedItem> {
    let mut out: Vec<&SeedItem> =
        seed.iter().filter(|(code, _, _)| tier_codes.contains(code.as_str())).collect();
    for item in seed {
        if out.len() >= cap {
            break;
        }
        if !tier_codes.contains(item.0.as_str()) {
            out.push(item);
        }
    }
    out.truncate(cap);
    out
}

/// 从定档变量里取出全部 code（用于把「有定档的票」排到扫描队列最前）。
fn analysis_tier_codes(tiers: Option<&Value>) -> std::collections::HashSet<&str> {
    tiers
        .and_then(|v| v.as_object())
        .map(|m| m.keys().map(String::as_str).collect())
        .unwrap_or_default()
}

/// 把「分析侧逐档结论」应用到 watchlist 出票（2026-10-19 立，2026-10-20 改为逐档）。
///
/// 变量形态（`reco_loop::load_analysis_tier_var` 产出）：
/// `{ "<code>": { "ultra_short"|"short"|"mid"|"long": {"action","confidence"}, "asOfDate": "2026-09-28" } }`
///
/// 规则（**只看本档**）：
/// 1. 本档**没有结论** ⇒ 原样保留、维持 synthetic（没有依据就不升级，也不因此隐藏）；
/// 2. 本档结论是 卖出/减持 ⇒ **丢**（与 `gates` 的回环否决同向，覆盖全部档）；
/// 3. 其余 ⇒ 保留，把「本档结论」插到 reasons 首位（来源可追溯），并把 `synthetic`
///    翻成 **false**（见下）。
///
/// **为什么不再按“主档”指派**（2026-10-20 用户裁定）：四周期是**四个相互独立的预测**，
/// 同一只票可以「超短判卖出、中线判持有」。旧实现用 `decision_time_horizon`（按置信度
/// 选出来的一个主档）决定它属于哪一档 ⇒ 另外三档即使自己判了「买入」也永远进不去，
/// 这不是按档归位而是单档指派，直接违背四档独立。现在由**本档自己的结论**决定去留。
///
/// **为什么定了结论要升级成真实票（2026-10-20 实测）**：本策略的出票过去**无条件**标
/// `synthetic: true`（文件头：入场位是现价乘数、无技术信号），于是面板
/// `filter(p => !p.synthetic)` 把整个「系统初筛」桶连同**带分析结论的那几条**一起藏掉 ——
/// 库里 `as_of=2026-09-28` 的 002696 明明带着「【分析定档】…结论 持有」落了库，
/// 用户在四个档里都看不到它 ⇒ 接线在呈现层归零。定档命中意味着这条票的**动作有
/// completed 分析做依据**，不再是纯现价合成物，因此按真实票对待：随之进入 T+N 验证与
/// 策略胜率分母、推送候选 —— 这是「有依据」应当承担的后果，不是副作用。
///
/// 为什么放在 watchlist 而不是新开策略：这一桶本来就是「候选池里值得看一眼的票」，
/// 语义与「分析结论说它这档该怎么做」一致；新开风格会另立一套权重与胜率口径。
fn apply_analysis_tier(
    period: Period,
    picks: Vec<RecoPick>,
    tiers: Option<&Value>,
) -> Vec<RecoPick> {
    use serde_json::Value as JsonValue;
    let Some(map) = tiers.and_then(|v: &JsonValue| v.as_object()) else {
        return picks;
    };
    picks
        .into_iter()
        .filter_map(|mut p| {
            let Some(entry) = map.get(&p.stock_code) else {
                return Some(p);
            };
            // 本档没有结论 ⇒ 不升级、不隐藏（别的档有结论与这一档无关）
            let Some(verdict) = entry.get(period.as_str()) else {
                return Some(p);
            };
            let action = verdict.get("action").and_then(|v| v.as_str()).unwrap_or("");
            if action.is_empty() {
                return Some(p);
            }
            if matches!(action, "卖出" | "减持") {
                return None;
            }
            let conf = verdict.get("confidence").and_then(|v| v.as_f64()).unwrap_or(0.0);
            let as_of = entry.get("asOfDate").and_then(|v| v.as_str()).unwrap_or("");
            p.reasons.insert(
                0,
                format!(
                    "【分析定档】{as_of} 分析对{}的结论：{action}（置信 {conf:.1}）",
                    period.label_zh()
                ),
            );
            p.synthetic = false;
            Some(p)
        })
        .collect()
}

/// 数据稀疏兜底：为指定 style 用 `get_quote` 拉取基础行情 emit 合成 picks
///
/// 调用场景：vendor 的 K 线 / 财务 / 资金流数据全不可用时，4 个主策略
/// (trend/value/capital/reversion) 都返回空，导致面板上对应 style 桶为空。
/// 该函数保证 5 个 style 桶都有数据展示，避免出现"只有 Watchlist 有 10 条，
/// 其他 4 个全空"的局面。
///
/// 合成 pick 的 `reasons` 明确标注"信号缺失，按现价合成"，方便用户区分
/// 真实信号 vs 兜底。`confidence` 偏低（0.45），让真实 pick 排序优先。
pub async fn emit_synthetic_picks(
    client: Arc<dyn MarketDataProvider>,
    style: Style,
    period: Period,
    raw_seed: &[SeedItem],
    per_code_locks: Arc<PerCodeLocks>,
    vars: &HashMap<String, Value>,
) -> Vec<RecoPick> {
    let mut picks = Vec::new();
    // 合成兜底只需前 30 只即可满足 10 个 pick 上限
    for (code, name, sector) in raw_seed.iter().take(30) {
        let _g = per_code_locks.lock_for(code).await;
        if let Some(p) =
            scan_synthetic_one(client.as_ref(), code, name, sector.clone(), style, period, vars)
                .await
        {
            picks.push(p);
        }
    }
    picks
}

async fn scan_synthetic_one(
    client: &dyn MarketDataProvider,
    code: &str,
    name: &str,
    sector: Option<String>,
    style: Style,
    period: Period,
    vars: &HashMap<String, Value>,
) -> Option<RecoPick> {
    let quote = client.get_quote(code).await.ok()?;
    if quote.is_st {
        return None;
    }
    if quote.price <= 0.0 {
        return None;
    }
    let price = quote.price;

    let style_label = match style {
        Style::Trend => "趋势跟踪",
        Style::Value => "价值低估",
        Style::Capital => "资金驱动",
        Style::Reversion => "超跌反弹",
        Style::Watchlist => "系统初筛",
        Style::Bottleneck => "供给瓶颈",
        Style::Policy => "政策驱动",
        Style::Earnings => "业绩驱动",
        Style::CapitalFlow => "资金驱动",
        Style::Event => "事件驱动",
        Style::Technical => "技术驱动",
    };

    let (entry_low, entry_high, stop_loss, target_price, base_position, reason) = match period {
        Period::UltraShort => {
            let el = read_f64(vars, "syn_ultra_short_entry_low", 0.998);
            let eh = read_f64(vars, "syn_ultra_short_entry_high", 1.005);
            let sl = read_f64(vars, "syn_ultra_short_stop", 0.98);
            let tg = read_f64(vars, "syn_ultra_short_target", 1.03);
            let bp = read_f64(vars, "syn_ultra_short_base_pos", 2.0);
            (
                price * el,
                price * eh,
                price * sl,
                price * tg,
                bp,
                format!("候选池初筛（超短线 — {} 信号缺失，按现价合成）", style_label),
            )
        },
        Period::Short => {
            let el = read_f64(vars, "syn_short_entry_low", 0.99);
            let eh = read_f64(vars, "syn_short_entry_high", 1.01);
            let sl = read_f64(vars, "syn_short_stop", 0.96);
            let tg = read_f64(vars, "syn_short_target", 1.06);
            let bp = read_f64(vars, "syn_short_base_pos", 4.0);
            (
                price * el,
                price * eh,
                price * sl,
                price * tg,
                bp,
                format!("候选池初筛（短线 — {} 信号缺失，按现价合成）", style_label),
            )
        },
        Period::Mid => {
            let el = read_f64(vars, "syn_mid_entry_low", 0.97);
            let eh = read_f64(vars, "syn_mid_entry_high", 1.03);
            let sl = read_f64(vars, "syn_mid_stop", 0.92);
            let tg = read_f64(vars, "syn_mid_target", 1.15);
            let bp = read_f64(vars, "syn_mid_base_pos", 6.0);
            (
                price * el,
                price * eh,
                price * sl,
                price * tg,
                bp,
                format!("候选池初筛（中线 — {} 信号缺失，按现价合成）", style_label),
            )
        },
        Period::Long => {
            let el = read_f64(vars, "syn_long_entry_low", 0.95);
            let eh = read_f64(vars, "syn_long_entry_high", 1.05);
            let sl = read_f64(vars, "syn_long_stop", 0.88);
            let tg = read_f64(vars, "syn_long_target", 1.25);
            let bp = read_f64(vars, "syn_long_base_pos", 8.0);
            (
                price * el,
                price * eh,
                price * sl,
                price * tg,
                bp,
                format!("候选池初筛（长线 — {} 信号缺失，按现价合成）", style_label),
            )
        },
    };

    // 信心度低（0.45），比 Watchlist 真实初筛（0.55）更低，方便排序时真实 pick 优先
    let conf = calc_confidence(
        read_f64(vars, "syn_conf_consistency", 0.45),
        read_f64(vars, "syn_conf_signal", 0.4),
        read_f64(vars, "syn_conf_direction", 0.4),
        read_f64(vars, "syn_conf_market", 0.0),
        read_f64(vars, "syn_conf_base", 1.0),
    );
    // P1-12: 兜底也不永远输出——置信度不足时跳过
    let min_conf = read_f64(vars, "syn_min_confidence", 25.0) as u8;
    if conf < min_conf {
        return None;
    }
    let position = calc_position(base_position, conf);

    Some(RecoPick {
        stock_code: code.into(),
        stock_name: name.into(),
        sector,
        style,
        period,
        price,
        entry_low,
        entry_high,
        stop_loss,
        target_price,
        position_pct: position,
        holding_days: period.default_holding_days(),
        confidence: conf,
        reasons: vec![reason],
        risk_notes: vec![
            format!(
                "{} 风格专属信号缺失（K 线 / 财务 / 资金数据不可用），本条为按现价合成的兜底结果",
                style_label
            ),
            "数据源恢复后可重新拉取获取更准确入场区间".to_string(),
        ],
        secondary_styles: vec![],
        confidence_percentile: None,
        prior_source: None,
        prior_samples: None,
        evidence_score: None,
        stop_source: None,
        position_source: None,

        synthetic: true,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn watchlist_strategy_ids() {
        assert_eq!(WatchlistStrategy::short().id(), "watchlist_short");
        assert_eq!(WatchlistStrategy::mid().id(), "watchlist_mid");
        assert_eq!(WatchlistStrategy::long().id(), "watchlist_long");
        assert_eq!(WatchlistStrategy::short().style(), Style::Watchlist);
    }
}

#[cfg(test)]
mod analysis_tier_tests {
    use super::*;

    fn pick(code: &str) -> RecoPick {
        RecoPick {
            stock_code: code.into(),
            stock_name: code.into(),
            sector: None,
            style: Style::Watchlist,
            period: Period::Mid,
            price: 10.0,
            entry_low: 9.9,
            entry_high: 10.1,
            stop_loss: 9.5,
            target_price: 10.6,
            position_pct: 2.0,
            // 夹具也走权威来源：单源门把「测试里直接写 5」同样视为第二份天数表
            holding_days: Period::Mid.default_holding_days(),
            confidence: 40,
            reasons: vec!["初筛".into()],
            risk_notes: vec![],
            secondary_styles: vec![],
            confidence_percentile: None,
            prior_source: None,
            prior_samples: None,
            evidence_score: None,
            stop_source: None,
            position_source: None,
            // 生产形态：本策略的出票一律 synthetic=true（见文件头），
            // 夹具若写 false，「是否升级」这条断言就恒真。
            synthetic: true,
        }
    }

    /// 四格必须同时成立：**只看本档**的结论（别档的结论不参与）、本档卖出/减持要挡掉、
    /// 本档没有结论的不得被误删也不得被升级；保留且有结论的那条把来源写进 reasons。
    #[test]
    fn analysis_tier_gates_by_this_tier_verdict_and_annotates_source() {
        let v = serde_json::json!({
            // 四档独立：同一只票超短判卖出、中线判持有
            "002696": {
                "ultra_short": {"action":"卖出","confidence":19.9},
                "mid": {"action":"持有","confidence":60.5},
                "asOfDate":"2026-09-28"
            },
            // 只有超短的结论 ⇒ 对中线而言是「本档无结论」
            "600001": {"ultra_short":{"action":"买入","confidence":70.0},"asOfDate":"2026-09-28"},
            "600002": {"mid":{"action":"卖出","confidence":50.0},"asOfDate":"2026-09-28"},
        });
        let out = apply_analysis_tier(
            Period::Mid,
            vec![pick("002696"), pick("600001"), pick("600002"), pick("600003")],
            Some(&v),
        );
        let codes: Vec<&str> = out.iter().map(|p| p.stock_code.as_str()).collect();
        assert_eq!(
            codes,
            vec!["002696", "600001", "600003"],
            "本档卖出必须挡掉；本档没结论的（别档有结论/完全没分析）必须原样保留"
        );
        assert!(
            out[0].reasons[0].contains("【分析定档】") && out[0].reasons[0].contains("持有"),
            "本档结论必须写进首条理由：{:?}",
            out[0].reasons
        );
        // 呈现层升级：面板 `filter(p => !p.synthetic)` 会藏掉所有 synthetic 票，
        // 本档有结论的那条若不翻成 false，落库了也永远看不见（2026-10-20 实测 002696）。
        assert!(!out[0].synthetic, "本档有结论且非卖减持 ⇒ 必须是真实票");
        assert!(out[1].synthetic, "只有别档结论的票对本档没有依据，不得升级");
        assert!(out[2].synthetic, "未定档的初筛票不得被顺手升级");

        // 同一份变量喂给超短档：002696 因**本档**卖出被丢、600001 因**本档**买入被升级 ——
        // 这是「四档独立」的直接判据：同一只票在不同档的结论可以相反。
        let us = apply_analysis_tier(
            Period::UltraShort,
            vec![pick("002696"), pick("600001"), pick("600002")],
            Some(&v),
        );
        let us_codes: Vec<&str> = us.iter().map(|p| p.stock_code.as_str()).collect();
        assert_eq!(us_codes, vec!["600001", "600002"], "超短档按自己的结论留弃");
        assert!(!us[0].synthetic, "600001 超短判买入 ⇒ 在超短档升级");
        assert!(us[1].synthetic, "600002 超短无结论 ⇒ 不升级");

        // 负控：变量缺席时一条都不许消失（不得把「没有分析」当成「该档不符」）
        let keep = apply_analysis_tier(Period::Mid, vec![pick("002696")], None);
        assert_eq!(keep.len(), 1);
        assert!(keep[0].synthetic, "没有本档结论时保持 synthetic=true");
    }
}

#[cfg(test)]
mod scan_order_tests {
    use super::*;

    fn seed(code: &str) -> SeedItem {
        (code.into(), code.into(), None)
    }

    /// 有定档的票必须排到扫描队列最前（否则 150 只池子按序截 30 只时它永远扫不到），
    /// 其余按池序补足上限；负控=没有定档时队列与池序完全一致（不得凭空重排）。
    #[test]
    fn tier_codes_are_scanned_first_then_pool_order() {
        let pool = vec![seed("A"), seed("B"), seed("C"), seed("D")];
        let tiers: std::collections::HashSet<&str> = ["C"].into_iter().collect();
        let ordered: Vec<&str> =
            order_scan_codes(&pool, &tiers, 3).iter().map(|s| s.0.as_str()).collect();
        assert_eq!(ordered, vec!["C", "A", "B"], "定档票先扫，其余按池序补足上限");

        let none: std::collections::HashSet<&str> = std::collections::HashSet::new();
        let ordered2: Vec<&str> =
            order_scan_codes(&pool, &none, 3).iter().map(|s| s.0.as_str()).collect();
        assert_eq!(ordered2, vec!["A", "B", "C"], "没有定档时不得改变原有顺序");
    }
}
