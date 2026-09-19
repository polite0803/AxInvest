#![allow(clippy::result_large_err)]
// SPDX-License-Identifier: AGPL-3.0-only

//! Document text extraction — PDF, DOCX, XLSX, PPTX, and plain text.
//!
//! Extracted from `axagent-core` as part of harness architecture refactoring.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::sync::mpsc;
use std::time::Duration;

use axagent_harness::core_error::{AxAgentError, Result};
use calamine::Reader;

/// 解析器外部工具的可配置参数。
///
/// 由 wiring 层（主 crate init）在启动时通过 [`set_parser_options`] 注入一次；
/// 未注入时使用默认值（tesseract 语言 `eng+chi_sim`、超时 120s、whisper-cli）。
/// 仅承载「外部命令路径 / 参数」这类进程级配置，不承载每次调用不同的参数。
#[derive(Debug, Clone)]
pub struct ParserOptions {
    /// tesseract OCR 语言组合（`-l` 参数，如 `eng+chi_sim`）
    pub ocr_lang: String,
    /// 单次外部命令超时秒数（OCR / 转写共用）
    pub external_timeout_secs: u64,
    /// whisper 转写可执行文件（whisper.cpp 的 `whisper-cli` 或等价 CLI）
    pub whisper_bin: String,
    /// whisper 模型文件路径（`-m` 参数）；空串则省略该参数（让 CLI 用默认模型）
    pub whisper_model: String,
    /// 转码可执行文件（`ffmpeg`），非 WAV 音视频转写前先转 16k 单声道 WAV
    pub ffmpeg_bin: String,
}

impl Default for ParserOptions {
    fn default() -> Self {
        Self {
            ocr_lang: "eng+chi_sim".to_string(),
            external_timeout_secs: 120,
            whisper_bin: "whisper-cli".to_string(),
            whisper_model: String::new(),
            ffmpeg_bin: "ffmpeg".to_string(),
        }
    }
}

static PARSER_OPTIONS: OnceLock<ParserOptions> = OnceLock::new();

/// 设置进程级解析器配置（启动时注入一次；重复调用仅保留首次）。
pub fn set_parser_options(opts: ParserOptions) {
    let _ = PARSER_OPTIONS.set(opts);
}

/// 读取当前解析器配置（未注入时返回默认值）。
pub fn parser_options() -> &'static ParserOptions {
    PARSER_OPTIONS.get_or_init(ParserOptions::default)
}

/// Extract plain text from a document file based on its MIME type.
pub fn extract_text(file_path: &Path, mime_type: &str) -> Result<String> {
    match mime_type {
        // Plain text files
        "text/plain" | "text/markdown" | "text/csv" | "text/html" | "text/xml"
        | "application/xml" => std::fs::read_to_string(file_path).map_err(|e| {
            AxAgentError::execution_with_source(
                format!("Failed to read file: {}", file_path.display()),
                e,
            )
        }),

        // JSON（ipynb 是 JSON 的笔记本格式，单独提取 cell 源码）
        "application/json" => {
            if file_path
                .extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| e.eq_ignore_ascii_case("ipynb"))
            {
                extract_ipynb(file_path)
            } else {
                std::fs::read_to_string(file_path).map_err(|e| {
                    AxAgentError::execution_with_source(
                        format!("Failed to read file: {}", file_path.display()),
                        e,
                    )
                })
            }
        },

        // YAML / TOML 配置类
        "application/yaml" | "application/x-yaml" => extract_yaml(file_path),
        "application/toml" => extract_toml(file_path),

        // 邮件
        "message/rfc822" => extract_eml(file_path),

        // PDF
        "application/pdf" => {
            let text = extract_pdf(file_path)?;
            if !text.trim().is_empty() {
                return Ok(text);
            }
            // 文本层为空 ⇒ 扫描版 PDF：pdftoppm 转逐页图片后走 tesseract OCR。
            // 注意 tesseract 不能直接读 PDF，此前把 PDF 路径直喂 tesseract 必然失败。
            //
            // OCR 不可用（Err）与「OCR 可用但图中确实没有文字」（Ok 空串）是两种
            // 不同的事实，但**都必须报错**：此前这里 `unwrap_or_default()` 后返回
            // `Ok("")`，上游 `prepare_chunks` 得到零 chunk、文档状态仍被置为 `ready`
            // —— 用户看到「索引成功」而知识库零内容（2026-09-15 修，假成功源头）。
            match extract_scanned_pdf(file_path) {
                Ok(ocr_text) if !ocr_text.trim().is_empty() => Ok(ocr_text),
                Ok(_) => Err(AxAgentError::Provider(format!(
                    "PDF 文本层为空，且 OCR 未识别出文字（可能为纯图片/矢量图）: {}",
                    file_path.display()
                ))),
                Err(e) => Err(AxAgentError::Provider(format!(
                    "PDF 文本层为空，扫描 PDF OCR 失败（{e}）: {}",
                    file_path.display()
                ))),
            }
        },

        // 图片类型 —— 直接走 OCR
        "image/png" | "image/jpeg" | "image/tiff" | "image/bmp" | "image/webp" => {
            match ocr_fallback(file_path) {
                Ok(ocr_text) if !ocr_text.trim().is_empty() => Ok(ocr_text),
                Ok(_) => Err(AxAgentError::Provider(format!(
                    "OCR 未识别出文字，MIME 类型 '{mime_type}': {}",
                    file_path.display()
                ))),
                // OCR 不可用（tesseract 未安装 / 调用失败）必须把原因带出来，
                // 不能降级成「未识别出文字」—— 那会把「环境缺依赖」误报成「图里没字」。
                Err(e) => Err(AxAgentError::Provider(format!(
                    "OCR 不可用（{e}），无法解析图片: {}",
                    file_path.display()
                ))),
            }
        },

        // 音频 / 视频 —— 外部 whisper CLI 转写（非 WAV 先经 ffmpeg 转 16k 单声道 WAV）
        "audio/mpeg" | "audio/wav" | "audio/x-wav" | "audio/ogg" | "audio/mp4" | "audio/flac"
        | "audio/webm" | "audio/aac" | "audio/x-m4a" | "audio/m4a" | "audio/opus"
        | "audio/x-aiff" | "audio/x-ms-wma" | "video/mp4" | "video/webm" | "video/quicktime"
        | "video/x-msvideo" | "video/x-matroska" | "video/ogg" | "video/3gpp" | "video/mpeg" => {
            extract_transcript(file_path).map_err(|e| {
                AxAgentError::Provider(format!("音视频转写失败（{e}）: {}", file_path.display()))
            })
        },

        // DOCX — basic XML extraction without external crate
        "application/vnd.openxmlformats-officedocument.wordprocessingml.document" => {
            extract_docx(file_path)
        },

        // 旧版二进制 Word — 外部 antiword 命令
        "application/msword" => extract_doc(file_path),

        // ODT — zip 内 content.xml（text:p / text:h 段落）
        "application/vnd.oasis.opendocument.text" => extract_odt(file_path),

        // RTF — 自研控制字剥离
        "application/rtf" | "text/rtf" => extract_rtf(file_path),

        // EPUB — zip 内 OPS content.xhtml，复用 HTML→文本剥离
        "application/epub+zip" => extract_epub(file_path),

        // XLSX / XLS / ODS — calamine（统一走 workbook 读取）
        "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"
        | "application/vnd.ms-excel"
        | "application/vnd.oasis.opendocument.spreadsheet" => extract_spreadsheet(file_path),

        // PPTX — extract text from PowerPoint presentations
        "application/vnd.openxmlformats-officedocument.presentationml.presentation" => {
            extract_pptx(file_path)
        },

        _ => {
            // Try reading as plain text as fallback
            std::fs::read_to_string(file_path).map_err(|e| {
                AxAgentError::execution_with_source(
                    format!("Unsupported MIME type '{}' for {}", mime_type, file_path.display()),
                    e,
                )
            })
        },
    }
}

