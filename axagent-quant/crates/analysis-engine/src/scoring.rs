//! 100 分制客观评分引擎（已下沉到 axagent-astock-data crate）
//!
//! 保持向后兼容的 re-export 层。
//!
//! `PeBands` 也在这一层：它是基本面修正的 PE 阈值（面板 `val_pe_low` / `val_pe_high` 的落点），
//! 而**回放链**（`commands::stock_analysis`）要用同一个来源，不能再抄一份数字 ——
//! 2026-10-08 A 批之前正是「scoring.rs 抄 15/50、回放链抄 20/40」两处并存。
//!
//! `PbBands` 与它同形（面板 `val_pb_low` / `val_pb_high`，2026-10-09 B 批接线）：
//! 成对 re-export 是为了「只出 PE 的载体」不把下游逼回 `axagent_astock_data` 直连。

pub use axagent_astock_data::scoring::{
    ObjectiveScore, PbBands, PeBands, ScoreBands, ScoringEngine, ScoringWeights,
};
