//! 资金驱动子策略：主力净流入 / 北向加仓 / 龙虎榜
//!
//! 注：v1 资金流向数据仅当日，逻辑上做了"日级窗口"近似；
//! 真正多日窗口需后续扩 AStockClient 接口。

use super::super::strategy::{RecoContext, RecommendStrategy, read_f64};
use crate::recommender::scoring::{calc_confidence, calc_position};
use crate::recommender::types::{Period, RecoPick, Style};
use async_trait::async_trait;
use axagent_astock_data::AStockClient;
use serde_json::Value;
use std::collections::HashMap;

/// capital 个股证据强度（0~1，2026-10-18 四档常数塌缩修复）。
///
/// 换手率强度 50%（0~10% 满格）+ 主力净流入强度 50%（0~5000 万满格）——
/// 归一化刻度非过滤阈值。负值/非有限按 0 处理（clamp 语义）。
fn capital_evidence_strength(turnover_rate: f64, main_inflow_wan: f64) -> f64 {
    let vol = (turnover_rate.max(0.0) / 10.0).min(1.0);
    let inflow = (main_inflow_wan.max(0.0) / 5000.0).min(1.0);
    (0.5 * vol + 0.5 * inflow).clamp(0.0, 1.0)
}

/// 融资余额趋势（P1 影子腿判据，纯函数）：最近 `window` 个交易日相对 `window` 日前的
/// 变化率（%）。输入序列日期任意序（内部按 date 倒序取两端，vendor 返回倒序不信任）。
/// 样本不足（< window+1 条）或窗口首点非正（坏数据）⇒ None —— 影子腿 fail-open：
/// 数据问题不拦出票、不刷日志，缺席即声明（`PLAN-capital-long-evidence.md` E3）。
fn margin_balance_trend_pct(
    series: &[axagent_astock_data::MarginData],
    window: usize,
) -> Option<f64> {
    if series.len() < window + 1 {
        return None;
    }
    let mut sorted: Vec<&axagent_astock_data::MarginData> = series.iter().collect();
    sorted.sort_by(|a, b| b.date.cmp(&a.date));
    let newest = sorted.first()?.margin_balance;
    let oldest = sorted.get(window)?.margin_balance;
    if oldest <= 0.0 {
        return None;
    }
    Some((newest / oldest - 1.0) * 100.0)
}

/// 大宗交易折价画像（P3 影子腿判据 E4，纯函数）：以序列内最新成交日为锚，
/// 取锚点前 `window_days` 个自然日窗口，产出笔数 / 折溢价率中位数 / 成交额合计（亿）/
/// 买方去重家数。**数据源 = 既有 `client.get_block_trades`**（`BlockTrade` 自带
/// `discount_pct` 折溢价与 as-of 截断——回放语义免费获得，见 lib.rs）。
/// 窗口内 < [`BLOCK_TRADE_MIN_COUNT`] 笔 ⇒ None；`discount_pct` 缺失的笔不进中位数
///（披露口径差异，fail-open 不编数）。
#[derive(Debug, Clone, PartialEq)]
struct BlockTradeProfile {
    count: usize,
    /// 折溢价率中位数（%，负=折价）
    median_premium_pct: f64,
    /// 成交金额合计（亿元）
    total_amount_yi: f64,
    /// 买方去重家数（买方集中的原材料）
    distinct_buyers: usize,
}

const BLOCK_TRADE_MIN_COUNT: usize = 3;

