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
//! ## 「一致」与「闭合」的效力边界（2026-10-18）
//!
//! 这两个判据**只能证明构造自洽**，不能证明信号算对，必须按效力分档呈现：
//!
//! - σ 一致分两类（见 [`BasisSource`]）：基准独立于 σ 才算核对过；基准与 σ 是同一
//!   rhai 变量的两条落库副本时，delta 上限恒等于容差推导值 ⇒ 恒真。计数
//!   [`EvidenceAuditReport::sigma_reconciled_legs`] 只认前者，且只认参与腿。
//! - 闭合（[`Closure::Closed`]）只验 `Σ(σ·w)/weightSum ≟ avgSignal` 这条算术，
//!   两侧同源于一次脚本执行 ⇒ 附带 `unverified_weight_ratio` 交代「多少权重其实没核过」。
//!
//! 归因与来源**一律现场推导自既有权威表**，不抄第二份：
//! [`crate::evidence_weight::DECISION_LEG_ANALYST`]、[`crate::evidence_weight::VERDICT_FACTOR_SOURCES`]、
//! `axagent_harness::holding_period::Period::{factor_owner, verdict_spec}`。

use crate::evidence_weight::{DECISION_LEG_ANALYST, VERDICT_FACTOR_SOURCES, channel_snapshot_key};
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

/// 基准住在哪份文档里。
#[derive(Debug, Clone, Copy, PartialEq)]
enum BasisDoc {
    DecisionJson,
    /// `blackboard_snapshot`（主图节点的产出）
    Snapshot,
}

/// 拿到基准原值后要做的变换 —— 与产端**同一个函数**，不在这里复述公式。
#[derive(Debug, Clone, Copy, PartialEq)]
enum BasisTransform {
    /// 基准本身就是 σ
    Identity,
    /// 基准是 0–100 分，需经 `portfolio_formula::score_to_signal` 压到 `[-1, 1]`
    ScoreToSignal,
}

/// 基准的**来源独立性** —— 决定「一致」这条结论能证明什么。
///
/// 没有这一列时，两条性质完全不同的比对会被渲染成同一个绿标签：
///
/// - 独立上游（`f1`）：快照里的 `totalScore` 是**产出侧真值**，σ 由它经
///   `score_to_signal` 派生 ⇒ 改中枢而审计不跟着改、落库与快照不一致，都会红。
/// - 同源副本（`f4` / `f9` / `f10` / `f11` / `f12`）：基准键与 `factors.fN.sigma`
///   是**同一个 rhai 变量的两条落库副本**（`(x*1000).round()/1000` vs
///   `(x*100).round()/100`）⇒ delta 上限恒等于两侧舍入界之和、恰好等于
///   [`sigma_basis_tolerance`] 的推导值 ⇒ **数学上永不越界**。
///
/// 后者只能证明「副本没被篡改」，不能证明「信号算对了」。2026-10-18 截图上
/// `6/12` 全绿正是这个构造的产物，故必须分成两类展示与计数。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum BasisSource {
    /// 基准是独立上游产出，经产端同一函数变换
    IndependentUpstream,
    /// 基准与 σ 是同一变量的第二条落库副本（舍入位数不同）
    SameVariableCopy,
}

/// 主档因子腿 → 其在**同一条记录内**可对的落库基准。
///
/// 这张表登记的是「基准住在哪份文档的哪个键、要不要归一化、**来源是否独立**」
/// 这组事实（此前无任何表登记），不是别处的第二份拷贝；归一化本身走
/// [`crate::portfolio_formula::score_to_signal`]（裁定 P-B1 上收后的唯一权威）。
///
/// **f2 刻意不入表**：现网实测它的输入 `consensusScore` 取自收敛节点
/// （`debate-convergence` 一族），而该节点正随「逐档实例化」在飞 —— 同一条记录上
/// 老形态的键是裸 `debate-convergence.content.consensus_score`、新形态按 seed 指向
/// `debate-convergence--mid.*`（v161），且**都不在 `result.*` 命名空间里**。
/// 给一条会随模板漂移的静态路径＝把「对不上」伪装成「对过了」，所以如实报无基准。
///
/// 第 5 列 [`BasisSource`] 的取值判据是**变量同一性**，不是路径形态：
/// 基准键取到的值若与 `factors.fN.sigma` 出自同一个 rhai 变量，就是 `SameVariableCopy`。
const FACTOR_SIGMA_BASIS: &[(&str, BasisDoc, &str, BasisTransform, BasisSource)] = &[
    (
        "f1",
        BasisDoc::Snapshot,
        "/result.t-scoring/content/totalScore",
        BasisTransform::ScoreToSignal,
        BasisSource::IndependentUpstream,
    ),
    // 以下 5 条全部取自 `portfolio-mgr.rhai` 的同一批变量：
    //   f4  ← `f4_adjusted`（`:3667` 的 σ 与 `:3689` 的基准）
    //   f9  ← `f9_signal`（`:3671` vs `:3644`）
    //   f10 ← `f10_signal`（`:3672` vs `:3645`）
    //   f11 ← `f11_signal`（`:3673` vs `:3646`）
    //   f12 ← `f12_signal`（`:3674` vs `:3647`）
    (
        "f4",
        BasisDoc::DecisionJson,
        "/risk_quant/f4_adjusted",
        BasisTransform::Identity,
        BasisSource::SameVariableCopy,
    ),
    (
        "f9",
        BasisDoc::DecisionJson,
        "/evidence/f9_money_flow_signal",
        BasisTransform::Identity,
        BasisSource::SameVariableCopy,
    ),
    (
        "f10",
        BasisDoc::DecisionJson,
        "/evidence/f10_chip_signal",
        BasisTransform::Identity,
        BasisSource::SameVariableCopy,
    ),
    (
        "f11",
        BasisDoc::DecisionJson,
        "/evidence/f11_pace_signal",
        BasisTransform::Identity,
        BasisSource::SameVariableCopy,
    ),
    (
        "f12",
        BasisDoc::DecisionJson,
        "/evidence/f12_momentum_signal",
        BasisTransform::Identity,
        BasisSource::SameVariableCopy,
    ),
];

