// SPDX-License-Identifier: AGPL-3.0-only

//! 时间旅行（As-Of）上下文 — 纯 DTO + 契约层
//!
//! 提供 AsOfContext、AsOfSource 等共享类型定义，
//! 让消费者（stock-analysis、quant）不直接依赖 axagent-astock-data。
//!
//! **不含运行时状态**（task_local、全局 Mutex 等实现在 astock-data 中）。

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// As-Of 上下文构造与解析错误
///
/// 权威错误类型，由 `AsOfContext::new` / `AsOfContext::parse` 返回。
/// 其他 crate 通过 `pub use axagent_harness::as_of::AsOfError` 引用，
/// 禁止在 astock-data 等下游 crate 中重复定义同义错误枚举。
#[derive(Debug, Error)]
pub enum AsOfError {
    /// as_of_date 不能晚于今天
    #[error("as_of_date cannot be in the future: {date} (today is {today})")]
    FutureDate { date: String, today: String },

    /// 日期字符串格式无效或为空
    #[error("as_of_date format invalid: {reason}")]
    InvalidFormat { reason: String },

    /// as_of_date 距今过老（预留变体，当前未启用）
    #[error("as_of_date too old: {0} days ago, max is {1}")]
    TooOld(i64, i64),
}

/// As-Of 数据的来源标签，用于审计
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AsOfSource {
    /// 用户在 UI 手动选择
    UserReplay,
    /// Sweep 工具批量跑
    BacktestSweep,
    /// 调度器周期跑
    ScheduledReplay,
}

impl std::fmt::Display for AsOfSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AsOfSource::UserReplay => write!(f, "user_replay"),
            AsOfSource::BacktestSweep => write!(f, "backtest_sweep"),
            AsOfSource::ScheduledReplay => write!(f, "scheduled_replay"),
        }
    }
}

/// 时间锚点：在该任务执行期间，所有 vendor 调用应被视为"截至 as_of_date"
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct AsOfContext {
    pub as_of_date: chrono::NaiveDate,
    pub source: AsOfSource,
    /// 数据截止范围(混合 as-of 模式)。默认 All 兼容旧行为。
    #[serde(default)]
    pub data_scope: AsOfDataScope,
}

impl AsOfContext {
    /// 创建 AsOfContext；as_of_date 必须在今天及之前
    ///
    /// 含 FutureDate 契约验证：若 date 晚于本地今天，返回 `AsOfError::FutureDate`。
    /// 该验证是契约层的一部分（时间旅行不允许指向未来），因此权威实现放在 harness。
    pub fn new(date: chrono::NaiveDate, source: AsOfSource) -> Result<Self, AsOfError> {
        let today = chrono::Local::now().date_naive();
        if date > today {
            return Err(AsOfError::FutureDate { date: date.to_string(), today: today.to_string() });
        }
        Ok(Self { as_of_date: date, source, data_scope: AsOfDataScope::All })
    }

    /// 创建带数据范围的 AsOfContext（消费式 builder API）
    pub fn with_data_scope(mut self, scope: AsOfDataScope) -> Self {
        self.data_scope = scope;
        self
    }

    /// 解析 'YYYY-MM-DD' 字符串；空字符串视为非法
    ///
    /// 返回 `AsOfError` 以便调用方按变体匹配（如 `Err(AsOfError::FutureDate { .. })`）。
    /// 同时执行 FutureDate 验证（与 `new` 一致）。
    pub fn parse(s: &str) -> Result<Self, AsOfError> {
        if s.is_empty() {
            return Err(AsOfError::InvalidFormat { reason: "empty string".into() });
        }
        let date = chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d")
            .map_err(|e| AsOfError::InvalidFormat { reason: e.to_string() })?;
        Self::new(date, AsOfSource::UserReplay)
    }

