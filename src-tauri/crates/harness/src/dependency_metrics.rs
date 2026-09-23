// SPDX-License-Identifier: AGPL-3.0-only

//! 外部依赖调用指标（LLM / 行情 vendor）—— 全局进程内注册表
//!
//! # 为什么需要它（问题形态）
//!
//! 两条最关键的外部依赖路径**只有日志、没有指标**：
//! - LLM 调用（`providers` crate 零 telemetry 命中）
//! - 行情抓取（`astock-data` 零 telemetry 命中，而它已有 `vendor_health` 降级机制，
//!   阈值 8 次 / 30s 窗口 ⇒ **该机制的触发频率无法观测**）
//!
//! 后果：回答不了「近 24h 各供应商成功率 / 降级次数」，只能翻日志逐行数。
//!
//! # 为什么放在 `harness` 而不是 `telemetry`
//!
//! `axagent-telemetry` **依赖** `axagent-harness`（见其 `Cargo.toml`）⇒ 若把注册表放
//! telemetry，harness 内的 LLM 收口点引用它会形成 **crate 环**。本模块零依赖（仅
//! `chrono` / `parking_lot` / `serde`），放在最底层的 harness 里，上下游都能直接调用。
//! 另有硬约束：`scripts/check-layer-discipline.mjs` 规则 3 要求 harness 不得出现任何
//! `axagent_*` 依赖 —— 本模块已满足（不引用任何兄弟 crate）。
//!
//! # 口径（**必须读，否则读数会被误读**）
//!
//! | 维度 | 口径 |
//! |---|---|
//! | LLM 覆盖 | `execute_llm` / `execute_llm_stream` 是**唯一收口**（全仓 30+ 调用点均经它） |
//! | LLM 缓存命中 | **不计入**（没有真实网络调用，记进去会把延迟稀释成 0） |
//! | LLM 提前丢弃 | 流被调用方 drop（取消）⇒ 记 `aborted`，**不**混入 ok / failed |
//! | vendor 覆盖 | `VendorHealthTracker::record_success` / `record_failure` 是唯一记账点（含手写重试循环） |
//! | vendor 成功率 | = `ok / (ok + failed)`，**不含** 429 与被判定为「数据为空」的失败 —— 它们本就不进健康窗口，计入会让这里的数字与降级机制自相矛盾 |
//! | vendor `Disabled` | 完全冻结，事件被忽略 ⇒ 不计入（与 `record_*` 的既有冻结语义一致） |
//! | 时间窗 | 累计值（进程内）+ **24 个懒轮转小时桶**；桶按 epoch 小时对齐，读时过滤过期桶 |
//! | 进程重启 | 归零。本注册表**不做持久化** —— 跨会话统计属另一件事（需落库） |
//!
//! # 谁在消费
//!
//! `src/commands/dependency_metrics.rs` 的 `get_dependency_metrics` 命令返回
//! [`DependencyMetricsSnapshot`]，供 UI / agent 查询。

use std::collections::HashMap;
use std::sync::OnceLock;

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};

/// 依赖类别。参与 key 前缀，避免同名 vendor 与 provider 撞键。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DependencyKind {
    /// LLM 供应商（key = `llm:<provider_id>`）
    Llm,
    /// 行情数据源（key = `vendor:<name>`）
    Vendor,
}

impl DependencyKind {
    fn prefix(self) -> &'static str {
        match self {
            DependencyKind::Llm => "llm",
            DependencyKind::Vendor => "vendor",
        }
    }
}

/// 小时桶容量 = 窗口长度（小时）。
const WINDOW_HOURS: i64 = 24;
const MS_PER_HOUR: i64 = 3_600_000;

/// 一个懒轮转小时桶：`hour` 与当前小时不符时整体重置。
///
/// `hour = 0` 是「空桶」哨兵（epoch 小时 0 = 1970-01-01T00:00Z，真实运行不可能命中）。
#[derive(Debug, Clone, Copy, Default)]
struct HourBucket {
    hour: i64,
    calls: u64,
    ok: u64,
    failed: u64,
    aborted: u64,
    degraded_events: u64,
    recovered_events: u64,
}

