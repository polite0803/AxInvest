//! 东方财富股吧 (guba.eastmoney.com) 社交舆情数据源
//!
//! 通过东方财富股吧 HTML 页面获取个股讨论热度、帖子数、情感倾向。
//! 仅实现 `get_social_sentiment`，其他行情/财务类方法返回空或错误，
//! 由路由层 (`VendorRouting`) 自动降级到其他 vendor。

use crate::as_of_capability::AsOfCapability;
use crate::error::DataError;
use crate::types::*;
use crate::vendors::StockVendor;
use async_trait::async_trait;
use chrono::TimeZone;
use serde_json::Value;
use std::collections::HashSet;
use std::sync::Arc;

pub struct GubaVendor {
    pub http: reqwest::Client,
}

impl GubaVendor {
    /// 股吧专用 client —— **禁用连接复用**。
    ///
    /// 2026-10-23 实测（000001，同一进程内对照，差别只有是否复用连接）：
    /// - 复用 keep-alive 连接连发 5 次 ⇒ **5/5** 回 2834 字节的「身份核实」页
    ///   （`<title>身份核实</title>` + `passport` + `captcha`），没有 `article_list`；
    /// - 每次新建连接连发 5 次 ⇒ **5/5** 都是 142853 字节的完整列表页。
    ///
    /// 共享 client 的池化连接会被批量分析反复复用，这就是「偶发但成批出现、1 秒重试无效、
    /// 事后单独 curl 又正常」的**全部**成因（10-17 09:07 一分钟 43 条起跑、34 条失败）。
    /// 所以这里不走池：`pool_max_idle_per_host(0)` 不保留空闲连接，`http1_only()` 让
    /// 「不复用」的语义不依赖 h2 是否协商成功（h2 里 `Connection` 是非法 header）。
    /// 代价是每次请求多一次 TCP+TLS 握手，股吧每股只 1–2 个请求，可接受。
    ///
    /// 也刻意不共享 `cookie_store`：身份核实页会下发 cookie，带着它进池只会加固被核实状态。
    ///
    /// ⚠ 2026-10-25 更正：禁复用**不是壳页的全部成因**。现网 23:46 在已带本配置的客户端上
    /// 仍拿到 2834 字节身份核实页（批量抓取后的 IP 级风控，单次 1s 重试同样命中），
    /// 而同一时刻 `mguba.eastmoney.com` 仍能返回完整的服务端渲染列表页。故 live 路径在
    /// 桌面页取不到 `article_list` 时回退移动端 `window.list`（见 `extract_mobile_list`），
    /// 不再把「换一次连接」当成唯一解法。
    pub fn new() -> Self {
        let http = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .connect_timeout(std::time::Duration::from_secs(15))
            .user_agent(
                "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 \
                 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36",
            )
            .gzip(true)
            .min_tls_version(reqwest::tls::Version::TLS_1_2)
            .dns_resolver(Arc::new(crate::Ipv4OnlyResolver))
            .http1_only()
            .pool_max_idle_per_host(0)
            .build()
            .unwrap_or_else(|e| {
                tracing::error!("[guba] 专用 HTTP client 构建失败，退回默认配置: {e}");
                reqwest::Client::new()
            });
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
    ///
    /// 2026-10-22 补 429 归类: 舆情路由只有 guba 一个源，而 `try_vendors_retry` 对普通
    /// `VendorError` 会 `record_failure` ⇒ 唯一源一旦被健康追踪降级就从路由里消失，
    /// 舆情变成**长期**缺席且只剩 debug 一行。限流按 `RateLimited` 上报才不增加连续失败
    /// 计数（与 `lib.rs` 中「限流(429)单独处理」那条判据同源，`guba_hot_rank` 早已这么做）。
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

        crate::check_response_429(&resp, "guba")?;

        if !resp.status().is_success() {
            return Err(DataError::VendorError {
                vendor: "guba".into(),
                message: format!("HTTP {}", resp.status()),
            });
        }

        resp.text().await.map_err(DataError::from)
    }

