// SPDX-License-Identifier: AGPL-3.0-only

//! 反思结论的**落地层级**权威轴（PLAN-mover-coverage 阶段 3 / 决-3）。
//!
//! ## 本仓有三条互不相同的归因轴，混用即错
//!
//! | 轴 | 权威源 | 谁填 | 回答的问题 |
//! |---|---|---|---|
//! | [`crate::failure_mode::FailureMode`] | `failure_mode.rs` | LLM 从受控词表选 | **错成什么样**（追涨 / 大盘替个股背书 / 题材没进证据面…） |
//! | [`crate::mover_recall::MissLayer`] | `mover_recall.rs` | 系统从漏检事件**机械盖章** | **漏在哪一层**（没进池 / 评分截断 / 档位错配…） |
//! | [`ReflectionTier`]（本模块） | 本文件 | LLM 自报、Rust 白名单过滤 | **要改哪一处**才能不再错（参数 / 公式 / 提示词） |
//!
//! 阶段 3 立项时曾打算新造一个 `miss_layer` 五值枚举
//! （`data_gap`/`no_factor`/`gate_shape`/`pool_coverage`/`parameter`），落笔前查重发现
//! 它的每一格都已有主：`data_gap` 属 `evolution_proposal` 的数据可达性分桶（P4 硬规则），
//! `pool_coverage`/`gate_shape` 属 `MissLayer` 与失败模式词表，`parameter` 正是本轴的 L1。
//! 再造一套 = 第四份手抄清单（禁区 12），故**扩既有词表 + 把已声明却没落地的本轴接上**。
//!
//! ## 为什么必须落地成列而不是留在 `decision_json` 里
//!
//! 现网实测（80 条 completed 反思）：模板**早就要求** `implementation_tier`，但
//! JSON 路径 `reflection.implementation_tier` 只有 **1 条**有值，而面板的缺省写死 `"L1"`
//! —— 于是 79 条「模型压根没判」被伪装成「判成了参数层」。JSON 大 blob 里的键：
//! ① 无法用 SQL 计数（`like '%implementation_tier%'` 会命中提示词原文自身，实测 80/80
//! 全「命中」，是假读数）；② 消费方各自解析、各自兜底 ⇒ 兜底口径不可能一致。
//! 因此本列由写侧过一次白名单，NULL 只代表「没判」，不再和「判成 L1」混为一谈。
//!
//! ## 三值的含义与判据（与 `reflection.md` 的表格同源）
//!
//! - **L1** 改 workflow 模板变量（阈值 / 权重）⇒ 可进 WFO 参数网格。
//! - **L2** 改 `.rhai` 决策公式或算法约束 ⇒ **禁止**进参数网格：网格只能在既有标量里
//!   挪数字，而根因是「门形状 / 腿集合」时，任何挪法都是在替一个不相干的假设调参。
//! - **L3** 改 `.md` 业务提示词 ⇒ 同 L2 禁入网格。
//!
//! ⚠ L2/L3 不落进任何执行器：`accept_evolution_plan` 会真的执行 `execute_pending_plan`
//! 并要求 `parameter_result`，把结构层提案塞进进化计划等于让它自动改图。
//! 结构层的出口是**待裁清单**（只读、无人自动应用），见
//! `crate::reflection_stats` 的结构层分桶与面板的「AI 辅助修改」通道。

use serde::{Deserialize, Serialize};

/// 反思结论要落在哪一层（值域 = 反思模板可见的全部选项）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub enum ReflectionTier {
    /// 改 workflow 模板变量（阈值 / 权重 / 先验）—— 唯一可进参数校准网格的层级
    L1,
    /// 改 `.rhai` 决策公式或算法约束（门形状、腿集合、出场规则）
    L2,
    /// 改 `.md` 业务提示词（分析师 / 辩论 / 反思模板本身）
    L3,
}

impl ReflectionTier {
    /// 值域全集（顺序 = 模板清单顺序 = 待裁清单分组顺序）。
    pub const ALL: &'static [ReflectionTier] =
        &[ReflectionTier::L1, ReflectionTier::L2, ReflectionTier::L3];

    /// 落库/IPC 用的键（与提示词里写的 `L1 | L2 | L3` 逐字一致）。
    pub fn as_str(self) -> &'static str {
        match self {
            Self::L1 => "L1",
            Self::L2 => "L2",
            Self::L3 => "L3",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        let t = s.trim();
        Self::ALL.iter().copied().find(|m| m.as_str().eq_ignore_ascii_case(t))
    }

    /// 结构层（非 L1）⇒ 该反思的根因不能靠调参数解决。
    pub fn is_structural(self) -> bool {
        self != Self::L1
    }

    /// 给反思模板的说明行（只进 prompt，不进 UI —— UI 侧按 `as_str()` 取 i18n 译名）。
    fn hint(self) -> &'static str {
        match self {
            Self::L1 => {
                "L1：改 workflow 模板变量（阈值 / 权重 / 先验），本次可给 params_suggestion"
            },
            Self::L2 => {
                "L2：改 .rhai 决策公式或算法约束（门形状 / 腿集合 / 出场规则），\
                          必须在 code_diff_proposal 写出目标文件与最小改动，params_suggestion 留空 []"
            },
            Self::L3 => {
                "L3：改 .md 业务提示词（分析师 / 辩论 / 反思模板），\
                          必须在 code_diff_proposal 写出目标 md 路径与要加的判据，params_suggestion 留空 []"
            },
        }
    }

    /// 模板用的清单文本（单一权威源：加层级不需要另改 prompt 里的选项表）。
    pub fn catalog_for_prompt() -> String {
        Self::ALL.iter().map(|m| format!("- {}", m.hint())).collect::<Vec<_>>().join("\n")
    }

    /// 必填声明行：明确「没判」要输出 null 而不是随便挑一个层级。
    pub fn absence_rule() -> &'static str {
        "判不出落在哪一层时输出 null —— 不要因为「大概是要调参」就填 L1"
    }
}

