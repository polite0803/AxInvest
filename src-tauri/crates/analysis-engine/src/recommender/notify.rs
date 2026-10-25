//! 荐股定时任务的"扫 + 通知"核心逻辑
//!
//! 跟 [`super::recommend_stocks`] 的区别：
//! - **过滤兜底 pick** (`synthetic == true`)：用户明确要求推送"真实"推荐
//! - **过滤低 confidence**：通过 `min_confidence` 阈值
//! - **取 top N**：按 confidence 降序
//! - **生成 OS 通知文本**：`build_notification`
//!
//! 不依赖 Tauri，方便在 Tauri command 之外（如 CLI / 测试）复用。

use std::sync::Arc;

use axagent_astock_data::AStockClient;

use super::types::{Period, RecoPick, Style, TrimmedPick};
use super::{recommend_stocks_multi, RecoResponse};

/// 一次定时荐股扫描的完整结果。
///
/// 除 `picks` 外还带 **本次扫描实际使用的候选池快照** —— 调用方（cron executor）
/// 需要把它一并落库到 `reco_picks.seed_pool_json`，否则回测的负向样本
/// （候选池 − 正向样本）会缺失，且「候选池→逐只分析」的候选来源也无从追溯。
pub struct RecommendationScan {
    /// 过滤 synthetic、过阈值、按 confidence 降序、截断到 top N 后的真实推荐
    pub picks: Vec<RecoPick>,
    /// 首个成功 period 的 seed pool 快照（`[[code,name],...]` JSON）
    pub seed_pool_snapshot: Option<String>,
    /// 各档的 L3 截断留痕（该档算出但被组内 top-N 截掉的候选）。
    ///
    /// 调用方要一并落 `reco_scan_audit`。此前本结构不透出留痕 ⇒ 定时链与时间旅行批次
    /// 走这条路时该表**恒空**，只有经命令层 `recommend_stocks` 的批次才有留痕，
    /// 于是漏检归因整批落到 `Unexplained`（语义边界见 `axagent_entities::reco_scan_audit`）。
    pub period_audit: Vec<(Period, Vec<TrimmedPick>)>,
    /// **按档**的过滤后真实 picks（`synthetic` 已剔除、过阈值已应用、**未跨档去重**、**未 top-N 截断**）。
    ///
    /// ## 为什么需要它（R-3d，2026-10-26）
    ///
    /// `picks` 是「四档合并后按 stock_code 去重、再 top-N」的结果 ⇒
    /// **同一只票在多档命中时只保留 confidence 最高的那一档**，
    /// 于是**档位语义被压平成一档**（实测：回放批次 230 条 picks 只覆盖 58 只票，
    /// 且 `period` **全部是 `short`** —— 其余三档的票全被去重吃掉）。
    ///
    /// 对**通知**场景这是对的（推给用户的是「这几只票」，不该同一只票推四遍）。
    /// 但对**回放落库**场景是错的：回放的目的是「当时各档分别判了什么」，
    /// 档位丢了 ⇒ 反思按档分档、命中率按档统计、`decision_validations` 的
    /// `period ↔ t_plus_n` 匹配**全部失真**。
    ///
    /// ⇒ 落库方（时间旅行批次）必须用本字段，**不能**用 `picks`。
    pub picks_by_period: Vec<(Period, Vec<RecoPick>)>,
}

