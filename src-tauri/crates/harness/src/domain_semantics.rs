// SPDX-License-Identifier: AGPL-3.0-only

//! 领域语义注册表 — 「表面名 ↔ 概念身份」的权威声明层。
//!
//! ## 为什么需要它
//!
//! 项目内同一个表面名承载**多个不同概念**，且值域不同（`0–1` vs `0–100`）。
//! 两边类型都是 `f64`（或 `u8`），类型系统看不出来；静态扫描也无法区分
//! 「概念 A 有两个量纲」与「概念 A / 概念 B 同名」——**缺的正是这份权威声明**。
//!
//! 两个已发生的一手事故（非假设，均在源码留有修复注释）：
//! - [`DashboardReport::target_price`](crate::dashboard_report::DashboardReport) 与
//!   `intrinsic_value_*` 被 UI 同名展示 ⇒ 用户读到「同一工作流结论矛盾」（603466）。
//!   `src-tauri/crates/harness/src/dashboard_report.rs:52-55` 有 ⚠️ 注释。
//! - `position_pct / total_mv * 100.0` **量纲错乱**（百分比 ÷ 市值 × 100）。
//!   `src-tauri/crates/analysis-engine/src/exit_recommend.rs:392` 有修复注释。
//!
//! ## 与相邻模块的分工
//!
//! | 模块 | 层次 | 回答的问题 |
//! |---|---|---|
//! | `analysis-engine::concept_index` | 实体层 | 「AI 概念包含哪些股票」 |
//! | [`dashboard_report`](crate::dashboard_report) | 运行时 | 「这份报告的值越界了吗」 |
//! | **本模块** | 字段层 | 「`confidence` 此刻指的是哪个概念、值域多少」 |
//!
//! ## 用法
//!
//! - 改动承载字段前：先 [`by_name`] 查该名字还有哪些概念，避免误判量纲；
//! - 工具链（`scripts/check-domain-semantics.mjs`）以 [`CONCEPTS`] 为输入，**不再自带清单**；
//! - 新增多义 ⇒ [`ambiguous_names`] 的锁定测试失败 ⇒ 强制显式登记（防止腐烂）。
//!
//! 本模块还提供两个**量纲化标量** [`Ratio01`] / [`Percent100`]：声明管「工具能读」，
//! 标量管「编译器能拦」。字段类型把它们用起来，量纲归属就从「靠人看」变成「靠编译」。
//!
//! 定位：声明 + 查询 + 两个钳制型标量，**无运行时状态、无 IO、无阻塞**
//! （符合本 crate「零业务逻辑」约束；钳制是纯函数，不构成业务决策）。

use serde::{Deserialize, Serialize};

/// 量纲 — 概念值域的物理/数学单位。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unit {
    /// 无量纲比 / 概率：`[0, 1]`
    Ratio,
    /// 百分数：`[0, 100]`
    Percent,
    /// 金额，单位：**元**
    Cny,
    /// 损失 / 误差值：**越小越好，无统一上界**（如参数校准的拟合误差）
    Loss,
}

impl Unit {
    /// 该量纲的规范值域 `(下界, 上界)`，两端均含。
    pub const fn canonical_range(self) -> (f64, f64) {
        match self {
            Unit::Ratio => (0.0, 1.0),
            Unit::Percent => (0.0, 100.0),
            // 金额与损失值均无统一上界（下界为 0）
            Unit::Cny | Unit::Loss => (0.0, f64::INFINITY),
        }
    }

    /// 量纲的中文标签（报告与体检输出用）。
    pub const fn label(self) -> &'static str {
        match self {
            Unit::Ratio => "0-1 比",
            Unit::Percent => "0-100 百分数",
            Unit::Cny => "元",
            Unit::Loss => "损失值（越小越好）",
        }
    }
}