/// 通用外部命令文本抽取：spawn 子线程执行命令并取 stdout，带超时。
///
/// - 命令不存在（PATH 找不到）/ 调用失败 / 超时 / 工作线程异常 一律返回 `Err(原因)`
/// - 供 tesseract OCR、antiword（.doc）等外部 CLI 解析器复用
/// - 同步实现（std::process），超时通过子线程 + mpsc channel
///
/// 返回类型显式写 `std::result::Result` 以避免与 crate 内 import 的
/// `axagent_harness::core_error::Result`（type alias，只接受 1 个泛型）冲突。
pub fn external_command_extract(
    cmd: &str,
    args: &[&str],
    timeout: Duration,
    timeout_hint: &str,
) -> std::result::Result<String, String> {
    let cmd_owned = cmd.to_string();
    let args_owned: Vec<String> = args.iter().map(|s| (*s).to_string()).collect();
    let (tx, rx) = mpsc::channel::<std::io::Result<std::process::Output>>();

    let worker = std::thread::spawn(move || {
        let result = std::process::Command::new(&cmd_owned).args(&args_owned).output();
        // 忽略发送失败（接收端超时后已 drop）
        let _ = tx.send(result);
    });

    match rx.recv_timeout(timeout) {
        Ok(Ok(output)) => {
            // 命令成功执行 —— 取 stdout 文本
            let text = String::from_utf8_lossy(&output.stdout).to_string();
            // 等待工作线程退出，避免泄漏
            let _ = worker.join();
            Ok(text)
        },
        Ok(Err(e)) if e.kind() == std::io::ErrorKind::NotFound => {
            // 命令未安装 —— 属环境缺依赖，必须报错让用户知道
            let _ = worker.join();
            Err(format!("{cmd} 未安装（PATH 中找不到该命令）"))
        },
        Ok(Err(e)) => {
            // 命令调用失败 —— 同上，带出原始原因
            let _ = worker.join();
            Err(format!("{cmd} 调用失败: {e}"))
        },
        Err(mpsc::RecvTimeoutError::Timeout) => {
            // 超时：尽力 kill 已 spawn 的子进程（通过 drop worker 不能 kill child，
            // 这里只能让 worker 线程在后台自然结束，主流程继续）
            tracing::warn!(target: "document-parser", "{cmd} 超时 ({timeout_hint})，跳过");
            Err(format!("{cmd} 超时 ({timeout_hint})"))
        },
        Err(mpsc::RecvTimeoutError::Disconnected) => {
            // 工作线程 panic 或提前结束 —— 属外部命令不可用，不得伪装成「无内容」
            let _ = worker.join();
            Err(format!("{cmd} 工作线程异常退出（未取到输出）"))
        },
    }
}

/// OCR 回退：调用系统 tesseract 命令行工具识别图片中的文字。
///
/// - 语言组合与超时来自 [`parser_options`]（默认 `eng+chi_sim` / 120s，可启动时注入）
/// - **失败一律返回 `Err(原因)`**（tesseract 未安装 / 调用失败 / 工作线程异常退出）。
///   `Ok` 只表示 tesseract 正常执行完毕，其返回值**可能为空**（图里确实没文字）。
///   此前三种失败都返回 `Ok(String::new())`，把「环境缺依赖」伪装成「文档为空」，
///   是文档索引假成功的源头（2026-09-15 修）。
pub fn ocr_fallback(path: &Path) -> std::result::Result<String, String> {
    let opts = parser_options();
    let path_str = path.to_string_lossy().to_string();
    external_command_extract(
        "tesseract",
        &[&path_str, "stdout", "-l", opts.ocr_lang.as_str()],
        Duration::from_secs(opts.external_timeout_secs),
        &format!("{} 秒", opts.external_timeout_secs),
    )
}

