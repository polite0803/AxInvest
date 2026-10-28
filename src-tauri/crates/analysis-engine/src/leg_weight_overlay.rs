// SPDX-License-Identifier: AGPL-3.0-only

//! P2-a 的权重裁决层：把「腿 × 档」预测力读数应用到逐档分支表。
//!
//! ## 分工与边界
//!
//! 判据本体（四态状态机、Fisher-z 区间、方向胜率分母）与第三会话曾经落地的
//! `leg_predictiveness` 逐字同口径（`ok` / `insufficient_sample` / `ic_undefined` /
//! `ci_spans_zero`、门槛 30）。**该模块与其 bin 参数已在 2026-10-16 被那一批整体回滚**
//! （PLAN 第九节有现场读数），而 PLAN 原文要求「IC 不显著的权重置零」必须有人做 ——
//! 故本模块自带取数与统计以形成闭环；`leg_predictiveness` 一旦重新落地，本文件的
//! 「装配 + 统计」两段应删除、改由它映射 cells，覆盖层判据不动。
//! 两边通过 `settings` 里同一份 camelCase JSON 通信，故这里自带一个反序列化形状：
//! `LegCell::status` 是 `&'static str`（借用语义），不能直接从 JSON 反序列化，必须由本模块
//! 用 owned 字段承接。这不是第二套口径 —— 状态值域与字段名都以读数为权威。
//!
//! ## 判据（为什么只认 `ci_spans_zero`）
//!
//! 读数有四种状态：`ok` / `insufficient_sample` / `ic_undefined` / `ci_spans_zero`。
//! 只有最后一种是「样本够了、区间跨 0」即**测过了确实没有预测力**；前三种分别是
//! 「有效」「没测够」「取数退化」。把后两种也算成没预测力，等于让取数故障去惩罚一条腿
//! （本仓对 fail-open 付过代价）。[`only_zeroing_status`] 用一条测试把字面量钉住：
//! 上游改值域时这里会红，而不是静默退化成「永不置零」。
//!
//! ## 为什么置零后必须重新归一
//!
//! 分支表的权重是配比（合计 1）。直接抹掉一条腿会让该档配比总和 < 1，等效于**全档被动降权**
//! —— 那是「一条腿没用」被放大成「整个档位证据不足」，与判据本意不符。若全档方向腿都被判
//! 无预测力，则**不重归一、不退回等权**：该档按既有口径不出结论。

use axagent_harness::Period;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

/// 一格最少观测数：Fisher-z 的正态近似在 n≥30 才够用，低于此区间宽度本身不可信。
pub const LEG_MIN_SAMPLES: usize = 30;

/// 判「该腿在该档确无预测力」的状态字面量（值域见本模块 `leg_cell_status`）。
const ONLY_ZEROING_STATUS: &str = "ci_spans_zero";

/// 裁决结果的载体键：与读数层约定同一份报告。
pub const LEG_IC_SETTING_KEY: &str = "leg_ic_report";

/// 一格读数的 owned 形态（字段名与读数层的 camelCase 输出一字对齐）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LegIcCellInput {
    pub tier: String,
    pub factor: String,
    pub samples: usize,
    pub status: String,
    #[serde(default)]
    pub rank_ic: Option<f64>,
    #[serde(default)]
    pub ic_ci_low: Option<f64>,
    #[serde(default)]
    pub ic_ci_high: Option<f64>,
    /// 方向胜率（腿与结果都给出方向的观测里同号占比）；无从判方向时 NULL
    #[serde(default)]
    pub direction_hit_rate: Option<f64>,
}

/// 一份可落 `settings` 的裁决输入（读数报告 + 生成时刻 + 当时的样本门槛）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LegIcReportInput {
    /// 生成时刻（毫秒）—— 呈现层必须显示它，否则「未判定」会被读成刚测过的结论
    pub generated_at_ms: i64,
    pub min_samples: usize,
    pub observations: usize,
    pub cells: Vec<LegIcCellInput>,
}