// ── 量纲化标量：让**编译器**承担「归属」判断 ────────────────────────────
//
// 注册表（[`CONCEPTS`]）是**声明**：工具能读它，却拦不住新代码写错量纲
// —— `f64` 之间互相赋值永远合法。下面这两个 newtype 是**执行**侧的另一半：
// 字段类型一旦是 [`Ratio01`]，0–30.7 或 85.0 就不可能在「构造处」悄悄通过。
//
// 设计取舍（刻意如此，勿顺手"简化"）：
// - **构造即钳制**而不 panic：越界多来自上游数据而非编程错误，panic 会把
//   「量纲错」升级为「链路断」。需要严格判定时用 `is_in_range` 自行处理。
// - **两个单位之间必须显式换算**（只提供 `From`，不提供 `as`/乘法捷径）
//   —— 隐式穿越正是本文件所记录的两起事故的成因。
// - `#[serde(transparent)]`：**JSON 形状不变**（仍是裸数字），
//   否则前端契约会被无声改变（前端 `matchConfidence * 100` 依赖这个形状）。

/// 无量纲比 / 概率，值域 `[0, 1]` —— 对应 [`Unit::Ratio`]。构造即钳制。
///
/// ⚠ **不派生 `Deserialize`**：`#[serde(transparent)]` 会让派生的 `Deserialize`
/// **直连内层 `f64`**，从而**绕过** [`new`](Self::new) —— 于是「构造即钳制」在
/// JSON / DB 回读路径上完全失效。2026-09-14 实测：`from_str("450.0")` 得到 `450.0`，
/// 而 `new(450.0)` 得到 `1.0`。**校验写在构造函数里、却有一条 ingress 绕过构造函数**，
/// 是本项目反复出现的形态 ⇒ 反序列化必须显式走 `new`（见下方手写实现）。
///
/// `Serialize` 仍可派生（新类型结构体天然序列化为内层值，即裸数字）。
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct Ratio01(f64);

/// 反序列化显式经过 [`Ratio01::new`]，让钳制在**每一条入口**都生效。
impl<'de> Deserialize<'de> for Ratio01 {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        Ok(Self::new(f64::deserialize(deserializer)?))
    }
}

impl Ratio01 {
    /// `0` 的具名常量（避免链路上出现无归属的裸 `0.0`）。
    pub const ZERO: Self = Self(0.0);
    /// `1` 的具名常量。
    pub const ONE: Self = Self(1.0);

    /// 构造：越界钳制到 `[0, 1]`；`NaN` 归 [`ZERO`](Self::ZERO)。
    pub fn new(v: f64) -> Self {
        if v.is_nan() {
            return Self::ZERO;
        }
        Self(v.clamp(0.0, 1.0))
    }

    /// 取值。
    pub const fn get(self) -> f64 {
        self.0
    }

    /// 原始值是否**本就**落在规范值域内（用于审计越界输入，钳制会掩盖它）。
    pub fn is_in_range(v: f64) -> bool {
        !v.is_nan() && (0.0..=1.0).contains(&v)
    }
}

/// 百分数，值域 `[0, 100]` —— 对应 [`Unit::Percent`]。构造即钳制。
///
/// 同 [`Ratio01`]：**不派生 `Deserialize`**，理由与实现完全一致（绕过构造函数）。
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct Percent100(f64);

/// 反序列化显式经过 [`Percent100::new`]。
impl<'de> Deserialize<'de> for Percent100 {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        Ok(Self::new(f64::deserialize(deserializer)?))
    }
}

impl Percent100 {
    /// `0` 的具名常量。
    pub const ZERO: Self = Self(0.0);
    /// `100` 的具名常量（满值）。
    pub const MAX: Self = Self(100.0);

    /// 构造：越界钳制到 `[0, 100]`；`NaN` 归 [`ZERO`](Self::ZERO)。
    pub fn new(v: f64) -> Self {
        if v.is_nan() {
            return Self::ZERO;
        }
        Self(v.clamp(0.0, 100.0))
    }

    /// 取值。
    pub const fn get(self) -> f64 {
        self.0
    }

    /// 原始值是否**本就**落在规范值域内。
    pub fn is_in_range(v: f64) -> bool {
        !v.is_nan() && (0.0..=100.0).contains(&v)
    }
}

/// `比率 → 百分数`（×100）。**唯一允许的跨单位通道**，调用处必须写明类型。
impl From<Ratio01> for Percent100 {
    fn from(r: Ratio01) -> Self {
        Self::new(r.get() * 100.0)
    }
}

/// `百分数 → 比率`（÷100）。同样显式。
impl From<Percent100> for Ratio01 {
    fn from(p: Percent100) -> Self {
        Self::new(p.get() / 100.0)
    }
}

