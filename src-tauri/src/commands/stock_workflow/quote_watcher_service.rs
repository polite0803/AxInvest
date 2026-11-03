// SPDX-License-Identifier: AGPL-3.0-only

//! `quote.watcher` 接缝的**宿主侧实现**（wiring 层，宿主独持 `RealTimeQuoteWatcher`）。
//!
//! 契约在 `axagent-core/crates/harness/src/quote_watcher.rs`（已冻结）：三个 op 全是**集合管理面**
//! —— 轮询本体（`start` 返回 `JoinHandle` 且不幂等、`stop` 翻全局闸）不过线，插件拿到它们
//! 等于把对 vendor 的请求数翻倍、或一句话让所有插件与前端一起失去行情推送。同样**不收 `as_of`**。
//!
//! ## 数量闸（裁定 J5）
//!
//! 两道闸的数字只引用 harness 的 [`WATCH_BATCH_LIMIT`] / [`WATCH_SET_LIMIT`]（本文件不写第二套）：
//! 集合是**所有插件与前端共享的一格**，无 owner 隔离 ⇒ 一次无上限的 `watch` 就能把取数配额打满。
//! 超限时**只截断不报错**，由 [`WatchOutcome`] 的 `requested` / `applied` 两个数如实暴露 ——
//! 只回一个 bool 就分不出「全成功 / 被截断 / 集合本来就这么大」。
//! 截断判定抽成纯函数 [`plan_watch_batch`]，判据直接打在这一格上，不必真起一个 watcher。
//!
//! ## `replace` 与 `set_priority`
//!
//! `replace = true` 先清空当前集合再写入，照命令层 `watch_stock_quotes` 那三行（先
//! `watched_stocks()` 再逐个 `unwatch`），引擎零改动。`set_priority` 不单发 op：它与 `watch`
//! 函数体同形（都是 `watches.insert(code, priority)`），插件重复 `watch` 覆盖同一只即可。
//!
//! ## `priority` 严格解析
//!
//! 命令层历史上写的是 `match … { "background" | "bg" => Background, _ => Active }` —— 拼错的串
//! **静默落到高频档**。接缝不继承这个宽容（插件侧没有「人看着面板选」这道兜底）：分派层用
//! `serde` 严格反序列化 `WatchPriority`，`"bg"` 一类未知串落 `SEAM_CALL_INVALID_ARGS`；本批
//! **不做**别名。trait 收到的已经是枚举，所以 provider 侧没有可宽容的字符串入口。
//!
//! ## 行情变化的第四跳
//!
//! 本接缝不新增流式 op，变化经 [`dispatch_quote_change`] 补发 `DomainEvent`（kind 只引用
//! harness 常量 [`STOCK_QUOTE_CHANGE_KIND`]，裁定 N6）。payload 是 `QuoteChangeEvent` 的
//! **序列化本体**，不摊平字段 —— 那份手工 `json!` 副本曾与前端消费端漂移，把面板打成整页错误。

use std::collections::HashSet;
use std::sync::Arc;

use async_trait::async_trait;
use axagent_astock_data::realtime_quote::{QuoteChangeEvent, RealTimeQuoteWatcher};
use axagent_harness::quote_watcher::{STOCK_QUOTE_CHANGE_KIND, WATCH_BATCH_LIMIT, WATCH_SET_LIMIT};
use axagent_harness::{
    AxAgentError, DispatchMode, DomainEvent, EventCategory, EventDispatchBus, QuoteWatcherProvider,
    WatchOutcome, WatchPriority,
};

/// `quote.watcher` 的宿主实现。
///
/// 与 `AppState.quote_watcher` **同一个 `Arc`**（构造点见 `init/services.rs`，注册在
/// `set(...)` 成功之后 —— 裁定 J8）。本 provider 不持 `AppHandle`，也不反向引用 AppState。
pub struct HostQuoteWatcher {
    watcher: Arc<RealTimeQuoteWatcher>,
}

impl HostQuoteWatcher {
    pub fn new(watcher: Arc<RealTimeQuoteWatcher>) -> Self {
        Self { watcher }
    }
}

