// SPDX-License-Identifier: AGPL-3.0-only

//! 决策失败模式的结构化归因（PLAN P2-b）。
//!
//! 为什么必须是受控词表而不是自由文本：`what_went_wrong` 是 LLM 叙述，跨样本无法聚合，
//! 「本月到底错在哪一类」只能靠人读。本次紫金误判识别出的四类成因（周期顶部 DCF 乐观 /
//! 大阳线追涨 / 大盘情绪替个股背书 / 反向 DCF 被无视）因此被写成本枚举，反思模板只能从
//! 清单里选，Rust 侧再按白名单过滤 —— 看板才有余量给占比与月度复盘的分组依据。
//!
//! 与 `miss_layer` 的分界：后者是**漏检链**的机械归因层（由系统从事件盖章，LLM 不得自报），
//! 本枚举是**反思判定**。两者语义不同域，不得互相顶替或并列为可比计数。
//!
//! ## 2026-10-19 扩四类（PLAN-mover-coverage 阶段 3 / 决-3）
//!
//! 扩表前现网实测：80 条 completed 反思里只有 32 条带标注，其中 **23 条单独就是
//! `["other"]`** —— 兜底值成了主值，看板的失败模式格等于没有信息。逐条读那 23 条的
//! `what_went_wrong` 可以归出四个反复出现、而旧四类都装不下的成因（题材没进证据面 /
//! 逆势相对强度没参与 / 门形态拦住正确方向 / 标的没进候选宇宙），且每一类都在
//! 002696@2026-09-28 那个样本上有实据。因此**扩词表**而不是放宽 `other` 的门槛：
//! `other` 退回它本该有的位置（真兜底），其占比即词表覆盖度的可观测指标。

use serde::{Deserialize, Serialize};
use std::collections::HashSet;

/// 决策失败模式（受控词表，值域即反思模板可见的全部选项）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FailureMode {
    /// 周期顶部用 DCF 给出乐观目标价（成长外推取高点、折价不足）
    DcfOptimisticAtCycleTop,
    /// 大阳线当日追涨（涨幅与成交额同时处于历史高分位仍出开仓动作）
    ChaseBigUpBar,
    /// 用大盘情绪替个股背书（市场级 regime / 涨停家数被当作个股多头证据）
    MarketRegimeEndorsement,
    /// 反向 DCF 被无视（估值结论与最终动作方向相反却未处置）
    ReverseDcfIgnored,
    /// 题材/热点轮动没进证据面 —— 驱动是所属题材与连板梯队，而本档没有任何腿能读到它。
    /// 实证：as-of 2026-10-18 复盘的 002696@09-28，12 交易日 +65.3% 的驱动是水产/鱼粉题材，
    /// 全仓当时**零通道**（`industry_chain.rs` 无农业链、无商品价）。
    ThemeRotationMissed,
    /// 逆势抗跌的相对强度被忽略 —— 同期基准下跌而个股跌得少甚至上涨，这个量没参与判定。
    /// 实证：同一样本 09-28 大盘 −7.3% 而该股 +2.25%（超额），当时四档无一条腿读它。
    CounterTrendStrengthIgnored,
    /// 进门形态把正确方向拦在门外 —— 方向判断本身没错，错在门只接受「已在σ带内 / 已涨停」
    /// 这类**已经发生**的形状，埋伏型（未涨但相对强度刚起）天然进不去。
    /// 实证：002696 超短档 `sealRate=0.75` 广度达标，却因「近 5 日未涨停」被进门门否掉。
    EntryGateShapeBlocked,
    /// 标的根本没进候选宇宙 —— 荐股/扫描的候选池来源退化或覆盖不足，连被判定一次的机会都没有。
    /// 与 [`crate::mover_recall::MissLayer::NotInPool`] 的分工：那一层由系统从漏检事件机械盖章，
    /// 本层是**反思侧承认这次错在这里**，两者计数域不同，面板不得并列成可比数字。
    CandidatePoolCoverageMissed,
    /// 不属于其余八类：占比走高即说明词表覆盖不足，必须上看板而不是被折算进别类
    Other,
}