/// `f1` → `1`、`f13` → `13`；非 `f<n>` 形态按字面量排在最后。
///
/// 面板原先直接 `ids.sort()`（字符串序）⇒ `f10` 落在 `f2` 前面，读者按 f1→f13
/// 找腿时顺序全乱。键里带一个「形态位」是为了让未知 id 的次序仍然**确定**。
fn leg_sort_key(id: &str) -> (u8, u32, String) {
    match id.strip_prefix('f').and_then(|n| n.parse::<u32>().ok()) {
        Some(n) => (0, n, String::new()),
        None => (1, 0, id.to_string()),
    }
}

/// 按 `/` 分层取值，**途中遇到 JSON 字符串就剥壳继续**：工具节点在快照里是
/// `{"content": "<JSON 字符串>"}` 这种双层形态，不剥壳取不到里层字段。
fn resolve_path(doc: &Value, path: &str) -> Option<Value> {
    let mut cur = doc.clone();
    for seg in path.trim_start_matches('/').split('/') {
        let next = match &cur {
            Value::String(s) => serde_json::from_str::<Value>(s).ok()?.get(seg).cloned(),
            other => other.get(seg).cloned(),
        }?;
        cur = next;
    }
    Some(cur)
}

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
        /// 基准的来源独立性 —— `sameVariableCopy` 时「一致」是构造恒真，
        /// UI 不得染成与 `independentUpstream` 同一种绿。
        source: BasisSource,
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
    /// 该腿的落库 `sigma` 字段**缺失或类型不对**。
    ///
    /// 与 [`Reconcile::NoBasis`] 严格区分：`NoBasis` 缺的是**基准**（对账参照物没建），
    /// 本态缺的是**被检对象本身**。把两者混在一起会让面板把「这条腿根本没数」
    /// 说成「这条腿没有可对账的基准」——归因正好反了。
    InputMissing {
        field: String,
    },
}

/// A 面：主档一条因子腿。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FactorLegAudit {
    pub leg_id: String,
    pub name: String,
    /// 落库 σ。**`None` = 本记录该腿的 `sigma` 键缺失或类型不对，不是 0**。
    ///
    /// 2026-10-18 前这里 `unwrap_or(0.0)`：缺失被兜成 0 之后，它照样进 `Σ(σ·w)`
    /// 和 `participates` 判定，于是「字段坏了」在面板上与「信号真的是零」同形，
    /// 而闭合还能照常通过 —— 等于用缺失值把判据喂绿。
    pub sigma: Option<f64>,
    /// 落库权重。`None` 语义同上；`participates` 因此不能只看 `> 0.0`。
    pub weight: Option<f64>,
    /// 证据属主分析师 id；`None` = 元约束腿（风险分类 / 数据质量 / trader 观点），
    /// 按 `DECISION_LEG_ANALYST` 的**显式**登记，不是「查不到」。
    pub owner: Option<String>,
    /// 是否进方向加权 = `weight` **存在**且 > 0。字段缺失不算参与（也不能算 0 权重参与）。
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
    /// 落库权重。`None` = 字段缺失/类型不对，与「权重真的是 0」可分。
    pub weight: Option<f64>,
    pub participates: bool,
    /// 本档 `verdict_spec().participating` 是否要求这条腿进方向加权
    pub expected_to_participate: bool,
    pub upstream: UpstreamState,
    /// L4：不参与时是否有交代
    pub absence_declared: bool,
    /// 裁决层的腿预测力状态（`untested` / `insufficient_sample` / `ok` / `ci_spans_zero`）。
    /// `None` = 本记录的分支产出未携带该键（旧脚本 / 旧代）。
    pub ic_status: Option<String>,
    /// 该腿是否因「测出无预测力」被裁决层置零。
    /// 与「没接线 / 没数」严格区分：它仍在上游在场、有信号，只是权重被主动裁掉。
    pub ic_zeroed: bool,
}

/// 主档全局闭合：`Σ(σ·w)/weightSum` 是否等于落库的 `avgSignal`。
///
/// ⚠ **闭合只证明算术自洽**：`weightSum` 与 `avgSignal` 与 `factors.fN` 出自同一次
/// rhai 执行（`portfolio-mgr.rhai:1895` 的除法 + `:3683` 的落库），审计拿同一批 σ·w
/// 复算 ⇒ 除非舍入越界，它恒成立。因此 `Closed` 额外携带**权重覆盖率**，
/// 让「37% 的权重经独立核对、63% 从未被任何外部数据核过」这种状态不再显示成纯绿。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "state")]
pub enum Closure {
    Closed {
        recomputed: f64,
        stored: f64,
        tolerance: f64,
        /// 参与腿里「σ 未经独立上游核对」的权重占比 ∈ [0, 1]。
        /// `0.0` = 全部权重都有独立基准；`1.0` = 一条都没有。
        ///
        /// ⚠ 枚举上的 `rename_all` 只改 variant 名，**不改 variant 内字段**；漏掉这行
        /// 时 IPC 出的是 `unverified_weight_ratio`，前端读 `unverifiedWeightRatio` 得
        /// `undefined` ⇒ 覆盖率被算成 `NaN%`（2026-10-21 截图实证）。
        #[serde(rename = "unverifiedWeightRatio")]
        unverified_weight_ratio: f64,
    },
    Broken {
        recomputed: f64,
        stored: f64,
        tolerance: f64,
    },
    /// 无从判定。三个原因码：
    /// - `NO_WEIGHT_SUM_OR_FACTORS`：缺 `weightSum` / `avgSignal`，或分母为 0
    /// - `MISSING_FACTOR_INPUT`：某条腿的 σ / 权重字段缺失 —— **不得**补 0 硬算
    /// - `DECISION_DEGRADED`：本轮决策未产出，没有可审计对象
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
    /// B 面里 `role=direction` 且 `weight` **有值且 > 0** 的腿数（字段缺失不算参与）
    pub participating_branch_legs: usize,
    /// B 面里 L1 判为 `Present` 的腿数
    pub upstream_present_legs: usize,
    /// B 面里 L1 **不适用**（快照不携带该来源键）的腿数 —— 单独计数，不并入缺席
    pub upstream_not_checked_legs: usize,
    /// A 面里「**参与**且经**独立上游**回算一致」的腿数（判据见 [`BasisSource`]）。
    ///
    /// 三项各自排除一类假绿：
    /// - 不参与（`weight = 0`）的腿即使「一致」也不构成证据 —— f10 的 σ=0 ⇒ w=0
    ///   是产端构造耦合（`portfolio-mgr.rhai:1533`），把它计入分子会让可核率虚高；
    /// - 同源副本互比恒真（`SameVariableCopy`），只能证明副本没被篡改；
    /// - 越界（`Inconsistent`）当然不算。
    pub sigma_reconciled_legs: usize,
    /// A 面里 `weight` **有值且 > 0** 的腿数（字段缺失不算参与）
    pub participating_factor_legs: usize,
    pub factor_legs_total: usize,
}

