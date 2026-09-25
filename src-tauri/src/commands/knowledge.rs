// SPDX-License-Identifier: AGPL-3.0-only

use crate::AppState;
use crate::commands::spawn_guard::catch_unwind_logged;
use axagent_agent_macro::agent_command;
use axagent_dao::repo::index_jobs as jobs;
use axagent_entities::{
    knowledge_bases, knowledge_documents, knowledge_entities, knowledge_relations,
};
use axagent_harness::IpcEventName;
use axagent_harness::types::*;
use axagent_search::rag::KnowledgeContainer;
use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, PaginatorTrait, QueryFilter, Set};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use tauri::{AppHandle, Emitter, Manager, State};

/// 目录导入结果（单文档批量导入的汇总）。
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportDirectoryError {
    pub path: String,
    pub error: String,
    /// 可选错误码（对应 `error_code.rs` 常量），前端可按码走 i18n 翻译；
    /// 为 `None` 时前端回退显示原始 `error` 文本。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
}

impl ImportDirectoryError {
    /// 构造带错误码的错误项（前端可按 `code` 翻译）。
    fn with_code(path: String, error: String, code: impl Into<String>) -> Self {
        Self { path, error, code: Some(code.into()) }
    }
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportDirectoryResult {
    pub base_id: String,
    pub imported_count: usize,
    pub skipped_count: usize,
    pub error_count: usize,
    pub entity_count: usize,   // 知识图谱实体导入数
    pub relation_count: usize, // 知识图谱关系导入数
    /// 实际使用的嵌入模型 provider（None 表示未配置，向量检索不可用）
    pub embedding_provider: Option<String>,
    pub imported: Vec<KnowledgeDocument>,
    pub skipped: Vec<String>,
    pub errors: Vec<ImportDirectoryError>,
}

/// 目录导入遇到「目标文档已存在」时的冲突处理策略。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ConflictPolicy {
    /// 跳过已存在的文档（默认；不触发重复写入）
    Skip,
    /// 删旧文档（含向量）+ 重新添加并索引，保证磁盘内容与 KB 一致
    Overwrite,
}

/// 目录预扫描结果中的单个可导入文件。
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DirectoryScanFile {
    /// 文件绝对路径（压缩包内部文件为解包后的临时路径，导入时按此路径读取）
    pub path: String,
    /// 相对目录根的路径（POSIX 风格 `\` → `/`），导入时作为文档标题
    pub rel_path: String,
    /// 无点小写扩展名（如 `md`、`pdf`）
    pub extension: String,
    /// 文件大小（字节）
    pub size_bytes: u64,
    /// 是否来自压缩包解包（true 时 `path` 指向临时解包目录）
    pub from_archive: bool,
    /// KB 中是否已存在相同 source_path 的文档（导入时按 `conflict` 策略处理）
    pub exists: bool,
}

/// 目录预扫描结果（导入前预览：文件清单 + 统计 + 与 KB 现有文档的重叠情况）。
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DirectoryScanResult {
    pub directory_path: String,
    pub recursive: bool,
    /// 可导入文件总数（含压缩包解包出的文件）
    pub total_count: usize,
    /// 被跳过的文件数（隐藏项 / 不支持的扩展名 / ignore 命中 / 解包失败）
    pub skipped_count: usize,
    pub skipped: Vec<String>,
    pub files: Vec<DirectoryScanFile>,
    /// KB 中已存在相同 source_path 的文档数（`conflict=skip` 时这些文件将被跳过）
    pub existing_count: usize,
    /// 实际使用的嵌入模型 provider（None 表示未配置，导入后不会自动索引）
    pub embedding_provider: Option<String>,
}

/// document-parser 支持解析的扩展名；目录导入仅收录这些类型。
/// 收口到 document-parser 的 `SUPPORTED_EXTENSIONS` 权威白名单，避免双源漂移。
fn is_supported_knowledge_ext(ext: &str) -> bool {
    axagent_document_parser::is_supported_ext(ext)
}

/// 压缩包解包上限：单次导入累计解包体积（200MB）。
const ARCHIVE_MAX_UNPACK_BYTES: u64 = 200 * 1024 * 1024;
/// 压缩包解包上限：单次导入累计条目数。
const ARCHIVE_MAX_ENTRIES: usize = 5000;
/// 压缩包解包上限：嵌套解包深度（1 = 压缩包内再套压缩包）。
const ARCHIVE_MAX_DEPTH: u32 = 2;
/// 导入会话临时解包根目录前缀；残留目录在应用启动时清理一次。
const IMPORT_TMP_ROOT_PREFIX: &str = "axagent-import-";

/// 目录导入默认排除的「噪音」目录名（大小写不敏感）。
/// 通常为构建产物 / 依赖 / 版本控制元数据，导入会污染知识库。
const NOISE_DIRS: &[&str] = &[
    "node_modules",
    ".git",
    ".svn",
    ".hg",
    "dist",
    "build",
    "target",
    "out",
    "__pycache__",
    ".venv",
    "venv",
    ".idea",
    ".vscode",
    "coverage",
    ".next",
    ".nuxt",
    ".cache",
    "vendor",
    "bower_components",
    "Pods",
];

/// 判断文件是否为受支持的压缩包（.zip / .tar.gz / .tgz）。
fn is_archive_path(path: &std::path::Path) -> bool {
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or_default();
    let lower = name.to_ascii_lowercase();
    lower.ends_with(".zip") || lower.ends_with(".tar.gz") || lower.ends_with(".tgz")
}

/// 解包累计统计（跨嵌套层共享，用于体积 / 条数上限）。
#[derive(Default)]
struct ArchiveStats {
    total_bytes: u64,
    total_entries: usize,
}

/// 目录收集上下文：临时解包根目录 + 压缩包计数器 + 累计解包统计。
struct CollectCtx {
    unpack_root: std::path::PathBuf,
    archive_counter: usize,
    stats: ArchiveStats,
}

impl CollectCtx {
    /// 创建带时间戳的导入临时根目录（每次导入/同步独立，互不干扰）。
    fn new() -> std::io::Result<Self> {
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0);
        let unpack_root = std::env::temp_dir().join(format!("{IMPORT_TMP_ROOT_PREFIX}{ts}"));
        std::fs::create_dir_all(&unpack_root)?;
        Ok(Self { unpack_root, archive_counter: 0, stats: ArchiveStats::default() })
    }
}

/// 将压缩包安全解包到 `dest_dir`。
///
/// 防护：
/// - zip：逐条 `by_index` + `enclosed_name()`，绝对路径 / `..` 逃逸的条目直接拒绝
/// - tar.gz / tgz：`tar::Archive` + `GzDecoder`，`unpack_in` 自带 zip-slip 防护
/// - 累计条数 / 累计解包体积 / 嵌套深度超限一律返回 `Err`
/// - 嵌套压缩包递归解包到 `_nested_<i>` 子目录，解包成功后删除原压缩包，
///   避免后续目录扫描重复解包
fn unpack_archive_to(
    archive_path: &std::path::Path,
    dest_dir: &std::path::Path,
    depth: u32,
    stats: &mut ArchiveStats,
) -> std::io::Result<()> {
    if depth > ARCHIVE_MAX_DEPTH {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("压缩包嵌套深度超过上限（{ARCHIVE_MAX_DEPTH} 层）"),
        ));
    }
    let name = archive_path.file_name().and_then(|n| n.to_str()).unwrap_or_default();
    if name.to_ascii_lowercase().ends_with(".zip") {
        unpack_zip(archive_path, dest_dir, depth, stats)
    } else {
        unpack_tar_gz(archive_path, dest_dir, depth, stats)
    }
}

fn unpack_zip(
    archive_path: &std::path::Path,
    dest_dir: &std::path::Path,
    depth: u32,
    stats: &mut ArchiveStats,
) -> std::io::Result<()> {
    let file = std::fs::File::open(archive_path)?;
    let mut archive = zip::ZipArchive::new(file).map_err(|e| {
        std::io::Error::new(std::io::ErrorKind::InvalidData, format!("无法解析 ZIP: {e}"))
    })?;

    for i in 0..archive.len() {
        let mut entry = archive.by_index(i).map_err(|e| {
            std::io::Error::new(std::io::ErrorKind::InvalidData, format!("读取 ZIP 条目失败: {e}"))
        })?;

        // 路径逃逸防护：enclosed_name 返回 None 表示条目越界（绝对路径 / `..`）
        let Some(out_path) = entry.enclosed_name() else {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("ZIP 条目路径越界，已拒绝: {}", entry.name()),
            ));
        };

        stats.total_entries += 1;
        if stats.total_entries > ARCHIVE_MAX_ENTRIES {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("压缩包条目数超过上限（{ARCHIVE_MAX_ENTRIES}）"),
            ));
        }
        stats.total_bytes = stats.total_bytes.saturating_add(entry.size());
        if stats.total_bytes > ARCHIVE_MAX_UNPACK_BYTES {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("解包体积超过上限（{ARCHIVE_MAX_UNPACK_BYTES} 字节）"),
            ));
        }

        let full = dest_dir.join(&out_path);
        if entry.is_dir() {
            std::fs::create_dir_all(&full)?;
            continue;
        }
        if let Some(parent) = full.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut out = std::fs::File::create(&full)?;
        std::io::copy(&mut entry, &mut out)?;

        // 嵌套压缩包：递归解包后删除原文件，避免目录扫描时重复处理
        if is_archive_path(&full) {
            let nested = dest_dir.join(format!("_nested_{i}"));
            std::fs::create_dir_all(&nested)?;
            unpack_archive_to(&full, &nested, depth + 1, stats)?;
            let _ = std::fs::remove_file(&full);
        }
    }
    Ok(())
}

fn unpack_tar_gz(
    archive_path: &std::path::Path,
    dest_dir: &std::path::Path,
    depth: u32,
    stats: &mut ArchiveStats,
) -> std::io::Result<()> {
    let file = std::fs::File::open(archive_path)?;
    let decoder = flate2::read::GzDecoder::new(file);
    let mut archive = tar::Archive::new(decoder);
    let entries = archive.entries().map_err(|e| {
        std::io::Error::new(std::io::ErrorKind::InvalidData, format!("无法解析 tar: {e}"))
    })?;

    for (i, entry) in entries.enumerate() {
        let mut entry = entry?;

        stats.total_entries += 1;
        if stats.total_entries > ARCHIVE_MAX_ENTRIES {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("压缩包条目数超过上限（{ARCHIVE_MAX_ENTRIES}）"),
            ));
        }
        stats.total_bytes = stats.total_bytes.saturating_add(entry.size());
        if stats.total_bytes > ARCHIVE_MAX_UNPACK_BYTES {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("解包体积超过上限（{ARCHIVE_MAX_UNPACK_BYTES} 字节）"),
            ));
        }

        // unpack_in 自带 zip-slip 防护：条目路径越界返回 Err
        entry.unpack_in(dest_dir)?;

        // 嵌套压缩包：递归解包后删除原文件
        let entry_path = entry.path()?.to_path_buf();
        if is_archive_path(&entry_path) {
            let full = dest_dir.join(&entry_path);
            if full.exists() {
                let nested = dest_dir.join(format!("_nested_{i}"));
                std::fs::create_dir_all(&nested)?;
                unpack_archive_to(&full, &nested, depth + 1, stats)?;
                let _ = std::fs::remove_file(&full);
            }
        }
    }
    Ok(())
}

/// 清理所有遗留的目录导入临时解包目录（应用启动时调用一次）。
/// 崩溃 / 强杀可能导致残留，静默清理失败仅记日志，不阻塞启动。
pub fn cleanup_import_tmp_dirs() {
    if let Ok(entries) = std::fs::read_dir(std::env::temp_dir()) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with(IMPORT_TMP_ROOT_PREFIX)
                && entry.file_type().map(|t| t.is_dir()).unwrap_or(false)
            {
                if let Err(e) = std::fs::remove_dir_all(entry.path()) {
                    tracing::warn!(
                        target: "knowledge",
                        "清理目录导入临时目录失败: {}: {e}",
                        entry.path().display()
                    );
                }
            }
        }
    }
}

/// 将用户 ignore 模式编译为 globset（非法模式静默忽略，不阻断导入）。
fn build_ignore_set(ignore_patterns: &Option<Vec<String>>) -> Option<globset::GlobSet> {
    let patterns = ignore_patterns.as_ref()?;
    let mut builder = globset::GlobSetBuilder::new();
    let mut valid = false;
    for p in patterns {
        if let Ok(glob) = globset::Glob::new(p) {
            builder.add(glob);
            valid = true;
        }
    }
    if valid { builder.build().ok() } else { None }
}

/// 相对路径是否匹配 ignore 模式（统一 POSIX 风格，`\` → `/`）。
fn is_ignored(
    path: &std::path::Path,
    root: &std::path::Path,
    ignore_set: &Option<globset::GlobSet>,
) -> bool {
    let Some(set) = ignore_set else {
        return false;
    };
    let rel = path.strip_prefix(root).unwrap_or(path);
    let rel_str = rel.to_string_lossy().replace('\\', "/");
    set.is_match(rel_str)
}

/// 收集目录下的可导入文件，跳过隐藏文件/目录与不支持的扩展名。
/// `extensions` 指定时仅收录该白名单内的扩展名，否则使用 [`is_supported_knowledge_ext`]。
/// `ignore_patterns` 为文件级 glob 排除模式（相对目录根匹配）。
/// 遇压缩包（.zip / .tar.gz / .tgz）时安全解包到临时目录并收集内部文件（含嵌套压缩包）。
fn collect_importable_files(
    dir: &std::path::Path,
    recursive: bool,
    extensions: &Option<Vec<String>>,
    ignore_patterns: &Option<Vec<String>>,
    files: &mut Vec<PathBuf>,
    skipped: &mut Vec<String>,
) -> std::io::Result<()> {
    let _ = collect_importable_files_owned(
        dir,
        recursive,
        extensions,
        ignore_patterns,
        files,
        skipped,
    )?;
    Ok(())
}

/// [`collect_importable_files`] 的 owned 版本：额外返回本次会话的临时解包根目录。
///
/// 调用方负责在不再需要时清理（如预扫描场景用 `std::fs::remove_dir_all`）。
/// 注意：目录导入 / 同步场景**必须保留**该目录 —— 解包出的文件由异步索引任务
/// 按绝对路径读取，删除会导致索引失败；残留目录由应用启动时统一清理。
fn collect_importable_files_owned(
    dir: &std::path::Path,
    recursive: bool,
    extensions: &Option<Vec<String>>,
    ignore_patterns: &Option<Vec<String>>,
    files: &mut Vec<PathBuf>,
    skipped: &mut Vec<String>,
) -> std::io::Result<std::path::PathBuf> {
    let mut ctx = CollectCtx::new()?;
    let unpack_root = ctx.unpack_root.clone();
    let ignore_set = build_ignore_set(ignore_patterns);
    collect_importable_files_inner(
        dir,
        recursive,
        extensions,
        &ignore_set,
        files,
        skipped,
        0,
        &mut ctx,
    )?;
    Ok(unpack_root)
}

/// [`collect_importable_files`] 的递归实现。
/// `archive_depth` 表示当前扫描目录所处的解包层级（用户目录 = 0）。
fn collect_importable_files_inner(
    dir: &std::path::Path,
    recursive: bool,
    extensions: &Option<Vec<String>>,
    ignore_set: &Option<globset::GlobSet>,
    files: &mut Vec<PathBuf>,
    skipped: &mut Vec<String>,
    archive_depth: u32,
    ctx: &mut CollectCtx,
) -> std::io::Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        let file_type = entry.file_type()?;

        // 跳过隐藏项（如 .git / .DS_Store）
        if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
            if name.starts_with('.') {
                continue;
            }
        }

        if file_type.is_dir() {
            // 噪音目录（构建产物 / 依赖 / VCS 元数据）直接整目录跳过
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                if NOISE_DIRS.iter().any(|n| n.eq_ignore_ascii_case(name)) {
                    continue;
                }
            }
            if recursive {
                collect_importable_files_inner(
                    &path,
                    recursive,
                    extensions,
                    ignore_set,
                    files,
                    skipped,
                    archive_depth,
                    ctx,
                )?;
            }
        } else if file_type.is_file() {
            // 压缩包：解包到临时目录后收集内部文件（解包目录强制全量递归）
            if is_archive_path(&path) {
                ctx.archive_counter += 1;
                let dest_dir = ctx.unpack_root.join(format!("archive_{}", ctx.archive_counter));
                std::fs::create_dir_all(&dest_dir)?;
                match unpack_archive_to(&path, &dest_dir, archive_depth + 1, &mut ctx.stats) {
                    Ok(()) => {
                        collect_importable_files_inner(
                            &dest_dir,
                            true,
                            extensions,
                            ignore_set,
                            files,
                            skipped,
                            archive_depth + 1,
                            ctx,
                        )?;
                    },
                    Err(e) => {
                        // 解包失败 / 超限：计入 skipped，不中断整个导入
                        skipped.push(format!("{}: 解包失败（{e}）", path.to_string_lossy()));
                    },
                }
                continue;
            }
            // ignore 模式命中的文件跳过
            if is_ignored(&path, dir, ignore_set) {
                skipped.push(path.to_string_lossy().to_string());
                continue;
            }
            let ext = path.extension().and_then(|e| e.to_str()).map(|s| s.to_ascii_lowercase());
            let allowed = match extensions {
                Some(exts) => ext
                    .as_ref()
                    .map(|e| exts.iter().any(|x| x.eq_ignore_ascii_case(e)))
                    .unwrap_or(false),
                None => ext.as_deref().map(is_supported_knowledge_ext).unwrap_or(false),
            };
            if allowed {
                files.push(path);
            } else {
                skipped.push(path.to_string_lossy().to_string());
            }
        }
    }
    Ok(())
}

#[agent_command(domain = knowledge, safety = Safe, call_mode = StateOnly, description = "列出知识库")]
#[tauri::command]
pub async fn list_knowledge_bases(
    state: State<'_, AppState>,
) -> Result<Vec<KnowledgeBase>, String> {
    axagent_dao::repo::knowledge::list_knowledge_bases(state.harness.db()).await.map_err(|e| {
        String::from(crate::commands::error::ErrorResponse::from_error(
            e,
            crate::commands::error::ErrorCategory::Unrecoverable,
        ))
    })
}

