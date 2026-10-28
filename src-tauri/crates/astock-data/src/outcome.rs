//! 一次取数调用的**结果状态**（单一权威）。
//!
//! 与既有三条轴不重叠，禁止互相顶替：
//! - [`crate::vendor_health::VendorStatus`] —— 厂商整体健康（跨接口的 30s 滑动窗口）；
//! - [`crate::validation::FieldStatus`] —— 单个字段的校验结论（必填/非法）；
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

use crate::error::DataError;

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

/// 把 vendor/链级错误归到四个状态 —— **全仓唯一判据**。
///
/// - `ParseError` ⇒ `SchemaChanged`：响应拿到了但结构不符（字段缺失/改名/非 JSON）；
/// - 明确答「没有这份数据」的错误 ⇒ `EmptyByTime`：源已正常应答，不是故障；
/// - 其余（HTTP/限流/连接/内部）⇒ `SourceFailed`。
///
/// ⚠ 顺序不能换：`ParseError` 的文案可能含「无数据」字样，先判结构再判空。
pub fn classify_error(err: &DataError) -> DataStatus {
    match err {
        DataError::ParseError(_) => DataStatus::SchemaChanged,
        e if e.is_empty_data() => DataStatus::EmptyByTime,
        _ => DataStatus::SourceFailed,
    }
}

/// 从逐源证据决定**链级**状态 —— 与 [`classify_error`] 同一族的唯一判据。
///
/// 优先级刻意是「最可行动」而非「最常见」：
/// 1. 任一源结构变更 ⇒ `SchemaChanged`（要改代码，重试/换源都不会好）；
/// 2. 任一源传输故障 ⇒ `SourceFailed`（换源或换时段可能好转）；
/// 3. 全部源都明确答「没有」⇒ `EmptyByTime`；
/// 4. 没有任何证据（通道全被健康过滤/无凭据）⇒ `SourceFailed`（不能伪装成「今日无数据」）。
///
/// ⚠ 混合（有的答空、有的故障）不得判成 `EmptyByTime` —— 那是把故障藏进「今天没有」。
pub fn chain_status(sources: &[(String, DataStatus, String)]) -> DataStatus {
    if sources.iter().any(|(_, st, _)| *st == DataStatus::SchemaChanged) {
        return DataStatus::SchemaChanged;
    }
    if sources.iter().any(|(_, st, _)| *st == DataStatus::SourceFailed) {
        return DataStatus::SourceFailed;
    }
    if !sources.is_empty() && sources.iter().all(|(_, st, _)| *st == DataStatus::EmptyByTime) {
        return DataStatus::EmptyByTime;
    }
    DataStatus::SourceFailed
}

/// 逐源证据的**唯一拼装形态**：`源=状态` 或 `源=状态(原因)`，分号分隔。
///
/// 面板、报告、工具返回的 `detail` 都读这一句，避免三处各写一种拼接方式。
pub fn summarize_sources(sources: &[(String, DataStatus, String)]) -> String {
    sources
        .iter()
        .map(|(name, status, detail)| {
            let detail = detail.trim();
            if detail.is_empty() {
                format!("{name}={}", status.as_str())
            } else {
                format!("{name}={}({detail})", status.as_str())
            }
        })
        .collect::<Vec<_>>()
        .join("; ")
}

/// 收盘后「本应有数据」的期望 —— 空值归因的**唯一分档**。
///
/// 与 [`DataStatus`] 分开的理由：同样是「返回空」，`MustHaveRows`（行情/财务/资金流）
/// 说明取数被静默降级，`MayBeEmpty`（新闻/快讯/龙虎榜）在清淡日**本来**可以没有。
/// 把前者写成后者，正是本轮要修的形态。日检与 MCP 工具层共用这一档，不得各写一份。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Expect {
    /// 收盘后必须有行；返回空 = 疑似静默降级（`SourceFailed`）。
    MustHaveRows,
    /// 清淡日可以合法为空（`EmptyByTime`）。
    MayBeEmpty,
}

/// 「返回空且未报错」的归因 —— 唯一判据，见 [`Expect`]。
pub fn empty_evaluation(expect: Expect) -> (DataStatus, Option<String>) {
    match expect {
        Expect::MustHaveRows => (
            DataStatus::SourceFailed,
            Some("收盘后返回空且未报错（疑似被下游静默降级）".to_string()),
        ),
        Expect::MayBeEmpty => (DataStatus::EmptyByTime, None),
    }
}

