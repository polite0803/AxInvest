// SPDX-License-Identifier: AGPL-3.0-only

//! 沙箱拒绝启发式判据。
//!
//! 移植自 codex `sandboxing/src/denial.rs` 的两级判据：
//!
//! 1. **退出码快路**：`2` / `126` / `127` 表示「用户命令自身的问题」
//!    （用法错误 / 不可执行 / 命令未找到），直接排除，不再看 stderr；
//! 2. **关键词匹配**：stderr 小写后命中沙箱 / 权限类关键词，视为疑似沙箱拒绝。
//!
//! codex 在 Linux 侧另有一条 `exit_code == 128 + SIGSYS`（seccomp 触发）分支，
//! 本仓 Linux 沙箱走 `unshare` 命名空间（见 `linux_sandbox.rs`）、不装 seccomp，
//! 不会产生 SIGSYS，故不移植。
//!
//! ⚠ 这是**启发式**而非内核级协议，本身会误判。调用方应把它当作「是否值得多问
//! 一次用户」的收窄条件，而**不是**安全边界本身 —— 安全边界始终是沙箱与
//! `ApprovalPolicy` 的决策矩阵。

/// 退出码快路排除集：这些退出码表示命令自身有问题，与沙箱无关。
const EXCLUDED_EXIT_CODES: [i32; 3] = [2, 126, 127];

/// codex `denial.rs L45-L72` 的 7 条关键词，**逐字照抄**（其语义面向 Unix）。
const CODEX_DENIAL_KEYWORDS: [&str; 7] = [
    "operation not permitted",
    "permission denied",
    "read-only file system",
    "seccomp",
    "sandbox",
    "landlock",
    "failed to write file",
];

/// Windows 扩展关键词 —— **对 codex 的刻意偏离**。
///
/// 理由：codex 的 7 条词全部面向 Unix，Windows 侧一条也命中不了。而本仓 Windows
/// 沙箱（SAFER 受限令牌，见 `win_sandbox.rs`）被拒时，报错文案由操作系统给出、
/// **随系统语言本地化**：本机（zh-CN）实测受限令牌内
/// `cmd /d /s /c "echo x > C:\Windows\x.txt"` ⇒ stderr 为 GBK 编码的「拒绝访问。」
/// （退出码 `1`，stdout 为空）。若只照抄 codex 关键词表，`OnFailure` 的
/// 「沙箱被拒 → 询问 → 沙箱外重试」在 Windows 上**永远不会触发**，
/// 收窄误问就变成了删能力。
///
/// `is denied` 覆盖英文系统 / PowerShell 的 `Access is denied.` 一族；
/// `拒绝访问` 覆盖简体中文系统的本地化文案。代价是「命令自身报错恰含这些词」
/// 会多问一次（与原有行为一致，非新增风险）。
///
/// ⚠ 关键词匹配的前提是 stderr 已被 [`decode_console_text`] 正确解码：
/// 本地化文案走 ANSI/OEM 码页，直接 `from_utf8_lossy` 会得到乱码而匹配不到。
const WINDOWS_DENIAL_KEYWORDS: [&str; 2] = ["is denied", "拒绝访问"];

/// 把控制台输出的原始字节解码为可读文本。
///
/// Windows 下 `cmd.exe` 等系统程序自身产生的报错走 **ANSI/OEM 码页**
/// （zh-CN 为 GBK），不是 UTF-8；直接 `from_utf8_lossy` 会得到乱码，既让
/// [`is_likely_sandbox_denied`] 的关键词匹配失效，用户看到的 stderr 也是乱码。
/// 故先按 UTF-8 试解（现代 CLI 多为 UTF-8 输出），失败再按本机 ANSI 码页解码。
/// 非 Windows 平台的沙箱输出本就是 UTF-8，直接 lossy。
#[must_use]
pub fn decode_console_text(bytes: &[u8]) -> String {
    if let Ok(text) = std::str::from_utf8(bytes) {
        return text.to_string();
    }
    #[cfg(windows)]
    if let Some(text) = decode_ansi(bytes) {
        return text;
    }
    String::from_utf8_lossy(bytes).into_owned()
}

