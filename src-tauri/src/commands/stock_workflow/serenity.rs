use super::decision::{ExecutionParams, load_and_inject_template, parse_asof_param};
use crate::AppState;
use crate::commands::error::ErrorResponse;
use crate::commands::error_code::stock_workflow as wf_err;
use axagent_agent_macro::agent_command;
use axagent_astock_data::AStockClient;
use axagent_astock_data::as_of::{self};
use axagent_harness::DatabaseConnection;
use axagent_harness::IpcEventName;
use axagent_harness::response_normalizer::ResponseNormalizer;
use axagent_harness::types::{ChatResponse, ContentBlock};
use axagent_rt_workflow::work_engine::{ProgressCallback, RunOptions, StepProgressEvent};
use axagent_runtime_core::DefaultResponseNormalizer;
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QueryOrder};
use std::sync::Arc;
use tauri::{Emitter, State};

/// 趋势智选两链的模板 id 白名单（第 0 项为默认 = 原链）。
/// 与种子侧对应：`serenity-screening`（`seed_serenity.rs`）/
/// `serenity-screening-fast`（`seed_serenity_fast.rs`）。
///
/// `pub(crate)`：时间旅行批次复用默认原链（与每日 trend-screening cron 同模板）。
pub(crate) const SERENITY_TEMPLATE_IDS: [&str; 2] =
    ["serenity-screening", "serenity-screening-fast"];

/// 从 Agent 节点输出中提取结构化 JSON。
///
/// 优先顺序：
///   1) 顶层 `params` 字段
///   2) 顶层 `output` / `result` / `data` / `candidates` / `trends` 字段
///      2.5) 顶层 `report` 字段的**解包**（含双重编码的 JSON 字符串，见函数内注释）
///      3) 顶层 `content` 字符串：直接用 `axagent_kit::utils::extract_json_from_llm_response`
///      解析（不经过 ResponseNormalizer——它针对工具调用场景，会将 ````json` 块
///      误识别为 ToolUse）
///   4) 原始包装对象（兜底）
pub(crate) async fn extract_agent_output(raw: serde_json::Value) -> serde_json::Value {
    let obj = match raw.as_object() {
        Some(o) => o,
        None => return raw,
    };
    // 1) 顶层 params
    if let Some(params) = obj.get("params") {
        return params.clone();
    }
    // 2) 顶层常见容器字段
    for key in ["output", "result", "data", "candidates", "trends"] {
        if let Some(v) = obj.get(key) {
            return v.clone();
        }
    }
    // 2.5) `report` 包装解包。
    //
    // 部分专家 prompt（如 reflection.md）要求 LLM 把结构化结果整体放进 `report` 字段，
    // 而落库时该字段常是**已被 JSON 序列化过的字符串**（双重编码）：
    //   {"report": "{\"verdict\": \"partial\", ...}"}
    // 旧实现没有 `report` 分支 ⇒ 落到 4) 兜底原样返回包装对象 ⇒ 消费侧
    // `json.get("verdict")` 永远取不到（对象里只有 `report` 这一个键）。
    //
    // 实测依据：`stock_reflections` 11 条 completed 中 10 条为该形态，
    // 以致 verdict / lesson_summary / alpha_cited 全为 NULL、下游 was_correct 被压成 0
    // （详见 PLAN-analysis-data-repair.md 缺陷 D1）。
    //
    // 判据：只在「内层确实是对象/数组」时解包 —— 纯自然语言的 report 解析失败即不命中，
    // 流程继续走 3)，不会吞掉原有兜底行为。
    if let Some(report) = obj.get("report") {
        if report.is_object() || report.is_array() {
            return report.clone();
        }
        if let Some(inner) = report
            .as_str()
            .and_then(|s| serde_json::from_str::<serde_json::Value>(s).ok())
            .filter(|v| v.is_object() || v.is_array())
        {
            return inner;
        }
    }
    // 3) 直接从 content 提取 JSON：找到第一个 { 或 [，找匹配闭合，解析。
    //    不依赖 extract_json_from_llm_response 的 fence 剥离（在复杂嵌套场景可能失效）。
    if let Some(content) = obj.get("content").and_then(|c| c.as_str()) {
        let candidate = axagent_kit::utils::extract_json_from_llm_response(content);
        // 诊断：打印 candidate 前后各 200 字符
        let preview: String = candidate.chars().take(200).collect();
        let tail: String =
            candidate.chars().rev().take(200).collect::<String>().chars().rev().collect();
        tracing::info!("[serenity] 提取文本 前200: {} / 后200: {}", preview, tail);
        // A: 精确解析（fence 剥离后的文本）
        if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&candidate) {
            if parsed.is_object() || parsed.is_array() {
                // 拆包 tool_json 格式: {"name": "...", "arguments": {...}} → arguments
                // 有些 LLM 用 "input" 代替 "arguments"
                if let Some(args) = parsed
                    .as_object()
                    .and_then(|o| o.get("arguments"))
                    .or_else(|| parsed.as_object().and_then(|o| o.get("input")))
                {
                    return args.clone();
                }
                return parsed;
            }
        }
        // B: 裸括号提取 candidates/trends 数组（免疫未转义引号）
        if let Some(parsed) = extract_named_arrays(&candidate) {
            return parsed;
        }
        if let Some(parsed) = extract_named_arrays(content) {
            return parsed;
        }
        // C: 修复后重试
        let repaired = repair_json(&candidate);
        if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&repaired) {
            if parsed.is_object() || parsed.is_array() {
                return parsed;
            }
        }
        // D: extract_outer_json（多起点 + in_string 追踪）
        if let Some(parsed) = extract_outer_json(content) {
            return parsed;
        }
        // E: 检测 LLM 自然语言拒绝（短文本非 JSON），防御性降级
        let content_len = content.chars().count();
        if content_len < 30 {
            tracing::warn!(
                "[serenity] LLM 内容为短自然语言（长度={}），返回空值防御性降级: {}",
                content_len,
                content.chars().take(50).collect::<String>()
            );
            return serde_json::Value::Null;
        }
        let head: String = content.chars().take(300).collect();
        let tail_start = content.chars().count().saturating_sub(200);
        let tail: String = content.chars().skip(tail_start).collect();
        let c_head: String = candidate.chars().take(1000).collect();
        let c_tail: String =
            candidate.chars().rev().take(200).collect::<String>().chars().rev().collect();
        tracing::warn!(
            "[serenity] content JSON 提取失败，总长度 {}, 前300: {} / 后200: {}",
            content.chars().count(),
            head,
            tail
        );
        tracing::warn!("[serenity] 预处理文本 前1000: {} / 后200: {}", c_head, c_tail);
    }
    // 4) 兜底
    raw
}

