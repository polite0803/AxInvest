// SPDX-License-Identifier: AGPL-3.0-only

//! Lightweight file directory index for pre-filtering during code search.
//!
//! Stores four metadata fields per file (path, extension, size, modification time)
//! in a SQLite table, enabling sub-millisecond filtering before more expensive
//! AST or vector operations.
//!
//! # Architecture
//!
//! - Index is rebuilt or updated via `scan_directory()`
//! - Queries use `filter_by_extension()`, `filter_by_modified_since()`, `filter_by_size_range()`
//! - Results are returned as `FileEntry` structs sorted by modification time descending
//!
//! # 访问层（2026-09-16 改造：原生 SQL → SeaORM 实体）
//!
//! 原先持 `rusqlite::Connection` + 手写 DDL + 原生 SQL；现改持 `DatabaseConnection`
//! 并全部走 `axagent_entities::file_index`：
//! - 建表由实体派生（`Schema::create_table_from_entity`）；
//! - `INSERT OR REPLACE` → `OnConflict`（`path` 已是主键，语义等价）；
//! - 前缀操作（`remove_by_prefix` / 陈旧清理）**不再用 SQL 前缀比较**，见下。
//!
//! ## ⚠ 前缀比较为何改为「取回路径 + Rust `starts_with`」
//!
//! 原实现刻意避开 `LIKE 'prefix%'`（`_` 与 `%` 都是 `LIKE` 通配符，而扫描根由
//! `WorkspaceUri::cache_path` 生成、形态 `<authority>_<md5[..8]>` **必定含 `_`**
//! ⇒ `LIKE` 会误删兄弟目录），改用 `substr(path, 1, length(?1)) = ?1`。
//!
//! 改用实体后 `LIKE` 通路已被排除（`ColumnTrait::starts_with` 会生成 `LIKE`，
//! 故**禁止使用**），而 `substr` 需经 `Expr::cust_with_values` 写裸 SQL —— 那会把
//! 方言（`?` vs `$N`）重新引回来。因此改为：取回全部 `path` 列 → Rust 侧
//! `str::starts_with` 精确比较 → 按主键 `is_in` 批量删除。
//!
//! 代价：一次全列读取（O(n) 行），而非服务端删除。本表是**可重建的索引缓存**，
//! 行数上限为工作区文件数（`all_entries` 自己也按 5000 截断），故可接受；
//! 收益是**语义比 `substr` 更严格**（纯字符串前缀，无 SQL 通配符与转义问题）。
//!
//! ## 同步 → async
//!
//! 方法全部改为 `async`：SeaORM 的 API 是 async，且 `DatabaseConnection` 是
//! `Send + Sync + Clone`，从而**解除了** `rusqlite::Connection`（`Send` 但非 `Sync`）
//! 带来的「所有访问必须收进 `spawn_blocking` 且不得跨 `.await` 共享」约束
//! （原裁定见 `PLAN-weknora-borrowings.md §12.11.2`）。
//! ⚠ 目录遍历（`read_dir` / `metadata`）仍是阻塞 I/O，调用方仍需注意不要让它
//! 阻塞关键路径 —— 见 `src/indexing_triggers.rs` 的说明。

use axagent_entities::file_index;
use sea_orm::sea_query::OnConflict;
use sea_orm::{
    ColumnTrait, ConnectionTrait, DatabaseConnection, EntityTrait, PaginatorTrait, QueryFilter,
    QueryOrder, QuerySelect, Schema, Set,
};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::time::UNIX_EPOCH;

/// 批量写入的块大小（`insert_many` 单语句参数上限的经验值）。
const INSERT_CHUNK: usize = 500;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileEntry {
    pub path: String,
    pub extension: String,
    pub size_bytes: u64,
    pub modified_at: u64,
}

#[derive(Debug, Clone)]
pub struct FileIndexConfig {
    pub max_depth: usize,
    pub include_hidden: bool,
    pub exclude_patterns: Vec<String>,
}

impl Default for FileIndexConfig {
    fn default() -> Self {
        Self {
            max_depth: 32,
            include_hidden: false,
            exclude_patterns: vec![
                "target/".to_string(),
                "node_modules/".to_string(),
                ".git/".to_string(),
                "dist/".to_string(),
                "build/".to_string(),
                "__pycache__/".to_string(),
                ".venv/".to_string(),
                "vendor/".to_string(),
                ".next/".to_string(),
            ],
        }
    }
}

pub struct FileIndex {
    pub(crate) db: DatabaseConnection,
}

