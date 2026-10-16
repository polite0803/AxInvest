//! 决策证据对账审计 —— 替代 `evidence_citation` 的文本相似度判据。
//!
//! ## 为什么换判据（现网实证，2026-10-16）
//!
//! 旧判据把 `decision_reasoning` 按句拆成 claim，再与各分析师报告正文做字符集 Jaccard。
//! 实测 156 条 `completed` 行里 **129 条**的理由是 `portfolio-mgr` 的**计算轨迹串**
//! （`决策=… 置信=…(branch_row) … | …`），它不含 `。！？\n` ⇒ 恒拆出 **1 条 claim**
//! ⇒ 面板只有 0% / 100% 两个读数。而命中与否实测只由 claim 的**唯一字符数**决定
//! （未命中组 47–87、命中组 84–158；两组「claim 字符被报告覆盖率」同为 0.6–0.78）
//! ⇒ `MATCH_THRESHOLD=0.15` 等价于「claim ≥ 约 85 个唯一字符」，测的是**文案长度**不是证据。
//! 详见 `PLAN-evidence-factor-audit.md` §〇。
//!
//! ## 本模块的判据（全部读已落库的数值，不做任何文本相似度）
//!
//! 阶段 0 普查（同 PLAN §七）证明四项判据**不可能都作用在同一套腿上**，故分两面：
//!
//! - **A 面｜主档因子腿** `decision_json.evidence.factors.fN{sigma,weight}`
//!   → L2 σ 可回算（对同记录内已落库的上游基准）+ L3 参与融合 + 全局闭合。
//! - **B 面｜本档分支腿** `blackboard_snapshot["result.h_<档>"].legs[]{factor,role,signal,weight}`
//!   → L1 上游在场 + L3 参与 + L4 缺席有交代。
//!   分母取 `Period::verdict_spec().participating`（裁定 P-B3）。
//!
//! 归因与来源**一律现场推导自既有权威表**，不抄第二份：
//! [`crate::evidence_weight::DECISION_LEG_ANALYST`]、[`crate::evidence_weight::VERDICT_FACTOR_SOURCES`]、
//! `axagent_harness::holding_period::Period::{factor_owner, verdict_spec}`。

use crate::evidence_weight::{DECISION_LEG_ANALYST, VERDICT_FACTOR_SOURCES};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// σ 落库时的舍入位数（`src-tauri/src/commands/portfolio-mgr.rhai` 的
/// `(x*1000.0).round()/1000.0`）。阶段 4 的门禁逐字比对该字面量，
/// 改舍入而不同步本常量即红（同 `VOL_LOOKBACK_DAYS` 的先例）。
const SIGMA_DECIMALS: u32 = 3;
/// 上游基准与 `avgSignal` 落库时的舍入位数（同文件 `(x*100.0).round()/100.0`）。
const BASIS_DECIMALS: u32 = 2;
/// 权重落库时的舍入位数（`(x*10000.0).round()/10000.0`）。
const WEIGHT_DECIMALS: u32 = 4;

/// 第 `d` 位小数舍入的绝对误差界。
fn rounding_bound(decimals: u32) -> f64 {
    0.5_f64 * 10f64.powi(-(decimals as i32))
}

/// 单条腿的 σ 与其上游基准的容差 = 两侧各自舍入界之和（**推导**，非拍常数）。
fn sigma_basis_tolerance() -> f64 {
    rounding_bound(SIGMA_DECIMALS) + rounding_bound(BASIS_DECIMALS)
}

/// 全局闭合 `Σ(σ·w)/weightSum ≟ avgSignal` 的容差。
///
/// 误差传播：`|Δavg| ≤ δσ + δw·n/weightSum + δavg`（`Σw_i = weightSum`、`|σ_i| ≤ 1`、n 条腿）。
/// 每一项都来自上面三个舍入常量，**不写死数值**。
fn closure_tolerance(legs: usize, weight_sum: f64) -> f64 {
    let weight_term = if weight_sum > 0.0 {
        rounding_bound(WEIGHT_DECIMALS) * legs as f64 / weight_sum
    } else {
        0.0
    };
    rounding_bound(SIGMA_DECIMALS) + weight_term + rounding_bound(BASIS_DECIMALS)
}

/// 主档因子腿 → 其在**同一条记录内**可对的落库基准（JSON 指针）。
///
/// 这张表登记的是「基准住在哪个键」这一**新事实**（此前无任何表登记），不是别处的第二份拷贝。
/// 刻意**不含** f1/f2：它们的 σ 由 `(totalScore−50)/50` 归一化而来，而该口径目前住在
/// `portfolio-mgr.rhai:328/336` ⇒ 在 Rust 侧再写一遍就是第二个权威源。裁定 P-B1 已批上收
/// harness，上收完成后这两条才进本表（PLAN 阶段 1b）。
const FACTOR_SIGMA_BASIS: &[(&str, &str)] = &[
    ("f4", "/risk_quant/f4_adjusted"),
    ("f9", "/evidence/f9_money_flow_signal"),
    ("f10", "/evidence/f10_chip_signal"),
    ("f11", "/evidence/f11_pace_signal"),
    ("f12", "/evidence/f12_momentum_signal"),
];

/// 「查了但没有」与「没法查」必须可分：`NotChecked` 不得渲染成 `Absent`。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "state")]
pub enum UpstreamState {
    /// 上游产出在本记录内可定位且非空
    Present,
    /// 分支脚本显式声明该腿缺席（`absentLegs` / `dataGaps` / `constraint`）
    DeclaredAbsent,
    /// 该腿参与了计算，却既无在场证据也无缺席声明
    SilentAbsent,
    /// 本记录快照不携带该来源的产出键 ⇒ 判据对本腿**不适用**（不是 0 分）
    NotChecked { reason: String },
}

