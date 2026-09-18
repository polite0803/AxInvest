// SPDX-License-Identifier: AGPL-3.0-only

/// Auto-trigger indexing pipeline after cloud workspace sync.
///
/// When a cloud workspace is synced, this module automatically triggers:
/// 1. FileIndex scan (file metadata)
/// 2. AST Index (code semantics)
/// 3. L2 索引快照登记（`l2_index_snapshots`，侧车库）
///
/// # 接线状态裁定（2026-09-15，PLAN-weknora-borrowings `§12.11`）
///
/// **改动前的真实行为**：两个索引都建在 `rusqlite::Connection::open_in_memory()`
/// 上，且是 `index_workspace_blocking` 的**局部变量** ⇒ 函数一返回即 drop，
/// 扫描产物**不可达**；而调用方仍打印
/// `"Post-sync indexing complete: N files, M AST nodes"` ——
/// 一条**肯定但为假**的成功信号（与「风控警示永远绿」同型）：
/// 日志说已索引，检索侧却永远查不到。
///
/// **第四轮三项裁定**：
/// - **落盘＝是**：写 `<app_data_dir>/index.db`。与 sea-orm 侧的 `axagent.db`
///   **分文件** —— 参见 [`INDEX_DB_FILENAME`] 的说明。
/// - **槽位＝本轮不加**：`RecallPipeline` / `IncrementalIndexer` /
///   `VectorSearchCache` 三者生产实例化点实测为 **0**，此时往 `AppState` 加槽位
///   等于制造一条新的悬空链（判据 #152「接线点须公共下游」）。落盘已把产物从
///   「必然丢失」变成「持久可达」，等第一个真实消费者出现时再**与它同批**加槽。
/// - **非 `Sync` 如何共享＝不跨 `.await` 共享**：`rusqlite::Connection` 是
///   `Send` 但**非 `Sync`**。全部索引访问都收在 `spawn_blocking` 闭包内，
///   闭包内没有任何 `.await` ⇒ 既不需要 `unsafe`，也不会让 future 变成非 `Send`。
///   将来接 `RecallPipeline<'a>`（它借用 `&FileIndex`/`&AstIndex`）时，槽位形态
///   定为 `Arc<Mutex<FileIndex>>`，且**只允许在 `spawn_blocking` 内锁**——
///   禁止把 `MutexGuard` 持过 `.await`。
///
/// # 访问层改造（2026-09-16）—— 上一条裁定的**技术前提已消失**
///
/// `search` crate 的 `FileIndex` / `AstIndex` 已由 `rusqlite` 迁到 SeaORM
/// （见 `crates/search/src/file_index.rs` 的「访问层」模块文档）。
/// `DatabaseConnection` 是 `Send + Sync + Clone`
/// ⇒ 「必须整体塞进 `spawn_blocking`、且不得跨 `.await`」的理由**不再成立**：
///
/// | 改动前 | 改动后 |
/// |---|---|
/// | `open_index_db` 返回 `rusqlite::Connection` | 返回 `DatabaseConnection` |
/// | 两次 `open_index_db`（FileIndex 一条、AstIndex 一条） | **一条连接池**，两侧 `clone()` 共享 |
/// | 整个 `index_workspace_blocking` 在 `spawn_blocking` 内 | `index_workspace` 为 `async fn` |
/// | 目录遍历 + 写库都在阻塞池 | **只有文件系统那段**在阻塞池（`spawn_blocking` 精确收窄） |
///
/// ⚠ **收窄而不是删除 `spawn_blocking`**：即使 DB 已 async，`read_dir` /
/// `read_to_string` 仍是**阻塞 I/O**。直接把阻塞 I/O 丢进 async 运行时会在负载高时
/// 拖住工作线程（`file_index.rs` 的 `scan_directory` 采用同一处置）。
///
/// ⚠ **两条独立连接 → 一条共享池** 顺带消掉了一处结构性隐患：改造前
/// `sync_cloud_workspace` 与 `push_cloud_workspace_changes` 并发触发时是**四条**
/// rusqlite 连接争同一文件的写锁，只能靠 `busy_timeout` 兜住；现在同一次调用内
/// 只有一条物理连接（`max_connections(1)`），写争用在调用内**结构上不存在**，
/// `busy_timeout` 仅用于兜住「两个调用并发」这一剩余情形。
///
/// # L2 快照接线（2026-09-16，用户裁定 C「给 `l2_*` 接上消费路径」）
///
/// 每次索引完成后把本次计数登记进 `l2_index_snapshots`（侧车库），并在**写入前**
/// 读回同 root 的上一轮计数 —— 两列都非空才算接线（判据 #183：只有写入端
/// = 零读取端的补写 = 噪声）。日志因此能给出「相对上次增减多少 / 这是不是首次」，
/// 那是「本次扫到 N 个文件」回答不了的问题。
use std::path::Path;