impl std::fmt::Debug for FileIndex {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FileIndex").finish_non_exhaustive()
    }
}

/// 实体行 → 对外 DTO（实体用 `i64`，DTO 用 `u64`）。
fn to_entry(m: file_index::Model) -> FileEntry {
    FileEntry {
        path: m.path,
        extension: m.extension,
        size_bytes: m.size_bytes.max(0) as u64,
        modified_at: m.modified_at.max(0) as u64,
    }
}

impl FileIndex {
    pub async fn new(db: DatabaseConnection) -> Result<Self, String> {
        let index = Self { db };
        index.ensure_table().await?;
        Ok(index)
    }

    /// 建表语句由实体生成（不再手写列清单 —— 手写清单与实体漂移不会报错，
    /// 只会表现为「写入成功但读出的字段是 None」）。
    async fn ensure_table(&self) -> Result<(), String> {
        let mut stmt = Schema::new(self.db.get_database_backend())
            .create_table_from_entity(file_index::Entity);
        stmt.if_not_exists();
        self.db
            .execute(&stmt)
            .await
            .map_err(|e| format!("Failed to create file_index table: {e}"))?;
        Ok(())
    }

    /// 取回全部路径（只读 `path` 一列，供 Rust 侧前缀过滤用）。
    async fn all_paths(&self) -> Result<Vec<String>, String> {
        file_index::Entity::find()
            .select_only()
            .column(file_index::Column::Path)
            .into_tuple::<String>()
            .all(&self.db)
            .await
            .map_err(|e| format!("load file_index paths: {e}"))
    }

    /// 落在 `prefix` 之下（字面前缀）的全部路径。
    async fn paths_under(&self, prefix: &str) -> Result<Vec<String>, String> {
        Ok(self.all_paths().await?.into_iter().filter(|p| p.starts_with(prefix)).collect())
    }

    /// 按主键批量删除，返回尝试删除的行数。
    async fn delete_paths(&self, paths: Vec<String>) -> Result<usize, String> {
        if paths.is_empty() {
            return Ok(0);
        }
        let n = paths.len();
        file_index::Entity::delete_many()
            .filter(file_index::Column::Path.is_in(paths))
            .exec(&self.db)
            .await
            .map_err(|e| format!("delete file_index rows: {e}"))?;
        Ok(n)
    }

    /// Scan a directory recursively, storing metadata for all matching files.
    ///
    /// ⚠ 本函数**只清理不在本 `root` 之下的旧行**（Rust 字面前缀比较，勿用
    /// `LIKE`，理由见模块文档）。它**不会**删除 `root` 内
    /// 「已从磁盘消失」的行 —— 内存库时代每次调用都是新建的，故此前无所谓；但索引
    /// **落盘后**（见 `src/indexing_triggers.rs`）会留下幽灵行，调用方必须先用
    /// [`Self::remove_by_prefix`] 对 `root` 做「切片全量替换」。
    pub async fn scan_directory(
        &self,
        root: &Path,
        config: &FileIndexConfig,
    ) -> Result<usize, String> {
        // 目录遍历（read_dir / metadata）是**阻塞 I/O** ⇒ 放回阻塞池，别占用 async
        // 运行时的工作线程。改造前整条流水线都在 `spawn_blocking` 里，此处是把它
        // 精确收窄到「只包住文件系统那段」：文件系统仍在阻塞池，数据库改走 async。
        let root_owned = root.to_path_buf();
        let cfg = config.clone();
        let entries = tokio::task::spawn_blocking(move || {
            let mut v = Vec::new();
            scan_recursive(&root_owned, &root_owned, &cfg, 0, &mut v)?;
            Ok::<_, String>(v)
        })
        .await
        .map_err(|e| format!("scan task panicked: {e}"))??;

        let count = entries.len();

        // 分块 upsert（`path` 是主键 ⇒ 真 upsert）
        for chunk in entries.chunks(INSERT_CHUNK) {
            let models: Vec<file_index::ActiveModel> = chunk
                .iter()
                .map(|e| file_index::ActiveModel {
                    path: Set(e.path.clone()),
                    extension: Set(e.extension.clone()),
                    size_bytes: Set(e.size_bytes as i64),
                    modified_at: Set(e.modified_at as i64),
                })
                .collect();
            file_index::Entity::insert_many(models)
                .on_conflict(
                    OnConflict::column(file_index::Column::Path)
                        .update_columns([
                            file_index::Column::Extension,
                            file_index::Column::SizeBytes,
                            file_index::Column::ModifiedAt,
                        ])
                        .to_owned(),
                )
                .exec(&self.db)
                .await
                .map_err(|e| format!("insert file_index: {e}"))?;
        }

        // 清掉不属于本 root 的行
        let root_prefix = root.to_string_lossy().to_string();
        let stale: Vec<String> =
            self.all_paths().await?.into_iter().filter(|p| !p.starts_with(&root_prefix)).collect();
        self.delete_paths(stale)
            .await
            .map_err(|e| format!("Failed to clean stale entries: {e}"))?;

        Ok(count)
    }