impl LegIcReportInput {
    pub fn to_json_text(&self) -> String {
        serde_json::to_string(self).unwrap_or_else(|_| "{}".to_string())
    }

    pub fn from_json_text(text: &str) -> Option<Self> {
        serde_json::from_str(text).ok()
    }
}

/// 该状态是否作废这条腿的方向权重。
pub fn only_zeroing_status(status: &str) -> bool {
    status == ONLY_ZEROING_STATUS
}

/// 一条观测：某次分析在某档位上，某条腿的信号值与该档位实际持有的净收益（百分点）。
#[derive(Debug, Clone, PartialEq)]
pub struct LegSample {
    pub tier: String,
    pub factor: String,
    pub signal: f64,
    pub return_pct: f64,
}

/// Fisher-z 区间的最少点数（`n-3` 开方在此之前无意义）。
const FISHER_Z_MIN_SAMPLES: usize = 4;
/// 95% 双侧临界值。
const Z95: f64 = 1.959_963_984_0;

/// Spearman rank IC 的 Fisher-z 双侧 95% 区间。
///
/// `|rho| = 1`（严格单调）时 `atanh` 发散 ⇒ 返回 `None` 而不是 ±1 的假区间 ——
/// 与 `ic_undefined` 同臂处置（那是「秩退化」，不是「区间不跨 0」）。
pub fn fisher_z_ci(rho: f64, n: usize) -> Option<(f64, f64)> {
    if n < FISHER_Z_MIN_SAMPLES || !rho.is_finite() || rho.abs() >= 1.0 {
        return None;
    }
    let z = rho.atanh();
    let se = 1.0 / ((n as f64) - 3.0).sqrt();
    Some(((z - Z95 * se).tanh(), (z + Z95 * se).tanh()))
}

/// 一格状态（有序判据：先样本量、再秩可算性、最后显著性 —— 分格不足时谈区间没有意义）。
pub fn leg_cell_status(samples: usize, rank_ic: Option<f64>, min_samples: usize) -> &'static str {
    if samples < min_samples {
        return "insufficient_sample";
    }
    let Some(rho) = rank_ic else { return "ic_undefined" };
    match fisher_z_ci(rho, samples) {
        None => "ic_undefined",
        Some((low, high)) => {
            if low > 0.0 || high < 0.0 {
                "ok"
            } else {
                "ci_spans_zero"
            }
        },
    }
}

/// 按 (档 × 腿) 分格算读数。纯函数，便于直接打表锁行为。
pub fn compute_leg_cells(rows: &[LegSample], min_samples: usize) -> Vec<LegIcCellInput> {
    let mut grouped: std::collections::BTreeMap<(&str, &str), Vec<&LegSample>> =
        std::collections::BTreeMap::new();
    for r in rows {
        grouped.entry((r.tier.as_str(), r.factor.as_str())).or_default().push(r);
    }
    grouped
        .iter()
        .map(|((tier, factor), g)| {
            let pairs: Vec<(f64, f64)> = g
                .iter()
                .map(|o| (o.signal, o.return_pct))
                .filter(|(x, y)| x.is_finite() && y.is_finite())
                .collect();
            let rank_ic = axagent_harness::indicators::spearman_rank_ic(&pairs);
            let status = leg_cell_status(pairs.len(), rank_ic, min_samples);
            let ci = rank_ic.and_then(|r| fisher_z_ci(r, pairs.len()));
            // 方向胜率分母 = 腿与结果**都给出方向**的观测（signal=0 或收益=0 两侧都不进，
            // 否则「持平」会被记成看空命中，分母与分子不同集合）。
            let signed: Vec<&(f64, f64)> =
                pairs.iter().filter(|(x, y)| *x != 0.0 && *y != 0.0).collect();
            let hit_rate = if signed.is_empty() {
                None
            } else {
                let hits = signed.iter().filter(|(x, y)| (*x > 0.0) == (*y > 0.0)).count();
                Some(((hits as f64) / (signed.len() as f64) * 1e4).round() / 1e4)
            };
            LegIcCellInput {
                tier: tier.to_string(),
                factor: factor.to_string(),
                samples: pairs.len(),
                status: status.to_string(),
                rank_ic: rank_ic.map(|r| (r * 1e4).round() / 1e4),
                ic_ci_low: ci.map(|c| (c.0 * 1e4).round() / 1e4),
                ic_ci_high: ci.map(|c| (c.1 * 1e4).round() / 1e4),
                direction_hit_rate: hit_rate,
            }
        })
        .collect()
}

