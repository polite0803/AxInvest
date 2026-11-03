// SPDX-License-Identifier: AGPL-3.0-only

//! 实时行情推送集合接缝（`quote.watcher`）— DTO + Trait。
//!
//! 宿主把 `astock-data` 的 `RealTimeQuoteWatcher` 的**推送集合管理面**经本接缝暴露给插件侧；
//! 插件经 `call_seam("quote.watcher", op, args)` 读当前监听列表、加监、退监。
//!
//! ## 主权边界：轮询本体不过线
//!
//! - `start()` 返回 `tokio::task::JoinHandle`（句柄跨不过帧协议），且**不幂等** —— 每次调用
//!   都 spawn 一条新循环（无 `running` 前置检查）⇒ 插件拿到句柄或反复调它，等于把对 vendor
//!   的取数请求翻倍。
//! - `stop()` 只翻一个全局 `AtomicBool` ⇒ 一句话让**所有**插件与前端同时失去行情推送，
//!   属跨插件 DoS 面。
//! - 行情变化推送走 `event.dispatch`（宿主在 `QuoteCallback` 内补发 `DomainEvent`），
//!   **不新增流式 op**。
//!
//! 本 watcher 的 `client` 是**具体类型** `Arc<AStockClient>` 而非 `Arc<dyn MarketDataProvider>`
//! ⇒ 它与内置取数天生绑定，「插件提供行情源」这条路推不动它（那走 `market.quote`）。
//!
//! ## 上收的类型
//!
//! [`WatchPriority`] 原先定义在 `axagent-quant/crates/astock-data/src/realtime_quote.rs` 且
//! **完全没有 serde**（只有 `Debug, Clone, Copy, PartialEq, Eq, Hash`）⇒ 接缝只能收裸串。
//! 按 AGENTS 禁区 12「已有定义 → 必须复用」，定义上收本模块并补
//! `Serialize, Deserialize` + `#[serde(rename_all = "snake_case")]`，原处改
//! `pub use axagent_harness::WatchPriority`；它的档位间隔 `interval_ms` 随类型一起上收
//! （Rust 不允许在别的 crate 给 foreign 类型写 inherent impl）。
//! 线上字面量就是命令层现用的那两个串（`"active"` / `"background"`），改注解零风险。
//!
//! ## 写面权限
//!
//! `watch` / `unwatch` 要求插件已声明 `write` 权限（由 worker 分派层强制，拒绝码
//! `PLUGIN_PERMISSION_DENIED`）；`list_watched` 是读面。本层只提供契约，不重复校验。
//!
//! ## `as_of`
//!
//! 三个 op 全部读写进程内推送集合，不取行情 ⇒ **一律不收 `as_of`**。

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::core_error::{AxAgentError, Result};

/// 单次 `watch` 调用可提交的代码数上限（`unwatch` **不**受它约束 —— 退订只会让集合变小，
/// 放大的是取数这一侧才需要闸）。
///
/// 数量闸的理由：`watch` 直接放大对 vendor 的请求数（集合里每一只都在被轮询），且集合是
/// **所有插件与前端共享的一格**，没有 owner 隔离 ⇒ 无上限的一次调用就能把取数配额打满。
pub const WATCH_BATCH_LIMIT: u32 = 200;
/// 推送集合的总量上限（跨调用累计，含既有成员）。
pub const WATCH_SET_LIMIT: u32 = 1000;

// 两个闸的相对关系是编译期不变量：单次闸若大于总量闸，截断语义就自相矛盾。
const _: () = assert!(WATCH_BATCH_LIMIT <= WATCH_SET_LIMIT);

/// 监控优先级（上收自 `astock-data`，本层补 serde）。
///
/// ⚠ 接缝侧**严格解析**：未知串 ⇒ `SEAM_CALL_INVALID_ARGS`。命令层现在是
/// `match … { "background" | "bg" => Background, _ => Active }` —— 拼错的串静默落到高频档，
/// 这个宽容**不得继承**到接缝（插件侧没有「人看着面板选」这道兜底）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WatchPriority {
    /// 主动监控（~2s 轮询）
    Active,
    /// 背景监控（~10s 轮询）
    Background,
}

impl WatchPriority {
    /// 该档位的轮询间隔（毫秒）。随类型一起上收：inherent impl 必须住在定义该类型的 crate。
    ///
    /// 数值与 `astock-data` 改造前的私有实现逐字相同 —— 它是宿主轮询循环唯一的节流来源，
    /// 不是给插件写的读面。
    #[must_use]
    pub fn interval_ms(self) -> u64 {
        match self {
            WatchPriority::Active => 2000,
            WatchPriority::Background => 10000,
        }
    }
}

