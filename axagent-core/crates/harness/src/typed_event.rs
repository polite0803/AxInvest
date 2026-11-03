// SPDX-License-Identifier: AGPL-3.0-only

//! 类型化事件派发总线 —— P2 事件化（缺陷 #3）地基本体。
//!
//! 在统一广播总线（`event_bus.rs`）之上，新增**有裁决语义**的派发机制：
//! 订阅者按 [`DispatchMode`] 被调用，可阅读 / 改写 / 拒绝事件，支撑把策略、
//! 审批、遥测等横切逻辑剥离为独立插件（经能力注册表 `event.dispatch` 接缝挂载）。
//!
//! 与 `event_bus.rs`（广播 / stream 订阅）的区别：本模块是**命令式派发**，
//! 订阅者以 `Arc<dyn EventSubscriber>` 回调形式注册，可对事件做改写 / 拒绝，
//! 而非被动接收拷贝。二者互补：广播用于"观测"，派发用于"拦截/改写"。
//!
//! ## 四派发模式
//!
//! | 模式 | 语义 | 用途 |
//! |------|------|------|
//! | [`DispatchMode::Emit`] | 广播，订阅者只读 | 遥测 / 审计 |
//! | [`DispatchMode::Waterfall`] | 按注册顺序，订阅者可改写 payload 或拒绝（中断） | 策略 / 审批（`agent/request`） |
//! | [`DispatchMode::Parallel`] | 订阅者并发调用（只读） | 并行观测 |
//! | [`DispatchMode::Serial`] | 按注册顺序调用（只读） | 顺序观测 |
//!
//! harness 为 foundation 层，禁止依赖任何 axagent-* crate；事件统一用
//! [`DomainEvent`]（category + kind + payload 三元组），订阅者经
//! [`EventMatcher`] 按 category / kind 过滤。

use crate::EffectScope;
use crate::event_bus::{DomainEvent, EventCategory};
use crate::reversible_effect::EffectHandle;
use async_trait::async_trait;
use parking_lot::RwLock;
use serde_json::Value;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

/// 派发模式。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DispatchMode {
    /// 广播：订阅者只读，互不影响。
    Emit,
    /// 瀑布：按注册顺序调用，订阅者可改写 payload 或拒绝（中断后续）。
    Waterfall,
    /// 并行：订阅者并发调用（只读），全部完成后汇总。
    Parallel,
    /// 串行：按注册顺序调用（只读）。
    Serial,
}

/// 订阅者裁决。
#[derive(Debug, Clone, PartialEq)]
pub enum SubscriberVerdict {
    /// 继续，不改写。
    Continue,
    /// 改写 payload 后继续（仅 [`DispatchMode::Waterfall`] 生效）。
    Rewrite(Value),
    /// 拒绝并中断处理链（仅 [`DispatchMode::Waterfall`] 生效）。
    Reject,
}

/// 事件匹配器：按来源分类 + 类型字符串过滤订阅者。
///
/// `None` 表示不限制该维度（匹配全部）。
#[derive(Debug, Clone)]
pub struct EventMatcher {
    /// 事件来源分类；`None` 匹配所有分类。
    pub category: Option<EventCategory>,
    /// 事件类型字符串（如 `"agent/request"`、`"llm/stream"`）；`None` 匹配所有类型。
    pub kind: Option<String>,
}

impl EventMatcher {
    /// 匹配所有事件。
    pub fn any() -> Self {
        Self { category: None, kind: None }
    }

    /// 仅匹配某来源分类。
    pub fn category(category: EventCategory) -> Self {
        Self { category: Some(category), kind: None }
    }

    /// 仅匹配某事件类型字符串。
    pub fn kind(kind: impl Into<String>) -> Self {
        Self { category: None, kind: Some(kind.into()) }
    }

