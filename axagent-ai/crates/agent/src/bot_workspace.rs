// SPDX-License-Identifier: AGPL-3.0-only

//! Bot 工作区文件系统层（docs/bot-integration/MODEL.md §5 / ADR-003）。
//!
//! 目录布局（`~/.axagent/bots/<botId>/`）：
//! ```text
//! SOUL.md      — 镜像（DB `bots.soul` 为真相源）
//! MEMORY.md    — 真相源（注入用）
//! memory/      — 主题文件（user/feedback/project/reference + 任意子目录）
//! ```
//!
//! 与 `personality.rs` / `project_memory.rs` 同层：只负责文件系统，不依赖 DB。
//! MEMORY 预算（200 行 / 24 KB）复用 harness 常量，注入与前端预算条同口径。

use std::path::{Path, PathBuf};

use axagent_harness::constants::memory as mem_const;
use axagent_harness::util_fns::truncate_to_char_boundary;
use sha2::{Digest, Sha256};

/// Bot 记忆主题条目（`read_bot_memory` 的 topics 元素）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BotMemoryTopic {
    /// 相对于 `memory/` 的路径（如 `project/architecture.md`）。
    pub relative_path: String,
    /// 分类（首层目录名，非四类时归 `reference`）。
    pub category: String,
    /// 标题（首个 Markdown 一级标题，缺省用文件名）。
    pub title: String,
    /// 文件修改时间（Unix 毫秒；不可得时为 0）。
    pub updated_at: i64,
}

/// Bot 工作区：只持有根路径，不缓存任何状态。
#[derive(Debug, Clone)]
pub struct BotWorkspace {
    root: PathBuf,
}