    /// Filter entries by file extension (e.g. "rs", "ts", "py").
    pub async fn filter_by_extension(&self, extensions: &[&str]) -> Result<Vec<FileEntry>, String> {
        if extensions.is_empty() {
            return self.all_entries().await;
        }
        let rows = file_index::Entity::find()
            .filter(file_index::Column::Extension.is_in(extensions.iter().copied()))
            .order_by_desc(file_index::Column::ModifiedAt)
            .all(&self.db)
            .await
            .map_err(|e| format!("query by extension: {e}"))?;
        Ok(rows.into_iter().map(to_entry).collect())
    }

    /// Filter entries modified after the given Unix timestamp.
    pub async fn filter_by_modified_since(&self, timestamp: u64) -> Result<Vec<FileEntry>, String> {
        let rows = file_index::Entity::find()
            .filter(file_index::Column::ModifiedAt.gt(timestamp as i64))
            .order_by_desc(file_index::Column::ModifiedAt)
            .all(&self.db)
            .await
            .map_err(|e| format!("query by modified_since: {e}"))?;
        Ok(rows.into_iter().map(to_entry).collect())
    }

    /// Filter entries by file size range (inclusive).
    pub async fn filter_by_size_range(
        &self,
        min_bytes: u64,
        max_bytes: u64,
    ) -> Result<Vec<FileEntry>, String> {
        let rows = file_index::Entity::find()
            .filter(file_index::Column::SizeBytes.between(min_bytes as i64, max_bytes as i64))
            .order_by_desc(file_index::Column::ModifiedAt)
            .all(&self.db)
            .await
            .map_err(|e| format!("query by size_range: {e}"))?;
        Ok(rows.into_iter().map(to_entry).collect())
    }

    /// Search by partial path match.
    ///
    /// 此处**刻意保留 `LIKE` 语义**：这是「按用户给的模式搜路径」，通配符行为
    /// 是调用方期望的一部分（与 `remove_by_prefix` 的删除语义不同，后者必须字面匹配）。
    pub async fn search_by_path(&self, pattern: &str) -> Result<Vec<FileEntry>, String> {
        let rows = file_index::Entity::find()
            .filter(file_index::Column::Path.like(format!("%{pattern}%")))
            .order_by_desc(file_index::Column::ModifiedAt)
            .limit(100)
            .all(&self.db)
            .await
            .map_err(|e| format!("query by path pattern: {e}"))?;
        Ok(rows.into_iter().map(to_entry).collect())
    }

    /// Override or add a single file entry.
    pub async fn upsert(
        &self,
        path: &str,
        extension: &str,
        size_bytes: u64,
        modified_at: u64,
    ) -> Result<(), String> {
        let am = file_index::ActiveModel {
            path: Set(path.to_string()),
            extension: Set(extension.to_string()),
            size_bytes: Set(size_bytes as i64),
            modified_at: Set(modified_at as i64),
        };
        file_index::Entity::insert(am)
            .on_conflict(
                OnConflict::column(file_index::Column::Path)
                    .update_columns([
                        file_index::Column::Extension,
                        file_index::Column::SizeBytes,
                        file_index::Column::ModifiedAt,
                    ])
                    .to_owned(),
            )
            .exec(&self.db)
            .await
            .map_err(|e| format!("upsert {path}: {e}"))?;
        Ok(())
    }

    /// Remove a single file entry.
    pub async fn remove(&self, path: &str) -> Result<(), String> {
        file_index::Entity::delete_by_id(path.to_string())
            .exec(&self.db)
            .await
            .map_err(|e| format!("remove {path}: {e}"))?;
        Ok(())
    }

    /// Remove all entries whose paths start with the given prefix.
    ///
    /// ⚠ 字面前缀（见模块文档）：**不得**改用 `ColumnTrait::starts_with`——它生成
    /// `LIKE 'prefix%'`，而 `_`/`%` 是通配符，本项目扫描根必定含 `_`。
    pub async fn remove_by_prefix(&self, prefix: &str) -> Result<usize, String> {
        let hit = self.paths_under(prefix).await?;
        self.delete_paths(hit).await.map_err(|e| format!("remove prefix {prefix}: {e}"))
    }