/// σ 与上游基准的回算结果。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "state")]
pub enum Reconcile {
    Consistent {
        /// |σ − 基准|
        delta: f64,
        /// 由两侧舍入界推导
        tolerance: f64,
        basis: String,
    },
    Inconsistent {
        delta: f64,
        tolerance: f64,
        basis: String,
    },
    /// 本记录内没有该腿的落库基准（含「归一化口径尚未上收」）
    NoBasis {
        reason: String,
    },
}

/// A 面：主档一条因子腿。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FactorLegAudit {
    pub leg_id: String,
    pub name: String,
    pub sigma: f64,
    pub weight: f64,
    /// 证据属主分析师 id；`None` = 元约束腿（风险分类 / 数据质量 / trader 观点），
    /// 按 `DECISION_LEG_ANALYST` 的**显式**登记，不是「查不到」。
    pub owner: Option<String>,
    pub participates: bool,
    pub reconcile: Reconcile,
}

/// B 面：本档一条分支腿。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BranchLegAudit {
    pub factor: String,
    pub owner: Option<String>,
    pub owner_name: String,
    pub role: String,
    pub signal: Option<f64>,
    pub weight: f64,
    pub participates: bool,
    /// 本档 `verdict_spec().participating` 是否要求这条腿进方向加权
    pub expected_to_participate: bool,
    pub upstream: UpstreamState,
    /// L4：不参与时是否有交代
    pub absence_declared: bool,
}

/// 主档全局闭合：`Σ(σ·w)/weightSum` 是否等于落库的 `avgSignal`。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "state")]
pub enum Closure {
    Closed {
        recomputed: f64,
        stored: f64,
        tolerance: f64,
    },
    Broken {
        recomputed: f64,
        stored: f64,
        tolerance: f64,
    },
    /// 缺 `weightSum` / `avgSignal` 或分母为 0 ⇒ 无从判定
    NotComputable {
        reason: String,
    },
}

/// 完整审计报告（替代 `CitationReport`）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EvidenceAuditReport {
    pub stock_code: String,
    pub stock_name: String,
    pub analysis_date: String,
    pub decision_action: String,
    pub decision_confidence: f64,
    /// 本轮决策未产出（action 为 `Unavailable` 哨兵）⇒ 没有任何可审计的决策
    pub decision_degraded: bool,
    /// 本记录的主档（`decision_json.timeHorizon`）
    pub tier: String,
    pub branch_legs: Vec<BranchLegAudit>,
    pub factor_legs: Vec<FactorLegAudit>,
    pub closure: Closure,
    // ── 计数分三族，**不得并排成同一个可比数字**（两套计数域各自有消费点）──
    /// 本档 `verdict_spec().participating` 的腿数（裁定 P-B3 的总分母）
    pub expected_branch_legs: usize,
    /// B 面里 `role=direction` 且 `weight>0` 的腿数
    pub participating_branch_legs: usize,
    /// B 面里 L1 判为 `Present` 的腿数
    pub upstream_present_legs: usize,
    /// B 面里 L1 **不适用**（快照不携带该来源键）的腿数 —— 单独计数，不并入缺席
    pub upstream_not_checked_legs: usize,
    /// A 面里 L2 判为 `Consistent` 的腿数
    pub sigma_reconciled_legs: usize,
    /// A 面里 `weight>0` 的腿数
    pub participating_factor_legs: usize,
    pub factor_legs_total: usize,
}

fn as_f(v: Option<&Value>) -> Option<f64> {
    v.and_then(Value::as_f64)
}

fn as_s(v: Option<&Value>) -> Option<&str> {
    v.and_then(Value::as_str)
}

/// 按 JSON 指针取值（只按 `/` 分层；快照的键名含 `.` 但那是键的一部分，不是层级）。
fn pointer<'a>(doc: &'a Value, path: &str) -> Option<&'a Value> {
    path.trim_start_matches('/').split('/').try_fold(doc, |acc, seg| acc.get(seg))
}

/// 把 `report.*` 键集归一为「属主 base → 是否有正文」。
///
/// 带档后缀（`report.a-sentiment--mid`）先经 `analyst_base_of` 剥掉 —— 与显示名走同一条接线，
/// 两侧必须同判据，否则「有显示名但查不到正文」会分裂。
fn report_presence(blackboard: &Value) -> Vec<(String, bool)> {
    let mut out: Vec<(String, bool)> = Vec::new();
    let Some(obj) = blackboard.as_object() else { return out };
    for (key, value) in obj {
        let Some(rest) = key.strip_prefix("report.") else { continue };
        let base = axagent_harness::holding_period::analyst_base_of(rest).unwrap_or(rest);
        let text = match value {
            Value::String(t) => t.clone(),
            other => other.to_string(),
        };
        let trimmed = text.trim();
        let present = !trimmed.is_empty() && trimmed != "null";
        match out.iter_mut().find(|(b, _)| b == base) {
            Some(slot) => slot.1 |= present,
            None => out.push((base.to_string(), present)),
        }
    }
    out
}

