//! 首字节（HTTP 响应头）等待预算 —— 全仓 LLM 流式调用的**唯一**等待口径。
//!
//! 背景（2026-10-27）：此前只有 OpenAI 兼容路径有一道响应头闸门，且是
//! 「10s 固定 + 按请求体积每 64KB 加 5s、封顶 60s」。三处结构性问题：
//!
//! ① **口径不一致** —— Anthropic / Gemini / OpenAI-Responses / transport 四条路径
//!    没有闸门（只有 client 级 300s 总超时），同一个上游在不同协议下等待时长完全不同。
//! ② **代理变量选错** —— 等响应头的时长主要由**上游排队 / prefill** 决定，与请求体积
//!    只是弱相关；体积大不代表慢，体积小排在繁忙上游照样等 30s。
//! ③ **把「慢」当「死」硬杀** —— 10s 一到立刻判「网关挂起」，慢节点被误杀；
//!    而「快速失败换供应商」的设计意图在现网**空转**（没有配任何回退模型），
//!    于是这 10s 纯粹是白烧：错误被归为瞬态可重试，引擎拿**同一个供应商**重试，
//!    每次再烧一遍。
//!
//! 现在改为**软 / 硬两档**：
//!
//! - **软档**（默认 45s）：判「慢」。超时 ⇒ **同供应商原地重试一次**（只重试这一次），
//!   因为慢 ≠ 死，重发一次往往就回来了。
//! - **硬档**（默认 90s）：判「挂」。整个请求（含那次原地重试）的**总**等待上限，
//!   超过即把错误交回上层，由上层切供应商。
//!
//! 硬档是总上限而非「第二次再给一份」，这样单次调用的最坏等待是 90s，
//! 稳稳落在工作流节点的 `step_timeout`（默认 300s）之内。
//!
//! 原地重试的代价：第一次请求可能已被上游接收并开始处理，上游若不感知客户端断开，
//! 这一请求仍可能计费。这是一次失败节点换回一次成功的代价，上限是「每次失败多一个请求」。

use std::time::Duration;

use reqwest::Response;

/// 软档默认秒数：超过它判「慢」，触发同供应商原地重试一次。
pub const DEFAULT_SOFT_HEADER_TIMEOUT_SECS: u64 = 45;

/// 硬档默认秒数：整个请求（含原地重试）的总等待上限，超过它判「挂」。
pub const DEFAULT_HARD_HEADER_TIMEOUT_SECS: u64 = 90;

/// 软档封顶秒数（体积加成后的上限）。
pub const MAX_SOFT_HEADER_TIMEOUT_SECS: u64 = 120;

/// 硬档封顶秒数（体积加成后的上限）。
pub const MAX_HARD_HEADER_TIMEOUT_SECS: u64 = 240;

/// 用户可配置值的下限秒数。
pub const MIN_CONFIGURED_HEADER_TIMEOUT_SECS: u64 = 10;

/// 用户可配置值的上限秒数（再大没有意义 —— client 总超时只有 300s）。
pub const MAX_CONFIGURED_HEADER_TIMEOUT_SECS: u64 = 240;

/// 请求体每到这个字节数，预算加一档。
pub const HEADER_BUDGET_BODY_STEP_BYTES: usize = 64 * 1024;

/// 每个字节档位加多少秒。
pub const HEADER_BUDGET_BODY_STEP_SECS: u64 = 5;

/// 首字节预算（软 / 硬两档）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HeaderBudget {
    /// 第一次尝试等待响应头的上限。超时判「慢」⇒ 同供应商原地重试一次。
    pub soft: Duration,
    /// 整个请求（含原地重试）的总等待上限。超时判「挂」⇒ 上层切供应商。
    pub hard: Duration,
}

impl HeaderBudget {
    /// 软档秒数（日志与错误文案用）。
    pub fn soft_secs(&self) -> u64 {
        self.soft.as_secs()
    }

    /// 硬档秒数（日志与错误文案用）。
    pub fn hard_secs(&self) -> u64 {
        self.hard.as_secs()
    }
}

