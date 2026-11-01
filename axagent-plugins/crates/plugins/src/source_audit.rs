// SPDX-License-Identifier: AGPL-3.0-only

//! 插件源码静态审计（`PLAN-everything-is-plugin.md` §12.2）。
//!
//! **定位：形式合规检查，不是安全边界。** `build.rs` 与 proc-macro 在编译期以宿主
//! 用户身份执行任意代码，不在任何运行期沙箱范围内 ⇒ 任何源码级检查都可能被绕过。
//! 本模块的价值是「拦掉已知的绕过面」+「给拦不住的能力打信任分级标注」，真正的
//! 纵深防御在 §12.3 的隔离构建（离线 + 独立 target-dir + 无网络）。
//!
//! 两类产出：
//! - [`SourceAuditReport::violations`]：命中**禁项**即 `passed = false`（一票否决）；
//! - [`SourceAuditReport::annotations`]：**不拦截**但须标注的能力（任意路径文件读写、
//!   子进程调用）—— 见 §8.2「无 OS 级约束」的诚实结论，它用于**信任分级**。
//!
//! [`rules`] 里的规则标识是**稳定字符串**（前端与日志据此分支），不要跟着文案改。
//!
//! 已知局限（形式合规检查的固有代价，不试图消除）：块注释 `/* */` 与字符串字面量
//! 内的关键字仍会被命中；行注释（`//` 起首）已跳过以降低误报。命中项一律交人工复核。

use serde::{Deserialize, Serialize};

/// 审计规则标识（稳定字符串）。
pub mod rules {
    /// 存在 `build.rs`：编译期任意代码执行。
    pub const BUILD_SCRIPT: &str = "build_script";
    /// 源码中定义 proc-macro：proc-macro 本身是编译期代码。
    pub const PROC_MACRO: &str = "proc_macro";
    /// `Cargo.toml` 依赖含 `git =`：拉入不可审计的外部代码。
    pub const GIT_DEPENDENCY: &str = "git_dependency";
    /// `Cargo.toml` 依赖含 `path =`：同上，且可指向插件目录之外。
    pub const PATH_DEPENDENCY: &str = "path_dependency";
    /// 源码含 `unsafe`：需显式申请 + 人工复核（严格档）。
    pub const UNSAFE_CODE: &str = "unsafe_code";
    /// 源码引用 `std::fs::`：**标注**（不拦截），用于信任分级。
    pub const FS_ACCESS: &str = "fs_access";
    /// 源码引用 `std::process::Command`：**标注**（不拦截），用于信任分级。
    pub const PROCESS_COMMAND: &str = "process_command";
}

/// 待审计的单个源文件（路径 + 内容）。
///
/// 内容由调用方提供 —— 本模块**不读盘、不联网**，是纯函数，便于单测与复用
/// （生成管线产出源码后可直接送审，无需先落盘）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceFile {
    /// 仓库内相对路径（如 `src/main.rs`、`Cargo.toml`）。
    pub path: String,
    /// 文件全文。
    pub content: String,
}

/// 一条审计发现。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuditFinding {
    /// 规则标识，取值见 [`rules`]。
    pub rule: String,
    /// 命中的文件路径。
    pub path: String,
    /// 命中行号（1-based）；`0` 表示该规则不针对具体行。
    pub line: usize,
    /// 命中的原文摘要（技术详情，不做 i18n）。
    pub detail: String,
}

/// 审计报告。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceAuditReport {
    /// 无任何禁项命中 ⇒ `true`；标注项不影响本字段。
    pub passed: bool,
    /// 命中禁项（一票否决）。
    pub violations: Vec<AuditFinding>,
    /// 须标注的能力（不拦截，用于信任分级）。
    pub annotations: Vec<AuditFinding>,
}

/// 审计一组源文件。
///
/// 逐文件扫描：路径 basename 为 `build.rs` 即命中禁项（**内容仍继续扫描** —— 否则
/// 加一个 `build.rs` 就能让其余规则集体沉默）；`Cargo.toml` 额外扫依赖段的 `git` /
/// `path`；每行扫 `#[proc_macro` / `unsafe`（禁项）与 `std::fs::` / `Command`（标注）。
pub fn audit_sources(files: &[SourceFile]) -> SourceAuditReport {
    let mut violations = Vec::new();
    let mut annotations = Vec::new();

    for file in files {
        let base = basename(&file.path).to_ascii_lowercase();

        if base == "build.rs" {
            violations.push(AuditFinding {
                rule: rules::BUILD_SCRIPT.to_string(),
                path: file.path.clone(),
                line: 1,
                detail: "存在 build.rs：编译期以宿主用户身份执行任意代码".to_string(),
            });
        }

        if base == "cargo.toml" {
            audit_manifest(file, &mut violations);
        }

        audit_lines(file, &mut violations, &mut annotations);
    }

    SourceAuditReport { passed: violations.is_empty(), violations, annotations }
}

