//! 时间旅行（As-Of）上下文
//!
//! 通过 `tokio::task_local!` 注入当前任务的 `AsOfContext`，所有 vendor 调用
//! 都可以隐式读取截止日，从而过滤或降级数据。Live 模式下该 task-local 为 None。

use chrono::{Local, NaiveDate};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use thiserror::Error;

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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AsOfContext {
    pub as_of_date: NaiveDate,
    pub source: AsOfSource,
}

impl AsOfContext {
    /// 创建 AsOfContext；as_of_date 必须在今天及之前
    pub fn new(date: NaiveDate, source: AsOfSource) -> Result<Self, AsOfError> {
        let today = Local::now().date_naive();
        if date > today {
            return Err(AsOfError::FutureDate {
                date: date.to_string(),
                today: today.to_string(),
            });
        }
        Ok(Self {
            as_of_date: date,
            source,
        })
    }

    /// 解析 'YYYY-MM-DD' 字符串；空字符串视为非法
    pub fn parse(s: &str) -> Result<Self, AsOfError> {
        if s.is_empty() {
            return Err(AsOfError::InvalidFormat {
                reason: "empty string".into(),
            });
        }
        let date =
            NaiveDate::parse_from_str(s, "%Y-%m-%d").map_err(|e| AsOfError::InvalidFormat {
                reason: e.to_string(),
            })?;
        Self::new(date, AsOfSource::UserReplay)
    }

    /// 解析可选入参（None / 空 / 全空白 → None；合法 → Some；非法 → Err）
    pub fn parse_optional(s: Option<&str>) -> Result<Option<Self>, String> {
        match s.map(str::trim).filter(|s| !s.is_empty()) {
            None => Ok(None),
            Some(s) => Self::parse(s)
                .map(Some)
                .map_err(|e| format!("as_of_date 解析失败: {e}")),
        }
    }

    /// 转 'YYYY-MM-DD' 字符串
    pub fn as_string(&self) -> String {
        self.as_of_date.format("%Y-%m-%d").to_string()
    }
}

#[derive(Debug, Error)]
pub enum AsOfError {
    #[error("as_of_date cannot be in the future: {date} (today is {today})")]
    FutureDate { date: String, today: String },

    #[error("as_of_date format invalid: {reason}")]
    InvalidFormat { reason: String },

    #[error("as_of_date too old: {0} days ago, max is {1}")]
    TooOld(i64, i64),
}

tokio::task_local! {
    /// 当前任务内的 AsOfContext；None 表示 live 模式
    pub static AS_OF: Option<AsOfContext>;
}

/// 读取当前任务的 AsOfContext；scope 外返回 None
pub fn current_as_of() -> Option<AsOfContext> {
    AS_OF.try_with(|c| *c).ok().flatten()
}

/// 判断当前是否处于时间旅行模式
pub fn is_asof_active() -> bool {
    current_as_of().is_some()
}

/// 生成当前 AsOf 的 cache key 后缀（live 模式返回 "live"）
pub fn cache_suffix() -> String {
    current_as_of()
        .map(|c| format!("asof-{}", c.as_of_date.format("%Y%m%d")))
        .unwrap_or_else(|| "live".to_string())
}

/// As-Of 降级条目：vendor / method 在该时间点不可用或无历史语义
/// (spec §4.1 统一降级协议)
#[derive(Debug, Clone, serde::Serialize)]
pub struct DegradationEntry {
    pub vendor: String,
    pub method: String,
    pub reason: String,
    pub as_of: String,
}

// 任务级降级日志：每个 tokio 任务一个 Vec，scope 结束时不重置，
// 由 workflow 节点通过 take_asof_degradation_report() 一次性消费并清空。
tokio::task_local! {
    static DEGRADATION_LOG: std::cell::RefCell<Vec<DegradationEntry>>;
}

/// 全局降级环形缓冲(缺陷 E 修复):供前端 poll 实时显示降级数量/详情。
/// 不依赖 task_local 作用域(全局可见),cap 256 条,满了弹出最早。
static GLOBAL_DEGRADATION_LOG: Mutex<VecDeque<DegradationEntry>> = Mutex::new(VecDeque::new());
static GLOBAL_DEGRADATION_TOTAL: AtomicU64 = AtomicU64::new(0);

const GLOBAL_DEGRADATION_CAP: usize = 256;