    /// Get the last modification timestamp in the index.
    pub async fn latest_modified(&self) -> Result<Option<u64>, String> {
        let v: Option<i64> = file_index::Entity::find()
            .select_only()
            .column(file_index::Column::ModifiedAt)
            .order_by_desc(file_index::Column::ModifiedAt)
            .limit(1)
            .into_tuple::<i64>()
            .one(&self.db)
            .await
            .map_err(|e| format!("latest_modified: {e}"))?;
        Ok(v.map(|x| x.max(0) as u64))
    }

    /// Get total file count.
    pub async fn count(&self) -> Result<usize, String> {
        let n =
            file_index::Entity::find().count(&self.db).await.map_err(|e| format!("count: {e}"))?;
        Ok(n as usize)
    }

    /// Return all entries.
    pub async fn all_entries(&self) -> Result<Vec<FileEntry>, String> {
        let rows = file_index::Entity::find()
            .order_by_desc(file_index::Column::ModifiedAt)
            .limit(5000)
            .all(&self.db)
            .await
            .map_err(|e| format!("query all entries: {e}"))?;
        Ok(rows.into_iter().map(to_entry).collect())
    }
}

/// 递归收集文件元数据（纯文件系统操作，不碰数据库）。
fn scan_recursive(
    root: &Path,
    current: &Path,
    config: &FileIndexConfig,
    depth: usize,
    out: &mut Vec<FileEntry>,
) -> Result<(), String> {
    if depth > config.max_depth {
        return Ok(());
    }

    let entries = std::fs::read_dir(current).map_err(|e| format!("read_dir {current:?}: {e}"))?;

    for entry in entries {
        let entry = entry.map_err(|e| format!("dir entry: {e}"))?;
        let path = entry.path();
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");

        if name.starts_with('.') && !config.include_hidden {
            continue;
        }

        if path.is_dir() {
            let rel = path.strip_prefix(root).unwrap_or(&path);
            let rel_str = rel.to_string_lossy();
            if config.exclude_patterns.iter().any(|p| rel_str.contains(p.as_str())) {
                continue;
            }
            scan_recursive(root, &path, config, depth + 1, out)?;
        } else if path.is_file() {
            let metadata =
                std::fs::metadata(&path).map_err(|e| format!("metadata {path:?}: {e}"))?;
            let size = metadata.len();
            let modified = metadata
                .modified()
                .ok()
                .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                .map(|d| d.as_secs())
                .unwrap_or(0);

            let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_string();

            out.push(FileEntry {
                path: path.to_string_lossy().to_string(),
                extension: ext,
                size_bytes: size,
                modified_at: modified,
            });
        }
    }
    Ok(())
}

/// Recommended source code extensions for filtering.
pub const CODE_EXTENSIONS: &[&str] = &[
    "rs",
    "ts",
    "tsx",
    "js",
    "jsx",
    "py",
    "go",
    "java",
    "c",
    "cpp",
    "h",
    "hpp",
    "swift",
    "kt",
    "scala",
    "rb",
    "php",
    "cs",
    "vue",
    "svelte",
    "sql",
    "toml",
    "yaml",
    "yml",
    "json",
    "md",
    "css",
    "html",
    "sh",
    "bash",
    "zsh",
    "proto",
    "graphql",
    "prisma",
    "tf",
    "dockerfile",
];

#[cfg(test)]
mod tests {
    use super::*;
    use sea_orm::{ConnectOptions, Database};

    /// ⚠ `sqlite::memory:` 必须 `max_connections(1)`（sqlx 每条池连接各持一份内存库）。
    async fn test_index() -> FileIndex {
        let mut opt = ConnectOptions::new("sqlite::memory:");
        opt.max_connections(1).min_connections(1).sqlx_logging(false);
        let db = Database::connect(opt).await.expect("测试：打开内存数据库应成功");
        FileIndex::new(db).await.expect("测试应成功")
    }