/// 通过 `ResponseNormalizer` 把 `content` 字符串规范化为 IR 块，再从 IR 中
/// 提取结构化 JSON。优先取 `ContentBlock::ToolUse.input`（通常是 JSON 串），
/// 文本块拼接后走 `axagent_kit::utils::extract_json_from_llm_response` 兜底。
///
/// 注意：`extract_agent_output` 不再调用此函数（改用 `extract_json_from_llm_response` 直接提取）。
/// 此函数保留供测试和未来工具调用场景复用（当前唯一调用方在 mod tests）。
#[cfg_attr(not(test), allow(dead_code))]
async fn extract_via_normalizer(content: &str) -> Option<serde_json::Value> {
    if content.trim().is_empty() {
        return None;
    }
    let response = ChatResponse {
        id: String::new(),
        model: String::new(),
        content: content.to_string(),
        thinking: None,
        usage: Default::default(),
        tool_calls: None,
    };
    let normalizer = DefaultResponseNormalizer;
    let blocks: Vec<ContentBlock> = normalizer.normalize(&response).await;

    // 优先：ToolUse 块的 input（项目里工具参数就是 JSON 串）
    for block in &blocks {
        if let ContentBlock::ToolUse { input, .. } = block
            && let Some(parsed) = parse_loose_json(input)
        {
            return Some(parsed);
        }
    }
    // 兜底：拼接所有 Text 块，用项目统一的 LLM JSON 提取函数
    let joined: String = blocks
        .iter()
        .filter_map(|b| match b {
            ContentBlock::Text { text } => Some(text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n");
    if !joined.trim().is_empty() {
        let candidate = axagent_kit::utils::extract_json_from_llm_response(&joined);
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(candidate) {
            return Some(v);
        }
    }
    None
}

/// 轻量 JSON 修复：处理 LLM 偶发的括号不匹配和引号未闭合。
///
/// 修复层（按执行顺序）：
/// 0. **内部引号转义** — 字符串值内部未转义的 ASCII `"`（如 `"凯盛"科技""`）
///    会让 serde_json 把字符串提前截断、后续 token 被误认为 key，
///    表现为 "expected `:`" 类解析错误。启发式：in_string 时遇到 `"`，
///    看其后第一个非空白字符——是 `,` `}` `]` `:` `"` 或 EOF 视为合法闭合，
///    其他（如中文字符）视为内部引号，转义为 `\"`
/// 1. **括号平衡** — 跳过字符串内部，统计 `{`/`[` vs `}`/`]`，补/删尾部括号
/// 2. **引号闭合** — 奇数个未转义 `"` 时末尾补一个
///
/// 对合法 JSON 零开销（不改变原文）；只在 `serde_json::from_str` 已失败后调用。
fn repair_json(s: &str) -> String {
    // 第 0 层必须先执行：后续括号统计依赖引号状态追踪，
    // 未转义的内部引号会让追踪提前脱轨，导致补括号位置全错。
    let mut result = escape_inner_quotes(s);

    // LLM 高频手滑："nulll"→"null"
    result = result.replace("nulll", "null");

    // LLM 尾逗号：,"→"、,}→}、,]→]
    // 只在可能有尾逗号的上下文中处理（简单字符串替换，低风险）
    result = result.replace(",]", "]");
    result = result.replace(",}", "}");

    let bytes = result.as_bytes();
    let len = bytes.len();
    if len == 0 {
        return result;
    }

    // 第一遍：统计括号和引号，跳过字符串内部
    let mut open_curly = 0i32;
    let mut open_bracket = 0i32;
    let mut in_string = false;

    let mut i = 0;
    while i < len {
        let b = bytes[i];
        match in_string {
            false => match b {
                b'{' => open_curly += 1,
                b'}' => open_curly -= 1,
                b'[' => open_bracket += 1,
                b']' => open_bracket -= 1,
                b'"' => in_string = true,
                _ => {},
            },
            true => {
                // 在字符串内部：只关心 \" 和 字符串结束 "
                if b == b'\\' {
                    i += 1; // 跳过下一个字符（转义序列）
                } else if b == b'"' {
                    in_string = false; // 字符串结束
                }
            },
        }
        i += 1;
    }

    // 第二遍：从尾部修复 — 只处理末尾多余的闭合括号
    // 复用第一遍已经过 nulll→null 修复的 result，不重新从 s 构建
    // (这行是故意 blank 的以使用前面的 result 变量)

    // 先处理括号不平衡：补缺失的闭合括号
    let needs_curly = open_curly.max(0) as usize;
    let needs_bracket = open_bracket.max(0) as usize;

    // 如果有缺失闭合，在尾部补上
    for _ in 0..needs_curly {
        result.push('}');
    }
    for _ in 0..needs_bracket {
        result.push(']');
    }

    // 引号修复：如果正在字符串中（奇数个引号），末尾补 "
    if in_string {
        result.push('"');
    }

    // 处理尾部多余闭合（open 为负数 → 多了闭合括号）
    // S-H3 修复：旧逻辑用 rposition 全局找最后一个 }，可能删除合法的闭合括号。
    // 新逻辑：只从字符串末尾向前删除连续的 } 或 ]，避免破坏合法 JSON 结构。
    let mut extra_curly = (-open_curly).max(0) as usize;
    while extra_curly > 0 {
        let bytes = result.as_bytes();
        let last = *bytes.last().unwrap_or(&0);
        // 只删除末尾的 }（跳过空白）
        if last == b'}' {
            result.pop();
            extra_curly -= 1;
        } else if last == b' ' || last == b'\n' || last == b'\r' || last == b'\t' {
            result.pop();
        } else {
            break;
        }
    }
    let mut extra_bracket = (-open_bracket).max(0) as usize;
    while extra_bracket > 0 {
        let bytes = result.as_bytes();
        let last = *bytes.last().unwrap_or(&0);
        if last == b']' {
            result.pop();
            extra_bracket -= 1;
        } else if last == b' ' || last == b'\n' || last == b'\r' || last == b'\t' {
            result.pop();
        } else {
            break;
        }
    }

    result
}

/// 修复字符串值内部的未转义 ASCII 双引号（repair_json 第 0 层）。
///
/// 从第一个 `{` 或 `[` 开始处理（前导的非 JSON 文本原样保留，避免
/// markdown 叙述文字里的成对引号被误当作字符串起点导致级联错乱）。
/// 合法 JSON 中已转义的 `\"` 与结构引号不受影响（零改动）。
fn escape_inner_quotes(s: &str) -> String {
    let Some(start) = s.find(['{', '[']) else {
        return s.to_string();
    };
    let mut out = String::with_capacity(s.len() + 16);
    out.push_str(&s[..start]);

    let mut in_string = false;
    let mut escaped = false;
    let mut chars = s[start..].chars().peekable();
    while let Some(c) = chars.next() {
        if !in_string {
            if c == '"' {
                in_string = true;
            }
            out.push(c);
            continue;
        }
        // in_string
        if escaped {
            escaped = false;
            out.push(c);
            continue;
        }
        match c {
            '\\' => {
                escaped = true;
                out.push(c);
            },
            '"' => {
                // 向后看第一个非空白字符，判断是合法闭合还是内部引号
                let mut ws_buf = Vec::new();
                let mut next = None;
                while let Some(&nc) = chars.peek() {
                    if nc.is_whitespace() {
                        ws_buf.push(nc);
                        chars.next();
                    } else {
                        next = Some(nc);
                        break;
                    }
                }
                // 合法闭合引号后面只会是 `,` `}` `]`（值/键结尾）、
                // `:`（键结尾，状态已脱轨时的自愈路径）、`"`（缺逗号场景，
                // 无法与内部引号区分，保守按闭合处理）或 EOF（截断场景）
                let is_close = matches!(
                    next,
                    None | Some(',') | Some('}') | Some(']') | Some(':') | Some('"')
                );
                if is_close {
                    in_string = false;
                    out.push('"');
                } else {
                    // 内部引号：转义，字符串继续
                    out.push_str("\\\"");
                }
                for w in ws_buf {
                    out.push(w);
                }
            },
            _ => out.push(c),
        }
    }
    out
}

/// 提取 serde_json 解析失败位置附近的诊断窗口（±`width` 字节），
/// 用于在日志中直接看到损坏点原文，替代"只知道报错不知道内容"的黑盒。
fn parse_error_window(text: &str, err: &serde_json::Error, width: usize) -> String {
    let line = err.line().saturating_sub(1);
    let col = err.column().saturating_sub(1);
    // 定位失败行起始字节偏移
    let mut offset = 0usize;
    for (i, l) in text.split('\n').enumerate() {
        if i >= line {
            break;
        }
        offset += l.len() + 1; // +1 为换行符本身
    }
    offset = offset.saturating_add(col).min(text.len());
    let start = text.floor_char_boundary(offset.saturating_sub(width));
    let end = text.ceil_char_boundary((offset + width).min(text.len()));
    format!("[…{}…]", &text[start..end])
}

/// 用裸括号追踪从文本中提取指定 key 的 JSON 数组（容忍引号错乱）。
///
/// LLM 常在字符串值中使用未转义双引号（如：他说"这是关键"），
/// 导致 `serde_json` 全量解析失败。此函数绕过引号状态追踪，
/// 直接匹配 `"key": [` 找到数组起始，然后裸计 `[`/`]` 深度找到闭合，
/// 只对这一小段 `[...]` 调用 `serde_json::from_str`。
///
/// 返回 `{"candidates": [...], "trends": [...]}`（只含成功解析的 key）。
fn extract_named_arrays(text: &str) -> Option<serde_json::Value> {
    let keys = ["candidates", "trends"];
    let mut result = serde_json::Map::new();

    for key in &keys {
        let pattern = format!("\"{}\":", key);
        // 找所有匹配位置（可能有多个同名 key，取最后一个）
        let mut pos = 0;
        let mut last_match = None;
        while let Some(mut p) = text[pos..].find(&pattern) {
            p += pos;
            last_match = Some(p + pattern.len());
            pos = p + 1;
        }
        let Some(after_key) = last_match else { continue };

        // 在 after_key.. 中找第一个 [
        let remaining = &text[after_key..];
        let bracket_start = remaining.find('[')?;
        let array_slice = &remaining[bracket_start..];

        // 裸 `[`/`]` 深度追踪：不处理引号，只数括号
        let mut depth = 0u32;
        let mut end = 0;
        for (i, b) in array_slice.bytes().enumerate() {
            if b == b'[' {
                depth += 1;
            } else if b == b']' {
                depth -= 1;
                if depth == 0 {
                    end = i;
                    break;
                }
            }
        }
        if depth != 0 {
            continue;
        } // 数组未闭合

        // 尝试解析这个数组片段
        let array_text = &array_slice[..=end];
        let parsed = match serde_json::from_str::<serde_json::Value>(array_text) {
            Ok(v) => Some(v),
            Err(_) => {
                // 数组内部可能有尾逗号等小语法问题，尝试 repair_json 修复后重试
                let repaired = repair_json(array_text);
                serde_json::from_str::<serde_json::Value>(&repaired).ok()
            },
        };
        if let Some(v) = parsed {
            result.insert(key.to_string(), v);
        }
    }

    if result.is_empty() {
        None
    } else {
        Some(serde_json::Value::Object(result))
    }
}

/// 从文本中提取最外层 JSON 对象或数组。
/// 跳过开头的空白和非 JSON 字符，找到第一个 `{` 或 `[`，
/// 追踪括号平衡（带 in_string 追踪）找到匹配闭合，返回解析结果。
/// 如果第一个起点解析失败，尝试下一个起点。
fn extract_outer_json(text: &str) -> Option<serde_json::Value> {
    let bytes = text.as_bytes();
    let len = bytes.len();

    // 收集所有 { 和 [ 的位置
    let start_positions: Vec<usize> = bytes
        .iter()
        .enumerate()
        .filter_map(|(i, &b)| {
            if b == b'{' || b == b'[' {
                Some(i)
            } else {
                None
            }
        })
        .collect();

    for &start in &start_positions {
        let open = bytes[start];
        let close: u8 = if open == b'{' { b'}' } else { b']' };

        let mut depth: i32 = 0;
        let mut in_string = false;
        let mut escaped = false;
        let mut found = false;
        let mut end = 0;

        for (idx, &b) in bytes[start..len].iter().enumerate() {
            let i = start + idx;
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
                if b == open {
                    depth += 1;
                } else if b == close {
                    depth -= 1;
                    if depth == 0 {
                        found = true;
                        end = i;
                        break;
                    }
                }
            }
        }
        if !found {
            continue;
        }

        let snippet = &text[start..=end];
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(snippet) {
            return Some(v);
        }
    }
    None
}

/// 宽松 JSON 解析：处理模型在 `input` 字段里偶尔出现的轻微格式问题。
/// 生产路径仅被 extract_via_normalizer（test-only）调用，测试独立消费。
#[cfg_attr(not(test), allow(dead_code))]
fn parse_loose_json(s: &str) -> Option<serde_json::Value> {
    let trimmed = s.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(trimmed) {
        return Some(v);
    }
    // 兼容：input 有时是单引号 / 带尾逗号 / 缺外层花括号，这里走 IR 文本块的抽取
    let candidate = axagent_kit::utils::extract_json_from_llm_response(trimmed);
    serde_json::from_str(candidate).ok()
}

/// 深度搜索：从任意嵌套的 JSON 对象中找到含 stock_code 的候选数组
/// 用于兜底提取，当正常路径（params → candidates）失败时
fn find_candidates_deep(value: &serde_json::Value) -> Vec<serde_json::Value> {
    let mut results = Vec::new();
    match value {
        serde_json::Value::Array(arr) => {
            // 检查数组元素是否像候选对象（有 stock_code）
            for item in arr {
                if item.get("stock_code").is_some()
                    && item
                        .get("stock_code")
                        .and_then(|v| v.as_str())
                        .is_some_and(|s| !s.is_empty())
                {
                    results.push(item.clone());
                } else if item.is_object() || item.is_array() {
                    // 递归搜索
                    results.extend(find_candidates_deep(item));
                }
            }
        },
        serde_json::Value::Object(map) => {
            // 优先找 candidates/stocks 等容器字段
            for key in ["candidates", "stocks", "list", "data", "items"] {
                if let Some(v) = map.get(key) {
                    if v.is_array() {
                        for item in v.as_array().unwrap() {
                            if item.get("stock_code").is_some() {
                                results.push(item.clone());
                            }
                        }
                    }
                }
            }
            // 没找到则递归搜索所有值
            if results.is_empty() {
                for v in map.values() {
                    results.extend(find_candidates_deep(v));
                }
            }
        },
        _ => {},
    }
    results
}

/// 逐个提取候选对象：在 candidates 数组内对每个顶层 `{...}` 独立尝试解析。
/// 当某个候选对象内部有语法错误（如字符串中未转义的 `"`）时，不影响其他候选的提取。
fn extract_candidates_one_by_one(text: &str) -> Option<Vec<serde_json::Value>> {
    // 1. 定位 candidates 数组起始
    let arr_start = {
        let key_pos = text.find("\"candidates\"")?;
        let after_key = &text[key_pos + 12..];
        let bracket = after_key.find('[')?;
        key_pos + 12 + bracket + 1
    };
    let content = &text[arr_start..];
    // 2. 逐个扫描顶层对象：正确追踪 in_string
    let mut depth: i32 = 0;
    let mut obj_start: Option<usize> = None;
    let mut in_string = false;
    let mut escaped = false;
    let mut results = Vec::new();
    for (i, &b) in content.as_bytes().iter().enumerate() {
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
        if in_string {
            continue;
        }
        if b == b'{' {
            depth += 1;
            if depth == 1 {
                obj_start = Some(i);
            }
        } else if b == b'}' {
            depth -= 1;
            if depth == 0 {
                if let Some(os) = obj_start.take() {
                    let slice = &content[os..=i];
                    if let Ok(v) = serde_json::from_str::<serde_json::Value>(slice) {
                        results.push(v);
                    } else {
                        // 单个候选内部有语法错误 → repair_json 后重试
                        let repaired = repair_json(slice);
                        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&repaired) {
                            results.push(v);
                        }
                    }
                }
            }
        } else if b == b']' && depth == 0 {
            break;
        }
    }
    if results.is_empty() {
        None
    } else {
        Some(results)
    }
}

/// 从文本内容中尝试启发式提取候选列表
/// 用于最终兜底：当所有结构化提取都失败时，直接从 LLM 文本输出中挖
///
/// 契约：返回 `None` 或**非空**数组 —— 两条路径都以「挖到候选」为成功条件，
/// 调用方拿到 `Some` 就可以直接判定兜底命中，不需要再判空。
fn try_extract_candidates_from_text(text: &str) -> Option<Vec<serde_json::Value>> {
    // 尝试1: 找 "candidates": [ ... ] 块，逐个提取
    // 逐个提取相比于全量解析更稳健——即使某个候选对象内部有语法错误，
    // 其他候选仍能被回收。（LLM 高频问题：字符串值中未转义的引号）
    if let Some(found) = extract_candidates_one_by_one(text) {
        return Some(found);
    }

    // 尝试2: 搜索 "stock_code": "XXXXXX" 模式，提取周围的对象
    let mut found = Vec::new();
    let mut search_start = 0;
    while let Some(pos) = text[search_start..].find("\"stock_code\"") {
        let abs_pos = search_start + pos;
        // 验证后面跟着 : "6位数字"
        let after_key = &text[abs_pos + 12..];
        if after_key.starts_with("\": \"") {
            let code_start = abs_pos + 15;
            if code_start + 6 <= text.len() {
                let code = &text[code_start..code_start + 6];
                if code.chars().all(|c| c.is_ascii_digit()) {
                    // 向前找 { 向后找 } 来包围这个对象
                    let region_start = abs_pos.saturating_sub(300);
                    let region_end = (abs_pos + 500).min(text.len());
                    let region = &text[region_start..region_end];
                    let obj_offset = abs_pos - region_start;
                    if let Some(obj_s) = region[..obj_offset].rfind('{') {
                        if let Some(obj_e) = region[obj_s..].find('}') {
                            let candidate_str = &region[obj_s..obj_s + obj_e + 1];
                            if let Ok(obj) =
                                serde_json::from_str::<serde_json::Value>(candidate_str)
                            {
                                if obj.get("stock_code").is_some()
                                    && obj.get("stock_name").is_some()
                                {
                                    found.push(obj);
                                }
                            }
                        }
                    }
                }
            }
        }
        search_start = abs_pos + 13; // 跳过已搜索部分
    }

    if found.is_empty() { None } else { Some(found) }
}

/// 候选评分输入的**唯一优先级实现**。
///
/// `serenity_score` = 确定性评分链的产物（快速链由 `strategy-scorer` 算出
/// `bottleneck_composite`，候选组装端取最高分并按可靠度折价）；`confidence` 是 LLM
/// 自报置信度。落库胜率必须由前者驱动，否则确定性评分全部白算。旧模板/旧记录没有
/// `serenity_score` 时才回落后者，再缺省中性 50。值统一 clamp 到 0..100。
fn candidate_score_input(c: &serde_json::Value) -> i32 {
    c["serenity_score"]
        .as_f64()
        .or_else(|| c["confidence"].as_f64())
        .map(|v| v.clamp(0.0, 100.0))
        .unwrap_or(50.0) as i32
}

/// 从节点原始输出中直接提取 candidates 数组
/// 与通用 extract_agent_output 不同，此函数直接导航已知 JSON 路径：
///   {"content": "...```json\n{\"name\": \"...\", \"arguments\": {\"candidates\": [...]}\n```..."}
/// 返回 {"candidates": [...], "summary": "..."} 或 null。
/// `summary` 取自 arguments.summary（当上游趋势/瓶颈数据缺失时，LLM 通常会
/// 在此字段给出"为什么没有候选"的解释，前端需要在空候选时把它展示给用户）。
fn serenity_extract_from_node(raw: &serde_json::Value) -> serde_json::Value {
    let content = match raw.get("content").and_then(|c| c.as_str()) {
        Some(c) => c,
        None => {
            tracing::warn!("[serenity] 节点输出无 content 字段");
            return serde_json::Value::Null;
        },
    };
    let first = serenity_extract_from_content(content);
    if serenity_has_candidates(&first) {
        return first;
    }
    // rt-workflow 的 strict_mode 在 tool_json 块解析失败时，会把整段原文压进
    // `{"report": "…", "verdict": {…}}`（agent_executor 的 VERDICT 重构分支）。
    // 此时 content 是**合法 JSON**，只是换了形状：下面四层提取与修复链全部空转，
    // 文本兜底又匹配不到（report 内的引号已被 JSON 转义成 `\"candidates\"`）。
    // 实测 2026-10-02 轮：5 只候选完整躺在 report 里，趋势智选产出 0。解包重跑一次。
    //
    // ⚠ 与 `data-verifier.rhai` 的路径4（同为 report 解包）**不是重复实现**：路径4 是
    // 整体 json_parse，只对**语法完好**的 report 生效；能走到这里说明上游三层
    // （verifier 路径4 → 本函数四层提取 → 文本兜底）都已对该 report 空手而归。
    // 本层靠 `extract_candidates_one_by_one` 的**逐对象**提取，才能在被截断/多括号的
    // 坏块里回收损坏点之后的候选。
    let Some(report) = serenity_strict_mode_report(content) else {
        return first;
    };
    let inner = serenity_extract_from_content(&report);
    // 命中判据 = 回收到的候选 **或** 模型自己给出的缺席理由。只认候选会把后者一并丢掉，
    // 界面就退回「无候选、也无原因」—— 与本轮刚修掉的层 4 那句编造的「上游数据不足」
    // 是同一族的两面：一面凭空造原因，一面把真原因扔掉。
    let inner_has_reason =
        inner.get("summary").and_then(|s| s.as_str()).is_some_and(|s| !s.trim().is_empty());
    if serenity_has_candidates(&inner) || inner_has_reason {
        tracing::info!(
            candidates = inner["candidates"].as_array().map_or(0, |a| a.len()),
            keeps_reason = inner_has_reason,
            "[serenity] strict_mode report 解包重跑成功"
        );
        return inner;
    }
    first
}

/// strict_mode VERDICT 包装里的 `report` 文本；content 不是该形状时返回 None。
fn serenity_strict_mode_report(content: &str) -> Option<String> {
    let parsed: serde_json::Value = serde_json::from_str(content).ok()?;
    let report = parsed.get("report")?.as_str()?;
    (!report.is_empty()).then(|| report.to_string())
}

/// 提取结果是否已含非空 candidates 数组。
fn serenity_has_candidates(v: &serde_json::Value) -> bool {
    v.get("candidates").and_then(|c| c.as_array()).is_some_and(|a| !a.is_empty())
}

/// 对一段候选文本执行四层提取 + 修复链（`serenity_extract_from_node` 的实现体）。
fn serenity_extract_from_content(content: &str) -> serde_json::Value {
    let extracted = axagent_kit::utils::extract_json_from_llm_response(content);
    let parsed: serde_json::Value = match serde_json::from_str(extracted) {
        Ok(v) => v,
        Err(e) => {
            // 2026-09-07 诊断增强：解析失败必须能看到失败点附近原文，
            // 否则修复链全败后无法定位 LLM 输出的具体语法损坏。
            tracing::warn!(
                extracted_len = extracted.len(),
                content_len = content.len(),
                err_line = e.line(),
                err_col = e.column(),
                window = %parse_error_window(extracted, &e, 160),
                "[serenity] JSON 解析失败: {e}, 尝试修复链"
            );
            // 第一层：repair_json 修复括号/引号 → 重新解析
            let repaired = repair_json(extracted);
            if let Ok(v) = serde_json::from_str(&repaired) {
                tracing::info!("[serenity] repair_json 成功");
                return v;
            }
            // 第二层：extract_named_arrays 从裁剪后文本提取
            if let Some(named) = extract_named_arrays(extracted) {
                tracing::info!("[serenity] extract_named_arrays(extracted) 成功");
                return named;
            }
            // 第三层：extract_named_arrays 从原始 content 提取
            // 裁剪后的 extracted 可能被 trim_after_json 截断，
            // 原始 content 包含完整 JSON，免疫截断问题
            if let Some(named) = extract_named_arrays(content) {
                tracing::info!("[serenity] extract_named_arrays(content) 成功");
                return named;
            }
            // 第四层：文本启发式兜底
            tracing::warn!("[serenity] 修复链前三层均失败，尝试文本兜底提取");
            // 兜底命中即采用回收到的候选 —— 这里**没有**「有 summary 就返回空候选」的
            // 分支了（2026-10-05 归因）：旧分支靠 `has_summary` 先判，而本函数契约是
            // `Some` ⇒ 非空数组，于是它唯一能触发的时机恰恰是「候选已回收」那次，
            // 结果把候选整段丢弃、换成一句硬编码的「上游数据不足」—— 既产出 0 候选，
            // 又向用户编造了一句模型自己没说过的缺席原因。
            // 模型真的拒绝出票时输出语法完好（`{"candidates": [], "summary": "…"}`），
            // 在层 0 就带着**模型原文的** summary 返回，不经兜底。
            if let Some(found) = try_extract_candidates_from_text(content) {
                tracing::info!("[serenity] 文本兜底提取成功，回收 {} 个候选", found.len());
                return serde_json::json!({"candidates": found});
            }
            return serde_json::Value::Null;
        },
    };
    // 🚨 2026-07-31 修复：agent_executor 拆包 tool_json 时，若 LLM 输出的 arguments 是裸数组
    // （{"name":"submit_candidates","arguments":[{candidate},...]}，GLM 偶发形态），
    // content 就是 candidates 数组本身。此前只处理"对象含 candidates 字段"，裸数组
    // 一路走到 None → 返回 Null → 种子永不注入（21:47 轮 candidate-mapper 4 候选全被丢弃）。
    if parsed.is_array() {
        let count = parsed.as_array().map(|a| a.len()).unwrap_or(0);
        tracing::info!(
            "[serenity] parsed 为裸候选数组（arguments 直出形态），直接作为 candidates: {} 个",
            count
        );
        return serde_json::json!({"candidates": parsed});
    }
    // 导航到 arguments/input → candidates
    let args = parsed
        .as_object()
        .and_then(|o| o.get("arguments"))
        .or_else(|| parsed.as_object().and_then(|o| o.get("input")));
    let candidates = match args {
        Some(a) => a.get("candidates"),
        None => parsed.as_object().and_then(|o| o.get("candidates")),
    };
    // summary 同样在 arguments.summary（或顶层 summary），用来在 candidates 为空时
    // 告知前端"为什么没有候选"（如：上游 data_gaps=true、模型反幻觉拒绝编造等）
    let summary = args
        .and_then(|a| a.get("summary"))
        .and_then(|s| s.as_str())
        .map(|s| s.to_string())
        .or_else(|| {
            parsed
                .as_object()
                .and_then(|o| o.get("summary"))
                .and_then(|s| s.as_str())
                .map(|s| s.to_string())
        });
    match candidates {
        Some(arr) if arr.is_array() => {
            let count = arr.as_array().map(|a| a.len()).unwrap_or(0);
            tracing::info!("[serenity] 直接提取成功，找到 {} 个候选", count);
            if let Some(s) = summary {
                serde_json::json!({"candidates": arr, "summary": s})
            } else {
                serde_json::json!({"candidates": arr})
            }
        },
        Some(_) => {
            tracing::warn!(
                "[serenity] candidates 不是数组，keys={:?}",
                candidates
                    .and_then(|c| c.as_object())
                    .map(|o| o.keys().cloned().collect::<Vec<_>>())
            );
            // 即使 candidates 字段格式异常，summary 仍可能有用
            if let Some(s) = summary {
                serde_json::json!({"candidates": [], "summary": s})
            } else {
                serde_json::Value::Null
            }
        },
        None => {
            // 最后的兜底：parsed 本身可能是裸候选对象
            if parsed.get("stock_code").is_some() {
                let mut out = serde_json::json!({"candidates": [parsed]});
                if let Some(s) = summary {
                    out.as_object_mut()
                        .map(|o| o.insert("summary".to_string(), serde_json::Value::String(s)));
                }
                out
            } else if let Some(s) = summary {
                // 找不到 candidates 字段但有 summary：典型场景是 LLM 拒绝编造
                tracing::info!("[serenity] 未找到 candidates 字段但有 summary，空候选 + 原因");
                serde_json::json!({"candidates": [], "summary": s})
            } else {
                tracing::warn!(
                    "[serenity] 无法找到 candidates 字段, parsed keys={:?}",
                    parsed.as_object().map(|o| o.keys().cloned().collect::<Vec<_>>())
                );
                serde_json::Value::Null
            }
        },
    }
}

/// 运行 Serenity 瓶颈筛选工作流（`serenity-screening` 模板）。
///
/// 与 run_stock_workflow 不同：
///   - 不需要 stock_code 输入（自驱动，从市场数据发现趋势）
///   - 不写 stock_analyses 表
///   - 返回候选股清单（而非单只股票的分析结论）
///
/// `run_id`：由前端生成的单次运行标识，原样回灌到所有事件（step/completed/failed）
/// 的 `runId` 字段。前端据此丢弃其它运行（并发/残留）的事件，避免进度串台。
/// 不传时退化为 workflow id，保持对旧调用方可用。
///
/// `template_id`：两链共用本命令，由前端按钮决定跑哪条 ——
///   - 不传 / `serenity-screening`：原链（12 个 Agent 腿，慢但覆盖全）
///   - `serenity-screening-fast`：快速链（确定性简报 + 单 Agent，见 `seed_serenity_fast.rs`）
/// ⚠️ 只接受 `SERENITY_TEMPLATE_IDS` 白名单内的 id：本参数直接决定 `load_and_inject_template`
/// 读哪一行模板，不做白名单就等于把「任意模板 id」开放给前端。
/// ⚠️ 事件 `type` 两链**共用** `serenity-screening`（前端 `SerenityScreeningPanel` 的监听器
/// 与 store 按该名注册）—— 两链产物是同一个业务对象（趋势智选候选），共用面板即零改前端；
/// 单次运行用 `runId` 区分（并发/残留事件按它过滤），不需要按链拆事件名。
#[agent_command(domain = "finance", safety = Caution, call_mode = StateOnly, description =  "运行Serenity瓶颈筛选工作流")]
#[tauri::command]
pub async fn run_serenity_screening(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    as_of_date: Option<String>,
    themes: Option<Vec<String>>,
    run_id: Option<String>,
    template_id: Option<String>,
) -> Result<serde_json::Value, String> {
    // 模板 id 白名单校验（默认原链，保持对旧调用方可用）—— 留在命令层，
    // inner 信任调用方传入的 id（批次恒用 SERENITY_TEMPLATE_IDS[0]）。
    let template_id = template_id.unwrap_or_else(|| SERENITY_TEMPLATE_IDS[0].to_string());
    if !SERENITY_TEMPLATE_IDS.contains(&template_id.as_str()) {
        return Err(format!(
            "未知的趋势智选模板 id: {template_id}（可用: {}）",
            SERENITY_TEMPLATE_IDS.join(", ")
        ));
    }

    // 解析 as_of_date（支持回放模式）
    let as_of_ctx = parse_asof_param(as_of_date.clone())?;

    run_serenity_screening_inner(SerenityScreeningArgs {
        db: state.harness.db(),
        client: &state.astock_client,
        engine: Arc::clone(&state.work_engine),
        app: Some(app),
        as_of_ctx,
        themes,
        run_id,
        replay_purpose: crate::commands::_shared_reco_pool::purpose::MANUAL_REPLAY,
        template_id,
    })
    .await
}

/// 趋势智选执行参数 —— Tauri 命令与时间旅行批次共用同一份执行体。
///
/// 两条入口的分叉（2026-10-12 裁定「回放的 as-of 候选落库」之后，只剩一处行为差异）：
/// - `app = None`（批次）：静默运行，不 emit step/completed/failed 前端事件；
/// - `as_of_ctx = Some(_)`（回放）：`reco_picks` **照落库**，`as_of_date` 写本批截止日 ⇒
///   回放候选可供反思取样；只有**全局 serenity 种子同步**仍限 live —— 种子是进程内单槽、
///   无代际维度，回放若写它，live 的下一轮建池会被换成历史池。
pub(crate) struct SerenityScreeningArgs<'a> {
    pub db: &'a DatabaseConnection,
    pub client: &'a AStockClient,
    pub engine: Arc<axagent_rt_workflow::work_engine::WorkEngine>,
    /// None = 时间旅行批次静默模式
    pub app: Option<tauri::AppHandle>,
    pub as_of_ctx: Option<as_of::AsOfContext>,
    pub themes: Option<Vec<String>>,
    pub run_id: Option<String>,
    /// 回放行落库时的 `purpose`：手动入口传 `manual_replay`，时间旅行批次传
    /// `time_travel_replay`（live 行恒为 `live`）。两条入口共用同一执行体，
    /// 不显式传就只能靠猜，历史面板会把时间旅行记成手动回放。
    pub replay_purpose: &'static str,
    /// 已过白名单校验的模板 id（`serenity-screening` / `-fast`）
    pub template_id: String,
}