/// 取路径最后一段（`/` 与 `\` 都算分隔符，兼容 Windows 形态的相对路径）。
fn basename(path: &str) -> &str {
    path.rsplit(['/', '\\']).next().unwrap_or(path)
}

/// 扫描 `Cargo.toml` 的各依赖段，命中 `git` / `path` 依赖。
fn audit_manifest(file: &SourceFile, violations: &mut Vec<AuditFinding>) {
    // 段名含 `dependencies` 即视为依赖段 —— 同时覆盖 `[dependencies]`、
    // `[dependencies.foo]`、`[build-dependencies]`、`[target.'cfg(..)'.dependencies]`
    // 与 `[workspace.dependencies]` 五种形态。
    let mut in_deps = false;

    for (idx, raw) in file.content.lines().enumerate() {
        let line = raw.trim();

        if line.starts_with('[') {
            in_deps = line.contains("dependencies");
            continue;
        }
        if !in_deps {
            continue;
        }

        if has_toml_key(line, "git") {
            violations.push(finding(rules::GIT_DEPENDENCY, file, idx + 1, line));
        }
        if has_toml_key(line, "path") {
            violations.push(finding(rules::PATH_DEPENDENCY, file, idx + 1, line));
        }
    }
}

/// 逐行扫描禁项与标注项。
fn audit_lines(
    file: &SourceFile,
    violations: &mut Vec<AuditFinding>,
    annotations: &mut Vec<AuditFinding>,
) {
    for (idx, raw) in file.content.lines().enumerate() {
        let line_no = idx + 1;
        let trimmed = raw.trim_start();

        // 行注释不算代码：跳过可显著降低误报（块注释与字符串是已知局限）。
        if trimmed.starts_with("//") {
            continue;
        }

        if trimmed.contains("#[proc_macro") {
            violations.push(finding(rules::PROC_MACRO, file, line_no, trimmed));
        }
        // 词边界匹配：`unsafe_code`（lint 名）/ `UnsafeCell` 这类不应命中。
        if contains_word(raw, "unsafe") {
            violations.push(finding(rules::UNSAFE_CODE, file, line_no, trimmed));
        }
        // 以下两项**只标注**：§8.2 已说明无 OS 级约束，拦截只会制造虚假安全感。
        if raw.contains("std::fs::") {
            annotations.push(finding(rules::FS_ACCESS, file, line_no, trimmed));
        }
        if raw.contains("std::process::Command") || raw.contains("Command::new(") {
            annotations.push(finding(rules::PROCESS_COMMAND, file, line_no, trimmed));
        }
    }
}

/// 构造一条发现，`detail` 取命中行的截断摘要。
fn finding(rule: &str, file: &SourceFile, line: usize, excerpt_src: &str) -> AuditFinding {
    AuditFinding {
        rule: rule.to_string(),
        path: file.path.clone(),
        line,
        detail: excerpt(excerpt_src),
    }
}

/// 截断超长行（按**字符**截断，不在多字节字符中间切开）。
fn excerpt(line: &str) -> String {
    const MAX_CHARS: usize = 120;

    let mut out: String = line.chars().take(MAX_CHARS).collect();
    if line.chars().count() > MAX_CHARS {
        out.push('…');
    }
    out
}

/// 行内是否存在**词边界**上的 `word`。
fn contains_word(line: &str, word: &str) -> bool {
    let mut start = 0;
    while let Some(pos) = line[start..].find(word) {
        let begin = start + pos;
        let end = begin + word.len();
        let before_ok = begin == 0 || !is_word_byte(line.as_bytes()[begin - 1]);
        let after_ok = end >= line.len() || !is_word_byte(line.as_bytes()[end]);
        if before_ok && after_ok {
            return true;
        }
        start = end;
    }
    false
}

/// 行内是否存在形如 `key = ...`（或 `key=...`）的 TOML 键，`key` 须落在词边界上。
///
/// 为什么不用 `contains("git")`：`digit = "1"` 含子串 `git`，会假阳性。
fn has_toml_key(line: &str, key: &str) -> bool {
    let mut start = 0;
    while let Some(pos) = line[start..].find(key) {
        let begin = start + pos;
        let end = begin + key.len();
        let before_ok = begin == 0 || !is_word_byte(line.as_bytes()[begin - 1]);
        if before_ok && line[end..].trim_start().starts_with('=') {
            return true;
        }
        start = end;
    }
    false
}

