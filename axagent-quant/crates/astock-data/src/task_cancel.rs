//! 批次级协作式取消 —— 运行时实现。
//!
//! 慢命令（收盘采集 / 历史回补 / 荐股回测）会被登记为后台任务，用户可从任务中心
//! 请求停止。停止不是 abort（那会把任务丢在半个事务里、且 spawn 的 catch_unwind
//! 接不到正常结果），而是**协作式**：命令层置旗标，engine 的循环在**批间/票间**
//! 检查一次，命中就带着已完成部分提前返回。
//!
//! 旗标经 `tokio::task_local!` 注入当前任务，engine 侧的检查是零参数的
//! [`is_cancelled()`] —— 这样 cron 触发与直接调用（无作用域）天然为 `false`，
//! 不需要给三个 engine 函数的签名加参数、也不用改它们的全部调用点。
//!
//! 与 `as_of` 的区别：as_of 需要跨 `tokio::spawn` 传播，因此有进程级全局回退；
//! 取消**刻意不做**全局回退 —— 全局旗标会让一个任务的取消误伤并发运行的另一个
//! 任务。三条慢命令的 engine 循环都在当前任务内顺序执行、内部不 spawn 子任务，
//! 所以 task_local 的可见范围恰好就是需要取消的范围。

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

/// 取消旗标。克隆共享同一原子状态（命令层持一份置位、engine 侧持一份读取）。
#[derive(Clone, Debug, Default)]
pub struct CancelToken {
    flag: Arc<AtomicBool>,
}

impl CancelToken {
    pub fn new() -> Self {
        Self::default()
    }

    /// 请求取消（幂等）。
    pub fn cancel(&self) {
        self.flag.store(true, Ordering::SeqCst);
    }

    pub fn is_cancelled(&self) -> bool {
        self.flag.load(Ordering::SeqCst)
    }
}

tokio::task_local! {
    static CURRENT_CANCEL: CancelToken;
}

/// 在取消作用域内运行 future（命令层 spawn 出去的执行体用本函数包裹）。
pub async fn with_cancel<F, T>(token: CancelToken, f: F) -> T
where
    F: std::future::Future<Output = T>,
{
    CURRENT_CANCEL.scope(token, f).await
}

/// 当前任务是否已被要求取消。无作用域（cron / 测试 / 直接调用）⇒ `false`。
pub fn is_cancelled() -> bool {
    CURRENT_CANCEL.try_with(|t| t.is_cancelled()).unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn outside_scope_is_never_cancelled() {
        assert!(!is_cancelled(), "无作用域时不得读到取消（cron 路径依赖这一点）");
    }

    #[tokio::test]
    async fn cancel_after_scope_start_is_visible_inside() {
        let token = CancelToken::new();
        let probe = token.clone();
        let inside = with_cancel(token.clone(), async {
            let before = is_cancelled();
            // 命令层在另一个句柄上置位 —— 执行体下次检查必须看到
            probe.cancel();
            (before, is_cancelled())
        })
        .await;
        assert_eq!(inside, (false, true), "已完成的批次不回溯，置位后必须可见");
        assert!(token.is_cancelled());
        assert!(!is_cancelled(), "作用域退出后不得残留（否则并发任务互相误伤）");
    }

    #[tokio::test]
    async fn nested_scopes_use_their_own_token() {
        let outer = CancelToken::new();
        let inner = CancelToken::new();
        let seen = with_cancel(outer.clone(), async {
            let outer_seen = is_cancelled();
            let inner_seen = with_cancel(inner.clone(), async {
                // 内层作用域只看自己的旗标
                is_cancelled()
            })
            .await;
            (outer_seen, inner_seen, is_cancelled())
        })
        .await;
        assert_eq!(seen, (false, false, false));
        inner.cancel();
        // 内层 token 置位不影响已退出作用域的外层读取
        assert!(!is_cancelled());
        assert!(!outer.is_cancelled());
    }
}