/// 跑一次"定时荐股扫描"，返回符合要求的 picks（按 confidence 降序）
///
/// 流程：
/// 1. **一次拿多档**（`recommend_stocks_multi`）——候选池只建一次；此前这里是
///    对每个 period `tokio::spawn(recommend_stocks(p))`，四档即 4× 建池 + 4× 流动性过滤
/// 2. 合并所有 RecoResponse.picks（去重：同 stock_code 留 confidence 最高）
/// 3. 过滤 `synthetic == true`（兜底合成）—— **关键** 用户要求不推兜底
/// 4. 过滤 `confidence < min_confidence`
/// 5. 按 confidence 降序
/// 6. 截取前 `top_n` 条
pub async fn run_recommendation_scan(
    client: Arc<AStockClient>,
    periods: &[Period],
    template_vars: &[(String, serde_json::Value)],
    min_confidence: u8,
    top_n: usize,
    horizon_prior: Option<serde_json::Value>,
) -> RecommendationScan {
    if periods.is_empty() || top_n == 0 {
        return RecommendationScan {
            picks: Vec::new(),
            seed_pool_snapshot: None,
            period_audit: Vec::new(),
            picks_by_period: Vec::new(),
        };
    }

    // 1. 一次拿所有 period（共享同一个候选池）
    let results = recommend_stocks_multi(
        client,
        periods,
        template_vars,
        None,
        horizon_prior,
        super::SeedOverrides::default(),
    )
    .await;

    let mut all_picks: Vec<RecoPick> = Vec::new();
    let mut seed_pool_snapshot: Option<String> = None;
    let mut period_audit: Vec<(Period, Vec<TrimmedPick>)> = Vec::new();
    // R-3d（2026-10-26）：按档留存**未跨档去重**的真实 picks（见结构体注释）。
    let mut picks_by_period: Vec<(Period, Vec<RecoPick>)> = Vec::new();
    for (period, res) in results {
        match res {
            Ok(resp) => {
                if seed_pool_snapshot.is_none() {
                    seed_pool_snapshot = resp.seed_pool_snapshot.clone();
                }
                if !resp.scan_audit.is_empty() {
                    period_audit.push((period, resp.scan_audit.clone()));
                }
                // 按档留存：只剔除 synthetic 与过阈值，**不去重、不截断**。
                // 去重/截断是**通知口径**（跨档同一只票只推一次、总共推 top_n 条），
                // 落库口径要的是「各档各自判了什么」。
                let per_period = real_picks_of_period(&resp.picks, min_confidence);
                if !per_period.is_empty() {
                    picks_by_period.push((period, per_period));
                }
                collect_real_picks(&resp, &mut all_picks);
            },
            Err(e) => {
                tracing::warn!("[recommendation_cron] period {:?} 扫描失败: {e}", period);
            },
        }
    }

    // 2. 同 stock_code 去重：留 confidence 最高
    //    （多策略命中同一只时，dedup_and_merge 已在 recommender 内部处理过；
    //     这里只处理"短/中/长"三个 period 之间的重复——比如短线 + 中线都看多同一只）
    let mut by_code: std::collections::HashMap<String, RecoPick> = std::collections::HashMap::new();
    for p in all_picks {
        match by_code.get(&p.stock_code) {
            Some(existing) if existing.confidence >= p.confidence => {
                // 保留已有
            },
            _ => {
                by_code.insert(p.stock_code.clone(), p);
            },
        }
    }

    // 3. 过滤 synthetic + min_confidence
    let mut filtered: Vec<RecoPick> =
        by_code.into_values().filter(|p| !p.synthetic && p.confidence >= min_confidence).collect();

    // 4. 按 confidence 降序
    filtered.sort_by_key(|p| std::cmp::Reverse(p.confidence));

    // 5. top N
    filtered.truncate(top_n);
    RecommendationScan { picks: filtered, seed_pool_snapshot, period_audit, picks_by_period }
}

fn collect_real_picks(resp: &RecoResponse, out: &mut Vec<RecoPick>) {
    for picks in resp.picks.values() {
        for p in picks {
            if !p.synthetic {
                out.push(p.clone());
            }
        }
    }
}

/// 取某一档的**真实** picks（剔除兜底合成与过阈值），**不去重、不截断**。
///
/// R-3d（2026-10-26）：这是「按档留存」的**唯一**实现点（纯函数，可单测）。
///
/// 与通知口径的差别（**不要合并这两个**）：
/// - 通知口径（[`run_recommendation_scan`] 的 `picks`）：跨档去重 + top-N 截断
///   ⇒ 同一只票只推一次、总共推 N 条；
/// - 落库口径（本函数）：**保留各档全部命中** ⇒ 落库的 `period` 列如实反映
///   「当时各档分别判了什么」。
///
/// 实测反例（修复前）：回放批次落库 230 行只覆盖 58 只票、`period` **全是 `short`**
/// ⇒ 反思按档分档 / 命中率按档统计 / `decision_validations` 的 `period ↔ t_plus_n`
/// 匹配**全部失真**。
fn real_picks_of_period(
    by_style: &std::collections::BTreeMap<Style, Vec<RecoPick>>,
    min_confidence: u8,
) -> Vec<RecoPick> {
    let mut out: Vec<RecoPick> = Vec::new();
    for picks in by_style.values() {
        for p in picks {
            if !p.synthetic {
                out.push(p.clone());
            }
        }
    }
    out.retain(|p| p.confidence >= min_confidence);
    out
}

