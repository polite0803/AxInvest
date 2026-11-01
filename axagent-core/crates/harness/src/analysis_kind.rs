//! 分析记录的**来历**值域（`stock_analyses.analysis_kind`）—— 唯一权威。
//!
//! ## 这一列只答一个问题：这次运行带不带 as-of 上下文
//!
//! | 值 | 含义 | 写侧判据 |
//! |---|---|---|
//! | `live` | 跑在**当前**数据上 | [`AnalysisKind::from_asof`]（`false`） |
//! | `replay` | as-of 时间旅行：资料锚到过去的某一天 | [`AnalysisKind::from_asof`]（`true`） |
//!
//! ## 为什么这里没有第三个值（两条退役）
//!
//! 1. **`chat` 从本列退役**（2026-10-23）。对话直执行通道此前在 `hooks.rs` 里往本列写
//!    `"chat"`，而**同一行它已经写了 `source = "chat"`** —— 同一件事在两列各记一遍，
//!    且把「谁触发的」（`source` 轴）混进了「带不带 as-of」（本列轴）。实体注释
//!    （`entities/stock_analyses.rs` 的 `source` 字段）本来就写明分工：
//!    「kind 只分 live/replay 两态，source 记业务出处」。本改动是**照注释归位**，不是重新设计。
//!    归位后：chat 通道记 `live`（它确实跑在当前数据上），通道信息由 `source='chat'` 唯一承担。
//!    ⚠ 副作用（已知且刻意）：对话通道产出的记录从此会进入 `= 'live'` 的读者
//!    （`pool_scan` 的「当日已分析」、`reco_loop` 的回环否决窗口）。现网 `analysis_kind='chat'`
//!    **0 行** ⇒ 今天零变化；将来第一次对话直执行才会进入这些统计。
//! 2. **`ab_test` 从未存在**。实体列注释曾写 `'live' | 'replay' | 'ab_test'`，但全仓
//!    `ab_test` 只出现在 `prompt_template` 的字段名里，没有任何生产者写过它 ——
//!    属「声明了值域中不存在的档」，本文件与实体注释一并改掉。
//!
//! ## 与 `decision_action` 是两个值域，勿合并
//!
//! 同挂一行 `stock_analyses`，但 `decision_action` 答「结论是什么」、本列答「怎么跑出来的」。
//! 合成一张表会让消费端把「来历」当「结论」—— 与 [`crate::decision_action`] 模块头记的
//! 「同名语义散成多套值域 ⇒ 必有一处 fail-open」同一类缺陷的反向形态（值域过度合并）。
//!
//! ## 同词不同源，别混
//!
//! 前端 `useTimeAnchorStore.anchorMode === "replay"` 是**查看者当前的时间锚**，与本列
//! （这条记录当时怎么产生的）不是一回事。另：现网 `as_of_date` 在 198 条 live 行上**全部非空**
//! 且等于 `analysis_date` ⇒ `as_of_date IS NULL` 在 `stock_analyses` 上**不等于**「实盘代」
//! （该谓词只在 `reco_picks` / `decision_validations` / `reco_scan_audit` 上成立）。

use serde::{Deserialize, Serialize};

/// 记录来历。写侧只允许经 [`AnalysisKind::from_asof`] 产生，
/// 读侧筛选与值解析走 [`AnalysisKind::as_storage`] / [`AnalysisKind::from_storage`]。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AnalysisKind {
    /// 跑在当前数据上（含对话直执行通道）
    Live,
    /// as-of 时间旅行产出的样本
    Replay,
}

/// 值域清单 —— DB 的 `CHECK` 约束（`dao::reconcile::extras::CHECK_CONSTRAINTS`）
/// 与前端 `provenanceOfRecord` 的判据都以它为准（门 R13 逐方向对账）。
pub const ANALYSIS_KIND_VALUES: &[&str] = &["live", "replay"];

impl AnalysisKind {
    /// 落库值（`stock_analyses.analysis_kind`）。
    pub fn as_storage(self) -> &'static str {
        match self {
            Self::Live => "live",
            Self::Replay => "replay",
        }
    }

    /// 展示用中文（界面上「实况 / 回放」徽标的文案口径）。
    pub fn as_cn(self) -> &'static str {
        match self {
            Self::Live => "实况",
            Self::Replay => "回放",
        }
    }

    /// 严格解析落库值；未识别返回 `None` 由调用方决定兜底。
    ///
    /// **不得**图省事默认成 `Live` —— 那会把「来历不明」伪装成「跑在当前数据上」，
    /// 而回放样本的决策日是 `as_of_date`、不是入库时刻。退役掉的 `chat` 也走 `None`
    /// （库里 0 行 ⇒ 读不到；万一有遗留行，宁可显示「来源未知」也不猜）。
    pub fn from_storage(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "live" => Some(Self::Live),
            "replay" => Some(Self::Replay),
            _ => None,
        }
    }

    /// 写侧唯一判据：本次运行是否带 as-of 上下文。
    pub fn from_asof(is_time_travel_run: bool) -> Self {
        if is_time_travel_run {
            Self::Replay
        } else {
            Self::Live
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 两态的落库值与值域清单逐项一致() {
        let storage = [AnalysisKind::Live.as_storage(), AnalysisKind::Replay.as_storage()];
        assert_eq!(storage.len(), ANALYSIS_KIND_VALUES.len());
        for v in storage {
            assert!(ANALYSIS_KIND_VALUES.contains(&v), "{v} 不在值域清单里");
        }
        assert_eq!(AnalysisKind::Live.as_cn(), "实况");
        assert_eq!(AnalysisKind::Replay.as_cn(), "回放");
    }

    #[test]
    fn 写侧判据只认_asof_有无() {
        assert_eq!(AnalysisKind::from_asof(true), AnalysisKind::Replay);
        assert_eq!(AnalysisKind::from_asof(false), AnalysisKind::Live);
    }

    #[test]
    fn 退役值与未知值一律读不出来() {
        assert_eq!(AnalysisKind::from_storage("live"), Some(AnalysisKind::Live));
        assert_eq!(AnalysisKind::from_storage("REPLAY"), Some(AnalysisKind::Replay));
        assert_eq!(AnalysisKind::from_storage(" replay "), Some(AnalysisKind::Replay));
        // `chat` 已从本列退役（改由 source 承担）、`ab_test` 从未存在 —— 都不许被猜成任何一态
        for gone in ["chat", "ab_test", "", "实况"] {
            assert_eq!(AnalysisKind::from_storage(gone), None, "{gone} 不该被解析出来");
        }
    }
}
