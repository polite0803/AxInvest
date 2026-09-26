//! 磁盘 L2 缓存(spec §3.2 缺陷 D 修复)
//!
//! 解决:replay 模式跨日/跨切 tab 时,每次都重新调 vendor。
//! 设计:JSON 文件落盘 + 内存 HashMap + LRU 淘汰,无新依赖(避免 SeaORM 编译开销)。
//!
//! 关键点:
//! - 启动时 `DiskCache::load_or_default` 一次性加载到内存
//! - `set` 写内存 + 标记 dirty(后台任务每 30s flush 一次)
//! - 容量满按 last_access LRU 淘汰最旧 10%
//! - TTL 保留(replay 模式由 cache_set cap 到 1h,这里再检查一次)
//!
//! 路径: `~/.axagent/astock_l2_cache.json` (与 L1 内存缓存同生命周期)

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

const DEFAULT_CAPACITY: usize = 10_000;
const EVICT_RATIO: f64 = 0.1;
const FLUSH_DIRTY_THRESHOLD: usize = 32;

/// 字节预算（两个 DiskCache 实例共用同一默认值）。
///
/// **为什么条数上限不够**：`capacity` 只数条目，而 astock 的缓存值体量差三个数量级——
/// 一条指数行情约 400 B，一条日线约 **70 KB**（`fetch_limit = max(limit, 500)`，
/// 单根 K 线 JSON 实测 ~144 B，且缓存存的是全量、读时才切最后 limit 根）。
/// 10_000 条的上限因此等于允许「几百 MB 一个 JSON 文件」，
/// 而它每次 flush 都是**全量重写**（临时文件 + rename）。
/// ⇒ 以字节为准再设一道硬预算，超了按 `last_access` LRU 淘汰。
const DEFAULT_MAX_BYTES: i64 = 64 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CacheEntry {
    value: String,
    /// unix seconds 过期时间戳
    expires_at: i64,
    /// unix seconds 最近一次访问(get/set)
    last_access: i64,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
struct DiskSnapshot {
    entries: HashMap<String, CacheEntry>,
}

// H1.3 说明:DiskCache 用 parking_lot::Mutex 而非 tokio::sync::Mutex 的理由
//
// 设计要点(参照 as_of.rs C1.5 的处理方式):
// 1. 所有锁都不持锁跨 await —— `get/set/flush_to_disk/clear/len` 中 lock()
//    仅在同步操作内持有,锁内不调任何 await,不会破坏 tokio 调度器,
//    也不会出现"MutexGuard 跨越 await"的未定义行为。
// 2. `flush_to_disk` 在锁内仅 clone entries,释放锁后再做磁盘 IO,锁外做 IO。
// 3. parking_lot::Mutex 不会中毒(poison),无需额外处理。
// 4. spawn_flush_loop 是后台异步任务,调用 should_flush/flush_to_disk 时
//    都走同步 lock,不会与未来可能新增的 await 调用产生冲突。
pub struct DiskCache {
    path: PathBuf,
    inner: Arc<Mutex<HashMap<String, CacheEntry>>>,
    capacity: usize,
    /// 字节预算，见 `DEFAULT_MAX_BYTES` 的注释
    max_bytes: i64,
    dirty_count: AtomicUsize,
    last_flush_unix: AtomicI64,
}

impl DiskCache {
    /// 加载磁盘缓存到内存;若文件不存在,初始化空缓存。
    pub fn load_or_default(path: PathBuf) -> Arc<Self> {
        Self::load_with_budget(path, DEFAULT_CAPACITY, DEFAULT_MAX_BYTES)
    }

    /// 按指定条数与字节预算加载（测试用小额度验证淘汰；生产走 `load_or_default`）
    pub fn load_with_budget(path: PathBuf, capacity: usize, max_bytes: i64) -> Arc<Self> {
        let inner = match std::fs::read_to_string(&path) {
            Ok(json) => match serde_json::from_str::<DiskSnapshot>(&json) {
                Ok(snap) => {
                    tracing::info!(
                        "[l2] loaded {} entries from {}",
                        snap.entries.len(),
                        path.display()
                    );
                    snap.entries
                },
                Err(e) => {
                    tracing::warn!(
                        "[l2] corrupt cache file {}: {}, starting empty",
                        path.display(),
                        e
                    );
                    HashMap::new()
                },
            },
            Err(_) => {
                tracing::info!("[l2] no existing cache at {}, starting fresh", path.display());
                HashMap::new()
            },
        };
        let cache = Self {
            path,
            inner: Arc::new(Mutex::new(inner)),
            capacity,
            max_bytes,
            dirty_count: AtomicUsize::new(0),
            last_flush_unix: AtomicI64::new(0),
        };
        // 旧文件可能是在「只有条数上限」的年代写下的（预算机制上线前已攒到几百 MB）
        // ⇒ 加载后立刻按预算裁一次并标脏，让本次 flush 就把文件缩回去。
        {
            let mut g = cache.inner.lock();
            if Self::trim_to_budget(&mut g, cache.max_bytes) > 0 {
                cache.dirty_count.fetch_add(1, Ordering::Relaxed);
            }
        }
        Arc::new(cache)
    }

    /// 单条目占用的字节数（key + value；元数据两三个 i64 忽略不计）
    fn entry_bytes(key: &str, entry: &CacheEntry) -> i64 {
        (key.len() + entry.value.len()) as i64
    }

    /// 按 `last_access` 从最旧开始淘汰，直到总量不超过 `max_bytes`。返回淘汰条数。
    ///
    /// 调用方持锁 —— 与 `set` 的条数淘汰共用同一把锁，不额外开临界区。
    fn trim_to_budget(entries: &mut HashMap<String, CacheEntry>, max_bytes: i64) -> usize {
        let mut total: i64 = entries.iter().map(|(k, e)| Self::entry_bytes(k, e)).sum();
        if total <= max_bytes {
            return 0;
        }
        let mut by_age: Vec<(String, i64)> =
            entries.iter().map(|(k, e)| (k.clone(), e.last_access)).collect();
        // 同 last_access 时按 key 定序，保证淘汰顺序可复现（测试依赖）
        by_age.sort();
        let mut evicted = 0usize;
        for (k, _) in by_age {
            if total <= max_bytes {
                break;
            }
            if let Some(e) = entries.remove(&k) {
                total -= Self::entry_bytes(&k, &e);
                evicted += 1;
            }
        }
        tracing::info!("[l2] 超字节预算 {max_bytes}，按 LRU 淘汰 {evicted} 条（余 {total} 字节）");
        evicted
    }

    fn now_unix() -> i64 {
        SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or_else(
            |e| {
                // 修复 M-RES-5: 系统时间早于 UNIX_EPOCH（嵌入式/虚拟机时钟漂移）
                // 时 unwrap_or(0) 静默返回 0，导致缓存条目立即过期。
                // 添加 warn 日志便于发现时钟异常。
                tracing::warn!("[disk_cache] SystemTime 早于 UNIX_EPOCH（时钟倒流）: {e}");
                0
            },
        )
    }

    /// 查缓存;命中且未过期返回 Some(value),否则 None(顺便清理过期项)。
    pub fn get(&self, key: &str) -> Option<String> {
        let now = Self::now_unix();
        let mut inner = self.inner.lock();
        let entry = inner.get_mut(key)?;
        if entry.expires_at > 0 && entry.expires_at < now {
            // 过期
            inner.remove(key);
            return None;
        }
        entry.last_access = now;
        Some(entry.value.clone())
    }

    /// 写缓存。
    /// - `ttl_secs > 0`: 标准 TTL(秒)
    /// - `ttl_secs == 0`: 永不过期(`expires_at=0`)
    /// - `ttl_secs < 0`: 立即过期(测试用,`expires_at=1`)
    pub fn set(&self, key: String, value: String, ttl_secs: i64) {
        let now = Self::now_unix();
        let expires_at = if ttl_secs < 0 {
            1
        } else if ttl_secs == 0 {
            0
        } else {
            now + ttl_secs
        };
        // 单条就超预算 ⇒ 直接不存。否则会出现「每写一条都把整个缓存清空」的自杀式淘汰，
        // 缓存退化成零，而日志被刷满。
        if (key.len() + value.len()) as i64 > self.max_bytes {
            tracing::warn!(
                "[l2] 单条超字节预算（{} > {}），跳过缓存: key={key}",
                key.len() + value.len(),
                self.max_bytes
            );
            return;
        }
        let mut inner = self.inner.lock();

        // 容量满时 LRU 淘汰
        if inner.len() >= self.capacity {
            let to_evict = ((self.capacity as f64) * EVICT_RATIO).ceil() as usize;
            let mut entries: Vec<(String, i64)> =
                inner.iter().map(|(k, v)| (k.clone(), v.last_access)).collect();
            entries.sort_by_key(|(_, la)| *la);
            for (k, _) in entries.into_iter().take(to_evict) {
                inner.remove(&k);
            }
            tracing::info!(
                "[l2] capacity reached {}, evicted {} oldest entries",
                self.capacity,
                to_evict
            );
        }

        inner.insert(key, CacheEntry { value, expires_at, last_access: now });
        // 字节预算淘汰（条数上限拦不住几条 70 KB 的 K 线把文件撑爆）
        Self::trim_to_budget(&mut inner, self.max_bytes);
        self.dirty_count.fetch_add(1, Ordering::Relaxed);
    }

    /// 检查是否需要 flush(脏条目数超过阈值或距上次 flush 超过 30s)。
    /// 返回 true 时调用方应调 `flush_to_disk`。
    pub fn should_flush(&self) -> bool {
        let dirty = self.dirty_count.load(Ordering::Relaxed);
        if dirty >= FLUSH_DIRTY_THRESHOLD {
            return true;
        }
        let now = Self::now_unix();
        let last = self.last_flush_unix.load(Ordering::Relaxed);
        dirty > 0 && now - last >= 30
    }

    /// 同步 flush 到磁盘;失败仅 warn,不阻塞。
    pub fn flush_to_disk(&self) {
        let inner = self.inner.lock();
        let snap = DiskSnapshot { entries: inner.clone() };
        match serde_json::to_string(&snap) {
            Ok(json) => {
                if let Some(parent) = self.path.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                // 写临时文件再 rename,避免半截文件污染
                let tmp = self.path.with_extension("json.tmp");
                if let Err(e) = std::fs::write(&tmp, json) {
                    tracing::warn!("[l2] flush write failed: {}", e);
                    return;
                }
                if let Err(e) = std::fs::rename(&tmp, &self.path) {
                    tracing::warn!("[l2] flush rename failed: {}", e);
                    return;
                }
                self.dirty_count.store(0, Ordering::Relaxed);
                self.last_flush_unix.store(Self::now_unix(), Ordering::Relaxed);
                tracing::debug!("[l2] flushed {} entries to disk", inner.len());
            },
            Err(e) => tracing::warn!("[l2] serialize failed: {}", e),
        }
    }

    /// 缓存条目数(供测试和监控用)
    pub fn len(&self) -> usize {
        let g = self.inner.lock();
        g.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// 当前占用字节数（key + value）。字节预算是否真生效，只有这个数能证明。
    pub fn total_bytes(&self) -> i64 {
        let g = self.inner.lock();
        g.iter().map(|(k, e)| Self::entry_bytes(k, e)).sum()
    }

    /// 本实例的字节预算（见 `DEFAULT_MAX_BYTES`）
    pub fn max_bytes(&self) -> i64 {
        self.max_bytes
    }

    /// 清空所有条目(测试用)
    pub fn clear(&self) {
        let mut g = self.inner.lock();
        g.clear();
        self.dirty_count.store(0, Ordering::Relaxed);
    }
}

/// 后台 flush 任务的句柄，用于优雅关闭。
///
/// 修复 M-PERF-2: 原 `spawn_flush_loop` 返回 ()，调用方无法停止后台任务，
/// 进程退出时可能丢失未落盘的脏数据或卡在 30s tick 之间。
/// 现在返回 `FlushLoopHandle`，调用方可通过 `shutdown()` 触发停止，
/// 并通过 `join().await` 等待最后一次 flush 完成。
pub struct FlushLoopHandle {
    shutdown: Arc<AtomicBool>,
    join_handle: tokio::task::JoinHandle<()>,
}

impl FlushLoopHandle {
    /// 触发后台 flush 任务停止。
    ///
    /// 注意：此方法仅设置 flag，不会阻塞。如需等待最后一次 flush 完成，
    /// 调用 `join().await`。
    pub fn shutdown(&self) {
        self.shutdown.store(true, Ordering::Release);
    }

    /// 等待后台任务真正退出。
    pub async fn join(self) {
        self.join_handle.await.ok();
    }

    /// 触发停止并等待退出（便捷方法）。
    pub async fn shutdown_and_join(self) {
        self.shutdown();
        self.join().await;
    }
}

/// 启动后台 flush 任务:每 30s 检查一次,脏时落盘。
/// `cache` 弱引用(Arc)由调用方持有。
///
/// 返回 `FlushLoopHandle` 用于优雅关闭。
pub fn spawn_flush_loop(cache: Arc<DiskCache>) -> FlushLoopHandle {
    let shutdown = Arc::new(AtomicBool::new(false));
    let shutdown_clone = shutdown.clone();
    let join_handle = tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(30));
        // 跳过首次 immediate tick
        interval.tick().await;
        loop {
            // 修复 M-PERF-2: 检查 shutdown flag，优雅退出
            if shutdown_clone.load(Ordering::Acquire) {
                tracing::info!("[l2] flush loop 收到 shutdown 信号，执行最后一次 flush");
                if cache.should_flush() {
                    cache.flush_to_disk();
                }
                break;
            }
            interval.tick().await;
            if cache.should_flush() {
                cache.flush_to_disk();
            }
        }
    });
    FlushLoopHandle { shutdown, join_handle }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;

    fn tmp_path(name: &str) -> PathBuf {
        let mut p = env::temp_dir();
        p.push(format!("axagent_l2_test_{}_{}.json", name, std::process::id()));
        let _ = std::fs::remove_file(&p);
        p
    }

    #[test]
    fn set_get_roundtrip() {
        let path = tmp_path("roundtrip");
        let c = DiskCache::load_or_default(path.clone());
        c.set("k".into(), "v".into(), 60);
        assert_eq!(c.get("k").as_deref(), Some("v"));
        assert_eq!(c.len(), 1);
        c.flush_to_disk();
        // 重新加载,验证落盘有效
        let c2 = DiskCache::load_or_default(path);
        assert_eq!(c2.get("k").as_deref(), Some("v"));
        assert_eq!(c2.len(), 1);
    }

    #[test]
    fn get_returns_none_for_expired() {
        let path = tmp_path("expired");
        let c = DiskCache::load_or_default(path);
        c.set("k".into(), "v".into(), -1); // 已过期
                                           // 模拟"过去时间"通过改 expires_at:这里 ttl=-1 → expires_at = now-1
                                           // get 内部判 expires_at < now,直接走 remove 分支
        assert!(c.get("k").is_none());
        assert_eq!(c.len(), 0, "过期项应在 get 时被清理");
    }

    #[test]
    fn get_returns_none_for_missing() {
        let path = tmp_path("missing");
        let c = DiskCache::load_or_default(path);
        assert!(c.get("nonexistent").is_none());
    }

    #[test]
    fn lru_eviction_on_capacity_overflow() {
        let path = tmp_path("lru");
        let c = DiskCache::load_or_default(path);
        // 写入 10000 + 1 项触发 LRU 淘汰
        for i in 0..DEFAULT_CAPACITY + 1 {
            c.set(format!("k{i}"), format!("v{i}"), 60);
        }
        // 容量被 cap 在 10000
        let len = c.len();
        assert!(len <= DEFAULT_CAPACITY, "容量应不超过 {},实际 {}", DEFAULT_CAPACITY, len);
    }

    #[test]
    fn flush_idempotent_when_clean() {
        let path = tmp_path("idempotent");
        let c = DiskCache::load_or_default(path);
        c.set("k".into(), "v".into(), 60);
        c.flush_to_disk();
        let dirty_before = c.dirty_count.load(Ordering::Relaxed);
        c.flush_to_disk(); // 再次 flush,不应报错
        let dirty_after = c.dirty_count.load(Ordering::Relaxed);
        assert_eq!(dirty_before, 0, "首次 flush 后 dirty 应清零");
        assert_eq!(dirty_after, 0);
    }

    #[test]
    fn should_flush_threshold() {
        let path = tmp_path("threshold");
        let c = DiskCache::load_or_default(path);
        assert!(!c.should_flush(), "空缓存不应 flush");
        for i in 0..FLUSH_DIRTY_THRESHOLD {
            c.set(format!("k{i}"), "v".into(), 60);
        }
        assert!(c.should_flush(), "达到 dirty 阈值应 flush");
    }

    // ── 字节预算（2026-09-26 新增）─────────────────────────────
    // 只有条数上限时，几条 70 KB 的 K 线就能把文件撑到几百 MB，而 flush 是**全量重写**
    // ⇒ 上限必须按字节算，不能只按条数。

    #[test]
    fn byte_budget_caps_total_size() {
        let c = DiskCache::load_with_budget(tmp_path("budget"), 10_000, 1_000);
        for i in 0..20 {
            c.set(format!("k{i:02}"), "x".repeat(200), 300);
        }
        assert!(c.total_bytes() <= 1_000, "超预算必须裁到 1_000 以内，实际 {}", c.total_bytes());
        assert!(c.len() > 1, "预算淘汰不得退化成清空，否则缓存等于没有");
    }

    #[test]
    fn oversized_single_entry_is_skipped_not_wipe_all() {
        // 反例形态：超预算的单条也先写入再 trim ⇒ 每写一条大的就把整个缓存清空
        let c = DiskCache::load_with_budget(tmp_path("oversize"), 10_000, 500);
        c.set("small".into(), "z".repeat(100), 300);
        c.set("big".into(), "y".repeat(5_000), 300);
        assert!(c.get("big").is_none(), "单条超预算应跳过写入");
        assert!(c.get("small").is_some(), "跳过超预算条目不得伤及已有条目");
    }

    #[test]
    fn legacy_oversized_file_trimmed_on_load() {
        let path = tmp_path("legacy");
        {
            let c = DiskCache::load_with_budget(path.clone(), 10_000, 10_000);
            for i in 0..20 {
                c.set(format!("k{i:02}"), "x".repeat(200), 300);
            }
            c.flush_to_disk();
        }
        // 更小的预算重新加载 = 模拟「旧文件是只有条数上限的年代攒下的」
        let c = DiskCache::load_with_budget(path, 10_000, 1_000);
        assert!(c.total_bytes() <= 1_000, "加载即按新预算裁剪，不得等到第一次写才生效");
        assert!(c.should_flush(), "裁过必须标脏，让本次 flush 把文件缩回去");
    }

    #[test]
    fn default_budget_is_64mb() {
        let c = DiskCache::load_or_default(tmp_path("default_budget"));
        assert_eq!(c.max_bytes(), 64 * 1024 * 1024, "默认预算变了 ⇒ 说明有人改了常量");
    }
}