/// 标识符字符（ASCII 字母 / 数字 / 下划线）。
fn is_word_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(path: &str, content: &str) -> SourceFile {
        SourceFile { path: path.to_string(), content: content.to_string() }
    }

    fn rules_of(findings: &[AuditFinding]) -> Vec<&str> {
        findings.iter().map(|f| f.rule.as_str()).collect()
    }

    #[test]
    fn clean_source_passes_without_findings() {
        let files = [file("src/main.rs", "fn main() {\n    println!(\"hello\");\n}\n")];
        let report = audit_sources(&files);

        assert!(report.passed, "干净源码应通过: {report:?}");
        assert!(report.violations.is_empty());
        assert!(report.annotations.is_empty());
    }

    #[test]
    fn build_script_is_violation_and_content_still_scanned() {
        // build.rs 命中禁项，且**不因它的存在**而让同批文件里的其他规则沉默。
        let files = [
            file("build.rs", "fn main() {}\n"),
            file("src/main.rs", "fn f() { let _ = unsafe { 1 }; }\n"),
        ];
        let report = audit_sources(&files);

        assert!(!report.passed);
        assert_eq!(rules_of(&report.violations), vec!["build_script", "unsafe_code"]);
        assert_eq!(report.violations[0].line, 1);
    }

    #[test]
    fn windows_style_build_script_path_is_matched() {
        let files = [file("sub\\build.rs", "fn main() {}\n")];
        let report = audit_sources(&files);

        assert_eq!(rules_of(&report.violations), vec!["build_script"]);
    }

    #[test]
    fn proc_macro_definition_is_violation() {
        let files = [file(
            "src/lib.rs",
            "use proc_macro::TokenStream;\n\n#[proc_macro]\npub fn my_macro(input: TokenStream) -> TokenStream { input }\n",
        )];
        let report = audit_sources(&files);

        assert_eq!(rules_of(&report.violations), vec!["proc_macro"]);
        assert_eq!(report.violations[0].line, 3);
    }

    #[test]
    fn git_and_path_dependencies_are_violations() {
        let manifest = "[package]\nname = \"demo\"\n\n[dependencies]\nserde = \"1\"\ndigit = \"1\"\nfoo = { git = \"https://example.com/foo\" }\n\n[dependencies.bar]\npath = \"../bar\"\n";
        let files = [file("Cargo.toml", manifest)];
        let report = audit_sources(&files);

        // `digit = "1"` 不得被当成 `git` 依赖（子串陷阱）。
        assert_eq!(rules_of(&report.violations), vec!["git_dependency", "path_dependency"]);
        assert_eq!(report.violations[0].line, 7);
        assert_eq!(report.violations[1].line, 10);
    }

    #[test]
    fn dependency_like_keys_outside_dependency_sections_are_ignored() {
        let manifest =
            "[package]\nname = \"demo\"\nrepository = { git = \"https://example.com/demo\" }\n";
        let files = [file("Cargo.toml", manifest)];
        let report = audit_sources(&files);

        assert!(report.passed, "非依赖段的 git 键不应命中: {report:?}");
    }

    #[test]
    fn unsafe_word_boundary_and_line_comments() {
        let src = "// unsafe 是禁项（注释应跳过）\nlet cell = UnsafeCell::new(1);\n#[allow(unsafe_code)]\nfn f() { let _ = unsafe { 1 }; }\n";
        let files = [file("src/main.rs", src)];
        let report = audit_sources(&files);

        // 只命中真正写出的 `unsafe` 块（第 4 行）；注释、`UnsafeCell`、`unsafe_code` 都不命中。
        assert_eq!(rules_of(&report.violations), vec!["unsafe_code"]);
        assert_eq!(report.violations[0].line, 4);
    }

    #[test]
    fn fs_and_process_are_annotations_not_violations() {
        let src = "use std::fs;\nuse std::process::Command;\nfn f() { let _ = std::fs::read_to_string(\"a\"); let _ = Command::new(\"ls\"); }\n";
        let files = [file("src/main.rs", src)];
        let report = audit_sources(&files);

        assert!(report.passed, "标注项不拦截: {report:?}");
        assert!(report.violations.is_empty());
        // `use std::fs;` 不含 `std::fs::`（是 `;` 不是 `::`），故只命中第 2、3 行。
        assert_eq!(
            rules_of(&report.annotations),
            vec!["process_command", "fs_access", "process_command"]
        );
    }

    #[test]
    fn excerpt_truncates_on_char_boundary() {
        // 200 个中文字符（每字符 3 字节）：按字符截断不得产生非法 UTF-8。
        let line = "汉".repeat(200);
        let files = [file("src/main.rs", format!("let _ = unsafe {{}}; {line}\n").as_str())];
        let report = audit_sources(&files);

        let detail = &report.violations[0].detail;
        assert!(detail.ends_with('…'), "超长行应带截断标记: {detail}");
        assert_eq!(detail.chars().count(), 121);
    }
}