    /// 桌面列表页被风控拦下时的**兜底取数口**：移动端 `mguba.eastmoney.com`。
    ///
    /// 2026-10-25 现网实测：桌面 `list,{code}.html` 回 2834 字节「身份核实」壳页的同一时刻，
    /// 移动端 `mguba/list/{code}` 仍正常返回服务端渲染页，内嵌 `window.list` 里有完整的
    /// `re` 帖子数组（`post_id` / `post_title` / `post_publish_time` 与桌面页同形，
    /// 首页 20 条）。两站只在「取数入口」上不同 ⇒ 解析与情感算法完全复用。
    ///
    /// 只用于 **live** 路径：移动端首页只有最近 20 条，不提供桌面页的 `f_N` 翻页，
    /// 拿它铺 as-of 的「按发帖时间重建」会静默漏掉截止日更早的帖子。
    async fn guba_mobile_posts(
        &self,
        pure_code: &str,
    ) -> Result<(Vec<Value>, Option<String>), DataError> {
        let url = format!("https://mguba.eastmoney.com/mguba/list/{pure_code}");
        let html = self.guba_get_html(&url).await?;
        let Some((json_str, short_name)) = Self::extract_mobile_list(&html) else {
            tracing::warn!(
                "[guba] 移动端 {url} 也没有 window.list（桌面页与移动页同时不可用） | {}",
                html_forensics(&html)
            );
            return Err(DataError::RateLimited {
                vendor: "guba".into(),
                reason: format!("桌面页与移动页均取不到帖子列表, stock_code={pure_code}"),
            });
        };
        let posts: Vec<Value> = serde_json::from_str(json_str).map_err(|e| {
            let preview: String = json_str.chars().take(200).collect();
            DataError::ParseError(format!(
                "股吧移动页 window.list 解析失败: {e}, preview={preview}"
            ))
        })?;
        Ok((posts, short_name.map(|s| s.trim_end_matches("吧").to_string())))
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
    /// 定位策略与各步失败的对应关系见 [`ListPageFailure`]。
    fn extract_article_list(html: &str) -> Result<(&str, Option<&str>), ListPageFailure> {
        let key = "article_list=";
        let Some(start_idx) = html.find(key) else {
            return Err(ListPageFailure::NotListPage);
        };
        let rest = &html[start_idx + key.len()..];

        // article_list 后面是 { 开头的对象
        if !rest.starts_with('{') {
            return Err(ListPageFailure::UnexpectedShape);
        }

        // 先确定 article_list 对象的范围
        // 实际页面用 `};    var other_list` (多空格) 作为对象结束标记
        // 用 `var other_list` 作为锚点(空格数量可变),向前找最近的 `};`
        let anchor = "var other_list";
        let anchor_rel = match rest.find(anchor) {
            Some(v) => v,
            None => return Err(ListPageFailure::MissingEndAnchor),
        };
        let before_anchor = &rest[..anchor_rel];
        let obj_end_rel = match before_anchor.rfind("};") {
            Some(v) => v,
            None => return Err(ListPageFailure::MissingEndAnchor),
        };
        let obj_str = &rest[..obj_end_rel + 1]; // 包含 `}`

        // 提取顶层 bar_name 字段值(请求股票的股吧名)
        // 格式: "bar_name":"华如科技" 或 "bar_name":null
        let bar_name = Self::extract_bar_name(obj_str);

        // 在对象内找 "re":[ 子串
        let re_key = "\"re\":[";
        let re_start_rel = match obj_str.find(re_key) {
            Some(v) => v,
            None => return Err(ListPageFailure::MissingPostArray),
        };
        // 数组开始位置 = "re":[ 中的 [ 的位置
        let arr_start = re_start_rel + re_key.len() - 1; // 指向 `[`
        let arr_rest = &obj_str[arr_start..]; // 以 `[` 开头

        // 用平衡括号法找数组的结束 `]`
        // (帖子对象内有嵌套的 ],如 user_extendinfos 字段,简单字符串匹配会误判)
        let end_rel = match Self::find_balanced_array_end(arr_rest) {
            Some(v) => v,
            None => return Err(ListPageFailure::Truncated),
        };
        Ok((&arr_rest[..end_rel + 1], bar_name)) // 包含 `]`
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
        Self::find_balanced_end(s, b']')
    }

    /// 同 [`Self::find_balanced_array_end`]，只是收口字符是 `}` —— 移动端 `window.list`
    /// 是对象而非数组。
    fn find_balanced_object_end(s: &str) -> Option<usize> {
        Self::find_balanced_end(s, b'}')
    }

    fn find_balanced_end(s: &str, close: u8) -> Option<usize> {
        let open = match close {
            b']' => b'[',
            b'}' => b'{',
            _ => return None,
        };
        if s.as_bytes().first() != Some(&open) {
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
                    if depth == 0 && b == close {
                        return Some(i);
                    }
                },
                _ => {},
            }
        }
        None
    }

    /// 从移动端列表页内嵌的 `window.list = {...}` 中提取 `re` 帖子数组与 `bar_info.ShortName`。
    ///
    /// 页面形态（2026-10-25 实测，`mguba.eastmoney.com/mguba/list/002343`）：
    ///   `window.type = 1; window.sort = 1; window.list = {"bar_info":{"ShortName":"慈文传媒",...},`
    ///   `"re":[{"post_id":...,"post_title":"...","post_publish_time":"2026-10-25 18:52:47",...}],...};`
    /// 字段与桌面页 `article_list.re` 同形（含嵌套 `reply_list`），故解析器可复用。
    ///
    /// 返回 `(re 数组字符串, ShortName)`；找不到 `window.list` 赋值或任一锚点缺失 ⇒ `None`
    /// （调用方据此把「桌面页 + 移动页都不可用」如实上报）。
    fn extract_mobile_list(html: &str) -> Option<(&str, Option<&str>)> {
        let key = "window.list";
        let mut from = 0usize;
        while let Some(rel) = html[from..].find(key) {
            let idx = from + rel;
            from = idx + key.len();
            let rest = html[from..].trim_start();
            let Some(rest) = rest.strip_prefix('=') else { continue };
            let rest = rest.trim_start();
            let Some(obj_end) = Self::find_balanced_object_end(rest) else { continue };
            let obj_str = &rest[..=obj_end];
            let re_key = "\"re\":[";
            let Some(re_start_rel) = obj_str.find(re_key) else { continue };
            let arr_start = re_start_rel + re_key.len() - 1;
            let arr_rest = &obj_str[arr_start..];
            let Some(end_rel) = Self::find_balanced_array_end(arr_rest) else { continue };
            let short_name = Self::extract_short_name(obj_str);
            return Some((&arr_rest[..=end_rel], short_name));
        }
        None
    }

    /// `bar_info` 里的 `ShortName`（请求股票对应的股吧简称）。
    fn extract_short_name(obj_str: &str) -> Option<&str> {
        let key = "\"ShortName\":";
        let key_idx = obj_str.find(key)?;
        let rest = obj_str[key_idx + key.len()..].trim_start();
        let inner = rest.strip_prefix('"')?;
        let end = inner.find('"')?;
        Some(&inner[..end])
    }
}