/// 去重并保留**首次出现**的顺序。
///
/// 为什么必须去重：引擎的集合是 `HashMap` 键，同一只股票在一次调用里出现两次只会写入一次，
/// 若不去重就会让 `applied` 把「同一只」数成两只，`requested > applied` 变得没有意义。
fn dedup_preserve_order(codes: &[String]) -> Vec<String> {
    let mut seen: HashSet<&str> = HashSet::with_capacity(codes.len());
    let mut out: Vec<String> = Vec::with_capacity(codes.len());
    for code in codes {
        if seen.insert(code.as_str()) {
            out.push(code.clone());
        }
    }
    out
}

/// 两道数量闸的截断计划（纯函数，返回「要真正写入的代码」+ 两个计数）。
///
/// - `requested`：**截断前**的调用长度（含重复，插件要能看见自己传了多少）。
/// - 先按传入顺序去重，再依次受单次闸 [`WATCH_BATCH_LIMIT`] 与总量闸 [`WATCH_SET_LIMIT`]
///   约束（`existing` = 集合里已有的成员数；`replace` 已清空时传 0）。
/// - 两个闸取交集，**不报错**：`applied < requested` 就是被截断过。
fn plan_watch_batch(codes: &[String], existing: usize) -> (Vec<String>, WatchOutcome) {
    let requested = codes.len() as u32;
    let mut unique = dedup_preserve_order(codes);
    let room = (WATCH_SET_LIMIT as usize).saturating_sub(existing);
    let keep = unique.len().min(WATCH_BATCH_LIMIT as usize).min(room);
    unique.truncate(keep);
    let applied = keep as u32;
    (unique, WatchOutcome { requested, applied })
}

/// 推送集合的**升序**代码列表（引擎的集合是 `HashMap` 键 ⇒ 无序，裁定 J6：不排序则
/// 插件侧断言不可复现）。
fn sorted_codes(codes: Vec<String>) -> Vec<String> {
    let mut sorted = codes;
    sorted.sort();
    sorted
}

/// 把一条行情变化补发到插件总线（fire-and-forget，形态照 `workflow_run_service.rs` 的 `dispatch_event`）。
///
/// 无匹配订阅者时直接返回（`would_dispatch` 短路，避免白做 JSON 序列化与 spawn）。
/// 派发**一律不 await**：`DispatchMode::Emit` 是串行 await，把它接进行情回调就等于把
/// 插件 RTT 塞进宿主轮询的热路径。
pub fn dispatch_quote_change(dispatcher: &Arc<EventDispatchBus>, event: &QuoteChangeEvent) {
    let Ok(payload) = serde_json::to_value(event) else {
        tracing::warn!("[quote.watcher] 行情变化事件序列化失败，本次不外推");
        return;
    };
    let mut domain_event =
        DomainEvent::new(EventCategory::Stock, STOCK_QUOTE_CHANGE_KIND, payload, "quote.watcher");
    if !dispatcher.would_dispatch(&domain_event) {
        return;
    }
    let dispatcher = Arc::clone(dispatcher);
    tokio::spawn(async move {
        let outcome = dispatcher.dispatch(&mut domain_event, DispatchMode::Emit).await;
        tracing::trace!(
            invoked = outcome.invoked,
            rejected = outcome.rejected,
            "[quote.watcher] 行情变化事件已派发"
        );
    });
}

#[async_trait]
impl QuoteWatcherProvider for HostQuoteWatcher {
    async fn watched_codes(&self) -> Result<Vec<String>, AxAgentError> {
        Ok(sorted_codes(self.watcher.watched_stocks().await))
    }

    async fn watch(
        &self,
        codes: &[String],
        priority: WatchPriority,
        replace: bool,
    ) -> Result<WatchOutcome, AxAgentError> {
        // `replace` 与 `watch` 同体：先清空当前集合（照命令层那三行，引擎零改动）。
        if replace {
            let current = self.watcher.watched_stocks().await;
            for code in &current {
                self.watcher.unwatch(code).await;
            }
        }
        // 清空**之后**再读一次：总量闸的「已有」必须是集合的真实大小。
        // 写死 0 会在清空不彻底时多放行几只。
        let existing = self.watcher.watched_stocks().await.len();
        let (to_apply, outcome) = plan_watch_batch(codes, existing);
        for code in &to_apply {
            // 引擎的 `watch` 是 insert ⇒ 同一只重复调用即覆盖优先级（`set_priority` 合并的依据）
            self.watcher.watch(code, priority).await;
        }
        tracing::info!(
            "[quote.watcher] watch：请求 {} 只 / 生效 {} 只 / priority={:?} / replace={}",
            outcome.requested,
            outcome.applied,
            priority,
            replace
        );
        Ok(outcome)
    }