/// 一个「概念身份」的声明。
///
/// **不变量**（由本模块的测试锁定）：
/// - [`id`](Self::id) 全局唯一；
/// - [`name`](Self::name) **允许重复** —— 重复即「多义」，此时 [`carrier`](Self::carrier) 必须足以消歧；
/// - `min`/`max` 必须落在 [`unit`](Self::unit) 的规范值域内。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ConceptDecl {
    /// 全局唯一概念 ID，形如 `<项目>.<领域>.<名>`（如 `axinvest.report.confidence`）。
    pub id: &'static str,
    /// 表面名：代码里真实出现的字段名 / 变量名（**可重复 = 多义**）。
    pub name: &'static str,
    /// 承载者：消歧依据（结构体名 / 变量用途）。同名多义时**必须**能区分。
    pub carrier: &'static str,
    /// 量纲。
    pub unit: Unit,
    /// 值域下界（含）。
    pub min: f64,
    /// 值域上界（含）。无穷表示「无统一上界」。
    pub max: f64,
    /// 人读语义（一句话）。
    pub meaning: &'static str,
    /// 权威依据，格式 `<仓库根相对路径>:<行号>`，多个用 ` | ` 分隔。
    pub evidence: &'static str,
}

/// 已登记概念的全量清单 —— **本表的唯一权威源**。
///
/// 登记标准：每一行都必须有可核对的源码锚点（[`ConceptDecl::evidence`]）。
/// 未经实证的条目**不得**写入（否则本表会退化为「无权威源的现状副本」）。
pub const CONCEPTS: &[ConceptDecl] = &[
    // ─────────── `confidence` 族：一名多义的典型现场 ───────────
    // 0–100 组（投资决策侧）
    ConceptDecl {
        id: "axinvest.report.confidence",
        name: "confidence",
        carrier: "DashboardReport",
        unit: Unit::Percent,
        min: 0.0,
        max: 100.0,
        meaning: "投资决策报告的综合置信度",
        evidence: "src-tauri/crates/harness/src/dashboard_report.rs:42 | src-tauri/crates/harness/src/dashboard_report.rs:260",
    },
    ConceptDecl {
        id: "axinvest.pick.confidence",
        name: "confidence",
        carrier: "RecommenderPick (u8)",
        unit: Unit::Percent,
        min: 0.0,
        max: 100.0,
        meaning: "荐股条目的推荐置信度",
        evidence: "src-tauri/crates/analysis-engine/src/recommender/mod.rs:649",
    },
    // 0–1 组（各自独立的若干概念）
    ConceptDecl {
        id: "axinvest.candle_pattern.confidence",
        name: "confidence",
        carrier: "CandlePattern",
        unit: Unit::Ratio,
        min: 0.0,
        max: 1.0,
        meaning: "K 线形态识别置信度",
        evidence: "src-tauri/crates/analysis-engine/src/exit_recommend.rs:347",
    },
    ConceptDecl {
        id: "axinvest.market_regime.confidence",
        name: "confidence",
        carrier: "MarketRegime",
        unit: Unit::Ratio,
        min: 0.0,
        max: 1.0,
        meaning: "市场状态（牛/熊/震荡）判定置信度",
        evidence: "src-tauri/crates/analysis-engine/src/market_regime.rs:14",
    },
    ConceptDecl {
        id: "axagent.route_stage.confidence",
        name: "confidence",
        carrier: "RouteStageRecord",
        unit: Unit::Ratio,
        min: 0.0,
        max: 1.0,
        meaning: "认知路由单阶段判定置信度",
        evidence: "src-tauri/crates/harness/src/cognitive_router.rs:323",
    },
    ConceptDecl {
        id: "axagent.node_rec.confidence",
        name: "confidence",
        carrier: "NodeRecommendation",
        unit: Unit::Ratio,
        min: 0.0,
        max: 1.0,
        meaning: "工作流节点推荐的匹配置信度",
        evidence: "src-tauri/src/commands/workflow_ai/generate.rs:388",
    },
    ConceptDecl {
        id: "axagent.skill_extract.confidence",
        name: "confidence",
        carrier: "局部变量（技能内容提取）",
        unit: Unit::Ratio,
        min: 0.0,
        max: 1.0,
        meaning: "从技能文档提取内容的置信度",
        evidence: "src-tauri/src/commands/skills/management.rs:1248",
    },
    ConceptDecl {
        id: "axagent.skill_upgrade.confidence",
        name: "confidence",
        carrier: "局部变量（技能升级提议）",
        unit: Unit::Ratio,
        min: 0.0,
        max: 1.0,
        meaning: "技能升级提议的置信度",
        evidence: "src-tauri/src/commands/agent_nudge.rs:134",
    },
    ConceptDecl {
        id: "axagent.adaptive_decision.confidence",
        name: "confidence",
        carrier: "自适应分析决策结构",
        unit: Unit::Ratio,
        min: 0.0,
        max: 1.0,
        meaning: "自适应分析的默认决策置信度",
        evidence: "src-tauri/crates/analysis-engine/src/stock_adaptive_engine.rs:539",
    },
    ConceptDecl {
        id: "axagent.routing_result.confidence",
        name: "confidence",
        carrier: "路由结果结构",
        unit: Unit::Ratio,
        min: 0.0,
        max: 1.0,
        meaning: "路由判定的置信度（含 LLM 回退档）",
        evidence: "src-tauri/src/commands/cognitive.rs:2196",
    },
    // ─────────── `score` 族 ───────────
    ConceptDecl {
        id: "axinvest.report.score",
        name: "score",
        carrier: "DashboardReport (u32)",
        unit: Unit::Percent,
        min: 0.0,
        max: 100.0,
        meaning: "投资决策报告的综合评分",
        evidence: "src-tauri/crates/harness/src/dashboard_report.rs:38 | src-tauri/crates/harness/src/dashboard_report.rs:254",
    },
    ConceptDecl {
        id: "axinvest.round_eval.score",
        name: "score",
        carrier: "RoundEvaluation",
        unit: Unit::Ratio,
        min: 0.0,
        max: 1.0,
        meaning: "分析轮次的质量评估得分",
        evidence: "src-tauri/crates/analysis-engine/src/stock_analysis_round.rs:312 | src-tauri/crates/analysis-engine/src/stock_analysis_round.rs:343",
    },
    ConceptDecl {
        id: "axinvest.evidence.match_confidence",
        name: "match_confidence",
        carrier: "EvidenceCitation",
        unit: Unit::Ratio,
        min: 0.0,
        max: 1.0,
        meaning: "证据引用与分析师报告的匹配度",
        evidence: "src-tauri/crates/analysis-engine/src/evidence_citation.rs:176 | src-tauri/crates/analysis-engine/src/evidence_citation.rs:198",
    },
    ConceptDecl {
        id: "axagent.calibration.score",
        name: "score",
        carrier: "CalibrationResult（市场模拟参数校准）",
        unit: Unit::Loss,
        min: 0.0,
        max: f64::INFINITY,
        meaning: "参数校准的拟合误差（**越小越好**，正常量级 ~3）；无效结果置具名哨兵 `SCORE_NO_TRADES`(999) / `SCORE_SIM_FAILED`(9999)，须用 `CalibrationResult::is_valid()` 过滤后再取最优",
        evidence: "src-tauri/crates/market-sim/src/calibration.rs:307 | src-tauri/crates/market-sim/src/calibration.rs:346",
    },
    // ─────────── 仓位族 ───────────
    ConceptDecl {
        id: "axinvest.report.position_pct",
        name: "position_pct",
        carrier: "DashboardReport",
        unit: Unit::Percent,
        min: 0.0,
        max: 100.0,
        meaning: "报告给出的建议仓位百分比",
        evidence: "src-tauri/crates/harness/src/dashboard_report.rs:59 | src-tauri/crates/harness/src/dashboard_report.rs:266",
    },
    ConceptDecl {
        id: "axinvest.pick.position_pct",
        name: "position_pct",
        carrier: "RecommenderPick",
        unit: Unit::Percent,
        min: 0.0,
        max: 100.0,
        meaning: "荐股条目给出的建议仓位百分比",
        evidence: "src-tauri/crates/analysis-engine/src/recommender/mod.rs:650",
    },
    ConceptDecl {
        id: "axinvest.risk.final_position_pct",
        name: "final_position_pct",
        carrier: "局部变量（风控闸门后）",
        unit: Unit::Percent,
        min: 0.0,
        max: 100.0,
        meaning: "经风控闸门压缩后的最终仓位百分比（单票业务上限 30）",
        evidence: "src-tauri/crates/analysis-engine/src/evidence_weight.rs:762",
    },
    ConceptDecl {
        id: "axinvest.router.min_confidence",
        name: "min_confidence",
        carrier: "RouterConfig (u8)",
        unit: Unit::Percent,
        min: 0.0,
        max: 100.0,
        meaning: "荐股筛选的最小置信阈值（对比对象是 0–100 的 RecommenderPick.confidence）",
        evidence: "src-tauri/crates/analysis-engine/src/recommender/mod.rs:313 | src-tauri/crates/analysis-engine/src/recommender/mod.rs:722",
    },
    ConceptDecl {
        id: "axinvest.opc.min_confidence",
        name: "min_confidence",
        carrier: "ValueAssessmentAgent",
        unit: Unit::Ratio,
        min: 0.0,
        max: 1.0,
        meaning: "OPC 价值评估的最低置信阈值（对比对象是 0–1 的 confidence；构造侧有 clamp(0,1) 保护）",
        evidence: "src-tauri/crates/analysis-engine/src/opc/agent.rs:217 | src-tauri/crates/analysis-engine/src/opc/agent.rs:241",
    },
    ConceptDecl {
        id: "axagent.smart_router.min_confidence",
        name: "min_confidence",
        carrier: "CostAwareRouter",
        unit: Unit::Ratio,
        min: 0.0,
        max: 1.0,
        meaning: "ML 覆盖启发式路由前的最低置信阈值（对比对象是 0–1 的 TierStats::confidence）",
        evidence: "src-tauri/src/smart_router/mod.rs:379 | src-tauri/src/smart_router/mod.rs:559",
    },
    // ─────────── 决策 / 估值族 ───────────
    ConceptDecl {
        id: "axinvest.decision.posterior",
        name: "posterior",
        carrier: "map_posterior_to_action 入参",
        unit: Unit::Ratio,
        min: 0.0,
        max: 1.0,
        meaning: "决策后验概率（阈值 0.63 / 0.53 / 0.48）",
        evidence: "src-tauri/crates/analysis-engine/src/decision.rs:58",
    },
    ConceptDecl {
        id: "axinvest.decision.effective_posterior",
        name: "effective_posterior",
        carrier: "decision.rs 派生量",
        unit: Unit::Ratio,
        min: 0.0,
        max: 1.0,
        meaning: "有效后验概率（×100 得置信度，×200 得方向强度）",
        evidence: "src-tauri/crates/analysis-engine/src/decision.rs:22",
    },
    ConceptDecl {
        id: "axinvest.portfolio.total_mv",
        name: "total_mv",
        carrier: "局部变量（组合层）",
        unit: Unit::Cny,
        min: 0.0,
        max: f64::INFINITY,
        meaning: "**组合**总市值（元），非个股总市值；仓位% = position_value / total_mv × 100",
        evidence: "src-tauri/crates/analysis-engine/src/exit_recommend.rs:128 | src-tauri/crates/analysis-engine/src/exit_recommend.rs:192",
    },
    ConceptDecl {
        id: "axinvest.dqi.dqi_score",
        name: "dqi_score",
        carrier: "data-quality.rhai 产出",
        unit: Unit::Percent,
        min: 0.0,
        max: 100.0,
        meaning: "数据质量分（分档阈值 25 / 50 / 75 / 90）",
        evidence: "src-tauri/src/commands/portfolio-mgr.rhai:693",
    },
];

