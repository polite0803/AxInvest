// SPDX-License-Identifier: AGPL-3.0-only

//! 增量索引器：文件监听 → 单文件重索引。
//!
//! ⚠ 接线状态：生产实例化点为 **0**（全仓仅本文件定义 + 下方单测）。本模块已随
//! `FileIndex` / `AstIndex` 一起改为 async，以保证 crate 能编译；**这不构成接线**
//! —— 真接线需要用户可见入口（产品决策，见 `PLAN-weknora-borrowings.md §12.11.4`）。
//!
//! # 改造要点（2026-09-16）
//!
//! `FileIndex` / `AstIndex` 改为 SeaORM 实体 + `async fn` 后：
//! - `watch_and_index` 的阻塞接收循环 `for event in rx` 改为
//!   `tokio::sync::mpsc` + `while let Some(event) = rx.recv().await`；
//!   notify 的回调运行在它自己的线程里，`UnboundedSender::send` 是同步非阻塞方法，
//!   故回调闭包**无需**改动。
//! - `reindex_file` / `handle_event` / `initial_scan_and_watch` 相应改 async。
//!
//! ⚠ `last_event: RefCell<Instant>` 使本类型非 `Sync`；`handle_event` 里对它的
//! `borrow`/`borrow_mut` 都**不跨越** `.await`（先取值、再 await）。

#[cfg(not(target_os = "android"))]
use crate::ast_index::AstIndex;
#[cfg(not(target_os = "android"))]
use crate::file_index::{FileIndex, FileIndexConfig};
#[cfg(not(target_os = "android"))]
use notify::{Event, EventKind, RecursiveMode, Watcher};
#[cfg(not(target_os = "android"))]
use std::cell::RefCell;
#[cfg(not(target_os = "android"))]
use std::path::{Path, PathBuf};
#[cfg(not(target_os = "android"))]
use std::time::{Duration, Instant};

#[cfg(not(target_os = "android"))]
#[derive(Debug, Clone)]
pub struct WatchConfig {
    pub debounce_ms: u64,
    pub code_extensions: Vec<String>,
}

#[cfg(not(target_os = "android"))]
impl Default for WatchConfig {
    fn default() -> Self {
        Self {
            debounce_ms: 500,
            code_extensions: crate::file_index::CODE_EXTENSIONS
                .iter()
                .map(|s| s.to_string())
                .collect(),
        }
    }
}

#[cfg(not(target_os = "android"))]
pub struct IncrementalIndexer {
    watch_config: WatchConfig,
    file_index_config: FileIndexConfig,
    last_event: RefCell<Instant>,
}

#[cfg(not(target_os = "android"))]
impl IncrementalIndexer {
    pub fn new(watch_config: WatchConfig, file_index_config: FileIndexConfig) -> Self {
        Self { watch_config, file_index_config, last_event: RefCell::new(Instant::now()) }
    }

    pub async fn watch_and_index(
        &self,
        root: &Path,
        file_index: &FileIndex,
        ast_index: &AstIndex,
    ) -> Result<(), String> {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let root = root.to_path_buf();

        let mut watcher = notify::recommended_watcher(move |res: Result<Event, notify::Error>| {
            if let Ok(event) = res {
                let _ = tx.send(event);
            }
        })
        .map_err(|e| format!("Failed to create watcher: {e}"))?;

        watcher
            .watch(&root, RecursiveMode::Recursive)
            .map_err(|e| format!("Failed to watch directory {:?}: {e}", root))?;

        tracing::info!("File watcher started for {:?}", root);

        while let Some(event) = rx.recv().await {
            self.handle_event(&event, &root, file_index, ast_index).await;
        }

        Ok(())
    }

    async fn handle_event(
        &self,
        event: &Event,
        root: &Path,
        file_index: &FileIndex,
        ast_index: &AstIndex,
    ) {
        if self.should_skip(event, root) {
            return;
        }

        let now = Instant::now();
        // 先取出上次事件时间，**不**把 `RefCell` 借用带进下面的 async 调用。
        let last = *self.last_event.borrow();
        if now.duration_since(last) < Duration::from_millis(self.watch_config.debounce_ms) {
            return;
        }
        *self.last_event.borrow_mut() = now;

        let paths = &event.paths;
        match event.kind {
            EventKind::Create(_) | EventKind::Modify(_) => {
                for path in paths {
                    if let Err(e) = self.reindex_file(path, root, file_index, ast_index).await {
                        tracing::warn!("Failed to reindex {:?}: {e}", path);
                    }
                }
            },
            EventKind::Remove(_) => {
                for path in paths {
                    if let Ok(rel) = path.strip_prefix(root) {
                        let rel_str = rel.to_string_lossy().to_string();
                        let _ = ast_index.remove_file(&rel_str).await;
                        let _ = file_index.remove(&rel_str).await;
                        tracing::debug!("Removed index entry for {:?}", rel);
                    }
                }
            },
            _ => {},
        }
    }

    fn should_skip(&self, event: &Event, root: &Path) -> bool {
        for path in &event.paths {
            if let Ok(rel) = path.strip_prefix(root) {
                let rel_str = rel.to_string_lossy();
                for pattern in &self.file_index_config.exclude_patterns {
                    if rel_str.contains(pattern.as_str()) {
                        return true;
                    }
                }
                if let Some(ext) = path.extension().and_then(|e| e.to_str())
                    && self.watch_config.code_extensions.iter().any(|e| e == ext)
                {
                    return false;
                }
                if path.extension().is_none() && path.is_file() {
                    return false;
                }
            }
        }
        true
    }