/// 单个依赖的累计 + 窗口计数。
#[derive(Debug, Clone)]
struct Entry {
    kind: DependencyKind,
    name: String,
    calls: u64,
    ok: u64,
    failed: u64,
    aborted: u64,
    /// vendor：触发降级（`record_failure` 返回 true）的次数
    degraded_events: u64,
    /// vendor：Degraded → Healthy 自动恢复次数
    recovered_events: u64,
    /// 仅统计**成功**调用的延迟：把失败（含快速失败、本地超时）混进来会让均值失真
    ok_latency_ms_total: u64,
    ok_latency_n: u64,
    last_ok_at_ms: Option<i64>,
    last_failure_at_ms: Option<i64>,
    buckets: [HourBucket; WINDOW_HOURS as usize],
}

impl Entry {
    fn new(kind: DependencyKind, name: &str) -> Self {
        Self {
            kind,
            name: name.to_string(),
            calls: 0,
            ok: 0,
            failed: 0,
            aborted: 0,
            degraded_events: 0,
            recovered_events: 0,
            ok_latency_ms_total: 0,
            ok_latency_n: 0,
            last_ok_at_ms: None,
            last_failure_at_ms: None,
            buckets: [HourBucket::default(); WINDOW_HOURS as usize],
        }
    }

    /// 取当前小时对应的桶（不一致则重置 —— 这就是「懒轮转」）。
    fn bucket(&mut self, now_ms: i64) -> &mut HourBucket {
        let hour = now_ms.div_euclid(MS_PER_HOUR);
        let idx = hour.rem_euclid(WINDOW_HOURS) as usize;
        let slot = &mut self.buckets[idx];
        if slot.hour != hour {
            *slot = HourBucket { hour, ..Default::default() };
        }
        slot
    }

    /// 窗口聚合：只统计 `hour` 落在 `(current - 24, current]` 的桶。
    fn window(&self, now_ms: i64) -> WindowAgg {
        let current = now_ms.div_euclid(MS_PER_HOUR);
        let mut agg = WindowAgg::default();
        for b in &self.buckets {
            if b.hour == 0 || b.hour > current || b.hour <= current - WINDOW_HOURS {
                continue;
            }
            agg.calls += b.calls;
            agg.ok += b.ok;
            agg.failed += b.failed;
            agg.aborted += b.aborted;
            agg.degraded_events += b.degraded_events;
            agg.recovered_events += b.recovered_events;
        }
        agg
    }
}

/// 窗口聚合结果（内部用）。
#[derive(Debug, Clone, Copy, Default)]
struct WindowAgg {
    calls: u64,
    ok: u64,
    failed: u64,
    aborted: u64,
    degraded_events: u64,
    recovered_events: u64,
}

static REGISTRY: OnceLock<Mutex<HashMap<String, Entry>>> = OnceLock::new();
static SINCE_MS: OnceLock<i64> = OnceLock::new();

fn registry() -> &'static Mutex<HashMap<String, Entry>> {
    REGISTRY.get_or_init(|| Mutex::new(HashMap::new()))
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

fn since_ms() -> i64 {
    *SINCE_MS.get_or_init(now_ms)
}

/// 首次**记录**时锚定起点。
///
/// ⚠ 不能在 `snapshot` 里锚 —— 生产路径是「先跑一堆调用、UI 之后才来读」，
/// 若等到首次读快照才锚，`since_ms` 会**晚于真实起点**，字段就变成了自证谎言
/// （读者会以为「计数只从刚才开始」，进而低估累计窗口、把累计值当窗口值用）。
fn anchor_since(now: i64) {
    let _ = SINCE_MS.get_or_init(|| now);
}

fn key_of(kind: DependencyKind, name: &str) -> String {
    format!("{}:{}", kind.prefix(), name)
}

/// 取/建条目并施加一次记录。`name` 为空时不记（避免出现 `llm:` 这种无意义键）。
fn with_entry<F>(kind: DependencyKind, name: &str, now: i64, f: F)
where
    F: FnOnce(&mut Entry),
{
    if name.is_empty() {
        return;
    }
    anchor_since(now);
    let mut guard = registry().lock();
    let entry = guard.entry(key_of(kind, name)).or_insert_with(|| Entry::new(kind, name));
    f(entry);
}

// ── 记录 API：LLM ──────────────────────────────────────────────────────