impl FailureMode {
    /// 词表全集（顺序 = 模板清单顺序 = 看板单元格顺序）。
    pub const ALL: &'static [FailureMode] = &[
        FailureMode::DcfOptimisticAtCycleTop,
        FailureMode::ChaseBigUpBar,
        FailureMode::MarketRegimeEndorsement,
        FailureMode::ReverseDcfIgnored,
        FailureMode::ThemeRotationMissed,
        FailureMode::CounterTrendStrengthIgnored,
        FailureMode::EntryGateShapeBlocked,
        FailureMode::CandidatePoolCoverageMissed,
        FailureMode::Other,
    ];

    /// 落库/IPC 用的 snake_case 键。
    pub fn as_str(self) -> &'static str {
        match self {
            Self::DcfOptimisticAtCycleTop => "dcf_optimistic_at_cycle_top",
            Self::ChaseBigUpBar => "chase_big_up_bar",
            Self::MarketRegimeEndorsement => "market_regime_endorsement",
            Self::ReverseDcfIgnored => "reverse_dcf_ignored",
            Self::ThemeRotationMissed => "theme_rotation_missed",
            Self::CounterTrendStrengthIgnored => "counter_trend_strength_ignored",
            Self::EntryGateShapeBlocked => "entry_gate_shape_blocked",
            Self::CandidatePoolCoverageMissed => "candidate_pool_coverage_missed",
            Self::Other => "other",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|m| m.as_str() == s.trim())
    }

    /// 给反思模板的说明行（只进 prompt，不进 UI —— UI 侧走 i18n 按 `as_str()` 取译名）。
    fn hint(self) -> &'static str {
        match self {
            Self::DcfOptimisticAtCycleTop => {
                "周期顶部用 DCF 给出乐观目标价（成长外推取了周期高点、折价/终止值不足）"
            },
            Self::ChaseBigUpBar => "大阳线追涨（当日涨幅与成交额同时处于历史高分位仍出开仓动作）",
            Self::MarketRegimeEndorsement => {
                "大盘情绪替个股背书（用市场级 regime、涨停家数或封板率替代个股自身的多头证据）"
            },
            Self::ReverseDcfIgnored => {
                "反向 DCF 被无视（估值/DCF 结论与最终动作方向相反，却未据此降动作或降置信）"
            },
            Self::ThemeRotationMissed => {
                "题材/热点轮动未进证据面（驱动是所属题材与连板梯队，而本档没有任何腿能读到它）"
            },
            Self::CounterTrendStrengthIgnored => {
                "逆势抗跌的相对强度被忽略（同期基准下跌而个股跌得少甚至上涨，该量未参与判定）"
            },
            Self::EntryGateShapeBlocked => {
                "进门形态拦住正确方向（方向本身没判错，但门只接受「已在σ带内 / 已涨停」这类已发生的形状）"
            },
            Self::CandidatePoolCoverageMissed => {
                "标的未进候选宇宙（荐股/扫描候选池来源退化或覆盖不足，连被判定一次的机会都没有）"
            },
            Self::Other => "以上八类都不是 —— 选它必须同时在 what_went_wrong 里给新问题命名（词表按该名字扩类）；只写 other 等于没有归因",
        }
    }

    /// 反思模板用的清单文本（单一权威源：改词表不需要另改 prompt 里的选项表）。
    pub fn catalog_for_prompt() -> String {
        Self::ALL
            .iter()
            .map(|m| format!("- {}：{}", m.as_str(), m.hint()))
            .collect::<Vec<_>>()
            .join("\n")
    }
}

/// 白名单化的产出：命中词表的部分 + 被丢弃的自造 token。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ParsedFailureModes {
    pub modes: Vec<FailureMode>,
    pub rejected: Vec<String>,
}

impl ParsedFailureModes {
    /// 落库形态：JSON 数组文本（与 `missed_signals` 同族）；一个都没命中时落 `[]` 而非 NULL，
    /// 让「反思产出了但全是非法 token」与「模板根本没产出该字段」在库里可区分。
    pub fn to_json_text(&self) -> String {
        let keys: Vec<&str> = self.modes.iter().map(|m| m.as_str()).collect();
        serde_json::to_string(&keys).unwrap_or_else(|_| "[]".to_string())
    }
}