/// 按本机 ANSI 码页（`CP_ACP`）解码；解码失败返回 `None` 由调用方回退 lossy。
#[cfg(windows)]
fn decode_ansi(bytes: &[u8]) -> Option<String> {
    use windows_sys::Win32::Globalization::{CP_ACP, MultiByteToWideChar};

    let len = bytes.len().try_into().ok()?;
    // SAFETY: 首调用只探长度（目标缓冲为 null、长度 0），不写内存；
    // 指针与长度来自同一个 slice，调用期间有效。
    let wide_len =
        unsafe { MultiByteToWideChar(CP_ACP, 0, bytes.as_ptr(), len, std::ptr::null_mut(), 0) };
    if wide_len <= 0 {
        return None;
    }
    let mut buf = vec![0u16; wide_len as usize];
    // SAFETY: buf 容量即 wide_len，与第二参数一致；cbmultibyte 仍为同一 slice 长度。
    let written =
        unsafe { MultiByteToWideChar(CP_ACP, 0, bytes.as_ptr(), len, buf.as_mut_ptr(), wide_len) };
    if written <= 0 {
        return None;
    }
    buf.truncate(written as usize);
    Some(String::from_utf16_lossy(&buf))
}

/// 判断一次非零退出是否「疑似沙箱拒绝」。
///
/// 判定顺序（与 codex 一致，顺序本身即语义）：
///
/// 1. `exit_code == 0` → `false`（成功，无所谓拒绝）；
/// 2. `exit_code ∈ {2, 126, 127}` → `false`（命令自身问题，快路排除）；
/// 3. stderr 小写后含任一关键词（codex 7 条 + Windows 扩展 2 条）→ `true`；
/// 4. 其余 → `false`。
///
/// ⚠ 第 2 步优先于第 3 步：即使 stderr 里出现 `permission denied`，
/// `exit 2` 也**不**判为沙箱拒绝（这是 codex 快路的语义，不要调换顺序）。
pub fn is_likely_sandbox_denied(exit_code: i32, stderr: &str) -> bool {
    if exit_code == 0 {
        return false;
    }
    if EXCLUDED_EXIT_CODES.contains(&exit_code) {
        return false;
    }
    let lower = stderr.to_lowercase();
    CODEX_DENIAL_KEYWORDS.iter().chain(WINDOWS_DENIAL_KEYWORDS.iter()).any(|kw| lower.contains(kw))
}

#[cfg(test)]
mod tests {
    use super::{
        CODEX_DENIAL_KEYWORDS, EXCLUDED_EXIT_CODES, WINDOWS_DENIAL_KEYWORDS, decode_console_text,
        is_likely_sandbox_denied,
    };

