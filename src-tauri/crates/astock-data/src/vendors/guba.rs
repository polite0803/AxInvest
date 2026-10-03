//! 东方财富股吧 (guba.eastmoney.com) 社交舆情数据源
//!
//! 通过东方财富股吧 HTML 页面获取个股讨论热度、帖子数、情感倾向。
//! 仅实现 `get_social_sentiment`，其他行情/财务类方法返回空或错误，
//! 由路由层 (`VendorRouting`) 自动降级到其他 vendor。

use crate::error::DataError;
use crate::types::*;
use crate::vendors::StockVendor;
use async_trait::async_trait;
use serde_json::Value;

pub struct GubaVendor {
    pub http: reqwest::Client,
}

impl GubaVendor {
    pub fn new(http: reqwest::Client) -> Self {
        Self { http }
    }

    /// 东财**股吧**人气榜名次（`emappdata.eastmoney.com/stockrank/getAllCurrentList`）。
    ///
    /// 这是 `SocialSentiment.hot_rank`（文档口径「热度排名（平台内）」）唯一语义对得上的
    /// 供应方 —— 与帖子数同平台。实测（2026-10-03）：
    /// - `pageNo=1 & pageSize=100` 回 `data` 100 行，形如 `{"sc":"SH601127","rk":1,"rc":0,"hisRc":0}`；
    /// - `pageNo>=2` 或 `pageSize>100` 都回 `code=0` + **空数组** —— 「参数被拒」伪装成「无数据」，
    ///   与涨停池 `limit>200` 是同族陷阱，故 `pageSize` 锁死 100 且把「首轮回空」判为接口异常；
    /// - 榜外 ⇒ `None`，语义是「没进前 100」，**不是**「第 101 名」。
    ///
    /// 回填是增强：任何失败都只 warn + `None`，不阻断舆情主流程
    /// （与 `get_valuation_snapshot_asof` 的「失败不报错」同一处置）。
    async fn guba_hot_rank(&self, pure_code: &str) -> Option<u32> {
        let body = serde_json::json!({
            "appId": "001",
            "fc": "",
            "globalList": true,
            "marketType": "nas.cn",
            "pageNo": 1,
            "pageSize": 100,
            "sourceSite": "Web",
            "sourceId": "Web03",
        });
        let resp = self
            .http
            .post("https://emappdata.eastmoney.com/stockrank/getAllCurrentList")
            .header("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36")
            .header("Referer", "https://guba.eastmoney.com/")
            .header("Origin", "https://guba.eastmoney.com")
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| tracing::warn!("[guba] 人气榜请求失败: {e}"))
            .ok()?;
        crate::check_response_429(&resp, "guba").ok()?;
        let json: Value = resp
            .json()
            .await
            .map_err(|e| {
                tracing::warn!("[guba] 人气榜响应非 JSON: {e}");
                e
            })
            .ok()?;
        if json.get("code").and_then(Value::as_i64) != Some(0) {
            tracing::warn!("[guba] 人气榜接口拒绝: code={:?}", json.get("code"));
            return None;
        }
        let rows = json.get("data").and_then(Value::as_array).cloned().unwrap_or_default();
        if rows.is_empty() {
            tracing::warn!(
                "[guba] 人气榜回空（实测正常应有 100 行）⇒ 判接口异常，不当成「该票无人气」"
            );
            return None;
        }
        // `sc` 带交易所前缀（SH600519 / SZ000001），只比数字段即可与 pure_code 对齐
        pick_guba_rank(&rows, pure_code)
    }

    /// 带反爬头的 GET 请求（返回 HTML 文本）
    ///
    /// 修复(2026-07-21): 原接口 `guba.eastmoney.com/interface/GetData.aspx`
    /// 已废弃(返回 Content-Length: 0)。改用 `guba.eastmoney.com/list,{code}.html`
    /// HTML 页面,该页面在 `<script>` 标签中嵌入了 `var article_list=[...]`
    /// 格式的 JSON 数据。需要用 text() 拿原文后手动剥 JSON。
    async fn guba_get_html(&self, url: &str) -> Result<String, DataError> {
        let resp = self
            .http
            .get(url)
            .header("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36")
            .header("Referer", "https://guba.eastmoney.com/")
            .header("Accept", "text/html, application/xhtml+xml, */*")
            .send()
            .await
            .map_err(DataError::from)?;

        if !resp.status().is_success() {
            return Err(DataError::VendorError {
                vendor: "guba".into(),
                message: format!("HTTP {}", resp.status()),
            });
        }

        resp.text().await.map_err(DataError::from)
    }

    /// 从 HTML 文本中提取 `var article_list={"re":[...]}` 中的 `re` 数组字符串
    /// 以及顶层 `bar_name` 字段值
    ///
    /// 实际页面格式(2026-07-21 验证):
    ///   `var article_list={"re":[{...},{...}],...,"bar_name":"华如科技","bar_code":"301302",...};    var other_list=...`
    ///   (注意 `};` 和 `var` 之间可能有多个空格)
    ///
    /// 返回 `(re 数组字符串, bar_name 字段值)`:
    ///   - `re 数组字符串`: 完整的 `[...]` 字符串,可直接 serde_json::from_str
    ///   - `bar_name`: article_list 顶层 `bar_name` 字段值(请求股票的股吧名),
    ///     若该字段缺失则为 None(调用方需用 stock_code 兜底)
    ///
    /// 定位策略:
    ///   1. 找 `article_list=` 后的 `{` 开头对象
    ///   2. 用 `var other_list` 作为对象结束锚点(空格数量可变),向前找最近的 `};`
    ///   3. 在对象内找 `"re":[` 子串定位数组开始
    ///   4. 用 **平衡括号法** 找数组结束 `]`(因为帖子对象内有嵌套的 `]`,
    ///      如 `user_extendinfos` 字段结束,简单字符串匹配会误判)
    ///   5. 在对象内正则匹配 `"bar_name":"xxx"` 提取顶层 bar_name
    ///
    /// 修复(2026-07-22): stock_name 之前从帖子的 `stockbar_name` 字段提取,
    /// 但该字段是帖子所在股吧名(可能是其他股票的股吧,如聚合页/推荐帖场景),
    /// 导致返回的 stock_name 错误。改用 article_list 顶层的 `bar_name` 字段,
    /// 它对应请求的 stock_code 的股吧名,不会错位。
    fn extract_article_list(html: &str) -> Option<(&str, Option<&str>)> {
        let key = "article_list=";
        let start_idx = html.find(key)? + key.len();
        let rest = &html[start_idx..];

        // article_list 后面是 { 开头的对象
        if !rest.starts_with('{') {
            return None;
        }

        // 先确定 article_list 对象的范围
        // 实际页面用 `};    var other_list` (多空格) 作为对象结束标记
        // 用 `var other_list` 作为锚点(空格数量可变),向前找最近的 `};`
        let anchor = "var other_list";
        let anchor_rel = rest.find(anchor)?;
        let before_anchor = &rest[..anchor_rel];
        let obj_end_rel = before_anchor.rfind("};")?;
        let obj_str = &rest[..obj_end_rel + 1]; // 包含 `}`

        // 提取顶层 bar_name 字段值(请求股票的股吧名)
        // 格式: "bar_name":"华如科技" 或 "bar_name":null
        let bar_name = Self::extract_bar_name(obj_str);

        // 在对象内找 "re":[ 子串
        let re_key = "\"re\":[";
        let re_start_rel = obj_str.find(re_key)?;
        // 数组开始位置 = "re":[ 中的 [ 的位置
        let arr_start = re_start_rel + re_key.len() - 1; // 指向 `[`
        let arr_rest = &obj_str[arr_start..]; // 以 `[` 开头

        // 用平衡括号法找数组的结束 `]`
        // (帖子对象内有嵌套的 ],如 user_extendinfos 字段,简单字符串匹配会误判)
        let end_rel = Self::find_balanced_array_end(arr_rest)?;
        Some((&arr_rest[..end_rel + 1], bar_name)) // 包含 `]`
    }

    /// 从 article_list 对象字符串中提取顶层 `bar_name` 字段值
    ///
    /// 格式: `"bar_name":"华如科技"` 或 `"bar_name":null`
    /// 返回 None 表示字段缺失或为 null
    fn extract_bar_name(obj_str: &str) -> Option<&str> {
        let key = "\"bar_name\":";
        let key_idx = obj_str.find(key)?;
        let value_start = key_idx + key.len();
        let value_rest = &obj_str[value_start..];
        // 跳过空白
        let value_rest = value_rest.trim_start();
        if let Some(inner) = value_rest.strip_prefix('"') {
            // 字符串值,找结束引号(忽略转义)
            let end = inner.find('"')?;
            Some(&inner[..end])
        } else {
            // null 或其他字面量,视为无 bar_name
            None
        }
    }

    /// 用平衡括号法找 JSON 数组的结束位置
    ///
    /// 输入 `s` 必须以 `[` 开头。返回匹配的 `]` 的位置(从 0 开始)。
    /// 处理字符串内的 `[`/`]`(忽略)、转义字符 `\\`、嵌套数组/对象。
    fn find_balanced_array_end(s: &str) -> Option<usize> {
        if !s.starts_with('[') {
            return None;
        }
        let bytes = s.as_bytes();
        let mut depth: i32 = 0;
        let mut in_str = false;
        let mut escape = false;
        for (i, &b) in bytes.iter().enumerate() {
            if escape {
                escape = false;
                continue;
            }
            match b {
                b'\\' if in_str => escape = true,
                b'"' => in_str = !in_str,
                b'[' | b'{' if !in_str => depth += 1,
                b']' | b'}' if !in_str => {
                    depth -= 1;
                    if depth == 0 && b == b']' {
                        return Some(i);
                    }
                },
                _ => {},
            }
        }
        None
    }
}