/// 趋势智选工作流执行体（[`run_serenity_screening`] 命令与时间旅行批次共用，
/// 保证手动运行 / 每日 cron 同模板 / as-of 批次三条入口口径一致）。
/// 注入智选的达标清单最多列多少只（清单是提示词素材，不是全量导出）。
const MOVER_WATCHLIST_LIMIT: usize = 20;

pub(crate) async fn run_serenity_screening_inner(
    args: SerenityScreeningArgs<'_>,
) -> Result<serde_json::Value, String> {
    let SerenityScreeningArgs {
        db,
        client,
        engine,
        app,
        as_of_ctx,
        themes,
        run_id,
        replay_purpose,
        template_id,
    } = args;

    // 运行前确保模板为最新版（幂等：版本已是最新则跳过）。
    // 历史上启动阶段 seed 在 fire-and-forget 异步任务中执行，失败仅 log 不阻塞，
    // 若数据库停留在旧版本（如 v2 缺少 baseline_* input_mapping），
    // Rhai 脚本会报 "Variable not found: baseline_semi"。此处兜底重新 seed。
    // 批次入口同样需要：as-of 回放必须跑在当前模板版本上。
    crate::commands::stock_analysis_setup::ensure_stock_analysis_experts_seeded(db)
        .await
        .map_err(|e| format!("重新种子化失败: {e}"))?;

    // 1. 加载趋势智选模板（原链 / 快速链，由 template_id 决定）
    let loaded = load_and_inject_template(db, "", "", template_id.as_str()).await?;
    tracing::info!("[serenity] 运行模板: {template_id}");

    // 注入 vendor 启用状态过滤器（与 stock-analysis 主工作流一致）
    super::decision::inject_vendor_state(client, loaded.variables.as_ref());

    // v47: 注入用户主题到 variables（对话式主题荐股）
    let mut variables = loaded.variables;
    if let Some(ref theme_list) = themes {
        if !theme_list.is_empty() {
            let themes_value = serde_json::json!(theme_list);
            if let Some(ref mut vars) = variables {
                if let Some(var) = vars.iter_mut().find(|v| v.name == "user_themes") {
                    var.value = themes_value;
                } else {
                    vars.push(axagent_harness::workflow_types::Variable {
                        name: "user_themes".into(),
                        var_type: "json".into(),
                        value: themes_value,
                        description: Some("用户指定主题词列表".into()),
                        is_secret: false,
                    });
                }
            } else {
                variables = Some(vec![axagent_harness::workflow_types::Variable {
                    name: "user_themes".into(),
                    var_type: "json".into(),
                    value: themes_value,
                    description: Some("用户指定主题词列表".into()),
                    is_secret: false,
                }]);
            }
        }
    }

    // ── B-1：窗口涨幅达标清单作为 a-trend-scanner 的第 4 个输入（输入面，不改权重口径）──
    // live：取最新收盘日的窗口达标票（判据与漏检面板同源）；as-of：显式声明本代无清单，
    // **不开 Fallthrough** —— 回放若读 live 的清单，就是把未来摆到 LLM 眼前。
    let watchlist_text = if as_of_ctx.is_some() {
        axagent_analysis_engine::mover_recall::MOVER_WATCHLIST_ASOF_DECLARATION.to_string()
    } else {
        axagent_analysis_engine::mover_recall::mover_watchlist_digest(db, MOVER_WATCHLIST_LIMIT)
            .await
            .unwrap_or_else(|e| {
                // 取数失败不得静默成空串：空串在提示词里与「无达标票」不可区分 ⇒ 显式声明原因
                tracing::warn!("[serenity] 达标清单取数失败，按「本代无清单」注入: {e}");
                format!("（本代无达标清单：取数失败 —— {e}）")
            })
    };
    let watchlist_value = serde_json::json!(watchlist_text);
    let mut pushed = false;
    if let Some(ref mut vars) = variables {
        if let Some(var) = vars.iter_mut().find(|v| v.name == "mover_watchlist") {
            var.value = watchlist_value.clone();
        } else {
            vars.push(axagent_harness::workflow_types::Variable {
                name: "mover_watchlist".into(),
                var_type: "string".into(),
                value: watchlist_value.clone(),
                description: Some("系统注入：最新收盘日窗口涨幅达标清单".into()),
                is_secret: false,
            });
        }
        pushed = true;
    }
    if !pushed {
        variables = Some(vec![axagent_harness::workflow_types::Variable {
            name: "mover_watchlist".into(),
            var_type: "string".into(),
            value: watchlist_value,
            description: Some("系统注入：最新收盘日窗口涨幅达标清单".into()),
            is_secret: false,
        }]);
    }

    // 执行参数四面（并发 / 单步超时 / 工具超时 / 按类型上限）的唯一解析点。
    // ⚠ 本链此前只取了 max_concurrent 与 step_timeout 两格、其余交给 `..Default::default()`
    //   ⇒ crate 默认的 `llm=agent=3` 把面板设定的并发静默压回 3（P2 收口，2026-10-25 修）。
    //   本链不做外层总超时，`total_timeout` 不取。
    let exec = ExecutionParams::from_template_vars(variables.as_deref());

    // 2. 创建 Workflow（实例名带模板标识：两链实例在 DB/日志中可区分）
    let wf_name = format!("{template_id}-{}", chrono::Utc::now().timestamp_millis());
    // 统一 with_hooks 模式：模板未来声明钩子时不会静默丢失
    let workflow = engine
        .create_workflow_with_hooks(&wf_name, loaded.nodes, loaded.edges, loaded.hooks_config)
        .await
        .map_err(|e| {
            ErrorResponse::new(wf_err::INTERNAL).with_detail(format!("创建工作流失败: {e}"))
        })?;
    let wf_id = workflow.id.clone();
    let wf_id_ret = wf_id.clone();
    let app_h = app.clone();
    // 事件运行标识：优先用前端传入的 runId（前端按此过滤串台事件），
    // 未传时退化为 workflow id（旧调用方仍可用）。
    let event_run_id = run_id.clone().unwrap_or_else(|| wf_id.clone());

    // ── 〇-B v2 / Phase 6 / Q2 裁定：趋势智选服务 **mid + long 两档**，短/超短按设计不做 ──
    // 档位集合的唯一权威是 `recommender/style_matrix.rs` 的 serenity 行（= {mid, long}），
    // 与策略链注册表同源（`SerenityStrategy::mid()/long()`，`strategies/serenity.rs:22-29`
    // 「只做中长期」，瓶颈/政策/业绩类催化剂以周-月展开）。
    // 旧形态本链**恒 mid 一档**，而策略链已注册两档 ⇒ 同一个「趋势智选」名目下
    // 两条链的档位集合不一致（Q2 裁定：收口为两档各落一行，而不是把策略链砍成一档）。
    // 再往前这里是裸字面量 `"mid"` / 止损 ×0.80 / 目标 ×1.30 / 持有 20 天：
    //   ① 20 天与 `Period::Mid` 的 28 天**口径互相矛盾**（同一行 reco_picks 里
    //      period=mid 却 holding_days=20，反思按天判成熟即错位）；
    //   ② 乘数与策略包变量（`serenity_*_mult`，可在设置面板改）分叉 —— 面板调了、
    //      工作流落库价仍然按老常数算，「可配置」形同虚设。
    // ⚠ 短/超短档**不支持**是产品定位而非遗漏：不产该档记录，也不把别的档静默标成 mid。
    let serenity_tiers = [axagent_harness::Period::Mid, axagent_harness::Period::Long];
    let read_var = |name: &str, default: f64| -> f64 {
        variables
            .as_ref()
            .and_then(|vs| vs.iter().find(|v| v.name == name))
            .and_then(|v| v.value.as_f64())
            .unwrap_or(default)
    };
    let serenity_stop_mult = read_var("serenity_stop_mult", 0.80);
    let serenity_target_mult = read_var("serenity_target_mult", 1.30);
    let serenity_entry_range = read_var("serenity_entry_range", 0.05);
    // 波动率风控参数（Phase R-D）：键名与默认值与 `recommender/mod.rs` 的 R-D 读取**逐字一致**，
    // 否则同一只票经两条链会拿到不同 k1/k2/R/成本 —— 那正是本轮要合流的缺陷。
    let reco_stop_k1 = read_var("reco_stop_vol_mult", 1.2);
    let reco_target_k2 = read_var("reco_target_vol_mult", 2.0);
    let reco_risk_budget_pct = read_var("reco_risk_budget_pct", 1.5);
    let reco_round_trip_cost_pct = read_var("reco_round_trip_cost_pct", 0.6);
    let serenity_tier_desc = serenity_tiers
        .iter()
        .map(|t| format!("{}({}天)", t.as_str(), t.default_holding_days()))
        .collect::<Vec<_>>()
        .join("+");
    tracing::info!(
        "[serenity] 档位口径: {serenity_tier_desc}          止损/目标=k·σ_daily·√h（k1={reco_stop_k1} k2={reco_target_k2}），σ 不可得退 stop×{serenity_stop_mult}/target×{serenity_target_mult}          建仓带=该档止损距离一半（Q1=B）          （短/超短档不适用：serenity 行按设计不做）"
    );

    // 3. 进度回调
    let progress_app = app.clone();
    let progress_wf_id = wf_id.clone();
    let progress_run_id = event_run_id.clone();
    let progress_cb: ProgressCallback = Arc::new(move |event: StepProgressEvent| {
        let app = progress_app.clone();
        let wf_id = progress_wf_id.clone();
        let run_id = progress_run_id.clone();
        Box::pin(async move {
            // 过滤 streaming 增量（AgentExecutor 每 2s 一次）：Serenity 执行日志
            // 只关心节点级状态切换，透传会以全量文本刷屏。
            if event.status == "streaming" {
                return;
            }
            let payload = serde_json::json!({
                "workflowId": wf_id,
                "runId": run_id,
                "type": "serenity-screening",
                "nodeId": event.node_id,
                "status": event.status,
                "totalNodes": event.total_nodes,
                "completedNodes": event.completed_nodes,
                // 修复：透传节点真实输出与错误信息（与 stock_workflow/core.rs 的
                // workflow-step-done 事件对齐）。此前只发 nodeId/status/counts，
                // 前端 SerenityScreeningPanel 执行日志永远显示"执行完成，无输出内容"，
                // 用户无法判断节点是否拿到真实数据。
                "output": event.output,
                "error": event.error,
                // 与 `stock_workflow/core.rs` 的 `workflow-step-done` 对齐（同一个映射函数，
                // 见 `super::core::node_error_code` 的可见性注释）：
                // **`error` 负责展示、`errorCode` 负责判定** —— 前端不得再用
                // `error.startsWith("EXECUTION_CANCELLED")` 这类子串嗅探反推语义。
                // `None`（running/completed）⇒ 序列化为 `null`，表示「本事件无失败」；
                // 消费端须用 `typeof errorCode === "string"` 判定，别把 `null` 当兜底码。
                "errorCode": super::core::node_error_code(event.error_code.as_deref())
                    .map(|(c, _)| c),
            });
            // 批次静默模式（app=None）跳过事件发射；命令路径行为不变。
            if let Some(app) = app {
                let _ = app.emit(IpcEventName::SerenityScreeningStep.as_str(), payload);
            }
        })
    });

    // 4. 运行（支持 as-of 时间截断）
    // 注入模板变量（来自 UI 可编辑的 Variables）。
    // 🚨 2026-07-31 修复：原过滤只认 ref_*/serenity_* 前缀，policy_news_keywords（t-policy-news
    // 的搜索关键词变量）被滤掉 → 运行时 resolve_var_path 查不到 → search_news keyword 恒空
    // （连续 8 轮日志"keyword="）。v17 已删除 ref_*_code，前缀白名单过时。
    // 更彻底的做法：模板 variables 全部注入（均为可编辑参数，无敏感字段）。
    // v47: 使用已注入用户主题的 variables（而非原始 loaded.variables）
    let serenity_vars: Option<Vec<axagent_harness::workflow_types::Variable>> = variables;
    let mut opts = RunOptions {
        progress_callback: Some(progress_cb),
        input: None,
        input_schema: loaded.input_schema.clone(),
        output_schema: loaded.output_schema.clone(),
        variables: serenity_vars,
        dry_run: false,
        // 接线激活 strict_mode：VERDICT 缺失兜底重试 / strict JSON 校验与降级
        tool_permissions: Some(super::strict_tool_permissions()),
        // 与 `core.rs` 的主分析链同一接线（P1-8，2026-10-03）：引擎为**每个节点单独
        // spawn**，而 `task_local` 不跨 spawn ⇒ 下方裸 `AS_OF.scope` 罩不住节点内取数，
        // 回放时 tool/agent 节点会全部读成 live。必须把本轮截止日带进每个节点任务。
        node_task_scope: Some(axagent_rt_workflow::work_engine::engine::node_task_scope_hook(
            move |fut| as_of::with_optional_asof(as_of_ctx, fut),
        )),
        ..Default::default()
    };
    exec.apply_to(&mut opts);

    // 本轮降级留痕绑定到**这次运行**（与荐股链 `recommend_stocks` 的 H3 同一形态）：
    // `with_degradation_log` 给本任务提供容器；进出各取一次水位 ⇒ 切片只含本轮新增，
    // 挡掉同截止日上一轮的残留（R5 基线陷阱同族）。
    let deg_watermark = as_of::global_degradation_seq_watermark();

    // 切片必须在容器**内**取：退出 `with_degradation_log` 后父任务那一层条目已随作用域结束，
    // 只剩全局环形缓冲，且 `take_asof_degradation_report` 会多打一条「访问失败」warn。
    let (result, asof_degradations) = as_of::AS_OF
        .scope(
            as_of_ctx,
            as_of::with_degradation_log(async {
                let wf = engine.run_workflow(&wf_id, opts).await;
                let degradations: Vec<
                    axagent_analysis_engine::recommender::types::RecoDegradation,
                > = if as_of::current_as_of().is_some() {
                    as_of::take_global_degradations_since(deg_watermark)
                        .into_iter()
                        .map(Into::into)
                        .collect()
                } else {
                    Vec::new()
                };
                (wf, degradations)
            }),
        )
        .await;
    // 把**本轮实际使用的截止日**回灌进 payload 与返回值：渲染层因此不必去读全局时间锚点
    // （锚点在运行后被改动就会把实盘显示成回放，或反之 —— T11 同族）。live ⇒ None。
    let run_as_of_date = as_of_ctx.map(|c| c.as_string());

    match result {
        Ok(wf_result) => {
            tracing::info!(
                "[serenity] wf_result.results 所有键: {:?}",
                wf_result.results.keys().cloned().collect::<Vec<_>>(),
            );

            let candidates_raw = wf_result
                .results
                .get("c-data-verifier")  // FIX-02: 优先使用 data-verifier 的输出（含验证状态）
                .map(|v| {
                    // CodeNode 输出: {"status": "executed", "result": [...], "params": [...]}
                    // 从 result 中提取候选列表，兼容 AgentNode 的 content 格式
                    if v.get("result").is_some() {
                        serde_json::json!({"content": serde_json::to_string(&v["result"]).unwrap_or_default()})
                    } else {
                        v.clone()
                    }
                })
                // v44 修复（2026-07-31 23:05）：data-verifier 因 input_mapping 路径失效
                // （content 为 arguments 文本字符串）early return 空数组时，不能吞掉真候选，
                // 回退到 a-candidate-mapper 原始输出重新提取。
                .filter(|v| {
                    let is_empty_array = v
                        .get("content")
                        .and_then(|c| c.as_str())
                        .and_then(|s| serde_json::from_str::<serde_json::Value>(s).ok())
                        .and_then(|p| p.as_array().map(|a| a.is_empty()))
                        .unwrap_or(false);
                    !is_empty_array
                })
                .or_else(|| {
                    // 快速链（`serenity-screening-fast`）修复：该链的 a-candidate-mapper 是
                    // **Code 节点**（不再是 Agent），输出形如
                    // `{"status":"executed","language":"rhai","result":{candidates,summary},...}`
                    // ——**没有 `content` 字段**，`serenity_extract_from_node` 会走
                    // 「节点输出无 content 字段」分支直接返回 Null。此处与上面 data-verifier
                    // 分支同样先包装 `{"content": to_string(result)}`（`result` 的顶层键就是
                    // `candidates`/`summary`，正是该提取函数认的路径）。
                    // 原链该节点仍是 Agent（输出有 content）⇒ `result` 不存在，走 `else`
                    // 原样透传，零影响。
                    wf_result.results.get("a-candidate-mapper").map(|v| {
                        if v.get("result").is_some() {
                            serde_json::json!({
                                "content": serde_json::to_string(&v["result"]).unwrap_or_default()
                            })
                        } else {
                            v.clone()
                        }
                    })
                })
                .unwrap_or(serde_json::Value::Null);
            // 诊断：打印原始节点输出
            {
                let preview = serde_json::to_string(&candidates_raw)
                    .map(|s| s.chars().take(500).collect::<String>())
                    .unwrap_or_default();
                tracing::info!("[serenity] a-candidate-mapper 原始输出 (前500字符): {}", preview);
            }
            // 使用专属提取函数直接从已知 JSON 路径 (content → arguments.candidates) 提取，
            // 绕过 extract_agent_output 的复杂 fallback 逻辑（该函数在 tool_json 格式下可能返回首条候选而非完整数组）
            let candidates_raw_fallback = candidates_raw.clone();
            let candidates = serenity_extract_from_node(&candidates_raw);
            // 诊断：serenity_extract_from_node 的返回值
            {
                let preview = serde_json::to_string(&candidates)
                    .map(|s| s.chars().take(400).collect::<String>())
                    .unwrap_or_default();
                tracing::info!(
                    "[serenity] serenity_extract 返回类型={}  前400字符: {}",
                    if candidates.is_array() {
                        "数组".to_string()
                    } else if candidates.is_object() {
                        let keys = candidates
                            .as_object()
                            .map(|o| o.keys().cloned().collect::<Vec<_>>())
                            .unwrap_or_default();
                        format!("对象 keys=[{}]", keys.join(","))
                    } else if candidates.is_null() {
                        "null".to_string()
                    } else {
                        "其他".to_string()
                    },
                    preview,
                );
            }
            // 规范化：如果 extract_agent_output 返回裸候选对象（有 stock_code 但无 candidates 包装键），
            // 包装成 {"candidates": [obj]}，使下游 .get("candidates") 能正常工作。
            let candidates = if candidates.is_object()
                && !candidates.as_object().is_some_and(|o| o.contains_key("candidates"))
                && candidates.get("stock_code").is_some()
            {
                serde_json::json!({"candidates": [candidates]})
            } else {
                candidates
            };
            // 提取 candidates 数组（各种包装格式统一为平级数组，直接供前端消费）
            let raw_candidate_array = if candidates.is_array() {
                candidates.clone()
            } else if let Some(obj) = candidates.as_object() {
                obj.get("candidates").cloned().unwrap_or(serde_json::Value::Array(vec![]))
            } else {
                serde_json::Value::Array(vec![])
            };
            // 校验：过滤缺少 stock_code 的残缺候选，避免前端渲染空白卡片
            let mut candidate_array: Vec<serde_json::Value> = Vec::new();
            let mut dropped_count = 0;
            let mut exit_now_count = 0;
            if let Some(arr) = raw_candidate_array.as_array() {
                for c in arr {
                    let has_code =
                        c.get("stock_code").and_then(|v| v.as_str()).is_some_and(|s| !s.is_empty());
                    if !has_code {
                        dropped_count += 1;
                        tracing::warn!(
                            "[serenity] 丢弃残缺候选（无 stock_code）: {}",
                            serde_json::to_string(c).unwrap_or_default()
                        );
                        continue;
                    }
                    // 自动剔除 exit_now 候选（LLM 可能违反 prompt 规则仍然输出）
                    let is_exit_now = c
                        .get("exit_signals")
                        .and_then(|es| es.get("overall_exit_urgency"))
                        .and_then(|v| v.as_str())
                        .is_some_and(|s| s == "exit_now");
                    if is_exit_now {
                        exit_now_count += 1;
                        tracing::warn!(
                            "[serenity] 自动剔除 exit_now 候选（{}）: {}",
                            c["stock_code"].as_str().unwrap_or("?"),
                            c["exit_signals"]["overall_exit_urgency"]
                                .as_str()
                                .unwrap_or("exit_now"),
                        );
                        continue;
                    }
                    candidate_array.push(c.clone());
                }
            }
            if dropped_count > 0 || exit_now_count > 0 || candidate_array.is_empty() {
                tracing::warn!(
                    "[serenity] 候选校验: 总量={}, 有效={}, 丢弃(无stock_code)={}, 剔除(exit_now)={}, candidates原始keys={:?}",
                    raw_candidate_array.as_array().map_or(0, |a| a.len()),
                    candidate_array.len(),
                    dropped_count,
                    exit_now_count,
                    raw_candidate_array
                        .as_array()
                        .and_then(|a| a.first())
                        .and_then(|c| c.as_object())
                        .map(|o| o.keys().cloned().collect::<Vec<_>>()),
                );
            }
            let mut candidate_array = serde_json::Value::Array(candidate_array);

            // 兜底：如果正常提取路径得到空数组，尝试从 candidates（extract_agent_output 结果）中深度搜索
            if candidate_array.as_array().is_none_or(|a| a.is_empty()) {
                tracing::warn!(
                    "[serenity] ⚠️ 候选数组为空，尝试兜底提取... candidates类型={} keys={:?}",
                    if candidates.is_array() {
                        "array"
                    } else if candidates.is_object() {
                        "object"
                    } else if candidates.is_null() {
                        "null"
                    } else {
                        "other"
                    },
                    candidates.as_object().map(|o| o.keys().cloned().collect::<Vec<_>>()),
                );
                // 兜底策略1: 从 candidates 对象的任意嵌套层搜索含 stock_code 的数组
                let fallback = find_candidates_deep(&candidates);
                if !fallback.is_empty() {
                    tracing::info!("[serenity] 兜底提取成功，找到 {} 个候选", fallback.len());
                    candidate_array = serde_json::json!(fallback);
                }
                // 兜底策略2: 从原始节点输出的 content 字段中提取
                if candidate_array.as_array().is_none_or(|a| a.is_empty()) {
                    if let Some(content) =
                        candidates_raw_fallback.get("content").and_then(|c| c.as_str())
                    {
                        if let Some(found) = try_extract_candidates_from_text(content) {
                            tracing::info!(
                                "[serenity] 文本兜底提取成功，找到 {} 个候选",
                                found.len()
                            );
                            candidate_array = serde_json::json!(found);
                        }
                    }
                }
            }

            // 提取趋势扫描结果（a-trend-scanner 节点输出）
            let trends_raw = wf_result
                .results
                .get("a-trend-scanner")
                .cloned()
                .unwrap_or(serde_json::Value::Null);
            let trends = extract_agent_output(trends_raw).await;
            // 规范化：如果返回裸 trend 对象（有 trend_name 但无 trends 包装键），
            // 包装成 {"trends": [obj]}
            let trends = if trends.is_object()
                && !trends.as_object().is_some_and(|o| o.contains_key("trends"))
                && trends.get("trend_name").is_some()
            {
                serde_json::json!({"trends": [trends]})
            } else {
                trends
            };
            // trends 可能是 { trends: [...] } 对象，也可能是原始数组
            let trends_list =
                trends.as_object().and_then(|obj| obj.get("trends")).cloned().unwrap_or(trends);

            tracing::info!(
                "[serenity] candidates 提取后类型: {}, keys: {:?}; trends 提取后类型: {}",
                if candidates.is_array() {
                    "数组"
                } else if candidates
                    .as_object()
                    .map(|o| o.contains_key("candidates"))
                    .unwrap_or(false)
                {
                    "含 candidates 字段"
                } else if candidates.is_null() {
                    "null"
                } else {
                    "其他"
                },
                candidates.as_object().map(|o| o.keys().cloned().collect::<Vec<_>>()),
                if trends_list.is_array() {
                    "数组"
                } else {
                    "对象"
                },
            );

            // 提取"为什么没有候选"的原因：a-candidate-mapper 的 arguments.summary
            // 当上游三个瓶颈节点均返回 data_gaps=true 时，LLM 会拒绝编造候选
            // 并在 summary 字段说明原因；前端在 candidates 为空时展示给用户。
            let empty_reason = candidates
                .as_object()
                .and_then(|o| o.get("summary"))
                .and_then(|s| s.as_str())
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty());

            // ── 持久化 Serenity 候选到 reco_picks 表（style="serenity"）──
            // 先持久化再 emit 事件，确保数据一致性
            let mut persistence_success = true;
            // 结构化三元组：`persistence_stock_code` = params（哪只）、`persistence_detail` = 技术详情（DB 原文）。
            // 主文案由**码**决定（前端 `translateFailureText` 取 11 语言译文）——
            // 此前这里是 `format!("写入 {} 失败: {e}")` 的自由文本，中文硬编码：
            // ① 非中文界面看到中文；② 「哪只 + 为什么」揉进一个串，无法只本地化主文案而保留技术详情。
            let mut persistence_stock_code = String::new();
            let mut persistence_detail = String::new();
            // ── 逐档展开的对外候选（选项 B）──
            // completed payload 直接按 (候选 × 档) 展开，逐档带上 period 与风控字段，
            // 使面板**运行后立即**呈现 mid/long 两张卡。此前只发原始 LLM 候选（无 period），
            // 卡片只能出「未知周期」，档位要等重挂载读 `reco_picks` 才出现 —— 同一次运行
            // 的实时候选与恢复候选口径分叉。展开形态与前端 `restoreCandidate`
            // （`seed_pool_json` + `pick_data`）的解出结果同形，两路呈现一致。
            let mut live_candidates: Vec<serde_json::Value> = Vec::new();
            // best-effort：失败只记日志，不影响返回结果
            {
                // db 复用 inner 入参（命令 / 批次两条入口同一连接）
                // 统一 generated_at 格式：与 recommend_stocks 一致(ISO 8601 带毫秒)
                let now_str = chrono::Local::now().format("%Y-%m-%dT%H:%M:%S%.3f").to_string();
                // 代际与用途：与荐股链同口径（实盘 = None + live；回放 = 截止日 + 本链用途）
                let serenity_as_of: Option<String> = as_of_ctx.as_ref().map(|c| c.as_string());
                let serenity_purpose = if as_of_ctx.is_some() {
                    replay_purpose
                } else {
                    axagent_dao::repo::reco_pool::purpose::LIVE
                };
                // candidates 可能是 { candidates: [...] } 对象、{name, arguments: {candidates: [...]}} 格式、
                // 也可能是原始数组
                // 优先使用已经过校验(过滤缺 stock_code + exit_now)的 candidate_array
                let candidate_list: Vec<&serde_json::Value> =
                    if let Some(arr) = candidate_array.as_array() {
                        if !arr.is_empty() {
                            arr.iter().collect()
                        } else {
                            // 兜底：从 candidates 中提取
                            candidates
                                .as_object()
                                .and_then(|obj| {
                                    obj.get("candidates")
                                        .or_else(|| {
                                            obj.get("arguments").and_then(|a| a.get("candidates"))
                                        })
                                        .and_then(|v| v.as_array())
                                })
                                .or_else(|| candidates.as_array())
                                .map(|arr| arr.iter().collect())
                                .unwrap_or_default()
                        }
                    } else {
                        Vec::new()
                    };
                // F-1（2026-10-24）：**候选输入池**按批次落库（`chain='serenity'`）。
                // 此前智选不写 `reco_pool_batches`，归因只能读 pick 行上的**单对象**
                // `seed_pool_json` ⇒ 对别的票恒「不在池」（现网实测 21/21 not_in_pool 是伪证据）。
                // 0 候选也写（`None`），让「真的一个候选都没有」与「没跑过」可分。
                {
                    let pool_json = if candidate_list.is_empty() {
                        None
                    } else {
                        serde_json::to_string(&candidate_list).ok()
                    };
                    let purpose = if run_as_of_date.is_some() {
                        crate::commands::_shared_reco_pool::purpose::MANUAL_REPLAY
                    } else {
                        crate::commands::_shared_reco_pool::purpose::LIVE
                    };
                    crate::commands::_shared_reco_pool::persist_reco_pool_batch(
                        db,
                        "serenity",
                        "all",
                        pool_json.as_deref(),
                        &now_str,
                        run_as_of_date.as_deref(),
                        purpose,
                    )
                    .await;
                }
                let mut detail_cache: std::collections::HashMap<String, serde_json::Value> =
                    std::collections::HashMap::new();
                let mut serenity_seed: Vec<(String, String, Option<String>)> = Vec::new();
                // ── 逐档先验 + 置信灵敏度（口径统一，2026-10-02）──
                // 本链原先把工作流 LLM 给的候选评分**直接落库**、`priorSource` 硬编码
                // `absent` ⇒ 那个 confidence 是「候选有多符合瓶颈策略」的**评分**，不是
                // 「该档上涨胜率」；而分析链 `horizon_decisions[档].confidence` 是胜率
                // ⇒ UI 并排展示时「荐股 78 vs 分析 46」是两个不同量纲的数在比（实测
                // 002812：荐股 78 / 分析中期 40.1）。
                // 现改为与**另外两条链**同一份实现：`candidate_score_to_win_rate`
                // = 逐档先验 → logit 合成 → `snr_confidence` 时间折算。
                // 取数复用 `load_reco_served_vars`（演化权重覆盖过的变量表），
                // 失败**不阻断**趋势智选主流程，退化为「无先验 ⇒ 纯评分 + absent」。
                // 取数走 **engine 层**（`reco_loop` / `horizon_prior`），不经
                // `commands::stock_analysis` —— 后者会撞分层护栏 `commands-no-sibling-call`
                //（`src/commands/**` 不得跨模块互调）。这也是那两个函数被下沉到 engine 的原因。
                let served_vars =
                    axagent_analysis_engine::recommender::reco_loop::load_reco_served_vars_db(db)
                        .await
                        .unwrap_or_else(|e| {
                            tracing::warn!(
                                "[serenity] served vars 取数失败 ⇒ 逐档先验缺失，置信度退回纯候选评分: {e}"
                            );
                            Vec::new()
                        });
                let prior_kappa = served_vars
                    .iter()
                    .find(|(k, _)| k == "horizon_prior_kappa")
                    .and_then(|(_, v)| v.as_f64())
                    .unwrap_or(axagent_analysis_engine::horizon_prior::DEFAULT_KAPPA);
                let horizon_prior =
                    axagent_analysis_engine::horizon_prior::horizon_prior_from_db(db, prior_kappa)
                        .await;
                if horizon_prior.is_none() {
                    tracing::warn!(
                        "[serenity] 逐档先验取不到 ⇒ confidence 退回纯候选评分并标 absent（不假装合成过）"
                    );
                }
                let conf_sensitivity = served_vars
                    .iter()
                    .find(|(k, _)| k == "reco_conf_sensitivity")
                    .and_then(|(_, v)| v.as_f64())
                    .unwrap_or(1.0);
                // 去重：同一 stock_code 只保留第一个（置信度最高的）候选
                let mut seen_codes: std::collections::HashSet<String> =
                    std::collections::HashSet::new();
                for c in candidate_list.iter() {
                    let code = c["stock_code"].as_str().unwrap_or("");
                    let name = c["stock_name"].as_str().unwrap_or("");
                    let conf_input = candidate_score_input(c);
                    if code.is_empty() {
                        continue;
                    }
                    // 去重：跳过已处理的 code
                    if !seen_codes.insert(code.to_string()) {
                        tracing::debug!("[serenity] 跳过重复候选（{}）: 保留首次出现", code,);
                        continue;
                    }
                    // 构造完整 RecoPick JSON（与 types.rs 中 camelCase 一致）
                    // 价格修复（2026-09-10）：工作流候选由 LLM 产出，不保证有价格/入场/止损字段
                    // （LLM 不产价格，此前一律填 0，前端展示残缺）。落库前抓实时行情，
                    // 按 SerenityStrategy::scan_one 的默认参数（entry ±5% / stop 0.80 /
                    // target 1.30）计算，行情失败时保持 0 并告警。
                    // client 复用 inner 入参（&AStockClient）。
                    // 上方的 `AS_OF.scope` 只罩住 `run_workflow`，执行到这里已退出作用域 ⇒ 裸调用
                    // 会拿**今日**行情给回放票算入场/止损。逐点重挂本轮截止日，与节点侧同口径。
                    let quote = as_of::AS_OF.scope(as_of_ctx, client.get_quote(code)).await.ok();
                    let price = quote.as_ref().map(|q| q.price).unwrap_or(0.0);
                    // ── 止损/目标/仓位/建仓带：与荐股策略链**同一实现**（Phase R-D + Q1/Q2）──
                    // 旧形态：本链按 `price * serenity_stop_mult` 出固定百分比止损，且恒 mid 一档 ⇒
                    // 同一张 reco_picks、同一个趋势智选历史列表里并存两套风控口径 + 两套档位集合。
                    // 现：每个候选按 serenity 的出票档（mid、long）**各落一行**，h 取该档权威天数，
                    // 止损/目标 = k·σ_daily·√h，仓位 = min(候选上限, 风险预算) × 成本拖累，
                    // 建仓带 = 该档止损距离的一半（Q1 裁定 B）。
                    // σ 不可得 ⇒ 显式退回固定乘数并标来源，不伪装成波动率口径。
                    let closes = as_of::AS_OF
                        .scope(
                            as_of_ctx,
                            axagent_analysis_engine::recommender::risk::daily_closes(client, code),
                        )
                        .await;
                    // 候选的 positionPct **已是**最终建议权重（不是策略 base），故不再乘置信 ——
                    // 乘了就是把「策略链 base×置信」的公式套到已经乘过置信的数上，二次折扣。
                    let cap_position = c.get("positionPct").and_then(|v| v.as_f64()).unwrap_or(5.0);
                    let reasons_base: Vec<String> = c
                        .get("reasons")
                        .and_then(|v| v.as_array())
                        .map(|a| {
                            a.iter()
                                .filter_map(|v| v.as_str().map(|s| s.to_owned()))
                                .collect::<Vec<_>>()
                        })
                        .unwrap_or_default();
                    let holding_days_input = c.get("holdingDays").and_then(|v| v.as_i64());
                    let band_stop_pct = (1.0 - serenity_stop_mult) * 100.0;
                    let band_target_pct = (serenity_target_mult - 1.0) * 100.0;
                    for tier in serenity_tiers {
                        let tier_period = tier.as_str();
                        let tier_days = tier.default_holding_days() as i64;
                        let h_days = tier_days as usize;
                        // ── 口径统一（2026-10-02）：候选评分 → 该档上涨胜率 ──
                        // 与荐股策略链（`recommender/mod.rs` 的 R-C 段）、分析决策链
                        // （`portfolio-mgr.rhai` 的 `pm_snr_confidence`）**同一份实现**：
                        // 逐档先验 → logit 合成 → `snr_confidence` 时间折算。
                        // 先验不可得 ⇒ 原样评分 + `absent`（不假装合成过）。
                        let (conf, prior_source) = axagent_analysis_engine::recommender::scoring::candidate_score_to_win_rate(
                            conf_input,
                            horizon_prior.as_ref().and_then(|j| j.get(tier_period)),
                            conf_sensitivity,
                            tier_days as u32,
                            axagent_analysis_engine::reflection_stats::SNR_ANCHOR as u32,
                        );
                        let vol_stop = closes.as_ref().and_then(|closes| {
                            axagent_analysis_engine::recommender::risk::stop_pct(
                                closes,
                                h_days,
                                reco_stop_k1,
                            )
                        });
                        let vol_target = closes.as_ref().and_then(|closes| {
                            axagent_analysis_engine::recommender::risk::target_pct(
                                closes,
                                h_days,
                                reco_target_k2,
                            )
                        });
                        let (used_stop_pct, stop_src) = match vol_stop {
                            Some(v) => (v, "vol"),
                            None => (band_stop_pct, "fallback_pct"),
                        };
                        let used_target_pct = vol_target.unwrap_or(band_target_pct);
                        // Q1=B：建仓带 = 该档止损距离的一半 —— 推导收在 `risk::entry_band_pct`，
                        // 与荐股策略链共用同一个函数（两链各自写一遍正是本轮要消灭的形态）。
                        // 连止损乘数都不可用时才退模板 ±range，来源照实标出。
                        let (entry_half_pct, entry_src) =
                            axagent_analysis_engine::recommender::risk::entry_band_pct(
                                vol_stop,
                                band_stop_pct,
                                serenity_entry_range * 100.0,
                            );
                        let (entry_low, entry_high, stop_loss, target_price) = if price > 0.0 {
                            (
                                price * (1.0 - entry_half_pct / 100.0),
                                price * (1.0 + entry_half_pct / 100.0),
                                price * (1.0 - used_stop_pct / 100.0),
                                price * (1.0 + used_target_pct / 100.0),
                            )
                        } else {
                            tracing::warn!("[serenity] {code}: 行情获取失败，价格字段保持 0");
                            (0.0, 0.0, 0.0, 0.0)
                        };
                        let budget =
                            axagent_analysis_engine::recommender::risk::risk_budget_position(
                                used_stop_pct,
                                reco_risk_budget_pct,
                            );
                        let (pos_before_drag, pos_src) = match budget {
                            Some(b) => (cap_position.min(b), "risk_budget"),
                            None => (cap_position, "fallback_base"),
                        };
                        let drag = axagent_analysis_engine::recommender::risk::cost_drag_factor(
                            used_target_pct,
                            reco_round_trip_cost_pct,
                        );
                        let position_pct = (pos_before_drag * drag).clamp(0.0, 95.0);
                        let mut pick_reasons = reasons_base.clone();
                        pick_reasons.push(format!(
                            "风控({tier_period} {h_days} 天): 止损 {used_stop_pct:.2}% ({stop_src}) · 目标 {used_target_pct:.2}% · 建仓带 ±{entry_half_pct:.2}% ({entry_src}) · 仓位上限 {cap_position:.1}% → 风险预算 {:.1}% × 成本拖累 {drag:.2}",
                            budget.unwrap_or(0.0)
                        ));
                        // 逐档先验只在荐股策略链合成（`mod.rs` 的 R-C 段）；本链 confidence 是工作流
                        // 候选评分 ⇒ 沿用既有词表 `absent` 声明「本次未吸收该档先验」，不新造说法。
                        let mut pick_risk_notes: Vec<String> = Vec::new();
                        if stop_src == "fallback_pct" {
                            pick_risk_notes.push(format!(
                                "⚠ 未取到日线波动率 ⇒ {tier_period} 档止损退回固定乘数 ×{serenity_stop_mult}（stopSource=fallback_pct，非波动率口径）"
                            ));
                        }
                        // 持有期只认该档权威天数（旧缺陷形态：period=mid 却 holding_days=20，
                        // 反思按天判成熟即错位）。候选自带值与当前档不符 ⇒ 声明后按权威落库，
                        // 不静默改写；另一档更是无从沿用。
                        let holding_days = if holding_days_input == Some(tier_days) {
                            tier_days
                        } else {
                            if let Some(d) = holding_days_input {
                                pick_reasons.push(format!(
                                    "候选持有期 {d} 天与 {tier_period} 档权威 {tier_days} 天不一致 ⇒ 按权威落库"
                                ));
                            }
                            tier_days
                        };
                        let pick_data_val = serde_json::json!({
                            "stockCode": code,
                            "stockName": name,
                            "style": "serenity",
                            "strategy_type": c.get("strategy_type").and_then(|v| v.as_str()).unwrap_or("bottleneck"),
                            "period": tier_period,
                            "price": price,
                            "entryLow": entry_low,
                            "entryHigh": entry_high,
                            "stopLoss": stop_loss,
                            "targetPrice": target_price,
                            "positionPct": position_pct,
                            "holdingDays": holding_days,
                            "confidence": conf,
                            "reasons": pick_reasons,
                            "riskNotes": pick_risk_notes,
                            "secondaryStyles": [],
                            // 风控来源键与 `recommender/types.rs` 的 RecoPick 同名同义（camelCase）
                            "stopSource": stop_src,
                            "positionSource": pos_src,
                            "entrySource": entry_src,
                            // 逐档先验来源（`pooled`/`shrunk`/`own`/`neutral_default`），
                            // 或 `absent`（先验取不到 ⇒ confidence 是原始候选评分）。
                            // 2026-10-02 前此处硬编码 `absent`，与另外两条链的词表脱节。
                            "priorSource": prior_source,
                            "synthetic": false,
                        });
                        // 逐档展开的实时候选：克隆原始候选 + 覆盖逐档字段，与前端
                        // `restoreCandidate`（`seed_pool_json` 铺底 + `pick_data` 覆盖）
                        // 解出同形。confidence 刻意不覆盖 —— 恢复路径取的是原始候选评分
                        // （`c` 的 confidence），此处保持同源，免得两路呈现又分叉。
                        // winRate 单独携带该档折算胜率（= reco_picks.confidence 口径）：
                        // 时间旅行批次合并荐股/趋势智选候选去重时，必须与 live 池扫描读到
                        // 的落库胜率同量纲，不能拿原始评分跟胜率比大小。
                        let mut live_candidate = (*c).clone();
                        if let Some(obj) = live_candidate.as_object_mut() {
                            obj.insert("period".to_string(), serde_json::json!(tier_period));
                            obj.insert("holdingDays".to_string(), serde_json::json!(holding_days));
                            obj.insert("price".to_string(), serde_json::json!(price));
                            obj.insert("stopLoss".to_string(), serde_json::json!(stop_loss));
                            obj.insert("targetPrice".to_string(), serde_json::json!(target_price));
                            obj.insert("entryLow".to_string(), serde_json::json!(entry_low));
                            obj.insert("entryHigh".to_string(), serde_json::json!(entry_high));
                            obj.insert("positionPct".to_string(), serde_json::json!(position_pct));
                            obj.insert("stopSource".to_string(), serde_json::json!(stop_src));
                            obj.insert("entrySource".to_string(), serde_json::json!(entry_src));
                            obj.insert("winRate".to_string(), serde_json::json!(conf));
                            obj.insert("generatedAt".to_string(), serde_json::json!(now_str));
                        }
                        live_candidates.push(live_candidate);
                        // 持久化到 reco_picks。style 统一为 "serenity" 便于历史过滤；
                        // 策略子类型（bottleneck/policy/earnings…）仍在 pick_data.strategy_type 里。
                        // ⚠ 必须走 `persist_reco_pick`（dao 的**同业务键覆盖**）—— 库级唯一索引
                        // `uq_reco_picks_business_key` 的键是 `(as_of_date, purpose, period,
                        // stock_code, style)`，**不含 generated_at**：同票同档重跑（今日再跑一次、
                        // 或之前已出过该票）时现造主键 INSERT 必撞约束，前端表现为
                        // 「筛选已完成，但结果未能保存到历史记录」。
                        let seed_pool_json = serde_json::to_string(c).unwrap_or_default();
                        let pick = axagent_dao::repo::reco_pool::NewRecoPick {
                            period: tier_period.to_string(),
                            stock_code: code.to_string(),
                            stock_name: name.to_string(),
                            style: "serenity".to_string(),
                            // `reco_picks.confidence` 是 i32（列类型）；折算函数给 u8（胜率 0-100）
                            confidence: conf as i32,
                            synthetic: false,
                            pick_data: Some(
                                serde_json::to_string(&pick_data_val).unwrap_or_default(),
                            ),
                        };
                        let batch_key = axagent_dao::repo::reco_pool::BatchKey {
                            generated_at: &now_str,
                            as_of_date: serenity_as_of.as_deref(),
                            purpose: serenity_purpose,
                        };
                        // 回放**同样落库**（2026-10-12 裁定：as-of 候选必须持久化，否则回放
                        // 结束后无样本可供反思）。代际写在每行的 `as_of_date` 上 ⇒ 与实盘票
                        // 同表但不混码；只读实盘的下游（趋势智选历史面板、退出信号 / 反馈
                        // 刷新、缓存回读）必须带 `as_of_date IS NULL` 谓词。
                        if let Err(e) = crate::commands::_shared_reco_pool::persist_reco_pick(
                            db,
                            &pick,
                            Some(seed_pool_json.as_str()),
                            &batch_key,
                        )
                        .await
                        {
                            tracing::warn!(
                                "[serenity] 写入 reco_picks 失败 ({code} {tier_period}): {e}"
                            );
                            persistence_success = false;
                            // 只保留**首个**失败的结构化三元组；多只失败时上方逐只 `warn!` 已记录全量原因。
                            if persistence_stock_code.is_empty() {
                                persistence_stock_code = code.to_string();
                                persistence_detail = e;
                            }
                        }
                    }
                    // 构建全量数据缓存
                    detail_cache.insert(
                        code.to_string(),
                        serde_json::json!({
                            "serenity_score": c["serenity_score"],
                            "strategy_type": c.get("strategy_type").and_then(|v| v.as_str()).unwrap_or("bottleneck"),
                            "catalysts": c["catalysts"],
                            "exit_signals": c["exit_signals"],
                            "attention_metrics": c["attention_metrics"],
                            "bottleneck_product": c["bottleneck_product"],
                            "primary_risk": c["primary_risk"],
                            "relevance": c["relevance"],
                            // 这里是**候选本身**的原始评分（不是该档胜率）：该缓存描述
                            // 「这个候选有多符合瓶颈策略」，供策略链/展示读取，故不折算。
                            "confidence": conf_input,
                        }),
                    );
                    // 构建种子列表
                    serenity_seed.push((code.to_string(), name.to_string(), None));
                }
                // 同步到全局种子 + 全量数据缓存
                // 全局 serenity 种子会被**智能荐股的 live 扫描**消费（`recommender` 的
                // `get_serenity_seed`）⇒ 回放写入会把历史候选带进下一次实盘池，只在 live 同步。
                // 注意这与上面的落库裁定不冲突：落库是「持久化 + 代际标记」，这里是**进程内
                // 可变状态**，没有代际可打，一旦被回放写过就分不回来，所以继续只在 live 写。
                if !serenity_seed.is_empty() && as_of_ctx.is_none() {
                    axagent_analysis_engine::recommender::set_serenity_seed(serenity_seed);
                    axagent_analysis_engine::recommender::set_serenity_candidate_cache(
                        detail_cache,
                    );
                }
            }

            // 逐档展开后的对外候选：展开为空（无候选 / 无有效行）时退回原始数组，
            // 不让「落库 best-effort」的异常连带把候选呈现清空。
            let live_candidate_array = if live_candidates.is_empty() {
                candidate_array.clone()
            } else {
                serde_json::Value::Array(live_candidates)
            };

            // 持久化完成后 emit completed 事件
            let persistence_status = if persistence_success {
                "completed"
            } else {
                "partial_failure"
            };
            if !persistence_success {
                tracing::warn!(
                    "[serenity] 持久化部分失败: 写入 {} 失败: {}",
                    persistence_stock_code,
                    persistence_detail
                );
            }
            // v47: 根据是否有用户主题判断 source
            let source = if themes.as_ref().is_some_and(|t| !t.is_empty()) {
                "user"
            } else {
                "auto"
            };
            // 批次静默模式（app=None）不发 completed 事件；invoke 返回值仍是批次取数来源。
            if let Some(app_h) = &app_h {
                let _ = app_h.emit(
                    IpcEventName::SerenityScreeningCompleted.as_str(),
                    serde_json::json!({
                        "workflowId": wf_id_ret,
                        "runId": event_run_id.clone(),
                        "status": persistence_status,
                        // 逐档展开（选项 B）：前端 `p.candidates` 直接就是含 period 的逐档候选，
                        // `result` 同步为其数组形态，避免回退分支又拿到无档位的原始候选。
                        "result": live_candidate_array,
                        "candidates": live_candidate_array,
                        // 本轮 as-of 降级归因（live 恒空数组），与荐股面板同一字段名、同一形态
                        "asofDegradations": asof_degradations,
                        "asOfDate": run_as_of_date,
                        "trends": trends_list,
                        "emptyReason": empty_reason,
                        "source": source,
                        "persistenceCode": if persistence_success {
                            serde_json::Value::Null
                        } else {
                            serde_json::json!(wf_err::PERSIST_FAILED)
                        },
                        "persistenceStockCode": if persistence_success {
                            serde_json::Value::Null
                        } else {
                            serde_json::json!(persistence_stock_code)
                        },
                        "persistenceError": if persistence_success {
                            serde_json::Value::Null
                        } else {
                            serde_json::json!(persistence_detail)
                        },
                    }),
                );
            }

            Ok(serde_json::json!({
                "status": "completed",
                // invoke 返回值与事件 payload 同源（逐档展开），否则非 Tauri 环境
                // 或事件未到达时，`r.candidates` 又会退回无档位的原始候选。
                "candidates": live_candidate_array,
                "asofDegradations": asof_degradations,
                "asOfDate": run_as_of_date,
                "trends": trends_list,
                "emptyReason": empty_reason,
                "source": source,
            }))
        },
        Err(e) => {
            // 结构化码：复用 `core.rs::workflow_error_code`（覆盖 `WorkflowError` 全部 10 个变体，
            // 故**无需新增码**）。与 `error` 的分工：**码负责判定与本地化、`error` 负责技术详情** ——
            // 后者是 `WorkflowError::Display` 原文中文化后的整句，非中文界面不该只看到它。
            let (code, _category) = super::core::workflow_error_code(&e);
            let err_msg = format!("Serenity 筛选工作流失败: {e}");
            // 批次静默模式（app=None）不发 failed 事件；错误照常通过 Err 返回给批次。
            if let Some(app_h) = &app_h {
                let _ = app_h.emit(
                    IpcEventName::SerenityScreeningCompleted.as_str(),
                    serde_json::json!({
                        "workflowId": wf_id_ret,
                        "runId": event_run_id.clone(),
                        "status": "failed",
                        "error": err_msg,
                        "code": code,
                    }),
                );
            }
            Err(err_msg)
        },
    }
}

