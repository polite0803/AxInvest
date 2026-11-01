// SPDX-License-Identifier: AGPL-3.0-only

use crate::reasoning_state::ReasoningContext;
use crate::thought_chain::ThoughtChain;

/// 目标评估结果
#[derive(Debug, Clone)]
pub struct GoalEvaluation {
    /// 目标是否已达成。
    ///
    /// **语义约束**：本字段只表示「真实判定结果」。因连续未达上限而放弃继续
    /// 判定时，它**保持** `false`，放行意图由 [`Self::forced_synthesis`] 表达
    /// （2026-09-12 修复，见该字段文档）。
    pub achieved: bool,
    /// 置信度 0.0-1.0
    pub confidence: f32,
    /// 评估理由
    pub reason: String,
    /// 缺失的子目标
    pub missing: Vec<String>,
    /// 是否因「连续未达成次数超限」而**放弃继续判定并放行**。
    ///
    /// # 为什么与 `achieved` 分开
    ///
    /// 修复前该场景直接返回 `achieved: true`（理由文案写的是「强制进入综合
    /// 阶段」），把「放弃判定」**伪装成「已达成」**。后果：任何连续 N 轮未达
    /// 成的任务，此后每一轮都恒定输出 `achieved=true` ⇒ 该信号在长任务后段
    /// **完全失去判别力**，而下游会把它当真实达成信号消费
    /// （铁律 #5：同向恒定 = 偏置非证据）。
    ///
    /// 现在两个语义各有出口：
    /// - `achieved` —— 目标到底达成没有（可能是 `false`）
    /// - `forced_synthesis` —— 判定是否已被放弃（保护性放行）
    ///
    /// 消费方必须**同时**检查两者：`!achieved && !forced_synthesis` 才回退继续
    /// 推理；`forced_synthesis` 为真时应放行进综合阶段并自行标注「未确认达成」。
    pub forced_synthesis: bool,
}

/// 目标达成评估器
///
/// 在进入 Synthesizing 阶段前评估：
/// 1. 快速检查：sub_goals 是否有对应的已完成步骤
/// 2. 基本指标：验证步骤数、失败步骤数、观察结果
pub struct GoalEvaluator {
    /// 连续未达成次数（防无限重试）
    consecutive_not_achieved: usize,
    /// 最大允许的连续未达成次数（超过后强制进入 Synthesizing）
    max_not_achieved: usize,
}

impl GoalEvaluator {
    pub fn new(max_not_achieved: usize) -> Self {
        Self { consecutive_not_achieved: 0, max_not_achieved }
    }

    /// 评估目标是否已达成
    ///
    /// 返回 `GoalEvaluation`，调用方根据结果决定是否进入 Synthesizing。
    pub fn evaluate(&mut self, chain: &ThoughtChain, context: &ReasoningContext) -> GoalEvaluation {
        let total_steps = chain.steps.len();
        let _verified_steps = chain.steps.iter().filter(|s| s.is_verified).count();
        let failed_steps = chain
            .steps
            .iter()
            .filter(|s| {
                s.is_verified
                    && s.observation.as_deref().map(|o| o.contains("Error")).unwrap_or(false)
            })
            .count();
        let completed_steps =
            chain.steps.iter().filter(|s| s.is_verified && s.observation.is_some()).count();

        // 检查 sub_goals 覆盖率
        let sub_goals = &context.sub_goals;
        let missing_goals: Vec<String> = if !sub_goals.is_empty() {
            sub_goals
                .iter()
                .filter(|goal| {
                    !chain.steps.iter().any(|s| {
                        s.is_verified && s.reasoning.to_lowercase().contains(&goal.to_lowercase())
                    })
                })
                .cloned()
                .collect()
        } else {
            Vec::new()
        };

        let goal_coverage = if sub_goals.is_empty() {
            1.0
        } else {
            1.0 - (missing_goals.len() as f32 / sub_goals.len() as f32)
        };

        // 综合判断
        let has_completed_steps = completed_steps > 0;
        let failure_ratio = if total_steps > 0 {
            failed_steps as f32 / total_steps as f32
        } else {
            0.0
        };
        let no_sub_goals = sub_goals.is_empty();

        // 安全检查：连续多次未达成 ⇒ 放弃继续判定并放行，防止无限重试。
        //
        // 修复（2026-09-12）：此前此处返回 `achieved: true`，把「放弃判定」
        // 伪装成「已达成」。凡连续 N 轮未达成的任务，此后每轮都恒定
        // `achieved=true` ⇒ 信号在长任务后段完全失去判别力，而下游当真实信号
        // 消费（铁律 #5：同向恒定 = 偏置非证据）。
        // 现改为 `achieved` 保持 false + `forced_synthesis: true`，分离两个语义。
        if self.consecutive_not_achieved >= self.max_not_achieved {
            return GoalEvaluation {
                achieved: false,
                confidence: 0.5,
                reason: format!(
                    "连续 {} 次评估未达成，放弃继续判定，强制进入综合阶段（达成状态未确认）",
                    self.consecutive_not_achieved
                ),
                missing: Vec::new(),
                forced_synthesis: true,
            };
        }

        if !has_completed_steps {
            self.consecutive_not_achieved += 1;
            return GoalEvaluation {
                achieved: false,
                confidence: 0.2,
                reason: "尚未完成任何已验证步骤".to_string(),
                missing: if no_sub_goals {
                    vec!["(无子目标)".to_string()]
                } else {
                    sub_goals.clone()
                },
                forced_synthesis: false,
            };
        }

        if failure_ratio > 0.5 && completed_steps < 3 {
            self.consecutive_not_achieved += 1;
            return GoalEvaluation {
                achieved: false,
                confidence: 0.3,
                reason: format!(
                    "失败率过高 ({:.0}%)，仅 {} 个步骤完成",
                    failure_ratio * 100.0,
                    completed_steps
                ),
                missing: missing_goals,
                forced_synthesis: false,
            };
        }

        if goal_coverage < 0.5 && !no_sub_goals {
            self.consecutive_not_achieved += 1;
            return GoalEvaluation {
                achieved: false,
                confidence: goal_coverage,
                reason: format!(
                    "子目标覆盖率仅 {:.0}%，缺失: {}",
                    goal_coverage * 100.0,
                    missing_goals.join(", ")
                ),
                missing: missing_goals,
                forced_synthesis: false,
            };
        }

        self.consecutive_not_achieved = 0;
        GoalEvaluation {
            achieved: true,
            confidence: goal_coverage.min(0.95),
            reason: format!(
                "目标基本达成: {} 个已验证步骤, {} 个子目标已完成",
                completed_steps,
                sub_goals.len().saturating_sub(missing_goals.len())
            ),
            missing: missing_goals,
            forced_synthesis: false,
        }
    }

