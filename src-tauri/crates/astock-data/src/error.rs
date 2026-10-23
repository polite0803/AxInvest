use thiserror::Error;

#[derive(Debug, Error)]
pub enum DataError {
    #[error("HTTP request failed: {0}")]
    HttpError(#[from] reqwest::Error),

    #[error("JSON parse error: {0}")]
    ParseError(String),

    /// 限流／频控。
    ///
    /// `reason` 保留**具体形态**：429 状态码只是限流的一种表现，股吧这类 HTML 抓页通道
    /// 的限流表现为「HTTP 200 但站点不给列表页」。归成限流之后如果只剩一句
    /// "Rate limit exceeded from guba"，上游条目反而比归因前更不可诊断。
    #[error("Rate limit exceeded from {vendor}: {reason}")]
    RateLimited { vendor: String, reason: String },

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

    /// 是否属于「同一批源再问一次可能就好了」的瞬态故障。
    ///
    /// 与 `is_empty_data` 同住这里：`try_vendors_retry` 的「降级／重试」分档只能有一个
    /// 权威，否则会出现「重试时算故障、调用方算瞬态」两套口径。
    ///
    /// 只认三类有明确语义信号的：429（站点自陈「请求过多」）、连接失败与超时。
    /// `HttpError` 变体内部还要再分 —— 它同时装着「连不上」（换一次可能就好）和
    /// 「URL 非法／响应解码失败」（每次都一样），整族算瞬态会让接口改版白付退避。
    /// `VendorError` 不在内 —— 这条是靠 vendor 侧**先分道**才成立的：股吧那种「HTTP 200
    /// 但站点没给列表页」的频控形态，上报时就已经归成 `RateLimited`（见
    /// `vendors/guba.rs::to_live_error`），留在 `VendorError` 里的都是改版那类持续故障。
    pub fn is_transient(&self) -> bool {
        match self {
            Self::RateLimited { .. } => true,
            Self::HttpError(e) => e.is_connect() || e.is_timeout(),
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

    /// 瞬态只认三个有明确语义信号的情形。`VendorError` 不得算瞬态 —— 否则
    /// `try_vendors_retry` 的「单源不重试」短路对所有失败失效，接口改版也白付退避。
    ///
    /// 两条都走本机回环与非法 URL，不出本机、不依赖外网。
    #[tokio::test]
    async fn transient_is_limited_to_rate_limit_connect_and_timeout() {
        assert!(DataError::RateLimited { vendor: "guba".into(), reason: "HTTP 429".into() }
            .is_transient());

        // 127.0.0.1:1 无人监听 ⇒ 立即被拒（is_connect），属「再问一次可能就好」
        let refused = reqwest::Client::new()
            .get("http://127.0.0.1:1/x")
            .timeout(std::time::Duration::from_secs(3))
            .send()
            .await
            .unwrap_err();
        let refused = DataError::from(refused);
        assert!(refused.is_transient(), "连接被拒不属瞬态的话，路由层退避就永远不会触发");
        assert!(!refused.is_empty_data(), "连接失败不得被当成空数据");

        // 同一个 HttpError 变体里的持续故障不得算瞬态：端口越界 ⇒ URL 解析失败
        let bad_url = reqwest::Client::new().get("http://127.0.0.1:99999/").build().unwrap_err();
        assert!(!DataError::from(bad_url).is_transient(), "URL 非法每次都得到同一个结果，不是瞬态");

        assert!(!DataError::ParseError("bad json".into()).is_transient());
        assert!(!DataError::NotFound("003029".into()).is_transient());
        assert!(
            !DataError::VendorError { vendor: "guba".into(), message: "结构变更".into() }
                .is_transient(),
            "结构类持续故障不得算瞬态；壳页那类已在 vendor 侧归成 RateLimited"
        );
    }
}