/// 解析本次调用生效的首字节预算。
///
/// `configured_secs` = 用户在设置页「供应商-模型 → 首字节超时」填的值，
/// 语义是**硬档总上限**（含那次原地重试），软档取其一半。
/// `None` ⇒ 用默认的软 45s / 硬 90s。
///
/// `body_size` = 请求体字节数，只作为**上限加成**（每 64KB 加 5s）：大 prompt 的
/// 上游 prefill 确实更久，但它不是主变量 —— 排队时间由上游负载决定，与体积无关，
/// 所以加成只用来把封顶值抬高，不用来缩放主档位。
pub fn resolve_header_budget(configured_secs: Option<u64>, body_size: usize) -> HeaderBudget {
    let (soft_base, hard_base) = match configured_secs {
        Some(v) if v >= MIN_CONFIGURED_HEADER_TIMEOUT_SECS => {
            let hard = v.min(MAX_CONFIGURED_HEADER_TIMEOUT_SECS);
            ((hard / 2).max(MIN_CONFIGURED_HEADER_TIMEOUT_SECS / 2), hard)
        },
        // 填了但小于下限 ⇒ 视为未填，回落默认（不静默按用户填的值跑）。
        _ => (DEFAULT_SOFT_HEADER_TIMEOUT_SECS, DEFAULT_HARD_HEADER_TIMEOUT_SECS),
    };
    let bonus = (body_size / HEADER_BUDGET_BODY_STEP_BYTES) as u64 * HEADER_BUDGET_BODY_STEP_SECS;
    let soft = (soft_base + bonus).min(MAX_SOFT_HEADER_TIMEOUT_SECS);
    let hard = (hard_base + bonus).min(MAX_HARD_HEADER_TIMEOUT_SECS).max(soft);
    HeaderBudget { soft: Duration::from_secs(soft), hard: Duration::from_secs(hard) }
}

/// 日志上下文（provider / model / url），用于 `[SSE-DIAG]` 诊断行。
#[derive(Debug, Clone, Default)]
pub struct HeaderBudgetContext {
    pub provider_id: String,
    pub model: String,
    pub url: String,
}

/// 发送失败的两类原因。
#[derive(Debug)]
pub enum HeaderSendError {
    /// 网络层失败（连接 / TLS / 中途断开）—— 不是超时，按 reqwest 错误原样上抛。
    Request(reqwest::Error),
    /// 硬档超时：上游在总预算内没有返回响应头，判「挂起」。
    Hung { waited: Duration, budget: Duration, attempts: u32 },
}

/// 发送成功的结果。
#[derive(Debug)]
pub struct HeaderSendOutcome {
    /// 已收到响应头的响应（body 还没读）。
    pub response: Response,
    /// 实测首字节时延（从发起请求到拿到响应头）。
    pub ttfb: Duration,
    /// 实际发了几次（2 = 软档超时后原地重试过一次）。
    pub attempts: u32,
}

/// 按首字节预算发送请求：软档超时 ⇒ 同供应商原地重试一次；硬档超时 ⇒ 判「挂」。
///
/// `build` 必须能**重复调用**，每次造一个全新的请求（原地重试要重发同一个请求）。
pub async fn send_with_header_budget<F>(
    budget: HeaderBudget,
    log_ctx: &HeaderBudgetContext,
    mut build: F,
) -> Result<HeaderSendOutcome, HeaderSendError>
where
    F: FnMut() -> reqwest::RequestBuilder,
{
    let started = std::time::Instant::now();
    let first = build().send();
    match tokio::time::timeout(budget.soft, first).await {
        Ok(Ok(response)) => {
            let ttfb = started.elapsed();
            log_header_arrived(log_ctx, ttfb, 1);
            Ok(HeaderSendOutcome { response, ttfb, attempts: 1 })
        },
        Ok(Err(e)) => Err(HeaderSendError::Request(e)),
        Err(_) => {
            let waited = started.elapsed();
            let remaining = budget.hard.saturating_sub(waited);
            tracing::warn!(
                target: "axagent.providers.sse",
                provider_id = %log_ctx.provider_id,
                model = %log_ctx.model,
                url = %log_ctx.url,
                soft_secs = budget.soft_secs(),
                hard_secs = budget.hard_secs(),
                waited_ms = waited.as_millis() as u64,
                "[SSE-DIAG] 响应头软档超时（判慢）：同供应商原地重试一次"
            );
            if remaining.is_zero() {
                return Err(HeaderSendError::Hung { waited, budget: budget.hard, attempts: 1 });
            }
            let second = build().send();
            match tokio::time::timeout(remaining, second).await {
                Ok(Ok(response)) => {
                    let ttfb = started.elapsed();
                    log_header_arrived(log_ctx, ttfb, 2);
                    Ok(HeaderSendOutcome { response, ttfb, attempts: 2 })
                },
                Ok(Err(e)) => Err(HeaderSendError::Request(e)),
                Err(_) => {
                    let waited = started.elapsed();
                    tracing::warn!(
                        target: "axagent.providers.sse",
                        provider_id = %log_ctx.provider_id,
                        model = %log_ctx.model,
                        url = %log_ctx.url,
                        hard_secs = budget.hard_secs(),
                        waited_ms = waited.as_millis() as u64,
                        "[SSE-DIAG] 响应头硬档超时：上游挂起，交由上层切供应商"
                    );
                    Err(HeaderSendError::Hung { waited, budget: budget.hard, attempts: 2 })
                },
            }
        },
    }
}

