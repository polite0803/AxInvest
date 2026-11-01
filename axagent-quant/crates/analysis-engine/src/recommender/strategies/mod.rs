//! 子策略导出（荐股侧的 5 个风格；趋势智选不再是其中之一，见 `recommender::mod` 的裁定说明）

pub mod capital;
pub mod reversion;
pub mod trend;
pub mod value;
pub mod watchlist;

pub use capital::CapitalStrategy;
pub use reversion::ReversionStrategy;
pub use trend::TrendStrategy;
pub use value::ValueStrategy;
pub use watchlist::{WatchlistStrategy, emit_synthetic_picks};