use sea_orm::{
    ConnectOptions, ConnectionTrait, Database, DatabaseConnection, DbBackend, Statement,
};
use tracing::{info, warn};

use axagent_disk_cache::IndexSnapshotMeta;
use axagent_search::ast_index::AstIndex;
use axagent_search::file_index::{CODE_EXTENSIONS, FileIndex, FileIndexConfig};

/// 落盘索引库文件名（位于 `AppState::app_data_dir` 下）。
///
/// ⚠ 刻意**不**与 `axagent.db` 合并。改造前理由是「sea-orm 与 rusqlite 是两个
/// 独立访问层，同库会争 schema」；**改造后两侧都是 sea-orm，该理由已不成立**，
/// 但「分文件」的结论**不变**，换成更硬的理由：
/// 1. 代码索引是**可重建的缓存**（删掉重扫即可恢复），而主库承载不可重建的用户数据。
///    混库会让「清缓存」这种操作带上删主库数据的风险。
/// 2. 主库有 schema 自愈（`dao::migrations::schema_diff::heal_all`），会给**主库里
///    存在的表**补列；索引表进主库就会进入该自愈的作用域，而它们的真相源是实体
///    派生 DDL（`Schema::create_table_from_entity`）—— 两套建表主体会漂移。
/// 3. `file_index` / `ast_*` 的行数与主库同量级但生命周期完全不同（每次 sync 全量
///    替换一个 root 切片），混库会让主库的 `VACUUM` / 备份体积被缓存污染。
pub const INDEX_DB_FILENAME: &str = "index.db";

/// Trigger the full post-sync indexing pipeline.
///
/// `index_dir` 为落盘位置（生产传 `AppState::app_data_dir`）。
pub async fn trigger_post_sync_indexing_for_cloud_workspace(
    index_dir: &Path,
    workspace_path: &Path,
) -> IndexingReport {
    match tokio::fs::try_exists(workspace_path).await {
        Ok(true) => {},
        Ok(false) => {
            warn!("Workspace path does not exist: {}", workspace_path.display());
            return IndexingReport {
                skipped: true,
                reason: Some("workspace not found".to_string()),
                ..Default::default()
            };
        },
        Err(e) => {
            warn!("Workspace path not accessible: {} ({e})", workspace_path.display());
            return IndexingReport {
                skipped: true,
                reason: Some(format!("workspace not accessible: {e}")),
                ..Default::default()
            };
        },
    }

    index_workspace(index_dir, workspace_path).await
}

/// 打开（或创建）落盘索引库 —— SeaORM 连接。
///
/// - `busy_timeout` 兜住 `sync_cloud_workspace` 与 `push_cloud_workspace_changes`
///   **并发触发**时两个连接互相等待写锁（`SQLITE_BUSY`）。调用内部的争用已因
///   共用单条连接池而消失（见模块文档），此处只剩跨调用这一种情形。
/// - `journal_mode=WAL` 进一步降低「同进程两条连接写同一文件」的争用。
///   WAL 在只读介质 / `:memory:` 上会失败，故**忽略其错误**（不是致命条件）。
async fn open_index_db(path: &Path) -> Result<DatabaseConnection, String> {
    // 与 `crates/dao/src/db.rs` 同款 URL 形态（`mode=rwc` = 不存在则创建）。
    // 反斜杠必须换成正斜杠，否则 Windows 路径会被 sqlx 解析成非法 URL。
    let url = format!("sqlite:{}?mode=rwc", path.to_string_lossy().replace('\\', "/"));
    let mut opt = ConnectOptions::new(&url);
    opt.max_connections(1)
        .min_connections(1)
        .acquire_timeout(std::time::Duration::from_secs(15))
        .sqlx_logging(false);

    let db = Database::connect(opt)
        .await
        .map_err(|e| format!("Failed to open index db {}: {e}", path.display()))?;

    for pragma in
        ["PRAGMA journal_mode=WAL;", "PRAGMA busy_timeout=5000;", "PRAGMA synchronous=NORMAL;"]
    {
        let _ = db.execute_raw(Statement::from_string(DbBackend::Sqlite, pragma.to_string())).await;
    }

    Ok(db)
}