    /// 判断事件是否匹配本订阅过滤器。
    pub fn matches(&self, event: &DomainEvent) -> bool {
        if let Some(c) = self.category
            && c != event.category
        {
            return false;
        }
        if let Some(k) = &self.kind
            && *k != event.kind
        {
            return false;
        }
        true
    }
}

/// 类型化事件订阅者。
///
/// 事件以只读 `&DomainEvent` 传入（支持并发共享）；需要改写 / 拒绝时
/// 通过 [`SubscriberVerdict`] 返回值声明，由总线统一应用。
#[async_trait]
pub trait EventSubscriber: Send + Sync + 'static {
    /// 处理一个事件并返回裁决。
    async fn handle(&self, event: &DomainEvent) -> SubscriberVerdict;
}

/// 一次派发的汇总结果。
#[derive(Debug, Clone, Default)]
pub struct DispatchOutcome {
    /// Waterfall 模式下被订阅者改写的最终 payload（未改写则为 `None`）。
    pub rewritten: Option<Value>,
    /// 是否被订阅者拒绝（Waterfall 中断）。
    pub rejected: bool,
    /// 实际**进入**回调的订阅者数（含超时未返回的，见 [`Self::timed_out`]）。
    pub invoked: usize,
    /// 其中超过宿主兜底预算、未拿到裁决的订阅者数。
    ///
    /// 计数口径要说清：本字段是「**总线这一层放弃等待**」的条数，不是「事件没送到插件」。
    /// 插件侧真正的投递失败由 `plugins::worker` 的投递计数负责（内层帧预算先到点时，
    /// 总线只看到一个正常返回 `Continue` 的订阅者 ⇒ 计入 [`Self::invoked`]，不计入本数）。
    pub timed_out: usize,
}

/// 一条订阅的宿主侧计数（B3「丢弃计数」在总线层的落点）。
///
/// 三个量是**单调不减**的 `AtomicU64`，不是一组瞬时布尔：一条订阅在生命周期里会
/// 被派发很多次，判据要能回答「这条订阅一共吞了多少次」。
/// `entered == completed + timed_out` 恒成立（panic 不在本机制管辖内，见 [`DispatchOutcome::timed_out`] 注）。
#[derive(Debug, Default)]
struct SubscriptionCounters {
    entered: AtomicU64,
    completed: AtomicU64,
    timed_out: AtomicU64,
}

/// 一条订阅的计数快照（供诊断面读出，不含内部原子类型）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubscriberStats {
    /// 订阅 ID（[`EventDispatchBus::subscribe`] 分配的单调序号）。
    pub id: u64,
    /// 归因标签：命名订阅取注册方给的标签（插件事件桥用 plugin_id），
    /// 未命名取 `sub-{id}` —— 读口必须能指出**是谁**在吞事件，否则计数没有处置价值。
    pub label: String,
    /// 被进入回调的总次数。
    pub entered: u64,
    /// 拿到裁决的次数。
    pub completed: u64,
    /// 超过宿主兜底预算、被总线放弃的次数。
    pub timed_out: u64,
}

struct Subscription {
    id: u64,
    label: String,
    matcher: EventMatcher,
    subscriber: Arc<dyn EventSubscriber>,
    counters: Arc<SubscriptionCounters>,
}

struct Inner {
    next_id: u64,
    /// `Arc` 包装是**必需的**，不只是风格：派发快照要带着同一条订阅的计数器写回，
    /// 而快照期间该订阅可能已被撤销句柄从 `Vec` 摘除 —— 摘除后在途派发仍要能把它
    /// 的 `completed` / `timed_out` 记完，否则计数对不上账（`entered` 已加）。
    subscriptions: Vec<Arc<Subscription>>,
}