impl Default for GubaVendor {
    fn default() -> Self {
        Self::new()
    }
}

/// 取不到帖子数组的**具体**失败点。
///
/// 2026-10-22：此前五步锚点失败全压成同一句「股吧 HTML 中未找到 article_list 数据」，
/// 运行时不留任何现场 ⇒ 「站点偶发不给列表页」和「列表页改版」这两种**处置完全相反**的
/// 情形（前者退避重试有意义，后者重试永远拿同一个结果、必须改解析器）无法区分，
/// 定性只能靠事后重新 curl —— 站点不复发就永远归不了因（本轮实测：现网 8/8 全成功、
/// 库里 50 条失败分散在 20+ 只票、10-17 09:07 单分钟内十几只，指向偶发而非漂移）。
///
/// 文案刻意避开 `DataError::is_empty_data` 的词表（「为空 / 返回空 / empty / no data /
/// 无数据」）—— 命中会被路由层当成「源明确回答没有这份数据」的非故障，既不记降级也不重试
/// （同族教训见 `vendors/xueqiu.rs` 的 get_news 注释）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ListPageFailure {
    /// 整页没有 `article_list` 字样 ⇒ 拿回来的不是股吧列表页（风控/登录壳页）
    NotListPage,
    /// `article_list=` 后面不是 `{` ⇒ 赋值形态变了
    UnexpectedShape,
    /// 定位不到对象边界（`var other_list` 锚点或其前的 `};`）
    MissingEndAnchor,
    /// 对象内没有 `"re":[` ⇒ 帖子数组的键名或排版变了
    MissingPostArray,
    /// 帖子数组的括号未闭合 ⇒ 响应被截断
    Truncated,
}

impl ListPageFailure {
    fn phrase(self) -> &'static str {
        match self {
            Self::NotListPage => "页面不含 article_list（拿到的不是股吧列表页，疑似风控/登录壳页）",
            Self::UnexpectedShape => "article_list 赋值形态变更",
            Self::MissingEndAnchor => "定位不到 article_list 对象边界",
            Self::MissingPostArray => "article_list 对象内不含帖子数组键",
            Self::Truncated => "帖子数组括号未闭合（响应截断）",
        }
    }

    /// live 路径的**处置分道**：「站点没给列表页」按限流上报，其余四格按持续故障上抛。
    ///
    /// 依据（2026-10-22/23）：库里 462 行含 guba 的执行中 `from guba: HTTP` 命中 **0**
    /// —— 股吧从未报过任何 HTTP 状态码错误，50 条失败全是「HTTP 200 但解析不出」；同批 URL
    /// 事后连抓 8/8 全部可解析 —— 排除契约变更；再往下追，壳页的真身是东财**身份核实页**
    /// （2834 字节、`<title>身份核实</title>` + passport + captcha），由**连接复用**触发
    /// （对照实验见 `GubaVendor::new`）。上报成 `RateLimited` 的收益是**不计入 vendor 降级**
    /// （唯一舆情源不会因此从路由消失、舆情不会变长期缺席）。
    /// ⚠ 它拿不到的另一样是「重试」—— 重试会复用同一条池化连接、照样回核实页；
    /// 治本在 `GubaVendor::new` 的禁复用，不在这里。
    ///
    /// ⚠ as-of 首页**不走这里**：它在 `asof_probe` 里，不在 `try_vendors_retry` 的重试窗口内，
    /// 归限流拿不到收益，反而会改变 `record_degradation_kind` 的记录口径。
    fn to_live_error(self, stock_code: &str) -> DataError {
        let why = self.phrase();
        match self {
            Self::NotListPage => DataError::RateLimited {
                vendor: "guba".into(),
                reason: format!("{why}, stock_code={stock_code}"),
            },
            _ => DataError::VendorError {
                vendor: "guba".into(),
                message: format!("股吧页面不可用: {why}, stock_code={stock_code}"),
            },
        }
    }
}

