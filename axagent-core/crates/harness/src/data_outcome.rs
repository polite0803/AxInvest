// SPDX-License-Identifier: AGPL-3.0-only

//! 一次取数调用的**结果状态**（单一权威）。
//!
//! 2026-11-02 自 `axagent-astock-data::outcome` 上收（纯机械迁移）：
//! 插件 worker 只能依赖 `axagent-harness`，拿不到 implementor crate 的 DTO。
//! 原路径 `axagent_astock_data::outcome::*` 由该 crate 的 `pub use` 垫片继续提供。
//! 与既有三条轴不重叠，禁止互相顶替：
//! - 「厂商整体健康」（astock-data 的 `vendor_health::VendorStatus`）—— 跨接口的 30s 滑动窗口；
//! - 「单个字段的校验结论」（astock-data 的 `validation::FieldStatus`）—— 必填/非法；
//! - 本模块 [`DataStatus`] —— **这次调用到底取到了什么、为什么没取到**。
//!
//! ## 存在理由（2026-10-28）
//!
//! `get_index_quotes` 在三条通道全部失败时返回 `Ok(vec![])`：下游只能看到「空数组」，
//! 无法区分「今日无行情」「通道故障」「接口改版」。一个「看起来正常但其实是空数据」的
//! 接口会污染整条决策链 —— 报告板块恒空、因子链把缺失当 0、分析师把没取到数当成
//! 「今天没有」。本模块把成因做成**返回值的一部分**，而不是只留在 tracing 里。
//!
//! ## 四个状态与会话边界
//!
//! - `Ok`：拿到了可用数据（允许 `detail` 说明部分缺失，例如 3 个指数只回了 2 个）；
//! - `EmptyByTime`：各通道都**正常应答**，但这个时点确实没有数据（收盘前/休市/当日无事件）；
//! - `SourceFailed`：传输/反爬/限流等故障，换时段可能好转；
//! - `SchemaChanged`：响应拿到了但结构与预期不符（字段改名/字段缺失/JSON 解析失败），
//!   重试无意义，要改代码。
//!
//! ⚠ 四态之间不得静态塌缩：尤其「源故障」不能写成「时段性无数据」——那正是本轮要修的形态。

use serde::{Deserialize, Serialize};

/// 一次取数调用的结论。序列化为 snake_case 字符串（`ok` / `empty_by_time` / ...）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DataStatus {
    /// 拿到了可用数据。
    Ok,
    /// 通道正常应答，但该时点没有数据。
    EmptyByTime,
    /// 传输/反爬/限流等故障（可重试、换源或换时段）。
    SourceFailed,
    /// 响应结构变了（字段缺失/改名/解析失败），重试无意义。
    SchemaChanged,
}

impl DataStatus {
    pub fn is_ok(self) -> bool {
        matches!(self, Self::Ok)
    }

    /// 序列化用的稳定字面量（与 `serde(rename_all = "snake_case")` 同一套拼写）。
    ///
    /// 供 Rhai / 日志等拿不到 serde 的场合使用；改拼写必须同时改枚举，否则两套口径。
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::EmptyByTime => "empty_by_time",
            Self::SourceFailed => "source_failed",
            Self::SchemaChanged => "schema_changed",
        }
    }
}

/// 一次取数调用的结果信封：`data` 始终存在（空集合也是合法值），`status` 说明成因。
///
/// 为什么 `data` 不用 `Option`：下游（报告渲染、Rhai 摘要）本来就按「空集合」处理，
/// 保持形状不变才能让历史消费方零改动地继续工作；状态是**附加信息**，不是替代形态。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DataOutcome<T> {
    pub status: DataStatus,
    pub data: T,
    /// 人类可读的成因/逐源证据（进面板、进报告、进工具返回）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

impl<T> DataOutcome<T> {
    pub fn ok(data: T) -> Self {
        Self { status: DataStatus::Ok, data, detail: None }
    }

    pub fn empty_by_time(data: T, detail: impl Into<String>) -> Self {
        Self { status: DataStatus::EmptyByTime, data, detail: Some(detail.into()) }
    }

    pub fn source_failed(data: T, detail: impl Into<String>) -> Self {
        Self { status: DataStatus::SourceFailed, data, detail: Some(detail.into()) }
    }

    pub fn schema_changed(data: T, detail: impl Into<String>) -> Self {
        Self { status: DataStatus::SchemaChanged, data, detail: Some(detail.into()) }
    }

    /// 附加/替换成因说明（`ok` 也允许带「部分缺失」这类说明）。
    pub fn with_detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = Some(detail.into());
        self
    }

    pub fn is_ok(&self) -> bool {
        self.status.is_ok()
    }

    /// 只取数据，丢掉成因 —— **仅限**能安全降级为「空即无数据」的调用方使用。
    ///
    /// 会丢掉成因的路径必须在调用处写明为什么可以丢；对外/对 LLM 的通道一律用信封。
    pub fn into_data(self) -> T {
        self.data
    }
}
