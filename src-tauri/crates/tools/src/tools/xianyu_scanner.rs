//! 闲鱼扫描器
//!
//! 采集闲鱼平台的二手/定制需求线索。
//!
//! ## 合规约束
//!
//! 闲鱼未对外开放检索 API。原实现伪装 Chrome UA 直接抓取搜索页 HTML，
//! 属于规避站点反爬措施。现改为：
//! - 未配置 `XIANYU_API_TOKEN` 时直接跳过，不发起任何请求；
//! - 使用真实 UA，不伪造浏览器指纹；
//! - 如已获得官方授权，可通过 `with_config` 注入 token 与 API 端点。

use super::marketplace_scanner::{MarketplaceScanner, RawLead};
use crate::tools::scanner_common;
use async_trait::async_trait;

/// 闲鱼扫描器
pub struct XianyuScanner {
    http: reqwest::Client,
    /// API Token，用于官方 API 认证（可选）
    api_token: Option<String>,
    /// 基础 URL
    base_url: String,
}

impl XianyuScanner {
    pub fn new() -> Self {
        Self::with_config(None, None)
    }

    /// 从配置创建
    ///
    /// `api_token` 未提供时回退读环境变量（桌面 GUI 进程通常不带环境变量，
    /// 平台配置里的 token 由路由层经本方法直接注入 —— 凭证三层断链修复）。
    pub fn with_config(api_token: Option<String>, base_url: Option<String>) -> Self {
        let http = scanner_common::build_http_client(scanner_common::DEFAULT_TIMEOUT_SECS);
        let api_token = api_token.or_else(|| std::env::var("XIANYU_API_TOKEN").ok());
        Self {
            http,
            api_token,
            base_url: base_url.unwrap_or_else(|| "https://api.goofish.com".to_string()),
        }
    }

    /// 构建搜索 URL
    fn build_search_url(&self, query: &str) -> String {
        // 简单的 URL 编码：空格转为 +，其他保持原样
        let encoded_query = scanner_common::encode_query(query);
        format!("{}/search?q={}", self.base_url, encoded_query)
    }

    /// 构建请求头（真实身份 + Bearer 认证）
    ///
    /// 原实现伪造 Chrome UA 与 `Referer` 以绕过站点反爬，已移除。
    fn build_headers(&self) -> reqwest::header::HeaderMap {
        scanner_common::build_headers(
            self.api_token.as_deref(),
            "text/html,application/xhtml+xml,application/xml;q=0.9,image/webp,*/*;q=0.8",
        )
    }