    async fn reindex_file(
        &self,
        path: &PathBuf,
        root: &Path,
        file_index: &FileIndex,
        ast_index: &AstIndex,
    ) -> Result<(), String> {
        if !path.is_file() {
            return Ok(());
        }

        let metadata = std::fs::metadata(path).map_err(|e| format!("metadata: {e}"))?;
        let size = metadata.len();
        let modified = metadata
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs())
            .unwrap_or(0);

        let rel = path.strip_prefix(root).map_err(|e| format!("strip prefix: {e}"))?;
        let rel_str = rel.to_string_lossy().to_string();
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_string();

        file_index.upsert(&rel_str, &ext, size, modified).await?;

        let content = std::fs::read_to_string(path).map_err(|e| format!("read: {e}"))?;
        ast_index.index_file(&rel_str, &content).await?;

        tracing::debug!("Reindexed {:?} ({} bytes)", rel, size);
        Ok(())
    }

    pub async fn initial_scan_and_watch(
        &self,
        root: &Path,
        file_index: &FileIndex,
        ast_index: &AstIndex,
    ) -> Result<(), String> {
        let file_count = file_index.scan_directory(root, &self.file_index_config).await?;
        tracing::info!("Initial file scan complete: {} files indexed", file_count);

        let mut ast_count = 0;
        let entries = file_index.all_entries().await?;
        for entry in &entries {
            let abs_path = root.join(&entry.path);
            if abs_path.exists()
                && abs_path.is_file()
                && let Ok(content) = std::fs::read_to_string(&abs_path)
            {
                match ast_index.index_file(&entry.path, &content).await {
                    Ok(count) => ast_count += count,
                    Err(e) => {
                        tracing::debug!("AST index skipped for {}: {e}", entry.path);
                    },
                }
            }
        }
        tracing::info!("Initial AST index complete: {} definitions", ast_count);

        self.watch_and_index(root, file_index, ast_index).await
    }
}

#[cfg(not(target_os = "android"))]
impl Default for IncrementalIndexer {
    fn default() -> Self {
        Self::new(WatchConfig::default(), FileIndexConfig::default())
    }
}

#[cfg(not(target_os = "android"))]
#[cfg(test)]
mod tests {
    use super::*;
    use sea_orm::{ConnectOptions, Database};
    use std::io::Write;

    /// ⚠ `sqlite::memory:` 必须 `max_connections(1)`：sqlx 每条池连接各持一份
    /// 独立内存库，多连接下建表与写入会落到不同库（表现为「表不存在」）。
    async fn memory_db() -> sea_orm::DatabaseConnection {
        let mut opt = ConnectOptions::new("sqlite::memory:");
        opt.max_connections(1).min_connections(1).sqlx_logging(false);
        Database::connect(opt).await.expect("测试：打开内存数据库应成功")
    }

    #[tokio::test]
    async fn test_reindex_file() {
        let dir = std::env::temp_dir().join("axagent_incremental_test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("测试：创建目录应成功");

        let test_file = dir.join("test.rs");
        let mut f = std::fs::File::create(&test_file).expect("测试应成功");
        writeln!(f, "fn hello() {{ println!(\"hi\"); }}").expect("测试应成功");

        let fi = FileIndex::new(memory_db().await).await.expect("测试：new 应成功");
        let ai = AstIndex::new(memory_db().await).await.expect("测试：new 应成功");

        let indexer = IncrementalIndexer::default();
        indexer.reindex_file(&test_file, &dir, &fi, &ai).await.expect("测试：reindex_file 应成功");

        let results =
            ai.search_functions("hello", 10).await.expect("测试：search_functions 应成功");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].name, "hello");

        // 文件索引也应写入
        assert_eq!(fi.count().await.expect("测试：count 应成功"), 1);

        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[cfg(target_os = "android")]
use std::path::Path;

#[cfg(target_os = "android")]
#[derive(Debug, Clone)]
pub struct WatchConfig {
    pub debounce_ms: u64,
    pub code_extensions: Vec<String>,
}

#[cfg(target_os = "android")]
impl Default for WatchConfig {
    fn default() -> Self {
        Self { debounce_ms: 500, code_extensions: Vec::new() }
    }
}

#[cfg(target_os = "android")]
pub struct IncrementalIndexer;

#[cfg(target_os = "android")]
impl IncrementalIndexer {
    pub fn new(
        _watch_config: WatchConfig,
        _file_index_config: crate::file_index::FileIndexConfig,
    ) -> Self {
        Self
    }

    pub fn watch_and_index(
        &self,
        _root: &Path,
        _file_index: &crate::file_index::FileIndex,
        _ast_index: &crate::ast_index::AstIndex,
    ) -> Result<(), String> {
        Err("Incremental indexing is not available on Android".to_string())
    }

    pub fn initial_scan_and_watch(
        &self,
        _root: &Path,
        _file_index: &crate::file_index::FileIndex,
        _ast_index: &crate::ast_index::AstIndex,
    ) -> Result<(), String> {
        Err("Incremental indexing is not available on Android".to_string())
    }
}

#[cfg(target_os = "android")]
impl Default for IncrementalIndexer {
    fn default() -> Self {
        Self
    }
}