/// `watch` / `unwatch` 的结果：**两个计数并列**，超限只截断不报错。
///
/// 为什么是两个数而不是 bool：集合总量有闸（[`WATCH_SET_LIMIT`]），单次调用也有闸
/// （[`WATCH_BATCH_LIMIT`]），超限时**只应用能应用的那部分**。只回一个 bool 就分不出
/// 「全成功」/「被截断」/「集合本来就这么大」。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WatchOutcome {
    /// 本次调用**请求**的代码数（截断前）。
    pub requested: u32,
    /// 实际生效的数量（`unwatch` 指真从集合里摘掉的数量 —— 不存在的 code 静默跳过）。
    /// `applied < requested` 即本次被数量闸截断过，消费方须如实提示而不是当全成功。
    pub applied: u32,
}

/// 实时行情推送集合提供者（宿主独占 `RealTimeQuoteWatcher`）。
///
/// 默认实现统一返回「该提供者不支持此数据面」，便于外部实现只覆盖关心的 op。
#[async_trait]
pub trait QuoteWatcherProvider: Send + Sync {
    /// 当前监听的股票代码集合（`list_watched` op，读面），返回
    /// `{ "codes": [...] }` 的具名对象而不是裸数组。
    ///
    /// 引擎的集合是 `HashMap` 键 ⇒ **无序**；实现方须 `sort` 后再返回，否则插件侧断言不可复现。
    /// 首批**不带 priority**（引擎只给键；要带就得加一个 `watched_with_priority`，
    /// 属可顺手做但不应阻塞本接缝的改动）。
    async fn watched_codes(&self) -> Result<Vec<String>> {
        Err(AxAgentError::Provider(
            "QuoteWatcherProvider::watched_codes 未实现（该提供者不支持此数据面）".into(),
        ))
    }

    /// 加入监听（`watch` op，**写**，要求 `write` 权限）。
    ///
    /// - `priority` 严格解析，未知串 ⇒ `SEAM_CALL_INVALID_ARGS`（见 [`WatchPriority`]）。
    /// - `replace = true` 先清空当前集合再写入（照命令层那两行「先 `watched_stocks()`
    ///   再逐个 `unwatch`」，引擎零改动）；`replace` 与 `watch` 同体 ⇒ `set_priority`
    ///   不单发一个 op，插件重复调用 `watch` 覆盖同一只即可。
    /// - 数量闸：单次 ≤ [`WATCH_BATCH_LIMIT`]、集合总量 ≤ [`WATCH_SET_LIMIT`]，超限**截断
    ///   不报错**，由 [`WatchOutcome`] 的 `requested` / `applied` 两个数如实暴露。
    async fn watch(
        &self,
        _codes: &[String],
        _priority: WatchPriority,
        _replace: bool,
    ) -> Result<WatchOutcome> {
        Err(AxAgentError::Provider(
            "QuoteWatcherProvider::watch 未实现（该提供者不支持此数据面）".into(),
        ))
    }

    /// 移出监听（`unwatch` op，**写**，要求 `write` 权限）。
    ///
    /// 引擎对不存在的 code 静默成功 ⇒ `applied` 只数真正摘掉的；代码集合同时也要从
    /// 宿主的上次快照缓存里清掉（引擎 `unwatch` 已连带清，实现方不要另写一份）。
    ///
    /// **不受 [`WATCH_BATCH_LIMIT`] 约束**：退订只让集合变小，闸要防的是取数放大那一侧。
    async fn unwatch(&self, _codes: &[String]) -> Result<WatchOutcome> {
        Err(AxAgentError::Provider(
            "QuoteWatcherProvider::unwatch 未实现（该提供者不支持此数据面）".into(),
        ))
    }
}

// ── 事件 kind（裁定 N6：发射点只准引用常量，禁止再抄一遍字面量）───────────

/// 行情变化推送的 `DomainEvent.kind`。
///
/// 本接缝**不新增流式 op**：轮询本体留在宿主，变化经 `event.dispatch` 推给订阅者，用的就是
/// 这一串。前缀必须等于 `EventCategory::Stock::as_str()`（由本模块单测在运行期钉住 ——
/// `&str` 常量拼不出方法调用的结果，而把前缀抄成字面量正是本裁定要消灭的那件事）。
///
/// ⚠ payload 是 `astock_data::QuoteChangeEvent` 的 camelCase 序列化本体（与前端
/// `stock-quote-update` 同一份真相），宿主侧**不得**再摊平一遍字段 —— 那份手工 `json!`
/// 副本漂移过一次，直接把面板打成整页错误。
pub const STOCK_QUOTE_CHANGE_KIND: &str = "stock/quote_change";

