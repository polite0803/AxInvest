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

/// 测试辅助：断言 URL 中某个查询参数解码后等于原始查询词
///
/// [`encode_query`] 做标准百分号编码，中文与空格在 URL 里都是 `%XX` 形式，
/// 因此**不能**直接断言 `url.contains("中文")` 或 `url.contains("a+b")` —— 明文
/// 不会出现。各 scanner 的 URL 单测统一走这里：先按参数名取出原始值，再解码比对。
///
/// # Panics
/// 参数缺失、值不是合法百分号编码、或解码结果与 `expected` 不一致时 panic。
#[cfg(test)]
pub fn assert_url_query_param(url: &str, param: &str, expected: &str) {
    let marker = format!("{}=", param);
    let tail = url.split_once(&marker).unwrap_or_else(|| panic!("URL 缺少 {param} 参数: {url}"));
    let raw = tail.1.split('&').next().unwrap_or_default();
    let decoded = urlencoding::decode(raw)
        .unwrap_or_else(|e| panic!("{param} 参数不是合法的百分号编码: {raw} ({e})"));
    assert_eq!(decoded, expected, "{param} 参数解码后与原始查询词不一致: {url}");
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

// ── 价格解析（P0-2）──────────────────────────────────────────

/// 提取价格文本片段（`¥500` / `￥500` / `1200元` / `500块`）
///
/// 自闲鱼扫描器下沉共用：此前只有闲鱼提取价格文本，归一化层又不解析，
/// 价格在 `DemandLead::new_from_raw` 全链路被丢弃。
pub fn extract_price_text(text: &str) -> Option<String> {
    // 尝试匹配 "¥500" 或 "￥500" 格式
    for prefix in ["¥", "￥"] {
        if let Some(start) = text.find(prefix) {
            let after = &text[start + prefix.len()..];
            let end = after
                .find(|c: char| !c.is_ascii_digit() && c != '.')
                .map(|pos| start + prefix.len() + pos)
                .unwrap_or(text.len());
            let price_str = &text[start + prefix.len()..end];
            if !price_str.is_empty() {
                return Some(format!("{}{}", prefix, price_str));
            }
        }
    }

    // 尝试匹配 "1200元" 或 "500块" 格式（向前回溯连续数字）
    for suffix in ["元", "块"] {
        if let Some(end) = text.find(suffix) {
            let mut chars = text[..end].chars().rev();
            let mut num_start = end;
            let mut found_digit = false;
            for c in chars.by_ref() {
                if c.is_ascii_digit() || c == '.' {
                    found_digit = true;
                    num_start -= c.len_utf8();
                } else if found_digit {
                    break;
                }
            }
            if found_digit && num_start < end {
                let price_num = &text[num_start..end];
                return Some(format!("{}{}", price_num, suffix));
            }
        }
    }

    None
}

/// 解析价格文本为预算区间 `(min, max)`
///
/// 支持：
/// - 区间：`"8000-15000元"` / `"20000~30000"` / `"3千到5千"`（分隔符 `-` `–` `~` `～` `到` `至`）
/// - 单值：`"¥500"` / `"1200元"` / `"2万"` / `"8000"`
/// - 倍率后缀：`万`/`w` → ×10000，`k`/`千` → ×1000
///
/// 无法解析出正数时返回 `None`。
pub fn parse_price_range(text: &str) -> Option<(f64, f64)> {
    let text = text.trim();
    if text.is_empty() {
        return None;
    }

    /// 数字 token：`(起始字节, 结束字节, 数值)`（数值已应用倍率）
    struct Token {
        start: usize,
        end: usize,
        value: f64,
    }

    let bytes = text.as_bytes();
    let mut tokens: Vec<Token> = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i].is_ascii_digit() {
            let start = i;
            while i < bytes.len() && (bytes[i].is_ascii_digit() || bytes[i] == b'.') {
                i += 1;
            }
            // 跳过结尾的孤立 '.'（如 "100."）
            let mut num_end = i;
            while num_end > start && bytes[num_end - 1] == b'.' {
                num_end -= 1;
            }
            if num_end == start {
                continue;
            }
            let Ok(mut value) = text[start..num_end].parse::<f64>() else {
                continue;
            };
            // 倍率后缀：万/w ×10000，k/千 ×1000（后缀与数字之间允许空白）
            let mut j = num_end;
            while j < bytes.len() && bytes[j] == b' ' {
                j += 1;
            }
            let rest = &text[j..];
            if rest.starts_with('万') || rest.starts_with(['w', 'W']) {
                value *= 10_000.0;
            } else if rest.starts_with(['k', 'K']) || rest.starts_with('千') {
                value *= 1_000.0;
            }
            if value.is_finite() && value > 0.0 && value < 1_000_000_000.0 {
                tokens.push(Token { start, end: num_end, value });
            }
        } else {
            i += 1;
        }
    }

    /// 两个数字之间是否构成价格区间（隔着空白 / 分隔符 / 货币单位字 / 倍率后缀）
    fn is_range_separator(s: &str) -> bool {
        let mut has_sep = false;
        for c in s.chars() {
            match c {
                '-' | '–' | '~' | '～' | '到' | '至' => has_sep = true,
                // 空白与货币单位字
                ' ' | '元' | '块' => {},
                // 倍率后缀（"3千到5千" / "2万-5万" 中残留的后缀字符）
                '万' | 'w' | 'W' | 'k' | 'K' | '千' => {},
                _ => return false,
            }
        }
        has_sep
    }

    match tokens.as_slice() {
        [] => None,
        [a, b, ..] if is_range_separator(&text[a.end..b.start]) => {
            let (min, max) = if a.value <= b.value {
                (a.value, b.value)
            } else {
                (b.value, a.value)
            };
            Some((min, max))
        },
        [a, ..] => Some((a.value, a.value)),
    }
}