    async fn unwatch(&self, codes: &[String]) -> Result<WatchOutcome, AxAgentError> {
        let requested = codes.len() as u32;
        // `applied` 只数**真在集合里**的那几只：引擎对不存在的 code 静默成功，
        // 照实返回会把「本来就没盯」报成「摘掉了几只」。
        let current: HashSet<String> = self.watcher.watched_stocks().await.into_iter().collect();
        let mut applied = 0_u32;
        for code in codes {
            if current.contains(code.as_str()) {
                // 引擎的 unwatch 连带清掉上次快照缓存（`last_quotes`）⇒ 本层不另写一份
                self.watcher.unwatch(code).await;
                applied += 1;
            }
        }
        tracing::info!("[quote.watcher] unwatch：请求 {} 只 / 实际摘除 {} 只", requested, applied);
        Ok(WatchOutcome { requested, applied })
    }
}

#[cfg(test)]
mod tests {
    //! 数量闸与有序性的形态锁。
    //!
    //! `plan_watch_batch` / `sorted_codes` 刻意是纯函数：`RealTimeQuoteWatcher` 的构造要
    //! `Arc<AStockClient>`（vendor 客户端 + 配置），在单测里造它会把「闸算得对不对」这件事
    //! 拖进网络与配置依赖里。判据打在纯函数上，provider 侧只剩 plumbing。

    use super::*;
    use axagent_harness::market_data::StockQuote;

    fn codes(prefix: &str, n: usize) -> Vec<String> {
        (0..n).map(|i| format!("{prefix}{i:04}")).collect()
    }

    // ── 截断语义（裁定 J5）────────────────────────────────────────────

    #[test]
    fn under_both_gates_applies_everything() {
        let req = codes("6", 50);
        let (to_apply, outcome) = plan_watch_batch(&req, 0);
        assert_eq!(outcome.requested, 50);
        assert_eq!(outcome.applied, 50);
        assert_eq!(to_apply.len(), 50);
        // 截断按传入顺序，不引入任何重排
        assert_eq!(to_apply[0], "60000");
    }

    #[test]
    fn batch_gate_truncates_without_erroring() {
        let req = codes("6", 300);
        let (to_apply, outcome) = plan_watch_batch(&req, 0);
        assert_eq!(outcome.requested, 300);
        assert_eq!(outcome.applied, WATCH_BATCH_LIMIT);
        assert_eq!(to_apply.len(), WATCH_BATCH_LIMIT as usize);
        assert!(outcome.applied < outcome.requested, "截断必须暴露成 applied < requested");
    }

    #[test]
    fn set_gate_counts_existing_members() {
        // 集合已有 950 只 ⇒ 只剩 50 个额度，哪怕本次只请求 100 只
        let req = codes("0", 100);
        let (to_apply, outcome) = plan_watch_batch(&req, 950);
        assert_eq!(outcome.requested, 100);
        assert_eq!(outcome.applied, WATCH_SET_LIMIT - 950);
        assert_eq!(to_apply.len(), 50);
    }

    #[test]
    fn full_set_leaves_no_room_and_still_returns_ok() {
        let (to_apply, outcome) = plan_watch_batch(&codes("0", 10), WATCH_SET_LIMIT as usize);
        assert_eq!(outcome.requested, 10);
        assert_eq!(outcome.applied, 0, "集合已满 ⇒ 全部截断，但**不是错误**");
        assert!(to_apply.is_empty());
    }