/// 刷新 Serenity 候选的退出信号（Phase 3 持续监控）
/// 加载最近一次 Serenity 筛选的候选列表，逐个检查退出条件
/// 支持 as_of_date 参数用于回放模式
#[agent_command(domain = "finance", safety = Safe, call_mode = StateOnly, description =  "刷新Serenity退出信号")]
#[tauri::command]
pub async fn refresh_serenity_exit_signals(
    state: State<'_, AppState>,
    as_of_date: Option<String>,
) -> Result<serde_json::Value, String> {
    // 如果指定了 as_of_date，在 as-of 作用域内执行
    if let Some(ref date_str) = as_of_date {
        let as_of_ctx = parse_asof_param(Some(date_str.clone())).map_err(|e| {
            ErrorResponse::new(wf_err::INTERNAL).with_detail(format!("解析 as_of_date 失败: {e}"))
        })?;
        let exec = async { do_refresh_exit_signals(&state).await };
        return as_of::AS_OF.scope(as_of_ctx, exec).await;
    }
    do_refresh_exit_signals(&state).await
}

async fn do_refresh_exit_signals(state: &State<'_, AppState>) -> Result<serde_json::Value, String> {
    use axagent_entities::reco_picks;

    let db = state.harness.db();
    // 加载最近 50 条 Serenity 候选（按 created_at 降序）
    let picks = reco_picks::Entity::find()
        .filter(reco_picks::Column::Style.eq("serenity"))
        // 代际过滤：退出信号服务的是**实盘持仓**，回放候选没有真实仓位，
        // 让它们进这一轮会按历史价算出"该止损"的信号推给用户。
        .filter(reco_picks::Column::AsOfDate.is_null())
        .order_by_desc(reco_picks::Column::CreatedAt)
        .all(db)
        .await
        .map_err(|e| {
            ErrorResponse::new(wf_err::INTERNAL).with_detail(format!("查询 Serenity 候选失败: {e}"))
        })?;
    // 只取最近 50 条
    let picks: Vec<_> = picks.into_iter().take(50).collect();

    let client = &state.astock_client;
    let mut results = Vec::new();

    for pick in &picks {
        let stop_loss = pick.seed_pool_json.as_ref().and_then(|seed_json| {
            serde_json::from_str::<serde_json::Value>(seed_json)
                .ok()
                .and_then(|v| v["stop_loss"].as_f64())
        });

        // 获取当前行情
        let quote = client.get_quote(&pick.stock_code).await.ok();
        let price = quote.as_ref().map(|q| q.price).unwrap_or(0.0);

        // 搜索退出相关新闻
        let news = client
            .search_news(&format!("{} 技术替代 产能过剩", pick.stock_code), 5)
            .await
            .unwrap_or_default();
        let has_disruption_news = news.len() >= 2;

        // 检查毛利率趋势
        let margin_declining = client
            .get_financials(&pick.stock_code)
            .await
            .ok()
            .and_then(|f| {
                if f.len() >= 2 {
                    let curr = f[0].gross_margin.unwrap_or(0.0);
                    let prev = f[1].gross_margin.unwrap_or(0.0);
                    Some(prev > 0.0 && curr < prev * 0.85)
                } else {
                    None
                }
            })
            .unwrap_or(false);

        // 判断退出紧迫度
        let stop_loss_hit = stop_loss.map(|sl| price < sl).unwrap_or(false);
        let urgency = if stop_loss_hit || (has_disruption_news && margin_declining) {
            "exit_now"
        } else if has_disruption_news || margin_declining {
            "caution"
        } else {
            "no_urgency"
        };

        results.push(serde_json::json!({
            "stock_code": pick.stock_code,
            "stock_name": pick.stock_name,
            "current_price": price,
            "stop_loss_hit": stop_loss_hit,
            "has_disruption_news": has_disruption_news,
            "margin_declining": margin_declining,
            "exit_urgency": urgency,
            "confidence": pick.confidence,
        }));
    }

    Ok(serde_json::json!({
        "status": "completed",
        "checked_count": results.len(),
        "exit_now_count": results.iter().filter(|r| r["exit_urgency"] == "exit_now").count(),
        "caution_count": results.iter().filter(|r| r["exit_urgency"] == "caution").count(),
        "candidates": results,
    }))
}