impl BotWorkspace {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// 默认根：`~/.axagent/bots/<botId>`。
    ///
    /// SOUL/MEMORY **始终**在这里；`bots.workdir` 只影响工具的工作目录，
    /// 记忆不跟着用户项目跑（ADR-003）。
    pub fn for_bot(bot_id: &str) -> Self {
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
        Self::new(home.join(".axagent").join("bots").join(bot_id))
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn soul_path(&self) -> PathBuf {
        self.root.join("SOUL.md")
    }

    pub fn memory_path(&self) -> PathBuf {
        self.root.join("MEMORY.md")
    }

    pub fn memory_dir(&self) -> PathBuf {
        self.root.join("memory")
    }

    /// 建目录（0700）。已存在即为成功。
    pub fn ensure(&self) -> Result<(), String> {
        std::fs::create_dir_all(&self.root)
            .map_err(|e| format!("创建 {} 失败: {e}", self.root.display()))?;
        std::fs::create_dir_all(self.memory_dir())
            .map_err(|e| format!("创建 {} 失败: {e}", self.memory_dir().display()))?;
        set_dir_permissions(&self.root);
        set_dir_permissions(&self.memory_dir());
        Ok(())
    }

    /// 读 SOUL 镜像。文件不存在时视为空镜像（由调用方结合 DB hash 判漂移）。
    pub fn read_soul(&self) -> Result<String, String> {
        match std::fs::read_to_string(self.soul_path()) {
            Ok(s) => Ok(s),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
            Err(e) => Err(format!("读取 {} 失败: {e}", self.soul_path().display())),
        }
    }

    /// 原子写 SOUL 镜像：`SOUL.md.tmp` → rename（同目录 rename 在同一文件系统内原子）。
    pub fn write_soul_atomic(&self, content: &str) -> Result<(), String> {
        self.ensure()?;
        let target = self.soul_path();
        let tmp = target.with_extension("md.tmp");
        std::fs::write(&tmp, content).map_err(|e| format!("写入 {} 失败: {e}", tmp.display()))?;
        set_file_permissions(&tmp);
        std::fs::rename(&tmp, &target).map_err(|e| {
            let _ = std::fs::remove_file(&tmp);
            format!("替换 {} 失败: {e}", target.display())
        })
    }

    /// 统一 hash 算法：`sha256(content)` 的小写十六进制。
    pub fn soul_sha256(content: &str) -> String {
        let mut hasher = Sha256::new();
        hasher.update(content.as_bytes());
        hex::encode(hasher.finalize())
    }

    /// 读 MEMORY.md 原文。文件不存在 → 空串（不是错误）。
    pub fn read_memory_index(&self) -> Result<String, String> {
        match std::fs::read_to_string(self.memory_path()) {
            Ok(s) => Ok(s),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
            Err(e) => Err(format!("读取 {} 失败: {e}", self.memory_path().display())),
        }
    }

    /// 渲染注入提示词用的记忆段：
    /// - 文件不存在或全空白 → `None`
    /// - 截断到 `MEMORY_INDEX_MAX_LINES` 行 / `MEMORY_INDEX_MAX_BYTES` 字节，并留截断标记
    ///
    /// 截断只发生在这里（工作区层），调用点不再二次截断（ADR-005）。
    pub fn render_memory_for_prompt(&self) -> Result<Option<String>, String> {
        let raw = self.read_memory_index()?;
        if raw.trim().is_empty() {
            return Ok(None);
        }
        let mut omitted_lines = 0usize;
        let mut content: String = {
            let mut lines: Vec<&str> = raw.lines().collect();
            if lines.len() > mem_const::MEMORY_INDEX_MAX_LINES {
                omitted_lines = lines.len() - mem_const::MEMORY_INDEX_MAX_LINES;
                lines.truncate(mem_const::MEMORY_INDEX_MAX_LINES);
            }
            lines.join("\n")
        };
        if content.len() > mem_const::MEMORY_INDEX_MAX_BYTES {
            let cut = truncate_to_char_boundary(&content, mem_const::MEMORY_INDEX_MAX_BYTES);
            content = cut.to_string();
            omitted_lines = omitted_lines.max(1);
        }
        if omitted_lines > 0 {
            content.push_str(&format!(
                "\n<!-- truncated: {omitted_lines} more line(s) omitted (memory budget) -->"
            ));
        }
        Ok(Some(content))
    }

    /// 扫描 `memory/**/*.md`（深度 ≤ 3），返回主题清单（按路径排序）。
    pub fn scan_topics(&self) -> Vec<BotMemoryTopic> {
        let mut out = Vec::new();
        collect_topics(&self.memory_dir(), &self.memory_dir(), 0, &mut out);
        out.sort_by(|a, b| a.relative_path.cmp(&b.relative_path));
        out
    }
}

fn collect_topics(base: &Path, dir: &Path, depth: usize, out: &mut Vec<BotMemoryTopic>) {
    if depth > 3 {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_topics(base, &path, depth + 1, out);
            continue;
        }
        if path.extension().and_then(|e| e.to_str()) != Some("md") {
            continue;
        }
        let Ok(rel) = path.strip_prefix(base) else { continue };
        let relative_path = rel.to_string_lossy().replace('\\', "/");
        let category = if relative_path.contains('/') {
            relative_path.split('/').next().unwrap_or("reference").to_string()
        } else {
            "reference".to_string()
        };
        let content = std::fs::read_to_string(&path).unwrap_or_default();
        let title = content
            .lines()
            .find_map(|l| l.trim().strip_prefix("# ").map(|t| t.trim().to_string()))
            .filter(|t| !t.is_empty())
            .unwrap_or_else(|| {
                path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default()
            });
        let updated_at = entry
            .metadata()
            .ok()
            .and_then(|m| m.modified().ok())
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0);
        out.push(BotMemoryTopic { relative_path, category, title, updated_at });
    }
}

#[cfg(unix)]
fn set_dir_permissions(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700));
}

#[cfg(unix)]
fn set_file_permissions(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
}

#[cfg(not(unix))]
fn set_dir_permissions(_path: &Path) {}

#[cfg(not(unix))]
fn set_file_permissions(_path: &Path) {}

#[cfg(test)]
mod tests {
    use super::*;

    fn workspace() -> (tempfile::TempDir, BotWorkspace) {
        let dir = tempfile::tempdir().expect("临时目录");
        let ws = BotWorkspace::new(dir.path().join("bot-1"));
        (dir, ws)
    }

