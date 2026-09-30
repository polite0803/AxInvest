// SPDX-License-Identifier: AGPL-3.0-only

use chrono::{Datelike, Local, Timelike};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

use super::executor::CronExecutor;
use super::job_store::CronJobStore;

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
            let mut last_check = chrono::Utc::now();
            // 同分钟重复触发防护：轮询周期 30s < 分钟粒度，同一任务在同一分钟内会
            // 两次命中 `should_run_now` ⇒ 凌晨链（荐股/趋势智选/池分析）会跑出双倍批次。
            let mut last_fired: HashMap<String, (i64, i64)> = HashMap::new();

            loop {
                if !*running_flag.read().await {
                    break;
                }

                let now = chrono::Utc::now();
                let jobs = store.list_active().await;

                for job in &jobs {
                    if let Some(next_run) = job.next_run_at {
                        let next = chrono::DateTime::from_timestamp_millis(next_run)
                            .unwrap_or(chrono::Utc::now());
                        if now < next {
                            continue;
                        }
                    }

                    if should_run_now(&job.schedule, &last_check, &now) {
                        let fired_at = minute_key(&now);
                        if last_fired.get(&job.id) == Some(&fired_at) {
                            continue;
                        }
                        last_fired.insert(job.id.clone(), fired_at);
                        tracing::info!("Cron: running job '{}' ({})", job.name, job.id);
                        // 异步执行，结果由 executor handler 写回 store
                        executor.execute(job.clone()).await;
                    }
                }

                last_check = now;
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
}

/// 本地时区的「年内日 + 本地分钟」键，用于同分钟去重。
///
/// ⚠ 必须细到**分钟**：只到小时会让 `*/15 * * * *` 这类分钟步进任务在整点后
/// 第二次命中被误拦（同一小时内本该跑 4 次只跑 1 次）。单测
/// `test_minute_key_stable_within_same_minute` 锁住这一点。
fn minute_key(now: &chrono::DateTime<chrono::Utc>) -> (i64, i64) {
    let local = now.with_timezone(&Local);
    (local.ordinal() as i64, local.hour() as i64 * 60 + local.minute() as i64)
}

fn should_run_now(
    schedule: &str,
    last_check: &chrono::DateTime<chrono::Utc>,
    now: &chrono::DateTime<chrono::Utc>,
) -> bool {
    let _ = last_check;
    if schedule.contains('*')
        || schedule.contains('/')
        || schedule.contains(',')
        || schedule.contains('-')
    {
        let parts: Vec<&str> = schedule.split_whitespace().collect();
        if parts.len() == 5 {
            // ⚠ 按**本地时区**匹配，不按 UTC：CronJob 的表达式（含前端所有默认值，
            // 如 `0 17 * * *`）都是按北京时间书写的，旧实现用 `now.*()`（UTC）匹配
            // ⇒ 所有定时任务实际晚 8 小时触发。
            let local = now.with_timezone(&Local);
            let minute_match = match_cron_field(parts[0], local.minute() as i64, 0, 59);
            let hour_match = match_cron_field(parts[1], local.hour() as i64, 0, 23);
            let day_match = match_cron_field(parts[2], local.day() as i64, 1, 31);
            let month_match = match_cron_field(parts[3], local.month() as i64, 1, 12);
            let weekday_match =
                match_cron_field(parts[4], local.weekday().num_days_from_sunday() as i64, 0, 6);

            return minute_match && hour_match && day_match && month_match && weekday_match;
        }
    }

    false
}

fn match_cron_field(field: &str, current: i64, _min: i64, _max: i64) -> bool {
    if field == "*" {
        return true;
    }

    if let Some(step) = field.strip_prefix("*/")
        && let Ok(interval) = step.parse::<i64>()
    {
        return current % interval == 0;
    }

    if field.contains(',') {
        return field.split(',').any(|p| match_cron_field(p, current, _min, _max));
    }

    if field.contains('-') {
        let parts: Vec<&str> = field.split('-').collect();
        if parts.len() == 2
            && let (Ok(lo), Ok(hi)) = (parts[0].parse::<i64>(), parts[1].parse::<i64>())
        {
            return current >= lo && current <= hi;
        }
    }

    if let Ok(exact) = field.parse::<i64>() {
        return current == exact;
    }

    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_match_cron_field_wildcard() {
        assert!(match_cron_field("*", 30, 0, 59));
    }

    #[test]
    fn test_match_cron_field_exact() {
        assert!(match_cron_field("30", 30, 0, 59));
        assert!(!match_cron_field("30", 31, 0, 59));
    }

    #[test]
    fn test_match_cron_field_range() {
        assert!(match_cron_field("10-20", 15, 0, 59));
        assert!(!match_cron_field("10-20", 25, 0, 59));
    }

    #[test]
    fn test_match_cron_field_list() {
        assert!(match_cron_field("0,30", 30, 0, 59));
        assert!(!match_cron_field("0,30", 15, 0, 59));
    }

    #[test]
    fn test_match_cron_field_step() {
        assert!(match_cron_field("*/15", 30, 0, 59));
        assert!(!match_cron_field("*/15", 31, 0, 59));
    }

    /// 时区纠偏锁单测：命中小时必须取 **Local**，不得取 UTC。
    ///
    /// 判据与机器时区绑定（本机东八区）：UTC 00:00 ⇒ 本地 08:00，
    /// `0 8 * * *` 应命中、`0 0 * * *` 应不命中。旧实现（UTC 匹配）恰好相反，
    /// 时区纠偏锁单测：命中判定必须取 **Local** 的分/时，不得取 UTC。
    ///
    /// 与机器时区无关：先用本地分时起一条「必命中」的表达式证明走 Local，
    /// 再仅当本地分/时与 UTC 分/时确有差异时，断言 UTC 那一条**不命中**
    /// （在东八区机器上两者恒差 8 小时 ⇒ 第二条必然执行）。
    /// 旧实现按 UTC 匹配 ⇒ 第一条在本机就会红，可自证。
    #[test]
    fn test_should_run_now_matches_local_clock_not_utc() {
        let now = chrono::Utc::now();
        let last = now - chrono::Duration::seconds(30);
        let local = now.with_timezone(&Local);
        let local_expr = format!("{} {} * * *", local.minute(), local.hour());
        assert!(should_run_now(&local_expr, &last, &now), "本地分时应命中 {local_expr}");

        if local.hour() != now.hour() || local.minute() != now.minute() {
            let utc_expr = format!("{} {} * * *", now.minute(), now.hour());
            assert!(
                !should_run_now(&utc_expr, &last, &now),
                "本地与 UTC 钟点不同（{local_expr} vs {utc_expr}），UTC 那条不得命中"
            );
        }
    }

    #[test]
    fn test_minute_key_stable_within_same_minute() {
        // 取当前分钟的起点，避免依赖真实时钟的秒数：+59s 仍在同一本地分钟，
        // +60s 必然跨到下一分钟 ⇒ 两个方向的断言都是确定的，不闪断。
        let minute_start = chrono::Utc::now()
            .with_second(0)
            .expect("0 秒恒有效")
            .with_nanosecond(0)
            .expect("0 纳秒恒有效");
        let same_minute = minute_start + chrono::Duration::seconds(59);
        let next_minute = minute_start + chrono::Duration::seconds(60);
        assert_eq!(minute_key(&minute_start), minute_key(&same_minute));
        assert_ne!(minute_key(&minute_start), minute_key(&next_minute));
    }
}