/// 运行外部命令并检查退出码（stdout 不作为结果，适合「写文件型」命令：
/// pdftoppm 转图片、ffmpeg 转码、whisper-cli 写 txt 等）。
///
/// - 命令不存在 / 调用失败 / 超时 / 非零退出码 一律 `Err(原因)`（stderr 带出）
/// - 复用 `external_command_extract` 的 worker 线程 + mpsc 超时骨架；
///   两者语义不同（取 stdout vs 检查退出码），故保留两个函数
fn run_external_command_checked(
    cmd: &str,
    args: &[&str],
    timeout: Duration,
    timeout_hint: &str,
) -> std::result::Result<(), String> {
    let cmd_owned = cmd.to_string();
    let args_owned: Vec<String> = args.iter().map(|s| (*s).to_string()).collect();
    let (tx, rx) = mpsc::channel::<std::io::Result<std::process::Output>>();

    let worker = std::thread::spawn(move || {
        let result = std::process::Command::new(&cmd_owned).args(&args_owned).output();
        let _ = tx.send(result);
    });

    match rx.recv_timeout(timeout) {
        Ok(Ok(output)) => {
            let _ = worker.join();
            if output.status.success() {
                Ok(())
            } else {
                let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
                let detail = if stderr.is_empty() {
                    "无 stderr 输出".to_string()
                } else {
                    stderr
                };
                Err(format!("{cmd} 退出码非 0（{}）: {detail}", output.status))
            }
        },
        Ok(Err(e)) if e.kind() == std::io::ErrorKind::NotFound => {
            let _ = worker.join();
            Err(format!("{cmd} 未安装（PATH 中找不到该命令）"))
        },
        Ok(Err(e)) => {
            let _ = worker.join();
            Err(format!("{cmd} 调用失败: {e}"))
        },
        Err(mpsc::RecvTimeoutError::Timeout) => {
            tracing::warn!(target: "document-parser", "{cmd} 超时 ({timeout_hint})，跳过");
            Err(format!("{cmd} 超时 ({timeout_hint})"))
        },
        Err(mpsc::RecvTimeoutError::Disconnected) => {
            let _ = worker.join();
            Err(format!("{cmd} 工作线程异常退出（未取到输出）"))
        },
    }
}

/// 扫描版 PDF OCR：pdftoppm 逐页转 PNG 后逐页 tesseract，拼接页文本。
///
/// - 依赖 poppler 的 `pdftoppm`（转图）与 tesseract（OCR），二者缺一即报错
/// - 页间以 `=== 第 N 页 ===` 分隔，便于定位来源页
fn extract_scanned_pdf(pdf_path: &Path) -> std::result::Result<String, String> {
    let opts = parser_options();
    let timeout = Duration::from_secs(opts.external_timeout_secs);
    let timeout_hint = format!("{} 秒", opts.external_timeout_secs);

    let dir = tempfile::tempdir().map_err(|e| format!("创建临时目录失败: {e}"))?;
    let prefix = dir.path().join("page");
    let prefix_str = prefix.to_string_lossy().to_string();

    run_external_command_checked(
        "pdftoppm",
        &["-r", "200", "-png", &pdf_path.to_string_lossy(), &prefix_str],
        timeout,
        &timeout_hint,
    )?;

    // 枚举生成的 page-*.png，按文件名排序（页码顺序）
    let mut pages: Vec<PathBuf> = std::fs::read_dir(dir.path())
        .map_err(|e| format!("读取临时目录失败: {e}"))?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("page-") && n.ends_with(".png"))
        })
        .collect();
    pages.sort();

    if pages.is_empty() {
        return Err("pdftoppm 未生成任何页面图片（PDF 可能已损坏或无页）".to_string());
    }

    let mut result = String::new();
    for (i, page) in pages.iter().enumerate() {
        let text = ocr_fallback(page)?;
        if !result.is_empty() {
            result.push('\n');
        }
        result.push_str(&format!("=== 第 {} 页 ===\n{}", i + 1, text.trim()));
    }
    Ok(result)
}

/// 音视频转写：非 WAV 输入先经 ffmpeg 转 16k 单声道 WAV，再调 whisper CLI 转写。
///
/// - 依赖 whisper.cpp 的 `whisper-cli`（或等价 CLI；命令名 / 模型路径经 [`parser_options`] 配置）
/// - whisper.cpp 只吃 WAV，故非 WAV 输入需 ffmpeg 前置转码
/// - CLI 用法约定：`whisper-cli [-m <model>] -f <wav> -otxt -of <prefix>`，产物 `<prefix>.txt`
fn extract_transcript(file_path: &Path) -> std::result::Result<String, String> {
    let opts = parser_options();
    // 转写比 OCR 更耗时（长音频），超时放宽 5 倍
    let timeout = Duration::from_secs(opts.external_timeout_secs * 5);
    let timeout_hint = format!("{} 秒", opts.external_timeout_secs * 5);

    let dir = tempfile::tempdir().map_err(|e| format!("创建临时目录失败: {e}"))?;

    // 1) 非 WAV → ffmpeg 转 16k 单声道 WAV
    let is_wav = file_path
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("wav"));
    let input = if is_wav {
        file_path.to_path_buf()
    } else {
        let wav_path = dir.path().join("audio.wav");
        run_external_command_checked(
            &opts.ffmpeg_bin,
            &[
                "-y",
                "-i",
                &file_path.to_string_lossy(),
                "-ar",
                "16000",
                "-ac",
                "1",
                &wav_path.to_string_lossy(),
            ],
            timeout,
            &timeout_hint,
        )?;
        wav_path
    };

    // 2) whisper-cli 转写 → <prefix>.txt
    let prefix = dir.path().join("out");
    let input_str = input.to_string_lossy().to_string();
    let prefix_str = prefix.to_string_lossy().to_string();
    let mut args: Vec<&str> = Vec::new();
    if !opts.whisper_model.is_empty() {
        args.push("-m");
        args.push(&opts.whisper_model);
    }
    args.push("-f");
    args.push(&input_str);
    args.push("-otxt");
    args.push("-of");
    args.push(&prefix_str);
    run_external_command_checked(&opts.whisper_bin, &args, timeout, &timeout_hint)?;

    let out_txt = dir.path().join("out.txt");
    std::fs::read_to_string(&out_txt).map_err(|e| {
        format!("读取转写结果 {} 失败（{e}），whisper 可能未生成输出", out_txt.display())
    })
}