fn block_trade_discount_profile(
    series: &[axagent_astock_data::BlockTrade],
    window_days: u32,
) -> Option<BlockTradeProfile> {
    if series.is_empty() {
        return None;
    }
    // 锚点 = 序列内最新成交日（不依赖时钟：live/回放语义自洽）
    let anchor = series.iter().map(|p| p.trade_date.as_str()).max()?;
    let anchor_date = chrono::NaiveDate::parse_from_str(anchor, "%Y-%m-%d").ok()?;
    let cutoff =
        (anchor_date - chrono::Duration::days(window_days as i64)).format("%Y-%m-%d").to_string();
    let in_window: Vec<&axagent_astock_data::BlockTrade> =
        series.iter().filter(|p| p.trade_date.as_str() >= cutoff.as_str()).collect();
    if in_window.len() < BLOCK_TRADE_MIN_COUNT {
        return None;
    }
    let premiums: Vec<f64> = in_window.iter().filter_map(|p| p.discount_pct).collect();
    if premiums.len() < BLOCK_TRADE_MIN_COUNT {
        return None;
    }
    let mut sorted_p = premiums.clone();
    sorted_p.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let mid = sorted_p.len() / 2;
    let median_premium_pct = if sorted_p.len() % 2 == 1 {
        sorted_p[mid]
    } else {
        (sorted_p[mid - 1] + sorted_p[mid]) / 2.0
    };
    let total_amount_yi = in_window.iter().map(|p| p.amount).sum::<f64>() / 1e8;
    let mut buyers: Vec<&str> =
        in_window.iter().map(|p| p.buyer_dept.as_deref().unwrap_or("")).collect();
    buyers.sort_unstable();
    buyers.dedup();
    Some(BlockTradeProfile {
        count: in_window.len(),
        median_premium_pct,
        total_amount_yi,
        distinct_buyers: buyers.len(),
    })
}

pub struct CapitalStrategy {
    pub period: Period,
}

impl CapitalStrategy {
    pub const fn ultra_short() -> Self {
        Self { period: Period::UltraShort }
    }
    pub const fn short() -> Self {
        Self { period: Period::Short }
    }
    pub const fn mid() -> Self {
        Self { period: Period::Mid }
    }
    // as-of K线代理回退