/// 从落库两列配对逐腿观测。
///
/// 键族两侧都试：`decisionsByHorizon` 的生产键是 camelCase（`ultraShort`，全库实测 100/100，
/// 见 PLAN 偏差 #20），而 `horizon_results_json` 顶层键是 snake_case（实测）——
/// 只按一族取会静默丢掉整档。该档收益取不到 ⇒ 整条跳过：没有后验收益的信号不是样本。
pub fn leg_samples_from_db_pair(decision_json: &str, horizon_results_json: &str) -> Vec<LegSample> {
    let Ok(dec) = serde_json::from_str::<Value>(decision_json) else { return Vec::new() };
    let Ok(res) = serde_json::from_str::<Value>(horizon_results_json) else { return Vec::new() };
    let Some(by) = dec.get("decisionsByHorizon") else { return Vec::new() };
    let mut out = Vec::new();
    for p in Period::ALL {
        let Some(h) = by.get(p.as_str()).or_else(|| by.get(p.camel_key())) else { continue };
        let tier_res = res.get(p.as_str()).or_else(|| res.get(p.camel_key()));
        let Some(return_pct) = tier_res
            .and_then(|t| t.get("market"))
            .and_then(|m| m.get("returnPct"))
            .and_then(Value::as_f64)
        else {
            continue;
        };
        let Some(legs) = h.get("legs").and_then(Value::as_array) else { continue };
        for leg in legs {
            let (Some(factor), Some(signal)) = (
                leg.get("factor").and_then(Value::as_str),
                leg.get("signal").and_then(Value::as_f64),
            ) else {
                continue;
            };
            out.push(LegSample {
                tier: p.as_str().to_string(),
                factor: factor.to_string(),
                signal,
                return_pct,
            });
        }
    }
    out
}

/// 由「(分析 decision_json, 反思 horizon_results_json)」配对行装配并算出报告。
pub fn build_report(pairs: &[(String, String)], generated_at_ms: i64) -> LegIcReportInput {
    let mut rows = Vec::new();
    for (decision, results) in pairs {
        rows.extend(leg_samples_from_db_pair(decision, results));
    }
    let observations = rows.len();
    let cells = compute_leg_cells(&rows, LEG_MIN_SAMPLES);
    LegIcReportInput { generated_at_ms, min_samples: LEG_MIN_SAMPLES, observations, cells }
}