    /// 闲鱼需求分类（二手交易中可能转化为需求的品类）
    fn demand_categories() -> Vec<&'static str> {
        vec![
            // 数码类（可能有定制需求）
            "手机",
            "电脑",
            "相机",
            "无人机",
            "智能设备",
            // 设计类（设计服务需求）
            "设计",
            "定制",
            "Logo",
            "VI设计",
            "包装设计",
            // 服务类（外包/兼职需求）
            "代练",
            "代写",
            "代做",
            "代运营",
            "代设计",
            // 开发类（技术需求）
            "小程序",
            "网站",
            "APP",
            "系统",
            "软件",
            // 创意类
            "手绘",
            "插画",
            "动画",
            "视频剪辑",
        ]
    }

    /// 需求模式识别
    fn extract_demand_patterns(title: &str) -> Vec<String> {
        let text = title.to_lowercase();
        let mut patterns = Vec::new();

        // 求购类关键词。P1-3 修正：移除单字「求/要/找/换」与「招/聘」——
        // 单字在中文标题里是超高频误判源（"换电池"、"找到"、"招聘会"）
        let buy_patterns = [
            ("demand:want_to_buy", vec!["求购", "想收", "求收"]),
            ("demand:custom_make", vec!["定制", "定做", "代做", "代开发", "代设计"]),
            ("demand:service_need", vec!["代练", "代写", "代运营", "代办"]),
            ("demand:collaboration", vec!["合作", "合伙", "招合伙人"]),
            ("demand:swap", vec!["以物换物", "置换"]),
        ];

        for (tag, keywords) in &buy_patterns {
            if keywords.iter().any(|k| text.contains(k)) {
                patterns.push(tag.to_string());
            }
        }

        patterns
    }

    /// 检查是否属于需求相关内容
    fn is_demand_related(title: &str, description: &str) -> bool {
        let full_text = format!("{} {}", title, description).to_lowercase();

        // 首先排除明显的出售/卖出内容
        let sell_patterns = [
            "出售",
            "卖出",
            "转卖",
            "闲置",
            "九成新",
            "全新",
            "包邮",
            "包邮出",
            "低价出",
            "诚心出",
        ];
        if sell_patterns.iter().any(|p| full_text.contains(p)) {
            return false;
        }

        // 检查需求模式。P1-3 修正：移除单字「求/换」与弱信号词「需要/帮忙/
        // 协助/设计/开发/制作/实现」—— 后者在技术类商品描述里几乎必现，
        // 误判率极高；只保留强需求意图词
        let demand_patterns = [
            "求购",
            "定制",
            "定做",
            "代做",
            "代开发",
            "代设计",
            "代练",
            "代写",
            "代运营",
            "代办",
            "合作",
            "合伙",
            "招合伙人",
            "以物换物",
            "置换",
        ];

        if demand_patterns.iter().any(|p| full_text.contains(p)) {
            return true;
        }

        // 检查是否包含需求分类关键词
        let categories = Self::demand_categories();
        categories.iter().any(|c| full_text.contains(c))
    }

    /// 提取价格信息（实现下沉到 [`scanner_common::extract_price_text`] 共用，
    /// 归一化层会进一步解析为预算区间）
    fn extract_price(text: &str) -> Option<String> {
        scanner_common::extract_price_text(text)
    }

    /// 提取核心需求描述
    fn extract_demand_summary(title: &str, description: &str) -> String {
        let combined = if description.is_empty() {
            title.to_string()
        } else {
            format!("{} - {}", title, description)
        };

        scanner_common::truncate_chars(&combined, 150)
    }
}

impl Default for XianyuScanner {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl MarketplaceScanner for XianyuScanner {
    fn platform(&self) -> String {
        "xianyu".to_string()
    }

    async fn search(&self, q: &str) -> Result<Vec<RawLead>, String> {
        if q.is_empty() {
            return Ok(Vec::new());
        }

        // 合规门禁：无官方凭证直接跳过，绝不退化为页面/内部接口抓取
        scanner_common::require_official_api_credential(
            "xianyu",
            self.api_token.as_deref(),
            &self.base_url,
        )?;

        let url = self.build_search_url(q);
        let headers = self.build_headers();

        tracing::info!(query = q, "[XianyuScanner] 发起搜索请求");

        let response = self.http.get(&url).headers(headers).send().await;

        let mut leads = Vec::new();

        match response {
            Ok(resp) if resp.status().is_success() => {
                if let Ok(body) = resp.text().await {
                    // 闲鱼页面通常是 JavaScript 渲染，需要特殊处理
                    // 这里提供基础的文本分析逻辑

                    // 按行分割，查找包含需求信号的文本块
                    for segment in body.split(|c: char| c.is_control() && c != ' ' && c != '\n') {
                        let trimmed = segment.trim();
                        if trimmed.len() < 10 {
                            continue;
                        }

                        // 检查是否为需求相关内容
                        if Self::is_demand_related(trimmed, "") {
                            let patterns = Self::extract_demand_patterns(trimmed);
                            let price = Self::extract_price(trimmed);
                            let summary = Self::extract_demand_summary(trimmed, "");

                            leads.push(RawLead {
                                platform: "xianyu".to_string(),
                                title: scanner_common::truncate_chars(trimmed, 80),
                                description: summary,
                                url: url.clone(),
                                price_text: price,
                                contact: None,
                                contact_email: None,
                                contact_phone: None,
                                snapshot: serde_json::json!({
                                    "source": "xianyu_scanner",
                                    "patterns": patterns,
                                    "raw_text": trimmed,
                                    "type": "second_hand_demand",
                                }),
                            });
                        }
                    }
                }
            },
            Ok(resp) => {
                let status = resp.status();
                tracing::warn!(status = status.as_u16(), "[XianyuScanner] 请求失败");
                if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
                    return Err("闲鱼 API 速率限制".to_string());
                }
            },
            Err(e) => {
                tracing::warn!(error = %e, "[XianyuScanner] 网络请求异常，返回空结果");
            },
        }