/// 失败现场摘要。
///
/// 头部用 `chars()` 取 —— 股吧 HTML 含中文，按字节切会在非 UTF-8 边界 panic。
fn html_forensics(html: &str) -> String {
    let head: String = html.chars().take(160).collect();
    let head = head.split_whitespace().collect::<Vec<_>>().join(" ");
    format!("len={}, head={head}", html.len())
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

        // 从 HTML 中提取 article_list JSON 数组 + 顶层 bar_name 字段。
        // 桌面页被东财「身份核实」拦下（HTTP 200 + 无 article_list）时改走移动端
        // `window.list` —— 同一时刻移动端仍正常，两站字段同形（见 `guba_mobile_posts`）。
        let (posts, stock_name) = match Self::extract_article_list(&html) {
            Ok((json_str, bar_name)) => {
                let posts: Vec<Value> = serde_json::from_str(json_str).map_err(|e| {
                    // 预览按 chars() 切 —— 帖子标题是中文，字节切片会在非 UTF-8 边界 panic。
                    // 这一处刻意随本批改掉：它是「出错时才走」的路径，留着就等于归因函数自己会崩。
                    let preview: String = json_str.chars().take(200).collect();
                    DataError::ParseError(format!(
                        "股吧 article_list JSON 解析失败: {e}, preview={preview}"
                    ))
                })?;
                (posts, bar_name.map(|s| s.trim_end_matches("吧").to_string()))
            },
            Err(f) => {
                tracing::warn!(
                    "[guba] {url} 帖子数组取不到: {} | {} ⇒ 改用移动端列表页",
                    f.phrase(),
                    html_forensics(&html)
                );
                match self.guba_mobile_posts(pure_code).await {
                    Ok(v) => {
                        tracing::warn!(
                            "[guba] {pure_code} 桌面页不可用，已用移动端列表页兜底（{} 条帖子）",
                            v.0.len()
                        );
                        v
                    },
                    Err(e) => {
                        tracing::warn!("[guba] {pure_code} 桌面页与移动端兜底均失败: {e}");
                        return Err(f.to_live_error(stock_code));
                    },
                }
            },
        };

        // live 与 as-of 共用同一个情感计算（as-of 只是在「取哪些帖子」上不同）
        Ok(vec![social_sentiment_from_posts(
            stock_code,
            stock_name.unwrap_or_default(),
            &posts,
            hot_rank,
            chrono::Utc::now().timestamp(),
        )])
    }

    /// 舆情的 as-of 通道：按**发帖时间**翻页重建截止日可见的帖子集合。
    ///
    /// 页面 `list,{code},f_N.html` 按最后回复倒序，但每条都带 `post_publish_time`；
    /// 逐页收集 `publish <= 截止日 23:59:59` 的帖子，按发帖时间去重后取最近 81 条
    /// （与 live 首页条数同量级），再交给**同一个** `social_sentiment_from_posts`。
    /// 两路只在「取哪些帖子」上不同，情感算法完全一致。
    ///
    /// 提前停止：已集满 81 条且当前页最大发帖时间也 <= 截止日；上限 40 页
    /// （实测 002696 的 40 页覆盖约 3 个月，足够任意常用回放锚点）。
    /// 结果由调用方写入每日快照 ⇒ 同一截止日后续不再翻页。
    async fn get_social_sentiment_with_asof(
        &self,
        stock_code: &str,
    ) -> Result<Vec<SocialSentiment>, DataError> {
        const TARGET_POSTS: usize = 81;
        const MAX_PAGES: usize = 40;
        let ctx = crate::as_of::current_as_of()
            .ok_or_else(|| DataError::ParseError("no as_of context".into()))?;
        let cutoff = ctx.as_of_date.format("%Y-%m-%d").to_string();
        let cutoff_end = format!("{cutoff} 23:59:59");
        let pure_code =
            stock_code.trim_start_matches("sh").trim_start_matches("sz").trim_start_matches("bj");
        let mut bar_name: Option<String> = None;
        let mut seen: HashSet<String> = HashSet::new();
        let mut picked: Vec<(String, Value)> = Vec::new();
        for page in 1..=MAX_PAGES {
            let url = if page == 1 {
                format!("https://guba.eastmoney.com/list,{pure_code}.html")
            } else {
                format!("https://guba.eastmoney.com/list,{pure_code},f_{page}.html")
            };
            // 第 1 页取不到 ⇒ 真失败（源不可达/结构变了）；后续页取不到则用已收集的
            // 结果收尾 —— 翻页途中被风控掐断不应把前面 20 页已拿到的帖子全丢掉。
            let html = match self.guba_get_html(&url).await {
                Ok(h) => h,
                Err(e) if page == 1 => return Err(e),
                Err(e) => {
                    tracing::warn!("[guba] as-of 翻页在第 {page} 页中断，使用已收集帖子: {e}");
                    break;
                },
            };
            let (json_str, name) = match Self::extract_article_list(&html) {
                Ok(v) => v,
                Err(f) => {
                    if page == 1 {
                        tracing::warn!(
                            "[guba] as-of {url} 帖子数组取不到: {} | {}",
                            f.phrase(),
                            html_forensics(&html)
                        );
                        return Err(DataError::VendorError {
                            vendor: "guba".into(),
                            message: format!(
                                "股吧 as-of 首页不可用: {}, stock={stock_code}",
                                f.phrase()
                            ),
                        });
                    }
                    tracing::warn!(
                        "[guba] as-of 第 {page} 页解析不到帖子数组（{}）⇒ 用已收集帖子收尾",
                        f.phrase()
                    );
                    break;
                },
            };
            if bar_name.is_none() {
                bar_name = name.map(|s| s.trim_end_matches("吧").to_string());
            }
            let posts: Vec<Value> = serde_json::from_str(json_str).map_err(|e| {
                DataError::ParseError(format!("股吧 as-of article_list 解析失败: {e}"))
            })?;
            if posts.is_empty() {
                break;
            }
            let page_max = collect_posts_as_of(posts, &cutoff_end, &mut seen, &mut picked);
            let enough = picked.len() >= TARGET_POSTS
                && page_max.as_deref().is_some_and(|m| m <= cutoff_end.as_str());
            if enough {
                break;
            }
            // 轻节流：避免连续翻 40 页触发站点风控
            tokio::time::sleep(std::time::Duration::from_millis(120)).await;
        }
        if picked.is_empty() {
            return Ok(vec![]);
        }
        picked.sort_by(|a, b| b.0.cmp(&a.0));
        picked.truncate(TARGET_POSTS);
        let posts: Vec<Value> = picked.into_iter().map(|(_, p)| p).collect();
        // fetched_at 必须是截止日，不是 now —— 否则下游会把历史舆情读成刚采的
        let fetched_at = chrono::NaiveDate::parse_from_str(&cutoff, "%Y-%m-%d")
            .ok()
            .and_then(|d| d.and_hms_opt(23, 59, 59))
            .and_then(|dt| {
                chrono::FixedOffset::east_opt(8 * 3600)?.from_local_datetime(&dt).single()
            })
            .map(|dt| dt.timestamp())
            .unwrap_or_else(|| chrono::Utc::now().timestamp());
        Ok(vec![social_sentiment_from_posts(
            stock_code,
            bar_name.unwrap_or_default(),
            &posts,
            // 东财人气榜只有当下名次；历史行情里不跨平台借位，如实留 None
            None,
            fetched_at,
        )])
    }

    fn asof_capability(&self, method: &str) -> AsOfCapability {
        match method {
            // 2026-10-18：股吧列表页带 post_publish_time 且可按 f_N 翻页 ⇒ 有真日期通道
            "get_social_sentiment" => AsOfCapability::NativeDateParam,
            _ => AsOfCapability::Fallthrough,
        }
    }
}

