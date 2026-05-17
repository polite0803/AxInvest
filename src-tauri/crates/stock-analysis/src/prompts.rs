//! 从 Markdown 文件加载专家系统提示词。
//!
//! 各专家提示词定义在 `agency_experts/stock-analysis/*.md`，
//! 格式为 YAML frontmatter + Markdown body。

use std::collections::HashMap;
use std::path::PathBuf;

/// 17 个专家 ID（对应 17 个 .md 文件）
pub const EXPERT_IDS: &[&str] = &[
    "market-analyst",
    "sentiment-analyst",
    "news-analyst",
    "fundamentals-analyst",
    "policy-analyst",
    "hot-money-tracker",
    "lockup-watcher",
    "research-analyst",
    "sector-analyst",
    "bull-researcher",
    "bear-researcher",
    "aggressive-debator",
    "conservative-debator",
    "neutral-debator",
    "research-manager",
    "trader",
    "portfolio-manager",
];

/// 从指定目录加载所有专家系统提示词。
///
/// `base_dir` 应为包含 `*.md` 文件的目录路径。
/// 返回从 expert_id 到 body（去掉 YAML frontmatter）的映射。
pub fn load_expert_prompts(base_dir: &str) -> HashMap<String, String> {
    let mut prompts = HashMap::new();

    for id in EXPERT_IDS {
        let path = PathBuf::from(base_dir).join(format!("{id}.md"));
        match std::fs::read_to_string(&path) {
            Ok(content) => {
                let body = extract_body(&content);
                prompts.insert(id.to_string(), body);
                tracing::trace!("已加载专家提示词: {}", id);
            },
            Err(e) => {
                tracing::warn!("未能加载专家提示词文件 {}: {}", path.display(), e);
            },
        }
    }

    tracing::info!("从 {} 加载了 {} 个专家提示词", base_dir, prompts.len());
    prompts
}

/// 从 Markdown 中提取 body（跳过 YAML frontmatter）。
///
/// Frontmatter 由开头的 `---` 行和结尾的 `---` 行界定。
/// 如果没有 frontmatter，则返回整个内容。
fn extract_body(content: &str) -> String {
    if let Some(rest) = content.strip_prefix("---") {
        if let Some(end) = rest.find("\n---") {
            return rest[end + 4..].trim().to_string();
        }
    }
    content.to_string()
}

/// 从已加载的提示词中获取指定专家的分析上下文。
///
/// 如果未找到匹配的 expert_id，返回 `None`，调用方应提供回退。
pub fn get_analyst_context(expert_id: &str, prompts: &HashMap<String, String>) -> Option<String> {
    prompts.get(expert_id).cloned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_body_with_frontmatter() {
        let input = "---\nname: test\ndescription: desc\n---\n\n# Body content\n\nHello world";
        let result = extract_body(input);
        assert_eq!(result, "# Body content\n\nHello world");
    }

    #[test]
    fn test_extract_body_without_frontmatter() {
        let input = "# Just a heading\nSome content";
        let result = extract_body(input);
        assert_eq!(result, "# Just a heading\nSome content");
    }

    #[test]
    fn test_get_analyst_context_found() {
        let mut prompts = HashMap::new();
        prompts.insert("test-expert".to_string(), "你是测试专家".to_string());
        let ctx = get_analyst_context("test-expert", &prompts);
        assert_eq!(ctx, Some("你是测试专家".to_string()));
    }

    #[test]
    fn test_get_analyst_context_not_found() {
        let prompts = HashMap::new();
        let ctx = get_analyst_context("unknown", &prompts);
        assert_eq!(ctx, None);
    }
}