    #[test]
    fn soul_roundtrip_is_atomic_and_creates_dirs() {
        let (_dir, ws) = workspace();
        ws.write_soul_atomic("# 投研助手\n只回答 A 股问题").expect("写 SOUL 应成功");

        assert!(ws.soul_path().is_file(), "SOUL.md 应存在");
        assert!(!ws.soul_path().with_extension("md.tmp").exists(), "临时文件应已被 rename 掉");
        assert_eq!(ws.read_soul().expect("读 SOUL"), "# 投研助手\n只回答 A 股问题");
        assert!(ws.memory_dir().is_dir(), "memory/ 应一并创建");
    }

    #[test]
    fn soul_hash_is_stable_sha256_hex() {
        assert_eq!(
            BotWorkspace::soul_sha256("hello"),
            "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824"
        );
        assert_eq!(BotWorkspace::soul_sha256("hello"), BotWorkspace::soul_sha256("hello"));
        assert_ne!(BotWorkspace::soul_sha256("hello"), BotWorkspace::soul_sha256("hello "));
        assert_eq!(
            BotWorkspace::soul_sha256(""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn memory_lives_at_bot_root_not_dot_axagent() {
        let (_dir, ws) = workspace();
        assert_eq!(ws.memory_path(), ws.root().join("MEMORY.md"));
        assert_eq!(ws.memory_dir(), ws.root().join("memory"));
    }

    #[test]
    fn missing_memory_renders_none() {
        let (_dir, ws) = workspace();
        assert_eq!(ws.render_memory_for_prompt().expect("读取"), None);
    }

    #[test]
    fn memory_render_truncates_to_line_budget() {
        let (_dir, ws) = workspace();
        ws.ensure().expect("建目录");
        let body = (0..250).map(|i| format!("line {i}")).collect::<Vec<_>>().join("\n");
        std::fs::write(ws.memory_path(), body).expect("写 MEMORY.md");

        let rendered = ws.render_memory_for_prompt().expect("渲染").expect("应有内容");
        let lines: Vec<&str> = rendered.lines().collect();
        assert!(lines.len() <= mem_const::MEMORY_INDEX_MAX_LINES + 1, "行数应受预算约束");
        assert!(rendered.contains("truncated: 50 more line(s)"), "应给出截断提示");
        assert!(rendered.contains("line 0"));
        assert!(!rendered.contains("line 249"), "超出预算的尾部不应注入");
    }

    #[test]
    fn memory_render_truncates_to_byte_budget() {
        let (_dir, ws) = workspace();
        ws.ensure().expect("建目录");
        std::fs::write(ws.memory_path(), "记".repeat(mem_const::MEMORY_INDEX_MAX_BYTES))
            .expect("写 MEMORY.md");

        let rendered = ws.render_memory_for_prompt().expect("渲染").expect("应有内容");
        assert!(rendered.len() <= mem_const::MEMORY_INDEX_MAX_BYTES + 128, "字节预算应有界");
        assert!(rendered.contains("truncated"));
    }

    #[test]
    fn scan_topics_reads_category_and_heading() {
        let (_dir, ws) = workspace();
        ws.ensure().expect("建目录");
        let project = ws.memory_dir().join("project");
        std::fs::create_dir_all(&project).expect("建子目录");
        std::fs::write(project.join("architecture.md"), "# 项目架构\nCargo workspace 分层")
            .expect("写主题");
        std::fs::write(ws.memory_dir().join("root-note.md"), "没有标题").expect("写主题");

        let topics = ws.scan_topics();
        assert_eq!(topics.len(), 2);
        let arch =
            topics.iter().find(|t| t.relative_path == "project/architecture.md").expect("应命中");
        assert_eq!(arch.category, "project");
        assert_eq!(arch.title, "项目架构");
        let root_note = topics.iter().find(|t| t.relative_path == "root-note.md").expect("应命中");
        assert_eq!(root_note.category, "reference");
        assert_eq!(root_note.title, "root-note");
    }
}