/// 汇总趋势智选候选的当前浮盈（**只读**：不落库、不改权重、不产反思行）。
///
/// ⚠ 2026-10-20 更正描述：旧文案写「跟踪推荐表现、验证催化剂、**调优权重**」，但
/// [`do_feedback_loop`] 全文零写库，只把结果返回给面板显示 —— 顶着「闭环/调优」的名目
/// 会让 Agent 侧以为调权已生效。真实的自学习入口是 07:00 决策回测（写 `decision_validations`）。
#[agent_command(domain = "finance", safety = Caution, call_mode = StateOnly, description =  "汇总智选候选当前浮盈（只读）")]
#[tauri::command]
pub async fn refresh_serenity_feedback(
    state: State<'_, AppState>,
    as_of_date: Option<String>,
) -> Result<serde_json::Value, String> {
    // 如果指定了 as_of_date，在 as-of 作用域内执行
    if let Some(ref date_str) = as_of_date {
        let as_of_ctx = parse_asof_param(Some(date_str.clone())).map_err(|e| {
            ErrorResponse::new(wf_err::INTERNAL).with_detail(format!("解析 as_of_date 失败: {e}"))
        })?;
        let exec = async { do_feedback_loop(&state).await };
        return as_of::AS_OF.scope(as_of_ctx, exec).await;
    }
    do_feedback_loop(&state).await
}