/// Run the full indexing pipeline.
///
/// 索引**落盘**（`index_dir/index.db`）而非 `open_in_memory()`：落盘是产物可达的
/// 前提；由于两份索引都改为指向同一持久库，重新扫描同一个 root 前必须做
/// 「root 切片全量替换」（`remove_by_prefix`），否则磁盘上已删除/改名的文件会
/// 留下幽灵行 —— 内存库时代每次调用都是全新的，这个缺陷被掩盖了。
async fn index_workspace(index_dir: &Path, workspace_path: &Path) -> IndexingReport {
    let mut report = IndexingReport::default();

    if let Err(e) = tokio::fs::create_dir_all(index_dir).await {
        report.file_index_error =
            Some(format!("Failed to create index dir {}: {}", index_dir.display(), e));
        report.skipped = true;
        return report;
    }

    let db_path = index_dir.join(INDEX_DB_FILENAME);
    report.index_path = Some(db_path.to_string_lossy().to_string());

    let db = match open_index_db(&db_path).await {
        Ok(db) => db,
        Err(e) => {
            report.file_index_error = Some(e);
            report.skipped = true;
            return report;
        },
    };

    // FileIndex 与 AstIndex 共用**同一个连接池**（`DatabaseConnection::clone` 共享池，
    // `max_connections(1)` ⇒ 全部 clone 序列化在同一条物理连接上）。
    let file_index = match FileIndex::new(db.clone()).await {
        Ok(fi) => fi,
        Err(e) => {
            report.file_index_error = Some(format!("Failed to create FileIndex: {e}"));
            report.skipped = true;
            return report;
        },
    };

    // root 切片全量替换（持久化前提，见函数文档）
    let root_prefix = workspace_path.to_string_lossy().to_string();
    match file_index.remove_by_prefix(&root_prefix).await {
        Ok(n) => report.stale_rows_removed += n,
        Err(e) => warn!("FileIndex stale-row cleanup failed: {}", e),
    }

    // Step 1: FileIndex scan
    let config = FileIndexConfig::default();
    match file_index.scan_directory(workspace_path, &config).await {
        Ok(count) => {
            report.files_indexed = count;
            info!("FileIndex scan completed: {} files indexed", count);
        },
        Err(e) => {
            warn!("FileIndex scan failed: {}", e);
            report.file_index_error = Some(e);
        },
    }

    // Step 2: AST Index（同一连接池的第二份句柄；两步写是串行的，故不会互锁）
    let ast_index = match AstIndex::new(db.clone()).await {
        Ok(ai) => ai,
        Err(e) => {
            warn!("Failed to create AstIndex: {}", e);
            report.ast_skipped = true;
            return report;
        },
    };

    match ast_index.remove_by_prefix(&root_prefix).await {
        Ok(n) => report.stale_rows_removed += n,
        Err(e) => warn!("AstIndex stale-row cleanup failed: {}", e),
    }

    let ast_report = index_code_files_with_ast(&ast_index, workspace_path).await;
    report.ast_nodes_indexed = ast_report.ast_nodes_indexed;
    if let Some(e) = ast_report.ast_index_error {
        report.ast_index_error = Some(e);
    }

    // Step 3: L2 索引快照登记（侧车库；未注册时静默跳过）
    record_l2_snapshot(&mut report, &root_prefix).await;

    report
}

/// 把本次索引计数登记进 `l2_index_snapshots`，并取出**上一轮**计数。
///
/// 落库的是**本次运行的计数**（`files_indexed` / `ast_nodes_indexed`），而非「全库
/// 行数」：本表以 `snapshot_id` 为键、一个工作区一行，语义是「该 root 最近一次索引
/// 的结果」，与运行计数同量纲。用 `file_index.count()` 反而不对 —— 它统计**全库**，
/// 多工作区时会把别人的行算进来，变成「同一事实的两个载体」。
///
/// `snapshot_id` 取 root 路径的稳定哈希 ⇒ 重跑是 **upsert** 而非累积
/// （`record_snapshot` 用 `OnConflict::column(SnapshotId)` 真 upsert，有回归锁
/// `test_snapshot_upsert_overwrites`）。
///
/// ⚠ **失败不写入 `snapshot_id`**：调用方据此判断「快照登记是否成功」，
/// 若登记失败却仍回报一个 id，就又造出一条「肯定但为假」的信号（§12.11.4）。
async fn record_l2_snapshot(report: &mut IndexingReport, workspace_root: &str) {
    let Some(l2) = axagent_disk_cache::l2() else {
        // 未注册 ⇒ 静默跳过。`init_l2` 在启动时调用（`src/init/state.rs`，照
        // `axagent_tools::audit::init_audit_db` 先例）；单测 / 未初始化场景
        // 不因此报错，行为与「无 L2」等价。
        return;
    };

    let snapshot_id = axagent_disk_cache::DiskCache::query_hash(workspace_root);

    // 先读上一轮 —— 这是本表在**生产路径上的读取端**（判据 #183）。
    let previous = match l2.get_snapshot(&snapshot_id).await {
        Ok(v) => v,
        Err(e) => {
            warn!("L2 上一轮快照读取失败（不影响本次索引）: {e}");
            None
        },
    };

    let path = report.index_path.clone().unwrap_or_default();
    match l2
        .record_snapshot(&snapshot_id, report.files_indexed, report.ast_nodes_indexed, &path)
        .await
    {
        Ok(()) => {
            report.snapshot_id = Some(snapshot_id);
            report.previous_snapshot = previous;
        },
        Err(e) => warn!("L2 索引快照登记失败（索引本身已落盘，产物不受影响）: {e}"),
    }
}

