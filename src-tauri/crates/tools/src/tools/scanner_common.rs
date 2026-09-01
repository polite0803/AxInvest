// SPDX-License-Identifier: AGPL-3.0-only

//! 需求发现扫描器公共工具
//!
//! 集中提供各平台 scanner 共用的能力，避免 18 个 scanner 各写一份、各错一次。
//!
//! ## 合规约束（务必遵守）
//!
//! 本模块**只提供发起「官方开放 API 请求」的辅助能力**：
//! - User-Agent 一律使用 [`SCANNER_USER_AGENT`] 这种**真实自报身份**的标识，
//!   **禁止**伪造 Chrome / Safari 等浏览器指纹伪装真人访问，那属于规避平台
//!   反爬措施，违反多数平台 ToS。
//! - 不具备官方 API 的平台连接器，必须走「未配置凭证即不开工」的门禁
//!   （见 [`require_official_api_credential`]），**禁止**退化为 HTML 抓取。

use reqwest::header::{ACCEPT, AUTHORIZATION, HeaderMap, HeaderValue, USER_AGENT};

/// 扫描器对外声明的 User-Agent
///
/// 真实标识自身，便于平台方在需要时联系或限流，不伪装成浏览器。
pub const SCANNER_USER_AGENT: &str =
    "AxAgent-DemandDiscovery/1.0 (+https://github.com/polite0803/AxInvest)";

/// 默认请求超时（秒）
pub const DEFAULT_TIMEOUT_SECS: u64 = 10;

/// 单条线索摘要的最大字符数
pub const DEFAULT_SUMMARY_CHARS: usize = 150;

// ── 文本截断 ──────────────────────────────────────────────────

/// 按**字符**截断，超出部分以 `...` 结尾
///
/// 原实现直接 `&text[..N]` 按**字节**切片，中文一个字占 3 字节，
/// 只要 `N` 不落在 UTF-8 字符边界上就会 panic（中文文本几乎必然命中）。
/// 这里改为按 `char` 计数，永不越界。
///
/// # 行为
/// - 字符数 `<= max_chars`：原样返回，不追加省略号
/// - 字符数 `> max_chars`：返回前 `max_chars` 个字符 + `...`
pub fn truncate_chars(text: &str, max_chars: usize) -> String {
    let mut chars = text.chars();
    let head: String = chars.by_ref().take(max_chars).collect();
    if chars.next().is_some() {
        format!("{}...", head)
    } else {
        head
    }
}

/// 生成线索标题（超出 `max_chars` 个字符时截断）
pub fn summarize_title(text: &str, max_chars: usize) -> String {
    truncate_chars(text, max_chars)
}

/// 拼接标题与描述生成摘要
pub fn summarize_demand(title: &str, description: &str, max_chars: usize) -> String {
    if description.is_empty() {
        truncate_chars(title, max_chars)
    } else {
        truncate_chars(&format!("{} - {}", title, description), max_chars)
    }
}

/// URL 编码查询词
///
/// 原实现用 `replace(' ', "+")` 手工拼查询串，中文与 `&` `?` `#` 等
/// 保留字符会直接破坏 URL 结构。改用标准百分号编码。
pub fn encode_query(query: &str) -> String {
    urlencoding::encode(query).into_owned()
}

// ── HTTP 构造 ─────────────────────────────────────────────────

/// 构造带真实身份标识的请求头
///
/// `token` 存在时附加 `Authorization: Bearer <token>`。
pub fn build_headers(token: Option<&str>, accept: &str) -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert(USER_AGENT, HeaderValue::from_static(SCANNER_USER_AGENT));
    if let Ok(value) = HeaderValue::from_str(accept) {
        headers.insert(ACCEPT, value);
    }
    if let Some(token) = token.filter(|t| !t.trim().is_empty()) {
        match HeaderValue::from_str(&format!("Bearer {}", token)) {
            Ok(value) => {
                headers.insert(AUTHORIZATION, value);
            },
            Err(e) => {
                tracing::warn!(error = %e, "[scanner_common] API Token 含非法字符，已忽略认证头");
            },
        }
    }
    headers
}

/// 构造统一配置的 HTTP 客户端
///
/// 统一设置超时，避免某个平台挂起拖垮整轮 `search_all`。
///
/// # 已知环境坑
/// 本机网络走 IPv6 时访问部分站点（如东方财富行情接口）会被 RST。
/// 需求发现各平台目前未复现该问题，若后续出现连接类失败，
/// 应在此处统一加 IPv4-only 的 DNS resolver，而不是在各 scanner 里各改一遍。
pub fn build_http_client(timeout_secs: u64) -> reqwest::Client {
    match reqwest::Client::builder().timeout(std::time::Duration::from_secs(timeout_secs)).build() {
        Ok(client) => client,
        Err(e) => {
            tracing::warn!(error = %e, "[scanner_common] HTTP 客户端构造失败，回退默认配置");
            reqwest::Client::new()
        },
    }
}

// ── 合规门禁 ──────────────────────────────────────────────────

/// 无官方 API 凭证时的统一降级理由
pub const NO_CREDENTIAL_SKIP_REASON: &str =
    "未配置官方 API 凭证，已跳过（本连接器仅支持官方开放 API，不做页面抓取）";

