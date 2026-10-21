use thiserror::Error;

#[derive(Debug, Error)]
pub enum DataError {
    #[error("HTTP request failed: {0}")]
    HttpError(#[from] reqwest::Error),

    #[error("JSON parse error: {0}")]
    ParseError(String),

    #[error("Rate limit exceeded from {vendor}")]
    RateLimited { vendor: String },

    #[error("Vendor error from {vendor}: {message}")]
    VendorError { vendor: String, message: String },

    #[error("Stock code not found: {0}")]
    NotFound(String),

    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),

    /// As-Of 模式下数据降级：vendor / method 在该时间点不支持或无历史语义。
    /// 不视为致命错误——workflow 节点应降级到空数据并在 `dataQualitySummary`
    /// 里记录"降级原因",但允许继续推进决策(置信度由 aggregator 扣分)。
    ///
    /// spec §4.1 统一降级协议
    #[error("As-Of 模式下 {vendor}::{method} 降级: {reason} (as_of={as_of})")]
    AsOfDegraded { vendor: String, method: String, reason: String, as_of: String },

    /// 内部错误（信号量关闭、运行时异常等不应发生但需要传播的错误）
    /// 修复 M-DEF-1: 用于替代 `acquire_owned().expect("semaphore closed")`
    #[error("Internal error: {0}")]
    Internal(String),
}

impl DataError {
    /// 是否属于「源明确回答没有这份数据」而非故障。
    ///
    /// 判据与 `try_vendors_retry` 的降级/重试分档同源：`VendorError` 的消息含
    /// 「为空 / 返回空 / empty / no data / 无数据」即视为空数据。调用方用它与
    /// 真故障分开处理（例如分红为空表示没有除权事件，可安全走恒等复权；
    /// 接口失效则不能静默）。
    pub fn is_empty_data(&self) -> bool {
        match self {
            Self::VendorError { message, .. } => {
                let m = message.to_lowercase();
                m.contains("为空")
                    || m.contains("返回空")
                    || m.contains("empty")
                    || m.contains("no data")
                    || m.contains("无数据")
            },
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn asof_degraded_error_displays_all_fields() {
        let e = DataError::AsOfDegraded {
            vendor: "eastmoney".into(),
            method: "get_hot_stocks".into(),
            reason: "no historical semantics".into(),
            as_of: "2026-06-01".into(),
        };
        let s = e.to_string();
        assert!(s.contains("eastmoney"));
        assert!(s.contains("get_hot_stocks"));
        assert!(s.contains("2026-06-01"));
        assert!(s.contains("降级"));
    }

    /// 空数据与真故障必须可分：分红主流程依赖这条判据决定「返回未复权」还是「报错」。
    #[test]
    fn empty_data_is_distinct_from_vendor_failure() {
        let empty = DataError::VendorError {
            vendor: "eastmoney".into(),
            message: "分红数据为空".into(),
        };
        assert!(empty.is_empty_data());

        let failure = DataError::VendorError {
            vendor: "eastmoney".into(),
            message: "报表不可用: 报表配置不存在".into(),
        };
        assert!(!failure.is_empty_data(), "接口失效不得被当成无分红");
        assert!(!DataError::ParseError("bad json".into()).is_empty_data());
    }
}