/// `dataGaps` 是给人读的句子，按**分支因子名**字面命中来回收「哪条腿已被声明缺席」。
///
/// 因子名清单取 `VERDICT_FACTOR_SOURCES` 的定义域（分支词汇表），不另列一份。
fn gap_declared_factors(branch: &Value) -> Vec<String> {
    let Some(lines) = branch.get("dataGaps").and_then(Value::as_array) else { return Vec::new() };
    lines
        .iter()
        .filter_map(Value::as_str)
        .flat_map(|line| {
            VERDICT_FACTOR_SOURCES
                .iter()
                .filter(move |(factor, _, _)| line.contains(factor))
                .map(|(factor, _, _)| (*factor).to_string())
        })
        .collect()
}

/// 对一条记录做证据对账。
///
/// - `decision_json` / `blackboard`：`stock_analyses` 两列的解析结果（缺列传 `Value::Null`）
/// - `decision_degraded`：命令侧按 `decision_action` 的 `Unavailable` 哨兵判定；为真则**不对账**
#[allow(clippy::too_many_arguments)]
pub fn audit_decision_evidence(
    stock_code: &str,
    stock_name: &str,
    analysis_date: &str,
    decision_action: &str,
    decision_confidence: f64,
    decision_degraded: bool,
    decision_json: &Value,
    blackboard: &Value,
) -> EvidenceAuditReport {
    let tier_raw = as_s(pointer(decision_json, "/timeHorizon")).unwrap_or_default().to_string();
    let period = axagent_harness::holding_period::Period::ALL
        .iter()
        .find(|p| p.as_str() == tier_raw)
        .copied();

    if decision_degraded {
        return EvidenceAuditReport {
            stock_code: stock_code.to_string(),
            stock_name: stock_name.to_string(),
            analysis_date: analysis_date.to_string(),
            decision_action: decision_action.to_string(),
            decision_confidence,
            decision_degraded: true,
            tier: String::new(),
            branch_legs: Vec::new(),
            factor_legs: Vec::new(),
            closure: Closure::NotComputable { reason: "DECISION_DEGRADED".into() },
            expected_branch_legs: 0,
            participating_branch_legs: 0,
            upstream_present_legs: 0,
            upstream_not_checked_legs: 0,
            sigma_reconciled_legs: 0,
            participating_factor_legs: 0,
            factor_legs_total: 0,
        };
    }

    let spec = period.map(|p| p.verdict_spec());
    // `HashSet<String>` 而非 `Vec<&str>`：两处判定都是「某个 &str 在不在这个集合里」，
    // 用 Vec 会撞 `clippy::manual_contains`，而它的机械建议（`contains(&factor)`）在
    // `&'static str` 元素 + 短生命周期 `&str` 参数上**并不成立** ⇒ 换成 HashSet 走
    // `Borrow<str>` 通道，判定语义与原来逐字一致。
    let participating_set: std::collections::HashSet<String> = spec
        .as_ref()
        .map(|sp| sp.participating.iter().map(|s| s.to_string()).collect())
        .unwrap_or_default();

    // ── B 面：本档分支腿 ──
    let primary = format!("/result.h_{tier_raw}");
    // 同一份分支产出在快照里有两把键（节点 id 与 `pm-h-*` 别名），任一命中即可
    let alias = format!("/result.pm-h-{}", tier_raw.replace('_', "-"));
    let branch = pointer(blackboard, &primary)
        .or_else(|| pointer(blackboard, &alias))
        .cloned()
        .unwrap_or(Value::Null);
    let reports = report_presence(blackboard);
    let absent: Vec<String> = branch
        .get("absentLegs")
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect())
        .unwrap_or_default();
    let gap_factors = gap_declared_factors(&branch);
    // 两条登记路径合并成**同一份**缺席清单：`absentLegs`（结构化）∪ `dataGaps`（散文句里
    // 回收到的因子名）。合并的理由是实测出来的：只把 `dataGaps` 用于「给已有腿置标」时，
    // 仅出现在句子里、不在 `absentLegs` 里的腿**压根不会成行** ⇒ 「有交代的缺席」被漏掉。
    let mut declared_absent_factors: Vec<String> = absent.clone();
    for f in &gap_factors {
        if !declared_absent_factors.contains(f) {
            declared_absent_factors.push(f.clone());
        }
    }

    let mut branch_legs: Vec<BranchLegAudit> = Vec::new();
    if let Some(legs) = branch.get("legs").and_then(Value::as_array) {
        for leg in legs {
            let Some(factor) = as_s(leg.get("factor")) else { continue };
            let role = as_s(leg.get("role")).unwrap_or("").to_string();
            let weight = as_f(leg.get("weight")).unwrap_or(0.0);
            let signal = as_f(leg.get("signal"));
            let owner = period
                .and_then(|_| axagent_harness::holding_period::Period::factor_owner(factor))
                .map(str::to_string);
            let declared_absent = declared_absent_factors.iter().any(|a| a == factor);
            let constraint = as_s(leg.get("constraint")).is_some();

            let upstream = if declared_absent {
                UpstreamState::DeclaredAbsent
            } else if let Some((_, kind, channel)) =
                VERDICT_FACTOR_SOURCES.iter().find(|(cf, _, _)| *cf == factor)
            {
                match *kind {
                    "verdict" => match &owner {
                        Some(o) if reports.iter().any(|(b, ok)| *ok && b == o) => {
                            UpstreamState::Present
                        },
                        Some(_) => UpstreamState::SilentAbsent,
                        None => UpstreamState::NotChecked { reason: "NO_OWNER".into() },
                    },
                    // tierScore / sharedTool：只有当 channel 字面量**本身就是**快照里的节点键
                    // 时才判在场。阶段 0 实测：12 条登记里 4 条的 channel 是工具名而非节点 id，
                    // 且逐档评分节点（`Period::scoring_node_id`）的产出根本不落主快照
                    // ⇒ 判不了就标 NotChecked，不手抄一张 channel→节点 别名表当第二份权威
                    // （PLAN 待裁 P-B5）。
                    _ => {
                        let located = !channel.is_empty()
                            && pointer(blackboard, &format!("/result.{channel}")).is_some();
                        if located {
                            UpstreamState::Present
                        } else {
                            UpstreamState::NotChecked { reason: "NO_SNAPSHOT_KEY".into() }
                        }
                    },
                }
            } else {
                UpstreamState::NotChecked { reason: "UNREGISTERED_FACTOR".into() }
            };

            branch_legs.push(BranchLegAudit {
                owner_name: crate::evidence_citation::analyst_display_name(
                    owner.as_deref().unwrap_or(factor),
                ),
                factor: factor.to_string(),
                participates: role == "direction" && weight > 0.0,
                expected_to_participate: participating_set.contains(factor),
                absence_declared: declared_absent || constraint,
                owner,
                role,
                signal,
                weight,
                upstream,
            });
        }
    }

    // 缺席登记里的腿（不在 `legs` 中）必须**显式成行**：否则「缺席」在面板上等于不存在，
    // 与「本来就没这条腿」同形。真实形态见 600406 · 2026-10-15 的 `absentLegs=["microstructure"]`
    // —— 它既不在 `legs` 里，也不该因此被审计漏掉。
    // 整段以「本档分支产出在场」为前提：分支产出缺失（旧格式记录）时不做缺席判定，
    // 那属于「拿不到」而不是「缺席」。
    if !branch.is_null() {
        for f in &declared_absent_factors {
            if branch_legs.iter().any(|l| l.factor == *f) {
                continue;
            }
            let owner =
                axagent_harness::holding_period::Period::factor_owner(f).map(str::to_string);
            branch_legs.push(BranchLegAudit {
                owner_name: crate::evidence_citation::analyst_display_name(
                    owner.as_deref().unwrap_or(f),
                ),
                factor: f.clone(),
                owner,
                role: "absent".to_string(),
                signal: None,
                weight: 0.0,
                participates: false,
                expected_to_participate: participating_set.contains(f.as_str()),
                upstream: UpstreamState::DeclaredAbsent,
                absence_declared: true,
            });
        }

        // 本档**应参与**却既不在 `legs` 也不在缺席登记里的腿 ⇒ 静默缺席（L4 要抓的正是它）。
        for f in &participating_set {
            if branch_legs.iter().any(|l| l.factor == *f) {
                continue;
            }
            let owner =
                axagent_harness::holding_period::Period::factor_owner(f).map(str::to_string);
            branch_legs.push(BranchLegAudit {
                owner_name: crate::evidence_citation::analyst_display_name(
                    owner.as_deref().unwrap_or(f),
                ),
                factor: (*f).to_string(),
                owner,
                role: "absent".to_string(),
                signal: None,
                weight: 0.0,
                participates: false,
                expected_to_participate: true,
                upstream: UpstreamState::SilentAbsent,
                absence_declared: false,
            });
        }
    }

    // ── A 面：主档因子腿 ──
    let factors = pointer(decision_json, "/evidence/factors").cloned().unwrap_or(Value::Null);
    let mut factor_legs: Vec<FactorLegAudit> = Vec::new();
    if let Some(map) = factors.as_object() {
        let mut ids: Vec<&String> = map.keys().collect();
        ids.sort();
        for id in ids {
            let node = &map[id];
            let sigma = as_f(node.get("sigma")).unwrap_or(0.0);
            let weight = as_f(node.get("weight")).unwrap_or(0.0);
            let owner = DECISION_LEG_ANALYST
                .iter()
                .find(|(l, _)| *l == id)
                .and_then(|(_, a)| a.map(str::to_string));
            let basis = FACTOR_SIGMA_BASIS.iter().find(|(l, _)| *l == id).map(|(_, p)| *p);
            let reconcile = match (basis, basis.and_then(|p| as_f(pointer(decision_json, p)))) {
                (Some(p), Some(bv)) => {
                    let tol = sigma_basis_tolerance();
                    let delta = (sigma - bv).abs();
                    let basis = p.to_string();
                    if delta <= tol {
                        Reconcile::Consistent { delta, tolerance: tol, basis }
                    } else {
                        Reconcile::Inconsistent { delta, tolerance: tol, basis }
                    }
                },
                (Some(_), None) => Reconcile::NoBasis { reason: "BASIS_MISSING_IN_RECORD".into() },
                (None, _) => Reconcile::NoBasis { reason: "NO_PERSISTED_BASIS".into() },
            };
            factor_legs.push(FactorLegAudit {
                leg_id: id.clone(),
                name: as_s(node.get("name")).unwrap_or_default().to_string(),
                participates: weight > 0.0,
                sigma,
                weight,
                owner,
                reconcile,
            });
        }
    }

    let closure = match (
        as_f(pointer(decision_json, "/evidence/weightSum")),
        as_f(pointer(decision_json, "/evidence/avgSignal")),
    ) {
        (Some(w), Some(stored)) if w > 0.0 && !factor_legs.is_empty() => {
            let recomputed: f64 = factor_legs.iter().map(|l| l.sigma * l.weight).sum::<f64>() / w;
            let tol = closure_tolerance(factor_legs.len(), w);
            let delta = (recomputed - stored).abs();
            if delta <= tol {
                Closure::Closed { recomputed, stored, tolerance: tol }
            } else {
                Closure::Broken { recomputed, stored, tolerance: tol }
            }
        },
        _ => Closure::NotComputable { reason: "NO_WEIGHT_SUM_OR_FACTORS".into() },
    };

    // 计数必须在两族腿被 move 进报告**之前**算完
    let participating_branch_legs = branch_legs.iter().filter(|l| l.participates).count();
    let upstream_present_legs =
        branch_legs.iter().filter(|l| l.upstream == UpstreamState::Present).count();
    let upstream_not_checked_legs = branch_legs
        .iter()
        .filter(|l| matches!(l.upstream, UpstreamState::NotChecked { .. }))
        .count();
    let sigma_reconciled_legs =
        factor_legs.iter().filter(|l| matches!(l.reconcile, Reconcile::Consistent { .. })).count();
    let participating_factor_legs = factor_legs.iter().filter(|l| l.participates).count();
    let factor_legs_total = factor_legs.len();

    EvidenceAuditReport {
        stock_code: stock_code.to_string(),
        stock_name: stock_name.to_string(),
        analysis_date: analysis_date.to_string(),
        decision_action: decision_action.to_string(),
        decision_confidence,
        decision_degraded: false,
        tier: tier_raw,
        branch_legs,
        factor_legs,
        closure,
        expected_branch_legs: participating_set.len(),
        participating_branch_legs,
        upstream_present_legs,
        upstream_not_checked_legs,
        sigma_reconciled_legs,
        participating_factor_legs,
        factor_legs_total,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// 夹具按 **600406 · 2026-10-15 真记录**的形状裁剪（腿名/角色/字段都取真值），
    /// 数值只保留判据要用的那几条；`avgSignal` 按本夹具的 Σ(σ·w)/weightSum 现算成 2 位。
    fn fixture() -> (Value, Value) {
        let dj = json!({
            "timeHorizon": "ultra_short",
            "risk_quant": { "f4_base": 0.0, "f4_quant_adjust": -0.17, "f4_adjusted": -0.17 },
            "evidence": {
                "weightSum": 1.105,
                "avgSignal": 0.06,
                "factors": {
                    "f1":  { "name": "trend",    "sigma": 0.56,   "weight": 0.105 },
                    "f4":  { "name": "risk",     "sigma": -0.174, "weight": 0.15 },
                    "f11": { "name": "pace",     "sigma": 0.31,   "weight": 0.052 },
                    "f12": { "name": "momentum", "sigma": 0.3,    "weight": 0.075 }
                },
                "f11_pace_signal": 0.31,
                "f12_momentum_signal": 0.3
            }
        });
        let bb = json!({
            "result.h_ultra_short": {
                "legs": [
                    { "factor": "momentumSignal", "role": "direction", "signal": 0.5, "weight": 0.565217, "constraint": null },
                    { "factor": "eventCatalyst",  "role": "direction", "signal": 0.5, "weight": 0.434783, "constraint": null },
                    { "factor": "breadthState",   "role": "filter",    "signal": 0.674, "weight": 0.0, "constraint": "breadthGateOnly" }
                ],
                "absentLegs": ["microstructure"],
                "dataGaps": ["microstructure 腿本轮不可评估：涨停池成员判定尚未接进本分支"]
            },
            "report.a-catalyst--ultra_short": "催化剂等级 3（事件驱动）",
            "result.t-valuation-band": { "content": "{}" }
        });
        (dj, bb)
    }

    fn run(dj: &Value, bb: &Value, degraded: bool) -> EvidenceAuditReport {
        audit_decision_evidence("600406", "国电南瑞", "2026-10-15", "持有", 51.7, degraded, dj, bb)
    }

    /// 降级闸口：本轮没有决策 ⇒ 一条腿都不许对账，闭合也必须报「无从判定」，
    /// 不得返回一个看起来像 0 分的报告。
    #[test]
    fn degraded_decision_is_not_audited() {
        let (dj, bb) = fixture();
        let r = run(&dj, &bb, true);
        assert!(r.decision_degraded);
        assert!(r.branch_legs.is_empty() && r.factor_legs.is_empty());
        assert_eq!(r.closure, Closure::NotComputable { reason: "DECISION_DEGRADED".into() });
        assert_eq!(r.factor_legs_total, 0);

        // 负对照：同一份输入经闸口**确实**会产出腿集（证明上面拦住的正是该形态）
        let unguarded = run(&dj, &bb, false);
        assert!(!unguarded.decision_degraded);
        assert!(!unguarded.factor_legs.is_empty() && !unguarded.branch_legs.is_empty());
    }

    /// L1 两臂：verdict 类腿看属主分析师正文在场；sharedTool 类腿只在 channel
    /// **字面量本身就是快照节点键**时才判在场，否则必须报 `NotChecked` ——
    /// 阶段 0 实测 4/12 的 channel 是工具名（`get_limit_up_pool` 等）而非节点 id，
    /// 判不了就说不适用，**不许**降级成 Absent。
    #[test]
    fn upstream_presence_two_arms_and_the_not_checked_bucket() {
        let (dj, bb) = fixture();
        let r = run(&dj, &bb, false);
        let leg = |f: &str| r.branch_legs.iter().find(|l| l.factor == f).unwrap();

        // verdict 臂：a-catalyst 的正文在场
        assert_eq!(leg("eventCatalyst").upstream, UpstreamState::Present);
        assert_eq!(leg("eventCatalyst").owner.as_deref(), Some("a-catalyst"));

        // 负控同期：把该分析师的正文撤掉 ⇒ 参与了的腿却无来源 ⇒ SilentAbsent
        let mut bb2 = bb.clone();
        bb2.as_object_mut().unwrap().remove("report.a-catalyst--ultra_short");
        let r2 = run(&dj, &bb2, false);
        let l2 = r2.branch_legs.iter().find(|l| l.factor == "eventCatalyst").unwrap();
        assert_eq!(l2.upstream, UpstreamState::SilentAbsent, "撤掉正文后必须落到静默缺席");

        // sharedTool 臂：夹具里 breadthState 的 channel 是工具名
        // ⇒ 必须落 NotChecked 而不是 Absent，且单独计数
        assert!(
            matches!(leg("breadthState").upstream, UpstreamState::NotChecked { .. }),
            "工具名 channel 不得被判成缺席，实际 {:?}",
            leg("breadthState").upstream
        );
        assert!(r.upstream_not_checked_legs >= 1);
        assert_eq!(r.upstream_present_legs, 1, "本夹具只有 eventCatalyst 一条能判到场");
    }

    /// L1 的 sharedTool 臂上电：channel 字面量等于节点键时必须判 Present
    /// （否则「判不了」会吞掉「真的判到了」这一头）。
    #[test]
    fn shared_tool_arm_fires_when_channel_is_a_real_snapshot_key() {
        let (dj, bb) = fixture();
        // 夹具的 h_ultra_short 分支里本没有 valuationBand 腿；补一条，其 channel
        // `t-valuation-band` 在快照里确有其键（真记录同形）。
        let mut bb2 = bb.clone();
        bb2["result.h_ultra_short"]["legs"].as_array_mut().unwrap().push(json!({
            "factor": "valuationBand", "role": "direction", "signal": 0.2, "weight": 0.1, "constraint": null
        }));
        let r = run(&dj, &bb2, false);
        let leg = r.branch_legs.iter().find(|l| l.factor == "valuationBand").unwrap();
        assert_eq!(leg.upstream, UpstreamState::Present, "channel 命中真实节点键时应判在场");
        assert_eq!(leg.owner.as_deref(), Some("value-investor"));

        // 负控：撤掉该节点键 ⇒ 同一腿必须翻面（证明上一条不是恒真）
        let mut bb3 = bb2.clone();
        bb3.as_object_mut().unwrap().remove("result.t-valuation-band");
        let r3 = run(&dj, &bb3, false);
        let l3 = r3.branch_legs.iter().find(|l| l.factor == "valuationBand").unwrap();
        assert!(matches!(l3.upstream, UpstreamState::NotChecked { .. }));
    }

    /// L4：缺席必须有交代，且**缺席登记里的腿要显式成行**（真实形态：600406 · 2026-10-15 的
    /// `absentLegs=["microstructure"]`，而 `legs` 里根本没有它 —— 不成行就等于「没这条腿」）。
    /// 两条回收路径（`absentLegs` / `dataGaps`）各验一次。
    #[test]
    fn declared_absence_is_recovered_from_both_registries() {
        let (dj, bb) = fixture();
        let r = run(&dj, &bb, false);

        // ① 缺席登记里的腿必须成行，状态是「显式缺席」且已交代
        let micro = r
            .branch_legs
            .iter()
            .find(|l| l.factor == "microstructure")
            .expect("缺席登记里的腿必须成行");
        assert_eq!(micro.upstream, UpstreamState::DeclaredAbsent);
        assert!(micro.absence_declared);
        assert!(
            !micro.participates && !micro.expected_to_participate,
            "超短档 microstructure 不进方向加权，缺席是声明而非漏项"
        );

        // ② 第二条回收路径：清掉 `absentLegs`，仅靠 `dataGaps` 的句子仍须判为显式缺席
        let mut bb2 = bb.clone();
        bb2["result.h_ultra_short"]["absentLegs"] = json!([]);
        let r2 = run(&dj, &bb2, false);
        let m2 = r2.branch_legs.iter().find(|l| l.factor == "microstructure").unwrap();
        assert_eq!(m2.upstream, UpstreamState::DeclaredAbsent, "dataGaps 里的声明也必须被回收");

        // ③ 只作过滤、带 constraint 的腿：有交代；但上游判据仍是「无法核查」（channel 是工具名）
        let breadth = r.branch_legs.iter().find(|l| l.factor == "breadthState").unwrap();
        assert!(!breadth.participates, "role=filter 不得算参与");
        assert!(breadth.absence_declared, "带 constraint 的过滤腿必须有交代");
        assert!(
            matches!(breadth.upstream, UpstreamState::NotChecked { .. }),
            "收了口径 ≠ 来源在场"
        );

        // ④ 负控：撤掉 constraint ⇒ 同一腿必须翻成「未交代」（证明③不是恒真）
        let mut bb3 = bb.clone();
        bb3["result.h_ultra_short"]["legs"][2]["constraint"] = Value::Null;
        let r3 = run(&dj, &bb3, false);
        let b3 = r3.branch_legs.iter().find(|l| l.factor == "breadthState").unwrap();
        assert!(!b3.absence_declared, "既无登记又无 constraint ⇒ 必须报未交代");

        // ⑤ 负控：两条路径都撤 ⇒ 该行必须消失（证明①不是「任何腿都成行」）
        let mut bb4 = bb.clone();
        bb4["result.h_ultra_short"]["absentLegs"] = json!([]);
        bb4["result.h_ultra_short"]["dataGaps"] = json!([]);
        let r4 = run(&dj, &bb4, false);
        assert!(
            r4.branch_legs.iter().all(|l| l.factor != "microstructure"),
            "两处登记都撤掉后不应再有该行"
        );
    }

    /// L4 的核心信号：本档**应参与**的腿既不在 `legs` 也不在缺席登记 ⇒ 静默缺席
    /// （这才是「结构缺口不得在 UI 造成歧义」要抓的那一格）。
    #[test]
    fn participating_leg_missing_from_both_registries_is_silent_absence() {
        let (dj, bb) = fixture();
        let mut bb2 = bb.clone();
        let legs = bb2["result.h_ultra_short"]["legs"].as_array_mut().unwrap();
        legs.retain(|l| l["factor"] != "momentumSignal");
        let r = run(&dj, &bb2, false);

        let leg = r
            .branch_legs
            .iter()
            .find(|l| l.factor == "momentumSignal")
            .expect("静默缺席也必须成行");
        assert_eq!(leg.upstream, UpstreamState::SilentAbsent);
        assert!(!leg.absence_declared, "既无登记也无 constraint ⇒ 未交代");
        assert!(leg.expected_to_participate, "超短档它本该参与");
        assert_eq!(r.participating_branch_legs, 1, "抽掉后只剩 eventCatalyst 参与");

        // 对照：把它写进 absentLegs ⇒ 同一位置从「静默缺席」翻成「显式缺席」
        let mut bb3 = bb2.clone();
        bb3["result.h_ultra_short"]["absentLegs"] = json!(["momentumSignal"]);
        let r3 = run(&dj, &bb3, false);
        let l3 = r3.branch_legs.iter().find(|l| l.factor == "momentumSignal").unwrap();
        assert_eq!(l3.upstream, UpstreamState::DeclaredAbsent);
        assert!(l3.absence_declared);
    }

    /// L2：容差必须由**两侧舍入界**推出（σ 3 位 / 基准 2 位 ⇒ 0.0055）。
    /// 夹具的 f4 特意取真记录里最险的一对：σ=−0.174 vs 基准 −0.17（delta 0.004）。
    #[test]
    fn sigma_reconcile_uses_rounding_derived_tolerance() {
        let (dj, bb) = fixture();
        let r = run(&dj, &bb, false);
        let f4 = r.factor_legs.iter().find(|l| l.leg_id == "f4").unwrap();
        match &f4.reconcile {
            Reconcile::Consistent { delta, tolerance, basis } => {
                assert_eq!(basis, "/risk_quant/f4_adjusted");
                assert!((delta - 0.004).abs() < 1e-9, "delta 应现算得 0.004，实际 {delta}");
                // 容差按公式现算，不写死常数
                let expect = 0.5 * 10f64.powi(-3) + 0.5 * 10f64.powi(-2);
                assert!(
                    (tolerance - expect).abs() < 1e-12,
                    "容差应等于 {expect}，实际 {tolerance}"
                );
                assert!(*delta <= *tolerance);
            },
            other => panic!("f4 应判一致，实际 {other:?}"),
        }
        // 夹具里 f4/f11/f12 三条都登记了基准且都在容差内 ⇒ 一致者 3（f1 无基准，不算一致）
        assert_eq!(r.sigma_reconciled_legs, 3);
        let f11 = r.factor_legs.iter().find(|l| l.leg_id == "f11").unwrap();
        assert!(matches!(f11.reconcile, Reconcile::Consistent { .. }));

        // 负控①：把 σ 推到容差之外 ⇒ 必须翻成 Inconsistent（而不是被容差吃掉）
        let mut dj2 = dj.clone();
        dj2["evidence"]["factors"]["f4"]["sigma"] = json!(-0.30);
        let r2 = run(&dj2, &bb, false);
        let f4b = r2.factor_legs.iter().find(|l| l.leg_id == "f4").unwrap();
        assert!(matches!(f4b.reconcile, Reconcile::Inconsistent { .. }), "越界必须报不一致");

        // 负控②：撤掉基准 ⇒ NoBasis（不是「一致」）
        let mut dj3 = dj.clone();
        dj3["risk_quant"] = json!({});
        let r3 = run(&dj3, &bb, false);
        let f4c = r3.factor_legs.iter().find(|l| l.leg_id == "f4").unwrap();
        assert!(matches!(f4c.reconcile, Reconcile::NoBasis { .. }), "缺基准必须报 NoBasis");
    }

    /// A 面 f1 必须报「无落库基准」：它的 σ 由 `(totalScore−50)/50` 归一化而来，
    /// 该口径尚未上收（裁定 P-B1 的阶段 1b）⇒ 在 Rust 侧回算就是第二份权威。
    #[test]
    fn f1_has_no_basis_until_normalization_is_centralized() {
        let (dj, bb) = fixture();
        let r = run(&dj, &bb, false);
        let f1 = r.factor_legs.iter().find(|l| l.leg_id == "f1").unwrap();
        assert_eq!(f1.reconcile, Reconcile::NoBasis { reason: "NO_PERSISTED_BASIS".into() });
    }

    /// 全局闭合：容差按 `δσ + δw·n/W + δavg` 现算；越界必须报 Broken。
    #[test]
    fn closure_tolerance_is_derived_and_breaks_when_out_of_range() {
        let (dj, bb) = fixture();
        let r = run(&dj, &bb, false);
        let (recomputed, stored, tolerance) = match r.closure {
            Closure::Closed { recomputed, stored, tolerance } => (recomputed, stored, tolerance),
            ref other => panic!("夹具应闭合，实际 {other:?}"),
        };
        // 期望按公式现算（n 条腿、W = 1.105）
        let n = r.factor_legs.len();
        let expect = 0.5e-3 + (0.5e-4 * n as f64 / 1.105) + 0.5e-2;
        assert!((tolerance - expect).abs() < 1e-12, "容差应等于 {expect}，实际 {tolerance}");
        assert!((recomputed - stored).abs() <= tolerance);

        // 负控：把 avgSignal 挪出容差 ⇒ Broken
        let mut dj2 = dj.clone();
        dj2["evidence"]["avgSignal"] = json!(0.5);
        let r2 = run(&dj2, &bb, false);
        assert!(matches!(r2.closure, Closure::Broken { .. }), "越界必须报 Broken");

        // 负控：分母为 0 ⇒ 无从判定，不得当成「闭合」
        let mut dj3 = dj.clone();
        dj3["evidence"]["weightSum"] = json!(0.0);
        let r3 = run(&dj3, &bb, false);
        assert!(matches!(r3.closure, Closure::NotComputable { .. }));
    }

    /// 总数与分母取自**权威 spec**，不是写死的条数：说「本档应参与 N 条腿」时
    /// N 必须等于 `verdict_spec().participating.len()`。
    #[test]
    fn expected_leg_count_comes_from_the_authority() {
        let (dj, bb) = fixture();
        let r = run(&dj, &bb, false);
        let want =
            axagent_harness::holding_period::Period::UltraShort.verdict_spec().participating.len();
        assert_eq!(r.expected_branch_legs, want);
        assert_eq!(r.tier, "ultra_short");
        // 夹具里两条 direction 腿权重为正
        assert_eq!(r.participating_branch_legs, 2);
        // A 面：meta 腿（f4 风险分类）必须 owner=None 而不是空串
        let f4 = r.factor_legs.iter().find(|l| l.leg_id == "f4").unwrap();
        assert_eq!(f4.owner, None, "f4 在 DECISION_LEG_ANALYST 里是显式 None（元约束）");
        let f11 = r.factor_legs.iter().find(|l| l.leg_id == "f11").unwrap();
        assert_eq!(f11.owner.as_deref(), Some("a-sentiment"));

        // 未登记的档位串：不许 panic，且分母必须为 0（判据不存在 ≠ 通过）
        let mut dj2 = dj.clone();
        dj2["timeHorizon"] = json!("no_such_tier");
        let r2 = run(&dj2, &bb, false);
        assert_eq!(r2.expected_branch_legs, 0);
        assert!(r2.branch_legs.is_empty());
    }

    /// 门（2026-10-16）：本模块的三个**舍入位数常量**必须与产端字面量逐一对应。
    ///
    /// 理由同 `VOL_LOOKBACK_DAYS` 的先例：两处一致地不同（改了一处）比一处缺失更难发现。
    /// 容差是从这三个常量**推导**出来的（`rounding_bound`），所以字面量一变而常量没动，
    /// 判据就会静默变松/变紧 —— 这条测试把那个静默状态钉住。
    ///
    /// 用 `include_str!` 直接读产端脚本：跨包读源码是刻意选择（本 crate 的测试要能独立
    /// 跑这条断言，不必等主 crate 那 20 分钟编译）。**代价**：改 `portfolio-mgr.rhai`
    /// 会让本 crate 重编 —— 这正是「资产被 `include_str!` 就属于构建图」的已知行为。
    #[test]
    fn rounding_digits_match_the_rhai_literals() {
        const PM: &str = include_str!("../../../../src-tauri/src/commands/portfolio-mgr.rhai");

        // σ：`(fN_signal*1000.0).round()/1000.0`
        assert!(
            PM.contains("(f4_adjusted*1000.0).round()/1000.0"),
            "σ 的舍入字面量变了 ⇒ SIGMA_DECIMALS={SIGMA_DECIMALS} 需同步"
        );
        // 权重：`(fN_weight*10000.0).round()/10000.0`
        assert!(
            PM.contains("(f11_weight*10000.0).round()/10000.0"),
            "权重的舍入字面量变了 ⇒ WEIGHT_DECIMALS={WEIGHT_DECIMALS} 需同步"
        );
        // 基准（f4_adjusted）与 avgSignal：`(x*100.0).round()/100.0`
        assert!(
            PM.contains("\"f4_adjusted\": (f4_adjusted*100.0).round()/100.0"),
            "f4 基准的舍入字面量变了 ⇒ BASIS_DECIMALS={BASIS_DECIMALS} 需同步"
        );
        assert!(
            PM.contains("\"avgSignal\": (avg_signal*100.0).round()/100.0"),
            "avgSignal 的舍入字面量变了 ⇒ BASIS_DECIMALS={BASIS_DECIMALS} 需同步"
        );

        // 常量与上面四条字面量的**位数**必须一致（把「/1000 ⇒ 3 位」这条语义写死）
        assert_eq!(SIGMA_DECIMALS, 3);
        assert_eq!(WEIGHT_DECIMALS, 4);
        assert_eq!(BASIS_DECIMALS, 2);

        // 容差由常量推导：0.5e-3 + 0.5e-2 = 0.0055
        let want =
            0.5 * 10f64.powi(-(SIGMA_DECIMALS as i32)) + 0.5 * 10f64.powi(-(BASIS_DECIMALS as i32));
        assert!((sigma_basis_tolerance() - want).abs() < 1e-12);
    }
}