// ── 联系方式提取（P0-4）──────────────────────────────────────

/// 从自由文本中提取的联系方式集合
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ExtractedContacts {
    pub email: Option<String>,
    /// 真实电话号码；微信号**不会**出现在这里（见 [`extract_wechat`]）
    pub phone: Option<String>,
    pub wechat: Option<String>,
}

/// 从自由文本中统一提取联系方式（邮箱 / 电话 / 微信）
///
/// 各内置扫描器此前全部硬编码 `contact_*: None`，唯一有提取逻辑的 API
/// 连接器又因凭证断链跑不起来 → 实际产出零联系方式。所有扫描器的
/// 归一化入口（`DemandLead::new_from_raw`）统一调用本函数兜底。
pub fn extract_contacts(text: &str) -> ExtractedContacts {
    ExtractedContacts {
        email: extract_email_from_text(text),
        phone: extract_phone_from_text(text),
        wechat: extract_wechat(text),
    }
}

/// 从文本中提取邮箱地址（标准 `xxx@yyy.zzz` 格式，取第一个）
pub fn extract_email_from_text(text: &str) -> Option<String> {
    static RE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let re = RE.get_or_init(|| regex::Regex::new(r"[\w.+-]+@[\w-]+(\.[\w-]+)+").expect("邮箱正则"));
    re.find(text).map(|m| m.as_str().to_string())
}

/// 从文本中提取手机号（中国大陆 11 位 `1[3-9]xxxxxxxxx`，要求独立边界）
pub fn extract_phone_from_text(text: &str) -> Option<String> {
    static RE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    // regex crate 不支持 lookaround，用 \b 数字边界（11 位号码两侧非数字即可命中）
    let re = RE.get_or_init(|| regex::Regex::new(r"\b(1[3-9]\d{9})\b").expect("手机号正则"));
    re.captures(text).and_then(|c| c.get(1)).map(|m| m.as_str().to_string())
}

/// 从文本中提取微信号
///
/// 触发词：`微信` / `微信号` / `weixin` / `vx` / `wx`（大小写不敏感），
/// 后跟可选 `号` 与 `:` `：` 空白，再接 5-20 位字母开头的微信号标识。
/// 微信号不是电话，禁止写入 phone 字段（历史 bug：mock 数据把
/// `"微信: wangzhuren_biz"` 塞进了 contact_phone）。
pub fn extract_wechat(text: &str) -> Option<String> {
    static RE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let re = RE.get_or_init(|| {
        regex::Regex::new(
            r"(?i)(?:微信|weixin|vx|wx)\s*号?\s*[:：=]?\s*([A-Za-z][-_A-Za-z0-9]{4,19})",
        )
        .expect("微信号正则")
    });
    re.captures(text).and_then(|c| c.get(1)).map(|m| m.as_str().to_string())
}