#[agent_command(domain = knowledge, safety = Caution, call_mode = StateOnly, description = "创建知识库")]
#[tauri::command]
pub async fn create_knowledge_base(
    state: State<'_, AppState>,
    input: CreateKnowledgeBaseInput,
) -> Result<KnowledgeBase, String> {
    axagent_dao::repo::knowledge::create_knowledge_base(state.harness.db(), input).await.map_err(
        |e| {
            String::from(crate::commands::error::ErrorResponse::from_error(
                e,
                crate::commands::error::ErrorCategory::Unrecoverable,
            ))
        },
    )
}

#[agent_command(domain = knowledge, safety = Caution, call_mode = StateOnly, description = "更新知识库")]
#[tauri::command]
pub async fn update_knowledge_base(
    state: State<'_, AppState>,
    id: String,
    input: UpdateKnowledgeBaseInput,
) -> Result<KnowledgeBase, String> {
    axagent_dao::repo::knowledge::update_knowledge_base(state.harness.db(), &id, input)
        .await
        .map_err(|e| {
            String::from(crate::commands::error::ErrorResponse::from_error(
                e,
                crate::commands::error::ErrorCategory::Unrecoverable,
            ))
        })
}

#[agent_command(domain = knowledge, safety = Dangerous, call_mode = StateOnly, description = "删除知识库")]
#[tauri::command]
pub async fn delete_knowledge_base(state: State<'_, AppState>, id: String) -> Result<(), String> {
    // 校验 base_id 格式，防止 SQL 注入（与 list_memory_items 一致的规则）
    if id.is_empty()
        || id.len() > 128
        || id.contains(|c: char| !c.is_alphanumeric() && c != '-' && c != '_')
    {
        return Err(String::from(crate::commands::error::ErrorResponse::from_error(
            "Invalid base_id: must be 1-128 alphanumeric/hyphen/underscore characters",
            crate::commands::error::ErrorCategory::Unrecoverable,
        )));
    }

    // Delete vector collection (vec_kb_{id} and vec_kb_{id}_meta tables)
    let collection_id = format!("kb_{}", id);
    let _ = state.vector_store.delete_collection(&collection_id).await;

    // 若为 ConnectedVault 类型，注销全局 VaultRegistry 中的绑定
    axagent_tools::tools::obsidian::unregister_vault(&id);

    axagent_dao::repo::knowledge::delete_knowledge_base(state.harness.db(), &id).await.map_err(
        |e| {
            String::from(crate::commands::error::ErrorResponse::from_error(
                e,
                crate::commands::error::ErrorCategory::Unrecoverable,
            ))
        },
    )
}

/// 将已有 KB 转换为 ConnectedVault 类型，并绑定 Obsidian vault 路径
///
/// 用法场景：用户先创建了一个普通 KB，后来决定让它指向 Obsidian vault。
/// 转换后该 KB 不再走 RAG 索引，agent 通过 9 个 `obsidian_*` 工具直接读写。
#[agent_command(domain = knowledge, safety = Caution, call_mode = StateOnly, description = "连接Obsidian Vault")]
#[tauri::command]
pub async fn kb_connect_vault(
    state: State<'_, AppState>,
    id: String,
    vault_path: String,
) -> Result<KnowledgeBase, String> {
    let path = std::path::Path::new(&vault_path);
    if !path.is_absolute() {
        return Err(crate::commands::error::ErrorResponse::err_with_detail(
            crate::commands::error_code::common::INVALID_INPUT,
            "vault_path must be an absolute path",
        ));
    }
    if !path.is_dir() {
        return Err(crate::commands::error::ErrorResponse::err_with_detail(
            crate::commands::error_code::common::INVALID_INPUT,
            format!("vault_path is not a directory: {vault_path}"),
        ));
    }

    let kb = axagent_dao::repo::knowledge::get_knowledge_base(state.harness.db(), &id)
        .await
        .map_err(|e| {
            String::from(crate::commands::error::ErrorResponse::from_error(
                e,
                crate::commands::error::ErrorCategory::Unrecoverable,
            ))
        })?;

    // 直接更新 kind/vault_path 字段（通过 update_knowledge_base 走 DAO）
    let updated = axagent_dao::repo::knowledge::set_vault_binding(
        state.harness.db(),
        &id,
        axagent_harness::KbKind::ConnectedVault,
        Some(vault_path.clone()),
    )
    .await
    .map_err(|e| {
        String::from(crate::commands::error::ErrorResponse::from_error(
            e,
            crate::commands::error::ErrorCategory::Unrecoverable,
        ))
    })?;

    // 注册到全局 VaultRegistry
    if let Err(e) =
        axagent_tools::tools::obsidian::register_vault(&id, std::path::PathBuf::from(&vault_path))
    {
        tracing::warn!(kb_id = %id, error = %e, "Failed to register Obsidian vault after connect");
    }

    let _ = kb; // 保留原始 KB 引用便于未来审计
    Ok(updated)
}

#[agent_command(domain = knowledge, safety = Caution, call_mode = StateOnly, description = "断开Obsidian Vault")]
/// 解除 KB 的 Obsidian vault 绑定，转换回默认 Indexed 类型
#[tauri::command]
pub async fn kb_disconnect_vault(
    state: State<'_, AppState>,
    id: String,
) -> Result<KnowledgeBase, String> {
    let updated = axagent_dao::repo::knowledge::set_vault_binding(
        state.harness.db(),
        &id,
        axagent_harness::KbKind::Indexed,
        None,
    )
    .await
    .map_err(|e| {
        String::from(crate::commands::error::ErrorResponse::from_error(
            e,
            crate::commands::error::ErrorCategory::Unrecoverable,
        ))
    })?;

    axagent_tools::tools::obsidian::unregister_vault(&id);
    Ok(updated)
}

#[agent_command(domain = knowledge, safety = Caution, call_mode = StateOnly, description = "重排序知识库")]
#[tauri::command]
pub async fn reorder_knowledge_bases(
    state: State<'_, AppState>,
    base_ids: Vec<String>,
) -> Result<(), String> {
    axagent_dao::repo::knowledge::reorder_knowledge_bases(state.harness.db(), &base_ids)
        .await
        .map_err(|e| {
            String::from(crate::commands::error::ErrorResponse::from_error(
                e,
                crate::commands::error::ErrorCategory::Unrecoverable,
            ))
        })
}

#[agent_command(domain = knowledge, safety = Safe, call_mode = StateOnly, description = "列出知识库文档")]
#[tauri::command]
pub async fn list_knowledge_documents(
    state: State<'_, AppState>,
    base_id: String,
) -> Result<Vec<KnowledgeDocument>, String> {
    axagent_dao::repo::knowledge::list_documents(state.harness.db(), &base_id).await.map_err(|e| {
        String::from(crate::commands::error::ErrorResponse::from_error(
            e,
            crate::commands::error::ErrorCategory::Unrecoverable,
        ))
    })
}

#[agent_command(domain = knowledge, safety = Caution, call_mode = StateOnly, description = "添加知识库文档")]
#[tauri::command]
pub async fn add_knowledge_document(
    app: AppHandle,
    state: State<'_, AppState>,
    base_id: String,
    title: String,
    source_path: String,
    mime_type: String,
) -> Result<KnowledgeDocument, String> {
    let doc = axagent_dao::repo::knowledge::add_document(
        state.harness.db(),
        &base_id,
        &title,
        &source_path,
        &mime_type,
        None, // doc_type defaults to "file"
    )
    .await
    .map_err(|e| {
        String::from(crate::commands::error::ErrorResponse::from_error(
            e,
            crate::commands::error::ErrorCategory::Unrecoverable,
        ))
    })?;

    // 将文档状态标记为pending（等待队列处理）
    let kb = axagent_dao::repo::knowledge::get_knowledge_base(state.harness.db(), &base_id)
        .await
        .map_err(|e| {
        String::from(crate::commands::error::ErrorResponse::from_error(
            e,
            crate::commands::error::ErrorCategory::Unrecoverable,
        ))
    })?;

    if kb.embedding_provider.is_some() {
        let _ = axagent_dao::repo::knowledge::update_document_status(
            state.harness.db(),
            &doc.id,
            "pending",
        )
        .await;
        if let Err(e) = crate::index_queue::enqueue_job_sync(
            &state,
            &app,
            jobs::JOB_TYPE_INDEX_DOCUMENT,
            "kb",
            &base_id,
            &doc.id,
            None,
            None,
        ) {
            // 入队失败时回滚状态到 "skipped"，避免文档永久卡在 pending
            let _ = axagent_dao::repo::knowledge::update_document_status_with_error(
                state.harness.db(),
                &doc.id,
                "skipped",
                Some(&format!("enqueue failed: {e}")),
            )
            .await;
            return Err(String::from(crate::commands::error::ErrorResponse::from_error(
                e,
                crate::commands::error::ErrorCategory::Unrecoverable,
            )));
        }
    }

    Ok(doc)
}

/// 目录导入进度回调：`(phase, processed, total, imported, skipped, failed)`。
/// 六参数签名语义明确，抽别名仅为满足 `clippy::type_complexity`（内联形式无法通过）。
#[allow(clippy::type_complexity)]
type ImportProgress<'a> =
    Option<&'a mut (dyn FnMut(&str, usize, usize, usize, usize, usize) + Send)>;

/// 目录导入核心实现（同步命令与异步任务共用）。
///
/// - `conflict`：与 KB 已有文档同名时的处理策略（`Skip` 跳过 / `Overwrite` 删旧重加）
/// - `cancel_token`：异步任务传入取消令牌，每处理一个文档前检查；`None` = 同步调用不取消
/// - `on_progress`：进度回调（阶段, 已处理, 总数, 已导入, 已跳过, 失败）；`None` = 同步调用不上报
///
/// 返回 `(result, cancelled)`：`cancelled` 为 true 表示因取消提前结束。
#[allow(clippy::too_many_arguments)]
async fn run_directory_import_impl(
    app: &AppHandle,
    state: &AppState,
    base_id: &str,
    directory_path: &str,
    recursive: bool,
    extensions: &Option<Vec<String>>,
    ignore_patterns: &Option<Vec<String>>,
    conflict: ConflictPolicy,
    generate_markdown: bool,
    vault_id: Option<&str>,
    cancel_token: Option<&tokio_util::sync::CancellationToken>,
    mut on_progress: ImportProgress<'_>,
) -> Result<(ImportDirectoryResult, bool), String> {
    let dir = PathBuf::from(directory_path);
    if !dir.exists() || !dir.is_dir() {
        return Err(crate::commands::error::ErrorResponse::err_with_detail(
            crate::commands::error_code::common::INVALID_INPUT,
            format!("路径不存在或不是目录: {directory_path}"),
        ));
    }

    let mut files = Vec::new();
    let mut skipped = Vec::new();
    collect_importable_files(
        &dir,
        recursive,
        extensions,
        ignore_patterns,
        &mut files,
        &mut skipped,
    )
    .map_err(|e| {
        crate::commands::error::ErrorResponse::err_with_detail(
            crate::commands::error_code::knowledge::IMPORT_DIR_FAILED,
            format!("读取目录失败 {directory_path}: {e}"),
        )
    })?;

    let kb = axagent_dao::repo::knowledge::get_knowledge_base(state.harness.db(), base_id)
        .await
        .map_err(|e| {
            String::from(crate::commands::error::ErrorResponse::from_error(
                e,
                crate::commands::error::ErrorCategory::Unrecoverable,
            ))
        })?;
    let has_embedding = kb.embedding_provider.is_some();

    // Wiki 副本目标：generate_markdown 开启时预加载 vault 根路径（一次查询，供每文件复用）。
    // 校验失败直接报错，避免循环内逐文件重复报错。
    let wiki_copy_target: Option<(String, String)> = if generate_markdown {
        let v = vault_id.filter(|v| !v.is_empty()).ok_or_else(|| {
            crate::commands::error::ErrorResponse::err_with_detail(
                crate::commands::error_code::common::INVALID_INPUT,
                "generateMarkdown 开启时必须提供 vaultId",
            )
        })?;
        let wiki = axagent_dao::repo::wiki::get_wiki(state.harness.db(), v).await.map_err(|e| {
            String::from(crate::commands::error::ErrorResponse::from_error(
                e,
                crate::commands::error::ErrorCategory::Unrecoverable,
            ))
        })?;
        if wiki.root_path.trim().is_empty() {
            return Err(crate::commands::error::ErrorResponse::err_with_detail(
                crate::commands::error_code::common::INVALID_INPUT,
                format!("Wiki {} 未配置根目录，无法生成 Markdown 副本", wiki.name),
            ));
        }
        Some((v.to_string(), wiki.root_path))
    } else {
        None
    };

    // KB 现有文档 → source_path 索引（与 sync 一致的大小写不敏感 key），
    // 用于冲突检测（Skip 跳过 / Overwrite 删旧重加）。
    let existing_docs = axagent_dao::repo::knowledge::list_documents(state.harness.db(), base_id)
        .await
        .map_err(|e| {
            String::from(crate::commands::error::ErrorResponse::from_error(
                e,
                crate::commands::error::ErrorCategory::Unrecoverable,
            ))
        })?;
    let mut doc_by_path: HashMap<String, &KnowledgeDocument> = HashMap::new();
    for doc in &existing_docs {
        doc_by_path.insert(doc.source_path.to_ascii_lowercase(), doc);
    }
    // 内容指纹索引（去重用）：新路径文件的内容与 KB 已有文档相同 → 跳过（duplicate）。
    // 旧数据 content_hash 为空串不入索引。
    let doc_by_hash: HashMap<String, &KnowledgeDocument> = existing_docs
        .iter()
        .filter(|d| !d.content_hash.is_empty())
        .map(|d| (d.content_hash.clone(), d))
        .collect();
    let doc_mtimes =
        axagent_dao::repo::knowledge::get_document_mtime_map(state.harness.db(), base_id)
            .await
            .map_err(|e| {
                String::from(crate::commands::error::ErrorResponse::from_error(
                    e,
                    crate::commands::error::ErrorCategory::Unrecoverable,
                ))
            })?;

    let total = files.len();
    if let Some(cb) = on_progress.as_deref_mut() {
        cb("scan", 0, total, 0, skipped.len(), 0);
    }

    let mut result = ImportDirectoryResult {
        base_id: base_id.to_string(),
        imported_count: 0,
        skipped_count: 0,
        error_count: 0,
        entity_count: 0,
        relation_count: 0,
        embedding_provider: kb.embedding_provider.clone(),
        imported: Vec::new(),
        skipped,
        errors: Vec::new(),
    };

    for (i, path) in files.into_iter().enumerate() {
        // 取消检查：已请求取消则提前结束，剩余文件不处理
        if let Some(token) = cancel_token {
            if token.is_cancelled() {
                result.skipped_count = result.skipped.len();
                return Ok((result, true));
            }
        }

        let abs = path.to_string_lossy().to_string();
        let mime = axagent_document_parser::mime_from_extension(&path).to_string();

        // 递归导入时用相对路径作为标题，避免重名；非递归用文件名
        let title = if recursive {
            path.strip_prefix(&dir).map(|p| p.to_string_lossy().replace('\\', "/")).unwrap_or_else(
                |_| path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default(),
            )
        } else {
            path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default()
        };

        // 冲突检测：KB 中已有同 source_path 文档
        if let Some(existing) = doc_by_path.get(&abs.to_ascii_lowercase()) {
            match conflict {
                ConflictPolicy::Skip => {
                    // 跳过已存在的文档，不触发重复写入
                    result.skipped_count += 1;
                    result.skipped.push(format!("{abs}: 已存在，跳过"));
                    if let Some(cb) = on_progress.as_deref_mut() {
                        cb(
                            "import",
                            i + 1,
                            total,
                            result.imported_count,
                            result.skipped_count,
                            result.error_count,
                        );
                    }
                    continue;
                },
                ConflictPolicy::Overwrite => {
                    // 增量比对：size + mtime 均未变化 → 跳过，避免对未变更文件全量重索引
                    let fmeta = std::fs::metadata(&path).ok();
                    let fsize = fmeta.as_ref().map(|m| m.len() as i64).unwrap_or(-1);
                    let fmtime = fmeta
                        .as_ref()
                        .and_then(|m| m.modified().ok())
                        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                        .map(|d| d.as_secs() as i64)
                        .unwrap_or(0);
                    let size_unchanged = existing.size_bytes == fsize;
                    let doc_mtime = doc_mtimes.get(&existing.id).copied().unwrap_or(0);
                    let mtime_unchanged = doc_mtime > 0 && fmtime > 0 && fmtime <= doc_mtime;
                    if size_unchanged && mtime_unchanged {
                        result.skipped_count += 1;
                        result.skipped.push(format!("{abs}: 内容未变化，跳过"));
                        if let Some(cb) = on_progress.as_deref_mut() {
                            cb(
                                "import",
                                i + 1,
                                total,
                                result.imported_count,
                                result.skipped_count,
                                result.error_count,
                            );
                        }
                        continue;
                    }
                    // 内容变化 → 删旧（向量 + DB 记录）+ 加新，保证磁盘内容与 KB 一致
                    let doc_id = existing.id.clone();
                    let collection_id = format!("kb_{}", base_id);
                    if let Err(e) =
                        state.vector_store.delete_document_embeddings(&collection_id, &doc_id).await
                    {
                        result.error_count += 1;
                        result.errors.push(ImportDirectoryError::with_code(
                            abs,
                            format!("清理旧文档向量失败: {e}"),
                            crate::commands::error_code::knowledge::VECTOR_STORE_FAILED,
                        ));
                        if let Some(cb) = on_progress.as_deref_mut() {
                            cb(
                                "import",
                                i + 1,
                                total,
                                result.imported_count,
                                result.skipped_count,
                                result.error_count,
                            );
                        }
                        continue;
                    }
                    if let Err(e) =
                        axagent_dao::repo::knowledge::delete_document(state.harness.db(), &doc_id)
                            .await
                    {
                        result.error_count += 1;
                        result.errors.push(ImportDirectoryError::with_code(
                            abs,
                            format!("删除旧文档失败: {e}"),
                            crate::commands::error_code::knowledge::DELETE_DOCUMENT_FAILED,
                        ));
                        if let Some(cb) = on_progress.as_deref_mut() {
                            cb(
                                "import",
                                i + 1,
                                total,
                                result.imported_count,
                                result.skipped_count,
                                result.error_count,
                            );
                        }
                        continue;
                    }
                },
            }
        }

        // 内容去重：新文件（source_path 未冲突）的内容指纹与 KB 已有文档相同 → 视为重复，
        // 跳过入库（同 hash → duplicate）。hash 为空串（不可读 / 旧数据无指纹）则跳过此判定。
        // 注：此处计算与 add_document 内部各读一次文件；文档导入场景可接受，正确性优先。
        let fhash = axagent_dao::repo::knowledge::file_sha256(&path);
        if !fhash.is_empty() && doc_by_hash.contains_key(&fhash) {
            result.skipped_count += 1;
            result.skipped.push(format!("{abs}: 内容与已有文档重复，跳过"));
            if let Some(cb) = on_progress.as_deref_mut() {
                cb(
                    "import",
                    i + 1,
                    total,
                    result.imported_count,
                    result.skipped_count,
                    result.error_count,
                );
            }
            continue;
        }

        match axagent_dao::repo::knowledge::add_document(
            state.harness.db(),
            base_id,
            &title,
            &abs,
            &mime,
            None,
        )
        .await
        {
            Ok(doc) => {
                if has_embedding {
                    let _ = axagent_dao::repo::knowledge::update_document_status(
                        state.harness.db(),
                        &doc.id,
                        "pending",
                    )
                    .await;
                    if let Err(e) = crate::index_queue::enqueue_job_sync(
                        state,
                        app,
                        jobs::JOB_TYPE_INDEX_DOCUMENT,
                        "kb",
                        base_id,
                        &doc.id,
                        None,
                        None,
                    ) {
                        // 入队失败时回滚状态到 "skipped"，避免文档永久卡在 pending
                        let _ = axagent_dao::repo::knowledge::update_document_status_with_error(
                            state.harness.db(),
                            &doc.id,
                            "skipped",
                            Some(&format!("enqueue failed: {e}")),
                        )
                        .await;
                        tracing::warn!("[knowledge] 目录导入入队索引失败 {}: {}", doc.id, e);
                    }
                }
                result.imported_count += 1;
                result.imported.push(doc);
                // Wiki 副本：generateMarkdown 开启时同步生成 vault 侧 Markdown 并建笔记。
                // 单文件副本失败只记错误不中断导入，其余文件照常处理。
                if let Some((vault_id, vault_root)) = &wiki_copy_target {
                    if let Err(e) = write_wiki_markdown_copy(
                        state, app, vault_id, vault_root, &dir, &path, recursive, &mime,
                    )
                    .await
                    {
                        result.error_count += 1;
                        result.errors.push(ImportDirectoryError::with_code(
                            abs,
                            format!("生成 Wiki 副本失败: {e}"),
                            crate::commands::error_code::knowledge::IMPORT_DIR_FAILED,
                        ));
                    }
                }
            },
            Err(e) => {
                result.error_count += 1;
                result.errors.push(ImportDirectoryError::with_code(
                    abs,
                    e.to_string(),
                    crate::commands::error_code::knowledge::ADD_DOCUMENT_FAILED,
                ));
            },
        }

        // 进度上报（每处理完一个文件）
        if let Some(cb) = on_progress.as_deref_mut() {
            cb(
                "import",
                i + 1,
                total,
                result.imported_count,
                result.skipped_count,
                result.error_count,
            );
        }
    }

    result.skipped_count = result.skipped.len();

    Ok((result, false))
}