/// 按表面名查全部概念（含同名多义）。
///
/// 改动某字段前先查这里：返回多于 1 条即表示**该名字是多义的**，必须按承载者判定量纲。
pub fn by_name(name: &str) -> Vec<&'static ConceptDecl> {
    CONCEPTS.iter().filter(|c| c.name == name).collect()
}

/// 按概念 ID 精确查找。
pub fn by_id(id: &str) -> Option<&'static ConceptDecl> {
    CONCEPTS.iter().find(|c| c.id == id)
}

/// 全部「一名多义」的表面名，按字典序、去重。
///
/// 返回顺序稳定（字典序），便于测试锁定与报告输出。
pub fn ambiguous_names() -> Vec<&'static str> {
    let mut counts: std::collections::BTreeMap<&'static str, usize> =
        std::collections::BTreeMap::new();
    for c in CONCEPTS {
        *counts.entry(c.name).or_insert(0) += 1;
    }
    counts.into_iter().filter(|(_, n)| *n > 1).map(|(name, _)| name).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    /// 概念 ID 必须全局唯一
    #[test]
    fn test_ids_are_unique() {
        let mut seen = HashSet::new();
        for c in CONCEPTS {
            assert!(seen.insert(c.id), "概念 ID 重复：{}", c.id);
        }
    }

    /// 每条都必须有可核对的源码锚点（格式 `<路径>:<行号>`）
    #[test]
    fn test_every_decl_cites_evidence() {
        for c in CONCEPTS {
            assert!(!c.evidence.is_empty(), "{} 缺少依据锚点", c.id);
            for anchor in c.evidence.split('|') {
                let a = anchor.trim();
                assert!(
                    a.contains(".rs:") || a.contains(".rhai:") || a.contains(".ts:"),
                    "{} 的依据锚点格式不对：{}",
                    c.id,
                    a
                );
                let (_, line) = a.rsplit_once(':').expect("锚点必须含行号");
                assert!(
                    line.split(',').all(|n| n.trim().parse::<u32>().is_ok()),
                    "{} 的行号不可解析：{}",
                    c.id,
                    line
                );
            }
        }
    }

    /// 值域必须落在量纲的规范范围内；上下界不得倒置
    #[test]
    fn test_range_within_unit_canonical() {
        for c in CONCEPTS {
            let (lo, hi) = c.unit.canonical_range();
            assert!(c.min <= c.max, "{} 值域倒置：{} > {}", c.id, c.min, c.max);
            assert!(
                c.min >= lo,
                "{} 下界 {} 低于 {} 的规范下界 {}",
                c.id,
                c.min,
                c.unit.label(),
                lo
            );
            assert!(
                c.max <= hi,
                "{} 上界 {} 高于 {} 的规范上界 {}",
                c.id,
                c.max,
                c.unit.label(),
                hi
            );
        }
    }

    /// `Unit::Ratio` 的概念必须是 0–1，`Unit::Percent` 必须是 0–100。
    ///
    /// 锁死这层映射，避免有人「顺手」把 Percent 概念写成 `max: 1.0`。
    #[test]
    fn test_ratio_and_percent_use_canonical_bounds() {
        for c in CONCEPTS {
            match c.unit {
                Unit::Ratio => {
                    assert_eq!((c.min, c.max), (0.0, 1.0), "{} 声明 Ratio 但值域不是 0–1", c.id)
                },
                Unit::Percent => assert_eq!(
                    (c.min, c.max),
                    (0.0, 100.0),
                    "{} 声明 Percent 但值域不是 0–100",
                    c.id
                ),
                Unit::Cny | Unit::Loss => {},
            }
        }
    }

    /// 多义名单锁定：**新增多义必须显式登记并更新此断言**。
    ///
    /// 这是防腐烂机制 —— 若某名字被拆成多义（或合并回单义）而本测试未同步，
    /// 说明登记动作被跳过了。
    #[test]
    fn test_ambiguous_names_locked() {
        assert_eq!(
            ambiguous_names(),
            vec!["confidence", "min_confidence", "position_pct", "score"],
            "一名多义名单发生变化：请确认新概念已登记，并同步本断言"
        );
    }

    /// `confidence` 必须同时存在 0–1 与 0–100 两种量纲（这正是本表存在的理由）
    #[test]
    fn test_confidence_is_genuinely_dual_scale() {
        let conf = by_name("confidence");
        assert!(conf.len() >= 2, "confidence 应登记多个概念，实际 {}", conf.len());

        let ratios = conf.iter().filter(|c| c.unit == Unit::Ratio).count();
        let percents = conf.iter().filter(|c| c.unit == Unit::Percent).count();
        assert!(ratios > 0, "应有 0–1 的 confidence 概念");
        assert!(percents > 0, "应有 0–100 的 confidence 概念");
    }

    /// 同名概念的消歧依据必须互不相同，否则等于没消歧
    #[test]
    fn test_same_name_carriers_are_distinguishable() {
        for name in ambiguous_names() {
            let mut carriers = HashSet::new();
            for c in by_name(name) {
                assert!(
                    carriers.insert(c.carrier),
                    "表面名 `{}` 的两个概念共用承载者 `{}` ⇒ 无法消歧",
                    name,
                    c.carrier
                );
            }
        }
    }

    /// 查询接口的负向对照（防止过滤器写反导致「查谁都命中」）
    #[test]
    fn test_lookup_negative_control() {
        assert!(by_id("axinvest.report.confidence").is_some());
        assert!(by_id("不存在的概念").is_none());
        assert!(by_name("不存在的名字").is_empty());
        assert!(!CONCEPTS.is_empty(), "登记表为空 ⇒ 所有查询都会静默返回空");
    }

    // ── 量纲化标量（Ratio01 / Percent100）：钳制、判定口径、换算、serde 形状 ──

    /// 越界与 `NaN` 必须被收进规范值域，不得静默穿过
    #[test]
    fn test_newtype_clamps_out_of_range_and_nan() {
        assert_eq!(Ratio01::new(85.0).get(), 1.0, "比率越上界须钳制");
        assert_eq!(Ratio01::new(-0.2).get(), 0.0, "比率越下界须钳制");
        assert_eq!(Ratio01::new(f64::NAN).get(), 0.0, "NaN 须归 0（不得沿链路传播）");
        assert_eq!(Percent100::new(450.0).get(), 100.0, "百分数越上界须钳制");
        assert_eq!(Percent100::new(f64::INFINITY).get(), 100.0);
        assert_eq!(Ratio01::new(0.0), Ratio01::ZERO);
        assert_eq!(Percent100::new(100.0), Percent100::MAX);
    }

    /// `is_in_range` 判**原始值** —— 钳制不得掩盖越界事实（审计口径）
    #[test]
    fn test_is_in_range_is_about_raw_value() {
        assert!(Ratio01::is_in_range(0.0) && Ratio01::is_in_range(1.0));
        assert!(!Ratio01::is_in_range(1.0001) && !Ratio01::is_in_range(-0.0001));
        assert!(!Ratio01::is_in_range(f64::NAN));
        // 30.7 正是 `evidence_citation` 那起事故的真实上界：对比率非法，对百分数合法
        assert!(!Ratio01::is_in_range(30.7), "30.7 不是合法比率");
        assert!(Percent100::is_in_range(30.7), "但它是合法百分数");
    }

    /// 跨单位换算必须自洽，且「方向写反」要能被察觉
    #[test]
    fn test_unit_conversion_round_trip() {
        let r = Ratio01::new(0.85);
        assert_eq!(Percent100::from(r).get(), 85.0);
        assert_eq!(Ratio01::from(Percent100::from(r)).get(), 0.85);
        assert_eq!(Ratio01::from(Percent100::new(85.0)).get(), 0.85);
        // 少了这一次显式换算（把 85 直接当比率）会得到钳制值 1.0 —— 与 0.85 不等
        assert_ne!(Ratio01::new(85.0).get(), 0.85);
    }

    /// serde 形状必须仍是裸数字 —— 否则前端 JSON 契约被无声改变。
    ///
    /// ⚠ **后两条断言是「设计缺陷」的回归锁**：`#[serde(transparent)]` 会让**派生的**
    /// `Deserialize` 直连内层 `f64`、**绕过 `new`** ⇒ 越界值从 JSON 长驱直入。
    /// 首次实现即踩中（`from_str("450.0")` 得到 `450.0` 而非 `100.0`）。
    #[test]
    fn test_newtype_serde_shape_unchanged() {
        assert_eq!(serde_json::to_string(&Ratio01::new(0.85)).expect("序列化失败"), "0.85");
        let back: Ratio01 = serde_json::from_str("0.85").expect("反序列化失败");
        assert_eq!(back.get(), 0.85);
        // 钳制后的值序列化出去也是规范值
        assert_eq!(serde_json::to_string(&Ratio01::new(450.0)).expect("序列化失败"), "1.0");

        // 越界输入经**反序列化**后同样被钳制 —— 不能靠"上游保证"，更不能绕过构造函数
        let over: Percent100 = serde_json::from_str("450.0").expect("反序列化失败");
        assert_eq!(over.get(), 100.0, "反序列化必须经过 new()，否则钳制在入口处失效");
        let neg: Ratio01 = serde_json::from_str("-1.5").expect("反序列化失败");
        assert_eq!(neg.get(), 0.0);
    }
}
