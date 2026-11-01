// SPDX-License-Identifier: AGPL-3.0-only

//! 上下文分量预算的**唯一供值入口**。
//!
//! ## 为什么需要它
//!
//! 分量预算（system prompt / working memory / retrieved memories / skills / nudges）
//! 原先是 `src-tauri/src/context_manager.rs` 里的**绝对值常量**（8000 / 800 / 10000 / 5000 / 2000），
//! 与模型的真实窗口**解耦**：32k 窗口的本地模型和 200k 窗口的云端模型拿到同一份预算，
//! 小窗口下五个分量之和（25 800）几乎吃掉整个窗口，留给对话历史的额度只剩 4 030 token。
//!
//! 现在改为「**比例 + 上限**」双约束：`min(ratio × window, cap)`，比例以
//! [`REFERENCE_WINDOW`]（200 000）为标定基准 —— 即**恰好**让 200k 窗口复现旧的绝对值，
//! 更小的窗口按比例收缩，更大的窗口被 cap 兜住（不会无限膨胀）。
//!
//! ## 单一来源（本模块存在的真正理由）
//!
//! 「剩余额度查询」工具与 auto-compact 阈值判据**必须共用本模块的
//! [`budgets_for`]**：两处各自换算窗口比例是本仓最易漂移的一类缺陷
//! （口径漂移后，工具报「还剩 30%」而压缩在 20% 就触发，模型据此做的规划全错）。
//! 对标 codex 把 `buffered_auto_compact_limit` 单一化的做法。
//!
//! 本模块只做**纯算术**，不读配置、不碰 IO、不依赖任何 axagent-* crate。

/// auto-compact 触发比例：上下文总量达到窗口的 70% 即压缩。
///
/// ⚠ 这是**唯一**的阈值来源。`src-tauri/src/context_manager.rs` 的
/// `should_auto_compress` 经由 [`ContextBudgets::auto_compact_threshold`] 取值，
/// 不得在别处再写一遍 `0.70`。
pub const AUTO_COMPACT_THRESHOLD_RATIO: f64 = 0.70;

/// 分量预算的标定基准窗口（token）。比例 = cap / REFERENCE_WINDOW。
///
/// 选 200 000 是为了**行为兼容**：该窗口下 `min(ratio × window, cap) == cap`，
/// 与改造前的绝对值逐字相同。
pub const REFERENCE_WINDOW: usize = 200_000;

/// system prompt 分量上限（含权限说明段，见 `render_permission_notice`）。
pub const SYSTEM_PROMPT_CAP: usize = 8_000;
/// working memory 注入上限。
pub const WORKING_MEMORY_CAP: usize = 800;
/// RAG / 记忆检索结果注入上限。
pub const RETRIEVED_MEMORIES_CAP: usize = 10_000;
/// 技能索引（渐进式披露的目录层）上限。
pub const SKILLS_CAP: usize = 5_000;
/// nudge 建议上限。
pub const NUDGES_CAP: usize = 2_000;

/// 历史消息占比：扣掉五个分量后，窗口剩余部分的 65% 留给对话历史。
///
/// 这是**比例**（已随窗口伸缩），故不参与 `min(ratio × window, cap)` 改造。
pub const HISTORY_RATIO: f64 = 0.65;

/// 某一模型窗口下的全部分量预算。
///
/// 由 [`budgets_for`] 构造；所有需要「按窗口取预算」的代码（判据 + 工具 + 展示）
/// 都从这里取值，避免各算各的。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ContextBudgets {
    /// 模型上下文窗口（token）。
    pub context_window: usize,
    /// system prompt 预算。
    pub system_prompt: usize,
    /// working memory 预算。
    pub working_memory: usize,
    /// RAG / 记忆检索预算。
    pub retrieved_memories: usize,
    /// 技能索引预算。
    pub skills: usize,
    /// nudge 预算。
    pub nudges: usize,
    /// 历史消息占比。
    pub history_ratio: f64,
}

impl ContextBudgets {
    /// 五个固定分量之和 —— 计算历史额度前必须先扣掉的部分。
    #[must_use]
    pub fn fixed_overhead(&self) -> usize {
        self.system_prompt
            .saturating_add(self.working_memory)
            .saturating_add(self.retrieved_memories)
            .saturating_add(self.skills)
            .saturating_add(self.nudges)
    }

    /// 对话历史的 token 额度：`(窗口 - 固定分量) × history_ratio`。
    #[must_use]
    pub fn history_budget(&self) -> usize {
        let remaining = self.context_window.saturating_sub(self.fixed_overhead());
        (remaining as f64 * self.history_ratio) as usize
    }

    /// auto-compact 触发阈值：`窗口 × AUTO_COMPACT_THRESHOLD_RATIO`。
    ///
    /// 「剩余额度」= 本值 − 当前已用量，工具与判据两边的口径由此对齐。
    #[must_use]
    pub fn auto_compact_threshold(&self) -> usize {
        (self.context_window as f64 * AUTO_COMPACT_THRESHOLD_RATIO) as usize
    }
}