async fn do_feedback_loop(state: &State<'_, AppState>) -> Result<serde_json::Value, String> {
    use axagent_entities::reco_picks;

    let db = state.harness.db();
    // 固定取过去 30 天的 Serenity 候选，避免新工作流产出的记录不断顶替旧样本
    let thirty_days_ago = chrono::Utc::now() - chrono::Duration::days(30);
    let cutoff = thirty_days_ago.to_rfc3339();
    let picks = reco_picks::Entity::find()
        .filter(reco_picks::Column::Style.eq("serenity"))
        .filter(reco_picks::Column::CreatedAt.gte(cutoff))
        // 代际过滤：本反馈环算的是实盘出票的真实表现；回放批次的数据由
        // as-of 闭环（decision_validations 带 as_of_date）单独成组，不混进这一分母。
        .filter(reco_picks::Column::AsOfDate.is_null())
        .order_by_desc(reco_picks::Column::CreatedAt)
        .all(db)
        .await
        .map_err(|e| {
            ErrorResponse::new(wf_err::INTERNAL).with_detail(format!("查询 Serenity 候选失败: {e}"))
        })?;

    let client = &state.astock_client;
    let mut performances = Vec::new();

    for (idx, pick) in picks.iter().enumerate() {
        // 提取推荐日期（从 created_at 取前 10 字符 = YYYY-MM-DD）
        let rec_date = pick.created_at.as_str().get(..10).unwrap_or("2025-01-01");

        // 提取候选全量数据（seed_pool_json 存储的是候选 JSON 对象）
        let detail = pick
            .seed_pool_json
            .as_ref()
            .and_then(|j| serde_json::from_str::<serde_json::Value>(j).ok());

        if detail.is_none() {
            tracing::info!(
                "[serenity-feedback] pick={} seed_pool_json=None，跳过（历史数据）",
                pick.stock_code
            );
            // 历史数据：seed_pool_json 为 None，无法计算催化剂，跳过
            continue;
        }

        // 限流：每处理 1 条记录后延迟 500ms，避免触发东方财富 API 限流
        if idx > 0 {
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        }

        // 计算表现：获取推荐日至今的 K 线
        tracing::info!("[serenity-feedback] pick={} 获取 K 线", pick.stock_code);
        let entry_kline = match client.get_klines(&pick.stock_code, "daily", 120).await {
            Ok(k) => {
                tracing::info!(
                    "[serenity-feedback] pick={} K 线成功, {} 条",
                    pick.stock_code,
                    k.len()
                );
                Some(k)
            },
            Err(e) => {
                tracing::warn!("[serenity-feedback] pick={} K 线失败: {e:?}", pick.stock_code);
                None
            },
        };
        let (entry_price, used_fallback) = entry_kline
            .as_ref()
            .and_then(|k| {
                // 优先找推荐日当天的 K 线
                k.iter().find(|k| k.date.starts_with(rec_date)).map(|k| (k.close, false))
                // 找不到则用倒数第二根（推荐日 K 线不在时避免与 current_price 撞车）
                .or_else(|| {
                    if k.len() >= 2 {
                        Some((k[k.len()-2].close, true))
                    } else {
                        k.last().map(|k| (k.close, true))
                    }
                })
            })
            .unwrap_or((0.0, false));
        if entry_price <= 0.0 {
            tracing::warn!(
                "[serenity-feedback] pick={} entry_price=0 (rec_date={}, kline_count={})",
                pick.stock_code,
                rec_date,
                entry_kline.as_ref().map(|k| k.len()).unwrap_or(0)
            );
        } else {
            tracing::info!(
                "[serenity-feedback] pick={} entry_price={} (rec_date={}){}",
                pick.stock_code,
                entry_price,
                rec_date,
                if used_fallback {
                    " [参考值:推荐日K线未收盘，取前一日]"
                } else {
                    ""
                },
            );
        }

        let current_quote = match client.get_quote(&pick.stock_code).await {
            Ok(q) => {
                tracing::info!(
                    "[serenity-feedback] pick={} get_quote 成功: price={}",
                    pick.stock_code,
                    q.price
                );
                Some(q)
            },
            Err(e) => {
                // 打印完整错误链，帮助定位 error sending request 的根因
                tracing::warn!(
                    "[serenity-feedback] pick={} get_quote 失败: {e:#?}",
                    pick.stock_code
                );
                // 同时打印 source chain
                let mut src: Option<&dyn std::error::Error> = std::error::Error::source(&e);
                while let Some(s) = src {
                    tracing::warn!("[serenity-feedback]   Caused by: {s:#?}");
                    src = s.source();
                }
                None
            },
        };
        let current_price = current_quote.as_ref().map(|q| q.price).unwrap_or(0.0);
        let return_pct = if entry_price > 0.0 && current_price > 0.0 {
            (current_price - entry_price) / entry_price * 100.0
        } else {
            0.0
        };

        // 验证催化剂：从 detail 中提取（兼容多种字段名）
        let catalysts_info = detail
            .as_ref()
            .map(|d| {
                // 尝试多种可能的字段名
                let arr = d
                    .get("catalysts")
                    .or_else(|| d.get("catalyst"))
                    .or_else(|| d.get("catalyst_list"));
                arr.and_then(|v| v.as_array()).map(|a| a.len()).unwrap_or(0)
            })
            .unwrap_or(0);
        // 同时尝试嵌套路径：有些工作流输出把 catalysts 放在 params.catalysts
        let catalysts_info = if catalysts_info == 0 {
            detail
                .as_ref()
                .and_then(|d| {
                    d.get("params")
                        .and_then(|p| p.get("catalysts"))
                        .and_then(|v| v.as_array())
                        .map(|a| a.len())
                })
                .unwrap_or(0)
        } else {
            catalysts_info
        };

        // 搜索该股相关新闻作为催化剂验证的 proxy
        let catalyst_news =
            match client.search_news(&format!("{} 财报 量产 订单", pick.stock_code), 5).await
            {
                Ok(news) => {
                    tracing::info!(
                        "[serenity-feedback] pick={} search_news 成功: {} 条",
                        pick.stock_code,
                        news.len()
                    );
                    news
                },
                Err(e) => {
                    tracing::warn!(
                        "[serenity-feedback] pick={} search_news 失败: {e:#?}",
                        pick.stock_code
                    );
                    Vec::new()
                },
            };
        let catalysts_verified_count = catalyst_news.len().min(catalysts_info);

        performances.push(serde_json::json!({
            "id": pick.id,
            "stock_code": pick.stock_code,
            "stock_name": pick.stock_name,
            "confidence": pick.confidence,
            "recommend_date": rec_date,
            "entry_price": entry_price,
            "current_price": current_price,
            "return_pct": (return_pct * 100.0).round() / 100.0,
            "is_profitable": return_pct > 0.0,
            "return_pending": used_fallback,
            "catalysts": serde_json::json!({
                "total": catalysts_info,
                "verified": catalysts_verified_count,
            }),
        }));
    }

    // 计算汇总指标
    let profitable =
        performances.iter().filter(|p| p["is_profitable"].as_bool().unwrap_or(false)).count();
    let total = performances.len();
    let avg_return = if total > 0 {
        performances.iter().map(|p| p["return_pct"].as_f64().unwrap_or(0.0)).sum::<f64>()
            / total as f64
    } else {
        0.0
    };

    Ok(serde_json::json!({
        "status": "completed",
        "total": total,
        "profitable_count": profitable,
        "win_rate": if total > 0 { (profitable as f64 / total as f64 * 100.0).round() / 100.0 } else { 0.0 },
        "avg_return_pct": (avg_return * 100.0).round() / 100.0,
        "performances": performances,
    }))
}