    #[test]
    fn duplicates_count_once_in_applied_but_not_in_requested() {
        let mut req = codes("3", 3);
        req.push("30000".to_string());
        req.push("30001".to_string());
        let (to_apply, outcome) = plan_watch_batch(&req, 0);
        assert_eq!(outcome.requested, 5, "requested 是插件**提交**的数量，不替它美化");
        assert_eq!(outcome.applied, 3, "去重后只有 3 只会真的写入");
        assert_eq!(to_apply, vec!["30000", "30001", "30002"]);
    }

    /// 两闸同时生效时取更紧的那一个（单次闸 200 < 总量闸剩的 500）。
    #[test]
    fn tighter_gate_wins() {
        let (to_apply, outcome) = plan_watch_batch(&codes("6", 400), 100);
        assert_eq!(outcome.applied, WATCH_BATCH_LIMIT);
        assert_eq!(to_apply.len(), 200);
    }

    // ── 有序性（裁定 J6）──────────────────────────────────────────────

    #[test]
    fn watched_codes_are_sorted_ascending() {
        let sorted = sorted_codes(vec!["600519".into(), "000001".into(), "300750".into()]);
        assert_eq!(sorted, vec!["000001", "300750", "600519"]);
        // 空集合仍是空（不 panic、不返回 None）
        assert!(sorted_codes(Vec::new()).is_empty());
    }

    // ── priority 严格解析：不继承 `bg` 别名 ────────────────────────────

    /// 契约回归锁：接缝只认 `active` / `background`，命令层那个 `bg` 别名**不得**跟过来。
    #[test]
    fn priority_is_parsed_strictly_without_the_bg_alias() {
        for bad in ["bg", "BG", "active ", "high", ""] {
            assert!(
                serde_json::from_value::<WatchPriority>(serde_json::json!(bad)).is_err(),
                "别名 / 未知串 {bad:?} 必须解析失败（分派层据此落 SEAM_CALL_INVALID_ARGS）"
            );
        }
        assert_eq!(
            serde_json::from_value::<WatchPriority>(serde_json::json!("background"))
                .expect("background 是合法档位"),
            WatchPriority::Background
        );
    }

    // ── 行情事件的 kind 与 payload 形状 ────────────────────────────────

    #[test]
    fn quote_change_kind_uses_the_stock_category_prefix() {
        // kind 串的唯一权威是 harness 常量（裁定 N6）；本文件**不许**出现字面量
        assert!(
            STOCK_QUOTE_CHANGE_KIND.starts_with(EventCategory::Stock.as_str()),
            "kind {STOCK_QUOTE_CHANGE_KIND} 的前缀必须是 stock 档位"
        );
    }

    #[test]
    fn dispatch_payload_is_the_quote_event_itself() {
        // payload 不摊平：`current.price` 在 `current` 下面（那份手工 json! 副本漂移过一次，
        // 直接把前端面板打成整页错误 —— 见 realtime_quote.rs 的 QuoteChangeEvent 文档）
        let event = QuoteChangeEvent {
            stock_code: "600519".to_string(),
            previous: None,
            current: quote_of(1_400.0),
            change_pct: 2.83,
            trigger: "price_change".to_string(),
        };
        let payload = serde_json::to_value(&event).expect("QuoteChangeEvent 必须可序列化");
        assert_eq!(payload["stockCode"], serde_json::json!("600519"));
        assert_eq!(payload["current"]["price"], serde_json::json!(1_400.0));
        assert!(payload.get("price").is_none(), "不得出现被摊平的顶层 price 键");
        assert!(payload.get("current").is_some());
    }

    fn quote_of(price: f64) -> StockQuote {
        StockQuote {
            code: "600519".to_string(),
            name: "贵州茅台".to_string(),
            price,
            pre_close: 1_360.0,
            open: 1_365.0,
            high: 1_410.0,
            low: 1_360.0,
            volume: 30_000.0,
            amount: 4.2e8,
            change_pct: 2.83,
            turnover_rate: 0.24,
            pe: Some(30.0),
            pb: Some(9.0),
            total_mv: Some(1.8e12),
            circulating_mv: Some(1.8e12),
            limit_up: Some(1_496.0),
            limit_down: Some(1_224.0),
            is_st: false,
            timestamp: String::new(),
        }
    }
}
