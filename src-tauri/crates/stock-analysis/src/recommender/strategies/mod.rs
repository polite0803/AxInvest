//! 子策略导出

pub mod capital;
pub mod reversion;
pub mod trend;
pub mod value;

pub use capital::CapitalStrategy;
pub use reversion::ReversionStrategy;
pub use trend::TrendStrategy;
pub use value::ValueStrategy;
