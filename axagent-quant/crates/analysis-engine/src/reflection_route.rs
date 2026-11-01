//! 反思的**确定性归因优先**分流（2026-10-28）。
//!
//! ## 为什么需要它
//!
//! 现网每条反思都固定一次 LLM 调用（实测 373 条里 31 条直接 429/网络失败），而其中
//! 相当一部分的结论**根本不需要模型**：止损被触发、目标价被触及——这些是行情事实，
//! 模型能做的只是把它复述成一段话，且每次复述都可能漂移。
//!
//! 本模块只回答一个问题：**这条反思能不能不调模型就定性？**
//! 能 ⇒ 返回结构化的归因（结论 + 教训 + 下次怎么办），由调用方直接落库；
//! 不能 ⇒ 返回 `NeedsLlm`，交回原来的链路。
//!
//! 它是纯函数：不读库、不取行情、不调模型。事实由调用方算好后传入 —— 这样
//! 「什么算确定性」只有一处定义，且可单测。
//!
//! ## 边界
//!
//! 只有**目标价触及 / 止损触发**这两种「价格事实」算确定性。`alpha_return` 为负
//! 但没触发任何价位，**不算** —— 那可能是大盘拖累、也可能是判断错了，需要模型结合
//! 板块与事件去分辨。宁可多调一次模型，也不要让确定性归因替模型下定性结论。

/// 归因所需的事实（全部由调用方算好）。
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct AttributionFacts {
    /// T+N 实际收益（%）
    pub raw_return_pct: Option<f64>,
    /// 相对基准的超额收益（%）
    pub alpha_return_pct: Option<f64>,
    /// 是否触发止损位
    pub stop_loss_hit: Option<bool>,
    /// 是否触及目标价
    pub target_hit: Option<bool>,
}

/// 确定性归因结论。
#[derive(Debug, Clone, PartialEq)]
pub struct DeterministicAttribution {
    /// 结论标签（落 `verdict`）
    pub verdict: &'static str,
    /// 一句话教训（落 `lesson_summary`，≤200 字符）
    pub lesson: String,
    /// 哪里判断错了（落 `what_went_wrong`；判对时为 None）
    pub what_went_wrong: Option<String>,
    /// 下次怎么办（落 `fix_for_future`）
    pub fix_for_future: String,
}

/// 分流结果。
#[derive(Debug, Clone, PartialEq)]
pub enum Route {
    /// 行情事实本身已定性，不必调模型
    Deterministic(DeterministicAttribution),
    /// 需要模型结合板块/事件判断
    NeedsLlm,
}

/// 按行情事实分流。判定顺序：目标触及 → 止损触发 → 其余交给模型。
pub fn attribute(facts: &AttributionFacts) -> Route {
    if facts.target_hit == Some(true) {
        let gain = facts.raw_return_pct.unwrap_or(0.0);
        return Route::Deterministic(DeterministicAttribution {
            verdict: "correct",
            lesson: format!("目标价被触及（实际 {gain:+.2}%），方向判断成立。"),
            what_went_wrong: None,
            fix_for_future: "保持触发目标价的入场条件，不必为单次回撤放宽止损。".to_string(),
        });
    }
    if facts.stop_loss_hit == Some(true) {
        let loss = facts.raw_return_pct.unwrap_or(0.0);
        let alpha = facts.alpha_return_pct;
        let alpha_note = match alpha {
            Some(a) => format!("，超额 {a:+.2}%"),
            None => String::new(),
        };
        return Route::Deterministic(DeterministicAttribution {
            verdict: "wrong",
            lesson: format!("止损位被触发（实际 {loss:+.2}%{alpha_note}），该笔判断被证伪。"),
            what_went_wrong: Some(format!(
                "入场后价格跌破止损位（实际 {loss:+.2}%{alpha_note}）——止损是价格事实，\
                 不需要模型归因。"
            )),
            fix_for_future: "复盘入场时的追高幅度与量能确认；若属放量突破后的正常回踩，\
                             记录该形态以免下次把正常波动当破位。"
                .to_string(),
        });
    }
    Route::NeedsLlm
}

/// 把归因渲染成一行提示词文本，供仍需调模型的分支复用（避免模型重新猜一遍发生了什麼）。
pub fn render_attribution_line(facts: &AttributionFacts) -> String {
    let mut parts = Vec::new();
    if let Some(r) = facts.raw_return_pct {
        parts.push(format!("实际收益 {r:+.2}%"));
    }
    if let Some(a) = facts.alpha_return_pct {
        parts.push(format!("相对基准超额 {a:+.2}%"));
    }
    if facts.stop_loss_hit == Some(true) {
        parts.push("已触发止损".to_string());
    }
    if facts.target_hit == Some(true) {
        parts.push("已触及目标价".to_string());
    }
    if parts.is_empty() {
        return "（行情事实不全，按缺失处理，不要臆造收益数字）".to_string();
    }
    format!("【确定性事实】{}", parts.join("；"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn target_hit_short_circuits_with_correct_verdict() {
        let r = attribute(&AttributionFacts {
            raw_return_pct: Some(8.5),
            target_hit: Some(true),
            ..Default::default()
        });
        match r {
            Route::Deterministic(d) => {
                assert_eq!(d.verdict, "correct");
                assert!(d.lesson.contains("+8.50%"), "{}", d.lesson);
                assert!(d.what_went_wrong.is_none());
            },
            Route::NeedsLlm => panic!("目标价触及必须走确定性"),
        }
    }

    /// 止损优先于「alpha 为正」：价格事实 > 超额解释。
    #[test]
    fn stop_loss_wins_over_positive_alpha() {
        let r = attribute(&AttributionFacts {
            raw_return_pct: Some(-6.0),
            alpha_return_pct: Some(2.0),
            stop_loss_hit: Some(true),
            ..Default::default()
        });
        match r {
            Route::Deterministic(d) => {
                assert_eq!(d.verdict, "wrong");
                assert!(d.what_went_wrong.is_some());
            },
            Route::NeedsLlm => panic!("止损触发必须走确定性"),
        }
    }

    /// 负超额但两个价位都没触发 ⇒ **不得**替模型下结论，必须回 LLM。
    #[test]
    fn negative_alpha_without_price_events_needs_llm() {
        assert_eq!(
            attribute(&AttributionFacts {
                raw_return_pct: Some(-3.0),
                alpha_return_pct: Some(-2.5),
                ..Default::default()
            }),
            Route::NeedsLlm
        );
    }

    /// 事实缺失（None）不得被当成 false 之外的东西 —— 仍然回 LLM。
    #[test]
    fn missing_facts_need_llm() {
        assert_eq!(attribute(&AttributionFacts::default()), Route::NeedsLlm);
    }

    #[test]
    fn attribution_line_states_only_known_facts() {
        let line = render_attribution_line(&AttributionFacts {
            raw_return_pct: Some(-1.235),
            ..Default::default()
        });
        assert!(line.contains("-1.24%"), "{line}");
        assert!(!line.contains("止损"), "{line}");
        assert!(render_attribution_line(&AttributionFacts::default()).contains("不全"));
    }
}