/// 宿主对**单个订阅者**一次回调的兜底等待预算（毫秒）。
///
/// 定位是「兜底」，不是 SLA：它只保证一条订阅者永远不会把一次派发挂死
/// （B3 的判据前提 —— 派发点是 `spawn` 出去的，挂死的订阅者会**堆积任务**而不是
/// 报错，光看日志看不出漏了多少）。
///
/// ⚠ 取值必须**晚于任何内层预算**，否则总线先放弃、订阅者还在等 ⇒ 计数把「正常返回」
/// 与「被吞掉」混为一谈。盘上已知的内层预算上限是 plugin-proto 的
/// `DEFAULT_SEAM_TIMEOUT_MS` = 15 s（插件事件帧），且任务 #73 M-3 之后
/// 「嵌套 `call_seam` 期间计时挂起」⇒ 一条事件完全可能合法地跑过 15 s。
/// 故本值取 60 s（与宿主既有 `tool.set` 通道 `TOOL_SET_EXEC_TIMEOUT_MS` 同源的那个 60 s 量级），
/// **不要**把它当成「事件应在 60 s 内处理完」的承诺。
/// 更紧的预算属于订阅者自己的职责（插件桥在内层自带上限并计入投递失败），
/// 不是靠这里调小来凑。
pub const DEFAULT_SUBSCRIBER_TIMEOUT_MS: u64 = 60_000;

/// 类型化事件派发总线。
///
/// 维护订阅者集合，支持四派发模式；订阅返回可撤销 [`EffectHandle`]，
/// 撤销后不再参与后续派发（供插件挂载 / 卸载热插拔）。
#[derive(Clone)]
pub struct EventDispatchBus {
    inner: Arc<RwLock<Inner>>,
    effects: EffectScope,
    subscriber_timeout: Duration,
}

impl Default for EventDispatchBus {
    fn default() -> Self {
        Self::new()
    }
}

impl EventDispatchBus {
    /// 构造一个空总线（每订阅者兜底预算取 [`DEFAULT_SUBSCRIBER_TIMEOUT_MS`]）。
    pub fn new() -> Self {
        Self::with_subscriber_timeout(Duration::from_millis(DEFAULT_SUBSCRIBER_TIMEOUT_MS))
    }

    /// 指定每订阅者兜底预算构造 —— 面向测试与「宿主明确知道某条链更紧」的场景。
    pub fn with_subscriber_timeout(subscriber_timeout: Duration) -> Self {
        Self {
            inner: Arc::new(RwLock::new(Inner { next_id: 0, subscriptions: Vec::new() })),
            effects: EffectScope::new(),
            subscriber_timeout,
        }
    }

    /// 注册一个订阅者，返回可撤销句柄（归因标签自动取 `sub-{id}`）。
    ///
    /// 撤销后该订阅者即从总线移除，不再收到后续事件。
    pub fn subscribe(
        &self,
        matcher: EventMatcher,
        subscriber: Arc<dyn EventSubscriber>,
    ) -> EffectHandle {
        self.subscribe_named("", matcher, subscriber)
    }

    /// 注册一个**带归因标签**的订阅者（标签建议写「谁」，如 plugin_id）。
    ///
    /// `label` 为空串 ⇒ 退化为 `sub-{id}`。标签只进 [`Self::subscriber_stats`]，
    /// 不参与匹配判定。
    pub fn subscribe_named(
        &self,
        label: &str,
        matcher: EventMatcher,
        subscriber: Arc<dyn EventSubscriber>,
    ) -> EffectHandle {
        let counters = Arc::new(SubscriptionCounters::default());
        let id = {
            let mut g = self.inner.write();
            let id = g.next_id;
            g.next_id += 1;
            let label = if label.trim().is_empty() {
                format!("sub-{id}")
            } else {
                label.trim().to_string()
            };
            g.subscriptions.push(Arc::new(Subscription {
                id,
                label,
                matcher,
                subscriber,
                counters,
            }));
            id
        };
        let inner = self.inner.clone();
        self.effects.register(format!("event-subscriber:{id}"), move || {
            let mut g = inner.write();
            g.subscriptions.retain(|s| s.id != id);
        })
    }

    /// 当前订阅者数量（含未匹配当前事件的）。
    pub fn subscriber_count(&self) -> usize {
        self.inner.read().subscriptions.len()
    }

