// SPDX-License-Identifier: AGPL-3.0-only

use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

use super::executor::CronExecutor;
use super::job_store::{CronJob, CronJobStore};

pub struct CronScheduler {
    store: Arc<CronJobStore>,
    executor: Arc<CronExecutor>,
    running: Arc<RwLock<bool>>,
    handle: Arc<RwLock<Option<tokio::task::JoinHandle<()>>>>,
}

impl CronScheduler {
    pub fn new(store: Arc<CronJobStore>, executor: Arc<CronExecutor>) -> Self {
        Self {
            store,
            executor,
            running: Arc::new(RwLock::new(false)),
            handle: Arc::new(RwLock::new(None)),
        }
    }

    pub async fn start(&self) {
        let mut running = self.running.write().await;
        if *running {
            return;
        }
        *running = true;
        drop(running);

        let store = self.store.clone();
        let executor = self.executor.clone();
        let running_flag = self.running.clone();

        let handle = tokio::spawn(async move {
            // 在途去重：job.id → 派发时该任务的 next_run_at。
            //
            // executor handler 是「入队即返回」（内部再 tokio::spawn，长任务可达数
            // 分钟），`record_run` 在任务真正结束时才把 next_run_at 推进到下一档期。
            // 在这之前每轮 tick 都会看到同一到期值，必须靠本表保证同一档期只派一次；
            // record_run 推进后值变化，下一档期自然可再次派发。进程重启本表清空 ⇒
            // 重启时仍在跑的任务按 at-least-once 重跑一次（可接受，业务侧行级认领幂等）。
            let mut dispatched: HashMap<String, Option<i64>> = HashMap::new();

            loop {
                if !*running_flag.read().await {
                    break;
                }

                // 兜底编排：next_run_at=None 的活跃任务（老版本遗留 / 外部直写内存）
                // 按表达式补算下一档期并持久化。正常 add / 恢复路径已编排，通常为 0。
                let materialized = store.materialize_next_runs().await;
                if materialized > 0 {
                    tracing::info!(
                        "[CronScheduler] 为 {materialized} 个未编排任务补算下次触发时刻"
                    );
                }

                // 到期即触发：不再要求「当前分钟精确匹配 cron」。
                //
                // 旧实现 AND 上 `should_run_now(schedule)` 的当前分钟匹配，且轮询周期
                // 30s、跨 tick 补偿被禁用 ⇒ 档期那一分钟机器睡眠 / 关机 / 正在重启，
                // 当天档期就永久蒸发、重启也不补（next=0 只放开时间下界，仍要等分钟
                // 匹配）。现在 next_run_at 已由 `record_run` 用同一套 cron 口径算成
                // 精确档期时刻，到期即派发；错过的档期在恢复后补发**一次**（恢复策略
                // 见 runtime-core 的 `resumed_next_run`），不逐档回溯。
                //
                // 每轮最多派发一个：重启补跑积压时按 list_due 的优先级排序逐个出队
                // （30s 一个），避免荐股 / 池分析 / 反思等重任务在同一瞬间雪崩。
                let due = store.list_due().await;
                if let Some(job) = next_dispatch(&due, &dispatched) {
                    let job = job.clone();
                    dispatched.insert(job.id.clone(), job.next_run_at);
                    tracing::info!("Cron: running job '{}' ({})", job.name, job.id);
                    // 异步执行，结果由 executor handler 的 record_run 写回 store
                    executor.execute(job).await;
                }

                tokio::time::sleep(std::time::Duration::from_secs(30)).await;
            }
        });

        let mut h = self.handle.write().await;
        *h = Some(handle);
    }

    pub async fn stop(&self) {
        let mut running = self.running.write().await;
        *running = false;
        drop(running);

        if let Some(handle) = self.handle.write().await.take() {
            handle.abort();
            let _ = handle.await;
        }
    }

    pub async fn is_running(&self) -> bool {
        *self.running.read().await
    }