/// 把读数应用到分支表（[`crate::evidence_weight::horizon_branch_specs`] 的产物）：
/// 判「无预测力」的方向腿权重置零 + 该档其余方向腿重新归一到合计 1 + 每腿标注读数。
///
/// 标注是呈现层的依据：`icStatus` 缺失会被读成「已验证」，所以**每条腿**都要落一个状态 ——
/// 报告里没有对应格时写 `untested`。返回被置零的方向腿条数。
///
/// 档位键两族都认（`mid` 与 camel 形态同义）：分支表用 snake，而 `decisionsByHorizon` 的
/// 生产键是 camelCase（PLAN 偏差 #20 实测），只比一族会静默永不命中。
pub fn apply_leg_weight_overlay(branches: &mut Value, report: &LegIcReportInput) -> usize {
    let mut zeroed_total = 0usize;
    let Some(obj) = branches.as_object_mut() else { return 0 };
    for (tier, legs) in obj.iter_mut() {
        let Some(legs) = legs.get_mut("legs").and_then(Value::as_array_mut) else { continue };
        let mut tier_zeroed = 0usize;
        for leg in legs.iter_mut() {
            let Some(factor) = leg.get("factor").and_then(Value::as_str).map(str::to_string) else {
                continue;
            };
            let cell =
                report.cells.iter().find(|c| c.factor == factor && tier_key_matches(tier, &c.tier));
            let Some(map) = leg.as_object_mut() else { continue };
            match cell {
                None => {
                    map.insert("icStatus".into(), json!("untested"));
                },
                Some(c) => {
                    map.insert("icStatus".into(), json!(c.status.as_str()));
                    map.insert("icSamples".into(), json!(c.samples));
                    map.insert("icRank".into(), json!(c.rank_ic));
                    let is_direction = map.get("role").and_then(Value::as_str) == Some("direction");
                    let has_weight = map.get("weight").and_then(Value::as_f64).unwrap_or(0.0) > 0.0;
                    if is_direction && has_weight && only_zeroing_status(&c.status) {
                        map.insert("weight".into(), json!(0.0));
                        map.insert("icZeroed".into(), json!(true));
                        tier_zeroed += 1;
                    }
                },
            }
        }
        if tier_zeroed > 0 {
            renormalise_direction_legs(legs, tier);
            zeroed_total += tier_zeroed;
        }
    }
    zeroed_total
}

/// snake 档位名与 camel 档位名是否同指一档（四档归一后互不相同，不会误命中）。
fn tier_key_matches(branch_tier: &str, cell_tier: &str) -> bool {
    branch_tier == cell_tier || norm_tier(branch_tier) == norm_tier(cell_tier)
}

fn norm_tier(s: &str) -> String {
    s.chars().filter(|c| *c != '_').flat_map(char::to_lowercase).collect()
}

/// 置零后把该档方向腿重新归一到合计 1（保留 4 位小数，与分支表生成口径一致）。
fn renormalise_direction_legs(legs: &mut [Value], tier: &str) {
    let sum: f64 =
        legs.iter().filter(|l| l["role"] == "direction").filter_map(|l| l["weight"].as_f64()).sum();
    if sum <= 0.0 {
        tracing::warn!(
            "[leg_weight_overlay] 档 {tier} 方向腿全部被判无预测力 ⇒ 该档不出结论（不重归一、不退等权）"
        );
        return;
    }
    for l in legs.iter_mut() {
        if l["role"] != "direction" {
            continue;
        }
        if let Some(map) = l.as_object_mut() {
            let w = map.get("weight").and_then(Value::as_f64).unwrap_or(0.0) / sum;
            map.insert("weight".into(), json!((w * 1e6).round() / 1e6));
        }
    }
}

#[cfg(test)]
mod leg_weight_overlay_tests {
    use super::*;

    fn cell(tier: &str, factor: &str, status: &str) -> LegIcCellInput {
        LegIcCellInput {
            tier: tier.to_string(),
            factor: factor.to_string(),
            samples: 40,
            status: status.to_string(),
            rank_ic: Some(0.05),
            ic_ci_low: Some(-0.2),
            ic_ci_high: Some(0.3),
            direction_hit_rate: Some(0.5),
        }
    }

    fn report(cells: Vec<LegIcCellInput>) -> LegIcReportInput {
        LegIcReportInput {
            generated_at_ms: 1_760_000_000_000,
            min_samples: 30,
            observations: 40,
            cells,
        }
    }

    fn branches() -> Value {
        json!({
            "mid": {
                "tier": "mid",
                "legs": [
                    {"factor": "trendStrength", "role": "direction", "weight": 0.6},
                    {"factor": "flowPersistence", "role": "direction", "weight": 0.4},
                    {"factor": "valuationBand", "role": "filter", "weight": 0.0}
                ]
            }
        })
    }