/// 把一页帖子中 `post_publish_time <= cutoff_end` 的部分收进 `picked`（按 post_id 去重）。
///
/// 返回该页的**最大发帖时间**（含截止日之后的帖子）：调用方用它判断
/// 「是否已经翻过截止日边界」。`post_publish_time` 是 `YYYY-MM-DD HH:MM:SS`，
/// 字典序即时间序，无需解析成 DateTime。
fn collect_posts_as_of(
    posts: Vec<Value>,
    cutoff_end: &str,
    seen: &mut HashSet<String>,
    picked: &mut Vec<(String, Value)>,
) -> Option<String> {
    let mut page_max: Option<String> = None;
    for post in posts {
        let Some(publish) = post.get("post_publish_time").and_then(Value::as_str) else {
            continue;
        };
        if page_max.as_deref().is_none_or(|m| publish > m) {
            page_max = Some(publish.to_string());
        }
        if publish > cutoff_end {
            continue;
        }
        let key = post
            .get("post_id")
            .and_then(|v| v.as_i64())
            .map(|v| v.to_string())
            .or_else(|| post.get("post_id").and_then(Value::as_str).map(str::to_string))
            .unwrap_or_else(|| {
                format!(
                    "{publish}:{}",
                    post.get("post_title").and_then(Value::as_str).unwrap_or("")
                )
            });
        if seen.insert(key) {
            picked.push((publish.to_string(), post));
        }
    }
    page_max
}