/// 目录导入时生成 Wiki Markdown 副本：提取文本 → 写入 vault 根目录 → 建笔记 → 入队索引。
///
/// - `dir`：导入根目录；`recursive` 为 true 时按「相对 dir 的路径」在 vault 内落位，
///   否则仅以文件名落位到 vault 根（避免非递归导入覆盖同名文件）。
/// - `.md` 源文件原样保留路径，其余扩展名追加 `.md`，保证 vault 侧都是可渲染的 Markdown。
/// - 正文为一级标题 + 提取文本；文本提取失败直接报错（该文件的副本不生成）。
#[allow(clippy::too_many_arguments)]
async fn write_wiki_markdown_copy(
    state: &AppState,
    app: &AppHandle,
    vault_id: &str,
    vault_root: &str,
    dir: &std::path::Path,
    path: &std::path::Path,
    recursive: bool,
    mime: &str,
) -> Result<String, String> {
    // 1. vault 内相对路径（递归保留目录层级；非递归仅文件名）
    let rel = if recursive {
        path.strip_prefix(dir).map(|p| p.to_string_lossy().replace('\\', "/")).unwrap_or_else(
            |_| path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default(),
        )
    } else {
        path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default()
    };
    if rel.is_empty() {
        return Err("无法确定 Wiki 副本的相对路径".to_string());
    }
    let md_rel = if rel.ends_with(".md") {
        rel
    } else {
        format!("{rel}.md")
    };

    // 2. 提取文本作为 Markdown 正文
    let text = axagent_document_parser::extract_text(path, mime)
        .map_err(|e| format!("提取文本失败: {e}"))?;
    let title = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    let content = format!("# {title}\n\n{text}");

    // 3. 写入 vault 根目录下的 Markdown 文件
    let target = std::path::Path::new(vault_root).join(&md_rel);
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("创建目录失败 {}: {e}", parent.display()))?;
    }
    std::fs::write(&target, &content).map_err(|e| format!("写入失败 {}: {e}", target.display()))?;

    // 4. 建笔记（source_refs 记录源文件绝对路径）
    let note = axagent_dao::repo::note::create_note(
        state.harness.db(),
        axagent_dao::repo::note::CreateNoteInput {
            vault_id: vault_id.to_string(),
            title,
            file_path: md_rel.clone(),
            content,
            author: "directory-import".to_string(),
            page_type: None,
            source_refs: Some(vec![path.to_string_lossy().to_string()]),
        },
    )
    .await
    .map_err(|e| format!("创建笔记失败: {e}"))?;

    // 5. 入队笔记索引（与 wiki 笔记同步链路一致）
    crate::index_queue::enqueue_job_sync(
        state,
        app,
        jobs::JOB_TYPE_INDEX_WIKI_NOTE,
        "wiki",
        vault_id,
        &note.id,
        None,
        None,
    )
    .map_err(|e| format!("入队笔记索引失败: {e}"))?;

    Ok(md_rel)
}

/// 批量导入一个目录下的文档到指定知识库（同步，阻塞至全部完成）。
///
/// - `directory_path`：要导入的目录绝对路径
/// - `recursive`：是否递归子目录（默认 false）
/// - `extensions`：可选扩展名白名单（不含点，如 `["md", "txt"]`），未指定则使用支持的类型集
/// - `ignore_patterns`：文件级 glob 排除模式（相对目录根匹配）
/// - `conflict`：与 KB 已有文档冲突时的处理策略（`skip` 跳过默认 / `overwrite` 删旧重加）
/// - `generate_markdown`：是否同时生成 Wiki Markdown 副本（需 `vault_id` 指定目标 Wiki）
/// - `vault_id`：目标 Wiki 的 ID，`generate_markdown` 开启时必填；副本写入其根目录并建笔记
///
/// 仅收录 document-parser 支持的类型；其余文件计入 `skipped`。
/// 若知识库配置了 embedding 提供方，每个文档会被标记为 pending 并入队索引任务。
#[agent_command(domain = knowledge, safety = Caution, call_mode = StateOnly, description = "导入目录到知识库")]
#[tauri::command]
pub async fn import_knowledge_directory(
    app: AppHandle,
    state: State<'_, AppState>,
    base_id: String,
    directory_path: String,
    recursive: Option<bool>,
    extensions: Option<Vec<String>>,
    ignore_patterns: Option<Vec<String>>,
    conflict: Option<ConflictPolicy>,
    generate_markdown: Option<bool>,
    vault_id: Option<String>,
) -> Result<ImportDirectoryResult, String> {
    let (result, _cancelled) = run_directory_import_impl(
        &app,
        state.inner(),
        &base_id,
        &directory_path,
        recursive.unwrap_or(false),
        &extensions,
        &ignore_patterns,
        conflict.unwrap_or(ConflictPolicy::Skip),
        generate_markdown.unwrap_or(false),
        vault_id.as_deref(),
        None,
        None,
    )
    .await?;
    Ok(result)
}

/// 异步目录导入任务的进度上报器：
/// 更新 AppState 中的任务状态（status 命令可查）+ 推送 `knowledge-import-progress` 事件。
struct ProgressEmitter {
    app: AppHandle,
    state: std::sync::Arc<
        tokio::sync::RwLock<
            std::collections::HashMap<String, crate::app_state::KnowledgeImportTaskStatus>,
        >,
    >,
    task_id: String,
}

impl ProgressEmitter {
    fn emit(
        &mut self,
        phase: &str,
        processed: usize,
        total: usize,
        imported: usize,
        skipped: usize,
        failed: usize,
        error: Option<String>,
    ) {
        let status = crate::app_state::KnowledgeImportTaskStatus {
            task_id: self.task_id.clone(),
            phase: phase.to_string(),
            total,
            processed,
            imported,
            skipped,
            failed,
            error,
        };
        // 尽力同步写状态表（try_write 失败时跳过，进度事件才是权威）
        if let Ok(mut guard) = self.state.try_write() {
            guard.insert(self.task_id.clone(), status.clone());
        }
        let _ = self.app.emit("knowledge-import-progress", &status);
    }
}

/// 异步导入一个目录下的文档到指定知识库（后台任务，立即返回 taskId）。
///
/// 进度通过 `knowledge-import-progress` 事件推送；完成时推送 `knowledge-import-completed`
/// （含完整 `ImportDirectoryResult`）。可用 [`cancel_knowledge_import`] 取消，
/// 用 [`get_knowledge_import_status`] 查询运行状态。
/// `conflict`：与 KB 已有文档冲突时的处理策略（`skip` 跳过默认 / `overwrite` 删旧重加）。
/// `generate_markdown`：是否同时生成 Wiki Markdown 副本（需 `vault_id` 指定目标 Wiki）。
#[agent_command(domain = knowledge, safety = Caution, call_mode = StateOnly, description = "异步导入目录到知识库（后台任务）")]
#[tauri::command]
pub async fn import_knowledge_directory_async(
    app: AppHandle,
    state: State<'_, AppState>,
    base_id: String,
    directory_path: String,
    recursive: Option<bool>,
    extensions: Option<Vec<String>>,
    ignore_patterns: Option<Vec<String>>,
    conflict: Option<ConflictPolicy>,
    generate_markdown: Option<bool>,
    vault_id: Option<String>,
) -> Result<String, String> {
    let dir = PathBuf::from(&directory_path);
    if !dir.exists() || !dir.is_dir() {
        return Err(crate::commands::error::ErrorResponse::err_with_detail(
            crate::commands::error_code::common::INVALID_INPUT,
            format!("路径不存在或不是目录: {directory_path}"),
        ));
    }

    let task_id = uuid::Uuid::new_v4().to_string();
    let token = tokio_util::sync::CancellationToken::new();
    state.knowledge_import_cancels.insert(task_id.clone(), token.clone());
    state.knowledge_import_status.write().await.insert(
        task_id.clone(),
        crate::app_state::KnowledgeImportTaskStatus {
            task_id: task_id.clone(),
            phase: "scan".to_string(),
            total: 0,
            processed: 0,
            imported: 0,
            skipped: 0,
            failed: 0,
            error: None,
        },
    );

    let app_handle = app.clone();
    let task_id_clone = task_id.clone();
    tauri::async_runtime::spawn(async move {
        let state = app_handle.state::<AppState>();
        let mut progress = ProgressEmitter {
            app: app_handle.clone(),
            state: state.knowledge_import_status.clone(),
            task_id: task_id_clone.clone(),
        };
        let res = run_directory_import_impl(
            &app_handle,
            state.inner(),
            &base_id,
            &directory_path,
            recursive.unwrap_or(false),
            &extensions,
            &ignore_patterns,
            conflict.unwrap_or(ConflictPolicy::Skip),
            generate_markdown.unwrap_or(false),
            vault_id.as_deref(),
            Some(&token),
            Some(&mut |phase, processed, total, imported, skipped, failed| {
                progress.emit(phase, processed, total, imported, skipped, failed, None);
            }),
        )
        .await;

        // 收尾：移除取消令牌，推送最终状态与完成事件
        state.knowledge_import_cancels.remove(&task_id_clone);
        match res {
            Ok((result, cancelled)) => {
                let phase = if cancelled { "cancelled" } else { "done" };
                let processed = result.imported_count + result.skipped_count + result.error_count;
                progress.emit(
                    phase,
                    processed,
                    processed,
                    result.imported_count,
                    result.skipped_count,
                    result.error_count,
                    None,
                );
                let _ = app_handle.emit(
                    "knowledge-import-completed",
                    &serde_json::json!({
                        "taskId": task_id_clone,
                        "cancelled": cancelled,
                        "result": result,
                    }),
                );
            },
            Err(e) => {
                progress.emit("error", 0, 0, 0, 0, 0, Some(e.clone()));
                tracing::error!("[knowledge] 异步目录导入失败: {e}");
            },
        }
    });

    Ok(task_id)
}

/// 取消一个进行中的异步目录导入任务。
#[agent_command(domain = knowledge, safety = Safe, call_mode = StateOnly, description = "取消异步目录导入任务")]
#[tauri::command]
pub async fn cancel_knowledge_import(
    state: State<'_, AppState>,
    task_id: String,
) -> Result<(), String> {
    if let Some(token) = state.knowledge_import_cancels.get(&task_id) {
        token.cancel();
    }
    Ok(())
}

/// 查询异步目录导入任务的最新运行状态。
#[agent_command(domain = knowledge, safety = Safe, call_mode = StateOnly, description = "查询异步目录导入任务状态")]
#[tauri::command]
pub async fn get_knowledge_import_status(
    state: State<'_, AppState>,
    task_id: String,
) -> Result<Option<crate::app_state::KnowledgeImportTaskStatus>, String> {
    Ok(state.knowledge_import_status.read().await.get(&task_id).cloned())
}

/// 预扫描一个目录（导入前预览）：复用与导入完全相同的收集逻辑，
/// 返回可导入文件清单 + 统计 + 与 KB 现有文档的重叠情况，供前端向导展示。
///
/// 参数与 [`import_knowledge_directory`] 一致，保证「预览即所见，所见即所得」。
/// 预扫描产生的临时解包目录在扫描结束后立即清理（与导入不同——导入需保留
/// 供异步索引任务读取解包文件）。
#[agent_command(domain = knowledge, safety = Safe, call_mode = StateOnly, description = "预扫描目录（导入前预览文件清单）")]
#[tauri::command]
pub async fn scan_knowledge_directory(
    state: State<'_, AppState>,
    base_id: String,
    directory_path: String,
    recursive: Option<bool>,
    extensions: Option<Vec<String>>,
    ignore_patterns: Option<Vec<String>>,
) -> Result<DirectoryScanResult, String> {
    let dir = PathBuf::from(&directory_path);
    if !dir.exists() || !dir.is_dir() {
        return Err(crate::commands::error::ErrorResponse::err_with_detail(
            crate::commands::error_code::knowledge::SCAN_DIR_FAILED,
            format!("路径不存在或不是目录: {directory_path}"),
        ));
    }

    let recursive = recursive.unwrap_or(false);
    let mut files = Vec::new();
    let mut skipped = Vec::new();
    let unpack_root = collect_importable_files_owned(
        &dir,
        recursive,
        &extensions,
        &ignore_patterns,
        &mut files,
        &mut skipped,
    )
    .map_err(|e| {
        crate::commands::error::ErrorResponse::err_with_detail(
            crate::commands::error_code::knowledge::SCAN_DIR_FAILED,
            format!("读取目录失败 {directory_path}: {e}"),
        )
    })?;

    // 校验 KB 存在并取 embedding 配置；同时加载现有文档用于重叠标记
    let kb = axagent_dao::repo::knowledge::get_knowledge_base(state.harness.db(), &base_id)
        .await
        .map_err(|e| {
        String::from(crate::commands::error::ErrorResponse::from_error(
            e,
            crate::commands::error::ErrorCategory::Unrecoverable,
        ))
    })?;
    let existing_docs = axagent_dao::repo::knowledge::list_documents(state.harness.db(), &base_id)
        .await
        .map_err(|e| {
            String::from(crate::commands::error::ErrorResponse::from_error(
                e,
                crate::commands::error::ErrorCategory::Unrecoverable,
            ))
        })?;
    let existing_keys: HashSet<String> =
        existing_docs.iter().map(|d| d.source_path.to_ascii_lowercase()).collect();

    let mut scanned = Vec::with_capacity(files.len());
    let mut existing_count = 0usize;
    for path in &files {
        let abs = path.to_string_lossy().to_string();
        let rel = path
            .strip_prefix(&dir)
            .map(|p| p.to_string_lossy().replace('\\', "/"))
            .unwrap_or_else(|_| abs.clone());
        let exists = existing_keys.contains(&abs.to_ascii_lowercase());
        if exists {
            existing_count += 1;
        }
        scanned.push(DirectoryScanFile {
            path: abs,
            rel_path: rel,
            extension: path
                .extension()
                .and_then(|e| e.to_str())
                .map(|s| s.to_ascii_lowercase())
                .unwrap_or_default(),
            size_bytes: std::fs::metadata(path).map(|m| m.len()).unwrap_or(0),
            from_archive: path.starts_with(&unpack_root),
            exists,
        });
    }

    // 预扫描仅做预览，清理本次解包出的临时目录，避免残留
    let _ = std::fs::remove_dir_all(&unpack_root);

    Ok(DirectoryScanResult {
        directory_path,
        recursive,
        total_count: scanned.len(),
        skipped_count: skipped.len(),
        skipped,
        files: scanned,
        existing_count,
        embedding_provider: kb.embedding_provider.clone(),
    })
}