/// 记录一次降级(仅在 as-of 模式下有效，live 模式直接忽略)
pub fn record_degradation(vendor: &str, method: &str, reason: &str) {
    let as_of = match current_as_of() {
        Some(c) => c.as_string(),
        None => return, // live 模式无降级概念
    };
    let entry = DegradationEntry {
        vendor: vendor.to_string(),
        method: method.to_string(),
        reason: reason.to_string(),
        as_of,
    };
    // 任务级尝试：若没在 task_local scope 中，单独初始化一个新 scope
    let _ = DEGRADATION_LOG.try_with(|cell| {
        cell.borrow_mut().push(entry.clone());
    });
    // 全局环形缓冲: 累计总数 + 保留最近 N 条详情
    if let Ok(mut g) = GLOBAL_DEGRADATION_LOG.lock() {
        if g.len() >= GLOBAL_DEGRADATION_CAP {
            g.pop_front();
        }
        g.push_back(entry);
        GLOBAL_DEGRADATION_TOTAL.fetch_add(1, Ordering::Relaxed);
    }
}

/// 在指定 scope 内运行闭包，并提供降级日志的 task_local 容器
pub async fn with_degradation_log<F, T>(f: F) -> T
where
    F: std::future::Future<Output = T>,
{
    DEGRADATION_LOG
        .scope(std::cell::RefCell::new(Vec::new()), f)
        .await
}

/// 在可选 AsOfContext 包裹下运行闭包。
///
/// - `ctx = None`  → 直接执行 `f`（live 模式，零开销）
/// - `ctx = Some(c)` → `AS_OF.scope(Some(c), f)`（让 vendor 调用能读到 task_local）
///
/// 用于 Tauri command 一致化：所有 vendor 命令都加 `as_of_date: Option<String>` 参数
/// 并用本函数包裹核心查询,无需在每个命令里重复 `if let Some / else` 模板。
pub async fn with_optional_asof<F, T>(ctx: Option<AsOfContext>, f: F) -> T
where
    F: std::future::Future<Output = T>,
{
    match ctx {
        Some(c) => AS_OF.scope(Some(c), f).await,
        None => f.await,
    }
}

/// 消费并清空当前任务的降级日志。返回累积的降级条目。
/// 必须在 with_degradation_log scope 内调用，否则返回空 Vec。
pub fn take_asof_degradation_report() -> Vec<DegradationEntry> {
    DEGRADATION_LOG
        .try_with(|cell| std::mem::take(&mut *cell.borrow_mut()))
        .unwrap_or_default()
}

/// 仅快照全局降级日志(不清空,供前端 poll 显示)。
/// 返回按时间顺序排列(旧 → 新)的最近 256 条。
pub fn peek_global_degradation_report() -> Vec<DegradationEntry> {
    GLOBAL_DEGRADATION_LOG
        .lock()
        .map(|g| g.iter().cloned().collect())
        .unwrap_or_default()
}

/// 当前累计降级总数(从进程启动起算,跨 live/replay 切换)。
pub fn global_degradation_count() -> u64 {
    GLOBAL_DEGRADATION_TOTAL.load(Ordering::Relaxed)
}