/// 帖集合 → 舆情指标（live 与 as-of 的唯一实现）。
///
/// 空帖集合返回 `post_count = 0` 的**有效观测**（live 页面空列表就是这个语义）；
/// 调用方不要把它当成「没取到」——没取到是 `Err`。
fn social_sentiment_from_posts(
    stock_code: &str,
    stock_name: String,
    posts: &[Value],
    hot_rank: Option<u32>,
    fetched_at: i64,
) -> SocialSentiment {
    if posts.is_empty() {
        return SocialSentiment {
            stock_code: stock_code.to_string(),
            stock_name,
            platform: "guba".to_string(),
            post_count: 0,
            hot_rank,
            sentiment_score: None,
            bull_ratio: None,
            fetched_at,
        };
    }

    let mut bull_count = 0u32;
    let mut bear_count = 0u32;
    let mut score_sum = 0.0f64;
    let mut score_count = 0u32;
    for post in posts {
        let title = post["post_title"].as_str().unwrap_or("");
        let is_bull = crate::sentiment::POSITIVE_KEYWORDS.iter().any(|kw| title.contains(kw));
        let is_bear = crate::sentiment::RISK_KEYWORDS.iter().any(|kw| title.contains(kw))
            || crate::sentiment::HIGH_RISK_KEYWORDS.iter().any(|kw| title.contains(kw));
        if is_bull {
            bull_count += 1;
        }
        if is_bear {
            bear_count += 1;
        }
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
    // 统一词典全部未命中（纯水帖）时用 bull_ratio 兜底；两者都没有 ⇒ None
    let sentiment_score = if score_count > 0 {
        Some(score_sum / score_count as f64)
    } else {
        bull_ratio.map(|r| (r - 0.5) * 2.0)
    };

    SocialSentiment {
        stock_code: stock_code.to_string(),
        stock_name,
        platform: "guba".to_string(),
        post_count: posts.len() as u32,
        hot_rank,
        sentiment_score,
        bull_ratio,
        fetched_at,
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

    /// 「取不到帖子数组」必须**按失败步**分别可归因：「站点没给列表页」和「列表页改版」
    /// 的处置完全相反（前者退避重试有意义，后者重试永远拿同一个结果、要改解析器）。
    /// 每个样本只落一格；若某格没有样本、或两格被同一样本命中，本测试当场红。
    #[test]
    fn each_extract_failure_step_has_its_own_reason() {
        let cases: [(&str, ListPageFailure); 6] = [
            // 整页没有 `article_list=`（风控/登录壳页就是这个形态）
            (r#"<html><body><div>安全验证</div></body></html>"#, ListPageFailure::NotListPage),
            // `article_list=` 后面不是 `{`
            (
                r#"<script>var article_list=null; var other_list={};</script>"#,
                ListPageFailure::UnexpectedShape,
            ),
            // 找不到对象结束锚点 `var other_list`
            (
                r#"<script>var article_list={"re":[{"post_title":"涨"}]};</script>"#,
                ListPageFailure::MissingEndAnchor,
            ),
            // 有锚点但锚点之前没有 `};` ⇒ 同样归「定位不到对象边界」
            (
                r#"<script>var article_list={"re":[];} var other_list={};</script>"#,
                ListPageFailure::MissingEndAnchor,
            ),
            // 对象内没有 `"re":[`（键名/排版变了）
            (
                r#"<script>var article_list={"posts":[{"post_title":"涨"}],"bar_name":"x"}; var other_list={};</script>"#,
                ListPageFailure::MissingPostArray,
            ),
            // `"re":[` 在但括号未闭合（响应被截断）
            (
                r#"<script>var article_list={"re":[{"post_id":1,"m":[1,2]}; var other_list={};</script>"#,
                ListPageFailure::Truncated,
            ),
        ];
        // 上面 6 个样本覆盖 5 个变体（MissingEndAnchor 有两个样本）
        let mut seen: Vec<ListPageFailure> = Vec::new();
        for (html, expected) in cases {
            let got = GubaVendor::extract_article_list(html).err();
            assert_eq!(got, Some(expected), "样本未按预期归因: {html}");
            seen.push(expected);
        }
        for v in [
            ListPageFailure::NotListPage,
            ListPageFailure::UnexpectedShape,
            ListPageFailure::MissingEndAnchor,
            ListPageFailure::MissingPostArray,
            ListPageFailure::Truncated,
        ] {
            assert!(seen.contains(&v), "变体 {v:?} 没有样本，失败归因是空话");
        }
    }

    /// 失败文案不得命中 `DataError::is_empty_data` 的词表 —— 一旦命中，路由层把它
    /// 当成「源明确回答没有这份数据」的非故障：既不记降级也不重试，真故障就此消失
    /// （同族教训见 `vendors/xueqiu.rs` 的 get_news 注释）。
    #[test]
    fn failure_phrases_are_not_classified_as_empty_data() {
        for v in [
            ListPageFailure::NotListPage,
            ListPageFailure::UnexpectedShape,
            ListPageFailure::MissingEndAnchor,
            ListPageFailure::MissingPostArray,
            ListPageFailure::Truncated,
        ] {
            let e = DataError::VendorError { vendor: "guba".into(), message: v.phrase().into() };
            assert!(!e.is_empty_data(), "{} 被当成空数据（非故障）了", v.phrase());
        }
    }

    /// 拆桶的收益必须由**处置分道**兑现，不是只换一句文案：
    /// 「站点没给列表页」⇒ 限流（不计入 vendor 降级 + 拿得到重试）；其余四格 ⇒ 持续故障
    /// （立即上抛）。两格各钉一条 —— 若把分道写成「全归限流」或「全归故障」，当场红。
    #[test]
    fn only_absent_list_page_routes_to_rate_limit() {
        let shell = ListPageFailure::NotListPage.to_live_error("003029");
        assert!(
            matches!(shell, DataError::RateLimited { .. }),
            "站点没给列表页必须按限流上报，否则唯一舆情源会被健康追踪降级: {shell:?}"
        );
        assert!(shell.is_transient(), "只有算瞬态才拿得到 V54 让路后的那次重试");
        assert!(shell.to_string().contains("article_list"), "归限流不得丢掉具体形态: {shell}");
        assert!(shell.to_string().contains("003029"), "条目要带上标的: {shell}");

        for v in [
            ListPageFailure::UnexpectedShape,
            ListPageFailure::MissingEndAnchor,
            ListPageFailure::MissingPostArray,
            ListPageFailure::Truncated,
        ] {
            let e = v.to_live_error("003029");
            assert!(
                matches!(e, DataError::VendorError { .. }),
                "结构类失败必须仍按持续故障上抛: {e:?}"
            );
            assert!(!e.is_transient(), "改版重试永远拿同一个结果，不该放开: {}", v.phrase());
            assert!(!e.is_empty_data(), "{} 不得被当成空数据", v.phrase());
        }
    }

    /// 五格文案必须两两不同，否则「拆开」只是换了个混合桶。
    #[test]
    fn failure_phrases_are_distinct() {
        let all = [
            ListPageFailure::NotListPage,
            ListPageFailure::UnexpectedShape,
            ListPageFailure::MissingEndAnchor,
            ListPageFailure::MissingPostArray,
            ListPageFailure::Truncated,
        ];
        let phrases: Vec<&str> = all.iter().map(|v| v.phrase()).collect();
        for (i, p) in phrases.iter().enumerate() {
            for (j, q) in phrases.iter().enumerate() {
                if i != j {
                    assert_ne!(p, q, "两格共用文案: {p}");
                }
            }
        }
    }

    /// **禁连接复用**的判据：按真实 TCP 连接数断言，不读配置值。
    ///
    /// 用本地 listener 数 accept：同一 client 连发 2 次请求 —— 复用的 client 只建 1 条连接
    /// （这是负控，证明本测法真能测出复用）；股吧 client 必须建 2 条，拿到池化连接就红。
    #[tokio::test]
    async fn guba_client_never_reuses_connections() {
        use std::sync::atomic::{AtomicU32, Ordering};
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        async fn accepted_connections(client: &reqwest::Client) -> u32 {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let addr = listener.local_addr().unwrap();
            let counter = Arc::new(AtomicU32::new(0));
            let c = counter.clone();
            let server = tokio::spawn(async move {
                while let Ok((mut sock, _)) = listener.accept().await {
                    c.fetch_add(1, Ordering::SeqCst);
                    tokio::spawn(async move {
                        let mut buf = [0u8; 1024];
                        // 不主动关连接：让它能 keep-alive，复用才有机会发生
                        while let Ok(n) = sock.read(&mut buf).await {
                            if n == 0 {
                                break;
                            }
                            // 写失败 = 客户端已断开，结束本连接的读取循环即可。
                            if sock
                                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok")
                                .await
                                .is_err()
                            {
                                break;
                            }
                        }
                    });
                }
            });
            let url = format!("http://{addr}/list,000001.html");
            for _ in 0..2 {
                let resp = client.get(&url).send().await.unwrap();
                assert_eq!(resp.text().await.unwrap(), "ok");
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            server.abort();
            counter.load(Ordering::SeqCst)
        }

        let shared = accepted_connections(&reqwest::Client::new()).await;
        assert_eq!(shared, 1, "负控失败：默认 client 本该复用同一条连接，否本测法测不出复用");

        let guba = GubaVendor::new();
        let dedicated = accepted_connections(&guba.http).await;
        assert_eq!(dedicated, 2, "股吧 client 必须每次新建连接 —— 复用会拿到「身份核实」页");
    }

    /// 现场摘要按 `chars()` 切 —— 中文 HTML 按字节切会在非 UTF-8 边界 panic。
    #[test]
    fn forensics_survives_multibyte_head() {
        let html = "股吧".repeat(200);
        let s = html_forensics(&html);
        assert!(s.contains("len=1200"), "摘要要带长度: {s}");
        let head24: String = s.chars().take(24).collect();
        assert!(s.contains("股吧"), "摘要头部应可读: {head24}");
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

    /// 移动端兜底：`window.list` 的 `re` 数组与 `bar_info.ShortName` 必须能取出，
    /// 且帖子内嵌套 `reply_list` 的括号不得截断数组。
    #[test]
    fn extract_mobile_list_parses_window_list_and_short_name() {
        let html = r#"<script>
        window.type = 1;
        window.sort = 1;
        window.list = {"bar_info":{"OuterCode":"002343","ShortName":"慈文传媒","Type":[200]},
        "re":[{"post_id":1,"post_title":"涨","reply_list":[{"reply_text":"含 } 的正文"}],"post_publish_time":"2026-10-25 18:52:47"},
        {"post_id":2,"post_title":"跌","reply_list":[]}],"count":2};
    </script>"#;
        let (json_str, short_name) =
            GubaVendor::extract_mobile_list(html).expect("应解析出 window.list");
        let posts: Vec<Value> = serde_json::from_str(json_str).unwrap();
        assert_eq!(posts.len(), 2);
        assert_eq!(posts[0]["post_title"].as_str(), Some("涨"));
        assert_eq!(posts[0]["post_publish_time"].as_str(), Some("2026-10-25 18:52:47"));
        assert_eq!(short_name, Some("慈文传媒"));
    }

    /// 身份核实壳页不含 `window.list` ⇒ 兜底解析必须如实返回 None，不能伪造空帖。
    #[test]
    fn extract_mobile_list_rejects_verify_shell_page() {
        let shell = r#"<html lang="en"><head><title>身份核实</title></head><body><div id="root"></div></body></html>"#;
        assert!(GubaVendor::extract_mobile_list(shell).is_none());
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

    /// as-of 选帖：只收截止日及之前的帖子；截止日之后的必须被滤掉；
    /// 同一 post_id 跨页只算一次；返回的 page_max 含**截止日之后**的帖子
    /// （调用方靠它判断翻页是否已越过边界）。
    #[test]
    fn collect_posts_as_of_filters_cutoff_and_dedups() {
        let page = vec![
            serde_json::json!({"post_id": 1, "post_title": "旧帖", "post_publish_time": "2026-09-20 10:00:00"}),
            serde_json::json!({"post_id": 2, "post_title": "截止日当天", "post_publish_time": "2026-09-28 23:59:59"}),
            serde_json::json!({"post_id": 3, "post_title": "未来帖", "post_publish_time": "2026-10-01 09:00:00"}),
            serde_json::json!({"post_id": 1, "post_title": "旧帖重复", "post_publish_time": "2026-09-20 10:00:00"}),
            serde_json::json!({"post_title": "无 id", "post_publish_time": "2026-09-21 08:00:00"}),
        ];
        let mut seen = HashSet::new();
        let mut picked = Vec::new();
        let page_max = collect_posts_as_of(page, "2026-09-28 23:59:59", &mut seen, &mut picked);
        assert_eq!(page_max.as_deref(), Some("2026-10-01 09:00:00"), "page_max 必须含未来帖");
        assert_eq!(picked.len(), 3, "旧帖 + 截止日当天 + 无 id 帖；重复/未来帖被滤掉: {picked:?}");
        assert!(picked.iter().all(|(t, _)| t.as_str() <= "2026-09-28 23:59:59"));
    }

    /// live/as-of 共用的情感计算：空帖返回 post_count=0 的有效观测，不是 None/Err。
    #[test]
    fn sentiment_from_posts_same_algorithm_for_both_paths() {
        let posts = vec![
            serde_json::json!({"post_title": "涨停 利好 看好"}),
            serde_json::json!({"post_title": "下跌 风险 利空"}),
        ];
        let s = social_sentiment_from_posts("002696", "百洋股份".into(), &posts, None, 123);
        assert_eq!(s.post_count, 2);
        assert!(s.bull_ratio.is_some());
        assert!(s.sentiment_score.is_some());
        assert_eq!(s.fetched_at, 123);
        let empty = social_sentiment_from_posts("002696", "百洋股份".into(), &[], None, 456);
        assert_eq!(empty.post_count, 0);
        assert!(empty.sentiment_score.is_none());
        assert_eq!(empty.fetched_at, 456);
    }

    /// 真接口冒烟（`#[ignore]`，约 20–30 页、~10s）：
    /// `cargo test -p axagent-astock-data social_asof_live -- --ignored --nocapture`
    ///
    /// 002696 / 2026-09-28 实测：26 页可收到 142 条截止日前帖子，取最近 81 条。
    #[tokio::test]
    #[ignore = "需真实网络 + 多页翻页；仅在手工验证舆情 as-of 通道时跑"]
    async fn social_asof_live() {
        use crate::as_of::{AsOfContext, AsOfSource, AS_OF};
        let v = GubaVendor::new();
        let ctx = AsOfContext::new(
            chrono::NaiveDate::from_ymd_opt(2026, 9, 28).unwrap(),
            AsOfSource::UserReplay,
        )
        .unwrap();
        let got = AS_OF
            .scope(Some(ctx), async { v.get_social_sentiment_with_asof("002696").await })
            .await
            .expect("股吧翻页应可达");
        let s = got.first().expect("002696 在 09-28 前应有帖子");
        assert!(s.post_count > 0, "重建出的帖子数应 > 0");
        assert_eq!(s.stock_name, "百洋股份");
        // 截止日 23:59:59 北京时间
        assert!(s.fetched_at > 1_790_000_000, "fetched_at 应是 2026-09-28 附近的历史时间戳");
        println!(
            "002696 as-of 舆情: posts={} score={:?} bull={:?}",
            s.post_count, s.sentiment_score, s.bull_ratio
        );
    }
}