/// 记录一次 LLM 调用结果。`latency_ms` 为调用方实测耗时。
///
/// ⚠ 只有成功调用的延迟进入均值（见模块头「口径」表）。
pub fn record_llm_call(provider_id: &str, ok: bool, latency_ms: u64) {
    record_llm_call_at(provider_id, ok, latency_ms, now_ms());
}

/// 记录一次被**提前丢弃**的流式 LLM 调用（调用方 drop / 取消）。
///
/// 它既不是成功也不是失败 —— 混进任何一侧都会污染成功率，故单列 `aborted`。
pub fn record_llm_aborted(provider_id: &str) {
    record_llm_aborted_at(provider_id, now_ms());
}

fn record_llm_call_at(provider_id: &str, ok: bool, latency_ms: u64, now: i64) {
    with_entry(DependencyKind::Llm, provider_id, now, |entry: &mut Entry| {
        entry.calls += 1;
        entry.bucket(now).calls += 1;
        if ok {
            entry.ok += 1;
            entry.ok_latency_ms_total += latency_ms;
            entry.ok_latency_n += 1;
            entry.last_ok_at_ms = Some(now);
            entry.bucket(now).ok += 1;
        } else {
            entry.failed += 1;
            entry.last_failure_at_ms = Some(now);
            entry.bucket(now).failed += 1;
        }
    });
}

fn record_llm_aborted_at(provider_id: &str, now: i64) {
    with_entry(DependencyKind::Llm, provider_id, now, |entry: &mut Entry| {
        entry.calls += 1;
        entry.aborted += 1;
        let b = entry.bucket(now);
        b.calls += 1;
        b.aborted += 1;
    });
}

// ── 记录 API：vendor ──────────────────────────────────────────────────

/// 记录一次 vendor 健康记账**成功**（`VendorHealthTracker::record_success`）。
pub fn record_vendor_success(name: &str) {
    record_vendor_success_at(name, now_ms());
}

/// 记录一次 vendor 健康记账**失败**；`degraded = true` 表示本次失败触发了降级。
pub fn record_vendor_failure(name: &str, degraded: bool) {
    record_vendor_failure_at(name, degraded, now_ms());
}

/// 记录一次 vendor **自动恢复**（Degraded → Healthy）。
pub fn record_vendor_recovered(name: &str) {
    record_vendor_recovered_at(name, now_ms());
}

fn record_vendor_success_at(name: &str, now: i64) {
    with_entry(DependencyKind::Vendor, name, now, |entry: &mut Entry| {
        entry.calls += 1;
        entry.ok += 1;
        entry.last_ok_at_ms = Some(now);
        let b = entry.bucket(now);
        b.calls += 1;
        b.ok += 1;
    });
}

fn record_vendor_failure_at(name: &str, degraded: bool, now: i64) {
    with_entry(DependencyKind::Vendor, name, now, |entry: &mut Entry| {
        entry.calls += 1;
        entry.failed += 1;
        entry.last_failure_at_ms = Some(now);
        if degraded {
            entry.degraded_events += 1;
        }
        let b = entry.bucket(now);
        b.calls += 1;
        b.failed += 1;
        if degraded {
            b.degraded_events += 1;
        }
    });
}

fn record_vendor_recovered_at(name: &str, now: i64) {
    // 恢复事件既不是调用也不是失败 ⇒ 不进 calls（否则成功率会被抬高）。
    // 但**必须**进窗口桶：`degraded_events` 有窗口对应项，若恢复侧缺失，
    // 「近 24h 降级 5 次、恢复 0 次 ⇒ 该供应商卡死」这一判据就答不出来 ——
    // 而这正是降级指标最有诊断价值的用法。
    with_entry(DependencyKind::Vendor, name, now, |entry: &mut Entry| {
        entry.recovered_events += 1;
        entry.bucket(now).recovered_events += 1;
    });
}

// ── 快照 ──────────────────────────────────────────────────────────────