#[cfg(test)]
mod serenity_extract_tests {
    use super::*;
    use axagent_harness::types::{ChatResponse, ContentBlock};
    use axagent_runtime_core::DefaultResponseNormalizer;

    // ── 0) 评分输入优先级：确定性 serenity_score 必须压过 LLM 自报 confidence ──
    #[test]
    fn candidate_score_input_prefers_deterministic_score_then_falls_back() {
        // 两者都在 ⇒ 用确定性评分
        assert_eq!(
            candidate_score_input(&serde_json::json!({
                "serenity_score": 77.0, "confidence": 66.0
            })),
            77
        );
        // 旧模板/旧记录没有 serenity_score ⇒ 回落 confidence
        assert_eq!(candidate_score_input(&serde_json::json!({"confidence": 66.0})), 66);
        // 两者都缺 ⇒ 中性 50（不是 0）
        assert_eq!(candidate_score_input(&serde_json::json!({})), 50);
        // 越界钳制；非数值 serenity_score 不得冒充 0，回落 confidence
        assert_eq!(candidate_score_input(&serde_json::json!({"serenity_score": 150.0})), 100);
        assert_eq!(candidate_score_input(&serde_json::json!({"serenity_score": -5.0})), 0);
        assert_eq!(
            candidate_score_input(&serde_json::json!({
                "serenity_score": "77", "confidence": 61.0
            })),
            61
        );
    }