/// 异步版本的文本提取，通过 spawn_blocking 包装同步 I/O，避免阻塞 tokio 运行时。
/// 参数使用 `PathBuf` 和 `String`（owned），因为闭包需要 `'static` 生命周期。
pub async fn extract_text_async(file_path: PathBuf, mime_type: String) -> Result<String> {
    let path_display = file_path.display().to_string();
    tokio::task::spawn_blocking(move || extract_text(&file_path, &mime_type))
        .await
        .map_err(|e| {
            AxAgentError::execution_with_source(format!("文本提取任务失败: {}", path_display), e)
        })
        .and_then(|r| r)
}

/// Extract text from PDF using pdf-extract crate.
fn extract_pdf(file_path: &Path) -> Result<String> {
    let bytes = std::fs::read(file_path).map_err(|e| {
        AxAgentError::execution_with_source(
            format!("Failed to read PDF file: {}", file_path.display()),
            e,
        )
    })?;

    pdf_extract::extract_text_from_mem(&bytes)
        .map_err(|e| AxAgentError::execution_with_source("Failed to extract PDF text", e))
}

/// Extract text from DOCX by reading the internal XML.
/// DOCX files are ZIP archives containing word/document.xml.
fn extract_docx(file_path: &Path) -> Result<String> {
    let file = std::fs::File::open(file_path).map_err(|e| {
        AxAgentError::execution_with_source(
            format!("Failed to open DOCX file: {}", file_path.display()),
            e,
        )
    })?;

    let mut archive = zip::ZipArchive::new(file).map_err(|e| {
        AxAgentError::execution_with_source(
            format!("Failed to read DOCX as ZIP: {}", file_path.display()),
            e,
        )
    })?;

    let mut xml_content = String::new();
    if let Ok(mut entry) = archive.by_name("word/document.xml") {
        use std::io::Read;
        entry
            .read_to_string(&mut xml_content)
            .map_err(|e| AxAgentError::execution_with_source("Failed to read document.xml", e))?;
    } else {
        return Err(AxAgentError::Provider("DOCX: word/document.xml not found".into()));
    }

    // Simple XML text extraction: find all <w:t> tag contents
    Ok(extract_text_from_xml(&xml_content))
}

/// Simple XML text extraction — pulls text from <w:t> and <w:t xml:space="preserve"> tags.
fn extract_text_from_xml(xml: &str) -> String {
    let mut result = String::new();
    let mut in_paragraph = false;

    for part in xml.split("<w:p") {
        if in_paragraph && !result.is_empty() {
            result.push('\n');
        }
        in_paragraph = true;

        for segment in part.split("<w:t") {
            if let Some(text_start) = segment.find('>') {
                let after_tag = &segment[text_start + 1..];
                if let Some(end) = after_tag.find("</w:t>") {
                    result.push_str(&after_tag[..end]);
                }
            }
        }
    }

    result
}

/// 目录导入支持的扩展名白名单（不含点，全小写）。
/// 权威来源：knowledge.rs 的 `is_supported_knowledge_ext` 收口于此，避免双源。
/// 覆盖：文本 / 办公 / PDF / 表格 / 演示 / 邮件 / 笔记 / 配置 / 代码文件族。
pub const SUPPORTED_EXTENSIONS: &[&str] = &[
    // 纯文本与标记
    "txt", "md", "markdown", "csv", "html", "htm", "xml", "json", "ipynb", "yaml", "yml", "toml",
    // 邮件
    "eml", // 办公文档
    "pdf", "doc", "docx", "odt", "rtf", "epub", "xls", "xlsx", "ods", "pptx",
    // 代码文件族（走 text/plain）
    "rs", "py", "js", "jsx", "ts", "tsx", "go", "java", "c", "cc", "cpp", "cxx", "h", "hh", "hpp",
    "hxx", "cs", "kt", "kts", "swift", "rb", "php", "sql", "sh", "bash", "zsh", "ps1", "bat",
    "cmd", // 音视频（走 whisper 转写；非 WAV 需 ffmpeg 前置转码）
    "wav", "mp3", "ogg", "flac", "m4a", "aac", "opus", "aiff", "wma", "mp4", "mov", "webm", "mkv",
    "avi", "mpeg", "mpg", "3gp",
];

/// 判断扩展名是否属于目录导入支持的白名单（大小写不敏感）。
pub fn is_supported_ext(ext: &str) -> bool {
    let e = ext.to_ascii_lowercase();
    SUPPORTED_EXTENSIONS.contains(&e.as_str())
}

/// Determine the MIME type from a file extension.
pub fn mime_from_extension(path: &Path) -> &'static str {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
    match ext.to_ascii_lowercase().as_str() {
        "txt" => "text/plain",
        "md" | "markdown" => "text/markdown",
        "csv" => "text/csv",
        "html" | "htm" => "text/html",
        "xml" => "text/xml",
        "json" => "application/json",
        "ipynb" => "application/json",
        "yaml" | "yml" => "application/yaml",
        "toml" => "application/toml",
        "eml" => "message/rfc822",
        "pdf" => "application/pdf",
        "doc" => "application/msword",
        "docx" => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        "xls" => "application/vnd.ms-excel",
        "xlsx" => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        "ods" => "application/vnd.oasis.opendocument.spreadsheet",
        "odt" => "application/vnd.oasis.opendocument.text",
        "rtf" => "application/rtf",
        "epub" => "application/epub+zip",
        "pptx" => "application/vnd.openxmlformats-officedocument.presentationml.presentation",
        // 音视频
        "wav" => "audio/wav",
        "mp3" => "audio/mpeg",
        "ogg" => "audio/ogg",
        "flac" => "audio/flac",
        "m4a" => "audio/x-m4a",
        "aac" => "audio/aac",
        "opus" => "audio/opus",
        "aiff" => "audio/x-aiff",
        "wma" => "audio/x-ms-wma",
        "mp4" => "video/mp4",
        "mov" => "video/quicktime",
        "webm" => "video/webm",
        "mkv" => "video/x-matroska",
        "avi" => "video/x-msvideo",
        "mpeg" | "mpg" => "video/mpeg",
        "3gp" => "video/3gpp",
        _ => "text/plain",
    }
}