// ── 测试 ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn watch_priority_snake_case_literals() {
        // 线上字面量必须与命令层现用的串逐字相同
        assert_eq!(
            serde_json::to_value(WatchPriority::Active).expect("serialize"),
            json!("active")
        );
        assert_eq!(
            serde_json::to_value(WatchPriority::Background).expect("serialize"),
            json!("background")
        );
        let back: WatchPriority = serde_json::from_value(json!("background")).expect("反序列化");
        assert_eq!(back, WatchPriority::Background);
        // 未知串（含 `bg` 这种命令层会宽容回落的写法）一律 reject —— 严格解析是契约的一部分
        for bad in ["bg", "ACTIVE", "high", ""] {
            assert!(
                serde_json::from_value::<WatchPriority>(json!(bad)).is_err(),
                "未知串 {bad} 必须被拒绝"
            );
        }
    }

    #[test]
    fn watch_priority_intervals_match_engine() {
        assert_eq!(WatchPriority::Active.interval_ms(), 2000);
        assert_eq!(WatchPriority::Background.interval_ms(), 10000);
    }

    #[test]
    fn watch_outcome_camel() {
        let outcome = WatchOutcome { requested: 300, applied: 200 };
        let val = serde_json::to_value(outcome).expect("序列化应成功");
        assert_eq!(val["requested"], json!(300));
        assert_eq!(val["applied"], json!(200));
        assert!(val.get("requested").is_some());
        let back: WatchOutcome = serde_json::from_value(val).expect("往返应成立");
        assert_eq!(back, outcome);
        // 截断不报错：契约要求 applied < requested 是合法结果，不是 Err
        assert!(back.applied < back.requested);
    }

    #[test]
    fn quantity_gates_are_contract_numbers() {
        // 数量闸住在契约层：宿主与分派两侧都按这两个数 clamp，不得各自再写一份
        // （两者的相对关系由常量下方的 `const _: () = assert!(…)` 在编译期钉住）
        assert_eq!(WATCH_BATCH_LIMIT, 200);
        assert_eq!(WATCH_SET_LIMIT, 1000);
    }

    /// 未覆盖任何方法的实现：三个 op 全部走默认实现 ⇒ 一律 `Err(Provider)`。
    struct BareWatcher;
    impl QuoteWatcherProvider for BareWatcher {}

    #[test]
    fn default_methods_unimplemented() {
        let watcher = BareWatcher;
        let codes: Vec<String> = vec!["600519".into()];
        let checks: Vec<(&str, Result<serde_json::Value>)> = vec![
            (
                "watched_codes",
                futures::executor::block_on(watcher.watched_codes()).map(|v| json!(v)),
            ),
            (
                "watch",
                futures::executor::block_on(watcher.watch(&codes, WatchPriority::Active, false))
                    .map(|v| json!(v)),
            ),
            ("unwatch", futures::executor::block_on(watcher.unwatch(&codes)).map(|v| json!(v))),
        ];
        assert_eq!(checks.len(), 3);
        for (op, outcome) in checks {
            let err = outcome.expect_err("默认实现必须返回 Err");
            match err {
                AxAgentError::Provider(msg) => {
                    assert!(msg.contains("QuoteWatcherProvider::"), "文案须点明 trait：{msg}");
                    assert!(msg.contains(op), "文案须点明 op {op}：{msg}");
                },
                other => panic!("{op} 的默认实现应返回 AxAgentError::Provider，实际 {other:?}"),
            }
        }
    }

    #[test]
    fn trait_is_object_safe() {
        let provider: std::sync::Arc<dyn QuoteWatcherProvider> = std::sync::Arc::new(BareWatcher);
        assert!(futures::executor::block_on(provider.watched_codes()).is_err());
    }

    /// 裁定 N6 的形态锁：行情 kind 的前缀必须等于 `EventCategory::Stock::as_str()`，
    /// 且与 `stock_monitor.rs` 那两条同源（三条串都挂在同一个 category 名下，插件按前缀筛）。
    #[test]
    fn stock_quote_change_kind_prefix_matches_category() {
        use crate::EventCategory;

        let prefix = format!("{}/", EventCategory::Stock.as_str());
        assert!(
            STOCK_QUOTE_CHANGE_KIND.starts_with(&prefix),
            "kind `{STOCK_QUOTE_CHANGE_KIND}` 的前缀不是 `{prefix}`"
        );
        assert!(
            STOCK_QUOTE_CHANGE_KIND.len() > prefix.len(),
            "kind 只有前缀 ⇒ 订阅端会当成空事件名"
        );
        assert_ne!(STOCK_QUOTE_CHANGE_KIND, crate::stock_monitor::STOCK_PRICE_ALERT_KIND);
        assert_ne!(STOCK_QUOTE_CHANGE_KIND, crate::stock_monitor::STOCK_PORTFOLIO_SIGNAL_KIND);
    }
}
