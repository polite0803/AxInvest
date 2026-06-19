// SPDX-License-Identifier: AGPL-3.0-only

use std::process::Command as StdCommand;

/// 创建不弹出控制台窗口的进程命令（Windows 专用）
#[cfg(windows)]
pub fn cmd(program: &str) -> StdCommand {
    use std::os::windows::process::CommandExt;
    let mut c = StdCommand::new(program);
    c.creation_flags(0x08000000); // CREATE_NO_WINDOW
    c
}

#[cfg(not(windows))]
pub fn cmd(program: &str) -> StdCommand {
    StdCommand::new(program)
}

pub use axagent_harness::util_fns::{current_rfc3339, gen_id, now_ts};

const OUTPUT_LANGUAGE_TAG: &str = "<output-language>";

pub fn language_code_to_name(code: &str) -> &str {
    match code {
        "zh-CN" | "zh-TW" | "zh-Hans" | "zh-Hant" => "Chinese",
        "en-US" | "en-GB" | "en" => "English",
        "ja-JP" | "ja" => "Japanese",
        "ko-KR" | "ko" => "Korean",
        "ru" | "ru-RU" => "Russian",
        "fr" | "fr-FR" => "French",
        "de" | "de-DE" => "German",
        "es" | "es-ES" => "Spanish",
        "pt" | "pt-BR" | "pt-PT" => "Portuguese",
        "it" | "it-IT" => "Italian",
        "ar" | "ar-SA" => "Arabic",
        "th" | "th-TH" => "Thai",
        "vi" | "vi-VN" => "Vietnamese",
        "id" | "id-ID" => "Indonesian",
        other => other,
    }
}

pub fn build_output_language_directive(language_code: &str) -> String {
    let lang_name = language_code_to_name(language_code);
    let thinking_emphasis = String::new();
    format!(
        "{tag}\nIMPORTANT: You MUST respond entirely in {lang_name}. All your output, including explanations, summaries, and any text directed to the user, must be written in {lang_name}. This is a strict requirement — do not switch to any other language unless the user explicitly asks you to.{thinking_emphasis}\n</output-language>",
        tag = OUTPUT_LANGUAGE_TAG,
        lang_name = lang_name,
    )
}

pub fn has_output_language_directive(content: &str) -> bool {
    content.contains(OUTPUT_LANGUAGE_TAG)
}

pub fn append_language_directive(system_prompt: &str, language_code: &str) -> String {
    if language_code.is_empty() || has_output_language_directive(system_prompt) {
        return system_prompt.to_string();
    }
    format!("{}\n\n{}", system_prompt, build_output_language_directive(language_code))
}

/// 从 LLM 响应中提取 JSON 内容。
///
/// 处理 LLM 可能在 JSON 外包裹 markdown 代码块或额外文本的情况。
/// 按优先级尝试：\`\`\`json 围栏 → { 起始的裸 JSON → 原始文本。
pub fn extract_json_from_llm_response(text: &str) -> &str {
    let trimmed = text.trim();

    // 尝试从 ```json 围栏中提取
    if let Some(start) = trimmed.find("```json") {
        let inner = &trimmed[start + 7..];
        if let Some(end) = find_fence_end(inner) {
            return trim_after_json(inner[..end].trim());
        }
        return trim_after_json(inner.trim());
    }

    // 尝试从 ``` 围栏中提取（支持 json、tool_json、tool 等任意语言标签）
    if let Some(start) = trimmed.find("```") {
        let inner = &trimmed[start + 3..];
        // 跳过语言标签行（如 tool_json、json、tool 等），直到遇到换行后的实际内容
        let inner = if let Some(newline) = inner.find('\n') {
            &inner[newline + 1..]
        } else {
            inner
        };
        if let Some(end) = find_fence_end(inner) {
            return trim_after_json(inner[..end].trim());
        }
        return trim_after_json(inner.trim());
    }

    trimmed
}

/// 在字符串中找到关闭 fence 的位置。
/// 找最后一个 ` 之前的连续反引号序列的起始位置（无论 3/4/5/... 个反引号）。
/// 如果 JSON 内容不含反引号，这就是正确的 fence 边界。
fn find_fence_end(s: &str) -> Option<usize> {
    let bytes = s.as_bytes();
    let len = bytes.len();
    if len < 3 {
        return None;
    }
    // 从尾部往前找连续反引号序列：找连续 3+ 反引号中最靠左的位置
    let mut backtick_run_start = None;
    let mut i = 0;
    while i < len {
        if bytes[i] == b'`' {
            let run_start = i;
            while i < len && bytes[i] == b'`' {
                i += 1;
            }
            let run_len = i - run_start;
            if run_len >= 3 {
                backtick_run_start = Some(run_start);
            }
        } else {
            i += 1;
        }
    }
    // 返回最后一个 3+ 反引号序列的起始位置
    backtick_run_start
}

/// 从尾部找到最后一个完整闭合的 JSON 对象/数组，截掉后面的垃圾文本。
/// LLM 经常在 ````json``` 围栏内的 JSON 后面追加自然语言评论。
fn trim_after_json(s: &str) -> &str {
    let bytes = s.as_bytes();
    let len = bytes.len();
    if len == 0 {
        return s;
    }

    // 正向扫描: 记录每个 depth=0 的位置（即每个顶层闭合点）
    let mut depth: i32 = 0;
    let mut in_string = false;
    let mut escaped = false;
    let mut last_complete_end = 0; // 最后一个 depth 回到 0 的位置

    for i in 0..len {
        let b = bytes[i];
        if escaped {
            escaped = false;
            continue;
        }
        if b == b'\\' && in_string {
            escaped = true;
            continue;
        }
        if b == b'"' {
            in_string = !in_string;
            continue;
        }
        if !in_string {
            match b {
                b'{' | b'[' => depth += 1,
                b'}' | b']' => {
                    depth -= 1;
                    if depth == 0 {
                        last_complete_end = i; // 一个完整对象/数组在此闭合
                    }
                },
                _ => {},
            }
        }
    }

    // 如果找到了完整的顶层闭合，截断到那里
    // 注意：last_complete_end 的初始值是 0，但我们需要确保真的有找到一个
    if last_complete_end > 0 {
        &s[..=last_complete_end]
    } else {
        s
    }
}