/// 按导入错误清单重试（多用于修复根因后重试，如磁盘空间 / 文件权限）。
///
/// - `paths`：上一次导入 `errors` 中的 `path` 清单（前端可勾选后提交）
/// - 文件已不存在 / 非文件 → 计入 `skipped`；其余重新 `add_document` + 入队索引
///
/// 返回与 [`import_knowledge_directory`] 相同结构的 `ImportDirectoryResult`。
#[agent_command(domain = knowledge, safety = Caution, call_mode = StateOnly, description = "重试目录导入失败项")]
#[tauri::command]
pub async fn retry_knowledge_import_errors(
    app: AppHandle,
    state: State<'_, AppState>,
    base_id: String,
    paths: Vec<String>,
) -> Result<ImportDirectoryResult, String> {
    let db = state.harness.db();
    let kb = axagent_dao::repo::knowledge::get_knowledge_base(db, &base_id).await.map_err(|e| {
        String::from(crate::commands::error::ErrorResponse::from_error(
            e,
            crate::commands::error::ErrorCategory::Unrecoverable,
        ))
    })?;
    let has_embedding = kb.embedding_provider.is_some();

    let mut result = ImportDirectoryResult {
        base_id: base_id.clone(),
        imported_count: 0,
        skipped_count: 0,
        error_count: 0,
        entity_count: 0,
        relation_count: 0,
        embedding_provider: kb.embedding_provider.clone(),
        imported: Vec::new(),
        skipped: Vec::new(),
        errors: Vec::new(),
    };

    for path_str in paths {
        let path = PathBuf::from(&path_str);
        let abs = path.to_string_lossy().to_string();
        let title = path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| abs.clone());
        let mime = axagent_document_parser::mime_from_extension(&path).to_string();

        // 文件已被移除 / 非文件：无法重试，计入 skipped
        if !path.exists() || !path.is_file() {
            result.skipped.push(format!("{abs}: 文件不存在或已移除"));
            continue;
        }

        match axagent_dao::repo::knowledge::add_document(db, &base_id, &title, &abs, &mime, None)
            .await
        {
            Ok(doc) => {
                if has_embedding {
                    let _ = axagent_dao::repo::knowledge::update_document_status(
                        db, &doc.id, "pending",
                    )
                    .await;
                    if let Err(e) = crate::index_queue::enqueue_job_sync(
                        state.inner(),
                        &app,
                        jobs::JOB_TYPE_INDEX_DOCUMENT,
                        "kb",
                        &base_id,
                        &doc.id,
                        None,
                        None,
                    ) {
                        tracing::warn!("[knowledge] 重试导入入队索引失败 {}: {}", doc.id, e);
                    }
                }
                result.imported_count += 1;
                result.imported.push(doc);
            },
            Err(e) => {
                result.error_count += 1;
                result.errors.push(ImportDirectoryError::with_code(
                    abs,
                    e.to_string(),
                    crate::commands::error_code::knowledge::ADD_DOCUMENT_FAILED,
                ));
            },
        }
    }

    result.skipped_count = result.skipped.len();
    Ok(result)
}

#[agent_command(domain = knowledge, safety = Dangerous, call_mode = StateOnly, description = "删除知识库文档")]
#[tauri::command]
pub async fn delete_knowledge_document(
    state: State<'_, AppState>,
    base_id: String,
    id: String,
) -> Result<(), String> {
    // 向量删除必须成功后才继续删除 DB 记录（2026-09-15 修：此前 `let _ =` 吞错，
    // 于是「文档记录已删、向量还在库中」—— 已删除文档的内容仍会被检索命中）。
    // 失败即返回错误并保留文档记录，用户可重试。
    let collection_id = format!("kb_{}", base_id);
    state.vector_store.delete_document_embeddings(&collection_id, &id).await.map_err(|e| {
        crate::commands::error::ErrorResponse::err_with_detail(
            crate::commands::error_code::knowledge::DELETE_DOCUMENT_FAILED,
            format!("清理文档 {id} 的向量失败: {e}"),
        )
    })?;

    axagent_dao::repo::knowledge::delete_document(state.harness.db(), &id).await.map_err(|e| {
        String::from(crate::commands::error::ErrorResponse::from_error(
            e,
            crate::commands::error::ErrorCategory::Unrecoverable,
        ))
    })
}

#[agent_command(domain = knowledge, safety = Safe, call_mode = StateOnly, description = "搜索知识库")]
#[tauri::command]
pub async fn search_knowledge_base(
    state: State<'_, AppState>,
    base_id: String,
    query: String,
    top_k: Option<usize>,
) -> Result<Vec<axagent_search::vector_store::VectorSearchResult>, String> {
    // 阈值必须先取到，才能交给**检索层**在截断之前过滤（理由见函数末尾说明）。
    let kb = axagent_dao::repo::knowledge::get_knowledge_base(state.harness.db(), &base_id)
        .await
        .map_err(|e| {
        String::from(crate::commands::error::ErrorResponse::from_error(
            e,
            crate::commands::error::ErrorCategory::Unrecoverable,
        ))
    })?;
    // `retrieval_threshold` 是**相关度下限 ∈ [0,1]**，检索层按同一标尺过滤
    // （`combined_score >= 下限`）。换算规则的单一真源在 `axagent_search::rag`。
    let min_similarity =
        axagent_search::rag::similarity_floor_from_threshold(kb.retrieval_threshold.unwrap_or(0.0));

    let results = crate::indexing::search_knowledge(
        state.harness.db(),
        state.harness.master_key(),
        &state.vector_store,
        &base_id,
        &query,
        top_k.unwrap_or(5),
        Some(min_similarity),
    )
    .await
    .map_err(|e| {
        String::from(crate::commands::error::ErrorResponse::from_error(
            e,
            crate::commands::error::ErrorCategory::Unrecoverable,
        ))
    })?;

    // ⚠ 2026-09-15 二次修（过滤位置）：此处原先在结果**返回之后**按
    // `r.score <= distance_ceiling_from_similarity_floor(threshold)` 筛。两个问题：
    // ① **两把尺子** —— 这里用 `r.score`（pipeline 路径下是**重排阶段**分数）去比一个
    //    按**融合分数**（`combined_score`）标定的阈值 ⇒ 合格集非前缀 ⇒ 真会丢中间合格项，
    //    方向还可能整体反（同 #186）。
    //    ⚠ 「已截断成 top_k ⇒ 候选集里二十条过阈值却只检查了五条」这一说法**已被自查
    //    否定**（候选池 `top_k*3`，筛选键 == 排序键 ⇒ 两次序等价，见 `MEMORY-RULES` #205）。
    // ② 同一条规则同时存在于调用方与检索层两处 ⇒ 必然漂移（本文件此前就各写了一份
    //    写死的 `20.0`）。
    // 现在阈值经 `min_similarity` 进入 `HybridSearchOptions.min_score`，由四条收尾
    // 路径在 `truncate(top_k)` **之前**统一处理 ⇒ 过滤只剩一处真源。
    Ok(results)
}

#[agent_command(domain = knowledge, safety = Caution, call_mode = StateOnly, description = "重建知识库索引")]
#[tauri::command]
pub async fn rebuild_knowledge_index(
    app: AppHandle,
    state: State<'_, AppState>,
    base_id: String,
) -> Result<(), String> {
    let kb = axagent_dao::repo::knowledge::get_knowledge_base(state.harness.db(), &base_id)
        .await
        .map_err(|e| {
        String::from(crate::commands::error::ErrorResponse::from_error(
            e,
            crate::commands::error::ErrorCategory::Unrecoverable,
        ))
    })?;

    let embedding_provider = kb.embedding_provider.ok_or_else(|| {
        crate::commands::error::ErrorResponse::err(
            crate::commands::error_code::knowledge::NO_EMBEDDING_PROVIDER,
        )
    })?;

    let collection_id = format!("kb_{}", base_id);

    // Get all documents
    let docs = axagent_dao::repo::knowledge::list_documents(state.harness.db(), &base_id)
        .await
        .map_err(|e| {
            String::from(crate::commands::error::ErrorResponse::from_error(
                e,
                crate::commands::error::ErrorCategory::Unrecoverable,
            ))
        })?;

    if docs.is_empty() {
        let _ = app.emit(
            IpcEventName::KnowledgeRebuildComplete.as_str(),
            serde_json::json!({ "baseId": base_id }),
        );
        return Ok(());
    }

    // Reset all document statuses to "indexing"
    for doc in &docs {
        let _ = axagent_dao::repo::knowledge::update_document_status(
            state.harness.db(),
            &doc.id,
            "indexing",
        )
        .await;
    }

    // Clear only embeddings (vec0), keep _meta intact
    let _ = state.vector_store.clear_embeddings(&collection_id).await;

    let db = state.harness.db().clone();
    let master_key = state.harness.master_key_owned();
    let vector_store = state.vector_store.clone();
    let ep = embedding_provider.clone();
    let provider_registry = state.harness.provider_registry().clone();

    tokio::spawn(catch_unwind_logged("knowledge.batch_index_docs", async move {
        for doc in &docs {
            let chunks = match vector_store.list_document_chunks_raw(&collection_id, &doc.id).await
            {
                Ok(c) => c,
                Err(e) => {
                    let err_msg = e.to_string();
                    let _ = axagent_dao::repo::knowledge::update_document_status_with_error(
                        &db,
                        &doc.id,
                        "failed",
                        Some(&err_msg),
                    )
                    .await;
                    let _ = app.emit(
                        IpcEventName::KnowledgeDocumentIndexed.as_str(),
                        serde_json::json!({
                            "documentId": doc.id,
                            "success": false,
                            "error": err_msg,
                        }),
                    );
                    continue;
                },
            };

            if chunks.is_empty() {
                let _ = axagent_dao::repo::knowledge::update_document_status_with_error(
                    &db, &doc.id, "ready", None,
                )
                .await;
                let _ = app.emit(
                    IpcEventName::KnowledgeDocumentIndexed.as_str(),
                    serde_json::json!({ "documentId": doc.id, "success": true }),
                );
                continue;
            }

            let texts: Vec<String> = chunks.iter().map(|(_, _, content)| content.clone()).collect();
            let rowids: Vec<i64> = chunks.iter().map(|(rid, _, _)| *rid).collect();

            match crate::indexing::generate_embeddings(
                &db,
                &master_key,
                &provider_registry,
                &ep,
                texts,
                None,
            )
            .await
            {
                Ok(embed_response) => {
                    let entries: Vec<(i64, Vec<f32>)> =
                        rowids.into_iter().zip(embed_response.embeddings).collect();

                    if let Err(e) =
                        vector_store.upsert_document_embeddings(&collection_id, entries).await
                    {
                        let err_msg = e.to_string();
                        tracing::error!(
                            "Failed to upsert embeddings for doc {}: {}",
                            doc.id,
                            err_msg
                        );
                        let _ = axagent_dao::repo::knowledge::update_document_status_with_error(
                            &db,
                            &doc.id,
                            "failed",
                            Some(&err_msg),
                        )
                        .await;
                        let _ = app.emit(
                            IpcEventName::KnowledgeDocumentIndexed.as_str(),
                            serde_json::json!({
                                "documentId": doc.id,
                                "success": false,
                                "error": err_msg,
                            }),
                        );
                    } else {
                        let _ = axagent_dao::repo::knowledge::update_document_status_with_error(
                            &db, &doc.id, "ready", None,
                        )
                        .await;
                        let _ = app.emit(
                            IpcEventName::KnowledgeDocumentIndexed.as_str(),
                            serde_json::json!({
                                "documentId": doc.id,
                                "success": true,
                            }),
                        );
                    }
                },
                Err(e) => {
                    let err_msg = e.to_string();
                    tracing::error!("Failed to embed doc {} during rebuild: {}", doc.id, err_msg);
                    let _ = axagent_dao::repo::knowledge::update_document_status_with_error(
                        &db,
                        &doc.id,
                        "failed",
                        Some(&err_msg),
                    )
                    .await;
                    let _ = app.emit(
                        IpcEventName::KnowledgeDocumentIndexed.as_str(),
                        serde_json::json!({
                            "documentId": doc.id,
                            "success": false,
                            "error": err_msg,
                        }),
                    );
                },
            }
        }

        // 兜底：把本 KB 下所有仍处于 "indexing" 状态的文档标记为 "failed"，
        // 防止中途 panic / 任务取消导致状态永久卡死。
        if let Ok(stuck_docs) = axagent_dao::repo::knowledge::list_documents(&db, &base_id).await {
            for doc in &stuck_docs {
                if doc.indexing_status == "indexing" {
                    let _ = axagent_dao::repo::knowledge::update_document_status_with_error(
                        &db,
                        &doc.id,
                        "failed",
                        Some("rebuild task terminated unexpectedly"),
                    )
                    .await;
                }
            }
        }

        let _ = app.emit(
            IpcEventName::KnowledgeRebuildComplete.as_str(),
            serde_json::json!({ "baseId": base_id }),
        );
    }));

    Ok(())
}

#[agent_command(domain = knowledge, safety = Safe, call_mode = StateOnly, description = "列出知识库容器")]
#[tauri::command]
pub async fn list_knowledge_containers(
    state: State<'_, AppState>,
) -> Result<Vec<KnowledgeContainer>, String> {
    let mut containers = Vec::new();

    let kbs = axagent_dao::repo::knowledge::list_knowledge_bases(state.harness.db())
        .await
        .map_err(|e| {
            String::from(crate::commands::error::ErrorResponse::from_error(
                e,
                crate::commands::error::ErrorCategory::Unrecoverable,
            ))
        })?;
    for kb in kbs {
        containers.push(KnowledgeContainer::from_knowledge_base(&kb));
    }

    let namespaces =
        axagent_dao::repo::memory::list_namespaces(state.harness.db()).await.map_err(|e| {
            String::from(crate::commands::error::ErrorResponse::from_error(
                e,
                crate::commands::error::ErrorCategory::Unrecoverable,
            ))
        })?;
    for ns in namespaces {
        containers.push(KnowledgeContainer::from_memory_ns(&ns));
    }

    let wikis = axagent_dao::repo::wiki::list_wikis(state.harness.db()).await.map_err(|e| {
        String::from(crate::commands::error::ErrorResponse::from_error(
            e,
            crate::commands::error::ErrorCategory::Unrecoverable,
        ))
    })?;
    for wiki in wikis {
        containers.push(KnowledgeContainer::from_wiki(&wiki));
    }

    containers.sort_by_key(|c| c.sort_order);

    Ok(containers)
}

#[agent_command(domain = knowledge, safety = Safe, call_mode = StateOnly, description = "列出知识图谱实体")]
#[tauri::command]
pub async fn list_knowledge_entities(
    state: State<'_, AppState>,
    base_id: String,
) -> Result<Vec<axagent_harness::types::KnowledgeEntity>, String> {
    axagent_dao::repo::knowledge_graph::list_knowledge_entities(state.harness.db(), &base_id)
        .await
        .map_err(|e| {
            String::from(crate::commands::error::ErrorResponse::from_error(
                e,
                crate::commands::error::ErrorCategory::Unrecoverable,
            ))
        })
}

#[agent_command(domain = knowledge, safety = Caution, call_mode = StateOnly, description = "创建知识图谱实体")]
#[tauri::command]
pub async fn create_knowledge_entity(
    state: State<'_, AppState>,
    input: axagent_harness::types::CreateKnowledgeEntityInput,
) -> Result<axagent_harness::types::KnowledgeEntity, String> {
    axagent_dao::repo::knowledge_graph::create_knowledge_entity(state.harness.db(), input)
        .await
        .map_err(|e| {
            String::from(crate::commands::error::ErrorResponse::from_error(
                e,
                crate::commands::error::ErrorCategory::Unrecoverable,
            ))
        })
}

#[agent_command(domain = knowledge, safety = Safe, call_mode = StateOnly, description = "列出知识图谱属性")]
#[tauri::command]
pub async fn list_knowledge_attributes(
    state: State<'_, AppState>,
    entity_id: String,
) -> Result<Vec<axagent_harness::types::KnowledgeAttribute>, String> {
    axagent_dao::repo::knowledge_graph::list_knowledge_attributes(state.harness.db(), &entity_id)
        .await
        .map_err(|e| {
            String::from(crate::commands::error::ErrorResponse::from_error(
                e,
                crate::commands::error::ErrorCategory::Unrecoverable,
            ))
        })
}

#[agent_command(domain = knowledge, safety = Caution, call_mode = StateOnly, description = "创建知识图谱属性")]
#[tauri::command]
pub async fn create_knowledge_attribute(
    state: State<'_, AppState>,
    input: axagent_harness::types::CreateKnowledgeAttributeInput,
) -> Result<axagent_harness::types::KnowledgeAttribute, String> {
    axagent_dao::repo::knowledge_graph::create_knowledge_attribute(state.harness.db(), input)
        .await
        .map_err(|e| {
            String::from(crate::commands::error::ErrorResponse::from_error(
                e,
                crate::commands::error::ErrorCategory::Unrecoverable,
            ))
        })
}

#[agent_command(domain = knowledge, safety = Safe, call_mode = StateOnly, description = "列出知识图谱关系")]
#[tauri::command]
pub async fn list_knowledge_relations(
    state: State<'_, AppState>,
    base_id: String,
) -> Result<Vec<axagent_harness::types::KnowledgeRelation>, String> {
    axagent_dao::repo::knowledge_graph::list_knowledge_relations(state.harness.db(), &base_id)
        .await
        .map_err(|e| {
            String::from(crate::commands::error::ErrorResponse::from_error(
                e,
                crate::commands::error::ErrorCategory::Unrecoverable,
            ))
        })
}

#[agent_command(domain = knowledge, safety = Caution, call_mode = StateOnly, description = "创建知识图谱关系")]
#[tauri::command]
pub async fn create_knowledge_relation(
    state: State<'_, AppState>,
    input: axagent_harness::types::CreateKnowledgeRelationInput,
) -> Result<axagent_harness::types::KnowledgeRelation, String> {
    axagent_dao::repo::knowledge_graph::create_knowledge_relation(state.harness.db(), input)
        .await
        .map_err(|e| {
            String::from(crate::commands::error::ErrorResponse::from_error(
                e,
                crate::commands::error::ErrorCategory::Unrecoverable,
            ))
        })
}

#[agent_command(domain = knowledge, safety = Safe, call_mode = StateOnly, description = "列出知识图谱流程")]
#[tauri::command]
pub async fn list_knowledge_flows(
    state: State<'_, AppState>,
    base_id: String,
) -> Result<Vec<axagent_harness::types::KnowledgeFlow>, String> {
    axagent_dao::repo::knowledge_graph::list_knowledge_flows(state.harness.db(), &base_id)
        .await
        .map_err(|e| {
            String::from(crate::commands::error::ErrorResponse::from_error(
                e,
                crate::commands::error::ErrorCategory::Unrecoverable,
            ))
        })
}