    // ── helper：把字符串送进 IR Normalizer 拿到 ContentBlock 列表 ──
    async fn normalize(content: &str) -> Vec<ContentBlock> {
        let resp = ChatResponse {
            id: String::new(),
            model: String::new(),
            content: content.to_string(),
            thinking: None,
            usage: Default::default(),
            tool_calls: None,
        };
        let normalizer = DefaultResponseNormalizer;
        normalizer.normalize(&resp).await
    }

    // ── 1) 标准 tool_json 块：name=submit_candidates，arguments 是数据 ──
    #[tokio::test]
    async fn tool_json_block_extracts_candidates() {
        let content = r#"```tool_json
{"name": "submit_candidates", "arguments": {"candidates": [{"stock_code": "300285", "stock_name": "国瓷材料", "serenity_score": 75}], "summary": "ok"}}
```"#;
        let v = extract_via_normalizer(content).await;
        let v = v.expect("IR 提取应成功");
        let arr = v.get("candidates").and_then(|x| x.as_array()).expect("candidates 应为数组");
        assert_eq!(arr.len(), 1);
        assert_eq!(arr[0]["stock_code"], "300285");
    }

    // ── 2) 普通 json 块（无 name 字段）→ IR 当文本块保留 → extract_json_from_llm_response 兜底 ──
    #[tokio::test]
    async fn plain_json_block_falls_back_to_text_extraction() {
        let content = r#"```json
{"trends": [{"trend_name": "AI 算力散热", "confidence": 80}]}
```"#;
        let v = extract_via_normalizer(content).await;
        let v = v.expect("纯 json 块应能解析");
        let arr = v.get("trends").and_then(|x| x.as_array()).expect("trends 应为数组");
        assert_eq!(arr[0]["trend_name"], "AI 算力散热");
    }

    // ── 3) 截断 JSON（用户日志里 "market_cap_level 混文字" 场景）──
    //     LLM 把思考文字夹进了字符串值；我们的策略是 IR + 文本块内部 JSON 解析，
    //     若破损则返回 None，让上层走降级。
    #[tokio::test]
    async fn truncated_json_returns_none_or_partial() {
        // 模拟用户日志中的破损输出：缺右括号、字符串值被截断
        let content = r#"```json
{
  "candidates": [
    {
      "stock_code": "300285",
      "stock_name": "国瓷材料",
      "market_cap_level": "中盘",
      "serenity_score": 75
    }
  ]
"#;
        // 不抛 panic，要么成功（拿到部分有效 JSON）要么返回 None
        let result = extract_via_normalizer(content).await;
        if let Some(v) = result {
            // 如果能解析，至少应该能拿到 candidates 字段
            assert!(v.get("candidates").is_some() || v.get("stock_code").is_some());
        }
        // None 也是可接受的——上层会走降级
    }

    // ── 4) IR Normalizer 自身：tool_json 块 → ContentBlock::ToolUse ──
    #[tokio::test]
    async fn normalizer_emits_tool_use_for_tool_json() {
        let blocks = normalize(
            r#"```tool_json
{"name": "submit_chain", "arguments": {"trend_name": "AI 算力"}}
```"#,
        )
        .await;
        assert!(
            blocks.iter().any(|b| matches!(b, ContentBlock::ToolUse { .. })),
            "tool_json 块应被 Normalizer 解析为 ToolUse，实际：{:?}",
            blocks
        );
    }

    // ── 5) IR Normalizer：纯文本无代码块 → ContentBlock::Text ──
    #[tokio::test]
    async fn normalizer_passes_plain_text_through() {
        let blocks = normalize("hello world").await;
        assert!(matches!(blocks.as_slice(), [ContentBlock::Text { .. }]));
    }

    // ── 6) extract_agent_output 顶层字段优先级：params > output > content ──
    #[tokio::test]
    pub(crate) async fn extract_agent_output_prefers_top_level_params() {
        let raw = serde_json::json!({
            "content": "ignored",
            "params": {"candidates": [{"stock_code": "1"}]},
            "output": {"should_not": "appear"},
        });
        let v = extract_agent_output(raw).await;
        assert_eq!(v["candidates"][0]["stock_code"], "1");
    }

    // ── 7) extract_agent_output 顶层 candidates 字段直通 ──
    #[tokio::test]
    pub(crate) async fn extract_agent_output_passes_top_level_candidates() {
        let raw = serde_json::json!({
            "candidates": [{"stock_code": "600519"}],
            "content": "ignored",
        });
        let v = extract_agent_output(raw).await;
        let arr = v.as_array().expect("candidates 应直返为数组");
        assert_eq!(arr[0]["stock_code"], "600519");
    }

    // ── 8) 兜底：content 是破损 JSON（无 code fence），返回 None（不走原始对象）──
    //     extract_via_normalizer 内部：直接尝试 `serde_json::from_str(content)` → 失败
    //     所以会从内容中找 ```json``` 失败，最终返回 None。
    #[tokio::test]
    async fn extract_via_normalizer_handles_garbage_input() {
        let v = extract_via_normalizer("not a json at all").await;
        assert!(v.is_none());
    }

    // ── 9) parse_loose_json：标准 JSON 直通 ──
    #[test]
    fn parse_loose_json_accepts_valid() {
        let v = parse_loose_json(r#"{"k": 1}"#);
        assert_eq!(v.expect("应能解析")["k"], 1);
    }

    // ── 10) parse_loose_json：空字符串 → None ──
    #[test]
    fn parse_loose_json_empty_string() {
        assert!(parse_loose_json("").is_none());
        assert!(parse_loose_json("   ").is_none());
    }

    // ── 11) repair_json 第 0 层：字符串值内部未转义引号（2026-09-07 生产故障类别）──
    //     复现 "expected `:`" 场景：内部引号截断字符串，后续 token 被误认为 key
    #[test]
    fn repair_json_escapes_unescaped_inner_quotes() {
        let raw =
            r#"{"candidates": [{"stock_code": "600552", "name": "凯盛"科技"股份", "score": 80}]}"#;
        let repaired = repair_json(raw);
        let v: serde_json::Value = serde_json::from_str(&repaired).expect("内部引号转义后应可解析");
        assert_eq!(v["candidates"][0]["stock_code"], "600552");
        assert_eq!(v["candidates"][0]["name"], "凯盛\"科技\"股份");
        assert_eq!(v["candidates"][0]["score"], 80);
    }

    // ── 12) repair_json 对合法 JSON 零改动（转义过的引号不受影响）──
    #[test]
    fn repair_json_keeps_valid_json_untouched() {
        let raw = r#"{"a": "正常\"转义\"", "b": [1, 2], "c": "说"x"话"}"#;
        let v: serde_json::Value = serde_json::from_str(&repair_json(raw)).expect("修复后应可解析");
        assert_eq!(v["a"], "正常\"转义\"");
        assert_eq!(v["c"], "说\"x\"话");
    }

    // ── 13) 前导 markdown 文本 + 内部引号：组合场景 ──
    #[test]
    fn extract_named_arrays_with_inner_quotes() {
        let text = r#"分析结果如下：
```json
{"summary": "基于"瓶颈"逻辑筛选", "candidates": [{"stock_code": "600552"}]}
```"#;
        let extracted = axagent_kit::utils::extract_json_from_llm_response(text);
        let v = extract_named_arrays(extracted).expect("应提取出 candidates");
        assert_eq!(v["candidates"][0]["stock_code"], "600552");
    }

    // ── 14) parse_error_window：失败点附近原文可见 ──
    #[test]
    fn parse_error_window_shows_context() {
        let text = r#"{"a": 1, "b": ??}"#;
        let e = serde_json::from_str::<serde_json::Value>(text).unwrap_err();
        let w = parse_error_window(text, &e, 10);
        assert!(w.contains("??"), "窗口应包含损坏点，实际: {w}");
    }

    /// strict_mode 信封：tool_json 块严格解析失败后，agent_executor 把整段原文塞进
    /// `report` 并附 `verdict`，content 本身变成合法 JSON、只是换了形状。
    /// 2026-10-02 实证形态：模型在最后一个候选对象边界多打一个 `}`。
    fn strict_mode_envelope() -> serde_json::Value {
        let report = concat!(
            "```tool_json\n",
            r#"{"name": "submit_candidates", "arguments": {"candidates": ["#,
            r#"{"stock_code": "688114", "stock_name": "华大智造", "serenity_score": 68, "#,
            r#""exit_signals": {"overall_exit_urgency": "watch"}},"#,
            r#"{"stock_code": "300676", "stock_name": "华大基因", "serenity_reason": "较低"}}], "#,
            r#""summary": "共筛选2个候选"}}"#,
            "\n```",
        );
        serde_json::json!({
            "report": report,
            "verdict": {"verdict": "偏多", "bull_score": 65, "confidence": 50},
            "__verdict_only": false,
        })
    }

    // ── 15) 修复前形态自证：信封喂给解包前的实现体必须返回 Null（这正是 0 候选的成因）──
    #[test]
    fn content_only_extract_returns_null_on_strict_mode_envelope() {
        let envelope = strict_mode_envelope();
        assert!(
            serenity_extract_from_content(&envelope.to_string()).is_null(),
            "信封形状下实现体必须空手而归，否则 report 解包这一层没有存在的理由"
        );
    }

    // ── 16) report 解包重跑：逐个提取必须回收全部候选（含损坏点之后的）──
    #[test]
    fn extract_recovers_candidates_from_strict_mode_report() {
        let envelope = strict_mode_envelope();
        let node_out = serde_json::json!({"content": envelope.to_string()});
        let got = serenity_extract_from_node(&node_out);
        let arr = got["candidates"].as_array().expect("report 解包后应回收候选");
        assert_eq!(arr.len(), 2, "损坏点之后的候选也必须回收");
        assert_eq!(arr[0]["stock_code"], "688114");
        assert_eq!(arr[1]["stock_code"], "300676");
    }

    // ── 17) 正常形态零影响：首轮命中即返回，不进解包分支 ──
    #[test]
    fn extract_short_circuits_when_candidates_present() {
        let content = r#"{"candidates": [{"stock_code": "600552", "stock_name": "中国海油"}]}"#;
        let node_out = serde_json::json!({"content": content});
        let got = serenity_extract_from_node(&node_out);
        assert_eq!(got["candidates"].as_array().map_or(0, |a| a.len()), 1);
    }

    // ── 18) 解包既拿不到候选、也拿不到模型理由时才保留原判定（信封里是纯散文）──
    #[test]
    fn extract_falls_back_to_first_when_unwrap_yields_nothing() {
        let envelope = serde_json::json!({
            "report": "上游趋势数据缺失，无法识别有效候选标的",
            "verdict": {"verdict": "观望", "confidence": 50},
        });
        let node_out = serde_json::json!({"content": envelope.to_string()});
        let got = serenity_extract_from_node(&node_out);
        assert!(got.is_null(), "既无候选也无 summary 时保持原判定");
    }

    // ── 19) 负控：report 语法完好时不得重复计数 ──
    //      解包层与四层提取链叠加，最坏形态是「同一批候选被两条路径各挖一次」⇒ 候选数翻倍。
    //      完好 report 在解包后的层 0 就命中，压根不进逐对象兜底；这条锁住那个前提。
    #[test]
    fn extract_does_not_double_count_well_formed_report() {
        let report = concat!(
            "```tool_json\n",
            r#"{"name": "submit_candidates", "arguments": {"candidates": "#,
            r#"[{"stock_code": "688114"}, {"stock_code": "300676"}], "summary": "共筛选2个候选"}}"#,
            "\n```",
        );
        let envelope = serde_json::json!({"report": report, "verdict": {"verdict": "偏多"}});
        let node_out = serde_json::json!({"content": envelope.to_string()});
        let got = serenity_extract_from_node(&node_out);
        let arr = got["candidates"].as_array().expect("完好 report 应产出候选");
        assert_eq!(arr.len(), 2, "不得重复计数（每条候选只出现一次）");
        let codes: Vec<&str> = arr.iter().filter_map(|c| c["stock_code"].as_str()).collect();
        assert_eq!(codes, vec!["688114", "300676"]);
        // summary 取模型原文，不是兜底层编的句子
        assert_eq!(got["summary"], "共筛选2个候选");
    }

    // ── 20) 负控：模型真的拒绝出票时，空候选 + 它自己的理由必须原样保留 ──
    //      这条锁的是旧 `has_summary` 分支**本意**想服务的那个场景 —— 旧实现里它永远
    //      服务不到（兜底命中 ⇒ 候选非空），反而把有候选的轮次压成 0 候选 + 一句硬编码
    //      「上游数据不足」。该场景语法完好，走层 0，summary 是模型原文。
    #[test]
    fn declined_round_keeps_model_own_reason_and_empty_candidates() {
        let content = r#"{"candidates": [], "summary": "上游趋势数据缺失，无法识别有效候选标的"}"#;
        let node_out = serde_json::json!({"content": content});
        let got = serenity_extract_from_node(&node_out);
        assert_eq!(got["candidates"].as_array().map_or(99, |a| a.len()), 0);
        assert_eq!(got["summary"], "上游趋势数据缺失，无法识别有效候选标的");
        assert_ne!(
            got["summary"].as_str().unwrap_or_default(),
            "上游数据不足，无法识别有效候选标的",
            "缺席原因必须来自模型原文，不得由提取层合成"
        );
    }

    // ── 21) 负控：report 里是「模型拒绝出票」的完好输出 ⇒ 缺席理由必须活着到界面 ──
    //      这一路 candidates 为空，旧命中判据（只认候选）把 inner 整个丢弃、退回 first=Null，
    //      用户看到「无候选且无原因」，而模型其实写明了为什么（与 #26 修的层 4 同族）。
    #[test]
    fn extract_preserves_model_reason_from_wrapped_decline() {
        // 自证这条锁有电（旧命中判据只认候选 ⇒ 对本夹具必须为空手而归，否则它什么也没锁）。
        let report = concat!(
            "```tool_json\n",
            r#"{"candidates": [], "summary": "三个瓶颈节点均 data_gaps=true，拒绝编造候选"}"#,
            "\n```",
        );
        let inner = serenity_extract_from_content(report);
        assert!(inner.get("summary").is_some(), "夹具必须带模型理由");
        assert!(
            !serenity_has_candidates(&inner),
            "夹具必须落在「候选为空但有理由」那一侧，旧判据才会丢掉它"
        );
        let envelope = serde_json::json!({"report": report, "verdict": {"verdict": "观望"}});
        let node_out = serde_json::json!({"content": envelope.to_string()});
        let got = serenity_extract_from_node(&node_out);
        assert_eq!(got["candidates"].as_array().map_or(99, |a| a.len()), 0, "候选确实为空");
        assert_eq!(got["summary"], "三个瓶颈节点均 data_gaps=true，拒绝编造候选");
    }
}