    /// 四种状态里只有 `ci_spans_zero` 会作废权重 —— 这条同时钉住了字面量，上游改值域即红。
    #[test]
    fn only_ci_spans_zero_zeroes_weight() {
        for s in ["ok", "insufficient_sample", "ic_undefined"] {
            assert!(
                !only_zeroing_status(s),
                "{s} 不得触发置零（没测够/取数退化不能当成没有预测力）"
            );
        }
        assert!(only_zeroing_status("ci_spans_zero"));
        let mut b = branches();
        assert_eq!(
            apply_leg_weight_overlay(
                &mut b,
                &report(vec![cell("mid", "trendStrength", "insufficient_sample")])
            ),
            0
        );
        assert_eq!(b["mid"]["legs"][0]["weight"], json!(0.6), "样本不足时分支表须逐字不变");
    }

    #[test]
    fn zeroed_leg_is_followed_by_renormalisation_of_the_rest() {
        let mut b = branches();
        let zeroed = apply_leg_weight_overlay(
            &mut b,
            &report(vec![cell("mid", "trendStrength", "ci_spans_zero")]),
        );
        assert_eq!(zeroed, 1);
        let legs = b["mid"]["legs"].as_array().expect("legs").clone();
        assert_eq!(legs[0]["weight"], json!(0.0));
        assert_eq!(legs[0]["icZeroed"], json!(true));
        assert_eq!(legs[1]["weight"], json!(1.0), "剩余方向腿必须归一回 1，否则等于全档被动降权");
        assert_eq!(legs[2]["weight"], json!(0.0), "非方向腿不参与归一");
    }

    #[test]
    fn camel_cell_tier_still_hits_snake_branch_tier() {
        // 分支表键是 snake（`mid`/`ultra_short`），而生产决策键是 camelCase（`ultraShort`）
        let mut b = json!({"ultra_short": {"legs": [
            {"factor": "trendStrength", "role": "direction", "weight": 1.0}
        ]}});
        let zeroed = apply_leg_weight_overlay(
            &mut b,
            &report(vec![cell("ultraShort", "trendStrength", "ci_spans_zero")]),
        );
        assert_eq!(zeroed, 1, "档位键族不同也必须命中，否则裁决静默失效");
        assert_eq!(b["ultra_short"]["legs"][0]["weight"], json!(0.0));
    }

    #[test]
    fn every_leg_gets_a_status_even_without_a_cell() {
        let mut b = branches();
        assert_eq!(apply_leg_weight_overlay(&mut b, &report(vec![])), 0);
        assert_eq!(b["mid"]["legs"][0]["icStatus"], json!("untested"));
        assert_eq!(b["mid"]["legs"][1]["icStatus"], json!("untested"));
    }

    #[test]
    fn all_legs_zeroed_does_not_renormalise_into_equal_weight() {
        let mut b = branches();
        let zeroed = apply_leg_weight_overlay(
            &mut b,
            &report(vec![
                cell("mid", "trendStrength", "ci_spans_zero"),
                cell("mid", "flowPersistence", "ci_spans_zero"),
            ]),
        );
        assert_eq!(zeroed, 2);
        let legs = b["mid"]["legs"].as_array().expect("legs").clone();
        assert_eq!(legs[0]["weight"], json!(0.0));
        assert_eq!(
            legs[1]["weight"],
            json!(0.0),
            "全档无预测力时不得退等权，也不得把 0/0 写成 NaN"
        );
    }