#[agent_command(domain = knowledge, safety = Caution, call_mode = StateOnly, description = "创建知识图谱流程")]
#[tauri::command]
pub async fn create_knowledge_flow(
    state: State<'_, AppState>,
    input: axagent_harness::types::CreateKnowledgeFlowInput,
) -> Result<axagent_harness::types::KnowledgeFlow, String> {
    axagent_dao::repo::knowledge_graph::create_knowledge_flow(state.harness.db(), input)
        .await
        .map_err(|e| {
            String::from(crate::commands::error::ErrorResponse::from_error(
                e,
                crate::commands::error::ErrorCategory::Unrecoverable,
            ))
        })
}

#[agent_command(domain = knowledge, safety = Safe, call_mode = StateOnly, description = "列出知识图谱接口")]
#[tauri::command]
pub async fn list_knowledge_interfaces(
    state: State<'_, AppState>,
    base_id: String,
) -> Result<Vec<axagent_harness::types::KnowledgeInterface>, String> {
    axagent_dao::repo::knowledge_graph::list_knowledge_interfaces(state.harness.db(), &base_id)
        .await
        .map_err(|e| {
            String::from(crate::commands::error::ErrorResponse::from_error(
                e,
                crate::commands::error::ErrorCategory::Unrecoverable,
            ))
        })
}

#[agent_command(domain = knowledge, safety = Caution, call_mode = StateOnly, description = "创建知识图谱接口")]
#[tauri::command]
pub async fn create_knowledge_interface(
    state: State<'_, AppState>,
    input: axagent_harness::types::CreateKnowledgeInterfaceInput,
) -> Result<axagent_harness::types::KnowledgeInterface, String> {
    axagent_dao::repo::knowledge_graph::create_knowledge_interface(state.harness.db(), input)
        .await
        .map_err(|e| {
            String::from(crate::commands::error::ErrorResponse::from_error(
                e,
                crate::commands::error::ErrorCategory::Unrecoverable,
            ))
        })
}

#[agent_command(domain = knowledge, safety = Dangerous, call_mode = StateOnly, description = "清空知识库索引")]
#[tauri::command]
pub async fn clear_knowledge_index(
    state: State<'_, AppState>,
    base_id: String,
) -> Result<(), String> {
    let collection_id = format!("kb_{}", base_id);
    // Only clear embeddings (vec0), keep chunk metadata (_meta) intact
    state.vector_store.clear_embeddings(&collection_id).await.map_err(|e| {
        String::from(crate::commands::error::ErrorResponse::from_error(
            e,
            crate::commands::error::ErrorCategory::Unrecoverable,
        ))
    })?;

    // 清空索引后把文档状态重置为 "skipped"（而非 "pending"），
    // 避免文档永久卡在 pending 但无索引任务可执行。
    // 用户如需重新索引，可调用 rebuild_knowledge_index。
    let docs = axagent_dao::repo::knowledge::list_documents(state.harness.db(), &base_id)
        .await
        .map_err(|e| {
            String::from(crate::commands::error::ErrorResponse::from_error(
                e,
                crate::commands::error::ErrorCategory::Unrecoverable,
            ))
        })?;

    for doc in docs {
        let _ = axagent_dao::repo::knowledge::update_document_status_with_error(
            state.harness.db(),
            &doc.id,
            "skipped",
            Some("index cleared by user"),
        )
        .await;
    }

    Ok(())
}

#[agent_command(domain = knowledge, safety = Safe, call_mode = StateOnly, description = "列出知识文档分块")]
#[tauri::command]
pub async fn list_knowledge_document_chunks(
    state: State<'_, AppState>,
    base_id: String,
    document_id: String,
) -> Result<Vec<axagent_search::vector_store::VectorSearchResult>, String> {
    let collection_id = format!("kb_{}", base_id);
    state.vector_store.list_document_chunks(&collection_id, &document_id).await.map_err(|e| {
        String::from(crate::commands::error::ErrorResponse::from_error(
            e,
            crate::commands::error::ErrorCategory::Unrecoverable,
        ))
    })
}

#[agent_command(domain = knowledge, safety = Dangerous, call_mode = StateOnly, description = "删除知识分块")]
#[tauri::command]
pub async fn delete_knowledge_chunk(
    state: State<'_, AppState>,
    base_id: String,
    chunk_id: String,
) -> Result<(), String> {
    let collection_id = format!("kb_{}", base_id);
    state.vector_store.delete_chunk(&collection_id, &chunk_id).await.map_err(|e| {
        String::from(crate::commands::error::ErrorResponse::from_error(
            e,
            crate::commands::error::ErrorCategory::Unrecoverable,
        ))
    })
}

#[agent_command(domain = knowledge, safety = Caution, call_mode = StateOnly, description = "更新知识分块")]
#[tauri::command]
pub async fn update_knowledge_chunk(
    app: AppHandle,
    state: State<'_, AppState>,
    base_id: String,
    chunk_id: String,
    content: String,
) -> Result<(), String> {
    let collection_id = format!("kb_{}", base_id);
    state.vector_store.update_chunk_content(&collection_id, &chunk_id, &content).await.map_err(
        |e| {
            String::from(crate::commands::error::ErrorResponse::from_error(
                e,
                crate::commands::error::ErrorCategory::Unrecoverable,
            ))
        },
    )?;

    // Auto-reindex: re-embed the chunk with the updated content
    let kb = axagent_dao::repo::knowledge::get_knowledge_base(state.harness.db(), &base_id)
        .await
        .map_err(|e| {
        String::from(crate::commands::error::ErrorResponse::from_error(
            e,
            crate::commands::error::ErrorCategory::Unrecoverable,
        ))
    })?;

    if let Some(embedding_provider) = kb.embedding_provider {
        let db = state.harness.db().clone();
        let master_key = state.harness.master_key_owned();
        let provider_registry = state.harness.provider_registry().clone();
        let vector_store = state.vector_store.clone();
        let cid = chunk_id.clone();
        let chunk_content = content.clone();

        tokio::spawn(catch_unwind_logged("knowledge.auto_reindex_chunk", async move {
            let result = async {
                let embed_response = crate::indexing::generate_embeddings(
                    &db,
                    &master_key,
                    &provider_registry,
                    &embedding_provider,
                    vec![chunk_content],
                    None,
                )
                .await?;

                if let Some(embedding) = embed_response.embeddings.into_iter().next() {
                    vector_store.update_chunk_embedding(&collection_id, &cid, &embedding).await?;
                }
                Ok::<_, axagent_harness::core_error::AxAgentError>(())
            }
            .await;

            if let Err(e) = &result {
                tracing::warn!("Auto-reindex failed for chunk {}: {}", cid, e);
            }

            let _ = app.emit(
                IpcEventName::KnowledgeChunkReindexed.as_str(),
                serde_json::json!({
                    "chunkId": cid,
                    "success": result.is_ok(),
                    "error": result.err().map(|e| e.to_string()),
                }),
            );
        }));
    }

    Ok(())
}

#[agent_command(domain = knowledge, safety = Caution, call_mode = StateOnly, description = "添加知识分块")]
#[tauri::command]
pub async fn add_knowledge_chunk(
    app: AppHandle,
    state: State<'_, AppState>,
    base_id: String,
    document_id: String,
    content: String,
) -> Result<String, String> {
    let kb = axagent_dao::repo::knowledge::get_knowledge_base(state.harness.db(), &base_id)
        .await
        .map_err(|e| {
        String::from(crate::commands::error::ErrorResponse::from_error(
            e,
            crate::commands::error::ErrorCategory::Unrecoverable,
        ))
    })?;

    let embedding_provider = kb.embedding_provider.ok_or_else(|| {
        crate::commands::error::ErrorResponse::err(
            crate::commands::error_code::knowledge::NO_EMBEDDING_PROVIDER,
        )
    })?;

    let collection_id = format!("kb_{}", base_id);
    let db = state.harness.db().clone();
    let master_key = state.harness.master_key_owned();
    let vector_store = state.vector_store.clone();
    let doc_id = document_id.clone();
    let chunk_content = content.clone();
    let provider_registry = state.harness.provider_registry().clone();

    let chunk_id_result = tokio::spawn(async move {
        let embed_response = crate::indexing::generate_embeddings(
            &db,
            &master_key,
            &provider_registry,
            &embedding_provider,
            vec![chunk_content.clone()],
            None,
        )
        .await?;

        let embedding = embed_response.embeddings.into_iter().next().ok_or_else(|| {
            axagent_harness::core_error::AxAgentError::Provider("No embedding returned".to_string())
        })?;

        let chunk_id = vector_store
            .add_single_chunk(&collection_id, &doc_id, &chunk_content, &embedding)
            .await?;

        let _ = app.emit(
            "knowledge-chunk-added",
            serde_json::json!({
                "baseId": base_id,
                "documentId": doc_id,
                "chunkId": chunk_id,
            }),
        );

        Ok::<String, axagent_harness::core_error::AxAgentError>(chunk_id)
    })
    .await
    .map_err(|e| {
        String::from(crate::commands::error::ErrorResponse::from_error(
            e,
            crate::commands::error::ErrorCategory::Unrecoverable,
        ))
    })?
    .map_err(|e| {
        String::from(crate::commands::error::ErrorResponse::from_error(
            e,
            crate::commands::error::ErrorCategory::Unrecoverable,
        ))
    })?;

    Ok(chunk_id_result)
}

#[agent_command(domain = knowledge, safety = Caution, call_mode = StateOnly, description = "重索引知识分块")]
#[tauri::command]
pub async fn reindex_knowledge_chunk(
    app: AppHandle,
    state: State<'_, AppState>,
    base_id: String,
    chunk_id: String,
) -> Result<(), String> {
    let kb = axagent_dao::repo::knowledge::get_knowledge_base(state.harness.db(), &base_id)
        .await
        .map_err(|e| {
        String::from(crate::commands::error::ErrorResponse::from_error(
            e,
            crate::commands::error::ErrorCategory::Unrecoverable,
        ))
    })?;

    let embedding_provider = kb.embedding_provider.ok_or_else(|| {
        crate::commands::error::ErrorResponse::err(
            crate::commands::error_code::knowledge::NO_EMBEDDING_PROVIDER,
        )
    })?;

    // Whitelist check: base_id must only contain alphanumeric chars and hyphens (for safe table name usage)
    if !base_id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
        return Err(crate::commands::error::ErrorResponse::err_with_detail(
            crate::commands::error_code::common::INVALID_INPUT,
            format!("Invalid base_id: '{base_id}' — only ASCII alphanumeric and hyphens allowed"),
        ));
    }

    let collection_id = format!("kb_{}", base_id);

    let chunk_content = {
        use sea_orm::{ConnectionTrait, DbBackend, Statement};
        // 2026-07-31 修复：原 SQL 用 $1（PG 风格）却标 DbBackend::Sqlite（反向标记）。
        // SQLite 模式 `$1` 占位符不合法（需 `?`）→ 该查询在 SQLite 下必炸，PG 恰好能跑。
        // 统一按 backend 分支。
        let db = state.harness.db();
        let is_pg = db.get_database_backend() == DbBackend::Postgres;
        let backend = if is_pg {
            DbBackend::Postgres
        } else {
            DbBackend::Sqlite
        };
        let name = format!("vec_kb_{}", base_id.replace('-', "_"));
        let sql = if is_pg {
            format!("SELECT content FROM {name}_meta WHERE id = $1")
        } else {
            format!("SELECT content FROM {name}_meta WHERE id = ?")
        };
        let row = db
            .query_one_raw(Statement::from_sql_and_values(
                backend,
                sql,
                vec![chunk_id.clone().into()],
            ))
            .await
            .map_err(|e| {
                String::from(crate::commands::error::ErrorResponse::from_error(
                    e,
                    crate::commands::error::ErrorCategory::Unrecoverable,
                ))
            })?
            .ok_or_else(|| {
                crate::commands::error::ErrorResponse::err_with_detail(
                    crate::commands::error_code::knowledge::DOCUMENT_NOT_FOUND,
                    format!("Chunk {chunk_id} not found"),
                )
            })?;
        row.try_get::<String>("", "content").map_err(|e| {
            String::from(crate::commands::error::ErrorResponse::from_error(
                e,
                crate::commands::error::ErrorCategory::Unrecoverable,
            ))
        })?
    };

    // Embed the single chunk
    let db = state.harness.db().clone();
    let master_key = state.harness.master_key_owned();
    let provider_registry = state.harness.provider_registry().clone();
    let vector_store = state.vector_store.clone();
    let cid = chunk_id.clone();

    tokio::spawn(catch_unwind_logged("knowledge.reindex_chunk", async move {
        let result = async {
            let embed_response = crate::indexing::generate_embeddings(
                &db,
                &master_key,
                &provider_registry,
                &embedding_provider,
                vec![chunk_content],
                None,
            )
            .await?;

            if let Some(embedding) = embed_response.embeddings.into_iter().next() {
                vector_store.update_chunk_embedding(&collection_id, &cid, &embedding).await?;
            }
            Ok::<_, axagent_harness::core_error::AxAgentError>(())
        }
        .await;

        if let Err(ref e) = result {
            tracing::warn!("[knowledge] 重索引单块失败 (chunk={}): {}", cid, e);
        }

        let _ = app.emit(
            IpcEventName::KnowledgeChunkReindexed.as_str(),
            serde_json::json!({
                "chunkId": cid,
                "success": result.is_ok(),
                "error": result.err().map(|e| e.to_string()),
            }),
        );
    }));

    Ok(())
}

#[agent_command(domain = knowledge, safety = Caution, call_mode = StateOnly, description = "重建知识文档索引")]
/// Rebuild the index for a single document (re-embed its chunks only).
#[tauri::command]
pub async fn rebuild_knowledge_document(
    app: AppHandle,
    state: State<'_, AppState>,
    base_id: String,
    document_id: String,
) -> Result<(), String> {
    let kb = axagent_dao::repo::knowledge::get_knowledge_base(state.harness.db(), &base_id)
        .await
        .map_err(|e| {
        String::from(crate::commands::error::ErrorResponse::from_error(
            e,
            crate::commands::error::ErrorCategory::Unrecoverable,
        ))
    })?;

    let embedding_provider = kb.embedding_provider.ok_or_else(|| {
        crate::commands::error::ErrorResponse::err(
            crate::commands::error_code::knowledge::NO_EMBEDDING_PROVIDER,
        )
    })?;

    let collection_id = format!("kb_{}", base_id);

    let chunks =
        state.vector_store.list_document_chunks_raw(&collection_id, &document_id).await.map_err(
            |e| {
                String::from(crate::commands::error::ErrorResponse::from_error(
                    e,
                    crate::commands::error::ErrorCategory::Unrecoverable,
                ))
            },
        )?;

    if chunks.is_empty() {
        let _ = app.emit(
            IpcEventName::KnowledgeDocumentIndexed.as_str(),
            serde_json::json!({ "documentId": document_id, "success": true }),
        );
        return Ok(());
    }

    // Set document status to "indexing"
    let _ = axagent_dao::repo::knowledge::update_document_status(
        state.harness.db(),
        &document_id,
        "indexing",
    )
    .await;

    let db = state.harness.db().clone();
    let master_key = state.harness.master_key_owned();
    let vector_store = state.vector_store.clone();
    let ep = embedding_provider.clone();
    let doc_id = document_id.clone();
    let provider_registry = state.harness.provider_registry().clone();

    tokio::spawn(catch_unwind_logged("knowledge.rebuild_doc", async move {
        let texts: Vec<String> = chunks.iter().map(|(_, _, content)| content.clone()).collect();
        let rowids: Vec<i64> = chunks.iter().map(|(rid, _, _)| *rid).collect();

        let result = crate::indexing::generate_embeddings(
            &db,
            &master_key,
            &provider_registry,
            &ep,
            texts,
            None,
        )
        .await;

        match result {
            Ok(embed_response) => {
                let entries: Vec<(i64, Vec<f32>)> =
                    rowids.into_iter().zip(embed_response.embeddings).collect();

                if let Err(e) =
                    vector_store.upsert_document_embeddings(&collection_id, entries).await
                {
                    let err_msg = e.to_string();
                    tracing::error!("Failed to upsert embeddings for doc {}: {}", doc_id, err_msg);
                    let _ = axagent_dao::repo::knowledge::update_document_status_with_error(
                        &db,
                        &doc_id,
                        "failed",
                        Some(&err_msg),
                    )
                    .await;
                    let _ = app.emit(
                        IpcEventName::KnowledgeDocumentIndexed.as_str(),
                        serde_json::json!({
                            "documentId": doc_id,
                            "success": false,
                            "error": err_msg,
                        }),
                    );
                } else {
                    let _ = axagent_dao::repo::knowledge::update_document_status_with_error(
                        &db, &doc_id, "ready", None,
                    )
                    .await;
                    let _ = app.emit(
                        IpcEventName::KnowledgeDocumentIndexed.as_str(),
                        serde_json::json!({
                            "documentId": doc_id,
                            "success": true,
                        }),
                    );
                }
            },
            Err(e) => {
                let err_msg = e.to_string();
                tracing::error!("Failed to embed doc {}: {}", doc_id, err_msg);
                let _ = axagent_dao::repo::knowledge::update_document_status_with_error(
                    &db,
                    &doc_id,
                    "failed",
                    Some(&err_msg),
                )
                .await;
                let _ = app.emit(
                    IpcEventName::KnowledgeDocumentIndexed.as_str(),
                    serde_json::json!({
                        "documentId": doc_id,
                        "success": false,
                        "error": err_msg,
                    }),
                );
            },
        }
    }));

    Ok(())
}

// ── lemonhu 开源股票知识库导入 ─────────────────────────────