/// 单个依赖的指标行（对外 DTO）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DependencyMetric {
    /// 形如 `llm:agnes` / `vendor:eastmoney`
    pub key: String,
    pub kind: DependencyKind,
    pub name: String,
    pub calls: u64,
    pub ok: u64,
    pub failed: u64,
    /// 仅流式 LLM：调用方提前丢弃
    pub aborted: u64,
    /// vendor：触发降级的次数
    pub degraded_events: u64,
    /// vendor：自动恢复次数
    pub recovered_events: u64,
    /// 成功率；`ok = failed = 0` 时为 `None`（**不写 1.0** —— 那会让「没数据」看起来像「全成功」）
    pub success_rate: Option<f64>,
    /// 成功调用平均延迟（ms）；无成功样本时为 `None`
    pub avg_ok_latency_ms: Option<f64>,
    pub last_ok_at_ms: Option<i64>,
    pub last_failure_at_ms: Option<i64>,
    /// 窗口内（近 24h）计数
    pub window_calls: u64,
    pub window_ok: u64,
    pub window_failed: u64,
    pub window_aborted: u64,
    pub window_degraded_events: u64,
    /// 窗口内的自动恢复次数。与 `window_degraded_events` **成对读**才有诊断力：
    /// 「近 24h 降级 5 次、恢复 0 次」= 该供应商卡死，而非抖动。
    pub window_recovered_events: u64,
    pub window_success_rate: Option<f64>,
}

/// 全量快照（对外 DTO）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DependencyMetricsSnapshot {
    /// 本进程首次记录指标的时刻（epoch ms）—— 累计值的下界
    pub since_ms: i64,
    pub generated_at_ms: i64,
    /// 窗口长度（小时），恒为 24
    pub window_hours: u32,
    /// 按 (kind, name) 排序，读数稳定可比
    pub entries: Vec<DependencyMetric>,
}

fn rate(ok: u64, total: u64) -> Option<f64> {
    if total == 0 {
        None
    } else {
        Some(ok as f64 / total as f64)
    }
}

/// 取当前快照。`now` 由调用方传入（便于测试注入）。
fn snapshot_at(now: i64) -> DependencyMetricsSnapshot {
    let guard = registry().lock();
    let mut entries: Vec<DependencyMetric> = guard
        .values()
        .map(|e| {
            let w = e.window(now);
            DependencyMetric {
                key: key_of(e.kind, &e.name),
                kind: e.kind,
                name: e.name.clone(),
                calls: e.calls,
                ok: e.ok,
                failed: e.failed,
                aborted: e.aborted,
                degraded_events: e.degraded_events,
                recovered_events: e.recovered_events,
                success_rate: rate(e.ok, e.ok + e.failed),
                avg_ok_latency_ms: if e.ok_latency_n == 0 {
                    None
                } else {
                    Some(e.ok_latency_ms_total as f64 / e.ok_latency_n as f64)
                },
                last_ok_at_ms: e.last_ok_at_ms,
                last_failure_at_ms: e.last_failure_at_ms,
                window_calls: w.calls,
                window_ok: w.ok,
                window_failed: w.failed,
                window_aborted: w.aborted,
                window_degraded_events: w.degraded_events,
                window_recovered_events: w.recovered_events,
                window_success_rate: rate(w.ok, w.ok + w.failed),
            }
        })
        .collect();
    entries.sort_by(|a, b| (a.kind as u8, a.name.as_str()).cmp(&(b.kind as u8, b.name.as_str())));
    DependencyMetricsSnapshot {
        since_ms: since_ms(),
        generated_at_ms: now,
        window_hours: WINDOW_HOURS as u32,
        entries,
    }
}

/// 取当前快照（生产入口）。
pub fn snapshot() -> DependencyMetricsSnapshot {
    snapshot_at(now_ms())
}

// ── 测试 ──────────────────────────────────────────────────────────────
//
// ⚠ 本模块含全局单例 ⇒ 测试必须**只看自己造的键**（用随机名字），
// 不能断言全表长度 / 全表和 —— 否则会被同进程其它用例污染。
//   **实例（2026-09-20）**：`empty_name_is_ignored` 起初用
//   `before = entries.len()` / `after = entries.len()` 断言「未新增」，
//   单跑绿、`--lib` 全跑红（`left: 0, right: 10`）—— `cargo test` 是多线程
//   同进程，兄弟用例在两次取样之间正常加了条目。**判据：断言的对象必须
//   只由本用例写入，不能是共享总量。**
#[cfg(test)]
mod tests {
    use super::*;