/// 把反思输出的 `implementation_tier` 变成落库值。
///
/// 口径与 `failure_modes_column_of` 同形：**非法值不落列**（返回 `None` = 「没判」），
/// 不在这里替模型兜底成 L1 —— 那正是本轴此前 79/80 条被伪装成参数层的成因。
/// 非法但非空（模型自造 `L4`、写中文「参数层」）由调用方记 warn，
/// 库里只留「没判」，避免同一形态在库里既可能是真缺席也可能是脏值。
pub fn parse_reflection_tier(raw: Option<&str>) -> Option<ReflectionTier> {
    let s = raw?.trim();
    if s.is_empty() || s.eq_ignore_ascii_case("null") || s == "无" {
        return None;
    }
    ReflectionTier::parse(s)
}

/// WFO 参数校准的入池判据（**纯函数**，阶段 3 的验收就打在它身上）。
///
/// - `Some("L1")` ⇒ 入网格；
/// - `Some("L2" | "L3")` ⇒ **排除**并计入结构层待裁数；
/// - `None` ⇒ 入网格。
///
/// `None` 放行不是宽松，是存量口径：本列引入前的 79/80 条都是 NULL，排除它们等于
/// 把参数校准的样本清零 —— 那会把「新列生效」变成「校准失效」，与修复目的相反
/// （同族先例：`stock_reflections.source` 的「其余一切值（含 bootstrap 加列后的空串）
/// 一律归 due 桶，不依赖存量回填」）。真·未标注的增量行会被写侧白名单归一为 NULL，
/// 因此本函数无法区分「新行没判」与「老行没这列」—— 面板必须另出
/// 「层级未标注」计数，不得把它当成 L1。
pub fn may_enter_param_grid(tier: Option<ReflectionTier>) -> bool {
    !tier.is_some_and(|t| t.is_structural())
}

#[cfg(test)]
mod reflection_tier_tests {
    use super::*;

    #[test]
    fn catalog_covers_every_variant_and_is_prompt_facing() {
        assert_eq!(ReflectionTier::ALL.len(), 3);
        assert_eq!(ReflectionTier::ALL[0].as_str(), "L1");
        assert_eq!(ReflectionTier::ALL[2].as_str(), "L3");
        let txt = ReflectionTier::catalog_for_prompt();
        for m in ReflectionTier::ALL {
            assert!(txt.contains(m.as_str()), "模板清单必须逐项列出 {}", m.as_str());
        }
        // 结构层必须被要求「params_suggestion 留空」—— 否则 L2/L3 的反思仍会带参数建议，
        // 而 WFO 侧的排除只是把它们扔掉，等于白产。
        assert!(txt.contains("params_suggestion 留空"));
    }

    #[test]
    fn whitelist_never_falls_back_to_a_tier() {
        assert_eq!(parse_reflection_tier(Some("L1")), Some(ReflectionTier::L1));
        assert_eq!(parse_reflection_tier(Some(" l2 ")), Some(ReflectionTier::L2));
        assert_eq!(parse_reflection_tier(Some("L3")), Some(ReflectionTier::L3));
        // 脏值 = 没判，不是 L1（旧实现把缺省当 L1，正是伪造读数的成因）
        for raw in [None, Some(""), Some("   "), Some("null"), Some("L4"), Some("参数层")] {
            assert_eq!(parse_reflection_tier(raw), None, "{raw:?} 应判成未标注");
        }
    }

    /// 阶段 3 验收（PLAN 断言的本体）：结构层反思**不得**进参数网格。
    /// 用 `try_parse_param_suggestion` 可解析的真建议当样本，证明被挡住的是**层级**，
    /// 不是「这条建议解析不出来」—— 两者在库里的表现一样，只有一处能区分。
    #[test]
    fn structural_tiers_are_excluded_from_the_param_grid_but_l1_and_absence_are_not() {
        assert!(may_enter_param_grid(Some(ReflectionTier::L1)));
        assert!(!may_enter_param_grid(Some(ReflectionTier::L2)));
        assert!(!may_enter_param_grid(Some(ReflectionTier::L3)));
        // 存量 NULL 放行（理由见函数文档：排除即清零，与修复目的相反）
        assert!(may_enter_param_grid(None));
        // 被挡掉的那条本身必须是一条**能被解析的真建议**
        let parsed = crate::portfolio_formula::try_parse_param_suggestion(
            r#"[{"param":"action_buy_threshold","current_value":0.55,"suggested_value":0.62}]"#,
        );
        assert!(parsed.is_some(), "夹具必须是一条真建议，否则「层级挡住」这条断言测的是空气");
    }

    #[test]
    fn serde_shape_is_pascal_case_for_ipc_and_snake_for_storage() {
        // 落库走 `as_str()`（"L1"），IPC 侧 serde 形态必须与 TS 消费一致
        assert_eq!(serde_json::to_string(&ReflectionTier::L2).unwrap(), "\"L2\"");
    }
}