    /// as-of 模式下 money_flow / north_bound 不可用，回退到 K 线量价检测
    async fn scan_from_klines(
        &self,
        client: &AStockClient,
        code: &str,
        name: &str,
        sector: Option<String>,
        vars: &HashMap<String, Value>,
    ) -> Option<RecoPick> {
        let quote = client.get_quote(code).await.ok()?;
        let price = quote.price;
        // 尺度由档位决定（K线代理回退也须按档取尺度，否则「长档看 20 个交易日」这种
        // 与 90 天持有期脱节的口径会继续留在产出里）。
        let (klines, profile) =
            // ⚠ R-3e：声明 12 天而不是 20 —— 12 是**各档 fetch_limit 所覆盖天数的最小值**
            //   （hourly 48 根 × 0.25 = 12 天；daily 30 天 / weekly 180 天 / monthly 600 天）。
            //   取该最小值 ⇒ 各档换算后都不超 fetch_limit ⇒ **取数根数逐字不变**（零回归）。
            //   `w20` 在 hourly 档仍会超（80 根 vs 48 根）⇒ 那是**既有偏差**，不在本次修。
            match crate::recommender::scale::fetch(client, code, self.period, 12).await {
                Ok(v) => v,
                Err(e) => {
                    tracing::debug!("[capital] {code} 档位 {:?} 取数失败: {e}", self.period);
                    return None;
                },
            };
        if !crate::recommender::scale::enough_bars(&klines, self.period) {
            return None;
        }

        // 量价信号：最近 5 日平均量 / 20 日平均量 > 阈值
        // 窗口按**日历日**换算到当前尺度（5 日 / 20 日），并保证至少 5 根 bar 才算「平均」
        let w5 = (crate::recommender::scale::bars_for_daily_span(5, &profile)).max(5);
        let w20 = (crate::recommender::scale::bars_for_daily_span(20, &profile)).max(5);
        let vols_5: Vec<f64> = klines.iter().rev().take(w5).map(|k| k.amount).collect();
        let avg_vol_5 = vols_5.iter().sum::<f64>() / (vols_5.len() as f64).max(1.0);
        let vols_20: Vec<f64> = klines.iter().rev().take(w20).map(|k| k.amount).collect();
        let avg_vol_20 = vols_20.iter().sum::<f64>() / (vols_20.len() as f64).max(1.0);
        let vol_ratio = if avg_vol_20 > 0.0 {
            avg_vol_5 / avg_vol_20
        } else {
            1.0
        };

        // 价格动量：近 w5 根 bar 的涨幅（= 日历 5 个交易日换算来的 bar 数）
        let closes: Vec<f64> = klines.iter().map(|k| k.close).collect();
        let mom_5 = if closes.len() > w5 {
            closes[closes.len() - 1] / closes[closes.len() - 1 - w5] - 1.0
        } else {
            0.0
        };

        let vol_ratio_min = read_f64(vars, "cap_kline_vol_ratio_min", 1.5);
        let mom_5_min = read_f64(vars, "cap_kline_mom_5_min", -0.02);
        let mom_5_max = read_f64(vars, "cap_kline_mom_5_max", 0.10);

        if vol_ratio < vol_ratio_min || mom_5 < mom_5_min || mom_5 > mom_5_max {
            return None;
        }

        // 计算置信度（量比贡献 0.6，动量贡献 0.4）
        let vol_score = ((vol_ratio - 1.0) / 3.0).clamp(0.0, 1.0);
        let mom_score = ((mom_5 - mom_5_min) / (mom_5_max - mom_5_min)).clamp(0.0, 1.0);
        let conf_raw = 0.6 * vol_score + 0.4 * mom_score;

        // 持有天数一律取 `Period::default_holding_days()`（全仓唯一天数源，`harness::holding_period`）。
        // 此前这里自带一张逐档表且 short=**7**，而权威源 short=**5** ⇒ 荐股按 7 天判成熟、
        // 反思/回测按 5 天分档，两处各自都「没 bug」但互相错档（门 a 的第一条靶）。
        let (entry_low, entry_high, stop_loss, target, base_position) = match self.period {
            Period::UltraShort => (price * 0.998, price * 1.005, price * 0.97, price * 1.05, 3.0),
            Period::Short => (price * 0.97, price * 1.03, price * 0.93, price * 1.10, 5.0),
            Period::Mid => (price * 0.95, price * 1.05, price * 0.90, price * 1.20, 8.0),
            Period::Long => (price * 0.95, price * 1.05, price * 0.88, price * 1.30, 10.0),
        };

        let conf = (conf_raw * 100.0).round() as u8;
        let position = calc_position(base_position, conf);

        Some(RecoPick {
            stock_code: code.into(),
            stock_name: name.into(),
            sector,
            style: Style::Capital,
            period: self.period,
            price,
            entry_low,
            entry_high,
            stop_loss,
            target_price: target,
            position_pct: position,
            holding_days: self.period.default_holding_days(),
            confidence: conf,
            reasons: vec![
                format!("K线量比 {:.2}x", vol_ratio),
                format!("5日动量 {:.2}%", mom_5 * 100.0),
            ],
            risk_notes: vec!["K线代理模式：无资金流向数据，仅基于量价".to_string()],
            secondary_styles: vec![],
            confidence_percentile: None,
            prior_source: None,
            prior_samples: None,
            evidence_score: None,
            stop_source: None,
            position_source: None,

            // ⚠ 本分支是**降级占位**（无资金流，仅量价代理）⇒ 恒 `synthetic: true`。
            //   回放（as-of）语义下这不是偶发：`get_money_flow` 的 as-of 通道只放
            //   NativeDateParam 源（eastmoney/快照），失败即返回 None ⇒ 本函数被调用
            //   （`scan_one` 的 `mf.as_ref().is_none_or(|m| !mf_is_effective(m))` 分支），
            //   于是 capital 在回放里产出的每一行都不是 `synthetic=0`。
            //   2026-10-19 现网读数（as_of=2026-09-28，41 批）：capital×long 真行累计 71 条，
            //   且集中在资金流 as-of 通道可用的少数批次（14:14–21:46）；22:01 之后各批 0 条 ——
            //   与 min_confidence（=50）/ regime 硬门 / enough_bars 都无关。
            //   处置：这是「没有资金流就不冒充资金驱动」的如实标注，**不得**改成 false。
            synthetic: true,
        })
    }

    pub const fn long() -> Self {
        Self { period: Period::Long }
    }