/// Index all code files：文件系统扫描走阻塞池，写库走 async。
async fn index_code_files_with_ast(ast_index: &AstIndex, workspace_path: &Path) -> IndexingReport {
    let mut report = IndexingReport::default();

    // 只有「遍历目录 + 读文件内容」是阻塞 I/O，精确收窄到这一段。
    let ws = workspace_path.to_path_buf();
    let code_files = match tokio::task::spawn_blocking(move || scan_code_files_blocking(&ws)).await
    {
        Ok(Ok(files)) => files,
        Ok(Err(e)) => {
            report.ast_index_error = Some(e);
            return report;
        },
        Err(e) => {
            report.ast_index_error = Some(format!("scan task panicked: {e}"));
            return report;
        },
    };

    info!("Found {} code files to index", code_files.len());

    let mut total_nodes = 0;
    for (file_path, content) in code_files {
        match ast_index.index_file(&file_path, &content).await {
            Ok(nodes) => {
                total_nodes += nodes;
            },
            Err(e) => {
                warn!("Failed to index {}: {}", file_path, e);
            },
        }
    }

    report.ast_nodes_indexed = total_nodes;
    report
}

/// Scan for code files and read their content.
///
/// ⚠ **必须在阻塞池内调用**（`spawn_blocking`）：函数体全是阻塞文件系统 I/O。
fn scan_code_files_blocking(dir: &Path) -> Result<Vec<(String, String)>, String> {
    let mut files = Vec::new();
    let extensions: Vec<String> = CODE_EXTENSIONS.iter().map(|e| e.to_string()).collect();
    scan_code_files_recursive(dir, &extensions, 0, &mut files)?;
    Ok(files)
}

fn scan_code_files_recursive(
    dir: &Path,
    extensions: &[String],
    depth: usize,
    files: &mut Vec<(String, String)>,
) -> Result<(), String> {
    if depth > 32 {
        return Ok(());
    }

    let entries =
        std::fs::read_dir(dir).map_err(|e| format!("read_dir {}: {}", dir.display(), e))?;

    for entry in entries {
        let entry = entry.map_err(|e| format!("dir entry: {}", e))?;
        let path = entry.path();
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");

        if name.starts_with('.') {
            continue;
        }

        let skip_dirs = [
            "target",
            "node_modules",
            ".git",
            "dist",
            "build",
            "__pycache__",
            ".venv",
            "vendor",
            ".next",
        ];
        if path.is_dir() && skip_dirs.contains(&name) {
            continue;
        }

        if path.is_dir() {
            scan_code_files_recursive(&path, extensions, depth + 1, files)?;
        } else if path.is_file() {
            let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_string();

            if extensions.contains(&ext) {
                if let Ok(content) = std::fs::read_to_string(&path) {
                    let path_str = path.to_string_lossy().to_string();
                    files.push((path_str, content));
                }
            }
        }
    }

    Ok(())
}

/// Report of indexing operations triggered after sync.
#[derive(Debug, Default, serde::Serialize)]
pub struct IndexingReport {
    pub files_indexed: usize,
    pub ast_nodes_indexed: usize,
    pub file_index_error: Option<String>,
    pub ast_index_error: Option<String>,
    pub ast_skipped: bool,
    pub skipped: bool,
    pub reason: Option<String>,
    /// 落盘索引库路径（`None` = 未走到落盘那一步，例如工作区不存在）。
    ///
    /// 存在的意义是让调用方的日志能**如实**指出产物落在哪儿；此前日志只报
    /// 「N files / M nodes」这种成功语气，无法区分「产物可达」与「产物已丢」。
    pub index_path: Option<String>,
    /// 本次为 `root` 切片做「全量替换」时清掉的旧行数（FileIndex + AstIndex 合计）。
    pub stale_rows_removed: usize,
    /// L2 快照登记成功时的 `snapshot_id`（**失败则为 `None`**）。
    ///
    /// 调用方据此区分「计数已登记进侧车库」与「只是本次内存里的数」；
    /// 登记失败时不回报 id，避免又一条「肯定但为假」的信号。
    pub snapshot_id: Option<String>,
    /// 同 root 的**上一轮**索引计数（`None` = 首次索引，或因读取失败而未知）。
    ///
    /// 它让日志能给出真实增减 —— 「本次扫到 N 个文件」回答不了「相对上次是涨是跌」，
    /// 而后者才是判断索引是否被意外清空（root 前缀变化 / 同步目录切换）的唯一线索。
    pub previous_snapshot: Option<IndexSnapshotMeta>,
}