/// 生成 (title, body) 用于 OS 通知
///
/// 格式：
/// - 标题：`📈 智能荐股更新 · {N} 只新推荐`
/// - 正文：每只一行 `{name}({code}) ¥{price} 目标+{upside}%` 最多 8 行（OS 通知有长度限制）
pub fn build_notification(picks: &[RecoPick]) -> (String, String) {
    if picks.is_empty() {
        return ("📈 智能荐股更新".to_string(), "本次未发现符合条件的新推荐".to_string());
    }

    let title = format!("📈 智能荐股更新 · {} 只", picks.len());
    // OS 通知有长度限制；最多 8 行：前 7 只详情 + 1 行 "等共 N"
    let max_detail_lines = 7;
    let mut body = String::new();
    for (i, p) in picks.iter().take(max_detail_lines).enumerate() {
        let upside_pct = if p.price > 0.0 {
            (p.target_price / p.price - 1.0) * 100.0
        } else {
            0.0
        };
        let line = format!(
            "{}. {} ({}) ¥{:.2} → 目标 +{:.1}%\n",
            i + 1,
            p.stock_name,
            p.stock_code,
            p.price,
            upside_pct
        );
        body.push_str(&line);
    }
    if picks.len() > max_detail_lines {
        body.push_str(&format!("…等共 {} 只", picks.len()));
    } else {
        body = body.trim_end().to_string();
    }

    (title, body)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::recommender::types::Style;

    fn pick(
        code: &str,
        name: &str,
        conf: u8,
        price: f64,
        target: f64,
        synthetic: bool,
    ) -> RecoPick {
        RecoPick {
            stock_code: code.into(),
            stock_name: name.into(),
            sector: None,
            style: Style::Trend,
            period: Period::Short,
            price,
            entry_low: price * 0.98,
            entry_high: price * 1.02,
            stop_loss: price * 0.95,
            target_price: target,
            position_pct: 5.0,
            holding_days: 5,
            confidence_percentile: None,
            prior_source: None,
            prior_samples: None,
            evidence_score: None,
            stop_source: None,
            position_source: None,
            confidence: conf,
            reasons: vec![],
            risk_notes: vec![],
            secondary_styles: vec![],
            synthetic,
        }
    }

    #[test]
    fn build_notification_empty() {
        let (title, body) = build_notification(&[]);
        assert!(title.contains("智能荐股更新"));
        assert!(body.contains("未发现"));
    }

    #[test]
    fn build_notification_truncates_to_8_lines() {
        let picks: Vec<RecoPick> =
            (0..15).map(|i| pick(&format!("6{i:05}"), "X", 80, 10.0, 11.0, false)).collect();
        let (title, body) = build_notification(&picks);
        assert!(title.contains("15"));
        // 最多 8 行：前 7 只 + 1 行 "等共 N"
        let line_count = body.lines().count();
        assert!(line_count <= 8, "got {} lines: {}", line_count, body);
        assert!(body.contains("等共 15"), "应包含 footer: {body}");
    }

    #[test]
    fn build_notification_uses_upside_pct() {
        let p = pick("688056", "莱伯泰科", 85, 48.0, 53.0, false);
        let (_, body) = build_notification(&[p]);
        // 53/48 - 1 = 0.1042 → +10.4%
        assert!(body.contains("+10.4%"), "body: {body}");
    }

    #[test]
    fn build_notification_handles_zero_price() {
        let mut p = pick("688056", "X", 80, 0.0, 10.0, false);
        p.price = 0.0;
        p.target_price = 10.0;
        let (_, body) = build_notification(&[p]);
        // upside 应该是 0% 而非 NaN/Inf
        assert!(body.contains("+0.0%"), "body: {body}");
    }

    // ── R-3d（2026-10-26）：按档留存不被跨档去重压平 ──────────────────────────

    /// 构造「同一只票在两档都被判中」的按风格分组形态。
    fn two_period_buckets() -> std::collections::BTreeMap<Style, Vec<RecoPick>> {
        let mut by_style: std::collections::BTreeMap<Style, Vec<RecoPick>> =
            std::collections::BTreeMap::new();
        // 同一只票 000001：short 档置信 60、mid 档置信 70
        let mut short_pick = pick("000001", "平安银行", 60, 10.0, 11.0, false);
        short_pick.period = Period::Short;
        let mut mid_pick = pick("000001", "平安银行", 70, 10.0, 12.0, false);
        mid_pick.period = Period::Mid;
        by_style.entry(Style::Trend).or_default().push(short_pick);
        by_style.entry(Style::Value).or_default().push(mid_pick);
        by_style
    }

    /// **核心回归**：按档留存必须**保留该档的全部命中**（不去重、不截断）。
    ///
    /// 反例形态（本次修复前的真实行为）：跨档去重后只剩 confidence 最高的一档
    /// ⇒ 落库 `period` 恒为单档（实测回放批次 230 行全 `short`）
    /// ⇒ 反思按档分档 / 命中率按档统计 / `period ↔ t_plus_n` 匹配全部失真。
    #[test]
    fn real_picks_of_period_keeps_every_hit_in_that_period() {
        let buckets = two_period_buckets();
        let out = real_picks_of_period(&buckets, 0);
        assert_eq!(out.len(), 2, "按档留存必须两条都在（同一只票的两档）");
        let periods: Vec<Period> = out.iter().map(|p| p.period).collect();
        assert!(
            periods.contains(&Period::Short) && periods.contains(&Period::Mid),
            "两档都必须保留，实际只有 {:?}",
            periods
        );
    }

    /// **负控（区分力）**：跨档去重口径下，同一只票**只会剩一条**。
    ///
    /// 少了这条，上面那条断言可能只是「去重逻辑恰好没生效」而非「按档逻辑生效」。
    #[test]
    fn cross_period_dedup_would_collapse_the_same_stock() {
        let buckets = two_period_buckets();
        let mut all: Vec<RecoPick> = Vec::new();
        for picks in buckets.values() {
            all.extend(picks.iter().cloned());
        }
        // 复刻 `run_recommendation_scan` 步骤 2：按 stock_code 去重留 confidence 最高
        let mut by_code: std::collections::HashMap<String, RecoPick> =
            std::collections::HashMap::new();
        for p in all {
            match by_code.get(&p.stock_code) {
                Some(existing) if existing.confidence >= p.confidence => {},
                _ => {
                    by_code.insert(p.stock_code.clone(), p);
                },
            }
        }
        assert_eq!(by_code.len(), 1, "去重后应只剩 1 只（这正是必须另存按档结果的原因）");
        // 且留下的那条是 mid 档（confidence 70 > 60）⇒ 档位被压成 mid，short 档丢失
        let kept = by_code.values().next().unwrap();
        assert_eq!(kept.period, Period::Mid, "去重后保留 confidence 更高的那一档");
    }

    /// **负控**：`synthetic`（兜底合成）与过阈值的票**不得**进按档结果。
    ///
    /// 少了这条，本函数可能把兜底合成票带回落库 ⇒ 库里又是合成票满天飞
    /// （那正是 2026-10-17~10-22 的形态）。
    #[test]
    fn real_picks_of_period_drops_synthetic_and_low_confidence() {
        let mut by_style: std::collections::BTreeMap<Style, Vec<RecoPick>> =
            std::collections::BTreeMap::new();
        by_style
            .entry(Style::Trend)
            .or_default()
            .push(pick("000002", "合成", 90, 10.0, 11.0, true));
        by_style.entry(Style::Trend).or_default().push(pick(
            "000003",
            "低置信",
            20,
            10.0,
            11.0,
            false,
        ));
        by_style
            .entry(Style::Value)
            .or_default()
            .push(pick("000004", "正常", 70, 10.0, 11.0, false));
        let out = real_picks_of_period(&by_style, 55);
        assert_eq!(out.len(), 1, "只应留下 000004（真实且过阈值）");
        assert_eq!(out[0].stock_code, "000004");
    }
}