    /// 每条订阅的计数快照，按注册顺序。
    ///
    /// 这是「丢弃计数」的读口：诊断面据此把「插件吞事件」定位到具体订阅者。
    /// 快照不是原子整体 —— 取的时候派发可能正在推进，故只保证**单调不减**，
    /// 不保证三条计数在同一瞬间自洽（调用方按增量读，不要按「本次派发刚好 +1」断言）。
    #[must_use]
    pub fn subscriber_stats(&self) -> Vec<SubscriberStats> {
        self.inner
            .read()
            .subscriptions
            .iter()
            .map(|s| SubscriberStats {
                id: s.id,
                label: s.label.clone(),
                entered: s.counters.entered.load(Ordering::Acquire),
                completed: s.counters.completed.load(Ordering::Acquire),
                timed_out: s.counters.timed_out.load(Ordering::Acquire),
            })
            .collect()
    }

    /// 是否有订阅者可能匹配该事件（热路径读取前的廉价短路）。
    ///
    /// 用于流式路径：构造/序列化 payload 前先判断有没有匹配订阅者，
    /// 避免无订阅时仍为每个 chunk 做 JSON 序列化的开销。
    pub fn would_dispatch(&self, event: &DomainEvent) -> bool {
        self.inner.read().subscriptions.iter().any(|s| s.matcher.matches(event))
    }

    /// 调用一条订阅并套兜底预算；返回 `None` = 超时被放弃。
    ///
    /// 计数在**进入前**加 `entered`，返回后二选一加 `completed` / `timed_out`，
    /// 于是 `entered == completed + timed_out` 是可对账的。
    async fn invoke_one(
        sub: &Arc<Subscription>,
        event: &DomainEvent,
        budget: Duration,
    ) -> Option<SubscriberVerdict> {
        sub.counters.entered.fetch_add(1, Ordering::AcqRel);
        match tokio::time::timeout(budget, sub.subscriber.handle(event)).await {
            Ok(verdict) => {
                sub.counters.completed.fetch_add(1, Ordering::AcqRel);
                Some(verdict)
            },
            Err(_) => {
                sub.counters.timed_out.fetch_add(1, Ordering::AcqRel);
                tracing::warn!(
                    subscriber = %sub.label,
                    timeout_ms = budget.as_millis(),
                    "订阅者超过宿主兜底预算未返回裁决，本次派发按放弃处理"
                );
                None
            },
        }
    }