/// 把 `Vec` 型取数结果包成信封（MCP 工具层与日检共用）。
///
/// ⚠ `Err` 分支的 `data` 必须是空集合而不是丢弃信封 —— 下游按「空集合」处理是既有
/// 契约（见 [`DataOutcome`] 的字段注释），信封只额外携带成因。
pub fn outcome_from_vec<T>(res: Result<Vec<T>, DataError>, expect: Expect) -> DataOutcome<Vec<T>> {
    match res {
        Ok(rows) if !rows.is_empty() => DataOutcome::ok(rows),
        Ok(rows) => {
            let (status, detail) = empty_evaluation(expect);
            DataOutcome { status, data: rows, detail }
        },
        Err(e) => DataOutcome {
            status: classify_error(&e),
            data: Vec::new(),
            detail: Some(e.to_string()),
        },
    }
}

/// 把 `Option` 型取数结果包成信封（单对象维度：资金流/北向/单只行情）。
pub fn outcome_from_opt<T>(
    res: Result<Option<T>, DataError>,
    expect: Expect,
) -> DataOutcome<Option<T>> {
    match res {
        Ok(Some(v)) => DataOutcome::ok(Some(v)),
        Ok(None) => {
            let (status, detail) = empty_evaluation(expect);
            DataOutcome { status, data: None, detail }
        },
        Err(e) => {
            DataOutcome { status: classify_error(&e), data: None, detail: Some(e.to_string()) }
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 四个拼写是与前端/面板/门禁共用的契约，改动必须同步消费方。
    #[test]
    fn status_literals_match_serde_output() {
        for st in [
            DataStatus::Ok,
            DataStatus::EmptyByTime,
            DataStatus::SourceFailed,
            DataStatus::SchemaChanged,
        ] {
            let json = serde_json::to_string(&st).unwrap();
            assert_eq!(json, format!("\"{}\"", st.as_str()));
        }
    }

    /// 结构错误优先于「空数据」：`ParseError("无数据字段缺失")` 必须归到 schema，
    /// 不能被文案里的「无数据」误判成时段性空值。
    #[test]
    fn parse_error_wins_over_empty_wording() {
        let parse = DataError::ParseError("data=null，无数据字段".into());
        assert_eq!(classify_error(&parse), DataStatus::SchemaChanged);

        // 源明确回答「没有」⇒ 时段性空值
        let answered_empty = DataError::VendorError {
            vendor: "eastmoney_push2".into(),
            message: "指数行情数据为空".into(),
        };
        assert_eq!(classify_error(&answered_empty), DataStatus::EmptyByTime);

        // 传输故障不得被当成空值
        let failed = DataError::VendorError {
            vendor: "tencent".into(),
            message: "HTTP request failed: connection reset".into(),
        };
        assert_eq!(classify_error(&failed), DataStatus::SourceFailed);
    }

    /// 链级判据的决策表：优先级顺序与「空/故障混合」的处理都要被锁住。
    #[test]
    fn chain_status_prefers_actionable_cause() {
        let s = |name: &str, st: DataStatus| (name.to_string(), st, String::new());
        // 全部答空 ⇒ 时段性空
        assert_eq!(
            chain_status(&[s("a", DataStatus::EmptyByTime), s("b", DataStatus::EmptyByTime)]),
            DataStatus::EmptyByTime
        );
        // 有空有故障 ⇒ 故障（不得把故障藏进「今天没有」）
        assert_eq!(
            chain_status(&[s("a", DataStatus::EmptyByTime), s("b", DataStatus::SourceFailed)]),
            DataStatus::SourceFailed
        );
        // 故障与结构变更并存 ⇒ 结构变更（最可行动）
        assert_eq!(
            chain_status(&[s("a", DataStatus::SourceFailed), s("b", DataStatus::SchemaChanged)]),
            DataStatus::SchemaChanged
        );
        // 没有任何证据（通道全被过滤）⇒ 故障，而不是时段性空
        assert_eq!(chain_status(&[]), DataStatus::SourceFailed);
    }

    /// 信封的序列化形状：状态 + 数据 + 可选 detail；无 detail 时不得出现 `detail: null`
    /// （下游按「键缺席」判，不吃 null 占位）。
    #[test]
    fn envelope_shape_omits_absent_detail() {
        let ok = DataOutcome::ok(vec![1, 2]);
        let v = serde_json::to_value(&ok).unwrap();
        assert_eq!(v["status"], "ok");
        assert_eq!(v["data"], serde_json::json!([1, 2]));
        assert!(v.get("detail").is_none());

        let failed = DataOutcome::<Vec<i32>>::source_failed(vec![], "全部通道 429");
        let v = serde_json::to_value(&failed).unwrap();
        assert_eq!(v["status"], "source_failed");
        assert_eq!(v["data"], serde_json::json!([]));
        assert_eq!(v["detail"], "全部通道 429");
    }
}