// ── 内容指纹（P0-5 去重）────────────────────────────────────

/// 计算线索内容指纹（标题 + 描述归一化后哈希，16 位 hex）
///
/// 去重语义修正：旧键 `(platform, source_url)` 在「所有线索指向同一
/// 搜索页」的平台（闲鱼等）上把一轮 100 条线索压成 1 条。指纹只看
/// **内容**：同一需求换个链接重发会被识别为重复，同页不同需求各自成立。
///
/// 归一化：删除全部空白、小写。内容为空时返回 `None`
/// （空内容不参与指纹去重，避免不相干的空线索互相吞并）。
pub fn content_fingerprint(title: &str, description: &str) -> Option<String> {
    let normalize = |s: &str| {
        s.chars().filter(|c| !c.is_whitespace()).flat_map(|c| c.to_lowercase()).collect::<String>()
    };
    let norm_title = normalize(title);
    let norm_desc = normalize(description);
    if norm_title.is_empty() && norm_desc.is_empty() {
        return None;
    }
    let normalized = format!("{norm_title}\n{norm_desc}");
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    normalized.hash(&mut hasher);
    Some(format!("{:016x}", hasher.finish()))
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

    #[test]
    fn parse_price_range_handles_common_formats() {
        // 区间（带货币单位字 + 分隔符）
        assert_eq!(parse_price_range("8000-15000元"), Some((8000.0, 15000.0)));
        assert_eq!(parse_price_range("20000~30000"), Some((20000.0, 30000.0)));
        assert_eq!(parse_price_range("3千到5千"), Some((3000.0, 5000.0)));
        // 单值
        assert_eq!(parse_price_range("¥500"), Some((500.0, 500.0)));
        assert_eq!(parse_price_range("价格 1200 元"), Some((1200.0, 1200.0)));
        // 倍率
        assert_eq!(parse_price_range("2万"), Some((20000.0, 20000.0)));
        assert_eq!(parse_price_range("预算50k"), Some((50000.0, 50000.0)));
        // 无法解析 / 非法输入
        assert_eq!(parse_price_range(""), None);
        assert_eq!(parse_price_range("面议"), None);
    }

    #[test]
    fn extract_price_text_matches_xianyu_formats() {
        assert_eq!(extract_price_text("售价1200元"), Some("1200元".to_string()));
        assert_eq!(extract_price_text("价格：¥500"), Some("¥500".to_string()));
        assert_eq!(extract_price_text("无价格信息"), None);
    }

    #[test]
    fn extract_contacts_covers_email_phone_wechat() {
        let c = extract_contacts("联系 zhang@example.com 或 13800138000，微信：wang_biz123");
        assert_eq!(c.email.as_deref(), Some("zhang@example.com"));
        assert_eq!(c.phone.as_deref(), Some("13800138000"));
        assert_eq!(c.wechat.as_deref(), Some("wang_biz123"));

        // 无联系信息
        let c = extract_contacts("普通文本，无任何联系方式");
        assert_eq!(c, ExtractedContacts::default());

        // vx 触发词（大小写不敏感）
        let c = extract_contacts("加 vx: Foo_Bar99");
        assert_eq!(c.wechat.as_deref(), Some("Foo_Bar99"));
    }

    #[test]
    fn extract_phone_rejects_non_phone_numbers() {
        assert!(extract_phone_from_text("订单号 12345678901").is_none(), "12 开头不是手机号");
        assert!(extract_phone_from_text("号码太短 1380013800").is_none());
        assert_eq!(extract_phone_from_text("电话 19912345678."), Some("19912345678".to_string()));
    }

    #[test]
    fn content_fingerprint_is_stable_and_content_only() {
        // 空白与大小写归一化
        assert_eq!(
            content_fingerprint("求购 相机", "成色好"),
            content_fingerprint("求购相机", "成色好")
        );
        // 内容相同（忽略空白差异）→ 指纹一致，与 URL 无关
        assert_eq!(
            content_fingerprint("求购相机", "成色好"),
            content_fingerprint("求购相机", "成色好")
        );
        // 内容不同 → 指纹不同
        assert_ne!(content_fingerprint("求购", "相机"), content_fingerprint("出售", "相机"));
        // 空内容 → None
        assert_eq!(content_fingerprint("", ""), None);
        assert_eq!(content_fingerprint("   ", "\t"), None);
    }
}