fn as_f(v: Option<&Value>) -> Option<f64> {
    v.and_then(Value::as_f64)
}

fn as_s(v: Option<&Value>) -> Option<&str> {
    v.and_then(Value::as_str)
}

/// 无属主因子（`injectedVar` 通道）的显示名。
///
/// `analyst_display_name` 的键域是分析师 id；把因子塞进去等于宣称「这条证据由某个
/// 分析师判断」，与 `injectedVar` 的语义相反。因此单独给因子名，不冒充分析师。
fn factor_display_name(factor: &str) -> Option<&'static str> {
    match factor {
        "relativeStrength" => Some("相对强度"),
        _ => None,
    }
}

/// 面板「属主」列的唯一取数口：有属主用分析师名，无属主用因子名，都没有才回退键名。
fn display_name_for(owner: Option<&str>, factor: &str) -> String {
    owner
        .map(crate::evidence_citation::analyst_display_name)
        .or_else(|| factor_display_name(factor).map(str::to_string))
        .unwrap_or_else(|| factor.to_string())
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

/// `dataGaps` 里**真的在声明缺席**的句子所用词表。
///
/// 不能只看「句子包含因子名」：分支脚本也会把**设计说明**写进 `dataGaps`
/// （mid 档「supplyShock 只作风险提示、不进方向」就是一条无条件的说明句）。
/// 只按因子名回收会把一条在场、有信号的风险提示腿标成「显式缺席」
/// （现网 688498 · 2026-10-21 实证）。
const ABSENCE_MARKERS: &[&str] = &[
    "缺",
    "无数据",
    "无数",
    "拿不到",
    "不可得",
    "不可评估",
    "未产出",
    "没有产出",
    "按缺席处理",
    "不适用",
    "尚未接",
];

/// `dataGaps` 是给人读的句子，按**分支因子名 + 缺席语义**回收「哪条腿已被声明缺席」。
///
/// 因子名清单取 `VERDICT_FACTOR_SOURCES` 的定义域（分支词汇表），不另列一份。
/// 回收结果只用于给**不在 `legs` 里**的腿合成缺席行；已在 `legs` 里的腿以
/// `absentLegs` 为准（见 `audit_decision_evidence` 里的注释）。
fn gap_declared_factors(branch: &Value) -> Vec<String> {
    let Some(lines) = branch.get("dataGaps").and_then(Value::as_array) else { return Vec::new() };
    lines
        .iter()
        .filter_map(Value::as_str)
        .filter(|line| ABSENCE_MARKERS.iter().any(|marker| line.contains(marker)))
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
    // 两条登记路径合并成同一份清单，但**用途分层**：`gap_factors` 只给不在 `legs`
    // 里的腿合成缺席行；已在 `legs` 里的腿以结构化的 `absentLegs` 为准 —— 脚本自己
    // 就把「有信号」的腿放进 `legs`，此时 `dataGaps` 里的设计说明不得反过来把它判缺席
    // （现网 supplyShock 的「只作风险提示」句就是这种假阳性来源）。
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
            // 缺失即 `None`，绝不兜 0：「权重字段坏了」与「权重真的是 0」在
            // `participates`、覆盖率、闭合求和三处的后果完全不同。
            let weight = as_f(leg.get("weight"));
            let signal = as_f(leg.get("signal"));
            let owner = period
                .and_then(|_| axagent_harness::holding_period::Period::factor_owner(factor))
                .map(str::to_string);
            // 已有腿的缺席判定**只认结构化登记**：它在 `legs` 里就说明脚本拿到了信号，
            // 此时 `dataGaps` 的散文只能作解释，不能把上游状态改判成缺席。
            let declared_absent = absent.iter().any(|a| a == factor);
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
                    // sharedTool：channel 是**工具名**、落库键是跑该工具的**节点 id**，两者
                    // 字面不同（阶段 0 实测 7 条里只有 3 条同名）⇒ 走 `CHANNEL_SNAPSHOT_KEYS`
                    // 这张权威表（裁定 P-B5）。
                    // ⚠ 表里有映射而本记录缺该键 ⇒ 那个节点本轮**没产出**＝静默缺席，
                    // 不是「判不了」：判据已经能判，只是上游真的没出数。
                    "sharedTool" => match channel_snapshot_key(channel) {
                        Some(key) => {
                            if pointer(blackboard, &format!("/{key}")).is_some() {
                                UpstreamState::Present
                            } else {
                                UpstreamState::SilentAbsent
                            }
                        },
                        None => UpstreamState::NotChecked { reason: "NO_CHANNEL_MAPPING".into() },
                    },
                    // injectedVar（阶段 1 的 `relativeStrength`）：值由 wiring 层注入，
                    // **实测确实进了快照**（002696 那次黑板里 `market_regime` 出现 8 次、
                    // `input_params` 25 次），但本仓**没有**「注入变量 → 哪个节点键的 input_params」
                    // 这张权威表 ⇒ 硬造一个 pointer 会把「我猜它在该节点」伪装成「查过且在场」。
                    // 如实报不适用；要升级为可判，先补一张与 `CHANNEL_SNAPSHOT_KEYS` 同形的
                    // 注入变量键表（并给它自己的负控），别在这里凑数。
                    "injectedVar" => {
                        UpstreamState::NotChecked { reason: "INJECTED_VAR_HAS_NO_KEY_TABLE".into() }
                    },
                    // tierScore（`macd` / `totalScore`）：逐档评分节点的产出**不落主快照**
                    // （阶段 0 实测 0/49）⇒ 判据对本腿不适用，如实报出，不给假键。
                    _ => UpstreamState::NotChecked { reason: "TIER_SCORE_NOT_IN_SNAPSHOT".into() },
                }
            } else {
                UpstreamState::NotChecked { reason: "UNREGISTERED_FACTOR".into() }
            };

            branch_legs.push(BranchLegAudit {
                owner_name: display_name_for(owner.as_deref(), factor),
                factor: factor.to_string(),
                participates: role == "direction" && weight.is_some_and(|w| w > 0.0),
                expected_to_participate: participating_set.contains(factor),
                absence_declared: declared_absent || constraint,
                owner,
                role,
                signal,
                weight,
                upstream,
                ic_status: as_s(leg.get("icStatus")).map(str::to_string),
                ic_zeroed: leg.get("icZeroed").and_then(Value::as_bool).unwrap_or(false),
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
                owner_name: display_name_for(owner.as_deref(), f),
                factor: f.clone(),
                owner,
                role: "absent".to_string(),
                signal: None,
                // 审计**合成**的缺席行：明确记为 0 权重（不是字段缺失），
                // 与 `as_f` 读不到键的 `None` 语义分开。
                weight: Some(0.0),
                participates: false,
                expected_to_participate: participating_set.contains(f.as_str()),
                upstream: UpstreamState::DeclaredAbsent,
                absence_declared: true,
                ic_status: None,
                ic_zeroed: false,
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
                owner_name: display_name_for(owner.as_deref(), f),
                factor: (*f).to_string(),
                owner,
                role: "absent".to_string(),
                signal: None,
                // 审计**合成**的缺席行：明确记为 0 权重（不是字段缺失），
                // 与 `as_f` 读不到键的 `None` 语义分开。
                weight: Some(0.0),
                participates: false,
                expected_to_participate: true,
                upstream: UpstreamState::SilentAbsent,
                absence_declared: false,
                ic_status: None,
                ic_zeroed: false,
            });
        }
    }

    // ── A 面：主档因子腿 ──
    let factors = pointer(decision_json, "/evidence/factors").cloned().unwrap_or(Value::Null);
    let mut factor_legs: Vec<FactorLegAudit> = Vec::new();
    if let Some(map) = factors.as_object() {
        let mut ids: Vec<&String> = map.keys().collect();
        // 数值序，不是字符串序：后者会把 f10/f11/f12/f13 排到 f2 前面
        ids.sort_by_key(|id| leg_sort_key(id.as_str()));
        for id in ids {
            let node = &map[id];
            // 两个字段都保留 `Option`：缺失不进任何求和，也不被当作 0 参与。
            let sigma = as_f(node.get("sigma"));
            let weight = as_f(node.get("weight"));
            let owner = DECISION_LEG_ANALYST
                .iter()
                .find(|(l, _)| *l == id)
                .and_then(|(_, a)| a.map(str::to_string));
            let spec = FACTOR_SIGMA_BASIS.iter().find(|(l, ..)| *l == id);
            let frozen = spec.and_then(|(_, doc, path, tf, _)| {
                let raw = match doc {
                    BasisDoc::DecisionJson => pointer(decision_json, path).cloned(),
                    BasisDoc::Snapshot => resolve_path(blackboard, path),
                }?;
                let value = raw.as_f64()?;
                Some(match tf {
                    BasisTransform::Identity => value,
                    // 与产端**同一个函数**（裁定 P-B1）：这里若自己再写一遍 `(x-50)/50`
                    // 就是第二个权威源 —— 脚本改了中枢而审计不跟着改也照样「一致」。
                    BasisTransform::ScoreToSignal => {
                        crate::portfolio_formula::score_to_signal(value)
                    },
                })
            });
            let reconcile = match (sigma, spec, frozen) {
                // σ 缺失优先于一切：没有被检对象时，「有没有基准」无从谈起，
                // 更不能拿 0 去和基准比出一个「一致」。
                (None, _, _) => Reconcile::InputMissing { field: "sigma".into() },
                (Some(sg), Some((_, _, path, _, source)), Some(bv)) => {
                    let tol = sigma_basis_tolerance();
                    let delta = (sg - bv).abs();
                    let basis = (*path).to_string();
                    let source = *source;
                    if delta <= tol {
                        Reconcile::Consistent { delta, tolerance: tol, basis, source }
                    } else {
                        Reconcile::Inconsistent { delta, tolerance: tol, basis }
                    }
                },
                (Some(_), Some(_), None) => {
                    Reconcile::NoBasis { reason: "BASIS_MISSING_IN_RECORD".into() }
                },
                (Some(_), None, _) => Reconcile::NoBasis { reason: "NO_PERSISTED_BASIS".into() },
            };
            factor_legs.push(FactorLegAudit {
                leg_id: id.clone(),
                name: as_s(node.get("name")).unwrap_or_default().to_string(),
                participates: weight.is_some_and(|w| w > 0.0),
                sigma,
                weight,
                owner,
                reconcile,
            });
        }
    }

    // 「已核对」的唯一定义：参与 + 一致 + 基准独立于 σ。计数与覆盖率共用，避免两处判据分叉。
    let is_verified = |l: &FactorLegAudit| {
        l.participates
            && matches!(
                l.reconcile,
                Reconcile::Consistent { source: BasisSource::IndependentUpstream, .. }
            )
    };
    // 只累加**确实取到值**的腿：`weight = null` 的腿连参与都不算，不进分母
    let participating_weight: f64 =
        factor_legs.iter().filter(|l| l.participates).filter_map(|l| l.weight).sum();
    let verified_weight: f64 =
        factor_legs.iter().filter(|l| is_verified(l)).filter_map(|l| l.weight).sum();
    // 参与权重为 0 时没有「未覆盖」可言，报 0 而不是 NaN
    let unverified_weight_ratio = if participating_weight > 0.0 {
        (1.0 - verified_weight / participating_weight).clamp(0.0, 1.0)
    } else {
        0.0
    };

    // 任一腿的 σ / 权重缺失 ⇒ 闭合**无从判定**。
    // 理由：落库的 `avgSignal` 是脚本拿它自己的默认值算出来的，审计若在这里补 0，
    // 就是拿一个伪造的输入去验证一个伪造的结论 —— 缺失反而被"闭合"背书，比不判更糟。
    let inputs_complete = factor_legs.iter().all(|l| l.sigma.is_some() && l.weight.is_some());

    let closure = match (
        as_f(pointer(decision_json, "/evidence/weightSum")),
        as_f(pointer(decision_json, "/evidence/avgSignal")),
    ) {
        (Some(w), Some(stored)) if w > 0.0 && !factor_legs.is_empty() => {
            if !inputs_complete {
                Closure::NotComputable { reason: "MISSING_FACTOR_INPUT".into() }
            } else {
                // 已被 `inputs_complete` 证明全为 Some；兜底分支不会命中
                let recomputed: f64 = factor_legs
                    .iter()
                    .map(|l| l.sigma.unwrap_or_default() * l.weight.unwrap_or_default())
                    .sum::<f64>()
                    / w;
                let tol = closure_tolerance(factor_legs.len(), w);
                let delta = (recomputed - stored).abs();
                if delta <= tol {
                    Closure::Closed { recomputed, stored, tolerance: tol, unverified_weight_ratio }
                } else {
                    Closure::Broken { recomputed, stored, tolerance: tol }
                }
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
    let sigma_reconciled_legs = factor_legs.iter().filter(|l| is_verified(l)).count();
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
            "result.t-valuation-band": { "content": "{}" },
            "result.t-limitup-pool": { "content": "{}" },
            // 工具节点在快照里是 `{"content": "<JSON 字符串>"}` 双层形态；取真记录的 78
            "result.t-scoring": { "content": r#"{"totalScore": 78, "bollScore": 3}"# }
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

        // sharedTool 臂（P-B5）：channel 是工具名 `get_limit_up_pool`，经权威表定位到
        // `result.t-limitup-pool`，该键在夹具里在场 ⇒ 判得到场
        assert_eq!(
            leg("breadthState").upstream,
            UpstreamState::Present,
            "工具名通道必须能经 CHANNEL_SNAPSHOT_KEYS 定位上游"
        );
        // tierScore 臂：momentumSignal 的 channel 是字段名 `macd`，逐档评分节点产出不落主快照
        // ⇒ 如实报「不适用」，既不算在场也不算缺席
        assert!(
            matches!(leg("momentumSignal").upstream, UpstreamState::NotChecked { .. }),
            "tierScore 通道不得被判成在场，实际 {:?}",
            leg("momentumSignal").upstream
        );
        assert_eq!(r.upstream_not_checked_legs, 1, "只剩 tierScore 那一条不可判");
        assert_eq!(r.upstream_present_legs, 2, "eventCatalyst + breadthState 两条能判到场");
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

        // 负控：撤掉该节点键 ⇒ 同一腿必须翻面。⚠ 翻到的是**静默缺席**而不是「判不了」：
        // 映射表里有这个键（判据能判），缺的只是本记录里那个节点的产出 ⇒ 上游真没出数。
        let mut bb3 = bb2.clone();
        bb3.as_object_mut().unwrap().remove("result.t-valuation-band");
        let r3 = run(&dj, &bb3, false);
        let l3 = r3.branch_legs.iter().find(|l| l.factor == "valuationBand").unwrap();
        assert_eq!(
            l3.upstream,
            UpstreamState::SilentAbsent,
            "映射在场而记录缺键 ⇒ 静默缺席（不是 NotChecked）"
        );
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

        // ③ 只作过滤、带 constraint 的腿：角色上「有交代」与上游判据是**两条独立轴** ——
        // 它不参与方向加权（role=filter），但上游经 P-B5 表判得到场
        let breadth = r.branch_legs.iter().find(|l| l.factor == "breadthState").unwrap();
        assert!(!breadth.participates, "role=filter 不得算参与");
        assert!(breadth.absence_declared, "带 constraint 的过滤腿必须有交代");
        assert_eq!(breadth.upstream, UpstreamState::Present, "不参与 ≠ 上游不在场");

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
    /// 2026-10-21：`dataGaps` 里的**设计说明**不是缺席声明。
    ///
    /// 现网 688498 的 mid 档把 supplyShock 放进 `legs`（signal=0、role=riskNote、
    /// constraint=supplyAsRiskNoteOnly），同一份 `h_mid.dataGaps` 里还有「只作风险提示、
    /// 不进方向」与「共 1 条」两句。按因子名回收会把一条**在场**的腿标成「显式缺席」。
    #[test]
    fn design_note_in_data_gaps_does_not_mask_a_present_leg() {
        let (dj, bb) = fixture();
        let mut bb2 = bb.clone();
        bb2["result.h_ultra_short"]["legs"].as_array_mut().unwrap().push(json!({
            "factor": "supplyShock", "role": "riskNote", "signal": 0.0, "weight": 0.0,
            "constraint": "supplyAsRiskNoteOnly"
        }));
        bb2["result.h_ultra_short"]["dataGaps"] =
            json!(["超短档 supplyShock 只作风险提示（role=riskNote、权重恒 0），不进方向"]);
        bb2["result.t-lockup-data"] = json!({ "content": "{}" });

        let r = run(&dj, &bb2, false);
        let s = r.branch_legs.iter().find(|l| l.factor == "supplyShock").unwrap();
        assert_eq!(s.upstream, UpstreamState::Present, "在场且有信号的设计说明不得被判缺席");
        assert!(s.absence_declared, "constraint 本身就是交代");

        // 负控①：同一因子从 `legs` 撤掉、句子换成真缺席声明 ⇒ 必须合成「显式缺席」行
        let mut bb3 = bb2.clone();
        bb3["result.h_ultra_short"]["legs"]
            .as_array_mut()
            .unwrap()
            .retain(|l| l["factor"] != "supplyShock");
        bb3["result.h_ultra_short"]["dataGaps"] =
            json!(["超短档 supplyShock 无数据：t-lockup-data 没有产出 supply_shock 块"]);
        let r3 = run(&dj, &bb3, false);
        let s3 = r3.branch_legs.iter().find(|l| l.factor == "supplyShock").unwrap();
        assert_eq!(s3.upstream, UpstreamState::DeclaredAbsent);
        assert!(s3.absence_declared);

        // 负控②：不在 `legs`、但只有设计说明 ⇒ 不得无中生有一条缺席行
        let mut bb4 = bb3.clone();
        bb4["result.h_ultra_short"]["dataGaps"] =
            json!(["超短档 supplyShock 只作风险提示（role=riskNote、权重恒 0），不进方向"]);
        let r4 = run(&dj, &bb4, false);
        assert!(
            r4.branch_legs.iter().all(|l| l.factor != "supplyShock"),
            "只有设计说明时不得合成缺席行"
        );
    }

    /// 裁决层置零必须随腿透传：它和「没接线 / 没数」是两种完全不同的原因。
    #[test]
    fn measured_zeroing_is_distinct_from_missing_input() {
        let (dj, bb) = fixture();
        let mut bb2 = bb.clone();
        bb2["result.h_ultra_short"]["legs"][0]["weight"] = json!(0.0);
        bb2["result.h_ultra_short"]["legs"][0]["icStatus"] = json!("ci_spans_zero");
        bb2["result.h_ultra_short"]["legs"][0]["icZeroed"] = json!(true);
        let r = run(&dj, &bb2, false);
        let zeroed = r.branch_legs.iter().find(|l| l.factor == "momentumSignal").unwrap();
        assert_eq!(zeroed.ic_status.as_deref(), Some("ci_spans_zero"));
        assert!(zeroed.ic_zeroed, "裁决置零必须透传，面板据此不再报「未交代」");
        assert!(!zeroed.participates, "权重已置零 ⇒ 不得算参与");
        assert!(zeroed.expected_to_participate, "它本该参与 —— 这正是要归因的那一格");

        // 负控：没有该键的腿不得被当成置零（旧记录 = 没携带，不是「测过了」）
        let untouched = r.branch_legs.iter().find(|l| l.factor == "eventCatalyst").unwrap();
        assert_eq!(untouched.ic_status, None);
        assert!(!untouched.ic_zeroed);
    }

    /// `injectedVar` 腿在面板上必须显示因子中文名，不得回退成英文键；
    /// 上游判定如实报「无法核查」（快照没有登记可核对的键），而不是「静默缺席」。
    #[test]
    fn injected_var_leg_uses_factor_display_name_and_not_checked() {
        let (dj, bb) = fixture();
        let mut bb2 = bb.clone();
        bb2["result.h_ultra_short"]["legs"].as_array_mut().unwrap().push(json!({
            "factor": "relativeStrength", "role": "direction", "signal": 0.2,
            "weight": 0.1, "constraint": null
        }));
        let r = run(&dj, &bb2, false);
        let l = r.branch_legs.iter().find(|x| x.factor == "relativeStrength").unwrap();
        assert_eq!(l.owner, None, "injectedVar 没有分析师属主");
        assert_eq!(l.owner_name, "相对强度", "无属主因子不得回退成英文键名");
        match &l.upstream {
            UpstreamState::NotChecked { reason } => {
                assert_eq!(reason, "INJECTED_VAR_HAS_NO_KEY_TABLE");
            },
            other => panic!("injectedVar 应报无法核查，实际 {other:?}"),
        }
        assert_eq!(r.upstream_not_checked_legs, 2, "tierScore + injectedVar 两条不可判");
    }

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
            Reconcile::Consistent { delta, tolerance, basis, source } => {
                assert_eq!(basis, "/risk_quant/f4_adjusted");
                // f4 的 σ 与基准取自同一个 `f4_adjusted` 变量 ⇒ 必须标成同源副本，
                // 否则面板会把它和 f1（独立快照）染成同一种「已核对」绿。
                assert_eq!(*source, BasisSource::SameVariableCopy);
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
        // f1（独立快照）/ f4 / f11 / f12 四条都在容差内，但只有 f1 的基准独立于 σ
        // ⇒ 可核计数只认 f1 一条。这是 2026-10-18 修掉「同源副本互比算作已核对」后的读数。
        assert_eq!(r.sigma_reconciled_legs, 1, "只有独立上游基准能计入可核");
        let f1 = r.factor_legs.iter().find(|l| l.leg_id == "f1").unwrap();
        match &f1.reconcile {
            Reconcile::Consistent { source, .. } => {
                assert_eq!(*source, BasisSource::IndependentUpstream)
            },
            other => panic!("f1 应判一致，实际 {other:?}"),
        }
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

    /// A 面 f1：σ 必须能经**快照里的总评分** + 上收后的归一化口径回算出来（裁定 P-B1 落地）。
    /// 夹具取真记录那一对：`totalScore = 78` ⇒ `score_to_signal(78) = 0.56` = 落库 σ。
    #[test]
    fn f1_is_reconciled_from_the_snapshot_score() {
        let (dj, bb) = fixture();
        let r = run(&dj, &bb, false);
        let f1 = r.factor_legs.iter().find(|l| l.leg_id == "f1").unwrap();
        match &f1.reconcile {
            Reconcile::Consistent { delta, basis, .. } => {
                assert_eq!(basis, "/result.t-scoring/content/totalScore");
                assert_eq!(*delta, 0.0, "同一口径现算应与落库 σ 逐位相等");
            },
            other => panic!("f1 应判一致，实际 {other:?}"),
        }

        // 负控①：把快照里的总评分改掉 ⇒ 必须翻成不一致（证明上一条不是恒真）
        let mut bb2 = bb.clone();
        bb2["result.t-scoring"]["content"] = json!(r#"{"totalScore": 60}"#);
        let r2 = run(&dj, &bb2, false);
        let f1b = r2.factor_legs.iter().find(|l| l.leg_id == "f1").unwrap();
        assert!(
            matches!(f1b.reconcile, Reconcile::Inconsistent { .. }),
            "总分变了而落库 σ 没变 ⇒ 必须报不一致"
        );

        // 负控②：撤掉快照键 ⇒ NoBasis（不是「一致」）
        let mut bb3 = bb.clone();
        bb3.as_object_mut().unwrap().remove("result.t-scoring");
        let r3 = run(&dj, &bb3, false);
        let f1c = r3.factor_legs.iter().find(|l| l.leg_id == "f1").unwrap();
        assert!(matches!(f1c.reconcile, Reconcile::NoBasis { .. }), "缺快照键必须报 NoBasis");
    }

    /// A 面 f2：**如实报无基准**。它的输入取自收敛节点，而该节点正随「逐档实例化」在飞 ——
    /// 同一条记录上老形态的键（裸 `debate-convergence.content.consensus_score`）与新形态
    /// 按 seed 指向的 `debate-convergence--mid.*`（v161）**都不是 `result.*`**，给一条静态
    /// 路径就是把「对不上」伪装成「对过了」。
    #[test]
    fn f2_stays_without_basis_until_the_convergence_node_settles() {
        let (dj, bb) = fixture();
        let mut dj2 = dj.clone();
        dj2["evidence"]["factors"]["f2"] =
            json!({ "name": "consensus", "sigma": 0.1, "weight": 0.25 });
        let r = run(&dj2, &bb, false);
        let f2 = r.factor_legs.iter().find(|l| l.leg_id == "f2").unwrap();
        assert_eq!(f2.reconcile, Reconcile::NoBasis { reason: "NO_PERSISTED_BASIS".into() });
    }

    /// 全局闭合：容差按 `δσ + δw·n/W + δavg` 现算；越界必须报 Broken。
    #[test]
    fn closure_tolerance_is_derived_and_breaks_when_out_of_range() {
        let (dj, bb) = fixture();
        let r = run(&dj, &bb, false);
        let (recomputed, stored, tolerance, unverified) = match r.closure {
            Closure::Closed { recomputed, stored, tolerance, unverified_weight_ratio } => {
                (recomputed, stored, tolerance, unverified_weight_ratio)
            },
            ref other => panic!("夹具应闭合，实际 {other:?}"),
        };
        // 覆盖率必须与「闭合」一起给出：夹具 4 条参与腿（权重 .105/.15/.052/.075）里
        // 只有 f1 的基准独立 ⇒ 72.5% 的参与权重从未经任何外部数据核对。
        // 没有这个读数时，「闭合」会被读成「证据链已核对」，而它只证明算术自洽。
        let expected_unverified = 1.0 - 0.105 / (0.105 + 0.15 + 0.052 + 0.075);
        assert!(
            (unverified - expected_unverified).abs() < 1e-9,
            "未核权重占比应为 {expected_unverified}，实际 {unverified}"
        );
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
    /// IPC 键名：`Closure` 是枚举，`rename_all` 只作用于 variant 名，**不作用于
    /// variant 内字段**。漏了字段级 rename 时前端读 `unverifiedWeightRatio` 得
    /// `undefined`，覆盖率被算成 `NaN%`（2026-10-21 截图实证）。
    #[test]
    fn closure_serialises_coverage_as_camel_case() {
        let v = serde_json::to_value(Closure::Closed {
            recomputed: 0.0839,
            stored: 0.08,
            tolerance: 0.0057,
            unverified_weight_ratio: 0.83,
        })
        .expect("Closure 必须可序列化");
        assert_eq!(v["state"], "closed");
        assert!(v.get("unverifiedWeightRatio").is_some(), "IPC 键必须是 camelCase，实际 {v}");
        assert!(
            v.get("unverified_weight_ratio").is_none(),
            "snake_case 键会让前端读成 undefined ⇒ 覆盖率 NaN%，实际 {v}"
        );
    }

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

    /// 2026-10-18：可核计数的**三重排除**各配一个负控。
    ///
    /// 修前分子是「`Consistent` 腿数」，把两类不构成证据的绿都算了进去：
    /// ① `weight = 0` 的腿 —— f10 的 σ=0 ⇒ w=0 是产端构造耦合（`portfolio-mgr.rhai:1533`），
    /// ② 基准与 σ 同变量的副本互比 —— delta 上限恒等于容差推导值，数学上永不越界。
    #[test]
    fn reconciled_count_excludes_nonparticipating_and_same_source_copies() {
        let (dj, bb) = fixture();
        let r = run(&dj, &bb, false);

        // 修前口径（四条全绿）与修后口径（只认独立源）必须能被区分开
        let raw_consistent = r
            .factor_legs
            .iter()
            .filter(|l| matches!(l.reconcile, Reconcile::Consistent { .. }))
            .count();
        assert_eq!(raw_consistent, 4, "f1/f4/f11/f12 都在容差内 —— 这正是修前的分子");
        assert_eq!(r.sigma_reconciled_legs, 1, "修后只认基准独立的 f1");

        // ① 参与过滤：f1 权重清零后仍「一致」，但不得再计入分子
        let mut dj2 = dj.clone();
        dj2["evidence"]["factors"]["f1"]["weight"] = json!(0.0);
        let r2 = run(&dj2, &bb, false);
        let f1b = r2.factor_legs.iter().find(|l| l.leg_id == "f1").unwrap();
        assert!(matches!(f1b.reconcile, Reconcile::Consistent { .. }), "改权重不动 σ 与基准的关系");
        assert_eq!(r2.sigma_reconciled_legs, 0, "w=0 的腿不构成证据");

        // ② 同源副本的恒真性：把 σ 与基准**同时**改成同一个值 ⇒ 照样「一致」
        //    （证明这条绿来自两个副本同源，不是数据干净）
        let mut dj3 = dj.clone();
        dj3["evidence"]["f11_pace_signal"] = json!(0.77);
        dj3["evidence"]["factors"]["f11"]["sigma"] = json!(0.77);
        let r3 = run(&dj3, &bb, false);
        let f11c = r3.factor_legs.iter().find(|l| l.leg_id == "f11").unwrap();
        assert!(matches!(f11c.reconcile, Reconcile::Consistent { .. }), "两副本同改必然一致");
        assert_eq!(r3.sigma_reconciled_legs, 1, "同源副本仍不进分子");

        // ③ 但要保留它的唯一价值：**单边**改动必须翻红（副本托管检测仍有效）
        let mut dj4 = dj.clone();
        dj4["evidence"]["f11_pace_signal"] = json!(0.77);
        let r4 = run(&dj4, &bb, false);
        let f11d = r4.factor_legs.iter().find(|l| l.leg_id == "f11").unwrap();
        assert!(matches!(f11d.reconcile, Reconcile::Inconsistent { .. }), "单边改动必须翻红");
    }

    /// 因子腿必须按**数值序**排列。`ids.sort()` 的字符串序会把 `f10`~`f13` 排到 `f2`
    /// 前面（2026-10-18 截图上 12 行顺序逐行印证），读者按 f1→f13 找腿时全乱。
    #[test]
    fn factor_legs_are_ordered_numerically_not_lexicographically() {
        let (mut dj, bb) = fixture();
        for id in ["f2", "f3", "f5", "f6", "f7", "f9", "f10", "f13"] {
            dj["evidence"]["factors"][id] = json!({ "name": id, "sigma": 0.0, "weight": 0.0 });
        }
        let r = run(&dj, &bb, false);
        let ids: Vec<&str> = r.factor_legs.iter().map(|l| l.leg_id.as_str()).collect();
        assert_eq!(
            ids,
            vec!["f1", "f2", "f3", "f4", "f5", "f6", "f7", "f9", "f10", "f11", "f12", "f13"],
            "必须是数值序"
        );
        // 负控：两种排法确实不同，否则本测试证明不了「换掉了字符串序」
        let mut lex = ids.clone();
        lex.sort_unstable();
        assert_ne!(lex, ids, "字符串序与本序列相同 ⇒ 夹具无法区分两种排法");
    }

    /// 2026-10-18：「字段缺失」与「值真的是 0」必须可分，且**缺失不得让闭合通过**。
    ///
    /// 修前两处 `unwrap_or(0.0)` 把缺失兜成 0：σ 照样进 `Σ(σ·w)`、weight 照样参与
    /// `participates` 判定 ⇒ 数据坏了在面板上与「信号为零」同形，闭合还能照常绿。
    /// 最坏的一种是「用缺失值喂出一个闭合」——比不判更糟，因为它给了通过背书。
    #[test]
    fn missing_input_is_distinct_from_zero_and_blocks_closure() {
        let (dj, bb) = fixture();
        let base = run(&dj, &bb, false);
        assert!(matches!(base.closure, Closure::Closed { .. }), "夹具基线应闭合");

        // ① 把 f4 的 sigma 键整个删掉（不是设 0）
        let mut dj2 = dj.clone();
        dj2["evidence"]["factors"]["f4"].as_object_mut().unwrap().remove("sigma");
        let r2 = run(&dj2, &bb, false);
        let f4 = r2.factor_legs.iter().find(|l| l.leg_id == "f4").unwrap();
        assert_eq!(f4.sigma, None, "缺键必须解析成 None，不是 0");
        assert_eq!(f4.reconcile, Reconcile::InputMissing { field: "sigma".into() });
        assert_eq!(
            r2.closure,
            Closure::NotComputable { reason: "MISSING_FACTOR_INPUT".into() },
            "输入缺失时闭合必须报无从判定，不得拿 0 补齐后照常通过"
        );

        // ② 负控：把 sigma 显式设成 0 ⇒ 仍属「有值」，走正常对账而非 InputMissing
        let mut dj3 = dj.clone();
        dj3["evidence"]["factors"]["f4"]["sigma"] = json!(0.0);
        let r3 = run(&dj3, &bb, false);
        let f4b = r3.factor_legs.iter().find(|l| l.leg_id == "f4").unwrap();
        assert_eq!(f4b.sigma, Some(0.0), "显式 0 是「有值」");
        assert!(
            matches!(f4b.reconcile, Reconcile::Inconsistent { .. }),
            "σ=0 与基准 −0.17 差 0.17 ⇒ 报不一致，而不是「缺失」"
        );

        // ③ 权重缺失：不得静默变成「0 权重参与」
        let mut dj4 = dj.clone();
        dj4["evidence"]["factors"]["f11"].as_object_mut().unwrap().remove("weight");
        let r4 = run(&dj4, &bb, false);
        let f11 = r4.factor_legs.iter().find(|l| l.leg_id == "f11").unwrap();
        assert_eq!(f11.weight, None);
        assert!(!f11.participates, "权重缺失不得算作参与");
        assert!(matches!(r4.closure, Closure::NotComputable { .. }));

        // ④ B 面同族：分支腿的 weight 缺失同样不得被当作 0
        let mut bb2 = bb.clone();
        bb2["result.h_ultra_short"]["legs"][0].as_object_mut().unwrap().remove("weight");
        let r5 = run(&dj, &bb2, false);
        let momentum = r5.branch_legs.iter().find(|l| l.factor == "momentumSignal").unwrap();
        assert_eq!(momentum.weight, None);
        assert!(!momentum.participates);
        assert!(
            momentum.expected_to_participate,
            "它本该参与 —— 正是「应参与未计入」那一格要报的形态"
        );
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

    /// 门（P-B1）：评分 → 因子信号的归一化**不得再内联**，五处调用点必须全走唯一权威
    /// `portfolio_formula::score_to_signal`（主链经宿主函数，逐档腿直接调）。
    ///
    /// 判据针对**实质**而非某一种写法：脚本侧只要求「调了宿主函数」，不锁整行格式
    /// （rustfmt/手改缩进不该假红）。
    #[test]
    fn score_normalization_is_not_inlined_anywhere() {
        const PM: &str = include_str!("../../../../src-tauri/src/commands/portfolio-mgr.rhai");
        assert!(
            !PM.contains("- 50.0) / 50.0"),
            "主链又把归一化内联回来了 ⇒ 权威应只有 portfolio_formula::score_to_signal"
        );
        for call in [
            "pm_score_to_signal(totalScore)",
            "pm_score_to_signal(dqi_score)",
            "pm_score_to_signal(ss_f)",
        ] {
            assert!(PM.contains(call), "主链缺这条归一化调用：{call}");
        }
        // f2 不在上述清单里（v182 起）：`consensus_score` 按契约是**收敛程度**、无方向含义，
        // 「减中点除以中点」那套「评分→信号」的归一化对它**语义就不成立** —— 现网一条
        // direction=neutral、consensus_score=55 的记录被它投成了看多。f2 现在读
        // `aggregate_prediction.direction` 的符号 × 收敛度，见主链 f2 段注释。
        // 这条断言是**反向锁**：谁把旧口径抄回来，这里当场红。
        assert!(
            !PM.contains("pm_score_to_signal(consensusScore)"),
            "主链又把「收敛程度」按评分→信号当方向用了 ⇒ 违反 f2 的方向契约（v182）"
        );
        assert!(
            PM.contains("consensusDirection"),
            "主链 f2 必须消费辩论聚合方向 `consensusDirection`，否则方向又落回 consensus_score"
        );

        // 逐档腿那一份也必须走同一实现（原先它自带一份内联的 `(ts - 50.0) / 50.0`）
        const LEG: &str = include_str!("leg_signal.rs");
        assert!(!LEG.contains("(ts - 50.0) / 50.0"), "leg_signal 又内联了归一化");
        assert!(LEG.contains("score_to_signal(ts)"), "leg_signal 应复用唯一权威");
    }
}