impl HeaderSendError {
    /// 统一转成可上抛的 `anyhow` 错误（transport 层用的是 anyhow 错误类型）。
    pub fn into_anyhow(self, provider_label: &str) -> anyhow::Error {
        match self {
            HeaderSendError::Request(e) => anyhow::anyhow!(e),
            HeaderSendError::Hung { waited, budget, .. } => {
                anyhow::anyhow!(hung_error_message(provider_label, waited, budget))
            },
        }
    }
}

/// 响应头到达的诊断行（成功路径也记 —— 这是判断「上游慢」还是「闸门太紧」的唯一依据）。
fn log_header_arrived(log_ctx: &HeaderBudgetContext, ttfb: Duration, attempts: u32) {
    tracing::info!(
        target: "axagent.providers.sse",
        provider_id = %log_ctx.provider_id,
        model = %log_ctx.model,
        url = %log_ctx.url,
        ttfb_ms = ttfb.as_millis() as u64,
        attempts,
        "[SSE-DIAG] 响应头已到达"
    );
}

/// 硬档超时的错误文案。
///
/// ⚠ 文案里的 `response headers not received within` 是**契约**，三处消费点按它判定：
/// `rt-workflow` 的 `is_gateway_header_timeout`（据此归 LLM_CALL_FAILED 而不是
/// UNSUPPORTED_PROVIDER）、`harness::llm_executor` 的 `is_header_timeout`
/// （据此降为 WARN 而非 ERROR）。改文案必须同步这三处。
pub fn hung_error_message(provider_label: &str, waited: Duration, budget: Duration) -> String {
    format!(
        "{provider_label} API error 504: response headers not received within {}s \
         (response headers timeout; upstream gateway hung, total budget {}s). \
         The upstream gateway did not return response headers within the total budget. \
         Switching to a fallback provider is recommended.",
        waited.as_secs(),
        budget.as_secs()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_budget_is_soft_45_hard_90() {
        let b = resolve_header_budget(None, 0);
        assert_eq!(b.soft_secs(), 45);
        assert_eq!(b.hard_secs(), 90);
    }

    #[test]
    fn configured_value_is_hard_ceiling_and_soft_is_half() {
        let b = resolve_header_budget(Some(60), 0);
        assert_eq!(b.hard_secs(), 60);
        assert_eq!(b.soft_secs(), 30);
    }

    #[test]
    fn configured_value_below_floor_falls_back_to_default() {
        let b = resolve_header_budget(Some(3), 0);
        assert_eq!(b.soft_secs(), 45);
        assert_eq!(b.hard_secs(), 90);
    }

    #[test]
    fn body_size_only_raises_the_cap_never_lowers_the_base() {
        let small = resolve_header_budget(None, 1024);
        let big = resolve_header_budget(None, 512 * 1024);
        assert_eq!(small.soft_secs(), 45);
        assert!(big.soft_secs() >= small.soft_secs());
        assert!(big.hard_secs() >= big.soft_secs());
    }

    #[test]
    fn body_size_bonus_is_capped() {
        let huge = resolve_header_budget(None, 100 * 1024 * 1024);
        assert_eq!(huge.soft_secs(), MAX_SOFT_HEADER_TIMEOUT_SECS);
        assert_eq!(huge.hard_secs(), MAX_HARD_HEADER_TIMEOUT_SECS);
    }

    #[test]
    fn hard_is_never_below_soft() {
        for configured in [None, Some(10), Some(60), Some(240)] {
            for size in [0usize, 64 * 1024, 1024 * 1024] {
                let b = resolve_header_budget(configured, size);
                assert!(b.hard >= b.soft, "configured={configured:?} size={size}");
            }
        }
    }

    #[test]
    fn hung_message_keeps_the_consumer_contract_phrase() {
        let msg = hung_error_message("OpenAI", Duration::from_secs(90), Duration::from_secs(90));
        assert!(msg.contains("response headers not received within"));
        assert!(msg.contains("504"));
    }
}

/// 行为级测试：软档超时 ⇒ 同供应商原地重试一次；硬档超时 ⇒ 判「挂」。
///
/// 用真的本地 TCP 服务当上游，不 mock —— 判据是「服务端实际被连了几次」与
/// 「最终是成功还是挂起」，这两件事 mock 反而测不准（重试次数与连接次数会被绕开）。
#[cfg(test)]
mod http_behavior_tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    /// 起一个假上游：前 `hang_count` 次连接**不回响应**（模拟网关挂起），
    /// 之后的连接正常回 200。返回 (URL, 被连接次数计数器)。
    async fn spawn_hang_then_ok(hang_count: usize) -> (String, Arc<AtomicUsize>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let hits = Arc::new(AtomicUsize::new(0));
        let hits2 = hits.clone();
        tokio::spawn(async move {
            loop {
                let Ok((mut socket, _)) = listener.accept().await else {
                    break;
                };
                let n = hits2.fetch_add(1, Ordering::SeqCst);
                // 每个连接**单独一个任务**：挂起的那次要睡很久，若在 accept 循环里
                // 直接睡，第二个连接永远等不到 accept ⇒ 重试根本发不出去。
                tokio::spawn(async move {
                    let mut buf = vec![0u8; 4096];
                    let _ = socket.read(&mut buf).await;
                    if n < hang_count {
                        // 挂起：不写任何响应，连接保持打开直到客户端超时取消。
                        tokio::time::sleep(Duration::from_secs(30)).await;
                        return;
                    }
                    let _ =
                        socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok").await;
                    let _ = socket.flush().await;
                });
            }
        });
        (format!("http://{addr}"), hits)
    }

    fn budget(soft_secs: u64, hard_secs: u64) -> HeaderBudget {
        HeaderBudget { soft: Duration::from_secs(soft_secs), hard: Duration::from_secs(hard_secs) }
    }

    /// 本地测试客户端：**必须 `no_proxy()`**。
    /// `reqwest::Client::new()` 默认吃系统代理（HTTP_PROXY / Windows 系统代理），
    /// 本机有代理时 127.0.0.1 的请求会被转出去 ⇒ 假上游收不到，回来的却是代理的 502。
    fn local_client() -> reqwest::Client {
        reqwest::Client::builder().no_proxy().build().unwrap()
    }

    #[tokio::test]
    async fn soft_timeout_retries_once_in_place_then_succeeds() {
        let (url, hits) = spawn_hang_then_ok(1).await;
        let client = local_client();
        let ctx = HeaderBudgetContext {
            provider_id: "p-test".into(),
            model: "m-test".into(),
            url: url.clone(),
        };
        let outcome = send_with_header_budget(budget(1, 5), &ctx, || client.get(&url))
            .await
            .expect("软档超时后原地重试应当成功");

        assert_eq!(outcome.attempts, 2, "应当是「第一次挂起 + 重试一次」");
        assert_eq!(outcome.response.status().as_u16(), 200);
        assert_eq!(hits.load(Ordering::SeqCst), 2, "上游应被连了两次");
    }

    #[tokio::test]
    async fn hard_timeout_reports_hung_without_a_third_attempt() {
        let (url, hits) = spawn_hang_then_ok(99).await;
        let client = local_client();
        let ctx = HeaderBudgetContext {
            provider_id: "p-test".into(),
            model: "m-test".into(),
            url: url.clone(),
        };
        let err = send_with_header_budget(budget(1, 2), &ctx, || client.get(&url))
            .await
            .expect_err("上游一直挂起应当判「挂起」");

        match err {
            HeaderSendError::Hung { waited, budget, attempts } => {
                assert_eq!(attempts, 2, "软档重试过一次后不该再试第三次");
                assert!(waited >= Duration::from_secs(2), "总等待应达到硬档上限");
                assert_eq!(budget, Duration::from_secs(2));
            },
            HeaderSendError::Request(e) => panic!("应是超时而不是网络错误: {e}"),
        }
        assert_eq!(hits.load(Ordering::SeqCst), 2, "上游最多被连两次");
    }
}