/// 从 knowledge-sources/lemonhu/ 导入全部知识图谱数据
///
/// 导入 CSV（stock/concept/industry/executive + 关系）和 wiki_pages 到 DB。
/// 幂等：已存在的记录会被跳过。
#[agent_command(domain = knowledge, safety = Caution, call_mode = StateOnly, description = "导入lemonhu开源股票知识库")]
#[tauri::command]
pub async fn import_lemonhu_knowledge(
    state: State<'_, AppState>,
    knowledge_dir: Option<String>,
) -> Result<serde_json::Value, String> {
    let db = state.harness.db();
    let dir = knowledge_dir.map(std::path::PathBuf::from);
    ensure_lemonhu_knowledge_imported(db, dir).await
}

/// 确保 lemonhu 开源股票知识库已导入（幂等：KB 已存在且数据已导入则跳过重导）。
///
/// 供 `import_lemonhu_knowledge` 命令与启动初始化（`seed_concept_index::ensure_concept_index`）
/// 共用，避免启动时重复实现目录解析/KB 创建逻辑。
pub(crate) async fn ensure_lemonhu_knowledge_imported(
    db: &sea_orm::DatabaseConnection,
    knowledge_dir: Option<std::path::PathBuf>,
) -> Result<serde_json::Value, String> {
    let kb_id = "lemonhu_knowledge_graph";

    // 确定知识库目录
    let knowledge_dir = match knowledge_dir {
        Some(d) => d,
        None => {
            let cwd = std::env::current_dir().map_err(|e| format!("获取 cwd 失败: {e}"))?;
            let candidate = cwd.parent().unwrap_or(&cwd).join("knowledge-sources").join("lemonhu");
            if candidate.exists() {
                candidate
            } else {
                cwd.join("knowledge-sources").join("lemonhu")
            }
        },
    };
    if !knowledge_dir.exists() {
        return Err(format!("知识库目录不存在: {}", knowledge_dir.display()));
    }

    // 确保 knowledge_bases 存在
    let kb_exists = knowledge_bases::Entity::find_by_id(kb_id)
        .one(db)
        .await
        .map_err(|e| format!("查 knowledge_bases 失败: {e}"))?
        .is_some();
    if !kb_exists {
        knowledge_bases::ActiveModel {
            id: Set(kb_id.to_string()),
            name: Set("开源股票知识库(lemonhu)".into()),
            description: Set(Some(
                "由开源项目 lemonhu 构建的 A 股知识图谱，含概念/行业/公司/高管关系及百科文档"
                    .into(),
            )),
            embedding_provider: Set(None),
            enabled: Set(1),
            icon_type: Set(Some("book".into())),
            icon_value: Set(None),
            sort_order: Set(0),
            embedding_dimensions: Set(None),
            retrieval_threshold: Set(None),
            retrieval_top_k: Set(None),
            chunk_size: Set(None),
            chunk_overlap: Set(None),
            separator: Set(None),
            kind: Set("indexed".into()),
            vault_path: Set(None),
        }
        .insert(db)
        .await
        .map_err(|e| format!("创建 knowledge_bases 失败: {e}"))?;
    }

    let (entity_count, rel_count, doc_count) =
        import_lemonhu_graph(db, kb_id, &knowledge_dir, false).await;

    tracing::info!(
        "[lemonhu] 导入完成: {entity_count} 节点 + {rel_count} 关系 + {doc_count} 文档 (kb={kb_id})"
    );

    Ok(serde_json::json!({
        "knowledgeBaseId": kb_id,
        "entityCount": entity_count,
        "relationCount": rel_count,
        "documentCount": doc_count,
    }))
}

/// 导入 lemonhu 开源知识图谱的实体、关系及文档到指定知识库。
///
/// 由 [`import_lemonhu_knowledge`] 和 [`import_project_knowledge_sources`] 共用。
/// 读取 `{lemonhu_dir}/raw/*.csv` 解析 entity/relation，读取 `{lemonhu_dir}/wiki_pages/*.md` 导入文档。
///
/// - `force_reimport_wiki_pages`：true 时即使 KB 已有文档也重新导入 wiki_pages（用于 update 模式）。
///   实体/关系始终按 id 幂等（已存在则跳过）。
async fn import_lemonhu_graph(
    db: &sea_orm::DatabaseConnection,
    kb_id: &str,
    lemonhu_dir: &std::path::Path,
    force_reimport_wiki_pages: bool,
) -> (usize, usize, usize) {
    let now_ms = chrono::Utc::now().timestamp_millis();
    let mut entity_count = 0usize;
    let mut rel_count = 0usize;
    let mut doc_count = 0usize;
    let mut skipped_entities = 0usize;
    let mut skipped_relations = 0usize;

    // 实体/关系 id 加 KB 前缀，保证全局唯一，支持跨 KB 复用
    // （旧实现用全局 id，导致同名 KB 重导时实体被错误跳过且 knowledge_base_id 仍指向旧 KB）
    let prefix = format!("{}_", kb_id);

    // ── 收集 entities：优先标准格式 nodes.csv，回退到历史 raw/*.csv ──
    let raw_dir = lemonhu_dir.join("raw");
    let nodes_path = lemonhu_dir.join("nodes.csv");
    let mut entity_data: Vec<(String, String, String)> = Vec::new();

    if nodes_path.exists() {
        // 标准格式：id,title,type,tags
        if let Ok(csv) = std::fs::read_to_string(&nodes_path) {
            let mut lines = 0usize;
            for line in csv.lines().skip(1) {
                let line = line.trim();
                if line.is_empty() {
                    continue;
                }
                lines += 1;
                let fields: Vec<&str> = line.splitn(4, ',').collect();
                if fields.len() < 3 {
                    continue;
                }
                let id = fields[0].trim_matches('"').to_string();
                let title = fields[1].trim_matches('"').to_string();
                let etype = fields[2].trim_matches('"').to_string();
                if id.is_empty() || title.is_empty() {
                    continue;
                }
                entity_data.push((id, title, etype));
            }
            tracing::info!(
                "[graph_import] nodes.csv 读取 {lines} 行 → {} 条实体 (path={})",
                entity_data.len(),
                nodes_path.display()
            );
        } else {
            tracing::warn!("[graph_import] nodes.csv 读取失败: {}", nodes_path.display());
        }
    }
    if raw_dir.exists() {
        // 补充加载 raw/*.csv 中的行业/概念/高管实体（即使 nodes.csv 存在）
        // edges.csv 中引用的行业 hash ID 来自这些文件，不导入则关系指向"空气"
        tracing::info!("[graph_import] 从 raw/*.csv 补充实体");
        if let Ok(csv) = std::fs::read_to_string(raw_dir.join("stock.csv")) {
            for line in csv.lines().skip(1) {
                let line = line.trim();
                if line.is_empty() {
                    continue;
                }
                let fields: Vec<&str> = line.splitn(4, ',').collect();
                if fields.len() < 3 {
                    continue;
                }
                entity_data.push((fields[0].to_string(), fields[1].to_string(), "company".into()));
            }
        }
        if let Ok(csv) = std::fs::read_to_string(raw_dir.join("concept.csv")) {
            for line in csv.lines().skip(1) {
                let line = line.trim();
                if line.is_empty() {
                    continue;
                }
                let fields: Vec<&str> = line.splitn(3, ',').collect();
                if fields.len() < 2 {
                    continue;
                }
                entity_data.push((fields[0].to_string(), fields[1].to_string(), "concept".into()));
            }
        }
        if let Ok(csv) = std::fs::read_to_string(raw_dir.join("industry.csv")) {
            for line in csv.lines().skip(1) {
                let line = line.trim();
                if line.is_empty() {
                    continue;
                }
                let fields: Vec<&str> = line.splitn(3, ',').collect();
                if fields.len() < 2 {
                    continue;
                }
                entity_data.push((fields[0].to_string(), fields[1].to_string(), "industry".into()));
            }
        }
        if let Ok(csv) = std::fs::read_to_string(raw_dir.join("executive.csv")) {
            for line in csv.lines().skip(1) {
                let line = line.trim();
                if line.is_empty() {
                    continue;
                }
                let fields: Vec<&str> = line.splitn(5, ',').collect();
                if fields.len() < 2 {
                    continue;
                }
                entity_data.push((fields[0].to_string(), fields[1].to_string(), "person".into()));
            }
        }
        tracing::info!("[graph_import] raw/*.csv 读取完成 → {} 条实体", entity_data.len());
    } else {
        tracing::warn!(
            "[graph_import] 未找到 nodes.csv 或 raw/*.csv，实体跳过 (lemonhu_dir={})",
            lemonhu_dir.display()
        );
    }

    // P1-write（2026-09-14）：实体类型观测。
    //
    // 本路径**绕过 DAO 写函数**（循环里直接 `ActiveModel::insert`），所以 DAO 侧的
    // 三处观测都覆盖不到它 —— 这里显式调用**同一个** helper（`observe_entity_types`），
    // 不另写一份校验/去重。
    //
    // 必须在**循环之前**做：本路径实测写入 6 万+ 行，逐条校验不仅慢，同一类型的
    // 警告还会重复 6 万次 —— 那不是可观测，是刷屏。本路径的 `etype` 来自固定映射
    // （`nodes.csv` 的 type 列 + `raw/*.csv` 的硬编码字面量），
    // 所以这里真正要抓的是「数据文件换了取值」。
    axagent_dao::repo::knowledge_graph::observe_entity_types(
        "commands::knowledge::graph_import",
        entity_data.iter().map(|(_, _, etype)| etype.as_str()),
    );

    for (id, name, etype) in entity_data {
        let prefixed_id = format!("{}{}", prefix, id);
        let exists = knowledge_entities::Entity::find_by_id(&prefixed_id)
            .one(db)
            .await
            .map(|o| o.is_some())
            .unwrap_or(false);
        if exists {
            skipped_entities += 1;
            continue;
        }
        let active = knowledge_entities::ActiveModel {
            id: Set(prefixed_id),
            knowledge_base_id: Set(kb_id.to_string()),
            name: Set(name),
            entity_type: Set(etype),
            description: Set(None),
            source_path: Set("nodes.csv".into()),
            source_language: Set(None),
            properties: Set(serde_json::json!({})),
            lifecycle: Set(None),
            behaviors: Set(None),
            metadata: Set(None),
            aliases: Set(String::new()),
            mention_count: Set(0),
            confidence: Set(0.0),
            first_seen_at: Set(None),
            last_seen_at: Set(None),
            source_type: Set(String::from("knowledge_base")),
            source_id: Set(String::new()),
            node_type: Set(String::from(
                axagent_harness::knowledge_graph::GraphNodeType::Entity.as_str(),
            )),
            external_id: Set(None),
            created_at: Set(now_ms),
            updated_at: Set(now_ms),
        };
        if active.insert(db).await.is_ok() {
            entity_count += 1;
        }
    }

    // ── 收集 relations：优先标准 edges.csv，回退到历史 raw/*.csv ──
    let edges_path = lemonhu_dir.join("edges.csv");
    let mut rel_data: Vec<(String, String, String, String)> = Vec::new();

    if edges_path.exists() {
        // 标准格式：source,target,type
        if let Ok(csv) = std::fs::read_to_string(&edges_path) {
            let mut lines = 0usize;
            for line in csv.lines().skip(1) {
                let line = line.trim();
                if line.is_empty() {
                    continue;
                }
                lines += 1;
                let fields: Vec<&str> = line.splitn(3, ',').collect();
                if fields.len() < 3 {
                    continue;
                }
                let src = fields[0].trim_matches('"').to_string();
                let tgt = fields[1].trim_matches('"').to_string();
                let rtype = fields[2].trim_matches('"').to_string();
                if src.is_empty() || tgt.is_empty() || rtype.is_empty() {
                    continue;
                }
                rel_data.push((format!("{src}_{rtype}_{tgt}"), src, tgt, rtype));
            }
            tracing::info!(
                "[graph_import] edges.csv 读取 {lines} 行 → {} 条关系 (path={})",
                rel_data.len(),
                edges_path.display()
            );
        } else {
            tracing::warn!("[graph_import] edges.csv 读取失败: {}", edges_path.display());
        }
    } else if raw_dir.exists() {
        // 历史兼容：raw/stock_concept.csv 等
        tracing::info!("[graph_import] edges.csv 不存在，回退到 raw/*.csv 路径");
        if let Ok(csv) = std::fs::read_to_string(raw_dir.join("stock_concept.csv")) {
            for line in csv.lines().skip(1) {
                let line = line.trim();
                if line.is_empty() {
                    continue;
                }
                let fields: Vec<&str> = line.splitn(3, ',').collect();
                if fields.len() < 3 {
                    continue;
                }
                let src = fields[0].to_string();
                let tgt = fields[1].to_string();
                rel_data.push((format!("{src}_has_concept_{tgt}"), src, tgt, "has_concept".into()));
            }
        }
        if let Ok(csv) = std::fs::read_to_string(raw_dir.join("stock_industry.csv")) {
            for line in csv.lines().skip(1) {
                let line = line.trim();
                if line.is_empty() {
                    continue;
                }
                let fields: Vec<&str> = line.splitn(3, ',').collect();
                if fields.len() < 3 {
                    continue;
                }
                let src = fields[0].to_string();
                let tgt = fields[1].to_string();
                rel_data.push((format!("{src}_in_industry_{tgt}"), src, tgt, "in_industry".into()));
            }
        }
        if let Ok(csv) = std::fs::read_to_string(raw_dir.join("executive_stock.csv")) {
            for line in csv.lines().skip(1) {
                let line = line.trim();
                if line.is_empty() {
                    continue;
                }
                let fields: Vec<&str> = line.splitn(4, ',').collect();
                if fields.len() < 4 {
                    continue;
                }
                let src = fields[0].to_string();
                let position = fields[1].replace('/', "_");
                let tgt = fields[2].to_string();
                let rel_type = format!("employ_{position}");
                rel_data.push((format!("{src}_{rel_type}_{tgt}"), src, tgt, rel_type));
            }
        }
        tracing::info!("[graph_import] raw/*.csv 关系文件读取完成 → {} 条关系", rel_data.len());
    } else {
        tracing::warn!(
            "[graph_import] 未找到 edges.csv 或 raw/*.csv，关系跳过 (lemonhu_dir={})",
            lemonhu_dir.display()
        );
    }

    // B1（2026-09-14）：关系词表校验。**按「类型」去重上报**，避免逐行刷屏。
    // 只校验 id —— 导入路径在这里没有解析实体的节点类，域/值域无从校验。
    // 不阻断导入：`edges.csv` 的 rtype 直接来自数据文件（非代码常量），
    // 闭合词表必然误伤它；这里只让「未登记值」可见。
    {
        let mut seen: std::collections::HashSet<&str> = std::collections::HashSet::new();
        for (_, _, _, rtype) in &rel_data {
            if !seen.insert(rtype.as_str()) {
                continue;
            }
            if let Some(v) = axagent_harness::knowledge_graph::validate_relation_id(rtype) {
                axagent_harness::knowledge_graph::warn_on_violations(
                    "commands::knowledge::graph_import",
                    &[v],
                );
            }
        }
    }

    for (id, src, tgt, rtype) in rel_data {
        let prefixed_id = format!("{}{}", prefix, id);
        let prefixed_src = format!("{}{}", prefix, src);
        let prefixed_tgt = format!("{}{}", prefix, tgt);
        let exists = knowledge_relations::Entity::find_by_id(&prefixed_id)
            .one(db)
            .await
            .map(|o| o.is_some())
            .unwrap_or(false);
        if exists {
            skipped_relations += 1;
            continue;
        }
        let active = knowledge_relations::ActiveModel {
            id: Set(prefixed_id),
            knowledge_base_id: Set(kb_id.to_string()),
            source_entity_id: Set(prefixed_src),
            target_entity_id: Set(prefixed_tgt),
            relation_type: Set(rtype),
            description: Set(None),
            properties: Set(None),
            metadata: Set(None),
            weight: Set(0.0),
            source_type: Set(String::from("knowledge_base")),
            source_id: Set(String::new()),
            created_at: Set(now_ms),
            updated_at: Set(now_ms),
        };
        if active.insert(db).await.is_ok() {
            rel_count += 1;
        }
    }

    // ── 导入 wiki_pages ──
    let wiki_dir = lemonhu_dir.join("wiki_pages");
    if wiki_dir.exists() {
        let existing_docs = knowledge_documents::Entity::find()
            .filter(knowledge_documents::Column::KnowledgeBaseId.eq(kb_id))
            .count(db)
            .await
            .unwrap_or(0);
        // update 模式（force_reimport_wiki_pages=true）或 KB 为空时执行导入
        if existing_docs == 0 || force_reimport_wiki_pages {
            if force_reimport_wiki_pages && existing_docs > 0 {
                tracing::info!(
                    "[graph_import] update 模式：删除 KB={} 下 {} 条现有文档以重新导入 wiki_pages",
                    kb_id,
                    existing_docs
                );
                let _ = knowledge_documents::Entity::delete_many()
                    .filter(knowledge_documents::Column::KnowledgeBaseId.eq(kb_id))
                    .exec(db)
                    .await;
            }
            if let Ok(mut reader) = std::fs::read_dir(&wiki_dir) {
                while let Ok(Some(entry)) = reader.next().transpose() {
                    let path = entry.path();
                    if path.extension().and_then(|s| s.to_str()) != Some("md") {
                        continue;
                    }
                    let content = match std::fs::read_to_string(&path) {
                        Ok(c) => c,
                        Err(_) => continue,
                    };
                    let file_stem =
                        path.file_stem().and_then(|s| s.to_str()).unwrap_or("unknown").to_string();
                    // 标题优先取 frontmatter title 字段，回退到首行非空行（去掉 # 前缀）
                    let title = extract_frontmatter_title(&content)
                        .or_else(|| {
                            content.lines().find(|l| !l.trim().is_empty()).map(|l| {
                                l.trim()
                                    .trim_start_matches('#')
                                    .trim()
                                    .chars()
                                    .take(80)
                                    .collect::<String>()
                            })
                        })
                        .unwrap_or_else(|| file_stem.clone());
                    let active = knowledge_documents::ActiveModel {
                        id: Set(uuid::Uuid::new_v4().to_string()),
                        knowledge_base_id: Set(kb_id.to_string()),
                        title: Set(title),
                        source_path: Set(path.to_string_lossy().to_string()),
                        mime_type: Set("text/markdown".into()),
                        size_bytes: Set(content.len() as i64),
                        indexing_status: Set("pending".into()),
                        doc_type: Set("markdown".into()),
                        content_hash: Set(String::new()),
                        index_error: Set(None),
                        source_conversation_id: Set(None),
                        created_at: Set(now_ms),
                        updated_at: Set(now_ms),
                    };
                    if active.insert(db).await.is_ok() {
                        doc_count += 1;
                    }
                }
            }
        } else {
            tracing::info!("[graph_import] DB 已有 {existing_docs} 篇文档，跳过 wiki_pages 导入");
        }
    }

    tracing::info!(
        "[graph_import] 导入知识图谱: {entity_count} 节点 (+{skipped_entities} 跳过) + {rel_count} 关系 (+{skipped_relations} 跳过) + {doc_count} 文档 (kb={kb_id})"
    );

    (entity_count, rel_count, doc_count)
}