    async fn scan_one(
        &self,
        client: &AStockClient,
        code: &str,
        name: &str,
        sector: Option<String>,
        vars: &HashMap<String, Value>,
    ) -> Option<RecoPick> {
        let quote = client.get_quote(code).await.ok()?;
        let price = quote.price;

        let mf = client.get_money_flow(code).await.ok().flatten();

        // 检测"空壳" MoneyFlow：Tencent 等vendor 返回了对象但关键字段全为 0，
        // 这种情况等同于数据不可用，应回退到 K 线量价检测。
        let mf_is_effective = |m: &axagent_astock_data::MoneyFlow| -> bool {
            // None = 该源未披露这一档 ⇒ 与 0 一样不计入「有内容」，不改变原有判定语义
            m.main_net_inflow != 0.0
                || m.super_large_net.unwrap_or(0.0) != 0.0
                || m.large_net.unwrap_or(0.0) != 0.0
                || m.medium_net.unwrap_or(0.0) != 0.0
                || m.small_net.unwrap_or(0.0) != 0.0
        };

        // eastmoney 资金流向被反爬拦截，或 vendor 返回零值空壳数据时回退到 K 线量价检测。
        //
        // 顺序要求：本判断必须在拉 north_bound / dragon_tiger **之前**。
        // 旧实现无条件先拉三个源再判断，导致"资金流已注定走 K 线代理"的股票
        // 仍要付出 north_bound + dragon_tiger 的完整请求与重试代价
        // （北向个股持仓已全源失效，单只冷启动 3 源 × 3 轮重试实测 ≈ 8.5s；
        //   筛池 80+ 只累加即达分钟级，期间租户只能干等）。
        if mf.as_ref().is_none_or(|m| !mf_is_effective(m)) {
            return self.scan_from_klines(client, code, name, sector, vars).await;
        }

        // 仅当资金流确实有效、要走资金驱动路径时，才继续拉另外两个源
        let nb = client.get_north_bound_holding(code).await.ok().flatten();
        let dt = client.get_dragon_tiger(code).await.ok();

        // 三个资金数据源在 as-of 下可能全部不可用，回退到 K 线量价检测
        if nb.is_none() && dt.as_ref().is_none_or(|e| e.is_empty()) {
            return self.scan_from_klines(client, code, name, sector, vars).await;
        }

        // 主力净流入
        let main_inflow_wan = mf.as_ref().map(|m| m.main_net_inflow / 10_000.0).unwrap_or(0.0);
        // 北向持仓占比
        let nb_ratio = nb.as_ref().map(|n| n.holding_ratio).unwrap_or(0.0);
        // 龙虎榜净买入
        let dt_net_wan = dt
            .as_ref()
            .map(|entries| entries.iter().map(|e| e.net_amount).sum::<f64>() / 10_000.0)
            .unwrap_or(0.0);

        let (pass, reasons) = match self.period {
            Period::UltraShort => {
                // 超短线只看龙虎榜 — 游资隔夜行为
                let dt_net_min = read_f64(vars, "cap_ultra_short_dt_net_min", 100.0);
                if dt_net_wan.abs() < dt_net_min {
                    return None;
                }
                let turnover_min = read_f64(vars, "cap_ultra_short_turnover_min", 5.0);
                if quote.turnover_rate < turnover_min {
                    return None;
                }
                let mut r = Vec::new();
                if dt_net_wan > 0.0 {
                    r.push(format!("龙虎榜净买入 {:.0} 万", dt_net_wan));
                } else {
                    r.push(format!("龙虎榜净卖出 {:.0} 万（反弹博弈）", dt_net_wan.abs()));
                }
                r.push(format!("换手 {:.2}%", quote.turnover_rate));
                (true, r)
            },
            Period::Short => {
                let main_inflow_min = read_f64(vars, "cap_short_main_inflow_min", 200.0);
                if main_inflow_wan < main_inflow_min {
                    return None;
                }
                let turnover_min = read_f64(vars, "cap_short_turnover_min", 2.0);
                if quote.turnover_rate < turnover_min {
                    return None;
                }
                (
                    true,
                    vec![
                        format!("主力净流入 {:.0} 万", main_inflow_wan),
                        format!("换手 {:.2}%", quote.turnover_rate),
                    ],
                )
            },
            Period::Mid => {
                let nb_ratio_min = read_f64(vars, "cap_mid_nb_ratio_min", 0.3);
                let main_inflow_min = read_f64(vars, "cap_mid_main_inflow_min", 500.0);
                if nb_ratio < nb_ratio_min && main_inflow_wan < main_inflow_min {
                    return None;
                }
                let mut r = Vec::new();
                if main_inflow_wan > 0.0 {
                    r.push(format!("主力净流入 {:.0} 万", main_inflow_wan));
                }
                if nb_ratio > 0.0 {
                    r.push(format!("北向持仓 {:.2}%", nb_ratio));
                }
                if dt_net_wan.abs() > 0.0 {
                    r.push(format!("龙虎榜净买入 {:.0} 万", dt_net_wan));
                }
                (true, r)
            },
            Period::Long => {
                let nb_ratio_min = read_f64(vars, "cap_long_nb_ratio_min", 0.1);
                let main_inflow_min = read_f64(vars, "cap_long_main_inflow_min", 100.0);
                if nb_ratio < nb_ratio_min && main_inflow_wan < main_inflow_min {
                    return None;
                }
                let mut r = if nb_ratio > 0.0 {
                    vec![format!("北向长期持仓 {:.2}%", nb_ratio)]
                } else {
                    vec![format!("主力长期净流入 {:.0} 万", main_inflow_wan)]
                };
                // ── P1 影子腿（PLAN-capital-long-evidence.md E3）──
                // 融资余额 20 日趋势：只进 reasons 作信息证据，**不 gate、不加权**
                //（转正在 P4 换血；本分支旧判据的退役也随 P4 一并做，此处不调旧阈值）。
                // 数据不可得/非两融标的 ⇒ 静默缺席（影子腿缺席是常态，不逐票刷日志）。
                // as-of 回放下序列恒空（client 层声明缺口），影子腿自然缺席。
                if let Ok(series) = client.get_margin_balance_trend(code, 30).await {
                    if let Some(pct) = margin_balance_trend_pct(&series, 20) {
                        r.push(format!("融资余额20日趋势 {pct:+.1}%"));
                    }
                }
                // ── P2 影子腿（PLAN-capital-long-evidence.md E1）──
                // 数据源 = **既有** `client.get_holder_count`（最新一期快照， HolderCount
                // 自带 holder_num_ratio 环比；as-of 走降级账本 StructuralGap）。
                // 多期「连降 streak」需 RPT_F10_EH_HOLDERNUM 序列——既有实现已声明
                // 属下一轮，影子阶段先用单点环比。
                if let Ok(Some(hc)) = client.get_holder_count(code).await {
                    if let Some(ratio) = hc.holder_num_ratio {
                        if ratio < 0.0 {
                            r.push(format!("股东户数环比{ratio:+.1}%(筹码集中)"));
                        }
                    }
                }
                // ── P3 影子腿（PLAN-capital-long-evidence.md E4）──
                // 数据源 = **既有** `client.get_block_trades`（Vec<BlockTrade> 带
                // discount_pct 折溢价与 as-of 截断，回放语义免费）。近 60 自然日
                // 折价画像（中位数 <0 = 折价吸筹形态）：只进 reasons 不 gate；
                // 溢价画像（>0）语义是派发，影子阶段不报，P4 转正时再定。
                if let Ok(series) = client.get_block_trades(code).await {
                    if let Some(p) = block_trade_discount_profile(&series, 60) {
                        if p.median_premium_pct < 0.0 {
                            r.push(format!(
                                "近60日大宗{}笔 折价中位{:+.1}%(买方{}家 金额{:.1}亿)",
                                p.count, p.median_premium_pct, p.distinct_buyers, p.total_amount_yi
                            ));
                        }
                    }
                }
                (true, r)
            },
        };

        if !pass {
            return None;
        }

        let (entry_low, entry_high, stop_loss, target, base_position) = match self.period {
            Period::UltraShort => {
                let el = read_f64(vars, "cap_ultra_short_entry_low", 0.998);
                let eh = read_f64(vars, "cap_ultra_short_entry_high", 1.005);
                let sl = read_f64(vars, "cap_ultra_short_stop", 0.97);
                let tg = read_f64(vars, "cap_ultra_short_target", 1.05);
                let bp = read_f64(vars, "cap_ultra_short_base_pos", 3.0);
                (price * el, price * eh, price * sl, price * tg, bp)
            },
            Period::Short => {
                let el = read_f64(vars, "cap_short_entry_low", 0.97);
                let eh = read_f64(vars, "cap_short_entry_high", 1.03);
                let sl = read_f64(vars, "cap_short_stop", 0.93);
                let tg = read_f64(vars, "cap_short_target", 1.10);
                let bp = read_f64(vars, "cap_short_base_pos", 5.0);
                (price * el, price * eh, price * sl, price * tg, bp)
            },
            Period::Mid => {
                let el = read_f64(vars, "cap_mid_entry_low", 0.95);
                let eh = read_f64(vars, "cap_mid_entry_high", 1.05);
                let sl = read_f64(vars, "cap_mid_stop", 0.90);
                let tg = read_f64(vars, "cap_mid_target", 1.20);
                let bp = read_f64(vars, "cap_mid_base_pos", 8.0);
                (price * el, price * eh, price * sl, price * tg, bp)
            },
            Period::Long => {
                let el = read_f64(vars, "cap_long_entry_low", 0.95);
                let eh = read_f64(vars, "cap_long_entry_high", 1.05);
                let sl = read_f64(vars, "cap_long_stop", 0.88);
                let tg = read_f64(vars, "cap_long_target", 1.30);
                let bp = read_f64(vars, "cap_long_base_pos", 10.0);
                (price * el, price * eh, price * sl, price * tg, bp)
            },
        };

        // ── 个股证据强度（2026-10-18，四档常数塌缩修复）──
        // 换手率强度 50% + 主力净流入强度 50%（换手 0~10% 满格 / 净流入 0~5000 万满格，
        // 归一化刻度非过滤阈值）。此前 signal 槽是模板常数 ⇒ 同档原始置信无个股差异。
        // ⚠ 仅主路径（money_flow 有效）携带个股证据；K 线回退分支是 synthetic 占位，不调制。
        let evidence = capital_evidence_strength(quote.turnover_rate, main_inflow_wan);
        let conf = calc_confidence(
            read_f64(vars, "cap_conf_consistency", 0.80),
            read_f64(vars, "cap_conf_signal", 0.7) * (0.6 + 0.4 * evidence),
            read_f64(vars, "cap_conf_direction", 0.7),
            read_f64(vars, "cap_conf_market", 0.0),
            1.0, // turnover_anomaly: 资金驱动策略无量比数据（有 money_flow），默认无惩罚
        );
        let position = calc_position(base_position, conf);

        let risk = match self.period {
            Period::UltraShort => vec!["次日冲高回落 / T+1 无法止损".to_string()],
            Period::Short => vec!["次日冲高回落".to_string()],
            Period::Mid => vec!["资金切换 / 主力出货".to_string()],
            Period::Long => vec!["机构持仓变动".to_string()],
        };

        Some(RecoPick {
            stock_code: code.into(),
            stock_name: name.into(),
            sector,
            style: Style::Capital,
            period: self.period,
            price,
            entry_low,
            entry_high,
            stop_loss,
            target_price: target,
            position_pct: position,
            holding_days: self.period.default_holding_days(),
            confidence: conf,
            reasons,
            risk_notes: risk,
            secondary_styles: vec![],
            confidence_percentile: None,
            prior_source: None,
            prior_samples: None,
            evidence_score: None,
            stop_source: None,
            position_source: None,

            synthetic: false,
        })
    }
}