#[async_trait]
impl StockVendor for GubaVendor {
    // ── 行情/财务类方法：股吧不提供，返回空或错误，由路由层降级 ──

    async fn get_quote(&self, _stock_code: &str) -> Result<StockQuote, DataError> {
        Err(DataError::VendorError {
            vendor: "guba".into(), message: "股吧不提供行情数据".into()
        })
    }

    async fn get_klines(
        &self,
        _stock_code: &str,
        _period: &str,
        _limit: u32,
        _adj: Option<AdjType>,
    ) -> Result<Vec<KLine>, DataError> {
        Ok(vec![])
    }

    async fn get_financials(&self, _stock_code: &str) -> Result<Vec<FinancialReport>, DataError> {
        Ok(vec![])
    }

    async fn get_news(&self, _stock_code: &str, _limit: u32) -> Result<Vec<NewsItem>, DataError> {
        Ok(vec![])
    }

    async fn get_money_flow(&self, _: &str) -> Result<Option<MoneyFlow>, DataError> {
        Ok(None)
    }

    async fn get_dragon_tiger(&self, _: &str) -> Result<Vec<DragonTigerEntry>, DataError> {
        Ok(vec![])
    }

    async fn get_lockup_schedule(&self, _: &str) -> Result<Vec<LockupSchedule>, DataError> {
        Ok(vec![])
    }