    /// 解析可选入参（None / 空 / 全空白 → None；合法 → Some；非法 → Err）
    ///
    /// 返回 `Result<Option<Self>, String>`（错误被扁平化为字符串），
    /// 便于 Tauri command 直接 `?` 透传到前端。
    pub fn parse_optional(s: Option<&str>) -> Result<Option<Self>, String> {
        match s.map(str::trim).filter(|s| !s.is_empty()) {
            None => Ok(None),
            Some(s) => Self::parse(s).map(Some).map_err(|e| format!("as_of_date 解析失败: {e}")),
        }
    }

    /// 转 'YYYY-MM-DD' 字符串
    pub fn as_string(&self) -> String {
        self.as_of_date.format("%Y-%m-%d").to_string()
    }
}

/// 数据截止范围(混合 as-of 模式核心枚举)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum AsOfDataScope {
    /// 所有数据按 as_of 截止(旧行为,默认)
    #[default]
    All,
    /// 仅"结构化数据"按 as_of 截止;新闻/公告/研报/排行 保持实时
    Structured,
}

/// 数据源种类(用于 AsOfDataScope 决策)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AsOfDataKind {
    /// 结构化数据
    Structured,
    /// 非结构化数据:新闻/公告/研报/社媒
    Unstructured,
    /// 排行榜/分类/指数
    Rank,
}

/// 降级条目的模式取值（与 serde 输出同一套词表，避免两处漂移）
pub const DEGRADATION_MODE_ASOF: &str = "asOf";
/// 实盘（无 as-of 上下文）模式
pub const DEGRADATION_MODE_LIVE: &str = "live";

/// 降级的**严重度分档**（T14，2026-09-27）。
///
/// 动因：降级条目此前只有一种形态 ⇒ 面板上「个股没有场内期权」和「接口 301 挂了」
/// 长得一样，用户只能读出「还是没好」，读不出哪条值得去修。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub enum DegradationKind {
    /// 真故障：被探测的源报错 / 连接被拒 / 接口失效 ⇒ **这一档才需要修**
    #[default]
    Failure,
    /// 取数面到顶，或源明确回答「截止日没有这份数据」（新闻倒序索引窗口、归档未积累）
    NoData,
    /// 该维度对回放本就不适用（个股无场内期权、当日榜单语义、快照是唯一通道）
    StructuralGap,
}

impl DegradationKind {
    /// 与 serde 输出**同一套**字符串（避免两处词表）
    pub fn label(self) -> &'static str {
        match self {
            Self::Failure => "failure",
            Self::NoData => "noData",
            Self::StructuralGap => "structuralGap",
        }
    }
}

/// As-Of 降级条目
#[derive(Debug, Clone, serde::Serialize)]
pub struct DegradationEntry {
    pub vendor: String,
    pub method: String,
    pub reason: String,
    pub as_of: String,
    /// 这条降级发生在哪个模式：`asOf`（回放）或 `live`（实盘）。
    /// Q10（2026-10-03）之前 live 根本不记 ⇒「取数质量优先于事后降权」在实盘无读数。
    /// 判据消费方**必须按 mode 过滤**：回放的豁免逻辑若把 live 条目也算进来，
    /// 就会把「实盘降级过」当成「本轮回放降级过」而误豁免（同 `asof_degraded_methods` 的 R5 教训）。
    #[serde(default)]
    pub mode: String,
    /// 缺省 `Failure` ⇒ 新增分档时**宁可多标一档，也不把真故障洗白**
    #[serde(default)]
    pub kind: DegradationKind,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// T14：面板分档靠 `kind` 走 wire，序列化值就是前端 i18n 的键 ⇒ 改动即断契约
    #[test]
    fn degradation_kind_serializes_to_camel_case_and_labels_match() {
        for (k, name) in [
            (DegradationKind::Failure, "failure"),
            (DegradationKind::NoData, "noData"),
            (DegradationKind::StructuralGap, "structuralGap"),
        ] {
            assert_eq!(serde_json::to_value(k).unwrap(), serde_json::json!(name));
            assert_eq!(k.label(), name, "label 与 serde 必须同一套词表");
        }
        // 缺省档 = Failure：新站点漏标只会多标红，不会把真故障洗白
        assert_eq!(DegradationKind::default(), DegradationKind::Failure);
    }
}