    /// 重置计数
    pub fn reset(&mut self) {
        self.consecutive_not_achieved = 0;
    }
}

impl Default for GoalEvaluator {
    fn default() -> Self {
        Self::new(3)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reasoning_state::{ReasoningContext, ReasoningState};
    use crate::thought_chain::{ThoughtChain, ThoughtStep};

    #[test]
    fn test_empty_chain_not_achieved() {
        let chain = ThoughtChain::new();
        let context = ReasoningContext::new("test goal");
        let mut evaluator = GoalEvaluator::new(3);

        let result = evaluator.evaluate(&chain, &context);
        assert!(!result.achieved);
    }

    #[test]
    fn test_verified_steps_achieved() {
        let mut chain = ThoughtChain::new();
        let mut step = ThoughtStep::new(ReasoningState::Acting, "did something".to_string());
        step.is_verified = true;
        step.observation = Some("success".to_string());
        chain.add_step(step);

        let context = ReasoningContext::new("test");
        let mut evaluator = GoalEvaluator::new(3);

        let result = evaluator.evaluate(&chain, &context);
        assert!(result.achieved);
    }

    /// 超限放行**不等于**达成 —— P1-A 核心回归断言。
    ///
    /// 修复前此处断言的是 `assert!(r2.achieved)`，即把「放弃判定」伪装成
    /// 「已达成」并被测试**固化**。修复后 `achieved` 必须保持 `false`，
    /// 放行意图由 `forced_synthesis` 表达。
    ///
    /// 两个断言缺一不可：只断言 `forced_synthesis` 会漏掉「又偷偷把 achieved
    /// 写成 true」这种回退，而 `achieved` 才是下游的最终决策字段
    /// （铁律 #5：修 bug ≠ 修结论，必须核对最终决策字段）。
    #[test]
    fn test_consecutive_not_achieved_force_through() {
        let chain = ThoughtChain::new();
        let context = ReasoningContext::new("impossible goal");
        let mut evaluator = GoalEvaluator::new(1);

        // 第一次 — 未达成
        let r1 = evaluator.evaluate(&chain, &context);
        assert!(!r1.achieved);
        assert!(!r1.forced_synthesis, "首次未达成不应触发放弃判定");

        // 第二次 — consecutive=1 >= max=1 ⇒ 放弃判定并放行
        let r2 = evaluator.evaluate(&chain, &context);
        assert!(!r2.achieved, "放弃判定不得伪装成已达成");
        assert!(r2.forced_synthesis, "必须用独立字段表达放行原因");
        assert!(r2.confidence < 0.6);
    }

    /// 铁律 #5 回归：「同向恒定 = 偏置非证据」。
    ///
    /// 空 chain + `max_not_achieved=3` ⇒ 前 3 轮 consec 递增，第 4 轮起进入
    /// 放弃判定分支并**持续**放行。断言无论放行多少轮，`achieved` 都不得变为
    /// `true` —— 即该信号不因「轮次多」而失去判别力。
    #[test]
    fn forced_synthesis_never_reports_achieved() {
        let chain = ThoughtChain::new();
        let context = ReasoningContext::new("impossible goal");
        let mut evaluator = GoalEvaluator::new(3);

        let mut forced_rounds = 0;
        for round in 1..=10 {
            let r = evaluator.evaluate(&chain, &context);
            if r.forced_synthesis {
                forced_rounds += 1;
                assert!(!r.achieved, "第 {round} 轮：forced_synthesis 时 achieved 必须为 false");
            }
        }
        // 第 4..=10 轮放行（前 3 轮 consec 尚未达阈值）—— 同时也证明该分支
        // 确实被走到，避免「分支永不触发所以断言恒真」的假绿。
        assert_eq!(forced_rounds, 7, "max_not_achieved=3 ⇒ 第 4..=10 轮放行");
    }

    #[test]
    fn test_reset() {
        let mut evaluator = GoalEvaluator::new(2);
        let chain = ThoughtChain::new();
        let context = ReasoningContext::new("test");

        evaluator.evaluate(&chain, &context);
        evaluator.reset();
        // reset 后计数归零
        let result = evaluator.evaluate(&chain, &context);
        assert!(!result.achieved); // 不是强制通过
        assert!(!result.forced_synthesis, "reset 后不应仍处于放弃判定状态");
    }
}