    /// 手动触发：绕过 cron 时刻判定，直接走与定时触发完全相同的 executor handler
    /// （交易日闸门、各内置 task_type 分支、工作流兜底）。
    ///
    /// 供 `execute_scheduled_task` 命令复用 —— 该命令此前对无 workflow 的内置任务
    /// （vldec / pool-scan / stock-recommendation 等）只记一条「已手动触发」的假成功，
    /// 真正的 handler 链从未被调用。
    ///
    /// 注意：handler 内各分支自行 `tokio::spawn` 并 `record_run`，本方法在入队后
    /// 即返回，不等执行完成（长任务可达数分钟）。
    pub async fn trigger_now(&self, job: super::job_store::CronJob) {
        self.executor.execute(job).await;
    }
}

/// 从已排序的到期任务中选出本轮该派发的一个。
///
/// `due` 必须是 `CronJobStore::list_due` 的结果（已按优先级 → 成本 → 创建时间
/// 排序）；本函数只负责在途去重：跳过「派发时记录的 next_run_at 与当前值相同」
/// 的任务（任务尚未 `record_run` 推进），返回第一个可派发者。
fn next_dispatch<'a>(
    due: &'a [CronJob],
    dispatched: &HashMap<String, Option<i64>>,
) -> Option<&'a CronJob> {
    due.iter().find(|job| dispatched.get(&job.id) != Some(&job.next_run_at))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_job(id: &str, next_run_at: Option<i64>) -> CronJob {
        let mut job = CronJob::new(id, "0 9 * * *", "prompt", "desc");
        job.id = id.to_string();
        job.next_run_at = next_run_at;
        job
    }

    #[test]
    fn empty_due_returns_none() {
        let dispatched = HashMap::new();
        assert!(next_dispatch(&[], &dispatched).is_none());
    }

    #[test]
    fn first_seen_due_job_is_dispatched() {
        let job = make_job("j1", Some(0));
        let dispatched = HashMap::new();
        assert_eq!(next_dispatch(&[job], &dispatched).unwrap().id, "j1");
    }

    #[test]
    fn inflight_job_with_same_next_run_is_skipped() {
        // 已按 next=1000 派发，但任务还没 record_run（值未推进）⇒ 不得重复派发。
        let job = make_job("j1", Some(1000));
        let mut dispatched = HashMap::new();
        dispatched.insert("j1".to_string(), Some(1000));
        assert!(next_dispatch(&[job], &dispatched).is_none());
    }

    #[test]
    fn job_is_redispatched_after_record_run_advances_next() {
        // record_run 把 next 推进到未来后，到期再次出现时应可派发。
        let job = make_job("j1", Some(2000));
        let mut dispatched = HashMap::new();
        dispatched.insert("j1".to_string(), Some(1000));
        assert_eq!(next_dispatch(&[job], &dispatched).unwrap().id, "j1");
    }

    #[test]
    fn first_dispatchable_wins_when_multiple_due() {
        // 模拟重启补跑积压：j1 在途跳过，j2（list_due 排序后的下一个）被选中。
        let j1 = make_job("j1", Some(1000));
        let j2 = make_job("j2", Some(0));
        let mut dispatched = HashMap::new();
        dispatched.insert("j1".to_string(), Some(1000));
        let due = [j1, j2];
        let picked = next_dispatch(&due, &dispatched).unwrap();
        assert_eq!(picked.id, "j2");
    }

    #[test]
    fn none_next_run_dispatch_marker_roundtrips() {
        // None 不会出现在 list_due 结果里（list_due 只收 Some 到期）；但在途标记
        // 按 Option 比较，若出现 None 也必须与 Some 区分，不能误判为同一档期。
        let job = make_job("j1", Some(5));
        let mut dispatched = HashMap::new();
        dispatched.insert("j1".to_string(), None);
        assert!(next_dispatch(&[job], &dispatched).is_some());
    }
}