/// Extract text from PPTX (PowerPoint) by reading slide XML files.
fn extract_pptx(file_path: &Path) -> Result<String> {
    let file = std::fs::File::open(file_path)
        .map_err(|e| AxAgentError::Provider(format!("Failed to open PPTX file: {e}")))?;

    let mut archive = zip::ZipArchive::new(file)
        .map_err(|e| AxAgentError::Provider(format!("Failed to read PPTX as ZIP: {e}")))?;

    let mut result = String::new();
    let mut slide_index = 1;

    loop {
        let slide_path = format!("ppt/slides/slide{}.xml", slide_index);
        let mut xml_content = String::new();

        match archive.by_name(&slide_path) {
            Ok(mut entry) => {
                use std::io::Read;
                entry.read_to_string(&mut xml_content).map_err(|e| {
                    AxAgentError::Provider(format!("Failed to read {}: {e}", slide_path))
                })?;

                if !result.is_empty() {
                    result.push_str("\n\n");
                }
                result.push_str(&format!("=== Slide {} ===\n", slide_index));

                let slide_text = extract_text_from_pptx_xml(&xml_content);
                if !slide_text.is_empty() {
                    result.push_str(&slide_text);
                }
            },
            Err(_) => break,
        }

        slide_index += 1;
    }

    if result.is_empty() {
        return Err(AxAgentError::Provider("No slides found in PPTX file".into()));
    }

    Ok(result)
}

fn extract_text_from_pptx_xml(xml: &str) -> String {
    let mut result = String::new();
    let mut current_shape_text = String::new();

    for part in xml.split("<p:sp") {
        if !current_shape_text.is_empty() && !result.is_empty() {
            result.push('\n');
        }
        current_shape_text.clear();

        for segment in part.split("<a:t") {
            if let Some(text_start) = segment.find('>') {
                let after_tag = &segment[text_start + 1..];
                if let Some(end) = after_tag.find("</a:t>") {
                    let text = &after_tag[..end];
                    if !text.is_empty() {
                        current_shape_text.push_str(text);
                    }
                }
            }
        }

        if !current_shape_text.trim().is_empty() {
            if !result.is_empty() && !result.ends_with('\n') {
                result.push(' ');
            }
            result.push_str(current_shape_text.trim());
        }
    }

    result
}

// ── 新增格式解析器（阶段 1 目录导入强化）──────────────────

/// 旧版二进制 Word（.doc）—— 外部 `antiword` 命令提取文本。
/// 依赖系统安装 antiword；未安装/失败返回明确错误（沿用 OCR 的「环境缺依赖必须报错」原则）。
fn extract_doc(file_path: &Path) -> Result<String> {
    let path_str = file_path.to_string_lossy().to_string();
    match external_command_extract("antiword", &[&path_str], Duration::from_secs(60), "60 秒") {
        Ok(text) if !text.trim().is_empty() => Ok(text),
        Ok(_) => Err(AxAgentError::Provider(format!(
            "antiword 未提取到文本（可能为加密或损坏文档）: {}",
            file_path.display()
        ))),
        Err(e) => {
            Err(AxAgentError::Provider(format!("DOC 解析失败（{e}）: {}", file_path.display())))
        },
    }
}

/// XLS / XLSX / ODS 统一读取（calamine）。逐 sheet 输出「sheet 名 + 制表符分隔单元格」。
fn extract_spreadsheet(file_path: &Path) -> Result<String> {
    let mut workbook = calamine::open_workbook_auto(file_path).map_err(|e| {
        AxAgentError::Provider(format!("打开表格失败（{e}）: {}", file_path.display()))
    })?;
    let sheets = workbook.sheet_names().to_vec();
    if sheets.is_empty() {
        return Err(AxAgentError::Provider(format!("表格中没有工作表: {}", file_path.display())));
    }

    let mut result = String::new();
    for name in sheets {
        let range = workbook
            .worksheet_range(&name)
            .map_err(|e| AxAgentError::Provider(format!("读取工作表 {name} 失败（{e}）")))?;
        if !result.is_empty() {
            result.push_str("\n\n");
        }
        result.push_str(&format!("--- {name} ---\n"));
        for row in range.rows() {
            let cells: Vec<String> =
                row.iter().map(calamine_cell_text).filter(|s| !s.is_empty()).collect();
            if !cells.is_empty() {
                result.push_str(&cells.join("\t"));
                result.push('\n');
            }
        }
    }

    if result.trim().is_empty() {
        return Err(AxAgentError::Provider(format!("表格未提取到文本: {}", file_path.display())));
    }
    Ok(result)
}

/// calamine 单元格 → 文本（整数值不带小数点）。
fn calamine_cell_text(cell: &calamine::Data) -> String {
    match cell {
        calamine::Data::String(s) => s.clone(),
        calamine::Data::Float(f) => {
            if f.fract() == 0.0 && f.abs() < 1e15 {
                format!("{}", *f as i64)
            } else {
                f.to_string()
            }
        },
        calamine::Data::Int(i) => i.to_string(),
        calamine::Data::Bool(b) => b.to_string(),
        calamine::Data::DateTime(dt) => dt.to_string(),
        calamine::Data::DateTimeIso(s) => s.clone(),
        calamine::Data::DurationIso(s) => s.clone(),
        calamine::Data::Error(e) => e.to_string(),
        calamine::Data::Empty => String::new(),
    }
}