    #[test]
    fn cell_status_orders_sample_before_significance() {
        assert_eq!(
            leg_cell_status(10, Some(0.9), LEG_MIN_SAMPLES),
            "insufficient_sample",
            "样本不足先于区间判定"
        );
        assert_eq!(
            leg_cell_status(40, None, LEG_MIN_SAMPLES),
            "ic_undefined",
            "秩无定义不是不显著"
        );
        assert_eq!(
            leg_cell_status(40, Some(1.0), LEG_MIN_SAMPLES),
            "ic_undefined",
            "严格单调时 atanh 发散、区间不可给 ⇒ 属秩退化臂，不得算成 ok"
        );
        assert_eq!(leg_cell_status(40, Some(0.4), LEG_MIN_SAMPLES), "ok", "区间不跨 0");
        assert_eq!(
            leg_cell_status(40, Some(0.1), LEG_MIN_SAMPLES),
            "ci_spans_zero",
            "同分母下小 IC 必须判不显著"
        );
        assert!(only_zeroing_status("ci_spans_zero"));
    }

    #[test]
    fn samples_pair_both_key_families_and_refuse_missing_return() {
        let dec = r#"{"decisionsByHorizon":{"ultraShort":{"legs":[{"factor":"trendStrength","role":"direction","signal":0.4}]}}}"#;
        let res = r#"{"ultra_short":{"market":{"returnPct":-3.1}}}"#;
        let rows = leg_samples_from_db_pair(dec, res);
        assert_eq!(rows.len(), 1, "决策侧 camel × 结果侧 snake 必须能配对: {rows:?}");
        assert_eq!(rows[0].tier, "ultra_short");
        assert_eq!(rows[0].return_pct, -3.1);
        assert!(
            leg_samples_from_db_pair(dec, r#"{"schemaVersion":3}"#).is_empty(),
            "没有后验收益的信号不是样本，不得造伪观测"
        );
    }

    #[test]
    fn direction_hit_rate_denominator_excludes_flat_observations() {
        let rows: Vec<LegSample> = vec![
            LegSample { tier: "mid".into(), factor: "f".into(), signal: 0.5, return_pct: 1.0 },
            LegSample { tier: "mid".into(), factor: "f".into(), signal: -0.5, return_pct: -1.0 },
            LegSample { tier: "mid".into(), factor: "f".into(), signal: 0.0, return_pct: 5.0 },
            LegSample { tier: "mid".into(), factor: "f".into(), signal: 0.2, return_pct: 0.0 },
        ];
        let cells = compute_leg_cells(&rows, 2);
        assert_eq!(cells[0].samples, 4, "IC 分母含全部配对（含中性观测）");
        assert_eq!(cells[0].direction_hit_rate, Some(1.0), "胜率分母只含双方都给方向的 2 条");
    }

    #[test]
    fn build_report_on_today_shape_observations_produces_no_zeroing() {
        // 现网形态：每格 n < 30 ⇒ 全部 insufficient_sample ⇒ 裁决层不得动任何权重
        let dec = r#"{"decisionsByHorizon":{"mid":{"legs":[{"factor":"trendStrength","role":"direction","signal":0.4},{"factor":"earningsQuality","role":"direction","signal":-0.2}]}}}"#;
        let res = r#"{"mid":{"market":{"returnPct":2.0}}}"#;
        let report = build_report(&[(dec.to_string(), res.to_string())], 1_760_000_000_000);
        assert_eq!(report.observations, 2);
        assert_eq!(report.min_samples, LEG_MIN_SAMPLES);
        assert!(
            report.cells.iter().all(|c| c.status == "insufficient_sample"),
            "{:?}",
            report.cells
        );
        let mut branches = branches();
        assert_eq!(apply_leg_weight_overlay(&mut branches, &report), 0);
        assert_eq!(
            branches["mid"]["legs"][0]["weight"],
            json!(0.6),
            "样本不足时分支表必须逐字不变"
        );
    }

    #[test]
    fn report_roundtrips_with_camel_case_ipc_keys() {
        let r = report(vec![cell("mid", "trendStrength", "ok")]);
        let text = r.to_json_text();
        for key in ["generatedAtMs", "minSamples", "rankIc", "icCiLow"] {
            assert!(text.contains(key), "落库/IPC 键必须是 camelCase，缺 {key}: {text}");
        }
        assert_eq!(LegIcReportInput::from_json_text(&text), Some(r));
    }
}