    async fn search_stock(&self, _: &str) -> Result<Vec<StockSearchResult>, DataError> {
        Ok(vec![])
    }

    // ── 社交舆情：核心实现 ──

    /// 获取股吧社交舆情数据
    ///
    /// 修复(2026-07-21): 原接口 `GetData.aspx` 已废弃,返回空 body。
    /// 改用 `https://guba.eastmoney.com/list,{code}.html` HTML 页面,
    /// 从嵌入的 `var article_list=[...]` 中解析帖子数据。
    ///
    /// 修复(2026-07-22):
    ///   1. URL 中去除 stock_code 的 sh/sz/bj 前缀(东方财富股吧 URL 不支持带前缀格式,
    ///      带前缀会返回聚合页/错误页面,导致帖子的 stockbar_name 是其他股票名)。
    ///   2. stock_name 改用 article_list 顶层 `bar_name` 字段(请求股票对应的股吧名),
    ///      不再从帖子的 `stockbar_name` 字段提取(该字段是帖子所在股吧名,
    ///      在聚合页/推荐帖场景下可能是其他股票)。
    ///
    /// 情感分析基于帖子标题关键词,使用 `crate::sentiment` 统一词典(P2-B4)。
    /// - bull_ratio: 旧有指标,基于词典 bull/bear 计数
    /// - sentiment_score: 细粒度评分 [-1.0, 1.0],含否定词检测
    async fn get_social_sentiment(
        &self,
        stock_code: &str,
    ) -> Result<Vec<SocialSentiment>, DataError> {
        // 去除 sh/sz/bj 前缀,东方财富股吧 URL 只接受纯数字代码
        let pure_code =
            stock_code.trim_start_matches("sh").trim_start_matches("sz").trim_start_matches("bj");
        let url = format!("https://guba.eastmoney.com/list,{pure_code}.html");
        let html = self.guba_get_html(&url).await?;

        // 同平台人气榜名次（东财 stockrank，实测只到前 100）。
        // ⚠ as-of 回放刻意不取：人气榜只有当下语义，把今天的排名塞进历史判定是时间泄露，
        //   而 `hot_rank: None` 至少不会说谎。
        let hot_rank = if crate::as_of::is_asof_active() {
            None
        } else {
            self.guba_hot_rank(pure_code).await
        };

        // 从 HTML 中提取 article_list JSON 数组 + 顶层 bar_name 字段
        let (json_str, bar_name) =
            Self::extract_article_list(&html).ok_or_else(|| DataError::VendorError {
                vendor: "guba".into(),
                message: format!("股吧 HTML 中未找到 article_list 数据, stock_code={stock_code}"),
            })?;

        let posts: Vec<Value> = serde_json::from_str(json_str).map_err(|e| {
            DataError::ParseError(format!(
                "股吧 article_list JSON 解析失败: {e}, preview={}",
                &json_str[..json_str.len().min(200)]
            ))
        })?;

        // 优先使用 article_list 顶层的 bar_name 字段作为股票名称
        // (bar_name 对应请求的 stock_code,不会错位)
        // 兜底:如果 bar_name 缺失,则 stock_name 为空字符串
        let stock_name = bar_name.map(|s| s.trim_end_matches("吧").to_string()).unwrap_or_default();

        if posts.is_empty() {
            return Ok(vec![SocialSentiment {
                stock_code: stock_code.to_string(),
                stock_name,
                platform: "guba".to_string(),
                post_count: 0,
                hot_rank,
                sentiment_score: None,
                bull_ratio: None,
                fetched_at: chrono::Utc::now().timestamp(),
            }]);
        }

        // 统计帖子数 + 情感分析(P2-B4: 改用统一词典)
        let post_count = posts.len() as u32;
        let mut bull_count = 0u32;
        let mut bear_count = 0u32;
        // P2-B4: 累积每条帖子的细粒度 sentiment_score,用于计算平均情感分
        let mut score_sum = 0.0f64;
        let mut score_count = 0u32;

        for post in &posts {
            let title = post["post_title"].as_str().unwrap_or("");

            // 基于统一词典的情感分析(替换原 8 词硬编码)
            // bull/bear 计数保留(用于 bull_ratio),score 累加用于 sentiment_score
            let is_bull = crate::sentiment::POSITIVE_KEYWORDS.iter().any(|kw| title.contains(kw));
            let is_bear = crate::sentiment::RISK_KEYWORDS.iter().any(|kw| title.contains(kw))
                || crate::sentiment::HIGH_RISK_KEYWORDS.iter().any(|kw| title.contains(kw));

            if is_bull {
                bull_count += 1;
            }
            if is_bear {
                bear_count += 1;
            }

            // 细粒度 score(标题单文本评分,[-1.0, 1.0])
            if let Some(s) = crate::sentiment::compute_text_sentiment(title) {
                score_sum += s;
                score_count += 1;
            }
        }

        let total = bull_count + bear_count;
        let bull_ratio = if total > 0 {
            Some(bull_count as f64 / total as f64)
        } else {
            None
        };
        // sentiment_score: P2-B4 改为统一词典的平均评分(更精确,包含否定词检测)
        // fallback: 若统一词典全部未命中(纯水帖),用 bull_ratio 兜底
        let sentiment_score = if score_count > 0 {
            Some(score_sum / score_count as f64)
        } else {
            bull_ratio.map(|r| (r - 0.5) * 2.0)
        };

        Ok(vec![SocialSentiment {
            stock_code: stock_code.to_string(),
            stock_name,
            platform: "guba".to_string(),
            post_count,
            hot_rank,
            sentiment_score,
            bull_ratio,
            fetched_at: chrono::Utc::now().timestamp(),
        }])
    }
}

