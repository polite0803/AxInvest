// SPDX-License-Identifier: AGPL-3.0-only

use serde::{Deserialize, Serialize};

/// 防幻觉锚定配置
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct HallucinationGuardConfig {
    pub enabled: bool,
    /// 引用匹配阈值（0-1），低于此值判定为幻觉
    pub match_threshold: f64,
}

impl Default for HallucinationGuardConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            match_threshold: 0.5,
        }
    }
}

/// 锚定检查结果
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct AnchorResult {
    pub passed: bool,
    pub score: f64,
    pub unverified_claims: Vec<String>,
    pub details: String,
}

/// 检查 LLM 输出中的关键信息是否在源文档中出现
///
/// # 参数
/// - `output`: LLM 的输出文本
/// - `source_context`: 源文档/上下文文本（RAG 检索结果、文档内容等）
/// - `threshold`: 锚定分数阈值（0-1），低于此值判定为幻觉
pub fn check_anchor(output: &str, source_context: &str, threshold: f64) -> AnchorResult {
    let sentences = split_sentences(output);
    let mut unverified = Vec::new();
    let mut verified_count = 0usize;

    for sentence in &sentences {
        let trimmed = sentence.trim();
        if trimmed.len() < 10 {
            continue;
        }

        // 提取句子中的关键术语（兼容中文和英文）
        let key_terms = extract_key_terms(trimmed);

        if key_terms.is_empty() {
            continue;
        }

        // 检查每个关键术语是否出现在 source_context 中
        let source_match = key_terms
            .iter()
            .filter(|w| source_context.contains(w.as_str()))
            .count();
        let match_rate = source_match as f64 / key_terms.len() as f64;

        if match_rate >= 0.5 {
            verified_count += 1;
        } else {
            unverified.push(trimmed.to_string());
        }
    }

    let total_checked = verified_count + unverified.len();
    let score = if total_checked > 0 {
        verified_count as f64 / total_checked as f64
    } else {
        1.0
    };

    let unverified_count = unverified.len();
    AnchorResult {
        passed: score >= threshold,
        score,
        unverified_claims: unverified,
        details: format!(
            "锚定分数: {:.2} (阈值: {}), 未验证句子: {}",
            score, threshold, unverified_count
        ),
    }
}

fn split_sentences(text: &str) -> Vec<String> {
    text.split(['.', '!', '?', '\n'])
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

/// 判断字符是否为 CJK（中日韩统一表意文字）
fn is_cjk(c: char) -> bool {
    matches!(
        c,
        '\u{4E00}'..='\u{9FFF}'   // CJK Unified Ideographs
        | '\u{3400}'..='\u{4DBF}' // CJK Extension A
        | '\u{F900}'..='\u{FAFF}' // CJK Compatibility
        | '\u{2F800}'..='\u{2FA1F}' // CJK Extension B Supplement
    )
}

/// 从文本中提取关键术语，兼容中文和英文
///
/// - 英文：按空白分割，保留长度 > 3 的词
/// - 中文：识别连续 CJK 字符块，短块(2-4字符)保留原样，长块(≥5字符)切分为
///   重叠 2-3 字符窗口——这样 "华如科技基本面估值分析报告" 会生成
///   "华如/如科/科技/科技基/基本/基本面/估值/分析" 等子串，便于在 source_context 中匹配。
fn extract_key_terms(text: &str) -> Vec<String> {
    let mut terms: Vec<String> = Vec::new();

    // 1. 空白分割词（英文/数字/标点组）
    for word in text.split_whitespace() {
        // 过滤纯标点
        let clean: String = word
            .chars()
            .filter(|c| c.is_alphanumeric() || is_cjk(*c))
            .collect();
        if clean.len() > 3 {
            terms.push(clean);
        }
    }

    // 2. CJK 连续字符块 → 滑窗切分
    let mut cjk_run = String::new();
    for ch in text.chars() {
        if is_cjk(ch) {
            cjk_run.push(ch);
        } else if !cjk_run.is_empty() {
            extract_cjk_terms(&cjk_run, &mut terms);
            cjk_run.clear();
        }
    }
    if !cjk_run.is_empty() {
        extract_cjk_terms(&cjk_run, &mut terms);
    }

    // 去重
    terms.sort();
    terms.dedup();
    terms
}

/// 将连续 CJK 字符块切分为关键术语
///
/// - 短块(2-4字符)：整块作为 1 个术语
/// - 长块(≥5字符)：2-3 字符重叠窗口
fn extract_cjk_terms(run: &str, terms: &mut Vec<String>) {
    let chars: Vec<char> = run.chars().collect();
    let n = chars.len();
    if n < 2 {
        return;
    }
    if n <= 4 {
        terms.push(run.to_string());
        return;
    }
    // 2-char 滑窗
    for w in chars.windows(2) {
        terms.push(w.iter().collect());
    }
    // 3-char 滑窗
    for w in chars.windows(3) {
        terms.push(w.iter().collect());
    }
}