/// 校验是否具备「通过官方 API 访问」的前提
///
/// 返回 `Err` 时调用方应**直接跳过**并记 warn 日志，绝不退化为 HTML 抓取。
///
/// # 判定
/// - `credential` 为空 → 未配置，跳过
/// - `base_url` 不是 https → 拒绝（避免明文传输凭证）
pub fn require_official_api_credential(
    platform: &str,
    credential: Option<&str>,
    base_url: &str,
) -> Result<(), String> {
    if credential.is_none_or(|c| c.trim().is_empty()) {
        tracing::info!(platform = platform, "[{}] {}", platform, NO_CREDENTIAL_SKIP_REASON);
        return Err(NO_CREDENTIAL_SKIP_REASON.to_string());
    }
    if !base_url.starts_with("https://") {
        return Err(format!("{}: base_url 必须为 https，拒绝明文传输凭证", platform));
    }
    Ok(())
}

/// 从 JSON 响应中按候选路径取字符串字段
///
/// 各平台响应结构千差万别，这里统一按「候选字段列表」顺序取值，
/// 避免每个 scanner 手写一套 `get("x").and_then(|v| v.as_str())`。
pub fn pick_str<'a>(value: &'a serde_json::Value, candidates: &[&str]) -> Option<&'a str> {
    candidates.iter().find_map(|key| value.get(*key).and_then(|v| v.as_str()))
}

/// 从 JSON 响应中按候选路径取数值字段
pub fn pick_f64(value: &serde_json::Value, candidates: &[&str]) -> Option<f64> {
    candidates.iter().find_map(|key| value.get(*key).and_then(|v| v.as_f64()))
}

/// 从可能为数组 / 对象 / 嵌套包装的响应体中取出条目数组
///
/// `path` 为逐层字段名，如 `["data", "items"]`；空数组表示响应本身就是数组。
pub fn pick_items<'a>(
    value: &'a serde_json::Value,
    path: &[&str],
) -> Option<&'a Vec<serde_json::Value>> {
    let mut current = value;
    for key in path {
        current = current.get(*key)?;
    }
    current.as_array()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncate_chars_handles_cjk() {
        // 这是一段远超 10 字符的中文，按字节切片必然落在字符中间
        let text = "这是一个用于验证中文截断安全性的较长文本";
        let out = truncate_chars(text, 10);
        assert_eq!(out.chars().count(), 13, "应为 10 个字符 + 省略号三点");
        assert!(out.ends_with("..."));
        assert!(text.starts_with(out.trim_end_matches('.')));
    }

    #[test]
    fn truncate_chars_never_panics_at_any_boundary() {
        // 逐位扫描，确保任意 max_chars 都不会 panic
        let text = "中文abc混合😀emoji以及标点的混合字符串";
        for n in 0..=text.chars().count() + 2 {
            let out = truncate_chars(text, n);
            assert!(out.chars().count() <= n + 3);
        }
    }

    #[test]
    fn truncate_chars_no_ellipsis_when_short() {
        assert_eq!(truncate_chars("短文本", 10), "短文本");
    }

    #[test]
    fn truncate_chars_handles_empty_and_zero() {
        assert_eq!(truncate_chars("", 10), "");
        assert_eq!(truncate_chars("中文", 0), "...");
    }

    #[test]
    fn encode_query_escapes_cjk_and_reserved() {
        let encoded = encode_query("设计 定制&开发");
        assert!(!encoded.contains(' '));
        assert!(!encoded.contains('&'));
        assert!(!encoded.contains('中'));
    }

    #[test]
    fn summarize_demand_joins_title_and_desc() {
        assert_eq!(summarize_demand("标题", "", 50), "标题");
        assert_eq!(summarize_demand("标题", "描述", 50), "标题 - 描述");
    }

    #[test]
    fn build_headers_uses_real_identity() {
        let headers = build_headers(None, "application/json");
        let ua = headers.get(USER_AGENT).unwrap().to_str().unwrap();
        assert!(ua.starts_with("AxAgent-DemandDiscovery"));
        assert!(!ua.contains("Mozilla"), "禁止伪造浏览器指纹");
        assert!(headers.get(AUTHORIZATION).is_none());
    }

    #[test]
    fn build_headers_attaches_bearer_token() {
        let headers = build_headers(Some("tok-123"), "application/json");
        assert_eq!(headers.get(AUTHORIZATION).unwrap().to_str().unwrap(), "Bearer tok-123");
    }

    #[test]
    fn build_headers_ignores_blank_token() {
        let headers = build_headers(Some("   "), "application/json");
        assert!(headers.get(AUTHORIZATION).is_none());
    }

    #[test]
    fn credential_gate_rejects_missing_or_insecure() {
        assert!(require_official_api_credential("x", None, "https://a.com").is_err());
        assert!(require_official_api_credential("x", Some(""), "https://a.com").is_err());
        assert!(require_official_api_credential("x", Some("t"), "http://a.com").is_err());
        assert!(require_official_api_credential("x", Some("t"), "https://a.com").is_ok());
    }

    #[test]
    fn json_pick_helpers() {
        let v = serde_json::json!({"data": {"items": [{"title": "t", "score": 3}]}});
        let items = pick_items(&v, &["data", "items"]).unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(pick_str(&items[0], &["name", "title"]), Some("t"));
        assert_eq!(pick_f64(&items[0], &["score"]), Some(3.0));
        assert!(pick_items(&v, &["data", "missing"]).is_none());
    }
}