/// 清空全局降级缓冲(切换到 live 模式时由前端触发,避免过期条目一直显示)。
pub fn reset_global_degradation_log() {
    if let Ok(mut g) = GLOBAL_DEGRADATION_LOG.lock() {
        g.clear();
    }
    // total 不重置,保留"曾经降级过多少项"作为历史指标
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;

    #[tokio::test]
    async fn current_as_of_returns_none_outside_scope() {
        assert!(current_as_of().is_none());
        assert!(!is_asof_active());
    }

    #[tokio::test]
    async fn current_as_of_returns_value_inside_scope() {
        let date = NaiveDate::from_ymd_opt(2026, 6, 1).unwrap();
        let ctx = AsOfContext::new(date, AsOfSource::UserReplay).unwrap();
        let got = AS_OF.scope(Some(ctx), async { current_as_of() }).await;
        assert_eq!(got.unwrap().as_of_date, date);
        assert_eq!(got.unwrap().source, AsOfSource::UserReplay);
    }

    #[tokio::test]
    async fn validate_rejects_future_date() {
        let future = Local::now().date_naive() + Duration::days(7);
        let result = AsOfContext::new(future, AsOfSource::UserReplay);
        assert!(matches!(result, Err(AsOfError::FutureDate { .. })));
    }

    #[tokio::test]
    async fn validate_rejects_empty_string() {
        let result = AsOfContext::parse("");
        assert!(matches!(result, Err(AsOfError::InvalidFormat { .. })));
    }

    #[tokio::test]
    async fn validate_rejects_invalid_format() {
        let result = AsOfContext::parse("2026/06/01");
        assert!(matches!(result, Err(AsOfError::InvalidFormat { .. })));
    }

    #[tokio::test]
    async fn today_is_accepted() {
        let today = Local::now().date_naive();
        let ctx = AsOfContext::new(today, AsOfSource::UserReplay).unwrap();
        assert_eq!(ctx.as_of_date, today);
    }

    #[tokio::test]
    async fn parse_roundtrip() {
        let today = Local::now().date_naive();
        let s = today.format("%Y-%m-%d").to_string();
        let ctx = AsOfContext::parse(&s).unwrap();
        assert_eq!(ctx.as_string(), s);
    }

    #[test]
    fn parse_optional_none_is_live() {
        let r = AsOfContext::parse_optional(None).unwrap();
        assert!(r.is_none());
    }

    #[test]
    fn parse_optional_empty_is_live() {
        assert!(AsOfContext::parse_optional(Some("")).unwrap().is_none());
        assert!(AsOfContext::parse_optional(Some("   ")).unwrap().is_none());
    }

    #[test]
    fn parse_optional_past_date_is_replay() {
        let today = Local::now().date_naive();
        let past = today - Duration::days(7);
        let s = past.format("%Y-%m-%d").to_string();
        let r = AsOfContext::parse_optional(Some(&s)).unwrap();
        assert!(r.is_some());
        assert_eq!(r.unwrap().as_string(), s);
    }

    #[test]
    fn parse_optional_future_date_rejected() {
        let future = Local::now().date_naive() + Duration::days(7);
        let s = future.format("%Y-%m-%d").to_string();
        let r = AsOfContext::parse_optional(Some(&s));
        assert!(r.is_err());
        assert!(r.unwrap_err().contains("as_of_date 解析失败"));
    }

    #[test]
    fn parse_optional_invalid_format_rejected() {
        assert!(AsOfContext::parse_optional(Some("2026/06/01")).is_err());
        assert!(AsOfContext::parse_optional(Some("garbage")).is_err());
    }

    #[tokio::test]
    async fn cache_suffix_returns_live_outside_scope() {
        let s = cache_suffix();
        assert_eq!(s, "live");
    }

    #[tokio::test]
    async fn cache_suffix_includes_date_inside_scope() {
        let date = NaiveDate::from_ymd_opt(2026, 6, 1).unwrap();
        let ctx = AsOfContext::new(date, AsOfSource::UserReplay).unwrap();
        let s = AS_OF.scope(Some(ctx), async { cache_suffix() }).await;
        assert_eq!(s, "asof-20260601");
    }

    #[tokio::test]
    async fn nested_scope_uses_inner_value() {
        let outer_date = NaiveDate::from_ymd_opt(2026, 6, 1).unwrap();
        let inner_date = NaiveDate::from_ymd_opt(2026, 5, 1).unwrap();
        let outer = AsOfContext::new(outer_date, AsOfSource::UserReplay).unwrap();
        let inner = AsOfContext::new(inner_date, AsOfSource::UserReplay).unwrap();
        let result = AS_OF
            .scope(Some(outer), async {
                let outer_val = current_as_of().unwrap().as_of_date;
                let inner_val = AS_OF
                    .scope(Some(inner), async { current_as_of().unwrap().as_of_date })
                    .await;
                // 内层 scope 结束后，外层值恢复
                let after_inner = current_as_of().unwrap().as_of_date;
                (outer_val, inner_val, after_inner)
            })
            .await;
        assert_eq!(result, (outer_date, inner_date, outer_date));
    }

    // ── 降级日志(spec §4.1 统一降级协议) ────────────────────────
    // 实时性方法在 as-of 模式下跳过时，必须把降级原因写入日志供 workflow 消费

    #[tokio::test]
    async fn record_degradation_ignored_in_live_mode() {
        record_degradation("vendor", "method", "test");
        // live 模式下没有 task_local scope，take 返回空
        let report = take_asof_degradation_report();
        assert!(report.is_empty());
    }

    #[tokio::test]
    async fn record_degradation_captures_entries_in_asof_scope() {
        let date = NaiveDate::from_ymd_opt(2026, 6, 1).unwrap();
        let ctx = AsOfContext::new(date, AsOfSource::UserReplay).unwrap();
        // 正确用法：AS_OF.scope 包裹 with_degradation_log，workflow 节点调用入口
        AS_OF
            .scope(Some(ctx), async {
                with_degradation_log(async {
                    record_degradation("eastmoney", "get_hot_stocks", "no historical semantics");
                    record_degradation("tencent", "get_cls_flash", "future-only feed");
                    let report = take_asof_degradation_report();
                    assert_eq!(report.len(), 2);
                    assert_eq!(report[0].vendor, "eastmoney");
                    assert_eq!(report[0].as_of, "2026-06-01");
                    assert_eq!(report[1].method, "get_cls_flash");
                    // 消费后清空
                    let second = take_asof_degradation_report();
                    assert!(second.is_empty(), "take 应当清空日志");
                })
                .await
            })
            .await;
    }
}