/// 把反思输出里的 `failure_modes` 字段白名单化。
///
/// 输入取自 `deep_get_reflection_text`，实测有三种形态（数组 / 裸串 / 双重编码的 JSON 字符串），
/// 三层都要容错 —— 只认数组会让历史 10/11 的双重编码形态静默变成「未标注」。
pub fn parse_failure_modes(raw: &str) -> ParsedFailureModes {
    let candidates: Vec<String> =
        if let Ok(arr) = serde_json::from_str::<Vec<serde_json::Value>>(raw) {
            arr.iter().filter_map(|v| v.as_str().map(str::to_string)).collect()
        } else if let Ok(one) = serde_json::from_str::<String>(raw) {
            vec![one]
        } else {
            vec![raw.trim().to_string()]
        };

    let mut seen: HashSet<FailureMode> = HashSet::new();
    let mut rejected: Vec<String> = Vec::new();
    for token in candidates {
        match FailureMode::parse(&token) {
            Some(m) => {
                seen.insert(m);
            },
            None => {
                if !token.trim().is_empty() && !rejected.iter().any(|r| r == &token) {
                    rejected.push(token);
                }
            },
        }
    }
    // 按词表顺序输出，落库与看板都不受模型吐字顺序影响
    let modes = FailureMode::ALL.iter().copied().filter(|m| seen.contains(m)).collect::<Vec<_>>();
    ParsedFailureModes { modes, rejected }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FailureModeCell {
    pub mode: String,
    pub count: usize,
    /// 占**已标注反思行**的比例。一行可命中多个模式 ⇒ 各格占比之和可 >100%，
    /// 面板文案必须按「占已标注样本」表述，不得写成「占总反思」。
    pub share: f64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FailureModeBoard {
    /// 参与统计的反思行（completed 行全集）
    pub total_rows: usize,
    /// `failure_modes` 非空且命中词表的行数
    pub labelled: usize,
    /// 模板未产出该字段的行数（结构性缺口，面板必须显式给出，不得折算进 other）
    pub unlabelled: usize,
    pub cells: Vec<FailureModeCell>,
    /// 模型自造、不在词表内的 token（显式呈现，供裁定是否扩词表）
    pub rejected_tokens: Vec<String>,
}

/// 聚合看板。入参 = 每条 completed 反思行的 `failure_modes` 列原文（可为 NULL）。
pub fn build_failure_mode_board(rows: &[Option<String>]) -> FailureModeBoard {
    let mut counts: Vec<usize> = vec![0; FailureMode::ALL.len()];
    let mut rejected: Vec<String> = Vec::new();
    let mut labelled = 0usize;
    for row in rows {
        let Some(text) = row.as_deref() else { continue };
        let parsed = parse_failure_modes(text);
        // 词表外的自造归因逐条并入（同一 token 只列一次）—— 先收 rejected 再判 modes，
        // 否则「modes 为空」这条分支会提前 continue 把 rejected 一起吞掉。
        for token in parsed.rejected {
            if !rejected.contains(&token) {
                rejected.push(token);
            }
        }
        if parsed.modes.is_empty() {
            // 空数组 / 全非法 token ⇒ 该条没有任何可用标注
            continue;
        }
        labelled += 1;
        for m in parsed.modes {
            counts[FailureMode::ALL.iter().position(|x| *x == m).unwrap_or(0)] += 1;
        }
    }
    let mut cells: Vec<FailureModeCell> = FailureMode::ALL
        .iter()
        .zip(counts.iter())
        .filter(|(_, c)| **c > 0)
        .map(|(m, c)| FailureModeCell {
            mode: m.as_str().to_string(),
            count: *c,
            share: if labelled == 0 {
                0.0
            } else {
                *c as f64 / labelled as f64
            },
        })
        .collect();
    // 计数降序（同数保持词表序）：看板第一格即最高频失败模式
    cells.sort_by_key(|c| std::cmp::Reverse(c.count));
    FailureModeBoard {
        total_rows: rows.len(),
        labelled,
        unlabelled: rows.len() - labelled,
        cells,
        rejected_tokens: rejected,
    }
}

#[cfg(test)]
mod failure_mode_tests {
    use super::*;

    #[test]
    fn catalog_covers_every_variant_and_keeps_other_last() {
        // 9 = 旧四类 + 2026-10-19 扩四类 + other
        assert_eq!(FailureMode::ALL.len(), 9);
        assert_eq!(FailureMode::ALL[0].as_str(), "dcf_optimistic_at_cycle_top");
        // `other` 必须排在最后：它进 prompt 清单的位置就是「以上都不是」，
        // 插到中间会让模型把兜底值当普通选项多选（实测 23/32 就是这么来的）。
        assert_eq!(FailureMode::ALL[FailureMode::ALL.len() - 1].as_str(), "other");
        let txt = FailureMode::catalog_for_prompt();
        for m in FailureMode::ALL {
            assert!(txt.contains(m.as_str()), "模板清单必须逐项列出 {}", m.as_str());
        }
        // 兜底项的说明必须带「给新问题命名」的要求，否则 other 仍是免答通道
        assert!(txt.contains("给新问题命名"), "other 的提示语必须要求具名: {txt}");
    }

    /// 2026-10-19 扩的四类：必须**逐名可解析**且顺序稳定（看板单元格顺序 = 词表顺序）。
    #[test]
    fn the_four_added_modes_are_parseable_and_ordered() {
        let added = [
            "theme_rotation_missed",
            "counter_trend_strength_ignored",
            "entry_gate_shape_blocked",
            "candidate_pool_coverage_missed",
        ];
        for s in added {
            let m = FailureMode::parse(s).unwrap_or_else(|| panic!("{s} 未进值域"));
            assert_eq!(m.as_str(), s);
        }
        // 反手抄：新增四类必须排在旧四类之后、other 之前（顺序变了 ⇒ 看板列序与历史读数不可比）
        let idx = |s: &str| {
            FailureMode::ALL
                .iter()
                .position(|m| m.as_str() == s)
                .unwrap_or_else(|| panic!("{s} 不在 ALL"))
        };
        assert!(idx("reverse_dcf_ignored") < idx("theme_rotation_missed"));
        assert!(idx("candidate_pool_coverage_missed") < idx("other"));
    }

    #[test]
    fn parses_json_array_and_rejects_tokens_outside_vocabulary() {
        let p = parse_failure_modes(r#"["chase_big_up_bar","valuation_was_fine"]"#);
        assert_eq!(p.modes, vec![FailureMode::ChaseBigUpBar]);
        assert_eq!(p.rejected, vec!["valuation_was_fine".to_string()]);
    }

    #[test]
    fn parses_bare_string_and_double_encoded_json_string() {
        assert_eq!(
            parse_failure_modes("reverse_dcf_ignored").modes,
            vec![FailureMode::ReverseDcfIgnored]
        );
        assert_eq!(
            parse_failure_modes("\"market_regime_endorsement\"").modes,
            vec![FailureMode::MarketRegimeEndorsement]
        );
    }

    #[test]
    fn dedupes_and_orders_by_catalog_not_by_model_output() {
        let p = parse_failure_modes(
            r#"["other","chase_big_up_bar","other","dcf_optimistic_at_cycle_top"]"#,
        );
        assert_eq!(
            p.modes,
            vec![
                FailureMode::DcfOptimisticAtCycleTop,
                FailureMode::ChaseBigUpBar,
                FailureMode::Other
            ],
            "落库顺序必须按词表序，否则同一批输出会因模型吐字顺序不同而不可比对"
        );
    }

    #[test]
    fn empty_vocabulary_hit_still_serialises_to_empty_array_not_null() {
        let p = parse_failure_modes(r#"["invented_mode"]"#);
        assert_eq!(p.to_json_text(), "[]");
    }

    #[test]
    fn board_splits_labelled_from_unlabelled() {
        let rows = vec![
            Some(r#"["chase_big_up_bar"]"#.to_string()),
            Some(r#"["chase_big_up_bar","market_regime_endorsement"]"#.to_string()),
            Some("[]".to_string()),
            None,
        ];
        let b = build_failure_mode_board(&rows);
        assert_eq!(b.total_rows, 4);
        assert_eq!(b.labelled, 2, "空数组与 NULL 都不得算已标注");
        assert_eq!(b.unlabelled, 2);
        let chase =
            b.cells.iter().find(|c| c.mode == "chase_big_up_bar").expect("追涨模式应有计数");
        assert_eq!(chase.count, 2);
        assert!((chase.share - 1.0).abs() < 1e-9, "分母是已标注的 2 条，实得 {}", chase.share);
    }

    #[test]
    fn board_with_no_labelled_row_has_no_cells_and_zero_share() {
        let rows = vec![None, Some("[]".to_string())];
        let b = build_failure_mode_board(&rows);
        assert!(b.cells.is_empty());
        assert_eq!(b.labelled, 0);
        assert_eq!(b.unlabelled, 2);
    }

    #[test]
    fn board_surfaces_rejected_tokens_once() {
        let rows = vec![
            Some(r#"["sentiment_over_signal"]"#.to_string()),
            Some(r#"["sentiment_over_signal"]"#.to_string()),
            Some(r#"["chase_big_up_bar","another_invented"]"#.to_string()),
        ];
        let b = build_failure_mode_board(&rows);
        assert_eq!(b.labelled, 1);
        assert!(
            b.rejected_tokens.contains(&"sentiment_over_signal".to_string()),
            "词表外的自造归因必须上看板，不得静默丢弃: {:?}",
            b.rejected_tokens
        );
        assert_eq!(
            b.rejected_tokens.iter().filter(|t| *t == "sentiment_over_signal").count(),
            1,
            "同一 token 反复出现只列一次"
        );
    }
}