/// ODT —— zip 内 `content.xml`，抽取 `<text:p>` 与 `<text:h>` 段落文本。
fn extract_odt(file_path: &Path) -> Result<String> {
    let file = std::fs::File::open(file_path)
        .map_err(|e| AxAgentError::Provider(format!("Failed to open ODT file: {e}")))?;
    let mut archive = zip::ZipArchive::new(file)
        .map_err(|e| AxAgentError::Provider(format!("Failed to read ODT as ZIP: {e}")))?;

    use std::io::Read;
    let mut xml = String::new();
    {
        let mut entry = archive
            .by_name("content.xml")
            .map_err(|_| AxAgentError::Provider("ODT: content.xml not found".into()))?;
        entry
            .read_to_string(&mut xml)
            .map_err(|e| AxAgentError::Provider(format!("Failed to read content.xml: {e}")))?;
    }

    let mut result = String::new();
    for fragment in xml.split("<text:p").skip(1) {
        let text = xml_inner_text(fragment, "</text:p>");
        if !text.trim().is_empty() {
            if !result.is_empty() {
                result.push('\n');
            }
            result.push_str(text.trim());
        }
    }
    for fragment in xml.split("<text:h").skip(1) {
        let text = xml_inner_text(fragment, "</text:h>");
        if !text.trim().is_empty() {
            if !result.is_empty() {
                result.push('\n');
            }
            result.push_str(text.trim());
        }
    }

    if result.trim().is_empty() {
        return Err(AxAgentError::Provider(format!("ODT 未提取到文本: {}", file_path.display())));
    }
    Ok(result)
}

/// 轻量 RTF 文本剥离 —— 无外部 crate。
/// - 剥离花括号分组与控制字，保留文本
/// - `\par` / `\line` → 换行；`\uNNNN` → Unicode 字符；`\'xx` → Latin-1 字节近似
/// - 未知控制字静默丢弃（不影响可读文本主干）
fn extract_rtf(file_path: &Path) -> Result<String> {
    let bytes = std::fs::read(file_path)
        .map_err(|e| AxAgentError::Provider(format!("Failed to read RTF file: {e}")))?;
    let text = rtf_to_text(&bytes);
    if text.trim().is_empty() {
        return Err(AxAgentError::Provider(format!("RTF 未提取到文本: {}", file_path.display())));
    }
    Ok(text)
}

fn rtf_to_text(bytes: &[u8]) -> String {
    let mut out = String::new();
    let n = bytes.len();
    let mut i = 0;
    while i < n {
        match bytes[i] {
            b'\\' => {
                i += 1;
                if i >= n {
                    break;
                }
                let c = bytes[i];
                if c == b'\'' {
                    // \'xx —— Latin-1 字节近似
                    if i + 3 <= n {
                        let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or("");
                        if let Ok(v) = u8::from_str_radix(hex, 16) {
                            out.push(v as char);
                            i += 3;
                            continue;
                        }
                    }
                    i += 1;
                } else if c == b'u' {
                    // \uNNNN? —— Unicode 十进制，后随替换字符
                    let mut j = i + 1;
                    while j < n && bytes[j].is_ascii_digit() {
                        j += 1;
                    }
                    if let Ok(num) =
                        std::str::from_utf8(&bytes[i + 1..j]).unwrap_or("").parse::<u32>()
                        && let Some(ch) = char::from_u32(num)
                    {
                        out.push(ch);
                    }
                    i = j;
                    if i < n && bytes[i] == b'?' {
                        i += 1;
                    }
                } else if c.is_ascii_alphabetic() {
                    // 控制字：字母 + 可选参数数字，后跟空格分隔符
                    let mut j = i + 1;
                    while j < n
                        && (bytes[j].is_ascii_alphabetic()
                            || bytes[j].is_ascii_digit()
                            || bytes[j] == b'-')
                    {
                        j += 1;
                    }
                    let word = &bytes[i + 1..j];
                    if word == b"par" || word == b"line" {
                        out.push('\n');
                    }
                    if j < n && bytes[j] == b' ' {
                        j += 1;
                    }
                    i = j;
                } else {
                    // 控制符号
                    match c {
                        b'{' | b'}' | b'\\' => out.push(c as char),
                        b'~' => out.push(' '),
                        b'_' => out.push('-'),
                        _ => {},
                    }
                    i += 1;
                }
            },
            b'{' | b'}' => i += 1,
            b'\r' => i += 1,
            _ => {
                // 普通文本：按 UTF-8 收集
                let len = utf8_char_len(bytes[i]);
                if len > 0 && i + len <= n {
                    if let Ok(s) = std::str::from_utf8(&bytes[i..i + len]) {
                        out.push_str(s);
                    }
                    i += len;
                } else {
                    i += 1;
                }
            },
        }
    }
    out
}

/// EPUB —— zip 内按文件名顺序读取 content 文档（xhtml），剥离 HTML 标签取文本。
/// 简化：遍历 zip 内所有 .xhtml/.html/.htm 条目（跳过 toc.ncx 等辅助文件），
/// 不解析 OPF spine（大多数 EPUB 的条目名顺序即阅读顺序）。
fn extract_epub(file_path: &Path) -> Result<String> {
    let file = std::fs::File::open(file_path)
        .map_err(|e| AxAgentError::Provider(format!("Failed to open EPUB file: {e}")))?;
    let mut archive = zip::ZipArchive::new(file)
        .map_err(|e| AxAgentError::Provider(format!("Failed to read EPUB as ZIP: {e}")))?;

    use std::io::Read;
    let mut entries: Vec<String> = Vec::new();
    for idx in 0..archive.len() {
        let name = archive.by_index(idx).map(|e| e.name().to_string()).unwrap_or_default();
        if name.ends_with(".xhtml") || name.ends_with(".html") || name.ends_with(".htm") {
            entries.push(name);
        }
    }
    entries.sort();

    let mut result = String::new();
    for name in entries {
        let mut html = String::new();
        {
            let mut entry = archive
                .by_name(&name)
                .map_err(|e| AxAgentError::Provider(format!("Failed to read {name}: {e}")))?;
            entry
                .read_to_string(&mut html)
                .map_err(|e| AxAgentError::Provider(format!("Failed to read {name}: {e}")))?;
        }
        let text = strip_html(&html);
        if !text.trim().is_empty() {
            if !result.is_empty() {
                result.push('\n');
            }
            result.push_str(text.trim());
        }
    }

    if result.trim().is_empty() {
        return Err(AxAgentError::Provider(format!("EPUB 未提取到文本: {}", file_path.display())));
    }
    Ok(result)
}