/// 从 markdown frontmatter 中提取 `title:` 字段值。
/// 仅处理 YAML frontmatter（首行 `---` 开头的块），简单按行扫描避免引入 yaml 解析依赖。
fn extract_frontmatter_title(content: &str) -> Option<String> {
    let lines: Vec<&str> = content.lines().collect();
    if lines.is_empty() || lines[0].trim() != "---" {
        return None;
    }
    for line in &lines[1..] {
        let trimmed = line.trim();
        if trimmed == "---" {
            break;
        }
        if let Some(rest) = trimmed.strip_prefix("title:") {
            let v = rest.trim().trim_matches('"').trim_matches('\'').trim();
            if !v.is_empty() {
                return Some(v.chars().take(80).collect());
            }
        }
    }
    None
}

/// 目录同步结果（与 [`ImportDirectoryResult`] 的区别：含 added/updated/deleted 三类计数）。
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncDirectoryResult {
    pub base_id: String,
    pub added_count: usize,
    pub updated_count: usize,
    pub deleted_count: usize,
    pub skipped_count: usize,
    pub error_count: usize,
    pub added: Vec<KnowledgeDocument>,
    pub updated: Vec<String>,
    pub deleted: Vec<String>,
    pub skipped: Vec<String>,
    pub errors: Vec<ImportDirectoryError>,
}

/// 一键同步更新：对比文件系统与知识库，自动新增/更新/删除文档。
///
/// 逻辑：
/// - 收集目录下所有可导入文件，记录 mtime（文件修改时间）
/// - 获取知识库现有文档列表，以 source_path 为 key 建立索引
/// - **新增**：文件在磁盘上但不在 KB 中 → `add_document` + 入队索引
/// - **更新**：文件在 KB 中且 mtime 晚于文档时间 → 删旧文档 + 加新文档 + 入队索引
/// - **删除**：文档在 KB 中但对应文件不存在磁盘 → `delete_knowledge_document`
/// - **跳过**：文件在 KB 中且 mtime 未变 → 跳过
#[agent_command(domain = knowledge, safety = Caution, call_mode = StateOnly, description = "同步项目知识源目录")]
#[tauri::command]
pub async fn sync_project_knowledge_sources(
    app: AppHandle,
    state: State<'_, AppState>,
    base_id: String,
    source_path: String,
    recursive: Option<bool>,
    ignore_patterns: Option<Vec<String>>,
) -> Result<SyncDirectoryResult, String> {
    let dir = PathBuf::from(&source_path);
    if !dir.exists() || !dir.is_dir() {
        return Err(format!("路径不存在或不是目录: {source_path}"));
    }

    let recursive = recursive.unwrap_or(true);
    let db = state.harness.db();

    // 1) 收集文件系统上的文件
    let mut disk_files: Vec<PathBuf> = Vec::new();
    let mut skipped = Vec::new();
    collect_importable_files(
        &dir,
        recursive,
        &None,
        &ignore_patterns,
        &mut disk_files,
        &mut skipped,
    )
    .map_err(|e| format!("读取目录失败 {source_path}: {e}"))?;

    // 2) 获取 KB 现有文档 → source_path → document 索引
    let existing_docs =
        axagent_dao::repo::knowledge::list_documents(db, &base_id).await.map_err(|e| {
            String::from(crate::commands::error::ErrorResponse::from_error(
                e,
                crate::commands::error::ErrorCategory::Unrecoverable,
            ))
        })?;

    // 用 source_path 做唯一 key 建立索引（标准化为小写）
    let mut doc_by_path: std::collections::HashMap<String, &KnowledgeDocument> =
        std::collections::HashMap::new();
    for doc in &existing_docs {
        doc_by_path.insert(doc.source_path.to_ascii_lowercase(), doc);
    }

    // 内容指纹索引（移动识别用）：磁盘上出现「新路径 + 与既有文档同内容」时，
    // 判定为文件移动而非新增。旧数据 content_hash 为空串不入索引（退化为纯新增）。
    let mut doc_by_hash: std::collections::HashMap<String, &KnowledgeDocument> =
        std::collections::HashMap::new();
    for doc in &existing_docs {
        if !doc.content_hash.is_empty() {
            doc_by_hash.entry(doc.content_hash.clone()).or_insert(doc);
        }
    }

    // 3) 处理 on-disk 文件：新增或更新
    let mut result = SyncDirectoryResult {
        base_id: base_id.clone(),
        added_count: 0,
        updated_count: 0,
        deleted_count: 0,
        skipped_count: 0,
        error_count: 0,
        added: Vec::new(),
        updated: Vec::new(),
        deleted: Vec::new(),
        skipped: Vec::new(),
        errors: Vec::new(),
    };

    // 记录所有被匹配到的路径（用于后续找已删除的文件）
    let mut matched_keys: std::collections::HashSet<String> = std::collections::HashSet::new();

    let kb = axagent_dao::repo::knowledge::get_knowledge_base(db, &base_id).await.map_err(|e| {
        String::from(crate::commands::error::ErrorResponse::from_error(
            e,
            crate::commands::error::ErrorCategory::Unrecoverable,
        ))
    })?;
    let has_embedding = kb.embedding_provider.is_some();

    // 文档 id → 源文件 mtime（add_document 写入；旧数据为 0，退化为仅 size 比对）
    let doc_mtimes =
        axagent_dao::repo::knowledge::get_document_mtime_map(db, &base_id).await.map_err(|e| {
            String::from(crate::commands::error::ErrorResponse::from_error(
                e,
                crate::commands::error::ErrorCategory::Unrecoverable,
            ))
        })?;

    for path in &disk_files {
        let abs = path.to_string_lossy().to_string();
        let key = abs.to_ascii_lowercase();
        let mime = axagent_document_parser::mime_from_extension(path).to_string();
        let title =
            path.strip_prefix(&dir).map(|p| p.to_string_lossy().replace('\\', "/")).unwrap_or_else(
                |_| path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default(),
            );

        matched_keys.insert(key.clone());

        if let Some(existing) = doc_by_path.get(&key) {
            // 增量比对：size + mtime 均未变化 → 跳过，避免每次同步对全部已存在
            // 文件删旧重加（大知识库会触发全量重索引）。
            // 旧数据 updated_at 为 0（未记录 mtime），此时退化为仅 size 比对。
            let fmeta = std::fs::metadata(path).ok();
            let fsize = fmeta.as_ref().map(|m| m.len() as i64).unwrap_or(-1);
            let fmtime = fmeta
                .as_ref()
                .and_then(|m| m.modified().ok())
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_secs() as i64)
                .unwrap_or(0);
            let size_unchanged = existing.size_bytes == fsize;
            let doc_mtime = doc_mtimes.get(&existing.id).copied().unwrap_or(0);
            let mtime_unchanged = doc_mtime > 0 && fmtime > 0 && fmtime <= doc_mtime;
            if size_unchanged && mtime_unchanged {
                result.skipped_count += 1;
                result.skipped.push(abs);
                continue;
            }

            // touch 识别：mtime 变了但 size 未变时，用内容指纹判定是否「真变化」。
            // 文件被 touch / 复制后 mtime 变新而内容未变 → 仅刷新记录中的 mtime 后跳过，
            // 避免误走删旧加新触发全量重索引；hash 不同才落到下方的删旧加新。
            if size_unchanged && !existing.content_hash.is_empty() {
                let fhash = axagent_dao::repo::knowledge::file_sha256(path);
                if fhash == existing.content_hash {
                    if fmtime > 0 {
                        let _ = axagent_dao::repo::knowledge::update_document_mtime(
                            db,
                            &existing.id,
                            fmtime,
                        )
                        .await;
                    }
                    result.skipped_count += 1;
                    result.skipped.push(format!("{abs}: 内容未变化（touch），跳过"));
                    continue;
                }
            }

            // 文件已存在且发生变化 → 删旧 + 加新（保证磁盘内容与 KB 一致）
            let doc_id = existing.id.clone();
            let collection_id = format!("kb_{}", base_id);
            // 向量删不掉就不删 DB 记录（2026-09-15 修）：否则记录消失而向量残留，
            // 旧内容继续被检索命中。计入 errors 后 continue，下次同步会重试。
            if let Err(e) =
                state.vector_store.delete_document_embeddings(&collection_id, &doc_id).await
            {
                result.error_count += 1;
                result.errors.push(ImportDirectoryError::with_code(
                    abs.clone(),
                    format!("清理旧文档向量失败: {e}"),
                    crate::commands::error_code::knowledge::VECTOR_STORE_FAILED,
                ));
                continue;
            }
            if let Err(e) = axagent_dao::repo::knowledge::delete_document(db, &doc_id).await {
                result.error_count += 1;
                result.errors.push(ImportDirectoryError::with_code(
                    abs,
                    format!("删除旧文档失败: {e}"),
                    crate::commands::error_code::knowledge::DELETE_DOCUMENT_FAILED,
                ));
                continue;
            }
            match axagent_dao::repo::knowledge::add_document(
                db, &base_id, &title, &abs, &mime, None,
            )
            .await
            {
                Ok(new_doc) => {
                    if has_embedding {
                        let _ = axagent_dao::repo::knowledge::update_document_status(
                            db,
                            &new_doc.id,
                            "pending",
                        )
                        .await;
                        if let Err(e) = crate::index_queue::enqueue_job_sync(
                            &state,
                            &app,
                            jobs::JOB_TYPE_INDEX_DOCUMENT,
                            "kb",
                            &base_id,
                            &new_doc.id,
                            None,
                            None,
                        ) {
                            tracing::warn!("[sync_project] 入队索引失败 {}: {}", new_doc.id, e);
                        }
                    }
                    result.updated_count += 1;
                    result.updated.push(abs);
                },
                Err(e) => {
                    result.error_count += 1;
                    result.errors.push(ImportDirectoryError::with_code(
                        abs,
                        format!("重加文档失败: {e}"),
                        crate::commands::error_code::knowledge::ADD_DOCUMENT_FAILED,
                    ));
                },
            }
        } else {
            // 文件不存在于 KB（source_path 维度）→ 先做移动识别：内容指纹与某已存在文档
            // 相同视为「文件挪了位置」，更新既有文档的 source_path / title / size / mtime
            // 而非新增（内容未变，向量保留，不触发重索引）。
            let fhash = axagent_dao::repo::knowledge::file_sha256(path);
            if !fhash.is_empty() {
                if let Some(moved) = doc_by_hash.get(&fhash) {
                    // 从 doc_by_path 移除旧 key：第 4 步按 doc_by_path 判定「磁盘已删除」，
                    // 不移除会把刚更新位置的文档当「已删除」再删掉（内容没变、向量残留）。
                    doc_by_path.remove(&moved.source_path.to_ascii_lowercase());
                    match axagent_dao::repo::knowledge::update_document_source_path(
                        db, &moved.id, &abs, &title,
                    )
                    .await
                    {
                        Ok(()) => {
                            result.updated_count += 1;
                            result.updated.push(abs);
                            continue;
                        },
                        Err(e) => {
                            // 更新失败：按旧行为退化为「新增 + 第 4 步删旧」，不丢数据
                            result.error_count += 1;
                            result.errors.push(ImportDirectoryError::with_code(
                                abs.clone(),
                                format!("移动识别更新失败: {e}"),
                                crate::commands::error_code::knowledge::ADD_DOCUMENT_FAILED,
                            ));
                        },
                    }
                }
            }

            // 文件不存在于 KB → 新增
            match axagent_dao::repo::knowledge::add_document(
                db, &base_id, &title, &abs, &mime, None,
            )
            .await
            {
                Ok(new_doc) => {
                    if has_embedding {
                        let _ = axagent_dao::repo::knowledge::update_document_status(
                            db,
                            &new_doc.id,
                            "pending",
                        )
                        .await;
                        if let Err(e) = crate::index_queue::enqueue_job_sync(
                            &state,
                            &app,
                            jobs::JOB_TYPE_INDEX_DOCUMENT,
                            "kb",
                            &base_id,
                            &new_doc.id,
                            None,
                            None,
                        ) {
                            tracing::warn!("[sync_project] 入队索引失败 {}: {}", new_doc.id, e);
                        }
                    }
                    result.added_count += 1;
                    result.added.push(new_doc);
                },
                Err(e) => {
                    result.error_count += 1;
                    result.errors.push(ImportDirectoryError::with_code(
                        abs,
                        format!("添加文档失败: {e}"),
                        crate::commands::error_code::knowledge::ADD_DOCUMENT_FAILED,
                    ));
                },
            }
        }
    }

    // 4) 处理 KB 中存在但磁盘上已删除的文档
    for (key, doc) in &doc_by_path {
        if !matched_keys.contains(key) {
            let collection_id = format!("kb_{}", base_id);
            // 向量删不掉就不删 DB 记录（2026-09-15 修）：否则文档记录消失而向量残留，
            // 已被移除的文件内容继续被检索命中。
            if let Err(e) =
                state.vector_store.delete_document_embeddings(&collection_id, &doc.id).await
            {
                result.error_count += 1;
                result.errors.push(ImportDirectoryError::with_code(
                    doc.source_path.clone(),
                    format!("清理已移除文档的向量失败: {e}"),
                    crate::commands::error_code::knowledge::VECTOR_STORE_FAILED,
                ));
            } else if let Err(e) = axagent_dao::repo::knowledge::delete_document(db, &doc.id).await
            {
                result.error_count += 1;
                result.errors.push(ImportDirectoryError::with_code(
                    doc.source_path.clone(),
                    format!("删除已移除文档失败: {e}"),
                    crate::commands::error_code::knowledge::DELETE_DOCUMENT_FAILED,
                ));
            } else {
                result.deleted_count += 1;
                result.deleted.push(doc.source_path.clone());
            }
        }
    }

    result.skipped_count += skipped.len();
    result.skipped.extend(skipped);

    tracing::info!(
        "[sync_project] 同步完成: +{} 新增, ~{} 更新, -{} 删除, ={} 跳过, !{} 错误 (kb={})",
        result.added_count,
        result.updated_count,
        result.deleted_count,
        result.skipped_count,
        result.error_count,
        base_id,
    );

    Ok(result)
}

/// 修复 Wiki 中所有笔记的 wikilink 关联。
///
/// 遍历 Wiki 下所有笔记，重新解析内容中的 `[[wikilink]]` 并同步到
/// `note_links` / `note_backlinks` 表。用于修复历史导入过程中可能
/// 遗漏的双向链接记录，确保图谱节点正确关联。
///
/// 实现委托 `resync_vault_note_links`（一次性构建全 vault 映射 + 批量写入）。
/// 旧实现逐篇调用 `sync_note_links_from_content`（每篇全量加载 vault，O(N²)），
/// 2 万篇笔记场景下不可用；且逐篇同步无法修复批量导入时被丢弃的前向引用。
///
/// 返回值：处理的笔记数量。
async fn repair_wiki_note_links(db: &sea_orm::DatabaseConnection, wiki_id: &str) -> usize {
    match axagent_dao::repo::note::resync_vault_note_links(db, wiki_id).await {
        Ok((notes, links)) => {
            tracing::info!(
                "[repair_links] Wiki {} 链接重建完成: {} 篇笔记, {} 条链接",
                wiki_id,
                notes,
                links
            );
            notes
        },
        Err(e) => {
            tracing::warn!("[repair_links] Wiki {} 链接重建失败: {e}", wiki_id);
            0
        },
    }
}

/// 将知识图谱的实体/关系桥接到 Wiki vault 中：为每个实体创建一篇笔记，
/// 在内容中嵌入 `[[关联实体]]` wikilinks，使 Wiki 图谱视图展示关联关系。
async fn bridge_graph_to_wiki(
    db: &sea_orm::DatabaseConnection,
    kb_id: &str,
    wiki_id: &str,
) -> (usize, usize) {
    // 1) 读取图谱侧全部实体 + 关系
    let entities = match knowledge_entities::Entity::find()
        .filter(knowledge_entities::Column::KnowledgeBaseId.eq(kb_id))
        .all(db)
        .await
    {
        Ok(e) => e,
        Err(e) => {
            tracing::warn!("[graph_to_wiki] 读取 entity 失败: {e}");
            return (0, 0);
        },
    };
    let relations = match knowledge_relations::Entity::find()
        .filter(knowledge_relations::Column::KnowledgeBaseId.eq(kb_id))
        .all(db)
        .await
    {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!("[graph_to_wiki] 读取 relation 失败: {e}");
            return (0, 0);
        },
    };

    // 2) 建立 entity_id → name 索引
    let name_by_id: HashMap<String, String> =
        entities.iter().map(|e| (e.id.clone(), e.name.clone())).collect();

    // 3) 建立 entity_id → [(target_id, relation_type)]
    let mut rel_map: HashMap<String, Vec<(String, String)>> = HashMap::new();
    for r in &relations {
        rel_map
            .entry(r.source_entity_id.clone())
            .or_default()
            .push((r.target_entity_id.clone(), r.relation_type.clone()));
        rel_map
            .entry(r.target_entity_id.clone())
            .or_default()
            .push((r.source_entity_id.clone(), format!("inverse_{}", r.relation_type)));
    }

    // 4) 读取 Wiki 已有笔记标题，跳过已存在的
    let existing = axagent_dao::repo::note::list_notes(db, wiki_id).await.unwrap_or_default();
    let existing_titles: HashSet<String> = existing.iter().map(|n| n.title.clone()).collect();

    let mut created = 0usize;
    let mut skipped = 0usize;

    for entity in &entities {
        if existing_titles.contains(&entity.name) {
            skipped += 1;
            continue;
        }

        // 构建笔记内容：关联节点用 [[wikilinks]] 嵌入
        let mut content = format!(
            "# {}\n\n> ℹ️ 从知识图谱自动导入的实体节点\n\n**类型**: {} \n\n",
            entity.name, entity.entity_type
        );
        if let Some(related) = rel_map.get(&entity.id) {
            let mut links: Vec<String> = Vec::new();
            for (tid, rtype) in related {
                if let Some(tname) = name_by_id.get(tid) {
                    if tname != &entity.name {
                        links.push(format!("- [[{}]]  — *{}*", tname, rtype));
                    }
                }
            }
            if !links.is_empty() {
                content.push_str("## 关联节点\n\n");
                content.push_str(&links.join("\n"));
            }
        }

        let input = axagent_harness::note_dtos::CreateNoteInput {
            vault_id: wiki_id.to_string(),
            title: entity.name.clone(),
            file_path: format!("graph-entities/{}.md", entity.name),
            content,
            author: "graph-import".to_string(),
            page_type: None,
            source_refs: None,
        };

        let created_note = axagent_dao::repo::note::create_note(db, input).await;
        match created_note {
            Ok(_) => {
                // 链接同步不再逐篇执行：create_note 内部的同步只含「已导入」笔记映射，
                // 前向引用会被丢弃；且此处逐篇调用是 O(N²)。统一由导入流程末尾的
                // resync_vault_note_links（全量映射 + 批量写入）完成。
                created += 1;
            },
            Err(e) => {
                tracing::warn!("[graph_to_wiki] 创建笔记失败 {}: {e}", entity.name);
                skipped += 1;
            },
        }
    }

    tracing::info!(
        "[graph_to_wiki] 桥接完成: 创建 {} 篇, 跳过 {} 篇 (kb={}, wiki={})",
        created,
        skipped,
        kb_id,
        wiki_id,
    );

    (created, skipped)
}