/// 从人气榜行里取该票的名次（`sc` 形如 `SH600519`，只比数字段即可与去前缀代码对齐）。
///
/// 抽成自由函数是为了让「榜外 ⇒ None（而不是第 101 名）」这条判据能零网络测到。
fn pick_guba_rank(rows: &[Value], pure_code: &str) -> Option<u32> {
    rows.iter().find_map(|r| {
        let sc = r.get("sc").and_then(Value::as_str)?;
        let digits: String = sc.chars().filter(|c| c.is_ascii_digit()).collect();
        if digits != pure_code {
            return None;
        }
        r.get("rk").and_then(Value::as_u64).map(|v| v as u32)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_article_list_finds_array_and_bar_name() {
        // 实际页面格式: article_list={"re":[...],"bar_name":"..."}
        // 注意 `};` 和 `var` 之间有多个空格(实际页面格式)
        let html = r#"<script>var article_list={"re":[{"post_id":1,"post_title":"涨"},{"post_id":2,"post_title":"跌"}],"count":2,"bar_name":"华如科技","bar_code":"301302"};    var other_list={"re":[]};</script>"#;
        let (json_str, bar_name) = GubaVendor::extract_article_list(html).unwrap();
        assert!(json_str.starts_with('['));
        assert!(json_str.ends_with(']'));
        let posts: Vec<Value> = serde_json::from_str(json_str).unwrap();
        assert_eq!(posts.len(), 2);
        assert_eq!(posts[0]["post_title"].as_str(), Some("涨"));
        assert_eq!(posts[1]["post_title"].as_str(), Some("跌"));
        // 验证 bar_name 字段正确提取
        assert_eq!(bar_name, Some("华如科技"));
    }

    #[test]
    fn extract_article_list_returns_none_when_missing() {
        let html = "<html>no article_list here</html>";
        assert!(GubaVendor::extract_article_list(html).is_none());
    }

    #[test]
    fn extract_article_list_handles_empty_re_and_null_bar_name() {
        // re 为空数组 + bar_name 为 null 的情况
        let html = r#"<script>var article_list={"re":[],"count":0,"bar_name":null}; var other_list={"re":[]};</script>"#;
        let (json_str, bar_name) = GubaVendor::extract_article_list(html).unwrap();
        assert_eq!(json_str, "[]");
        let posts: Vec<Value> = serde_json::from_str(json_str).unwrap();
        assert!(posts.is_empty());
        // bar_name 为 null,返回 None
        assert_eq!(bar_name, None);
    }

    #[test]
    fn extract_article_list_handles_nested_arrays_in_posts() {
        // 帖子对象内有嵌套数组(如 modules:["a","b"]),不能用简单字符串匹配
        // 必须用平衡括号法才能正确找到 re 数组的结束
        let html = r#"<script>var article_list={"re":[{"post_id":1,"post_title":"涨","modules":["news","video"],"user_extendinfos":{"medal_list":["a","b"]}},{"post_id":2,"post_title":"跌"}],"count":2,"bar_name":"测试股票"};    var other_list={"re":[]};</script>"#;
        let (json_str, bar_name) = GubaVendor::extract_article_list(html).unwrap();
        assert!(json_str.starts_with('['));
        assert!(json_str.ends_with(']'));
        let posts: Vec<Value> = serde_json::from_str(json_str).unwrap();
        assert_eq!(posts.len(), 2);
        assert_eq!(posts[0]["post_title"].as_str(), Some("涨"));
        assert_eq!(posts[0]["modules"][0].as_str(), Some("news"));
        assert_eq!(posts[1]["post_title"].as_str(), Some("跌"));
        assert_eq!(bar_name, Some("测试股票"));
    }

    #[test]
    fn extract_article_list_bar_name_with_spaces() {
        // bar_name 字段值前后可能有空格,需正确处理
        let html = r#"<script>var article_list={"re":[],"bar_name": "伊利股份"};    var other_list={};</script>"#;
        let (_json_str, bar_name) = GubaVendor::extract_article_list(html).unwrap();
        assert_eq!(bar_name, Some("伊利股份"));
    }

    /// 名次提取：`sc` 带交易所前缀，只比数字段；榜外判 `None` 而不是「第 101 名」。
    #[test]
    fn rank_matches_by_digits_only() {
        let rows = serde_json::json!([
            {"sc": "SH600519", "rk": 1, "rc": 0, "hisRc": 0},
            {"sc": "SZ000002", "rk": 2, "rc": 1, "hisRc": -1},
            {"sc": "BJ871245", "rk": 3, "rc": 0, "hisRc": 0}
        ]);
        let arr = rows.as_array().unwrap();
        assert_eq!(pick_guba_rank(arr, "600519"), Some(1));
        assert_eq!(pick_guba_rank(arr, "000002"), Some(2));
        assert_eq!(pick_guba_rank(arr, "871245"), Some(3));
        assert_eq!(pick_guba_rank(arr, "600520"), None, "榜外应是缺席，不是第 101 名");
    }

    /// `rk` 缺失或非数值 ⇒ 不编名次。
    #[test]
    fn missing_rank_stays_absent() {
        let rows = serde_json::json!([{"sc": "SH600519"}, {"sc": "SH600520", "rk": "x"}]);
        let arr = rows.as_array().unwrap();
        assert_eq!(pick_guba_rank(arr, "600519"), None);
        assert_eq!(pick_guba_rank(arr, "600520"), None);
    }
}