/// 由模型上下文窗口算出全部分量预算（**唯一**供值函数）。
///
/// `context_window` 为 0（未知窗口的调用方传入）时各分量均为 0 —— 调用方应先判断
/// 「窗口是否已知」，不要用 0 当作「不限」。
#[must_use]
pub fn budgets_for(context_window: usize) -> ContextBudgets {
    ContextBudgets {
        context_window,
        system_prompt: scaled(SYSTEM_PROMPT_CAP, context_window),
        working_memory: scaled(WORKING_MEMORY_CAP, context_window),
        retrieved_memories: scaled(RETRIEVED_MEMORIES_CAP, context_window),
        skills: scaled(SKILLS_CAP, context_window),
        nudges: scaled(NUDGES_CAP, context_window),
        history_ratio: HISTORY_RATIO,
    }
}

/// `min(ratio × window, cap)`，其中 `ratio = cap / REFERENCE_WINDOW`。
fn scaled(cap: usize, context_window: usize) -> usize {
    let ratio_scaled = (cap as f64 / REFERENCE_WINDOW as f64) * context_window as f64;
    (ratio_scaled as usize).min(cap)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 200k 参考窗下必须与改造前的绝对值逐字相同（行为兼容的**唯一**硬判据）。
    #[test]
    fn reference_window_reproduces_legacy_absolute_values() {
        let b = budgets_for(REFERENCE_WINDOW);
        assert_eq!(b.system_prompt, 8_000);
        assert_eq!(b.working_memory, 800);
        assert_eq!(b.retrieved_memories, 10_000);
        assert_eq!(b.skills, 5_000);
        assert_eq!(b.nudges, 2_000);
        assert_eq!(b.fixed_overhead(), 25_800);
        assert_eq!(b.history_ratio, 0.65);
    }

    /// 小窗口按比例收缩（这才是「随动」的实证）。
    #[test]
    fn smaller_window_scales_components_down() {
        let b = budgets_for(32_000);
        assert_eq!(b.system_prompt, 1_280); // 0.04 × 32_000
        assert_eq!(b.working_memory, 128); // 0.004 × 32_000
        assert_eq!(b.retrieved_memories, 1_600); // 0.05 × 32_000
        assert_eq!(b.skills, 800); // 0.025 × 32_000
        assert_eq!(b.nudges, 320); // 0.01 × 32_000
        assert_eq!(b.fixed_overhead(), 4_128);
        assert!(b.fixed_overhead() < budgets_for(REFERENCE_WINDOW).fixed_overhead());
    }

    /// 超出参考窗时被 cap 兜住，不随窗口无限膨胀。
    #[test]
    fn larger_window_is_capped() {
        let b = budgets_for(1_000_000);
        assert_eq!(b.system_prompt, SYSTEM_PROMPT_CAP);
        assert_eq!(b.working_memory, WORKING_MEMORY_CAP);
        assert_eq!(b.retrieved_memories, RETRIEVED_MEMORIES_CAP);
        assert_eq!(b.skills, SKILLS_CAP);
        assert_eq!(b.nudges, NUDGES_CAP);
        assert_eq!(b.fixed_overhead(), 25_800);
    }

    /// 分量随窗口单调不减（防「大窗口反而拿到更小预算」这类符号错误）。
    #[test]
    fn components_are_monotonic_in_window() {
        let windows = [1_000usize, 8_000, 32_000, 128_000, 200_000, 1_000_000];
        let mut prev = budgets_for(0);
        for w in windows {
            let cur = budgets_for(w);
            assert!(cur.system_prompt >= prev.system_prompt, "window={w}");
            assert!(cur.working_memory >= prev.working_memory, "window={w}");
            assert!(cur.retrieved_memories >= prev.retrieved_memories, "window={w}");
            assert!(cur.skills >= prev.skills, "window={w}");
            assert!(cur.nudges >= prev.nudges, "window={w}");
            prev = cur;
        }
    }

    /// 阈值 = 窗口 × 0.70，且与 `context_manager::should_auto_compress` 同源。
    #[test]
    fn auto_compact_threshold_is_ratio_of_window() {
        assert_eq!(budgets_for(200_000).auto_compact_threshold(), 140_000);
        assert_eq!(budgets_for(32_000).auto_compact_threshold(), 22_400);
        assert_eq!(budgets_for(0).auto_compact_threshold(), 0);
    }

    /// 历史额度 = (窗口 - 固定分量) × 0.65；窗口小于分量之和时不得下溢 panic。
    #[test]
    fn history_budget_deducts_fixed_overhead() {
        let b = budgets_for(REFERENCE_WINDOW);
        assert_eq!(b.history_budget(), ((200_000 - 25_800) as f64 * 0.65) as usize);
        // 极端小窗口：固定分量可能超过窗口，必须饱和到 0 而不是 panic
        assert_eq!(budgets_for(0).history_budget(), 0);
        assert_eq!(budgets_for(1).history_budget(), 0);
    }

    /// 未知窗口（0）时各分量归零 —— 调用方须先判断「窗口是否已知」。
    #[test]
    fn zero_window_yields_zero_budgets() {
        let b = budgets_for(0);
        assert_eq!(b.fixed_overhead(), 0);
        assert_eq!(b.context_window, 0);
        assert_eq!(b.auto_compact_threshold(), 0);
    }
}