    fn all_keywords() -> impl Iterator<Item = &'static str> {
        CODEX_DENIAL_KEYWORDS.iter().chain(WINDOWS_DENIAL_KEYWORDS.iter()).copied()
    }

    /// 覆盖矩阵用样本：每个关键词造一条 stderr。
    fn keyword_stderrs() -> Vec<String> {
        all_keywords().map(|kw| format!("sh: {kw} (os error 1)")).collect()
    }

    #[test]
    fn exit_zero_is_never_denied() {
        for stderr in keyword_stderrs() {
            assert!(!is_likely_sandbox_denied(0, &stderr), "exit 0 不应判为沙箱拒绝: {stderr}");
        }
        // 空 stderr 的成功路径
        assert!(!is_likely_sandbox_denied(0, ""));
    }

    /// 快路：`2` / `126` / `127` 一律排除，**即使 stderr 命中关键词**。
    /// 这是 codex 快路的语义，必须锁住。
    #[test]
    fn excluded_exit_codes_win_over_keywords() {
        for code in EXCLUDED_EXIT_CODES {
            for stderr in keyword_stderrs() {
                assert!(
                    !is_likely_sandbox_denied(code, &stderr),
                    "exit {code} 应被快路排除，即便 stderr 命中关键词: {stderr}"
                );
            }
            // 显式钉住最容易被误改的那条组合
            assert!(!is_likely_sandbox_denied(code, "bash: /x: permission denied"));
        }
    }

    #[test]
    fn keyword_hit_is_denied() {
        for stderr in keyword_stderrs() {
            assert!(is_likely_sandbox_denied(1, &stderr), "exit 1 + 关键词应判沙箱拒绝: {stderr}");
        }
    }

    #[test]
    fn keyword_match_is_case_insensitive() {
        assert!(is_likely_sandbox_denied(1, "PERMISSION DENIED"));
        assert!(is_likely_sandbox_denied(1, "Read-Only File System"));
        assert!(is_likely_sandbox_denied(1, "LandLock: denied"));
    }

    /// Windows 沙箱被拒的**实测文案**必须判为拒绝；否则 OnFailure 升级重试
    /// 在 Windows 上永不触发。
    #[test]
    fn windows_denial_wording_is_denied() {
        // zh-CN 系统本地化文案（本机受限令牌内实测：stderr + exit 1）
        assert!(is_likely_sandbox_denied(1, "拒绝访问。"));
        // 英文系统 / cmd 的等价文案
        assert!(is_likely_sandbox_denied(1, "Access is denied."));
        // PowerShell UnauthorizedAccessException
        assert!(is_likely_sandbox_denied(
            1,
            "Set-Content : Access to the path 'C:\\Windows\\x.txt' is denied."
        ));
    }

    /// GBK 编码的本地化报错必须先按 ANSI 码页解码，否则关键词匹配不到
    /// （`from_utf8_lossy` 会把它解成 U+FFFD 乱码）。
    #[test]
    fn console_text_decodes_legacy_encoding_for_matching() {
        // UTF-8 输入原样保留
        assert_eq!(decode_console_text("拒绝访问。".as_bytes()), "拒绝访问。");
        assert_eq!(decode_console_text(b"Access is denied."), "Access is denied.");
        assert_eq!(decode_console_text(b""), "");

        #[cfg(windows)]
        {
            use windows_sys::Win32::Globalization::GetACP;

            // SAFETY: GetACP 无参数、无副作用，仅返回本机 ANSI 码页。
            let acp = unsafe { GetACP() };
            if acp == 936 {
                // 「拒绝访问。」的 GBK（码页 936）字节
                let gbk = [0xBE, 0xDC, 0xBE, 0xF8, 0xB7, 0xC3, 0xCE, 0xCA, 0xA1, 0xA3];
                let decoded = decode_console_text(&gbk);
                assert_eq!(decoded, "拒绝访问。", "GBK 字节应按本机 ANSI 码页解码");
                assert!(is_likely_sandbox_denied(1, &decoded), "解码后的本地化文案应判为沙箱拒绝");
            }
        }
    }

    /// 命令自身报错（无关键词）不得判为沙箱拒绝 —— 这是 R1-2 收窄误问的基础。
    #[test]
    fn plain_command_failure_is_not_denied() {
        assert!(!is_likely_sandbox_denied(1, ""));
        assert!(!is_likely_sandbox_denied(1, "grep: foo.txt: No such file or directory"));
        assert!(!is_likely_sandbox_denied(1, "test failed: expected 1, got 2"));
        assert!(!is_likely_sandbox_denied(65, "error: could not compile `axagent`"));
        // 负退出码（信号终止）无关键词时同样不判拒绝
        assert!(!is_likely_sandbox_denied(-1, "killed"));
    }

    /// 完整矩阵：退出码 × 关键词含/不含。
    #[test]
    fn exit_code_keyword_matrix() {
        for code in [0, 1, 2, 126, 127, 128, 137] {
            let fast_path_excluded = code == 0 || EXCLUDED_EXIT_CODES.contains(&code);
            for (stderr, has_kw) in [
                ("", false),
                ("grep: no such file", false),
                ("bash: line 1: operation not permitted", true),
                ("Access is denied.", true),
            ] {
                let expected = !fast_path_excluded && has_kw;
                assert_eq!(
                    is_likely_sandbox_denied(code, stderr),
                    expected,
                    "exit={code} stderr={stderr:?} 判定与预期不符"
                );
            }
        }
    }
}