// ── 项目知识源一键导入 ───────────────────────────────────

/// 项目知识源导入结果。
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectKnowledgeImportResult {
    /// Wiki 知识库 ID（存放所有 markdown 文件）
    pub wiki_id: String,
    pub wiki_name: String,
    pub wiki_imported: usize,
    pub wiki_failed: usize,
    pub wiki_skipped: usize,
    /// RAG 知识库 ID（存放 lemonhu 知识图谱实体 + 关系）
    pub kb_id: String,
    pub kb_name: String,
    pub entity_count: usize,
    pub relation_count: usize,
    /// 本次图谱→Wiki 桥接创建的笔记数（含 [[wikilinks]]）
    pub bridged_notes: usize,
    pub bridged_skipped: usize,
    pub embedding_provider: Option<String>,
    /// 本次操作是否变更了 embedding_provider（前端据此提示用户重建索引）
    pub embedding_changed: bool,
}

/// 一键导入项目知识源：创建 Wiki 知识库 + 导入知识图谱。
///
/// 参数：
/// - `source_path`：要导入的目录绝对路径（如 `/path/to/knowledge-sources`）
/// - `source_name`：知识源名称（默认 `项目知识源`）。
///   - Wiki vault 名 = `source_name`
///   - RAG KB 名 = `{source_name}图谱`
/// - `mode`：模式（默认 `create`）
///   - `create`：清理同名无 embedding 残次 KB → 创建/复用 Wiki + KB → 全量导入
///   - `update`：找到/创建同名 Wiki + KB → 软删除 Wiki 现有 notes → 重新导入笔记和 wiki_pages
///     （图谱实体/关系按 id 幂等，已存在则跳过）
/// - `embedding_provider`：可选向量模型，格式 `providerId::modelId`。
///   - 创建模式：新 Wiki/KB 直接写入该字段；复用已有 Wiki/KB 时若与现有不同则更新。
///   - 更新模式：传入时与现有不同则更新；不传则保持现状。
///   - 返回 `embedding_changed=true` 时前端应提示用户重建索引。
#[agent_command(domain = knowledge, safety = Caution, call_mode = StateInput, description = "导入项目知识源")]
#[tauri::command]
pub async fn import_project_knowledge_sources(
    app: AppHandle,
    state: State<'_, AppState>,
    source_path: String,
    source_name: Option<String>,
    mode: Option<String>,
    embedding_provider: Option<String>,
) -> Result<ProjectKnowledgeImportResult, String> {
    let dir = PathBuf::from(&source_path);
    if !dir.exists() || !dir.is_dir() {
        return Err(format!("路径不存在或不是目录: {source_path}"));
    }

    let source_name = source_name.unwrap_or_else(|| "项目知识源".to_string());
    let mode = mode.unwrap_or_else(|| "create".to_string());
    let is_update = match mode.as_str() {
        "create" => false,
        "update" => true,
        other => return Err(format!("不支持的模式: {other}（仅支持 create / update）")),
    };

    // 校验 embedding_provider 格式：必须为 `providerId::modelId` 或 None
    let embedding_provider = match embedding_provider.as_deref() {
        None => None,
        Some("") => None,
        Some(ep) => {
            let parts: Vec<&str> = ep.splitn(2, "::").collect();
            if parts.len() != 2 || parts[0].is_empty() || parts[1].is_empty() {
                return Err(format!(
                    "embedding_provider 格式非法：'{ep}'（应为 providerId::modelId）"
                ));
            }
            Some(ep.to_string())
        },
    };

    let wiki_name = source_name.clone();
    let kb_name = format!("{source_name}图谱");

    // ── 所有 DB 操作一次性完成（释放 state 的借用）──
    let (
        wiki_id,
        wiki_name,
        kb_id,
        kb_name,
        (entity_count, relation_count, _doc_count),
        final_embedding_provider,
        embedding_changed,
    ) = {
        let db = state.harness.db();

        // 1) create 模式：清理同名无 embedding 的残次 KB（update 模式不动）
        if !is_update {
            let existing_bases =
                axagent_dao::repo::knowledge::list_knowledge_bases(db).await.map_err(|e| {
                    String::from(crate::commands::error::ErrorResponse::from_error(
                        e,
                        crate::commands::error::ErrorCategory::Unrecoverable,
                    ))
                })?;
            for kb in &existing_bases {
                if kb.name == kb_name && kb.embedding_provider.is_none() {
                    tracing::info!("[import_project] 清理旧的残次 KB: {} ({})", kb.name, kb.id);
                    let collection_id = format!("kb_{}", kb.id);
                    let _ = state.vector_store.delete_collection(&collection_id).await;
                    let _ = axagent_dao::repo::knowledge::delete_knowledge_base(db, &kb.id).await;
                }
            }
        }

        // 2) 创建/复用 Wiki vault
        //    - 新建：直接写入 embedding_provider
        //    - 复用：若传入的 embedding_provider 与现有不同，调用 update_wiki 同步字段
        let (wiki_id, wiki_embedding_changed) = {
            let existing_wikis = axagent_dao::repo::wiki::list_wikis(db).await.map_err(|e| {
                String::from(crate::commands::error::ErrorResponse::from_error(
                    e,
                    crate::commands::error::ErrorCategory::Unrecoverable,
                ))
            })?;
            if let Some(w) = existing_wikis.into_iter().find(|w| w.name == wiki_name) {
                tracing::info!("[import_project] 复用已有 Wiki: {} ({})", wiki_name, w.id);
                let changed = match &embedding_provider {
                    Some(ep) if w.embedding_provider.as_deref() != Some(ep.as_str()) => {
                        tracing::info!(
                            "[import_project] Wiki {} embedding_provider 变更：{:?} => {:?}",
                            w.id,
                            w.embedding_provider,
                            embedding_provider
                        );
                        let _ = axagent_dao::repo::wiki::update_wiki(
                            db,
                            &w.id,
                            None,
                            None,
                            embedding_provider.clone(),
                            None,
                        )
                        .await
                        .map_err(|e| {
                            String::from(crate::commands::error::ErrorResponse::from_error(
                                e,
                                crate::commands::error::ErrorCategory::Unrecoverable,
                            ))
                        })?;
                        true
                    },
                    _ => false,
                };
                (w.id, changed)
            } else {
                tracing::info!("[import_project] 创建新 Wiki: {}", wiki_name);
                let wiki = axagent_dao::repo::wiki::create_wiki(
                    db,
                    axagent_dao::repo::wiki::CreateWikiInput {
                        name: wiki_name.clone(),
                        description: Some(format!("从 {} 自动导入的项目知识源", source_path)),
                        root_path: source_path.clone(),
                        embedding_provider: embedding_provider.clone(),
                        knowledge_base_id: None,
                    },
                )
                .await
                .map_err(|e| {
                    String::from(crate::commands::error::ErrorResponse::from_error(
                        e,
                        crate::commands::error::ErrorCategory::Unrecoverable,
                    ))
                })?;
                // 新建时若指定了 embedding_provider，视为「变更」（前端提示需要建索引）
                (wiki.id, embedding_provider.is_some())
            }
        };

        // 2.5) update 模式：软删除 Wiki 下现有 notes，让 wiki_import_obsidian_vault 重新导入磁盘最新内容
        if is_update {
            let existing_notes =
                axagent_dao::repo::note::list_notes(db, &wiki_id).await.map_err(|e| {
                    String::from(crate::commands::error::ErrorResponse::from_error(
                        e,
                        crate::commands::error::ErrorCategory::Unrecoverable,
                    ))
                })?;
            let note_count = existing_notes.len();
            for note in &existing_notes {
                let _ = axagent_dao::repo::note::delete_note(db, &note.id).await;
            }
            if note_count > 0 {
                tracing::info!(
                    "[import_project] update 模式：软删除 Wiki {} 下 {} 条现有 notes",
                    wiki_id,
                    note_count
                );
            }
        }

        // 3) 创建/复用 KB + 同步 embedding_provider
        let (kb_id, kb_embedding_provider, kb_embedding_changed) = {
            let existing_bases =
                axagent_dao::repo::knowledge::list_knowledge_bases(db).await.map_err(|e| {
                    String::from(crate::commands::error::ErrorResponse::from_error(
                        e,
                        crate::commands::error::ErrorCategory::Unrecoverable,
                    ))
                })?;
            if let Some(kb) = existing_bases.into_iter().find(|b| b.name == kb_name) {
                let changed = match &embedding_provider {
                    Some(ep) if kb.embedding_provider.as_deref() != Some(ep.as_str()) => {
                        tracing::info!(
                            "[import_project] KB {} embedding_provider 变更：{:?} => {:?}",
                            kb.id,
                            kb.embedding_provider,
                            embedding_provider
                        );
                        let _ = axagent_dao::repo::knowledge::update_knowledge_base(
                            db,
                            &kb.id,
                            axagent_harness::types::UpdateKnowledgeBaseInput {
                                name: None,
                                description: None,
                                embedding_provider: embedding_provider.clone(),
                                enabled: None,
                                icon_type: None,
                                icon_value: None,
                                update_icon: false,
                                embedding_dimensions: None,
                                update_embedding_dimensions: false,
                                retrieval_threshold: None,
                                update_retrieval_threshold: false,
                                retrieval_top_k: None,
                                update_retrieval_top_k: false,
                                chunk_size: None,
                                update_chunk_size: false,
                                chunk_overlap: None,
                                update_chunk_overlap: false,
                                separator: None,
                                update_separator: false,
                            },
                        )
                        .await
                        .map_err(|e| {
                            String::from(crate::commands::error::ErrorResponse::from_error(
                                e,
                                crate::commands::error::ErrorCategory::Unrecoverable,
                            ))
                        })?;
                        true
                    },
                    _ => false,
                };
                let updated = if changed {
                    axagent_dao::repo::knowledge::get_knowledge_base(db, &kb.id).await.map_err(
                        |e| {
                            String::from(crate::commands::error::ErrorResponse::from_error(
                                e,
                                crate::commands::error::ErrorCategory::Unrecoverable,
                            ))
                        },
                    )?
                } else {
                    kb
                };
                (updated.id, updated.embedding_provider, changed)
            } else {
                let new_kb = axagent_dao::repo::knowledge::create_knowledge_base(
                    db,
                    axagent_harness::types::CreateKnowledgeBaseInput {
                        name: kb_name.clone(),
                        description: Some("lemonhu A 股知识图谱（实体 + 关系）".into()),
                        embedding_provider: embedding_provider.clone(),
                        enabled: Some(true),
                        kind: Default::default(),
                        vault_path: None,
                    },
                )
                .await
                .map_err(|e| {
                    String::from(crate::commands::error::ErrorResponse::from_error(
                        e,
                        crate::commands::error::ErrorCategory::Unrecoverable,
                    ))
                })?;
                (new_kb.id, new_kb.embedding_provider, embedding_provider.is_some())
            }
        };

        // 3.5) 将 KB ID 关联到 Wiki，建立 Wiki 与 KB 的 1:1 关联
        // 这是修复 Wiki 图谱关联断裂的关键步骤
        if let Err(e) = axagent_dao::repo::wiki::update_wiki(
            db,
            &wiki_id,
            None,
            None,
            None,
            Some(Some(kb_id.clone())),
        )
        .await
        {
            tracing::warn!("[import_project] 关联 Wiki {} 与 KB {} 失败: {}", wiki_id, kb_id, e);
        } else {
            tracing::info!("[import_project] 成功关联 Wiki {} 与 KB {}", wiki_id, kb_id);
        }

        // 4) 导入 lemonhu 图谱（update 模式下强制重新导入 wiki_pages；实体/关系按 id 幂等）
        let lemonhu_dir = dir.join("lemonhu");
        let graph_result = if lemonhu_dir.exists() {
            import_lemonhu_graph(db, &kb_id, &lemonhu_dir, is_update).await
        } else {
            (0, 0, 0)
        };

        // Wiki 与 KB 任一发生变更，则整体视为 embedding_changed
        let embedding_changed = wiki_embedding_changed || kb_embedding_changed;
        (wiki_id, wiki_name, kb_id, kb_name, graph_result, kb_embedding_provider, embedding_changed)
    }; // ← db 引用在此释放，state 恢复可移动状态

    // 5) 桥接图谱→Wiki：为图谱中的实体创建 Wiki 笔记，内含 [[wikilinks]] 关联
    let (bridged_notes, bridged_skipped) =
        bridge_graph_to_wiki(state.harness.db(), &kb_id, &wiki_id).await;

    // 5.5) 链接修复已移至 Wiki 导入（步骤 6）之后执行：
    // wiki_import_obsidian_vault 才是笔记量最大的导入源，在其之前修复只会
    // 处理到部分笔记，且此时构建的映射不完整（前向引用仍会丢失）。

    // 5.6) 失效 Wiki 图谱缓存，确保下次加载时获取最新的图谱数据
    let _ =
        axagent_dao::repo::wiki_graph_cache::invalidate_cache(state.harness.db(), &wiki_id).await;

    // 6) 入队 KB 文档索引任务
    // import_lemonhu_graph 创建文档时仅写入 DB（indexing_status="pending"），未入队 index_queue。
    // 此处统一补齐：查询 KB 下所有 pending 文档，循环入队 JOB_TYPE_INDEX_DOCUMENT，
    // 否则 RAG 检索永远查不到这些文档（vector_store 中无对应 embedding）。
    let kb_has_embedding = final_embedding_provider.is_some();
    if kb_has_embedding {
        let pending_docs: Vec<String> = {
            let db = state.harness.db();
            knowledge_documents::Entity::find()
                .filter(knowledge_documents::Column::KnowledgeBaseId.eq(&kb_id))
                .filter(knowledge_documents::Column::IndexingStatus.eq("pending"))
                .all(db)
                .await
                .map(|docs| docs.into_iter().map(|d| d.id).collect())
                .unwrap_or_default()
        };
        if !pending_docs.is_empty() {
            let count = pending_docs.len();
            for doc_id in &pending_docs {
                let _ = crate::index_queue::enqueue_job_sync(
                    &state,
                    &app,
                    jobs::JOB_TYPE_INDEX_DOCUMENT,
                    "kb",
                    &kb_id,
                    doc_id,
                    None,
                    None,
                );
            }
            tracing::info!("[import_project] 入队 {count} 个 KB 文档索引任务 (kb={kb_id})");
        }
    } else {
        tracing::info!(
            "[import_project] KB 未配置 embedding_provider，跳过文档索引入队 (kb={kb_id})"
        );
    }

    // ── Wiki markdown 导入（state 被移入但不需再使用）──
    // 先克隆 db 句柄（DatabaseConnection 是 Arc 包装，克隆廉价），
    // state 移入导入函数后仍需用它做链接重建与缓存失效。
    let db_after_import = state.harness.db().clone();
    let wiki_result = crate::commands::wiki::wiki_import_obsidian_vault(
        app.clone(),
        state,
        wiki_id.clone(),
        source_path.clone(),
    )
    .await
    .map_err(|e| format!("导入 Wiki 笔记失败: {e}"))?;

    // 7) 全量重建链接表：必须在所有笔记导入完成之后执行。
    // 逐篇导入时的链接同步只含「已导入」笔记的映射，指向后续导入笔记的
    // 前向引用被静默丢弃 —— 这是图谱 0 边、无聚类的根因。
    let repaired_links = repair_wiki_note_links(&db_after_import, &wiki_id).await;
    if repaired_links > 0 {
        tracing::info!(
            "[import_project] 重建 Wiki {} 链接表完成（{} 篇笔记）",
            wiki_id,
            repaired_links
        );
    }

    // 8) 失效图谱缓存（链接表已全量变化）
    let _ = axagent_dao::repo::wiki_graph_cache::invalidate_cache(&db_after_import, &wiki_id).await;

    let result = ProjectKnowledgeImportResult {
        wiki_id,
        wiki_name,
        wiki_imported: wiki_result.imported,
        wiki_failed: wiki_result.failed,
        wiki_skipped: wiki_result.skipped,
        kb_id,
        kb_name,
        entity_count,
        relation_count,
        bridged_notes,
        bridged_skipped,
        embedding_provider: final_embedding_provider,
        embedding_changed,
    };

    tracing::info!(
        "[import_project] {} 完成: Wiki +{}/-{}/={} notes, Graph {} 实体 + {} 关系, Bridge {} notes",
        if is_update { "更新" } else { "导入" },
        result.wiki_imported,
        result.wiki_failed,
        result.wiki_skipped,
        result.entity_count,
        result.relation_count,
        result.bridged_notes,
    );

    Ok(result)
}