    /// 造一个不会与其它用例/生产撞名的键
    fn uniq(tag: &str) -> String {
        format!("test-{tag}-{}", uuid::Uuid::new_v4())
    }

    fn find<'a>(snap: &'a DependencyMetricsSnapshot, name: &str) -> &'a DependencyMetric {
        snap.entries.iter().find(|e| e.name == name).expect("指标行应存在")
    }

    #[test]
    fn llm_success_and_failure_counted_with_latency() {
        let p = uniq("llm-basic");
        let now = now_ms();
        record_llm_call_at(&p, true, 100, now);
        record_llm_call_at(&p, true, 300, now);
        record_llm_call_at(&p, false, 9999, now);

        let snap = snapshot_at(now);
        let e = find(&snap, &p);
        assert_eq!(e.calls, 3);
        assert_eq!(e.ok, 2);
        assert_eq!(e.failed, 1);
        assert_eq!(e.aborted, 0);
        // 失败延迟 9999 **不得**稀释平均（否则失败越快均值越好看）
        assert_eq!(e.avg_ok_latency_ms, Some(200.0));
        assert_eq!(e.success_rate, Some(2.0 / 3.0));
        assert_eq!(e.kind, DependencyKind::Llm);
        assert_eq!(e.key, format!("llm:{p}"));
    }

    #[test]
    fn zero_calls_yields_none_rate_not_one() {
        // 负对照：若实现把空样本写成 1.0，「没有数据」会被读成「100% 成功」
        let p = uniq("llm-empty");
        record_llm_aborted_at(&p, now_ms());
        let snap = snapshot_at(now_ms());
        let e = find(&snap, &p);
        assert_eq!(e.calls, 1);
        assert_eq!(e.aborted, 1);
        assert_eq!(e.ok + e.failed, 0);
        assert_eq!(e.success_rate, None);
        assert_eq!(e.avg_ok_latency_ms, None);
        // 分母也不得把 aborted 算进去
        assert_eq!(e.window_success_rate, None);
    }

    #[test]
    fn vendor_degrade_and_recover_events_counted() {
        let v = uniq("vendor-degrade");
        let now = now_ms();
        record_vendor_success_at(&v, now);
        record_vendor_failure_at(&v, false, now);
        record_vendor_failure_at(&v, true, now);
        record_vendor_failure_at(&v, true, now);
        record_vendor_recovered_at(&v, now);

        let snap = snapshot_at(now);
        let e = find(&snap, &v);
        assert_eq!(e.calls, 4, "恢复事件不是调用，不该计入 calls");
        assert_eq!(e.ok, 1);
        assert_eq!(e.failed, 3);
        assert_eq!(e.degraded_events, 2);
        assert_eq!(e.recovered_events, 1);
        assert_eq!(e.kind, DependencyKind::Vendor);
        assert_eq!(e.key, format!("vendor:{v}"));
        // 窗口桶必须与累计值同步（同一小时内）—— 恢复侧此前缺窗口量，这里钉住
        assert_eq!(e.window_calls, 4, "同一小时内 ⇒ 窗口值应与累计值一致");
        assert_eq!(e.window_degraded_events, 2);
        assert_eq!(e.window_recovered_events, 1, "恢复事件必须进窗口桶（与降级对称）");
    }

    #[test]
    fn window_degrade_minus_recover_is_net_degradation() {
        // 诊断场景：窗口内降级 2 次、只恢复 1 次 ⇒ 该供应商尚未完全恢复。
        // 恢复侧若没有窗口量，这个判据就答不出来（只能看累计值，而累计值不会回落）。
        let v = uniq("vendor-stuck");
        let now = now_ms();
        record_vendor_failure_at(&v, true, now);
        record_vendor_failure_at(&v, true, now);
        record_vendor_recovered_at(&v, now);

        let snap = snapshot_at(now);
        let e = find(&snap, &v);
        assert_eq!(e.window_degraded_events, 2);
        assert_eq!(e.window_recovered_events, 1);
        assert_eq!(
            e.window_degraded_events - e.window_recovered_events,
            1,
            "净降级次数 = 窗口降级 - 窗口恢复"
        );
    }

    #[test]
    fn stale_recovery_falls_out_of_window() {
        // 负对照：25 小时前的恢复**不得**计入窗口。
        // 不钉这条，「近 24h 恢复 0 次 ⇒ 供应商卡死」就永远判不出来。
        let v = uniq("vendor-stale-recover");
        let now = now_ms();
        record_vendor_recovered_at(&v, now - 25 * MS_PER_HOUR);
        record_vendor_failure_at(&v, true, now);

        let snap = snapshot_at(now);
        let e = find(&snap, &v);
        assert_eq!(e.recovered_events, 1, "累计值不随时间缩水");
        assert_eq!(e.window_recovered_events, 0, "过期恢复必须被窗口排除");
        assert_eq!(e.window_degraded_events, 1);
    }

    #[test]
    fn window_rotates_and_excludes_stale_buckets() {
        let p = uniq("llm-window");
        let now = now_ms();
        // 25 小时前的一次成功：应落在窗口外（但仍计入累计）
        record_llm_call_at(&p, true, 10, now - 25 * MS_PER_HOUR);
        record_llm_call_at(&p, false, 0, now - 3 * MS_PER_HOUR);
        record_llm_call_at(&p, true, 10, now);

        let snap = snapshot_at(now);
        let e = find(&snap, &p);
        assert_eq!(e.calls, 3, "累计值不随时间缩水");
        assert_eq!(e.ok, 2);
        assert_eq!(e.window_calls, 2, "25 小时前那次必须被排除");
        assert_eq!(e.window_ok, 1);
        assert_eq!(e.window_failed, 1);
        assert_eq!(e.window_success_rate, Some(0.5));
    }

    #[test]
    fn same_name_across_kinds_does_not_collide() {
        // 正负对照：`llm:foo` 与 `vendor:foo` 必须是两行
        let n = uniq("same-name");
        let now = now_ms();
        record_llm_call_at(&n, true, 5, now);
        record_vendor_failure_at(&n, true, now);

        let snap = snapshot_at(now);
        let llm = snap.entries.iter().find(|e| e.key == format!("llm:{n}")).expect("llm 行");
        let vendor =
            snap.entries.iter().find(|e| e.key == format!("vendor:{n}")).expect("vendor 行");
        assert_eq!(llm.ok, 1);
        assert_eq!(llm.failed, 0);
        assert_eq!(vendor.ok, 0);
        assert_eq!(vendor.failed, 1);
        assert_eq!(vendor.degraded_events, 1);
    }

    #[test]
    fn empty_name_is_ignored() {
        // 空名字会造出 `llm:` 这种无意义键 ⇒ 直接丢弃
        //
        // ⚠ 断言载体是「**形态**」而非「全表长度」：空名若被记录，会以
        //   `name == ""`（键 `llm:` / `vendor:`）的条目出现。用长度差会被
        //   兄弟用例污染 —— 见本测试模块头部 2026-09-20 实例。
        let now = now_ms();
        record_llm_call_at("", true, 1, now);
        record_vendor_failure_at("", true, now);
        let snap = snapshot_at(now);
        assert!(!snap.entries.iter().any(|e| e.name.is_empty()), "空名字不得新增条目");
        assert!(
            !snap.entries.iter().any(|e| e.key == "llm:" || e.key == "vendor:"),
            "空名字不得造出无意义的畸形键"
        );
    }

    #[test]
    fn snapshot_is_sorted_and_self_describing() {
        let a = uniq("sort-a");
        let b = uniq("sort-b");
        let now = now_ms();
        record_vendor_success_at(&a, now);
        record_llm_call_at(&b, true, 1, now);

        let snap = snapshot_at(now);
        assert_eq!(snap.window_hours, 24);
        assert_eq!(snap.generated_at_ms, now);
        assert!(snap.since_ms <= now, "since 不得晚于生成时刻");
        let mut sorted = snap.entries.clone();
        sorted
            .sort_by(|x, y| (x.kind as u8, x.name.as_str()).cmp(&(y.kind as u8, y.name.as_str())));
        let lhs: Vec<&str> = snap.entries.iter().map(|e| e.key.as_str()).collect();
        let rhs: Vec<&str> = sorted.iter().map(|e| e.key.as_str()).collect();
        assert_eq!(lhs, rhs, "输出必须已排序");
    }
}