/// 简单 HTML 标签剥离：移除 `<script>`/`<style>` 块与标签，保留文本并归一化空白。
fn strip_html(html: &str) -> String {
    let bytes = html.as_bytes();
    let n = bytes.len();
    let mut out = String::new();
    let mut i = 0;
    let mut skip_block = false;
    while i < n {
        if bytes[i] == b'<' {
            let mut j = i + 1;
            while j < n && bytes[j] != b'>' {
                j += 1;
            }
            let tag = String::from_utf8_lossy(&bytes[i + 1..j.min(n)]).to_ascii_lowercase();
            let is_close = tag.starts_with('/');
            let name = if is_close {
                tag[1..].trim_start()
            } else {
                &tag
            };
            if name.starts_with("script") || name.starts_with("style") {
                skip_block = !is_close;
            } else if is_block_breaker(name) && !out.ends_with('\n') {
                out.push('\n');
            }
            i = if j < n { j + 1 } else { n };
        } else if skip_block {
            i += 1;
        } else {
            let len = utf8_char_len(bytes[i]);
            if len > 0 && i + len <= n {
                if let Ok(s) = std::str::from_utf8(&bytes[i..i + len]) {
                    out.push_str(s);
                }
                i += len;
            } else {
                i += 1;
            }
        }
    }
    // 归一化：压缩连续空白与超长空行
    let mut cleaned = String::new();
    let mut last_space = false;
    let mut newline_count = 0;
    for ch in out.chars() {
        if ch == '\n' {
            newline_count += 1;
            if newline_count <= 2 {
                cleaned.push('\n');
            }
            last_space = true;
        } else if ch.is_whitespace() {
            if !last_space {
                cleaned.push(' ');
            }
            last_space = true;
        } else {
            cleaned.push(ch);
            last_space = false;
            newline_count = 0;
        }
    }
    cleaned
}

fn is_block_breaker(tag: &str) -> bool {
    matches!(
        tag.trim_start(),
        "p" | "div" | "br" | "li" | "tr" | "table" | "h1" | "h2" | "h3" | "h4"
    )
}

/// Jupyter Notebook —— 解析 cells，markdown 直接输出、code 用代码块包裹。
fn extract_ipynb(file_path: &Path) -> Result<String> {
    let content = std::fs::read_to_string(file_path)
        .map_err(|e| AxAgentError::Provider(format!("Failed to read ipynb: {e}")))?;
    let root: serde_json::Value = serde_json::from_str(&content)
        .map_err(|e| AxAgentError::Provider(format!("ipynb JSON 解析失败: {e}")))?;

    let mut result = String::new();
    if let Some(cells) = root.get("cells").and_then(|c| c.as_array()) {
        for cell in cells {
            let cell_type = cell.get("cell_type").and_then(|t| t.as_str()).unwrap_or("");
            let source = cell
                .get("source")
                .and_then(|s| s.as_array())
                .map(|arr| arr.iter().filter_map(|s| s.as_str()).collect::<Vec<_>>().join(""))
                .unwrap_or_default();
            if source.trim().is_empty() {
                continue;
            }
            if !result.is_empty() {
                result.push('\n');
            }
            if cell_type == "markdown" {
                result.push_str(&source);
            } else {
                result.push_str(&format!("```\n{}\n```", source.trim_end()));
            }
        }
    }

    if result.trim().is_empty() {
        return Err(AxAgentError::Provider(format!(
            "ipynb 未包含可提取的 cell: {}",
            file_path.display()
        )));
    }
    Ok(result)
}

/// YAML —— 结构化为可读文本（key: value，嵌套缩进）。
fn extract_yaml(file_path: &Path) -> Result<String> {
    let content = std::fs::read_to_string(file_path)
        .map_err(|e| AxAgentError::Provider(format!("Failed to read YAML: {e}")))?;
    let value: serde_yaml_ng::Value = serde_yaml_ng::from_str(&content)
        .map_err(|e| AxAgentError::Provider(format!("YAML 解析失败: {e}")))?;
    let mut out = String::new();
    yaml_walk(&value, 0, &mut out);
    if out.trim().is_empty() {
        return Err(AxAgentError::Provider(format!("YAML 未提取到文本: {}", file_path.display())));
    }
    Ok(out)
}

fn yaml_walk(value: &serde_yaml_ng::Value, depth: usize, out: &mut String) {
    let indent = "  ".repeat(depth);
    match value {
        serde_yaml_ng::Value::Mapping(map) => {
            for (k, v) in map {
                let key = k.as_str().unwrap_or("");
                if !out.is_empty() {
                    out.push('\n');
                }
                match v {
                    serde_yaml_ng::Value::Mapping(_) | serde_yaml_ng::Value::Sequence(_) => {
                        out.push_str(&format!("{indent}{key}:"));
                        yaml_walk(v, depth + 1, out);
                    },
                    serde_yaml_ng::Value::Null => {
                        out.push_str(&format!("{indent}{key}: null"));
                    },
                    other => {
                        out.push_str(&format!("{indent}{key}: {}", yaml_scalar_text(other)));
                    },
                }
            }
        },
        serde_yaml_ng::Value::Sequence(seq) => {
            for item in seq {
                if !out.is_empty() {
                    out.push('\n');
                }
                out.push_str(&format!("{indent}- {}", yaml_scalar_text(item)));
            }
        },
        other => out.push_str(&serde_yaml_ng::to_string(other).unwrap_or_default()),
    }
}

fn yaml_scalar_text(v: &serde_yaml_ng::Value) -> String {
    match v {
        serde_yaml_ng::Value::Null => "null".to_string(),
        serde_yaml_ng::Value::String(s) => s.clone(),
        serde_yaml_ng::Value::Number(n) => n.to_string(),
        serde_yaml_ng::Value::Bool(b) => b.to_string(),
        _ => serde_yaml_ng::to_string(v).unwrap_or_default(),
    }
}

/// TOML —— 结构化为可读文本。
fn extract_toml(file_path: &Path) -> Result<String> {
    let content = std::fs::read_to_string(file_path)
        .map_err(|e| AxAgentError::Provider(format!("Failed to read TOML: {e}")))?;
    let value: toml::Value = toml::from_str(&content)
        .map_err(|e| AxAgentError::Provider(format!("TOML 解析失败: {e}")))?;
    let mut out = String::new();
    toml_walk(&value, 0, &mut out);
    if out.trim().is_empty() {
        return Err(AxAgentError::Provider(format!("TOML 未提取到文本: {}", file_path.display())));
    }
    Ok(out)
}