    /// 按派发模式派发一个事件。
    ///
    /// 仅匹配的订阅者参与回调；无匹配订阅者时优雅返回（不 panic）。
    ///
    /// ⚠ **超时不中断链**：Waterfall 里一条订阅超时 ⇒ 按 `Continue` 处理，继续问下一条
    /// （与插件桥同口径：「不因插件异常阻断宿主」）。也就是说**超时永远不会让你的事件
    /// 被拒**，只会让它少一道审批 —— 需要「订阅者必须表态否则不放行」的场景，
    /// 不该挂在水流模式上。
    pub async fn dispatch(&self, event: &mut DomainEvent, mode: DispatchMode) -> DispatchOutcome {
        let subs: Vec<Arc<Subscription>> = {
            let g = self.inner.read();
            g.subscriptions.iter().filter(|s| s.matcher.matches(event)).cloned().collect()
        };
        let budget = self.subscriber_timeout;
        let mut outcome = DispatchOutcome::default();
        match mode {
            DispatchMode::Emit | DispatchMode::Serial => {
                for s in &subs {
                    if Self::invoke_one(s, event, budget).await.is_none() {
                        outcome.timed_out += 1;
                    }
                    outcome.invoked += 1;
                }
            },
            DispatchMode::Waterfall => {
                for s in &subs {
                    match Self::invoke_one(s, event, budget).await {
                        // 超时：不中断瀑布，但**必须计入 timed_out** —— 漏了这一支，
                        // 调用方只看 outcome 时「插件装死」与「插件表态继续」就无法区分
                        // （本仓库第一轮单测正是这样抓出来的）。
                        None => outcome.timed_out += 1,
                        Some(SubscriberVerdict::Continue) => {},
                        Some(SubscriberVerdict::Rewrite(payload)) => {
                            event.payload = payload;
                            outcome.rewritten = Some(event.payload.clone());
                        },
                        Some(SubscriberVerdict::Reject) => {
                            outcome.rejected = true;
                            outcome.invoked += 1;
                            break;
                        },
                    }
                    outcome.invoked += 1;
                }
            },
            DispatchMode::Parallel => {
                let event_ref: &DomainEvent = event;
                let results = futures::future::join_all(
                    subs.iter().map(|s| Self::invoke_one(s, event_ref, budget)),
                )
                .await;
                outcome.invoked = results.len();
                outcome.timed_out = results.iter().filter(|r| r.is_none()).count();
            },
        }
        outcome
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::ChatContent;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// 测试订阅者：按返回策略响应事件。
    struct VerdictSub {
        verdict: SubscriberVerdict,
        calls: Arc<AtomicUsize>,
    }

    #[async_trait]
    impl EventSubscriber for VerdictSub {
        async fn handle(&self, _event: &DomainEvent) -> SubscriberVerdict {
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.verdict.clone()
        }
    }

    fn event() -> DomainEvent {
        DomainEvent::new(EventCategory::Agent, "agent/request", serde_json::json!({"q": 1}), "test")
    }

    fn sub(verdict: SubscriberVerdict) -> Arc<dyn EventSubscriber> {
        Arc::new(VerdictSub { verdict, calls: Arc::new(AtomicUsize::new(0)) })
    }

    #[tokio::test]
    async fn emit_is_readonly_and_invokes_all() {
        let bus = EventDispatchBus::new();
        bus.subscribe(EventMatcher::any(), sub(SubscriberVerdict::Continue));
        let mut e = event();
        let outcome = bus.dispatch(&mut e, DispatchMode::Emit).await;
        assert_eq!(outcome.invoked, 1);
        assert!(!outcome.rejected);
        assert!(outcome.rewritten.is_none());
    }

    #[tokio::test]
    async fn waterfall_rejects_and_stops() {
        let bus = EventDispatchBus::new();
        let _h1 = bus.subscribe(EventMatcher::any(), sub(SubscriberVerdict::Continue));
        let _h2 = bus.subscribe(EventMatcher::any(), sub(SubscriberVerdict::Reject));
        let _h3 = bus.subscribe(EventMatcher::any(), sub(SubscriberVerdict::Continue));
        let mut e = event();
        let outcome = bus.dispatch(&mut e, DispatchMode::Waterfall).await;
        assert!(outcome.rejected);
        assert_eq!(outcome.invoked, 2, "第二个订阅者拒绝后中断，第三个不再被调用");
    }

    #[tokio::test]
    async fn waterfall_rewrites_payload() {
        let bus = EventDispatchBus::new();
        let _h1 = bus.subscribe(
            EventMatcher::any(),
            sub(SubscriberVerdict::Rewrite(serde_json::json!({"q": 42}))),
        );
        let mut e = event();
        let outcome = bus.dispatch(&mut e, DispatchMode::Waterfall).await;
        assert_eq!(outcome.rewritten.as_ref(), Some(&serde_json::json!({"q": 42})));
        assert_eq!(e.payload, serde_json::json!({"q": 42}));
    }

    #[tokio::test]
    async fn matcher_filters_by_kind() {
        let bus = EventDispatchBus::new();
        let _h = bus.subscribe(EventMatcher::kind("llm/stream"), sub(SubscriberVerdict::Continue));
        let mut e = event(); // kind = "agent/request"
        let outcome = bus.dispatch(&mut e, DispatchMode::Emit).await;
        assert_eq!(outcome.invoked, 0, "kind 不匹配的订阅者不应触发");
    }

    #[tokio::test]
    async fn would_dispatch_short_circuits_by_matcher() {
        let bus = EventDispatchBus::new();
        // 无订阅 → 假
        assert!(!bus.would_dispatch(&event()));
        // 仅匹配 llm/stream → agent/request 事件不应触发派发
        let _h = bus.subscribe(EventMatcher::kind("llm/stream"), sub(SubscriberVerdict::Continue));
        assert!(!bus.would_dispatch(&event()), "kind 不匹配不应触发");
        // 补一个匹配 agent/request 的订阅者 → 真
        let _h2 =
            bus.subscribe(EventMatcher::kind("agent/request"), sub(SubscriberVerdict::Continue));
        assert!(bus.would_dispatch(&event()));
        // llm/stream 事件仍应匹配（订阅了 llm/stream）
        let llm =
            DomainEvent::new(EventCategory::Agent, "llm/stream", serde_json::json!({}), "agent");
        assert!(bus.would_dispatch(&llm));
    }

    #[tokio::test]
    async fn parallel_invokes_all_concurrently() {
        let bus = EventDispatchBus::new();
        bus.subscribe(EventMatcher::any(), sub(SubscriberVerdict::Continue));
        bus.subscribe(EventMatcher::any(), sub(SubscriberVerdict::Continue));
        bus.subscribe(EventMatcher::any(), sub(SubscriberVerdict::Continue));
        let mut e = event();
        let outcome = bus.dispatch(&mut e, DispatchMode::Parallel).await;
        assert_eq!(outcome.invoked, 3);
    }

    #[tokio::test]
    async fn unsubscribe_removes_subscriber() {
        let bus = EventDispatchBus::new();
        let handle = bus.subscribe(EventMatcher::any(), sub(SubscriberVerdict::Continue));
        assert_eq!(bus.subscriber_count(), 1);
        handle.undo();
        assert_eq!(bus.subscriber_count(), 0);
        let mut e = event();
        let outcome = bus.dispatch(&mut e, DispatchMode::Emit).await;
        assert_eq!(outcome.invoked, 0);
    }

    /// 端到端模拟 provider_adapter 的 agent/request 瀑布拦截：
    /// 订阅者改写 ChatRequest（在最后一条消息追加合规 prompt），
    /// 反序列化回 ChatRequest 后验证改写生效。
    #[tokio::test]
    async fn agent_request_waterfall_rewrites_chat_request() {
        use crate::types::{ChatMessage, ChatRequest};

        let req = ChatRequest {
            model: "gpt-4o".into(),
            messages: vec![ChatMessage {
                role: "user".into(),
                content: ChatContent::Text("帮我写一份合同".into()),
                tool_calls: None,
                tool_call_id: None,
                thinking: None,
            }],
            stream: true,
            ..Default::default()
        };

        // 订阅者：把合规声明追加到最后一条用户消息
        let bus = EventDispatchBus::new();
        let _h = bus.subscribe(EventMatcher::kind("agent/request"), Arc::new(ComplianceSub));

        // 复刻 provider_adapter 的派发流程
        let mut event = DomainEvent::new(
            EventCategory::Agent,
            "agent/request",
            serde_json::to_value(&req).unwrap(),
            "agent",
        );
        let outcome = bus.dispatch(&mut event, DispatchMode::Waterfall).await;
        assert!(!outcome.rejected, "改写订阅者不应拒绝请求");

        let rewritten: ChatRequest = serde_json::from_value(outcome.rewritten.unwrap()).unwrap();
        let last = rewritten.messages.last().unwrap();
        match &last.content {
            ChatContent::Text(t) => {
                assert!(t.contains("【合规声明】"), "改写后应追加合规声明: {t}")
            },
            ChatContent::Multipart(_) => panic!("文本消息不应被改写成 multipart"),
        }
    }

    /// 合规订阅者：给最后一条消息追加合规声明。
    struct ComplianceSub;

    #[async_trait]
    impl EventSubscriber for ComplianceSub {
        async fn handle(&self, event: &DomainEvent) -> SubscriberVerdict {
            let mut req: crate::types::ChatRequest =
                serde_json::from_value(event.payload.clone()).unwrap();
            if let Some(last) = req.messages.last_mut()
                && let ChatContent::Text(t) = &mut last.content
            {
                t.push_str("\n【合规声明】本回复仅用于合法用途。");
            }
            SubscriberVerdict::Rewrite(serde_json::to_value(req).unwrap())
        }
    }

    // ─────────────── B3：每订阅者兜底预算与丢弃计数 ───────────────

    /// 慢订阅者：睡 `delay` 再表态 —— 用来证明兜底预算真的会到点，而不是只写在文档里。
    struct SlowSub {
        delay: Duration,
        verdict: SubscriberVerdict,
    }

    #[async_trait]
    impl EventSubscriber for SlowSub {
        async fn handle(&self, _event: &DomainEvent) -> SubscriberVerdict {
            tokio::time::sleep(self.delay).await;
            self.verdict.clone()
        }
    }

    fn slow(delay_ms: u64, verdict: SubscriberVerdict) -> Arc<dyn EventSubscriber> {
        Arc::new(SlowSub { delay: Duration::from_millis(delay_ms), verdict })
    }

    /// 兜底预算到点：派发**必须返回**（这是本机制存在的全部理由 —— 派发点是 spawn 出去的，
    /// 挂死的订阅者堆积的是任务，不是错误），且超时记在 `timed_out` 上。
    #[tokio::test]
    async fn hung_subscriber_is_abandoned_at_budget_and_counted() {
        let bus = EventDispatchBus::with_subscriber_timeout(Duration::from_millis(30));
        let _h = bus.subscribe_named(
            "demo@external",
            EventMatcher::any(),
            slow(30_000, SubscriberVerdict::Continue),
        );

        let mut e = event();
        let outcome = bus.dispatch(&mut e, DispatchMode::Emit).await;

        assert_eq!(outcome.invoked, 1, "进入过回调就计数（口径见 DispatchOutcome::invoked）");
        assert_eq!(outcome.timed_out, 1, "到点未返回必须计入超时");

        let stats = bus.subscriber_stats();
        assert_eq!(stats.len(), 1);
        assert_eq!(stats[0].label, "demo@external", "计数必须能归因到具体订阅者");
        assert_eq!(
            (stats[0].entered, stats[0].completed, stats[0].timed_out),
            (1, 0, 1),
            "entered / completed / timed_out 三个数要能对上"
        );
    }

    /// Waterfall 里一条订阅超时 ⇒ **不拒、不改写，链继续问下一条**。
    /// 这条判据是有方向的：超时的代价是「少一道审批」，不是「事件被卡住」。
    #[tokio::test]
    async fn waterfall_timeout_keeps_the_chain_running() {
        let bus = EventDispatchBus::with_subscriber_timeout(Duration::from_millis(30));
        let _slow = bus.subscribe(EventMatcher::any(), slow(30_000, SubscriberVerdict::Reject));
        let _ok = bus.subscribe(
            EventMatcher::any(),
            sub(SubscriberVerdict::Rewrite(serde_json::json!({ "q": 7 }))),
        );

        let mut e = event();
        let outcome = bus.dispatch(&mut e, DispatchMode::Waterfall).await;

        assert!(!outcome.rejected, "装死的订阅者不得替宿主做拒绝决定");
        assert_eq!(
            outcome.rewritten.as_ref(),
            Some(&serde_json::json!({ "q": 7 })),
            "超时不得中断瀑布，后面的订阅者仍要被问到"
        );
        assert_eq!((outcome.invoked, outcome.timed_out), (2, 1));
        assert_eq!(e.payload, serde_json::json!({ "q": 7 }));
    }

    /// Parallel：一条挂死不拖累另一条 —— 快的正常计数，慢的单独计超时。
    #[tokio::test]
    async fn parallel_bounds_each_subscriber_independently() {
        let bus = EventDispatchBus::with_subscriber_timeout(Duration::from_millis(30));
        let _fast =
            bus.subscribe_named("fast", EventMatcher::any(), sub(SubscriberVerdict::Continue));
        let _slow = bus.subscribe_named(
            "slow",
            EventMatcher::any(),
            slow(30_000, SubscriberVerdict::Continue),
        );

        let mut e = event();
        let outcome = bus.dispatch(&mut e, DispatchMode::Parallel).await;
        assert_eq!((outcome.invoked, outcome.timed_out), (2, 1));

        let stats = bus.subscriber_stats();
        let by_label: std::collections::HashMap<_, _> =
            stats.iter().map(|s| (s.label.as_str(), s)).collect();
        assert_eq!((by_label["fast"].completed, by_label["fast"].timed_out), (1, 0));
        assert_eq!((by_label["slow"].completed, by_label["slow"].timed_out), (0, 1));
    }

    /// 计数跨多次派发**累加**，且 `entered == completed + timed_out` 逐条对账成立。
    #[tokio::test]
    async fn counters_accumulate_and_balance_across_dispatches() {
        let bus = EventDispatchBus::with_subscriber_timeout(Duration::from_millis(30));
        let _ok = bus.subscribe(EventMatcher::any(), sub(SubscriberVerdict::Continue));
        let _slow = bus.subscribe(EventMatcher::any(), slow(30_000, SubscriberVerdict::Continue));

        for _ in 0..3 {
            let mut e = event();
            bus.dispatch(&mut e, DispatchMode::Serial).await;
        }

        let stats = bus.subscriber_stats();
        assert_eq!(stats.len(), 2);
        for s in &stats {
            assert_eq!(s.entered, s.completed + s.timed_out, "对账恒等式：{s:?}");
            assert_eq!(s.entered, 3, "未命名订阅的标签形如 sub-序号，计数按条累加");
        }
        assert_eq!(stats[0].completed, 3, "先注册的是正常订阅者");
        assert_eq!(stats[1].timed_out, 3, "后注册的是装死订阅者：三次全被兜底");
    }

    /// 撤销即从读口消失 —— 这是**已知取舍**，不是遗漏：快照按「当前订阅者」列，
    /// 卸载后的历史要留得住，得由持有方（插件管理器）在卸载前取走，见 PLAN §65。
    #[tokio::test]
    async fn stats_vanish_with_unsubscribe() {
        let bus = EventDispatchBus::new();
        let handle = bus.subscribe_named(
            "demo@external",
            EventMatcher::any(),
            sub(SubscriberVerdict::Continue),
        );
        let mut e = event();
        bus.dispatch(&mut e, DispatchMode::Emit).await;
        assert_eq!(bus.subscriber_stats().len(), 1);

        handle.undo();
        assert!(bus.subscriber_stats().is_empty(), "已撤销的订阅不再出现在快照里");
    }

    /// 兜底预算的定位锁：它必须**晚于**内层事件帧的 15 s 预算。
    ///
    /// 反过来的话（比如把它调成 5 s），插件桥那边还在等帧、总线已经判死 ⇒
    /// `timed_out` 会把「内层正常返回 Continue」与「内层其实还在跑」混成一坨，
    /// 两级计数从此无法对账。真要收紧事件预算，收紧的位置是**订阅者自己**，不是这里。
    #[test]
    fn subscriber_budget_is_a_backstop_above_the_inner_frame_budget() {
        const _: () = assert!(DEFAULT_SUBSCRIBER_TIMEOUT_MS > 15_000);
        assert_eq!(DEFAULT_SUBSCRIBER_TIMEOUT_MS, 60_000);
    }
}