        tracing::info!(query = q, filtered = leads.len(), "[XianyuScanner] 搜索完成");

        Ok(leads)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_platform() {
        let scanner = XianyuScanner::new();
        assert_eq!(scanner.platform(), "xianyu");
    }

    #[test]
    fn test_build_search_url() {
        let scanner = XianyuScanner::new();
        let url = scanner.build_search_url("设计定制");
        // 空格编码为 %20（不是 +），中文整体转义
        scanner_common::assert_url_query_param(&url, "q", "设计定制");
        assert!(url.contains("search"));

        let url_with_space = scanner.build_search_url("设计 定制");
        scanner_common::assert_url_query_param(&url_with_space, "q", "设计 定制");
    }

    #[test]
    fn test_is_demand_related() {
        // 需求相关内容
        assert!(XianyuScanner::is_demand_related("求购一台二手相机", ""));
        assert!(XianyuScanner::is_demand_related("定制Logo设计", ""));
        assert!(XianyuScanner::is_demand_related("代做小程序", ""));
        assert!(XianyuScanner::is_demand_related("找合伙人合作", ""));

        // 不相关内容
        assert!(!XianyuScanner::is_demand_related("出售一台手机", ""));
        assert!(!XianyuScanner::is_demand_related("九成新，包邮", ""));

        // P1-3 回归：单字「求/换」与弱信号词已移除，高频误判样本不应命中
        assert!(!XianyuScanner::is_demand_related("换电池服务，上门安装", ""));
        assert!(!XianyuScanner::is_demand_related("求折叠桌配件", "")); // "求" 单字不再命中
        assert!(!XianyuScanner::is_demand_related("程序员自用键盘，成色好", ""));
    }

    #[test]
    fn test_extract_demand_patterns() {
        let patterns = XianyuScanner::extract_demand_patterns("求购一台高端相机");
        assert!(patterns.iter().any(|p| p.contains("want_to_buy")));

        let patterns = XianyuScanner::extract_demand_patterns("代开发一个电商网站");
        assert!(patterns.iter().any(|p| p.contains("custom_make")));
    }

    #[test]
    fn test_extract_price() {
        assert_eq!(XianyuScanner::extract_price("售价1200元"), Some("1200元".to_string()));
        assert_eq!(XianyuScanner::extract_price("价格：¥500"), Some("¥500".to_string()));
        assert!(XianyuScanner::extract_price("无价格信息").is_none());
    }

    #[test]
    fn test_extract_demand_summary() {
        let summary = XianyuScanner::extract_demand_summary("短标题", "");
        assert_eq!(summary, "短标题");

        let long_title = "这是一个非常长的标题".repeat(20);
        let summary = XianyuScanner::extract_demand_summary(&long_title, "");
        // 截断按**字符**计数（中文 1 字 = 1 字符 = 3 字节），不能用字节长度断言
        assert_eq!(summary.chars().count(), 153, "应为 150 个字符 + 省略号三点");
        assert!(summary.ends_with("..."));
    }

    #[tokio::test]
    async fn test_search_empty_query() {
        let scanner = XianyuScanner::new();
        let result = scanner.search("").await;
        assert!(result.is_ok());
        assert!(result.unwrap().is_empty());
    }
}