fn toml_walk(value: &toml::Value, depth: usize, out: &mut String) {
    let indent = "  ".repeat(depth);
    match value {
        toml::Value::Table(t) => {
            for (k, v) in t {
                if !out.is_empty() {
                    out.push('\n');
                }
                match v {
                    toml::Value::Table(_) => {
                        out.push_str(&format!("{indent}[{k}]"));
                        toml_walk(v, depth + 1, out);
                    },
                    toml::Value::Array(arr) => {
                        out.push_str(&format!("{indent}{k}:"));
                        for item in arr {
                            out.push('\n');
                            out.push_str(&format!("{indent}  - {}", toml_scalar_text(item)));
                        }
                    },
                    other => {
                        out.push_str(&format!("{indent}{k} = {}", toml_scalar_text(other)));
                    },
                }
            }
        },
        other => out.push_str(&toml_scalar_text(other)),
    }
}

fn toml_scalar_text(v: &toml::Value) -> String {
    match v {
        toml::Value::String(s) => s.clone(),
        toml::Value::Integer(i) => i.to_string(),
        toml::Value::Float(f) => f.to_string(),
        toml::Value::Boolean(b) => b.to_string(),
        toml::Value::Datetime(d) => d.to_string(),
        _ => v.to_string(),
    }
}

/// 邮件（.eml）—— mailparse 解析 RFC822，取头（Subject/From/To/Date）+ text 正文。
fn extract_eml(file_path: &Path) -> Result<String> {
    let raw = std::fs::read(file_path)
        .map_err(|e| AxAgentError::Provider(format!("Failed to read EML: {e}")))?;
    let parsed = mailparse::parse_mail(&raw)
        .map_err(|e| AxAgentError::Provider(format!("EML 解析失败: {e}")))?;

    let mut headers: Vec<String> = Vec::new();
    use mailparse::MailHeaderMap;
    for key in ["Subject", "From", "To", "Date"] {
        if let Some(value) = parsed.headers.get_first_value(key) {
            headers.push(format!("{key}: {value}"));
        }
    }
    let body = eml_text_part(&parsed);

    let mut out = headers.join("\n");
    if !body.trim().is_empty() {
        if !out.is_empty() {
            out.push_str("\n\n");
        }
        out.push_str(body.trim());
    }
    if out.trim().is_empty() {
        return Err(AxAgentError::Provider(format!("EML 未提取到文本: {}", file_path.display())));
    }
    Ok(out)
}

/// 递归取 multipart 中最底层的 text/plain（无则 text/html 剥离标签）。
fn eml_text_part(parsed: &mailparse::ParsedMail) -> String {
    let mut texts = Vec::new();
    if parsed.subparts.is_empty() {
        match parsed.ctype.mimetype.as_str() {
            "text/plain" => {
                if let Ok(body) = parsed.get_body() {
                    texts.push(body);
                }
            },
            "text/html" => {
                if let Ok(body) = parsed.get_body() {
                    texts.push(strip_html(&body));
                }
            },
            _ => {},
        }
    } else {
        for sub in &parsed.subparts {
            texts.push(eml_text_part(sub));
        }
    }
    texts.join("\n")
}

/// 从 XML 片段抽取 `>` 与 `</closing>` 之间的纯文本（剥离内部子标签）。
fn xml_inner_text(fragment: &str, closing_tag: &str) -> String {
    let Some(start) = fragment.find('>') else {
        return String::new();
    };
    let after = &fragment[start + 1..];
    let Some(end) = after.find(closing_tag) else {
        return String::new();
    };
    let inner = &after[..end];
    let mut out = String::new();
    let mut in_tag = false;
    for ch in inner.chars() {
        match ch {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => out.push(ch),
            _ => {},
        }
    }
    out
}

/// UTF-8 首字节 → 该字符总字节数。
fn utf8_char_len(b: u8) -> usize {
    match b {
        0x00..=0x7F => 1,
        0xC0..=0xDF => 2,
        0xE0..=0xEF => 3,
        0xF0..=0xF7 => 4,
        _ => 1,
    }
}

// ── trait 默认实现 ──
pub mod parser_impl;

// ── 单测：只覆盖不依赖外部工具（tesseract/pdftoppm/whisper/ffmpeg）的纯逻辑分支 ──
#[cfg(test)]
mod tests {
    use super::*;

    /// 外部命令不存在 → Err 且提示未安装（不依赖本机安装任何外部工具）
    #[test]
    fn external_command_checked_not_found() {
        let err = run_external_command_checked(
            "definitely-not-a-real-command-axagent-xyz",
            &[],
            Duration::from_secs(3),
            "3 秒",
        )
        .unwrap_err();
        assert!(err.contains("未安装"), "err = {err}");
    }

    /// 取 stdout 版本同样在命令不存在时返回 Err
    #[test]
    fn external_command_extract_not_found() {
        let err = external_command_extract(
            "definitely-not-a-real-command-axagent-xyz",
            &[],
            Duration::from_secs(3),
            "3 秒",
        )
        .unwrap_err();
        assert!(err.contains("未安装"), "err = {err}");
    }

    /// 未注入配置时的默认值（OCR 语言 / whisper 命令）
    #[test]
    fn parser_options_defaults() {
        let opts = parser_options();
        assert_eq!(opts.ocr_lang, "eng+chi_sim");
        assert_eq!(opts.whisper_bin, "whisper-cli");
        assert_eq!(opts.external_timeout_secs, 120);
    }

    /// 音视频扩展名已进白名单并映射到对应 MIME
    #[test]
    fn media_extensions_supported() {
        for ext in ["mp3", "wav", "mp4", "mkv", "m4a"] {
            assert!(is_supported_ext(ext), "{ext} 应在白名单");
        }
        assert_eq!(mime_from_extension(std::path::Path::new("a.mp4")), "video/mp4");
        assert_eq!(mime_from_extension(std::path::Path::new("a.wav")), "audio/wav");
    }
}