    #[tokio::test]
    async fn test_upsert_and_filter() {
        let idx = test_index().await;
        idx.upsert("/src/main.rs", "rs", 1024, 1000).await.expect("测试：upsert 应成功");
        idx.upsert("/src/lib.rs", "rs", 2048, 2000).await.expect("测试：upsert 应成功");
        idx.upsert("/app.ts", "ts", 512, 500).await.expect("测试：upsert 应成功");

        let rs = idx.filter_by_extension(&["rs"]).await.expect("测试：filter_by_extension 应成功");
        assert_eq!(rs.len(), 2);
        // 排序：modified_at DESC
        assert_eq!(rs[0].path, "/src/lib.rs");

        let ts = idx.filter_by_extension(&["ts"]).await.expect("测试：filter_by_extension 应成功");
        assert_eq!(ts.len(), 1);

        let since = idx
            .filter_by_modified_since(1500)
            .await
            .expect("测试：filter_by_modified_since 应成功");
        assert_eq!(since.len(), 1);
        assert_eq!(since[0].path, "/src/lib.rs");

        let sized =
            idx.filter_by_size_range(1000, 2048).await.expect("测试：filter_by_size_range 应成功");
        assert_eq!(sized.len(), 2);
    }

    #[tokio::test]
    async fn test_count_and_remove() {
        let idx = test_index().await;
        idx.upsert("/a.rs", "rs", 100, 1).await.expect("测试：upsert 应成功");
        idx.upsert("/b.rs", "rs", 200, 2).await.expect("测试：upsert 应成功");
        assert_eq!(idx.count().await.expect("测试：count 应成功"), 2);

        idx.remove("/a.rs").await.expect("测试：移除操作应成功");
        assert_eq!(idx.count().await.expect("测试：count 应成功"), 1);

        // upsert 同一路径必须覆盖而非新增
        idx.upsert("/b.rs", "rs", 999, 42).await.expect("测试：覆盖应成功");
        assert_eq!(idx.count().await.expect("测试：count 应成功"), 1);
        let all = idx.all_entries().await.expect("测试：all_entries 应成功");
        assert_eq!(all[0].size_bytes, 999);
        assert_eq!(all[0].modified_at, 42);

        idx.remove_by_prefix("/").await.expect("测试：remove_by_prefix 应成功");
        assert_eq!(idx.count().await.expect("测试：count 应成功"), 0);
    }

    /// 回归锁：前缀必须是**字面**比较。
    ///
    /// 扫描根形态 `<authority>_<md5[..8]>`（`crates/storage/src/workspace_uri.rs:95`）
    /// **必定含 `_`**。若实现改用 `LIKE`，`_` 会匹配任意字符 ⇒
    /// `/cache_a1` 会误删 `/cacheXa1`。本测试用这一对兄弟目录锁死该语义。
    #[tokio::test]
    async fn test_remove_by_prefix_is_literal_not_like() {
        let idx = test_index().await;
        idx.upsert("/cache_a1/keep-me.rs", "rs", 1, 1).await.expect("测试：upsert 应成功");
        idx.upsert("/cacheXa1/sibling.rs", "rs", 1, 1).await.expect("测试：upsert 应成功");

        let removed = idx.remove_by_prefix("/cache_a1").await.expect("测试：前缀删除应成功");
        assert_eq!(removed, 1, "只应删掉字面命中那条");

        let left = idx.all_entries().await.expect("测试：all_entries 应成功");
        assert_eq!(left.len(), 1);
        assert_eq!(left[0].path, "/cacheXa1/sibling.rs", "兄弟目录不得被误删");
    }

    #[tokio::test]
    async fn test_latest_modified_and_search_by_path() {
        let idx = test_index().await;
        assert!(idx.latest_modified().await.expect("测试：空表应成功").is_none());

        idx.upsert("/x/alpha.rs", "rs", 1, 111).await.expect("测试：upsert 应成功");
        idx.upsert("/x/beta.ts", "ts", 1, 222).await.expect("测试：upsert 应成功");

        assert_eq!(idx.latest_modified().await.expect("测试：应成功"), Some(222));

        let hits = idx.search_by_path("alpha").await.expect("测试：search_by_path 应成功");
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].path, "/x/alpha.rs");
    }

    #[tokio::test]
    async fn test_scan_directory_upserts_real_files() {
        let dir = std::env::temp_dir().join("axagent_file_index_test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("sub")).expect("测试：建目录应成功");
        std::fs::write(dir.join("a.rs"), "fn a() {}").expect("测试：写文件应成功");
        std::fs::write(dir.join("sub/b.rs"), "fn b() {}").expect("测试：写文件应成功");

        let idx = test_index().await;
        let n = idx
            .scan_directory(&dir, &FileIndexConfig::default())
            .await
            .expect("测试：scan_directory 应成功");
        assert_eq!(n, 2);

        let rs = idx.filter_by_extension(&["rs"]).await.expect("测试：filter 应成功");
        assert_eq!(rs.len(), 2);

        let _ = std::fs::remove_dir_all(&dir);
    }
}