#[async_trait]
impl RecommendStrategy for CapitalStrategy {
    fn id(&self) -> &'static str {
        match self.period {
            Period::UltraShort => "capital_ultra_short",
            Period::Short => "capital_short",
            Period::Mid => "capital_mid",
            Period::Long => "capital_long",
        }
    }
    fn style(&self) -> Style {
        Style::Capital
    }
    fn period(&self) -> Period {
        self.period
    }
    fn required_vendors(&self) -> &'static [&'static str] {
        &["ths", "baidu_stock"]
    }

    async fn scan(&self, ctx: &RecoContext<'_>) -> Result<Vec<RecoPick>, String> {
        let mut picks = Vec::new();
        for (code, name, sector) in ctx.seed {
            let _g = ctx.per_code_locks.lock_for(code).await;
            if let Some(p) = self.scan_one(ctx.client, code, name, sector.clone(), ctx.vars).await {
                picks.push(p);
            }
        }
        Ok(picks)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capital_strategy_ids() {
        assert_eq!(CapitalStrategy::ultra_short().id(), "capital_ultra_short");
        assert_eq!(CapitalStrategy::short().id(), "capital_short");
        assert_eq!(CapitalStrategy::mid().id(), "capital_mid");
        assert_eq!(CapitalStrategy::long().id(), "capital_long");
    }

    #[test]
    fn capital_evidence_monotonic_and_clamped() {
        let base = capital_evidence_strength(5.0, 2500.0);
        assert!(base > 0.0 && base < 1.0, "中性输入应落在 (0,1)，实得 {base}");
        // 换手 ↑ / 净流入 ↑ ⇒ 证据 ↑
        assert!(capital_evidence_strength(8.0, 2500.0) > base);
        assert!(capital_evidence_strength(5.0, 4000.0) > base);
        // 满格：换手 ≥10% + 净流入 ≥5000 万 ⇒ 1.0
        assert!((capital_evidence_strength(10.0, 5000.0) - 1.0).abs() < 1e-9);
        // 零证据 / 负值防御 ⇒ 0
        assert!((capital_evidence_strength(0.0, 0.0) - 0.0).abs() < 1e-9);
        assert!((capital_evidence_strength(-1.0, -100.0) - 0.0).abs() < 1e-9);
    }

    fn margin_point(date: &str, balance: f64) -> axagent_astock_data::MarginData {
        axagent_astock_data::MarginData {
            stock_code: "002696".into(),
            date: date.into(),
            margin_buy: 0.0,
            margin_balance: balance,
            short_sell_volume: 0.0,
            short_balance: 0.0,
        }
    }

    #[test]
    fn margin_trend_uses_newest_minus_oldest_within_window() {
        // 乱序输入（vendor 按日期倒序返回）：排序后窗口首尾 = 06-05(10) → 06-30(11)。
        let series = [
            margin_point("2026-06-30", 11.0),
            margin_point("2026-06-05", 10.0),
            margin_point("2026-06-20", 10.5),
        ];
        let pct = margin_balance_trend_pct(&series, 2).expect("样本足");
        assert!((pct - 10.0).abs() < 1e-9, "窗口首尾 10→11 应得 +10%，实得 {pct}");
    }

    #[test]
    fn margin_trend_insufficient_samples_fail_open() {
        let series = [margin_point("2026-06-30", 11.0)];
        assert_eq!(margin_balance_trend_pct(&series, 20), None);
    }

    #[test]
    fn margin_trend_nonpositive_oldest_fail_open() {
        // 窗口首点非正 = 坏数据 ⇒ None（fail-open，不产出伪造趋势）。
        let series = [margin_point("2026-06-30", 11.0), margin_point("2026-06-05", 0.0)];
        assert_eq!(margin_balance_trend_pct(&series, 1), None);
    }

    fn block_trade(
        date: &str,
        discount: Option<f64>,
        amount_yi: f64,
        buyer: &str,
    ) -> axagent_astock_data::BlockTrade {
        axagent_astock_data::BlockTrade {
            stock_code: "002696".into(),
            stock_name: "百洋股份".into(),
            trade_date: date.into(),
            price: 10.0,
            volume: 1.0e6,
            amount: amount_yi * 1e8,
            buyer_dept: Some(buyer.into()),
            seller_dept: None,
            discount_pct: discount,
        }
    }

    #[test]
    fn block_profile_window_median_and_buyers() {
        // 锚点 = 序列内最新日 06-30；60 天窗口 cutoff = 05-01。
        // 04-15 < 05-01 ⇒ 窗外截掉；窗内 3 笔 premiums=[0,-1,-3] 奇数中位 = -1；
        // 买方去重 = 2 家；总额 = 2+1+1 = 4 亿。
        let series = [
            block_trade("2026-04-15", Some(-5.0), 1.0, "甲"),
            block_trade("2026-06-02", Some(-1.0), 2.0, "甲"),
            block_trade("2026-06-15", Some(-3.0), 1.0, "乙"),
            block_trade("2026-06-30", Some(0.0), 1.0, "甲"),
        ];
        let p = block_trade_discount_profile(&series, 60).expect("窗内 ≥3 笔");
        assert_eq!(p.count, 3, "04-15 被窗口截掉");
        assert!((p.median_premium_pct - (-1.0)).abs() < 1e-9, "实得 {p:?}");
        assert_eq!(p.distinct_buyers, 2);
        assert!((p.total_amount_yi - 4.0).abs() < 1e-9, "实得 {p:?}");
    }

    #[test]
    fn block_profile_insufficient_count_fail_open() {
        let series = [block_trade("2026-06-30", Some(-5.0), 1.0, "甲")];
        assert_eq!(block_trade_discount_profile(&series, 60), None);
    }

    #[test]
    fn block_profile_median_odd_count() {
        let series = [
            block_trade("2026-06-01", Some(-1.0), 1.0, "甲"),
            block_trade("2026-06-15", Some(-2.0), 1.0, "甲"),
            block_trade("2026-06-30", Some(-9.0), 1.0, "乙"),
        ];
        let p = block_trade_discount_profile(&series, 60).expect("3 笔");
        assert!((p.median_premium_pct - (-2.0)).abs() < 1e-9, "奇数中位数取中间值，实得 {p:?}");
    }
}
